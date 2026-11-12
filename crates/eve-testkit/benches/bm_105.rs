//! BM-105: 巨大ファイル上の編集 (docs/09-testing-quality.md §5.2、01 NFR-105)。
//! 100 MB のファイルの中央へ 1 文字挿入 → Undo を 1,100 回 (先頭 100 回はウォームアップとして除く) 繰り返し、
//! コマンド実行時間 (編集コマンドの開始から Transaction の適用完了まで。Frame 生成を含まない) の p50 / p99 を出す。
//! 目標: p50 1 ms / p99 5 ms。ヘッドレス (`Harness`・`MemFs`) で測る。暫定判定 (ADR-0004、S-02)。
//!
//! **v0.4 (参考)**: 保持方式の記録が多い文書 (`Shift_JIS` の 100 MB。2 行に 1 つ区分 B の文字があり、記録は約 65 万件) でも
//! 同じことを測る。編集 1 回の記録の表の更新が、記録の総数に比例しないこと (03 §8.7.1、NFR-105) を見る。
//!
//! ```text
//! cargo bench -p eve-testkit --bench bm_105
//! cargo bench -p eve-testkit --bench bm_105 -- --quick   # 動作確認 (回数と大きさを減らす。判定には使わない)
//! ```
//! 基準機 (ENV-W1) で他の重い作業を止めて実行する。結果は標準出力に出す。

#![allow(clippy::print_stdout, clippy::print_stderr)]
// 文字コードを指定して開くのに EditorState を直接使う (試験と計測だけに許す Editor::state_mut、STR-B3)
#![allow(clippy::disallowed_methods)]
#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "計測用の数値変換。値は十分に小さい"
)]

use std::time::{Duration, Instant};

use eve_core::{Args, Document, Encoding, OpenOptions};
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

/// 平均 80 B 弱の `Shift_JIS` の行を `bytes` 以上になるまで並べる。偶数番目の行に区分 B の文字 (NEC 選定 IBM 拡張の
/// 「纊」0xED40。再符号化すると IBM 拡張の 0xFA5C になる) を 1 つ入れる (保持方式の記録が 1 件できる)。決定的。
fn sjis_dense(bytes: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes + 128);
    let mut i = 0u64;
    while out.len() < bytes {
        out.extend_from_slice(format!("{i:08} ").as_bytes());
        // 「日本語」
        out.extend_from_slice(b"\x93\xFA\x96\x7B\x8C\xEA");
        out.extend_from_slice(b" the quick brown fox jumps over the lazy dog 0123456789 ");
        if i.is_multiple_of(2) {
            out.extend_from_slice(b"\xED\x40");
        }
        out.extend_from_slice(b"abc\n");
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

fn doc(h: &Harness) -> &Document {
    let id = h.editor.state().active_document().expect("文書がある");
    h.editor.state().document(id).expect("文書がある")
}

/// `data` を開いて、中央の行へ 1 文字挿入 → Undo を `runs` 回測る。`records` なら保持方式の記録がある文書 (`Shift_JIS`
/// と判定され、記録が 1 件以上あること) を確かめる。
fn measure(data: Vec<u8>, runs: usize, warmup: usize, records: bool) {
    let lines = data.iter().filter(|b| **b == b'\n').count();
    let mut h = Harness::new().with_file("/w/big.txt", &data);
    drop(data);
    let t = Instant::now();
    if records {
        // ASCII の多い文書は UTF-8 と判定されることがあるので、文字コードを指定して開く
        h.editor.state_mut().open_path(
            "/w/big.txt",
            OpenOptions {
                forced: Some(Encoding::ShiftJis),
                ..OpenOptions::default()
            },
        );
    } else {
        h.open("/w/big.txt");
    }
    h.run_jobs();
    println!("  読込 (参考): {:?}", t.elapsed());
    assert!(!h.is_dirty());
    let d = doc(&h);
    if records {
        assert_eq!(
            d.format.encoding,
            Encoding::ShiftJis,
            "Shift_JIS と判定する"
        );
        assert!(!d.preserved().is_empty(), "保持方式の記録がある");
        println!(
            "  文字コード {:?}、保持方式の記録 {} 件 (元のバイト列 {} バイト)",
            d.format.encoding,
            d.preserved().len(),
            d.preserved().total_bytes()
        );
    }

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

fn main() {
    let quick = std::env::args().any(|a| a == "--quick");
    let (size, runs, warmup) = if quick {
        (8 * 1024 * 1024, 200, 20)
    } else {
        (100 * 1024 * 1024, 1100, 100)
    };
    let mark = if quick {
        " [--quick: 判定には使わない]"
    } else {
        ""
    };
    println!(
        "BM-105 巨大ファイル上の編集 ({} MB, {runs} 回, ウォームアップ {warmup} 回){mark}",
        size / (1024 * 1024),
    );
    measure(corpus(size), runs, warmup, false);

    // v0.4 (参考): 保持方式の記録が多い文書
    println!(
        "BM-105 保持方式の記録が多い文書 (Shift_JIS {} MB、2 行に 1 件、v0.4 の参考){mark}",
        size / (1024 * 1024),
    );
    measure(sjis_dense(size), runs, warmup, true);
}
