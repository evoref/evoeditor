//! 編集コマンド (docs/03 §5.3 / §15.3、docs/07 §11.6 の「編集」)。
//!
//! どのコマンドも Selection の各 Range に同じ規則を適用して 1 つの `ChangeSet` に合成し、1 つの Transaction として
//! 適用する (03 §4.5)。変更が無いと分かれば Transaction を作らない (Revision も履歴も変わらない)。
//!
//! spec: 03 §4.5, 03 §4.5.2, 03 §4.5.3, 03 §15.3

use eve_text::{
    Bias, ByteOff, ByteRange, ChangeSet, Edit, Range, Selection, TextRead, next_grapheme_boundary,
    prev_grapheme_boundary,
};
use eve_text::{WidthConfig, str_display_width};
use eve_view::{DocumentId, ViewId};

use crate::command::auto_pairs;
use crate::command::ids;
use crate::command::insert_rule::{self, EscapeContext, InsertNotice};
use crate::document::{Document, Origin, Transaction};
use crate::ui_model::keys;

use crate::command::{
    CommandCtx, CommandKind, CommandRegistry, CommandResult, Permission, RejectReason,
};
use crate::view::motion::{indent_end, word_stop};

/// 1 つの Range に対する編集。
struct Plan {
    del: ByteRange,
    /// 入れる文字列 (共有。全文を各 Range へ入れる貼り付けで、Range ごとに写しを作らない)。
    ins: std::sync::Arc<str>,
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
    let w = ctx.state.focused_window()?;
    let v = ctx.state.window_view(w)?;
    Some((v, ctx.state.view(v)?.doc))
}

/// 重なる編集を切り詰める (03 §4.5 の 2): 先行する Range を優先し、後続は先行の削除の範囲に入る部分を切り詰める
/// (削除の始まりを、それまでの削除の末尾 `covered_end` まで後ろへずらす。挿入はずらした始まりに入る)。削除も挿入も
/// 空の編集 (切り詰めて何も残らない・空の片の貼り付け) は、挿入の直後に置くカーソルなら、その Range のカーソルを先行の
/// 編集の後ろ (文書の順のまま) に置くために残し (文書は変えない)、写像で追うカーソルなら捨てる。結果の削除の範囲は
/// 昇順で重ならない (`from_sorted_edits` の前提)。
fn trim_overlaps(plans: Vec<Option<Plan>>) -> Vec<Option<Plan>> {
    let mut covered_end = 0usize;
    plans
        .into_iter()
        .map(|p| {
            let mut p = p?;
            if p.del.start.0 < covered_end {
                p.del = ByteRange::new(ByteOff(covered_end), ByteOff(p.del.end.0.max(covered_end)));
            }
            if p.del.is_empty() && p.ins.is_empty() && !matches!(p.cursor, CursorRule::AfterInsert)
            {
                return None;
            }
            covered_end = p.del.end.0;
            Some(p)
        })
        .collect()
}

/// 各 Range に `f` を適用して 1 つの Transaction として適用する。`f` が全部 `None` なら何もしない。
fn edit_each(
    ctx: &mut CommandCtx<'_>,
    f: impl Fn(&Document, &Range) -> Option<Plan>,
) -> CommandResult {
    edit_each_as(ctx, Origin::User, |d, _, r| f(d, r))
}

