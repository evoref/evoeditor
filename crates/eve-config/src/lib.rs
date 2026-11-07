//! 設定スキーマ・既定値・TOML 読込・検証・テーマ解決・言語ファイルの解析 (docs/02 §4.1、docs/07)。
//!
//! 言語ファイル (文言カタログ、`Catalog`)、ユーザー層の設定ファイルの読込 (`load_str`。公開キーは `config.rs` の
//! 宣言の表、07 §4.1)、キーマップ (`keymap.toml`) とテーマファイルの読込。

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod catalog;
mod config;
mod keymap;
mod layers;
mod locale;
pub mod reserved;
pub mod theme;
pub mod theme_file;

pub use catalog::{Catalog, CatalogError, base_key, placeholders};
pub use config::{
    BackspaceUnit, ColorMode, Config, DefaultEncoding, DefaultLineEnding, DiagKind, Diagnostic,
    ExternalChangePolicy, IndentUnit, KeymapProblem, Loaded, Osc52Policy, Severity, ShowWhitespace,
    ThemeBackground, WrapIndent, WrapMode, changed_keys, default_display, is_theme_name, load_str,
    load_str_onto,
};
pub use keymap::{KeyAction, KeymapEntry, KeymapFile, load_keymap_str};
pub use layers::{Resolved, SET_SOURCE, Source, dump, load_set, resolve, values};
pub use locale::{Locale, LocaleChoice, LocaleSource, LocaleWarning, select_locale};
