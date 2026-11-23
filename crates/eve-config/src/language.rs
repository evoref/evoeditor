//! 言語定義 (docs/07 §9.6 の LANG-1〜LANG-9、01 FR-704): 組込みの表、層ごとの定義の併合、言語の判定、文書ごとに決まる
//! 値 (§3.6 の LAYER-1)、`--dump-languages` の出力。
//!
//! 組込みの定義は静的な表 ([`BUILTIN_LANGUAGES`]。正本は 07 §9.6.1) で持ち、起動時に TOML を解析しない (LANG-1)。
//! ユーザー層の定義 (`config.toml` の `[[language]]` と `languages/*.toml`) は読込 (`config.rs`) が [`LanguageDef`] にし、
//! [`LanguageSet::build`] が組込みの表に重ねる (LANG-5)。`first_line` の正規表現は重ねるときには組まず、
//! [`LanguageSet::precompile`] (起動時のパスを開くジョブの後ろの io のジョブと `--check-config`) で組んで、組めない値を診断に
//! する。それより先に判定で要れば、使うときに 1 回だけ組む (組込みの定義の正規表現はプロセスで共有する)。行頭に要る文字列で
//! 始まらない行には正規表現を当てない (組まずに済む)。

use std::sync::OnceLock;

use regex::Regex;

use crate::config::{Config, IndentUnit};
use crate::glob::{Glob, GlobCase, GlobError};
use crate::layers::{Origin, Origins};
use crate::{DiagKind, Diagnostic, Severity};

/// 判定の行き先 (どの定義にも当たらない文書の言語。定義できない、LANG-3)。
pub const PLAIN: &str = "plain";

/// `first_line` を当てる、文書の先頭の大きさ (LANG-6 (c))。
pub const FIRST_LINE_BYTES: usize = 1_024;

/// `first_line` の正規表現を組むときの大きさの上限 (LANG-3)。
const REGEX_SIZE_LIMIT: usize = 1 << 20;

/// 組込みの言語定義 1 行 (07 §9.6.1 の表の 1 行)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinLanguage {
    /// 言語 ID。
    pub id: &'static str,
    /// 表示の名前。
    pub display_name: &'static str,
    /// 拡張子 (先頭の `.` を書かない)。
    pub file_types: &'static [&'static str],
    /// ファイル名。
    pub file_names: &'static [&'static str],
    /// 先頭行の正規表現。
    pub first_line: Option<&'static str>,
    /// 行コメントの記号。
    pub comment_line: Option<&'static str>,
    /// 字下げの単位 (形式が字下げを決めている言語だけ、LANG-3)。
    pub indent_unit: Option<IndentUnit>,
}

const fn lang(
    id: &'static str,
    display_name: &'static str,
    file_types: &'static [&'static str],
    file_names: &'static [&'static str],
    first_line: Option<&'static str>,
    comment_line: Option<&'static str>,
    indent_unit: Option<IndentUnit>,
) -> BuiltinLanguage {
    BuiltinLanguage {
        id,
        display_name,
        file_types,
        file_names,
        first_line,
        comment_line,
        indent_unit,
    }
}

