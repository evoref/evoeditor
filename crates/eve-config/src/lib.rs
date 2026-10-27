//! 設定スキーマ・既定値・TOML 読込・検証・テーマ解決・言語ファイルの解析 (docs/02 §4.1、docs/07)。
//!
//! 言語ファイル (文言カタログ、`Catalog`) と、ユーザー層の設定ファイルの読込 (`load_str`、公開キー 7 個)。
//! テーマは、それを使う版で足す。

#![forbid(unsafe_code)]

mod catalog;
mod config;
mod reserved;

pub use catalog::{Catalog, CatalogError, base_key};
pub use config::{
    ColorMode, Config, DefaultEncoding, DefaultLineEnding, DiagKind, Diagnostic, IndentUnit,
    Loaded, Severity, default_display, load_str,
};
