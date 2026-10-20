//! BM-105: 巨大ファイル上の編集 (docs/09-testing-quality.md §5.2、01 NFR-105)。
//! 100 MB のファイルの中央へ 1 文字挿入 → Undo を 1,100 回 (先頭 100 回はウォームアップとして除く) 繰り返し、
//! コマンド実行時間 (編集コマンドの開始から Transaction の適用完了まで。Frame 生成を含まない) の p50 / p99 を出す。
//! 目標: p50 1 ms / p99 5 ms。ヘッドレス (`Harness`・`MemFs`) で測る。M1 の core 完成後の暫定判定 (ADR-0004、S-02)。
//!
//! ```text
//! cargo bench -p eve-testkit --bench bm_105
//! cargo bench -p eve-testkit --bench bm_105 -- --quick   # 動作確認 (回数と大きさを減らす。判定には使わない)
//! ```
//! 基準機 (ENV-W1) で他の重い作業を止めて実行する。結果は標準出力に出す。

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

fn report(name: &str, mut v: Vec<Duration>, p50_goal: Duration, p99_goal: Duration) {
    v.sort();
    let (p50, p99, max) = (percentile(&v, 0.5), percentile(&v, 0.99), v[v.len() - 1]);
    let verdict = if p50 <= p50_goal && p99 <= p99_goal {
        "合格"
    } else {
        "目標超過"
    };
    println!(
        "  {name:<28} n={:<5} p50 {p50:>10.3?}  p99 {p99:>10.3?}  max {max:>10.3?}  (目標 p50 {p50_goal:?} / p99 {p99_goal:?}) {verdict}",
        v.len()
    );
}

fn main() {
    let quick = std::env::args().any(|a| a == "--quick");
    let (size, runs, warmup) = if quick {
        (8 * 1024 * 1024, 200, 20)
    } else {
        (100 * 1024 * 1024, 1100, 100)
    };
    println!(
        "BM-105 巨大ファイル上の編集 ({} MB, {runs} 回, ウォームアップ {warmup} 回){}",
        size / (1024 * 1024),
        if quick {
            " [--quick: 判定には使わない]"
        } else {
            ""
        }
    );

    let data = corpus(size);
    let lines = data.iter().filter(|b| **b == b'\n').count();
    let mut h = Harness::new().with_file("/w/big.txt", &data);
    drop(data);
    let t = Instant::now();
    h.open("/w/big.txt");
    h.run_jobs();
    println!("  読込 (参考): {:?}", t.elapsed());
    assert!(!h.is_dirty());

    // 中央の行へ移動 (表示はビューポート中央)
    h.cmd_with(
        "cursor.goto_line",
        Args::new().with_int("line", (lines / 2) as i64 + 1),
    );

    let mut insert = Vec::with_capacity(runs);
    let mut undo = Vec::with_capacity(runs);
    for _ in 0..runs {
        let t0 = Instant::now();
        h.cmd_with("edit.insert_text", Args::new().with_str("text", "x"));
        insert.push(t0.elapsed());
        // 結合の時間窓を越えて、毎回別の Undo 単位にする
        h.advance_ms(1500);
        let t1 = Instant::now();
        h.cmd("edit.undo");
        undo.push(t1.elapsed());
    }
    assert!(!h.is_dirty(), "挿入と Undo で元に戻る");

    let goal = (Duration::from_millis(1), Duration::from_millis(5));
    report(
        "中央へ 1 文字挿入",
        insert.split_off(warmup),
        goal.0,
        goal.1,
    );
    report("Undo", undo.split_off(warmup), goal.0, goal.1);
}
