//! `[[language]]` の読込 (docs/07 §9.6 の LANG-2〜LANG-4): `config.toml` (と `--config` のファイル) の `[[language]]` と、
//! 言語定義のファイル (`languages/*.toml`) を [`LanguageDef`] にする。誤りの項目は使わずに診断にし (下の層の値を使う)、
//! v0.5 で効かない項目は「このバージョンでは未対応」の情報診断にする。

use toml::Spanned;
use toml::de::{DeTable, DeValue};

use super::{Cx, DiagKind, Diagnostic, IndentUnit, Loaded, Severity, nearest};
use crate::language::{LangValue, LanguageDef, LanguageProblem, ListValue, is_language_name};

/// `display_name` の上限 (文字)。
const DISPLAY_NAME_MAX_CHARS: usize = 64;
/// `first_line` と配列の要素の上限 (バイト)。
const ITEM_MAX_BYTES: usize = 256;
/// 配列の要素の数の上限。
const LIST_MAX_ITEMS: usize = 64;
/// `comment.line` の上限 (バイト)。
const COMMENT_MAX_BYTES: usize = 16;

/// v0.5 で効かない項目 (LANG-4)。書いても起動を妨げず、項目ごとに「このバージョンでは未対応」の情報診断にする。
const UNSUPPORTED: &[&str] = &[
    "auto_pairs",
    "word_separators",
    "grammar",
    "highlight_query",
    "language_servers",
    "roots",
    "formatter",
    "format_on_save",
    "max_line_length",
];

/// 定義の中のキー (綴りの候補に使う)。
const KEYS: &[&str] = &[
    "language.name",
    "language.display_name",
    "language.file_types",
    "language.file_names",
    "language.file_globs",
    "language.first_line",
    "language.indent",
    "language.comment",
    "language.auto_pairs",
    "language.word_separators",
    "language.grammar",
    "language.highlight_query",
    "language.language_servers",
    "language.roots",
    "language.formatter",
    "language.format_on_save",
    "language.max_line_length",
];

const INDENT_KEYS: &[&str] = &[
    "language.indent.unit",
    "language.indent.width",
    "language.indent.tab_width",
];

const COMMENT_KEYS: &[&str] = &["language.comment.line", "language.comment.block"];

/// 言語定義のファイル (`languages/*.toml`) の本文を読む。`source` は診断と由来に出す表示名 (`languages/<file>.toml`)。
/// 1 つのファイルに `[[language]]` を幾つ書いてもよい (LANG-2)。ほかの表・キーは未知のキーの警告 (条件付きセクションも、
/// 07 §3.4 の COND-1)。構文エラーはファイル全体を使わない (エラー)。
#[must_use]
pub fn load_language_str(source: &str, text: &str) -> (Vec<LanguageDef>, Vec<Diagnostic>) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut cx = Cx {
        text,
        out: Loaded {
            config: super::Config::default(),
            diagnostics: Vec::new(),
            written: Vec::new(),
            languages: Vec::new(),
        },
        conditions: crate::Conditions::default(),
        apply: true,
        condition: None,
    };
    if let Some(d) = crate::toml_scan::structure_diagnostic(text) {
        return (Vec::new(), vec![d]);
    }
    let root = match DeTable::parse(text) {
        Ok(t) => t,
        Err(e) => {
            let (line, column) = e.span().map_or((0, 0), |s| cx.pos(s.start));
            let d = Diagnostic {
                severity: Severity::Error,
                kind: DiagKind::Syntax {
                    message: e.message().to_owned(),
                },
                key: String::new(),
                value: String::new(),
                line,
                column,
                condition: None,
            };
            return (Vec::new(), vec![d]);
        }
    };
    for (name, value) in root.get_ref() {
        let n: &str = name.get_ref();
        if n == "language" {
            cx.languages(value);
        } else {
            let suggestion = nearest(n, &["language"]).map(str::to_owned);
            cx.diag(
                Severity::Warning,
                DiagKind::UnknownKey { suggestion },
                n.to_owned(),
                name.span().start,
            );
        }
    }
    let mut defs = cx.out.languages;
    for d in &mut defs {
        d.source = source.to_owned();
    }
    (defs, cx.out.diagnostics)
}

