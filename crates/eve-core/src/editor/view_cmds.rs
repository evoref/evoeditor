//! View の表示属性のコマンド (docs/07 §11.6 の `view.*`、04 §7.2)。

use super::command::{
    CommandCtx, CommandKind, CommandRegistry, CommandResult, CommandSpec, Permission, RejectReason,
};
use super::layout::WrapMode;

/// `view.toggle_wrap`: 折り返しの ON/OFF。ON にするときの方式は設定 `view.wrap` (`off` なら `word`)。
fn toggle_wrap(ctx: &mut CommandCtx<'_>) -> CommandResult {
    let Some(w) = ctx.state.first_window() else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let Some(view) = ctx.state.window_view(w) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let mut s = ctx.state.config_wrap();
    if ctx.state.view(view).is_some_and(|v| v.wrap.is_on()) {
        s.mode = WrapMode::Off;
    } else if s.mode == WrapMode::Off {
        s.mode = WrapMode::Word;
    }
    ctx.state.set_wrap(view, s);
    // 折り返しを切り替えても、カーソルが見える位置にいるようにする
    ctx.state.ensure_cursor_visible(view);
    CommandResult::Done
}

pub(super) fn register(r: &mut CommandRegistry) {
    r.register(
        CommandSpec::new(
            "view.toggle_wrap",
            CommandKind::Ui,
            Permission::Ui,
            "",
            toggle_wrap,
        )
        .expect("カタログの when 句は構文が正しい"),
    );
}
