//! キー表記 (04 §7.8)。

use eve_view::{KeyCode, KeyLabel, KeyStroke, Modifiers};

pub(super) fn key_name(code: KeyCode, ascii: bool) -> String {
    match code {
        KeyCode::Char(' ') => "Space".to_owned(),
        KeyCode::Char(c) => c.to_uppercase().collect(),
        KeyCode::F(n) => format!("F{n}"),
        KeyCode::Enter => "Enter".to_owned(),
        KeyCode::Tab => "Tab".to_owned(),
        KeyCode::Esc => "Esc".to_owned(),
        KeyCode::Backspace => "Backspace".to_owned(),
        KeyCode::Delete => "Del".to_owned(),
        KeyCode::Insert => "Ins".to_owned(),
        KeyCode::Home => "Home".to_owned(),
        KeyCode::End => "End".to_owned(),
        KeyCode::PageUp => "PgUp".to_owned(),
        KeyCode::PageDown => "PgDn".to_owned(),
        KeyCode::Up => if ascii { "Up" } else { "↑" }.to_owned(),
        KeyCode::Down => if ascii { "Down" } else { "↓" }.to_owned(),
        KeyCode::Left => if ascii { "Left" } else { "←" }.to_owned(),
        KeyCode::Right => if ascii { "Right" } else { "→" }.to_owned(),
        KeyCode::Menu => "Menu".to_owned(),
        KeyCode::CapsLock => "CapsLock".to_owned(),
        KeyCode::PrintScreen => "PrtSc".to_owned(),
        _ => "?".to_owned(),
    }
}

/// `Ctrl+S` 形式。チョードは空白区切り。
pub fn key_label_full(label: &KeyLabel, ascii: bool) -> String {
    let stroke = |s: &KeyStroke| {
        let mut out = String::new();
        for (flag, name) in [
            (Modifiers::CTRL, "Ctrl"),
            (Modifiers::ALT, "Alt"),
            (Modifiers::SHIFT, "Shift"),
            (Modifiers::SUPER, "Super"),
        ] {
            if s.mods.contains(flag) {
                out.push_str(name);
                out.push('+');
            }
        }
        out.push_str(&key_name(s.key, ascii));
        out
    };
    label
        .strokes
        .iter()
        .map(stroke)
        .collect::<Vec<_>>()
        .join(" ")
}

/// ヒントバーの圧縮表記: `Ctrl` + 1 文字は `^S`、それ以外はそのまま。
pub fn key_label_compact(label: &KeyLabel, ascii: bool) -> String {
    let stroke = |s: &KeyStroke| match (s.mods, s.key) {
        (m, KeyCode::Char(c)) if m == Modifiers::CTRL && c != ' ' => {
            format!("^{}", c.to_uppercase().collect::<String>())
        }
        _ => key_label_full(&KeyLabel { strokes: vec![*s] }, ascii),
    };
    label
        .strokes
        .iter()
        .map(stroke)
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn label_width(s: &str) -> u16 {
    // この層が作るキー表記は ASCII と、非縮退時の矢印 (1 セル) だけ
    s.chars().count() as u16
}
