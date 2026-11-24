//! 行単位の編集 (docs/03 §15.3・§15.4、FR-207 / FR-208): 選択範囲のインデント / アンインデント、行の複製・削除・上下移動。
//!
//! どれも「対象の行」(選択が触れる行) に対する編集で、1 つの Transaction (Undo 1 回) で適用する。対象の行は、各 Range が
//! 触れる行の和集合を、連続する行のまとまり (ブロック) にしたもの。選択が行頭 (列 0) で終わり、その行より前から始まっているときは、
//! 最後の行は含めない (選んでいない行を巻き込まない)。各 Range は、始まりの行を含むブロックに付いていく (03 §4.5.2)。
//!
//! spec: 03 §4.5.2, 03 §15.3, 03 §15.4

use eve_text::{ByteOff, ByteRange, ChangeSet, Edit, LineIdx, Pos, Range, Selection, TextRead};
use eve_view::{CommandId, DocumentId, ViewId};

use crate::command::ids;
use crate::document::{Document, Origin, Transaction};

use crate::command::{
    CommandCtx, CommandKind, CommandRegistry, CommandResult, Permission, RejectReason,
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
pub(super) fn active(ctx: &CommandCtx<'_>) -> Option<(ViewId, DocumentId)> {
    let w = ctx.state.focused_window()?;
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
pub(crate) fn target_blocks(doc: &Document, sel: &Selection) -> Vec<Block> {
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

/// 各 Range の始まりの行を含むブロックの番号 (`blocks` は `target_blocks` の結果)。どの Range にも、始まりの行を含む
/// ブロックがある (ブロックは全 Range の触れる行から作る)。
fn block_of_ranges(t: &impl TextRead, sel: &Selection, blocks: &[Block]) -> Vec<usize> {
    sel.iter()
        .map(|r| {
            let line = bl(t, r.start());
            blocks
                .iter()
                .position(|&(a, b)| line >= a && line <= b)
                .unwrap_or(0)
        })
        .collect()
}

/// 編集を 1 つの Transaction として適用する。`after` は適用後の Selection を (`ChangeSet` を見て) 作る。
pub(super) fn commit(
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
    // 重なった・同じ位置になった Range を併合してから記録する (Undo / Redo で戻す選択も INV-S1〜S3 を満たす)
    let Some(after_sel) =
        Selection::new(ranges, before.primary_index()).map(|s| s.merged(cs.len_after()))
    else {
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

/// 各 Range の位置 (`head`) が行頭 (列 0) か。行の編集の前の本文で調べ、[`map_range`] に渡す。
pub(crate) fn heads_at_line_start(t: &impl TextRead, sel: &Selection) -> Vec<bool> {
    sel.iter()
        .map(|r| t.line_to_byte(t.byte_to_line(r.head)) == r.head)
        .collect()
}

/// 行の編集 (インデント / アンインデント・行コメントの切替) の後の Range (03 §15.4 の下の箇条、§15.6 の CMT-5 の細目 (b))。
/// 空でない Range は、向きを保って始まりを前寄り・終わりを後ろ寄りに写す (行全体を選んでいれば行全体のまま)。空のカーソルは
/// 自分の編集の後ろへ写す (`MapPolicy::Own`) が、行頭 (列 0) のものは列 0 に留める (足した字下げ・記号を選んだ選択に
/// 変えない)。`at_line_start` は編集の前の本文で調べた値 ([`heads_at_line_start`])。
pub(crate) fn map_range(r: &Range, at_line_start: bool, cs: &ChangeSet) -> Range {
    use eve_text::Bias;
    if r.is_empty() {
        let bias = if at_line_start {
            Bias::Before
        } else {
            Bias::After
        };
        return Range::point(cs.map_pos(r.head, bias));
    }
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
pub(crate) fn indent_lines(ctx: &mut CommandCtx<'_>, unit: &str) -> CommandResult {
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
    let at_line_start = heads_at_line_start(t, &sel);
    commit(ctx, ids::EDIT_INDENT, edits, move |_, cs| {
        sel.iter()
            .zip(&at_line_start)
            .map(|(r, &s)| map_range(r, s, cs))
            .collect()
    })
}

/// 選択が触れる各行から、`width` 以内に収まる行頭空白を取り除く (03 §15.3 の規則)。
pub(crate) fn outdent_lines(
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
    let at_line_start = heads_at_line_start(t, &sel);
    commit(ctx, ids::EDIT_OUTDENT, edits, move |_, cs| {
        sel.iter()
            .zip(&at_line_start)
            .map(|(r, &s)| map_range(r, s, cs))
            .collect()
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
    // 各 Range は、始まりの行を含むブロックの複製に移る (終わりが次の行の行頭でも同じブロック。move_lines と同じ)
    let block_of = block_of_ranges(t, &sel, &blocks);
    let after_sel = sel.clone();
    commit(ctx, ids::EDIT_DUPLICATE_LINE, edits, move |_, _| {
        let pos = |p: Pos, i: usize| -> Pos {
            let (_, s, c) = starts[i];
            let earlier: usize = inserted[..i].iter().sum();
            ByteOff(p.0 - s + c + earlier)
        };
        after_sel
            .iter()
            .zip(&block_of)
            .map(|(r, &i)| Range::new(pos(r.anchor, i), pos(r.head, i)))
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
    let mut cursors: Vec<usize> = Vec::new(); // 削除後に カーソルを置く元の座標 (ブロックごと)
    let blocks = target_blocks(doc, &sel);
    for &(a, b) in &blocks {
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
    // 各 Range は、始まりの行を含むブロックの削除の位置へ (Range の数と主の番号を保ち、同じ位置は併合で 1 つになる)
    let block_of = block_of_ranges(t, &sel, &blocks);
    commit(ctx, ids::EDIT_DELETE_LINE, edits, move |_, cs| {
        block_of
            .iter()
            .map(|&i| Range::point(cs.map_pos(ByteOff(cursors[i]), eve_text::Bias::After)))
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
    // 動かしたブロックと、ブロックの本文 (`[bs, be]`) の位置のずれ (符号つき)、ブロックの後の行頭の写し先
    // (選択が行頭で終わるとき、その位置もブロックについていく)
    let mut moved: Vec<Moved> = Vec::new();
    let blocks = target_blocks(doc, &sel);
    // どれか 1 つのブロックが文書の端で動けなければ、全体を何もしない (一部のブロックだけを動かして並びを崩さない。
    // 03 §4.5.2)
    let stuck = |&(a, b): &Block| if up { a == 0 } else { b + 1 >= total };
    if blocks.iter().any(stuck) {
        return CommandResult::Done;
    }
    for (a, b) in blocks {
        if up {
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
            moved.push(Moved {
                block: (a, b),
                be,
                back: bs - ps,
                fwd: 0,
                after: (end, ps + (be - bs) + (bs - pe)),
            });
        } else {
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
            // ブロックの本文は、隣の行の本文とブロックの改行の後に来る
            moved.push(Moved {
                block: (a, b),
                be,
                back: 0,
                fwd: ne - be,
                after: (ns, end),
            });
        }
    }
    // 各 Range は、始まりの行を含むブロックについていく (終わりが次の行の行頭でも同じブロック)
    let start_line: Vec<usize> = sel.iter().map(|r| bl(t, r.start())).collect();
    commit(ctx, "edit.move_line", edits, move |_, _| {
        sel.iter()
            .zip(&start_line)
            .map(|(r, &line)| {
                let Some(m) = moved
                    .iter()
                    .find(|m| line >= m.block.0 && line <= m.block.1)
                else {
                    return *r;
                };
                let shift = |p: Pos| -> Pos {
                    if p.0 <= m.be {
                        ByteOff((p.0 + m.fwd).saturating_sub(m.back))
                    } else if p.0 == m.after.0 {
                        ByteOff(m.after.1)
                    } else {
                        p
                    }
                };
                Range::new(shift(r.anchor), shift(r.head))
            })
            .collect()
    })
}

/// 上下移動で動かしたブロック (`move_lines`)。
struct Moved {
    block: Block,
    /// ブロックの最後の行の本文の末尾 (改行の前。旧座標)。
    be: usize,
    /// ブロックの本文の位置のずれ (前へ動かす量と後へ動かす量。どちらかは 0。符号付きの変換をしない)。
    back: usize,
    fwd: usize,
    /// ブロックの後の行頭 (旧座標) と、その写し先 (移した後のブロックの本文の末尾と、それに続く改行の後)。
    after: (usize, usize),
}

fn move_line_up(ctx: &mut CommandCtx<'_>) -> CommandResult {
    move_lines(ctx, true)
}

fn move_line_down(ctx: &mut CommandCtx<'_>) -> CommandResult {
    move_lines(ctx, false)
}

/// v0.2 の行操作のコマンド (07 §11.6 の「編集」)。
pub(crate) fn register(r: &mut CommandRegistry) {
    let handlers: [(&str, crate::Handler); 4] = [
        (ids::EDIT_DUPLICATE_LINE, duplicate_line),
        (ids::EDIT_DELETE_LINE, delete_line),
        (ids::EDIT_MOVE_LINE_UP, move_line_up),
        (ids::EDIT_MOVE_LINE_DOWN, move_line_down),
    ];
    for (id, h) in handlers {
        r.add(
            id,
            CommandKind::Edit,
            Permission::Propose,
            "!doc.read_only",
            h,
        );
    }
}
