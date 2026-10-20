//! docs の Markdown の表を、機械可読の規則で読む小さな lib (docs/02 §4.1、docs/11 §8.5)。
//!
//! 見出しと列の名前を固定して読み、規則に合わない表は **失敗** にする (黙って読み飛ばさない)。
//! 利用者は `xtask` と、eve-core・eve-tui のテスト (dev 依存)。外部依存はなく、std だけを使う。

#![forbid(unsafe_code)]

use std::fmt;

pub mod ac;
pub mod crates;

/// 表の 1 行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// docs のファイルの中の行番号 (1 始まり)。
    pub line: usize,
    /// セルの内容 (前後の空白を除いたもの)。
    pub cells: Vec<String>,
}

/// 見出しの下にある 1 つの表。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    /// 表の見出し行の行番号。
    pub line: usize,
    pub columns: Vec<String>,
    pub rows: Vec<Row>,
}

impl Table {
    /// 列の名前から位置を引く。
    pub fn column(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c == name)
    }

    /// 行のセルを、列の名前で引く。
    pub fn cell<'a>(&self, row: &'a Row, name: &str) -> Option<&'a str> {
        let i = self.column(name)?;
        row.cells.get(i).map(String::as_str)
    }
}

/// 表を読むときのエラー。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// 固定された見出しが見つからない。
    HeadingNotFound { heading: String },
    /// 見出しの下に表が 1 つもない。
    NoTable { heading: String },
    /// 必要な列がない。
    MissingColumn {
        heading: String,
        line: usize,
        column: String,
    },
    /// 行の形が規則に合わない。
    BadRow { line: usize, reason: String },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::HeadingNotFound { heading } => write!(f, "見出しが見つかりません: {heading}"),
            Error::NoTable { heading } => write!(f, "見出しの下に表がありません: {heading}"),
            Error::MissingColumn {
                heading,
                line,
                column,
            } => {
                write!(f, "{heading} の表 (行 {line}) に列 `{column}` がありません")
            }
            Error::BadRow { line, reason } => write!(f, "行 {line}: {reason}"),
        }
    }
}

impl std::error::Error for Error {}

fn heading_level(line: &str) -> Option<usize> {
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    if (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
        Some(hashes)
    } else {
        None
    }
}

/// 見出し (`### 4.1 crate 一覧` のように `#` から始まる行全体) の下の本文を返す。
/// 同じか上の階層の次の見出しまでを本文とする。戻り値は (行番号 (1 始まり)、本文の行) の組。
pub fn section<'a>(markdown: &'a str, heading: &str) -> Result<Vec<(usize, &'a str)>, Error> {
    let mut lines = markdown
        .lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l.trim_end()));
    let mut in_fence = false;
    let mut level = None;
    for (n, l) in lines.by_ref() {
        if l.starts_with("```") {
            in_fence = !in_fence;
        }
        if !in_fence && l == heading {
            level = heading_level(l);
            let _ = n;
            break;
        }
    }
    let level = level.ok_or_else(|| Error::HeadingNotFound {
        heading: heading.to_owned(),
    })?;
    let mut body = Vec::new();
    let mut in_fence = false;
    for (n, l) in lines {
        if l.starts_with("```") {
            in_fence = !in_fence;
        }
        if !in_fence && heading_level(l).is_some_and(|h| h <= level) {
            break;
        }
        body.push((n, l));
    }
    Ok(body)
}

