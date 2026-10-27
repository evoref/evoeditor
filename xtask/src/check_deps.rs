//! `cargo xtask check-deps`: 依存規則 R1〜R12 の検査 (docs/02-architecture.md §4.3、docs/09-testing-quality.md §7)。
//!
//! 入力は `cargo metadata` の出力と、リポジトリ直下の `deps-policy.toml`。検査が見る依存の辺は
//! normal 依存だけで、dev 依存と build 依存は対象外 (02 §4.1 の「禁止 crate 列」の注)。
//! `--docs` を付けると、docs/02 §4.1 の表の「禁止 crate」列と `deps-policy.toml` も突合する。

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;
use std::process::Command;

use serde::Deserialize;

use crate::util::{read_text, run_capture, rust_files, workspace_root};

// ---------- deps-policy.toml ----------

#[derive(Debug, Deserialize)]
struct Policy {
    limits: Limits,
    #[serde(default)]
    allowed_direct_dependencies: BTreeMap<String, Vec<String>>,
    thread_spawn: ThreadSpawn,
    #[serde(rename = "crate", default)]
    crates: BTreeMap<String, CrateRule>,
}

#[derive(Debug, Deserialize)]
struct Limits {
    max_transitive_dependencies: usize,
    max_c_dependencies: usize,
}

#[derive(Debug, Deserialize)]
struct ThreadSpawn {
    allowed_crates: Vec<String>,
    #[serde(default)]
    allowed_files: Vec<AllowedFile>,
}

#[derive(Debug, Deserialize)]
struct AllowedFile {
    #[serde(rename = "crate")]
    krate: String,
    file: String,
}

#[derive(Debug, Deserialize)]
struct CrateRule {
    forbidden: Vec<String>,
}

// ---------- cargo metadata ----------

#[derive(Debug, Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    workspace_members: Vec<String>,
    resolve: Resolve,
}

#[derive(Debug, Deserialize)]
struct Package {
    id: String,
    name: String,
    manifest_path: String,
    links: Option<String>,
    dependencies: Vec<Dependency>,
}

#[derive(Debug, Deserialize)]
struct Dependency {
    name: String,
    kind: Option<String>,
    optional: bool,
}

#[derive(Debug, Deserialize)]
struct Resolve {
    nodes: Vec<Node>,
}

#[derive(Debug, Deserialize)]
struct Node {
    id: String,
    deps: Vec<NodeDep>,
}

#[derive(Debug, Deserialize)]
struct NodeDep {
    pkg: String,
    dep_kinds: Vec<DepKind>,
}

#[derive(Debug, Deserialize)]
struct DepKind {
    kind: Option<String>,
}

/// 依存グラフ (normal 依存の辺だけ)。
struct Graph {
    names: BTreeMap<String, String>,      // id -> name
    edges: BTreeMap<String, Vec<String>>, // id -> 子の id (normal のみ)
    by_name: BTreeMap<String, String>,    // workspace member の name -> id
    packages: BTreeMap<String, Package>,  // id -> package
    members: BTreeSet<String>,            // workspace member の id
}

impl Graph {
    fn from_metadata(md: Metadata) -> Graph {
        let members: BTreeSet<String> = md.workspace_members.iter().cloned().collect();
        let mut names = BTreeMap::new();
        let mut by_name = BTreeMap::new();
        let mut packages = BTreeMap::new();
        for p in md.packages {
            names.insert(p.id.clone(), p.name.clone());
            if members.contains(&p.id) {
                by_name.insert(p.name.clone(), p.id.clone());
            }
            packages.insert(p.id.clone(), p);
        }
        let mut edges = BTreeMap::new();
        for n in md.resolve.nodes {
            let normal: Vec<String> = n
                .deps
                .into_iter()
                .filter(|d| d.dep_kinds.iter().any(|k| k.kind.is_none()))
                .map(|d| d.pkg)
                .collect();
            edges.insert(n.id, normal);
        }
        Graph {
            names,
            edges,
            by_name,
            packages,
            members,
        }
    }

    fn name(&self, id: &str) -> &str {
        self.names.get(id).map_or("?", String::as_str)
    }

    /// `start` から normal 依存の辺だけをたどって到達できる package を、経路つきで返す。
    fn reachable(&self, start: &str) -> BTreeMap<String, Vec<String>> {
        let mut seen: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut queue = VecDeque::from([(start.to_owned(), vec![self.name(start).to_owned()])]);
        while let Some((id, path)) = queue.pop_front() {
            for child in self.edges.get(&id).into_iter().flatten() {
                if child == start || seen.contains_key(child) {
                    continue;
                }
                let mut p = path.clone();
                p.push(self.name(child).to_owned());
                seen.insert(child.clone(), p.clone());
                queue.push_back((child.clone(), p));
            }
        }
        seen
    }
}

