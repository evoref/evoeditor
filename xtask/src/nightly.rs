//! `cargo xtask nightly`: 夜間ジョブの入口 (docs/09-testing-quality.md §6.1 / §6.3、docs/11 §1)。`.github/workflows/nightly.yml` の薄いトリガが
//! チェックアウトしたコードのこのコマンドを呼ぶ (手順の中身は対象のブランチの `xtask` にあるので、トリガを直さずに済む)。Linux 前提。
//!
//! v0.1 の手順 (`--only` で絞れる):
//! - `fuzz`: `fuzz_decode` と `fuzz_editor` (v0.2) を、それぞれ `--fuzz-seconds` (既定 1200 秒。09 §6.3 の v0.2)。`-close_fd_mask=3` で入力とスタックをログに出さない。クラッシュ入力は
//!   `.github/fuzz-recipients.txt` の公開鍵で `age` により暗号化し、平文は消す (artifact に上げるのは暗号化したものだけ)。
//! - `proptest`: `PROPTEST_CASES=10000` で Linux CI の対象 crate のテスト。
//! - `audit`: `cargo audit` と `cargo deny check advisories` (本体と `fuzz/`)。例外は `deny.toml` の `[advisories] ignore` の 1 か所。
//! - `coverage`: `cargo llvm-cov nextest --lcov` (報告のみ、OQ-101)。
//!
//! 結果の要約は `target/nightly/summary.txt` (ターゲット名・コミット・seed・クラッシュの件数・暗号化した artifact の名前だけ。
//! 入力の内容・ハッシュ・スタックは書かない)。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::util::{run, run_capture, workspace_root};

/// Linux CI の対象 crate (ci.yml の test (linux) と同じ、09 §6.4)。
const LINUX_CRATES: [&str; 9] = [
    "eve-text",
    "eve-encoding",
    "eve-view",
    "eve-config",
    "eve-core",
    "eve-host",
    "eve-testkit",
    "eve-docs-tables",
    "xtask",
];

const STEPS: [&str; 4] = ["fuzz", "proptest", "audit", "coverage"];

pub fn run_all(args: &[String]) -> Result<(), String> {
    let only: Vec<String> = flag(args, "--only")
        .map(|v| v.split(',').map(str::to_owned).collect())
        .unwrap_or_else(|| STEPS.iter().map(|s| (*s).to_owned()).collect());
    for o in &only {
        if !STEPS.contains(&o.as_str()) {
            return Err(format!(
                "知らない手順です: {o} (fuzz / proptest / audit / coverage)"
            ));
        }
    }
    let fuzz_seconds: u64 = flag(args, "--fuzz-seconds")
        .map(|v| {
            v.parse()
                .map_err(|_| format!("--fuzz-seconds が数ではありません: {v}"))
        })
        .transpose()?
        .unwrap_or(1200);
    let seed: u64 = flag(args, "--seed")
        .map(|v| {
            v.parse()
                .map_err(|_| format!("--seed が数ではありません: {v}"))
        })
        .transpose()?
        .unwrap_or_else(default_seed);

    let out_dir = workspace_root().join("target").join("nightly");
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let mut summary: Vec<String> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    summary.push(format!("commit: {}", commit()));

    for step in STEPS.iter().filter(|s| only.iter().any(|o| o == *s)) {
        let r = match *step {
            "fuzz" => fuzz(fuzz_seconds, seed, &out_dir, &mut summary),
            "proptest" => proptest(),
            "audit" => audit(),
            _ => coverage(&out_dir),
        };
        match r {
            Ok(()) => summary.push(format!("{step}: 成功")),
            Err(e) => {
                summary.push(format!("{step}: 失敗"));
                failures.push(format!("{step}: {e}"));
            }
        }
    }
    let text = summary.join("\n") + "\n";
    std::fs::write(out_dir.join("summary.txt"), &text)
        .map_err(|e| format!("summary.txt を書けません: {e}"))?;
    println!("\n{text}");
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
}

/// seed の既定: 日 (UNIX 時刻 / 86400)。同じ日の再実行は同じ seed になる。
fn default_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400)
}

fn commit() -> String {
    run_capture(Command::new("git").args(["rev-parse", "--short", "HEAD"]))
        .map(|s| s.trim().to_owned())
        .unwrap_or_else(|_| "unknown".to_owned())
}

/// libFuzzer が書く、失敗した入力のファイル名の接頭辞。
const CRASH_PREFIXES: [&str; 3] = ["crash-", "oom-", "timeout-"];

/// fuzz のターゲット (docs/09 §3.3)。それぞれ `--fuzz-seconds` 秒ずつ回す。
const FUZZ_TARGETS: [&str; 2] = ["fuzz_decode", "fuzz_editor"];

