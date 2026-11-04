//! `xtask trace`: 仕様と実装・試験の対応の検査と、トレーサビリティ行列 `docs/traceability.md` の生成
//! (docs/11 §3.3、09 §8.3、NFR-708 AC-2)。
//!
//! - ソースのコメントの `verifies:` が指す ID (AC ID・要件 ID・規範行 ID) と、`spec:` が指す規範行 ID が存在すること。
//!   存在しなければ失敗。
//! - v0.2 の受入基準 (01 §5.5 の v0.2 の行) ごとに、それを確かめる試験があるかを `docs/traceability.md` に並べる。
//!   単位は「要件」と、番号付きの AC があれば「AC」。試験の `verifies:` が AC ID か要件 ID を指していれば確かめたと数える。
//!   Must の要件で何も無いものは警告する (版の終盤に失敗へ、09 §8.3)。v0.1 の AC は `tests/ac-checklist.toml` が受ける。
//!
//! 規範行 ID は全文書に一括では付けず、付いたものだけを存在の検査に使う (11 §3.1 ID-6)。

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use crate::check_docs::id_refs;
use crate::spec_index::scan;
use crate::util::{read_text, rust_files, workspace_root};

const MATRIX: &str = "docs/traceability.md";

/// ソースの `verifies:` 1 件。
#[derive(Clone, Debug)]
struct Verify {
    id: String,
    at: String,
    test: String,
}

/// v0.2 の検証の単位。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Unit {
    requirement: String,
    /// 番号付きの AC があればその ID (`AC-FR-101-1`)。無ければ要件だけ。
    ac: Option<String>,
}

/// 規範行 ID の形 (`04-4.2-3`)。
fn is_row_id(s: &str) -> bool {
    let p: Vec<&str> = s.split('-').collect();
    p.len() == 3
        && p[0].len() == 2
        && p[0].bytes().all(|b| b.is_ascii_digit())
        && p[1]
            .split('.')
            .all(|x| !x.is_empty() && x.bytes().all(|b| b.is_ascii_digit()))
        && !p[2].is_empty()
        && p[2].bytes().all(|b| b.is_ascii_digit())
}

/// ソースのコメントの `verifies:` とシナリオ DSL の `verifies`、`spec:` の中の規範行 ID を集める。
fn scan_sources(root: &Path) -> (Vec<Verify>, Vec<(String, String)>) {
    let mut verifies = Vec::new();
    let mut spec_rows = Vec::new();
    for dir in ["crates", "bins", "xtask"] {
        for f in rust_files(&root.join(dir)) {
            let Ok(text) = std::fs::read_to_string(&f) else {
                continue;
            };
            let rel = f
                .strip_prefix(root)
                .unwrap_or(&f)
                .to_string_lossy()
                .replace('\\', "/");
            let lines: Vec<&str> = text.lines().collect();
            for (i, l) in lines.iter().enumerate() {
                let t = l.trim_start();
                if !t.starts_with("//") {
                    continue;
                }
                let body = t
                    .trim_start_matches('/')
                    .trim_start_matches('!')
                    .trim_start();
                let at = format!("{rel}:{}", i + 1);
                if let Some(rest) = body.strip_prefix("verifies:") {
                    // 直後 5 行以内の `fn <名前>` が対象の試験
                    let test = lines[i + 1..lines.len().min(i + 6)]
                        .iter()
                        .find_map(|n| {
                            let n = n.trim_start();
                            let n = n.strip_prefix("pub ").unwrap_or(n);
                            n.strip_prefix("fn ")
                                .map(|r| r.split(['(', '<']).next().unwrap_or("").to_owned())
                        })
                        .unwrap_or_default();
                    for tok in rest.split([',', '、']) {
                        let id = tok
                            .split_whitespace()
                            .next()
                            .unwrap_or("")
                            .trim_end_matches(['.', '。']);
                        if !id.is_empty() {
                            verifies.push(Verify {
                                id: id.to_owned(),
                                at: at.clone(),
                                test: test.clone(),
                            });
                        }
                    }
                } else if let Some(rest) = body.strip_prefix("spec:") {
                    for w in rest.split(|c: char| {
                        c.is_whitespace() || matches!(c, ',' | '、' | '(' | ')' | '/')
                    }) {
                        if is_row_id(w) {
                            spec_rows.push((at.clone(), w.to_owned()));
                        }
                    }
                }
            }
        }
    }
    // シナリオ DSL の `[meta] verifies` (試験名は `scenario:<id>`)
    for (rel, path) in scenario_files(&root.join("tests/scenarios")) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(doc) = text.parse::<toml::Table>() else {
            continue;
        };
        let meta = doc.get("meta").and_then(toml::Value::as_table);
        let id = meta
            .and_then(|m| m.get("id"))
            .and_then(toml::Value::as_str)
            .unwrap_or("");
        for v in meta
            .and_then(|m| m.get("verifies"))
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(toml::Value::as_str)
        {
            verifies.push(Verify {
                id: v.to_owned(),
                at: format!("tests/scenarios/{rel}"),
                test: format!("scenario:{id}"),
            });
        }
    }
    (verifies, spec_rows)
}

