//! ファイルの開く・保存 (docs/03 §9.2 / §9.3 の M1 の範囲)。ジョブを投入し、結果で Document を更新する。
//!
//! M1 の範囲: UTF-8、原子的保存、未編集の保存の照合 (手順 0)。次は後続: 確認ダイアログ (dirty の置き換え、
//! 保存の失敗の選択肢、バイナリ・巨大ファイル)、名前の入力プロンプト、保存の連鎖 (Continuation)、保存中フラグ、
//! 外部変更の再読込。確認が要る場面は、確認の UI ができるまで `RejectReason::NeedsInput` で拒否する。

use eve_view::{DocumentId, EditorEvent, MessageLevel, ViewId};

use crate::document::{Document, NodeId};
use crate::fs::{Expect, FsError, FsErrorDetail, FsErrorKind};
use crate::fspath::FsPath;
use crate::io_jobs::OpenOptions;
use crate::jobs::{ConfirmAsk, JobError, JobKind, JobResult, MetaPurpose};

use super::command::{
    CommandCtx, CommandKind, CommandRegistry, CommandResult, CommandSpec, Permission, RejectReason,
};
use super::state::EditorState;

/// 投入したジョブと、その結果の宛先。
#[derive(Debug)]
pub(super) enum Pending {
    Open {
        doc: DocumentId,
        view: ViewId,
        path: FsPath,
        replace: Option<DocumentId>,
    },
    /// 未編集の上書き保存の照合 (03 §9.3 手順 0)。
    CheckUnchanged { doc: DocumentId },
    Save {
        doc: DocumentId,
        at: NodeId,
        revision: eve_view::Revision,
        path: FsPath,
    },
}

impl EditorState {
    fn message(&mut self, level: MessageLevel, key: &str) {
        self.push_event(EditorEvent::MessageShown {
            level,
            key: key.to_owned(),
        });
    }

    /// 読込を待っている間、置き換え対象の今の文書は編集を受け付けない (03 §9.2 の置き換えの手順)。
    /// 編集は読込の完了で黙って捨てられてしまうため。
    pub(super) fn open_replacing(&self) -> bool {
        self.pending.values().any(|p| {
            matches!(
                p,
                Pending::Open {
                    replace: Some(_),
                    ..
                }
            )
        })
    }

    fn open_in_progress(&self) -> bool {
        self.pending
            .values()
            .any(|p| matches!(p, Pending::Open { .. }))
    }

    fn save_in_progress(&self, doc_id: DocumentId) -> bool {
        self.pending
            .values()
            .any(|p| matches!(p, Pending::Save { doc, .. } | Pending::CheckUnchanged { doc } if *doc == doc_id))
    }

    /// 今の文書 (最初の Window の View の文書)。
    pub fn active_document(&self) -> Option<DocumentId> {
        let w = self.first_window()?;
        self.view(self.window_view(w)?).map(|v| v.doc)
    }