/// 組込みの言語定義 (07 §9.6.1 の表と同じ順。表との一致は eve-core の試験 `language_table` が確かめる)。
pub const BUILTIN_LANGUAGES: &[BuiltinLanguage] = &[
    lang("rust", "Rust", &["rs"], &[], None, Some("//"), None),
    lang(
        "markdown",
        "Markdown",
        &["md", "markdown"],
        &[],
        None,
        None,
        None,
    ),
    lang(
        "toml",
        "TOML",
        &["toml"],
        &["Cargo.lock"],
        None,
        Some("#"),
        None,
    ),
    lang("json", "JSON", &["json"], &[], None, None, None),
    lang(
        "jsonc",
        "JSON with Comments",
        &["jsonc", "json5"],
        &[],
        None,
        Some("//"),
        None,
    ),
    lang("yaml", "YAML", &["yml", "yaml"], &[], None, Some("#"), None),
    lang(
        "python",
        "Python",
        &["py", "pyi"],
        &[],
        Some(r"^#!.*\bpython[0-9.]*\b"),
        Some("#"),
        None,
    ),
    lang(
        "javascript",
        "JavaScript",
        &["js", "mjs", "cjs"],
        &[],
        Some(r"^#!.*\bnode\b"),
        Some("//"),
        None,
    ),
    lang(
        "typescript",
        "TypeScript",
        &["ts", "mts", "cts"],
        &[],
        None,
        Some("//"),
        None,
    ),
    lang("tsx", "TSX", &["tsx"], &[], None, Some("//"), None),
    lang("jsx", "JSX", &["jsx"], &[], None, Some("//"), None),
    lang("c", "C", &["c", "h"], &[], None, Some("//"), None),
    lang(
        "cpp",
        "C++",
        &["cc", "cpp", "cxx", "hpp", "hh", "hxx"],
        &[],
        None,
        Some("//"),
        None,
    ),
    lang(
        "go",
        "Go",
        &["go"],
        &[],
        None,
        Some("//"),
        Some(IndentUnit::Tab),
    ),
    lang(
        "bash",
        "Bash",
        &["sh", "bash"],
        &[".bashrc", ".bash_profile", ".profile"],
        Some(r"^#!.*\b(ba)?sh\b"),
        Some("#"),
        None,
    ),
    lang(
        "zsh",
        "Zsh",
        &["zsh"],
        &[".zshrc", ".zprofile"],
        Some(r"^#!.*\bzsh\b"),
        Some("#"),
        None,
    ),
    lang("html", "HTML", &["html", "htm"], &[], None, None, None),
    lang("css", "CSS", &["css"], &[], None, None, None),
    lang("diff", "Diff", &["diff", "patch"], &[], None, None, None),
    lang(
        "gitcommit",
        "Git Commit",
        &[],
        &["COMMIT_EDITMSG", "MERGE_MSG", "TAG_EDITMSG"],
        None,
        Some("#"),
        None,
    ),
    lang(
        "gitrebase",
        "Git Rebase",
        &[],
        &["git-rebase-todo"],
        None,
        Some("#"),
        None,
    ),
    lang(
        "ini",
        "INI",
        &["ini", "cfg"],
        &[".editorconfig", ".gitconfig"],
        None,
        Some(";"),
        None,
    ),
    lang(
        "dockerfile",
        "Dockerfile",
        &["dockerfile"],
        &["Dockerfile", "Containerfile"],
        None,
        Some("#"),
        None,
    ),
    lang(
        "make",
        "Makefile",
        &["mk", "mak"],
        &["Makefile", "GNUmakefile"],
        None,
        Some("#"),
        Some(IndentUnit::Tab),
    ),
];

/// 言語定義の誤りの種類 (07 §9.6 の LANG-2 / LANG-3 / LANG-6。文言は `msg.language.<名前>`)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LanguageProblem {
    /// 使えない言語の名前 (`[a-z][a-z0-9_+-]*` で 32 バイトまで。`plain` は使えない)。その定義を使わない。
    InvalidName,
    /// `name` が無い。その定義を使わない。
    MissingName,
    /// 長さ・数の上限を超えた (`display_name` は 64 文字、`first_line` と配列の要素は 256 バイト、配列は 64 個まで)。
    /// その項目を使わない。
    TooLong,
    /// `comment.line` の記号が使えない (1〜16 バイトで、空白・制御文字を含まない)。その項目を使わない。
    BadComment,
    /// `first_line` の正規表現を組めない。その項目を使わない。
    BadFirstLine {
        /// 組めない理由 (`regex` の説明の 1 行目)。
        reason: String,
    },
    /// `file_globs` の glob が資源の上限を超えた (07 §10.3 の EC-4)。その glob を使わない。
    BadGlob,
    /// 同じ層の 2 つの言語がファイルに当たった (LANG-6 (d)。後の定義を使う)。`Diagnostic.value` は文書のファイル名。
    Ambiguous {
        /// 先の定義の言語。
        first: String,
        /// 後の (使う) 定義の言語。
        second: String,
    },
    /// `languages/` のファイルが多すぎる (LANG-2)。超えた分を読まない。`Diagnostic.value` は上限の数。
    TooManyFiles,
    /// `languages/` のファイルが大きすぎる (LANG-2)。そのファイルを読まない。
    FileTooLarge,
}

impl LanguageProblem {
    /// 文言のキーの名前 (`msg.language.<これ>`) と、ログの種類の名前。
    #[must_use]
    pub fn message_name(&self) -> &'static str {
        match self {
            LanguageProblem::InvalidName => "invalid_name",
            LanguageProblem::MissingName => "missing_name",
            LanguageProblem::TooLong => "too_long",
            LanguageProblem::BadComment => "bad_comment",
            LanguageProblem::BadFirstLine { .. } => "bad_first_line",
            LanguageProblem::BadGlob => "bad_glob",
            LanguageProblem::Ambiguous { .. } => "ambiguous",
            LanguageProblem::TooManyFiles => "too_many_files",
            LanguageProblem::FileTooLarge => "file_too_large",
        }
    }
}

