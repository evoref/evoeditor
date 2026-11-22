//! カーソルを足す操作 (docs/03 §4.5.1 の MCUR-1〜MCUR-6、07 §11.6 の「複数カーソルと対応括弧」、01 FR-211 AC-1 / AC-3)。
//!
//! - `select.add_next_match`: 主の Range の文字列の次の一致を足す (主が空なら単語を選ぶ)。同期検索で見つからなければ単発の
//!   ジョブ (`FindNext`) に回す (MCUR-2 (d))。
//! - `select.all_matches`: 一致をすべて選ぶ。1 MiB 以下の文書はメインスレッドで、超えれば全件の検索のジョブで探す (MCUR-3)。
//! - `select.add_cursor_above` / `select.add_cursor_below`: 一番上 / 一番下の Range の上下にカーソルを足す (MCUR-4)。
//!
//! どれも Motion で文書を変えず (MCUR-7)、足した Range を主にする (MCUR-1)。数が上限 (`MAX_CURSORS`) を超えるなら選択を
//! 変えずに知らせる (MCUR-6)。`ui.cancel` の「主の Range だけを残す」(MCUR-5) は `ui_model::overlay` の `ui_cancel`。
//!
//! spec: 03 §4.5.1, 03 §4.5.4, 03 §4.7.5

use eve_text::{ByteOff, ByteRange, MAX_CURSORS, Range, Selection, TextRead, word_at};
use eve_view::{DocumentId, MessageLevel, Revision, ViewId};

use crate::command::{
    CommandCtx, CommandKind, CommandRegistry, CommandResult, Permission, RejectReason, ids,
};
use crate::editor::state::EditorState;
use crate::io::pending::Pending;
use crate::jobs::cancel::CancelToken;
use crate::jobs::{JobKind, JobResult};
use crate::layout::Affinity;
use crate::search::search_state::Found;
use crate::search::{Case, Prepared, ScanCtx, ScanEnd, SearchQuery, scan_range};
use crate::ui_model::keys;
use crate::view::motion;

/// カーソルの位置の単語を探す窓の上限 (バイト。長大行で行の全体を読まない。単語移動の窓と同じ、03 §11.5.1)。
const WORD_WINDOW: usize = 64 * 1024;

/// `select.all_matches` をメインスレッドで探す文書の大きさの上限 (バイト、03 §4.5.1 の MCUR-3)。超えれば `bg` の全件の
/// 検索のジョブで探す。
pub(crate) const ALL_MATCHES_SYNC_MAX_BYTES: usize = 1 << 20;

/// 主の Range の文字列を検索語にする長さの上限 (バイト、03 §4.5.1 の MCUR-2 (k))。超えたら何もせずに知らせる (大きな選択を
/// メインスレッドで文字列に写さない)。単語の窓 (`WORD_WINDOW`) と同じ大きさ。
pub(crate) const MATCH_QUERY_MAX_BYTES: usize = 64 * 1024;

/// 「単語の一致」の印 (03 §4.5.1 の MCUR-2 (a) / (e))。印を立てた (足す操作・`ui.cancel` で更新した) ときの文書の Revision と
/// 選択を持ち、どちらかが変わっていたら (移動・クリック・編集・Undo など、選択を変えるほかの操作の後) 印は無いものとする。
#[derive(Clone, Debug)]
pub(crate) struct WordMark {
    revision: Revision,
    selection: Selection,
}

/// `select.add_next_match` の次の一致を探す途中 (同期検索と単発のジョブで引き継ぐ、03 §4.5.1 の MCUR-2 (b) / (d))。
#[derive(Debug)]
pub(crate) struct NextMatchAsk {
    doc: DocumentId,
    view: ViewId,
    /// 投入したときの文書の Revision と選択 (結果が届いた時点で違えば捨てる)。
    revision: Revision,
    selection: Selection,
    /// 主の Range の文字列のリテラル (大小を区別する。単語の一致なら単語単位)。
    query: SearchQuery,
    /// 単語の一致の印 (足した後も保つ)。
    word: bool,
    /// 主の Range の `start` (文書の末尾から先頭へ回った後、ここで止める)。
    origin: ByteOff,
    /// 次に探し始める位置。
    cursor: ByteOff,
    /// 文書の末尾から先頭へ回った。
    wrapped: bool,
}

