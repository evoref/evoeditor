//! 書き換えの層の試験 (docs/03 §12.3.1、09 §3.2 の検索の拡張の行の (c)・(d)・(g)・(h) と L0 のゴールデン)。
//!
//! 行をまたぐ検索は M24 の (4) まで開かないので、CRLF の例は「全文を 1 本の文字列として、書き換えた検索語で `regex` を
//! 走らせ、03 §12.1 の選び方 (重ならないものを左から・空の一致は採らない・両端が書記素境界) で選んだ列」で確かめる。

use eve_text::{ByteOff, Text, is_grapheme_boundary};
use proptest::prelude::*;

use super::*;

/// 書き換えた検索語を `regex` にする (フラグ `m`・`R` を付ける。照合器と同じ)。
fn compile(pattern: &str, insensitive: bool) -> regex::Regex {
    regex::RegexBuilder::new(pattern)
        .multi_line(true)
        .crlf(true)
        .case_insensitive(insensitive)
        .size_limit(1 << 26)
        .build()
        .unwrap_or_else(|e| panic!("{pattern:?}: {e}"))
}

/// 全文を 1 本の文字列として、03 §12.1 の選び方で選んだ一致 (キャプチャつき)。
fn select(doc: &str, re: &regex::Regex) -> Vec<Vec<Option<(usize, usize)>>> {
    let t = Text::from(doc);
    let next_char = |i: usize| i + doc[i..].chars().next().map_or(1, char::len_utf8);
    let mut out = Vec::new();
    let mut pos = 0;
    while pos <= doc.len() {
        let Some(c) = re.captures_at(doc, pos) else {
            break;
        };
        let Some(m) = c.get(0) else {
            break;
        };
        let (s, e) = (m.start(), m.end());
        if s == e {
            pos = next_char(e);
            continue;
        }
        if is_grapheme_boundary(&t, ByteOff(s)) && is_grapheme_boundary(&t, ByteOff(e)) {
            out.push(c.iter().map(|g| g.map(|g| (g.start(), g.end()))).collect());
            pos = e;
        } else {
            pos = next_char(s);
        }
    }
    out
}

/// 書き換えた検索語で選んだ一致の範囲。
fn matches(doc: &str, pattern: &str, sugar: bool) -> Vec<(usize, usize)> {
    let w = rewrite(pattern, false, sugar).unwrap_or_else(|e| panic!("{pattern:?}: {e:?}"));
    select(doc, &compile(&w.pattern, false))
        .into_iter()
        .filter_map(|c| c[0])
        .collect()
}

