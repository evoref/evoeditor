//! ジャンプ履歴 (FR-310、docs/03 §5.9 の JMP-1〜JMP-7、07 §11.6 の「編集の拡張」、01 FR-310 AC-1)。
//!
//! - 持ち方: ペイン (`Pane.jumps`) ごとに項目 (文書, 位置) の並びと今の位置 (JMP-2)。分割は写し、閉じたペインは捨てる。
//! - 記録: ジャンプにする移動の前の (文書, 主の Range の `head`) を、移動で位置か文書が変わったときに足す (JMP-1)。呼び側は
//!   移動の前に `jump_mark` で印を取り、移動の後に `commit_jump` で確かめる。文書の切り替えは `show_document_in` が記録する。
//! - `cursor.jump_back` / `cursor.jump_forward`: 前 / 次の項目へ移る (JMP-3)。文書を変えず、Undo に残らない (JMP-7)。
//! - 位置の写像は文書の変更のたびに `MapPolicy::Other` と同じ向き (JMP-4)、閉じた文書の項目は消す (JMP-5)。メモリだけに
//!   持つ (JMP-6)。
//!
//! spec: 03 §5.9, 03 §4.5.2, 03 §9.9, 01 FR-310

use std::collections::BTreeMap;

use eve_text::{ByteOff, ChangeSet, Op, Selection, TextRead};
use eve_view::{DocumentId, MessageLevel, ViewId};

use crate::command::{
    CommandCtx, CommandKind, CommandRegistry, CommandResult, Permission, RejectReason, ids,
};
use crate::document::Document;
use crate::editor::state::{DocSet, EditorState};
use crate::editor::window::{PaneRef, Window};
use crate::ui_model::keys;

/// ペインごとのジャンプ履歴の項目の数の上限 (03 §5.9 の JMP-2 の `JUMP_HISTORY_MAX`)。超えたら古いものから捨てる。
pub(crate) const JUMP_HISTORY_MAX: usize = 100;

/// ジャンプ履歴の項目 (03 §5.9 の JMP-2)。`pos` は文書の変更のたびに写像する (JMP-4)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct JumpEntry {
    pub(crate) doc: DocumentId,
    pub(crate) pos: ByteOff,
}

/// ペインのジャンプ履歴 (03 §5.9 の JMP-2)。`at` は今の位置の添字で、`entries.len()` なら末尾 (まだ戻っていない)。
#[derive(Clone, Debug, Default)]
pub(crate) struct JumpHistory {
    entries: Vec<JumpEntry>,
    at: usize,
}

impl JumpHistory {
    /// 項目の並び (古い順)。
    pub(crate) fn entries(&self) -> &[JumpEntry] {
        &self.entries
    }

    /// 今の位置の添字 (`entries().len()` なら末尾)。
    pub(crate) fn at(&self) -> usize {
        self.at
    }

    /// `e` を記録する (JMP-2): 今の位置より先の項目を捨ててから末尾に足す。末尾の項目が `e` と同じ文書の同じ行
    /// (`same_line`) なら、足さずに置き換える。上限を超えたら古いものから捨てる。今の位置は末尾になる。
    pub(crate) fn record(
        &mut self,
        e: JumpEntry,
        same_line: impl Fn(JumpEntry, JumpEntry) -> bool,
    ) {
        // 今の位置の項目 (戻った先) は残し、その先 (進む側) を捨てる
        let keep = (self.at + 1).min(self.entries.len());
        self.entries.truncate(keep);
        match self.entries.last_mut() {
            Some(last) if same_line(*last, e) => *last = e,
            _ => self.entries.push(e),
        }
        if self.entries.len() > JUMP_HISTORY_MAX {
            let over = self.entries.len() - JUMP_HISTORY_MAX;
            self.entries.drain(..over);
        }
        self.at = self.entries.len();
    }

