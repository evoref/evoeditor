//! `Frame` → ratatui `Buffer` の変換 (docs/04 §4.2)。ratatui のウィジェットは使わず、`Cell` に 1 書記素ずつ書く。
//!
//! 幅 2 以上の書記素は、先頭セルに書記素、後続セルを「継続セル」(`CellDiffOption::Skip`) にする (S-06 で、この前提で
//! 差分出力が崩れないことを確認した)。幅は core が `Span.width` で与えた値に従い、幅関数は持たない。
//!
//! `render_panes` は編集面 (ペイン) とカーソル位置を書き、バッファはクロームの行ぶんを確保して空のままにする。
//! クローム (ステータス・メッセージ・ヒント) とオーバーレイは `chrome::render_screen` が足す (E5)。

use eve_view::{
    CellPos, Color, ColorDepth, Frame, PaneFrame, RenderRow, RowSource, Span, Style, StyleMods,
};
use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::Rect;
use ratatui::style::{Color as RColor, Modifier, Style as RStyle};

use crate::color::clamp_style;
use crate::glyphs::Glyphs;
use crate::sanitize::replace_controls;
use crate::theme::Theme;

/// 編集面の下のクロームの行数 (ステータスバー・メッセージ行・ヒントバー、04 §3.1)。
pub const CHROME_ROWS: u16 = 3;

/// 編集面が `surface_rows` 行のときのクロームの行数。端末が 10 行未満 (= 編集面が 8 行未満) のときは、メッセージ行を
/// 確保せず 2 行にする (04 §3.5。メッセージは表示中だけヒントバーの行を使う)。
pub const fn chrome_rows_for_surface(surface_rows: u16) -> u16 {
    if surface_rows < 8 {
        CHROME_ROWS - 1
    } else {
        CHROME_ROWS
    }
}

/// 端末が `window_rows` 行のときの編集面の行数 (`chrome_rows_for_surface` の逆)。
pub const fn surface_rows_for_window(window_rows: u16) -> u16 {
    if window_rows < 10 {
        window_rows.saturating_sub(CHROME_ROWS - 1)
    } else {
        window_rows - CHROME_ROWS
    }
}

/// 変換の結果。
#[derive(Debug)]
pub struct PaneRender {
    /// 端末全体 (編集面 + クローム) の大きさのバッファ。
    pub buffer: Buffer,
    /// ハードウェアカーソルの位置 (編集面にフォーカスがあるときだけ)。
    pub cursor: Option<CellPos>,
    /// 幅危険書記素を含む行 (バッファの行番号)。変化したときは行全体を再出力する (04 §4.7 (c))。
    pub risky_rows: Vec<u16>,
    /// 段を超えていて近似したセルの数 (debug ログ用。C1 の最終防衛線)。
    pub clamped_cells: usize,
    /// 制御文字を置換した個数 (04 §4.9。本文は記録しない)。
    pub sanitized: usize,
}

/// `eve_view::Style` → ratatui の `Style`。
pub fn to_ratatui(style: Style) -> RStyle {
    let conv = |c: Color| match c {
        Color::Rgb(r, g, b) => RColor::Rgb(r, g, b),
        Color::Indexed(i) => RColor::Indexed(i),
        _ => RColor::Reset,
    };
    let mut s = RStyle::default().fg(conv(style.fg)).bg(conv(style.bg));
    let m = style.mods;
    for (flag, modifier) in [
        (StyleMods::BOLD, Modifier::BOLD),
        (StyleMods::ITALIC, Modifier::ITALIC),
        (StyleMods::UNDERLINE, Modifier::UNDERLINED),
        (StyleMods::REVERSE, Modifier::REVERSED),
        (StyleMods::DIM, Modifier::DIM),
    ] {
        if m.contains(flag) {
            s = s.add_modifier(modifier);
        }
    }
    s
}

pub(crate) struct Writer<'a> {
    pub(crate) buf: &'a mut Buffer,
    pub(crate) depth: ColorDepth,
    pub(crate) clamped: usize,
    pub(crate) sanitized: usize,
}

