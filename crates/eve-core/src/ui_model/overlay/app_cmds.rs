//! オーバーレイ・検索バーの中のコマンドと `app.quit` / `app.quit_with_error` / `app.save_and_quit` / `app.suspend` の登録 (07 §11.6)。

use eve_view::Effect;

use crate::command::ids;
use crate::command::{
    CommandCtx, CommandKind, CommandRegistry, CommandResult, Permission, RejectReason,
};

use super::Overlay;

fn cmd_confirm(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.ui_confirm()
}

fn cmd_cancel(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.ui_cancel()
}

fn cmd_list_next(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.ui_list_move(true)
}

fn cmd_list_prev(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.ui_list_move(false)
}

fn cmd_dialog_choose(ctx: &mut CommandCtx<'_>) -> CommandResult {
    match ctx.args.int("choice").and_then(|n| usize::try_from(n).ok()) {
        Some(n) => ctx.state.ui_dialog_choose(n),
        None => CommandResult::Rejected(RejectReason::InvalidArgs),
    }
}

fn cmd_search_open(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.search_open()
}

fn cmd_search_toggle_case(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.search_toggle_case()
}

fn cmd_search_toggle_regex(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.search_toggle_regex()
}

fn cmd_search_toggle_word(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.search_toggle_word()
}

fn cmd_search_toggle_selection(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.search_toggle_selection()
}

fn cmd_search_replace_open(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.search_replace_open()
}

fn cmd_search_replace(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.search_replace()
}

fn cmd_search_replace_all(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.search_replace_all()
}

fn cmd_search_next(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.search_move(true);
    CommandResult::Done
}

fn cmd_search_prev(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.search_move(false);
    CommandResult::Done
}

/// `search.next` / `search.prev` は Motion (実行すると Undo の結合を封印する、03 §7.3 R3)。
fn reg_motion_search(r: &mut CommandRegistry) {
    let handlers: [(&str, crate::Handler); 2] = [
        (ids::SEARCH_NEXT, cmd_search_next),
        (ids::SEARCH_PREV, cmd_search_prev),
    ];
    for (id, h) in handlers {
        r.add(
            id,
            CommandKind::Motion,
            Permission::Read,
            "search.active",
            h,
        );
    }
}

fn cmd_list_page_down(ctx: &mut CommandCtx<'_>) -> CommandResult {
    if matches!(ctx.state.ui.overlay, Some(Overlay::Help(_))) {
        return ctx.state.help_page(true);
    }
    ctx.state.palette_page(true)
}

fn cmd_list_page_up(ctx: &mut CommandCtx<'_>) -> CommandResult {
    if matches!(ctx.state.ui.overlay, Some(Overlay::Help(_))) {
        return ctx.state.help_page(false);
    }
    ctx.state.palette_page(false)
}

fn cmd_palette_open(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.open_palette()
}

fn cmd_redraw(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.push_effect(Effect::Redraw);
    CommandResult::Done
}

fn cmd_quit(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.request_quit()
}

fn cmd_quit_with_error(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.request_quit_with_error()
}

fn cmd_save_and_quit(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.request_save_and_quit()
}

fn cmd_suspend(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.push_effect(Effect::Suspend);
    CommandResult::Done
}

