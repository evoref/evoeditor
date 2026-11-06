//! 表示の行の組み立て (折り返しの行の切り方・ガター・選択と不可視文字の印・スタイルの重ね合わせ、docs/03 §14、docs/02 §8)。

use eve_text::{LineIdx, TextRead, grapheme_display, grapheme_indices};
use eve_view::{CellText, CutPolicy, RenderRow, RowSource, Span, Style, StyleMods, ViewId};

use eve_encoding::unescape_char;

use crate::document::Document;

use super::Affinity;
use super::WrapSettings;
use super::cursor_in_rows;
use super::units::Unit;
use super::units::is_whitespace_mark;
use super::units::line_content;
use super::units::long_index;
use super::units::long_units;
use super::units::units_for;
use super::units::width_config;
use super::wrap;
use super::wrap_line;
use crate::editor::state::EditorState;

/// `top` を `base` に重ねる: 既定でない色は `top` が勝ち、属性は足す (07 §14.5 の Decoration の合成の簡略)。
fn over(base: Style, top: Style) -> Style {
    let pick = |b: eve_view::Color, t: eve_view::Color| {
        if t == eve_view::Color::Default { b } else { t }
    };
    Style {
        fg: pick(base.fg, top.fg),
        bg: pick(base.bg, top.bg),
        mods: base.mods | top.mods,
        underline_color: top.underline_color.or(base.underline_color),
    }
}

/// 折り返し幅: 本文の幅 − 右余白 1 (行末にぶら下げた空白がその 1 セルに入る、03 §14.3.1)。
pub(crate) fn wrap_width(doc: &Document, viewport: (u16, u16)) -> usize {
    let cols = viewport.0 as usize;
    let gw = gutter_width(doc, cols) as usize;
    cols.saturating_sub(gw).saturating_sub(1).max(1)
}

/// 折り返し ON の View の (設定, 折り返し幅)。OFF は `None` (移動とスクロールは論理行のまま)。
/// 長大行を含む文書は、`view.wrap` の値に依らず折り返さない (03 §11.5.1)。
pub(crate) fn wrap_ctx(state: &EditorState, view: ViewId) -> Option<(WrapSettings, usize)> {
    let v = state.view(view)?;
    if !v.wrap.is_on() {
        return None;
    }
    let d = state.document(v.doc)?;
    if d.format.long_lines > 0 {
        return None;
    }
    Some((v.wrap, wrap_width(d, state.viewport())))
}

/// 1 論理行を表示行に分けた結果 (03 §14.3.1)。移動とスクロールが表示行を数えるのに使う。
/// 位置はすべて行頭からのバイト位置 (`rel`)。
pub(crate) struct LineRows {
    /// 行頭の絶対バイト位置。
    pub ls: usize,
    content_len: usize,
    units: std::rc::Rc<Vec<Unit>>,
    rows: Vec<wrap::WrapRow>,
}

pub(crate) fn line_rows(doc: &Document, line: usize, s: WrapSettings, cols: usize) -> LineRows {
    let (ls, content) = line_content(doc, line);
    let units = units_for(doc, line, &content);
    let rows = wrap_line(&content, &units, cols, s);
    LineRows {
        ls,
        content_len: content.len(),
        units,
        rows,
    }
}

impl LineRows {
    /// 表示行の数 (1 以上)。
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    fn byte_of_unit(&self, i: usize) -> usize {
        self.units.get(i).map_or(self.content_len, |u| u.byte)
    }

    /// 表示行 `r` の先頭 / 末尾 (次の表示行の先頭と同じ位置) のバイト位置。
    pub fn row_start(&self, r: usize) -> usize {
        self.byte_of_unit(self.rows[r].first)
    }

    pub fn row_end(&self, r: usize) -> usize {
        self.byte_of_unit(self.rows[r].end)
    }

    /// `rel` にあるカーソルの (表示行, 画面上の列 = 継続行のずらしを含む)。
    pub fn locate(&self, rel: usize, affinity: Affinity) -> (usize, usize) {
        cursor_in_rows(&self.units, &self.rows, rel, affinity)
    }

