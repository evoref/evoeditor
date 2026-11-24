//! 単語境界 (docs/03-core-design.md §13.3 日本語補助規則) と単語移動の停止位置 (§5.3)。
//!
//! 単語境界は **文字種 (`ScriptClass`) の遷移** だけで決める。UAX #29 の単語境界は使わない (§13.3 の規則 5)。
//! 分類の単位は書記素で、書記素の先頭のスカラ値の分類をその書記素の分類とする。
//!
//! 分類は標準ライブラリの `char` の判定と、日本語の範囲表で行う (Unicode の Script / General Category の表は持たない):
//! - 空白は `char::is_whitespace`、英数は `char::is_alphanumeric` (L* / N* と `Other_Alphabetic`)
//! - 書記素の先頭の結合文字 (Mn / Mc) は主な結合文字のブロックだけを `Alnum` とする (孤立した結合文字は稀)
//! - 不正バイトのエスケープ文字 (`U+10FF00..=U+10FFFF`) は `InvalidByte` (§13.3 の先頭の項、§8.3.1)。書記素の境界
//!   (§13.1) と同じく、文書の `pua_escape` は見ない

use crate::grapheme::{is_escape_char, str_graphemes};
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
    /// 不正バイトのエスケープ文字 (03 §8.3.1)。連続する並びを 1 単語とし、どの文字種ともつながらない。
    InvalidByte,
}

