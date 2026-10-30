//! メッセージ行 (docs/04 §8.9、docs/07 §17、01 FR-505)。MessageKey と引数を選択中の言語で文字列に解決し、
//! 現在のメッセージとして持つ。Frame の `MessageModel` は `layout` がこれから作る。

use eve_config::Catalog;
use eve_view::{EditorEvent, MessageLevel};

use super::state::EditorState;

/// 表示中のメッセージ (解決済みの文言)。
#[derive(Clone, Debug)]
pub(super) struct ShownMessage {
    pub level: MessageLevel,
    pub text: String,
    pub hint: Option<String>,
    /// 表示した時刻 (Warning は最低 1 秒表示する、04 §8.9)。
    pub shown_at: crate::time::Tick,
}

/// 履歴に残すメッセージの上限 (04 §8.9、02 §14.1 MR8)。
pub(super) const HISTORY_MAX: usize = 1_000;

/// 履歴の 1 件 (解決済みの文言)。時刻は起動からの経過 (core は壁時計を持たない)。
#[derive(Clone, Debug)]
pub(super) struct LoggedMessage {
    pub level: MessageLevel,
    pub text: String,
    pub hint: Option<String>,
    pub at: crate::time::Tick,
}

impl EditorState {
    /// 選択中の言語の文言カタログ。
    pub fn catalog(&self) -> &'static Catalog {
        Catalog::builtin(self.locale)
    }

    /// `{key:<command_id>}` のキー表記を引く (解決済みキーマップの第一割当)。`Editor` がキーマップの変更ごとに更新する。
    pub(super) fn key_label(&self, command: &str) -> Option<String> {
        self.key_labels.get(command).cloned()
    }

    pub fn set_key_bindings(
        &mut self,
        labels: std::collections::BTreeMap<String, String>,
        seqs: std::collections::BTreeMap<String, Vec<eve_view::KeyStroke>>,
    ) {
        self.key_labels = labels;
        self.key_seqs = seqs;
    }

    /// すべての割当を更新する (ヘルプのキー一覧用)。
    pub fn set_all_key_bindings(
        &mut self,
        all: std::collections::BTreeMap<String, Vec<(Vec<eve_view::KeyStroke>, bool)>>,
    ) {
        self.key_all = all;
    }

    pub(super) fn key_sequence(&self, command: &str) -> Option<Vec<eve_view::KeyStroke>> {
        self.key_seqs.get(command).cloned()
    }

    /// MessageKey を解決して表示する。`msg.*` は `.text` と `.hint`、それ以外は 1 つの文。
    /// 外向きには `EditorEvent::MessageShown` (キーだけ) も積む。
    /// メッセージ `key` の本文だけを解決する (確認ダイアログの本文に入れる。`hint` は含めない)。
    pub(super) fn message_text(&self, key: &str, args: &[(&str, &str)]) -> String {
        let cat = self.catalog();
        let label = |c: &str| self.key_label(c);
        let text_key = format!("{key}.text");
        if cat.contains(&text_key) {
            cat.render(&text_key, args, &label)
        } else {
            cat.render(key, args, &label)
        }
    }

    /// 読み取り専用の文書を編集しようとした: 理由つきで拒否したことを示す (`msg.edit.read_only`、FR-116 AC-1)。
    pub(super) fn notify_read_only(&mut self) {
        use crate::document::ReadOnlyReason as R;
        let reason = self
            .active_document()
            .and_then(|d| self.document(d))
            .and_then(|d| d.read_only.primary())
            .and_then(|r| {
                Some(match r {
                    R::FileAttribute => "file_attribute",
                    R::UserRequested => "user_requested",
                    R::NoWritePermission => "no_write_permission",
                    R::InvalidBytes => "invalid_bytes",
                    R::StatefulEncodingWithInvalidBytes => "stateful_encoding_with_invalid_bytes",
                    R::Binary => "binary",
                    R::LargeFileViewer => "large_file_viewer",
                    R::LongLine => "long_line",
                    // 読込が終わっていない (v0.3 の Partial)。理由の文言はまだ無い
                    R::LoadIncomplete => return None,
                })
            })
            .map(|k| {
                self.catalog()
                    .render(&format!("ui.read_only_reason.{k}"), &[], &|_| None)
            })
            .unwrap_or_default();
        self.show_message(
            eve_view::MessageLevel::Warn,
            "msg.edit.read_only",
            &[("reason", &reason)],
        );
    }

    pub fn show_message(&mut self, level: MessageLevel, key: &str, args: &[(&str, &str)]) {
        let cat = self.catalog();
        let label = |c: &str| self.key_label(c);
        let text_key = format!("{key}.text");
        let (text, hint) = if cat.contains(&text_key) {
            let hint_key = format!("{key}.hint");
            (
                cat.render(&text_key, args, &label),
                cat.contains(&hint_key)
                    .then(|| cat.render(&hint_key, args, &label)),
            )
        } else {
            (cat.render(key, args, &label), None)
        };
        // 表示中のメッセージが読まれる前に置き換わったら、未読として数える (履歴は `ui.message_history` で見る)
        if self.message.is_some() {
            self.unread = self.unread.saturating_add(1);
        }
        if self.message_log.len() >= HISTORY_MAX {
            self.message_log.pop_front();
        }
        self.message_log.push_back(LoggedMessage {
            level,
            text: text.clone(),
            hint: hint.clone(),
            at: self.now,
        });
        self.message = Some(ShownMessage {
            level,
            text,
            hint,
            shown_at: self.now,
        });
        self.push_event(EditorEvent::MessageShown {
            level,
            key: key.to_owned(),
        });
        self.mark_dirty_render();
    }

    /// キー入力でメッセージを消す (04 §8.9): Info は次のキー入力で、Warning は `Esc` または (最低 1 秒表示した後の)
    /// 次のキー入力で、Error は `Esc` で消える。`Esc` の優先順位 (オーバーレイ・チョード・読込の取消が先) は呼び出し側が決める。
    pub(super) fn dismiss_message_on_key(&mut self, is_esc: bool) {
        let now = self.now;
        let dismiss = match &self.message {
            Some(m) => match m.level {
                MessageLevel::Info => true,
                MessageLevel::Warn => {
                    is_esc || now.saturating_sub(m.shown_at) >= crate::time::Millis(1000)
                }
                _ => is_esc,
            },
            None => false,
        };
        if dismiss {
            self.message = None;
            self.unread = 0;
            self.mark_dirty_render();
        }
    }

    /// 編集などのコマンドを実行したら Error を消す (カーソル移動では消えない)。
    pub(super) fn dismiss_error_on_command(&mut self) {
        if self
            .message
            .as_ref()
            .is_some_and(|m| m.level == MessageLevel::Error)
        {
            self.message = None;
            self.unread = 0;
            self.mark_dirty_render();
        }
    }

    pub(super) fn unread_messages(&self) -> u16 {
        self.unread
    }

    pub(super) fn current_message(&self) -> Option<&ShownMessage> {
        self.message.as_ref()
    }
}
