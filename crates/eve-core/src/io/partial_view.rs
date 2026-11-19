//! 先行表示 (Partial) の core の側 (docs/03 §9.2.1、v0.4 の M25 の (2)、01 FR-122 AC-2〜AC-4): 先頭の受け取りと
//! 差し替え、表示の切り替えとオーバーレイの間の保留、新しく開く読込の間に使えない操作の拒否、先頭の末尾の知らせ。
//!
//! 先行表示の文書は `LoadState::Partial` で、`Loading` と同じく編集・保存・スワップ・緊急退避の対象にならない (どの入口も
//! `load_state == Ready` か `is_editable` で判定しているので、`Partial` を足すだけで弾かれる)。本文の差し替え (先頭・
//! 2 回目の先頭・全文) は `put_document` を通り、Revision は前の文書の続きになる (SAF-06)。

use eve_text::{ByteOff, Selection, Text, TextRead};
use eve_view::{DocumentId, JobId, MessageLevel, ViewId};

use crate::command::ids;
use crate::document::{Document, LoadState, PreservedMap, TextFormat};
use crate::editor::state::{EditorState, ScrollPos};
use crate::ui_model::keys;

use super::open::{OpenKind, human_size};
use super::pending::Pending;

/// 新しく開く読込の間 (本文の無い `Loading` と先行表示の `Partial`) は、`msg.open.partial_unavailable` で拒否する
/// コマンド (03 §9.2.1 の「できない操作」)。先頭の中だけの結果が文書全体の結果に見えるため。
const PARTIAL_UNAVAILABLE: [&str; 9] = [
    ids::SEARCH_OPEN,
    ids::SEARCH_NEXT,
    ids::SEARCH_PREV,
    ids::SEARCH_REPLACE_OPEN,
    ids::SEARCH_REPLACE,
    ids::SEARCH_REPLACE_ALL,
    ids::CURSOR_GOTO_LINE,
    ids::SELECT_ALL,
    ids::SELECT_EXTEND_TO_DOC_END,
];

/// 先頭の末尾より先へ動こうとしたかを見る前進の移動 (03 §9.2.1 の「できる操作」の `info.open.partial_end`)。
const FORWARD_MOTIONS: [&str; 11] = [
    ids::CURSOR_MOVE_DOWN,
    ids::CURSOR_MOVE_RIGHT,
    ids::CURSOR_MOVE_WORD_RIGHT,
    ids::CURSOR_PAGE_DOWN,
    ids::CURSOR_DOC_END,
    ids::CURSOR_END,
    ids::SELECT_EXTEND_DOWN,
    ids::SELECT_EXTEND_RIGHT,
    ids::SELECT_EXTEND_WORD_RIGHT,
    ids::SELECT_EXTEND_PAGE_DOWN,
    ids::SELECT_EXTEND_END,
];

/// `file.open` の読込の表示の切り替え (03 §9.2.1 の「表示の切り替え」、§9.8 の `file.open` の行)。切り替え元
/// (`Pending::Open.replace`) のある読込だけが持つ。先行表示か完了を受けた時点で作り、切り替えた・条件を満たさなかった
/// ・読込が終わった (失敗・取消し・閉じる) ときに消す。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OpenSwitch {
    /// 切り替え元 (開いたときに表示していた文書)。
    pub from: DocumentId,
    /// 読み込んでいる文書の View。
    pub view: ViewId,
    /// 読込を終えた (`Complete` / `NewFile`)。
    pub complete: bool,
    /// オーバーレイが開いていたので保留した。閉じた後に (その時点でも条件を満たしていれば) 切り替える。
    pub deferred: bool,
    /// 先行表示の時点で切り替えた (完了では切り替え直さない)。`Some(true)` は、切り替え元が何も入力していない無題の
    /// バッファで、閉じずに表示から外しただけ (完了の時点でまだ何も入力していなければ閉じる)。
    pub switched: Option<bool>,
}

/// 前進の移動の前の位置 (`note_partial_end` に渡す)。
#[derive(Clone, Copy, Debug)]
pub(crate) struct EndProbe {
    doc: DocumentId,
    head: ByteOff,
    line: usize,
}

