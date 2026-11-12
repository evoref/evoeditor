//! 検索の走査の速さ (参考値、NFR-108 / 109)。時刻を測るので通常のテストでは走らせない。
//! `cargo test -p eve-core --release --test search_speed -- --ignored --nocapture`

#![allow(clippy::disallowed_methods, clippy::print_stdout)]

use eve_core::{CancelToken, Case, SearchQuery, scan_bytes};
use eve_text::{ByteOff, ByteRange, Text, TextRead};

#[test]
#[ignore = "計測用"]
fn 走査の速さ() {
    let line = "0123456789 the quick brown fox jumps over the lazy dog 日本語の行です。0123456789
";
    let doc = line.repeat(100 * 1024 * 1024 / line.len());
    let t = Text::from(doc.as_str());
    let full = ByteRange::new(ByteOff(0), ByteOff(t.len_bytes()));
    for (name, q) in [
        (
            "区別あり",
            SearchQuery::literal("zzzz").with_case(Case::Sensitive),
        ),
        (
            "大小無視 (ASCII)",
            SearchQuery::literal("ZZZZ").with_case(Case::Insensitive),
        ),
        ("日本語", SearchQuery::literal("存在しない")),
        // `k` `s` を含む ASCII の語の大小無視は、行ごとの正規表現の経路になる (03 §12.4 の 3)
        (
            "大小無視 (s を含む ASCII、一致なし)",
            SearchQuery::literal("ZEBRAS").with_case(Case::Insensitive),
        ),
        (
            "大小無視 (k を含む ASCII、一致が多い)",
            SearchQuery::literal("QUICK").with_case(Case::Insensitive),
        ),
    ] {
        let m = q.matcher().unwrap();
        let start = std::time::Instant::now();
        scan_bytes(&t, &m, full, &CancelToken::new(), &mut |_| true);
        let sec = start.elapsed().as_secs_f64();
        println!("{name}: {:.0} MB/s ({:.2} s, 100 MB)", 100.0 / sec, sec);
    }
}

/// 打鍵ごとにメインスレッドで行う解析 (構文の解析・書き換え・k の計算) と、`bg` のジョブで行う `regex` のコンパイルの
/// 時間 (参考値、03 §12.3.1 の「メインスレッドでのコンパイル」)。
/// `cargo test -p eve-core --release --test search_speed -- --ignored --nocapture 解析`
#[test]
#[ignore = "計測用"]
fn 解析とコンパイルの時間() {
    let long = "The quick brown fox jumps over the lazy dog. ".repeat(5);
    let cases: [(&str, SearchQuery); 8] = [
        ("リテラル foo", SearchQuery::literal("foo")),
        (
            "大小無視のリテラル (s を含む 225 文字)",
            SearchQuery::literal(&long).with_case(Case::Insensitive),
        ),
        ("正規表現 fo+", SearchQuery::literal("fo+").with_regex(true)),
        (
            r"正規表現 (\w+)\s*=\s*(\d+)",
            SearchQuery::literal(r"(\w+)\s*=\s*(\d+)").with_regex(true),
        ),
        (
            r#"正規表現 "[^"]*"|\p{L}+\P{L}"#,
            SearchQuery::literal(r#""[^"]*"|\p{L}+\P{L}"#).with_regex(true),
        ),
        (
            r"正規表現 (?i)[\s\S]{0,8}\w{3}",
            SearchQuery::literal(r"(?i)[\s\S]{0,8}\w{3}").with_regex(true),
        ),
        (
            r"正規表現 [^\n]{0,3000}zq",
            SearchQuery::literal(r"[^\n]{0,3000}zq").with_regex(true),
        ),
        (
            r"正規表現 (?s).{0,1000}zq",
            SearchQuery::literal(r"(?s).{0,1000}zq").with_regex(true),
        ),
    ];
    for (name, q) in cases {
        let n = 50;
        let start = std::time::Instant::now();
        for _ in 0..n {
            let _ = q.clone().with_case(q.case()).analyze();
        }
        let analyze = start.elapsed().as_secs_f64() * 1000.0 / f64::from(n);
        let start = std::time::Instant::now();
        let n2 = 5;
        for _ in 0..n2 {
            let _ = q.clone().with_case(q.case()).compile();
        }
        let compile = start.elapsed().as_secs_f64() * 1000.0 / f64::from(n2);
        println!("{name}: 解析 {analyze:.3} ms / 解析とコンパイル {compile:.2} ms");
    }
}
