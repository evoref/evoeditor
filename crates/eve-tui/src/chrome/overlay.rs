//! オーバーレイの共通の部品: 枠・行・ボタン。

use eve_view::{CellText, Choice, Span, Style};

use super::parts::{ascii_span, fill, glyph_span, put, truncate_end};
use crate::buffer::Writer;
use crate::glyphs::Glyphs;
use crate::theme::{Theme, overlay_style};

/// 枠。`bordered` が偽なら外周も内側として使う。
#[derive(Clone, Copy)]
pub(super) struct BoxRect {
    pub(super) x: u16,
    pub(super) y: u16,
    pub(super) cols: u16,
    pub(super) rows: u16,
    pub(super) bordered: bool,
}

impl BoxRect {
    /// 内側の左端・幅 (枠ありは左右に 1 セルの余白)。
    pub(super) fn inner_x(&self) -> u16 {
        self.x + if self.bordered { 2 } else { 0 }
    }

    pub(super) fn inner_cols(&self) -> u16 {
        self.cols.saturating_sub(if self.bordered { 4 } else { 0 })
    }

    pub(super) fn inner_y(&self) -> u16 {
        self.y + u16::from(self.bordered)
    }
}

pub(super) fn draw_box(
    w: &mut Writer<'_>,
    b: &BoxRect,
    title: Option<&CellText>,
    style: Style,
    glyphs: &Glyphs,
) {
    for r in 0..b.rows {
        fill(w, b.x, b.y + r, b.cols, style);
    }
    if !b.bordered || b.cols < 2 || b.rows < 2 {
        return;
    }
    let st = w.style(style);
    let g = &glyphs.boxed;
    let (x1, y1) = (b.x + b.cols - 1, b.y + b.rows - 1);
    for x in b.x + 1..x1 {
        w.set(x, b.y, g.horizontal.text, st, false);
        w.set(x, y1, g.horizontal.text, st, false);
    }
    for y in b.y + 1..y1 {
        w.set(b.x, y, g.vertical.text, st, false);
        w.set(x1, y, g.vertical.text, st, false);
    }
    w.set(b.x, b.y, g.top_left.text, st, false);
    w.set(x1, b.y, g.top_right.text, st, false);
    w.set(b.x, y1, g.bottom_left.text, st, false);
    w.set(x1, y1, g.bottom_right.text, st, false);
    if let Some(t) = title {
        // `┌─ 見出し ─…┐`
        let room = b.cols.saturating_sub(6);
        let spans = truncate_end(&t.spans, room, &glyphs.ellipsis);
        if !spans.is_empty() {
            let x = b.x + 2;
            let x = w.span(x, b.y, x1, &ascii_span(" ", style));
            let x = put(w, x, b.y, x1, &spans, style);
            w.span(x, b.y, x1, &ascii_span(" ", style));
        }
    }
}

/// 1 行のスパン列を、内側の幅に収めて書く。
pub(super) fn draw_line(
    w: &mut Writer<'_>,
    b: &BoxRect,
    row: u16,
    spans: &[Span],
    style: Style,
    glyphs: &Glyphs,
) {
    let cut = truncate_end(spans, b.inner_cols(), &glyphs.ellipsis);
    put(
        w,
        b.inner_x(),
        b.inner_y() + row,
        b.inner_x() + b.inner_cols(),
        &cut,
        style,
    );
}

/// 枠の幅: 端末幅 − 4 まで。それ未満 (狭すぎる) なら全幅で枠なし (04 §3.5)。
pub(super) fn box_width(want_inner: u16, cols: u16) -> (u16, bool) {
    const MIN_BOXED: u16 = 16;
    let avail = cols.saturating_sub(4);
    if avail < MIN_BOXED {
        return (cols, false);
    }
    ((want_inner + 4).min(avail).max(MIN_BOXED), true)
}

pub(super) fn button_spans(
    choices: &[Choice],
    focused: eve_view::ChoiceId,
    theme: &Theme,
    glyphs: &Glyphs,
) -> (Vec<Span>, u16) {
    let mut spans = Vec::new();
    let mut focus_x = 0;
    let mut x = 0u16;
    for (i, c) in choices.iter().enumerate() {
        if i > 0 {
            spans.push(ascii_span("   ", theme.dialog_button));
            x += 3;
        }
        let is_focus = c.id == focused;
        let st = if is_focus {
            theme.dialog_button_focused
        } else {
            theme.dialog_button
        };
        let mark = if is_focus {
            glyph_span(&glyphs.focus, st)
        } else {
            ascii_span(" ", st)
        };
        if is_focus {
            focus_x = x;
        }
        x += mark.width;
        spans.push(mark);
        // 選択肢の文言は `[S] 保存して終了` のようにアクセラレータを含んで届く (03 / 言語ファイル)
        x += c.label.width;
        spans.extend(
            c.label
                .spans
                .iter()
                .map(|s| Span::new(s.text.clone(), s.width, overlay_style(st, s.style))),
        );
    }
    (spans, focus_x)
}
