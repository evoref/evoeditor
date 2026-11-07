//! 1 行の入力欄の編集 (プロンプト・パレット・一覧の絞り込みで共通、07 §7.1 の `prompt` コンテキスト)。
//!
//! オーバーレイが出ている間の、入力欄の編集と貼り付けの横取り (`overlay_intercept`)。

use std::ops::Range;

use eve_text::{
    ByteOff, Text, next_grapheme_boundary, next_word_stop, prev_grapheme_boundary, prev_word_stop,
};

use crate::command::ids;
use crate::command::{Args, CommandInvocation, CommandResult, RejectReason};

use crate::editor::state::EditorState;

use super::Overlay;

fn text_of(s: &str) -> Text {
    Text::from(s)
}

/// 入力欄の表示モデルの選択 (欄内の書記素番号)。欄内全選択の最中で、欄が空でなければ欄全体。
pub(crate) fn field_selection(all_selected: bool, len: usize) -> Option<Range<usize>> {
    (all_selected && len > 0).then_some(0..len)
}

/// 編集中の 1 行の入力欄 (テキスト・カーソル・欄内全選択)。
struct FieldMut<'a> {
    text: &'a mut String,
    cursor: &'a mut usize,
    all_selected: &'a mut bool,
}

/// 入力欄が受けるコマンド (07 §7.1 の `prompt` の「`edit.*` / `cursor.*` / `select.all` の欄内版」)。
fn is_field_command(id: &str) -> bool {
    matches!(
        id,
        ids::EDIT_INSERT_TEXT
            | ids::EDIT_DELETE_BACKWARD
            | ids::EDIT_DELETE_FORWARD
            | ids::EDIT_DELETE_WORD_BACKWARD
            | ids::EDIT_DELETE_WORD_FORWARD
            | ids::CURSOR_MOVE_LEFT
            | ids::CURSOR_MOVE_RIGHT
            | ids::CURSOR_MOVE_WORD_LEFT
            | ids::CURSOR_MOVE_WORD_RIGHT
            | ids::CURSOR_HOME
            | ids::CURSOR_END
            | ids::SELECT_ALL
    )
}

impl EditorState {
    /// 貼り付けを受けられる入力欄があるか。
    pub(crate) fn input_mut_available(&mut self) -> bool {
        self.input_mut().is_some()
    }

    /// 入力欄 (プロンプトとパレットで共通の編集)。
    fn input_mut(&mut self) -> Option<FieldMut<'_>> {
        match &mut self.ui.overlay {
            Some(Overlay::Prompt(p)) => Some(FieldMut {
                text: &mut p.text,
                cursor: &mut p.cursor,
                all_selected: &mut p.all_selected,
            }),
            Some(Overlay::Palette(p)) => Some(FieldMut {
                text: &mut p.text,
                cursor: &mut p.cursor,
                all_selected: &mut p.all_selected,
            }),
            None => {
                let b = self.find.search_bar.as_mut()?;
                match b.focus {
                    eve_view::BarFocus::Search => Some(FieldMut {
                        text: &mut b.text,
                        cursor: &mut b.cursor,
                        all_selected: &mut b.all_selected,
                    }),
                    eve_view::BarFocus::Replace => match &mut b.replace {
                        Some((t, c)) => Some(FieldMut {
                            text: t,
                            cursor: c,
                            all_selected: &mut b.all_selected,
                        }),
                        None => None,
                    },
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
        if self.ui.overlay.is_none() && !is_field_command(id) {
            return None;
        }
        let is_prompt = matches!(self.ui.overlay, Some(Overlay::Prompt(_)));
        let Some(f) = self.input_mut() else {
            return Some(CommandResult::Rejected(RejectReason::NotAvailable));
        };
        let FieldMut {
            text,
            cursor,
            all_selected,
        } = f;
        let t = text_of(text);
        // 欄内全選択は次の編集・移動で解ける。選択中の削除・入力は欄全体を置き換え、左右の移動は選択の端へ寄せる
        let all = std::mem::take(all_selected) && !text.is_empty();
        let mut edited = false;
        match id {
            ids::SELECT_ALL => {
                *all_selected = true;
                *cursor = text.len();
            }
            ids::EDIT_INSERT_TEXT => {
                if let Some(s) = inv.args.str("text") {
                    if all {
                        text.clear();
                        *cursor = 0;
                    }
                    // 入力欄は 1 行: 改行は取り除く
                    let s: String = s.chars().filter(|c| !matches!(c, '\n' | '\r')).collect();
                    text.insert_str(*cursor, &s);
                    *cursor += s.len();
                    edited = true;
                } else {
                    *all_selected = all;
                }
            }
            ids::EDIT_DELETE_BACKWARD
            | ids::EDIT_DELETE_FORWARD
            | ids::EDIT_DELETE_WORD_BACKWARD
            | ids::EDIT_DELETE_WORD_FORWARD
                if all =>
            {
                text.clear();
                *cursor = 0;
                edited = true;
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
            // 単語の範囲は編集面と同じ (03 §5.3 / §13.3)
            ids::EDIT_DELETE_WORD_BACKWARD => {
                let b = prev_word_stop(&t, ByteOff(*cursor)).0;
                text.replace_range(b..*cursor, "");
                *cursor = b;
                edited = true;
            }
            ids::EDIT_DELETE_WORD_FORWARD => {
                let e = next_word_stop(&t, ByteOff(*cursor)).0;
                text.replace_range(*cursor..e, "");
                edited = true;
            }
            ids::CURSOR_MOVE_LEFT | ids::CURSOR_MOVE_WORD_LEFT if all => *cursor = 0,
            ids::CURSOR_MOVE_RIGHT | ids::CURSOR_MOVE_WORD_RIGHT if all => *cursor = text.len(),
            ids::CURSOR_MOVE_LEFT => *cursor = prev_grapheme_boundary(&t, ByteOff(*cursor)).0,
            ids::CURSOR_MOVE_RIGHT => *cursor = next_grapheme_boundary(&t, ByteOff(*cursor)).0,
            ids::CURSOR_MOVE_WORD_LEFT => *cursor = prev_word_stop(&t, ByteOff(*cursor)).0,
            ids::CURSOR_MOVE_WORD_RIGHT => *cursor = next_word_stop(&t, ByteOff(*cursor)).0,
            ids::CURSOR_HOME => *cursor = 0,
            ids::CURSOR_END => *cursor = text.len(),
            _ => {
                *all_selected = all;
                return Some(CommandResult::Rejected(RejectReason::NotAvailable));
            }
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
