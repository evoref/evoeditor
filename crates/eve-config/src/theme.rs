//! テーマ (docs/07 §8、04 §12): ロール名 → 4 段 (truecolor / 256 / 16 / mono) のスタイル。
//!
//! 色はテーマが、構造は core が持つ (ADR-0008)。段の選択は core が `Capabilities.color_depth` を見て 1 回だけ行い
//! (04 §12.1 C1)、Frame には端末が描ける色だけが載る。組込みテーマ `default` の値は 07 §8.2 の表、
//! `high-contrast-dark` / `high-contrast-light` の値は 07 §8.7 の表が正本で、表との一致は core の試験
//! (`tests/theme_table.rs`) が確かめる。

/// 色 (07 §8.2 の「色」の文法の解決後)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThemeColor {
    /// 端末の既定色。
    Default,
    /// 24 ビット色 (赤・緑・青、各 0〜255)。
    Rgb(u8, u8, u8),
    /// 端末のパレットの番号 (`ansi.<0-255>`、16 色の名前は 0〜15)。
    Ansi(u8),
}

/// 属性 (`mods`) のビット。
pub mod mods {
    /// 属性なし。
    pub const NONE: u8 = 0;
    /// 太字。
    pub const BOLD: u8 = 1;
    /// 下線。
    pub const UNDERLINE: u8 = 2;
    /// 反転。
    pub const REVERSE: u8 = 4;
    /// 淡色。
    pub const DIM: u8 = 8;
}

/// ある段のスタイル。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ThemeStyle {
    /// 前景色。
    pub fg: ThemeColor,
    /// 背景色。
    pub bg: ThemeColor,
    /// 属性 (`mods` の定数のビット和)。
    pub mods: u8,
}

impl ThemeStyle {
    /// 端末の既定色で属性なし。
    pub const DEFAULT: ThemeStyle = ThemeStyle {
        fg: ThemeColor::Default,
        bg: ThemeColor::Default,
        mods: mods::NONE,
    };
}

/// 色深度 (端末が描ける段)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Depth {
    /// 24 ビット色。
    TrueColor,
    /// 256 色。
    Ansi256,
    /// 16 色。
    Ansi16,
    /// 色なし (属性だけ)。
    Mono,
}

/// ロール 1 つの 4 段。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Role {
    /// ロール名 (`ui.text` など、07 §8.2)。
    pub name: &'static str,
    /// 24 ビット色の段。
    pub truecolor: ThemeStyle,
    /// 256 色の段。
    pub ansi256: ThemeStyle,
    /// 16 色の段。
    pub ansi16: ThemeStyle,
    /// 無色の段は属性だけ。
    pub mono: u8,
}

impl Role {
    /// 色深度に合う段。
    pub fn at(&self, depth: Depth) -> ThemeStyle {
        match depth {
            Depth::TrueColor => self.truecolor,
            Depth::Ansi256 => self.ansi256,
            Depth::Ansi16 => self.ansi16,
            Depth::Mono => ThemeStyle {
                fg: ThemeColor::Default,
                bg: ThemeColor::Default,
                mods: self.mono,
            },
        }
    }
}

const D: ThemeColor = ThemeColor::Default;

