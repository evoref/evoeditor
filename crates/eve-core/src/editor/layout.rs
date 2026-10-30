//! 編集面のレイアウトと Frame の生成 (docs/03 §14、docs/02 §8)。可視範囲だけを生成する (コストは可視行数 × 可視列数)。
//!
//! M1 の範囲: 折り返しなし、水平スクロール、行番号のガター、タブの展開、全角・結合文字・絵文字の幅、不可視文字・制御文字・
//! 不正バイトのエスケープの可視化、全角スペースとタブの可視化 (FR-508 の最小)、選択、カーソル。
//! スタイルはテーマ (eve-config) ができるまで属性だけ (選択 = 反転、記号 = 弱い表示)。

use eve_text::{
    ByteOff, ByteRange, ControlStyle, EmojiWidth, LineIdx, TextRead, WidthConfig, first_grapheme,
    grapheme_display, grapheme_indices, is_width_risky,
};
use eve_view::{
    CellPos, CellRect, CellSize, CellText, CursorPlacement, CursorShape, CutPolicy, FocusTarget,
    Frame, Message, MessageModel, PaneFrame, RenderRow, RowSource, Span, Style, StyleMods, ViewId,
    ViewRevision, WindowId,
};

use eve_encoding::{PuaEscapeMode, unescape_char};

use crate::document::Document;

use super::state::EditorState;

mod wrap;
pub use wrap::{Affinity, WrapMode, WrapSettings};
use wrap::{cursor_in_rows, wrap_line};

/// 記号 (制御文字・不可視文字・空白の可視化) の表示。
const SYMBOL: Style = Style {
    fg: eve_view::Color::Default,
    bg: eve_view::Color::Default,
    mods: StyleMods::DIM,
    underline_color: None,
};

fn width_config() -> WidthConfig {
    WidthConfig {
        tab_width: 4,
        ambiguous_wide: false,
        control_style: ControlStyle::Caret,
        emoji_width: EmojiWidth::Auto,
        show_invisible: true,
    }
}

/// 本文の表示幅の設定 (文書ごとの `display` を反映する)。
pub(super) fn doc_width_config(doc: &Document) -> WidthConfig {
    WidthConfig {
        tab_width: doc.display.tab_width,
        ambiguous_wide: doc.display.ambiguous_wide,
        ..width_config()
    }
}

/// 表示の 1 単位 (1 書記素)。
struct Unit {
    /// 行頭からの表示列。
    col: usize,
    /// 描く Span (文字列, 幅, 記号か)。
    parts: Vec<(String, u16, bool)>,
    width: usize,
    /// 行内のバイト位置。
    byte: usize,
    risky: bool,
}

/// 単独で幅 2・記号でない・不可視のマークなし・幅の危険なし、の通常の文字か (かな・漢字・全角の英数と主な句読点)。
/// `units` の近道の対象。曖昧幅の設定にも依らないことを、範囲の全文字を `grapheme_display` と照合するテストで確かめている。
fn is_plain_wide(c: char) -> bool {
    matches!(c as u32,
        0x3001..=0x3002        // 、。
        | 0x300C..=0x300D      // 「」
        | 0x3041..=0x3096      // ひらがな
        | 0x30A1..=0x30FA | 0x30FC // カタカナ (・ U+30FB は除く)
        | 0x4E00..=0x9FFF      // CJK 統合漢字
        | 0xFF01..=0xFF5E) // 全角の ASCII
}

/// 溜めた文字列を通常の書記素として表示に変換して `parts` に足す。
fn flush_run(run: &mut String, parts: &mut Vec<(String, u16, bool)>, cfg: &WidthConfig) {
    if run.is_empty() {
        return;
    }
    let d = grapheme_display(run, cfg);
    if !d.base.is_empty() {
        parts.push((d.base, d.base_width, false));
    }
    if !d.marks.is_empty() {
        parts.push((d.marks, d.marks_width, true));
    }
    run.clear();
}

/// 展開の覚えを使う行の大きさ (これ未満は毎回展開する)。
const UNITS_CACHE_MIN: usize = 4096;

