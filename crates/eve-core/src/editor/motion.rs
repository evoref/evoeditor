//! 移動・選択コマンド (docs/03 §5.1〜§5.6、docs/07 §11.6 の「移動・選択」)。文書は変えない (`Motion`)。
//!
//! 移動の起点はいつも `head`。`select.extend_*` は移動先を `with_head` で head にし (anchor は動かさない)、
//! `cursor.*` は移動先へ潰す。複数 Range には各 Range 独立に適用し、`normalized()` で併合する。

use eve_text::{
    ByteOff, ByteRange, LineIdx, Range, Selection, TextRead, WidthConfig, display_col_to_byte,
    next_grapheme_boundary, next_word_stop, prev_grapheme_boundary, prev_word_stop,
    str_display_width,
};
use eve_view::{DocumentId, ViewId};

use crate::document::Document;

use super::command::{
    ArgValue, CommandCtx, CommandKind, CommandRegistry, CommandResult, CommandSpec, Permission,
    RejectReason,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    Home,
    End,
    DocStart,
    DocEnd,
    PageUp,
    PageDown,
}

impl Motion {
    /// 目標列 (`goal_dcol`) を維持する移動 (垂直移動とページ移動)。
    fn keeps_goal(self) -> bool {
        matches!(
            self,
            Motion::Up | Motion::Down | Motion::PageUp | Motion::PageDown
        )
    }
}

fn active(ctx: &CommandCtx<'_>) -> Option<(ViewId, DocumentId)> {
    let w = ctx.state.first_window()?;
    let v = ctx.state.window_view(w)?;
    Some((v, ctx.state.view(v)?.doc))
}

/// (行, 行頭, 行末)。
fn line_of(doc: &Document, p: ByteOff) -> (usize, usize, usize) {
    let t = doc.text();
    let l = t.byte_to_line(p);
    (l.0, t.line_to_byte(l).0, t.line_end_byte(l).0)
}

fn display_col(doc: &Document, ls: usize, p: usize) -> u32 {
    let s = doc
        .text()
        .slice_to_string(ByteRange::new(ByteOff(ls), ByteOff(p)));
    u32::try_from(str_display_width(&s, &WidthConfig::default())).unwrap_or(u32::MAX)
}

/// 行 `line` の表示列 `dcol` の位置 (セルの途中は前に寄せる)。
fn pos_at_dcol(doc: &Document, line: usize, dcol: u32) -> ByteOff {
    let t = doc.text();
    let l = LineIdx(line);
    let (ls, le) = (t.line_to_byte(l).0, t.line_end_byte(l).0);
    let s = t.slice_to_string(ByteRange::new(ByteOff(ls), ByteOff(le)));
    ByteOff(ls + display_col_to_byte(&s, dcol as usize, &WidthConfig::default()))
}

/// 行頭空白の直後 (インデント終端)。
fn indent_end(doc: &Document, ls: usize, le: usize) -> usize {
    let s = doc
        .text()
        .slice_to_string(ByteRange::new(ByteOff(ls), ByteOff(le)));
    ls + s.bytes().take_while(|b| matches!(b, b' ' | b'\t')).count()
}