    /// 表示行 `r` の画面上の列 `col` の位置 (セルの途中は前に寄せる)。行の幅を超えたら表示行の末尾で、
    /// 折り返し位置なら上の表示行の末尾 (`Before`) にとどめる。
    pub fn at_col(&self, r: usize, col: usize) -> (usize, Affinity) {
        let row = self.rows[r];
        let base = self.units.get(row.first).map_or(0, |u| u.col);
        let want = col.saturating_sub(row.indent) + base;
        for u in &self.units[row.first..row.end] {
            if want < u.col + u.width {
                return (u.byte, Affinity::After);
            }
        }
        if r + 1 == self.rows.len() {
            (self.content_len, Affinity::After)
        } else {
            (self.byte_of_unit(row.end), Affinity::Before)
        }
    }
}

/// 行番号のガターの幅 (桁数 + 区切りの空白 1)。画面が狭くて本文が入らないときは、ガターを出さない (0)。
pub(crate) fn gutter_width(doc: &Document, cols: usize) -> u16 {
    let lines = doc.text().len_lines().max(1);
    let w = lines.to_string().len().max(2) + 1;
    if cols >= w + 2 { w as u16 } else { 0 }
}

/// 隣り合う ASCII の記号でない Span を結合する (Span の契約: 含む書記素の幅がすべて同じ)。
fn push_span(spans: &mut Vec<Span>, text: &str, width: u16, style: Style) {
    let ascii_cells =
        width as usize == text.len() && text.bytes().all(|b| (0x20..0x7F).contains(&b));
    if ascii_cells
        && let Some(last) = spans.last_mut()
        && last.style == style
        // 幅とバイト数が等しい Span は、ここで結合した ASCII の Span だけ (全書記素が幅 1)。毎回全体を走査しない
        && last.width as usize == last.text.len()
    {
        debug_assert!(last.text.bytes().all(|b| (0x20..0x7F).contains(&b)));
        last.text.push_str(text);
        last.width += width;
        return;
    }
    spans.push(Span {
        text: text.to_owned(),
        width,
        style,
    });
}

pub(super) struct RowOut {
    pub(super) row: RenderRow,
    /// 行頭からの表示列でのカーソル位置。
    pub(super) cursor_cols: Vec<usize>,
}

/// 検索の強調 (文書のバイト位置の半開区間)。`found` は一致 (開始位置の昇順)、`current` は入力で見つけた現在の一致。
pub(super) struct Marks<'a> {
    pub(super) found: &'a [(usize, usize)],
    pub(super) current: Option<(usize, usize)>,
    /// 保存先の文字コードで表せない文字 (E3、03 §8.6)。
    pub(super) unmappable: &'a [(usize, usize)],
    /// 行の組み立てで使うロールのスタイル (フレームごとに 1 回引く)。
    pub(super) styles: &'a RowStyles,
}

/// 行の組み立てで使うロールのスタイル (07 §8.6)。`RoleStyles::get` は名前の二分探索なので、
/// 単位ごとに引かず、フレームの初めに 1 回だけ引いて持つ (PRF-01)。
pub(super) struct RowStyles {
    pub(super) text: Style,
    pub(super) whitespace: Style,
    pub(super) control_char: Style,
    pub(super) selection: Style,
    pub(super) search_match: Style,
    pub(super) search_current: Style,
    pub(super) line_number: Style,
    pub(super) line_number_active: Style,
}

impl RowStyles {
    pub(super) fn new(roles: &eve_view::RoleStyles) -> RowStyles {
        RowStyles {
            text: roles.get("ui.text"),
            whitespace: roles.get("ui.whitespace"),
            control_char: roles.get("ui.control_char"),
            selection: roles.get("ui.selection"),
            search_match: roles.get("ui.search.match"),
            search_current: roles.get("ui.search.current"),
            line_number: roles.get("ui.line_number"),
            line_number_active: roles.get("ui.line_number.active"),
        }
    }
}

