//! `Frontend` trait (docs/02 §9.2)。frontend は入力の源と描画先を提供し、Host がループを回す。
//! 緊急の端末復元の情報 (`emergency_restore_info`) は、端末を持つ frontend (eve-tui) と同時に足す。

use std::fmt;
use std::sync::mpsc::Sender;
use std::time::Duration;

use eve_view::{
    Capabilities, CellSize, Effect, Frame, InputEvent, RawTermHandle, SavedTermMode,
    TermRestoredFlag, WindowId,
};

use crate::msg::HostMsg;

/// Host が解決済み設定から作って `Frontend::init` に渡す。載せるのは frontend が読む公開キー (07 §5.0) だけ。
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct FrontendConfig {
    /// `view.ambiguous_width` (1 または 2)。0 は未設定。
    pub ambiguous_width: u8,
    /// `ui.mouse`: 起動時にマウスの取得を有効にするか (04 §6.8)。
    pub mouse: bool,
}

/// `Frontend::init` / `start_input` の失敗。利用者への文言は bin が `kind` から UI の言語で作る (日本語の文字列を運ばない、
/// 01 NFR-603。docs/review/v0.3-findings.md の STR-C2)。`detail` は OS の誤りの文 (OS が作ったもの)。
#[derive(Debug)]
pub struct FrontendError {
    /// 失敗の種類。bin が UI の言語の文言を選ぶ手掛かり。
    pub kind: FrontendErrorKind,
    /// OS の誤りの文。無ければ `None`。
    pub detail: Option<String>,
}

/// `FrontendError` の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FrontendErrorKind {
    /// `TERM=dumb` など、使えない端末 (04 §5.2)。
    UnusableTerminal,
    /// 描画先のコンソール・端末を開けない、または VT モードにできない。
    ConsoleOutput,
    /// 入力のコンソール・端末を設定できない。
    ConsoleInput,
    /// `init` の前に `start_input` が呼ばれた (呼び側の誤り)。
    InputBeforeInit,
    /// 入力スレッドを起動できない。
    InputThread,
    /// そのほか (外部の frontend の実装が使う)。
    Other,
}

impl FrontendErrorKind {
    /// 文言のキーの末尾 (bin の stderr の表で `stderr.reason.frontend.<key>`)。
    pub fn key(self) -> &'static str {
        match self {
            FrontendErrorKind::UnusableTerminal => "unusable_terminal",
            FrontendErrorKind::ConsoleOutput => "console_output",
            FrontendErrorKind::ConsoleInput => "console_input",
            FrontendErrorKind::InputBeforeInit => "input_before_init",
            FrontendErrorKind::InputThread => "input_thread",
            FrontendErrorKind::Other => "other",
        }
    }
}

impl FrontendError {
    /// 詳細なしで作る。
    pub fn new(kind: FrontendErrorKind) -> FrontendError {
        FrontendError { kind, detail: None }
    }

    /// OS の誤りを詳細に持つ。
    pub fn io(kind: FrontendErrorKind, e: &std::io::Error) -> FrontendError {
        FrontendError {
            kind,
            detail: Some(e.to_string()),
        }
    }
}

impl fmt::Display for FrontendError {
    /// ログのための英語の短い文 (利用者への文言は bin)。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "frontend error ({})", self.kind.key())?;
        if let Some(d) = &self.detail {
            write!(f, ": {d}")?;
        }
        Ok(())
    }
}

impl std::error::Error for FrontendError {}

/// 描画の結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RenderOutcome {
    /// 描き終えた。
    Done,
    /// 描いたら編集面の大きさが変わっていた (予約。v0.1 の TUI は常に `Done`)。
    SurfaceChanged {
        /// 新しい編集面の列数 (セル)。
        cols: u16,
        /// 新しい編集面の行数 (セル)。
        rows: u16,
    },
}

/// `apply_effect` の結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectOutcome {
    /// frontend が処理した (Host は何もしない)。
    Handled,
    /// frontend は扱わない (Host が処理する: 終了・クリップボード等)。
    Unhandled,
}

