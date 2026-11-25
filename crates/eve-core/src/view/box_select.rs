//! 矩形 (ボックス) 選択 (FR-212、docs/03 §4.8 の BOX-1〜BOX-9、07 §11.6 の「編集の拡張」、01 FR-212 AC-1 / AC-2)。
//!
//! - `select.toggle_box`: View の矩形選択のモードを入れる・切る (BOX-1。`Ui`)。
//! - モードの間の `select.extend_*`・左のドラッグ・`Shift` 付きの左のクリックは、View の矩形 (論理行と行頭からの表示列の
//!   2 つの角) を作る・動かし、矩形の各行に 1 つの Range を持つ Selection にする (BOX-2 / BOX-3 / BOX-6)。
//! - 矩形の Selection は複数カーソルと同じ規則で編集し、コピー・貼り付けは矩形の高さの片で扱う (BOX-4)。
//!
//! 新しい選択の型は作らない (03 §4.5 の 7)。矩形は、作ったときの文書の Revision と Selection の写しとともに View に持ち、
//! どちらかが変われば無いものとする (03 §4.5.1 の MCUR-2 (j) の「単語の一致」の印と同じ持ち方)。
//!
//! spec: 03 §4.8, 03 §4.5, 03 §4.5.2, 03 §4.5.3, 01 FR-212

use eve_text::{
    ByteOff, ByteRange, LineIdx, MAX_CURSORS, Range, Selection, TextRead, next_grapheme_boundary,
    prev_grapheme_boundary,
};
use eve_view::{Revision, ViewId};

use crate::command::{
    CommandCtx, CommandKind, CommandRegistry, CommandResult, Permission, RejectReason, ids,
};
use crate::document::Document;
use crate::editor::state::EditorState;
use crate::layout::{byte_at_col, cursor_cell, cursor_col};
use crate::view::motion::{indent_end, word_stop};

/// 矩形の角 (論理行, 行頭からの表示列)。表示列は行の幅より右 (仮想の列) でもよい (03 §4.8 の BOX-2)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BoxPoint {
    pub(crate) line: usize,
    pub(crate) col: usize,
}

/// 矩形 (03 §4.8 の BOX-2)。`anchor` は動かさない角、`head` は選択を伸ばす操作で動かす角。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BoxRect {
    pub(crate) anchor: BoxPoint,
    pub(crate) head: BoxPoint,
}

impl BoxRect {
    /// 1 点の矩形 (幅 0・高さ 1)。
    pub(crate) fn at(p: BoxPoint) -> BoxRect {
        BoxRect { anchor: p, head: p }
    }

    fn top(&self) -> usize {
        self.anchor.line.min(self.head.line)
    }

    fn bottom(&self) -> usize {
        self.anchor.line.max(self.head.line)
    }

    /// 矩形の行の数 (`anchor` と `head` の行の差 + 1)。
    pub(crate) fn rows(&self) -> usize {
        self.bottom() - self.top() + 1
    }

    /// 左の列 `l` (2 つの列の小さいほう)。
    fn left(&self) -> usize {
        self.anchor.col.min(self.head.col)
    }

    /// 右の列 `r` (2 つの列の大きいほう)。
    fn right(&self) -> usize {
        self.anchor.col.max(self.head.col)
    }

    /// 列の数 (`r - l`)。
    pub(crate) fn cols(&self) -> usize {
        self.right() - self.left()
    }
}

/// View の矩形の印 (03 §4.8 の BOX-2)。矩形から Selection を作ったときの文書の Revision と Selection を持ち、今の文書の
/// Revision と Selection が写しと同じ間だけ有効。
#[derive(Clone, Debug)]
pub(crate) struct BoxMark {
    rect: BoxRect,
    revision: Revision,
    selection: Selection,
}

/// 矩形の `head` の動かし方 (03 §4.8 の BOX-2。モードの間の `select.extend_*`)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BoxMove {
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    Home,
    End,
    PageUp,
    PageDown,
    DocStart,
    DocEnd,
}

/// ステータスバーの選択情報に出すもの (03 §4.8 の BOX-7)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BoxStatus {
    /// モードで、矩形が無い。
    Mode,
    /// モードで、矩形がある (行の数, 列の数)。
    Rect { rows: usize, cols: usize },
}

/// 矩形の行の数が上限 (`MAX_CURSORS`) を超える (03 §4.8 の BOX-3。行を走査せずに断る)。
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct TooManyRows;

/// 行 `line` の (行頭, 行末) のバイト位置 (行末は区切りの前)。
fn line_bounds(t: &dyn TextRead, line: usize) -> (usize, usize) {
    (
        t.line_to_byte(LineIdx(line)).0,
        t.line_end_byte(LineIdx(line)).0,
    )
}

/// 行 `line` (行頭 `ls`・行末 `le`) の表示の幅が列 `col` に届く (幅 ≥ `col`) か。表示列 `col` の位置が行の中なら届いており、
/// 行末に達したときだけ行末の列を求める (長大行でも、列の索引を作るのは `col` までか、`col` より短い行の行末まで。
/// 03 §4.8 の BOX-8 / BOX-9)。
fn reaches(doc: &Document, line: usize, (ls, le): (usize, usize), col: usize) -> bool {
    ls + byte_at_col(doc, line, col) < le || cursor_col(doc, line, le - ls) >= col
}

