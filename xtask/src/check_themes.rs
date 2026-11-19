//! `cargo xtask check-themes`: 組込みテーマのロール表の検査 (docs/09-testing-quality.md §3.5、docs/07 §8.2・§8.7、FR-513 AC-1 / AC-2、NFR-604)。
//!
//! (a)〜(d) docs/07 §8.2 の表そのもの (truecolor と mono の欠落・形・コントラスト・意味のあるロールの mono) は、
//! `eve-docs-tables` の `theme` で検査する。(g) 高コントラストのテーマ (07 §8.7.1 / §8.7.2 の表) は、(a)(b)(d) と
//! 07 §8.7 の基準 (h1)〜(h10) を同じ crate で検査し、ロール名の集合が 07 §8.2 の表と同じことを確かめる。
//! (e)(f) 実装 (eve-config・eve-tui) との突合と近似した 256 / 16 の段は、core と eve-tui のテスト `theme_table` で行う
//! (実装の型を使うため)。

use std::process::Command;

use eve_docs_tables::theme::{
    HIGH_CONTRAST_TABLES, check_high_contrast, check_role_set, check_table, parse_high_contrast,
    parse_roles,
};

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
    let mut errs = Vec::new();
    for (theme, heading) in HIGH_CONTRAST_TABLES {
        let hc = parse_high_contrast(&md, heading)?;
        let mut theme_errs = check_high_contrast(theme, &hc);
        theme_errs.extend(check_role_set(theme, &hc, &rows));
        if theme_errs.is_empty() {
            println!(
                "  OK  07 §8.7 の {theme} のロール表 {} 行: 4 段の値・形・(h1)〜(h10)・§8.2 とのロール名の一致",
                hc.len()
            );
        } else {
            println!(
                "  NG  07 §8.7 の {theme} のロール表: {} 件",
                theme_errs.len()
            );
        }
        errs.extend(theme_errs);
    }
    if !errs.is_empty() {
        return Err(errs.join("\n"));
    }
    run(
        "組込みテーマ (eve-config) と core の解決の、表の全ロール・4 段との突合",
        Command::new("cargo").args(["test", "-p", "eve-core", "--test", "theme_table"]),
    )?;
    run(
        "実装 (eve-tui) と表の突合・256/16 の近似",
        Command::new("cargo").args(["test", "-p", "eve-tui", "--test", "theme_table"]),
    )?;
    println!("\ncheck-themes: すべて通りました");
    Ok(())
}
