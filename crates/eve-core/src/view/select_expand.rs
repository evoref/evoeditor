//! 選択の拡張 (FR-309、docs/03 §5.7 の SEL-1〜SEL-7、07 §11.6 の「編集の拡張」、01 FR-309 AC-1)。
//!
//! - `select.line`: 各 Range を行の全体に広げ、行の全体なら次の 1 行を足す (SEL-1)。
//! - `select.expand`: 各 Range を、真に含む最小の候補 (単語・括弧の中・括弧を含む・行の中身・行の全体・文書) に広げる (SEL-2)。
//!   変える前の Selection を View の拡張の履歴に積む (SEL-4)。
//! - `select.shrink`: 拡張の履歴の最後の Selection に戻す (SEL-3)。
//!
//! 3 つとも Motion で文書を変えない (SEL-7)。矩形選択のモード (03 §4.8) の間は矩形を消してから行う。候補を求める関数は
//! `view::text_object`。
//!
//! spec: 03 §5.7, 03 §4.5.2, 03 §4.8, 01 FR-309

use std::collections::VecDeque;

use eve_text::{Range, Selection};
use eve_view::{Revision, ViewId};

use crate::command::{
    CommandCtx, CommandKind, CommandRegistry, CommandResult, Permission, RejectReason, ids,
};
use crate::editor::state::EditorState;
use crate::view::motion;
use crate::view::text_object::{self, ExpandCtx};

/// View ごとの拡張の履歴に積む Selection の数の上限 (03 §5.7 の SEL-4)。超えたら古いものから捨てる。
pub(crate) const SELECT_EXPAND_HISTORY_MAX: usize = 64;

/// 拡張の履歴 (03 §5.7 の SEL-4)。`select.expand` で変える前の Selection を積み、`select.expand` / `select.shrink` の後に
/// 取った文書の Revision と Selection の写しを持つ。今の文書の Revision と Selection が写しと同じ間だけ有効 (03 §4.5.1 の
/// MCUR-2 (j) の「単語の一致」の印と同じ持ち方。選択を置き直す `set_selection` は履歴を捨てる)。
#[derive(Clone, Debug)]
pub(crate) struct ExpandHistory {
    revision: Revision,
    selection: Selection,
    stack: VecDeque<Selection>,
}

impl EditorState {
    /// View `view` の拡張の履歴を取り出す。有効でなければ (編集・ほかの選択の操作の後) 空にする。
    fn take_expand_history(&mut self, view: ViewId) -> VecDeque<Selection> {
        let Some(v) = self.docs.views.get_mut(&view) else {
            return VecDeque::new();
        };
        let history = v.expand_history.take();
        let Some(d) = self.docs.documents.get(&v.doc) else {
            return VecDeque::new();
        };
        match history {
            Some(h) if h.revision == d.revision() && h.selection == v.selection => h.stack,
            _ => VecDeque::new(),
        }
    }

    /// 今の文書の Revision と Selection を写しにして、拡張の履歴を置く (空なら置かない)。
    fn put_expand_history(&mut self, view: ViewId, stack: VecDeque<Selection>) {
        if stack.is_empty() {
            return;
        }
        let Some(v) = self.docs.views.get(&view) else {
            return;
        };
        let Some(revision) = self.docs.documents.get(&v.doc).map(|d| d.revision()) else {
            return;
        };
        let history = ExpandHistory {
            revision,
            selection: v.selection.clone(),
            stack,
        };
        if let Some(v) = self.docs.views.get_mut(&view) {
            v.expand_history = Some(history);
        }
    }

