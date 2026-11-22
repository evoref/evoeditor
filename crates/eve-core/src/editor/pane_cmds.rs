//! ペインの操作とコマンド (docs/03 §9.9 の PANE-4〜PANE-6、07 §11.6 の `pane.*`): 分割・フォーカスの移動・閉じる・
//! 大きさ・揃える、縮退表示の出入り。木の形の操作は `pane_tree`、矩形の計算は `layout::panes`。
//!
//! v0.5 の M30 の段 2a ではパレットと `execute` からだけ実行する (既定キーは段 2b)。
//!
//! spec: 03 §9.9 (PANE-4, PANE-5, PANE-6), 07 §11.6

use std::collections::BTreeMap;

use eve_text::TextRead;
use eve_view::{CellRect, MessageLevel, ViewId, WindowId};

use crate::command::ids;
use crate::command::{
    CommandCtx, CommandKind, CommandRegistry, CommandResult, Permission, RejectReason,
};
use crate::editor::pane_tree::ratios_from_cells;
use crate::editor::state::EditorState;
use crate::editor::window::{Axis, LayoutNode, Pane, PaneId, PaneRef};
use crate::layout::panes::{LeafRect, PaneLayout, min_size};
use crate::ui_model::keys;

/// 方向の移動の向き (PANE-5 (c))。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl EditorState {
    /// View `view` を表示中のペイン (PANE-7 (g)。境界の入力はペインを表示中の `ViewId` で指す)。どのペインも表示して
    /// いなければ `None`。
    pub(crate) fn pane_of_shown_view(&self, view: ViewId) -> Option<PaneRef> {
        self.docs
            .windows
            .iter()
            .find_map(|w| w.pane_showing(view).map(|p| (w.id, p)))
    }

    /// フォーカスをペイン `at` に移す (PANE-6 (a)): 前のペインの文書の Undo の結合を区切り (03 §7.3 の封印)、検索バーと
    /// 進行中の検索を閉じ、`pane_recent` と文書ごとの最後にフォーカスのあった View を記録する。縮退表示では、移した
    /// ペインが編集面の全体に出るので、カーソルが見えるようにスクロールする (PANE-4 (b))。
    pub(crate) fn focus_pane(&mut self, at: PaneRef) {
        if self.focused_pane() == Some(at)
            || self
                .window(at.0)
                .is_none_or(|w| !w.panes.contains_key(&at.1))
        {
            return;
        }
        self.seal_active_history();
        self.search_close();
        let size = self.surface_size();
        // 移す先のペインの View の、移す前の本文の桁数 (縮退表示で表示が替わったときに合わせる)
        let before_cols = self
            .window(at.0)
            .and_then(|w| w.panes.get(&at.1))
            .map(|p| self.view_size(p.view).0);
        let Some(w) = self.window_mut(at.0) else {
            return;
        };
        let collapsed = w.is_collapsed(size);
        w.focus = at.1;
        w.pane_recent.retain(|p| *p != at.1);
        w.pane_recent.push(at.1);
        let Some(view) = w.panes.get(&at.1).map(|p| p.view) else {
            return;
        };
        if let Some(doc) = self.view(view).map(|v| v.doc) {
            self.docs.last_view.insert(doc, view);
        }
        if collapsed {
            self.fit_view(view, before_cols);
        }
        self.mark_dirty_render();
    }

    /// 可視の大きさが変わった View を合わせる: 本文の桁数 (折り返し幅) が `before_cols` から変わったときだけ折り返しの
    /// 表示行の番号を 0 に戻し (`apply_viewport` と同じ規則、03 §14.3.1)、カーソルが見えるようにスクロールする。
    /// `before_cols` が `None` (前の大きさが分からない) なら戻す。
    fn fit_view(&mut self, view: ViewId, before_cols: Option<u16>) {
        if before_cols != Some(self.view_size(view).0)
            && let Some(v) = self.docs.views.get_mut(&view)
        {
            v.scroll.wrap_idx = 0;
        }
        self.ensure_cursor_visible(view);
    }

    /// Window `w` の木を変えた後: 本文の大きさが変わったペイン (と新しく出たペイン) の表の View (表示していないものを
    /// 含む) を合わせ、縮退表示の出入りを見る。`before` は変える前の配置。
    fn after_relayout(&mut self, w: WindowId, before: &PaneLayout) {
        let size = self.surface_size();
        let Some(win) = self.window(w) else {
            return;
        };
        // (View, 変える前のペインの本文の桁数。新しく出たペインは `None`)
        let changed: Vec<(ViewId, Option<u16>)> = win
            .layout(size)
            .leaves
            .iter()
            .filter_map(|x| {
                let prev = before.leaves.iter().find(|b| b.pane == x.pane);
                prev.is_none_or(|b| (b.body.cols, b.body.rows) != (x.body.cols, x.body.rows))
                    .then(|| (x.pane, prev.map(|b| b.body.cols)))
            })
            // 表示中の View だけでなく、そのペインの表に残っている (表示していない) View も合わせる (次にそのペインで
            // 表示するときの大きさが変わった)
            .filter_map(|(pane, cols)| win.panes.get(&pane).map(|p| (p, cols)))
            .flat_map(|(p, cols)| p.views.values().map(move |v| (*v, cols)))
            .collect();
        self.with_view_sizes(|st| {
            for (v, cols) in changed {
                st.fit_view(v, cols);
            }
        });
        self.refresh_collapse();
        self.mark_dirty_render();
    }

    /// 縮退表示 (PANE-4 (b)) に入ったら 1 回だけ `info.pane.collapsed` を出し、今の状態を覚える。
    pub(crate) fn refresh_collapse(&mut self) {
        let size = self.surface_size();
        let mut entered = false;
        for w in &mut self.docs.windows {
            let now = w.is_collapsed(size);
            entered |= now && !w.collapsed;
            w.collapsed = now;
        }
        if entered {
            self.show_message(MessageLevel::Info, keys::INFO_PANE_COLLAPSED, &[]);
        }
    }

    /// 縮退表示の間の、フォーカスのペインの葉の順での番号 (1 始まり) とペインの数 (ステータスバーの機能縮退の項目、
    /// 04 §8.7)。縮退表示でなければ `None`。
    pub(crate) fn collapsed_position(&self) -> Option<(usize, usize)> {
        let w = self.window(self.focused_window()?)?;
        if !w.is_collapsed(self.surface_size()) {
            return None;
        }
        let leaves = w.leaves();
        let i = leaves.iter().position(|p| *p == w.focus)?;
        Some((i + 1, leaves.len()))
    }

    /// `pane.split_right` / `pane.split_down` (PANE-5 (a))。分割した後の木の最小が編集面に入らなければ分割せず、
    /// `info.pane.no_room` を出す (PANE-4 (a))。
    pub(crate) fn split_pane(&mut self, axis: Axis) -> CommandResult {
        let Some((w, p)) = self.focused_pane() else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let size = self.surface_size();
        let new = PaneId(self.docs.next_pane);
        let Some(win) = self.window(w) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(root) = win.root.with_split(p, axis, new) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let min = min_size(&root);
        if min.cols > size.cols || min.rows > size.rows {
            self.show_message(MessageLevel::Info, keys::INFO_PANE_NO_ROOM, &[]);
            return CommandResult::Done;
        }
        let Some((src, recent)) = win.panes.get(&p).map(|x| (x.view, x.recent.clone())) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let before = win.layout(size);
        let Some(doc) = self.view(src).map(|v| v.doc) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        // 新しいペインは、元のペインと同じ文書を、元のペインの View を写した View で出す (PANE-1)
        let Some(view) = self.copy_view(src) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        self.docs.next_pane += 1;
        let Some(win) = self.window_mut(w) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        win.root = root;
        win.panes.insert(
            new,
            Pane {
                view,
                views: BTreeMap::from([(doc, view)]),
                recent,
            },
        );
        self.focus_pane((w, new));
        self.after_relayout(w, &before);
        CommandResult::Done
    }

    /// `pane.close` (PANE-5 (d)): フォーカスのペインを閉じ、その表の View を消す (その文書のほかの View が無ければ、
    /// ペインに属さない View として残す、PANE-1)。文書は閉じない。フォーカスは残ったペインのうち最後にフォーカスの
    /// あったものに移る。最後のペインは閉じない。
    pub(crate) fn close_pane(&mut self) -> CommandResult {
        let Some((w, p)) = self.focused_pane() else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(root) = self.window(w).and_then(|win| win.root.without_leaf(p)) else {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        };
        // 閉じるペインの文書の Undo の結合を区切り、検索バーを閉じる (フォーカスが替わる、PANE-6 (a))
        self.seal_active_history();
        self.search_close();
        let size = self.surface_size();
        let Some(win) = self.window_mut(w) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let before = win.layout(size);
        let Some(pane) = win.panes.remove(&p) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        win.root = root;
        win.pane_recent.retain(|x| *x != p);
        let leaves = win.leaves();
        let next = win
            .pane_recent
            .iter()
            .rev()
            .copied()
            .find(|x| win.panes.contains_key(x))
            .or_else(|| leaves.first().copied());
        let Some(next) = next else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        win.focus = next;
        win.pane_recent.retain(|x| *x != next);
        win.pane_recent.push(next);
        let next_view = win.panes.get(&next).map(|x| x.view);
        for (doc, view) in pane.views {
            let others = self
                .docs
                .views
                .iter()
                .any(|(id, v)| *id != view && v.doc == doc);
            if others {
                self.docs.views.remove(&view);
                self.docs.last_view.retain(|_, v| *v != view);
            }
        }
        self.term.mouse_drag = None;
        if let Some(view) = next_view
            && let Some(doc) = self.view(view).map(|v| v.doc)
        {
            self.docs.last_view.insert(doc, view);
        }
        self.after_relayout(w, &before);
        CommandResult::Done
    }

    /// `pane.focus_next` / `pane.focus_prev` (PANE-5 (b)): 木の葉の順で次 / 前へ。端から反対の端へ巡る。
    fn cycle_focus(&mut self, forward: bool) -> CommandResult {
        let Some((w, p)) = self.focused_pane() else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let leaves = self.window(w).map(|x| x.leaves()).unwrap_or_default();
        let n = leaves.len();
        let Some(i) = leaves.iter().position(|x| *x == p).filter(|_| n >= 2) else {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        };
        let next = if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        };
        self.focus_pane((w, leaves[next]));
        CommandResult::Done
    }

    /// `pane.focus_left` 等 (PANE-5 (c)): その向きで隣り合うペインのうち、フォーカスのペインの主カーソルの画面の行
    /// (左右) ・列 (上下) を含むもの、無ければ最も近いもの (同じ距離なら上・左)。隣が無ければ何もしない (巡らない)。
    /// 縮退表示では、木の最小の大きさで計算した配置で決める (PANE-4 (b))。
    fn focus_toward(&mut self, dir: Direction) -> CommandResult {
        let Some((w, p)) = self.focused_pane() else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let size = self.surface_size();
        let Some(win) = self.window(w) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let l = win.logical_layout(size);
        let (Some(me), Some(view)) = (
            l.leaves.iter().find(|x| x.pane == p).copied(),
            win.panes.get(&p).map(|x| x.view),
        ) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let (row, col) = self.cursor_cell_in(view, me.body);
        let key = match dir {
            Direction::Left | Direction::Right => row,
            Direction::Up | Direction::Down => col,
        };
        let target = pick_neighbor(&l, &me, dir, key);
        if let Some(t) = target {
            self.focus_pane((w, t));
        }
        CommandResult::Done
    }

    /// View `view` の主カーソルの画面のセル (行, 列。編集面の左上が原点)。`rect` はその View を出すペインの本文の
    /// 矩形。主カーソルが画面の外にあるときは、その行・列を本文の範囲に寄せる (PANE-5 (c))。
    fn cursor_cell_in(&self, view: ViewId, rect: CellRect) -> (u16, u16) {
        let last_row = rect.rows.saturating_sub(1);
        let last_col = rect.cols.saturating_sub(1);
        let (r, c) = match crate::layout::primary_cursor_in(self, view, rect) {
            Some(pos) => (pos.row.min(last_row), pos.col.min(last_col)),
            None => self.offscreen_cursor_cell(view, rect),
        };
        (rect.row.saturating_add(r), rect.col.saturating_add(c))
    }

    /// 画面の外にある主カーソルの、本文の範囲に寄せた (行, 列) (`rect` の左上が原点)。
    fn offscreen_cursor_cell(&self, view: ViewId, rect: CellRect) -> (u16, u16) {
        let last_row = rect.rows.saturating_sub(1);
        let last_col = rect.cols.saturating_sub(1);
        let Some(v) = self.view(view) else {
            return (0, 0);
        };
        let Some(d) = self.document(v.doc) else {
            return (0, 0);
        };
        let t = d.text();
        let line = t.byte_to_line(v.selection.primary().head).0;
        let top = t.byte_to_line(v.scroll.top).0;
        let row = u16::try_from(line.saturating_sub(top))
            .unwrap_or(u16::MAX)
            .min(last_row);
        let gw = usize::from(crate::layout::gutter_width(d, usize::from(rect.cols)));
        let dcol = crate::layout::primary_cursor_cell(self, view).map_or(0, |(c, _)| c);
        let col = u16::try_from(gw + dcol.saturating_sub(v.scroll.left_col as usize))
            .unwrap_or(u16::MAX)
            .min(last_col);
        (row, col)
    }

    /// `pane.grow` / `pane.shrink` (PANE-5 (e)): フォーカスのペインを、親の分割の向きに `cells` (既定: 親の矩形のその
    /// 向きの長さの 1/10、少なくとも 1) だけ広げる / 狭める。差は次の兄弟 (末尾なら前の兄弟) から取る / に返し、どちらの
    /// ペインも最小を割らない所で止める。比率はセルの大きさから計算し直す。
    fn resize_pane(&mut self, grow: bool, cells: Option<u16>) -> CommandResult {
        let Some((w, p)) = self.focused_pane() else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let size = self.surface_size();
        let Some(win) = self.window(w) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some((axis, children, i)) = win.root.parent_of(p) else {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        };
        let l = win.logical_layout(size);
        let mut ext: Vec<u16> = children.iter().map(|(c, _)| extent(c, &l, axis)).collect();
        let mins: Vec<u16> = children
            .iter()
            .map(|(c, _)| {
                let m = min_size(c);
                match axis {
                    Axis::Row => m.cols,
                    Axis::Column => m.rows,
                }
            })
            .collect();
        let n = ext.len();
        let gaps = match axis {
            Axis::Row => u16::try_from(n.saturating_sub(1)).unwrap_or(u16::MAX),
            Axis::Column => 0,
        };
        let parent_len = ext.iter().fold(gaps, |acc, &e| acc.saturating_add(e));
        let step = cells.unwrap_or((parent_len / 10).max(1));
        let j = if i + 1 < n {
            i + 1
        } else {
            i.saturating_sub(1)
        };
        let (from, to) = if grow { (j, i) } else { (i, j) };
        let d = step.min(ext[from].saturating_sub(mins[from]));
        if d == 0 || from == to {
            return CommandResult::Done;
        }
        ext[from] -= d;
        ext[to] = ext[to].saturating_add(d);
        let ratios = ratios_from_cells(&ext);
        let before = win.layout(size);
        let Some(win) = self.window_mut(w) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        if let Some((_, children, _)) = win.root.parent_of_mut(p) {
            for ((_, r), new) in children.iter_mut().zip(ratios) {
                *r = new;
            }
        }
        self.after_relayout(w, &before);
        CommandResult::Done
    }

    /// `pane.equalize` (PANE-5 (f))。
    fn equalize_panes(&mut self) -> CommandResult {
        let Some((w, _)) = self.focused_pane() else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let size = self.surface_size();
        let Some(win) = self.window_mut(w) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let before = win.layout(size);
        win.root.equalize();
        self.after_relayout(w, &before);
        CommandResult::Done
    }
}

