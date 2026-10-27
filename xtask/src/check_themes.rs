//! `cargo xtask check-themes`: 組込みテーマのロール表の検査 (docs/09-testing-quality.md §3.5、docs/07 §8.2、FR-513 AC-1 / AC-2、NFR-604)。
//!
//! (a)〜(d) docs/07 §8.2 の表そのもの (truecolor と mono の欠落・形・コントラスト・意味のあるロールの mono) は、
//! `eve-docs-tables` の `theme` で検査する。(e)(f) 実装 (eve-tui) との突合と近似した 256 / 16 の段は、
//! eve-tui のテスト `theme_table` で行う (実装の型を使うため)。

use std::process::Command;

use eve_docs_tables::theme::{check_table, parse_roles};

use crate::util::{read_text, run, workspace_root};

pub fn run_all() -> Result<(), String> {
    let root = workspace_root();
    let md = read_text(&root.join("docs/07-config-extensibility.md"))?;
    let rows = parse_roles(&md)?;
    let errs = check_table(&rows);
    if !errs.is_empty() {
        return Err(errs.join("\n"));
    }
    println!(
        "  OK  07 §8.2 のロール表 {} 行: truecolor と mono の値・形・コントラスト",
        rows.len()
    );
    run(
        "実装 (eve-tui) と表の突合・256/16 の近似",
        Command::new("cargo").args(["test", "-p", "eve-tui", "--test", "theme_table"]),
    )?;
    println!("\ncheck-themes: すべて通りました");
    Ok(())
}
