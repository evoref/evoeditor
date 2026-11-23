//! ユーザーのキーマップファイル (`keymap.toml`) の読込 (docs/07 §7.1、01 FR-604)。
//!
//! 書式だけを解く: `[keys.<コンテキスト>]` の表の「キー = 値」を、書かれた順の `KeymapEntry` の列にする。
//! キー名・コマンド・`when`・`args` の検証と層への組み込みは core が行う (コマンドの一覧は core が持つため)。
//! ファイルが無い・壊れているときも `KeymapFile` は返り、問題は `Diagnostic` の列として返す。

use toml::Spanned;
use toml::de::{DeString, DeTable, DeValue};

use crate::{DiagKind, Diagnostic, Severity};

/// 1 件の割当の動作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAction {
    /// コマンドを実行する。
    Command {
        /// コマンド ID (core が一覧と照合する)。
        command: String,
        /// `when` 句の本文。書かれていなければ `None` (読み方は core、07 §7.3)。
        when: Option<String>,
        /// `args` の表 (書かれた順)。書かれていなければ `None`。引数の名前と型の照合は core が行う (07 §7.6)。
        args: Option<Vec<(String, KeyArgValue)>>,
    },
    /// `"none"`: 下位の層の割当をこの層で遮蔽する。
    None,
}

/// 割当の `args` の 1 つの値 (07 §7.1)。コマンドの引数の型 (07 §11.2) に当たる値だけを区別し、それ以外は型の名前だけを残す。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyArgValue {
    /// 整数。
    Int(i64),
    /// 文字列。
    Str(String),
    /// 真偽値。
    Bool(bool),
    /// コマンドの引数にならない型の値 (浮動小数点数・配列・表など)。中身は TOML の型の名前 (`float` など)。
    Other(&'static str),
}

/// `[keys.<context>]` の 1 行。配列値は書かれた順に複数の `KeymapEntry` になる。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeymapEntry {
    /// `[keys.<context>]` のコンテキスト名 (検証は core)。
    pub context: String,
    /// 書かれたとおりのキー (空白区切りでストロークの列)。
    pub keys: String,
    /// 割当の動作。
    pub action: KeyAction,
    /// 書かれた行 (1 始まり)。
    pub line: u32,
    /// 書かれた桁 (1 始まりの文字数)。
    pub column: u32,
}

/// 読み込んだ `keymap.toml`。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeymapFile {
    /// 割当 (書かれた順)。
    pub entries: Vec<KeymapEntry>,
    /// 書式の問題 (出現順)。
    pub diagnostics: Vec<Diagnostic>,
}

/// `args` の 1 つの値。`i64` に収まらない整数は、引数にならない値として型の名前だけを残す。
fn arg_value(v: &DeValue<'_>) -> KeyArgValue {
    match v {
        DeValue::String(s) => KeyArgValue::Str(s.to_string()),
        DeValue::Boolean(b) => KeyArgValue::Bool(*b),
        DeValue::Integer(i) => i64::from_str_radix(&i.as_str().replace('_', ""), i.radix())
            .map_or(KeyArgValue::Other("integer"), KeyArgValue::Int),
        other => KeyArgValue::Other(other.type_str()),
    }
}

struct Cx<'a> {
    out: KeymapFile,
    /// 行の始まりの表 (位置を引く)。
    lines: crate::position::LineIndex<'a>,
    /// 上限を超えて残さなかった診断の数。
    dropped: usize,
}

