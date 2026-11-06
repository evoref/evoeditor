//! 改行コードの変換の実行 (docs/03 §8.4、01 FR-113)。`file.set_line_ending` の手順 (3)〜(5)。
//! 文書が 1 MiB 以下ならメインスレッドで ChangeSet を作り、超えればジョブ (`JobKind::ConvertEol`) で作る。

// document/ の中に置くが、文書テキストを直接変える経路 (`eve_text::Text::apply`) は使わない
// (document/mod.rs の許可を受け継がない。変更は Transaction を通す、02 §5.2)
#![deny(clippy::disallowed_methods)]

use eve_text::{ChangeSet, LineEnding, TextRead};
use eve_view::{CommandId, DocumentId, MessageLevel, Revision, ViewId};

use crate::command::ids;
use crate::document::eol::build_eol_conversion;
use crate::document::{FormatChange, Origin, Transaction, UndoGroupHint};
use crate::jobs::cancel::CancelToken;
use crate::jobs::{JobKind, JobResult};
use crate::ui_model::keys;

use crate::editor::state::EditorState;
use crate::io::pending::Pending;

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
pub(crate) struct EolConvert {
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
    pub(crate) fn convert_eol(
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
        let Some(d) = self.docs.documents.get(&c.doc) else {
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
            .io
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
        self.io.pending.insert(job, Pending::ConvertEol(c));
    }

    /// ジョブの結果。作った Revision が古ければ今の Snapshot で 1 回だけ作り直し、それも古ければ中止を知らせる。
    pub(crate) fn finish_eol_conversion(&mut self, c: EolConvert, r: JobResult) {
        let JobResult::ConvertEolDone { changes, .. } = r else {
            return;
        };
        let fresh = self
            .docs
            .documents
            .get(&c.doc)
            .is_some_and(|d| d.revision() == c.revision);
        if fresh {
            self.apply_eol_conversion(c, changes);
        } else if c.retried {
            self.show_message(MessageLevel::Warn, keys::MSG_EOL_CONVERT_STALE, &[]);
        } else {
            self.start_eol_conversion(EolConvert { retried: true, ..c });
        }
    }

    /// ChangeSet を 1 つの Transaction として適用し、同時に `default_eol` を `eol` にする。
    /// Undo ノードの `format_change` に前後の `default_eol` だけを持たせ、1 回の Undo で本文と `default_eol` が戻る
    /// (文字コード・BOM などは戻さない。`eol_stats` は本文の適用で数え直す。03 §7.2)。
    fn apply_eol_conversion(&mut self, c: EolConvert, cs: ChangeSet) {
        let Some(doc) = self.docs.documents.get(&c.doc) else {
            return;
        };
        let change = FormatChange::DefaultEol {
            before: doc.format.default_eol,
            after: c.eol,
        };
        let sel = self.view(c.view).map(|v| v.selection.clone());
        let tx = Transaction::new(
            cs,
            Origin::Command(CommandId::new(ids::FILE_SET_LINE_ENDING)),
            self.now(),
        )
        .with_view(c.view)
        .with_undo_group(UndoGroupHint::Break)
        .with_selections(sel, None)
        .with_format_change(change);
        match self.apply_transaction(c.doc, tx) {
            Ok(_) => self.show_message(
                MessageLevel::Info,
                keys::INFO_EOL_CONVERTED,
                &[("count", &c.count.to_string()), ("eol", name(c.eol))],
            ),
            Err(_) => self.show_message(MessageLevel::Warn, keys::MSG_EOL_CONVERT_STALE, &[]),
        }
    }
}