impl EditorState {
    /// 文書 `doc` は新しく開く読込の途中か: 先行表示の `Partial`、または `file.open`・起動時のパスの読込中の `Loading`
    /// (本文が無い)。再読込・開き直しの間の `Loading` (旧本文を表示し続ける) は含めない (03 §9.2.1 の「できない操作」)。
    pub(crate) fn is_new_load(&self, doc: DocumentId) -> bool {
        match self.docs.documents.get(&doc).map(|d| d.load_state) {
            Some(LoadState::Partial) => true,
            Some(LoadState::Loading) => self.io.pending.values().any(
                |p| matches!(p, Pending::Open { doc: d, kind: OpenKind::Open, .. } if *d == doc),
            ),
            _ => false,
        }
    }

    /// コマンド `id` が、表示中の文書の新しく開く読込の間に使えないものなら、`msg.open.partial_unavailable` を出して
    /// `true` を返す (呼び側は実行せずに拒否する。保留はしない、OQ-219 の 1)。
    pub(crate) fn reject_partial_unavailable(&mut self, id: &str) -> bool {
        if !PARTIAL_UNAVAILABLE.contains(&id) {
            return false;
        }
        match self.active_document() {
            Some(doc) if self.is_new_load(doc) => {
                // 同じ理由の拒否が続くときは 1 回にまとめる (表示中なら出し直さない、04 §15.2)
                if self.shown_message_key() != Some(keys::MSG_OPEN_PARTIAL_UNAVAILABLE) {
                    self.show_message(MessageLevel::Warn, keys::MSG_OPEN_PARTIAL_UNAVAILABLE, &[]);
                }
                true
            }
            _ => false,
        }
    }

    /// 表示中の文書が読込中 (`Loading` / `Partial`) なら `info.reload.busy` を出して `true` を返す (ファイルのコマンドを
    /// 黙って拒否しない。03 §9.2.1 の「できない操作」)。
    pub(crate) fn notify_loading_busy(&mut self) -> bool {
        let Some(d) = self
            .active_document()
            .and_then(|d| self.docs.documents.get(&d))
        else {
            return false;
        };
        if !matches!(d.load_state, LoadState::Loading | LoadState::Partial) {
            return false;
        }
        let shown = d
            .path
            .as_ref()
            .map(crate::io::fspath::FsPath::display)
            .unwrap_or_default();
        self.show_message(
            MessageLevel::Info,
            keys::INFO_RELOAD_BUSY,
            &[("path", &shown)],
        );
        true
    }

    /// 先行表示の先頭 (`JobResult::Partial`) を受けた (03 §9.2.1 の「状態」・「Revision」・「判定のやり直し」)。
    /// 文書を `Partial` にして本文を先頭にする。2 回目の先頭 (判定のやり直しの送り直し) は先頭を差し替え、全 View の
    /// カーソル・選択・スクロールを文書の先頭に戻す。表示の切り替えは完了と同じ条件で行う。
    pub(crate) fn on_open_partial(&mut self, job: JobId, head: Text, format: TextFormat) {
        let Some(Pending::Open {
            doc,
            view,
            replace,
            kind,
            ..
        }) = self.io.pending.get(&job)
        else {
            return;
        };
        // 先行表示は新しく開く読込だけ (再読込・開き直しは `OpenOptions.partial` を立てない)
        if *kind != OpenKind::Open {
            return;
        }
        let (doc, view, replace) = (*doc, *view, *replace);
        let Some(old) = self.docs.documents.get(&doc) else {
            return;
        };
        let again = old.load_state == LoadState::Partial;
        let mut d = Document::from_loaded(head, format, PreservedMap::new());
        d.path = old.path.clone();
        d.load_state = LoadState::Partial;
        d.load_percent = old.load_percent;
        d.load_total = old.load_total;
        // Revision は前の文書 (`Loading` の空の文書・前の先頭) の続き。新しい `Document` はレイアウトの覚え (長大行の列の
        // 索引を含む、03 §14.4) を持たないので、前の本文の覚えはここで捨てる
        self.put_document(doc, d);
        self.forget_doc_memos(doc);
        if again {
            self.reset_views_of(doc);
        }
        self.mark_snapshot_update(doc);
        if let Some(from) = replace {
            self.io.open_switch.entry(doc).or_insert(OpenSwitch {
                from,
                view,
                complete: false,
                deferred: false,
                switched: None,
            });
            self.try_open_switch(doc);
        }
        self.mark_dirty_render();
    }

