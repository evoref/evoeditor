//! 入力プロンプト (開く・名前を付けて保存・行へ移動など、docs/04 §8)。開く・確定・確定の後の失敗の扱い。

use crate::command::{ArgValue, Args, CommandCtx, CommandResult, RejectReason};
use crate::io::io_jobs::OpenOptions;

use crate::editor::state::EditorState;

use super::{Continuation, Overlay};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PromptKind {
    Open,
    SaveAs {
        then: Option<Continuation>,
    },
    GotoLine,
    /// 開くときの確認で「文字コードを指定して開く」を選んだ後の文字コードの入力 (03 §9.2)。
    Encoding {
        ask: crate::io::open::OpenAsk,
    },
    /// `file.reopen_with_encoding` に引数が無いときの文字コードの入力 (03 §8.8)。
    ReopenEncoding,
    /// `file.set_encoding` に引数が無いときの文字コードの入力 (03 §8.8)。
    SetEncoding {
        bom: Option<bool>,
    },
    /// `file.set_encoding` で、パスのない・読み取り専用の文書の名前の入力 (名前を付けて保存へ新しい書式を渡す)。
    SaveAsFormat {
        fmt: crate::io::pending::NewFormat,
    },
}

impl PromptKind {
    /// `ui.prompt.<name>` の名前。
    pub(crate) fn name(&self) -> &'static str {
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
pub(crate) struct PromptState {
    pub kind: PromptKind,
    pub text: String,
    /// `text` 内のカーソル (バイト。書記素境界)。
    pub cursor: usize,
    /// 欄内全選択 (`select.all`) の最中。次の編集・移動で解ける。
    pub all_selected: bool,
    /// 検証エラーの `MessageKey` の末尾 (`ui.prompt.<name>.error.<kind>`)。
    pub error: Option<&'static str>,
}

/// 行へ移動の入力 (`12` / `12:5` / `$` / `$:3`) を `cursor.goto_line` の引数にする。
pub(crate) fn parse_goto(s: &str) -> Option<Args> {
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

impl EditorState {
    pub(crate) fn open_prompt(&mut self, kind: PromptKind, initial: String) {
        let cursor = initial.len();
        self.set_overlay(Some(Overlay::Prompt(PromptState {
            kind,
            text: initial,
            cursor,
            all_selected: false,
            error: None,
        })));
        self.mark_dirty_render();
    }

    pub(super) fn prompt_mut(&mut self) -> Option<&mut PromptState> {
        match &mut self.ui.overlay {
            Some(Overlay::Prompt(p)) => Some(p),
            _ => None,
        }
    }

    pub(super) fn submit_prompt(&mut self) -> CommandResult {
        let Some(Overlay::Prompt(p)) = self.ui.overlay.clone() else {
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
                    crate::view::motion::goto_line_command(&mut ctx)
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
        if failed && self.ui.overlay.is_none() {
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
