//! eve-tui が自分で描く領域 (クローム・オーバーレイ・`~`・横スクロールの印) のロール (docs/07 §8.2・§8.6)。
//! 値は core が解決して `Frame.roles` で渡す (段の選択も core、04 §12.1 C1)。ここは名前で引いて、描画で使う形にまとめるだけ。
//! 編集面の文字のスタイルは core が `Span.style` に付けて渡す。

use eve_view::{Color, RoleStyles, Style};

/// ロール → スタイルの対応 (描画で使う形)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub status_bar: Style,
    pub status_chord: Style,
    pub message_info: Style,
    pub message_warning: Style,
    pub message_error: Style,
    pub hint_bar: Style,
    pub hint_key: Style,
    pub overlay: Style,
    pub overlay_selected: Style,
    pub overlay_match: Style,
    pub overlay_disabled: Style,
    pub placeholder: Style,
    pub selection: Style,
    pub dialog_button: Style,
    pub dialog_button_focused: Style,
    pub toggle_on: Style,
    pub eof_marker: Style,
    pub scroll_marker: Style,
}

/// eve-tui が引くロール名 (07 §8.2 の表のロール名と同じ。表との突合は `tests/theme_table.rs`)。
pub const ROLES: [&str; 18] = [
    "ui.status_bar",
    "ui.status_bar.chord",
    "ui.message.info",
    "ui.message.warning",
    "ui.message.error",
    "ui.hint_bar",
    "ui.hint_bar.key",
    "ui.overlay",
    "ui.overlay.selected",
    "ui.overlay.match",
    "ui.overlay.disabled",
    "ui.input.placeholder",
    "ui.selection",
    "ui.dialog.button",
    "ui.dialog.button.focused",
    "ui.search_bar.toggle_on",
    "ui.eof_marker",
    "ui.scroll_marker",
];

impl Theme {
    /// core が解決したロールの表から作る。表に無いロールは既定 (端末の既定色・属性なし)。
    pub fn from_roles(r: &RoleStyles) -> Theme {
        Theme {
            status_bar: r.get("ui.status_bar"),
            status_chord: r.get("ui.status_bar.chord"),
            message_info: r.get("ui.message.info"),
            message_warning: r.get("ui.message.warning"),
            message_error: r.get("ui.message.error"),
            hint_bar: r.get("ui.hint_bar"),
            hint_key: r.get("ui.hint_bar.key"),
            overlay: r.get("ui.overlay"),
            overlay_selected: r.get("ui.overlay.selected"),
            overlay_match: r.get("ui.overlay.match"),
            overlay_disabled: r.get("ui.overlay.disabled"),
            placeholder: r.get("ui.input.placeholder"),
            selection: r.get("ui.selection"),
            dialog_button: r.get("ui.dialog.button"),
            dialog_button_focused: r.get("ui.dialog.button.focused"),
            toggle_on: r.get("ui.search_bar.toggle_on"),
            eof_marker: r.get("ui.eof_marker"),
            scroll_marker: r.get("ui.scroll_marker"),
        }
    }
}

/// `over` の指定 (既定でない色・属性) を `base` に重ねる。core が渡した `Span.style` がロールの上に載るときに使う。
pub fn overlay_style(base: Style, over: Style) -> Style {
    Style {
        fg: if over.fg == Color::Default {
            base.fg
        } else {
            over.fg
        },
        bg: if over.bg == Color::Default {
            base.bg
        } else {
            over.bg
        },
        mods: base.mods | over.mods,
        underline_color: over.underline_color.or(base.underline_color),
    }
}
