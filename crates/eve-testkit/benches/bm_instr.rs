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

include!("shared/instr_cases.rs");

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