/// 1 回の移動。`goal` は垂直移動の目標列 (未設定なら現在の表示列で初期化する)。
fn step(doc: &Document, goal: &mut Option<u32>, p: ByteOff, m: Motion, rows: usize) -> ByteOff {
    let t = doc.text();
    let last_line = t.len_lines().saturating_sub(1);
    match m {
        Motion::Left => prev_grapheme_boundary(t, p),
        Motion::Right => next_grapheme_boundary(t, p),
        Motion::WordLeft => prev_word_stop(t, p),
        Motion::WordRight => next_word_stop(t, p),
        Motion::Up | Motion::Down | Motion::PageUp | Motion::PageDown => {
            let (line, ls, _) = line_of(doc, p);
            let delta = match m {
                Motion::Up | Motion::Down => 1,
                _ => rows.saturating_sub(1).max(1),
            };
            let going_up = matches!(m, Motion::Up | Motion::PageUp);
            // 先頭行で上 → 文書先頭、最終行で下 → 文書末尾 (CUA 慣習)
            if going_up && line == 0 {
                return ByteOff(0);
            }
            if !going_up && line == last_line {
                return ByteOff(t.len_bytes());
            }
            let target = if going_up {
                line.saturating_sub(delta)
            } else {
                (line + delta).min(last_line)
            };
            let g = *goal.get_or_insert_with(|| display_col(doc, ls, p.0));
            pos_at_dcol(doc, target, g)
        }
        Motion::Home => {
            let (_, ls, le) = line_of(doc, p);
            let ie = indent_end(doc, ls, le);
            // インデント終端以外 → インデント終端、インデント終端 → 行頭 (インデントが無ければ常に行頭)
            ByteOff(if p.0 != ie && ie != ls { ie } else { ls })
        }
        Motion::End => ByteOff(line_of(doc, p).2),
        Motion::DocStart => ByteOff(0),
        Motion::DocEnd => ByteOff(t.len_bytes()),
    }
}

/// 移動を各 Range に適用する。垂直・ページ移動は目標列を維持し、移動の後にカーソルが見えるようスクロールを調整する。
fn run(ctx: &mut CommandCtx<'_>, m: Motion, extend: bool) -> CommandResult {
    let Some((view_id, doc_id)) = active(ctx) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let count = match ctx.args.get("count") {
        None => 1,
        Some(ArgValue::Int(n)) if *n >= 1 => *n as usize,
        Some(_) => return CommandResult::Rejected(RejectReason::InvalidArgs),
    };
    let rows = ctx.state.viewport().1 as usize;
    let (Some(doc), Some(view)) = (ctx.state.document(doc_id), ctx.state.view(view_id)) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let mut goal = if m.keeps_goal() { view.goal_dcol } else { None };
    let mut out = Vec::with_capacity(view.selection.len());
    for r in view.selection.iter() {
        let mut head = r.head;
        let mut collapsed_first = false;
        // 選択がある状態の水平移動 (Shift なし) は start / end へ潰して移動しない (CUA 慣習、03 §5.1)
        if !extend && !r.is_empty() && matches!(m, Motion::Left | Motion::Right) {
            head = if m == Motion::Left {
                r.start()
            } else {
                r.end()
            };
            collapsed_first = true;
        }
        for _ in 0..(if collapsed_first { count - 1 } else { count }) {
            head = step(doc, &mut goal, head, m, rows);
        }
        out.push(if extend {
            r.with_head(head)
        } else {
            Range::point(head)
        });
    }
    let primary = view.selection.primary_index();
    let Some(sel) = Selection::new(out, primary) else {
        return CommandResult::Rejected(RejectReason::InvalidArgs);
    };
    // ページ移動はスクロール位置も同じ行数だけ動かし、カーソルの画面上の相対位置を保つ
    let page_scroll = match m {
        Motion::PageUp => Some(-(rows.saturating_sub(1).max(1) as i64) * count as i64),
        Motion::PageDown => Some((rows.saturating_sub(1).max(1) as i64) * count as i64),
        _ => None,
    };
    let keep = m.keeps_goal().then_some(goal).flatten();
    ctx.state.set_selection_with_goal(view_id, sel, keep);
    if let Some(d) = page_scroll {
        ctx.state.scroll_lines(view_id, d);
    }
    ctx.state.ensure_cursor_visible(view_id);
    CommandResult::Done
}

