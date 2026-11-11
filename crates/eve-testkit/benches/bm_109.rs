//! BM-109 / BM-109r: 文書の検索の全件の走査 (docs/09-testing-quality.md §5.2、01 NFR-109。参考値として記録する。
//! docs/review/v0.3-findings.md の §3.3 の 5)。ヘッドレス (`Harness`・`MemFs`) で、検索を始めてから全件を数え終える
//! (`SearchCount::Exact`) までの時間を測る。
//!
//! **BM-109** (100 MB のリテラル検索。チャンクごとの memmem の経路)。検索語は 09 §5.2 のとおり:
//!
//! - `ascii`: ASCII の語 (`k` `K` `s` `S` を含まない) を大小の区別あり (`Sensitive`) で。
//! - `ascii-i`: 同じ語を大小の区別なし (`Insensitive`) で (ASCII の大小無視は memmem の経路のまま)。
//! - `j3` / `j10`: 日本語の 3 文字・10 文字の語 (BM-109j。区別の無い文字だけなので `Smart` でも memmem)。
//! - `many`: 一致の多い語 (文書の各行にある語)。
//!
//! **BM-109r** (行単位の方式。1 行で済む正規表現と、行ごとの経路になるリテラル。v0.4 の S-04 で、複数行の正規表現の
//! 候補の方式と比べる相手、08 §3)。100 MB (平均 80 B の行) と、10 MB の 1 行 (`-1line`) の 2 つの文書で測る:
//!
//! - `digits`: 文字クラスで始まる正規表現 (`\d+ a`)。一致が多い。
//! - `anchor`: 行頭に固定した正規表現 (`^末尾`)。ほとんどの行は一致しない (行ごとの固定の費用が出る)。
//! - `ci`: 大小無視の正規表現 (`(?i)zebra`)。
//! - `ks`: `k` を含む ASCII の語の大小無視のリテラル (`quick`。行ごとの経路になる、03 §12.1 の経路 3)。
//!
//! 一致の数が数える上限 (`search.count_limit` = 100 万) に届くと走査がそこで止まるので、どの検索語も上限より
//! 少ない数だけ一致するものにしてある (文書の作り方を変えたら、検索語も見直す)。
//!
//! BM-108 (最初の一致までの時間) は、ヘッドレスのジョブ (`ManualJobSpawner`) が 1 回で最後まで走るので分けて測れない。
//! 本物のワーカーで測る計測 (`bm_pty` の計測用ビルド) に足すまで、ここでは測らない。
//!
//! ```text
//! cargo bench -p eve-testkit --bench bm_109            # 100 MB (と 10 MB の 1 行)
//! cargo bench -p eve-testkit --bench bm_109 -- --quick # 10 MB (と 1 MB の 1 行。動作確認。記録には使わない)
//! cargo bench -p eve-testkit --bench bm_109 -- --json  # 1 行ずつの JSON (xtask bench が集める)
//! ```

#![allow(clippy::print_stdout, clippy::print_stderr)]
// 検索を始める・止めるのに EditorState を直接使う (試験と計測だけに許す Editor::state_mut、STR-B3)
#![allow(clippy::disallowed_methods)]

use std::time::{Duration, Instant};

use eve_core::{Case, SearchCount, SearchQuery};
use eve_testkit::Harness;

/// 数える一致の上限 (`search.count_limit`、07 §5.5)。届くと走査が止まるので、全件の走査の計測にならない。
const COUNT_LIMIT: u64 = 1_000_000;

/// 平均 80 B の行 (ASCII と日本語が混ざる) を `bytes` 以上になるまで並べる。決定的。末尾にだけ目印を置く。
/// `sep` は行の区切り (`\n`。1 行の文書を作るときは空白)。
fn corpus(bytes: usize, sep: char) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes + 256);
    let mut i = 0u64;
    while out.len() < bytes {
        let line = if i.is_multiple_of(3) {
            format!("{i:08} 日本語の行です。テキストエディタの試験データ。0123456789{sep}")
        } else {
            format!("{i:08} the quick brown fox jumps over the lazy dog 0123456789 abcdef{sep}")
        };
        out.extend_from_slice(line.as_bytes());
        i += 1;
    }
    out.extend_from_slice(format!("末尾の目印 zebra 終わりの十文字の日本語です{sep}").as_bytes());
    out
}

