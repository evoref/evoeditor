//! `cargo xtask perf-diff --base <ref> --mode instr|time [--out FILE]`: 基準のコミットと今の作業ツリーを同じランナーで測り、
//! 劣化を警告する (docs/09 §5.4、NFR-703、v0.2)。ブロックしない (いつも成功で終わり、劣化は表と GitHub Actions の警告で示す)。
//!
//! - `instr` (Linux): eve-testkit の `bm_instr` を `valgrind --tool=callgrind` で走らせ、命令数 (Ir) を比べる。+5% 超で警告。
//! - `time` (Windows など): `bm_instr --time` の 1 回ごとの時間を、基準と今を交互に測り (同じランナーでの A/B)、中央値で比べる。
//!   +10% 超で警告。
//!
//! 基準は `git worktree add --detach` で `target/perf-base/src` に取り出し、`target/perf-base/target` でビルドする
//! (終わったら worktree を外す)。基準に `bm_instr` が無い (この仕組みを入れる前のコミット) ときは比べずに今の値だけを出す。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::util::{run_capture, workspace_root};

const INSTR_LIMIT: f64 = 5.0;
const TIME_LIMIT: f64 = 10.0;

/// `bm_instr` をビルドし、実行ファイルのパスを返す。`manifest` が `None` なら今の作業ツリー。
fn build(manifest: Option<&Path>, target_dir: Option<&Path>) -> Result<Option<PathBuf>, String> {
    build_bench("bm_instr", manifest, target_dir)
}

/// eve-testkit の bench `name` をビルドし、実行ファイルのパスを返す (`no-network` も使う)。失敗したら `None`。
pub fn build_bench(
    name: &str,
    manifest: Option<&Path>,
    target_dir: Option<&Path>,
) -> Result<Option<PathBuf>, String> {
    let mut cmd = Command::new("cargo");
    cmd.args([
        "bench",
        "--no-run",
        "-p",
        "eve-testkit",
        "--bench",
        name,
        "--message-format=json-render-diagnostics",
    ]);
    if let Some(m) = manifest {
        cmd.arg("--manifest-path").arg(m);
    }
    if let Some(t) = target_dir {
        cmd.env("CARGO_TARGET_DIR", t);
    }
    let out = cmd
        .current_dir(workspace_root())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("cargo bench --no-run: {e}"))?;
    if !out.status.success() {
        // 基準に bm_instr が無い (この仕組みの前のコミット) と、ここで失敗する
        return Ok(None);
    }
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v["reason"] == "compiler-artifact"
            && v["target"]["name"] == name
            && let Some(exe) = v["executable"].as_str()
        {
            return Ok(Some(PathBuf::from(exe)));
        }
    }
    Ok(None)
}

fn cases(exe: &Path) -> Result<Vec<String>, String> {
    let out = run_capture(Command::new(exe).arg("--list"))?;
    Ok(out
        .lines()
        .filter_map(|l| l.split('\t').next())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect())
}

/// callgrind の命令数 (`Collected : <Ir>`)。
fn instructions(exe: &Path, case: &str, tmp: &Path) -> Result<u64, String> {
    let out = Command::new("valgrind")
        .args(["--tool=callgrind", "--quiet"])
        .arg(format!(
            "--callgrind-out-file={}",
            tmp.join("callgrind.out").display()
        ))
        .arg(exe)
        .arg(case)
        .output()
        .map_err(|e| {
            format!("valgrind を実行できません (Linux で apt-get install valgrind): {e}")
        })?;
    // --quiet でも要約は書かれないので、出力ファイルの `summary:` (または `totals:`) の行を読む
    let text = std::fs::read_to_string(tmp.join("callgrind.out"))
        .map_err(|e| format!("callgrind.out: {e}"))?;
    if !out.status.success() {
        return Err(format!("{case}: valgrind が失敗しました ({})", out.status));
    }
    text.lines()
        .find_map(|l| {
            l.strip_prefix("summary:")
                .or_else(|| l.strip_prefix("totals:"))
        })
        .and_then(|v| v.split_whitespace().next())
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| format!("{case}: callgrind の出力に命令数がありません"))
}

/// `--time` の 1 回ごとの時間 (ns)。
fn times(exe: &Path, case: &str, n: usize) -> Result<Vec<u64>, String> {
    let out = run_capture(Command::new(exe).args(["--time", case, &n.to_string()]))?;
    Ok(out.lines().filter_map(|l| l.trim().parse().ok()).collect())
}

fn median(mut v: Vec<u64>) -> u64 {
    v.sort_unstable();
    v.get(v.len() / 2).copied().unwrap_or(0)
}

