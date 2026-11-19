//! `cargo xtask nightly`: 夜間ジョブの入口 (docs/09-testing-quality.md §6.1 / §6.3、docs/11 §1)。`.github/workflows/nightly.yml` の薄いトリガが
//! チェックアウトしたコードのこのコマンドを呼ぶ (手順の中身は対象のブランチの `xtask` にあるので、トリガを直さずに済む)。Linux 前提。
//!
//! v0.1 の手順 (`--only` で絞れる):
//! - `fuzz`: `fuzz_decode`・`fuzz_editor` (v0.2)・`fuzz_encode`・`fuzz_vt_input` (v0.3)・`fuzz_regex_search` (v0.4) を、それぞれ `--fuzz-seconds` (既定 960 秒。5 つの合計 80 分。09 §6.3)。
//!   ターゲットを絞らないときは、5 つの合計が `FUZZ_TOTAL_SECONDS` (80 分) を超えないように 1 つの時間を抑える (既定のブランチの
//!   nightly.yml が古い既定の 1200 秒を渡しても、夜間の段がジョブの上限 240 分に収まるように)。
//!   `--only fuzz:fuzz_decode+fuzz_editor` のように回すターゲットを絞れる (リリース前の各 1 時間、09 §8.2。5 つ × 1 時間はジョブの上限 240 分に収まらない)。`-close_fd_mask=3` で入力とスタックをログに出さない。クラッシュ入力は
//!   `.github/fuzz-recipients.txt` の公開鍵で `age` により暗号化し、平文は消す (artifact に上げるのは暗号化したものだけ)。
//! - `proptest`: `PROPTEST_CASES=10000` で Linux CI の対象 crate のテスト。
//! - `audit`: `cargo audit` と `cargo deny check advisories` (本体と `fuzz/`)。例外は `deny.toml` の `[advisories] ignore` の 1 か所。
//! - `coverage`: `cargo llvm-cov nextest --lcov` (報告のみ、OQ-101)。
//! - `crash` (v0.2): 保存のクラッシュ一貫性 (`cargo xtask crash-consistency`。`LazyFS` は環境変数 `LAZYFS_BIN`、下の FS は
//!   `EVE_CRASH_ROOTS`。nightly.yml が `LazyFS` をビルドし、ext4 / xfs / btrfs のループバックを用意する。NFR-301、09 §6.3)。
//!   `--only` を付けない実行で `LAZYFS_BIN` が無いときは、失敗にせず飛ばして要約に書く (定時の実行は既定ブランチ (main) の
//!   nightly.yml で動き、LazyFS を用意するステップを持たない版がある。`--only crash` と明示したら失敗にする)。
//! - `soak` (v0.2): NFR-306 の長時間の soak (BM-306、`cargo bench -p eve-testkit --bench bm_306`)。`--soak-minutes N` を
//!   渡したときだけ走らせる (nightly.yml が 60 を渡す。定時の実行が古い nightly.yml (時間の上限が足りない) で動くときは
//!   飛ばす)。RSS と入力の遅延の推移を `target/nightly/soak.jsonl` に書き、要約に傾向を 1 行で出す。**観察のみで失敗にしない**
//!   (09 §6.3、§8.2 の v0.2 の表)。`--only soak` と明示して `--soak-minutes` が無ければ 60 分。
//!
//! 結果の要約は `target/nightly/summary.txt` (ターゲット名・コミット・seed・クラッシュの件数・暗号化した artifact の名前だけ。
//! 入力の内容・ハッシュ・スタックは書かない)。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::util::{read_text, run, run_capture, workspace_root};

/// Linux CI の対象 crate (`Cargo.toml` の `[workspace.metadata.evoeditor] linux-ci-crates`、09 §6.4)。
/// ci.yml の clippy (linux) / test (linux) の `-p` の並びと同じ (試験で確かめる)。
fn linux_crates() -> Result<Vec<String>, String> {
    let text = read_text(&workspace_root().join("Cargo.toml"))?;
    let v: toml::Value = toml::from_str(&text).map_err(|e| format!("Cargo.toml: {e}"))?;
    let list = v["workspace"]["metadata"]["evoeditor"]["linux-ci-crates"]
        .as_array()
        .ok_or_else(|| {
            "Cargo.toml に workspace.metadata.evoeditor.linux-ci-crates がありません".to_owned()
        })?;
    list.iter()
        .map(|c| {
            c.as_str()
                .map(str::to_owned)
                .ok_or_else(|| "linux-ci-crates の要素は文字列にしてください".to_owned())
        })
        .collect()
}