fn fuzz(seconds: u64, seed: u64, out_dir: &Path, summary: &mut Vec<String>) -> Result<(), String> {
    let artifacts = out_dir.join("fuzz-artifacts");
    let _ = std::fs::remove_dir_all(&artifacts);
    std::fs::create_dir_all(&artifacts).map_err(|e| format!("{}: {e}", artifacts.display()))?;
    let host = host_triple()?;
    let mut errs = Vec::new();
    for target in FUZZ_TARGETS {
        if let Err(e) = fuzz_one(target, &host, seconds, seed, &artifacts, summary) {
            errs.push(e);
        }
    }
    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs.join(
            "
",
        ))
    }
}

fn fuzz_one(
    target: &str,
    host: &str,
    seconds: u64,
    seed: u64,
    artifacts: &Path,
    summary: &mut Vec<String>,
) -> Result<(), String> {
    // 失敗した入力はターゲットごとの下位フォルダに受ける (暗号化した名前にはターゲット名を付ける)
    let artifacts = &artifacts.join(target);
    std::fs::create_dir_all(artifacts).map_err(|e| format!("{}: {e}", artifacts.display()))?;
    // ビルドの出力は見せる (入力もスタックも含まない)。失敗の原因を後から追えるようにする
    // 対象は nightly の rustc のホストにする。cargo-fuzz は既定で自分がビルドされた対象 (配布の実行ファイルは musl) を使うので、
    // 明示しないと「musl の std が無い」「sanitizer は静的リンクの libc と非互換」で失敗する
    run(
        &format!("cargo +nightly fuzz build {target}"),
        Command::new("cargo").args(["+nightly", "fuzz", "build", "--target", host, target]),
    )?;
    println!("▶ {target} ({seconds} 秒、seed {seed})");
    // 出力は受けて、クラッシュが見つかったときは出さない: 公開のログに入力とスタックを出さない (-close_fd_mask=3 に加えて、こちらでも受けて出さない)
    let prefix = format!("-artifact_prefix={}/", artifacts.display());
    let output = Command::new("cargo")
        .args(["+nightly", "fuzz", "run", "--target", host, target, "--"])
        .arg(format!("-max_total_time={seconds}"))
        .arg("-close_fd_mask=3")
        // 1 入力あたりの上限。既定の 1200 秒だと、ハングする入力 1 件で夜間ジョブが 20 分止まる
        .arg("-timeout=60")
        .arg(format!("-seed={seed}"))
        .arg(prefix)
        .current_dir(workspace_root())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("cargo fuzz を実行できません: {e}"))?;
    let status = output.status;

    let crashes = list_crashes(artifacts)?;
    summary.push(format!(
        "fuzz: ターゲット {target}、seed {seed}、クラッシュ {} 件",
        crashes.len()
    ));
    if crashes.is_empty() {
        return if status.success() {
            Ok(())
        } else {
            // クラッシュ入力が無い失敗 (ビルドや環境の問題) だけ、出力の末尾を示す。クラッシュがあるときは出さない
            let tail = |b: &[u8]| {
                let t = String::from_utf8_lossy(b);
                let lines: Vec<&str> = t.lines().collect();
                lines[lines.len().saturating_sub(30)..].join(
                    "
",
                )
            };
            Err(format!(
                "fuzz がクラッシュ無しで失敗しました ({status})。ビルドや環境の問題の可能性があります
--- stderr の末尾 ---
{}
--- stdout の末尾 ---
{}",
                tail(&output.stderr),
                tail(&output.stdout)
            ))
        };
    }
    // クラッシュ入力は暗号化してから平文を消す。暗号化できなければ平文を残さず失敗にする
    let recipients = workspace_root().join(".github").join("fuzz-recipients.txt");
    let mut encrypted: Vec<String> = Vec::new();
    for (i, f) in crashes.iter().enumerate() {
        // 種類 (crash / oom / timeout) は名前に残す (入力の内容ではないので公開してよい)。メンテナが調べる順序の手掛かりになる
        let kind = f
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.split('-').next())
            .unwrap_or("crash")
            .to_owned();
        // 上げる artifact は `fuzz-artifacts/*.age` (nightly.yml) なので、1 つ上に置く
        let dst = artifacts
            .parent()
            .unwrap_or(artifacts)
            .join(format!("{target}-{kind}-{:03}.age", i + 1));
        let ok = Command::new("age")
            .arg("-R")
            .arg(&recipients)
            .arg("-o")
            .arg(&dst)
            .arg(f)
            .status()
            .is_ok_and(|s| s.success());
        let _ = std::fs::remove_file(f);
        if ok {
            encrypted.push(
                dst.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            );
        } else {
            let _ = std::fs::remove_file(&dst);
        }
    }
    summary.push(format!(
        "fuzz: 暗号化した artifact {}",
        encrypted.join(", ")
    ));
    Err(format!(
        "{target} のクラッシュ {} 件 (暗号化できたもの {} 件。メンテナが手元で復号して再現する、09 §6.3)",
        crashes.len(),
        encrypted.len()
    ))
}

