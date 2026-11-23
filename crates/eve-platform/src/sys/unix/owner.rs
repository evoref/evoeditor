//! 所有者の検査 (docs/07 §12.1 の OWN-1〜OWN-7、05 §5.7) の Unix の API: ディレクトリの中の項目の `fstatat`・`openat`
//! (リンクを辿らない)・`fstatfs`・uid の名前。判定の規則そのものは `read_checked` (unsafe なし)。
//!
//! spec: 05 §5.7, 07 §12.1 (OWN-3, OWN-6)

use std::ffi::{CString, OsStr};
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;

fn c_name(name: &OsStr) -> io::Result<CString> {
    CString::new(name.as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
}

/// `dir` の中の `name` の `st_mode` (`fstatat(…, AT_SYMLINK_NOFOLLOW)`。リンクは辿らない)。無ければ `Ok(None)`。
pub(crate) fn mode_at(dir: &File, name: &OsStr) -> io::Result<Option<u32>> {
    let c = c_name(name)?;
    // SAFETY: ゼロ初期化した `stat` (全フィールドが整数) に書かせる。
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `dir` は開いているディレクトリの記述子 (この関数の間は閉じられない)、`c` は NUL 終端の名前、`st` は有効な書込先。
    let r = unsafe {
        libc::fstatat(
            dir.as_raw_fd(),
            c.as_ptr(),
            &raw mut st,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if r == 0 {
        // `mode_t` は Linux で u32、macOS・BSD で u16
        #[allow(
            clippy::useless_conversion,
            reason = "mode_t は OS ごとに u32 か u16 (Linux では同じ型の変換)"
        )]
        return Ok(Some(u32::from(st.st_mode)));
    }
    let e = io::Error::last_os_error();
    match e.raw_os_error() {
        Some(libc::ENOENT | libc::ENOTDIR) => Ok(None),
        _ => Err(e),
    }
}

/// `dir` の中の `name` を読むために開く (`openat(…, O_RDONLY | O_CLOEXEC | O_NONBLOCK | O_NOFOLLOW | O_NOCTTY)`)。
/// リンクなら `ELOOP` で失敗する。FIFO は書き手を待たずに開ける (呼び側が `fstat` でふつうのファイルかを確かめる)。
pub(crate) fn open_at_nofollow(dir: &File, name: &OsStr) -> io::Result<File> {
    let c = c_name(name)?;
    // SAFETY: `dir` は開いているディレクトリの記述子、`c` は NUL 終端の名前。作らないので mode は渡さない。
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            c.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_NOCTTY,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` は今開いた記述子で、`File` だけが所有する (閉じるのは `File` の Drop)。
    Ok(unsafe { File::from_raw_fd(fd) })
}

/// `dir` の中のディレクトリ `name` を、リンクを辿らずに開く (`openat(…, O_RDONLY | O_CLOEXEC | O_DIRECTORY | O_NOFOLLOW)`)。
/// リンクなら Linux は `ENOTDIR`、ほかは `ELOOP` などで失敗する (呼び側が `mode_at` でリンクかを見分ける)。
pub(crate) fn open_dir_at_nofollow(dir: &File, name: &OsStr) -> io::Result<File> {
    let c = c_name(name)?;
    // SAFETY: `dir` は開いているディレクトリの記述子、`c` は NUL 終端の名前。作らないので mode は渡さない。
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            c.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` は今開いた記述子で、`File` だけが所有する。
    Ok(unsafe { File::from_raw_fd(fd) })
}

/// 開いた物の最終のパス (Linux は `/proc/self/fd/<fd>` の `readlink`。07 §12.1 の OWN-5)。取れなければ `None`
/// (`/proc` が無い・Linux 以外。呼び側は安全な場所の下でないとみなす)。
pub(crate) fn final_path(f: &File) -> Option<std::path::PathBuf> {
    if cfg!(target_os = "linux") {
        std::fs::read_link(format!("/proc/self/fd/{}", f.as_raw_fd())).ok()
    } else {
        None
    }
}

/// 開いている物のある FS の `f_type` (`fstatfs`。下位 32 bit)。
#[cfg(target_os = "linux")]
pub(crate) fn fs_type(f: &File) -> io::Result<u32> {
    // SAFETY: ゼロ初期化した `statfs` (全フィールドが整数) に書かせる。
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: `f` は開いている記述子、`st` は有効な書込先。
    if unsafe { libc::fstatfs(f.as_raw_fd(), &raw mut st) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // `f_type` の型は libc ごとに違う (glibc は符号付き、musl は符号無し)。下位 32 bit で比べる
    u32::try_from(st.f_type & 0xFFFF_FFFF).map_err(|_| io::Error::other("f_type"))
}

/// uid のアカウントの名前 (`getpwuid_r`)。引けなければ `None`。
pub(crate) fn user_name(uid: u32) -> Option<String> {
    let mut buf = vec![0u8; 4096];
    // SAFETY: ゼロ初期化した passwd に、`buf` を作業領域として書かせる。結果のポインタは `buf` か `pw` を指し、
    // 名前は `buf` が生きている間に写す。
    unsafe {
        let mut pw: libc::passwd = std::mem::zeroed();
        let mut out: *mut libc::passwd = std::ptr::null_mut();
        let r = libc::getpwuid_r(
            uid,
            &raw mut pw,
            buf.as_mut_ptr().cast(),
            buf.len(),
            &raw mut out,
        );
        if r != 0 || out.is_null() || pw.pw_name.is_null() {
            return None;
        }
        let name = std::ffi::CStr::from_ptr(pw.pw_name)
            .to_string_lossy()
            .into_owned();
        (!name.is_empty()).then_some(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ディレクトリの中の項目を辿らずに見る() {
        use std::os::unix::fs::OpenOptionsExt;
        let d = std::env::temp_dir().join(format!("eve-own-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a"), b"x").unwrap();
        std::os::unix::fs::symlink(d.join("a"), d.join("l")).unwrap();
        let dir = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY)
            .open(&d)
            .unwrap();
        // POSIX の値 (`mode_t` の型に依らない)
        let (fmt, reg, lnk) = (0o170_000u32, 0o100_000u32, 0o120_000u32);
        let m = mode_at(&dir, OsStr::new("a")).unwrap().unwrap();
        assert_eq!(m & fmt, reg);
        let l = mode_at(&dir, OsStr::new("l")).unwrap().unwrap();
        assert_eq!(l & fmt, lnk);
        assert_eq!(mode_at(&dir, OsStr::new("none")).unwrap(), None);
        assert!(open_at_nofollow(&dir, OsStr::new("a")).is_ok());
        assert!(open_at_nofollow(&dir, OsStr::new("l")).is_err());
        #[cfg(target_os = "linux")]
        assert_eq!(
            open_at_nofollow(&dir, OsStr::new("l"))
                .unwrap_err()
                .raw_os_error(),
            Some(libc::ELOOP)
        );
        assert!(final_path(&dir).is_some() || !cfg!(target_os = "linux"));
        assert_eq!(user_name(0).as_deref(), Some("root"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
