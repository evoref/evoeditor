//! Ports の OS 実装 (docs/02 §4.1、docs/05)。Windows (`fs::WinFs` ほか) と Unix (`unix_fs::UnixFs`、v0.2 の M7。Linux で確かめる)。
//! 純粋な計算 (パスの正規化・ディレクトリの解決) はどの OS でもテストできるようにしてある。
//!
//! 状態ディレクトリの部品 (保存中フラグ `inflight`・直接上書き `inplace`・状態ファイル・スワップ・緊急退避 `rescue`・ログ) は
//! OS に依らず、OS ごとに違う小さな部品 (所有者のみの作成・プロセスの生存と起動時刻・空き容量・代替の退避先) だけを `sys` から呼ぶ。
//! OS の API を直接呼ぶ部分 (Windows のコンソール・セッション・クリップボード・`ReplaceFileW`、Unix の xattr など) は `cfg` のモジュールに閉じる。

// unsafe は OS の API を呼ぶ `fs::winapi`・`console`・`dll`・`save` など (Windows) と `unix_fs`・`sys`・`inplace`・`hooks` (Unix) だけに許す (R5)。
#![deny(unsafe_code)]
// eve-core の `FsError` は大きい型 (エラーは稀な経路)。値で返す。
#![allow(clippy::result_large_err)]

/// `FileSystem` のうち状態ディレクトリを使うメソッド (保存中フラグ・孤児のバックアップ・緊急退避・状態ファイル・スワップ・
/// 直接上書き)。実装は OS に依らないので、`WinFs` と `UnixFs` の `impl FileSystem` の中で展開して共有する
/// (どちらも `state_dir()`・`state_dir`・`replacing` を持つ)。
macro_rules! state_dir_methods {
    () => {
        fn inflight_for(
            &self,
            path: &::eve_core::FsPath,
        ) -> Result<Vec<::eve_core::InflightRecord>, ::eve_core::FsError> {
            Ok(self
                .state_dir()
                .map(|sd| $crate::inflight::read_flags(sd, path.as_path()))
                .unwrap_or_default())
        }

        fn list_inflight(&self) -> Result<Vec<::eve_core::InflightRecord>, ::eve_core::FsError> {
            Ok(self
                .state_dir()
                .map($crate::inflight::list_all)
                .unwrap_or_default())
        }

        fn list_orphan_backups(
            &self,
        ) -> Result<Vec<::eve_core::OrphanBackup>, ::eve_core::FsError> {
            Ok(self
                .state_dir()
                .map($crate::inplace::list_orphan_backups)
                .unwrap_or_default())
        }

        fn list_recovery_dirs(
            &self,
        ) -> Result<Vec<::eve_core::RecoveryDirEntry>, ::eve_core::FsError> {
            Ok($crate::rescue::list_recovery_dirs(self.state_dir()))
        }

        fn write_recovery(
            &self,
            doc: &::eve_core::DocSnapshot,
        ) -> Result<::eve_core::FsPath, ::eve_core::FsError> {
            use ::eve_core::{RescueReason, RescueWriter};
            let out = $crate::rescue::StateRescue::new(self.state_dir.clone()).write(
                &[std::sync::Arc::new(doc.clone())],
                RescueReason::UserSave,
                std::time::Duration::from_secs(30),
            );
            match out.dir {
                Some(dir) if out.complete => Ok(::eve_core::FsPath::new(dir)),
                _ => {
                    let msg = out
                        .error
                        .unwrap_or_else(|| "書き込めませんでした".to_owned());
                    let mut e = ::eve_core::FsError::new(::eve_core::FsErrorKind::Other);
                    e.source = Some(::std::io::Error::other(msg));
                    Err(e)
                }
            }
        }

        fn read_state_file(&self, name: &str) -> Result<Vec<u8>, ::eve_core::FsError> {
            let sd = self
                .state_dir()
                .ok_or_else(|| ::eve_core::FsError::new(::eve_core::FsErrorKind::Unsupported))?;
            $crate::state_file::read(sd, name)
        }

        fn write_state_file(&self, name: &str, data: &[u8]) -> Result<(), ::eve_core::FsError> {
            let sd = self
                .state_dir()
                .ok_or_else(|| ::eve_core::FsError::new(::eve_core::FsErrorKind::Unsupported))?;
            $crate::state_file::write(sd, name, data)
        }

        fn set_aside_state_file(&self, name: &str) -> Result<(), ::eve_core::FsError> {
            let sd = self
                .state_dir()
                .ok_or_else(|| ::eve_core::FsError::new(::eve_core::FsErrorKind::Unsupported))?;
            $crate::state_file::set_aside(sd, name)
        }

        fn write_swap(&self, doc: &::eve_core::DocSnapshot) -> Result<(), ::eve_core::FsError> {
            // 状態ディレクトリを使えないときはスワップを無効のままにする (05 §7.1)
            let sd = self
                .state_dir()
                .ok_or_else(|| ::eve_core::FsError::new(::eve_core::FsErrorKind::Unsupported))?;
            $crate::swap::write_swap(sd, doc)
        }

        fn discard_swap(
            &self,
            doc: ::eve_core::DocumentId,
            path: Option<&::eve_core::FsPath>,
        ) -> Result<(), ::eve_core::FsError> {
            let sd = self
                .state_dir()
                .ok_or_else(|| ::eve_core::FsError::new(::eve_core::FsErrorKind::Unsupported))?;
            $crate::swap::discard_swap(sd, doc.0, path)
        }

        fn list_swaps(&self) -> Result<Vec<::eve_core::SwapEntry>, ::eve_core::FsError> {
            Ok(self
                .state_dir()
                .map($crate::swap_scan::list_swaps)
                .unwrap_or_default())
        }

        fn read_swap(
            &self,
            entry: &::eve_core::SwapEntry,
        ) -> Result<::eve_core::SwapBody, ::eve_core::FsError> {
            $crate::swap_scan::read_swap(entry)
        }

        fn remove_swap(&self, entry: &::eve_core::SwapEntry) -> Result<(), ::eve_core::FsError> {
            let sd = self
                .state_dir()
                .ok_or_else(|| ::eve_core::FsError::new(::eve_core::FsErrorKind::Unsupported))?;
            $crate::swap_scan::remove_swap(sd, entry)
        }

        fn write_in_place(
            &self,
            path: &::eve_core::FsPath,
            data: &mut dyn ::eve_core::ReadChunks,
            len: u64,
            opts: &::eve_core::SaveOptions,
            cancel: &::eve_core::CancelToken,
        ) -> Result<::eve_core::SaveReport, ::eve_core::FsError> {
            $crate::inplace::write_in_place(
                self,
                self.state_dir(),
                &self.replacing,
                path,
                data,
                len,
                opts,
                cancel,
            )
        }
    };
}

pub mod clipboard;
pub mod clipboard_cmd;
#[cfg(all(windows, feature = "test-hooks"))]
pub mod clipboard_test;
#[cfg(windows)]
pub mod console;
pub mod dirs;
#[cfg(windows)]
pub mod dll;
#[cfg(windows)]
pub mod fs;
mod hooks;
mod inflight;
mod inplace;
pub mod locale;
pub mod log;
#[cfg(windows)]
mod owner;
pub mod path;
pub mod rescue;
#[cfg(windows)]
mod save;
#[cfg(windows)]
pub mod session;
#[cfg(unix)]
pub mod signals;
mod state_file;
mod swap;
mod swap_scan;
mod sys;
#[cfg(windows)]
pub mod term;
#[cfg(unix)]
#[path = "term_unix.rs"]
pub mod term;
#[cfg(unix)]
pub mod unix_fs;
