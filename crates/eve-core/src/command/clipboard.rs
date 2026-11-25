//! クリップボード (docs/03 §15.3 / 02 §6.3 / 05 §8)。内部レジスタと、`Effect::SetClipboard` / `Effect::RequestClipboard`。
//!
//! コピーは常に内部レジスタへ書き (Snapshot と範囲を持つだけ。O(1))、システムクリップボードへは Effect で依頼する。
//! 64 KiB 未満は文字列にして `Inline`、それ以上はハンドルで渡し、Host が `take_clipboard` で取り出してワーカーで文字列化する。
//! 貼り付けは `RequestClipboard` で読み取りを要求し、応答 (`ClipboardContent`) が届いた時点の Selection に入れる。
//! 複数カーソルのコピーは、空でない Range ごとの片を内部レジスタに持つ (03 §4.5.3 の MCUR-8)。矩形のコピーは、矩形の行ごとの
//! 片を持つ (03 §4.8 の BOX-4)。
//!
//! 不正バイトのエスケープ文字 (v0.4 の M23、03 §8.3.1 の「本文に入れる規則」): 内部レジスタはエスケープ文字のまま持ち、
//! コピー元の `notation` と `escape_origin` を覚える。システムのクリップボードへ渡す文字列は `ClipboardSource::to_text`
//! (表記 `<0xNN>`)。貼り付けで届いた文字列と内部レジスタの照合は `command::insert_rule`。

use std::collections::BTreeMap;

/// システムのクリップボードを使えず内部レジスタに切り替えた理由 (`msg.clipboard.fallback` の `{reason}`)。文言は
/// `ui.clipboard_reason.<key>` (04 §15.1)。Host・eve-platform は文字列ではなくこの値を渡す (文言は core が言語に合わせて作る)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClipboardFallbackReason {
    /// 読み取りに失敗した。
    ReadFailed,
    /// 書込に失敗した。
    WriteFailed,
    /// 大きすぎてシステムのクリップボードに入らない (64 MiB)。
    TooLarge,
    /// 読み取りが期限 (2 秒) までに終わらなかった。
    Timeout,
    /// SSH 接続中は外部のコマンドを使わない (Linux)。
    Ssh,
    /// 画面 (`DISPLAY` / `WAYLAND_DISPLAY`) が無い (Linux)。
    NoDisplay,
    /// 使えるコマンド (wl-clipboard / xclip) が無い (Linux)。
    NoCommand,
}

impl ClipboardFallbackReason {
    /// 文言キーの末尾 (`ui.clipboard_reason.<key>`)。
    pub fn key(self) -> &'static str {
        match self {
            ClipboardFallbackReason::ReadFailed => "read_failed",
            ClipboardFallbackReason::WriteFailed => "write_failed",
            ClipboardFallbackReason::TooLarge => "too_large",
            ClipboardFallbackReason::Timeout => "timeout",
            ClipboardFallbackReason::Ssh => "ssh",
            ClipboardFallbackReason::NoDisplay => "no_display",
            ClipboardFallbackReason::NoCommand => "no_command",
        }
    }
}

use eve_text::{ByteRange, LineEnding, Snapshot, TextRead};
use eve_view::{
    ClipboardHandleId, ClipboardTarget, ClipboardText, ClipboardToken, Effect, MessageLevel,
};

use crate::command::{CommandCtx, CommandResult, RejectReason};
use crate::editor::state::EditorState;
use crate::ui_model::keys;

/// `Inline` にする上限 (これ未満は core が文字列化する)。
pub const INLINE_MAX_BYTES: usize = 64 * 1024;