#[expect(
    clippy::cast_possible_truncation,
    reason = "0xRRGGBB から各 8 ビットを取り出す (上位は捨てる)"
)]
const fn rgb(c: u32) -> ThemeColor {
    ThemeColor::Rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

const fn ansi(n: u8) -> ThemeColor {
    ThemeColor::Ansi(n)
}

const fn st(fg: ThemeColor, bg: ThemeColor, mods: u8) -> ThemeStyle {
    ThemeStyle { fg, bg, mods }
}

use mods::{BOLD, DIM, NONE, REVERSE, UNDERLINE};

/// 組込みテーマ `default` (07 §8.2 の表。256 / 16 の段は表の値を明示値として採る、07 §8.6 の 6)。
const DEFAULT_ROLES: &[Role] = &[
    Role {
        name: "ui.text",
        truecolor: st(D, D, NONE),
        ansi256: st(D, D, NONE),
        ansi16: st(D, D, NONE),
        mono: NONE,
    },
    Role {
        name: "ui.cursor_line",
        truecolor: st(rgb(0xe8e8e8), rgb(0x3a3d4a), NONE),
        ansi256: st(ansi(254), ansi(236), NONE),
        ansi16: st(D, D, NONE),
        mono: NONE,
    },
    Role {
        name: "ui.line_number",
        truecolor: st(rgb(0x808080), D, NONE),
        ansi256: st(ansi(244), D, NONE),
        ansi16: st(ansi(8), D, NONE),
        mono: NONE,
    },
    Role {
        name: "ui.line_number.active",
        truecolor: st(D, D, BOLD),
        ansi256: st(D, D, BOLD),
        ansi16: st(D, D, BOLD),
        mono: BOLD,
    },
    Role {
        name: "ui.eof_marker",
        truecolor: st(rgb(0x808080), D, NONE),
        ansi256: st(ansi(244), D, NONE),
        ansi16: st(ansi(8), D, NONE),
        mono: NONE,
    },
    Role {
        name: "ui.scroll_marker",
        truecolor: st(rgb(0x808080), D, BOLD),
        ansi256: st(ansi(244), D, BOLD),
        ansi16: st(ansi(8), D, BOLD),
        mono: BOLD,
    },
    Role {
        name: "ui.selection",
        truecolor: st(rgb(0xeef0f6), rgb(0x3b5bb5), NONE),
        ansi256: st(ansi(255), ansi(61), NONE),
        ansi16: st(ansi(15), ansi(4), NONE),
        mono: REVERSE,
    },
    Role {
        name: "ui.search.match",
        truecolor: st(rgb(0x1a1a1a), rgb(0xe6c84a), UNDERLINE),
        ansi256: st(ansi(233), ansi(221), UNDERLINE),
        ansi16: st(ansi(0), ansi(3), UNDERLINE),
        mono: UNDERLINE,
    },
    Role {
        name: "ui.search.current",
        truecolor: st(rgb(0x1a1a1a), rgb(0xf0945a), UNDERLINE | BOLD),
        ansi256: st(ansi(233), ansi(209), UNDERLINE | BOLD),
        ansi16: st(ansi(0), ansi(11), UNDERLINE | BOLD),
        mono: REVERSE | UNDERLINE,
    },
    Role {
        name: "ui.status_bar",
        truecolor: st(rgb(0xe8e8e8), rgb(0x2f3340), NONE),
        ansi256: st(ansi(254), ansi(237), NONE),
        ansi16: st(D, D, REVERSE),
        mono: REVERSE,
    },
    Role {
        name: "ui.status_bar.chord",
        truecolor: st(D, D, BOLD),
        ansi256: st(D, D, BOLD),
        ansi16: st(D, D, BOLD),
        mono: BOLD,
    },
    Role {
        name: "ui.message.info",
        truecolor: st(D, D, NONE),
        ansi256: st(D, D, NONE),
        ansi16: st(D, D, NONE),
        mono: NONE,
    },
    Role {
        name: "ui.message.warning",
        truecolor: st(rgb(0x1a1a1a), rgb(0xe0a53b), BOLD),
        ansi256: st(ansi(233), ansi(179), BOLD),
        ansi16: st(ansi(0), ansi(3), BOLD),
        mono: BOLD,
    },
    Role {
        name: "ui.message.error",
        truecolor: st(rgb(0xffffff), rgb(0xb3261e), BOLD),
        ansi256: st(ansi(231), ansi(124), BOLD),
        ansi16: st(ansi(15), ansi(1), BOLD),
        mono: BOLD | REVERSE,
    },
    Role {
        name: "ui.hint_bar",
        truecolor: st(D, D, NONE),
        ansi256: st(D, D, NONE),
        ansi16: st(D, D, NONE),
        mono: NONE,
    },
    Role {
        name: "ui.hint_bar.key",
        truecolor: st(rgb(0x1a1a1a), rgb(0x8fd0ff), NONE),
        ansi256: st(ansi(233), ansi(117), NONE),
        ansi16: st(ansi(0), ansi(6), NONE),
        mono: REVERSE,
    },
    Role {
        name: "ui.overlay",
        truecolor: st(D, D, NONE),
        ansi256: st(D, D, NONE),
        ansi16: st(D, D, NONE),
        mono: NONE,
    },
    Role {
        name: "ui.overlay.selected",
        truecolor: st(rgb(0xeef0f6), rgb(0x3b5bb5), NONE),
        ansi256: st(ansi(255), ansi(61), NONE),
        ansi16: st(ansi(15), ansi(4), NONE),
        mono: REVERSE,
    },
    Role {
        name: "ui.overlay.match",
        truecolor: st(D, D, UNDERLINE | BOLD),
        ansi256: st(D, D, UNDERLINE | BOLD),
        ansi16: st(D, D, UNDERLINE | BOLD),
        mono: UNDERLINE | BOLD,
    },
    Role {
        name: "ui.overlay.disabled",
        truecolor: st(rgb(0x808080), D, NONE),
        ansi256: st(ansi(244), D, NONE),
        ansi16: st(ansi(8), D, NONE),
        mono: NONE,
    },
    Role {
        name: "ui.input.placeholder",
        truecolor: st(rgb(0x808080), D, NONE),
        ansi256: st(ansi(244), D, NONE),
        ansi16: st(ansi(8), D, NONE),
        mono: NONE,
    },
    Role {
        name: "ui.dialog.button",
        truecolor: st(D, D, NONE),
        ansi256: st(D, D, NONE),
        ansi16: st(D, D, NONE),
        mono: NONE,
    },
    Role {
        name: "ui.dialog.button.focused",
        truecolor: st(D, D, REVERSE | BOLD),
        ansi256: st(D, D, REVERSE | BOLD),
        ansi16: st(D, D, REVERSE | BOLD),
        mono: REVERSE | BOLD,
    },
    Role {
        name: "ui.search_bar.toggle_on",
        truecolor: st(D, D, REVERSE),
        ansi256: st(D, D, REVERSE),
        ansi16: st(D, D, REVERSE),
        mono: REVERSE,
    },
    Role {
        name: "ui.control_char",
        truecolor: st(rgb(0xffffff), rgb(0x8a3ab9), NONE),
        ansi256: st(ansi(231), ansi(91), NONE),
        ansi16: st(ansi(15), ansi(5), NONE),
        mono: REVERSE,
    },
    Role {
        name: "ui.whitespace",
        truecolor: st(rgb(0x808080), D, NONE),
        ansi256: st(ansi(244), D, NONE),
        ansi16: st(ansi(8), D, NONE),
        mono: DIM,
    },
];

const fn role(
    name: &'static str,
    truecolor: ThemeStyle,
    ansi256: ThemeStyle,
    ansi16: ThemeStyle,
    mono: u8,
) -> Role {
    Role {
        name,
        truecolor,
        ansi256,
        ansi16,
        mono,
    }
}

/// 端末の既定色で属性だけ (形 (ii)。背景を塗るテーマでは本文の色になる)。
const fn attrs(m: u8) -> ThemeStyle {
    st(D, D, m)
}

/// 16 色の名前の番号 (07 §8.2 の「色」の文法の `ansi.<name>`)。
const BLACK: ThemeColor = ansi(0);
const RED: ThemeColor = ansi(1);
const YELLOW: ThemeColor = ansi(3);
const BLUE: ThemeColor = ansi(4);
const MAGENTA: ThemeColor = ansi(5);
const CYAN: ThemeColor = ansi(6);
const WHITE: ThemeColor = ansi(7);
const BRIGHT_RED: ThemeColor = ansi(9);
const BRIGHT_YELLOW: ThemeColor = ansi(11);
const BRIGHT_CYAN: ThemeColor = ansi(14);
const BRIGHT_WHITE: ThemeColor = ansi(15);

/// 組込みテーマ `high-contrast-dark` (07 §8.7.1 の表。4 段とも明示値。背景を塗る)。
const HIGH_CONTRAST_DARK_ROLES: &[Role] = &[
    role(
        "ui.text",
        st(rgb(0xffffff), rgb(0x000000), NONE),
        st(ansi(231), ansi(16), NONE),
        st(BRIGHT_WHITE, BLACK, NONE),
        NONE,
    ),
    role(
        "ui.cursor_line",
        attrs(NONE),
        attrs(NONE),
        attrs(NONE),
        NONE,
    ),
    role(
        "ui.line_number",
        st(rgb(0xc0c0c0), D, NONE),
        st(ansi(250), D, NONE),
        st(CYAN, D, NONE),
        NONE,
    ),
    role(
        "ui.line_number.active",
        attrs(BOLD),
        attrs(BOLD),
        attrs(UNDERLINE),
        BOLD,
    ),
    role(
        "ui.eof_marker",
        st(rgb(0xc0c0c0), D, NONE),
        st(ansi(250), D, NONE),
        st(CYAN, D, NONE),
        NONE,
    ),
    role(
        "ui.scroll_marker",
        st(rgb(0xc0c0c0), D, BOLD),
        st(ansi(250), D, BOLD),
        st(CYAN, D, NONE),
        BOLD,
    ),
    role(
        "ui.selection",
        st(rgb(0x000000), rgb(0x9ccfff), NONE),
        st(ansi(16), ansi(153), NONE),
        st(BLACK, BRIGHT_CYAN, NONE),
        REVERSE,
    ),
    role(
        "ui.search.match",
        st(rgb(0x000000), rgb(0xffff00), UNDERLINE),
        st(ansi(16), ansi(226), UNDERLINE),
        st(BLACK, BRIGHT_YELLOW, UNDERLINE),
        UNDERLINE,
    ),
    role(
        "ui.search.current",
        st(rgb(0x000000), rgb(0xffa64d), UNDERLINE | BOLD),
        st(ansi(16), ansi(215), UNDERLINE | BOLD),
        st(BRIGHT_WHITE, BLUE, UNDERLINE | BOLD),
        REVERSE | UNDERLINE,
    ),
    role(
        "ui.status_bar",
        st(rgb(0x000000), rgb(0xffffff), NONE),
        st(ansi(16), ansi(231), NONE),
        st(BLACK, BRIGHT_WHITE, NONE),
        REVERSE,
    ),
    role(
        "ui.status_bar.chord",
        attrs(BOLD),
        attrs(BOLD),
        attrs(UNDERLINE),
        BOLD,
    ),
    role(
        "ui.message.info",
        attrs(NONE),
        attrs(NONE),
        attrs(NONE),
        NONE,
    ),
    role(
        "ui.message.warning",
        st(rgb(0x000000), rgb(0xffd75f), BOLD),
        st(ansi(16), ansi(221), BOLD),
        st(BLACK, YELLOW, NONE),
        BOLD,
    ),
    role(
        "ui.message.error",
        st(rgb(0x000000), rgb(0xff9e9e), BOLD),
        st(ansi(16), ansi(217), BOLD),
        st(BLACK, BRIGHT_RED, NONE),
        BOLD | REVERSE,
    ),
    role("ui.hint_bar", attrs(NONE), attrs(NONE), attrs(NONE), NONE),
    role(
        "ui.hint_bar.key",
        st(rgb(0x000000), rgb(0x7fffff), NONE),
        st(ansi(16), ansi(123), NONE),
        st(BLACK, CYAN, NONE),
        REVERSE,
    ),
    role("ui.overlay", attrs(NONE), attrs(NONE), attrs(NONE), NONE),
    role(
        "ui.overlay.selected",
        st(rgb(0x000000), rgb(0x9ccfff), NONE),
        st(ansi(16), ansi(153), NONE),
        st(BLACK, BRIGHT_CYAN, NONE),
        REVERSE,
    ),
    role(
        "ui.overlay.match",
        attrs(UNDERLINE | BOLD),
        attrs(UNDERLINE | BOLD),
        attrs(UNDERLINE),
        UNDERLINE | BOLD,
    ),
    role(
        "ui.overlay.disabled",
        st(rgb(0xc0c0c0), D, NONE),
        st(ansi(250), D, NONE),
        st(CYAN, D, NONE),
        NONE,
    ),
    role(
        "ui.input.placeholder",
        st(rgb(0xc0c0c0), D, NONE),
        st(ansi(250), D, NONE),
        st(CYAN, D, NONE),
        NONE,
    ),
    role(
        "ui.dialog.button",
        attrs(NONE),
        attrs(NONE),
        attrs(NONE),
        NONE,
    ),
    role(
        "ui.dialog.button.focused",
        attrs(REVERSE | BOLD),
        attrs(REVERSE | BOLD),
        attrs(REVERSE | BOLD),
        REVERSE | BOLD,
    ),
    role(
        "ui.search_bar.toggle_on",
        attrs(REVERSE),
        attrs(REVERSE),
        attrs(REVERSE),
        REVERSE,
    ),
    role(
        "ui.control_char",
        st(rgb(0x000000), rgb(0xe0a8ff), NONE),
        st(ansi(16), ansi(183), NONE),
        st(BLACK, WHITE, NONE),
        REVERSE,
    ),
    role(
        "ui.whitespace",
        st(rgb(0xc0c0c0), D, NONE),
        st(ansi(250), D, NONE),
        st(CYAN, D, NONE),
        BOLD,
    ),
];

/// 組込みテーマ `high-contrast-light` (07 §8.7.2 の表。4 段とも明示値。背景を塗る)。
const HIGH_CONTRAST_LIGHT_ROLES: &[Role] = &[
    role(
        "ui.text",
        st(rgb(0x000000), rgb(0xffffff), NONE),
        st(ansi(16), ansi(231), NONE),
        st(BLACK, BRIGHT_WHITE, NONE),
        NONE,
    ),
    role(
        "ui.cursor_line",
        attrs(NONE),
        attrs(NONE),
        attrs(NONE),
        NONE,
    ),
    role(
        "ui.line_number",
        st(rgb(0x404040), D, NONE),
        st(ansi(238), D, NONE),
        st(BLUE, D, NONE),
        NONE,
    ),
    role(
        "ui.line_number.active",
        attrs(BOLD),
        attrs(BOLD),
        attrs(UNDERLINE),
        BOLD,
    ),
    role(
        "ui.eof_marker",
        st(rgb(0x404040), D, NONE),
        st(ansi(238), D, NONE),
        st(BLUE, D, NONE),
        NONE,
    ),
    role(
        "ui.scroll_marker",
        st(rgb(0x404040), D, BOLD),
        st(ansi(238), D, BOLD),
        st(BLUE, D, NONE),
        BOLD,
    ),
    role(
        "ui.selection",
        st(rgb(0xffffff), rgb(0x0a3d91), NONE),
        st(ansi(231), ansi(19), NONE),
        st(BRIGHT_WHITE, BLUE, NONE),
        REVERSE,
    ),
    role(
        "ui.search.match",
        st(rgb(0xffffff), rgb(0x005f00), UNDERLINE),
        st(ansi(231), ansi(22), UNDERLINE),
        st(BRIGHT_WHITE, MAGENTA, UNDERLINE),
        UNDERLINE,
    ),
    role(
        "ui.search.current",
        st(rgb(0xffffff), rgb(0x000000), UNDERLINE | BOLD),
        st(ansi(231), ansi(16), UNDERLINE | BOLD),
        st(BRIGHT_WHITE, BLACK, UNDERLINE | BOLD),
        REVERSE | UNDERLINE,
    ),
    role(
        "ui.status_bar",
        st(rgb(0xffffff), rgb(0x000000), NONE),
        st(ansi(231), ansi(16), NONE),
        st(BRIGHT_WHITE, BLACK, NONE),
        REVERSE,
    ),
    role(
        "ui.status_bar.chord",
        attrs(BOLD),
        attrs(BOLD),
        attrs(BOLD),
        BOLD,
    ),
    role(
        "ui.message.info",
        attrs(NONE),
        attrs(NONE),
        attrs(NONE),
        NONE,
    ),
    role(
        "ui.message.warning",
        st(rgb(0xffffff), rgb(0x6b4400), BOLD),
        st(ansi(231), ansi(237), BOLD),
        st(BLACK, YELLOW, NONE),
        BOLD,
    ),
    role(
        "ui.message.error",
        st(rgb(0xffffff), rgb(0xa00000), BOLD),
        st(ansi(231), ansi(124), BOLD),
        st(BRIGHT_WHITE, RED, BOLD),
        BOLD | REVERSE,
    ),
    role("ui.hint_bar", attrs(NONE), attrs(NONE), attrs(NONE), NONE),
    role(
        "ui.hint_bar.key",
        st(rgb(0xffffff), rgb(0x005a5a), NONE),
        st(ansi(231), ansi(23), NONE),
        st(BRIGHT_WHITE, BLUE, NONE),
        REVERSE,
    ),
    role("ui.overlay", attrs(NONE), attrs(NONE), attrs(NONE), NONE),
    role(
        "ui.overlay.selected",
        st(rgb(0xffffff), rgb(0x0a3d91), NONE),
        st(ansi(231), ansi(19), NONE),
        st(BRIGHT_WHITE, BLUE, NONE),
        REVERSE,
    ),
    role(
        "ui.overlay.match",
        attrs(UNDERLINE | BOLD),
        attrs(UNDERLINE | BOLD),
        attrs(UNDERLINE),
        UNDERLINE | BOLD,
    ),
    role(
        "ui.overlay.disabled",
        st(rgb(0x404040), D, NONE),
        st(ansi(238), D, NONE),
        st(BLUE, D, NONE),
        NONE,
    ),
    role(
        "ui.input.placeholder",
        st(rgb(0x404040), D, NONE),
        st(ansi(238), D, NONE),
        st(BLUE, D, NONE),
        NONE,
    ),
    role(
        "ui.dialog.button",
        attrs(NONE),
        attrs(NONE),
        attrs(NONE),
        NONE,
    ),
    role(
        "ui.dialog.button.focused",
        attrs(REVERSE | BOLD),
        attrs(REVERSE | BOLD),
        attrs(REVERSE),
        REVERSE | BOLD,
    ),
    role(
        "ui.search_bar.toggle_on",
        attrs(REVERSE),
        attrs(REVERSE),
        attrs(REVERSE),
        REVERSE,
    ),
    role(
        "ui.control_char",
        st(rgb(0xffffff), rgb(0x6a1b9a), NONE),
        st(ansi(231), ansi(54), NONE),
        st(BRIGHT_WHITE, MAGENTA, NONE),
        REVERSE,
    ),
    role(
        "ui.whitespace",
        st(rgb(0x404040), D, NONE),
        st(ansi(238), D, NONE),
        st(BLUE, D, NONE),
        BOLD,
    ),
];

/// 組込みテーマの名前 (07 §5.4 `theme.name`)。
pub const BUILTIN_NAMES: &[&str] = &[
    "default",
    "default-dark",
    "default-light",
    "ansi",
    "high-contrast-dark",
    "high-contrast-light",
];

/// 組込みの名前に近い名前 (07 §8.7): `name` が組込みの名前の接頭辞か、編集距離 2 以下の組込みの名前を、
/// `BUILTIN_NAMES` の順に返す (どちらも大文字小文字を区別しない)。`name` が組込みの名前なら空。テーマファイルも
/// 無いときの診断 (`DiagKind::ThemeNotFound`) の候補に使う。
pub fn near_builtin_names(name: &str) -> Vec<&'static str> {
    if BUILTIN_NAMES.contains(&name) {
        return Vec::new();
    }
    let lower = name.to_ascii_lowercase();
    BUILTIN_NAMES
        .iter()
        .copied()
        .filter(|b| b.starts_with(&lower) || crate::config::distance(&lower, b) <= 2)
        .collect()
}

