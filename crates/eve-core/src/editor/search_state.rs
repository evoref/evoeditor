//! 文書内検索の状態と、次へ・前への移動 (docs/03 §12.2、01 FR-401・FR-404)。
//!
//! 全件走査はバックグラウンドのジョブで行い、届いた一致を保持する。移動は保持した一致から選ぶ。保持に無い (走査中で
//! まだ届いていない、保持の上限を超えた、編集で無効になった) ときは、カーソルから同期検索する (最大 256 KiB)。
//! 編集があると保持は無効になり、件数は「約」表示に落ちる。再検索は呼び出し側 (検索バー) が入力の間を置いて始める。

use eve_text::{ByteOff, ByteRange, LineIdx, Range, Selection, TextRead};
use eve_view::{DocumentId, JobId, MessageLevel, Revision};

use crate::cancel::CancelToken;
use crate::jobs::{JobKind, JobResult};
use crate::search::{SearchQuery, scan_bytes};

use super::file::Pending;
use super::state::EditorState;

/// 保持する一致の上限 (`search.max_matches`、07 §5.5)。
const MAX_MATCHES: usize = 10_000;
/// 数える一致の上限 (`search.count_limit`、07 §5.5)。
const COUNT_LIMIT: u64 = 1_000_000;
/// 同期検索で走査する量 (`SEARCH_SYNC_STEP_BYTES`、03 §12.2)。
const SYNC_STEP_BYTES: usize = 1 << 18;
/// 同期検索で一度に `scan_bytes` へ渡す量 (打ち切りの粒度)。
const SYNC_BATCH_BYTES: usize = 64 << 10;

#[derive(Debug)]
pub(super) struct SearchState {
    doc: DocumentId,
    /// 結果が属する文書の Revision。違えば保持は無効 (`Approx`)。
    revision: Revision,
    query: SearchQuery,
    job: Option<JobId>,
    /// 走査した順に届いた一致 (終わったら開始位置の昇順に整える)。
    matches: Vec<ByteRange>,
    total: u64,
    count_limited: bool,
    /// 保持の上限を超えて、一致の一部しか持っていない。
    truncated: bool,
}

/// 件数の表示の材料 (`SearchCount`、04 §8.4)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchCount {
    /// 検索語が空、または検索していない。
    Idle,
    /// 走査中 (届いた件数)。
    Running { found: u64 },
    /// 全件を数えた。`current` は選択が一致のとき、その番号 (1 始まり)。
    Exact { current: Option<u64>, total: u64 },
    /// 数える上限に達した。
    AtLeast { current: Option<u64>, total: u64 },
    /// 編集で保持が無効になった。目安の件数。
    Approx { total: u64 },
}

/// `search_step` の結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchStep {
    /// 一致へ移った。`wrapped` は文書の端を越えた。
    Moved { wrapped: bool },
    /// 一致が無い。
    NoMatch,
    /// 同期検索の範囲では見つからなかった (全件の走査が終われば見つかる可能性がある)。
    NotYet,
    /// 検索していない・検索語が空。
    NoQuery,
}

pub(super) enum Found {
    Hit(ByteRange, bool),
    None,
    Incomplete,
}

impl SearchState {
    pub(super) fn doc(&self) -> DocumentId {
        self.doc
    }
}

impl EditorState {
    /// 検索を始める (進行中の検索は取り消す)。検索語が空なら検索を止めて状態を消す。対象は今の文書。
    pub fn search_start(&mut self, query: SearchQuery) {
        self.search_cancel_job();
        let Some(doc_id) = self.active_document() else {
            return;
        };
        if query.pattern().is_empty() {
            self.search = None;
            self.mark_dirty_render();
            return;
        }
        let Some(doc) = self.documents.get(&doc_id) else {
            return;
        };
        let revision = doc.revision();
        let snapshot = doc.snapshot();
        let from = self.search_cursor(true).unwrap_or(ByteOff(0));
        let job = self.jobs.spawn(
            Some(doc_id),
            revision,
            JobKind::Search {
                snapshot,
                query: query.clone(),
                from,
                max_matches: MAX_MATCHES,
                count_limit: COUNT_LIMIT,
            },
        );
        self.pending.insert(job, Pending::Search);
        self.search = Some(SearchState {
            doc: doc_id,
            revision,
            query,
            job: Some(job),
            matches: Vec::new(),
            total: 0,
            count_limited: false,
            truncated: false,
        });
        self.mark_dirty_render();
    }

