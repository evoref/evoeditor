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
    /// 読み取り専用の切替 (`file.toggle_read_only`、03 §9.6)。解除できるのは `FileAttribute`・`UserRequested` と、
    /// `pua_escape == Enabled` の文書の `InvalidBytes` (v0.4 の M23、03 §8.3.1 の「編集を許可」)。理由に
    /// `InvalidBytes` を含む文書は、外す前に確認 (`ui.confirm.read_only.invalid_bytes`) を 1 回出す。
    /// 編集できる文書では `UserRequested` を付ける (解除の記録は残す)。
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
        } else if doc.read_only_releasable() {
            if doc.read_only.contains(ReadOnlyReason::InvalidBytes) {
                self.confirm_release_invalid_bytes(doc_id);
                return CommandResult::Done;
            }
            release_reasons(doc);
            keys::INFO_READ_ONLY_RELEASED
        } else {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        };
        let path = doc.path.as_ref().map(FsPath::display).unwrap_or_default();
        self.message(MessageLevel::Info, key, &path);
        self.mark_dirty_render();
        CommandResult::Done
    }

    /// 不正バイトを含む文書の「編集を許可」の確認を出す (03 §8.3.1 の「確認」、04 §15.2 の
    /// `ui.confirm.read_only.invalid_bytes`)。本文は文書の名前・文字コード・不正なバイトの数・最初の不正なバイトの位置、
    /// `details` は要点の 4 行。選択肢は `[A] 編集を許可` (既定)・`[E] 文字コードを指定して開き直す`・キャンセル。
    fn confirm_release_invalid_bytes(&mut self, doc_id: eve_view::DocumentId) {
        use crate::ui_model::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
        let Some(doc) = self.docs.documents.get(&doc_id) else {
            return;
        };
        let name = doc
            .path
            .as_ref()
            .map(FsPath::display)
            .unwrap_or_else(|| self.untitled_label(doc));
        let has_path = doc.path.is_some();
        let encoding = doc.format.encoding.name().to_owned();
        // 数と最初の位置は、本文のエスケープ文字から求める (1 回の走査)。`invalid_bytes` は読込の時点の数で、復旧・
        // 編集の後の本文とは合わないため (03 §8.3.1 の細目 (e))
        let (first, count) = escape_summary(doc.text());
        let count = count.to_string();
        let mut args = vec![
            ("path".to_owned(), name),
            ("encoding".to_owned(), encoding),
            ("count".to_owned(), count),
        ];
        // 最初の不正なバイトが見つからなければ (全部消した後の復旧など) 位置を省く (〔 〕 ごと)
        if let Some(off) = first {
            let (line, col) = self.pos_of_offset(doc_id, off);
            args.push(("line".to_owned(), line.to_string()));
            args.push(("col".to_owned(), col.to_string()));
        }
        let details = [
            keys::UI_CONFIRM_READ_ONLY_INVALID_BYTES_DETAIL_RESTORE,
            keys::UI_CONFIRM_READ_ONLY_INVALID_BYTES_DETAIL_DELETE,
            keys::UI_CONFIRM_READ_ONLY_INVALID_BYTES_DETAIL_MERGE,
            keys::UI_CONFIRM_READ_ONLY_INVALID_BYTES_DETAIL_COPY,
        ]
        .iter()
        .map(|k| self.catalog().render(k, &[], &|_| None))
        .collect();
        let mut choices = vec![ChoiceDef {
            id: "release",
            accel: Some('a'),
            destructive: false,
        }];
        // 開き直しはパスのある文書だけ (無題の文書には `file.reopen_with_encoding` が使えない)
        if has_path {
            choices.push(ChoiceDef {
                id: "reopen",
                accel: Some('e'),
                destructive: false,
            });
        }
        self.open_confirm(ConfirmState {
            name: "read_only.invalid_bytes",
            args,
            choices,
            details,
            // 既定は「編集を許可」(比率が 1% を超えた文書でも変えない、OQ-217)
            default: 0,
            focused: 0,
            message: None,
            action: ConfirmAction::ReleaseInvalidBytes(doc_id),
        });
    }

    /// 「編集を許可」の確認の選択 (キャンセルは何も変えない)。`release` は `InvalidBytes`・`FileAttribute`・
    /// `UserRequested` を外して解除の記録を立てる。`reopen` は `file.reopen_with_encoding` (文字コードを尋ねる)。
    pub(crate) fn choose_release_invalid_bytes(
        &mut self,
        doc_id: eve_view::DocumentId,
        id: &str,
    ) -> CommandResult {
        // 開き直すのは確認を出した文書 (確定の時点の表示中の文書ではない、03 §9.8 の「入力・確認の対象」)
        if id == "reopen" {
            return self.reopen_with_encoding_for(doc_id, None);
        }
        // 確認を出している間に文書が変わっていれば (読込が始まった等)、外さない
        let Some(doc) = self
            .docs
            .documents
            .get_mut(&doc_id)
            .filter(|d| d.read_only_releasable())
        else {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        };
        release_reasons(doc);
        doc.invalid_bytes_released = true;
        self.show_message(
            MessageLevel::Info,
            keys::INFO_READ_ONLY_RELEASED_INVALID_BYTES,
            &[],
        );
        self.mark_dirty_render();
        CommandResult::Done
    }
}