    /// `cursor.jump_back` (JMP-3): 今の位置 `cur` と同じ文書の同じ行の項目を飛ばして、前の項目へ移る。末尾 (まだ戻って
    /// いない) なら、先に `cur` を末尾に記録する (進むで戻れる)。前の項目が無ければ何もせず `None`。
    pub(crate) fn back(
        &mut self,
        cur: JumpEntry,
        same_line: impl Fn(JumpEntry, JumpEntry) -> bool,
    ) -> Option<JumpEntry> {
        let at = self.at.min(self.entries.len());
        // 移る先が無ければ記録もしない (端では何もしない)
        self.entries[..at]
            .iter()
            .rev()
            .find(|e| !same_line(**e, cur))?;
        let from = if at >= self.entries.len() {
            self.record(cur, &same_line);
            // 末尾の項目は今の位置 (記録した `cur`)
            self.entries.len().saturating_sub(1)
        } else {
            at
        };
        let i = (0..from)
            .rev()
            .find(|&i| !same_line(self.entries[i], cur))?;
        self.at = i;
        Some(self.entries[i])
    }

    /// `cursor.jump_forward` (JMP-3): 今の位置より後ろの項目のうち、`cur` と同じ文書の同じ行でない最初のものへ移る。
    /// 末尾 (戻っていない)・後ろの項目が無ければ何もせず `None`。
    pub(crate) fn forward(
        &mut self,
        cur: JumpEntry,
        same_line: impl Fn(JumpEntry, JumpEntry) -> bool,
    ) -> Option<JumpEntry> {
        let start = self.at.checked_add(1)?;
        let i = (start..self.entries.len()).find(|&i| !same_line(self.entries[i], cur))?;
        self.at = i;
        Some(self.entries[i])
    }

    /// 文書 `doc` の項目を消し、今の位置の添字を消した項目の分だけ詰める (JMP-5)。今の位置の項目 (`at < len`) が `doc`
    /// のものなら、消さずに `cur` (ペインが代わりに出した文書の今の位置) で置き換える (`cur` が無ければ消す)。
    pub(crate) fn forget(&mut self, doc: DocumentId, cur: Option<JumpEntry>) {
        if let (Some(e), Some(cur)) = (self.entries.get_mut(self.at), cur)
            && e.doc == doc
        {
            *e = cur;
        }
        let before = self.entries[..self.at.min(self.entries.len())]
            .iter()
            .filter(|e| e.doc == doc)
            .count();
        self.entries.retain(|e| e.doc != doc);
        self.at = (self.at - before).min(self.entries.len());
    }

    /// 文書 `doc` の項目の位置を `f` で写す (JMP-4)。
    pub(crate) fn map_doc(&mut self, doc: DocumentId, mut f: impl FnMut(ByteOff) -> ByteOff) {
        for e in self.entries.iter_mut().filter(|e| e.doc == doc) {
            e.pos = f(e.pos);
        }
    }
}

/// `a` と `b` が同じ文書の同じ行 (03 §5.9 の JMP-2 / JMP-3)。文書が無ければ偽。
fn same_line(docs: &BTreeMap<DocumentId, Document>, a: JumpEntry, b: JumpEntry) -> bool {
    if a.doc != b.doc {
        return false;
    }
    docs.get(&a.doc).is_some_and(|d| {
        let t = d.text();
        let len = t.len_bytes();
        t.byte_to_line(ByteOff(a.pos.0.min(len))) == t.byte_to_line(ByteOff(b.pos.0.min(len)))
    })
}

/// `pos` を文書の範囲と書記素の境界に収める (03 §5.9 の JMP-4)。
fn settle(t: &dyn TextRead, pos: ByteOff) -> ByteOff {
    eve_text::snap_to_nearest_grapheme(t, ByteOff(pos.0.min(t.len_bytes())))
}