/// 言語定義の値 1 つと、それを書いた所 (行と桁。1 始まり)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LangValue<T> {
    /// 値。
    pub value: T,
    /// キーを書いた行。
    pub line: u32,
    /// キーを書いた桁 (文字数)。
    pub column: u32,
}

/// 配列の項目 (`file_types` など)。`replace` は置換 (`file_types = [...]`)、`add` は下の層への追加 (`file_types+ = [...]`、
/// 07 §3.3)。同じ定義に両方を書けば、置換してから追加する。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListValue {
    /// 置換する値。
    pub replace: Option<LangValue<Vec<String>>>,
    /// 追加する値。
    pub add: Option<LangValue<Vec<String>>>,
}

/// 1 つの層の 1 つの `[[language]]` (書いた項目だけ `Some`)。誤りの項目は入れない (下の層の値を使う、LANG-4)。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LanguageDef {
    /// 言語 ID (`name`)。
    pub name: String,
    /// 書いたファイルの表示名 (`config.toml`・`--config` のパス・`languages/<file>.toml`)。
    pub source: String,
    /// `name` を書いた行。
    pub line: u32,
    /// `name` を書いた桁。
    pub column: u32,
    /// `display_name`。
    pub display_name: Option<LangValue<String>>,
    /// `file_types`。
    pub file_types: ListValue,
    /// `file_names`。
    pub file_names: ListValue,
    /// `file_globs`。
    pub file_globs: ListValue,
    /// `first_line`。
    pub first_line: Option<LangValue<String>>,
    /// `indent.unit`。
    pub indent_unit: Option<LangValue<IndentUnit>>,
    /// `indent.width`。
    pub indent_width: Option<LangValue<u8>>,
    /// `indent.tab_width`。
    pub tab_width: Option<LangValue<u8>>,
    /// `comment.line`。
    pub comment_line: Option<LangValue<String>>,
}

/// 言語定義の値の由来 (`config.show` の右の列、07 §3.5)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LangOrigin {
    /// 組込みの定義。
    Builtin,
    /// ユーザー層のファイル。
    File {
        /// 表示名 (`config.toml`・`languages/<file>.toml` など)。
        source: String,
        /// 行。
        line: u32,
        /// 桁。
        column: u32,
    },
}

/// 定義の層 (判定で、より強い層の定義が勝つ。LANG-6 (d))。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LanguageLayer {
    /// 組込み。
    Builtin,
    /// ユーザー層 (`config.toml` の `[[language]]`・`--config`・`languages/*.toml`)。
    User,
}

/// 判定の名前 1 つ (`file_types` の拡張子・`file_names` のファイル名・`file_globs` の glob)。
#[derive(Debug, Clone)]
struct Pattern {
    /// 書いたとおり (`--dump-languages` に出す)。
    text: String,
    /// ASCII の小文字にしたもの (照らす、LANG-6 (a))。
    lower: String,
    layer: LanguageLayer,
    origin: LangOrigin,
}

/// 組込みの定義の `first_line` の正規表現 (プロセスで 1 回だけ組んで、どの [`LanguageSet`] も共有する)。添字は
/// [`BUILTIN_LANGUAGES`] の添字。
static BUILTIN_FIRST_LINE: [OnceLock<Option<Regex>>; BUILTIN_LANGUAGES.len()] =
    [const { OnceLock::new() }; BUILTIN_LANGUAGES.len()];

/// 組込みの定義の `first_line` の正規表現を組んでおく (起動時のパスを開くジョブの後ろの io のジョブで呼ぶ。判定で初めて
/// 要ったときに主スレッドで組まないように)。2 回目からは何もしない。
pub fn precompile_builtin() {
    for (i, b) in BUILTIN_LANGUAGES.iter().enumerate() {
        if let Some(src) = b.first_line {
            let _ = BUILTIN_FIRST_LINE[i].get_or_init(|| compile_first_line(src).ok());
        }
    }
}

/// `first_line` の正規表現の持ち方。
#[derive(Debug, Clone)]
enum Compiled {
    /// 組込みの定義 (添字。共有の正規表現を使うときに組む)。
    Builtin(usize),
    /// ユーザー層の定義 (使うときか [`LanguageSet::precompile`] で組む。組めなければ理由)。
    User(OnceLock<Result<Regex, String>>),
}

