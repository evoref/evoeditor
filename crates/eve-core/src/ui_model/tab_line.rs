//! タブライン (FR-510、docs/03 §9.9 の PANE-12、04 §3.4 (f))。開いている文書の一覧を画面の最上行に出す。
//!
//! core が決めるのは、出すか (`ui.tab_line` と文書の数と編集面の行数) と、タブの中身 (開いた順・アクティブ・名前と印)。
//! 並べ方 (どの範囲を出すか・省いた側の印) は frontend が決める (PANE-12 (e))。タブの項目はペインの見出しと同じ
//! `StatusItem` (`Path` = PANE-11 の名前・`Dirty`・`ReadOnly`) で、組み立ては `chrome::document_items` と共有する。
//!
//! spec: 03 §9.9 (PANE-7, PANE-11, PANE-12)

use std::collections::BTreeMap;

use eve_view::{DocumentId, TabItem, TabLineModel};

use crate::editor::state::EditorState;

/// タブラインを出したときに残す編集面の最小の行数 (PANE-12 (b))。タブラインを出すと編集面がこれを割るときは出さない
/// (frontend の最小 `min_surface` に近い端末。ペインの本文の最小の 3 行 (PANE-4) と同じ値)。
pub(crate) const TAB_LINE_MIN_SURFACE: u16 = 3;

impl EditorState {
    /// タブラインに出す文書 (開いた順 = `buffer.next` と同じ `DocumentId` の昇順)。置き換えで読込中の文書は、まだ表示して
    /// いない間は数えない (ステータスバーの `(2/3)` と同じ集合、03 §9.8 の「読込中のバッファ」)。
    pub(crate) fn tab_documents(&self) -> Vec<DocumentId> {
        self.tab_documents_iter().collect()
    }

    fn tab_documents_iter(&self) -> impl Iterator<Item = DocumentId> + '_ {
        let active = self.active_document();
        self.docs
            .documents
            .keys()
            .copied()
            .filter(move |d| Some(*d) == active || !self.is_loading_replacement(*d))
    }

    /// 設定 (`ui.tab_line`) と文書の数から、タブラインを出したいか (編集面の行数の条件は `apply_viewport` が見る)。
    /// 入力のたびに呼ぶので、文書を 2 つ数えたら止める。
    pub(crate) fn tab_line_wanted(&self) -> bool {
        match self.config().tab_line {
            eve_config::TabLineMode::Never => false,
            eve_config::TabLineMode::Always => true,
            eve_config::TabLineMode::Auto => self.tab_documents_iter().nth(1).is_some(),
        }
    }

    /// タブのクリック (`ChromeTarget::Tab`、PANE-7 (e) / PANE-12 (g))。`doc` をフォーカスのペインに表示する
    /// (`buffer.next` と同じ経路: Undo の結合の封印・検索バーを閉じる・`recent` の記録)。開いていない文書・タブに出して
    /// いない文書・アクティブなタブ (既に表示している) は何もしない。オーバーレイと検索バーの判定は呼び側が行う。
    pub(crate) fn click_tab(&mut self, doc: DocumentId) {
        if self.active_document() == Some(doc) || !self.tab_documents().contains(&doc) {
            return;
        }
        self.seal_active_history();
        self.activate_document(doc);
    }
}

/// タブラインのモデル (PANE-12 (c))。`names` は PANE-11 の名前 (`pane_bar::display_names`)。
pub(crate) fn tab_line_model(
    state: &EditorState,
    names: &BTreeMap<DocumentId, String>,
) -> TabLineModel {
    let active = state.active_document();
    let docs = state.tab_documents();
    let tabs: Vec<TabItem> = docs
        .iter()
        .filter_map(|&id| {
            let doc = state.document(id)?;
            let name = names
                .get(&id)
                .cloned()
                .unwrap_or_else(|| state.untitled_label(doc));
            Some(TabItem {
                doc: id,
                items: crate::ui_model::chrome::document_items(state, doc, &name),
            })
        })
        .collect();
    let active = active.and_then(|a| tabs.iter().position(|t| t.doc == a));
    TabLineModel { tabs, active }
}
