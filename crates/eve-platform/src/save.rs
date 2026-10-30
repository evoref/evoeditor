//! 原子的保存 `write_atomic` の Windows 実装 (docs/05 §4.1 の共通手順と §4.2、§5.6)。
//!
//! 順序: 0 事前検査 (リンクの解決・親・照合・ハードリンク) → 1 一時ファイル (所有者のみの DACL) → 2 チャンク書込 → 3 fsync・ハンドルを閉じる
//! → 4 属性 (HIDDEN / SYSTEM) の複製 → 4b 置換の直前の照合 → 5 `ReplaceFileW` (新規ファイルは `MoveFileExW`) → 7 事後 (FileId の確認・退避名の削除)。
//! 失敗では一時ファイルを消し、元のファイルは無傷に保つ。置換は `ReplaceFileW` だけで、フラグは 0 (ACL・ADS・作成日時は
//! `ReplaceFileW` が保つ)。置換に失敗して T が無傷なら `AtomicUnsupported` (確認付きの直接上書きは core 側)。
//!
//! 範囲外 (後続): 保存中フラグ (05 §4.6 / §7.5)、置換中の計数 (`ReplacingGuard`)、`write_in_place`、一時ファイルの規則 (`InPlacePattern`)、内容ハッシュ。

#![allow(unsafe_code)]

use std::collections::hash_map::RandomState;
use std::fs::{self, File};
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle;
use std::path::{Path, PathBuf};
use std::ptr;
use std::thread;
use std::time::Duration;