/// `first_line` の正規表現。
#[derive(Debug, Clone)]
struct FirstLine {
    source: String,
    /// 行頭に要る文字列 (`^#!.*` の `#!`)。これで始まらない行には正規表現を当てない (組まずに済む)。
    prefix: String,
    layer: LanguageLayer,
    origin: LangOrigin,
    compiled: Compiled,
    /// 下の層の値 (この正規表現を組めなかったときに使う、LANG-4 / FR-703)。
    fallback: Option<Box<FirstLine>>,
}

impl FirstLine {
    fn new(
        source: &str,
        layer: LanguageLayer,
        origin: LangOrigin,
        compiled: Compiled,
    ) -> FirstLine {
        FirstLine {
            source: source.to_owned(),
            prefix: anchored_prefix(source),
            layer,
            origin,
            compiled,
            fallback: None,
        }
    }

    /// `line` に当たれば、当たった値 (この層か、組めなかったときの下の層)。行頭に要る文字列で始まらない行は、正規表現を
    /// 組まずに外れとする (まだ組んでいない正規表現は、組めるとみなす)。
    fn matching(&self, line: &str) -> Option<&FirstLine> {
        let failed = match &self.compiled {
            Compiled::User(cell) => cell.get().is_some_and(Result::is_err),
            Compiled::Builtin(_) => false,
        };
        if !failed && !line.starts_with(&self.prefix) {
            return None;
        }
        match self.regex() {
            Some(r) => r.is_match(line).then_some(self),
            None => self.fallback.as_deref().and_then(|f| f.matching(line)),
        }
    }

    fn regex(&self) -> Option<&Regex> {
        match &self.compiled {
            Compiled::Builtin(i) => BUILTIN_FIRST_LINE[*i]
                .get_or_init(|| compile_first_line(&self.source).ok())
                .as_ref(),
            Compiled::User(cell) => cell
                .get_or_init(|| compile_first_line(&self.source).map_err(|e| error_line(&e)))
                .as_ref()
                .ok(),
        }
    }

    /// 正規表現を組み (下の層の値も)、組めなかったユーザー層の値の理由を `out` に足す。
    fn precompile(&self, out: &mut Vec<(LangOrigin, String, String)>) {
        let _ = self.regex();
        if let Compiled::User(cell) = &self.compiled
            && let Some(Err(reason)) = cell.get()
        {
            out.push((self.origin.clone(), self.source.clone(), reason.clone()));
            if let Some(f) = &self.fallback {
                f.precompile(out);
            }
        }
    }
}

/// `regex` の説明の最後の行 (どこがどう誤りか)。
fn error_line(e: &regex::Error) -> String {
    let text = e.to_string();
    text.lines().last().unwrap_or("").trim().to_owned()
}

/// `^` で始まる正規表現の、行頭に要る文字列 (`^#!.*\bpython` の `#!`)。`^` の後の、正規表現の記号でない文字の並び。
/// 並びの直後が量指定 (`*`・`?`・`{`) なら最後の文字は省ける (要らない) ので含めない。`^` で始まらない (フラグ `(?i)` など)
/// ときは空 (どの行も正規表現に当てる)。
fn anchored_prefix(source: &str) -> String {
    // 選択 (`^ab|cd`) があれば、行頭の並びが要るとは限らない
    let Some(rest) = source.strip_prefix('^').filter(|_| !source.contains('|')) else {
        return String::new();
    };
    let meta = |c: char| "\\.+*?()|[]{}^$".contains(c);
    let mut out: Vec<char> = rest.chars().take_while(|&c| !meta(c)).collect();
    if matches!(rest.chars().nth(out.len()), Some('*' | '?' | '{')) {
        out.pop();
    }
    out.into_iter().collect()
}

fn compile_first_line(source: &str) -> Result<Regex, regex::Error> {
    regex::RegexBuilder::new(source)
        .size_limit(REGEX_SIZE_LIMIT)
        .dfa_size_limit(REGEX_SIZE_LIMIT)
        .build()
}

/// 層を重ねた言語の定義 1 つ。
#[derive(Debug, Clone)]
pub struct Language {
    name: String,
    display_name: String,
    file_types: Vec<Pattern>,
    file_names: Vec<Pattern>,
    file_globs: Vec<(Glob, Pattern)>,
    first_line: Option<FirstLine>,
    /// `indent.unit` (値と由来)。
    pub indent_unit: Option<(IndentUnit, LangOrigin)>,
    /// `indent.width`。
    pub indent_width: Option<(u8, LangOrigin)>,
    /// `indent.tab_width`。
    pub tab_width: Option<(u8, LangOrigin)>,
    /// `comment.line`。
    pub comment_line: Option<(String, LangOrigin)>,
}