/// 節 `node` が配置 `l` で占める、向き `axis` の長さ (葉の本文と見出しの矩形を合わせたもの)。
fn extent(node: &LayoutNode, l: &PaneLayout, axis: Axis) -> u16 {
    let leaves = node.leaves();
    let spans = l
        .leaves
        .iter()
        .filter(|x| leaves.contains(&x.pane))
        .map(|x| match axis {
            Axis::Row => (x.body.col, x.body.col.saturating_add(x.body.cols)),
            Axis::Column => (x.body.row, outer(x).row.saturating_add(outer(x).rows)),
        });
    let (lo, hi) = spans.fold((u16::MAX, 0u16), |(lo, hi), (a, b)| (lo.min(a), hi.max(b)));
    hi.saturating_sub(lo)
}

/// 葉の本文と見出しを合わせた矩形。
fn outer(x: &LeafRect) -> CellRect {
    let bar = x.bar.map_or(0, |b| b.rows);
    CellRect {
        rows: x.body.rows.saturating_add(bar),
        ..x.body
    }
}

/// 半開区間 `[a0, a1)` と `[b0, b1)` が重なる。
fn overlaps(a: (u16, u16), b: (u16, u16)) -> bool {
    a.0 < b.1 && b.0 < a.1
}

/// 方向の移動の行き先 (PANE-5 (c))。`me` はフォーカスのペイン、`key` は主カーソルの画面の行 (左右) ・列 (上下)。
fn pick_neighbor(l: &PaneLayout, me: &LeafRect, dir: Direction, key: u16) -> Option<PaneId> {
    let m = outer(me);
    let rows = |r: &CellRect| (r.row, r.row.saturating_add(r.rows));
    let cols = |r: &CellRect| (r.col, r.col.saturating_add(r.cols));
    let candidates: Vec<(PaneId, (u16, u16))> = l
        .leaves
        .iter()
        .filter(|x| x.pane != me.pane)
        .filter_map(|x| {
            let o = outer(x);
            // 区切り (左右) か見出し (上下) を挟んで接し、接する辺が重なるもの
            let touches = match dir {
                Direction::Left => {
                    u32::from(o.col) + u32::from(o.cols) + 1 == u32::from(m.col)
                        && overlaps(rows(&o), rows(&m))
                }
                Direction::Right => {
                    u32::from(m.col) + u32::from(m.cols) + 1 == u32::from(o.col)
                        && overlaps(rows(&o), rows(&m))
                }
                Direction::Up => {
                    u32::from(o.row) + u32::from(o.rows) == u32::from(m.row)
                        && overlaps(cols(&o), cols(&m))
                }
                Direction::Down => {
                    u32::from(m.row) + u32::from(m.rows) == u32::from(o.row)
                        && overlaps(cols(&o), cols(&m))
                }
            };
            let span = match dir {
                Direction::Left | Direction::Right => rows(&o),
                Direction::Up | Direction::Down => cols(&o),
            };
            touches.then_some((x.pane, span))
        })
        .collect();
    let distance = |(a, b): (u16, u16)| {
        if key < a {
            a - key
        } else if key >= b {
            key - b.saturating_sub(1)
        } else {
            0
        }
    };
    candidates
        .into_iter()
        .min_by_key(|(_, span)| (distance(*span), span.0))
        .map(|(p, _)| p)
}

