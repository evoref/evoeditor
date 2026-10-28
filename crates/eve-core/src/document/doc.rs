//! `Document` と `Document::apply` (docs/03-core-design.md §6.5、docs/02 §5.1 / §5.2)。
//! 文書テキストを変える唯一の経路。View の Selection・Decoration の写像 (手順 7〜9) と通知 (手順 10・12) は、
//! それらを持つ `EditorState` (C2 / C3) が `Applied` を使って行う。

use std::sync::Arc;

use eve_text::{ByteOff, ChangeSet, ChangeSetError, Selection, Snapshot, Text, TextRead};
use eve_view::Revision;

use crate::fs::FileStamp;
use crate::fspath::FsPath;

use super::format::{EolStats, LoadState, ReadOnlyReasons, TextFormat, count_range};
use super::history::{NodeId, RecordOutcome, UndoHistory};
use super::transaction::{Origin, Transaction};

/// `Document::apply` が返す結果。
#[derive(Clone, Debug)]
pub enum AppliedTx {
    /// 変更のない Transaction (手順 1)。文書・Revision・履歴・通知のどれも変わらない。
    /// 発行 View の Selection だけを `selection` (あれば) に更新する。
    Identity {
        selection: Option<Selection>,
    },
    Changed(Applied),
}

/// 適用した変更。呼び出し側が View・Decoration の写像と通知に使う。
#[derive(Clone, Debug)]
pub struct Applied {
    pub old_revision: Revision,
    pub revision: Revision,
    pub changes: Arc<ChangeSet>,
    pub origin: Origin,
    pub is_undo: bool,
    /// 発行 View に設定する Selection。通常の編集は `selection_after` (なければ `None` = 写像)、
    /// Undo は `selection_before`、Redo は `selection_after`。
    pub selection: Option<Selection>,
    /// 履歴への記録の結果 (Undo / Redo では `None`)。`pruned` が真なら通知を 1 回出す。
    pub record: Option<RecordOutcome>,
}

#[derive(Debug)]
pub enum ApplyError {
    /// `tx.changes.len_before` が文書の長さと違う (古い Revision に基づく)。
    StaleBase { expected: usize, actual: usize },
    /// 読み取り専用、または読込が終わっていない。
    Rejected,
    /// 操作の境界が char 境界でない (文書は変えない)。
    Text(ChangeSetError),
}

/// 表示の設定 (`editor.tab_width`・`view.ambiguous_width`、07 §5.0)。Editor が文書を持つときに設定から写す。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplaySettings {
    pub tab_width: u8,
    pub ambiguous_wide: bool,
}

impl Default for DisplaySettings {
    fn default() -> DisplaySettings {
        DisplaySettings {
            tab_width: 4,
            ambiguous_wide: false,
        }
    }
}

pub struct Document {
    text: Text,
    pub path: Option<FsPath>,
    pub disk_stamp: Option<FileStamp>,
    pub format: TextFormat,
    revision: Revision,
    history: UndoHistory,
    pub read_only: ReadOnlyReasons,
    pub load_state: LoadState,
    /// 読込中の進捗 (百分率)。ステータスバーの `[読込中 n%]` に出す。
    pub load_percent: u8,
    pub display: DisplaySettings,
    /// 長い行の表示単位の計算結果 (最後の 1 行分)。1 回の操作で同じ行を何度も展開しないための覚え。
    /// 中身の型は表示の層 (`editor::layout`) だけが知る。
    pub(crate) layout_cache: std::cell::RefCell<Option<Box<dyn std::any::Any>>>,
}

impl Document {
    /// 新規文書 (無題)。履歴の根が保存点。
    pub fn new_untitled() -> Document {
        Document::from_text(Text::new(), TextFormat::new_document())
    }