    /// 検索を終える (検索バーを閉じる)。
    pub fn search_stop(&mut self) {
        self.search_cancel_job();
        if self.search.take().is_some() {
            self.mark_dirty_render();
        }
    }

    /// 今の検索語。
    pub fn search_query(&self) -> Option<&SearchQuery> {
        self.search.as_ref().map(|s| &s.query)
    }

    fn search_cancel_job(&mut self) {
        if let Some(old) = self.find_job.take() {
            self.jobs.cancel(old);
            self.pending.remove(&old);
        }
        if let Some(job) = self.search.as_mut().and_then(|s| s.job.take()) {
            self.jobs.cancel(job);
            self.pending.remove(&job);
        }
    }

    /// 選択の端 (次へは終端、前へは始端)。
    fn search_cursor(&self, forward: bool) -> Option<ByteOff> {
        let v = self.view(self.window_view(self.first_window()?)?)?;
        let r = v.selection.primary();
        Some(if forward { r.end() } else { r.start() })
    }

    pub(super) fn search_fresh(&self) -> bool {
        self.search.as_ref().is_some_and(|s| {
            self.documents
                .get(&s.doc)
                .is_some_and(|d| d.revision() == s.revision)
        })
    }

    /// 件数の表示の材料。
    pub fn search_count(&self) -> SearchCount {
        self.search_count_for(None)
    }

    /// `search_count` と同じだが、現在の一致の番号を `probe` (選択していない強調中の一致) から求める。
    pub(super) fn search_count_for(&self, probe: Option<ByteRange>) -> SearchCount {
        let Some(s) = &self.search else {
            return SearchCount::Idle;
        };
        if !self.search_fresh() {
            return SearchCount::Approx { total: s.total };
        }
        if s.job.is_some() {
            return SearchCount::Running {
                found: s.matches.len() as u64,
            };
        }
        let current = self.search_current_index(probe).map(|i| i as u64 + 1);
        if s.count_limited {
            SearchCount::AtLeast {
                current,
                total: s.total,
            }
        } else {
            SearchCount::Exact {
                current,
                total: s.total,
            }
        }
    }

    /// 選択が保持した一致のどれかと一致していれば、その位置。一部しか持っていないときは求めない。
    fn search_current_index(&self, probe: Option<ByteRange>) -> Option<usize> {
        let s = self.search.as_ref()?;
        if s.truncated || s.count_limited {
            return None;
        }
        let (a, b) = match probe {
            Some(r) => (r.start, r.end),
            None => {
                let sel = self
                    .view(self.window_view(self.first_window()?)?)?
                    .selection
                    .primary();
                (sel.start(), sel.end())
            }
        };
        s.matches
            .binary_search_by_key(&a, |m| m.start)
            .ok()
            .filter(|&i| s.matches[i].end == b)
    }

    /// 次の一致 (`forward`) か前の一致へ選択を移す。
    pub fn search_step(&mut self, forward: bool) -> SearchStep {
        let Some(s) = &self.search else {
            return SearchStep::NoQuery;
        };
        let (doc_id, query) = (s.doc, s.query.clone());
        let Some(cursor) = self.search_cursor(forward) else {
            return SearchStep::NoQuery;
        };
        // 編集で無効になっていたら、保持を捨てて走査をやり直す (移動は同期検索で行う)
        let fresh = self.search_fresh();
        if !fresh {
            self.search_start(query.clone());
        }
        // 保持した一致を使えるのは、走査が終わり、上限にも達していないとき (全件を持っているとき) だけ
        let complete = fresh
            && self
                .search
                .as_ref()
                .is_some_and(|s| s.job.is_none() && !s.truncated && !s.count_limited);
        let found = if complete {
            match self.find_retained(cursor, forward) {
                Some(h) => h,
                None => match self.search.as_ref().map(|s| &s.matches[..]) {
                    Some([]) | None => Found::None,
                    // 端を越えて反対側の端へ
                    Some(m) => Found::Hit(if forward { m[0] } else { m[m.len() - 1] }, true),
                },
            }
        } else {
            self.sync_find(doc_id, &query, cursor, forward)
        };
        match found {
            Found::Hit(m, wrapped) => {
                self.search_select(m, wrapped, forward);
                SearchStep::Moved { wrapped }
            }
            Found::None => SearchStep::NoMatch,
            // 同期検索の範囲では見つからなかった: 単発のジョブに任せる。届いたら移る
            Found::Incomplete => {
                self.spawn_find_next(doc_id, query, cursor, forward);
                SearchStep::NotYet
            }
        }
    }