/// `select.all_matches` の全件の検索のジョブの宛先 (03 §4.5.1 の MCUR-3)。一致は束 (`SearchBatch`) ごとに貯める。
#[derive(Debug)]
pub(crate) struct AllMatchesAsk {
    doc: DocumentId,
    view: ViewId,
    revision: Revision,
    selection: Selection,
    /// 検索語の範囲 (主の Range か、主のカーソルの位置の単語)。主にする一致を決めるのに使う。
    term: ByteRange,
    /// 単語単位で探した (単語の一致の印を立てる)。
    word: bool,
    /// 届いた一致。
    matches: Vec<ByteRange>,
}

impl AllMatchesAsk {
    /// 探している文書。
    pub(crate) fn doc(&self) -> DocumentId {
        self.doc
    }
}

/// 見つけた一致の扱い。
enum Judge {
    /// 足す。
    Take,
    /// 主の Range の始まりまで戻った (足せる一致が無い)。
    Stop,
    /// 既にある Range と重なる・接するので飛ばし、その後ろから探し続ける。
    Skip,
}

/// 次の一致の探し方の結果。
enum Next {
    Hit(ByteRange),
    None,
    /// 同期検索の範囲では決まらない (単発のジョブに回す)。
    Job,
}

/// `r` が空でない既存の Range と重なるか接する (足すと併合されて、検索語の範囲が変わる。03 §4.4 の N3)。
fn conflicts(sel: &Selection, r: ByteRange) -> bool {
    let rs = sel.ranges();
    let k = rs.partition_point(|x| x.end() < r.start);
    rs[k..]
        .iter()
        .take_while(|x| x.start() <= r.end)
        .any(|x| !x.is_empty())
}

impl NextMatchAsk {
    /// 探している文書。
    pub(crate) fn doc(&self) -> DocumentId {
        self.doc
    }

    /// 前向きの検索で `cursor` から見つけた一致 `r` (`w` は文書の末尾から先頭へ回った) を判定する。飛ばすときは、次に探す位置を
    /// 一致の終わりへ進める。
    fn judge(&mut self, r: ByteRange, w: bool) -> Judge {
        if self.wrapped && w {
            return Judge::Stop;
        }
        let wrapped = self.wrapped || w;
        if wrapped && r.start >= self.origin {
            return Judge::Stop;
        }
        if conflicts(&self.selection, r) {
            self.cursor = r.end;
            self.wrapped = wrapped;
            return Judge::Skip;
        }
        Judge::Take
    }
}

/// 主の Range (空でない) を検索語にできるか (03 §4.5.1 の MCUR-2 (k))。
enum QueryFit {
    Fits,
    /// 文書の全体 (ほかに一致は無い。何もしない)。
    WholeDocument,
    /// `MATCH_QUERY_MAX_BYTES` を超える (知らせる)。
    TooLong,
}

fn query_fit(primary: Range, len: usize) -> QueryFit {
    if primary.start().0 == 0 && primary.end().0 >= len {
        QueryFit::WholeDocument
    } else if primary.end().0 - primary.start().0 > MATCH_QUERY_MAX_BYTES {
        QueryFit::TooLong
    } else {
        QueryFit::Fits
    }
}

/// 検索語のリテラル (03 §4.5.4: リテラル・大小を区別。検索バーのトグルと CP932 互換文字の同一視の設定に依らない)。
fn literal_query(text: &str, word: bool) -> SearchQuery {
    SearchQuery::literal(text)
        .with_case(Case::Sensitive)
        .with_compat_fold(false)
        .with_whole_word(word)
}

impl EditorState {
    /// View `view` に「単語の一致」の印があり、今も有効 (印を立てた後に文書も選択も変わっていない)。
    pub(crate) fn word_mark_valid(&self, view: ViewId) -> bool {
        let Some(v) = self.view(view) else {
            return false;
        };
        let Some(d) = self.document(v.doc) else {
            return false;
        };
        v.word_match
            .as_ref()
            .is_some_and(|m| m.revision == d.revision() && m.selection == v.selection)
    }

