//! BM-103a: キー入力の core 区間 (docs/09-testing-quality.md §5.2、01 NFR-103 の (a))。
//! ヘッドレスで `InputEvent::Text("a")` を渡し、`handle_input` の入口から Frame の生成完了までを 10,000 回
//! (先頭 1,000 回はウォームアップとして除く) 測り、p50 / p99 / p99.9 を出す。条件は 01 §3.0.1 に合わせる:
//! 端末 120×40、1 MB の日本語混在 UTF-8、文書の中央で入力。**診断と記録用** で、目標値の合否には使わない
//! (合否は端末込みの BM-103b)。
//!
//! ```text
//! cargo bench -p eve-testkit --bench bm_103a
//! cargo bench -p eve-testkit --bench bm_103a -- --quick   # 動作確認 (回数を減らす。記録には使わない)
//! cargo bench -p eve-testkit --bench bm_103a -- --json    # 1 行の JSON (xtask bench が集める)
//! cargo bench -p eve-testkit --bench bm_103a -- --wrap    # 折り返し ON (`view.wrap = word`) で測る (03 §14.3.1)
//! ```

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::time::{Duration, Instant};

use eve_core::Args;
use eve_testkit::Harness;

/// 平均 80 B の行 (ASCII と日本語が混ざる) を `bytes` 以上になるまで並べる。決定的。
fn corpus(bytes: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes + 128);
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
    out
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx]
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let quick = args.iter().any(|a| a == "--quick");
    let json = args.iter().any(|a| a == "--json");
    let wrap = args.iter().any(|a| a == "--wrap");
    let (runs, warmup) = if quick { (1_000, 100) } else { (10_000, 1_000) };

    let data = corpus(1024 * 1024);
    let lines = data.iter().filter(|b| **b == b'\n').count();
    let mut h = Harness::new()
        .with_size(120, 40)
        .with_file("/w/doc.txt", &data);
    drop(data);
    h.open("/w/doc.txt");
    h.run_jobs();
    if wrap {
        h.cmd("view.toggle_wrap");
    }
    h.cmd_with(
        "cursor.goto_line",
        Args::new().with_int("line", (lines / 2) as i64 + 1),
    );
    // 初回の Frame (キャッシュを温める) は計測に含めない
    let _ = h.frame();

    let mut samples = Vec::with_capacity(runs);
    // 内訳 (診断用): 入力の処理 (`handle_input` ほか) と Frame 生成
    let mut handle_samples = Vec::with_capacity(runs);
    for i in 0..runs {
        // 行が伸び続けないよう、時々改行する (入力そのものは "a")
        if i % 60 == 59 {
            h.text("\n");
            let _ = h.frame();
        }
        let t = Instant::now();
        h.text("a");
        let t_handle = t.elapsed();
        let f = h.frame();
        let d = t.elapsed();
        handle_samples.push(t_handle);
        std::hint::black_box(&f);
        samples.push(d);
    }
    let mut hv = handle_samples.split_off(warmup);
    hv.sort();
    let mut v = samples.split_off(warmup);
    v.sort();
    let (p50, p99, p999, max) = (
        percentile(&v, 0.5),
        percentile(&v, 0.99),
        percentile(&v, 0.999),
        v[v.len() - 1],
    );
    if json {
        println!(
            "{{\"id\":\"BM-103a\",\"n\":{},\"p50_us\":{:.1},\"p99_us\":{:.1},\"p999_us\":{:.1},\"max_us\":{:.1},\"quick\":{quick}}}",
            v.len(),
            p50.as_secs_f64() * 1e6,
            p99.as_secs_f64() * 1e6,
            p999.as_secs_f64() * 1e6,
            max.as_secs_f64() * 1e6,
        );
    } else {
        println!(
            "BM-103a キー入力の core 区間 (120x40, 1 MB, {runs} 回, ウォームアップ {warmup} 回){}",
            if quick {
                " [--quick: 記録には使わない]"
            } else {
                ""
            }
        );
        println!(
            "  内訳: 入力の処理 (handle_input) p50 {:?}  p99 {:?}  / Frame 生成は残り",
            percentile(&hv, 0.5),
            percentile(&hv, 0.99)
        );
        println!(
            "  入力 → Frame 生成: n={} p50 {p50:?}  p99 {p99:?}  p99.9 {p999:?}  max {max:?}  (診断用。目標 p50 2 ms / p99 8 ms は端末込みの BM-103b で判定)",
            v.len()
        );
    }
}
