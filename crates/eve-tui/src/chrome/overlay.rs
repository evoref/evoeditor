//! オーバーレイの共通の部品: 枠・行・ボタン。

use eve_view::{CellText, Choice, CutPolicy, Span, Style};

use super::parts::{ascii_span, fill, glyph_span, put, truncate_end, truncate_middle};
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
        // `cut` が中間なら中間を詰める (ファインダの見出しの長いルート、04 §8.11)。ほかは末尾を切る
        let spans = if t.cut == CutPolicy::Middle {
            truncate_middle(&t.spans, room, &glyphs.ellipsis)
        } else {
            truncate_end(&t.spans, room, &glyphs.ellipsis)
        };
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

#[cfg(test)]
mod tests {
    use eve_view::{ColorDepth, StyleMods};
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Modifier;

    use super::*;

    const AREA: Rect = Rect::new(0, 0, 12, 5);

    const BOX: BoxRect = BoxRect {
        x: 1,
        y: 1,
        cols: 10,
        rows: 3,
        bordered: true,
    };

    /// 全面を属性 `mods` の本文で埋めた画面 (現在行の行番号の太字・選択の反転・カーソルの反転などの代わり)。
    fn screen_under(mods: StyleMods) -> Buffer {
        let mut buf = Buffer::empty(AREA);
        let mut w = Writer {
            buf: &mut buf,
            depth: ColorDepth::Mono,
            clamped: 0,
            sanitized: 0,
        };
        let st = w.style(Style::with_mods(mods));
        for y in 0..AREA.height {
            for x in 0..AREA.width {
                w.set(x, y, "a", st, false);
            }
        }
        buf
    }

    /// `buf` に枠つきのオーバーレイ `BOX` をスタイル `style` で描き、枠の中のセルの属性を返す。
    fn box_over(buf: &mut Buffer, style: Style) -> Vec<((u16, u16), Modifier)> {
        let mut w = Writer {
            buf,
            depth: ColorDepth::Mono,
            clamped: 0,
            sanitized: 0,
        };
        draw_box(&mut w, &BOX, None, style, &Glyphs::unicode());
        (BOX.y..BOX.y + BOX.rows)
            .flat_map(|y| (BOX.x..BOX.x + BOX.cols).map(move |x| (x, y)))
            .map(|pos| (pos, buf[pos].modifier))
            .collect()
    }

    #[test]
    fn オーバーレイの枠と塗りは下のセルの属性を引き継がない() {
        let under = StyleMods::BOLD
            | StyleMods::ITALIC
            | StyleMods::UNDERLINE
            | StyleMods::REVERSE
            | StyleMods::DIM;
        let mut buf = screen_under(under);
        for (pos, m) in box_over(&mut buf, Style::DEFAULT) {
            assert_eq!(m, Modifier::empty(), "{pos:?} に下の属性が残る");
        }
        // 枠の外の本文の属性はそのまま
        let all = Modifier::BOLD
            | Modifier::ITALIC
            | Modifier::UNDERLINED
            | Modifier::REVERSED
            | Modifier::DIM;
        assert_eq!(buf[(0, 0)].modifier, all);
    }

    #[test]
    fn オーバーレイのセルの属性はオーバーレイのスタイルのものだけになる() {
        let mut buf = screen_under(StyleMods::BOLD | StyleMods::UNDERLINE);
        for (pos, m) in box_over(&mut buf, Style::with_mods(StyleMods::REVERSE)) {
            assert_eq!(m, Modifier::REVERSED, "{pos:?}");
        }
    }
}
