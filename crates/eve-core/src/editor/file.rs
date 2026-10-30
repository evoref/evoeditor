//! ファイルの開く・保存 (docs/03 §9.2 / §9.3 の M1 の範囲)。ジョブを投入し、結果で Document を更新する。
//!
//! M1 の範囲: UTF-8、原子的保存、未編集の保存の照合 (手順 0)。次は後続: 確認ダイアログ (dirty の置き換え、
//! 保存の失敗の選択肢、バイナリ・巨大ファイル)、名前の入力プロンプト、保存の連鎖 (Continuation)、保存中フラグ、
//! 外部変更の再読込。確認が要る場面は、確認の UI ができるまで `RejectReason::NeedsInput` で拒否する。

use eve_view::{DocumentId, EditorEvent, MessageLevel, ViewId};

use crate::document::{Document, NodeId};
use crate::fs::{
    Expect, FsError, FsErrorDetail, FsErrorKind, HardlinkPolicy, InPlaceBackup, SaveMethod,
};
use crate::fspath::FsPath;
use crate::io_jobs::OpenOptions;
use crate::jobs::{ConfirmAsk, JobError, JobKind, JobResult, MetaPurpose, SaveStrategy};
use eve_encoding::Encoding;

use super::command::{
    CommandCtx, CommandKind, CommandRegistry, CommandResult, CommandSpec, Permission, RejectReason,
};
use super::overlay::{Continuation, PromptKind};
use super::state::EditorState;

/// 投入したジョブと、その結果の宛先。
#[derive(Debug)]
pub(super) enum Pending {
    /// `config.reload` の読込 (07 §6)。
    ConfigReload,
    Open {
        doc: DocumentId,
        view: ViewId,
        path: FsPath,
        replace: Option<DocumentId>,
        /// 投入したときの指定 (確認の回答で足して再投入する、03 §9.2)。
        opts: OpenOptions,
        /// 用途。`Reopen` / `Reload` は同じ文書・View の中身を差し替える。
        kind: OpenKind,
    },
    /// 未編集の上書き保存の照合 (03 §9.3 手順 0)。
    CheckUnchanged { doc: DocumentId },
    /// 文書内の全件検索。状態は `EditorState::search` にある。
    Search,
    /// 単発の検索 (保持に無く、同期検索でも見つからなかった `search.next` / `search.prev`)。
    FindNext {
        doc: DocumentId,
        revision: eve_view::Revision,
        /// 投入時の選択の端。違っていたら結果を捨てる。
        cursor: eve_text::ByteOff,
        forward: bool,
    },
    /// 改行コードの変換の ChangeSet を作るジョブ (03 §8.4)。
    ConvertEol(super::eol::EolConvert),
    /// すべて置換の ChangeSet を作るジョブ (03 §12.5)。
    ReplaceAll {
        doc: DocumentId,
        revision: eve_view::Revision,
    },
    /// 起動時の退避ディレクトリの走査。
    RecoveryScan,
    /// スワップを書くジョブ (03 §10.2)。
    Swap { doc: DocumentId },
    /// スワップを消すジョブ。
    SwapDiscard { doc: DocumentId },
    /// 他のプロセスのスワップの本文を読むジョブ (復旧の適用)。`revision` は提案の時点の文書の Revision。
    SwapRead {
        doc: DocumentId,
        revision: eve_view::Revision,
        entry: Box<crate::SwapEntry>,
    },
    /// 置換途中の痕跡の操作のジョブ (05 §4.6)。
    Leftover(Box<super::leftover_offer::LeftoverAsk>),
    /// 他のプロセスのスワップを消すジョブ。`notify` なら結果を知らせる (利用者が選んだ破棄)。
    SwapRemove {
        entry: Box<crate::SwapEntry>,
        notify: bool,
    },
    /// 「退避先に保存」(03 §9.7 (e))。`path` は元のファイル。
    /// `release`: 閉じた文書の保存の失敗で書くとき、書き終えるまで緊急退避に残している公開物の文書。
    WriteRecovery {
        path: FsPath,
        release: Option<DocumentId>,
    },
    /// 表現不能の文字の置換の ChangeSet を作るジョブ (E3 の選択肢 2)。`revision` は投入時の文書の Revision。
    ReplaceUnmappable {
        ask: UnmappableAsk,
        revision: eve_view::Revision,
        retried: bool,
    },
    Save {
        doc: DocumentId,
        at: NodeId,
        revision: eve_view::Revision,
        path: FsPath,
        /// 確認の後に同じ保存をやり直すための照合の期待 (03 §9.3)。
        expected: Expect,
        /// `file.set_encoding` の新しい書式。保存の成功で文書の書式に確定する。
        fmt: Option<NewFormat>,
    },
}

/// `file.set_encoding` が保存に渡す新しい書式 (保存が成功するまで文書の書式は変えない、03 §8.8)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct NewFormat {
    pub encoding: Encoding,
    pub bom: bool,
}

/// 読込ジョブの用途。`Reopen` / `Reload` は同じ文書の中身を差し替える (03 §8.8 / §9.5)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OpenKind {
    Open,
    /// 文字コードを指定して開き直す: 履歴を消して差し替える (Undo できない)。
    Reopen,
    /// 再読込: `Origin::Reload` の Transaction で適用する (Undo できる)。
    Reload,
}

/// 開くときの確認 (`ui.confirm.open.*`、03 §9.2) に答えるための材料。読込中の文書 (Loading) は確認の間も残す。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct OpenAsk {
    pub doc: DocumentId,
    pub view: ViewId,
    pub path: FsPath,
    pub replace: Option<DocumentId>,
    pub opts: OpenOptions,
    pub ask: ConfirmAsk,
    /// 用途 (開き直し・再読込は同じ文書の中身を差し替える)。
    pub kind: OpenKind,
}

/// 保存の失敗の後の選択 (`ui.confirm.save_failed`、03 §9.7 (e))。
#[derive(Clone, Debug)]
pub(super) struct SaveFailedAsk {
    pub doc: DocumentId,
    pub path: FsPath,
    pub expected: Expect,
    pub cont: Option<Continuation>,
    /// 失敗した保存の時点の本文 (確認の間は編集できない)。「退避先に保存」で書く。
    pub snapshot: std::sync::Arc<crate::DocSnapshot>,
    pub fmt: Option<NewFormat>,
}

/// 表現不能の文字があって保存を中止した後の選択 (`ui.confirm.save.unmappable`、03 §8.6 の「E3 の選択肢」)。
#[derive(Clone, Debug)]
pub(super) struct UnmappableAsk {
    pub doc: DocumentId,
    pub path: FsPath,
    pub expected: Expect,
    pub cont: Option<Continuation>,
    /// 最初の表現不能の文字の通し番号 (Unicode スカラー値、0 始まり)。キャンセルでカーソルをここへ移す。
    pub first_char: u64,
    /// 保存先の文字コード (`file.set_encoding` の指定があればそれ)。
    pub encoding: Encoding,
    pub fmt: Option<NewFormat>,
}

/// 区分 B / D の確認 (別のバイト表現で保存される箇所がある、03 §8.6 E2')。承認したら同じ保存を投入し直す。
#[derive(Clone, Debug)]
pub(super) struct LossyAsk {
    pub doc: DocumentId,
    pub path: FsPath,
    pub expected: Expect,
    pub cont: Option<Continuation>,
    pub strategy: SaveStrategy,
    pub fmt: Option<NewFormat>,
}

/// 保存の代替の確認 (`AtomicUnsupported` / ハードリンク / `BackupFailed`、03 §9.3 手順 7・05 §4.4)。
#[derive(Clone, Debug)]
pub(super) struct FallbackAsk {
    pub kind: FallbackKind,
    pub doc: DocumentId,
    pub path: FsPath,
    pub expected: Expect,
    pub cont: Option<Continuation>,
    pub fmt: Option<NewFormat>,
    /// 競合 (`Conflict`) の確認のとき、照合で見つけた今の保存先の状態 (`None` = ファイルが無い)。「上書き」はこれを新しい期待値にする。
    pub conflict_current: Option<crate::FileStamp>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FallbackKind {
    /// 開いた後に既存のファイルが外部で変更された (`ui.confirm.save.conflict_changed`、03 §9.5)。選択肢: 上書き / 別名で保存 (既定)。
    ConflictChanged,
    /// 開いた後に既存のファイルが外部で削除された (`ui.confirm.save.conflict_deleted`)。「上書き」はファイルを作り直す。
    ConflictDeleted,
    /// 新規ファイルの保存先に、同名のファイルが後から作られていた (`ui.confirm.save.conflict_created`)。
    ConflictCreated,
    /// 原子的置換ができない (`ui.confirm.save.atomic_unsupported`)。選択肢: 直接上書き / 別名で保存 (既定)。
    AtomicUnsupported,
    /// ハードリンク (`ui.confirm.save.hardlink`)。選択肢: リンクを保って直接上書き / 切り離して保存。既定はキャンセル。
    Hardlink,
    /// 事前バックアップ・保存中フラグを書けなかった (`ui.confirm.save.overwrite_without_backup`)。選択肢: バックアップなしで上書き / 別名で保存。既定はキャンセル。
    NoBackup,
}

impl EditorState {
    /// MessageKey を解決して表示する。`path` は `{path}` (表示用のパス)。
    fn message(&mut self, level: MessageLevel, key: &str, path: &str) {
        self.show_message(level, key, &[("path", path)]);
    }

    /// 保存ジョブの完了を待っているか (ステータスバーの `[保存中]`)。
    pub(super) fn is_saving(&self, doc: DocumentId) -> bool {
        self.pending
            .values()
            .any(|p| matches!(p, Pending::Save { doc: d, .. } if *d == doc))
    }

    /// 読込を待っている間、置き換え対象の今の文書は編集を受け付けない (03 §9.2 の置き換えの手順)。
    /// 編集は読込の完了で黙って捨てられてしまうため。
    pub(super) fn open_in_progress(&self) -> bool {
        self.pending
            .values()
            .any(|p| matches!(p, Pending::Open { .. }))
    }

