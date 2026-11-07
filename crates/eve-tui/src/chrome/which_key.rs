//! which-key (04 §7.4)。

use eve_view::{CellSize, CellText, KeyLabel, KeyStroke, Span, Style, StyleMods, WhichKeyModel};

use super::keys::{key_label_full, label_width};
use super::overlay::{BoxRect, box_width, draw_box, draw_line};
use super::parts::ascii_span;
use crate::buffer::Writer;
use crate::glyphs::Glyphs;
use crate::theme::{Theme, overlay_style};

/// which-key (04 §7.4): チョードの保留が 500 ms を超えたときの、続けて押せるキーの一覧。編集面の左下に重ねる。
pub(super) fn render_which_key(
    w: &mut Writer<'_>,
    area: CellSize,
    model: &WhichKeyModel,
    theme: &Theme,
    glyphs: &Glyphs,
    ascii: bool,
) {
    let style = theme.overlay;
    let bold = overlay_style(style, Style::with_mods(StyleMods::BOLD));
    let label = |k: &KeyStroke| key_label_full(&KeyLabel { strokes: vec![*k] }, ascii);
    let prefix = key_label_full(
        &KeyLabel {
            strokes: model.prefix.clone(),
        },
        ascii,
    );
    let title_text = format!("{prefix} {}", glyphs.ellipsis.text);
    let title = CellText {
        width: label_width(&title_text),
        spans: vec![Span::new(
            title_text.clone(),
            label_width(&title_text),
            bold,
        )],
        cut: eve_view::CutPolicy::End,
    };
    let key_w = model
        .items
        .iter()
        .map(|i| label_width(&label(&i.key)))
        .max()
        .unwrap_or(0);
    let want_inner =
        (key_w + 2 + model.items.iter().map(|i| i.text.width).max().unwrap_or(0)).max(title.width);
    let (bw, bordered) = box_width(want_inner, area.cols);
    let pad = u16::from(bordered) * 2;
    let rows = crate::cells(model.items.len())
        .saturating_add(pad)
        .min(area.rows);
    if rows == 0 {
        return;
    }
    let b = BoxRect {
        x: 0,
        y: area.rows - rows,
        cols: bw,
        rows,
        bordered,
    };
    draw_box(w, &b, bordered.then_some(&title), style, glyphs);
    for (i, item) in model.items.iter().take(usize::from(rows - pad)).enumerate() {
        let k = label(&item.key);
        let spans = [
            Span::new(k.clone(), label_width(&k), bold),
            ascii_span(&" ".repeat(usize::from(key_w + 2 - label_width(&k))), style),
        ];
        let mut line: Vec<Span> = spans.to_vec();
        line.extend(item.text.spans.iter().cloned());
        draw_line(w, &b, crate::cells(i), &line, style, glyphs);
    }
}
