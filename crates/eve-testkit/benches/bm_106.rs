//! BM-106: 長大行 (1 行 10 MB、改行なし) の表示・`End`・横スクロール・垂直スクロール (docs/09-testing-quality.md §5.2、
//! 01 NFR-106 AC-3)。v0.1 の判定: 閾値 (1 MiB) を超える行を含む文書は読み取り専用で開き、その行は先頭 64 KiB だけを
//! 表示する経路で、各操作の p99 ≤ 100 ms (S-03)。ASCII / 日本語 / 絵文字混在の 3 種を別のケースとして測る。
//! ヘッドレス (core の区間)。**開発機の値は目安で、判定は基準機で行う**。
//!
//! ```text
//! cargo bench -p eve-testkit --bench bm_106
//! cargo bench -p eve-testkit --bench bm_106 -- --quick   # 動作確認 (回数を減らす。記録には使わない)
//! cargo bench -p eve-testkit --bench bm_106 -- --json    # 1 行ずつの JSON
//! ```

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::time::{Duration, Instant};

use eve_testkit::Harness;

const LINE_BYTES: usize = 10 * 1024 * 1024;

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    sorted[((sorted.len() as f64 - 1.0) * p).round() as usize]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let quick = args.iter().any(|a| a == "--quick");
    let json = args.iter().any(|a| a == "--json");
    let runs = if quick { 20 } else { 200 };

    let cases: [(&str, &str); 3] = [
        ("ascii", "abcdefghij"),
        ("japanese", "日本語のテキスト"),
        ("emoji", "a日😀b"),
    ];
    for (id, unit) in cases {
        let line = unit.repeat(LINE_BYTES / unit.len() + 1);
        let mut data = line.into_bytes();
        data.extend_from_slice(b"\nsecond\n");
        // 開く → 最初の Frame (測るたびに新しい Harness で開く。MemFs の用意は測らない)
        let mut open = Vec::new();
        for _ in 0..runs.min(10) {
            let mut h = Harness::new()
                .with_size(120, 40)
                .with_file("/w/long.txt", &data);
            let t = Instant::now();
            h.open("/w/long.txt");
            h.run_jobs();
            let f = h.frame();
            open.push(t.elapsed());
            std::hint::black_box(&f);
        }
        let mut h = Harness::new()
            .with_size(120, 40)
            .with_file("/w/long.txt", &data);
        drop(data);
        h.open("/w/long.txt");
        h.run_jobs();
        let _ = h.frame();
        // End → Frame
        let mut end = Vec::new();
        for _ in 0..runs {
            h.keys("home");
            let _ = h.frame();
            let t = Instant::now();
            h.keys("end");
            let f = h.frame();
            end.push(t.elapsed());
            std::hint::black_box(&f);
        }
        // 横スクロール (右へ 1 文字) → Frame
        let mut hscroll = Vec::new();
        h.keys("home");
        for _ in 0..runs {
            let t = Instant::now();
            h.keys("right");
            let f = h.frame();
            hscroll.push(t.elapsed());
            std::hint::black_box(&f);
        }
        // 垂直スクロール (長い行と次の行の間を行き来) → Frame
        let mut vscroll = Vec::new();
        for i in 0..runs {
            let t = Instant::now();
            h.keys(if i % 2 == 0 { "down" } else { "up" });
            let f = h.frame();
            vscroll.push(t.elapsed());
            std::hint::black_box(&f);
        }
        for (op, mut v) in [
            ("open", open),
            ("end", end),
            ("hscroll", hscroll),
            ("vscroll", vscroll),
        ] {
            v.sort();
            let (p50, p99, max) = (percentile(&v, 0.5), percentile(&v, 0.99), v[v.len() - 1]);
            if json {
                println!(
                    "{{\"id\":\"BM-106\",\"case\":\"{id}\",\"op\":\"{op}\",\"n\":{},\"p50_ms\":{:.2},\"p99_ms\":{:.2},\"max_ms\":{:.2},\"quick\":{quick}}}",
                    v.len(),
                    ms(p50),
                    ms(p99),
                    ms(max)
                );
            } else {
                println!(
                    "BM-106 {id:8} {op:8} n={:4} p50 {:7.2} ms  p99 {:7.2} ms  max {:7.2} ms  (目標 p99 ≤ 100 ms → {})",
                    v.len(),
                    ms(p50),
                    ms(p99),
                    ms(max),
                    if ms(p99) <= 100.0 { "OK" } else { "超過" }
                );
            }
        }
    }
    if quick && !json {
        println!("(--quick は動作確認用です。記録には使いません)");
    }
}
