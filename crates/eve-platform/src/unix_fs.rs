//! Unix (Linux) のファイルシステム (docs/05 §4.1 / §4.3 / §5.1、v0.2 の M7)。
//!
//! 原子的保存の手順: 照合 → 一時ファイル (`O_EXCL`、0600) → 本文 → `fsync` → 所有者と mode の複製 (`fchown` → `fchmod` の順) →
//! xattr (`user.*`・`trusted.*`) と POSIX ACL (`system.posix_acl_access`) の複製 → 置換の直前の照合 → `rename` (新規は
//! 上書きしない `renameat2(RENAME_NOREPLACE)`) → ディレクトリの `fsync` → 事後の確認。
//!
//! 状態ディレクトリを持つ (`UnixFs::new`) と、保存中フラグ (05 §4.1 手順 0d) を書き、直接上書き (05 §4.4)・状態ファイル・
//! スワップ・緊急退避の部品 (OS に依らない実装、`state_dir_methods!`) を使う。持たない (`default()`) ときは Windows と同じく
//! フラグを書かず、状態ディレクトリの操作は `Unsupported` / 空を返す。

#![allow(unsafe_code)]

use std::collections::hash_map::RandomState;
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

use eve_core::{
    AttrKind, ChunkError, Expect, FileId, FileKind, FileMeta, FileStamp, FileSystem, FileTime,
    FsError, FsErrorDetail, FsErrorKind, FsKind, FsPath, FsyncLevel, HardlinkPolicy, LinkInfo,
    LinkKind, ReadChunks, SaveMethod, SaveOptions, SaveReport, SaveWarning,
};

/// 読込のチャンク (02 §4.5: 1 MiB)。
const READ_CHUNK_BYTES: usize = 1024 * 1024;

/// Unix のファイルシステム。`state_dir` (05 §7.5) は保存中フラグ・状態ファイル・スワップ・緊急退避の置き場所。
#[derive(Clone, Debug, Default)]
pub struct UnixFs {
    state_dir: Option<PathBuf>,
    /// 置換中の計数 (02 §9.6。終了の経路が置換の完了を待つ)。
    pub(crate) replacing: eve_core::ReplacingCount,
}

impl UnixFs {
    pub fn new(state_dir: impl Into<PathBuf>) -> UnixFs {
        UnixFs {
            state_dir: Some(state_dir.into()),
            replacing: eve_core::ReplacingCount::default(),
        }
    }

    /// Host が作った置換中の計数を使う。
    #[must_use]
    pub fn with_replacing(mut self, replacing: eve_core::ReplacingCount) -> UnixFs {
        self.replacing = replacing;
        self
    }

    pub fn state_dir(&self) -> Option<&Path> {
        self.state_dir.as_deref()
    }
}

/// OS のエラーを `FsError` に写す (05 §2.1。errno)。
pub fn map_io_error(e: io::Error, path: &FsPath) -> FsError {
    let (kind, detail) = match e.raw_os_error() {
        Some(libc::ENOENT) => {
            // 親のフォルダが無い (フォルダは作らない)
            let parent = path.parent().filter(|d| !d.as_path().is_dir());
            (
                FsErrorKind::NotFound,
                parent.map(|dir| FsErrorDetail::ParentMissing { dir }),
            )
        }
        Some(libc::EACCES | libc::EPERM) => (FsErrorKind::PermissionDenied, None),
        Some(libc::EROFS) => (FsErrorKind::ReadOnlyFs, None),
        Some(libc::ENOSPC | libc::EDQUOT) => (FsErrorKind::DiskFull, None),
        Some(libc::EBUSY | libc::ETXTBSY) => (FsErrorKind::Busy, None),
        Some(libc::ENAMETOOLONG | libc::EINVAL | libc::ELOOP | libc::ENOTDIR) => {
            (FsErrorKind::InvalidPath, None)
        }
        Some(libc::EISDIR) => (FsErrorKind::InvalidPath, Some(FsErrorDetail::NotAFile)),
        _ => match e.kind() {
            io::ErrorKind::NotFound => (FsErrorKind::NotFound, None),
            io::ErrorKind::PermissionDenied => (FsErrorKind::PermissionDenied, None),
            _ => (FsErrorKind::Other, None),
        },
    };
    FsError {
        kind,
        detail,
        source: Some(e),
    }
}

