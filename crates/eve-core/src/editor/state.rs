//! `EditorState`: 文書・View・Window の集合と、変更の唯一の経路 (docs/02 §5.1 / §5.2、docs/03 §6.5 の手順 7〜12)。

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::Arc;

use eve_text::{Bias, ByteOff, LineIdx, Selection, TextRead};
use eve_view::{
    Capabilities, DocumentId, EditorEvent, Effect, JobId, OriginKind, Revision, ViewId, WindowId,
};

use super::file::Pending;
use crate::document::{Applied, AppliedTx, ApplyError, Document, Transaction};
use crate::jobs::JobSpawner;
use crate::paths::Paths;
use crate::time::Tick;

use super::context::CtxValue;

/// 上下の余白の既定 (行)。
const SCROLL_MARGIN: usize = 3;

/// スクロール位置。`top` は先頭可視行の行頭 (03 §4.6)。折り返しと水平スクロールは TUI の版で足す。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScrollPos {
    pub top: ByteOff,
    /// 水平スクロール量 (表示列。折り返しなしのとき)。
    pub left_col: u32,
}

/// 文書の表示と Selection の単位。
#[derive(Clone, Debug)]
pub struct View {
    pub doc: DocumentId,
    pub selection: Selection,
    pub scroll: ScrollPos,
    /// 垂直移動の目標の表示列 (03 §5.2)。水平移動・編集・クリックでクリアし、垂直移動は維持する。
    pub goal_dcol: Option<u32>,
}

/// frontend が持つ編集面。v0.x は 1 つで、View を 1 つ持つ。
#[derive(Clone, Copy, Debug)]
pub struct Window {
    pub id: WindowId,
    pub root: ViewId,
}

/// 表現不能の文字の強調 (文書, 作った Revision, 文書内のバイト範囲)。
pub(super) type UnmappableMarks = (DocumentId, eve_view::Revision, Vec<(usize, usize)>);

pub struct EditorState {
    pub(crate) documents: BTreeMap<DocumentId, Document>,
    pub(crate) views: BTreeMap<ViewId, View>,
    pub(crate) windows: Vec<Window>,
    next_id: u64,
    /// Editor の「今」。Host が各メッセージの処理の前に `tick` で渡した値で、コマンドの実行中に読み直さない。
    pub(crate) now: Tick,
    pub(crate) caps: Capabilities,
    /// Window の編集面の大きさ (セル)。ページ移動などが使う。
    viewport: (u16, u16),
    effects: Vec<Effect>,
    events: Vec<EditorEvent>,
    snapshot_updates: BTreeSet<DocumentId>,
    needs_render: bool,
    /// ジョブの投入口 (`Ports.jobs`)。
    pub(super) jobs: Rc<dyn JobSpawner>,
    pub(super) paths: Arc<dyn Paths>,
    /// 未完了のジョブと、その結果の宛先。
    pub(super) pending: BTreeMap<JobId, Pending>,
    /// `{key:<command_id>}` の解決用のキー表記 (コマンド → 第一割当)。
    pub(super) key_labels: BTreeMap<String, String>,
    /// コマンド → 第一割当のストローク列 (ヒントバーの逆引き)。
    pub(super) key_seqs: BTreeMap<String, Vec<eve_view::KeyStroke>>,
    /// 出ているオーバーレイ (確認・入力プロンプト)。モーダル。
    pub(super) overlay: Option<super::overlay::Overlay>,
    /// 保存ジョブの完了後に続ける操作。
    pub(super) continuations: BTreeMap<JobId, super::overlay::Continuation>,
    /// 表示中のメッセージ (解決済み)。
    pub(super) message: Option<super::message::ShownMessage>,
    /// 未読のメッセージの数。
    pub(super) unread: u16,
    /// 履歴の破棄を通知した文書 (文書ごとに 1 回)。
    trim_notified: BTreeSet<DocumentId>,
    /// 内部レジスタ・ハンドルの表・貼り付けの token (クリップボード、`clipboard.rs`)。
    pub(super) clipboard: super::clipboard::ClipboardState,
    /// 解決済みの設定 (07 §5.0 の公開キー)。起動時に Host が `Editor::set_config` で渡す。
    pub(super) config: eve_config::Config,
    /// パレットの候補 (`hidden` を除く登録済みコマンド。CommandId 順)。
    pub(super) palette_entries: Vec<super::palette::PaletteEntry>,
    /// パレットで選ばれ、Editor が次に実行するコマンド。
    pub(super) deferred_command: Option<(String, super::command::Args)>,
    /// 文書内検索の状態 (検索バーが開いている間)。
    pub(super) search: Option<super::search_state::SearchState>,
    /// 検索バー (開いている間)。
    pub(super) search_bar: Option<super::search_bar::SearchBarState>,
    /// 表現不能の文字の強調 (文書, 作った Revision, 位置)。Revision が変わる (次の編集) まで残る。
    pub(super) unmappable_marks: Option<UnmappableMarks>,
    /// 編集で検索の結果が無効になった (Revision, 気づいた時刻)。入力が止まって 300 ms 後に再検索する (03 §12.2)。
    pub(super) research: Option<(eve_view::Revision, Tick)>,
    /// 単発の検索ジョブ (進行中の 1 つだけ)。
    pub(super) find_job: Option<eve_view::JobId>,
    /// ログファイルの場所 (設定の問題の要約に出す)。
    pub(super) log_path: Option<String>,
    /// OS のロケール由来の文字コードのヒント。
    pub(super) system_encoding: Option<crate::Encoding>,
    /// 検索バーが取る行数 (0 か 1)。編集面はその分だけ縮む。
    pub(super) bar_rows: u16,
    /// ホストが渡した編集面の大きさ (検索バーの分を引く前)。
    pub(super) surface: (u16, u16),
}

