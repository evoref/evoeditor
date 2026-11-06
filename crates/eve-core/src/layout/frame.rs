//! 編集面の Frame の生成 (可視範囲の行・カーソル・メッセージ・タイトル、docs/02 §8、docs/03 §14)。

use eve_text::{LineIdx, TextRead};
use eve_view::{
    CellPos, CellRect, CellSize, CursorPlacement, CursorShape, CutPolicy, FocusTarget, Frame,
    Message, MessageModel, PaneFrame, RenderRow, RowSource, Span, ViewId, ViewRevision, WindowId,
};

use crate::document::Document;

use super::Affinity;
use super::cursor_in_rows;
use super::rows::Marks;
use super::rows::RowPart;
use super::rows::build_row;
use super::rows::cell_text;
use super::rows::gutter_spans;
use super::rows::gutter_width;
use super::rows::line_number_style;
use super::units::cursor_cell;
use super::units::line_content;
use super::units::units_for;
use super::wrap_line;
use crate::editor::state::EditorState;

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
    view: &crate::editor::state::View,
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
    let cursor_line = t.byte_to_line(eve_text::ByteOff(primary_head)).0;
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
        let (ls, content) = line_content(doc, line);
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
                gutter_spans(line, gw, line_number_style(marks.roles, line, cursor_line))
            } else if gw > 0 {
                vec![Span {
                    text: " ".repeat(gw as usize),
                    width: gw,
                    style: marks.roles.get("ui.line_number"),
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
pub(crate) fn build_frame(
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

    let overlay_open = state.ui.overlay.is_some();
    let bar_open = state.find.search_bar.is_some();
    let found = state.visible_matches(view.doc, top_line, rows);
    let current = state.bar_current();
    let unmappable = state.unmappable_marks_for(view.doc);
    let marks = Marks {
        found: &found,
        current,
        unmappable: &unmappable,
        roles: &state.settings.roles,
    };
    let cursor_line = t.byte_to_line(view.selection.primary().head).0;
    // 長大行を含む文書は折り返さない (`view.wrap` の値は変えない。03 §11.5.1)
    let (out_rows, cursors, primary) = if view.wrap.is_on() && doc.format.long_lines == 0 {
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
                gutter_spans(
                    line,
                    gw,
                    line_number_style(&state.settings.roles, line, cursor_line),
                ),
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
        status: crate::ui_model::chrome::status_model(state, view_id, chord),
        hints: crate::ui_model::chrome::hint_model(state, chord),
        hint_bar: state.term.hint_bar_on,
        overlays: crate::ui_model::chrome::overlay_models(state),
        which_key: None,
        messages: message_model(state),
        texts: crate::ui_model::chrome::chrome_texts(state),
        title: title(state, doc),
        roles: state.settings.roles.clone(),
    })
}

/// 水平スクロールの調整に使う、primary のカーソルの (表示列, セルの幅)。
pub(crate) fn primary_cursor_cell(state: &EditorState, view: ViewId) -> Option<(usize, usize)> {
    let v = state.view(view)?;
    let d = state.document(v.doc)?;
    let head = v.selection.primary().head;
    let line = d.text().byte_to_line(head);
    let ls = d.text().line_to_byte(line).0;
    Some(cursor_cell(d, line.0, head.0.saturating_sub(ls)))
}
