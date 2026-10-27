//! 出力の無害化 (docs/04 §4.9。最終防衛線)。core が Frame を作る前に無害化する (INV-F1) が、eve-tui も、端末へ書く前に
//! すべての文字列を検査する。リリースでも有効 (コストは文字列走査だけ)。

use std::borrow::Cow;

/// 端末タイトルの長さの上限 (文字数、§4.9 の 3)。
pub const MAX_TITLE_CHARS: usize = 256;

/// C0 制御 (U+0000–001F)・DEL (U+007F)・C1 制御 (U+0080–009F) か。
pub fn is_control(c: char) -> bool {
    matches!(c, '\u{0000}'..='\u{001F}' | '\u{007F}' | '\u{0080}'..='\u{009F}')
}

/// 制御文字を `U+FFFD` に置換する。置換した個数も返す (本文は記録しない。NFR-504)。
pub fn replace_controls(s: &str) -> (Cow<'_, str>, usize) {
    if !s.chars().any(is_control) {
        return (Cow::Borrowed(s), 0);
    }
    let mut n = 0;
    let out: String = s
        .chars()
        .map(|c| {
            if is_control(c) {
                n += 1;
                '\u{FFFD}'
            } else {
                c
            }
        })
        .collect();
    (Cow::Owned(out), n)
}

/// 端末タイトル (OSC 0/2) の無害化: 制御文字 (BEL・ST の ESC を含む) を除き、長さを切る。
pub fn sanitize_title(s: &str) -> String {
    s.chars()
        .filter(|c| !is_control(*c))
        .take(MAX_TITLE_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 制御文字は_fffd_に置き換わる() {
        let (s, n) = replace_controls("a\u{1B}[31mb\u{7}\u{7F}\u{85}c");
        assert_eq!(s, "a\u{FFFD}[31mb\u{FFFD}\u{FFFD}\u{FFFD}c");
        assert_eq!(n, 4);
    }

    #[test]
    fn 制御文字が無ければ借用のまま() {
        let (s, n) = replace_controls("日本語 abc 😀 \u{200B}");
        assert!(matches!(s, Cow::Borrowed(_)));
        assert_eq!(n, 0);
    }

    #[test]
    fn タブと改行も制御文字() {
        // 描画の段で展開済みのはずなので、残っていれば置換する
        let (s, n) = replace_controls("a\tb\nc");
        assert_eq!((s.as_ref(), n), ("a\u{FFFD}b\u{FFFD}c", 2));
    }

    #[test]
    fn タイトルは制御文字を除き_256_文字で切る() {
        assert_eq!(sanitize_title("a\u{7}b\u{1B}\\c"), "ab\\c");
        let long: String = "あ".repeat(300);
        assert_eq!(sanitize_title(&long).chars().count(), MAX_TITLE_CHARS);
    }
}
