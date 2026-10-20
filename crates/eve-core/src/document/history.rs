//! Undo 履歴 (docs/03-core-design.md §7)。線形の 2 本のスタック (undo / redo)。
//!
//! 位置は `NodeId` (文書ごとの単調増加の通番) で表し、スタック上の深さでは表さない (§7.1)。
//! 結合規則 R1〜R9 (§7.3) は `UndoHistory::record` が上から順に評価する。
//! `FormatChange` (Reload・改行コード変換が `TextFormat` を一緒に戻す) は、その最初の利用者の版で足す。

use std::collections::VecDeque;

use eve_text::{ByteRange, ChangeSet, Op, Selection};

use crate::time::{Millis, Tick};

use super::transaction::{Origin, Transaction, UndoGroupHint};

/// 履歴上の位置。文書ごとの通番で、破棄しても再利用しない。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub u64);

/// 結合の時間窓 (`editor.undo.group_timeout_ms`、07 §5.1。v0.1 は定数)。
pub const GROUP_TIMEOUT: Millis = Millis(1000);
/// 連続入力の 1 ノードあたりの挿入長の上限 (R7)。
pub const MAX_COALESCED_INSERT: usize = 4 * 1024;
/// ノードごとの固定オーバーヘッド (§7.5)。
const NODE_OVERHEAD: usize = 128;

/// 文書ごとの Undo の予算 (§7.5)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UndoBudget {
    pub max_bytes: usize,
    pub max_nodes: usize,
}

impl Default for UndoBudget {
    fn default() -> UndoBudget {
        UndoBudget {
            max_bytes: 64 * 1024 * 1024,
            max_nodes: 10_000,
        }
    }
}

#[derive(Clone, Debug)]
struct UndoNode {
    id: NodeId,
    redo: ChangeSet,
    undo: ChangeSet,
    selection_before: Option<Selection>,
    selection_after: Option<Selection>,
    last_edit_at: Tick,
    group_sealed: bool,
    bytes: usize,
}

/// `record` の結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordOutcome {
    /// 直前のノードに結合した。
    pub merged: bool,
    /// 予算の超過で、古いノードを破棄した (通知は呼び出し側が 1 回出す)。
    pub pruned: bool,
}

/// Undo / Redo で適用する変更。
#[derive(Clone, Debug)]
pub struct HistoryStep {
    pub changes: ChangeSet,
    /// 発行 View に復元する Selection。Undo は `selection_before`、Redo は `selection_after`。
    pub selection: Option<Selection>,
}

#[derive(Debug)]
pub struct UndoHistory {
    undo: VecDeque<UndoNode>,
    redo: Vec<UndoNode>,
    /// `undo` が空のときの現在位置。
    base: NodeId,
    next_id: u64,
    saved: Option<NodeId>,
    budget: UndoBudget,
}

impl Default for UndoHistory {
    fn default() -> UndoHistory {
        UndoHistory::new(UndoBudget::default())
    }
}

/// 編集の種類 (R5)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditKind {
    /// 純挿入、または置換 (選択範囲への入力)。
    Insert,
    Delete,
    /// 挿入と削除が別々の箇所に混ざる。
    Mixed,
}

/// ChangeSet の変更のまとまり (適用前の範囲, 適用後の範囲) の一覧。
fn hunks(cs: &ChangeSet) -> Vec<(ByteRange, ByteRange)> {
    cs.changed_ranges().collect()
}

fn kind_of(hunks: &[(ByteRange, ByteRange)]) -> EditKind {
    let inserts = hunks.iter().filter(|(_, after)| !after.is_empty()).count();
    if inserts == hunks.len() {
        EditKind::Insert
    } else if inserts == 0 {
        EditKind::Delete
    } else {
        EditKind::Mixed
    }
}

fn inserted_texts(cs: &ChangeSet) -> impl Iterator<Item = &str> {
    cs.ops().iter().filter_map(|op| match op {
        Op::Insert(s) if !s.is_empty() => Some(s.as_str()),
        _ => None,
    })
}

fn inserted_len(cs: &ChangeSet) -> usize {
    inserted_texts(cs).map(str::len).sum()
}

