//! テキスト変換のコマンド (docs/03 §15.8 の TXT-1〜TXT-7、07 §11.6 の「編集の拡張」、01 FR-214)。
//!
//! `edit.transform_upper` / `edit.transform_lower` (大文字・小文字)・`edit.sort_lines_ascending` /
//! `edit.sort_lines_descending` (行の並べ替え)・`edit.trim_trailing_whitespace` (行末の空白の除去)。どれも既定キーは無く、
//! パレットから使う。実行は `document::transform_cmd` (対象・同期とジョブ・Transaction と選択)。
//!
//! spec: 03 §15.8, 07 §11.6

use crate::command::line_ops::active;
use crate::command::{
    CommandCtx, CommandKind, CommandRegistry, CommandResult, Handler, Permission, RejectReason, ids,
};
use crate::document::transform::TransformOp;

fn run(ctx: &mut CommandCtx<'_>, op: TransformOp) -> CommandResult {
    let Some((view, doc)) = active(ctx) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    ctx.state.text_transform(view, doc, op)
}

fn upper(ctx: &mut CommandCtx<'_>) -> CommandResult {
    run(ctx, TransformOp::Upper)
}

fn lower(ctx: &mut CommandCtx<'_>) -> CommandResult {
    run(ctx, TransformOp::Lower)
}

fn sort_ascending(ctx: &mut CommandCtx<'_>) -> CommandResult {
    run(ctx, TransformOp::SortAscending)
}

fn sort_descending(ctx: &mut CommandCtx<'_>) -> CommandResult {
    run(ctx, TransformOp::SortDescending)
}

fn trim_trailing(ctx: &mut CommandCtx<'_>) -> CommandResult {
    run(ctx, TransformOp::TrimTrailing)
}

/// v0.5 のテキスト変換のコマンド (07 §11.6 の「編集の拡張」。種類は Edit、権限は propose、読み取り専用では実行できない)。
pub(crate) fn register(r: &mut CommandRegistry) {
    let commands: [(&str, Handler); 5] = [
        (ids::EDIT_TRANSFORM_UPPER, upper),
        (ids::EDIT_TRANSFORM_LOWER, lower),
        (ids::EDIT_SORT_LINES_ASCENDING, sort_ascending),
        (ids::EDIT_SORT_LINES_DESCENDING, sort_descending),
        (ids::EDIT_TRIM_TRAILING_WHITESPACE, trim_trailing),
    ];
    for (id, handler) in commands {
        r.add(
            id,
            CommandKind::Edit,
            Permission::Propose,
            "!doc.read_only",
            handler,
        );
    }
}
