//! 行単位の編集 (docs/03 §15.3・§15.4、FR-207 / FR-208): 選択範囲のインデント / アンインデント、行の複製・削除・上下移動。
//!
//! どれも「対象の行」(選択が触れる行) に対する編集で、1 つの Transaction (Undo 1 回) で適用する。対象の行は、各 Range が
//! 触れる行の和集合を、連続する行のまとまり (ブロック) にしたもの。選択が行頭 (列 0) で終わり、その行より前から始まっているときは、
//! 最後の行は含めない (選んでいない行を巻き込まない)。

use eve_text::{ByteOff, ByteRange, ChangeSet, Edit, LineIdx, Pos, Range, Selection, TextRead};
use eve_view::{CommandId, DocumentId, ViewId};

use crate::document::{Document, Origin, Transaction};

use super::command::{
    CommandCtx, CommandKind, CommandRegistry, CommandResult, CommandSpec, Permission, RejectReason,
};

fn lb(t: &impl TextRead, l: usize) -> usize {
    t.line_to_byte(LineIdx(l)).0
}

fn le(t: &impl TextRead, l: usize) -> usize {
    t.line_end_byte(LineIdx(l)).0
}

fn bl(t: &impl TextRead, b: ByteOff) -> usize {
    t.byte_to_line(b).0
}

/// 連続する行のまとまり `[first, last]` (両端を含む)。
type Block = (usize, usize);

/// 操作の対象 (Window の View とその文書)。
fn active(ctx: &CommandCtx<'_>) -> Option<(ViewId, DocumentId)> {
    let w = ctx.state.first_window()?;
    let v = ctx.state.window_view(w)?;
    Some((v, ctx.state.view(v)?.doc))
}

/// 行 `l` の先頭と、改行を含む末尾 (次の行の先頭。最後の行は文書の末尾)。
fn line_span(doc: &Document, l: usize) -> (usize, usize) {
    let t = doc.text();
    let start = lb(t, l);
    let end = if l + 1 < t.len_lines() {
        lb(t, l + 1)
    } else {
        t.len_bytes()
    };
    (start, end)
}