use eve_core::{
    AttrKind, ChunkError, Expect, FileKind, FileMeta, FileStamp, FileSystem, FsError,
    FsErrorDetail, FsErrorKind, FsPath, FsyncLevel, HardlinkPolicy, ReadChunks, SaveMethod,
    SaveOptions, SaveReport, SaveWarning,
};
use windows_sys::Win32::Foundation::{
    CloseHandle, GENERIC_WRITE, GetLastError, HANDLE, INVALID_HANDLE_VALUE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    CREATE_NEW, CreateFileW, DeleteFileW, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_NORMAL,
    FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_SYSTEM, FILE_SHARE_READ, FlushFileBuffers,
    GetFileAttributesW, INVALID_FILE_ATTRIBUTES, MOVEFILE_WRITE_THROUGH, MoveFileExW, ReplaceFileW,
    SetFileAttributesW,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::fs::{WinFs, map_io_error};
use crate::hooks;
use crate::inflight;
use crate::path::simplify_verbatim;

/// 共有違反の再試行の待ち (ミリ秒、05 §4.2。合計 2.55 s)。
pub(crate) const SHARING_RETRY_MS: [u64; 6] = [50, 100, 200, 400, 800, 1000];

// Windows のエラーコード (windows-sys の定数の型の違いを避けて数値で持つ)
const ERROR_ACCESS_DENIED: u32 = 5;
pub(crate) const ERROR_SHARING_VIOLATION: u32 = 32;
pub(crate) const ERROR_LOCK_VIOLATION: u32 = 33;
const ERROR_FILE_EXISTS: u32 = 80;
const ERROR_ALREADY_EXISTS: u32 = 183;
const ERROR_UNABLE_TO_REMOVE_REPLACED: u32 = 1175;
const ERROR_UNABLE_TO_MOVE_REPLACEMENT_2: u32 = 1177;

/// Windows のファイル名の上限 (UTF-16 単位)。
const NAME_LIMIT: usize = 255;

/// Win32 の `…W` 関数に渡す NUL 終端の UTF-16。長いパスは拡張長形式 (`\\?\`) にする (`extend_long`)。
/// 既存のファイルを置き換えない名前の変更 (`MOVEFILE_REPLACE_EXISTING` なし。`to` が在れば失敗する)。
pub(crate) fn rename_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    let (wf, wt) = (wide(from), wide(to));
    // SAFETY: NUL 終端のパス 2 つ。
    let ok = unsafe { MoveFileExW(wf.as_ptr(), wt.as_ptr(), MOVEFILE_WRITE_THROUGH) };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

pub(crate) fn wide(p: &Path) -> Vec<u16> {
    let mut w = extend_long(p.as_os_str().encode_wide().collect());
    w.push(0);
    w
}

/// `MAX_PATH` に近い長さのパスを、`\\?\C:\…` / `\\?\UNC\server\share\…` の形にする (NFR-404)。
/// Rust の `std::fs` は自動でこの形にするが、`ReplaceFileW` などの直接の呼出しは自動では変換されない。
/// 短いパス・相対パス・すでに `\\?\` / `\\.\` のものは変えない (拡張長形式は `.` `..` `/` を解釈しないため、必要なときだけ使う)。
pub(crate) fn extend_long(w: Vec<u16>) -> Vec<u16> {
    // MAX_PATH (260) から、ファイル名と NUL の余裕を引いた値 (ディレクトリの作成の上限 248 より手前)
    const LIMIT: usize = 240;
    let bs = u16::from(b'\\');
    let is_alpha = |c: u16| u8::try_from(c).is_ok_and(|b| b.is_ascii_alphabetic());
    if w.len() < LIMIT {
        return w;
    }
    let unc = w.len() >= 3 && w[0] == bs && w[1] == bs;
    let mut out: Vec<u16> = if unc {
        if w[2] == u16::from(b'?') || w[2] == u16::from(b'.') {
            return w;
        }
        r"\\?\UNC\"
            .encode_utf16()
            .chain(w[2..].iter().copied())
            .collect()
    } else if w.len() >= 3
        && is_alpha(w[0])
        && w[1] == u16::from(b':')
        && (w[2] == bs || w[2] == u16::from(b'/'))
    {
        r"\\?\".encode_utf16().chain(w.iter().copied()).collect()
    } else {
        return w;
    };
    for c in &mut out {
        if *c == u16::from(b'/') {
            *c = bs;
        }
    }
    out
}

pub(crate) fn last_error() -> u32 {
    // SAFETY: スレッドの最後のエラーを読むだけ。
    unsafe { GetLastError() }
}

pub(crate) fn os_error(code: u32, path: &FsPath) -> FsError {
    map_io_error(io::Error::from_raw_os_error(code as i32), path)
}

// ---------- 所有者のみの DACL (05 §5.6) ----------

/// 「所有者のみ」の `SECURITY_ATTRIBUTES` (現在のユーザーと SYSTEM だけにフルアクセス、継承を遮断)。
pub(crate) struct OwnerOnly {
    descriptor: *mut core::ffi::c_void,
    attrs: SECURITY_ATTRIBUTES,
}

impl OwnerOnly {
    /// ファイル用 (継承は遮断)。
    pub(crate) fn new() -> io::Result<OwnerOnly> {
        OwnerOnly::with_aces("")
    }

    /// ディレクトリ用 (中のファイルとディレクトリへ継承させる)。
    pub(crate) fn new_dir() -> io::Result<OwnerOnly> {
        OwnerOnly::with_aces("OICI")
    }

    fn with_aces(inherit: &str) -> io::Result<OwnerOnly> {
        let sid = current_user_sid()?;
        let sddl: Vec<u16> = format!("O:{sid}D:P(A;{inherit};FA;;;{sid})(A;{inherit};FA;;;SY)")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut descriptor: *mut core::ffi::c_void = ptr::null_mut();
        // SAFETY: `sddl` は NUL 終端の有効な文字列、`descriptor` は書込先の有効なポインタ。
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(OwnerOnly {
            descriptor,
            attrs: SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            },
        })
    }

    pub(crate) fn as_ptr(&self) -> *const SECURITY_ATTRIBUTES {
        &self.attrs
    }
}

impl Drop for OwnerOnly {
    fn drop(&mut self) {
        // SAFETY: `descriptor` は `ConvertStringSecurityDescriptorToSecurityDescriptorW` が `LocalAlloc` で確保したもの。
        unsafe {
            LocalFree(self.descriptor);
        }
    }
}