/// 解除できる理由を外す (`FileAttribute` を外したら上書き保存の確認のために記録する、03 §9.6 の M9d の規則)。
fn release_reasons(doc: &mut crate::document::Document) {
    use crate::document::ReadOnlyReason;
    if doc.read_only.contains(ReadOnlyReason::FileAttribute) {
        doc.attribute_released = true;
    }
    doc.read_only.remove(ReadOnlyReason::FileAttribute);
    doc.read_only.remove(ReadOnlyReason::UserRequested);
    doc.read_only.remove(ReadOnlyReason::InvalidBytes);
}

/// 本文の最初のエスケープ文字 (`U+10FF00..=U+10FFFF`) のバイト位置と、エスケープ文字の数 (03 §8.3.1 の「確認」:
/// UTF-8 の `F4 8F BC`〜`F4 8F BF` を先頭から `memchr` で探す)。チャンクは char の境界で分かれるので、4 バイトの文字は
/// 1 つのチャンクの中にある。
fn escape_summary(text: &eve_text::Text) -> (Option<usize>, u64) {
    let mut it = escape_bytes(text);
    let first = it.next();
    let count = u64::from(first.is_some()) + it.count() as u64;
    (first, count)
}

/// 本文のエスケープ文字 (`U+10FF00..=U+10FFFF`、UTF-8 で `F4 8F BC`〜`F4 8F BF` で始まる) のバイト位置を先頭から順に。
fn escape_bytes(text: &eve_text::Text) -> impl Iterator<Item = usize> + '_ {
    use eve_text::{ByteOff, ByteRange, TextRead};
    let mut base = 0;
    text.chunks(ByteRange::new(ByteOff(0), ByteOff(text.len_bytes())))
        .flat_map(move |chunk| {
            let start = base;
            base += chunk.len();
            let b = chunk.as_bytes();
            memchr::memchr_iter(0xF4, b)
                .filter(move |&at| {
                    b.get(at + 1) == Some(&0x8F)
                        && b.get(at + 2).is_some_and(|x| (0xBC..=0xBF).contains(x))
                })
                .map(move |at| start + at)
        })
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
    let Some(doc) = ctx.state.active_document() else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    // 読込中 (`Loading` / 先行表示の `Partial`) の文書は名前を尋ねずに拒否する (中身が揃っていない。03 §9.2.1 の
    // 「できない操作」。名前の入力の後で黙って拒否しない)
    if ctx
        .state
        .document(doc)
        .is_some_and(|d| d.load_state != crate::document::LoadState::Ready)
    {
        ctx.state.notify_loading_busy();
        return CommandResult::Rejected(RejectReason::NotAvailable);
    }
    match arg_path(ctx) {
        Some(p) => ctx.state.request_save_as(doc, &p, None),
        None => {
            // 名前を尋ねる前に、今の文字コードで書けない文書を拒否する (03 §9.7 の「連鎖の開始時の拒否」)
            if ctx.state.reject_stateful_invalid(doc, None) {
                return CommandResult::Rejected(RejectReason::NotAvailable);
            }
            // 名前を尋ねる。初期値は今のパス。対象は今の文書に固定する (確定の時点の表示中の文書ではない、03 §9.8)
            let initial = ctx
                .state
                .document(doc)
                .and_then(|d| d.path.as_ref().map(FsPath::display))
                .unwrap_or_default();
            ctx.state
                .open_prompt(PromptKind::SaveAs { doc, then: None }, initial);
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
