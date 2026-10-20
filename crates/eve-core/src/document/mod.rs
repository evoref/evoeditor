//! 文書 (docs/03 §6〜§7)。`Transaction` と Undo 履歴。文書テキストを変える唯一の経路 (`Document::apply`) もここに閉じる
//! (docs/02 §5.2。`eve_text::Text::apply` は clippy の `disallowed_methods` で、このモジュール以外から呼べない)。

// 文書テキストを変える経路はこのモジュールに閉じる (clippy.toml の `eve_text::Text::apply`)
#![allow(clippy::disallowed_methods)]

mod history;
mod transaction;

pub use history::{HistoryStep, NodeId, RecordOutcome, UndoBudget, UndoHistory};
pub use transaction::{Origin, Transaction, TransactionMeta, UndoGroupHint};
