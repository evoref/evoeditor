//! 文書内検索の状態と、次へ・前への移動 (docs/03 §12.2・§12.3.1、01 FR-401・FR-404)。
//!
//! 全件走査はバックグラウンドのジョブで行い、届いた一致を保持する。移動は保持した一致から選ぶ。保持に無い (走査中で
//! まだ届いていない、保持の上限を超えた、編集で無効になった) ときは、カーソルから同期検索する (最大 256 KiB)。
//! 正規表現の照合器は `bg` のジョブがコンパイルするので、まだ無いときは同期検索をせずに単発のジョブに回す (03 §12.3.1)。
//! 編集があると保持は無効になり、件数は「約」表示に落ちる。再検索は呼び出し側 (検索バー) が入力の間を置いて始める。
//!
//! **行をまたぐ検索の規則** (`Shape::whole_doc`。k ≥ 1・上限なし・重い検索語、03 §12.3.1 の「呼び出し側」):
//! 全件の走査は文書の先頭から (正本) で、カーソル以降の最初の一致は `first_after` で届き、再開の位置 (1 MiB ごと) が
//! `SearchDone` で届く。次 / 前は同期検索をせず、保持した一致 (正本の先頭の部分) に答えがあればそれへ、全件の走査中なら
//! その結果を待ち、それ以外は単発のジョブを正本の位置 (再開の位置・今の一致の終端) から投げる。単発のジョブを待つ間に同じ
//! 向きを押した回数は数え、届いて移った後に続ける。編集では、編集した行の (k + 1) 行より前の再開の位置だけを残す。

use eve_text::{ByteOff, ByteRange, LineIdx, Range, Selection, TextRead};
use eve_view::{DocumentId, JobId, MessageLevel, Revision};

use crate::jobs::cancel::CancelToken;
use crate::jobs::{JobKind, JobOutput, JobResult};
use crate::search::{LineSpan, Prepared, SEARCH_FIND_WAIT_MS, ScanLimit, SearchQuery, scan_bytes};
use crate::time::{Millis, Tick};
use crate::ui_model::keys;

use crate::editor::state::EditorState;
use crate::io::pending::Pending;

/// 同期検索で走査する量 (`SEARCH_SYNC_STEP_BYTES`、03 §12.2)。
const SYNC_STEP_BYTES: usize = 1 << 18;
/// 同期検索で一度に `scan_bytes` へ渡す量 (打ち切りの粒度)。
const SYNC_BATCH_BYTES: usize = 64 << 10;

#[derive(Debug)]
pub(crate) struct SearchState {
    doc: DocumentId,
    /// 結果が属する文書の Revision。違えば保持は無効 (`Approx`)。
    revision: Revision,
    query: SearchQuery,
    job: Option<JobId>,
    /// 走査した順に届いた一致 (終わったら開始位置の昇順に整える)。行をまたぐ検索の規則では文書の順に届き、正本の先頭の部分。
    matches: Vec<ByteRange>,
    total: u64,
    count_limited: bool,
    /// 保持の上限を超えて、一致の一部しか持っていない。
    truncated: bool,
    /// 行をまたぐ検索の規則で探している (`Shape::whole_doc`、03 §12.3.1 の「呼び出し側」)。
    whole: bool,
    /// メインスレッドで同期検索をしない (`Shape::job_driven`。行をまたぐ検索の規則か、重い検索語)。
    jobbed: bool,
    /// 全件ジョブの起点 (投げ直すときに使う)。
    from: ByteOff,
    /// 全件ジョブが `SearchDone` で終わった (取り消し・失敗で終わったときは偽。保持した一致を全部とみなさない)。
    done_ok: bool,
    /// 検索語の k (編集で残す再開の位置の範囲を決める)。
    lines: LineSpan,
    /// 全件ジョブが知らせた、起点以降の最初の一致 (行をまたぐ検索の規則)。
    first_after: Option<ByteRange>,
    /// 再開の位置 (昇順。`marks_rev` の文書で有効。編集では、編集した行の (k + 1) 行より前のものだけを残す)。
    resume: Vec<ByteOff>,
    /// 最後に移った・知らされた正本の一致 (その終端は正本の位置。`marks_rev` の文書で有効)。
    canon: Option<ByteRange>,
    /// `resume` と `canon` が有効な文書の Revision。
    marks_rev: Revision,
    /// 探さなかった・重すぎて打ち切った理由 (03 §12.3.1)。あれば一致・件数は持たない。
    limit: Option<ScanLimit>,
    /// 投入したときの主選択 (`first_after` が届いたとき、選択が変わっていなければスクロールする)。
    spawn_sel: Option<(ByteOff, ByteOff)>,
}