    pub(super) fn save_in_progress(&self, doc_id: DocumentId) -> bool {
        self.pending
            .values()
            .any(|p| matches!(p, Pending::Save { doc, .. } | Pending::CheckUnchanged { doc } if *doc == doc_id))
    }

    /// 進行中の読込 (開く・開き直し・再読込) の進捗 (百分率)。無ければ `None`。
    pub(super) fn loading_percent(&self) -> Option<u8> {
        self.pending.values().find_map(|p| match p {
            Pending::Open { doc, .. } => self.documents.get(doc).map(|d| d.load_percent),
            _ => None,
        })
    }

    /// 読込中の文書の読込ジョブを取り消す。取り消すものがあれば `true`。
    pub(super) fn cancel_loading(&mut self) -> bool {
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

    /// `doc` を表示する (最初の Window の View をその文書のものにする)。文書の View が無ければ作る。
    /// 検索バーは文書ごとには持たないので、切り替える前に閉じる (03 §9.8)。
    pub(super) fn activate_document(&mut self, doc: DocumentId) {
        if self.active_document() == Some(doc) || !self.documents.contains_key(&doc) {
            return;
        }
        self.search_close();
        if let Some(prev) = self.active_document() {
            self.recent.retain(|d| *d != prev);
            self.recent.push(prev);
        }
        let view = match self.views.iter().find(|(_, v)| v.doc == doc) {
            Some((id, _)) => *id,
            None => self.add_view(doc),
        };
        if let Some(w) = self.windows.first_mut() {
            w.root = view;
        }
        self.mark_dirty_render();
    }

    /// 無題の新規文書を作る (FR-124)。今の文書があれば置き換える。dirty なら保存・破棄・キャンセルの確認を出す。
    pub fn new_untitled(&mut self) -> CommandResult {
        // 今の文書は置き換えず、無題のバッファを足して切り替える (03 §9.8)。何も捨てないので確認は要らない
        let doc = self.blank_document();
        let (id, _) = self.add_document(doc);
        self.activate_document(id);
        CommandResult::Done
    }

    /// Window の表示を `view` に切り替え、`old` の文書を閉じる。Window がなければ作る。
    pub(super) fn show_view(&mut self, view: ViewId, old: Option<DocumentId>) {
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

    fn take_pending_for(&mut self, doc: DocumentId) {
        let ids: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, p)| match p {
                Pending::Open { doc: d, .. } | Pending::CheckUnchanged { doc: d } => *d == doc,
                Pending::ReplaceUnmappable { ask, .. } => ask.doc == doc,
                Pending::Search => self.search.as_ref().is_some_and(|s| s.doc() == doc),
                Pending::FindNext { doc: d, .. } => *d == doc,
                Pending::ReplaceAll { doc: d, .. } => *d == doc,
                Pending::ConvertEol(c) => c.doc == doc,
                // 保存ジョブは取り消さず、完了を待つ (03 §9.2 の「閉じる手順」。ユーザーが求めた保存を中断しない)
                Pending::Save { .. }
                | Pending::ConfigReload
                | Pending::RecoveryScan
                | Pending::WriteRecovery { .. }
                | Pending::Swap { .. }
                | Pending::SwapDiscard { .. }
                | Pending::SwapRead { .. }
                | Pending::SwapRemove { .. }
                | Pending::Leftover(_) => false,
            })
            .map(|(id, _)| *id)
            .collect();
        if self.search.as_ref().is_some_and(|s| s.doc() == doc) {
            self.search = None;
        }
        for id in ids {
            self.continuations.remove(&id);
            self.pending.remove(&id);
            self.jobs.cancel(id);
        }
    }

