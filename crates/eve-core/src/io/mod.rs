//! 開く・保存・再読込・外部変更の確認の論理手順と、そのジョブの本体、`FileSystem` / `Paths` の Port、スワップ・退避・
//! 起動時の走査 (docs/02 §4.6 の `io/`)。
//!
//! spec: 03 §9, 03 §10, 02 §4.5, 05 §4, 05 §7
pub(crate) mod config_reload;
pub(crate) mod content_hash;
pub(crate) mod external;
pub(crate) mod file;
pub(crate) mod file_cmds;
pub(crate) mod fs;
pub(crate) mod fspath;
pub(crate) mod interrupted;
pub(crate) mod io_jobs;
pub(crate) mod last_version;
pub(crate) mod leftover;
pub(crate) mod leftover_offer;
pub(crate) mod open;
pub(crate) mod paths;
pub(crate) mod pending;
pub(crate) mod reload;
pub(crate) mod replacing;
pub(crate) mod rescue;
pub(crate) mod save;
pub(crate) mod save_all;
pub(crate) mod secret;
pub(crate) mod snapshot;
pub(crate) mod startup;
pub(crate) mod swap;
pub(crate) mod swap_recovery;
pub(crate) mod unmap;
pub(crate) mod unmappable;