impl Language {
    /// 言語 ID。
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 表示の名前 (無ければ ID)。
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    fn empty(name: &str) -> Language {
        Language {
            name: name.to_owned(),
            display_name: name.to_owned(),
            file_types: Vec::new(),
            file_names: Vec::new(),
            file_globs: Vec::new(),
            first_line: None,
            indent_unit: None,
            indent_width: None,
            tab_width: None,
            comment_line: None,
        }
    }

    fn builtin(index: usize, b: &BuiltinLanguage) -> Language {
        let pats = |items: &[&str]| -> Vec<Pattern> {
            items
                .iter()
                .map(|t| Pattern {
                    text: (*t).to_owned(),
                    lower: t.to_ascii_lowercase(),
                    layer: LanguageLayer::Builtin,
                    origin: LangOrigin::Builtin,
                })
                .collect()
        };
        Language {
            name: b.id.to_owned(),
            display_name: b.display_name.to_owned(),
            file_types: pats(b.file_types),
            file_names: pats(b.file_names),
            file_globs: Vec::new(),
            first_line: b.first_line.map(|s| {
                FirstLine::new(
                    s,
                    LanguageLayer::Builtin,
                    LangOrigin::Builtin,
                    Compiled::Builtin(index),
                )
            }),
            indent_unit: b.indent_unit.map(|u| (u, LangOrigin::Builtin)),
            indent_width: None,
            tab_width: None,
            comment_line: b.comment_line.map(|c| (c.to_owned(), LangOrigin::Builtin)),
        }
    }
}

/// 判定に使う文書の事実 (LANG-6)。
#[derive(Debug, Clone, Copy, Default)]
pub struct DetectInput<'a> {
    /// ファイル名 (パスの最後の要素)。無題・標準入力の文書は `None`。
    pub file_name: Option<&'a str>,
    /// ワークスペースのルートからの相対パス (区切りは `/`)。ルートの外・ルートが決まっていなければ `None`。
    pub rel_path: Option<&'a str>,
    /// 本文の最初の行 (先頭から 1 KiB までの、最初の改行の前)。
    pub first_line: Option<&'a str>,
}

/// 同じ層の 2 つの言語が当たった (LANG-6 (d))。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ambiguity {
    /// 先の定義の言語。
    pub first: String,
    /// 後の (使う) 定義の言語。
    pub second: String,
    /// 使う定義の、当たった名前を書いた所。
    pub origin: LangOrigin,
}

/// 判定の結果。
#[derive(Debug, Clone)]
pub struct Detection<'a> {
    /// 当たった言語 (`None` は `plain`)。
    pub language: Option<&'a Language>,
    /// 同じ層の 2 つの言語が当たった。
    pub ambiguous: Option<Ambiguity>,
}

/// 層を重ねた言語の定義の集まり (組込み + ユーザー層)。
#[derive(Debug, Clone)]
pub struct LanguageSet {
    languages: Vec<Language>,
}

impl Default for LanguageSet {
    fn default() -> LanguageSet {
        LanguageSet::builtin()
    }
}

impl LanguageSet {
    /// 組込みの定義だけ (正規表現は組まない)。
    #[must_use]
    pub fn builtin() -> LanguageSet {
        LanguageSet {
            languages: BUILTIN_LANGUAGES
                .iter()
                .enumerate()
                .map(|(i, b)| Language::builtin(i, b))
                .collect(),
        }
    }

    /// 組込みの定義に、ユーザー層の定義 `defs` を書いた順に重ねる (LANG-5)。`file_globs` の glob はここで組み、資源の上限を
    /// 超えるものは使わずに診断 (表示名ごと) にする。`first_line` の正規表現は組まない (組むのは [`LanguageSet::precompile`]
    /// か、判定で使うとき。正規表現を組む時間で、起動時の文書を開くのを待たせない)。
    #[must_use]
    pub fn build(defs: &[LanguageDef]) -> (LanguageSet, Vec<(String, Vec<Diagnostic>)>) {
        let mut set = LanguageSet::builtin();
        let mut reports: Vec<(String, Vec<Diagnostic>)> = Vec::new();
        for def in defs {
            let diags = set.apply(def);
            if !diags.is_empty() {
                crate::merge_reports(&mut reports, [(def.source.clone(), diags)]);
            }
        }
        (set, reports)
    }

