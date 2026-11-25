//! Window・ペイン・レイアウトの木と、文書・ペイン・View の関係 (docs/03 §9.9 の PANE-1 / PANE-2 / PANE-8 / PANE-9)。
//!
//! ペインの ID (`PaneId`) は core の内部の型で、境界の型には出さない (境界ではペインを表示中の `ViewId` で指す、PANE-7)。
//! View からペインを引くときは各ペインの表 (`Pane.views`) をたどる (View にペインの ID を持たせない)。
//!
//! spec: 03 §9.9 (PANE-1, PANE-8, PANE-9, PANE-13)

use std::collections::{BTreeMap, BTreeSet};

use eve_view::{CellRect, CellSize, DocumentId, ViewId, WindowId};

use crate::editor::state::{EditorState, View};
use crate::layout::panes::{LeafRect, PANE_MIN, PaneLayout, layout, min_size};
use crate::view::jump::JumpHistory;

/// ペインの識別子 (core の内部。Window の中で一意で、閉じた後も再利用しない)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct PaneId(pub(crate) u64);

/// 分割の向き (PANE-3)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Axis {
    /// 左右に並べる。子の間に区切りを 1 列置く。
    Row,
    /// 上下に並べる。区切りは置かない (上の子の見出しが区切りになる)。
    Column,
}

/// レイアウトの木 (PANE-3)。分割の子は 2 つ以上で、比率は分割ごとに合計 10,000 の正の整数。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LayoutNode {
    /// ペイン。
    Leaf(PaneId),
    /// 分割。子と、その比率。
    Split {
        axis: Axis,
        children: Vec<(LayoutNode, u16)>,
    },
}

impl LayoutNode {
    /// 葉を木の葉の順 (深さ優先。`Row` は左から、`Column` は上から) に並べる。
    pub(crate) fn leaves(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        self.collect_leaves(&mut out);
        out
    }

    fn collect_leaves(&self, out: &mut Vec<PaneId>) {
        match self {
            LayoutNode::Leaf(p) => out.push(*p),
            LayoutNode::Split { children, .. } => {
                for (c, _) in children {
                    c.collect_leaves(out);
                }
            }
        }
    }
}

/// ペイン (レイアウトの木の葉)。編集面の矩形を 1 つ持ち、そこに View を 1 つ表示する (PANE-1)。
#[derive(Clone, Debug)]
pub(crate) struct Pane {
    /// 表示中の View (`views` の値のどれか)。
    pub(crate) view: ViewId,
    /// このペインで表示したことのある文書の View (文書 → View)。どの View も高々 1 つのペインの表にある。
    pub(crate) views: BTreeMap<DocumentId, ViewId>,
    /// このペインで表示した文書 (表示した順。末尾が直近。PANE-9 の閉じた後に表示する文書の選択に使う)。
    pub(crate) recent: Vec<DocumentId>,
    /// ジャンプ履歴 (03 §5.9 の JMP-2。ペインの中で文書を切り替えても同じ履歴が続く。分割は写し、閉じると捨てる)。
    pub(crate) jumps: JumpHistory,
}

/// frontend の画面 1 つ (v0.x は 1 つ)。レイアウトの木と、フォーカスのあるペインを持つ (03 §9.9)。
#[derive(Clone, Debug)]
pub struct Window {
    /// Window の識別子。
    pub id: WindowId,
    pub(crate) panes: BTreeMap<PaneId, Pane>,
    pub(crate) root: LayoutNode,
    /// フォーカスのあるペイン (入力と表示の対象。`panes` にある)。
    pub(crate) focus: PaneId,
    /// フォーカスしたペイン (順。末尾が直近。PANE-5 (d) のペインを閉じた後のフォーカスに使う)。
    pub(crate) pane_recent: Vec<PaneId>,
    /// 縮退表示の最中 (PANE-4 (b))。入ったときに 1 回だけ知らせるために、前の状態を覚える。
    pub(crate) collapsed: bool,
}

impl Window {
    /// ペインを 1 つ持つ Window。`view` は `doc` の View。
    fn new(id: WindowId, pane: PaneId, view: ViewId, doc: Option<DocumentId>) -> Window {
        let views = doc.map(|d| (d, view)).into_iter().collect();
        Window {
            id,
            panes: BTreeMap::from([(
                pane,
                Pane {
                    view,
                    views,
                    recent: Vec::new(),
                    jumps: JumpHistory::default(),
                },
            )]),
            root: LayoutNode::Leaf(pane),
            focus: pane,
            pane_recent: vec![pane],
            collapsed: false,
        }
    }

    /// フォーカスのあるペイン。
    pub(crate) fn focused_pane(&self) -> Option<&Pane> {
        self.panes.get(&self.focus)
    }

    /// ペインを木の葉の順に並べる。
    pub(crate) fn leaves(&self) -> Vec<PaneId> {
        self.root.leaves()
    }

