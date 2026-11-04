//! ユーザーのキーマップファイル (`keymap.toml`) の読込 (docs/07 §7.1、01 FR-604)。
//!
//! 書式だけを解く: `[keys.<コンテキスト>]` の表の「キー = 値」を、書かれた順の `KeymapEntry` の列にする。
//! キー名・コマンド・`when` の検証と層への組み込みは core が行う (コマンドの一覧は core が持つため)。
//! ファイルが無い・壊れているときも `KeymapFile` は返り、問題は `Diagnostic` の列として返す。

use toml::Spanned;
use toml::de::{DeString, DeTable, DeValue};

use crate::{DiagKind, Diagnostic, Severity};

/// 1 件の割当の動作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAction {
    /// コマンドを実行する。`has_args` は `args` が書かれていた (v0.2 は未対応)。
    Command {
        command: String,
        when: Option<String>,
        has_args: bool,
    },
    /// `"none"`: 下位の層の割当をこの層で遮蔽する。
    None,
}

/// `[keys.<context>]` の 1 行。配列値は書かれた順に複数の `KeymapEntry` になる。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeymapEntry {
    pub context: String,
    /// 書かれたとおりのキー (空白区切りでストロークの列)。
    pub keys: String,
    pub action: KeyAction,
    pub line: u32,
    pub column: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeymapFile {
    pub entries: Vec<KeymapEntry>,
    pub diagnostics: Vec<Diagnostic>,
}

fn pos(text: &str, offset: usize) -> (u32, u32) {
    let head = text.get(..offset.min(text.len())).unwrap_or("");
    let line = head.matches('\n').count() + 1;
    let col = head.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line as u32, col as u32)
}

struct Cx<'a> {
    text: &'a str,
    out: KeymapFile,
}

impl Cx<'_> {
    fn diag(&mut self, severity: Severity, kind: DiagKind, key: &str, at: usize) {
        let (line, column) = pos(self.text, at);
        self.out.diagnostics.push(Diagnostic {
            severity,
            kind,
            key: key.to_owned(),
            value: String::new(),
            line,
            column,
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
                has_args: false,
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
                KeyAction::Command {
                    command,
                    when: str_of("when"),
                    has_args: t.contains_key("args"),
                }
            }
            _ => {
                self.mismatch(&full, v, "string");
                return;
            }
        };
        let (line, column) = pos(self.text, at);
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
        text,
        out: KeymapFile::default(),
    };
    let root = match DeTable::parse(text) {
        Ok(t) => t,
        Err(e) => {
            let (line, column) = e.span().map_or((0, 0), |s| pos(text, s.start));
            cx.out.diagnostics.push(Diagnostic {
                severity: Severity::Error,
                kind: DiagKind::Syntax {
                    message: e.message().to_owned(),
                },
                key: String::new(),
                value: String::new(),
                line,
                column,
            });
            return cx.out;
        }
    };
    for (name, value) in root.get_ref() {
        cx.top(name, value);
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
                has_args: false
            }
        );
        assert_eq!(f5[1].context, "editor");
    }

    #[test]
    fn 構文エラーはファイル全体を無視し_診断を返す() {
        let f = load_keymap_str("[keys.editor\n");
        assert!(f.entries.is_empty());
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
        assert!(f.entries.is_empty());
    }

    #[test]
    fn args_は読み取って有りとだけ覚える() {
        let f =
            load_keymap_str("[keys.editor]\n\"f5\" = { command = \"x.y\", args = { a = 1 } }\n");
        assert!(matches!(
            f.entries[0].action,
            KeyAction::Command { has_args: true, .. }
        ));
    }
}
