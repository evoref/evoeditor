//! 編集コマンド (docs/03 §5.3 / §15.3、docs/07 §11.6 の「編集」)。
//!
//! どのコマンドも Selection の各 Range に同じ規則を適用して 1 つの `ChangeSet` に合成し、1 つの Transaction として
//! 適用する (03 §4.5)。変更が無いと分かれば Transaction を作らない (Revision も履歴も変わらない)。

use eve_text::{
    Bias, ByteOff, ByteRange, ChangeSet, Edit, Range, Selection, TextRead, next_grapheme_boundary,
    next_word_stop, prev_grapheme_boundary, prev_word_stop,
};
use eve_text::{WidthConfig, str_display_width};
use eve_view::{DocumentId, ViewId};

use crate::document::{Document, Origin, Transaction};

use super::command::{
    CommandCtx, CommandKind, CommandRegistry, CommandResult, CommandSpec, Permission, RejectReason,
};

/// インデントの 1 段 (v0.1 は定数: 空白 4 つ。設定の公開は後の版)。
const INDENT_WIDTH: usize = 4;

/// 1 つの Range に対する編集。
struct Plan {
    del: ByteRange,
    ins: String,
    cursor: CursorRule,
}

enum CursorRule {
    /// 挿入の直後 (空のカーソルになる)。
    AfterInsert,
    /// 元の Range を、この編集による写像 (`MapPolicy::Own`) で追う。
    Keep,
}

/// 操作の対象 (Window の View とその文書)。
fn active(ctx: &CommandCtx<'_>) -> Option<(ViewId, DocumentId)> {
    let w = ctx.state.first_window()?;
    let v = ctx.state.window_view(w)?;
    Some((v, ctx.state.view(v)?.doc))
}

/// 重なる編集を切り詰める (03 §4.5 の 2): 先行する Range を優先し、後続は重なる部分を切り詰める。
/// 切り詰めて何も残らない編集は捨てる。先行より前にさかのぼる編集 (順序が逆) は捨てる。
fn trim_overlaps(plans: Vec<Option<Plan>>) -> Vec<Option<Plan>> {
    let mut covered_end = 0usize;
    let mut prev_start = 0usize;
    plans
        .into_iter()
        .map(|p| {
            let mut p = p?;
            if p.del.start.0 < prev_start {
                return None;
            }
            if p.del.start.0 < covered_end {
                p.del = ByteRange::new(ByteOff(covered_end), ByteOff(p.del.end.0.max(covered_end)));
            }
            if p.del.is_empty() && p.ins.is_empty() {
                return None;
            }
            prev_start = p.del.start.0;
            covered_end = covered_end.max(p.del.end.0);
            Some(p)
        })
        .collect()
}

/// 各 Range に `f` を適用して 1 つの Transaction として適用する。`f` が全部 `None` なら何もしない。
fn edit_each(
    ctx: &mut CommandCtx<'_>,
    f: impl Fn(&Document, &Range) -> Option<Plan>,
) -> CommandResult {
    edit_each_as(ctx, Origin::User, f)
}

