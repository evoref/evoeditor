//! マウス (docs/03 §5.8.1、01 FR-307): 左クリックで移動、Shift+クリックで拡張、ドラッグで選択、ホイールでスクロール。
//! ダブル / トリプルクリック (03 §5.8.2 の CLK-1〜CLK-7、01 FR-311): 押下の回数を Editor の「今」と位置で数え、単語・行を選ぶ。
//! 矩形選択のモード (03 §4.8 の BOX-6、01 FR-212) の間のドラッグと `Shift` 付きのクリックは、(論理行, 表示列) の矩形を選ぶ。
//!
//! 位置は面の左上を原点とするセルで届く。折り返し・タブ・全角を考慮した文書位置の確定は、ここでスクロール位置とレイアウトから直接引く。
//!
//! spec: 03 §5.8.1, 03 §5.8.2, 03 §4.8

use eve_text::{ByteOff, ByteRange, LineIdx, Range, Selection, TextRead, unit_at, unit_before};
use eve_view::{
    ChromeMouseEvent, ChromeTarget, Effect, MessageLevel, Modifiers, MouseAt, MouseButton,
    MouseEvent, MouseKind, ViewId,
};

use crate::command::ids;
use crate::command::{CommandCtx, CommandKind, CommandRegistry, CommandResult, Permission};
use crate::document::Document;
use crate::editor::state::{EditorState, View};
use crate::layout::{Affinity, byte_at_col, cursor_col, line_rows, wrap_ctx};
use crate::time::{Millis, Tick};
use crate::ui_model::keys;
use crate::view::box_select::{BoxPoint, BoxRect};
use crate::view::text_object::{WORD_WINDOW, line_whole};

/// Shift+ホイールの横スクロール量 (列)。
const WHEEL_COLUMNS: i64 = 4;
/// ドラッグが面の外にあるとき、1 表示行スクロールする間隔。
const AUTO_SCROLL_INTERVAL: Millis = Millis(50);
/// 多重クリックの間隔 (03 §5.8.2 の CLK-1)。直前の左の押下からこの時間以内 (ちょうどを含む) の押下を続けて数える。
/// Windows の既定のダブルクリックの時間と同じ。
pub const DOUBLE_CLICK_MS: Millis = Millis(500);
/// 数える回数の上限。4 回目より後も 3 回目 (行の全体) と同じに扱う (CLK-3)。
const MAX_CLICK_COUNT: u8 = 3;

/// 直前の左の押下 (修飾なし) の記録 (03 §5.8.2 の CLK-1。Editor に 1 つ)。ほかの入力で捨てる。
#[derive(Clone, Copy, Debug)]
pub(crate) struct Click {
    view: ViewId,
    /// 押した面の表示行 (`MouseAt.row`)。
    row: i32,
    /// 押した位置を確定した文書の位置。
    pos: ByteOff,
    /// 押したときの Editor の「今」。
    at: Tick,
    /// 続けて数えた回数 (1〜`MAX_CLICK_COUNT`)。
    count: u8,
}

/// ドラッグで選択を伸ばす単位 (03 §5.8.2 の CLK-4)。
#[derive(Clone, Copy, Debug)]
enum DragUnit {
    /// 1 回の押下: 文字の単位 (03 §5.8.1)。選択の起点 (動かさない側)。
    Char(ByteOff),
    /// 2 回の押下: §13.3 の単位 (CLK-2)。起点の単位 (空の行では空の範囲) と、直前に求めた位置の単位 (位置がその中に
    /// あれば読み直さない)。
    Word {
        origin: ByteRange,
        last: Option<ByteRange>,
    },
    /// 3 回以上の押下: 行の全体 (CLK-3)。起点の行の全体。
    Line(ByteRange),
    /// 矩形選択のモードの 1 回の押下 (03 §4.8 の BOX-6)。`anchor` は矩形の動かさない角 (押した位置。`Shift` 付きの押下では
    /// 矩形の `anchor`)、`last` は最後に置こうとした矩形、`refused` はそれを行の数の上限で断った (上限以下から上限超えへ
    /// 変わったときだけ知らせる。03 §4.8 の BOX-3)。
    Box {
        anchor: BoxPoint,
        last: Option<BoxRect>,
        refused: bool,
    },
}

