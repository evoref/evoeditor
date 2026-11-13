//! BM-106w (spike、M25 の 6): 長大行 (1 行 10 MB、改行なし) を `view.wrap = "word"`、120×40 で開いたときの、
//! 開く・`End`・表示行の単位の `PageDown`・中央挿入 (docs/09 §5.2 の BM-106w、03 §14.4 の「長大行の折り返し」)。
//! 判定の条件 (03 §14.4 の 4 の (a)): 各操作の p99 ≤ 100 ms。BM-106 と同じ 3 種のコーパス。ヘッドレス (core の区間)。
//!
//! 操作 (BM-106 に倣い、`open` / `end*` は回ごとに開き直す):
//! - `open`: 開く → 最初の Frame。
//! - `end`: 開いた直後の 1 回目の `End` → Frame。折り返し ON の `End` は、まず **表示行の末尾** へ動く (03 §14.3.1)
//!   ので、行末までは走査しない。
//! - `end_line`: 開いて `End` を 1 回 (測らない) した後の 2 回目の `End` (論理行の末尾へ) → Frame。行末までの折り返しの
//!   走査を含む (BM-106 の `end` に当たる重い 1 回)。
//! - `doc_end` (参考): 開いた直後の `Ctrl+End` → Frame (文書の末尾の画面に長大行の最後の表示行が入る。行末まで折り返す)。
//! - `pagedown`: 開いた文書の先頭から `PageDown` → Frame を繰り返す (表示行の単位。索引を要るところまで伸ばす)。
//! - `insert_mid`: 行の中央へ移ってから、1 文字の挿入 → Frame を繰り返す (変更の位置の手前の記録点から作り直す)。
//! - `end_after_insert` (参考): 開いて中央に 1 文字挿入し、`End` を 1 回 (測らない) した後の 2 回目の `End` → Frame
//!   (変更の位置から行末までの折り返しの作り直しを含む)。
//!
//! ```text
//! cargo bench -p eve-testkit --bench bm_106w
//! cargo bench -p eve-testkit --bench bm_106w -- --quick | --json | --only ascii | --reference
//! ```

#![allow(clippy::print_stdout, clippy::print_stderr)]
#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "計測用の数値変換。値は十分に小さい"
)]

use std::time::{Duration, Instant};

use eve_core::Args;
use eve_testkit::Harness;

const LINE_BYTES: usize = 10 * 1024 * 1024;

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    sorted[((sorted.len() as f64 - 1.0) * p).round() as usize]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn fresh(data: &[u8]) -> Harness {
    let cfg = eve_config::Config {
        wrap: eve_config::WrapMode::Word,
        ..eve_config::Config::default()
    };
    Harness::builder()
        .size(120, 40)
        .config(cfg)
        .file("/w/long.txt", data)
        .build()
}

/// 開いて最初の Frame まで進めた Harness (ここは測らない)。
fn opened(data: &[u8]) -> Harness {
    let mut h = fresh(data);
    h.open("/w/long.txt");
    h.run_jobs();
    let _ = h.frame();
    h
}

/// 行の中央の桁 (書記素数。単位の繰り返しの境目) へ移る (ここは測らない)。
fn goto_mid(h: &mut Harness, unit: &str) {
    let per_unit = eve_text::grapheme_indices(unit).len() as i64;
    let mid_col = (LINE_BYTES / 2 / unit.len()) as i64 * per_unit + 1;
    h.cmd_with(
        "cursor.goto_line",
        Args::new().with_int("line", 1).with_int("col", mid_col),
    );
    let _ = h.frame();
}

