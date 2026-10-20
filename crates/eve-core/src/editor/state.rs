//! `EditorState`: 文書・View・Window の集合と、変更の唯一の経路 (docs/02 §5.1 / §5.2、docs/03 §6.5 の手順 7〜12)。

use std::collections::{BTreeMap, BTreeSet};

use eve_text::{Bias, ByteOff, LineIdx, Selection, TextRead};
use eve_view::{
    Capabilities, DocumentId, EditorEvent, Effect, OriginKind, Revision, ViewId, WindowId,
};

use crate::document::{Applied, AppliedTx, ApplyError, Document, Transaction};
use crate::time::Tick;

use super::context::CtxValue;

/// スクロール位置。`top` は先頭可視行の行頭 (03 §4.6)。折り返しと水平スクロールは TUI の版で足す。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScrollPos {
    pub top: ByteOff,
}

/// 文書の表示と Selection の単位。
#[derive(Clone, Debug)]
pub struct View {
    pub doc: DocumentId,
    pub selection: Selection,
    pub scroll: ScrollPos,
    /// 垂直移動の目標の表示列 (03 §5.2)。水平移動・編集・クリックでクリアし、垂直移動は維持する。
    pub goal_dcol: Option<u32>,
}

/// frontend が持つ編集面。v0.x は 1 つで、View を 1 つ持つ。
#[derive(Clone, Copy, Debug)]
pub struct Window {
    pub id: WindowId,
    pub root: ViewId,
}

pub struct EditorState {
    pub(crate) documents: BTreeMap<DocumentId, Document>,
    pub(crate) views: BTreeMap<ViewId, View>,
    pub(crate) windows: Vec<Window>,
    next_id: u64,
    /// Editor の「今」。Host が各メッセージの処理の前に `tick` で渡した値で、コマンドの実行中に読み直さない。
    pub(crate) now: Tick,
    pub(crate) caps: Capabilities,
    effects: Vec<Effect>,
    events: Vec<EditorEvent>,
    snapshot_updates: BTreeSet<DocumentId>,
    needs_render: bool,
}

impl EditorState {
    pub fn new(caps: Capabilities) -> EditorState {
        EditorState {
            documents: BTreeMap::new(),
            views: BTreeMap::new(),
            windows: Vec::new(),
            next_id: 1,
            now: Tick(0),
            caps,
            effects: Vec::new(),
            events: Vec::new(),
            snapshot_updates: BTreeSet::new(),
            needs_render: true,
        }
    }

    fn fresh(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn now(&self) -> Tick {
        self.now
    }

    // ---------- 文書・View・Window ----------

    /// 文書を追加し、それを表示する View を作る (カーソルは先頭)。文書の ID は閉じた後も再利用しない。
    pub fn add_document(&mut self, doc: Document) -> (DocumentId, ViewId) {
        let doc_id = DocumentId(self.fresh());
        self.documents.insert(doc_id, doc);
        let view_id = self.add_view(doc_id);
        self.events
            .push(EditorEvent::DocumentOpened { doc: doc_id });
        self.snapshot_updates.insert(doc_id);
        self.needs_render = true;
        (doc_id, view_id)
    }

    pub fn add_view(&mut self, doc: DocumentId) -> ViewId {
        let id = ViewId(self.fresh());
        self.views.insert(
            id,
            View {
                doc,
                selection: Selection::point(ByteOff(0)),
                scroll: ScrollPos::default(),
                goal_dcol: None,
            },
        );
        id
    }

    pub fn add_window(&mut self, root: ViewId) -> WindowId {
        let id = WindowId(self.fresh());
        self.windows.push(Window { id, root });
        id
    }

    pub fn document(&self, id: DocumentId) -> Option<&Document> {
        self.documents.get(&id)
    }

    /// 文書の形式・読み取り専用の理由・パスなどを更新するための可変参照 (テキストは `apply` でしか変えられない)。
    pub fn document_mut(&mut self, id: DocumentId) -> Option<&mut Document> {
        self.documents.get_mut(&id)
    }

    /// 先頭可視行を設定する (行頭へ丸める)。
    pub fn set_scroll_top(&mut self, view: ViewId, pos: ByteOff) {
        let Some(v) = self.views.get_mut(&view) else {
            return;
        };
        let Some(d) = self.documents.get(&v.doc) else {
            return;
        };
        let text = d.text();
        v.scroll.top = text.line_to_byte(text.byte_to_line(pos));
        self.needs_render = true;
    }

    pub fn view(&self, id: ViewId) -> Option<&View> {
        self.views.get(&id)
    }

    pub fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows.iter().find(|w| w.id == id)
    }

    /// Window が表示している View。
    pub fn window_view(&self, id: WindowId) -> Option<ViewId> {
        self.window(id).map(|w| w.root)
    }

    /// 最初の Window (v0.x は 1 つ)。
    pub fn first_window(&self) -> Option<WindowId> {
        self.windows.first().map(|w| w.id)
    }

    // ---------- 出力 ----------

    pub fn push_effect(&mut self, e: Effect) {
        self.effects.push(e);
    }

    pub fn drain_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    pub fn push_event(&mut self, e: EditorEvent) {
        self.events.push(e);
    }

    pub fn drain_events(&mut self) -> Vec<EditorEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn take_snapshot_updates(&mut self) -> Vec<DocumentId> {
        std::mem::take(&mut self.snapshot_updates)
            .into_iter()
            .collect()
    }

