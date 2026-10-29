//! 単語境界 (docs/03-core-design.md §13.3 日本語補助規則) と単語移動の停止位置 (§5.3)。
//!
//! 単語境界は **文字種 (`ScriptClass`) の遷移** だけで決める。UAX #29 の単語境界は使わない (§13.3 の規則 5)。
//! 分類の単位は書記素で、書記素の先頭のスカラ値の分類をその書記素の分類とする。
//!
//! 分類は標準ライブラリの `char` の判定と、日本語の範囲表で行う (Unicode の Script / General Category の表は持たない):
//! - 空白は `char::is_whitespace`、英数は `char::is_alphanumeric` (L* / N* と Other_Alphabetic)
//! - 書記素の先頭の結合文字 (Mn / Mc) は主な結合文字のブロックだけを `Alnum` とする (孤立した結合文字は稀)

use unicode_segmentation::UnicodeSegmentation;

use crate::text::TextRead;
use crate::types::{ByteOff, ByteRange, LineIdx, Pos};

/// 文字種 (§13.3)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptClass {
    Han,
    Hiragana,
    Katakana,
    Alnum,
    FullwidthAlnum,
    Punct,
    Space,
}

/// 書記素 `g` の文字種。`prev` は直前の書記素の文字種 (行頭は `None`)。
/// 長音 `ー`・踊り字・濁点は、直前が漢字・かな・英数ならそれに従い、ランの先頭では既定の種別になる。
pub fn classify_grapheme(g: &str, prev: Option<ScriptClass>) -> ScriptClass {
    let Some(c) = g.chars().next() else {
        return ScriptClass::Space;
    };
    if c.is_whitespace() {
        return ScriptClass::Space;
    }
    let follow = |default: ScriptClass| match prev {
        Some(ScriptClass::Punct | ScriptClass::Space) | None => default,
        Some(p) => p,
    };
    match c {
        '\u{3005}' => follow(ScriptClass::Han),
        '\u{30FC}' | '\u{30FD}' | '\u{30FE}' | '\u{FF9E}' | '\u{FF9F}' => {
            follow(ScriptClass::Katakana)
        }
        '\u{309D}' | '\u{309E}' => follow(ScriptClass::Hiragana),
        '\u{3006}'
        | '\u{3007}'
        | '\u{30F5}'
        | '\u{30F6}'
        | '\u{3400}'..='\u{4DBF}'
        | '\u{4E00}'..='\u{9FFF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{2E80}'..='\u{2FDF}'
        | '\u{20000}'..='\u{2FA1F}'
        | '\u{30000}'..='\u{323AF}'
        | '\u{3021}'..='\u{3029}'
        | '\u{3038}'..='\u{303B}' => ScriptClass::Han,
        '\u{3041}'..='\u{3096}' | '\u{309F}' => ScriptClass::Hiragana,
        '\u{30A1}'..='\u{30FA}'
        | '\u{30FF}'
        | '\u{31F0}'..='\u{31FF}'
        | '\u{FF66}'..='\u{FF9D}' => ScriptClass::Katakana,
        '\u{FF10}'..='\u{FF19}' | '\u{FF21}'..='\u{FF3A}' | '\u{FF41}'..='\u{FF5A}' => {
            ScriptClass::FullwidthAlnum
        }
        '_' => ScriptClass::Alnum,
        '\u{0300}'..='\u{036F}'
        | '\u{1AB0}'..='\u{1AFF}'
        | '\u{1DC0}'..='\u{1DFF}'
        | '\u{20D0}'..='\u{20FF}'
        | '\u{FE20}'..='\u{FE2F}'
        | '\u{3099}'
        | '\u{309A}' => ScriptClass::Alnum,
        c if c.is_alphanumeric() => ScriptClass::Alnum,
        _ => ScriptClass::Punct,
    }
}

/// 1 行の内容から、`Space` 以外の単位の先頭の位置 (行頭からのバイト) を昇順に返す。
/// 単位: 同じ文字種のラン。ただし `Punct` は 1 書記素ごと、`Space` のランは 1 単位 (先頭は返さない)。
fn unit_starts(line: &str) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut prev: Option<ScriptClass> = None;
    for (i, g) in line.grapheme_indices(true) {
        let class = classify_grapheme(g, prev);
        let new_unit = match prev {
            None => true,
            Some(p) => p != class || class == ScriptClass::Punct,
        };
        if new_unit && class != ScriptClass::Space {
            starts.push(i);
        }
        prev = Some(class);
    }
    starts
}

