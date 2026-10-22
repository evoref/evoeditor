//! `cargo xtask verify-tag <タグ>`: リリースのタグの検証 (docs/09 §6.1 `release.yml` の最初のステップ、docs/11 §9.3)。
//!
//! 検査: (1) `git verify-tag` (署名の鍵は `--allowed-signers` のファイル (既定 `.github/allowed_signers`) に載るものだけ)、
//! (2) 正式版のタグは `origin/main` のマージコミット (`origin/main` から到達でき、親が 2 つ)、プレリリース (`-rc.N` 等) は
//! `origin/develop` の先頭、(3) タグの版と workspace の `Cargo.toml` の版が一致、(4) 正式版は、リリースのチェックリスト
//! (`docs/compat/<版>/release-checklist.md` の「判定コミット: <SHA>」) の判定コミットとの差分が許可リストのファイルだけ。
//! どれかが失敗したら終了コード 1。タグを作る・消す・push することはしない。
//!
//! 署名の鍵の一覧は、タグ自身のコミットではなく **`origin/develop` のもの** を使う (タグのコミットで一覧を書き換えて通すことを防ぐ)。
//! `release.yml` は `--allowed-signers` に、`origin/develop` から取り出したファイルを渡す。

use std::path::PathBuf;
use std::process::Command;

use crate::util::{run_capture, workspace_root};

/// 判定コミットとの差分に許す場所 (09 §9.2 手順 3)。
const ALLOWED_PREFIXES: &[&str] = &["docs/compat/", "docs/perf/"];
const ALLOWED_FILES: &[&str] = &["tests/ac-checklist.toml"];

pub fn run(args: &[String]) -> Result<(), String> {
    let signers_arg = args
        .iter()
        .position(|a| a == "--allowed-signers")
        .and_then(|i| args.get(i + 1));
    let tag = args
        .iter()
        .find(|a| !a.starts_with("--") && Some(*a) != signers_arg)
        .ok_or("使い方: cargo xtask verify-tag <タグ> [--allowed-signers FILE]")?;
    let signers =
        signers_arg.map_or_else(|| PathBuf::from(".github/allowed_signers"), PathBuf::from);
    let (version, pre) = parse_tag(tag)?;

    println!("▶ (1) 署名 (git verify-tag)");
    let signers = signers
        .canonicalize()
        .map_err(|e| format!("{}: {e}", signers.display()))?;
    run_capture(
        Command::new("git")
            .arg("-c")
            .arg(format!("gpg.ssh.allowedSignersFile={}", signers.display()))
            .args(["verify-tag", tag]),
    )
    .map_err(|e| format!("タグの署名を検証できません: {e}"))?;

    let commit = git(&["rev-parse", &format!("{tag}^{{commit}}")])?;
    println!("▶ (2) タグの位置 ({commit})");
    if pre {
        let head = git(&["rev-parse", "origin/develop"])?;
        if head != commit {
            return Err(format!(
                "プレリリースのタグは origin/develop の先頭 ({head}) を指す必要があります"
            ));
        }
    } else {
        if !git_ok(&["merge-base", "--is-ancestor", &commit, "origin/main"]) {
            return Err(
                "正式版のタグは origin/main から到達できるコミットを指す必要があります".into(),
            );
        }
        let parents = git(&["rev-list", "--parents", "-n", "1", &commit])?;
        if parents.split_whitespace().count() != 3 {
            return Err(format!(
                "正式版のタグは main のマージコミット (親が 2 つ) を指す必要があります: {parents}"
            ));
        }
    }

    println!("▶ (3) 版の一致");
    let manifest = git(&["show", &format!("{commit}:Cargo.toml")])?;
    let cargo = workspace_version(&manifest)?;
    if cargo != version {
        return Err(format!(
            "タグの版 {version} と Cargo.toml の版 {cargo} が違います"
        ));
    }

    if !pre {
        println!("▶ (4) 判定コミットとの内容の一致");
        let sheet = git(&[
            "show",
            &format!("{commit}:docs/compat/{version}/release-checklist.md"),
        ])
        .map_err(|e| format!("リリースのチェックリストを読めません: {e}"))?;
        let gate = gate_commit(&sheet)?;
        let diff = git(&["diff", "--name-only", &gate, &commit])?;
        let bad = disallowed(&diff);
        if !bad.is_empty() {
            return Err(format!(
                "判定コミット {gate} との差分に許可リスト外のファイルがあります (ゲートをやり直してください): {bad:?}"
            ));
        }
    }
    println!("verify-tag: OK ({tag})");
    Ok(())
}