    /// View `view` を表示中のペイン。
    pub(crate) fn pane_showing(&self, view: ViewId) -> Option<PaneId> {
        self.panes
            .iter()
            .find(|(_, p)| p.view == view)
            .map(|(id, _)| *id)
    }

    /// 編集面 `size` が木の最小より小さい (縮退表示、PANE-4 (b))。葉が 1 つの木は縮退しない (端末の最小は frontend)。
    pub(crate) fn is_collapsed(&self, size: CellSize) -> bool {
        if matches!(self.root, LayoutNode::Leaf(_)) {
            return false;
        }
        let min = min_size(&self.root);
        size.cols < min.cols || size.rows < min.rows
    }

    /// 編集面 `size` に出す各ペインの配置 (PANE-3、PANE-10 (a))。縮退表示では、フォーカスのペインだけを編集面の全体に、
    /// 見出しと区切りなしで置く (PANE-4 (b))。
    pub(crate) fn layout(&self, size: CellSize) -> PaneLayout {
        if self.is_collapsed(size) {
            return PaneLayout {
                leaves: vec![LeafRect {
                    pane: self.focus,
                    body: CellRect {
                        col: 0,
                        row: 0,
                        cols: size.cols,
                        rows: size.rows,
                    },
                    bar: None,
                }],
                dividers: Vec::new(),
            };
        }
        layout(&self.root, size)
    }

    /// ペインの操作 (方向の移動・大きさ) が使う木の配置: 縮退表示では木の最小の大きさで計算した配置、そうでなければ
    /// 編集面の配置 (PANE-4 (b))。
    pub(crate) fn logical_layout(&self, size: CellSize) -> PaneLayout {
        if self.is_collapsed(size) {
            return layout(&self.root, min_size(&self.root));
        }
        layout(&self.root, size)
    }
}

/// 文書を表示するペインの位置 (Window とその中のペイン)。
pub(crate) type PaneRef = (WindowId, PaneId);

impl EditorState {
    /// View `root` を表示するペインを 1 つ持つ Window を作る。`root` はペインに属さない View (PANE-1) で、
    /// そのペインの表に移す。
    pub fn add_window(&mut self, root: ViewId) -> WindowId {
        let id = WindowId(self.fresh_id());
        // ペインの ID は文書・View・Window とは別に振る (ViewId・DocumentId の採番を変えない)
        let pane = PaneId(self.docs.next_pane);
        self.docs.next_pane += 1;
        let doc = self.docs.views.get(&root).map(|v| v.doc);
        self.docs.windows.push(Window::new(id, pane, root, doc));
        if let Some(doc) = doc {
            self.docs.last_view.insert(doc, root);
        }
        id
    }

    /// 入力と表示の対象のペイン (フォーカスのある Window のフォーカスのあるペイン)。
    pub(crate) fn focused_pane(&self) -> Option<PaneRef> {
        let w = self.docs.windows.first()?;
        Some((w.id, w.focus))
    }

    pub(crate) fn window_mut(&mut self, id: WindowId) -> Option<&mut Window> {
        self.docs.windows.iter_mut().find(|w| w.id == id)
    }

    fn pane(&self, at: PaneRef) -> Option<&Pane> {
        self.window(at.0)?.panes.get(&at.1)
    }

    fn pane_mut(&mut self, at: PaneRef) -> Option<&mut Pane> {
        self.window_mut(at.0)?.panes.get_mut(&at.1)
    }

    /// ペイン `at` が表示中の View。
    pub(crate) fn pane_view(&self, at: PaneRef) -> Option<ViewId> {
        self.pane(at).map(|p| p.view)
    }

    /// ペイン `at` が表示中の文書。
    pub(crate) fn pane_document(&self, at: PaneRef) -> Option<DocumentId> {
        self.view(self.pane(at)?.view).map(|v| v.doc)
    }

    /// 文書 `doc` を表示中のペイン (Window の順、Window の中は木の葉の順)。
    pub(crate) fn panes_showing(&self, doc: DocumentId) -> Vec<PaneRef> {
        self.docs
            .windows
            .iter()
            .flat_map(|w| w.leaves().into_iter().map(move |p| (w.id, p)))
            .filter(|at| self.pane_document(*at) == Some(doc))
            .collect()
    }

    /// View `view` を表に持つペイン (表示中でなくてもよい)。どのペインにも属さない View は `None`。
    pub(crate) fn pane_owning(&self, view: ViewId) -> Option<PaneRef> {
        self.docs.windows.iter().find_map(|w| {
            w.panes
                .iter()
                .find(|(_, p)| p.views.values().any(|v| *v == view))
                .map(|(id, _)| (w.id, *id))
        })
    }

    /// View `view` を表示するペインの本文の矩形 (編集面の左上が原点)。どのペインにも表示されていない View は `None`。
    pub(crate) fn view_rect(&self, view: ViewId) -> Option<CellRect> {
        let size = self.surface_size();
        self.docs.windows.iter().find_map(|w| {
            let pane = w.pane_showing(view)?;
            pane_body(&w.layout(size), pane)
        })
    }