/// `units` の結果。長い行 (長大行の表示範囲 64 KiB など) は、Revision・行・幅の設定が同じ間は覚えを使う
/// (1 回の操作で `cursor_cell` と Frame の生成が同じ行を何度も展開するのを避ける、NFR-106)。
fn units_for(doc: &Document, line: usize, content: &str) -> std::rc::Rc<Vec<Unit>> {
    use std::rc::Rc;
    let esc = esc_enabled(doc);
    let cfg = doc_width_config(doc);
    if content.len() < UNITS_CACHE_MIN {
        return Rc::new(units(content, esc, &cfg));
    }
    type Key = (eve_view::Revision, usize, bool, u8, bool);
    let key: Key = (doc.revision(), line, esc, cfg.tab_width, cfg.ambiguous_wide);
    if let Some(any) = doc.layout_cache.borrow().as_ref()
        && let Some((k, u)) = any.downcast_ref::<(Key, Rc<Vec<Unit>>)>()
        && *k == key
    {
        return u.clone();
    }
    let u = Rc::new(units(content, esc, &cfg));
    *doc.layout_cache.borrow_mut() = Some(Box::new((key, u.clone())));
    u
}

/// 1 書記素 `g` (行内のバイト位置 `off`、表示列 `col`) を表示の単位に展開する (一般の経路)。
fn unit_slow(g: &str, off: usize, col: usize, tab: usize, esc: bool, cfg: &WidthConfig) -> Unit {
    let mut parts: Vec<(String, u16, bool)> = Vec::new();
    let mut risky = false;
    if g == "\t" {
        let w = tab - col % tab;
        parts.push((">".to_owned(), 1, true));
        if w > 1 {
            parts.push((" ".repeat(w - 1), (w - 1) as u16, true));
        }
    } else if g == "\u{3000}" {
        parts.push(("[]".to_owned(), 2, true));
    } else if esc && g.chars().any(|c| unescape_char(c).is_some()) {
        // 不正バイトのエスケープ文字は 1 文字ずつ `<0xHH>` にする (結合文字などと同じ書記素になることがある)
        let mut run = String::new();
        for c in g.chars() {
            if let Some(b) = unescape_char(c) {
                flush_run(&mut run, &mut parts, cfg);
                parts.push((format!("<0x{b:02X}>"), 6, true));
            } else {
                run.push(c);
            }
        }
        flush_run(&mut run, &mut parts, cfg);
    } else {
        let d = grapheme_display(g, cfg);
        let first = g.chars().next().unwrap_or(' ');
        let symbolic = (first as u32) < 0x20
            || (0x7F..=0x9F).contains(&(first as u32))
            || d.base.starts_with("<U+");
        if !d.base.is_empty() {
            parts.push((d.base, d.base_width, symbolic));
        }
        if !d.marks.is_empty() {
            parts.push((d.marks, d.marks_width, true));
        }
        risky = is_width_risky(g);
    }
    let width: usize = parts.iter().map(|p| p.1 as usize).sum();
    Unit {
        col,
        parts,
        width,
        byte: off,
        risky,
    }
}

/// 行の内容を表示の単位に展開する。
fn units(content: &str, esc: bool, cfg: &WidthConfig) -> Vec<Unit> {
    let tab = cfg.tab_width.max(1) as usize;
    // 印字できる ASCII だけの行 (本文の大半): 書記素の分割も表示の組み立ても要らない (1 文字 = 1 書記素 = 幅 1)
    if content.bytes().all(|b| (0x20..=0x7E).contains(&b)) {
        return content
            .bytes()
            .enumerate()
            .map(|(i, b)| Unit {
                col: i,
                parts: vec![((b as char).to_string(), 1, false)],
                width: 1,
                byte: i,
                risky: false,
            })
            .collect();
    }
    let mut out = Vec::new();
    let mut col = 0usize;
    let mut at = 0usize;
    while at < content.len() {
        let rest = &content[at..];
        let Some(c) = rest.chars().next() else { break };
        // 印字できる ASCII と幅 2 の通常の文字 (かな・漢字・全角の英数。本文の大半) が続くところは、その 1 文字が 1 書記素
        // (次の文字も同じ種類なら結合しない) で、書記素の分割も表示の組み立て (文字列の確保が複数回) も通さない
        let simple = |c: char| matches!(c as u32, 0x20..=0x7E) || is_plain_wide(c);
        if simple(c) && rest[c.len_utf8()..].chars().next().is_none_or(simple) {
            let w: u16 = if c.is_ascii() { 1 } else { 2 };
            out.push(Unit {
                col,
                parts: vec![(c.to_string(), w, false)],
                width: usize::from(w),
                byte: at,
                risky: false,
            });
            col += usize::from(w);
            at += c.len_utf8();
            continue;
        }
        let g = first_grapheme(rest);
        let off = at;
        at += g.len();
        let u = unit_slow(g, off, col, tab, esc, cfg);
        col += u.width;
        out.push(u);
    }
    out
}

