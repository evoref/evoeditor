//! 検索バー (docs/04 §8.4、03 §12、01 FR-401〜404)。
//!
//! 開いている間は編集面を 1 行縮め、キー入力はバーの入力欄へ行く (コンテキスト `search`)。入力のたびに検索をやり直し、
//! 最初の一致 (開いたときの位置から先) を **強調するだけで選択はしない**。`Enter` / `↓` / `F3` で一致が主選択になる (確定)。
//! `Esc` で閉じると、確定していない一致は選択として残さない。

use eve_text::{ByteOff, ByteRange, LineIdx, Selection, TextRead};
use eve_view::{
    BarFocus, CaseMode, CutPolicy, InputFieldModel, MessageLevel, ReplaceRowModel, SearchBarModel,
};

use crate::command::CommandResult;
use crate::command::ids;
use crate::command::insert_rule::{self, EscapeContext, InsertNotice};
use crate::document::{Origin, Transaction, UndoGroupHint};
use crate::jobs::cancel::CancelToken;
use crate::jobs::{JobKind, JobResult};
use crate::search::{
    Case, LineSpan, Prepared, QueryError, SEARCH_CONCAT_MAX_BYTES, SEARCH_SYNC_MULTILINE_MAX,
    ScanCtx, ScanEnd, ScanLimit, SearchQuery, SearchTuning, Shape, build_replace_all_in,
    replace_one, replacement_for, scan_bytes, scan_range, unescape_replacement,
};
use crate::time::Tick;
use crate::ui_model::keys;

use crate::editor::state::EditorState;
use crate::io::pending::Pending;
use crate::layout::cell_text;
use crate::search::search_state::{Found, SearchCount, SearchStep};

impl SearchBarState {
    /// 今の入力とトグルから作る問い合わせ。条件が前と同じなら前のもの (コンパイル済みの照合器を共有する) を返す (PRF-09)。
    pub(crate) fn query(&self) -> SearchQuery {
        let q = SearchQuery::literal(&self.text)
            .with_case(self.case)
            .with_regex(self.regex)
            .with_whole_word(self.whole_word)
            .with_scope(self.in_selection.then(|| self.sel_ranges.clone()))
            .with_newline_sugar(self.newline_sugar)
            .with_compat_fold(self.compat_fold)
            .tuned(self.tuning);
        let mut last = self.memo.query.borrow_mut();
        match &*last {
            Some(prev) if *prev == q => prev.clone(),
            _ => {
                *last = Some(q.clone());
                q
            }
        }
    }
}

/// 可視行の一致 (文書のバイト位置の半開区間、開始位置の昇順)。
pub(crate) type VisibleMatches = std::rc::Rc<[(usize, usize)]>;

/// 可視行の一致の覚えのキー (文書, Revision, 先頭の行, 行数, 問い合わせ)。
type VisibleKey = (
    eve_view::DocumentId,
    eve_view::Revision,
    usize,
    usize,
    SearchQuery,
);

/// 検索バーの、Frame を作るたびに同じ結果になる計算の覚え (PRF-09 / REF-04)。
#[derive(Clone, Debug, Default)]
pub(crate) struct SearchBarMemo {
    /// 最後に作った問い合わせ (コンパイル済みの照合器を持つ)。
    query: std::cell::RefCell<Option<SearchQuery>>,
    /// 最後に求めた可視行の一致。検索の途中経過の再描画などで、条件が同じ間は可視行を走査し直さない。
    visible: std::cell::RefCell<Option<(VisibleKey, VisibleMatches)>>,
    /// 最後に確かめた「今の一致が同一視で当たったものか」(件数の印、03 §12.4.1)。キーは (文書, Revision, 一致, 問い合わせ)。
    folded: std::cell::RefCell<Option<(FoldedKey, bool)>>,
}

/// 今の一致が同一視で当たったものかの覚えのキー。
type FoldedKey = (
    eve_view::DocumentId,
    eve_view::Revision,
    ByteRange,
    SearchQuery,
);

/// 可視行の強調で、1 行あたり検索する上限 (`SEARCH_SYNC_LINE_MAX`、03 §12.2)。これを超える行は強調しない。
const SYNC_LINE_MAX: usize = 16 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct SearchBarState {
    pub text: String,
    /// `text` 内のカーソル (バイト。書記素境界)。
    pub cursor: usize,
    pub case: Case,
    /// 正規表現として探す (`search.toggle_regex`)。バーを開くたびに off から始まる。
    pub regex: bool,
    /// 単語単位 (`search.toggle_word`)。同じく off から始まる。
    pub whole_word: bool,
    /// 選択範囲内だけを探す (`search.toggle_selection`)。off から始まる。
    pub in_selection: bool,
    /// バーを開いたときの選択 (空でない Range。開始位置の昇順)。編集に合わせて写像する (03 §12.5)。
    pub sel_ranges: std::sync::Arc<[ByteRange]>,
    /// 検索語をコンパイルできない理由。あれば検索しない (03 §12.3)。
    pub error: Option<QueryError>,
    /// 探さない理由 (行の数に上限の無い検索語と 16 MiB を超える文書・重すぎて打ち切った、03 §12.3.1 の「探さない場合」と
    /// 「最悪の入力の上限」)。構文の誤り (`error`) とは別の項目で、件数の位置に同じく示す。
    pub refusal: Option<ScanLimit>,
    /// 正規表現の `\n` の糖衣 (`search.newline_sugar`。検索バーを開いたときの設定の値、03 §12.3.1 の「型と状態の持ち主」)。
    pub newline_sugar: bool,
    /// CP932 互換文字の同一視 (`search.cp932_compat_fold`。検索バーを開いたときの設定の値、03 §12.4.1)。
    pub compat_fold: bool,
    /// 保持する一致の上限 (`search.max_matches`。検索バーを開いたときの設定の値、03 §12.2)。
    pub max_matches: usize,
    /// 数える一致の上限 (`search.count_limit`。検索バーを開いたときの設定の値、03 §12.2)。
    pub count_limit: u64,
    /// 行の窓の核の大きさと連結の上限 (既定は定数。試験だけが小さくする、`SearchTuning`)。
    pub tuning: SearchTuning,
    /// 入力しているときの検索の起点 (開いたときの選択の始端。一致へ移ったらその一致の始端)。
    pub anchor: ByteOff,
    /// 入力で見つかった、まだ選択していない現在の一致。
    pub current: Option<ByteRange>,
    /// 入力中の現在の一致を、正規表現の照合器ができて (`bg` のジョブの最初の結果が届いて) から求める (03 §12.3.1)。
    pub current_pending: bool,
    /// 置換欄 (開いているとき。テキストとカーソル)。
    pub replace: Option<(String, usize)>,
    pub focus: BarFocus,
    /// フォーカスのある欄 (検索欄・置換欄) の欄内全選択 (`select.all`) の最中。編集・移動・フォーカスの移動で解ける。
    pub all_selected: bool,
    /// Frame を作るたびの計算の覚え。
    pub memo: SearchBarMemo,
}

/// この件数以上のすべて置換は、Undo の記録を 1 件の置き換えにしてよい (`REPLACE_COARSE_UNDO_MIN`、03 §12.5)。
const REPLACE_COARSE_UNDO_MIN: u64 = 10_000;

/// すべて置換をジョブで作るときの境界 (`SEARCH_SYNC_STEP_BYTES`、03 §12.2)。これ以下の文書はメインスレッドで作る。
const REPLACE_SYNC_BYTES: usize = 1 << 20;

impl EditorState {
    /// 検索バーが開いているか (条件式 `search.active`)。
    pub fn search_bar_open(&self) -> bool {
        self.find.search_bar.is_some()
    }

    /// 行の窓の核の大きさと連結の上限を試験から小さくする (feature `test-hooks` の口。配布ビルドには無い。既定の値より
    /// 大きくはしない、`SearchTuning`)。開いている検索バーにも効く (次の再検索から)。
    #[cfg(feature = "test-hooks")]
    #[doc(hidden)]
    pub fn set_search_tuning(&mut self, tuning: SearchTuning) {
        let tuning = tuning.clamped();
        self.find.tuning = tuning;
        if let Some(bar) = &mut self.find.search_bar {
            bar.tuning = tuning;
        }
    }