    /// View `view` の可視の大きさ (列, 行): その View を表示するペインの本文の大きさ (03 §9.9 の PANE-13)。表示していない
    /// View は、表に持つペイン (次に表示するペイン) の大きさ、どのペインにも属さない View はフォーカスのペインの大きさ。
    /// 縮退表示で隠れているペインの View は、編集面の全体 (フォーカスを移すとその大きさで出る、PANE-4 (b))。
    pub(crate) fn view_size(&self, view: ViewId) -> (u16, u16) {
        if let Some(size) = self
            .term
            .view_sizes
            .as_ref()
            .and_then(|m| m.get(&view).copied())
        {
            return size;
        }
        let rect = self.view_rect(view).or_else(|| {
            let (w, p) = self.pane_owning(view).or_else(|| self.focused_pane())?;
            pane_body(&self.window(w)?.layout(self.surface_size()), p)
        });
        match rect {
            Some(r) => (r.cols, r.rows),
            None => self.viewport(),
        }
    }

    /// 全 View の可視の大きさ (`view_size` と同じ規則)。Window ごとに配置を 1 回だけ求める (全 View のスクロールを
    /// 調整するときに、View ごとに配置を求め直さない)。
    pub(crate) fn all_view_sizes(&self) -> BTreeMap<ViewId, (u16, u16)> {
        let size = self.surface_size();
        let whole = (size.cols, size.rows);
        let mut out = BTreeMap::new();
        let mut free = None;
        for (i, w) in self.docs.windows.iter().enumerate() {
            let l = w.layout(size);
            for (pid, p) in &w.panes {
                let body = pane_body(&l, *pid).map_or(whole, |r| (r.cols, r.rows));
                for v in p.views.values() {
                    out.insert(*v, body);
                }
                if i == 0 && *pid == w.focus {
                    free = Some(body);
                }
            }
        }
        let free = free.unwrap_or(whole);
        for v in self.docs.views.keys() {
            out.entry(*v).or_insert(free);
        }
        out
    }

    /// `f` の間だけ、全 View の可視の大きさを 1 回求めたものを使う (配置は `f` の中で変えないこと)。
    pub(crate) fn with_view_sizes<R>(&mut self, f: impl FnOnce(&mut EditorState) -> R) -> R {
        let outer = self.term.view_sizes.is_some();
        if !outer {
            self.term.view_sizes = Some(self.all_view_sizes());
        }
        let r = f(self);
        if !outer {
            self.term.view_sizes = None;
        }
        r
    }

    /// 編集面の大きさ (`Frame.size`)。
    pub(crate) fn surface_size(&self) -> CellSize {
        let (cols, rows) = self.viewport();
        CellSize { cols, rows }
    }

    /// 文書 `doc` をペイン `at` に表示する (PANE-1 / PANE-8 (a))。そのペインの表に文書の View があればそれを、無ければ
    /// ペインに属さない View を移し、それも無ければ最後にフォーカスのあった View を写した View を作る。表示を替える前の
    /// 文書を、そのペインの `recent` と Window 全体の `recent` に記録する。フォーカスのペインなら検索バーを閉じる
    /// (検索の状態はペインごとに持たない、03 §9.8)。表示を替える前の文書の位置を、そのペインのジャンプ履歴に記録する
    /// (03 §5.9 の JMP-1 (d))。
    pub(crate) fn show_document_in(&mut self, at: PaneRef, doc: DocumentId) {
        self.switch_document_in(at, doc, SwitchJump::Record);
    }

    /// `show_document_in` と同じだが、ジャンプ履歴に記録しない (閉じる文書の代わりを出す・`cursor.jump_back` /
    /// `cursor.jump_forward` の移動・表示する View の決まっている所。03 §5.9 の JMP-1)。
    pub(crate) fn show_document_in_without_jump(&mut self, at: PaneRef, doc: DocumentId) {
        self.switch_document_in(at, doc, SwitchJump::Skip);
    }

