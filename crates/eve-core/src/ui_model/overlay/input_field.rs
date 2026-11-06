//! 1 行の入力欄の編集 (プロンプト・パレット・一覧の絞り込みで共通、07 §7.1 の `prompt` コンテキスト)。
//!
//! オーバーレイが出ている間の、入力欄の編集と貼り付けの横取り (`overlay_intercept`)。

use eve_text::{ByteOff, Text, next_grapheme_boundary, prev_grapheme_boundary};

use crate::command::ids;
use crate::command::{Args, CommandInvocation, CommandResult, RejectReason};

use crate::editor::state::EditorState;

use super::Overlay;

fn text_of(s: &str) -> Text {
    Text::from(s)
}

impl EditorState {
    /// 貼り付けを受けられる入力欄があるか。
    pub(crate) fn input_mut_available(&mut self) -> bool {
        self.input_mut().is_some()
    }

    /// 入力欄 (プロンプトとパレットで共通の編集)。
    fn input_mut(&mut self) -> Option<(&mut String, &mut usize)> {
        match &mut self.ui.overlay {
            Some(Overlay::Prompt(p)) => Some((&mut p.text, &mut p.cursor)),
            Some(Overlay::Palette(p)) => Some((&mut p.text, &mut p.cursor)),
            None => {
                let b = self.find.search_bar.as_mut()?;
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
    pub(crate) fn overlay_intercept(&mut self, inv: &CommandInvocation) -> Option<CommandResult> {
        if self.ui.overlay.is_none() && self.find.search_bar.is_none() {
            return None;
        }
        let id = inv.id.as_str();
        if id.starts_with("ui.") || id == ids::APP_QUIT {
            return None;
        }
        // バッファ一覧の中の `buffer.close` は、選んでいる文書を閉じる (07 §11.6 の `list` コンテキスト)
        if id == ids::BUFFER_CLOSE && self.overlay_name() == "buffer_list" {
            return None;
        }
        // 入力欄への貼り付け: システムクリップボードを要求し、応答は入力欄の最初の行を入れる (`on_clipboard_content`)
        if id == ids::EDIT_PASTE {
            return Some(if self.input_mut().is_some() {
                self.request_paste()
            } else {
                CommandResult::Rejected(RejectReason::NotAvailable)
            });
        }
        // 検索バーはモーダルではない: 入力欄の編集だけを受け、それ以外のコマンドは通常どおり実行する
        if self.ui.overlay.is_none()
            && !matches!(
                id,
                ids::EDIT_INSERT_TEXT
                    | ids::EDIT_DELETE_BACKWARD
                    | ids::EDIT_DELETE_FORWARD
                    | ids::CURSOR_MOVE_LEFT
                    | ids::CURSOR_MOVE_RIGHT
                    | ids::CURSOR_HOME
                    | ids::CURSOR_END
            )
        {
            return None;
        }
        let is_prompt = matches!(self.ui.overlay, Some(Overlay::Prompt(_)));
        let Some((text, cursor)) = self.input_mut() else {
            return Some(CommandResult::Rejected(RejectReason::NotAvailable));
        };
        let t = text_of(text);
        let mut edited = false;
        match id {
            ids::EDIT_INSERT_TEXT => {
                if let Some(s) = inv.args.str("text") {
                    // 入力欄は 1 行: 改行は取り除く
                    let s: String = s.chars().filter(|c| !matches!(c, '\n' | '\r')).collect();
                    text.insert_str(*cursor, &s);
                    *cursor += s.len();
                    edited = true;
                }
            }
            ids::EDIT_DELETE_BACKWARD => {
                let b = prev_grapheme_boundary(&t, ByteOff(*cursor)).0;
                text.replace_range(b..*cursor, "");
                *cursor = b;
                edited = true;
            }
            ids::EDIT_DELETE_FORWARD => {
                let e = next_grapheme_boundary(&t, ByteOff(*cursor)).0;
                text.replace_range(*cursor..e, "");
                edited = true;
            }
            ids::CURSOR_MOVE_LEFT => *cursor = prev_grapheme_boundary(&t, ByteOff(*cursor)).0,
            ids::CURSOR_MOVE_RIGHT => *cursor = next_grapheme_boundary(&t, ByteOff(*cursor)).0,
            ids::CURSOR_HOME => *cursor = 0,
            ids::CURSOR_END => *cursor = text.len(),
            _ => return Some(CommandResult::Rejected(RejectReason::NotAvailable)),
        }
        if edited {
            if is_prompt {
                if let Some(p) = self.prompt_mut() {
                    p.error = None;
                }
            } else if self.ui.overlay.is_none() {
                self.bar_edited();
            } else {
                self.palette_reset_selection();
            }
        }
        self.mark_dirty_render();
        Some(CommandResult::Done)
    }

    /// 貼り付けを入力欄へ (改行は取り除く)。
    pub(crate) fn prompt_paste(&mut self, s: &str) -> bool {
        if self.input_mut().is_none() {
            return false;
        }
        let inv = CommandInvocation::new(
            ids::EDIT_INSERT_TEXT,
            crate::command::InvocationSource::Keymap,
        )
        .with_args(Args::new().with_str("text", s));
        self.overlay_intercept(&inv);
        true
    }
}
