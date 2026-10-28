//! swap-lite の予約と後始末 (docs/03 §10.2、FR-118 / NFR-307)。
//!
//! dirty な文書の全文を、最後の入力から `file.swap.interval_ms` (既定 2 秒) 後に `bg` のジョブで書く。続く書出しは同じ文書で
//! 最短 `file.swap.min_interval_ms` (既定 10 秒) 間隔。止まらずに打ち続けて 2 秒の空きが来なくても、未書出しの変更が
//! `min_interval_ms` 続けば書く。文書が保存された・閉じられた・対象外になったときはスワップを消す。
//!
//! 同じ文書のスワップのジョブは同時に 1 つだけ。書込中のスワップはキャンセルせず、その間の変更は完了後に最新の Snapshot で
//! 1 回だけ書く (書きかけを残さない)。Port が実装していなければ (`FsErrorKind::Unsupported`) そのセッションでは止める。
//! 配置・命名・pid の確認は Port の側 (05 §7.5)。

use std::cmp::{max, min};
use std::collections::BTreeMap;
use std::sync::Arc;

use eve_view::{DocumentId, JobId, Revision};

use super::file::Pending;
use super::state::EditorState;
use crate::document::LoadState;
use crate::time::{Millis, Tick};
use crate::{FsErrorKind, FsPath, JobError, JobKind, JobResult};

/// 失敗した後始末をやり直すまでの間隔。
const DISCARD_RETRY: Millis = Millis(10_000);

#[derive(Default)]
pub(super) struct SwapState {
    docs: BTreeMap<DocumentId, SwapDoc>,
    /// Port がスワップを実装していない。以後このセッションでは何もしない。
    stopped: bool,
}

#[derive(Default)]
struct SwapDoc {
    /// 書くべき Revision と、それに気づいた時刻。
    seen: Option<(Revision, Tick)>,
    /// 書けていない変更が始まった時刻 (連続入力でも `min_interval_ms` で書くため)。
    first_change: Option<Tick>,
    /// 最後に書き終えた Revision。
    written: Option<Revision>,
    /// 書込中のジョブとその Revision。
    inflight: Option<(JobId, Revision)>,
    /// 最後に書き始めた時刻。
    last_start: Option<Tick>,
    /// ディスクにスワップがある (書き終えた、または書込中)。
    on_disk: bool,
    /// スワップを書いたときの文書のパス (消すときの宛先。無題は `None`)。
    path: Option<FsPath>,
    /// 消すジョブ。
    discarding: Option<JobId>,
    /// 消すのに失敗した後、やり直してよい時刻。
    discard_retry_at: Option<Tick>,
}

impl EditorState {
    /// 書いてよい文書なら、その Revision。dirty で読込済み・編集可・上限以下で、秘密ファイルの除外に当たらないもの。
    fn swap_eligible(&self, id: DocumentId) -> Option<Revision> {
        let cfg = &self.config;
        if !cfg.swap_enabled {
            return None;
        }
        let d = self.documents.get(&id)?;
        if !d.is_dirty() || d.load_state != LoadState::Ready || !d.read_only.is_empty() {
            return None;
        }
        if d.len_bytes() as u64 > cfg.swap_max_bytes {
            return None;
        }
        if cfg.swap_exclude_secret_files && d.path.as_ref().is_some_and(crate::is_secret_path) {
            return None;
        }
        Some(d.revision())
    }

    /// 次に書く時刻。気づいていない (`seen` が無い) ときは `None`。
    fn swap_due(&self, e: &SwapDoc) -> Option<Tick> {
        let (_, seen_at) = e.seen?;
        let first = e.first_change?;
        let cfg = &self.config;
        let min_interval = Millis(u64::from(cfg.swap_min_interval_ms));
        let base = min(
            seen_at + Millis(u64::from(cfg.swap_interval_ms)),
            first + min_interval,
        );
        Some(match e.last_start {
            Some(t) => max(base, t + min_interval),
            None => base,
        })
    }

    /// `Editor::tick` から呼ぶ。期限が来た文書のスワップを書くジョブを投入し、要らなくなったスワップを消すジョブを投入する。
    pub(super) fn swap_tick(&mut self) {
        if self.swap.stopped {
            return;
        }
        let mut ids: Vec<DocumentId> = self.documents.keys().copied().collect();
        ids.extend(self.swap.docs.keys().copied());
        ids.sort();
        ids.dedup();
        for id in ids {
            self.swap_step(id);
        }
    }

