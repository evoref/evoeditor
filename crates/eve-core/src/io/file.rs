//! ファイルの開く・保存の共通の補助 (docs/03 §9.2 / §9.3): 保存中・読込中の問い合わせ、読込の取消し、
//! 失敗の理由の文言、ディスクの状態の比較 (文書の表示の切り替えは editor/buffer.rs、起動時の走査は io/startup.rs)。
//!
//! 本体は STR-B1 (docs/review/v0.3-findings.md) で分けた: ジョブの表と結果の振り分けは `pending`、開くは `open`、保存は `save`、
//! 表現できない文字は `unmappable`、開き直し・再読込は `reload`、`file.*` のコマンドは `file_cmds`。

use eve_view::{DocumentId, EditorEvent, MessageLevel};

use crate::document::Document;
use crate::io::fs::{Expect, FsError};
use crate::io::fspath::FsPath;
use crate::io::io_jobs::OpenOptions;
use crate::jobs::{JobError, JobKind, MetaPurpose};
use eve_encoding::Encoding;

use super::open::*;
use super::pending::*;
use crate::command::{CommandResult, RejectReason};
use crate::editor::state::EditorState;
use crate::ui_model::overlay::{Continuation, PromptKind};

impl EditorState {
    /// MessageKey を解決して表示する。`path` は `{path}` (表示用のパス)。
    pub(crate) fn message(&mut self, level: MessageLevel, key: &str, path: &str) {
        self.show_message(level, key, &[("path", path)]);
    }

    /// 保存ジョブの完了を待っているか (ステータスバーの `[保存中]`)。
    pub(crate) fn is_saving(&self, doc: DocumentId) -> bool {
        self.pending
            .values()
            .any(|p| matches!(p, Pending::Save { doc: d, .. } if *d == doc))
    }

    /// 読込を待っている間、置き換え対象の今の文書は編集を受け付けない (03 §9.2 の置き換えの手順)。
    /// 編集は読込の完了で黙って捨てられてしまうため。
    pub(crate) fn open_in_progress(&self) -> bool {
        self.pending
            .values()
            .any(|p| matches!(p, Pending::Open { .. }))
    }

    pub(crate) fn save_in_progress(&self, doc_id: DocumentId) -> bool {
        self.pending
            .values()
            .any(|p| matches!(p, Pending::Save { doc, .. } | Pending::CheckUnchanged { doc } if *doc == doc_id))
    }

    /// 進行中の読込 (開く・開き直し・再読込) の進捗 (百分率)。無ければ `None`。
    pub(crate) fn loading_percent(&self) -> Option<u8> {
        self.pending.values().find_map(|p| match p {
            Pending::Open { doc, .. } => self.documents.get(doc).map(|d| d.load_percent),
            _ => None,
        })
    }

