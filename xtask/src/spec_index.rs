//! `xtask spec-index`: 仕様書の索引 `docs/spec-index.toml` の生成と検査 (docs/11 §3.3、02 §17.2、NFR-708)。
//!
//! - 節: `docs/*.md` の見出し (`## 1.` `### 6.1` …) → 節 ID (`03 §9.3`、README は `README §3.8`)・行範囲・題名。
//! - ID: 規範行 ID (`<文書>-<節>-<連番>`。表の先頭列・箇条書きの先頭の `**…**`・行末の `⟨…⟩`、11 §3.1 ID-5)、
//!   表の先頭列の既存の ID (FR / NFR / K1 / INV-F1 / R1 …、ID-1)、10 の ADR / OQ の見出し。
//!   同じ ID が同じ文書の表に何度も出るときは最初の 1 か所だけを載せる (要件の表と AC の表の両方に出る FR など)。
//!
//! `--check`: 生成物との差分、規範行 ID と ADR / OQ の重複、ソースの `spec:` が指す節の存在。`--show <ID>`: 該当の本文。

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use crate::util::{read_text, rust_files, workspace_root};

const INDEX: &str = "docs/spec-index.toml";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Section {
    pub(crate) id: String,
    pub(crate) file: String,
    pub(crate) line: usize,
    pub(crate) end: usize,
    level: usize,
    pub(crate) title: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Id {
    pub(crate) id: String,
    pub(crate) file: String,
    pub(crate) line: usize,
    pub(crate) section: String,
    /// 一意でなければならない種類 (規範行 ID・ADR・OQ)。
    unique: bool,
}

/// 文書のファイル名から文書番号 (`03-core-design.md` → `03`、`README.md` → `README`)。対象外は `None`。
fn doc_number(name: &str) -> Option<String> {
    if name == "README.md" {
        return Some("README".into());
    }
    let (n, rest) = name.split_once('-')?;
    (n.len() == 2 && n.bytes().all(|b| b.is_ascii_digit()) && rest.ends_with(".md"))
        .then(|| n.to_owned())
}

/// 見出しの節番号 (`6.1 題名` → `6.1`、`1. 題名` → `1`)。番号の無い見出しは `None`。
fn section_number(text: &str) -> Option<(String, String)> {
    let (num, title) = text.split_once(' ')?;
    let num = num.trim_end_matches('.');
    (!num.is_empty()
        && num
            .split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())))
    .then(|| (num.to_owned(), title.trim().to_owned()))
}

/// 規範行 ID の形 (`04-4.2-3`、`03-8.6-2`、README は対象外)。
fn is_row_id(s: &str) -> bool {
    let mut it = s.split('-');
    let (Some(d), Some(sec), Some(n), None) = (it.next(), it.next(), it.next(), it.next()) else {
        return false;
    };
    d.len() == 2
        && d.bytes().all(|b| b.is_ascii_digit())
        && !sec.is_empty()
        && sec
            .split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        && !n.is_empty()
        && n.bytes().all(|b| b.is_ascii_digit())
}

/// 表の先頭列に置く既存の ID の形 (英大文字で始まり、英大文字・数字・`-` だけで、数字を含む。`FR-105`・`K1`・`INV-F1`)。
fn is_plain_id(s: &str) -> bool {
    s.len() >= 2
        && s.as_bytes()[0].is_ascii_uppercase()
        && s.bytes().any(|b| b.is_ascii_digit())
        && s.bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')
        && !s.ends_with('-')
        && !s.contains("--")
}

fn strip_marks(s: &str) -> &str {
    s.trim()
        .trim_start_matches("**")
        .trim_end_matches("**")
        .trim_matches('`')
        .trim()
}