    /// 検索バーを開く。開いていれば何もしない (欄はそのまま)。
    pub fn search_open(&mut self) -> CommandResult {
        if self.find.search_bar.is_some() {
            return CommandResult::Done;
        }
        let anchor = self.bar_cursor(false).unwrap_or(ByteOff(0));
        let sel_ranges: std::sync::Arc<[ByteRange]> = self
            .focused_window()
            .and_then(|w| self.window_view(w))
            .and_then(|v| self.view(v))
            .map(|v| {
                v.selection
                    .iter()
                    .filter(|r| !r.is_empty())
                    .map(|r| ByteRange::new(r.start(), r.end()))
                    .collect()
            })
            .unwrap_or_else(|| std::sync::Arc::from([]));
        self.find.search_bar = Some(SearchBarState {
            text: String::new(),
            cursor: 0,
            case: match self.config().search_case {
                eve_config::SearchCase::Smart => Case::Smart,
                eve_config::SearchCase::Sensitive => Case::Sensitive,
                eve_config::SearchCase::Insensitive => Case::Insensitive,
            },
            regex: self.config().regex_default,
            whole_word: false,
            in_selection: false,
            sel_ranges,
            error: None,
            refusal: None,
            // 設定の変更は次に検索バーを開いたときから効く (03 §12.3.1 の「型と状態の持ち主」)
            newline_sugar: self.config().newline_sugar,
            compat_fold: self.config().cp932_compat_fold,
            max_matches: usize::try_from(self.config().max_matches).unwrap_or(usize::MAX),
            count_limit: self.config().count_limit,
            tuning: self.find.tuning,
            anchor,
            current: None,
            current_pending: false,
            replace: None,
            focus: BarFocus::Search,
            all_selected: false,
            memo: SearchBarMemo::default(),
        });
        self.set_bar_rows();
        CommandResult::Done
    }

    /// `search.replace_open`: 置換欄つきで開く (開いていれば置換欄を足す)。読み取り専用の文書では開かない。
    pub fn search_replace_open(&mut self) -> CommandResult {
        let read_only = self.active_document().is_some_and(|d| {
            self.docs
                .documents
                .get(&d)
                .is_some_and(|d| !d.is_editable())
        });
        if read_only {
            self.notify_read_only();
            return CommandResult::Done;
        }
        self.search_open();
        if let Some(bar) = &mut self.find.search_bar {
            if bar.replace.is_none() {
                bar.replace = Some((String::new(), 0));
            }
            // 検索語が空なら検索欄、あれば置換欄にフォーカスを置く
            bar.all_selected = false;
            bar.focus = if bar.text.is_empty() {
                BarFocus::Search
            } else {
                BarFocus::Replace
            };
        }
        self.set_bar_rows();
        CommandResult::Done
    }

    /// `Tab` / `Shift+Tab`: 検索欄 → 置換欄 → ボタン → 検索欄 (置換欄が開いているときだけ)。
    pub(crate) fn bar_focus_cycle(&mut self, forward: bool) {
        let Some(bar) = &mut self.find.search_bar else {
            return;
        };
        if bar.replace.is_none() {
            return;
        }
        let order = [BarFocus::Search, BarFocus::Replace, BarFocus::Button];
        let i = order.iter().position(|f| *f == bar.focus).unwrap_or(0);
        bar.focus = order[if forward { (i + 1) % 3 } else { (i + 2) % 3 }];
        bar.all_selected = false;
        self.mark_dirty_render();
    }

    /// 入力欄を編集した: 検索欄なら検索をやり直す。
    pub(crate) fn bar_edited(&mut self) {
        if self
            .find
            .search_bar
            .as_ref()
            .is_some_and(|b| b.focus == BarFocus::Search)
        {
            self.bar_changed();
        } else {
            // 置換の文字列が変わった: 待っているすべて置換・1 件置換の確かめは古い条件のものなので取り消す
            self.cancel_replace_job();
            self.mark_dirty_render();
        }
    }

    /// 検索バーを閉じる。確定していない現在の一致は選択として残さない。
    pub fn search_close(&mut self) {
        if self.find.search_bar.take().is_none() {
            return;
        }
        self.search_stop();
        self.cancel_replace_job();
        self.set_bar_rows();
    }

    /// 検索バーの行数 (検索欄 1 行 + 置換欄があれば 1 行、閉じていれば 0)。編集面はその分だけ縮む。
    fn set_bar_rows(&mut self) {
        self.term.bar_rows = self
            .find
            .search_bar
            .as_ref()
            .map_or(0, |b| 1 + u16::from(b.replace.is_some()));
        let (cols, rows) = self.term.surface;
        self.apply_viewport(cols, rows);
        self.mark_dirty_render();
    }

    /// 選択の端 (`end` が偽なら始端)。
    fn bar_cursor(&self, end: bool) -> Option<ByteOff> {
        let v = self.view(self.focused_view()?)?;
        let r = v.selection.primary();
        Some(if end { r.end() } else { r.start() })
    }

    /// 大小の区別を巡回する (Smart → Sensitive → Insensitive → Smart) 。切り替えたら再検索する。
    pub fn search_toggle_case(&mut self) -> CommandResult {
        let Some(bar) = &mut self.find.search_bar else {
            return CommandResult::Rejected(crate::command::RejectReason::NotAvailable);
        };
        bar.case = bar.case.next();
        self.bar_changed();
        CommandResult::Done
    }

    /// 正規表現の ON / OFF を切り替える。切り替えたら再検索する。
    pub fn search_toggle_regex(&mut self) -> CommandResult {
        let Some(bar) = &mut self.find.search_bar else {
            return CommandResult::Rejected(crate::command::RejectReason::NotAvailable);
        };
        bar.regex = !bar.regex;
        self.bar_changed();
        CommandResult::Done
    }

    /// 単語単位の ON / OFF を切り替える。切り替えたら再検索する。
    pub fn search_toggle_word(&mut self) -> CommandResult {
        let Some(bar) = &mut self.find.search_bar else {
            return CommandResult::Rejected(crate::command::RejectReason::NotAvailable);
        };
        bar.whole_word = !bar.whole_word;
        self.bar_changed();
        CommandResult::Done
    }

    /// 選択範囲内だけを探すかを切り替える (`search.toggle_selection`)。範囲は検索バーを開いたときの選択で、
    /// 選択が無かったときは切り替えずに知らせる。切り替えたら再検索する。
    pub fn search_toggle_selection(&mut self) -> CommandResult {
        let Some(bar) = &mut self.find.search_bar else {
            return CommandResult::Rejected(crate::command::RejectReason::NotAvailable);
        };
        if bar.sel_ranges.is_empty() {
            self.show_message(MessageLevel::Info, keys::INFO_SEARCH_NO_SELECTION, &[]);
            return CommandResult::Done;
        }
        bar.in_selection = !bar.in_selection;
        // 入力の起点は範囲の先頭にする (範囲の外から探し始めない)
        if bar.in_selection {
            bar.anchor = bar.sel_ranges[0].start;
        }
        self.bar_changed();
        CommandResult::Done
    }

    /// 編集に合わせて、選択範囲内の検索の範囲を写像する (範囲の端での挿入は範囲に含める)。
    pub(crate) fn map_search_scope(&mut self, doc: eve_view::DocumentId, cs: &eve_text::ChangeSet) {
        if self.active_document() != Some(doc) {
            return;
        }
        let Some(bar) = &mut self.find.search_bar else {
            return;
        };
        if bar.sel_ranges.is_empty() {
            return;
        }
        bar.sel_ranges = bar
            .sel_ranges
            .iter()
            .map(|r| {
                ByteRange::new(
                    cs.map_pos(r.start, eve_text::Bias::Before),
                    cs.map_pos(r.end, eve_text::Bias::After),
                )
            })
            .collect();
    }