    /// 本文を差し替えた文書の、(文書, Revision) を鍵にした承認・強調を捨てる (03 §9.2.1 の「Revision」。Revision は続きなので
    /// 鍵の照合でも古くなるが、先頭の間に作られたものを全文に持ち越さないよう明示して捨てる)。
    pub(crate) fn forget_doc_memos(&mut self, doc: DocumentId) {
        let io = &mut self.io;
        io.lossy_ok.take_if(|(d, _)| *d == doc);
        io.readonly_ok.take_if(|(d, _)| *d == doc);
        io.escape_merge_ok.take_if(|(d, _, _)| *d == doc);
        io.unmappable_marks.take_if(|(d, _, _)| *d == doc);
        io.link_create_ok.take_if(|(d, _)| *d == doc);
    }

    /// 文書 `doc` の全 View のカーソル・選択・スクロールを文書の先頭に戻す。
    fn reset_views_of(&mut self, doc: DocumentId) {
        for v in self.docs.views.values_mut().filter(|v| v.doc == doc) {
            v.selection = Selection::point(ByteOff(0));
            v.scroll = ScrollPos::default();
            v.goal_dcol = None;
            v.cursor_affinity = Default::default();
        }
    }

    /// 先行表示の文書を全文へ差し替えた後 (03 §9.2.1 の「全文への切り替え」): カーソル・選択・スクロール・上下の移動の
    /// 目標の列は引き継ぎ (先頭は全文の接頭辞なので、バイト位置がそのまま通る)、全 View の選択を書記素の境界へ寄せる
    /// (先頭の末尾の書記素に、全文では結合する文字が続くことがあるため)。
    pub(crate) fn snap_views_after_partial(&mut self, doc: DocumentId) {
        let Some(d) = self.docs.documents.get(&doc) else {
            return;
        };
        let text = d.text();
        for v in self.docs.views.values_mut().filter(|v| v.doc == doc) {
            v.selection = v.selection.clone().normalized(text);
        }
    }

    /// 先行表示の文書を `Loading` (本文は空) に戻す (03 §9.2.1 の「確認」。読込中のメモリ検査の確認が先行表示の後に来た
    /// とき。今のジョブは先行表示の後に確認を送らない)。全 View の選択とスクロールを 0 に戻す。先行表示の時点の切り替えを
    /// 保留していたら、その保留は落とす (確認の後に、本文の無い `Loading` の文書へ切り替えない。回答で投入し直した読込が
    /// 改めて先頭を送った時点で、もう一度切り替えを見る)。切り替えた後なら表示はそのまま (03 §9.2.1 の「確認」)。
    pub(crate) fn drop_partial_head(&mut self, doc: DocumentId) {
        let Some(old) = self.docs.documents.get(&doc) else {
            return;
        };
        if old.load_state != LoadState::Partial {
            return;
        }
        let mut d = Document::new_untitled();
        d.path = old.path.clone();
        d.load_state = LoadState::Loading;
        d.load_percent = old.load_percent;
        d.load_total = old.load_total;
        self.put_document(doc, d);
        self.forget_doc_memos(doc);
        self.reset_views_of(doc);
        self.io.partial_end_noted.remove(&doc);
        if let Some(e) = self.io.open_switch.get_mut(&doc)
            && e.switched.is_none()
        {
            e.deferred = false;
        }
        self.mark_snapshot_update(doc);
    }