/// 1 行の内容の単語境界 (行頭からのバイト、昇順。行頭と行末を含む)。文字種が変わる位置と、`Punct` の 1 書記素ごとの位置。
/// 検索の「単語単位」(01 FR-402) が、一致の両端がこの境界上にあるかを調べるのに使う。
pub fn word_boundaries(line: &str) -> Vec<usize> {
    let mut out = vec![0];
    let mut prev: Option<ScriptClass> = None;
    for (i, g) in line.grapheme_indices(true) {
        let class = classify_grapheme(g, prev);
        if i > 0 && prev.is_some_and(|p| p != class || class == ScriptClass::Punct) {
            out.push(i);
        }
        prev = Some(class);
    }
    if !line.is_empty() {
        out.push(line.len());
    }
    out
}

fn line_of(text: &dyn TextRead, pos: Pos) -> (LineIdx, usize, usize) {
    let l = text.byte_to_line(pos);
    (l, text.line_to_byte(l).0, text.line_end_byte(l).0)
}

fn line_text(text: &dyn TextRead, start: usize, end: usize) -> String {
    text.slice_to_string(ByteRange::new(ByteOff(start), ByteOff(end)))
}

/// `cursor.move_word_right` の停止位置 (§5.3 / §13.3)。`pos` より後ろの、`Space` 以外の単位の先頭のうち最も近い位置。
/// 行内に無ければ行末。行末にいれば次の行の行頭 (最終行の行末では動かない)。
pub fn next_word_stop(text: &dyn TextRead, pos: Pos) -> Pos {
    let pos = ByteOff(pos.0.min(text.len_bytes()));
    let (l, ls, le) = line_of(text, pos);
    if pos.0 >= le {
        return if l.0 + 1 < text.len_lines() {
            ByteOff(text.line_to_byte(LineIdx(l.0 + 1)).0)
        } else {
            ByteOff(le)
        };
    }
    let starts = unit_starts(&line_text(text, ls, le));
    let rel = pos.0 - ls;
    let i = starts.partition_point(|&s| s <= rel);
    ByteOff(starts.get(i).map_or(le, |s| ls + s))
}

