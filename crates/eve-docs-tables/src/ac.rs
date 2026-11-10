//! 受入基準 (AC) 表の読み取り (docs/11 §8.5「AC の抽出規則」)。
//!
//! 01 §5.1〜§5.4 の表から、`要件` 列の要件 ID と、`受入基準` 列のうち `AC-<n>:` で始まる部分を取り出し、
//! `AC-<要件 ID>-<n>` (例 `AC-FR-105-1`) の一覧にする。

use crate::{Error, read_tables, strip_parentheticals};

/// 01 の AC 表の見出し (固定)。
pub const AC_HEADINGS: [&str; 4] = [
    "### 5.1 v0.1: ファイル・文書管理",
    "### 5.2 v0.1: 編集・移動・検索",
    "### 5.3 v0.1: 表示・入力・設定",
    "### 5.4 v0.1: 非機能",
];

/// v0.1 の AC 1 件。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ac {
    /// `AC-FR-105-1` の形。
    pub id: String,
    /// `FR-105` の形。
    pub requirement: String,
    pub n: u32,
}

/// `要件` 列のセルから要件 ID を取り出す。括弧の注記は読まず、`〜` の範囲は展開する。
/// 例: `NFR-101〜105` → `NFR-101` … `NFR-105`、`FR-201〜204, 206` → `FR-201` … `FR-204`、`FR-206`。
pub fn requirement_ids(cell: &str) -> Vec<String> {
    let cell = strip_parentheticals(cell);
    let mut ids = Vec::new();
    let mut prefix: Option<String> = None;
    for tok in cell.split([',', '、']) {
        let tok = tok.trim();
        if tok.is_empty() {
            continue;
        }
        let (head, rest) = match tok.split_once('〜').or_else(|| tok.split_once('~')) {
            Some((a, b)) => (a.trim(), Some(b.trim())),
            None => (tok, None),
        };
        // 先頭が `FR-123` / `NFR-123` なら接頭辞を更新し、数字だけなら直前の接頭辞を引き継ぐ
        let (pre, num) = if let Some(n) = head.strip_prefix("NFR-") {
            (Some("NFR".to_owned()), n)
        } else if let Some(n) = head.strip_prefix("FR-") {
            (Some("FR".to_owned()), n)
        } else {
            (prefix.clone(), head)
        };
        let (Some(pre), Ok(start)) = (pre, num.trim().parse::<u32>()) else {
            continue;
        };
        let end = rest.and_then(|r| r.parse::<u32>().ok()).unwrap_or(start);
        for n in start..=end {
            ids.push(format!("{pre}-{n:03}"));
        }
        prefix = Some(pre);
    }
    ids
}

/// セルの中の AC の頭部。`AC-<n>` に、括弧の注記、`:` が続くものだけを AC の始まりとして数える。
struct Head {
    n: u32,
    note: String,
}