fn edit_each_as(
    ctx: &mut CommandCtx<'_>,
    origin: Origin,
    f: impl Fn(&Document, &Range) -> Option<Plan>,
) -> CommandResult {
    let Some((view_id, doc_id)) = active(ctx) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let (Some(doc), Some(view)) = (ctx.state.document(doc_id), ctx.state.view(view_id)) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let before = view.selection.clone();
    let plans = trim_overlaps(before.iter().map(|r| f(doc, r)).collect());
    if plans.iter().all(Option::is_none) {
        return CommandResult::Done;
    }
    let edits: Vec<Edit> = plans
        .iter()
        .flatten()
        .map(|p| Edit {
            range: p.del,
            insert: p.ins.as_str().into(),
        })
        .collect();
    let Ok(cs) = ChangeSet::from_sorted_edits(&doc.snapshot(), edits) else {
        return CommandResult::Rejected(RejectReason::InvalidArgs);
    };
    // 適用後の Selection: 挿入の直後に置くものは数値で、それ以外は写像で求める
    let mut delta: i64 = 0;
    let mut ranges = Vec::with_capacity(plans.len());
    for (r, plan) in before.iter().zip(&plans) {
        match plan {
            Some(p) => {
                let new_start = (p.del.start.0 as i64 + delta) as usize;
                delta += p.ins.len() as i64 - p.del.len() as i64;
                ranges.push(match p.cursor {
                    CursorRule::AfterInsert => Range::point(ByteOff(new_start + p.ins.len())),
                    CursorRule::Keep => r.map(&cs, Bias::After),
                });
            }
            // 自分の編集がない Range は、他の Range の挿入位置では前に留まる (03 §4.5 の 3)
            None => ranges.push(r.map(&cs, Bias::Before)),
        }
    }
    let Some(after) = Selection::new(ranges, before.primary_index()) else {
        return CommandResult::Rejected(RejectReason::InvalidArgs);
    };
    let tx = Transaction::new(cs, origin, ctx.state.now())
        .with_view(view_id)
        .with_selections(Some(before), Some(after));
    match ctx.state.apply_transaction(doc_id, tx) {
        Ok(_) => CommandResult::Done,
        Err(_) => CommandResult::Rejected(RejectReason::ReadOnly),
    }
}

// ---------- 改行コードの変換 (03 §8.4、FR-113) ----------

/// `file.set_line_ending { eol }`。`eol` と異なるすべての区切りを `eol` に置き換え、`default_eol` も `eol` にする。
/// 1 つの Transaction (`UndoGroupHint::Break`) で適用し、Undo ノードの `format_change` に前後の書式を持たせる (1 回の Undo で本文と
/// `default_eol` が戻る、03 §7.2)。変える改行が無ければ本文は変えず、`default_eol` だけを変える (メタデータ。Undo の対象外)。
fn set_line_ending(ctx: &mut CommandCtx<'_>) -> CommandResult {
    use crate::document::UndoGroupHint;
    use crate::document::{EolStats, FormatChange, FormatState};
    use eve_text::LineEnding;
    use eve_view::{CommandId, MessageLevel};
    let eol = match ctx.args.str("eol").map(str::to_ascii_lowercase).as_deref() {
        Some("lf") => LineEnding::Lf,
        Some("crlf") => LineEnding::Crlf,
        Some("cr") => LineEnding::Cr,
        _ => return CommandResult::Rejected(RejectReason::InvalidArgs),
    };
    let name = match eol {
        LineEnding::Lf => "LF",
        LineEnding::Crlf => "CRLF",
        LineEnding::Cr => "CR",
    };
    let Some(doc_id) = ctx.state.active_document() else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let Some(view_id) = ctx
        .state
        .first_window()
        .and_then(|w| ctx.state.window_view(w))
    else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let Some(doc) = ctx.state.document(doc_id) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    // (1) 編集できない文書は拒否する (読み取り専用・読込中、03 §9.6)
    if !doc.is_editable() {
        return CommandResult::Rejected(RejectReason::ReadOnly);
    }
    // (2) 変える改行の数 k (`eol` 以外の 2 種の和。走査しない)
    let stats = doc.format.eol_stats;
    let same = match eol {
        LineEnding::Lf => stats.lf,
        LineEnding::Crlf => stats.crlf,
        LineEnding::Cr => stats.cr,
    };
    let k = stats.total() - same;
    if k == 0 {
        ctx.state.set_default_eol(doc_id, eol);
        ctx.state
            .show_message(MessageLevel::Info, "info.eol.unchanged", &[("eol", name)]);
        return CommandResult::Done;
    }
    // (3) `eol` と異なるすべての区切りを置き換える ChangeSet を作る
    let snapshot = doc.snapshot();
    let body = snapshot.slice_to_string(ByteRange::all(snapshot.len_bytes()));
    let b = body.as_bytes();
    let mut edits: Vec<Edit> = Vec::with_capacity(k);
    let mut i = 0;
    while i < b.len() {
        let (len, kind) = match b[i] {
            b'\r' if b.get(i + 1) == Some(&b'\n') => (2, LineEnding::Crlf),
            b'\r' => (1, LineEnding::Cr),
            b'\n' => (1, LineEnding::Lf),
            _ => {
                i += 1;
                continue;
            }
        };
        if kind != eol {
            edits.push(Edit {
                range: ByteRange::new(ByteOff(i), ByteOff(i + len)),
                insert: eol.as_str().into(),
            });
        }
        i += len;
    }
    let Ok(cs) = ChangeSet::from_sorted_edits(&snapshot, edits) else {
        return CommandResult::Rejected(RejectReason::InvalidArgs);
    };
    let before = doc.format.clone();
    let mut after = before.clone();
    after.default_eol = eol;
    after.eol_stats = match eol {
        LineEnding::Lf => EolStats {
            lf: stats.total(),
            crlf: 0,
            cr: 0,
        },
        LineEnding::Crlf => EolStats {
            lf: 0,
            crlf: stats.total(),
            cr: 0,
        },
        LineEnding::Cr => EolStats {
            lf: 0,
            crlf: 0,
            cr: stats.total(),
        },
    };
    let read_only = doc.read_only;
    let change = FormatChange {
        before: FormatState {
            format: before,
            read_only,
        },
        after: FormatState {
            format: after,
            read_only,
        },
    };
    // (5) 1 つの Transaction。選択は ChangeSet で写像する
    let sel = ctx.state.view(view_id).map(|v| v.selection.clone());
    let tx = Transaction::new(
        cs,
        Origin::Command(CommandId::new("file.set_line_ending")),
        ctx.state.now(),
    )
    .with_view(view_id)
    .with_undo_group(UndoGroupHint::Break)
    .with_selections(sel, None)
    .with_format_change(change);
    match ctx.state.apply_transaction(doc_id, tx) {
        Ok(_) => {
            ctx.state.show_message(
                MessageLevel::Info,
                "info.eol.converted",
                &[("count", &k.to_string()), ("eol", name)],
            );
            CommandResult::Done
        }
        Err(_) => CommandResult::Rejected(RejectReason::ReadOnly),
    }
}

