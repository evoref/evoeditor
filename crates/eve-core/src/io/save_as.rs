//! 名前を付けて保存の保存先の照合 (`CheckTarget`、docs/03 §9.7 (a)。SAF-28、§12.7.6 の 2)。
//!
//! 入力したパスが今の文書のパスと構文的に同じでなければ、`JobKind::Metadata { purpose: SaveAsTarget }` を投入して保存先の
//! 状態と canonical のパスを調べ、(a) の表の順に扱う: 1 今の文書と同じファイル → 通常の上書き保存、1b ほかの開いている
//! 文書と同じファイル → 拒んで名前の入力に戻る、2 無い → `Expect::Absent` で保存、3 ダングリングのリンク → エラー、4 ファイル
//! でない・パスが不正 → エラー、5 読み取り専用属性 → 拒む、6 既存 → 上書きの確認、7 調べられない → エラー。エラーはどれも
//! 名前の入力に戻る (入力した名前は残す)。
//!
//! 不変条件 (データを失わないため):
//! - 既存のファイルには、上書きの確認で「上書き」を選んだときだけ書く。そのとき照合で得た Stamp を期待値にするので、確認の
//!   間に変わっていれば書かずに `Conflict` になる。
//! - ほかの開いている文書と同じファイルには書かない (2 つの文書が 1 つのファイルを指すと、片方の保存がもう片方の外部変更
//!   になり、古い本文で上書きし得る)。
//! - 保存先の識別は、そのパスへの保存が成功したときだけ文書に写す (失敗・取消しでは文書の canonical を変えない)。
//!
//! spec: 03 §9.7, 03 §12.7.6

use eve_view::{DocumentId, MessageLevel};

use crate::command::CommandResult;
use crate::editor::state::EditorState;
use crate::io::error_keys::fs_reason;
use crate::io::fs::{Expect, FileKind, FileStamp, FsError, FsErrorDetail, FsErrorKind};
use crate::io::fspath::FsPath;
use crate::io::pending::{NewFormat, Pending};
use crate::io::same_file::FileIdentity;
use crate::jobs::{JobError, JobKind, JobResult, MetaPurpose};
use crate::ui_model::keys;
use crate::ui_model::overlay::{Continuation, PromptKind};

/// 名前を付けて保存の照合と、その後の上書きの確認に持ち回る材料。
#[derive(Clone, Debug)]
pub(crate) struct SaveAsAsk {
    /// 保存する文書 (名前の入力を開いた時点の文書、03 §9.8)。
    pub doc: DocumentId,
    /// 入力したパス (正規化したもの)。
    pub path: FsPath,
    /// 入力したままの名前 (名前の入力に戻るときに残す)。
    pub raw: String,
    /// 保存が成功したときに続ける操作。
    pub cont: Option<Continuation>,
    /// `file.set_encoding` の新しい書式。
    pub fmt: Option<NewFormat>,
    /// 上書きの確認で使う、照合で得た保存先の状態と識別 (照合の前は既定値)。
    pub stamp: Option<FileStamp>,
    pub target: FileIdentity,
}

impl EditorState {
    /// 保存先の照合のジョブを投入する (03 §9.7 の `CheckTarget`)。呼び側は、文書が保存できる状態 (読込済み・保存中でない)
    /// で、入力したパスが今の文書のパスと構文的に違うことを確かめてから呼ぶ。
    pub(crate) fn start_save_as_check(&mut self, ask: SaveAsAsk) -> CommandResult {
        let revision = self.docs.documents.get(&ask.doc).map_or(
            eve_view::Revision::INITIAL,
            crate::document::Document::revision,
        );
        // 今の文書も含める (今の文書の識別が古いと、表の 1 に当たって別のファイルへ書かずに済ませてしまう)
        let known_ids = self.known_file_ids(None);
        let job = self.jobs.spawn(
            Some(ask.doc),
            revision,
            JobKind::Metadata {
                path: ask.path.clone(),
                purpose: MetaPurpose::SaveAsTarget,
                known_ids,
            },
        );
        self.io
            .pending
            .insert(job, Pending::SaveAsTarget(Box::new(ask)));
        CommandResult::Pending(job)
    }

