//! ジョブの型と `JobSpawner` Port (docs/02-architecture.md §4.4 / §9.3)。
//!
//! ジョブはワーカーで動き、結果を `JobResult` で返す。結果は `base` (投入時の Revision) を持ち、受け取る側が
//! 古い結果を捨てる (ADR-0020)。変種は `JobKind` を参照。
//!
//! spec: 02 §4.4, 02 §9.3

pub(crate) mod cancel;

mod kind;
mod ports;
mod result;
mod run;

pub use kind::{JobKind, JobRequest, MetaPurpose, SaveStrategy};
pub use ports::{JobPorts, JobSpawner, Ports};
pub use result::{ConfigFile, ConfirmAsk, JobError, JobResult, OpenNotes};
pub use run::run_job;