impl Cx<'_> {
    /// `[[language]]` (表の配列) を読み、使える定義を `out.languages` に足す。
    pub(super) fn languages(&mut self, value: &Spanned<DeValue<'_>>) {
        let DeValue::Array(items) = value.get_ref() else {
            self.mismatch("language", value, "array");
            return;
        };
        for item in items {
            let DeValue::Table(t) = item.get_ref() else {
                self.mismatch("language", item, "table");
                continue;
            };
            if let Some(def) = self.language_def(t, item.span().start) {
                self.out.languages.push(def);
            }
        }
    }

    /// `[[language]]` の 1 つ。`name` が無い・使えなければ、ほかの項目も検証して診断を出し、定義は使わない (LANG-3)。
    fn language_def(&mut self, t: &DeTable<'_>, at: usize) -> Option<LanguageDef> {
        let mut def = LanguageDef::default();
        let mut name: Option<bool> = None;
        for (k, v) in t {
            let key: &str = k.get_ref();
            let full = format!("language.{key}");
            let kat = k.span().start;
            match key {
                "name" => {
                    let ok = self.raw_string(&full, v).is_some_and(|s| {
                        if is_language_name(&s) {
                            (def.line, def.column) = self.pos(kat);
                            def.name = s;
                            true
                        } else {
                            self.lang_problem(LanguageProblem::InvalidName, &full, v);
                            false
                        }
                    });
                    name = Some(ok);
                }
                "display_name" => {
                    if let Some(s) = self.raw_string(&full, v) {
                        if s.chars().count() > DISPLAY_NAME_MAX_CHARS {
                            self.lang_problem(LanguageProblem::TooLong, &full, v);
                        } else {
                            def.display_name = Some(self.lang_value(s, kat));
                        }
                    }
                }
                "file_types" | "file_names" | "file_globs" | "file_types+" | "file_names+"
                | "file_globs+" => {
                    let Some(items) = self.string_list(&full, v) else {
                        continue;
                    };
                    // 拡張子に先頭の `.` を書いた (`.rs`): 当たらないので知らせる (値は書いたとおり使う)
                    if key.starts_with("file_types")
                        && let Some(dot) = items.iter().find(|t| t.starts_with('.'))
                    {
                        let dot = dot.clone();
                        self.diag(
                            Severity::Warning,
                            DiagKind::Language(LanguageProblem::FileTypeDot),
                            full.clone(),
                            v.span().start,
                        );
                        if let Some(d) = self.out.diagnostics.last_mut() {
                            d.value = dot;
                        }
                    }
                    let value = Some(self.lang_value(items, kat));
                    let list: &mut ListValue = match key.trim_end_matches('+') {
                        "file_types" => &mut def.file_types,
                        "file_names" => &mut def.file_names,
                        _ => &mut def.file_globs,
                    };
                    if key.ends_with('+') {
                        list.add = value;
                    } else {
                        list.replace = value;
                    }
                }
                "first_line" => {
                    if let Some(s) = self.raw_string(&full, v) {
                        if s.len() > ITEM_MAX_BYTES {
                            self.lang_problem(LanguageProblem::TooLong, &full, v);
                        } else {
                            def.first_line = Some(self.lang_value(s, kat));
                        }
                    }
                }
                "indent" => self.language_indent(&full, v, &mut def),
                "comment" => self.language_comment(&full, v, &mut def),
                k if UNSUPPORTED.contains(&k) => {
                    self.diag(Severity::Info, DiagKind::Unsupported, full, kat);
                }
                _ => {
                    let suggestion = nearest(&full, KEYS).map(str::to_owned);
                    self.diag(
                        Severity::Warning,
                        DiagKind::UnknownKey { suggestion },
                        full,
                        kat,
                    );
                }
            }
        }
        match name {
            Some(true) => Some(def),
            Some(false) => None,
            None => {
                self.diag(
                    Severity::Warning,
                    DiagKind::Language(LanguageProblem::MissingName),
                    "language.name".to_owned(),
                    at,
                );
                None
            }
        }
    }

    /// `indent = { unit, width, tab_width }`。
    fn language_indent(&mut self, full: &str, v: &Spanned<DeValue<'_>>, def: &mut LanguageDef) {
        let DeValue::Table(t) = v.get_ref() else {
            self.mismatch(full, v, "table");
            return;
        };
        for (k, iv) in t {
            let key = format!("{full}.{}", k.get_ref());
            let kat = k.span().start;
            match k.get_ref().as_ref() {
                "unit" => {
                    if let Some(u) = self
                        .string(&key, iv, IndentUnit::NAMES)
                        .and_then(|s| IndentUnit::from_name(&s))
                    {
                        def.indent_unit = Some(self.lang_value(u, kat));
                    }
                }
                "width" | "tab_width" => {
                    let Some(n) = self.int(&key, iv, 1, 16) else {
                        continue;
                    };
                    let n = u8::try_from(n).expect("1〜16 は u8 に収まる");
                    let value = Some(self.lang_value(n, kat));
                    if k.get_ref() == "width" {
                        def.indent_width = value;
                    } else {
                        def.tab_width = value;
                    }
                }
                _ => {
                    let suggestion = nearest(&key, INDENT_KEYS).map(str::to_owned);
                    self.diag(
                        Severity::Warning,
                        DiagKind::UnknownKey { suggestion },
                        key,
                        kat,
                    );
                }
            }
        }
    }

    /// `comment = { line, block }`。`block` は v0.5 では効かない (LANG-4)。
    fn language_comment(&mut self, full: &str, v: &Spanned<DeValue<'_>>, def: &mut LanguageDef) {
        let DeValue::Table(t) = v.get_ref() else {
            self.mismatch(full, v, "table");
            return;
        };
        for (k, iv) in t {
            let key = format!("{full}.{}", k.get_ref());
            let kat = k.span().start;
            match k.get_ref().as_ref() {
                "line" => {
                    if let Some(s) = self.raw_string(&key, iv) {
                        let ok = (1..=COMMENT_MAX_BYTES).contains(&s.len())
                            && !s.chars().any(|c| c.is_whitespace() || c.is_control());
                        if ok {
                            def.comment_line = Some(self.lang_value(s, kat));
                        } else {
                            self.lang_problem(LanguageProblem::BadComment, &key, iv);
                        }
                    }
                }
                "block" => self.diag(Severity::Info, DiagKind::Unsupported, key, kat),
                _ => {
                    let suggestion = nearest(&key, COMMENT_KEYS).map(str::to_owned);
                    self.diag(
                        Severity::Warning,
                        DiagKind::UnknownKey { suggestion },
                        key,
                        kat,
                    );
                }
            }
        }
    }

    /// 文字列の配列 (64 要素・1 要素 256 バイトまで)。誤りは診断を積んで `None` (項目を使わない)。
    fn string_list(&mut self, key: &str, v: &Spanned<DeValue<'_>>) -> Option<Vec<String>> {
        let DeValue::Array(items) = v.get_ref() else {
            self.mismatch(key, v, "array");
            return None;
        };
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            let DeValue::String(s) = item.get_ref() else {
                self.mismatch(key, item, "string");
                return None;
            };
            out.push(s.to_string());
        }
        if out.len() > LIST_MAX_ITEMS || out.iter().any(|s| s.len() > ITEM_MAX_BYTES) {
            self.lang_problem(LanguageProblem::TooLong, key, v);
            return None;
        }
        Some(out)
    }

    fn lang_value<T>(&self, value: T, at: usize) -> LangValue<T> {
        let (line, column) = self.pos(at);
        LangValue {
            value,
            line,
            column,
        }
    }

    /// 言語定義の誤りの警告 (書かれた値を記録する)。
    fn lang_problem(&mut self, p: LanguageProblem, key: &str, v: &Spanned<DeValue<'_>>) {
        self.diag(
            Severity::Warning,
            DiagKind::Language(p),
            key.to_owned(),
            v.span().start,
        );
        self.set_value(v);
    }
}
