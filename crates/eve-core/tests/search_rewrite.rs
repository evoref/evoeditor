//! 検索語の書き換えの表 (docs/03 §12.3.1 の「行の終端に一致する書き方」の表、09 §3.2 の検索の拡張の行の L0 のゴールデン)。
//!
//! 表の書き方と、表の意味を保つように決めた細目 (入れ子のクラス・集合の演算・フラグ) を、書き換えた検索語・k・最大の長さ・
//! 行の終端を明示したか で記録する。比べ方と書き換え方は `eve_testkit::golden` (`EVE_UPDATE_GOLDEN=1` で作る・書き換える)。
//! 書き換えの意味の確かめ (CRLF の例・性質) は `eve-core` の `search::rewrite` の単体試験。

use std::fmt::Write as _;
use std::path::PathBuf;

use eve_core::{Case, LineSpan, SearchQuery};
use eve_testkit::golden;

fn k_text(l: LineSpan) -> String {
    match l {
        LineSpan::Bounded(k) => k.to_string(),
        LineSpan::Unbounded => "∞".to_owned(),
    }
}

/// 03 §12.3.1 の表の書き方 (と、表の意味を保つように決めた入れ子のクラス・集合の演算・フラグの細目)。
const TABLE: &[&str] = &[
    // クラスの外の `\n` (符号位置で書いたものを含む)
    r"\n",
    r"\x0A",
    r"\x{A}",
    r"\u000A",
    r"a\nb",
    r"\n\n",
    r"\n{2,}",
    r"\n\n+",
    r"\n+",
    r"\n{3}",
    // 続けて書いた `\r\n` と、グループ・量指定子を挟んだもの
    r"\r\n",
    r"\r\n+",
    r"\r(?:\n)",
    r"(\r\n)",
    // クラスの外の `\r`
    r"\r",
    r"\x0D",
    r"\x{D}",
    r"\r?\n",
    r"\r+",
    r"a\r|b",
    // 要素として `\n` / `\r` を書いたクラス
    r"[\n]",
    r"[ \t\n]",
    r"[\s\n]",
    r"[\s\n]{2}",
    r"[\s\n]*",
    r"[\x0A]",
    r"[\r]",
    r"[\r ]",
    r"[\r\n]",
    r"[\r\s]",
    r"[\n ]+",
    // すべての文字
    r"(?s).",
    r"(?s)a.*b",
    r"(?s).{0,3}",
    r"[\s\S]",
    r"[\w\W]",
    r"[\d\D]",
    r"[\s\S]*",
    r"\p{Any}",
    r"[\n\s\S]",
    // `.` (s なし)
    r".",
    r"a.b",
    r"(?-R).",
    r"(?-R:.)+",
    // 否定・略記・Unicode のクラスと範囲
    r#"[^"]"#,
    r#""[^"]*""#,
    r"[^\n]",
    r"[^\r\n]",
    r"[^a]",
    r"\s",
    r"\s+",
    r"\S",
    r"\W",
    r"\D",
    r"\w",
    r"\d",
    r"\p{L}",
    r"\P{L}",
    r"\p{Cc}",
    r"[[:space:]]",
    r"[\x00-\x1F]",
    r"[\t-\r]",
    // 入れ子のクラス・集合の演算 (要素に数えない。集合から行の終端を除く)
    r"[a[\n]]",
    r"[\n&&\s]",
    r"[\s--\n]",
    r"[\s~~a]",
    // フラグ (挿む `^` は `(?mR:…)` で包むので、利用者の `(?-m)` などに左右されない)
    r"(?i)\n",
    r"(?-m)\r",
    r"(?x) \n \r ",
    r"(?-u:\s)",
    r"(?U)\s+",
    // 捕獲グループは足さない (番号と名前を保つ)
    r"(a)\n(?P<x>b)",
    r"(?<y>\r)(\s)",
    // 行の終端に関わらないもの (書き換えない)
    r"abc",
    r"^a$",
    r"\Aa\z",
    r"\bfoo\b",
    r"[a-z]+",
];