/// `cursor.move_word_left` の停止位置。`pos` より前の、`Space` 以外の単位の先頭のうち最も近い位置。
/// 行内に無ければ行頭。行頭にいれば前の行の行末 (先頭行の行頭では動かない)。
pub fn prev_word_stop(text: &dyn TextRead, pos: Pos) -> Pos {
    let pos = ByteOff(pos.0.min(text.len_bytes()));
    let (l, ls, le) = line_of(text, pos);
    if pos.0 <= ls {
        return if l.0 > 0 {
            ByteOff(text.line_end_byte(LineIdx(l.0 - 1)).0)
        } else {
            ByteOff(ls)
        };
    }
    let starts = unit_starts(&line_text(text, ls, le));
    let rel = pos.0.min(le) - ls;
    let i = starts.partition_point(|&s| s < rel);
    ByteOff(if i == 0 { ls } else { ls + starts[i - 1] })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::Text;

    /// 1 行だけの文書で、行頭から Ctrl+→ を繰り返したときの停止位置を書記素数で返す。
    fn right_stops(s: &str) -> Vec<usize> {
        let t = Text::from(s);
        let mut out = Vec::new();
        let mut p = ByteOff(0);
        loop {
            let n = next_word_stop(&t, p);
            if n == p {
                break;
            }
            out.push(s[..n.0].graphemes(true).count());
            p = n;
        }
        out
    }

    /// 行末から Ctrl+← を繰り返したときの停止位置。
    fn left_stops(s: &str) -> Vec<usize> {
        let t = Text::from(s);
        let mut out = Vec::new();
        let mut p = ByteOff(s.len());
        loop {
            let n = prev_word_stop(&t, p);
            if n == p {
                break;
            }
            out.push(s[..n.0].graphemes(true).count());
            p = n;
        }
        out
    }

    /// 03 §13.3 の期待結果の表 (FR-302 の受入条件)。
    // verifies: AC-FR-302-1
    #[test]
    fn 期待結果の表() {
        let rows: &[(&str, &[usize], &[usize])] = &[
            ("日本語を勉強する", &[3, 4, 6, 8], &[6, 4, 3, 0]),
            ("食べる", &[1, 3], &[1, 0]),
            ("東京の天気", &[2, 3, 5], &[3, 2, 0]),
            ("コーヒーを飲む", &[4, 5, 6, 7], &[6, 5, 4, 0]),
            ("ＡＢＣ１２３とabc", &[6, 7, 10], &[7, 6, 0]),
            ("ＡＢＣ１２３と abc", &[6, 8, 11], &[8, 6, 0]),
            ("ＡＢＣabc", &[3, 6], &[3, 0]),
            ("佃々木ヶ原", &[5], &[0]),
            ("データベース設計", &[6, 8], &[6, 0]),
            ("ｱｲｳｴｵ", &[5], &[0]),
            ("「こんにちは」", &[1, 6, 7], &[6, 1, 0]),
            ("行きます。次に", &[1, 4, 5, 6, 7], &[6, 5, 4, 1, 0]),
            ("hello world", &[6, 11], &[6, 0]),
            ("foo_bar baz", &[8, 11], &[8, 0]),
            ("foo-bar", &[3, 4, 7], &[4, 3, 0]),
            ("foo_bar.baz()", &[7, 8, 11, 12, 13], &[12, 11, 8, 7, 0]),
            ("123,456", &[3, 4, 7], &[4, 3, 0]),
            ("version2.0", &[8, 9, 10], &[9, 8, 0]),
            ("don't", &[3, 4, 5], &[4, 3, 0]),
            ("日本語English", &[3, 10], &[3, 0]),
            ("第3回", &[1, 2, 3], &[2, 1, 0]),
            ("ひらがなカタカナ", &[4, 8], &[4, 0]),
            ("ﾌﾞﾛｸﾞ記事", &[3, 5], &[3, 0]),
            ("お茶", &[1, 2], &[1, 0]),
            ("😀😀abc", &[1, 2, 5], &[2, 1, 0]),
            ("\u{3000}全角スペース\u{3000}", &[1, 3, 8], &[3, 1, 0]),
            (
                "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}abc",
                &[1, 4],
                &[1, 0],
            ),
            ("한국어 텍스트", &[4, 7], &[4, 0]),
            ("한국어abc", &[6], &[0]),
            ("foo\u{2060}bar", &[3, 4, 7], &[4, 3, 0]),
        ];
        for (input, right, left) in rows {
            assert_eq!(right_stops(input), *right, "→ {input}");
            assert_eq!(left_stops(input), *left, "← {input}");
        }
    }

    #[test]
    fn 行をまたぐ移動() {
        let t = Text::from("ab  \r\n  cd\nef");
        // 行末 (4) → 次の行頭 (6)
        assert_eq!(next_word_stop(&t, ByteOff(4)), ByteOff(6));
        // 行頭の空白は読み飛ばす: 次の行頭 (6) から 'cd' (8)
        assert_eq!(next_word_stop(&t, ByteOff(6)), ByteOff(8));
        // 行頭 → 前の行の行末
        assert_eq!(prev_word_stop(&t, ByteOff(6)), ByteOff(4));
        assert_eq!(
            prev_word_stop(&t, ByteOff(8)),
            ByteOff(6),
            "行内に単語が無ければ行頭"
        );
        // 文書の端では動かない
        assert_eq!(prev_word_stop(&t, ByteOff(0)), ByteOff(0));
        assert_eq!(
            next_word_stop(&t, ByteOff(t.len_bytes())),
            ByteOff(t.len_bytes())
        );
    }

    #[test]
    fn 記号は連続しても_1_文字ずつ止まる() {
        assert_eq!(right_stops("()"), [1, 2]);
        assert_eq!(left_stops("()"), [1, 0]);
        assert_eq!(right_stops("a.b"), [1, 2, 3]);
    }

    #[test]
    fn 分類の例外() {
        use ScriptClass::*;
        assert_eq!(classify_grapheme("々", None), Han, "ランの先頭では漢字");
        assert_eq!(
            classify_grapheme("々", Some(Hiragana)),
            Hiragana,
            "直前に従う"
        );
        assert_eq!(classify_grapheme("ー", None), Katakana);
        assert_eq!(classify_grapheme("ー", Some(Hiragana)), Hiragana);
        assert_eq!(
            classify_grapheme("ー", Some(Punct)),
            Katakana,
            "記号の後はランの先頭"
        );
        assert_eq!(classify_grapheme("\u{3000}", None), Space);
        assert_eq!(classify_grapheme("_", None), Alnum);
        assert_eq!(classify_grapheme("한", None), Alnum, "ハングルは英数と同じ");
        assert_eq!(classify_grapheme("\u{200B}", None), Punct, "Cf は Punct");
        assert_eq!(
            classify_grapheme("🇯🇵", None),
            Punct,
            "国旗は 1 書記素で記号"
        );
        assert_eq!(classify_grapheme("！", None), Punct, "全角の記号");
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::word_boundaries;

    #[test]
    fn 文字種が変わる位置が境界になる() {
        assert_eq!(word_boundaries(""), vec![0]);
        assert_eq!(word_boundaries("foo"), vec![0, 3]);
        assert_eq!(word_boundaries("foo bar"), vec![0, 3, 4, 7]);
        // 記号は 1 書記素ごと
        assert_eq!(word_boundaries("a..b"), vec![0, 1, 2, 3, 4]);
        // 漢字・かな・カタカナ・全角英数は別の単位
        assert_eq!(word_boundaries("日本語です"), vec![0, 9, 15]);
        assert_eq!(word_boundaries("ＡＢＣabc"), vec![0, 9, 12]);
    }
}