// ---------- 文字入力・改行 ----------

fn insert_text(ctx: &mut CommandCtx<'_>) -> CommandResult {
    let Some(text) = ctx.args.str("text") else {
        return CommandResult::Rejected(RejectReason::InvalidArgs);
    };
    let text = text.to_owned();
    if text.is_empty() {
        return CommandResult::Done;
    }
    edit_each(ctx, |_, r| {
        Some(Plan {
            del: ByteRange::new(r.start(), r.end()),
            ins: text.clone(),
            cursor: CursorRule::AfterInsert,
        })
    })
}

/// 行の先頭の空白 (空白とタブ) の終端。
fn indent_end(doc: &Document, line_start: usize, line_end: usize) -> usize {
    let s = doc
        .text()
        .slice_to_string(ByteRange::new(ByteOff(line_start), ByteOff(line_end)));
    line_start + s.bytes().take_while(|b| matches!(b, b' ' | b'\t')).count()
}

fn newline(ctx: &mut CommandCtx<'_>) -> CommandResult {
    edit_each(ctx, |doc, r| {
        let p = r.start();
        let text = doc.text();
        let line = text.byte_to_line(p);
        let (ls, le) = (text.line_to_byte(line).0, text.line_end_byte(line).0);
        // 参照行の行頭空白のうち p より前の部分を複製する (03 §15.3)
        let copy_end = indent_end(doc, ls, le).min(p.0);
        let indent = text.slice_to_string(ByteRange::new(ByteOff(ls), ByteOff(copy_end)));
        let ins = format!("{}{}", doc.format.default_eol.as_str(), indent);
        Some(Plan {
            del: ByteRange::new(r.start(), r.end()),
            ins,
            cursor: CursorRule::AfterInsert,
        })
    })
}

