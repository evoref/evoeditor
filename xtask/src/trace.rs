//! `xtask trace`: 仕様と実装・試験の対応の検査と、トレーサビリティ行列 `docs/traceability.md` の生成
//! (docs/11 §3.3、09 §8.3、NFR-708 AC-2)。
//!
//! - ソースのコメントの `verifies:` が指す ID (AC ID・要件 ID・規範行 ID) と、`spec:` が指す規範行 ID が存在すること。
//!   ベンチの表 (09 §5.2) と手順書 (`docs/compat/checklist.md`) に書いた AC の参照も、AC が存在すること。存在しなければ失敗。
//! - 01 §5.5 の版の行 ([`VERSIONS`]) の受入基準ごとに、それを確かめる試験・ベンチ・手順書の項目があるかを
//!   `docs/traceability.md` に並べる。単位は「要件」と、番号付きの AC があれば「AC」。試験の `verifies:` が AC ID か要件 ID を
//!   指していれば確かめたと数える。ベンチと手順書は、表の行・項目の行に `FR-122 AC-2` (範囲 `AC-2〜AC-4`、並び `AC-1・AC-3`)
//!   か `AC-FR-122-2` の形で書いた AC だけを数える (要件 ID だけの参照は数えない)。
//!   Must の要件で何も無いものは警告する (版の終盤に失敗へ、09 §8.3)。後の版の AC に替わった AC (その AC の文に
//!   「に替わる」がある) は警告しない。`--version vX.Y` は警告をその版の行に絞る。v0.1 の AC は `tests/ac-checklist.toml` が受ける。
//!
//! 規範行 ID は全文書に一括では付けず、付いたものだけを存在の検査に使う (11 §3.1 ID-6)。

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

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

