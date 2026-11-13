//! CP932 互換文字の同一視 (docs/03 §12.4.1、01 §5.5 の FR-401 AC-4・AC-5、OQ-41 / OQ-218 の 5〜8、08 §2.5 の M24 の (5))。
//!
//! 09 §4 のコーパス: 8 組の両側を並べた UTF-8 の文書 (`utf8_corpus`) と、CP932 の側を並べた `Shift_JIS` の文書
//! (`SJIS_CORPUS`)。同一視しない `¥` / `￥`・`‾` / `￣`・`\` / `¥`・`~` / `～` を並べた行も置く。リテラル・正規表現 (リテラルの
//! 文字・クラス・否定・Unicode のクラス・符号位置)・大小区別の 3 値・単語単位・選択範囲内・置換の組で確かめる。
//! 一致の範囲は文書の元のバイト範囲。行の窓・全文の連結の経路は `fuzz_regex_search` (09 §3.3) でも同一視を入れて確かめる。

use std::sync::Arc;

use eve_core::{
    CancelToken, Case, Encoding, OpenOptions, SearchQuery, build_replace_all, build_replace_all_in,
    scan_bytes,
};
use eve_testkit::Harness;
use eve_testkit::helpers::cell_text;
use eve_text::{ByteOff, ByteRange, Text, TextRead, word_boundaries};
use proptest::prelude::*;

/// 同一視する 8 組 (JIS 系の写像の側, CP932 の写像の側)。03 §12.4.1 の表 (実装の表とは別に、ここに書き写す)。
const PAIRS: [(char, char); 8] = [
    ('\u{301C}', '\u{FF5E}'),
    ('\u{2016}', '\u{2225}'),
    ('\u{2212}', '\u{FF0D}'),
    ('\u{00A2}', '\u{FFE0}'),
    ('\u{00A3}', '\u{FFE1}'),
    ('\u{00AC}', '\u{FFE2}'),
    ('\u{00A6}', '\u{FFE4}'),
    ('\u{2014}', '\u{2015}'),
];

/// 同一視しない組 (`¥` / `￥`・`‾` / `￣`・`\` / `¥`・`~` / `～`)。
const NOT_FOLDED: [(char, char); 4] = [
    ('\u{00A5}', '\u{FFE5}'),
    ('\u{203E}', '\u{FFE3}'),
    ('\\', '\u{00A5}'),
    ('~', '\u{FF5E}'),
];

/// 09 §4 のコーパス (UTF-8): 各組の両側を 1 行に並べ、最後に同一視しない組の行を置く。
fn utf8_corpus() -> String {
    let mut s = String::new();
    for (a, b) in PAIRS {
        s.push_str(&format!("x{a}y{b}z\n"));
    }
    s.push_str("¥￥‾￣\\~\n");
    s
}

/// 09 §4 のコーパス (`Shift_JIS`): CP932 の側 (0x8160 → U+FF5E、0x8161 → U+2225、0x817C → U+FF0D、0x8191 → U+FFE0、
/// 0x8192 → U+FFE1、0x81CA → U+FFE2、0x815C → U+2015、0xFA55 → U+FFE4) を 1 つずつ、行に分けて並べたもの。
const SJIS_CORPUS: &[u8] =
    b"a\x81\x60\na\x81\x61\na\x81\x7C\na\x81\x91\na\x81\x92\na\x81\xCA\na\x81\x5C\na\xFA\x55\n";

/// `SJIS_CORPUS` の各行の CP932 の側の文字 (行の順)。
const SJIS_SIDE: [char; 8] = [
    '\u{FF5E}', '\u{2225}', '\u{FF0D}', '\u{FFE0}', '\u{FFE1}', '\u{FFE2}', '\u{2015}', '\u{FFE4}',
];

fn partner(c: char) -> Option<char> {
    PAIRS.iter().find_map(|&(a, b)| {
        if c == a {
            Some(b)
        } else if c == b {
            Some(a)
        } else {
            None
        }
    })
}

/// 走査の入口で文書の全体を探した一致 (照合器はここで作る)。
fn find_all(doc: &str, q: &SearchQuery) -> Vec<(usize, usize)> {
    let t = Text::from(doc);
    let Some(m) = q.matcher() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    scan_bytes(
        &t,
        &m,
        ByteRange::new(ByteOff(0), ByteOff(t.len_bytes())),
        &CancelToken::new(),
        &mut |r| {
            out.push((r.start.0, r.end.0));
            true
        },
    );
    out
}

fn lit(p: &str) -> SearchQuery {
    SearchQuery::literal(p)
}

fn re(p: &str) -> SearchQuery {
    SearchQuery::literal(p).with_regex(true)
}

/// `doc` の中の `c` の位置 (バイト範囲) をすべて。
fn positions(doc: &str, c: char) -> Vec<(usize, usize)> {
    doc.char_indices()
        .filter(|&(_, x)| x == c)
        .map(|(i, x)| (i, i + x.len_utf8()))
        .collect()
}

/// 組 (a, b) の両方の位置 (昇順)。
fn both(doc: &str, a: char, b: char) -> Vec<(usize, usize)> {
    let mut v = positions(doc, a);
    v.extend(positions(doc, b));
    v.sort_unstable();
    v
}

const CASES: [Case; 3] = [Case::Smart, Case::Sensitive, Case::Insensitive];

// ---- リテラル検索 ----

// verifies: FR-401
#[test]
fn 八組のどちらで探しても両方に一致し_範囲は文書の元のバイト範囲() {
    let doc = utf8_corpus();
    for (a, b) in PAIRS {
        let want = both(&doc, a, b);
        assert_eq!(want.len(), 2);
        for c in [a, b] {
            for case in CASES {
                let q = lit(&c.to_string()).with_case(case);
                assert_eq!(find_all(&doc, &q), want, "{c} ({case:?})");
                // 前後に文字を付けた語でも (行の窓の経路。k は 0)
                let q = lit(&format!("x{a}y")).with_case(case);
                assert_eq!(
                    find_all(&doc, &q),
                    positions(&doc, 'x')[..]
                        .iter()
                        .filter(|r| doc[r.0..].starts_with(&format!("x{a}y")))
                        .map(|r| (r.0, r.0 + 2 + a.len_utf8()))
                        .collect::<Vec<_>>(),
                    "x{a}y"
                );
            }
        }
        // バイト長の違う組 (`¢` は 2 バイト、`￠` は 3 バイト) も、一致の範囲は元の文字の長さ
        let lens: Vec<usize> = want.iter().map(|r| r.1 - r.0).collect();
        assert_eq!(lens, vec![a.len_utf8(), b.len_utf8()], "{a}{b}");
    }
}

