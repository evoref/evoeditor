//! テーマ (docs/07 §8、04 §12): ロール名 → 4 段 (truecolor / 256 / 16 / mono) のスタイル。
//!
//! 色はテーマが、構造は core が持つ (ADR-0008)。段の選択は core が `Capabilities.color_depth` を見て 1 回だけ行い
//! (04 §12.1 C1)、Frame には端末が描ける色だけが載る。組込みテーマ `default` の値は 07 §8.2 の表が正本で、
//! 表との一致は core の試験 (`tests/theme_table.rs`) が確かめる。

/// 色 (07 §8.2 の「色」の文法の解決後)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThemeColor {
    /// 端末の既定色。
    Default,
    Rgb(u8, u8, u8),
    /// 端末のパレットの番号 (`ansi.<0-255>`、16 色の名前は 0〜15)。
    Ansi(u8),
}

/// 属性 (`mods`) のビット。
pub mod mods {
    pub const NONE: u8 = 0;
    pub const BOLD: u8 = 1;
    pub const UNDERLINE: u8 = 2;
    pub const REVERSE: u8 = 4;
    pub const DIM: u8 = 8;
}

/// ある段のスタイル。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ThemeStyle {
    pub fg: ThemeColor,
    pub bg: ThemeColor,
    pub mods: u8,
}

impl ThemeStyle {
    pub const DEFAULT: ThemeStyle = ThemeStyle {
        fg: ThemeColor::Default,
        bg: ThemeColor::Default,
        mods: mods::NONE,
    };
}

/// 色深度 (端末が描ける段)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Depth {
    TrueColor,
    Ansi256,
    Ansi16,
    Mono,
}

/// ロール 1 つの 4 段。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Role {
    pub name: &'static str,
    pub truecolor: ThemeStyle,
    pub ansi256: ThemeStyle,
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

/// 組込みテーマ。名前が無ければ `None`。
pub fn builtin(name: &str) -> Option<&'static [Role]> {
    match name {
        "default" => Some(DEFAULT_ROLES),
        _ => None,
    }
}

/// テーマの、色深度に合う段のロールの表 (ロール名の順)。
pub fn resolve_roles(theme: &[Role], depth: Depth) -> Vec<(&'static str, ThemeStyle)> {
    let mut out: Vec<(&'static str, ThemeStyle)> =
        theme.iter().map(|r| (r.name, r.at(depth))).collect();
    out.sort_by_key(|(n, _)| *n);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_は無色の段では属性だけ() {
        let t = builtin("default").unwrap();
        let roles = resolve_roles(t, Depth::Mono);
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
        for r in builtin("default").unwrap() {
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