/// 行 `line` (行頭 `ls`・行末 `le`) の表示列 `col` の位置 (行頭からのバイト)。書記素の途中に落ちたらその書記素の後ろ
/// (掛かった文字を含める)。行の幅以上なら行末。
fn ceil_at(doc: &Document, line: usize, (ls, le): (usize, usize), col: usize) -> usize {
    let p = byte_at_col(doc, line, col);
    if ls + p >= le {
        return le - ls;
    }
    let (c, _) = cursor_cell(doc, line, p);
    if c >= col {
        return p;
    }
    let next = next_grapheme_boundary(doc.text(), ByteOff(ls + p))
        .0
        .min(le);
    next - ls
}

/// 矩形の各行の Range と、主の Range (`head` の行の Range) の番号 (03 §4.8 の BOX-3)。先に行の数を上限と比べ、超えれば
/// 行を走査せずに `TooManyRows`。左の列に届かない行には Range を作らない (`head` の行は届かなければ行末の空のカーソル)。
/// Range の向きは、`head` の列が `anchor` の列以上なら前向き。
pub(crate) fn box_ranges(
    doc: &Document,
    rect: BoxRect,
) -> Result<(Vec<Range>, usize), TooManyRows> {
    if rect.rows() > MAX_CURSORS {
        return Err(TooManyRows);
    }
    let t = doc.text();
    let (l, r) = (rect.left(), rect.right());
    let forward = rect.head.col >= rect.anchor.col;
    let mut out = Vec::with_capacity(rect.rows());
    let mut primary = 0;
    for line in rect.top()..=rect.bottom() {
        let (ls, le) = line_bounds(t, line);
        let range = if !reaches(doc, line, (ls, le), l) {
            if line != rect.head.line {
                continue;
            }
            Range::point(ByteOff(le))
        } else {
            let start = ByteOff(ls + byte_at_col(doc, line, l));
            if l == r {
                Range::point(start)
            } else {
                let end = ByteOff(ls + ceil_at(doc, line, (ls, le), r));
                if forward {
                    Range::new(start, end)
                } else {
                    Range::new(end, start)
                }
            }
        };
        if line == rect.head.line {
            primary = out.len();
        }
        out.push(range);
    }
    Ok((out, primary))
}

/// 行 `line` の位置 `p` (絶対のバイト) の表示列。
fn col_of(doc: &Document, line: usize, ls: usize, p: usize) -> usize {
    cursor_col(doc, line, p - ls)
}

/// 矩形の行のうち、表示の幅が列 `col` に届く行があるか (`select.extend_right` の止まり方: 次の列 `col + 1` は、矩形の行の
/// 最も長い行の幅 + 1 以下。03 §4.8 の BOX-2)。行ごとに `col` の位置を引くだけで、行の全体の幅は求めない。
fn any_reaches(doc: &Document, rect: BoxRect, col: usize) -> bool {
    let t = doc.text();
    (rect.top()..=rect.bottom()).any(|line| reaches(doc, line, line_bounds(t, line), col))
}

/// 矩形の行のうち最も長い行の幅 (`select.extend_end`。各行の行末の列を求めるので、長大行では行の長さに比例する。
/// 03 §4.8 の BOX-9)。
fn widest(doc: &Document, rect: BoxRect) -> usize {
    let t = doc.text();
    (rect.top()..=rect.bottom())
        .map(|line| {
            let (ls, le) = line_bounds(t, line);
            cursor_col(doc, line, le - ls)
        })
        .max()
        .unwrap_or(0)
}