// verifies: FR-401
#[test]
fn 設定が_off_なら書いた文字だけに一致する() {
    let doc = utf8_corpus();
    for (a, b) in PAIRS {
        for c in [a, b] {
            let q = lit(&c.to_string()).with_compat_fold(false);
            assert_eq!(find_all(&doc, &q), positions(&doc, c), "{c}");
            let q = re(&c.to_string()).with_compat_fold(false);
            assert_eq!(find_all(&doc, &q), positions(&doc, c), "{c} (正規表現)");
            assert!(!q.analyze().unwrap().unwrap().compat_folded());
        }
    }
}

// verifies: FR-401
#[test]
fn 円記号とオーバーラインと逆斜線とチルダは同一視しない() {
    let doc = utf8_corpus();
    for (a, b) in NOT_FOLDED {
        for (c, other) in [(a, b), (b, a)] {
            for q in [lit(&c.to_string()), re(&regex_escape(c))] {
                let got = find_all(&doc, &q);
                // 書いた文字には一致し、組にしない相手には一致しない (`～` は `〜` と組なので、`〜` には一致する)
                let mut want = positions(&doc, c);
                if let Some(p) = partner(c) {
                    want.extend(positions(&doc, p));
                    want.sort_unstable();
                }
                assert_eq!(got, want, "{c} ({a}{b})");
                assert!(
                    positions(&doc, other).iter().all(|r| !got.contains(r)),
                    "{c} は {other} に一致しない"
                );
            }
        }
    }
    // 互換文字を含まない検索語は、今の経路 (memmem) のまま。印も出ない
    let q = lit("¥");
    assert!(q.analyze().unwrap().unwrap().regex_pattern().is_none());
    assert!(!q.analyze().unwrap().unwrap().compat_folded());
}

fn regex_escape(c: char) -> String {
    if c.is_ascii_punctuation() {
        format!("\\{c}")
    } else {
        c.to_string()
    }
}

// verifies: FR-401
#[test]
fn 大小の区別のある文字と組み合わせても_大小の区別と同一視の両方が掛かる() {
    // `Ａ` (全角の大文字) は大小の区別がある。Insensitive では `ａ` にも、同一視で `～` にも一致する
    let doc = "Ａ〜 ａ～ Ａ～ ａ〜";
    let q = lit("Ａ〜").with_case(Case::Insensitive);
    assert_eq!(find_all(doc, &q).len(), 4);
    let q = lit("Ａ〜").with_case(Case::Sensitive);
    assert_eq!(find_all(doc, &q), vec![(0, 6), (14, 20)]);
    // Smart: 大文字を含むので区別する (互換文字は Smart の判定に影響しない)
    let q = lit("Ａ〜");
    assert!(q.is_case_sensitive());
    assert_eq!(find_all(doc, &q), vec![(0, 6), (14, 20)]);
    let q = lit("ａ〜");
    assert!(!q.is_case_sensitive());
    assert_eq!(find_all(doc, &q).len(), 4);
    // `k` は K (U+212A) にも一致する経路 (§12.4 の 3) と同じ意味のまま
    let q = lit("k〜").with_case(Case::Insensitive);
    assert_eq!(find_all("K～ \u{212A}〜 k～", &q).len(), 3);
}

// verifies: FR-401
#[test]
fn 単語単位の境界は文書の元の文字で調べる() {
    for doc in [
        "a〜b a～b",
        "あ〜い あ～い",
        "x 〜 y ～ z",
        "〜〜 ～～",
        "1−2 1－2",
    ] {
        for (a, b) in PAIRS {
            for c in [a, b] {
                let all = find_all(doc, &lit(&c.to_string()));
                // 1 行の文書なので、行の単語境界は文書の全体の単語境界
                let bounds = word_boundaries(doc);
                let want: Vec<(usize, usize)> = all
                    .iter()
                    .copied()
                    .filter(|r| bounds.contains(&r.0) && bounds.contains(&r.1))
                    .collect();
                for q in [
                    lit(&c.to_string()).with_whole_word(true),
                    re(&c.to_string()).with_whole_word(true),
                ] {
                    assert_eq!(find_all(doc, &q), want, "{doc:?} {c}");
                }
            }
        }
    }
    // 空白で区切った語は単語単位でも両方に一致する
    assert_eq!(
        find_all("x 〜 y ～ z", &lit("〜").with_whole_word(true)),
        vec![(2, 5), (8, 11)]
    );
}

// verifies: FR-401
#[test]
fn 選択範囲内の検索でも同一視する() {
    let doc = utf8_corpus();
    // 1 行目と 3 行目だけを範囲にする
    let line = |i: usize| {
        let s: usize = doc.split_inclusive('\n').take(i).map(str::len).sum();
        let e = s + doc.split_inclusive('\n').nth(i).unwrap().len();
        ByteRange::new(ByteOff(s), ByteOff(e))
    };
    let scope: Arc<[ByteRange]> = Arc::from(vec![line(0), line(2)]);
    let q = lit("〜").with_scope(Some(scope.clone()));
    assert_eq!(find_all(&doc, &q), both(&doc, '〜', '～'));
    let q = lit("－").with_scope(Some(scope.clone()));
    assert_eq!(find_all(&doc, &q), both(&doc, '−', '－'));
    let q = lit("∥").with_scope(Some(scope));
    assert!(find_all(&doc, &q).is_empty(), "2 行目は範囲の外");
}

// ---- 正規表現 ----

// verifies: FR-401
#[test]
fn 正規表現のリテラルの文字とクラスと符号位置に掛かる() {
    let doc = utf8_corpus();
    let others = |doc: &str, a: char, b: char| -> Vec<(usize, usize)> {
        doc.char_indices()
            .filter(|&(_, x)| x != a && x != b && x != '\n')
            .map(|(i, x)| (i, i + x.len_utf8()))
            .collect()
    };
    for (a, b) in PAIRS {
        let want = both(&doc, a, b);
        for c in [a, b] {
            for case in CASES {
                for p in [
                    c.to_string(),
                    format!("[{c}]"),
                    format!("[{c}q]"),
                    format!("\\x{{{:X}}}", u32::from(c)),
                    format!("[\\x{{{:X}}}]", u32::from(c)),
                    format!("(?i:{c})"),
                    format!("(?:{c})"),
                    format!("[{c}-{c}]"),
                ] {
                    let q = re(&p).with_case(case);
                    assert_eq!(find_all(&doc, &q), want, "{p} ({case:?})");
                    assert!(q.analyze().unwrap().unwrap().compat_folded(), "{p}: 印");
                }
                // 否定のクラスは、否定する前の集合に掛けてから否定する (`[^〜]` は `〜` にも `～` にも一致しない)
                let q = re(&format!("[^{c}\\n]")).with_case(case);
                assert_eq!(find_all(&doc, &q), others(&doc, a, b), "[^{c}]");
            }
        }
    }
}