    /// 単発の検索ジョブを投入する (古い単発は取り消す)。
    fn spawn_find_next(
        &mut self,
        doc_id: DocumentId,
        query: SearchQuery,
        cursor: ByteOff,
        forward: bool,
    ) {
        if let Some(old) = self.find_job.take() {
            self.jobs.cancel(old);
            self.pending.remove(&old);
        }
        let Some(doc) = self.documents.get(&doc_id) else {
            return;
        };
        let revision = doc.revision();
        let job = self.jobs.spawn(
            Some(doc_id),
            revision,
            JobKind::FindNext {
                snapshot: doc.snapshot(),
                query,
                from: cursor,
                forward,
            },
        );
        self.pending.insert(
            job,
            Pending::FindNext {
                doc: doc_id,
                revision,
                cursor,
                forward,
            },
        );
        self.find_job = Some(job);
    }

    /// 単発の検索の結果。文書の Revision と選択が投入時のままなら移る (違えば捨てる)。
    pub(super) fn finish_find_next(
        &mut self,
        doc_id: DocumentId,
        revision: Revision,
        cursor: ByteOff,
        forward: bool,
        r: JobResult,
    ) {
        self.find_job = None;
        let JobResult::FindNextDone { found, .. } = r else {
            return;
        };
        let same = self
            .documents
            .get(&doc_id)
            .is_some_and(|d| d.revision() == revision)
            && self.search_cursor(forward) == Some(cursor);
        if !same {
            return;
        }
        if let Some((m, wrapped)) = found {
            self.search_select(m, wrapped, forward);
        }
    }

    /// 保持した一致から、`cursor` より先 (前) の最初の一致。
    fn find_retained(&self, cursor: ByteOff, forward: bool) -> Option<Found> {
        let s = self.search.as_ref()?;
        let m = &s.matches;
        if forward {
            let i = m.partition_point(|r| r.start < cursor);
            m.get(i).map(|r| Found::Hit(*r, false))
        } else {
            let i = m.partition_point(|r| r.end <= cursor);
            i.checked_sub(1).map(|i| Found::Hit(m[i], false))
        }
    }

