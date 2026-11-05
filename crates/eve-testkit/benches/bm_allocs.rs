//! 確保の回数の計測 (docs/review/v0.3-findings.md の §3.3 の 3)。`bm_instr` と同じケースを 1 回ずつ行い、その間のヒープの
//! 確保の回数 (alloc + realloc) とバイト数を出す。決定的なので OS を問わず比べられる。`cargo xtask perf-diff --mode allocs`
//! が基準のコミットと比べる。
//!
//! 確保を数える allocator は dev 依存の `stats_alloc` のもの (自前の `unsafe` を書かない、02 R5 / NFR-704。2026-11-05 PO)。
//! 数える処理が命令数や時間に混ざらないよう、`bm_instr` とは別のバイナリにしてある。
//!
//! ```text
//! cargo bench -p eve-testkit --bench bm_allocs -- --list
//! cargo bench -p eve-testkit --bench bm_allocs -- type_1m      # "<確保の回数>\t<バイト数>"
//! ```

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::alloc::System;

use eve_testkit::Harness;
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

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
        Some(case) => {
            let region = Region::new(GLOBAL);
            run(case);
            let st = region.change();
            println!(
                "{}\t{}",
                st.allocations + st.reallocations,
                st.bytes_allocated + st.bytes_reallocated.max(0) as usize
            );
        }
    }
}
