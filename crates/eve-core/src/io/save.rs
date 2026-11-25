//! 保存 (docs/03 §9.3): 上書き・名前を付けて保存・未編集の保存の照合 (手順 0)・保存の失敗の選択肢・直接上書きへの退路・
//! 読み取り専用の解除・保存の警告 (STR-B1 で io/file.rs から分けた)。

use eve_view::{DocumentId, EditorEvent, MessageLevel};

use crate::io::fs::{
    Expect, FsError, FsErrorDetail, FsErrorKind, HardlinkPolicy, InPlaceBackup, SaveMethod,
};
use crate::io::fspath::FsPath;
use crate::jobs::{JobError, JobKind, JobResult, SaveStrategy};
use crate::ui_model::keys;

use super::error_keys::{fs_reason, reason_label, save_error_key, save_failure};
use super::file::*;
use super::open::*;
use super::pending::*;
use super::unmappable::*;
use crate::command::{CommandResult, RejectReason};
use crate::editor::state::EditorState;
use crate::ui_model::overlay::{Continuation, PromptKind};

/// 保存の失敗の後の選択 (`ui.confirm.save_failed`、03 §9.7 (e))。
#[derive(Clone, Debug)]
pub(crate) struct SaveFailedAsk {
    pub doc: DocumentId,
    pub path: FsPath,
    pub expected: Expect,
    pub cont: Option<Continuation>,
    /// 失敗した保存の時点の本文 (確認の間は編集できない)。「退避先に保存」で書く。
    pub snapshot: std::sync::Arc<crate::DocSnapshot>,
    pub fmt: Option<NewFormat>,
}

/// 保存の代替の確認 (`AtomicUnsupported` / ハードリンク / `BackupFailed`、03 §9.3 手順 7・05 §4.4)。
#[derive(Clone, Debug)]
pub(crate) struct FallbackAsk {
    pub kind: FallbackKind,
    pub doc: DocumentId,
    pub path: FsPath,
    pub expected: Expect,
    pub cont: Option<Continuation>,
    pub fmt: Option<NewFormat>,
    /// 競合 (`Conflict`) の確認のとき、照合で見つけた今の保存先の状態 (`None` = ファイルが無い)。「上書き」はこれを新しい期待値にする。
    pub conflict_current: Option<crate::FileStamp>,
}

