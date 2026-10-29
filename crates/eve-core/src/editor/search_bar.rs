//! 検索バー (docs/04 §8.4、03 §12、01 FR-401〜404)。
//!
//! 開いている間は編集面を 1 行縮め、キー入力はバーの入力欄へ行く (コンテキスト `search`)。入力のたびに検索をやり直し、
//! 最初の一致 (開いたときの位置から先) を **強調するだけで選択はしない**。`Enter` / `↓` / `F3` で一致が主選択になる (確定)。
//! `Esc` で閉じると、確定していない一致は選択として残さない。

use eve_text::{ByteOff, ByteRange, LineIdx, Selection, TextRead};
use eve_view::{
    BarFocus, CaseMode, CutPolicy, InputFieldModel, MessageLevel, ReplaceRowModel, SearchBarModel,
};

use crate::cancel::CancelToken;
use crate::document::{Origin, Transaction, UndoGroupHint};
use crate::editor::command::CommandResult;
use crate::jobs::{JobKind, JobResult};
use crate::search::{Case, QueryError, SearchQuery, build_replace_all, scan_lines};
use crate::time::Tick;

use super::file::Pending;
use super::layout::cell_text;
use super::search_state::{Found, SearchCount, SearchStep};
use super::state::EditorState;

impl SearchBarState {
    /// 今の入力とトグルから作る問い合わせ。
    pub(super) fn query(&self) -> SearchQuery {
        SearchQuery::literal(&self.text)
            .with_case(self.case)
            .with_regex(self.regex)
            .with_whole_word(self.whole_word)
    }
}

/// 可視行の強調で、1 行あたり検索する上限 (`SEARCH_SYNC_LINE_MAX`、03 §12.2)。これを超える行は強調しない。
const SYNC_LINE_MAX: usize = 16 * 1024;

#[derive(Clone, Debug)]
pub(super) struct SearchBarState {
    pub text: String,
    /// `text` 内のカーソル (バイト。書記素境界)。
    pub cursor: usize,
    pub case: Case,
    /// 正規表現として探す (`search.toggle_regex`)。バーを開くたびに off から始まる。
    pub regex: bool,
    /// 単語単位 (`search.toggle_word`)。同じく off から始まる。
    pub whole_word: bool,
    /// 検索語をコンパイルできない理由。あれば検索しない (03 §12.3)。
    pub error: Option<QueryError>,
    /// 入力しているときの検索の起点 (開いたときの選択の始端。一致へ移ったらその一致の始端)。
    pub anchor: ByteOff,
    /// 入力で見つかった、まだ選択していない現在の一致。
    pub current: Option<ByteRange>,
    /// 置換欄 (開いているとき。テキストとカーソル)。
    pub replace: Option<(String, usize)>,
    pub focus: BarFocus,
}

/// すべて置換をジョブで作るときの境界 (`SEARCH_SYNC_STEP_BYTES`、03 §12.2)。これ以下の文書はメインスレッドで作る。
const REPLACE_SYNC_BYTES: usize = 1 << 20;

impl EditorState {
    /// 検索バーが開いているか (条件式 `search.active`)。
    pub fn search_bar_open(&self) -> bool {
        self.search_bar.is_some()
    }

    /// 検索バーを開く。開いていれば何もしない (欄はそのまま)。
    pub fn search_open(&mut self) -> CommandResult {
        if self.search_bar.is_some() {
            return CommandResult::Done;
        }
        let anchor = self.bar_cursor(false).unwrap_or(ByteOff(0));
        self.search_bar = Some(SearchBarState {
            text: String::new(),
            cursor: 0,
            case: Case::Smart,
            regex: false,
            whole_word: false,
            error: None,
            anchor,
            current: None,
            replace: None,
            focus: BarFocus::Search,
        });
        self.set_bar_rows();
        CommandResult::Done
    }

