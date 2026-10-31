//! `cargo xtask area-check --base <ref> [--head <ref>] [--labels a,b]`: PR の変更ファイルとラベルの整合 (docs/09 §6.2 の `area-check`、
//! §7 R9、11 §8.3、v0.2)。
//!
//! - `area:<frontend>` (例 `area:tui`) のラベルの PR は、`crates/eve-core` / `crates/eve-view` / `crates/eve-config` を変えない
//!   (frontend の担当は境界の型と core を自分の PR で変えない。必要なら契約変更要求を先に出す、11 §8.4)。
//! - `crates/eve-view` を変える PR には `contract-change` のラベルが要る (11 §8.4 の手続き)。

use std::process::Command;

use crate::util::run_capture;

/// frontend の PR が変えてはいけない場所。
const CORE_PATHS: [&str; 3] = ["crates/eve-core/", "crates/eve-view/", "crates/eve-config/"];

/// 変更ファイルとラベルから、規則に反するものを返す。
pub fn violations(files: &[String], labels: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let frontend: Vec<&String> = labels.iter().filter(|l| l.starts_with("area:")).collect();
    if !frontend.is_empty() {
        for f in files {
            if CORE_PATHS.iter().any(|p| f.starts_with(p)) {
                out.push(format!(
                    "{} の PR は {f} を変更できません (09 §7 R9。必要なら契約変更要求を先に出す、11 §8.4)",
                    frontend.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
                ));
            }
        }
    }
    let touches_view = files.iter().any(|f| f.starts_with("crates/eve-view/"));
    if touches_view && !labels.iter().any(|l| l == "contract-change") {
        out.push("crates/eve-view を変更する PR には `contract-change` のラベルが要ります (11 §8.3 / §8.4)".to_owned());
    }
    out
}

pub fn run(args: &[String]) -> Result<(), String> {
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let base = flag("--base").ok_or("--base <ref> を指定してください")?;
    let head = flag("--head").unwrap_or_else(|| "HEAD".to_owned());
    let labels: Vec<String> = flag("--labels")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    println!("▶ area-check ({base}...{head}、ラベル {labels:?})");
    let diff = run_capture(Command::new("git").args([
        "diff",
        "--name-only",
        &format!("{base}...{head}"),
    ]))?;
    let files: Vec<String> = diff
        .lines()
        .map(|l| l.trim().replace('\\', "/"))
        .filter(|l| !l.is_empty())
        .collect();
    let v = violations(&files, &labels);
    if v.is_empty() {
        println!("  変更 {} ファイル。OK", files.len());
        Ok(())
    } else {
        Err(format!(
            "area-check に {} 件の問題があります\n  - {}",
            v.len(),
            v.join("\n  - ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }

    // verifies: NFR-708
    #[test]
    fn frontend_の_pr_は_core_を変えず_eve_view_の変更にはラベルが要る() {
        // ラベルなし・core の変更だけ: OK
        assert!(violations(&s(&["crates/eve-core/src/lib.rs"]), &[]).is_empty());
        // area:tui で eve-tui だけ: OK
        assert!(violations(&s(&["crates/eve-tui/src/win.rs"]), &s(&["area:tui"])).is_empty());
        // area:tui で core を変える: 失敗
        assert_eq!(
            violations(
                &s(&["crates/eve-tui/src/win.rs", "crates/eve-config/src/a.rs"]),
                &s(&["area:tui"])
            )
            .len(),
            1
        );
        // eve-view の変更: contract-change が要る
        assert_eq!(
            violations(&s(&["crates/eve-view/src/frame.rs"]), &[]).len(),
            1
        );
        assert!(
            violations(
                &s(&["crates/eve-view/src/frame.rs"]),
                &s(&["contract-change"])
            )
            .is_empty()
        );
        // area:tui で eve-view を変える: ラベルがあっても R9 で失敗
        assert_eq!(
            violations(
                &s(&["crates/eve-view/src/frame.rs"]),
                &s(&["area:tui", "contract-change"])
            )
            .len(),
            1
        );
    }
}
