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
    ] {
        let m = q.matcher().unwrap();
        let start = std::time::Instant::now();
        scan_bytes(&t, &m, full, &CancelToken::new(), &mut |_| true);
        let sec = start.elapsed().as_secs_f64();
        println!("{name}: {:.0} MB/s ({:.2} s, 100 MB)", 100.0 / sec, sec);
    }
}