    pub fn needs_render(&self) -> bool {
        self.needs_render
    }

    /// Frame を生成した後に呼ぶ。
    pub fn mark_rendered(&mut self) {
        self.needs_render = false;
    }

    pub fn mark_dirty_render(&mut self) {
        self.needs_render = true;
    }

    // ---------- 変更の唯一の経路 ----------

    /// Transaction を適用し、全 View の Selection・スクロールを写像し、通知を積む (03 §6.5 の手順 7・10・12)。
    /// 発行 View は `tx.view`。
    pub fn apply_transaction(
        &mut self,
        doc: DocumentId,
        tx: Transaction,
    ) -> Result<Revision, ApplyError> {
        let document = self.documents.get_mut(&doc).ok_or(ApplyError::Rejected)?;
        match document.apply(&tx)? {
            AppliedTx::Identity { selection } => {
                // 変更なし: 発行 View の Selection だけを更新する (goal_dcol は編集と同じく解除)
                if let (Some(view), Some(sel)) = (tx.view, selection) {
                    self.set_selection(view, sel);
                }
                Ok(self.documents[&doc].revision())
            }
            AppliedTx::Changed(applied) => {
                let rev = applied.revision;
                self.after_change(doc, &applied, tx.view);
                Ok(rev)
            }
        }
    }

    /// Undo。取り消すものがなければ `Ok(false)`。発行 View の Selection は `selection_before` に戻す。
    pub fn undo(&mut self, doc: DocumentId, view: Option<ViewId>) -> Result<bool, ApplyError> {
        let document = self.documents.get_mut(&doc).ok_or(ApplyError::Rejected)?;
        let applied = document.undo()?;
        Ok(applied.map(|a| self.after_change(doc, &a, view)).is_some())
    }

    pub fn redo(&mut self, doc: DocumentId, view: Option<ViewId>) -> Result<bool, ApplyError> {
        let document = self.documents.get_mut(&doc).ok_or(ApplyError::Rejected)?;
        let applied = document.redo()?;
        Ok(applied.map(|a| self.after_change(doc, &a, view)).is_some())
    }

    fn after_change(&mut self, doc: DocumentId, a: &Applied, issuer: Option<ViewId>) {
        let Some(document) = self.documents.get(&doc) else {
            return;
        };
        let text = document.text();
        for (id, view) in self.views.iter_mut().filter(|(_, v)| v.doc == doc) {
            if Some(*id) == issuer {
                // 発行 View: 指定された Selection (なければ自分の編集として写像)。goal_dcol はクリア
                let sel = match &a.selection {
                    Some(s) => s.clone().normalized(text),
                    None => view
                        .selection
                        .map(&a.changes, eve_text::MapPolicy::Own)
                        .normalized(text),
                };
                view.selection = sel;
                view.goal_dcol = None;
            } else {
                // 他の View: 他者の編集として写像。goal_dcol は維持
                view.selection = view
                    .selection
                    .map(&a.changes, eve_text::MapPolicy::Other)
                    .normalized(text);
            }
            // スクロール位置は同じ関数で写像し、行頭へ丸める
            let top = a.changes.map_pos(view.scroll.top, Bias::Before);
            view.scroll.top = text.line_to_byte(LineIdx(text.byte_to_line(top).0));
        }
        self.events.push(EditorEvent::DocumentChanged {
            doc,
            revision: a.revision,
            origin_kind: OriginKind {
                origin: a.origin.tag(),
                is_undo: a.is_undo,
            },
        });
        self.snapshot_updates.insert(doc);
        self.needs_render = true;
    }

    /// Window の View が表示している文書の Undo の結合を封印する (カーソル移動の実行時)。
    pub fn seal_active_history(&mut self) {
        let doc = self
            .windows
            .first()
            .and_then(|w| self.views.get(&w.root))
            .map(|v| v.doc);
        if let Some(d) = doc.and_then(|d| self.documents.get_mut(&d)) {
            d.seal_history();
        }
    }

    /// View の Selection を設定する (書記素境界へ正規化し、`goal_dcol` はクリア)。
    pub fn set_selection(&mut self, view: ViewId, sel: Selection) {
        let Some(v) = self.views.get_mut(&view) else {
            return;
        };
        let Some(d) = self.documents.get(&v.doc) else {
            return;
        };
        v.selection = sel.normalized(d.text());
        v.goal_dcol = None;
        self.needs_render = true;
    }

    // ---------- 条件式の文脈 ----------

    /// `when` 句で参照できる識別子の値 (07 §7.1 / §11.6)。知らない識別子は `None` (偽)。
    pub fn context_value(&self, name: &str) -> Option<CtxValue> {
        let view = self.windows.first().and_then(|w| self.views.get(&w.root));
        let doc = view.and_then(|v| self.documents.get(&v.doc));
        let b = |x: bool| Some(CtxValue::Bool(x));
        match name {
            "doc.read_only" => doc.map_or(b(false), |d| b(!d.is_editable())),
            "doc.has_path" => b(doc.is_some_and(|d| d.path.is_some())),
            "doc.dirty" => b(doc.is_some_and(Document::is_dirty)),
            "selection.empty" => b(view.is_none_or(|v| v.selection.iter().all(|r| r.is_empty()))),
            "overlay.active" => Some(CtxValue::Str(String::new())),
            "search.active" | "search.replace_active" => b(false),
            "caps.can_suspend" => b(self.caps.can_suspend),
            "editor" => b(true),
            _ => None,
        }
    }
}
