//! OS ごとの小さな部品の窓口。状態ディレクトリの部品 (保存中フラグ・直接上書き・状態ファイル・スワップ・緊急退避・ログ) は
//! OS に依らずここの関数だけを呼ぶ (docs/05 §5.6 / §7.5)。Windows の実体は `fs` / `owner` / `save`、Unix の実体はこの下にある。

#![cfg_attr(unix, allow(unsafe_code))]

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use eve_core::FileTime;
use eve_core::{FileMeta, FileStamp};

#[cfg(windows)]
pub(crate) use crate::fs::map_io_error;
#[cfg(windows)]
pub(crate) use crate::owner::{free_bytes, owned_by_me};
#[cfg(windows)]
pub(crate) use crate::save::{delete, resolve_target};
#[cfg(unix)]
pub(crate) use crate::unix_fs::{map_io_error, resolve_target};

/// 照合の比較 (05 §5.5。ハッシュは持たないので (mtime, len, FileId))。
pub(crate) fn matches(want: &FileStamp, have: &FileStamp) -> bool {
    want.mtime == have.mtime && want.len == have.len && want.file_id == have.file_id
}

pub(crate) fn same_file_state(a: &FileMeta, b: &FileMeta) -> bool {
    a.mtime == b.mtime && a.len == b.len && a.file_id == b.file_id
}

/// 状態ディレクトリの中のファイルを書く `OpenOptions` (05 §5.6 の「所有者のみ」。Unix は `0600` で作る。umask に依らない。
/// Windows はディレクトリの保護付き DACL を継承する)。
pub(crate) fn private_options() -> OpenOptions {
    #[cfg_attr(windows, allow(unused_mut))]
    let mut o = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o
}

/// `File::create` の所有者のみ版 (在れば切り詰める)。
pub(crate) fn create_private(path: &Path) -> io::Result<File> {
    private_options()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
}

/// 緊急退避の代替の退避先 (05 §7.5)。Windows は `%TEMP%\eve-recovery-<SID>\`、Unix は `$TMPDIR/eve-recovery-<uid>/`
/// (`TMPDIR` が無ければ `/tmp`。`std::env::temp_dir` と同じ規則)。
pub(crate) fn recovery_fallback_dir() -> io::Result<PathBuf> {
    #[cfg(windows)]
    let who = crate::save::current_user_sid()?;
    #[cfg(unix)]
    let who = euid().to_string();
    Ok(std::env::temp_dir().join(format!("eve-recovery-{who}")))
}

// ---------- Unix ----------

#[cfg(unix)]
pub(crate) fn euid() -> u32 {
    // SAFETY: 引数の無いシステムコール。
    unsafe { libc::geteuid() }
}

#[cfg(unix)]
pub(crate) fn delete(path: &Path) -> bool {
    std::fs::remove_file(path).is_ok()
}

/// 代替の退避先などの既存のディレクトリを使ってよいか (05 §7.5): シンボリックリンクでない・所有者が自分・mode が 0700。
#[cfg(unix)]
pub(crate) fn owned_by_me(path: &Path) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::symlink_metadata(path)?;
    Ok(!m.file_type().is_symlink() && m.uid() == euid() && m.mode() & 0o777 == 0o700)
}

/// `path` のある FS の、呼び出し元が使える空き容量 (バイト、`statvfs` の `f_bavail`)。調べられなければ `None`。
#[cfg(unix)]
pub(crate) fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: ゼロ初期化した `statvfs` (全フィールドが整数) に書かせる。
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: NUL 終端のパスと有効な書込先。
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    #[allow(clippy::useless_conversion)]
    Some(u64::from(st.f_bavail).saturating_mul(u64::from(st.f_frsize)))
}

/// 1 段だけ 0700 で作る (05 §5.6。在れば何もしない)。
#[cfg(unix)]
pub(crate) fn create_dir_private(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => Ok(()),
        // 並行して作られた
        Err(_) if dir.is_dir() => Ok(()),
        Err(e) => Err(e),
    }
}

/// pid のプロセスの起動時刻 (Linux: `/proc/<pid>/stat` の starttime + `/proc/stat` の btime)。取れなければ `None`。
#[cfg(unix)]
pub(crate) fn process_start_of(pid: u32) -> Option<FileTime> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // comm は括弧で囲まれ、空白や括弧を含み得るので、最後の `)` の後ろから数える (state が 3 番目、starttime が 22 番目)
    let rest = &stat[stat.rfind(')')? + 1..];
    let ticks: u64 = rest.split_whitespace().nth(19)?.parse().ok()?;
    let btime: i64 = std::fs::read_to_string("/proc/stat")
        .ok()?
        .lines()
        .find_map(|l| l.strip_prefix("btime "))?
        .trim()
        .parse()
        .ok()?;
    // SAFETY: 引数の値だけを読む問い合わせ。
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    let hz = u64::try_from(hz).ok().filter(|h| *h > 0)?;
    let secs = btime + i64::try_from(ticks / hz).ok()?;
    let nanos = u32::try_from((ticks % hz) * 1_000_000_000 / hz).ok()?;
    Some(FileTime { secs, nanos })
}

/// パスワードデータベースの自分 (`geteuid()`) のホーム (`getpwuid_r`)。引けなければ `None`。
#[cfg(unix)]
pub(crate) fn passwd_home() -> Option<String> {
    let mut buf = vec![0u8; 4096];
    // SAFETY: ゼロ初期化した passwd に、`buf` を作業領域として書かせる。結果のポインタは `buf` か `pw` を指す。
    unsafe {
        let mut pw: libc::passwd = std::mem::zeroed();
        let mut out: *mut libc::passwd = std::ptr::null_mut();
        let r = libc::getpwuid_r(
            euid(),
            &mut pw,
            buf.as_mut_ptr().cast(),
            buf.len(),
            &mut out,
        );
        if r != 0 || out.is_null() || pw.pw_dir.is_null() {
            return None;
        }
        let dir = std::ffi::CStr::from_ptr(pw.pw_dir)
            .to_string_lossy()
            .into_owned();
        (!dir.is_empty()).then_some(dir)
    }
}

/// pid のプロセスが動いているか (`kill(pid, 0)`。`EPERM` は「いるが送れない」)。
#[cfg(unix)]
pub(crate) fn pid_running(pid: u32) -> bool {
    // 0 や負の値 (プロセスグループ) にならない pid だけを問い合わせる
    let Ok(p) = libc::pid_t::try_from(pid) else {
        return false;
    };
    if p <= 0 {
        return false;
    }
    // SAFETY: シグナル 0 は送らずに存在と権限だけを確かめる。
    if unsafe { libc::kill(p, 0) } == 0 {
        return true;
    }
    io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn 自分の起動時刻と生存を取れる() {
        let me = std::process::id();
        let t = process_start_of(me).expect("起動時刻");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        assert!(t.secs <= now + 2 && t.secs > now - 24 * 3600, "{t:?}");
        assert!(pid_running(me));
        assert!(!pid_running(0));
        assert!(!pid_running(0xFFFF_FFF0));
    }

    #[test]
    fn 所有者のみのディレクトリとファイル() {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("eve-sys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("private");
        create_dir_private(&p).unwrap();
        create_dir_private(&p).unwrap();
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert!(owned_by_me(&p).unwrap());
        let f = p.join("a.toml");
        create_private(&f).unwrap();
        assert_eq!(
            std::fs::metadata(&f).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!owned_by_me(&p).unwrap(), "0700 でなければ使わない");
        assert!(free_bytes(&d).is_some_and(|f| f > 0));
        let _ = std::fs::remove_dir_all(&d);
    }
}