impl EditorState {
    pub fn new(caps: Capabilities, jobs: Rc<dyn JobSpawner>, paths: Arc<dyn Paths>) -> EditorState {
        EditorState {
            documents: BTreeMap::new(),
            views: BTreeMap::new(),
            windows: Vec::new(),
            next_id: 1,
            now: Tick(0),
            caps,
            viewport: (80, 24),
            effects: Vec::new(),
            events: Vec::new(),
            snapshot_updates: BTreeSet::new(),
            needs_render: true,
            jobs,
            paths,
            pending: BTreeMap::new(),
            key_labels: BTreeMap::new(),
            key_seqs: BTreeMap::new(),
            overlay: None,
            continuations: BTreeMap::new(),
            message: None,
            unread: 0,
            trim_notified: BTreeSet::new(),
            clipboard: Default::default(),
            config: eve_config::Config::default(),
            palette_entries: Vec::new(),
            deferred_command: None,
            search: None,
            search_bar: None,
            unmappable_marks: None,
            research: None,
            find_job: None,
            system_encoding: None,
            log_path: None,
            bar_rows: 0,
            surface: (80, 24),
        }
    }

    pub fn config(&self) -> &eve_config::Config {
        &self.config
    }

    /// 設定を差し替え、持っている文書の表示設定にも写す。
    pub fn set_config(&mut self, config: eve_config::Config) {
        self.config = config;
        let display = self.display_settings();
        for d in self.documents.values_mut() {
            d.display = display;
        }
        self.needs_render = true;
    }

    fn display_settings(&self) -> crate::document::DisplaySettings {
        crate::document::DisplaySettings {
            tab_width: self.config.tab_width,
            ambiguous_wide: self.config.ambiguous_width == 2,
        }
    }

    /// ログファイルの場所を設定する (設定の問題の要約 `msg.config.problems` の `{log}`)。
    pub fn set_log_path(&mut self, path: Option<String>) {
        self.log_path = path;
    }

    /// OS のロケール由来の文字コードのヒントを設定する (`file.default_encoding = "system"` が使う、05 §6)。
    pub fn set_system_encoding(&mut self, encoding: Option<crate::Encoding>) {
        self.system_encoding = encoding;
    }

    /// `file.default_line_ending` の実際の改行 (`native` は OS に従う)。
    pub(super) fn config_eol(&self) -> eve_text::LineEnding {
        use eve_config::DefaultLineEnding;
        match self.config.default_line_ending {
            DefaultLineEnding::Lf => eve_text::LineEnding::Lf,
            DefaultLineEnding::Crlf => eve_text::LineEnding::Crlf,
            DefaultLineEnding::Cr => eve_text::LineEnding::Cr,
            DefaultLineEnding::Native if cfg!(windows) => eve_text::LineEnding::Crlf,
            DefaultLineEnding::Native => eve_text::LineEnding::Lf,
        }
    }

