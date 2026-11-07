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
            })
        };
        let (before, after) = (kind(&self.ui.overlay), kind(&new));
        self.ui.overlay = new;
        if before != after
            && let Some(window) = self.focused_window()
        {
            self.push_event(EditorEvent::OverlayChanged {
                window,
                overlay: after,
            });
        }
    }

    fn take_overlay(&mut self) -> Option<Overlay> {
        let old = self.ui.overlay.take();
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

    /// 出ているオーバーレイの種類 (`""` なし / `"dialog"` / `"prompt"`)。
    pub fn overlay_kind(&self) -> &'static str {
        self.overlay_name()
    }

    /// 条件式 `overlay.active` の値: 出ていなければ空、確認は `dialog`、プロンプトは `prompt`。
    pub(crate) fn overlay_name(&self) -> &'static str {
        match &self.ui.overlay {
            None => "",
            Some(Overlay::Confirm(_)) => "dialog",
            Some(Overlay::Prompt(_)) => "prompt",
            Some(Overlay::Palette(p)) if p.buffers => "buffer_list",
            Some(Overlay::Palette(p)) if p.diagnostics => "config_diagnostics",
            Some(Overlay::Palette(_)) => "palette",
            Some(Overlay::Help(_)) => "help",
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
        // 読込中なら取り消す (FR-122。結果は `Failed { Cancelled }` で届き、旧文書があれば残る)
        if self.ui.overlay.is_none() && self.cancel_loading() {
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
        if self.take_overlay().is_some() {
            self.mark_dirty_render();
        }
        CommandResult::Done
    }

    /// `ui.list_next` / `ui.list_prev`: 確認のフォーカスを移す (折り返す)。
    pub fn ui_list_move(&mut self, forward: bool) -> CommandResult {
        if matches!(self.ui.overlay, Some(Overlay::Palette(_))) {
            return self.palette_move(forward);
        }
        if matches!(self.ui.overlay, Some(Overlay::Help(_))) {
            return self.help_scroll(if forward { 1 } else { -1 });
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