fn c_path(p: &Path) -> io::Result<CString> {
    CString::new(p.as_os_str().as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
}

/// 自分 (実効ユーザー) が書けるか (`access(W_OK)`)。
fn writable(p: &Path) -> bool {
    match c_path(p) {
        // SAFETY: NUL 終端のパス。
        Ok(c) => unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 },
        Err(_) => false,
    }
}

use crate::hooks;
use crate::inflight::FlagGuard;
use crate::sys::{euid, matches, same_file_state};

fn meta_of(p: &Path, meta: &fs::Metadata) -> FileMeta {
    let kind = if meta.is_file() {
        FileKind::File
    } else if meta.is_dir() {
        FileKind::Dir
    } else {
        FileKind::Other
    };
    let link = fs::symlink_metadata(p)
        .ok()
        .filter(|sm| sm.file_type().is_symlink())
        .map(|_| LinkInfo {
            kind: LinkKind::Symlink,
            target: fs::read_link(p).ok().map(FsPath::new),
            dangling: false,
        });
    FileMeta {
        kind,
        len: meta.len(),
        mtime: FileTime {
            secs: meta.mtime(),
            nanos: u32::try_from(meta.mtime_nsec()).unwrap_or(0),
        },
        readonly: kind == FileKind::File && !writable(p),
        file_id: Some(FileId {
            volume: meta.dev(),
            index: u128::from(meta.ino()),
        }),
        nlink: u32::try_from(meta.nlink()).unwrap_or(u32::MAX),
        link,
        fs_kind: FsKind::Unknown,
        // root は他人のファイルの所有者も保てるので「自分の」として扱う (05 §4.4)
        owner_is_me: Some(meta.uid() == euid() || euid() == 0),
    }
}

/// 保存先 T: シンボリックリンクなら最終リンク先 (ダングリングは `NotFound` + `DanglingLink`)。`create` ならダングリングの
/// リンクのリンク先を T にする (05 §4.1)。リンク先もリンク (連鎖) なら `InvalidPath`。
pub(crate) fn resolve_target(path: &FsPath, create: bool) -> Result<PathBuf, FsError> {
    let target = path.as_path().to_path_buf();
    let Ok(sm) = fs::symlink_metadata(&target) else {
        return Ok(target);
    };
    if !sm.file_type().is_symlink() {
        return Ok(target);
    }
    match fs::canonicalize(&target) {
        Ok(t) => Ok(t),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let raw = fs::read_link(&target).ok();
            if create && let Some(raw) = &raw {
                let t = match target.parent() {
                    Some(dir) if raw.is_relative() => dir.join(raw),
                    _ => raw.clone(),
                };
                if fs::symlink_metadata(&t).is_ok() {
                    return Err(FsError::new(FsErrorKind::InvalidPath));
                }
                return Ok(t);
            }
            Err(FsError {
                kind: FsErrorKind::NotFound,
                detail: raw.map(|t| FsErrorDetail::DanglingLink {
                    target: FsPath::new(t),
                }),
                source: Some(e),
            })
        }
        Err(e) => Err(map_io_error(e, path)),
    }
}

/// 一時ファイルの名前 (`.<name>.eve-tmp-<pid>-<rand>`。05 §4.6 の痕跡の名前と同じ形)。
fn sibling_name(name: &str, kind: &str) -> String {
    let mut h = RandomState::new().build_hasher();
    h.write_u32(std::process::id());
    let r = h.finish() as u32;
    let suffix = format!(".eve-{kind}-{}-{r:08x}", std::process::id());
    // NAME_MAX (255 バイト) に収める
    let room = 255usize.saturating_sub(1 + suffix.len());
    let mut cut = String::new();
    for c in name.chars() {
        if cut.len() + c.len_utf8() > room {
            break;
        }
        cut.push(c);
    }
    format!(".{cut}{suffix}")
}

