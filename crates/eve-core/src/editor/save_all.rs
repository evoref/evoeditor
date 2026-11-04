//! 全保存 (`file.save_all`、docs/03 §9.8、01 FR-104 AC-2)。
//!
//! dirty でパスがあり、編集できる文書を開いた順に保存する。名前の無い文書・読み取り専用の文書は飛ばす。
//! 全保存は確認を出さない: 確認が要る保存は、保存せず失敗として記録し、最後に一覧で知らせる。

use std::collections::BTreeSet;

use eve_view::{DocumentId, JobId, MessageLevel};

use super::command::{
    CommandCtx, CommandKind, CommandRegistry, CommandResult, CommandSpec, Permission, RejectReason,
};
use super::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
use super::state::EditorState;
use crate::fs::Expect;
use crate::jobs::{JobError, JobResult};

/// 実行中の全保存。
#[derive(Debug, Default)]
pub(super) struct SaveAll {
    /// 結果を待っている保存ジョブ。
    jobs: BTreeSet<JobId>,
    saved: usize,
    /// 飛ばした文書 (名前が無い・読み取り専用) の数。
    skipped: usize,
    /// 保存できなかった文書: (パス, 理由)。
    failed: Vec<(String, String)>,
}

impl EditorState {
    fn save_all(&mut self) -> CommandResult {
        if self.save_all.is_some() {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        let mut batch = SaveAll::default();
        let ids: Vec<DocumentId> = self.documents.keys().copied().collect();
        for id in ids {
            let Some(doc) = self.documents.get(&id) else {
                continue;
            };
            if !doc.is_dirty() || self.save_in_progress(id) {
                continue;
            }
            let Some(path) = doc.path.clone().filter(|_| doc.is_editable()) else {
                batch.skipped += 1;
                continue;
            };
            let expected = doc.disk_stamp.map_or(Expect::Absent, Expect::Stamp);
            // 書く前の確認 (区分 B、03 §8.6 E2'・読み取り専用を解除して保存、FR-116) が要る文書は保存しない
            let needs_readonly_ok =
                doc.attribute_released && self.readonly_ok != Some((id, doc.revision()));
            if (doc.format.roundtrip.lossy_lines > 0 && self.lossy_ok != Some((id, doc.revision())))
                || needs_readonly_ok
            {
                let reason = self.message_text("ui.confirm.save_all_failed.needs_confirm", &[]);
                batch.failed.push((path.display(), reason));
                continue;
            }
            match self.start_save(id, path.clone(), expected, None) {
                CommandResult::Pending(job) => {
                    batch.jobs.insert(job);
                }
                _ => batch
                    .failed
                    .push((path.display(), "unsupported".to_owned())),
            }
        }
        self.save_all = Some(batch);
        self.finish_save_all_if_done();
        CommandResult::Done
    }

    /// 保存ジョブ `job` の結果を全保存の 1 件として数える。全保存の保存でなければ `None`。
    /// 失敗は確認を出さずに記録して `Some(true)`、成功は `Some(false)` (通常どおり文書に反映する)。
    pub(super) fn save_all_failed(
        &mut self,
        job: JobId,
        doc: DocumentId,
        path: &crate::FsPath,
        r: &JobResult,
    ) -> Option<bool> {
        let batch = self.save_all.as_mut()?;
        if !batch.jobs.remove(&job) {
            return None;
        }
        if matches!(r, JobResult::Saved { .. }) {
            batch.saved += 1;
            return Some(false);
        }
        let reason = match r {
            JobResult::Failed { err, .. } => super::file::reason_label(err),
            JobResult::Unmappable { .. } => "unmappable".to_owned(),
            _ => "failed".to_owned(),
        };
        batch.failed.push((path.display(), reason));
        // 上書き保存が権限エラーで失敗したら書込権限なしにする (FR-116 AC-3、通常の保存と同じ)
        let denied = matches!(
            r,
            JobResult::Failed { err: JobError::Fs(e), .. }
                if e.kind == crate::FsErrorKind::PermissionDenied
        );
        if denied
            && let Some(d) = self.documents.get_mut(&doc)
            && d.path.as_ref() == Some(path)
        {
            d.read_only
                .insert(crate::document::ReadOnlyReason::NoWritePermission);
            self.mark_dirty_render();
        }
        Some(true)
    }

    /// 全部の保存ジョブが終わったら 1 回だけ知らせる (失敗があれば一覧の確認)。
    pub(super) fn finish_save_all_if_done(&mut self) {
        if self.save_all.as_ref().is_none_or(|b| !b.jobs.is_empty()) {
            return;
        }
        let Some(b) = self.save_all.take() else {
            return;
        };
        let key = match (b.saved + b.skipped + b.failed.len(), b.skipped) {
            (0, _) => "info.save_all.none",
            (_, 0) => "info.save_all.done",
            _ => "info.save_all.done_skipped",
        };
        let (saved, skipped) = (b.saved.to_string(), b.skipped.to_string());
        self.show_message(
            MessageLevel::Info,
            key,
            &[("saved", &saved), ("skipped", &skipped)],
        );
        if b.failed.is_empty() {
            return;
        }
        self.open_confirm(ConfirmState {
            name: "save_all_failed",
            args: vec![("count".to_owned(), b.failed.len().to_string())],
            // 選択肢が 1 つだけの通知。`abort` を持つ確認はキャンセルの行を足さない
            choices: vec![ChoiceDef {
                id: "abort",
                accel: Some('c'),
                destructive: false,
            }],
            details: b
                .failed
                .iter()
                .map(|(path, reason)| format!("{path}: {reason}"))
                .collect(),
            default: 0,
            focused: 0,
            action: ConfirmAction::Notice,
        });
    }
}

fn file_save_all(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.save_all()
}

/// v0.2 の `file.save_all` (07 §11.6)。
pub(super) fn register(r: &mut CommandRegistry) {
    r.register(
        CommandSpec::new(
            "file.save_all",
            CommandKind::File,
            Permission::None,
            "",
            file_save_all,
        )
        .expect("カタログの when 句は構文が正しい"),
    );
}