/// 対象の行のブロック。
pub(super) fn target_blocks(doc: &Document, sel: &Selection) -> Vec<Block> {
    let t = doc.text();
    let mut lines: Vec<Block> = sel
        .iter()
        .map(|r| {
            let first = bl(t, r.start());
            let mut last = bl(t, r.end());
            // 行頭で終わる選択は、その行を含めない
            if !r.is_empty() && last > first && ByteOff(lb(t, last)) == r.end() {
                last -= 1;
            }
            (first, last)
        })
        .collect();
    lines.sort_unstable();
    let mut out: Vec<Block> = Vec::new();
    for (a, b) in lines {
        match out.last_mut() {
            Some(last) if a <= last.1 + 1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// 編集を 1 つの Transaction として適用する。`after` は適用後の Selection を (`ChangeSet` を見て) 作る。
fn commit(
    ctx: &mut CommandCtx<'_>,
    name: &str,
    edits: Vec<Edit>,
    after: impl FnOnce(&Document, &ChangeSet) -> Vec<Range>,
) -> CommandResult {
    let Some((view_id, doc_id)) = active(ctx) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    if edits.is_empty() {
        return CommandResult::Done;
    }
    let (Some(doc), Some(view)) = (ctx.state.document(doc_id), ctx.state.view(view_id)) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let before = view.selection.clone();
    let Ok(cs) = ChangeSet::from_sorted_edits(&doc.snapshot(), edits) else {
        return CommandResult::Rejected(RejectReason::InvalidArgs);
    };
    let ranges = after(doc, &cs);
    let Some(after_sel) = Selection::new(ranges, before.primary_index()) else {
        return CommandResult::Rejected(RejectReason::InvalidArgs);
    };
    let tx = Transaction::new(cs, Origin::Command(CommandId::new(name)), ctx.state.now())
        .with_view(view_id)
        .with_selections(Some(before), Some(after_sel));
    match ctx.state.apply_transaction(doc_id, tx) {
        Ok(_) => CommandResult::Done,
        Err(_) => CommandResult::Rejected(RejectReason::ReadOnly),
    }
}

/// 元の Range を、向きを保って写す: 始まりは前寄り、終わりは後ろ寄りに (行全体を選んでいれば行全体のまま)。
fn map_range(r: &Range, cs: &ChangeSet) -> Range {
    use eve_text::Bias;
    let (start, end) = (
        cs.map_pos(r.start(), Bias::Before),
        cs.map_pos(r.end(), Bias::After),
    );
    if r.anchor <= r.head {
        Range::new(start, end)
    } else {
        Range::new(end, start)
    }
}

// ---------- インデント / アンインデント ----------

/// 選択が触れる各行にインデントを足す (空行は飛ばす)。選択が無ければ `false` (呼び出し側が 1 カーソルの経路に進む)。
pub(super) fn indent_lines(ctx: &mut CommandCtx<'_>, unit: &str) -> CommandResult {
    let Some((view_id, doc_id)) = active(ctx) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let (Some(doc), Some(view)) = (ctx.state.document(doc_id), ctx.state.view(view_id)) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let sel = view.selection.clone();
    let t = doc.text();
    let mut edits = Vec::new();
    for (a, b) in target_blocks(doc, &sel) {
        for l in a..=b {
            let (ls, le) = (lb(t, l), le(t, l));
            if ls < le {
                edits.push(Edit {
                    range: ByteRange::new(ByteOff(ls), ByteOff(ls)),
                    insert: unit.into(),
                });
            }
        }
    }
    commit(ctx, "edit.indent", edits, move |_, cs| {
        sel.iter().map(|r| map_range(r, cs)).collect()
    })
}

/// 選択が触れる各行から、`width` 以内に収まる行頭空白を取り除く (03 §15.3 の規則)。
pub(super) fn outdent_lines(
    ctx: &mut CommandCtx<'_>,
    leading_removal: impl Fn(&Document, usize, usize) -> usize,
) -> CommandResult {
    let Some((view_id, doc_id)) = active(ctx) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let (Some(doc), Some(view)) = (ctx.state.document(doc_id), ctx.state.view(view_id)) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let sel = view.selection.clone();
    let t = doc.text();
    let mut edits = Vec::new();
    for (a, b) in target_blocks(doc, &sel) {
        for l in a..=b {
            let (ls, le) = (lb(t, l), le(t, l));
            let n = leading_removal(doc, ls, le);
            if n > 0 {
                edits.push(Edit {
                    range: ByteRange::new(ByteOff(ls), ByteOff(ls + n)),
                    insert: "".into(),
                });
            }
        }
    }
    commit(ctx, "edit.outdent", edits, move |_, cs| {
        sel.iter().map(|r| map_range(r, cs)).collect()
    })
}

// ---------- 行の複製 ----------

fn duplicate_line(ctx: &mut CommandCtx<'_>) -> CommandResult {
    let Some((view_id, doc_id)) = active(ctx) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let (Some(doc), Some(view)) = (ctx.state.document(doc_id), ctx.state.view(view_id)) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let sel = view.selection.clone();
    let eol = doc.format.default_eol.as_str().to_owned();
    let t = doc.text();
    let blocks = target_blocks(doc, &sel);
    // ブロックごとの (元の先頭, 複製の先頭)
    let mut shifts: Vec<(Block, usize, usize)> = Vec::new();
    let mut edits = Vec::new();
    for &(a, b) in &blocks {
        let (start, _) = line_span(doc, a);
        let (_, end) = line_span(doc, b);
        let block = t.slice_to_string(ByteRange::new(ByteOff(start), ByteOff(end)));
        // 最後の行に改行が無ければ、複製の前に改行を足す
        let ends_with_eol = le(t, b) < end;
        let (ins, copy_start) = if ends_with_eol {
            (block, end)
        } else {
            (format!("{eol}{block}"), end + eol.len())
        };
        shifts.push(((a, b), start, copy_start));
        edits.push(Edit {
            range: ByteRange::new(ByteOff(end), ByteOff(end)),
            insert: ins.as_str().into(),
        });
    }
    // 複製に移る: 元の位置を、ブロック内の相対位置のまま複製の中へ。他のブロックの複製の分だけ後ろにずれる
    let starts: Vec<(usize, usize, usize)> =
        shifts.iter().map(|&((a, _), s, c)| (a, s, c)).collect();
    let inserted: Vec<usize> = edits.iter().map(|e| e.insert.len()).collect();
    let after_sel = sel.clone();
    let line_of = {
        let t = doc.text();
        sel.iter()
            .map(|r| (bl(t, r.anchor), bl(t, r.head)))
            .collect::<Vec<_>>()
    };
    let blocks2 = blocks.clone();
    commit(ctx, "edit.duplicate_line", edits, move |_, _| {
        let pos = |p: Pos, line: usize| -> Pos {
            // p が属するブロックの番号
            let i = blocks2
                .iter()
                .position(|&(a, b)| line >= a && line <= b)
                .unwrap_or(0);
            let (_, s, c) = starts[i];
            let earlier: usize = inserted[..i].iter().sum();
            ByteOff(p.0 - s + c + earlier)
        };
        after_sel
            .iter()
            .zip(&line_of)
            .map(|(r, &(la, lh))| Range::new(pos(r.anchor, la), pos(r.head, lh)))
            .collect()
    })
}

// ---------- 行の削除 ----------

fn delete_line(ctx: &mut CommandCtx<'_>) -> CommandResult {
    let Some((view_id, doc_id)) = active(ctx) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let (Some(doc), Some(view)) = (ctx.state.document(doc_id), ctx.state.view(view_id)) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let sel = view.selection.clone();
    let t = doc.text();
    let total = t.len_lines();
    let mut edits: Vec<Edit> = Vec::new();
    let mut cursors: Vec<usize> = Vec::new(); // 削除後に カーソルを置く元の座標
    for (a, b) in target_blocks(doc, &sel) {
        let (mut start, end) = (line_span(doc, a).0, line_span(doc, b).1);
        // 最後の行が改行で終わっていなければ、手前の改行も取り除く (空の行を残さない)
        if b + 1 == total && le(t, b) == end && a > 0 {
            start = le(t, a - 1);
        }
        cursors.push(start);
        edits.push(Edit {
            range: ByteRange::new(ByteOff(start), ByteOff(end)),
            insert: "".into(),
        });
    }
    commit(ctx, "edit.delete_line", edits, move |_, cs| {
        cursors
            .iter()
            .map(|&p| Range::point(cs.map_pos(ByteOff(p), eve_text::Bias::After)))
            .collect()
    })
}

// ---------- 行の上下移動 ----------

fn move_lines(ctx: &mut CommandCtx<'_>, up: bool) -> CommandResult {
    let Some((view_id, doc_id)) = active(ctx) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let (Some(doc), Some(view)) = (ctx.state.document(doc_id), ctx.state.view(view_id)) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let sel = view.selection.clone();
    let t = doc.text();
    let total = t.len_lines();
    let text = |s: usize, e: usize| t.slice_to_string(ByteRange::new(ByteOff(s), ByteOff(e)));
    let mut edits: Vec<Edit> = Vec::new();
    // 動かすブロックと、位置のずれ (符号つき)
    let mut moved: Vec<(Block, i64)> = Vec::new();
    for (a, b) in target_blocks(doc, &sel) {
        if up {
            if a == 0 {
                continue;
            }
            // [前の行 + 改行][ブロック 本体][ブロックの改行] → [ブロック 本体][前の行の改行][前の行 本体][ブロックの改行]
            let (ps, pe) = (lb(t, a - 1), le(t, a - 1));
            let (bs, be) = (line_span(doc, a).0, le(t, b));
            let end = line_span(doc, b).1;
            let prev_eol = text(pe, bs);
            let block = text(bs, be);
            let block_eol = text(be, end);
            let prev = text(ps, pe);
            let new = format!("{block}{prev_eol}{prev}{block_eol}");
            edits.push(Edit {
                range: ByteRange::new(ByteOff(ps), ByteOff(end)),
                insert: new.as_str().into(),
            });
            moved.push(((a, b), -((bs - ps) as i64)));
        } else {
            if b + 1 >= total {
                continue;
            }
            let (bs, be) = (line_span(doc, a).0, le(t, b));
            let (ns, ne) = (lb(t, b + 1), le(t, b + 1));
            let end = line_span(doc, b + 1).1;
            let block = text(bs, be);
            let block_eol = text(be, ns);
            let next = text(ns, ne);
            let next_eol = text(ne, end);
            let new = format!("{next}{block_eol}{block}{next_eol}");
            edits.push(Edit {
                range: ByteRange::new(ByteOff(bs), ByteOff(end)),
                insert: new.as_str().into(),
            });
            moved.push(((a, b), (ns - bs) as i64));
        }
    }
    let line_of: Vec<(usize, usize)> = sel
        .iter()
        .map(|r| (bl(t, r.anchor), bl(t, r.head)))
        .collect();
    commit(ctx, "edit.move_line", edits, move |_, _| {
        let shift = |p: Pos, line: usize| -> Pos {
            let d = moved
                .iter()
                .find(|((a, b), _)| line >= *a && line <= *b)
                .map_or(0, |(_, d)| *d);
            ByteOff((p.0 as i64 + d) as usize)
        };
        sel.iter()
            .zip(&line_of)
            .map(|(r, &(la, lh))| Range::new(shift(r.anchor, la), shift(r.head, lh)))
            .collect()
    })
}

fn move_line_up(ctx: &mut CommandCtx<'_>) -> CommandResult {
    move_lines(ctx, true)
}

fn move_line_down(ctx: &mut CommandCtx<'_>) -> CommandResult {
    move_lines(ctx, false)
}

/// v0.2 の行操作のコマンド (07 §11.6 の「編集」)。
pub(super) fn register(r: &mut CommandRegistry) {
    let handlers: [(&str, crate::Handler); 4] = [
        ("edit.duplicate_line", duplicate_line),
        ("edit.delete_line", delete_line),
        ("edit.move_line_up", move_line_up),
        ("edit.move_line_down", move_line_down),
    ];
    for (id, h) in handlers {
        r.register(
            CommandSpec::new(
                id,
                CommandKind::Edit,
                Permission::Propose,
                "!doc.read_only",
                h,
            )
            .expect("カタログの when 句は構文が正しい"),
        );
    }
}
