//! オーバーレイ (確認ダイアログと入力プロンプト、docs/04 §8、docs/03 §9.7、docs/02 §8.2)。
//!
//! v0.1 の範囲: 未保存の変更の確認 (終了・別のファイルを開く・新規作成による置き換え。FR-107)、
//! 入力プロンプト (開く・名前を付けて保存・行へ移動)。オーバーレイが出ている間はモーダルで、キーマップの
//! コンテキストは確認が `dialog` のみ、プロンプトが `prompt` → `global`。

use eve_text::{ByteOff, Text, next_grapheme_boundary, prev_grapheme_boundary};
use eve_view::{EditorEvent, Effect, MessageLevel, OverlayKind, QuitRescue};

use crate::editor::command::{
    ArgValue, Args, CommandCtx, CommandInvocation, CommandKind, CommandRegistry, CommandResult,
    CommandSpec, Permission, RejectReason,
};
use crate::io_jobs::OpenOptions;

use super::state::EditorState;

/// 保存や破棄の後に続ける操作 (03 §9.7 の連鎖の後続)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Continuation {
    /// 別のファイルを開く (入力されたままのパス)。
    Open(String),
    /// 新規の無題の文書を作る。
    New,
    /// 終了する。
    Quit,
    /// 文字コードを指定して開き直す (`file.reopen_with_encoding`、未保存の確認の後)。
    Reopen(eve_encoding::Encoding),
}

/// 確認ダイアログの選択肢 1 つ。文言は `ui.confirm.<name>.<id>`。キャンセル (`ui.confirm.cancel`、`Esc`) は選択肢の最後に自動で付く。
#[derive(Clone, Copy, Debug)]
pub(super) struct ChoiceDef {
    pub id: &'static str,
    /// アクセラレータ (小文字。大小は区別しない)。
    pub accel: Option<char>,
    /// 破棄などの破壊的な選択肢 (既定にしない)。
    pub destructive: bool,
}

/// 選択肢を選んだときの動作 (確認の種類ごと)。
#[derive(Clone, Debug)]
pub(super) enum ConfirmAction {
    /// 未保存の変更 (終了・置き換え): 選択肢 0 = 保存して続ける、1 = 破棄して続ける。
    Unsaved(Continuation),
    /// 保存の代替 (原子的置換ができない・ハードリンク・バックアップを作れない)。選択肢の意味は `SaveFallback` の種類による。
    Fallback(super::file::FallbackAsk),
    /// 別のバイト表現で保存される箇所がある文書の保存 (保存 / 別名で保存、03 §8.6 E2')。
    Lossy(super::file::LossyAsk),
    /// 保存の失敗の後の選択 (再試行 / 別名で保存 / 退避先に保存)。
    SaveFailed(super::file::SaveFailedAsk),
    /// 開くときの確認 (読み取り専用で開く / 文字コードを指定して開く / 中止、03 §9.2)。
    Open(super::file::OpenAsk),
    /// dirty な文書の再読込 (変更を破棄、03 §9.5)。
    Reload,
    /// 表現不能の文字があって保存を中止した後の選択 (UTF-8 で保存 / キャンセル、03 §8.6)。
    Unmappable(super::file::UnmappableAsk),
    /// 置換の一覧の確認 (置換して保存 / キャンセル、03 §8.6)。`revision` は ChangeSet を作った時点の文書の Revision。
    UnmappableReplace {
        ask: super::file::UnmappableAsk,
        changes: eve_text::ChangeSet,
        revision: eve_view::Revision,
    },
}

/// 確認ダイアログ。選択肢は `choices` (+ 最後にキャンセル)。
#[derive(Clone, Debug)]
pub(super) struct ConfirmState {
    /// `ui.confirm.<name>` の名前 (`unsaved` / `save.atomic_unsupported` など)。
    pub name: &'static str,
    /// 文言の引数 (`{path}` など)。
    pub args: Vec<(String, String)>,
    pub choices: Vec<ChoiceDef>,
    /// 本文の下に 1 行ずつ出す詳細 (E3 の置換の一覧)。無ければ空。
    pub details: Vec<String>,
    /// 初めにフォーカスする選択肢 (`choices.len()` はキャンセル)。
    pub default: usize,
    pub focused: usize,
    pub action: ConfirmAction,
}