/// ほかのロールの上に重ねて使うロール (07 §8.6 の 3 本目の「重ねるロール」)。背景を塗るテーマでも、端末の既定色を
/// 本文の色に置き換えず、既定色のまま下のロールの色を通す。
pub const LAYERED_ROLES: [&str; 4] = [
    "ui.status_bar.chord",
    "ui.line_number.active",
    "ui.overlay.match",
    "ui.search_bar.toggle_on",
];

/// 組込みテーマ (07 §8.6)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    /// ロールの一覧。
    pub roles: Vec<Role>,
    /// 背景を塗る (`default-dark` / `default-light` / `high-contrast-dark` / `high-contrast-light`)。色の段では、各ロールの
    /// 端末の既定色を `ui.text` の色で置き換える (重ねるロール `LAYERED_ROLES` は置き換えない)。
    pub paints: bool,
}

/// 背景を塗るテーマの本文の色 (07 §8.6 の表。`ui.text` の 3 段)。
const DARK_TEXT: [ThemeStyle; 3] = [
    st(rgb(0xd4d4d4), rgb(0x1e1e1e), NONE),
    st(ansi(252), ansi(234), NONE),
    st(ansi(7), ansi(0), NONE),
];
const LIGHT_TEXT: [ThemeStyle; 3] = [
    st(rgb(0x1e1e1e), rgb(0xfafafa), NONE),
    st(ansi(235), ansi(255), NONE),
    st(ansi(0), ansi(15), NONE),
];