/// 折り返した論理行のうち、作る表示行 1 本 (単位の範囲、継続行のずらし、行内の表示行の番号、最後の表示行か)。
#[derive(Clone, Copy)]
pub(super) struct RowPart {
    pub(super) first: usize,
    pub(super) end: usize,
    pub(super) indent: usize,
    pub(super) wrap_idx: u32,
    pub(super) last: bool,
}

/// 1 行を作る。`left` は水平スクロール (表示列)、`cols` は本文の幅。`sel` は選択 (文書のバイト位置の半開区間)、
/// `heads` はこの行にあるカーソル (文書のバイト位置)。
#[allow(clippy::too_many_arguments)]
pub(super) fn build_row(
    doc: &Document,
    line: usize,
    left: usize,
    cols: usize,
    sel: &[(usize, usize)],
    heads: &[usize],
    marks: &Marks<'_>,
    gutter: Vec<Span>,
    part: Option<RowPart>,
) -> RowOut {
    let t = doc.text();
    let (ls, le) = (
        t.line_to_byte(LineIdx(line)).0,
        t.line_end_byte(LineIdx(line)).0,
    );
    let has_terminator = t.line_terminator(LineIdx(line)).is_some();
    // 長大行 (折り返さない) は、見える範囲の char だけを単位にする (03 §11.5.1)
    let long = long_index(doc, line);
    let body = match &long {
        Some((_, ix)) => std::rc::Rc::new(long_units(doc, ls, ix, left, cols)),
        None => units_for(doc, line, &line_content(doc, line).1),
    };
    let line_width = match &long {
        Some((_, ix)) => ix.end().col,
        None => body.last().map_or(0, |u| u.col + u.width),
    };
    // 折り返した表示行は、その範囲の単位だけを、行内の先頭の列を 0 (+ ずらし) として並べる
    let (slice, base, shift) = match part {
        Some(p) => {
            let slice = &body[p.first.min(body.len())..p.end.min(body.len())];
            let base = body.get(p.first).map_or(line_width, |u| u.col);
            (slice, base, p.indent)
        }
        None => (&body[..], 0, 0),
    };
    let us = slice.iter();
    let total_abs = if part.is_none_or(|p| p.last) {
        line_width
    } else {
        slice.last().map_or(base, |u| u.col + u.width)
    };
    let total = total_abs.saturating_sub(base) + shift;
    // 選択は start 昇順で重ならないので二分探索する
    let selected = |byte_abs: usize| {
        let i = sel.partition_point(|&(_, e)| e <= byte_abs);
        sel.get(i).is_some_and(|&(s, _)| s <= byte_abs)
    };
    let (win_start, win_end) = (left, left + cols);
    let (mut clipped_left, mut clipped_right, mut risky) = (false, false, false);
    let mut spans: Vec<Span> = Vec::new();
    'units: for u in us.clone() {
        let at = ls + u.byte;
        // 重ねる順 (低 → 高、03 §14.5): 記号 → 表現不能の文字 → 検索の一致 / 現在の一致 → 選択
        let mut decor = Style::DEFAULT;
        if marks.unmappable.iter().any(|&(s, e)| s <= at && at < e) {
            decor = over(
                decor,
                Style::with_mods(StyleMods::REVERSE | StyleMods::UNDERLINE),
            );
        }
        if marks.current.is_some_and(|(s, e)| s <= at && at < e) {
            decor = over(decor, marks.styles.search_current);
        } else {
            let i = marks.found.partition_point(|&(_, e)| e <= at);
            if marks.found.get(i).is_some_and(|&(s, _)| s <= at) {
                decor = over(decor, marks.styles.search_match);
            }
        }
        if selected(at) {
            decor = over(decor, marks.styles.selection);
        }
        let mut c = u.col - base + shift;
        for (text, w, symbol) in &u.parts {
            let (w, c0) = (*w as usize, c);
            c += w;
            if w == 0 {
                continue;
            }
            let base = if !*symbol {
                marks.styles.text
            } else if is_whitespace_mark(text) {
                marks.styles.whitespace
            } else {
                marks.styles.control_char
            };
            let style = over(base, decor);
            if c0 >= win_end {
                clipped_right = true;
                break 'units;
            }
            if c <= win_start {
                clipped_left = true;
                continue;
            }
            risky |= u.risky;
            if c0 >= win_start && c <= win_end {
                push_span(&mut spans, text, w as u16, style);
            } else {
                // セルの途中で切れる: 見える分のセルを空白で埋める
                let visible = c.min(win_end) - c0.max(win_start);
                clipped_left |= c0 < win_start;
                clipped_right |= c > win_end;
                push_span(&mut spans, &" ".repeat(visible), visible as u16, style);
            }
        }
    }
    if shift > 0 {
        spans.insert(
            0,
            Span {
                text: " ".repeat(shift),
                width: shift as u16,
                style: Style::DEFAULT,
            },
        );
    }
    // 行末の改行が選択に含まれるときは、行末に 1 セルの反転を足す
    let line_end_abs = le;
    let eol_selected = has_terminator
        && part.is_none_or(|p| p.last)
        && sel
            .iter()
            .any(|&(s, e)| s <= line_end_abs && e > line_end_abs);
    if eol_selected && total >= win_start && total < win_end {
        push_span(&mut spans, " ", 1, marks.styles.selection);
    }
    let cursor_cols = heads
        .iter()
        .map(|&h| {
            let rel = h.saturating_sub(ls);
            if let Some((_, ix)) = &long {
                return ix.at_byte(t, ls, rel).0.col;
            }
            match us.clone().find(|u| u.byte >= rel) {
                Some(u) => u.col - base + shift,
                None => total,
            }
        })
        .collect();
    RowOut {
        row: RenderRow {
            source: RowSource::Line {
                line: line as u32,
                wrap_idx: part.map_or(0, |p| p.wrap_idx),
            },
            gutter,
            spans,
            clipped_left,
            clipped_right,
            width_risky: risky,
        },
        cursor_cols,
    }
}

