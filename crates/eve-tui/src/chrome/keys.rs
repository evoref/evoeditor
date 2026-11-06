//! キー表記 (04 §7.8)。

use eve_view::{KeyCode, KeyLabel, KeyStroke, Modifiers, key_notation};

/// `Ctrl+S` 形式。チョードは空白区切り。表記は eve-view の `key_notation` (core と共有) に任せる。
pub fn key_label_full(label: &KeyLabel, ascii: bool) -> String {
    key_notation(&label.strokes, ascii)
}

/// ヒントバーの圧縮表記: `Ctrl` + 1 文字は `^S`、それ以外はそのまま。
pub fn key_label_compact(label: &KeyLabel, ascii: bool) -> String {
    let stroke = |s: &KeyStroke| match (s.mods, s.key) {
        (m, KeyCode::Char(c)) if m == Modifiers::CTRL && c != ' ' => {
            format!("^{}", c.to_uppercase().collect::<String>())
        }
        _ => key_notation(std::slice::from_ref(s), ascii),
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