    /// 読込完了した文書。`format.eol_stats` は文書から数え直す。履歴の根が保存点。
    pub fn from_text(text: Text, mut format: TextFormat) -> Document {
        format.eol_stats = EolStats::count(&text);
        Document {
            text,
            path: None,
            disk_stamp: None,
            format,
            revision: Revision::INITIAL,
            history: UndoHistory::default(),
            read_only: ReadOnlyReasons::default(),
            load_state: LoadState::Ready,
            load_percent: 0,
            display: DisplaySettings::default(),
            layout_cache: std::cell::RefCell::new(None),
        }
    }

    pub fn text(&self) -> &Text {
        &self.text
    }

    pub fn snapshot(&self) -> Snapshot {
        self.text.snapshot()
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn history(&self) -> &UndoHistory {
        &self.history
    }

    pub fn is_dirty(&self) -> bool {
        self.history.is_dirty()
    }

    /// 編集できるか (`read_only` が空かつ `load_state == Ready`)。
    pub fn is_editable(&self) -> bool {
        self.read_only.is_empty() && self.load_state == LoadState::Ready
    }

    /// Transaction を適用する (03 §6.5 の手順 1〜6・11)。
    pub fn apply(&mut self, tx: &Transaction) -> Result<AppliedTx, ApplyError> {
        // 1. 前提検査
        if tx.changes.len_before() != self.text.len_bytes() {
            return Err(ApplyError::StaleBase {
                expected: tx.changes.len_before(),
                actual: self.text.len_bytes(),
            });
        }
        let exempt = matches!(tx.origin, Origin::Recovery | Origin::Reload);
        if !self.is_editable() && !exempt {
            return Err(ApplyError::Rejected);
        }
        if tx.changes.is_identity() {
            // 本文が同じ再読込: 書式と理由だけ差し替える (履歴のノードは作らない)
            if let Some(fc) = &tx.format_change {
                self.format = fc.after.format.clone();
                self.read_only = fc.after.read_only;
            }
            return Ok(AppliedTx::Identity {
                selection: tx.selection_after.clone(),
            });
        }
        // 2.〜4. 適用前の Snapshot、逆変換、適用。`Text::apply` は検査を先に行い、失敗では何も変えない
        let before = self.text.snapshot();
        let inverse = tx.changes.invert(&before);
        self.text.apply(&tx.changes).map_err(ApplyError::Text)?;
        self.update_eol_stats(&before, &tx.changes);
        // 5. Revision
        let old_revision = self.revision;
        self.revision = self.revision.next();
        // 6. 履歴への記録 (Undo / Redo の Transaction は `undo` / `redo` を使う)
        let record = if tx.is_undo {
            None
        } else {
            Some(self.history.record(tx, inverse))
        };
        // 再読込: 書式と読み取り専用の理由を新しい内容のものに差し替える (前後の値は Undo ノードが持つ)
        if let Some(fc) = &tx.format_change {
            self.format = fc.after.format.clone();
            self.read_only = fc.after.read_only;
        }
        Ok(AppliedTx::Changed(Applied {
            old_revision,
            revision: self.revision,
            changes: Arc::new(tx.changes.clone()),
            origin: tx.origin.clone(),
            is_undo: tx.is_undo,
            selection: tx.selection_after.clone(),
            record,
        }))
    }

    /// Undo。取り消すものがなければ `None`。読み取り専用では `Err(Rejected)` (Reload・Recovery のノードは例外にしない:
    /// 03 §7.2 の `edit.undo` の when は `!doc.read_only`)。
    pub fn undo(&mut self) -> Result<Option<Applied>, ApplyError> {
        if !self.is_editable() {
            return Err(ApplyError::Rejected);
        }
        let Some(step) = self.history.undo_step() else {
            return Ok(None);
        };
        Ok(Some(self.apply_step(
            step.changes,
            step.origin,
            step.selection,
            step.format,
        )?))
    }

    /// Redo。やり直すものがなければ `None`。
    pub fn redo(&mut self) -> Result<Option<Applied>, ApplyError> {
        if !self.is_editable() {
            return Err(ApplyError::Rejected);
        }
        let Some(step) = self.history.redo_step() else {
            return Ok(None);
        };
        Ok(Some(self.apply_step(
            step.changes,
            step.origin,
            step.selection,
            step.format,
        )?))
    }

    fn apply_step(
        &mut self,
        changes: ChangeSet,
        origin: Origin,
        selection: Option<Selection>,
        format: Option<super::format::FormatState>,
    ) -> Result<Applied, ApplyError> {
        let before = self.text.snapshot();
        // 履歴と文書が食い違った場合も、文書は変えない (履歴の不整合は呼び出しの誤り)
        self.text.apply(&changes).map_err(ApplyError::Text)?;
        self.update_eol_stats(&before, &changes);
        // 再読込のノードは、書式と読み取り専用の理由も一緒に戻す (03 §7.2)
        if let Some(f) = format {
            self.format = f.format;
            self.read_only = f.read_only;
        }
        let old_revision = self.revision;
        self.revision = self.revision.next();
        Ok(Applied {
            old_revision,
            revision: self.revision,
            changes: Arc::new(changes),
            origin,
            is_undo: true,
            selection,
            record: None,
        })
    }

    /// 再読込の完了 (03 §9.5): 読み込んだ内容がディスクの状態なので、ここを保存点にして `Ready` に戻す。
    pub fn finish_reload(&mut self, stamp: FileStamp) {
        self.history.seal();
        self.history.mark_saved();
        self.disk_stamp = Some(stamp);
        self.load_state = LoadState::Ready;
    }

    /// 保存の開始 (03 §9.3 手順 3): 履歴を封印し、保存に使う Snapshot とその時点の履歴上の位置を返す。
    pub fn begin_save(&mut self) -> (Snapshot, NodeId) {
        self.history.seal();
        (self.text.snapshot(), self.history.current())
    }

    /// 保存の完了: 保存に使った位置を保存点にする (その位置が履歴に無ければ保存点なし、INV-H3)。
    pub fn finish_save(&mut self, at: NodeId, stamp: Option<FileStamp>) {
        self.history.mark_saved_at(at);
        if stamp.is_some() {
            self.disk_stamp = stamp;
        }
    }

    /// プロセス全体の Undo の上限のために、最も古い履歴を 1 つ破棄する。解放した会計値を返す。dirty 表示は変わらない。
    pub fn trim_history_oldest(&mut self) -> Option<usize> {
        self.history.trim_oldest()
    }

    /// 履歴の封印 (カーソル移動など)。
    pub fn seal_history(&mut self) {
        self.history.seal();
    }

    /// 手順 4 の `eol_stats` の更新 (INV-D5): 変更ごとに、旧文書と新文書の窓の改行を数え直して差し替える。
    fn update_eol_stats(&mut self, before: &Snapshot, cs: &ChangeSet) {
        let mut stats = self.format.eol_stats;
        for (old, new) in cs.changed_ranges() {
            let o = count_range(before, old.start.0.saturating_sub(1), old.end.0 + 1);
            let n = count_range(&self.text, new.start.0.saturating_sub(1), new.end.0 + 1);
            stats = stats.replaced(o, n);
        }
        self.format.eol_stats = stats;
    }

    /// 文書全体から数え直した値 (検証用)。
    pub fn recount_eol(&self) -> EolStats {
        EolStats::count(&self.text)
    }

    /// 文書の内容全体 (小さな文書・テスト用)。
    pub fn to_string_lossy(&self) -> String {
        self.text
            .slice_to_string(eve_text::ByteRange::all(self.text.len_bytes()))
    }

    pub fn len_bytes(&self) -> usize {
        self.text.len_bytes()
    }

    pub fn byte_off(&self, i: usize) -> ByteOff {
        ByteOff(i.min(self.text.len_bytes()))
    }
}