/// 1 行を、バッククォートの外の `|` で分ける (`\|` は区切りにしない)。
fn split_cells(line: &str) -> Vec<String> {
    let line = line.trim();
    let inner = line.strip_prefix('|').unwrap_or(line);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut in_code = false;
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '`' => {
                in_code = !in_code;
                cur.push(c);
            }
            '\\' if chars.peek() == Some(&'|') => {
                cur.push('|');
                chars.next();
            }
            '|' if !in_code => {
                cells.push(cur.trim().to_owned());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    cells.push(cur.trim().to_owned());
    cells
}

fn is_separator(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|') && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

/// 本文から、パイプ形式の表をすべて取り出す。
pub fn tables_in(body: &[(usize, &str)]) -> Vec<Table> {
    let mut tables = Vec::new();
    let mut i = 0;
    let mut in_fence = false;
    while i < body.len() {
        let (n, l) = body[i];
        if l.starts_with("```") {
            in_fence = !in_fence;
        }
        let starts = !in_fence && l.trim_start().starts_with('|');
        if starts && i + 1 < body.len() && is_separator(body[i + 1].1) {
            let columns = split_cells(l);
            let mut rows = Vec::new();
            i += 2;
            while i < body.len() && body[i].1.trim_start().starts_with('|') {
                rows.push(Row {
                    line: body[i].0,
                    cells: split_cells(body[i].1),
                });
                i += 1;
            }
            tables.push(Table {
                line: n,
                columns,
                rows,
            });
        } else {
            i += 1;
        }
    }
    tables
}

/// 固定された見出しの下の表をすべて読み、必要な列がそろっていることを確かめる。
/// 列の追加は末尾にだけ許し、知らない列は無視する (docs/11 §8.5)。
pub fn read_tables(markdown: &str, heading: &str, required: &[&str]) -> Result<Vec<Table>, Error> {
    let body = section(markdown, heading)?;
    let tables = tables_in(&body);
    if tables.is_empty() {
        return Err(Error::NoTable {
            heading: heading.to_owned(),
        });
    }
    for t in &tables {
        for col in required {
            if t.column(col).is_none() {
                return Err(Error::MissingColumn {
                    heading: heading.to_owned(),
                    line: t.line,
                    column: (*col).to_owned(),
                });
            }
        }
    }
    Ok(tables)
}

/// セルの中のバッククォートで囲まれた部分だけを、識別子として取り出す。
/// 説明の文や括弧書きは読まない。
pub fn identifiers(cell: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = cell;
    while let Some(start) = rest.find('`') {
        let after = &rest[start + 1..];
        match after.find('`') {
            Some(end) => {
                out.push(after[..end].to_owned());
                rest = &after[end + 1..];
            }
            None => break,
        }
    }
    out
}

/// 括弧 (半角・全角) の中を除いた文字列を返す。入れ子は外側の括弧に含める。
pub fn strip_parentheticals(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for c in s.chars() {
        match c {
            '(' | '（' => depth += 1,
            ')' | '）' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const MD: &str = "\
# 題

### 4.1 crate 一覧

| crate | 版 | 禁止 crate |
|---|---|---|
| `eve-text` | v0.1 | `ratatui`, `crossterm` |
| `eve-ai` | v1.L | — |

#### 4.1a 補足

| a | b |
|---|---|
| 1 | 2 |

### 4.2 次

| x |
|---|
| 1 |
";

    #[test]
    fn 見出しの下の表を読む() {
        let t = read_tables(MD, "### 4.1 crate 一覧", &[]).unwrap();
        assert_eq!(t.len(), 2); // 下位の見出しの表も、同じ節として読む
        assert_eq!(t[0].rows.len(), 2);
        assert_eq!(t[0].cell(&t[0].rows[0], "crate"), Some("`eve-text`"));
    }

    #[test]
    fn 次の見出しで終わる() {
        let t = read_tables(MD, "### 4.2 次", &["x"]).unwrap();
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn 見出しがなければ失敗する() {
        assert!(matches!(
            read_tables(MD, "### 9.9 無い", &[]),
            Err(Error::HeadingNotFound { .. })
        ));
    }

    #[test]
    fn 必要な列がなければ失敗する() {
        // 下位の見出しの表 (a / b) には `crate` 列がない
        let e = read_tables(MD, "### 4.1 crate 一覧", &["crate"]).unwrap_err();
        assert!(matches!(e, Error::MissingColumn { .. }), "{e}");
    }

    #[test]
    fn 識別子はバッククォートの中だけ() {
        assert_eq!(identifiers("`a`, `b` (注: `c`の話)"), ["a", "b", "c"]);
        assert!(identifiers("説明だけ").is_empty());
    }

    #[test]
    fn セルの区切りはバッククォートの外の縦線だけ() {
        let c = split_cells("| `a|b` | c \\| d | e |");
        assert_eq!(c, ["`a|b`", "c | d", "e"]);
    }

    #[test]
    fn 括弧の中を除く() {
        assert_eq!(strip_parentheticals("FR-117 (保存直前の照合)"), "FR-117 ");
        assert_eq!(strip_parentheticals("a（b(c)d）e"), "ae");
    }

    #[test]
    fn コードブロックの中の見出しは見出しとして読まない() {
        let md = "### A\n\n```\n### B\n```\n\n| x |\n|---|\n| 1 |\n";
        assert_eq!(read_tables(md, "### A", &["x"]).unwrap().len(), 1);
    }
}