    /// 何も入力していない無題のバッファ (パスなし・未編集・空・読込済みで、その文書を待つジョブ (保存・スワップ・
    /// 復旧の読込など) が無い。03 §9.8 の `file.open` の行の「例外」) か。
    pub(crate) fn is_pristine_untitled(&self, doc: DocumentId) -> bool {
        let quiet = self.docs.documents.get(&doc).is_some_and(|d| {
            d.path.is_none()
                && !d.is_dirty()
                && d.len_bytes() == 0
                && d.load_state == LoadState::Ready
        });
        quiet
            && !self.io.closing_snapshots.contains_key(&doc)
            && !self
                .io
                .pending
                .values()
                .any(|p| pending_doc(self, p) == Some(doc))
    }

    /// 自動の表示の移動 (先行表示・完了の切り替え、読込の失敗・取消しで表示を戻す) を保留するか: オーバーレイ (入力・確認・
    /// パレット・一覧・ヘルプ) か検索バーが開いている間 (03 §9.2.1 の「表示の切り替え」。入力の途中で下の文書を替えない)。
    pub(crate) fn auto_switch_blocked(&self) -> bool {
        self.ui.overlay.is_some() || self.find.search_bar.is_some()
    }

    /// 自動の切り替えで表示を `from` から `to` へ移したとき、ほかの待っている読込の切り替え元を `to` に書き換える
    /// (03 §9.8 の `file.open` の行の「切り替え元の書き換え」)。続けて開いた読込が、利用者の見ていない元の文書を基準に
    /// 表示を奪わないため。`to` 自身の読込の切り替え元 (失敗で戻す先) は書き換えない。
    fn retarget_open_sources(&mut self, from: DocumentId, to: DocumentId) {
        for p in self.io.pending.values_mut() {
            if let Pending::Open { doc, replace, .. } = p
                && *doc != to
                && *replace == Some(from)
            {
                *replace = Some(to);
            }
        }
        for (d, e) in &mut self.io.open_switch {
            if *d != to && e.from == from {
                e.from = to;
            }
        }
    }

    /// 文書 `doc` の読込の表示の切り替えを試みる (03 §9.2.1 の「表示の切り替え」)。切り替え元をまだ表示していれば
    /// 切り替える。オーバーレイ・検索バーが開いている間は保留する。先行表示で切り替えた後の完了では、表示から外した
    /// 何も入力していない無題のバッファを (まだそのままで、表示していなければ) 閉じる。
    pub(crate) fn try_open_switch(&mut self, doc: DocumentId) {
        let Some(sw) = self.io.open_switch.get(&doc).copied() else {
            return;
        };
        if !self.docs.documents.contains_key(&doc) {
            self.io.open_switch.remove(&doc);
            return;
        }
        if let Some(pristine_from) = sw.switched {
            if !sw.complete {
                return;
            }
            if pristine_from && self.auto_switch_blocked() {
                // 一覧の行がずれないよう、オーバーレイ・検索バーの間は閉じない
                if let Some(e) = self.io.open_switch.get_mut(&doc) {
                    e.deferred = true;
                }
                return;
            }
            self.io.open_switch.remove(&doc);
            if pristine_from
                && self.active_document() != Some(sw.from)
                && self.is_pristine_untitled(sw.from)
            {
                self.close_document(sw.from);
                self.docs.recent.retain(|d| *d != sw.from);
            }
            return;
        }
        if self.active_document() != Some(sw.from) {
            // 利用者が別の文書へ切り替えた: 切り替えない (完了の前なら、完了の時点でもう一度見る)
            if sw.complete {
                self.io.open_switch.remove(&doc);
            } else if let Some(e) = self.io.open_switch.get_mut(&doc) {
                e.deferred = false;
            }
            return;
        }
        if self.auto_switch_blocked() {
            // 入力の途中で下の文書を替えない (03 §9.8 の「検索バー・オーバーレイを開いたままの切替」)
            if let Some(e) = self.io.open_switch.get_mut(&doc) {
                e.deferred = true;
            }
            return;
        }
        let pristine = self.is_pristine_untitled(sw.from);
        self.retarget_open_sources(sw.from, doc);
        if sw.complete {
            self.io.open_switch.remove(&doc);
            if pristine {
                // 何も失わない置き換え: 無題のバッファを開いたファイルで置き換える (一覧の番号を増やさない)
                self.show_view(sw.view, Some(sw.from));
            } else {
                self.activate_document(doc);
            }
        } else {
            // 先行表示の時点: 無題のバッファは閉じずに表示から外すだけ (完了・失敗で決める)
            if let Some(e) = self.io.open_switch.get_mut(&doc) {
                e.deferred = false;
                e.switched = Some(pristine);
            }
            self.activate_document(doc);
        }
    }

