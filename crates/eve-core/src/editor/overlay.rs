//! オーバーレイ (確認ダイアログと入力プロンプト、docs/04 §8、docs/03 §9.7、docs/02 §8.2)。
//!
//! v0.1 の範囲: 未保存の変更の確認 (終了・別のファイルを開く・新規作成による置き換え。FR-107)、
//! 入力プロンプト (開く・名前を付けて保存・行へ移動)。オーバーレイが出ている間はモーダルで、キーマップの
//! コンテキストは確認が `dialog` のみ、プロンプトが `prompt` → `global`。

use eve_text::{ByteOff, Text, next_grapheme_boundary, prev_grapheme_boundary};
use eve_view::{EditorEvent, Effect, OverlayKind, QuitRescue};

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
}

/// 未保存の確認ダイアログ。選択肢は 0 = 保存、1 = 破棄、2 = キャンセル (既定は保存)。
#[derive(Clone, Debug)]
pub(super) struct ConfirmState {
    /// `ui.confirm.<name>` の名前。
    pub name: &'static str,
    pub path: String,
    pub cont: Continuation,
    pub focused: usize,
}

pub(super) const CHOICE_SAVE: usize = 0;
pub(super) const CHOICE_DISCARD: usize = 1;
pub(super) const CHOICE_CANCEL: usize = 2;
/// アクセラレータ (大小を区別しない)。
pub(super) const ACCELERATORS: [(char, usize); 2] = [('s', CHOICE_SAVE), ('d', CHOICE_DISCARD)];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PromptKind {
    Open,
    SaveAs { then: Option<Continuation> },
    GotoLine,
}

impl PromptKind {
    /// `ui.prompt.<name>` の名前。
    pub(super) fn name(&self) -> &'static str {
        match self {
            PromptKind::Open => "open",
            PromptKind::SaveAs { .. } => "save_as",
            PromptKind::GotoLine => "goto_line",
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
    Confirm(ConfirmState),
    Prompt(PromptState),
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
        self.set_overlay(Some(Overlay::Confirm(ConfirmState {
            name,
            path,
            cont,
            focused: CHOICE_SAVE,
        })));
        self.mark_dirty_render();
        true
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
        match choice {
            CHOICE_SAVE => self.save_then(c.cont),
            CHOICE_DISCARD => self.run_continuation(c.cont),
            _ => CommandResult::Done,
        }
    }

    /// 破棄または保存の後に続ける操作を実行する。
    pub(super) fn run_continuation(&mut self, cont: Continuation) -> CommandResult {
        match cont {
            Continuation::Open(path) => self.open_path_forced(&path, OpenOptions::default()),
            Continuation::New => self.new_untitled_forced(),
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

    /// プロンプトの入力欄を編集するコマンドを横取りする (オーバーレイの間は文書を編集しない)。
    /// オーバーレイが出ていて、オーバーレイ用でも編集用でもないコマンドは拒否する (モーダル)。
    pub(super) fn overlay_intercept(&mut self, inv: &CommandInvocation) -> Option<CommandResult> {
        self.overlay.as_ref()?;
        let id = inv.id.as_str();
        if id.starts_with("ui.") || id == "app.quit" {
            return None;
        }
        let Some(p) = self.prompt_mut() else {
            return Some(CommandResult::Rejected(RejectReason::NotAvailable));
        };
        let t = text_of(&p.text);
        match id {
            "edit.insert_text" => {
                if let Some(s) = inv.args.str("text") {
                    // 入力欄は 1 行: 改行は取り除く
                    let s: String = s.chars().filter(|c| !matches!(c, '\n' | '\r')).collect();
                    p.text.insert_str(p.cursor, &s);
                    p.cursor += s.len();
                    p.error = None;
                }
            }
            "edit.delete_backward" => {
                let b = prev_grapheme_boundary(&t, ByteOff(p.cursor)).0;
                p.text.replace_range(b..p.cursor, "");
                p.cursor = b;
                p.error = None;
            }
            "edit.delete_forward" => {
                let e = next_grapheme_boundary(&t, ByteOff(p.cursor)).0;
                p.text.replace_range(p.cursor..e, "");
                p.error = None;
            }
            "cursor.move_left" => p.cursor = prev_grapheme_boundary(&t, ByteOff(p.cursor)).0,
            "cursor.move_right" => p.cursor = next_grapheme_boundary(&t, ByteOff(p.cursor)).0,
            "cursor.home" => p.cursor = 0,
            "cursor.end" => p.cursor = p.text.len(),
            _ => return Some(CommandResult::Rejected(RejectReason::NotAvailable)),
        }
        self.mark_dirty_render();
        Some(CommandResult::Done)
    }

    /// 貼り付けを入力欄へ (改行は取り除く)。
    pub(super) fn prompt_paste(&mut self, s: &str) -> bool {
        if self.prompt_mut().is_none() {
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
            PromptKind::Open | PromptKind::SaveAs { .. } if text.is_empty() => {
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
            None => CommandResult::Rejected(RejectReason::NotAvailable),
        }
    }

    /// `ui.cancel`: オーバーレイを閉じる (データは変えない)。出ていなければ何もしない。
    pub fn ui_cancel(&mut self) -> CommandResult {
        if self.take_overlay().is_some() {
            self.mark_dirty_render();
        }
        CommandResult::Done
    }

    /// `ui.list_next` / `ui.list_prev`: 確認のフォーカスを移す (折り返す)。
    pub fn ui_list_move(&mut self, forward: bool) -> CommandResult {
        let Some(Overlay::Confirm(c)) = &mut self.overlay else {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        };
        let n = 3;
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
        if !self.is_dialog() || choice > CHOICE_CANCEL {
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
        if let Some((_, idx)) = ACCELERATORS.iter().find(|(k, _)| *k == c) {
            self.choose(*idx);
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
    let active = "overlay.active != \"\"";
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
    reg_ui!(r, "app.quit", "", cmd_quit);
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
