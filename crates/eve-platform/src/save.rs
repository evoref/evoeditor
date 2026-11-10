//! 原子的保存 `write_atomic` の Windows 実装 (docs/05 §4.1 の共通手順と §4.2、§5.6)。
//!
//! 順序: 0 事前検査 (リンクの解決・親・照合・ハードリンク) → 1 一時ファイル (所有者のみの DACL) → 2 チャンク書込 → 3 fsync・ハンドルを閉じる
//! → 4 属性 (HIDDEN / SYSTEM) の複製 → 4b 置換の直前の照合 → 5 `ReplaceFileW` (新規ファイルは `MoveFileExW`) → 7 事後 (`FileId` の確認・退避名の削除)。
//! 失敗では一時ファイルを消し、元のファイルは無傷に保つ。置換は `ReplaceFileW` だけで、フラグは 0 (ACL・ADS・作成日時は
//! `ReplaceFileW` が保つ)。置換に失敗して T が無傷なら `AtomicUnsupported` (確認付きの直接上書きは core 側)。
//!
//! 直接上書き (`write_in_place`) は `inplace` にある。

use std::collections::hash_map::RandomState;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use eve_core::{
    AttrKind, ChunkError, Expect, FileKind, FileMeta, FileStamp, FileSystem, FsError,
    FsErrorDetail, FsErrorKind, FsPath, FsyncLevel, HardlinkPolicy, ReadChunks, SaveMethod,
    SaveOptions, SaveReport, SaveWarning,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_SYSTEM,
};

use crate::fs::{WinFs, map_io_error};
use crate::hooks;
use crate::inflight::FlagGuard;
use crate::path::simplify_verbatim;
use crate::sys::{
    OwnerOnly, attributes, create_new, delete, flush, matches, move_no_replace, replace_file_once,
    same_file_state, set_attributes,
};

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

pub(crate) fn os_error(code: u32, path: &FsPath) -> FsError {
    // Win32 のエラー番号 (DWORD) を、io::Error の i32 にビットのまま入れる
    map_io_error(io::Error::from_raw_os_error(code.cast_signed()), path)
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
    // 乱数としてハッシュの下位 32 ビットを使う (上位は捨ててよい)
    (h.finish() & u64::from(u32::MAX))
        .try_into()
        .unwrap_or_default()
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

/// 置換の前の失敗の後片付け: 一時ファイルを消せたらフラグも消す (消せなければ残す。05 §4.1 手順 8)。
fn abort(tmp: &Path, flag: &mut FlagGuard) {
    if delete(tmp) {
        flag.remove();
    } else {
        // 消せなければフラグを残し、名前をログに残す (05 §4.1 手順 8)。既に無いときは書かない
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::NotFound {
            flag.log_leftover_tmp(tmp, &e);
        }
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
            Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED.cast_signed()) => {
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
                        err.source = Some(io::Error::from_raw_os_error(code.cast_signed()));
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
            if let Err(code) = move_no_replace(&tmp, &target) {
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
    let mut attempt = 0;
    loop {
        let Err(code) = replace_file_once(target, tmp, backup) else {
            return Ok(());
        };
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
}