impl Cx<'_> {
    fn diag(&mut self, severity: Severity, kind: DiagKind, key: &str, at: usize) {
        // 上限を超えた分は位置も引かずに数えるだけ (07 §4.3)
        if self.out.diagnostics.len() >= crate::CONFIG_MAX_DIAGNOSTICS {
            self.dropped += 1;
            return;
        }
        let (line, column) = self.lines.pos(at);
        self.out.diagnostics.push(Diagnostic {
            severity,
            kind,
            key: key.to_owned(),
            value: String::new(),
            line,
            column,
            condition: None,
        });
    }

    fn mismatch(&mut self, key: &str, v: &Spanned<DeValue<'_>>, expected: &'static str) {
        self.diag(
            Severity::Warning,
            DiagKind::TypeMismatch {
                expected,
                found: v.get_ref().type_str(),
            },
            key,
            v.span().start,
        );
    }

    fn context(&mut self, name: &str, table: &DeTable<'_>) {
        for (k, v) in table {
            let keys: &str = k.get_ref();
            let at = k.span().start;
            match v.get_ref() {
                DeValue::Array(items) => {
                    for item in items {
                        self.action(name, keys, at, item);
                    }
                }
                _ => self.action(name, keys, at, v),
            }
        }
    }

    fn action(&mut self, context: &str, keys: &str, at: usize, v: &Spanned<DeValue<'_>>) {
        let full = format!("keys.{context}.{keys}");
        let action = match v.get_ref() {
            DeValue::String(s) if s.as_ref() == "none" => KeyAction::None,
            DeValue::String(s) => KeyAction::Command {
                command: s.to_string(),
                when: None,
                args: None,
            },
            DeValue::Table(t) => {
                let str_of = |name: &str| match t.get(name).map(|x| x.get_ref()) {
                    Some(DeValue::String(s)) => Some(s.to_string()),
                    _ => None,
                };
                let Some(command) = str_of("command") else {
                    self.mismatch(&format!("{full}.command"), v, "string");
                    return;
                };
                let args = match t.get("args") {
                    None => None,
                    Some(a) => match a.get_ref() {
                        DeValue::Table(items) => Some(
                            items
                                .iter()
                                .map(|(k, x)| (k.get_ref().to_string(), arg_value(x.get_ref())))
                                .collect(),
                        ),
                        _ => {
                            // `args` が表でない割当は使わない (07 §7.6 の「合わない args」)
                            self.mismatch(&format!("{full}.args"), a, "table");
                            return;
                        }
                    },
                };
                KeyAction::Command {
                    command,
                    when: str_of("when"),
                    args,
                }
            }
            _ => {
                self.mismatch(&full, v, "string");
                return;
            }
        };
        let (line, column) = self.lines.pos(at);
        self.out.entries.push(KeymapEntry {
            context: context.to_owned(),
            keys: keys.to_owned(),
            action,
            line,
            column,
        });
    }

    fn top(&mut self, name: &Spanned<DeString<'_>>, value: &Spanned<DeValue<'_>>) {
        let n: &str = name.get_ref();
        // キーマップは条件付きセクションを読まない。OS ごとの割当は `when` の `os` で書く (07 §3.4 の COND-6)
        if crate::reserved::CONDITIONAL_SECTIONS.contains(&n) {
            self.diag(
                Severity::Warning,
                DiagKind::Condition(crate::ConditionProblem::InKeymap),
                n,
                name.span().start,
            );
            return;
        }
        if n != "keys" {
            self.diag(
                Severity::Warning,
                DiagKind::UnknownKey { suggestion: None },
                n,
                name.span().start,
            );
            return;
        }
        let DeValue::Table(contexts) = value.get_ref() else {
            self.mismatch(n, value, "table");
            return;
        };
        for (ctx, v) in contexts {
            match v.get_ref() {
                DeValue::Table(t) => self.context(ctx.get_ref(), t),
                _ => {
                    // `[keys]` 直下のオプション (`preset` など) は v0.2 では未対応
                    self.diag(
                        Severity::Info,
                        DiagKind::Unsupported,
                        &format!("keys.{}", ctx.get_ref()),
                        ctx.span().start,
                    );
                }
            }
        }
    }
}