/// 現在のユーザーの SID の文字列 (`S-1-5-21-…`)。
pub(crate) fn current_user_sid() -> io::Result<String> {
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: 自プロセスのトークンを問い合わせ用に開く。
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let result = (|| {
        let mut len = 0u32;
        // SAFETY: 長さの問い合わせ (バッファなし)。失敗 (ERROR_INSUFFICIENT_BUFFER) が正常。
        unsafe { GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut len) };
        let mut buf = vec![0u8; len as usize];
        // SAFETY: `buf` は `len` バイトの書込先。
        if unsafe { GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `GetTokenInformation(TokenUser)` は `TOKEN_USER` を書く (先頭に置かれ、アラインは確保時の `Vec<u8>` に依らず
        // `read_unaligned` で読む)。
        let user: TOKEN_USER = unsafe { ptr::read_unaligned(buf.as_ptr().cast()) };
        let mut s: *mut u16 = ptr::null_mut();
        // SAFETY: `user.User.Sid` は `buf` の中の有効な SID。
        if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut s) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `s` は NUL 終端の UTF-16 (`LocalAlloc` で確保)。読んでから `LocalFree` する。
        let text = unsafe {
            let mut n = 0;
            while *s.add(n) != 0 {
                n += 1;
            }
            let t = String::from_utf16_lossy(std::slice::from_raw_parts(s, n));
            LocalFree(s.cast());
            t
        };
        Ok(text)
    })();
    // SAFETY: 上で開いたトークンのハンドル。
    unsafe { CloseHandle(token) };
    result
}

// ---------- 一時ファイル ----------