/// 複製する xattr の名前か (05 §4.3 / §5.1: `user.*` と `trusted.*`。`security.*` と `system.*` は OS に任せる。
/// POSIX ACL `system.posix_acl_access` は別に扱う)。
fn copy_xattr_name(name: &[u8]) -> bool {
    name.starts_with(b"user.") || name.starts_with(b"trusted.")
}

const ACL_ACCESS: &[u8] = b"system.posix_acl_access";
/// ディレクトリの既定 ACL (一時ファイルが継承する)。
const ACL_DEFAULT: &[u8] = b"system.posix_acl_default";

/// `fd` の xattr の名前の一覧。対応しない FS (ENOTSUP) は空。
fn list_xattrs(fd: i32) -> io::Result<Vec<Vec<u8>>> {
    // SAFETY: 長さを問い合わせるだけ (バッファなし)。
    let n = unsafe { libc::flistxattr(fd, std::ptr::null_mut(), 0) };
    if n < 0 {
        let e = io::Error::last_os_error();
        return if e.raw_os_error() == Some(libc::ENOTSUP) {
            Ok(Vec::new())
        } else {
            Err(e)
        };
    }
    let mut buf = vec![0u8; n as usize];
    // SAFETY: `buf` は `n` バイト。
    let n = unsafe { libc::flistxattr(fd, buf.as_mut_ptr().cast(), buf.len()) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    buf.truncate(n as usize);
    Ok(buf
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(<[u8]>::to_vec)
        .collect())
}

fn get_xattr(fd: i32, name: &[u8]) -> io::Result<Vec<u8>> {
    let c = CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: NUL 終端の名前、長さの問い合わせ。
    let n = unsafe { libc::fgetxattr(fd, c.as_ptr(), std::ptr::null_mut(), 0) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buf = vec![0u8; n as usize];
    // SAFETY: `buf` は `n` バイト。
    let n = unsafe { libc::fgetxattr(fd, c.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    buf.truncate(n as usize);
    Ok(buf)
}

fn set_xattr(fd: i32, name: &[u8], value: &[u8]) -> io::Result<()> {
    let c = CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: NUL 終端の名前と、長さ付きの値。
    let r = unsafe { libc::fsetxattr(fd, c.as_ptr(), value.as_ptr().cast(), value.len(), 0) };
    if r != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// T の xattr と POSIX ACL を tmp に写す。写せなかった種類を返す。
fn copy_xattrs(src: &File, dst: &File) -> Vec<AttrKind> {
    let mut lost = Vec::new();
    let names = match list_xattrs(src.as_raw_fd()) {
        Ok(n) => n,
        Err(_) => return vec![AttrKind::StreamsXattr],
    };
    for name in names {
        let kind = if name == ACL_ACCESS {
            AttrKind::Security
        } else if copy_xattr_name(&name) {
            AttrKind::StreamsXattr
        } else {
            continue;
        };
        let ok = get_xattr(src.as_raw_fd(), &name)
            .and_then(|v| set_xattr(dst.as_raw_fd(), &name, &v))
            .is_ok();
        if !ok && !lost.contains(&kind) {
            lost.push(kind);
        }
    }
    lost
}

/// 上書きしない移動 (`renameat2(RENAME_NOREPLACE)`。使えない FS ではハードリンク + 削除)。在れば `AlreadyExists`。
/// libc の関数ではなくシステムコールで呼ぶ (musl 1.2.5 (Rust の musl ターゲットが同梱する版) には `renameat2` の関数が無い)。
fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    let (f, t) = (c_path(from)?, c_path(to)?);
    // SAFETY: NUL 終端のパス 2 つ。`AT_FDCWD` は相対パスを作業ディレクトリから解く。引数はカーネルの renameat2 と同じ並び
    let r = unsafe {
        // 可変長の引数は long として渡す (libc の syscall の作法)
        libc::syscall(
            libc::SYS_renameat2,
            libc::c_long::from(libc::AT_FDCWD),
            f.as_ptr(),
            libc::c_long::from(libc::AT_FDCWD),
            t.as_ptr(),
            libc::c_long::from(libc::RENAME_NOREPLACE),
        )
    };
    if r == 0 {
        return Ok(());
    }
    let e = io::Error::last_os_error();
    match e.raw_os_error() {
        Some(libc::ENOSYS | libc::EINVAL) => {
            fs::hard_link(from, to)?;
            fs::remove_file(from)
        }
        _ => Err(e),
    }
}

fn fsync_dir(dir: &Path) -> bool {
    File::open(dir).and_then(|d| d.sync_all()).is_ok()
}

struct FileChunks {
    file: File,
    buf: Vec<u8>,
    path: FsPath,
}

impl ReadChunks for FileChunks {
    fn next_chunk(&mut self) -> Result<Option<&[u8]>, ChunkError> {
        loop {
            match self.file.read(&mut self.buf) {
                Ok(0) => return Ok(None),
                Ok(n) => return Ok(Some(&self.buf[..n])),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(ChunkError::Io(map_io_error(e, &self.path))),
            }
        }
    }
}

impl FileSystem for UnixFs {
    fn metadata(&self, path: &FsPath) -> Result<FileMeta, FsError> {
        let p = path.as_path();
        match fs::metadata(p) {
            Ok(m) => Ok(meta_of(p, &m)),
            Err(e) => {
                // リンクは在るがリンク先が無い (ダングリング)
                if e.kind() == io::ErrorKind::NotFound
                    && let Ok(sm) = fs::symlink_metadata(p)
                    && sm.file_type().is_symlink()
                {
                    let target = fs::read_link(p).ok().map(FsPath::new);
                    return Err(FsError {
                        kind: FsErrorKind::NotFound,
                        detail: target.map(|target| FsErrorDetail::DanglingLink { target }),
                        source: Some(e),
                    });
                }
                Err(map_io_error(e, path))
            }
        }
    }

    fn canonicalize(&self, path: &FsPath) -> Result<FsPath, FsError> {
        fs::canonicalize(path.as_path())
            .map(FsPath::new)
            .map_err(|e| map_io_error(e, path))
    }

    fn open_read(&self, path: &FsPath) -> Result<Box<dyn ReadChunks + Send>, FsError> {
        let f = File::open(path.as_path()).map_err(|e| map_io_error(e, path))?;
        if let Ok(m) = f.metadata()
            && !m.is_file()
        {
            return Err(FsError::with_detail(
                FsErrorKind::InvalidPath,
                FsErrorDetail::NotAFile,
            ));
        }
        Ok(Box::new(FileChunks {
            file: f,
            buf: vec![0u8; READ_CHUNK_BYTES],
            path: path.clone(),
        }))
    }

    fn rename_no_replace(&self, from: &FsPath, to: &FsPath) -> Result<(), FsError> {
        rename_no_replace(from.as_path(), to.as_path()).map_err(|e| map_io_error(e, to))
    }

    state_dir_methods!();

    fn remove_file(&self, path: &FsPath) -> Result<(), FsError> {
        fs::remove_file(path.as_path()).map_err(|e| map_io_error(e, path))
    }

    fn write_atomic(
        &self,
        path: &FsPath,
        data: &mut dyn ReadChunks,
        opts: &SaveOptions,
    ) -> Result<SaveReport, FsError> {
        let conflict = |current| FsError::new(FsErrorKind::Conflict { current });

        // 0a. シンボリックリンクなら最終リンク先 T を対象にする
        let target = resolve_target(path, opts.create_link_target)?;
        let target_path = FsPath::new(target.clone());
        let parent = target
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
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

        // 0b. 書き始める前の照合
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
        if let Some(m) = &existing
            && m.nlink > 1
            && opts.hardlink != HardlinkPolicy::BreakLink
        {
            return Err(FsError::with_detail(
                FsErrorKind::AtomicUnsupported,
                FsErrorDetail::Hardlink { nlink: m.nlink },
            ));
        }
        // 書込権限の無い T: rename は置き換えてしまうので、「読み取り専用を解除して保存」(FR-116) の確認を経たときだけ置換する
        // (mode は下で T のものを写すので、解除は置換の間だけになる。Windows の手順 5b と同じ結果)
        if existing.as_ref().is_some_and(|m| m.readonly) && !opts.clear_readonly && euid() != 0 {
            return Err(FsError::new(FsErrorKind::PermissionDenied));
        }
        let orig_meta = existing.as_ref().and_then(|_| fs::metadata(&target).ok());

        // 1. 一時ファイル (`O_EXCL`)。T が在る・`private` なら 0600 で作り、下で T の mode に合わせる。
        // 新規は 0666 (umask が掛かる。通常の新規ファイルと同じ)
        let tmp: PathBuf = parent.join(sibling_name(&name, "tmp"));
        let mut warnings = Vec::new();
        // 0d. 保存中フラグ (一時ファイルを作る前。rename は退避名を作らないので bak は無い)
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
            None,
            &mut warnings,
        );
        let mode = if existing.is_some() || opts.private {
            0o600
        } else {
            0o666
        };
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .custom_flags(libc::O_CLOEXEC)
            .open(&tmp)
        {
            Ok(f) => f,
            // 親に書込権限がない (一時ファイルを作れない): 確認付きの直接上書きへ (05 §4.4)
            Err(e) if e.raw_os_error() == Some(libc::EACCES) => {
                flag.remove();
                return Err(FsError::new(FsErrorKind::AtomicUnsupported));
            }
            Err(e) => {
                flag.remove();
                return Err(map_io_error(e, path));
            }
        };
        hooks::hit("kill_after_tmp_alloc");
        let abort = |file: Option<File>, flag: &mut FlagGuard| {
            drop(file);
            let _ = fs::remove_file(&tmp);
            flag.remove();
        };

        // 2. 本文
        loop {
            match data.next_chunk() {
                Ok(Some(chunk)) => {
                    if let Err(e) = file.write_all(chunk) {
                        abort(Some(file), &mut flag);
                        return Err(map_io_error(e, path));
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    abort(Some(file), &mut flag);
                    return Err(match e {
                        ChunkError::Io(err) => err,
                        _ => FsError::new(FsErrorKind::SourceFailed),
                    });
                }
            }
        }

        hooks::hit("kill_after_tmp_write");

        // 3. fsync (失敗は警告。ext4 の data=ordered でも rename の前の fsync は要る)
        if opts.fsync != FsyncLevel::None && file.sync_all().is_err() {
            warnings.push(SaveWarning::FsyncFailed);
        }

        // 4. 所有者と mode (fchown → fchmod の順。setuid / setgid は保たない)、xattr と POSIX ACL
        if let Some(om) = &orig_meta {
            let mut lost: Vec<AttrKind> = Vec::new();
            let mut lose = |k: AttrKind| {
                if !lost.contains(&k) {
                    lost.push(k);
                }
            };
            if opts.preserve.security {
                let (uid, gid) = (om.uid(), om.gid());
                if uid != euid() || gid != file.metadata().map_or(gid, |m| m.gid()) {
                    // 他人の所有を自分が引き継げない (EPERM) ときは、グループだけでも合わせて警告する
                    if std::os::unix::fs::fchown(&file, Some(uid), Some(gid)).is_err() {
                        let _ = std::os::unix::fs::fchown(&file, None, Some(gid));
                        lose(AttrKind::Security);
                    }
                }
                let m = om.permissions().mode();
                let chmod_failed = file
                    .set_permissions(fs::Permissions::from_mode(m & 0o1777))
                    .is_err();
                if chmod_failed || m & 0o6000 != 0 {
                    lose(AttrKind::Security);
                }
            }
            if (opts.preserve.streams_xattr || opts.preserve.security)
                && let Ok(src) = File::open(&target)
            {
                copy_xattrs(&src, &file).into_iter().for_each(&mut lose);
                // T に ACL が無いのに親の既定 ACL を tmp が継承すると、元と違う相手に読める (05 §4.3)
                if opts.preserve.security
                    && get_xattr(src.as_raw_fd(), ACL_ACCESS).is_err()
                    && File::open(&parent)
                        .is_ok_and(|d| get_xattr(d.as_raw_fd(), ACL_DEFAULT).is_ok())
                {
                    lose(AttrKind::Security);
                }
            }
            if !lost.is_empty() {
                warnings.push(SaveWarning::AttrsNotPreserved { what: lost });
            }
        }
        let tmp_meta = file.metadata().ok().map(|m| meta_of(&tmp, &m));
        drop(file);
        hooks::hit("kill_after_flush");

        // 4b. 置換の直前の照合
        if opts.expected != Expect::Any {
            let now = match self.metadata(&target_path) {
                Ok(m) => Some(m),
                Err(e) if e.kind == FsErrorKind::NotFound => None,
                Err(e) => {
                    abort(None, &mut flag);
                    return Err(e);
                }
            };
            let stale = match (&existing, &now) {
                (Some(a), Some(b)) => !same_file_state(a, b),
                (None, None) => false,
                _ => true,
            };
            if stale {
                abort(None, &mut flag);
                return Err(conflict(
                    now.as_ref().map(|m| FileStamp::from_meta(m, None)),
                ));
            }
        }

        hooks::hit("kill_before_rename");

        // 5. 置換 (終了の経路はここが済むまで待つ)
        let _replacing = self.replacing.guard(&target_path.display());
        let method = if existing.is_some() {
            if let Err(e) = fs::rename(&tmp, &target) {
                abort(None, &mut flag);
                return Err(map_io_error(e, path));
            }
            SaveMethod::Atomic
        } else {
            if let Err(e) = rename_no_replace(&tmp, &target) {
                abort(None, &mut flag);
                if e.kind() == io::ErrorKind::AlreadyExists {
                    let cur = self
                        .metadata(&target_path)
                        .ok()
                        .map(|m| FileStamp::from_meta(&m, None));
                    return Err(conflict(cur));
                }
                return Err(map_io_error(e, path));
            }
            SaveMethod::NewFile
        };
        // 6. ディレクトリの fsync (失敗は警告)
        if opts.fsync != FsyncLevel::None && !fsync_dir(&parent) {
            warnings.push(SaveWarning::FsyncFailed);
        }

        // 7. 事後: 置換後の T が自分の書いたものか (FileId) を確かめて new_stamp にする
        let after = self.metadata(&target_path).ok();
        let new_stamp = match (&after, &tmp_meta) {
            (Some(a), Some(t)) if a.file_id == t.file_id => FileStamp::from_meta(a, None),
            (Some(_), Some(t)) => {
                warnings.push(SaveWarning::ChangedAfterSave);
                FileStamp::from_meta(t, None)
            }
            (Some(a), None) => FileStamp::from_meta(a, None),
            (None, _) => return Err(FsError::new(FsErrorKind::Other)),
        };
        // 8. 成功: フラグ (と同じキーに残る他の記録) を消す
        flag.finish_success();
        Ok(SaveReport {
            method,
            new_stamp,
            warnings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eve_core::{Expect, SaveOptions};

    struct Tmp(PathBuf);

    impl Tmp {
        fn new(tag: &str) -> Tmp {
            let d = std::env::temp_dir().join(format!("eve-unixfs-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&d);
            fs::create_dir_all(&d).unwrap();
            Tmp(d)
        }
        fn p(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct Bytes<'a>(Option<&'a [u8]>);

    impl ReadChunks for Bytes<'_> {
        fn next_chunk(&mut self) -> Result<Option<&[u8]>, ChunkError> {
            Ok(self.0.take())
        }
    }

    fn save(fs_: &UnixFs, p: &Path, body: &[u8], expected: Expect) -> Result<SaveReport, FsError> {
        fs_.write_atomic(
            &FsPath::new(p.to_path_buf()),
            &mut Bytes(Some(body)),
            &SaveOptions::user_file(expected),
        )
    }

    fn leftovers(t: &Tmp) -> Vec<String> {
        fs::read_dir(&t.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".eve-"))
            .collect()
    }

    // verifies: NFR-401
    #[test]
    fn 新規と上書きの保存で内容と_mode_を保ち_一時ファイルを残さない() {
        let t = Tmp::new("save");
        let ufs = UnixFs::default();
        let a = t.p("a.txt");
        let r = save(&ufs, &a, b"one\n", Expect::Absent).unwrap();
        assert_eq!(r.method, SaveMethod::NewFile);
        assert_eq!(fs::read(&a).unwrap(), b"one\n");
        fs::set_permissions(&a, fs::Permissions::from_mode(0o640)).unwrap();
        let stamp = FileStamp::from_meta(&ufs.metadata(&FsPath::new(a.clone())).unwrap(), None);
        let r = save(&ufs, &a, b"two\n", Expect::Stamp(stamp)).unwrap();
        assert_eq!(r.method, SaveMethod::Atomic);
        assert_eq!(fs::read(&a).unwrap(), b"two\n");
        assert_eq!(
            fs::metadata(&a).unwrap().permissions().mode() & 0o7777,
            0o640
        );
        assert_eq!(
            r.new_stamp.file_id,
            ufs.metadata(&FsPath::new(a.clone())).unwrap().file_id
        );
        assert!(leftovers(&t).is_empty(), "{:?}", leftovers(&t));
    }

    #[test]
    fn setuid_は保たずに警告する() {
        let t = Tmp::new("setuid");
        let ufs = UnixFs::default();
        let a = t.p("a.sh");
        fs::write(
            &a,
            b"#!/bin/sh
",
        )
        .unwrap();
        fs::set_permissions(&a, fs::Permissions::from_mode(0o4755)).unwrap();
        let r = save(
            &ufs,
            &a,
            b"#!/bin/sh
true
",
            Expect::Any,
        )
        .unwrap();
        assert_eq!(
            fs::metadata(&a).unwrap().permissions().mode() & 0o7777,
            0o755
        );
        assert!(r.warnings.iter().any(|w| matches!(
            w,
            SaveWarning::AttrsNotPreserved { what } if what.contains(&AttrKind::Security)
        )));
    }

    #[test]
    fn 書込権限の無いファイルは解除の確認を経たときだけ置換し_mode_を保つ() {
        if euid() == 0 {
            return; // root は書込権限に縛られない
        }
        let t = Tmp::new("readonly");
        let ufs = UnixFs::default();
        let a = t.p("a.txt");
        fs::write(&a, b"orig").unwrap();
        fs::set_permissions(&a, fs::Permissions::from_mode(0o444)).unwrap();
        assert!(ufs.metadata(&FsPath::new(a.clone())).unwrap().readonly);
        let e = save(&ufs, &a, b"x", Expect::Any).unwrap_err();
        assert_eq!(e.kind, FsErrorKind::PermissionDenied);
        assert_eq!(fs::read(&a).unwrap(), b"orig");
        let mut opts = SaveOptions::user_file(Expect::Any);
        opts.clear_readonly = true;
        ufs.write_atomic(&FsPath::new(a.clone()), &mut Bytes(Some(b"x")), &opts)
            .unwrap();
        assert_eq!(fs::read(&a).unwrap(), b"x");
        assert_eq!(
            fs::metadata(&a).unwrap().permissions().mode() & 0o7777,
            0o444
        );
        assert!(leftovers(&t).is_empty());
    }

    #[test]
    fn 照合が合わなければ何も書かず_conflict() {
        let t = Tmp::new("conflict");
        let ufs = UnixFs::default();
        let a = t.p("a.txt");
        fs::write(&a, b"orig").unwrap();
        let e = save(&ufs, &a, b"x", Expect::Absent).unwrap_err();
        assert!(matches!(e.kind, FsErrorKind::Conflict { current: Some(_) }));
        let mut stale = FileStamp::from_meta(&ufs.metadata(&FsPath::new(a.clone())).unwrap(), None);
        stale.len += 1;
        let e = save(&ufs, &a, b"x", Expect::Stamp(stale)).unwrap_err();
        assert!(matches!(e.kind, FsErrorKind::Conflict { .. }));
        assert_eq!(fs::read(&a).unwrap(), b"orig");
        assert!(leftovers(&t).is_empty());
    }

    #[test]
    fn シンボリックリンクはリンク先を更新しリンクは保つ_ダングリングは理由つき() {
        let t = Tmp::new("link");
        let ufs = UnixFs::default();
        let real = t.p("real.txt");
        fs::write(&real, b"r").unwrap();
        let link = t.p("link.txt");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        save(&ufs, &link, b"new", Expect::Any).unwrap();
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&real).unwrap(), b"new");
        let dangling = t.p("d.txt");
        std::os::unix::fs::symlink(t.p("missing.txt"), &dangling).unwrap();
        let e = save(&ufs, &dangling, b"x", Expect::Any).unwrap_err();
        assert!(matches!(e.detail, Some(FsErrorDetail::DanglingLink { .. })));
    }

    #[test]
    fn ハードリンクは既定で原子的置換をしない() {
        let t = Tmp::new("hard");
        let ufs = UnixFs::default();
        let a = t.p("a.txt");
        fs::write(&a, b"a").unwrap();
        fs::hard_link(&a, t.p("b.txt")).unwrap();
        let e = save(&ufs, &a, b"x", Expect::Any).unwrap_err();
        assert_eq!(e.kind, FsErrorKind::AtomicUnsupported);
        assert!(matches!(
            e.detail,
            Some(FsErrorDetail::Hardlink { nlink: 2 })
        ));
    }

    #[test]
    fn user_の_xattr_を保つ() {
        let t = Tmp::new("xattr");
        let ufs = UnixFs::default();
        let a = t.p("a.txt");
        fs::write(&a, b"a").unwrap();
        let f = File::open(&a).unwrap();
        if set_xattr(f.as_raw_fd(), b"user.eve_test", b"v1").is_err() {
            // この FS は user xattr に対応しない: 試験を省く
            return;
        }
        drop(f);
        let r = save(&ufs, &a, b"b", Expect::Any).unwrap();
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        let f = File::open(&a).unwrap();
        assert_eq!(get_xattr(f.as_raw_fd(), b"user.eve_test").unwrap(), b"v1");
    }

    #[test]
    fn 親が無ければ_parent_missing_で_errno_を写す() {
        let t = Tmp::new("parent");
        let ufs = UnixFs::default();
        let e = save(&ufs, &t.p("no/such/a.txt"), b"x", Expect::Any).unwrap_err();
        assert!(matches!(
            e.detail,
            Some(FsErrorDetail::ParentMissing { .. })
        ));
        let e = map_io_error(
            io::Error::from_raw_os_error(libc::ENOSPC),
            &FsPath::new(t.p("x")),
        );
        assert_eq!(e.kind, FsErrorKind::DiskFull);
    }
}
