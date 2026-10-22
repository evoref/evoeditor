//! ユーザー層の設定ファイル (`config.toml`) の読込と検証 (docs/07 §4.2・§5.0、01 FR-701〜703)。
//!
//! v0.1 の公開キーは 7 個。ファイルが無い・壊れている・値が不正なときも `Config` は必ず返り、
//! 問題は `Diagnostic` の列として返す (最初のエラーで止まらない)。不正な項目は既定値に戻す。

use toml::Spanned;
use toml::de::{DeString, DeTable, DeValue};

/// 診断の重さ。表示の文言は host / core が MessageKey で決める (docs/07 §4.3)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// 診断の種類 (docs/07 §4.2 の表)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagKind {
    /// TOML の構文エラー。ファイル全体を無視する。
    Syntax { message: String },
    /// 未知のキー・セクション。`suggestion` は編集距離 2 以下の公開キー。
    UnknownKey { suggestion: Option<String> },
    /// 型が違う。
    TypeMismatch {
        expected: &'static str,
        found: &'static str,
    },
    /// 範囲外 (整数)。
    OutOfRange { min: i64, max: i64 },
    /// 列挙値が不正。許容値を提示する。
    InvalidValue { allowed: &'static [&'static str] },
}

/// 設定の問題 1 件。`line` / `column` は 1 始まり (構文エラーで位置が無いときは 0)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub kind: DiagKind,
    /// ドット区切りのキー (`editor.tab_width`)。構文エラーでは空。
    pub key: String,
    /// 書かれた値 (文字列はそのまま、整数は書かれた表記。値を持たない診断では空)。
    pub value: String,
    pub line: u32,
    pub column: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndentUnit {
    Tab,
    Space,
}

/// 新規ファイル・判定不能時の文字コード (`file.default_encoding`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultEncoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    ShiftJis,
    EucJp,
    Iso2022Jp,
    /// OS ロケール由来のヒント (docs/05 §6)。
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultLineEnding {
    Lf,
    Crlf,
    Cr,
    Native,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    Auto,
    TrueColor,
    Color256,
    Color16,
    None,
}

/// v0.1 の公開キー 7 個 (docs/07 §5.0)。`Default` が既定値。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    pub tab_width: u8,
    pub indent_unit: IndentUnit,
    pub indent_width: u8,
    pub default_encoding: DefaultEncoding,
    pub default_line_ending: DefaultLineEnding,
    /// East Asian Ambiguous の幅 (1 か 2)。
    pub ambiguous_width: u8,
    pub color_mode: ColorMode,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            tab_width: 4,
            indent_unit: IndentUnit::Space,
            indent_width: 4,
            default_encoding: DefaultEncoding::Utf8,
            default_line_ending: DefaultLineEnding::Lf,
            ambiguous_width: 1,
            color_mode: ColorMode::Auto,
        }
    }
}

/// 読込の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub config: Config,
    pub diagnostics: Vec<Diagnostic>,
}

const SECTIONS: &[&str] = &["editor", "file", "view", "theme"];
const KEYS: &[&str] = &[
    "editor.tab_width",
    "editor.indent_unit",
    "editor.indent_width",
    "file.default_encoding",
    "file.default_line_ending",
    "view.ambiguous_width",
    "theme.color_mode",
];

const INDENT_UNITS: &[&str] = &["tab", "space"];
const ENCODINGS: &[&str] = &[
    "utf-8",
    "utf-8-bom",
    "utf-16le",
    "utf-16be",
    "shift_jis",
    "euc-jp",
    "iso-2022-jp",
    "system",
];
const LINE_ENDINGS: &[&str] = &["lf", "crlf", "cr", "native"];
const COLOR_MODES: &[&str] = &["auto", "truecolor", "256", "16", "none"];

/// 設定ファイルの本文を読む。`text` が空でも構文エラーでも `Config` を返す。
pub fn load_str(text: &str) -> Loaded {
    let mut cx = Cx {
        text,
        out: Loaded {
            config: Config::default(),
            diagnostics: Vec::new(),
        },
    };
    let root = match DeTable::parse(text) {
        Ok(t) => t,
        Err(e) => {
            let (line, column) = e.span().map_or((0, 0), |s| cx.pos(s.start));
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
        cx.section(name, value);
    }
    cx.out
}

struct Cx<'a> {
    text: &'a str,
    out: Loaded,
}

