//! 未保存の変更の確認と、終了の連鎖 (FR-107、docs/03 §9.7、docs/04 §8)。
//!
//! 確認ダイアログの選択肢と、選んだ後に続ける操作 (`Continuation`)、終了するときに未保存の文書を順に確認する連鎖。

use eve_view::{DocumentId, Effect, MessageLevel, QuitRescue, Revision};

use crate::command::{CommandResult, RejectReason};
use crate::ui_model::keys;

use crate::editor::state::EditorState;

use super::Overlay;

/// `app.quit_with_error` の終了コード (07 §18.4)。
const EXIT_QUIT_WITH_ERROR: u8 = 4;

/// 保存や破棄の後に続ける操作 (03 §9.7 の連鎖の後続)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Continuation {
    /// 終了する。
    Quit,
    /// 文書を文字コードを指定して開き直す (`file.reopen_with_encoding`、未保存の確認の後)。
    /// 対象の文書を持つ (保存の完了の時点でアクティブな文書とは限らない、03 §9.7)。
    Reopen(DocumentId, eve_encoding::Encoding),
    /// 文書を閉じる (`buffer.close`、03 §9.2 の閉じる手順と共通)。
    Close(DocumentId),
}

/// 終了の連鎖 (`app.quit` で dirty な文書が 1 件以上、03 §9.8)。保存の連鎖 (`SaveChain`) の外に 1 つだけ持つ。
/// 破棄は印を付けるだけで、文書の内容は終了が確定するまで残す。
#[derive(Debug)]
pub(crate) struct QuitChain {
    /// 未確認の文書。先頭が確認中。
    remaining: Vec<DocumentId>,
    /// 破棄を選んだ文書と、そのときの Revision。後で編集された文書の印は無効 (もう一度尋ねる)。
    discarded: Vec<(DocumentId, Revision)>,
    /// 済んだ (保存した・破棄した) 文書の数。
    done: usize,
}

impl QuitChain {
    /// 連鎖で尋ねる文書の総数 (`ui.confirm.unsaved.chain` の `{total}`)。
    fn total(&self) -> usize {
        self.done + self.remaining.len()
    }
}

/// 確認ダイアログの選択肢 1 つ。文言は `ui.confirm.<name>.<id>`。キャンセル (`ui.confirm.cancel`、`Esc`) は選択肢の最後に自動で付く。
#[derive(Clone, Copy, Debug)]
pub(crate) struct ChoiceDef {
    pub id: &'static str,
    /// アクセラレータ (小文字。大小は区別しない)。
    pub accel: Option<char>,
    /// 破棄などの破壊的な選択肢 (既定にしない)。
    pub destructive: bool,
}

/// 選択肢を選んだときの動作 (確認の種類ごと)。
#[derive(Clone, Debug)]
pub(crate) enum ConfirmAction {
    /// 未保存の変更 (終了・置き換え): 選択肢 0 = 保存して続ける、1 = 破棄して続ける。
    Unsaved(Continuation),
    /// 知らせるだけの確認 (全保存で保存できなかった文書の一覧)。どの選択肢でも何もしない。
    Notice,
    /// 保存の代替 (原子的置換ができない・ハードリンク・バックアップを作れない)。選択肢の意味は `SaveFallback` の種類による。
    Fallback(crate::io::save::FallbackAsk),
    /// 別のバイト表現で保存される箇所がある文書の保存 (保存 / 別名で保存、03 §8.6 E2')。
    Lossy(crate::io::open::LossyAsk),
    /// 読み取り専用属性を解除した文書の上書き保存 (別名で保存 / 読み取り専用を解除して保存、FR-116)。
    ClearReadOnly(crate::io::open::LossyAsk),
    /// 保存先がダングリングのシンボリックリンク (リンク先に作成 / キャンセル、FR-106・05 §4.1)。
    DanglingLink(crate::io::save::SaveFailedAsk),
    /// 外部変更の検知 (再読込 / 無視、03 §9.5)。`stamp` は見つけたときのファイルの状態。
    ExternalModified {
        doc: DocumentId,
        stamp: crate::FileStamp,
    },
    /// 外部で削除された (新規として保持 / 閉じる、03 §9.5)。
    ExternalDeleted(DocumentId),
    /// 保存の失敗の後の選択 (再試行 / 別名で保存 / 退避先に保存)。
    SaveFailed(crate::io::save::SaveFailedAsk),
    /// 開くときの確認 (読み取り専用で開く / 文字コードを指定して開く / 中止、03 §9.2)。
    Open(crate::io::open::OpenAsk),
    /// dirty な文書の再読込 (変更を破棄、03 §9.5)。
    Reload(DocumentId),
    /// 表現不能の文字があって保存を中止した後の選択 (UTF-8 で保存 / キャンセル、03 §8.6)。
    Unmappable(crate::io::unmappable::UnmappableAsk),
    /// スワップの復旧の提案 (適用 / 破棄 / キャンセル、03 §10.3)。`revision` は提案の時点の文書の Revision。
    SwapRecovery {
        entry: Box<crate::SwapEntry>,
        doc: eve_view::DocumentId,
        revision: eve_view::Revision,
    },
    /// 保存中の終了の確認 (待つ / 保存を中止して終了、03 §9.7 (f))。
    QuitSaving,
    /// 置換途中の痕跡の提案 (05 §4.6。選択肢は提案の種類による)。
    Leftover(Box<crate::Leftover>),
    /// エスケープ文字のつながりの確認 (このまま保存 / キャンセル、03 §8.3.1・§9.7 (e))。
    EscapeMerge(Box<crate::io::escape_merge::EscapeMergeAsk>),
    /// 置換の一覧の確認 (置換して保存 / キャンセル、03 §8.6)。`revision` は `ChangeSet` を作った時点の文書の Revision。
    UnmappableReplace {
        ask: crate::io::unmappable::UnmappableAsk,
        changes: eve_text::ChangeSet,
        revision: eve_view::Revision,
    },
}