    fn switch_document_in(&mut self, at: PaneRef, doc: DocumentId, jump: SwitchJump) {
        if !self.docs.documents.contains_key(&doc) || self.pane(at).is_none() {
            return;
        }
        let prev = self.pane_document(at);
        if prev == Some(doc) {
            return;
        }
        if jump == SwitchJump::Record
            && let Some(e) = self.jump_point(at)
        {
            self.note_jump(at, e);
        }
        let focused = self.focused_pane() == Some(at);
        if focused {
            self.search_close();
        }
        if let Some(prev) = prev {
            self.note_recent(at, prev);
        }
        let (view, fresh) = match self.pane(at).and_then(|p| p.views.get(&doc).copied()) {
            Some(v) if self.docs.views.contains_key(&v) => (NewPaneView::Existing(v), false),
            _ => (self.view_for_new_pane(doc), true),
        };
        let (view, before_cols) = match view {
            NewPaneView::Existing(v) => (v, None),
            // 写した View のスクロールは、写し元の View の本文の桁数で求めたもの
            NewPaneView::Copied { view, src_cols } => (view, Some(src_cols)),
        };
        let before_cols = before_cols.or_else(|| fresh.then(|| self.view_size(view).0));
        self.put_view_in(at, doc, view);
        if fresh {
            // 表示するペインと本文の桁数 (折り返し幅) が違えば、折り返しの表示行の番号を 0 に戻す (PANE-1、03 §14.3.1)
            if before_cols != Some(self.view_size(view).0)
                && let Some(v) = self.docs.views.get_mut(&view)
            {
                v.scroll.wrap_idx = 0;
            }
            // 写して作った View は写し元のペインと、ペインに属さない View はそれまで大きさを求めたペインと、本文の大きさが
            // 違うことがある。表示するペインの矩形でカーソルが見えるようにする (PANE-1)。ペインの表から出した View は
            // そのまま使う (位置を保つ、FR-103 AC-2。大きさの変更は `after_relayout` と `apply_viewport` が合わせている)
            self.ensure_cursor_visible(view);
        }
        if focused {
            self.docs.last_view.insert(doc, view);
        }
        self.mark_dirty_render();
    }

    /// View `view` をペイン `at` に表示する (読込の完了・起動・最後の文書を閉じた後の無題の文書など、表示する View の
    /// 決まっている所)。`recent` には記録しない。`view` をほかのペインが表示していれば、そのペインから取り上げずに
    /// 文書を表示する (`show_document_in`。どのペインの表示中の View も、そのペインの表にある)。
    pub(crate) fn show_view_in(&mut self, at: PaneRef, view: ViewId) {
        let Some(doc) = self.view(view).map(|v| v.doc) else {
            return;
        };
        let shown_elsewhere = self.docs.windows.iter().any(|w| {
            w.panes
                .iter()
                .any(|(id, p)| (w.id, *id) != at && p.view == view)
        });
        if shown_elsewhere {
            return self.show_document_in_without_jump(at, doc);
        }
        self.put_view_in(at, doc, view);
        if self.focused_pane() == Some(at) {
            self.docs.last_view.insert(doc, view);
        }
        self.mark_dirty_render();
    }

    /// ペイン `at` の表の `doc` を `view` にして表示する。表にあった別の View は消す (文書にはほかの View があるので、
    /// ペインに属さない View として残さない)。ほかのペインの表にあった `view` は、そのペインの表から外す。
    fn put_view_in(&mut self, at: PaneRef, doc: DocumentId, view: ViewId) {
        if let Some(owner) = self.pane_owning(view).filter(|o| *o != at)
            && let Some(p) = self.pane_mut(owner)
        {
            p.views.retain(|_, v| *v != view);
        }
        let Some(p) = self.pane_mut(at) else {
            return;
        };
        p.view = view;
        let replaced = p.views.insert(doc, view).filter(|old| *old != view);
        if let Some(old) = replaced {
            self.docs.views.remove(&old);
            self.docs.last_view.retain(|_, v| *v != old);
        }
    }

    /// 文書 `doc` を初めて表示するペインに渡す View (PANE-1): ペインに属さない View があればそれ (`ViewId` は変わらない)、
    /// 無ければ最後にフォーカスのあった View を写した新しい View。
    fn view_for_new_pane(&mut self, doc: DocumentId) -> NewPaneView {
        let own: Vec<ViewId> = self
            .docs
            .views
            .iter()
            .filter(|(_, v)| v.doc == doc)
            .map(|(id, _)| *id)
            .collect();
        if let Some(free) = own.iter().copied().find(|v| self.pane_owning(*v).is_none()) {
            return NewPaneView::Existing(free);
        }
        let source = self
            .docs
            .last_view
            .get(&doc)
            .copied()
            .filter(|v| own.contains(v))
            .or_else(|| own.first().copied());
        let src_cols = source.map(|v| self.view_size(v).0);
        match (source.and_then(|v| self.copy_view(v)), src_cols) {
            (Some(view), Some(src_cols)) => NewPaneView::Copied { view, src_cols },
            (Some(view), None) => NewPaneView::Existing(view),
            (None, _) => NewPaneView::Existing(self.add_view(doc)),
        }
    }

    /// View `src` の選択・スクロール・上下の移動の目標の列・折り返しの設定を写した新しい View (PANE-1、PANE-5 (a))。
    /// 矩形選択のモードと矩形は写さない (写した View は切った状態。03 §4.8 の BOX-1)。
    pub(crate) fn copy_view(&mut self, src: ViewId) -> Option<ViewId> {
        let mut copy: View = self.docs.views.get(&src).cloned()?;
        copy.box_mode = false;
        copy.box_mark = None;
        let id = self.add_view(copy.doc);
        if let Some(v) = self.docs.views.get_mut(&id) {
            *v = copy;
        }
        Some(id)
    }

