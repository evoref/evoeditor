//! 開発用のタスクの入口 (`cargo xtask <サブコマンド>`)。サブコマンドの一覧と版の正本は docs/11-dev-workflow.md §1。
//!
//! 現在は骨格で、サブコマンドはない。M1 のタスク (0) の続きで `check` / `check-deps` / `i18n` / `gen-corpus` を足す。

#![forbid(unsafe_code)]
// 開発用ツールなので、標準エラーへの出力は許す。
#![allow(clippy::print_stderr)]

use std::process::ExitCode;

fn main() -> ExitCode {
    eprintln!("xtask: サブコマンドはまだありません (M1 のタスク (0) の続きで追加します)");
    ExitCode::from(2)
}