/// 確認ダイアログ。選択肢は `choices` (+ 最後にキャンセル)。
#[derive(Clone, Debug)]
pub(crate) struct ConfirmState {
    /// `ui.confirm.<name>` の名前 (`unsaved` / `save.atomic_unsupported` など)。
    pub name: &'static str,
    /// 文言の引数 (`{path}` など)。`index` があるときの本文は `message` ではなく `chain` (終了の連鎖、03 §9.8)。
    pub args: Vec<(String, String)>,
    pub choices: Vec<ChoiceDef>,
    /// 本文の下に 1 行ずつ出す詳細 (E3 の置換の一覧)。無ければ空。
    pub details: Vec<String>,
    /// 本文の文言の葉 (`ui.confirm.<name>.<leaf>`)。`None` は `message` (終了の連鎖の何件目かがあれば `chain`)。
    /// 同じ確認で理由に応じて本文を替えるときに使う (E3 のエスケープ文字が理由の本文、つながりの確認の末尾まで等)。
    pub message: Option<&'static str>,
    /// 初めにフォーカスする選択肢 (`choices.len()` はキャンセル)。
    pub default: usize,
    pub focused: usize,
    pub action: ConfirmAction,
}

impl ConfirmState {
    /// キャンセルの番号 (選択肢の数)。
    pub(crate) fn cancel_index(&self) -> usize {
        self.choices.len()
    }
}

/// 未保存の確認の選択肢 (既定は保存)。
pub(crate) const UNSAVED_CHOICES: [ChoiceDef; 2] = [
    ChoiceDef {
        id: "save",
        accel: Some('s'),
        destructive: false,
    },
    ChoiceDef {
        id: "discard",
        accel: Some('d'),
        destructive: true,
    },
];

impl EditorState {
    /// `doc_id` が dirty なら確認を出して `true`。保存の後続は保存した文書に属するので、アクティブ文書ではなく文書を指す。
    pub(crate) fn confirm_doc_if_dirty(
        &mut self,
        doc_id: DocumentId,
        name: &'static str,
        cont: Continuation,
    ) -> bool {
        let Some(doc) = self.document(doc_id) else {
            return false;
        };
        if !doc.is_dirty() {
            return false;
        }
        let path = doc
            .path
            .as_ref()
            .map(|p| p.display())
            .unwrap_or_else(|| self.untitled_label(doc));
        let mut args = vec![("path".to_owned(), path)];
        // 終了の連鎖で尋ねる文書が 2 件以上なら、何件目かを前置する
        if cont == Continuation::Quit
            && let Some(chain) = self.ui.quit_chain.as_ref().filter(|c| c.total() >= 2)
        {
            args.push(("index".to_owned(), (chain.done + 1).to_string()));
            args.push(("total".to_owned(), chain.total().to_string()));
        }
        self.set_overlay(Some(Overlay::Confirm(Box::new(ConfirmState {
            name,
            args,
            choices: UNSAVED_CHOICES.to_vec(),
            details: Vec::new(),
            default: 0,
            focused: 0,
            message: None,
            action: ConfirmAction::Unsaved(cont),
        }))));
        self.mark_dirty_render();
        true
    }

    /// 確認ダイアログを出す (種類は `action`)。
    pub(crate) fn open_confirm(&mut self, c: ConfirmState) {
        self.set_overlay(Some(Overlay::Confirm(Box::new(c))));
        self.mark_dirty_render();
    }

