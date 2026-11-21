//! 実 FS (Windows は `WinFs`、Unix は `UnixFs`) に `FileSystem` の契約試験を掛ける (docs/02 §4.5、05 §2.1 / §4.1、STR-D3)。
//! 同じ検査を eve-testkit の `tests/fs_contract.rs` が `MemFs` に掛ける。状態ディレクトリを持つものと持たないものの両方。
//! `read_dir` の検査の準備 (ディレクトリ・リンク・属性・FIFO・列挙の拒否) は `NativeFixture` が OS の操作で行う。

#![allow(clippy::print_stderr)]

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use eve_core::{FileKind, FsPath};
#[cfg(windows)]
use eve_platform::fs::WinFs as NativeFs;
#[cfg(unix)]
use eve_platform::unix_fs::UnixFs as NativeFs;
use eve_testkit::fs_contract::{Caps, Fixture, Subject, suite};

/// 一時ディレクトリ (終わりに消す)。
struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Tmp {
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("eve-contract-{tag}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(p.join("work")).unwrap();
        Tmp(p)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn names(dir: &FsPath) -> Vec<String> {
    fs::read_dir(dir.as_path())
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// 実 FS の `read_dir` の検査の準備。列挙を拒んだディレクトリは `Drop` で戻す (一時ディレクトリを消せるように)。
#[derive(Default)]
struct NativeFixture {
    denied: RefCell<Vec<PathBuf>>,
}

/// 作る側のパス。Windows は verbatim の接頭辞を付けて、末尾のドット・空白と予約名をそのまま作る (05 §3.1)。
fn raw(p: &FsPath) -> PathBuf {
    #[cfg(windows)]
    {
        let mut s = std::ffi::OsString::from(r"\\?\");
        s.push(p.as_path().as_os_str());
        PathBuf::from(s)
    }
    #[cfg(not(windows))]
    {
        p.as_path().to_path_buf()
    }
}

/// シンボリックリンクを作れたか。権限が無い開発機 (Windows) では飛ばす。CI (`GITHUB_ACTIONS`) では作れるはずなので、
/// 作れなければ失敗にする (`tests/save_atomic.rs` と同じ扱い)。
fn made(r: std::io::Result<()>, what: &str) -> bool {
    match r {
        Ok(()) => true,
        Err(e) if std::env::var_os("GITHUB_ACTIONS").is_some() => {
            panic!("CI で{what}を作れません: {e}")
        }
        Err(e) => {
            eprintln!("{what}を作れない環境 ({e}): この項目を確かめない");
            false
        }
    }
}

fn run(cmd: &mut Command) -> std::io::Result<()> {
    let out = cmd.output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ))
    }
}

impl Fixture for NativeFixture {
    fn dir(&self, path: &FsPath) {
        fs::create_dir_all(raw(path)).expect("create_dir_all");
    }

    fn file(&self, path: &FsPath, data: &[u8]) {
        fs::write(raw(path), data).expect("write");
    }

    #[cfg(windows)]
    fn hidden_file(&self, path: &FsPath) {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_HIDDEN;
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .attributes(FILE_ATTRIBUTE_HIDDEN)
            .open(raw(path))
            .expect("隠しファイル");
    }

    #[cfg(windows)]
    fn offline_file(&self, path: &FsPath) -> bool {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_OFFLINE;
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .attributes(FILE_ATTRIBUTE_OFFLINE)
            .open(raw(path))
            .expect("オフラインの属性のファイル");
        let attrs = fs::metadata(raw(path)).expect("metadata").file_attributes();
        assert_ne!(attrs & FILE_ATTRIBUTE_OFFLINE, 0, "属性が付かない");
        true
    }

    fn symlink(&self, link: &FsPath, target: &FsPath, kind: FileKind) -> bool {
        #[cfg(windows)]
        let r = if kind == FileKind::Dir {
            std::os::windows::fs::symlink_dir(target.as_path(), link.as_path())
        } else {
            std::os::windows::fs::symlink_file(target.as_path(), link.as_path())
        };
        #[cfg(unix)]
        let r = {
            let _ = kind;
            std::os::unix::fs::symlink(target.as_path(), link.as_path())
        };
        made(r, "シンボリックリンク")
    }

    #[cfg(windows)]
    fn junction(&self, link: &FsPath, target: &FsPath) -> bool {
        // ジャンクションは権限なしで作れる
        run(Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link.as_path())
            .arg(target.as_path()))
        .expect("mklink /J");
        true
    }

    #[cfg(unix)]
    fn special(&self, path: &FsPath) -> bool {
        run(Command::new("mkfifo").arg(path.as_path())).expect("mkfifo");
        true
    }

    fn device_in(&self, dir: &FsPath) -> Option<FsPath> {
        // Unix は FIFO (書き手がいないと、ふつうに開くと止まる)、Windows は `NUL` (GetFileType が FILE_TYPE_CHAR)
        #[cfg(unix)]
        {
            let p = dir.join("fifo-open");
            self.special(&p);
            Some(p)
        }
        #[cfg(windows)]
        {
            Some(dir.join("NUL"))
        }
    }

    fn deny_list(&self, dir: &FsPath) -> bool {
        let p = dir.as_path();
        deny(p);
        self.denied.borrow_mut().push(p.to_path_buf());
        // 拒めない (Unix の root は権限を見ない) なら確かめない
        let denied = fs::read_dir(p).is_err();
        if !denied {
            eprintln!("列挙を拒めない環境 (root など): PermissionDenied を確かめない");
        }
        denied
    }
}

#[cfg(unix)]
fn deny(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(p, fs::Permissions::from_mode(0o000)).expect("chmod 000");
}

#[cfg(unix)]
fn allow(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(p, fs::Permissions::from_mode(0o755));
}

/// Everyone (S-1-1-0) にディレクトリの一覧 (RD) を拒む ACE を足す。
#[cfg(windows)]
fn deny(p: &Path) {
    run(Command::new("icacls")
        .arg(p)
        .args(["/deny", "*S-1-1-0:(RD)"]))
    .expect("icacls /deny");
}

#[cfg(windows)]
fn allow(p: &Path) {
    let _ = run(Command::new("icacls")
        .arg(p)
        .args(["/remove:d", "*S-1-1-0"]));
}

impl Drop for NativeFixture {
    fn drop(&mut self) {
        for p in self.denied.borrow().iter() {
            allow(p);
        }
    }
}

fn caps(state_dir: bool) -> Caps {
    Caps {
        state_dir,
        ..Caps::default()
    }
}

#[test]
fn 状態ディレクトリを持つ実_fs_は_filesystem_の契約を満たす() {
    suite(|| {
        let t = Tmp::new("state");
        let fsys = NativeFs::new(t.0.join("state"));
        Subject::new(Arc::new(fsys), FsPath::new(t.0.join("work")))
            .caps(caps(true))
            .lister(names)
            .fixture(NativeFixture::default())
            .keep(t)
    });
}

#[test]
fn 状態ディレクトリを持たない実_fs_は_filesystem_の契約を満たす() {
    suite(|| {
        let t = Tmp::new("nostate");
        Subject::new(Arc::new(NativeFs::default()), FsPath::new(t.0.join("work")))
            .caps(caps(false))
            .lister(names)
            .fixture(NativeFixture::default())
            .keep(t)
    });
}
