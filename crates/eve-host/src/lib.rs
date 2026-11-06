//! Composition Root の共通部: Port の注入、単一受信口、メインループ (docs/02 §4.1 / §9)。
//!
//! 持つもの: `Frontend` trait、`HostMsg`、`Host::step` / `Host::run`、システム時計、本番の `JobSpawner` (`ThreadJobSpawner`)、
//! 緊急退避の制御 (`EmergencyCtl`)、非対話の終了の手順 (`NonInteractiveShutdown`)。

#![forbid(unsafe_code)]

mod clock;
mod emergency;
mod frontend;
mod host;
mod msg;
mod shutdown;
mod workers;

pub use clock::SystemClock;
pub use emergency::{EmergencyCtl, EmergencyState};
pub use eve_platform::clipboard::{ClipboardFailure, ClipboardProvider};
pub use frontend::{
    EffectOutcome, EmergencyRestoreInfo, Frontend, FrontendConfig, FrontendError,
    FrontendErrorKind, InputSink, RenderOutcome,
};
pub use host::{
    Host, RESCUE_BUDGET_CLOSE, RESCUE_BUDGET_FATAL, RESCUE_BUDGET_SESSION, RESCUE_BUDGET_SIGNAL,
    StartError, StepOutcome, channel,
};
pub use msg::{ExitCode, FatalReason, HostMsg, SignalKind};
pub use shutdown::{
    EMERGENCY_RESTORE_RESERVE, MAIN_EXIT_WAIT, NonInteractiveShutdown, REPLACING_DEADLINE_CLOSE,
    REPLACING_DEADLINE_SIGNAL, REPLACING_WAIT_FATAL, SIGNAL_TOTAL_LIMIT, ShutdownEnd,
    ShutdownParts, ShutdownTrigger,
};
pub use workers::ThreadJobSpawner;