fn has_newline(cs: &ChangeSet) -> bool {
    inserted_texts(cs).any(|s| s.contains(['\n', '\r']))
}

/// 挿入が改行 1 つだけか (R6 (a)): すべての挿入文字列が LF / CRLF / CR のどれか 1 つ。
fn only_single_newlines(cs: &ChangeSet) -> bool {
    let mut any = false;
    for s in inserted_texts(cs) {
        if !matches!(s, "\n" | "\r\n" | "\r") {
            return false;
        }
        any = true;
    }
    any
}

fn cost_of(redo: &ChangeSet, undo: &ChangeSet, sels: [&Option<Selection>; 2]) -> usize {
    let sel_bytes: usize = sels
        .iter()
        .filter_map(|s| s.as_ref())
        .map(|s| s.len() * std::mem::size_of::<eve_text::Range>())
        .sum();
    inserted_len(redo) + inserted_len(undo) + NODE_OVERHEAD + sel_bytes
}

impl UndoHistory {
    pub fn new(budget: UndoBudget) -> UndoHistory {
        UndoHistory {
            undo: VecDeque::new(),
            redo: Vec::new(),
            base: NodeId(0),
            next_id: 1,
            saved: Some(NodeId(0)),
            budget,
        }
    }

    /// 現在位置。
    pub fn current(&self) -> NodeId {
        self.undo.back().map_or(self.base, |n| n.id)
    }

    pub fn saved(&self) -> Option<NodeId> {
        self.saved
    }

