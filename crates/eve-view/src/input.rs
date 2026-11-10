//! 入力の抽象 (docs/02-architecture.md §7.1)。キー押下 (`Key`) と生成された文字 (`Text`) を別のイベントにする。
//! 端末の癖 (Ctrl+H ⇔ Backspace など) は frontend が吸収し、core は正規化済みのキーだけを見る。

use crate::caps::Capabilities;
use crate::ids::{ClipboardToken, ViewId};

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
    /// マウス (docs/03 §5.8.1)。編集面の外 (クローム・オーバーレイ) は frontend が先に判定し、ここへは渡さない。
    Mouse(MouseEvent),
    /// Window の編集面のサイズ (セル単位)。端末の全体からクローム (ステータスバー・ヒントバー) の行を除いた大きさで、
    /// 検索バーの行は含む (core が引く。`Frame.size` は引いた後の大きさ)。core の扱いは 1 つ (編集面の大きさを置き換える) だが、
    /// frontend が送る契機は 2 つある。
    /// 1. 端末の大きさが変わった (入力スレッドが送る)。
    /// 2. 描こうとした `Frame` の大きさが今の編集面と合わない (ヒントバーの表示の切り替えでクロームの行数が変わった直後など)。
    ///    frontend は描かずに今の大きさを送り直し、次の `Frame` で描く (04 §3.6)。大きさが前と同じなら core は何もしない。
    ///
    /// 起動時の最初の大きさは、このイベントではなく `Frontend::surface_size` から Host が渡す。
    Resize {
        /// 編集面の列数 (セル)。
        cols: u16,
        /// 編集面の行数 (セル)。
        rows: u16,
    },
    /// 端末のフォーカスの有無 (真 = 得た、偽 = 失った)。
    Focus(bool),
    /// `Effect::RequestClipboard` の応答。`None` は取得できなかった。
    ClipboardContent {
        /// 要求 (`Effect::RequestClipboard`) の `token` と同じ値。古い要求への応答は core が捨てる。
        token: ClipboardToken,
        /// 読んだ文字列 (改行は正規化していない)。
        text: Option<String>,
    },
    /// 実行中に判明・変化した能力 (拡張キーの検出など)。
    CapabilitiesChanged(Capabilities),
}

/// マウスの操作 1 つ (docs/03 §5.8.1、02 §7.3)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseEvent {
    /// 操作の種類 (押下・ドラッグ・解放・ホイール)。
    pub kind: MouseKind,
    /// 操作の時に押されていた修飾キー。
    pub mods: Modifiers,
    /// 操作の位置 (編集面のセル)。
    pub at: MouseAt,
}

/// マウスの操作の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MouseKind {
    /// ボタンを押した。
    Down(MouseButton),
    /// ボタンを押したままの移動。
    Drag(MouseButton),
    /// ボタンを放した。
    Up(MouseButton),
    /// ホイールを上 (文書の先頭の向き) へ回した。
    WheelUp,
    /// ホイールを下 (文書の末尾の向き) へ回した。
    WheelDown,
}

/// マウスのボタン。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MouseButton {
    /// 左ボタン。
    Left,
    /// 中ボタン (ホイールの押し込み)。
    Middle,
    /// 右ボタン。
    Right,
}

/// 編集面 (`PaneFrame.rect`) の左上を原点とするセル。ボタンを押している間は面の外 (負・はみ出し) もそのまま渡す。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseAt {
    /// 操作の対象の編集面の View。
    pub view: ViewId,
    /// 編集面の中の表示行 (0 始まり。面の上は負)。
    pub row: i32,
    /// 編集面の中の表示列 (セル、0 始まり。ガターを含む。面の左は負)。
    pub col: i32,
}

/// キー 1 つの押下・リピート・解放。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyEvent {
    /// 押されたキー (正規化済み)。
    pub code: KeyCode,
    /// 押されていた修飾キー。
    pub mods: Modifiers,
    /// 押下・リピート・解放の別。
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
    /// 押下。
    Press,
    /// 押し続けによる自動の繰り返し。
    Repeat,
    /// 解放 (core には届かない)。
    Release,
}

/// 端末・OS から届いた論理キー (キーボードのレイアウトに依存しない物理キーではない)。
/// 文字キーは小文字に正規化して `Char` に入れ、大文字は `Shift` との組で表す (07 §7.2)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum KeyCode {
    /// 文字キー (英字は小文字。空白は `' '`)。
    Char(char),
    /// `f1`〜`f24` (値は 1〜24)。
    F(u8),
    /// Enter。
    Enter,
    /// Tab (`Shift+Tab` は `Shift` との組)。
    Tab,
    /// Esc。
    Esc,
    /// Backspace。
    Backspace,
    /// Delete。
    Delete,
    /// Insert。
    Insert,
    /// Home。
    Home,
    /// End。
    End,
    /// Page Up。
    PageUp,
    /// Page Down。
    PageDown,
    /// 上矢印。
    Up,
    /// 下矢印。
    Down,
    /// 左矢印。
    Left,
    /// 右矢印。
    Right,
    /// アプリケーション (メニュー) キー。
    Menu,
    /// Caps Lock。
    CapsLock,
    /// Print Screen。
    PrintScreen,
}

