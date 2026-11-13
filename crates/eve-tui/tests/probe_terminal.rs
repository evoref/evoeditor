//! 端末を使えるかの確かめ (`probe_terminal`、docs/03 §9.2.2 の「端末の初期化の時期」)。標準入力を移して読み始める前に
//! Host (bin) が呼び、端末を開けなければ標準入力を読まずに終わる。端末の無い子プロセス (Unix は `setsid` で制御端末を
//! 外す、Windows はコンソールを持たない `DETACHED_PROCESS`) で、この試験自身を起こして確かめる。

use std::process::Command;

/// 子プロセスとして起こされたときの印。
const CHILD: &str = "EVE_PROBE_TERMINAL_CHILD";

#[test]
fn probe_child() {
    // 子プロセスの中だけで意味を持つ (ふつうに走らせたときは何もしない)
    if std::env::var_os(CHILD).is_none() {
        return;
    }
    let r = eve_tui::frontend::probe_terminal();
    std::process::exit(if r.is_err() { 0 } else { 1 });
}

#[test]
fn 端末の無いプロセスでは使えないと判定する() {
    let mut cmd = Command::new(std::env::current_exe().expect("試験のバイナリ"));
    cmd.args(["--exact", "probe_child", "--nocapture", "--test-threads=1"])
        .env(CHILD, "1")
        .env("TERM", "xterm-256color")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: fork と exec の間で、非同期シグナル安全な `setsid` だけを呼ぶ。
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS: コンソールを持たない (`CONIN$` / `CONOUT$` を開けない)
        cmd.creation_flags(0x0000_0008);
    }
    let status = cmd.status().expect("子プロセスを起こせる");
    assert_eq!(status.code(), Some(0), "端末が無いので使えないと判定する");
}

#[test]
fn term_が_dumb_なら使えない() {
    // `TERM=dumb` は端末を開けても使えない (`init` と同じ判定)。子プロセスで環境を変えて確かめる
    let status = Command::new(std::env::current_exe().expect("試験のバイナリ"))
        .args(["--exact", "probe_child", "--nocapture", "--test-threads=1"])
        .env(CHILD, "1")
        .env("TERM", "dumb")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("子プロセスを起こせる");
    assert_eq!(status.code(), Some(0));
}
