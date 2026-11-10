//! `Paths` Port: 設定・状態・データ・実行時のディレクトリと、パスの正規化 (docs/02-architecture.md §4.4)。
//! 本番は eve-platform (OS 規約)、テストは固定の仮想パス。

use crate::io::fs::FsError;
use crate::io::fspath::FsPath;

/// 各ディレクトリの解決結果とパスの正規化 (02 §4.4)。起動時に解決済みの値を返すだけで I/O しない。ディレクトリの存在は保証しない。
pub trait Paths: Send + Sync {
    /// 設定のディレクトリ (`EVE_CONFIG_DIR` が最優先、07 §3.1。OS 別の既定は 05 §7.1)。
    fn config_dir(&self) -> FsPath;
    /// 状態のディレクトリ (ログ・recovery・swap・backup・inflight の親、05 §7.5)。
    fn state_dir(&self) -> FsPath;
    /// アプリのデータのディレクトリ (05 §7.1)。
    fn data_dir(&self) -> FsPath;
    /// 実行時の一時ファイル (ソケット等) の置き場。
    fn runtime_dir(&self) -> FsPath;
    /// 境界から入ったパス (文字列) を `FsPath` にする。相対パスは作業ディレクトリ基準で絶対にし、`.` / `..` を畳む。
    /// 存在の確認はしない (存在しないパスも正規化できる)。
    ///
    /// # Errors
    ///
    /// パスの構文が不正 (05 §3.2 の手順 1〜5 の検査に通らない) なとき `FsError`。
    fn normalize(&self, raw: &str) -> Result<FsPath, FsError>;
}