struct Base {
    dir: PathBuf,
}

impl Drop for Base {
    fn drop(&mut self) {
        let _ = Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(&self.dir)
            .current_dir(workspace_root())
            .status();
    }
}

pub fn run(args: &[String]) -> Result<(), String> {
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let base_ref = flag("--base").ok_or("--base <ref> を指定してください")?;
    let mode = flag("--mode").unwrap_or_else(|| "instr".to_owned());
    if mode != "instr" && mode != "time" {
        return Err(format!("--mode は instr か time です: {mode}"));
    }
    let out_file = flag("--out");
    let root = workspace_root();
    let work = root.join("target").join("perf-base");
    let src = work.join("src");
    let _ = Command::new("git")
        .args(["worktree", "remove", "--force"])
        .arg(&src)
        .current_dir(&root)
        .status();
    let _ = std::fs::remove_dir_all(&src);
    std::fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;

    println!("▶ 今の作業ツリーの bm_instr をビルド");
    let cur = build(None, None)?.ok_or("今の作業ツリーで bm_instr をビルドできません")?;
    println!("▶ 基準 {base_ref} を取り出してビルド");
    let added = Command::new("git")
        .args(["worktree", "add", "--detach"])
        .arg(&src)
        .arg(&base_ref)
        .current_dir(&root)
        .status()
        .map_err(|e| format!("git worktree add: {e}"))?;
    if !added.success() {
        return Err(format!(
            "基準 {base_ref} を取り出せません (fetch してあるか)"
        ));
    }
    let _guard = Base { dir: src.clone() };
    let base = build(Some(&src.join("Cargo.toml")), Some(&work.join("target")))?;

    let names = cases(&cur)?;
    let base_names = match &base {
        Some(b) => cases(b).unwrap_or_default(),
        None => Vec::new(),
    };
    let tmp = work.join("tmp");
    std::fs::create_dir_all(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let (unit, limit) = if mode == "instr" {
        ("命令数 (Ir)", INSTR_LIMIT)
    } else {
        ("時間 (ms、中央値)", TIME_LIMIT)
    };
    let mut md = format!(
        "## 性能の比較 ({mode}、基準 `{base_ref}`)\n\n{unit}。+{limit}% を超えたら警告 (ブロックしない、docs/09 §5.4)。\n\n| ケース | 基準 | 今 | 差 | 判定 |\n|---|---|---|---|---|\n"
    );
    let mut warnings = Vec::new();
    for c in &names {
        let has_base = base.is_some() && base_names.contains(c);
        let (b, n) = if mode == "instr" {
            let n = instructions(&cur, c, &tmp)?;
            let b = if has_base {
                Some(instructions(base.as_ref().unwrap_or(&cur), c, &tmp)?)
            } else {
                None
            };
            (b, n)
        } else {
            // 同じランナーで交互に 3 回ずつ (各 5 回) 測り、中央値で比べる
            let (mut bv, mut nv) = (Vec::new(), Vec::new());
            for _ in 0..3 {
                if has_base && let Some(b) = &base {
                    bv.extend(times(b, c, 5)?);
                }
                nv.extend(times(&cur, c, 5)?);
            }
            ((!bv.is_empty()).then(|| median(bv)), median(nv))
        };
        let fmt = |v: u64| {
            if mode == "instr" {
                v.to_string()
            } else {
                format!("{:.2}", v as f64 / 1e6)
            }
        };
        let (diff, verdict) = match b {
            Some(b) if b > 0 => {
                let pct = (n as f64 - b as f64) * 100.0 / b as f64;
                let bad = pct > limit;
                if bad {
                    warnings.push(format!(
                        "{c}: {mode} が {pct:+.1}% (基準 {}、今 {})",
                        fmt(b),
                        fmt(n)
                    ));
                }
                (format!("{pct:+.1}%"), if bad { "**警告**" } else { "OK" })
            }
            _ => ("—".to_owned(), "基準なし"),
        };
        md.push_str(&format!(
            "| `{c}` | {} | {} | {diff} | {verdict} |\n",
            b.map_or("—".to_owned(), fmt),
            fmt(n)
        ));
    }
    if mode == "time" {
        md.push_str(
            "\n共有ランナーの wall-clock は揺れるので、警告は目安 (基準機の判定は 09 §5.3)。\n",
        );
    }
    println!("{md}");
    if std::env::var_os("GITHUB_ACTIONS").is_some() {
        for w in &warnings {
            println!("::warning title=性能の劣化 ({mode})::{w}");
        }
    }
    if let Some(f) = out_file {
        std::fs::write(&f, &md).map_err(|e| format!("{f}: {e}"))?;
    }
    Ok(())
}