    /// `first_line` の正規表現をすべて組み (組込みの定義の共有のものも)、組めないユーザー層の値の診断 (表示名ごと。
    /// `msg.language.bad_first_line`) を返す。組めない値は使わず、下の層の値を使う (LANG-3 / LANG-4)。io のジョブ
    /// (`PrecompileLanguages`) と `--check-config` で呼ぶ。
    #[must_use]
    pub fn precompile(&self) -> Vec<(String, Vec<Diagnostic>)> {
        precompile_builtin();
        let mut failed = Vec::new();
        for l in &self.languages {
            if let Some(f) = &l.first_line {
                f.precompile(&mut failed);
            }
        }
        let mut reports: Vec<(String, Vec<Diagnostic>)> = Vec::new();
        for (origin, source, reason) in failed {
            if let LangOrigin::File {
                source: file,
                line,
                column,
            } = origin
            {
                let d = lang_diag(
                    LanguageProblem::BadFirstLine { reason },
                    "language.first_line",
                    &source,
                    line,
                    column,
                );
                crate::merge_reports(&mut reports, [(file, vec![d])]);
            }
        }
        reports
    }

    fn apply(&mut self, def: &LanguageDef) -> Vec<Diagnostic> {
        let mut diags = Vec::new();
        let i = match self.languages.iter().position(|l| l.name == def.name) {
            Some(i) => i,
            None => {
                self.languages.push(Language::empty(&def.name));
                self.languages.len() - 1
            }
        };
        let at = |line: u32, column: u32| LangOrigin::File {
            source: def.source.clone(),
            line,
            column,
        };
        let l = &mut self.languages[i];
        if let Some(d) = &def.display_name {
            l.display_name.clone_from(&d.value);
        }
        let pats = |v: &LangValue<Vec<String>>| -> Vec<Pattern> {
            v.value
                .iter()
                .map(|t| Pattern {
                    text: t.clone(),
                    lower: t.to_ascii_lowercase(),
                    layer: LanguageLayer::User,
                    origin: at(v.line, v.column),
                })
                .collect()
        };
        merge_list(&mut l.file_types, &def.file_types, &pats);
        merge_list(&mut l.file_names, &def.file_names, &pats);
        // glob は組んでから重ねる。資源の上限を超えたものは使わない
        let mut globs = |v: &LangValue<Vec<String>>| -> Vec<(Glob, Pattern)> {
            pats(v)
                .into_iter()
                .filter_map(|p| match Glob::new(&p.text, GlobCase::AsciiInsensitive) {
                    Ok(g) => Some((g, p)),
                    Err(GlobError::TooDeep | GlobError::TooManyAlternatives) => {
                        diags.push(lang_diag(
                            LanguageProblem::BadGlob,
                            "language.file_globs",
                            &p.text,
                            v.line,
                            v.column,
                        ));
                        None
                    }
                })
                .collect()
        };
        if let Some(v) = &def.file_globs.replace {
            l.file_globs = globs(v);
        }
        if let Some(v) = &def.file_globs.add {
            let more = globs(v);
            l.file_globs.extend(more);
        }
        if let Some(v) = &def.first_line {
            // 正規表現は組まずに置く (組むのは使うときか `precompile`)。組めなければ下の層の値を使う
            let mut f = FirstLine::new(
                &v.value,
                LanguageLayer::User,
                at(v.line, v.column),
                Compiled::User(OnceLock::new()),
            );
            f.fallback = l.first_line.take().map(Box::new);
            l.first_line = Some(f);
        }
        if let Some(v) = &def.indent_unit {
            l.indent_unit = Some((v.value, at(v.line, v.column)));
        }
        if let Some(v) = &def.indent_width {
            l.indent_width = Some((v.value, at(v.line, v.column)));
        }
        if let Some(v) = &def.tab_width {
            l.tab_width = Some((v.value, at(v.line, v.column)));
        }
        if let Some(v) = &def.comment_line {
            l.comment_line = Some((v.value.clone(), at(v.line, v.column)));
        }
        diags
    }

    /// 定義のある言語 (組込みの表の順、その後にユーザー層で足した順)。
    #[must_use]
    pub fn languages(&self) -> &[Language] {
        &self.languages
    }

    /// 言語 ID の定義。
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Language> {
        self.languages.iter().find(|l| l.name == name)
    }