impl Cx<'_> {
    /// バイト位置 → (行, 桁)。どちらも 1 始まり、桁は文字数。
    fn pos(&self, offset: usize) -> (u32, u32) {
        let end = offset.min(self.text.len());
        let head = self.text.get(..end).unwrap_or("");
        let line = head.matches('\n').count() + 1;
        let col = head.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
        (line as u32, col as u32)
    }

    fn diag(&mut self, severity: Severity, kind: DiagKind, key: String, at: usize) {
        let (line, column) = self.pos(at);
        self.out.diagnostics.push(Diagnostic {
            severity,
            kind,
            key,
            value: String::new(),
            line,
            column,
        });
    }

    fn section(&mut self, name: &Spanned<DeString<'_>>, value: &Spanned<DeValue<'_>>) {
        let n: &str = name.get_ref();
        if !SECTIONS.contains(&n) {
            let suggestion = nearest(n, SECTIONS).map(str::to_owned);
            self.diag(
                Severity::Warning,
                DiagKind::UnknownKey { suggestion },
                n.to_owned(),
                name.span().start,
            );
            return;
        }
        let DeValue::Table(table) = value.get_ref() else {
            self.diag(
                Severity::Warning,
                DiagKind::TypeMismatch {
                    expected: "table",
                    found: value.get_ref().type_str(),
                },
                n.to_owned(),
                value.span().start,
            );
            return;
        };
        for (k, v) in table {
            let full = format!("{n}.{}", k.get_ref());
            if KEYS.contains(&full.as_str()) {
                self.value(&full, v);
            } else {
                let suggestion = nearest(&full, KEYS).map(str::to_owned);
                self.diag(
                    Severity::Warning,
                    DiagKind::UnknownKey { suggestion },
                    full,
                    k.span().start,
                );
            }
        }
    }

    fn value(&mut self, key: &str, v: &Spanned<DeValue<'_>>) {
        match key {
            "editor.tab_width" => {
                if let Some(n) = self.int(key, v, 1, 16) {
                    self.out.config.tab_width = n as u8;
                }
            }
            "editor.indent_width" => {
                if let Some(n) = self.int(key, v, 1, 16) {
                    self.out.config.indent_width = n as u8;
                }
            }
            "view.ambiguous_width" => {
                if let Some(n) = self.int(key, v, 1, 2) {
                    self.out.config.ambiguous_width = n as u8;
                }
            }
            "editor.indent_unit" => {
                if let Some(s) = self.string(key, v, INDENT_UNITS) {
                    self.out.config.indent_unit = match s.as_str() {
                        "tab" => IndentUnit::Tab,
                        _ => IndentUnit::Space,
                    };
                }
            }
            "file.default_encoding" => {
                let Some(s) = self.raw_string(key, v) else {
                    return;
                };
                match encoding(&s) {
                    Some(e) => self.out.config.default_encoding = e,
                    None => self.invalid(key, ENCODINGS, v),
                }
            }
            "file.default_line_ending" => {
                if let Some(s) = self.string(key, v, LINE_ENDINGS) {
                    self.out.config.default_line_ending = match s.as_str() {
                        "crlf" => DefaultLineEnding::Crlf,
                        "cr" => DefaultLineEnding::Cr,
                        "native" => DefaultLineEnding::Native,
                        _ => DefaultLineEnding::Lf,
                    };
                }
            }
            "theme.color_mode" => {
                if let Some(s) = self.string(key, v, COLOR_MODES) {
                    self.out.config.color_mode = match s.as_str() {
                        "truecolor" => ColorMode::TrueColor,
                        "256" => ColorMode::Color256,
                        "16" => ColorMode::Color16,
                        "none" => ColorMode::None,
                        _ => ColorMode::Auto,
                    };
                }
            }
            _ => {}
        }
    }

    fn mismatch(&mut self, key: &str, v: &Spanned<DeValue<'_>>, expected: &'static str) {
        self.diag(
            Severity::Warning,
            DiagKind::TypeMismatch {
                expected,
                found: v.get_ref().type_str(),
            },
            key.to_owned(),
            v.span().start,
        );
        self.set_value(v);
    }

    fn invalid(&mut self, key: &str, allowed: &'static [&'static str], v: &Spanned<DeValue<'_>>) {
        self.diag(
            Severity::Warning,
            DiagKind::InvalidValue { allowed },
            key.to_owned(),
            v.span().start,
        );
        self.set_value(v);
    }

    /// 直前に積んだ診断に、書かれた値を記録する。
    fn set_value(&mut self, v: &Spanned<DeValue<'_>>) {
        let text = match v.get_ref() {
            DeValue::String(s) => s.to_string(),
            DeValue::Integer(i) => i.as_str().to_owned(),
            DeValue::Float(f) => f.as_str().to_owned(),
            DeValue::Boolean(b) => b.to_string(),
            other => other.type_str().to_owned(),
        };
        if let Some(d) = self.out.diagnostics.last_mut() {
            d.value = text;
        }
    }

    /// 範囲 `min..=max` の整数。型違い・範囲外は診断を積んで `None` (呼び出し側は既定値のまま)。
    fn int(&mut self, key: &str, v: &Spanned<DeValue<'_>>, min: i64, max: i64) -> Option<i64> {
        let DeValue::Integer(i) = v.get_ref() else {
            self.mismatch(key, v, "integer");
            return None;
        };
        let Ok(n) = i64::from_str_radix(&i.as_str().replace('_', ""), i.radix()) else {
            self.diag(
                Severity::Warning,
                DiagKind::OutOfRange { min, max },
                key.to_owned(),
                v.span().start,
            );
            self.set_value(v);
            return None;
        };
        if (min..=max).contains(&n) {
            Some(n)
        } else {
            self.diag(
                Severity::Warning,
                DiagKind::OutOfRange { min, max },
                key.to_owned(),
                v.span().start,
            );
            self.set_value(v);
            None
        }
    }

    fn raw_string(&mut self, key: &str, v: &Spanned<DeValue<'_>>) -> Option<String> {
        if let DeValue::String(s) = v.get_ref() {
            Some(s.to_string())
        } else {
            self.mismatch(key, v, "string");
            None
        }
    }

    /// 許容値のどれかに一致する文字列。
    fn string(
        &mut self,
        key: &str,
        v: &Spanned<DeValue<'_>>,
        allowed: &'static [&'static str],
    ) -> Option<String> {
        let s = self.raw_string(key, v)?;
        if allowed.contains(&s.as_str()) {
            Some(s)
        } else {
            self.invalid(key, allowed, v);
            None
        }
    }
}

