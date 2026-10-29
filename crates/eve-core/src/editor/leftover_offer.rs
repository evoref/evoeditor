//! 置換途中の痕跡の提案と適用 (docs/05 §4.6「起動時の提案の規則」、03 §10.3 手順 1b)。
//!
//! 起動時の `RecoveryScan` が作った提案 (`Leftover`) を、新しい順に 1 件ずつ確認ダイアログで出す。スワップの提案 (`swap_offer`) が
//! 先で、ダイアログが出ている間は待つ。答えのうちディスクを変えるもの (退避名を戻す・消す) はジョブで行い、
//! 「一時ファイルを別バッファで開く」は読み取り専用の追加の文書として開く。Esc は何もせず、痕跡を残す (次の起動でまた提案する)。

use eve_view::MessageLevel;

use super::file::Pending;
use super::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
use super::state::EditorState;
use crate::io_jobs::OpenOptions;
use crate::{CommandResult, JobError, JobKind, JobResult, Leftover, LeftoverKind, LeftoverOp};

const fn choice(id: &'static str, accel: char, destructive: bool) -> ChoiceDef {
    ChoiceDef {
        id,
        accel: Some(accel),
        destructive,
    }
}

/// 提案ごとの確認ダイアログの名前 (`ui.confirm.<name>`) と選択肢 (先頭が既定)。
fn dialog(kind: LeftoverKind) -> (&'static str, Vec<ChoiceDef>) {
    match kind {
        LeftoverKind::BakAndTmp => (
            "leftover_both",
            vec![choice("both", 'b', false), choice("restore", 'r', false)],
        ),
        LeftoverKind::Bak => (
            "leftover_bak",
            vec![choice("restore", 'r', false), choice("delete", 'd', true)],
        ),
        LeftoverKind::StaleBak => ("leftover_stale", vec![choice("delete", 'd', true)]),
        LeftoverKind::Tmp => (
            "leftover_tmp",
            vec![choice("open", 'o', false), choice("delete", 'd', true)],
        ),
    }
}

/// 提案への答えの後に続ける動作。
#[derive(Clone, Debug)]
pub(super) struct LeftoverAsk {
    pub leftover: Leftover,
    pub op: LeftoverOp,
    /// 操作が成功したら、一時ファイルを別バッファで開く (「両方」)。
    pub then_open_tmp: bool,
}

impl EditorState {
    /// 次の提案を出す。ダイアログなどが出ている間、提案が無いときは何もしない。
    /// 呼ぶのは `swap_offer` の後 (スワップの提案が先)。
    pub(super) fn leftover_offer(&mut self) {
        if self.leftovers.is_empty() || self.overlay_kind() != "" {
            return;
        }
        let l = self.leftovers.remove(0);
        let (name, choices) = dialog(l.kind);
        let path = l.target.display();
        self.open_confirm(ConfirmState {
            name,
            args: vec![
                ("path".to_owned(), path),
                (
                    "bak".to_owned(),
                    l.bak.as_ref().map(|p| p.display()).unwrap_or_default(),
                ),
                (
                    "tmp".to_owned(),
                    l.tmp.as_ref().map(|p| p.display()).unwrap_or_default(),
                ),
            ],
            choices,
            details: Vec::new(),
            default: 0,
            focused: 0,
            action: ConfirmAction::Leftover(Box::new(l)),
        });
    }

    /// 提案への答え。
    pub(super) fn choose_leftover(&mut self, l: Leftover, id: &str) -> CommandResult {
        let (op, then_open_tmp) = match (l.kind, id) {
            (LeftoverKind::BakAndTmp, "both") => (LeftoverOp::Restore, true),
            (LeftoverKind::BakAndTmp | LeftoverKind::Bak, "restore") => {
                (LeftoverOp::Restore, false)
            }
            (LeftoverKind::Bak | LeftoverKind::StaleBak, "delete") => {
                (LeftoverOp::DeleteBak, false)
            }
            (LeftoverKind::Tmp, "delete") => (LeftoverOp::DeleteTmp, false),
            (LeftoverKind::Tmp, "open") => {
                self.open_leftover_tmp(&l);
                return CommandResult::Done;
            }
            _ => return CommandResult::Done,
        };
        let job = self.jobs.spawn(
            None,
            eve_view::Revision::INITIAL,
            JobKind::LeftoverAct {
                leftover: Box::new(l.clone()),
                op,
            },
        );
        self.pending.insert(
            job,
            Pending::Leftover(Box::new(LeftoverAsk {
                leftover: l,
                op,
                then_open_tmp,
            })),
        );
        CommandResult::Pending(job)
    }

    /// 一時ファイルを、読み取り専用の追加の文書として開いて表示する。末尾が切れている可能性を知らせる。
    fn open_leftover_tmp(&mut self, l: &Leftover) {
        let Some(tmp) = l.tmp.as_ref() else {
            return;
        };
        let opts = OpenOptions {
            user_read_only: true,
            ..OpenOptions::default()
        };
        if let Some(doc) = self.open_additional(&tmp.display(), opts) {
            self.activate_document(doc);
        }
        self.show_message(
            MessageLevel::Warn,
            "msg.leftover.tmp_opened",
            &[("path", &l.target.display())],
        );
    }

    /// 操作のジョブの結果。
    pub(super) fn finish_leftover(&mut self, ask: LeftoverAsk, r: JobResult) {
        let path = ask.leftover.target.display();
        match r {
            JobResult::LeftoverDone { .. } => {
                let key = match ask.op {
                    LeftoverOp::Restore => "info.leftover.restored",
                    LeftoverOp::DeleteBak | LeftoverOp::DeleteTmp => "info.leftover.deleted",
                };
                self.show_message(MessageLevel::Info, key, &[("path", &path)]);
                if ask.then_open_tmp {
                    self.open_leftover_tmp(&ask.leftover);
                }
            }
            JobResult::Failed {
                err: JobError::Fs(e),
                ..
            } => {
                self.show_message(
                    MessageLevel::Error,
                    "msg.leftover.failed",
                    &[("path", &path), ("reason", &format!("{:?}", e.kind))],
                );
            }
            _ => {}
        }
    }
}
