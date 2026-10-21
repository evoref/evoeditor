//! ファイルの開く・保存 (docs/03 §9.2 / §9.3 の M1 の範囲)。ジョブを投入し、結果で Document を更新する。
//!
//! M1 の範囲: UTF-8、原子的保存、未編集の保存の照合 (手順 0)。次は後続: 確認ダイアログ (dirty の置き換え、
//! 保存の失敗の選択肢、バイナリ・巨大ファイル)、名前の入力プロンプト、保存の連鎖 (Continuation)、保存中フラグ、
//! 外部変更の再読込。確認が要る場面は、確認の UI ができるまで `RejectReason::NeedsInput` で拒否する。

use eve_view::{DocumentId, EditorEvent, MessageLevel, ViewId};

use crate::document::{Document, NodeId};
use crate::fs::{
    Expect, FsError, FsErrorDetail, FsErrorKind, HardlinkPolicy, InPlaceBackup, SaveMethod,
};
use crate::fspath::FsPath;
use crate::io_jobs::OpenOptions;
use crate::jobs::{ConfirmAsk, JobError, JobKind, JobResult, MetaPurpose, SaveStrategy};
use eve_encoding::Encoding;

use super::command::{
    CommandCtx, CommandKind, CommandRegistry, CommandResult, CommandSpec, Permission, RejectReason,
};
use super::overlay::{Continuation, PromptKind};
use super::state::EditorState;

/// 投入したジョブと、その結果の宛先。
#[derive(Debug)]
pub(super) enum Pending {
    Open {
        doc: DocumentId,
        view: ViewId,
        path: FsPath,
        replace: Option<DocumentId>,
        /// 投入したときの指定 (確認の回答で足して再投入する、03 §9.2)。
        opts: OpenOptions,
        /// 文字コードを指定した開き直し (`file.reopen_with_encoding`、03 §8.8)。同じ文書・View の中身を差し替える。
        reopen: bool,
    },
    /// 未編集の上書き保存の照合 (03 §9.3 手順 0)。
    CheckUnchanged { doc: DocumentId },
    /// 起動時の退避ディレクトリの走査。
    RecoveryScan,
    /// 「退避先に保存」(03 §9.7 (e))。`path` は元のファイル。
    WriteRecovery { path: FsPath },
    Save {
        doc: DocumentId,
        at: NodeId,
        revision: eve_view::Revision,
        path: FsPath,
        /// 確認の後に同じ保存をやり直すための照合の期待 (03 §9.3)。
        expected: Expect,
    },
}

/// 開くときの確認 (`ui.confirm.open.*`、03 §9.2) に答えるための材料。読込中の文書 (Loading) は確認の間も残す。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct OpenAsk {
    pub doc: DocumentId,
    pub view: ViewId,
    pub path: FsPath,
    pub replace: Option<DocumentId>,
    pub opts: OpenOptions,
    pub ask: ConfirmAsk,
    /// 開き直し (同じ文書の中身を差し替える)。
    pub reopen: bool,
}

/// 保存の失敗の後の選択 (`ui.confirm.save_failed`、03 §9.7 (e))。
#[derive(Clone, Debug)]
pub(super) struct SaveFailedAsk {
    pub doc: DocumentId,
    pub path: FsPath,
    pub expected: Expect,
    pub cont: Option<Continuation>,
    /// 失敗した保存の時点の本文 (確認の間は編集できない)。「退避先に保存」で書く。
    pub snapshot: std::sync::Arc<crate::DocSnapshot>,
}

/// 保存の代替の確認 (`AtomicUnsupported` / ハードリンク / `BackupFailed`、03 §9.3 手順 7・05 §4.4)。
#[derive(Clone, Debug)]
pub(super) struct FallbackAsk {
    pub kind: FallbackKind,
    pub doc: DocumentId,
    pub path: FsPath,
    pub expected: Expect,
    pub cont: Option<Continuation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FallbackKind {
    /// 原子的置換ができない (`ui.confirm.save.atomic_unsupported`)。選択肢: 直接上書き / 別名で保存 (既定)。
    AtomicUnsupported,
    /// ハードリンク (`ui.confirm.save.hardlink`)。選択肢: リンクを保って直接上書き / 切り離して保存。既定はキャンセル。
    Hardlink,
    /// 事前バックアップ・保存中フラグを書けなかった (`ui.confirm.save.overwrite_without_backup`)。選択肢: バックアップなしで上書き / 別名で保存。既定はキャンセル。
    NoBackup,
}

impl EditorState {
    /// MessageKey を解決して表示する。`path` は `{path}` (表示用のパス)。
    fn message(&mut self, level: MessageLevel, key: &str, path: &str) {
        self.show_message(level, key, &[("path", path)]);
    }

    /// 保存ジョブの完了を待っているか (ステータスバーの `[保存中]`)。
    pub(super) fn is_saving(&self, doc: DocumentId) -> bool {
        self.pending
            .values()
            .any(|p| matches!(p, Pending::Save { doc: d, .. } if *d == doc))
    }

    /// 読込を待っている間、置き換え対象の今の文書は編集を受け付けない (03 §9.2 の置き換えの手順)。
    /// 編集は読込の完了で黙って捨てられてしまうため。
    pub(super) fn open_replacing(&self) -> bool {
        self.pending.values().any(|p| {
            matches!(
                p,
                Pending::Open {
                    replace: Some(_),
                    ..
                }
            )
        })
    }

    fn open_in_progress(&self) -> bool {
        self.pending
            .values()
            .any(|p| matches!(p, Pending::Open { .. }))
    }

    fn save_in_progress(&self, doc_id: DocumentId) -> bool {
        self.pending
            .values()
            .any(|p| matches!(p, Pending::Save { doc, .. } | Pending::CheckUnchanged { doc } if *doc == doc_id))
    }

    /// 今の文書 (最初の Window の View の文書)。
    pub fn active_document(&self) -> Option<DocumentId> {
        let w = self.first_window()?;
        self.view(self.window_view(w)?).map(|v| v.doc)
    }

