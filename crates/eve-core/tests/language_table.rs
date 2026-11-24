//! 組込みの言語定義 (`eve_config::BUILTIN_LANGUAGES`) が 07 §9.6.1 の表 (1 行 1 言語、LANG-1 の正本。最後の列
//! `trim_trailing_whitespace_on_save` は v0.5 の M34 の段 3 から読む) と一致すること。どの
//! 2 つの組込みの言語も同じ `file_types`・`file_names` を持たないこと。
//! 表は `eve-docs-tables` で読む (docs/11 §8.5。`config_reserved` と同じ形。eve-config は `eve-docs-tables` に依存しない)。

use std::collections::BTreeMap;

use eve_config::{BUILTIN_LANGUAGES, IndentUnit};
use eve_docs_tables::{identifiers, section, tables_in};

const DOC: &str = include_str!("../../../docs/07-config-extensibility.md");

/// 1 つのセルの値 (バッククォートの中。`—` は空)。
fn cell(t: &eve_docs_tables::Table, r: &eve_docs_tables::Row, col: &str) -> Vec<String> {
    let c = t.cell(r, col).unwrap_or_else(|| panic!("列 {col} がある"));
    identifiers(c)
}

fn unit_name(u: Option<IndentUnit>) -> Vec<String> {
    match u {
        Some(IndentUnit::Tab) => vec!["tab".to_owned()],
        Some(IndentUnit::Space) => vec!["space".to_owned()],
        None => Vec::new(),
    }
}

// verifies: AC-FR-704-1, AC-FR-216-1
#[test]
fn 組込みの言語定義は_07_の_9_6_1_の表と一致する() {
    let body = section(
        DOC,
        "#### 9.6.1 組込みの言語定義の表 (v0.5 の M33。LANG-1 の正本)",
    )
    .expect("07 §9.6.1 を読める");
    let tables = tables_in(&body);
    let t = tables
        .iter()
        .find(|t| t.column("display_name").is_some())
        .expect("表がある");
    let mut spec: Vec<(String, Vec<Vec<String>>)> = Vec::new();
    for r in &t.rows {
        let id = cell(t, r, "ID");
        assert_eq!(id.len(), 1, "{:?}", r.cells);
        spec.push((
            id[0].clone(),
            vec![
                cell(t, r, "display_name"),
                cell(t, r, "file_types"),
                cell(t, r, "file_names"),
                cell(t, r, "first_line"),
                cell(t, r, "comment.line"),
                cell(t, r, "indent.unit"),
                cell(t, r, "trim_trailing_whitespace_on_save"),
            ],
        ));
    }
    let have: Vec<(String, Vec<Vec<String>>)> = BUILTIN_LANGUAGES
        .iter()
        .map(|b| {
            let s = |v: &[&str]| v.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
            (
                b.id.to_owned(),
                vec![
                    vec![b.display_name.to_owned()],
                    s(b.file_types),
                    s(b.file_names),
                    b.first_line.map(str::to_owned).into_iter().collect(),
                    b.comment_line.map(str::to_owned).into_iter().collect(),
                    unit_name(b.indent_unit),
                    b.trim_trailing_whitespace_on_save
                        .map(|v| v.to_string())
                        .into_iter()
                        .collect(),
                ],
            )
        })
        .collect();
    assert_eq!(
        have, spec,
        "eve-config の BUILTIN_LANGUAGES を 07 §9.6.1 の表に合わせる"
    );
}

#[test]
fn 組込みの言語どうしは同じ名前を持たない() {
    let mut seen: BTreeMap<(bool, String), &str> = BTreeMap::new();
    for b in BUILTIN_LANGUAGES {
        for (is_type, names) in [(true, b.file_types), (false, b.file_names)] {
            for n in names {
                if let Some(other) = seen.insert((is_type, n.to_ascii_lowercase()), b.id) {
                    panic!("{n} が {other} と {} の両方にある", b.id);
                }
            }
        }
    }
}
