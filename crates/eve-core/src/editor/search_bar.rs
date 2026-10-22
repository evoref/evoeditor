//! 検索バー (docs/04 §8.4、03 §12、01 FR-401〜404)。
//!
//! 開いている間は編集面を 1 行縮め、キー入力はバーの入力欄へ行く (コンテキスト `search`)。入力のたびに検索をやり直し、
//! 最初の一致 (開いたときの位置から先) を **強調するだけで選択はしない**。`Enter` / `↓` / `F3` で一致が主選択になる (確定)。
//! `Esc` で閉じると、確定していない一致は選択として残さない。

use eve_text::{ByteOff, ByteRange, LineIdx, Selection, TextRead};
use eve_view::{CaseMode, CutPolicy, InputFieldModel, SearchBarModel};

use crate::cancel::CancelToken;
use crate::editor::command::CommandResult;
use crate::search::{Case, SearchQuery, scan_lines};

use super::layout::cell_text;
use super::search_state::{Found, SearchCount, SearchStep};
use super::state::EditorState;

/// 可視行の強調で、1 行あたり検索する上限 (`SEARCH_SYNC_LINE_MAX`、03 §12.2)。これを超える行は強調しない。
const SYNC_LINE_MAX: usize = 16 * 1024;

#[derive(Clone, Debug)]
pub(super) struct SearchBarState {
    pub text: String,
    /// `text` 内のカーソル (バイト。書記素境界)。
    pub cursor: usize,
    pub case: Case,
    /// 入力しているときの検索の起点 (開いたときの選択の始端。一致へ移ったらその一致の始端)。
    pub anchor: ByteOff,
    /// 入力で見つかった、まだ選択していない現在の一致。
    pub current: Option<ByteRange>,
}

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
            anchor,
            current: None,
        });
        self.set_bar_rows(1);
        CommandResult::Done
    }

    /// 検索バーを閉じる。確定していない現在の一致は選択として残さない。
    pub fn search_close(&mut self) {
        if self.search_bar.take().is_none() {
            return;
        }
        self.search_stop();
        self.set_bar_rows(0);
    }

    fn set_bar_rows(&mut self, n: u16) {
        self.bar_rows = n;
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

    /// 入力欄・大小の区別が変わった: 検索をやり直し、起点から先の最初の一致を強調する。
    pub(super) fn bar_changed(&mut self) {
        let Some(bar) = &self.search_bar else {
            return;
        };
        let query = SearchQuery::literal(&bar.text).with_case(bar.case);
        let anchor = bar.anchor;
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
        let query = SearchQuery::literal(&bar.text).with_case(bar.case);
        Some(SearchBarModel {
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
        let Some(m) = SearchQuery::literal(&bar.text)
            .with_case(bar.case)
            .matcher()
        else {
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
