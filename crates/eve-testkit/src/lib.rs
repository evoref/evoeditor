//! ヘッドレス駆動ハーネス (Harness)、偽 Port (MemFs・ManualClock・ManualJobSpawner)、HeadlessFrontend (docs/02 §4.1、docs/09 §3.4)。
//!
//! 現在は偽 Port (`ManualClock` / `FixedPaths` / `MemFs` / `ManualJobSpawner`)、`Harness`、`HeadlessFrontend`、ASCII の Frame
//! がある。

#![forbid(unsafe_code)]
#![deny(missing_docs)]
// eve-core の FsError は大きい型 (エラーは稀な経路)。値で返す。
#![allow(clippy::result_large_err)]

mod ascii;
mod clock;
pub mod fs_contract;
pub mod fuzzing;
pub mod golden;
mod harness;
mod headless;
pub mod helpers;
mod jobs;
mod memfs;
mod paths;
pub mod scenario;
mod stdin;
mod virtual_screen;

pub use ascii::{AsciiOptions, render_ascii};
pub use clock::ManualClock;
/// `Harness::with_locale` に渡す言語 (eve-config に依存しない crate の試験からも使えるように)。
pub use eve_config::Locale;

/// ユーザーの言語ファイルの本文 `src` (言語 `language` のもの。`ja`・`fr` など) を検査して、補う言語の同梱の表に重ねた表と、
/// その診断を返す (bin の起動時の読込と同じ規則。07 §17.4)。`Harness::with_catalog` に渡す試験用。
///
/// # Panics
///
/// `language` が言語の名前の形 (英字 2〜3 字) でないとき。
pub fn overlay_catalog(
    language: &str,
    src: &str,
) -> (
    Locale,
    std::sync::Arc<eve_config::Catalog>,
    Vec<eve_config::Diagnostic>,
) {
    let locale = Locale::from_tag(language).expect("言語の名前");
    let base = eve_config::Catalog::builtin(locale);
    let file = eve_config::load_locale_file(
        locale.code(),
        src.as_bytes(),
        base,
        eve_config::LOCALE_MAX_DIAGNOSTICS,
    );
    let catalog = std::sync::Arc::new(base.overlay(&file));
    (locale, catalog, file.into_diagnostics())
}
pub use harness::{Harness, HarnessBuilder};
pub use headless::{HeadlessFrontend, HeadlessProbe};
pub use jobs::ManualJobSpawner;
pub use memfs::{BytesSource, FsOp, MEMFS_STATE_DIR, MemFs, SourceFailure, WriteStep};
pub use paths::FixedPaths;
pub use stdin::{FakeStdin, StdinStep};
pub use virtual_screen::{Attr, VirtualScreen, WidthConfig};