/// ドラッグの位置と起点の単位の関係 (03 §5.8.2 の CLK-4)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    /// 位置の単位が起点の単位より前。
    Earlier,
    /// 位置の単位が起点の単位。
    Inside,
    /// 位置の単位が起点の単位より後ろ。
    Later,
}

/// 左ボタンのドラッグ中の状態。
#[derive(Clone, Copy, Debug)]
pub(crate) struct Drag {
    view: ViewId,
    /// 選択を伸ばす単位と起点。
    unit: DragUnit,
    /// 最後に届いたマウスの位置。
    at: MouseAt,
    /// 次の自動スクロールの時刻 (面の外にいる間だけ)。
    next_scroll: Option<Tick>,
}

impl EditorState {
    pub(crate) fn on_mouse(&mut self, m: MouseEvent) {
        // 多重クリックを数えるのは修飾なしの左の押下だけ。左のドラッグと離すは回数を変えず、ほかの操作 (ホイール・ほかの
        // ボタン・修飾付きの押下) は数え直す (03 §5.8.2 の CLK-1)
        let plain_left_down = m.kind == MouseKind::Down(MouseButton::Left) && m.mods.is_empty();
        let keeps_count = matches!(
            m.kind,
            MouseKind::Drag(MouseButton::Left) | MouseKind::Up(MouseButton::Left)
        );
        if !plain_left_down && !keeps_count {
            self.reset_click_count();
        }
        // オーバーレイ・検索バーが開いている間は、編集面のマウスを無視する
        if self.ui.overlay.is_some()
            || self.find.search_bar.is_some()
            || !self.docs.views.contains_key(&m.at.view)
        {
            self.term.mouse_drag = None;
            self.reset_click_count();
            return;
        }
        // 押す・ホイールは、指す View を表示中のペインに対して行う。どのペインの表示中の View でもなければ (Frame を
        // 作った後にペインが別の文書へ切り替わった・閉じた) 捨てる (03 §9.9 の PANE-7 (g))。ドラッグと Up は押した
        // ペインの View のまま (PANE-7 (b))
        let shown = self.pane_of_shown_view(m.at.view);
        match m.kind {
            // v0.2 は修飾なしと Shift だけ (Ctrl / Alt 付きは後の版)
            MouseKind::Down(MouseButton::Left)
                if m.mods.is_empty() || m.mods == Modifiers::SHIFT =>
            {
                let Some(at) = shown else {
                    self.term.mouse_drag = None;
                    self.reset_click_count();
                    return;
                };
                // フォーカスの無いペインを押した: そのペインにフォーカスを移してから、同じクリックの処理をする (PANE-7 (a))
                self.focus_pane(at);
                self.mouse_down(m);
            }
            MouseKind::Drag(MouseButton::Left) => self.mouse_move(m.at),
            MouseKind::Up(MouseButton::Left) => self.term.mouse_drag = None,
            // ホイールはポインタの下のペインをスクロールし、フォーカスは移さない (PANE-7 (c))
            MouseKind::WheelUp if shown.is_some() => self.mouse_wheel(m, -1),
            MouseKind::WheelDown if shown.is_some() => self.mouse_wheel(m, 1),
            _ => {}
        }
    }