    /// 足す操作・`ui.cancel` の後に「単語の一致」の印を立てる (`on`) か外す。立てるときは今の文書と選択を覚える。
    pub(crate) fn set_word_mark(&mut self, view: ViewId, on: bool) {
        let Some(v) = self.docs.views.get(&view) else {
            return;
        };
        let mark = match (on, self.docs.documents.get(&v.doc)) {
            (true, Some(d)) => Some(WordMark {
                revision: d.revision(),
                selection: v.selection.clone(),
            }),
            _ => None,
        };
        if let Some(v) = self.docs.views.get_mut(&view) {
            v.word_match = mark;
        }
    }

    /// `select.add_next_match` (03 §4.5.1 の MCUR-2)。
    fn add_next_match(&mut self, view: ViewId, doc: DocumentId) -> CommandResult {
        let (Some(d), Some(v)) = (self.document(doc), self.view(view)) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let sel = v.selection.clone();
        let primary = *sel.primary();
        let t = d.text();
        if primary.is_empty() {
            // (a) / (c): 主のカーソルの位置の単語を選ぶ。全部が空なら、各カーソルの位置の単語をそれぞれ選ぶ
            let Some(w) = word_at(t, primary.head, WORD_WINDOW) else {
                self.show_message(MessageLevel::Info, keys::INFO_SELECT_NO_WORD, &[]);
                return CommandResult::Done;
            };
            let mut out: Vec<Range> = sel.ranges().to_vec();
            if sel.iter().all(Range::is_empty) {
                // 同じ単語の中の続くカーソルは、前のカーソルの単語を使う (同じ長い単語をカーソルごとに読まない)
                let mut last: Option<ByteRange> = None;
                for r in &mut out {
                    let found = match last {
                        Some(l) if l.start <= r.head && r.head < l.end => Some(l),
                        _ => word_at(t, r.head, WORD_WINDOW),
                    };
                    if let Some(f) = found {
                        last = Some(f);
                        *r = Range::new(f.start, f.end);
                    }
                }
            } else {
                out[sel.primary_index()] = Range::new(w.start, w.end);
            }
            let Some(new) = Selection::new(out, sel.primary_index()) else {
                return CommandResult::Rejected(RejectReason::InvalidArgs);
            };
            self.set_selection(view, new);
            self.set_word_mark(view, true);
            self.ensure_cursor_visible(view);
            return CommandResult::Done;
        }
        // (b): 主の Range の文字列の次の一致を足す (上限は、足した後の併合の後の数で調べる。`add_match`)
        match query_fit(primary, t.len_bytes()) {
            QueryFit::Fits => {}
            QueryFit::WholeDocument => return CommandResult::Done,
            QueryFit::TooLong => {
                self.show_message(MessageLevel::Info, keys::INFO_SELECT_QUERY_TOO_LONG, &[]);
                return CommandResult::Done;
            }
        }
        let word = self.word_mark_valid(view);
        let text = t.slice_to_string(ByteRange::new(primary.start(), primary.end()));
        let ask = NextMatchAsk {
            doc,
            view,
            revision: d.revision(),
            selection: sel,
            query: literal_query(&text, word),
            word,
            origin: primary.start(),
            cursor: primary.end(),
            wrapped: false,
        };
        self.continue_next_match(ask);
        CommandResult::Done
    }

    /// `ask` の続きを探す (同期検索で決まればその場で足すか知らせ、決まらなければ単発のジョブを投げる)。
    fn continue_next_match(&mut self, mut ask: NextMatchAsk) {
        match self.next_match_sync(&mut ask) {
            Next::Hit(r) => self.add_match(&ask, r),
            Next::None => {
                self.show_message(MessageLevel::Info, keys::INFO_SELECT_NO_MORE_MATCHES, &[])
            }
            Next::Job => self.spawn_next_match(ask),
        }
    }