/// 修飾キーの集合 (ビット集合)。組は `|` で作る (例 `Modifiers::CTRL | Modifiers::SHIFT`)。
/// 文字の属性 (`StyleMods`) とは別の型。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers(u8);

impl Modifiers {
    /// 修飾なし (空の集合)。
    pub const NONE: Modifiers = Modifiers(0);
    /// Ctrl。
    pub const CTRL: Modifiers = Modifiers(1);
    /// Alt (端末の ESC 前置も frontend がこれに正規化する)。
    pub const ALT: Modifiers = Modifiers(2);
    /// Shift。
    pub const SHIFT: Modifiers = Modifiers(4);
    /// Super (Windows キー・Command キー)。
    pub const SUPER: Modifiers = Modifiers(8);

    /// `other` のすべての修飾を含むか。
    pub fn contains(self, other: Modifiers) -> bool {
        self.0 & other.0 == other.0
    }

    /// 修飾が 1 つも無いか。
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
    /// 修飾キー。
    pub mods: Modifiers,
    /// キー。
    pub key: KeyCode,
}

/// キーの表示用の表記 (04 §7.8)。core (`{key:…}`・ヘルプのキー一覧・チョードの表示) と frontend (パレット・ヒントバー) が
/// 共有する唯一の関数。`Ctrl+Shift+S` の形で、チョードは空白区切り (`Ctrl+K S`)。修飾は `Ctrl` `Alt` `Shift` `Super` の順。
/// キー名は先頭を大文字にする (`Delete` `Insert` `PrintScreen`)。例外は `PgUp` / `PgDn` と矢印 (`↑` `↓` `←` `→`。
/// `ascii` (`Capabilities.ascii_only`) が真なら `Up` `Down` `Left` `Right`)。言語に依存しない。
/// ヒントバーの圧縮表記 (`^S`) は frontend が作る。
pub fn key_notation(strokes: &[KeyStroke], ascii: bool) -> String {
    let mut out = String::new();
    for (i, k) in strokes.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        for (m, name) in [
            (Modifiers::CTRL, "Ctrl"),
            (Modifiers::ALT, "Alt"),
            (Modifiers::SHIFT, "Shift"),
            (Modifiers::SUPER, "Super"),
        ] {
            if k.mods.contains(m) {
                out.push_str(name);
                out.push('+');
            }
        }
        let arrow = |sym: &'static str, word: &'static str| if ascii { word } else { sym };
        let name = match k.key {
            KeyCode::Char(' ') => "Space",
            KeyCode::Char(c) => {
                out.extend(c.to_uppercase());
                continue;
            }
            KeyCode::F(n) => {
                out.push_str(&format!("F{n}"));
                continue;
            }
            KeyCode::Enter => "Enter",
            KeyCode::Tab => "Tab",
            KeyCode::Esc => "Esc",
            KeyCode::Backspace => "Backspace",
            KeyCode::Delete => "Delete",
            KeyCode::Insert => "Insert",
            KeyCode::Home => "Home",
            KeyCode::End => "End",
            KeyCode::PageUp => "PgUp",
            KeyCode::PageDown => "PgDn",
            KeyCode::Up => arrow("↑", "Up"),
            KeyCode::Down => arrow("↓", "Down"),
            KeyCode::Left => arrow("←", "Left"),
            KeyCode::Right => arrow("→", "Right"),
            KeyCode::Menu => "Menu",
            KeyCode::CapsLock => "CapsLock",
            KeyCode::PrintScreen => "PrintScreen",
        };
        out.push_str(name);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn キーの表示の表記() {
        let k = |mods, key| KeyStroke { mods, key };
        let seq = [
            k(Modifiers::CTRL, KeyCode::Char('k')),
            k(Modifiers::NONE, KeyCode::Char('s')),
        ];
        assert_eq!(key_notation(&seq, false), "Ctrl+K S");
        let up = [k(Modifiers::CTRL | Modifiers::SHIFT, KeyCode::Up)];
        assert_eq!(key_notation(&up, false), "Ctrl+Shift+↑");
        assert_eq!(key_notation(&up, true), "Ctrl+Shift+Up");
        for (key, want) in [
            (KeyCode::PageDown, "PgDn"),
            (KeyCode::PageUp, "PgUp"),
            (KeyCode::Delete, "Delete"),
            (KeyCode::Insert, "Insert"),
            (KeyCode::PrintScreen, "PrintScreen"),
            (KeyCode::Char(' '), "Space"),
            (KeyCode::F(12), "F12"),
        ] {
            assert_eq!(key_notation(&[k(Modifiers::NONE, key)], false), want);
        }
        assert_eq!(key_notation(&[], false), "");
    }

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
