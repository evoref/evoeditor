//! Composition Root の共通部: Port の注入、単一受信口、メインループ (docs/02 §4.1 / §9)。
//!
//! 持つもの: `Frontend` trait、`HostMsg`、`Host::step` / `Host::run`、システム時計、本番の `JobSpawner` (`ThreadJobSpawner`)、
//! 緊急退避の制御 (`EmergencyCtl`)、非対話の終了の手順 (`NonInteractiveShutdown`)、Host の作業の口 (`HostTasks`) と
//! ファイル監視の組み立て (`file_watcher`)。

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod clock;
mod emergency;
mod frontend;
mod host;
mod msg;
mod shutdown;
mod watch;
mod workers;

pub use clock::SystemClock;
pub use emergency::{EmergencyCtl, EmergencyState};
pub use eve_platform::clipboard::{ClipboardFailure, ClipboardProvider};
pub use frontend::{
    EffectOutcome, EmergencyRestoreInfo, Frontend, FrontendConfig, FrontendError,
    FrontendErrorKind, InputSink, RenderOutcome,
};
pub use host::{
    Host, HostInit, RESCUE_BUDGET_CLOSE, RESCUE_BUDGET_FATAL, RESCUE_BUDGET_SESSION,
    RESCUE_BUDGET_SIGNAL, RESCUE_BUDGET_UNBOUNDED, StartError, StepOutcome, channel,
};
pub use msg::{ExitCode, FatalReason, HostMsg, SignalKind};
pub use shutdown::{
    EMERGENCY_RESTORE_RESERVE, MAIN_EXIT_WAIT, NonInteractiveShutdown, REPLACING_DEADLINE_CLOSE,
    REPLACING_DEADLINE_SIGNAL, REPLACING_WAIT_FATAL, SIGNAL_TOTAL_LIMIT, ShutdownEnd,
    ShutdownParts, ShutdownTrigger,
};
pub use watch::file_watcher;
pub use workers::{HostTasks, ThreadJobSpawner};