/// `edit_each` と同じだが、`f` は Range の番号 (文書の順、0 始まり) も受け取り、Transaction の `Origin` を指定する。
fn edit_each_as(
    ctx: &mut CommandCtx<'_>,
    origin: Origin,
    f: impl Fn(&Document, usize, &Range) -> Option<Plan>,
) -> CommandResult {
    let Some((view_id, doc_id)) = active(ctx) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let (Some(doc), Some(view)) = (ctx.state.document(doc_id), ctx.state.view(view_id)) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let before = view.selection.clone();
    let plans = trim_overlaps(
        before
            .iter()
            .enumerate()
            .map(|(i, r)| f(doc, i, r))
            .collect(),
    );
    // どの Range も文書を変えないなら何もしない (選択も動かさない)
    if plans
        .iter()
        .flatten()
        .all(|p| p.del.is_empty() && p.ins.is_empty())
    {
        return CommandResult::Done;
    }
    let edits: Vec<Edit> = plans
        .iter()
        .flatten()
        .map(|p| Edit {
            range: p.del,
            insert: std::sync::Arc::clone(&p.ins),
        })
        .collect();
    let cs = match ChangeSet::from_sorted_edits(&doc.snapshot(), edits) {
        Ok(cs) => cs,
        Err(e) => {
            // どれもハンドラの誤り。リリースでは何もせずに終える
            match e {
                eve_text::EditError::Overlap => {
                    debug_assert!(false, "切り詰めた後の編集が重なる (03 §4.5 の 2)");
                }
                other => debug_assert!(false, "編集の端が char の境界でない・範囲外: {other:?}"),
            }
            return CommandResult::Rejected(RejectReason::InvalidArgs);
        }
    };
    // 適用後の Selection: 挿入の直後に置くものは数値で、それ以外は写像で求める
    // 前の編集で足した量と消した量 (符号付きの変換をしない)。編集は昇順で重ならないので、
    // 開始 + 足した量 >= 消した量
    let mut added = 0usize;
    let mut removed = 0usize;
    let mut ranges = Vec::with_capacity(plans.len());
    for (r, plan) in before.iter().zip(&plans) {
        match plan {
            Some(p) => {
                let new_start = (p.del.start.0 + added) - removed;
                added += p.ins.len();
                removed += p.del.len();
                ranges.push(match p.cursor {
                    CursorRule::AfterInsert => Range::point(ByteOff(new_start + p.ins.len())),
                    CursorRule::Keep => r.map(&cs, Bias::After),
                });
            }
            // 自分の編集がない Range は、他の Range の挿入位置では前に留まる (03 §4.5 の 3)
            None => ranges.push(r.map(&cs, Bias::Before)),
        }
    }
    // 重なった・同じ位置になった Range を併合してから記録する (Undo / Redo で戻す選択も INV-S1〜S3 を満たす)
    let Some(after) =
        Selection::new(ranges, before.primary_index()).map(|s| s.merged(cs.len_after()))
    else {
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

// ---------- クリップボード (03 §15.3) ----------

/// `clipboard.cut`: 選択をクリップボードへ入れてから削除する。空でない Range が無ければ何もしない。空でない Range を
/// 全部消し (1 つの Transaction)、空のカーソルはそのまま写す (03 §4.5.3 の MCUR-9)。
fn clipboard_cut(ctx: &mut CommandCtx<'_>) -> CommandResult {
    if !ctx.state.copy_selection() {
        return CommandResult::Done;
    }
    edit_each_as(
        ctx,
        Origin::Command(eve_view::CommandId::new(ids::CLIPBOARD_CUT)),
        |_, _, r| {
            (!r.is_empty()).then(|| Plan {
                del: ByteRange::new(r.start(), r.end()),
                ins: "".into(),
                cursor: CursorRule::AfterInsert,
            })
        },
    )
}

// ---------- 改行コードの変換 (03 §8.4、FR-113) ----------

/// `file.set_line_ending { eol }`。`eol` と異なるすべての区切りを `eol` に置き換え、`default_eol` も `eol` にする。
/// 1 つの Transaction (`UndoGroupHint::Break`) で適用し、Undo ノードの `format_change` に前後の `default_eol` だけを持たせる (1 回の Undo で本文と
/// `default_eol` が戻る、03 §7.2)。変える改行が無ければ本文は変えず、`default_eol` だけを変える (メタデータ。Undo の対象外)。
fn set_line_ending(ctx: &mut CommandCtx<'_>) -> CommandResult {
    use eve_text::LineEnding;
    use eve_view::MessageLevel;
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
        .focused_window()
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
        ctx.state.show_message(
            MessageLevel::Info,
            keys::INFO_EOL_UNCHANGED,
            &[("eol", name)],
        );
        return CommandResult::Done;
    }
    // (3)〜(5) ChangeSet を作って 1 つの Transaction で適用する (1 MiB 超はジョブ。editor/eol.rs)
    ctx.state.convert_eol(doc_id, view_id, eol, k);
    CommandResult::Done
}

// ---------- 文字入力・改行 ----------

/// `edit.insert_text` (文字の入力 `InputEvent::Text`・マクロ・駆動 API)。挿入の入口 (2) (03 §8.3.1): 外からの文字列として、
/// 挿入先が表記にする文書なら `U+10FF00..` を `<U+10FFNN>` にして知らせる。
fn insert_text(ctx: &mut CommandCtx<'_>) -> CommandResult {
    let Some(text) = ctx.args.str("text") else {
        return CommandResult::Rejected(RejectReason::InvalidArgs);
    };
    let text = text.to_owned();
    if text.is_empty() {
        return CommandResult::Done;
    }
    let Some(target) = target_context(ctx) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let (text, pua) = insert_rule::external_text(&text, target);
    let notices: Vec<InsertNotice> = (pua > 0)
        .then_some(InsertNotice::PuaAsText(pua))
        .into_iter()
        .collect();
    // 打鍵の経路の入力で、自動閉じが効いていて組の文字を含むなら、書記素ごとに自動閉じの規則を当てる (03 §15.7 の APR-2)
    if let Some((view_id, doc_id, pairs)) = typed_pairs(ctx, &text) {
        let r = if ctx
            .state
            .document(doc_id)
            .is_some_and(Document::is_editable)
        {
            auto_pairs::insert_typed(ctx, view_id, doc_id, &text, &pairs)
        } else {
            CommandResult::Rejected(RejectReason::ReadOnly)
        };
        if matches!(r, CommandResult::Done) {
            show_notices(ctx.state, &notices);
        }
        return r;
    }
    let text: std::sync::Arc<str> = text.into();
    let r = edit_each(ctx, |_, r| {
        Some(Plan {
            del: ByteRange::new(r.start(), r.end()),
            ins: std::sync::Arc::clone(&text),
            cursor: CursorRule::AfterInsert,
        })
    });
    if matches!(r, CommandResult::Done) {
        show_notices(ctx.state, &notices);
    }
    r
}

/// 打鍵の経路の入力 `text` に自動閉じを当てるなら、対象 (View, 文書) と組 (03 §15.7 の APR-1 / APR-2 (a))。打鍵の経路で
/// ない (キーマップの割当・パレット)・自動閉じを切っている・組が無い・組の文字を含まないときは `None` (今の入力)。
fn typed_pairs(
    ctx: &CommandCtx<'_>,
    text: &str,
) -> Option<(ViewId, DocumentId, auto_pairs::Pairs)> {
    if ctx.source != crate::command::InvocationSource::Typing {
        return None;
    }
    let (view, doc) = active(ctx)?;
    let pairs = auto_pairs::pairs_for(ctx.state, ctx.state.document(doc)?)?;
    text.chars()
        .any(|ch| pairs.iter().any(|(o, c)| *o == ch || *c == ch))
        .then_some((view, doc, pairs))
}

/// 操作の対象の文書の、本文に入れる規則に効く値 (03 §8.3.1)。
fn target_context(ctx: &CommandCtx<'_>) -> Option<EscapeContext> {
    let (_, doc) = active(ctx)?;
    Some(EscapeContext::of(&ctx.state.document(doc)?.format))
}

/// 本文に入れたときの知らせを出す (`info.paste.*`、04 §15.2)。
pub(crate) fn show_notices(
    state: &mut crate::editor::state::EditorState,
    notices: &[InsertNotice],
) {
    for n in notices {
        let count = n.count().map(|c| c.to_string());
        let args: Vec<(&str, &str)> = count.iter().map(|c| ("count", c.as_str())).collect();
        state.show_message(eve_view::MessageLevel::Info, n.key(), &args);
    }
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
            ins: ins.into(),
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

/// `editor.paste_normalize_eol` (07 §5.13。公開は後の版で、今は常に正規化する)。
const PASTE_NORMALIZE_EOL: bool = true;

/// 複数カーソルへの貼り付けで入れる合計のバイト数の上限 (03 §4.5.3 の MCUR-10。定数)。超えたら何もせずに知らせる。
pub const MULTI_PASTE_MAX_BYTES: usize = 64 * 1024 * 1024;

/// 貼り付けの本文を挿入する (挿入の入口 (1)、03 §8.3.1: `edit.paste` の応答 `InputEvent::ClipboardContent` と、端末からの
/// 貼り付け `InputEvent::Paste`)。`text` は届いた文字列 (読めなかったときは `None` で、内部レジスタを使う)。
/// 届いた文字列を内部レジスタと照合し、改行を文書の `default_eol` に正規化し (§8.4)、エスケープ文字・私用文字の
/// 規則をかけてから入れる (`insert_rule::resolve_paste`)。自動インデントは適用しない。本文の制御文字はコマンドとして
/// 解釈せず文字として入れる。Origin は `Paste` (1 操作 = 1 Undo)。
///
/// Range が 2 つ以上なら、内部レジスタの片 → 行 → 全文の 3 段で分ける (03 §4.5.3 の MCUR-10、
/// `insert_rule::resolve_multi_paste`)。入れる合計が `MULTI_PASTE_MAX_BYTES` を超えるなら何もせずに知らせる。高さ 2 以上の
/// 矩形があり、内部レジスタの片の数が矩形の高さと同じなら、矩形の行ごとに片を入れる (03 §4.8 の BOX-4)。
pub(crate) fn paste_text(
    state: &mut crate::editor::state::EditorState,
    text: Option<&str>,
) -> CommandResult {
    let args = crate::command::Args::new();
    let mut ctx = CommandCtx {
        state,
        args: &args,
        source: crate::command::InvocationSource::Keymap,
    };
    let Some((view_id, doc_id)) = active(&ctx) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let (Some(doc), Some(view)) = (ctx.state.document(doc_id), ctx.state.view(view_id)) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    // 編集できない文書には、分けて入れる計画も上限の判定もしない (読み取り専用・読込中。03 §9.6)
    if !doc.is_editable() {
        return CommandResult::Rejected(RejectReason::ReadOnly);
    }
    let target = EscapeContext::of(&doc.format);
    let eol = doc.format.default_eol;
    let ranges = view.selection.len();
    let box_rows = ctx.state.box_paste_rows(view_id);
    let register = ctx.state.clipboard_register();
    // 矩形への貼り付け (03 §4.8 の BOX-4): 内部レジスタの片の数が矩形の高さと同じで、届いた文字列が内部レジスタと一致する
    // なら、矩形の i 行目に i 番目の片を入れる (Range の無い行の片は入れない)。そうでなければ下の MCUR-10 の 3 段
    // 高さ 1 の矩形 (Range は 1 つ) は今の経路のまま
    if let Some((height, rows)) = box_rows.filter(|(height, _)| *height >= 2)
        && register
            .is_some_and(|r| r.pieces().len() == height && text.is_none_or(|a| r.matches_text(a)))
    {
        let plan = match insert_rule::resolve_multi_paste(
            text,
            register,
            target,
            eol,
            PASTE_NORMALIZE_EOL,
            height,
            MULTI_PASTE_MAX_BYTES,
        ) {
            None => return CommandResult::Done,
            Some(insert_rule::MultiPaste::TooLarge) => {
                ctx.state.show_message(
                    eve_view::MessageLevel::Info,
                    keys::INFO_PASTE_MULTI_TOO_LARGE,
                    &[],
                );
                return CommandResult::Done;
            }
            Some(insert_rule::MultiPaste::Plan(plan)) => plan,
        };
        let r = edit_each_as(&mut ctx, Origin::Paste, |_, i, r| {
            Some(Plan {
                del: ByteRange::new(r.start(), r.end()),
                ins: plan.text_for(rows.get(i).copied().unwrap_or(i)),
                cursor: CursorRule::AfterInsert,
            })
        });
        if matches!(r, CommandResult::Done) {
            show_notices(ctx.state, &plan.notices);
        }
        return r;
    }
    if ranges >= 2 {
        let plan = match insert_rule::resolve_multi_paste(
            text,
            register,
            target,
            eol,
            PASTE_NORMALIZE_EOL,
            ranges,
            MULTI_PASTE_MAX_BYTES,
        ) {
            None => return CommandResult::Done,
            Some(insert_rule::MultiPaste::TooLarge) => {
                ctx.state.show_message(
                    eve_view::MessageLevel::Info,
                    keys::INFO_PASTE_MULTI_TOO_LARGE,
                    &[],
                );
                return CommandResult::Done;
            }
            Some(insert_rule::MultiPaste::Plan(plan)) => plan,
        };
        let r = edit_each_as(&mut ctx, Origin::Paste, |_, i, r| {
            Some(Plan {
                del: ByteRange::new(r.start(), r.end()),
                ins: plan.text_for(i),
                cursor: CursorRule::AfterInsert,
            })
        });
        if matches!(r, CommandResult::Done) {
            show_notices(ctx.state, &plan.notices);
        }
        return r;
    }
    let Some(plan) = insert_rule::resolve_paste(text, register, target, eol, PASTE_NORMALIZE_EOL)
    else {
        return CommandResult::Done;
    };
    let ins: std::sync::Arc<str> = plan.text.into();
    let r = edit_each_as(&mut ctx, Origin::Paste, |_, _, r| {
        Some(Plan {
            del: ByteRange::new(r.start(), r.end()),
            ins: std::sync::Arc::clone(&ins),
            cursor: CursorRule::AfterInsert,
        })
    });
    if matches!(r, CommandResult::Done) {
        show_notices(ctx.state, &plan.notices);
    }
    r
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
            ins: "".into(),
            cursor: CursorRule::AfterInsert,
        })
    })
}

