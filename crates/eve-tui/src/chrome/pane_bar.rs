//! ペインの見出しと区切り (04 §3.4 (f)、§3.7、§12.4、07 §8.2)。セルは core が確保して Frame で渡す
//! (`PaneFrame.bar.rect`・`Frame.dividers`)。TUI は項目の省略 (ステータスバーの左群と同じ規則) と記号だけを決める。
//!
//! - フォーカスのあるペイン (`PaneFrame.active`): 見出しの全体を `ui.pane.bar.active` (無色の段は反転) の帯にし、先頭に
//!   `▶` (ASCII の縮退では `>`) と空白 1 つ、続けて項目を置く。
//! - ほかのペイン: 先頭に空白 1 つ、項目を `ui.pane.bar` で置き、空白 1 つの後ろの残りを `─` (`-`) で埋める
//!   (`ui.pane.border`)。
//! - 区切り: 1 列を `│` (`|`) で埋める (`ui.pane.border`)。
//!
//! 無色の段と ASCII の縮退を重ねても、「反転の帯」と「`-` で埋めた行」で区別できる (FR-509 AC-1)。

use eve_view::{Divider, PaneFrame, Span};

use super::parts::{fill, put, text_width};
use super::status::fit_bar_items;
use crate::buffer::Writer;
use crate::glyphs::Glyphs;
use crate::theme::Theme;

/// ペイン `pane` の見出しを描く (見出しが無ければ何もしない)。`top` は編集面を置いた画面の行 (タブラインの下)。
pub(crate) fn render_pane_bar(
    w: &mut Writer<'_>,
    pane: &PaneFrame,
    top: u16,
    theme: &Theme,
    glyphs: &Glyphs,
) {
    let Some(bar) = &pane.bar else {
        return;
    };
    let r = bar.rect;
    if r.rows == 0 || r.cols == 0 {
        return;
    }
    let (x0, y, limit) = (r.col, top + r.row, r.col.saturating_add(r.cols));
    if pane.active {
        let base = theme.pane_bar_active;
        fill(w, x0, y, r.cols, base);
        let mark = [Span::new(glyphs.focus.text, glyphs.focus.width, base)];
        let mut x = put(w, x0, y, limit, &mark, base);
        x = x.saturating_add(1).min(limit);
        let items = fit_bar_items(&bar.items, limit - x, glyphs);
        put(w, x, y, limit, &items, base);
    } else {
        fill(w, x0, y, r.cols, theme.pane_bar);
        let x = x0.saturating_add(1).min(limit);
        // 項目の後ろに空白と少なくとも 1 つの `─` を残す
        let room = (limit - x).saturating_sub(2);
        let items = fit_bar_items(&bar.items, room, glyphs);
        let mut x = put(w, x, y, limit, &items, theme.pane_bar);
        if text_width(&items) > 0 {
            x = x.saturating_add(1).min(limit);
        }
        let style = w.style(theme.pane_border);
        while x < limit {
            w.set(x, y, glyphs.boxed.horizontal.text, style, false);
            x += 1;
        }
    }
}

/// 区切りを描く。`top` は編集面を置いた画面の行。
pub(crate) fn render_divider(
    w: &mut Writer<'_>,
    d: &Divider,
    top: u16,
    theme: &Theme,
    glyphs: &Glyphs,
) {
    let style = w.style(theme.pane_border);
    let r = d.rect;
    let y0 = top + r.row;
    for y in y0..y0.saturating_add(r.rows) {
        for x in r.col..r.col.saturating_add(r.cols) {
            w.set(x, y, glyphs.boxed.vertical.text, style, false);
        }
    }
}
