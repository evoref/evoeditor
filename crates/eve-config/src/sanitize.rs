//! ファイル由来の文字列の無害化 (docs/07 §4.3・§17.4、01 NFR-503)。
//!
//! 設定ファイル・キーマップ・テーマファイル・言語ファイルから来た文字列 (キーの名前・書かれた値・`[meta] language` の値・
//! パーサの説明) を診断に埋める前に、制御文字・双方向の書式文字・方向の印など (07 §17.4 の検査 4 の範囲、
//! [`forbidden_char`](crate::forbidden_char)) を `<U+XXXX>` の表記に直す。画面 (Frame) に載せる前・ログに書く前・
//! `--check-config` で標準出力へ出す前のどれも、ここの関数を使う。

use crate::chars::forbidden_char;

/// 診断に埋めるファイル由来の文字列の長さの上限 (文字数。07 §4.3)。
pub const UNTRUSTED_MAX_CHARS: usize = 64;

/// 省いたことを示す末尾の記号。
const ELLIPSIS: char = '…';

/// 使えない文字を `<U+XXXX>` (16 進の大文字、4 桁以上) に直す。長さは切らない (`--check-config` の行の全体に使う)。
pub fn escape_untrusted(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        push_escaped(&mut out, c);
    }
    out
}

/// 診断に埋める文字列の無害化: 使えない文字を `<U+XXXX>` に直し、直した後の長さが
/// [`UNTRUSTED_MAX_CHARS`] 文字を超えるなら、末尾の `…` を含めて上限の文字数に収まるように切る。
/// 書記素 (eve-text の境界) の途中と、`<U+XXXX>` の表記の途中では切らない。
pub fn sanitize_untrusted(s: &str) -> String {
    let mut out = String::new();
    let mut len = 0usize;
    let mut piece = String::new();
    let mut rest = s;
    while !rest.is_empty() {
        let g = eve_text::first_grapheme(rest);
        // 空でない文字列の先頭の書記素は空でない。念のため 1 文字ずつ進める形にも備える
        let g = if g.is_empty() {
            rest.chars().next().map_or(rest, |c| &rest[..c.len_utf8()])
        } else {
            g
        };
        rest = &rest[g.len()..];
        piece.clear();
        for c in g.chars() {
            push_escaped(&mut piece, c);
        }
        let n = piece.chars().count();
        if len + n > UNTRUSTED_MAX_CHARS - 1 {
            // これが最後の書記素で、上限にちょうど収まるなら、切らずに足す
            if rest.is_empty() && len + n <= UNTRUSTED_MAX_CHARS {
                out.push_str(&piece);
            } else {
                out.push(ELLIPSIS);
            }
            return out;
        }
        out.push_str(&piece);
        len += n;
    }
    out
}

fn push_escaped(out: &mut String, c: char) {
    if forbidden_char(c).is_some() {
        out.push_str(&format!("<U+{:04X}>", u32::from(c)));
    } else {
        out.push(c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // verifies: NFR-503
    #[test]
    fn 制御文字と書式文字を_u_表記に直す() {
        let cases: &[(&str, &str)] = &[
            ("abc", "abc"),
            ("日本語", "日本語"),
            ("a\u{1B}[31mb", "a<U+001B>[31mb"),
            ("\u{9B}2J", "<U+009B>2J"),
            ("x\ty\nz", "x<U+0009>y<U+000A>z"),
            ("\u{202E}abc\u{2066}", "<U+202E>abc<U+2066>"),
            ("\u{200E}\u{200F}\u{061C}", "<U+200E><U+200F><U+061C>"),
            ("\u{7F}", "<U+007F>"),
            ("\u{2028}", "<U+2028>"),
            ("\u{10FFFF}", "<U+10FFFF>"),
            ("\u{E000}", "<U+E000>"),
            // ZWJ の絵文字の列は直さない
            ("👨\u{200D}👩", "👨\u{200D}👩"),
        ];
        for &(input, want) in cases {
            assert_eq!(escape_untrusted(input), want, "{input:?}");
            assert_eq!(sanitize_untrusted(input), want, "{input:?}");
        }
    }

    // verifies: NFR-503
    #[test]
    fn 長さ_64_文字で切る() {
        let exact = "a".repeat(64);
        assert_eq!(
            sanitize_untrusted(&exact),
            exact,
            "ちょうど 64 文字は切らない"
        );
        let over = "a".repeat(65);
        let got = sanitize_untrusted(&over);
        assert_eq!(got.chars().count(), 64);
        assert!(got.ends_with('…'), "{got}");
        assert_eq!(got, format!("{}…", "a".repeat(63)));
        // 全角も文字数で数える
        let wide = "あ".repeat(100);
        assert_eq!(sanitize_untrusted(&wide).chars().count(), 64);
        // `<U+XXXX>` の途中では切らない
        let esc = format!("{}\u{1B}{}", "a".repeat(60), "b".repeat(10));
        let got = sanitize_untrusted(&esc);
        assert_eq!(got, format!("{}…", "a".repeat(60)));
        assert!(!got.contains('<'), "{got}");
        // 最後の 1 つが表記でも、上限に収まるなら切らない
        let tail = format!("{}\u{1B}", "a".repeat(56));
        assert_eq!(
            sanitize_untrusted(&tail),
            format!("{}<U+001B>", "a".repeat(56))
        );
        // 長い制御文字の列も上限を超えない
        let ctl = "\u{1B}".repeat(1000);
        let got = sanitize_untrusted(&ctl);
        assert!(got.chars().count() <= 64, "{got}");
        assert!(!got.contains('\u{1B}'));
        // 切らない関数は長さを変えない
        assert_eq!(escape_untrusted(&over), over);
        // 書記素の途中では切らない (基底と結合文字を分けない)
        let marks = format!("{}e\u{0301}\u{0302}x", "a".repeat(61));
        assert_eq!(sanitize_untrusted(&marks), format!("{}…", "a".repeat(61)));
        let fits = format!("{}e\u{0301}\u{0302}", "a".repeat(61 - 1));
        assert_eq!(sanitize_untrusted(&fits), fits, "63 文字に収まれば切らない");
        // 結合文字が際限なく続く 1 つの書記素は、`…` だけにする
        let huge = format!("e{}", "\u{0301}".repeat(10_000));
        assert_eq!(sanitize_untrusted(&huge), "…");
    }
}
