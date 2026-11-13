//! バッファ (複数文書) の操作: 切替と閉じる (docs/03 §9.8、07 §11.6 の `buffer.*`)。
//!
//! 文書の順は開いた順 (`DocumentId` の昇順)。閉じた後に表示する文書は、最近使った順で次の文書。

use std::ops::Range;

use eve_view::{CutPolicy, DocumentId, InputFieldModel, ListItem, OverlayModel, ViewId};

use super::state::EditorState;
use crate::command::ids;
use crate::command::{
    CommandCtx, CommandKind, CommandRegistry, CommandResult, Permission, RejectReason,
};
use crate::io::pending::Pending;
use crate::layout::cell_text;
use crate::ui_model::keys;
use crate::ui_model::overlay::{Continuation, Overlay};
use crate::ui_model::palette::{PaletteState, WINDOW, find, lower_graphemes};

/// 一覧の行頭の変更マーカーの幅 (`[+] `)。照合した箇所の位置を合わせるのに使う。
const MARKER_WIDTH: usize = 4;

impl EditorState {
    /// 表示中の文書の、開いた順での番号 (1 始まり) と文書の数。文書が 1 件なら `None`
    /// (ステータスバーの `(2/3)`、03 §9.8)。置き換えの読込中の文書は数えない。
    pub(crate) fn buffer_position(&self) -> Option<(usize, usize)> {
        let active = self.active_document()?;
        let ids: Vec<DocumentId> = self
            .docs
            .documents
            .keys()
            .copied()
            .filter(|d| *d == active || !self.is_loading_replacement(*d))
            .collect();
        let pos = ids.iter().position(|d| *d == active)?;
        (ids.len() >= 2).then_some((pos + 1, ids.len()))
    }

    /// 置き換えで読込中の文書か (まだ表示されておらず、読込が終わるまで集合の外のものとして扱う)。先行表示 (`Partial`)
    /// の文書は先頭を表示できるので、一覧に載せる (03 §9.8 の「読込中のバッファ」)。
    fn is_loading_replacement(&self, doc: DocumentId) -> bool {
        let loading = self
            .docs
            .documents
            .get(&doc)
            .is_some_and(|d| d.load_state == crate::document::LoadState::Loading);
        loading
            && self.io.pending.values().any(|p| {
                matches!(
                    p,
                    Pending::Open { doc: d, replace: Some(_), .. } if *d == doc
                )
            })
    }

    /// バッファ一覧の並び: 最近使った順 (表示中の文書が先頭、直前の文書が 2 番目、04 §8.5)。
    /// 一度も表示していない文書は開いた順で最後に並べる。置き換えで読込中の文書は載せない。
    fn buffer_order(&self) -> Vec<DocumentId> {
        let mut out: Vec<DocumentId> = self.active_document().into_iter().collect();
        let others = self
            .docs
            .recent
            .iter()
            .rev()
            .chain(self.docs.documents.keys())
            .copied()
            .filter(|d| self.docs.documents.contains_key(d) && !self.is_loading_replacement(*d));
        for d in others {
            if !out.contains(&d) {
                out.push(d);
            }
        }
        out
    }

    /// 文書の表示名 (ファイル名、無題は `status.untitled`)。
    fn buffer_name(&self, doc: DocumentId) -> String {
        self.docs
            .documents
            .get(&doc)
            .map(|d| {
                d.path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .unwrap_or_else(|| self.untitled_label(d))
            })
            .unwrap_or_default()
    }

    /// 入力に合うバッファ (文書, 名前の中で照合した書記素の範囲)。空の入力は全件を並び順のまま返す。
    pub(crate) fn buffer_hits(&self, query: &str) -> Vec<(DocumentId, Vec<Range<usize>>)> {
        let q = lower_graphemes(query.trim());
        self.buffer_order()
            .into_iter()
            .filter_map(|doc| {
                if q.is_empty() {
                    return Some((doc, Vec::new()));
                }
                let name = lower_graphemes(&self.buffer_name(doc));
                if let Some(s) = find(&name, &q) {
                    return Some((doc, std::iter::once(s..s + q.len()).collect()));
                }
                // ディレクトリでも探せる (照合した箇所は名前の外なので印は付けない)
                let path = self.docs.documents.get(&doc)?.path.as_ref()?.display();
                find(&lower_graphemes(&path), &q).map(|_| (doc, Vec::new()))
            })
            .collect()
    }

    /// `buffer.list`: バッファ一覧を開く。直前の文書を選んでおく (`Ctrl+E` `Enter` で直前と往復できる)。
    fn open_buffer_list(&mut self) -> CommandResult {
        let selected = usize::from(self.buffer_order().len() >= 2);
        self.set_overlay(Some(Overlay::Palette(PaletteState {
            buffers: true,
            selected,
            ..PaletteState::default()
        })));
        self.mark_dirty_render();
        CommandResult::Done
    }

