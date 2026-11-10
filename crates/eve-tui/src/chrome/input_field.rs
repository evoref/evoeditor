//! 入力欄 (オーバーレイと検索バーで共有する)。

use eve_view::InputFieldModel;

use super::parts::{glyph_span, put, truncate_end};
use crate::buffer::Writer;
use crate::glyphs::Glyphs;
use crate::theme::{Theme, overlay_style};

/// 入力欄の表示: カーソルの周りを幅に収める。返り値は (描く範囲の最初と最後+1、左右の続き、カーソルまでの幅)。
pub(super) fn field_window(f: &InputFieldModel, width: u16) -> (usize, usize, bool, bool, u16) {
    let cur = f.cursor.saturating_sub(f.first).min(f.cells.len());
    // 左に戻れる幅: カーソルのセルと、左右の続きの印のぶんを残す
    let room = width.saturating_sub(1);
    let mut start = cur;
    let mut used = 0u16;
    while start > 0 {
        let wd = f.cells[start - 1].width;
        let more_left = start - 1 > 0 || f.first > 0;
        let reserve = u16::from(more_left);
        if used + wd + reserve > room.saturating_sub(1) && start != cur {
            break;
        }
        if used + wd + reserve > room {
            break;
        }
        used += wd;
        start -= 1;
    }
    let clip_left = start > 0 || f.first > 0;
    let off = u16::from(clip_left);
    let mut end = start;
    let mut x = off;
    while end < f.cells.len() {
        let wd = f.cells[end].width;
        let more_right = end + 1 < f.cells.len() || f.first + f.cells.len() < f.len;
        if x + wd + u16::from(more_right) > width && end >= cur {
            break;
        }
        if x + wd > width {
            break;
        }
        x += wd;
        end += 1;
    }
    let clip_right = end < f.cells.len() || f.first + f.cells.len() < f.len;
    let cursor_x = off
        + f.cells[start..cur.max(start)]
            .iter()
            .map(|s| s.width)
            .sum::<u16>();
    (start, end, clip_left, clip_right, cursor_x)
}

/// 入力欄を 1 行に描く。カーソルの、欄の左端からのセル位置を返す。
pub(super) fn draw_input(
    w: &mut Writer<'_>,
    input: &InputFieldModel,
    ix: u16,
    iy: u16,
    iw: u16,
    theme: &Theme,
    glyphs: &Glyphs,
) -> u16 {
    let style = theme.overlay;
    let limit = ix + iw;
    let mut cursor_x = 0;
    if input.cells.is_empty() {
        if let Some(p) = &input.placeholder {
            let ph = truncate_end(&p.spans, iw, &glyphs.ellipsis);
            put(w, ix, iy, limit, &ph, theme.placeholder);
        }
    } else {
        let (s, e, left, right, cx) = field_window(input, iw);
        cursor_x = cx;
        let mut x = ix;
        if left {
            x = w.span(
                x,
                iy,
                limit,
                &glyph_span(&glyphs.clip_left, theme.scroll_marker),
            );
        }
        for (k, sp) in input.cells[s..e].iter().enumerate() {
            let idx = input.first + s + k;
            let selected = input.selection.as_ref().is_some_and(|r| r.contains(&idx));
            let st = if selected {
                overlay_style(style, theme.selection)
            } else {
                style
            };
            x = put(w, x, iy, limit, std::slice::from_ref(sp), st);
        }
        if right {
            w.span(
                limit - 1,
                iy,
                limit,
                &glyph_span(&glyphs.clip_right, theme.scroll_marker),
            );
        }
    }
    cursor_x
}
