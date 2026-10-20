//! rope ラッパ (`TextStorage`)、位置変換、`ChangeSet` など、テキスト層 (docs/02 §4.1、docs/03 §2〜§4・§6)。
//!
//! - 位置の正準表現は UTF-8 のバイトオフセット (ADR-0010)。
//! - 内容は常に妥当な UTF-8 (INV-T1)。ropey の型は公開しない (T9)。
//! - 他の `eve-*` crate に依存しない。I/O をしない。
//!
//! 実装は docs/08-roadmap.md §2.2 の「最初の縦切り」の順に入れている。現在は (1-a) テキストストレージと行の定義。
//! `ChangeSet` (1-b)、書記素・単語境界・表示幅、`Range` / `Selection` は、後のタスクで足す。

#![forbid(unsafe_code)]

mod text;
mod types;

pub use text::{Chunks, Snapshot, Text, TextBuilder, TextRead};
pub use types::{ByteOff, ByteRange, CharIdx, LineEnding, LineIdx, Pos, Utf16Off};