/// 1 文書を走査する。
fn scan_doc(file: &str, doc: &str, text: &str, sections: &mut Vec<Section>, ids: &mut Vec<Id>) {
    let lines: Vec<&str> = text.lines().collect();
    let first_section = sections.len();
    let mut in_fence = false;
    let mut current = String::new();
    let mut seen_plain: BTreeSet<String> = BTreeSet::new();
    // 10 の「OQ」「ADR」の列で始まる表の中か (番号だけの行 `| 117 |`・`| [0004](#…) |` を `OQ-117`・`ADR-0004` として拾う)
    let mut table_prefix: Option<&'static str> = None;
    let push = |ids: &mut Vec<Id>, id: &str, line: usize, section: &str, unique: bool| {
        ids.push(Id {
            id: id.to_owned(),
            file: file.to_owned(),
            line,
            section: section.to_owned(),
            unique,
        });
    };
    for (i, raw) in lines.iter().enumerate() {
        let ln = i + 1;
        let l = raw.trim_end();
        if l.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        // 見出し
        let level = l.bytes().take_while(|&b| b == b'#').count();
        if level >= 2 && l.as_bytes().get(level) == Some(&b' ') {
            let text = &l[level + 1..];
            if let Some((num, title)) = section_number(text) {
                current = format!("{doc} §{num}");
                sections.push(Section {
                    id: current.clone(),
                    file: file.to_owned(),
                    line: ln,
                    end: lines.len(),
                    level,
                    title,
                });
            }
            // 10 の ADR / OQ の見出し (`### ADR-0040 …`・`### OQ-31 …`)
            if doc == "10" {
                let first = text.split([' ', ':']).next().unwrap_or("");
                if (first.starts_with("ADR-") || first.starts_with("OQ-")) && is_plain_id(first) {
                    push(ids, first, ln, &current, true);
                }
            }
            continue;
        }
        // 表の先頭列
        let row = l.trim_start().strip_prefix('|');
        if row.is_none() {
            table_prefix = None;
        }
        if let Some(rest) = row {
            let cell = strip_marks(rest.split('|').next().unwrap_or(""));
            match cell {
                "OQ" => table_prefix = Some("OQ"),
                "ADR" => table_prefix = Some("ADR"),
                "OQ / ADR" => table_prefix = Some("OQ / ADR"),
                _ => {}
            }
            let num = cell
                .strip_prefix('[')
                .and_then(|c| c.split_once(']'))
                .map_or(cell, |(n, _)| n);
            if let Some(pre) = table_prefix
                && !num.is_empty()
                && num.bytes().all(|b| b.is_ascii_digit())
            {
                let pre = match pre {
                    "OQ / ADR" if num.len() == 4 => "ADR",
                    "OQ / ADR" => "OQ",
                    p => p,
                };
                let id = format!("{pre}-{num}");
                if seen_plain.insert(id.clone()) {
                    push(ids, &id, ln, &current, false);
                }
            } else if is_row_id(cell) {
                push(ids, cell, ln, &current, true);
            } else if is_plain_id(cell) && seen_plain.insert(cell.to_owned()) {
                push(ids, cell, ln, &current, false);
            }
        }
        // 箇条書きの先頭の `**04-4.2-3**`
        if let Some(rest) = l.trim_start().strip_prefix("- **")
            && let Some((cand, _)) = rest.split_once("**")
            && is_row_id(cand)
        {
            push(ids, cand, ln, &current, true);
        }
        // 行末などの `⟨04-4.2-3⟩`
        let mut s = l;
        while let Some(p) = s.find('⟨') {
            let after = &s[p + '⟨'.len_utf8()..];
            let Some(q) = after.find('⟩') else { break };
            let cand = &after[..q];
            // コード表記の中の例 (`` `⟨04-4.2-3⟩` ``) は数えない
            let in_code = s[..p].ends_with('`');
            if is_row_id(cand) && !in_code {
                push(ids, cand, ln, &current, true);
            }
            s = &after[q..];
        }
    }
    // 節の終わり: 次の同じ水準以上の見出しの直前
    for k in first_section..sections.len() {
        let lv = sections[k].level;
        if let Some(next) = sections[k + 1..].iter().find(|s| s.level <= lv) {
            sections[k].end = next.line - 1;
        }
    }
}

pub(crate) fn doc_files(root: &Path) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    let dir = root.join("docs");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| doc_number(n).is_some())
        .collect();
    // 00〜12、README の順
    names.sort_by_key(|n| (n == "README.md", n.clone()));
    for n in names {
        out.push((n.clone(), doc_number(&n).unwrap_or_default()));
    }
    Ok(out)
}

pub(crate) fn scan(root: &Path) -> Result<(Vec<Section>, Vec<Id>), String> {
    let mut sections = Vec::new();
    let mut ids = Vec::new();
    for (name, doc) in doc_files(root)? {
        let text = read_text(&root.join("docs").join(&name))?;
        scan_doc(
            &format!("docs/{name}"),
            &doc,
            &text,
            &mut sections,
            &mut ids,
        );
    }
    Ok((sections, ids))
}