    /// 同期検索で次の一致を探す (`search.next` と同じ予算: 256 KiB、行ごとに切り出す経路は 1/16。既存の Range と重なる
    /// 一致を飛ばして探し続ける間の全部の呼び出しで、1 回分の予算を分け合う。03 §4.5.1 の MCUR-2 (d) / (i))。
    fn next_match_sync(&self, ask: &mut NextMatchAsk) -> Next {
        // 行をまたぐ検索語・重い検索語は同期検索をしない (03 §12.3.1)
        if ask.query.shape().is_some_and(|s| s.job_driven()) {
            return Next::Job;
        }
        let mut budget: Option<usize> = None;
        loop {
            let found = self.sync_find_within(ask.doc, &ask.query, ask.cursor, true, &mut budget);
            let (r, w) = match found {
                Found::Hit(r, w) => (r, w),
                Found::None => return Next::None,
                Found::Incomplete => return Next::Job,
            };
            match ask.judge(r, w) {
                Judge::Take => return Next::Hit(r),
                Judge::Stop => return Next::None,
                Judge::Skip => {}
            }
        }
    }

    /// 単発のジョブで次の一致を探す (`find_job` を `search.next` / `search.prev` と共有し、古い要求を取り消す。走っている全件の
    /// 走査は取り消して後ろに投げ直す。03 §4.5.1 の MCUR-2 (d))。
    fn spawn_next_match(&mut self, ask: NextMatchAsk) {
        self.cancel_find_next();
        let Some(d) = self.document(ask.doc) else {
            return;
        };
        let (doc, revision, snapshot) = (ask.doc, d.revision(), d.snapshot());
        let (query, from) = (ask.query.clone(), ask.cursor);
        self.yield_search(|st| {
            let job = st.jobs.spawn(
                Some(doc),
                revision,
                JobKind::FindNext {
                    snapshot,
                    query,
                    from,
                    forward: true,
                    resume: Vec::new(),
                },
            );
            st.io
                .pending
                .insert(job, Pending::AddNextMatch(Box::new(ask)));
            st.find.find_job = Some(job);
        });
    }

    /// `select.add_next_match` の単発のジョブの結果。文書の Revision と選択が投入時のままなら判定し、足すか、続きを探す。
    pub(crate) fn finish_add_next_match(&mut self, mut ask: NextMatchAsk, r: JobResult) {
        self.find.find_job = None;
        let found = match r {
            JobResult::FindNextDone { found } => found,
            JobResult::SearchLimited { limit } => {
                self.show_limit_message(limit);
                return;
            }
            _ => return,
        };
        let same = self
            .document(ask.doc)
            .is_some_and(|d| d.revision() == ask.revision)
            && self
                .view(ask.view)
                .is_some_and(|v| v.doc == ask.doc && v.selection == ask.selection);
        if !same {
            return;
        }
        let Some((m, w)) = found else {
            self.show_message(MessageLevel::Info, keys::INFO_SELECT_NO_MORE_MATCHES, &[]);
            return;
        };
        match ask.judge(m, w) {
            Judge::Take => self.add_match(&ask, m),
            Judge::Stop => {
                self.show_message(MessageLevel::Info, keys::INFO_SELECT_NO_MORE_MATCHES, &[]);
            }
            Judge::Skip => self.continue_next_match(ask),
        }
    }

    /// 一致 `m` を新しい主の Range として足す (MCUR-1)。上限を超えるなら選択を変えずに知らせる (MCUR-6)。
    fn add_match(&mut self, ask: &NextMatchAsk, m: ByteRange) {
        let (Some(d), Some(v)) = (self.document(ask.doc), self.view(ask.view)) else {
            return;
        };
        let Some(new) = v
            .selection
            .with_new_primary(Range::new(m.start, m.end), d.text())
        else {
            self.notify_cursor_limit();
            return;
        };
        self.set_selection(ask.view, new);
        self.set_word_mark(ask.view, ask.word);
        self.ensure_cursor_visible(ask.view);
    }

