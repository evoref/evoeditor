//! BM-216: 保存時の変換のメインスレッドの時間 (docs/09-testing-quality.md §5.2、01 FR-216 AC-3、03 §9.3.1 の SVT-1〜SVT-5)。
//! 参考値で、合否は付けない。1 MiB (`TRANSFORM_SYNC_BYTES`、上限) 以下の文書 (BM-103a と同じ日本語混在のコーパスの全行の
//! 末尾に空白を 2 つ足したもの) で、`editor.trim_trailing_whitespace_on_save` と `editor.insert_final_newline_on_save` を
//! 真にして `file.save` を実行した時間 (変換の判定・`ChangeSet` の作成・Transaction の適用・保存ジョブの投入。ジョブの
//! 実行は含めない) を 200 回 (先頭 20 回はウォームアップとして除く) 測って p50 / p99 を出す。各回の後に (測らずに)
//! ジョブを走らせ、1 回の Undo で変換を戻す (次の回も同じ本文で測る)。比べるために、設定が偽の保存 (`-off`) も測る。
//!
//! ```text
//! cargo bench -p eve-testkit --bench bm_216
//! cargo bench -p eve-testkit --bench bm_216 -- --quick   # 動作確認 (回数を減らす。記録には使わない)
//! cargo bench -p eve-testkit --bench bm_216 -- --json    # 1 行の JSON を 2 行 (xtask bench が集める)
//! ```

#![allow(clippy::print_stdout, clippy::print_stderr)]
#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "計測用の数値変換。値は十分に小さい"
)]

use std::time::{Duration, Instant};

use eve_core::{InitialFile, TRANSFORM_SYNC_BYTES};
use eve_testkit::Harness;

const PATH: &str = "/w/doc.txt";

/// 平均 80 B の行 (ASCII と日本語が混ざる。BM-103a と同じ) の末尾に空白を 2 つ足し、`bytes` を超えない所まで並べる。
/// 最後の行は改行で終える。決定的。
fn corpus(bytes: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes);
    let mut i = 0u64;
    loop {
        let line = if i.is_multiple_of(3) {
            format!("{i:08} 日本語の行です。テキストエディタの試験データ。0123456789  \n")
        } else {
            format!("{i:08} the quick brown fox jumps over the lazy dog 0123456789 abcdef  \n")
        };
        if out.len() + line.len() > bytes {
            return out;
        }
        out.extend_from_slice(line.as_bytes());
        i += 1;
    }
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx]
}

/// 設定 `on` で測る。結果は (p50, p99, max, 測った回数)。
fn measure(
    data: &[u8],
    on: bool,
    runs: usize,
    warmup: usize,
) -> (Duration, Duration, Duration, usize) {
    let config = eve_config::Config {
        trim_trailing_whitespace_on_save: on,
        insert_final_newline_on_save: on,
        ..eve_config::Config::default()
    };
    let mut h = Harness::builder()
        .defer_start()
        .size(120, 40)
        .config(config)
        .file(PATH, data)
        .build();
    h.editor
        .open_initial_files(&[InitialFile::path(PATH)], false, None);
    h.started();
    h.run_jobs();
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        // 保存の前に 1 文字打って dirty にする (測らない)。変換は打った文字の後に 1 つの Undo のノードになる
        h.text("x");
        h.advance_ms(2_000);
        let t = Instant::now();
        h.cmd("file.save");
        let d = t.elapsed();
        samples.push(d);
        h.run_jobs();
        if on {
            h.cmd("edit.undo");
        }
        h.cmd("edit.undo");
        h.advance_ms(2_000);
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
    let (runs, warmup) = if quick { (20, 2) } else { (200, 20) };
    let data = corpus(TRANSFORM_SYNC_BYTES - 64);
    for (name, on) in [("1m", true), ("1m-off", false)] {
        let id = format!("BM-216-{name}");
        let (p50, p99, max, count) = measure(&data, on, runs, warmup);
        if json {
            println!(
                "{{\"id\":\"{id}\",\"n\":{count},\"bytes\":{},\"p50_us\":{:.1},\"p99_us\":{:.1},\"max_us\":{:.1},\"quick\":{quick}}}",
                data.len(),
                p50.as_secs_f64() * 1e6,
                p99.as_secs_f64() * 1e6,
                max.as_secs_f64() * 1e6,
            );
        } else {
            println!(
                "{id} 保存時の変換 ({} バイト、変換 {}, {runs} 回, ウォームアップ {warmup} 回){}",
                data.len(),
                if on { "あり" } else { "なし" },
                if quick {
                    " [--quick: 記録には使わない]"
                } else {
                    ""
                }
            );
            println!(
                "  n={count} p50 {p50:?}  p99 {p99:?}  max {max:?}  (参考値。打鍵の予算 NFR-103 と比べて上限を見直す材料)"
            );
        }
    }
}