fn heads(cell: &str) -> Vec<Head> {
    let bytes = cell.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(off) = cell[i..].find("AC-") {
        let start = i + off;
        let mut j = start + 3;
        let digits = bytes[j..].iter().take_while(|b| b.is_ascii_digit()).count();
        if digits == 0 {
            i = j;
            continue;
        }
        let n: u32 = cell[j..j + digits].parse().unwrap_or(0);
        j += digits;
        // 空白、括弧の注記 (入れ子を許す)、空白、`:` の順
        let skip_ws = |mut k: usize| {
            while k < cell.len() && cell[k..].starts_with([' ', '\t']) {
                k += cell[k..].chars().next().map_or(1, char::len_utf8);
            }
            k
        };
        let mut k = skip_ws(j);
        let mut note = String::new();
        if let Some(c) = cell[k..].chars().next().filter(|c| matches!(c, '(' | '（')) {
            let mut depth = 0usize;
            let mut end = None;
            for (idx, ch) in cell[k..].char_indices() {
                match ch {
                    '(' | '（' => depth += 1,
                    ')' | '）' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(k + idx + ch.len_utf8());
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if let Some(end) = end {
                note = cell[k + c.len_utf8()..end - 1].to_owned();
                k = skip_ws(end);
            }
        }
        if cell[k..].starts_with([':', '：']) {
            out.push(Head { n, note });
        }
        i = j;
    }
    out
}

/// 頭部の注記が、後の版の AC (v0.1 から除くもの) を示すか。
fn is_later_version(note: &str) -> bool {
    let has_version = note
        .match_indices('v')
        .any(|(i, _)| note[i + 1..].starts_with(|c: char| c.is_ascii_digit()));
    has_version || note.contains("版から")
}

/// 1 行 (要件 ID のセルと受入基準のセル) から、v0.1 の AC を取り出す。
///
/// # Errors
///
/// 要件 ID のセルから ID を 1 つも読めないとき、または受入基準のセルに `AC-<n>:` が 1 つもないとき (理由の文言を返す)。
pub fn extract_row(requirement_cell: &str, acceptance_cell: &str) -> Result<Vec<Ac>, String> {
    let reqs = requirement_ids(requirement_cell);
    if reqs.is_empty() {
        return Err(format!("要件 ID を読めません: {requirement_cell}"));
    }
    let hs = heads(acceptance_cell);
    if hs.is_empty() {
        return Err(format!("`AC-<n>:` が 1 つもありません: {requirement_cell}"));
    }
    let mut ns = Vec::new();
    for h in &hs {
        // 後の版の AC、または重複は除く
        if !is_later_version(&h.note) && !ns.contains(&h.n) {
            ns.push(h.n);
        }
    }
    let mut out = Vec::new();
    for r in &reqs {
        for &n in &ns {
            out.push(Ac {
                id: format!("AC-{r}-{n}"),
                requirement: r.clone(),
                n,
            });
        }
    }
    Ok(out)
}

/// 01 の markdown から v0.1 の AC をすべて読む。読めない行があれば失敗する。
///
/// # Errors
///
/// 表の見出しや列が見つからないとき、または行から AC を読めないとき (`Error::BadRow` など)。
pub fn read_acs(markdown_01: &str) -> Result<Vec<Ac>, Error> {
    let mut all: Vec<Ac> = Vec::new();
    for heading in AC_HEADINGS {
        for table in read_tables(markdown_01, heading, &["要件", "受入基準"])? {
            for row in &table.rows {
                let req = table.cell(row, "要件").unwrap_or_default();
                let acc = table.cell(row, "受入基準").unwrap_or_default();
                let acs = extract_row(req, acc).map_err(|reason| Error::BadRow {
                    line: row.line,
                    reason,
                })?;
                for ac in acs {
                    if all.iter().any(|a| a.id == ac.id) {
                        return Err(Error::BadRow {
                            line: row.line,
                            reason: format!("AC ID が重複しています: {}", ac.id),
                        });
                    }
                    all.push(ac);
                }
            }
        }
    }
    all.sort();
    Ok(all)
}

/// 01 §5.5 (v0.2 以降の概要) の見出し (固定)。
pub const LATER_HEADING: &str = "### 5.5 v0.2 以降 (概要。各版の着手前に詳細化)";

/// §5.5 の検証の単位: 番号付きの AC か、AC を持たない要件。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Unit {
    /// `FR-101` の形。
    pub requirement: String,
    /// 番号付きの AC があればその ID (`AC-FR-101-1`)。無ければ要件だけ。
    pub ac: Option<String>,
}

/// セルの中の要件 ID (`FR-123` / `NFR-123`) を、書かれた順に取り出す (`NFR-` の中の `FR-` は数えない)。
/// §5.5 の `要件` 列は `FR-103 / FR-101 (v0.2)`・`NFR-106 長大行の編集 (v0.2)` のように説明の語を挟むので、
/// `requirement_ids` (区切りが `,` / `、` だけ) とは別に読む。
fn requirement_refs(cell: &str) -> Vec<String> {
    let bytes = cell.as_bytes();
    let mut out: Vec<(usize, String)> = Vec::new();
    for prefix in ["NFR-", "FR-"] {
        let mut from = 0;
        while let Some(p) = cell[from..].find(prefix) {
            let at = from + p;
            from = at + prefix.len();
            let before_ok = at == 0 || {
                let b = bytes[at - 1];
                !(b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            };
            let digits: String = cell[from..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            if before_ok && !digits.is_empty() {
                out.push((at, format!("{prefix}{digits}")));
            }
        }
    }
    out.sort();
    out.into_iter().map(|(_, id)| id).collect()
}

/// 受入基準のセルの `AC-<n>:` (間に括弧の注記を許す) と、その直前に書かれた要件 ID (`FR-101 AC-1:`)。
fn later_heads(cell: &str) -> Vec<(Option<String>, u32)> {
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

/// 01 §5.5 の、`要件` 列に `version` (`v0.2` など) を含む行から検証の単位を読む。
/// その版の行で要件 ID を読めないものは失敗にする (黙って読み飛ばさない、docs/11 §8.5)。
///
/// # Errors
///
/// 表の見出しや列が見つからないとき、またはその版の行で要件 ID を読めないとき (`Error::BadRow` など)。
pub fn read_later_units(markdown_01: &str, version: &str) -> Result<Vec<Unit>, Error> {
    let mut out = std::collections::BTreeSet::new();
    for table in read_tables(markdown_01, LATER_HEADING, &["要件", "受入基準 (概要)"])? {
        for row in &table.rows {
            let req_cell = table.cell(row, "要件").unwrap_or_default();
            if !req_cell.contains(version) {
                continue;
            }
            let reqs: Vec<String> = requirement_refs(req_cell);
            let Some(first) = reqs.first().cloned() else {
                return Err(Error::BadRow {
                    line: row.line,
                    reason: format!("要件 ID を読めません: {req_cell}"),
                });
            };
            let mut with_ac = std::collections::BTreeSet::new();
            for (req, n) in later_heads(table.cell(row, "受入基準 (概要)").unwrap_or_default())
            {
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
    }
    Ok(out.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: Vec<Ac>) -> Vec<String> {
        v.into_iter().map(|a| a.id).collect()
    }

    #[test]
    fn 要件_id_の範囲を展開する() {
        assert_eq!(
            requirement_ids("NFR-101〜105"),
            ["NFR-101", "NFR-102", "NFR-103", "NFR-104", "NFR-105"]
        );
        assert_eq!(
            requirement_ids("FR-201〜204, 206"),
            ["FR-201", "FR-202", "FR-203", "FR-204", "FR-206"]
        );
        assert_eq!(requirement_ids("FR-117 (保存直前の照合)"), ["FR-117"]);
    }

    #[test]
    fn ac_の始まりは_ac_n_コロンだけ() {
        let r = extract_row("FR-111", "AC-1: a。AC-2: b。AC-1 と同じ。AC-3: c").unwrap();
        assert_eq!(ids(r), ["AC-FR-111-1", "AC-FR-111-2", "AC-FR-111-3"]);
    }

    #[test]
    fn 注記の括弧は区切りにしない() {
        let r = extract_row("NFR-603", "AC-6 (ダウングレード: x): y").unwrap();
        assert_eq!(ids(r), ["AC-NFR-603-6"]);
    }

    #[test]
    fn 後の版の_ac_は除く() {
        let r = extract_row(
            "FR-104",
            "AC-1: a。AC-2 (全保存) は §5.5 (v0.2)。AC-3 (OQ-116): b。AC-4 (参考、v0.2 から): c",
        )
        .unwrap();
        assert_eq!(ids(r), ["AC-FR-104-1", "AC-FR-104-3"]);
    }

    #[test]
    fn 範囲の行は要件ごとに展開する() {
        let r = extract_row("NFR-101〜103", "AC-1: x").unwrap();
        assert_eq!(ids(r), ["AC-NFR-101-1", "AC-NFR-102-1", "AC-NFR-103-1"]);
    }

    #[test]
    fn 読めない行は失敗する() {
        assert!(extract_row("説明", "AC-1: x").is_err());
        assert!(extract_row("FR-101", "AC なし").is_err());
    }
    #[test]
    fn 後の版の受入基準の_ac_の頭を読む() {
        assert_eq!(
            later_heads("FR-101 AC-1: 起動する。FR-103 AC-1: 一覧。AC-2 (v0.3): 後。AC-3 は本文"),
            [
                (Some("FR-101".to_owned()), 1),
                (Some("FR-103".to_owned()), 1),
                (None, 2)
            ]
        );
    }

    #[test]
    fn 後の版の単位を取り出す() {
        let doc = "### 5.5 v0.2 以降 (概要。各版の着手前に詳細化)
| 要件 | 受入基準 (概要) |
|---|---|
| FR-103 / FR-101 (v0.2) | FR-101 AC-1: a。FR-103 AC-2: b |
| NFR-106 長大行 (v0.2) | AC-1: c。AC-2: d |
| FR-518 (v0.2、Could) | 文だけ |
| FR-122 (v0.3) | AC-1: 後の版 |
## 6. 次
";
        let got: Vec<_> = read_later_units(doc, "v0.2")
            .unwrap()
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

    #[test]
    fn 後の版の行で要件_id_を読めなければ失敗する() {
        let doc = "### 5.5 v0.2 以降 (概要。各版の着手前に詳細化)
| 要件 | 受入基準 (概要) |
|---|---|
| 複数文書 (v0.2) | AC-1: a |
";
        assert!(matches!(
            read_later_units(doc, "v0.2"),
            Err(Error::BadRow { line: 4, .. })
        ));
    }
}