    /// `select.all_matches` (03 §4.5.1 の MCUR-3)。
    fn all_matches(&mut self, view: ViewId, doc: DocumentId) -> CommandResult {
        let (Some(d), Some(v)) = (self.document(doc), self.view(view)) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let sel = v.selection.clone();
        let primary = *sel.primary();
        let t = d.text();
        let (term, word) = if primary.is_empty() {
            match word_at(t, primary.head, WORD_WINDOW) {
                Some(w) => (w, true),
                None => {
                    self.show_message(MessageLevel::Info, keys::INFO_SELECT_NO_WORD, &[]);
                    return CommandResult::Done;
                }
            }
        } else {
            match query_fit(primary, t.len_bytes()) {
                QueryFit::Fits => {}
                QueryFit::WholeDocument => return CommandResult::Done,
                QueryFit::TooLong => {
                    self.show_message(MessageLevel::Info, keys::INFO_SELECT_QUERY_TOO_LONG, &[]);
                    return CommandResult::Done;
                }
            }
            (
                ByteRange::new(primary.start(), primary.end()),
                self.word_mark_valid(view),
            )
        };
        let query = literal_query(&t.slice_to_string(term), word);
        let mut ask = AllMatchesAsk {
            doc,
            view,
            revision: d.revision(),
            selection: sel,
            term,
            word,
            matches: Vec::new(),
        };
        let len = t.len_bytes();
        let ready = match query.prepared() {
            Prepared::Ready(m) if len <= ALL_MATCHES_SYNC_MAX_BYTES => Some(m),
            _ => None,
        };
        let Some(m) = ready else {
            self.spawn_all_matches(ask, query);
            return CommandResult::Done;
        };
        // 小さな文書はメインスレッドで探す。上限 + 1 件で止める (それより先は数えない)
        let cancel = CancelToken::new();
        let mut found: Vec<ByteRange> = Vec::new();
        let end = scan_range(
            t,
            &m,
            ByteRange::new(ByteOff(0), ByteOff(len)),
            &mut ScanCtx::new(&cancel),
            &mut |r| {
                found.push(r);
                found.len() <= MAX_CURSORS
            },
        );
        match end {
            ScanEnd::Done | ScanEnd::Stopped => {}
            e => {
                if let Some(l) = e.limit() {
                    self.show_limit_message(l);
                }
                return CommandResult::Done;
            }
        }
        ask.matches = found;
        let over = ask.matches.len() > MAX_CURSORS;
        self.apply_all_matches(ask, over);
        CommandResult::Done
    }

    /// `select.all_matches` を全件の検索のジョブで探す (02 §9.3 の `JobKind::Search`。保持と数える上限を上限 + 1 にする)。
    /// 前の `select.all_matches` のジョブだけを取り消す (検索バーの全件の走査は取り消さない)。
    fn spawn_all_matches(&mut self, ask: AllMatchesAsk, query: SearchQuery) {
        self.cancel_all_matches();
        let Some(d) = self.document(ask.doc) else {
            return;
        };
        let (doc, revision, snapshot) = (ask.doc, d.revision(), d.snapshot());
        let limit = MAX_CURSORS + 1;
        let job = self.jobs.spawn(
            Some(doc),
            revision,
            JobKind::Search {
                snapshot,
                query,
                from: ByteOff(0),
                max_matches: limit,
                count_limit: u64::try_from(limit).unwrap_or(u64::MAX),
            },
        );
        self.io
            .pending
            .insert(job, Pending::SelectAllMatches(Box::new(ask)));
        self.find.select_all_job = Some(job);
    }

    /// 走っている `select.all_matches` のジョブを取り消す。
    fn cancel_all_matches(&mut self) {
        if let Some(old) = self.find.select_all_job.take() {
            self.jobs.cancel(old);
            self.io.pending.remove(&old);
        }
    }

    /// `select.all_matches` のジョブの途中経過と終わり (束は貯め、上限を超えたらジョブを取り消して終える)。
    pub(crate) fn on_all_matches_result(&mut self, job: eve_view::JobId, r: JobResult) {
        if let JobResult::SearchBatch { matches, .. } = r {
            let over = match self.io.pending.get_mut(&job) {
                Some(Pending::SelectAllMatches(ask)) => {
                    ask.matches.extend(matches);
                    ask.matches.len() > MAX_CURSORS
                }
                _ => return,
            };
            if over && let Some(Pending::SelectAllMatches(ask)) = self.io.pending.remove(&job) {
                self.jobs.cancel(job);
                self.find.select_all_job = None;
                self.apply_all_matches(*ask, true);
            }
            return;
        }
        let Some(Pending::SelectAllMatches(ask)) = self.io.pending.remove(&job) else {
            return;
        };
        if self.find.select_all_job == Some(job) {
            self.find.select_all_job = None;
        }
        // 失敗・取り消し (`SearchDone` 以外の終わり) は選択を変えない
        let JobResult::SearchDone {
            total,
            count_limited,
            truncated,
            limit,
            ..
        } = r
        else {
            return;
        };
        if let Some(l) = limit {
            self.show_limit_message(l);
            return;
        }
        let over =
            count_limited || truncated || usize::try_from(total).map_or(true, |n| n > MAX_CURSORS);
        self.apply_all_matches(*ask, over);
    }