/// 昇順の位置 `ps` を `cs` で写す (`ChangeSet::map_pos` の `Bias::Before` と同じ結果。`MapPolicy::Other`)。演算を 1 回だけ
/// たどる (費用は演算の数 + 位置の数)。各位置について、写した位置と、それが変更の範囲かその隣 (変更の範囲から
/// `GRAPHEME_CONTEXT` バイト以内。書記素の境界が変わり得る所) にあるかを返す。
fn map_sorted(cs: &ChangeSet, ps: &[ByteOff]) -> Vec<(ByteOff, bool)> {
    const NEAR: usize = eve_text::GRAPHEME_CONTEXT;
    let mut out = Vec::with_capacity(ps.len());
    let mut i = 0;
    let (mut old, mut new) = (0usize, 0usize);
    // 直前の変更の終わり (新しい本文の座標)
    let mut last_change: Option<usize> = None;
    let ops = cs.ops();
    for (k, op) in ops.iter().enumerate() {
        match op {
            Op::Retain(n) => {
                let end = old.saturating_add(*n);
                // 次に変更の演算が続くなら、残す範囲の終わりの近くも変更の隣
                let change_after = ops.get(k + 1).is_some();
                while let Some(&p) = ps.get(i).filter(|p| p.0 < end) {
                    let q = new + (p.0 - old);
                    let near = last_change.is_some_and(|c| q - c <= NEAR)
                        || (change_after && end - p.0 <= NEAR);
                    out.push((ByteOff(q), near));
                    i += 1;
                }
                old = end;
                new = new.saturating_add(*n);
            }
            Op::Delete(n) => {
                let end = old.saturating_add(*n);
                while ps.get(i).is_some_and(|p| p.0 <= end) {
                    out.push((ByteOff(new), true));
                    i += 1;
                }
                old = end;
                last_change = Some(new);
            }
            Op::Insert(t) => {
                while ps.get(i).is_some_and(|p| p.0 == old) {
                    out.push((ByteOff(new), true));
                    i += 1;
                }
                new = new.saturating_add(t.len());
                last_change = Some(new);
            }
        }
    }
    // 文書の末尾 (と範囲の外)
    while i < ps.len() {
        out.push((ByteOff(new), last_change.is_some_and(|c| new - c <= NEAR)));
        i += 1;
    }
    out
}

/// ペイン `at` のジャンプ履歴。
fn jumps_in(windows: &mut [Window], at: PaneRef) -> Option<&mut JumpHistory> {
    windows
        .iter_mut()
        .find(|w| w.id == at.0)?
        .panes
        .get_mut(&at.1)
        .map(|p| &mut p.jumps)
}

impl EditorState {
    /// ペイン `at` の今の (文書, 主の Range の `head`)。
    pub(crate) fn jump_point(&self, at: PaneRef) -> Option<JumpEntry> {
        let v = self.view(self.pane_view(at)?)?;
        Some(JumpEntry {
            doc: v.doc,
            pos: v.selection.primary().head,
        })
    }

    /// View `view` を表示するペインと、その今の位置 (ジャンプにする移動の前に取る印)。どのペインにも表示していなければ
    /// `None` (記録しない)。
    pub(crate) fn jump_mark(&self, view: ViewId) -> Option<(PaneRef, JumpEntry)> {
        let at = self.pane_of_shown_view(view)?;
        Some((at, self.jump_point(at)?))
    }

    /// ジャンプにする移動の後: 印のペインの位置か文書が印から変わっていれば、印の位置を記録する (03 §5.9 の JMP-1)。
    /// 記録したら `true`。
    pub(crate) fn commit_jump(&mut self, mark: Option<(PaneRef, JumpEntry)>) -> bool {
        let Some((at, before)) = mark else {
            return false;
        };
        if self.jump_point(at) == Some(before) {
            return false;
        }
        self.note_jump(at, before);
        true
    }

    /// 検索の移動 (`search.next` / `search.prev`・`search.replace` の後の移動) の前の印 (03 §5.9 の JMP-1 (b))。検索バーを
    /// 開いてから閉じるまでに、もう記録していれば `None` (続けた移動は最初の移動の前の 1 項目だけ)。
    pub(crate) fn search_jump_mark(&self, view: ViewId) -> Option<(PaneRef, JumpEntry)> {
        if self.find.search_bar.as_ref().is_some_and(|b| b.jump_noted) {
            return None;
        }
        self.jump_mark(view)
    }

    /// 検索の移動の後 (`commit_jump`)。記録したら、検索バーが閉じるまで続く移動では記録しない。
    pub(crate) fn commit_search_jump(&mut self, mark: Option<(PaneRef, JumpEntry)>) {
        if self.commit_jump(mark)
            && let Some(bar) = &mut self.find.search_bar
        {
            bar.jump_noted = true;
        }
    }