/// `file.default_encoding` の値。WHATWG の別名ラベルは正規名に寄せる (docs/07 §5.2)。
fn encoding(s: &str) -> Option<DefaultEncoding> {
    Some(match s.to_ascii_lowercase().as_str() {
        "utf-8" | "utf8" | "unicode-1-1-utf-8" => DefaultEncoding::Utf8,
        "utf-8-bom" => DefaultEncoding::Utf8Bom,
        "utf-16le" | "utf-16" | "ucs-2" | "unicode" | "csunicode" => DefaultEncoding::Utf16Le,
        "utf-16be" | "unicodefffe" => DefaultEncoding::Utf16Be,
        "shift_jis" | "shift-jis" | "sjis" | "windows-31j" | "ms_kanji" | "csshiftjis"
        | "x-sjis" | "ms932" | "cp932" => DefaultEncoding::ShiftJis,
        "euc-jp" | "x-euc-jp" | "cseucpkdfmtjapanese" => DefaultEncoding::EucJp,
        "iso-2022-jp" | "csiso2022jp" => DefaultEncoding::Iso2022Jp,
        "system" => DefaultEncoding::System,
        _ => return None,
    })
}

/// 公開キーの既定値の表記 (診断の「既定値 {default} を使います」に入れる)。未知のキーは空。
pub fn default_display(key: &str) -> &'static str {
    match key {
        "editor.tab_width" | "editor.indent_width" => "4",
        "editor.indent_unit" => "space",
        "file.default_encoding" => "utf-8",
        "file.default_line_ending" => "lf",
        "view.ambiguous_width" => "1",
        "theme.color_mode" => "auto",
        _ => "",
    }
}

/// 編集距離 2 以下で最も近い候補 (同点は先頭)。
fn nearest<'a>(name: &str, candidates: &[&'a str]) -> Option<&'a str> {
    candidates
        .iter()
        .map(|c| (distance(name, c), *c))
        .filter(|(d, _)| *d <= 2)
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}

fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(ca != cb);
            cur.push(sub.min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests;