    /// 一覧で選んでいる文書。
    fn buffer_list_selected(&self) -> Option<DocumentId> {
        let Some(Overlay::Palette(p)) = &self.ui.overlay else {
            return None;
        };
        let hits = self.buffer_hits(&p.text);
        hits.get(p.selected.min(hits.len().saturating_sub(1)))
            .map(|(doc, _)| *doc)
    }

    /// `ui.confirm`: 選んだ文書に切り替えて一覧を閉じる。候補が無ければ閉じない。
    pub(crate) fn buffer_list_confirm(&mut self) -> CommandResult {
        let Some(doc) = self.buffer_list_selected() else {
            return CommandResult::Done;
        };
        self.set_overlay(None);
        self.activate_document(doc);
        self.mark_dirty_render();
        CommandResult::Done
    }

    /// 一覧の中の `buffer.close`: 選んだ文書を閉じる。一覧は開いたままにする (dirty なら確認に替わる)。
    fn buffer_list_close_selected(&mut self) -> CommandResult {
        match self.buffer_list_selected() {
            Some(doc) => self.request_close(doc),
            None => CommandResult::Done,
        }
    }

    /// 一覧の表示 (パレットと同じ `OverlayModel::List`)。行: 変更マーカー・名前、右に ディレクトリ と文字コード。
    pub(crate) fn buffer_list_model(&self, p: &PaletteState) -> OverlayModel {
        let render = |key: &str, args: &[(&str, &str)]| {
            self.catalog().render(key, args, &|c| self.key_label(c))
        };
        let hits = self.buffer_hits(&p.text);
        let selected = p.selected.min(hits.len().saturating_sub(1));
        let first = if selected < WINDOW {
            0
        } else {
            selected + 1 - WINDOW
        };
        let items: Vec<ListItem> = hits
            .iter()
            .skip(first)
            .take(WINDOW)
            .filter_map(|(doc_id, ranges)| {
                let doc = self.docs.documents.get(doc_id)?;
                let marker = if doc.is_dirty() { "[+] " } else { "    " };
                let dir = doc
                    .path
                    .as_ref()
                    .and_then(|p| p.parent())
                    .map(|d| d.display())
                    .unwrap_or_default();
                // 読込中 (`Loading` / 先行表示の `Partial`) の文書は、文字コードの代わりに「読込中」と出す (03 §9.8 の
                // 「読込中のバッファ」、04 §8.5。文字コードは読込の完了で決まる)
                let kind = if matches!(
                    doc.load_state,
                    crate::document::LoadState::Loading | crate::document::LoadState::Partial
                ) {
                    render(keys::UI_LOAD_STATE_LOADING, &[])
                } else {
                    doc.format.encoding.name().to_owned()
                };
                let detail = format!("{dir}  {kind}");
                Some(ListItem {
                    label: cell_text(
                        &format!("{marker}{}", self.buffer_name(*doc_id)),
                        CutPolicy::End,
                    ),
                    detail: Some(cell_text(detail.trim_start(), CutPolicy::Middle)),
                    match_ranges: ranges
                        .iter()
                        .map(|r| r.start + MARKER_WIDTH..r.end + MARKER_WIDTH)
                        .collect(),
                    keys: Vec::new(),
                    disabled_reason: None,
                })
            })
            .collect();
        let cells = cell_text(&p.text, CutPolicy::End).spans;
        let cursor = cell_text(&p.text[..p.cursor], CutPolicy::End).spans.len();
        let selection = crate::ui_model::overlay::field_selection(p.all_selected, cells.len());
        let placeholder = p.text.is_empty().then(|| {
            cell_text(
                &render(keys::UI_BUFFER_LIST_PLACEHOLDER, &[]),
                CutPolicy::End,
            )
        });
        let footer = render(
            keys::UI_PALETTE_FOOTER,
            &[
                ("matched", &hits.len().to_string()),
                ("total", &self.buffer_order().len().to_string()),
            ],
        );
        OverlayModel::List {
            title: cell_text(&render(keys::UI_BUFFER_LIST_TITLE, &[]), CutPolicy::Never),
            input: InputFieldModel {
                len: cells.len(),
                cells,
                first: 0,
                cursor,
                selection,
                placeholder,
            },
            items,
            first,
            selected,
            footer: cell_text(&footer, CutPolicy::Never),
            no_match: cell_text(&render(keys::UI_BUFFER_LIST_NO_MATCH, &[]), CutPolicy::End),
        }
    }

