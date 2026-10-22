//! 開発用のタスクの入口 (`cargo xtask <サブコマンド>`)。サブコマンドの一覧と版の正本は docs/11-dev-workflow.md §1。
//!
//! 現在のサブコマンド:
//! - `check`: 速い車線 (fmt・clippy・nextest・check-deps・AC のチェックリスト)。docs/11 §8.1
//! - `check-deps`: 依存規則 R1〜R12 の検査。`--docs` で docs/02 §4.1 の表との突合。docs/09 §7
//! - `check-dist`: 配布ビルドにテスト専用の経路が無いことと CRT の静的リンクの確認 (docs/09 §6.2 `build-release-smoke`)
//! - `gen-notices`: THIRD-PARTY-NOTICES.md の生成 (`--check` で最新性の検査。docs/05 §17.1)
//! - `check-ac-checklist`: `tests/ac-checklist.toml` の検査 (docs/11 §3.3)
//! - `bench`: ベンチの記録 (docs/09 §5.2)
//! - `check-themes`: 組込みテーマのロール表の検査 (docs/09 §3.5、docs/07 §8.2)
//! - `i18n`: UI 文言の検査 (docs/09 §6.2、docs/07 §17)
//! - `gen-corpus`: 巨大・長大のコーパスの決定的な生成 (docs/09 §4)
//! - `nightly`: 夜間ジョブの入口 (fuzz・proptest 拡張・audit・coverage。Linux。docs/09 §6.3)

#![forbid(unsafe_code)]
// 開発用ツールなので、標準出力・標準エラーへの出力は許す。
#![allow(clippy::print_stdout, clippy::print_stderr)]

mod ac_checklist;
mod bench;
mod check;
mod check_deps;
mod check_dist;
mod check_themes;
mod gen_corpus;
mod gen_notices;
mod i18n;
mod nightly;
mod util;

use std::process::ExitCode;

const USAGE: &str = "\
使い方: cargo xtask <サブコマンド>

  check               速い車線 (fmt・clippy・nextest・check-deps・AC のチェックリスト)
  check-deps          依存規則 R1〜R12 の検査 (--docs: docs/02 §4.1 の表との突合も行う)
  check-dist          配布ビルドの確認 (feature・目印の文字列・隠しオプション・CRT の静的リンク。--exe PATH で既存の exe)
  gen-notices         THIRD-PARTY-NOTICES.md の生成 (cargo-about。--check: 最新性の検査)
  check-ac-checklist  tests/ac-checklist.toml の検査 (check の一部)
  bench               ベンチの記録 (BM-103a・BM-205 → target/bench/bench.json。--quick は動作確認用)
  check-themes        組込みテーマのロール表の検査 (07 §8.2。truecolor と mono・コントラスト・実装との突合)
  i18n                UI 文言の検査 (04 §15.2 の目録・07 §11.6 のカタログ・locales/ja.toml)
  gen-corpus          巨大・長大のコーパスの生成 (--seed N --out DIR --only a,b --with-1gb --list)
  nightly             夜間ジョブ (Linux。--only fuzz,proptest,audit,coverage --fuzz-seconds N --seed N)";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let result = match args.next().as_deref() {
        Some("check") => check::run_all(),
        Some("check-deps") => check_deps::run(args.any(|a| a == "--docs")),
        Some("check-dist") => check_dist::run(&args.collect::<Vec<_>>()),
        Some("gen-notices") => gen_notices::run(&args.collect::<Vec<_>>()),
        Some("check-ac-checklist") => ac_checklist::run(),
        Some("bench") => bench::run_all(&args.collect::<Vec<_>>()),
        Some("check-themes") => check_themes::run_all(),
        Some("i18n") => i18n::run(),
        Some("gen-corpus") => gen_corpus::run(&args.collect::<Vec<_>>()),
        Some("nightly") => nightly::run_all(&args.collect::<Vec<_>>()),
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