fn q(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04X}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn render(sections: &[Section], ids: &[Id]) -> String {
    let mut o = String::new();
    o.push_str("# 仕様書の索引 (生成物。手で直さない: `cargo xtask spec-index` で作り直す。docs/11 §3.3)。\n");
    o.push_str("# [[section]] は節 ID → ファイルと行範囲、[[id]] は規範行 ID・既存の ID → ファイルと行。\n");
    o.push_str(
        "# 読むときは `cargo xtask spec-index --show <ID>` (例: --show \"03 §9.3\"、--show K1)。\n",
    );
    for s in sections {
        let _ = write!(
            o,
            "\n[[section]]\nid = {}\nfile = {}\nline = {}\nend = {}\ntitle = {}\n",
            q(&s.id),
            q(&s.file),
            s.line,
            s.end,
            q(&s.title)
        );
    }
    for i in ids {
        let _ = write!(
            o,
            "\n[[id]]\nid = {}\nfile = {}\nline = {}\nsection = {}\n",
            q(&i.id),
            q(&i.file),
            i.line,
            q(&i.section)
        );
    }
    o
}

/// ソースの `spec:` の行が指す節 ID (`03 §9.3`) を集める ((ファイル:行, 節 ID))。
fn spec_refs(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
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
                let Some(p) = t.find("spec: ") else { continue };
                // コメントの `spec:` だけ (`fn f(spec: …)` の引数は除く)
                if !(t.starts_with("//")
                    && t[..p]
                        .trim_start_matches('/')
                        .trim_start_matches('!')
                        .trim()
                        .is_empty())
                {
                    continue;
                }
                for r in section_refs_in(&t[p + 6..]) {
                    out.push((format!("{rel}:{}", i + 1), r));
                }
            }
        }
    }
    out
}

/// 文字列中の `NN §a.b` (と `README §a.b`) を取り出す。`/ §6.2` のような文書番号の無い略記は数えない。
pub(crate) fn section_refs_in(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(p) = rest.find(" §") {
        let before = &rest[..p];
        let doc: String = before
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let after = &rest[p + " §".len()..];
        let num: String = after
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        let num = num.trim_end_matches('.');
        let doc_ok = doc == "README" || (doc.len() == 2 && doc.bytes().all(|b| b.is_ascii_digit()));
        if doc_ok && !num.is_empty() {
            out.push(format!("{doc} §{num}"));
        }
        rest = after;
    }
    out
}

fn show(sections: &[Section], ids: &[Id], root: &Path, key: &str) -> Result<(), String> {
    let mut hits: Vec<(String, usize, usize)> = Vec::new();
    if let Some(s) = sections.iter().find(|s| s.id == key) {
        hits.push((s.file.clone(), s.line, s.end));
    }
    for i in ids.iter().filter(|i| i.id == key) {
        let end = sections
            .iter()
            .find(|s| s.id == i.section && s.file == i.file)
            .map_or(i.line + 20, |s| s.end.min(i.line + 20));
        hits.push((i.file.clone(), i.line, end.max(i.line)));
    }
    if hits.is_empty() {
        return Err(format!("索引に `{key}` がありません"));
    }
    for (file, a, b) in hits {
        let text = read_text(&root.join(&file))?;
        println!("── {file}:{a}-{b}");
        for (n, l) in text.lines().enumerate().take(b).skip(a - 1) {
            println!("{:>5}  {l}", n + 1);
        }
    }
    Ok(())
}

/// 検査 (生成物との差分を除く)。問題の一覧を返す。
fn problems(sections: &[Section], ids: &[Id], refs: &[(String, String)]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen: BTreeMap<&str, &Id> = BTreeMap::new();
    for i in ids.iter().filter(|i| i.unique) {
        if let Some(prev) = seen.insert(&i.id, i) {
            out.push(format!(
                "ID `{}` が重複しています ({}:{} と {}:{})",
                i.id, prev.file, prev.line, i.file, i.line
            ));
        }
    }
    let mut sec_seen: BTreeMap<&str, &Section> = BTreeMap::new();
    for s in sections {
        if let Some(prev) = sec_seen.insert(&s.id, s) {
            out.push(format!(
                "節 ID `{}` が重複しています ({}:{} と {}:{})",
                s.id, prev.file, prev.line, s.file, s.line
            ));
        }
    }
    let known: BTreeSet<&str> = sections.iter().map(|s| s.id.as_str()).collect();
    for (at, r) in refs {
        if !known.contains(r.as_str()) {
            out.push(format!("{at}: `spec:` の `{r}` は索引にありません"));
        }
    }
    out
}

