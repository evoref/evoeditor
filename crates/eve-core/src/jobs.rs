//! ジョブの型と `JobSpawner` Port (docs/02-architecture.md §4.4 / §9.3)。
//!
//! ジョブはワーカーで動き、結果を `JobResult` で返す。結果は `base` (投入時の Revision) を持ち、受け取る側が
//! 古い結果を捨てる (ADR-0020)。M1 の変種は `Metadata` だけ。読込・保存・検索は、その最初の利用者の版で足す。

use std::rc::Rc;
use std::sync::Arc;

use eve_view::{DocumentId, JobId, Revision};

use crate::cancel::CancelToken;
use crate::fs::{FileMeta, FileStamp, FileSystem, FsError};
use crate::fspath::FsPath;
use crate::paths::Paths;
use crate::time::Clock;

/// ジョブの依頼。
#[derive(Clone, Debug)]
pub struct JobRequest {
    pub id: JobId,
    /// 結果の宛先の文書。文書に紐づかないジョブは `None`。
    pub doc: Option<DocumentId>,
    /// 投入時の文書の Revision。古い結果の判定に使う。
    pub base: Revision,
    pub kind: JobKind,
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum JobKind {
    /// ファイルの `metadata` と Stamp を調べる。
    Metadata { path: FsPath, purpose: MetaPurpose },
}

/// `Metadata` ジョブの目的 (結果を受けた側の分岐に使う)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetaPurpose {
    /// 未変更の保存の可否を判断する。
    UnchangedSave,
    /// 名前を付けて保存の保存先を調べる。
    SaveAsTarget,
    /// 外部変更の確認。
    ExternalCheck,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum JobResult {
    Meta {
        job: JobId,
        meta: FileMeta,
        stamp: FileStamp,
    },
    Failed {
        job: JobId,
        err: JobError,
    },
}

#[derive(Debug)]
#[non_exhaustive]
pub enum JobError {
    Fs(FsError),
    Cancelled,
    /// ワーカー内のパニック (eve-host が捕まえて変換する)。
    Panic,
    OutOfMemory {
        requested: u64,
    },
}

/// ジョブの投入 (core のスレッドから呼ぶ。待たない)。本番は eve-host、テストは `ManualJobSpawner`。
pub trait JobSpawner {
    fn spawn(&self, doc: Option<DocumentId>, base: Revision, kind: JobKind) -> JobId;
    /// 存在しない・完了済みの `JobId` は何もしない。
    fn cancel(&self, job: JobId);
}

/// ジョブの中から使う Port (ワーカースレッドに渡すので `Send + Sync`)。
#[derive(Clone)]
pub struct JobPorts {
    pub fs: Arc<dyn FileSystem>,
    pub clock: Arc<dyn Clock>,
    pub paths: Arc<dyn Paths>,
}

/// core が持つ Port の一式。
#[derive(Clone)]
pub struct Ports {
    pub fs: Arc<dyn FileSystem>,
    pub clock: Arc<dyn Clock>,
    pub jobs: Rc<dyn JobSpawner>,
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

/// ジョブを同期的に実行する。eve-host のワーカーとテストが同じ関数を使う (スレッドは呼び出し側が用意する)。
/// 開始前にキャンセルされていれば何もせず `Failed { Cancelled }` を返す。結果は `emit` に 1 つ渡す。
pub fn run_job(
    req: &JobRequest,
    ports: &JobPorts,
    cancel: &CancelToken,
    emit: &mut dyn FnMut(JobResult),
) {
    if cancel.is_cancelled() {
        emit(JobResult::Failed {
            job: req.id,
            err: JobError::Cancelled,
        });
        return;
    }
    let result = match &req.kind {
        JobKind::Metadata { path, .. } => match ports.fs.metadata(path) {
            Ok(meta) => {
                if cancel.is_cancelled() {
                    JobResult::Failed {
                        job: req.id,
                        err: JobError::Cancelled,
                    }
                } else {
                    let stamp = FileStamp::from_meta(&meta, None);
                    JobResult::Meta {
                        job: req.id,
                        meta,
                        stamp,
                    }
                }
            }
            Err(err) => JobResult::Failed {
                job: req.id,
                err: JobError::Fs(err),
            },
        },
    };
    emit(result);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::*;
    use crate::time::Tick;

    struct StubFs(Result<FileMeta, FsErrorKind>);
    impl FileSystem for StubFs {
        fn metadata(&self, _: &FsPath) -> Result<FileMeta, FsError> {
            self.0.clone().map_err(FsError::new)
        }
        fn canonicalize(&self, p: &FsPath) -> Result<FsPath, FsError> {
            Ok(p.clone())
        }
        fn open_read(&self, _: &FsPath) -> Result<Box<dyn ReadChunks + Send>, FsError> {
            Err(FsError::new(FsErrorKind::Other))
        }
        fn write_atomic(
            &self,
            _: &FsPath,
            _: &mut dyn ReadChunks,
            _: &SaveOptions,
        ) -> Result<SaveReport, FsError> {
            Err(FsError::new(FsErrorKind::Other))
        }
    }
    struct StubClock;
    impl Clock for StubClock {
        fn now(&self) -> Tick {
            Tick(0)
        }
    }
    struct StubPaths;
    impl Paths for StubPaths {
        fn config_dir(&self) -> FsPath {
            FsPath::from("/c")
        }
        fn state_dir(&self) -> FsPath {
            FsPath::from("/s")
        }
        fn data_dir(&self) -> FsPath {
            FsPath::from("/d")
        }
        fn runtime_dir(&self) -> FsPath {
            FsPath::from("/r")
        }
        fn normalize(&self, raw: &str) -> Result<FsPath, FsError> {
            Ok(FsPath::from(raw))
        }
    }

    fn meta() -> FileMeta {
        FileMeta {
            kind: FileKind::File,
            len: 3,
            mtime: FileTime::default(),
            readonly: false,
            file_id: None,
            nlink: 1,
            link: None,
            fs_kind: FsKind::Local,
            owner_is_me: None,
        }
    }

    fn ports(r: Result<FileMeta, FsErrorKind>) -> JobPorts {
        JobPorts {
            fs: Arc::new(StubFs(r)),
            clock: Arc::new(StubClock),
            paths: Arc::new(StubPaths),
        }
    }

    fn req() -> JobRequest {
        JobRequest {
            id: JobId(7),
            doc: None,
            base: Revision(0),
            kind: JobKind::Metadata {
                path: FsPath::from("/a"),
                purpose: MetaPurpose::ExternalCheck,
            },
        }
    }

    fn run(p: &JobPorts, c: &CancelToken) -> Vec<JobResult> {
        let mut out = Vec::new();
        run_job(&req(), p, c, &mut |r| out.push(r));
        out
    }

    #[test]
    fn メタ情報のジョブは_stamp_を返す() {
        let out = run(&ports(Ok(meta())), &CancelToken::new());
        assert_eq!(out.len(), 1);
        match &out[0] {
            JobResult::Meta { job, stamp, .. } => {
                assert_eq!(*job, JobId(7));
                assert_eq!(stamp.len, 3);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn 失敗は_failed_で返る() {
        let out = run(&ports(Err(FsErrorKind::NotFound)), &CancelToken::new());
        assert!(
            matches!(&out[0], JobResult::Failed { err: JobError::Fs(e), .. } if e.kind == FsErrorKind::NotFound)
        );
    }

    #[test]
    fn 開始前のキャンセルは_cancelled() {
        let c = CancelToken::new();
        c.cancel();
        let out = run(&ports(Ok(meta())), &c);
        assert_eq!(out.len(), 1);
        assert!(matches!(
            &out[0],
            JobResult::Failed {
                err: JobError::Cancelled,
                ..
            }
        ));
    }
}