    /// 文書の言語を判定する (07 §9.2 の 2〜4 と LANG-6)。`file_names` → `file_globs` → `file_types` (長いほうが勝つ) →
    /// `first_line` の順に、最初に当たった段で決める。段の中では、より強い層の定義が勝ち、同じ層なら後の定義が勝って
    /// [`Detection::ambiguous`] に組を返す。どれにも当たらなければ `plain` (`language` が `None`)。
    #[must_use]
    pub fn detect(&self, input: &DetectInput<'_>) -> Detection<'_> {
        let name = input.file_name.map(str::to_ascii_lowercase);
        // (長さ, 層, 言語の番号, 当たった名前の由来)
        let mut stages: [Vec<(usize, LanguageLayer, usize, LangOrigin)>; 4] = Default::default();
        for (i, l) in self.languages.iter().enumerate() {
            if let Some(n) = &name {
                for p in &l.file_names {
                    if p.lower == *n {
                        stages[0].push((0, p.layer, i, p.origin.clone()));
                    }
                }
                for (g, p) in &l.file_globs {
                    let subject = if g.has_slash() {
                        input.rel_path
                    } else {
                        input.file_name
                    };
                    if subject.is_some_and(|s| g.is_match(s)) {
                        stages[1].push((0, p.layer, i, p.origin.clone()));
                    }
                }
                for p in &l.file_types {
                    // 名前の末尾の `.<型>`。先頭の `.` は拡張子の区切りに数えない (`.bashrc` は `file_names` で当てる)
                    let hit = n.len() > p.lower.len() + 1
                        && n.ends_with(&p.lower)
                        && n.as_bytes()[n.len() - p.lower.len() - 1] == b'.';
                    if hit && !p.lower.is_empty() {
                        stages[2].push((p.lower.len(), p.layer, i, p.origin.clone()));
                    }
                }
            }
            if let (Some(f), Some(line)) = (&l.first_line, input.first_line)
                && let Some(hit) = f.matching(line)
            {
                stages[3].push((0, hit.layer, i, hit.origin.clone()));
            }
        }
        for mut cands in stages {
            if cands.is_empty() {
                continue;
            }
            cands.sort_by_key(|(len, layer, i, _)| (*len, *layer, *i));
            let Some((len, layer, win, origin)) = cands.pop() else {
                continue;
            };
            let other = cands
                .iter()
                .rev()
                .find(|(l2, y2, j, _)| (*l2, *y2) == (len, layer) && *j != win)
                .map(|(_, _, j, _)| *j);
            return Detection {
                language: Some(&self.languages[win]),
                ambiguous: other.map(|j| Ambiguity {
                    first: self.languages[j].name.clone(),
                    second: self.languages[win].name.clone(),
                    origin,
                }),
            };
        }
        Detection {
            language: None,
            ambiguous: None,
        }
    }

    /// `--dump-languages` の出力 (`[[language]]` の TOML。v0.5 で効く項目だけ、LANG-1)。
    #[must_use]
    pub fn dump(&self) -> String {
        let mut out = String::new();
        for l in &self.languages {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str("[[language]]\n");
            out.push_str(&format!("name = {}\n", quote(&l.name)));
            out.push_str(&format!("display_name = {}\n", quote(&l.display_name)));
            let list = |ps: &mut dyn Iterator<Item = &Pattern>| -> String {
                let items: Vec<String> = ps.map(|p| quote(&p.text)).collect();
                format!("[{}]", items.join(", "))
            };
            out.push_str(&format!(
                "file_types = {}\n",
                list(&mut l.file_types.iter())
            ));
            out.push_str(&format!(
                "file_names = {}\n",
                list(&mut l.file_names.iter())
            ));
            out.push_str(&format!(
                "file_globs = {}\n",
                list(&mut l.file_globs.iter().map(|(_, p)| p))
            ));
            if let Some(f) = &l.first_line {
                out.push_str(&format!("first_line = {}\n", quote(&f.source)));
            }
            let mut indent = Vec::new();
            if let Some((u, _)) = &l.indent_unit {
                indent.push(format!("unit = {}", quote(u.name())));
            }
            if let Some((w, _)) = &l.indent_width {
                indent.push(format!("width = {w}"));
            }
            if let Some((w, _)) = &l.tab_width {
                indent.push(format!("tab_width = {w}"));
            }
            if !indent.is_empty() {
                out.push_str(&format!("indent = {{ {} }}\n", indent.join(", ")));
            }
            if let Some((c, _)) = &l.comment_line {
                out.push_str(&format!("comment = {{ line = {} }}\n", quote(c)));
            }
        }
        out
    }
}

/// 配列の項目を重ねる: 置換してから追加する (07 §3.3)。
fn merge_list(
    into: &mut Vec<Pattern>,
    v: &ListValue,
    pats: &dyn Fn(&LangValue<Vec<String>>) -> Vec<Pattern>,
) {
    if let Some(r) = &v.replace {
        *into = pats(r);
    }
    if let Some(a) = &v.add {
        into.extend(pats(a));
    }
}

/// TOML の基本文字列 (`"…"`)。
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn lang_diag(
    problem: LanguageProblem,
    key: &str,
    value: &str,
    line: u32,
    column: u32,
) -> Diagnostic {
    Diagnostic {
        severity: Severity::Warning,
        kind: DiagKind::Language(problem),
        key: key.to_owned(),
        value: value.to_owned(),
        line,
        column,
        condition: None,
    }
}

/// 言語の名前として使えるか (LANG-3): `[a-z][a-z0-9_+-]*` で 32 バイトまで。`plain` は使えない。
#[must_use]
pub fn is_language_name(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 32
        && b[0].is_ascii_lowercase()
        && b[1..].iter().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'+' | b'-')
        })
        && s != PLAIN
}