// verifies: FR-401
#[test]
fn unicode_のクラスと範囲は組の片方を含めば相手も含める() {
    let doc = utf8_corpus();
    // `\p{Sm}` (数学記号) は `～`・`∥`・`−`・`¬`・`￢` を含み、相手の `〜`・`‖`・`－` を含まない → 足す
    let sm: Vec<char> = find_all(&doc, &re(r"\p{Sm}"))
        .iter()
        .map(|r| doc[r.0..r.1].chars().next().unwrap())
        .collect();
    for c in ['〜', '～', '‖', '∥', '−', '－', '¬', '￢'] {
        assert!(sm.contains(&c), "\\p{{Sm}} は {c} に一致する: {sm:?}");
    }
    // 否定 (`\P{Sm}`) は相手を差し引く (どちらにも一致しない)
    let nsm: Vec<char> = find_all(&doc, &re(r"\P{Sm}"))
        .iter()
        .map(|r| doc[r.0..r.1].chars().next().unwrap())
        .collect();
    for c in ['〜', '～', '‖', '∥', '−', '－', '¬', '￢'] {
        assert!(!nsm.contains(&c), "\\P{{Sm}} は {c} に一致しない");
    }
    assert!(nsm.contains(&'x'));
    // 範囲 `[\x{2010}-\x{2014}]` は `—` を含み `―` を含まない → `―` も
    let q = re(r"[\x{2010}-\x{2014}]");
    assert_eq!(find_all(&doc, &q), both(&doc, '—', '―'));
    // 同一視を切ると元の集合
    let q = re(r"\p{Sm}").with_compat_fold(false);
    assert!(!find_all(&doc, &q).iter().any(|r| &doc[r.0..r.1] == "〜"));
}

// verifies: FR-401
#[test]
fn 一致する文字が変わらないクラスには印を付けない() {
    for p in [
        r"\S",
        r"\W",
        ".",
        "[^a]",
        "[〜～]",
        "[〜～a]",
        r"\w+",
        "abc",
        r"(?s).",
        r"[\s\S]",
    ] {
        let plan = re(p).analyze().unwrap().unwrap();
        assert!(!plan.compat_folded(), "{p}");
    }
    for p in [
        "〜",
        r"\p{Sm}",
        r"\P{Sm}",
        "[^〜]",
        r"[\x{2010}-\x{2014}]",
        r"\x{FF5E}",
        "a|―",
    ] {
        let plan = re(p).analyze().unwrap().unwrap();
        assert!(plan.compat_folded(), "{p}");
    }
    for p in ["〜", "x―y"] {
        assert!(lit(p).analyze().unwrap().unwrap().compat_folded(), "{p}");
    }
    for p in ["abc", "¥", "~", "‾"] {
        assert!(!lit(p).analyze().unwrap().unwrap().compat_folded(), "{p}");
    }
}

#[test]
fn 書き換えは捕獲グループを足さず_行の終端の数を変えない() {
    let q = re(r"(〜)\n(?P<n>[−a])");
    let plan = q.analyze().unwrap().unwrap();
    let r = regex::Regex::new(plan.regex_pattern().unwrap()).unwrap();
    assert_eq!(r.captures_len(), 3);
    assert_eq!(r.capture_names().flatten().collect::<Vec<_>>(), ["n"]);
    assert_eq!(
        plan.lines(),
        re(r"(x)\n(?P<n>[ya])").analyze().unwrap().unwrap().lines()
    );
}

#[test]
fn 非_unicode_の範囲には掛けない() {
    let q = re("(?-u:〜)");
    let plan = q.analyze().unwrap().unwrap();
    assert!(!plan.compat_folded());
    assert_eq!(find_all("〜～", &q), vec![(0, 3)]);
}

// ---- 置換 ----

// verifies: FR-401
#[test]
fn 置換の文字列は書いたまま入り_同一視で当たった件数を数える() {
    let cancel = CancelToken::new();
    let run = |doc: &str, q: &SearchQuery, rep: &str| {
        let t = Text::from(doc);
        let b = build_replace_all_in(&t, q, rep, None, &cancel)
            .unwrap()
            .unwrap();
        let mut t2 = Text::from(doc);
        #[allow(clippy::disallowed_methods)]
        t2.apply(&b.changes).unwrap();
        (
            t2.slice_to_string(ByteRange::new(ByteOff(0), ByteOff(t2.len_bytes()))),
            b.count,
            b.folded,
        )
    };
    // `〜` を `～` にそろえる: もとから `～` だった箇所にも一致するが、一致した文字列と置換の文字列が同じなので置き換えず、
    // 件数にも同一視の件数にも数えない (03 §12.5、OQ-218 の 6 の改訂)
    assert_eq!(run("〜～〜", &lit("〜"), "～"), ("～～～".to_owned(), 2, 0));
    assert_eq!(run("〜～〜", &lit("〜"), "x"), ("xxx".to_owned(), 3, 1));
    // 各組で、書いたほうの文字だけが入る
    for (a, b) in PAIRS {
        let doc = format!("{a}{b}");
        let (out, n, folded) = run(&doc, &lit(&a.to_string()), "#");
        assert_eq!((out.as_str(), n, folded), ("##", 2, 1), "{a}{b}");
        let (out, ..) = run(&doc, &lit(&b.to_string()), &a.to_string());
        assert_eq!(out, format!("{a}{a}"));
    }
    // `$1` は文書の元の文字を運ぶ
    assert_eq!(
        run("〜～", &re("(〜)"), "<$1>"),
        ("<〜><～>".to_owned(), 2, 1)
    );
    // 利用者が両方を書いたクラスは、同一視を切っても同じ範囲に一致するので数えない
    assert_eq!(run("〜～", &re("[〜～]"), "-"), ("--".to_owned(), 2, 0));
    // 大小を区別しない語: `ａ～` は大小の畳み込みと同一視、`Ａ〜` は大小だけ → 同一視で当たったのは 1 件
    let q = lit("Ａ〜").with_case(Case::Insensitive);
    assert_eq!(run("ａ～ Ａ〜 ａ〜", &q, "x"), ("x x x".to_owned(), 3, 1));
    // 同一視を切ると、書いた文字だけ
    assert_eq!(
        run("〜～", &lit("〜").with_compat_fold(false), "x"),
        ("x～".to_owned(), 1, 0)
    );
    // 文書の端と `^`・`$`・`\b` の判定は前後の文字で決まる (同一視を切った検索語も同じ文脈で調べる)
    assert_eq!(run("～\n〜", &re("^〜$"), "x"), ("x\nx".to_owned(), 2, 1));
    assert_eq!(
        run("～a 〜", &re(r"\A〜|〜\z"), "x"),
        ("xa x".to_owned(), 2, 1)
    );
    // 互換文字を含まない検索語は 0
    assert_eq!(run("a〜a", &lit("a"), "b"), ("b〜b".to_owned(), 2, 0));
    // すべて置換の簡易版 (件数だけ) も同じ件数 (`〜` は置換の文字列と同じなので数えない)
    let (_, n) = build_replace_all(&Text::from("〜～"), &lit("～"), "〜", &cancel).unwrap();
    assert_eq!(n, 1);
}