fn timed(h: &mut Harness, f: impl FnOnce(&mut Harness)) -> Duration {
    let t = Instant::now();
    f(h);
    let fr = h.frame();
    let d = t.elapsed();
    std::hint::black_box(&fr);
    d
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let quick = args.iter().any(|a| a == "--quick");
    let json = args.iter().any(|a| a == "--json");
    let runs = if quick { 20 } else { 200 };
    let reopen_runs = runs.min(100);
    let only = args
        .iter()
        .position(|a| a == "--only")
        .and_then(|i| args.get(i + 1));
    let reference = args.iter().any(|a| a == "--reference");
    let cases: [(&str, &str, bool); 5] = [
        ("ascii", "abcdefghij", false),
        ("japanese", "日本語のテキスト", false),
        ("emoji", "a日😀b", false),
        // 参考: 単語の境界 (空白) で折る英文 (表示行ごとに後ろへ戻って折る位置を選び、残りを次の行へ回す)
        ("ascii_words", "the quick brown fox jumps ", true),
        ("korean", "한국어 텍스트입니다. ", true),
    ];
    for (id, unit, ref_case) in cases {
        if only.is_some_and(|o| o != id) || (ref_case && !reference && only.is_none()) {
            continue;
        }
        let line = unit.repeat(LINE_BYTES / unit.len() + 1);
        let mut data = line.into_bytes();
        data.extend_from_slice(b"\nsecond\n");
        let mut results: Vec<(&str, Vec<Duration>)> = Vec::new();

        let mut open = Vec::new();
        for _ in 0..reopen_runs {
            let mut h = fresh(&data);
            open.push(timed(&mut h, |h| {
                h.open("/w/long.txt");
                h.run_jobs();
            }));
        }
        results.push(("open", open));

        let mut end = Vec::new();
        let mut end_line = Vec::new();
        for _ in 0..reopen_runs {
            let mut h = opened(&data);
            end.push(timed(&mut h, |h| h.keys("end")));
            end_line.push(timed(&mut h, |h| h.keys("end")));
        }
        results.push(("end", end));
        results.push(("end_line", end_line));

        let mut doc_end = Vec::new();
        for _ in 0..reopen_runs.min(30) {
            let mut h = opened(&data);
            doc_end.push(timed(&mut h, |h| h.keys("ctrl+end")));
        }
        results.push(("doc_end", doc_end));

        let mut pagedown = Vec::new();
        {
            let mut h = opened(&data);
            for _ in 0..runs {
                pagedown.push(timed(&mut h, |h| h.keys("pagedown")));
            }
        }
        results.push(("pagedown", pagedown));

        let mut end_after_insert = Vec::new();
        for _ in 0..reopen_runs {
            let mut h = opened(&data);
            goto_mid(&mut h, unit);
            h.text("x");
            let _ = h.frame();
            h.keys("end");
            let _ = h.frame();
            end_after_insert.push(timed(&mut h, |h| h.keys("end")));
        }

        let mut insert = Vec::new();
        {
            let mut h = opened(&data);
            drop(data);
            goto_mid(&mut h, unit);
            for _ in 0..runs {
                insert.push(timed(&mut h, |h| h.text("x")));
            }
        }
        results.push(("insert_mid", insert));
        results.push(("end_after_insert", end_after_insert));

        for (op, mut v) in results {
            v.sort();
            let (p50, p99, max) = (percentile(&v, 0.5), percentile(&v, 0.99), v[v.len() - 1]);
            let refop = ref_case || matches!(op, "doc_end" | "end_after_insert");
            if json {
                println!(
                    "{{\"id\":\"BM-106w\",\"case\":\"{id}\",\"op\":\"{op}\",\"n\":{},\"p50_ms\":{:.2},\"p99_ms\":{:.2},\"max_ms\":{:.2},\"reference\":{refop},\"quick\":{quick}}}",
                    v.len(),
                    ms(p50),
                    ms(p99),
                    ms(max)
                );
            } else {
                println!(
                    "BM-106w {id:11} {op:16} n={:4} p50 {:7.2} ms  p99 {:7.2} ms  max {:7.2} ms  ({})",
                    v.len(),
                    ms(p50),
                    ms(p99),
                    ms(max),
                    if refop {
                        "参考"
                    } else if ms(p99) <= 100.0 {
                        "p99 ≤ 100 ms → OK"
                    } else {
                        "p99 ≤ 100 ms → 超過"
                    }
                );
            }
        }
    }
    if quick && !json {
        println!("(--quick は動作確認用です。記録には使いません)");
    }
}
