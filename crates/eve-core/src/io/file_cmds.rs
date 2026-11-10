//! `file.*` のコマンドとその登録 (docs/07 §11.6) (STR-B1 で io/file.rs から分けた)。

use eve_view::MessageLevel;

use crate::command::ids;
use crate::io::fspath::FsPath;
use crate::io::io_jobs::OpenOptions;
use crate::ui_model::keys;
use eve_encoding::Encoding;

use crate::command::{
    CommandCtx, CommandKind, CommandRegistry, CommandResult, Permission, RejectReason,
};
use crate::editor::state::EditorState;
use crate::ui_model::overlay::PromptKind;

impl EditorState {
    /// 読み取り専用の切替 (`file.toggle_read_only`、03 §9.6)。解除できるのは `FileAttribute` と `UserRequested` だけ。
    /// 編集できる文書では `UserRequested` を付ける。
    pub(crate) fn toggle_read_only(&mut self) -> CommandResult {
        use crate::document::{LoadState, ReadOnlyReason};
        let Some(doc_id) = self.active_document() else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(doc) = self.docs.documents.get_mut(&doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        if doc.load_state != LoadState::Ready {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        let key = if doc.read_only.is_empty() {
            doc.read_only.insert(ReadOnlyReason::UserRequested);
            keys::INFO_READ_ONLY_SET
        } else if doc.read_only.releasable() {
            if doc.read_only.contains(ReadOnlyReason::FileAttribute) {
                doc.attribute_released = true;
            }
            doc.read_only.remove(ReadOnlyReason::FileAttribute);
            doc.read_only.remove(ReadOnlyReason::UserRequested);
            keys::INFO_READ_ONLY_RELEASED
        } else {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        };
        let path = doc.path.as_ref().map(FsPath::display).unwrap_or_default();
        self.message(MessageLevel::Info, key, &path);
        self.mark_dirty_render();
        CommandResult::Done
    }
}

pub(crate) fn arg_path(ctx: &CommandCtx<'_>) -> Option<String> {
    ctx.args.str("path").map(str::to_owned)
}

pub(crate) fn file_new(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.new_untitled()
}

pub(crate) fn file_open(ctx: &mut CommandCtx<'_>) -> CommandResult {
    match arg_path(ctx) {
        Some(p) => ctx.state.open_path(&p, OpenOptions::default()),
        // パスがなければ入力プロンプトで尋ねる
        None => {
            ctx.state.open_prompt(PromptKind::Open, String::new());
            CommandResult::Done
        }
    }
}

pub(crate) fn file_reopen_with_encoding(ctx: &mut CommandCtx<'_>) -> CommandResult {
    let enc = match ctx.args.str("encoding") {
        Some(name) => match Encoding::from_label(name) {
            Some(e) => Some(e),
            None => return CommandResult::Rejected(RejectReason::InvalidArgs),
        },
        None => None,
    };
    ctx.state.reopen_with_encoding(enc)
}

pub(crate) fn file_reload(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.reload()
}

pub(crate) fn file_toggle_read_only(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.toggle_read_only()
}

pub(crate) fn file_set_encoding(ctx: &mut CommandCtx<'_>) -> CommandResult {
    let enc = match ctx.args.str("encoding") {
        Some(name) => match Encoding::from_label(name) {
            Some(e) => Some(e),
            None => return CommandResult::Rejected(RejectReason::InvalidArgs),
        },
        None => None,
    };
    let bom = match ctx.args.get("bom") {
        Some(crate::command::ArgValue::Bool(b)) => Some(*b),
        _ => None,
    };
    ctx.state.set_encoding(enc, bom)
}

pub(crate) fn file_save(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.request_save()
}

pub(crate) fn file_save_as(ctx: &mut CommandCtx<'_>) -> CommandResult {
    match arg_path(ctx) {
        Some(p) => ctx.state.request_save_as(&p, None),
        None => {
            // 名前を尋ねる。初期値は今のパス
            let initial = ctx
                .state
                .active_document()
                .and_then(|d| ctx.state.document(d))
                .and_then(|d| d.path.as_ref().map(FsPath::display))
                .unwrap_or_default();
            ctx.state
                .open_prompt(PromptKind::SaveAs { then: None }, initial);
            CommandResult::Done
        }
    }
}

/// v0.1 のファイルコマンドのうち、確認 UI なしで動くもの (07 §11.6 の「ファイル・アプリ」)。
pub(crate) fn register(r: &mut CommandRegistry) {
    r.add(
        ids::FILE_NEW,
        CommandKind::File,
        Permission::None,
        "",
        file_new,
    );
    r.add(
        ids::FILE_OPEN,
        CommandKind::File,
        Permission::None,
        "",
        file_open,
    );
    r.add(
        ids::FILE_SAVE,
        CommandKind::File,
        Permission::None,
        "!doc.read_only",
        file_save,
    );
    r.add(
        ids::FILE_SAVE_AS,
        CommandKind::File,
        Permission::None,
        "",
        file_save_as,
    );
    r.add(
        ids::FILE_RELOAD,
        CommandKind::File,
        Permission::None,
        "doc.has_path",
        file_reload,
    );
    r.add(
        ids::FILE_SET_ENCODING,
        CommandKind::File,
        Permission::None,
        "",
        file_set_encoding,
    );
    r.add(
        ids::FILE_TOGGLE_READ_ONLY,
        CommandKind::File,
        Permission::None,
        "doc.read_only_releasable || !doc.read_only",
        file_toggle_read_only,
    );
    r.add(
        ids::FILE_REOPEN_WITH_ENCODING,
        CommandKind::File,
        Permission::None,
        "doc.has_path",
        file_reopen_with_encoding,
    );
}