/// 行番号のガター (`ui.line_number`。主カーソルのある行は `ui.line_number.active` を重ねる)。
pub(super) fn gutter_spans(line: usize, width: u16, style: Style) -> Vec<Span> {
    if width == 0 {
        return Vec::new();
    }
    let text = format!("{:>w$} ", line + 1, w = width as usize - 1);
    vec![Span { text, width, style }]
}

/// 行 `line` の行番号のスタイル。
pub(super) fn line_number_style(styles: &RowStyles, line: usize, cursor_line: usize) -> Style {
    let base = styles.line_number;
    if line == cursor_line {
        over(base, styles.line_number_active)
    } else {
        base
    }
}

/// 文字列を `CellText` にする (INV-F1: 制御文字などは記号に展開済み。幅は core が計算する)。
/// `cut` が `Never` 以外のときは 1 書記素 = 1 `Span` で渡す。
pub(crate) fn cell_text(s: &str, cut: CutPolicy) -> CellText {
    let cfg = width_config();
    let mut spans = Vec::new();
    let mut width = 0u16;
    for (_, g) in grapheme_indices(s) {
        let (text, w, style) = match g {
            "\t" => (" ".to_owned(), 1, Style::DEFAULT),
            // 不正バイトのエスケープ文字は表示用の記号にする (INV-F1)
            g if g.chars().count() == 1 && g.chars().next().and_then(unescape_char).is_some() => {
                let b = g.chars().next().and_then(unescape_char).unwrap_or(0);
                (format!("<0x{b:02X}>"), 6, Style::DEFAULT)
            }
            _ => {
                let d = grapheme_display(g, &cfg);
                let w = d.base_width + d.marks_width;
                (format!("{}{}", d.base, d.marks), w, Style::DEFAULT)
            }
        };
        if w == 0 {
            continue;
        }
        width = width.saturating_add(w);
        spans.push(Span {
            text,
            width: w,
            style,
        });
    }
    CellText { spans, width, cut }
}