/// 一時ファイルを `CREATE_NEW`・`FILE_SHARE_READ` で作る (所有者のみの DACL を渡せる)。
fn create_new(path: &Path, owner_only: Option<&OwnerOnly>) -> io::Result<File> {
    let w = wide(path);
    let sa = owner_only.map_or(ptr::null(), OwnerOnly::as_ptr);
    // SAFETY: `w` は NUL 終端、`sa` は null または有効な `SECURITY_ATTRIBUTES`。
    let h = unsafe {
        CreateFileW(
            w.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ,
            sa,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `h` は有効なファイルハンドルで、`File` が所有する (閉じるのは `File` の Drop)。
    Ok(unsafe { File::from_raw_handle(h) })
}

pub(crate) fn delete(path: &Path) -> bool {
    let w = wide(path);
    // SAFETY: NUL 終端のパス。
    unsafe { DeleteFileW(w.as_ptr()) != 0 }
}

fn flush(f: &File) -> bool {
    use std::os::windows::io::AsRawHandle;
    // SAFETY: 開いているファイルのハンドル。
    unsafe { FlushFileBuffers(f.as_raw_handle()) != 0 }
}

fn attributes(path: &Path) -> Option<u32> {
    let w = wide(path);
    // SAFETY: NUL 終端のパス。
    let a = unsafe { GetFileAttributesW(w.as_ptr()) };
    (a != INVALID_FILE_ATTRIBUTES).then_some(a)
}

fn set_attributes(path: &Path, attrs: u32) -> bool {
    let w = wide(path);
    // SAFETY: NUL 終端のパス。
    unsafe { SetFileAttributesW(w.as_ptr(), attrs) != 0 }
}

/// `.<name>.eve-<kind>-<pid>-<rand8>`。全体が 255 UTF-16 単位を超えるときは `<name>` を文字境界で切り詰め、`~<hash8>` を付ける (05 §4.1)。
fn sibling_name(name: &str, kind: &str) -> String {
    let suffix = format!(".eve-{kind}-{}-{:08x}", std::process::id(), rand32());
    let full = format!(".{name}{suffix}");
    if full.encode_utf16().count() <= NAME_LIMIT {
        return full;
    }
    let hash = {
        let mut h: u32 = 0x811c_9dc5;
        for b in name.bytes() {
            h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
        }
        format!("~{h:08x}")
    };
    let room = NAME_LIMIT - 1 - suffix.encode_utf16().count() - hash.len();
    let mut cut = String::new();
    let mut units = 0;
    for c in name.chars() {
        if units + c.len_utf16() > room {
            break;
        }
        units += c.len_utf16();
        cut.push(c);
    }
    format!(".{cut}{hash}{suffix}")
}

fn rand32() -> u32 {
    let mut h = RandomState::new().build_hasher();
    h.write_u32(std::process::id());
    h.finish() as u32
}

/// 照合の比較 (05 §5.5。ハッシュは持たないので (mtime, len, FileId))。
pub(crate) fn matches(want: &FileStamp, have: &FileStamp) -> bool {
    want.mtime == have.mtime && want.len == have.len && want.file_id == have.file_id
}

pub(crate) fn same_file_state(a: &FileMeta, b: &FileMeta) -> bool {
    a.mtime == b.mtime && a.len == b.len && a.file_id == b.file_id
}

/// 保存先 T: シンボリックリンクなら最終リンク先 (ダングリングは `NotFound` + `DanglingLink`)。
/// `create` ならダングリングのリンクのリンク先を T にする (新規作成の経路で書く。05 §4.1)。リンク先もリンク (連鎖) なら `InvalidPath`。
pub(crate) fn resolve_target(path: &FsPath, create: bool) -> Result<PathBuf, FsError> {
    let mut target: PathBuf = path.as_path().to_path_buf();
    if let Ok(sm) = fs::symlink_metadata(&target)
        && sm.file_type().is_symlink()
    {
        match fs::canonicalize(&target) {
            Ok(t) => target = PathBuf::from(simplify_verbatim(&t.to_string_lossy())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if create && let Ok(raw) = fs::read_link(&target) {
                    // 相対のリンク先はリンクのあるディレクトリから
                    let t = match target.parent() {
                        Some(dir) if raw.is_relative() => dir.join(&raw),
                        _ => raw,
                    };
                    if fs::symlink_metadata(&t).is_ok() {
                        return Err(FsError::new(FsErrorKind::InvalidPath));
                    }
                    return Ok(t);
                }
                let link_target = fs::read_link(&target).ok().map(FsPath::new);
                return Err(FsError {
                    kind: FsErrorKind::NotFound,
                    detail: link_target.map(|target| FsErrorDetail::DanglingLink { target }),
                    source: Some(e),
                });
            }
            Err(e) => return Err(map_io_error(e, path)),
        }
    }
    Ok(target)
}

/// 保存中フラグ (05 §4.1 手順 0d / 7 / 8)。状態ディレクトリが無い・状態ファイルの書込 (`state_file`) ではフラグを書かない。
pub(crate) struct FlagGuard {
    pub(crate) state_dir: Option<PathBuf>,
    pub(crate) key: Option<String>,
    pub(crate) path: Option<PathBuf>,
}

impl FlagGuard {
    /// 一時ファイルを作る前に書く。書けなければ警告して続ける (原子的置換では T が無傷のまま残るため)。
    fn begin(
        state_dir: Option<&Path>,
        state_file: bool,
        target: &Path,
        method: SaveMethod,
        tmp: &Path,
        bak: Option<&Path>,
        warnings: &mut Vec<SaveWarning>,
    ) -> FlagGuard {
        let mut g = FlagGuard {
            state_dir: None,
            key: None,
            path: None,
        };
        let Some(sd) = state_dir.filter(|_| !state_file) else {
            return g;
        };
        g.state_dir = Some(sd.to_path_buf());
        g.key = inflight::flag_key(target);
        let written = g.key.as_deref().and_then(|key| {
            inflight::write_flag(
                sd,
                key,
                &inflight::FlagBody {
                    target,
                    method,
                    tmp: Some(tmp),
                    backup: bak,
                    backup_recorded: false,
                    backup_complete: false,
                },
            )
            .ok()
        });
        match written {
            Some(p) => g.path = Some(p),
            None => warnings.push(SaveWarning::InflightFlagNotWritten),
        }
        g
    }

    /// 直接上書きのフラグ (05 §4.4 手順 2)。書けなければ `None` (呼び側が `BackupFailed` にするか、警告で続ける)。
    pub(crate) fn begin_in_place(
        state_dir: &Path,
        target: &Path,
        backup: Option<&Path>,
    ) -> Option<FlagGuard> {
        let key = inflight::flag_key(target)?;
        let path = inflight::write_flag(
            state_dir,
            &key,
            &inflight::FlagBody {
                target,
                method: SaveMethod::InPlace,
                tmp: None,
                backup,
                backup_recorded: backup.is_some(),
                backup_complete: false,
            },
        )
        .ok()?;
        Some(FlagGuard {
            state_dir: Some(state_dir.to_path_buf()),
            key: Some(key),
            path: Some(path),
        })
    }

    /// 事前バックアップを最終名にした後、フラグを `backup_complete = true` で書き直す。
    pub(crate) fn mark_backup_complete(&self, target: &Path, backup: &Path) -> io::Result<()> {
        let Some(p) = &self.path else {
            return Ok(());
        };
        inflight::rewrite_flag(
            p,
            &inflight::FlagBody {
                target,
                method: SaveMethod::InPlace,
                tmp: None,
                backup: Some(backup),
                backup_recorded: true,
                backup_complete: true,
            },
        )
    }

    /// 自分のフラグを消す。
    pub(crate) fn remove(&mut self) {
        if let Some(p) = self.path.take() {
            let _ = fs::remove_file(p);
        }
    }

    /// 成功: 自分のフラグと、同じキーに残る他の記録 (痕跡ファイルが実在しないもの) を消す。
    pub(crate) fn finish_success(&mut self) {
        let mine = self.path.clone();
        self.remove();
        if let (Some(sd), Some(key), Some(mine)) = (&self.state_dir, &self.key, mine) {
            inflight::clear_others(sd, key, &mine);
        }
    }
}

/// 置換の前の失敗の後片付け: 一時ファイルを消せたらフラグも消す (消せなければ残す。05 §4.1 手順 8)。
fn abort(tmp: &Path, flag: &mut FlagGuard) {
    if delete(tmp) {
        flag.remove();
    }
}

impl WinFs {
    pub(crate) fn write_atomic_impl(
        &self,
        path: &FsPath,
        data: &mut dyn ReadChunks,
        opts: &SaveOptions,
    ) -> Result<SaveReport, FsError> {
        let conflict = |current| FsError::new(FsErrorKind::Conflict { current });

        // 0a. シンボリックリンクなら最終リンク先 T を対象にする。ダングリングは保存しない (v0.1、05 §4.1)
        let target = resolve_target(path, opts.create_link_target)?;
        let target_path = FsPath::new(target.clone());
        let parent = target
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| FsError::new(FsErrorKind::InvalidPath))?;
        if !parent.is_dir() {
            return Err(FsError::with_detail(
                FsErrorKind::NotFound,
                FsErrorDetail::ParentMissing {
                    dir: FsPath::new(parent),
                },
            ));
        }
        let name = target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| FsError::new(FsErrorKind::InvalidPath))?;

        // 0b. 書き始める前の照合 (T が在るか、期待どおりか)
        let existing: Option<FileMeta> = match self.metadata(&target_path) {
            Ok(m) => Some(m),
            Err(e) if e.kind == FsErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        if let Some(m) = &existing
            && m.kind != FileKind::File
        {
            return Err(FsError::with_detail(
                FsErrorKind::InvalidPath,
                FsErrorDetail::NotAFile,
            ));
        }
        let current = existing.as_ref().map(|m| FileStamp::from_meta(m, None));
        match (opts.expected, &current) {
            (Expect::Absent, Some(_)) => return Err(conflict(current)),
            (Expect::Stamp(_), None) => return Err(conflict(None)),
            (Expect::Stamp(want), Some(have)) if !matches(&want, have) => {
                return Err(conflict(current));
            }
            _ => {}
        }
        // ハードリンク: 既定は確認 (`Ask`)。`BreakLink` はリンクを切る通常の置換、`WriteInPlace` は直接上書き (core が呼ぶ)
        if let Some(m) = &existing
            && m.nlink > 1
            && opts.hardlink != HardlinkPolicy::BreakLink
        {
            return Err(FsError::with_detail(
                FsErrorKind::AtomicUnsupported,
                FsErrorDetail::Hardlink { nlink: m.nlink },
            ));
        }

        // 1. 一時ファイル。T が在るときは所有者のみの DACL (T の ACL は置換で `ReplaceFileW` が適用する)。
        // 新規ファイルは `private` のときだけ所有者のみ (継承 ACL で作り、通常の新規ファイルと同じにする)
        let owner = if existing.is_some() || opts.private {
            Some(OwnerOnly::new().map_err(|e| map_io_error(e, path))?)
        } else {
            None
        };
        let tmp: PathBuf = parent.join(sibling_name(&name, "tmp"));
        let bak: Option<PathBuf> = existing
            .is_some()
            .then(|| parent.join(sibling_name(&name, "bak")));
        let mut warnings = Vec::new();
        // 0d. 保存中フラグ (一時ファイルを作る前)
        let mut flag = FlagGuard::begin(
            self.state_dir(),
            opts.state_file,
            &target,
            if existing.is_some() {
                SaveMethod::Atomic
            } else {
                SaveMethod::NewFile
            },
            &tmp,
            bak.as_deref(),
            &mut warnings,
        );
        let mut file = match create_new(&tmp, owner.as_ref()) {
            Ok(f) => f,
            // 親に書込権限がない (一時ファイルを作れない): 確認付きの直接上書きへ (05 §4.4)
            Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) => {
                flag.remove();
                return Err(FsError::new(FsErrorKind::AtomicUnsupported));
            }
            Err(e) => {
                flag.remove();
                return Err(map_io_error(e, path));
            }
        };

        hooks::hit("kill_after_tmp_alloc");

        // 2. 本文をチャンクで書く
        loop {
            match data.next_chunk() {
                Ok(Some(chunk)) => {
                    if let Err(e) = file.write_all(chunk) {
                        drop(file);
                        abort(&tmp, &mut flag);
                        return Err(map_io_error(e, path));
                    }
                }
                Ok(None) => break,
                // 供給側の失敗・取消し: 一時ファイルを消し、元のファイルは無傷
                Err(e) => {
                    drop(file);
                    abort(&tmp, &mut flag);
                    return Err(match e {
                        ChunkError::Io(err) => err,
                        _ => FsError::new(FsErrorKind::SourceFailed),
                    });
                }
            }
        }

        hooks::hit("kill_after_tmp_write");

        // 3. fsync。失敗 (ネットワーク FS・リムーバブル) は警告として続ける
        if opts.fsync != FsyncLevel::None && !flush(&file) {
            warnings.push(SaveWarning::FsyncFailed);
        }
        hooks::hit("kill_after_flush");

        // 3b. ハンドルを閉じる (開いたまま `ReplaceFileW` を呼ぶと共有違反)
        drop(file);
        let tmp_path = FsPath::new(tmp.clone());
        let tmp_meta = self.metadata(&tmp_path).ok();

        // 4. 属性の複製 (HIDDEN / SYSTEM は置換の前に tmp へ付ける。READONLY は付けない)
        if opts.preserve.file_attributes
            && existing.is_some()
            && let Some(attrs) = attributes(&target)
        {
            let keep = attrs & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM);
            if keep != 0 {
                let cur = attributes(&tmp).unwrap_or(FILE_ATTRIBUTE_NORMAL);
                if !set_attributes(&tmp, cur | keep) {
                    warnings.push(SaveWarning::AttrsNotPreserved {
                        what: vec![AttrKind::FileAttributes],
                    });
                }
            }
        }

        // 4b. 置換の直前の照合 ((mtime, len, FileId) だけ。T を読み直さない)
        if opts.expected != Expect::Any {
            let now = match self.metadata(&target_path) {
                Ok(m) => Some(m),
                Err(e) if e.kind == FsErrorKind::NotFound => None,
                Err(e) => {
                    abort(&tmp, &mut flag);
                    return Err(e);
                }
            };
            let stale = match (&existing, &now) {
                (Some(a), Some(b)) => !same_file_state(a, b),
                (None, None) => false,
                _ => true,
            };
            if stale {
                abort(&tmp, &mut flag);
                return Err(conflict(
                    now.as_ref().map(|m| FileStamp::from_meta(m, None)),
                ));
            }
        }

        hooks::hit("kill_before_rename");

        // 「読み取り専用を解除して保存」: 置換の直前に T の READONLY を外す (置換の後に付け直す、手順 5b)
        let readonly_attrs = match &existing {
            Some(_) if opts.clear_readonly => {
                attributes(&target).filter(|a| a & FILE_ATTRIBUTE_READONLY != 0)
            }
            _ => None,
        };
        if let Some(a) = readonly_attrs
            && !set_attributes(&target, a & !FILE_ATTRIBUTE_READONLY)
        {
            abort(&tmp, &mut flag);
            return Err(FsError::new(FsErrorKind::PermissionDenied));
        }

        // 5. 置換 (終了の経路はここが済むまで待つ。置換の途中で終わると元のファイルが退避名に移ったままになる)
        let _replacing = self.replacing.guard(&target_path.display());
        let method;
        let backup: Option<PathBuf>;
        if let Some(orig) = &existing {
            method = SaveMethod::Atomic;
            let bak = bak.clone().expect("T が在るときは退避名を決めてある");
            backup = Some(bak.clone());
            // `ReplaceFileW` の途中の状態 (T を退避名へ移した後、一時ファイルを T へ移す前) の模擬 (テストのフックだけ)
            #[cfg(feature = "test-hooks")]
            if std::env::var("EVE_TEST_HOOK").is_ok_and(|v| v == "kill_after_replace_begin") {
                let _ = fs::rename(&target, &bak);
                hooks::kill_now();
            }
            if let Err(e) = replace_file(&target, &tmp, &bak) {
                // 置換に失敗: T が無傷なら一時ファイルを消して `AtomicUnsupported`、そうでなければ退避名に残っている
                let intact = self
                    .metadata(&target_path)
                    .is_ok_and(|m| same_file_state(orig, &m));
                // 外した READONLY を戻す (T が無傷なら T に、退避名に移っていれば退避名に)
                if let Some(a) = readonly_attrs {
                    let at = if intact { &target } else { &bak };
                    let _ = set_attributes(at, a);
                }
                return Err(match e {
                    ReplaceFailure::MovedToBackup => {
                        FsError::new(FsErrorKind::ReplacedMovedToBackup {
                            backup: FsPath::new(bak),
                        })
                    }
                    ReplaceFailure::Other(code) if intact => {
                        abort(&tmp, &mut flag);
                        let mut err = FsError::new(FsErrorKind::AtomicUnsupported);
                        err.source = Some(io::Error::from_raw_os_error(code as i32));
                        err
                    }
                    ReplaceFailure::Other(_) => FsError::new(FsErrorKind::ReplacedMovedToBackup {
                        backup: FsPath::new(bak),
                    }),
                });
            }
        } else {
            method = SaveMethod::NewFile;
            backup = None;
            // 既存のファイルを上書きしない移動 (`MOVEFILE_REPLACE_EXISTING` なし)。同名のファイルが作られていたら Conflict
            let (wt, wd) = (wide(&tmp), wide(&target));
            // SAFETY: NUL 終端のパス 2 つ。
            let ok = unsafe { MoveFileExW(wt.as_ptr(), wd.as_ptr(), MOVEFILE_WRITE_THROUGH) };
            if ok == 0 {
                let code = last_error();
                abort(&tmp, &mut flag);
                if code == ERROR_ALREADY_EXISTS || code == ERROR_FILE_EXISTS {
                    let cur = self
                        .metadata(&target_path)
                        .ok()
                        .map(|m| FileStamp::from_meta(&m, None));
                    return Err(conflict(cur));
                }
                return Err(os_error(code, path));
            }
        }

        // 5b. 外した READONLY を付け直す
        if readonly_attrs.is_some() {
            let cur = attributes(&target).unwrap_or(FILE_ATTRIBUTE_NORMAL);
            if !set_attributes(&target, cur | FILE_ATTRIBUTE_READONLY) {
                warnings.push(SaveWarning::AttrsNotPreserved {
                    what: vec![AttrKind::FileAttributes],
                });
            }
        }

        // 7. 事後: 置換後の T が自分の書いたものか (FileId) を確かめて new_stamp にする
        let after = self.metadata(&target_path).ok();
        let new_stamp = match (&after, &tmp_meta) {
            (Some(a), Some(t)) if a.file_id == t.file_id => FileStamp::from_meta(a, None),
            (Some(_), Some(t)) => {
                // 置換の直後に他のプログラムが T を置き換えた可能性: 自分の書いた内容の値を返す (次の保存は照合で Conflict になる)
                warnings.push(SaveWarning::ChangedAfterSave);
                FileStamp::from_meta(t, None)
            }
            (Some(a), None) => FileStamp::from_meta(a, None),
            (None, _) => {
                return Err(FsError::new(FsErrorKind::Other));
            }
        };
        // 退避名を消せたらフラグを消す。消せなければフラグを残す (v0.2 の起動時の走査の手掛かり)
        let mut bak_gone = true;
        if let Some(bak) = backup
            && bak.exists()
            && !delete(&bak)
        {
            bak_gone = false;
            warnings.push(SaveWarning::BackupNotRemoved {
                path: FsPath::new(bak),
            });
        }
        if bak_gone {
            flag.finish_success();
        }
        Ok(SaveReport {
            method,
            new_stamp,
            warnings,
        })
    }
}

