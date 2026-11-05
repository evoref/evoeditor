//! BM-109: 100 MB の文書のリテラル検索の全件の走査 (docs/09-testing-quality.md §5.2、01 NFR-109。参考値として記録する。
//! docs/review/v0.3-findings.md の §3.3 の 5)。ヘッドレス (`Harness`・`MemFs`) で、検索を始めてから全件を数え終える
//! (`SearchCount::Exact`) までの時間を測る。検索語は 09 §5.2 のとおり:
//!
//! - `ascii`: ASCII の語 (`k` `K` `s` `S` を含まない) を大小の区別あり (`Sensitive`) で。
//! - `ascii-i`: 同じ語を大小の区別なし (`Insensitive`) で (ASCII の大小無視は memmem の経路のまま)。
//! - `j3` / `j10`: 日本語の 3 文字・10 文字の語 (BM-109j。区別の無い文字だけなので `Smart` でも memmem)。
//! - `many`: 一致の多い語 (文書の各行にある語)。
//!
//! BM-108 (最初の一致までの時間) は、ヘッドレスのジョブ (`ManualJobSpawner`) が 1 回で最後まで走るので分けて測れない。
//! 本物のワーカーで測る計測 (bm_pty の計測用ビルド) に足すまで、ここでは測らない。
//!
//! ```text
//! cargo bench -p eve-testkit --bench bm_109            # 100 MB
//! cargo bench -p eve-testkit --bench bm_109 -- --quick # 10 MB (動作確認。記録には使わない)
//! cargo bench -p eve-testkit --bench bm_109 -- --json  # 1 行ずつの JSON (xtask bench が集める)
//! ```

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::time::{Duration, Instant};

use eve_core::{Case, SearchCount, SearchQuery};
use eve_testkit::Harness;

/// 平均 80 B の行 (ASCII と日本語が混ざる) を `bytes` 以上になるまで並べる。決定的。末尾にだけ目印を置く。
fn corpus(bytes: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes + 256);
    let mut i = 0u64;
    while out.len() < bytes {
        let line = if i.is_multiple_of(3) {
            format!("{i:08} 日本語の行です。テキストエディタの試験データ。0123456789\n")
        } else {
            format!("{i:08} the quick brown fox jumps over the lazy dog 0123456789 abcdef\n")
        };
        out.extend_from_slice(line.as_bytes());
        i += 1;
    }
    out.extend_from_slice("末尾の目印 zebra 終わりの十文字の日本語です\n".as_bytes());
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let quick = args.iter().any(|a| a == "--quick");
    let json = args.iter().any(|a| a == "--json");
    let mb = if quick { 10 } else { 100 };

    let data = corpus(mb << 20);
    let mut h = Harness::new()
        .with_size(120, 40)
        .with_file("/w/big.txt", &data);
    drop(data);
    h.open("/w/big.txt");
    h.run_jobs();

    // (名前, 検索語, 大小の扱い)。`k` `s` を含む ASCII の語は行単位の正規表現の経路になるので使わない (09 §5.2)
    let cases: [(&str, &str, Case); 5] = [
        ("ascii", "zebra", Case::Sensitive),
        ("ascii-i", "ZEBRA", Case::Insensitive),
        ("j3", "終わり", Case::Smart),
        ("j10", "終わりの十文字の日本語", Case::Smart),
        ("many", "fox", Case::Sensitive),
    ];
    for (name, pattern, case) in cases {
        let runs = if quick { 1 } else { 3 };
        let mut times: Vec<Duration> = Vec::with_capacity(runs);
        let mut total = 0;
        for _ in 0..runs {
            let t = Instant::now();
            h.editor
                .state_mut()
                .search_start(SearchQuery::literal(pattern).with_case(case));
            h.run_jobs();
            let d = t.elapsed();
            match h.editor.state().search_count() {
                SearchCount::Exact { total: n, .. } | SearchCount::AtLeast { total: n, .. } => {
                    total = n;
                }
                other => panic!("{name}: 全件を数え終えていない: {other:?}"),
            }
            times.push(d);
            h.editor.state_mut().search_stop();
        }
        times.sort();
        let med = times[times.len() / 2];
        if json {
            println!(
                "{{\"id\":\"BM-109-{name}\",\"n\":{runs},\"p50_ms\":{:.1},\"matches\":{total},\"mb\":{mb},\"quick\":{quick}}}",
                med.as_secs_f64() * 1e3
            );
        } else {
            println!(
                "BM-109-{name} ({mb} MB, {pattern:?}, {case:?}): 全件の走査 {med:?} (一致 {total} 件、{runs} 回の中央値){}",
                if quick { " [--quick]" } else { "" }
            );
        }
    }
}