fn git(args: &[&str]) -> Result<String, String> {
    Ok(run_capture(Command::new("git").args(args))?
        .trim()
        .to_owned())
}

fn git_ok(args: &[&str]) -> bool {
    Command::new("git")
        .args(args)
        .current_dir(workspace_root())
        .status()
        .is_ok_and(|s| s.success())
}

/// `vX.Y.Z` / `vX.Y.Z-rc.N` → (版, プレリリースか)。
fn parse_tag(tag: &str) -> Result<(String, bool), String> {
    let v = tag
        .strip_prefix('v')
        .ok_or_else(|| format!("タグは v で始まる必要があります: {tag}"))?;
    let (core, pre) = match v.split_once('-') {
        Some((c, p)) if !p.is_empty() => (c, true),
        Some(_) => return Err(format!("タグの接尾辞が空です: {tag}")),
        None => (v, false),
    };
    let ok = core.split('.').count() == 3
        && core
            .split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    if !ok {
        return Err(format!("タグの版が X.Y.Z の形ではありません: {tag}"));
    }
    Ok((v.to_owned(), pre))
}

fn workspace_version(manifest: &str) -> Result<String, String> {
    let v: toml::Value = toml::from_str(manifest).map_err(|e| format!("Cargo.toml: {e}"))?;
    v["workspace"]["package"]["version"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "Cargo.toml に workspace.package.version がありません".to_owned())
}

/// チェックリストの「判定コミット: <40 桁の SHA>」の行。
fn gate_commit(sheet: &str) -> Result<String, String> {
    for line in sheet.lines() {
        if let Some((_, rest)) = line.split_once("判定コミット:") {
            let sha: String = rest
                .trim()
                .trim_matches('`')
                .chars()
                .take_while(char::is_ascii_hexdigit)
                .collect();
            if sha.len() == 40 {
                return Ok(sha);
            }
        }
    }
    Err("チェックリストに「判定コミット: <40 桁の SHA>」の行がありません".to_owned())
}

/// 差分のファイルのうち、許可リストに無いもの。
fn disallowed(diff: &str) -> Vec<&str> {
    diff.lines()
        .filter(|f| !f.is_empty())
        .filter(|f| {
            !ALLOWED_FILES.contains(f) && !ALLOWED_PREFIXES.iter().any(|p| f.starts_with(p))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn タグを版とプレリリースに分ける() {
        assert_eq!(parse_tag("v0.1.0").unwrap(), ("0.1.0".into(), false));
        assert_eq!(parse_tag("v0.1.10").unwrap(), ("0.1.10".into(), false));
        assert_eq!(
            parse_tag("v0.2.0-rc.1").unwrap(),
            ("0.2.0-rc.1".into(), true)
        );
        for bad in ["0.1.0", "v0.1", "v0.1.x", "v0.1.0-", "vx"] {
            assert!(parse_tag(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn 判定コミットの行を読む() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(
            gate_commit(&format!("# x\n- 判定コミット: `{sha}`\n")).unwrap(),
            sha
        );
        assert_eq!(
            gate_commit(&format!("判定コミット: {sha} (develop の先頭)")).unwrap(),
            sha
        );
        assert!(gate_commit("判定コミット: abc").is_err());
        assert!(gate_commit("無し").is_err());
    }

    #[test]
    fn 差分は許可リストのファイルだけ() {
        let diff = "docs/compat/0.1.0/release-checklist.md\ndocs/perf/0.1.0/a.md\ntests/ac-checklist.toml\n";
        assert!(disallowed(diff).is_empty());
        let diff = "docs/compat/0.1.0/a.md\nsrc/x.rs\ndocs/08-roadmap.md\n";
        assert_eq!(disallowed(diff), ["src/x.rs", "docs/08-roadmap.md"]);
    }

    #[test]
    fn manifest_の版を読む() {
        assert_eq!(
            workspace_version("[workspace.package]\nversion = \"0.1.0\"\n").unwrap(),
            "0.1.0"
        );
    }
}