/// クリップボードの中身 (Snapshot と片の範囲の並び) と、コピーした時点の文書の状態 (03 §8.3.1 の「コピー」)。`Send`。
/// 文字列化は Host がワーカーで行う (`to_text`)。
///
/// 複数カーソルのコピー (03 §4.5.3 の MCUR-8) は、空でない Range ごとの片を文書の順に持ち、文字列にするときは片の間に
/// `joiner` (コピー元の文書の既定の改行) を入れる (末尾には足さない)。片が 1 つなら v0.4 までと同じ。
#[derive(Clone, Debug)]
pub struct ClipboardSource {
    pub snapshot: Snapshot,
    /// 片の範囲 (文書の順で重ならない。1 つ以上)。
    pieces: Vec<ByteRange>,
    /// 片の間に入れる改行。
    joiner: LineEnding,
    /// コピー元が `pua_escape != DisabledCollision` だった (エスケープ文字を表記 `<0xNN>` にして文字列化する)。
    /// 偽は衝突の文書で、私用文字をそのまま渡す。文書の外の文字列 (メッセージ履歴など) も偽。
    pub notation: bool,
    /// コピー元の `escape_encoding` (本文のエスケープ文字がどの文字コードのバイトか)。貼り付け先が同じ `Some` のときだけ、
    /// エスケープ文字のまま (バイトのまま) 運ぶ。
    pub escape_origin: Option<crate::Encoding>,
    /// コピー元が衝突の文書 (b) (`DisabledCollision` で不正バイトもある) だった。`U+10FF00..` が不正バイトか真の私用文字かを
    /// 区別できないので、貼り付けではどの文書へも (衝突の文書 (a) を含む) `<U+10FFNN>` の文字列にする (03 §8.3.1)。
    pub indistinct: bool,
}

impl ClipboardSource {
    /// 片 1 つの中身 (文書の外の文字列や、1 つの Range のコピー)。
    pub(crate) fn single(snapshot: Snapshot, range: ByteRange) -> ClipboardSource {
        ClipboardSource {
            snapshot,
            pieces: vec![range],
            joiner: LineEnding::Lf,
            notation: false,
            escape_origin: None,
            indistinct: false,
        }
    }

    /// 片の並び (文書の順) と、連結に使う改行を入れた中身。`pieces` が空なら `None`。
    pub(crate) fn with_pieces(
        snapshot: Snapshot,
        pieces: Vec<ByteRange>,
        joiner: LineEnding,
    ) -> Option<ClipboardSource> {
        (!pieces.is_empty()).then_some(ClipboardSource {
            snapshot,
            pieces,
            joiner,
            notation: false,
            escape_origin: None,
            indistinct: false,
        })
    }

    /// 片の範囲 (文書の順)。
    pub fn pieces(&self) -> &[ByteRange] {
        &self.pieces
    }

    /// 片の間に入れる改行。
    pub fn joiner(&self) -> LineEnding {
        self.joiner
    }

    /// 片の合計のバイト数に、連結の改行を足したもの (文字列化の前に上限と比べる。表記にすると長くなる分は数えない、
    /// 05 §8.1。03 §4.5.3 の MCUR-8)。
    pub fn len_bytes(&self) -> usize {
        let body: usize = self.pieces.iter().map(|r| r.end.0 - r.start.0).sum();
        body + self.joiner.byte_len() * (self.pieces.len() - 1)
    }

    /// システムのクリップボードへ渡す文字列 (`Inline`・Host が `Handle` を文字列化するとき・OSC 52 で共通、02 §5.4)。
    /// `notation` が真ならエスケープ文字を表記 `<0xNN>` にする (表示と同じ。03 §8.3.1)。片の中の改行は変えず、片の間に
    /// `joiner` を入れる。
    pub fn to_text(&self) -> String {
        let mut out = String::with_capacity(self.len_bytes());
        for (i, &range) in self.pieces.iter().enumerate() {
            if i > 0 {
                out.push_str(self.joiner.as_str());
            }
            self.push_piece_text(&mut out, range);
        }
        out
    }

    /// 片 `i` を `to_text` と同じ規則で文字列にしたもの。
    pub(crate) fn piece_text(&self, i: usize) -> String {
        let mut out = String::new();
        if let Some(&range) = self.pieces.get(i) {
            self.push_piece_text(&mut out, range);
        }
        out
    }

    /// 片 `i` の中身 (エスケープ文字のまま)。
    pub(crate) fn piece_raw(&self, i: usize) -> String {
        self.pieces
            .get(i)
            .map(|&r| self.snapshot.slice_to_string(r))
            .unwrap_or_default()
    }

    /// 内部レジスタの中身 (エスケープ文字のまま。片の間に `joiner` を入れる)。
    pub(crate) fn raw_text(&self) -> String {
        let mut out = String::with_capacity(self.len_bytes());
        for i in 0..self.pieces.len() {
            if i > 0 {
                out.push_str(self.joiner.as_str());
            }
            out.push_str(&self.piece_raw(i));
        }
        out
    }