/// 全件ジョブの結果を待っている次 / 前 (行をまたぐ検索の規則、03 §12.3.1 の「次 / 前」の (4))。
#[derive(Clone, Copy, Debug)]
pub(crate) struct StepWait {
    forward: bool,
    /// 待っている間に同じ向きを押した回数 (移った後に続ける)。
    presses: u32,
    cursor: ByteOff,
    revision: Revision,
}

/// 件数の表示の材料 (`SearchCount`、04 §8.4)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchCount {
    /// 検索語が空、または検索していない。
    Idle,
    /// 走査中 (届いた件数)。
    Running {
        /// これまでに届いた件数。
        found: u64,
    },
    /// 全件を数えた。`current` は選択が一致のとき、その番号 (1 始まり)。
    Exact {
        /// 選択している一致の番号 (1 始まり)。選択が一致でなければ `None`。
        current: Option<u64>,
        /// 一致の総数。
        total: u64,
    },
    /// 数える上限に達した。
    AtLeast {
        /// 選択している一致の番号 (1 始まり)。選択が一致でなければ `None`。
        current: Option<u64>,
        /// 数えた件数 (これ以上ある)。
        total: u64,
    },
    /// 編集で保持が無効になった。目安の件数。
    Approx {
        /// 編集の前に数えた件数。
        total: u64,
    },
}

/// `search_step` の結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchStep {
    /// 一致へ移った。`wrapped` は文書の端を越えた。
    Moved {
        /// 文書の端を越えて折り返した。
        wrapped: bool,
    },
    /// 一致が無い。
    NoMatch,
    /// 同期検索の範囲では見つからなかった (全件の走査が終われば見つかる可能性がある)。
    NotYet,
    /// 検索していない・検索語が空。
    NoQuery,
}

pub(crate) enum Found {
    Hit(ByteRange, bool),
    None,
    Incomplete,
}

/// 行をまたぐ検索の規則の次 / 前の答え方。
enum Resolve {
    /// 保持した一致に答えがある (`wrapped`)。
    Hit(ByteRange, bool),
    /// 一致が無い (全件を数えて 0 件)。
    None,
    /// 全件の走査の結果を待つ。
    Wait,
    /// 単発のジョブで探す。
    Job,
}

impl SearchState {
    pub(crate) fn doc(&self) -> DocumentId {
        self.doc
    }

    /// 行をまたぐ検索の規則で探している。
    pub(crate) fn is_whole(&self) -> bool {
        self.whole
    }

    /// 探さなかった・打ち切った理由。
    pub(crate) fn limit(&self) -> Option<ScanLimit> {
        self.limit
    }

    /// 全件ジョブが知らせた、起点以降の最初の一致。
    pub(crate) fn first_after(&self) -> Option<ByteRange> {
        self.first_after
    }

    /// 全件の走査が終わった。
    pub(crate) fn finished(&self) -> bool {
        self.job.is_none()
    }

    /// 全件の走査が終わり、数える上限で打ち切っていない (一致を最後まで数えた)。
    pub(crate) fn counted_all(&self) -> bool {
        self.done_ok && !self.count_limited
    }

    /// メインスレッドで同期検索をしない検索語 (`Shape::job_driven`)。
    pub(crate) fn is_job_driven(&self) -> bool {
        self.jobbed
    }

    /// 全件の一致を持っている (走査が `SearchDone` で終わり、保持と数える上限で打ち切っていない)。
    fn complete(&self) -> bool {
        self.done_ok && !self.truncated && !self.count_limited
    }

    /// 保持した一致 (行をまたぐ検索の規則では文書の順)。
    pub(crate) fn retained(&self) -> &[ByteRange] {
        &self.matches
    }

    /// 保持した一致が文書のどこまでの一致を漏れなく持つか (行をまたぐ検索の規則。この位置より前で始まる正本の一致は
    /// すべて保持している)。全件を持てば文書の末尾 (`usize::MAX`)。
    /// 走査の順が文書の順でない (v0.3 の順の) 検索語は、全件を持つときのほかは 0。
    pub(crate) fn covered(&self) -> usize {
        if self.complete() {
            return usize::MAX;
        }
        if !self.whole {
            return 0;
        }
        self.matches.last().map_or(0, |r| r.start.0)
    }

    /// 投入したときの主選択。
    pub(crate) fn spawn_sel(&self) -> Option<(ByteOff, ByteOff)> {
        self.spawn_sel
    }

    /// 保持した一致 (編集で無効になっていない前提で呼ぶ) に `r` があるか。
    pub(crate) fn retains(&self, r: ByteRange) -> bool {
        self.matches
            .binary_search_by_key(&r.start, |m| m.start)
            .is_ok_and(|i| self.matches[i] == r)
    }

