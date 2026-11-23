//! `.editorconfig` の読み方と重ね方 (docs/07 §10.1・§10.3 の EC-3〜EC-6、01 FR-705 AC-4)。
//!
//! - 読み方 ([`parse_editorconfig`]、EC-3): 行ごとに前後の空白を落とし、空行と `#` / `;` で始まる行を飛ばす。`[...]` は
//!   セクション (中身が glob、EC-4)、`=` を含む行は `キー = 値` (キーは小文字、値は大文字小文字を区別しない)。最初の
//!   セクションの前は前置きで、`root` だけを読む。長さの上限を超えた行は飛ばし、10,000 行・1,000 セクションで止める。
//! - 診断 (EC-6): 写すプロパティの誤った値 (`msg.editorconfig.invalid_value`)・写せない `charset`
//!   (`msg.editorconfig.charset_unmapped`、情報)・v0.5 で写さないプロパティ (未対応の情報診断)・資源の上限を超える glob
//!   (`msg.editorconfig.bad_glob`)。当たるかに依らず、ファイルの中身だけで決まる (同じ中身なら同じ診断)。
//! - 重ね方 ([`resolve_editorconfig`]、EC-5): 遠いファイルから近いファイルの順に、各ファイルの中は書いた順に、当たる
//!   セクションのプロパティを重ねる (後が勝つ。`unset` は取り消す)。`tab_width` を書いていなくて `indent_size` が整数なら、
//!   その値を `tab_width` にも使う。結果は 07 §3.6 の LAYER-1 の (5) の段 ([`EditorConfigValues`])。
//!
//! ファイルを集めるのは eve-core の `ReadEditorConfig` のジョブ (所有者の検査、EC-2)。
//!
//! spec: 07 §10.1, 07 §10.3

use crate::glob::{Glob, GlobCase, GlobError};
use crate::{DefaultEncoding, DefaultLineEnding, DiagKind, Diagnostic, IndentUnit, Severity};

/// ファイルの名前。
pub const EDITORCONFIG_FILE: &str = ".editorconfig";
/// 1 つのファイルの大きさの上限 (バイト。EC-2)。
pub const EDITORCONFIG_FILE_MAX_BYTES: u64 = 256 * 1024;
/// 1 つの文書で読むファイルの大きさの合計の上限 (バイト。EC-2)。
pub const EDITORCONFIG_TOTAL_MAX_BYTES: u64 = 1024 * 1024;
/// 1 つの文書で読むセクションの合計の上限 (EC-2)。
pub const EDITORCONFIG_TOTAL_MAX_SECTIONS: usize = 2_000;
/// 上へ探す段数の上限 (EC-2)。
pub const EDITORCONFIG_MAX_DEPTH: usize = 64;
/// セクションの glob の長さの上限 (バイト。超えた行は飛ばす、EC-3)。
pub const EDITORCONFIG_GLOB_MAX_BYTES: usize = 4_096;
/// キーの長さの上限 (バイト。EC-3)。
pub const EDITORCONFIG_KEY_MAX_BYTES: usize = 1_024;
/// 値の長さの上限 (バイト。EC-3)。
pub const EDITORCONFIG_VALUE_MAX_BYTES: usize = 4_096;
/// 1 つのファイルで読む行の上限 (EC-3)。
pub const EDITORCONFIG_FILE_MAX_LINES: usize = 10_000;
/// 1 つのファイルで読むセクションの上限 (EC-3)。
pub const EDITORCONFIG_FILE_MAX_SECTIONS: usize = 1_000;
/// `indent_size`・`tab_width` の範囲の上限 (§10.1。`editor.tab_width` と同じ 1〜16)。
const WIDTH_MAX: u8 = 16;

/// ファイルを読まなかった理由 (`msg.editorconfig.too_large` の `{reason}`、EC-2)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotReadReason {
    /// 256 KiB を超える。
    TooLarge,
    /// UTF-8 でない。
    NotUtf8,
    /// 1 つの文書で読む量 (1 MiB・2,000 セクション) の上限を超えた (このファイルとより上のファイルを使わない)。
    TotalLimit,
}

