//! `xtask check-docs`: 仕様書の参照の検査 (docs/09 §11、11 §2.3、NFR-708)。
//!
//! (1) 本文の ID 参照 (`FR-105`・`NFR-106`・`ADR-0040`・`OQ-31`・`BM-106`) が定義されていること (索引、BM は 09 §5.2)。
//! (2) 本文の節の参照 (`03 §9.3`) が索引にあること (文書番号の無い `§9.3` は数えない)。
//! (3) 内部リンク (`[..](04-tui-design.md#…)`・`(#…)`) の先のファイルと見出しがあること (見出しは GitHub の規則でアンカーにする)。
//! (4) `docs/ownership.toml` の `path` が存在し、`spec` の節が索引にあること。
//!
//! コードフェンスとインライン コード (`` `…` ``) の中は例として扱い、検査しない。

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Deserialize;

use crate::spec_index::{doc_files, scan, section_refs_in};
use crate::util::{read_text, workspace_root};

/// インライン コードを空白に置き換える (位置は保たない)。
fn strip_code(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_code = false;
    for c in line.chars() {
        if c == '`' {
            in_code = !in_code;
            out.push(' ');
        } else if !in_code {
            out.push(c);
        }
    }
    out
}

/// 本文から ID 参照を取り出す (前後が英数字・`-` でない語)。
fn id_refs(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    for prefix in ["NFR-", "FR-", "ADR-", "OQ-", "BM-"] {
        let mut from = 0;
        while let Some(p) = text[from..].find(prefix) {
            let at = from + p;
            from = at + prefix.len();
            let before_ok = at == 0 || {
                let b = bytes[at - 1];
                !(b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            };
            // `NFR-` の中の `FR-` は数えない (上の条件で除かれる)
            if !before_ok {
                continue;
            }
            let rest = &text[from..];
            let tail: String = if prefix == "BM-" {
                rest.chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                    .collect::<String>()
                    .trim_end_matches('-')
                    .to_owned()
            } else {
                let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
                // `FR-1xx` のような分類名は参照ではない
                if rest[digits.len()..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphanumeric())
                {
                    continue;
                }
                digits
            };
            if tail.is_empty() || !tail.as_bytes()[0].is_ascii_alphanumeric() {
                continue;
            }
            out.push(format!("{prefix}{tail}"));
        }
    }
    out
}

/// GitHub の見出しのアンカー (小文字、英数字・`-`・`_`・空白以外を除き、空白を `-` に)。
fn anchor_of(title: &str) -> String {
    let mut out = String::new();
    for c in strip_code_marks(title).chars() {
        if c.is_alphanumeric() || c == '-' || c == '_' {
            out.extend(c.to_lowercase());
        } else if c == ' ' {
            out.push('-');
        }
    }
    out
}

/// 見出しの表示の文字列 (`` ` `` を除き、`[文字](先)` を文字だけにする)。
fn strip_code_marks(s: &str) -> String {
    let s = s.replace('`', "");
    let mut out = String::new();
    let mut rest = s.as_str();
    while let Some(p) = rest.find('[') {
        out.push_str(&rest[..p]);
        let after = &rest[p + 1..];
        match after.split_once("](") {
            Some((text, tail)) if !text.contains('[') => {
                out.push_str(text);
                rest = tail.split_once(')').map_or("", |(_, t)| t);
            }
            _ => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// OQ の番号は `OQ-07` と `OQ-7` を同じものとして比べる。
fn normalize(id: &str) -> String {
    match id.strip_prefix("OQ-") {
        Some(n) if n.bytes().all(|b| b.is_ascii_digit()) => {
            format!("OQ-{}", n.trim_start_matches('0'))
        }
        _ => id.to_owned(),
    }
}

/// 文書の見出しのアンカーの集合 (重複は `-1`、`-2` … を付ける)。
fn anchors(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut count: BTreeMap<String, usize> = BTreeMap::new();
    let mut in_fence = false;
    for l in text.lines() {
        if l.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let level = l.bytes().take_while(|&b| b == b'#').count();
        if level == 0 || l.as_bytes().get(level) != Some(&b' ') {
            continue;
        }
        let a = anchor_of(l[level + 1..].trim());
        let n = count.entry(a.clone()).or_insert(0);
        out.insert(if *n == 0 {
            a.clone()
        } else {
            format!("{a}-{n}")
        });
        *n += 1;
    }
    out
}

/// 09 §5.2 (NFR とベンチ ID の対応。表と、実装上の ID の箇条書き) に出る BM の ID (定義の側)。`BM-104-{1m,10m,100m}` は `BM-104` も定義する。
fn bm_defined(doc09: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut inside = false;
    for l in doc09.lines() {
        if l.starts_with("### ") {
            inside = l.starts_with("### 5.2 ");
            continue;
        }
        if inside {
            for r in id_refs(&l.replace('`', " ")) {
                if r.starts_with("BM-") {
                    out.insert(r);
                }
            }
        }
    }
    out
}

/// Markdown のリンクの先 (`](…)` の中身)。
fn link_targets(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut s = line;
    while let Some(p) = s.find("](") {
        let after = &s[p + 2..];
        let Some(q) = after.find(')') else { break };
        out.push(after[..q].trim().to_owned());
        s = &after[q..];
    }
    out
}

#[derive(Debug, Deserialize)]
struct Ownership {
    version: u32,
    #[serde(default)]
    unit: Vec<Unit>,
}

#[derive(Debug, Deserialize)]
struct Unit {
    path: String,
    #[serde(default)]
    spec: Vec<String>,
    owner: String,
}

/// `"02 §5.1-5.2"` のような範囲は、両端の節があることを確かめる。`"04"` のような文書全体の指定は文書の存在だけ。
fn spec_ok(spec: &str, sections: &BTreeSet<String>, docs: &BTreeSet<String>) -> bool {
    match spec.split_once(" §") {
        None => docs.contains(spec),
        Some((doc, range)) => range.split(['-', '–']).all(|n| {
            let n = n.trim();
            sections.contains(&format!("{doc} §{n}"))
        }),
    }
}

pub fn problems(root: &Path) -> Result<Vec<String>, String> {
    let (sections, ids) = scan(root)?;
    let known_sections: BTreeSet<String> = sections.iter().map(|s| s.id.clone()).collect();
    let mut known_ids: BTreeSet<String> = ids.iter().map(|i| normalize(&i.id)).collect();
    known_ids.extend(bm_defined(&read_text(
        &root.join("docs/09-testing-quality.md"),
    )?));
    let files = doc_files(root)?;
    let doc_numbers: BTreeSet<String> = files.iter().map(|(_, d)| d.clone()).collect();
    let mut texts = BTreeMap::new();
    for (name, _) in &files {
        texts.insert(name.clone(), read_text(&root.join("docs").join(name))?);
    }
    let anchor_sets: BTreeMap<&str, BTreeSet<String>> = texts
        .iter()
        .map(|(n, t)| (n.as_str(), anchors(t)))
        .collect();

    let mut out = Vec::new();
    for (name, text) in &texts {
        let mut in_fence = false;
        for (i, raw) in text.lines().enumerate() {
            if raw.trim_start().starts_with("```") {
                in_fence = !in_fence;
                continue;
            }
            if in_fence {
                continue;
            }
            let at = format!("docs/{name}:{}", i + 1);
            let line = strip_code(raw);
            // 見出しの ID は範囲を示すことがある (`OQ-161 以降`)。定義は索引が拾う
            let heading = raw.starts_with('#');
            // 欠番を述べる行 (`ADR-0024 / 0029 は欠番`) の参照は数えない
            let numbering_note = line.contains("欠番");
            for r in id_refs(&line) {
                // `BM-1xx` は番号の総称。`BM-101` は `BM-101a` / `BM-101b` の総称として受ける
                let family = r.starts_with("BM-")
                    && (r.contains("xx")
                        || known_ids.range(r.clone()..).next().is_some_and(|k| {
                            k.strip_prefix(r.as_str())
                                .is_some_and(|t| t.starts_with(|c: char| c.is_ascii_lowercase()))
                        }));
                if !numbering_note && !heading && !family && !known_ids.contains(&normalize(&r)) {
                    out.push(format!("{at}: `{r}` は定義されていません"));
                }
            }
            for r in section_refs_in(&line) {
                if !known_sections.contains(&r) {
                    out.push(format!("{at}: 節 `{r}` は索引にありません"));
                }
            }
            for t in link_targets(raw) {
                if t.contains("://") || t.starts_with("mailto:") {
                    continue;
                }
                let (file, anchor) = t.split_once('#').unwrap_or((&t, ""));
                let target = if file.is_empty() { name.as_str() } else { file };
                if !file.is_empty() && !root.join("docs").join(file).exists() {
                    out.push(format!("{at}: リンク先 `{file}` がありません"));
                    continue;
                }
                if anchor.is_empty() {
                    continue;
                }
                match anchor_sets.get(target) {
                    Some(set) if !set.contains(&anchor.to_lowercase()) => {
                        out.push(format!(
                            "{at}: リンク先 `{target}#{anchor}` の見出しがありません"
                        ));
                    }
                    _ => {}
                }
            }
        }
    }

    // (4) ownership.toml
    let own_path = root.join("docs/ownership.toml");
    let own: Ownership = toml::from_str(&read_text(&own_path)?)
        .map_err(|e| format!("docs/ownership.toml を読めません: {e}"))?;
    if own.version != 1 {
        out.push(format!(
            "docs/ownership.toml: version = {} は知らない版です",
            own.version
        ));
    }
    let mut paths = BTreeSet::new();
    for u in &own.unit {
        if !paths.insert(u.path.as_str()) {
            out.push(format!(
                "docs/ownership.toml: path `{}` が重複しています",
                u.path
            ));
        }
        if !root.join(&u.path).exists() {
            out.push(format!(
                "docs/ownership.toml: path `{}` がありません",
                u.path
            ));
        }
        if !(u.owner == "shared" || u.owner.starts_with("human:") || u.owner.starts_with("agent:"))
        {
            out.push(format!(
                "docs/ownership.toml: `{}` の owner `{}` の形が違います",
                u.path, u.owner
            ));
        }
        for s in &u.spec {
            if !spec_ok(s, &known_sections, &doc_numbers) {
                out.push(format!(
                    "docs/ownership.toml: `{}` の spec `{s}` は索引にありません",
                    u.path
                ));
            }
        }
    }
    Ok(out)
}

pub fn run() -> Result<(), String> {
    println!("▶ check-docs");
    let errs = problems(&workspace_root())?;
    if errs.is_empty() {
        println!("  OK");
        Ok(())
    } else {
        Err(format!(
            "check-docs に {} 件の問題があります\n  - {}",
            errs.len(),
            errs.join("\n  - ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_参照を取り出す() {
        assert_eq!(
            id_refs("FR-105 と NFR-106、ADR-0040 / OQ-31。BM-103b-reconcile と XFR-1"),
            [
                "NFR-106",
                "FR-105",
                "ADR-0040",
                "OQ-31",
                "BM-103b-reconcile"
            ]
        );
        assert_eq!(strip_code("a `FR-1` b"), "a    b");
    }

    #[test]
    fn 見出しのアンカー() {
        assert_eq!(
            anchor_of("4.6 eve-core のモジュール所有境界"),
            "46-eve-core-のモジュール所有境界"
        );
        assert_eq!(
            anchor_of("ADR-0040: 更新の方針 (エディタ内に更新機能を持たない)"),
            "adr-0040-更新の方針-エディタ内に更新機能を持たない"
        );
        assert_eq!(
            anchor_of("3.3 `xtask spec-index` と `xtask trace`"),
            "33-xtask-spec-index-と-xtask-trace"
        );
        let a = anchors("## 例\n## 例\n```\n## 例\n```\n");
        assert!(a.contains("例") && a.contains("例-1") && !a.contains("例-2"));
    }

    /// リポジトリの仕様書に壊れた参照が無い (CI の `check-docs` と同じ検査)。
    // verifies: NFR-708
    #[test]
    fn リポジトリの仕様書の参照は壊れていない() {
        let errs = problems(&workspace_root()).unwrap();
        assert!(errs.is_empty(), "{}", errs.join("\n"));
    }
}