// ---------- 検査 ----------

#[derive(Default)]
struct Report {
    failures: Vec<String>,
}

impl Report {
    fn ok(&self, rule: &str, what: &str) {
        println!("  OK  {rule} {what}");
    }
    fn fail(&mut self, rule: &str, msg: String) {
        println!("  NG  {rule} {msg}");
        self.failures.push(format!("{rule}: {msg}"));
    }
}

pub fn run(with_docs: bool) -> Result<(), String> {
    let root = workspace_root();
    println!("▶ check-deps");
    let policy: Policy = toml::from_str(&read_text(&root.join("deps-policy.toml"))?)
        .map_err(|e| format!("deps-policy.toml を読めません: {e}"))?;
    let json =
        run_capture(Command::new("cargo").args(["metadata", "--format-version", "1", "--locked"]))?;
    let md: Metadata = serde_json::from_str(&json)
        .map_err(|e| format!("cargo metadata の出力を読めません: {e}"))?;
    let graph = Graph::from_metadata(md);
    let mut rep = Report::default();

    check_forbidden(&graph, &policy, &mut rep);
    check_allowed_direct(&graph, &policy, &mut rep);
    check_forbid_unsafe(&graph, &root, &mut rep);
    check_lexical(&graph, &policy, &root, &mut rep);
    check_limits(&graph, &policy, &mut rep);
    if with_docs {
        check_docs(&policy, &root, &mut rep);
    }

    if rep.failures.is_empty() {
        println!("check-deps: すべて通りました");
        Ok(())
    } else {
        Err(format!(
            "check-deps: {} 件の違反があります\n  - {}",
            rep.failures.len(),
            rep.failures.join("\n  - ")
        ))
    }
}

/// R1 / R2 / 禁止 crate 列: 各 crate から normal 依存で到達できる crate に、禁止 crate がない。
fn check_forbidden(g: &Graph, policy: &Policy, rep: &mut Report) {
    let mut checked = 0;
    for (name, rule) in &policy.crates {
        let Some(id) = g.by_name.get(name) else {
            continue;
        };
        checked += 1;
        let reach = g.reachable(id);
        let reach_names: BTreeMap<&str, &Vec<String>> = reach
            .iter()
            .map(|(rid, path)| (g.name(rid), path))
            .collect();
        let mut bad = false;
        for f in &rule.forbidden {
            if let Some(path) = reach_names.get(f.as_str()) {
                bad = true;
                rep.fail(
                    "R1/R2",
                    format!(
                        "{name} から禁止 crate `{f}` に到達します: {}",
                        path.join(" -> ")
                    ),
                );
            }
        }
        if !bad {
            rep.ok(
                "R1/R2",
                &format!("{name}: 禁止 crate {} 個に到達しない", rule.forbidden.len()),
            );
        }
    }
    if checked == 0 {
        rep.fail(
            "R1/R2",
            "検査できる crate がありません (deps-policy.toml と workspace が対応していない)".into(),
        );
    }
}

/// R4 / R10: 直接依存の集合が許可されたものだけ。R4 では serde が optional。
fn check_allowed_direct(g: &Graph, policy: &Policy, rep: &mut Report) {
    for (name, allowed) in &policy.allowed_direct_dependencies {
        let Some(id) = g.by_name.get(name) else {
            continue;
        };
        let pkg = &g.packages[id];
        let direct: Vec<&Dependency> = pkg
            .dependencies
            .iter()
            .filter(|d| d.kind.is_none())
            .collect();
        let mut bad = false;
        for d in &direct {
            if !allowed.contains(&d.name) {
                bad = true;
                rep.fail(
                    "R4/R10",
                    format!(
                        "{name} の直接依存 `{}` は許可されていません (許可: {allowed:?})",
                        d.name
                    ),
                );
            }
        }
        if name == "eve-view" {
            for d in direct.iter().filter(|d| d.name == "serde" && !d.optional) {
                bad = true;
                rep.fail(
                    "R4",
                    format!("eve-view の `{}` は optional でなければなりません", d.name),
                );
            }
        }
        if !bad {
            rep.ok(
                "R4/R10",
                &format!("{name}: 直接依存 {} 個はすべて許可済み", direct.len()),
            );
        }
    }
}

