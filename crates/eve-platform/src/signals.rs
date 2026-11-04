//! Unix シグナルの受信 (docs/05 §11.1 / §11.5、02 §9.1 の「シグナル / コンソールハンドラ」)。Windows の `console` に当たる。
//!
//! `signal-hook` でシグナルを自己パイプに変え、**専用のシグナルスレッド** (`eve-signal`) が受けて呼び出し側のコールバックへ渡す
//! (シグナルハンドラの中では何もしない)。緊急退避・終了コードの決定・端末の復元は呼び側 (eve-host / bin) の責務で、
//! コールバックはこのスレッドで呼ばれるので、退避を直接書いてよい (§11.4 の「役割分担」)。
//!
//! 終了系のシグナル (INT / TERM / HUP) で終わるときは、後始末の後に `die_by_signal` でハンドラを既定に戻して自分に再送し、
//! シェルが「シグナルによる終了」(`128 + signo`) と認識できるようにする (§11.5)。

#![allow(unsafe_code)]

use std::io;

use signal_hook::consts::{SIGCONT, SIGHUP, SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

/// 受けるシグナル。SIGWINCH は入力の経路 (端末のリサイズ)、SIGTSTP はサスペンド (§11.3) で別に扱う。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnixSignal {
    /// SIGINT (`kill -INT`。raw モードの Ctrl+C では来ない)。
    Interrupt,
    /// SIGTERM。
    Terminate,
    /// SIGHUP (端末が閉じた・SSH の切断。端末は既に無い)。
    Hangup,
    /// SIGCONT (`fg`。端末の再初期化と全面の再描画)。
    Continue,
}

impl UnixSignal {
    /// シグナル番号 (終了コード `128 + signo` と再送に使う)。
    pub fn signo(self) -> i32 {
        match self {
            UnixSignal::Interrupt => SIGINT,
            UnixSignal::Terminate => SIGTERM,
            UnixSignal::Hangup => SIGHUP,
            UnixSignal::Continue => SIGCONT,
        }
    }

    fn from_signo(n: i32) -> Option<UnixSignal> {
        match n {
            SIGINT => Some(UnixSignal::Interrupt),
            SIGTERM => Some(UnixSignal::Terminate),
            SIGHUP => Some(UnixSignal::Hangup),
            SIGCONT => Some(UnixSignal::Continue),
            _ => None,
        }
    }

    /// 終了を求めるシグナルか (INT / TERM / HUP)。
    pub fn is_termination(self) -> bool {
        !matches!(self, UnixSignal::Continue)
    }
}

/// シグナルの受信を始める。`cb` はシグナルスレッドで、届いた順に 1 つずつ呼ばれる (戻るまで次は呼ばない)。
/// 登録した後は、これらのシグナルの既定の動作 (終了) は起きない (呼び側が終了を決める)。
pub fn start(cb: impl Fn(UnixSignal) + Send + 'static) -> io::Result<()> {
    let mut signals = Signals::new([SIGINT, SIGTERM, SIGHUP, SIGCONT])?;
    // シグナルスレッド (02 §9.1 の表。deps-policy.toml の R11 の許可)
    #[allow(clippy::disallowed_methods)]
    std::thread::Builder::new()
        .name("eve-signal".to_owned())
        .spawn(move || {
            for n in signals.forever() {
                if let Some(s) = UnixSignal::from_signo(n) {
                    cb(s);
                }
            }
        })?;
    Ok(())
}

/// 後始末を終えた後、`signo` のハンドラを既定 (`SIG_DFL`) に戻して自分に再送し、OS に終了させる (§11.5)。
/// 再送しても終わらなかった (無視・保留された) ときだけ `_exit(128 + signo)` で終わる。
pub fn die_by_signal(signo: i32) -> ! {
    // SAFETY: 既定の動作への付け替え・自分の信号マスクからの除去・自分への送信・即時の終了は、いずれも引数の値だけを使う。
    unsafe {
        libc::signal(signo, libc::SIG_DFL);
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, signo);
        libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
        libc::raise(signo);
        libc::_exit(128 + signo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn 届いたシグナルをシグナルスレッドで受け_既定の終了は起きない() {
        let (tx, rx) = mpsc::channel();
        start(move |s| {
            let _ = tx.send((s, std::thread::current().name().map(str::to_owned)));
        })
        .unwrap();
        for (signo, want) in [
            (SIGHUP, UnixSignal::Hangup),
            (SIGTERM, UnixSignal::Terminate),
            (SIGCONT, UnixSignal::Continue),
            (SIGINT, UnixSignal::Interrupt),
        ] {
            // SAFETY: 自分へのシグナルの送信 (受信は登録済み)。
            unsafe { libc::raise(signo) };
            let (got, thread) = rx.recv_timeout(Duration::from_secs(5)).expect("届く");
            assert_eq!(got, want);
            assert_eq!(got.signo(), signo);
            assert_eq!(thread.as_deref(), Some("eve-signal"));
        }
        assert!(UnixSignal::Hangup.is_termination() && !UnixSignal::Continue.is_termination());
    }

    #[test]
    fn die_by_signal_はシグナルによる終了になる() {
        // SAFETY: 子は fork の直後に非同期シグナル安全な呼出し (signal / sigprocmask / raise / _exit) だけを行う。
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0);
        if pid == 0 {
            die_by_signal(SIGTERM);
        }
        let mut status = 0;
        // SAFETY: 自分の子を待つ。
        assert_eq!(unsafe { libc::waitpid(pid, &mut status, 0) }, pid);
        assert!(libc::WIFSIGNALED(status), "{status:#x}");
        assert_eq!(libc::WTERMSIG(status), SIGTERM);
    }
}