/// 長大行 (閾値超。読み取り専用で開いたもの) は先頭 64 KiB (書記素の境界) までを表示とカーソルの対象にする (NFR-106、03 §11.5)。
/// 戻り値は表示の終端のバイト位置 (それ以外の行は行末)。
pub(super) fn line_visible_end(doc: &Document, ls: usize, le: usize) -> usize {
    use crate::document::{LONG_LINE_THRESHOLD, LONG_LINE_VISIBLE, ReadOnlyReason};
    if le - ls > LONG_LINE_THRESHOLD && doc.read_only.contains(ReadOnlyReason::LongLine) {
        eve_text::snap_to_grapheme(
            doc.text(),
            ByteOff(ls + LONG_LINE_VISIBLE),
            eve_text::Bias::Before,
        )
        .0
        .max(ls)
    } else {
        le
    }
}

/// (行頭, 表示する内容, 改行があるか, 省略したバイト数)。
fn line_content(doc: &Document, line: usize) -> (usize, String, bool, usize) {
    let t = doc.text();
    let l = LineIdx(line);
    let (ls, le) = (t.line_to_byte(l).0, t.line_end_byte(l).0);
    let has_terminator = t.line_terminator(l).is_some();
    let ve = line_visible_end(doc, ls, le);
    (
        ls,
        t.slice_to_string(ByteRange::new(ByteOff(ls), ByteOff(ve))),
        has_terminator,
        le - ve,
    )
}

fn esc_enabled(doc: &Document) -> bool {
    doc.format.pua_escape == PuaEscapeMode::Enabled
}

/// 行内のバイト位置 `byte` にあるカーソルの (表示列, そのセルの幅)。行末では (行の幅, 1)。
pub(super) fn cursor_cell(doc: &Document, line: usize, byte: usize) -> (usize, usize) {
    let (_, content, _, _) = line_content(doc, line);
    let us = units_for(doc, line, &content);
    match us.iter().find(|u| u.byte >= byte) {
        Some(u) => (u.col, u.width.max(1)),
        None => (us.last().map_or(0, |u| u.col + u.width), 1),
    }
}

/// 行内のバイト位置 `byte` にあるカーソルの表示列 (行頭から)。移動の目標列もこの表示と同じ幅で数える。
pub(super) fn cursor_col(doc: &Document, line: usize, byte: usize) -> usize {
    cursor_cell(doc, line, byte).0
}

/// 表示列 `dcol` に対応する行内のバイト位置。セルの途中は、その単位の前に寄せる (`Bias::Before`)。行の幅を超えれば行末。
pub(super) fn byte_at_col(doc: &Document, line: usize, dcol: usize) -> usize {
    let (_, content, _, _) = line_content(doc, line);
    let us = units_for(doc, line, &content);
    us.iter()
        .find(|u| dcol < u.col + u.width)
        .map_or(content.len(), |u| u.byte)
}

/// 折り返し幅: 本文の幅 − 右余白 1 (行末にぶら下げた空白がその 1 セルに入る、03 §14.3.1)。
pub(super) fn wrap_width(doc: &Document, viewport: (u16, u16)) -> usize {
    let cols = viewport.0 as usize;
    let gw = gutter_width(doc, cols) as usize;
    cols.saturating_sub(gw).saturating_sub(1).max(1)
}

/// 折り返し ON の View の (設定, 折り返し幅)。OFF は `None` (移動とスクロールは論理行のまま)。
pub(super) fn wrap_ctx(state: &EditorState, view: ViewId) -> Option<(WrapSettings, usize)> {
    let v = state.view(view)?;
    if !v.wrap.is_on() {
        return None;
    }
    let d = state.document(v.doc)?;
    Some((v.wrap, wrap_width(d, state.viewport())))
}