fn delete_backward(ctx: &mut CommandCtx<'_>) -> CommandResult {
    let unit = ctx.state.config().backspace_unit;
    let span = move |d: &Document, p: ByteOff| {
        let g = prev_grapheme_boundary(d.text(), p);
        match unit {
            eve_config::BackspaceUnit::Grapheme => ByteRange::new(g, p),
            // コードポイント単位: 書記素の最後の 1 文字だけを消す (結合文字・ハングル字母を 1 つずつ戻す)
            eve_config::BackspaceUnit::Codepoint => {
                let last = d
                    .text()
                    .slice_to_string(ByteRange::new(g, p))
                    .chars()
                    .last()
                    .map_or(0, char::len_utf8);
                ByteRange::new(ByteOff(p.0 - last), p)
            }
        }
    };
    // 自動で足した閉じ字と開き字の間なら両方を消す (03 §15.7 の APR-4)
    if let Some((view_id, doc_id)) = active(ctx)
        && let Some(r) = auto_pairs::delete_backward_pairs(ctx, view_id, doc_id, &span)
    {
        return r;
    }
    delete_with(ctx, span)
}

fn delete_forward(ctx: &mut CommandCtx<'_>) -> CommandResult {
    delete_with(ctx, |d, p| {
        ByteRange::new(p, next_grapheme_boundary(d.text(), p))
    })
}

