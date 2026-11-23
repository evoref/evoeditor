//! 設定スキーマ・既定値・TOML 読込・検証・テーマ解決・言語ファイルの解析 (docs/02 §4.1、docs/07)。
//!
//! 言語ファイル (文言カタログ、`Catalog`。ユーザーの言語ファイルの検査と重ね合わせは `load_locale_file` と
//! `Catalog::overlay`)、ファイル由来の文字列の無害化 (`sanitize_untrusted`)、ユーザー層の設定ファイルの読込 (`load_str`。公開キーは `config.rs` の
//! 宣言の表、07 §4.1。条件付きセクションの解決は `load_str_for` と `Conditions`、07 §3.4)、層の合成と値の由来 (`resolve`・
//! `Origins`、§3.5)、キーマップ (`keymap.toml`) とテーマファイルの読込、言語定義 (`[[language]]`・`languages/*.toml` の
//! 読込・組込みの表・併合・判定・文書ごとの値。`LanguageSet`、07 §9.6 / §3.6)。

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod catalog;
mod chars;
mod condition;
mod config;
pub mod glob;
mod keymap;
mod language;
mod layers;
mod locale;
mod locale_file;
mod locales_gen;
mod position;
pub mod reserved;
mod sanitize;
pub mod theme;
pub mod theme_file;
mod toml_scan;
mod workspace;

pub use catalog::{Catalog, CatalogError, base_key, placeholders};
pub use chars::{CharClass, forbidden_char};
pub use condition::{Condition, Conditions, EnvConditions, Frontend, Os};
pub use config::{
    BackspaceUnit, ColorMode, ConditionProblem, Config, DefaultEncoding, DefaultLineEnding,
    DiagKind, Diagnostic, ExternalChangePolicy, IndentUnit, KeymapProblem, ListItemProblem, Loaded,
    Osc52Policy, SearchCase, Severity, ShowWhitespace, TabLineMode, ThemeBackground, WrapIndent,
    WrapMode, Written, changed_keys, default_display, is_theme_name, load_language_str, load_str,
    load_str_for, load_str_onto, workspace_policy,
};
pub use keymap::{KeyAction, KeyArgValue, KeymapEntry, KeymapFile, load_keymap_str};
pub use language::{
    AUTO, Ambiguity, BUILTIN_LANGUAGES, BuiltinLanguage, DetectInput, Detection, DocLayers,
    DocValue, DocValues, FIRST_LINE_BYTES, LANGUAGE_DEFS_MAX, LangOrigin, LangValue, Language,
    LanguageDef, LanguageLayer, LanguageProblem, LanguageSet, ListValue, PLAIN, ValueOrigin,
    doc_values, first_line_compiles, first_line_of, is_language_name, precompile_builtin,
};
pub use layers::{
    Composed, KeyOrigin, Origin, Origins, Resolved, SET_SOURCE, Source, UserLayer, check_ui_locale,
    compose, declared, dump, load_set, merge_reports, resolve, resolve_home, values,
};
pub use locale::{Locale, LocaleChoice, LocaleSource, LocaleWarning, select_locale};
pub use locale_file::{
    HINT_MAX_WIDTH, LOCALE_CHECK_MAX_DIAGNOSTICS, LOCALE_MAX_BYTES, LOCALE_MAX_DIAGNOSTICS,
    LOCALE_MAX_MARKS, LOCALE_MAX_TEXT_CHARS, LOCALE_RESERVED_PREFIXES, LocaleFile, LocaleProblem,
    PlaceholderIssue, TextCheck, check_text, hint_width, load_locale_file,
};
pub use position::{CONFIG_MAX_DIAGNOSTICS, cap_diagnostics};
pub use sanitize::{UNTRUSTED_MAX_CHARS, escape_untrusted, sanitize_untrusted};
pub use toml_scan::{StructureProblem, TOML_MAX_KEY_DEPTH, TOML_MAX_NESTING};
pub use workspace::{
    Classified, KeyRejectReason, WorkspaceClass, WorkspaceLayer, WorkspacePolicy, WorkspaceProblem,
    load_workspace_str,
};