    /// 最後に移った正本の一致が `r` で、`rev` の文書で有効か。
    pub(crate) fn is_canon(&self, r: ByteRange, rev: Revision) -> bool {
        self.marks_rev == rev && self.canon == Some(r)
    }
}

impl EditorState {
    /// 検索を始める (進行中の検索は取り消す)。検索語が空なら検索を止めて状態を消す。対象は今の文書。
    pub fn search_start(&mut self, query: SearchQuery) {
        self.search_start_from(query, None);
    }

    /// `search_start` と同じで、起点を `from` にする (行をまたぐ検索の規則で、この位置以降の最初の一致を `first_after` で
    /// 受け取る。`None` は選択の終端)。
    pub(crate) fn search_start_from(&mut self, query: SearchQuery, from: Option<ByteOff>) {
        self.search_cancel_job();
        if query.pattern().is_empty() {
            if self.active_document().is_some() {
                self.find.search = None;
                self.mark_dirty_render();
            }
            return;
        }
        let from = from
            .or_else(|| self.search_cursor(true))
            .unwrap_or(ByteOff(0));
        self.spawn_search(query, from, None);
    }

    /// 全件ジョブを投げて、検索の状態を作り直す (進行中のジョブは呼び出し側が取り消しておく)。`limit` があれば、ジョブを
    /// 投げずに「探さない理由」の状態にする (編集で文書が変わったら、次の再検索で判定し直す、03 §12.3.1 の「探さない場合」)。
    fn spawn_search(&mut self, query: SearchQuery, from: ByteOff, limit: Option<ScanLimit>) {
        let Some(doc_id) = self.active_document() else {
            return;
        };
        let spawn_sel = self.primary_sel();
        let Some(doc) = self.docs.documents.get(&doc_id) else {
            return;
        };
        let revision = doc.revision();
        let snapshot = doc.snapshot();
        let shape = query.shape();
        let whole = shape.is_some_and(|s| s.whole_doc());
        let jobbed = shape.is_some_and(|s| s.job_driven());
        let lines = shape.map_or(LineSpan::Bounded(0), |s| s.lines());
        // 保持と件数の上限は、検索バーを開いたときの設定の値 (バーが無いときは今の設定、`search.max_matches` /
        // `search.count_limit`、07 §5.5)
        let (max_matches, count_limit) = match &self.find.search_bar {
            Some(bar) => (bar.max_matches, bar.count_limit),
            None => (
                usize::try_from(self.config().max_matches).unwrap_or(usize::MAX),
                self.config().count_limit,
            ),
        };
        let job = if limit.is_none() {
            let job = self.jobs.spawn(
                Some(doc_id),
                revision,
                JobKind::Search {
                    snapshot,
                    query: query.clone(),
                    from,
                    max_matches,
                    count_limit,
                },
            );
            self.io.pending.insert(job, Pending::Search);
            Some(job)
        } else {
            None
        };
        self.find.search = Some(SearchState {
            doc: doc_id,
            revision,
            query,
            job,
            matches: Vec::new(),
            total: 0,
            count_limited: false,
            truncated: false,
            whole,
            jobbed,
            from,
            done_ok: false,
            lines,
            first_after: None,
            resume: Vec::new(),
            canon: None,
            marks_rev: revision,
            limit,
            spawn_sel,
        });
        self.mark_dirty_render();
    }

    /// 始める前に探さないと分かった (03 §12.3.1 の「探さない場合」): 進行中のジョブを取り消し、ジョブを投げずに理由だけを持つ
    /// 状態にする。編集で文書が変わると古い状態になり、入力が止まって 300 ms 後の再検索で判定し直す (走査の途中で分かった
    /// 場合と同じ)。
    pub(crate) fn search_refuse(&mut self, query: SearchQuery, limit: ScanLimit) {
        self.search_cancel_job();
        let from = self.search_cursor(true).unwrap_or(ByteOff(0));
        self.spawn_search(query, from, Some(limit));
    }

    /// `bg` のジョブ (単発の検索・すべて置換・1 件置換の確かめ) を、走っている全件ジョブの後ろで待たせない (`bg` は 1 本の
    /// 先着順、02 §9.3): 全件ジョブを取り消してから `spawn` で投げ、同じ条件の全件ジョブを、その後ろに投げ直す (03 §12.3.1 の
    /// 「`bg` レーンの順序と取り消し」)。待ちは全件ジョブの取り消しの遅れだけになる。
    pub(crate) fn yield_search(&mut self, spawn: impl FnOnce(&mut EditorState)) {
        let running = self.find.search.as_mut().and_then(|s| {
            let job = s.job.take()?;
            Some((job, s.query.clone(), s.from))
        });
        if let Some((job, ..)) = &running {
            self.jobs.cancel(*job);
            self.io.pending.remove(job);
        }
        spawn(self);
        if let Some((_, query, from)) = running {
            self.spawn_search(query, from, None);
        }
    }

