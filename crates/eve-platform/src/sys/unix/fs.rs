//! ファイルの API (書けるかの問い合わせ・xattr・上書きしない名前の変更・領域の確保・fsync)。
//! Linux 専用のものは `cfg(target_os = "linux")` に閉じる。他の Unix は「使えない」の値を返す (呼び側は今の代替の経路に進む)。

use std::ffi::CString;
use std::fs::File;
use std::io;
use std::os::unix::ffi::OsStrExt;
#[cfg(target_os = "linux")]
use std::os::unix::io::AsRawFd;
use std::path::Path;

fn c_path(p: &Path) -> io::Result<CString> {
    CString::new(p.as_os_str().as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
}

/// 自分 (実効ユーザー) が書けるか (`access(W_OK)`)。
pub(crate) fn can_write(p: &Path) -> bool {
    match c_path(p) {
        // SAFETY: NUL 終端のパス。
        Ok(c) => unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 },
        Err(_) => false,
    }
}

/// ファイルの fsync (`sync_all`)。差し替え点: std の `sync_all` は Apple ではすでに `F_FULLFSYNC` を呼ぶが、FS が受け付けない
/// (ENOTSUP / EINVAL) ときに `fsync` へ戻して警告にする部分 (05 §4.1 手順 3) は v1.x。今これを通るのは `unix_fs` の一時ファイルと
/// ディレクトリだけで、`inplace`・`inflight`・`state_file`・`rescue` は `sync_all` を直に呼ぶ (macOS へ移すときにここへ寄せる)。
pub(crate) fn fsync(f: &File) -> io::Result<()> {
    f.sync_all()
}

// ---------- 上書きしない名前の変更 ----------

/// `renameat2(RENAME_NOREPLACE)` を 1 回だけ呼ぶ (代替の経路は呼び側)。在れば `EEXIST`。
/// libc の関数ではなくシステムコールで呼ぶ (musl 1.2.5 (Rust の musl ターゲットが同梱する版) には `renameat2` の関数が無い)。
#[cfg(target_os = "linux")]
pub(crate) fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
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
    Err(io::Error::last_os_error())
}

/// Linux 以外: `ENOSYS` (呼び側はハードリンク + 削除に進む)。macOS へ移すときは `renamex_np(RENAME_EXCL)` にする (05 §4.1。
/// ハードリンクの無い FS (exFAT・一部の SMB) では、今の代替の経路だと新規ファイルを保存できない)。
#[cfg(not(target_os = "linux"))]
pub(crate) fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    let (_f, _t) = (c_path(from)?, c_path(to)?);
    Err(io::Error::from_raw_os_error(libc::ENOSYS))
}

// ---------- 領域の確保 ----------

/// `fallocate(FALLOC_FL_KEEP_SIZE)` (長さは延ばさない)。
#[cfg(target_os = "linux")]
pub(crate) fn fallocate_keep_size(file: &File, new_alloc: u64) -> io::Result<()> {
    let len = libc::off_t::try_from(new_alloc).unwrap_or(libc::off_t::MAX);
    // SAFETY: 開いているファイルの記述子。長さは変えない (KEEP_SIZE)。
    let r = unsafe { libc::fallocate(file.as_raw_fd(), libc::FALLOC_FL_KEEP_SIZE, 0, len) };
    if r == 0 {
        return Ok(());
    }
    Err(io::Error::last_os_error())
}

/// Linux 以外: `ENOSYS` (呼び側は空き容量の確認に進む)。
#[cfg(not(target_os = "linux"))]
pub(crate) fn fallocate_keep_size(file: &File, new_alloc: u64) -> io::Result<()> {
    let _ = (file, new_alloc);
    Err(io::Error::from_raw_os_error(libc::ENOSYS))
}

// ---------- xattr ----------

/// `f` の xattr の名前の一覧。対応しない FS (ENOTSUP) は空。
#[cfg(target_os = "linux")]
pub(crate) fn list_xattrs(f: &File) -> io::Result<Vec<Vec<u8>>> {
    let fd = f.as_raw_fd();
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
    // n >= 0 を確かめた後
    let mut buf = vec![0u8; n.unsigned_abs()];
    // SAFETY: `buf` は `n` バイト。
    let n = unsafe { libc::flistxattr(fd, buf.as_mut_ptr().cast(), buf.len()) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    buf.truncate(n.unsigned_abs());
    Ok(buf
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(<[u8]>::to_vec)
        .collect())
}

#[cfg(target_os = "linux")]
pub(crate) fn get_xattr(f: &File, name: &[u8]) -> io::Result<Vec<u8>> {
    let fd = f.as_raw_fd();
    let c = CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: NUL 終端の名前、長さの問い合わせ。
    let n = unsafe { libc::fgetxattr(fd, c.as_ptr(), std::ptr::null_mut(), 0) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    // n >= 0 を確かめた後
    let mut buf = vec![0u8; n.unsigned_abs()];
    // SAFETY: `buf` は `n` バイト。
    let n = unsafe { libc::fgetxattr(fd, c.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    buf.truncate(n.unsigned_abs());
    Ok(buf)
}

#[cfg(target_os = "linux")]
pub(crate) fn set_xattr(f: &File, name: &[u8], value: &[u8]) -> io::Result<()> {
    let fd = f.as_raw_fd();
    let c = CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: NUL 終端の名前と、長さ付きの値。
    let r = unsafe { libc::fsetxattr(fd, c.as_ptr(), value.as_ptr().cast(), value.len(), 0) };
    if r != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Linux 以外: 対応しない FS と同じ (空)。**注意**: 呼び側 (`copy_xattrs`) は失った属性が無いとみなすので、macOS などでは
/// 拡張属性 (Finder のタグ・`com.apple.*`) を黙って失う (05 §4.1 の「黙って失わない」に反する)。移すときに実装する (v1.x)。
#[cfg(not(target_os = "linux"))]
pub(crate) fn list_xattrs(_: &File) -> io::Result<Vec<Vec<u8>>> {
    Ok(Vec::new())
}

/// Linux 以外: `ENOTSUP`。
#[cfg(not(target_os = "linux"))]
pub(crate) fn get_xattr(_: &File, _: &[u8]) -> io::Result<Vec<u8>> {
    Err(io::Error::from_raw_os_error(libc::ENOTSUP))
}

/// Linux 以外: `ENOTSUP`。
#[cfg(not(target_os = "linux"))]
pub(crate) fn set_xattr(_: &File, _: &[u8], _: &[u8]) -> io::Result<()> {
    Err(io::Error::from_raw_os_error(libc::ENOTSUP))
}