    /// ペイン `at` で表示を替える前の文書 `prev` を、そのペインと Window 全体の最近使った順に記録する。
    fn note_recent(&mut self, at: PaneRef, prev: DocumentId) {
        if let Some(p) = self.pane_mut(at) {
            p.recent.retain(|d| *d != prev);
            p.recent.push(prev);
        }
        self.docs.recent.retain(|d| *d != prev);
        self.docs.recent.push(prev);
    }

    /// 閉じた文書 `doc` を、全ペインの表・最近使った順・最後にフォーカスのあった View の記録から消す (閉じた文書の ID は
    /// どの記録にも残さない、03 §9.9)。表示中のペインの表示は替えない (呼び側が先に替える)。
    pub(crate) fn forget_document_in_panes(&mut self, doc: DocumentId) {
        for w in &mut self.docs.windows {
            for p in w.panes.values_mut() {
                p.views.remove(&doc);
                p.recent.retain(|d| *d != doc);
            }
        }
        self.docs.recent.retain(|d| *d != doc);
        self.docs.last_view.remove(&doc);
    }

    /// 文書 `doc` を閉じた後にペイン `at` に表示する文書 (PANE-9): そのペインの `recent` の新しい順、Window 全体の
    /// `recent` の新しい順、開いた順の最初の文書のうち、開いていて `doc` でないもの。
    pub(crate) fn next_document_for(&self, at: PaneRef, doc: DocumentId) -> Option<DocumentId> {
        let open = |d: &DocumentId| *d != doc && self.docs.documents.contains_key(d);
        self.pane(at)
            .and_then(|p| p.recent.iter().rev().copied().find(open))
            .or_else(|| self.docs.recent.iter().rev().copied().find(open))
            .or_else(|| self.docs.documents.keys().copied().find(open))
    }

    /// ペインの表と View の関係の不変条件 (03 §9.9 の PANE-1) を確かめる。試験と fuzz が各操作の後に呼ぶ。
    ///
    /// # Errors
    ///
    /// 破れている条件を文で返す。
    pub fn check_panes(&self) -> Result<(), String> {
        let mut owners: BTreeMap<ViewId, PaneRef> = BTreeMap::new();
        let size = self.surface_size();
        for w in &self.docs.windows {
            let leaves = w.leaves();
            let set: BTreeSet<PaneId> = leaves.iter().copied().collect();
            if set.len() != leaves.len() || !set.iter().eq(w.panes.keys()) {
                return Err(format!(
                    "{:?}: 木の葉 {leaves:?} とペインの表 {:?} が合わない",
                    w.id,
                    w.panes.keys().collect::<Vec<_>>()
                ));
            }
            // 木の形 (PANE-3): 分割の子は 2 つ以上・比率は正で合計 10,000・同じ向きの分割を親子にしない
            w.root
                .check_shape()
                .map_err(|e| format!("{:?}: 木の形: {e}", w.id))?;
            if !w.panes.contains_key(&w.focus) {
                return Err(format!("{:?}: フォーカスのペインが無い", w.id));
            }
            if w.pane_recent.iter().any(|p| !w.panes.contains_key(p)) {
                return Err(format!("{:?}: 閉じたペインが pane_recent に残る", w.id));
            }
            check_tiling(w, size)?;
            for (pid, p) in &w.panes {
                if !p.views.values().any(|v| *v == p.view) {
                    return Err(format!("{pid:?}: 表示中の View が表に無い"));
                }
                for (doc, view) in &p.views {
                    match self.docs.views.get(view) {
                        Some(v) if v.doc == *doc => {}
                        _ => return Err(format!("{pid:?}: 表の {doc:?} → {view:?} が合わない")),
                    }
                    if let Some(other) = owners.insert(*view, (w.id, *pid)) {
                        return Err(format!("{view:?} が 2 つのペイン ({other:?}) の表にある"));
                    }
                }
                if let Some(d) = p
                    .recent
                    .iter()
                    .find(|d| !self.docs.documents.contains_key(d))
                {
                    return Err(format!("{pid:?}: 閉じた文書 {d:?} が recent に残る"));
                }
            }
        }
        for doc in self.docs.documents.keys() {
            if !self.docs.views.values().any(|v| v.doc == *doc) {
                return Err(format!("{doc:?} に View が無い"));
            }
        }
        if let Some(d) = self
            .docs
            .recent
            .iter()
            .find(|d| !self.docs.documents.contains_key(d))
        {
            return Err(format!("閉じた文書 {d:?} が Window 全体の recent に残る"));
        }
        for (doc, view) in &self.docs.last_view {
            if self.docs.views.get(view).map(|v| v.doc) != Some(*doc) {
                return Err(format!("{doc:?} の最後の View {view:?} が無い・別の文書"));
            }
        }
        // ジャンプ履歴 (03 §5.9 の JMP-2 / JMP-4 / JMP-5)
        self.check_jumps()
    }
}

/// 文書の切り替えをジャンプ履歴に記録するか (03 §5.9 の JMP-1 (d))。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SwitchJump {
    Record,
    Skip,
}

