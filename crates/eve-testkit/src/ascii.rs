//! ASCII の Frame (docs/09 §3.4)。シナリオのスナップショットに使う、`Frame` の読みやすい文字列表現。
//!
//! 各行を 1 行の文字列にする。全角など幅 2 以上の文字は 1 文字として出し、継続セルは出さない。
//! 反転表示 (選択) の連続は `«` `»` で囲む。ガターは `show_gutter` で出す。末尾は主カーソルの位置 `cursor: (col,row)` と、
//! フォーカスのペインに主でないカーソルがあるときの `secondary: (col,row) …` (03 §4.5.5)。

use eve_view::{Frame, StyleMods};

/// `render_ascii` の出力の選び方。
#[derive(Clone, Copy, Debug)]
pub struct AsciiOptions {
    /// ガター (行番号の欄) を出す。
    pub show_gutter: bool,
    /// 反転表示 (選択) の連続を `«` `»` で囲む。
    pub mark_selection: bool,
    /// 末尾に主カーソルの位置 `cursor: (col,row)` の行と、主でないカーソルの `secondary:` の行 (あれば) を出す。
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
#[derive(Clone)]
struct Cell {
    text: Option<String>,
    reverse: bool,
}

/// `sel_bg` は選択の背景色 (テーマの `ui.selection`。色の段では選択は反転でなく背景色で示す)。
fn put_spans(cells: &mut Vec<Cell>, spans: &[eve_view::Span], sel_bg: eve_view::Color) {
    for sp in spans {
        let w = sp.width as usize;
        let n = sp.text.chars().count();
        let reverse = sp.style.mods.contains(StyleMods::REVERSE)
            || (sel_bg != eve_view::Color::Default && sp.style.bg == sel_bg);
        // 幅の同じ書記素の連続 (Span の契約、eve_view::Span)。幅を書記素数で割って 1 書記素ずつ置く
        let graphemes: Vec<&str> = if n == w {
            Vec::new()
        } else {
            eve_text::grapheme_indices(&sp.text)
                .into_iter()
                .map(|(_, g)| g)
                .collect()
        };
        if n == w {
            // 1 文字 = 1 セル (ASCII の連続など)
            for c in sp.text.chars() {
                cells.push(Cell {
                    text: Some(c.to_string()),
                    reverse,
                });
            }
        } else if graphemes.len() > 1 && w.is_multiple_of(graphemes.len()) {
            let gw = w / graphemes.len();
            for g in graphemes {
                cells.push(Cell {
                    text: Some(g.to_owned()),
                    reverse,
                });
                for _ in 1..gw {
                    cells.push(Cell {
                        text: None,
                        reverse,
                    });
                }
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
                .map(|k| eve_view::key_notation(&k.strokes, false))
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

/// 1 行のセル (ガターを出すときはガターから) と、本文の始まるセルの位置。
fn row_cells(
    row: &eve_view::RenderRow,
    opts: AsciiOptions,
    sel_bg: eve_view::Color,
) -> (Vec<Cell>, usize) {
    let mut cells: Vec<Cell> = Vec::new();
    if opts.show_gutter {
        put_spans(&mut cells, &row.gutter, sel_bg);
    }
    let body_start = cells.len();
    put_spans(&mut cells, &row.spans, sel_bg);
    (cells, body_start)
}

/// セルの並びを文字列にする (選択の連続を `«` `»` で囲む)。
fn cells_text(cells: &[Cell], body_start: usize, opts: AsciiOptions) -> String {
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
    line
}

/// 選択の外の行末の空白を落として、1 行として足す。
fn push_line(out: &mut String, mut line: String) {
    while line.ends_with(' ') {
        line.pop();
    }
    out.push_str(&line);
    out.push('\n');
}

/// タブの名前の幅の上限 (セル。03 §9.9 の PANE-12 (e)。eve-tui の `chrome::tab_line::TAB_NAME_MAX` と同じ)。
const TAB_NAME_MAX: usize = 32;
/// ASCII の縮退の省略記号。
const ELLIPSIS: &str = "...";

fn plain(t: &str) -> Cell {
    Cell {
        text: Some(t.to_owned()),
        reverse: false,
    }
}

/// `spans` を `max` セルに収める。入らなければ中間を `...` にして先頭と末尾を残す (eve-tui の `truncate_middle` と同じ規則)。
fn truncate_middle(spans: &[eve_view::Span], max: usize) -> Vec<eve_view::Span> {
    let width = |s: &[eve_view::Span]| s.iter().map(|x| usize::from(x.width)).sum::<usize>();
    if width(spans) <= max {
        return spans.to_vec();
    }
    if max < ELLIPSIS.len() {
        return Vec::new();
    }
    let budget = max - ELLIPSIS.len();
    let head_budget = budget / 2;
    let tail_budget = budget - head_budget;
    let mut head = Vec::new();
    let mut acc = 0;
    for s in spans {
        if acc + usize::from(s.width) > head_budget {
            break;
        }
        acc += usize::from(s.width);
        head.push(s.clone());
    }
    let mut tail = Vec::new();
    let mut acc = 0;
    for s in spans[head.len()..].iter().rev() {
        if acc + usize::from(s.width) > tail_budget {
            break;
        }
        acc += usize::from(s.width);
        tail.push(s.clone());
    }
    tail.reverse();
    head.push(eve_view::Span::new(ELLIPSIS, 3, eve_view::Style::DEFAULT));
    head.extend(tail);
    head
}

/// タブ 1 つのセル (`[`・印の欄 (`>` か空白)・項目 (間は 1 セル)・空白・`]`。PANE-12 (e))。名前 (`cut` が中間の項目) は
/// `name_max` セルまで、印は短い形があれば短い形。
fn tab_cells(tab: &eve_view::TabItem, active: bool, name_max: usize) -> Vec<Cell> {
    let mut cells = vec![plain("["), plain(if active { ">" } else { " " })];
    for (i, item) in tab.items.iter().enumerate() {
        if i > 0 {
            cells.push(plain(" "));
        }
        let spans = if item.long.cut == eve_view::CutPolicy::Middle {
            truncate_middle(&item.long.spans, name_max)
        } else {
            item.short.as_ref().unwrap_or(&item.long).spans.clone()
        };
        put_spans(&mut cells, &spans, eve_view::Color::Default);
    }
    cells.push(plain(" "));
    cells.push(plain("]"));
    for c in &mut cells {
        c.reverse = false;
    }
    cells
}

/// タブの並べ方 (PANE-12 (e)。eve-tui の `chrome::tab_line::arrange` と同じ手順): (出す範囲, 最初の列, 左を省いた,
/// 右を省いた, アクティブだけでも入らないときにそのタブに使える幅)。
fn arrange(
    widths: &[usize],
    active: usize,
    cols: usize,
) -> (std::ops::Range<usize>, usize, bool, bool, Option<usize>) {
    let n = widths.len();
    if n == 0 {
        return (0..0, 0, false, false, None);
    }
    let a = active.min(n - 1);
    if widths.iter().sum::<usize>() + (n - 1) <= cols {
        return (0..n, 0, false, false, None);
    }
    let avail = cols.saturating_sub(2);
    if widths[a] > avail {
        return (a..a + 1, 1, a > 0, a + 1 < n, Some(avail));
    }
    let mut used = widths[a];
    let (mut lo, mut hi) = (a, a + 1);
    let (mut right, mut left) = (true, true);
    while right || left {
        if right {
            match widths.get(hi) {
                Some(&w) if used + 1 + w <= avail => {
                    used += 1 + w;
                    hi += 1;
                }
                _ => right = false,
            }
        }
        if left {
            match lo.checked_sub(1).map(|i| widths[i]) {
                Some(w) if used + 1 + w <= avail => {
                    used += 1 + w;
                    lo -= 1;
                }
                _ => left = false,
            }
        }
    }
    (lo..hi, 1, lo > 0, hi < n, None)
}

/// タブラインの 1 行の文字列 (03 §9.9 の PANE-12 (h))。eve-tui と同じ並べ方を、ASCII の縮退の記号 (アクティブの印は `>`、
/// 省いた側は `<` / `>`、省略記号は `...`) で出す。末尾の空白は落とす。
pub fn tab_line_text(model: &eve_view::TabLineModel, cols: u16) -> String {
    let cols = usize::from(cols);
    let active = model.active.unwrap_or(usize::MAX);
    let tabs: Vec<Vec<Cell>> = model
        .tabs
        .iter()
        .enumerate()
        .map(|(i, t)| tab_cells(t, i == active, TAB_NAME_MAX))
        .collect();
    let widths: Vec<usize> = tabs.iter().map(Vec::len).collect();
    let (range, start, more_left, more_right, squeeze) = arrange(&widths, active, cols);
    let mut line: Vec<Cell> = (0..start).map(|_| plain(" ")).collect();
    for i in range {
        if line.len() > start {
            line.push(plain(" "));
        }
        let cells = match squeeze {
            Some(room) => {
                let fixed = tab_cells(&model.tabs[i], i == active, 0).len();
                tab_cells(
                    &model.tabs[i],
                    i == active,
                    room.saturating_sub(fixed).min(TAB_NAME_MAX),
                )
            }
            None => tabs[i].clone(),
        };
        line.extend(cells);
    }
    while line.len() < cols {
        line.push(plain(" "));
    }
    // 幅で切る所が全角の文字の途中なら、その文字の先頭から空白にする (行の幅を崩さない)
    if line.get(cols).is_some_and(|c| c.text.is_none()) {
        for i in (0..cols).rev() {
            let head = line[i].text.is_some();
            line[i] = plain(" ");
            if head {
                break;
            }
        }
    }
    line.truncate(cols);
    if more_left && cols > 0 {
        line[0] = plain("<");
    }
    if more_right && cols > 0 {
        // 最後の列が全角の文字の続きなら、その文字の先頭を空白にする
        if line[cols - 1].text.is_none() {
            for i in (0..cols - 1).rev() {
                let head = line[i].text.is_some();
                line[i] = plain(" ");
                if head {
                    break;
                }
            }
        }
        line[cols - 1] = plain(">");
    }
    let mut s: String = line.iter().filter_map(|c| c.text.as_deref()).collect();
    while s.ends_with(' ') {
        s.pop();
    }
    s
}

/// 見出しの 1 行のセル (03 §9.9 の PANE-13): フォーカスのペイン (`active`) は `> ` と項目の後ろを `=` で、ほかのペインは
/// 先頭の空白と項目の後ろを `-` で埋める (無色・ASCII でもフォーカスが分かる)。項目は長い形を空白 1 つで並べる。
fn bar_cells(pane: &eve_view::PaneFrame, bar: &eve_view::PaneBarModel) -> Vec<Cell> {
    let plain = |t: &str| Cell {
        text: Some(t.to_owned()),
        reverse: false,
    };
    let mut cells = vec![plain(if pane.active { ">" } else { " " })];
    if pane.active {
        cells.push(plain(" "));
    }
    for (i, item) in bar.items.iter().enumerate() {
        if i > 0 {
            cells.push(plain(" "));
        }
        put_spans(&mut cells, &item.long.spans, eve_view::Color::Default);
    }
    for c in &mut cells {
        c.reverse = false;
    }
    let width = usize::from(bar.rect.cols);
    if cells.len() < width {
        cells.push(plain(" "));
    }
    let fill = if pane.active { "=" } else { "-" };
    while cells.len() < width {
        cells.push(plain(fill));
    }
    // 幅で切る所が全角の文字の途中なら、その文字の先頭から空白にする (行の幅を崩さない)
    if cells.get(width).is_some_and(|c| c.text.is_none()) {
        for i in (0..width).rev() {
            let head = cells[i].text.is_some();
            cells[i] = plain(" ");
            if head {
                break;
            }
        }
    }
    cells.truncate(width);
    cells
}

/// 格子の `y` 行の `x0` 列から、`limit` セルまで `cells` を置く。`body_start` 番目より後ろのセルは選択の印の対象。
fn place(
    grid: &mut [Vec<(Cell, bool)>],
    x0: u16,
    y: u16,
    cells: Vec<Cell>,
    body_start: usize,
    limit: u16,
) {
    let Some(line) = grid.get_mut(usize::from(y)) else {
        return;
    };
    for (i, c) in cells.into_iter().enumerate().take(usize::from(limit)) {
        if let Some(slot) = line.get_mut(usize::from(x0) + i) {
            *slot = (c, i >= body_start);
        }
    }
}

/// ペインが 2 つ以上の Frame: 編集面の大きさの格子に、各ペインの行・見出し (`=` / `-` の埋め)・区切り (`|`) を矩形の位置に
/// 置く (03 §9.9 の PANE-13)。選択の印 (`«` `»`) は本文のセルだけに付ける。
fn render_panes(out: &mut String, frame: &Frame, opts: AsciiOptions, sel_bg: eve_view::Color) {
    let (cols, rows) = (usize::from(frame.size.cols), usize::from(frame.size.rows));
    // (セル, 選択の印を付けてよい本文のセルか)
    let blank = || {
        (
            Cell {
                text: Some(" ".to_owned()),
                reverse: false,
            },
            false,
        )
    };
    let mut grid: Vec<Vec<(Cell, bool)>> = (0..rows)
        .map(|_| (0..cols).map(|_| blank()).collect())
        .collect();
    for p in &frame.panes {
        for (i, row) in p.rows.iter().enumerate().take(usize::from(p.rect.rows)) {
            let (cells, body_start) = row_cells(row, opts, sel_bg);
            let y = p
                .rect
                .row
                .saturating_add(u16::try_from(i).unwrap_or(u16::MAX));
            place(&mut grid, p.rect.col, y, cells, body_start, p.rect.cols);
        }
        if let Some(bar) = &p.bar {
            place(
                &mut grid,
                bar.rect.col,
                bar.rect.row,
                bar_cells(p, bar),
                usize::MAX,
                bar.rect.cols,
            );
        }
    }
    for d in &frame.dividers {
        for y in d.rect.row..d.rect.row.saturating_add(d.rect.rows) {
            let cells = (0..d.rect.cols)
                .map(|_| Cell {
                    text: Some("|".to_owned()),
                    reverse: false,
                })
                .collect();
            place(&mut grid, d.rect.col, y, cells, usize::MAX, d.rect.cols);
        }
    }
    for line in grid {
        let mut s = String::new();
        let mut in_sel = false;
        for (c, body) in &line {
            let sel = opts.mark_selection && *body && c.reverse;
            if sel != in_sel {
                s.push(if sel { '«' } else { '»' });
                in_sel = sel;
            }
            if let Some(t) = &c.text {
                s.push_str(t);
            }
        }
        if in_sel {
            s.push('»');
        }
        push_line(out, s);
    }
}

/// Frame を ASCII の文字列にする。ペインが 1 つなら各行をそのまま、2 つ以上なら矩形の位置に並べる (03 §9.9 の PANE-13)。
/// タブラインがあれば最上行に出す (PANE-12 (h))。
pub fn render_ascii(frame: &Frame, opts: AsciiOptions) -> String {
    let mut out = String::new();
    let sel_bg = frame.roles.get("ui.selection").bg;
    if let Some(t) = &frame.tab_line {
        out.push_str(&tab_line_text(t, frame.size.cols));
        out.push('\n');
    }
    if frame.panes.len() > 1 {
        render_panes(&mut out, frame, opts, sel_bg);
    } else {
        for pane in &frame.panes {
            for row in &pane.rows {
                let (cells, body_start) = row_cells(row, opts, sel_bg);
                push_line(&mut out, cells_text(&cells, body_start, opts));
            }
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
    if opts.show_cursor {
        let secondary = secondary_cursors(frame);
        if !secondary.is_empty() {
            let cells: Vec<String> = secondary
                .iter()
                .map(|p| format!("({},{})", p.col, p.row))
                .collect();
            out.push_str(&format!("secondary: {}\n", cells.join(" ")));
        }
    }
    out
}

/// フォーカスのペインの主でないカーソル (03 §4.5.5)。`PaneFrame.cursors` に `rect` の位置を足した編集面の座標で、
/// `primary_cursor` に当たるものを除き、行・列の順に並べる。オーバーレイ・検索バーの間 (`focused` が偽) は出さない
/// (TUI がカーソルを描かないのと同じ)。
pub fn secondary_cursors(frame: &Frame) -> Vec<eve_view::CellPos> {
    let primary = frame.primary_cursor.map(|c| c.pos);
    let mut out: Vec<eve_view::CellPos> = frame
        .panes
        .iter()
        .filter(|p| p.focused)
        .flat_map(|p| {
            p.cursors.iter().map(move |c| eve_view::CellPos {
                col: p.rect.col.saturating_add(c.col),
                row: p.rect.row.saturating_add(c.row),
            })
        })
        .filter(|c| Some(*c) != primary)
        .collect();
    out.sort_by_key(|c| (c.row, c.col));
    out.dedup();
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
                rows: u16::try_from(rows.len()).expect("試験の画面の行数は u16 に収まる"),
            },
            panes: vec![PaneFrame {
                view: ViewId(1),
                rect: CellRect::default(),
                revision: Default::default(),
                gutter_width: 0,
                rows,
                cursors: vec![],
                focused: true,
                active: true,
                bar: None,
            }],
            dividers: Vec::new(),
            tab_line: None,
            search: None,
            focus: FocusTarget::Pane(ViewId(1)),
            primary_cursor: Some(CursorPlacement {
                pos: CellPos { col: 2, row: 0 },
                shape: CursorShape::Bar,
            }),
            status: Default::default(),
            hints: Default::default(),
            hint_bar: true,
            overlays: Vec::new(),
            which_key: None,
            messages: Default::default(),
            texts: Default::default(),
            title: String::new(),
            roles: Default::default(),
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

    #[test]
    fn 複数のペインは矩形の位置に並べる() {
        let pane = |view, col, rows: Vec<RenderRow>| PaneFrame {
            view: ViewId(view),
            rect: CellRect {
                col,
                row: 0,
                cols: 4,
                rows: 2,
            },
            revision: Default::default(),
            gutter_width: 0,
            rows,
            cursors: vec![],
            focused: view == 1,
            active: view == 1,
            bar: None,
        };
        let mut f = frame(vec![]);
        f.size = CellSize { cols: 9, rows: 2 };
        f.panes = vec![
            pane(
                1,
                0,
                vec![row(vec![Span::new("ab", 2, Style::DEFAULT)]), row(vec![])],
            ),
            pane(
                2,
                5,
                vec![
                    row(vec![Span::new("wxyz!", 5, Style::DEFAULT)]),
                    row(vec![Span::new("q", 1, Style::DEFAULT)]),
                ],
            ),
        ];
        f.primary_cursor = None;
        assert_eq!(
            render_ascii(&f, AsciiOptions::default()),
            "ab   wxyz\n     q\n"
        );
    }

    #[test]
    fn 主でないカーソルは_secondary_の行に編集面の座標で行と列の順に出す() {
        let pane = |view, col, cursors: Vec<CellPos>, focused| PaneFrame {
            view: ViewId(view),
            rect: CellRect {
                col,
                row: 1,
                cols: 4,
                rows: 2,
            },
            revision: Default::default(),
            gutter_width: 0,
            rows: vec![row(vec![]), row(vec![])],
            cursors,
            focused,
            active: focused,
            bar: None,
        };
        let at = |col, row| CellPos { col, row };
        let mut f = frame(vec![]);
        f.size = CellSize { cols: 9, rows: 3 };
        // フォーカスのペイン (列 5 から): 主は (6,2) = ペインの (1,1)。ほかのペインのカーソルは出さない
        f.panes = vec![
            pane(1, 0, vec![at(0, 0)], false),
            pane(2, 5, vec![at(2, 1), at(1, 1), at(3, 0)], true),
        ];
        f.primary_cursor.as_mut().expect("主のカーソル").pos = at(6, 2);
        let out = render_ascii(&f, AsciiOptions::default());
        assert!(
            out.ends_with("cursor: (6,2)\nsecondary: (8,1) (7,2)\n"),
            "{out}"
        );
        // 主が画面の外でも出す
        f.primary_cursor = None;
        let out = render_ascii(&f, AsciiOptions::default());
        assert!(out.ends_with("secondary: (8,1) (6,2) (7,2)\n"), "{out}");
        // オーバーレイの間 (`focused` が偽) と、主でないカーソルが無いときは出さない
        f.panes[1].focused = false;
        assert!(!render_ascii(&f, AsciiOptions::default()).contains("secondary"));
        f.panes[1].focused = true;
        f.panes[1].cursors = vec![at(1, 1)];
        f.primary_cursor = frame(vec![]).primary_cursor;
        f.primary_cursor.as_mut().expect("主のカーソル").pos = at(6, 2);
        assert!(!render_ascii(&f, AsciiOptions::default()).contains("secondary"));
    }

    #[test]
    fn 見出しの全角の名前が幅を超えるときは途中で切らず空白にする() {
        let item = |text: &str, width: u16| eve_view::StatusItem {
            kind: eve_view::StatusKind::Path,
            long: eve_view::CellText {
                spans: vec![Span::new(text, width, Style::DEFAULT)],
                width,
                cut: eve_view::CutPolicy::Middle,
            },
            short: None,
            priority: 4,
            align: eve_view::StatusAlign::Left,
        };
        let bar = |cols| eve_view::PaneBarModel {
            rect: CellRect {
                col: 0,
                row: 0,
                cols,
                rows: 1,
            },
            items: vec![item("あいう", 6)],
        };
        let pane = |active| PaneFrame {
            view: ViewId(1),
            rect: CellRect::default(),
            revision: Default::default(),
            gutter_width: 0,
            rows: vec![],
            cursors: vec![],
            focused: active,
            active,
            bar: None,
        };
        let text =
            |cells: &[Cell]| -> String { cells.iter().filter_map(|c| c.text.as_deref()).collect() };
        // `> ` + `あい` + `う` の左半分で切れる: `う` を空白にして 5 セル
        let cells = bar_cells(&pane(true), &bar(5));
        assert_eq!(cells.len(), 5);
        assert_eq!(text(&cells), "> あ ");
        // 全角の境目で切れる: そのまま
        let cells = bar_cells(&pane(true), &bar(6));
        assert_eq!(text(&cells), "> あい");
        // フォーカスの無いペイン (` ` + `あいう`): 4 セルで `い` の左半分
        let cells = bar_cells(&pane(false), &bar(4));
        assert_eq!((cells.len(), text(&cells)), (4, " あ ".to_owned()));
        // 幅に余れば `-` / `=` で埋める
        assert_eq!(text(&bar_cells(&pane(false), &bar(10))), " あいう --");
        assert_eq!(text(&bar_cells(&pane(true), &bar(10))), "> あいう =");
    }
}