#[test]
fn 表の書き換えの形() {
    let rw = |p: &str, sugar: bool| rewrite(p, false, sugar).map(|w| w.pattern);
    let any = r"(?:\r\n|\r(?mR:^)|[^\r])";
    assert_eq!(rw(r"\n", true).unwrap(), r"(?:\r\n|\n|\r(?mR:^))");
    assert_eq!(rw(r"\n", false).unwrap(), r"\n");
    assert_eq!(rw(r"\x0A", true), rw(r"\n", true), "符号位置で書いた LF");
    assert_eq!(
        rw(r"\r\n", true).unwrap(),
        r"\r\n",
        "続けて書いた CRLF は糖衣を掛けない"
    );
    assert_eq!(rw(r"\r", true).unwrap(), r"\r(?mR:^)");
    assert_eq!(rw(r"\r+", true).unwrap(), r"(?:\r(?mR:^))+");
    assert_eq!(rw(r"[ \n]", true).unwrap(), r"(?:\r\n|\r(?mR:^)|[ \n])");
    assert_eq!(rw(r"[ \n]", false).unwrap(), r"[ \n]");
    assert_eq!(rw(r"[\r ]", true).unwrap(), r"(?:\r(?mR:^)|[ ])");
    assert_eq!(
        rw(r"(?s).", true).unwrap(),
        format!("(?s){any}"),
        "利用者のフラグは残す"
    );
    assert_eq!(rw(r"[\s\S]", true).unwrap(), any);
    assert_eq!(rw(r"[\w\W]", false).unwrap(), any);
    assert_eq!(rw(r"[\d\D]", true).unwrap(), any);
    assert_eq!(
        rw(r".", true).unwrap(),
        ".",
        "`R` の `.` は CR と LF を除く (`regex` のまま)"
    );
    assert_eq!(rw(r"[^\n]", true).unwrap(), r"[[^\n]--\r]");
    assert_eq!(rw(r#"[^"]"#, true).unwrap(), r#"[[^"]--[\n\r]]"#);
    assert_eq!(rw(r"\s", true).unwrap(), r"[\s--[\n\r]]");
    assert_eq!(
        rw(r"\p{L}", true).unwrap(),
        r"\p{L}",
        "行の終端を含まないクラスは書き換えない"
    );
}

#[test]
fn k_と最大の長さ() {
    let k = |p: &str| rewrite(p, false, true).unwrap().lines;
    let max = |p: &str| rewrite(p, false, true).unwrap().max_len;
    assert_eq!(k("abc"), LineSpan::Bounded(0));
    assert_eq!(
        k(r"[^\n]+"),
        LineSpan::Bounded(0),
        "`[^\\n]` は行をまたがない"
    );
    assert_eq!(k(r"\s+"), LineSpan::Bounded(0));
    assert_eq!(k(r#""[^"]*""#), LineSpan::Bounded(0));
    assert_eq!(k(r"\n"), LineSpan::Bounded(2), "CRLF は 2 と数える");
    assert_eq!(k(r"a\nb"), LineSpan::Bounded(2));
    assert_eq!(k(r"\n{3}"), LineSpan::Bounded(6));
    assert_eq!(k(r"\r\n"), LineSpan::Bounded(2));
    assert_eq!(k(r"\r"), LineSpan::Bounded(1));
    assert_eq!(k(r"(?s).{0,3}"), LineSpan::Bounded(6));
    assert_eq!(k(r"\n{0}"), LineSpan::Bounded(0));
    for p in [r"\n+", r"(?s).*", r"[\s\n]*", r"[\s\S]*", r"\n{2,}"] {
        assert_eq!(k(p), LineSpan::Unbounded, "{p}");
    }
    assert_eq!(max("abc"), Some(3));
    assert_eq!(max(r"\n"), Some(2));
    assert_eq!(max(r"a.b"), Some(6), "`.` は 4 バイトまで");
    assert_eq!(max(r"a+"), None);
}

#[test]
fn 行の終端を明示した書き方だけを数える() {
    let ex = |p: &str| rewrite(p, false, true).unwrap().explicit_line_end;
    for p in [r"\n", r"\x0A", r"\r", r"\r\n", r"[\n ]", r"[\r]", r"[\x0D]"] {
        assert!(ex(p), "{p}");
    }
    // 要素に `\n` / `\r` を書いた、すべての文字のクラス: 書き換えは `[\s\S]` と同じで、明示した書き方に数える
    let any = r"(?:\r\n|\r(?mR:^)|[^\r])";
    for p in [r"[\n\s\S]", r"[\r\n\s\S]", r"[\r\w\W]", r"[\x0A\d\D]"] {
        assert!(ex(p), "{p}");
        assert_eq!(rewrite(p, false, true).unwrap().pattern, any, "{p}");
    }
    assert!(ex(r"[\n\s\S]+"));
    for p in [
        r"(?s).",
        r"[\s\S]",
        r"\s",
        r"[^\n]",
        r"[\x00-\x1F]",
        r"[a[\n]]",
        r"abc",
        r"\p{Any}",
    ] {
        assert!(!ex(p), "{p}");
    }
}

// verifies: FR-402
#[test]
fn crlf_を分けて一致させない() {
    // 03 §12.3.1 の「CRLF は 1 つの行の終端」の例 (09 §3.2 の (h) の L0 のゴールデン)
    for p in [r"\n\n", r"\n{2,}", r"\n\s*\n", r"[\s\n]{2}"] {
        assert_eq!(matches("a\r\nb", p, true), [], "{p}");
    }
    assert_eq!(matches("a\r\n\r\nb", r"\n\n", true), [(1, 5)]);
    assert_eq!(matches("a\r\n\r\nb", r"\n{2,}", true), [(1, 5)]);
    assert_eq!(matches("a\r\n\r\nb", r"\n\s*\n", true), [(1, 5)]);
    assert_eq!(matches("a\r\n\r\nb", r"[\s\n]{2}", true), [(1, 5)]);
    // 改行 1 つ: LF・CRLF・単独の CR
    assert_eq!(
        matches("a\nb\r\nc\rd", r"\n", true),
        [(1, 2), (3, 5), (6, 7)]
    );
    // 糖衣を切ると LF だけの改行 (CRLF の LF だけには一致しない)
    assert_eq!(matches("a\nb\r\nc\rd", r"\n", false), [(1, 2)]);
    // 続けて書いた `\r\n` は CRLF だけ、`\r` は単独の CR だけ
    assert_eq!(matches("a\nb\r\nc\rd", r"\r\n", true), [(3, 5)]);
    assert_eq!(matches("a\nb\r\nc\rd", r"\r", true), [(6, 7)]);
    // 明示した `\r?\n` は CRLF と、単独の CR に続く改行 (2 つの改行)
    assert_eq!(matches("a\r\nb\r\r\nc", r"\r?\n", true), [(1, 3), (4, 7)]);
    // `(?s)` の `.` と `[\s\S]` は CRLF を 1 文字に数える
    assert_eq!(matches("a\r\nb", r"(?s)a.b", true), [(0, 4)]);
    assert_eq!(matches("a\r\nb", r"a[\s\S]b", true), [(0, 4)]);
    assert_eq!(matches("a\r\nb", r"(?s)a..b", true), []);
    // 否定・略記のクラスは行の終端に一致しない (`[^\n]` は CR にも一致しない)
    assert_eq!(matches("a\rb", r"a[^\n]b", true), []);
    assert_eq!(matches("a\r\nb", r"a\s+b", true), []);
    assert_eq!(matches("\"a\r\nb\"", r#""[^"]*""#, true), []);
    // 行をまたぐ空白は、行の終端をクラスの要素として書き足す
    assert_eq!(matches("a \r\n b", r"a[\s\n]*b", true), [(0, 6)]);
    assert_eq!(matches("a\r\n\r\nb", r"[\n ]+", true), [(1, 5)]);
}

// verifies: FR-402
#[test]
fn crlf_の文書で空行をまとめる置換は空行を足さない() {
    // `\n\n+` を `\n\n` (置換の文字列の `\n` は文書の改行 CRLF) にすべて置換しても、行の数が変わらない
    let lines = |s: &str| s.matches("\r\n").count();
    for doc in ["a\r\nb\r\nc\r\n", "a\r\n\r\nb\r\n", "a\r\nb"] {
        let mut out = String::new();
        let mut last = 0;
        for (s, e) in matches(doc, r"\n\n+", true) {
            out.push_str(&doc[last..s]);
            out.push_str("\r\n\r\n");
            last = e;
        }
        out.push_str(&doc[last..]);
        assert_eq!(lines(&out), lines(doc), "{doc:?} → {out:?}");
    }
    // 空行が 2 つ以上続くところだけをまとめる
    assert_eq!(matches("a\r\n\r\n\r\nb", r"\n\n+", true), [(1, 7)]);
}

#[test]
fn 構文の誤りは利用者の検索語の位置で示す() {
    for p in ["(", "[", r"(a)\1", r"a(?=b)", r"\p{Foo}", r"(?-u)\xFF"] {
        assert!(
            matches!(rewrite(p, false, true), Err(QueryError::Syntax(_))),
            "{p}"
        );
    }
    // 文言は `regex` が返すものと同じ (検索語は実行時の値にして、clippy の `invalid_regex` を避ける)
    for p in ["(", "[a", r"\p{Foo}", r"a{2,1}"].map(String::from) {
        let Err(QueryError::Syntax(msg)) = rewrite(&p, false, true) else {
            unreachable!()
        };
        let Err(regex::Error::Syntax(want)) = regex::Regex::new(&p) else {
            unreachable!()
        };
        assert_eq!(msg, first_line(&want), "{p}");
    }
}

// ---- 性質 ----

/// 書き換えの対象になる書き方と、ならない書き方の部品。
const ATOMS: &[&str] = &[
    "a",
    "b",
    "あ",
    "é",
    " ",
    "\"",
    "_",
    "1",
    r"\t",
    ".",
    r"\n",
    r"\r",
    r"\x0A",
    r"\x{D}",
    r"\s",
    r"\S",
    r"\w",
    r"\W",
    r"\d",
    r"\D",
    r"\p{L}",
    r"\P{L}",
    r"\pN",
    r"\p{Cc}",
    r"\p{Any}",
    r"[^\n]",
    r#"[^"]"#,
    r"[^a]",
    r"[\s\S]",
    r"[\w\W]",
    r"[\n ]",
    r"[\r a]",
    r"[\s\n]",
    r"[\r\n]",
    r"[\x00-\x1F]",
    r"[[:space:]]",
    r"[a-c\n]",
    r"[a[\n]]",
    r"[\n&&\s]",
    r"[\w--a]",
    "^",
    "$",
    r"\A",
    r"\z",
    r"\b",
    r"\B",
    "(?i)",
    "(?s)",
    "(?-m)",
    "(?-R)",
    "(?U)",
    "(?-s)",
];

fn pattern() -> impl Strategy<Value = String> {
    let leaf = prop::sample::select(ATOMS).prop_map(str::to_owned);
    leaf.prop_recursive(4, 24, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 1..4).prop_map(|v| v.concat()),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| format!("{a}|{b}")),
            inner.clone().prop_map(|a| format!("({a})")),
            (inner.clone(), 0u32..50).prop_map(|(a, n)| format!("(?P<g{n}>{a})")),
            inner.clone().prop_map(|a| format!("(?:{a})")),
            (
                inner.clone(),
                prop::sample::select(&["i", "s", "-m", "-R", "U", "x", "-i", "is", "s-R"][..])
            )
                .prop_map(|(a, f)| format!("(?{f}:{a})")),
            (
                inner,
                prop::sample::select(
                    &["?", "*", "+", "{2}", "{1,3}", "{0,}", "*?", "+?", "{0}"][..]
                )
            )
                .prop_map(|(a, q)| format!("(?:{a}){q}")),
        ]
    })
}

/// CR と LF を含まない文書の文字 (行の終端でない U+0085・U+2028、エスケープ文字 U+10FF0A を含む)。
const DOC_NO_EOL: &[char] = &[
    'a',
    'b',
    'あ',
    'é',
    'e',
    '\u{301}',
    ' ',
    '"',
    '\t',
    '_',
    '1',
    'A',
    '😀',
    '\u{85}',
    '\u{2028}',
    '\u{10FF0A}',
    'Σ',
    'σ',
];

/// CR と LF を混ぜた文書の文字。
const DOC_EOL: &[char] = &[
    'a', 'b', 'あ', ' ', '"', '\t', '\r', '\n', '\r', '\n', 'é', '\u{301}',
];

fn doc(chars: &'static [char]) -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(chars), 0..24).prop_map(|v| v.into_iter().collect())
}

/// 文書の全体の一致とキャプチャ (`captures_iter`。選び方の規則は掛けない)。
fn all_captures(re: &regex::Regex, doc: &str) -> Vec<Vec<Option<(usize, usize)>>> {
    re.captures_iter(doc)
        .map(|c| c.iter().map(|g| g.map(|g| (g.start(), g.end()))).collect())
        .collect()
}

fn build(p: &str, insensitive: bool) -> Option<regex::Regex> {
    regex::RegexBuilder::new(p)
        .multi_line(true)
        .crlf(true)
        .case_insensitive(insensitive)
        .size_limit(1 << 24)
        .build()
        .ok()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    // 09 §3.2 の (g): CR と LF を含まない文書・同一視 OFF では、書き換えた検索語と元の検索語 (`m`・`R` 付き) の
    // 一致とキャプチャ (番号と名前) が同じ
    #[test]
    fn crlf_の無い文書では書き換えの前後で一致とキャプチャが同じ(
        p in pattern(),
        d in doc(DOC_NO_EOL),
        insensitive: bool,
        sugar: bool,
    ) {
        let Some(orig) = build(&p, insensitive) else {
            // 元の検索語が作れないなら、書き換えも誤りになる
            prop_assert!(rewrite(&p, insensitive, sugar).is_err() || build(&p, insensitive).is_none());
            return Ok(());
        };
        let w = rewrite(&p, insensitive, sugar).map_err(|e| TestCaseError::fail(format!("{p:?}: {e:?}")))?;
        let Some(re) = build(&w.pattern, insensitive) else {
            return Err(TestCaseError::fail(format!("{p:?} → {:?} を作れない", w.pattern)));
        };
        prop_assert_eq!(re.captures_len(), orig.captures_len(), "{:?} → {:?}", p, w.pattern);
        prop_assert!(re.capture_names().eq(orig.capture_names()), "{:?} → {:?}", p, w.pattern);
        prop_assert_eq!(all_captures(&re, &d), all_captures(&orig, &d), "{:?} → {:?}", p, w.pattern);
    }

    // 09 §3.2 の (c)・(d)・(h): CR と LF を含む文書で、選んだ一致の行の終端のバイトの数は k 以下、長さは最大の長さ以下。
    // 一致とキャプチャの端は CRLF の CR と LF の間に来ない。行の終端を明示しない検索語の一致は行の終端を含まない
    #[test]
    fn 行の終端の規則と_k_の健全さ(
        p in pattern(),
        d in doc(DOC_EOL),
        insensitive: bool,
        sugar: bool,
    ) {
        let Ok(w) = rewrite(&p, insensitive, sugar) else {
            return Ok(());
        };
        let Some(re) = build(&w.pattern, insensitive) else {
            return Ok(());
        };
        let inside_crlf = |i: usize| i > 0 && d.as_bytes().get(i - 1) == Some(&b'\r') && d.as_bytes().get(i) == Some(&b'\n');
        // `(?s)` の `.` とすべての文字のクラスは明示した書き方と同じく行の終端に一致する
        let any = p.contains("(?s") || p.contains("(?is") || p.contains(r"[\s\S]") || p.contains(r"[\w\W]") || p.contains(r"\p{Any}");
        for caps in select(&d, &re) {
            let Some((s, e)) = caps[0] else { continue };
            let ends = d.as_bytes()[s..e].iter().filter(|b| matches!(b, b'\r' | b'\n')).count();
            match w.lines {
                LineSpan::Bounded(k) => prop_assert!(ends <= k, "{:?} → {:?}: {:?} の [{}, {}) は {} 個", p, w.pattern, d, s, e, ends),
                LineSpan::Unbounded => {}
            }
            if let Some(max) = w.max_len {
                prop_assert!(e - s <= max);
            }
            if !w.explicit_line_end && !any {
                prop_assert_eq!(ends, 0, "{:?} → {:?}: {:?} の [{}, {})", p, w.pattern, d, s, e);
            }
            for g in caps.iter().flatten() {
                prop_assert!(!inside_crlf(g.0) && !inside_crlf(g.1), "{:?} → {:?}: {:?} の {:?}", p, w.pattern, d, caps);
            }
        }
    }
}
