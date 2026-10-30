//! 保存中の終了操作 (docs/03 §9.7 (f)、04 §15.2 `ui.confirm.quit_saving`、05 §11.5)。
//!
//! 保存ジョブの実行中に `app.quit` を受けたら、各保存の後続を `Quit` にして待つ。5 秒以内に終われば確認を出さずに完了の規則へ
//! (成功して clean なら終了、失敗なら通常の保存失敗の確認)。5 秒を超えたら「待つ / 保存を中止して終了」を尋ねる。
//! 「中止して終了」は保存ジョブに取消しを送り、全部の保存ジョブが終わったら、結果に依らず終了する
//! (`Effect::Quit { rescue: DirtyDocuments }`。dirty な文書の退避は Host が行う)。保存が失敗で終わっていれば、
//! 失敗の行を終了時の stderr に出す (`stderr.quit_abort.*`)。

use eve_view::{DocumentId, Effect, JobId, QuitRescue, StderrLine};

use super::overlay::{ChoiceDef, ConfirmAction, ConfirmState, Continuation};
use super::state::EditorState;
use crate::time::{Millis, Tick};
use crate::{CommandResult, FsErrorKind, FsPath, JobError, JobResult};

use super::file::Pending;

/// 保存を待つ時間。これを超えたら確認を出す。
const WAIT: Millis = Millis(5_000);

/// 保存中に終了を求められている。
#[derive(Clone, Copy, Debug)]
pub(super) struct QuitWait {
    /// 確認を出す時刻。
    pub deadline: Tick,
    /// 確認を出した (自動では出し直さない)。
    pub asked: bool,
}

/// 「保存を中止して終了」を選んだ後の状態。保存ジョブがすべて終わったら終了する。
#[derive(Debug, Default)]
pub(super) struct QuitAbort {
    /// 終了時の stderr に出す行 (失敗した保存のもの)。
    pub stderr: Vec<StderrLine>,
}

impl EditorState {
    /// 実行中の保存ジョブ (ジョブ, 文書)。
    fn saving_jobs(&self) -> Vec<(JobId, DocumentId)> {
        self.pending
            .iter()
            .filter_map(|(j, p)| match p {
                Pending::Save { doc, .. } => Some((*j, *doc)),
                _ => None,
            })
            .collect()
    }

    /// `app.quit` の入口: 保存ジョブが実行中なら待つ。待つ場合は `Some`。
    pub(super) fn quit_while_saving(&mut self, user: bool) -> Option<CommandResult> {
        if self.quit_abort.is_some() {
            // 中止して終了の最中: 終了は取り消せない
            return Some(CommandResult::Done);
        }
        let jobs = self.saving_jobs();
        let Some(&(first, _)) = jobs.first() else {
            self.quit_wait = None;
            return None;
        };
        // 終了で置き換える (置き換え・開き直しなどの後続は終了に統合する)
        for (j, _) in &jobs {
            self.continuations.insert(*j, Continuation::Quit);
        }
        match self.quit_wait {
            // もう一度 `app.quit`: 確認をすぐに出す
            Some(_) if user => self.open_quit_saving(),
            Some(_) => {}
            None => {
                self.quit_wait = Some(QuitWait {
                    deadline: self.now + WAIT,
                    asked: false,
                });
            }
        }
        Some(CommandResult::Pending(first))
    }

    /// 次に確認を出す時刻。
    pub(super) fn quit_wait_deadline(&self) -> Option<Tick> {
        self.quit_wait.filter(|w| !w.asked).map(|w| w.deadline)
    }

    /// `Editor::tick` から: 待ちが終わっていれば片付け、5 秒を超えていれば確認を出す。
    pub(super) fn quit_wait_tick(&mut self) {
        let Some(w) = self.quit_wait else {
            return;
        };
        if self.saving_jobs().is_empty() {
            self.quit_wait = None;
            return;
        }
        if !w.asked && self.now >= w.deadline && self.overlay_kind().is_empty() {
            self.open_quit_saving();
        }
    }