/// 配置の不変条件 (09 §3.2 の「eve-core 複数ペイン」の (a)): 縮退表示でなければ、全ペインの本文と見出しと区切りの矩形が
/// 編集面を重なりも隙間もなく埋め、各ペインが最小 (20 × 3 と見出し) 以上。縮退表示ではフォーカスのペインだけが編集面の
/// 全体に出る。
fn check_tiling(w: &Window, size: CellSize) -> Result<(), String> {
    let l = w.layout(size);
    if w.is_collapsed(size) {
        let ok = l.leaves.len() == 1
            && l.leaves[0].pane == w.focus
            && l.leaves[0].bar.is_none()
            && (l.leaves[0].body.cols, l.leaves[0].body.rows) == (size.cols, size.rows)
            && l.dividers.is_empty();
        return if ok {
            Ok(())
        } else {
            Err(format!("{:?}: 縮退表示の配置 {l:?}", w.id))
        };
    }
    let split = w.panes.len() > 1;
    let mut rects: Vec<CellRect> = l.dividers.clone();
    for x in &l.leaves {
        if split {
            let Some(bar) = x.bar else {
                return Err(format!("{:?}: {:?} に見出しが無い", w.id, x.pane));
            };
            if x.body.cols < PANE_MIN.cols || x.body.rows + bar.rows < PANE_MIN.rows {
                return Err(format!("{:?}: {:?} が最小より小さい: {x:?}", w.id, x.pane));
            }
            rects.push(bar);
        }
        rects.push(x.body);
    }
    let area = |r: &CellRect| u32::from(r.cols) * u32::from(r.rows);
    let total: u32 = rects.iter().map(area).sum();
    if total != u32::from(size.cols) * u32::from(size.rows) {
        return Err(format!(
            "{:?}: 矩形の面積の和 {total} が編集面 {size:?} と違う: {l:?}",
            w.id
        ));
    }
    for (i, a) in rects.iter().enumerate() {
        if u32::from(a.col) + u32::from(a.cols) > u32::from(size.cols)
            || u32::from(a.row) + u32::from(a.rows) > u32::from(size.rows)
        {
            return Err(format!("{:?}: {a:?} が編集面の外に出る", w.id));
        }
        for b in &rects[i + 1..] {
            let apart = a.col + a.cols <= b.col
                || b.col + b.cols <= a.col
                || a.row + a.rows <= b.row
                || b.row + b.rows <= a.row;
            if !apart && area(a) > 0 && area(b) > 0 {
                return Err(format!("{:?}: {a:?} と {b:?} が重なる", w.id));
            }
        }
    }
    Ok(())
}

/// 配置 `l` の中のペイン `pane` の本文の矩形。
fn pane_body(l: &PaneLayout, pane: PaneId) -> Option<CellRect> {
    l.leaves
        .iter()
        .find(|x: &&LeafRect| x.pane == pane)
        .map(|x| x.body)
}

/// ペインに文書を初めて出すときに渡す View (`view_for_new_pane`)。
enum NewPaneView {
    /// そのまま使う View (ペインの表の View・ペインに属さない View・新しく作った View)。
    Existing(ViewId),
    /// ほかの View を写した View と、写し元の本文の桁数 (折り返し幅が違えば `wrap_idx` を戻す、PANE-1)。
    Copied { view: ViewId, src_cols: u16 },
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;
    use std::sync::Arc;

    use eve_text::{ByteOff, Selection, Text};
    use eve_view::{Capabilities, CellPos, JobId, Revision};

    use super::*;
    use crate::FsPath;
    use crate::document::{Document, TextFormat};
    use crate::io::fs::FsError;
    use crate::jobs::{JobKind, JobSpawner};

    struct NoJobs;
    impl JobSpawner for NoJobs {
        fn spawn(&self, _: Option<DocumentId>, _: Revision, _: JobKind) -> JobId {
            JobId(1)
        }
        fn cancel(&self, _: JobId) {}
    }

    struct StubPaths;
    impl crate::io::paths::Paths for StubPaths {
        fn config_dir(&self) -> FsPath {
            FsPath::from("/c")
        }
        fn state_dir(&self) -> FsPath {
            FsPath::from("/s")
        }
        fn data_dir(&self) -> FsPath {
            FsPath::from("/d")
        }
        fn runtime_dir(&self) -> FsPath {
            FsPath::from("/r")
        }
        fn normalize(&self, raw: &str) -> Result<FsPath, FsError> {
            Ok(FsPath::from(raw))
        }
    }

    fn doc(text: &str) -> Document {
        Document::from_text(Text::from(text), TextFormat::new_document())
    }

    /// 81 × 24 の編集面に、文書 `first` を出す Window を 1 つ作る。
    fn state(first: &str) -> (EditorState, DocumentId, ViewId) {
        let mut st = EditorState::new(
            Capabilities::default(),
            Rc::new(NoJobs),
            Arc::new(StubPaths),
        );
        st.set_viewport(81, 24);
        let (d, v) = st.add_document(doc(first));
        st.add_window(v);
        (st, d, v)
    }