/// 引数 `cells` (省略可。1 以上の整数)。
fn cells_arg(ctx: &CommandCtx<'_>) -> Result<Option<u16>, CommandResult> {
    match ctx.args.get("cells") {
        None => Ok(None),
        Some(_) => match ctx.args.int("cells") {
            Some(n) if n >= 1 => Ok(Some(u16::try_from(n).unwrap_or(u16::MAX))),
            _ => Err(CommandResult::Rejected(RejectReason::InvalidArgs)),
        },
    }
}

fn split_right(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.split_pane(Axis::Row)
}

fn split_down(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.split_pane(Axis::Column)
}

fn close(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.close_pane()
}

fn focus_next(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.cycle_focus(true)
}

fn focus_prev(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.cycle_focus(false)
}

fn focus_left(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.focus_toward(Direction::Left)
}

fn focus_right(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.focus_toward(Direction::Right)
}

fn focus_up(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.focus_toward(Direction::Up)
}

fn focus_down(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.focus_toward(Direction::Down)
}

fn grow(ctx: &mut CommandCtx<'_>) -> CommandResult {
    match cells_arg(ctx) {
        Ok(cells) => ctx.state.resize_pane(true, cells),
        Err(r) => r,
    }
}

fn shrink(ctx: &mut CommandCtx<'_>) -> CommandResult {
    match cells_arg(ctx) {
        Ok(cells) => ctx.state.resize_pane(false, cells),
        Err(r) => r,
    }
}