/// (置換後の本文, 件数, 同一視で当たった件数)。
fn replace_all(doc: &str, q: &SearchQuery, rep: &str) -> (String, u64, u64) {
    let t = Text::from(doc);
    let b = build_replace_all_in(&t, q, rep, None, &CancelToken::new())
        .unwrap()
        .unwrap();
    let mut t2 = Text::from(doc);
    #[allow(clippy::disallowed_methods)]
    t2.apply(&b.changes).unwrap();
    (
        t2.slice_to_string(ByteRange::new(ByteOff(0), ByteOff(t2.len_bytes()))),
        b.count,
        b.folded,
    )
}

// 同一視で当たった件数は「同一視を切った検索語が、文書の前後の文脈のまま、その範囲にちょうど一致できない一致」(03 §12.4.1 の
// 「置換」、OQ-218 の 6 の改訂)。一致の後ろの文字列を切ったところで `$`・`\b`・`\z` が偽って成り立たず、選択の優先にも
// 左右されない (レビューの指摘から)
// verifies: FR-401
#[test]
fn 同一視で当たった件数は文書の文脈のまま範囲にちょうど一致できるかで数える() {
    // `a` は同一視していない一致 (`b$` などは文書の上では偽)
    for p in [r"a(?:b$)?|〜", r"a(?:b\b)?|〜", r"a(?:b\z)?|〜"] {
        assert_eq!(
            replace_all("abc", &re(p), "#"),
            ("#bc".to_owned(), 1, 0),
            "{p}"
        );
    }
    // ありそうな形: 「— の後ろに数があれば数も」。文書の `—` は同一視で当たっていない
    assert_eq!(
        replace_all("—2nd", &re(r"—(?:\d+\b)?"), "-"),
        ("-2nd".to_owned(), 1, 0)
    );
    assert_eq!(
        replace_all("―2nd", &re(r"—(?:\d+\b)?"), "-"),
        ("-2nd".to_owned(), 1, 1)
    );
    // 同一視を切った検索語の最初の一致は `～cc` だが、3 つ目の選択肢で `～` にちょうど一致できる → 数えない
    assert_eq!(
        replace_all("～cc", &re("〜|～cc|[〜～]"), "#"),
        ("#cc".to_owned(), 1, 0)
    );
    // `$1` のキャプチャと番号
    assert_eq!(
        replace_all("〜a ～b", &re(r"(〜)(?P<n>[ab])"), "$2$1"),
        ("a〜 b～".to_owned(), 2, 1)
    );
    // 否定のクラス: 同一視すると `[^〜]` は `～` に一致しない
    assert_eq!(
        replace_all("〜～a", &re("[^〜]"), "#"),
        ("〜～#".to_owned(), 1, 0)
    );
    // \P{Sm}: 同一視で外れる文字
    assert_eq!(
        replace_all("〜‖－x", &re(r"\P{Sm}"), "#"),
        ("〜‖－#".to_owned(), 1, 0)
    );
    // 行をまたぐ一致: 行の終端の後ろの文脈で `^` が決まる
    assert_eq!(
        replace_all("～\nx", &re(r"〜\n^x"), "#"),
        ("#".to_owned(), 1, 1)
    );
}

// 一致した文字列と置換の文字列 (展開した後) が同じ一致は置き換えず、件数にも数えない。同一視でない普通の置換にも効く
// (03 §12.5、OQ-218 の 6 の改訂)
// verifies: FR-403
#[test]
fn 置換しても文字が変わらない一致は置き換えず数えない() {
    assert_eq!(
        replace_all("a b a", &lit("a"), "a"),
        ("a b a".to_owned(), 0, 0)
    );
    assert_eq!(
        replace_all("ab cd", &re(r"(\w+)"), "$1"),
        ("ab cd".to_owned(), 0, 0)
    );
    assert_eq!(replace_all("ab", &re("[ab]"), "a"), ("aa".to_owned(), 1, 0));
    // 大小を区別しない検索で、大小だけが違う一致は置き換える (バイトで比べる)
    assert_eq!(
        replace_all("A a", &lit("a").with_case(Case::Insensitive), "a"),
        ("a a".to_owned(), 1, 0)
    );
    // 長さの違う組 (`¢` は 2 バイト、`￠` は 3 バイト): `￠` は置換の文字列と同じなので置き換えない
    assert_eq!(
        replace_all("¢￠", &lit("¢"), "￠"),
        ("￠￠".to_owned(), 1, 0)
    );
    assert_eq!(replace_all("¢￠", &lit("¢"), "c"), ("cc".to_owned(), 2, 1));
}

// ---- エディタを通して (設定・件数の印・知らせ・Shift_JIS) ----

fn bar_count(h: &mut Harness) -> String {
    let s = h.frame().search.expect("検索バー");
    s.count_text.map(|t| cell_text(&t)).unwrap_or_default()
}

fn message(h: &mut Harness) -> (String, String) {
    let m = h.frame().messages.current.expect("メッセージ");
    (
        cell_text(&m.text),
        m.hint.map(|t| cell_text(&t)).unwrap_or_default(),
    )
}

/// `doc` を開いて検索バーを開く (`fold` が `Some` なら設定ファイルに書く)。
fn open_bar(doc: &str, fold: Option<bool>, replace: bool) -> Harness {
    let mut h = Harness::new()
        .with_size(100, 12)
        .with_file("/w/a.txt", doc.as_bytes());
    if let Some(v) = fold {
        let loaded = eve_config::load_str(&format!("[search]\ncp932_compat_fold = {v}\n"));
        assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);
        assert_eq!(loaded.config.cp932_compat_fold, v);
        h.editor.configure(eve_core::StartupConfig {
            config: Some(loaded.config),
            ..Default::default()
        });
    }
    h.open("/w/a.txt");
    h.run_jobs();
    h.keys("ctrl+home");
    h.keys(if replace { "ctrl+r" } else { "ctrl+f" });
    h
}

// verifies: FR-401
#[test]
fn 設定_search_cp932_compat_fold_は未設定で_on_で_検索バーを開いたときの値で効く() {
    let count = |fold: Option<bool>| {
        let mut h = open_bar("a〜b a～b", fold, false);
        h.text("〜");
        h.run_jobs();
        bar_count(&mut h)
    };
    assert_eq!(count(None), "1/2 互換", "既定は ON");
    assert_eq!(count(Some(true)), "1/2 互換");
    assert_eq!(
        count(Some(false)),
        "1/1",
        "OFF は書いた文字だけで、印も出ない"
    );
    // 設定にキーを書いても「このバージョンでは未対応」の情報診断を出さない (公開キー)
    let loaded = eve_config::load_str("[search]\ncp932_compat_fold = false\n");
    assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);
}

