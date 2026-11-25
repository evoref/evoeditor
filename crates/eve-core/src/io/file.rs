//! ファイルの開く・保存の共通の補助 (docs/03 §9.2 / §9.3): 保存中・読込中の問い合わせ、読込の取消し、
//! 失敗の理由の文言、ディスクの状態の比較 (文書の表示の切り替えは editor/buffer.rs、起動時の走査は io/startup.rs)。
//!
//! 本体は STR-B1 (docs/review/v0.3-findings.md) で分けた: ジョブの表と結果の振り分けは `pending`、開くは `open`、保存は `save`、
//! 表現できない文字は `unmappable`、開き直し・再読込は `reload`、`file.*` のコマンドは `file_cmds`。

use eve_view::{DocumentId, EditorEvent, MessageLevel};

use crate::document::Document;
use crate::io::fs::Expect;
use crate::io::fspath::FsPath;
use crate::io::io_jobs::OpenOptions;
use crate::jobs::{JobKind, MetaPurpose};
use crate::ui_model::keys;
use eve_encoding::Encoding;

use super::open::*;
use super::pending::*;
use crate::command::{CommandResult, RejectReason};
use crate::editor::state::EditorState;
use crate::ui_model::overlay::{Continuation, PromptKind};

impl EditorState {
    /// `MessageKey` を解決して表示する。`path` は `{path}` (表示用のパス)。
    pub(crate) fn message(&mut self, level: MessageLevel, key: &str, path: &str) {
        self.show_message(level, key, &[("path", path)]);
    }

    /// 保存ジョブの完了を待っているか (ステータスバーの `[保存中]`)。
    pub(crate) fn is_saving(&self, doc: DocumentId) -> bool {
        self.io
            .pending
            .values()
            .any(|p| matches!(p, Pending::Save { doc: d, .. } if *d == doc))
    }

    /// 読込を待っている間、置き換え対象の今の文書は編集を受け付けない (03 §9.2 の置き換えの手順)。
    /// 編集は読込の完了で黙って捨てられてしまうため。
    pub(crate) fn open_in_progress(&self) -> bool {
        self.io
            .pending
            .values()
            .any(|p| matches!(p, Pending::Open { .. }))
    }

    pub(crate) fn save_in_progress(&self, doc_id: DocumentId) -> bool {
        self.io.pending.values().any(|p| match p {
            Pending::Save { doc, .. } | Pending::CheckUnchanged { doc } => *doc == doc_id,
            _ => false,
        }) || self.save_as_checking(Some(doc_id)).is_some()
    }