    /// ペイン `at` のジャンプ履歴に `e` を記録する (03 §5.9 の JMP-2)。
    pub(crate) fn note_jump(&mut self, at: PaneRef, e: JumpEntry) {
        let DocSet {
            windows, documents, ..
        } = &mut self.docs;
        let Some(d) = documents.get(&e.doc) else {
            return;
        };
        // 本文の範囲と書記素の境界に収めて記録する (写した位置を渡す置換の後の移動など、JMP-4)
        let e = JumpEntry {
            pos: settle(d.text(), e.pos),
            ..e
        };
        if let Some(h) = jumps_in(windows, at) {
            h.record(e, |a, b| same_line(documents, a, b));
        }
    }

    /// 文書 `doc` の全ペインの項目の位置 (昇順、重複なし)。
    fn jump_positions(&self, doc: DocumentId) -> Vec<ByteOff> {
        let mut ps: Vec<ByteOff> = self
            .docs
            .windows
            .iter()
            .flat_map(|w| w.panes.values())
            .flat_map(|p| p.jumps.entries().iter())
            .filter(|e| e.doc == doc)
            .map(|e| e.pos)
            .collect();
        ps.sort_unstable();
        ps.dedup();
        ps
    }

    /// 文書 `doc` の全ペインの項目の位置を、昇順の位置 `from` と同じ順の写した先 `to` の表で置き換える。
    fn replace_jump_positions(&mut self, doc: DocumentId, from: &[ByteOff], to: &[ByteOff]) {
        for p in self
            .docs
            .windows
            .iter_mut()
            .flat_map(|w| w.panes.values_mut())
        {
            p.jumps
                .map_doc(doc, |pos| from.binary_search(&pos).map_or(pos, |i| to[i]));
        }
    }

    /// 文書 `doc` の変更 `cs` で、全ペインの項目の位置を写す (03 §5.9 の JMP-4。`MapPolicy::Other` と同じ `Bias::Before`)。
    /// 文書の全ペインの項目をまとめて並べ、演算を 1 回だけたどる (費用は演算の数 + 項目の数 × log。JMP-7)。書記素の
    /// 境界に収めるのは、写した位置が変更の範囲かその隣のときだけ。
    pub(crate) fn map_jumps(&mut self, doc: DocumentId, cs: &ChangeSet) {
        let from = self.jump_positions(doc);
        if from.is_empty() {
            return;
        }
        let Some(d) = self.docs.documents.get(&doc) else {
            return;
        };
        let t = d.text();
        let to: Vec<ByteOff> = map_sorted(cs, &from)
            .into_iter()
            .map(|(q, near)| if near { settle(t, q) } else { q })
            .collect();
        self.replace_jump_positions(doc, &from, &to);
    }

    /// 本文を丸ごと置き換えた文書 `doc` (再読込とその Undo / Redo) の項目を、前の本文 `old` の行:桁から今の本文の同じ
    /// 行:桁へ写す (03 §5.9 の JMP-4、§9.5 の写像。行・桁が足りなければ最後の行・行末)。
    pub(crate) fn remap_jumps_line_col(&mut self, doc: DocumentId, old: &dyn TextRead) {
        let Some(new) = self.docs.documents.get(&doc).map(Document::snapshot) else {
            return;
        };
        self.remap_jumps_line_col_to(doc, old, &new);
    }

    /// `remap_jumps_line_col` の、新しい本文 `new` を渡す形 (開き直しで、文書を差し替える前に呼ぶ)。
    pub(crate) fn remap_jumps_line_col_to(
        &mut self,
        doc: DocumentId,
        old: &dyn TextRead,
        new: &dyn TextRead,
    ) {
        let from = self.jump_positions(doc);
        if from.is_empty() {
            return;
        }
        let to = crate::io::reload::same_line_cols(old, new, &from, |t, p| {
            eve_text::next_grapheme_boundary(t, p)
        });
        self.replace_jump_positions(doc, &from, &to);
    }

    /// 本文を差し替えた文書 `doc` (読込の完了・先行表示から全文へ・読込の失敗の保留) の項目を、新しい本文の範囲と書記素の
    /// 境界に収める (03 §5.9 の段 8 の細目)。
    pub(crate) fn settle_jumps_of(&mut self, doc: DocumentId) {
        let DocSet {
            windows, documents, ..
        } = &mut self.docs;
        let Some(d) = documents.get(&doc) else {
            return;
        };
        let t = d.text();
        for p in windows.iter_mut().flat_map(|w| w.panes.values_mut()) {
            p.jumps.map_doc(doc, |pos| settle(t, pos));
        }
    }