    /// 入力欄・トグルが変わった: 検索をやり直し、起点から先の最初の一致を強調する。
    /// 検索語をコンパイルできないとき (正規表現の構文エラーなど) は検索せず、理由を示す。
    pub(crate) fn bar_changed(&mut self) {
        let Some(bar) = &self.find.search_bar else {
            return;
        };
        let query = bar.query();
        let anchor = bar.anchor;
        // 検索語・トグルが変わった: 待っているすべて置換・1 件置換の確かめは古い条件のものなので取り消す (古い結果を当てない)
        self.cancel_replace_job();
        // メインスレッドで行うのは構文の解析と書き換えまで。`regex` のコンパイルは検索のジョブで行う (03 §12.3.1)
        let error = query.known_error();
        if let Some(bar) = &mut self.find.search_bar {
            bar.error = error.clone();
            bar.refusal = None;
            bar.current_pending = false;
        }
        if error.is_some() {
            // 前の検索の結果を残さない
            self.search_start(SearchQuery::literal(""));
            if let Some(bar) = &mut self.find.search_bar {
                bar.current = None;
            }
            self.mark_dirty_render();
            return;
        }
        // 探さない理由 (03 §12.3.1 の「探さない場合」): 検索を始めるときの文書の大きさで決め、ジョブを投げない
        let shape = query.shape();
        let doc_len = self
            .active_document()
            .and_then(|d| self.docs.documents.get(&d))
            .map_or(0, |d| d.text().len_bytes());
        if let Some(l) = shape.and_then(|s| s.refusal_for(doc_len)) {
            // 理由だけを持つ検索の状態にする (編集で文書が変わったら、次の再検索で判定し直す)
            self.search_refuse(query, l);
            if let Some(bar) = &mut self.find.search_bar {
                bar.refusal = Some(l);
                bar.current = None;
            }
            self.mark_dirty_render();
            return;
        }
        // 行をまたぐ検索の規則と重い検索語: 入力中の現在の一致は同期検索をせず、全件ジョブが知らせる起点以降の最初の一致
        // (`first_after`) で決める (03 §12.3.1 の「入力中の現在の一致」と「重い検索語」)
        if shape.is_some_and(|s| s.job_driven()) {
            self.search_start_from(query, Some(anchor));
            if let Some(bar) = &mut self.find.search_bar {
                bar.current = None;
                bar.current_pending = true;
            }
            self.mark_dirty_render();
            return;
        }
        // memmem の経路の照合器はここで作る (ジョブと同じ覚えを使う)。正規表現は `Pending` のまま
        let prepared = query.prepared();
        self.search_start(query.clone());
        let found = match (self.active_document(), &prepared) {
            (Some(doc), Prepared::Ready(_)) => match self.sync_find(doc, &query, anchor, true) {
                Found::Hit(m, _) => Some(m),
                _ => None,
            },
            _ => None,
        };
        if let Some(bar) = &mut self.find.search_bar {
            bar.current = found;
            bar.current_pending = matches!(prepared, Prepared::Pending);
        }
        if let Some(m) = found {
            self.reveal(m.start);
        }
        self.mark_dirty_render();
    }

    /// 検索のジョブ (全件・単発・すべて置換) の結果が届いた: ジョブのコンパイルで分かった誤り (`TooLarge`) があれば、
    /// 構文の誤りと同じく検索バーに示して検索をやめる (03 §12.3.1 の「メインスレッドでのコンパイル」)。入力中の現在の
    /// 一致を待っているか (`current_pending`) に依らない (`Enter` で単発のジョブに移った後に届いても示す)。示したら `true`。
    pub(crate) fn bar_show_compile_error(&mut self) -> bool {
        let Some(bar) = &self.find.search_bar else {
            return false;
        };
        if bar.error.is_some() {
            return false;
        }
        let Some(e) = bar.query().known_error() else {
            return false;
        };
        if let Some(bar) = &mut self.find.search_bar {
            bar.error = Some(e);
            bar.current = None;
            bar.current_pending = false;
        }
        self.search_start(SearchQuery::literal(""));
        self.mark_dirty_render();
        true
    }

    /// 検索のジョブの結果が届いた: 正規表現の照合器ができていれば、入力中の現在の一致を求めて強調する (起点から先の
    /// 最初の一致。照合器をメインスレッドで作らないので、入力の時点では求めない、03 §12.3.1)。コンパイルで分かった誤り
    /// (`TooLarge`) は、構文の誤りと同じく検索バーに示して検索をやめる (`bar_show_compile_error`)。
    pub(crate) fn bar_search_progress(&mut self) {
        if self.bar_show_compile_error() {
            return;
        }
        let Some(bar) = &self.find.search_bar else {
            return;
        };
        // 探さなかった・重すぎて打ち切った (走査の途中で分かったものを含む): 理由を示し、現在の一致を消す (03 §12.3.1)
        if let Some(l) = self.find.search.as_ref().and_then(|s| s.limit()) {
            if bar.refusal != Some(l) || bar.current.is_some() || bar.current_pending {
                self.bar_set_refusal(l);
            }
            return;
        }
        if !bar.current_pending {
            return;
        }
        // 行をまたぐ検索の規則と重い検索語: 入力中の現在の一致は、全件ジョブの `first_after` で決める (同期検索をしない、
        // 03 §12.3.1)。編集の前の文書を走査したジョブの結果は採らない (次の再検索で求め直す)
        if let Some(s) = self.find.search.as_ref().filter(|s| s.is_job_driven()) {
            if !self.search_fresh() {
                return;
            }
            let found = match s.first_after() {
                Some(r) => Some(r),
                // 起点以降に一致が無い: 数え終えたら先頭の一致 (折り返し)。数える上限で打ち切ったときは決めない
                None if s.finished() => {
                    if s.counted_all() {
                        s.retained().first().copied()
                    } else {
                        None
                    }
                }
                None => return,
            };
            // 投入した後にカーソル・選択を動かしていたら、強調だけ (スクロールしない)
            let moved = s.spawn_sel() != self.primary_selection();
            if let Some(bar) = &mut self.find.search_bar {
                bar.current = found;
                bar.current_pending = false;
            }
            if let Some(m) = found
                && !moved
            {
                self.reveal(m.start);
            }
            self.mark_dirty_render();
            return;
        }
        let (query, anchor) = (bar.query(), bar.anchor);
        let found = match query.prepared() {
            Prepared::Pending => return,
            Prepared::Nothing => None,
            Prepared::Ready(_) => match self.active_document() {
                Some(doc) => match self.sync_find(doc, &query, anchor, true) {
                    Found::Hit(m, _) => Some(m),
                    _ => None,
                },
                None => None,
            },
        };
        if let Some(bar) = &mut self.find.search_bar {
            bar.current = found;
            bar.current_pending = false;
        }
        if let Some(m) = found {
            self.reveal(m.start);
        }
        self.mark_dirty_render();
    }

    /// 探さない理由を検索バーに示し、検索の一致を捨てる (03 §12.3.1)。
    pub(crate) fn bar_set_refusal(&mut self, l: ScanLimit) {
        if let Some(bar) = &mut self.find.search_bar {
            bar.refusal = Some(l);
            bar.current = None;
            bar.current_pending = false;
        }
        if self
            .find
            .search
            .as_ref()
            .is_some_and(|s| s.limit() != Some(l))
        {
            self.search_set_limit(l);
        }
        self.mark_dirty_render();
    }

    /// 主選択 (始端, 終端)。
    fn primary_selection(&self) -> Option<(ByteOff, ByteOff)> {
        let r = *self.view(self.focused_view()?)?.selection.primary();
        Some((r.start(), r.end()))
    }

    /// 選択を動かさずに、`pos` が見えるようスクロールする。
    fn reveal(&mut self, pos: ByteOff) {
        let Some(view) = self.focused_view() else {
            return;
        };
        let Some(saved) = self.view(view).map(|v| v.selection.clone()) else {
            return;
        };
        self.set_selection(view, Selection::point(pos));
        self.ensure_cursor_visible(view);
        if let Some(v) = self.docs.views.get_mut(&view) {
            v.selection = saved;
        }
    }

