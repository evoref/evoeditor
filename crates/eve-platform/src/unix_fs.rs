//! Unix (Linux) のファイルシステム (docs/05 §4.1 / §4.3 / §5.1、v0.2)。
//!
//! 原子的保存の手順: 照合 → 一時ファイル (`O_EXCL`、0600) → 本文 → `fsync` → 所有者と mode の複製 (`fchown` → `fchmod` の順) →
//! xattr (`user.*`・`trusted.*`) と POSIX ACL (`system.posix_acl_access`) の複製 → 置換の直前の照合 → `rename` (新規は
//! 上書きしない `renameat2(RENAME_NOREPLACE)`) → ディレクトリの `fsync` → 事後の確認。
//!
//! 状態ディレクトリを持つ (`UnixFs::new`) と、保存中フラグ (05 §4.1 手順 0d) を書き、直接上書き (05 §4.4)・状態ファイル・
//! スワップ・緊急退避の部品 (OS に依らない実装、`state_dir_methods!`) を使う。持たない (`default()`) ときは Windows と同じく
//! フラグを書かず、状態ディレクトリの操作は `Unsupported` / 空を返す。

use std::collections::hash_map::RandomState;
use std::fs::{self, File, OpenOptions};
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use eve_core::{
    AttrKind, ChunkError, Expect, FileId, FileKind, FileMeta, FileStamp, FileSystem, FileTime,
    FsError, FsErrorDetail, FsErrorKind, FsKind, FsPath, FsyncLevel, HardlinkPolicy, ReadChunks,
    SaveMethod, SaveOptions, SaveReport, SaveWarning,
};

use crate::fs_common::{file_kind, link_info, metadata_error};

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

use crate::hooks;
use crate::inflight::FlagGuard;
use crate::sys::{
    can_write, euid, fsync, get_xattr, list_xattrs, matches, rename_noreplace, same_file_state,
    set_xattr,
};

fn meta_of(p: &Path, meta: &fs::Metadata) -> FileMeta {
    let kind = file_kind(meta);
    FileMeta {
        kind,
        len: meta.len(),
        mtime: FileTime {
            secs: meta.mtime(),
            nanos: u32::try_from(meta.mtime_nsec()).unwrap_or(0),
        },
        readonly: kind == FileKind::File && !can_write(p),
        file_id: Some(FileId {
            volume: meta.dev(),
            index: u128::from(meta.ino()),
        }),
        nlink: u32::try_from(meta.nlink()).unwrap_or(u32::MAX),
        link: link_info(p),
        fs_kind: FsKind::Unknown,
        // root は他人のファイルの所有者も保てるので「自分の」として扱う (05 §4.4)
        owner_is_me: Some(meta.uid() == my_euid() || my_euid() == 0),
    }
}

/// 実効 uid (単体試験では `hooks::pretend_uid("euid", ..)` で差し替えられる。SAF-36 / 37 の分岐の試験)。
fn my_euid() -> u32 {
    hooks::uid_override("euid").unwrap_or_else(euid)
}

