//! `cargo xtask bench`: ベンチの記録 (docs/09-testing-quality.md §5.2 / §6、docs/11 §1)。
//!
//! 記録するもの: BM-103a (キー入力の core 区間、ヘッドレス。診断用)、BM-104 / BM-202 (サイズ別オープンの時間とメモリ、実ファイル。診断用。
//! 判定は基準機)、BM-205 (配布ビルドの exe のサイズ)。
//! BM-101a (起動から最初の Frame まで) は計測用ビルドの隠しフラグと `bench-trace` (記録の仕組み) が要るので M3 以降。
//! `--env ENV-W1 --out DIR` を付けると基準機の記録になる: BM-106 と、実バイナリを ConPTY で駆動する BM-101b / 102b / 103b / 107
//! (`evoeditor` の `bm_pty`) も測り、DIR にベンチ ID ごとの JSON と `summary.md` を書く (`bench_report`、09 §5.3)。
//! 結果は `target/bench/bench.json` に書く (`bench.yml` が Actions の artifact にする。リポジトリにはコミットしない)。
//! CI ランナーの値は変動が大きい共有 VM のものなので、判定には使わない (09 §5.1)。`--quick` は動作確認用で、記録には使わない。

use std::process::Command;

use crate::util::{run_capture, workspace_root};

pub fn run_all(args: &[String]) -> Result<(), String> {
    let quick = args.iter().any(|a| a == "--quick");
    let value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
    };
    let (env, out_dir) = (value("--env"), value("--out"));
    if env.is_some() != out_dir.is_some() {
        return Err(
            "--env と --out は一緒に指定します (例: --env ENV-W1 --out docs/perf/0.1.0)".to_owned(),
        );
    }
    if let Some(e) = env
        && !(e.starts_with("ENV-")
            && e.len() > 4
            && e[4..].chars().all(|c| c.is_ascii_alphanumeric()))
    {
        return Err(format!("環境 ID は ENV-W1 のような形です: {e}"));
    }
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

    // BM-104 / BM-202 (サイズ別オープンの時間とメモリ。実ファイル、Windows)。S-02 の最終判定の材料 (09 §5.2)
    if cfg!(windows) {
        println!("▶ BM-104 / BM-202 (サイズ別オープンの時間とメモリ)");
        let mut cmd = Command::new("cargo");
        cmd.args([
            "bench",
            "-p",
            "eve-platform",
            "--bench",
            "bm_104_202",
            "--",
            "--json",
        ]);
        if quick {
            cmd.arg("--quick");
        }
        let out = run_capture(&mut cmd)?;
        let lines: Vec<&str> = out
            .lines()
            .filter(|l| l.starts_with("{\"id\":\"BM-104"))
            .collect();
        if lines.is_empty() {
            return Err("BM-104 の結果の行が見つかりません".to_owned());
        }
        for l in lines {
            println!("  {l}");
            results.push(l.to_owned());
        }
    }

    if out_dir.is_some() {
        // BM-106 (長大行。3 種 × 4 操作)
        println!("▶ BM-106 (長大行)");
        let mut cmd = Command::new("cargo");
        cmd.args([
            "bench",
            "-p",
            "eve-testkit",
            "--bench",
            "bm_106",
            "--",
            "--json",
        ]);
        if quick {
            cmd.arg("--quick");
        }
        for l in run_capture(&mut cmd)?
            .lines()
            .filter(|l| l.starts_with("{\"id\":\"BM-106\""))
        {
            results.push(l.to_owned());
        }
        // 実バイナリを ConPTY で駆動する計測 (Windows)。--quick は動作確認の回数
        if cfg!(windows) {
            println!("▶ BM-101b / 102b / 103b / 107 (ConPTY、実バイナリ)");
            let runs = if quick { "30" } else { "1000" };
            let out = run_capture(Command::new("cargo").args([
                "bench",
                "-p",
                "evoeditor",
                "--features",
                "bench-trace",
                "--bench",
                "bm_pty",
                "--",
                "--json",
                "--runs",
                runs,
            ]))?;
            let lines: Vec<&str> = out
                .lines()
                .filter(|l| l.starts_with("{\"id\":\"BM-"))
                .collect();
            if lines.is_empty() {
                return Err("bm_pty の結果の行が見つかりません".to_owned());
            }
            for l in lines {
                println!("  {l}");
                results.push(l.to_owned());
            }
        }
    }

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
    if let (Some(env), Some(dir)) = (env, out_dir) {
        let dir = root.join(dir);
        crate::bench_report::write(&dir, env, &commit, quick, &results)?;
        println!(
            "bench: {} に JSON と summary.md を書きました",
            dir.display()
        );
    }
    if quick {
        println!("(--quick は動作確認用です。記録には使いません)");
    }
    Ok(())
}