/// 1 論理行を表示行に分けた結果 (03 §14.3.1)。移動とスクロールが表示行を数えるのに使う。
/// 位置はすべて行頭からのバイト位置 (`rel`)。
pub(super) struct LineRows {
    /// 行頭の絶対バイト位置。
    pub ls: usize,
    content_len: usize,
    units: std::rc::Rc<Vec<Unit>>,
    rows: Vec<wrap::WrapRow>,
}

pub(super) fn line_rows(doc: &Document, line: usize, s: WrapSettings, cols: usize) -> LineRows {
    let (ls, content, _, _) = line_content(doc, line);
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
pub(super) fn gutter_width(doc: &Document, cols: usize) -> u16 {
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

struct RowOut {
    row: RenderRow,
    /// 行頭からの表示列でのカーソル位置。
    cursor_cols: Vec<usize>,
}

/// 検索の強調 (文書のバイト位置の半開区間)。`found` は一致 (開始位置の昇順)、`current` は入力で見つけた現在の一致。
struct Marks<'a> {
    found: &'a [(usize, usize)],
    current: Option<(usize, usize)>,
    /// 保存先の文字コードで表せない文字 (E3、03 §8.6)。
    unmappable: &'a [(usize, usize)],
    /// 長大行の省略の印の文言 (省略したバイト数 → 文字列)。
    omit: &'a dyn Fn(usize) -> String,
}

/// 折り返した論理行のうち、作る表示行 1 本 (単位の範囲、継続行のずらし、行内の表示行の番号、最後の表示行か)。
#[derive(Clone, Copy)]
struct RowPart {
    first: usize,
    end: usize,
    indent: usize,
    wrap_idx: u32,
    last: bool,
}

/// 1 行を作る。`left` は水平スクロール (表示列)、`cols` は本文の幅。`sel` は選択 (文書のバイト位置の半開区間)、
/// `heads` はこの行にあるカーソル (文書のバイト位置)。
#[allow(clippy::too_many_arguments)]
fn build_row(
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
    let (ls, content, has_terminator, omitted) = line_content(doc, line);
    let body = units_for(doc, line, &content);
    // 省略した分は行末に印を出す (「… (以下 N バイト省略)」。表示だけで、残りは文書に保たれている)
    let marker: Option<Unit> = (omitted > 0).then(|| {
        let col = body.last().map_or(0, |u| u.col + u.width);
        let parts: Vec<(String, u16, bool)> = cell_text(&(marks.omit)(omitted), CutPolicy::Never)
            .spans
            .into_iter()
            .map(|sp| (sp.text.to_string(), sp.width, true))
            .collect();
        let width = parts.iter().map(|p| p.1 as usize).sum();
        Unit {
            col,
            parts,
            width,
            byte: content.len(),
            risky: false,
        }
    });
    // 折り返した表示行は、その範囲の単位だけを、行内の先頭の列を 0 (+ ずらし) として並べる。省略の印は最後の表示行にだけ付く
    let (slice, with_marker, base, shift) = match part {
        Some(p) => {
            let slice = &body[p.first.min(body.len())..p.end.min(body.len())];
            let base = body
                .get(p.first)
                .map_or_else(|| body.last().map_or(0, |u| u.col + u.width), |u| u.col);
            (slice, p.last, base, p.indent)
        }
        None => (&body[..], true, 0, 0),
    };
    let us = slice.iter().chain(marker.iter().filter(|_| with_marker));
    let total_abs = if part.is_none_or(|p| p.last) {
        marker
            .as_ref()
            .or(body.last())
            .map_or(0, |u| u.col + u.width)
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
        let mut style_base = if selected(at) {
            StyleMods::REVERSE
        } else {
            StyleMods::NONE
        };
        // 検索の一致は下線、入力で見つけた現在の一致は反転 + 太字 + 下線 (色に頼らない、04 §8.4)
        if marks.unmappable.iter().any(|&(s, e)| s <= at && at < e) {
            style_base = style_base | StyleMods::REVERSE | StyleMods::UNDERLINE;
        }
        if marks.current.is_some_and(|(s, e)| s <= at && at < e) {
            style_base = style_base | StyleMods::REVERSE | StyleMods::BOLD | StyleMods::UNDERLINE;
        } else {
            let i = marks.found.partition_point(|&(_, e)| e <= at);
            if marks.found.get(i).is_some_and(|&(s, _)| s <= at) {
                style_base = style_base | StyleMods::UNDERLINE;
            }
        }
        let mut c = u.col - base + shift;
        for (text, w, symbol) in &u.parts {
            let (w, c0) = (*w as usize, c);
            c += w;
            if w == 0 {
                continue;
            }
            let style = Style::with_mods(
                style_base
                    | if *symbol {
                        StyleMods::DIM
                    } else {
                        StyleMods::NONE
                    },
            );
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
    let line_end_abs = ls + content.len();
    let eol_selected = has_terminator
        && part.is_none_or(|p| p.last)
        && sel
            .iter()
            .any(|&(s, e)| s <= line_end_abs && e > line_end_abs);
    if eol_selected && total >= win_start && total < win_end {
        push_span(&mut spans, " ", 1, Style::with_mods(StyleMods::REVERSE));
    }
    let cursor_cols = heads
        .iter()
        .map(|&h| {
            let rel = h.saturating_sub(ls);
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

fn gutter_spans(line: usize, width: u16) -> Vec<Span> {
    if width == 0 {
        return Vec::new();
    }
    let text = format!("{:>w$} ", line + 1, w = width as usize - 1);
    vec![Span {
        text,
        width,
        style: SYMBOL,
    }]
}

/// 文字列を `CellText` にする (INV-F1: 制御文字などは記号に展開済み。幅は core が計算する)。
/// `cut` が `Never` 以外のときは 1 書記素 = 1 `Span` で渡す。
pub(super) fn cell_text(s: &str, cut: CutPolicy) -> CellText {
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

fn message_model(state: &EditorState) -> MessageModel {
    let current = state.current_message().map(|m| Message {
        level: m.level,
        text: cell_text(&m.text, CutPolicy::End),
        hint: m.hint.as_deref().map(|h| cell_text(h, CutPolicy::End)),
    });
    MessageModel {
        current,
        unread: state.unread_messages(),
    }
}

/// 端末タイトル (無害化済み)。
fn title(state: &EditorState, doc: &Document) -> String {
    let name = doc
        .path
        .as_ref()
        .and_then(|p| p.file_name())
        .unwrap_or_else(|| "無題".to_owned());
    // 制御文字と、表示を偽装できる双方向の制御文字は端末のタイトルに出さない
    let name: String = name
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(c, '\u{200E}' | '\u{200F}' | '\u{061C}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
        })
        .collect();
    // 文書が 2 件以上のときは、ファイル名の後ろに何番目かを出す (03 §9.8。ステータスバーと同じ)
    let pos = match state.buffer_position() {
        Some((n, total)) => format!(" ({n}/{total})"),
        None => String::new(),
    };
    format!(
        "{}{}{} - evoeditor",
        if doc.is_dirty() { "* " } else { "" },
        name,
        pos
    )
}

/// 折り返し ON の表示行を `rows` 本作る (03 §14.3.1)。先頭は `top` の論理行の `wrap_idx` 番目の表示行。
/// 戻り値は (行, カーソルのセル位置, 主カーソルのセル位置)。
#[allow(clippy::too_many_arguments)]
fn wrapped_rows(
    doc: &Document,
    view: &super::state::View,
    gw: u16,
    text_cols: usize,
    rows: usize,
    sel: &[(usize, usize)],
    heads: &[usize],
    primary_head: usize,
    marks: &Marks<'_>,
) -> (Vec<RenderRow>, Vec<CellPos>, Option<CellPos>) {
    let t = doc.text();
    let n_lines = t.len_lines();
    // 折り返し幅は本文の幅 − 右余白 1 (行末にぶら下げた空白がその 1 セルに入る)
    let wrap_cols = text_cols.saturating_sub(1).max(1);
    let mut out_rows = Vec::with_capacity(rows);
    let mut cursors = Vec::new();
    let mut primary: Option<CellPos> = None;
    let mut line = t.byte_to_line(view.scroll.top).0;
    let mut widx = view.scroll.wrap_idx as usize;
    while out_rows.len() < rows {
        if line >= n_lines {
            out_rows.push(RenderRow {
                source: RowSource::Virtual,
                gutter: Vec::new(),
                spans: Vec::new(),
                clipped_left: false,
                clipped_right: false,
                width_risky: false,
            });
            continue;
        }
        let (ls, content, _, _) = line_content(doc, line);
        let le = t.line_end_byte(LineIdx(line)).0;
        let us = units_for(doc, line, &content);
        let wrows = wrap_line(&content, &us, wrap_cols, view.wrap);
        let here: Vec<usize> = heads
            .iter()
            .copied()
            .filter(|&h| h >= ls && h <= le)
            .collect();
        // この論理行の表示行を、先頭 (最初の行だけ `widx` から) から画面の残りの分だけ作る
        for (r, wr) in wrows.iter().enumerate().skip(widx) {
            if out_rows.len() >= rows {
                break;
            }
            let gutter = if r == 0 {
                gutter_spans(line, gw)
            } else if gw > 0 {
                vec![Span {
                    text: " ".repeat(gw as usize),
                    width: gw,
                    style: SYMBOL,
                }]
            } else {
                Vec::new()
            };
            let part = RowPart {
                first: wr.first,
                end: wr.end,
                indent: wr.indent,
                wrap_idx: r as u32,
                last: r + 1 == wrows.len(),
            };
            let ro = build_row(doc, line, 0, text_cols, sel, &[], marks, gutter, Some(part));
            for &h in &here {
                let aff = if h == primary_head {
                    view.cursor_affinity
                } else {
                    Affinity::After
                };
                let (cr, cc) = cursor_in_rows(&us, &wrows, h - ls, aff);
                if cr == r && cc < text_cols {
                    let pos = CellPos {
                        col: gw + cc as u16,
                        row: out_rows.len() as u16,
                    };
                    cursors.push(pos);
                    if h == primary_head {
                        primary = Some(pos);
                    }
                }
            }
            out_rows.push(ro.row);
        }
        line += 1;
        widx = 0;
    }
    (out_rows, cursors, primary)
}

/// Window の Frame を作る。
pub(super) fn build_frame(
    state: &EditorState,
    window: WindowId,
    chord: &[eve_view::KeyStroke],
) -> Option<Frame> {
    let view_id = state.window_view(window)?;
    let view = state.view(view_id)?;
    let doc = state.document(view.doc)?;
    let (cols, rows) = state.viewport();
    let (cols, rows) = (cols as usize, rows as usize);
    let gw = gutter_width(doc, cols);
    let text_cols = cols.saturating_sub(gw as usize);
    let t = doc.text();
    let top_line = t.byte_to_line(view.scroll.top).0;
    let left = view.scroll.left_col as usize;
    let n_lines = t.len_lines();

    // 可視範囲に触れる Range だけを使う (Range は start 昇順で重ならないので二分探索できる)。
    // 多数のカーソルがあっても、フレームの生成は可視範囲の大きさに比例する
    let vis_start = t
        .line_to_byte(LineIdx(top_line.min(n_lines.saturating_sub(1))))
        .0;
    let vis_end = t
        .line_end_byte(LineIdx((top_line + rows).min(n_lines.saturating_sub(1))))
        .0;
    let ranges = view.selection.ranges();
    let first = ranges.partition_point(|r| r.end().0 < vis_start);
    let last = ranges.partition_point(|r| r.start().0 <= vis_end);
    let visible = &ranges[first..last.max(first)];
    let sel: Vec<(usize, usize)> = visible
        .iter()
        .filter(|r| !r.is_empty())
        .map(|r| (r.start().0, r.end().0))
        .collect();
    let heads: Vec<usize> = visible.iter().map(|r| r.head.0).collect();
    let primary_head = view.selection.primary().head.0;

    let overlay_open = state.overlay.is_some();
    let bar_open = state.search_bar.is_some();
    let found = state.visible_matches(view.doc, top_line, rows);
    let current = state.bar_current();
    let unmappable = state.unmappable_marks_for(view.doc);
    let omit = |n: usize| {
        state.catalog().render(
            "ui.long_line.omitted",
            &[("bytes", &n.to_string())],
            &|_| None,
        )
    };
    let marks = Marks {
        found: &found,
        current,
        unmappable: &unmappable,
        omit: &omit,
    };
    let (out_rows, cursors, primary) = if view.wrap.is_on() {
        wrapped_rows(
            doc,
            view,
            gw,
            text_cols,
            rows,
            &sel,
            &heads,
            primary_head,
            &marks,
        )
    } else {
        let mut out_rows = Vec::with_capacity(rows);
        let mut cursors = Vec::new();
        let mut primary: Option<CellPos> = None;
        for r in 0..rows {
            let line = top_line + r;
            if line >= n_lines {
                out_rows.push(RenderRow {
                    source: RowSource::Virtual,
                    gutter: Vec::new(),
                    spans: Vec::new(),
                    clipped_left: false,
                    clipped_right: false,
                    width_risky: false,
                });
                continue;
            }
            let (ls, le) = (
                t.line_to_byte(LineIdx(line)).0,
                t.line_end_byte(LineIdx(line)).0,
            );
            // この行にあるカーソル (行末の位置は次の行の行頭と区別する: 行の終端を含む)
            let here: Vec<usize> = heads
                .iter()
                .copied()
                .filter(|&h| h >= ls && h <= le)
                .collect();
            let ro = build_row(
                doc,
                line,
                left,
                text_cols,
                &sel,
                &here,
                &marks,
                gutter_spans(line, gw),
                None,
            );
            for (&h, &c) in here.iter().zip(&ro.cursor_cols) {
                if c >= left && c - left < text_cols {
                    let pos = CellPos {
                        col: gw + (c - left) as u16,
                        row: r as u16,
                    };
                    cursors.push(pos);
                    if h == primary_head {
                        primary = Some(pos);
                    }
                }
            }
            out_rows.push(ro.row);
        }
        (out_rows, cursors, primary)
    };
    let pane = PaneFrame {
        view: view_id,
        rect: CellRect {
            col: 0,
            row: 0,
            cols: cols as u16,
            rows: rows as u16,
        },
        revision: (doc.revision(), ViewRevision(0)),
        gutter_width: gw,
        rows: out_rows,
        cursors,
        focused: !overlay_open && !bar_open,
    };
    Some(Frame {
        window,
        size: CellSize {
            cols: cols as u16,
            rows: rows as u16,
        },
        panes: vec![pane],
        focus: if overlay_open {
            FocusTarget::Overlay(0)
        } else if bar_open {
            FocusTarget::SearchBar
        } else {
            FocusTarget::Pane(view_id)
        },
        primary_cursor: primary
            .filter(|_| !overlay_open && !bar_open)
            .map(|pos| CursorPlacement {
                pos,
                shape: CursorShape::Bar,
            }),
        search: state.search_bar_model(),
        status: super::chrome::status_model(state, view_id, chord),
        hints: super::chrome::hint_model(state, chord),
        overlays: super::chrome::overlay_models(state),
        which_key: None,
        messages: message_model(state),
        texts: super::chrome::chrome_texts(state),
        title: title(state, doc),
    })
}

/// 水平スクロールの調整に使う、primary のカーソルの (表示列, セルの幅)。
pub(super) fn primary_cursor_cell(state: &EditorState, view: ViewId) -> Option<(usize, usize)> {
    let v = state.view(view)?;
    let d = state.document(v.doc)?;
    let head = v.selection.primary().head;
    let line = d.text().byte_to_line(head);
    let ls = d.text().line_to_byte(line).0;
    Some(cursor_cell(d, line.0, head.0.saturating_sub(ls)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `units` の ASCII の近道は、一般の経路 (書記素の表示の組み立て) と同じ結果になる前提に立つ:
    /// 印字できる ASCII 1 文字は、幅 1・マークなし・記号でない・幅の危険なし。
    #[test]
    fn 印字できる_ascii_の近道は一般の経路と同じ結果になる() {
        for ambiguous_wide in [false, true] {
            let cfg = WidthConfig {
                ambiguous_wide,
                ..width_config()
            };
            for b in 0x20u8..=0x7E {
                let g = (b as char).to_string();
                let d = grapheme_display(&g, &cfg);
                assert_eq!(d.base, g, "{b:#x}");
                assert_eq!(d.base_width, 1, "{b:#x}");
                assert!(d.marks.is_empty(), "{b:#x}");
                assert!(!d.base.starts_with("<U+"), "{b:#x}");
                assert!(!is_width_risky(&g), "{b:#x}");
            }
        }
    }

    #[test]
    fn 幅_2_の通常の文字の近道は範囲の全文字で一般の経路と同じ結果になる() {
        for ambiguous_wide in [false, true] {
            let cfg = WidthConfig {
                ambiguous_wide,
                ..width_config()
            };
            let mut checked = 0;
            for v in 0x3000u32..=0xFFFF {
                let Some(c) = char::from_u32(v).filter(|&c| is_plain_wide(c)) else {
                    continue;
                };
                let g = c.to_string();
                let d = grapheme_display(&g, &cfg);
                assert_eq!(d.base, g, "{v:#x}");
                assert_eq!((d.base_width, d.marks_width), (2, 0), "{v:#x}");
                assert!(d.marks.is_empty() && !d.base.starts_with("<U+"), "{v:#x}");
                assert!(!is_width_risky(&g), "{v:#x}");
                checked += 1;
            }
            assert!(checked > 20_000, "{checked}");
        }
    }

    #[test]
    fn 近道の展開は列とバイト位置が一般の経路と同じ() {
        let cfg = width_config();
        // ASCII だけの行 (近道) と、同じ文字列に非 ASCII を足して一般の経路を通した行の、共通部分の展開が同じ
        let ascii = "the quick brown fox 0123456789";
        let fast = units(ascii, false, &cfg);
        let mixed = units(&format!("{ascii}日"), false, &cfg);
        assert_eq!(fast.len() + 1, mixed.len());
        for (a, b) in fast.iter().zip(&mixed) {
            assert_eq!(
                (a.col, a.width, a.byte, a.risky, &a.parts),
                (b.col, b.width, b.byte, b.risky, &b.parts)
            );
        }
        assert!(units("", false, &cfg).is_empty());
    }

    /// 基準: 全ての書記素を一般の経路で展開する。
    fn units_reference(content: &str, esc: bool, cfg: &WidthConfig) -> Vec<Unit> {
        let tab = cfg.tab_width.max(1) as usize;
        let mut col = 0usize;
        grapheme_indices(content)
            .into_iter()
            .map(|(off, g)| {
                let u = unit_slow(g, off, col, tab, esc, cfg);
                col += u.width;
                u
            })
            .collect()
    }

    #[test]
    fn 近道の展開は全ての書記素を一般の経路で展開した結果と同じになる() {
        // 決定的な擬似乱数で、近道の文字と、結合・絵文字・ハングル・国旗・タブ・全角空白・制御文字・エスケープを混ぜる
        let pieces: [&str; 24] = [
            "a",
            "Z",
            " ",
            "~",
            "あ",
            "ア",
            "漢",
            "字",
            "、",
            "「",
            "Ａ",
            "ー",
            "・",
            "e\u{301}",
            "\u{301}",
            "😀",
            "👨\u{200D}👩",
            "🇯🇵",
            "한",
            "\t",
            "\u{3000}",
            "\u{7}",
            "\u{85}",
            "\u{10FF80}",
        ];
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for ambiguous_wide in [false, true] {
            for esc in [false, true] {
                let cfg = WidthConfig {
                    ambiguous_wide,
                    ..width_config()
                };
                for _ in 0..3000 {
                    let mut line = String::new();
                    for _ in 0..(next() % 14) {
                        line.push_str(pieces[(next() % 24) as usize]);
                    }
                    let fast = units(&line, esc, &cfg);
                    let slow = units_reference(&line, esc, &cfg);
                    assert_eq!(fast.len(), slow.len(), "{line:?}");
                    for (a, b) in fast.iter().zip(&slow) {
                        assert_eq!(
                            (a.col, a.width, a.byte, a.risky, &a.parts),
                            (b.col, b.width, b.byte, b.risky, &b.parts),
                            "{line:?}"
                        );
                    }
                }
            }
        }
    }
}
