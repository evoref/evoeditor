//! バッファ (複数文書) の操作: 切替と閉じる (docs/03 §9.8、07 §11.6 の `buffer.*`)。
//!
//! 文書の順は開いた順 (`DocumentId` の昇順)。閉じた後に表示する文書は、最近使った順で次の文書。

use eve_view::DocumentId;

use super::command::{
    CommandCtx, CommandKind, CommandRegistry, CommandResult, CommandSpec, Permission, RejectReason,
};
use super::overlay::Continuation;
use super::state::EditorState;

impl EditorState {
    /// 開いた順で `delta` 個先の文書へ切り替える (端で循環する)。文書が 1 件なら何もしない。
    fn switch_buffer(&mut self, delta: isize) -> CommandResult {
        // 置き換えの読込中は、読込中の文書がまだ集合に入っている。切り替えの対象にしない
        if self.open_in_progress() {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        let ids: Vec<DocumentId> = self.documents.keys().copied().collect();
        let Some(pos) = self
            .active_document()
            .and_then(|a| ids.iter().position(|d| *d == a))
        else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let next = (pos as isize + delta).rem_euclid(ids.len() as isize) as usize;
        self.activate_document(ids[next]);
        CommandResult::Done
    }

    /// 文書を閉じる要求。dirty なら FR-107 の確認を出す (保存を選んだら保存が成功したときだけ閉じる)。
    pub(super) fn request_close(&mut self, doc: DocumentId) -> CommandResult {
        if self.open_in_progress() {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        if self.confirm_doc_if_dirty(doc, "unsaved_close", Continuation::Close(doc)) {
            return CommandResult::Done;
        }
        self.close_buffer(doc)
    }

    /// 確認を済ませた (破棄または保存した) 後に文書を閉じる。表示中の文書なら、最近使った順で次の文書を表示する。
    /// 最後の 1 件なら無題の文書を新しく作る (終了はしない、03 §9.8)。
    pub(super) fn close_buffer(&mut self, doc: DocumentId) -> CommandResult {
        if self.active_document() == Some(doc) {
            let next = self
                .recent
                .iter()
                .rev()
                .copied()
                .find(|d| *d != doc && self.documents.contains_key(d))
                .or_else(|| self.documents.keys().copied().find(|d| *d != doc));
            match next {
                Some(n) => {
                    self.activate_document(n);
                    self.close_document(doc);
                }
                None => return self.new_untitled_forced(),
            }
        } else {
            self.close_document(doc);
        }
        self.recent.retain(|d| *d != doc);
        CommandResult::Done
    }
}

fn buffer_close(ctx: &mut CommandCtx<'_>) -> CommandResult {
    match ctx.state.active_document() {
        Some(doc) => ctx.state.request_close(doc),
        None => CommandResult::Rejected(RejectReason::NoTarget),
    }
}

fn buffer_next(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.switch_buffer(1)
}

fn buffer_prev(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.switch_buffer(-1)
}

/// v0.2 のバッファコマンド (07 §11.6 の `buffer.close` / `buffer.next` / `buffer.prev`)。
pub(super) fn register(r: &mut CommandRegistry) {
    let spec = |id, perm, h| {
        CommandSpec::new(id, CommandKind::Ui, perm, "", h)
            .expect("カタログの when 句は構文が正しい")
    };
    r.register(spec("buffer.close", Permission::None, buffer_close));
    r.register(spec("buffer.next", Permission::Ui, buffer_next));
    r.register(spec("buffer.prev", Permission::Ui, buffer_prev));
}
