//! オーバーレイ (確認ダイアログと入力プロンプト、docs/04 §8、docs/03 §9.7、docs/02 §8.2)。
//!
//! v0.1 の範囲: 未保存の変更の確認 (終了・別のファイルを開く・新規作成による置き換え。FR-107)、
//! 入力プロンプト (開く・名前を付けて保存・行へ移動)。オーバーレイが出ている間はモーダルで、キーマップの
//! コンテキストは確認が `dialog` のみ、プロンプトが `prompt` → `global`。

mod app_cmds;
mod input_field;
mod prompt;
mod quit_chain;

pub(crate) use app_cmds::register;
#[cfg(feature = "test-hooks")]
pub(crate) use app_cmds::test_hook_panic_command;
pub(crate) use input_field::field_selection;
pub(crate) use prompt::{PromptKind, PromptState};
pub(crate) use quit_chain::{ChoiceDef, ConfirmAction, ConfirmState, Continuation, QuitChain};

use eve_text::Selection;
use eve_view::{EditorEvent, OverlayKind};

use crate::command::{CommandResult, RejectReason};

use crate::editor::state::EditorState;

#[derive(Clone, Debug)]
pub(crate) enum Overlay {
    // 大きいので Box にする (clippy::large_enum_variant。Prompt との差が大きい)
    Confirm(Box<ConfirmState>),
    Prompt(PromptState),
    Palette(crate::ui_model::palette::PaletteState),
    /// ヘルプ画面 (全画面、04 §8.6)。
    Help(crate::ui_model::help::HelpState),
    /// ファインダ (一覧のオーバーレイ、03 §12.7.4・04 §8.11)。
    Finder(Box<crate::workspace::finder::FinderOverlay>),
    /// 横断検索の結果 (一覧のオーバーレイ、03 §12.7.5・04 §8.12)。
    WorkspaceSearch(Box<crate::workspace::ws_search_ui::WsSearchOverlay>),
}

impl EditorState {
    /// オーバーレイを置き換える。種類が変わる (開く・閉じる・種類が替わる) ときは `OverlayChanged` を積む。
    pub(crate) fn set_overlay(&mut self, new: Option<Overlay>) {
        let kind = |o: &Option<Overlay>| {
            o.as_ref().map(|o| match o {
                Overlay::Confirm(_) => OverlayKind::Confirm,
                Overlay::Prompt(_) => OverlayKind::Prompt,
                Overlay::Palette(_) => OverlayKind::List,
                Overlay::Help(_) => OverlayKind::Help,
                Overlay::Finder(_) | Overlay::WorkspaceSearch(_) => OverlayKind::List,
            })
        };
        let (before, after) = (kind(&self.ui.overlay), kind(&new));
        let mut old = std::mem::replace(&mut self.ui.overlay, new);
        self.overlay_closed(&mut old);
        if before != after
            && let Some(window) = self.focused_window()
        {
            self.push_event(EditorEvent::OverlayChanged {
                window,
                overlay: after,
            });
        }
    }

    pub(crate) fn take_overlay(&mut self) -> Option<Overlay> {
        let mut old = self.ui.overlay.take();
        self.overlay_closed(&mut old);
        if old.is_some()
            && let Some(window) = self.focused_window()
        {
            self.push_event(EditorEvent::OverlayChanged {
                window,
                overlay: None,
            });
        }
        old
    }

    /// 閉じたオーバーレイの後始末 (ファインダの照合の取消し・横断検索の状態を持っておく)。
    fn overlay_closed(&mut self, old: &mut Option<Overlay>) {
        match old {
            Some(Overlay::Finder(f)) => self.finder_closed(f),
            Some(Overlay::WorkspaceSearch(w)) => self.ws_search_closed(std::mem::take(&mut **w)),
            _ => {}
        }
    }

