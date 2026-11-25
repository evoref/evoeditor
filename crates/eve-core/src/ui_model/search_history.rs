//! 検索履歴 (docs/03 §12.8 の SHI-1〜SHI-6、01 FR-407。v0.5 の M34 の段 9)。
//!
//! 検索語の履歴 (検索バーの検索欄と横断検索の入力欄で共有) と置換の文字列の履歴 (検索バーの置換欄) を、プロセスに 1 つずつ
//! メモリだけに持つ (ディスクに書かない。`Debug` にも中身を出さない、SHI-5)。`ui.history_prev` / `ui.history_next` で
//! フォーカスのある欄の文字列を 1 つ古い / 新しい項目に置き換える (SHI-4)。
//!
//! spec: 03 §12.8, 07 §11.6, 01 FR-407

use std::collections::VecDeque;
use std::fmt;

use eve_view::BarFocus;

use crate::command::{
    CommandCtx, CommandKind, CommandRegistry, CommandResult, Permission, RejectReason, ids,
};
use crate::editor::state::EditorState;
use crate::ui_model::overlay::Overlay;

/// 1 つの履歴に持つ項目の上限 (02 §14.1 の MR8 の「検索履歴 100 件」、03 §12.8 の SHI-3)。超えたら古いものから捨てる。
pub(crate) const SEARCH_HISTORY_MAX: usize = 100;

/// 履歴に記録する文字列の大きさの上限 (バイト。`SEARCH_HISTORY_ENTRY_MAX_BYTES`、SHI-2)。これを超える文字列は記録しない。
pub(crate) const SEARCH_HISTORY_ENTRY_MAX_BYTES: usize = 4 * 1024;

/// `ui.history_prev` / `ui.history_next` が効く欄 (検索バーの欄と横断検索の入力欄、SHI-6)。既定キーの割当の `when` にも
/// 使う (効かない欄ではキーを下の層へ進める)。
pub(crate) const HISTORY_WHEN: &str =
    "(overlay.active == \"\" && search.active) || overlay.active == \"workspace_search\"";

/// 1 つの履歴 (古い順)。記録するたびに世代を進める (呼び出しの途中で履歴が変わったことを、入力欄の [`Recall`] が知る)。
#[derive(Clone, Default)]
pub(crate) struct HistoryList {
    entries: VecDeque<String>,
    generation: u64,
}

impl HistoryList {
    /// `s` を最新の項目として記録する (SHI-2 / SHI-3)。空の文字列と上限を超える文字列は記録しない。同じ文字列 (大小を
    /// 区別して比べる) があれば、それを消して最新に置く。上限を超えたら最も古い項目を捨てる。
    fn record(&mut self, s: &str) {
        if s.is_empty() || s.len() > SEARCH_HISTORY_ENTRY_MAX_BYTES {
            return;
        }
        if self.entries.back().is_some_and(|last| last == s) {
            return;
        }
        if let Some(i) = self.entries.iter().position(|e| e == s) {
            self.entries.remove(i);
        }
        self.entries.push_back(s.to_owned());
        while self.entries.len() > SEARCH_HISTORY_MAX {
            self.entries.pop_front();
        }
        self.generation = self.generation.wrapping_add(1);
    }

    /// 古い順の項目。
    pub(crate) fn iter(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(String::as_str)
    }
}

/// 中身 (利用者の検索語) は出さない (SHI-5。検索語に秘密を含み得る)。
impl fmt::Debug for HistoryList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HistoryList")
            .field("len", &self.entries.len())
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

/// 検索の 2 つの履歴 (プロセスに 1 つ、SHI-1)。
#[derive(Clone, Debug, Default)]
pub(crate) struct SearchHistory {
    /// 検索語 (検索バーの検索欄と横断検索の入力欄で共有する)。
    pub(crate) terms: HistoryList,
    /// 置換の文字列 (検索バーの置換欄)。
    pub(crate) replacements: HistoryList,
}

/// 呼び出しの向き。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Direction {
    /// 古い側 (`ui.history_prev`)。
    Older,
    /// 新しい側 (`ui.history_next`)。
    Newer,
}

