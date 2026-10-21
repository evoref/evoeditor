//! 単一受信口のメッセージ (docs/02 §9.2)。変種は最初の利用者の版で足す。

use eve_core::JobResult;
use eve_view::{InputEvent, WindowId};

/// プロセスの終了コード (値の正本は 07 §18.4)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitCode(pub u8);

impl ExitCode {
    pub const OK: ExitCode = ExitCode(0);
    /// 非対話の終了要求 (シグナル・コンソールの閉鎖) の既定。
    pub const INTERRUPTED: ExitCode = ExitCode(130);
    /// 内部エラー (入力スレッドの死亡など)。
    pub const FATAL: ExitCode = ExitCode(70);
}

/// シグナル・コンソールイベントの種類 (v0.1 は Windows のコンソールイベント)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SignalKind {
    Interrupt,
    Terminate,
    ConsoleClose,
}

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
    Job(Box<JobResult>),
    /// シグナル・コンソールイベント。
    Signal(SignalKind),
    /// 入力スレッドの死亡・非メインスレッドのパニック。
    Fatal(FatalReason),
    /// クリップボード取得の結果 (使い捨てのスレッドから、02 §9.1)。token が待っているものと違えば捨てる。
    Clipboard {
        token: eve_view::ClipboardToken,
        result: Result<Option<String>, eve_platform::clipboard::ClipboardFailure>,
    },
    /// システムクリップボードへの書込の失敗 (使い捨てのスレッドから)。
    ClipboardWriteFailed(eve_platform::clipboard::ClipboardFailure),
    /// `run()` の待機が期限で終わった (メッセージなし)。
    Tick,
}