fn delete_word_backward(ctx: &mut CommandCtx<'_>) -> CommandResult {
    delete_with(ctx, |d, p| ByteRange::new(word_stop(d, p, false), p))
}

fn delete_word_forward(ctx: &mut CommandCtx<'_>) -> CommandResult {
    delete_with(ctx, |d, p| ByteRange::new(p, word_stop(d, p, true)))
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

/// 行頭から `end` までの表示列 (タブは次のタブストップまで)。`from` は同じ行の `end` 以前の位置とその表示列で、
/// あればそこから数え続ける (複数カーソルで同じ行のカーソルを行頭から数え直さない。03 §4.5.4 の「費用」)。
fn display_col(
    doc: &Document,
    line_start: usize,
    end: usize,
    from: Option<(usize, usize)>,
    cfg: &WidthConfig,
) -> usize {
    // 長大行は行頭から文字列にしない (列の索引で数える、03 §11.5.1)
    let line = doc.text().byte_to_line(ByteOff(line_start)).0;
    if crate::layout::is_long_line(doc, line) {
        return crate::layout::cursor_col(doc, line, end - line_start);
    }
    let (start, col) = from
        .filter(|&(p, _)| line_start <= p && p <= end)
        .unwrap_or((line_start, 0));
    let s = doc
        .text()
        .slice_to_string(ByteRange::new(ByteOff(start), ByteOff(end)));
    if col == 0 {
        return str_display_width(&s, cfg);
    }
    // タブの幅は、その前の表示列で決まるので、続きから数える
    let tab = usize::from(cfg.tab_width.max(1));
    eve_text::grapheme_indices(&s)
        .into_iter()
        .fold(col, |c, (_, g)| {
            if g == "\t" {
                c + tab - c % tab
            } else {
                c + usize::from(eve_text::grapheme_width(g, cfg))
            }
        })
}

/// Tab キー (選択なし): 次のインデント停止位置までの空白を挿入する。選択があるときは何もしない (v0.1、OQ-193)。
/// 選択がある (空でない Range が 1 つ以上ある)。
fn has_selection(ctx: &CommandCtx<'_>) -> bool {
    active(ctx)
        .and_then(|(v, _)| ctx.state.view(v))
        .is_some_and(|v| v.selection.iter().any(|r| !r.is_empty()))
}

/// フォーカスの文書の字下げの単位と幅 (文書ごとの値、07 §3.6。言語定義の `indent` が効く、01 FR-704 AC-1)。
fn doc_indent(ctx: &CommandCtx<'_>) -> (eve_config::IndentUnit, usize) {
    let i = active(ctx)
        .and_then(|(_, d)| ctx.state.document(d))
        .map_or_else(crate::document::IndentSettings::default, |d| d.indent);
    (i.unit, usize::from(i.width.max(1)))
}

fn indent(ctx: &mut CommandCtx<'_>) -> CommandResult {
    let (unit, width) = doc_indent(ctx);
    // 選択範囲のインデント (v0.2、03 §15.3): 触れる各行の行頭に 1 段足す
    if has_selection(ctx) {
        let step = match unit {
            eve_config::IndentUnit::Tab => "\t".to_owned(),
            eve_config::IndentUnit::Space => " ".repeat(width),
        };
        return crate::command::line_ops::indent_lines(ctx, &step);
    }
    // 直前のカーソルの (行頭, 位置, 表示列)
    let last = std::cell::Cell::new(None::<(usize, usize, usize)>);
    edit_each(ctx, move |doc, r| {
        let text = doc.text();
        let line = text.byte_to_line(r.head);
        let ls = text.line_to_byte(line).0;
        let cfg = crate::layout::doc_width_config(doc);
        // Range は昇順なので、同じ行の前のカーソルの表示列から数え続ける
        let from = last
            .get()
            .filter(|&(l, _, _)| l == ls)
            .map(|(_, p, c)| (p, c));
        let col = display_col(doc, ls, r.head.0, from, &cfg);
        last.set(Some((ls, r.head.0, col)));
        let ins = match unit {
            eve_config::IndentUnit::Tab => "\t".to_owned(),
            eve_config::IndentUnit::Space => " ".repeat(width - col % width),
        };
        Some(Plan {
            del: ByteRange::new(r.head, r.head),
            ins: ins.into(),
            cursor: CursorRule::AfterInsert,
        })
    })
}

/// Shift+Tab: 行頭空白のうち、表示幅が `editor.indent_width` (文書ごとの値) 以内に収まる部分を削除する。
/// 選択があれば、触れる各行に同じ規則を適用する (v0.2、03 §15.3)。
fn outdent(ctx: &mut CommandCtx<'_>) -> CommandResult {
    let (_, width) = doc_indent(ctx);
    if has_selection(ctx) {
        return crate::command::line_ops::outdent_lines(ctx, move |doc, ls, le| {
            outdent_bytes(doc, ls, le, width)
        });
    }
    edit_each(ctx, move |doc, r| {
        let text = doc.text();
        let line = text.byte_to_line(r.head);
        let (ls, le) = (text.line_to_byte(line).0, text.line_end_byte(line).0);
        let bytes = outdent_bytes(doc, ls, le, width);
        (bytes > 0).then(|| Plan {
            del: ByteRange::new(ByteOff(ls), ByteOff(ls + bytes)),
            ins: "".into(),
            cursor: CursorRule::Keep,
        })
    })
}

/// 行 `[ls, le)` の行頭空白のうち、表示幅が `width` 以内に収まる部分のバイト数 (03 §15.3 の Shift+Tab)。
fn outdent_bytes(doc: &Document, ls: usize, le: usize, width: usize) -> usize {
    let tab = doc.display.tab_width.max(1) as usize;
    let lead = doc.text().slice_to_string(ByteRange::new(
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
        if next > width {
            // 先頭の 1 文字が幅を超えるタブなら、そのタブ 1 つを削除する (何も起きない状態を避ける)
            if bytes == 0 && ch == '\t' {
                bytes = 1;
            }
            break;
        }
        col = next;
        bytes += ch.len_utf8();
    }
    bytes
}

// ---------- 登録 ----------

/// v0.1 の編集コマンド (07 §11.6 の「編集」。カタログが正本)。
pub(crate) fn register(r: &mut CommandRegistry) {
    use CommandKind::{Edit, Motion};
    use Permission::{None as NoPerm, Propose, Read};
    let w = "!doc.read_only";
    r.add(ids::FILE_SET_LINE_ENDING, Edit, Propose, w, set_line_ending);
    r.add(
        ids::CLIPBOARD_COPY,
        CommandKind::Ui,
        NoPerm,
        "!selection.empty",
        crate::command::clipboard::copy,
    );
    r.add(
        ids::CLIPBOARD_CUT,
        Edit,
        NoPerm,
        "!selection.empty && !doc.read_only",
        clipboard_cut,
    );
    r.add(
        ids::EDIT_PASTE,
        Edit,
        NoPerm,
        w,
        crate::command::clipboard::paste,
    );
    r.add(ids::EDIT_INSERT_TEXT, Edit, Propose, w, insert_text);
    r.add(ids::EDIT_NEWLINE, Edit, Propose, w, newline);
    r.add(ids::EDIT_UNDO, Edit, NoPerm, w, undo);
    r.add(ids::EDIT_REDO, Edit, NoPerm, w, redo);
    r.add(ids::EDIT_DELETE_BACKWARD, Edit, Propose, w, delete_backward);
    r.add(ids::EDIT_DELETE_FORWARD, Edit, Propose, w, delete_forward);
    r.add(
        ids::EDIT_DELETE_WORD_BACKWARD,
        Edit,
        Propose,
        w,
        delete_word_backward,
    );
    r.add(
        ids::EDIT_DELETE_WORD_FORWARD,
        Edit,
        Propose,
        w,
        delete_word_forward,
    );
    r.add(ids::EDIT_INDENT, Edit, Propose, w, indent);
    r.add(ids::EDIT_OUTDENT, Edit, Propose, w, outdent);
    r.add(ids::SELECT_ALL, Motion, Read, "", select_all);
}