// verifies: FR-401
#[test]
fn 件数の印は同一視が効いているときだけ付く() {
    let mut h = open_bar("a〜b a～b\nx", None, false);
    h.text("a");
    h.run_jobs();
    assert_eq!(bar_count(&mut h), "1/2", "互換文字を含まない語");
    h.keys("ctrl+a");
    h.text("〜");
    h.run_jobs();
    assert_eq!(bar_count(&mut h), "1/2 互換");
    // 0 件でも印は付く (同一視して探したことを示す)
    h.keys("ctrl+a");
    h.text("〜〜");
    h.run_jobs();
    assert_eq!(bar_count(&mut h), "0 件 互換");
    // 正規表現: リテラルの互換文字・組の片方を含むクラスには付き、両方を含むクラスには付かない
    h.keys("alt+r");
    h.keys("ctrl+a");
    h.text(r"\S");
    h.run_jobs();
    assert!(!bar_count(&mut h).contains("互換"));
    h.keys("ctrl+a");
    h.text(r"[〜～]");
    h.run_jobs();
    assert_eq!(bar_count(&mut h), "1/2");
    h.keys("ctrl+a");
    h.text(r"\p{Sm}");
    h.run_jobs();
    // 今の一致 `〜` (数学記号でない) は、`～` (数学記号) との同一視で当たった
    assert_eq!(bar_count(&mut h), "1/2 互換一致");
}

// verifies: FR-401
#[test]
fn 入力すると両方の一致を強調し_次へで移る() {
    let mut h = open_bar("a〜b a～b", None, false);
    h.text("～");
    h.run_jobs();
    h.keys("enter");
    assert_eq!(h.selection_range(), (1, 4));
    h.keys("enter");
    assert_eq!(h.selection_range(), (7, 10));
    assert_eq!(h.doc_text(), "a〜b a～b", "文書は変わらない");
}

// verifies: FR-401
#[test]
fn すべて置換の知らせは同一視で当たった件数を分けて言う() {
    let mut h = open_bar("〜 ～ 〜", None, true);
    h.text("〜");
    h.keys("tab");
    h.text("x");
    h.keys("alt+a");
    h.run_jobs();
    assert_eq!(h.doc_text(), "x x x");
    assert!(
        h.messages()
            .iter()
            .any(|k| k == "info.search.replaced_folded")
    );
    assert_eq!(
        message(&mut h),
        (
            "3 件を置換しました (互換文字の同一視で一致したのは 1 件。元に戻すで戻せます)"
                .to_owned(),
            String::new()
        )
    );
    // 元に戻すで戻る
    h.keys("esc");
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "〜 ～ 〜");
}

// `〜` を `～` にそろえるすべて置換: もとから `～` の箇所は置き換えず数えない。全部がそうなら文書は変わらず、未保存にならない
// (03 §12.5、OQ-218 の 6 の改訂)
// verifies: FR-403
#[test]
fn そろえるすべて置換はもとからそろっている箇所を置き換えない() {
    let mut h = open_bar("〜 ～ 〜", None, true);
    h.text("〜");
    h.keys("tab");
    h.text("～");
    h.keys("alt+a");
    h.run_jobs();
    assert_eq!(h.doc_text(), "～ ～ ～");
    assert_eq!(message(&mut h).0, "2 件を置換しました (元に戻すで戻せます)");
    // 全部がもとからそろっている: 文書は変わらず、未保存にならない
    let mut h = open_bar("～ ～", None, true);
    h.text("〜");
    h.keys("tab");
    h.text("～");
    h.keys("alt+a");
    h.run_jobs();
    assert_eq!(h.doc_text(), "～ ～");
    assert!(!h.is_dirty());
    assert!(
        h.messages()
            .iter()
            .any(|k| k == "info.search.replace_no_match")
    );
}

// 1 件置換で、一致した文字列と置換の文字列が同じなら置き換えずに次の一致へ進む (03 §12.5)
// verifies: FR-403
#[test]
fn 一件置換で文字が変わらない一致は置き換えず次へ進む() {
    let mut h = open_bar("〜 ～", None, true);
    h.text("〜");
    h.run_jobs();
    h.keys("tab");
    h.text("〜");
    // 今の一致は `〜` (置換の文字列と同じ): 置き換えず、次の `～` へ
    h.keys("enter");
    h.run_jobs();
    assert_eq!(h.doc_text(), "〜 ～");
    assert!(!h.is_dirty(), "文書は変わらない");
    assert_eq!(bar_count(&mut h), "2/2 互換一致");
    // 次の `～` を置き換える
    h.keys("enter");
    h.run_jobs();
    assert_eq!(h.doc_text(), "〜 〜");
}

// 今の一致が同一視で当たったものなら、件数の後ろの印が「互換一致」になる。次へで 1 件ずつ見て、片方だけを 1 件置換で直せる
// (03 §12.4.1 の「検索バーの表示」、OQ-218 の 7 の改訂)
// verifies: FR-401
#[test]
fn 今の一致が同一視で当たったものなら互換一致と出し_片方だけを一件置換で直せる() {
    let mut h = open_bar("〜 ～ 〜", None, true);
    h.text("〜");
    h.run_jobs();
    assert_eq!(
        bar_count(&mut h),
        "1/3 互換",
        "入力中の現在の一致は書いた文字"
    );
    h.keys("enter");
    assert_eq!(bar_count(&mut h), "1/3 互換");
    h.keys("enter");
    assert_eq!(h.selection_range(), (4, 7));
    assert_eq!(bar_count(&mut h), "2/3 互換一致", "`～` は同一視で当たった");
    h.keys("enter");
    assert_eq!(bar_count(&mut h), "3/3 互換");
    // 前へで `～` に戻り、置換欄で 1 件置換すると `～` だけが変わる
    h.keys("shift+enter");
    assert_eq!(bar_count(&mut h), "2/3 互換一致");
    h.keys("tab");
    h.text("~");
    h.keys("enter");
    h.run_jobs();
    assert_eq!(h.doc_text(), "〜 ~ 〜");
    // 正規表現でも同じ判定 (`[〜a]` の `～` は同一視で当たった)
    let mut h = open_bar("～", None, false);
    h.keys("alt+r");
    h.text("[〜a]");
    h.run_jobs();
    assert_eq!(bar_count(&mut h), "1/1 互換一致");
    // 英語の文言
    let mut h = Harness::new()
        .with_size(100, 12)
        .with_locale(eve_config::Locale::En)
        .with_file("/w/a.txt", "～".as_bytes());
    h.open("/w/a.txt");
    h.run_jobs();
    h.keys("ctrl+f");
    h.text("〜");
    h.run_jobs();
    assert_eq!(bar_count(&mut h), "1/1 compat match");
}