    /// 一致の全部で今の Range を置き換える (文書の Revision と選択が始めたときのままのときだけ)。上限を超えるなら選択を変えずに
    /// 知らせる (一部だけを選ばない)。主は、検索語の範囲と重なる一致 (無ければその後ろの最初、それも無ければ最後)。
    fn apply_all_matches(&mut self, ask: AllMatchesAsk, over: bool) {
        let same = self
            .document(ask.doc)
            .is_some_and(|d| d.revision() == ask.revision)
            && self
                .view(ask.view)
                .is_some_and(|v| v.doc == ask.doc && v.selection == ask.selection);
        if !same {
            return;
        }
        if over {
            let limit = MAX_CURSORS.to_string();
            self.show_message(
                MessageLevel::Info,
                keys::INFO_SELECT_TOO_MANY_MATCHES,
                &[("limit", &limit)],
            );
            return;
        }
        let mut ms = ask.matches;
        ms.sort_unstable_by_key(|m| (m.start, m.end));
        ms.dedup();
        let term = ask.term;
        let primary = ms
            .iter()
            .position(|m| m.start < term.end && term.start < m.end)
            .or_else(|| ms.iter().position(|m| m.start >= term.end))
            .unwrap_or(ms.len().saturating_sub(1));
        let ranges: Vec<Range> = ms.iter().map(|m| Range::new(m.start, m.end)).collect();
        let Some(new) = Selection::new(ranges, primary) else {
            // 一致が無い (検索語は文書から取ったので、ふつうは自分自身に一致する)
            self.show_message(MessageLevel::Info, keys::INFO_SELECT_NO_MORE_MATCHES, &[]);
            return;
        };
        self.set_selection(ask.view, new);
        self.set_word_mark(ask.view, ask.word);
        self.ensure_cursor_visible(ask.view);
    }

    /// `select.add_cursor_above` / `select.add_cursor_below` (03 §4.5.1 の MCUR-4)。
    fn add_cursor_vertical(&mut self, view: ViewId, doc: DocumentId, up: bool) -> CommandResult {
        let wrap = crate::layout::wrap_ctx(self, view);
        let (Some(d), Some(v)) = (self.document(doc), self.view(view)) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let sel = &v.selection;
        let (from_index, from) = if up {
            (0, sel.ranges()[0])
        } else {
            (sel.len() - 1, sel.ranges()[sel.len() - 1])
        };
        // 目標の列は View のもの (主の Range の列)。無ければ起点の表示列で埋めてから使い、足した後も保つ
        let mut goal = v.goal_dcol;
        let mut aff = if from_index == sel.primary_index() {
            v.cursor_affinity
        } else {
            Affinity::After
        };
        let Some(p) = motion::vertical_neighbor(d, &mut goal, &mut aff, from.head, up, wrap) else {
            // 文書の先頭の行 / 最後の行からは何もしない (知らせない)
            return CommandResult::Done;
        };
        let Some(new) = sel.with_new_primary(Range::point(p), d.text()) else {
            self.notify_cursor_limit();
            return CommandResult::Done;
        };
        let word = self.word_mark_valid(view);
        self.set_selection_with_goal(view, new, goal);
        self.set_cursor_affinity(view, aff);
        self.set_word_mark(view, word);
        self.ensure_cursor_visible(view);
        CommandResult::Done
    }

    /// カーソルの数の上限を超える操作をしなかったことを知らせる (`info.cursor.limit`、MCUR-6)。
    fn notify_cursor_limit(&mut self) {
        let limit = MAX_CURSORS.to_string();
        self.show_message(
            MessageLevel::Info,
            keys::INFO_CURSOR_LIMIT,
            &[("limit", &limit)],
        );
    }
}