impl ConfirmState {
    /// キャンセルの番号 (選択肢の数)。
    pub(super) fn cancel_index(&self) -> usize {
        self.choices.len()
    }
}

/// 未保存の確認の選択肢 (既定は保存)。
pub(super) const UNSAVED_CHOICES: [ChoiceDef; 2] = [
    ChoiceDef {
        id: "save",
        accel: Some('s'),
        destructive: false,
    },
    ChoiceDef {
        id: "discard",
        accel: Some('d'),
        destructive: true,
    },
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PromptKind {
    Open,
    SaveAs {
        then: Option<Continuation>,
    },
    GotoLine,
    /// 開くときの確認で「文字コードを指定して開く」を選んだ後の文字コードの入力 (03 §9.2)。
    Encoding {
        ask: super::file::OpenAsk,
    },
    /// `file.reopen_with_encoding` に引数が無いときの文字コードの入力 (03 §8.8)。
    ReopenEncoding,
    /// `file.set_encoding` に引数が無いときの文字コードの入力 (03 §8.8)。
    SetEncoding {
        bom: Option<bool>,
    },
    /// `file.set_encoding` で、パスのない・読み取り専用の文書の名前の入力 (名前を付けて保存へ新しい書式を渡す)。
    SaveAsFormat {
        fmt: super::file::NewFormat,
    },
}

impl PromptKind {
    /// `ui.prompt.<name>` の名前。
    pub(super) fn name(&self) -> &'static str {
        match self {
            PromptKind::Open => "open",
            PromptKind::SaveAs { .. } => "save_as",
            PromptKind::GotoLine => "goto_line",
            PromptKind::Encoding { .. }
            | PromptKind::ReopenEncoding
            | PromptKind::SetEncoding { .. } => "encoding",
            PromptKind::SaveAsFormat { .. } => "save_as",
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct PromptState {
    pub kind: PromptKind,
    pub text: String,
    /// `text` 内のカーソル (バイト。書記素境界)。
    pub cursor: usize,
    /// 検証エラーの MessageKey の末尾 (`ui.prompt.<name>.error.<kind>`)。
    pub error: Option<&'static str>,
}

#[derive(Clone, Debug)]
pub(super) enum Overlay {
    // 大きいので Box にする (clippy::large_enum_variant。Prompt との差が大きい)
    Confirm(Box<ConfirmState>),
    Prompt(PromptState),
    Palette(super::palette::PaletteState),
}

fn text_of(s: &str) -> Text {
    Text::from(s)
}

impl EditorState {
    /// オーバーレイを置き換える。種類が変わる (開く・閉じる・種類が替わる) ときは `OverlayChanged` を積む。
    pub(super) fn set_overlay(&mut self, new: Option<Overlay>) {
        let kind = |o: &Option<Overlay>| {
            o.as_ref().map(|o| match o {
                Overlay::Confirm(_) => OverlayKind::Confirm,
                Overlay::Prompt(_) => OverlayKind::Prompt,
                Overlay::Palette(_) => OverlayKind::List,
            })
        };
        let (before, after) = (kind(&self.overlay), kind(&new));
        self.overlay = new;
        if before != after
            && let Some(window) = self.first_window()
        {
            self.push_event(EditorEvent::OverlayChanged {
                window,
                overlay: after,
            });
        }
    }

    fn take_overlay(&mut self) -> Option<Overlay> {
        let old = self.overlay.take();
        if old.is_some()
            && let Some(window) = self.first_window()
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
    pub(super) fn overlay_name(&self) -> &'static str {
        match &self.overlay {
            None => "",
            Some(Overlay::Confirm(_)) => "dialog",
            Some(Overlay::Prompt(_)) => "prompt",
            Some(Overlay::Palette(_)) => "palette",
        }
    }

    /// キー入力のコンテキスト名: オーバーレイ、無ければ検索バー (`search`)、無ければ空 (編集面)。
    pub(super) fn input_context(&self) -> &'static str {
        match self.overlay_name() {
            "" if self.search_bar.is_some() => "search",
            n => n,
        }
    }

    pub(super) fn is_dialog(&self) -> bool {
        matches!(self.overlay, Some(Overlay::Confirm(_)))
    }

    // ---------- 未保存の確認 ----------

    /// 今の文書が dirty なら確認を出して `true`。`name` は `unsaved` (終了) か `unsaved_replace` (置き換え)。
    pub(super) fn confirm_if_dirty(&mut self, name: &'static str, cont: Continuation) -> bool {
        let Some(doc) = self.active_document().and_then(|d| self.document(d)) else {
            return false;
        };
        if !doc.is_dirty() {
            return false;
        }
        let path = doc
            .path
            .as_ref()
            .map(|p| p.display())
            .unwrap_or_else(|| self.untitled_name());
        self.set_overlay(Some(Overlay::Confirm(Box::new(ConfirmState {
            name,
            args: vec![("path".to_owned(), path)],
            choices: UNSAVED_CHOICES.to_vec(),
            details: Vec::new(),
            default: 0,
            focused: 0,
            action: ConfirmAction::Unsaved(cont),
        }))));
        self.mark_dirty_render();
        true
    }

    /// 確認ダイアログを出す (種類は `action`)。
    pub(super) fn open_confirm(&mut self, c: ConfirmState) {
        self.set_overlay(Some(Overlay::Confirm(Box::new(c))));
        self.mark_dirty_render();
    }

    fn untitled_name(&self) -> String {
        self.catalog()
            .render("status.untitled", &[("n", "1")], &|_| None)
    }

    fn choose(&mut self, choice: usize) -> CommandResult {
        let Some(Overlay::Confirm(c)) = self.take_overlay() else {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        };
        self.mark_dirty_render();
        // キャンセル (最後の選択肢): データを変えない
        if choice >= c.cancel_index() {
            match &c.action {
                ConfirmAction::SaveFailed(_) | ConfirmAction::Lossy(_) => {
                    self.show_message(MessageLevel::Info, "info.save.aborted", &[]);
                }
                ConfirmAction::Open(ask) => self.open_aborted(ask),
                ConfirmAction::Unmappable(ask) | ConfirmAction::UnmappableReplace { ask, .. } => {
                    self.unmappable_cancelled(ask)
                }
                _ => {}
            }
            return CommandResult::Done;
        }
        let id = c.choices[choice].id;
        match c.action {
            ConfirmAction::Unsaved(cont) => match choice {
                0 => self.save_then(cont),
                _ => self.run_continuation(cont),
            },
            ConfirmAction::Fallback(ask) => self.choose_fallback(ask, choice),
            ConfirmAction::Lossy(ask) => self.choose_lossy(ask, choice),
            ConfirmAction::SaveFailed(ask) => self.choose_save_failed(ask, id),
            ConfirmAction::Open(ask) => self.choose_open(ask, id),
            ConfirmAction::Reload => self.start_reload(),
            ConfirmAction::Unmappable(ask) => self.choose_unmappable(ask, id),
            ConfirmAction::UnmappableReplace {
                ask,
                changes,
                revision,
            } => self.apply_replacement_and_save(ask, changes, revision),
        }
    }

    /// 破棄または保存の後に続ける操作を実行する。
    pub(super) fn run_continuation(&mut self, cont: Continuation) -> CommandResult {
        match cont {
            Continuation::Open(path) => self.open_path_forced(&path, OpenOptions::default()),
            Continuation::New => self.new_untitled_forced(),
            Continuation::Reopen(enc) => self.start_reopen(enc),
            Continuation::Quit => {
                self.push_effect(Effect::Quit {
                    code: 0,
                    rescue: QuitRescue::None,
                    stderr: Vec::new(),
                });
                CommandResult::Done
            }
        }
    }

    /// 終了する。dirty なら確認を出す。
    pub fn request_quit(&mut self) -> CommandResult {
        if self.confirm_if_dirty("unsaved", Continuation::Quit) {
            return CommandResult::Done;
        }
        self.run_continuation(Continuation::Quit)
    }

    // ---------- 入力プロンプト ----------

    pub(super) fn open_prompt(&mut self, kind: PromptKind, initial: String) {
        let cursor = initial.len();
        self.set_overlay(Some(Overlay::Prompt(PromptState {
            kind,
            text: initial,
            cursor,
            error: None,
        })));
        self.mark_dirty_render();
    }

    fn prompt_mut(&mut self) -> Option<&mut PromptState> {
        match &mut self.overlay {
            Some(Overlay::Prompt(p)) => Some(p),
            _ => None,
        }
    }

    /// 貼り付けを受けられる入力欄があるか。
    pub(super) fn input_mut_available(&mut self) -> bool {
        self.input_mut().is_some()
    }

    /// 入力欄 (プロンプトとパレットで共通の編集)。
    fn input_mut(&mut self) -> Option<(&mut String, &mut usize)> {
        match &mut self.overlay {
            Some(Overlay::Prompt(p)) => Some((&mut p.text, &mut p.cursor)),
            Some(Overlay::Palette(p)) => Some((&mut p.text, &mut p.cursor)),
            None => {
                let b = self.search_bar.as_mut()?;
                match b.focus {
                    eve_view::BarFocus::Search => Some((&mut b.text, &mut b.cursor)),
                    eve_view::BarFocus::Replace => b.replace.as_mut().map(|(t, c)| (t, c)),
                    eve_view::BarFocus::Button => None,
                }
            }
            _ => None,
        }
    }

    /// プロンプトの入力欄を編集するコマンドを横取りする (オーバーレイの間は文書を編集しない)。
    /// オーバーレイが出ていて、オーバーレイ用でも編集用でもないコマンドは拒否する (モーダル)。
    pub(super) fn overlay_intercept(&mut self, inv: &CommandInvocation) -> Option<CommandResult> {
        if self.overlay.is_none() && self.search_bar.is_none() {
            return None;
        }
        let id = inv.id.as_str();
        if id.starts_with("ui.") || id == "app.quit" {
            return None;
        }
        // 入力欄への貼り付け: システムクリップボードを要求し、応答は入力欄の最初の行を入れる (`on_clipboard_content`)
        if id == "edit.paste" {
            return Some(if self.input_mut().is_some() {
                self.request_paste()
            } else {
                CommandResult::Rejected(RejectReason::NotAvailable)
            });
        }
        // 検索バーはモーダルではない: 入力欄の編集だけを受け、それ以外のコマンドは通常どおり実行する
        if self.overlay.is_none()
            && !matches!(
                id,
                "edit.insert_text"
                    | "edit.delete_backward"
                    | "edit.delete_forward"
                    | "cursor.move_left"
                    | "cursor.move_right"
                    | "cursor.home"
                    | "cursor.end"
            )
        {
            return None;
        }
        let is_prompt = matches!(self.overlay, Some(Overlay::Prompt(_)));
        let Some((text, cursor)) = self.input_mut() else {
            return Some(CommandResult::Rejected(RejectReason::NotAvailable));
        };
        let t = text_of(text);
        let mut edited = false;
        match id {
            "edit.insert_text" => {
                if let Some(s) = inv.args.str("text") {
                    // 入力欄は 1 行: 改行は取り除く
                    let s: String = s.chars().filter(|c| !matches!(c, '\n' | '\r')).collect();
                    text.insert_str(*cursor, &s);
                    *cursor += s.len();
                    edited = true;
                }
            }
            "edit.delete_backward" => {
                let b = prev_grapheme_boundary(&t, ByteOff(*cursor)).0;
                text.replace_range(b..*cursor, "");
                *cursor = b;
                edited = true;
            }
            "edit.delete_forward" => {
                let e = next_grapheme_boundary(&t, ByteOff(*cursor)).0;
                text.replace_range(*cursor..e, "");
                edited = true;
            }
            "cursor.move_left" => *cursor = prev_grapheme_boundary(&t, ByteOff(*cursor)).0,
            "cursor.move_right" => *cursor = next_grapheme_boundary(&t, ByteOff(*cursor)).0,
            "cursor.home" => *cursor = 0,
            "cursor.end" => *cursor = text.len(),
            _ => return Some(CommandResult::Rejected(RejectReason::NotAvailable)),
        }
        if edited {
            if is_prompt {
                if let Some(p) = self.prompt_mut() {
                    p.error = None;
                }
            } else if self.overlay.is_none() {
                self.bar_edited();
            } else {
                self.palette_reset_selection();
            }
        }
        self.mark_dirty_render();
        Some(CommandResult::Done)
    }

    /// 貼り付けを入力欄へ (改行は取り除く)。
    pub(super) fn prompt_paste(&mut self, s: &str) -> bool {
        if self.input_mut().is_none() {
            return false;
        }
        let inv =
            CommandInvocation::new("edit.insert_text", super::command::InvocationSource::Keymap)
                .with_args(Args::new().with_str("text", s));
        self.overlay_intercept(&inv);
        true
    }

    fn submit_prompt(&mut self) -> CommandResult {
        let Some(Overlay::Prompt(p)) = self.overlay.clone() else {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        };
        let text = p.text.trim().to_owned();
        match &p.kind {
            PromptKind::Open | PromptKind::SaveAs { .. } | PromptKind::SaveAsFormat { .. }
                if text.is_empty() =>
            {
                self.prompt_error("empty")
            }
            PromptKind::Open => {
                let res = self.run_from_prompt(|st| st.open_path(&text, OpenOptions::default()));
                self.restore_prompt_if_rejected(Overlay::Prompt(p.clone()), &res, false);
                res
            }
            PromptKind::SaveAs { then } => {
                let res = self.run_from_prompt(|st| st.request_save_as(&text, then.clone()));
                // 保存が始まらなかったとき (不正なパス・保存中・保存できない文書) は、名前の入力に戻る (03 §9.7)
                self.restore_prompt_if_rejected(Overlay::Prompt(p.clone()), &res, true);
                res
            }
            PromptKind::Encoding { ask } => match eve_encoding::Encoding::from_label(&text) {
                Some(enc) => {
                    self.set_overlay(None);
                    self.mark_dirty_render();
                    let mut opts = ask.opts;
                    opts.forced = Some(enc);
                    self.respawn_open(ask.clone(), opts)
                }
                None => self.prompt_error("invalid"),
            },
            PromptKind::SaveAsFormat { fmt } => {
                let res =
                    self.run_from_prompt(|st| st.request_save_as_fmt(&text, None, Some(*fmt)));
                self.restore_prompt_if_rejected(Overlay::Prompt(p.clone()), &res, true);
                res
            }
            PromptKind::SetEncoding { bom } => match eve_encoding::Encoding::from_label(&text) {
                Some(enc) => {
                    self.set_overlay(None);
                    self.mark_dirty_render();
                    self.set_encoding(Some(enc), *bom)
                }
                None => self.prompt_error("invalid"),
            },
            PromptKind::ReopenEncoding => match eve_encoding::Encoding::from_label(&text) {
                Some(enc) => {
                    self.set_overlay(None);
                    self.mark_dirty_render();
                    self.reopen_with_encoding(Some(enc))
                }
                None => self.prompt_error("invalid"),
            },
            PromptKind::GotoLine => match parse_goto(&text) {
                Some(args) => {
                    self.set_overlay(None);
                    self.mark_dirty_render();
                    let mut ctx = CommandCtx {
                        state: self,
                        args: &args,
                    };
                    super::motion::goto_line_command(&mut ctx)
                }
                None => self.prompt_error("invalid"),
            },
        }
    }

    /// プロンプトを閉じてから `f` を実行する (`f` が別のオーバーレイを出すことがある)。
    fn run_from_prompt(
        &mut self,
        f: impl FnOnce(&mut EditorState) -> CommandResult,
    ) -> CommandResult {
        self.set_overlay(None);
        self.mark_dirty_render();
        f(self)
    }

    /// 確定が拒否された (または `need_job` で保存ジョブが始まらなかった) ときは、プロンプトを元に戻す。
    /// 別のオーバーレイが出ていれば、それを優先する。
    fn restore_prompt_if_rejected(&mut self, prev: Overlay, res: &CommandResult, need_job: bool) {
        let failed = match res {
            CommandResult::Rejected(_) => true,
            CommandResult::Done => need_job,
            _ => false,
        };
        if failed && self.overlay.is_none() {
            self.set_overlay(Some(prev));
            self.mark_dirty_render();
        }
    }

    fn prompt_error(&mut self, kind: &'static str) -> CommandResult {
        if let Some(p) = self.prompt_mut() {
            p.error = Some(kind);
        }
        self.mark_dirty_render();
        CommandResult::Done
    }

    // ---------- オーバーレイ内のコマンド ----------

    /// `ui.confirm`: 確認はフォーカス中の選択肢を、プロンプトは入力を確定する。
    pub fn ui_confirm(&mut self) -> CommandResult {
        match &self.overlay {
            Some(Overlay::Confirm(c)) => {
                let f = c.focused;
                self.choose(f)
            }
            Some(Overlay::Prompt(_)) => self.submit_prompt(),
            Some(Overlay::Palette(_)) => self.palette_confirm(),
            None => CommandResult::Rejected(RejectReason::NotAvailable),
        }
    }

    /// `ui.cancel`: オーバーレイを閉じる (データは変えない)。出ていなければ何もしない。
    pub fn ui_cancel(&mut self) -> CommandResult {
        if self.overlay.is_none() && self.search_bar.is_some() {
            self.search_close();
            return CommandResult::Done;
        }
        // パレットで引数を尋ねている途中は、`Esc` で 1 段戻る
        if self.palette_back() {
            return CommandResult::Done;
        }
        // 読込中なら取り消す (FR-122。結果は `Failed { Cancelled }` で届き、旧文書があれば残る)
        if self.overlay.is_none() && self.cancel_loading() {
            return CommandResult::Done;
        }
        // 確認の `Esc` はキャンセルの選択 (保存の失敗の確認では中止を示す)
        if let Some(Overlay::Confirm(c)) = &self.overlay {
            let cancel = c.cancel_index();
            return self.choose(cancel);
        }
        // 文字コードの入力の `Esc` は、開くこと自体の中止
        if let Some(Overlay::Prompt(PromptState {
            kind: PromptKind::Encoding { ask },
            ..
        })) = &self.overlay
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
        if matches!(self.overlay, Some(Overlay::Palette(_))) {
            return self.palette_move(forward);
        }
        if self.overlay.is_none() && self.search_bar.is_some() {
            self.bar_focus_cycle(forward);
            return CommandResult::Done;
        }
        let Some(Overlay::Confirm(c)) = &mut self.overlay else {
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
        let n = match &self.overlay {
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
        let idx = match &self.overlay {
            Some(Overlay::Confirm(st)) => st.choices.iter().position(|d| d.accel == Some(c)),
            _ => None,
        };
        if let Some(idx) = idx {
            self.choose(idx);
        }
    }
}

fn cmd_confirm(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.ui_confirm()
}

fn cmd_cancel(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.ui_cancel()
}

fn cmd_list_next(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.ui_list_move(true)
}

fn cmd_list_prev(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.ui_list_move(false)
}

fn cmd_dialog_choose(ctx: &mut CommandCtx<'_>) -> CommandResult {
    match ctx.args.int("choice").and_then(|n| usize::try_from(n).ok()) {
        Some(n) => ctx.state.ui_dialog_choose(n),
        None => CommandResult::Rejected(RejectReason::InvalidArgs),
    }
}

fn cmd_search_open(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.search_open()
}

fn cmd_search_toggle_case(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.search_toggle_case()
}

fn cmd_search_replace_open(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.search_replace_open()
}

fn cmd_search_replace_all(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.search_replace_all()
}

fn cmd_search_next(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.search_move(true);
    CommandResult::Done
}

fn cmd_search_prev(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.search_move(false);
    CommandResult::Done
}

/// `search.next` / `search.prev` は Motion (実行すると Undo の結合を封印する、03 §7.3 R3)。
fn reg_motion_search(r: &mut CommandRegistry) {
    let handlers: [(&str, crate::Handler); 2] = [
        ("search.next", cmd_search_next),
        ("search.prev", cmd_search_prev),
    ];
    for (id, h) in handlers {
        r.register(
            CommandSpec::new(
                id,
                CommandKind::Motion,
                Permission::Read,
                "search.active",
                h,
            )
            .expect("カタログの when 句は構文が正しい"),
        );
    }
}

fn cmd_list_page_down(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.palette_page(true)
}

fn cmd_list_page_up(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.palette_page(false)
}

fn cmd_palette_open(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.open_palette()
}

fn cmd_quit(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.request_quit()
}

macro_rules! reg_ui {
    ($r:expr, $id:expr, $when:expr, $h:expr) => {
        $r.register(
            CommandSpec::new($id, CommandKind::Ui, Permission::None, $when, $h)
                .expect("カタログの when 句は構文が正しい"),
        )
    };
}

/// オーバーレイ内のコマンドと `app.quit` (07 §11.6)。
pub(super) fn register(r: &mut CommandRegistry) {
    let active = "overlay.active != \"\" || search.active";
    reg_ui!(r, "ui.confirm", active, cmd_confirm);
    reg_ui!(r, "ui.cancel", "", cmd_cancel);
    reg_ui!(r, "ui.list_next", active, cmd_list_next);
    reg_ui!(r, "ui.list_prev", active, cmd_list_prev);
    reg_ui!(
        r,
        "ui.dialog_choose",
        "overlay.active == \"dialog\"",
        cmd_dialog_choose
    );
    reg_ui!(r, "ui.palette_open", "", cmd_palette_open);
    reg_ui!(r, "ui.list_page_down", active, cmd_list_page_down);
    reg_ui!(r, "ui.list_page_up", active, cmd_list_page_up);
    // 07 §11.6 の perm 列は `ui` (キーマップ以外からも呼べるが、外部クライアントには許可を要する)
    for (id, when, h) in [
        ("search.open", "", cmd_search_open as crate::Handler),
        (
            "search.toggle_case",
            "search.active",
            cmd_search_toggle_case,
        ),
    ] {
        r.register(
            CommandSpec::new(id, CommandKind::Ui, Permission::Ui, when, h)
                .expect("カタログの when 句は構文が正しい"),
        );
    }
    reg_motion_search(r);
    // 置換 (03 §12.5): 置換欄を開く (Ui)、すべて置換 (Edit。承認を要する `propose`)
    r.register(
        CommandSpec::new(
            "search.replace_open",
            CommandKind::Ui,
            Permission::Ui,
            "!doc.read_only",
            cmd_search_replace_open,
        )
        .expect("カタログの when 句は構文が正しい"),
    );
    r.register(
        CommandSpec::new(
            "search.replace_all",
            CommandKind::Edit,
            Permission::Propose,
            "search.replace_active && !doc.read_only",
            cmd_search_replace_all,
        )
        .expect("カタログの when 句は構文が正しい"),
    );
    reg_ui!(r, "app.quit", "", cmd_quit);
    #[cfg(feature = "test-hooks")]
    if test_hook_panic_command() {
        reg_ui!(r, "internal.debug_panic", "", cmd_debug_panic);
    }
}

/// `EVE_TEST_HOOK` に `panic_command` があるか (09 §3.6。無ければ `internal.debug_panic` を登録しない)。
#[cfg(feature = "test-hooks")]
pub(super) fn test_hook_panic_command() -> bool {
    std::env::var("EVE_TEST_HOOK").is_ok_and(|v| {
        v.split(',')
            .any(|h| h.split('=').next() == Some("panic_command"))
    })
}

/// パニックの注入 (NFR-302 AC-1)。
#[cfg(feature = "test-hooks")]
fn cmd_debug_panic(_ctx: &mut CommandCtx<'_>) -> CommandResult {
    panic!("internal.debug_panic: テストによるパニックの注入");
}

/// 行へ移動の入力 (`12` / `12:5` / `$` / `$:3`) を `cursor.goto_line` の引数にする。
pub(super) fn parse_goto(s: &str) -> Option<Args> {
    let (line, col) = match s.split_once(':') {
        Some((l, c)) => (l.trim(), Some(c.trim())),
        None => (s.trim(), None),
    };
    let mut args = Args::new();
    if line == "$" {
        args = args.with_str("line", "$");
    } else {
        args = args.with(
            "line",
            ArgValue::Int(
                line.parse::<i64>()
                    .ok()
                    .filter(|n| !line.is_empty() && *n >= 1)?,
            ),
        );
    }
    if let Some(c) = col {
        args = args.with(
            "col",
            ArgValue::Int(c.parse::<i64>().ok().filter(|_| !c.is_empty())?),
        );
    }
    Some(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 行へ移動の入力() {
        assert_eq!(parse_goto("12").unwrap().int("line"), Some(12));
        let a = parse_goto("12:5").unwrap();
        assert_eq!((a.int("line"), a.int("col")), (Some(12), Some(5)));
        assert_eq!(parse_goto("$").unwrap().str("line"), Some("$"));
        assert_eq!(parse_goto(" 7 : 2 ").unwrap().int("col"), Some(2));
        for bad in ["", "abc", "1:", ":2", "1:x", "$x", "1.5"] {
            assert!(parse_goto(bad).is_none(), "{bad:?}");
        }
    }
}
