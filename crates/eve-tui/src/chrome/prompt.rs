//! 入力プロンプト (04 §8.2)。

use eve_view::{CellPos, CellSize, OverlayModel};

use super::input_field::draw_input;
use super::overlay::{BoxRect, box_width, draw_box, draw_line};
use crate::buffer::Writer;
use crate::glyphs::Glyphs;
use crate::theme::Theme;

pub(super) fn render_prompt(
    w: &mut Writer<'_>,
    area: CellSize,
    overlay: &OverlayModel,
    theme: &Theme,
    glyphs: &Glyphs,
) -> Option<CellPos> {
    let OverlayModel::Prompt {
        title,
        input,
        hint,
        error,
    } = overlay
    else {
        return None;
    };
    let style = theme.overlay;
    let (bw, mut bordered) = box_width(60, area.cols);
    let mut show_hint = hint.is_some();
    let mut show_error = error.is_some();
    let rows_for = |bordered: bool, h: bool, e: bool| {
        u16::from(bordered) * 2 + 1 + u16::from(h) + u16::from(e)
    };
    if rows_for(bordered, show_hint, show_error) > area.rows {
        show_hint = false;
    }
    if rows_for(bordered, show_hint, show_error) > area.rows {
        bordered = false;
    }
    if rows_for(bordered, show_hint, show_error) > area.rows {
        show_error = false;
    }
    let rows = rows_for(bordered, show_hint, show_error).min(area.rows);
    let b = BoxRect {
        x: (area.cols - bw) / 2,
        y: u16::from(area.rows > rows + 1).min(area.rows - rows),
        cols: bw,
        rows,
        bordered,
    };
    draw_box(w, &b, bordered.then_some(title), style, glyphs);
    let (iw, ix, iy) = (b.inner_cols(), b.inner_x(), b.inner_y());
    let cursor_x = draw_input(w, input, ix, iy, iw, theme, glyphs);
    let mut row = 1;
    if show_error && let Some(err) = error {
        draw_line(w, &b, row, &err.spans, theme.message_error, glyphs);
        row += 1;
    }
    if show_hint && let Some(h) = hint {
        draw_line(w, &b, row, &h.spans, theme.placeholder, glyphs);
    }
    Some(CellPos {
        col: ix + cursor_x.min(iw.saturating_sub(1)),
        row: iy,
    })
}