    /// 無題の新規文書を作る (FR-124)。今の文書があれば置き換える。dirty なら保存・破棄・キャンセルの確認を出す。
    pub fn new_untitled(&mut self) -> CommandResult {
        if self.open_in_progress() {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        if self.confirm_if_dirty("unsaved_replace", Continuation::New) {
            return CommandResult::Done;
        }
        self.new_untitled_forced()
    }

    /// 確認を済ませた (破棄または保存した) 後の新規作成。
    pub(super) fn new_untitled_forced(&mut self) -> CommandResult {
        let replace = self.active_document();
        let (_, view) = self.add_document(Document::new_untitled());
        self.show_view(view, replace);
        CommandResult::Done
    }

    /// Window の表示を `view` に切り替え、`old` の文書を閉じる。Window がなければ作る。
    fn show_view(&mut self, view: ViewId, old: Option<DocumentId>) {
        match self.windows.first_mut() {
            Some(w) => w.root = view,
            None => {
                self.add_window(view);
            }
        }
        if let Some(old) = old {
            self.close_document(old);
        }
        self.mark_dirty_render();
    }

    fn close_document(&mut self, doc: DocumentId) {
        self.views.retain(|_, v| v.doc != doc);
        if self.documents.remove(&doc).is_some() {
            // 退避用の公開物から外す (破棄した内容を緊急退避に残さない)
            self.mark_snapshot_update(doc);
            self.push_event(EditorEvent::DocumentClosed { doc });
            self.take_pending_for(doc);
        }
    }

    fn take_pending_for(&mut self, doc: DocumentId) {
        let ids: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, p)| match p {
                Pending::Open { doc: d, .. } | Pending::CheckUnchanged { doc: d } => *d == doc,
                // 保存ジョブは取り消さず、完了を待つ (03 §9.2 の「閉じる手順」。ユーザーが求めた保存を中断しない)
                Pending::Save { .. } | Pending::RecoveryScan | Pending::WriteRecovery { .. } => {
                    false
                }
            })
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.continuations.remove(&id);
            self.pending.remove(&id);
            self.jobs.cancel(id);
        }
    }

    /// ファイルを開く (03 §9.2)。Loading の文書を作ってすぐ読込ジョブを投入する。
    /// すでに文書があるときは、読込が終わるまで今の文書を表示し続け、完了で置き換える。
    pub fn open_path(&mut self, raw_path: &str, opts: OpenOptions) -> CommandResult {
        self.open_path_inner(raw_path, opts, false)
    }

    /// 確認を済ませた (破棄または保存した) 後に開く。
    pub(super) fn open_path_forced(&mut self, raw_path: &str, opts: OpenOptions) -> CommandResult {
        self.open_path_inner(raw_path, opts, true)
    }

    fn open_path_inner(&mut self, raw_path: &str, opts: OpenOptions, force: bool) -> CommandResult {
        // 読込の途中の置き換えの重なりは受け付けない (置き換え対象の文書が孤立しないように)
        if self.open_in_progress() {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        let Ok(path) = self.paths.normalize(raw_path) else {
            self.show_message(
                MessageLevel::Error,
                "msg.open.failed",
                &[("path", raw_path), ("reason", "invalid path")],
            );
            return CommandResult::Rejected(RejectReason::InvalidArgs);
        };
        let replace = self.active_document();
        if replace
            .and_then(|d| self.document(d))
            .is_some_and(|cur| cur.path.as_ref() == Some(&path))
        {
            self.message(
                MessageLevel::Info,
                "info.open.already_open",
                &path.display(),
            );
            return CommandResult::Done;
        }
        // 未保存の変更があれば、保存・破棄・キャンセルを尋ねる (FR-107 AC-3)
        if !force
            && self.confirm_if_dirty("unsaved_replace", Continuation::Open(raw_path.to_owned()))
        {
            return CommandResult::Done;
        }
        let mut doc = Document::new_untitled();
        doc.path = Some(path.clone());
        doc.load_state = crate::document::LoadState::Loading;
        let (doc_id, view) = self.add_document(doc);
        if replace.is_none() {
            self.show_view(view, None);
        }
        let job = self.jobs.spawn(
            Some(doc_id),
            eve_view::Revision::INITIAL,
            JobKind::Open {
                path: path.clone(),
                opts,
            },
        );
        self.pending.insert(
            job,
            Pending::Open {
                doc: doc_id,
                view,
                path,
                replace,
                opts,
                reopen: false,
            },
        );
        CommandResult::Pending(job)
    }

    /// 上書き保存 (03 §9.3)。パスのない文書は名前の入力が要る。
    pub fn request_save(&mut self) -> CommandResult {
        let Some(doc_id) = self.active_document() else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(doc) = self.document(doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(path) = doc.path.clone() else {
            // パスがなければ名前を尋ねる
            self.open_prompt(PromptKind::SaveAs { then: None }, String::new());
            return CommandResult::Done;
        };
        if self.save_in_progress(doc_id) {
            self.message(MessageLevel::Info, "info.save.in_progress", &path.display());
            return CommandResult::Done;
        }
        if !doc.is_dirty() {
            // 手順 0: 未編集なら、ディスク上のファイルも変わっていないことを確かめてから書かない
            let job = self.jobs.spawn(
                Some(doc_id),
                doc.revision(),
                JobKind::Metadata {
                    path,
                    purpose: MetaPurpose::UnchangedSave,
                },
            );
            self.pending
                .insert(job, Pending::CheckUnchanged { doc: doc_id });
            return CommandResult::Pending(job);
        }
        let expected = doc.disk_stamp.map_or(Expect::Absent, Expect::Stamp);
        self.start_save(doc_id, path, expected, None)
    }

    /// 確認で「保存」を選んだ後: 保存し、成功したら `cont` を続ける。パスがなければ名前を尋ねてから保存する。
    pub(super) fn save_then(&mut self, cont: Continuation) -> CommandResult {
        let Some(doc_id) = self.active_document() else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(doc) = self.document(doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        // 保存中なら、その保存の完了後に続ける (二重に保存しない)
        let in_flight = self.pending.iter().find_map(|(job, p)| match p {
            Pending::Save { doc, .. } if *doc == doc_id => Some(*job),
            _ => None,
        });
        if let Some(job) = in_flight {
            self.continuations.insert(job, cont);
            return CommandResult::Pending(job);
        }
        let Some(path) = doc.path.clone() else {
            self.open_prompt(PromptKind::SaveAs { then: Some(cont) }, String::new());
            return CommandResult::Done;
        };
        let expected = doc.disk_stamp.map_or(Expect::Absent, Expect::Stamp);
        self.start_save(doc_id, path, expected, Some(cont))
    }

    /// 名前を付けて保存 (パスを指定する)。
    pub(super) fn request_save_as(
        &mut self,
        raw_path: &str,
        cont: Option<Continuation>,
    ) -> CommandResult {
        let Some(doc_id) = self.active_document() else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Ok(path) = self.paths.normalize(raw_path) else {
            self.show_message(
                MessageLevel::Error,
                "msg.save.invalid_path",
                &[("path", raw_path), ("reason", "invalid path")],
            );
            return CommandResult::Rejected(RejectReason::InvalidArgs);
        };
        let Some(doc) = self.documents.get(&doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        // 読込中・読込に失敗した文書は保存できない (中身が揃っていない)
        if doc.load_state != crate::document::LoadState::Ready {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        if self.save_in_progress(doc_id) {
            self.message(MessageLevel::Info, "info.save.in_progress", &path.display());
            return CommandResult::Done;
        }
        let same = doc.path.as_ref() == Some(&path);
        // 読み取り専用の文書を同じファイルへ書くと、元のバイト列を損なうことがある (03 §9.7 (a) の 1)
        if same && !doc.is_editable() {
            self.message(
                MessageLevel::Warn,
                "msg.save_as.same_file_read_only",
                &path.display(),
            );
            return CommandResult::Rejected(RejectReason::ReadOnly);
        }
        // 同じファイルなら読んだときの Stamp で照合する。別のファイルへは存在しないことを期待する
        // (既存なら Conflict。上書きの確認は確認ダイアログ (後続) の仕事)。パスと Stamp は成功してから更新する
        let expected = if same {
            doc.disk_stamp.map_or(Expect::Absent, Expect::Stamp)
        } else {
            Expect::Absent
        };
        self.start_save(doc_id, path, expected, cont)
    }

    fn start_save(
        &mut self,
        doc_id: DocumentId,
        path: FsPath,
        expected: Expect,
        cont: Option<Continuation>,
    ) -> CommandResult {
        self.start_save_with(doc_id, path, expected, cont, SaveStrategy::default())
    }

    /// 保存方式を指定して保存ジョブを投入する (確認の後の再投入で直接上書きにする)。
    fn start_save_with(
        &mut self,
        doc_id: DocumentId,
        path: FsPath,
        expected: Expect,
        cont: Option<Continuation>,
        strategy: SaveStrategy,
    ) -> CommandResult {
        let Some(doc) = self.documents.get_mut(&doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        // 状態付き符号化 (ISO-2022-JP) で不正バイトを含む文書は保存しない (03 §8.3 (a)。不正バイトを失わないため)
        if doc.format.encoding.is_stateful() && doc.format.invalid_bytes > 0 {
            self.show_message(
                MessageLevel::Error,
                "msg.save.encoding_unsupported_iso2022",
                &[("path", &path.display())],
            );
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        let (encoding, bom, pua) = (
            doc.format.encoding,
            doc.format.has_bom,
            doc.format.pua_escape,
        );
        let revision = doc.revision();
        let (snapshot, at) = doc.begin_save();
        let job = self.jobs.spawn(
            Some(doc_id),
            revision,
            JobKind::Save {
                path: path.clone(),
                snapshot,
                encoding,
                bom,
                pua,
                expected,
                strategy,
            },
        );
        if let Some(c) = cont {
            self.continuations.insert(job, c);
        }
        self.pending.insert(
            job,
            Pending::Save {
                doc: doc_id,
                at,
                revision,
                path,
                expected,
            },
        );
        CommandResult::Pending(job)
    }

    /// ジョブの結果を受け取る (02 §5.4)。
    pub fn on_job_result(&mut self, r: JobResult) {
        let job = r.job();
        let cont = self.continuations.remove(&job);
        let Some(pending) = self.pending.remove(&job) else {
            return;
        };
        match pending {
            Pending::Open {
                doc,
                view,
                path,
                replace,
                opts,
                reopen,
            } => self.finish_open(doc, view, path, replace, opts, reopen, r),
            Pending::CheckUnchanged { doc } => self.finish_check(doc, r),
            Pending::WriteRecovery { path } => self.finish_write_recovery(&path, r),
            Pending::RecoveryScan => self.finish_recovery_scan(r),
            Pending::Save {
                doc,
                at,
                revision,
                path,
                expected,
            } => self.finish_save(doc, at, revision, &path, expected, cont, r),
        }
        self.mark_dirty_render();
    }

    /// 保存の失敗の確認に渡す材料 (文書が無い・読込中なら `None`)。
    fn save_failed_ask(
        &self,
        doc: DocumentId,
        path: &FsPath,
        expected: Expect,
        cont: Option<Continuation>,
    ) -> Option<SaveFailedAsk> {
        let snapshot = self.rescue_snapshot(doc, false)?;
        Some(SaveFailedAsk {
            doc,
            path: path.clone(),
            expected,
            cont,
            snapshot: std::sync::Arc::new(snapshot),
        })
    }

    /// 保存の失敗を `ui.confirm.save_failed` で示す (03 §9.7 (e))。本文にメッセージ `key` の text を入れる。
    /// 材料が無ければ (文書が閉じている等) メッセージだけ示す。`retry` は Busy のときだけ (何も書いていないので照合は通る)。
    fn report_save_failure(
        &mut self,
        ask: Option<SaveFailedAsk>,
        key: &str,
        args: &[(&str, &str)],
        retry: bool,
    ) {
        use super::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
        let Some(ask) = ask else {
            self.show_message(MessageLevel::Error, key, args);
            return;
        };
        let detail = self.message_text(key, args);
        let choice = |id: &'static str, accel: char| ChoiceDef {
            id,
            accel: Some(accel),
            destructive: false,
        };
        let mut choices = Vec::new();
        if retry {
            choices.push(choice("retry", 't'));
        }
        choices.push(choice("alt", 'a'));
        choices.push(choice("recovery", 'r'));
        let default = usize::from(retry);
        self.open_confirm(ConfirmState {
            name: "save_failed",
            args: vec![("detail".to_owned(), detail)],
            choices,
            default,
            focused: default,
            action: ConfirmAction::SaveFailed(ask),
        });
    }

    /// 保存の失敗の確認の選択 (キャンセルは呼び側が処理済み)。`id` は選択肢の ID。
    pub(super) fn choose_save_failed(&mut self, ask: SaveFailedAsk, id: &str) -> CommandResult {
        let SaveFailedAsk {
            doc,
            path,
            expected,
            cont,
            snapshot,
        } = ask;
        match id {
            "retry" => self.start_save(doc, path, expected, cont),
            "recovery" => {
                // 続きの操作 (終了・置き換え) は行わない。書いた場所は結果のメッセージで示す
                let job = self.jobs.spawn(
                    Some(doc),
                    snapshot.revision,
                    JobKind::WriteRecovery { doc: snapshot },
                );
                self.pending.insert(job, Pending::WriteRecovery { path });
                CommandResult::Pending(job)
            }
            // 別名で保存: 名前の入力へ (保存の後に続ける操作は引き継ぐ)
            _ => {
                self.open_prompt(PromptKind::SaveAs { then: cont }, String::new());
                CommandResult::Done
            }
        }
    }

    /// 「退避先に保存」の結果。文書の状態 (dirty・保存点) は変えない。
    fn finish_write_recovery(&mut self, path: &FsPath, r: JobResult) {
        match r {
            JobResult::RecoveryWritten { path: target, .. } => self.show_message(
                MessageLevel::Info,
                "info.save.to_recovery",
                &[("target", &target.display()), ("path", &path.display())],
            ),
            JobResult::Failed { err, .. } => {
                let reason = reason_label(&err);
                self.show_message(
                    MessageLevel::Error,
                    "msg.save.recovery_failed",
                    &[("reason", &reason), ("path", &path.display())],
                );
            }
            _ => {}
        }
    }

    /// 保存の代替の確認を出す。
    fn open_fallback(&mut self, ask: FallbackAsk, reason: &str, links: &str) {
        use super::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
        let choice = |id: &'static str, accel: char| ChoiceDef {
            id,
            accel: Some(accel),
            destructive: false,
        };
        let (name, choices, default): (&'static str, Vec<ChoiceDef>, usize) = match ask.kind {
            FallbackKind::AtomicUnsupported => (
                "save.atomic_unsupported",
                vec![choice("direct", 'o'), choice("alt", 'a')],
                1,
            ),
            FallbackKind::Hardlink => (
                "save.hardlink",
                vec![choice("direct", 'k'), choice("break", 'b')],
                2,
            ),
            FallbackKind::NoBackup => (
                "save.overwrite_without_backup",
                vec![choice("overwrite", 'o'), choice("alt", 'a')],
                2,
            ),
        };
        self.open_confirm(ConfirmState {
            name,
            args: vec![
                ("path".to_owned(), ask.path.display()),
                ("reason".to_owned(), reason.to_owned()),
                ("links".to_owned(), links.to_owned()),
            ],
            choices,
            default,
            focused: default,
            action: ConfirmAction::Fallback(ask),
        });
    }

    /// 保存の代替の確認の選択 (キャンセルは呼び側が処理済み)。
    pub(super) fn choose_fallback(&mut self, ask: FallbackAsk, choice: usize) -> CommandResult {
        let FallbackAsk {
            kind,
            doc,
            path,
            expected,
            cont,
        } = ask;
        let in_place = |backup| SaveStrategy::InPlace { backup };
        let strategy = match (kind, choice) {
            (FallbackKind::AtomicUnsupported | FallbackKind::Hardlink, 0) => {
                Some(in_place(InPlaceBackup::Required))
            }
            (FallbackKind::Hardlink, 1) => Some(SaveStrategy::Atomic {
                hardlink: HardlinkPolicy::BreakLink,
            }),
            (FallbackKind::NoBackup, 0) => Some(in_place(InPlaceBackup::SkipConfirmed)),
            // 別名で保存: 名前の入力へ (保存の後に続ける操作は引き継ぐ)
            _ => None,
        };
        match strategy {
            Some(s) => self.start_save_with(doc, path, expected, cont, s),
            None => {
                self.open_prompt(PromptKind::SaveAs { then: cont }, String::new());
                CommandResult::Done
            }
        }
    }

    /// 起動時に退避ディレクトリを走査するジョブを投入する (`open_initial` が呼ぶ)。
    pub(super) fn start_recovery_scan(&mut self) {
        let job = self
            .jobs
            .spawn(None, eve_view::Revision::INITIAL, JobKind::RecoveryScan);
        self.pending.insert(job, Pending::RecoveryScan);
    }

    /// 退避が残っていれば場所を案内する (`msg.recovery.found` / `found_rescue`、02 §9.3。中身は読まない)。
    /// 案内するのは最も新しい退避 (名前の `<UTC timestamp>-<pid>[-<n>]` の順)。「退避先に保存」の退避だけなら文言を分ける。
    fn finish_recovery_scan(&mut self, r: JobResult) {
        let JobResult::RecoveryFound { mut dirs, .. } = r else {
            return;
        };
        if dirs.is_empty() {
            return;
        }
        let only_rescue = dirs.iter().all(|d| d.rescue);
        // 異常終了の退避があれば、その中で最も新しいものを示す (無ければ「退避先に保存」の中で最も新しいもの)
        if !only_rescue {
            dirs.retain(|d| !d.rescue);
        }
        let key = |d: &crate::fs::RecoveryDirEntry| {
            d.path
                .as_path()
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        // 同じ時刻は `-<n>` の大きい方が新しい。名前の長さ → 辞書順で比べる (`-2` は `-` なしより後)
        dirs.sort_by(|a, b| {
            let (ka, kb) = (key(a), key(b));
            ka.get(..16)
                .cmp(&kb.get(..16))
                .then(ka.len().cmp(&kb.len()))
                .then(ka.cmp(&kb))
        });
        let count = dirs.len().to_string();
        let newest = dirs.last().map(|d| d.path.display()).unwrap_or_default();
        self.show_message(
            MessageLevel::Warn,
            if only_rescue {
                "msg.recovery.found_rescue"
            } else {
                "msg.recovery.found"
            },
            &[("dir", &newest), ("count", &count)],
        );
    }

    #[allow(clippy::too_many_arguments)] // 読込ジョブの文脈 (Pending::Open) をそのまま受ける
    fn finish_open(
        &mut self,
        doc_id: DocumentId,
        view: ViewId,
        path: FsPath,
        replace: Option<DocumentId>,
        opts: OpenOptions,
        reopen: bool,
        r: JobResult,
    ) {
        if reopen {
            self.finish_reopen(doc_id, view, path, opts, r);
            return;
        }
        match r {
            JobResult::Complete {
                text,
                format,
                meta: _,
                stamp,
                changed_during_load,
                detected,
                notes,
                interrupted_save,
                ..
            } => {
                let lossy = format.roundtrip.lossy_lines;
                let encoding = format.encoding;
                let mut d = Document::from_text(text, format);
                d.path = Some(path.clone());
                d.disk_stamp = Some(stamp);
                d.read_only = detected;
                self.documents.insert(doc_id, d);
                if changed_during_load {
                    self.message(
                        MessageLevel::Warn,
                        "msg.open.changed_during_load",
                        &path.display(),
                    );
                }
                // 文字コードに関する通知 (最後に出したものが現在のメッセージ)
                if lossy > 0 {
                    self.show_message(
                        MessageLevel::Info,
                        "info.open.lossy_roundtrip",
                        &[("count", &lossy.to_string())],
                    );
                }
                if let Some(e) = notes.redetected {
                    self.show_message(
                        MessageLevel::Info,
                        "info.open.redetected",
                        &[("encoding", e.name())],
                    );
                }
                if let Some(guess) = notes.guess_unsupported {
                    self.show_message(
                        MessageLevel::Info,
                        "info.open.guess_unsupported",
                        &[
                            ("path", &path.display()),
                            ("guess", guess),
                            ("encoding", encoding.name()),
                        ],
                    );
                }
                // 中断された保存の痕跡は最も重要なので最後に出す (現在のメッセージにする)
                if let Some(is) = &interrupted_save {
                    self.show_interrupted(&path, is);
                }
                self.complete_open(doc_id, view, replace);
            }
            JobResult::NewFile {
                parent_missing,
                interrupted_save,
                ..
            } => {
                // 存在しないパス: 空の新規文書 (dirty ではない)。保存すると作成する
                let mut d = Document::new_untitled();
                d.path = Some(path.clone());
                self.documents.insert(doc_id, d);
                let key = if parent_missing.is_some() {
                    "msg.open.parent_missing"
                } else {
                    "info.open.new_file"
                };
                let dir = parent_missing
                    .as_ref()
                    .map(FsPath::display)
                    .unwrap_or_default();
                self.show_message(
                    if parent_missing.is_some() {
                        MessageLevel::Warn
                    } else {
                        MessageLevel::Info
                    },
                    key,
                    &[("path", &path.display()), ("dir", &dir)],
                );
                if let Some(is) = &interrupted_save {
                    self.show_interrupted(&path, is);
                }
                self.complete_open(doc_id, view, replace);
            }
            // 確認が要る: 読込中の文書は残したまま確認を出し、回答で先頭から読み直す (03 §9.2)
            JobResult::NeedsConfirm { ask, .. } => {
                self.open_open_confirm(OpenAsk {
                    doc: doc_id,
                    view,
                    path,
                    replace,
                    opts,
                    ask,
                    reopen: false,
                });
            }
            JobResult::Failed { err, .. } => {
                let reason = reason_label(&err);
                let (level, key) = match err {
                    JobError::Cancelled => (MessageLevel::Info, "info.open.cancelled"),
                    JobError::Fs(e) => (MessageLevel::Error, open_error_key(&e)),
                    JobError::OutOfMemory { .. } => (MessageLevel::Error, "msg.open.out_of_memory"),
                    _ => (MessageLevel::Error, "msg.open.failed"),
                };
                self.open_failed(doc_id, replace, level, key, &path, &reason);
            }
            _ => {}
        }
    }

    /// 開くときの確認 (`ui.confirm.open.*`) を出す。
    fn open_open_confirm(&mut self, ask: OpenAsk) {
        use super::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
        let choice = |id: &'static str, accel: char| ChoiceDef {
            id,
            accel: Some(accel),
            destructive: false,
        };
        let (name, choices, args): (&'static str, Vec<ChoiceDef>, Vec<(String, String)>) =
            match ask.ask {
                ConfirmAsk::Binary => (
                    "open.binary",
                    vec![choice("readonly", 'r'), choice("encoding", 'e')],
                    vec![],
                ),
                ConfirmAsk::UnsupportedBom { .. } => (
                    "open.utf32",
                    vec![choice("readonly", 'r'), choice("encoding", 'e')],
                    vec![],
                ),
                ConfirmAsk::HugeFile { len } => (
                    "open.huge",
                    vec![choice("readonly", 'r')],
                    vec![("size".to_owned(), human_size(len))],
                ),
                ConfirmAsk::ManyInvalidBytes { encoding, permille } => (
                    "open.invalid_bytes",
                    vec![
                        choice("readonly", 'r'),
                        choice("encoding", 'e'),
                        choice("abort", 'c'),
                    ],
                    vec![
                        ("encoding".to_owned(), encoding.name().to_owned()),
                        (
                            "ratio".to_owned(),
                            format!("{}.{}%", permille / 10, permille % 10),
                        ),
                        (
                            "factor".to_owned(),
                            format!("{:.1}", encoding.memory_factor()),
                        ),
                    ],
                ),
            };
        let mut args = args;
        args.insert(0, ("path".to_owned(), ask.path.display()));
        self.open_confirm(ConfirmState {
            name,
            args,
            choices,
            default: 0,
            focused: 0,
            action: ConfirmAction::Open(ask),
        });
    }

    /// 開くときの確認の選択 (キャンセルは呼び側が `open_aborted` を呼ぶ)。`id` は選択肢の ID。
    pub(super) fn choose_open(&mut self, ask: OpenAsk, id: &str) -> CommandResult {
        match id {
            "readonly" => {
                let mut opts = ask.opts;
                opts.read_only_requested = true;
                match ask.ask {
                    ConfirmAsk::HugeFile { .. } => {
                        opts.confirmed_large = true;
                        opts.pending
                            .insert(crate::document::ReadOnlyReason::LargeFileViewer);
                    }
                    ConfirmAsk::Binary | ConfirmAsk::UnsupportedBom { .. } => {
                        opts.pending.insert(crate::document::ReadOnlyReason::Binary);
                    }
                    _ => {}
                }
                self.respawn_open(ask, opts)
            }
            "encoding" => {
                self.open_prompt(PromptKind::Encoding { ask }, String::new());
                CommandResult::Done
            }
            _ => {
                self.open_aborted(&ask);
                CommandResult::Done
            }
        }
    }

    /// 開くのを中止した (確認の `Esc` / 中止、文字コードの入力の `Esc`)。読込中の文書を捨てる。
    pub(super) fn open_aborted(&mut self, ask: &OpenAsk) {
        if ask.reopen {
            // 開き直しの中止: 旧本文・書式・履歴をそのまま保つ (03 §8.8)
            self.reopen_restore(ask.doc);
            self.message(
                MessageLevel::Info,
                "info.open.cancelled",
                &ask.path.display(),
            );
            return;
        }
        self.open_failed(
            ask.doc,
            ask.replace,
            MessageLevel::Info,
            "info.open.cancelled",
            &ask.path,
            "",
        );
    }

    /// 確認の回答の後、同じ読込中の文書へ `opts` で先頭から読み直すジョブを投入する。
    pub(super) fn respawn_open(&mut self, ask: OpenAsk, opts: OpenOptions) -> CommandResult {
        let job = self.jobs.spawn(
            Some(ask.doc),
            eve_view::Revision::INITIAL,
            JobKind::Open {
                path: ask.path.clone(),
                opts,
            },
        );
        self.pending.insert(
            job,
            Pending::Open {
                doc: ask.doc,
                view: ask.view,
                path: ask.path,
                replace: ask.replace,
                opts,
                reopen: ask.reopen,
            },
        );
        CommandResult::Pending(job)
    }

    // ---------- 文字コードを指定して開き直す (03 §8.8) ----------

    /// `file.reopen_with_encoding`。`enc` が無ければ文字コードを尋ねる。dirty なら保存か破棄を求める (FR-111 AC-2)。
    pub fn reopen_with_encoding(&mut self, enc: Option<Encoding>) -> CommandResult {
        let Some(doc_id) = self.active_document() else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(doc) = self.documents.get(&doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(path) = doc.path.clone() else {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        };
        if doc.load_state != crate::document::LoadState::Ready || self.open_in_progress() {
            self.message(MessageLevel::Info, "info.reload.busy", &path.display());
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        // 保存ジョブの実行中はその完了を待たせる (保存が終わってから実行し直す)
        if self.save_in_progress(doc_id) {
            self.message(MessageLevel::Info, "info.save.in_progress", &path.display());
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        let Some(enc) = enc else {
            self.open_prompt(PromptKind::ReopenEncoding, String::new());
            return CommandResult::Done;
        };
        if self.confirm_if_dirty("unsaved_reopen", Continuation::Reopen(enc)) {
            return CommandResult::Done;
        }
        self.start_reopen(enc)
    }

    /// 確認を済ませた (または dirty でない) 開き直しの開始。同じ `DocumentId` で読込ジョブを投入し、読込中は旧本文を表示し続ける。
    pub(super) fn start_reopen(&mut self, enc: Encoding) -> CommandResult {
        let Some(doc_id) = self.active_document() else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(w) = self.first_window() else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(view) = self.window_view(w) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(doc) = self.documents.get_mut(&doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(path) = doc.path.clone() else {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        };
        // 利用者の指定と書込権限の理由は開き直しでも保つ (03 §8.8 の「読み取り専用の理由の差し替え」)
        let mut pending = crate::document::ReadOnlyReasons::default();
        for r in [
            crate::document::ReadOnlyReason::UserRequested,
            crate::document::ReadOnlyReason::NoWritePermission,
        ] {
            if doc.read_only.contains(r) {
                pending.insert(r);
            }
        }
        doc.load_state = crate::document::LoadState::Loading;
        let opts = OpenOptions {
            forced: Some(enc),
            pending,
            ..OpenOptions::default()
        };
        self.mark_dirty_render();
        let job = self.jobs.spawn(
            Some(doc_id),
            eve_view::Revision::INITIAL,
            JobKind::Open {
                path: path.clone(),
                opts,
            },
        );
        self.pending.insert(
            job,
            Pending::Open {
                doc: doc_id,
                view,
                path,
                replace: None,
                opts,
                reopen: true,
            },
        );
        CommandResult::Pending(job)
    }

    /// 開き直しの取消し・失敗: 旧本文・書式・履歴をそのまま保って `Ready` に戻す。
    fn reopen_restore(&mut self, doc: DocumentId) {
        if let Some(d) = self.documents.get_mut(&doc) {
            d.load_state = crate::document::LoadState::Ready;
        }
        self.mark_dirty_render();
    }

    fn finish_reopen(
        &mut self,
        doc_id: DocumentId,
        view: ViewId,
        path: FsPath,
        opts: OpenOptions,
        r: JobResult,
    ) {
        match r {
            JobResult::Complete {
                text,
                format,
                stamp,
                changed_during_load,
                detected,
                ..
            } => {
                let encoding = format.encoding;
                let mut d = Document::from_text(text, format);
                d.path = Some(path.clone());
                d.disk_stamp = Some(stamp);
                d.read_only = detected;
                // 本文・書式・理由を一括で差し替え、履歴は新しい根 (保存点 = 根)。開き直しは Undo できない (03 §8.8)
                self.documents.insert(doc_id, d);
                if let Some(v) = self.views.get_mut(&view) {
                    v.selection = eve_text::Selection::point(eve_text::ByteOff(0));
                    v.scroll = Default::default();
                    v.goal_dcol = None;
                }
                self.mark_snapshot_update(doc_id);
                self.mark_dirty_render();
                if changed_during_load {
                    self.message(
                        MessageLevel::Warn,
                        "msg.open.changed_during_load",
                        &path.display(),
                    );
                }
                self.show_message(
                    MessageLevel::Info,
                    "info.reopen.history_cleared",
                    &[("encoding", encoding.name())],
                );
            }
            JobResult::NeedsConfirm { ask, .. } => {
                self.open_open_confirm(OpenAsk {
                    doc: doc_id,
                    view,
                    path,
                    replace: None,
                    opts,
                    ask,
                    reopen: true,
                });
            }
            JobResult::Failed { err, .. } => {
                self.reopen_restore(doc_id);
                let reason = reason_label(&err);
                let (level, key) = match err {
                    JobError::Cancelled => (MessageLevel::Info, "info.open.cancelled"),
                    JobError::Fs(e) => (MessageLevel::Error, open_error_key(&e)),
                    JobError::OutOfMemory { .. } => (MessageLevel::Error, "msg.open.out_of_memory"),
                    _ => (MessageLevel::Error, "msg.open.failed"),
                };
                self.show_message(
                    level,
                    key,
                    &[("path", &path.display()), ("reason", &reason)],
                );
            }
            // ファイルが無くなった: 旧本文のまま戻す
            _ => {
                self.reopen_restore(doc_id);
                self.show_message(
                    MessageLevel::Error,
                    "msg.open.failed",
                    &[("path", &path.display()), ("reason", "not found")],
                );
            }
        }
    }

    /// 中断された保存の痕跡の案内 (04 §15.2 の `msg.open.interrupted_*`、02 §4.5 の写像)。
    fn show_interrupted(&mut self, path: &FsPath, is: &crate::fs::InterruptedSave) {
        use crate::fs::SaveMethod;
        let key = match (is.method, is.backup.is_some(), is.complete) {
            (SaveMethod::NewFile, _, _) => "msg.open.interrupted_new",
            (_, true, true) => "msg.open.interrupted_save",
            (_, true, false) => "msg.open.interrupted_save_incomplete",
            (_, false, _) => "msg.open.interrupted_save_no_backup",
        };
        let backup = is.backup.as_ref().map(FsPath::display).unwrap_or_default();
        let tmp = is.tmp.as_ref().map(FsPath::display).unwrap_or_default();
        self.show_message(
            MessageLevel::Warn,
            key,
            &[
                ("path", &path.display()),
                ("backup", &backup),
                ("tmp", &tmp),
            ],
        );
    }

    fn complete_open(&mut self, doc: DocumentId, view: ViewId, replace: Option<DocumentId>) {
        if replace.is_some() {
            self.show_view(view, replace);
        }
        self.mark_snapshot_update(doc);
    }

    fn open_failed(
        &mut self,
        doc: DocumentId,
        replace: Option<DocumentId>,
        level: MessageLevel,
        key: &str,
        path: &FsPath,
        reason: &str,
    ) {
        self.show_message(level, key, &[("path", &path.display()), ("reason", reason)]);
        if replace.is_some() {
            // 今の文書はそのまま残す (FR-107 AC-3)
            self.close_document(doc);
        } else {
            // 最初の文書の読込に失敗: 失敗した文書は閉じ、無題の文書で始める (再試行できる、03 §9.2 の「失敗」)
            let (_, untitled) = self.add_document(Document::new_untitled());
            self.show_view(untitled, Some(doc));
        }
    }

    fn finish_check(&mut self, doc_id: DocumentId, r: JobResult) {
        let Some(doc) = self.documents.get(&doc_id) else {
            return;
        };
        let shown = doc.path.as_ref().map(FsPath::display).unwrap_or_default();
        let unchanged = match &r {
            // 読んだときの Stamp と一致し、編集もなければ書き込まない
            JobResult::Meta { stamp, .. } => {
                !doc.is_dirty()
                    && doc.disk_stamp.is_some_and(|d| {
                        d.mtime == stamp.mtime && d.len == stamp.len && d.file_id == stamp.file_id
                    })
            }
            _ => false,
        };
        match r {
            _ if unchanged => self.message(MessageLevel::Info, "info.save.unchanged", &shown),
            // NotFound は不一致として保存へ進む。その他の失敗は照合の失敗として示す
            JobResult::Failed {
                err: JobError::Fs(e),
                ..
            } if e.kind != FsErrorKind::NotFound => {
                self.show_message(
                    MessageLevel::Error,
                    "msg.save.check_failed",
                    &[("path", &shown), ("reason", &fs_reason(&e))],
                );
            }
            JobResult::Failed {
                err: JobError::Cancelled,
                ..
            } => self.message(MessageLevel::Info, "info.save.aborted", &shown),
            _ => {
                let Some((path, expected)) = self.documents.get(&doc_id).and_then(|d| {
                    Some((
                        d.path.clone()?,
                        d.disk_stamp.map_or(Expect::Absent, Expect::Stamp),
                    ))
                }) else {
                    return;
                };
                self.start_save(doc_id, path, expected, None);
            }
        }
    }

    #[allow(clippy::too_many_arguments)] // 保存ジョブの文脈 (Pending::Save) をそのまま受ける
    fn finish_save(
        &mut self,
        doc_id: DocumentId,
        at: NodeId,
        revision: eve_view::Revision,
        path: &FsPath,
        expected: Expect,
        cont: Option<Continuation>,
        r: JobResult,
    ) {
        match r {
            JobResult::Saved { report, .. } => {
                let report_method = report.method;
                if let Some(doc) = self.documents.get_mut(&doc_id) {
                    // 名前を付けて保存のパスは、書けてから文書のパスにする (03 §9.3 手順 7)
                    doc.path = Some(path.clone());
                    doc.finish_save(at, Some(report.new_stamp));
                }
                self.push_event(EditorEvent::DocumentSaved {
                    doc: doc_id,
                    revision,
                    path: path.display(),
                });
                let key = if report_method == SaveMethod::InPlace {
                    "info.save.in_place"
                } else {
                    "info.save.done"
                };
                self.message(MessageLevel::Info, key, &path.display());
                self.mark_snapshot_update(doc_id);
                // 保存の後に続ける操作 (開く・新規作成・終了)。保存が失敗したときは続けない
                // 保存の間に編集されていたら、続きの操作の前にもう一度尋ねる (03 §9.7。編集を捨てない)
                if let Some(c) = cont {
                    let name = match c {
                        Continuation::Quit => "unsaved",
                        Continuation::Reopen(_) => "unsaved_reopen",
                        _ => "unsaved_replace",
                    };
                    if !self.confirm_if_dirty(name, c.clone()) {
                        self.run_continuation(c);
                    }
                }
            }
            // 保存先の文字コードで表せない文字がある: 何も書いていない (確認ダイアログと置換は後続、03 §8.6 E3)
            JobResult::Unmappable { total, .. } => {
                self.show_message(
                    MessageLevel::Error,
                    "msg.save.failed",
                    &[
                        ("path", &path.display()),
                        ("reason", &format!("unmappable ({total})")),
                    ],
                );
            }
            // 原子的置換ができない・ハードリンク・バックアップを作れない: 確認を出す (承認の後は直接上書き、03 §9.3 手順 7)
            JobResult::Failed {
                err: JobError::Fs(ref e),
                ..
            } if fallback_kind(&e.kind, &e.detail).is_some()
                && self.documents.contains_key(&doc_id) =>
            {
                let kind =
                    fallback_kind(&e.kind, &e.detail).unwrap_or(FallbackKind::AtomicUnsupported);
                let links = match &e.detail {
                    Some(FsErrorDetail::Hardlink { nlink }) => nlink.saturating_sub(1).to_string(),
                    _ => String::new(),
                };
                let reason = fs_reason(e);
                self.open_fallback(
                    FallbackAsk {
                        kind,
                        doc: doc_id,
                        path: path.clone(),
                        expected,
                        cont,
                    },
                    &reason,
                    &links,
                );
            }
            JobResult::Failed {
                err:
                    JobError::Fs(FsError {
                        kind: FsErrorKind::InPlaceFailed { backup },
                        ..
                    }),
                ..
            } => {
                let ask = self.save_failed_ask(doc_id, path, expected, cont);
                // 直接上書きの途中の失敗: 元のファイルは壊れている可能性がある。バックアップの場所を必ず示す (05 §4.4 手順 8)
                let b = backup.as_ref().map(FsPath::display).unwrap_or_default();
                let key = if backup.is_some() {
                    "msg.save.in_place_failed"
                } else {
                    "msg.save.in_place_failed_no_backup"
                };
                self.report_save_failure(
                    ask,
                    key,
                    &[("path", &path.display()), ("backup", &b)],
                    false,
                );
            }
            JobResult::Failed { err, .. } => {
                let ask = self.save_failed_ask(doc_id, path, expected, cont);
                // 照合の失敗 (Conflict) は別の経路 (外部変更の確認、後続)。取消しは確認なしで示す
                let ask = ask.filter(|_| {
                    !matches!(
                        err,
                        JobError::Cancelled
                            | JobError::Fs(FsError {
                                kind: FsErrorKind::Conflict { .. },
                                ..
                            })
                    )
                });
                let retry = matches!(&err, JobError::Fs(e) if e.kind == FsErrorKind::Busy);
                let reason = reason_label(&err);
                let dir = match &err {
                    JobError::Fs(FsError {
                        detail: Some(FsErrorDetail::ParentMissing { dir }),
                        ..
                    }) => dir.display(),
                    _ => String::new(),
                };
                let (level, key) = match err {
                    JobError::Cancelled => (MessageLevel::Info, "info.save.aborted"),
                    JobError::Fs(e) => (MessageLevel::Error, save_error_key(&e)),
                    _ => (MessageLevel::Error, "msg.save.failed"),
                };
                let args = [
                    ("path", path.display()),
                    ("reason", reason.clone()),
                    ("dir", dir),
                ];
                let args: Vec<(&str, &str)> = args.iter().map(|(k, v)| (*k, v.as_str())).collect();
                match ask {
                    Some(ask) => self.report_save_failure(Some(ask), key, &args, retry),
                    None => self.show_message(level, key, &args),
                }
            }
            _ => {}
        }
    }
}

fn fallback_kind(kind: &FsErrorKind, detail: &Option<FsErrorDetail>) -> Option<FallbackKind> {
    match (kind, detail) {
        (FsErrorKind::AtomicUnsupported, Some(FsErrorDetail::Hardlink { .. })) => {
            Some(FallbackKind::Hardlink)
        }
        (FsErrorKind::AtomicUnsupported, _) => Some(FallbackKind::AtomicUnsupported),
        (FsErrorKind::BackupFailed, _) => Some(FallbackKind::NoBackup),
        _ => None,
    }
}

/// バイト数を「512.0 MiB」のような短い表記にする。
fn human_size(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u + 1 < UNITS.len() {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

/// `{reason}` に入れる短い分類名 (技術的な識別子。OS のエラーの分類文言 `ui.reason.*` は目録ができてから)。
fn reason_label(err: &JobError) -> String {
    match err {
        JobError::Fs(e) => fs_reason(e),
        JobError::Panic => "panic".to_owned(),
        JobError::OutOfMemory { .. } => "out of memory".to_owned(),
        JobError::Cancelled => "cancelled".to_owned(),
    }
}

fn fs_reason(e: &FsError) -> String {
    format!("{:?}", e.kind)
        .split([' ', '{'])
        .next()
        .unwrap_or("unknown")
        .to_lowercase()
}

/// 開く失敗の MessageKey (04 §15.2)。
fn open_error_key(e: &FsError) -> &'static str {
    match (&e.kind, &e.detail) {
        (FsErrorKind::PermissionDenied, _) => "msg.open.permission_denied",
        (FsErrorKind::Busy, _) => "msg.open.busy",
        (_, Some(FsErrorDetail::NotAFile)) => "msg.open.not_a_file",
        (_, Some(FsErrorDetail::ReservedName)) => "msg.open.reserved_name",
        _ => "msg.open.failed",
    }
}

/// 保存失敗の MessageKey (04 §15.2)。失敗の後の選択肢 (確認ダイアログ) は後続。
fn save_error_key(e: &FsError) -> &'static str {
    match (&e.kind, &e.detail) {
        (FsErrorKind::PermissionDenied, _) => "msg.save.permission_denied",
        (FsErrorKind::DiskFull, _) => "msg.save.disk_full",
        (FsErrorKind::ReadOnlyFs, _) => "msg.save.read_only_fs",
        (FsErrorKind::InvalidPath, _) => "msg.save.invalid_path",
        (FsErrorKind::Busy, _) => "msg.save.busy",
        (FsErrorKind::NotFound, Some(FsErrorDetail::ParentMissing { .. })) => {
            "msg.save.parent_missing"
        }
        _ => "msg.save.failed",
    }
}

// ---------- コマンド ----------

fn arg_path(ctx: &CommandCtx<'_>) -> Option<String> {
    ctx.args.str("path").map(str::to_owned)
}

fn file_new(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.new_untitled()
}

fn file_open(ctx: &mut CommandCtx<'_>) -> CommandResult {
    match arg_path(ctx) {
        Some(p) => ctx.state.open_path(&p, OpenOptions::default()),
        // パスがなければ入力プロンプトで尋ねる
        None => {
            ctx.state.open_prompt(PromptKind::Open, String::new());
            CommandResult::Done
        }
    }
}

fn file_reopen_with_encoding(ctx: &mut CommandCtx<'_>) -> CommandResult {
    let enc = match ctx.args.str("encoding") {
        Some(name) => match Encoding::from_label(name) {
            Some(e) => Some(e),
            None => return CommandResult::Rejected(RejectReason::InvalidArgs),
        },
        None => None,
    };
    ctx.state.reopen_with_encoding(enc)
}

fn file_save(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.request_save()
}

fn file_save_as(ctx: &mut CommandCtx<'_>) -> CommandResult {
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

macro_rules! reg {
    ($r:expr, $id:expr, $kind:expr, $when:expr, $h:expr) => {
        $r.register(
            CommandSpec::new($id, $kind, Permission::None, $when, $h)
                .expect("カタログの when 句は構文が正しい"),
        )
    };
}

/// v0.1 のファイルコマンドのうち、確認 UI なしで動くもの (07 §11.6 の「ファイル・アプリ」)。
pub(super) fn register(r: &mut CommandRegistry) {
    reg!(r, "file.new", CommandKind::File, "", file_new);
    reg!(r, "file.open", CommandKind::File, "", file_open);
    reg!(
        r,
        "file.save",
        CommandKind::File,
        "!doc.read_only",
        file_save
    );
    reg!(r, "file.save_as", CommandKind::File, "", file_save_as);
    reg!(
        r,
        "file.reopen_with_encoding",
        CommandKind::File,
        "doc.has_path",
        file_reopen_with_encoding
    );
}