    /// 保存点と現在位置が違う (`saved == None` は dirty)。
    pub fn is_dirty(&self) -> bool {
        self.saved != Some(self.current())
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    /// 履歴の合計バイト数 (会計。プロセス全体の上限の算出に使う)。
    pub fn bytes(&self) -> usize {
        self.undo
            .iter()
            .chain(self.redo.iter())
            .map(|n| n.bytes)
            .sum()
    }

    /// 現在位置を保存点にする (読込完了・新規文書・開き直し・保存完了で保存に使った位置が現在のとき)。
    pub fn mark_saved(&mut self) {
        self.saved = Some(self.current());
    }

    /// 保存に使った位置 `at` を保存点にする。`at` が `base`・`undo`・`redo` のどこにも無ければ `None` (INV-H3)。
    pub fn mark_saved_at(&mut self, at: NodeId) {
        self.saved = self.exists(at).then_some(at);
    }

    fn exists(&self, id: NodeId) -> bool {
        id == self.base
            || self.undo.iter().any(|n| n.id == id)
            || self.redo.iter().any(|n| n.id == id)
    }

    /// 履歴を空にして新しい根を作る (読込完了・開き直し)。保存点は根。
    pub fn reset(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.base = NodeId(self.next_id);
        self.next_id += 1;
        self.saved = Some(self.base);
    }

    /// 直前のノードを封印する (カーソル移動・保存の開始・Undo / Redo の後)。
    pub fn seal(&mut self) {
        if let Some(n) = self.undo.back_mut() {
            n.group_sealed = true;
        }
    }

    fn fresh_id(&mut self) -> NodeId {
        let id = NodeId(self.next_id);
        self.next_id += 1;
        id
    }

    /// 適用した Transaction を記録する (§6.5 手順 6)。`inverse` は `tx.changes.invert(適用前)`。
    /// `tx.is_undo` の Transaction は渡さない (Undo / Redo は履歴上の移動で、新ノードを作らない)。
    pub fn record(&mut self, tx: &Transaction, inverse: ChangeSet) -> RecordOutcome {
        debug_assert!(!tx.is_undo, "Undo / Redo は record しない");
        // 新しい編集は redo を破棄する。保存点が redo のノードを指していれば戻れない
        if !self.redo.is_empty() {
            if self
                .saved
                .is_some_and(|s| self.redo.iter().any(|n| n.id == s))
            {
                self.saved = None;
            }
            self.redo.clear();
        }
        let merge = self
            .undo
            .back()
            .is_some_and(|prev| self.should_merge(prev, tx));
        let (merged, seal_after) = self.decide(tx, merge);
        if merged {
            self.merge_into_last(tx, inverse);
        } else {
            self.push_node(tx, inverse);
        }
        if seal_after {
            self.seal();
        }
        let pruned = self.prune();
        RecordOutcome { merged, pruned }
    }

    /// 結合する (`true`) か新ノード (`false`) かと、記録の後で封印するか (§7.3)。
    fn decide(&mut self, tx: &Transaction, merge: bool) -> (bool, bool) {
        let newline = has_newline(&tx.changes);
        let prev_open = self.undo.back().is_some_and(|p| !p.group_sealed);
        // R1
        if tx.undo_group == UndoGroupHint::Merge && prev_open {
            return (true, newline);
        }
        // R2
        if tx.origin != Origin::User || tx.undo_group == UndoGroupHint::Break {
            self.seal();
            return (false, true);
        }
        // R3〜R9 は should_merge が評価した結果 (merge)。改行を含む挿入は記録の後で封印する (R6)
        (merge, newline)
    }

    /// R3〜R9: 直前のノードに結合するか。R1 / R2 は呼び出し側で先に評価する。
    fn should_merge(&self, prev: &UndoNode, tx: &Transaction) -> bool {
        // R3
        if prev.group_sealed {
            return false;
        }
        // R2 の条件は decide が先に見る。ここでは User 以外・Break・Merge を扱わない
        if tx.origin != Origin::User || tx.undo_group != UndoGroupHint::Auto {
            return false;
        }
        // R4
        if tx.timestamp.saturating_sub(prev.last_edit_at) > GROUP_TIMEOUT {
            return false;
        }
        let new_hunks = hunks(&tx.changes);
        let prev_hunks = hunks(&prev.redo);
        let new_kind = kind_of(&new_hunks);
        // R5
        if new_kind == EditKind::Mixed || new_kind != kind_of(&prev_hunks) {
            return false;
        }
        // 各 Range が対応する (同じ数の変更のまとまり)
        if new_hunks.len() != prev_hunks.len() {
            return false;
        }
        match new_kind {
            EditKind::Insert => {
                let newline = has_newline(&tx.changes);
                // R6 (b): 改行と他の文字を含む → 結合しない
                if newline && !only_single_newlines(&tx.changes) {
                    return false;
                }
                // R7 の位置の条件: 新しい挿入位置 == 直前の挿入の末尾 (各 Range)
                let adjacent = new_hunks
                    .iter()
                    .zip(&prev_hunks)
                    .all(|((nb, _), (_, pa))| nb.start == pa.end);
                if !adjacent {
                    return false;
                }
                // R7 の長さの条件 (R6 (a) の改行 1 つには課さない)
                newline
                    || inserted_len(&prev.redo) + inserted_len(&tx.changes) <= MAX_COALESCED_INSERT
            }
            // R8: Backspace の連打 (新しい削除の末尾 == 直前の削除の先頭) か Delete の連打 (先頭 == 直前の削除の先頭)
            EditKind::Delete => new_hunks
                .iter()
                .zip(&prev_hunks)
                .all(|((nb, _), (_, pa))| nb.end == pa.start || nb.start == pa.start),
            EditKind::Mixed => false,
        }
    }

    fn merge_into_last(&mut self, tx: &Transaction, inverse: ChangeSet) {
        let new_id = self.fresh_id();
        let Some(prev) = self.undo.back_mut() else {
            return;
        };
        // 合成できないのは呼び出しの誤り (長さの不一致)。履歴を壊さないよう、結合せず新ノードにする
        let (Ok(redo), Ok(undo)) = (prev.redo.compose(&tx.changes), inverse.compose(&prev.undo))
        else {
            debug_assert!(false, "結合の合成に失敗した");
            self.push_node(tx, inverse);
            return;
        };
        let old_id = prev.id;
        prev.redo = redo;
        prev.undo = undo;
        prev.selection_after = tx.selection_after.clone();
        prev.last_edit_at = tx.timestamp;
        prev.id = new_id;
        prev.bytes = cost_of(
            &prev.redo,
            &prev.undo,
            [&prev.selection_before, &prev.selection_after],
        );
        // 内容が変わったので以前の位置とは別の位置になる
        if self.saved == Some(old_id) {
            self.saved = None;
        }
    }

    fn push_node(&mut self, tx: &Transaction, inverse: ChangeSet) {
        let id = self.fresh_id();
        let bytes = cost_of(
            &tx.changes,
            &inverse,
            [&tx.selection_before, &tx.selection_after],
        );
        self.undo.push_back(UndoNode {
            id,
            redo: tx.changes.clone(),
            undo: inverse,
            selection_before: tx.selection_before.clone(),
            selection_after: tx.selection_after.clone(),
            last_edit_at: tx.timestamp,
            group_sealed: false,
            bytes,
        });
    }

    /// 予算を超えたら古い側から破棄する (最低 1 ノードは残す)。破棄したら `true`。
    fn prune(&mut self) -> bool {
        let mut pruned = false;
        let mut total = self.bytes();
        while self.undo.len() > 1
            && (self.undo.len() > self.budget.max_nodes || total > self.budget.max_bytes)
        {
            if let Some(n) = self.undo.pop_front() {
                total = total.saturating_sub(n.bytes);
                self.base = n.id;
                pruned = true;
            }
        }
        if pruned && self.saved.is_some_and(|s| s < self.base) {
            self.saved = None;
        }
        pruned
    }

    /// Undo: `undo` の末尾のノードを `redo` へ移し、適用する変更を返す。呼び出し側が適用する。
    /// 適用後に封印する (直後の入力を Undo 前のノードに結合しない)。
    pub fn undo_step(&mut self) -> Option<HistoryStep> {
        let node = self.undo.pop_back()?;
        let step = HistoryStep {
            changes: node.undo.clone(),
            selection: node.selection_before.clone(),
        };
        self.redo.push(node);
        self.seal();
        Some(step)
    }

    /// Redo: `redo` の末尾のノードを `undo` へ戻し、適用する変更を返す。適用後に封印する。
    pub fn redo_step(&mut self) -> Option<HistoryStep> {
        let node = self.redo.pop()?;
        let step = HistoryStep {
            changes: node.redo.clone(),
            selection: node.selection_after.clone(),
        };
        self.undo.push_back(node);
        self.seal();
        Some(step)
    }

    #[cfg(test)]
    fn node_ids(&self) -> Vec<u64> {
        self.undo.iter().map(|n| n.id.0).collect()
    }
}

#[cfg(test)]
mod tests {
    use eve_text::{Edit, Text, TextRead};

