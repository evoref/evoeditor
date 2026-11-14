//! 設定スキーマ・既定値・TOML 読込・検証・テーマ解決・言語ファイルの解析 (docs/02 §4.1、docs/07)。
//!
//! 言語ファイル (文言カタログ、`Catalog`。ユーザーの言語ファイルの検査と重ね合わせは `load_locale_file` と
//! `Catalog::overlay`)、ファイル由来の文字列の無害化 (`sanitize_untrusted`)、ユーザー層の設定ファイルの読込 (`load_str`。公開キーは `config.rs` の
//! 宣言の表、07 §4.1)、キーマップ (`keymap.toml`) とテーマファイルの読込。

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod catalog;
mod chars;
mod config;
mod keymap;
mod layers;
mod locale;
mod locale_file;
pub mod reserved;
mod sanitize;
pub mod theme;
pub mod theme_file;

pub use catalog::{Catalog, CatalogError, base_key, placeholders};
pub use chars::{CharClass, forbidden_char};
pub use config::{
    BackspaceUnit, ColorMode, Config, DefaultEncoding, DefaultLineEnding, DiagKind, Diagnostic,
    ExternalChangePolicy, IndentUnit, KeymapProblem, Loaded, Osc52Policy, SearchCase, Severity,
    ShowWhitespace, ThemeBackground, WrapIndent, WrapMode, changed_keys, default_display,
    is_theme_name, load_str, load_str_onto,
};
pub use keymap::{KeyAction, KeymapEntry, KeymapFile, load_keymap_str};
pub use layers::{
    KeyOrigin, Resolved, SET_SOURCE, Source, check_ui_locale, dump, load_set, merge_reports,
    resolve, values,
};
pub use locale::{Locale, LocaleChoice, LocaleSource, LocaleWarning, select_locale};
pub use locale_file::{
    HINT_MAX_WIDTH, LOCALE_CHECK_MAX_DIAGNOSTICS, LOCALE_MAX_BYTES, LOCALE_MAX_DIAGNOSTICS,
    LOCALE_MAX_MARKS, LOCALE_MAX_TEXT_CHARS, LOCALE_RESERVED_PREFIXES, LocaleFile, LocaleProblem,
    PlaceholderIssue, TextCheck, check_text, hint_width, load_locale_file,
};
pub use sanitize::{UNTRUSTED_MAX_CHARS, escape_untrusted, sanitize_untrusted};