/// 保存中 (`Saving`、後続なし) に届いた同じ文書の保存要求。前の保存が `Saved` で終わった後に、新しい要求として再実行する
/// (03 §9.3 手順 4・§9.7 の全状態の遷移表の `file.save` の列。`file.save` は `file.save_as` / `file.set_encoding` を含む)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum QueuedSave {
    /// `file.save`。
    Save,
    /// `file.save_as` (入力した名前と、`file.set_encoding` の名前の入力から来たときの新しい書式)。
    SaveAs { raw: String, fmt: Option<NewFormat> },
    /// `file.set_encoding` (新しい書式。パスが無い・読み取り専用の文書は、再実行で名前を尋ねる)。
    SetEncoding(NewFormat),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FallbackKind {
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

/// 保存の連鎖の開始時に拒否する文書か (03 §9.7 の「連鎖の開始時の拒否」、§8.3 の (a)、OQ-207): 書く文字コード
/// (`fmt` が無ければ文書の今の文字コード) が状態付き (ISO-2022-JP) で、文書が不正バイトを含む (`invalid_bytes > 0`)。
/// 状態付き符号化では不正バイトを元のバイトに戻して書けないため。判断はここだけで行う。
pub(crate) fn stateful_invalid(doc: &crate::document::Document, fmt: Option<NewFormat>) -> bool {
    let out_encoding = fmt.map_or(doc.format.encoding, |f| f.encoding);
    out_encoding.is_stateful() && doc.format.invalid_bytes > 0
}

impl EditorState {
    /// 保存の連鎖の開始時の拒否: 文書が `stateful_invalid` に当たれば、
    /// `msg.save.encoding_unsupported_iso2022` を示して `true` を返す。呼び側はジョブを投入せず、名前の入力・照合・確認の
    /// どれにも進まず、後続 (`cont`) も実行しない (文書・書式は変えない)。
    /// 保存の連鎖の入口 (`file.save`・`file.save_as`・`file.set_encoding`・`AskUnsaved` の「保存」・全保存) は、
    /// 書式が決まった直後にこれを呼ぶ。`start_save_with` も投入の前に呼ぶ (入口を経ずに投入し直す経路の受け止め)。
    /// 読込が済んでいない文書は対象にしない (読込中の拒否は各入口の規則に任せる)。
    pub(crate) fn reject_stateful_invalid(
        &mut self,
        doc_id: DocumentId,
        fmt: Option<NewFormat>,
    ) -> bool {
        let Some(doc) = self.docs.documents.get(&doc_id) else {
            return false;
        };
        if doc.load_state != crate::document::LoadState::Ready || !stateful_invalid(doc, fmt) {
            return false;
        }
        let shown = doc.path.as_ref().map(FsPath::display).unwrap_or_default();
        self.show_message(
            MessageLevel::Error,
            keys::MSG_SAVE_ENCODING_UNSUPPORTED_ISO2022,
            &[("path", &shown)],
        );
        true
    }

    /// 確認で「保存」を選んだ後: `doc_id` を保存し、成功したら `cont` を続ける。パスがなければ名前を尋ねてから保存する。
    /// 対象はアクティブ文書ではなく `doc_id` (保存の連鎖は保存した文書に属する、03 §9.8)。
    pub(crate) fn save_doc_then(
        &mut self,
        doc_id: DocumentId,
        cont: Continuation,
    ) -> CommandResult {
        let command = match cont {
            // `app.quit_with_error` は終了コードで見分ける (07 §18.4)
            Continuation::Quit if self.ui.quit_code != 0 => {
                crate::command::ids::APP_QUIT_WITH_ERROR
            }
            Continuation::Quit => crate::command::ids::APP_QUIT,
            Continuation::Close(_) => crate::command::ids::BUFFER_CLOSE,
            Continuation::Reopen(..) => crate::command::ids::FILE_REOPEN_WITH_ENCODING,
        };
        self.save_doc_then_from(doc_id, cont, command)
    }

    /// `save_doc_then` の、保存を始めたコマンド (保存時の変換の Transaction の `Origin::Command`、03 §9.3.1 の SVT-5) を
    /// 指定する版 (`app.save_and_quit`)。
    pub(crate) fn save_doc_then_from(
        &mut self,
        doc_id: DocumentId,
        cont: Continuation,
        command: &'static str,
    ) -> CommandResult {
        if self.document(doc_id).is_none() {
            return CommandResult::Rejected(RejectReason::NoTarget);
        }
        // 保存中なら、その保存の完了後に続ける (二重に保存しない)
        let in_flight = self.io.pending.iter().find_map(|(job, p)| match p {
            Pending::Save { doc, .. } if *doc == doc_id => Some(*job),
            _ => None,
        });
        if let Some(job) = in_flight {
            // 後続を付け直したので、`Esc` で後続を取り消した印 (`cancel_origin = User`) は外す (03 §9.7 の
            // `cancel_origin = User` の行の `app.quit` と同じ。取消しが間に合っていれば中止になり、後続は行わない)
            self.ui.save_cancelled_by_user.remove(&job);
            self.ui.continuations.insert(job, cont);
            // 後続を付けた保存は単独の保存でなくなるので、積んだ保存要求も捨てる (03 §9.3 手順 4)
            self.io.save_queue.remove(&doc_id);
            return CommandResult::Pending(job);
        }
        // `AskUnsaved` の「保存」は新しい連鎖 (保存の完了の後に出し直したものも。03 §9.3.1 の SVT-4 (c))
        self.begin_save_chain(doc_id, command);
        // 名前の入力 (無題・読み取り専用) より先に拒否する。後続 (終了・閉じる・開き直し) も行わない (03 §9.7)
        if self.reject_stateful_invalid(doc_id, None) {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        let Some(doc) = self.document(doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(path) = doc.path.clone() else {
            self.open_prompt(
                PromptKind::SaveAs {
                    doc: doc_id,
                    then: Some(cont),
                },
                String::new(),
            );
            return CommandResult::Done;
        };
        if doc.load_state != crate::document::LoadState::Ready {
            self.message(MessageLevel::Info, keys::INFO_RELOAD_BUSY, &path.display());
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        // 読み取り専用 (`--readonly`・書込権限なし・不正バイト等) の文書は同じパスへ書かず、名前を尋ねる
        // (03 §9.7 の AskUnsaved の行。docs/review/v0.3-findings.md の SAF-18)
        if !doc.read_only.is_empty() {
            self.open_prompt(
                PromptKind::SaveAs {
                    doc: doc_id,
                    then: Some(cont),
                },
                path.display(),
            );
            return CommandResult::Done;
        }
        let expected = doc.disk_stamp.map_or(Expect::Absent, Expect::Stamp);
        self.start_save(doc_id, path, expected, Some(cont))
    }

    /// 文書 `doc_id` を名前を付けて保存する (パスを指定する)。対象は名前の入力を開いた時点の文書で、確定の時点の
    /// 表示中の文書ではない (03 §9.6・§9.8 の「入力・確認の対象」)。
    pub(crate) fn request_save_as(
        &mut self,
        doc_id: DocumentId,
        raw_path: &str,
        cont: Option<Continuation>,
    ) -> CommandResult {
        self.request_save_as_fmt(doc_id, raw_path, cont, None)
    }

    /// `request_save_as` に新しい書式を渡す版 (`file.set_encoding` の無題・読み取り専用の文書、03 §8.8)。
    pub(crate) fn request_save_as_fmt(
        &mut self,
        doc_id: DocumentId,
        raw_path: &str,
        cont: Option<Continuation>,
        fmt: Option<NewFormat>,
    ) -> CommandResult {
        if self.document(doc_id).is_none() {
            return CommandResult::Rejected(RejectReason::NoTarget);
        }
        // 保存先を調べる前に拒否する (03 §9.7 の「連鎖の開始時の拒否」)。名前の入力の後の呼び出しでは、入力の前の
        // 拒否を経ているので、ここで拒否するのは待つ間に書式が変わったときだけ
        if self.reject_stateful_invalid(doc_id, fmt) {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        let Ok(path) = self.paths.normalize(raw_path) else {
            self.show_message(
                MessageLevel::Error,
                keys::MSG_SAVE_INVALID_PATH,
                &[("path", raw_path), ("reason", "invalid path")],
            );
            return CommandResult::Rejected(RejectReason::InvalidArgs);
        };
        let Some(doc) = self.docs.documents.get(&doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        // 読込中・読込に失敗した文書は保存できない (中身が揃っていない)
        if doc.load_state != crate::document::LoadState::Ready {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        // 保存の連鎖の途中: 単独の保存を待っていれば積み、ほかは無視する (03 §9.7 の全状態の遷移表の `file.save` の列)。
        // 後続のある名前の入力 (`AskUnsaved` の「保存」から) の確定は積まない (後続は保存中の連鎖に付く)
        let shown = path.display();
        let deferred = match &cont {
            None => self.queue_or_ignore_save_request(
                doc_id,
                QueuedSave::SaveAs {
                    raw: raw_path.to_owned(),
                    fmt,
                },
                &shown,
            ),
            Some(_) => self.ignore_save_request(doc_id, &shown),
        };
        if let Some(r) = deferred {
            return r;
        }
        let Some(doc) = self.docs.documents.get(&doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        // 03 §9.7 (a) の 1 (構文的に同じパス、I/O なし): 通常の上書き保存。読み取り専用の文書は同じファイルへ書かない
        if doc.path.as_ref() == Some(&path) {
            return self.save_as_same_file(doc_id, raw_path, cont, fmt);
        }
        // 別のパス: 保存先の照合のジョブ (`CheckTarget`) を投入する。別名・リンクで今の文書・ほかの文書と同じファイルか、
        // 既存か、読み取り専用かは結果で決める (io/save_as.rs)。v0.4 までは照合せず `Expect::Absent` で書いていた
        self.start_save_as_check(super::save_as::SaveAsAsk {
            doc: doc_id,
            path,
            raw: raw_path.to_owned(),
            cont,
            fmt,
            stamp: None,
            target: Default::default(),
        })
    }

    pub(crate) fn start_save(
        &mut self,
        doc_id: DocumentId,
        path: FsPath,
        expected: Expect,
        cont: Option<Continuation>,
    ) -> CommandResult {
        self.start_save_fmt(doc_id, path, expected, cont, None)
    }

    /// 新しい書式 `fmt` を保存に渡す (`file.set_encoding`)。成功したときだけ文書の書式に確定する。
    pub(crate) fn start_save_fmt(
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
    pub(crate) fn start_save_with(
        &mut self,
        doc_id: DocumentId,
        path: FsPath,
        expected: Expect,
        cont: Option<Continuation>,
        strategy: SaveStrategy,
        fmt: Option<NewFormat>,
    ) -> CommandResult {
        // ダングリングのリンクのリンク先への作成は、承認した保存の 1 回だけ
        let create_link_target = self
            .io
            .link_create_ok
            .take_if(|(d, p)| *d == doc_id && *p == path)
            .is_some();
        let Some(doc) = self.docs.documents.get_mut(&doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        // 読込中 (開き直し・再読込の途中を含む) は保存しない。旧本文の保存の結果が新しい文書の保存点と照合を狂わせる
        // (保存の経路はすべてここを通る。コマンドの when 句を経ない確認の「保存」からも。docs/review/v0.3-findings.md の SAF-17)
        if doc.load_state != crate::document::LoadState::Ready {
            let shown = path.display();
            self.message(MessageLevel::Info, keys::INFO_RELOAD_BUSY, &shown);
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        // 状態付き符号化 (ISO-2022-JP) で不正バイトを含む文書は保存しない (03 §8.3 (a)。不正バイトを失わないため)。
        // 入口 (`file.save` 等) は名前の入力の前に同じ判断で拒否している。ここは入口を経ずに投入し直す経路
        // (確認の後の再投入・E3 の「UTF-8 で保存」等) の受け止めで、保存の前の確認のどれよりも先に拒否する
        // (03 §9.7 の「連鎖の開始時の拒否」)
        if self.reject_stateful_invalid(doc_id, fmt) {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        let Some(doc) = self.docs.documents.get_mut(&doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        // 読み取り専用属性を解除した文書の上書き保存は、「読み取り専用を解除して保存」の確認を経る (FR-116、03 §9.3 手順 1)。
        // 承認した Revision では繰り返さない
        let clear_readonly = doc.attribute_released && doc.path.as_ref() == Some(&path);
        if clear_readonly && self.io.readonly_ok != Some((doc_id, doc.revision())) {
            let ask = LossyAsk {
                doc: doc_id,
                path,
                expected,
                cont,
                strategy,
                fmt,
            };
            self.open_clear_readonly(ask);
            return CommandResult::Done;
        }
        let out_encoding = fmt.map_or(doc.format.encoding, |f| f.encoding);
        // 保存の前の確認 (FR-112 AC-4 (a)・NFR-303、03 §8.6 E2'・§8.7.1): 元の文字コードのまま書くと、元のバイト列を
        // 覚えていない区分 B / D の箇所が別のバイト表現になり得る文書 (「保持なし」「保持できなかった箇所がある」) で、
        // 書き込む前に確認する。v0.4 は未編集の文書でも出す (名前を付けて保存・`file.set_encoding` で同じ文字コード。
        // 未編集の上書き保存は手順 0 で書き込まないので、ここへ来ない)。承認した Revision では繰り返さない
        if let Some(kind) = super::open::preconfirm(doc, out_encoding)
            && self.io.lossy_ok != Some((doc_id, doc.revision()))
        {
            let ask = LossyAsk {
                doc: doc_id,
                path,
                expected,
                cont,
                strategy,
                fmt,
            };
            self.open_lossy(ask, kind);
            return CommandResult::Done;
        }
        // 保存時の変換 (03 §9.3.1): すべての確認の後、書出しのジョブを投入する直前 (Snapshot を取る前) に同期で 1 回だけ
        // 当てる。この連鎖で判定済み (確認の後の投入し直し) なら当てない。変換で進んだ Revision に承認を引き継ぐ
        self.apply_save_transform(doc_id, &path);
        let Some(doc) = self.docs.documents.get_mut(&doc_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let (encoding, bom, escape) = (
            out_encoding,
            fmt.map_or(doc.format.has_bom, |f| f.bom),
            doc.format.save_escape_policy(out_encoding),
        );
        let revision = doc.revision();
        // つながりの承認 (03 §8.3.1 の「承認」、§9.7 の表の `escape_merge_ok` の列): 投入のたびに `SaveApprovals` から、
        // この文書の今の Revision のものを引く (無ければ `None`)。ジョブは結果が承認と同じときだけ止めずに書く
        let escape_merge_ok = self
            .io
            .escape_merge_ok
            .as_ref()
            .filter(|(d, r, _)| *d == doc_id && *r == revision)
            .map(|(_, _, a)| a.clone());
        let merged = escape_merge_ok.as_ref().map(|a| a.total);
        // 保持方式の記録の、保存を始めた時点の版 (複製は O(1)。03 §9.3 手順 4)。Snapshot と同じ時点で取る
        let preserved = doc.preserved_for(out_encoding);
        let (snapshot, at) = doc.begin_save();
        let job = self.jobs.spawn(
            Some(doc_id),
            revision,
            JobKind::Save {
                path: path.clone(),
                snapshot,
                encoding,
                bom,
                escape,
                expected,
                strategy,
                clear_readonly,
                create_link_target,
                preserved,
                escape_merge_ok,
            },
        );
        if let Some(c) = cont {
            self.ui.continuations.insert(job, c);
        }
        self.io.pending.insert(
            job,
            Pending::Save {
                doc: doc_id,
                at,
                revision,
                path,
                expected,
                fmt,
                strategy,
                merged,
            },
        );
        CommandResult::Pending(job)
    }

    /// 閉じた文書の保存の結果が届いた: 公開物を外す。保存が成功していなければ、内容を失わないよう退避先に書く
    /// (閉じた文書なので確認は出さない。書いた場所は `finish_write_recovery` のメッセージで示す)。
    pub(crate) fn release_closing_snapshot(&mut self, doc: DocumentId, path: &FsPath, saved: bool) {
        if self.docs.documents.contains_key(&doc) {
            return;
        }
        if saved {
            self.io.closing_snapshots.remove(&doc);
            self.mark_snapshot_update(doc);
            return;
        }
        // 失敗: 退避先への書込が終わるまで、公開物は緊急退避に残す
        let Some(snapshot) = self.io.closing_snapshots.get(&doc).cloned() else {
            return;
        };
        let job = self.jobs.spawn(
            Some(doc),
            snapshot.revision,
            JobKind::WriteRecovery {
                doc: std::sync::Arc::new(snapshot),
            },
        );
        self.io.pending.insert(
            job,
            Pending::WriteRecovery {
                doc,
                path: path.clone(),
                release: Some(doc),
            },
        );
    }

    /// 保存の失敗の確認に渡す材料 (文書が無い・読込中なら `None`)。
    pub(crate) fn save_failed_ask(
        &self,
        doc: DocumentId,
        path: &FsPath,
        expected: Expect,
        fmt: Option<NewFormat>,
        cont: Option<Continuation>,
    ) -> Option<SaveFailedAsk> {
        // 閉じた文書には確認を出さない (保存の結果の処理が退避先に書く)
        self.document(doc)?;
        let snapshot = self.rescue_snapshot(doc, crate::io::snapshot::SnapshotScope::Any)?;
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
    pub(crate) fn report_save_failure(
        &mut self,
        ask: Option<SaveFailedAsk>,
        key: &str,
        args: &[(&str, &str)],
        retry: bool,
    ) {
        use crate::ui_model::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
        let Some(ask) = ask else {
            self.show_message(MessageLevel::Error, key, args);
            return;
        };
        // 保存時の変換を当てた連鎖なら、変換が本文に残っていることを本文の後ろに付ける (03 §9.3.1 の SVT-8)
        let detail = match self.save_transform_kept_suffix(ask.doc) {
            Some(suffix) => format!("{} {suffix}", self.message_text(key, args)),
            None => self.message_text(key, args),
        };
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
            message: None,
            action: ConfirmAction::SaveFailed(ask),
        });
    }

    /// 保存の失敗の確認の選択 (キャンセルは呼び側が処理済み)。`id` は選択肢の ID。
    pub(crate) fn choose_save_failed(&mut self, ask: SaveFailedAsk, id: &str) -> CommandResult {
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
                self.io.pending.insert(
                    job,
                    Pending::WriteRecovery {
                        doc,
                        path,
                        release: None,
                    },
                );
                CommandResult::Pending(job)
            }
            // 別名で保存: 名前の入力へ (保存の後に続ける操作は引き継ぐ)
            _ => {
                self.open_prompt(PromptKind::SaveAs { doc, then: cont }, String::new());
                CommandResult::Done
            }
        }
    }

    /// 「退避先に保存」の結果。文書の状態 (dirty・保存点) は変えない。保存時の変換が本文に残っていれば付記する (SVT-8)。
    pub(crate) fn finish_write_recovery(&mut self, doc: DocumentId, path: &FsPath, r: JobResult) {
        let kept = self.save_transform_kept_suffix(doc);
        match r {
            JobResult::RecoveryWritten { path: target, .. } => self.show_message_with_suffix(
                MessageLevel::Info,
                keys::INFO_SAVE_TO_RECOVERY,
                &[("target", &target.display()), ("path", &path.display())],
                kept.as_deref(),
            ),
            JobResult::Failed { err, .. } => {
                let reason = reason_label(&err);
                self.show_message_with_suffix(
                    MessageLevel::Error,
                    keys::MSG_SAVE_RECOVERY_FAILED,
                    &[("reason", &reason), ("path", &path.display())],
                    kept.as_deref(),
                );
            }
            _ => {}
        }
    }

    /// 保存の代替の確認を出す。
    pub(crate) fn open_fallback(&mut self, ask: FallbackAsk, reason: &str, links: &str) {
        use crate::ui_model::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
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
            message: None,
            action: ConfirmAction::Fallback(ask),
        });
    }

    /// 「読み取り専用を解除して保存」の確認を出す。選択肢: 別名で保存 (既定) / 読み取り専用を解除して保存 (04 §15.2)。
    pub(crate) fn open_clear_readonly(&mut self, ask: LossyAsk) {
        use crate::ui_model::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
        let choice = |id: &'static str, accel: char| ChoiceDef {
            id,
            accel: Some(accel),
            destructive: false,
        };
        self.open_confirm(ConfirmState {
            name: "save.read_only",
            args: vec![("path".to_owned(), ask.path.display())],
            choices: vec![choice("alt", 'a'), choice("write", 'w')],
            details: Vec::new(),
            default: 0,
            focused: 0,
            message: None,
            action: ConfirmAction::ClearReadOnly(ask),
        });
    }

    /// 「読み取り専用を解除して保存」の確認の選択 (キャンセルは呼び側が処理済み)。
    pub(crate) fn choose_clear_readonly(&mut self, ask: LossyAsk, choice: usize) -> CommandResult {
        let LossyAsk {
            doc,
            path,
            expected,
            cont,
            strategy,
            fmt,
        } = ask;
        if choice == 1 {
            if let Some(d) = self.docs.documents.get(&doc) {
                self.io.readonly_ok = Some((doc, d.revision()));
            }
            return self.start_save_with(doc, path, expected, cont, strategy, fmt);
        }
        self.open_prompt(PromptKind::SaveAs { doc, then: cont }, String::new());
        CommandResult::Done
    }

    /// 保存の代替の確認の選択 (キャンセルは呼び側が処理済み)。
    pub(crate) fn choose_fallback(&mut self, ask: FallbackAsk, choice: usize) -> CommandResult {
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
                self.open_prompt(PromptKind::SaveAs { doc, then: cont }, String::new());
                CommandResult::Done
            }
        }
    }

    /// 文書 `doc` の保存の連鎖がジョブを待っていれば (`CheckingUnchanged`・`CheckTarget`・`Saving`・`BuildingReplacement`・
    /// `WritingRecovery`。判定は `save_chain_busy`)、同じ文書の保存要求を無視して
    /// `info.save.in_progress` (`{path}` は `shown`) を示し、`Some` を返す (03 §9.7 の全状態の遷移表の「無視」)。
    /// 待っていなければ `None` (呼び側が続ける)。
    ///
    /// spec: 03 §9.7
    pub(crate) fn ignore_save_request(
        &mut self,
        doc: DocumentId,
        shown: &str,
    ) -> Option<CommandResult> {
        if !self.save_chain_busy(doc) {
            return None;
        }
        self.message(MessageLevel::Info, keys::INFO_SAVE_IN_PROGRESS, shown);
        Some(CommandResult::Done)
    }

    /// 同じ文書の保存要求 `req` が保存の連鎖の途中に届いた (03 §9.7 の全状態の遷移表の `file.save` の列)。単独の保存
    /// (`Saving`。後続が無く、`Esc` で取り消しておらず、「保存を中止して終了」の最中でない) を待っていれば、深さ 1 の
    /// 待ち行列に積む (後に届いた要求で置き換える。知らせない、04 §15.2 の `info.save.in_progress`)。ほかの待ち (未編集の
    /// 照合・名前を付けて保存の照合・後続のある保存・取り消した保存・置換の一覧の作成・退避先への保存) では `ignore_save_request` のとおり
    /// 無視する。連鎖の途中でなければ `None` (呼び側が続ける)。
    ///
    /// spec: 03 §9.3, 03 §9.7
    pub(crate) fn queue_or_ignore_save_request(
        &mut self,
        doc: DocumentId,
        req: QueuedSave,
        shown: &str,
    ) -> Option<CommandResult> {
        let single = self.ui.quit_abort.is_none()
            && self.io.pending.iter().any(|(job, p)| {
                matches!(p, Pending::Save { doc: d, .. } if *d == doc)
                    && !self.ui.continuations.contains_key(job)
                    && !self.ui.save_cancelled_by_user.contains(job)
            });
        if !single {
            return self.ignore_save_request(doc, shown);
        }
        self.io.save_queue.insert(doc, req);
        Some(CommandResult::Done)
    }

    /// 前の保存が `Saved` で終わった後、待ち行列に積んだ保存要求を再実行する。オーバーレイ・検索バーが出ていれば保留し、
    /// 閉じた後に `flush_held_queued_saves` が流す (利用者が開いた入力・確認を黙って置き換えない)。
    ///
    /// spec: 03 §9.3, 03 §9.7
    pub(crate) fn rerun_queued_save(&mut self, doc: DocumentId, req: QueuedSave) {
        if self.auto_switch_blocked() || !self.io.held_queued_saves.is_empty() {
            self.io.held_queued_saves.push((doc, req));
            return;
        }
        self.run_queued_save(doc, req);
    }

    /// 保留した保存の再実行を、オーバーレイ・検索バーが閉じている間、届いた順に流す (`flush_open_switches` から呼ぶ)。
    pub(crate) fn flush_held_queued_saves(&mut self) {
        while !self.auto_switch_blocked() && !self.io.held_queued_saves.is_empty() {
            let (doc, req) = self.io.held_queued_saves.remove(0);
            self.run_queued_save(doc, req);
        }
    }

    /// 積んだ保存要求を新しい要求として再実行する (03 §9.3 手順 4。保存時の変換も新しい連鎖として判定する、§9.3.1 の
    /// SVT-4 (c))。コマンドの経路と同じ判定を通す: `file.save` は読込済みで読み取り専用でない文書だけ (`when` の
    /// `!doc.read_only` と §9.3 手順 1。積んだ後に読み取り専用にした文書は書かない)、かつ dirty のときだけ (保存の間に編集が
    /// 無ければ、前の保存が最新の本文を書いている)。`file.save_as` / `file.set_encoding` は入口の関数を通す (同じファイルで
    /// 読み取り専用なら拒む・名前を尋ねるなど、その時点の文書の状態で決まる)。照合の期待値は、前の保存の結果で更新した
    /// `disk_stamp`。表示中のメッセージが警告・エラー (前の保存の警告など) なら、再実行の完了のメッセージは背景の知らせに
    /// する。文書が閉じられていれば何もしない。
    ///
    /// spec: 03 §9.3, 03 §9.3.1, 03 §9.7
    fn run_queued_save(&mut self, doc: DocumentId, req: QueuedSave) {
        let Some(d) = self.document(doc) else {
            return;
        };
        let runnable = match &req {
            QueuedSave::Save => {
                d.is_dirty()
                    && d.read_only.is_empty()
                    && d.load_state == crate::document::LoadState::Ready
            }
            QueuedSave::SaveAs { .. } | QueuedSave::SetEncoding(_) => true,
        };
        if !runnable {
            return;
        }
        if self.message_is_alert() {
            self.io.quiet_save_done.insert(doc);
        }
        match req {
            QueuedSave::Save => {
                self.request_save_for(doc);
            }
            QueuedSave::SaveAs { raw, fmt } => {
                // 書式付き (`file.set_encoding` の名前の入力から) なら、変換の発行コマンドは `file.set_encoding` (SVT-5)
                let command = if fmt.is_some() {
                    crate::command::ids::FILE_SET_ENCODING
                } else {
                    crate::command::ids::FILE_SAVE_AS
                };
                self.begin_save_chain(doc, command);
                self.request_save_as_fmt(doc, &raw, None, fmt);
            }
            QueuedSave::SetEncoding(fmt) => {
                self.set_encoding_for(doc, Some(fmt.encoding), Some(fmt.bom));
            }
        }
        // 保存ジョブを投入しなかった (照合・名前の入力・確認・拒否) なら、背景の知らせの印は使わない
        let saving = self
            .io
            .pending
            .values()
            .any(|p| matches!(p, Pending::Save { doc: d, .. } if *d == doc));
        if !saving {
            self.io.quiet_save_done.remove(&doc);
        }
    }

    /// `Esc` の段 (04 §7.5): 後続 (終了・閉じる・開き直し) を持つ保存 (`Saving`、`cont` が Some) を待っていれば、保存ジョブに
    /// 取消しを常に送り (ジョブの段は見ない)、後続を捨てて `cancel_origin = User` にする (03 §9.7 の全状態の遷移表)。
    /// 結果は単独の保存として扱う: `Saved` は採用して後続を行わず、`Cancelled` は中止 (`info.save.aborted`)、ほかの失敗は
    /// (e) の表。後続の無い保存・取り消し済みの保存・「保存を中止して終了」の最中は取り消さない (`false` を返して次の段へ
    /// 通す)。今の文書の保存を、無ければ最後に投入した保存を取り消す。後続が終了なら、終了そのものを取り消す (ほかの文書の
    /// 保存に付けた終了も外し、終了の連鎖と保存中の終了の待ちを止める。外した保存は単独の保存として続ける)。
    ///
    /// spec: 03 §9.7, 04 §7.5
    pub(crate) fn cancel_save_continuation(&mut self) -> bool {
        if self.ui.quit_abort.is_some() {
            return false;
        }
        let active = self.active_document();
        let saves: Vec<(eve_view::JobId, DocumentId)> = self
            .io
            .pending
            .iter()
            .filter_map(|(job, p)| match p {
                Pending::Save { doc, .. }
                    if self.ui.continuations.contains_key(job)
                        && !self.ui.save_cancelled_by_user.contains(job) =>
                {
                    Some((*job, *doc))
                }
                _ => None,
            })
            .collect();
        let Some((job, _)) = active_or_latest(&saves, active) else {
            return false;
        };
        self.jobs.cancel(job);
        self.ui.save_cancelled_by_user.insert(job);
        if self.ui.continuations.remove(&job) == Some(Continuation::Quit) {
            let pending = &self.io.pending;
            self.ui.continuations.retain(|j, c| {
                *c != Continuation::Quit || !matches!(pending.get(j), Some(Pending::Save { .. }))
            });
            self.ui.quit_chain = None;
            self.ui.quit_wait = None;
        }
        self.mark_dirty_render();
        true
    }

    /// 中止のメッセージの `{path}`: 文書のパス、無題なら文書の名前 (`無題-1` など)。文書が無ければ `None`。
    pub(crate) fn save_abort_path(&self, doc: DocumentId) -> Option<String> {
        let d = self.document(doc)?;
        Some(match d.path.as_ref() {
            Some(p) => p.display(),
            None => self.untitled_label(d),
        })
    }

    /// 保存の連鎖の中止を `info.save.aborted` で示す (03 §9.7 の「中止」、04 §15.2)。`{path}` は `path` (無ければ
    /// `save_abort_path`)。保存時の変換が本文に残っていれば付記する (03 §9.3.1 の SVT-8)。文書が閉じられていて `path` も
    /// 無ければ何も出さない。
    ///
    /// spec: 03 §9.7, 03 §9.3.1
    pub(crate) fn notify_save_aborted(&mut self, doc: DocumentId, path: Option<&FsPath>) {
        let Some(shown) = path
            .map(FsPath::display)
            .or_else(|| self.save_abort_path(doc))
        else {
            return;
        };
        let kept = self.save_transform_kept_suffix(doc);
        self.show_message_with_suffix(
            MessageLevel::Info,
            keys::INFO_SAVE_ABORTED,
            &[("path", &shown)],
            kept.as_deref(),
        );
    }

    /// `Esc` の段 (04 §7.5): 未編集の上書き保存の照合 (`CheckingUnchanged`) を待っていれば、照合の結果を捨てて後続も捨てる
    /// (03 §9.7 の全状態の遷移表。何も書いていない)。今の文書の照合を、無ければ最後に投入した照合を中止する。中止したら `true`。
    pub(crate) fn cancel_unchanged_check(&mut self) -> bool {
        let active = self.active_document();
        let checks: Vec<(eve_view::JobId, DocumentId)> = self
            .io
            .pending
            .iter()
            .filter_map(|(job, p)| match p {
                Pending::CheckUnchanged { doc } => Some((*job, *doc)),
                _ => None,
            })
            .collect();
        let Some((job, doc)) = active_or_latest(&checks, active) else {
            return false;
        };
        self.jobs.cancel(job);
        self.io.pending.remove(&job);
        self.ui.continuations.remove(&job);
        let shown = self
            .docs
            .documents
            .get(&doc)
            .and_then(|d| d.path.as_ref())
            .map(FsPath::display)
            .unwrap_or_default();
        self.message(MessageLevel::Info, keys::INFO_SAVE_ABORTED, &shown);
        self.mark_dirty_render();
        true
    }

    /// `Esc` の段 (04 §7.5): 表現不能の置換の一覧を作っている (`BuildingReplacement`) なら、ジョブに取消しを送り、E3 の
    /// 選択肢 3 (キャンセル) と同じ扱いで保存を中止する (保存しない。後続も捨てる。最初の該当位置へ移る、03 §8.6)。
    /// 今の文書の置換を、無ければ最後に投入した置換を中止する。中止したら `true`。
    pub(crate) fn cancel_building_replacement(&mut self) -> bool {
        let active = self.active_document();
        let jobs: Vec<(eve_view::JobId, DocumentId)> = self
            .io
            .pending
            .iter()
            .filter_map(|(job, p)| match p {
                Pending::ReplaceUnmappable { ask, .. } => Some((*job, ask.doc)),
                _ => None,
            })
            .collect();
        let Some((job, _)) = active_or_latest(&jobs, active) else {
            return false;
        };
        self.jobs.cancel(job);
        self.ui.continuations.remove(&job);
        if let Some(Pending::ReplaceUnmappable { ask, .. }) = self.io.pending.remove(&job) {
            self.unmappable_cancelled(&ask);
        }
        self.mark_dirty_render();
        true
    }

    pub(crate) fn finish_check(&mut self, doc_id: DocumentId, r: JobResult) {
        let Some(doc) = self.docs.documents.get(&doc_id) else {
            return;
        };
        let shown = doc.path.as_ref().map(FsPath::display).unwrap_or_default();
        let unchanged = match &r {
            // 読んだときの Stamp と一致し、編集もなければ書き込まない
            JobResult::Meta { stamp, .. } => {
                !doc.is_dirty() && doc.disk_stamp.is_some_and(|d| same_on_disk(&d, stamp))
            }
            _ => false,
        };
        match r {
            _ if unchanged => self.message(MessageLevel::Info, keys::INFO_SAVE_UNCHANGED, &shown),
            // NotFound は不一致として保存へ進む。その他の失敗は照合の失敗として示す
            JobResult::Failed {
                err: JobError::Fs(e),
                ..
            } if e.kind != FsErrorKind::NotFound => {
                self.show_message(
                    MessageLevel::Error,
                    keys::MSG_SAVE_CHECK_FAILED,
                    &[("path", &shown), ("reason", &fs_reason(&e))],
                );
            }
            JobResult::Failed {
                err: JobError::Cancelled,
                ..
            } => self.message(MessageLevel::Info, keys::INFO_SAVE_ABORTED, &shown),
            _ => {
                let Some((path, expected)) = self.docs.documents.get(&doc_id).and_then(|d| {
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

    pub(crate) fn finish_save(
        &mut self,
        job: &SaveCtx,
        path: &FsPath,
        cont: Option<Continuation>,
        r: JobResult,
    ) {
        let SaveCtx {
            doc: doc_id,
            at,
            revision,
            expected,
            fmt,
            ..
        } = *job;
        match r {
            JobResult::Saved { report, .. } => {
                let report_method = report.method;
                // 途中まで読んだ標準入力の文書 (`LoadIncomplete`) は、名前を付けて保存が成功したら理由を外す (保存した
                // ファイルの全体と一致し、読まなかった後半を失う誤解が無い。03 §9.2.2 の「`LoadIncomplete` の外し方」)。
                // 完了の文言は `info.save.partial_saved` (`{size}` は開いた量)
                let partial = self.partial_size(doc_id);
                // 名前を付けて保存で別のパスへ書いた (同じファイルの判定の識別を直す、03 §12.7.6 の 2)
                let path_changed = self
                    .docs
                    .documents
                    .get(&doc_id)
                    .is_some_and(|d| d.path.as_ref() != Some(path));
                if let Some(doc) = self.docs.documents.get_mut(&doc_id) {
                    if partial.is_some() {
                        doc.read_only
                            .remove(crate::document::ReadOnlyReason::LoadIncomplete);
                    }
                    // `file.set_encoding`: 保存が成功したので新しい書式に確定する。違う文字コードで書いたら、往復の検査結果
                    // (旧文字コードのもの) と保持方式の記録を捨てる (状態も対象外に戻る。書いた内容を再エンコードすれば
                    // 同じバイト列になる、03 §8.8)。同じ文字コードなら、表・状態・往復の要約を残す (記録を使って書いたので、
                    // 記録は有効なまま。v0.3 までの「要約を空にする」は違う文字コードへ変えたときだけ、03 §8.7.1)
                    // 違う文字コードで書いたら、本文のエスケープ文字の由来の文字コードも捨てる (`None`。この後に Undo で
                    // エスケープ文字が戻っても元のバイトに戻さない、03 §8.3.1)。同じ文字コードなら変えない
                    if let Some(f) = fmt {
                        if f.encoding != doc.format.encoding {
                            doc.format.roundtrip = Default::default();
                            doc.clear_preserved();
                            doc.forget_escape_encoding();
                        }
                        doc.format.encoding = f.encoding;
                        doc.format.has_bom = f.bom;
                    }
                    // 名前を付けて保存 (別のパスへ書いた) では、元のファイルに由来する読み取り専用の理由
                    // (属性・書込権限なし) を外す。書き込めた保存先の文書は編集できる (FR-116 AC-4、03 §9.6)。
                    // `--readonly` と内容に由来する理由 (不正バイト・バイナリ・長大行) は残る
                    if doc.path.as_ref() != Some(path) {
                        doc.attribute_released = false;
                        doc.read_only
                            .remove(crate::document::ReadOnlyReason::FileAttribute);
                        doc.read_only
                            .remove(crate::document::ReadOnlyReason::NoWritePermission);
                    }
                    // 名前を付けて保存のパスは、書けてから文書のパスにする (03 §9.3 手順 7)
                    doc.path = Some(path.clone());
                    doc.finish_save(at, Some(report.new_stamp));
                    // 保存した: もう新しいファイルの文書ではない (設定が届いても形を決め直さない、07 §3.6 の LAYER-4)
                    doc.new_file = false;
                }
                self.push_event(EditorEvent::DocumentSaved {
                    doc: doc_id,
                    revision,
                    path: path.display(),
                });
                // 名前を付けて保存でパスが変わった: 明示の指定が無ければ言語を判定し直し、文書ごとの値を決め直す
                // (07 §9.6 の LANG-7、§3.6 の LAYER-3)
                if path_changed {
                    self.redetect_language(doc_id);
                    // `.editorconfig` も新しいパスで読み直す (07 §10.3 の EC-1。名前が変わると当たるセクションも変わる)
                    self.editorconfig_on_path_changed(doc_id);
                }
                // 直接上書きで書いたことは 1 回示す (03 §9.3 手順 7)。つながりを承認した保存の完了は、`info.save.done` に
                // 代えて `info.save.escape_merged` にする (03 §8.3.1 の「つながったまま保存した後」)。直接上書きでは
                // `info.save.in_place` の後に出す (完了の要点を最後に残す)
                let shown = path.display();
                let count = job.merged.map(|c| c.to_string()).unwrap_or_default();
                let mut done: Vec<(&str, Vec<(&str, &str)>)> = Vec::new();
                if report_method == SaveMethod::InPlace {
                    done.push((keys::INFO_SAVE_IN_PLACE, vec![("path", &shown)]));
                }
                match job.merged {
                    Some(_) => done.push((
                        keys::INFO_SAVE_ESCAPE_MERGED,
                        vec![("path", &shown), ("count", &count)],
                    )),
                    None if report_method != SaveMethod::InPlace && partial.is_none() => {
                        done.push((keys::INFO_SAVE_DONE, vec![("path", &shown)]));
                    }
                    None => {}
                }
                // 途中までの文書は、つながりの知らせ (`info.save.escape_merged`) を出した後でも、印を外したことを最後に示す
                if let Some(size) = partial.as_deref() {
                    done.push((
                        keys::INFO_SAVE_PARTIAL_SAVED,
                        vec![("path", &shown), ("size", size)],
                    ));
                }
                // 保存時の変換の知らせは、最後に出す完了のメッセージの後ろに 1 つだけ付ける (04 §8.16、03 §9.3.1 の SVT-5 /
                // SVT-6)。連鎖はここで終わるので印を外す
                let suffix = self.take_save_transform_suffix(doc_id);
                let last = done.len().saturating_sub(1);
                // 積んだ要求の再実行で、前の保存の警告・エラーが表示中なら、完了は背景の知らせにして置き換えない
                // (03 §9.7 の「深さ 1 の待ち行列の細目」)
                let quiet = self.io.quiet_save_done.remove(&doc_id);
                for (i, (key, args)) in done.iter().enumerate() {
                    let s = if i == last { suffix.as_deref() } else { None };
                    if quiet {
                        self.show_background_message_with_suffix(MessageLevel::Info, key, args, s);
                    } else {
                        self.show_message_with_suffix(MessageLevel::Info, key, args, s);
                    }
                }
                self.show_save_warning(path, &report.warnings);
                if path_changed {
                    self.after_save_as(doc_id, path);
                    self.watch_forget_fs(doc_id);
                }
                // 保存した: 監視があることを確かめる (パスが変われば古いディレクトリの数を減らす。失効していれば張り直す。
                // 03 §9.5.1 の WATCH-1)
                self.watch_after_save(doc_id);
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
                        Continuation::Reopen(..) => "unsaved_reopen",
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
                escapes_unmappable,
                escapes_restored,
                encoding,
            } if self.docs.documents.contains_key(&doc_id) && !recorded.is_empty() => {
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
                    // 記録した先頭 100 件ではなく、全件で数えたエスケープ文字の数で判定する (03 §8.6 の「E3 の選択肢」)。
                    // 元のバイトに戻したエスケープ文字があれば、UTF-8 では表現不能になるので「UTF-8 で保存」も出さない
                    // (03 §8.3.1 の「E3 との関係」)
                    EscapesInSave {
                        unmappable: escapes_unmappable,
                        restored: escapes_restored > 0,
                        // 本文の「最初」はエスケープ文字の最初 (記録した先頭 100 件に無ければ表現不能の最初)
                        first: recorded
                            .iter()
                            .find(|u| is_escape_char(u.ch))
                            .map(|u| u.char_index),
                    },
                );
            }
            JobResult::Unmappable { total, .. } => {
                self.show_message(
                    MessageLevel::Error,
                    keys::MSG_SAVE_FAILED,
                    &[
                        ("path", &path.display()),
                        ("reason", &format!("unmappable ({total})")),
                    ],
                );
            }
            // エスケープ文字を戻したバイトが隣とつながる (03 §8.3.1): 何も書いていない。書く前の確認
            // (`ui.confirm.save.escape_merge`) を出す。後続 (終了・置き換え・開き直し) は「このまま保存」で保ち、
            // キャンセルで捨てる (03 §9.7 (e))
            JobResult::EscapeMerge {
                total,
                recorded,
                digest,
                changed_chars,
                changed_lines,
                to_eof,
            } if self.docs.documents.contains_key(&doc_id) && !recorded.is_empty() => {
                self.open_escape_merge(
                    super::escape_merge::EscapeMergeAsk {
                        doc: doc_id,
                        path: path.clone(),
                        expected,
                        cont,
                        strategy: job.strategy,
                        fmt,
                        base: revision,
                        approval: crate::jobs::EscapeMergeApproval {
                            total,
                            recorded,
                            digest,
                        },
                    },
                    super::escape_merge::MergeExtent {
                        chars: changed_chars,
                        lines: changed_lines,
                        to_eof,
                    },
                );
            }
            // 閉じた文書 (確認を出す先が無い): 保存の失敗として示す (退避先への書込は結果の処理が行う)
            JobResult::EscapeMerge { total, .. } => {
                self.show_message(
                    MessageLevel::Error,
                    keys::MSG_SAVE_FAILED,
                    &[
                        ("path", &path.display()),
                        ("reason", &format!("escape merge ({total})")),
                    ],
                );
            }
            // 原子的置換ができない・ハードリンク・バックアップを作れない: 確認を出す (承認の後は直接上書き、03 §9.3 手順 7)
            JobResult::Failed {
                err: JobError::Fs(ref e),
                ..
            } if fallback_kind(&e.kind, &e.detail).is_some()
                && self.docs.documents.contains_key(&doc_id) =>
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
            } if self.docs.documents.contains_key(&doc_id) => {
                let current = current.map(|c| *c);
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
                    JobError::Fs(
                        ref e @ FsError {
                            kind: FsErrorKind::InPlaceFailed { ref backup },
                            ..
                        },
                    ),
                ..
            } => {
                let ask = self.save_failed_ask(doc_id, path, expected, fmt, cont);
                // 直接上書きの途中の失敗: 元のファイルは壊れている可能性がある。バックアップの場所を必ず示す (05 §4.4 手順 8)
                let b = backup.as_ref().map(FsPath::display).unwrap_or_default();
                let key = save_error_key(e);
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
                    && self
                        .docs
                        .documents
                        .get(&doc_id)
                        .is_some_and(|d| d.path.as_ref() == Some(path))
                {
                    // 読み取り専用にすると変換を Undo で戻せなくなるので、先に戻す (03 §9.3.1 の SVT-8 の例外)
                    self.revert_save_transform_before_read_only(doc_id);
                    if let Some(doc) = self.docs.documents.get_mut(&doc_id) {
                        doc.read_only
                            .insert(crate::document::ReadOnlyReason::NoWritePermission);
                    }
                    self.mark_dirty_render();
                }
                let ask = self.save_failed_ask(doc_id, path, expected, fmt, cont);
                // 照合の失敗 (Conflict) は別の経路 (外部変更の確認)。取消しは確認なしで示す
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
                let (level, key) = save_failure(&err);
                let backup = match &err_backup {
                    Some(b) => b.display(),
                    None => String::new(),
                };
                let args = [
                    ("path", path.display()),
                    ("reason", reason.clone()),
                    ("dir", dir),
                    ("backup", backup),
                    ("target", link_target.clone()),
                ];
                let args: Vec<(&str, &str)> = args.iter().map(|(k, v)| (*k, v.as_str())).collect();
                match ask {
                    // リンク先が無いリンク: 失敗の選択ではなく、リンク先に作るかを尋ねる (v0.2)
                    Some(ask) if !link_target.is_empty() => {
                        self.open_dangling_link(ask, link_target)
                    }
                    Some(ask) => self.report_save_failure(Some(ask), key, &args, retry),
                    // 取消し (`info.save.aborted`) など確認を出さない失敗も、変換が本文に残っていれば付記する (SVT-8)
                    None => {
                        let kept = self.save_transform_kept_suffix(doc_id);
                        self.show_message_with_suffix(level, key, &args, kept.as_deref());
                    }
                }
            }
            _ => {}
        }
    }
}

/// `Esc` で取り消すジョブの選び方 (04 §7.5 の保存の連鎖の段で共通): 表示中の文書 `active` のジョブを先に、無ければ最後に
/// 投入したもの (`JobId` の大きいもの)。
fn active_or_latest(
    jobs: &[(eve_view::JobId, DocumentId)],
    active: Option<DocumentId>,
) -> Option<(eve_view::JobId, DocumentId)> {
    jobs.iter()
        .find(|(_, d)| Some(*d) == active)
        .or_else(|| jobs.iter().max_by_key(|(j, _)| j.0))
        .copied()
}

pub(crate) fn fallback_kind(
    kind: &FsErrorKind,
    detail: &Option<FsErrorDetail>,
) -> Option<FallbackKind> {
    match (kind, detail) {
        (FsErrorKind::AtomicUnsupported, Some(FsErrorDetail::Hardlink { .. })) => {
            Some(FallbackKind::Hardlink)
        }
        (FsErrorKind::AtomicUnsupported, _) => Some(FallbackKind::AtomicUnsupported),
        (FsErrorKind::BackupFailed, _) => Some(FallbackKind::NoBackup),
        _ => None,
    }
}

impl EditorState {
    /// 保存は成功したが知らせること (`SaveReport.warnings`) のうち、最も重い 1 つを警告として出す (03 §9.3 手順 6、04 §15.2)。
    /// 黙って失わない (02 §4.5。docs/review/v0.3-findings.md の SAF-05)。
    pub(crate) fn show_save_warning(&mut self, path: &FsPath, warnings: &[crate::SaveWarning]) {
        use crate::SaveWarning as W;
        let rank = |w: &W| match w {
            // 一時ファイルの残りは次の保存のハードリンクの確認の原因になるので、隠さない (docs/review/v0.3-findings.md の SAF-40)
            W::TempNotRemoved { .. } => 0,
            W::FsyncFailed => 1,
            W::ChangedAfterSave => 2,
            W::BackupNotRemoved { .. } => 3,
            W::PermissionNarrowed => 4,
            W::AttrsNotPreserved { .. } => 5,
            W::InflightFlagNotWritten => 6,
        };
        let Some(w) = warnings.iter().min_by_key(|w| rank(w)) else {
            return;
        };
        let p = path.display();
        match w {
            W::FsyncFailed => self.show_message(
                MessageLevel::Warn,
                keys::MSG_SAVE_FSYNC_FAILED,
                &[("path", &p)],
            ),
            W::ChangedAfterSave => {
                self.show_message(
                    MessageLevel::Warn,
                    keys::MSG_SAVE_CHANGED_AFTER_SAVE,
                    &[("path", &p)],
                );
            }
            W::BackupNotRemoved { path: backup } => self.show_message(
                MessageLevel::Warn,
                keys::MSG_SAVE_BACKUP_NOT_REMOVED,
                &[("path", &p), ("backup", &backup.display())],
            ),
            W::TempNotRemoved { path: tmp } => self.show_message(
                MessageLevel::Warn,
                keys::MSG_SAVE_TMP_NOT_REMOVED,
                &[("path", &p), ("tmp", &tmp.display())],
            ),
            W::PermissionNarrowed => {
                self.show_message(
                    MessageLevel::Warn,
                    keys::MSG_SAVE_PERMISSION_NARROWED,
                    &[("path", &p)],
                );
            }
            W::AttrsNotPreserved { what } => {
                let reason = what
                    .iter()
                    .map(|k| match k {
                        crate::AttrKind::FileAttributes => "attributes",
                        crate::AttrKind::Security => "security",
                        crate::AttrKind::StreamsXattr => "streams/xattr",
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                self.show_message(
                    MessageLevel::Warn,
                    keys::MSG_SAVE_ATTRIBUTES_NOT_COPIED,
                    &[("path", &p), ("reason", &reason)],
                );
            }
            W::InflightFlagNotWritten => self.show_message(
                MessageLevel::Warn,
                keys::MSG_SAVE_INFLIGHT_FLAG_NOT_WRITTEN,
                &[("path", &p)],
            ),
        }
    }
}

// ---------- コマンド ----------