    /// 読込中の文書の読込ジョブを取り消す。取り消すものがあれば `true`。
    pub(crate) fn cancel_loading(&mut self) -> bool {
        let ids: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, p)| matches!(p, Pending::Open { .. }))
            .map(|(id, _)| *id)
            .collect();
        for id in &ids {
            self.jobs.cancel(*id);
        }
        !ids.is_empty()
    }

    /// 今の文書 (最初の Window の View の文書)。
    pub fn active_document(&self) -> Option<DocumentId> {
        let w = self.first_window()?;
        self.view(self.window_view(w)?).map(|v| v.doc)
    }

    /// 無題の新規文書を作る (FR-124)。今の文書があれば置き換える。dirty なら保存・破棄・キャンセルの確認を出す。
    pub fn new_untitled(&mut self) -> CommandResult {
        // 今の文書は置き換えず、無題のバッファを足して切り替える (03 §9.8)。何も捨てないので確認は要らない
        let doc = self.blank_document();
        let (id, _) = self.add_document(doc);
        self.activate_document(id);
        CommandResult::Done
    }

    /// 文書を閉じる。保存ジョブが実行中なら、その保存の結果が届くまで内容を緊急退避の対象に残す
    /// (結果が失敗なら退避先に書く。03 §9.2 の閉じる手順 (a))。
    pub fn close_document(&mut self, doc: DocumentId) {
        self.open_jumps.remove(&doc);
        self.views.retain(|_, v| v.doc != doc);
        let saving = self
            .pending
            .values()
            .any(|p| matches!(p, Pending::Save { doc: d, .. } if *d == doc));
        let retained = saving.then(|| self.rescue_snapshot(doc, true)).flatten();
        if self.documents.remove(&doc).is_some() {
            if let Some(snap) = retained {
                self.closing_snapshots.insert(doc, snap);
            }
            // 退避用の公開物から外す (破棄した内容を緊急退避に残さない。保存中の文書は上で残している)
            self.mark_snapshot_update(doc);
            self.push_event(EditorEvent::DocumentClosed { doc });
            self.take_pending_for(doc);
        }
    }

    /// ファイルを開く (03 §9.2)。Loading の文書を作ってすぐ読込ジョブを投入する。
    /// すでに文書があるときは、読込が終わるまで今の文書を表示し続け、完了で置き換える。
    pub fn open_path(&mut self, raw_path: &str, opts: OpenOptions) -> CommandResult {
        let opts = OpenOptions {
            default_eol: opts.default_eol.or(Some(self.config_eol())),
            huge_file_warn_bytes: opts
                .huge_file_warn_bytes
                .or(Some(self.config().huge_file_warn_bytes)),
            long_line_threshold: opts
                .long_line_threshold
                .or(Some(self.config_long_line_threshold())),
            ..opts
        };
        let Ok(path) = self.paths.normalize(raw_path) else {
            self.show_message(
                MessageLevel::Error,
                "msg.open.failed",
                &[("path", raw_path), ("reason", "invalid path")],
            );
            return CommandResult::Rejected(RejectReason::InvalidArgs);
        };
        // 今の文書は置き換えない (03 §9.8)。同じパスが既に開いていれば、そのバッファへ切り替える
        let existing = self
            .documents
            .iter()
            .find(|(_, d)| d.path.as_ref() == Some(&path))
            .map(|(id, _)| *id);
        if let Some(id) = existing {
            if self.active_document() == Some(id) {
                self.message(
                    MessageLevel::Info,
                    "info.open.already_open",
                    &path.display(),
                );
            } else {
                self.activate_document(id);
            }
            return CommandResult::Done;
        }
        // 読込が終わったとき、その間に別の文書へ切り替えていなければ新しいバッファへ切り替える
        let replace = self.active_document();
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
                opts,
                kind: OpenKind::Open,
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
            // パスがなければ名前を尋ねる
            self.open_prompt(PromptKind::SaveAs { then: None }, String::new());
            return CommandResult::Done;
        };
        if self.save_in_progress(doc_id) {
            self.message(MessageLevel::Info, "info.save.in_progress", &path.display());
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
        self.start_save(doc_id, path, expected, None)
    }

    /// `file.set_encoding`: 指定した文字コードで保存する (03 §8.8)。未編集でも常に書く。文書の書式は保存が成功するまで変えない。
    /// 文字コードが無ければ尋ねる。パスのない文書と読み取り専用の文書は、名前を付けて保存の流れへ新しい書式を渡す。
    pub fn set_encoding(&mut self, enc: Option<Encoding>, bom: Option<bool>) -> CommandResult {
        let Some(doc_id) = self.active_document() else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(doc) = self.documents.get(&doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        if doc.load_state != crate::document::LoadState::Ready {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        let Some(enc) = enc else {
            self.open_prompt(PromptKind::SetEncoding { bom }, String::new());
            return CommandResult::Done;
        };
        // BOM: UTF-16 は付ける、UTF-8 は今の文書のまま、それ以外は BOM を持たない (指定しても誤りにしない)
        let has_bom = match enc {
            Encoding::Utf16Le | Encoding::Utf16Be => bom.unwrap_or(true),
            Encoding::Utf8 => bom.unwrap_or(doc.format.has_bom),
            _ => false,
        };
        let fmt = NewFormat {
            encoding: enc,
            bom: has_bom,
        };
        let path = doc.path.clone();
        if self.save_in_progress(doc_id) {
            let shown = path.as_ref().map(FsPath::display).unwrap_or_default();
            self.message(MessageLevel::Info, "info.save.in_progress", &shown);
            return CommandResult::Done;
        }
        match path {
            Some(path) if doc.is_editable() => {
                let expected = doc.disk_stamp.map_or(Expect::Absent, Expect::Stamp);
                self.start_save_fmt(doc_id, path, expected, None, Some(fmt))
            }
            other => {
                let initial = other.as_ref().map(FsPath::display).unwrap_or_default();
                self.open_prompt(PromptKind::SaveAsFormat { fmt }, initial);
                CommandResult::Done
            }
        }
    }

    /// `file.reopen_with_encoding`。`enc` が無ければ文字コードを尋ねる。dirty なら保存か破棄を求める (FR-111 AC-2)。
    pub fn reopen_with_encoding(&mut self, enc: Option<Encoding>) -> CommandResult {
        let Some(doc_id) = self.active_document() else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(doc) = self.documents.get(&doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(path) = doc.path.clone() else {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        };
        if doc.load_state != crate::document::LoadState::Ready || self.open_in_progress() {
            self.message(MessageLevel::Info, "info.reload.busy", &path.display());
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        // 保存ジョブの実行中はその完了を待たせる (保存が終わってから実行し直す)
        if self.save_in_progress(doc_id) {
            self.message(MessageLevel::Info, "info.save.in_progress", &path.display());
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        let Some(enc) = enc else {
            self.open_prompt(PromptKind::ReopenEncoding, String::new());
            return CommandResult::Done;
        };
        if self.confirm_doc_if_dirty(doc_id, "unsaved_reopen", Continuation::Reopen(doc_id, enc)) {
            return CommandResult::Done;
        }
        self.start_reopen(doc_id, enc)
    }

    /// `file.reload`。dirty なら `ui.confirm.reload_discard` で確認する (既定はキャンセル、03 §9.5)。
    pub fn reload(&mut self) -> CommandResult {
        let Some(doc_id) = self.active_document() else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(doc) = self.documents.get(&doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(path) = doc.path.clone() else {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        };
        if doc.load_state != crate::document::LoadState::Ready || self.open_in_progress() {
            self.message(MessageLevel::Info, "info.reload.busy", &path.display());
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        if self.save_in_progress(doc_id) {
            self.message(MessageLevel::Info, "info.save.in_progress", &path.display());
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        if doc.is_dirty() {
            use crate::ui_model::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
            self.open_confirm(ConfirmState {
                name: "reload_discard",
                args: vec![("path".to_owned(), path.display())],
                choices: vec![ChoiceDef {
                    id: "reload",
                    accel: Some('l'),
                    destructive: true,
                }],
                // 既定はキャンセル (破壊的な選択肢は既定にしない)
                details: Vec::new(),
                default: 1,
                focused: 1,
                action: ConfirmAction::Reload(doc_id),
            });
            return CommandResult::Done;
        }
        self.start_reload(doc_id)
    }
}

/// `{reason}` に入れる短い分類名 (技術的な識別子。OS のエラーの分類文言 `ui.reason.*` は目録ができてから)。
pub(crate) fn reason_label(err: &JobError) -> String {
    match err {
        JobError::Fs(e) => fs_reason(e),
        JobError::Panic => "panic".to_owned(),
        JobError::OutOfMemory { .. } => "out of memory".to_owned(),
        JobError::Cancelled => "cancelled".to_owned(),
    }
}

/// 読んだ (書いた) ときの Stamp `d` と今の Stamp `now` が同じファイルの同じ内容を指すか (03 §9.3 手順 0・§9.5)。
pub(crate) fn same_on_disk(d: &crate::FileStamp, now: &crate::FileStamp) -> bool {
    // 内容ハッシュが双方にあれば、それで決める (mtime だけが違っても中身が同じなら変更なし、
    // mtime が同じでも中身が違えば変更あり。05 §5.5)
    let same_hash = d.hash.zip(now.hash).map(|(a, b)| a == b);
    d.len == now.len
        && d.file_id == now.file_id
        && match same_hash {
            Some(same) => same,
            None => d.mtime == now.mtime,
        }
}

pub(crate) fn fs_reason(e: &FsError) -> String {
    format!("{:?}", e.kind)
        .split([' ', '{'])
        .next()
        .unwrap_or("unknown")
        .to_lowercase()
}
