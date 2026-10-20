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
}