    /// `select.line` (03 §5.7 の SEL-1)。履歴には積まない (SEL-4)。
    fn select_line(&mut self, view: ViewId) -> CommandResult {
        let Some(v) = self.docs.views.get(&view) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(d) = self.docs.documents.get(&v.doc) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let t = d.text();
        let ranges = v.selection.ranges();
        let out: Vec<Range> = ranges
            .iter()
            .map(|r| text_object::select_line(t, *r))
            .collect();
        if out == ranges {
            // 文書の末尾まで選んでいる・区切りで終わる文書の最後の空の行: 変えない (知らせない)
            return CommandResult::Done;
        }
        let Some(sel) = Selection::new(out, v.selection.primary_index()) else {
            return CommandResult::Rejected(RejectReason::InvalidArgs);
        };
        // 重なった Range は `normalized()` で 1 つになる (SEL-5)。目標の列と「単語の一致」の印も消える (SEL-7)
        self.set_selection(view, sel);
        self.ensure_cursor_visible(view);
        CommandResult::Done
    }

    /// `select.expand` (03 §5.7 の SEL-2 / SEL-4 / SEL-5)。
    fn select_expand(&mut self, view: ViewId) -> CommandResult {
        let Some(v) = self.docs.views.get(&view) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(d) = self.docs.documents.get(&v.doc) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let ranges = v.selection.ranges();
        let primary = v.selection.primary_index();
        // 括弧を探して読む量は全 Range の合計で 8 MiB まで。主の Range を先に、残りは文書の順に広げる (SEL-5)。前の Range の
        // 単語と字下げは使い回す
        let out = text_object::expand_all(d.text(), &v.selection, &mut ExpandCtx::new());
        if out == ranges {
            // 文書の全体を選んでいる (どの候補も Range を真に含まない): 変えない
            return CommandResult::Done;
        }
        let before = v.selection.clone();
        let Some(sel) = Selection::new(out, primary) else {
            return CommandResult::Rejected(RejectReason::InvalidArgs);
        };
        let mut stack = self.take_expand_history(view);
        stack.push_back(before);
        while stack.len() > SELECT_EXPAND_HISTORY_MAX {
            stack.pop_front();
        }
        // 広げて重なった Range は `normalized()` で 1 つになる (`select.shrink` は併合の前の Selection に戻す)
        self.set_selection(view, sel);
        self.put_expand_history(view, stack);
        self.ensure_cursor_visible(view);
        CommandResult::Done
    }

    /// `select.shrink` (03 §5.7 の SEL-3)。履歴が無ければ何もしない (知らせない)。
    fn select_shrink(&mut self, view: ViewId) -> CommandResult {
        if !self.docs.views.contains_key(&view) {
            return CommandResult::Rejected(RejectReason::NoTarget);
        }
        let mut stack = self.take_expand_history(view);
        let Some(prev) = stack.pop_back() else {
            return CommandResult::Done;
        };
        self.set_selection(view, prev);
        self.put_expand_history(view, stack);
        self.ensure_cursor_visible(view);
        CommandResult::Done
    }
}

fn run(
    ctx: &mut CommandCtx<'_>,
    f: impl FnOnce(&mut EditorState, ViewId) -> CommandResult,
) -> CommandResult {
    match motion::active(ctx) {
        Some((view, _)) => {
            // 矩形選択のモードの間は、矩形を消してから広げる (SEL-7、03 §4.8)。Range は複数カーソルとして広げる
            ctx.state.clear_box_rect(view);
            f(ctx.state, view)
        }
        None => CommandResult::Rejected(RejectReason::NoTarget),
    }
}

fn select_line(ctx: &mut CommandCtx<'_>) -> CommandResult {
    run(ctx, EditorState::select_line)
}

fn select_expand(ctx: &mut CommandCtx<'_>) -> CommandResult {
    run(ctx, EditorState::select_expand)
}

fn select_shrink(ctx: &mut CommandCtx<'_>) -> CommandResult {
    run(ctx, EditorState::select_shrink)
}

/// v0.5 の M34 の選択の拡張のコマンド (07 §11.6 の「編集の拡張」。どれも Motion・`read`・`when` なし)。
pub(crate) fn register(r: &mut CommandRegistry) {
    for (id, h) in [
        (ids::SELECT_LINE, select_line as crate::Handler),
        (ids::SELECT_EXPAND, select_expand),
        (ids::SELECT_SHRINK, select_shrink),
    ] {
        r.add(id, CommandKind::Motion, Permission::Read, "", h);
    }
}