fn equalize(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.equalize_panes()
}

/// v0.5 のペインのコマンド (07 §11.6 の「ペイン」の表)。`when` の `pane.split` は Window のペインが 2 つ以上 (07 §7.3)。
pub(crate) fn register(r: &mut CommandRegistry) {
    let all: [(&str, &str, crate::command::Handler); 12] = [
        (ids::PANE_CLOSE, "pane.split", close),
        (ids::PANE_EQUALIZE, "pane.split", equalize),
        (ids::PANE_FOCUS_DOWN, "pane.split", focus_down),
        (ids::PANE_FOCUS_LEFT, "pane.split", focus_left),
        (ids::PANE_FOCUS_NEXT, "pane.split", focus_next),
        (ids::PANE_FOCUS_PREV, "pane.split", focus_prev),
        (ids::PANE_FOCUS_RIGHT, "pane.split", focus_right),
        (ids::PANE_FOCUS_UP, "pane.split", focus_up),
        (ids::PANE_GROW, "pane.split", grow),
        (ids::PANE_SHRINK, "pane.split", shrink),
        (ids::PANE_SPLIT_DOWN, "", split_down),
        (ids::PANE_SPLIT_RIGHT, "", split_right),
    ];
    for (id, when, handler) in all {
        r.add(id, CommandKind::Ui, Permission::Ui, when, handler);
    }
}