impl NotReadReason {
    /// 理由の語のキーの末尾 (`ui.editorconfig.reason.<これ>`)。
    #[must_use]
    pub fn reason_name(self) -> &'static str {
        match self {
            NotReadReason::TooLarge => "too_large",
            NotReadReason::NotUtf8 => "not_utf8",
            NotReadReason::TotalLimit => "total_limit",
        }
    }
}

/// `.editorconfig` の診断の種類 (`msg.editorconfig.*`、07 §10.1 / §10.3 の EC-6)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorConfigProblem {
    /// 写すプロパティの誤った値 (範囲の外・知らない値。警告。その値だけを使わない)。`Diagnostic.key` はプロパティの名前、
    /// `value` は書かれた値。
    InvalidValue {
        /// 許される値 (診断に出す順)。
        allowed: Vec<String>,
    },
    /// 写せない `charset` (`latin1` と仕様に無い値。情報)。`unset` と同じく、新しいファイルは `.editorconfig` より下の段の
    /// 文字コードで作る。
    CharsetUnmapped,
    /// ファイルを読まなかった (警告)。`Diagnostic.value` はファイルのパス。
    NotRead(NotReadReason),
    /// 資源の上限 (`{` の入れ子 8 段・選択肢 1,024 個) を超えるセクションの glob (警告。そのセクションを使わない)。
    /// `Diagnostic.key` は glob。
    BadGlob(GlobError),
}

impl EditorConfigProblem {
    /// 文言のキーの名前 (`msg.editorconfig.<これ>`)。ログの種類の名前にも使う。
    #[must_use]
    pub fn message_name(&self) -> &'static str {
        match self {
            EditorConfigProblem::InvalidValue { .. } => "invalid_value",
            EditorConfigProblem::CharsetUnmapped => "charset_unmapped",
            EditorConfigProblem::NotRead(_) => "too_large",
            EditorConfigProblem::BadGlob(_) => "bad_glob",
        }
    }
}

/// `indent_size` の値 (§10.1)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EcWidth {
    /// 整数の幅。
    Width(u8),
    /// `tab`: `tab_width` (無ければ下の段の `editor.tab_width`) の値を幅にする。
    Tab,
}

/// `.editorconfig` で決まった値を書いた所 (`config.show` の由来、07 §3.5)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EcOrigin {
    /// `.editorconfig` の絶対パス (表示名。一覧の `Enter` で開く)。
    pub file: String,
    /// プロパティを書いた行 (1 始まり)。
    pub line: u32,
    /// プロパティを書いた桁 (1 始まりの文字数)。
    pub column: u32,
    /// 当たったセクションの glob (`[*.py]` の `*.py`)。
    pub section: String,
}

/// 1 つの文書に当たる `.editorconfig` の値 (07 §3.6 の LAYER-1 の (5)。写像した後のもの)。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditorConfigValues {
    /// `indent_style` → `editor.indent_unit`。
    pub indent_unit: Option<(IndentUnit, EcOrigin)>,
    /// `indent_size` → `editor.indent_width`。
    pub indent_width: Option<(EcWidth, EcOrigin)>,
    /// `tab_width` (書いていなくて `indent_size` が整数なら、その値) → `editor.tab_width`。
    pub tab_width: Option<(u8, EcOrigin)>,
    /// `end_of_line` → `file.default_line_ending` (新しいファイルだけ、LAYER-4)。
    pub line_ending: Option<(DefaultLineEnding, EcOrigin)>,
    /// `charset` → `file.default_encoding` (新しいファイルだけ、LAYER-4)。
    pub encoding: Option<(DefaultEncoding, EcOrigin)>,
}

impl EditorConfigValues {
    /// どのキーも決めていない。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.indent_unit.is_none()
            && self.indent_width.is_none()
            && self.tab_width.is_none()
            && self.line_ending.is_none()
            && self.encoding.is_none()
    }
}

/// 写すプロパティ (§10.1)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    IndentStyle,
    IndentSize,
    TabWidth,
    EndOfLine,
    Charset,
}

