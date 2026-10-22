//! ASCII の Frame (docs/09 §3.4)。シナリオのスナップショットに使う、`Frame` の読みやすい文字列表現。
//!
//! 各行を 1 行の文字列にする。全角など幅 2 以上の文字は 1 文字として出し、継続セルは出さない。
//! 反転表示 (選択) の連続は `«` `»` で囲む。ガターは `show_gutter` で出す。末尾は主カーソルの位置 `cursor: (col,row)`。

use eve_view::{Frame, StyleMods};

#[derive(Clone, Copy, Debug)]
pub struct AsciiOptions {
    pub show_gutter: bool,
    pub mark_selection: bool,
    pub show_cursor: bool,
    /// ステータス・ヒント・メッセージ行を末尾に出す。
    pub show_chrome: bool,
}

impl Default for AsciiOptions {
    fn default() -> AsciiOptions {
        AsciiOptions {
            show_gutter: false,
            mark_selection: true,
            show_cursor: true,
            show_chrome: false,
        }
    }
}

/// 1 セル分の内容。継続セルは `None`。
struct Cell {
    text: Option<String>,
    reverse: bool,
}

fn put_spans(cells: &mut Vec<Cell>, spans: &[eve_view::Span]) {
    for sp in spans {
        let w = sp.width as usize;
        let n = sp.text.chars().count();
        let reverse = sp.style.mods.contains(StyleMods::REVERSE);
        if n == w {
            // 1 文字 = 1 セル (ASCII の連続など)
            for c in sp.text.chars() {
                cells.push(Cell {
                    text: Some(c.to_string()),
                    reverse,
                });
            }
        } else {
            // 1 書記素 (幅 2 以上、または複数の文字を持つ書記素): 先頭のセルに全体を置く
            cells.push(Cell {
                text: Some(sp.text.clone()),
                reverse,
            });
            for _ in 1..w {
                cells.push(Cell {
                    text: None,
                    reverse,
                });
            }
        }
    }
}

fn cell_text(t: &eve_view::CellText) -> String {
    t.spans.iter().map(|s| s.text.as_str()).collect()
}

/// クローム (ステータス・ヒント・メッセージ) を 1 行ずつの文字列にする。
fn chrome_lines(frame: &Frame) -> String {
    use eve_view::StatusAlign;
    let join = |align: StatusAlign| {
        frame
            .status
            .items
            .iter()
            .filter(|i| i.align == align)
            .map(|i| cell_text(&i.long))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut out = format!(
        "status: {} | {}\n",
        join(StatusAlign::Left),
        join(StatusAlign::Right)
    );
    let hints: Vec<String> = frame
        .hints
        .items
        .iter()
        .map(|h| {
            let keys: Vec<String> = h
                .keys
                .iter()
                .map(|k| {
                    k.strokes
                        .iter()
                        .map(|s| eve_core::format_stroke(*s))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect();
            format!("{} {}", keys.join("/"), cell_text(&h.text))
        })
        .collect();
    out.push_str(&format!("hints: {}\n", hints.join(" | ")));
    if let Some(m) = &frame.messages.current {
        out.push_str(&format!("message[{:?}]: {}", m.level, cell_text(&m.text)));
        if let Some(h) = &m.hint {
            out.push_str(&format!(" / {}", cell_text(h)));
        }
        out.push('\n');
    }
    out
}

/// Frame を ASCII の文字列にする (最初の Pane だけ。複数 Pane は分割を入れる版で足す)。
pub fn render_ascii(frame: &Frame, opts: AsciiOptions) -> String {
    let mut out = String::new();
    for pane in &frame.panes {
        for row in &pane.rows {
            let mut cells: Vec<Cell> = Vec::new();
            if opts.show_gutter {
                put_spans(&mut cells, &row.gutter);
            }
            let body_start = cells.len();
            put_spans(&mut cells, &row.spans);
            let mut line = String::new();
            let mut in_sel = false;
            for (i, c) in cells.iter().enumerate() {
                let sel = opts.mark_selection && i >= body_start && c.reverse;
                if sel != in_sel {
                    line.push(if sel { '«' } else { '»' });
                    in_sel = sel;
                }
                if let Some(t) = &c.text {
                    line.push_str(t);
                }
            }
            if in_sel {
                line.push('»');
            }
            // 選択の外の行末の空白は落とす
            while line.ends_with(' ') {
                line.pop();
            }
            out.push_str(&line);
            out.push('\n');
        }
    }
    if opts.show_chrome {
        out.push_str(&chrome_lines(frame));
    }
    if opts.show_cursor
        && let Some(c) = frame.primary_cursor
    {
        out.push_str(&format!("cursor: ({},{})\n", c.pos.col, c.pos.row));
    }
    out
}

#[cfg(test)]
mod tests {
    use eve_view::{
        CellPos, CellRect, CellSize, CursorPlacement, CursorShape, FocusTarget, PaneFrame,
        RenderRow, RowSource, Span, Style, ViewId, WindowId,
    };

    use super::*;

    fn frame(rows: Vec<RenderRow>) -> Frame {
        Frame {
            window: WindowId(1),
            size: CellSize {
                cols: 10,
                rows: rows.len() as u16,
            },
            panes: vec![PaneFrame {
                view: ViewId(1),
                rect: CellRect::default(),
                revision: Default::default(),
                gutter_width: 0,
                rows,
                cursors: vec![],
                focused: true,
            }],
            search: None,
            focus: FocusTarget::Pane(ViewId(1)),
            primary_cursor: Some(CursorPlacement {
                pos: CellPos { col: 2, row: 0 },
                shape: CursorShape::Bar,
            }),
            status: Default::default(),
            hints: Default::default(),
            overlays: Vec::new(),
            messages: Default::default(),
            texts: Default::default(),
            title: String::new(),
        }
    }

    fn row(spans: Vec<Span>) -> RenderRow {
        RenderRow {
            source: RowSource::Line {
                line: 0,
                wrap_idx: 0,
            },
            gutter: vec![],
            spans,
            clipped_left: false,
            clipped_right: false,
            width_risky: false,
        }
    }

    #[test]
    fn 全角と選択と末尾の空白() {
        let rev = Style::with_mods(StyleMods::REVERSE);
        let f = frame(vec![row(vec![
            Span::new("a", 1, Style::DEFAULT),
            Span::new("あ", 2, Style::DEFAULT),
            Span::new("bc", 2, rev),
            Span::new("  ", 2, Style::DEFAULT),
        ])]);
        assert_eq!(
            render_ascii(&f, AsciiOptions::default()),
            "aあ«bc»\ncursor: (2,0)\n"
        );
        let plain = AsciiOptions {
            mark_selection: false,
            show_cursor: false,
            ..AsciiOptions::default()
        };
        assert_eq!(render_ascii(&f, plain), "aあbc\n");
    }
}