    /// クローム (ペインの見出し・タブラインのタブ) のマウス (03 §9.9 の PANE-7 (d) / (e) / (f))。見出しの左 `Down` でその
    /// ペインにフォーカスを移す (カーソルは動かさない)。タブの左 `Down` でその文書をフォーカスのペインに表示する。見出しは、指す View がそのペインの表に残っていれば同じペインを指す (Frame を作った後に
    /// ペインが別の文書へ切り替わっても、フォーカスを移すだけなので)。ほかの操作 (中・右のボタン・ホイール) とオーバーレイ・
    /// 検索バーを開いている間は何もしない (PANE-6 (c))。
    pub(crate) fn on_chrome_mouse(&mut self, m: ChromeMouseEvent) {
        // 編集面の外の操作は、多重クリックを数え直す (03 §5.8.2 の CLK-1。見出し・タブの押下は編集面の押下と続けない)
        self.reset_click_count();
        if self.ui.overlay.is_some() || self.find.search_bar.is_some() {
            return;
        }
        // 左の押下だけに反応する (修飾キーは問わない)。見出しとタブの Drag / Up・ホイール・中と右のボタンは捨てる (PANE-7 (f))
        if m.kind != MouseKind::Down(MouseButton::Left) {
            return;
        }
        match m.target {
            ChromeTarget::PaneBar { view } => {
                self.term.mouse_drag = None;
                // 消えた View・どのペインにも属さない View を指す入力は捨てる (PANE-7 (g))
                if let Some(at) = self.pane_owning(view) {
                    self.focus_pane(at);
                }
            }
            // タブ (FR-510): その文書をフォーカスのペインに表示する (PANE-7 (e)、PANE-12 (g) / (i))
            ChromeTarget::Tab { doc } => {
                self.term.mouse_drag = None;
                self.click_tab(doc);
            }
            _ => {}
        }
    }

    /// 多重クリックの数えを捨てる (03 §5.8.2 の CLK-1 の「数え直す」。次の左の押下は 1 回目になる)。
    pub(crate) fn reset_click_count(&mut self) {
        self.term.mouse_click = None;
    }

    /// 修飾なしの左の押下の回数を数えて記録する (03 §5.8.2 の CLK-1)。直前の押下と同じ View・同じ表示行で、確定した位置が
    /// 同じで、間隔が `DOUBLE_CLICK_MS` 以内なら 1 増やし (上限 `MAX_CLICK_COUNT`)、そうでなければ 1。全角の左右の半分・
    /// 行末より右の別のセルは位置が同じなので続けて数え、スクロールや本文の変化で同じセルが別の位置を指せば数え直す。
    fn count_click(&mut self, at: MouseAt, pos: ByteOff) -> u8 {
        let now = self.now;
        let count = match self.term.mouse_click {
            Some(c)
                if c.view == at.view
                    && c.row == at.row
                    && c.pos == pos
                    && now.saturating_sub(c.at) <= DOUBLE_CLICK_MS =>
            {
                c.count.saturating_add(1).min(MAX_CLICK_COUNT)
            }
            _ => 1,
        };
        self.term.mouse_click = Some(Click {
            view: at.view,
            row: at.row,
            pos,
            at: now,
            count,
        });
        count
    }