/// 写すプロパティの、読んだ値。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Value {
    /// `unset` (それまでに重ねた値を取り消す)。写せない `charset` も同じ扱い。
    Unset,
    IndentStyle(IndentUnit),
    IndentSize(EcWidth),
    TabWidth(u8),
    EndOfLine(DefaultLineEnding),
    Charset(DefaultEncoding),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Prop {
    key: Key,
    value: Value,
    line: u32,
    column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Section {
    /// 書いた glob (`[` と `]` の中)。
    pub(crate) pattern: String,
    /// 組んだ glob。組めない (資源の上限) か、長すぎて飛ばした見出しなら `None` (そのセクションは使わない)。
    pub(crate) glob: Option<Glob>,
    props: Vec<Prop>,
}

/// 読んだ `.editorconfig` 1 つ。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditorConfigFile {
    root: bool,
    pub(crate) sections: Vec<Section>,
    diagnostics: Vec<Diagnostic>,
}

impl EditorConfigFile {
    /// 前置きに `root = true` がある (ここで探索を止める、§10.1)。
    #[must_use]
    pub fn root(&self) -> bool {
        self.root
    }

    /// 読んだセクションの数 (1 つの文書の合計の上限、EC-2)。
    #[must_use]
    pub fn section_count(&self) -> usize {
        self.sections.len()
    }

    /// 診断 (書いた所の順)。
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// 組めたセクションの glob (照合の手数を確かめる fuzz の性質、09 §3.3)。
    pub fn globs(&self) -> impl Iterator<Item = &Glob> {
        self.sections.iter().filter_map(|s| s.glob.as_ref())
    }
}

/// 1 行の位置の診断。
fn diag(
    severity: Severity,
    problem: EditorConfigProblem,
    key: &str,
    value: &str,
    line: u32,
    column: u32,
) -> Diagnostic {
    Diagnostic {
        severity,
        kind: DiagKind::EditorConfig(problem),
        key: key.to_owned(),
        value: value.to_owned(),
        line,
        column,
        condition: None,
    }
}

/// 読まなかったファイルの診断 (位置なし。`path` はファイルのパス)。
#[must_use]
pub fn not_read_diagnostic(reason: NotReadReason, path: &str) -> Diagnostic {
    diag(
        Severity::Warning,
        EditorConfigProblem::NotRead(reason),
        "",
        path,
        0,
        0,
    )
}

/// セクションの glob を組む (EC-4): `/` を含まなければ、どの深さのファイル名にも当てる (`**/` を前に付けたのと同じ)。
fn section_glob(pattern: &str) -> Result<Glob, GlobError> {
    let g = Glob::new(pattern, GlobCase::Sensitive)?;
    if g.has_slash() {
        Ok(g)
    } else {
        Glob::new(&format!("**/{pattern}"), GlobCase::Sensitive)
    }
}

/// 1 から 16 の整数 (先頭の `+`・符号・空白を許さない。§10.1)。
fn width(v: &str) -> Option<u8> {
    if v.is_empty() || v.len() > 3 || !v.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    v.parse::<u8>().ok().filter(|n| (1..=WIDTH_MAX).contains(n))
}

/// 写すプロパティの値を読む。`Err` は誤った値の許される値 (`charset` の写せない値は `Ok(None)`)。
fn read_value(key: Key, v: &str) -> Result<Option<Value>, Vec<&'static str>> {
    let lower = v.to_ascii_lowercase();
    if lower == "unset" {
        return Ok(Some(Value::Unset));
    }
    let widths = "1〜16";
    match key {
        Key::IndentStyle => IndentUnit::from_name(&lower)
            .map(|u| Some(Value::IndentStyle(u)))
            .ok_or_else(|| vec!["tab", "space", "unset"]),
        Key::IndentSize => match lower.as_str() {
            "tab" => Ok(Some(Value::IndentSize(EcWidth::Tab))),
            _ => width(&lower)
                .map(|n| Some(Value::IndentSize(EcWidth::Width(n))))
                .ok_or_else(|| vec![widths, "tab", "unset"]),
        },
        Key::TabWidth => width(&lower)
            .map(|n| Some(Value::TabWidth(n)))
            .ok_or_else(|| vec![widths, "unset"]),
        Key::EndOfLine => match lower.as_str() {
            "lf" => Ok(Some(Value::EndOfLine(DefaultLineEnding::Lf))),
            "crlf" => Ok(Some(Value::EndOfLine(DefaultLineEnding::Crlf))),
            "cr" => Ok(Some(Value::EndOfLine(DefaultLineEnding::Cr))),
            _ => Err(vec!["lf", "crlf", "cr", "unset"]),
        },
        Key::Charset => Ok(match lower.as_str() {
            "utf-8" => Some(Value::Charset(DefaultEncoding::Utf8)),
            "utf-8-bom" => Some(Value::Charset(DefaultEncoding::Utf8Bom)),
            "utf-16le" => Some(Value::Charset(DefaultEncoding::Utf16Le)),
            "utf-16be" => Some(Value::Charset(DefaultEncoding::Utf16Be)),
            _ => None,
        }),
    }
}