    /// `Enter` / `↓` / `F3` (`search.next`) と `↑` (`search.prev`)。入力で強調した一致があれば、まずそれを選択する。
    pub fn search_move(&mut self, forward: bool) -> SearchStep {
        let Some(bar) = &self.find.search_bar else {
            return SearchStep::NoQuery;
        };
        // ボタンにフォーカスがあるときの Enter はすべて置換 (Alt が使えない端末向け、04 §8.4)
        if bar.focus == BarFocus::Button {
            self.search_replace_all();
            return SearchStep::NoQuery;
        }
        // 探さない理由がある: 何もせず、メッセージ行に理由を出す (03 §12.3.1 の「探さない場合」)
        if bar.refusal.is_some() {
            self.notify_search_limit();
            return SearchStep::NoMatch;
        }
        // 移動を始めたら、照合器を待っていた入力中の現在の一致は求めない (移動は単発のジョブが担う)
        // メインスレッドで同期検索をしない検索語では、編集の前の文書の入力中の現在の一致を使わない (正本でないことがある)
        let stale_current =
            self.find.search.as_ref().is_some_and(|s| s.is_job_driven()) && !self.search_fresh();
        if let Some(bar) = &mut self.find.search_bar {
            bar.current_pending = false;
            if stale_current {
                bar.current = None;
            }
        }
        let Some(bar) = &self.find.search_bar else {
            return SearchStep::NoQuery;
        };
        if forward
            && let Some(cur) = bar.current
            && let Some(view) = self.focused_view()
        {
            self.set_selection(
                view,
                Selection::single(eve_text::Range::new(cur.start, cur.end)),
            );
            self.ensure_cursor_visible(view);
            if let Some(bar) = &mut self.find.search_bar {
                bar.current = None;
                bar.anchor = cur.start;
            }
            // 入力中の現在の一致は正本の一致 (行をまたぐ検索の規則では、その終端から次を探す、03 §12.3.1 の「次 / 前」の (1))
            self.mark_canon(cur);
            self.mark_dirty_render();
            return SearchStep::Moved { wrapped: false };
        }
        let r = self.search_step(forward);
        if matches!(r, SearchStep::Moved { .. }) {
            let a = self.bar_cursor(false);
            if let (Some(bar), Some(a)) = (&mut self.find.search_bar, a) {
                bar.current = None;
                bar.anchor = a;
            }
        }
        r
    }

    /// 検索バーの表示モデル。
    pub(crate) fn search_bar_model(&self) -> Option<SearchBarModel> {
        let bar = self.find.search_bar.as_ref()?;
        let render = |key: &str, args: &[(&str, &str)]| {
            self.catalog().render(key, args, &|c| self.key_label(c))
        };
        let cells = cell_text(&bar.text, CutPolicy::End).spans;
        let cursor = cell_text(&bar.text[..bar.cursor], CutPolicy::End)
            .spans
            .len();
        let selection = crate::ui_model::overlay::field_selection(
            bar.all_selected && bar.focus == BarFocus::Search,
            cells.len(),
        );
        let count = self.search_count_for(bar.current);
        // 行をまたぐ検索の規則の次 / 前の結果を約 100 ms 待っている: 件数の位置を待ちの表示にする (03 §12.3.1 の「次 / 前」の (6))
        let waiting = self.search_find_waiting().then(|| {
            let found = match count {
                SearchCount::Idle => 0,
                SearchCount::Running { found } => found,
                SearchCount::Exact { total, .. }
                | SearchCount::AtLeast { total, .. }
                | SearchCount::Approx { total } => total,
            };
            render(
                keys::UI_SEARCH_COUNT_RUNNING,
                &[("found", &found.to_string())],
            )
        });
        let count_text = waiting.or_else(|| match count {
            SearchCount::Idle => None,
            SearchCount::Running { found } => Some(render(
                keys::UI_SEARCH_COUNT_RUNNING,
                &[("found", &found.to_string())],
            )),
            SearchCount::Exact { current, total } | SearchCount::AtLeast { current, total } => {
                let at_least = matches!(
                    self.search_count_for(bar.current),
                    SearchCount::AtLeast { .. }
                );
                Some(if total == 0 {
                    render(keys::UI_SEARCH_COUNT_NO_MATCH, &[])
                } else {
                    let cur = current.map_or("-".to_owned(), |c| c.to_string());
                    let key = if at_least {
                        keys::UI_SEARCH_COUNT_AT_LEAST
                    } else {
                        keys::UI_SEARCH_COUNT_EXACT
                    };
                    render(key, &[("current", &cur), ("total", &total.to_string())])
                })
            }
            SearchCount::Approx { total } => Some(render(
                keys::UI_SEARCH_COUNT_APPROX,
                &[("total", &total.to_string())],
            )),
        });
        let query = bar.query();
        // CP932 互換文字の同一視が効いているときだけ、件数の後ろに印を付ける。今の一致が同一視で当たったものなら、印を
        // 「互換一致」にする (03 §12.4.1 の「検索バーの表示」、04 §8.4。境界の型は変えない)
        let folded = query
            .analyze()
            .ok()
            .flatten()
            .is_some_and(|p| p.compat_folded());
        let count_text = count_text.map(|t| {
            if !folded {
                return t;
            }
            let key = if self.current_match_folded(bar, &query) {
                keys::UI_SEARCH_COUNT_COMPAT_MATCH_SUFFIX
            } else {
                keys::UI_SEARCH_COUNT_COMPAT_SUFFIX
            };
            format!("{t} {}", render(key, &[]))
        });
        let error = bar
            .error
            .as_ref()
            .map(|e| {
                render(
                    keys::MSG_SEARCH_INVALID_REGEX_TEXT,
                    &[("reason", invalid_regex_reason(e))],
                )
            })
            // 探さない理由も件数の位置に出す (text だけ。検索バーは hint を描かない、03 §12.3.1・04 §8.4)
            .or_else(|| {
                bar.refusal.map(|l| {
                    let (_, text_key, args) = limit_message(l);
                    let args: Vec<(&str, &str)> =
                        args.iter().map(|(k, v)| (*k, v.as_str())).collect();
                    render(text_key, &args)
                })
            })
            .map(|text| cell_text(&text, CutPolicy::End));
        let replace = bar.replace.as_ref().map(|(text, cur)| {
            let cells = cell_text(text, CutPolicy::End).spans;
            let cursor = cell_text(&text[..*cur], CutPolicy::End).spans.len();
            let selection = crate::ui_model::overlay::field_selection(
                bar.all_selected && bar.focus == BarFocus::Replace,
                cells.len(),
            );
            ReplaceRowModel {
                label: cell_text(
                    &render(keys::UI_SEARCH_REPLACE_LABEL, &[]),
                    CutPolicy::Never,
                ),
                input: InputFieldModel {
                    len: cells.len(),
                    cells,
                    first: 0,
                    cursor,
                    selection,
                    placeholder: None,
                },
                button: cell_text(
                    &render(keys::UI_SEARCH_REPLACE_ALL_BUTTON, &[]),
                    CutPolicy::Never,
                ),
            }
        });
        Some(SearchBarModel {
            replace,
            focus: bar.focus,
            label: cell_text(&render(keys::UI_SEARCH_LABEL, &[]), CutPolicy::Never),
            input: InputFieldModel {
                len: cells.len(),
                cells,
                first: 0,
                cursor,
                selection,
                placeholder: None,
            },
            case: match bar.case {
                Case::Smart => CaseMode::Smart,
                Case::Sensitive => CaseMode::Sensitive,
                Case::Insensitive => CaseMode::Insensitive,
            },
            case_on: query.is_case_sensitive(),
            regex: bar.regex,
            whole_word: bar.whole_word,
            in_selection: bar.in_selection,
            error,
            count_text: count_text.map(|t| cell_text(&t, CutPolicy::End)),
        })
    }

    /// 件数の表示の今の一致が、CP932 互換文字の同一視で当たったものか (同一視を切った検索語が、文書の前後の文脈のまま、
    /// その範囲にちょうど一致できない、03 §12.4.1)。照合器が `bg` のジョブで作られた後で、重くない検索語の、64 KiB
    /// (`SEARCH_SYNC_MULTILINE_MAX`) 以下の一致だけを、メインスレッドで確かめる (それ以外は偽 = 「互換」の印)。
    fn current_match_folded(&self, bar: &SearchBarState, query: &SearchQuery) -> bool {
        let Some(r) = self.search_current_range(bar.current) else {
            return false;
        };
        if r.end.0 - r.start.0 > SEARCH_SYNC_MULTILINE_MAX
            || query.shape().is_some_and(|s| s.heavy_guard())
        {
            return false;
        }
        let Some(doc_id) = self.find.search.as_ref().map(|s| s.doc()) else {
            return false;
        };
        let Some(doc) = self.docs.documents.get(&doc_id) else {
            return false;
        };
        let key: FoldedKey = (doc_id, doc.revision(), r, query.clone());
        if let Some((k, v)) = &*bar.memo.folded.borrow()
            && *k == key
        {
            return *v;
        }
        let Prepared::Ready(m) = query.prepared() else {
            return false;
        };
        let Some(mut check) = m.fold_check_ready() else {
            return false;
        };
        let v = check.folded(doc.text(), r);
        *bar.memo.folded.borrow_mut() = Some((key, v));
        v
    }