    fn push_piece_text(&self, out: &mut String, range: ByteRange) {
        if !self.notation {
            out.push_str(&self.snapshot.slice_to_string(range));
            return;
        }
        for chunk in self.snapshot.chunks(range) {
            crate::command::insert_rule::push_notation(out, chunk);
        }
    }
}

/// 内部レジスタ・ハンドルの表・貼り付け要求の状態。
#[derive(Debug, Default)]
pub(crate) struct ClipboardState {
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
    pub(crate) fn active_document_editable(&self) -> bool {
        self.active_document()
            .and_then(|d| self.docs.documents.get(&d))
            .is_some_and(|d| d.is_editable())
    }

    /// システムクリップボードを使えなかったことを知らせる (`msg.clipboard.fallback`、同じ起動の中で最初の 1 回だけ)。
    pub fn notify_clipboard_fallback(&mut self, reason: ClipboardFallbackReason) {
        if self.clipboard.fallback_notified {
            return;
        }
        self.clipboard.fallback_notified = true;
        let reason = self.catalog().render(
            &format!("ui.clipboard_reason.{}", reason.key()),
            &[],
            &|_| None,
        );
        self.show_message(
            MessageLevel::Info,
            keys::MSG_CLIPBOARD_FALLBACK,
            &[("reason", &reason)],
        );
    }

    /// 選択をコピーする。空でない Range が無ければ何もしない (`false`)。内部レジスタへ書き、`SetClipboard` を積む。
    /// 空でない Range を文書の順に片にし、文書の既定の改行で連結する (03 §4.5.3 の MCUR-8。空のカーソルは数えない)。
    /// 矩形があれば、矩形の行ごとに 1 つの片にする (Range の無い短い行は空の片。03 §4.8 の BOX-4)。
    pub(crate) fn copy_selection(&mut self) -> bool {
        let Some(doc_id) = self.active_document() else {
            return false;
        };
        let Some(view_id) = self.focused_window().and_then(|w| self.window_view(w)) else {
            return false;
        };
        let Some(view) = self.docs.views.get(&view_id) else {
            return false;
        };
        let pieces: Vec<ByteRange> = match self.box_copy_pieces(view_id) {
            Some(pieces) => pieces,
            None => view
                .selection
                .iter()
                .filter(|r| !r.is_empty())
                .map(|r| ByteRange::new(r.start(), r.end()))
                .collect(),
        };
        let Some(doc) = self.docs.documents.get(&doc_id) else {
            return false;
        };
        let Some(mut source) =
            ClipboardSource::with_pieces(doc.snapshot(), pieces, doc.format.default_eol)
        else {
            return false;
        };
        // コピーした時点の文書の状態を覚える (03 §8.3.1 の「コピー」)
        let ctx = crate::command::insert_rule::EscapeContext::of(&doc.format);
        source.notation = ctx.notation;
        source.escape_origin = ctx.escape_encoding;
        // 衝突の文書 (b): 不正バイトと真の私用文字の両方があり、区別できない
        source.indistinct = doc.format.pua_escape == eve_encoding::PuaEscapeMode::DisabledCollision
            && doc.format.invalid_bytes > 0;
        self.put_clipboard(source);
        true
    }

    /// 文字列をコピーする (文書の外の文字列。メッセージ履歴など)。内部レジスタへ書き、`SetClipboard` を積む。
    pub(crate) fn copy_text(&mut self, text: &str) {
        let snapshot = eve_text::Text::from(text).snapshot();
        let range = ByteRange::new(eve_text::ByteOff(0), eve_text::ByteOff(text.len()));
        // 文書の外の文字列: 私用文字に不正バイトの意味は無い (貼り付け先では外からの文字列と同じに扱う)
        self.put_clipboard(ClipboardSource::single(snapshot, range));
    }

    /// 内部レジスタへ書き、システムへの書込を依頼する。64 KiB 未満は文字列、それ以上はハンドルで渡す。
    fn put_clipboard(&mut self, source: ClipboardSource) {
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
    }

