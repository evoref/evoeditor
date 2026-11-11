//! ジョブの投入口 `JobSpawner` と Port の一式 (docs/02 §9.3)。

use std::rc::Rc;
use std::sync::Arc;

use eve_view::{DocumentId, JobId, Revision};

use crate::io::fs::FileSystem;
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
        }
    }
}