/// オーバーレイ内のコマンドと `app.quit`・`app.quit_with_error`・`app.save_and_quit`・`app.suspend` (07 §11.6)。
pub(crate) fn register(r: &mut CommandRegistry) {
    let active = "overlay.active != \"\" || search.active";
    r.add(
        ids::UI_CONFIRM,
        CommandKind::Ui,
        Permission::None,
        active,
        cmd_confirm,
    );
    r.add(
        ids::UI_CANCEL,
        CommandKind::Ui,
        Permission::None,
        "",
        cmd_cancel,
    );
    r.add(
        ids::UI_LIST_NEXT,
        CommandKind::Ui,
        Permission::None,
        active,
        cmd_list_next,
    );
    r.add(
        ids::UI_LIST_PREV,
        CommandKind::Ui,
        Permission::None,
        active,
        cmd_list_prev,
    );
    r.add(
        ids::UI_DIALOG_CHOOSE,
        CommandKind::Ui,
        Permission::None,
        "overlay.active == \"dialog\"",
        cmd_dialog_choose,
    );
    r.add(
        ids::UI_PALETTE_OPEN,
        CommandKind::Ui,
        Permission::None,
        "",
        cmd_palette_open,
    );
    r.add(
        ids::UI_REDRAW,
        CommandKind::Ui,
        Permission::None,
        "",
        cmd_redraw,
    );
    r.add(
        ids::UI_LIST_PAGE_DOWN,
        CommandKind::Ui,
        Permission::None,
        active,
        cmd_list_page_down,
    );
    r.add(
        ids::UI_LIST_PAGE_UP,
        CommandKind::Ui,
        Permission::None,
        active,
        cmd_list_page_up,
    );
    // 07 §11.6 の perm 列は `ui` (キーマップ以外からも呼べるが、外部クライアントには許可を要する)
    for (id, when, h) in [
        (ids::SEARCH_OPEN, "", cmd_search_open as crate::Handler),
        (
            ids::SEARCH_TOGGLE_CASE,
            "search.active",
            cmd_search_toggle_case,
        ),
        (
            ids::SEARCH_TOGGLE_REGEX,
            "search.active",
            cmd_search_toggle_regex,
        ),
        (
            ids::SEARCH_TOGGLE_WORD,
            "search.active",
            cmd_search_toggle_word,
        ),
        (
            ids::SEARCH_TOGGLE_SELECTION,
            "search.active",
            cmd_search_toggle_selection,
        ),
    ] {
        r.add(id, CommandKind::Ui, Permission::Ui, when, h);
    }
    reg_motion_search(r);
    // 置換 (03 §12.5): 置換欄を開く (Ui)、すべて置換 (Edit。承認を要する `propose`)
    r.add(
        ids::SEARCH_REPLACE_OPEN,
        CommandKind::Ui,
        Permission::Ui,
        "!doc.read_only",
        cmd_search_replace_open,
    );
    r.add(
        ids::SEARCH_REPLACE,
        CommandKind::Edit,
        Permission::Propose,
        "search.replace_active && !doc.read_only",
        cmd_search_replace,
    );
    r.add(
        ids::SEARCH_REPLACE_ALL,
        CommandKind::Edit,
        Permission::Propose,
        "search.replace_active && !doc.read_only",
        cmd_search_replace_all,
    );
    r.add(
        ids::APP_QUIT,
        CommandKind::Ui,
        Permission::None,
        "",
        cmd_quit,
    );
    r.add(
        ids::APP_QUIT_WITH_ERROR,
        CommandKind::Ui,
        Permission::None,
        "",
        cmd_quit_with_error,
    );
    r.add(
        ids::APP_SAVE_AND_QUIT,
        CommandKind::File,
        Permission::None,
        "",
        cmd_save_and_quit,
    );
    r.add(
        ids::APP_SUSPEND,
        CommandKind::Ui,
        Permission::None,
        "caps.can_suspend",
        cmd_suspend,
    );
    #[cfg(feature = "test-hooks")]
    if test_hook_panic_command() {
        r.add(
            "internal.debug_panic",
            CommandKind::Ui,
            Permission::None,
            "",
            cmd_debug_panic,
        );
    }
}

/// `EVE_TEST_HOOK` に `panic_command` があるか (09 §3.6。無ければ `internal.debug_panic` を登録しない)。
#[cfg(feature = "test-hooks")]
pub(crate) fn test_hook_panic_command() -> bool {
    std::env::var("EVE_TEST_HOOK").is_ok_and(|v| {
        v.split(',')
            .any(|h| h.split('=').next() == Some("panic_command"))
    })
}

/// パニックの注入 (NFR-302 AC-1)。
#[cfg(feature = "test-hooks")]
fn cmd_debug_panic(_ctx: &mut CommandCtx<'_>) -> CommandResult {
    panic!("internal.debug_panic: テストによるパニックの注入");
}
