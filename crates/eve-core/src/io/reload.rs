//! 開き直し・再読込と、in-place の差し替え (docs/03 §9.5・§9.7) (STR-B1 で io/file.rs から分けた)。

use eve_view::{DocumentId, MessageLevel, ViewId};

use crate::document::Document;
use crate::io::fspath::FsPath;
use crate::io::io_jobs::OpenOptions;
use crate::jobs::{JobError, JobKind, JobResult};
use eve_encoding::Encoding;

use super::file::*;
use super::open::*;
use super::pending::*;
use crate::command::{CommandResult, RejectReason};
use crate::editor::state::EditorState;

impl EditorState {
    /// 確認を済ませた (または dirty でない) 開き直しの開始。同じ `DocumentId` で読込ジョブを投入し、読込中は旧本文を表示し続ける。
    pub(crate) fn start_reopen(&mut self, doc_id: DocumentId, enc: Encoding) -> CommandResult {
        // 利用者の指定と書込権限の理由は開き直しでも保つ (03 §8.8 の「読み取り専用の理由の差し替え」)
        self.start_in_place(
            doc_id,
            OpenKind::Reopen,
            &[
                crate::document::ReadOnlyReason::UserRequested,
                crate::document::ReadOnlyReason::NoWritePermission,
            ],
            Some(enc),
        )
    }

    /// 確認を済ませた (または dirty でない) 再読込の開始 (03 §9.5)。文字コードは今のものを明示指定して判定し直さない。
    pub(crate) fn start_reload(&mut self, doc_id: DocumentId) -> CommandResult {
        // 再読込は `Binary` と `LargeFileViewer` も保つ (明示指定の読込ではバイナリの判定をしない)
        self.start_in_place(
            doc_id,
            OpenKind::Reload,
            &[
                crate::document::ReadOnlyReason::UserRequested,
                crate::document::ReadOnlyReason::NoWritePermission,
                crate::document::ReadOnlyReason::Binary,
                crate::document::ReadOnlyReason::LargeFileViewer,
            ],
            None,
        )
    }