/// nightly の rustc のホストの対象 (`rustc +nightly -vV` の `host:`)。
fn host_triple() -> Result<String, String> {
    let out = run_capture(Command::new("rustc").args(["+nightly", "-vV"]))?;
    out.lines()
        .find_map(|l| l.strip_prefix("host: "))
        .map(|h| h.trim().to_owned())
        .ok_or_else(|| "rustc +nightly -vV に host の行がありません".to_owned())
}

fn list_crashes(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut v = Vec::new();
    for e in std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .flatten()
    {
        let name = e.file_name().to_string_lossy().into_owned();
        if CRASH_PREFIXES.iter().any(|p| name.starts_with(p)) && !name.ends_with(".age") {
            v.push(e.path());
        }
    }
    v.sort();
    Ok(v)
}

fn proptest() -> Result<(), String> {
    let mut cmd = Command::new("cargo");
    cmd.env("PROPTEST_CASES", "10000").args([
        "nextest",
        "run",
        "--profile",
        // 夜間の proptest は重い (eve-text の storage は 1 ケースで大きな文書を作る) ので、専用の profile で長い時間を許す
        "nightly",
        "--no-tests=pass",
    ]);
    for c in LINUX_CRATES {
        cmd.args(["-p", c]);
    }
    run("proptest 拡張 (PROPTEST_CASES=10000)", &mut cmd)
}

/// `deny.toml` の `[advisories] ignore` にある勧告 ID (`RUSTSEC-YYYY-NNNN`)。
fn ignored_advisories() -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(workspace_root().join("deny.toml")) else {
        return Vec::new();
    };
    let mut in_adv = false;
    let mut ids = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_adv = t == "[advisories]";
            continue;
        }
        if !in_adv || t.starts_with('#') {
            continue;
        }
        let mut rest = t;
        while let Some(i) = rest.find("RUSTSEC-") {
            let tail = &rest[i..];
            let end = tail
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                .unwrap_or(tail.len());
            ids.push(tail[..end].to_owned());
            rest = &tail[end..];
        }
    }
    ids
}

fn audit() -> Result<(), String> {
    let ids = ignored_advisories();
    let mut errors: Vec<String> = Vec::new();
    for (label, lock, manifest) in [
        ("本体", "Cargo.lock", "Cargo.toml"),
        ("fuzz/", "fuzz/Cargo.lock", "fuzz/Cargo.toml"),
    ] {
        let mut a = Command::new("cargo");
        a.args(["audit", "-f", lock]);
        for id in &ids {
            a.args(["--ignore", id]);
        }
        if let Err(e) = run(&format!("cargo audit ({label})"), &mut a) {
            errors.push(e);
        }
        let mut d = Command::new("cargo");
        d.args(["deny", "--manifest-path", manifest, "check", "advisories"]);
        if let Err(e) = run(&format!("cargo deny check advisories ({label})"), &mut d) {
            errors.push(e);
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

fn coverage(out_dir: &Path) -> Result<(), String> {
    let lcov = out_dir.join("lcov.info");
    let mut cmd = Command::new("cargo");
    cmd.args([
        "llvm-cov",
        "nextest",
        "--no-tests=pass",
        "--lcov",
        "--output-path",
    ])
    .arg(&lcov)
    .args(["--ignore-filename-regex", "crates/eve-testkit"]);
    for c in LINUX_CRATES {
        cmd.args(["-p", c]);
    }
    // v0.1 は報告のみ (OQ-101)。閾値のゲートは v0.4 から
    run("coverage (報告のみ)", &mut cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 手順の名前は_4_つ() {
        assert_eq!(STEPS.len(), 4);
    }

    #[test]
    fn 勧告_id_を拾う() {
        // deny.toml の現状は例外なし。形式だけ確かめる (実ファイルを読む)
        let ids = ignored_advisories();
        assert!(ids.iter().all(|i| i.starts_with("RUSTSEC-")));
    }

    #[test]
    fn 引数を読む() {
        let a = vec!["--fuzz-seconds".to_owned(), "30".to_owned()];
        assert_eq!(flag(&a, "--fuzz-seconds").map(String::as_str), Some("30"));
        assert!(flag(&a, "--seed").is_none());
    }

    #[test]
    fn クラッシュ入力のファイル名() {
        let dir = std::env::temp_dir().join(format!("eve-nightly-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for n in [
            "crash-aaa",
            "oom-bbb",
            "timeout-ccc",
            "crash-001.age",
            "other",
        ] {
            std::fs::write(dir.join(n), b"x").unwrap();
        }
        let got: Vec<String> = list_crashes(&dir)
            .unwrap()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(got, ["crash-aaa", "oom-bbb", "timeout-ccc"]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
