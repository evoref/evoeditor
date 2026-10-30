//! 端末が横取りする (アプリに届けない) キーの定数表 (docs/04 §7.3a が正本)。`Capabilities.intercepted_keys` を埋める。
//!
//! 表のキー表記は 07 §7.2 の設定表記で、docs の表と一致することを試験で確かめる (表を直したら、ここも同じ PR で直す)。

use eve_view::{KeyCode, KeyStroke, Modifiers};

use crate::caps::TerminalKind;

/// Windows Terminal の既定のキー割当から起こした横取りのキー (04 §7.3a の「横取りするキー」列)。
const WINDOWS_TERMINAL: &[&str] = &[
    "ctrl+shift+p",
    "ctrl+shift+t",
    "ctrl+shift+w",
    "ctrl+shift+n",
    "ctrl+shift+d",
    "ctrl+shift+f",
    "ctrl+shift+a",
    "ctrl+shift+m",
    "ctrl+shift+space",
    "ctrl+shift+1",
    "ctrl+shift+2",
    "ctrl+shift+3",
    "ctrl+shift+4",
    "ctrl+shift+5",
    "ctrl+shift+6",
    "ctrl+shift+7",
    "ctrl+shift+8",
    "ctrl+shift+9",
    "ctrl+alt+1",
    "ctrl+alt+2",
    "ctrl+alt+3",
    "ctrl+alt+4",
    "ctrl+alt+5",
    "ctrl+alt+6",
    "ctrl+alt+7",
    "ctrl+alt+8",
    "ctrl+alt+9",
    "ctrl+,",
    "ctrl+shift+up",
    "ctrl+shift+down",
    "ctrl+shift+home",
    "ctrl+shift+end",
    "ctrl+shift+pageup",
    "ctrl+shift+pagedown",
    "ctrl+tab",
    "ctrl+shift+tab",
    "ctrl+=",
    "ctrl+-",
    "ctrl+0",
    "alt+enter",
    "f11",
    "alt+left",
    "alt+right",
    "alt+up",
    "alt+down",
    "alt+shift+left",
    "alt+shift+right",
    "alt+shift+up",
    "alt+shift+down",
    "alt+shift+d",
    "alt+shift+-",
    "alt+shift+plus",
];

/// conhost (04 §7.3a)。
const CONHOST: &[&str] = &["alt+enter", "alt+space", "alt+f4"];

/// 端末種別ごとの横取りのキーの表記 (07 §7.2 の設定表記)。
pub fn table(kind: TerminalKind) -> &'static [&'static str] {
    match kind {
        TerminalKind::WindowsTerminal => WINDOWS_TERMINAL,
        TerminalKind::Conhost => CONHOST,
        TerminalKind::Other => &[],
    }
}

/// 端末種別ごとの横取りのキー。
pub fn intercepted_keys(kind: TerminalKind) -> Vec<KeyStroke> {
    table(kind).iter().filter_map(|s| stroke(s)).collect()
}

/// 表に現れる形だけを読む (修飾 `ctrl` / `alt` / `shift` と、文字・数字・記号・名前付きのキー)。
fn stroke(s: &str) -> Option<KeyStroke> {
    let mut parts: Vec<&str> = s.split('+').collect();
    let name = parts.pop()?;
    let mut mods = Modifiers::NONE;
    for p in parts {
        mods = mods
            | match p {
                "ctrl" => Modifiers::CTRL,
                "alt" => Modifiers::ALT,
                "shift" => Modifiers::SHIFT,
                _ => return None,
            };
    }
    let key = match name {
        "enter" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "space" => KeyCode::Char(' '),
        "plus" => KeyCode::Char('+'),
        f if f.len() > 1 && f.starts_with('f') => KeyCode::F(f[1..].parse().ok()?),
        c => {
            let mut it = c.chars();
            let ch = it.next()?;
            if it.next().is_some() {
                return None;
            }
            KeyCode::Char(ch)
        }
    };
    Some(KeyStroke { mods, key })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 表のキーはすべて読める() {
        for s in WINDOWS_TERMINAL.iter().chain(CONHOST) {
            assert!(stroke(s).is_some(), "{s}");
        }
        assert_eq!(
            intercepted_keys(TerminalKind::WindowsTerminal).len(),
            WINDOWS_TERMINAL.len()
        );
        assert!(intercepted_keys(TerminalKind::Other).is_empty());
    }
}