    /// 新規文書 (無題・存在しないパス)。文字コードと新しく挿入する改行は `file.default_*` (07 §5.2)。
    pub(super) fn blank_document(&self) -> Document {
        use eve_config::DefaultEncoding;
        let mut doc = Document::new_untitled();
        let (encoding, has_bom) = match self.config.default_encoding {
            DefaultEncoding::Utf8 => (crate::Encoding::Utf8, false),
            // `system` は OS ロケール由来のヒント (05 §6)。ヒントが無ければ UTF-8
            DefaultEncoding::System => {
                (self.system_encoding.unwrap_or(crate::Encoding::Utf8), false)
            }
            DefaultEncoding::Utf8Bom => (crate::Encoding::Utf8, true),
            DefaultEncoding::Utf16Le => (crate::Encoding::Utf16Le, true),
            DefaultEncoding::Utf16Be => (crate::Encoding::Utf16Be, true),
            DefaultEncoding::ShiftJis => (crate::Encoding::ShiftJis, false),
            DefaultEncoding::EucJp => (crate::Encoding::EucJp, false),
            DefaultEncoding::Iso2022Jp => (crate::Encoding::Iso2022Jp, false),
        };
        doc.format.encoding = encoding;
        doc.format.has_bom = has_bom;
        doc.format.default_eol = self.config_eol();
        doc.display = self.display_settings();
        doc
    }

    /// 読込の完了などで、既に採番した文書の中身を差し替える。表示設定を写す。
    pub(super) fn put_document(&mut self, id: DocumentId, mut doc: Document) {
        doc.display = self.display_settings();
        self.documents.insert(id, doc);
    }

    /// 退避用の公開物が変わった文書として記録する (03 §6.5 手順 12。保存の完了・読込の完了)。
    pub(super) fn mark_snapshot_update(&mut self, doc: DocumentId) {
        self.snapshot_updates.insert(doc);
    }

    fn fresh(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn viewport(&self) -> (u16, u16) {
        self.viewport
    }

    pub fn set_viewport(&mut self, cols: u16, rows: u16) {
        self.surface = (cols, rows);
        self.apply_viewport(cols, rows);
    }

    /// 編集面の大きさを、検索バーの分を引いて反映する。
    pub(super) fn apply_viewport(&mut self, cols: u16, rows: u16) {
        let rows = rows.saturating_sub(self.bar_rows).max(1);
        if self.viewport != (cols, rows) {
            self.viewport = (cols, rows);
            self.needs_render = true;
            // 画面の大きさが変わっても、カーソルが見えるようにする
            let views: Vec<ViewId> = self.views.keys().copied().collect();
            for v in views {
                self.ensure_cursor_visible(v);
            }
        }
    }

    pub fn set_caps(&mut self, caps: Capabilities) {
        self.caps = caps;
        self.needs_render = true;
    }

    pub fn now(&self) -> Tick {
        self.now
    }

    // ---------- 文書・View・Window ----------

    /// 文書を追加し、それを表示する View を作る (カーソルは先頭)。文書の ID は閉じた後も再利用しない。
    pub fn add_document(&mut self, mut doc: Document) -> (DocumentId, ViewId) {
        doc.display = self.display_settings();
        let doc_id = DocumentId(self.fresh());
        self.documents.insert(doc_id, doc);
        let view_id = self.add_view(doc_id);
        self.events
            .push(EditorEvent::DocumentOpened { doc: doc_id });
        self.snapshot_updates.insert(doc_id);
        self.needs_render = true;
        (doc_id, view_id)
    }

    /// 新しく挿入する改行だけを変える (本文を変えないメタデータの変更。Undo の対象外・dirty にしない、03 §8.4)。
    pub fn set_default_eol(&mut self, doc: DocumentId, eol: eve_text::LineEnding) {
        if let Some(d) = self.documents.get_mut(&doc) {
            d.format.default_eol = eol;
            self.needs_render = true;
        }
    }

    pub fn add_view(&mut self, doc: DocumentId) -> ViewId {
        let id = ViewId(self.fresh());
        self.views.insert(
            id,
            View {
                doc,
                selection: Selection::point(ByteOff(0)),
                scroll: ScrollPos::default(),
                goal_dcol: None,
            },
        );
        id
    }

    pub fn add_window(&mut self, root: ViewId) -> WindowId {
        let id = WindowId(self.fresh());
        self.windows.push(Window { id, root });
        id
    }

    /// 開いている文書の数。
    pub fn documents_len(&self) -> usize {
        self.documents.len()
    }

    pub fn document(&self, id: DocumentId) -> Option<&Document> {
        self.documents.get(&id)
    }

    /// 文書の形式・読み取り専用の理由・パスなどを更新するための可変参照 (テキストは `apply` でしか変えられない)。
    pub fn document_mut(&mut self, id: DocumentId) -> Option<&mut Document> {
        self.documents.get_mut(&id)
    }

    /// 先頭可視行を設定する (行頭へ丸める)。
    pub fn set_scroll_top(&mut self, view: ViewId, pos: ByteOff) {
        let Some(v) = self.views.get_mut(&view) else {
            return;
        };
        let Some(d) = self.documents.get(&v.doc) else {
            return;
        };
        let text = d.text();
        v.scroll.top = text.line_to_byte(text.byte_to_line(pos));
        self.needs_render = true;
    }

    pub fn view(&self, id: ViewId) -> Option<&View> {
        self.views.get(&id)
    }

    pub fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows.iter().find(|w| w.id == id)
    }