    fn mouse_down(&mut self, m: MouseEvent) {
        let view = m.at.view;
        let Some((pos, aff)) = self.mouse_position(m.at) else {
            self.reset_click_count();
            return;
        };
        // Undo の結合はカーソル移動と同じく封印する (2 回・3 回の押下も同じ。03 §5.8.2 の CLK-6)
        self.seal_active_history();
        let shift = m.mods.contains(Modifiers::SHIFT);
        let count = if shift {
            // `Shift` 付きの押下は数え直す (CLK-1)。次の修飾なしの押下は 1 回目
            self.reset_click_count();
            1
        } else {
            self.count_click(m.at, pos)
        };
        // 矩形選択のモードの 1 回の押下 (03 §4.8 の BOX-6)。2 回・3 回の押下はモードの間も単語・行を選ぶ (CLK-5)
        let cell = if count == 1 && self.box_mode(view) {
            self.box_cell(m.at)
        } else {
            None
        };
        let unit = match (count, cell) {
            (1, Some(cell)) if shift => {
                // 矩形の `head` をその位置へ (矩形が無ければ主の Range の `head` から作る)
                let anchor = self.box_rect_or_primary(view).map_or(cell, |r| r.anchor);
                let rect = BoxRect { anchor, head: cell };
                let applied = self.apply_box(view, rect);
                DragUnit::Box {
                    anchor,
                    last: Some(rect),
                    refused: !applied,
                }
            }
            (1, Some(cell)) => {
                // ドラッグしない押下は矩形を消して 1 つのカーソルにする。ドラッグは押した位置を `anchor` にした矩形
                self.mouse_select(view, pos, pos, aff);
                DragUnit::Box {
                    anchor: cell,
                    last: None,
                    refused: false,
                }
            }
            (1, None) => {
                let anchor = match self.docs.views.get(&view) {
                    Some(v) if shift => v.selection.primary().anchor,
                    _ => pos,
                };
                self.mouse_select(view, anchor, pos, aff);
                DragUnit::Char(anchor)
            }
            // 単位を前向きに選ぶ (CLK-2 / CLK-3。空の行の単位は空なので、1 回目のカーソルのまま)
            (2, _) => {
                let u = self.click_unit(view, pos, aff);
                self.mouse_select(view, u.start, u.end, word_head_affinity(u.start, u.end));
                DragUnit::Word {
                    origin: u,
                    last: None,
                }
            }
            (_, _) => {
                let u = self.click_line(view, pos);
                self.mouse_select(view, u.start, u.end, Affinity::After);
                DragUnit::Line(u)
            }
        };
        self.term.mouse_drag = Some(Drag {
            view,
            unit,
            at: m.at,
            next_scroll: None,
        });
    }

    /// 位置 `pos` (帰属 `aff`) の §13.3 の単位 (03 §5.8.2 の CLK-2。窓は 64 KiB)。位置が表示行の末尾 (`Before` に帰属する
    /// 折り返し位置。論理行の行末は `unit_at` が扱う) なら直前の書記素の単位。空の行では空の範囲 (何も選ばない)。
    fn click_unit(&self, view: ViewId, pos: ByteOff, aff: Affinity) -> ByteRange {
        self.view_text(view)
            .and_then(|t| {
                if aff == Affinity::Before {
                    unit_before(t, pos, WORD_WINDOW).or_else(|| unit_at(t, pos, WORD_WINDOW))
                } else {
                    unit_at(t, pos, WORD_WINDOW)
                }
            })
            .unwrap_or(ByteRange::new(pos, pos))
    }

    /// 位置 `pos` の行の全体 (03 §5.8.2 の CLK-3。折り返し ON でも論理行。本文は読まず、行の番号を引くだけ)。
    fn click_line(&self, view: ViewId, pos: ByteOff) -> ByteRange {
        self.view_text(view).map_or(ByteRange::new(pos, pos), |t| {
            line_whole(t, t.byte_to_line(pos).0)
        })
    }

    /// View `view` の文書の本文。
    fn view_text(&self, view: ViewId) -> Option<&dyn TextRead> {
        let v = self.docs.views.get(&view)?;
        let d = self.docs.documents.get(&v.doc)?;
        Some(d.text())
    }

    fn mouse_move(&mut self, at: MouseAt) {
        let Some(mut drag) = self.term.mouse_drag else {
            return;
        };
        drag.at = at;
        let rows = i32::from(self.view_size(drag.view).1);
        if at.row < 0 || at.row >= rows {
            drag.next_scroll = drag.next_scroll.or(Some(self.now + AUTO_SCROLL_INTERVAL));
        } else {
            drag.next_scroll = None;
        }
        self.drag_select(&mut drag);
        self.term.mouse_drag = Some(drag);
    }