/// ソースのコメントの `verifies:` (`crate::verifies` の走査) とシナリオ DSL の `verifies`、`spec:` の中の規範行 ID を集める。
/// 3 つ目は `verifies:` の規則に合わないものの説明 (`--check` で誤りにする)。
fn scan_sources(root: &Path) -> (Vec<Verify>, Vec<(String, String)>, Vec<String>) {
    let (found, scan_errors) = crate::verifies::scan(root);
    let mut verifies: Vec<Verify> = found
        .iter()
        .flat_map(|v| {
            v.ids.iter().map(|id| Verify {
                id: id.clone(),
                at: v.at(),
                test: v.test.clone(),
            })
        })
        .collect();
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
            for (i, l) in text.lines().enumerate() {
                let t = l.trim_start();
                if !t.starts_with("//") {
                    continue;
                }
                let body = t
                    .trim_start_matches('/')
                    .trim_start_matches('!')
                    .trim_start();
                if let Some(rest) = body.strip_prefix("spec:") {
                    let at = format!("{rel}:{}", i + 1);
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
    (verifies, spec_rows, scan_errors)
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

/// 行列に並べる版 (01 §5.5 の `要件` 列の版の注記)。v0.3 は整理の版で §5.5 に行が無い。次の版の行を足したら、ここにも足す。
const VERSIONS: &[&str] = &["v0.2", "v0.4"];

/// ベンチの表 (09 §5.2) の見出し。
const BENCH_HEADING: &str = "### 5.2 NFR とベンチ ID の対応";

/// 手での確認の手順書 (09 §3.7〜3.8)。
const CHECKLIST: &str = "docs/compat/checklist.md";

/// `s` の先頭の数字の並び (数, 残り)。
fn digits(s: &str) -> Option<(u32, &str)> {
    let n = s.bytes().take_while(u8::is_ascii_digit).count();
    s[..n].parse().ok().map(|v| (v, &s[n..]))
}

/// `s` の先頭の要件 ID (`FR-122` / `NFR-106`。書かれたとおり) と残り。
fn req_prefix(s: &str) -> Option<(String, &str)> {
    let p = if s.starts_with("NFR-") {
        4
    } else if s.starts_with("FR-") {
        3
    } else {
        return None;
    };
    let n = s[p..].bytes().take_while(u8::is_ascii_digit).count();
    (n > 0).then(|| (s[..p + n].to_owned(), &s[p + n..]))
}

/// 文の中の AC の参照を、書かれた順に取り出す (重複は除く)。`FR-122 AC-2`・`FR-122 AC-2〜AC-4` (範囲)・
/// `NFR-106 AC-1・AC-3` (並び。区切りは `・` / `、` / ` / `)・`AC-FR-122-2` の形。要件 ID の後ろに `AC-` が続かない
/// 参照 (`NFR-604` だけ) は数えない。
fn ac_refs(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |id: String| {
        if !out.contains(&id) {
            out.push(id);
        }
    };
    let b = text.as_bytes();
    for (i, _) in text.char_indices() {
        if i > 0 && (b[i - 1].is_ascii_alphanumeric() || matches!(b[i - 1], b'-' | b'_')) {
            continue;
        }
        let rest = &text[i..];
        if let Some(r) = rest.strip_prefix("AC-") {
            // 完全な形 (`AC-FR-122-2`)
            if let Some((req, t)) = req_prefix(r)
                && let Some((n, _)) = t.strip_prefix('-').and_then(digits)
            {
                push(format!("AC-{req}-{n}"));
            }
            continue;
        }
        let Some((req, t)) = req_prefix(rest) else {
            continue;
        };
        let Some((mut n, mut tail)) = t.strip_prefix(" AC-").and_then(digits) else {
            continue;
        };
        push(format!("AC-{req}-{n}"));
        loop {
            let next = [("〜", true), ("・", false), ("、", false), (" / ", false)]
                .into_iter()
                .find_map(|(sep, range)| {
                    tail.strip_prefix(sep)
                        .and_then(|t| t.strip_prefix("AC-"))
                        .and_then(digits)
                        .map(|(m, t)| (range, m, t))
                });
            let Some((range, m, t)) = next else { break };
            if range {
                for k in n + 1..=m {
                    push(format!("AC-{req}-{k}"));
                }
            } else {
                push(format!("AC-{req}-{m}"));
            }
            (n, tail) = (m, t);
        }
    }
    out
}

/// 文の中のベンチ ID (`BM-104v-100m`・`BM-104-{1m,10m,100m}`)。
fn bench_ids(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(p) = text[from..].find("BM-") {
        let at = from + p;
        let mut depth = 0usize;
        let len = text[at..]
            .char_indices()
            .find(|&(_, c)| {
                match c {
                    '{' => depth += 1,
                    '}' => depth = depth.saturating_sub(1),
                    ',' if depth > 0 => {}
                    c if c.is_ascii_alphanumeric() || c == '-' => {}
                    _ => return true,
                }
                false
            })
            .map_or(text.len() - at, |(i, _)| i);
        let id = text[at..at + len].trim_end_matches('-');
        if id.len() > 3 {
            out.push(id.to_owned());
        }
        from = at + len.max(3);
    }
    out
}

/// AC の参照の 1 件 (ベンチの表の行・手順書の項目)。
#[derive(Clone, Debug)]
struct Ref {
    ac: String,
    /// ベンチ ID か手順書の項目 (`G4`。番号の無い行は節の記号 `P`)。
    by: String,
    /// `<file>:<line>` (報告用)。
    at: String,
}

/// 09 §5.2 の表の行の `NFR` 列に書いた AC と、その行の `ベンチ ID` 列のベンチ。`NFR` 列が空の行は前の行の続き。
fn bench_refs(doc09: &str) -> Result<Vec<Ref>, String> {
    let tables = eve_docs_tables::read_tables(doc09, BENCH_HEADING, &["NFR", "ベンチ ID"])
        .map_err(|e| format!("09 §5.2 の表を読めません: {e}"))?;
    let mut out = Vec::new();
    for t in &tables {
        let mut acs: Vec<String> = Vec::new();
        for row in &t.rows {
            let nfr = t.cell(row, "NFR").unwrap_or_default();
            if !nfr.is_empty() {
                acs = ac_refs(nfr);
            }
            for by in bench_ids(t.cell(row, "ベンチ ID").unwrap_or_default()) {
                for ac in &acs {
                    out.push(Ref {
                        ac: ac.clone(),
                        by: by.clone(),
                        at: format!("docs/09-testing-quality.md:{}", row.line),
                    });
                }
            }
        }
    }
    Ok(out)
}

/// 手順書の行に書いた AC と項目。表の行は先頭のセルの番号 (`G4`)、ほかの行はその節の記号 (`## P. …` の `P`)。
/// 記号の無い節 (`## 準備` など) の行は数えない。
fn checklist_refs(text: &str) -> Vec<Ref> {
    let mut out = Vec::new();
    let mut section = String::new();
    for (i, l) in text.lines().enumerate() {
        if let Some(h) = l.strip_prefix("## ") {
            h.split_once(". ")
                .map(|(s, _)| s)
                .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_uppercase()))
                .unwrap_or_default()
                .clone_into(&mut section);
            continue;
        }
        let first = l
            .strip_prefix('|')
            .and_then(|r| r.split('|').next())
            .map(str::trim)
            .unwrap_or_default();
        let is_item = first.bytes().next().is_some_and(|b| b.is_ascii_uppercase())
            && first.bytes().any(|b| b.is_ascii_digit())
            && first.bytes().all(|b| b.is_ascii_alphanumeric());
        let by = if is_item { first } else { section.as_str() };
        if by.is_empty() {
            continue;
        }
        for ac in ac_refs(l) {
            out.push(Ref {
                ac,
                by: by.to_owned(),
                at: format!("{CHECKLIST}:{}", i + 1),
            });
        }
    }
    out
}

/// 受入基準のセルの AC の頭 (`AC-<n>:`。間に括弧の注記を許す) の位置と番号と、直前に書かれた要件 ID
/// (`eve_docs_tables::ac::read_later_units` と同じ読み方)。
fn heads(cell: &str) -> Vec<(usize, Option<String>, u32)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(p) = cell[from..].find("AC-") {
        let at = from + p;
        from = at + 3;
        let Some((n, rest)) = digits(&cell[from..]) else {
            continue;
        };
        let mut rest = rest.trim_start();
        if rest.starts_with('(') {
            match rest.find(')') {
                Some(q) => rest = rest[q + 1..].trim_start(),
                None => continue,
            }
        }
        if !rest.starts_with([':', '：']) {
            continue;
        }
        let before = cell[..at].trim_end();
        let last = before.rsplit([' ', '。', '、', '(']).next().unwrap_or("");
        let req = req_prefix(last)
            .filter(|(_, r)| r.is_empty())
            .map(|(r, _)| r);
        out.push((at, req, n));
    }
    out
}