/// R5: unsafe を許す crate (eve-platform / eve-tui) 以外は `#![forbid(unsafe_code)]` を先頭に持つ。
fn check_forbid_unsafe(g: &Graph, root: &Path, rep: &mut Report) {
    const UNSAFE_OK: [&str; 2] = ["eve-platform", "eve-tui"];
    let mut count = 0;
    for (name, id) in &g.by_name {
        if UNSAFE_OK.contains(&name.as_str()) {
            continue;
        }
        let manifest = Path::new(&g.packages[id].manifest_path);
        let dir = manifest.parent().unwrap_or(root);
        let Some(entry) = ["src/lib.rs", "src/main.rs"]
            .iter()
            .map(|f| dir.join(f))
            .find(|p| p.exists())
        else {
            continue;
        };
        count += 1;
        match read_text(&entry) {
            Ok(text) if has_leading_forbid_unsafe(&text) => {}
            Ok(_) => rep.fail(
                "R5",
                format!(
                    "{name}: {} の先頭に `#![forbid(unsafe_code)]` がありません",
                    entry.display()
                ),
            ),
            Err(e) => rep.fail("R5", e),
        }
    }
    if rep.failures.iter().all(|f| !f.starts_with("R5")) {
        rep.ok(
            "R5",
            &format!("{count} crate が `#![forbid(unsafe_code)]` を持つ"),
        );
    }
}

fn has_leading_forbid_unsafe(text: &str) -> bool {
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with("//") {
            continue;
        }
        if l == "#![forbid(unsafe_code)]" {
            return true;
        }
        if !l.starts_with("#![") {
            return false; // 最初の項目まで来た
        }
    }
    false
}

/// R3 / R11 / R12 の字句走査 (clippy の disallowed_* を補う)。
fn check_lexical(g: &Graph, policy: &Policy, root: &Path, rep: &mut Report) {
    // R12: eve-core の src の中で eve_testkit を参照しない
    // R3 (補助): eve-core の src の中で std::fs / std::process / std::net / std::thread / Instant::now を使わない
    let core_src = root.join("crates/eve-core/src");
    let mut r3 = true;
    for file in rust_files(&core_src) {
        let Ok(text) = read_text(&file) else { continue };
        for (i, line) in text.lines().enumerate() {
            let l = line.trim_start();
            if l.starts_with("//") {
                continue;
            }
            let at = || {
                format!(
                    "{}:{}",
                    file.strip_prefix(root).unwrap_or(&file).display(),
                    i + 1
                )
            };
            if l.contains("eve_testkit") {
                rep.fail(
                    "R12",
                    format!(
                        "{} eve-core の src から eve_testkit を参照しています (統合テストに置く)",
                        at()
                    ),
                );
            }
            for pat in [
                "std::fs",
                "std::process",
                "std::net",
                "std::thread",
                "Instant::now",
                "SystemTime::now",
            ] {
                if l.contains(pat) {
                    r3 = false;
                    rep.fail(
                        "R3",
                        format!(
                            "{} `{pat}` を直接使っています (Port / JobSpawner / Clock 経由にする)",
                            at()
                        ),
                    );
                }
            }
        }
    }
    if r3 {
        rep.ok(
            "R3",
            "eve-core の src に std::fs / process / net / thread / Instant::now の直接の使用がない",
        );
    }
    if rep.failures.iter().all(|f| !f.starts_with("R12")) {
        rep.ok("R12", "eve-core の src が eve_testkit を参照しない");
    }

    // R11: thread::spawn / thread::Builder を使ってよいのは許可された crate とファイルだけ
    let mut r11 = true;
    for (name, id) in &g.by_name {
        if name == "xtask" || policy.thread_spawn.allowed_crates.contains(name) {
            continue;
        }
        let manifest = Path::new(&g.packages[id].manifest_path);
        let dir = manifest.parent().unwrap_or(root);
        for file in rust_files(&dir.join("src")) {
            let rel = file
                .strip_prefix(dir)
                .unwrap_or(&file)
                .to_string_lossy()
                .replace('\\', "/");
            let allowed = policy
                .thread_spawn
                .allowed_files
                .iter()
                .any(|a| &a.krate == name && a.file == rel);
            if allowed {
                continue;
            }
            let Ok(text) = read_text(&file) else { continue };
            for (i, line) in text.lines().enumerate() {
                let l = line.trim_start();
                if l.starts_with("//") {
                    continue;
                }
                if l.contains("thread::spawn") || l.contains("thread::Builder") {
                    r11 = false;
                    rep.fail("R11", format!("{name}/{rel}:{} スレッドを作ってよいのは許可された crate / ファイルだけ", i + 1));
                }
            }
        }
    }
    if r11 {
        rep.ok(
            "R11",
            "thread::spawn / Builder を許可された crate / ファイルの外で使っていない",
        );
    }
}

