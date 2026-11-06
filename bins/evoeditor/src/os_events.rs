//! OS からの終了の要求 (05 §11) を、非対話の終了の手順 (`NonInteractiveShutdown`、02 §9.6 / §11.2.1) へつなぐ。
//! 手順と予算は eve-host が持ち、ここでは OS のコールバックへの登録と終わり方だけを書く。

#[cfg(unix)]
use std::io::Write as _;

use eve_host::NonInteractiveShutdown;
use eve_host::{ShutdownEnd, ShutdownTrigger, SignalKind};

#[cfg(windows)]
use crate::texts;

/// Unix で終了を求めたシグナルの番号 (0 = なし)。メインの後始末の後、同じシグナルで終わる (05 §11.5)。
#[cfg(unix)]
static EXIT_SIGNAL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// OS のコールバックを登録する (Unix のシグナル、Windows のコンソールのコントロールイベントとログオフ・シャットダウン)。
pub(crate) fn register(shutdown: &NonInteractiveShutdown) {
    // Unix のシグナル (05 §11.1): 受信スレッドが手順を行い、メインが終わらなければ同じシグナルで終わる (05 §11.5)。
    // SIGCONT は入力スレッドが扱う (端末の再初期化) ので、ここでは何もしない
    #[cfg(unix)]
    {
        let shutdown = shutdown.clone();
        let _ = eve_platform::signals::start(move |sig| {
            use eve_platform::signals::UnixSignal;
            if !sig.is_termination() {
                return;
            }
            let kind = match sig {
                UnixSignal::Interrupt => SignalKind::Interrupt,
                UnixSignal::Hangup => SignalKind::Hangup,
                _ => SignalKind::Terminate,
            };
            // 最初の終了のシグナルを覚える (メインが終わった後、同じシグナルで終わる、05 §11.5)
            let _ = EXIT_SIGNAL.compare_exchange(
                0,
                sig.signo(),
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            );
            if shutdown.run(ShutdownTrigger::Signal(kind)) == ShutdownEnd::Exit {
                eve_platform::signals::die_by_signal(sig.signo())
            }
        });
    }

    // コンソールのコントロールイベント (Ctrl+Break・ウィンドウの閉鎖など、05 §11.2)。ハンドラのスレッドは OS が作る。
    // CTRL_CLOSE は手順の後にハンドラから戻る (プロセスは OS が終わらせる)。CTRL_C / CTRL_BREAK は、メインが終わらなければ 130 で終わる
    #[cfg(windows)]
    {
        let shutdown = shutdown.clone();
        let _ = eve_platform::console::install_handler(move |ev| {
            use eve_platform::console::CtrlEvent;
            let trigger = match ev {
                CtrlEvent::Close => ShutdownTrigger::ConsoleClose,
                CtrlEvent::CtrlC | CtrlEvent::CtrlBreak => {
                    ShutdownTrigger::Signal(SignalKind::Interrupt)
                }
                _ => ShutdownTrigger::Signal(SignalKind::Terminate),
            };
            match shutdown.run(trigger) {
                ShutdownEnd::NotSent => false,
                ShutdownEnd::Exit => {
                    std::process::exit(i32::from(eve_host::ExitCode::INTERRUPTED.0))
                }
                _ => true,
            }
        });
    }

    // ログオフ・シャットダウン (v0.2、ベストエフォート。05 §11.2 の「セッション終了の受け方」): 専用スレッドの非表示の
    // トップレベルウィンドウで受ける。問い合わせ (`WM_QUERYENDSESSION`、使い捨てのスレッドで呼ばれる) で退避を始め、
    // 終了の通知 (`WM_ENDSESSION`) で手順を行って戻る。取り消しでは何もしない
    #[cfg(windows)]
    {
        let shutdown = shutdown.clone();
        let _ = eve_platform::session::start(&texts::session_block_reason(), move |ev| {
            use eve_platform::session::SessionEvent;
            match ev {
                SessionEvent::QueryEnd => {
                    let _ = shutdown.run(ShutdownTrigger::SessionQuery);
                }
                SessionEvent::End { ending: true } => {
                    let _ = shutdown.run(ShutdownTrigger::SessionEnd);
                }
                _ => {}
            }
        });
    }
}

/// Unix: シグナルで終わったなら、後始末 (退避・端末の復元・終了時の行) の後に同じシグナルで終わる (05 §11.5)。
/// それ以外は何もしない。
pub(crate) fn die_if_signaled() {
    #[cfg(unix)]
    {
        let s = EXIT_SIGNAL.load(std::sync::atomic::Ordering::Acquire);
        if s != 0 {
            let _ = std::io::stderr().flush();
            eve_platform::signals::die_by_signal(s);
        }
    }
}