/// 入力を受信口へ送る口。入力スレッド (または外のループ) が `HostMsg::Input` を送る。
#[derive(Clone, Debug)]
pub struct InputSink {
    tx: Sender<HostMsg>,
    window: WindowId,
}

impl InputSink {
    pub(crate) fn new(tx: Sender<HostMsg>, window: WindowId) -> InputSink {
        InputSink { tx, window }
    }

    /// 入力を送る。受信側が閉じていれば `false`。
    pub fn send(&self, ev: InputEvent) -> bool {
        self.tx.send(HostMsg::Input(self.window, ev)).is_ok()
    }

    /// 任意のメッセージを送る (外のループ・テスト用)。
    pub fn send_msg(&self, msg: HostMsg) -> bool {
        self.tx.send(msg).is_ok()
    }
}

/// 緊急の端末復元の情報 (`Frontend::emergency_restore_info`)。`Send + Sync`。
#[derive(Clone, Debug)]
pub struct EmergencyRestoreInfo {
    /// 描画先の生ハンドル。
    pub handle: RawTermHandle,
    /// 先頭の CAN (0x18) + 復元の固定のバイト列 (04 §14.3)。
    pub bytes: &'static [u8],
    /// 起動時に保存したモード。
    pub saved_mode: SavedTermMode,
    /// 「復元済み」の原子フラグ (frontend の `restore` と共有)。
    pub restored: TermRestoredFlag,
}

/// 端末など、描画と入力の実装 (docs/02 §9.2)。Host が起動時に `init`、`start_input` を呼び、ループの中で
/// `render` と `apply_effect` を呼び、どの終了経路でも `restore` を呼ぶ。
pub trait Frontend {
    /// 端末の確認・初期化・能力の判定。Host が設定を読んだ後、Editor を作る前に 1 回だけ呼ぶ。失敗なら起動エラー。
    ///
    /// # Errors
    ///
    /// 端末が使えない・コンソールを設定できないときの `FrontendError` (種類は `FrontendErrorKind`)。
    fn init(&mut self, cfg: &FrontendConfig) -> Result<Capabilities, FrontendError>;
    /// 入力の源を起動する。`tx` は受信口への送信口。
    ///
    /// # Errors
    ///
    /// 入力のコンソール・端末を設定できない、または入力スレッドを起動できないときの `FrontendError`。
    fn start_input(&mut self, tx: InputSink) -> Result<(), FrontendError>;
    /// 編集面の大きさ (最初の `Resize` 用)。
    fn surface_size(&self) -> CellSize;
    /// `Frame` を描く。
    fn render(&mut self, frame: &Frame) -> RenderOutcome;
    /// Host が処理する前に frontend が扱いたい `Effect`。
    fn apply_effect(&mut self, eff: &Effect) -> EffectOutcome;
    /// 端末の復元 (どの終了経路でも)。
    fn restore(&mut self);
    /// frontend 自身の時間で再描画が要るまでの待ち時間 (視覚ベルの復帰など)。要らなければ `None`。
    fn wake_after(&self) -> Option<Duration> {
        None
    }
    /// 受信スレッドが Frontend に触れずに端末を戻すための情報 (02 §9.2、§9.6 手順 4)。`init` の後に 1 回取得する。
    /// 端末を持たない frontend は `None`。
    fn emergency_restore_info(&self) -> Option<EmergencyRestoreInfo> {
        None
    }
    /// OSC 52 でクリップボードへ書く (`Capabilities.osc52_write` が真の frontend だけ実装する)。書けたら `true`。
    fn write_osc52(&mut self, _text: &str) -> bool {
        false
    }
    /// 端末を一時的に手放す (サスペンド、`Effect::Suspend` など)。既定では何もしない。
    fn suspend(&mut self) {}
    /// `suspend` の後に端末を取り戻す。既定では何もしない。
    fn resume(&mut self) {}
}