    /// 開いた順で `delta` 個先の文書へ切り替える (端で循環する)。文書が 1 件なら何もしない。
    fn switch_buffer(&mut self, delta: isize) -> CommandResult {
        let ids: Vec<DocumentId> = self.docs.documents.keys().copied().collect();
        let Some(pos) = self
            .active_document()
            .and_then(|a| ids.iter().position(|d| *d == a))
        else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        // 符号付きの変換をせずに、`(pos + delta) mod n` を求める
        let n = ids.len();
        let step = delta.unsigned_abs() % n;
        let next = if delta >= 0 {
            (pos + step) % n
        } else {
            (pos + n - step) % n
        };
        self.activate_document(ids[next]);
        CommandResult::Done
    }

    /// 文書を閉じる要求。dirty なら FR-107 の確認を出す (保存を選んだら保存が成功したときだけ閉じる)。
    pub(crate) fn request_close(&mut self, doc: DocumentId) -> CommandResult {
        if self.confirm_doc_if_dirty(doc, "unsaved_close", Continuation::Close(doc)) {
            return CommandResult::Done;
        }
        self.close_buffer(doc)
    }

    /// 確認を済ませた (破棄または保存した) 後に文書を閉じる。表示中の文書なら、最近使った順で次の文書を表示する。
    /// 最後の 1 件なら無題の文書を新しく作る (終了はしない、03 §9.8)。
    pub(crate) fn close_buffer(&mut self, doc: DocumentId) -> CommandResult {
        if self.active_document() == Some(doc) {
            let next = self
                .docs
                .recent
                .iter()
                .rev()
                .copied()
                .find(|d| *d != doc && self.docs.documents.contains_key(d))
                .or_else(|| self.docs.documents.keys().copied().find(|d| *d != doc));
            match next {
                Some(n) => {
                    self.activate_document(n);
                    self.close_document(doc);
                }
                None => {
                    // 最後の 1 件を閉じた: 無題の文書を新しく作る (終了はしない)
                    let blank = self.blank_document();
                    let (_, view) = self.add_document(blank);
                    self.show_view(view, Some(doc));
                }
            }
        } else {
            self.close_document(doc);
        }
        self.docs.recent.retain(|d| *d != doc);
        CommandResult::Done
    }
}

fn buffer_close(ctx: &mut CommandCtx<'_>) -> CommandResult {
    // バッファ一覧の中では、選んでいる文書を閉じる (07 §11.6 の `list` コンテキスト)
    if matches!(&ctx.state.ui.overlay, Some(Overlay::Palette(p)) if p.buffers) {
        return ctx.state.buffer_list_close_selected();
    }
    match ctx.state.active_document() {
        Some(doc) => ctx.state.request_close(doc),
        None => CommandResult::Rejected(RejectReason::NoTarget),
    }
}

fn buffer_list(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.open_buffer_list()
}

fn buffer_next(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.switch_buffer(1)
}

fn buffer_prev(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.switch_buffer(-1)
}

/// v0.2 のバッファコマンド (07 §11.6 の `buffer.close` / `buffer.next` / `buffer.prev`)。
pub(crate) fn register(r: &mut CommandRegistry) {
    r.add(
        ids::BUFFER_CLOSE,
        CommandKind::Ui,
        Permission::None,
        "",
        buffer_close,
    );
    r.add(
        ids::BUFFER_LIST,
        CommandKind::Ui,
        Permission::Ui,
        "",
        buffer_list,
    );
    r.add(
        ids::BUFFER_NEXT,
        CommandKind::Ui,
        Permission::Ui,
        "",
        buffer_next,
    );
    r.add(
        ids::BUFFER_PREV,
        CommandKind::Ui,
        Permission::Ui,
        "",
        buffer_prev,
    );
}

// ---- (v0.3 の M14、STR-B1 で io/ から移した) ----
impl EditorState {
    /// `doc` を表示する (最初の Window の View をその文書のものにする)。文書の View が無ければ作る。
    /// 検索バーは文書ごとには持たないので、切り替える前に閉じる (03 §9.8)。
    pub(crate) fn activate_document(&mut self, doc: DocumentId) {
        if self.active_document() == Some(doc) || !self.docs.documents.contains_key(&doc) {
            return;
        }
        self.search_close();
        if let Some(prev) = self.active_document() {
            self.docs.recent.retain(|d| *d != prev);
            self.docs.recent.push(prev);
        }
        let view = match self.docs.views.iter().find(|(_, v)| v.doc == doc) {
            Some((id, _)) => *id,
            None => self.add_view(doc),
        };
        if let Some(w) = self.docs.windows.first_mut() {
            w.root = view;
        }
        self.mark_dirty_render();
    }

    /// Window の表示を `view` に切り替え、`old` の文書を閉じる。Window がなければ作る。
    pub(crate) fn show_view(&mut self, view: ViewId, old: Option<DocumentId>) {
        match self.docs.windows.first_mut() {
            Some(w) => w.root = view,
            None => {
                self.add_window(view);
            }
        }
        if let Some(old) = old {
            self.close_document(old);
        }
        self.mark_dirty_render();
    }
}
