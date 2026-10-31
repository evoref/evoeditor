//! `cargo xtask check`: 速い車線 (docs/11-dev-workflow.md §8.1、docs/09-testing-quality.md §6.1、docs/08-roadmap.md §2.2)。
//!
//! v0.1 の中身: (1) fmt、(2) clippy、(3) nextest (eve-text・eve-encoding・eve-core)、(4) check-deps、
//! (5) AC のチェックリストの検査、(5b) 仕様書の索引の検査 (`spec-index --check`、v0.2 の M10)、
//! (6) 変更した crate の Harness のシナリオテスト。
//! この 3 か所 (11 §8.1・09 §6.1・08 §2.2) は同時に直す。

use std::collections::BTreeSet;
use std::process::Command;

use crate::{
    ac_checklist, check_deps, spec_index,
    util::{run, run_capture},
};

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
    spec_index::run(&["--check".to_owned()])?;
    scenario_tests()?;
    println!("\ncheck: すべて通りました");
    Ok(())
}

/// (3) で既に回した crate。
const COVERED: [&str; 3] = ["eve-text", "eve-encoding", "eve-core"];

/// 作業ブランチの変更 (`develop` との分岐点から作業ツリーまで、未追跡を含む) に含まれるファイルの crate 名。
fn changed_packages() -> Result<BTreeSet<String>, String> {
    let base = ["origin/develop", "develop", "main"]
        .into_iter()
        .find_map(|b| {
            run_capture(Command::new("git").args(["merge-base", "HEAD", b]))
                .ok()
                .map(|s| s.trim().to_owned())
        });
    let mut files: Vec<String> = Vec::new();
    if let Some(base) = base {
        files.extend(
            run_capture(Command::new("git").args(["diff", "--name-only", &base]))?
                .lines()
                .map(str::to_owned),
        );
    }
    files.extend(
        run_capture(Command::new("git").args(["ls-files", "--others", "--exclude-standard"]))?
            .lines()
            .map(str::to_owned),
    );
    Ok(files
        .iter()
        .filter_map(|f| {
            let mut it = f.split('/');
            match (it.next(), it.next()) {
                // `bins/evoeditor` の package 名は `evoeditor`
                (Some("crates"), Some(name)) | (Some("bins"), Some(name)) => Some(name.to_owned()),
                _ => None,
            }
        })
        .collect())
}

/// (6) 変更した crate のテスト (Harness のシナリオを含む)。(3) で回した crate は除く。
fn scenario_tests() -> Result<(), String> {
    let changed: Vec<String> = changed_packages()?
        .into_iter()
        .filter(|p| !COVERED.contains(&p.as_str()))
        .collect();
    if changed.is_empty() {
        println!(
            "▶ (6) 変更した crate のテスト: 対象なし ((3) で回した crate だけ、または変更なし)"
        );
        return Ok(());
    }
    let mut cmd = Command::new("cargo");
    cmd.args(["nextest", "run", "--no-tests=pass"]);
    for p in &changed {
        cmd.args(["-p", p]);
    }
    run(
        &format!("(6) 変更した crate のテスト ({})", changed.join(", ")),
        &mut cmd,
    )
}