/// 入力欄の呼び出しの位置 (欄ごと)。呼び出していなければ `None` (欄の文字列は利用者が打ったもの)。
#[derive(Clone, Default)]
pub(crate) struct Recall {
    at: Option<RecallAt>,
}

#[derive(Clone)]
struct RecallAt {
    /// 欄に入れている項目の添字 (古い順)。
    index: usize,
    /// 最初の `ui.history_prev` の時点の欄の文字列 (最新より新しい側へ戻ると欄に戻す)。
    draft: String,
    /// 位置を決めたときの履歴の世代。違えば履歴が変わったので、呼び出していないときと同じに扱う。
    generation: u64,
}

impl Recall {
    /// 呼び出しを止める (欄を編集した・開き直した。下書きも捨てる)。
    pub(crate) fn reset(&mut self) {
        self.at = None;
    }

    /// 履歴 `list` の中を `dir` へ 1 つ動き、欄に入れる文字列を返す (`current` は欄の今の文字列)。端なら `None`
    /// (何もしない)。欄の今の文字列と同じ項目は飛ばす (押しても欄が変わらないように見せない)。最新の項目より新しい側へ
    /// 戻ると下書きを返し、呼び出しを止める。
    pub(crate) fn step(
        &mut self,
        list: &HistoryList,
        current: &str,
        dir: Direction,
    ) -> Option<String> {
        let len = list.entries.len();
        // 世代が同じなら続きから (添字だけを書き換える)。違えば (呼び出していない・確定で履歴が変わった) 最新の 1 つ先
        // (下書きの位置) から数える
        if self
            .at
            .as_ref()
            .is_some_and(|at| at.generation != list.generation || at.index >= len)
        {
            self.at = None;
        }
        let mut i = self.at.as_ref().map_or(len, |at| at.index);
        match dir {
            Direction::Older => loop {
                if i == 0 {
                    return None;
                }
                i -= 1;
                if list.entries[i] != current {
                    break;
                }
            },
            Direction::Newer => loop {
                if i >= len {
                    return None;
                }
                i += 1;
                if i == len {
                    // 下書きに戻る (呼び出しを止める)
                    return self.at.take().map(|at| at.draft);
                }
                if list.entries[i] != current {
                    break;
                }
            },
        }
        match &mut self.at {
            Some(at) => at.index = i,
            None => {
                self.at = Some(RecallAt {
                    index: i,
                    draft: current.to_owned(),
                    generation: list.generation,
                });
            }
        }
        Some(list.entries[i].clone())
    }
}

/// 下書きは出さない (SHI-5)。
impl fmt::Debug for Recall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Recall")
            .field("index", &self.at.as_ref().map(|a| a.index))
            .finish_non_exhaustive()
    }
}

fn cmd_history_prev(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.history_recall(Direction::Older)
}

fn cmd_history_next(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.history_recall(Direction::Newer)
}

/// `ui.history_prev` / `ui.history_next` (07 §11.6 の「編集の拡張」)。
pub(crate) fn register(r: &mut CommandRegistry) {
    for (id, h) in [
        (ids::UI_HISTORY_PREV, cmd_history_prev as crate::Handler),
        (ids::UI_HISTORY_NEXT, cmd_history_next),
    ] {
        r.add(id, CommandKind::Ui, Permission::None, HISTORY_WHEN, h);
    }
}

impl EditorState {
    /// 検索バーの今の検索語を検索語の履歴に記録する (SHI-2: `search.next` / `search.prev`・置換の実行)。
    pub(crate) fn record_search_term(&mut self) {
        if let Some(bar) = &self.find.search_bar {
            self.find.history.terms.record(&bar.text);
        }
    }

    /// 検索バーの今の検索語と置換の文字列を記録する (SHI-2: `search.replace` / `search.replace_all`)。置換欄が無ければ
    /// 何もしない (置換を実行できない)。
    pub(crate) fn record_replace(&mut self) {
        let Some(bar) = &self.find.search_bar else {
            return;
        };
        let Some((replacement, _)) = &bar.replace else {
            return;
        };
        self.find.history.terms.record(&bar.text);
        self.find.history.replacements.record(replacement);
    }