    use super::*;

    /// 履歴と文書を持つ試験用の文書。
    struct Doc {
        text: Text,
        h: UndoHistory,
        now: u64,
    }

    impl Doc {
        fn new(initial: &str) -> Doc {
            Doc {
                text: Text::from(initial),
                h: UndoHistory::default(),
                now: 0,
            }
        }

        fn s(&self) -> String {
            self.text
                .slice_to_string(ByteRange::all(self.text.len_bytes()))
        }

        fn edit(
            &mut self,
            start: usize,
            end: usize,
            ins: &str,
            origin: Origin,
            hint: UndoGroupHint,
            dt: u64,
        ) -> RecordOutcome {
            self.now += dt;
            let snap = self.text.snapshot();
            let cs = ChangeSet::from_sorted_edits(
                &snap,
                [Edit {
                    range: ByteRange::from(start..end),
                    insert: ins.into(),
                }],
            )
            .unwrap();
            let inverse = cs.invert(&snap);
            self.text.apply(&cs).unwrap();
            let tx = Transaction::new(cs, origin, Tick(self.now)).with_undo_group(hint);
            self.h.record(&tx, inverse)
        }

        fn type_at(&mut self, pos: usize, s: &str, dt: u64) -> RecordOutcome {
            self.edit(pos, pos, s, Origin::User, UndoGroupHint::Auto, dt)
        }

        fn delete(&mut self, start: usize, end: usize, dt: u64) -> RecordOutcome {
            self.edit(start, end, "", Origin::User, UndoGroupHint::Auto, dt)
        }

        fn undo(&mut self) -> bool {
            let Some(step) = self.h.undo_step() else {
                return false;
            };
            self.text.apply(&step.changes).unwrap();
            true
        }