#[cfg(test)]
mod tests {
    use eve_view::CellSize;

    use super::*;
    use crate::layout::panes::layout;

    fn leaf(n: u64) -> LayoutNode {
        LayoutNode::Leaf(PaneId(n))
    }

    fn split(axis: Axis, children: Vec<(LayoutNode, u16)>) -> LayoutNode {
        LayoutNode::Split { axis, children }
    }

    /// 左に 1、右を上下に 2 / 3 で分けた木を 61 × 20 に置く (1 は 0..30 列、2 は 31.. 列の 0..10 行、3 は 10..20 行)。
    fn three() -> PaneLayout {
        layout(
            &split(
                Axis::Row,
                vec![
                    (leaf(1), 5_000),
                    (
                        split(Axis::Column, vec![(leaf(2), 5_000), (leaf(3), 5_000)]),
                        5_000,
                    ),
                ],
            ),
            CellSize { cols: 61, rows: 20 },
        )
    }

    fn at(l: &PaneLayout, p: u64) -> LeafRect {
        *l.leaves
            .iter()
            .find(|x| x.pane == PaneId(p))
            .expect("葉がある")
    }

    #[test]
    fn 方向の移動は主カーソルの行を含む隣へ_無ければ最も近い隣へ() {
        let l = three();
        // 1 から右: カーソルの行が上半分なら 2、下半分なら 3
        assert_eq!(
            pick_neighbor(&l, &at(&l, 1), Direction::Right, 3),
            Some(PaneId(2))
        );
        assert_eq!(
            pick_neighbor(&l, &at(&l, 1), Direction::Right, 15),
            Some(PaneId(3))
        );
        // 2 から左・下、3 から上
        assert_eq!(
            pick_neighbor(&l, &at(&l, 2), Direction::Left, 3),
            Some(PaneId(1))
        );
        assert_eq!(
            pick_neighbor(&l, &at(&l, 2), Direction::Down, 40),
            Some(PaneId(3))
        );
        assert_eq!(
            pick_neighbor(&l, &at(&l, 3), Direction::Up, 40),
            Some(PaneId(2))
        );
        // 隣が無い向きは巡らない
        assert_eq!(pick_neighbor(&l, &at(&l, 1), Direction::Left, 3), None);
        assert_eq!(pick_neighbor(&l, &at(&l, 1), Direction::Up, 3), None);
        assert_eq!(pick_neighbor(&l, &at(&l, 2), Direction::Right, 3), None);
    }