    /// 保存先の照合の結果 (03 §9.7 (a) の表)。文書が閉じられていれば何もしない。ほかのオーバーレイか検索バーが出ていれば、
    /// 閉じるまで保留する (03 §12.7.6 の細目 (7)。名前の入力・上書きの確認で、利用者が開いた入力を黙って置き換えない)。
    /// 保留した結果は `flush_held_save_as` が出した順に扱う。
    pub(crate) fn finish_save_as_target(&mut self, ask: SaveAsAsk, r: JobResult) {
        if self.document(ask.doc).is_none() {
            return;
        }
        if self.auto_switch_blocked() || !self.io.held_save_as.is_empty() {
            self.io.held_save_as.push_back((Box::new(ask), r));
            return;
        }
        self.apply_save_as_target(ask, r);
    }

    /// 保留した照合の結果を、オーバーレイ・検索バーが閉じている間、届いた順に扱う (`flush_open_switches` から呼ぶ)。
    pub(crate) fn flush_held_save_as(&mut self) {
        while !self.auto_switch_blocked() {
            let Some((ask, r)) = self.io.held_save_as.pop_front() else {
                return;
            };
            if self.document(ask.doc).is_some() {
                self.apply_save_as_target(*ask, r);
            }
        }
    }

    /// 照合中 (結果を待つ、または保留した) の名前を付けて保存の、(文書, 保存先)。`doc` を指定すればその文書のものだけ。
    pub(crate) fn save_as_checking(&self, doc: Option<DocumentId>) -> Option<(DocumentId, FsPath)> {
        let held = self.io.held_save_as.iter().rev().map(|(a, _)| &**a);
        let pending = self.io.pending.values().rev().filter_map(|p| match p {
            Pending::SaveAsTarget(a) => Some(&**a),
            _ => None,
        });
        held.chain(pending)
            .find(|a| doc.is_none_or(|d| d == a.doc))
            .map(|a| (a.doc, a.path.clone()))
    }

    /// 照合の間の `app.quit`・`file.open`・`file.new` (03 §9.7 の全状態の遷移表の `CheckTarget` の行): 照合中なら
    /// `info.save.in_progress` を示して `true` (呼び側は何もしない)。
    pub(crate) fn ignore_while_save_as_check(&mut self) -> bool {
        let Some((_, path)) = self.save_as_checking(None) else {
            return false;
        };
        self.message(
            MessageLevel::Info,
            keys::INFO_SAVE_IN_PROGRESS,
            &path.display(),
        );
        true
    }

    fn apply_save_as_target(&mut self, mut ask: SaveAsAsk, r: JobResult) {
        let shown = ask.path.display();
        match r {
            JobResult::Meta {
                meta,
                stamp,
                canonical,
            } => {
                ask.target = FileIdentity {
                    file_id: meta.file_id,
                    file_id_unique: meta.file_id_unique,
                    canonical: canonical.or_else(|| Some(ask.path.clone())),
                };
                if self.save_as_same_or_other(&ask) {
                    return;
                }
                // 4: ファイルでない (フォルダ・デバイス等)
                if meta.kind != FileKind::File {
                    self.save_as_back(&ask, keys::MSG_SAVE_AS_NOT_A_FILE, &[("path", &shown)]);
                    return;
                }
                // 5: 読み取り専用属性のファイルは、上書きの確認の前に拒む (FR-116)
                if meta.readonly {
                    self.save_as_back(
                        &ask,
                        keys::MSG_SAVE_AS_TARGET_READ_ONLY,
                        &[("path", &shown)],
                    );
                    return;
                }
                // 6: 既存のファイル: 上書きを確かめる (既定は別の名前を入力)
                let link = meta
                    .link
                    .as_ref()
                    .and_then(|l| l.target.as_ref())
                    .map(FsPath::display);
                ask.stamp = Some(stamp);
                self.open_save_as_overwrite(ask, link);
            }
            JobResult::TargetAbsent { err, canonical } => {
                ask.target = FileIdentity {
                    file_id: None,
                    file_id_unique: false,
                    canonical: canonical.or_else(|| Some(ask.path.clone())),
                };
                if self.save_as_same_or_other(&ask) {
                    return;
                }
                // 3: リンク先が無いシンボリックリンク: 名前の入力で選んだリンクには作らない (03 §9.3 の「シンボリックリンク」)
                if let Some(FsErrorDetail::DanglingLink { target }) = &err.detail {
                    let target = target.display();
                    self.save_as_back(
                        &ask,
                        keys::MSG_SAVE_DANGLING_LINK,
                        &[("path", &shown), ("target", &target)],
                    );
                    return;
                }
                // 2: 無い: 書くまでに他者が作れば `Conflict` (`ui.confirm.save.conflict_created`) になる。親フォルダが
                // 無ければ保存の失敗 (03 §9.7 (e) の `ParentMissing`) で示す (フォルダは作らない)
                self.save_as_write(ask, Expect::Absent);
            }
            JobResult::Failed {
                err: JobError::Cancelled,
            } => {}
            JobResult::Failed {
                err: JobError::Fs(e),
            } => self.save_as_failed(&ask, &e),
            _ => self.save_as_back(
                &ask,
                keys::MSG_SAVE_FAILED,
                &[("path", &shown), ("reason", "metadata")],
            ),
        }
    }