    /// `search.replace_open`: 置換欄つきで開く (開いていれば置換欄を足す)。読み取り専用の文書では開かない。
    pub fn search_replace_open(&mut self) -> CommandResult {
        let read_only = self
            .active_document()
            .is_some_and(|d| self.documents.get(&d).is_some_and(|d| !d.is_editable()));
        if read_only {
            self.notify_read_only();
            return CommandResult::Done;
        }
        self.search_open();
        if let Some(bar) = &mut self.search_bar {
            if bar.replace.is_none() {
                bar.replace = Some((String::new(), 0));
            }
            // 検索語が空なら検索欄、あれば置換欄にフォーカスを置く
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
    pub(super) fn bar_focus_cycle(&mut self, forward: bool) {
        let Some(bar) = &mut self.search_bar else {
            return;
        };
        if bar.replace.is_none() {
            return;
        }
        let order = [BarFocus::Search, BarFocus::Replace, BarFocus::Button];
        let i = order.iter().position(|f| *f == bar.focus).unwrap_or(0);
        bar.focus = order[if forward { (i + 1) % 3 } else { (i + 2) % 3 }];
        self.mark_dirty_render();
    }

    /// 入力欄を編集した: 検索欄なら検索をやり直す。
    pub(super) fn bar_edited(&mut self) {
        if self
            .search_bar
            .as_ref()
            .is_some_and(|b| b.focus == BarFocus::Search)
        {
            self.bar_changed();
        } else {
            self.mark_dirty_render();
        }
    }

    /// 検索バーを閉じる。確定していない現在の一致は選択として残さない。
    pub fn search_close(&mut self) {
        if self.search_bar.take().is_none() {
            return;
        }
        self.search_stop();
        self.cancel_replace_job();
        self.set_bar_rows();
    }

    /// 検索バーの行数 (検索欄 1 行 + 置換欄があれば 1 行、閉じていれば 0)。編集面はその分だけ縮む。
    fn set_bar_rows(&mut self) {
        self.bar_rows = self
            .search_bar
            .as_ref()
            .map_or(0, |b| 1 + u16::from(b.replace.is_some()));
        let (cols, rows) = self.surface;
        self.apply_viewport(cols, rows);
        self.mark_dirty_render();
    }

    /// 選択の端 (`end` が偽なら始端)。
    fn bar_cursor(&self, end: bool) -> Option<ByteOff> {
        let v = self.view(self.window_view(self.first_window()?)?)?;
        let r = v.selection.primary();
        Some(if end { r.end() } else { r.start() })
    }

    /// 大小の区別を巡回する (Smart → Sensitive → Insensitive → Smart) 。切り替えたら再検索する。
    pub fn search_toggle_case(&mut self) -> CommandResult {
        let Some(bar) = &mut self.search_bar else {
            return CommandResult::Rejected(super::command::RejectReason::NotAvailable);
        };
        bar.case = bar.case.next();
        self.bar_changed();
        CommandResult::Done
    }

    /// 正規表現の ON / OFF を切り替える。切り替えたら再検索する。
    pub fn search_toggle_regex(&mut self) -> CommandResult {
        let Some(bar) = &mut self.search_bar else {
            return CommandResult::Rejected(super::command::RejectReason::NotAvailable);
        };
        bar.regex = !bar.regex;
        self.bar_changed();
        CommandResult::Done
    }

    /// 単語単位の ON / OFF を切り替える。切り替えたら再検索する。
    pub fn search_toggle_word(&mut self) -> CommandResult {
        let Some(bar) = &mut self.search_bar else {
            return CommandResult::Rejected(super::command::RejectReason::NotAvailable);
        };
        bar.whole_word = !bar.whole_word;
        self.bar_changed();
        CommandResult::Done
    }

    /// 入力欄・トグルが変わった: 検索をやり直し、起点から先の最初の一致を強調する。
    /// 検索語をコンパイルできないとき (正規表現の構文エラーなど) は検索せず、理由を示す。
    pub(super) fn bar_changed(&mut self) {
        let Some(bar) = &self.search_bar else {
            return;
        };
        let query = bar.query();
        let anchor = bar.anchor;
        let error = query.compile().err();
        if let Some(bar) = &mut self.search_bar {
            bar.error = error.clone();
        }
        if error.is_some() {
            // 前の検索の結果を残さない
            self.search_start(SearchQuery::literal(""));
            if let Some(bar) = &mut self.search_bar {
                bar.current = None;
            }
            self.mark_dirty_render();
            return;
        }
        self.search_start(query.clone());
        let found = match (self.active_document(), query.pattern().is_empty()) {
            (Some(doc), false) => match self.sync_find(doc, &query, anchor, true) {
                Found::Hit(m, _) => Some(m),
                _ => None,
            },
            _ => None,
        };
        if let Some(bar) = &mut self.search_bar {
            bar.current = found;
        }
        if let Some(m) = found {
            self.reveal(m.start);
        }
        self.mark_dirty_render();
    }

    /// 選択を動かさずに、`pos` が見えるようスクロールする。
    fn reveal(&mut self, pos: ByteOff) {
        let Some(view) = self.first_window().and_then(|w| self.window_view(w)) else {
            return;
        };
        let Some(saved) = self.view(view).map(|v| v.selection.clone()) else {
            return;
        };
        self.set_selection(view, Selection::point(pos));
        self.ensure_cursor_visible(view);
        if let Some(v) = self.views.get_mut(&view) {
            v.selection = saved;
        }
    }

    /// `Enter` / `↓` / `F3` (`search.next`) と `↑` (`search.prev`)。入力で強調した一致があれば、まずそれを選択する。
    pub fn search_move(&mut self, forward: bool) -> SearchStep {
        let Some(bar) = &self.search_bar else {
            return SearchStep::NoQuery;
        };
        // ボタンにフォーカスがあるときの Enter はすべて置換 (Alt が使えない端末向け、04 §8.4)
        if bar.focus == BarFocus::Button {
            self.search_replace_all();
            return SearchStep::NoQuery;
        }
        if forward
            && let Some(cur) = bar.current
            && let Some(view) = self.first_window().and_then(|w| self.window_view(w))
        {
            self.set_selection(
                view,
                Selection::single(eve_text::Range::new(cur.start, cur.end)),
            );
            self.ensure_cursor_visible(view);
            if let Some(bar) = &mut self.search_bar {
                bar.current = None;
                bar.anchor = cur.start;
            }
            self.mark_dirty_render();
            return SearchStep::Moved { wrapped: false };
        }
        let r = self.search_step(forward);
        if matches!(r, SearchStep::Moved { .. }) {
            let a = self.bar_cursor(false);
            if let (Some(bar), Some(a)) = (&mut self.search_bar, a) {
                bar.current = None;
                bar.anchor = a;
            }
        }
        r
    }

    /// 検索バーの表示モデル。
    pub(super) fn search_bar_model(&self) -> Option<SearchBarModel> {
        let bar = self.search_bar.as_ref()?;
        let render = |key: &str, args: &[(&str, &str)]| {
            self.catalog().render(key, args, &|c| self.key_label(c))
        };
        let cells = cell_text(&bar.text, CutPolicy::End).spans;
        let cursor = cell_text(&bar.text[..bar.cursor], CutPolicy::End)
            .spans
            .len();
        let count_text = match self.search_count_for(bar.current) {
            SearchCount::Idle => None,
            SearchCount::Running { found } => Some(render(
                "ui.search.count.running",
                &[("found", &found.to_string())],
            )),
            SearchCount::Exact { current, total } | SearchCount::AtLeast { current, total } => {
                let at_least = matches!(
                    self.search_count_for(bar.current),
                    SearchCount::AtLeast { .. }
                );
                Some(if total == 0 {
                    render("ui.search.count.no_match", &[])
                } else {
                    let cur = current.map_or("-".to_owned(), |c| c.to_string());
                    let key = if at_least {
                        "ui.search.count.at_least"
                    } else {
                        "ui.search.count.exact"
                    };
                    render(key, &[("current", &cur), ("total", &total.to_string())])
                })
            }
            SearchCount::Approx { total } => Some(render(
                "ui.search.count.approx",
                &[("total", &total.to_string())],
            )),
        };
        let query = bar.query();
        let error = bar.error.as_ref().map(|e| {
            let text = match e {
                QueryError::Syntax(reason) => {
                    render("msg.search.invalid_regex.text", &[("reason", reason)])
                }
                QueryError::Multiline => render("msg.search.newline_unsupported.text", &[]),
                QueryError::TooLarge => render(
                    "msg.search.invalid_regex.text",
                    &[("reason", "pattern too large")],
                ),
            };
            cell_text(&text, CutPolicy::End)
        });
        let replace = bar.replace.as_ref().map(|(text, cur)| {
            let cells = cell_text(text, CutPolicy::End).spans;
            let cursor = cell_text(&text[..*cur], CutPolicy::End).spans.len();
            ReplaceRowModel {
                label: cell_text(&render("ui.search.replace_label", &[]), CutPolicy::Never),
                input: InputFieldModel {
                    len: cells.len(),
                    cells,
                    first: 0,
                    cursor,
                    selection: None,
                    placeholder: None,
                },
                button: cell_text(
                    &render("ui.search.replace_all_button", &[]),
                    CutPolicy::Never,
                ),
            }
        });
        Some(SearchBarModel {
            replace,
            focus: bar.focus,
            label: cell_text(&render("ui.search.label", &[]), CutPolicy::Never),
            input: InputFieldModel {
                len: cells.len(),
                cells,
                first: 0,
                cursor,
                selection: None,
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
            error,
            count_text: count_text.map(|t| cell_text(&t, CutPolicy::End)),
        })
    }

    /// 可視行の一致 (強調する範囲)。保持した一致ではなく、可視行だけを同期検索する (03 §12.2 の「可視範囲のハイライト」)。
    /// 1 行が `SYNC_LINE_MAX` を超える行は強調しない。検索バーが開いていなければ空。
    pub(super) fn visible_matches(
        &self,
        doc_id: eve_view::DocumentId,
        top_line: usize,
        rows: usize,
    ) -> Vec<(usize, usize)> {
        let Some(bar) = &self.search_bar else {
            return Vec::new();
        };
        let Some(m) = bar.query().matcher() else {
            return Vec::new();
        };
        let Some(doc) = self.documents.get(&doc_id) else {
            return Vec::new();
        };
        let t = doc.text();
        let n = t.len_lines();
        let mut out = Vec::new();
        let cancel = CancelToken::new();
        for l in top_line..(top_line + rows).min(n) {
            let len = t.line_end_byte(LineIdx(l)).0 - t.line_to_byte(LineIdx(l)).0;
            if len > SYNC_LINE_MAX {
                continue;
            }
            scan_lines(t, &m, l..l + 1, &cancel, &mut |r| {
                out.push((r.start.0, r.end.0));
                true
            });
        }
        out
    }

    /// 入力で強調している現在の一致 (文書内のバイト範囲)。
    pub(super) fn bar_current(&self) -> Option<(usize, usize)> {
        self.search_bar
            .as_ref()?
            .current
            .map(|r| (r.start.0, r.end.0))
    }
}

impl EditorState {
    fn cancel_replace_job(&mut self) {
        let ids: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, p)| matches!(p, Pending::ReplaceAll { .. }))
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.pending.remove(&id);
            self.jobs.cancel(id);
        }
    }

