//! 開発用のタスクの入口 (`cargo xtask <サブコマンド>`)。サブコマンドの一覧と版の正本は docs/11-dev-workflow.md §1。
//!
//! 現在のサブコマンド:
//! - `check`: 速い車線 (fmt・clippy・nextest・check-deps・AC のチェックリスト)。docs/11 §8.1
//! - `check-deps`: 依存規則 R1〜R12 の検査。`--docs` で docs/02 §4.1 の表との突合。docs/09 §7
//!
//! M1 のタスク (0) の続きで `i18n` と `gen-corpus` を足す。

#![forbid(unsafe_code)]
// 開発用ツールなので、標準出力・標準エラーへの出力は許す。
#![allow(clippy::print_stdout, clippy::print_stderr)]

mod ac_checklist;
mod check;
mod check_deps;
mod util;

use std::process::ExitCode;

const USAGE: &str = "\
使い方: cargo xtask <サブコマンド>

  check               速い車線 (fmt・clippy・nextest・check-deps・AC のチェックリスト)
  check-deps          依存規則 R1〜R12 の検査 (--docs: docs/02 §4.1 の表との突合も行う)
  check-ac-checklist  tests/ac-checklist.toml の検査 (check の一部)";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let result = match args.next().as_deref() {
        Some("check") => check::run_all(),
        Some("check-deps") => check_deps::run(args.any(|a| a == "--docs")),
        Some("check-ac-checklist") => ac_checklist::run(),
        Some("--help" | "-h" | "help") | None => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some(other) => {
            eprintln!("xtask: 知らないサブコマンドです: {other}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("\nxtask: 失敗しました\n{msg}");
            ExitCode::FAILURE
        }
    }
}