/// `select.extend_right` を `count` 回 (03 §4.8 の BOX-2)。行の中は書記素の境界をバイトで `count` 回進めてから 1 回だけ列に
/// 変え、行の幅より右 (仮想の列) は 1 列ずつ進めて、矩形の行の最も長い行の幅 + 1 列で止める (すでに右にいれば動かさない)。
fn right_of(doc: &Document, rect: BoxRect, count: usize) -> usize {
    let t = doc.text();
    let head = rect.head;
    let (ls, le) = line_bounds(t, head.line);
    let mut left = count;
    let mut col = head.col;
    let mut p = ls + byte_at_col(doc, head.line, col);
    if p < le {
        while left > 0 && p < le {
            p = next_grapheme_boundary(t, ByteOff(p)).0.min(le);
            left -= 1;
        }
        col = col_of(doc, head.line, ls, p);
    }
    if left == 0 {
        return col;
    }
    // 仮想の列: 列 `c` に届く行があれば `c + 1` へ進める。届くかは列について単調なので、最も遠い列から二分で探す
    let want = col.saturating_add(left);
    if any_reaches(doc, rect, want - 1) {
        return want;
    }
    if !any_reaches(doc, rect, col) {
        return col;
    }
    // any_reaches(lo) は真、any_reaches(hi) は偽
    let (mut lo, mut hi) = (col, want - 1);
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if any_reaches(doc, rect, mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo + 1
}

/// `select.extend_left` を `count` 回 (03 §4.8 の BOX-2)。行の幅より右 (仮想の列) は 1 列ずつ行の幅まで戻し、行の中は書記素の
/// 途中なら始まりへ、境界なら前の書記素の境界へ、バイトで `count` 回戻してから 1 回だけ列に変える。列 0 では動かない。
fn left_of(doc: &Document, head: BoxPoint, count: usize) -> usize {
    let t = doc.text();
    let (ls, le) = line_bounds(t, head.line);
    let mut left = count;
    let mut col = head.col;
    let mut p = ls + byte_at_col(doc, head.line, col);
    if p >= le {
        let width = col_of(doc, head.line, ls, le);
        if col > width {
            let back = left.min(col - width);
            col -= back;
            left -= back;
        }
        p = le;
    }
    if left == 0 {
        return col;
    }
    if col_of(doc, head.line, ls, p) < col {
        // 書記素の途中 (全角・タブ): その書記素の始まりが前の境界
        left -= 1;
    }
    while left > 0 && p > ls {
        p = prev_grapheme_boundary(t, ByteOff(p)).0.max(ls);
        left -= 1;
    }
    col_of(doc, head.line, ls, p)
}

/// 単語の移動を 1 回した列 (03 §4.8 の BOX-2。行の外へは出ない)。`select.extend_word_right` は列を左へ戻さない (行末・仮想の
/// 列では動かない)。
fn word_col(doc: &Document, head: BoxPoint, forward: bool) -> usize {
    let (ls, le) = line_bounds(doc.text(), head.line);
    let p = ls + byte_at_col(doc, head.line, head.col);
    let q = word_stop(doc, ByteOff(p), forward).0.clamp(ls, le);
    let c = col_of(doc, head.line, ls, q);
    if forward {
        c.max(head.col)
    } else {
        c.min(head.col)
    }
}

/// 矩形の `head` を `count` 回動かした角 (03 §4.8 の BOX-2)。`page` はページ移動の行の数 (`viewport_rows - 1`)。行を動かす
/// 移動は列を保ち、文書の端で止まる (文書の先頭 / 末尾へ寄せない)。
pub(crate) fn moved_head(
    doc: &Document,
    rect: BoxRect,
    m: BoxMove,
    count: usize,
    page: usize,
) -> BoxPoint {
    let head = rect.head;
    let last = doc.text().len_lines().saturating_sub(1);
    let line = |l: usize| BoxPoint { line: l, ..head };
    let col = |c: usize| BoxPoint { col: c, ..head };
    match m {
        BoxMove::Up => line(head.line.saturating_sub(count)),
        BoxMove::Down => line(head.line.saturating_add(count).min(last)),
        BoxMove::PageUp => line(head.line.saturating_sub(page.saturating_mul(count))),
        BoxMove::PageDown => line(
            head.line
                .saturating_add(page.saturating_mul(count))
                .min(last),
        ),
        BoxMove::DocStart => line(0),
        BoxMove::DocEnd => line(last),
        BoxMove::Left => col(left_of(doc, head, count)),
        BoxMove::Right => col(right_of(doc, rect, count)),
        BoxMove::WordLeft | BoxMove::WordRight => {
            let forward = m == BoxMove::WordRight;
            let mut h = head;
            for _ in 0..count {
                let next = BoxPoint {
                    col: word_col(doc, h, forward),
                    ..h
                };
                if next == h {
                    break;
                }
                h = next;
            }
            h
        }
        BoxMove::Home => {
            let (ls, le) = line_bounds(doc.text(), head.line);
            col(col_of(doc, head.line, ls, indent_end(doc, ls, le)))
        }
        BoxMove::End => col(widest(doc, rect)),
    }
}

impl EditorState {
    /// View `view` が矩形選択のモードか (03 §4.8 の BOX-1)。
    pub fn box_mode(&self, view: ViewId) -> bool {
        self.docs.views.get(&view).is_some_and(|v| v.box_mode)
    }

    /// 矩形選択のモードを入れる・切る (03 §4.8 の BOX-1)。入れても選択は変えない。切ると矩形を消し、今の Range を複数
    /// カーソルとしてそのまま残す。
    pub(crate) fn set_box_mode(&mut self, view: ViewId, on: bool) {
        if let Some(v) = self.docs.views.get_mut(&view) {
            v.box_mode = on;
            if !on {
                v.box_mark = None;
            }
            self.out.needs_render = true;
        }
    }

    /// 矩形を消す (モードは残す。今の Range は複数カーソルとして残る)。
    pub(crate) fn clear_box_rect(&mut self, view: ViewId) {
        if let Some(v) = self.docs.views.get_mut(&view)
            && v.box_mark.take().is_some()
        {
            self.out.needs_render = true;
        }
    }

    /// View `view` の有効な矩形 (モードの間で、作ったときから文書と選択が変わっていない)。
    pub(crate) fn box_rect(&self, view: ViewId) -> Option<BoxRect> {
        let v = self.docs.views.get(&view)?;
        if !v.box_mode {
            return None;
        }
        let d = self.docs.documents.get(&v.doc)?;
        v.box_mark
            .as_ref()
            .filter(|m| m.revision == d.revision() && m.selection == v.selection)
            .map(|m| m.rect)
    }

    /// View `view` の有効な矩形の (行の数, 列の数) (03 §4.8 の BOX-7 と同じ数え方)。矩形が無ければ `None`。
    pub fn box_size(&self, view: ViewId) -> Option<(usize, usize)> {
        self.box_rect(view).map(|r| (r.rows(), r.cols()))
    }

    /// View `view` の有効な矩形の (最初の行, 最後の行) (論理行、0 始まり)。矩形が無ければ `None`。
    pub fn box_lines(&self, view: ViewId) -> Option<(usize, usize)> {
        self.box_rect(view).map(|r| (r.top(), r.bottom()))
    }

    /// ステータスバーの選択情報 (03 §4.8 の BOX-7)。モードでなければ `None`。
    pub(crate) fn box_status(&self, view: ViewId) -> Option<BoxStatus> {
        if !self.box_mode(view) {
            return None;
        }
        Some(match self.box_rect(view) {
            Some(r) => BoxStatus::Rect {
                rows: r.rows(),
                cols: r.cols(),
            },
            None => BoxStatus::Mode,
        })
    }

    /// 矩形が無いときに作る矩形: 主の Range の `head` の (行, 表示列) (03 §4.8 の BOX-2)。
    fn box_from_primary(&self, view: ViewId) -> Option<BoxRect> {
        let v = self.docs.views.get(&view)?;
        let d = self.docs.documents.get(&v.doc)?;
        let t = d.text();
        let head = v.selection.primary().head;
        let line = t.byte_to_line(head).0;
        let ls = t.line_to_byte(LineIdx(line)).0;
        Some(BoxRect::at(BoxPoint {
            line,
            col: cursor_col(d, line, head.0 - ls),
        }))
    }

    /// 今の矩形 (無ければ主の Range の `head` から作った 1 点の矩形)。
    pub(crate) fn box_rect_or_primary(&self, view: ViewId) -> Option<BoxRect> {
        self.box_rect(view).or_else(|| self.box_from_primary(view))
    }

    /// 矩形 `rect` を View `view` の Selection にして、矩形の印を置く (03 §4.8 の BOX-3)。行の数が上限を超えるなら、行を
    /// 走査せずに断り (直前の矩形と Selection のまま) `false`。知らせは呼び側が出す (`apply_box`。ドラッグは上限以下から
    /// 上限超えへ変わったときだけ)。スクロールはしない (キーの操作は呼び側がカーソルを見えるようにする。マウスのドラッグは
    /// 今のスクロールと自動スクロールのまま)。前向きの矩形の主の `head` の帰属は、折り返し位置なら上の表示行の末尾
    /// (`Before`。03 §5.8.2 の CLK-4 の単語の単位と同じ)。
    pub(crate) fn try_apply_box(&mut self, view: ViewId, rect: BoxRect) -> bool {
        let Some(d) = self
            .docs
            .views
            .get(&view)
            .and_then(|v| self.docs.documents.get(&v.doc))
        else {
            return false;
        };
        let revision = d.revision();
        let Ok((ranges, primary)) = box_ranges(d, rect) else {
            return false;
        };
        let Some(sel) = Selection::new(ranges, primary) else {
            return false;
        };
        let head = *sel.primary();
        // 目標の列・帰属・「単語の一致」の印・選択の拡張の履歴は消える (`set_selection`)
        self.set_selection(view, sel);
        if head.head > head.anchor {
            self.set_cursor_affinity(view, crate::layout::Affinity::Before);
        }
        if let Some(v) = self.docs.views.get_mut(&view) {
            v.box_mark = Some(BoxMark {
                rect,
                revision,
                selection: v.selection.clone(),
            });
        }
        true
    }

    /// `try_apply_box` と同じだが、断ったら `info.cursor.limit` を出す (キーの 1 押し・`Shift` 付きの押下に 1 回)。
    pub(crate) fn apply_box(&mut self, view: ViewId, rect: BoxRect) -> bool {
        let applied = self.try_apply_box(view, rect);
        if !applied {
            self.notify_cursor_limit();
        }
        applied
    }

    /// モードの間の `select.extend_*` (03 §4.8 の BOX-2): 矩形 (無ければ主の Range の `head` から作る) の `head` を `count`
    /// 回動かして Selection にする。
    pub(crate) fn box_extend(&mut self, view: ViewId, m: BoxMove, count: usize) -> CommandResult {
        let Some(rect) = self.box_rect_or_primary(view) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let page = usize::from(self.view_size(view).1).saturating_sub(1).max(1);
        let Some(d) = self
            .docs
            .views
            .get(&view)
            .and_then(|v| self.docs.documents.get(&v.doc))
        else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let rect = BoxRect {
            anchor: rect.anchor,
            head: moved_head(d, rect, m, count, page),
        };
        if self.apply_box(view, rect) {
            self.ensure_cursor_visible(view);
        }
        CommandResult::Done
    }

    /// 矩形のコピーの片 (03 §4.8 の BOX-4): 矩形の行ごとに 1 つ (Range の無い短い行は空の片)。矩形が無い・どの Range も
    /// 空なら `None` (複数カーソルのコピーと同じ扱い)。
    pub(crate) fn box_copy_pieces(&self, view: ViewId) -> Option<Vec<ByteRange>> {
        let rect = self.box_rect(view)?;
        let v = self.docs.views.get(&view)?;
        let t = self.docs.documents.get(&v.doc)?.text();
        let ranges = v.selection.ranges();
        if ranges.iter().all(Range::is_empty) {
            return None;
        }
        let rows = rows_of(t, rect, ranges);
        Some(
            rows.iter()
                .enumerate()
                .map(|(i, row)| match row {
                    Some(k) => ByteRange::new(ranges[*k].start(), ranges[*k].end()),
                    None => {
                        let ls = t.line_to_byte(LineIdx(rect.top() + i));
                        ByteRange::new(ls, ls)
                    }
                })
                .collect(),
        )
    }

    /// 矩形への貼り付けの割り振り (03 §4.8 の BOX-4): (矩形の高さ, 各 Range (文書の順) の矩形の行の番号)。矩形が無ければ
    /// `None`。
    pub(crate) fn box_paste_rows(&self, view: ViewId) -> Option<(usize, Vec<usize>)> {
        let rect = self.box_rect(view)?;
        let v = self.docs.views.get(&view)?;
        let t = self.docs.documents.get(&v.doc)?.text();
        let rows = rows_of(t, rect, v.selection.ranges());
        let mut of_range = vec![0; v.selection.len()];
        for (i, row) in rows.iter().enumerate() {
            if let Some(k) = row {
                of_range[*k] = i;
            }
        }
        Some((rect.rows(), of_range))
    }
}

/// 矩形の各行 (上から) の Range の番号 (Range の無い短い行は `None`)。`ranges` は矩形から作った Selection の Range
/// (文書の順で、各行に 1 つまで)。
fn rows_of(t: &dyn TextRead, rect: BoxRect, ranges: &[Range]) -> Vec<Option<usize>> {
    let mut out = vec![None; rect.rows()];
    for (k, r) in ranges.iter().enumerate() {
        let line = t.byte_to_line(r.start()).0;
        if let Some(slot) = line.checked_sub(rect.top()).and_then(|i| out.get_mut(i)) {
            *slot = Some(k);
        }
    }
    out
}

/// `select.toggle_box` (03 §4.8 の BOX-1)。
fn toggle_box(ctx: &mut CommandCtx<'_>) -> CommandResult {
    let Some(view) = ctx.state.focused_view() else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let on = !ctx.state.box_mode(view);
    ctx.state.set_box_mode(view, on);
    CommandResult::Done
}

/// 矩形選択のコマンド (07 §11.6 の「編集の拡張」。`Ui`・`ui`・`when` なし。文書を変えず、Undo の履歴に残らない)。
pub(crate) fn register(r: &mut CommandRegistry) {
    r.add(
        ids::SELECT_TOGGLE_BOX,
        CommandKind::Ui,
        Permission::Ui,
        "",
        toggle_box,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, TextFormat};

    fn doc(text: &str) -> Document {
        Document::from_text(eve_text::Text::from(text), TextFormat::new_document())
    }

    fn pt(line: usize, col: usize) -> BoxPoint {
        BoxPoint { line, col }
    }

    fn rect(a: (usize, usize), h: (usize, usize)) -> BoxRect {
        BoxRect {
            anchor: pt(a.0, a.1),
            head: pt(h.0, h.1),
        }
    }

    /// 各 Range の (anchor, head) のバイト位置。
    fn spans(d: &Document, r: BoxRect) -> (Vec<(usize, usize)>, usize) {
        let (ranges, primary) = box_ranges(d, r).expect("上限以下");
        (
            ranges.iter().map(|r| (r.anchor.0, r.head.0)).collect(),
            primary,
        )
    }

    // spec: 03 §4.8 (BOX-3)
    #[test]
    fn 行の数が上限を超える矩形は行を走査せずに断る() {
        // 文書は 1 行だけ。行の番号は文書の外でも、走査の前に断るので読まない
        let d = doc("a");
        let r = rect((0, 0), (MAX_CURSORS, 0));
        assert_eq!(r.rows(), MAX_CURSORS + 1);
        assert_eq!(box_ranges(&d, r), Err(TooManyRows));
    }

    // spec: 03 §4.8 (BOX-3)
    #[test]
    fn 全角とタブの途中に落ちる列はその文字を含める() {
        // 1 行目: `aあb` (あ は列 1〜2)。2 行目: `\tx` (タブは列 0〜3、tab_width 4)
        let d = doc("aあb\n\tx");
        // 列 2〜3: あ の右半分から b の前まで → あ を含める
        let (s, _) = spans(&d, rect((0, 2), (0, 3)));
        assert_eq!(s, vec![(1, 4)]);
        // 列 1〜2 (あ の左半分だけ) → あ を含める
        let (s, _) = spans(&d, rect((0, 1), (0, 2)));
        assert_eq!(s, vec![(1, 4)]);
        // 2 行目の列 2〜5: タブの途中から → タブと x
        let (s, _) = spans(&d, rect((1, 2), (1, 5)));
        assert_eq!(s, vec![(6, 8)]);
    }

    // spec: 03 §4.8 (BOX-3)
    #[test]
    fn 幅_0_の矩形は書記素の前の空のカーソル() {
        let d = doc("aあb\nabcd");
        let (s, p) = spans(&d, rect((0, 2), (1, 2)));
        // 1 行目の列 2 は あ の右半分 → あ の前 (1)、2 行目は列 2 (行頭 6 + 2)
        assert_eq!(s, vec![(1, 1), (8, 8)]);
        assert_eq!(p, 1);
    }

    // spec: 03 §4.8 (BOX-3, BOX-5)
    #[test]
    fn 左の列に届かない行には_range_を作らない_head_の行は行末の空のカーソル() {
        let d = doc("abcdef\nab\nabcdef\nx");
        // 列 3〜5、0 行目から 3 行目 (head) まで。1 行目 (幅 2) と 3 行目 (幅 1) は届かない
        let (s, p) = spans(&d, rect((0, 3), (3, 5)));
        assert_eq!(s, vec![(3, 5), (13, 15), (18, 18)]);
        assert_eq!(p, 2);
        // 幅がちょうど左の列の行は届く (行末の空の Range)
        let (s, _) = spans(&d, rect((0, 2), (1, 4)));
        assert_eq!(s, vec![(2, 4), (9, 9)]);
    }

    // spec: 03 §4.8 (BOX-3)
    #[test]
    fn head_の列が_anchor_より左なら後ろ向き() {
        let d = doc("abcdef\nabcdef");
        let (s, p) = spans(&d, rect((0, 4), (1, 1)));
        assert_eq!(s, vec![(4, 1), (11, 8)]);
        assert_eq!(p, 1);
    }

    // spec: 03 §4.8 (BOX-3)
    #[test]
    fn crlf_の行は区切りの前まで() {
        let d = doc("abc\r\nab\r\nabcd");
        let (s, _) = spans(&d, rect((0, 1), (2, 9)));
        // 行末より右は行末まで (区切りを含めない)
        assert_eq!(s, vec![(1, 3), (6, 7), (10, 13)]);
    }

    /// `head` を `count` 回動かした角 (ページは 3 行)。
    fn moved(d: &Document, r: BoxRect, m: BoxMove, count: usize) -> BoxPoint {
        moved_head(d, r, m, count, 3)
    }

    /// `m` を 1 回ずつ `n` 回続けた列の並び。
    fn cols(d: &Document, mut r: BoxRect, m: BoxMove, n: usize) -> Vec<usize> {
        (0..n)
            .map(|_| {
                r.head = moved(d, r, m, 1);
                r.head.col
            })
            .collect()
    }

    // spec: 03 §4.8 (BOX-2)
    #[test]
    fn 右へは最も長い行の幅_1_列で止まり_左へは仮想の列を_1_列ずつ() {
        let d = doc("abc\nabcdef");
        // 矩形の行は 0〜1 行目 (最も長い行の幅 6 → 7 列で止まる)
        let r = rect((1, 0), (0, 2));
        assert_eq!(cols(&d, r, BoxMove::Right, 8), vec![3, 4, 5, 6, 7, 7, 7, 7]);
        let r = rect((1, 0), (0, 7));
        assert_eq!(cols(&d, r, BoxMove::Left, 8), vec![6, 5, 4, 3, 2, 1, 0, 0]);
        // `count` は 1 回ずつと同じ所で止まる
        assert_eq!(
            moved(&d, rect((1, 0), (0, 2)), BoxMove::Right, 100),
            pt(0, 7)
        );
        assert_eq!(moved(&d, rect((1, 0), (0, 2)), BoxMove::Right, 3), pt(0, 5));
        assert_eq!(moved(&d, rect((1, 0), (0, 7)), BoxMove::Left, 5), pt(0, 2));
        // すでに上限より右 (ドラッグで広げた列) なら右へは動かさない
        assert_eq!(
            moved(&d, rect((1, 0), (0, 12)), BoxMove::Right, 1),
            pt(0, 12)
        );
    }

    // spec: 03 §4.8 (BOX-2)
    #[test]
    fn 全角とタブを左右に動くと書記素の境界の列へ() {
        let d = doc("aあ\tb");
        // 列: a=0, あ=1〜2, タブ=3 (次のタブ位置 4 まで), b=4
        let r = BoxRect::at(pt(0, 0));
        assert_eq!(cols(&d, r, BoxMove::Right, 4), vec![1, 3, 4, 5]);
        assert_eq!(moved(&d, r, BoxMove::Right, 3), pt(0, 4));
        // 全角の右半分 (列 2) から左は あ の始まり、右は あ の後ろ
        let mid = BoxRect::at(pt(0, 2));
        assert_eq!(moved(&d, mid, BoxMove::Left, 1), pt(0, 1));
        assert_eq!(moved(&d, mid, BoxMove::Left, 2), pt(0, 0));
        assert_eq!(moved(&d, mid, BoxMove::Right, 1), pt(0, 3));
    }

    // spec: 03 §4.8 (BOX-2)
    #[test]
    fn 単語の移動は行の外へ出ない() {
        let d = doc("foo bar\nbaz");
        let at = |l, c| BoxRect::at(pt(l, c));
        assert_eq!(moved(&d, at(0, 4), BoxMove::WordRight, 1), pt(0, 7));
        // 行末から右は行末のまま (次の行へ出ない)
        assert_eq!(moved(&d, at(0, 7), BoxMove::WordRight, 1), pt(0, 7));
        // 仮想の列から右へは戻らない
        assert_eq!(moved(&d, at(0, 10), BoxMove::WordRight, 1), pt(0, 10));
        assert_eq!(moved(&d, at(1, 0), BoxMove::WordLeft, 1), pt(1, 0));
        assert_eq!(moved(&d, at(0, 10), BoxMove::WordLeft, 1), pt(0, 4));
        assert_eq!(moved(&d, at(0, 10), BoxMove::WordLeft, 5), pt(0, 0));
    }

    // spec: 03 §4.8 (BOX-2)
    #[test]
    fn 上下とページと文書の端は列を保つ() {
        let d = doc("a\nb\nc\nd\ne");
        let r = BoxRect::at(pt(2, 9));
        assert_eq!(moved(&d, r, BoxMove::Up, 1), pt(1, 9));
        assert_eq!(moved(&d, r, BoxMove::Down, 1), pt(3, 9));
        assert_eq!(moved(&d, r, BoxMove::Down, 9), pt(4, 9));
        assert_eq!(moved(&d, r, BoxMove::PageUp, 1), pt(0, 9));
        assert_eq!(moved(&d, r, BoxMove::PageDown, 1), pt(4, 9));
        assert_eq!(moved(&d, r, BoxMove::DocStart, 1), pt(0, 9));
        assert_eq!(moved(&d, r, BoxMove::DocEnd, 1), pt(4, 9));
        // 先頭の行で上・最後の行で下は動かない (文書の端へ寄せない)
        assert_eq!(moved(&d, BoxRect::at(pt(0, 9)), BoxMove::Up, 1), pt(0, 9));
        assert_eq!(moved(&d, BoxRect::at(pt(4, 9)), BoxMove::Down, 1), pt(4, 9));
    }

    // spec: 03 §4.8 (BOX-2)
    #[test]
    fn home_は字下げの終わり_end_は最も長い行の幅() {
        let d = doc("    abc\nabcdefgh");
        assert_eq!(moved(&d, BoxRect::at(pt(0, 6)), BoxMove::Home, 1), pt(0, 4));
        assert_eq!(moved(&d, BoxRect::at(pt(0, 0)), BoxMove::Home, 1), pt(0, 4));
        assert_eq!(moved(&d, rect((1, 0), (0, 6)), BoxMove::End, 1), pt(0, 8));
    }

    /// 長大行 (閾値 1,000 バイト) の文書。
    fn long_doc(text: &str) -> Document {
        let mut format = TextFormat::new_document();
        format.long_line_threshold = 1_000;
        Document::from_text(eve_text::Text::from(text), format)
    }

    // spec: 03 §4.8 (BOX-8, BOX-9), 03 §14.4
    #[test]
    fn 長大行の左右の移動で列の索引を作るのは列の近くまで() {
        let line = "aあ\t".repeat(40_000);
        let d = long_doc(&format!("{line}\nshort\n{line}"));
        assert_eq!(d.format.long_lines, 2);
        let built = |l| crate::layout::long_built_to(&d, l).unwrap_or(0);
        // 矩形は 0〜2 行目 (2 つの長大行と短い行)。列 40 から右へ 10 回・左へ 3 回
        let mut r = rect((2, 40), (0, 40));
        r.head = moved(&d, r, BoxMove::Right, 10);
        r.head = moved(&d, r, BoxMove::Left, 3);
        r.head = moved(&d, r, BoxMove::Right, 1);
        let (ranges, _) = box_ranges(&d, r).expect("上限以下");
        assert_eq!(ranges.len(), 2, "短い行は左の列に届かない");
        // 索引は今の列の近く (記録点の間隔とまとめて読む ASCII の分) までしか作らない。行の全体 (200,000 バイト) は読まない
        for l in [0, 2] {
            assert!(built(l) < 20_000, "{l} 行目の索引: {} バイト", built(l));
        }
        // 仮想の列へ伸ばす上限の判定も、行の全体の幅を求めない
        let short = long_doc(&format!("abc\n{line}"));
        let mut r = rect((1, 0), (0, 3));
        r.head = moved(&short, r, BoxMove::Right, 5);
        assert_eq!(r.head, pt(0, 8));
        assert!(crate::layout::long_built_to(&short, 1).unwrap_or(0) < 20_000);
        // `select.extend_end` だけは行末まで作る (03 §4.8 の BOX-9)
        let end = moved(&short, r, BoxMove::End, 1);
        assert_eq!(end.col, 40_000 * 4);
        assert_eq!(
            crate::layout::long_built_to(&short, 1),
            Some(line.len()),
            "行末まで"
        );
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(128))]

        // spec: 03 §4.8 (BOX-2)
        #[test]
        fn 右は最も長い行の幅_1_列で止まり_行の中の左右は往復する(
            pieces in proptest::collection::vec(
                proptest::collection::vec(0..PIECES.len(), 0..10),
                1..5,
            ),
            top in 0usize..5,
            col in 0usize..40,
            n in 1usize..60,
        ) {
            let lines: Vec<String> = pieces
                .iter()
                .map(|l| l.iter().map(|&i| PIECES[i]).collect())
                .collect();
            let d = doc(&lines.join("\n"));
            let last = lines.len() - 1;
            let r = rect((last, 0), (top.min(last), col));
            // 上限は矩形の行 (head の行から最後の行まで) の最も長い行の幅 + 1
            let cap = (r.head.line..=last).map(|l| ref_width(&lines[l])).max().unwrap_or(0) + 1;
            // 1 回ずつでも `count` でも、上限 (すでに右ならその列) で止まる
            let mut one = r;
            for _ in 0..n {
                one.head = moved(&d, one, BoxMove::Right, 1);
            }
            let many = moved(&d, r, BoxMove::Right, n);
            proptest::prop_assert_eq!(one.head, many);
            proptest::prop_assert!(many.col <= cap.max(col), "{:?} {} {}", lines, many.col, cap);
            if col < cap {
                proptest::prop_assert_eq!(many.col, ref_right(&lines[r.head.line], col, n, cap));
            }
            // 行の中の書記素の境界の列から右へ 1 つ・左へ 1 つで元に戻る
            let line = &lines[r.head.line];
            for (_, _, c, _) in ref_units(line) {
                let at = BoxRect { head: pt(r.head.line, c), ..r };
                let right = BoxRect { head: moved(&d, at, BoxMove::Right, 1), ..r };
                proptest::prop_assert_eq!(moved(&d, right, BoxMove::Left, 1), at.head);
            }
        }
    }

    /// 参照モデルの行の幅。
    fn ref_width(line: &str) -> usize {
        ref_units(line).last().map_or(0, |u| u.2 + u.3)
    }

    /// 参照モデルの `select.extend_right` を `n` 回: 行の中は次の書記素の境界の列 (書記素の途中なら後ろ)、行の幅から右は
    /// 1 列ずつで、`cap` で止まる。
    fn ref_right(line: &str, mut col: usize, n: usize, cap: usize) -> usize {
        let us = ref_units(line);
        for _ in 0..n {
            let next = us
                .iter()
                .find(|u| u.2 + u.3 > col)
                .map_or(col + 1, |u| u.2 + u.3);
            if next > cap {
                break;
            }
            col = next;
        }
        col
    }

    /// 参照の書記素 (タブは次の 4 の倍数の列まで、全角は 2、結合文字の付いた `e` は 1、制御文字は `^A` の 2、不正バイトの
    /// エスケープ文字は `<0x80>` の 6)。
    const PIECES: [&str; 8] = ["a", "b", " ", "\t", "あ", "e\u{301}", "\u{1}", "\u{10FF80}"];

    /// 参照モデル: 行の書記素の (始まりのバイト, 終わりのバイト, 始まりの列, 幅)。
    fn ref_units(line: &str) -> Vec<(usize, usize, usize, usize)> {
        let mut out = Vec::new();
        let (mut byte, mut col) = (0, 0);
        for (_, g) in eve_text::grapheme_indices(line) {
            let w = match g {
                "\t" => 4 - col % 4,
                "あ" | "\u{1}" => 2,
                "\u{10FF80}" => 6,
                _ => 1,
            };
            out.push((byte, byte + g.len(), col, w));
            byte += g.len();
            col += w;
        }
        out
    }

    /// 参照モデルの矩形の Range (行頭からのバイトの (anchor, head))。`None` は Range の無い行。
    fn ref_range(
        line: &str,
        (l, r): (usize, usize),
        forward: bool,
        head_line: bool,
    ) -> Option<(usize, usize)> {
        let us = ref_units(line);
        let width = us.last().map_or(0, |u| u.2 + u.3);
        if width < l {
            return head_line.then_some((line.len(), line.len()));
        }
        let floor = us
            .iter()
            .find(|u| u.2 + u.3 > l)
            .map_or(line.len(), |u| u.0);
        if l == r {
            return Some((floor, floor));
        }
        // [l, r) に掛かる書記素 (始まりの列 < r かつ 終わりの列 > l)
        let hit: Vec<_> = us.iter().filter(|u| u.2 < r && u.2 + u.3 > l).collect();
        let (s, e) = match (hit.first(), hit.last()) {
            (Some(f), Some(t)) => (f.0, t.1),
            _ => (floor, floor),
        };
        Some(if forward { (s, e) } else { (e, s) })
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(256))]

        // spec: 03 §4.8 (BOX-3, BOX-5), 03 §15.9.1 (proptest の (b) / (c))
        #[test]
        fn 矩形の各_range_は列の範囲に掛かる書記素だけを含み_正規化しても変わらない(
            pieces in proptest::collection::vec(
                proptest::collection::vec(0..PIECES.len(), 0..12),
                1..6,
            ),
            crlf in proptest::bool::ANY,
            a in (0usize..8, 0usize..30),
            h in (0usize..8, 0usize..30),
        ) {
            let lines: Vec<String> = pieces
                .iter()
                .map(|l| l.iter().map(|&i| PIECES[i]).collect())
                .collect();
            let eol = if crlf { "\r\n" } else { "\n" };
            let d = doc(&lines.join(eol));
            let last = lines.len() - 1;
            let rect = rect((a.0.min(last), a.1), (h.0.min(last), h.1));
            let (ranges, primary) = box_ranges(&d, rect).expect("上限以下");
            let forward = rect.head.col >= rect.anchor.col;
            let mut want = Vec::new();
            let mut want_primary = None;
            let mut ls = 0;
            for (i, line) in lines.iter().enumerate() {
                let in_rect = (rect.top()..=rect.bottom()).contains(&i);
                let range = in_rect
                    .then(|| {
                        ref_range(line, (rect.left(), rect.right()), forward, i == rect.head.line)
                    })
                    .flatten();
                if let Some((x, y)) = range {
                    if i == rect.head.line {
                        want_primary = Some(want.len());
                    }
                    want.push((ls + x, ls + y));
                }
                ls += line.len() + eol.len();
            }
            let got: Vec<(usize, usize)> = ranges.iter().map(|r| (r.anchor.0, r.head.0)).collect();
            proptest::prop_assert_eq!(&got, &want, "{:?} {:?}", lines, rect);
            proptest::prop_assert_eq!(Some(primary), want_primary);
            // INV-S1〜INV-S3: 正規化しても同じ (書記素の境界・文書の順・重ならない)
            let sel = Selection::new(ranges, primary).expect("上限以下");
            let normalized = sel.clone().normalized(d.text());
            proptest::prop_assert_eq!(normalized, sel);
        }
    }
}
