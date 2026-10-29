//! バッファ (複数文書) の操作: 切替と閉じる (docs/03 §9.8、07 §11.6 の `buffer.*`)。
//!
//! 文書の順は開いた順 (`DocumentId` の昇順)。閉じた後に表示する文書は、最近使った順で次の文書。

use std::ops::Range;

use eve_view::{CutPolicy, DocumentId, InputFieldModel, ListItem, OverlayModel};

use super::command::{
    CommandCtx, CommandKind, CommandRegistry, CommandResult, CommandSpec, Permission, RejectReason,
};
use super::file::Pending;
use super::layout::cell_text;
use super::overlay::{Continuation, Overlay};
use super::palette::{PaletteState, WINDOW, find, lower_graphemes};
use super::state::EditorState;

/// 一覧の行頭の変更マーカーの幅 (`[+] `)。照合した箇所の位置を合わせるのに使う。
const MARKER_WIDTH: usize = 4;

impl EditorState {
    /// 表示中の文書の、開いた順での番号 (1 始まり) と文書の数。文書が 1 件なら `None`
    /// (ステータスバーの `(2/3)`、03 §9.8)。置き換えの読込中の文書は数えない。
    pub(super) fn buffer_position(&self) -> Option<(usize, usize)> {
        let active = self.active_document()?;
        let ids: Vec<DocumentId> = self
            .documents
            .keys()
            .copied()
            .filter(|d| *d == active || !self.is_loading_replacement(*d))
            .collect();
        let pos = ids.iter().position(|d| *d == active)?;
        (ids.len() >= 2).then_some((pos + 1, ids.len()))
    }

    /// 置き換えで読込中の文書か (まだ表示されておらず、読込が終わるまで集合の外のものとして扱う)。
    fn is_loading_replacement(&self, doc: DocumentId) -> bool {
        self.pending.values().any(|p| {
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
            .recent
            .iter()
            .rev()
            .chain(self.documents.keys())
            .copied()
            .filter(|d| self.documents.contains_key(d) && !self.is_loading_replacement(*d));
        for d in others {
            if !out.contains(&d) {
                out.push(d);
            }
        }
        out
    }

    /// 文書の表示名 (ファイル名、無題は `status.untitled`)。
    fn buffer_name(&self, doc: DocumentId) -> String {
        self.documents
            .get(&doc)
            .and_then(|d| d.path.as_ref().and_then(|p| p.file_name()))
            .unwrap_or_else(|| {
                self.catalog()
                    .render("status.untitled", &[("n", "1")], &|_| None)
            })
    }

    /// 入力に合うバッファ (文書, 名前の中で照合した書記素の範囲)。空の入力は全件を並び順のまま返す。
    pub(super) fn buffer_hits(&self, query: &str) -> Vec<(DocumentId, Vec<Range<usize>>)> {
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
                let path = self.documents.get(&doc)?.path.as_ref()?.display();
                find(&lower_graphemes(&path), &q).map(|_| (doc, Vec::new()))
            })
            .collect()
    }

