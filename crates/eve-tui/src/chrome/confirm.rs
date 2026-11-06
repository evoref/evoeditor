//! 確認ダイアログ (04 §8.3)。

use eve_view::{CellPos, CellSize, Choice, OverlayModel};

use super::overlay::{BoxRect, box_width, button_spans, draw_box, draw_line};
use super::parts::{glyph_span, put, text_width, truncate_end};
use crate::buffer::Writer;
use crate::glyphs::Glyphs;
use crate::theme::Theme;

pub(super) fn render_confirm(
    w: &mut Writer<'_>,
    area: CellSize,
    overlay: &OverlayModel,
    theme: &Theme,
    glyphs: &Glyphs,
) -> Option<CellPos> {
    let OverlayModel::Confirm {
        title,
        message,
        details,
        choices,
        focused,
        nav_hint,
        ..
    } = overlay
    else {
        return None;
    };
    let style = theme.overlay;
    let (mut buttons, mut focus_off) = button_spans(choices, *focused, theme, glyphs);
    let detail_rows = details.as_deref().unwrap_or(&[]);
    let nav = nav_hint.as_ref();
    let want_inner = [
        message.width,
        text_width(&buttons),
        nav.map_or(0, |n| n.width),
        detail_rows.iter().map(|d| d.width).max().unwrap_or(0),
        title.width,
    ]
    .into_iter()
    .max()
    .unwrap_or(0);
    let (bw, mut bordered) = box_width(want_inner, area.cols);
    // 縦 (04 §3.5, OQ-182): 足りないときはボタン行と案内行を優先し、本文を切る。枠も省く
    let nav_n = u16::from(nav.is_some());
    let det_n = detail_rows.len() as u16;
    let min_bordered = 2 + 1 + 1 + 1 + nav_n; // 枠・本文・空行・ボタン・案内 (詳細なし)
    let (show_message, show_details, blank, show_nav);
    if bordered && area.rows >= min_bordered {
        show_message = true;
        blank = true;
        show_nav = nav.is_some();
        show_details = det_n.min(area.rows - min_bordered);
    } else {
        bordered = false;
        blank = false;
        let mut left = area.rows;
        left = left.saturating_sub(1); // ボタン行
        show_nav = nav.is_some() && left >= 1;
        left = left.saturating_sub(u16::from(show_nav));
        show_message = left >= 1;
        left = left.saturating_sub(u16::from(show_message));
        show_details = det_n.min(left);
    }
    let rows_needed = u16::from(bordered) * 2
        + u16::from(show_message)
        + show_details
        + u16::from(blank)
        + 1
        + u16::from(show_nav);
    let b = BoxRect {
        x: (area.cols - bw) / 2,
        y: (area.rows - rows_needed) / 2,
        cols: bw,
        rows: rows_needed,
        bordered,
    };
    draw_box(w, &b, bordered.then_some(title), style, glyphs);
    // ボタン行が幅に入らない: フォーカス中のボタンと、最後の選択肢 (Esc) だけを残す (04 §3.5)
    if text_width(&buttons) > b.inner_cols() {
        let keep: Vec<Choice> = choices
            .iter()
            .filter(|c| c.id == *focused)
            .chain(choices.last().filter(|l| l.id != *focused))
            .cloned()
            .collect();
        (buttons, focus_off) = button_spans(&keep, *focused, theme, glyphs);
    }
    let mut row = 0;
    if show_message {
        draw_line(w, &b, row, &message.spans, style, glyphs);
        row += 1;
    }
    for (i, d) in detail_rows.iter().take(show_details as usize).enumerate() {
        let last_cut = i as u16 + 1 == show_details && show_details < detail_rows.len() as u16;
        let spans = if last_cut {
            // 描けなかった行がある: 最後の行を省略記号にする
            let mut s = truncate_end(
                &d.spans,
                b.inner_cols().saturating_sub(glyphs.ellipsis.width),
                &glyphs.ellipsis,
            );
            if s.last().is_none_or(|l| l.text != glyphs.ellipsis.text) {
                s.push(glyph_span(&glyphs.ellipsis, style));
            }
            s
        } else {
            d.spans.clone()
        };
        draw_line(w, &b, row, &spans, style, glyphs);
        row += 1;
    }
    if blank {
        row += 1;
    }
    let button_row = row;
    let by = b.inner_y() + button_row;
    put(
        w,
        b.inner_x(),
        by,
        b.inner_x() + b.inner_cols(),
        &buttons,
        style,
    );
    row += 1;
    if show_nav && let Some(n) = nav {
        draw_line(w, &b, row, &n.spans, style, glyphs);
    }
    // ハードウェアカーソルはフォーカス中のボタンの先頭 (04 §8.3、IME の候補窓の位置)
    Some(CellPos {
        col: b.inner_x() + focus_off,
        row: by,
    })
}
