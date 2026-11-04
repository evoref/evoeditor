//! 装飾記号と枠線 (docs/04 §3.7)。eve-tui が自分で描く記号の対応表。`Capabilities.ascii_only` が真なら ASCII に縮退する。
//! 文書の本文と、core が渡す文言は対象外 (置き換えない)。幅は定数表で持ち、書記素の幅は計算しない (§16)。

/// 記号 1 つ。`width` はセル数 (ASCII の縮退時の `...` は 3)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyph {
    pub text: &'static str,
    pub width: u16,
}

const fn g(text: &'static str, width: u16) -> Glyph {
    Glyph { text, width }
}

/// 枠線の部品。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoxGlyphs {
    pub horizontal: Glyph,
    pub vertical: Glyph,
    pub top_left: Glyph,
    pub top_right: Glyph,
    pub bottom_left: Glyph,
    pub bottom_right: Glyph,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyphs {
    pub boxed: BoxGlyphs,
    /// ガターの区切り。
    pub gutter_sep: Glyph,
    /// 変更マーカー (`[+]` は常に併記する)。
    pub modified: Glyph,
    /// フォーカス中の項目・ボタン。
    pub focus: Glyph,
    pub ellipsis: Glyph,
    /// 横スクロールのクリップの印。
    pub clip_left: Glyph,
    pub clip_right: Glyph,
    pub arrow_up: Glyph,
    pub arrow_down: Glyph,
    pub arrow_left: Glyph,
    pub arrow_right: Glyph,
    pub info: Glyph,
    pub warning: Glyph,
    pub error: Glyph,
    pub toggle_on: Glyph,
    /// 文書末尾より後の行。
    pub eof_line: Glyph,
    /// 折り返しの継続行の印 (行番号欄の右端、04 §10)。
    pub wrap_cont: Glyph,
}

impl Glyphs {
    /// 縮退しないとき (Windows Terminal で曖昧幅 1)。すべて 1 セルで数える。
    pub const fn unicode() -> Glyphs {
        Glyphs {
            boxed: BoxGlyphs {
                horizontal: g("─", 1),
                vertical: g("│", 1),
                top_left: g("┌", 1),
                top_right: g("┐", 1),
                bottom_left: g("└", 1),
                bottom_right: g("┘", 1),
            },
            gutter_sep: g("│", 1),
            modified: g("●", 1),
            focus: g("▶", 1),
            ellipsis: g("…", 1),
            clip_left: g("‹", 1),
            clip_right: g("›", 1),
            arrow_up: g("↑", 1),
            arrow_down: g("↓", 1),
            arrow_left: g("←", 1),
            arrow_right: g("→", 1),
            info: g("ℹ", 1),
            warning: g("⚠", 1),
            error: g("✖", 1),
            toggle_on: g("✓", 1),
            eof_line: g("~", 1),
            wrap_cont: g("↪", 1),
        }
    }

    /// ASCII への縮退 (A1 / A2、§3.7 の対応表)。
    pub const fn ascii() -> Glyphs {
        Glyphs {
            boxed: BoxGlyphs {
                horizontal: g("-", 1),
                vertical: g("|", 1),
                top_left: g("+", 1),
                top_right: g("+", 1),
                bottom_left: g("+", 1),
                bottom_right: g("+", 1),
            },
            gutter_sep: g("|", 1),
            modified: g("*", 1),
            focus: g(">", 1),
            ellipsis: g("...", 3),
            clip_left: g("<", 1),
            clip_right: g(">", 1),
            arrow_up: g("^", 1),
            arrow_down: g("v", 1),
            arrow_left: g("<", 1),
            arrow_right: g(">", 1),
            info: g("i", 1),
            warning: g("!", 1),
            error: g("x", 1),
            toggle_on: g("*", 1),
            eof_line: g("~", 1),
            wrap_cont: g(">", 1),
        }
    }

    pub const fn for_ascii_only(ascii_only: bool) -> Glyphs {
        if ascii_only {
            Glyphs::ascii()
        } else {
            Glyphs::unicode()
        }
    }

    #[cfg(test)]
    fn all(&self) -> [Glyph; 25] {
        [
            self.boxed.horizontal,
            self.boxed.vertical,
            self.boxed.top_left,
            self.boxed.top_right,
            self.boxed.bottom_left,
            self.boxed.bottom_right,
            self.gutter_sep,
            self.modified,
            self.focus,
            self.ellipsis,
            self.clip_left,
            self.clip_right,
            self.arrow_up,
            self.arrow_down,
            self.arrow_left,
            self.arrow_right,
            self.info,
            self.warning,
            self.error,
            self.toggle_on,
            self.eof_line,
            self.wrap_cont,
            self.boxed.horizontal,
            self.boxed.vertical,
            self.gutter_sep,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 縮退した記号はすべて_ascii() {
        for gl in Glyphs::ascii().all() {
            assert!(gl.text.is_ascii(), "{:?}", gl.text);
            assert_eq!(gl.width as usize, gl.text.chars().count(), "{:?}", gl.text);
        }
    }

    #[test]
    fn 縮退しない記号はすべて_1_セルで数える() {
        for gl in Glyphs::unicode().all() {
            assert_eq!(gl.width, 1, "{:?}", gl.text);
            assert_eq!(gl.text.chars().count(), 1, "{:?}", gl.text);
        }
    }

    #[test]
    fn 対応表の主な行() {
        let (u, a) = (Glyphs::unicode(), Glyphs::ascii());
        assert_eq!((u.gutter_sep.text, a.gutter_sep.text), ("│", "|"));
        assert_eq!((u.wrap_cont.text, a.wrap_cont.text), ("↪", ">"));
        assert_eq!((u.modified.text, a.modified.text), ("●", "*"));
        assert_eq!((u.focus.text, a.focus.text), ("▶", ">"));
        assert_eq!((u.ellipsis.text, a.ellipsis.text), ("…", "..."));
        assert_eq!(a.ellipsis.width, 3);
        assert_eq!((u.warning.text, a.warning.text), ("⚠", "!"));
        assert_eq!((u.error.text, a.error.text), ("✖", "x"));
        assert_eq!((u.info.text, a.info.text), ("ℹ", "i"));
        assert_eq!(
            [
                a.boxed.top_left.text,
                a.boxed.top_right.text,
                a.boxed.bottom_left.text,
                a.boxed.bottom_right.text
            ],
            ["+"; 4]
        );
        assert_eq!(Glyphs::for_ascii_only(true), a);
        assert_eq!(Glyphs::for_ascii_only(false), u);
    }
}
