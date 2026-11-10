//! 保存 (docs/03 §9.3): 上書き・名前を付けて保存・未編集の保存の照合 (手順 0)・保存の失敗の選択肢・直接上書きへの退路・
//! 読み取り専用の解除・保存の警告 (STR-B1 で io/file.rs から分けた)。

use eve_view::{DocumentId, EditorEvent, MessageLevel};

use crate::document::NodeId;
use crate::io::fs::{
    Expect, FsError, FsErrorDetail, FsErrorKind, HardlinkPolicy, InPlaceBackup, SaveMethod,
};
use crate::io::fspath::FsPath;
use crate::jobs::{JobError, JobKind, JobResult, SaveStrategy};
use crate::ui_model::keys;

use super::file::*;
use super::open::*;
use super::pending::*;
use super::unmappable::*;
use crate::command::{CommandResult, RejectReason};
use crate::editor::state::EditorState;
use crate::ui_model::overlay::{Continuation, PromptKind};

/// 保存の失敗の後の選択 (`ui.confirm.save_failed`、03 §9.7 (e))。
#[derive(Clone, Debug)]
pub(crate) struct SaveFailedAsk {
    pub doc: DocumentId,
    pub path: FsPath,
    pub expected: Expect,
    pub cont: Option<Continuation>,
    /// 失敗した保存の時点の本文 (確認の間は編集できない)。「退避先に保存」で書く。
    pub snapshot: std::sync::Arc<crate::DocSnapshot>,
    pub fmt: Option<NewFormat>,
}

