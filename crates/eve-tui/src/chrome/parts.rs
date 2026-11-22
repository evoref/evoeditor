//! 共通の部品: 幅・省略・1 行の書き込み。

use eve_view::{CutPolicy, Span, Style};

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
    Span::new(text, crate::cells(text.len()), style)
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

/// `cut` に従って `max` セルに収め、各 `Span` に元の番号を添えて返す (省略記号は `None`)。`Middle` は中間を、それ以外
/// (`End`・`Never`) は末尾を切る (`Never` でも、入らなければ一覧の行は末尾を切って出す)。強調の位置 (元の番号) を切った後の
/// `Span` に写すのに使う (04 §8.11)。
pub(super) fn truncate_indexed(
    spans: &[Span],
    max: u16,
    cut: CutPolicy,
    ell: &Glyph,
) -> Vec<(Option<usize>, Span)> {
    if text_width(spans) <= max {
        return spans
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, s)| (Some(i), s))
            .collect();
    }
    if max < ell.width {
        return Vec::new();
    }
    let budget = max - ell.width;
    let head_budget = if cut == CutPolicy::Middle {
        budget / 2
    } else {
        budget
    };
    let mut head = 0;
    let mut acc = 0;
    for s in spans {
        if acc + s.width > head_budget {
            break;
        }
        acc += s.width;
        head += 1;
    }
    let mut tail = spans.len();
    if cut == CutPolicy::Middle {
        let tail_budget = budget - acc;
        let mut acc = 0;
        while tail > head && acc + spans[tail - 1].width <= tail_budget {
            acc += spans[tail - 1].width;
            tail -= 1;
        }
    }
    let style = spans[..head].last().map_or(Style::DEFAULT, |s| s.style);
    let mut out: Vec<(Option<usize>, Span)> = spans[..head]
        .iter()
        .cloned()
        .enumerate()
        .map(|(i, s)| (Some(i), s))
        .collect();
    out.push((None, glyph_span(ell, style)));
    out.extend(
        spans[tail..]
            .iter()
            .cloned()
            .enumerate()
            .map(|(k, s)| (Some(tail + k), s)),
    );
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(s: &str) -> Vec<Span> {
        s.chars()
            .map(|c| Span::new(c.to_string(), 1, Style::DEFAULT))
            .collect()
    }

    fn shown(v: &[(Option<usize>, Span)]) -> (String, Vec<Option<usize>>) {
        (
            v.iter().map(|(_, s)| s.text.as_str()).collect(),
            v.iter().map(|(i, _)| *i).collect(),
        )
    }

    const ELL: Glyph = Glyph {
        text: "~",
        width: 1,
    };

    #[test]
    fn 中間を切ると後ろの番号は元の番号のまま() {
        let v = truncate_indexed(&spans("abcdefghij"), 7, CutPolicy::Middle, &ELL);
        let (text, idx) = shown(&v);
        assert_eq!(text, "abc~hij");
        assert_eq!(
            idx,
            [Some(0), Some(1), Some(2), None, Some(7), Some(8), Some(9)]
        );
    }

    #[test]
    fn 末尾を切ると省略記号には番号が無い() {
        let v = truncate_indexed(&spans("abcdef"), 4, CutPolicy::End, &ELL);
        let (text, idx) = shown(&v);
        assert_eq!(text, "abc~");
        assert_eq!(idx, [Some(0), Some(1), Some(2), None]);
    }

    #[test]
    fn 入るときは切らない() {
        let v = truncate_indexed(&spans("abc"), 3, CutPolicy::Middle, &ELL);
        assert_eq!(shown(&v).1, [Some(0), Some(1), Some(2)]);
    }
}
