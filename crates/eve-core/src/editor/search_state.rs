//! 文書内検索の状態と、次へ・前への移動 (docs/03 §12.2、01 FR-401・FR-404)。
//!
//! 全件走査はバックグラウンドのジョブで行い、届いた一致を保持する。移動は保持した一致から選ぶ。保持に無い (走査中で
//! まだ届いていない、保持の上限を超えた、編集で無効になった) ときは、カーソルから同期検索する (最大 1 MiB)。
//! 編集があると保持は無効になり、件数は「約」表示に落ちる。再検索は呼び出し側 (検索バー) が入力の間を置いて始める。

use eve_text::{ByteOff, ByteRange, LineIdx, Range, Selection, TextRead};
use eve_view::{DocumentId, JobId, MessageLevel, Revision};

use crate::cancel::CancelToken;
use crate::jobs::{JobKind, JobResult};
use crate::search::{SearchQuery, scan_lines};

use super::file::Pending;
use super::state::EditorState;

/// 保持する一致の上限 (`search.max_matches`、07 §5.5)。
const MAX_MATCHES: usize = 10_000;
/// 数える一致の上限 (`search.count_limit`、07 §5.5)。
const COUNT_LIMIT: u64 = 1_000_000;
/// 同期検索で走査する量 (`SEARCH_SYNC_STEP_BYTES`、03 §12.2)。
const SYNC_STEP_BYTES: usize = 1 << 20;

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

enum Found {
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

    fn search_fresh(&self) -> bool {
        self.search.as_ref().is_some_and(|s| {
            self.documents
                .get(&s.doc)
                .is_some_and(|d| d.revision() == s.revision)
        })
    }

    /// 件数の表示の材料。
    pub fn search_count(&self) -> SearchCount {
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
        let current = self.search_current_index().map(|i| i as u64 + 1);
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
    fn search_current_index(&self) -> Option<usize> {
        let s = self.search.as_ref()?;
        if s.truncated || s.count_limited {
            return None;
        }
        let sel = self
            .view(self.window_view(self.first_window()?)?)?
            .selection
            .primary();
        let (a, b) = (sel.start(), sel.end());
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
            Found::Incomplete => SearchStep::NotYet,
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
    fn sync_find(
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
        let mut budget = SYNC_STEP_BYTES;
        // 走査する行の並び: (行, 折り返し済みか)
        let order: Vec<(usize, bool)> = if forward {
            (line..n)
                .map(|l| (l, false))
                .chain((0..=line).map(|l| (l, true)))
                .collect()
        } else {
            (0..=line)
                .rev()
                .map(|l| (l, false))
                .chain((line..n).rev().map(|l| (l, true)))
                .collect()
        };
        for (l, wrapped) in order {
            let (ls, le) = (t.line_to_byte(LineIdx(l)).0, t.line_end_byte(LineIdx(l)).0);
            let len = le - ls;
            if len > budget {
                return Found::Incomplete;
            }
            budget -= len.max(1);
            let mut hit: Option<ByteRange> = None;
            scan_lines(t, &m, l..l + 1, &cancel, &mut |r| {
                let ok = if forward {
                    // 折り返した後は、カーソルの行の全体を対象にする
                    wrapped || r.start >= cursor
                } else {
                    wrapped || r.end <= cursor
                };
                if ok {
                    if forward {
                        if hit.is_none() {
                            hit = Some(r);
                        }
                        return false;
                    }
                    hit = Some(r);
                }
                true
            });
            if let Some(r) = hit {
                return Found::Hit(r, wrapped);
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