#[test]
fn 書き換えの表() {
    let mut out = String::from("# pattern\tsugar\trewritten\tk\tmax_len\texplicit\n");
    for p in TABLE {
        for sugar in [true, false] {
            let q = SearchQuery::literal(p)
                .with_regex(true)
                .with_case(Case::Sensitive)
                .with_newline_sugar(sugar);
            let plan = q
                .analyze()
                .unwrap_or_else(|e| panic!("{p:?}: {e:?}"))
                .expect("空でない検索語");
            let _ = writeln!(
                out,
                "{p}\t{}\t{}\t{}\t{}\t{}",
                if sugar { "on" } else { "off" },
                plan.regex_pattern().expect("正規表現の経路"),
                k_text(plan.lines()),
                plan.max_len().map_or("∞".to_owned(), |n| n.to_string()),
                plan.explicit_line_end(),
            );
        }
    }
    golden::check(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/search_rewrite.tsv"),
        &out,
    );
}

/// CP932 互換文字の同一視の書き換え (03 §12.4.1 の「正規表現」と「リテラル検索」の細目。M24 の (5))。
const COMPAT_TABLE: &[&str] = &[
    // リテラルの互換文字 (符号位置で書いたものを含む)
    "〜",
    "～",
    "a―b",
    r"\x{301C}",
    r"\x{FFE0}",
    "(?i)〜",
    // クラス: 組の片方だけを含めば相手を足す。否定は否定する前の集合に掛ける
    "[〜]",
    "[^〜]",
    "[〜a-c]",
    r"[\x{2010}-\x{2014}]",
    r"\p{Sm}",
    r"\P{Sm}",
    r"[^\p{Sm}]",
    r"[\p{Sm}--−]",
    r"[[〜]&&[～]]",
    // 行の終端の書き換えと重なるもの
    r"[〜\n]",
    r"[^〜\n]",
    r"(〜)\n(?P<x>―)",
    // 一致する文字が変わらないもの (元のまま。印も付けない)
    "[〜～]",
    r"\S",
    r"\W",
    ".",
    "[^a]",
    // 同一視しないもの
    "(?-u:〜)",
    "¥",
    "‾",
    r"\\",
    "~",
];

/// リテラル検索で互換文字を含む語 (エスケープして行の窓の経路で探す、03 §12.4.1)。(検索語, 大小の扱い)。
const COMPAT_LITERALS: &[(&str, Case)] = &[
    ("〜", Case::Smart),
    ("x〜y", Case::Smart),
    ("a.〜", Case::Sensitive),
    ("Ａ〜", Case::Insensitive),
    ("k〜", Case::Insensitive),
    ("¢£", Case::Smart),
    ("¥", Case::Smart),
];

#[test]
fn 互換文字の同一視の書き換えの表() {
    let mut out = String::from("# pattern\trewritten\tcompat_folded\tk_fold_off\n");
    for p in COMPAT_TABLE {
        let q = SearchQuery::literal(p)
            .with_regex(true)
            .with_case(Case::Sensitive);
        let plan = q
            .analyze()
            .unwrap_or_else(|e| panic!("{p:?}: {e:?}"))
            .expect("空でない検索語");
        let off = SearchQuery::literal(p)
            .with_regex(true)
            .with_case(Case::Sensitive)
            .with_compat_fold(false)
            .analyze()
            .unwrap()
            .unwrap();
        // 同一視は行の終端の数 (k) を変えない
        assert_eq!(plan.lines(), off.lines(), "{p}");
        let _ = writeln!(
            out,
            "{p}\t{}\t{}\t{}",
            plan.regex_pattern().expect("正規表現の経路"),
            plan.compat_folded(),
            k_text(off.lines()),
        );
    }
    out.push_str("# literal\tcase\trewritten\tcompat_folded\n");
    for (p, case) in COMPAT_LITERALS {
        let plan = SearchQuery::literal(p)
            .with_case(*case)
            .analyze()
            .unwrap()
            .unwrap();
        let _ = writeln!(
            out,
            "{p}\t{case:?}\t{}\t{}",
            plan.regex_pattern().unwrap_or("(memmem)"),
            plan.compat_folded(),
        );
    }
    golden::check(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/search_rewrite_compat.tsv"),
        &out,
    );
}