    /// Host が `Effect::SetClipboard` の `Handle` を実行するときに中身を取り出す (02 §4.4)。
    pub fn take_clipboard(&mut self, id: ClipboardHandleId) -> Option<ClipboardSource> {
        self.clipboard.handles.remove(&id)
    }

    /// `edit.paste`: システムクリップボードの読み取りを要求する。応答は `on_clipboard_content`。
    pub(crate) fn request_paste(&mut self) -> CommandResult {
        self.clipboard.next_token += 1;
        let token = ClipboardToken(self.clipboard.next_token);
        self.clipboard.token = Some(token);
        self.push_effect(Effect::RequestClipboard {
            token,
            target: ClipboardTarget::System,
        });
        CommandResult::Done
    }

    /// 待っている貼り付けの応答の token か。
    pub(crate) fn clipboard_waiting(&self, token: ClipboardToken) -> bool {
        self.clipboard.token == Some(token)
    }

    /// `InputEvent::ClipboardContent` の応答。最後の要求の token と違えば捨てる。届いた時点の Selection に貼り付ける。
    /// 読み取れなかった (`None`) ときは内部レジスタの内容を貼り付ける (05 §8.1)。本文へは内部レジスタと照合して入れる
    /// (03 §8.3.1。`edit::paste_text`)。
    pub fn on_clipboard_content(&mut self, token: ClipboardToken, text: Option<String>) {
        if self.clipboard.token != Some(token) {
            return;
        }
        self.clipboard.token = None;
        // 入力欄 (プロンプト・パレット・検索バー) が出ていれば、そこへ最初の行だけを貼り付ける (04 §6.6)。
        // 入力欄は文書ではないので照合せず、届いた文字列のまま入れる (03 §8.3.1 の「入力欄」)
        if self.input_mut_available() {
            let t = match text {
                Some(t) => t,
                None => {
                    self.notify_clipboard_fallback(ClipboardFallbackReason::ReadFailed);
                    match &self.clipboard.register {
                        Some(src) => src.to_text(),
                        None => return,
                    }
                }
            };
            let line = t.lines().next().unwrap_or("").to_owned();
            self.prompt_paste(&line);
            self.notify_field_paste(&t);
            return;
        }
        let editable = self
            .active_document()
            .and_then(|d| self.docs.documents.get(&d))
            .is_some_and(|d| d.is_editable());
        if !editable {
            self.notify_read_only();
            return;
        }
        if text.is_none() {
            self.notify_clipboard_fallback(ClipboardFallbackReason::ReadFailed);
        }
        crate::command::edit::paste_text(self, text.as_deref());
    }

    /// 内部レジスタ (貼り付けの照合に使う、03 §8.3.1)。
    pub(crate) fn clipboard_register(&self) -> Option<&ClipboardSource> {
        self.clipboard.register.as_ref()
    }

    /// 検索欄への貼り付けが、内部レジスタ由来の `<0xNN>` を含むときに知らせる (`info.paste.search_notation`、
    /// 03 §8.3.1 の「入力欄」・04 §15.2)。欄には届いた文字列のまま入っている。文言が検索の説明なので、置換欄・
    /// プロンプト・パレットの欄では出さない。
    pub(crate) fn notify_field_paste(&mut self, arrived: &str) {
        let in_search_field = self.ui.overlay.is_none()
            && self
                .find
                .search_bar
                .as_ref()
                .is_some_and(|b| b.focus == eve_view::BarFocus::Search);
        if !in_search_field {
            return;
        }
        let from_register = self
            .clipboard
            .register
            .as_ref()
            .is_some_and(|r| r.notation && r.escape_count() > 0 && r.matches_text(arrived));
        if from_register {
            self.show_message(MessageLevel::Info, keys::INFO_PASTE_SEARCH_NOTATION, &[]);
        }
    }
}

pub(crate) fn copy(ctx: &mut CommandCtx<'_>) -> CommandResult {
    // 選択が空なら何もしない (通知もしない、03 §15.3)
    ctx.state.copy_selection();
    CommandResult::Done
}

pub(crate) fn paste(ctx: &mut CommandCtx<'_>) -> CommandResult {
    if !ctx.state.active_document_editable() {
        return CommandResult::Rejected(RejectReason::ReadOnly);
    }
    ctx.state.request_paste()
}