    /// Window が表示している View。
    pub fn window_view(&self, id: WindowId) -> Option<ViewId> {
        self.window(id).map(|w| w.root)
    }

    /// 最初の Window (v0.x は 1 つ)。
    pub fn first_window(&self) -> Option<WindowId> {
        self.windows.first().map(|w| w.id)
    }

    // ---------- 出力 ----------

    pub fn push_effect(&mut self, e: Effect) {
        self.effects.push(e);
    }

    pub fn drain_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    pub fn push_event(&mut self, e: EditorEvent) {
        self.events.push(e);
    }

    pub fn drain_events(&mut self) -> Vec<EditorEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn take_snapshot_updates(&mut self) -> Vec<DocumentId> {
        std::mem::take(&mut self.snapshot_updates)
            .into_iter()
            .collect()
    }

    pub fn needs_render(&self) -> bool {
        self.needs_render
    }

    /// Frame を生成した後に呼ぶ。
    pub fn mark_rendered(&mut self) {
        self.needs_render = false;
    }

    pub fn mark_dirty_render(&mut self) {
        self.needs_render = true;
    }

    // ---------- 変更の唯一の経路 ----------

    /// Transaction を適用し、全 View の Selection・スクロールを写像し、通知を積む (03 §6.5 の手順 7・10・12)。
    /// 発行 View は `tx.view`。
    pub fn apply_transaction(
        &mut self,
        doc: DocumentId,
        tx: Transaction,
    ) -> Result<Revision, ApplyError> {
        let document = self.documents.get_mut(&doc).ok_or(ApplyError::Rejected)?;
        match document.apply(&tx)? {
            AppliedTx::Identity { selection } => {
                // 変更なし: 発行 View の Selection だけを更新する (goal_dcol は編集と同じく解除)
                if let (Some(view), Some(sel)) = (tx.view, selection) {
                    self.set_selection(view, sel);
                }
                Ok(self.documents[&doc].revision())
            }
            AppliedTx::Changed(applied) => {
                let rev = applied.revision;
                self.after_change(doc, &applied, tx.view);
                Ok(rev)
            }
        }
    }

    /// Undo。取り消すものがなければ `Ok(false)`。発行 View の Selection は `selection_before` に戻す。
    pub fn undo(&mut self, doc: DocumentId, view: Option<ViewId>) -> Result<bool, ApplyError> {
        let document = self.documents.get_mut(&doc).ok_or(ApplyError::Rejected)?;
        let applied = document.undo()?;
        Ok(applied.map(|a| self.after_change(doc, &a, view)).is_some())
    }

    pub fn redo(&mut self, doc: DocumentId, view: Option<ViewId>) -> Result<bool, ApplyError> {
        let document = self.documents.get_mut(&doc).ok_or(ApplyError::Rejected)?;
        let applied = document.redo()?;
        Ok(applied.map(|a| self.after_change(doc, &a, view)).is_some())
    }

