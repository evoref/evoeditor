//! 02 §4.1 の crate 一覧の読み取り (docs/11 §8.5「crate 一覧」)。

use crate::{Error, identifiers, read_tables, strip_parentheticals};

/// 02 の crate 一覧の見出し (固定)。
pub const CRATE_HEADING: &str = "### 4.1 crate 一覧";

/// crate 一覧の 1 行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrateRow {
    pub name: String,
    /// `版` 列の先頭の版 (`v0.1` など。続く括弧の注記は読まない)。
    pub version: String,
    /// `禁止 crate` 列の crate 名。`—` のときは空。
    pub forbidden: Vec<String>,
    pub line: usize,
}

/// 02 の markdown から crate 一覧を読む。
pub fn read_crates(markdown_02: &str) -> Result<Vec<CrateRow>, Error> {
    let mut out = Vec::new();
    for table in read_tables(
        markdown_02,
        CRATE_HEADING,
        &[
            "crate",
            "種別",
            "版",
            "責務",
            "主な外部依存 (案)",
            "禁止事項",
            "禁止 crate",
        ],
    )? {
        for row in &table.rows {
            let bad = |reason: &str| Error::BadRow {
                line: row.line,
                reason: reason.to_owned(),
            };
            let name = identifiers(table.cell(row, "crate").unwrap_or_default())
                .into_iter()
                .next()
                .ok_or_else(|| bad("`crate` 列に crate 名 (バッククォート) がありません"))?;
            let ver = strip_parentheticals(table.cell(row, "版").unwrap_or_default());
            let version = ver.split_whitespace().next().unwrap_or_default().to_owned();
            if version.is_empty() {
                return Err(bad("`版` 列が空です"));
            }
            let cell = table.cell(row, "禁止 crate").unwrap_or_default();
            let cell = strip_parentheticals(cell);
            let forbidden = if cell.trim() == "—" || cell.trim().is_empty() {
                Vec::new()
            } else {
                let ids = identifiers(&cell);
                if ids.is_empty() {
                    return Err(bad(
                        "`禁止 crate` 列に crate 名 (バッククォート) も `—` もありません",
                    ));
                }
                ids
            };
            out.push(CrateRow {
                name,
                version,
                forbidden,
                line: row.line,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MD: &str = "\
### 4.1 crate 一覧

| crate | 種別 | 版 | 責務 | 主な外部依存 (案) | 禁止事項 | 禁止 crate |
|---|---|---|---|---|---|---|
| `eve-text` | lib | v0.1 | 責務 | ropey | 他の crate | `ratatui`, `crossterm` |
| `evoeditor` (bin) | bin | v0.1 (注) | 責務 | lexopt | 業務 | `eve-testkit` |
| `eve-ai` | lib | v1.L | 責務 | tokio | x | — |
";

    #[test]
    fn crate_一覧を読む() {
        let v = read_crates(MD).unwrap();
        assert_eq!(v.len(), 3);
        assert_eq!(v[0].name, "eve-text");
        assert_eq!(v[0].forbidden, ["ratatui", "crossterm"]);
        assert_eq!(v[1].name, "evoeditor");
        assert_eq!(v[1].version, "v0.1");
        assert!(v[2].forbidden.is_empty());
        assert_eq!(v[2].version, "v1.L");
    }

    #[test]
    fn 禁止_crate_が空でも_ダッシュでもなければ失敗する() {
        let md = MD.replace("`ratatui`, `crossterm`", "ratatui など");
        assert!(read_crates(&md).is_err());
    }
}
