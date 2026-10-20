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
}

impl EditorState {
    /// 文言カタログ (v0.1 は同梱の `ja` だけ)。
    pub fn catalog(&self) -> &'static Catalog {
        Catalog::builtin_ja()
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

    pub(super) fn key_sequence(&self, command: &str) -> Option<Vec<eve_view::KeyStroke>> {
        self.key_seqs.get(command).cloned()
    }

    /// MessageKey を解決して表示する。`msg.*` は `.text` と `.hint`、それ以外は 1 つの文。
    /// 外向きには `EditorEvent::MessageShown` (キーだけ) も積む。
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
        self.message = Some(ShownMessage { level, text, hint });
        self.unread = self.unread.saturating_add(1);
        self.push_event(EditorEvent::MessageShown {
            level,
            key: key.to_owned(),
        });
        self.mark_dirty_render();
    }

    /// キー入力でメッセージを消す: Info は次のキー入力で、Warning / Error は `Esc` で消える (04 §8.9)。
    pub(super) fn dismiss_message_on_key(&mut self, is_esc: bool) {
        let dismiss = match &self.message {
            Some(m) => m.level == MessageLevel::Info || is_esc,
            None => false,
        };
        if dismiss {
            self.message = None;
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