impl Writer<'_> {
    pub(crate) fn style(&mut self, s: Style) -> RStyle {
        let (s, changed) = clamp_style(s, self.depth);
        if changed {
            self.clamped += 1;
        }
        to_ratatui(s)
    }

    pub(crate) fn set(&mut self, x: u16, y: u16, symbol: &str, style: RStyle, skip: bool) {
        self.break_wide(x, y);
        if let Some(cell) = self.buf.cell_mut((x, y)) {
            cell.set_symbol(symbol).set_style(style);
            cell.set_diff_option(if skip {
                CellDiffOption::Skip
            } else {
                CellDiffOption::None
            });
        }
    }

    /// (x, y) に書くことで幅 2 以上の書記素が分断されるとき、残る側を空白にする (端末に半分だけの全角文字を残さない)。
    fn break_wide(&mut self, x: u16, y: u16) {
        let Some(cell) = self.buf.cell((x, y)) else {
            return;
        };
        if cell.diff_option == CellDiffOption::Skip {
            // 継続セルを上書きする: 先頭セルまでさかのぼって空白にする
            let mut hx = x;
            while hx > 0 {
                hx -= 1;
                let blank_style = self.buf.cell((hx, y)).map(|c| c.style());
                let is_skip = self
                    .buf
                    .cell((hx, y))
                    .is_some_and(|c| c.diff_option == CellDiffOption::Skip);
                if let (Some(st), Some(c)) = (blank_style, self.buf.cell_mut((hx, y))) {
                    c.set_symbol(" ").set_style(st);
                    c.set_diff_option(CellDiffOption::None);
                }
                if !is_skip {
                    break;
                }
            }
        }
        // 先頭セルを上書きする: 後ろに続く継続セルを空白にする
        let mut nx = x + 1;
        while self
            .buf
            .cell((nx, y))
            .is_some_and(|c| c.diff_option == CellDiffOption::Skip)
        {
            if let Some(c) = self.buf.cell_mut((nx, y)) {
                c.set_symbol(" ");
                c.set_diff_option(CellDiffOption::None);
            }
            nx += 1;
        }
    }

    /// `limit` (含まない) までに収めて Span を書く。次に書く位置を返す。
    pub(crate) fn span(&mut self, mut x: u16, y: u16, limit: u16, span: &Span) -> u16 {
        if span.width == 0 || x >= limit {
            return x;
        }
        let (text, n) = replace_controls(&span.text);
        self.sanitized += n;
        let style = self.style(span.style);
        // ASCII の連続 (幅 = バイト数) は 1 文字 1 セル。それ以外は 1 書記素 (core の Span の契約)
        if span.width as usize == text.len() && text.bytes().all(|b| (0x20..0x7F).contains(&b)) {
            for ch in text.chars() {
                if x >= limit {
                    break;
                }
                let mut tmp = [0u8; 4];
                self.set(x, y, ch.encode_utf8(&mut tmp), style, false);
                x += 1;
            }
            return x;
        }
        if x + span.width > limit {
            // 全角が矩形の端をまたぐときは、防御的に 1 セルずつ空白で埋める (04 §3.5)
            while x < limit {
                self.set(x, y, " ", style, false);
                x += 1;
            }
            return x;
        }
        self.set(x, y, &text, style, false);
        for k in 1..span.width {
            self.set(x + k, y, " ", style, true);
        }
        x + span.width
    }
}

fn render_row(
    w: &mut Writer<'_>,
    pane: &PaneFrame,
    i: u16,
    row: &RenderRow,
    glyphs: &Glyphs,
    theme: &Theme,
) {
    let r = pane.rect;
    let y = r.row + i;
    let limit = r.col + r.cols;
    let mut x = r.col;
    if matches!(row.source, RowSource::Virtual) {
        // 文書末尾より後の行 (ロール ui.eof_marker)
        let style = w.style(theme.eof_marker);
        w.set(x, y, glyphs.eof_line.text, style, false);
        return;
    }
    let gutter_w: u16 = row.gutter.iter().map(|s| s.width).sum();
    for sp in &row.gutter {
        x = w.span(x, y, limit, sp);
    }
    if gutter_w > 0
        && let Some(last) = row.gutter.last()
    {
        // ガターの最後のセルは区切り (core は空白で渡す。区切りの記号は eve-tui の対応表、04 §3.7)
        let style = w.style(last.style);
        w.set(
            r.col + gutter_w - 1,
            y,
            glyphs.gutter_sep.text,
            style,
            false,
        );
    }
    for sp in &row.spans {
        x = w.span(x, y, limit, sp);
    }
}

/// Frame の編集面を Buffer に書く。
pub fn render_panes(frame: &Frame, depth: ColorDepth, glyphs: &Glyphs) -> PaneRender {
    let theme = Theme::for_depth(depth);
    // 検索バーは編集面の下に 1 行取る (編集面の行数 `size.rows` はその分を引いてある)
    let base = frame.size.rows + u16::from(frame.search.is_some());
    let (cols, rows) = (frame.size.cols, base + chrome_rows_for_surface(base));
    let mut buffer = Buffer::empty(Rect::new(0, 0, cols, rows));
    let mut risky_rows = Vec::new();
    let (clamped_cells, sanitized);
    {
        let mut w = Writer {
            buf: &mut buffer,
            depth,
            clamped: 0,
            sanitized: 0,
        };
        for pane in &frame.panes {
            for (i, row) in pane.rows.iter().enumerate() {
                let i = i as u16;
                if i >= pane.rect.rows {
                    break;
                }
                if row.width_risky {
                    risky_rows.push(pane.rect.row + i);
                }
                render_row(&mut w, pane, i, row, glyphs, &theme);
            }
        }
        clamped_cells = w.clamped;
        sanitized = w.sanitized;
    }
    PaneRender {
        buffer,
        cursor: frame.primary_cursor.map(|c| c.pos),
        risky_rows,
        clamped_cells,
        sanitized,
    }
}

/// バッファの 1 行を文字列にする (テスト・診断用。継続セルは出さず、末尾の空白は残す)。
pub fn row_text(buf: &Buffer, y: u16) -> String {
    let mut s = String::new();
    for x in 0..buf.area.width {
        let Some(cell) = buf.cell((x, y)) else {
            continue;
        };
        if cell.diff_option == CellDiffOption::Skip {
            continue;
        }
        s.push_str(cell.symbol());
    }
    s
}