// ---------- 貼り付け ----------

/// 改行の正規化 (03 §8.4): `\r\n`・単独の `\r`・単独の `\n` をそれぞれ 1 つの `eol` にする。
pub fn normalize_eol(text: &str, eol: eve_text::LineEnding) -> String {
    let eol = eol.as_str();
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push_str(eol);
            }
            '\n' => out.push_str(eol),
            c => out.push(c),
        }
    }
    out
}

/// `InputEvent::Paste` の本文を挿入する。改行を文書の `default_eol` に正規化し、自動インデントは適用しない。
/// 本文の制御文字はコマンドとして解釈せず文字として入れる。Origin は `Paste` (1 操作 = 1 Undo)。
pub(super) fn paste_text(state: &mut super::state::EditorState, text: &str) -> CommandResult {
    let args = super::command::Args::new();
    let mut ctx = CommandCtx { state, args: &args };
    edit_each_as(&mut ctx, Origin::Paste, |doc, r| {
        let ins = normalize_eol(text, doc.format.default_eol);
        Some(Plan {
            del: ByteRange::new(r.start(), r.end()),
            ins,
            cursor: CursorRule::AfterInsert,
        })
    })
}

// ---------- 削除 ----------

fn delete_with(
    ctx: &mut CommandCtx<'_>,
    span: impl Fn(&Document, ByteOff) -> ByteRange,
) -> CommandResult {
    edit_each(ctx, |doc, r| {
        let del = if r.is_empty() {
            span(doc, r.head)
        } else {
            ByteRange::new(r.start(), r.end())
        };
        (!del.is_empty()).then(|| Plan {
            del,
            ins: String::new(),
            cursor: CursorRule::AfterInsert,
        })
    })
}

fn delete_backward(ctx: &mut CommandCtx<'_>) -> CommandResult {
    delete_with(ctx, |d, p| {
        ByteRange::new(prev_grapheme_boundary(d.text(), p), p)
    })
}

fn delete_forward(ctx: &mut CommandCtx<'_>) -> CommandResult {
    delete_with(ctx, |d, p| {
        ByteRange::new(p, next_grapheme_boundary(d.text(), p))
    })
}

fn delete_word_backward(ctx: &mut CommandCtx<'_>) -> CommandResult {
    delete_with(ctx, |d, p| ByteRange::new(prev_word_stop(d.text(), p), p))
}

fn delete_word_forward(ctx: &mut CommandCtx<'_>) -> CommandResult {
    delete_with(ctx, |d, p| ByteRange::new(p, next_word_stop(d.text(), p)))
}

// ---------- Undo / Redo ----------

fn undo(ctx: &mut CommandCtx<'_>) -> CommandResult {
    let Some((view, doc)) = active(ctx) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    match ctx.state.undo(doc, Some(view)) {
        Ok(_) => CommandResult::Done,
        Err(_) => CommandResult::Rejected(RejectReason::ReadOnly),
    }
}

fn redo(ctx: &mut CommandCtx<'_>) -> CommandResult {
    let Some((view, doc)) = active(ctx) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    match ctx.state.redo(doc, Some(view)) {
        Ok(_) => CommandResult::Done,
        Err(_) => CommandResult::Rejected(RejectReason::ReadOnly),
    }
}

// ---------- 全選択 ----------

fn select_all(ctx: &mut CommandCtx<'_>) -> CommandResult {
    let Some((view, doc)) = active(ctx) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let len = ctx.state.document(doc).map_or(0, Document::len_bytes);
    ctx.state.set_selection(
        view,
        Selection::single(Range::new(ByteOff(0), ByteOff(len))),
    );
    CommandResult::Done
}

// ---------- インデント (Tab / Shift+Tab) ----------

/// 行頭から `end` までの表示列 (タブは次のタブストップまで)。
fn display_col(doc: &Document, line_start: usize, end: usize, cfg: &WidthConfig) -> usize {
    let s = doc
        .text()
        .slice_to_string(ByteRange::new(ByteOff(line_start), ByteOff(end)));
    str_display_width(&s, cfg)
}