    fn open_quit_saving(&mut self) {
        if !self.overlay_kind().is_empty() {
            return;
        }
        let jobs = self.saving_jobs();
        let path = jobs
            .iter()
            .find_map(|(_, d)| self.documents.get(d).and_then(|d| d.path.as_ref()))
            .map(FsPath::display)
            .unwrap_or_else(|| {
                self.catalog()
                    .render("status.untitled", &[("n", "1")], &|_| None)
            });
        // 保存中の文書が 2 件以上なら、先頭の 1 件に「ほか N 件」を足す (03 §9.8)
        let docs: std::collections::BTreeSet<_> = jobs.iter().map(|(_, d)| *d).collect();
        let path = if docs.len() > 1 {
            let others = self.catalog().render(
                "ui.confirm.quit_saving.others",
                &[("count", &(docs.len() - 1).to_string())],
                &|_| None,
            );
            format!("{path} {others}")
        } else {
            path
        };
        if let Some(w) = self.quit_wait.as_mut() {
            w.asked = true;
        }
        self.open_confirm(ConfirmState {
            name: "quit_saving",
            args: vec![("path".to_owned(), path)],
            choices: vec![
                ChoiceDef {
                    id: "wait",
                    accel: Some('w'),
                    destructive: false,
                },
                ChoiceDef {
                    id: "abort",
                    accel: Some('q'),
                    destructive: true,
                },
            ],
            details: Vec::new(),
            default: 0,
            focused: 0,
            action: ConfirmAction::QuitSaving,
        });
    }

    /// 確認への答え。「待つ」は何もしない (`Esc` も同じ)。「中止して終了」は取消しを送り、全部の保存が終わるのを待つ。
    pub(super) fn choose_quit_saving(&mut self, id: &str) -> CommandResult {
        if id != "abort" {
            return CommandResult::Done;
        }
        self.quit_abort = Some(QuitAbort::default());
        self.quit_wait = None;
        let jobs = self.saving_jobs();
        for (j, _) in &jobs {
            self.continuations.remove(j);
            self.jobs.cancel(*j);
        }
        self.quit_abort_check();
        CommandResult::Done
    }

    /// 「中止して終了」の最中に保存ジョブの結果が届いた。採用する (成功) か、失敗の行を覚える。
    /// 取り扱った (通常の完了の規則に進まない) ときは `true`。
    pub(super) fn quit_abort_save_result(&mut self, path: &FsPath, r: &JobResult) -> bool {
        let Some(abort) = self.quit_abort.as_mut() else {
            return false;
        };
        let line = |key: &str, args: Vec<(&str, String)>| StderrLine {
            key: key.to_owned(),
            args: args.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(),
        };
        let p = path.display();
        match r {
            JobResult::Failed {
                err: JobError::Fs(e),
                ..
            } => {
                abort.stderr.push(match &e.kind {
                    FsErrorKind::ReplacedMovedToBackup { backup } => line(
                        "stderr.quit_abort.moved_to_backup",
                        vec![("path", p), ("backup", backup.display())],
                    ),
                    FsErrorKind::InPlaceFailed { backup: Some(b) } => line(
                        "stderr.quit_abort.in_place_failed",
                        vec![("path", p), ("backup", b.display())],
                    ),
                    FsErrorKind::InPlaceFailed { backup: None } => line(
                        "stderr.quit_abort.in_place_failed_no_backup",
                        vec![("path", p)],
                    ),
                    other => line(
                        "stderr.quit_abort.save_failed",
                        vec![("path", p), ("reason", format!("{other:?}"))],
                    ),
                });
                true
            }
            // 取り消された (何も書いていない) ・採用する成功は通常の経路 (`Saved` は呼び出し側が採用する)
            JobResult::Failed { .. } => true,
            _ => false,
        }
    }

    /// 保存ジョブがすべて終わっていれば、dirty な文書の退避を伴う終了を要求する。
    pub(super) fn quit_abort_check(&mut self) {
        if self.quit_abort.is_none() || !self.saving_jobs().is_empty() {
            return;
        }
        let stderr = self.quit_abort.take().map(|a| a.stderr).unwrap_or_default();
        self.push_effect(Effect::Quit {
            code: 0,
            rescue: QuitRescue::DirtyDocuments,
            stderr,
        });
    }
}
