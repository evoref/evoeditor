//! 文書 (docs/03 §6〜§7)。`Document`・`Transaction`・Undo 履歴。文書テキストを変える唯一の経路 (`Document::apply`) も
//! ここに閉じる (docs/02 §5.2。`eve_text::Text::apply` は clippy の `disallowed_methods` で、このモジュール以外から呼べない)。
//!
//! spec: 02 §5.1, 02 §5.2, 03 §6, 03 §7, 03 §8.4 (改行コードの変換)

// 文書テキストを変える経路はこのモジュールに閉じる (clippy.toml の `eve_text::Text::apply`)
#![allow(clippy::disallowed_methods)]

mod doc;
pub(crate) mod eol;
pub(crate) mod eol_cmd;
mod format;
mod history;
mod transaction;

pub use doc::{Applied, AppliedTx, ApplyError, DisplaySettings, Document};
pub use format::{
    EolStats, FormatChange, FormatState, LONG_LINE_THRESHOLD, LoadState, ReadOnlyReason,
    ReadOnlyReasons, TextFormat, count_long_lines,
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

/// 1 件の変更あたりの逆変換の大きさの見積り (演算 3 つと挿入する文字列の確保の分。03 §12.5)。
const COARSE_PER_EDIT: usize = 128;

/// 大量のすべて置換の Undo の記録 (03 §12.5): 変更の最初の始端から最後の終端までを旧本文で置き換える 1 件の逆変換。
/// 範囲の旧本文が、変更ごとの逆変換の見積り (削除する本文 + 1 件あたり `COARSE_PER_EDIT`) より大きければ `None`
/// (メモリを増やさない。変更ごとの逆変換を使う)。`after` は `changes` を適用した後の本文。
pub(crate) fn coarse_inverse(
    changes: &eve_text::ChangeSet,
    before: &dyn eve_text::TextRead,
    after: &dyn eve_text::TextRead,
) -> Option<eve_text::ChangeSet> {
    let hunks: Vec<_> = changes.changed_ranges().collect();
    let (first, last) = (hunks.first()?, hunks.last()?);
    let old = eve_text::ByteRange::new(first.0.start, last.0.end);
    let new = eve_text::ByteRange::new(first.1.start, last.1.end);
    let deleted: usize = hunks.iter().map(|(o, _)| o.len()).sum();
    if old.len() > deleted.saturating_add(COARSE_PER_EDIT.saturating_mul(hunks.len())) {
        return None;
    }
    eve_text::ChangeSet::from_sorted_edits(
        after,
        [eve_text::Edit {
            range: new,
            insert: before.slice_to_string(old).into(),
        }],
    )
    .ok()
}