    /// 出ている確認ダイアログの名前 (`ui.confirm.<name>` の `<name>`)。確認が出ていなければ `None` (試験用)。
    #[doc(hidden)]
    pub fn confirm_name(&self) -> Option<&'static str> {
        match &self.ui.overlay {
            Some(Overlay::Confirm(c)) => Some(c.name),
            _ => None,
        }
    }

    /// 出ているオーバーレイの種類。条件式 `overlay.active` の値と同じ (07 §7.3、11 §6.3 の `overlay`):
    /// `""` (なし) / `"dialog"` (確認) / `"prompt"` (入力プロンプト) / `"palette"` (コマンドパレット) /
    /// `"buffer_list"` (バッファ一覧) / `"help"` (ヘルプ) / `"finder"` (ファインダ) / `"workspace_search"` (横断検索)。
    pub fn overlay_kind(&self) -> &'static str {
        self.overlay_name()
    }

    /// 条件式 `overlay.active` の値: 出ていなければ空、確認は `dialog`、プロンプトは `prompt`、
    /// パレットは `palette` (バッファ一覧は `buffer_list`)、ヘルプは `help`。
    pub(crate) fn overlay_name(&self) -> &'static str {
        match &self.ui.overlay {
            None => "",
            Some(Overlay::Confirm(_)) => "dialog",
            Some(Overlay::Prompt(_)) => "prompt",
            Some(Overlay::Palette(p)) if p.buffers => "buffer_list",
            Some(Overlay::Palette(p)) if p.diagnostics => "config_diagnostics",
            Some(Overlay::Palette(p)) if p.settings => "config_show",
            Some(Overlay::Palette(_)) => "palette",
            Some(Overlay::Help(_)) => "help",
            Some(Overlay::Finder(_)) => "finder",
            Some(Overlay::WorkspaceSearch(_)) => "workspace_search",
        }
    }

    /// キー入力のコンテキスト名: オーバーレイ、無ければ検索バー (`search`)、無ければ空 (編集面)。
    pub(crate) fn input_context(&self) -> &'static str {
        match self.overlay_name() {
            "" if self.find.search_bar.is_some() => "search",
            n => n,
        }
    }

    pub(crate) fn is_dialog(&self) -> bool {
        matches!(self.ui.overlay, Some(Overlay::Confirm(_)))
    }

    // ---------- 未保存の確認 ----------

    // ---------- 入力プロンプト ----------

    // ---------- オーバーレイ内のコマンド ----------

    /// `ui.confirm`: 確認はフォーカス中の選択肢を、プロンプトは入力を確定する。
    pub fn ui_confirm(&mut self) -> CommandResult {
        match &self.ui.overlay {
            Some(Overlay::Confirm(c)) => {
                let f = c.focused;
                self.choose(f)
            }
            Some(Overlay::Prompt(_)) => self.submit_prompt(),
            Some(Overlay::Palette(_)) => self.palette_confirm(),
            // ヘルプは確定するものが無い (閉じるのは `Esc`)
            Some(Overlay::Help(_)) => CommandResult::Done,
            Some(Overlay::Finder(_)) => self.finder_confirm(),
            Some(Overlay::WorkspaceSearch(_)) => self.ws_search_confirm(),
            None => CommandResult::Rejected(RejectReason::NotAvailable),
        }
    }

    /// `ui.cancel`: オーバーレイを閉じる (データは変えない)。出ていなければ何もしない。
    pub fn ui_cancel(&mut self) -> CommandResult {
        if self.ui.overlay.is_none() && self.find.search_bar.is_some() {
            self.search_close();
            return CommandResult::Done;
        }
        // パレットで引数を尋ねている途中は、`Esc` で 1 段戻る
        if self.palette_back() {
            return CommandResult::Done;
        }
        // 取り消す順 (03 §12.7.6 の細目 (9)、04 §7.5): 表示中の文書の読込 (標準入力の確認を含む) → 未編集の上書き保存の
        // 照合 → 名前を付けて保存の照合 → 表示していない文書の読込 → 表現不能の置換の一覧の作成。読込の結果は
        // `Failed { Cancelled }` で届き、旧文書があれば残る (FR-122)
        if self.ui.overlay.is_none() && self.cancel_own_loading() {
            return CommandResult::Done;
        }
        // 未編集の上書き保存の照合 (`CheckingUnchanged`) は中止する (何も書いていない。後続も捨てる、03 §9.7)
        if self.ui.overlay.is_none() && self.cancel_unchanged_check() {
            return CommandResult::Done;
        }
        // 照合を待つ間は、結果を捨てて名前の入力に戻る (03 §9.7 の `CheckTarget` の行)
        if self.ui.overlay.is_none() && self.cancel_save_as_check() {
            self.mark_dirty_render();
            return CommandResult::Done;
        }
        if self.ui.overlay.is_none() && self.cancel_hidden_loading() {
            return CommandResult::Done;
        }
        // 表現不能の置換の一覧を作っている (`BuildingReplacement`) なら、E3 のキャンセルと同じく保存を中止する (03 §9.7)
        if self.ui.overlay.is_none() && self.cancel_building_replacement() {
            return CommandResult::Done;
        }
        // 確認の `Esc` はキャンセルの選択 (保存の失敗の確認では中止を示す)
        if let Some(Overlay::Confirm(c)) = &self.ui.overlay {
            let cancel = c.cancel_index();
            return self.choose(cancel);
        }
        // 文字コードの入力の `Esc` は、開くこと自体の中止
        if let Some(Overlay::Prompt(PromptState {
            kind: PromptKind::Encoding { ask },
            ..
        })) = &self.ui.overlay
        {
            let ask = ask.clone();
            self.take_overlay();
            self.mark_dirty_render();
            self.open_aborted(&ask);
            return CommandResult::Done;
        }
        // 名前を付けて保存の名前の入力の `Esc` は保存の連鎖の中止。保存時の変換が本文に残っていれば知らせる (03 §9.3.1 の
        // SVT-8。残っていなければ v0.4 と同じく何も出さない)
        let save_as_doc = match &self.ui.overlay {
            Some(Overlay::Prompt(PromptState {
                kind: PromptKind::SaveAs { doc, .. } | PromptKind::SaveAsFormat { doc, .. },
                ..
            })) => Some(*doc),
            _ => None,
        };
        if self.take_overlay().is_some() {
            if let Some(doc) = save_as_doc {
                self.notify_transform_kept_on_abort(doc);
            }
            self.mark_dirty_render();
            return CommandResult::Done;
        }
        self.cancel_selection();
        CommandResult::Done
    }

    /// `Esc` の最後の 3 段 (04 §7.5): 複数カーソルなら主の Range だけを残し (主の選択は残す。03 §4.5.1 の MCUR-5)、Range が
    /// 1 つで選択があれば選択を解いて head の空のカーソルにする。「単語の一致」の印は残す (MCUR-2 (e))。Range が 1 つで選択も
    /// 無く、矩形選択のモードならモードを切る (03 §4.8 の BOX-1)。1 回の `Esc` は 1 つだけ行う。
    fn cancel_selection(&mut self) {
        let Some(view) = self.focused_view() else {
            return;
        };
        let Some(sel) = self.view(view).map(|v| v.selection.clone()) else {
            return;
        };
        let primary = *sel.primary();
        let next = if sel.len() >= 2 {
            Selection::single(primary)
        } else if !primary.is_empty() {
            Selection::single(primary.collapse_to_head())
        } else {
            if self.box_mode(view) {
                self.set_box_mode(view, false);
            }
            return;
        };
        let word = self.word_mark_valid(view);
        self.set_selection(view, next);
        self.set_word_mark(view, word);
        self.ensure_cursor_visible(view);
    }

    /// `ui.list_next` / `ui.list_prev`: 確認のフォーカスを移す (折り返す)。
    pub fn ui_list_move(&mut self, forward: bool) -> CommandResult {
        if matches!(self.ui.overlay, Some(Overlay::Palette(_))) {
            return self.palette_move(forward);
        }
        if matches!(self.ui.overlay, Some(Overlay::Help(_))) {
            return self.help_scroll(if forward { 1 } else { -1 });
        }
        if matches!(self.ui.overlay, Some(Overlay::Finder(_))) {
            return self.finder_move(forward);
        }
        if matches!(self.ui.overlay, Some(Overlay::WorkspaceSearch(_))) {
            return self.ws_search_move(forward);
        }
        if self.ui.overlay.is_none() && self.find.search_bar.is_some() {
            self.bar_focus_cycle(forward);
            return CommandResult::Done;
        }
        let Some(Overlay::Confirm(c)) = &mut self.ui.overlay else {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        };
        let n = c.choices.len() + 1;
        c.focused = if forward {
            (c.focused + 1) % n
        } else {
            (c.focused + n - 1) % n
        };
        self.mark_dirty_render();
        CommandResult::Done
    }

    /// `ui.dialog_choose { choice }`。
    pub fn ui_dialog_choose(&mut self, choice: usize) -> CommandResult {
        let n = match &self.ui.overlay {
            Some(Overlay::Confirm(c)) => c.choices.len(),
            _ => return CommandResult::Rejected(RejectReason::NotAvailable),
        };
        if choice > n {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        self.choose(choice)
    }

    /// 確認ダイアログのアクセラレータ (`Text` / 修飾なしのキー。大小は区別しない)。一致しなければ捨てる。
    pub fn dialog_accelerator(&mut self, c: char) {
        // 全角英字は半角に畳み、大小は区別しない (02 §7.2 K1)
        let c = match c {
            '\u{FF21}'..='\u{FF3A}' => char::from_u32(c as u32 - 0xFF21 + 'A' as u32).unwrap_or(c),
            '\u{FF41}'..='\u{FF5A}' => char::from_u32(c as u32 - 0xFF41 + 'a' as u32).unwrap_or(c),
            _ => c,
        };
        let c = c.to_lowercase().next().unwrap_or(c);
        let idx = match &self.ui.overlay {
            Some(Overlay::Confirm(st)) => st.choices.iter().position(|d| d.accel == Some(c)),
            _ => None,
        };
        if let Some(idx) = idx {
            self.choose(idx);
        }
    }
}