    /// 可視行の一致 (強調する範囲)。保持した一致ではなく、可視行だけを同期検索する (03 §12.2 の「可視範囲のハイライト」)。
    /// 1 行が `SYNC_LINE_MAX` を超える行は強調しない。検索バーが開いていなければ `None`。
    /// (文書, Revision, 範囲, 問い合わせ) が前と同じなら、前の結果を返す (REF-04)。
    pub(crate) fn visible_matches(
        &self,
        doc_id: eve_view::DocumentId,
        top_line: usize,
        rows: usize,
    ) -> Option<VisibleMatches> {
        let bar = self.find.search_bar.as_ref()?;
        // 探さなかった・打ち切った: 強調しない (それまでに届いた一致も捨てる、03 §12.3.1)
        if bar.refusal.is_some() {
            return None;
        }
        let query = bar.query();
        let doc = self.docs.documents.get(&doc_id)?;
        if let Some(shape) = query.shape().filter(Shape::job_driven) {
            return Some(self.visible_whole(doc_id, doc, query, shape, top_line, rows));
        }
        let key: VisibleKey = (doc_id, doc.revision(), top_line, rows, query);
        if let Some((k, v)) = &*bar.memo.visible.borrow()
            && *k == key
        {
            return Some(v.clone());
        }
        // 正規表現の照合器は `bg` のジョブが作る。まだ無ければ強調しない (覚えにも入れない、03 §12.3.1)
        let Prepared::Ready(m) = key.4.prepared() else {
            return None;
        };
        let found: std::rc::Rc<[(usize, usize)]> =
            self.scan_visible(doc, &m, top_line, rows).into();
        *bar.memo.visible.borrow_mut() = Some((key, found.clone()));
        Some(found)
    }

    /// 行をまたぐ検索の規則の可視範囲の強調 (03 §12.3.1 の「可視範囲の強調」): 全件ジョブが終わり、保持した一致が可視行を
    /// 覆うときはそれを使う。それ以外は、検索語が重くなく、窓 (可視行の先頭の k 行前の行頭から可視行の末尾までを核にした窓の
    /// 全体) が 64 KiB 以下のときだけ、メインスレッドで窓を同期検索して可視行と交わる一致を強調する。条件に合わないとき・
    /// 上限なしの検索語では、届いている保持した一致だけを強調する (強調のためのジョブは作らない)。k 行前から始める同期検索は
    /// 正本と食い違うことがあるが、食い違うのは強調だけ。
    fn visible_whole(
        &self,
        doc_id: eve_view::DocumentId,
        doc: &crate::document::Document,
        query: SearchQuery,
        shape: Shape,
        top_line: usize,
        rows: usize,
    ) -> VisibleMatches {
        let t = doc.text();
        let n = t.len_lines();
        let line_at = |l: usize| {
            if l >= n {
                t.len_bytes()
            } else {
                t.line_to_byte(LineIdx(l)).0
            }
        };
        let last = top_line.saturating_add(rows).min(n);
        let (vis_s, vis_e) = (line_at(top_line), line_at(last));
        let retained = self
            .find
            .search
            .as_ref()
            .filter(|s| s.doc() == doc_id && s.is_job_driven() && self.search_fresh());
        // 保持した一致のうち可視行と交わるもの (k が 0 の重い検索語は走査の順が文書の順でないので、全体から選んで並べる)
        let from_retained = |s: &crate::search::search_state::SearchState| -> VisibleMatches {
            let mut v: Vec<(usize, usize)> = s
                .retained()
                .iter()
                .filter(|r| r.end.0 > vis_s && r.start.0 < vis_e)
                .map(|r| (r.start.0, r.end.0))
                .collect();
            v.sort_unstable();
            v.into()
        };
        // 1. 全件ジョブが終わり、保持した一致が可視行を覆う
        if let Some(s) = retained
            && s.finished()
            && s.covered() >= vis_e
        {
            return from_retained(s);
        }
        // 2. 重くなく、窓が 64 KiB 以下なら、窓を同期検索する
        if let LineSpan::Bounded(k) = shape.lines()
            && shape.whole_doc()
            && !shape.heavy_guard()
        {
            let core_s = line_at(top_line.saturating_sub(k));
            let we = line_at(last.saturating_add(k));
            if we.saturating_sub(core_s) <= SEARCH_SYNC_MULTILINE_MAX {
                let key: VisibleKey = (doc_id, doc.revision(), top_line, rows, query);
                let bar = self.find.search_bar.as_ref();
                if let Some(bar) = bar
                    && let Some((k0, v)) = &*bar.memo.visible.borrow()
                    && *k0 == key
                {
                    return v.clone();
                }
                if let Prepared::Ready(m) = key.4.prepared() {
                    let cancel = CancelToken::new();
                    let mut out = Vec::new();
                    let end = scan_range(
                        t,
                        &m,
                        ByteRange::new(ByteOff(core_s), ByteOff(vis_e)),
                        &mut ScanCtx::new(&cancel),
                        &mut |r| {
                            if r.end.0 > vis_s {
                                out.push((r.start.0, r.end.0));
                            }
                            true
                        },
                    );
                    // 捨てた候補の予算を超えた (`TooHeavy`) ときは、その場は同期検索で強調しない
                    if end == ScanEnd::Done {
                        let found: VisibleMatches = out.into();
                        if let Some(bar) = bar {
                            *bar.memo.visible.borrow_mut() = Some((key, found.clone()));
                        }
                        return found;
                    }
                }
            }
        }
        // 3. 届いている保持した一致だけ
        retained.map_or_else(|| Vec::new().into(), from_retained)
    }

    fn scan_visible(
        &self,
        doc: &crate::document::Document,
        m: &crate::search::Matcher,
        top_line: usize,
        rows: usize,
    ) -> Vec<(usize, usize)> {
        let t = doc.text();
        let n = t.len_lines();
        let mut out = Vec::new();
        let cancel = CancelToken::new();
        for l in top_line..(top_line + rows).min(n) {
            let len = t.line_end_byte(LineIdx(l)).0 - t.line_to_byte(LineIdx(l)).0;
            if len > SYNC_LINE_MAX {
                continue;
            }
            // 1 行の範囲 (行頭から次の行頭まで。最後の行は文書の末尾まで)
            let end = if l + 1 < n {
                t.line_to_byte(LineIdx(l + 1))
            } else {
                ByteOff(t.len_bytes())
            };
            let range = ByteRange::new(t.line_to_byte(LineIdx(l)), end);
            scan_bytes(t, m, range, &cancel, &mut |r| {
                out.push((r.start.0, r.end.0));
                true
            });
        }
        out
    }

    /// 入力で強調している現在の一致 (文書内のバイト範囲)。
    pub(crate) fn bar_current(&self) -> Option<(usize, usize)> {
        self.find
            .search_bar
            .as_ref()?
            .current
            .map(|r| (r.start.0, r.end.0))
    }
}