/// `keymap.toml` の本文を読む。空でも構文エラーでも `KeymapFile` を返す。
pub fn load_keymap_str(text: &str) -> KeymapFile {
    let mut cx = Cx {
        out: KeymapFile::default(),
        lines: crate::position::LineIndex::new(text),
        dropped: 0,
    };
    // 構文を読む前に、深い入れ子を作る書き方を拒む (07 §4.2。パーサがスタックを使い切らないように)
    if let Some(d) = crate::toml_scan::structure_diagnostic(text) {
        cx.out.diagnostics.push(d);
        return cx.out;
    }
    let root = match DeTable::parse(text) {
        Ok(t) => t,
        Err(e) => {
            let (line, column) = e.span().map_or((0, 0), |s| cx.lines.pos(s.start));
            cx.out.diagnostics.push(Diagnostic {
                severity: Severity::Error,
                kind: DiagKind::Syntax {
                    message: e.message().to_owned(),
                },
                key: String::new(),
                value: String::new(),
                line,
                column,
                condition: None,
            });
            return cx.out;
        }
    };
    for (name, value) in root.get_ref() {
        cx.top(name, value);
    }
    if cx.dropped > 0 {
        cx.out.diagnostics.push(crate::position::more(cx.dropped));
    }
    cx.out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 文字列_表_none_配列を書かれた順に読む() {
        let f = load_keymap_str(
            r#"
[keys.editor]
"ctrl+s" = "file.save"
"alt+up" = { command = "edit.move_line_up" }
"ctrl+w" = "none"
"f5" = [ { command = "a.b", when = "doc.modified" }, "c.d" ]
"#,
        );
        assert!(f.diagnostics.is_empty(), "{:?}", f.diagnostics);
        let by = |k: &str| f.entries.iter().filter(|e| e.keys == k).collect::<Vec<_>>();
        assert_eq!(f.entries.len(), 5);
        assert_eq!(by("ctrl+s")[0].line, 3);
        assert_eq!(by("ctrl+w")[0].action, KeyAction::None);
        let f5 = by("f5");
        assert_eq!(f5.len(), 2, "配列は書かれた順に複数の割当");
        assert_eq!(
            f5[0].action,
            KeyAction::Command {
                command: "a.b".into(),
                when: Some("doc.modified".into()),
                args: None
            }
        );
        assert_eq!(f5[1].context, "editor");
    }

    #[test]
    fn 構文エラーはファイル全体を無視し_診断を返す() {
        let f = load_keymap_str("[keys.editor\n");
        assert!(f.entries.is_empty(), "{:?}", f.entries);
        assert!(matches!(f.diagnostics[0].kind, DiagKind::Syntax { .. }));
    }

    #[test]
    fn 未知のトップと型違いは警告_keys_直下の値は未対応の情報() {
        let f = load_keymap_str("foo = 1\n[keys]\npreset = \"none\"\n[keys.editor]\n\"a\" = 3\n");
        let kinds: Vec<_> = f
            .diagnostics
            .iter()
            .map(|d| (&d.kind, d.severity))
            .collect();
        assert!(
            kinds
                .iter()
                .any(|(k, _)| matches!(k, DiagKind::UnknownKey { .. }))
        );
        assert!(
            kinds
                .iter()
                .any(|(k, s)| matches!(k, DiagKind::Unsupported) && *s == Severity::Info)
        );
        assert!(
            kinds
                .iter()
                .any(|(k, _)| matches!(k, DiagKind::TypeMismatch { .. }))
        );
        assert!(f.entries.is_empty(), "{:?}", f.entries);
    }

    #[test]
    fn args_は名前と値を書かれた順に読む() {
        let f = load_keymap_str(
            "[keys.editor]\n\"f5\" = { command = \"x.y\", args = { a = 1, b = \"s\", c = true, d = 1.5, e = 0x10 } }\n",
        );
        assert!(f.diagnostics.is_empty(), "{:?}", f.diagnostics);
        let KeyAction::Command {
            args: Some(args), ..
        } = &f.entries[0].action
        else {
            panic!("{:?}", f.entries[0].action)
        };
        assert_eq!(
            args,
            &[
                ("a".to_owned(), KeyArgValue::Int(1)),
                ("b".to_owned(), KeyArgValue::Str("s".to_owned())),
                ("c".to_owned(), KeyArgValue::Bool(true)),
                ("d".to_owned(), KeyArgValue::Other("float")),
                ("e".to_owned(), KeyArgValue::Int(16)),
            ]
        );
    }

    #[test]
    fn 表でない_args_の割当は型違いの警告にして使わない() {
        let f = load_keymap_str(
            "[keys.editor]\n\"f5\" = { command = \"x.y\", args = 1 }\n\"f6\" = \"x.y\"\n",
        );
        assert_eq!(f.entries.len(), 1, "{:?}", f.entries);
        assert_eq!(f.entries[0].keys, "f6");
        assert_eq!(f.diagnostics[0].key, "keys.editor.f5.args");
        assert!(matches!(
            f.diagnostics[0].kind,
            DiagKind::TypeMismatch {
                expected: "table",
                ..
            }
        ));
    }
}