// verifies: FR-401
#[test]
fn 行数が変わり同一視でも当たったすべて置換は_1_つの知らせにまとめる() {
    let mut h = open_bar("a〜\nb～\nc", None, true);
    h.keys("alt+r");
    h.text(r"〜\n");
    h.keys("tab");
    h.text("-");
    h.keys("alt+a");
    h.run_jobs();
    assert_eq!(h.doc_text(), "a-b-c");
    assert!(
        h.messages()
            .iter()
            .any(|k| k == "info.search.replaced_folded_lines_removed")
    );
    assert_eq!(
        message(&mut h).0,
        "2 件を置換しました。文書は 2 行減りました (互換文字の同一視で一致したのは 1 件。元に戻すで戻せます)"
    );
    // 行を足す置換
    let mut h = open_bar("a〜b～", None, true);
    h.keys("alt+r");
    h.text("〜");
    h.keys("tab");
    h.text(r"\n");
    h.keys("alt+a");
    h.run_jobs();
    // 置換の文字列の `\n` は文書の既定の改行になる
    assert_eq!(h.doc_text().replace("\r\n", "\n"), "a\nb\n");
    assert!(
        h.messages()
            .iter()
            .any(|k| k == "info.search.replaced_folded_lines_added")
    );
    // 同一視で当たらなかったら、今までの知らせ
    let mut h = open_bar("a〜b〜", None, true);
    h.text("〜");
    h.keys("tab");
    h.text("x");
    h.keys("alt+a");
    h.run_jobs();
    assert_eq!(h.doc_text(), "axbx");
    assert!(h.messages().iter().any(|k| k == "info.search.replaced"));
}

// verifies: FR-401
#[test]
fn 一件置換は書いたまま入れる() {
    let mut h = open_bar("a～b", None, true);
    h.text("〜");
    h.run_jobs();
    h.keys("tab");
    h.text("〜");
    h.keys("enter");
    h.run_jobs();
    assert_eq!(h.doc_text(), "a〜b");
}

// verifies: FR-401
#[test]
fn shift_jis_の文書の_cp932_の側にも_jis_の側の文字で一致し_文書とバイト列は変わらない() {
    let mut h = Harness::new()
        .with_size(100, 12)
        .with_file("/w/s.txt", SJIS_CORPUS);
    h.editor.state_mut().open_path(
        "/w/s.txt",
        OpenOptions {
            forced: Some(Encoding::ShiftJis),
            ..OpenOptions::default()
        },
    );
    h.run_jobs();
    let text = h.doc_text();
    for (i, c) in SJIS_SIDE.into_iter().enumerate() {
        assert_eq!(text.lines().nth(i).unwrap(), format!("a{c}"));
        let jis = partner(c).unwrap();
        let q = lit(&jis.to_string());
        assert_eq!(find_all(&text, &q), positions(&text, c), "{jis} → {c}");
        let q = lit(&jis.to_string()).with_compat_fold(false);
        assert!(find_all(&text, &q).is_empty(), "{jis}");
    }
    // エディタの検索バーで探して移っても、文書は変わらず、保存するバイト列も変わらない
    h.keys("ctrl+f");
    h.text("〜");
    h.run_jobs();
    // 文書の `～` は `〜` の同一視で当たった
    assert_eq!(bar_count(&mut h), "1/1 互換一致");
    h.keys("enter");
    h.keys("esc");
    assert_eq!(h.doc_text(), text);
    h.keys("ctrl+s");
    h.run_jobs();
    assert_eq!(h.fs.read("/w/s.txt").unwrap(), SJIS_CORPUS);
}

/// ヘルプの「検索の書き方」の、同一視の案内の行 (導入の行から `¥` キーの行まで)。
fn help_compat_lines(locale: Option<eve_config::Locale>, first: &str) -> Vec<String> {
    let mut h = Harness::new().with_size(200, 60);
    if let Some(l) = locale {
        h = h.with_locale(l);
    }
    h.keys("ctrl+k h");
    let f = h.frame();
    let Some(eve_view::OverlayModel::Help { lines, .. }) = f.overlays.into_iter().next() else {
        panic!("ヘルプが出ている")
    };
    let body: Vec<String> = lines.iter().map(|l| cell_text(&l.text)).collect();
    let s = body
        .iter()
        .position(|t| t.starts_with(first))
        .expect("導入の行");
    let e = body[s..]
        .iter()
        .position(|t| t.contains("U+005C"))
        .expect("`¥` キーの行");
    body[s..=s + e].to_vec()
}

// 04 §8.6: 同一視の組・切り方・印・除く組の案内があり、80 桁の端末で切れないよう 1 行が 78 セル以下 (曖昧幅を 1 / 2 の
// どちらに数えても)。英語も同じ
#[test]
fn ヘルプの同一視の案内は_78_セル以下の短い行に分かれている() {
    for (locale, first) in [
        (None, "互換文字の同一視"),
        (Some(eve_config::Locale::En), "Compatible characters"),
    ] {
        let lines = help_compat_lines(locale, first);
        assert_eq!(lines.len(), 9, "{lines:?}");
        let all = lines.concat();
        for (a, b) in PAIRS {
            assert!(all.contains(a) && all.contains(b), "{a}{b}");
        }
        for c in ['¥', '￥', '‾', '￣', '~', '～', '\\'] {
            assert!(all.contains(c), "{c}");
        }
        assert!(all.contains("search.cp932_compat_fold = false"));
        for l in &lines {
            for ambiguous_wide in [false, true] {
                let cfg = eve_text::WidthConfig {
                    ambiguous_wide,
                    ..eve_text::WidthConfig::default()
                };
                let w = eve_text::str_display_width(l, &cfg);
                assert!(w <= 78, "{w} セル (曖昧幅 {ambiguous_wide}): {l}");
            }
        }
    }
    let ja = help_compat_lines(None, "互換文字の同一視").concat();
    assert!(ja.contains("互換一致") && ja.contains("3/10 互換"));
    // `(?-u)` の範囲に掛けないことは利用者向けの約束にしない
    assert!(!ja.contains("(?-u"));
}

// ---- 性質 ----

/// 文書の文字 (組の両側・同一視しない組・大小の区別のある文字・結合文字)。
const DOC_CHARS: &[char] = &[
    '〜', '～', '‖', '∥', '−', '－', '¢', '￠', '£', '￡', '¬', '￢', '¦', '￤', '—', '―', '¥',
    '￥', '‾', '￣', '\\', '~', 'a', 'A', 'k', '\u{212A}', 's', 'ſ', 'Ａ', 'ａ', 'あ', ' ',
    '\u{301}', '\n',
];

/// 検索語の文字 (改行を含まない)。
const PAT_CHARS: &[char] = &[
    '〜', '～', '‖', '∥', '−', '－', '¢', '￠', '£', '￡', '¬', '￢', '¦', '￤', '—', '―', '¥',
    '~', 'a', 'A', 'k', 's', 'Ａ', 'あ', ' ',
];