    /// 起点からマウスの位置までを選ぶ (面の外の行は端の行に丸める)。2 回・3 回の押下に続くドラッグは、起点の単位を含めて
    /// 位置の単位の端まで選ぶ (03 §5.8.2 の CLK-4。`head` は位置の側。自動スクロールもここを通る)。位置が起点の単位の中
    /// なら位置の単位を求めず、直前に求めた単位の中なら読み直さない (CLK-7)。
    fn drag_select(&mut self, drag: &mut Drag) {
        let view = drag.view;
        if let DragUnit::Box {
            anchor,
            last,
            refused,
        } = &mut drag.unit
        {
            return self.box_drag(view, drag.at, anchor, last, refused);
        }
        let Some((pos, aff)) = self.mouse_position(drag.at) else {
            return;
        };
        match &mut drag.unit {
            DragUnit::Char(anchor) => self.mouse_select(view, *anchor, pos, aff),
            DragUnit::Word { origin, last } => {
                let origin = *origin;
                let side = side_of(origin, pos, aff);
                let target = match (side, *last) {
                    (Side::Inside, _) => origin,
                    (_, Some(t)) if unit_holds(t, pos, aff) => t,
                    _ => {
                        let t = self.click_unit(view, pos, aff);
                        *last = Some(t);
                        t
                    }
                };
                let (anchor, head) = unit_drag_range(origin, target, side);
                self.mouse_select(view, anchor, head, word_head_affinity(anchor, head));
            }
            DragUnit::Line(origin) => {
                let origin = *origin;
                let side = side_of(origin, pos, aff);
                let target = if side == Side::Inside {
                    origin
                } else {
                    self.click_line(view, pos)
                };
                let (anchor, head) = unit_drag_range(origin, target, side);
                self.mouse_select(view, anchor, head, Affinity::After);
            }
            DragUnit::Box { .. } => {}
        }
    }