    /// 検索を終える (検索バーを閉じる)。
    pub fn search_stop(&mut self) {
        self.search_cancel_job();
        if self.find.search.take().is_some() {
            self.mark_dirty_render();
        }
    }

    /// 今の検索語。
    pub fn search_query(&self) -> Option<&SearchQuery> {
        self.find.search.as_ref().map(|s| &s.query)
    }

    fn search_cancel_job(&mut self) {
        self.cancel_find_next();
        if let Some(job) = self.find.search.as_mut().and_then(|s| s.job.take()) {
            self.jobs.cancel(job);
            self.io.pending.remove(&job);
        }
    }

    /// 単発の検索と、全件ジョブを待つ次 / 前を取り消す (押した回数も捨てる)。
    fn cancel_find_next(&mut self) {
        if let Some(old) = self.find.find_job.take() {
            self.jobs.cancel(old);
            self.io.pending.remove(&old);
        }
        self.find.find_presses = 0;
        self.find.find_wait = None;
        self.find.find_since = None;
    }

    /// 主選択 (始端, 終端)。
    fn primary_sel(&self) -> Option<(ByteOff, ByteOff)> {
        let r = *self.view(self.focused_view()?)?.selection.primary();
        Some((r.start(), r.end()))
    }

    /// 選択の端 (次へは終端、前へは始端)。
    fn search_cursor(&self, forward: bool) -> Option<ByteOff> {
        let (s, e) = self.primary_sel()?;
        Some(if forward { e } else { s })
    }

    pub(crate) fn search_fresh(&self) -> bool {
        self.find.search.as_ref().is_some_and(|s| {
            self.docs
                .documents
                .get(&s.doc)
                .is_some_and(|d| d.revision() == s.revision)
        })
    }

    /// 検索の対象の文書の今の Revision。
    fn search_doc_revision(&self) -> Option<Revision> {
        let s = self.find.search.as_ref()?;
        self.docs.documents.get(&s.doc).map(|d| d.revision())
    }

    /// 件数の表示の材料。
    pub fn search_count(&self) -> SearchCount {
        self.search_count_for(None)
    }

    /// `search_count` と同じだが、現在の一致の番号を `probe` (選択していない強調中の一致) から求める。
    pub(crate) fn search_count_for(&self, probe: Option<ByteRange>) -> SearchCount {
        let Some(s) = &self.find.search else {
            return SearchCount::Idle;
        };
        // 探さなかった・打ち切った: 件数は出さない (検索バーは理由を出す、03 §12.3.1)
        if s.limit.is_some() {
            return SearchCount::Idle;
        }
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

    /// 行をまたぐ検索の規則の単発の検索 (か全件ジョブ) の結果を、`SEARCH_FIND_WAIT_MS` を過ぎて待っている (件数の位置を
    /// 待ちの表示にする、03 §12.3.1 の「次 / 前」の (6))。
    pub(crate) fn search_find_waiting(&self) -> bool {
        self.find
            .find_since
            .is_some_and(|t| self.now.saturating_sub(t) >= Millis(SEARCH_FIND_WAIT_MS))
    }

    /// 件数の表示の現在の一致 (`search_count_for` が番号を出す一致) の範囲。
    pub(crate) fn search_current_range(&self, probe: Option<ByteRange>) -> Option<ByteRange> {
        let s = self.find.search.as_ref()?;
        if !self.search_fresh() || s.job.is_some() || s.limit.is_some() {
            return None;
        }
        self.search_current_index(probe).map(|i| s.matches[i])
    }

    /// 選択が保持した一致のどれかと一致していれば、その位置。一部しか持っていないときは求めない。
    fn search_current_index(&self, probe: Option<ByteRange>) -> Option<usize> {
        let s = self.find.search.as_ref()?;
        if s.truncated || s.count_limited {
            return None;
        }
        let (a, b) = match probe {
            Some(r) => (r.start, r.end),
            None => self.primary_sel()?,
        };
        s.matches
            .binary_search_by_key(&a, |m| m.start)
            .ok()
            .filter(|&i| s.matches[i].end == b)
    }

    /// 次の一致 (`forward`) か前の一致へ選択を移す。
    pub fn search_step(&mut self, forward: bool) -> SearchStep {
        let Some(s) = &self.find.search else {
            return SearchStep::NoQuery;
        };
        if s.whole {
            return self.search_step_whole(forward);
        }
        if s.limit.is_some() {
            self.notify_search_limit();
            return SearchStep::NoMatch;
        }
        let (doc_id, query, jobbed) = (s.doc, s.query.clone(), s.jobbed);
        let Some(cursor) = self.search_cursor(forward) else {
            return SearchStep::NoQuery;
        };
        // 編集で無効になっていたら、保持を捨てて走査をやり直す (移動は同期検索で行う)
        let fresh = self.search_fresh();
        if !fresh {
            self.search_start(query.clone());
        }
        // 保持した一致を使えるのは、走査が終わり、上限にも達していないとき (全件を持っているとき) だけ
        let complete = fresh && self.find.search.as_ref().is_some_and(SearchState::complete);
        let found = if complete {
            match self.find_retained(cursor, forward) {
                Some(h) => h,
                None => match self.find.search.as_ref().map(|s| &s.matches[..]) {
                    Some([]) | None => Found::None,
                    // 端を越えて反対側の端へ
                    Some(m) => Found::Hit(if forward { m[0] } else { m[m.len() - 1] }, true),
                },
            }
        } else if jobbed {
            // 重い検索語はメインスレッドで同期検索をしない (単発のジョブで探す、03 §12.3.1 の「重い検索語」)
            Found::Incomplete
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
                self.spawn_find_next(doc_id, query, cursor, forward, Vec::new());
                SearchStep::NotYet
            }
        }
    }

