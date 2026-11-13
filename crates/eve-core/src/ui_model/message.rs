//! メッセージ行 (docs/04 §8.9、docs/07 §17、01 FR-505)。MessageKey と引数を選択中の言語で文字列に解決し、
//! 現在のメッセージとして持つ。Frame の `MessageModel` は `layout` が作る。

use eve_config::Catalog;
use eve_view::{EditorEvent, MessageLevel};

use crate::editor::state::EditorState;
use crate::ui_model::keys;

/// 表示中のメッセージ (解決済みの文言)。
#[derive(Clone, Debug)]
pub(crate) struct ShownMessage {
    pub level: MessageLevel,
    pub text: String,
    pub hint: Option<String>,
    /// 文言キー (同じ理由の拒否を 1 回にまとめる判定に使う、04 §15.2)。
    pub key: String,
    /// 表示した時刻 (Warning は最低 1 秒表示する、04 §8.9)。
    pub shown_at: crate::time::Tick,
}

/// 読み取り専用の理由の文言キー (`ui.read_only_reason.*`、04 §15.2)。`InvalidBytes` は、私用文字と衝突した文書 (b)
/// (`pua_escape == DisabledCollision`) では文言を分ける (03 §8.3.1 の「読み取り専用のときの案内」)。読込が終わって
/// いない (`LoadIncomplete`、予約) は文言が無いので `None`。
pub(crate) fn read_only_reason_key(
    doc: &crate::document::Document,
    r: crate::document::ReadOnlyReason,
) -> Option<&'static str> {
    use crate::document::ReadOnlyReason as R;
    Some(match r {
        R::FileAttribute => keys::UI_READ_ONLY_REASON_FILE_ATTRIBUTE,
        R::UserRequested => keys::UI_READ_ONLY_REASON_USER_REQUESTED,
        R::NoWritePermission => keys::UI_READ_ONLY_REASON_NO_WRITE_PERMISSION,
        R::InvalidBytes
            if doc.format.pua_escape == eve_encoding::PuaEscapeMode::DisabledCollision =>
        {
            keys::UI_READ_ONLY_REASON_INVALID_BYTES_COLLISION
        }
        R::InvalidBytes => keys::UI_READ_ONLY_REASON_INVALID_BYTES,
        R::StatefulEncodingWithInvalidBytes => keys::UI_READ_ONLY_REASON_STATEFUL,
        R::Binary => keys::UI_READ_ONLY_REASON_BINARY,
        R::LargeFileViewer => keys::UI_READ_ONLY_REASON_LARGE_FILE_VIEWER,
        R::LoadIncomplete => return None,
    })
}

/// 履歴に残すメッセージの上限 (04 §8.9、02 §14.1 MR8)。
pub(crate) const HISTORY_MAX: usize = 1_000;

/// 履歴の 1 件 (解決済みの文言)。時刻は起動からの経過 (core は壁時計を持たない)。
#[derive(Clone, Debug)]
pub(crate) struct LoggedMessage {
    pub level: MessageLevel,
    pub text: String,
    pub hint: Option<String>,
    pub at: crate::time::Tick,
}