    pub(super) fn choose(&mut self, choice: usize) -> CommandResult {
        let Some(Overlay::Confirm(c)) = self.take_overlay() else {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        };
        self.mark_dirty_render();
        // キャンセル (最後の選択肢): データを変えない
        if choice >= c.cancel_index() {
            match &c.action {
                ConfirmAction::SaveFailed(_) | ConfirmAction::Lossy(_) => {
                    self.show_message(MessageLevel::Info, keys::INFO_SAVE_ABORTED, &[]);
                }
                ConfirmAction::Open(ask) => self.open_aborted(ask),
                // 終了の連鎖のキャンセル: 連鎖全体を中止し、破棄の印をすべて無効にする (文書は元のまま)
                ConfirmAction::Unsaved(Continuation::Quit) => self.ui.quit_chain = None,
                ConfirmAction::Unmappable(ask) | ConfirmAction::UnmappableReplace { ask, .. } => {
                    self.unmappable_cancelled(ask)
                }
                ConfirmAction::EscapeMerge(ask) => self.escape_merge_cancelled(ask),
                _ => {}
            }
            return CommandResult::Done;
        }
        let id = c.choices[choice].id;
        match c.action {
            ConfirmAction::Unsaved(Continuation::Quit) => match choice {
                0 => self.quit_save_head(),
                _ => self.quit_discard_head(),
            },
            // 保存の後続は保存する文書に属する (アクティブ文書とは限らない、03 §9.8)
            ConfirmAction::Unsaved(
                cont @ (Continuation::Close(doc) | Continuation::Reopen(doc, _)),
            ) => match choice {
                0 => self.save_doc_then(doc, cont),
                _ => self.run_continuation(cont),
            },
            ConfirmAction::Notice => CommandResult::Done,
            ConfirmAction::Fallback(ask) => self.choose_fallback(ask, choice),
            ConfirmAction::Lossy(ask) => self.choose_lossy(ask, choice),
            ConfirmAction::ClearReadOnly(ask) => self.choose_clear_readonly(ask, choice),
            ConfirmAction::DanglingLink(ask) => self.create_link_target(ask),
            ConfirmAction::ExternalModified { doc, stamp } => {
                self.choose_external_modified(doc, stamp, choice)
            }
            ConfirmAction::ExternalDeleted(doc) => self.choose_external_deleted(doc, choice),
            ConfirmAction::SaveFailed(ask) => self.choose_save_failed(ask, id),
            ConfirmAction::Open(ask) => self.choose_open(ask, id),
            ConfirmAction::Reload(doc) => self.start_reload(doc),
            ConfirmAction::SwapRecovery {
                entry,
                doc,
                revision,
            } => self.choose_swap_recovery(*entry, doc, revision, id),
            ConfirmAction::Leftover(l) => self.choose_leftover(*l, id),
            ConfirmAction::QuitSaving => self.choose_quit_saving(id),
            ConfirmAction::Unmappable(ask) => self.choose_unmappable(ask, id),
            ConfirmAction::EscapeMerge(ask) => self.choose_escape_merge(*ask),
            ConfirmAction::UnmappableReplace {
                ask,
                changes,
                revision,
            } => self.apply_replacement_and_save(ask, changes, revision),
        }
    }

    /// 破棄または保存の後に続ける操作を実行する。
    pub(crate) fn run_continuation(&mut self, cont: Continuation) -> CommandResult {
        match cont {
            Continuation::Reopen(doc, enc) => self.start_reopen(doc, enc),
            Continuation::Close(doc) => self.close_buffer(doc),
            Continuation::Quit => {
                self.push_effect(Effect::Quit {
                    code: self.ui.quit_code,
                    rescue: QuitRescue::None,
                    stderr: Vec::new(),
                });
                CommandResult::Done
            }
        }
    }

    /// 終了する。dirty な文書があれば、全部について確認する終了の連鎖に入る (03 §9.8)。終了コードは 0 (07 §18.4)。
    pub fn request_quit(&mut self) -> CommandResult {
        self.request_quit_with(0)
    }

    /// `app.quit_with_error`: 保存せずに、終了コード 4 で終わる (07 §18.4)。確認は `app.quit` と同じで、
    /// dirty な文書があれば終了の連鎖 (保存 / 破棄 / キャンセル、FR-107) に入り、済んだら終了コード 4 で終わる。
    pub fn request_quit_with_error(&mut self) -> CommandResult {
        self.request_quit_with(EXIT_QUIT_WITH_ERROR)
    }

    /// 終了コード `code` で終了する要求。「保存を中止して終了」の最中は、その終了コードを変えない。
    fn request_quit_with(&mut self, code: u8) -> CommandResult {
        if self.ui.quit_abort.is_none() {
            self.ui.quit_code = code;
        }
        // 保存ジョブの実行中は、その完了を待つ (03 §9.7 (f))
        if let Some(r) = self.quit_while_saving(true) {
            return r;
        }
        self.begin_quit()
    }