        fn redo(&mut self) -> bool {
            let Some(step) = self.h.redo_step() else {
                return false;
            };
            self.text.apply(&step.changes).unwrap();
            true
        }
    }

    #[test]
    fn 連続した文字入力は_1_ノードになる() {
        let mut d = Doc::new("");
        assert!(!d.type_at(0, "a", 0).merged, "最初は新ノード");
        assert!(d.type_at(1, "b", 100).merged);
        assert!(d.type_at(2, "c", 100).merged);
        assert_eq!(d.h.undo_len(), 1);
        assert!(d.undo());
        assert_eq!(d.s(), "", "1 回の Undo で abc が戻る");
        assert!(d.redo());
        assert_eq!(d.s(), "abc");
    }

    #[test]
    fn 時間窓の境界は_1000ms_ちょうどまで結合する() {
        let mut d = Doc::new("");
        d.type_at(0, "a", 0);
        assert!(d.type_at(1, "b", 1000).merged, "ちょうど 1000 ms は結合");
        assert!(!d.type_at(2, "c", 1001).merged, "超えると新ノード");
        assert_eq!(d.h.undo_len(), 2);
    }

    #[test]
    fn 挿入と削除は別ノード() {
        let mut d = Doc::new("");
        d.type_at(0, "ab", 0);
        assert!(!d.delete(1, 2, 10).merged, "R5: 挿入 → 削除");
        assert!(!d.type_at(1, "x", 10).merged, "R5: 削除 → 挿入");
    }

    #[test]
    fn 離れた位置の入力は結合しない() {
        let mut d = Doc::new("abc");
        d.type_at(0, "x", 0);
        assert!(!d.type_at(3, "y", 10).merged, "R7: 位置が連続しない");
    }

    #[test]
    fn バックスペースとデリートの連打は結合する() {
        let mut d = Doc::new("abcdef");
        d.delete(5, 6, 0);
        assert!(d.delete(4, 5, 10).merged, "Backspace の連打");
        assert!(d.delete(3, 4, 10).merged);
        assert_eq!(d.s(), "abc");
        let mut d = Doc::new("abcdef");
        d.delete(1, 2, 0);
        assert!(d.delete(1, 2, 10).merged, "Delete の連打");
        assert!(!d.delete(3, 4, 10).merged, "どちらにも隣接しない削除");
    }

    #[test]
    fn 改行の入力は結合して封印する() {
        // `a` `b` Enter `c` → `ab⏎` と `c`
        let mut d = Doc::new("");
        d.type_at(0, "a", 0);
        d.type_at(1, "b", 10);
        assert!(d.type_at(2, "\n", 10).merged, "R6 (a): 位置が合えば結合");
        assert!(!d.type_at(3, "c", 10).merged, "改行の後は封印されている");
        assert_eq!(d.h.undo_len(), 2);
        d.undo();
        assert_eq!(d.s(), "ab\n");
    }

    #[test]
    fn 改行と他の文字を含む挿入は単独の新ノード() {
        let mut d = Doc::new("");
        d.type_at(0, "a", 0);
        assert!(!d.type_at(1, "\n  ", 10).merged, "R6 (b)");
        assert!(!d.type_at(4, "x", 10).merged, "続く入力もさらに別ノード");
        assert_eq!(d.h.undo_len(), 3);
    }

    #[test]
    fn user_以外の_origin_は結合せず前後を封印する() {
        let mut d = Doc::new("");
        d.type_at(0, "a", 0);
        assert!(
            !d.edit(1, 1, "P", Origin::Paste, UndoGroupHint::Auto, 10)
                .merged,
            "R2"
        );
        assert!(
            !d.type_at(2, "b", 10).merged,
            "貼り付けの後は封印されている"
        );
        assert_eq!(d.h.undo_len(), 3);
    }