    /// フォーカスのペインを右に分ける (`pane.split_right` の本体)。新しいペインにフォーカスが移る。
    fn split_right(st: &mut EditorState) -> PaneRef {
        assert_eq!(
            st.split_pane(Axis::Row),
            crate::command::CommandResult::Done
        );
        st.focused_pane().expect("ペインがある")
    }

    fn focus(st: &mut EditorState, at: PaneRef) {
        st.focus_pane(at);
    }

    #[test]
    fn 文書を作った_view_は最初に表示したペインの表へそのまま移す() {
        let (mut st, a, va) = state("a\n");
        let (b, vb) = st.add_document(doc("b\n"));
        assert_eq!(
            st.pane_owning(vb),
            None,
            "表示するまではどのペインにも属さない"
        );
        st.activate_document(b);
        let at = st.focused_pane().expect("ペインがある");
        assert_eq!(st.window_view(at.0), Some(vb), "ViewId は変わらない");
        assert_eq!(st.pane_owning(va), Some(at));
        assert_eq!(st.pane_owning(vb), Some(at));
        // 戻ると表の View をそのまま使う
        st.activate_document(a);
        assert_eq!(st.window_view(at.0), Some(va));
        assert_eq!(st.docs.views.len(), 2, "View を作り足さない");
        assert_eq!(st.pane(at).map(|p| p.recent.clone()), Some(vec![a, b]));
        assert_eq!(st.docs.recent, vec![a, b]);
        st.check_panes().expect("不変条件");
    }

    #[test]
    fn 同じ文書を_2_つのペインで出すと_view_は_2_つで_最後にフォーカスのあった_view_を写す() {
        let (mut st, a, va) = state("one\ntwo\nthree\n");
        st.set_selection(va, Selection::point(ByteOff(5)));
        let right = split_right(&mut st);
        let vr = st.pane(right).map(|p| p.view).expect("右のペイン");
        assert_ne!(vr, va);
        assert_eq!(st.view(vr).map(|v| v.doc), Some(a));
        assert_eq!(
            st.view(vr).map(|v| v.selection.primary().head),
            Some(ByteOff(5)),
            "選択を写す"
        );
        assert_eq!(st.panes_showing(a).len(), 2);
        st.check_panes().expect("不変条件");
    }

    #[test]
    fn view_の大きさはペインの本文の矩形() {
        let (mut st, _, va) = state("a\n");
        assert_eq!(st.view_size(va), (81, 24), "1 ペインは編集面の全体");
        let right = split_right(&mut st);
        let vr = st.pane(right).map(|p| p.view).expect("右のペイン");
        // 81 列 − 区切り 1 を半分ずつ、見出しの 1 行を除く
        assert_eq!(st.view_size(va), (40, 23));
        assert_eq!(st.view_size(vr), (40, 23));
        assert_eq!(
            st.view_rect(vr).map(|r| (r.col, r.row)),
            Some((41, 0)),
            "編集面の左上が原点"
        );
    }

    #[test]
    fn 文書を閉じると表示していた全ペインがそれぞれの順で次の文書を出す() {
        let (mut st, a, _) = state("a\n");
        let (b, _) = st.add_document(doc("b\n"));
        let (c, _) = st.add_document(doc("c\n"));
        let left = st.focused_pane().expect("ペインがある");
        st.activate_document(c);
        st.activate_document(b);
        st.activate_document(a);
        // 左のペインの直近は b、右のペインの直近は c (右は a を写して作り、c → a と切り替える)
        let right = split_right(&mut st);
        focus(&mut st, right);
        st.activate_document(c);
        st.activate_document(a);
        st.close_buffer(a);
        assert_eq!(st.pane_document(left), Some(b), "左はそのペインの直近");
        assert_eq!(st.pane_document(right), Some(c), "右はそのペインの直近");
        assert_eq!(st.panes_showing(a), Vec::<PaneRef>::new());
        assert!(!st.docs.recent.contains(&a), "閉じた文書を記録に残さない");
        st.check_panes().expect("不変条件");
    }

    #[test]
    fn 最後の文書を閉じると無題の文書を作り_表示していた全ペインに出す() {
        let (mut st, a, _) = state("a\n");
        let left = st.focused_pane().expect("ペインがある");
        let right = split_right(&mut st);
        st.close_buffer(a);
        assert_eq!(st.documents_len(), 1);
        let blank = st.pane_document(left).expect("無題の文書");
        assert_ne!(blank, a);
        assert_eq!(st.pane_document(right), Some(blank));
        st.check_panes().expect("不変条件");
    }