fn key_of(name: &str) -> Option<Key> {
    Some(match name {
        "indent_style" => Key::IndentStyle,
        "indent_size" => Key::IndentSize,
        "tab_width" => Key::TabWidth,
        "end_of_line" => Key::EndOfLine,
        "charset" => Key::Charset,
        _ => return None,
    })
}

/// v0.5 で写さないプロパティ (未対応の情報診断。§10.1)。
const UNSUPPORTED_KEYS: [&str; 3] = [
    "trim_trailing_whitespace",
    "insert_final_newline",
    "max_line_length",
];

/// 行の中の位置 (1 始まりの文字数)。
fn column_of(raw: &str, part: &str) -> u32 {
    let offset = part.as_ptr() as usize - raw.as_ptr() as usize;
    let n = raw.get(..offset).map_or(0, |h| h.chars().count()) + 1;
    u32::try_from(n).unwrap_or(u32::MAX)
}

thread_local! {
    /// この thread で `.editorconfig` を読んだ・重ねた回数 ([`editorconfig_work_on_this_thread`])。
    static WORK: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// この thread で `.editorconfig` を読んだ ([`parse_editorconfig`])・重ねた ([`resolve_editorconfig`]) 回数。試験で、主スレッドが
/// ジョブの結果を当てるときに読み・照合をしないこと (EC-1・EC-8) を確かめるのに使う。
#[doc(hidden)]
#[must_use]
pub fn editorconfig_work_on_this_thread() -> u64 {
    WORK.with(std::cell::Cell::get)
}

fn count_work() {
    WORK.with(|w| w.set(w.get().wrapping_add(1)));
}

/// `.editorconfig` の本文を読む (EC-3)。先頭の BOM は飛ばす。
#[must_use]
pub fn parse_editorconfig(text: &str) -> EditorConfigFile {
    count_work();
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut out = EditorConfigFile::default();
    let mut in_section = false;
    for (i, raw) in text
        .split('\n')
        .take(EDITORCONFIG_FILE_MAX_LINES)
        .enumerate()
    {
        let line_no = u32::try_from(i + 1).unwrap_or(u32::MAX);
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.len() >= 2 && line.starts_with('[') && line.ends_with(']') {
            if out.sections.len() >= EDITORCONFIG_FILE_MAX_SECTIONS {
                break;
            }
            in_section = true;
            let pattern = &line[1..line.len() - 1];
            // 長すぎる見出しは飛ばす。続くプロパティを前のセクションに付けないよう、使わないセクションにする
            let glob = if pattern.len() > EDITORCONFIG_GLOB_MAX_BYTES {
                None
            } else {
                match section_glob(pattern) {
                    Ok(g) => Some(g),
                    Err(e) => {
                        out.diagnostics.push(diag(
                            Severity::Warning,
                            EditorConfigProblem::BadGlob(e),
                            pattern,
                            "",
                            line_no,
                            column_of(raw, line),
                        ));
                        None
                    }
                }
            };
            out.sections.push(Section {
                pattern: if glob.is_some() {
                    pattern.to_owned()
                } else {
                    String::new()
                },
                glob,
                props: Vec::new(),
            });
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let (k, v) = (k.trim(), v.trim());
        if k.len() > EDITORCONFIG_KEY_MAX_BYTES || v.len() > EDITORCONFIG_VALUE_MAX_BYTES {
            continue;
        }
        let name = k.to_ascii_lowercase();
        let column = column_of(raw, k);
        if !in_section {
            // 前置きは `root` だけ (true / false。大文字小文字を区別しない)
            if name == "root" {
                match v.to_ascii_lowercase().as_str() {
                    "true" => out.root = true,
                    "false" => out.root = false,
                    _ => out.diagnostics.push(diag(
                        Severity::Warning,
                        EditorConfigProblem::InvalidValue {
                            allowed: vec!["true".to_owned(), "false".to_owned()],
                        },
                        &name,
                        v,
                        line_no,
                        column,
                    )),
                }
            }
            continue;
        }
        if UNSUPPORTED_KEYS.contains(&name.as_str()) {
            out.diagnostics.push(Diagnostic {
                severity: Severity::Info,
                kind: DiagKind::Unsupported,
                key: name,
                value: String::new(),
                line: line_no,
                column,
                condition: None,
            });
            continue;
        }
        let Some(key) = key_of(&name) else {
            continue;
        };
        let value = match read_value(key, v) {
            Ok(Some(value)) => value,
            Ok(None) => {
                out.diagnostics.push(diag(
                    Severity::Info,
                    EditorConfigProblem::CharsetUnmapped,
                    &name,
                    v,
                    line_no,
                    column,
                ));
                Value::Unset
            }
            Err(allowed) => {
                out.diagnostics.push(diag(
                    Severity::Warning,
                    EditorConfigProblem::InvalidValue {
                        allowed: allowed.into_iter().map(str::to_owned).collect(),
                    },
                    &name,
                    v,
                    line_no,
                    column,
                ));
                continue;
            }
        };
        if let Some(s) = out.sections.last_mut() {
            s.props.push(Prop {
                key,
                value,
                line: line_no,
                column,
            });
        }
    }
    out
}

/// 重ねるファイル 1 つ: (表示名 (絶対パス), 読んだファイル, そのファイルのあるディレクトリから文書への相対パス (区切りは `/`))。
pub type EditorConfigSource<'a> = (&'a str, &'a EditorConfigFile, &'a str);

/// 当たるセクションのプロパティを重ねる (EC-5)。`files` は遠い (上の) ファイルから近いファイルの順。
#[must_use]
pub fn resolve_editorconfig(files: &[EditorConfigSource<'_>]) -> EditorConfigValues {
    let mut steps = 0;
    resolve_editorconfig_counted(files, &mut steps)
}

/// [`resolve_editorconfig`] と同じだが、glob の照合の手数を `steps` に足す (fuzz の性質、09 §3.3)。
pub fn resolve_editorconfig_counted(
    files: &[EditorConfigSource<'_>],
    steps: &mut usize,
) -> EditorConfigValues {
    count_work();
    let mut slots: [Option<(Value, EcOrigin)>; 5] = Default::default();
    let slot = |k: Key| match k {
        Key::IndentStyle => 0,
        Key::IndentSize => 1,
        Key::TabWidth => 2,
        Key::EndOfLine => 3,
        Key::Charset => 4,
    };
    for (name, file, rel) in files {
        for s in &file.sections {
            let Some(g) = &s.glob else {
                continue;
            };
            if !g.is_match_counted(rel, steps) {
                continue;
            }
            for p in &s.props {
                slots[slot(p.key)] = match p.value {
                    Value::Unset => None,
                    v => Some((
                        v,
                        EcOrigin {
                            file: (*name).to_owned(),
                            line: p.line,
                            column: p.column,
                            section: s.pattern.clone(),
                        },
                    )),
                };
            }
        }
    }
    let [style, size, tab, eol, charset] = slots;
    let indent_width = size.and_then(|(v, o)| match v {
        Value::IndentSize(w) => Some((w, o)),
        _ => None,
    });
    let tab_width = match tab {
        Some((Value::TabWidth(n), o)) => Some((n, o)),
        // 書いていなくて `indent_size` が整数なら、その値 (仕様どおり)
        _ => match &indent_width {
            Some((EcWidth::Width(n), o)) => Some((*n, o.clone())),
            _ => None,
        },
    };
    EditorConfigValues {
        indent_unit: style.and_then(|(v, o)| match v {
            Value::IndentStyle(u) => Some((u, o)),
            _ => None,
        }),
        indent_width,
        tab_width,
        line_ending: eol.and_then(|(v, o)| match v {
            Value::EndOfLine(e) => Some((e, o)),
            _ => None,
        }),
        encoding: charset.and_then(|(v, o)| match v {
            Value::Charset(e) => Some((e, o)),
            _ => None,
        }),
    }
}

#[cfg(test)]
mod tests;
