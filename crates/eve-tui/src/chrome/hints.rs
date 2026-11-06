//! ヒントバー (04 §8.8)。

use eve_view::{HintItem, HintModel, KeyLabel, Span};

use super::keys::{key_label_compact, key_label_full, label_width};
use super::parts::{ascii_span, put};
use crate::buffer::Writer;
use crate::glyphs::Glyphs;
use crate::theme::Theme;

pub(super) struct HintEntry<'a> {
    item: &'a HintItem,
    full: String,
    compact: String,
    /// 説明を落としてキーだけにする (最後の段)。
    key_only: bool,
    dropped: bool,
}

pub(super) fn hint_total(entries: &[HintEntry<'_>], compact: bool) -> u16 {
    let live: Vec<&HintEntry<'_>> = entries.iter().filter(|e| !e.dropped).collect();
    let gap = if compact { 1 } else { 2 };
    let sum: u16 = live
        .iter()
        .map(|e| {
            let k = label_width(if compact { &e.compact } else { &e.full });
            if e.key_only {
                k
            } else {
                k + 1 + e.item.text.width
            }
        })
        .sum();
    sum + live.len().saturating_sub(1) as u16 * gap
}

pub(super) fn render_hints(
    w: &mut Writer<'_>,
    y: u16,
    cols: u16,
    model: &HintModel,
    theme: &Theme,
    glyphs: &Glyphs,
    ascii: bool,
) {
    let mut entries: Vec<HintEntry<'_>> = model
        .items
        .iter()
        .map(|item| {
            let label = item.keys.first().cloned().unwrap_or(KeyLabel {
                strokes: Vec::new(),
            });
            HintEntry {
                item,
                full: key_label_full(&label, ascii),
                compact: key_label_compact(&label, ascii),
                key_only: false,
                dropped: false,
            }
        })
        .collect();
    // (1) 全表示 → (2) ^S 表記 → (3) 優先度の低いものから落とす → (4) 優先度 5 はキーだけ
    let mut compact = false;
    if hint_total(&entries, false) > cols {
        compact = true;
        let mut order: Vec<usize> = (0..entries.len()).collect();
        order.sort_by_key(|&i| (entries[i].item.priority, std::cmp::Reverse(i)));
        for &i in &order {
            if hint_total(&entries, true) <= cols {
                break;
            }
            if entries[i].item.priority < 5 {
                entries[i].dropped = true;
            }
        }
        for &i in &order {
            if hint_total(&entries, true) <= cols {
                break;
            }
            if !entries[i].dropped {
                entries[i].key_only = true;
            }
        }
    }
    let gap = if compact { 1 } else { 2 };
    let mut x = 0;
    let mut first = true;
    for e in entries.iter().filter(|e| !e.dropped) {
        if !first {
            x += gap;
        }
        first = false;
        let key = if compact { &e.compact } else { &e.full };
        if x >= cols {
            break;
        }
        // キー表記は ASCII か 1 セルの矢印 → 1 文字ずつ書く
        for ch in key.chars() {
            let mut tmp = [0u8; 4];
            let s = Span::new(ch.encode_utf8(&mut tmp).to_owned(), 1, theme.hint_key);
            x = w.span(x, y, cols, &s);
        }
        if !e.key_only {
            x = w.span(x, y, cols, &ascii_span(" ", theme.hint_bar));
            x = put(w, x, y, cols, &e.item.text.spans, theme.hint_bar);
        }
    }
    let _ = glyphs;
}