/// Tab キー (選択なし): 次のインデント停止位置までの空白を挿入する。選択があるときは何もしない (v0.1、OQ-193)。
fn indent(ctx: &mut CommandCtx<'_>) -> CommandResult {
    edit_each(ctx, |doc, r| {
        if !r.is_empty() {
            return None;
        }
        let text = doc.text();
        let line = text.byte_to_line(r.head);
        let ls = text.line_to_byte(line).0;
        let col = display_col(doc, ls, r.head.0, &WidthConfig::default());
        let n = INDENT_WIDTH - col % INDENT_WIDTH;
        Some(Plan {
            del: ByteRange::new(r.head, r.head),
            ins: " ".repeat(n),
            cursor: CursorRule::AfterInsert,
        })
    })
}

/// Shift+Tab (選択なし): 行頭空白のうち、表示幅が `INDENT_WIDTH` 以内に収まる部分を削除する。選択があるときは何もしない。
fn outdent(ctx: &mut CommandCtx<'_>) -> CommandResult {
    edit_each(ctx, |doc, r| {
        if !r.is_empty() {
            return None;
        }
        let cfg = WidthConfig::default();
        let tab = cfg.tab_width.max(1) as usize;
        let text = doc.text();
        let line = text.byte_to_line(r.head);
        let (ls, le) = (text.line_to_byte(line).0, text.line_end_byte(line).0);
        let lead = text.slice_to_string(ByteRange::new(
            ByteOff(ls),
            ByteOff(indent_end(doc, ls, le)),
        ));
        let (mut col, mut bytes) = (0usize, 0usize);
        for ch in lead.chars() {
            let next = if ch == '\t' {
                col + tab - col % tab
            } else {
                col + 1
            };
            if next > INDENT_WIDTH {
                // 先頭の 1 文字が幅を超えるタブなら、そのタブ 1 つを削除する (何も起きない状態を避ける)
                if bytes == 0 && ch == '\t' {
                    bytes = 1;
                }
                break;
            }
            col = next;
            bytes += ch.len_utf8();
        }
        (bytes > 0).then(|| Plan {
            del: ByteRange::new(ByteOff(ls), ByteOff(ls + bytes)),
            ins: String::new(),
            cursor: CursorRule::Keep,
        })
    })
}

// ---------- 登録 ----------

macro_rules! reg {
    ($r:expr, $id:expr, $kind:expr, $perm:expr, $when:expr, $h:expr) => {
        $r.register(
            CommandSpec::new($id, $kind, $perm, $when, $h)
                .expect("カタログの when 句は構文が正しい"),
        )
    };
}

/// v0.1 の編集コマンド (07 §11.6 の「編集」。カタログが正本)。
pub(super) fn register(r: &mut CommandRegistry) {
    use CommandKind::{Edit, Motion};
    use Permission::{None as NoPerm, Propose, Read};
    let w = "!doc.read_only";
    reg!(r, "file.set_line_ending", Edit, Propose, w, set_line_ending);
    reg!(r, "edit.insert_text", Edit, Propose, w, insert_text);
    reg!(r, "edit.newline", Edit, Propose, w, newline);
    reg!(r, "edit.undo", Edit, NoPerm, w, undo);
    reg!(r, "edit.redo", Edit, NoPerm, w, redo);
    reg!(r, "edit.delete_backward", Edit, Propose, w, delete_backward);
    reg!(r, "edit.delete_forward", Edit, Propose, w, delete_forward);
    reg!(
        r,
        "edit.delete_word_backward",
        Edit,
        Propose,
        w,
        delete_word_backward
    );
    reg!(
        r,
        "edit.delete_word_forward",
        Edit,
        Propose,
        w,
        delete_word_forward
    );
    reg!(r, "edit.indent", Edit, Propose, w, indent);
    reg!(r, "edit.outdent", Edit, Propose, w, outdent);
    reg!(r, "select.all", Motion, Read, "", select_all);
}