/// 01 §5.5 の AC のうち、後の版の AC に替わったもの (その AC の文に「に替わる」がある)。
fn replaced_acs(doc01: &str) -> Result<BTreeSet<String>, String> {
    let tables = eve_docs_tables::read_tables(
        doc01,
        eve_docs_tables::ac::LATER_HEADING,
        &["要件", "受入基準 (概要)"],
    )
    .map_err(|e| format!("01 §5.5 の表を読めません: {e}"))?;
    let mut out = BTreeSet::new();
    for t in &tables {
        for row in &t.rows {
            let req_cell = t.cell(row, "要件").unwrap_or_default();
            let first = req_cell
                .char_indices()
                .find_map(|(i, _)| req_prefix(&req_cell[i..]).map(|(r, _)| r));
            let cell = t.cell(row, "受入基準 (概要)").unwrap_or_default();
            let hs = heads(cell);
            for (k, (at, req, n)) in hs.iter().enumerate() {
                let end = hs.get(k + 1).map_or(cell.len(), |h| h.0);
                if cell[*at..end].contains("に替わる")
                    && let Some(r) = req.clone().or_else(|| first.clone())
                {
                    out.insert(format!("AC-{r}-{n}"));
                }
            }
        }
    }
    Ok(out)
}

/// 01 §5.5 のすべての行の AC ID (`check-ac-checklist` と共有する。後の版の AC の打ち間違いの検出)。
pub(crate) fn later_ac_ids(doc01: &str) -> Result<BTreeSet<String>, String> {
    Ok(eve_docs_tables::ac::read_later_units(doc01, "")
        .map_err(|e| format!("01 §5.5 の表を読めません: {e}"))?
        .into_iter()
        .filter_map(|u| u.ac)
        .collect())
}

struct Report {
    errors: Vec<String>,
    warnings: Vec<String>,
    matrix: String,
}

/// 1 つの単位を確かめるもの。
#[derive(Default)]
struct Cover<'a> {
    tests: BTreeSet<&'a str>,
    benches: BTreeSet<&'a str>,
    items: BTreeSet<&'a str>,
}

impl Cover<'_> {
    fn is_empty(&self) -> bool {
        self.tests.is_empty() && self.benches.is_empty() && self.items.is_empty()
    }
}