    /// `search.replace_all` (03 §12.5): 検索欄の検索語・大小の区別で、置換欄の文字列にすべて置き換える。1 つの Undo 単位。
    /// 文書が 1 MiB 以下ならメインスレッドで、超えればジョブで ChangeSet を作る。
    pub fn search_replace_all(&mut self) -> CommandResult {
        let Some(bar) = &self.search_bar else {
            return CommandResult::Rejected(super::command::RejectReason::NotAvailable);
        };
        let Some((replacement, _)) = bar.replace.clone() else {
            return CommandResult::Rejected(super::command::RejectReason::NotAvailable);
        };
        let query = bar.query();
        let Some(doc_id) = self.active_document() else {
            return CommandResult::Done;
        };
        let Some(doc) = self.documents.get(&doc_id) else {
            return CommandResult::Done;
        };
        if !doc.is_editable() {
            self.notify_read_only();
            return CommandResult::Done;
        }
        if query.matcher().is_none() {
            self.show_message(MessageLevel::Info, "info.search.replace_empty_query", &[]);
            return CommandResult::Done;
        }
        // 実行中のすべて置換があれば二重に始めない
        if self
            .pending
            .values()
            .any(|p| matches!(p, Pending::ReplaceAll { .. }))
        {
            return CommandResult::Done;
        }
        let revision = doc.revision();
        if doc.text().len_bytes() <= REPLACE_SYNC_BYTES {
            let built = build_replace_all(doc.text(), &query, &replacement, &CancelToken::new());
            if let Some((changes, count)) = built {
                self.apply_replace_all(doc_id, changes, count);
            }
            return CommandResult::Done;
        }
        let snapshot = doc.snapshot();
        let job = self.jobs.spawn(
            Some(doc_id),
            revision,
            JobKind::ReplaceAll {
                snapshot,
                query,
                replacement,
            },
        );
        self.pending.insert(
            job,
            Pending::ReplaceAll {
                doc: doc_id,
                revision,
            },
        );
        CommandResult::Done
    }