/// `tests/scenarios` の下の `.toml` (相対パス, 絶対パス)。
fn scenario_files(dir: &Path) -> Vec<(String, std::path::PathBuf)> {
    fn walk(base: &Path, d: &Path, out: &mut Vec<(String, std::path::PathBuf)>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(base, &p, out);
            } else if p.extension().is_some_and(|x| x == "toml") {
                let rel = p
                    .strip_prefix(base)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, p));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// 01 §5.5 の v0.2 の行から検証の単位を取り出す。
fn v02_units(doc01: &str) -> Vec<Unit> {
    let mut out = BTreeSet::new();
    let mut inside = false;
    for l in doc01.lines() {
        if l.starts_with("## ") || l.starts_with("### ") {
            inside = l.starts_with("### 5.5 ");
            continue;
        }
        if !inside || !l.trim_start().starts_with('|') || l.contains("---") {
            continue;
        }
        let cells: Vec<&str> = l
            .trim()
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        if cells.len() < 2 || cells[0] == "要件" || !cells[0].contains("v0.2") {
            continue;
        }
        let reqs: Vec<String> = id_refs(cells[0])
            .into_iter()
            .filter(|r| r.starts_with("FR-") || r.starts_with("NFR-"))
            .collect();
        let Some(first) = reqs.first().cloned() else {
            continue;
        };
        let mut with_ac: BTreeSet<String> = BTreeSet::new();
        for (req, n) in ac_heads(cells[1]) {
            let req = req.unwrap_or_else(|| first.clone());
            with_ac.insert(req.clone());
            out.insert(Unit {
                ac: Some(format!("AC-{req}-{n}")),
                requirement: req,
            });
        }
        for r in reqs {
            if !with_ac.contains(&r) {
                out.insert(Unit {
                    requirement: r,
                    ac: None,
                });
            }
        }
    }
    out.into_iter().collect()
}

/// 受入基準のセルの `AC-<n>:` (間に括弧の注記を許す) と、その直前に書かれた要件 ID (`FR-101 AC-1:`)。
fn ac_heads(cell: &str) -> Vec<(Option<String>, u32)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(p) = cell[from..].find("AC-") {
        let at = from + p;
        from = at + 3;
        let digits: String = cell[from..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        let Ok(n) = digits.parse::<u32>() else {
            continue;
        };
        let mut rest = cell[from + digits.len()..].trim_start();
        if rest.starts_with('(') {
            match rest.find(')') {
                Some(q) => rest = rest[q + 1..].trim_start(),
                None => continue,
            }
        }
        if !rest.starts_with([':', '：']) {
            continue;
        }
        // 直前の語が要件 ID なら、その要件の AC
        let before = cell[..at].trim_end();
        let last = before.rsplit([' ', '。', '、', '(']).next().unwrap_or("");
        let req = (last.starts_with("FR-") || last.starts_with("NFR-")).then(|| last.to_owned());
        out.push((req, n));
    }
    out
}

/// 要件の優先度 (01 の要件の表の行の `Must` / `Should` / `Could` で始まるセル)。
fn priorities(doc01: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for l in doc01
        .lines()
        .filter(|l| l.starts_with("| FR-") || l.starts_with("| NFR-"))
    {
        let cells: Vec<&str> = l
            .trim()
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        let Some(id) = cells.first() else { continue };
        if out.contains_key(*id) {
            continue;
        }
        if let Some(p) = cells.iter().skip(1).find_map(|c| {
            ["Must", "Should", "Could"]
                .into_iter()
                .find(|p| c.starts_with(p))
        }) {
            out.insert((*id).to_owned(), p.to_owned());
        }
    }
    out
}

struct Report {
    errors: Vec<String>,
    warnings: Vec<String>,
    matrix: String,
}

