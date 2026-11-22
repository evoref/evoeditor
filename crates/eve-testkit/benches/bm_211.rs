//! BM-211: 複数カーソルの入力の core 区間 (docs/09-testing-quality.md §5.2、01 FR-211 AC-3 の (B)、03 §4.7.1)。参考値で、合否は
//! 付けない。1 MB の文書 (BM-103a と同じ日本語混在のコーパス) の先頭から 1,000 行 (`1k`) と 10,000 行 (`10k`、上限
//! `MAX_CURSORS`) の行頭にカーソルを置き、`InputEvent::Text("a")` → Frame の生成完了 (BM-103a と同じ区間) を 1,000 回
//! (先頭 100 回はウォームアップとして除く) 測って p50 / p99 を出す。各回の後に (測らずに) `Backspace` で入れた文字を消し、
//! 行が伸び続けないようにする。主のカーソルは先頭の行 (画面に多くのカーソルが見える)。端末 120×40。
//!
//! ```text
//! cargo bench -p eve-testkit --bench bm_211
//! cargo bench -p eve-testkit --bench bm_211 -- --quick   # 動作確認 (回数を減らす。記録には使わない)
//! cargo bench -p eve-testkit --bench bm_211 -- --json    # 1 行の JSON を 2 行 (xtask bench が集める)
//! ```

#![allow(clippy::print_stdout, clippy::print_stderr)]
#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "計測用の数値変換。値は十分に小さい"
)]

use std::time::{Duration, Instant};

use eve_testkit::Harness;
use eve_text::{ByteOff, MAX_CURSORS, Range, Selection};

/// 平均 80 B の行 (ASCII と日本語が混ざる) を `bytes` 以上になるまで並べる。決定的 (BM-103a と同じ)。
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

/// `n` 個のカーソル (先頭から `n` 行の行頭) で測る。結果は (p50, p99, max, 測った回数)。
fn measure(
    data: &[u8],
    n: usize,
    runs: usize,
    warmup: usize,
) -> (Duration, Duration, Duration, usize) {
    let mut h = Harness::new()
        .with_size(120, 40)
        .with_file("/w/doc.txt", data);
    h.open("/w/doc.txt");
    h.run_jobs();
    let starts: Vec<usize> = std::iter::once(0)
        .chain(
            data.iter()
                .enumerate()
                .filter(|(_, b)| **b == b'\n')
                .map(|(i, _)| i + 1),
        )
        .take(n)
        .collect();
    assert_eq!(starts.len(), n, "文書に {n} 行がある");
    let ranges: Vec<Range> = starts.iter().map(|&s| Range::point(ByteOff(s))).collect();
    h.set_selection(Selection::new(ranges, 0).expect("上限以下の Selection"));
    // 初回の Frame (キャッシュを温める) は計測に含めない
    let _ = h.frame();
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        let t = Instant::now();
        h.text("a");
        let f = h.frame();
        let d = t.elapsed();
        std::hint::black_box(&f);
        samples.push(d);
        h.keys("backspace");
        let _ = h.frame();
    }
    let mut v = samples.split_off(warmup);
    v.sort();
    (
        percentile(&v, 0.5),
        percentile(&v, 0.99),
        v[v.len() - 1],
        v.len(),
    )
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let quick = args.iter().any(|a| a == "--quick");
    let json = args.iter().any(|a| a == "--json");
    let (runs, warmup) = if quick { (100, 10) } else { (1_000, 100) };
    let data = corpus(1024 * 1024);
    for (name, n) in [("1k", 1_000), ("10k", MAX_CURSORS)] {
        let id = format!("BM-211-{name}");
        let (p50, p99, max, count) = measure(&data, n, runs, warmup);
        if json {
            println!(
                "{{\"id\":\"{id}\",\"n\":{count},\"cursors\":{n},\"p50_us\":{:.1},\"p99_us\":{:.1},\"max_us\":{:.1},\"quick\":{quick}}}",
                p50.as_secs_f64() * 1e6,
                p99.as_secs_f64() * 1e6,
                max.as_secs_f64() * 1e6,
            );
        } else {
            println!(
                "{id} 複数カーソルの入力 → Frame 生成 (120x40, 1 MB, カーソル {n} 個, {runs} 回, ウォームアップ {warmup} 回){}",
                if quick {
                    " [--quick: 記録には使わない]"
                } else {
                    ""
                }
            );
            println!(
                "  n={count} p50 {p50:?}  p99 {p99:?}  max {max:?}  (参考値。10k の p99 が 50 ms を超えたら上限を見直す、03 §4.7.1)"
            );
        }
    }
}