    /// 横断検索で走査した検索語を記録する (SHI-2。オーバーレイを閉じるとき、03 §12.8 の段 9 の細目)。
    pub(crate) fn record_workspace_term(&mut self, term: &str) {
        self.find.history.terms.record(term);
    }

    /// `ui.history_prev` / `ui.history_next` (SHI-4): フォーカスのある欄の文字列を履歴の 1 つ古い / 新しい項目に置き換える
    /// (欄の全体を置き換え、カーソルを末尾へ、欄内全選択を解く)。置き換えたら、打ったときと同じく逐次検索・横断検索を
    /// やり直す。端・ボタンにフォーカスがあるときは何もしない (知らせない)。
    pub(crate) fn history_recall(&mut self, dir: Direction) -> CommandResult {
        let history = &self.find.history;
        match &mut self.ui.overlay {
            Some(Overlay::WorkspaceSearch(w)) => {
                let Some(text) = w.recall.step(&history.terms, &w.text, dir) else {
                    return CommandResult::Done;
                };
                let changed = text != w.text;
                w.cursor = text.len();
                w.text = text;
                w.all_selected = false;
                if changed {
                    // 呼び出しで眺めるだけの走査は、閉じるときに記録しない (利用者が編集すれば印が立つ、03 §12.8.1)
                    w.searched = false;
                    self.ws_search_start();
                }
            }
            None => {
                let Some(bar) = &mut self.find.search_bar else {
                    return CommandResult::Rejected(RejectReason::NotAvailable);
                };
                let (list, recall, field) = match bar.focus {
                    BarFocus::Search => (&history.terms, &mut bar.recall_search, &mut bar.text),
                    BarFocus::Replace => match &mut bar.replace {
                        Some((t, _)) => (&history.replacements, &mut bar.recall_replace, t),
                        None => return CommandResult::Done,
                    },
                    BarFocus::Button => return CommandResult::Done,
                };
                let Some(text) = recall.step(list, field, dir) else {
                    return CommandResult::Done;
                };
                let changed = text != *field;
                let end = text.len();
                *field = text;
                match bar.focus {
                    BarFocus::Replace => {
                        if let Some((_, c)) = &mut bar.replace {
                            *c = end;
                        }
                    }
                    _ => bar.cursor = end,
                }
                bar.all_selected = false;
                if changed {
                    self.bar_edited();
                }
            }
            _ => return CommandResult::Rejected(RejectReason::NotAvailable),
        }
        self.mark_dirty_render();
        CommandResult::Done
    }

    /// フォーカスのある入力欄の呼び出しを止める (欄を編集した。SHI-4 の細目: 編集した文字列が次の下書きになる)。
    pub(crate) fn reset_field_recall(&mut self) {
        match &mut self.ui.overlay {
            Some(Overlay::WorkspaceSearch(w)) => w.recall.reset(),
            None => {
                if let Some(bar) = &mut self.find.search_bar {
                    match bar.focus {
                        BarFocus::Search => bar.recall_search.reset(),
                        BarFocus::Replace => bar.recall_replace.reset(),
                        BarFocus::Button => {}
                    }
                }
            }
            _ => {}
        }
    }

    /// 検索語の履歴 (古い順。試験用)。
    #[doc(hidden)]
    pub fn search_term_history(&self) -> Vec<String> {
        self.find.history.terms.iter().map(str::to_owned).collect()
    }

