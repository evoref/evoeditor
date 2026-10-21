//! クリップボード (docs/03 §15.3 / 02 §6.3 / 05 §8)。内部レジスタと、`Effect::SetClipboard` / `Effect::RequestClipboard`。
//!
//! コピーは常に内部レジスタへ書き (Snapshot と範囲を持つだけ。O(1))、システムクリップボードへは Effect で依頼する。
//! 64 KiB 未満は文字列にして `Inline`、それ以上はハンドルで渡し、Host が `take_clipboard` で取り出してワーカーで文字列化する。
//! 貼り付けは `RequestClipboard` で読み取りを要求し、応答 (`ClipboardContent`) が届いた時点の Selection に入れる。

use std::collections::BTreeMap;

use eve_text::{ByteRange, Snapshot, TextRead};
use eve_view::{
    ClipboardHandleId, ClipboardTarget, ClipboardText, ClipboardToken, Effect, MessageLevel,
};

use super::command::{CommandCtx, CommandResult, RejectReason};
use super::state::EditorState;

/// `Inline` にする上限 (これ未満は core が文字列化する)。
pub const INLINE_MAX_BYTES: usize = 64 * 1024;

/// クリップボードの中身 (Snapshot と範囲)。`Send`。文字列化は Host がワーカーで行う。
#[derive(Clone, Debug)]
pub struct ClipboardSource {
    pub snapshot: Snapshot,
    pub range: ByteRange,
}

impl ClipboardSource {
    /// 範囲の総バイト数 (文字列化の前に上限と比べる)。
    pub fn len_bytes(&self) -> usize {
        self.range.end.0 - self.range.start.0
    }

    pub fn to_text(&self) -> String {
        self.snapshot.slice_to_string(self.range)
    }
}

/// 内部レジスタ・ハンドルの表・貼り付け要求の状態。
#[derive(Debug, Default)]
pub(super) struct ClipboardState {
    register: Option<ClipboardSource>,
    handles: BTreeMap<ClipboardHandleId, ClipboardSource>,
    next_handle: u64,
    /// 最後に出した `RequestClipboard` の token。これと違う応答は捨てる。
    token: Option<ClipboardToken>,
    next_token: u64,
    /// `msg.clipboard.fallback` を出した (同じ起動の中で最初の 1 回だけ)。
    fallback_notified: bool,
}

impl EditorState {
    /// 今の文書が編集できるか。
    pub(super) fn active_document_editable(&self) -> bool {
        self.active_document()
            .and_then(|d| self.documents.get(&d))
            .is_some_and(|d| d.is_editable())
    }

    /// 選択をコピーする。選択が空なら何もしない (`false`)。内部レジスタへ書き、`SetClipboard` を積む。
    pub(super) fn copy_selection(&mut self) -> bool {
        let Some(doc_id) = self.active_document() else {
            return false;
        };
        let Some(view) = self
            .first_window()
            .and_then(|w| self.window_view(w))
            .and_then(|v| self.views.get(&v))
        else {
            return false;
        };
        let r = view.selection.primary();
        if r.is_empty() {
            return false;
        }
        let range = ByteRange::new(r.start(), r.end());
        let Some(doc) = self.documents.get(&doc_id) else {
            return false;
        };
        let source = ClipboardSource {
            snapshot: doc.snapshot(),
            range,
        };
        let content = if source.len_bytes() < INLINE_MAX_BYTES {
            ClipboardText::Inline(source.to_text())
        } else {
            // 古い未回収のハンドルは捨てる (Host が回収していない間に積み上げない)
            self.clipboard.handles.clear();
            self.clipboard.next_handle += 1;
            let id = ClipboardHandleId(self.clipboard.next_handle);
            self.clipboard.handles.insert(id, source.clone());
            ClipboardText::Handle(id)
        };
        self.clipboard.register = Some(source);
        self.push_effect(Effect::SetClipboard {
            content,
            target: ClipboardTarget::System,
        });
        true
    }

    /// Host が `Effect::SetClipboard` の `Handle` を実行するときに中身を取り出す (02 §4.4)。
    pub fn take_clipboard(&mut self, id: ClipboardHandleId) -> Option<ClipboardSource> {
        self.clipboard.handles.remove(&id)
    }

    /// `edit.paste`: システムクリップボードの読み取りを要求する。応答は `on_clipboard_content`。
    pub(super) fn request_paste(&mut self) -> CommandResult {
        self.clipboard.next_token += 1;
        let token = ClipboardToken(self.clipboard.next_token);
        self.clipboard.token = Some(token);
        self.push_effect(Effect::RequestClipboard {
            token,
            target: ClipboardTarget::System,
        });
        CommandResult::Done
    }

    /// `InputEvent::ClipboardContent` の応答。最後の要求の token と違えば捨てる。届いた時点の Selection に貼り付ける。
    /// 読み取れなかった (`None`) ときは内部レジスタの内容を貼り付ける (05 §8.1)。
    pub fn on_clipboard_content(&mut self, token: ClipboardToken, text: Option<String>) {
        if self.clipboard.token != Some(token) {
            return;
        }
        self.clipboard.token = None;
        let editable = self
            .active_document()
            .and_then(|d| self.documents.get(&d))
            .is_some_and(|d| d.is_editable());
        if !editable {
            self.show_message(MessageLevel::Warn, "msg.edit.read_only", &[]);
            return;
        }
        let text = match text {
            Some(t) => t,
            None => {
                if !self.clipboard.fallback_notified {
                    self.clipboard.fallback_notified = true;
                    self.show_message(
                        MessageLevel::Info,
                        "msg.clipboard.fallback",
                        &[("reason", "取得できませんでした")],
                    );
                }
                match &self.clipboard.register {
                    Some(src) => src.to_text(),
                    None => return,
                }
            }
        };
        super::edit::paste_text(self, &text);
    }
}

pub(super) fn copy(ctx: &mut CommandCtx<'_>) -> CommandResult {
    // 選択が空なら何もしない (通知もしない、03 §15.3)
    ctx.state.copy_selection();
    CommandResult::Done
}

pub(super) fn paste(ctx: &mut CommandCtx<'_>) -> CommandResult {
    if !ctx.state.active_document_editable() {
        return CommandResult::Rejected(RejectReason::ReadOnly);
    }
    ctx.state.request_paste()
}