    fn after_change(&mut self, doc: DocumentId, a: &Applied, issuer: Option<ViewId>) {
        // 履歴の予算による破棄は、文書ごとに 1 回だけ通知する (03 §7.5)
        if a.record.as_ref().is_some_and(|r| r.pruned) && self.trim_notified.insert(doc) {
            self.show_message(
                eve_view::MessageLevel::Info,
                "info.undo.history_trimmed",
                &[],
            );
        }
        let Some(document) = self.documents.get(&doc) else {
            return;
        };
        let text = document.text();
        for (id, view) in self.views.iter_mut().filter(|(_, v)| v.doc == doc) {
            if Some(*id) == issuer {
                // 発行 View: 指定された Selection (なければ自分の編集として写像)。goal_dcol はクリア
                let sel = match &a.selection {
                    Some(s) => s.clone().normalized(text),
                    None => view
                        .selection
                        .map(&a.changes, eve_text::MapPolicy::Own)
                        .normalized(text),
                };
                view.selection = sel;
                view.goal_dcol = None;
            } else {
                // 他の View: 他者の編集として写像。goal_dcol は維持
                view.selection = view
                    .selection
                    .map(&a.changes, eve_text::MapPolicy::Other)
                    .normalized(text);
            }
            // スクロール位置は同じ関数で写像し、行頭へ丸める
            let top = a.changes.map_pos(view.scroll.top, Bias::Before);
            view.scroll.top = text.line_to_byte(LineIdx(text.byte_to_line(top).0));
        }
        // 編集の後もカーソルが見えるようにする (行が短くなった・画面の右端を越えた)
        if let Some(v) = issuer {
            self.ensure_cursor_visible(v);
        }
        self.events.push(EditorEvent::DocumentChanged {
            doc,
            revision: a.revision,
            origin_kind: OriginKind {
                origin: a.origin.tag(),
                is_undo: a.is_undo,
            },
        });
        self.snapshot_updates.insert(doc);
        self.needs_render = true;
    }

    /// Window の View が表示している文書の Undo の結合を封印する (カーソル移動の実行時)。
    pub fn seal_active_history(&mut self) {
        let doc = self
            .windows
            .first()
            .and_then(|w| self.views.get(&w.root))
            .map(|v| v.doc);
        if let Some(d) = doc.and_then(|d| self.documents.get_mut(&d)) {
            d.seal_history();
        }
    }

    /// `set_selection` と同じだが、目標列 (`goal_dcol`) を指定する (垂直移動・ページ移動が維持する)。
    pub fn set_selection_with_goal(&mut self, view: ViewId, sel: Selection, goal: Option<u32>) {
        self.set_selection(view, sel);
        if let Some(v) = self.views.get_mut(&view) {
            v.goal_dcol = goal;
        }
    }

    /// 先頭可視行の上限 (最後の行が画面の最下行に来る位置)。これより下へは動かさない。
    fn max_top_line(&self, doc: DocumentId) -> usize {
        let rows = self.viewport.1 as usize;
        self.documents
            .get(&doc)
            .map_or(0, |d| d.text().len_lines().saturating_sub(rows.max(1)))
    }

    /// 先頭可視行を `delta` 行ずらす (文書の範囲内に収め、短い文書を画面の外へ押し出さない)。
    pub fn scroll_lines(&mut self, view: ViewId, delta: i64) {
        let Some(doc) = self.views.get(&view).map(|v| v.doc) else {
            return;
        };
        let max_top = self.max_top_line(doc) as i64;
        let Some(v) = self.views.get_mut(&view) else {
            return;
        };
        let Some(d) = self.documents.get(&doc) else {
            return;
        };
        let t = d.text();
        let top = t.byte_to_line(v.scroll.top).0 as i64;
        let new = top.saturating_add(delta).clamp(0, max_top.max(0)) as usize;
        v.scroll.top = t.line_to_byte(LineIdx(new));
        self.needs_render = true;
    }

    /// 上下の余白 (行)。`view.scroll_margin` の既定 3 (04 §10。v0.1 は定数)。小さな画面では半分以下にする。
    fn scroll_margin(&self) -> usize {
        SCROLL_MARGIN.min((self.viewport.1 as usize).saturating_sub(1) / 2)
    }

    /// カーソル (primary の head) が余白を除いた可視範囲に入るよう、先頭可視行を最小限に動かす。
    pub fn ensure_cursor_visible(&mut self, view: ViewId) {
        let margin = self.scroll_margin();
        self.ensure_cursor_visible_with_margin(view, margin);
    }

    pub fn ensure_cursor_visible_with_margin(&mut self, view: ViewId, margin: usize) {
        self.ensure_vertical(view, margin);
        self.ensure_horizontal(view);
    }

