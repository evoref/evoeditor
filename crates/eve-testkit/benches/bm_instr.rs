//! 回帰の検出に使う計測の本体 (docs/09 §5.1 / §5.4、NFR-703、v0.2)。主要な操作 5 本を、決定的なデータで 1 回ずつ行う。
//! - 命令数: `valgrind --tool=callgrind` でこのプログラムを `<ケース>` 付きで 1 回走らせ、命令数 (Ir) を比べる (Linux)。
//! - wall-clock の A/B: `--time <ケース> <回数>` で 1 回ごとの時間 (ns) を出し、同じランナーで基準と比べる (Windows)。
//!
//! 比較は `cargo xtask perf-diff` が行う (基準のコミットと今のコミットの両方をこのプログラムで測る)。
//!
//! ```text
//! cargo bench -p eve-testkit --bench bm_instr -- --list
//! cargo bench -p eve-testkit --bench bm_instr -- open_1m
//! cargo bench -p eve-testkit --bench bm_instr -- --time type_1m 5
//! ```

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::time::Instant;

use eve_testkit::Harness;

/// ケース (名前, 説明)。名前は比較の表のキーなので変えない (変えたら基準と比べられない)。
const CASES: [(&str, &str); 5] = [
    (
        "open_1m",
        "1 MB の日本語混在の文書を開き、最初の Frame を作る",
    ),
    (
        "type_1m",
        "1 MB の文書の中央の行で 100 文字を打ち、毎回 Frame を作る",
    ),
    (
        "undo_1m",
        "1 MB の文書で 200 回の編集の後、200 回 Undo する",
    ),
    (
        "open_sjis_1m",
        "1 MB の Shift_JIS の文書を判定・復号して開く",
    ),
    ("long_line_2m", "2 MB の 1 行を開き、End・挿入・Frame"),
];

fn japanese_1m() -> Vec<u8> {
    let unit = "日本語のテキストと ASCII text 123。かな・カナ・漢字が混ざる行です。\n";
    unit.repeat((1 << 20) / unit.len() + 1).into_bytes()
}

/// Shift_JIS の 1 MB (「日本語」= 93 FA 96 7B 8C EA と ASCII と改行)。
fn sjis_1m() -> Vec<u8> {
    let unit: &[u8] =
        b"\x93\xfa\x96\x7b\x8c\xea\x82\xcc\x83\x65\x83\x4c\x83\x58\x83\x67 sjis text 456\r\n";
    unit.repeat((1 << 20) / unit.len() + 1)
}

fn opened(data: &[u8]) -> Harness {
    let mut h = Harness::new()
        .with_size(120, 40)
        .with_file("/w/a.txt", data);
    h.open("/w/a.txt");
    h.run_jobs();
    h
}

fn run(case: &str) {
    match case {
        "open_1m" => {
            let mut h = opened(&japanese_1m());
            std::hint::black_box(h.frame());
        }
        "type_1m" => {
            let mut h = opened(&japanese_1m());
            // 中央の行へ (行番号で移る。準備の時間を小さくする)
            h.cmd_with(
                "cursor.goto_line",
                eve_core::Args::new().with_int("line", 6000),
            );
            for _ in 0..100 {
                h.text("x");
                std::hint::black_box(h.frame());
            }
        }
        "undo_1m" => {
            let mut h = opened(&japanese_1m());
            for i in 0..200 {
                h.text("ab");
                // 結合の境界を作る (1 回ずつ別の Undo 単位にする)
                h.advance_ms(1500);
                if i % 50 == 0 {
                    h.keys("down");
                }
            }
            for _ in 0..200 {
                h.keys("ctrl+z");
            }
            std::hint::black_box(h.frame());
        }
        "open_sjis_1m" => {
            let mut h = opened(&sjis_1m());
            std::hint::black_box(h.frame());
        }
        "long_line_2m" => {
            let line = "abcdefghij".repeat((2 << 20) / 10 + 1);
            let mut h = opened(line.as_bytes());
            h.keys("end");
            std::hint::black_box(h.frame());
            // 挿入は Revision を変えるので、列の索引の作り直し (行の 1 回の走査) を含む
            h.text("x");
            std::hint::black_box(h.frame());
        }
        other => {
            eprintln!("知らないケースです: {other} (--list で一覧)");
            std::process::exit(2);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| a != "--bench")
        .collect();
    match args.first().map(String::as_str) {
        Some("--list") | None => {
            for (name, what) in CASES {
                println!("{name}\t{what}");
            }
        }
        Some("--time") => {
            let case = args.get(1).map(String::as_str).unwrap_or("");
            let n: usize = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(5);
            for _ in 0..n {
                let t = Instant::now();
                run(case);
                println!("{}", t.elapsed().as_nanos());
            }
        }
        Some(case) => run(case),
    }
}