    fn swap_step(&mut self, id: DocumentId) {
        let now = self.now;
        let eligible = self.swap_eligible(id);
        let mut e = self.swap.docs.remove(&id).unwrap_or_default();
        let busy = e.inflight.is_some() || e.discarding.is_some();
        let Some(rev) = eligible else {
            // 書込中・削除中は完了を待つ (結果の処理の後の tick で見直す)
            if !busy && e.on_disk && e.discard_retry_at.is_none_or(|t| now >= t) {
                let job = self.jobs.spawn(
                    Some(id),
                    Revision(0),
                    JobKind::SwapDiscard {
                        doc: id,
                        path: e.path.clone(),
                    },
                );
                self.pending.insert(job, Pending::SwapDiscard { doc: id });
                e.discarding = Some(job);
            }
            // 書いていなければ記録は要らない
            if e.on_disk {
                self.swap.docs.insert(id, e);
            }
            return;
        };
        // 書込中の Snapshot より後の変更も、気づいた時刻を記録する (完了後に最新で 1 回、期限どおりに書くため)
        let covered = e.inflight.map(|(_, r)| r).or(e.written);
        if covered == Some(rev) {
            e.seen = None;
            e.first_change = None;
        } else if e.seen.map(|(r, _)| r) != Some(rev) {
            e.seen = Some((rev, now));
            e.first_change.get_or_insert(now);
        }
        if !busy
            && self.swap_due(&e).is_some_and(|due| now >= due)
            && let Some(mut snap) = self.rescue_snapshot(id, true)
        {
            snap.sensitive = snap.path.as_ref().is_some_and(crate::is_secret_path);
            let path = snap.path.clone();
            let job = self.jobs.spawn(
                Some(id),
                rev,
                JobKind::Swap {
                    doc: Arc::new(snap),
                },
            );
            self.pending.insert(job, Pending::Swap { doc: id });
            e.inflight = Some((job, rev));
            e.on_disk = true;
            e.path = path;
            e.last_start = Some(now);
            e.seen = None;
            e.first_change = None;
        }
        self.swap.docs.insert(id, e);
    }

    /// 次に `tick` が要る時刻 (スワップを書く・消す時刻)。
    pub(super) fn swap_deadline(&self) -> Option<Tick> {
        if self.swap.stopped {
            return None;
        }
        self.swap
            .docs
            .iter()
            .filter(|(_, e)| e.inflight.is_none() && e.discarding.is_none())
            .filter_map(|(id, e)| {
                if self.swap_eligible(*id).is_some() {
                    self.swap_due(e)
                } else if e.on_disk {
                    Some(e.discard_retry_at.unwrap_or(self.now))
                } else {
                    None
                }
            })
            .min()
    }

    /// ディスクにスワップがある文書 (正常終了のとき、Host が消す。書込中のものを含む)。
    pub(super) fn swap_leftovers(&self) -> Vec<(DocumentId, Option<FsPath>)> {
        self.swap
            .docs
            .iter()
            .filter(|(_, e)| e.on_disk)
            .map(|(id, e)| (*id, e.path.clone()))
            .collect()
    }

    /// スワップを書くジョブの結果。
    pub(super) fn finish_swap(&mut self, doc: DocumentId, r: JobResult) {
        let Some(e) = self.swap.docs.get_mut(&doc) else {
            return;
        };
        let inflight = e.inflight.take();
        match r {
            JobResult::SwapWritten { .. } => {
                if let Some((_, rev)) = inflight {
                    e.written = Some(rev);
                }
            }
            JobResult::Failed {
                err: JobError::Fs(err),
                ..
            } if err.kind == FsErrorKind::Unsupported => self.swap_stop(),
            // 失敗: 書けた Revision は変わらない。次の期限で書き直す (`last_start` が間隔を空ける)
            _ => {}
        }
    }

    /// スワップを消すジョブの結果。
    pub(super) fn finish_swap_discard(&mut self, doc: DocumentId, r: JobResult) {
        let now = self.now;
        let Some(e) = self.swap.docs.get_mut(&doc) else {
            return;
        };
        e.discarding = None;
        match r {
            JobResult::SwapDiscarded { .. } => {
                self.swap.docs.remove(&doc);
            }
            JobResult::Failed {
                err: JobError::Fs(err),
                ..
            } if err.kind == FsErrorKind::Unsupported => self.swap_stop(),
            _ => e.discard_retry_at = Some(now + DISCARD_RETRY),
        }
    }

    fn swap_stop(&mut self) {
        self.swap.stopped = true;
        self.swap.docs.clear();
    }
}
