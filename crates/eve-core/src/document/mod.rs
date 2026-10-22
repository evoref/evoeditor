//! 文書 (docs/03 §6〜§7)。`Document`・`Transaction`・Undo 履歴。文書テキストを変える唯一の経路 (`Document::apply`) も
//! ここに閉じる (docs/02 §5.2。`eve_text::Text::apply` は clippy の `disallowed_methods` で、このモジュール以外から呼べない)。

// 文書テキストを変える経路はこのモジュールに閉じる (clippy.toml の `eve_text::Text::apply`)
#![allow(clippy::disallowed_methods)]

mod doc;
mod format;
mod history;
mod transaction;

pub use doc::{Applied, AppliedTx, ApplyError, DisplaySettings, Document};
pub use format::{
    EolStats, FormatChange, FormatState, LONG_LINE_THRESHOLD, LONG_LINE_VISIBLE, LoadState,
    ReadOnlyReason, ReadOnlyReasons, TextFormat, has_long_line,
};
pub use history::{HistoryStep, NodeId, RecordOutcome, UndoBudget, UndoHistory};
pub use transaction::{Origin, Transaction, TransactionMeta, UndoGroupHint};