/// 文書の本文の先頭から、`first_line` に当てる最初の行 (先頭から 1 KiB までの、最初の改行 (`\n` か `\r`) の前。LANG-6 (c))。
#[must_use]
pub fn first_line_of(head: &str) -> &str {
    let mut end = head.len().min(FIRST_LINE_BYTES);
    while !head.is_char_boundary(end) {
        end -= 1;
    }
    let head = &head[..end];
    head.find(['\n', '\r']).map_or(head, |i| &head[..i])
}

/// 文書ごとに決まる値の由来 (`config.show`、07 §3.5 / §3.6 の LAYER-5)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueOrigin {
    /// 層 (既定・ユーザー層の一般の値・`--set`) で決まった。
    Config(Origin),
    /// 言語定義で決まった。
    Language {
        /// 言語 ID。
        language: String,
        /// 値を書いた所。
        at: LangOrigin,
    },
}

/// 文書ごとに決まる値 1 つと由来。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocValue<T> {
    /// 値。
    pub value: T,
    /// 由来。
    pub origin: ValueOrigin,
}

/// 文書ごとに決まる値 (07 §3.6。v0.5 の M33 の段 2 では言語定義が持つ `editor.tab_width`・`editor.indent_unit`・
/// `editor.indent_width`。`file.default_*` は言語定義が持たないので、層の値のまま)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocValues {
    /// `editor.tab_width`。
    pub tab_width: DocValue<u8>,
    /// `editor.indent_unit`。
    pub indent_unit: DocValue<IndentUnit>,
    /// `editor.indent_width`。
    pub indent_width: DocValue<u8>,
}

/// 文書ごとの値を決める (07 §3.6 の LAYER-1 の (1)〜(4)・(8))。`config` と `origins` は層を重ねた結果 ((1)(2)(8))、
/// `language` は文書の言語の定義 (組込みにユーザー層を重ねたもの、(3)(4))。`--set` で決まった値は言語より強く、ほかは
/// 言語定義がその項目を持てば言語の値になる (持たなければ層の値)。
#[must_use]
pub fn doc_values(config: &Config, origins: &Origins, language: Option<&Language>) -> DocValues {
    fn pick<T: Copy>(
        key: &str,
        value: T,
        lang: Option<(&Language, &Option<(T, LangOrigin)>)>,
        origins: &Origins,
    ) -> DocValue<T> {
        let origin = origins.of(key);
        match lang {
            Some((l, Some((v, at)))) if !matches!(origin, Origin::Cli { .. }) => DocValue {
                value: *v,
                origin: ValueOrigin::Language {
                    language: l.name.clone(),
                    at: at.clone(),
                },
            },
            _ => DocValue {
                value,
                origin: ValueOrigin::Config(origin.clone()),
            },
        }
    }
    DocValues {
        tab_width: pick(
            "editor.tab_width",
            config.tab_width,
            language.map(|l| (l, &l.tab_width)),
            origins,
        ),
        indent_unit: pick(
            "editor.indent_unit",
            config.indent_unit,
            language.map(|l| (l, &l.indent_unit)),
            origins,
        ),
        indent_width: pick(
            "editor.indent_width",
            config.indent_width,
            language.map(|l| (l, &l.indent_width)),
            origins,
        ),
    }
}

#[cfg(test)]
mod tests;