    /// すべて置換のジョブの結果。作った Revision が古ければ適用せず中止を知らせる。
    pub(super) fn finish_replace_all(
        &mut self,
        doc_id: eve_view::DocumentId,
        revision: eve_view::Revision,
        r: JobResult,
    ) {
        if let JobResult::ReplaceAllDone { changes, count, .. } = r {
            let fresh = self
                .documents
                .get(&doc_id)
                .is_some_and(|d| d.revision() == revision);
            if fresh {
                self.apply_replace_all(doc_id, changes, count);
            } else {
                self.show_message(MessageLevel::Warn, "msg.search.replace_stale", &[]);
            }
        }
    }

    /// ChangeSet を 1 つの Transaction (`Origin::Command("search.replace_all")`、Undo 1 回分) として適用する。
    fn apply_replace_all(
        &mut self,
        doc_id: eve_view::DocumentId,
        changes: eve_text::ChangeSet,
        count: u64,
    ) {
        if count == 0 {
            self.show_message(MessageLevel::Info, "info.search.replace_no_match", &[]);
            return;
        }
        let view = self.first_window().and_then(|w| self.window_view(w));
        let sel = view.and_then(|v| self.views.get(&v).map(|v| v.selection.clone()));
        let mut tx = Transaction::new(
            changes,
            Origin::Command(eve_view::CommandId::new("search.replace_all")),
            self.now,
        )
        .with_undo_group(UndoGroupHint::Break)
        .with_selections(sel, None);
        if let Some(v) = view {
            tx = tx.with_view(v);
        }
        if self.apply_transaction(doc_id, tx).is_err() {
            self.show_message(MessageLevel::Warn, "msg.search.replace_stale", &[]);
            return;
        }
        self.show_message(
            MessageLevel::Info,
            "info.search.replaced",
            &[("count", &count.to_string())],
        );
        // 保持した一致は編集で無効になった: 検索をやり直す
        self.bar_changed();
    }
}