    /// 表示していない文書の、新しく開く読込 (`file.open`・起動時の 2 つ目以降のパス) のジョブ。ステータスバーが
    /// 読込中として示し、`Esc` が取り消す読込 (表示中の文書が読込中でないとき。03 §9.8 の「読込中のバッファ」)。
    /// ほかの文書の再読込・開き直しは含めない。
    fn hidden_open_loads(&self) -> impl Iterator<Item = (eve_view::JobId, DocumentId)> + '_ {
        let active = self.active_document();
        self.io.pending.iter().filter_map(move |(id, p)| match p {
            Pending::Open {
                doc,
                kind: OpenKind::Open,
                ..
            } if Some(*doc) != active => Some((*id, *doc)),
            _ => None,
        })
    }

    /// 表示していない文書の新しく開く読込 (`hidden_open_loads`) の進捗 (百分率)。無ければ `None`。
    /// 表示中の文書が読込中なら、ステータスバーはその文書の進捗を出す (`ui_model/chrome.rs`)。
    pub(crate) fn loading_percent(&self) -> Option<u8> {
        self.hidden_open_loads()
            .find_map(|(_, doc)| self.docs.documents.get(&doc).map(|d| d.load_percent))
    }

    /// `Esc` (`ui.cancel`) で読込を取り消す (03 §9.8 の「読込中のバッファ」) 1 段目: 表示中の文書が読込中 (`Loading` /
    /// `Partial`。開く・再読込・開き直しのどれでも) ならその文書の読込を取り消す。2 段目の `cancel_hidden_loading` は、
    /// 間に名前を付けて保存の照合の取消しをはさんで呼ぶ (03 §12.7.6 の細目 (9))。ほかの文書の再読込・開き直しは取り消さない。
    /// 読込中の標準入力の文書なら、取り消さずに読むのをやめるかを尋ねる。取り消すものがあれば `true`。
    pub(crate) fn cancel_own_loading(&mut self) -> bool {
        // 表示中の文書が読込中の標準入力の文書なら、取り消さずに読むのをやめるかを尋ねる (03 §9.2.2 の「取消し (`Esc`)」)。
        // 表示していない標準入力の読込には触れない (下はファイルの読込 `Pending::Open` だけを見る)
        if self.confirm_stdin_stop() {
            return true;
        }
        let active = self.active_document();
        let own: Vec<_> = self
            .io
            .pending
            .iter()
            .filter(|(_, p)| matches!(p, Pending::Open { doc, .. } if Some(*doc) == active))
            .map(|(id, _)| *id)
            .collect();
        self.cancel_loads(own)
    }

    /// 読込のジョブ `ids` を取り消す。取り消し済み (結果を待っているだけ) のものは数えない (次の `Esc` を次の段へ通す。
    /// 03 §12.7.6 の細目 (9))。取り消したものがあれば `true`。
    fn cancel_loads(&mut self, ids: Vec<eve_view::JobId>) -> bool {
        // 結果が届いて表から消えた読込の印は捨てる
        let pending = &self.io.pending;
        self.io.cancelled_loads.retain(|j| pending.contains_key(j));
        let fresh: Vec<_> = ids
            .into_iter()
            .filter(|id| !self.io.cancelled_loads.contains(id))
            .collect();
        for id in &fresh {
            self.jobs.cancel(*id);
            self.io.cancelled_loads.insert(*id);
        }
        !fresh.is_empty()
    }

    /// 表示していない文書の新しく開く読込を取り消す (表示中の文書が読込中でないときに呼ぶ)。取り消すものがあれば `true`。
    pub(crate) fn cancel_hidden_loading(&mut self) -> bool {
        let ids: Vec<_> = self.hidden_open_loads().map(|(id, _)| id).collect();
        self.cancel_loads(ids)
    }

    /// 今の文書 (アクティブな文書 = フォーカスのペインの文書。03 §9.9 の PANE-8)。
    pub fn active_document(&self) -> Option<DocumentId> {
        let w = self.focused_window()?;
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
    ///
    /// 表示しているペインは、先に次の文書へ替える (03 §9.9 の PANE-9。ペインに優先して出す文書があるときは、呼び側が
    /// `close_shown_document` で替える)。ペインが消えた View を指したままにならない。
    pub fn close_document(&mut self, doc: DocumentId) {
        // 監視の登録を外し、その文書の待ち合わせ・照合の回を捨てる (閉じた文書の古い知らせを使わない、03 §9.5.1)
        self.watch_forget(doc);
        self.move_panes_off(doc, None);
        debug_assert!(
            self.panes_showing(doc).is_empty(),
            "表示中の文書 {doc:?} のペインを替えられなかった"
        );
        self.io.open_jumps.remove(&doc);
        self.io.ws_jumps.remove(&doc);
        // 名前を付けて保存の識別の覚えと保留した照合の結果も捨てる (03 §12.7.6 の細目 (10))
        self.io.save_as_targets.remove(&doc);
        self.io.held_save_as.retain(|(a, _)| a.doc != doc);
        // 先行表示の先頭は文書ごと捨てる (03 §9.2.1 の「失敗・取消し・閉じる」)
        self.forget_partial_state(doc);
        self.docs.views.retain(|_, v| v.doc != doc);
        // 全ペインの表・最近使った順からも消す (表示中のペインは呼び側が先に別の文書へ替える。03 §9.9 の PANE-1 / PANE-9)
        self.forget_document_in_panes(doc);
        // 全ペインのジャンプ履歴からも消す (閉じた文書を開き直さない、03 §5.9 の JMP-5)
        self.forget_jumps_of(doc);
        let saving = self
            .io
            .pending
            .values()
            .any(|p| matches!(p, Pending::Save { doc: d, .. } if *d == doc));
        let retained = saving
            .then(|| self.rescue_snapshot(doc, crate::io::snapshot::SnapshotScope::Rescue))
            .flatten();
        if self.docs.documents.remove(&doc).is_some() {
            if let Some(snap) = retained {
                self.io.closing_snapshots.insert(doc, snap);
            }
            self.forget_save_transform(doc);
            // 保存中に積んだ保存要求も捨てる (閉じた文書は再実行しない、03 §9.3 手順 4)
            self.io.save_queue.remove(&doc);
            self.io.held_queued_saves.retain(|(d, _)| *d != doc);
            // 退避用の公開物から外す (破棄した内容を緊急退避に残さない。保存中の文書は上で残している)
            self.mark_snapshot_update(doc);
            self.push_event(EditorEvent::DocumentClosed { doc });
            self.take_pending_for(doc);
            // 文書の数が変わるとタブラインの出し入れが変わる (03 §9.9 の PANE-12 (b))
            self.sync_tab_line();
        }
    }

    /// ファイルを開く (03 §9.2)。Loading の文書を作ってすぐ読込ジョブを投入する。
    /// すでに文書があるときは、読込が終わるまで今の文書を表示し続け、完了で置き換える。
    pub fn open_path(&mut self, raw_path: &str, opts: OpenOptions) -> CommandResult {
        let Ok(path) = self.paths.normalize(raw_path) else {
            self.show_message(
                MessageLevel::Error,
                keys::MSG_OPEN_FAILED,
                &[("path", raw_path), ("reason", "invalid path")],
            );
            return CommandResult::Rejected(RejectReason::InvalidArgs);
        };
        self.open_fs_path(path, opts)
    }

    /// 正規化した絶対パスを開く (`open_path` の後半。ファインダは候補のパスを文字列にせずに渡す。名前が UTF-8 で表せない
    /// ファイルも開ける、03 §12.7.4 の「開く」)。`opts` の設定の値 (改行コード・閾値) が無ければ今の設定で埋める。
    pub(crate) fn open_fs_path(&mut self, path: FsPath, opts: OpenOptions) -> CommandResult {
        let opts = OpenOptions {
            // 新しく開く読込なので、先行表示を送ってよい (03 §9.2.1。再読込・開き直しは io/reload.rs で立てない)
            partial: true,
            default_eol: opts.default_eol.or(Some(self.config_eol())),
            huge_file_warn_bytes: opts
                .huge_file_warn_bytes
                .or(Some(self.config().huge_file_warn_bytes)),
            long_line_threshold: opts
                .long_line_threshold
                .or(Some(self.config_long_line_threshold())),
            ..opts
        };
        // 今の文書は置き換えない (03 §9.8)。同じパスが既に開いていれば、そのバッファへ切り替える
        let existing = self
            .docs
            .documents
            .iter()
            .find(|(_, d)| d.path.as_ref() == Some(&path))
            .map(|(id, _)| *id);
        if let Some(id) = existing {
            if self.active_document() == Some(id) {
                self.message(
                    MessageLevel::Info,
                    keys::INFO_OPEN_ALREADY_OPEN,
                    &path.display(),
                );
            } else {
                self.activate_document(id);
            }
            return CommandResult::Done;
        }
        // 読込が終わったとき、その間に開いた時点のペインを別の文書へ切り替えていなければ、そのペインを新しいバッファへ
        // 切り替える (03 §9.9 の PANE-8 (b))
        let replace = self
            .focused_pane()
            .zip(self.active_document())
            .map(|(pane, doc)| crate::io::open::OpenFrom { pane, doc });
        let mut doc = Document::new_untitled();
        doc.path = Some(path.clone());
        doc.load_state = crate::document::LoadState::Loading;
        let (doc_id, view) = self.add_document(doc);
        if replace.is_none() {
            self.show_view(view, None);
        }
        let known_ids = self.known_file_ids(Some(doc_id));
        let job = self.jobs.spawn(
            Some(doc_id),
            eve_view::Revision::INITIAL,
            JobKind::Open {
                path: path.clone(),
                opts,
                known_ids,
            },
        );
        self.io.pending.insert(
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
        self.request_save_for(doc_id)
    }

    /// `request_save` の対象を文書 `doc_id` に固定する版 (保存中に積んだ `file.save` の再実行、03 §9.3 手順 4)。
    /// 保存の連鎖の途中なら、単独の保存を待つ間は深さ 1 の待ち行列に積み、ほかは無視する (03 §9.7 の全状態の遷移表)。
    ///
    /// spec: 03 §9.3, 03 §9.7
    pub(crate) fn request_save_for(&mut self, doc_id: DocumentId) -> CommandResult {
        // 名前の入力 (無題の文書)・未編集の照合 (手順 0) より先に拒否する (03 §9.7 の「連鎖の開始時の拒否」)
        if self.reject_stateful_invalid(doc_id, None) {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        // 保存の連鎖を始める (保存時の変換の印、03 §9.3.1 の SVT-4 (c)。保存中なら前の連鎖の印を保つ)
        self.begin_save_chain(doc_id, crate::command::ids::FILE_SAVE);
        // 保存の連鎖の途中なら積むか無視する。名前の入力より先に見る (無題の文書の最初の保存の間も積む、03 §9.7 の
        // 全状態の遷移表)
        let Some(shown) = self.save_abort_path(doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        if let Some(r) =
            self.queue_or_ignore_save_request(doc_id, super::save::QueuedSave::Save, &shown)
        {
            return r;
        }
        let Some(doc) = self.document(doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(path) = doc.path.clone() else {
            // パスがなければ名前を尋ねる
            self.open_prompt(
                PromptKind::SaveAs {
                    doc: doc_id,
                    then: None,
                },
                String::new(),
            );
            return CommandResult::Done;
        };
        if !doc.is_dirty() {
            // 手順 0: 未編集なら、ディスク上のファイルも変わっていないことを確かめてから書かない
            let job = self.jobs.spawn(
                Some(doc_id),
                doc.revision(),
                JobKind::Metadata {
                    path,
                    purpose: MetaPurpose::UnchangedSave,
                    known_ids: Vec::new(),
                },
            );
            self.io
                .pending
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
        self.set_encoding_for(doc_id, enc, bom)
    }

    /// `set_encoding` の対象を文書 `doc_id` に固定する版 (文字コードの入力の確定。対象は入力を開いた時点の文書、
    /// 03 §9.8 の「入力・確認の対象」)。
    pub(crate) fn set_encoding_for(
        &mut self,
        doc_id: DocumentId,
        enc: Option<Encoding>,
        bom: Option<bool>,
    ) -> CommandResult {
        let Some(doc) = self.docs.documents.get(&doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        if doc.load_state != crate::document::LoadState::Ready {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        let Some(enc) = enc else {
            self.open_prompt(PromptKind::SetEncoding { doc: doc_id, bom }, String::new());
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
        // 書式が決まった直後、名前の入力 (無題・読み取り専用の文書) より先に拒否する (03 §9.7 の「連鎖の開始時の拒否」)
        if self.reject_stateful_invalid(doc_id, Some(fmt)) {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        self.begin_save_chain(doc_id, crate::command::ids::FILE_SET_ENCODING);
        let Some(doc) = self.docs.documents.get(&doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let path = doc.path.clone();
        let shown = path.as_ref().map(FsPath::display).unwrap_or_default();
        if let Some(r) = self.queue_or_ignore_save_request(
            doc_id,
            super::save::QueuedSave::SetEncoding(fmt),
            &shown,
        ) {
            return r;
        }
        let Some(doc) = self.docs.documents.get(&doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        match path {
            Some(path) if doc.is_editable() => {
                let expected = doc.disk_stamp.map_or(Expect::Absent, Expect::Stamp);
                self.start_save_fmt(doc_id, path, expected, None, Some(fmt))
            }
            other => {
                let initial = other.as_ref().map(FsPath::display).unwrap_or_default();
                self.open_prompt(PromptKind::SaveAsFormat { doc: doc_id, fmt }, initial);
                CommandResult::Done
            }
        }
    }

    /// `file.reopen_with_encoding`。`enc` が無ければ文字コードを尋ねる。dirty なら保存か破棄を求める (FR-111 AC-2)。
    pub fn reopen_with_encoding(&mut self, enc: Option<Encoding>) -> CommandResult {
        let Some(doc_id) = self.active_document() else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        self.reopen_with_encoding_for(doc_id, enc)
    }

    /// `reopen_with_encoding` の対象を文書 `doc_id` に固定する版 (文字コードの入力の確定・「編集を許可」の確認の
    /// 「文字コードを指定して開き直す」。対象は入力・確認を開いた時点の文書、03 §9.8 の「入力・確認の対象」)。
    pub(crate) fn reopen_with_encoding_for(
        &mut self,
        doc_id: DocumentId,
        enc: Option<Encoding>,
    ) -> CommandResult {
        let Some(doc) = self.docs.documents.get(&doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(path) = doc.path.clone() else {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        };
        if doc.load_state != crate::document::LoadState::Ready || self.open_in_progress() {
            self.message(MessageLevel::Info, keys::INFO_RELOAD_BUSY, &path.display());
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        // 保存ジョブの実行中はその完了を待たせる (保存が終わってから実行し直す)
        if self.save_in_progress(doc_id) {
            self.message(
                MessageLevel::Info,
                keys::INFO_SAVE_IN_PROGRESS,
                &path.display(),
            );
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        let Some(enc) = enc else {
            self.open_prompt(PromptKind::ReopenEncoding { doc: doc_id }, String::new());
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
        let Some(doc) = self.docs.documents.get(&doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(path) = doc.path.clone() else {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        };
        if doc.load_state != crate::document::LoadState::Ready || self.open_in_progress() {
            self.message(MessageLevel::Info, keys::INFO_RELOAD_BUSY, &path.display());
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        if self.save_in_progress(doc_id) {
            self.message(
                MessageLevel::Info,
                keys::INFO_SAVE_IN_PROGRESS,
                &path.display(),
            );
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
                message: None,
                action: ConfirmAction::Reload(doc_id),
            });
            return CommandResult::Done;
        }
        self.start_reload(doc_id)
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