/// 行列のセル (`` `名前` `` を `<br>` で並べる。空なら `—`)。
fn cell(set: &BTreeSet<&str>) -> String {
    if set.is_empty() {
        "—".to_owned()
    } else {
        set.iter()
            .map(|t| format!("`{t}`"))
            .collect::<Vec<_>>()
            .join("<br>")
    }
}

/// `only` が `Some` なら、その版の行だけを警告する (行列はいつも [`VERSIONS`] のすべての版)。
fn build(root: &Path, only: Option<&str>) -> Result<Report, String> {
    let doc01 = read_text(&root.join("docs/01-requirements.md"))?;
    let doc09 = read_text(&root.join("docs/09-testing-quality.md"))?;
    let checklist = read_text(&root.join(CHECKLIST))?;
    let (_, ids) = scan(root)?;
    let known: BTreeSet<String> = ids.iter().map(|i| i.id.clone()).collect();
    let v01: BTreeSet<String> = eve_docs_tables::ac::read_acs(&doc01)
        .map_err(|e| format!("01 §5 の AC 表を読めません: {e}"))?
        .into_iter()
        .map(|a| a.id)
        .collect();
    // AC ID が実在するかは、§5.5 のすべての版の行で見る (行列に並べるのは VERSIONS の行だけ)
    let later_acs = later_ac_ids(&doc01)?;
    let replaced = replaced_acs(&doc01)?;
    let prio = priorities(&doc01);
    let (verifies, spec_rows, scan_errors) = scan_sources(root);
    let benches = bench_refs(&doc09)?;
    let items = checklist_refs(&checklist);

    let mut errors = scan_errors;
    for v in &verifies {
        let ok = if v.id.starts_with("AC-") {
            v01.contains(&v.id) || later_acs.contains(&v.id)
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
    for r in benches.iter().chain(&items) {
        if !v01.contains(&r.ac) && !later_acs.contains(&r.ac) {
            errors.push(format!("{}: AC の参照 `{}` は 01 にありません", r.at, r.ac));
        }
    }
    for (at, r) in &spec_rows {
        if !known.contains(r) {
            errors.push(format!(
                "{at}: `spec:` の規範行 ID `{r}` は索引にありません"
            ));
        }
    }

    // ID (AC ID か要件 ID) ごとの、確かめるもの
    let mut by_id: BTreeMap<&str, Cover> = BTreeMap::new();
    for v in verifies.iter().filter(|v| !v.test.is_empty()) {
        // ベンチのバイナリ全体 (`benches/` の `fn main`) はファイルで示す
        let name = if v.test == "main" && v.at.contains("/benches/") {
            v.at.rsplit_once(':').map_or(v.at.as_str(), |(f, _)| f)
        } else {
            v.test.as_str()
        };
        by_id.entry(v.id.as_str()).or_default().tests.insert(name);
    }
    for r in &benches {
        let c = by_id.entry(r.ac.as_str()).or_default();
        c.benches.insert(r.by.as_str());
    }
    for r in &items {
        let c = by_id.entry(r.ac.as_str()).or_default();
        c.items.insert(r.by.as_str());
    }

    let mut warnings = Vec::new();
    let mut m = String::new();
    m.push_str("# トレーサビリティ行列\n\n");
    let _ = writeln!(
        m,
        "生成物 (`cargo xtask trace` で作り直す。手で直さない。docs/11 §3.3、09 §8.3)。01 §5.5 の版の行 ({}) の受入基準ごとに、それを確かめる試験 (`// verifies:` の直後の関数とシナリオの `verifies`)・ベンチ (09 §5.2 の表の行に書いた AC)・手順書の項目 (`{CHECKLIST}` の行に書いた AC。番号の無い行は節の記号) を並べる。試験が要件 ID を指していれば、その要件の AC すべてを確かめたと数える。後の版の AC に替わった AC は、確かめるものが無くても警告しない。v0.1 の AC は `tests/ac-checklist.toml` を見る。\n",
        VERSIONS.join("・")
    );
    let mut versions: Vec<&str> = VERSIONS.to_vec();
    if let Some(v) = only.filter(|v| !VERSIONS.contains(v)) {
        versions.push(v);
    }
    for version in versions {
        let units = eve_docs_tables::ac::read_later_units(&doc01, version)
            .map_err(|e| format!("01 §5.5 の表を読めません: {e}"))?;
        let in_matrix = VERSIONS.contains(&version);
        if in_matrix {
            let _ = write!(
                m,
                "## {version}\n\n| 要件 | AC | 優先度 | 試験 | ベンチ | 手順書 |\n|---|---|---|---|---|---|\n"
            );
        }
        let (mut covered, mut total, mut gone) = (0usize, 0usize, 0usize);
        for u in &units {
            let p = prio.get(&u.requirement).map_or("—", String::as_str);
            let mut c = Cover::default();
            for id in std::iter::once(u.requirement.as_str()).chain(u.ac.as_deref()) {
                if let Some(x) = by_id.get(id) {
                    c.tests.extend(&x.tests);
                    c.benches.extend(&x.benches);
                    c.items.extend(&x.items);
                }
            }
            // 替わった AC は、要件 ID で数えた試験を並べない (確かめているのは替わった先の AC)
            let is_replaced = u.ac.as_ref().is_some_and(|a| replaced.contains(a));
            let tests = if is_replaced {
                "(後の版の AC に替わった)".to_owned()
            } else if !c.is_empty() {
                cell(&c.tests)
            } else {
                if p == "Must" && only.is_none_or(|o| o == version) {
                    warnings.push(format!(
                        "{version} {} {} (Must) を確かめる試験・ベンチ・手順書の項目がありません",
                        u.requirement,
                        u.ac.as_deref().unwrap_or("")
                    ));
                }
                "**なし**".to_owned()
            };
            if is_replaced {
                gone += 1;
            } else {
                total += 1;
                covered += usize::from(!c.is_empty());
            }
            if in_matrix {
                let _ = writeln!(
                    m,
                    "| {} | {} | {p} | {tests} | {} | {} |",
                    u.requirement,
                    u.ac.as_deref().unwrap_or("(番号なし)"),
                    cell(&c.benches),
                    cell(&c.items),
                );
            }
        }
        if in_matrix {
            let _ = write!(m, "\n確かめるもののある単位: {covered} / {total}");
            if gone > 0 {
                let _ = write!(m, " (後の版の AC に替わった {gone} 件を除く)");
            }
            m.push_str("\n\n");
        }
    }
    let matrix = format!("{}\n", m.trim_end());
    Ok(Report {
        errors,
        warnings,
        matrix,
    })
}

pub fn run(args: &[String]) -> Result<(), String> {
    let root = workspace_root();
    // 警告を絞る版 (01 §5.5 の行の版の注記)。無ければ行列のすべての版を警告する
    let version = args
        .iter()
        .position(|a| a == "--version")
        .and_then(|i| args.get(i + 1))
        .map(String::as_str);
    let r = build(&root, version)?;
    let check = args.iter().any(|a| a == "--check");
    println!(
        "▶ trace{}{}",
        if check { " --check" } else { "" },
        version
            .map(|v| format!(" --version {v}"))
            .unwrap_or_default()
    );
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

    /// リポジトリの `verifies:` / `spec:` が存在する ID を指し、行列が最新 (CI の検査と同じ)。
    // verifies: NFR-708
    #[test]
    fn リポジトリの対応は壊れておらず行列は最新() {
        run(&["--check".to_owned()]).unwrap();
    }

    /// v0.4 の判定の「要件充足」(09 §8.2 の v0.4 の表) の Must の AC に、確かめる試験・ベンチ・手順書の項目がある。
    // verifies: NFR-708
    #[test]
    fn v0_4_の_must_の_ac_に確かめるものがある() {
        let r = build(&workspace_root(), Some("v0.4")).unwrap();
        for ac in [
            "AC-NFR-303-5",
            "AC-NFR-303-6",
            "AC-NFR-303-7",
            "AC-NFR-304-4",
            "AC-NFR-304-5",
            "AC-NFR-304-6",
            "AC-FR-122-2",
            "AC-FR-122-3",
            "AC-FR-122-4",
        ] {
            let req = ac[3..].rsplit_once('-').unwrap().0;
            assert!(
                r.matrix.contains(&format!("| {req} | {ac} | Must |")),
                "行列に {ac} の行が無い"
            );
            assert!(
                !r.warnings.iter().any(|w| w.contains(&format!(" {ac} "))),
                "{ac} を確かめるものが無い: {:?}",
                r.warnings
            );
        }
    }

    #[test]
    fn 文の中の_ac_の参照を読む() {
        assert_eq!(
            ac_refs("(v0.4 の M25。先行表示、03 §9.2.1、FR-122 AC-2〜AC-4。BM-104v の手の確認)"),
            ["AC-FR-122-2", "AC-FR-122-3", "AC-FR-122-4"]
        );
        assert_eq!(
            ac_refs("NFR-106 長大行 (OQ-31。NFR-106 AC-1・AC-3 / AC-4)、AC-FR-513-4。FR-513 AC-4"),
            [
                "AC-NFR-106-1",
                "AC-NFR-106-3",
                "AC-NFR-106-4",
                "AC-FR-513-4"
            ]
        );
        // 要件 ID だけ・間に語のあるもの・ID の一部は数えない
        let none = ac_refs("NFR-604。FR-122 の AC-2。XFR-1 AC-1。AC-3: 文");
        assert!(none.is_empty(), "{none:?}");
    }

    #[test]
    fn ベンチ_id_を読む() {
        assert_eq!(
            bench_ids("BM-104v-100m / BM-104v-100m-sjis (M20)、BM-104-{1m,10m,100m}。BM-"),
            ["BM-104v-100m", "BM-104v-100m-sjis", "BM-104-{1m,10m,100m}"]
        );
    }

    #[test]
    fn ベンチの表と手順書から_ac_の参照を集める() {
        let doc09 = "### 5.2 NFR とベンチ ID の対応\n\n| NFR | ベンチ ID | 測り方 |\n|---|---|---|\n\
            | NFR-104 の先行表示 (FR-122 AC-2) | BM-104v-100m / BM-104v-100m-sjis | x |\n\
            | | BM-104v-extra | y |\n| NFR-105 | BM-105 | z |\n\n## 6. 次\n";
        let got: Vec<String> = bench_refs(doc09)
            .unwrap()
            .into_iter()
            .map(|r| format!("{} {}", r.ac, r.by))
            .collect();
        assert_eq!(
            got,
            [
                "AC-FR-122-2 BM-104v-100m",
                "AC-FR-122-2 BM-104v-100m-sjis",
                "AC-FR-122-2 BM-104v-extra",
            ]
        );

        let list = "# 手順書\n## 準備\nFR-101 AC-1 は数えない\n## G. 巨大\n\n| # | 手順 | 期待 |\n|---|---|---|\n\
            | G4 | (FR-122 AC-2〜AC-3) 開く | 出る |\n## P. 高コントラスト\n\nNFR-604 AC-2 の M。\n\
            | P1 | 見る | 見える |\n";
        let got: Vec<String> = checklist_refs(list)
            .into_iter()
            .map(|r| format!("{} {} {}", r.ac, r.by, r.at))
            .collect();
        assert_eq!(
            got,
            [
                "AC-FR-122-2 G4 docs/compat/checklist.md:8",
                "AC-FR-122-3 G4 docs/compat/checklist.md:8",
                "AC-NFR-604-2 P docs/compat/checklist.md:11",
            ]
        );
    }

    #[test]
    fn 後の版の_ac_に替わった_ac_を読む() {
        let doc = "### 5.5 v0.2 以降 (概要。各版の着手前に詳細化)\n\n| 要件 | 受入基準 (概要) |\n|---|---|\n\
            | FR-402 正規表現 (v0.2) | AC-1: a。AC-3: b (v0.4 は下の FR-402 複数行の AC-5〜AC-9 に替わる) |\n\
            | FR-103 / FR-101 (v0.2) | FR-101 AC-1: c (v0.4 で FR-101 AC-2 (x) に替わる)。FR-103 AC-2: d |\n\
            | NFR-106 (v0.4) | AC-5: e |\n";
        let got: Vec<String> = replaced_acs(doc).unwrap().into_iter().collect();
        assert_eq!(got, ["AC-FR-101-1", "AC-FR-402-3"]);
    }

    #[test]
    fn 行列は版ごとに節を分け_替わった_ac_は警告しない() {
        let r = build(&workspace_root(), None).unwrap();
        for v in VERSIONS {
            assert!(r.matrix.contains(&format!("\n## {v}\n")), "{v} の節が無い");
        }
        // 01 §5.5 の FR-402 AC-3 (v0.2。`\n` は未対応) は、v0.4 の AC-5〜AC-9 に替わった
        assert!(
            r.matrix
                .contains("| FR-402 | AC-FR-402-3 | Must | (後の版の AC に替わった) |"),
            "{}",
            r.matrix
        );
        assert!(
            !r.warnings.iter().any(|w| w.contains("AC-FR-402-3")),
            "{:?}",
            r.warnings
        );
    }
}
