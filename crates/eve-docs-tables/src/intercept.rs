//! 04 §7.3a の端末の横取り表 (docs/04-tui-design.md) を読む。
//!
//! 読み手は、eve-tui の横取りの定数表の突合と、eve-core の既定キーマップの到達性の検査 (FR-606) の 2 つ
//! (docs/11 §8.5、docs/review/v0.3-findings.md の STR-E1)。

use crate::{Error, identifiers, read_tables, strip_parentheticals};

/// 横取り表の見出し (04 §7.3a)。
pub const HEADING: &str = "### 7.3a 端末の横取り表 (FR-606 の到達性検査の入力)";

/// 横取り表の 1 行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Terminal {
    pub line: usize,
    /// `端末` 列 (`Windows Terminal`・`conhost` など。版の注記も含めて書かれたまま)。
    pub name: String,
    /// `横取りするキー` 列のキー表記 (バッククォートの中。括弧の注記の中は読まない)。
    pub keys: Vec<String>,
}

/// 04 の本文から横取り表を読む。
pub fn read(doc04: &str) -> Result<Vec<Terminal>, Error> {
    let mut out = Vec::new();
    for t in read_tables(doc04, HEADING, &["端末", "横取りするキー"])? {
        for r in &t.rows {
            out.push(Terminal {
                line: r.line,
                name: t.cell(r, "端末").unwrap_or_default().to_owned(),
                keys: identifiers(&strip_parentheticals(
                    t.cell(r, "横取りするキー").unwrap_or_default(),
                )),
            });
        }
    }
    Ok(out)
}

/// `name` の端末の行の、横取りするキー。行が無ければ失敗にする。
pub fn keys_of(doc04: &str, name: &str) -> Result<Vec<String>, Error> {
    read(doc04)?
        .into_iter()
        .find(|t| t.name == name)
        .map(|t| t.keys)
        .ok_or_else(|| Error::BadRow {
            line: 0,
            reason: format!("04 §7.3a に端末 `{name}` の行がありません"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 実際の_04_を読める() {
        let doc = include_str!("../../../docs/04-tui-design.md");
        let wt = keys_of(doc, "Windows Terminal").unwrap();
        assert!(wt.iter().any(|k| k == "ctrl+shift+p"), "{wt:?}");
        assert_eq!(
            keys_of(doc, "conhost").unwrap(),
            ["alt+enter", "alt+space", "alt+f4"]
        );
        assert!(keys_of(doc, "無い端末").is_err());
    }
}
