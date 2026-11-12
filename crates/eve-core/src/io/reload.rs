//! 開き直し・再読込と、in-place の差し替え (docs/03 §9.5・§9.7) (STR-B1 で io/file.rs から分けた)。

use eve_view::{DocumentId, MessageLevel, ViewId};

use crate::document::Document;
use crate::io::fspath::FsPath;
use crate::io::io_jobs::OpenOptions;
use crate::jobs::{JobError, JobKind, JobResult};
use crate::ui_model::keys;
use eve_encoding::Encoding;

use super::file::*;
use super::open::*;
use super::pending::*;
use crate::command::{CommandResult, RejectReason};
use crate::editor::state::EditorState;

/// 旧本文の位置 `head` の行:桁 (書記素) を、新本文の同じ行:桁へ写す (03 §9.5)。
/// 新本文の行が足りなければ最後の行、桁が足りなければ行末にする。
fn same_line_col(
    old: &eve_text::Text,
    new: &eve_text::Text,
    head: eve_text::ByteOff,
) -> eve_text::ByteOff {
    use eve_text::{ByteOff, TextRead, next_grapheme_boundary};
    let head = ByteOff(head.0.min(old.len_bytes()));
    let line = old.byte_to_line(head);
    let (mut p, mut col) = (old.line_to_byte(line), 1);
    while p < head {
        let n = next_grapheme_boundary(old, p);
        if n == p {
            break;
        }
        p = n;
        col += 1;
    }
    let last = new.len_lines().saturating_sub(1);
    crate::view::motion::line_col_byte(new, line.0.min(last), col)
}

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
        preserved: crate::document::PreservedMap,
        stamp: crate::io::fs::FileStamp,
        detected: crate::document::ReadOnlyReasons,
        read_only_requested: bool,
    ) {
        use crate::document::{FormatChange, FormatState, Origin, Transaction};
        use eve_text::{ByteOff, ChangeSet, InsertText, Op, Selection, TextRead};
        // 読み取り専用属性はディスクから判定し直したので、解除の記録と外部変更の見送りは捨てる
        if let Some(d) = self.docs.documents.get_mut(&doc_id) {
            d.attribute_released = false;
            d.external_seen = None;
            // 再読込の確認 (不正バイトの比率、FR-127) で「読み取り専用で開く」を選んだ: 解除を持ち越さず、記録も消す
            // (利用者が読み取り専用を選んだので、Undo / Redo でも外さない。03 §8.3.1 の細目 (a))
            if read_only_requested {
                d.invalid_bytes_released = false;
            }
        }
        let Some(old) = self.docs.documents.get(&doc_id) else {
            return;
        };
        // 「編集を許可」(不正バイトの解除) は同じ文書の再読込では持ち越す: 新しい読込に不正バイトがあっても
        // `InvalidBytes` を付けない (03 §8.3.1 の「解除の持ち越し」、§8.8)。新しい内容がバイトの文書でない (私用文字と
        // 衝突した内容 (衝突の文書 (b)) など、解除できない文書) ときは、持ち越さずに付ける
        let mut detected = detected;
        if old.invalid_bytes_released && format.is_bytes_document() {
            detected.remove(crate::document::ReadOnlyReason::InvalidBytes);
        }
        format.eol_stats = crate::document::EolStats::count(&text);
        let change = FormatChange::Reload {
            before: FormatState {
                format: old.format.clone(),
                read_only: old.read_only,
                preserved: old.preserved().clone(),
            },
            after: FormatState {
                format,
                read_only: detected,
                preserved,
            },
        };
        // 全置換。新全文は読込の rope を共有し (複写しない)、旧全文は逆変換が旧 rope を共有する (03 §9.5、SAF-33)
        let old_len = old.text().len_bytes();
        let cs = ChangeSet::from_ops([
            Op::Delete(old_len),
            Op::Insert(InsertText::Rope(text.snapshot())),
        ]);
        // カーソルは同じ行:桁 (書記素) を目安にし (行・桁が足りなければ最後の行・行末に丸める)、Undo では元の Selection に戻す
        let before_sel = self.docs.views.get(&view).map(|v| v.selection.clone());
        let head = before_sel.as_ref().map_or(0, |s| s.primary().head.0);
        let after_sel =
            Selection::point(same_line_col(old.text(), &text, ByteOff(head))).normalized(&text);
        let tx = Transaction::new(cs, Origin::Reload, self.now)
            .with_selections(before_sel, Some(after_sel))
            .with_view(view)
            .with_format_change(change);
        let applied = self.apply_transaction(doc_id, tx);
        if applied.is_err() {
            // 旧本文が残る。保存点・disk_stamp は動かさない (未保存の編集を保存済みの扱いにしない)
            self.reopen_restore(doc_id);
            self.show_message(
                MessageLevel::Error,
                keys::MSG_OPEN_FAILED,
                &[("path", &path.display()), ("reason", "reload")],
            );
            return;
        }
        if let Some(d) = self.docs.documents.get_mut(&doc_id) {
            d.finish_reload(stamp);
        }
        self.mark_dirty_render();
        let read_only = self
            .docs
            .documents
            .get(&doc_id)
            .is_some_and(|d| !d.read_only.is_empty());
        let key = if read_only {
            keys::INFO_RELOAD_DONE_READ_ONLY
        } else {
            keys::INFO_RELOAD_DONE
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
                preserved,
                stamp,
                changed_during_load,
                detected,
                ..
            } if kind == OpenKind::Reload => {
                self.apply_reload(
                    doc_id,
                    view,
                    &path,
                    text,
                    format,
                    preserved,
                    stamp,
                    detected,
                    opts.read_only_requested,
                );
                if changed_during_load {
                    self.message(
                        MessageLevel::Warn,
                        keys::MSG_OPEN_CHANGED_DURING_LOAD,
                        &path.display(),
                    );
                }
            }
            JobResult::Complete {
                text,
                format,
                preserved,
                stamp,
                changed_during_load,
                detected,
                ..
            } => {
                let encoding = format.encoding;
                let note = crate::io::open::preserve_note(&format);
                // 保持方式の記録も新しい読込の結果に入れ替える (03 §8.8 / §8.7.1)
                let mut d = Document::from_loaded(text, format, preserved);
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
                        keys::MSG_OPEN_CHANGED_DURING_LOAD,
                        &path.display(),
                    );
                }
                // 読込の完了時の保持方式の通知 (03 §8.7.1)。履歴を消した通知を現在のメッセージにするため、先に出す
                self.show_preserve_note(note);
                self.show_message(
                    MessageLevel::Info,
                    keys::INFO_REOPEN_HISTORY_CLEARED,
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
                    JobError::Cancelled => (MessageLevel::Info, keys::INFO_OPEN_CANCELLED),
                    JobError::Fs(e) => (MessageLevel::Error, open_error_key(&e)),
                    JobError::OutOfMemory { .. } => {
                        (MessageLevel::Error, keys::MSG_OPEN_OUT_OF_MEMORY)
                    }
                    _ => (MessageLevel::Error, keys::MSG_OPEN_FAILED),
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
                    keys::MSG_RELOAD_MISSING
                } else {
                    keys::MSG_OPEN_FAILED
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