    #[test]
    fn break_は新ノード_merge_は結合() {
        let mut d = Doc::new("");
        d.type_at(0, "a", 0);
        assert!(
            !d.edit(1, 1, "b", Origin::User, UndoGroupHint::Break, 10)
                .merged
        );
        assert!(
            !d.type_at(2, "c", 10).merged,
            "Break のノードは封印されている"
        );
        // Merge は封印されていなければ必ず結合 (種類や位置が違っても)
        let mut d = Doc::new("xyz");
        d.type_at(0, "a", 0);
        assert!(
            d.edit(
                3,
                4,
                "",
                Origin::Command(eve_view::CommandId::new("edit.test")),
                UndoGroupHint::Merge,
                5000
            )
            .merged
        );
        assert_eq!(d.h.undo_len(), 1);
        // 封印済みなら Merge でも当たらず R2 以降: Paste は新ノード
        d.h.seal();
        assert!(
            !d.edit(0, 0, "q", Origin::User, UndoGroupHint::Merge, 10)
                .merged
        );
    }

    #[test]
    fn 長い連続入力は_4kib_で区切る() {
        let mut d = Doc::new("");
        let chunk = "x".repeat(1024);
        let mut pos = 0;
        let mut merged = 0;
        for _ in 0..5 {
            if d.type_at(pos, &chunk, 10).merged {
                merged += 1;
            }
            pos += chunk.len();
        }
        assert_eq!(merged, 3, "4 KiB までは結合 (1 + 3 回)");
        assert_eq!(d.h.undo_len(), 2);
    }

    #[test]
    fn undo_と_redo_は封印して_redo_を新しい編集で捨てる() {
        let mut d = Doc::new("");
        d.type_at(0, "a", 0);
        d.type_at(1, "b", 10);
        d.undo();
        assert_eq!(d.s(), "");
        d.redo();
        // Redo の直後の入力は結合しない
        assert!(!d.type_at(2, "c", 10).merged);
        d.undo();
        d.undo();
        assert_eq!(d.s(), "");
        assert!(d.h.can_redo());
        d.type_at(0, "z", 10);
        assert!(!d.h.can_redo(), "新しい編集で redo を捨てる");
    }

    #[test]
    fn 保存点は履歴上の位置で判定する() {
        let mut d = Doc::new("");
        assert!(!d.h.is_dirty(), "根が保存点");
        d.type_at(0, "a", 0);
        assert!(d.h.is_dirty());
        d.h.mark_saved();
        assert!(!d.h.is_dirty());
        d.type_at(1, "b", 5000);
        assert!(d.h.is_dirty());
        d.undo();
        assert!(!d.h.is_dirty(), "Undo で保存時点に戻れば未変更");
        d.redo();
        assert!(d.h.is_dirty());
    }

    #[test]
    fn 保存点を結合で書き換えると戻れない() {
        let mut d = Doc::new("");
        d.type_at(0, "a", 0);
        d.h.mark_saved();
        // 保存の開始で封印するので、保存後の入力は新ノードになる
        d.h.seal();
        assert!(!d.type_at(1, "b", 10).merged);
        d.undo();
        assert!(!d.h.is_dirty());
        // 封印せずに結合すると、保存点だったノードの内容が変わるので保存点は失われる
        let mut d = Doc::new("");
        d.type_at(0, "a", 0);
        d.h.mark_saved();
        assert!(d.type_at(1, "b", 10).merged);
        assert_eq!(d.h.saved(), None);
        assert!(d.h.is_dirty());
    }

    #[test]
    fn redo_の破棄で保存点が消える() {
        let mut d = Doc::new("");
        d.type_at(0, "a", 0);
        d.type_at(1, "b", 5000);
        d.h.mark_saved(); // "ab" の位置
        d.undo();
        assert!(d.h.is_dirty());
        d.type_at(1, "c", 10); // redo ("b") を捨てる → 保存点は戻れない
        assert_eq!(d.h.saved(), None);
    }

    #[test]
    fn 保存完了時に位置が無ければ_none() {
        let mut d = Doc::new("");
        d.type_at(0, "a", 0);
        let at = d.h.current();
        d.undo();
        d.type_at(0, "z", 5000); // at のノードは redo ごと消える
        d.h.mark_saved_at(at);
        assert_eq!(d.h.saved(), None, "INV-H3");
        d.h.mark_saved_at(d.h.current());
        assert!(!d.h.is_dirty());
    }