const STEPS: [&str; 6] = ["fuzz", "proptest", "audit", "coverage", "crash", "soak"];

pub fn run_all(args: &[String]) -> Result<(), String> {
    let explicit = flag(args, "--only").is_some();
    let (only, fuzz_targets) = match flag(args, "--only") {
        Some(v) => parse_only(v)?,
        None => (STEPS.iter().map(|s| (*s).to_owned()).collect(), None),
    };
    let fuzz_seconds: u64 = flag(args, "--fuzz-seconds")
        .map(|v| {
            v.parse()
                .map_err(|_| format!("--fuzz-seconds が数ではありません: {v}"))
        })
        .transpose()?
        .unwrap_or(FUZZ_SECONDS_DEFAULT);
    let seed: u64 = flag(args, "--seed")
        .map(|v| {
            v.parse()
                .map_err(|_| format!("--seed が数ではありません: {v}"))
        })
        .transpose()?
        .unwrap_or_else(default_seed);
    let soak_minutes: Option<u64> = flag(args, "--soak-minutes")
        .map(|v| {
            v.parse()
                .map_err(|_| format!("--soak-minutes が数ではありません: {v}"))
        })
        .transpose()?
        .or(explicit.then_some(60));

    let out_dir = workspace_root().join("target").join("nightly");
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let mut summary: Vec<String> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    summary.push(format!("commit: {}", commit()));

    for step in STEPS.iter().filter(|s| only.iter().any(|o| o == *s)) {
        if *step == "crash" && skip_crash(explicit, std::env::var_os("LAZYFS_BIN").is_some()) {
            println!(
                "▶ crash: LAZYFS_BIN が無いので飛ばします (--only crash で明示すると失敗にします)"
            );
            summary.push(
                "crash: 飛ばした (LAZYFS_BIN が無い。nightly.yml が LazyFS を用意していない版)"
                    .to_owned(),
            );
            continue;
        }
        if *step == "soak" && soak_minutes.is_none() {
            println!("▶ soak: --soak-minutes が無いので飛ばします");
            summary.push(
                "soak: 飛ばした (--soak-minutes が無い。nightly.yml が soak を渡さない版)"
                    .to_owned(),
            );
            continue;
        }
        let r = match *step {
            "fuzz" => fuzz(
                fuzz_targets.as_deref().unwrap_or(&FUZZ_TARGETS),
                fuzz_seconds_for(fuzz_seconds, fuzz_targets.is_some()),
                seed,
                &out_dir,
                &mut summary,
            ),
            "soak" => soak(soak_minutes.unwrap_or(60), &out_dir, &mut summary),
            "proptest" => proptest(),
            "audit" => audit(),
            "coverage" => coverage(&out_dir),
            _ => crate::crash_consistency::run_cmd(&[]),
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

/// `--only` の値を読む: 手順のカンマ区切り。fuzz は `fuzz:<ターゲット>+<ターゲット>` で回すターゲットを絞れる
/// (例 `fuzz:fuzz_decode+fuzz_editor,audit`。リリース前の各 1 時間の fuzz を、ジョブの上限 240 分に収めるため、09 §8.2)。
/// 返すのは (手順, 絞ったときの fuzz のターゲット)。
fn parse_only(v: &str) -> Result<(Vec<String>, Option<Vec<&'static str>>), String> {
    let mut steps = Vec::new();
    let mut targets = None;
    for item in v.split(',') {
        let (step, rest) = item
            .split_once(':')
            .map_or((item, None), |(a, b)| (a, Some(b)));
        if !STEPS.contains(&step) {
            return Err(format!(
                "知らない手順です: {step} (fuzz / proptest / audit / coverage / crash / soak)"
            ));
        }
        if let Some(rest) = rest {
            if step != "fuzz" {
                return Err(format!("ターゲットを絞れるのは fuzz だけです: {item}"));
            }
            let mut list = Vec::new();
            for t in rest.split('+') {
                let known = FUZZ_TARGETS.iter().find(|k| **k == t).ok_or_else(|| {
                    format!(
                        "知らない fuzz のターゲットです: {t} ({})",
                        FUZZ_TARGETS.join(" / ")
                    )
                })?;
                list.push(*known);
            }
            targets = Some(list);
        }
        steps.push(step.to_owned());
    }
    Ok((steps, targets))
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

/// 1 つのターゲットを回す時間の既定 (秒)。5 つで合計 80 分。
const FUZZ_SECONDS_DEFAULT: u64 = 960;

/// ターゲットを絞らない夜間の fuzz の合計の上限 (秒)。夜間の段 (fuzz・proptest・audit・coverage・crash・soak) がジョブの上限
/// 240 分に収まるように (09 §8.2。v0.4 の M24 で 5 つ目を足したとき、20 分 × 5 では段の合計が 240 分に迫った)。
const FUZZ_TOTAL_SECONDS: u64 = 80 * 60;

/// 1 つのターゲットを回す時間。ターゲットを絞らないときは、合計が `FUZZ_TOTAL_SECONDS` を超えないように抑える (リリース前の
/// 各 1 時間は、ターゲットを `--only fuzz:<…>` で絞って回すので抑えない)。
fn fuzz_seconds_for(seconds: u64, narrowed: bool) -> u64 {
    if narrowed {
        return seconds;
    }
    seconds.min(FUZZ_TOTAL_SECONDS / FUZZ_TARGETS.len() as u64)
}

/// fuzz のターゲット (docs/09 §3.3)。それぞれ `--fuzz-seconds` 秒ずつ回す。
const FUZZ_TARGETS: [&str; 5] = [
    "fuzz_decode",
    "fuzz_editor",
    "fuzz_encode",
    "fuzz_vt_input",
    "fuzz_regex_search",
];

fn fuzz(
    targets: &[&str],
    seconds: u64,
    seed: u64,
    out_dir: &Path,
    summary: &mut Vec<String>,
) -> Result<(), String> {
    let artifacts = out_dir.join("fuzz-artifacts");
    let _ = std::fs::remove_dir_all(&artifacts);
    std::fs::create_dir_all(&artifacts).map_err(|e| format!("{}: {e}", artifacts.display()))?;
    let host = host_triple()?;
    let mut errs = Vec::new();
    for &target in targets {
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
    for c in linux_crates()? {
        cmd.args(["-p", &c]);
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
    for c in linux_crates()? {
        cmd.args(["-p", &c]);
    }
    // 報告のみ (OQ-101)。閾値のゲートは v0.5 から (NFR-702)
    run("coverage (報告のみ)", &mut cmd)
}

/// BM-306 の soak を回し、要約に傾向を 1 行で書く。傾向が平坦でなくても失敗にしない (観察のみ、09 §6.3)。
fn soak(minutes: u64, out_dir: &Path, summary: &mut Vec<String>) -> Result<(), String> {
    let out = out_dir.join("soak.jsonl");
    let minutes_s = minutes.to_string();
    let out_s = out.display().to_string();
    run(
        &format!("soak (BM-306、{minutes} 分)"),
        Command::new("cargo").args([
            "bench",
            "-q",
            "-p",
            "eve-testkit",
            "--bench",
            "bm_306",
            "--",
            "--minutes",
            &minutes_s,
            "--out",
            &out_s,
        ]),
    )?;
    let text = std::fs::read_to_string(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let line = text
        .lines()
        .rev()
        .find(|l| l.contains("\"kind\":\"summary\""))
        .ok_or("soak.jsonl に要約の行がありません")?;
    let v: serde_json::Value =
        serde_json::from_str(line).map_err(|e| format!("soak の要約を読めません: {e}"))?;
    summary.push(soak_line(&v));
    Ok(())
}

/// soak の要約の JSON を 1 行の所見にする。
fn soak_line(v: &serde_json::Value) -> String {
    let num = |k: &str| v.get(k).and_then(serde_json::Value::as_f64);
    let mib = |k: &str| num(k).map_or("-".to_owned(), |x| format!("{x:.1}"));
    format!(
        "soak (観察): {:.0} 分、RSS {}→{} MiB (増加 {:.1} MiB)、p99 {:.3}→{:.3} ms、{}",
        num("minutes").unwrap_or(0.0),
        mib("rss_first_mib"),
        mib("rss_last_mib"),
        num("rss_growth_mib").unwrap_or(0.0),
        num("p99_first_ms").unwrap_or(0.0),
        num("p99_last_ms").unwrap_or(0.0),
        if v.get("flat").and_then(serde_json::Value::as_bool) == Some(true) {
            "平坦"
        } else {
            "要確認 (右肩上がりか、遅延が 1.5 倍を超えた)"
        }
    )
}

/// `crash` を飛ばすか: 手順を明示せず (`--only` なし)、LazyFS が用意されていないとき。
fn skip_crash(explicit: bool, has_lazyfs: bool) -> bool {
    !explicit && !has_lazyfs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lazyfs_が無ければ_crash_を飛ばすのは明示しないときだけ() {
        assert!(skip_crash(false, false));
        assert!(!skip_crash(true, false));
        assert!(!skip_crash(false, true));
        assert!(!skip_crash(true, true));
    }

    #[test]
    fn linux_ci_の対象_crate_は_ci_yml_の_p_の並びと同じ() {
        let want = linux_crates().unwrap();
        let ci = read_text(&workspace_root().join(".github/workflows/ci.yml")).unwrap();
        // clippy (linux) と test (linux) の `args: -p ...` の行
        let lines: Vec<&str> = ci
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with("args: -p "))
            .collect();
        assert_eq!(
            lines.len(),
            2,
            "ci.yml の `args: -p` の行は 2 つ (clippy と test)"
        );
        for l in lines {
            let mut got = Vec::new();
            let mut words = l.split_whitespace();
            while let Some(w) = words.next() {
                if w == "-p" {
                    got.push(words.next().unwrap().to_owned());
                }
            }
            assert_eq!(
                got, want,
                "ci.yml と Cargo.toml の linux-ci-crates が違う: {l}"
            );
        }
    }

    #[test]
    fn only_で_fuzz_のターゲットを絞る() {
        let (steps, t) = parse_only("fuzz:fuzz_decode+fuzz_editor,audit").unwrap();
        assert_eq!(steps, ["fuzz", "audit"]);
        assert_eq!(t, Some(vec!["fuzz_decode", "fuzz_editor"]));
        // 絞らない夜間は合計 80 分 (1 つ 960 秒) に抑え、絞ったリリース前の 1 時間は抑えない
        assert_eq!(fuzz_seconds_for(1200, false), 960);
        assert_eq!(fuzz_seconds_for(600, false), 600);
        assert_eq!(fuzz_seconds_for(3600, true), 3600);
        assert_eq!(
            FUZZ_SECONDS_DEFAULT * FUZZ_TARGETS.len() as u64,
            FUZZ_TOTAL_SECONDS
        );
        let (steps, t) = parse_only("fuzz,crash").unwrap();
        assert_eq!(steps, ["fuzz", "crash"]);
        assert_eq!(t, None);
        assert!(parse_only("fuzz:fuzz_nothing").is_err());
        assert!(parse_only("audit:fuzz_decode").is_err());
        assert!(parse_only("fuzzz").is_err());
    }

    #[test]
    fn 手順の名前は_6_つ() {
        assert_eq!(STEPS.len(), 6);
    }

    #[test]
    fn soak_の要約を_1_行にする() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"kind":"summary","minutes":60.0,"rss_first_mib":19.4,"rss_last_mib":19.6,"rss_growth_mib":0.2,"p99_first_ms":0.3,"p99_last_ms":0.31,"flat":true}"#,
        )
        .unwrap();
        assert_eq!(
            soak_line(&v),
            "soak (観察): 60 分、RSS 19.4→19.6 MiB (増加 0.2 MiB)、p99 0.300→0.310 ms、平坦"
        );
        let w: serde_json::Value =
            serde_json::from_str(r#"{"minutes":1,"rss_first_mib":null,"flat":false}"#).unwrap();
        assert!(soak_line(&w).contains("RSS -→- MiB"));
        assert!(soak_line(&w).ends_with("要確認 (右肩上がりか、遅延が 1.5 倍を超えた)"));
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