/// 1 つの検索語の計測。
struct Case1 {
    /// 記録の ID (`BM-109-ascii` など)。
    id: String,
    query: SearchQuery,
}

fn literal(id: &str, pattern: &str, case: Case) -> Case1 {
    Case1 {
        id: id.to_owned(),
        query: SearchQuery::literal(pattern).with_case(case),
    }
}

fn regex(id: &str, pattern: &str) -> Case1 {
    Case1 {
        id: id.to_owned(),
        query: SearchQuery::literal(pattern)
            .with_regex(true)
            .with_case(Case::Sensitive),
    }
}

/// 行単位の方式の検索語 (BM-109r)。`suffix` は ID の末尾 (1 行の文書は `-1line`)。
fn line_route_cases(suffix: &str) -> Vec<Case1> {
    vec![
        regex(&format!("BM-109r-digits{suffix}"), r"\d+ a"),
        regex(&format!("BM-109r-anchor{suffix}"), "^末尾"),
        regex(&format!("BM-109r-ci{suffix}"), "(?i)zebra"),
        literal(&format!("BM-109r-ks{suffix}"), "QUICK", Case::Insensitive),
    ]
}

/// `data` を開いて、`cases` を順に測る。
fn measure(data: &[u8], mb: usize, cases: &[Case1], quick: bool, json: bool) {
    let mut h = Harness::new()
        .with_size(120, 40)
        .with_file("/w/big.txt", data);
    h.open("/w/big.txt");
    h.run_jobs();
    for c in cases {
        let id = &c.id;
        let runs = if quick { 1 } else { 3 };
        let mut times: Vec<Duration> = Vec::with_capacity(runs);
        let mut total = 0;
        for _ in 0..runs {
            let t = Instant::now();
            h.editor.state_mut().search_start(c.query.clone());
            h.run_jobs();
            let d = t.elapsed();
            match h.editor.state().search_count() {
                SearchCount::Exact { total: n, .. } | SearchCount::AtLeast { total: n, .. } => {
                    total = n;
                }
                other => panic!("{id}: 全件を数え終えていない: {other:?}"),
            }
            assert!(
                total < COUNT_LIMIT,
                "{id}: 一致が数える上限に届いた (走査が途中で止まる)。検索語を見直す"
            );
            times.push(d);
            h.editor.state_mut().search_stop();
        }
        times.sort();
        let med = times[times.len() / 2];
        if json {
            println!(
                "{{\"id\":\"{id}\",\"n\":{runs},\"p50_ms\":{:.1},\"matches\":{total},\"mb\":{mb},\"quick\":{quick}}}",
                med.as_secs_f64() * 1e3
            );
        } else {
            println!(
                "{id} ({mb} MB, {:?}): 全件の走査 {med:?} (一致 {total} 件、{runs} 回の中央値){}",
                c.query.pattern(),
                if quick { " [--quick]" } else { "" }
            );
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let quick = args.iter().any(|a| a == "--quick");
    let json = args.iter().any(|a| a == "--json");

    // 行に分かれた文書 (100 MB)
    let mb = if quick { 10 } else { 100 };
    // `k` `s` を含む ASCII の語の大小無視は行ごとの経路になるので、BM-109 では使わない (09 §5.2)
    let mut cases = vec![
        literal("BM-109-ascii", "zebra", Case::Sensitive),
        literal("BM-109-ascii-i", "ZEBRA", Case::Insensitive),
        literal("BM-109-j3", "終わり", Case::Smart),
        literal("BM-109-j10", "終わりの十文字の日本語", Case::Smart),
        literal("BM-109-many", "fox", Case::Sensitive),
    ];
    cases.extend(line_route_cases(""));
    let data = corpus(mb << 20, '\n');
    measure(&data, mb, &cases, quick, json);
    drop(data);

    // 1 行の文書 (10 MB)
    let mb = if quick { 1 } else { 10 };
    let data = corpus(mb << 20, ' ');
    measure(&data, mb, &line_route_cases("-1line"), quick, json);
}
