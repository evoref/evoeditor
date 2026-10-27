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
}

#[derive(Debug)]
pub struct FrontendError(pub String);

impl fmt::Display for FrontendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "frontend の初期化に失敗しました: {}", self.0)
    }
}

impl std::error::Error for FrontendError {}

/// 描画の結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RenderOutcome {
    Done,
    /// 描いたら編集面の大きさが変わっていた (予約。v0.1 の TUI は常に `Done`)。
    SurfaceChanged {
        cols: u16,
        rows: u16,
    },
}

/// `apply_effect` の結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectOutcome {
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

pub trait Frontend {
    /// 端末の確認・初期化・能力の判定。Host が設定を読んだ後、Editor を作る前に 1 回だけ呼ぶ。失敗なら起動エラー。
    fn init(&mut self, cfg: &FrontendConfig) -> Result<Capabilities, FrontendError>;
    /// 入力の源を起動する。`tx` は受信口への送信口。
    fn start_input(&mut self, tx: InputSink) -> Result<(), FrontendError>;
    /// 編集面の大きさ (最初の `Resize` 用)。
    fn surface_size(&self) -> CellSize;
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
    fn suspend(&mut self) {}
    fn resume(&mut self) {}
}
