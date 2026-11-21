//! Unix の `open_read` が、書込のリース (`F_SETLEASE`) の掛かった通常のファイルを読めること (docs/05 §2.1 の `open_read()` の行)。
//! `O_NONBLOCK` の `open` は `EWOULDBLOCK` で失敗するので、非ブロックなしで開き直してリースが外れるのを待つ (v0.4 までの
//! `File::open` と同じ振る舞い)。リースを掛けられない環境 (FS・権限) では飛ばす。

#![cfg(target_os = "linux")]
#![allow(clippy::print_stderr)]
// リースを掛けたまま別のスレッドで開くため (試験だけ。R11 の対象は製品のコード)
#![allow(clippy::disallowed_methods)]

use std::fs::{self, File};
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::time::Duration;

use eve_core::{FileSystem, FsPath};
use eve_platform::unix_fs::UnixFs;

/// リースを外すまでの時間 (読む側が開き直して待つ間)。
const HOLD: Duration = Duration::from_millis(300);

struct Tmp(PathBuf);

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn 書込のリースが掛かったファイルもリースが外れるのを待って読める() {
    let t = Tmp(std::env::temp_dir().join(format!("eve-lease-{}", std::process::id())));
    let _ = fs::remove_dir_all(&t.0);
    fs::create_dir_all(&t.0).expect("一時ディレクトリ");
    let p = t.0.join("leased.txt");
    fs::write(&p, b"leased").expect("write");

    // SAFETY: SIGIO を無視にするだけ (リースを破る知らせの既定の動作はプロセスの終了なので)。ほかの状態を変えない。
    unsafe { libc::signal(libc::SIGIO, libc::SIG_IGN) };
    let holder = File::open(&p).expect("open");
    // SAFETY: `holder` が持つ開いた記述子に書込のリースを掛けるだけ。記述子はこの関数の終わりまで開いている。
    if unsafe { libc::fcntl(holder.as_raw_fd(), libc::F_SETLEASE, libc::F_WRLCK) } != 0 {
        eprintln!(
            "書込のリースを掛けられない環境 ({}): この試験を飛ばす",
            std::io::Error::last_os_error()
        );
        return;
    }

    let target = FsPath::new(&p);
    let reader = std::thread::spawn(move || {
        let mut r = UnixFs::default().open_read(&target)?;
        let mut out = Vec::new();
        while let Some(c) = r.next_chunk().map_err(|e| match e {
            eve_core::ChunkError::Io(e) => e,
            other => panic!("読込側は Io だけ: {other:?}"),
        })? {
            out.extend_from_slice(c);
        }
        Ok::<_, eve_core::FsError>(out)
    });
    std::thread::sleep(HOLD);
    // SAFETY: 同じ記述子のリースを外すだけ。
    unsafe { libc::fcntl(holder.as_raw_fd(), libc::F_SETLEASE, libc::F_UNLCK) };
    let got = reader.join().expect("読む側のスレッド");
    assert_eq!(got.expect("リースが外れた後に読める"), b"leased");
}
