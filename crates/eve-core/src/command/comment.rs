//! 行コメントの切替 `edit.toggle_comment` (docs/03 §15.6 の CMT-1〜CMT-7、01 FR-209)。
//!
//! 対象の行 (全 Range が触れる行の和集合。空行と空白だけの行は除く) がすべて言語の行コメントの記号で始まっていれば外し、
//! そうでなければ、対象の行の最も浅い字下げの位置 (表示の列。タブを割らない) に記号と空白 1 つを付ける。全部の行の編集を
//! 1 つの Transaction (Undo 1 回) にする。
//!
//! spec: 03 §15.6, 03 §4.5.2

use eve_text::{ByteOff, ByteRange, Edit, LineIdx, Selection, TextRead};
use eve_view::MessageLevel;

use crate::command::line_ops::{active, commit, heads_at_line_start, map_range, target_blocks};
use crate::command::{
    CommandCtx, CommandKind, CommandRegistry, CommandResult, Permission, RejectReason, ids,
};
use crate::document::Document;
use crate::ui_model::keys;
use crate::view::motion::indent_end;

/// 対象の行 1 つ: 行頭のバイト位置と、行頭の空白の末尾 (改行の前の、空白でない最初の文字の位置)。
#[derive(Clone, Copy, Debug)]
struct TargetLine {
    start: usize,
    indent_end: usize,
}

/// 対象の行 (CMT-1)。空行と空白 (U+0020 とタブ) だけの行は飛ばす。同じ行は 1 回だけ (ブロックは重ならない)。
fn target_lines(doc: &Document, sel: &Selection) -> Vec<TargetLine> {
    let t = doc.text();
    let mut out = Vec::new();
    for (first, last) in target_blocks(doc, sel) {
        for l in first..=last {
            let start = t.line_to_byte(LineIdx(l)).0;
            let end = t.line_end_byte(LineIdx(l)).0;
            let indent_end = indent_end(doc, start, end);
            if indent_end < end {
                out.push(TargetLine { start, indent_end });
            }
        }
    }
    out
}

/// `[from, from + len)` の本文が `s` と同じか (行の全体を読まない。CMT-7)。
fn starts_with_at(t: &impl TextRead, from: usize, s: &str) -> bool {
    let end = from + s.len();
    end <= t.len_bytes()
        && t.is_char_boundary(ByteOff(end))
        && t.slice_to_string(ByteRange::new(ByteOff(from), ByteOff(end))) == s
}

/// 行頭の空白を表示の列で数え、幅が初めて `m` 以上になるバイト位置 (CMT-3)。行頭の空白の幅は `m` 以上であること。
/// タブは `tab_width` の次の停止位置まで進めるので、`m` がタブの途中に落ちればそのタブの後ろになる。
fn insert_pos(t: &impl TextRead, line: TargetLine, m: usize, tab_width: usize) -> usize {
    let (mut col, mut pos) = (0usize, line.start);
    let range = ByteRange::new(ByteOff(line.start), ByteOff(line.indent_end));
    'outer: for chunk in t.chunks(range) {
        for b in chunk.bytes() {
            if col >= m {
                break 'outer;
            }
            col = next_col(col, b, tab_width);
            pos += 1;
        }
    }
    pos
}

/// 行頭の空白の 1 バイト (`' '` か `'\t'`) の後の表示の列。
fn next_col(col: usize, b: u8, tab_width: usize) -> usize {
    if b == b'\t' {
        col + tab_width - col % tab_width
    } else {
        col + 1
    }
}

/// 行頭の空白の表示の幅。
fn indent_width(t: &impl TextRead, line: TargetLine, tab_width: usize) -> usize {
    let range = ByteRange::new(ByteOff(line.start), ByteOff(line.indent_end));
    t.chunks(range)
        .flat_map(str::bytes)
        .fold(0, |col, b| next_col(col, b, tab_width))
}

/// 付ける・外す編集 (CMT-2〜CMT-4)。`lines` は空でないこと。
fn plan_edits(doc: &Document, lines: &[TargetLine], token: &str) -> Vec<Edit> {
    let t = doc.text();
    let commented = lines.iter().all(|l| starts_with_at(t, l.indent_end, token));
    if commented {
        // 外す: 記号と、そのすぐ後の空白 1 つ (タブと 2 つ目の空白は残す)
        return lines
            .iter()
            .map(|l| {
                let mut end = l.indent_end + token.len();
                if starts_with_at(t, end, " ") {
                    end += 1;
                }
                Edit {
                    range: ByteRange::new(ByteOff(l.indent_end), ByteOff(end)),
                    insert: "".into(),
                }
            })
            .collect();
    }
    let tab_width = usize::from(doc.display.tab_width.max(1));
    let m = lines
        .iter()
        .map(|&l| indent_width(t, l, tab_width))
        .min()
        .unwrap_or(0);
    // 全部の行で同じ文字列を共有する (行ごとに確保しない)
    let insert: std::sync::Arc<str> = format!("{token} ").into();
    lines
        .iter()
        .map(|&l| {
            let p = ByteOff(insert_pos(t, l, m, tab_width));
            Edit {
                range: ByteRange::new(p, p),
                insert: std::sync::Arc::clone(&insert),
            }
        })
        .collect()
}

/// 言語の表示の名前 (`info.comment.unsupported` の `{language}`)。言語が決まっていなければ `plain`。
fn language_display(ctx: &CommandCtx<'_>, doc: &Document) -> String {
    let name = match ctx.state.language_of(doc) {
        Some(l) => l.display_name(),
        None => doc
            .language
            .id
            .as_deref()
            .filter(|id| !id.is_empty())
            .unwrap_or(eve_config::PLAIN),
    };
    eve_config::sanitize_untrusted(name)
}

fn toggle_comment(ctx: &mut CommandCtx<'_>) -> CommandResult {
    let Some((view_id, doc_id)) = active(ctx) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let (Some(doc), Some(view)) = (ctx.state.document(doc_id), ctx.state.view(view_id)) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    // 記号の無い言語 (plain を含む) は文書を変えずに知らせる (CMT-6)
    let Some(token) = ctx
        .state
        .language_of(doc)
        .and_then(|l| l.comment_line.as_ref())
        .map(|(c, _)| c.clone())
    else {
        let language = language_display(ctx, doc);
        ctx.state.show_message(
            MessageLevel::Info,
            keys::INFO_COMMENT_UNSUPPORTED,
            &[("language", &language)],
        );
        return CommandResult::Done;
    };
    let sel = view.selection.clone();
    let lines = target_lines(doc, &sel);
    if lines.is_empty() {
        ctx.state
            .show_message(MessageLevel::Info, keys::INFO_COMMENT_NO_LINES, &[]);
        return CommandResult::Done;
    }
    let edits = plan_edits(doc, &lines, &token);
    // 写し方は §15.4 のインデントの操作と同じ (CMT-5)
    let at_line_start = heads_at_line_start(doc.text(), &sel);
    commit(ctx, ids::EDIT_TOGGLE_COMMENT, edits, move |_, cs| {
        sel.iter()
            .zip(&at_line_start)
            .map(|(r, &s)| map_range(r, s, cs))
            .collect()
    })
}

/// v0.5 の行コメントのコマンド (07 §11.6 の「設定と言語」)。
pub(crate) fn register(r: &mut CommandRegistry) {
    r.add(
        ids::EDIT_TOGGLE_COMMENT,
        CommandKind::Edit,
        Permission::Propose,
        "!doc.read_only",
        toggle_comment,
    );
}
