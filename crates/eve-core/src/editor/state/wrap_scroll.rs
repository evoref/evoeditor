//! 折り返し ON のスクロール (docs/03 §14.3.1)。位置は (論理行, 行内の表示行) の組で表し、表示行の数は `wrap_line` から引く。
//!
//! 文書全体の折り返しは数えない: 動かす量は画面の大きさ以下で、カーソルが遠いときは間を数えずに直接置く。

use eve_text::{LineIdx, TextRead};
use eve_view::ViewId;

use super::EditorState;
use crate::document::Document;
use crate::editor::layout::{WrapSettings, line_rows, wrap_ctx};

/// (論理行, その行の中の表示行)。組の大小がそのまま文書内の前後。
type RowPos = (usize, usize);

/// 折り返し ON の View の (折り返しの設定, 折り返し幅)。
pub(super) type Geometry = (WrapSettings, usize);

fn rows_in(doc: &Document, line: usize, g: Geometry) -> usize {
    line_rows(doc, line, g.0, g.1).len()
}

/// `n` 表示行だけ後ろ (`up = false`) か前 (`up = true`) へ。文書の端で止まる。
fn walk(doc: &Document, g: Geometry, mut p: RowPos, n: usize, up: bool) -> RowPos {
    let last = doc.text().len_lines().saturating_sub(1);
    let mut count = rows_in(doc, p.0, g);
    for _ in 0..n {
        if up {
            if p.1 > 0 {
                p.1 -= 1;
            } else if p.0 > 0 {
                p.0 -= 1;
                count = rows_in(doc, p.0, g);
                p.1 = count - 1;
            } else {
                break;
            }
        } else if p.1 + 1 < count {
            p.1 += 1;
        } else if p.0 < last {
            p.0 += 1;
            count = rows_in(doc, p.0, g);
            p.1 = 0;
        } else {
            break;
        }
    }
    p
}

/// 画面の最下行に文書の最後の表示行が来る位置 (これより下へは動かさない)。
fn max_top(doc: &Document, g: Geometry, rows: usize) -> RowPos {
    let last = doc.text().len_lines().saturating_sub(1);
    let end = (last, rows_in(doc, last, g) - 1);
    walk(doc, g, end, rows.max(1) - 1, true)
}

/// `from` から `to` (`from` 以降) までの表示行の数。`cap` を超えたら打ち切って `cap + 1` を返す。
fn rows_between(doc: &Document, g: Geometry, from: RowPos, to: RowPos, cap: usize) -> usize {
    let mut d = 0;
    let mut first = from.1;
    for line in from.0..to.0 {
        d += rows_in(doc, line, g) - first;
        first = 0;
        if d > cap {
            return cap + 1;
        }
    }
    d + to.1 - first
}

impl EditorState {
    /// 折り返し ON の View の幾何。OFF なら `None`。
    pub(super) fn wrap_geometry(&self, view: ViewId) -> Option<Geometry> {
        wrap_ctx(self, view)
    }

    fn top_pos(&self, view: ViewId) -> Option<RowPos> {
        let v = self.views.get(&view)?;
        let d = self.documents.get(&v.doc)?;
        Some((
            d.text().byte_to_line(v.scroll.top).0,
            v.scroll.wrap_idx as usize,
        ))
    }

    /// 主カーソルの (論理行, 表示行)。
    fn cursor_pos(&self, view: ViewId, g: Geometry) -> Option<RowPos> {
        let v = self.views.get(&view)?;
        let d = self.documents.get(&v.doc)?;
        let head = v.selection.primary().head;
        let line = d.text().byte_to_line(head).0;
        let lr = line_rows(d, line, g.0, g.1);
        Some((line, lr.locate(head.0 - lr.ls, v.cursor_affinity).0))
    }

    fn place_top(&mut self, view: ViewId, pos: RowPos) {
        let Some(v) = self.views.get_mut(&view) else {
            return;
        };
        let Some(d) = self.documents.get(&v.doc) else {
            return;
        };
        let top = d.text().line_to_byte(LineIdx(pos.0));
        let widx = u32::try_from(pos.1).unwrap_or(u32::MAX);
        if v.scroll.top != top || v.scroll.wrap_idx != widx {
            v.scroll.top = top;
            v.scroll.wrap_idx = widx;
            self.needs_render = true;
        }
    }

    fn view_doc(&self, view: ViewId) -> Option<&Document> {
        self.documents.get(&self.views.get(&view)?.doc)
    }

    pub(super) fn wrapped_scroll_lines(&mut self, view: ViewId, g: Geometry, delta: i64) {
        let (Some(top), Some(doc)) = (self.top_pos(view), self.view_doc(view)) else {
            return;
        };
        let rows = self.viewport.1 as usize;
        let moved = walk(
            doc,
            g,
            top,
            usize::try_from(delta.unsigned_abs()).unwrap_or(usize::MAX),
            delta < 0,
        );
        let new = moved.min(max_top(doc, g, rows));
        self.place_top(view, new);
    }

    pub(super) fn wrapped_ensure_vertical(&mut self, view: ViewId, g: Geometry, margin: usize) {
        let rows = self.viewport.1 as usize;
        let (Some(top), Some(cur), Some(doc)) = (
            self.top_pos(view),
            self.cursor_pos(view, g),
            self.view_doc(view),
        ) else {
            return;
        };
        let below = rows.saturating_sub(1 + margin);
        let new = if cur < top {
            walk(doc, g, cur, margin, true)
        } else if cur.0 > top.0 + rows {
            // 1 画面より遠い: 間の折り返しを数えずに、カーソルが余白の内側に入る位置へ直接置く
            walk(doc, g, cur, below, true).min(max_top(doc, g, rows).max(top))
        } else {
            let d = rows_between(doc, g, top, cur, rows);
            if d < margin {
                walk(doc, g, cur, margin, true)
            } else if rows > 0 && d + margin >= rows {
                walk(doc, g, cur, below, true).min(max_top(doc, g, rows).max(top))
            } else {
                top
            }
        };
        self.place_top(view, new);
    }

    pub(super) fn wrapped_center_cursor(&mut self, view: ViewId, g: Geometry) {
        let rows = self.viewport.1 as usize;
        let (Some(cur), Some(doc)) = (self.cursor_pos(view, g), self.view_doc(view)) else {
            return;
        };
        let new = walk(doc, g, cur, rows / 2, true).min(max_top(doc, g, rows));
        self.place_top(view, new);
    }
}