pub fn run(args: &[String]) -> Result<(), String> {
    let root = workspace_root();
    let (sections, ids) = scan(&root)?;
    let mut it = args.iter();
    match it.next().map(String::as_str) {
        Some("--show") => {
            let key = it.next().ok_or("--show には ID か節 ID を渡してください")?;
            show(&sections, &ids, &root, key)
        }
        Some("--check") => {
            println!("▶ spec-index --check");
            let mut errs = problems(&sections, &ids, &spec_refs(&root));
            let want = render(&sections, &ids);
            let have = std::fs::read_to_string(root.join(INDEX)).unwrap_or_default();
            if have.replace("\r\n", "\n") != want {
                errs.push(format!(
                    "{INDEX} が古い: `cargo xtask spec-index` で作り直してコミットしてください"
                ));
            }
            if errs.is_empty() {
                println!("  節 {} 件・ID {} 件。OK", sections.len(), ids.len());
                Ok(())
            } else {
                Err(format!(
                    "spec-index に {} 件の問題があります\n  - {}",
                    errs.len(),
                    errs.join("\n  - ")
                ))
            }
        }
        None => {
            let errs = problems(&sections, &ids, &[]);
            std::fs::write(root.join(INDEX), render(&sections, &ids))
                .map_err(|e| format!("{INDEX}: {e}"))?;
            println!(
                "{INDEX} を書きました (節 {} 件・ID {} 件)",
                sections.len(),
                ids.len()
            );
            if errs.is_empty() {
                Ok(())
            } else {
                Err(format!("  - {}", errs.join("\n  - ")))
            }
        }
        Some(a) => Err(format!(
            "spec-index: 知らない引数 `{a}` (--check / --show <ID>)"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 見出しと_id_を拾う() {
        let text = "\
# 題
## 1. 範囲
| ID | 内容 |
|---|---|
| K1 | 規範 |
| **FR-105** | 要件 |
| FR-105 | 2 回目は載せない |
| 04-1-1 | 規範行 |
- **04-1-2** 箇条書き
本文 ⟨04-1-3⟩ と例 `⟨04-1-9⟩`
```
## 2. フェンスの中は見出しではない
| K9 | フェンスの中 |
```
### 1.1 小節
## 2. 次
";
        let (mut s, mut ids) = (Vec::new(), Vec::new());
        scan_doc("docs/04-x.md", "04", text, &mut s, &mut ids);
        let secs: Vec<_> = s.iter().map(|x| (x.id.as_str(), x.line, x.end)).collect();
        assert_eq!(
            secs,
            [("04 §1", 2, 15), ("04 §1.1", 15, 15), ("04 §2", 16, 16)]
        );
        let got: Vec<_> = ids
            .iter()
            .map(|i| (i.id.as_str(), i.line, i.unique))
            .collect();
        assert_eq!(
            got,
            [
                ("K1", 5, false),
                ("FR-105", 6, false),
                ("04-1-1", 8, true),
                ("04-1-2", 9, true),
                ("04-1-3", 10, true),
            ]
        );
    }

    #[test]
    fn spec_の節の参照を取り出す() {
        assert_eq!(
            section_refs_in("03 §6.1 / §6.2 (from_sorted_edits) と README §3.8、11 §3。"),
            ["03 §6.1", "README §3.8", "11 §3"]
        );
    }

    #[test]
    fn id_の形() {
        assert!(is_row_id("04-4.2-3") && is_row_id("03-11.5-1"));
        assert!(!is_row_id("4-4.2-3") && !is_row_id("04-4.2") && !is_row_id("AC-FR-1"));
        assert!(is_plain_id("K1") && is_plain_id("INV-F1") && is_plain_id("NFR-106"));
        assert!(!is_plain_id("ID") && !is_plain_id("v0.2") && !is_plain_id("A-"));
    }

    /// リポジトリの索引が最新で、問題が無い (CI の `spec-index` と同じ検査)。
    // verifies: NFR-708
    #[test]
    fn リポジトリの索引は最新() {
        run(&["--check".to_owned()]).unwrap();
    }
}