    #[test]
    fn 予算を超えたら古い側から破棄し_保存点を補正する() {
        let mut d = Doc::new("");
        d.h = UndoHistory::new(UndoBudget {
            max_bytes: usize::MAX,
            max_nodes: 3,
        });
        d.h.mark_saved(); // 根
        for i in 0..5u64 {
            let out = d.edit(
                i as usize,
                i as usize,
                "x",
                Origin::Paste,
                UndoGroupHint::Auto,
                10,
            );
            assert_eq!(out.pruned, i >= 3);
        }
        assert_eq!(d.h.undo_len(), 3);
        assert_eq!(d.h.saved(), None, "根は破棄されて戻れない");
        assert!(d.h.is_dirty());
        // 破棄しても残りの位置の通番は変わらない
        let ids = d.h.node_ids();
        assert!(ids.windows(2).all(|w| w[0] < w[1]));
        // 最低 1 ノードは残る
        let mut tiny = Doc::new("");
        tiny.h = UndoHistory::new(UndoBudget {
            max_bytes: 0,
            max_nodes: 10,
        });
        tiny.edit(0, 0, "x", Origin::Paste, UndoGroupHint::Auto, 0);
        assert_eq!(tiny.h.undo_len(), 1, "直前の 1 操作は予算より優先");
    }

    #[test]
    fn 底が保存点のとき_すべて_undo_すれば保存時点() {
        let mut d = Doc::new("");
        d.h = UndoHistory::new(UndoBudget {
            max_bytes: usize::MAX,
            max_nodes: 2,
        });
        d.edit(0, 0, "a", Origin::Paste, UndoGroupHint::Auto, 0);
        d.h.mark_saved(); // node 1 が保存点
        d.edit(1, 1, "b", Origin::Paste, UndoGroupHint::Auto, 0);
        d.edit(2, 2, "c", Origin::Paste, UndoGroupHint::Auto, 0); // node 1 を破棄 → base = node 1
        assert_eq!(d.h.undo_len(), 2);
        assert_eq!(d.h.saved(), Some(NodeId(1)), "saved == base は有効");
        d.undo();
        d.undo();
        assert!(!d.h.is_dirty());
        assert_eq!(d.s(), "a");
    }

    #[test]
    fn 複数の範囲は全部で条件を満たすときだけ結合する() {
        let mut d = Doc::new("ab\ncd");
        // 2 か所に同時に入力 (旧座標 0 と 3)
        let snap = d.text.snapshot();
        let mk = |snap: &eve_text::Snapshot, a: usize, b: usize, s: &str| {
            ChangeSet::from_sorted_edits(
                snap,
                [
                    Edit {
                        range: ByteRange::from(a..a),
                        insert: s.into(),
                    },
                    Edit {
                        range: ByteRange::from(b..b),
                        insert: s.into(),
                    },
                ],
            )
            .unwrap()
        };
        let cs = mk(&snap, 0, 3, "X");
        let inv = cs.invert(&snap);
        d.text.apply(&cs).unwrap();
        d.now = 0;
        d.h.record(&Transaction::new(cs, Origin::User, Tick(0)), inv);
        // 続けて各カーソルの後ろ (新座標 1 と 5) に入力 → 結合
        let snap = d.text.snapshot();
        let cs = mk(&snap, 1, 5, "Y");
        let inv = cs.invert(&snap);
        d.text.apply(&cs).unwrap();
        assert!(
            d.h.record(&Transaction::new(cs, Origin::User, Tick(10)), inv)
                .merged
        );
        // 片方だけ連続している → 結合しない
        let snap = d.text.snapshot();
        let cs = ChangeSet::from_sorted_edits(
            &snap,
            [
                Edit {
                    range: ByteRange::from(0..0),
                    insert: "Q".into(),
                },
                Edit {
                    range: ByteRange::from(7..7),
                    insert: "Q".into(),
                },
            ],
        )
        .unwrap();
        let inv = cs.invert(&snap);
        d.text.apply(&cs).unwrap();
        assert!(
            !d.h.record(&Transaction::new(cs, Origin::User, Tick(20)), inv)
                .merged
        );
        assert_eq!(
            d.s(),
            "QXYab
XYQcd"
        );
    }
}