fn build(root: &Path) -> Result<Report, String> {
    let doc01 = read_text(&root.join("docs/01-requirements.md"))?;
    let (_, ids) = scan(root)?;
    let known: BTreeSet<String> = ids.iter().map(|i| i.id.clone()).collect();
    let v01: BTreeSet<String> = eve_docs_tables::ac::read_acs(&doc01)
        .map_err(|e| format!("01 §5 の AC 表を読めません: {e}"))?
        .into_iter()
        .map(|a| a.id)
        .collect();
    let units = v02_units(&doc01);
    let v02_acs: BTreeSet<String> = units.iter().filter_map(|u| u.ac.clone()).collect();
    let prio = priorities(&doc01);
    let (verifies, spec_rows) = scan_sources(root);

    let mut errors = Vec::new();
    for v in &verifies {
        let ok = if v.id.starts_with("AC-") {
            v01.contains(&v.id) || v02_acs.contains(&v.id)
        } else if v.id.starts_with("FR-") || v.id.starts_with("NFR-") || is_row_id(&v.id) {
            known.contains(&v.id)
        } else {
            false
        };
        if !ok {
            errors.push(format!(
                "{}: `verifies: {}` は 01 にも索引にもありません",
                v.at, v.id
            ));
        }
    }
    for (at, r) in &spec_rows {
        if !known.contains(r) {
            errors.push(format!(
                "{at}: `spec:` の規範行 ID `{r}` は索引にありません"
            ));
        }
    }

    // 単位ごとの検証 (AC ID か要件 ID を指す試験)
    let mut by_id: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for v in &verifies {
        if !v.test.is_empty() {
            by_id
                .entry(v.id.as_str())
                .or_default()
                .insert(v.test.as_str());
        }
    }
    let mut warnings = Vec::new();
    let mut m = String::new();
    m.push_str("# トレーサビリティ行列 (v0.2)\n\n");
    m.push_str("生成物 (`cargo xtask trace` で作り直す。手で直さない。docs/11 §3.3、09 §8.3)。01 §5.5 の v0.2 の受入基準ごとに、それを確かめる試験 (`// verifies:` の直後の関数) を並べる。試験が要件 ID を指していれば、その要件の AC すべてを確かめたと数える。v0.1 の AC は `tests/ac-checklist.toml` を見る。\n\n");
    m.push_str("| 要件 | AC | 優先度 | 試験 |\n|---|---|---|---|\n");
    let (mut covered, mut total) = (0usize, 0usize);
    for u in &units {
        let p = prio.get(&u.requirement).map_or("—", String::as_str);
        let mut tests: BTreeSet<&str> = by_id
            .get(u.requirement.as_str())
            .cloned()
            .unwrap_or_default();
        if let Some(ac) = &u.ac {
            tests.extend(by_id.get(ac.as_str()).cloned().unwrap_or_default());
        }
        total += 1;
        let cell = if tests.is_empty() {
            if p == "Must" {
                warnings.push(format!(
                    "{} {} (Must) を確かめる試験がありません",
                    u.requirement,
                    u.ac.as_deref().unwrap_or("")
                ));
            }
            "**なし**".to_owned()
        } else {
            covered += 1;
            tests
                .iter()
                .map(|t| format!("`{t}`"))
                .collect::<Vec<_>>()
                .join("<br>")
        };
        let _ = writeln!(
            m,
            "| {} | {} | {p} | {cell} |",
            u.requirement,
            u.ac.as_deref().unwrap_or("(番号なし)")
        );
    }
    let _ = write!(m, "\n試験のある単位: {covered} / {total}\n");
    Ok(Report {
        errors,
        warnings,
        matrix: m,
    })
}

pub fn run(args: &[String]) -> Result<(), String> {
    let root = workspace_root();
    let r = build(&root)?;
    let check = args.iter().any(|a| a == "--check");
    println!("▶ trace{}", if check { " --check" } else { "" });
    for w in &r.warnings {
        println!("  警告: {w}");
    }
    let mut errors = r.errors;
    if check {
        let have = std::fs::read_to_string(root.join(MATRIX)).unwrap_or_default();
        if have.replace("\r\n", "\n") != r.matrix {
            errors.push(format!(
                "{MATRIX} が古い: `cargo xtask trace` で作り直してコミットしてください"
            ));
        }
    } else {
        std::fs::write(root.join(MATRIX), &r.matrix).map_err(|e| format!("{MATRIX}: {e}"))?;
        println!("  {MATRIX} を書きました");
    }
    if errors.is_empty() {
        println!("  OK (警告 {} 件)", r.warnings.len());
        Ok(())
    } else {
        Err(format!(
            "trace に {} 件の問題があります\n  - {}",
            errors.len(),
            errors.join("\n  - ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 受入基準の_ac_の頭を読む() {
        assert_eq!(
            ac_heads("FR-101 AC-1: 起動する。FR-103 AC-1: 一覧。AC-2 (v0.3): 後。AC-3 は本文"),
            [
                (Some("FR-101".to_owned()), 1),
                (Some("FR-103".to_owned()), 1),
                (None, 2)
            ]
        );
    }

    #[test]
    fn v02_の単位を取り出す() {
        let doc = "\
### 5.5 v0.2 以降
| 要件 | 受入基準 (概要) |
|---|---|
| FR-103 / FR-101 (v0.2) | FR-101 AC-1: a。FR-103 AC-2: b |
| NFR-106 長大行 (v0.2) | AC-1: c。AC-2: d |
| FR-518 (v0.2、Could) | 文だけ |
| FR-122 (v0.3) | AC-1: 後の版 |
## 6. 次
";
        let got: Vec<_> = v02_units(doc)
            .into_iter()
            .map(|u| format!("{} {}", u.requirement, u.ac.unwrap_or_default()))
            .collect();
        assert_eq!(
            got,
            [
                "FR-101 AC-FR-101-1",
                "FR-103 AC-FR-103-2",
                "FR-518 ",
                "NFR-106 AC-NFR-106-1",
                "NFR-106 AC-NFR-106-2",
            ]
        );
    }

    /// リポジトリの `verifies:` / `spec:` が存在する ID を指し、行列が最新 (CI の検査と同じ)。
    // verifies: NFR-708
    #[test]
    fn リポジトリの対応は壊れておらず行列は最新() {
        run(&["--check".to_owned()]).unwrap();
    }
}