/// 一時ファイルの所有者 (fstat の st_uid。単体試験では `hooks::pretend_uid("tmp_uid", ..)` で差し替えられる)。
fn tmp_owner(f: &File) -> Option<u32> {
    hooks::uid_override("tmp_uid").or_else(|| f.metadata().ok().map(|m| m.uid()))
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
    // 乱数としてハッシュの下位 32 ビットを使う (上位は捨ててよい)
    let r: u32 = (h.finish() & u64::from(u32::MAX))
        .try_into()
        .unwrap_or_default();
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

/// T の xattr と POSIX ACL を tmp に写す。写せなかった種類を返す。
/// `with_acl` が偽なら、アクセス ACL (`system.posix_acl_access`) は写さずに `Security` を失ったとする
/// (グループを合わせられないとき。写すと、ACL が mode のグループのビット (mask) と other のビットを
/// 書き直し、元の mode を別のグループに当てたのと同じになる。05 §4.1 手順 4)。
fn copy_xattrs(src: &File, dst: &File, with_acl: bool) -> Vec<AttrKind> {
    let mut lost = Vec::new();
    let names = match list_xattrs(src) {
        Ok(n) => n,
        Err(_) => return vec![AttrKind::StreamsXattr],
    };
    for name in names {
        let kind = if name == ACL_ACCESS {
            if !with_acl {
                if !lost.contains(&AttrKind::Security) {
                    lost.push(AttrKind::Security);
                }
                continue;
            }
            AttrKind::Security
        } else if copy_xattr_name(&name) {
            AttrKind::StreamsXattr
        } else {
            continue;
        };
        let ok = get_xattr(src, &name)
            .and_then(|v| set_xattr(dst, &name, &v))
            .is_ok();
        if !ok && !lost.contains(&kind) {
            lost.push(kind);
        }
    }
    lost
}

/// 上書きしない移動 (`renameat2(RENAME_NOREPLACE)`。使えない FS ではハードリンク + 削除)。在れば `AlreadyExists`。
/// Linux 以外では `renameat2` を使わない (`ENOSYS` として代替の経路へ。`sys::unix`)。
/// 退路で link が成功した後に `from` を消せなかったら `Ok(Some(e))` (`to` は完成している。`from` は `to` のハードリンクとして残る。
/// 05 §4.1「新規ファイル」、docs/review/v0.3-findings.md の SAF-40)。
fn rename_no_replace(from: &Path, to: &Path) -> io::Result<Option<io::Error>> {
    let e = match hooked("rename_noreplace", || rename_noreplace(from, to)) {
        Ok(()) => return Ok(None),
        Err(e) => e,
    };
    match e.raw_os_error() {
        // 上書きしない移動を使えない FS: ENOSYS (古いカーネル) と EINVAL (FUSE の ntfs-3g など) に加えて、
        // ENOTSUP / EOPNOTSUPP を返す FS もある (docs/review/v0.3-findings.md の REF-07)
        Some(c) if [libc::ENOSYS, libc::EINVAL, libc::ENOTSUP, libc::EOPNOTSUPP].contains(&c) => {
            hooked("link", || fs::hard_link(from, to))?;
            Ok(hooked("unlink_after_link", || fs::remove_file(from)).err())
        }
        _ => Err(e),
    }
}

/// 既存の T を置き換える rename の失敗の写像。FUSE・特殊な FS で rename が `EXDEV` / `ENOTSUP` で、取り直した T が無傷
/// (`intact`) なら、原子的な置換ができないだけなので `AtomicUnsupported` にする (core が確認の後に直接上書きへ進む、
/// 05 §4.4 の表。docs/review/v0.3-findings.md の SAF-23)。1 ファイルだけの bind mount の `EBUSY` も同じ (SAF-38)。
/// 新規ファイル (T が無い) には使わない (直接上書きは T があるときだけ)。
/// それ以外は `map_io_error` (他の操作の `EXDEV` は 05 §4.4 の表のとおり `Other`)。
fn rename_error(e: io::Error, path: &FsPath, intact: bool) -> FsError {
    match e.raw_os_error() {
        Some(c)
            if intact
                && [libc::EXDEV, libc::ENOTSUP, libc::EOPNOTSUPP, libc::EBUSY].contains(&c) =>
        {
            FsError {
                kind: FsErrorKind::AtomicUnsupported,
                detail: None,
                source: Some(e),
            }
        }
        _ => map_io_error(e, path),
    }
}

/// システムコールの呼び出し `f` の前に、試験の差し替え (`hooks::fail`) を見る。配布ビルドでは `f()` そのもの。
fn hooked<T>(point: &str, f: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    match hooks::fail(point) {
        Some(e) => Err(e),
        None => f(),
    }
}

/// 一時ファイルの fsync の失敗のうち、FS が受け付けないだけのもの (警告で続ける。05 §4.1 手順 3、SAF-46)。
fn fsync_unsupported(e: &io::Error) -> bool {
    e.raw_os_error()
        .is_some_and(|c| [libc::EINVAL, libc::ENOTSUP, libc::EOPNOTSUPP, libc::ENOSYS].contains(&c))
}

fn fsync_dir(dir: &Path) -> bool {
    hooked("fsync_dir", || File::open(dir).and_then(|d| fsync(&d))).is_ok()
}

impl FileSystem for UnixFs {
    fn metadata(&self, path: &FsPath) -> Result<FileMeta, FsError> {
        let p = path.as_path();
        fs::metadata(p)
            .map(|m| meta_of(p, &m))
            .map_err(|e| metadata_error(e, path))
    }

    fn canonicalize(&self, path: &FsPath) -> Result<FsPath, FsError> {
        fs::canonicalize(path.as_path())
            .map(FsPath::new)
            .map_err(|e| map_io_error(e, path))
    }

    fn open_read(&self, path: &FsPath) -> Result<Box<dyn ReadChunks + Send>, FsError> {
        crate::fs_common::open_read(path)
    }

    fn rename_no_replace(&self, from: &FsPath, to: &FsPath) -> Result<(), FsError> {
        // link の後に from を消せなかったときは従来どおり失敗として返す (呼び側は from が残っているのを前提にできる)
        match rename_no_replace(from.as_path(), to.as_path()) {
            Ok(None) => Ok(()),
            Ok(Some(e)) | Err(e) => Err(map_io_error(e, to)),
        }
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
        if existing.as_ref().is_some_and(|m| m.readonly) && !opts.clear_readonly && my_euid() != 0 {
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
        // 中断の後片付け: 一時ファイルを消せた (または既に無い) ときだけ保存中フラグを消す。消せなければフラグを残し、
        // 次の起動の走査が一時ファイルを見つけられるようにする (05 §4.1 手順 8、§4.6 の「記録の一時ファイルが実在しない
        // 記録は消す」。Windows の save.rs は不在でもフラグを残す。docs/review/v0.3-findings.md の SAF-10)
        let abort = |file: Option<File>, flag: &mut FlagGuard| {
            drop(file);
            match hooked("unlink_tmp", || fs::remove_file(&tmp)) {
                // 消せなければフラグを残し、名前をログに残す (05 §4.1 手順 8)
                Err(e) if e.kind() != io::ErrorKind::NotFound => flag.log_leftover_tmp(&tmp, &e),
                _ => flag.remove(),
            }
        };

        // 1b. 他ユーザー所有の判定 (本文を書く前): 一時ファイルの所有者が T と違えば、置換すると所有者が変わる。
        // root (所有者を写す preserve.security のとき) は、ここで fchown(uid, gid) して fstat で確かめる (SAF-37。root_squash・
        // 名前空間に写っていない uid・黙って何もしない FS では合わない)。合わなければ、または root でなければ、空の一時ファイルと
        // フラグを消して確認付きの直接上書きへ (本文を他人の所有のファイルに書かない)。uid を固定・写像して見せる FS
        // (sshfs・CIFS・all_squash 等) では同じ uid に見えるので置換を続ける (05 §4.1 手順 1b・§4.4 の表。
        // docs/review/v0.3-findings.md の SAF-36)
        if let Some(om) = &orig_meta
            && tmp_owner(&file) != Some(om.uid())
        {
            let fixed = my_euid() == 0
                && opts.preserve.security
                && hooked("fchown", || {
                    std::os::unix::fs::fchown(&file, Some(om.uid()), Some(om.gid()))
                })
                .is_ok()
                && tmp_owner(&file) == Some(om.uid());
            if !fixed {
                abort(Some(file), &mut flag);
                return Err(FsError::new(FsErrorKind::AtomicUnsupported));
            }
        }

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

        // 3. fsync (ext4 の data=ordered でも rename の前の fsync は要る)。警告で続けるのは FS が受け付けないときだけで、
        // EIO・ENOSPC 等 (データが届いていない恐れ) は置換に進まず T を残す (05 §4.1 手順 3。docs/review/v0.3-findings.md の SAF-46)
        if opts.fsync != FsyncLevel::None
            && let Err(e) = hooked("fsync", || fsync(&file))
        {
            if fsync_unsupported(&e) {
                warnings.push(SaveWarning::FsyncFailed);
            } else {
                abort(Some(file), &mut flag);
                return Err(map_io_error(e, path));
            }
        }

        // 4. グループと mode (fchown → fchmod の順。所有者は手順 1b で合わせてある。setuid / setgid は保たない)、xattr と POSIX ACL
        if let Some(om) = &orig_meta {
            let mut lost: Vec<AttrKind> = Vec::new();
            let mut lose = |k: AttrKind| {
                if !lost.contains(&k) {
                    lost.push(k);
                }
            };
            let m = om.permissions().mode();
            // 一時ファイルのグループが T と同じか (同じでなければ、元の mode や ACL を当てると別のグループに読める)
            let mut same_group = true;
            if opts.preserve.security {
                let gid = om.gid();
                let gid_is = |f: &File| f.metadata().is_ok_and(|x| x.gid() == gid);
                same_group = gid_is(&file);
                // 所有者は手順 1b で T と同じと確かめてある。ここで合わせるのはグループだけで、所属していないグループ (EPERM)
                // ならグループのビットを落として警告する
                if !same_group {
                    if hooked("fchown_group", || {
                        std::os::unix::fs::fchown(&file, None, Some(gid))
                    })
                    .is_err()
                    {
                        lose(AttrKind::Security);
                    }
                    // chown を黙って何もしない FS (vfat の quiet・unix 拡張の無い CIFS 等) があるので、結果は fstat で見る
                    same_group = gid_is(&file);
                }
                // グループを合わせられなかったら、元の mode を別のグループに当てない。所有者のビットだけにする
                // (05 §4.1 手順 4。docs/review/v0.3-findings.md の SAF-09)
                let mode = if same_group { m & 0o1777 } else { m & 0o700 };
                let chmod_failed = file
                    .set_permissions(fs::Permissions::from_mode(mode))
                    .is_err();
                if chmod_failed || m & 0o6000 != 0 {
                    lose(AttrKind::Security);
                }
            }
            if (opts.preserve.streams_xattr || opts.preserve.security)
                && let Ok(src) = File::open(&target)
            {
                copy_xattrs(&src, &file, same_group)
                    .into_iter()
                    .for_each(&mut lose);
                // T に ACL が無いのに親の既定 ACL を tmp が継承すると、元と違う相手に読める (05 §4.3)
                if opts.preserve.security
                    && get_xattr(&src, ACL_ACCESS).is_err()
                    && File::open(&parent).is_ok_and(|d| get_xattr(&d, ACL_DEFAULT).is_ok())
                {
                    lose(AttrKind::Security);
                }
            }
            if opts.preserve.security && !same_group {
                // xattr の複製の後にもう一度当てる (ACL が付いていれば mask を 0 にして、名前付きの項目も効かなくする)
                let _ = file.set_permissions(fs::Permissions::from_mode(m & 0o700));
            }
            if !lost.is_empty() {
                warnings.push(SaveWarning::AttrsNotPreserved { what: lost });
            }
            // 他の利用者からの権限が元より狭くなったか (グループを合わせられなかった・chmod に失敗した)。最後の mode で判定する
            let final_mode = file.metadata().map_or(0, |x| x.permissions().mode());
            if opts.preserve.security && (m & 0o077) & !final_mode != 0 {
                warnings.push(SaveWarning::PermissionNarrowed);
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
        let mut leftover_tmp = false;
        let method = if existing.is_some() {
            if let Err(e) = hooked("replace", || fs::rename(&tmp, &target)) {
                // T が無傷か (手順 0b の値と同じか) を取り直して確かめる。無傷でなければ AtomicUnsupported にしない
                let intact = matches!(
                    (&existing, self.metadata(&target_path).ok()),
                    (Some(a), Some(b)) if same_file_state(a, &b)
                );
                abort(None, &mut flag);
                return Err(rename_error(e, path, intact));
            }
            SaveMethod::Atomic
        } else {
            match rename_no_replace(&tmp, &target) {
                Ok(None) => {}
                // link で T は完成している: 成功として返す。一時ファイル (T のハードリンク) は消さずに名前をログに残し、
                // 保存中フラグを残して起動時の走査に片付けを任せる (05 §4.1「新規ファイル」。docs/review/v0.3-findings.md の SAF-40)
                Ok(Some(e)) => {
                    flag.log_leftover_tmp(&tmp, &e);
                    warnings.push(SaveWarning::TempNotRemoved {
                        path: FsPath::new(tmp.clone()),
                    });
                    leftover_tmp = true;
                }
                Err(e) => {
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
            }
            SaveMethod::NewFile
        };
        hooks::hit("kill_after_rename");
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
        // 8. 成功: フラグ (と同じキーに残る他の記録) を消す。一時ファイルが残ったときは自分のフラグを残す (SAF-40)
        if !leftover_tmp {
            flag.finish_success();
        }
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
        if set_xattr(&f, b"user.eve_test", b"v1").is_err() {
            // この FS は user xattr に対応しない: 試験を省く
            return;
        }
        drop(f);
        let r = save(&ufs, &a, b"b", Expect::Any).unwrap();
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        let f = File::open(&a).unwrap();
        assert_eq!(get_xattr(&f, b"user.eve_test").unwrap(), b"v1");
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

    // ---- システムコールの失敗の差し替え (hooks::inject_errno、docs/review/v0.3-findings.md の STR-T10) ----
    // `inject_errno` の戻り値は、落とすときにその点を通ったかを確かめる (通らずに試験が通るのを防ぐ)。

    fn stamp_of(ufs: &UnixFs, p: &Path) -> Expect {
        Expect::Stamp(FileStamp::from_meta(
            &ufs.metadata(&FsPath::new(p.to_path_buf())).unwrap(),
            None,
        ))
    }

    /// 保存中フラグを書く UnixFs (状態ディレクトリは `t` の下の `state`)。中断の後にフラグが残らないことを確かめる (05 §4.1 手順 8)。
    fn with_state(t: &Tmp) -> UnixFs {
        let sd = t.p("state");
        fs::create_dir_all(&sd).unwrap();
        UnixFs::new(sd)
    }

    fn leftovers_outside_state(t: &Tmp) -> Vec<String> {
        leftovers(t).into_iter().filter(|n| n != "state").collect()
    }

    // SAF-41: 中断の後片付けで一時ファイルを消せなければ、保存中フラグを残し、一時ファイルの名前を状態ディレクトリのログに残す
    // (05 §4.1 手順 8。本文は書かない)
    #[test]
    fn 中断の後に一時ファイルを消せなければ名前をログに残しフラグも残す() {
        let t = Tmp::new("unlink-tmp");
        let ufs = with_state(&t);
        let a = t.p("a.txt");
        fs::write(
            &a, b"old
",
        )
        .unwrap();
        let expect = stamp_of(&ufs, &a);
        let _r = hooks::inject_errno("replace", libc::EIO);
        let _u = hooks::inject_errno("unlink_tmp", libc::EACCES);
        save(
            &ufs,
            &a,
            b"secret body
",
            expect,
        )
        .unwrap_err();
        assert_eq!(
            fs::read(&a).unwrap(),
            b"old
",
            "T は無傷"
        );
        let tmp = leftovers_outside_state(&t)
            .into_iter()
            .find(|n| n != "a.txt")
            .expect("一時ファイルが残る");
        let log = fs::read_to_string(t.p("state").join("log").join("evoeditor.log")).unwrap();
        assert!(log.contains(&tmp), "{log}");
        assert!(!log.contains("secret body"), "本文は書かない: {log}");
        // 保存中フラグのファイルが残る (自分のプロセスの記録なので list_inflight には出ない)
        let flags: Vec<_> = fs::read_dir(t.p("state").join("recovery").join("inflight"))
            .unwrap()
            .flatten()
            .flat_map(|k| fs::read_dir(k.path()).unwrap().flatten())
            .filter(|e| e.path().extension().is_some_and(|x| x == "toml"))
            .collect();
        assert_eq!(flags.len(), 1, "フラグは残る");
    }

    // SAF-23: 置換の rename が EXDEV / ENOTSUP で、取り直した T が無傷なら AtomicUnsupported (core が確認の後に直接上書きへ進む、
    // 05 §4.4 の表)。T・一時ファイル・保存中フラグも、本物の失敗と同じく片付く
    #[test]
    fn 置換の_rename_が_exdev_や_enotsup_で_t_が無傷なら_atomic_unsupported() {
        for errno in [libc::EXDEV, libc::ENOTSUP, libc::EOPNOTSUPP] {
            let t = Tmp::new(&format!("xdev{errno}"));
            let ufs = with_state(&t);
            let a = t.p("a.txt");
            fs::write(&a, b"old\n").unwrap();
            let expect = stamp_of(&ufs, &a);
            let _g = hooks::inject_errno("replace", errno);
            let e = save(&ufs, &a, b"new\n", expect).unwrap_err();
            assert_eq!(e.kind, FsErrorKind::AtomicUnsupported, "errno {errno}");
            assert_eq!(fs::read(&a).unwrap(), b"old\n", "T は無傷");
            assert!(
                leftovers_outside_state(&t).is_empty(),
                "一時ファイルを片付ける: {:?}",
                leftovers(&t)
            );
            assert!(
                ufs.list_inflight().unwrap().is_empty(),
                "保存中フラグを消す"
            );
        }
    }

    // SAF-23 の守り: T が無傷でない (誰かが変えた) なら、EXDEV でも AtomicUnsupported にしない (直接上書きで他人の変更を潰さない)
    #[test]
    fn t_が無傷でなければ_exdev_でも_atomic_unsupported_にしない() {
        let p = FsPath::new(PathBuf::from("/x/a.txt"));
        let e = rename_error(io::Error::from_raw_os_error(libc::EXDEV), &p, false);
        assert_ne!(e.kind, FsErrorKind::AtomicUnsupported);
        let e = rename_error(io::Error::from_raw_os_error(libc::EXDEV), &p, true);
        assert_eq!(e.kind, FsErrorKind::AtomicUnsupported);
    }

    // 置換の rename の他の失敗 (EIO) は AtomicUnsupported にしない (直接上書きへ進めない)
    #[test]
    fn 置換の_rename_が_eio_なら_atomic_unsupported_にしない() {
        let t = Tmp::new("eio");
        let ufs = with_state(&t);
        let a = t.p("a.txt");
        fs::write(&a, b"old\n").unwrap();
        let expect = stamp_of(&ufs, &a);
        let _g = hooks::inject_errno("replace", libc::EIO);
        let e = save(&ufs, &a, b"new\n", expect).unwrap_err();
        assert_ne!(e.kind, FsErrorKind::AtomicUnsupported);
        assert_eq!(fs::read(&a).unwrap(), b"old\n");
        assert!(
            leftovers_outside_state(&t).is_empty(),
            "{:?}",
            leftovers(&t)
        );
        assert!(ufs.list_inflight().unwrap().is_empty());
    }

    // REF-07: 上書きしない移動 (Linux は renameat2) を使えない FS では、ハードリンク + 削除で新規ファイルを作る
    #[test]
    fn 上書きしない移動が使えなければハードリンクで新規ファイルを作る() {
        for errno in [libc::ENOSYS, libc::EINVAL, libc::ENOTSUP, libc::EOPNOTSUPP] {
            let t = Tmp::new(&format!("noreplace{errno}"));
            let ufs = UnixFs::default();
            let a = t.p("a.txt");
            let _g = hooks::inject_errno("rename_noreplace", errno);
            let r = save(&ufs, &a, b"one\n", Expect::Absent).unwrap();
            assert_eq!(r.method, SaveMethod::NewFile, "errno {errno}");
            assert_eq!(fs::read(&a).unwrap(), b"one\n");
            assert!(
                leftovers(&t).is_empty(),
                "errno {errno}: {:?}",
                leftovers(&t)
            );
        }
    }

    // ハードリンクの退路も上書きしない: その間に T ができていたら (link が EEXIST) Conflict で、T に触れない
    #[test]
    fn 退路の_link_が_eexist_なら_conflict_で_t_に触れない() {
        let t = Tmp::new("linkexist");
        let ufs = UnixFs::default();
        let a = t.p("a.txt");
        let _g1 = hooks::inject_errno("rename_noreplace", libc::ENOSYS);
        let _g2 = hooks::inject_errno("link", libc::EEXIST);
        let e = save(&ufs, &a, b"one\n", Expect::Absent).unwrap_err();
        assert!(
            matches!(e.kind, FsErrorKind::Conflict { .. }),
            "{:?}",
            e.kind
        );
        assert!(leftovers(&t).is_empty(), "{:?}", leftovers(&t));
    }

    // 退路の link がほかの理由で失敗したら、何も作らずにエラー (T は作られず、一時ファイルも残さない)。
    // エラーの種類は確かめない (EPERM が PermissionDenied に写るのは誤解を招く、SAF-39。REF-07 の扱いと合わせて v0.4 以降に決める)
    #[test]
    fn 退路の_link_も失敗したら何も作らない() {
        let t = Tmp::new("nolink");
        let ufs = with_state(&t);
        let a = t.p("a.txt");
        let _g1 = hooks::inject_errno("rename_noreplace", libc::ENOSYS);
        let _g2 = hooks::inject_errno("link", libc::EPERM);
        assert!(save(&ufs, &a, b"one\n", Expect::Absent).is_err());
        assert!(!a.exists(), "T を作らない");
        assert!(
            leftovers_outside_state(&t).is_empty(),
            "{:?}",
            leftovers(&t)
        );
        assert!(ufs.list_inflight().unwrap().is_empty());
    }

    // 05 §4.1 手順 3: 一時ファイルの fsync を FS が受け付けない (EINVAL / ENOTSUP / EOPNOTSUPP / ENOSYS。ネットワーク FS など) ときは警告にして保存を続ける
    #[test]
    fn 一時ファイルの_fsync_を受け付けない_fs_では警告にして保存する() {
        for errno in [libc::EINVAL, libc::ENOTSUP, libc::EOPNOTSUPP, libc::ENOSYS] {
            let t = Tmp::new(&format!("fsync{errno}"));
            let ufs = UnixFs::default();
            let a = t.p("a.txt");
            fs::write(&a, b"old\n").unwrap();
            let expect = stamp_of(&ufs, &a);
            let _g = hooks::inject_errno("fsync", errno);
            let r = save(&ufs, &a, b"new\n", expect).unwrap();
            assert_eq!(fs::read(&a).unwrap(), b"new\n");
            assert!(
                r.warnings.contains(&SaveWarning::FsyncFailed),
                "errno {errno}: {:?}",
                r.warnings
            );
        }
    }

    // SAF-46: 一時ファイルの fsync が EIO・ENOSPC (データが届いていない恐れ) なら置換に進まず、一時ファイルを消して T を残す
    // (05 §4.1 手順 3)
    #[test]
    fn 一時ファイルの_fsync_が_eio_や_enospc_なら置換せずに_t_を残す() {
        for (errno, kind) in [
            (libc::EIO, FsErrorKind::Other),
            (libc::ENOSPC, FsErrorKind::DiskFull),
        ] {
            let t = Tmp::new(&format!("fsyncfail{errno}"));
            let ufs = with_state(&t);
            let a = t.p("a.txt");
            fs::write(&a, b"old\n").unwrap();
            let expect = stamp_of(&ufs, &a);
            let _g = hooks::inject_errno("fsync", errno);
            let e = save(&ufs, &a, b"new\n", expect).unwrap_err();
            assert_eq!(e.kind, kind, "errno {errno}");
            assert_eq!(fs::read(&a).unwrap(), b"old\n", "T は無傷");
            assert!(
                leftovers_outside_state(&t).is_empty(),
                "errno {errno}: {:?}",
                leftovers(&t)
            );
            assert_eq!(flag_files(&t), 0, "保存中フラグを消す");
        }
    }

    /// 状態ディレクトリに残る保存中フラグのファイルの数 (自分のプロセスの記録は list_inflight に出ないので、ファイルを数える)。
    fn flag_files(t: &Tmp) -> usize {
        let Ok(rd) = fs::read_dir(t.p("state").join("recovery").join("inflight")) else {
            return 0;
        };
        rd.flatten()
            .flat_map(|k| fs::read_dir(k.path()).into_iter().flatten().flatten())
            .filter(|e| e.path().extension().is_some_and(|x| x == "toml"))
            .count()
    }

    // SAF-36: 一時ファイルの所有者が T と違う (他ユーザー所有で root でない) なら、本文を書く前に空の一時ファイルとフラグを消して
    // AtomicUnsupported (core が確認の後に直接上書きし、所有者を保つ。05 §4.1 手順 1b・§4.4 の表)。uid は変えられないので、
    // 実効 uid と一時ファイルの st_uid を「T の所有者でも root でもない」に差し替える
    #[test]
    fn 一時ファイルの所有者が_t_と違えば本文を書く前に_atomic_unsupported() {
        let t = Tmp::new("other-owner");
        let ufs = with_state(&t);
        let a = t.p("a.txt");
        fs::write(&a, b"old\n").unwrap();
        let other = fs::metadata(&a).unwrap().uid().wrapping_add(1).max(1);
        let _e = hooks::pretend_uid("euid", other);
        let _u = hooks::pretend_uid("tmp_uid", other);
        let expect = stamp_of(&ufs, &a);
        let e = save(&ufs, &a, b"secret\n", expect).unwrap_err();
        assert_eq!(e.kind, FsErrorKind::AtomicUnsupported);
        assert!(
            e.detail.is_none(),
            "確認付きの直接上書き (ハードリンク・一時ファイル規則ではない): {:?}",
            e.detail
        );
        assert_eq!(fs::read(&a).unwrap(), b"old\n");
        assert!(
            leftovers_outside_state(&t).is_empty(),
            "一時ファイルを消す: {:?}",
            leftovers(&t)
        );
        assert_eq!(flag_files(&t), 0, "保存中フラグを消す");
    }

    // SAF-36 の守り: uid を固定・写像して見せる FS (sshfs の idmap=none・CIFS・all_squash 等) では、他ユーザー所有に見えても
    // 一時ファイルが T と同じ uid になるので、置換を続ける (毎回直接上書きに落とさない)
    #[test]
    fn 一時ファイルの所有者が_t_と同じなら他ユーザー所有に見えても置換する() {
        let t = Tmp::new("idmap");
        let ufs = UnixFs::default();
        let a = t.p("a.txt");
        fs::write(&a, b"old\n").unwrap();
        let owner = fs::metadata(&a).unwrap().uid();
        let _e = hooks::pretend_uid("euid", owner.wrapping_add(1).max(1));
        assert_eq!(
            ufs.metadata(&FsPath::new(a.clone())).unwrap().owner_is_me,
            Some(false)
        );
        let expect = stamp_of(&ufs, &a);
        let r = save(&ufs, &a, b"new\n", expect).unwrap();
        assert_eq!(r.method, SaveMethod::Atomic);
        assert_eq!(fs::read(&a).unwrap(), b"new\n");
        assert_eq!(fs::metadata(&a).unwrap().uid(), owner);
        assert!(leftovers(&t).is_empty(), "{:?}", leftovers(&t));
    }

    // SAF-37 の残り: root で fchown が黙って何もしない FS (一時ファイルの st_uid が変わらない) と、所有者を写さない
    // (preserve.security = false) 保存でも、所有者の違う一時ファイルで置換しない (05 §4.1 手順 1b・4)
    #[test]
    fn root_でも一時ファイルの所有者を_t_に合わせられなければ置換しない() {
        for security in [true, false] {
            let t = Tmp::new(&format!("root-noop{security}"));
            let ufs = with_state(&t);
            let a = t.p("a.txt");
            fs::write(&a, b"old\n").unwrap();
            let other = fs::metadata(&a).unwrap().uid().wrapping_add(1).max(1);
            let _e = hooks::pretend_uid("euid", 0);
            let _u = hooks::pretend_uid("tmp_uid", other);
            let mut opts = SaveOptions::user_file(stamp_of(&ufs, &a));
            opts.preserve.security = security;
            let mut body = Bytes(Some(b"new\n"));
            let e = ufs
                .write_atomic(&FsPath::new(a.clone()), &mut body, &opts)
                .unwrap_err();
            assert_eq!(
                e.kind,
                FsErrorKind::AtomicUnsupported,
                "security {security}"
            );
            assert!(body.0.is_some(), "本文を書く前に中止する");
            assert_eq!(fs::read(&a).unwrap(), b"old\n");
            assert!(
                leftovers_outside_state(&t).is_empty(),
                "{:?}",
                leftovers(&t)
            );
            assert_eq!(flag_files(&t), 0);
        }
    }

    // SAF-37: root で fchown(uid) が失敗したら (root_squash 等)、置換の前に一時ファイルを消して AtomicUnsupported
    // (05 §4.1 手順 4・§4.4 の表)。root でなければ実効 uid を 0 に差し替え、root なら T を別の uid に渡してから試す
    #[test]
    fn root_で所有者を合わせられなければ一時ファイルを消して_atomic_unsupported() {
        let t = Tmp::new("root-chown");
        let ufs = with_state(&t);
        let a = t.p("a.txt");
        fs::write(&a, b"old\n").unwrap();
        // root でなければ実効 uid を 0 に、一時ファイルの所有者を T と別の uid に差し替える (root が作った一時ファイルを再現する)
        let other = fs::metadata(&a).unwrap().uid().wrapping_add(1).max(1);
        let _p = if euid() == 0 {
            std::os::unix::fs::chown(&a, Some(12345), None).unwrap();
            None
        } else {
            Some((
                hooks::pretend_uid("euid", 0),
                hooks::pretend_uid("tmp_uid", other),
            ))
        };
        let owner = fs::metadata(&a).unwrap().uid();
        let expect = stamp_of(&ufs, &a);
        let _g = hooks::inject_errno("fchown", libc::EPERM);
        let mut body = Bytes(Some(b"new\n"));
        let e = ufs
            .write_atomic(
                &FsPath::new(a.clone()),
                &mut body,
                &SaveOptions::user_file(expect),
            )
            .unwrap_err();
        assert_eq!(e.kind, FsErrorKind::AtomicUnsupported);
        assert!(
            body.0.is_some(),
            "本文を書く前に中止する (他人の所有の一時ファイルに書かない)"
        );
        assert_eq!(fs::read(&a).unwrap(), b"old\n", "T は無傷");
        assert_eq!(fs::metadata(&a).unwrap().uid(), owner, "所有者を変えない");
        assert!(
            leftovers_outside_state(&t).is_empty(),
            "一時ファイルを消す: {:?}",
            leftovers(&t)
        );
        assert_eq!(flag_files(&t), 0, "保存中フラグを消す");
    }

    // SAF-38: 1 ファイルだけの bind mount では置換の rename が EBUSY になる。T が無傷なら AtomicUnsupported (05 §4.4 の表)
    #[test]
    fn 置換の_rename_が_ebusy_で_t_が無傷なら_atomic_unsupported() {
        let t = Tmp::new("ebusy");
        let ufs = with_state(&t);
        let a = t.p("a.txt");
        fs::write(&a, b"old\n").unwrap();
        let expect = stamp_of(&ufs, &a);
        let _g = hooks::inject_errno("replace", libc::EBUSY);
        let e = save(&ufs, &a, b"new\n", expect).unwrap_err();
        assert_eq!(e.kind, FsErrorKind::AtomicUnsupported);
        assert_eq!(fs::read(&a).unwrap(), b"old\n");
        assert!(
            leftovers_outside_state(&t).is_empty(),
            "{:?}",
            leftovers(&t)
        );
        assert_eq!(flag_files(&t), 0);
        // T が無傷でなければ EBUSY でも直接上書きへ進めない
        let p = FsPath::new(PathBuf::from("/x/a.txt"));
        let e = rename_error(io::Error::from_raw_os_error(libc::EBUSY), &p, false);
        assert_eq!(e.kind, FsErrorKind::Busy);
    }

    // SAF-40: 退路の link が成功した後に一時ファイルを消せなくても、T は完成しているので成功を返す。一時ファイル (T のハードリンク)
    // は残し、名前をログに残して保存中フラグも残す (05 §4.1「新規ファイル」)。次の保存は nlink = 2 のハードリンクの確認になる
    #[test]
    fn 退路の_link_の後に一時ファイルを消せなくても保存は成功しフラグを残す() {
        let t = Tmp::new("unlink-after-link");
        let ufs = with_state(&t);
        let a = t.p("a.txt");
        let _g1 = hooks::inject_errno("rename_noreplace", libc::ENOSYS);
        let _g2 = hooks::inject_errno("unlink_after_link", libc::EIO);
        let r = save(&ufs, &a, b"one\n", Expect::Absent).unwrap();
        assert_eq!(r.method, SaveMethod::NewFile);
        assert_eq!(fs::read(&a).unwrap(), b"one\n");
        let tmp = leftovers_outside_state(&t)
            .into_iter()
            .find(|n| n.contains(".eve-tmp-"))
            .expect("一時ファイルが残る");
        assert_eq!(
            r.warnings,
            vec![SaveWarning::TempNotRemoved {
                path: FsPath::new(t.p(&tmp))
            }],
            "利用者に知らせる"
        );
        assert_eq!(
            fs::metadata(t.p(&tmp)).unwrap().ino(),
            fs::metadata(&a).unwrap().ino()
        );
        let log = fs::read_to_string(t.p("state").join("log").join("evoeditor.log")).unwrap();
        assert!(log.contains(&tmp), "{log}");
        assert_eq!(flag_files(&t), 1, "フラグは残る");
        // 次の保存: 照合は合い (Conflict にならない)、ハードリンクの確認になる
        let e = save(&ufs, &a, b"two\n", stamp_of(&ufs, &a)).unwrap_err();
        assert!(matches!(
            e.detail,
            Some(FsErrorDetail::Hardlink { nlink: 2 })
        ));
    }

    // 05 §4.1 手順 6・§4.3: ディレクトリの fsync の失敗は (置換の後なので) 警告にする
    #[test]
    fn ディレクトリの_fsync_の失敗は警告にする() {
        for errno in [libc::EINVAL, libc::EIO] {
            let t = Tmp::new(&format!("fsyncdir{errno}"));
            let ufs = UnixFs::default();
            let a = t.p("a.txt");
            fs::write(&a, b"old\n").unwrap();
            let expect = stamp_of(&ufs, &a);
            let _g = hooks::inject_errno("fsync_dir", errno);
            let r = save(&ufs, &a, b"new\n", expect).unwrap();
            assert_eq!(fs::read(&a).unwrap(), b"new\n");
            assert!(
                r.warnings.contains(&SaveWarning::FsyncFailed),
                "errno {errno}: {:?}",
                r.warnings
            );
        }
    }
}
