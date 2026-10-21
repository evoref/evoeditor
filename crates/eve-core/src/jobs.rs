//! ジョブの型と `JobSpawner` Port (docs/02-architecture.md §4.4 / §9.3)。
//!
//! ジョブはワーカーで動き、結果を `JobResult` で返す。結果は `base` (投入時の Revision) を持ち、受け取る側が
//! 古い結果を捨てる (ADR-0020)。M1 の変種は `Metadata`・`Open`・`Save`。検索などは、その最初の利用者の版で足す。

use std::rc::Rc;
use std::sync::Arc;

use eve_view::{DocumentId, JobId, Revision};

use crate::cancel::CancelToken;
use eve_encoding::{Encoding, PuaEscapeMode, UnsupportedBom};
use eve_text::{Snapshot, Text};

use crate::document::{ReadOnlyReasons, TextFormat};
use crate::fs::{
    Expect, FileMeta, FileStamp, FileSystem, FsError, HardlinkPolicy, InPlaceBackup,
    InterruptedSave, RecoveryDirEntry, SaveReport,
};
use crate::fspath::FsPath;
use crate::io_jobs::{OpenOptions, SaveParams, run_open, run_save};
use crate::paths::Paths;
use crate::time::Clock;

/// 保存の方式 (03 §9.3)。既定は原子的保存。確認 (`AtomicUnsupported` / ハードリンク / `BackupFailed`) の後で直接上書きにする。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveStrategy {
    /// 一時ファイル + 置換 (`write_atomic`)。`hardlink` は `nlink > 1` の方針。
    Atomic { hardlink: HardlinkPolicy },
    /// 直接上書き (`write_in_place`)。`backup = SkipConfirmed` は「バックアップなしで上書き」を承認した再投入。
    InPlace { backup: InPlaceBackup },
}

impl Default for SaveStrategy {
    fn default() -> SaveStrategy {
        SaveStrategy::Atomic {
            hardlink: HardlinkPolicy::Ask,
        }
    }
}

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
    /// 起動時の退避ディレクトリの走査 (02 §9.3 の `RecoveryScan`。メインスレッドでは列挙しない)。
    RecoveryScan,
    /// ファイルを開く (03 §9.2)。文字コードは判定する (03 §8.2) か、`opts.forced` で指定する。
    Open { path: FsPath, opts: OpenOptions },
    /// Snapshot を `encoding` で原子的に保存する (03 §9.3 / §8.6)。表現不能の文字があれば何も書かず `JobResult::Unmappable`。`expected` は書く前と置換の直前に照合する。
    Save {
        path: FsPath,
        snapshot: Snapshot,
        encoding: Encoding,
        bom: bool,
        pua: PuaEscapeMode,
        expected: Expect,
        strategy: SaveStrategy,
    },
    /// 保存の失敗の後の「退避先に保存」(03 §9.7 (e))。`doc` の公開物を退避ディレクトリへ書く。
    WriteRecovery {
        doc: std::sync::Arc<crate::DocSnapshot>,
    },
}

/// 確認が要る理由 (03 §9.2)。ジョブは確認を求めて終了し、回答を得た main が再投入する。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfirmAsk {
    /// バイナリの疑い。
    Binary,
    /// 対応していない BOM (UTF-32 など)。
    UnsupportedBom { kind: UnsupportedBom },
    /// 不正バイトが多い (sniff 範囲で 1% 超、03 §8.3)。`permille` は千分率 (本文の構築を始める前に判定する)。
    ManyInvalidBytes { encoding: Encoding, permille: u32 },
}

/// 読込の完了に添える通知 (info メッセージの材料)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OpenNotes {
    /// 先頭の判定が外れて、判定し直して読み直した (03 §8.2.1。`info.open.redetected`)。
    pub redetected: Option<Encoding>,
    /// chardetng が 7 種にない文字コードを推定した (その名前。`info.open.guess_unsupported`)。
    pub guess_unsupported: Option<&'static str>,
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
    /// 読込の完了。
    Complete {
        job: JobId,
        text: Text,
        format: TextFormat,
        meta: FileMeta,
        stamp: FileStamp,
        /// 読んでいる間にファイルが変わった、または読む量の上限で打ち切った。
        changed_during_load: bool,
        /// 内容・属性から決まる読み取り専用の理由。
        detected: ReadOnlyReasons,
        notes: OpenNotes,
        /// 残っていた保存中フラグから見つけた、中断された保存の痕跡 (直接上書きの中断など、存在するファイルでも返す)。
        interrupted_save: Option<InterruptedSave>,
    },
    /// 起動時の退避ディレクトリの走査の結果 (新しい順ではなく列挙した順。main が整えて案内する)。
    RecoveryFound {
        job: JobId,
        dirs: Vec<RecoveryDirEntry>,
    },
    /// 存在しないパスを開いた (新規ファイル)。フォルダも無ければ `parent_missing` にそのフォルダ。
    NewFile {
        job: JobId,
        parent_missing: Option<FsPath>,
        /// 残っていた保存中フラグから見つけた、中断された保存の痕跡 (新規作成の途中で中断した、置換の途中で退避名に残った、等)。
        interrupted_save: Option<InterruptedSave>,
    },
    NeedsConfirm {
        job: JobId,
        ask: ConfirmAsk,
    },
    Saved {
        job: JobId,
        report: SaveReport,
    },
    /// 保存先の文字コードで表現できない文字がある (03 §8.6 E1)。元のファイルには何も書いていない。
    Unmappable {
        job: JobId,
        /// 総数。
        total: u64,
        /// 先頭 100 件。
        recorded: Vec<eve_encoding::Unmappable>,
        encoding: Encoding,
    },
    /// 「退避先に保存」の成功。`path` は実際に書いたディレクトリ (代替の退避先を含む)。
    RecoveryWritten {
        job: JobId,
        path: FsPath,
    },
    Failed {
        job: JobId,
        err: JobError,
    },
}

impl JobResult {
    /// 結果が属するジョブ。
    pub fn job(&self) -> JobId {
        match self {
            JobResult::Meta { job, .. }
            | JobResult::Complete { job, .. }
            | JobResult::NewFile { job, .. }
            | JobResult::NeedsConfirm { job, .. }
            | JobResult::RecoveryFound { job, .. }
            | JobResult::Saved { job, .. }
            | JobResult::RecoveryWritten { job, .. }
            | JobResult::Unmappable { job, .. }
            | JobResult::Failed { job, .. } => *job,
        }
    }
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
        JobKind::RecoveryScan => JobResult::RecoveryFound {
            job: req.id,
            dirs: ports.fs.list_recovery_dirs().unwrap_or_default(),
        },
        JobKind::Open { path, opts } => run_open(req.id, path, *opts, ports, cancel),
        JobKind::Save {
            path,
            snapshot,
            encoding,
            bom,
            pua,
            expected,
            strategy,
        } => run_save(
            req.id,
            SaveParams {
                path,
                snapshot,
                encoding: *encoding,
                bom: *bom,
                pua: *pua,
                expected: *expected,
                strategy: *strategy,
            },
            ports,
            cancel,
        ),
        JobKind::WriteRecovery { doc } => match ports.fs.write_recovery(doc) {
            Ok(path) => JobResult::RecoveryWritten { job: req.id, path },
            Err(err) => JobResult::Failed {
                job: req.id,
                err: JobError::Fs(err),
            },
        },
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