/// 組の 2 つ目を 1 つ目にそろえた文字列と、そろえた文字列の位置 → 元の位置 (char 境界だけ使う)。
fn normalize(s: &str) -> (String, Vec<usize>) {
    let mut out = String::new();
    let mut map = Vec::new();
    for (i, c) in s.char_indices() {
        let n = PAIRS.iter().find(|p| p.1 == c).map_or(c, |p| p.0);
        for _ in 0..n.len_utf8() {
            map.push(i);
        }
        out.push(n);
    }
    map.push(s.len());
    (out, map)
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(512),
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    // 09 §3.2 の検索の拡張の行の (f): 同一視 ON のリテラル検索の一致は、文書と検索語の組の 2 つ目を 1 つ目にそろえて
    // 同一視 OFF で探した一致を元の位置へ戻したものと同じ (組は対称・範囲は元のバイト範囲)。大小区別の 3 値・単語単位なしで
    #[test]
    fn リテラル検索の同一視はそろえた文書で探したのと同じ(
        doc in prop::collection::vec(prop::sample::select(DOC_CHARS), 0..40).prop_map(|v| v.into_iter().collect::<String>()),
        pat in prop::collection::vec(prop::sample::select(PAT_CHARS), 1..4).prop_map(|v| v.into_iter().collect::<String>()),
        ci in 0usize..3,
        regex in any::<bool>(),
    ) {
        let case = CASES[ci];
        let (nd, map) = normalize(&doc);
        let (np, _) = normalize(&pat);
        let mk = |p: &str| if regex { re(&regex::escape(p)) } else { lit(p) }.with_case(case);
        let got = find_all(&doc, &mk(&pat));
        let want: Vec<(usize, usize)> = find_all(&nd, &mk(&np).with_compat_fold(false))
            .into_iter()
            .map(|(s, e)| (map[s], map[e]))
            .collect();
        prop_assert_eq!(&got, &want, "検索語 {:?} 文書 {:?}", pat, doc);
        // すべて置換の件数と、同一視で当たった件数 (一致の文字列が、同一視を切った検索語 (大小の扱いは同じ) にちょうど
        // 一致しないもの)
        let exact = regex::RegexBuilder::new(&format!("^{}$", regex::escape(&pat)))
            .case_insensitive(!mk(&pat).is_case_sensitive())
            .build()
            .unwrap();
        let folded = got.iter().filter(|r| !exact.is_match(&doc[r.0..r.1])).count() as u64;
        let t = Text::from(doc.as_str());
        let b = build_replace_all_in(&t, &mk(&pat), "#", None, &CancelToken::new()).unwrap().unwrap();
        prop_assert_eq!(b.count, got.len() as u64);
        prop_assert_eq!(b.folded, folded, "検索語 {:?} 文書 {:?}", pat, doc);
    }
}

// ---- レビューで確かめた形 (正規表現の印・組で閉じていること・単語単位の長い行・同一視の件数の参照) ----

/// 解析の結果の印 (同一視が効いているか)。
fn marked(q: &SearchQuery) -> bool {
    q.analyze()
        .ok()
        .flatten()
        .is_some_and(|p| p.compat_folded())
}

// 03 §12.4.1 の「検索バーの表示」: 書き換えで一致する文字が変わる検索語にだけ印が付く (互換文字を書いていない `\p{Sm}` にも
// 付き、組の両側を含むクラスには付かない)
#[test]
fn 正規表現の印は書き換えで一致する文字が変わるときだけ付く() {
    for p in [
        "〜",
        r"\x{301C}",
        r"\u{FF5E}",
        r"\U0000301C",
        "[〜]",
        "[^〜]",
        r"\p{Sm}",
        r"\P{Sm}",
        r"[^\P{Sm}]",
        r"[\x{2010}-\x{2014}]",
        r"[[〜]&&[～]]",
        r"[〜-～]",
        "(?i)[^a〜]",
        r"[\p{Pd}&&\p{Sm}]",
        // 選択の両側に組の文字を書いても、リテラルの互換文字なので付く (03 §12.4.1 の注)
        "〜|～",
    ] {
        assert!(marked(&re(p)), "印あり: {p}");
    }
    for p in [
        "[〜～]",
        r"\S",
        ".",
        "[^a]",
        r"[[〜]&&a]",
        "(?-u:〜)",
        "¥",
        r"[\p{Sm}\p{Pd}\p{Po}\p{Sc}]",
        r"[^\p{Sm}\p{Pd}\p{Po}\p{Sc}]",
    ] {
        assert!(!marked(&re(p)), "印なし: {p}");
    }
    assert!(!marked(&re("〜").with_compat_fold(false)));
    assert!(!marked(&lit("〜").with_compat_fold(false)));
}

/// 組の性質の試験の検索語 (集合の演算・入れ子・組の両側にまたがる範囲・否定の否定・Unicode のクラス)。
const CLOSED_PATS: &[&str] = &[
    "〜",
    "[^〜]",
    r"\P{Sm}",
    r"[\p{Sm}--−]",
    r"[[〜]&&[～]]",
    r"[〜-～]",
    r"[‖-∥]",
    r"[\x{2010}-\x{2014}]+",
    r"(?i)[^a〜]",
    r"[^\P{Sm}]",
    r"[\p{Pd}&&\p{Sm}]",
    r"[\p{Sc}~~[¢]]",
    r"[\p{Sc}~~[￠£]]",
    r"\p{Pd}",
    r"\p{Po}+",
    r"[^\p{Sm}\p{Pd}]",
    r"[[:^alpha:]&&[^〜]]",
    r"\x{FF5E}",
    r"[\u{2015}]",
    "(?x) 〜 # c",
    "[^[^〜]]",
    "(?i:[〜])",
    r"[〜&&\p{Sm}]",
    r"[^\p{Sm}--[^〜]]",
    r"[\p{Sm}--\p{Sm}]",
    r"[^‖--∥]",
    r"[\W&&[^\p{Sm}]]",
    r"\p{Sm}{2}",
    r"[\p{Any}--[〜¢]]",
    r"[\p{Sk}\p{So}]",
    r"[\x{A0}-\x{FF}]",
    r"[\x{FF00}-\x{FFEF}]",
    r"[^\x{FF00}-\x{FFEF}]",
    r"\b〜",
    r"(?u:[^a])",
    r"(?i)\p{Sm}|[^\p{Lu}〜]",
    r"[\p{Sm}&&[^\p{Pd}]]|[\x{FF0D}-\x{FF5E}]{2}",
    r"(?:―|—)+",
    r"[^\p{Sm}]{2}",
    r"\p{Sc}",
];

/// 組の性質の試験の文書の文字。
const CLOSED_DOC: &[char] = &[
    '〜', '～', '‖', '∥', '−', '－', '¢', '￠', '£', '￡', '¬', '￢', '¦', '￤', '—', '―', '¥',
    '￥', 'a', ' ', '-',
];

fn to_first(s: &str) -> String {
    s.chars()
        .map(|c| PAIRS.iter().find(|p| p.1 == c).map_or(c, |p| p.0))
        .collect()
}

fn to_second(s: &str) -> String {
    s.chars()
        .map(|c| PAIRS.iter().find(|p| p.0 == c).map_or(c, |p| p.1))
        .collect()
}

