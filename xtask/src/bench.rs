//! `cargo xtask bench`: ベンチの記録 (docs/09-testing-quality.md §5.2 / §6、docs/11 §1)。
//!
//! v0.1 の M2 で記録するもの: BM-103a (キー入力の core 区間、ヘッドレス。診断用)、BM-205 (配布ビルドの exe のサイズ)。
//! BM-101a (起動から最初の Frame まで) は計測用ビルドの隠しフラグと `bench-trace` (記録の仕組み) が要るので M3 以降。
//! 結果は `target/bench/bench.json` に書く (`bench.yml` が Actions の artifact にする。リポジトリにはコミットしない)。
//! CI ランナーの値は変動が大きい共有 VM のものなので、判定には使わない (09 §5.1)。`--quick` は動作確認用で、記録には使わない。

use std::process::Command;

use crate::util::{run_capture, workspace_root};

pub fn run_all(args: &[String]) -> Result<(), String> {
    let quick = args.iter().any(|a| a == "--quick");
    let root = workspace_root();
    let mut results: Vec<String> = Vec::new();

    println!("▶ BM-103a (キー入力の core 区間)");
    let mut cmd = Command::new("cargo");
    cmd.args([
        "bench",
        "-p",
        "eve-testkit",
        "--bench",
        "bm_103a",
        "--",
        "--json",
    ]);
    if quick {
        cmd.arg("--quick");
    }
    let out = run_capture(&mut cmd)?;
    let line = out
        .lines()
        .rev()
        .find(|l| l.starts_with("{\"id\":\"BM-103a\""))
        .ok_or("BM-103a の結果の行が見つかりません")?;
    println!("  {line}");
    results.push(line.to_owned());

    println!("▶ BM-205 (配布ビルドの exe のサイズ)");
    run_capture(Command::new("cargo").args(["build", "--release", "-p", "evoeditor"]))?;
    let exe = root.join("target/release").join(if cfg!(windows) {
        "evoeditor.exe"
    } else {
        "evoeditor"
    });
    let bytes = std::fs::metadata(&exe)
        .map_err(|e| format!("{}: {e}", exe.display()))?
        .len();
    let line = format!("{{\"id\":\"BM-205\",\"bytes\":{bytes},\"quick\":{quick}}}");
    println!("  {line}");
    results.push(line);

    let commit = run_capture(Command::new("git").args(["rev-parse", "--short", "HEAD"]))
        .map(|s| s.trim().to_owned())
        .unwrap_or_else(|_| "unknown".to_owned());
    let dir = root.join("target/bench");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let json = format!(
        "{{\"commit\":\"{commit}\",\"os\":\"{}\",\"results\":[{}]}}\n",
        std::env::consts::OS,
        results.join(",")
    );
    let path = dir.join("bench.json");
    std::fs::write(&path, json).map_err(|e| format!("{}: {e}", path.display()))?;
    println!("\nbench: {} に書きました", path.display());
    if quick {
        println!("(--quick は動作確認用です。記録には使いません)");
    }
    Ok(())
}