    /// `app.save_and_quit`: 表示中の文書を保存し (`file.save`)、成功したら終了する (`app.quit`。07 §18.6)。
    /// 保存は終了の連鎖の「保存」と同じ経路で、無題・読み取り専用の文書は名前を尋ね、保存が失敗・キャンセルされたら
    /// 終了しない。保存の後は `app.quit` と同じで、ほかに dirty な文書があれば終了の連鎖で確認する。
    /// 表示中の文書が dirty でなければ、保存せずに `app.quit` と同じ終了へ進む。終了コードは 0。
    pub fn request_save_and_quit(&mut self) -> CommandResult {
        if self.ui.quit_abort.is_some() {
            return CommandResult::Done;
        }
        let Some(doc) = self
            .active_document()
            .filter(|d| self.document(*d).is_some_and(|d| d.is_dirty()))
        else {
            return self.request_quit();
        };
        self.ui.quit_code = 0;
        // 連鎖の途中の (前の終了の) 状態は持ち越さない。保存の成功後に、連鎖を作り直して残りを確かめる
        self.ui.quit_chain = None;
        self.save_doc_then(doc, Continuation::Quit)
    }

    /// 保存中の待ちを済ませた後の終了の入口: dirty な全文書の確認 (終了の連鎖) へ進む。
    fn begin_quit(&mut self) -> CommandResult {
        let remaining = self.quit_targets(&[]);
        self.ui.quit_chain = Some(QuitChain {
            remaining,
            discarded: Vec::new(),
            done: 0,
        });
        self.advance_quit_chain()
    }

    /// 確認が要る文書。dirty な全文書から、破棄の印が有効なもの (Revision が変わっていない) を除く。
    /// アクティブな文書を先頭に、残りは開いた順。
    fn quit_targets(&self, discarded: &[(DocumentId, Revision)]) -> Vec<DocumentId> {
        let mut ids: Vec<DocumentId> = self
            .docs
            .documents
            .iter()
            .filter(|(id, d)| d.is_dirty() && !discarded.contains(&(**id, d.revision())))
            .map(|(id, _)| *id)
            .collect();
        if let Some(i) = self
            .active_document()
            .and_then(|a| ids.iter().position(|d| *d == a))
        {
            let active = ids.remove(i);
            ids.insert(0, active);
        }
        ids
    }

    /// 終了の連鎖を次へ進める: 次の dirty な文書をアクティブにして確認を出す。全部済んで、
    /// 確認の間に変わった文書も無ければ終了する。
    pub(crate) fn advance_quit_chain(&mut self) -> CommandResult {
        let Some(mut chain) = self.ui.quit_chain.take() else {
            if let Some(r) = self.quit_while_saving(false) {
                return r;
            }
            return self.begin_quit();
        };
        // 保存して clean になった・閉じた文書は済んだものとして外す
        while let Some(&head) = chain.remaining.first() {
            if self.document(head).is_some_and(|d| d.is_dirty()) {
                break;
            }
            chain.remaining.remove(0);
            chain.done += 1;
        }
        if chain.remaining.is_empty() {
            // 保存中の編集・破棄の印の後の編集を見落とさない (編集を捨てない)
            chain.remaining = self.quit_targets(&chain.discarded);
            if chain.remaining.is_empty() {
                return self.run_continuation(Continuation::Quit);
            }
        }
        let head = chain.remaining[0];
        self.ui.quit_chain = Some(chain);
        self.activate_document(head);
        self.confirm_doc_if_dirty(head, "unsaved", Continuation::Quit);
        CommandResult::Done
    }

    /// 連鎖の確認中の文書。
    fn quit_head(&self) -> Option<DocumentId> {
        self.ui.quit_chain.as_ref()?.remaining.first().copied()
    }

    /// 終了の連鎖で「保存」: 確認中の文書を保存し、成功したら次へ進む。
    fn quit_save_head(&mut self) -> CommandResult {
        match self.quit_head() {
            Some(doc) => self.save_doc_then(doc, Continuation::Quit),
            None => CommandResult::Rejected(RejectReason::NoTarget),
        }
    }

    /// 終了の連鎖で「破棄」: 内容は残して印だけ付け、次へ進む。
    fn quit_discard_head(&mut self) -> CommandResult {
        if let Some(chain) = self.ui.quit_chain.as_mut()
            && !chain.remaining.is_empty()
        {
            let doc = chain.remaining.remove(0);
            chain.done += 1;
            if let Some(d) = self.docs.documents.get(&doc) {
                chain.discarded.push((doc, d.revision()));
            }
        }
        self.advance_quit_chain()
    }
}