fn run(
    ctx: &mut CommandCtx<'_>,
    f: impl FnOnce(&mut EditorState, ViewId, DocumentId) -> CommandResult,
) -> CommandResult {
    match motion::active(ctx) {
        Some((view, doc)) => f(ctx.state, view, doc),
        None => CommandResult::Rejected(RejectReason::NoTarget),
    }
}

fn add_next_match(ctx: &mut CommandCtx<'_>) -> CommandResult {
    run(ctx, EditorState::add_next_match)
}

fn all_matches(ctx: &mut CommandCtx<'_>) -> CommandResult {
    run(ctx, EditorState::all_matches)
}

fn add_cursor_above(ctx: &mut CommandCtx<'_>) -> CommandResult {
    run(ctx, |st, v, d| st.add_cursor_vertical(v, d, true))
}

fn add_cursor_below(ctx: &mut CommandCtx<'_>) -> CommandResult {
    run(ctx, |st, v, d| st.add_cursor_vertical(v, d, false))
}

/// v0.5 の M32 のカーソルを足すコマンド (07 §11.6 の「複数カーソルと対応括弧」。どれも Motion・`read`・`when` なし)。
pub(crate) fn register(r: &mut CommandRegistry) {
    for (id, h) in [
        (
            ids::SELECT_ADD_CURSOR_ABOVE,
            add_cursor_above as crate::Handler,
        ),
        (ids::SELECT_ADD_CURSOR_BELOW, add_cursor_below),
        (ids::SELECT_ADD_NEXT_MATCH, add_next_match),
        (ids::SELECT_ALL_MATCHES, all_matches),
    ] {
        r.add(id, CommandKind::Motion, Permission::Read, "", h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(rs: &[(usize, usize)]) -> Selection {
        Selection::new(
            rs.iter()
                .map(|&(a, h)| Range::new(ByteOff(a), ByteOff(h)))
                .collect(),
            0,
        )
        .expect("Selection")
    }

    fn br(a: usize, b: usize) -> ByteRange {
        ByteRange::new(ByteOff(a), ByteOff(b))
    }

    #[test]
    fn 空でない_range_と重なる_接する一致は飛ばし_空のカーソルは数えない() {
        let s = sel(&[(2, 4), (8, 8), (12, 15)]);
        assert!(conflicts(&s, br(3, 6)), "重なる");
        assert!(conflicts(&s, br(4, 6)), "end に接する");
        assert!(conflicts(&s, br(0, 2)), "start に接する");
        assert!(conflicts(&s, br(10, 12)), "後ろの Range の start に接する");
        assert!(!conflicts(&s, br(5, 7)), "離れている");
        assert!(!conflicts(&s, br(7, 9)), "空のカーソルは含んでも重ならない");
        assert!(!conflicts(&s, br(16, 20)), "最後の Range の後ろ");
    }

    #[test]
    fn 末尾から先頭へ回った後は主の始まりで止まる() {
        let mut ask = NextMatchAsk {
            doc: DocumentId(1),
            view: ViewId(1),
            revision: Revision(0),
            selection: sel(&[(10, 13)]),
            query: literal_query("abc", false),
            word: false,
            origin: ByteOff(10),
            cursor: ByteOff(13),
            wrapped: false,
        };
        assert!(matches!(ask.judge(br(20, 23), false), Judge::Take));
        // 先頭へ回って、主の前にある一致は足す
        assert!(matches!(ask.judge(br(0, 3), true), Judge::Take));
        // 主そのもの (回った後に主の始まり以降) は足さない
        assert!(matches!(ask.judge(br(10, 13), true), Judge::Stop));
        // 飛ばしたら、その終わりから探し続け、回ったことを覚える
        let mut ask = NextMatchAsk {
            selection: sel(&[(0, 3), (10, 13)]),
            ..ask
        };
        assert!(matches!(ask.judge(br(0, 3), true), Judge::Skip));
        assert_eq!((ask.cursor, ask.wrapped), (ByteOff(3), true));
        // 2 回目に末尾を越えたら止める
        assert!(matches!(ask.judge(br(4, 7), true), Judge::Stop));
    }
}