/// 保存の代替の確認 (`AtomicUnsupported` / ハードリンク / `BackupFailed`、03 §9.3 手順 7・05 §4.4)。
#[derive(Clone, Debug)]
pub(crate) struct FallbackAsk {
    pub kind: FallbackKind,
    pub doc: DocumentId,
    pub path: FsPath,
    pub expected: Expect,
    pub cont: Option<Continuation>,
    pub fmt: Option<NewFormat>,
    /// 競合 (`Conflict`) の確認のとき、照合で見つけた今の保存先の状態 (`None` = ファイルが無い)。「上書き」はこれを新しい期待値にする。
    pub conflict_current: Option<crate::FileStamp>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FallbackKind {
    /// 開いた後に既存のファイルが外部で変更された (`ui.confirm.save.conflict_changed`、03 §9.5)。選択肢: 上書き / 別名で保存 (既定)。
    ConflictChanged,
    /// 開いた後に既存のファイルが外部で削除された (`ui.confirm.save.conflict_deleted`)。「上書き」はファイルを作り直す。
    ConflictDeleted,
    /// 新規ファイルの保存先に、同名のファイルが後から作られていた (`ui.confirm.save.conflict_created`)。
    ConflictCreated,
    /// 原子的置換ができない (`ui.confirm.save.atomic_unsupported`)。選択肢: 直接上書き / 別名で保存 (既定)。
    AtomicUnsupported,
    /// ハードリンク (`ui.confirm.save.hardlink`)。選択肢: リンクを保って直接上書き / 切り離して保存。既定はキャンセル。
    Hardlink,
    /// 事前バックアップ・保存中フラグを書けなかった (`ui.confirm.save.overwrite_without_backup`)。選択肢: バックアップなしで上書き / 別名で保存。既定はキャンセル。
    NoBackup,
}

impl EditorState {
    /// 確認で「保存」を選んだ後: `doc_id` を保存し、成功したら `cont` を続ける。パスがなければ名前を尋ねてから保存する。
    /// 対象はアクティブ文書ではなく `doc_id` (保存の連鎖は保存した文書に属する、03 §9.8)。
    pub(crate) fn save_doc_then(
        &mut self,
        doc_id: DocumentId,
        cont: Continuation,
    ) -> CommandResult {
        let Some(doc) = self.document(doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        // 保存中なら、その保存の完了後に続ける (二重に保存しない)
        let in_flight = self.io.pending.iter().find_map(|(job, p)| match p {
            Pending::Save { doc, .. } if *doc == doc_id => Some(*job),
            _ => None,
        });
        if let Some(job) = in_flight {
            self.ui.continuations.insert(job, cont);
            return CommandResult::Pending(job);
        }
        let Some(path) = doc.path.clone() else {
            self.open_prompt(PromptKind::SaveAs { then: Some(cont) }, String::new());
            return CommandResult::Done;
        };
        if doc.load_state != crate::document::LoadState::Ready {
            self.message(MessageLevel::Info, keys::INFO_RELOAD_BUSY, &path.display());
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        // 読み取り専用 (`--readonly`・書込権限なし・不正バイト等) の文書は同じパスへ書かず、名前を尋ねる
        // (03 §9.7 の AskUnsaved の行。docs/review/v0.3-findings.md の SAF-18)
        if !doc.read_only.is_empty() {
            self.open_prompt(PromptKind::SaveAs { then: Some(cont) }, path.display());
            return CommandResult::Done;
        }
        let expected = doc.disk_stamp.map_or(Expect::Absent, Expect::Stamp);
        self.start_save(doc_id, path, expected, Some(cont))
    }

    /// 名前を付けて保存 (パスを指定する)。
    pub(crate) fn request_save_as(
        &mut self,
        raw_path: &str,
        cont: Option<Continuation>,
    ) -> CommandResult {
        self.request_save_as_fmt(raw_path, cont, None)
    }

    /// `request_save_as` に新しい書式を渡す版 (`file.set_encoding` の無題・読み取り専用の文書、03 §8.8)。
    pub(crate) fn request_save_as_fmt(
        &mut self,
        raw_path: &str,
        cont: Option<Continuation>,
        fmt: Option<NewFormat>,
    ) -> CommandResult {
        let Some(doc_id) = self.active_document() else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Ok(path) = self.paths.normalize(raw_path) else {
            self.show_message(
                MessageLevel::Error,
                keys::MSG_SAVE_INVALID_PATH,
                &[("path", raw_path), ("reason", "invalid path")],
            );
            return CommandResult::Rejected(RejectReason::InvalidArgs);
        };
        let Some(doc) = self.docs.documents.get(&doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        // 読込中・読込に失敗した文書は保存できない (中身が揃っていない)
        if doc.load_state != crate::document::LoadState::Ready {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        if self.save_in_progress(doc_id) {
            self.message(
                MessageLevel::Info,
                keys::INFO_SAVE_IN_PROGRESS,
                &path.display(),
            );
            return CommandResult::Done;
        }
        let same = doc.path.as_ref() == Some(&path);
        // 読み取り専用の文書を同じファイルへ書くと、元のバイト列を損なうことがある (03 §9.7 (a) の 1)
        if same && !doc.is_editable() {
            self.message(
                MessageLevel::Warn,
                keys::MSG_SAVE_AS_SAME_FILE_READ_ONLY,
                &path.display(),
            );
            return CommandResult::Rejected(RejectReason::ReadOnly);
        }
        // 同じファイルなら読んだときの Stamp で照合する。別のファイルへは存在しないことを期待する
        // (既存なら Conflict。上書きの確認は確認ダイアログの仕事)。パスと Stamp は成功してから更新する
        let expected = if same {
            doc.disk_stamp.map_or(Expect::Absent, Expect::Stamp)
        } else {
            Expect::Absent
        };
        self.start_save_fmt(doc_id, path, expected, cont, fmt)
    }

    pub(crate) fn start_save(
        &mut self,
        doc_id: DocumentId,
        path: FsPath,
        expected: Expect,
        cont: Option<Continuation>,
    ) -> CommandResult {
        self.start_save_fmt(doc_id, path, expected, cont, None)
    }

    /// 新しい書式 `fmt` を保存に渡す (`file.set_encoding`)。成功したときだけ文書の書式に確定する。
    pub(crate) fn start_save_fmt(
        &mut self,
        doc_id: DocumentId,
        path: FsPath,
        expected: Expect,
        cont: Option<Continuation>,
        fmt: Option<NewFormat>,
    ) -> CommandResult {
        self.start_save_with(doc_id, path, expected, cont, SaveStrategy::default(), fmt)
    }

    /// 保存方式を指定して保存ジョブを投入する (確認の後の再投入で直接上書きにする)。
    pub(crate) fn start_save_with(
        &mut self,
        doc_id: DocumentId,
        path: FsPath,
        expected: Expect,
        cont: Option<Continuation>,
        strategy: SaveStrategy,
        fmt: Option<NewFormat>,
    ) -> CommandResult {
        // ダングリングのリンクのリンク先への作成は、承認した保存の 1 回だけ
        let create_link_target = self
            .io
            .link_create_ok
            .take_if(|(d, p)| *d == doc_id && *p == path)
            .is_some();
        let Some(doc) = self.docs.documents.get_mut(&doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        // 読込中 (開き直し・再読込の途中を含む) は保存しない。旧本文の保存の結果が新しい文書の保存点と照合を狂わせる
        // (保存の経路はすべてここを通る。コマンドの when 句を経ない確認の「保存」からも。docs/review/v0.3-findings.md の SAF-17)
        if doc.load_state != crate::document::LoadState::Ready {
            let shown = path.display();
            self.message(MessageLevel::Info, keys::INFO_RELOAD_BUSY, &shown);
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        // 読み取り専用属性を解除した文書の上書き保存は、「読み取り専用を解除して保存」の確認を経る (FR-116、03 §9.3 手順 1)。
        // 承認した Revision では繰り返さない
        let clear_readonly = doc.attribute_released && doc.path.as_ref() == Some(&path);
        if clear_readonly && self.io.readonly_ok != Some((doc_id, doc.revision())) {
            let ask = LossyAsk {
                doc: doc_id,
                path,
                expected,
                cont,
                strategy,
                fmt,
            };
            self.open_clear_readonly(ask);
            return CommandResult::Done;
        }
        let out_encoding = fmt.map_or(doc.format.encoding, |f| f.encoding);
        // 区分 B / D: 元の文字コードのまま、別のバイト表現で保存される箇所がある文書を編集して保存するときは、
        // 書き込む前に確認する (FR-112 AC-4 (a)・NFR-303、03 §8.6 E2')。無編集の上書き保存は書き込まないので出ない。
        // 承認した Revision では繰り返さない
        if out_encoding == doc.format.encoding
            && doc.format.roundtrip.lossy_lines > 0
            && doc.is_dirty()
            && self.io.lossy_ok != Some((doc_id, doc.revision()))
        {
            let ask = LossyAsk {
                doc: doc_id,
                path,
                expected,
                cont,
                strategy,
                fmt,
            };
            self.open_lossy(ask);
            return CommandResult::Done;
        }
        // 状態付き符号化 (ISO-2022-JP) で不正バイトを含む文書は保存しない (03 §8.3 (a)。不正バイトを失わないため)
        if out_encoding.is_stateful() && doc.format.invalid_bytes > 0 {
            self.show_message(
                MessageLevel::Error,
                keys::MSG_SAVE_ENCODING_UNSUPPORTED_ISO2022,
                &[("path", &path.display())],
            );
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        let (encoding, bom, escape) = (
            out_encoding,
            fmt.map_or(doc.format.has_bom, |f| f.bom),
            doc.format.save_escape_policy(out_encoding),
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
                escape,
                expected,
                strategy,
                clear_readonly,
                create_link_target,
            },
        );
        if let Some(c) = cont {
            self.ui.continuations.insert(job, c);
        }
        self.io.pending.insert(
            job,
            Pending::Save {
                doc: doc_id,
                at,
                revision,
                path,
                expected,
                fmt,
            },
        );
        CommandResult::Pending(job)
    }

    /// 閉じた文書の保存の結果が届いた: 公開物を外す。保存が成功していなければ、内容を失わないよう退避先に書く
    /// (閉じた文書なので確認は出さない。書いた場所は `finish_write_recovery` のメッセージで示す)。
    pub(crate) fn release_closing_snapshot(&mut self, doc: DocumentId, path: &FsPath, saved: bool) {
        if self.docs.documents.contains_key(&doc) {
            return;
        }
        if saved {
            self.io.closing_snapshots.remove(&doc);
            self.mark_snapshot_update(doc);
            return;
        }
        // 失敗: 退避先への書込が終わるまで、公開物は緊急退避に残す
        let Some(snapshot) = self.io.closing_snapshots.get(&doc).cloned() else {
            return;
        };
        let job = self.jobs.spawn(
            Some(doc),
            snapshot.revision,
            JobKind::WriteRecovery {
                doc: std::sync::Arc::new(snapshot),
            },
        );
        self.io.pending.insert(
            job,
            Pending::WriteRecovery {
                path: path.clone(),
                release: Some(doc),
            },
        );
    }

    /// 保存の失敗の確認に渡す材料 (文書が無い・読込中なら `None`)。
    pub(crate) fn save_failed_ask(
        &self,
        doc: DocumentId,
        path: &FsPath,
        expected: Expect,
        fmt: Option<NewFormat>,
        cont: Option<Continuation>,
    ) -> Option<SaveFailedAsk> {
        // 閉じた文書には確認を出さない (保存の結果の処理が退避先に書く)
        self.document(doc)?;
        let snapshot = self.rescue_snapshot(doc, false)?;
        Some(SaveFailedAsk {
            doc,
            path: path.clone(),
            expected,
            cont,
            snapshot: std::sync::Arc::new(snapshot),
            fmt,
        })
    }

    /// 保存の失敗を `ui.confirm.save_failed` で示す (03 §9.7 (e))。本文にメッセージ `key` の text を入れる。
    /// 材料が無ければ (文書が閉じている等) メッセージだけ示す。`retry` は Busy のときだけ (何も書いていないので照合は通る)。
    pub(crate) fn report_save_failure(
        &mut self,
        ask: Option<SaveFailedAsk>,
        key: &str,
        args: &[(&str, &str)],
        retry: bool,
    ) {
        use crate::ui_model::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
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
            details: Vec::new(),
            default,
            focused: default,
            action: ConfirmAction::SaveFailed(ask),
        });
    }

    /// 保存の失敗の確認の選択 (キャンセルは呼び側が処理済み)。`id` は選択肢の ID。
    pub(crate) fn choose_save_failed(&mut self, ask: SaveFailedAsk, id: &str) -> CommandResult {
        let SaveFailedAsk {
            doc,
            path,
            expected,
            cont,
            snapshot,
            fmt,
        } = ask;
        match id {
            "retry" => self.start_save_fmt(doc, path, expected, cont, fmt),
            "recovery" => {
                // 続きの操作 (終了・置き換え) は行わない。書いた場所は結果のメッセージで示す
                let job = self.jobs.spawn(
                    Some(doc),
                    snapshot.revision,
                    JobKind::WriteRecovery { doc: snapshot },
                );
                self.io.pending.insert(
                    job,
                    Pending::WriteRecovery {
                        path,
                        release: None,
                    },
                );
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
    pub(crate) fn finish_write_recovery(&mut self, path: &FsPath, r: JobResult) {
        match r {
            JobResult::RecoveryWritten { path: target, .. } => self.show_message(
                MessageLevel::Info,
                keys::INFO_SAVE_TO_RECOVERY,
                &[("target", &target.display()), ("path", &path.display())],
            ),
            JobResult::Failed { err, .. } => {
                let reason = reason_label(&err);
                self.show_message(
                    MessageLevel::Error,
                    keys::MSG_SAVE_RECOVERY_FAILED,
                    &[("reason", &reason), ("path", &path.display())],
                );
            }
            _ => {}
        }
    }

    /// 保存の代替の確認を出す。
    pub(crate) fn open_fallback(&mut self, ask: FallbackAsk, reason: &str, links: &str) {
        use crate::ui_model::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
        let choice = |id: &'static str, accel: char| ChoiceDef {
            id,
            accel: Some(accel),
            destructive: false,
        };
        let (name, choices, default): (&'static str, Vec<ChoiceDef>, usize) = match ask.kind {
            FallbackKind::ConflictChanged => (
                "save.conflict_changed",
                vec![choice("overwrite", 'o'), choice("alt", 'a')],
                1,
            ),
            FallbackKind::ConflictDeleted => (
                "save.conflict_deleted",
                vec![choice("overwrite", 'o'), choice("alt", 'a')],
                1,
            ),
            FallbackKind::ConflictCreated => (
                "save.conflict_created",
                vec![choice("overwrite", 'o'), choice("alt", 'a')],
                1,
            ),
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
            details: Vec::new(),
            default,
            focused: default,
            action: ConfirmAction::Fallback(ask),
        });
    }

    /// 「読み取り専用を解除して保存」の確認を出す。選択肢: 別名で保存 (既定) / 読み取り専用を解除して保存 (04 §15.2)。
    pub(crate) fn open_clear_readonly(&mut self, ask: LossyAsk) {
        use crate::ui_model::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
        let choice = |id: &'static str, accel: char| ChoiceDef {
            id,
            accel: Some(accel),
            destructive: false,
        };
        self.open_confirm(ConfirmState {
            name: "save.read_only",
            args: vec![("path".to_owned(), ask.path.display())],
            choices: vec![choice("alt", 'a'), choice("write", 'w')],
            details: Vec::new(),
            default: 0,
            focused: 0,
            action: ConfirmAction::ClearReadOnly(ask),
        });
    }

    /// 「読み取り専用を解除して保存」の確認の選択 (キャンセルは呼び側が処理済み)。
    pub(crate) fn choose_clear_readonly(&mut self, ask: LossyAsk, choice: usize) -> CommandResult {
        let LossyAsk {
            doc,
            path,
            expected,
            cont,
            strategy,
            fmt,
        } = ask;
        if choice == 1 {
            if let Some(d) = self.docs.documents.get(&doc) {
                self.io.readonly_ok = Some((doc, d.revision()));
            }
            return self.start_save_with(doc, path, expected, cont, strategy, fmt);
        }
        self.open_prompt(PromptKind::SaveAs { then: cont }, String::new());
        CommandResult::Done
    }

    /// 保存の代替の確認の選択 (キャンセルは呼び側が処理済み)。
    pub(crate) fn choose_fallback(&mut self, ask: FallbackAsk, choice: usize) -> CommandResult {
        let FallbackAsk {
            kind,
            doc,
            path,
            expected,
            cont,
            fmt,
            conflict_current,
        } = ask;
        // 競合の「上書き」: 照合で見つけた今の状態を新しい期待値にして、通常の保存を投入し直す (03 §9.5)
        let expected = match kind {
            FallbackKind::ConflictChanged
            | FallbackKind::ConflictDeleted
            | FallbackKind::ConflictCreated => {
                conflict_current.map_or(Expect::Absent, Expect::Stamp)
            }
            _ => expected,
        };
        let in_place = |backup| SaveStrategy::InPlace { backup };
        let strategy = match (kind, choice) {
            (
                FallbackKind::ConflictChanged
                | FallbackKind::ConflictDeleted
                | FallbackKind::ConflictCreated,
                0,
            ) => Some(SaveStrategy::default()),
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
            Some(s) => self.start_save_with(doc, path, expected, cont, s, fmt),
            None => {
                self.open_prompt(PromptKind::SaveAs { then: cont }, String::new());
                CommandResult::Done
            }
        }
    }

    pub(crate) fn finish_check(&mut self, doc_id: DocumentId, r: JobResult) {
        let Some(doc) = self.docs.documents.get(&doc_id) else {
            return;
        };
        let shown = doc.path.as_ref().map(FsPath::display).unwrap_or_default();
        let unchanged = match &r {
            // 読んだときの Stamp と一致し、編集もなければ書き込まない
            JobResult::Meta { stamp, .. } => {
                !doc.is_dirty() && doc.disk_stamp.is_some_and(|d| same_on_disk(&d, stamp))
            }
            _ => false,
        };
        match r {
            _ if unchanged => self.message(MessageLevel::Info, keys::INFO_SAVE_UNCHANGED, &shown),
            // NotFound は不一致として保存へ進む。その他の失敗は照合の失敗として示す
            JobResult::Failed {
                err: JobError::Fs(e),
                ..
            } if e.kind != FsErrorKind::NotFound => {
                self.show_message(
                    MessageLevel::Error,
                    keys::MSG_SAVE_CHECK_FAILED,
                    &[("path", &shown), ("reason", &fs_reason(&e))],
                );
            }
            JobResult::Failed {
                err: JobError::Cancelled,
                ..
            } => self.message(MessageLevel::Info, keys::INFO_SAVE_ABORTED, &shown),
            _ => {
                let Some((path, expected)) = self.docs.documents.get(&doc_id).and_then(|d| {
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
    pub(crate) fn finish_save(
        &mut self,
        doc_id: DocumentId,
        at: NodeId,
        revision: eve_view::Revision,
        path: &FsPath,
        expected: Expect,
        fmt: Option<NewFormat>,
        cont: Option<Continuation>,
        r: JobResult,
    ) {
        match r {
            JobResult::Saved { report, .. } => {
                let report_method = report.method;
                if let Some(doc) = self.docs.documents.get_mut(&doc_id) {
                    // `file.set_encoding`: 保存が成功したので新しい書式に確定する。往復の検査結果は旧文字コードのものなので捨てる
                    // (書いた内容を再エンコードすれば同じバイト列になる、03 §8.8)
                    if let Some(f) = fmt {
                        doc.format.encoding = f.encoding;
                        doc.format.has_bom = f.bom;
                        doc.format.roundtrip = Default::default();
                    }
                    // 名前を付けて保存 (別のパスへ書いた) では、元のファイルに由来する読み取り専用の理由
                    // (属性・書込権限なし) を外す。書き込めた保存先の文書は編集できる (FR-116 AC-4、03 §9.6)。
                    // `--readonly` と内容に由来する理由 (不正バイト・バイナリ・長大行) は残る
                    if doc.path.as_ref() != Some(path) {
                        doc.attribute_released = false;
                        doc.read_only
                            .remove(crate::document::ReadOnlyReason::FileAttribute);
                        doc.read_only
                            .remove(crate::document::ReadOnlyReason::NoWritePermission);
                    }
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
                    keys::INFO_SAVE_IN_PLACE
                } else {
                    keys::INFO_SAVE_DONE
                };
                self.message(MessageLevel::Info, key, &path.display());
                self.show_save_warning(path, &report.warnings);
                self.mark_snapshot_update(doc_id);
                // 保存の後に続ける操作 (開く・新規作成・終了)。保存が失敗したときは続けない
                // 保存の間に編集されていたら、続きの操作の前にもう一度尋ねる (03 §9.7。編集を捨てない)
                if let Some(c) = cont {
                    if c == Continuation::Quit {
                        // 終了の連鎖: 保存した文書が clean なら次の文書へ、保存中の編集で dirty なら同じ文書をもう一度尋ねる
                        self.advance_quit_chain();
                        return;
                    }
                    let name = match c {
                        Continuation::Reopen(..) => "unsaved_reopen",
                        Continuation::Close(_) => "unsaved_close",
                        _ => "unsaved",
                    };
                    if !self.confirm_doc_if_dirty(doc_id, name, c.clone()) {
                        self.run_continuation(c);
                    }
                }
            }
            // 保存先の文字コードで表せない文字がある: 何も書いていない。E3 の選択肢を出す (03 §8.6)
            JobResult::Unmappable {
                total,
                recorded,
                encoding,
                ..
            } if self.docs.documents.contains_key(&doc_id) && !recorded.is_empty() => {
                // 該当箇所 (先頭 100 件) を強調する。次の編集まで残す (03 §8.6)
                let indices: Vec<u64> = recorded.iter().map(|u| u.char_index).collect();
                self.mark_unmappable(doc_id, &indices);
                self.open_unmappable(
                    UnmappableAsk {
                        doc: doc_id,
                        path: path.clone(),
                        expected,
                        cont,
                        first_char: recorded[0].char_index,
                        encoding,
                        fmt,
                    },
                    total,
                    encoding,
                    recorded.iter().any(|u| is_escape_char(u.ch)),
                );
            }
            JobResult::Unmappable { total, .. } => {
                self.show_message(
                    MessageLevel::Error,
                    keys::MSG_SAVE_FAILED,
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
                && self.docs.documents.contains_key(&doc_id) =>
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
                        fmt,
                        conflict_current: None,
                    },
                    &reason,
                    &links,
                );
            }
            // 保存の直前の照合で、開いた後に保存先が変わっていた: 書き込まずに「上書き / 別名で保存 / 中止」を尋ねる
            // (FR-117 AC-1、03 §9.5)
            JobResult::Failed {
                err:
                    JobError::Fs(FsError {
                        kind: FsErrorKind::Conflict { current },
                        ..
                    }),
                ..
            } if self.docs.documents.contains_key(&doc_id) => {
                let kind = match (expected, current) {
                    (Expect::Absent, _) => FallbackKind::ConflictCreated,
                    (_, None) => FallbackKind::ConflictDeleted,
                    _ => FallbackKind::ConflictChanged,
                };
                self.open_fallback(
                    FallbackAsk {
                        kind,
                        doc: doc_id,
                        path: path.clone(),
                        expected,
                        cont,
                        fmt,
                        conflict_current: current,
                    },
                    "",
                    "",
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
                let ask = self.save_failed_ask(doc_id, path, expected, fmt, cont);
                // 直接上書きの途中の失敗: 元のファイルは壊れている可能性がある。バックアップの場所を必ず示す (05 §4.4 手順 8)
                let b = backup.as_ref().map(FsPath::display).unwrap_or_default();
                let key = if backup.is_some() {
                    keys::MSG_SAVE_IN_PLACE_FAILED
                } else {
                    keys::MSG_SAVE_IN_PLACE_FAILED_NO_BACKUP
                };
                self.report_save_failure(
                    ask,
                    key,
                    &[("path", &path.display()), ("backup", &b)],
                    false,
                );
            }
            JobResult::Failed { err, .. } => {
                // 上書き保存が権限エラーで失敗したら、文書を読み取り専用 (書込権限なし) に切り替える。編集した内容は失われず、
                // 名前を付けて保存で書ける (FR-116 AC-3、03 §9.6)。名前を付けて保存の失敗では付けない (失敗したのは別のパス)
                if matches!(&err, JobError::Fs(e) if e.kind == FsErrorKind::PermissionDenied)
                    && let Some(doc) = self.docs.documents.get_mut(&doc_id)
                    && doc.path.as_ref() == Some(path)
                {
                    doc.read_only
                        .insert(crate::document::ReadOnlyReason::NoWritePermission);
                    self.mark_dirty_render();
                }
                let ask = self.save_failed_ask(doc_id, path, expected, fmt, cont);
                // 照合の失敗 (Conflict) は別の経路 (外部変更の確認)。取消しは確認なしで示す
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
                let err_backup = match &err {
                    JobError::Fs(FsError {
                        kind: FsErrorKind::ReplacedMovedToBackup { backup },
                        ..
                    }) => Some(backup.clone()),
                    _ => None,
                };
                let reason = reason_label(&err);
                let dir = match &err {
                    JobError::Fs(FsError {
                        detail: Some(FsErrorDetail::ParentMissing { dir }),
                        ..
                    }) => dir.display(),
                    _ => String::new(),
                };
                // ダングリングリンク: リンク先を示す (FR-106 AC-2)
                let link_target = match &err {
                    JobError::Fs(FsError {
                        detail: Some(FsErrorDetail::DanglingLink { target }),
                        ..
                    }) => target.display(),
                    _ => String::new(),
                };
                let (level, key) = match err {
                    JobError::Cancelled => (MessageLevel::Info, keys::INFO_SAVE_ABORTED),
                    JobError::Fs(e) => (MessageLevel::Error, save_error_key(&e)),
                    _ => (MessageLevel::Error, keys::MSG_SAVE_FAILED),
                };
                let backup = match &err_backup {
                    Some(b) => b.display(),
                    None => String::new(),
                };
                let args = [
                    ("path", path.display()),
                    ("reason", reason.clone()),
                    ("dir", dir),
                    ("backup", backup),
                    ("target", link_target.clone()),
                ];
                let args: Vec<(&str, &str)> = args.iter().map(|(k, v)| (*k, v.as_str())).collect();
                match ask {
                    // リンク先が無いリンク: 失敗の選択ではなく、リンク先に作るかを尋ねる (v0.2)
                    Some(ask) if !link_target.is_empty() => {
                        self.open_dangling_link(ask, link_target)
                    }
                    Some(ask) => self.report_save_failure(Some(ask), key, &args, retry),
                    None => self.show_message(level, key, &args),
                }
            }
            _ => {}
        }
    }
}

pub(crate) fn fallback_kind(
    kind: &FsErrorKind,
    detail: &Option<FsErrorDetail>,
) -> Option<FallbackKind> {
    match (kind, detail) {
        (FsErrorKind::AtomicUnsupported, Some(FsErrorDetail::Hardlink { .. })) => {
            Some(FallbackKind::Hardlink)
        }
        (FsErrorKind::AtomicUnsupported, _) => Some(FallbackKind::AtomicUnsupported),
        (FsErrorKind::BackupFailed, _) => Some(FallbackKind::NoBackup),
        _ => None,
    }
}

impl EditorState {
    /// 保存は成功したが知らせること (`SaveReport.warnings`) のうち、最も重い 1 つを警告として出す (03 §9.3 手順 6、04 §15.2)。
    /// 黙って失わない (02 §4.5。docs/review/v0.3-findings.md の SAF-05)。
    pub(crate) fn show_save_warning(&mut self, path: &FsPath, warnings: &[crate::SaveWarning]) {
        use crate::SaveWarning as W;
        let rank = |w: &W| match w {
            // 一時ファイルの残りは次の保存のハードリンクの確認の原因になるので、隠さない (docs/review/v0.3-findings.md の SAF-40)
            W::TempNotRemoved { .. } => 0,
            W::FsyncFailed => 1,
            W::ChangedAfterSave => 2,
            W::BackupNotRemoved { .. } => 3,
            W::PermissionNarrowed => 4,
            W::AttrsNotPreserved { .. } => 5,
            W::InflightFlagNotWritten => 6,
        };
        let Some(w) = warnings.iter().min_by_key(|w| rank(w)) else {
            return;
        };
        let p = path.display();
        match w {
            W::FsyncFailed => self.show_message(
                MessageLevel::Warn,
                keys::MSG_SAVE_FSYNC_FAILED,
                &[("path", &p)],
            ),
            W::ChangedAfterSave => {
                self.show_message(
                    MessageLevel::Warn,
                    keys::MSG_SAVE_CHANGED_AFTER_SAVE,
                    &[("path", &p)],
                );
            }
            W::BackupNotRemoved { path: backup } => self.show_message(
                MessageLevel::Warn,
                keys::MSG_SAVE_BACKUP_NOT_REMOVED,
                &[("path", &p), ("backup", &backup.display())],
            ),
            W::TempNotRemoved { path: tmp } => self.show_message(
                MessageLevel::Warn,
                keys::MSG_SAVE_TMP_NOT_REMOVED,
                &[("path", &p), ("tmp", &tmp.display())],
            ),
            W::PermissionNarrowed => {
                self.show_message(
                    MessageLevel::Warn,
                    keys::MSG_SAVE_PERMISSION_NARROWED,
                    &[("path", &p)],
                );
            }
            W::AttrsNotPreserved { what } => {
                let reason = what
                    .iter()
                    .map(|k| match k {
                        crate::AttrKind::FileAttributes => "attributes",
                        crate::AttrKind::Security => "security",
                        crate::AttrKind::StreamsXattr => "streams/xattr",
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                self.show_message(
                    MessageLevel::Warn,
                    keys::MSG_SAVE_ATTRIBUTES_NOT_COPIED,
                    &[("path", &p), ("reason", &reason)],
                );
            }
            W::InflightFlagNotWritten => self.show_message(
                MessageLevel::Warn,
                keys::MSG_SAVE_INFLIGHT_FLAG_NOT_WRITTEN,
                &[("path", &p)],
            ),
        }
    }
}

/// 保存失敗の `MessageKey` (04 §15.2)。失敗の後の選択肢 (確認ダイアログ) は別に出す。
pub(crate) fn save_error_key(e: &FsError) -> &'static str {
    match (&e.kind, &e.detail) {
        (FsErrorKind::PermissionDenied, _) => keys::MSG_SAVE_PERMISSION_DENIED,
        (FsErrorKind::DiskFull, _) => keys::MSG_SAVE_DISK_FULL,
        (FsErrorKind::ReadOnlyFs, _) => keys::MSG_SAVE_READ_ONLY_FS,
        (FsErrorKind::InvalidPath, _) => keys::MSG_SAVE_INVALID_PATH,
        (FsErrorKind::Busy, _) => keys::MSG_SAVE_BUSY,
        // 置換の途中で失敗し、元のファイルは退避名に残っている (NFR-301 AC-2)。そのパスを必ず示す
        (FsErrorKind::ReplacedMovedToBackup { .. }, _) => keys::MSG_SAVE_MOVED_TO_BACKUP,
        (FsErrorKind::NotFound, Some(FsErrorDetail::ParentMissing { .. })) => {
            keys::MSG_SAVE_PARENT_MISSING
        }
        // リンク先が無いシンボリックリンク: 書かずに、リンク先を示して知らせる (FR-106 AC-2)
        (FsErrorKind::NotFound, Some(FsErrorDetail::DanglingLink { .. })) => {
            keys::MSG_SAVE_DANGLING_LINK
        }
        _ => keys::MSG_SAVE_FAILED,
    }
}

// ---------- コマンド ----------

#[cfg(test)]
mod save_error_tests {
    use super::*;

    // verifies: AC-FR-106-2
    #[test]
    fn ダングリングリンクは専用のメッセージになる() {
        let e = FsError {
            kind: FsErrorKind::NotFound,
            detail: Some(FsErrorDetail::DanglingLink {
                target: crate::FsPath::from(r"C:\gone\a.txt"),
            }),
            source: None,
        };
        assert_eq!(save_error_key(&e), "msg.save.dangling_link");
    }
}
