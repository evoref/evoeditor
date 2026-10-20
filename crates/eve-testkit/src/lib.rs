//! ヘッドレス駆動ハーネス (Harness)、偽 Port (MemFs・ManualClock・ManualJobSpawner)、HeadlessFrontend (docs/02 §4.1、docs/09 §3.4)。
//!
//! 現在は偽 Port (`ManualClock` / `FixedPaths` / `MemFs` / `ManualJobSpawner`) だけがある。Harness は `Editor` ができる
//! C2 で足す (docs/08-roadmap.md §2.2)。`ThreadGuard` の検査モードは eve-core 側の印と一緒に足す。

#![forbid(unsafe_code)]
// eve-core の FsError は大きい型 (エラーは稀な経路)。値で返す。
#![allow(clippy::result_large_err)]

mod clock;
mod jobs;
mod memfs;
mod paths;

pub use clock::ManualClock;
pub use jobs::ManualJobSpawner;
pub use memfs::{BytesSource, FsOp, MemFs, SourceFailure, WriteStep};
pub use paths::FixedPaths;