    #[test]
    fn 含む隣が無ければ近いほう_同じ距離なら上() {
        // 上下に 1 / 2 を並べた列の右に 3 / 4 (行の境目がずれる) を並べ、間の行から右へ
        let t = split(
            Axis::Row,
            vec![
                (leaf(1), 5_000),
                (
                    split(Axis::Column, vec![(leaf(3), 5_000), (leaf(4), 5_000)]),
                    5_000,
                ),
            ],
        );
        let l = layout(&t, CellSize { cols: 61, rows: 20 });
        // 区間の外の行 (編集面の外) は最も近い下のペイン
        assert_eq!(
            pick_neighbor(&l, &at(&l, 1), Direction::Right, 30),
            Some(PaneId(4))
        );
    }

    #[test]
    fn 節の長さは葉の本文と見出しを合わせる() {
        let t = split(
            Axis::Row,
            vec![
                (leaf(1), 5_000),
                (
                    split(Axis::Column, vec![(leaf(2), 5_000), (leaf(3), 5_000)]),
                    5_000,
                ),
            ],
        );
        let l = layout(&t, CellSize { cols: 61, rows: 20 });
        let LayoutNode::Split { children, .. } = &t else {
            unreachable!("分割")
        };
        assert_eq!(extent(&children[0].0, &l, Axis::Row), 30);
        assert_eq!(extent(&children[1].0, &l, Axis::Row), 30);
        assert_eq!(extent(&children[1].0, &l, Axis::Column), 20);
        assert_eq!(extent(&leaf(2), &l, Axis::Column), 10);
    }
}
