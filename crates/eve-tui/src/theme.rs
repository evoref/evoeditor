//! 組込みテーマ `default` のうち、eve-tui が自分で描く領域 (クローム・オーバーレイ・`~`) のロール (docs/07 §8.2)。
//! 編集面の文字のスタイルは core が解決して `Span.style` で渡すので、ここにあるのは `Frame` にロールとして届かない部分だけ。
//! v0.1 で実装するのは truecolor と mono の列で、256 / 16 の段は truecolor の値から `color::clamp_style` で近似する (07 §8.2)。

use eve_view::{Color, ColorDepth, Style, StyleMods};

/// ロール → スタイルの対応。
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
    pub placeholder: Style,
    pub dialog_button: Style,
    pub dialog_button_focused: Style,
    pub eof_marker: Style,
    pub scroll_marker: Style,
}

const BOLD: StyleMods = StyleMods::BOLD;
const REVERSE: StyleMods = StyleMods::REVERSE;

const fn rgb(c: u32) -> Color {
    Color::Rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

const fn style(fg: Color, bg: Color, mods: StyleMods) -> Style {
    Style {
        fg,
        bg,
        mods,
        underline_color: None,
    }
}

const fn mods(mods: StyleMods) -> Style {
    style(Color::Default, Color::Default, mods)
}

impl Theme {
    /// 色深度に合わせたテーマ。`Mono` (色なし) は属性だけ、それ以外は truecolor の値 (段への近似は描画時)。
    pub fn for_depth(depth: ColorDepth) -> Theme {
        if depth == ColorDepth::Mono {
            Theme::mono()
        } else {
            Theme::truecolor()
        }
    }

    pub fn truecolor() -> Theme {
        let gray = rgb(0x80_80_80);
        Theme {
            status_bar: style(rgb(0xe8_e8_e8), rgb(0x2f_33_40), StyleMods::NONE),
            status_chord: mods(BOLD),
            message_info: Style::DEFAULT,
            message_warning: style(rgb(0x1a_1a_1a), rgb(0xe0_a5_3b), BOLD),
            message_error: style(rgb(0xff_ff_ff), rgb(0xb3_26_1e), BOLD),
            hint_bar: Style::DEFAULT,
            hint_key: style(rgb(0x1a_1a_1a), rgb(0x8f_d0_ff), StyleMods::NONE),
            overlay: Style::DEFAULT,
            placeholder: style(gray, Color::Default, StyleMods::NONE),
            dialog_button: Style::DEFAULT,
            dialog_button_focused: mods(REVERSE | BOLD),
            eof_marker: style(gray, Color::Default, StyleMods::NONE),
            scroll_marker: style(gray, Color::Default, BOLD),
        }
    }

    pub fn mono() -> Theme {
        Theme {
            status_bar: mods(REVERSE),
            status_chord: mods(BOLD),
            message_info: Style::DEFAULT,
            message_warning: mods(BOLD),
            message_error: mods(BOLD | REVERSE),
            hint_bar: Style::DEFAULT,
            hint_key: mods(REVERSE),
            overlay: Style::DEFAULT,
            placeholder: Style::DEFAULT,
            dialog_button: Style::DEFAULT,
            dialog_button_focused: mods(REVERSE | BOLD),
            eof_marker: Style::DEFAULT,
            scroll_marker: mods(BOLD),
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
