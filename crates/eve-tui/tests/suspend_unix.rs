//! サスペンド (`app.suspend`、docs/05 §11.3) はプロセスグループ全体を止める (`kill(0, SIGTSTP)`。v0.4 の M25 で改めた)。
//! 書き手が生きているパイプライン (`tail -f log | evoeditor -`) でも、前面のジョブ全体が止まってシェルに戻るため。
//!
//! 子プロセスを新しいプロセスグループにし、同じグループに孫を置いてから子が `stop_job` を呼ぶ。親 (この試験) は、子と
//! 孫の両方が止まったことを確かめる (孫は `/proc/<pid>/stat` の状態 `T`)。

#![cfg(target_os = "linux")]

use std::time::{Duration, Instant};

/// `/proc/<pid>/stat` の状態の文字 (3 つ目の欄。名前の括弧の後)。
fn proc_state(pid: libc::pid_t) -> Option<char> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = &s[s.rfind(')')? + 1..];
    rest.trim_start().chars().next()
}

#[test]
fn サスペンドはプロセスグループ全体を止める() {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: `fds` は 2 つの fd を書く有効な配列。
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    // SAFETY: fork の後の子では、非同期シグナル安全な呼出し (setpgid / fork / pause / write / kill / _exit) だけを行う。
    let child = unsafe { libc::fork() };
    assert!(child >= 0, "fork");
    if child == 0 {
        // 子: 新しいプロセスグループを作り、同じグループに眠る孫を置いて、グループごと止める
        // SAFETY: 子の中の非同期シグナル安全な呼出しだけ。
        unsafe {
            libc::setpgid(0, 0);
            let gc = libc::fork();
            if gc == 0 {
                loop {
                    libc::pause();
                }
            }
            libc::write(
                fds[1],
                (&raw const gc).cast::<libc::c_void>(),
                std::mem::size_of::<libc::pid_t>(),
            );
            eve_tui::input::unix::stop_job();
            libc::_exit(0);
        }
    }
    // 親: 孫の pid を受け取る
    let mut gc: libc::pid_t = 0;
    // SAFETY: `gc` は pid_t の大きさの書込先。
    let n = unsafe {
        libc::read(
            fds[0],
            (&raw mut gc).cast::<libc::c_void>(),
            std::mem::size_of::<libc::pid_t>(),
        )
    };
    assert_eq!(
        usize::try_from(n).ok(),
        Some(std::mem::size_of::<libc::pid_t>())
    );
    // 子が止まった (WUNTRACED)
    let mut status = 0;
    // SAFETY: 自分の子を待つ。`status` は有効な書込先。
    let r = unsafe { libc::waitpid(child, &raw mut status, libc::WUNTRACED) };
    let child_stopped = r == child && libc::WIFSTOPPED(status);
    // 孫も止まった (同じグループ)。止まるまで少し待つ
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut gc_state = proc_state(gc);
    while gc_state != Some('T') && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
        gc_state = proc_state(gc);
    }
    // 後始末: グループごと終わらせる
    // SAFETY: 試験が作ったプロセスグループにだけ送る。
    unsafe {
        libc::kill(-child, libc::SIGKILL);
        libc::waitpid(child, &raw mut status, 0);
        libc::close(fds[0]);
        libc::close(fds[1]);
    }
    assert!(child_stopped, "サスペンドを呼んだプロセスが止まる");
    assert_eq!(
        gc_state,
        Some('T'),
        "同じプロセスグループのほかのプロセスも止まる"
    );
}
