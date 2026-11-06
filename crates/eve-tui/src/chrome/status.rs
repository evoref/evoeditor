//! ステータスバー (04 §8.7)。

use eve_view::{CutPolicy, Span, StatusAlign, StatusItem, StatusKind, StatusModel};

use super::parts::{
    GROUP_GAP, ITEM_GAP, ascii_span, fill, put, text_width, truncate_end, truncate_middle,
};
use crate::buffer::Writer;
use crate::glyphs::Glyphs;
use crate::theme::{Theme, overlay_style};

pub(super) struct StatusEntry {
    kind: StatusKind,
    align: StatusAlign,
    priority: u8,
    /// 現在の形。
    spans: Vec<Span>,
    short: Option<Vec<Span>>,
    middle: bool,
    dropped: bool,
    /// 表示位置の順位 (左群 → 右群、群の中は `items` の順)。
    rank: usize,
}

pub(super) fn status_total(entries: &[StatusEntry]) -> u16 {
    let group = |a: StatusAlign| {
        let ws: Vec<u16> = entries
            .iter()
            .filter(|e| !e.dropped && e.align == a)
            .map(|e| text_width(&e.spans))
            .collect();
        let sum: u16 = ws.iter().sum();
        let gaps = ws.len().saturating_sub(1) as u16 * ITEM_GAP;
        (ws.len(), sum + gaps)
    };
    let (ln, lw) = group(StatusAlign::Left);
    let (rn, rw) = group(StatusAlign::Right);
    lw + rw + if ln > 0 && rn > 0 { GROUP_GAP } else { 0 }
}

/// 優先度の低いものから、同じ優先度では右にあるものから。
pub(super) fn status_order(entries: &[StatusEntry]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..entries.len()).collect();
    order.sort_by_key(|&i| (entries[i].priority, std::cmp::Reverse(entries[i].rank)));
    order
}

pub(super) fn render_status(
    w: &mut Writer<'_>,
    y: u16,
    cols: u16,
    model: &StatusModel,
    theme: &Theme,
    glyphs: &Glyphs,
) {
    let base = theme.status_bar;
    fill(w, 0, y, cols, base);
    let mut entries: Vec<StatusEntry> = model
        .items
        .iter()
        .enumerate()
        .map(|(i, it): (usize, &StatusItem)| StatusEntry {
            kind: it.kind,
            align: it.align,
            priority: it.priority,
            spans: it.long.spans.clone(),
            short: it.short.as_ref().map(|s| s.spans.clone()),
            middle: it.long.cut == CutPolicy::Middle,
            dropped: false,
            rank: usize::from(it.align == StatusAlign::Right) * model.items.len() + i,
        })
        .collect();
    let order = status_order(&entries);
    // (1) 短い形にする
    for &i in &order {
        if status_total(&entries) <= cols {
            break;
        }
        if let Some(s) = entries[i].short.take() {
            entries[i].spans = s;
        }
    }
    // (2) 中間を省略できる項目 (ファイル名) を縮める
    let total = status_total(&entries);
    if total > cols {
        let excess = total - cols;
        if let Some(e) = entries.iter_mut().find(|e| e.middle && !e.dropped) {
            let want = text_width(&e.spans).saturating_sub(excess).max(5);
            e.spans = truncate_middle(&e.spans, want, &glyphs.ellipsis);
        }
    }
    // (3) 項目を落とす (優先度 5 は落とさない)
    for &i in &order {
        if status_total(&entries) <= cols {
            break;
        }
        if entries[i].priority < 5 {
            entries[i].dropped = true;
        }
    }
    let live = |a: StatusAlign| -> Vec<&StatusEntry> {
        entries
            .iter()
            .filter(|e| !e.dropped && e.align == a)
            .collect()
    };
    let styled = |e: &StatusEntry| {
        if e.kind == StatusKind::Chord {
            overlay_style(base, theme.status_chord)
        } else {
            base
        }
    };
    if status_total(&entries) > cols {
        // 優先度 5 だけでも入らない: 左群・右群の順に並べて末尾を省略記号で切る
        let mut line: Vec<Span> = Vec::new();
        for e in live(StatusAlign::Left)
            .into_iter()
            .chain(live(StatusAlign::Right))
        {
            if !line.is_empty() {
                line.push(ascii_span(" ", base));
            }
            let st = styled(e);
            line.extend(
                e.spans
                    .iter()
                    .map(|s| Span::new(s.text.clone(), s.width, overlay_style(st, s.style))),
            );
        }
        let line = truncate_end(&line, cols, &glyphs.ellipsis);
        put(w, 0, y, cols, &line, base);
        return;
    }
    let mut x = 0;
    for e in live(StatusAlign::Left) {
        x = put(w, x, y, cols, &e.spans, styled(e));
        x += ITEM_GAP;
    }
    let right = live(StatusAlign::Right);
    let rw: u16 = right.iter().map(|e| text_width(&e.spans)).sum::<u16>()
        + right.len().saturating_sub(1) as u16 * ITEM_GAP;
    let mut x = cols - rw;
    for e in right {
        x = put(w, x, y, cols, &e.spans, styled(e));
        x += ITEM_GAP;
    }
}