/// 編集が止まってから再検索するまでの間 (03 §12.2「編集後の結果」)。
const RESEARCH_DELAY: u64 = 300;

impl EditorState {
    /// 編集で検索の結果が無効になったら、入力が止まって 300 ms 後に再検索する。`Editor::tick` から呼ぶ。
    pub(super) fn search_tick(&mut self) {
        let stale = self.search_bar.is_some() && self.search.is_some() && !self.search_fresh();
        let cur = self
            .search
            .as_ref()
            .and_then(|s| self.documents.get(&s.doc()).map(|d| d.revision()));
        let (true, Some(cur)) = (stale, cur) else {
            self.research = None;
            return;
        };
        match self.research {
            Some((rev, at)) if rev == cur => {
                if self.now.saturating_sub(at) >= crate::time::Millis(RESEARCH_DELAY) {
                    self.research = None;
                    self.bar_changed();
                }
            }
            // 初めて気づいた、または気づいた後にまた編集された: 待ち直す
            _ => self.research = Some((cur, self.now)),
        }
    }

    /// 次に `tick` が要る時刻 (再検索の期限)。
    pub(super) fn search_deadline(&self) -> Option<Tick> {
        self.research.map(|(_, at)| Tick(at.0 + RESEARCH_DELAY))
    }
}