enum ReplaceFailure {
    /// T が退避名へ移動済みで、tmp の移動に失敗した (`ERROR_UNABLE_TO_MOVE_REPLACEMENT_2`)。
    MovedToBackup,
    Other(u32),
}

/// `ReplaceFileW(T, tmp, backup, 0)`。共有違反は指数バックオフで再試行する (05 §4.2。待つ間も取消しは見ない)。
fn replace_file(target: &Path, tmp: &Path, backup: &Path) -> Result<(), ReplaceFailure> {
    let (wt, wr, wb) = (wide(target), wide(tmp), wide(backup));
    let mut attempt = 0;
    loop {
        // SAFETY: NUL 終端のパス 3 つ。フラグ 0 (`IGNORE_MERGE_ERRORS` / `IGNORE_ACL_ERRORS` は付けない)。
        let ok = unsafe {
            ReplaceFileW(
                wt.as_ptr(),
                wr.as_ptr(),
                wb.as_ptr(),
                0,
                ptr::null(),
                ptr::null(),
            )
        };
        if ok != 0 {
            return Ok(());
        }
        let code = last_error();
        if code == ERROR_UNABLE_TO_MOVE_REPLACEMENT_2 {
            return Err(ReplaceFailure::MovedToBackup);
        }
        let sharing = matches!(
            code,
            ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION | ERROR_UNABLE_TO_REMOVE_REPLACED
        );
        if sharing && attempt < SHARING_RETRY_MS.len() {
            thread::sleep(Duration::from_millis(SHARING_RETRY_MS[attempt]));
            attempt += 1;
            continue;
        }
        return Err(ReplaceFailure::Other(code));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 一時ファイル名は上限に収める() {
        let n = sibling_name("a.txt", "tmp");
        assert!(n.starts_with(".a.txt.eve-tmp-"), "{n}");
        let long = "あ".repeat(300);
        let n = sibling_name(&long, "bak");
        assert!(
            n.encode_utf16().count() <= NAME_LIMIT,
            "{}",
            n.encode_utf16().count()
        );
        assert!(n.contains('~') && n.contains(".eve-bak-"), "{n}");
        // 文字の途中で切らない (有効な UTF-8 のまま)
        assert!(n.chars().all(|c| c != '\u{FFFD}'));
    }

    #[test]
    fn 所有者のみの_dacl_を作れる() {
        assert!(OwnerOnly::new().is_ok());
        let sid = current_user_sid().unwrap();
        // ローカル (S-1-5-21-…)・Microsoft アカウント / Entra ID (S-1-12-1-…) のどれでも SID の形
        assert!(sid.starts_with("S-1-"), "{sid}");
    }
}

#[cfg(test)]
mod long_path_tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn 短いパスは変えない() {
        assert_eq!(extend_long(w(r"C:\a\b.txt")), w(r"C:\a\b.txt"));
        assert_eq!(
            extend_long(w(r"\\srv\share\a.txt")),
            w(r"\\srv\share\a.txt")
        );
    }

    #[test]
    fn 長いパスは拡張長形式にする() {
        let tail = r"d\".repeat(130);
        assert_eq!(
            extend_long(w(&format!(r"C:\{tail}f.txt"))),
            w(&format!(r"\\?\C:\{tail}f.txt"))
        );
        assert_eq!(
            extend_long(w(&format!(r"\\srv\share\{tail}f.txt"))),
            w(&format!(r"\\?\UNC\srv\share\{tail}f.txt"))
        );
        // `/` は `\` にする
        assert_eq!(
            extend_long(w(&format!("C:/{}f.txt", "d/".repeat(130)))),
            w(&format!(r"\\?\C:\{tail}f.txt"))
        );
    }

    #[test]
    fn すでに拡張長形式や相対パスは変えない() {
        let tail = r"d\".repeat(130);
        let v = w(&format!(r"\\?\C:\{tail}f.txt"));
        assert_eq!(extend_long(v.clone()), v);
        let r = w(&format!("{tail}f.txt"));
        assert_eq!(extend_long(r.clone()), r);
    }
}