    #[test]
    fn frame_は葉の順にペインを並べ_主カーソルに矩形の位置を足す() {
        let (mut st, _, va) = state("abc\n");
        st.set_selection(va, Selection::point(ByteOff(2)));
        let left = st.focused_pane().expect("ペインがある");
        let right = split_right(&mut st);
        focus(&mut st, left);
        let w = right.0;
        let f = crate::layout::build_frame(&st, w, &[]).expect("Frame");
        assert_eq!(f.panes.len(), 2);
        assert_eq!(f.panes[0].view, va);
        assert!(f.panes[0].focused && !f.panes[1].focused);
        assert_eq!((f.panes[1].rect.col, f.panes[1].rect.cols), (41, 40));
        let left_cursor = f.primary_cursor.expect("カーソル").pos;
        focus(&mut st, right);
        let f = crate::layout::build_frame(&st, w, &[]).expect("Frame");
        assert!(!f.panes[0].focused && f.panes[1].focused);
        assert_eq!(
            f.primary_cursor.map(|c| c.pos),
            Some(CellPos {
                col: left_cursor.col + 41,
                row: left_cursor.row,
            }),
            "右のペインの主カーソルは、ペインの中の位置に矩形の左上を足す"
        );
    }

    #[test]
    fn 表示していない_view_は表に持つペイン_ペインに属さない_view_はフォーカスのペインの大きさ() {
        let (mut st, a, _) = state(
            "a
",
        );
        let (b, vb) = st.add_document(doc("b
"));
        let left = st.focused_pane().expect("ペインがある");
        let right = split_right(&mut st);
        // どのペインにも属さない View: フォーカスのペイン (右) の大きさ
        assert_eq!(st.pane_owning(vb), None);
        assert_eq!(st.view_size(vb), (40, 23));
        // 右で b を出してから a に戻す: vb は右の表に残り (表示していない)、右のペインの大きさ
        st.activate_document(b);
        st.activate_document(a);
        assert_eq!(st.pane_owning(vb), Some(right));
        assert_eq!(st.view_rect(vb), None);
        assert_eq!(st.view_size(vb), (40, 23));
        // 1 回で求めた全 View の大きさは、View ごとの大きさと同じ
        let all = st.all_view_sizes();
        for (v, size) in &all {
            assert_eq!(st.view_size(*v), *size, "{v:?}");
        }
        assert_eq!(all.len(), st.docs.views.len());
        // 右を閉じる: a の右の View は消え (左にある)、b の View は唯一なのでペインに属さない View として残る
        assert_eq!(st.close_pane(), crate::command::CommandResult::Done);
        assert_eq!(st.focused_pane(), Some(left));
        assert!(st.view(vb).is_some(), "唯一の View は残す");
        assert_eq!(st.pane_owning(vb), None);
        assert_eq!(st.view_size(vb), (81, 24));
        assert_eq!(st.docs.views.len(), 2);
        st.check_panes().expect("不変条件");
    }

    #[test]
    fn 縮退表示ではフォーカスのペインが編集面の全体で_戻すと元の大きさ() {
        let (mut st, _, va) = state(
            "a
",
        );
        let right = split_right(&mut st);
        let vr = st.pane(right).map(|p| p.view).expect("右のペイン");
        st.set_viewport(30, 24);
        assert_eq!(st.view_size(vr), (30, 24));
        assert_eq!(st.view_size(va), (30, 24), "隠れているペインの View も全体");
        assert_eq!(st.collapsed_position(), Some((2, 2)));
        assert_eq!(
            st.shown_message_key(),
            Some(crate::ui_model::keys::INFO_PANE_COLLAPSED)
        );
        st.check_panes().expect("不変条件");
        st.set_viewport(81, 24);
        assert_eq!(st.view_size(vr), (40, 23));
        assert_eq!(st.collapsed_position(), None);
        st.check_panes().expect("不変条件");
    }

    #[test]
    fn 写して作った_view_は表示するペインの矩形でカーソルが見えるようにする() {
        // 100 行の文書の 90 行目にカーソル。下に分けた 11 行のペインに写すと、写した直後に見える位置へスクロールする
        let text: String = (0..100)
            .map(|i| {
                format!(
                    "{i}
"
                )
            })
            .collect();
        let (mut st, a, va) = state(&text);
        let at = st.document(a).map(|d| {
            use eve_text::TextRead as _;
            d.text().line_to_byte(eve_text::LineIdx(90))
        });
        st.set_selection(va, Selection::point(at.expect("行")));
        st.ensure_cursor_visible(va);
        let (b, _) = st.add_document(doc("b
"));
        // 左を b にしておき、右に分けてから右で a を表示する (最後にフォーカスのあった va を写す)
        st.activate_document(b);
        let right = split_right(&mut st);
        st.set_viewport(81, 12);
        st.activate_document(a);
        let vr = st.pane(right).map(|p| p.view).expect("右のペイン");
        assert_ne!(vr, va);
        let v = st.view(vr).expect("View");
        let top = st.document(a).map(|d| {
            use eve_text::TextRead as _;
            d.text().byte_to_line(v.scroll.top).0
        });
        let rows = usize::from(st.view_size(vr).1);
        let top = top.expect("文書");
        assert!(top <= 90 && 90 < top + rows, "top {top}、行数 {rows}");
    }
}