macro_rules! motion_fns {
    ($($name:ident: $m:expr, $extend:expr;)*) => {
        $(fn $name(ctx: &mut CommandCtx<'_>) -> CommandResult { run(ctx, $m, $extend) })*
    };
}

motion_fns! {
    move_left: Motion::Left, false;
    move_right: Motion::Right, false;
    move_up: Motion::Up, false;
    move_down: Motion::Down, false;
    extend_left: Motion::Left, true;
    extend_right: Motion::Right, true;
    extend_up: Motion::Up, true;
    extend_down: Motion::Down, true;
    move_word_left: Motion::WordLeft, false;
    move_word_right: Motion::WordRight, false;
    extend_word_left: Motion::WordLeft, true;
    extend_word_right: Motion::WordRight, true;
    home: Motion::Home, false;
    end: Motion::End, false;
    extend_home: Motion::Home, true;
    extend_end: Motion::End, true;
    doc_start: Motion::DocStart, false;
    doc_end: Motion::DocEnd, false;
    extend_to_doc_start: Motion::DocStart, true;
    extend_to_doc_end: Motion::DocEnd, true;
    page_up: Motion::PageUp, false;
    page_down: Motion::PageDown, false;
    extend_page_up: Motion::PageUp, true;
    extend_page_down: Motion::PageDown, true;
}

/// `cursor.goto_line { line: int | "$", col?: int }`。行・桁は 1 始まり (桁は書記素数)。範囲外は丸める。
fn goto_line(ctx: &mut CommandCtx<'_>) -> CommandResult {
    let Some((view_id, doc_id)) = active(ctx) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let Some(doc) = ctx.state.document(doc_id) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let t = doc.text();
    let last = t.len_lines().saturating_sub(1);
    let line = match ctx.args.get("line") {
        Some(ArgValue::Int(n)) => (usize::try_from(*n).unwrap_or(1).max(1) - 1).min(last),
        Some(ArgValue::Str(s)) if s == "$" => last,
        None => 0,
        Some(_) => return CommandResult::Rejected(RejectReason::InvalidArgs),
    };
    let col = match ctx.args.get("col") {
        None => 1,
        Some(ArgValue::Int(n)) if *n >= 1 => *n as usize,
        Some(_) => return CommandResult::Rejected(RejectReason::InvalidArgs),
    };
    let (ls, le) = (
        t.line_to_byte(LineIdx(line)),
        t.line_end_byte(LineIdx(line)),
    );
    let mut p = ls;
    for _ in 1..col {
        let n = next_grapheme_boundary(t, p);
        if n > le || n == p {
            p = le;
            break;
        }
        p = n;
    }
    ctx.state
        .set_selection(view_id, Selection::point(p.min(le)));
    ctx.state.ensure_cursor_visible(view_id);
    CommandResult::Done
}

macro_rules! reg {
    ($r:expr, $id:expr, $h:expr) => {
        $r.register(
            CommandSpec::new($id, CommandKind::Motion, Permission::Read, "", $h)
                .expect("カタログの when 句は構文が正しい"),
        )
    };
}

/// v0.1 の移動・選択コマンド (07 §11.6。`ui.cancel` はオーバーレイと同時)。
pub(super) fn register(r: &mut CommandRegistry) {
    reg!(r, "cursor.move_left", move_left);
    reg!(r, "cursor.move_right", move_right);
    reg!(r, "cursor.move_up", move_up);
    reg!(r, "cursor.move_down", move_down);
    reg!(r, "select.extend_left", extend_left);
    reg!(r, "select.extend_right", extend_right);
    reg!(r, "select.extend_up", extend_up);
    reg!(r, "select.extend_down", extend_down);
    reg!(r, "cursor.move_word_left", move_word_left);
    reg!(r, "cursor.move_word_right", move_word_right);
    reg!(r, "select.extend_word_left", extend_word_left);
    reg!(r, "select.extend_word_right", extend_word_right);
    reg!(r, "cursor.home", home);
    reg!(r, "cursor.end", end);
    reg!(r, "select.extend_home", extend_home);
    reg!(r, "select.extend_end", extend_end);
    reg!(r, "cursor.doc_start", doc_start);
    reg!(r, "cursor.doc_end", doc_end);
    reg!(r, "select.extend_to_doc_start", extend_to_doc_start);
    reg!(r, "select.extend_to_doc_end", extend_to_doc_end);
    reg!(r, "cursor.page_up", page_up);
    reg!(r, "cursor.page_down", page_down);
    reg!(r, "select.extend_page_up", extend_page_up);
    reg!(r, "select.extend_page_down", extend_page_down);
    reg!(r, "cursor.goto_line", goto_line);
}
