//! `cargo xtask check`: 速い車線 (docs/11-dev-workflow.md §8.1、docs/09-testing-quality.md §6.1、docs/08-roadmap.md §2.2)。
//!
//! v0.1 の中身: (1) fmt、(2) clippy、(3) nextest (eve-text・eve-encoding・eve-core)、(4) check-deps、
//! (5) AC のチェックリストの検査、(6) 変更した crate の Harness のシナリオテスト。
//! この 3 か所 (11 §8.1・09 §6.1・08 §2.2) は同時に直す。

use std::process::Command;

use crate::{ac_checklist, check_deps, util::run};

pub fn run_all() -> Result<(), String> {
    run(
        "(1) fmt",
        Command::new("cargo").args(["fmt", "--all", "--", "--check"]),
    )?;
    run(
        "(2) clippy",
        Command::new("cargo").args([
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ]),
    )?;
    // 対象の crate にテストがまだ無くても通す (--no-tests=pass)。
    run(
        "(3) nextest",
        Command::new("cargo").args([
            "nextest",
            "run",
            "-p",
            "eve-text",
            "-p",
            "eve-encoding",
            "-p",
            "eve-core",
            "--no-tests=pass",
        ]),
    )
    .map_err(|e| format!("{e}\n  (cargo-nextest が無ければ `cargo install cargo-nextest --locked` で入れてください)"))?;
    check_deps::run(false)?;
    ac_checklist::run()?;
    println!("▶ (6) 変更した crate の Harness のシナリオテスト");
    println!("  (未実装: eve-testkit の Harness ができてから足す。M1 の最初の縦切り (2))");
    println!("\ncheck: すべて通りました");
    Ok(())
}
