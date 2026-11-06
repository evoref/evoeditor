//! 共通の部品: 幅・省略・1 行の書き込み。

use eve_view::{Span, Style};

use crate::buffer::Writer;
use crate::glyphs::Glyph;
use crate::theme::overlay_style;

/// 状態バーなどの項目の間の空白。
pub(super) const ITEM_GAP: u16 = 1;
/// ステータスバーの左群と右群の間の最小の空き。
pub(super) const GROUP_GAP: u16 = 2;

pub(super) fn text_width(spans: &[Span]) -> u16 {
    spans.iter().map(|s| s.width).sum()
}

pub(super) fn ascii_span(text: &str, style: Style) -> Span {
    Span::new(text, text.len() as u16, style)
}

pub(super) fn glyph_span(g: &Glyph, style: Style) -> Span {
    Span::new(g.text, g.width, style)
}

/// 先頭から入るだけ取り、省略記号を付ける。
pub(super) fn truncate_end(spans: &[Span], max: u16, ell: &Glyph) -> Vec<Span> {
    if text_width(spans) <= max {
        return spans.to_vec();
    }
    if max < ell.width {
        return Vec::new();
    }
    let budget = max - ell.width;
    let mut out = Vec::new();
    let mut acc = 0;
    for s in spans {
        if acc + s.width > budget {
            break;
        }
        acc += s.width;
        out.push(s.clone());
    }
    let style = out.last().map_or(Style::DEFAULT, |s| s.style);
    out.push(glyph_span(ell, style));
    out
}

/// 中間を省略し、先頭と末尾 (拡張子) を残す。
pub(super) fn truncate_middle(spans: &[Span], max: u16, ell: &Glyph) -> Vec<Span> {
    if text_width(spans) <= max {
        return spans.to_vec();
    }
    if max < ell.width {
        return Vec::new();
    }
    let budget = max - ell.width;
    let head_budget = budget / 2;
    let tail_budget = budget - head_budget;
    let mut head = Vec::new();
    let mut acc = 0;
    for s in spans {
        if acc + s.width > head_budget {
            break;
        }
        acc += s.width;
        head.push(s.clone());
    }
    let mut tail = Vec::new();
    let mut acc = 0;
    for s in spans[head.len()..].iter().rev() {
        if acc + s.width > tail_budget {
            break;
        }
        acc += s.width;
        tail.push(s.clone());
    }
    tail.reverse();
    let style = head.last().map_or(Style::DEFAULT, |s| s.style);
    head.push(glyph_span(ell, style));
    head.extend(tail);
    head
}

/// `base` の上に各 `Span` のスタイルを重ねて書く。次の位置を返す。
pub(super) fn put(
    w: &mut Writer<'_>,
    x: u16,
    y: u16,
    limit: u16,
    spans: &[Span],
    base: Style,
) -> u16 {
    let mut x = x;
    for sp in spans {
        x = w.span_with(
            x,
            y,
            limit,
            &sp.text,
            sp.width,
            overlay_style(base, sp.style),
        );
    }
    x
}

pub(super) fn fill(w: &mut Writer<'_>, x: u16, y: u16, n: u16, style: Style) {
    let st = w.style(style);
    for i in 0..n {
        w.set(x + i, y, " ", st, false);
    }
}