/// 書記素 `g` の文字種。`prev` は直前の書記素の文字種 (行頭は `None`)。
/// 長音 `ー`・踊り字・濁点は、直前が漢字・かな・英数ならそれに従い、ランの先頭では既定の種別になる
/// (直前が記号・空白・不正バイトのときもランの先頭と同じ)。
pub fn classify_grapheme(g: &str, prev: Option<ScriptClass>) -> ScriptClass {
    let Some(c) = g.chars().next() else {
        return ScriptClass::Space;
    };
    if c.is_whitespace() {
        return ScriptClass::Space;
    }
    if is_escape_char(c) {
        return ScriptClass::InvalidByte;
    }
    let follow = |default: ScriptClass| match prev {
        Some(ScriptClass::Punct | ScriptClass::Space | ScriptClass::InvalidByte) | None => default,
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
    for (i, g) in str_graphemes(line) {
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
    for (i, g) in str_graphemes(line) {
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

fn floor_char(text: &dyn TextRead, mut b: usize) -> usize {
    while !text.is_char_boundary(ByteOff(b)) {
        b -= 1;
    }
    b
}

fn ceil_char(text: &dyn TextRead, mut b: usize) -> usize {
    while !text.is_char_boundary(ByteOff(b)) {
        b += 1;
    }
    b
}

/// 前の文字種を決めるために `next_word_stop_near` が `pos` の手前から読むバイト数 (長音・踊り字の継承に使う)。
const NEAR_CONTEXT: usize = 16;

/// `next_word_stop` の窓付きの版 (長大行、03 §11.5.1 / §14.4 の「使う側」の (d))。行全体を読まず、`pos` の後ろ `window` バイト
/// (char の境界へ広げる) と手前の数バイト (前の文字種) だけを見る。窓の中に停止位置が無ければ窓の端に止まる (窓が行末に届いていれば行末)。
pub fn next_word_stop_near(text: &dyn TextRead, pos: Pos, window: usize) -> Pos {
    let pos = ByteOff(pos.0.min(text.len_bytes()));
    let (_, ls, le) = line_of(text, pos);
    if pos.0 >= le {
        return next_word_stop(text, pos);
    }
    let from = floor_char(text, pos.0.saturating_sub(NEAR_CONTEXT).max(ls));
    let s = line_text(text, from, ceil_char(text, (pos.0 + window).min(le)));
    let starts = unit_starts(&s);
    let rel = pos.0 - from;
    let i = starts.partition_point(|&st| st <= rel);
    ByteOff(starts.get(i).map_or(from + s.len(), |st| from + st))
}

/// `prev_word_stop` の窓付きの版 (03 §11.5.1)。`pos` の手前 `window` バイトだけを見る。窓の中に停止位置が無ければ窓の端
/// (窓が行頭に届いていれば行頭) に止まる。
pub fn prev_word_stop_near(text: &dyn TextRead, pos: Pos, window: usize) -> Pos {
    let pos = ByteOff(pos.0.min(text.len_bytes()));
    let (_, ls, le) = line_of(text, pos);
    if pos.0 <= ls {
        return prev_word_stop(text, pos);
    }
    let end = pos.0.min(le);
    let from = floor_char(text, end.saturating_sub(window).max(ls));
    let s = line_text(text, from, end);
    let starts = unit_starts(&s);
    let rel = end - from;
    let i = starts.partition_point(|&st| st < rel);
    ByteOff(if i == 0 { from } else { from + starts[i - 1] })
}

/// `word_at` が最初に読む窓 (`pos` の前後のバイト数)。単語が窓の端に届いたら倍にして読み直す。
const WORD_AT_FIRST_WINDOW: usize = 256;

/// カーソルの位置の単語 (03 §4.5.1 の MCUR-2: §13.3 の境界で区切った同じ文字種の並び。`Punct` は 1 書記素)。
/// カーソルの直後の書記素の単語を優先し、無ければ (直後が空白・改行・行末) 直前の書記素の単語。空白・改行の上なら `None`。
///
/// 行の全体は読まない: `pos` の前後 256 バイトから読み始め、単語が読んだ窓の端 (行の端でないところ) に届いたら窓を倍にして
/// 読み直す。窓は `max_window` で止め、それより長い単語は窓の中の部分を返す (長大行で、カーソルごとに行の全体を読まない。
/// 03 §11.5.1 の窓と同じ考え)。
pub fn word_at(text: &dyn TextRead, pos: Pos, max_window: usize) -> Option<ByteRange> {
    unit_near(text, pos, max_window, word_in)
}

/// 位置 `pos` の §13.3 の単位 (同じ文字種のラン・空白のラン・記号 1 書記素。03 §5.8.2 の CLK-2 のダブルクリックの単位)。
/// `pos` の直後の書記素を含む単位で、`pos` が行末 (直後が区切りか文書の末尾) なら直前の書記素の単位。空の行では `None`。
/// `word_at` と違い、空白のランも単位にする。窓の読み方は `word_at` と同じ (`max_window` まで)。
pub fn unit_at(text: &dyn TextRead, pos: Pos, max_window: usize) -> Option<ByteRange> {
    let at = ByteOff(pos.0.min(text.len_bytes()));
    let (_, _, le) = line_of(text, at);
    let at_line_end = at.0 >= le;
    unit_near(text, at, max_window, |s, rel| unit_in(s, rel, at_line_end))
}

/// 位置 `pos` の直前の書記素を含む §13.3 の単位 (03 §5.8.2 の CLK-2: 位置が表示行の末尾 (`Before` に帰属する折り返し位置)
/// のとき)。行頭 (直前の書記素が無い) では `None`。窓の読み方は `unit_at` と同じ (`max_window` まで)。
pub fn unit_before(text: &dyn TextRead, pos: Pos, max_window: usize) -> Option<ByteRange> {
    let at = ByteOff(pos.0.min(text.len_bytes()));
    unit_near(text, at, max_window, |s, rel| unit_in(s, rel, true))
}

/// `pos` の行の窓を読み、`pick(窓の文字列, 窓の中の pos)` が選んだ単位を返す (`word_at` / `unit_at` の本体)。単位が窓の端
/// (行の端でないところ) に届いたら窓を倍にして読み直し、`max_window` で止める。
fn unit_near(
    text: &dyn TextRead,
    pos: Pos,
    max_window: usize,
    pick: impl Fn(&str, usize) -> Option<(usize, usize)>,
) -> Option<ByteRange> {
    let pos = ByteOff(pos.0.min(text.len_bytes()));
    let (_, ls, le) = line_of(text, pos);
    let pos = pos.0.min(le);
    let mut window = WORD_AT_FIRST_WINDOW.min(max_window.max(1));
    loop {
        let from = floor_char(text, pos.saturating_sub(window).max(ls));
        let to = ceil_char(text, pos.saturating_add(window).min(le));
        let s = line_text(text, from, to);
        let (a, b) = pick(&s, pos - from)?;
        // 窓が行の途中で始まるとき、最初の書記素は信用しない: 書記素の途中で切れていることがあり、直前の文字種に従う文字
        // (`ー`・`々` など、§13.3) を直前なしで分類している。単位がその書記素の後ろまで届いていれば、窓を広げて読み直す
        let lead = if from > ls {
            str_graphemes(&s).next().map_or(0, |(i, g)| i + g.len())
        } else {
            0
        };
        let cut_left = from > ls && a <= lead;
        let cut_right = b == s.len() && to < le;
        if (cut_left || cut_right) && window < max_window {
            window = window.saturating_mul(2).min(max_window);
            continue;
        }
        return Some(ByteRange::new(ByteOff(from + a), ByteOff(from + b)));
    }
}

/// 1 行の部分 `s` の書記素ごとの (始まり, 終わり, 文字種)。
fn classified(s: &str) -> Vec<(usize, usize, ScriptClass)> {
    let mut units = Vec::new();
    let mut prev: Option<ScriptClass> = None;
    for (i, g) in str_graphemes(s) {
        let class = classify_grapheme(g, prev);
        units.push((i, i + g.len(), class));
        prev = Some(class);
    }
    units
}

/// `units` の `k` 番目の書記素を含む単位 (同じ文字種のラン。`Punct` は 1 書記素) の (始まり, 終わり)。
fn run_around(units: &[(usize, usize, ScriptClass)], k: usize) -> (usize, usize) {
    let class = units[k].2;
    if class == ScriptClass::Punct {
        return (units[k].0, units[k].1);
    }
    let mut first = k;
    while first > 0 && units[first - 1].2 == class {
        first -= 1;
    }
    let mut last = k;
    while last + 1 < units.len() && units[last + 1].2 == class {
        last += 1;
    }
    (units[first].0, units[last].1)
}

/// 1 行の部分 `s` の中で、位置 `rel` の単語 (`word_at` の本体。`s` の中の (始まり, 終わり))。
fn word_in(s: &str, rel: usize) -> Option<(usize, usize)> {
    let units = classified(s);
    let after = units
        .iter()
        .position(|&(a, _, c)| a == rel && c != ScriptClass::Space);
    let before = || {
        units
            .iter()
            .position(|&(_, b, c)| b == rel && c != ScriptClass::Space)
    };
    let k = after.or_else(before)?;
    Some(run_around(&units, k))
}

/// 1 行の部分 `s` の中で、位置 `rel` の単位 (`unit_at` の本体)。`at_line_end` なら直前の書記素、そうでなければ `rel` を含む
/// (直後の) 書記素の単位。
fn unit_in(s: &str, rel: usize, at_line_end: bool) -> Option<(usize, usize)> {
    let units = classified(s);
    let k = if at_line_end {
        units.iter().position(|&(_, b, _)| b == rel)
    } else {
        units.iter().position(|&(a, b, _)| a <= rel && rel < b)
    }?;
    Some(run_around(&units, k))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::Text;

    /// 書記素の数 (エスケープ文字の前後を境界とする、§13.1)。
    fn graphemes_count(s: &str) -> usize {
        str_graphemes(s).count()
    }

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
            out.push(graphemes_count(&s[..n.0]));
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
            out.push(graphemes_count(&s[..n.0]));
            p = n;
        }
        out
    }

    // verifies: FR-302
    #[test]
    fn near_matches_full_within_window_and_stops_at_edge() {
        for s in [
            "日本語を勉強する abc",
            "foo_bar.baz() コーヒー",
            "😀😀abc ｱｲｳ",
        ] {
            let t = Text::from(
                format!(
                    "x
{s}
y"
                )
                .as_str(),
            );
            let ls = 2;
            for (i, _) in s.char_indices().chain([(s.len(), ' ')]) {
                let p = ByteOff(ls + i);
                assert_eq!(
                    next_word_stop_near(&t, p, 1 << 20),
                    next_word_stop(&t, p),
                    "→ {s} {i}"
                );
                assert_eq!(
                    prev_word_stop_near(&t, p, 1 << 20),
                    prev_word_stop(&t, p),
                    "← {s} {i}"
                );
            }
        }
        // 窓の中に停止位置が無い: 窓の端 (char の境界へ広げる)
        let long = "a".repeat(100);
        let t = Text::from(long.as_str());
        assert_eq!(next_word_stop_near(&t, ByteOff(10), 30), ByteOff(40));
        assert_eq!(prev_word_stop_near(&t, ByteOff(90), 30), ByteOff(60));
        let wide = "あ".repeat(100);
        let t = Text::from(wide.as_str());
        assert_eq!(next_word_stop_near(&t, ByteOff(0), 10), ByteOff(12));
        assert_eq!(prev_word_stop_near(&t, ByteOff(300), 10), ByteOff(288));
        // 窓の中に境界があれば、そこに止まる
        let t = Text::from(format!("{} b{}", "a".repeat(50), "c".repeat(50)).as_str());
        assert_eq!(next_word_stop_near(&t, ByteOff(45), 30), ByteOff(51));
        assert_eq!(prev_word_stop_near(&t, ByteOff(60), 30), ByteOff(51));
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

    /// 不正バイトのエスケープ文字の並びは 1 単語で、前後のどの文字種ともつながらない (03 §13.3 の先頭の項、§8.3.1)。
    // spec: 03 §13.3
    #[test]
    fn エスケープ文字の並びは_1_単語() {
        let e = "\u{10FF80}";
        let f = "\u{10FF9A}";
        // 英数 → 並び → 英数 (書記素の数で a b c E E d e f)
        let s = format!("abc{e}{f}def");
        assert_eq!(right_stops(&s), [3, 5, 8]);
        assert_eq!(left_stops(&s), [5, 3, 0]);
        assert_eq!(word_boundaries(&s), [0, 3, 11, 14]);
        // 記号・空白・かな・漢字の隣でも、並びは 1 単位
        let s = format!("({e}{f}) {e}あ漢");
        assert_eq!(right_stops(&s), [1, 3, 5, 6, 7, 8]);
        assert_eq!(left_stops(&s), [7, 6, 5, 3, 1, 0]);
        // 直後の結合文字は別の書記素 (§13.1)。先頭の結合文字は英数として次の文字とつながる
        let s = format!("{e}\u{301}x");
        assert_eq!(right_stops(&s), [1, 3]);
        assert_eq!(word_boundaries(&s), [0, 4, 7]);
        // 長音・踊り字は並びに続けない (ランの先頭と同じ扱い)
        assert_eq!(
            classify_grapheme("ー", Some(ScriptClass::InvalidByte)),
            ScriptClass::Katakana
        );
        assert_eq!(
            classify_grapheme("々", Some(ScriptClass::InvalidByte)),
            ScriptClass::Han
        );
        let s = format!("{e}ーア");
        assert_eq!(right_stops(&s), [1, 3]);
        // 範囲の両端と、範囲の外の私用文字
        assert_eq!(
            classify_grapheme("\u{10FF00}", None),
            ScriptClass::InvalidByte
        );
        assert_eq!(
            classify_grapheme("\u{10FFFF}", None),
            ScriptClass::InvalidByte
        );
        assert_eq!(classify_grapheme("\u{10FEFF}", None), ScriptClass::Punct);
        assert_eq!(classify_grapheme("\u{E000}", None), ScriptClass::Punct);
        // 窓付きの版 (長大行) も同じ
        let t = Text::from(format!("x\nabc{e}{f}def {e}\ny").as_str());
        for p in 2..t.len_bytes() - 2 {
            if !t.is_char_boundary(ByteOff(p)) {
                continue;
            }
            assert_eq!(
                next_word_stop_near(&t, ByteOff(p), 1 << 20),
                next_word_stop(&t, ByteOff(p)),
                "→ {p}"
            );
            assert_eq!(
                prev_word_stop_near(&t, ByteOff(p), 1 << 20),
                prev_word_stop(&t, ByteOff(p)),
                "← {p}"
            );
        }
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

#[cfg(test)]
mod word_at_tests {
    use super::{unit_at, unit_before, word_at};
    use crate::text::Text;
    use crate::types::ByteOff;

    /// `s` の `|` の位置の単語 (`|` を除いた本文の中の文字列)。
    fn at(marked: &str) -> Option<String> {
        let p = marked.find('|').expect("| がある");
        let s = marked.replacen('|', "", 1);
        let t = Text::from(s.as_str());
        word_at(&t, ByteOff(p), 1 << 16).map(|r| s[r.start.0..r.end.0].to_owned())
    }

    #[test]
    fn 直後の文字の単語を優先し_無ければ直前() {
        assert_eq!(at("foo |bar").as_deref(), Some("bar"));
        assert_eq!(at("foo| bar").as_deref(), Some("foo"), "直後が空白なら直前");
        assert_eq!(at("fo|o bar").as_deref(), Some("foo"), "単語の内側");
        assert_eq!(
            at("foo|.bar").as_deref(),
            Some("."),
            "直後の記号は 1 書記素の単語"
        );
        assert_eq!(at("foo.|bar").as_deref(), Some("bar"));
        assert_eq!(at("foo|\nbar").as_deref(), Some("foo"), "行末");
        assert_eq!(at("foo\n|bar").as_deref(), Some("bar"), "行頭");
        assert_eq!(at("snake_|case").as_deref(), Some("snake_case"));
    }

    #[test]
    fn 空白と改行の上には単語が無い() {
        assert_eq!(at("foo  |  bar"), None);
        assert_eq!(at("|"), None, "空の文書");
        assert_eq!(at("a\n|\nb"), None, "空の行");
        assert_eq!(at("\t|\t"), None);
        assert_eq!(at("\u{3000}|\u{3000}"), None, "全角の空白");
    }

    #[test]
    fn 日本語は文字種の境界で区切る() {
        // 03 §4.5.1 の MCUR-2 (d) の例
        assert_eq!(at("設|定ファイル").as_deref(), Some("設定"));
        assert_eq!(
            at("再設|定").as_deref(),
            Some("再設定"),
            "漢字の連なりは 1 単語"
        );
        assert_eq!(at("|ファイル").as_deref(), Some("ファイル"));
        assert_eq!(
            at("ファイルを|開く").as_deref(),
            Some("開"),
            "直後の漢字 (送り仮名は別の文字種)"
        );
        assert_eq!(
            at("ファイルを|").as_deref(),
            Some("を"),
            "行末なら直前のかな"
        );
        assert_eq!(
            at("コーヒ|ー").as_deref(),
            Some("コーヒー"),
            "長音は直前の文字種に従う"
        );
    }

    #[test]
    fn 窓の左端が直前の文字種に従う文字や書記素の途中でも途中から始めない() {
        // 窓 (256 バイト) の左端が `ー` (直前のかなに従う) に当たる: `ー` を直前なしでカタカナと分類しない
        let s = format!("{}ー{} x", "ひ".repeat(10), "ひ".repeat(90));
        let t = Text::from(s.as_str());
        let pos = ByteOff(30 + 3 + 255);
        assert!(s.is_char_boundary(pos.0));
        let r = word_at(&t, pos, 1 << 16).expect("単語");
        assert_eq!((r.start.0, r.end.0), (0, 303), "`ー` を含むひらがなの並び");
        // 窓の左端が書記素 (か + 濁点 2 つ) の途中 (濁点) に当たる
        let g = "か\u{3099}\u{3099}";
        let s = format!("{} tail", g.repeat(60));
        let t = Text::from(s.as_str());
        let r = word_at(&t, ByteOff(9 * 50), 1 << 16).expect("単語");
        assert_eq!(
            (r.start.0, r.end.0),
            (0, 9 * 60),
            "書記素の途中から始めない"
        );
    }

    #[test]
    fn 長い単語は窓を広げて全体を返し_上限で止める() {
        let long = "a".repeat(5000);
        let s = format!("x {long} y");
        let t = Text::from(s.as_str());
        let mid = ByteOff(2 + 2500);
        let r = word_at(&t, mid, 1 << 16).expect("単語");
        assert_eq!(
            (r.start.0, r.end.0),
            (2, 2 + 5000),
            "256 バイトの窓を広げる"
        );
        let r = word_at(&t, mid, 1000).expect("単語");
        assert_eq!(
            (r.start.0, r.end.0),
            (2 + 1500, 2 + 3500),
            "上限の窓の中の部分"
        );
    }

    /// `s` の `|` の位置の単位 (`unit_at`。03 §5.8.2 の CLK-2)。
    fn unit(marked: &str, window: usize) -> Option<String> {
        let p = marked.find('|').expect("| がある");
        let s = marked.replacen('|', "", 1);
        let t = Text::from(s.as_str());
        unit_at(&t, ByteOff(p), window).map(|r| s[r.start.0..r.end.0].to_owned())
    }

    #[test]
    fn 単位は直後の書記素を含み_行末なら直前で_空白のランも単位() {
        let u = |m: &str| unit(m, 1 << 16);
        assert_eq!(u("foo |bar").as_deref(), Some("bar"));
        assert_eq!(u("fo|o bar").as_deref(), Some("foo"));
        assert_eq!(u("foo|  bar").as_deref(), Some("  "), "空白のラン");
        assert_eq!(u("foo|.bar").as_deref(), Some("."), "記号は 1 書記素");
        assert_eq!(u("foo|\nbar").as_deref(), Some("foo"), "行末は直前");
        assert_eq!(u("foo  |\nbar").as_deref(), Some("  "), "行末の空白");
        assert_eq!(u("foo|\r\nbar").as_deref(), Some("foo"), "CRLF の前");
        assert_eq!(u("foo\n|bar").as_deref(), Some("bar"), "行頭");
        assert_eq!(u("a\n|\nb"), None, "空の行");
        assert_eq!(u("|"), None, "空の文書");
        assert_eq!(u("a\n|"), None, "区切りで終わる文書の最後の空の行");
        assert_eq!(u("食|べる").as_deref(), Some("べる"));
        assert_eq!(u("コ|ーヒー").as_deref(), Some("コーヒー"));
        assert_eq!(
            u("x|\u{3000}\u{3000}y").as_deref(),
            Some("\u{3000}\u{3000}"),
            "全角の空白"
        );
        assert_eq!(
            u("|e\u{301}tude x").as_deref(),
            Some("e\u{301}tude"),
            "結合文字"
        );
    }

    /// `s` の `|` の位置の直前の書記素の単位 (`unit_before`。03 §5.8.2 の CLK-2 の表示行の末尾)。
    fn unit_left(marked: &str) -> Option<String> {
        let p = marked.find('|').expect("| がある");
        let s = marked.replacen('|', "", 1);
        let t = Text::from(s.as_str());
        unit_before(&t, ByteOff(p), 1 << 16).map(|r| s[r.start.0..r.end.0].to_owned())
    }

    #[test]
    fn 直前の単位は位置の前の書記素を含む() {
        assert_eq!(unit_left("foo |bar").as_deref(), Some(" "), "空白の後");
        assert_eq!(
            unit_left("foo|bar").as_deref(),
            Some("foobar"),
            "単語の途中"
        );
        assert_eq!(unit_left("fo|o bar").as_deref(), Some("foo"), "単語の中");
        assert_eq!(unit_left("foo.|bar").as_deref(), Some("."), "記号");
        assert_eq!(unit_left("日本|語").as_deref(), Some("日本語"));
        assert_eq!(unit_left("テキスト|abc").as_deref(), Some("テキスト"));
        assert_eq!(unit_left("foo|\nbar").as_deref(), Some("foo"), "行末");
        assert_eq!(unit_left("foo\n|bar"), None, "行頭");
        assert_eq!(unit_left("|"), None, "空の文書");
        assert_eq!(unit_left("e\u{301}|x").as_deref(), Some("e\u{301}x"));
    }

    #[test]
    fn 単位の窓は単語と同じく広げ_上限で止める() {
        let long = " ".repeat(5000);
        let s = format!("x{long}|y");
        assert_eq!(unit(&s, 1 << 16).map(|w| w.len()), Some(1), "y");
        let s = format!("x{}|{}y", &long[..2500], &long[2500..]);
        assert_eq!(
            unit(&s, 1 << 16).map(|w| w.len()),
            Some(5000),
            "空白のランの全体"
        );
        assert_eq!(unit(&s, 1000).map(|w| w.len()), Some(2000), "上限の窓");
    }
}