    /// 行をまたぐ検索の規則の次 / 前 (03 §12.3.1 の「次 / 前」)。同期検索をしない。
    fn search_step_whole(&mut self, forward: bool) -> SearchStep {
        let Some(s) = &self.find.search else {
            return SearchStep::NoQuery;
        };
        // 探さなかった・打ち切った: 何もせず、理由をメッセージ行に出す
        if s.limit.is_some() {
            self.notify_search_limit();
            return SearchStep::NoMatch;
        }
        let Some(rev) = self.search_doc_revision() else {
            return SearchStep::NoQuery;
        };
        let Some(cursor) = self.search_cursor(forward) else {
            return SearchStep::NoQuery;
        };
        // (5) 同じ (Revision, 起点, 向き) の単発のジョブを待っている間に押した: 取り消さずに数える (届いて移った後に続ける)。
        // 起点が違う (待つ間にカーソルを動かした) ときは、取り消して今の起点から投げ直す
        if let Some(job) = self.find.find_job {
            if let Some(Pending::FindNext {
                revision,
                cursor: c,
                forward: f,
                ..
            }) = self.io.pending.get(&job)
                && *f == forward
                && *revision == rev
                && *c == cursor
            {
                self.find.find_presses = self.find.find_presses.saturating_add(1);
                return SearchStep::NotYet;
            }
            self.cancel_find_next();
        }
        // (4) 全件ジョブの結果を待っている間も同じ
        if let Some(w) = &mut self.find.find_wait {
            if w.forward == forward && w.revision == rev && w.cursor == cursor {
                w.presses = w.presses.saturating_add(1);
                return SearchStep::NotYet;
            }
            self.cancel_find_next();
        }
        self.step_whole_from(cursor, forward, 0)
    }

    /// 行をまたぐ検索の規則で、`cursor` から 1 回移る。待つときは、待ちの後に続ける回数 `presses` を持たせる。
    fn step_whole_from(&mut self, cursor: ByteOff, forward: bool, presses: u32) -> SearchStep {
        let Some(s) = &self.find.search else {
            return SearchStep::NoQuery;
        };
        let (doc_id, query) = (s.doc, s.query.clone());
        let Some(rev) = self.search_doc_revision() else {
            return SearchStep::NoQuery;
        };
        match self.resolve_whole(cursor, forward) {
            Resolve::Hit(m, wrapped) => {
                self.search_select(m, wrapped, forward);
                self.mark_canon(m);
                SearchStep::Moved { wrapped }
            }
            Resolve::None => SearchStep::NoMatch,
            Resolve::Wait => {
                self.find.find_wait = Some(StepWait {
                    forward,
                    presses,
                    cursor,
                    revision: rev,
                });
                self.find.find_since = Some(self.now);
                SearchStep::NotYet
            }
            Resolve::Job => {
                let starts = self.find_starts(cursor);
                self.spawn_find_next(doc_id, query, cursor, forward, starts);
                self.find.find_presses = presses;
                self.find.find_since = Some(self.now);
                SearchStep::NotYet
            }
        }
    }

    /// 移った後に、待っている間に押された残りの回数だけ続ける (03 §12.3.1 の「次 / 前」の (5))。
    fn continue_presses(&mut self, forward: bool, mut presses: u32) {
        while presses > 0 {
            presses -= 1;
            let Some(cursor) = self.search_cursor(forward) else {
                return;
            };
            match self.step_whole_from(cursor, forward, presses) {
                SearchStep::Moved { .. } => {}
                // 待ちに入った (残りの回数は待ちが持つ)・一致が無い
                _ => return,
            }
        }
    }

