//! 検索バー (04 §8.4)。

use eve_view::{BarFocus, CaseMode, CellPos, SearchBarModel, Span};

use super::input_field::draw_input;
use super::parts::{ascii_span, fill, put, text_width};
use crate::buffer::Writer;
use crate::glyphs::Glyphs;
use crate::theme::{Theme, overlay_style};

/// 検索バー (04 §8.4): `検索: [入力欄] [Aa~] [.*] [ab] [sel] 1/2`。幅が足りなければ件数 → トグルの括弧の順に省き、入力欄は最低 10 セルを残す。
pub(super) fn render_search_bar(
    w: &mut Writer<'_>,
    y: u16,
    cols: u16,
    bar: &SearchBarModel,
    theme: &Theme,
    glyphs: &Glyphs,
) -> Option<CellPos> {
    let style = theme.overlay;
    let ascii = glyphs.ellipsis.width == 3;
    fill(w, 0, y, cols, style);
    let mark = match bar.case {
        CaseMode::Smart => "~",
        CaseMode::Sensitive if ascii => "*",
        CaseMode::Sensitive => "\u{2713}",
        CaseMode::Insensitive => "",
    };
    let on_mark = if ascii { "*" } else { "\u{2713}" };
    // トグル: 大小 `[Aa~]`、正規表現 `[.*]`、単語単位 `[ab]`、選択範囲内 `[sel]` (オンは ✓ を付けて反転。色に頼らない、04 §8.4)
    let toggles: [(String, String, bool); 4] = [
        (format!("[Aa{mark}]"), format!("Aa{mark}"), bar.case_on),
        (
            format!("[.*{}]", if bar.regex { on_mark } else { "" }),
            format!(".*{}", if bar.regex { on_mark } else { "" }),
            bar.regex,
        ),
        (
            format!("[ab{}]", if bar.whole_word { on_mark } else { "" }),
            format!("ab{}", if bar.whole_word { on_mark } else { "" }),
            bar.whole_word,
        ),
        (
            format!("[sel{}]", if bar.in_selection { on_mark } else { "" }),
            format!("sel{}", if bar.in_selection { on_mark } else { "" }),
            bar.in_selection,
        ),
    ];
    let width_of = |short: bool| -> u16 {
        toggles
            .iter()
            .map(|t| crate::cells((if short { &t.1 } else { &t.0 }).chars().count()))
            .sum::<u16>()
            // トグルの間の空白
            + (crate::cells(toggles.len()) - 1)
    };
    let label_w = text_width(&bar.label.spans);
    // 右端には件数を置く。検索語を使えないときは、件数の代わりに理由を出す
    let right_text = bar.error.as_ref().or(bar.count_text.as_ref());
    let count_w = right_text.map_or(0, |c| text_width(&c.spans));
    // 入力欄の最低幅を残して、入る構成を選ぶ
    let fits = |toggles_w: u16, count: u16| {
        let need = label_w + 1 + 10 + 1 + toggles_w + if count > 0 { 1 + count } else { 0 };
        need <= cols
    };
    let (short_toggles, show_count) = if fits(width_of(false), count_w) {
        (false, true)
    } else if fits(width_of(false), 0) {
        (false, false)
    } else {
        (true, false)
    };
    let toggles_w = width_of(short_toggles);
    let mut x = put(w, 0, y, cols, &bar.label.spans, style);
    x = w.span(x, y, cols, &ascii_span(" ", style));
    let right_w = 1 + toggles_w + if show_count { 1 + count_w } else { 0 };
    let iw = cols.saturating_sub(x + right_w).max(1);
    let ix = x;
    let cursor_x = draw_input(w, &bar.input, ix, y, iw, theme, glyphs);
    // 右側: トグル (有効なものは反転) と件数 / 理由
    let mut rx = ix + iw;
    rx = w.span(rx, y, cols, &ascii_span(" ", style));
    for (i, (full, short, on)) in toggles.iter().enumerate() {
        if i > 0 {
            rx = w.span(rx, y, cols, &ascii_span(" ", style));
        }
        let text = if short_toggles { short } else { full };
        let tstyle = if *on {
            overlay_style(style, theme.toggle_on)
        } else {
            style
        };
        let ts = Span::new(text.as_str(), crate::cells(text.chars().count()), tstyle);
        rx = w.span(rx, y, cols, &ts);
    }
    if show_count && let Some(c) = right_text {
        rx = w.span(rx, y, cols, &ascii_span(" ", style));
        let cstyle = if bar.error.is_some() {
            theme.message_error
        } else {
            style
        };
        put(w, rx, y, cols, &c.spans, cstyle);
    }
    let search_cursor = CellPos {
        col: ix + cursor_x.min(iw.saturating_sub(1)),
        row: y,
    };
    let Some(rep) = &bar.replace else {
        return Some(search_cursor);
    };
    // 2 行目: `置換: [入力欄]   [A] すべて置換` (幅が足りなければボタンを `[A]` だけにする)
    let y2 = y + 1;
    fill(w, 0, y2, cols, style);
    let mut x = put(w, 0, y2, cols, &rep.label.spans, style);
    x = w.span(x, y2, cols, &ascii_span(" ", style));
    let full_w = text_width(&rep.button.spans);
    let button_w = if x + 10 + 1 + full_w <= cols {
        full_w
    } else {
        3
    };
    let button: Vec<Span> = if button_w == full_w {
        rep.button.spans.clone()
    } else {
        vec![ascii_span("[A]", style)]
    };
    let riw = cols.saturating_sub(x + 1 + button_w).max(1);
    let rix = x;
    let rcursor_x = draw_input(w, &rep.input, rix, y2, riw, theme, glyphs);
    let bx = (rix + riw + 1).min(cols.saturating_sub(button_w));
    let bstyle = if bar.focus == BarFocus::Button {
        theme.dialog_button_focused
    } else {
        theme.dialog_button
    };
    put(w, bx, y2, cols, &button, overlay_style(style, bstyle));
    Some(match bar.focus {
        BarFocus::Search => search_cursor,
        BarFocus::Replace => CellPos {
            col: rix + rcursor_x.min(riw.saturating_sub(1)),
            row: y2,
        },
        BarFocus::Button => CellPos { col: bx, row: y2 },
    })
}
