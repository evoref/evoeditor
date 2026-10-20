//! `Paths` Port: 設定・状態・データ・実行時のディレクトリと、パスの正規化 (docs/02-architecture.md §4.4)。
//! 本番は eve-platform (OS 規約)、テストは固定の仮想パス。

use crate::fs::FsError;
use crate::fspath::FsPath;

pub trait Paths: Send + Sync {
    fn config_dir(&self) -> FsPath;
    fn state_dir(&self) -> FsPath;
    fn data_dir(&self) -> FsPath;
    /// 実行時の一時ファイル (ソケット等) の置き場。
    fn runtime_dir(&self) -> FsPath;
    /// 境界から入ったパス (文字列) を `FsPath` にする。相対パスは作業ディレクトリ基準で絶対にし、`.` / `..` を畳む。
    /// 存在の確認はしない (存在しないパスも正規化できる)。
    fn normalize(&self, raw: &str) -> Result<FsPath, FsError>;
}