    /// `buffer.list`: バッファ一覧を開く。直前の文書を選んでおく (`Ctrl+E` `Enter` で直前と往復できる)。
    fn open_buffer_list(&mut self) -> CommandResult {
        if self.open_in_progress() {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
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
        let Some(Overlay::Palette(p)) = &self.overlay else {
            return None;
        };
        let hits = self.buffer_hits(&p.text);
        hits.get(p.selected.min(hits.len().saturating_sub(1)))
            .map(|(doc, _)| *doc)
    }

    /// `ui.confirm`: 選んだ文書に切り替えて一覧を閉じる。候補が無ければ閉じない。
    pub(super) fn buffer_list_confirm(&mut self) -> CommandResult {
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
    pub(super) fn buffer_list_model(&self, p: &PaletteState) -> OverlayModel {
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
                let doc = self.documents.get(doc_id)?;
                let marker = if doc.is_dirty() { "[+] " } else { "    " };
                let dir = doc
                    .path
                    .as_ref()
                    .and_then(|p| p.parent())
                    .map(|d| d.display())
                    .unwrap_or_default();
                let detail = format!("{dir}  {}", doc.format.encoding.name());
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
        let placeholder = p
            .text
            .is_empty()
            .then(|| cell_text(&render("ui.buffer_list.placeholder", &[]), CutPolicy::End));
        let footer = render(
            "ui.palette.footer",
            &[
                ("matched", &hits.len().to_string()),
                ("total", &self.buffer_order().len().to_string()),
            ],
        );
        OverlayModel::List {
            title: cell_text(&render("ui.buffer_list.title", &[]), CutPolicy::Never),
            input: InputFieldModel {
                len: cells.len(),
                cells,
                first: 0,
                cursor,
                selection: None,
                placeholder,
            },
            items,
            first,
            selected,
            footer: cell_text(&footer, CutPolicy::Never),
            no_match: cell_text(&render("ui.buffer_list.no_match", &[]), CutPolicy::End),
        }
    }

    /// 開いた順で `delta` 個先の文書へ切り替える (端で循環する)。文書が 1 件なら何もしない。
    fn switch_buffer(&mut self, delta: isize) -> CommandResult {
        // 置き換えの読込中は、読込中の文書がまだ集合に入っている。切り替えの対象にしない
        if self.open_in_progress() {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        let ids: Vec<DocumentId> = self.documents.keys().copied().collect();
        let Some(pos) = self
            .active_document()
            .and_then(|a| ids.iter().position(|d| *d == a))
        else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let next = (pos as isize + delta).rem_euclid(ids.len() as isize) as usize;
        self.activate_document(ids[next]);
        CommandResult::Done
    }

    /// 文書を閉じる要求。dirty なら FR-107 の確認を出す (保存を選んだら保存が成功したときだけ閉じる)。
    pub(super) fn request_close(&mut self, doc: DocumentId) -> CommandResult {
        if self.open_in_progress() {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        if self.confirm_doc_if_dirty(doc, "unsaved_close", Continuation::Close(doc)) {
            return CommandResult::Done;
        }
        self.close_buffer(doc)
    }

    /// 確認を済ませた (破棄または保存した) 後に文書を閉じる。表示中の文書なら、最近使った順で次の文書を表示する。
    /// 最後の 1 件なら無題の文書を新しく作る (終了はしない、03 §9.8)。
    pub(super) fn close_buffer(&mut self, doc: DocumentId) -> CommandResult {
        if self.active_document() == Some(doc) {
            let next = self
                .recent
                .iter()
                .rev()
                .copied()
                .find(|d| *d != doc && self.documents.contains_key(d))
                .or_else(|| self.documents.keys().copied().find(|d| *d != doc));
            match next {
                Some(n) => {
                    self.activate_document(n);
                    self.close_document(doc);
                }
                None => return self.new_untitled_forced(),
            }
        } else {
            self.close_document(doc);
        }
        self.recent.retain(|d| *d != doc);
        CommandResult::Done
    }
}

fn buffer_close(ctx: &mut CommandCtx<'_>) -> CommandResult {
    // バッファ一覧の中では、選んでいる文書を閉じる (07 §11.6 の `list` コンテキスト)
    if matches!(&ctx.state.overlay, Some(Overlay::Palette(p)) if p.buffers) {
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
pub(super) fn register(r: &mut CommandRegistry) {
    let spec = |id, perm, h| {
        CommandSpec::new(id, CommandKind::Ui, perm, "", h)
            .expect("カタログの when 句は構文が正しい")
    };
    r.register(spec("buffer.close", Permission::None, buffer_close));
    r.register(spec("buffer.list", Permission::Ui, buffer_list));
    r.register(spec("buffer.next", Permission::Ui, buffer_next));
    r.register(spec("buffer.prev", Permission::Ui, buffer_prev));
}