/// バイト範囲を char の番号の範囲に。
fn char_ranges(doc: &str, v: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let idx = |b: usize| doc[..b].chars().count();
    v.iter().map(|&(s, e)| (idx(s), idx(e))).collect()
}

/// `unfolded` (同一視を切った書き換えの検索語) が、文書 `doc` の上で、前後の文脈のまま `[s, e)` にちょうど一致できるか
/// (同一視で当たった件数の参照。文書の全体を探す文字列にし、範囲を一致に限って始端に固定する)。
fn exact_on_whole_doc(unfolded: &str, insensitive: bool, doc: &str, s: usize, e: usize) -> bool {
    use regex_automata::nfa::thompson::pikevm::PikeVM;
    use regex_automata::util::syntax;
    use regex_automata::{Anchored, Input, MatchKind};
    let vm = PikeVM::builder()
        .syntax(
            syntax::Config::new()
                .case_insensitive(insensitive)
                .multi_line(true)
                .crlf(true),
        )
        .configure(PikeVM::config().match_kind(MatchKind::All))
        .build(unfolded)
        .expect("同一視を切った検索語");
    let mut cache = vm.create_cache();
    vm.find(
        &mut cache,
        Input::new(doc).span(s..e).anchored(Anchored::Yes),
    )
    .is_some_and(|m| m.end() == e)
}

/// 同一視の件数の性質の試験の部品 (選択・`?`・`\b`・`$`・`\z` と組み合わせる)。
const FOLDED_ATOMS: &[&str] = &[
    "〜", "～", "—", "―", "a", "b", "c", r"\d", r"\b", "$", "^", r"\z", "[〜～]", "[〜a]",
    r"\p{Sm}",
];

fn folded_pattern() -> impl Strategy<Value = String> {
    let leaf = prop::sample::select(FOLDED_ATOMS).prop_map(str::to_owned);
    leaf.prop_recursive(3, 12, 3, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 1..4).prop_map(|v| v.concat()),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| format!("{a}|{b}")),
            inner.clone().prop_map(|a| format!("(?:{a})?")),
            inner.prop_map(|a| format!("(?:{a})+")),
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(512),
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    // 同一視した検索は組で閉じている: どの検索語でも、文書の組の文字を相手の側にそろえても、一致の文字の位置は変わらない
    #[test]
    fn 同一視した検索は組で閉じている(
        pi in 0..CLOSED_PATS.len(),
        doc in prop::collection::vec(prop::sample::select(CLOSED_DOC), 0..16).prop_map(|v| v.into_iter().collect::<String>()),
    ) {
        let p = CLOSED_PATS[pi];
        let q = re(p);
        prop_assert!(q.analyze().is_ok(), "{}", p);
        let a = char_ranges(&doc, &find_all(&doc, &q));
        let d1 = to_first(&doc);
        let d2 = to_second(&doc);
        prop_assert_eq!(&a, &char_ranges(&d1, &find_all(&d1, &q)), "{} 文書 {:?}", p, doc);
        prop_assert_eq!(&a, &char_ranges(&d2, &find_all(&d2, &q)), "{} 文書 {:?}", p, doc);
        // 組に無い文字の一致は、同一視の ON / OFF で変わらない (同一視は組の文字の扱いだけを変える)
        for ch in doc.chars().filter(|c| partner(*c).is_none()) {
            let one = ch.to_string();
            prop_assert_eq!(
                find_all(&one, &re(p).with_compat_fold(false)),
                find_all(&one, &q),
                "{} / {:?}", p, ch
            );
        }
    }

    // 同一視で当たった件数 (`folded`) は、すべて置換で置き換える各一致について「同一視を切った検索語が、文書の全体の上で、
    // 前後の文脈のまま、その範囲にちょうど一致できない」ものの数と同じ (03 §12.4.1 の「置換」。窓の端で `$`・`\b`・`\z` が
    // 偽って成り立たず、選択の優先に左右されない)
    #[test]
    fn 同一視で当たった件数は文書の全体で確かめた値と同じ(
        p in folded_pattern(),
        doc in prop::collection::vec(
            prop::sample::select(&["〜", "～", "—", "―", "a", "b", "c", "1", "22", " ", "\n", "\r\n"][..]),
            0..14,
        ).prop_map(|v| v.concat()),
    ) {
        let q = re(&p);
        let Some(plan) = q.analyze().ok().flatten() else {
            return Ok(());
        };
        let Some(unfolded) = re(&p)
            .with_compat_fold(false)
            .analyze()
            .ok()
            .flatten()
            .and_then(|x| x.regex_pattern().map(str::to_owned))
        else {
            return Ok(());
        };
        let matches = find_all(&doc, &q);
        // 置換の文字列 `#` は文書に無いので、どの一致も置き換える
        let (_, count, folded) = replace_all(&doc, &q, "#");
        prop_assert_eq!(count, matches.len() as u64);
        let want = if plan.compat_folded() {
            matches
                .iter()
                .filter(|&&(s, e)| !exact_on_whole_doc(&unfolded, !q.is_case_sensitive(), &doc, s, e))
                .count() as u64
        } else {
            0
        };
        prop_assert_eq!(folded, want, "検索語 {:?} 文書 {:?} 一致 {:?}", p, doc, matches);
    }
}

// 互換文字を含むリテラル検索 + 単語単位は、行の窓の経路で行の境界に揃えた核で探す。窓の核を小さく、窓の上限を短くしても
// (連結の上限より長い行があっても)、結果は既定の大きさと同じ (レビューで確かめた形)
#[test]
fn 互換文字を含む単語単位のリテラル検索は窓の大きさに依らない() {
    use eve_core::{ScanCtx, SearchTuning, scan_range};
    let tuning = SearchTuning {
        core_bytes: 4,
        concat_max: 64,
    };
    let line = format!("〜 {} ～ あ\n", "x".repeat(200));
    let doc = format!("{line}〜 y\n");
    let t = Text::from(doc.as_str());
    for q in [
        lit("〜").with_whole_word(true),
        lit("〜").with_whole_word(true).with_compat_fold(false),
        lit("〜"),
    ] {
        let small = q.clone().with_tuning(tuning);
        let m = small.matcher().unwrap();
        let mut got = Vec::new();
        let cancel = CancelToken::new();
        let end = scan_range(
            &t,
            &m,
            ByteRange::new(ByteOff(0), ByteOff(t.len_bytes())),
            &mut ScanCtx::new(&cancel),
            &mut |r| {
                got.push((r.start.0, r.end.0));
                true
            },
        );
        assert_eq!(end, eve_core::ScanEnd::Done);
        assert_eq!(got, find_all(&doc, &q), "{q:?}");
    }
    assert_eq!(
        find_all(&doc, &lit("〜").with_whole_word(true)).len(),
        3,
        "行頭の `〜`・`～`・次の行の `〜`"
    );
}
