//! 設定スキーマ・既定値・TOML 読込・検証・テーマ解決・言語ファイルの解析 (docs/02 §4.1、docs/07)。
//!
//! 現在は言語ファイル (文言カタログ、`Catalog`) だけ。設定・テーマは、それを使う版で足す。

#![forbid(unsafe_code)]

mod catalog;

pub use catalog::{Catalog, CatalogError, base_key};
