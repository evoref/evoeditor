//! Editor ファサード、Document / View、Transaction / Undo、コマンド、検索、Job、Frame 生成、Ports の定義 (docs/02 §4.1、docs/03)。
//!
//! 現在は Ports とジョブの型 (docs/02 §4.4 / §4.5 / §9.3) だけがある。残りは docs/08-roadmap.md §2.2 の
//! 「最初の縦切り」の順に入れる。

#![forbid(unsafe_code)]
// FsError は詳細 (Conflict の Stamp など) を持つ大きな型。エラーは稀な経路なので、Box で包まず値で返す。
#![allow(clippy::result_large_err)]

mod cancel;
mod fs;
mod fspath;
mod jobs;
mod paths;
mod time;

pub use cancel::CancelToken;
pub use fs::*;
pub use fspath::FsPath;
pub use jobs::{
    JobError, JobKind, JobPorts, JobRequest, JobResult, JobSpawner, MetaPurpose, Ports, run_job,
};
pub use paths::Paths;
pub use time::{Clock, Millis, Tick};
