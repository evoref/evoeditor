//! 編集面のレイアウトと Frame の生成 (docs/03 §14、docs/02 §8)。可視範囲だけを生成する (コストは可視行数 × 可視列数)。
//!
//! M1 の範囲: 折り返しなし、水平スクロール、行番号のガター、タブの展開、全角・結合文字・絵文字の幅、不可視文字・制御文字・
//! 不正バイトのエスケープの可視化、全角スペースとタブの可視化 (FR-508 の最小)、選択、カーソル。
//! スタイルはテーマ (eve-config) ができるまで属性だけ (選択 = 反転、記号 = 弱い表示)。

use eve_text::{
    ByteOff, ByteRange, ControlStyle, EmojiWidth, LineIdx, TextRead, WidthConfig, grapheme_display,
    grapheme_indices, is_width_risky,
};
use eve_view::{
    CellPos, CellRect, CellSize, CellText, CursorPlacement, CursorShape, CutPolicy, FocusTarget,
    Frame, Message, MessageModel, PaneFrame, RenderRow, RowSource, Span, Style, StyleMods, ViewId,
    ViewRevision, WindowId,
};

use eve_encoding::{PuaEscapeMode, unescape_char};

use crate::document::Document;

use super::state::EditorState;

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

/// 行の内容を表示の単位に展開する。
fn units(content: &str, esc: bool) -> Vec<Unit> {
    let cfg = width_config();
    let tab = cfg.tab_width.max(1) as usize;
    let mut out = Vec::new();
    let mut col = 0usize;
    for (off, g) in grapheme_indices(content) {
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
                    flush_run(&mut run, &mut parts, &cfg);
                    parts.push((format!("<0x{b:02X}>"), 6, true));
                } else {
                    run.push(c);
                }
            }
            flush_run(&mut run, &mut parts, &cfg);
        } else {
            let d = grapheme_display(g, &cfg);
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
        out.push(Unit {
            col,
            parts,
            width,
            byte: off,
            risky,
        });
        col += width;
    }
    out
}

fn line_content(doc: &Document, line: usize) -> (usize, String, bool) {
    let t = doc.text();
    let l = LineIdx(line);
    let (ls, le) = (t.line_to_byte(l).0, t.line_end_byte(l).0);
    let has_terminator = t.line_terminator(l).is_some();
    (
        ls,
        t.slice_to_string(ByteRange::new(ByteOff(ls), ByteOff(le))),
        has_terminator,
    )
}

fn esc_enabled(doc: &Document) -> bool {
    doc.format.pua_escape == PuaEscapeMode::Enabled
}

/// 行内のバイト位置 `byte` にあるカーソルの (表示列, そのセルの幅)。行末では (行の幅, 1)。
pub(super) fn cursor_cell(doc: &Document, line: usize, byte: usize) -> (usize, usize) {
    let (_, content, _) = line_content(doc, line);
    let us = units(&content, esc_enabled(doc));
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
    let (_, content, _) = line_content(doc, line);
    let us = units(&content, esc_enabled(doc));
    us.iter()
        .find(|u| dcol < u.col + u.width)
        .map_or(content.len(), |u| u.byte)
}

/// 行番号のガターの幅 (桁数 + 区切りの空白 1)。画面が狭くて本文が入らないときは、ガターを出さない (0)。
pub(super) fn gutter_width(doc: &Document, cols: usize) -> u16 {
    let lines = doc.text().len_lines().max(1);
    let w = lines.to_string().len().max(2) + 1;
    if cols >= w + 2 { w as u16 } else { 0 }
}

/// 隣り合う ASCII の記号でない Span を結合する (Span の契約: 含む書記素の幅がすべて同じ)。
fn push_span(spans: &mut Vec<Span>, text: String, width: u16, style: Style) {
    let ascii_cells =
        width as usize == text.len() && text.bytes().all(|b| (0x20..0x7F).contains(&b));
    if ascii_cells
        && let Some(last) = spans.last_mut()
        && last.style == style
        && last.width as usize == last.text.len()
        && last.text.bytes().all(|b| (0x20..0x7F).contains(&b))
    {
        last.text.push_str(&text);
        last.width += width;
        return;
    }
    spans.push(Span { text, width, style });
}

struct RowOut {
    row: RenderRow,
    /// 行頭からの表示列でのカーソル位置。
    cursor_cols: Vec<usize>,
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
    gutter: Vec<Span>,
) -> RowOut {
    let (ls, content, has_terminator) = line_content(doc, line);
    let us = units(&content, esc_enabled(doc));
    let total = us.last().map_or(0, |u| u.col + u.width);
    // 選択は start 昇順で重ならないので二分探索する
    let selected = |byte_abs: usize| {
        let i = sel.partition_point(|&(_, e)| e <= byte_abs);
        sel.get(i).is_some_and(|&(s, _)| s <= byte_abs)
    };
    let (win_start, win_end) = (left, left + cols);
    let (mut clipped_left, mut clipped_right, mut risky) = (false, false, false);
    let mut spans: Vec<Span> = Vec::new();
    'units: for u in &us {
        let style_base = if selected(ls + u.byte) {
            StyleMods::REVERSE
        } else {
            StyleMods::NONE
        };
        let mut c = u.col;
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
                push_span(&mut spans, text.clone(), w as u16, style);
            } else {
                // セルの途中で切れる: 見える分のセルを空白で埋める
                let visible = c.min(win_end) - c0.max(win_start);
                clipped_left |= c0 < win_start;
                clipped_right |= c > win_end;
                push_span(&mut spans, " ".repeat(visible), visible as u16, style);
            }
        }
    }
    // 行末の改行が選択に含まれるときは、行末に 1 セルの反転を足す
    let line_end_abs = ls + content.len();
    let eol_selected = has_terminator
        && sel
            .iter()
            .any(|&(s, e)| s <= line_end_abs && e > line_end_abs);
    if eol_selected && total >= win_start && total < win_end {
        push_span(
            &mut spans,
            " ".to_owned(),
            1,
            Style::with_mods(StyleMods::REVERSE),
        );
    }
    let cursor_cols = heads
        .iter()
        .map(|&h| {
            let rel = h.saturating_sub(ls);
            match us.iter().find(|u| u.byte >= rel) {
                Some(u) => u.col,
                None => total,
            }
        })
        .collect();
    RowOut {
        row: RenderRow {
            source: RowSource::Line {
                line: line as u32,
                wrap_idx: 0,
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
            "	" => (" ".to_owned(), 1, Style::DEFAULT),
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
fn title(doc: &Document) -> String {
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
    format!(
        "{}{} - evoeditor",
        if doc.is_dirty() { "* " } else { "" },
        name
    )
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
            gutter_spans(line, gw),
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
        focused: true,
    };
    Some(Frame {
        window,
        size: CellSize {
            cols: cols as u16,
            rows: rows as u16,
        },
        panes: vec![pane],
        focus: FocusTarget::Pane(view_id),
        primary_cursor: primary.map(|pos| CursorPlacement {
            pos,
            shape: CursorShape::Bar,
        }),
        status: super::chrome::status_model(state, view_id, chord),
        hints: super::chrome::hint_model(state, chord),
        messages: message_model(state),
        texts: super::chrome::chrome_texts(state),
        title: title(doc),
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
