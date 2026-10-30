//! 文書 (docs/03 §6〜§7)。`Document`・`Transaction`・Undo 履歴。文書テキストを変える唯一の経路 (`Document::apply`) も
//! ここに閉じる (docs/02 §5.2。`eve_text::Text::apply` は clippy の `disallowed_methods` で、このモジュール以外から呼べない)。

// 文書テキストを変える経路はこのモジュールに閉じる (clippy.toml の `eve_text::Text::apply`)
#![allow(clippy::disallowed_methods)]

mod doc;
mod format;
mod history;
mod transaction;

pub use doc::{Applied, AppliedTx, ApplyError, DisplaySettings, Document, ExternalSeen};
pub use format::{
    EolStats, FormatChange, FormatState, LONG_LINE_THRESHOLD, LONG_LINE_VISIBLE, LoadState,
    ReadOnlyReason, ReadOnlyReasons, TextFormat, has_long_line,
};
pub use history::{HistoryStep, NodeId, RecordOutcome, UndoBudget, UndoHistory};
pub use transaction::{Origin, Transaction, TransactionMeta, UndoGroupHint};

/// プロセス全体の Undo 履歴の上限 (NFR-203、03 §7.5)。`docs` の履歴の会計値の合計が `limit` を超えている間、
/// 履歴が最も大きい文書から古い順に 1 ノードずつ破棄する。1 ノードしか残っていない文書は削らない
/// (直前の 1 操作は上限より優先)。削った文書のキーを返す (重複なし、初めて削った順)。dirty 表示は変わらない (03 §7.1)。
pub fn trim_to_total_budget<K: Copy + PartialEq>(
    docs: &mut [(K, &mut Document)],
    limit: usize,
) -> Vec<K> {
    let mut total: usize = docs.iter().map(|(_, d)| d.history().bytes()).sum();
    let mut trimmed: Vec<K> = Vec::new();
    while total > limit {
        // 削れる文書 (2 ノード以上) のうち、履歴が最も大きいもの (同点は先のもの)
        let Some(i) = (0..docs.len())
            .filter(|&i| docs[i].1.history().undo_len() > 1)
            .max_by_key(|&i| (docs[i].1.history().bytes(), std::cmp::Reverse(i)))
        else {
            break;
        };
        let Some(freed) = docs[i].1.trim_history_oldest() else {
            break;
        };
        total = total.saturating_sub(freed);
        if !trimmed.contains(&docs[i].0) {
            trimmed.push(docs[i].0);
        }
    }
    trimmed
}
