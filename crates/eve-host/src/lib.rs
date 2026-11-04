//! Composition Root の共通部: Port の注入、単一受信口、メインループ (docs/02 §4.1 / §9)。
//! G1a で着手するのは端末に依存しない部分 (`Host::step`・`Frontend` trait・受信口)。
//!
//! 現在の範囲 (M1 の D1a): `Frontend` trait、`HostMsg`、`Host::step` / `Host::run`、システム時計。
//! 本番の `JobSpawner` (固定 2 本のワーカー)、緊急退避の書き手、シグナル・クリップボードは、それぞれの版で足す。

#![forbid(unsafe_code)]

mod clock;
mod emergency;
mod frontend;
mod host;
mod msg;
mod workers;

pub use clock::SystemClock;
pub use emergency::{EmergencyCtl, EmergencyState};
pub use eve_platform::clipboard::{ClipboardFailure, ClipboardProvider};
pub use frontend::{
    EffectOutcome, EmergencyRestoreInfo, Frontend, FrontendConfig, FrontendError, InputSink,
    RenderOutcome,
};
pub use host::{
    Host, RESCUE_BUDGET_CLOSE, RESCUE_BUDGET_FATAL, RESCUE_BUDGET_SESSION, RESCUE_BUDGET_SIGNAL,
    StartError, StepOutcome, channel,
};
pub use msg::{ExitCode, FatalReason, HostMsg, SignalKind};
pub use workers::ThreadJobSpawner;