/// 組込みテーマ。名前が無ければ `None`。
/// `default-dark` / `default-light` は `default` の本文 (`ui.text`) に色を付けて背景を塗り、`ansi` は全段で `default` の
/// 16 色の段の値を使う (端末の配色に馴染む)。
pub fn builtin(name: &str) -> Option<Theme> {
    let paint = |text: [ThemeStyle; 3]| Theme {
        roles: DEFAULT_ROLES
            .iter()
            .map(|r| match r.name {
                "ui.text" => Role {
                    truecolor: text[0],
                    ansi256: text[1],
                    ansi16: text[2],
                    ..*r
                },
                _ => *r,
            })
            .collect(),
        paints: true,
    };
    Some(match name {
        "default" => Theme {
            roles: DEFAULT_ROLES.to_vec(),
            paints: false,
        },
        "default-dark" => paint(DARK_TEXT),
        "default-light" => paint(LIGHT_TEXT),
        "high-contrast-dark" => Theme {
            roles: HIGH_CONTRAST_DARK_ROLES.to_vec(),
            paints: true,
        },
        "high-contrast-light" => Theme {
            roles: HIGH_CONTRAST_LIGHT_ROLES.to_vec(),
            paints: true,
        },
        "ansi" => Theme {
            roles: DEFAULT_ROLES
                .iter()
                .map(|r| Role {
                    truecolor: r.ansi16,
                    ansi256: r.ansi16,
                    ..*r
                })
                .collect(),
            paints: false,
        },
        _ => return None,
    })
}