    /// 矩形選択のモードのドラッグ (03 §4.8 の BOX-6): `anchor` からドラッグの位置までの矩形にする。ドラッグの間に文書が縮んで
    /// いれば `anchor` の行を最後の行に丸める。矩形が前回と同じなら何もしない。行の数の上限を超えて断ったときは、上限以下
    /// から上限超えへ変わったときだけ `info.cursor.limit` を出す (自動スクロールで超え続けても知らせを積み上げない)。
    fn box_drag(
        &mut self,
        view: ViewId,
        at: MouseAt,
        anchor: &mut BoxPoint,
        last: &mut Option<BoxRect>,
        refused: &mut bool,
    ) {
        let Some(head) = self.box_cell(at) else {
            return;
        };
        let last_line = self
            .view_text(view)
            .map_or(0, |t| t.len_lines().saturating_sub(1));
        anchor.line = anchor.line.min(last_line);
        let rect = BoxRect {
            anchor: *anchor,
            head,
        };
        if *last == Some(rect) && (*refused || self.box_rect(view) == Some(rect)) {
            return;
        }
        *last = Some(rect);
        if self.try_apply_box(view, rect) {
            *refused = false;
        } else {
            if !*refused {
                self.notify_cursor_limit();
            }
            *refused = true;
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
        self.drag_select(&mut drag);
        self.term.mouse_drag = Some(drag);
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
        let rows = self.view_size(view).1 as usize;
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

    /// 面のセル `at` の (View, 文書, 面の行, ガターの後ろの列)。面の外の行は端の行に丸め、ガターの列は列 0 にする (03 §5.8.1 の
    /// 「位置の確定」)。
    fn surface_cell(&self, at: MouseAt) -> Option<(&View, &Document, usize, usize)> {
        let v = self.docs.views.get(&at.view)?;
        let d = self.docs.documents.get(&v.doc)?;
        let (cols, rows) = self.view_size(at.view);
        let gw = i32::from(crate::layout::gutter_width(d, cols as usize));
        // どちらも 0 以上に丸めてから変換する
        let row = usize::try_from(at.row.clamp(0, (i32::from(rows) - 1).max(0))).unwrap_or(0);
        let text_col = usize::try_from(at.col.saturating_sub(gw).max(0)).unwrap_or(0);
        Some((v, d, row, text_col))
    }

    /// 面のセルに対応する矩形の角 (論理行, 行頭からの表示列) (03 §4.8 の BOX-6)。行は §5.8.1 の「位置の確定」と同じに決め
    /// (面の外の行は端の行、文書の末尾より下は最後の行)、表示行の中のセルはその書記素の始まりの列、表示行の末尾より右の
    /// セルは表示行の末尾の表示列にはみ出したセルの数を足した列 (仮想の列)。折り返し ON でも論理行の行頭から数える。
    pub(crate) fn box_cell(&self, at: MouseAt) -> Option<BoxPoint> {
        let (v, d, row, text_col) = self.surface_cell(at)?;
        let t = d.text();
        let last = t.len_lines().saturating_sub(1);
        if let Some((s, width)) = wrap_ctx(self, at.view) {
            let (line, r) = self
                .wrapped_row_at(at.view, row)
                .unwrap_or_else(|| (last, line_rows(d, last, s, width).len().saturating_sub(1)));
            let lr = line_rows(d, line, s, width);
            return Some(BoxPoint {
                line,
                col: lr.display_col_at(r, text_col),
            });
        }
        let line = (t.byte_to_line(v.scroll.top).0 + row).min(last);
        let col = v.scroll.left_col as usize + text_col;
        let (ls, le) = (
            t.line_to_byte(LineIdx(line)).0,
            t.line_end_byte(LineIdx(line)).0,
        );
        let p = byte_at_col(d, line, col);
        let col = if ls + p < le {
            cursor_col(d, line, p)
        } else {
            col
        };
        Some(BoxPoint { line, col })
    }

    /// 面のセルに対応する文書の位置 (と折り返し位置の帰属)。03 §5.8.1 の規則。面の外の行は端の行に丸める。
    fn mouse_position(&self, at: MouseAt) -> Option<(ByteOff, Affinity)> {
        let (v, d, row, text_col) = self.surface_cell(at)?;
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

/// 位置 `pos` (帰属 `aff`) の単位が `r` と分かるか (03 §5.8.2 の CLK-2 の定義で、`r` を読み直さずに決められる場合)。
/// 単位の中の位置、`After` の始まり (直後の書記素が `r` の最初)、`Before` の終わり (直前の書記素が `r` の最後)。論理行の
/// 行末など、ほかの場合は `false` (読み直して求める)。
fn unit_holds(r: ByteRange, pos: ByteOff, aff: Affinity) -> bool {
    (r.start < pos && pos < r.end)
        || (pos == r.start && pos < r.end && aff == Affinity::After)
        || (pos == r.end && r.start < pos && aff == Affinity::Before)
}

/// ドラッグの位置 `pos` (帰属 `aff`) と起点の単位 `origin` の関係 (03 §5.8.2 の CLK-4)。
fn side_of(origin: ByteRange, pos: ByteOff, aff: Affinity) -> Side {
    if unit_holds(origin, pos, aff) {
        Side::Inside
    } else if pos >= origin.end {
        Side::Later
    } else {
        Side::Earlier
    }
}

/// 単位のドラッグの `(anchor, head)` (03 §5.8.2 の CLK-4)。`origin` は起点の単位、`target` はドラッグの位置の単位。
/// 位置が起点の単位より後ろなら位置の単位の終わりまで前向きに、前なら位置の単位の始まりまで後ろ向きに選び、起点の単位を常に
/// 含める。位置が起点の単位の中なら起点の単位を前向きに選ぶ。
fn unit_drag_range(origin: ByteRange, target: ByteRange, side: Side) -> (ByteOff, ByteOff) {
    match side {
        Side::Later => (origin.start, origin.end.max(target.end)),
        Side::Earlier => (origin.end, origin.start.min(target.start)),
        Side::Inside => (origin.start, origin.end),
    }
}

/// 単語の単位の選択の `head` の帰属 (03 §5.8.2 の CLK-4)。前向きの `head` は単位の終わりなので、折り返し位置なら上の表示行の
/// 末尾 (`Before`)。後ろ向き・空の選択は単位の始まりなので下の表示行の先頭 (`After`)。`Before` は折り返し位置でだけ効く。
fn word_head_affinity(anchor: ByteOff, head: ByteOff) -> Affinity {
    if head > anchor {
        Affinity::Before
    } else {
        Affinity::After
    }
}

/// `ui.toggle_mouse`: マウスの取得の ON/OFF。OFF の間は端末のネイティブ選択・スクロールが使える (04 §6.8)。
fn toggle_mouse(ctx: &mut CommandCtx<'_>) -> CommandResult {
    let enabled = !ctx.state.term.mouse_on;
    ctx.state.term.mouse_on = enabled;
    ctx.state.term.mouse_drag = None;
    ctx.state.term.mouse_click = None;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn r(start: usize, end: usize) -> ByteRange {
        ByteRange::new(ByteOff(start), ByteOff(end))
    }

    // spec: 03 §5.8.2 (CLK-4)
    #[test]
    fn 位置と起点の単位の関係() {
        use Affinity::{After, Before};
        // (起点, 位置, 帰属, 関係)
        let cases = [
            (r(4, 7), 4, After, Side::Inside),
            (r(4, 7), 5, After, Side::Inside),
            (r(4, 7), 5, Before, Side::Inside),
            (r(4, 7), 7, Before, Side::Inside),
            (r(4, 7), 7, After, Side::Later),
            (r(4, 7), 9, After, Side::Later),
            (r(4, 7), 4, Before, Side::Earlier),
            (r(4, 7), 2, After, Side::Earlier),
            // 空の起点 (空の行)
            (r(3, 3), 3, After, Side::Later),
            (r(3, 3), 5, After, Side::Later),
            (r(3, 3), 1, After, Side::Earlier),
        ];
        for (origin, pos, aff, want) in cases {
            assert_eq!(
                side_of(origin, ByteOff(pos), aff),
                want,
                "{origin:?} {pos} {aff:?}"
            );
        }
    }

    // spec: 03 §5.8.2 (CLK-4)
    #[test]
    fn 単位のドラッグの範囲() {
        let at = |(a, h): (ByteOff, ByteOff)| (a.0, h.0);
        // (起点, 位置の単位, 関係, (anchor, head))
        let cases = [
            (r(4, 7), r(8, 11), Side::Later, (4, 11)),
            (r(4, 7), r(0, 3), Side::Earlier, (7, 0)),
            (r(4, 7), r(4, 7), Side::Inside, (4, 7)),
            // 位置の単位が空 (空の行): 起点の単位を含めて位置まで
            (r(4, 7), r(9, 9), Side::Later, (4, 9)),
            (r(4, 7), r(2, 2), Side::Earlier, (7, 2)),
            // 起点が空 (空の行で 2 回押した)
            (r(3, 3), r(4, 8), Side::Later, (3, 8)),
            (r(3, 3), r(0, 2), Side::Earlier, (3, 0)),
            (r(3, 3), r(3, 3), Side::Later, (3, 3)),
            // 位置の単位が起点より手前で終わる (行末より右の位置が起点の終わりの直前の単位を指す) ときも起点を含める
            (r(4, 7), r(4, 7), Side::Later, (4, 7)),
        ];
        for (origin, target, side, want) in cases {
            assert_eq!(
                at(unit_drag_range(origin, target, side)),
                want,
                "{origin:?} {target:?} {side:?}"
            );
        }
    }

    // spec: 03 §5.8.2 (CLK-4)
    #[test]
    fn 単語の_head_の帰属は前向きなら上の表示行の末尾() {
        assert_eq!(word_head_affinity(ByteOff(4), ByteOff(7)), Affinity::Before);
        assert_eq!(word_head_affinity(ByteOff(7), ByteOff(4)), Affinity::After);
        assert_eq!(word_head_affinity(ByteOff(3), ByteOff(3)), Affinity::After);
    }
}
