//! ヘッドレス駆動ハーネス (Harness)、偽 Port (MemFs・ManualClock・ManualJobSpawner)、HeadlessFrontend (docs/02 §4.1、docs/09 §3.4)。
//!
//! 現在は偽 Port (`ManualClock` / `FixedPaths` / `MemFs` / `ManualJobSpawner`)、`Harness`、`HeadlessFrontend`、ASCII の Frame
//! がある。

#![forbid(unsafe_code)]
// eve-core の FsError は大きい型 (エラーは稀な経路)。値で返す。
#![allow(clippy::result_large_err)]

mod ascii;
mod clock;
pub mod fuzzing;
pub mod golden;
mod harness;
mod headless;
mod jobs;
mod memfs;
mod paths;
pub mod scenario;
mod virtual_screen;

pub use ascii::{AsciiOptions, render_ascii};
pub use clock::ManualClock;
/// `Harness::with_locale` に渡す言語 (eve-config に依存しない crate の試験からも使えるように)。
pub use eve_config::Locale;
pub use harness::Harness;
pub use headless::{HeadlessFrontend, HeadlessProbe};
pub use jobs::ManualJobSpawner;
pub use memfs::{BytesSource, FsOp, MEMFS_STATE_DIR, MemFs, SourceFailure, WriteStep};
pub use paths::FixedPaths;
pub use virtual_screen::{Attr, VirtualScreen, WidthConfig};