    /// 無題の新規文書を作る (FR-124)。今の文書があれば置き換える (dirty なら確認が要るので拒否する)。
    pub fn new_untitled(&mut self) -> CommandResult {
        if self.open_in_progress() {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        let replace = self.active_document();
        if replace
            .and_then(|d| self.document(d))
            .is_some_and(Document::is_dirty)
        {
            return CommandResult::Rejected(RejectReason::NeedsInput);
        }
        let (_, view) = self.add_document(Document::new_untitled());
        self.show_view(view, replace);
        CommandResult::Done
    }

    /// Window の表示を `view` に切り替え、`old` の文書を閉じる。Window がなければ作る。
    fn show_view(&mut self, view: ViewId, old: Option<DocumentId>) {
        match self.windows.first_mut() {
            Some(w) => w.root = view,
            None => {
                self.add_window(view);
            }
        }
        if let Some(old) = old {
            self.close_document(old);
        }
        self.mark_dirty_render();
    }

    fn close_document(&mut self, doc: DocumentId) {
        self.views.retain(|_, v| v.doc != doc);
        if self.documents.remove(&doc).is_some() {
            self.push_event(EditorEvent::DocumentClosed { doc });
            self.take_pending_for(doc);
        }
    }

    fn take_pending_for(&mut self, doc: DocumentId) {
        let ids: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, p)| match p {
                Pending::Open { doc: d, .. }
                | Pending::CheckUnchanged { doc: d }
                | Pending::Save { doc: d, .. } => *d == doc,
            })
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.pending.remove(&id);
            self.jobs.cancel(id);
        }
    }

    /// ファイルを開く (03 §9.2)。Loading の文書を作ってすぐ読込ジョブを投入する。
    /// すでに文書があるときは、読込が終わるまで今の文書を表示し続け、完了で置き換える。
    pub fn open_path(&mut self, raw_path: &str, opts: OpenOptions) -> CommandResult {
        // 読込の途中の置き換えの重なりは受け付けない (置き換え対象の文書が孤立しないように)
        if self.open_in_progress() {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        let Ok(path) = self.paths.normalize(raw_path) else {
            self.message(MessageLevel::Error, "msg.open.failed");
            return CommandResult::Rejected(RejectReason::InvalidArgs);
        };
        let replace = self.active_document();
        if let Some(cur) = replace.and_then(|d| self.document(d)) {
            if cur.is_dirty() {
                return CommandResult::Rejected(RejectReason::NeedsInput);
            }
            if cur.path.as_ref() == Some(&path) {
                self.message(MessageLevel::Info, "info.open.already_open");
                return CommandResult::Done;
            }
        }
        let mut doc = Document::new_untitled();
        doc.path = Some(path.clone());
        doc.load_state = crate::document::LoadState::Loading;
        let (doc_id, view) = self.add_document(doc);
        if replace.is_none() {
            self.show_view(view, None);
        }
        let job = self.jobs.spawn(
            Some(doc_id),
            eve_view::Revision::INITIAL,
            JobKind::Open {
                path: path.clone(),
                opts,
            },
        );
        self.pending.insert(
            job,
            Pending::Open {
                doc: doc_id,
                view,
                path,
                replace,
            },
        );
        CommandResult::Pending(job)
    }

    /// 上書き保存 (03 §9.3)。パスのない文書は名前の入力が要る。
    pub fn request_save(&mut self) -> CommandResult {
        let Some(doc_id) = self.active_document() else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(doc) = self.document(doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(path) = doc.path.clone() else {
            return CommandResult::Rejected(RejectReason::NeedsInput);
        };
        if self.save_in_progress(doc_id) {
            self.message(MessageLevel::Info, "info.save.in_progress");
            return CommandResult::Done;
        }
        if !doc.is_dirty() {
            // 手順 0: 未編集なら、ディスク上のファイルも変わっていないことを確かめてから書かない
            let job = self.jobs.spawn(
                Some(doc_id),
                doc.revision(),
                JobKind::Metadata {
                    path,
                    purpose: MetaPurpose::UnchangedSave,
                },
            );
            self.pending
                .insert(job, Pending::CheckUnchanged { doc: doc_id });
            return CommandResult::Pending(job);
        }
        let expected = doc.disk_stamp.map_or(Expect::Absent, Expect::Stamp);
        self.start_save(doc_id, path, expected)
    }

    /// 名前を付けて保存 (パスを指定する)。
    pub fn request_save_as(&mut self, raw_path: &str) -> CommandResult {
        let Some(doc_id) = self.active_document() else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Ok(path) = self.paths.normalize(raw_path) else {
            self.message(MessageLevel::Error, "msg.save.invalid_path");
            return CommandResult::Rejected(RejectReason::InvalidArgs);
        };
        let Some(doc) = self.documents.get(&doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        // 読込中・読込に失敗した文書は保存できない (中身が揃っていない)
        if doc.load_state != crate::document::LoadState::Ready {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        if self.save_in_progress(doc_id) {
            self.message(MessageLevel::Info, "info.save.in_progress");
            return CommandResult::Done;
        }
        let same = doc.path.as_ref() == Some(&path);
        // 読み取り専用の文書を同じファイルへ書くと、元のバイト列を損なうことがある (03 §9.7 (a) の 1)
        if same && !doc.is_editable() {
            self.message(MessageLevel::Warn, "msg.save_as.same_file_read_only");
            return CommandResult::Rejected(RejectReason::ReadOnly);
        }
        // 同じファイルなら読んだときの Stamp で照合する。別のファイルへは存在しないことを期待する
        // (既存なら Conflict。上書きの確認は確認ダイアログ (後続) の仕事)。パスと Stamp は成功してから更新する
        let expected = if same {
            doc.disk_stamp.map_or(Expect::Absent, Expect::Stamp)
        } else {
            Expect::Absent
        };
        self.start_save(doc_id, path, expected)
    }

    fn start_save(&mut self, doc_id: DocumentId, path: FsPath, expected: Expect) -> CommandResult {
        let Some(doc) = self.documents.get_mut(&doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let (bom, pua) = (doc.format.has_bom, doc.format.pua_escape);
        let revision = doc.revision();
        let (snapshot, at) = doc.begin_save();
        let job = self.jobs.spawn(
            Some(doc_id),
            revision,
            JobKind::Save {
                path: path.clone(),
                snapshot,
                bom,
                pua,
                expected,
            },
        );
        self.pending.insert(
            job,
            Pending::Save {
                doc: doc_id,
                at,
                revision,
                path,
            },
        );
        CommandResult::Pending(job)
    }

    /// ジョブの結果を受け取る (02 §5.4)。
    pub fn on_job_result(&mut self, r: JobResult) {
        let Some(pending) = self.pending.remove(&r.job()) else {
            return;
        };
        match pending {
            Pending::Open {
                doc,
                view,
                path,
                replace,
            } => self.finish_open(doc, view, path, replace, r),
            Pending::CheckUnchanged { doc } => self.finish_check(doc, r),
            Pending::Save {
                doc,
                at,
                revision,
                path,
            } => self.finish_save(doc, at, revision, &path, r),
        }
        self.mark_dirty_render();
    }

    fn finish_open(
        &mut self,
        doc_id: DocumentId,
        view: ViewId,
        path: FsPath,
        replace: Option<DocumentId>,
        r: JobResult,
    ) {
        match r {
            JobResult::Complete {
                text,
                format,
                meta: _,
                stamp,
                changed_during_load,
                detected,
                ..
            } => {
                let mut d = Document::from_text(text, format);
                d.path = Some(path);
                d.disk_stamp = Some(stamp);
                d.read_only = detected;
                self.documents.insert(doc_id, d);
                if changed_during_load {
                    self.message(MessageLevel::Warn, "msg.open.changed_during_load");
                }
                self.complete_open(doc_id, view, replace);
            }
            JobResult::NewFile { parent_missing, .. } => {
                // 存在しないパス: 空の新規文書 (dirty ではない)。保存すると作成する
                let mut d = Document::new_untitled();
                d.path = Some(path);
                self.documents.insert(doc_id, d);
                let key = if parent_missing.is_some() {
                    "msg.open.parent_missing"
                } else {
                    "info.open.new_file"
                };
                self.message(
                    if parent_missing.is_some() {
                        MessageLevel::Warn
                    } else {
                        MessageLevel::Info
                    },
                    key,
                );
                self.complete_open(doc_id, view, replace);
            }
            JobResult::NeedsConfirm { ask, .. } => {
                // 確認ダイアログ (後続) ができるまでは開かない。今の文書はそのまま
                let key = match ask {
                    ConfirmAsk::Binary => "ui.confirm.open.binary",
                    ConfirmAsk::UnsupportedBom { .. } => "ui.confirm.open.utf32",
                };
                self.open_failed(doc_id, view, replace, MessageLevel::Warn, key);
            }
            JobResult::Failed { err, .. } => {
                let (level, key) = match err {
                    JobError::Cancelled => (MessageLevel::Info, "info.open.cancelled"),
                    JobError::Fs(e) => (MessageLevel::Error, open_error_key(&e)),
                    JobError::OutOfMemory { .. } => (MessageLevel::Error, "msg.open.out_of_memory"),
                    _ => (MessageLevel::Error, "msg.open.failed"),
                };
                self.open_failed(doc_id, view, replace, level, key);
            }
            _ => {}
        }
    }

    fn complete_open(&mut self, doc: DocumentId, view: ViewId, replace: Option<DocumentId>) {
        if replace.is_some() {
            self.show_view(view, replace);
        }
        self.mark_snapshot_update(doc);
    }

    fn open_failed(
        &mut self,
        doc: DocumentId,
        view: ViewId,
        replace: Option<DocumentId>,
        level: MessageLevel,
        key: &str,
    ) {
        self.message(level, key);
        if replace.is_some() {
            // 今の文書はそのまま残す (FR-107 AC-3)
            self.close_document(doc);
        } else {
            // 最初の文書の読込に失敗: 失敗した文書は閉じ、無題の文書で始める (再試行できる、03 §9.2 の「失敗」)
            let (_, untitled) = self.add_document(Document::new_untitled());
            self.show_view(untitled, Some(doc));
            let _ = view;
        }
    }

    fn finish_check(&mut self, doc_id: DocumentId, r: JobResult) {
        let Some(doc) = self.documents.get(&doc_id) else {
            return;
        };
        let unchanged = match &r {
            // 読んだときの Stamp と一致し、編集もなければ書き込まない
            JobResult::Meta { stamp, .. } => {
                !doc.is_dirty()
                    && doc.disk_stamp.is_some_and(|d| {
                        d.mtime == stamp.mtime && d.len == stamp.len && d.file_id == stamp.file_id
                    })
            }
            _ => false,
        };
        match r {
            _ if unchanged => self.message(MessageLevel::Info, "info.save.unchanged"),
            // NotFound は不一致として保存へ進む。その他の失敗は照合の失敗として示す
            JobResult::Failed {
                err: JobError::Fs(e),
                ..
            } if e.kind != FsErrorKind::NotFound => {
                self.message(MessageLevel::Error, "msg.save.check_failed");
            }
            JobResult::Failed {
                err: JobError::Cancelled,
                ..
            } => self.message(MessageLevel::Info, "info.save.aborted"),
            _ => {
                let Some((path, expected)) = self.documents.get(&doc_id).and_then(|d| {
                    Some((
                        d.path.clone()?,
                        d.disk_stamp.map_or(Expect::Absent, Expect::Stamp),
                    ))
                }) else {
                    return;
                };
                self.start_save(doc_id, path, expected);
            }
        }
    }

    fn finish_save(
        &mut self,
        doc_id: DocumentId,
        at: NodeId,
        revision: eve_view::Revision,
        path: &FsPath,
        r: JobResult,
    ) {
        match r {
            JobResult::Saved { report, .. } => {
                if let Some(doc) = self.documents.get_mut(&doc_id) {
                    // 名前を付けて保存のパスは、書けてから文書のパスにする (03 §9.3 手順 7)
                    doc.path = Some(path.clone());
                    doc.finish_save(at, Some(report.new_stamp));
                }
                self.push_event(EditorEvent::DocumentSaved {
                    doc: doc_id,
                    revision,
                    path: path.display(),
                });
                self.message(MessageLevel::Info, "info.save.done");
                self.mark_snapshot_update(doc_id);
            }
            JobResult::Failed { err, .. } => {
                let (level, key) = match err {
                    JobError::Cancelled => (MessageLevel::Info, "info.save.aborted"),
                    JobError::Fs(e) => (MessageLevel::Error, save_error_key(&e)),
                    _ => (MessageLevel::Error, "msg.save.failed"),
                };
                self.message(level, key);
            }
            _ => {}
        }
    }
}

/// 開く失敗の MessageKey (04 §15.2)。
fn open_error_key(e: &FsError) -> &'static str {
    match (&e.kind, &e.detail) {
        (FsErrorKind::PermissionDenied, _) => "msg.open.permission_denied",
        (FsErrorKind::Busy, _) => "msg.open.busy",
        (_, Some(FsErrorDetail::NotAFile)) => "msg.open.not_a_file",
        (_, Some(FsErrorDetail::ReservedName)) => "msg.open.reserved_name",
        _ => "msg.open.failed",
    }
}

/// 保存失敗の MessageKey (04 §15.2)。失敗の後の選択肢 (確認ダイアログ) は後続。
fn save_error_key(e: &FsError) -> &'static str {
    match (&e.kind, &e.detail) {
        (FsErrorKind::PermissionDenied, _) => "msg.save.permission_denied",
        (FsErrorKind::DiskFull, _) => "msg.save.disk_full",
        (FsErrorKind::ReadOnlyFs, _) => "msg.save.read_only_fs",
        (FsErrorKind::InvalidPath, _) => "msg.save.invalid_path",
        (FsErrorKind::Busy, _) => "msg.save.busy",
        (FsErrorKind::NotFound, Some(FsErrorDetail::ParentMissing { .. })) => {
            "msg.save.parent_missing"
        }
        _ => "msg.save.failed",
    }
}

// ---------- コマンド ----------

fn arg_path(ctx: &CommandCtx<'_>) -> Option<String> {
    ctx.args.str("path").map(str::to_owned)
}

fn file_new(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.new_untitled()
}

fn file_open(ctx: &mut CommandCtx<'_>) -> CommandResult {
    match arg_path(ctx) {
        Some(p) => ctx.state.open_path(&p, OpenOptions::default()),
        // パスがなければ入力プロンプトで尋ねる (オーバーレイと同時)
        None => CommandResult::Rejected(RejectReason::NeedsInput),
    }
}

fn file_save(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.request_save()
}

fn file_save_as(ctx: &mut CommandCtx<'_>) -> CommandResult {
    match arg_path(ctx) {
        Some(p) => ctx.state.request_save_as(&p),
        None => CommandResult::Rejected(RejectReason::NeedsInput),
    }
}

macro_rules! reg {
    ($r:expr, $id:expr, $kind:expr, $when:expr, $h:expr) => {
        $r.register(
            CommandSpec::new($id, $kind, Permission::None, $when, $h)
                .expect("カタログの when 句は構文が正しい"),
        )
    };
}

/// v0.1 のファイルコマンドのうち、確認 UI なしで動くもの (07 §11.6 の「ファイル・アプリ」)。
pub(super) fn register(r: &mut CommandRegistry) {
    reg!(r, "file.new", CommandKind::File, "", file_new);
    reg!(r, "file.open", CommandKind::File, "", file_open);
    reg!(
        r,
        "file.save",
        CommandKind::File,
        "!doc.read_only",
        file_save
    );
    reg!(r, "file.save_as", CommandKind::File, "", file_save_as);
}
