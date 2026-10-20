//! rope ラッパ (`TextStorage`)、位置変換、`ChangeSet` など、テキスト層 (docs/02 §4.1、docs/03 §2〜§4・§6)。
//!
//! - 位置の正準表現は UTF-8 のバイトオフセット (ADR-0010)。
//! - 内容は常に妥当な UTF-8 (INV-T1)。ropey の型は公開しない (T9)。
//! - 他の `eve-*` crate に依存しない。I/O をしない。
//!
//! 実装は docs/08-roadmap.md §2.2 の「最初の縦切り」の順に入れている。現在は (1-a) テキストストレージと行の定義、(1-b) `ChangeSet`、(1-c) `Range` / `Selection` (書記素へのスナップを除く)。
//! 書記素・単語境界・表示幅と、`Selection` のスナップ (N1) は、後のタスクで足す。

#![forbid(unsafe_code)]

mod changeset;
mod grapheme;
mod selection;
mod text;
mod types;
mod width;
mod word;

pub use changeset::{
    Bias, ChangeSet, ChangeSetError, ComposeError, Edit, EditError, InsertText, Op, OverlapKind,
    RangeMap,
};
pub use grapheme::{
    CONTEXT as GRAPHEME_CONTEXT, is_grapheme_boundary, next_grapheme_boundary,
    prev_grapheme_boundary, snap_to_grapheme, snap_to_nearest_grapheme,
};
pub use selection::{MapPolicy, Range, Selection};
pub use text::{Chunks, Snapshot, Text, TextBuilder, TextRead};
pub use types::{ByteOff, ByteRange, CharIdx, LineEnding, LineIdx, Pos, Utf16Off};
pub use width::{
    ControlStyle, EmojiWidth, MAX_GRAPHEME_WIDTH, WidthConfig, display_col_to_byte, grapheme_width,
    invisible_marks, invisible_marks_text, is_width_risky, str_display_width,
};
pub use word::{ScriptClass, classify_grapheme, next_word_stop, prev_word_stop};
