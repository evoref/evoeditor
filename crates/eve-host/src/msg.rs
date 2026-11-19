//! 単一受信口のメッセージ (docs/02 §9.2)。変種は最初の利用者の版で足す。

use eve_core::JobOutput;
use eve_view::{InputEvent, WindowId};

/// プロセスの終了コード (値の正本は 07 §18.4)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitCode(pub u8);

impl ExitCode {
    /// 正常終了 (0)。
    pub const OK: ExitCode = ExitCode(0);
    /// 非対話の終了要求 (シグナル・コンソールの閉鎖) の既定。
    pub const INTERRUPTED: ExitCode = ExitCode(130);
    /// 内部エラー (入力スレッドの死亡など)。
    pub const FATAL: ExitCode = ExitCode(70);
}

/// シグナル・コンソールイベントの種類 (Windows のコンソールイベントと Unix のシグナル、05 §11.1 / §11.2)。どれも非対話の終了要求。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SignalKind {
    /// 割り込み (Unix の SIGINT、Windows の `CTRL_C_EVENT` / `CTRL_BREAK_EVENT`)。
    Interrupt,
    /// 終了要求 (Unix の SIGTERM、Windows のセッション終了)。
    Terminate,
    /// Windows のコンソールの閉鎖 (`CTRL_CLOSE_EVENT`)。
    ConsoleClose,
    /// SIGHUP (端末が閉じた・SSH の切断)。端末は既に無いので、復元の失敗は無視する。
    Hangup,
}

/// 致命的な内部エラーの理由 (`HostMsg::Fatal`)。
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FatalReason {
    /// 入力スレッドが死んだ。
    InputThreadDied,
    /// 非メインスレッドのパニック。
    ThreadPanic(String),
}

/// 受信口に届くメッセージ。`#[non_exhaustive]`: 外のループで `match` するときは既定の腕を持つ。
#[derive(Debug)]
#[non_exhaustive]
pub enum HostMsg {
    /// 入力スレッド (TUI)。
    Input(WindowId, InputEvent),
    /// `io` / `bg` ワーカー。
    Job(Box<JobOutput>),
    /// シグナル・コンソールイベント。
    Signal(SignalKind),
    /// 入力スレッドの死亡・非メインスレッドのパニック。
    Fatal(FatalReason),
    /// クリップボード取得の結果 (使い捨てのスレッドから、02 §9.1)。token が待っているものと違えば捨てる。
    Clipboard {
        /// 取得を要求したときの token。
        token: eve_view::ClipboardToken,
        /// 取得した本文。クリップボードが空なら `Ok(None)`。
        result: Result<Option<String>, eve_platform::clipboard::ClipboardFailure>,
    },
    /// システムクリップボードへの書込の失敗 (使い捨てのスレッドから)。
    ClipboardWriteFailed(eve_platform::clipboard::ClipboardFailure),
    /// OSC 52 で書く本文 (使い捨てのスレッドから、05 §8.1)。`system_failure` はシステムのクリップボードへの書込の失敗
    /// (OSC 52 も書けなかったときに、内部レジスタに切り替えたと知らせる)。
    Osc52 {
        /// 書く本文 (base64 に符号化する前の文字列)。
        text: String,
        /// システムのクリップボードへの書込の失敗。成功した (または試みなかった) ときは `None`。
        system_failure: Option<eve_platform::clipboard::ClipboardFailure>,
    },
    /// `run()` の待機が期限で終わった (メッセージなし)。
    Tick,
}