    /// 行をまたぐ検索の規則で、保持した一致 (正本の先頭の部分) から `cursor` の次 / 前を決める。
    fn resolve_whole(&self, cursor: ByteOff, forward: bool) -> Resolve {
        let Some(s) = &self.find.search else {
            return Resolve::None;
        };
        if self.search_fresh() {
            let m = &s.matches;
            let complete = s.covered() == usize::MAX;
            if forward {
                // 前向き: 保持した一致に起点以降のものがあれば、その最初のもの (保持は正本の先頭の部分なので、それが正本の次)
                let i = m.partition_point(|r| r.start < cursor);
                if let Some(r) = m.get(i) {
                    return Resolve::Hit(*r, false);
                }
                if complete {
                    return m.first().map_or(Resolve::None, |r| Resolve::Hit(*r, true));
                }
            } else if s.covered() >= cursor.0 {
                // 後ろ向き: 起点より前で始まる正本の一致をすべて保持しているときだけ
                let i = m.partition_point(|r| r.end <= cursor);
                if i > 0 {
                    return Resolve::Hit(m[i - 1], false);
                }
                if complete {
                    return m.last().map_or(Resolve::None, |r| Resolve::Hit(*r, true));
                }
            }
            // (4) 全件ジョブが走っている間は単発のジョブを投げずに、その結果を待つ
            if s.job.is_some() {
                return Resolve::Wait;
            }
        }
        Resolve::Job
    }

    /// 単発のジョブに渡す正本の位置 (再開の位置・最後に移った一致の終端・起点より前の保持した一致の終端)。
    fn find_starts(&self, cursor: ByteOff) -> Vec<ByteOff> {
        let Some(s) = &self.find.search else {
            return Vec::new();
        };
        let mut v = Vec::new();
        if self.search_doc_revision() == Some(s.marks_rev) {
            v.extend_from_slice(&s.resume);
            if let Some(c) = s.canon {
                v.push(c.end);
            }
        }
        if self.search_fresh() {
            let i = s.matches.partition_point(|r| r.end <= cursor);
            if i > 0 {
                v.push(s.matches[i - 1].end);
            }
        }
        v.sort_unstable();
        v.dedup();
        v
    }

    /// 正本の一致 `m` へ移った (その終端は正本の位置、03 §12.3.1 の「次 / 前」の (1))。
    pub(crate) fn mark_canon(&mut self, m: ByteRange) {
        let rev = self.search_doc_revision();
        let Some(s) = self.find.search.as_mut().filter(|s| s.whole) else {
            return;
        };
        if Some(s.marks_rev) != rev {
            s.resume.clear();
            if let Some(r) = rev {
                s.marks_rev = r;
            }
        }
        s.canon = Some(m);
    }

    /// 単発の検索ジョブを投入する (古い単発は取り消す)。`resume` は正本の位置 (行をまたぐ検索の規則だけ)。
    fn spawn_find_next(
        &mut self,
        doc_id: DocumentId,
        query: SearchQuery,
        cursor: ByteOff,
        forward: bool,
        resume: Vec<ByteOff>,
    ) {
        if let Some(old) = self.find.find_job.take() {
            self.jobs.cancel(old);
            self.io.pending.remove(&old);
        }
        let Some(doc) = self.docs.documents.get(&doc_id) else {
            return;
        };
        let revision = doc.revision();
        let snapshot = doc.snapshot();
        // 走っている全件ジョブの後ろで待たない (取り消して、単発の後ろに投げ直す)
        self.yield_search(|st| {
            let job = st.jobs.spawn(
                Some(doc_id),
                revision,
                JobKind::FindNext {
                    snapshot,
                    query,
                    from: cursor,
                    forward,
                    resume,
                },
            );
            st.io.pending.insert(
                job,
                Pending::FindNext {
                    doc: doc_id,
                    revision,
                    cursor,
                    forward,
                },
            );
            st.find.find_job = Some(job);
        });
    }