    /// カーソルの列が見えるよう水平スクロールを調整する (折り返しなし)。
    fn ensure_horizontal(&mut self, view: ViewId) {
        let Some((col, cell_w)) = super::layout::primary_cursor_cell(self, view) else {
            return;
        };
        let Some(doc) = self
            .views
            .get(&view)
            .and_then(|v| self.documents.get(&v.doc))
        else {
            return;
        };
        let text_cols = (self.viewport.0 as usize)
            .saturating_sub(super::layout::gutter_width(doc, self.viewport.0 as usize) as usize);
        let Some(v) = self.views.get_mut(&view) else {
            return;
        };
        let left = v.scroll.left_col as usize;
        let new_left = if col + cell_w <= text_cols {
            0 // 行頭からカーソルまでが画面に収まるなら、スクロールしない
        } else if col < left {
            col
        } else if text_cols > 0 && col + cell_w > left + text_cols {
            // 幅のある単位 (全角・記号) の全体が見えるようにする。画面より広ければ先頭のセルを優先する
            (col + cell_w - text_cols).min(col)
        } else {
            left
        };
        if new_left != left {
            v.scroll.left_col = u32::try_from(new_left).unwrap_or(u32::MAX);
            self.needs_render = true;
        }
    }

    fn ensure_vertical(&mut self, view: ViewId, margin: usize) {
        let rows = self.viewport.1 as usize;
        let Some(doc) = self.views.get(&view).map(|v| v.doc) else {
            return;
        };
        let max_top = self.max_top_line(doc);
        let Some(v) = self.views.get_mut(&view) else {
            return;
        };
        let Some(d) = self.documents.get(&doc) else {
            return;
        };
        let t = d.text();
        let line = t.byte_to_line(v.selection.primary().head).0;
        let top = t.byte_to_line(v.scroll.top).0;
        let new_top = if line < top + margin {
            line.saturating_sub(margin)
        } else if rows > 0 && line + margin >= top + rows {
            (line + margin + 1)
                .saturating_sub(rows)
                .min(max_top.max(top))
        } else {
            top
        };
        if new_top != top {
            v.scroll.top = t.line_to_byte(LineIdx(new_top));
            self.needs_render = true;
        }
    }

    /// カーソルを含む行をビューポートの中央に置く (指定行へ移動の後、03 §5.6)。
    pub fn center_cursor(&mut self, view: ViewId) {
        let rows = self.viewport.1 as usize;
        let Some(doc) = self.views.get(&view).map(|v| v.doc) else {
            return;
        };
        let max_top = self.max_top_line(doc);
        let Some(v) = self.views.get_mut(&view) else {
            return;
        };
        let Some(d) = self.documents.get(&doc) else {
            return;
        };
        let t = d.text();
        let line = t.byte_to_line(v.selection.primary().head).0;
        let new_top = line.saturating_sub(rows / 2).min(max_top);
        v.scroll.top = t.line_to_byte(LineIdx(new_top));
        self.needs_render = true;
    }

    /// View の Selection を設定する (書記素境界へ正規化し、`goal_dcol` はクリア)。
    pub fn set_selection(&mut self, view: ViewId, sel: Selection) {
        let Some(v) = self.views.get_mut(&view) else {
            return;
        };
        let Some(d) = self.documents.get(&v.doc) else {
            return;
        };
        v.selection = sel.normalized(d.text());
        v.goal_dcol = None;
        self.needs_render = true;
    }

    // ---------- 条件式の文脈 ----------

    /// `when` 句で参照できる識別子の値 (07 §7.1 / §11.6)。知らない識別子は `None` (偽)。
    pub fn context_value(&self, name: &str) -> Option<CtxValue> {
        let view = self.windows.first().and_then(|w| self.views.get(&w.root));
        let doc = view.and_then(|v| self.documents.get(&v.doc));
        let b = |x: bool| Some(CtxValue::Bool(x));
        match name {
            "doc.read_only" => {
                doc.map_or(b(false), |d| b(!d.is_editable() || self.open_replacing()))
            }
            "doc.has_path" => b(doc.is_some_and(|d| d.path.is_some())),
            "doc.dirty" => b(doc.is_some_and(Document::is_dirty)),
            "selection.empty" => b(view.is_none_or(|v| v.selection.iter().all(|r| r.is_empty()))),
            "overlay.active" => Some(CtxValue::Str(self.overlay_name().to_owned())),
            "search.active" => b(self.search_bar.is_some()),
            "search.replace_active" => b(self
                .search_bar
                .as_ref()
                .is_some_and(|s| s.replace.is_some())),
            "caps.can_suspend" => b(self.caps.can_suspend),
            "editor" => b(true),
            _ => None,
        }
    }
}
