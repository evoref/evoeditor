//! 改行コードの変換の実行 (docs/03 §8.4、01 FR-113)。`file.set_line_ending` の手順 (3)〜(5)。
//! 文書が 1 MiB 以下ならメインスレッドで ChangeSet を作り、超えればジョブ (`JobKind::ConvertEol`) で作る。

use eve_text::{ChangeSet, LineEnding, TextRead};
use eve_view::{CommandId, DocumentId, MessageLevel, Revision, ViewId};

use crate::cancel::CancelToken;
use crate::document::{EolStats, FormatChange, FormatState, Origin, Transaction, UndoGroupHint};
use crate::eol::build_eol_conversion;
use crate::jobs::{JobKind, JobResult};

use super::file::Pending;
use super::state::EditorState;

/// ジョブにする境界 (`SEARCH_SYNC_STEP_BYTES`、03 §12.2)。
const SYNC_BYTES: usize = 1 << 20;

fn name(eol: LineEnding) -> &'static str {
    match eol {
        LineEnding::Lf => "LF",
        LineEnding::Crlf => "CRLF",
        LineEnding::Cr => "CR",
    }
}

/// 変換を実行するときの材料 (ジョブの結果を受けるときまで持つ)。
#[derive(Clone, Copy, Debug)]
pub(super) struct EolConvert {
    pub doc: DocumentId,
    pub view: ViewId,
    pub eol: LineEnding,
    /// 変える区切りの数。
    pub count: usize,
    /// ChangeSet を作った Revision。
    pub revision: Revision,
    /// 作り直した (一度古くなった)。
    pub retried: bool,
}

impl EditorState {
    /// 変換を始める。`count` は変える区切りの数 (> 0)。
    pub(super) fn convert_eol(
        &mut self,
        doc: DocumentId,
        view: ViewId,
        eol: LineEnding,
        count: usize,
    ) {
        self.start_eol_conversion(EolConvert {
            doc,
            view,
            eol,
            count,
            revision: Revision(0),
            retried: false,
        });
    }

    fn start_eol_conversion(&mut self, mut c: EolConvert) {
        let Some(d) = self.documents.get(&c.doc) else {
            return;
        };
        c.revision = d.revision();
        if d.text().len_bytes() <= SYNC_BYTES {
            if let Some(cs) = build_eol_conversion(d.text(), c.eol, &CancelToken::new()) {
                self.apply_eol_conversion(c, cs);
            }
            return;
        }
        // 実行中の変換があれば二重に始めない
        if self
            .pending
            .values()
            .any(|p| matches!(p, Pending::ConvertEol(_)))
        {
            return;
        }
        let snapshot = d.snapshot();
        let job = self.jobs.spawn(
            Some(c.doc),
            c.revision,
            JobKind::ConvertEol {
                snapshot,
                eol: c.eol,
            },
        );
        self.pending.insert(job, Pending::ConvertEol(c));
    }

    /// ジョブの結果。作った Revision が古ければ今の Snapshot で 1 回だけ作り直し、それも古ければ中止を知らせる。
    pub(super) fn finish_eol_conversion(&mut self, c: EolConvert, r: JobResult) {
        let JobResult::ConvertEolDone { changes, .. } = r else {
            return;
        };
        let fresh = self
            .documents
            .get(&c.doc)
            .is_some_and(|d| d.revision() == c.revision);
        if fresh {
            self.apply_eol_conversion(c, changes);
        } else if c.retried {
            self.show_message(MessageLevel::Warn, "msg.eol.convert_stale", &[]);
        } else {
            self.start_eol_conversion(EolConvert { retried: true, ..c });
        }
    }

    /// ChangeSet を 1 つの Transaction として適用し、同時に `default_eol` を `eol` にする。
    /// Undo ノードの `format_change` に前後の書式を持たせ、1 回の Undo で本文と `default_eol` が戻る (03 §7.2)。
    fn apply_eol_conversion(&mut self, c: EolConvert, cs: ChangeSet) {
        let Some(doc) = self.documents.get(&c.doc) else {
            return;
        };
        let stats = doc.format.eol_stats;
        let before = doc.format.clone();
        let mut after = before.clone();
        after.default_eol = c.eol;
        after.eol_stats = match c.eol {
            LineEnding::Lf => EolStats {
                lf: stats.total(),
                crlf: 0,
                cr: 0,
            },
            LineEnding::Crlf => EolStats {
                lf: 0,
                crlf: stats.total(),
                cr: 0,
            },
            LineEnding::Cr => EolStats {
                lf: 0,
                crlf: 0,
                cr: stats.total(),
            },
        };
        let read_only = doc.read_only;
        let change = FormatChange {
            before: FormatState {
                format: before,
                read_only,
            },
            after: FormatState {
                format: after,
                read_only,
            },
        };
        let sel = self.view(c.view).map(|v| v.selection.clone());
        let tx = Transaction::new(
            cs,
            Origin::Command(CommandId::new("file.set_line_ending")),
            self.now(),
        )
        .with_view(c.view)
        .with_undo_group(UndoGroupHint::Break)
        .with_selections(sel, None)
        .with_format_change(change);
        match self.apply_transaction(c.doc, tx) {
            Ok(_) => self.show_message(
                MessageLevel::Info,
                "info.eol.converted",
                &[("count", &c.count.to_string()), ("eol", name(c.eol))],
            ),
            Err(_) => self.show_message(MessageLevel::Warn, "msg.eol.convert_stale", &[]),
        }
    }
}