    /// 単発の検索の結果。文書の Revision と選択が投入時のままなら移る (違えば捨てる)。ジョブのコンパイルで誤り
    /// (`TooLarge`) が分かったときは、移らずに検索バーに示す (03 §12.3.1)。探さなかった・打ち切ったときも示す。
    pub(crate) fn finish_find_next(
        &mut self,
        doc_id: DocumentId,
        revision: Revision,
        cursor: ByteOff,
        forward: bool,
        r: JobResult,
    ) {
        self.find.find_job = None;
        let presses = std::mem::take(&mut self.find.find_presses);
        if self.find.find_wait.is_none() {
            self.find.find_since = None;
        }
        self.mark_dirty_render();
        if self.bar_show_compile_error() {
            return;
        }
        let found = match r {
            JobResult::FindNextDone { found, .. } => found,
            JobResult::SearchLimited { limit } => {
                self.search_set_limit(limit);
                return;
            }
            _ => return,
        };
        let same = self
            .docs
            .documents
            .get(&doc_id)
            .is_some_and(|d| d.revision() == revision)
            && self.search_cursor(forward) == Some(cursor);
        if !same {
            return;
        }
        if let Some((m, wrapped)) = found {
            self.search_select(m, wrapped, forward);
            if self.find.search.as_ref().is_some_and(|s| s.whole) {
                self.mark_canon(m);
                self.continue_presses(forward, presses);
            }
        }
    }

    /// 探さなかった・打ち切った (03 §12.3.1): それまでに届いた一致・件数を捨て、走査と次 / 前をやめて、理由を持つ。
    pub(crate) fn search_set_limit(&mut self, limit: ScanLimit) {
        self.search_cancel_job();
        if let Some(s) = self.find.search.as_mut() {
            s.limit = Some(limit);
            s.matches.clear();
            s.total = 0;
            s.count_limited = false;
            s.truncated = false;
            s.first_after = None;
            s.resume.clear();
            s.canon = None;
        }
        self.mark_dirty_render();
        self.bar_search_progress();
    }