impl EditorState {
    /// 選択中の言語の文言カタログ。
    pub fn catalog(&self) -> &'static Catalog {
        Catalog::builtin(self.settings.locale)
    }

    /// `{key:<command_id>}` のキー表記を引く (解決済みキーマップの第一割当)。`Editor` がキーマップの変更ごとに更新する。
    pub(crate) fn key_label(&self, command: &str) -> Option<String> {
        self.keys.key_labels.get(command).cloned()
    }

    /// メッセージ・ヒントに出すキーの表記 (`labels`、コマンド ID → 表記) と割当の列 (`seqs`) を差し替える。
    pub fn set_key_bindings(
        &mut self,
        labels: std::collections::BTreeMap<String, String>,
        seqs: std::collections::BTreeMap<String, Vec<eve_view::KeyStroke>>,
    ) {
        self.keys.key_labels = labels;
        self.keys.key_seqs = seqs;
    }

    /// すべての割当を更新する (ヘルプのキー一覧用)。
    pub fn set_all_key_bindings(
        &mut self,
        all: std::collections::BTreeMap<String, Vec<(Vec<eve_view::KeyStroke>, bool)>>,
    ) {
        self.keys.key_all = all;
    }

    pub(crate) fn key_sequence(&self, command: &str) -> Option<Vec<eve_view::KeyStroke>> {
        self.keys.key_seqs.get(command).cloned()
    }

    /// `MessageKey` を解決して表示する。`msg.*` は `.text` と `.hint`、それ以外は 1 つの文。
    /// 外向きには `EditorEvent::MessageShown` (キーだけ) も積む。
    /// メッセージ `key` の本文だけを解決する (確認ダイアログの本文に入れる。`hint` は含めない)。
    pub(crate) fn message_text(&self, key: &str, args: &[(&str, &str)]) -> String {
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
    /// v0.4 の M23 から、理由ごとにメッセージを分ける (03 §8.3.1 の「読み取り専用のときの案内」、04 §15.2): 解除できない
    /// 理由があればそれ (衝突の文書 (b)・ISO-2022-JP・バイナリは専用のメッセージ。ほかは `msg.edit.read_only` に
    /// その理由)、無ければ `InvalidBytes` (切替で、確認の後に編集できる)、どちらでもなければ `msg.edit.read_only`。
    pub(crate) fn notify_read_only(&mut self) {
        use crate::document::ReadOnlyReason as R;
        let doc = self.active_document().and_then(|d| self.document(d));
        // 読込中 (`Loading` / 先行表示の `Partial`) は理由ではなく状態: 読込を待つか取り消すかを案内する専用の
        // メッセージ (03 §9.2.1 の「できない操作」、04 §15.2 の `msg.edit.read_only_loading`)
        if doc.is_some_and(|d| d.load_state != crate::document::LoadState::Ready) {
            self.show_message(
                eve_view::MessageLevel::Warn,
                keys::MSG_EDIT_READ_ONLY_LOADING,
                &[],
            );
            return;
        }
        let blocking = doc.and_then(crate::document::Document::blocking_read_only_reason);
        let key = match blocking {
            // 解除できない `InvalidBytes`: 衝突の文書 (b) は専用のメッセージ。由来の文字コードの無い文書 (文字コードの違う
            // 復旧の後など) は、切替の案内をしない一般のメッセージ (`{reason}` は「不正なバイトを含むファイル」)
            Some(R::InvalidBytes)
                if doc.is_some_and(|d| {
                    d.format.pua_escape == eve_encoding::PuaEscapeMode::DisabledCollision
                }) =>
            {
                keys::MSG_EDIT_READ_ONLY_PUA_COLLISION
            }
            Some(R::StatefulEncodingWithInvalidBytes) => keys::MSG_EDIT_READ_ONLY_STATEFUL,
            Some(R::Binary) => keys::MSG_EDIT_READ_ONLY_BINARY,
            Some(_) => keys::MSG_EDIT_READ_ONLY,
            None if doc.is_some_and(|d| d.read_only.contains(R::InvalidBytes)) => {
                keys::MSG_EDIT_READ_ONLY_INVALID_BYTES
            }
            None => keys::MSG_EDIT_READ_ONLY,
        };
        // `{reason}` は解除できない理由、無ければ優先順で先頭の理由
        let reason = doc
            .and_then(|d| {
                let r = d
                    .blocking_read_only_reason()
                    .or_else(|| d.read_only.primary())?;
                read_only_reason_key(d, r)
            })
            .map(|k| self.catalog().render(k, &[], &|_| None))
            .unwrap_or_default();
        self.show_message(eve_view::MessageLevel::Warn, key, &[("reason", &reason)]);
    }

    /// カタログのキー `key` の文言を `args` で埋めてメッセージに出し、履歴に積む。`<key>.text` / `<key>.hint` があればその 2 つを使う。
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
        if self.ui.message.is_some() {
            self.ui.unread = self.ui.unread.saturating_add(1);
        }
        if self.ui.message_log.len() >= HISTORY_MAX {
            self.ui.message_log.pop_front();
        }
        self.ui.message_log.push_back(LoggedMessage {
            level,
            text: text.clone(),
            hint: hint.clone(),
            at: self.now,
        });
        self.ui.message = Some(ShownMessage {
            level,
            text,
            hint,
            key: key.to_owned(),
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
    /// `Esc` で警告・エラーを消したら `true` (その `Esc` はメッセージを消すことだけを行う、04 §7.5)。
    pub(crate) fn dismiss_message_on_key(&mut self, is_esc: bool) -> bool {
        let now = self.now;
        let dismiss = match &self.ui.message {
            Some(m) => match m.level {
                MessageLevel::Info => true,
                MessageLevel::Warn => {
                    is_esc || now.saturating_sub(m.shown_at) >= crate::time::Millis(1000)
                }
                _ => is_esc,
            },
            None => false,
        };
        let warn_or_error = self
            .ui
            .message
            .as_ref()
            .is_some_and(|m| m.level != MessageLevel::Info);
        if dismiss {
            self.ui.message = None;
            self.ui.unread = 0;
            self.mark_dirty_render();
        }
        dismiss && is_esc && warn_or_error
    }

    /// 表示中のメッセージの文言キー (同じ理由の拒否を 1 回にまとめる)。
    pub(crate) fn shown_message_key(&self) -> Option<&str> {
        self.ui.message.as_ref().map(|m| m.key.as_str())
    }

    /// 編集などのコマンドを実行したら Error を消す (カーソル移動では消えない)。
    pub(crate) fn dismiss_error_on_command(&mut self) {
        if self
            .ui
            .message
            .as_ref()
            .is_some_and(|m| m.level == MessageLevel::Error)
        {
            self.ui.message = None;
            self.ui.unread = 0;
            self.mark_dirty_render();
        }
    }

    pub(crate) fn unread_messages(&self) -> u16 {
        self.ui.unread
    }

    pub(crate) fn current_message(&self) -> Option<&ShownMessage> {
        self.ui.message.as_ref()
    }
}