    /// ファイルを開く (03 §9.2)。Loading の文書を作ってすぐ読込ジョブを投入する。
    /// すでに文書があるときは、読込が終わるまで今の文書を表示し続け、完了で置き換える。
    pub fn open_path(&mut self, raw_path: &str, opts: OpenOptions) -> CommandResult {
        let opts = OpenOptions {
            default_eol: opts.default_eol.or(Some(self.config_eol())),
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

    /// 確認で「保存」を選んだ後: 保存し、成功したら `cont` を続ける。パスがなければ名前を尋ねてから保存する。
    pub(super) fn save_then(&mut self, cont: Continuation) -> CommandResult {
        let Some(doc_id) = self.active_document() else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        self.save_doc_then(doc_id, cont)
    }

    /// `save_then` を、アクティブ文書ではなく `doc_id` に対して行う (保存の連鎖は保存した文書に属する、03 §9.8)。
    pub(super) fn save_doc_then(
        &mut self,
        doc_id: DocumentId,
        cont: Continuation,
    ) -> CommandResult {
        let Some(doc) = self.document(doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        // 保存中なら、その保存の完了後に続ける (二重に保存しない)
        let in_flight = self.pending.iter().find_map(|(job, p)| match p {
            Pending::Save { doc, .. } if *doc == doc_id => Some(*job),
            _ => None,
        });
        if let Some(job) = in_flight {
            self.continuations.insert(job, cont);
            return CommandResult::Pending(job);
        }
        let Some(path) = doc.path.clone() else {
            self.open_prompt(PromptKind::SaveAs { then: Some(cont) }, String::new());
            return CommandResult::Done;
        };
        let expected = doc.disk_stamp.map_or(Expect::Absent, Expect::Stamp);
        self.start_save(doc_id, path, expected, Some(cont))
    }

    /// 名前を付けて保存 (パスを指定する)。
    pub(super) fn request_save_as(
        &mut self,
        raw_path: &str,
        cont: Option<Continuation>,
    ) -> CommandResult {
        self.request_save_as_fmt(raw_path, cont, None)
    }

    /// `request_save_as` に新しい書式を渡す版 (`file.set_encoding` の無題・読み取り専用の文書、03 §8.8)。
    pub(super) fn request_save_as_fmt(
        &mut self,
        raw_path: &str,
        cont: Option<Continuation>,
        fmt: Option<NewFormat>,
    ) -> CommandResult {
        let Some(doc_id) = self.active_document() else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Ok(path) = self.paths.normalize(raw_path) else {
            self.show_message(
                MessageLevel::Error,
                "msg.save.invalid_path",
                &[("path", raw_path), ("reason", "invalid path")],
            );
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
            self.message(MessageLevel::Info, "info.save.in_progress", &path.display());
            return CommandResult::Done;
        }
        let same = doc.path.as_ref() == Some(&path);
        // 読み取り専用の文書を同じファイルへ書くと、元のバイト列を損なうことがある (03 §9.7 (a) の 1)
        if same && !doc.is_editable() {
            self.message(
                MessageLevel::Warn,
                "msg.save_as.same_file_read_only",
                &path.display(),
            );
            return CommandResult::Rejected(RejectReason::ReadOnly);
        }
        // 同じファイルなら読んだときの Stamp で照合する。別のファイルへは存在しないことを期待する
        // (既存なら Conflict。上書きの確認は確認ダイアログ (後続) の仕事)。パスと Stamp は成功してから更新する
        let expected = if same {
            doc.disk_stamp.map_or(Expect::Absent, Expect::Stamp)
        } else {
            Expect::Absent
        };
        self.start_save_fmt(doc_id, path, expected, cont, fmt)
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

    pub(super) fn start_save(
        &mut self,
        doc_id: DocumentId,
        path: FsPath,
        expected: Expect,
        cont: Option<Continuation>,
    ) -> CommandResult {
        self.start_save_fmt(doc_id, path, expected, cont, None)
    }

    /// 新しい書式 `fmt` を保存に渡す (`file.set_encoding`)。成功したときだけ文書の書式に確定する。
    fn start_save_fmt(
        &mut self,
        doc_id: DocumentId,
        path: FsPath,
        expected: Expect,
        cont: Option<Continuation>,
        fmt: Option<NewFormat>,
    ) -> CommandResult {
        self.start_save_with(doc_id, path, expected, cont, SaveStrategy::default(), fmt)
    }

    /// 保存方式を指定して保存ジョブを投入する (確認の後の再投入で直接上書きにする)。
    fn start_save_with(
        &mut self,
        doc_id: DocumentId,
        path: FsPath,
        expected: Expect,
        cont: Option<Continuation>,
        strategy: SaveStrategy,
        fmt: Option<NewFormat>,
    ) -> CommandResult {
        let Some(doc) = self.documents.get_mut(&doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let out_encoding = fmt.map_or(doc.format.encoding, |f| f.encoding);
        // 区分 B / D: 元の文字コードのまま、別のバイト表現で保存される箇所がある文書を編集して保存するときは、
        // 書き込む前に確認する (FR-112 AC-4 (a)・NFR-303、03 §8.6 E2')。無編集の上書き保存は書き込まないので出ない。
        // 承認した Revision では繰り返さない
        if out_encoding == doc.format.encoding
            && doc.format.roundtrip.lossy_lines > 0
            && doc.is_dirty()
            && self.lossy_ok != Some((doc_id, doc.revision()))
        {
            let ask = LossyAsk {
                doc: doc_id,
                path,
                expected,
                cont,
                strategy,
                fmt,
            };
            self.open_lossy(ask);
            return CommandResult::Done;
        }
        // 状態付き符号化 (ISO-2022-JP) で不正バイトを含む文書は保存しない (03 §8.3 (a)。不正バイトを失わないため)
        if out_encoding.is_stateful() && doc.format.invalid_bytes > 0 {
            self.show_message(
                MessageLevel::Error,
                "msg.save.encoding_unsupported_iso2022",
                &[("path", &path.display())],
            );
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        let (encoding, bom, pua) = (
            out_encoding,
            fmt.map_or(doc.format.has_bom, |f| f.bom),
            doc.format.pua_escape,
        );
        let revision = doc.revision();
        let (snapshot, at) = doc.begin_save();
        let job = self.jobs.spawn(
            Some(doc_id),
            revision,
            JobKind::Save {
                path: path.clone(),
                snapshot,
                encoding,
                bom,
                pua,
                expected,
                strategy,
            },
        );
        if let Some(c) = cont {
            self.continuations.insert(job, c);
        }
        self.pending.insert(
            job,
            Pending::Save {
                doc: doc_id,
                at,
                revision,
                path,
                expected,
                fmt,
            },
        );
        CommandResult::Pending(job)
    }

    /// ジョブの結果を受け取る (02 §5.4)。
    pub fn on_job_result(&mut self, r: JobResult) {
        let job = r.job();
        // 読込の進捗は終わりではない。待ちの記録は残して、読込中の文書の進捗だけ更新する
        if let JobResult::Progress { bytes, total, .. } = r {
            if let Some(Pending::Open { doc, .. }) = self.pending.get(&job) {
                let pct = bytes
                    .saturating_mul(100)
                    .checked_div(total)
                    .unwrap_or(0)
                    .min(100) as u8;
                let doc = *doc;
                if let Some(d) = self.documents.get_mut(&doc) {
                    d.load_percent = pct;
                }
                self.mark_dirty_render();
            }
            return;
        }
        // 検索は途中経過が複数届く。待ちの記録は終わりまで残す
        if matches!(self.pending.get(&job), Some(Pending::Search)) {
            self.on_search_result(r);
            return;
        }
        let cont = self.continuations.remove(&job);
        let Some(pending) = self.pending.remove(&job) else {
            return;
        };
        match pending {
            Pending::Open {
                doc,
                view,
                path,
                replace,
                opts,
                kind,
            } => self.finish_open(doc, view, path, replace, opts, kind, r),
            Pending::CheckUnchanged { doc } => self.finish_check(doc, r),
            Pending::Search => {}
            Pending::FindNext {
                doc,
                revision,
                cursor,
                forward,
            } => self.finish_find_next(doc, revision, cursor, forward, r),
            Pending::ConvertEol(c) => self.finish_eol_conversion(c, r),
            Pending::ReplaceAll { doc, revision } => self.finish_replace_all(doc, revision, r),
            Pending::WriteRecovery { path, release } => {
                self.finish_write_recovery(&path, r);
                if let Some(doc) = release {
                    self.closing_snapshots.remove(&doc);
                    self.mark_snapshot_update(doc);
                }
            }
            Pending::ReplaceUnmappable {
                ask,
                revision,
                retried,
            } => self.finish_replace_unmappable(ask, revision, retried, r),
            Pending::RecoveryScan => self.finish_recovery_scan(r),
            Pending::ConfigReload => self.finish_config_reload(r),
            Pending::Swap { doc } => self.finish_swap(doc, r),
            Pending::SwapDiscard { doc } => self.finish_swap_discard(doc, r),
            Pending::SwapRead {
                doc,
                revision,
                entry,
            } => self.finish_swap_read(doc, revision, *entry, r),
            Pending::SwapRemove { entry, notify } => self.finish_swap_remove(&entry, notify, r),
            Pending::Leftover(ask) => self.finish_leftover(*ask, r),
            Pending::Save {
                doc,
                at,
                revision,
                path,
                expected,
                fmt,
            } => {
                let saved = matches!(r, JobResult::Saved { .. });
                // 「保存を中止して終了」の最中: 成功は採用し、失敗は終了時の stderr に残す。確認も後続も出さない
                if self.quit_abort.is_some() {
                    if !self.quit_abort_save_result(&path, &r) {
                        self.finish_save(doc, at, revision, &path, expected, fmt, None, r);
                    }
                    self.release_closing_snapshot(doc, &path, saved);
                    self.quit_abort_check();
                    self.mark_dirty_render();
                    return;
                }
                // 全保存の 1 件の失敗は、確認を出さず記録するだけ (03 §9.8)
                let in_batch = self.save_all_failed(job, doc, &path, &r);
                if in_batch != Some(true) {
                    self.finish_save(doc, at, revision, &path, expected, fmt, cont, r);
                }
                self.release_closing_snapshot(doc, &path, saved);
                if in_batch.is_some() {
                    self.finish_save_all_if_done();
                }
            }
        }
        self.mark_dirty_render();
    }

    /// 閉じた文書の保存の結果が届いた: 公開物を外す。保存が成功していなければ、内容を失わないよう退避先に書く
    /// (閉じた文書なので確認は出さない。書いた場所は `finish_write_recovery` のメッセージで示す)。
    fn release_closing_snapshot(&mut self, doc: DocumentId, path: &FsPath, saved: bool) {
        if self.documents.contains_key(&doc) {
            return;
        }
        if saved {
            self.closing_snapshots.remove(&doc);
            self.mark_snapshot_update(doc);
            return;
        }
        // 失敗: 退避先への書込が終わるまで、公開物は緊急退避に残す
        let Some(snapshot) = self.closing_snapshots.get(&doc).cloned() else {
            return;
        };
        let job = self.jobs.spawn(
            Some(doc),
            snapshot.revision,
            JobKind::WriteRecovery {
                doc: std::sync::Arc::new(snapshot),
            },
        );
        self.pending.insert(
            job,
            Pending::WriteRecovery {
                path: path.clone(),
                release: Some(doc),
            },
        );
    }

    /// 表現不能の文字の確認を出す。選択肢: UTF-8 で保存 (エスケープ文字を含まないとき) / キャンセル (既定、`Esc`)。
    /// 「置換して保存」は後続。
    fn open_unmappable(
        &mut self,
        ask: UnmappableAsk,
        total: u64,
        encoding: Encoding,
        has_escape: bool,
    ) {
        use super::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
        let (line, col) = self
            .char_pos(ask.doc, ask.first_char)
            .map_or((1, 1), |(_, l, c)| (l, c));
        let mut choices = Vec::new();
        // UTF-8 でも表せないのはエスケープ文字だけ (E4)。UTF-8 / UTF-16 で保存中のときは、選んでも結果が同じなので出さない
        if !has_escape && !matches!(encoding, Encoding::Utf8) {
            choices.push(ChoiceDef {
                id: "utf8",
                accel: Some('u'),
                destructive: false,
            });
        }
        // 置換して保存: 文書が編集できるときだけ (表現不能の文字にエスケープ文字を含まないとき)
        if !has_escape
            && self
                .documents
                .get(&ask.doc)
                .is_some_and(|d| d.is_editable())
        {
            choices.push(ChoiceDef {
                id: "replace",
                accel: Some('r'),
                destructive: false,
            });
        }
        let default = choices.len();
        self.open_confirm(ConfirmState {
            name: "save.unmappable",
            args: vec![
                ("count".to_owned(), total.to_string()),
                ("encoding".to_owned(), encoding.name().to_owned()),
                ("line".to_owned(), line.to_string()),
                ("col".to_owned(), col.to_string()),
            ],
            choices,
            details: Vec::new(),
            default,
            focused: default,
            action: ConfirmAction::Unmappable(ask),
        });
    }

    /// 通し番号 `n` (Unicode スカラー値) の文字の (バイト位置, 行, 桁)。行・桁は 1 始まり (桁は文字数)。
    fn char_pos(&self, doc: DocumentId, n: u64) -> Option<(usize, usize, usize)> {
        use eve_text::TextRead;
        let d = self.documents.get(&doc)?;
        let t = d.text();
        let body = t.slice_to_string(eve_text::ByteRange::all(t.len_bytes()));
        let (off, _) = body.char_indices().nth(usize::try_from(n).ok()?)?;
        let line = t.byte_to_line(eve_text::ByteOff(off)).0;
        let line_start = t.line_to_byte(eve_text::LineIdx(line)).0;
        let col = body[line_start..off].chars().count();
        Some((off, line + 1, col + 1))
    }

    /// 表現不能の確認の選択 (キャンセルは `unmappable_cancelled`)。
    pub(super) fn choose_unmappable(&mut self, ask: UnmappableAsk, id: &str) -> CommandResult {
        // 保存へ進む選択では強調は要らない (キャンセルのときだけ次の編集まで残す)
        self.unmappable_marks = None;
        if id == "replace" {
            return self.spawn_replace(ask, false);
        }
        let UnmappableAsk {
            doc,
            path,
            expected,
            cont,
            ..
        } = ask;
        match id {
            // 同じ保存先へ UTF-8 (BOM なし) の書式で保存し直す。書式は保存が成功したときだけ確定する
            "utf8" => self.start_save_fmt(
                doc,
                path,
                expected,
                cont,
                Some(NewFormat {
                    encoding: Encoding::Utf8,
                    bom: false,
                }),
            ),
            _ => CommandResult::Done,
        }
    }

    /// 表現不能の文字を置換する ChangeSet を作るジョブを投入する (03 §8.6 の選択肢 2)。
    fn spawn_replace(&mut self, ask: UnmappableAsk, retried: bool) -> CommandResult {
        let Some(doc) = self.documents.get(&ask.doc) else {
            return CommandResult::Done;
        };
        let revision = doc.revision();
        let job = self.jobs.spawn(
            Some(ask.doc),
            revision,
            JobKind::ReplaceUnmappable {
                snapshot: doc.snapshot(),
                encoding: ask.encoding,
                pua: doc.format.pua_escape,
            },
        );
        self.pending.insert(
            job,
            Pending::ReplaceUnmappable {
                ask,
                revision,
                retried,
            },
        );
        CommandResult::Pending(job)
    }

    /// 置換の ChangeSet ができた: 対象の一覧を示して確認を取る。作った Revision と今の Revision が違えば 1 回だけ作り直す。
    fn finish_replace_unmappable(
        &mut self,
        ask: UnmappableAsk,
        revision: eve_view::Revision,
        retried: bool,
        r: JobResult,
    ) {
        use super::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
        match r {
            JobResult::UnmappableReplacement {
                changes,
                targets,
                counts,
                ..
            } => {
                let Some(doc) = self.documents.get(&ask.doc) else {
                    return;
                };
                if doc.revision() != revision {
                    if retried {
                        self.show_message(
                            MessageLevel::Error,
                            "msg.save.failed",
                            &[("path", &ask.path.display()), ("reason", "stale")],
                        );
                    } else {
                        self.spawn_replace(ask, true);
                    }
                    return;
                }
                let mut details: Vec<String> = Vec::with_capacity(targets.len() + 1);
                for t in &targets {
                    let (line, col) = self.pos_of_offset(ask.doc, t.offset);
                    let shown = if t.ch.is_control() { '\u{FFFD}' } else { t.ch };
                    details.push(self.catalog().render(
                        "ui.confirm.save.unmappable_replace.item",
                        &[
                            ("line", &line.to_string()),
                            ("col", &col.to_string()),
                            ("char", &shown.to_string()),
                            ("codepoint", &format!("U+{:04X}", t.ch as u32)),
                            ("replacement", &t.replacement.to_string()),
                        ],
                        &|_| None,
                    ));
                }
                let rest = counts.total().saturating_sub(targets.len() as u64);
                if rest > 0 {
                    details.push(self.catalog().render(
                        "ui.confirm.save.unmappable_replace.more",
                        &[("count", &rest.to_string())],
                        &|_| None,
                    ));
                }
                self.open_confirm(ConfirmState {
                    name: "save.unmappable_replace",
                    args: vec![
                        ("total".to_owned(), counts.total().to_string()),
                        ("encoding".to_owned(), ask.encoding.name().to_owned()),
                        ("kana".to_owned(), counts.kana.to_string()),
                        ("compat".to_owned(), counts.compat.to_string()),
                        ("question".to_owned(), counts.question.to_string()),
                    ],
                    choices: vec![ChoiceDef {
                        id: "replace",
                        accel: Some('r'),
                        destructive: false,
                    }],
                    details,
                    // 既定はキャンセル
                    default: 1,
                    focused: 1,
                    action: ConfirmAction::UnmappableReplace {
                        ask,
                        changes,
                        revision,
                    },
                });
            }
            JobResult::Failed { err, .. } => {
                let (level, key) = match err {
                    JobError::Cancelled => (MessageLevel::Info, "info.save.aborted"),
                    _ => (MessageLevel::Error, "msg.save.failed"),
                };
                self.show_message(
                    level,
                    key,
                    &[("path", &ask.path.display()), ("reason", "replace")],
                );
            }
            _ => {}
        }
    }

    /// バイト位置 `off` の (行, 桁)。どちらも 1 始まり (桁は文字数)。
    fn pos_of_offset(&self, doc: DocumentId, off: usize) -> (usize, usize) {
        use eve_text::TextRead;
        let Some(d) = self.documents.get(&doc) else {
            return (1, 1);
        };
        let t = d.text();
        let line = t.byte_to_line(eve_text::ByteOff(off)).0;
        let line_start = t.line_to_byte(eve_text::LineIdx(line)).0;
        let body = t.slice_to_string(eve_text::ByteRange::new(
            eve_text::ByteOff(line_start),
            eve_text::ByteOff(off),
        ));
        (line + 1, body.chars().count() + 1)
    }

    /// 置換の確認で「置換して保存」: ChangeSet を 1 つの Transaction (`Origin::System`、Undo で戻せる) として適用し、
    /// 同じ文字コードで保存し直す (03 §8.6)。作った Revision が古ければ作り直す。
    pub(super) fn apply_replacement_and_save(
        &mut self,
        ask: UnmappableAsk,
        changes: eve_text::ChangeSet,
        revision: eve_view::Revision,
    ) -> CommandResult {
        use crate::document::{Origin, Transaction, UndoGroupHint};
        let Some(doc) = self.documents.get(&ask.doc) else {
            return CommandResult::Done;
        };
        if doc.revision() != revision {
            return self.spawn_replace(ask, true);
        }
        let view = self.first_window().and_then(|w| self.window_view(w));
        let sel = view.and_then(|v| self.views.get(&v).map(|v| v.selection.clone()));
        let mut tx = Transaction::new(changes, Origin::System, self.now)
            .with_undo_group(UndoGroupHint::Break)
            .with_selections(sel, None);
        if let Some(v) = view {
            tx = tx.with_view(v);
        }
        if self.apply_transaction(ask.doc, tx).is_err() {
            self.show_message(
                MessageLevel::Error,
                "msg.save.failed",
                &[("path", &ask.path.display()), ("reason", "replace")],
            );
            return CommandResult::Done;
        }
        self.start_save_fmt(ask.doc, ask.path, ask.expected, ask.cont, ask.fmt)
    }

    /// 表現不能の文字 (通し番号 `indices`) の位置を強調として記録する。文書の Revision が変わると無効になる。
    fn mark_unmappable(&mut self, doc: DocumentId, indices: &[u64]) {
        use eve_text::TextRead;
        let Some(d) = self.documents.get(&doc) else {
            return;
        };
        let t = d.text();
        let want: std::collections::BTreeSet<u64> = indices.iter().copied().collect();
        let last = want.last().copied().unwrap_or(0);
        let mut marks: Vec<(usize, usize)> = Vec::with_capacity(want.len());
        let mut n = 0u64;
        let mut pos = 0usize;
        'chunks: for chunk in t.chunks(eve_text::ByteRange::all(t.len_bytes())) {
            for (i, c) in chunk.char_indices() {
                if want.contains(&n) {
                    marks.push((pos + i, pos + i + c.len_utf8()));
                }
                n += 1;
                if n > last {
                    break 'chunks;
                }
            }
            pos += chunk.len();
        }
        self.unmappable_marks = Some((doc, d.revision(), marks));
        self.mark_dirty_render();
    }

    /// 強調する表現不能の文字 (文書が変わっていなければ)。
    pub(super) fn unmappable_marks_for(&self, doc: DocumentId) -> Vec<(usize, usize)> {
        match &self.unmappable_marks {
            Some((d, rev, marks))
                if *d == doc && self.documents.get(d).is_some_and(|x| x.revision() == *rev) =>
            {
                marks.clone()
            }
            _ => Vec::new(),
        }
    }

    /// 表現不能の確認のキャンセル: 保存しない (後続も中止)。カーソルを最初の表現不能の位置へ移す。
    pub(super) fn unmappable_cancelled(&mut self, ask: &UnmappableAsk) {
        if let Some((off, _, _)) = self.char_pos(ask.doc, ask.first_char)
            && let Some(view) = self
                .first_window()
                .and_then(|w| self.window_view(w))
                .filter(|v| self.views.get(v).is_some_and(|v| v.doc == ask.doc))
        {
            self.set_selection(view, eve_text::Selection::point(eve_text::ByteOff(off)));
            self.ensure_cursor_visible(view);
        }
        self.show_message(MessageLevel::Info, "info.save.aborted", &[]);
    }

    /// 保存の失敗の確認に渡す材料 (文書が無い・読込中なら `None`)。
    fn save_failed_ask(
        &self,
        doc: DocumentId,
        path: &FsPath,
        expected: Expect,
        fmt: Option<NewFormat>,
        cont: Option<Continuation>,
    ) -> Option<SaveFailedAsk> {
        // 閉じた文書には確認を出さない (保存の結果の処理が退避先に書く)
        self.document(doc)?;
        let snapshot = self.rescue_snapshot(doc, false)?;
        Some(SaveFailedAsk {
            doc,
            path: path.clone(),
            expected,
            cont,
            snapshot: std::sync::Arc::new(snapshot),
            fmt,
        })
    }

    /// 保存の失敗を `ui.confirm.save_failed` で示す (03 §9.7 (e))。本文にメッセージ `key` の text を入れる。
    /// 材料が無ければ (文書が閉じている等) メッセージだけ示す。`retry` は Busy のときだけ (何も書いていないので照合は通る)。
    fn report_save_failure(
        &mut self,
        ask: Option<SaveFailedAsk>,
        key: &str,
        args: &[(&str, &str)],
        retry: bool,
    ) {
        use super::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
        let Some(ask) = ask else {
            self.show_message(MessageLevel::Error, key, args);
            return;
        };
        let detail = self.message_text(key, args);
        let choice = |id: &'static str, accel: char| ChoiceDef {
            id,
            accel: Some(accel),
            destructive: false,
        };
        let mut choices = Vec::new();
        if retry {
            choices.push(choice("retry", 't'));
        }
        choices.push(choice("alt", 'a'));
        choices.push(choice("recovery", 'r'));
        let default = usize::from(retry);
        self.open_confirm(ConfirmState {
            name: "save_failed",
            args: vec![("detail".to_owned(), detail)],
            choices,
            details: Vec::new(),
            default,
            focused: default,
            action: ConfirmAction::SaveFailed(ask),
        });
    }

    /// 保存の失敗の確認の選択 (キャンセルは呼び側が処理済み)。`id` は選択肢の ID。
    pub(super) fn choose_save_failed(&mut self, ask: SaveFailedAsk, id: &str) -> CommandResult {
        let SaveFailedAsk {
            doc,
            path,
            expected,
            cont,
            snapshot,
            fmt,
        } = ask;
        match id {
            "retry" => self.start_save_fmt(doc, path, expected, cont, fmt),
            "recovery" => {
                // 続きの操作 (終了・置き換え) は行わない。書いた場所は結果のメッセージで示す
                let job = self.jobs.spawn(
                    Some(doc),
                    snapshot.revision,
                    JobKind::WriteRecovery { doc: snapshot },
                );
                self.pending.insert(
                    job,
                    Pending::WriteRecovery {
                        path,
                        release: None,
                    },
                );
                CommandResult::Pending(job)
            }
            // 別名で保存: 名前の入力へ (保存の後に続ける操作は引き継ぐ)
            _ => {
                self.open_prompt(PromptKind::SaveAs { then: cont }, String::new());
                CommandResult::Done
            }
        }
    }

    /// 「退避先に保存」の結果。文書の状態 (dirty・保存点) は変えない。
    fn finish_write_recovery(&mut self, path: &FsPath, r: JobResult) {
        match r {
            JobResult::RecoveryWritten { path: target, .. } => self.show_message(
                MessageLevel::Info,
                "info.save.to_recovery",
                &[("target", &target.display()), ("path", &path.display())],
            ),
            JobResult::Failed { err, .. } => {
                let reason = reason_label(&err);
                self.show_message(
                    MessageLevel::Error,
                    "msg.save.recovery_failed",
                    &[("reason", &reason), ("path", &path.display())],
                );
            }
            _ => {}
        }
    }

    /// 保存の代替の確認を出す。
    fn open_fallback(&mut self, ask: FallbackAsk, reason: &str, links: &str) {
        use super::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
        let choice = |id: &'static str, accel: char| ChoiceDef {
            id,
            accel: Some(accel),
            destructive: false,
        };
        let (name, choices, default): (&'static str, Vec<ChoiceDef>, usize) = match ask.kind {
            FallbackKind::ConflictChanged => (
                "save.conflict_changed",
                vec![choice("overwrite", 'o'), choice("alt", 'a')],
                1,
            ),
            FallbackKind::ConflictDeleted => (
                "save.conflict_deleted",
                vec![choice("overwrite", 'o'), choice("alt", 'a')],
                1,
            ),
            FallbackKind::ConflictCreated => (
                "save.conflict_created",
                vec![choice("overwrite", 'o'), choice("alt", 'a')],
                1,
            ),
            FallbackKind::AtomicUnsupported => (
                "save.atomic_unsupported",
                vec![choice("direct", 'o'), choice("alt", 'a')],
                1,
            ),
            FallbackKind::Hardlink => (
                "save.hardlink",
                vec![choice("direct", 'k'), choice("break", 'b')],
                2,
            ),
            FallbackKind::NoBackup => (
                "save.overwrite_without_backup",
                vec![choice("overwrite", 'o'), choice("alt", 'a')],
                2,
            ),
        };
        self.open_confirm(ConfirmState {
            name,
            args: vec![
                ("path".to_owned(), ask.path.display()),
                ("reason".to_owned(), reason.to_owned()),
                ("links".to_owned(), links.to_owned()),
            ],
            choices,
            details: Vec::new(),
            default,
            focused: default,
            action: ConfirmAction::Fallback(ask),
        });
    }

    /// 区分 B (ISO-2022-JP は区分 D のエスケープの書き方) の確認を出す。選択肢: 保存 / 別名で保存。
    /// 区分 B は別名が既定、区分 D は文字が変わらないので保存が既定 (04 §15.2)。
    fn open_lossy(&mut self, ask: LossyAsk) {
        use super::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
        let Some(doc) = self.documents.get(&ask.doc) else {
            return;
        };
        let count = doc.format.roundtrip.lossy_lines;
        let example = doc
            .format
            .roundtrip
            .first_offsets
            .first()
            .map(u64::to_string)
            .unwrap_or_default();
        let (name, default) = if doc.format.encoding.is_stateful() {
            ("save.escape_normalized", 0)
        } else {
            ("save.lossy_roundtrip", 1)
        };
        let choice = |id: &'static str, accel: char| ChoiceDef {
            id,
            accel: Some(accel),
            destructive: false,
        };
        self.open_confirm(ConfirmState {
            name,
            args: vec![
                ("count".to_owned(), count.to_string()),
                ("example".to_owned(), example),
            ],
            choices: vec![choice("save", 's'), choice("alt", 'a')],
            details: Vec::new(),
            default,
            focused: default,
            action: ConfirmAction::Lossy(ask),
        });
    }

    /// 区分 B / D の確認の選択 (キャンセルは呼び側が処理済み)。保存は、この Revision では確認を繰り返さずに投入し直す。
    pub(super) fn choose_lossy(&mut self, ask: LossyAsk, choice: usize) -> CommandResult {
        let LossyAsk {
            doc,
            path,
            expected,
            cont,
            strategy,
            fmt,
        } = ask;
        if choice == 0 {
            if let Some(d) = self.documents.get(&doc) {
                self.lossy_ok = Some((doc, d.revision()));
            }
            return self.start_save_with(doc, path, expected, cont, strategy, fmt);
        }
        // 別名で保存: 名前の入力へ (保存の後に続ける操作は引き継ぐ)
        self.open_prompt(PromptKind::SaveAs { then: cont }, String::new());
        CommandResult::Done
    }

    /// 保存の代替の確認の選択 (キャンセルは呼び側が処理済み)。
    pub(super) fn choose_fallback(&mut self, ask: FallbackAsk, choice: usize) -> CommandResult {
        let FallbackAsk {
            kind,
            doc,
            path,
            expected,
            cont,
            fmt,
            conflict_current,
        } = ask;
        // 競合の「上書き」: 照合で見つけた今の状態を新しい期待値にして、通常の保存を投入し直す (03 §9.5)
        let expected = match kind {
            FallbackKind::ConflictChanged
            | FallbackKind::ConflictDeleted
            | FallbackKind::ConflictCreated => {
                conflict_current.map_or(Expect::Absent, Expect::Stamp)
            }
            _ => expected,
        };
        let in_place = |backup| SaveStrategy::InPlace { backup };
        let strategy = match (kind, choice) {
            (
                FallbackKind::ConflictChanged
                | FallbackKind::ConflictDeleted
                | FallbackKind::ConflictCreated,
                0,
            ) => Some(SaveStrategy::default()),
            (FallbackKind::AtomicUnsupported | FallbackKind::Hardlink, 0) => {
                Some(in_place(InPlaceBackup::Required))
            }
            (FallbackKind::Hardlink, 1) => Some(SaveStrategy::Atomic {
                hardlink: HardlinkPolicy::BreakLink,
            }),
            (FallbackKind::NoBackup, 0) => Some(in_place(InPlaceBackup::SkipConfirmed)),
            // 別名で保存: 名前の入力へ (保存の後に続ける操作は引き継ぐ)
            _ => None,
        };
        match strategy {
            Some(s) => self.start_save_with(doc, path, expected, cont, s, fmt),
            None => {
                self.open_prompt(PromptKind::SaveAs { then: cont }, String::new());
                CommandResult::Done
            }
        }
    }

    /// 起動時に退避ディレクトリを走査するジョブを投入する (`open_initial` が呼ぶ)。
    pub(super) fn start_recovery_scan(&mut self) {
        let job = self
            .jobs
            .spawn(None, eve_view::Revision::INITIAL, JobKind::RecoveryScan);
        self.pending.insert(job, Pending::RecoveryScan);
    }

    /// 退避が残っていれば場所を案内する (`msg.recovery.found` / `found_rescue`、02 §9.3。中身は読まない)。
    /// 案内するのは最も新しい退避 (名前の `<UTC timestamp>-<pid>[-<n>]` の順)。「退避先に保存」の退避だけなら文言を分ける。
    fn finish_recovery_scan(&mut self, r: JobResult) {
        let JobResult::RecoveryFound {
            mut dirs,
            swaps,
            leftovers,
            ..
        } = r
        else {
            return;
        };
        self.leftovers = leftovers;
        self.swap_scan_done(swaps);
        if dirs.is_empty() {
            return;
        }
        let only_rescue = dirs.iter().all(|d| d.rescue);
        // 異常終了の退避があれば、その中で最も新しいものを示す (無ければ「退避先に保存」の中で最も新しいもの)
        if !only_rescue {
            dirs.retain(|d| !d.rescue);
        }
        let key = |d: &crate::fs::RecoveryDirEntry| {
            d.path
                .as_path()
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        // 同じ時刻は `-<n>` の大きい方が新しい。名前の長さ → 辞書順で比べる (`-2` は `-` なしより後)
        dirs.sort_by(|a, b| {
            let (ka, kb) = (key(a), key(b));
            ka.get(..16)
                .cmp(&kb.get(..16))
                .then(ka.len().cmp(&kb.len()))
                .then(ka.cmp(&kb))
        });
        let count = dirs.len().to_string();
        let newest = dirs.last().map(|d| d.path.display()).unwrap_or_default();
        self.show_message(
            MessageLevel::Warn,
            if only_rescue {
                "msg.recovery.found_rescue"
            } else {
                "msg.recovery.found"
            },
            &[("dir", &newest), ("count", &count)],
        );
    }

    #[allow(clippy::too_many_arguments)] // 読込ジョブの文脈 (Pending::Open) をそのまま受ける
    fn finish_open(
        &mut self,
        doc_id: DocumentId,
        view: ViewId,
        path: FsPath,
        replace: Option<DocumentId>,
        opts: OpenOptions,
        kind: OpenKind,
        r: JobResult,
    ) {
        if kind != OpenKind::Open {
            self.finish_reopen(doc_id, view, path, opts, kind, r);
            return;
        }
        match r {
            JobResult::Complete {
                text,
                format,
                meta: _,
                stamp,
                changed_during_load,
                detected,
                notes,
                interrupted_save,
                ..
            } => {
                let lossy = format.roundtrip.lossy_lines;
                let encoding = format.encoding;
                let mut d = Document::from_text(text, format);
                d.path = Some(path.clone());
                d.disk_stamp = Some(stamp);
                d.read_only = detected;
                self.put_document(doc_id, d);
                if detected.contains(crate::document::ReadOnlyReason::LongLine) {
                    self.message(MessageLevel::Info, "info.open.long_line", &path.display());
                }
                if changed_during_load {
                    self.message(
                        MessageLevel::Warn,
                        "msg.open.changed_during_load",
                        &path.display(),
                    );
                }
                // 文字コードに関する通知 (最後に出したものが現在のメッセージ)
                if lossy > 0 {
                    self.show_message(
                        MessageLevel::Info,
                        "info.open.lossy_roundtrip",
                        &[("count", &lossy.to_string())],
                    );
                }
                if let Some(e) = notes.redetected {
                    self.show_message(
                        MessageLevel::Info,
                        "info.open.redetected",
                        &[("encoding", e.name())],
                    );
                }
                if let Some(guess) = notes.guess_unsupported {
                    self.show_message(
                        MessageLevel::Info,
                        "info.open.guess_unsupported",
                        &[
                            ("path", &path.display()),
                            ("guess", guess),
                            ("encoding", encoding.name()),
                        ],
                    );
                }
                // 中断された保存の痕跡は最も重要なので最後に出す (現在のメッセージにする)
                if let Some(is) = &interrupted_save {
                    self.show_interrupted(&path, is);
                }
                self.complete_open(doc_id, view, replace);
            }
            JobResult::NewFile {
                parent_missing,
                interrupted_save,
                ..
            } => {
                // 存在しないパス: 空の新規文書 (dirty ではない)。保存すると作成する
                let mut d = self.blank_document();
                d.path = Some(path.clone());
                self.put_document(doc_id, d);
                let key = if parent_missing.is_some() {
                    "msg.open.parent_missing"
                } else {
                    "info.open.new_file"
                };
                let dir = parent_missing
                    .as_ref()
                    .map(FsPath::display)
                    .unwrap_or_default();
                self.show_message(
                    if parent_missing.is_some() {
                        MessageLevel::Warn
                    } else {
                        MessageLevel::Info
                    },
                    key,
                    &[("path", &path.display()), ("dir", &dir)],
                );
                if let Some(is) = &interrupted_save {
                    self.show_interrupted(&path, is);
                }
                self.complete_open(doc_id, view, replace);
            }
            // 確認が要る: 読込中の文書は残したまま確認を出し、回答で先頭から読み直す (03 §9.2)
            JobResult::NeedsConfirm { ask, .. } => {
                self.open_open_confirm(OpenAsk {
                    doc: doc_id,
                    view,
                    path,
                    replace,
                    opts,
                    ask,
                    kind: OpenKind::Open,
                });
            }
            JobResult::Failed { err, .. } => {
                let reason = reason_label(&err);
                let (level, key) = match err {
                    JobError::Cancelled => (MessageLevel::Info, "info.open.cancelled"),
                    JobError::Fs(e) => (MessageLevel::Error, open_error_key(&e)),
                    JobError::OutOfMemory { .. } => (MessageLevel::Error, "msg.open.out_of_memory"),
                    _ => (MessageLevel::Error, "msg.open.failed"),
                };
                self.open_failed(doc_id, replace, level, key, &path, &reason);
            }
            _ => {}
        }
    }

    /// 開くときの確認 (`ui.confirm.open.*`) を出す。
    fn open_open_confirm(&mut self, ask: OpenAsk) {
        use super::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
        let choice = |id: &'static str, accel: char| ChoiceDef {
            id,
            accel: Some(accel),
            destructive: false,
        };
        let (name, choices, args): (&'static str, Vec<ChoiceDef>, Vec<(String, String)>) =
            match ask.ask {
                ConfirmAsk::Binary => (
                    "open.binary",
                    vec![choice("readonly", 'r'), choice("encoding", 'e')],
                    vec![],
                ),
                ConfirmAsk::UnsupportedBom { .. } => (
                    "open.utf32",
                    vec![choice("readonly", 'r'), choice("encoding", 'e')],
                    vec![],
                ),
                ConfirmAsk::HugeFile { len } => (
                    "open.huge",
                    vec![choice("readonly", 'r')],
                    vec![("size".to_owned(), human_size(len))],
                ),
                ConfirmAsk::ManyInvalidBytes { encoding, permille } => (
                    "open.invalid_bytes",
                    vec![
                        choice("readonly", 'r'),
                        choice("encoding", 'e'),
                        choice("abort", 'c'),
                    ],
                    vec![
                        ("encoding".to_owned(), encoding.name().to_owned()),
                        (
                            "ratio".to_owned(),
                            format!("{}.{}%", permille / 10, permille % 10),
                        ),
                        (
                            "factor".to_owned(),
                            format!("{:.1}", encoding.memory_factor()),
                        ),
                    ],
                ),
            };
        let mut args = args;
        args.insert(0, ("path".to_owned(), ask.path.display()));
        self.open_confirm(ConfirmState {
            name,
            args,
            choices,
            details: Vec::new(),
            default: 0,
            focused: 0,
            action: ConfirmAction::Open(ask),
        });
    }

    /// 開くときの確認の選択 (キャンセルは呼び側が `open_aborted` を呼ぶ)。`id` は選択肢の ID。
    pub(super) fn choose_open(&mut self, ask: OpenAsk, id: &str) -> CommandResult {
        match id {
            "readonly" => {
                let mut opts = ask.opts;
                opts.read_only_requested = true;
                match ask.ask {
                    ConfirmAsk::HugeFile { .. } => {
                        opts.confirmed_large = true;
                        opts.pending
                            .insert(crate::document::ReadOnlyReason::LargeFileViewer);
                    }
                    ConfirmAsk::Binary | ConfirmAsk::UnsupportedBom { .. } => {
                        opts.pending.insert(crate::document::ReadOnlyReason::Binary);
                    }
                    _ => {}
                }
                self.respawn_open(ask, opts)
            }
            "encoding" => {
                self.open_prompt(PromptKind::Encoding { ask }, String::new());
                CommandResult::Done
            }
            _ => {
                self.open_aborted(&ask);
                CommandResult::Done
            }
        }
    }

    /// 開くのを中止した (確認の `Esc` / 中止、文字コードの入力の `Esc`)。読込中の文書を捨てる。
    pub(super) fn open_aborted(&mut self, ask: &OpenAsk) {
        if ask.kind != OpenKind::Open {
            // 開き直し・再読込の中止: 旧本文・書式・履歴をそのまま保つ (03 §8.8 / §9.5)
            self.reopen_restore(ask.doc);
            self.message(
                MessageLevel::Info,
                "info.open.cancelled",
                &ask.path.display(),
            );
            return;
        }
        self.open_failed(
            ask.doc,
            ask.replace,
            MessageLevel::Info,
            "info.open.cancelled",
            &ask.path,
            "",
        );
    }

    /// 確認の回答の後、同じ読込中の文書へ `opts` で先頭から読み直すジョブを投入する。
    pub(super) fn respawn_open(&mut self, ask: OpenAsk, opts: OpenOptions) -> CommandResult {
        let job = self.jobs.spawn(
            Some(ask.doc),
            eve_view::Revision::INITIAL,
            JobKind::Open {
                path: ask.path.clone(),
                opts,
            },
        );
        self.pending.insert(
            job,
            Pending::Open {
                doc: ask.doc,
                view: ask.view,
                path: ask.path,
                replace: ask.replace,
                opts,
                kind: ask.kind,
            },
        );
        CommandResult::Pending(job)
    }

    // ---------- 文字コードを指定して開き直す (03 §8.8) ----------

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
        if self.confirm_if_dirty("unsaved_reopen", Continuation::Reopen(enc)) {
            return CommandResult::Done;
        }
        self.start_reopen(enc)
    }

    /// 確認を済ませた (または dirty でない) 開き直しの開始。同じ `DocumentId` で読込ジョブを投入し、読込中は旧本文を表示し続ける。
    pub(super) fn start_reopen(&mut self, enc: Encoding) -> CommandResult {
        // 利用者の指定と書込権限の理由は開き直しでも保つ (03 §8.8 の「読み取り専用の理由の差し替え」)
        self.start_in_place(
            OpenKind::Reopen,
            &[
                crate::document::ReadOnlyReason::UserRequested,
                crate::document::ReadOnlyReason::NoWritePermission,
            ],
            Some(enc),
        )
    }

    /// 確認を済ませた (または dirty でない) 再読込の開始 (03 §9.5)。文字コードは今のものを明示指定して判定し直さない。
    pub(super) fn start_reload(&mut self) -> CommandResult {
        // 再読込は `Binary` と `LargeFileViewer` も保つ (明示指定の読込ではバイナリの判定をしない)
        self.start_in_place(
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

    /// 同じ文書・View の中身を差し替える読込ジョブの投入。`keep` は今の文書から引き継ぐ読み取り専用の理由。
    /// `enc` が無ければ今の文書の文字コードを明示指定する。
    fn start_in_place(
        &mut self,
        kind: OpenKind,
        keep: &[crate::document::ReadOnlyReason],
        enc: Option<Encoding>,
    ) -> CommandResult {
        let Some(doc_id) = self.active_document() else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(w) = self.first_window() else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(view) = self.window_view(w) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(doc) = self.documents.get_mut(&doc_id) else {
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
        self.pending.insert(
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
            use super::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
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
                action: ConfirmAction::Reload,
            });
            return CommandResult::Done;
        }
        self.start_reload()
    }

    /// 再読込の適用 (03 §9.5 手順 5): 新しい内容を全置換の `Origin::Reload` の Transaction として適用する。
    /// 書式と読み取り専用の理由は本文と一緒に差し替え、前後の値を Undo ノードに持たせる (Undo で一緒に戻る)。
    #[allow(clippy::too_many_arguments)] // 読込の結果 (JobResult::Complete) をそのまま受ける
    fn apply_reload(
        &mut self,
        doc_id: DocumentId,
        view: ViewId,
        path: &FsPath,
        text: eve_text::Text,
        mut format: crate::document::TextFormat,
        stamp: crate::fs::FileStamp,
        detected: crate::document::ReadOnlyReasons,
    ) {
        use crate::document::{FormatChange, FormatState, Origin, Transaction};
        use eve_text::{ByteOff, ByteRange, ChangeSet, Edit, Selection, TextRead};
        let Some(old) = self.documents.get(&doc_id) else {
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
        let before_sel = self.views.get(&view).map(|v| v.selection.clone());
        let head = before_sel.as_ref().map_or(0, |s| s.primary().head.0);
        let after_sel = Selection::point(ByteOff(head.min(new_len))).normalized(&text);
        let tx = Transaction::new(cs, Origin::Reload, self.now)
            .with_selections(before_sel, Some(after_sel))
            .with_view(view)
            .with_format_change(change);
        let applied = self.apply_transaction(doc_id, tx);
        if let Some(d) = self.documents.get_mut(&doc_id) {
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
    fn reopen_restore(&mut self, doc: DocumentId) {
        if let Some(d) = self.documents.get_mut(&doc) {
            d.load_state = crate::document::LoadState::Ready;
        }
        self.mark_dirty_render();
    }

    fn finish_reopen(
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
                if let Some(v) = self.views.get_mut(&view) {
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

    /// 中断された保存の痕跡の案内 (04 §15.2 の `msg.open.interrupted_*`、02 §4.5 の写像)。
    fn show_interrupted(&mut self, path: &FsPath, is: &crate::fs::InterruptedSave) {
        use crate::fs::SaveMethod;
        let key = match (is.method, is.backup.is_some(), is.complete) {
            (SaveMethod::NewFile, _, _) => "msg.open.interrupted_new",
            (_, true, true) => "msg.open.interrupted_save",
            (_, true, false) => "msg.open.interrupted_save_incomplete",
            (_, false, _) => "msg.open.interrupted_save_no_backup",
        };
        let backup = is.backup.as_ref().map(FsPath::display).unwrap_or_default();
        let tmp = is.tmp.as_ref().map(FsPath::display).unwrap_or_default();
        self.show_message(
            MessageLevel::Warn,
            key,
            &[
                ("path", &path.display()),
                ("backup", &backup),
                ("tmp", &tmp),
            ],
        );
    }

    fn complete_open(&mut self, doc: DocumentId, view: ViewId, replace: Option<DocumentId>) {
        // 読込の間に別の文書へ切り替えていなければ、新しいバッファへ切り替える (03 §9.8)
        if let Some(from) = replace
            && self.active_document() == Some(from)
        {
            // 何も入力していない無題のバッファは、開いたファイルで置き換える (何も失わない。起動直後の Ctrl+O で空のバッファが残らない)
            let pristine = self.documents.get(&from).is_some_and(|d| {
                d.path.is_none()
                    && !d.is_dirty()
                    && d.len_bytes() == 0
                    && d.load_state == crate::document::LoadState::Ready
            });
            if pristine {
                self.show_view(view, Some(from));
            } else {
                self.activate_document(doc);
            }
        }
        // 起動時の `+行:桁` / `path:行:桁`: 読み終わった文書の位置へ移る (範囲外は丸める)
        if let Some((line, col)) = self.open_jumps.remove(&doc) {
            self.jump_to_line_col(view, doc, line, col);
        }
        self.mark_snapshot_update(doc);
    }

    fn open_failed(
        &mut self,
        doc: DocumentId,
        replace: Option<DocumentId>,
        level: MessageLevel,
        key: &str,
        path: &FsPath,
        reason: &str,
    ) {
        self.show_message(level, key, &[("path", &path.display()), ("reason", reason)]);
        if replace.is_some() {
            // 足したバッファだけを閉じる。今の文書はそのまま残す (03 §9.8)
            self.close_document(doc);
        } else if self.active_document() == Some(doc) || self.first_window().is_none() {
            // 表示するはずの文書の読込に失敗: 失敗した文書は閉じ、ほかの文書があればそれを、無ければ
            // 無題の文書で始める (再試行できる、03 §9.2 の「失敗」)
            self.open_jumps.remove(&doc);
            let others = self.documents.keys().copied().find(|d| *d != doc);
            match others {
                Some(next) => {
                    self.activate_document(next);
                    self.close_document(doc);
                }
                None => {
                    let blank = self.blank_document();
                    let (_, untitled) = self.add_document(blank);
                    self.show_view(untitled, Some(doc));
                }
            }
        } else {
            // 起動時に追加で開いた文書の読込の失敗: 表示中の文書はそのままに、閉じるだけ
            self.close_document(doc);
        }
    }

    fn finish_check(&mut self, doc_id: DocumentId, r: JobResult) {
        let Some(doc) = self.documents.get(&doc_id) else {
            return;
        };
        let shown = doc.path.as_ref().map(FsPath::display).unwrap_or_default();
        let unchanged = match &r {
            // 読んだときの Stamp と一致し、編集もなければ書き込まない
            JobResult::Meta { stamp, .. } => {
                !doc.is_dirty()
                    && doc.disk_stamp.is_some_and(|d| {
                        // 内容ハッシュが双方にあれば、それで決める (mtime だけが違っても中身が同じなら変更なし、
                        // mtime が同じでも中身が違えば変更あり。05 §5.5)
                        let same_hash = d.hash.zip(stamp.hash).map(|(a, b)| a == b);
                        d.len == stamp.len
                            && d.file_id == stamp.file_id
                            && match same_hash {
                                Some(same) => same,
                                None => d.mtime == stamp.mtime,
                            }
                    })
            }
            _ => false,
        };
        match r {
            _ if unchanged => self.message(MessageLevel::Info, "info.save.unchanged", &shown),
            // NotFound は不一致として保存へ進む。その他の失敗は照合の失敗として示す
            JobResult::Failed {
                err: JobError::Fs(e),
                ..
            } if e.kind != FsErrorKind::NotFound => {
                self.show_message(
                    MessageLevel::Error,
                    "msg.save.check_failed",
                    &[("path", &shown), ("reason", &fs_reason(&e))],
                );
            }
            JobResult::Failed {
                err: JobError::Cancelled,
                ..
            } => self.message(MessageLevel::Info, "info.save.aborted", &shown),
            _ => {
                let Some((path, expected)) = self.documents.get(&doc_id).and_then(|d| {
                    Some((
                        d.path.clone()?,
                        d.disk_stamp.map_or(Expect::Absent, Expect::Stamp),
                    ))
                }) else {
                    return;
                };
                self.start_save(doc_id, path, expected, None);
            }
        }
    }

    #[allow(clippy::too_many_arguments)] // 保存ジョブの文脈 (Pending::Save) をそのまま受ける
    fn finish_save(
        &mut self,
        doc_id: DocumentId,
        at: NodeId,
        revision: eve_view::Revision,
        path: &FsPath,
        expected: Expect,
        fmt: Option<NewFormat>,
        cont: Option<Continuation>,
        r: JobResult,
    ) {
        match r {
            JobResult::Saved { report, .. } => {
                let report_method = report.method;
                if let Some(doc) = self.documents.get_mut(&doc_id) {
                    // `file.set_encoding`: 保存が成功したので新しい書式に確定する。往復の検査結果は旧文字コードのものなので捨てる
                    // (書いた内容を再エンコードすれば同じバイト列になる、03 §8.8)
                    if let Some(f) = fmt {
                        doc.format.encoding = f.encoding;
                        doc.format.has_bom = f.bom;
                        doc.format.roundtrip = Default::default();
                    }
                    // 名前を付けて保存 (別のパスへ書いた) では、元のファイルに由来する読み取り専用の理由
                    // (属性・書込権限なし) を外す。書き込めた保存先の文書は編集できる (FR-116 AC-4、03 §9.6)。
                    // `--readonly` と内容に由来する理由 (不正バイト・バイナリ・長大行) は残る
                    if doc.path.as_ref() != Some(path) {
                        doc.read_only
                            .remove(crate::document::ReadOnlyReason::FileAttribute);
                        doc.read_only
                            .remove(crate::document::ReadOnlyReason::NoWritePermission);
                    }
                    // 名前を付けて保存のパスは、書けてから文書のパスにする (03 §9.3 手順 7)
                    doc.path = Some(path.clone());
                    doc.finish_save(at, Some(report.new_stamp));
                }
                self.push_event(EditorEvent::DocumentSaved {
                    doc: doc_id,
                    revision,
                    path: path.display(),
                });
                let key = if report_method == SaveMethod::InPlace {
                    "info.save.in_place"
                } else {
                    "info.save.done"
                };
                self.message(MessageLevel::Info, key, &path.display());
                self.mark_snapshot_update(doc_id);
                // 保存の後に続ける操作 (開く・新規作成・終了)。保存が失敗したときは続けない
                // 保存の間に編集されていたら、続きの操作の前にもう一度尋ねる (03 §9.7。編集を捨てない)
                if let Some(c) = cont {
                    if c == Continuation::Quit {
                        // 終了の連鎖: 保存した文書が clean なら次の文書へ、保存中の編集で dirty なら同じ文書をもう一度尋ねる
                        self.advance_quit_chain();
                        return;
                    }
                    let name = match c {
                        Continuation::Reopen(_) => "unsaved_reopen",
                        Continuation::Close(_) => "unsaved_close",
                        _ => "unsaved",
                    };
                    if !self.confirm_doc_if_dirty(doc_id, name, c.clone()) {
                        self.run_continuation(c);
                    }
                }
            }
            // 保存先の文字コードで表せない文字がある: 何も書いていない。E3 の選択肢を出す (03 §8.6)
            JobResult::Unmappable {
                total,
                recorded,
                encoding,
                ..
            } if self.documents.contains_key(&doc_id) && !recorded.is_empty() => {
                // 該当箇所 (先頭 100 件) を強調する。次の編集まで残す (03 §8.6)
                let indices: Vec<u64> = recorded.iter().map(|u| u.char_index).collect();
                self.mark_unmappable(doc_id, &indices);
                self.open_unmappable(
                    UnmappableAsk {
                        doc: doc_id,
                        path: path.clone(),
                        expected,
                        cont,
                        first_char: recorded[0].char_index,
                        encoding,
                        fmt,
                    },
                    total,
                    encoding,
                    recorded.iter().any(|u| is_escape_char(u.ch)),
                );
            }
            JobResult::Unmappable { total, .. } => {
                self.show_message(
                    MessageLevel::Error,
                    "msg.save.failed",
                    &[
                        ("path", &path.display()),
                        ("reason", &format!("unmappable ({total})")),
                    ],
                );
            }
            // 原子的置換ができない・ハードリンク・バックアップを作れない: 確認を出す (承認の後は直接上書き、03 §9.3 手順 7)
            JobResult::Failed {
                err: JobError::Fs(ref e),
                ..
            } if fallback_kind(&e.kind, &e.detail).is_some()
                && self.documents.contains_key(&doc_id) =>
            {
                let kind =
                    fallback_kind(&e.kind, &e.detail).unwrap_or(FallbackKind::AtomicUnsupported);
                let links = match &e.detail {
                    Some(FsErrorDetail::Hardlink { nlink }) => nlink.saturating_sub(1).to_string(),
                    _ => String::new(),
                };
                let reason = fs_reason(e);
                self.open_fallback(
                    FallbackAsk {
                        kind,
                        doc: doc_id,
                        path: path.clone(),
                        expected,
                        cont,
                        fmt,
                        conflict_current: None,
                    },
                    &reason,
                    &links,
                );
            }
            // 保存の直前の照合で、開いた後に保存先が変わっていた: 書き込まずに「上書き / 別名で保存 / 中止」を尋ねる
            // (FR-117 AC-1、03 §9.5)
            JobResult::Failed {
                err:
                    JobError::Fs(FsError {
                        kind: FsErrorKind::Conflict { current },
                        ..
                    }),
                ..
            } if self.documents.contains_key(&doc_id) => {
                let kind = match (expected, current) {
                    (Expect::Absent, _) => FallbackKind::ConflictCreated,
                    (_, None) => FallbackKind::ConflictDeleted,
                    _ => FallbackKind::ConflictChanged,
                };
                self.open_fallback(
                    FallbackAsk {
                        kind,
                        doc: doc_id,
                        path: path.clone(),
                        expected,
                        cont,
                        fmt,
                        conflict_current: current,
                    },
                    "",
                    "",
                );
            }
            JobResult::Failed {
                err:
                    JobError::Fs(FsError {
                        kind: FsErrorKind::InPlaceFailed { backup },
                        ..
                    }),
                ..
            } => {
                let ask = self.save_failed_ask(doc_id, path, expected, fmt, cont);
                // 直接上書きの途中の失敗: 元のファイルは壊れている可能性がある。バックアップの場所を必ず示す (05 §4.4 手順 8)
                let b = backup.as_ref().map(FsPath::display).unwrap_or_default();
                let key = if backup.is_some() {
                    "msg.save.in_place_failed"
                } else {
                    "msg.save.in_place_failed_no_backup"
                };
                self.report_save_failure(
                    ask,
                    key,
                    &[("path", &path.display()), ("backup", &b)],
                    false,
                );
            }
            JobResult::Failed { err, .. } => {
                // 上書き保存が権限エラーで失敗したら、文書を読み取り専用 (書込権限なし) に切り替える。編集した内容は失われず、
                // 名前を付けて保存で書ける (FR-116 AC-3、03 §9.6)。名前を付けて保存の失敗では付けない (失敗したのは別のパス)
                if matches!(&err, JobError::Fs(e) if e.kind == FsErrorKind::PermissionDenied)
                    && let Some(doc) = self.documents.get_mut(&doc_id)
                    && doc.path.as_ref() == Some(path)
                {
                    doc.read_only
                        .insert(crate::document::ReadOnlyReason::NoWritePermission);
                    self.mark_dirty_render();
                }
                let ask = self.save_failed_ask(doc_id, path, expected, fmt, cont);
                // 照合の失敗 (Conflict) は別の経路 (外部変更の確認、後続)。取消しは確認なしで示す
                let ask = ask.filter(|_| {
                    !matches!(
                        err,
                        JobError::Cancelled
                            | JobError::Fs(FsError {
                                kind: FsErrorKind::Conflict { .. },
                                ..
                            })
                    )
                });
                let retry = matches!(&err, JobError::Fs(e) if e.kind == FsErrorKind::Busy);
                let err_backup = match &err {
                    JobError::Fs(FsError {
                        kind: FsErrorKind::ReplacedMovedToBackup { backup },
                        ..
                    }) => Some(backup.clone()),
                    _ => None,
                };
                let reason = reason_label(&err);
                let dir = match &err {
                    JobError::Fs(FsError {
                        detail: Some(FsErrorDetail::ParentMissing { dir }),
                        ..
                    }) => dir.display(),
                    _ => String::new(),
                };
                // ダングリングリンク: リンク先を示す (FR-106 AC-2)
                let link_target = match &err {
                    JobError::Fs(FsError {
                        detail: Some(FsErrorDetail::DanglingLink { target }),
                        ..
                    }) => target.display(),
                    _ => String::new(),
                };
                let (level, key) = match err {
                    JobError::Cancelled => (MessageLevel::Info, "info.save.aborted"),
                    JobError::Fs(e) => (MessageLevel::Error, save_error_key(&e)),
                    _ => (MessageLevel::Error, "msg.save.failed"),
                };
                let backup = match &err_backup {
                    Some(b) => b.display(),
                    None => String::new(),
                };
                let args = [
                    ("path", path.display()),
                    ("reason", reason.clone()),
                    ("dir", dir),
                    ("backup", backup),
                    ("target", link_target),
                ];
                let args: Vec<(&str, &str)> = args.iter().map(|(k, v)| (*k, v.as_str())).collect();
                match ask {
                    Some(ask) => self.report_save_failure(Some(ask), key, &args, retry),
                    None => self.show_message(level, key, &args),
                }
            }
            _ => {}
        }
    }
}

fn fallback_kind(kind: &FsErrorKind, detail: &Option<FsErrorDetail>) -> Option<FallbackKind> {
    match (kind, detail) {
        (FsErrorKind::AtomicUnsupported, Some(FsErrorDetail::Hardlink { .. })) => {
            Some(FallbackKind::Hardlink)
        }
        (FsErrorKind::AtomicUnsupported, _) => Some(FallbackKind::AtomicUnsupported),
        (FsErrorKind::BackupFailed, _) => Some(FallbackKind::NoBackup),
        _ => None,
    }
}

/// §8.3 のエスケープ文字 (不正バイトの保持マーカー、U+10FF00 + byte)。UTF-8 でも表現不能として扱う (E4)。
fn is_escape_char(c: char) -> bool {
    (0x10FF00..=0x10FFFF).contains(&(c as u32))
}

/// バイト数を「512.0 MiB」のような短い表記にする。
fn human_size(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u + 1 < UNITS.len() {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

/// `{reason}` に入れる短い分類名 (技術的な識別子。OS のエラーの分類文言 `ui.reason.*` は目録ができてから)。
pub(super) fn reason_label(err: &JobError) -> String {
    match err {
        JobError::Fs(e) => fs_reason(e),
        JobError::Panic => "panic".to_owned(),
        JobError::OutOfMemory { .. } => "out of memory".to_owned(),
        JobError::Cancelled => "cancelled".to_owned(),
    }
}

fn fs_reason(e: &FsError) -> String {
    format!("{:?}", e.kind)
        .split([' ', '{'])
        .next()
        .unwrap_or("unknown")
        .to_lowercase()
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
        // 置換の途中で失敗し、元のファイルは退避名に残っている (NFR-301 AC-2)。そのパスを必ず示す
        (FsErrorKind::ReplacedMovedToBackup { .. }, _) => "msg.save.moved_to_backup",
        (FsErrorKind::NotFound, Some(FsErrorDetail::ParentMissing { .. })) => {
            "msg.save.parent_missing"
        }
        // リンク先が無いシンボリックリンク: 書かずに、リンク先を示して知らせる (FR-106 AC-2)
        (FsErrorKind::NotFound, Some(FsErrorDetail::DanglingLink { .. })) => {
            "msg.save.dangling_link"
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
        // パスがなければ入力プロンプトで尋ねる
        None => {
            ctx.state.open_prompt(PromptKind::Open, String::new());
            CommandResult::Done
        }
    }
}

fn file_reopen_with_encoding(ctx: &mut CommandCtx<'_>) -> CommandResult {
    let enc = match ctx.args.str("encoding") {
        Some(name) => match Encoding::from_label(name) {
            Some(e) => Some(e),
            None => return CommandResult::Rejected(RejectReason::InvalidArgs),
        },
        None => None,
    };
    ctx.state.reopen_with_encoding(enc)
}

fn file_reload(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.reload()
}

fn file_set_encoding(ctx: &mut CommandCtx<'_>) -> CommandResult {
    let enc = match ctx.args.str("encoding") {
        Some(name) => match Encoding::from_label(name) {
            Some(e) => Some(e),
            None => return CommandResult::Rejected(RejectReason::InvalidArgs),
        },
        None => None,
    };
    let bom = match ctx.args.get("bom") {
        Some(super::command::ArgValue::Bool(b)) => Some(*b),
        _ => None,
    };
    ctx.state.set_encoding(enc, bom)
}

fn file_save(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.request_save()
}

fn file_save_as(ctx: &mut CommandCtx<'_>) -> CommandResult {
    match arg_path(ctx) {
        Some(p) => ctx.state.request_save_as(&p, None),
        None => {
            // 名前を尋ねる。初期値は今のパス
            let initial = ctx
                .state
                .active_document()
                .and_then(|d| ctx.state.document(d))
                .and_then(|d| d.path.as_ref().map(FsPath::display))
                .unwrap_or_default();
            ctx.state
                .open_prompt(PromptKind::SaveAs { then: None }, initial);
            CommandResult::Done
        }
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
    reg!(
        r,
        "file.reload",
        CommandKind::File,
        "doc.has_path",
        file_reload
    );
    reg!(
        r,
        "file.set_encoding",
        CommandKind::File,
        "",
        file_set_encoding
    );
    reg!(
        r,
        "file.reopen_with_encoding",
        CommandKind::File,
        "doc.has_path",
        file_reopen_with_encoding
    );
}

#[cfg(test)]
mod save_error_tests {
    use super::*;

    // verifies: AC-FR-106-2
    #[test]
    fn ダングリングリンクは専用のメッセージになる() {
        let e = FsError {
            kind: FsErrorKind::NotFound,
            detail: Some(FsErrorDetail::DanglingLink {
                target: crate::FsPath::from(r"C:\gone\a.txt"),
            }),
            source: None,
        };
        assert_eq!(save_error_key(&e), "msg.save.dangling_link");
    }
}