/// 探さない理由の文言 (03 §12.3.1 の「探さない場合」と「最悪の入力の上限」、04 §15.2): (メッセージの基の名前, その `.text`
/// のキー, 引数)。検索バーの件数の位置には `.text` だけを出し、次 / 前・置換ではメッセージ行に text と hint を出す。
fn limit_message(l: ScanLimit) -> (&'static str, &'static str, Vec<(&'static str, String)>) {
    match l {
        ScanLimit::TooLarge => (
            keys::MSG_SEARCH_MULTILINE_TOO_LARGE,
            keys::MSG_SEARCH_MULTILINE_TOO_LARGE_TEXT,
            vec![("mib", (SEARCH_CONCAT_MAX_BYTES >> 20).to_string())],
        ),
        ScanLimit::TooHeavy => (
            keys::MSG_SEARCH_PATTERN_TOO_HEAVY,
            keys::MSG_SEARCH_PATTERN_TOO_HEAVY_TEXT,
            Vec::new(),
        ),
    }
}

/// 正規表現が不正という誤りの理由 (構文エラーと大きさの上限の超過)。
fn invalid_regex_reason(e: &QueryError) -> &str {
    match e {
        QueryError::Syntax(reason) => reason,
        QueryError::TooLarge => "pattern too large",
    }
}

impl EditorState {
    /// 照合器の無い検索語で置換しようとした: 検索語が空なら `info.search.replace_empty_query`、コンパイルできなければ
    /// 検索バーに出ている誤りと同じ内容をメッセージ行に出す (04 §8.4。何も置き換えない)。
    fn notify_unusable_query(&mut self, query: &SearchQuery) {
        match query.known_error() {
            Some(e) => self.show_message(
                MessageLevel::Warn,
                keys::MSG_SEARCH_REPLACE_INVALID_REGEX,
                &[("reason", invalid_regex_reason(&e))],
            ),
            None => self.show_message(
                MessageLevel::Info,
                keys::INFO_SEARCH_REPLACE_EMPTY_QUERY,
                &[],
            ),
        }
    }

    /// 探さない理由をメッセージ行に text と hint で出す (03 §12.3.1、04 §8.4)。
    fn show_limit_message(&mut self, l: ScanLimit) {
        let (key, _, args) = limit_message(l);
        let args: Vec<(&str, &str)> = args.iter().map(|(k, v)| (*k, v.as_str())).collect();
        self.show_message(MessageLevel::Warn, key, &args);
    }

    /// 探さない理由があるので置換しなかった: メッセージ行に出し、検索バーにも示す (03 §12.3.1。何も置き換えない)。
    fn notify_limit(&mut self, l: ScanLimit) {
        self.show_limit_message(l);
        self.bar_set_refusal(l);
    }

    /// 探さない理由があるので次 / 前をしなかった: メッセージ行に text と hint を出す (03 §12.3.1)。
    pub(crate) fn notify_search_limit(&mut self) {
        let limit = self
            .find
            .search_bar
            .as_ref()
            .and_then(|b| b.refusal)
            .or_else(|| self.find.search.as_ref().and_then(|s| s.limit()));
        if let Some(l) = limit {
            self.show_limit_message(l);
        }
    }

    /// 1 件置換の確かめのジョブを待っている。
    fn replace_one_pending(&self) -> bool {
        self.io
            .pending
            .values()
            .any(|p| matches!(p, Pending::ReplaceOne { .. }))
    }

    /// 待っているすべて置換と 1 件置換の確かめのジョブを取り消す (検索バーを閉じた・条件が変わった)。
    fn cancel_replace_job(&mut self) {
        let ids: Vec<_> = self
            .io
            .pending
            .iter()
            .filter(|(_, p)| matches!(p, Pending::ReplaceAll { .. } | Pending::ReplaceOne { .. }))
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.io.pending.remove(&id);
            self.jobs.cancel(id);
        }
    }

    /// `search.replace_all` (03 §12.5): 検索欄の検索語・大小の区別で、置換欄の文字列にすべて置き換える。1 つの Undo 単位。
    /// 文書が 1 MiB 以下ならメインスレッドで、超えればジョブで `ChangeSet` を作る。
    pub fn search_replace_all(&mut self) -> CommandResult {
        let Some(bar) = &self.find.search_bar else {
            return CommandResult::Rejected(crate::command::RejectReason::NotAvailable);
        };
        let Some((replacement, _)) = bar.replace.clone() else {
            return CommandResult::Rejected(crate::command::RejectReason::NotAvailable);
        };
        let query = bar.query();
        let Some(doc_id) = self.active_document() else {
            return CommandResult::Done;
        };
        let Some(doc) = self.docs.documents.get(&doc_id) else {
            return CommandResult::Done;
        };
        if !doc.is_editable() {
            self.notify_read_only();
            return CommandResult::Done;
        }
        // 正規表現では置換文字列の `\n` を文書の既定の改行に直す (`$1` の展開は一致ごと、03 §12.5)
        let replacement = if query.is_regex() {
            unescape_replacement(&replacement, doc.format.default_eol.as_str())
        } else {
            replacement
        };
        // 置換の文字列の文字どおりの部分は外からの文字列 (挿入の入口 (3)、03 §8.3.1)。`$1` 等が運ぶ文字は同じ文書のもの
        let (replacement, pua) = replacement_text(doc, &replacement);
        let prepared = query.prepared();
        if matches!(prepared, Prepared::Nothing) {
            self.notify_unusable_query(&query);
            return CommandResult::Done;
        }
        let limit = bar.refusal.or_else(|| {
            query
                .shape()
                .and_then(|s| s.refusal_for(doc.text().len_bytes()))
        });
        if let Some(l) = limit {
            self.notify_limit(l);
            return CommandResult::Done;
        }
        // 重い検索語は、小さい文書でもメインスレッドで作らない (03 §12.3.1 の「重い検索語」)
        let heavy = query.shape().is_some_and(|s| s.heavy_guard());
        // 実行中のすべて置換があれば二重に始めない
        if self
            .io
            .pending
            .values()
            .any(|p| matches!(p, Pending::ReplaceAll { .. }))
        {
            return CommandResult::Done;
        }
        let revision = doc.revision();
        // 小さい文書はメインスレッドで作る。正規表現の照合器がまだ無い (`bg` のジョブがコンパイルしていない) ときは、
        // メインスレッドでコンパイルせずにジョブで作る (03 §12.3.1)
        if doc.text().len_bytes() <= REPLACE_SYNC_BYTES
            && matches!(prepared, Prepared::Ready(_))
            && !heavy
        {
            let built =
                build_replace_all_in(doc.text(), &query, &replacement, None, &CancelToken::new());
            self.find.replace_all_pua = pua;
            match built {
                Ok(Some(b)) => self.apply_replace_all(doc_id, b.changes, b.count, b.folded),
                Ok(None) => {}
                Err(e) => {
                    if let Some(l) = e.limit() {
                        self.notify_limit(l);
                    }
                }
            }
            return CommandResult::Done;
        }
        self.find.replace_all_pua = pua;
        let snapshot = doc.snapshot();
        // 走っている全件ジョブの後ろで待たない (取り消して、すべて置換の後ろに投げ直す。適用の後は再検索する)
        self.yield_search(|st| {
            let job = st.jobs.spawn(
                Some(doc_id),
                revision,
                JobKind::ReplaceAll {
                    snapshot,
                    query,
                    replacement,
                },
            );
            st.io.pending.insert(
                job,
                Pending::ReplaceAll {
                    doc: doc_id,
                    revision,
                },
            );
        });
        CommandResult::Done
    }

    /// 1 件置換の対象の一致: 選択範囲がちょうど一致ならそれ (`search.next` で確定した一致)、
    /// そうでなければ入力で強調している現在の一致。
    fn replace_target(
        &self,
        doc_id: eve_view::DocumentId,
        query: &SearchQuery,
        replacement: &str,
    ) -> Option<(ByteRange, String)> {
        let doc = self.docs.documents.get(&doc_id)?;
        let text = doc.text();
        let sel = self
            .focused_window()
            .and_then(|w| self.window_view(w))
            .and_then(|v| self.view(v))
            .map(|v| *v.selection.primary())?;
        let selected = ByteRange::new(sel.start(), sel.end());
        if !selected.is_empty()
            && let Some(s) = replacement_for(text, query, selected, replacement)
        {
            return Some((selected, s));
        }
        let cur = self.find.search_bar.as_ref()?.current?;
        replacement_for(text, query, cur, replacement).map(|s| (cur, s))
    }

    /// 同期検索をしない検索語 (行をまたぐ検索の規則・重い検索語) の 1 件置換の対象 (03 §12.3.1 の「置換」): 選択範囲が正本と
    /// 分かっている一致 (保持した一致・新しい入力中の現在の一致・次 / 前で移った一致) ならそれ、そうでなく新しい入力中の現在の
    /// 一致があればそれ。返すのは (対象, 選び始める正本の位置)。正本の位置は、行をまたぐ検索の規則では対象の始端 (一致は始まる
    /// 位置で決まる)、k が 0 の重い検索語では対象の始端を含む行の行頭。
    fn replace_candidate(&self, doc_id: eve_view::DocumentId) -> Option<(ByteRange, ByteOff)> {
        let doc = self.docs.documents.get(&doc_id)?;
        let (text, rev) = (doc.text(), doc.revision());
        let s = self.find.search.as_ref().filter(|s| s.doc() == doc_id)?;
        let fresh = self.search_fresh();
        let current = self.find.search_bar.as_ref()?.current.filter(|_| fresh);
        let known =
            |r: ByteRange| (fresh && s.retains(r)) || s.is_canon(r, rev) || current == Some(r);
        let (a, b) = self.primary_selection()?;
        let selected = ByteRange::new(a, b);
        let target = if !selected.is_empty() && known(selected) {
            selected
        } else {
            current?
        };
        let from = if s.is_whole() {
            target.start
        } else {
            text.line_to_byte(text.byte_to_line(target.start))
        };
        Some((target, from))
    }

    /// 行をまたぐ検索の規則の 1 件置換で対象が無い: 置き換えず、次の一致を探す (`search.next` と同じ。届いたら選び、もう一度
    /// `Enter` で置き換える)。次の一致・入力中の現在の一致を待っている間は何もしない (押した回数を数えない。1 回の `Enter` で
    /// 2 件を置き換えない、03 §12.3.1 の「置換」)。
    fn replace_seek_next(&mut self) -> CommandResult {
        if self.replace_waiting() {
            return CommandResult::Done;
        }
        if matches!(self.search_move(true), SearchStep::NoMatch) {
            self.show_message(MessageLevel::Info, keys::INFO_SEARCH_REPLACE_NO_MATCH, &[]);
        }
        CommandResult::Done
    }

    /// 同期検索をしない検索語の 1 件置換が、結果を待っている (単発の検索・全件ジョブの結果を待つ次 / 前・入力中の現在の一致・
    /// 1 件置換の確かめ)。待っている間の置換欄の `Enter` は何もしない (03 §12.3.1 の「置換」)。
    fn replace_waiting(&self) -> bool {
        self.find.find_job.is_some()
            || self.find.find_wait.is_some()
            || self
                .find
                .search_bar
                .as_ref()
                .is_some_and(|b| b.current_pending)
            || self.replace_one_pending()
    }

    /// `search.replace` (03 §12.5): 現在の一致を置換欄の文字列に置き換え、次の一致を強調する。置換ごとに別の Undo 単位。
    /// 正規表現では `$1` / `${name}` を展開し、`\n` を既定の改行にする。
    pub fn search_replace(&mut self) -> CommandResult {
        let Some(bar) = &self.find.search_bar else {
            return CommandResult::Rejected(crate::command::RejectReason::NotAvailable);
        };
        let Some((replacement, _)) = bar.replace.clone() else {
            return CommandResult::Rejected(crate::command::RejectReason::NotAvailable);
        };
        let query = bar.query();
        let Some(doc_id) = self.active_document() else {
            return CommandResult::Done;
        };
        let Some(doc) = self.docs.documents.get(&doc_id) else {
            return CommandResult::Done;
        };
        if !doc.is_editable() {
            self.notify_read_only();
            return CommandResult::Done;
        }
        let m = match query.prepared() {
            Prepared::Ready(m) => m,
            Prepared::Nothing => {
                self.notify_unusable_query(&query);
                return CommandResult::Done;
            }
            // 正規表現の照合器を `bg` のジョブがまだ作っていない: 置き換えず何もしない (件数の位置の `…` が待ちを示す。
            // メインスレッドでコンパイルしない、03 §12.3.1)
            Prepared::Pending => return CommandResult::Done,
        };
        if let Some(l) = bar.refusal {
            self.notify_limit(l);
            return CommandResult::Done;
        }
        let shape = query.shape();
        let jobbed = shape.is_some_and(|s| s.job_driven());
        // 同期検索をしない検索語で、次の一致・入力中の現在の一致・確かめの結果を待っている間は何もしない (押した回数も
        // 数えない。飛ばしたつもりの一致を置き換えない、03 §12.3.1 の「置換」)
        if jobbed && self.replace_waiting() {
            return CommandResult::Done;
        }
        let replacement = if query.is_regex() {
            unescape_replacement(&replacement, doc.format.default_eol.as_str())
        } else {
            replacement
        };
        // 置換の文字列の文字どおりの部分は外からの文字列 (挿入の入口 (3)、03 §8.3.1)
        let (replacement, pua) = replacement_text(doc, &replacement);
        if !jobbed {
            let Some((range, insert)) = self.replace_target(doc_id, &query, &replacement) else {
                self.show_message(MessageLevel::Info, keys::INFO_SEARCH_REPLACE_NO_MATCH, &[]);
                return CommandResult::Done;
            };
            self.apply_replace_one(doc_id, range, insert, pua);
            return CommandResult::Done;
        }
        let Some((range, from)) = self.replace_candidate(doc_id) else {
            return self.replace_seek_next();
        };
        let Some(shape) = shape else {
            return CommandResult::Done;
        };
        let text = doc.text();
        // 確かめをメインスレッドで行うのは、行をまたぐ検索の規則で、重くなく、行の数に上限があり、取り直しの窓 (対象の始端から、
        // 延ばす分の終わりまで) が 64 KiB 以下のときだけ。重い検索語・上限の無い検索語・大きな窓はジョブで行う (一致 1 件が
        // 大きくなりうる。重い検索語の照合はメインスレッドを止めうる、03 §12.3.1 の「重い検索語」)
        let on_main = match shape.lines() {
            LineSpan::Bounded(k) if shape.whole_doc() && !shape.heavy_guard() => {
                let last = text.byte_to_line(ByteOff(range.end.0.saturating_sub(1))).0;
                let n = text.len_lines();
                let lines_end = if last.saturating_add(k).saturating_add(1) >= n {
                    text.len_bytes()
                } else {
                    text.line_to_byte(LineIdx(last + k + 1)).0
                };
                let end = shape
                    .max_len()
                    .map_or(lines_end, |n| lines_end.min(range.end.0.saturating_add(n)));
                end.saturating_sub(range.start.0) <= SEARCH_SYNC_MULTILINE_MAX
            }
            _ => false,
        };
        if on_main {
            match replace_one(
                text,
                &m,
                from,
                range,
                &replacement,
                None,
                &CancelToken::new(),
            ) {
                Ok(Some(insert)) => self.apply_replace_one(doc_id, range, insert, pua),
                // 対象でない: 置き換えず、次の一致を探す
                Ok(None) => return self.replace_seek_next(),
                Err(e) => {
                    if let Some(l) = e.limit() {
                        self.notify_limit(l);
                    }
                }
            }
            return CommandResult::Done;
        }
        let revision = doc.revision();
        let snapshot = doc.snapshot();
        self.yield_search(|st| {
            let job = st.jobs.spawn(
                Some(doc_id),
                revision,
                JobKind::ReplaceOne {
                    snapshot,
                    query,
                    from,
                    range,
                    replacement,
                },
            );
            st.io.pending.insert(
                job,
                Pending::ReplaceOne {
                    doc: doc_id,
                    revision,
                    range,
                    pua,
                },
            );
        });
        CommandResult::Done
    }

    /// 1 件置換の確かめのジョブの結果 (03 §12.3.1 の「置換」)。投げたときの文書のままなら置き換える。対象が一致でなければ、
    /// 置き換えず次の一致を探す。
    pub(crate) fn finish_replace_one(
        &mut self,
        doc_id: eve_view::DocumentId,
        revision: eve_view::Revision,
        range: ByteRange,
        pua: u64,
        r: JobResult,
    ) {
        if self.bar_show_compile_error() {
            return;
        }
        let same = self
            .docs
            .documents
            .get(&doc_id)
            .is_some_and(|d| d.revision() == revision);
        match r {
            JobResult::ReplaceOneDone {
                insert: Some(insert),
            } if same && self.find.search_bar.is_some() => {
                self.apply_replace_one(doc_id, range, insert, pua);
            }
            JobResult::ReplaceOneDone { insert: None } if same => {
                self.replace_seek_next();
            }
            JobResult::SearchLimited { limit } if self.find.search_bar.is_some() => {
                self.notify_limit(limit);
            }
            _ => {}
        }
        self.mark_dirty_render();
    }

    /// 1 件置換を当てる (置換ごとに別の Undo 単位)。カーソルを置換の後ろに置き、そこから次の一致を探し直す。一致した文字列と
    /// 置換の文字列 (展開した後) が同じなら置き換えず (文書を変えない)、一致の後ろから次の一致へ進む (03 §12.5)。
    fn apply_replace_one(
        &mut self,
        doc_id: eve_view::DocumentId,
        range: ByteRange,
        insert: String,
        pua: u64,
    ) {
        let Some(doc) = self.docs.documents.get(&doc_id) else {
            return;
        };
        if insert.len() == range.end.0 - range.start.0
            && doc.text().slice_to_string(range) == insert
        {
            let view = self.focused_view();
            if let Some(v) = view {
                self.set_selection(v, Selection::point(range.end));
            }
            if let Some(bar) = &mut self.find.search_bar {
                bar.anchor = range.end;
            }
            self.bar_changed();
            return;
        }
        let after = ByteOff(range.start.0 + insert.len());
        let Ok(changes) = eve_text::ChangeSet::from_sorted_edits(
            doc.text(),
            [eve_text::Edit {
                range,
                insert: std::sync::Arc::from(insert),
            }],
        ) else {
            return;
        };
        let view = self.focused_view();
        let sel = view.and_then(|v| self.docs.views.get(&v).map(|v| v.selection.clone()));
        let mut tx = Transaction::new(
            changes,
            Origin::Command(eve_view::CommandId::new(ids::SEARCH_REPLACE)),
            self.now,
        )
        .with_undo_group(UndoGroupHint::Break)
        .with_selections(sel, None);
        if let Some(v) = view {
            tx = tx.with_view(v);
        }
        if self.apply_transaction(doc_id, tx).is_err() {
            return;
        }
        if pua > 0 {
            crate::command::edit::show_notices(self, &[InsertNotice::PuaAsText(pua)]);
        }
        // 次の一致は、置換した文字列の後ろから探す。カーソルも置換の後ろに置く
        if let Some(v) = view {
            self.set_selection(v, Selection::point(after));
        }
        if let Some(bar) = &mut self.find.search_bar {
            bar.anchor = after;
        }
        self.bar_changed();
    }

    /// すべて置換のジョブの結果。作った Revision が古ければ適用せず中止を知らせる。
    pub(crate) fn finish_replace_all(
        &mut self,
        doc_id: eve_view::DocumentId,
        revision: eve_view::Revision,
        r: JobResult,
    ) {
        if let JobResult::ReplaceAllDone {
            changes,
            count,
            folded,
        } = r
        {
            let fresh = self
                .docs
                .documents
                .get(&doc_id)
                .is_some_and(|d| d.revision() == revision);
            if fresh {
                self.apply_replace_all(doc_id, changes, count, folded);
            } else {
                self.show_message(MessageLevel::Warn, keys::MSG_SEARCH_REPLACE_STALE, &[]);
            }
            return;
        }
        // 探さなかった・重すぎて打ち切った (03 §12.3.1): 何も置き換えず、理由を示す
        if let JobResult::SearchLimited { limit } = r {
            if self.find.search_bar.is_some() {
                self.notify_limit(limit);
            }
            return;
        }
        // ジョブのコンパイルで誤り (`TooLarge`) が分かった: メッセージ行と検索バーに同じ理由を示す (何も置き換えない)
        let query = self.find.search_bar.as_ref().map(SearchBarState::query);
        if let Some(q) = query
            && q.known_error().is_some()
        {
            self.notify_unusable_query(&q);
            self.bar_show_compile_error();
        }
    }

    /// `ChangeSet` を 1 つの Transaction (`Origin::Command("search.replace_all")`、Undo 1 回分) として適用する。
    fn apply_replace_all(
        &mut self,
        doc_id: eve_view::DocumentId,
        changes: eve_text::ChangeSet,
        count: u64,
        folded: u64,
    ) {
        if count == 0 {
            self.show_message(MessageLevel::Info, keys::INFO_SEARCH_REPLACE_NO_MATCH, &[]);
            return;
        }
        let view = self.focused_view();
        let sel = view.and_then(|v| self.docs.views.get(&v).map(|v| v.selection.clone()));
        let mut tx = Transaction::new(
            changes,
            Origin::Command(eve_view::CommandId::new(ids::SEARCH_REPLACE_ALL)),
            self.now,
        )
        .with_undo_group(UndoGroupHint::Break)
        .with_selections(sel, None);
        if let Some(v) = view {
            tx = tx.with_view(v);
        }
        // 大量の置換は、Undo の記録を 1 件の置き換えにしてよい (メモリが増えないときだけ、03 §12.5)
        if count >= REPLACE_COARSE_UNDO_MIN {
            tx = tx.with_coarse_undo();
        }
        let lines_of = |st: &Self| {
            st.docs
                .documents
                .get(&doc_id)
                .map_or(0, |d| d.text().len_lines())
        };
        let before = lines_of(self);
        if self.apply_transaction(doc_id, tx).is_err() {
            self.show_message(MessageLevel::Warn, keys::MSG_SEARCH_REPLACE_STALE, &[]);
            return;
        }
        // 件数に加えて、文書の行数の変化を言う (行をまたぐ置換は 1 件で多くの行を消しうる。上限の無い検索語は文書の残りを
        // 「1 件」として消す。03 §12.3.1 の「置換」、04 §15.2 の `info.search.replaced`)
        // CP932 互換文字の同一視で当たった一致があれば、その件数を分けて言う (03 §12.4.1 の「置換」)。行数の変化と両方あるときは
        // 1 つの知らせにまとめ、件数・行数・同一視の件数の順に言う (幅が足りなければ後ろから切れるので、大事なものから並べる)
        let after = lines_of(self);
        let count_s = count.to_string();
        let folded_s = folded.to_string();
        let (key, lines) = match (after.cmp(&before), folded > 0) {
            (std::cmp::Ordering::Less, false) => {
                (keys::INFO_SEARCH_REPLACED_LINES_REMOVED, before - after)
            }
            (std::cmp::Ordering::Greater, false) => {
                (keys::INFO_SEARCH_REPLACED_LINES_ADDED, after - before)
            }
            (std::cmp::Ordering::Equal, false) => (keys::INFO_SEARCH_REPLACED, 0),
            (std::cmp::Ordering::Less, true) => (
                keys::INFO_SEARCH_REPLACED_FOLDED_LINES_REMOVED,
                before - after,
            ),
            (std::cmp::Ordering::Greater, true) => (
                keys::INFO_SEARCH_REPLACED_FOLDED_LINES_ADDED,
                after - before,
            ),
            (std::cmp::Ordering::Equal, true) => (keys::INFO_SEARCH_REPLACED_FOLDED, 0),
        };
        let lines_s = lines.to_string();
        self.show_message(
            MessageLevel::Info,
            key,
            &[
                ("count", &count_s),
                ("lines", &lines_s),
                ("folded", &folded_s),
            ],
        );
        // 置換の文字列の私用文字を表記にしたことは、置換の件数の後に知らせる (黙って文字列にしない、03 §8.3.1)
        let pua = std::mem::take(&mut self.find.replace_all_pua).saturating_mul(count);
        if pua > 0 {
            crate::command::edit::show_notices(self, &[InsertNotice::PuaAsText(pua)]);
        }
        // 保持した一致は編集で無効になった: 検索をやり直す
        self.bar_changed();
    }
}

