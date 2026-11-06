//! リスト選択 (コマンドパレット、04 §8.1)。

use eve_view::{CellPos, CellSize, OverlayModel, Span};

use super::input_field::draw_input;
use super::keys::key_label_full;
use super::overlay::{BoxRect, box_width, draw_box};
use super::parts::{ascii_span, fill, glyph_span, put, text_width, truncate_end};
use crate::buffer::Writer;
use crate::glyphs::Glyphs;
use crate::theme::{Theme, overlay_style};

/// リスト選択 (コマンドパレット、04 §8.1)。上部中央。入力欄 1 行 + 候補 (最大 12 行) + 枠。
pub(super) fn render_list(
    w: &mut Writer<'_>,
    area: CellSize,
    overlay: &OverlayModel,
    theme: &Theme,
    glyphs: &Glyphs,
) -> Option<CellPos> {
    let OverlayModel::List {
        title,
        input,
        items,
        first,
        selected,
        footer,
        no_match,
    } = overlay
    else {
        return None;
    };
    let style = theme.overlay;
    let ascii = glyphs.ellipsis.width == 3;
    let (bw, mut bordered) = box_width(76, area.cols);
    let body_rows = items.len().max(1) as u16;
    let rows_for = |bordered: bool, n: u16| u16::from(bordered) * 2 + 1 + n;
    if rows_for(bordered, body_rows) > area.rows {
        bordered = false;
    }
    let n = body_rows.min(area.rows.saturating_sub(rows_for(bordered, 0)));
    let rows = rows_for(bordered, n).min(area.rows);
    let b = BoxRect {
        x: (area.cols - bw) / 2,
        y: u16::from(area.rows > rows + 1).min(area.rows - rows),
        cols: bw,
        rows,
        bordered,
    };
    draw_box(w, &b, bordered.then_some(title), style, glyphs);
    let (iw, ix, iy) = (b.inner_cols(), b.inner_x(), b.inner_y());
    let limit = ix + iw;
    // 入力欄 (空のときは placeholder)
    let cursor_x = if input.cells.is_empty() {
        if let Some(p) = &input.placeholder {
            let ph = truncate_end(&p.spans, iw, &glyphs.ellipsis);
            put(w, ix, iy, limit, &ph, theme.placeholder);
        }
        0
    } else {
        draw_input(w, input, ix, iy, iw, theme, glyphs)
    };
    if items.is_empty() {
        let t = truncate_end(&no_match.spans, iw, &glyphs.ellipsis);
        put(w, ix, iy + 1, limit, &t, theme.placeholder);
    }
    for (k, item) in items.iter().take(n as usize).enumerate() {
        let y = iy + 1 + k as u16;
        let is_sel = *first + k == *selected;
        let disabled = item.disabled_reason.is_some();
        let base = if disabled {
            overlay_style(style, theme.overlay_disabled)
        } else {
            style
        };
        let base = if is_sel {
            overlay_style(base, theme.overlay_selected)
        } else {
            base
        };
        fill(w, ix, y, iw, base);
        // 先頭の印 (選択中は ▶。色に頼らない)
        let mut x = if is_sel {
            let x = w.span(ix, y, limit, &glyph_span(&glyphs.focus, base));
            w.span(x, y, limit, &ascii_span(" ", base))
        } else {
            ix + 2
        };
        // 右側: 実行できないときは理由、そうでなければ CommandId とキー
        let mut right: Vec<Span> = Vec::new();
        if let Some(reason) = &item.disabled_reason {
            right.extend(reason.spans.iter().cloned());
        } else {
            if let Some(d) = &item.detail {
                right.extend(d.spans.iter().cloned());
            }
            if let Some(k) = item.keys.first() {
                if !right.is_empty() {
                    right.push(ascii_span("  ", base));
                }
                right.push(ascii_span(&key_label_full(k, ascii), base));
            }
        }
        let left_room = limit.saturating_sub(x);
        let label_w = text_width(&item.label.spans);
        // 右側は、ラベルの後に 2 セル空けて入るときだけ出す (狭いと省略)
        let right_w = text_width(&right);
        let show_right = right_w > 0 && label_w + 2 + right_w <= left_room;
        let label_room = if show_right {
            left_room - right_w - 2
        } else {
            left_room
        };
        let label = truncate_end(&item.label.spans, label_room, &glyphs.ellipsis);
        for (i, sp) in label.iter().enumerate() {
            let hit = item.match_ranges.iter().any(|r| r.contains(&i));
            let st = if hit {
                overlay_style(base, theme.overlay_match)
            } else {
                base
            };
            x = put(w, x, y, limit, std::slice::from_ref(sp), st);
        }
        if show_right {
            put(
                w,
                limit - right_w,
                y,
                limit,
                &right,
                if is_sel { base } else { theme.placeholder },
            );
        }
    }
    // 件数は下辺に右寄せ
    if bordered && b.cols > 8 {
        let y1 = b.y + b.rows - 1;
        let fw = text_width(&footer.spans).min(b.cols.saturating_sub(8));
        let fs = truncate_end(&footer.spans, fw, &glyphs.ellipsis);
        let x = b.x + b.cols - 3 - fw;
        let x = w.span(x - 1, y1, b.x + b.cols - 1, &ascii_span(" ", style));
        let x = put(w, x, y1, b.x + b.cols - 1, &fs, style);
        w.span(x, y1, b.x + b.cols - 1, &ascii_span(" ", style));
    }
    Some(CellPos {
        col: ix + cursor_x.min(iw.saturating_sub(1)),
        row: iy,
    })
}
