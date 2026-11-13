//! ジョブの投入口 `JobSpawner` と Port の一式 (docs/02 §9.3)。

use std::rc::Rc;
use std::sync::Arc;

use eve_view::{DocumentId, JobId, Revision};

use crate::io::fs::{FileSystem, ReadChunks};
use crate::io::paths::Paths;
use crate::time::Clock;

use super::JobKind;

/// ジョブの投入 (core のスレッドから呼ぶ。待たない)。本番は eve-host、テストは `ManualJobSpawner`。
pub trait JobSpawner {
    /// ジョブを投入し、振った `JobId` を返す。`doc` は結果の宛先の文書、`base` は投入時の Revision。
    fn spawn(&self, doc: Option<DocumentId>, base: Revision, kind: JobKind) -> JobId;
    /// 存在しない・完了済みの `JobId` は何もしない。
    fn cancel(&self, job: JobId);
}

/// ジョブの中から使う Port (ワーカースレッドに渡すので `Send + Sync`)。
#[derive(Clone)]
pub struct JobPorts {
    /// ファイルシステム。
    pub fs: Arc<dyn FileSystem>,
    /// 時計。
    pub clock: Arc<dyn Clock>,
    /// 設定・データ・退避などのディレクトリ。
    pub paths: Arc<dyn Paths>,
    /// 標準入力 (`evoeditor -`、v0.4 の M25。03 §9.2.2)。標準入力が端末でないときだけ Host が作る。
    /// `JobKind::OpenStdin` だけが使う。ほかは `None`。
    pub stdin: Option<Arc<dyn StdinSource>>,
}

/// 標準入力をチャンクで読む Port (02 §4.4 の `StdinSource`、v0.4 の M25)。eve-platform が実装する (05 §9.1 / §9.2)。
///
/// 作るのは Host で、作るときに標準入力を私有のハンドル / fd に移し、読取のスレッド `eve-stdin` (02 §9.1) を始める。
/// `take` が返す `ReadChunks` の `next_chunk` は、`eve-stdin` が読んだチャンク (最大 1 MiB) を深さ 4 のチャネルから取る。
/// データが無い間はチャネルだけで待ち (時間で区切って空のチャンクを返さない。02 §14 の NFR-204)、`wake` の印を受けたら
/// 空のチャンク (`Ok(Some(&[]))`) を返して戻る。EOF は `Ok(None)`、読取の失敗は `ChunkError::Io`。`ReadChunks` を drop
/// したら (取消し・上限・終了)、`eve-stdin` を OS ごとの方法で起こして止め、`eve-stdin` が自分で私有のハンドル / fd を閉じる。
pub trait StdinSource: Send + Sync {
    /// 読み手を取る。1 回だけ `Some` (2 回目からは `None`)。
    fn take(&self) -> Option<Box<dyn ReadChunks + Send>>;
    /// 待っている `next_chunk` を起こす (取消しのとき Host の `JobSpawner` が呼ぶ。ジョブは起きて `CancelToken` を見る)。
    /// 印は 1 つに畳む (何度呼んでも、受け手が起きるのは 1 回)。待っていないときに呼んでもよい。
    fn wake(&self);
}

/// core が持つ Port の一式。
#[derive(Clone)]
pub struct Ports {
    /// ファイルシステム。
    pub fs: Arc<dyn FileSystem>,
    /// 時計。
    pub clock: Arc<dyn Clock>,
    /// ジョブの投入 (core のスレッドだけで使う)。
    pub jobs: Rc<dyn JobSpawner>,
    /// 設定・データ・退避などのディレクトリ。
    pub paths: Arc<dyn Paths>,
}

impl Ports {
    /// ワーカーに渡す Port (`jobs` は含まない)。
    pub fn job_ports(&self) -> JobPorts {
        JobPorts {
            fs: self.fs.clone(),
            clock: self.clock.clone(),
            paths: self.paths.clone(),
            stdin: None,
        }
    }
}