    /// 読込の完了 (`Complete` / `NewFile`) での表示の切り替え (03 §9.8 の `file.open` の行、§9.2.1)。先行表示で
    /// 切り替えた後は切り替え直さない (`try_open_switch`)。
    pub(crate) fn open_switch_on_complete(
        &mut self,
        doc: DocumentId,
        view: ViewId,
        replace: Option<DocumentId>,
    ) {
        if let Some(e) = self.io.open_switch.get_mut(&doc) {
            e.complete = true;
        } else if let Some(from) = replace {
            self.io.open_switch.insert(
                doc,
                OpenSwitch {
                    from,
                    view,
                    complete: true,
                    deferred: false,
                    switched: None,
                },
            );
        }
        self.try_open_switch(doc);
    }

    /// 読込の失敗・取消し・確認での中止で、表示中の読込の文書を閉じて開く前の文書へ表示を戻すのを、オーバーレイ・
    /// 検索バーが閉じるまで保留する (03 §9.2.1 の「失敗・取消し・閉じる」)。先頭はここで捨て (本文の無い `Failed`
    /// にする)、閉じるのは表示を移すときに行う。
    pub(crate) fn hold_failed_open(&mut self, doc: DocumentId, replace: Option<DocumentId>) {
        let Some(old) = self.docs.documents.get(&doc) else {
            return;
        };
        let mut d = Document::new_untitled();
        d.path = old.path.clone();
        d.load_state = LoadState::Failed;
        self.put_document(doc, d);
        self.forget_doc_memos(doc);
        self.reset_views_of(doc);
        self.mark_snapshot_update(doc);
        self.io.failed_holds.insert(doc, replace);
    }

    /// オーバーレイ・検索バーが閉じた後に、保留した表示の移動 (切り替え・失敗の戻し・無題の片付け) を行う (Editor が
    /// 状態を変える呼出しの最後に呼ぶ)。
    pub(crate) fn flush_open_switches(&mut self) {
        if self.auto_switch_blocked() {
            return;
        }
        self.flush_stdin_continuations();
        let holds: Vec<(DocumentId, Option<DocumentId>)> =
            std::mem::take(&mut self.io.failed_holds)
                .into_iter()
                .collect();
        for (doc, replace) in holds {
            if self.docs.documents.contains_key(&doc) {
                self.leave_failed_open(doc, replace);
            }
        }
        if self.io.open_switch.is_empty() {
            return;
        }
        let deferred: Vec<DocumentId> = self
            .io
            .open_switch
            .iter()
            .filter(|(_, s)| s.deferred)
            .map(|(d, _)| *d)
            .collect();
        for d in deferred {
            self.try_open_switch(d);
        }
    }

    /// 読込が終わった (失敗・取消し・確認での中止・閉じる) 文書の、先行表示の印を捨てる。
    pub(crate) fn forget_partial_state(&mut self, doc: DocumentId) {
        self.io.open_switch.remove(&doc);
        self.io.partial_end_noted.remove(&doc);
        self.io.failed_holds.remove(&doc);
    }

    /// 前進の移動 `id` の前の位置 (表示中の文書が先行表示のときだけ)。
    pub(crate) fn partial_end_probe(&self, id: &str) -> Option<EndProbe> {
        if !FORWARD_MOTIONS.contains(&id) {
            return None;
        }
        let doc = self.active_document()?;
        let d = self.docs.documents.get(&doc)?;
        if d.load_state != LoadState::Partial {
            return None;
        }
        let v = self.view(self.focused_view()?)?;
        let head = v.selection.primary().head;
        Some(EndProbe {
            doc,
            head,
            line: d.text().byte_to_line(head).0,
        })
    }

