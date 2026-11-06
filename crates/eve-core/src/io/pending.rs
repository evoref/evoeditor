//! ジョブの表 (`Pending`: 投入したジョブとその結果の宛先) と、ジョブの結果の振り分け (`on_job_result`)
//! (docs/02 §9.3、03 §9。docs/review/v0.3-findings.md の STR-B1 で io/file.rs から分けた)。

use eve_view::{DocumentId, ViewId};

use crate::document::NodeId;
use crate::io::fs::Expect;
use crate::io::fspath::FsPath;
use crate::io::io_jobs::OpenOptions;
use crate::jobs::JobResult;
use eve_encoding::Encoding;

use super::open::*;
use super::unmappable::*;
use crate::editor::state::EditorState;

/// 投入したジョブと、その結果の宛先。
#[derive(Debug)]
pub(crate) enum Pending {
    /// `config.reload` の読込 (07 §6)。
    ConfigReload,
    /// テーマファイルの読込 (07 §8.6)。
    ThemeRead,
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
    /// 外部変更の確認 (03 §9.5)。`explicit` は `file.check_external_changes` (フォーカス復帰でない)。
    ExternalCheck { doc: DocumentId, explicit: bool },
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
    ConvertEol(crate::document::eol_cmd::EolConvert),
    /// すべて置換の ChangeSet を作るジョブ (03 §12.5)。
    ReplaceAll {
        doc: DocumentId,
        revision: eve_view::Revision,
    },
    /// 起動時の退避ディレクトリの走査。
    RecoveryScan,
    /// 前回起動した版の記録 (FR-518)。
    LastVersion,
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
    Leftover(Box<crate::io::leftover_offer::LeftoverAsk>),
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
pub(crate) struct NewFormat {
    pub encoding: Encoding,
    pub bom: bool,
}

impl EditorState {
    pub(crate) fn take_pending_for(&mut self, doc: DocumentId) {
        let ids: Vec<_> = self
            .io
            .pending
            .iter()
            .filter(|(_, p)| match p {
                Pending::Open { doc: d, .. }
                | Pending::CheckUnchanged { doc: d }
                | Pending::ExternalCheck { doc: d, .. } => *d == doc,
                Pending::ReplaceUnmappable { ask, .. } => ask.doc == doc,
                Pending::Search => self.find.search.as_ref().is_some_and(|s| s.doc() == doc),
                Pending::FindNext { doc: d, .. } => *d == doc,
                Pending::ReplaceAll { doc: d, .. } => *d == doc,
                Pending::ConvertEol(c) => c.doc == doc,
                // 保存ジョブは取り消さず、完了を待つ (03 §9.2 の「閉じる手順」。ユーザーが求めた保存を中断しない)
                Pending::Save { .. }
                | Pending::ConfigReload
                | Pending::ThemeRead
                | Pending::RecoveryScan
                | Pending::LastVersion
                | Pending::WriteRecovery { .. }
                | Pending::Swap { .. }
                | Pending::SwapDiscard { .. }
                | Pending::SwapRead { .. }
                | Pending::SwapRemove { .. }
                | Pending::Leftover(_) => false,
            })
            .map(|(id, _)| *id)
            .collect();
        if self.find.search.as_ref().is_some_and(|s| s.doc() == doc) {
            self.find.search = None;
        }
        for id in ids {
            self.ui.continuations.remove(&id);
            self.io.pending.remove(&id);
            self.jobs.cancel(id);
        }
    }

    /// ジョブの結果を受け取る (02 §5.4)。
    pub fn on_job_result(&mut self, r: JobResult) {
        let job = r.job();
        // 読込の進捗は終わりではない。待ちの記録は残して、読込中の文書の進捗だけ更新する
        if let JobResult::Progress { bytes, total, .. } = r {
            if let Some(Pending::Open { doc, .. }) = self.io.pending.get(&job) {
                let pct = bytes
                    .saturating_mul(100)
                    .checked_div(total)
                    .unwrap_or(0)
                    .min(100) as u8;
                let doc = *doc;
                if let Some(d) = self.docs.documents.get_mut(&doc) {
                    d.load_percent = pct;
                }
                self.mark_dirty_render();
            }
            return;
        }
        // 検索は途中経過が複数届く。待ちの記録は終わりまで残す
        if matches!(self.io.pending.get(&job), Some(Pending::Search)) {
            self.on_search_result(r);
            return;
        }
        let cont = self.ui.continuations.remove(&job);
        let Some(pending) = self.io.pending.remove(&job) else {
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
            Pending::ExternalCheck { doc, explicit } => {
                self.finish_external_check(doc, explicit, r);
            }
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
                    self.io.closing_snapshots.remove(&doc);
                    self.mark_snapshot_update(doc);
                }
            }
            Pending::ReplaceUnmappable {
                ask,
                revision,
                retried,
            } => self.finish_replace_unmappable(ask, revision, retried, r),
            Pending::RecoveryScan => self.finish_recovery_scan(r),
            Pending::LastVersion => self.finish_last_version(r),
            Pending::ConfigReload => self.finish_config_reload(r),
            Pending::ThemeRead => self.finish_theme_read(r),
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
                if self.ui.quit_abort.is_some() {
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
}
