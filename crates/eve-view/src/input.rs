//! 入力の抽象 (docs/02-architecture.md §7.1)。キー押下 (`Key`) と生成された文字 (`Text`) を別のイベントにする。
//! 端末の癖 (Ctrl+H ⇔ Backspace など) は frontend が吸収し、core は正規化済みのキーだけを見る。

use crate::caps::Capabilities;
use crate::ids::ClipboardToken;

/// frontend から core に届く入力。宛先の Window は `Editor::handle_input` の引数で渡す。
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum InputEvent {
    /// 修飾付きキー (ショートカット判定用)。
    Key(KeyEvent),
    /// 確定済みのテキスト入力 (IME の確定、通常の文字入力)。入力スレッドが連続分を集約する (上限 64 KiB)。
    Text(String),
    /// ブラケットペースト。本文の制御文字はコマンドとして解釈せず、文字として扱う。
    Paste(String),
    /// Window の編集面のサイズ (セル単位)。
    Resize { cols: u16, rows: u16 },
    /// フォーカスの有無。
    Focus(bool),
    /// `Effect::RequestClipboard` の応答。`None` は取得できなかった。
    ClipboardContent {
        token: ClipboardToken,
        text: Option<String>,
    },
    /// 実行中に判明・変化した能力 (拡張キーの検出など)。
    CapabilitiesChanged(Capabilities),
}

/// キー 1 つの押下・リピート・解放。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub mods: Modifiers,
    pub kind: KeyKind,
}

impl KeyEvent {
    /// 押下のイベント。
    pub fn press(code: KeyCode, mods: Modifiers) -> KeyEvent {
        KeyEvent {
            code,
            mods,
            kind: KeyKind::Press,
        }
    }
}

/// キーの種類。`Release` は frontend が捨て、core には渡さない (型には残す)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum KeyKind {
    Press,
    Repeat,
    Release,
}

/// 端末・OS から届いた論理キー (キーボードのレイアウトに依存しない物理キーではない)。
/// 文字キーは小文字に正規化して `Char` に入れ、大文字は `Shift` との組で表す (07 §7.2)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum KeyCode {
    Char(char),
    /// `f1`〜`f24`。
    F(u8),
    Enter,
    Tab,
    Esc,
    Backspace,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    Up,
    Down,
    Left,
    Right,
    Menu,
    CapsLock,
    PrintScreen,
}

/// 修飾キーの集合。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers(u8);

impl Modifiers {
    pub const NONE: Modifiers = Modifiers(0);
    pub const CTRL: Modifiers = Modifiers(1);
    pub const ALT: Modifiers = Modifiers(2);
    pub const SHIFT: Modifiers = Modifiers(4);
    pub const SUPER: Modifiers = Modifiers(8);

    /// `other` のすべての修飾を含むか。
    pub fn contains(self, other: Modifiers) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl std::ops::BitOr for Modifiers {
    type Output = Modifiers;

    fn bitor(self, rhs: Modifiers) -> Modifiers {
        Modifiers(self.0 | rhs.0)
    }
}

/// キー 1 打 (修飾 + キー)。チョードの 1 ストローク、`Capabilities::intercepted_keys` などに使う。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyStroke {
    pub mods: Modifiers,
    pub key: KeyCode,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 修飾の集合() {
        let m = Modifiers::CTRL | Modifiers::SHIFT;
        assert!(m.contains(Modifiers::CTRL));
        assert!(m.contains(Modifiers::SHIFT));
        assert!(!m.contains(Modifiers::ALT));
        assert!(m.contains(Modifiers::NONE));
        assert!(Modifiers::NONE.is_empty());
        assert!(!m.is_empty());
    }

    #[test]
    fn キーの押下() {
        let ev = KeyEvent::press(KeyCode::Char('s'), Modifiers::CTRL);
        assert_eq!(ev.kind, KeyKind::Press);
        assert_eq!(ev.code, KeyCode::Char('s'));
        assert_eq!(InputEvent::Key(ev), InputEvent::Key(ev));
    }
}
