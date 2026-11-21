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
        let gaps = crate::cells(ws.len().saturating_sub(1)).saturating_mul(ITEM_GAP);
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

/// 項目の並び。`left_only` なら全部を左群として扱う (ペインの見出し。種類と配置で描き分けない、02 §8.2)。
fn status_entries(items: &[StatusItem], left_only: bool) -> Vec<StatusEntry> {
    items
        .iter()
        .enumerate()
        .map(|(i, it): (usize, &StatusItem)| {
            let align = if left_only {
                StatusAlign::Left
            } else {
                it.align
            };
            StatusEntry {
                kind: it.kind,
                align,
                priority: it.priority,
                spans: it.long.spans.clone(),
                short: it.short.as_ref().map(|s| s.spans.clone()),
                middle: it.long.cut == CutPolicy::Middle,
                dropped: false,
                rank: usize::from(align == StatusAlign::Right) * items.len() + i,
            }
        })
        .collect()
}

/// 項目を幅 `cols` に収める (04 §8.7 の省略の (1) 短い形 → (2) 名前の中間の省略 → (3) 優先度 5 未満を落とす)。
/// それでも入らなければ、呼び側が末尾を省略記号で切る。
fn shrink(entries: &mut [StatusEntry], cols: u16, glyphs: &Glyphs) {
    let order = status_order(entries);
    // (1) 短い形にする
    for &i in &order {
        if status_total(entries) <= cols {
            break;
        }
        if let Some(s) = entries[i].short.take() {
            entries[i].spans = s;
        }
    }
    // (2) 中間を省略できる項目 (ファイル名) を縮める
    let total = status_total(entries);
    if total > cols {
        let excess = total - cols;
        if let Some(e) = entries.iter_mut().find(|e| e.middle && !e.dropped) {
            let want = text_width(&e.spans).saturating_sub(excess).max(5);
            e.spans = truncate_middle(&e.spans, want, &glyphs.ellipsis);
        }
    }
    // (3) 項目を落とす (優先度 5 は落とさない)
    for &i in &order {
        if status_total(entries) <= cols {
            break;
        }
        if entries[i].priority < 5 {
            entries[i].dropped = true;
        }
    }
}

/// ペインの見出しの項目を、ステータスバーの左群と同じ規則で幅 `cols` に収めた 1 行 (04 §3.4 (f)、§8.7)。項目の間は
/// 1 セル。優先度 5 の項目だけでも入らなければ、末尾を省略記号で切る。各 `Span` のスタイルは項目のまま (呼び側が地の
/// スタイルに重ねる)。
pub(crate) fn fit_bar_items(items: &[StatusItem], cols: u16, glyphs: &Glyphs) -> Vec<Span> {
    let mut entries = status_entries(items, true);
    shrink(&mut entries, cols, glyphs);
    let mut line: Vec<Span> = Vec::new();
    for e in entries.iter().filter(|e| !e.dropped) {
        if !line.is_empty() {
            line.push(ascii_span(" ", eve_view::Style::DEFAULT));
        }
        line.extend(e.spans.iter().cloned());
    }
    truncate_end(&line, cols, &glyphs.ellipsis)
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
    let mut entries = status_entries(&model.items, false);
    shrink(&mut entries, cols, glyphs);
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
        + crate::cells(right.len().saturating_sub(1)).saturating_mul(ITEM_GAP);
    let mut x = cols - rw;
    for e in right {
        x = put(w, x, y, cols, &e.spans, styled(e));
        x += ITEM_GAP;
    }
}
