//! マウス (docs/03 §5.8.1、01 FR-307): 左クリックで移動、Shift+クリックで拡張、ドラッグで選択、ホイールでスクロール。
//!
//! 位置は面の左上を原点とするセルで届く。折り返し・タブ・全角を考慮した文書位置の確定は、ここでスクロール位置とレイアウトから直接引く。

use eve_text::{ByteOff, LineIdx, Range, Selection, TextRead};
use eve_view::{Modifiers, MouseAt, MouseButton, MouseEvent, MouseKind, ViewId};

use super::layout::{Affinity, line_rows, wrap_ctx};
use super::state::EditorState;
use crate::time::{Millis, Tick};

/// ホイール 1 目盛りのスクロール量 (表示行。`ui.mouse_wheel_lines` の既定、04 §6.8)。
const WHEEL_LINES: i64 = 3;
/// Shift+ホイールの横スクロール量 (列)。
const WHEEL_COLUMNS: i64 = 4;
/// ドラッグが面の外にあるとき、1 表示行スクロールする間隔。
const AUTO_SCROLL_INTERVAL: Millis = Millis(50);

/// 左ボタンのドラッグ中の状態。
#[derive(Clone, Copy, Debug)]
pub(super) struct Drag {
    view: ViewId,
    /// 選択の起点 (動かさない側)。
    anchor: ByteOff,
    /// 最後に届いたマウスの位置。
    at: MouseAt,
    /// 次の自動スクロールの時刻 (面の外にいる間だけ)。
    next_scroll: Option<Tick>,
}

impl EditorState {
    pub(super) fn on_mouse(&mut self, m: MouseEvent) {
        // オーバーレイ・検索バーが開いている間は、編集面のマウスを無視する
        if self.overlay.is_some()
            || self.search_bar.is_some()
            || !self.views.contains_key(&m.at.view)
        {
            self.mouse_drag = None;
            return;
        }
        match m.kind {
            // v0.2 は修飾なしと Shift だけ (Ctrl / Alt 付きは後の版)
            MouseKind::Down(MouseButton::Left)
                if m.mods.is_empty() || m.mods == Modifiers::SHIFT =>
            {
                self.mouse_down(m);
            }
            MouseKind::Drag(MouseButton::Left) => self.mouse_move(m.at),
            MouseKind::Up(MouseButton::Left) => self.mouse_drag = None,
            MouseKind::WheelUp => self.mouse_wheel(m, -1),
            MouseKind::WheelDown => self.mouse_wheel(m, 1),
            _ => {}
        }
    }

    fn mouse_down(&mut self, m: MouseEvent) {
        let view = m.at.view;
        let Some((pos, aff)) = self.mouse_position(m.at) else {
            return;
        };
        // Undo の結合はカーソル移動と同じく封印する
        self.seal_active_history();
        let anchor = match self.views.get(&view) {
            Some(v) if m.mods.contains(Modifiers::SHIFT) => v.selection.primary().anchor,
            _ => pos,
        };
        self.mouse_select(view, anchor, pos, aff);
        self.mouse_drag = Some(Drag {
            view,
            anchor,
            at: m.at,
            next_scroll: None,
        });
    }

    fn mouse_move(&mut self, at: MouseAt) {
        let Some(mut drag) = self.mouse_drag else {
            return;
        };
        drag.at = at;
        let rows = i32::from(self.viewport().1);
        if at.row < 0 || at.row >= rows {
            drag.next_scroll = drag.next_scroll.or(Some(self.now + AUTO_SCROLL_INTERVAL));
        } else {
            drag.next_scroll = None;
        }
        self.mouse_drag = Some(drag);
        self.drag_select(drag);
    }

    /// 起点からマウスの位置までを選ぶ (面の外の行は端の行に丸める)。
    fn drag_select(&mut self, drag: Drag) {
        if let Some((pos, aff)) = self.mouse_position(drag.at) {
            self.mouse_select(drag.view, drag.anchor, pos, aff);
        }
    }