/// 背景を塗るテーマの明暗の切替 (`theme.background`、07 §8.6・§8.7): `default-dark` / `default-light` と
/// `high-contrast-dark` / `high-contrast-light` を選んだときだけ、`dark` / `light` で対のその明暗に替える。
/// `auto` と、それ以外のテーマはそのまま。
pub fn with_background(name: &str, background: crate::ThemeBackground) -> &str {
    use crate::ThemeBackground::{Auto, Dark, Light};
    match (name, background) {
        ("default-dark" | "default-light", Dark) => "default-dark",
        ("default-dark" | "default-light", Light) => "default-light",
        ("high-contrast-dark" | "high-contrast-light", Dark) => "high-contrast-dark",
        ("high-contrast-dark" | "high-contrast-light", Light) => "high-contrast-light",
        (_, Auto | Dark | Light) => name,
    }
}

/// テーマの、色深度に合う段のロールの表 (ロール名の順)。背景を塗るテーマは、色の段で端末の既定色を本文の色にする
/// (重ねるロール `LAYERED_ROLES` は既定色のまま下のロールの色を通す、07 §8.6)。
pub fn resolve_roles(theme: &Theme, depth: Depth) -> Vec<(&'static str, ThemeStyle)> {
    let text = theme
        .roles
        .iter()
        .find(|r| r.name == "ui.text")
        .map_or(ThemeStyle::DEFAULT, |r| r.at(depth));
    let fill = theme.paints && depth != Depth::Mono;
    let mut out: Vec<(&'static str, ThemeStyle)> = theme
        .roles
        .iter()
        .map(|r| {
            let mut s = r.at(depth);
            if fill && !LAYERED_ROLES.contains(&r.name) {
                if s.fg == ThemeColor::Default {
                    s.fg = text.fg;
                }
                if s.bg == ThemeColor::Default {
                    s.bg = text.bg;
                }
            }
            (r.name, s)
        })
        .collect();
    out.sort_by_key(|(n, _)| *n);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_は無色の段では属性だけ() {
        let t = builtin("default").unwrap();
        let roles = resolve_roles(&t, Depth::Mono);
        assert!(
            roles
                .iter()
                .all(|(_, s)| s.fg == ThemeColor::Default && s.bg == ThemeColor::Default)
        );
        let sel = roles.iter().find(|(n, _)| *n == "ui.selection").unwrap().1;
        assert_eq!(sel.mods, REVERSE);
        let _ = (DIM, BOLD, UNDERLINE, NONE);
    }

    #[test]
    fn 各段は端末が描ける色だけを使う() {
        for name in BUILTIN_NAMES {
            for r in builtin(name).unwrap().roles {
                for c in [r.ansi16.fg, r.ansi16.bg] {
                    assert!(
                        matches!(c, ThemeColor::Default | ThemeColor::Ansi(0..=15)),
                        "{}",
                        r.name
                    );
                }
                for c in [r.ansi256.fg, r.ansi256.bg] {
                    assert!(!matches!(c, ThemeColor::Rgb(..)), "{}", r.name);
                }
            }
        }
    }

    #[test]
    fn 背景を塗るテーマは色の段で既定色を本文の色にし_無色の段は塗らない() {
        let t = builtin("default-dark").unwrap();
        let tc = resolve_roles(&t, Depth::TrueColor);
        assert!(
            tc.iter()
                .filter(|(n, _)| !LAYERED_ROLES.contains(n))
                .all(|(_, s)| s.fg != ThemeColor::Default && s.bg != ThemeColor::Default),
            "{tc:?}"
        );
        // 重ねるロールは既定色のまま (下のロールの色を通す、07 §8.6)
        for name in LAYERED_ROLES {
            let s = tc.iter().find(|(n, _)| *n == name).unwrap().1;
            assert_eq!(
                (s.fg, s.bg),
                (ThemeColor::Default, ThemeColor::Default),
                "{name}"
            );
            assert_ne!(s.mods, NONE, "{name}");
        }
        let text = tc.iter().find(|(n, _)| *n == "ui.text").unwrap().1;
        assert_eq!(text.bg, rgb(0x1e1e1e));
        let mono = resolve_roles(&t, Depth::Mono);
        assert!(mono.iter().all(|(_, s)| s.bg == ThemeColor::Default));
        // ansi は 16 色の段の値だけ
        let a = resolve_roles(&builtin("ansi").unwrap(), Depth::TrueColor);
        assert!(
            a.iter().all(|(_, s)| !matches!(s.fg, ThemeColor::Rgb(..))
                && !matches!(s.bg, ThemeColor::Rgb(..)))
        );
    }

    #[test]
    fn 明暗の切替は塗るテーマにだけ効く() {
        use crate::ThemeBackground::{Auto, Dark, Light};
        assert_eq!(with_background("default-dark", Light), "default-light");
        assert_eq!(with_background("default-light", Dark), "default-dark");
        assert_eq!(with_background("default-light", Auto), "default-light");
        assert_eq!(with_background("default", Light), "default");
        assert_eq!(
            with_background("high-contrast-dark", Light),
            "high-contrast-light"
        );
        assert_eq!(
            with_background("high-contrast-light", Dark),
            "high-contrast-dark"
        );
        assert_eq!(
            with_background("high-contrast-dark", Auto),
            "high-contrast-dark"
        );
        assert_eq!(with_background("ansi", Dark), "ansi");
    }

    #[test]
    fn 高コントラストのテーマは背景を塗り_dim_を使わない() {
        for name in ["high-contrast-dark", "high-contrast-light"] {
            let t = builtin(name).unwrap();
            assert!(t.paints, "{name}");
            let names: Vec<&str> = t.roles.iter().map(|r| r.name).collect();
            let default: Vec<&str> = DEFAULT_ROLES.iter().map(|r| r.name).collect();
            assert_eq!(names, default, "{name}: ロールは default と同じ 26 個");
            for r in &t.roles {
                for m in [r.truecolor.mods, r.ansi256.mods, r.ansi16.mods, r.mono] {
                    assert_eq!(m & DIM, 0, "{name} {}", r.name);
                }
            }
        }
    }

    #[test]
    fn 組込みの名前に近い名前は候補を返す() {
        assert_eq!(
            near_builtin_names("high-contrast"),
            ["high-contrast-dark", "high-contrast-light"]
        );
        assert_eq!(
            near_builtin_names("high-contrast-drak"),
            ["high-contrast-dark"]
        );
        assert_eq!(near_builtin_names("defualt"), ["default"]);
        assert_eq!(
            near_builtin_names("default-"),
            ["default", "default-dark", "default-light"]
        );
        assert_eq!(near_builtin_names("high-contrast-dark"), Vec::<&str>::new());
        assert_eq!(near_builtin_names("my-theme"), Vec::<&str>::new());
        assert_eq!(near_builtin_names("missing"), Vec::<&str>::new());
        // 大文字小文字を区別しない
        assert_eq!(
            near_builtin_names("High-Contrast"),
            ["high-contrast-dark", "high-contrast-light"]
        );
        assert_eq!(
            near_builtin_names("HIGH-CONTRAST-DARK"),
            ["high-contrast-dark"]
        );
    }
}
