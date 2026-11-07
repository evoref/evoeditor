//! マウス (docs/03 §5.8.1、01 FR-307): 左クリックで移動、Shift+クリックで拡張、ドラッグで選択、ホイールでスクロール。
//!
//! 位置は面の左上を原点とするセルで届く。折り返し・タブ・全角を考慮した文書位置の確定は、ここでスクロール位置とレイアウトから直接引く。

use eve_text::{ByteOff, LineIdx, Range, Selection, TextRead};
use eve_view::{
    Effect, MessageLevel, Modifiers, MouseAt, MouseButton, MouseEvent, MouseKind, ViewId,
};

use crate::command::ids;
use crate::command::{CommandCtx, CommandKind, CommandRegistry, CommandResult, Permission};
use crate::editor::state::EditorState;
use crate::layout::{Affinity, line_rows, wrap_ctx};
use crate::time::{Millis, Tick};
use crate::ui_model::keys;

/// Shift+ホイールの横スクロール量 (列)。
const WHEEL_COLUMNS: i64 = 4;
/// ドラッグが面の外にあるとき、1 表示行スクロールする間隔。
const AUTO_SCROLL_INTERVAL: Millis = Millis(50);

/// 左ボタンのドラッグ中の状態。
#[derive(Clone, Copy, Debug)]
pub(crate) struct Drag {
    view: ViewId,
    /// 選択の起点 (動かさない側)。
    anchor: ByteOff,
    /// 最後に届いたマウスの位置。
    at: MouseAt,
    /// 次の自動スクロールの時刻 (面の外にいる間だけ)。
    next_scroll: Option<Tick>,
}

impl EditorState {
    pub(crate) fn on_mouse(&mut self, m: MouseEvent) {
        // オーバーレイ・検索バーが開いている間は、編集面のマウスを無視する
        if self.ui.overlay.is_some()
            || self.find.search_bar.is_some()
            || !self.docs.views.contains_key(&m.at.view)
        {
            self.term.mouse_drag = None;
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
            MouseKind::Up(MouseButton::Left) => self.term.mouse_drag = None,
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
        let anchor = match self.docs.views.get(&view) {
            Some(v) if m.mods.contains(Modifiers::SHIFT) => v.selection.primary().anchor,
            _ => pos,
        };
        self.mouse_select(view, anchor, pos, aff);
        self.term.mouse_drag = Some(Drag {
            view,
            anchor,
            at: m.at,
            next_scroll: None,
        });
    }

    fn mouse_move(&mut self, at: MouseAt) {
        let Some(mut drag) = self.term.mouse_drag else {
            return;
        };
        drag.at = at;
        let rows = i32::from(self.viewport().1);
        if at.row < 0 || at.row >= rows {
            drag.next_scroll = drag.next_scroll.or(Some(self.now + AUTO_SCROLL_INTERVAL));
        } else {
            drag.next_scroll = None;
        }
        self.term.mouse_drag = Some(drag);
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
    pub(crate) fn mouse_tick(&mut self) {
        let Some(mut drag) = self.term.mouse_drag else {
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
        self.term.mouse_drag = Some(drag);
        self.drag_select(drag);
    }

    pub(crate) fn mouse_deadline(&self) -> Option<Tick> {
        self.term.mouse_drag.and_then(|d| d.next_scroll)
    }

    fn mouse_wheel(&mut self, m: MouseEvent, dir: i64) {
        let view = m.at.view;
        if m.mods.contains(Modifiers::SHIFT) {
            if wrap_ctx(self, view).is_none() {
                self.scroll_columns(view, dir * WHEEL_COLUMNS);
            }
        } else {
            self.scroll_lines(view, dir * i64::from(self.config().mouse_wheel_lines));
        }
    }

    /// 水平スクロール (折り返し OFF)。いちばん長い可視行の幅を超えて右へは動かさない。
    fn scroll_columns(&mut self, view: ViewId, delta: i64) {
        let rows = self.viewport().1 as usize;
        let Some(v) = self.docs.views.get(&view) else {
            return;
        };
        let Some(d) = self.docs.documents.get(&v.doc) else {
            return;
        };
        let t = d.text();
        let top = t.byte_to_line(v.scroll.top).0;
        let last = t.len_lines().saturating_sub(1);
        let widest = (top..=(top + rows).min(last))
            .map(|l| crate::layout::cursor_cell(d, l, usize::MAX).0)
            .max()
            .unwrap_or(0);
        let left = i64::from(v.scroll.left_col);
        let max_left = i64::try_from(widest.saturating_sub(1)).unwrap_or(i64::MAX);
        let new = left.saturating_add(delta).clamp(0, max_left);
        if new != left
            && let Some(v) = self.docs.views.get_mut(&view)
        {
            // left_col は u32。それより右へは動かさない (0 以上は clamp で保証)
            v.scroll.left_col = u32::try_from(new).unwrap_or(u32::MAX);
            self.mark_dirty_render();
        }
    }

    /// 面のセルに対応する文書の位置 (と折り返し位置の帰属)。03 §5.8.1 の規則。面の外の行は端の行に丸める。
    fn mouse_position(&self, at: MouseAt) -> Option<(ByteOff, Affinity)> {
        let v = self.docs.views.get(&at.view)?;
        let d = self.docs.documents.get(&v.doc)?;
        let (cols, rows) = self.viewport();
        let gw = i32::from(crate::layout::gutter_width(d, cols as usize));
        // どちらも 0 以上に丸めてから変換する
        let row = usize::try_from(at.row.clamp(0, (i32::from(rows) - 1).max(0))).unwrap_or(0);
        let text_col = usize::try_from(at.col.saturating_sub(gw).max(0)).unwrap_or(0);
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
            ByteOff(ls + crate::layout::byte_at_col(d, line, col)),
            Affinity::After,
        ))
    }
}

/// `ui.toggle_mouse`: マウスの取得の ON/OFF。OFF の間は端末のネイティブ選択・スクロールが使える (04 §6.8)。
fn toggle_mouse(ctx: &mut CommandCtx<'_>) -> CommandResult {
    let enabled = !ctx.state.term.mouse_on;
    ctx.state.term.mouse_on = enabled;
    ctx.state.term.mouse_drag = None;
    ctx.state.push_effect(Effect::SetMouseCapture { enabled });
    ctx.state.show_message(
        MessageLevel::Info,
        if enabled {
            keys::INFO_MOUSE_ENABLED
        } else {
            keys::INFO_MOUSE_DISABLED
        },
        &[],
    );
    CommandResult::Done
}

/// `ui.toggle_hint_bar`: ヒントバーの表示の ON/OFF (FR-504)。OFF の間はその行を本文に使う (04 §3.5)。
fn toggle_hint_bar(ctx: &mut CommandCtx<'_>) -> CommandResult {
    let on = !ctx.state.term.hint_bar_on;
    ctx.state.term.hint_bar_on = on;
    ctx.state.mark_dirty_render();
    ctx.state.show_message(
        MessageLevel::Info,
        if on {
            keys::INFO_HINT_BAR_SHOWN
        } else {
            keys::INFO_HINT_BAR_HIDDEN
        },
        &[],
    );
    CommandResult::Done
}

pub(crate) fn register(r: &mut CommandRegistry) {
    r.add(
        ids::UI_TOGGLE_HINT_BAR,
        CommandKind::Ui,
        Permission::Ui,
        "",
        toggle_hint_bar,
    );
    r.add(
        ids::UI_TOGGLE_MOUSE,
        CommandKind::Ui,
        Permission::None,
        "caps.mouse",
        toggle_mouse,
    );
}
