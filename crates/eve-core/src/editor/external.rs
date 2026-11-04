//! 外部変更の確認 (FR-117、docs/03 §9.5 の「M9d の実装」): フォーカス復帰 (`InputEvent::Focus(true)`) と
//! `file.check_external_changes`。表示中の文書のメタ情報をジョブで取り、読んだ (書いた) ときの Stamp と比べる。

use eve_view::{DocumentId, MessageLevel};

use super::command::{
    CommandCtx, CommandKind, CommandRegistry, CommandResult, CommandSpec, Permission, RejectReason,
};
use super::file::{Pending, fs_reason, same_on_disk};
use super::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
use super::state::EditorState;
use crate::FsPath;
use crate::document::LoadState;
use crate::fs::FsErrorKind;
use crate::jobs::{JobError, JobKind, JobResult, MetaPurpose};
use eve_config::ExternalChangePolicy;

impl EditorState {
    /// フォーカス復帰。設定が `never` でなく、オーバーレイが出ていなければ表示中の文書を確かめる。
    pub(super) fn on_focus_gained(&mut self) {
        if self.config().reload_on_external_change == ExternalChangePolicy::Never
            || !self.overlay_name().is_empty()
        {
            return;
        }
        self.start_external_check(false);
    }

    /// 表示中の文書の外部変更の確認ジョブを投入する。パスが無い・読込中・その文書のジョブがある間は投入しない。
    fn start_external_check(&mut self, explicit: bool) -> CommandResult {
        let Some(doc_id) = self.active_document() else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let busy = self.pending.values().any(|p| {
            matches!(p,
                Pending::Open { doc, .. }
                | Pending::Save { doc, .. }
                | Pending::CheckUnchanged { doc }
                | Pending::ExternalCheck { doc, .. } if *doc == doc_id)
        });
        let Some(doc) = self.documents.get(&doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(path) = doc.path.clone() else {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        };
        if busy || doc.load_state != LoadState::Ready {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        let job = self.jobs.spawn(
            Some(doc_id),
            doc.revision(),
            JobKind::Metadata {
                path,
                purpose: MetaPurpose::ExternalCheck,
            },
        );
        self.pending.insert(
            job,
            Pending::ExternalCheck {
                doc: doc_id,
                explicit,
            },
        );
        CommandResult::Pending(job)
    }

    /// 確認の結果。変わっていなければ何もしない (コマンドなら知らせる)。変わっていれば、dirty でなく既定の設定なら再読込し、
    /// そうでなければ尋ねる。削除されていれば「新規として保持 / 閉じる」を尋ねる。
    pub(super) fn finish_external_check(
        &mut self,
        doc_id: DocumentId,
        explicit: bool,
        r: JobResult,
    ) {
        // 届くまでに文書を切り替えた・確認などが出ている (フォーカス復帰): 捨てる (次の契機で確かめる)
        if self.active_document() != Some(doc_id) || (!explicit && !self.overlay_name().is_empty())
        {
            return;
        }
        let Some(doc) = self.documents.get(&doc_id) else {
            return;
        };
        if doc.load_state != LoadState::Ready {
            return;
        }
        let shown = doc.path.as_ref().map(FsPath::display).unwrap_or_default();
        let (disk, seen, dirty) = (doc.disk_stamp, doc.external_seen, doc.is_dirty());
        match r {
            JobResult::Meta { stamp, .. } => {
                if disk.is_some_and(|d| same_on_disk(&d, &stamp)) {
                    if explicit {
                        self.show_message(
                            MessageLevel::Info,
                            "info.external.unchanged",
                            &[("path", &shown)],
                        );
                    }
                    return;
                }
                if !explicit && seen == Some(stamp) {
                    return;
                }
                let policy = self.config().reload_on_external_change;
                if !dirty && disk.is_some() && policy == ExternalChangePolicy::AutoIfUnmodified {
                    self.start_reload();
                    return;
                }
                self.open_external(
                    "external_modified",
                    &shown,
                    vec![choice("reload", 'l'), choice("ignore", 'i')],
                    ConfirmAction::ExternalModified { doc: doc_id, stamp },
                );
            }
            JobResult::Failed {
                err: JobError::Fs(e),
                ..
            } if e.kind == FsErrorKind::NotFound => {
                // まだ一度も書いていない新規ファイル (「新規として保持」を選んだ文書を含む) は「削除」ではない
                if disk.is_none() {
                    if explicit {
                        self.show_message(
                            MessageLevel::Info,
                            "info.external.unchanged",
                            &[("path", &shown)],
                        );
                    }
                    return;
                }
                self.open_external(
                    "external_deleted",
                    &shown,
                    vec![choice("keep", 'k'), choice("close", 'c')],
                    ConfirmAction::ExternalDeleted(doc_id),
                );
            }
            JobResult::Failed {
                err: JobError::Fs(e),
                ..
            } if explicit => self.show_message(
                MessageLevel::Error,
                "msg.external.check_failed",
                &[("path", &shown), ("reason", &fs_reason(&e))],
            ),
            _ => {}
        }
    }

    fn open_external(
        &mut self,
        name: &'static str,
        path: &str,
        choices: Vec<ChoiceDef>,
        action: ConfirmAction,
    ) {
        // 既定は 2 つ目 (無視) / 1 つ目 (新規として保持)。破壊的な選択肢 (再読込・閉じる) を既定にしない
        let default = usize::from(name == "external_modified");
        self.open_confirm(ConfirmState {
            name,
            args: vec![("path".to_owned(), path.to_owned())],
            choices,
            details: Vec::new(),
            default,
            focused: default,
            action,
        });
    }

    /// 「再読込 / 無視」の選択 (キャンセルは呼び側が処理済み。次のフォーカス復帰でまた尋ねる)。
    pub(super) fn choose_external_modified(
        &mut self,
        doc: DocumentId,
        stamp: crate::FileStamp,
        choice: usize,
    ) -> CommandResult {
        if choice == 0 {
            // 変更の破棄は確認の選択で承認済み。表示中の文書でなくなっていれば何もしない
            if self.active_document() != Some(doc) {
                return CommandResult::Rejected(RejectReason::NotAvailable);
            }
            return self.start_reload();
        }
        if let Some(d) = self.documents.get_mut(&doc) {
            d.external_seen = Some(stamp);
        }
        CommandResult::Done
    }

    /// 「新規として保持 / 閉じる」の選択。保持は文書を新規ファイル扱いにする (読んだときの Stamp を捨てる) ので、
    /// 次の保存は確認なしでファイルを作り直し (`Expect::Absent`)、フォーカス復帰でも再び尋ねない。
    pub(super) fn choose_external_deleted(
        &mut self,
        doc: DocumentId,
        choice: usize,
    ) -> CommandResult {
        if choice == 1 {
            return self.request_close(doc);
        }
        if let Some(d) = self.documents.get_mut(&doc) {
            d.disk_stamp = None;
            d.external_seen = None;
        }
        CommandResult::Done
    }
}

fn choice(id: &'static str, accel: char) -> ChoiceDef {
    ChoiceDef {
        id,
        accel: Some(accel),
        destructive: id == "reload",
    }
}

fn file_check_external_changes(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.start_external_check(true)
}

pub(super) fn register(r: &mut CommandRegistry) {
    r.register(
        CommandSpec::new(
            "file.check_external_changes",
            CommandKind::File,
            Permission::None,
            "doc.has_path",
            file_check_external_changes,
        )
        .expect("カタログの when 句は構文が正しい"),
    );
}