/// 置換の文字列の文字どおりの部分を、挿入先の文書の規則で外からの文字列として直す (03 §8.3.1 の挿入の入口 (3))。
/// `U+10FF00..` を `<U+10FFNN>` にする (`$` を含まないので、`$1` / `${name}` の展開は変わらない)。返すのは
/// (置換の文字列, 置換 1 つあたりに置き換えた数)。
fn replacement_text(doc: &crate::document::Document, replacement: &str) -> (String, u64) {
    let (s, n) = insert_rule::external_text(replacement, EscapeContext::of(&doc.format));
    (s.into_owned(), n)
}

/// 編集が止まってから再検索するまでの間 (03 §12.2「編集後の結果」)。
const RESEARCH_DELAY: u64 = 300;

impl EditorState {
    /// 編集で検索の結果が無効になったら、入力が止まって 300 ms 後に再検索する。`Editor::tick` から呼ぶ。
    pub(crate) fn search_tick(&mut self) {
        // 次 / 前の待ちが 100 ms を過ぎた: 件数の位置を待ちの表示にする (03 §12.3.1 の「次 / 前」の (6))
        if self.search_find_waiting() {
            self.mark_dirty_render();
        }
        let stale =
            self.find.search_bar.is_some() && self.find.search.is_some() && !self.search_fresh();
        let cur = self
            .find
            .search
            .as_ref()
            .and_then(|s| self.docs.documents.get(&s.doc()).map(|d| d.revision()));
        let (true, Some(cur)) = (stale, cur) else {
            self.find.research = None;
            return;
        };
        match self.find.research {
            Some((rev, at)) if rev == cur => {
                if self.now.saturating_sub(at) >= crate::time::Millis(RESEARCH_DELAY) {
                    self.find.research = None;
                    self.bar_changed();
                }
            }
            // 初めて気づいた、または気づいた後にまた編集された: 待ち直す
            _ => self.find.research = Some((cur, self.now)),
        }
    }

    /// 次に `tick` が要る時刻 (再検索の期限)。
    pub(crate) fn search_deadline(&self) -> Option<Tick> {
        self.find
            .research
            .map(|(_, at)| Tick(at.0 + RESEARCH_DELAY))
    }
}