    /// (a) の 1 と 1b。当たれば扱いを済ませて `true`。
    fn save_as_same_or_other(&mut self, ask: &SaveAsAsk) -> bool {
        let Some(doc) = self.document(ask.doc) else {
            return true;
        };
        // 1: 今の文書と同じファイル (別名・リンク・ハードリンク・大文字小文字の違い): 通常の上書き保存として扱う
        if doc.file_identity().same_file(&ask.target) {
            let raw = ask.raw.clone();
            self.save_as_same_file(ask.doc, &raw, ask.cont.clone(), ask.fmt);
            return true;
        }
        // 1b: ほかの開いている文書 (読込中・確認待ちを含む) と同じファイル: 書かずに名前の入力に戻る。リンクでは名前が
        // 違うので、相手の文書のパスも示す
        if let Some(other) = self.document_same_as(&ask.target, Some(ask.doc)) {
            let shown = ask.path.display();
            let other = self.doc_label(other);
            self.save_as_back(
                ask,
                keys::MSG_SAVE_AS_OPEN_IN_OTHER,
                &[("path", &shown), ("other", &other)],
            );
            return true;
        }
        false
    }

    /// (a) の 1: 入力した保存先が今の文書と同じファイル。今の文書のパスへの上書き保存にする。読み取り専用の文書は同じ
    /// ファイルへ書かない (元のバイト列を損なうことがある、03 §9.3 手順 1)。
    pub(crate) fn save_as_same_file(
        &mut self,
        doc_id: DocumentId,
        raw: &str,
        cont: Option<Continuation>,
        fmt: Option<NewFormat>,
    ) -> CommandResult {
        let Some(doc) = self.document(doc_id) else {
            return CommandResult::Rejected(crate::command::RejectReason::NoTarget);
        };
        let (Some(path), editable) = (doc.path.clone(), doc.is_editable()) else {
            return CommandResult::Rejected(crate::command::RejectReason::NoTarget);
        };
        let expected = doc.disk_stamp.map_or(Expect::Absent, Expect::Stamp);
        if !editable {
            self.message(
                MessageLevel::Warn,
                keys::MSG_SAVE_AS_SAME_FILE_READ_ONLY,
                &path.display(),
            );
            self.reopen_save_as_prompt(doc_id, raw, cont, fmt);
            return CommandResult::Rejected(crate::command::RejectReason::ReadOnly);
        }
        // 未編集なら §9.3 手順 0 (ディスクも変わっていなければ書かない)。新しい書式を書く `file.set_encoding` の連鎖と、
        // 後続を伴う保存は手順 0 を飛ばして書く
        if !doc.is_dirty() && fmt.is_none() && cont.is_none() {
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
        self.start_save_fmt(doc_id, path, expected, cont, fmt)
    }

    /// 開いている文書のうち、`target` と同じファイルのもの (`except` を除く。03 §12.7.6 の 0 の比べ方)。開いている文書の
    /// 集合の中で同じファイルを指す文書は 1 つだけ (二重オープンの不変条件) なので、見つかれば 1 つ。
    pub(crate) fn document_same_as(
        &self,
        target: &FileIdentity,
        except: Option<DocumentId>,
    ) -> Option<DocumentId> {
        self.docs
            .documents
            .iter()
            .find(|(id, d)| Some(**id) != except && d.file_identity().same_file(target))
            .map(|(id, _)| *id)
    }

    /// (a) の 7 と 4 (パスが不正): metadata の失敗を示して名前の入力に戻る。
    fn save_as_failed(&mut self, ask: &SaveAsAsk, e: &FsError) {
        let shown = ask.path.display();
        let reason = fs_reason(e);
        let key = match e.kind {
            FsErrorKind::PermissionDenied => keys::MSG_SAVE_PERMISSION_DENIED,
            FsErrorKind::Busy => keys::MSG_SAVE_BUSY,
            FsErrorKind::InvalidPath if matches!(e.detail, Some(FsErrorDetail::NotAFile)) => {
                keys::MSG_SAVE_AS_NOT_A_FILE
            }
            FsErrorKind::InvalidPath => keys::MSG_SAVE_INVALID_PATH,
            _ => keys::MSG_SAVE_FAILED,
        };
        self.save_as_back(ask, key, &[("path", &shown), ("reason", &reason)]);
    }

    /// エラーを示して、名前の入力に戻る (入力した名前は残す。03 §9.7 の `AskName`)。
    fn save_as_back(&mut self, ask: &SaveAsAsk, key: &str, args: &[(&str, &str)]) {
        let level = if key == keys::MSG_SAVE_AS_OPEN_IN_OTHER
            || key == keys::MSG_SAVE_AS_TARGET_READ_ONLY
        {
            MessageLevel::Warn
        } else {
            MessageLevel::Error
        };
        self.show_message(level, key, args);
        self.reopen_save_as_prompt(ask.doc, &ask.raw, ask.cont.clone(), ask.fmt);
    }

    /// 名前の入力を `raw` を入れて開き直す。後続 (`cont`) と新しい書式 (`fmt`) は引き継ぐ。
    pub(crate) fn reopen_save_as_prompt(
        &mut self,
        doc: DocumentId,
        raw: &str,
        cont: Option<Continuation>,
        fmt: Option<NewFormat>,
    ) {
        let kind = match fmt {
            Some(fmt) => PromptKind::SaveAsFormat { doc, fmt },
            None => PromptKind::SaveAs { doc, then: cont },
        };
        self.open_prompt(kind, raw.to_owned());
    }

    /// 保存先へ書く (a の 2 と 6)。成功したときに文書の canonical を替えるため、保存先の識別を覚えておく。
    fn save_as_write(&mut self, ask: SaveAsAsk, expected: Expect) -> CommandResult {
        let SaveAsAsk {
            doc,
            path,
            cont,
            fmt,
            target,
            ..
        } = ask;
        self.io.save_as_targets.insert(doc, (path.clone(), target));
        self.start_save_fmt(doc, path, expected, cont, fmt)
    }

    /// (a) の 6: 上書きの確認 (`ui.confirm.save.overwrite`) を出す。既定は「別の名前を入力」(04 §15.2)。
    fn open_save_as_overwrite(&mut self, ask: SaveAsAsk, link: Option<String>) {
        use crate::ui_model::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
        let details = link
            .map(|target| {
                vec![self.catalog().render(
                    keys::UI_CONFIRM_SAVE_OVERWRITE_LINK,
                    &[("target", &target)],
                    &|_| None,
                )]
            })
            .unwrap_or_default();
        self.open_confirm(ConfirmState {
            name: "save.overwrite",
            args: vec![("path".to_owned(), ask.path.display())],
            choices: vec![
                ChoiceDef {
                    id: "overwrite",
                    accel: Some('o'),
                    destructive: true,
                },
                ChoiceDef {
                    id: "rename",
                    accel: Some('r'),
                    destructive: false,
                },
            ],
            details,
            default: 1,
            focused: 1,
            message: None,
            action: ConfirmAction::SaveAsOverwrite(Box::new(ask)),
        });
    }

    /// 上書きの確認の選択。「上書き」は照合で得た Stamp を期待値にして書く (確認の間に変わっていれば `Conflict`)。
    /// 「別の名前を入力」とキャンセル (`Esc`) は名前の入力に戻る (04 §7.5)。
    pub(crate) fn choose_save_as_overwrite(
        &mut self,
        ask: SaveAsAsk,
        choice: Option<&str>,
    ) -> CommandResult {
        match (choice, ask.stamp) {
            (Some("overwrite"), Some(stamp)) => self.save_as_write(ask, Expect::Stamp(stamp)),
            _ => {
                self.reopen_save_as_prompt(ask.doc, &ask.raw, ask.cont, ask.fmt);
                CommandResult::Done
            }
        }
    }

    /// 照合のジョブを待つ間の `Esc` (03 §9.7 の全状態の遷移表の `CheckTarget` の行): 結果を捨てて名前の入力に戻る。
    /// 照合を待っていなければ `false`。
    pub(crate) fn cancel_save_as_check(&mut self) -> bool {
        // 今の文書の照合を、無ければ最後に投入した照合を取り消す (03 §12.7.6 の細目 (9))
        let active = self.active_document();
        let checks: Vec<(eve_view::JobId, DocumentId)> = self
            .io
            .pending
            .iter()
            .filter_map(|(job, p)| match p {
                Pending::SaveAsTarget(ask) => Some((*job, ask.doc)),
                _ => None,
            })
            .collect();
        let Some(job) = checks
            .iter()
            .find(|(_, d)| Some(*d) == active)
            .or_else(|| checks.iter().max_by_key(|(j, _)| j.0))
            .map(|(j, _)| *j)
        else {
            return false;
        };
        let Some(Pending::SaveAsTarget(ask)) = self.io.pending.remove(&job) else {
            return false;
        };
        self.jobs.cancel(job);
        self.reopen_save_as_prompt(ask.doc, &ask.raw, ask.cont, ask.fmt);
        true
    }

    /// 保存の成功で文書のパスが `path` になった: 文書の canonical を照合で得たものに替え (識別は `disk_stamp` で直る)、
    /// もう一度ほかの文書と比べる (照合から保存の完了までの間に、別の文書の `Identity` が届いて同じファイルになっていたら
    /// 知らせる。どちらの文書も閉じない (保存していない編集を捨てない)。03 §12.7.6 の 2)。
    pub(crate) fn after_save_as(&mut self, doc_id: DocumentId, path: &FsPath) {
        let target = match self.io.save_as_targets.remove(&doc_id) {
            Some((p, t)) if p == *path => t,
            // 照合を経ずにパスが変わった (起きないはず): 正規化したパスを canonical にする (古いパスの識別を残さない)
            _ => FileIdentity {
                file_id: None,
                file_id_unique: false,
                canonical: Some(path.clone()),
            },
        };
        let Some(doc) = self.docs.documents.get_mut(&doc_id) else {
            return;
        };
        doc.canonical = target.canonical;
        doc.file_id_unique = target.file_id_unique;
        doc.loading_file_id = None;
        let ident = doc.file_identity();
        if let Some(other) = self.document_same_as(&ident, Some(doc_id)) {
            // 保存はできている。拒んだときの `msg.save_as.open_in_other` と読み違えないよう、別の文言で知らせる
            let other = self.doc_label(other);
            self.show_message(
                MessageLevel::Warn,
                keys::MSG_SAVE_AS_NOW_OPEN_IN_OTHER,
                &[("path", &path.display()), ("other", &other)],
            );
        }
    }

    /// 名前を付けて保存の識別の覚えを、そのパスのものなら消す。
    pub(crate) fn forget_save_as_target(&mut self, doc: DocumentId, path: &FsPath) {
        if self
            .io
            .save_as_targets
            .get(&doc)
            .is_some_and(|(p, _)| p == path)
        {
            self.io.save_as_targets.remove(&doc);
        }
    }

    /// 文書の表示用の名前 (パス、無題なら `無題-{n}`)。
    fn doc_label(&self, doc: DocumentId) -> String {
        self.document(doc)
            .map(|d| {
                d.path
                    .as_ref()
                    .map_or_else(|| self.untitled_label(d), FsPath::display)
            })
            .unwrap_or_default()
    }
}