    /// 文書 `doc_id` の中身を同じ文書・View のまま差し替える読込ジョブの投入。`keep` は今の文書から引き継ぐ
    /// 読み取り専用の理由。`enc` が無ければ今の文書の文字コードを明示指定する。対象はアクティブな文書とは
    /// 限らない (保存の後続・外部変更の確認の結果は、その文書に属する、03 §9.7)。
    pub(crate) fn start_in_place(
        &mut self,
        doc_id: DocumentId,
        kind: OpenKind,
        keep: &[crate::document::ReadOnlyReason],
        enc: Option<Encoding>,
    ) -> CommandResult {
        // その文書を表示している View (Window が表示していれば、その View)
        let shown = self
            .focused_window()
            .and_then(|w| self.window_view(w))
            .filter(|v| self.docs.views.get(v).is_some_and(|v| v.doc == doc_id));
        let view = match shown.or_else(|| {
            self.docs
                .views
                .iter()
                .find(|(_, v)| v.doc == doc_id)
                .map(|(id, _)| *id)
        }) {
            Some(v) => v,
            None => return CommandResult::Rejected(RejectReason::NoTarget),
        };
        let Some(doc) = self.docs.documents.get_mut(&doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(path) = doc.path.clone() else {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        };
        let mut pending = crate::document::ReadOnlyReasons::default();
        for r in keep {
            if doc.read_only.contains(*r) {
                pending.insert(*r);
            }
        }
        let confirmed_large = pending.contains(crate::document::ReadOnlyReason::LargeFileViewer);
        let forced = Some(enc.unwrap_or(doc.format.encoding));
        doc.load_state = crate::document::LoadState::Loading;
        let opts = OpenOptions {
            forced,
            pending,
            // 巨大ファイルとして開いた文書は、確認を繰り返さない
            confirmed_large,
            default_eol: Some(self.config_eol()),
            huge_file_warn_bytes: Some(self.config().huge_file_warn_bytes),
            long_line_threshold: Some(self.config_long_line_threshold()),
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
        self.io.pending.insert(
            job,
            Pending::Open {
                doc: doc_id,
                view,
                path,
                replace: None,
                opts,
                kind,
            },
        );
        CommandResult::Pending(job)
    }

    /// 再読込の適用 (03 §9.5 手順 5): 新しい内容を全置換の `Origin::Reload` の Transaction として適用する。
    /// 書式と読み取り専用の理由は本文と一緒に差し替え、前後の値を Undo ノードに持たせる (Undo で一緒に戻る)。
    #[allow(clippy::too_many_arguments)] // 読込の結果 (JobResult::Complete) をそのまま受ける
    pub(crate) fn apply_reload(
        &mut self,
        doc_id: DocumentId,
        view: ViewId,
        path: &FsPath,
        text: eve_text::Text,
        mut format: crate::document::TextFormat,
        stamp: crate::io::fs::FileStamp,
        detected: crate::document::ReadOnlyReasons,
    ) {
        use crate::document::{FormatChange, FormatState, Origin, Transaction};
        use eve_text::{ByteOff, ByteRange, ChangeSet, Edit, Selection, TextRead};
        // 読み取り専用属性はディスクから判定し直したので、解除の記録と外部変更の見送りは捨てる
        if let Some(d) = self.docs.documents.get_mut(&doc_id) {
            d.attribute_released = false;
            d.external_seen = None;
        }
        let Some(old) = self.docs.documents.get(&doc_id) else {
            return;
        };
        format.eol_stats = crate::document::EolStats::count(&text);
        let change = FormatChange {
            before: FormatState {
                format: old.format.clone(),
                read_only: old.read_only,
            },
            after: FormatState {
                format,
                read_only: detected,
            },
        };
        let new_len = text.len_bytes();
        let body = text.slice_to_string(ByteRange::all(new_len));
        let old_len = old.text().len_bytes();
        let Ok(cs) = ChangeSet::from_sorted_edits(
            old.text(),
            [Edit {
                range: ByteRange::all(old_len),
                insert: body.into(),
            }],
        ) else {
            self.reopen_restore(doc_id);
            return;
        };
        // カーソルは同じ位置を目安にし (新しい内容の範囲に収める)、Undo では元の Selection に戻す
        let before_sel = self.docs.views.get(&view).map(|v| v.selection.clone());
        let head = before_sel.as_ref().map_or(0, |s| s.primary().head.0);
        let after_sel = Selection::point(ByteOff(head.min(new_len))).normalized(&text);
        let tx = Transaction::new(cs, Origin::Reload, self.now)
            .with_selections(before_sel, Some(after_sel))
            .with_view(view)
            .with_format_change(change);
        let applied = self.apply_transaction(doc_id, tx);
        if let Some(d) = self.docs.documents.get_mut(&doc_id) {
            d.finish_reload(stamp);
        }
        self.mark_dirty_render();
        if applied.is_err() {
            self.show_message(
                MessageLevel::Error,
                "msg.open.failed",
                &[("path", &path.display()), ("reason", "reload")],
            );
            return;
        }
        let read_only = self
            .docs
            .documents
            .get(&doc_id)
            .is_some_and(|d| !d.read_only.is_empty());
        let key = if read_only {
            "info.reload.done_read_only"
        } else {
            "info.reload.done"
        };
        self.show_message(MessageLevel::Info, key, &[("path", &path.display())]);
    }

    /// 開き直しの取消し・失敗: 旧本文・書式・履歴をそのまま保って `Ready` に戻す。
    pub(crate) fn reopen_restore(&mut self, doc: DocumentId) {
        if let Some(d) = self.docs.documents.get_mut(&doc) {
            d.load_state = crate::document::LoadState::Ready;
        }
        self.mark_dirty_render();
    }

    pub(crate) fn finish_reopen(
        &mut self,
        doc_id: DocumentId,
        view: ViewId,
        path: FsPath,
        opts: OpenOptions,
        kind: OpenKind,
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
            } if kind == OpenKind::Reload => {
                self.apply_reload(doc_id, view, &path, text, format, stamp, detected);
                if changed_during_load {
                    self.message(
                        MessageLevel::Warn,
                        "msg.open.changed_during_load",
                        &path.display(),
                    );
                }
            }
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
                self.put_document(doc_id, d);
                if let Some(v) = self.docs.views.get_mut(&view) {
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
                    kind,
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
                let key = if kind == OpenKind::Reload {
                    "msg.reload.missing"
                } else {
                    "msg.open.failed"
                };
                self.show_message(
                    MessageLevel::Error,
                    key,
                    &[("path", &path.display()), ("reason", "not found")],
                );
            }
        }
    }
}
