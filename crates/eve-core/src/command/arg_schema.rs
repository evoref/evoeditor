//! 組込みのコマンドの引数の形 (`ArgSchema`、docs/07 §11.2 / §11.3)。07 §11.6 のカタログの `args` 列が正本で、
//! 一覧の出力 (`evoeditor --list-commands`、07 §11.5) と生成リファレンス (`docs/reference/commands.md`、09 §11.2) がここを読む。
//! 引数の無いコマンドは表に載せない (空の並び)。カタログとの突合は下の試験で行う。
//!
//! spec: 07 §11.2, 07 §11.6

use crate::command::ids;
use crate::ui_model::palette::{ENCODINGS, LINE_ENDINGS};

/// 引数の型 (07 §11.2 の表の型)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgType {
    /// `bool`。
    Bool,
    /// `int`。
    Int,
    /// `int|"$"`: 数値、または最終行・末尾を表す文字列 `"$"`。
    IntOrLast,
    /// `string`。
    String,
    /// `path` (文字列。変数展開あり)。
    Path,
    /// `enum(<値…>)`: (値, 表示) の並び。
    Enum(&'static [(&'static str, &'static str)]),
}

impl ArgType {
    /// 07 §11.2 の表記 (`bool`、`int|"$"`、`enum(lf|crlf|cr)` など)。
    pub fn notation(self) -> String {
        match self {
            ArgType::Bool => "bool".to_owned(),
            ArgType::Int => "int".to_owned(),
            ArgType::IntOrLast => "int|\"$\"".to_owned(),
            ArgType::String => "string".to_owned(),
            ArgType::Path => "path".to_owned(),
            ArgType::Enum(values) => format!(
                "enum({})",
                values.iter().map(|(v, _)| *v).collect::<Vec<_>>().join("|")
            ),
        }
    }
}

/// 引数 1 つの形。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArgSpec {
    /// 引数名 (`Args` のキー)。
    pub name: &'static str,
    /// 型。
    pub ty: ArgType,
    /// 必須か (カタログの `name?` は省略可能)。
    pub required: bool,
}

const fn arg(name: &'static str, ty: ArgType, required: bool) -> ArgSpec {
    ArgSpec { name, ty, required }
}

const COUNT: &[ArgSpec] = &[arg("count", ArgType::Int, false)];
const PATH: &[ArgSpec] = &[arg("path", ArgType::Path, false)];

/// コマンド → 引数の形。ID の順。
const TABLE: &[(&str, &[ArgSpec])] = &[
    (
        ids::CURSOR_GOTO_LINE,
        &[
            arg("line", ArgType::IntOrLast, false),
            arg("col", ArgType::Int, false),
        ],
    ),
    (ids::CURSOR_MOVE_DOWN, COUNT),
    (ids::CURSOR_MOVE_LEFT, COUNT),
    (ids::CURSOR_MOVE_RIGHT, COUNT),
    (ids::CURSOR_MOVE_UP, COUNT),
    (ids::CURSOR_MOVE_WORD_LEFT, COUNT),
    (ids::CURSOR_MOVE_WORD_RIGHT, COUNT),
    (ids::EDIT_INSERT_TEXT, &[arg("text", ArgType::String, true)]),
    (ids::FILE_OPEN, PATH),
    (
        ids::FILE_REOPEN_WITH_ENCODING,
        &[arg("encoding", ArgType::Enum(ENCODINGS), true)],
    ),
    (ids::FILE_SAVE_AS, PATH),
    (
        ids::FILE_SET_ENCODING,
        &[
            arg("encoding", ArgType::Enum(ENCODINGS), true),
            arg("bom", ArgType::Bool, false),
        ],
    ),
    (
        ids::FILE_SET_LINE_ENDING,
        &[arg("eol", ArgType::Enum(LINE_ENDINGS), true)],
    ),
    (ids::SEARCH_OPEN, &[arg("query", ArgType::String, false)]),
    (ids::SELECT_EXTEND_DOWN, COUNT),
    (ids::SELECT_EXTEND_LEFT, COUNT),
    (ids::SELECT_EXTEND_RIGHT, COUNT),
    (ids::SELECT_EXTEND_UP, COUNT),
    (ids::SELECT_EXTEND_WORD_LEFT, COUNT),
    (ids::SELECT_EXTEND_WORD_RIGHT, COUNT),
    (ids::UI_DIALOG_CHOOSE, &[arg("choice", ArgType::Int, true)]),
];

/// `id` のコマンドの引数の形 (引数が無ければ空)。
pub fn command_args(id: &str) -> &'static [ArgSpec] {
    TABLE
        .iter()
        .find(|(c, _)| *c == id)
        .map_or(&[], |(_, a)| *a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eve_docs_tables::{catalog, identifiers, strip_parentheticals};
    use std::collections::BTreeMap;

    /// 07 §11.6 の `args` 列: 括弧の注記を除いた識別子 (`path?` など) を、行の ID のすべてに当てる。
    fn catalog_args() -> BTreeMap<String, Vec<String>> {
        let doc = include_str!("../../../../docs/07-config-extensibility.md");
        let mut out = BTreeMap::new();
        for row in catalog::read(doc).expect("カタログを読める") {
            let names = identifiers(&strip_parentheticals(&row.args));
            for id in &row.ids {
                out.insert(id.clone(), names.clone());
            }
        }
        out
    }

    #[test]
    fn 表は_id_の順で_重複が無い() {
        assert!(TABLE.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn 引数の名前と省略の可否が_07_のカタログの_args_列と一致する() {
        let cat = catalog_args();
        let registry = crate::editor::builtin_registry();
        let mut errors = Vec::new();
        for spec in registry.iter() {
            let id = spec.id.as_str();
            let Some(want) = cat.get(id) else {
                continue; // カタログに無いコマンドは perm_catalog の試験が報告する
            };
            let have: Vec<String> = command_args(id)
                .iter()
                .map(|a| format!("{}{}", a.name, if a.required { "" } else { "?" }))
                .collect();
            if &have != want {
                errors.push(format!("{id}: 実装 {have:?} / カタログ {want:?}"));
            }
        }
        for (id, _) in TABLE {
            if registry.get(&eve_view::CommandId::new(id)).is_none() {
                errors.push(format!("{id}: 登録されていないコマンドの引数"));
            }
        }
        assert!(errors.is_empty(), "{}", errors.join("\n"));
    }

    #[test]
    fn 型の表記は_07_の表の書き方() {
        assert_eq!(ArgType::IntOrLast.notation(), "int|\"$\"");
        assert_eq!(ArgType::Enum(LINE_ENDINGS).notation(), "enum(lf|crlf|cr)");
    }
}