    /// カーソルから同期検索する (最大 `SYNC_STEP_BYTES`)。文書の端を越えたら反対側から続ける。
    pub(super) fn sync_find(
        &self,
        doc_id: DocumentId,
        query: &SearchQuery,
        cursor: ByteOff,
        forward: bool,
    ) -> Found {
        let Some(m) = query.matcher() else {
            return Found::None;
        };
        let Some(doc) = self.documents.get(&doc_id) else {
            return Found::None;
        };
        let t = doc.text();
        let n = t.len_lines();
        let line = t.byte_to_line(cursor).0.min(n.saturating_sub(1));
        let cancel = CancelToken::new();
        let len = t.len_bytes();
        // 行ごとに畳み込む経路は約 50 MB/s なので、1 MiB を走査すると約 20 ms (256 KiB でも約 5 ms) かかる (BM-404)。量を絞り、
        // 見つからなければ呼び出し側が単発のジョブに回す (`Found::Incomplete`)
        let mut budget = if m.is_chunked() {
            SYNC_STEP_BYTES
        } else {
            SYNC_STEP_BYTES / 16
        };
        // 走査する区間 (行の半開区間, 折り返し済みか)。後ろ向きは区間の終わりから、バッチごとに戻る
        let segs: [(std::ops::Range<usize>, bool); 2] = if forward {
            [(line..n, false), (0..line + 1, true)]
        } else {
            [(0..line + 1, false), (line..n, true)]
        };
        for (seg, wrapped) in segs {
            let (mut lo, mut hi) = (seg.start, seg.end);
            while lo < hi {
                // 次のバッチ: 目標のバイト数 (`SYNC_BATCH_BYTES` か残りの予算) を行の境界に丸める。
                // 1 行ずつ走査すると 1 MiB で約 20 ms かかる (BM-404) ので、まとめて `scan_bytes` (memmem) に渡す。
                // 行ごとの位置の引き直しも避け、バッチごとに 1 回だけ引く
                let want = SYNC_BATCH_BYTES.min(budget);
                let (a, b) = if forward {
                    let target = t.line_to_byte(LineIdx(lo)).0.saturating_add(want);
                    let b = if target >= len {
                        hi
                    } else {
                        (t.byte_to_line(ByteOff(target)).0 + 1).clamp(lo + 1, hi)
                    };
                    (lo, b)
                } else {
                    let end = if hi >= n {
                        len
                    } else {
                        t.line_to_byte(LineIdx(hi)).0
                    };
                    let a = t
                        .byte_to_line(ByteOff(end.saturating_sub(want)))
                        .0
                        .clamp(lo, hi - 1);
                    (a, hi)
                };
                let start = t.line_to_byte(LineIdx(a));
                let end = if b >= n {
                    ByteOff(len)
                } else {
                    t.line_to_byte(LineIdx(b))
                };
                let bytes = end.0 - start.0;
                if bytes > budget {
                    return Found::Incomplete;
                }
                budget = budget.saturating_sub(bytes.max(1));
                let mut hit: Option<ByteRange> = None;
                scan_bytes(t, &m, ByteRange::new(start, end), &cancel, &mut |r| {
                    let ok = if forward {
                        // 折り返した後は、カーソルの行の全体を対象にする
                        wrapped || r.start >= cursor
                    } else {
                        wrapped || r.end <= cursor
                    };
                    if ok {
                        hit = Some(r);
                        // 前向きは最初の一致で止める。後ろ向きはバッチ内の最後の一致を取る
                        return !forward;
                    }
                    true
                });
                if let Some(r) = hit {
                    return Found::Hit(r, wrapped);
                }
                if forward {
                    lo = b;
                } else {
                    hi = a;
                }
            }
        }
        Found::None
    }

    fn search_select(&mut self, m: ByteRange, wrapped: bool, forward: bool) {
        let Some(view) = self.first_window().and_then(|w| self.window_view(w)) else {
            return;
        };
        self.set_selection(view, Selection::single(Range::new(m.start, m.end)));
        self.ensure_cursor_visible(view);
        if wrapped {
            let key = if forward {
                "info.search.wrapped_forward"
            } else {
                "info.search.wrapped_backward"
            };
            self.show_message(MessageLevel::Info, key, &[]);
        }
        self.mark_dirty_render();
    }

    /// 検索ジョブの途中経過と終わり。古いジョブの結果は捨てる。
    pub(super) fn on_search_result(&mut self, r: JobResult) {
        let job = r.job();
        let Some(s) = self.search.as_mut().filter(|s| s.job == Some(job)) else {
            // 取り消した検索の結果。終わりなら待ちの記録を消す
            if !matches!(r, JobResult::SearchBatch { .. }) {
                self.pending.remove(&job);
            }
            return;
        };
        match r {
            JobResult::SearchBatch { matches, .. } => s.matches.extend(matches),
            JobResult::SearchDone {
                total,
                count_limited,
                truncated,
                ..
            } => {
                s.job = None;
                s.total = total;
                s.count_limited = count_limited;
                s.truncated = truncated;
                s.matches.sort_by_key(|m| m.start);
                self.pending.remove(&job);
            }
            // 失敗・取り消し: 件数は出さずに終える
            _ => {
                s.job = None;
                self.pending.remove(&job);
            }
        }
        self.mark_dirty_render();
    }
}