/// NFR-207: 推移的依存の総数 (報告) と C を含む依存の数 (失敗)。
/// 製品に入る crate (xtask と eve-docs-tables を除く workspace member) から到達できる外部 crate を数える。
fn check_limits(g: &Graph, policy: &Policy, rep: &mut Report) {
    const DEV_ONLY: [&str; 2] = ["xtask", "eve-docs-tables"];
    let mut external: BTreeSet<String> = BTreeSet::new();
    for (name, id) in &g.by_name {
        if DEV_ONLY.contains(&name.as_str()) {
            continue;
        }
        for rid in g.reachable(id).keys() {
            if !g.members.contains(rid) {
                external.insert(rid.clone());
            }
        }
    }
    let c_deps: Vec<&str> = external
        .iter()
        .filter(|id| g.packages.get(*id).is_some_and(|p| p.links.is_some()))
        .map(|id| g.name(id))
        .collect();
    let total = external.len();
    let note = if total > policy.limits.max_transitive_dependencies {
        " (上限超過。v0.1 では報告のみ)"
    } else {
        ""
    };
    println!(
        "  --  NFR-207 製品の推移的依存: {total} 個 (目安 {} 個以下){note}",
        policy.limits.max_transitive_dependencies
    );
    if c_deps.len() > policy.limits.max_c_dependencies {
        rep.fail(
            "NFR-207",
            format!(
                "C を含む依存が {} 個あります (上限 {}): {c_deps:?}",
                c_deps.len(),
                policy.limits.max_c_dependencies
            ),
        );
    } else {
        rep.ok(
            "NFR-207",
            &format!(
                "C を含む依存 {} 個 (上限 {})",
                c_deps.len(),
                policy.limits.max_c_dependencies
            ),
        );
    }
}

/// `--docs`: docs/02 §4.1 の表の「禁止 crate」列と deps-policy.toml の突合 (版が v0.1 の行)。
fn check_docs(policy: &Policy, root: &Path, rep: &mut Report) {
    let text = match read_text(&root.join("docs/02-architecture.md")) {
        Ok(t) => t,
        Err(e) => return rep.fail("docs", e),
    };
    let rows = match eve_docs_tables::crates::read_crates(&text) {
        Ok(r) => r,
        Err(e) => return rep.fail("docs", format!("02 §4.1 の表を読めません: {e}")),
    };
    let mut checked = 0;
    for row in rows.iter().filter(|r| r.version == "v0.1") {
        checked += 1;
        let Some(rule) = policy.crates.get(&row.name) else {
            rep.fail(
                "docs",
                format!(
                    "02 §4.1 の `{}` (行 {}) が deps-policy.toml にありません",
                    row.name, row.line
                ),
            );
            continue;
        };
        let doc: BTreeSet<&String> = row.forbidden.iter().collect();
        let pol: BTreeSet<&String> = rule.forbidden.iter().collect();
        if doc != pol {
            let only_doc: Vec<_> = doc.difference(&pol).collect();
            let only_pol: Vec<_> = pol.difference(&doc).collect();
            rep.fail("docs", format!("`{}` の禁止 crate が表と食い違います (表だけ: {only_doc:?}、deps-policy.toml だけ: {only_pol:?})", row.name));
        }
    }
    if rep.failures.iter().all(|f| !f.starts_with("docs")) {
        rep.ok(
            "docs",
            &format!("02 §4.1 の v0.1 の {checked} crate が deps-policy.toml と一致"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbid_unsafe_は項目より前になければならない() {
        assert!(has_leading_forbid_unsafe(
            "//! doc\n\n#![forbid(unsafe_code)]\n\nfn a() {}\n"
        ));
        assert!(has_leading_forbid_unsafe(
            "#![deny(missing_docs)]\n#![forbid(unsafe_code)]\n"
        ));
        assert!(!has_leading_forbid_unsafe(
            "fn a() {}\n#![forbid(unsafe_code)]\n"
        ));
        assert!(!has_leading_forbid_unsafe("//! doc\n"));
    }

    // verifies: AC-NFR-406-1
    #[test]
    fn 依存規則検査はこのリポジトリで通る() {
        // CI の `xtask check-deps` と同じ検査 (docs の突合を含む)。clippy の `disallowed_*` は `xtask check` の (2)
        run(true).expect("依存規則検査が通る");
    }

    // verifies: AC-NFR-704-1
    #[test]
    fn unsafe_を許す_crate_以外は_forbid_unsafe_code_を持つ() {
        let root = workspace_root();
        let json = run_capture(Command::new("cargo").args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
        ]))
        .expect("cargo metadata");
        let graph = Graph::from_metadata(serde_json::from_str(&json).expect("metadata"));
        let mut rep = Report::default();
        check_forbid_unsafe(&graph, &root, &mut rep);
        assert!(rep.failures.is_empty(), "{:?}", rep.failures);
    }
}