    fn mouse_select(&mut self, view: ViewId, anchor: ByteOff, head: ByteOff, aff: Affinity) {
        let sel = Selection::single(Range::new(anchor, head));
        self.set_selection(view, sel);
        self.set_cursor_affinity(view, aff);
    }

    /// 面の外へドラッグしている間、時間が来たら 1 表示行スクロールして選択を伸ばす。
    pub(super) fn mouse_tick(&mut self) {
        let Some(mut drag) = self.mouse_drag else {
            return;
        };
        let Some(due) = drag.next_scroll else {
            return;
        };
        if self.now < due {
            return;
        }
        let up = drag.at.row < 0;
        self.scroll_lines(drag.view, if up { -1 } else { 1 });
        drag.next_scroll = Some(self.now + AUTO_SCROLL_INTERVAL);
        self.mouse_drag = Some(drag);
        self.drag_select(drag);
    }

    pub(super) fn mouse_deadline(&self) -> Option<Tick> {
        self.mouse_drag.and_then(|d| d.next_scroll)
    }

    fn mouse_wheel(&mut self, m: MouseEvent, dir: i64) {
        let view = m.at.view;
        if m.mods.contains(Modifiers::SHIFT) {
            if wrap_ctx(self, view).is_none() {
                self.scroll_columns(view, dir * WHEEL_COLUMNS);
            }
        } else {
            self.scroll_lines(view, dir * WHEEL_LINES);
        }
    }

    /// 水平スクロール (折り返し OFF)。いちばん長い可視行の幅を超えて右へは動かさない。
    fn scroll_columns(&mut self, view: ViewId, delta: i64) {
        let rows = self.viewport().1 as usize;
        let Some(v) = self.views.get(&view) else {
            return;
        };
        let Some(d) = self.documents.get(&v.doc) else {
            return;
        };
        let t = d.text();
        let top = t.byte_to_line(v.scroll.top).0;
        let last = t.len_lines().saturating_sub(1);
        let widest = (top..=(top + rows).min(last))
            .map(|l| super::layout::cursor_cell(d, l, usize::MAX).0)
            .max()
            .unwrap_or(0);
        let left = i64::from(v.scroll.left_col);
        let new = (left + delta).clamp(0, widest.saturating_sub(1) as i64);
        if new != left
            && let Some(v) = self.views.get_mut(&view)
        {
            v.scroll.left_col = new as u32;
            self.mark_dirty_render();
        }
    }

    /// 面のセルに対応する文書の位置 (と折り返し位置の帰属)。03 §5.8.1 の規則。面の外の行は端の行に丸める。
    fn mouse_position(&self, at: MouseAt) -> Option<(ByteOff, Affinity)> {
        let v = self.views.get(&at.view)?;
        let d = self.documents.get(&v.doc)?;
        let (cols, rows) = self.viewport();
        let gw = i32::from(super::layout::gutter_width(d, cols as usize));
        let row = at.row.clamp(0, (i32::from(rows) - 1).max(0)) as usize;
        let text_col = (at.col - gw).max(0) as usize;
        let t = d.text();
        let n_lines = t.len_lines();
        if let Some((s, width)) = wrap_ctx(self, at.view) {
            let Some((line, r)) = self.wrapped_row_at(at.view, row) else {
                return Some((ByteOff(t.len_bytes()), Affinity::After));
            };
            let lr = line_rows(d, line, s, width);
            let (rel, aff) = lr.at_col(r, text_col);
            return Some((ByteOff(lr.ls + rel), aff));
        }
        let line = t.byte_to_line(v.scroll.top).0 + row;
        if line >= n_lines {
            return Some((ByteOff(t.len_bytes()), Affinity::After));
        }
        let ls = t.line_to_byte(LineIdx(line)).0;
        let col = v.scroll.left_col as usize + text_col;
        Some((
            ByteOff(ls + super::layout::byte_at_col(d, line, col)),
            Affinity::After,
        ))
    }
}