    /// 置換の文字列の履歴 (古い順。試験用)。
    #[doc(hidden)]
    pub fn replace_history(&self) -> Vec<String> {
        self.find
            .history
            .replacements
            .iter()
            .map(str::to_owned)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(items: &[&str]) -> HistoryList {
        let mut l = HistoryList::default();
        for s in items {
            l.record(s);
        }
        l
    }

    fn entries(l: &HistoryList) -> Vec<&str> {
        l.iter().collect()
    }

    // spec: 03 §12.8 (SHI-2)
    #[test]
    fn 空と上限を超える文字列は記録せず_同じ文字列は最新へ移す() {
        let mut l = list(&["a", "b", "c"]);
        l.record("");
        l.record("b");
        assert_eq!(entries(&l), ["a", "c", "b"]);
        // 大小を区別して比べる
        l.record("B");
        assert_eq!(entries(&l), ["a", "c", "b", "B"]);
        let ok = "x".repeat(SEARCH_HISTORY_ENTRY_MAX_BYTES);
        let over = "y".repeat(SEARCH_HISTORY_ENTRY_MAX_BYTES + 1);
        l.record(&ok);
        l.record(&over);
        assert_eq!(l.iter().count(), 5);
        assert_eq!(l.iter().last(), Some(ok.as_str()));
    }

    // spec: 03 §12.8 (SHI-3)
    #[test]
    fn 上限の_100_件を超えたら古いものから捨てる() {
        let mut l = HistoryList::default();
        for i in 0..=SEARCH_HISTORY_MAX {
            l.record(&format!("t{i}"));
        }
        assert_eq!(l.iter().count(), SEARCH_HISTORY_MAX);
        assert_eq!(l.iter().next(), Some("t1"));
        assert_eq!(l.iter().last(), Some("t100"));
    }

    // spec: 03 §12.8 (SHI-4)
    #[test]
    fn 前と次で項目をたどり_端では何もせず_新しい側の先で下書きに戻る() {
        let l = list(&["a", "b", "c"]);
        let mut r = Recall::default();
        assert_eq!(
            r.step(&l, "draft", Direction::Newer),
            None,
            "呼び出す前の次は端"
        );
        assert_eq!(r.step(&l, "draft", Direction::Older).as_deref(), Some("c"));
        assert_eq!(r.step(&l, "c", Direction::Older).as_deref(), Some("b"));
        assert_eq!(r.step(&l, "b", Direction::Older).as_deref(), Some("a"));
        assert_eq!(
            r.step(&l, "a", Direction::Older),
            None,
            "最も古い項目の先は端"
        );
        assert_eq!(r.step(&l, "a", Direction::Newer).as_deref(), Some("b"));
        assert_eq!(r.step(&l, "b", Direction::Newer).as_deref(), Some("c"));
        assert_eq!(r.step(&l, "c", Direction::Newer).as_deref(), Some("draft"));
        assert_eq!(r.step(&l, "draft", Direction::Newer), None);
    }

    // spec: 03 §12.8 (SHI-4)
    #[test]
    fn 欄と同じ項目は飛ばし_履歴が変わったら最新から数え直す() {
        let mut l = list(&["a", "b"]);
        let mut r = Recall::default();
        // 欄が最新の項目と同じ (確定した直後) なら、1 つ前の項目を出す
        assert_eq!(r.step(&l, "b", Direction::Older).as_deref(), Some("a"));
        assert_eq!(r.step(&l, "a", Direction::Newer).as_deref(), Some("b"));
        // 呼び出した "a" を確定して記録すると、次の前は今の欄を下書きにして最新から数える
        let mut r = Recall::default();
        assert_eq!(r.step(&l, "", Direction::Older).as_deref(), Some("b"));
        assert_eq!(r.step(&l, "b", Direction::Older).as_deref(), Some("a"));
        l.record("a");
        assert_eq!(
            r.step(&l, "a", Direction::Newer),
            None,
            "下書きの位置なので端"
        );
        assert_eq!(r.step(&l, "a", Direction::Older).as_deref(), Some("b"));
        // 項目が 1 つで欄と同じなら端
        let one = list(&["x"]);
        assert_eq!(Recall::default().step(&one, "x", Direction::Older), None);
    }

    // spec: 03 §12.8 (SHI-5)
    #[test]
    fn debug_に履歴と下書きの中身を出さない() {
        let mut h = SearchHistory::default();
        h.terms.record("s3cret-term");
        h.replacements.record("s3cret-repl");
        let mut r = Recall::default();
        r.step(&h.terms, "s3cret-draft", Direction::Older);
        let shown = format!("{h:?} {r:?}");
        assert!(!shown.contains("s3cret"), "{shown}");
    }
}
