//! `tests/scenarios/**/*.toml` (シナリオ DSL、docs/11 §6.3) を全部読み、型と規則を検証してから実行する。
//! `cargo xtask scenario run [<部分一致>] [--print-frame]` / `cargo xtask scenario check` はこの試験を環境変数で絞って呼ぶ:
//! `EVE_SCENARIO` (相対パスの部分一致)、`EVE_SCENARIO_PRINT_FRAME=1` (最後の Frame を出す)、`EVE_SCENARIO_CHECK=1` (実行しない)。

#![allow(clippy::print_stdout)]

use std::path::PathBuf;

use eve_testkit::scenario;

fn scenarios_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/scenarios")
}

// verifies: NFR-707
#[test]
fn シナリオはすべて検証を通り_期待どおりに動く() {
    let filter = std::env::var("EVE_SCENARIO").unwrap_or_default();
    let print = std::env::var("EVE_SCENARIO_PRINT_FRAME").is_ok_and(|v| v == "1");
    let check_only = std::env::var("EVE_SCENARIO_CHECK").is_ok_and(|v| v == "1");
    let all = scenario::collect(&scenarios_dir());
    assert!(!all.is_empty(), "tests/scenarios にシナリオがありません");
    let mut errs = Vec::new();
    let mut ran = 0;
    for (rel, path) in all.iter().filter(|(r, _)| r.contains(&filter)) {
        let text = std::fs::read_to_string(path).expect("シナリオを読める");
        let sc = match scenario::load(&text, rel) {
            Ok(sc) => sc,
            Err(e) => {
                errs.extend(e);
                continue;
            }
        };
        ran += 1;
        if check_only {
            continue;
        }
        match scenario::run(&sc) {
            Ok(out) => {
                if print {
                    println!("── {rel}\n{}", out.frame);
                }
            }
            Err(e) => errs.extend(e.into_iter().map(|m| format!("{rel}: {m}"))),
        }
    }
    println!(
        "シナリオ {ran} 本{}",
        if check_only { " (検証のみ)" } else { "" }
    );
    assert!(errs.is_empty(), "\n{}", errs.join("\n"));
}

#[test]
fn 規則に反するシナリオは読み込みで失敗する() {
    let bad = r#"
[meta]
id = "x/other"
title = "t"
since = "v0.2"
[[step]]
keys = "a"
text = "b"
[[step]]
cmd = "no.such_command"
[[assert]]
frame = "golden"
"#;
    let errs = scenario::load(bad, "x/bad.toml").unwrap_err();
    let all = errs.join("\n");
    assert!(all.contains("パス"), "{all}");
    assert!(all.contains("1 種類"), "{all}");
    assert!(all.contains("no.such_command"), "{all}");
    assert!(all.contains("未対応"), "{all}");
    // 未知のキーは型で落ちる
    assert!(
        scenario::load(
            "[meta]\nid='a'\ntitle='t'\nsince='v0.2'\nunknown=1\n",
            "a.toml"
        )
        .is_err()
    );
}