    /// 前進の移動の後: 先頭の末尾より先へ動こうとした (末尾の行での下・`PageDown`、末尾での右、`cursor.doc_end`、
    /// 途中で切れた末尾の行の `End`) なら、`info.open.partial_end` を出す (同じ文書の先行表示の間は 1 回だけ)。
    /// 移動は文書 (先頭) の範囲に収まるので、末尾に止まっている。
    pub(crate) fn note_partial_end(&mut self, id: &str, probe: EndProbe) {
        if self.active_document() != Some(probe.doc)
            || self.io.partial_end_noted.contains(&probe.doc)
        {
            return;
        }
        let Some(d) = self.docs.documents.get(&probe.doc) else {
            return;
        };
        if d.load_state != LoadState::Partial {
            return;
        }
        let Some(after) = self
            .focused_view()
            .and_then(|v| self.view(v))
            .map(|v| v.selection.primary().head)
        else {
            return;
        };
        let t = d.text();
        let len = t.len_bytes();
        let last_line = t.byte_to_line(ByteOff(len)).0;
        // 末尾の行が行の途中で切れている (長大行の途中で切った先頭。行の区切りで切った先頭の末尾の行は空)
        let mid_line = t.line_to_byte(eve_text::LineIdx(last_line)).0 < len;
        let beyond = match id {
            ids::CURSOR_DOC_END => true,
            ids::CURSOR_MOVE_DOWN | ids::SELECT_EXTEND_DOWN => probe.line == last_line,
            ids::CURSOR_PAGE_DOWN | ids::SELECT_EXTEND_PAGE_DOWN => {
                t.byte_to_line(after).0 == last_line
            }
            ids::CURSOR_END | ids::SELECT_EXTEND_END => {
                mid_line && after.0 == len && probe.line == last_line
            }
            _ => probe.head.0 == len,
        };
        if beyond {
            let size = human_size(len as u64);
            self.io.partial_end_noted.insert(probe.doc);
            self.show_message(
                MessageLevel::Info,
                keys::INFO_OPEN_PARTIAL_END,
                &[("size", &size)],
            );
        }
    }
}

/// 先行表示の間の行数の推定 (03 §11.4、§9.2.1 の「ステータスバー」): 先頭の行数 × ファイルの大きさ / 先頭のバイト数
/// (UTF-8)。行番号の欄の幅だけに使う。先行表示でない文書は先頭の行数のまま。
pub(crate) fn estimated_lines(doc: &Document) -> usize {
    let lines = doc.text().len_lines().max(1);
    let head = doc.len_bytes() as u128;
    if doc.load_state != LoadState::Partial || head == 0 {
        return lines;
    }
    let total = u128::from(doc.load_total);
    if total <= head {
        return lines;
    }
    let est = (lines as u128).saturating_mul(total) / head;
    usize::try_from(est).unwrap_or(usize::MAX).max(lines)
}

/// 待ちのジョブ `p` が結果を待っている文書 (`is_pristine_untitled` の「その文書を待つジョブ」)。
fn pending_doc(state: &EditorState, p: &Pending) -> Option<DocumentId> {
    match p {
        Pending::Open { doc, .. }
        | Pending::OpenStdin { doc, .. }
        | Pending::CheckUnchanged { doc }
        | Pending::ExternalCheck { doc, .. }
        | Pending::FindNext { doc, .. }
        | Pending::ReplaceAll { doc, .. }
        | Pending::ReplaceOne { doc, .. }
        | Pending::Swap { doc }
        | Pending::SwapDiscard { doc }
        | Pending::SwapRead { doc, .. }
        | Pending::Save { doc, .. } => Some(*doc),
        Pending::ConvertEol(c) => Some(c.doc),
        Pending::ReplaceUnmappable { ask, .. } => Some(ask.doc),
        Pending::WriteRecovery { release, .. } => *release,
        Pending::Search => state.find.search.as_ref().map(|s| s.doc()),
        Pending::ConfigReload
        | Pending::ThemeRead
        | Pending::RecoveryScan
        | Pending::LastVersion
        | Pending::Leftover(_)
        | Pending::SwapRemove { .. } => None,
    }
}