    /// 保持した一致から、`cursor` より先 (前) の最初の一致。
    fn find_retained(&self, cursor: ByteOff, forward: bool) -> Option<Found> {
        let s = self.find.search.as_ref()?;
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
    pub(crate) fn sync_find(
        &self,
        doc_id: DocumentId,
        query: &SearchQuery,
        cursor: ByteOff,
        forward: bool,
    ) -> Found {
        let m = match query.prepared() {
            Prepared::Ready(m) => m,
            Prepared::Nothing => return Found::None,
            // 正規表現の照合器は `bg` のジョブが作る。まだ無ければ同期検索をせず、単発のジョブに回す (03 §12.3.1)
            Prepared::Pending => return Found::Incomplete,
        };
        let Some(doc) = self.docs.documents.get(&doc_id) else {
            return Found::None;
        };
        let t = doc.text();
        let n = t.len_lines();
        let line = t.byte_to_line(cursor).0.min(n.saturating_sub(1));
        let cancel = CancelToken::new();
        let len = t.len_bytes();
        // 行ごとに切り出す経路 (正規表現と、大小を区別しないリテラルのうち §12.4 の 3 のエスケープした `(?i)` の正規表現) は
        // memmem より 1〜2 桁遅い (行ごとに畳み込んでいた v0.1 の実装で約 50 MB/s、1 MiB で約 20 ms。BM-404)。量を 1/16 に絞り、
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
        let Some(view) = self.focused_view() else {
            return;
        };
        self.set_selection(view, Selection::single(Range::new(m.start, m.end)));
        self.ensure_cursor_visible(view);
        if wrapped {
            let key = if forward {
                keys::INFO_SEARCH_WRAPPED_FORWARD
            } else {
                keys::INFO_SEARCH_WRAPPED_BACKWARD
            };
            self.show_message(MessageLevel::Info, key, &[]);
        }
        self.mark_dirty_render();
    }

    /// 検索ジョブの途中経過と終わり。古いジョブの結果は捨てる。
    pub(crate) fn on_search_result(&mut self, out: JobOutput) {
        let JobOutput { job, body: r } = out;
        let fresh = self.search_fresh();
        let Some(s) = self.find.search.as_mut().filter(|s| s.job == Some(job)) else {
            // 取り消した検索の結果。終わりなら待ちの記録を消す
            if !matches!(r, JobResult::SearchBatch { .. }) {
                self.io.pending.remove(&job);
            }
            return;
        };
        match r {
            JobResult::SearchBatch {
                matches,
                first_after,
                ..
            } => {
                s.matches.extend(matches);
                if first_after.is_some() {
                    s.first_after = first_after;
                }
            }
            JobResult::SearchDone {
                total,
                count_limited,
                truncated,
                resume,
                limit,
                ..
            } => {
                s.job = None;
                self.io.pending.remove(&job);
                if let Some(l) = limit {
                    // 途中で探さないと分かった・重すぎて打ち切った: それまでに届いた一致 (強調・保持・件数) を捨てる
                    self.search_set_limit(l);
                    return;
                }
                s.total = total;
                s.count_limited = count_limited;
                s.truncated = truncated;
                s.done_ok = true;
                s.matches.sort_by_key(|m| m.start);
                // 再開の位置は、走査した文書が今の文書のときだけ使う (編集の後に届いたものは、編集を写していない)
                if s.whole && fresh {
                    s.resume = resume;
                    s.marks_rev = s.revision;
                }
            }
            // 失敗・取り消し: 件数は出さずに終える
            _ => {
                s.job = None;
                self.io.pending.remove(&job);
            }
        }
        self.mark_dirty_render();
        // 正規表現の照合器はジョブが作る。届いたら、入力中の現在の一致を求める (03 §12.3.1)
        self.bar_search_progress();
        self.resolve_find_wait();
    }

    /// 全件ジョブの結果を待っていた次 / 前を、届いた保持した一致で決める (03 §12.3.1 の「次 / 前」の (4))。
    fn resolve_find_wait(&mut self) {
        let Some(w) = self.find.find_wait else {
            return;
        };
        let rev = self.search_doc_revision();
        let limited = self.find.search.as_ref().is_none_or(|s| s.limit.is_some());
        if limited || rev != Some(w.revision) || self.search_cursor(w.forward) != Some(w.cursor) {
            self.find.find_wait = None;
            self.find.find_since = None;
            return;
        }
        match self.resolve_whole(w.cursor, w.forward) {
            Resolve::Wait => {}
            Resolve::Hit(m, wrapped) => {
                self.find.find_wait = None;
                self.find.find_since = None;
                self.search_select(m, wrapped, w.forward);
                self.mark_canon(m);
                self.continue_presses(w.forward, w.presses);
            }
            Resolve::None => {
                self.find.find_wait = None;
                self.find.find_since = None;
            }
            Resolve::Job => {
                self.find.find_wait = None;
                let Some(s) = &self.find.search else {
                    return;
                };
                let (doc, query) = (s.doc, s.query.clone());
                let starts = self.find_starts(w.cursor);
                self.spawn_find_next(doc, query, w.cursor, w.forward, starts);
                self.find.find_presses = w.presses;
            }
        }
        self.mark_dirty_render();
    }

    /// 編集に合わせて、行をまたぐ検索の規則の再開の位置と最後に移った一致を写す (03 §12.3.1 の「全件走査」): 編集した行の
    /// (k + 1) 行より前のものは、そこまでの正本の走査が編集した範囲を読まないので残し、後ろは捨てる。待っている次 / 前は
    /// 取り消す (Revision が変わった)。
    pub(crate) fn map_search_marks(
        &mut self,
        doc: DocumentId,
        cs: &eve_text::ChangeSet,
        old_rev: Revision,
    ) {
        if !self
            .find
            .search
            .as_ref()
            .is_some_and(|s| s.doc == doc && s.jobbed)
        {
            return;
        }
        self.cancel_find_next();
        // 入力中の現在の一致は編集の前の文書のもの (正本として使わない。再検索で求め直す)
        if let Some(bar) = &mut self.find.search_bar {
            bar.current = None;
        }
        let Some(d) = self.docs.documents.get(&doc) else {
            return;
        };
        let (t, rev) = (d.text(), d.revision());
        let Some(s) = self.find.search.as_mut() else {
            return;
        };
        // 記録が編集の前の文書のものでなければ (文書の差し替えの後など) 残さない
        let first = cs.changed_ranges().next().map(|(old, _)| old.start);
        if s.marks_rev != old_rev || !s.whole {
            s.resume.clear();
            s.canon = None;
            s.marks_rev = rev;
            return;
        }
        let Some(first) = first else {
            s.marks_rev = rev;
            return;
        };
        // 編集の始まりより前の位置は、編集の前後で同じ
        let first = first.min(ByteOff(t.len_bytes()));
        let line = t.byte_to_line(first).0;
        let cutoff = match s.lines {
            LineSpan::Bounded(k) => match line.checked_sub(k.saturating_add(1)) {
                Some(l) => t.line_to_byte(LineIdx(l)).0,
                None => 0,
            },
            // 上限なし (全文の連結) は、文書の全体を読むので文書の先頭だけが残る
            LineSpan::Unbounded => 0,
        };
        s.resume.retain(|p| p.0 <= cutoff);
        if s.canon.is_some_and(|c| c.end.0 > cutoff) {
            s.canon = None;
        }
        s.marks_rev = rev;
    }

    /// 次に検索の待ちの表示を切り替える時刻 (`Editor::tick` の期限)。
    pub(crate) fn search_wait_deadline(&self) -> Option<Tick> {
        let t = self.find.find_since?;
        let at = Tick(t.0.saturating_add(SEARCH_FIND_WAIT_MS));
        (self.now < at).then_some(at)
    }
}