    /// 閉じた文書 `doc` の項目を、全ペインの履歴から消す (03 §5.9 の JMP-5)。ペインの今の位置の項目が `doc` のものなら、
    /// その場所には、そのペインが代わりに出した文書の今の位置を置く (閉じた後も進める)。表示中のペインを先に別の文書へ
    /// 替えてから呼ぶ。
    pub(crate) fn forget_jumps_of(&mut self, doc: DocumentId) {
        let panes: Vec<PaneRef> = self
            .docs
            .windows
            .iter()
            .flat_map(|w| w.panes.keys().map(move |p| (w.id, *p)))
            .collect();
        for at in panes {
            let cur = self.jump_point(at).filter(|e| e.doc != doc);
            if let Some(h) = jumps_in(&mut self.docs.windows, at) {
                h.forget(doc, cur);
            }
        }
    }

    /// ジャンプ履歴の不変条件 (03 §5.9 の JMP-2 / JMP-4 / JMP-5): 上限・今の位置が範囲の中・項目の文書が開いている・位置が
    /// 本文の範囲の中で書記素の境界。`check_panes` が呼ぶ。
    pub(crate) fn check_jumps(&self) -> Result<(), String> {
        for w in &self.docs.windows {
            for (pid, p) in &w.panes {
                let h = &p.jumps;
                if h.entries().len() > JUMP_HISTORY_MAX || h.at() > h.entries().len() {
                    return Err(format!(
                        "{pid:?}: ジャンプ履歴の項目 {} 件・今の位置 {}",
                        h.entries().len(),
                        h.at()
                    ));
                }
                for e in h.entries() {
                    let Some(d) = self.docs.documents.get(&e.doc) else {
                        return Err(format!(
                            "{pid:?}: 閉じた文書 {:?} がジャンプ履歴に残る",
                            e.doc
                        ));
                    };
                    let t = d.text();
                    if e.pos.0 > t.len_bytes() || !eve_text::is_grapheme_boundary(t, e.pos) {
                        return Err(format!(
                            "{pid:?}: ジャンプ履歴の位置 {:?} が {:?} の範囲の外か書記素の境界でない",
                            e.pos, e.doc
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// `cursor.jump_back` / `cursor.jump_forward` の本体 (03 §5.9 の JMP-3 / JMP-7)。
    fn jump(&mut self, at: PaneRef, dir: Direction) -> CommandResult {
        let Some(cur) = self.jump_point(at) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let DocSet {
            windows, documents, ..
        } = &mut self.docs;
        let docs: &BTreeMap<DocumentId, Document> = documents;
        let target = jumps_in(windows, at).and_then(|h| match dir {
            Direction::Back => h.back(cur, |a, b| same_line(docs, a, b)),
            Direction::Forward => h.forward(cur, |a, b| same_line(docs, a, b)),
        });
        let Some(target) = target else {
            let key = match dir {
                Direction::Back => keys::INFO_JUMP_NO_BACK,
                Direction::Forward => keys::INFO_JUMP_NO_FORWARD,
            };
            self.show_message(MessageLevel::Info, key, &[]);
            return CommandResult::Done;
        };
        // 項目の文書をそのペインに出す (ペインの文書の切り替えと同じ。記録はしない。View は文書ごと、03 §9.9 の PANE-2)
        if self.pane_document(at) != Some(target.doc) {
            self.show_document_in_without_jump(at, target.doc);
        }
        let Some(view) = self.pane_view(at) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(pos) = self
            .docs
            .documents
            .get(&target.doc)
            .map(|d| settle(d.text(), target.pos))
        else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        // 主のカーソル 1 つ (選択なし) を置き、`cursor.goto_line` と同じく行を見える範囲の中央へ置く (03 §4.5.2 の「1 つにする」)
        self.set_selection(view, Selection::point(pos));
        self.center_cursor(view);
        CommandResult::Done
    }
}

/// 戻る・進むの向き。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    Back,
    Forward,
}

fn run(ctx: &mut CommandCtx<'_>, d: Direction) -> CommandResult {
    match ctx.state.focused_pane() {
        Some(at) => ctx.state.jump(at, d),
        None => CommandResult::Rejected(RejectReason::NoTarget),
    }
}

fn jump_back(ctx: &mut CommandCtx<'_>) -> CommandResult {
    run(ctx, Direction::Back)
}

fn jump_forward(ctx: &mut CommandCtx<'_>) -> CommandResult {
    run(ctx, Direction::Forward)
}

/// v0.5 の M34 のジャンプ履歴のコマンド (07 §11.6 の「編集の拡張」。どれも Motion・`read`・`when` なし)。
pub(crate) fn register(r: &mut CommandRegistry) {
    for (id, h) in [
        (ids::CURSOR_JUMP_BACK, jump_back as crate::Handler),
        (ids::CURSOR_JUMP_FORWARD, jump_forward),
    ] {
        r.add(id, CommandKind::Motion, Permission::Read, "", h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: DocumentId = DocumentId(1);
    const B: DocumentId = DocumentId(2);

    fn e(doc: DocumentId, pos: usize) -> JumpEntry {
        JumpEntry {
            doc,
            pos: ByteOff(pos),
        }
    }

    /// 位置の 10 バイトごとを 1 行とみなす。
    fn line(a: JumpEntry, b: JumpEntry) -> bool {
        a.doc == b.doc && a.pos.0 / 10 == b.pos.0 / 10
    }

    #[test]
    fn 記録は末尾に足し_同じ行の末尾は置き換える() {
        let mut h = JumpHistory::default();
        h.record(e(A, 0), line);
        h.record(e(A, 5), line);
        h.record(e(A, 20), line);
        h.record(e(B, 20), line);
        assert_eq!(h.entries(), [e(A, 5), e(A, 20), e(B, 20)]);
        assert_eq!(h.at(), 3);
    }

    #[test]
    fn 上限を超えたら古いものから捨てる() {
        let mut h = JumpHistory::default();
        for i in 0..JUMP_HISTORY_MAX + 5 {
            h.record(e(A, i * 10), line);
        }
        assert_eq!(h.entries().len(), JUMP_HISTORY_MAX);
        assert_eq!(h.entries()[0], e(A, 50));
        assert_eq!(h.at(), JUMP_HISTORY_MAX);
    }

    #[test]
    fn 戻ると末尾に今の位置を記録し_同じ行の項目を飛ばす() {
        let mut h = JumpHistory::default();
        h.record(e(A, 0), line);
        h.record(e(A, 30), line);
        // 今は A の 35 (30 と同じ行): 30 を飛ばして 0 へ
        assert_eq!(h.back(e(A, 35), line), Some(e(A, 0)));
        // 30 は同じ行の 35 で置き換わる
        assert_eq!(h.entries(), [e(A, 0), e(A, 35)]);
        assert_eq!(h.at(), 0);
        assert_eq!(h.back(e(A, 0), line), None, "前の端");
        assert_eq!(h.forward(e(A, 0), line), Some(e(A, 35)));
        assert_eq!(h.forward(e(A, 35), line), None, "後ろの端");
    }

    #[test]
    fn 端で戻っても記録しない() {
        let mut h = JumpHistory::default();
        assert_eq!(h.back(e(A, 0), line), None);
        assert_eq!(h.entries(), []);
        h.record(e(A, 3), line);
        assert_eq!(h.back(e(A, 5), line), None, "同じ行の項目だけ");
        assert_eq!(h.entries(), [e(A, 3)]);
        assert_eq!(h.at(), 1);
        assert_eq!(h.forward(e(A, 5), line), None);
    }

    #[test]
    fn 戻った後の記録は先を捨てる() {
        let mut h = JumpHistory::default();
        for p in [0, 10, 20] {
            h.record(e(A, p), line);
        }
        assert_eq!(h.back(e(A, 30), line), Some(e(A, 20)));
        assert_eq!(h.back(e(A, 20), line), Some(e(A, 10)));
        // 10 にいて 50 へ移った: 10 の先 (20・30) を捨てて足す
        h.record(e(A, 10), line);
        assert_eq!(h.entries(), [e(A, 0), e(A, 10)]);
        h.record(e(B, 0), line);
        assert_eq!(h.entries(), [e(A, 0), e(A, 10), e(B, 0)]);
        assert_eq!(h.at(), 3);
    }

    #[test]
    fn 閉じた文書の項目を消して今の位置を詰める() {
        let mut h = JumpHistory::default();
        for x in [e(A, 0), e(B, 0), e(A, 10), e(B, 10), e(A, 20)] {
            h.record(x, line);
        }
        assert_eq!(h.back(e(A, 30), line), Some(e(A, 20)));
        assert_eq!(h.back(e(A, 20), line), Some(e(B, 10)));
        assert_eq!(h.at(), 3);
        h.forget(B, None);
        assert_eq!(h.entries(), [e(A, 0), e(A, 10), e(A, 20), e(A, 30)]);
        // B の 2 項目のうち、今の位置 (3) より前の 1 つの分だけ詰める (今の位置の項目は消えたので、次の項目が今の位置)
        assert_eq!(h.at(), 2);
        let mut h2 = JumpHistory::default();
        h2.record(e(B, 0), line);
        h2.forget(B, None);
        assert_eq!(h2.entries(), []);
        assert_eq!(h2.at(), 0);
    }

    const C: DocumentId = DocumentId(3);

    #[test]
    fn 今の位置の項目の文書を閉じると代わりに出した文書の位置を置き_進める() {
        // A10・B20・C30 と移って B20 へ戻った後に B を閉じ、ペインには C (5) が出た
        let mut h = JumpHistory::default();
        for x in [e(A, 10), e(B, 20)] {
            h.record(x, line);
        }
        assert_eq!(h.back(e(C, 30), line), Some(e(B, 20)));
        assert_eq!(h.at(), 1);
        h.forget(B, Some(e(C, 5)));
        assert_eq!(h.entries(), [e(A, 10), e(C, 5), e(C, 30)]);
        assert_eq!(h.at(), 1);
        assert_eq!(h.forward(e(C, 5), line), Some(e(C, 30)), "閉じた後も進める");
        assert_eq!(h.back(e(C, 30), line), Some(e(C, 5)));
        assert_eq!(h.back(e(C, 5), line), Some(e(A, 10)));
    }

    use eve_text::{Bias, Edit, Text};
    use proptest::prelude::*;

    proptest! {
        /// 1 回の走査の写像は、位置ごとの `map_pos` (`Bias::Before`) と同じ。変更の範囲の外 (隣より遠く) の位置は
        /// 変更の隣と見なさない
        #[test]
        fn まとめた写像は位置ごとの写像と同じ(
            len in 0usize..200,
            edits in proptest::collection::vec((0usize..200, 0usize..20, 0usize..6), 0..6),
            ps in proptest::collection::vec(0usize..210, 0..20),
        ) {
            let text = Text::from("a".repeat(len).as_str());
            let mut sorted: Vec<(usize, usize, usize)> =
                edits.into_iter().map(|(s, l, n)| (s.min(len), l, n)).collect();
            sorted.sort_unstable();
            let mut list = Vec::new();
            let mut at = 0;
            for (start, l, n) in sorted {
                if start < at {
                    continue;
                }
                let end = (start + l).min(len);
                list.push(Edit {
                    range: eve_text::ByteRange::new(ByteOff(start), ByteOff(end)),
                    insert: std::sync::Arc::from("b".repeat(n).as_str()),
                });
                at = end + 1;
            }
            let cs = ChangeSet::from_sorted_edits(&text, list).expect("重ならない編集");
            let mut ps: Vec<ByteOff> = ps.into_iter().map(ByteOff).collect();
            ps.sort_unstable();
            ps.dedup();
            let got = map_sorted(&cs, &ps);
            for (p, (q, near)) in ps.iter().zip(&got) {
                prop_assert_eq!(*q, cs.map_pos(*p, Bias::Before), "{:?}", p);
                if !*near {
                    // 変更の範囲から離れている位置は、変更の前後で同じ距離だけずれる (書記素の境界は変わらない)
                    let far = cs.changed_ranges().all(|(before, _)| {
                        p.0 + eve_text::GRAPHEME_CONTEXT < before.start.0
                            || p.0 > before.end.0 + eve_text::GRAPHEME_CONTEXT
                    });
                    prop_assert!(far, "{:?} は変更の隣", p);
                }
            }
        }
    }
}
