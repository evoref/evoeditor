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
    /// 設定ディレクトリの `config.toml` と `keymap.toml` を読む (`config.reload`、07 §6)。
    ReadConfig { dir: FsPath },
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
    /// 表現不能の文字を置換する ChangeSet を作る (E3 の選択肢 2、03 §8.6)。`snapshot` は投入時の Revision のもの。
    ReplaceUnmappable {
        snapshot: Snapshot,
        encoding: Encoding,
        pua: PuaEscapeMode,
    },
    /// 保存の失敗の後の「退避先に保存」(03 §9.7 (e))。`doc` の公開物を退避ディレクトリへ書く。
    WriteRecovery {
        doc: std::sync::Arc<crate::DocSnapshot>,
    },
    /// スワップを書く (03 §10.2)。`doc` は投入時の公開物で、書込中の文書は変わらない。同じ文書の Swap は同時に 1 つだけ。
    Swap {
        doc: std::sync::Arc<crate::DocSnapshot>,
    },
    /// 他のプロセスが残したスワップの本文を読む (復旧の適用、03 §10.3)。
    SwapRead { entry: crate::SwapEntry },
    /// 他のプロセスが残したスワップを消す (復旧の破棄・適用後)。
    SwapRemove { entry: crate::SwapEntry },
    /// 置換途中の痕跡の提案への答え (退避名を戻す・退避名や一時ファイルを消す、05 §4.6)。
    LeftoverAct {
        leftover: Box<crate::Leftover>,
        op: crate::LeftoverOp,
    },
    /// スワップを消す (03 §10.2「削除」)。
    SwapDiscard {
        doc: DocumentId,
        path: Option<FsPath>,
    },
    /// 文書内の全件検索 (03 §12.2)。`from` の行から末尾まで、続けて先頭から走査し、一致を `SearchBatch` で順に返して
    /// `SearchDone` で終わる。保持するのは `max_matches` 件まで、数えるのは `count_limit` 件まで。
    /// 単発の検索: `from` から先 (`forward`) か前の最初の一致 (03 §12.2「`search.next` / `search.prev`」)。
    FindNext {
        snapshot: Snapshot,
        query: crate::search::SearchQuery,
        from: eve_text::ByteOff,
        forward: bool,
    },
    /// 改行コードの変換の ChangeSet を作る (03 §8.4)。`snapshot` は投入時の Revision のもの。
    ConvertEol {
        snapshot: Snapshot,
        eol: eve_text::LineEnding,
    },
    /// すべて置換の ChangeSet を作る (03 §12.5)。`snapshot` は投入時の Revision のもの。
    ReplaceAll {
        snapshot: Snapshot,
        query: crate::search::SearchQuery,
        replacement: String,
    },
    Search {
        snapshot: Snapshot,
        query: crate::search::SearchQuery,
        from: eve_text::ByteOff,
        max_matches: usize,
        count_limit: u64,
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
    /// 巨大ファイル (`file.huge_file_warn_bytes`、FR-127)。開くと多くのメモリを使う。
    HugeFile { len: u64 },
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
    /// `ReadConfig` の結果。
    ConfigRead {
        job: JobId,
        config: ConfigFile,
        keymap: ConfigFile,
    },
    /// 起動時の退避ディレクトリの走査の結果 (新しい順ではなく列挙した順。main が整えて案内する)。
    RecoveryFound {
        job: JobId,
        dirs: Vec<RecoveryDirEntry>,
        /// 他のプロセスが残したスワップ (復旧の候補、03 §10.3)。
        swaps: Vec<crate::SwapEntry>,
        /// 置換途中の痕跡の提案 (05 §4.6)。新しい順。
        leftovers: Vec<crate::Leftover>,
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
    /// 表現不能の文字の置換の ChangeSet ができた (`targets` は先頭 100 件。総数は `counts` の合計)。
    UnmappableReplacement {
        job: JobId,
        changes: eve_text::ChangeSet,
        targets: Vec<crate::unmap::ReplaceTarget>,
        counts: crate::unmap::ReplaceCounts,
    },
    /// 「退避先に保存」の成功。`path` は実際に書いたディレクトリ (代替の退避先を含む)。
    RecoveryWritten {
        job: JobId,
        path: FsPath,
    },
    /// スワップを書いた。
    SwapWritten {
        job: JobId,
    },
    /// スワップを消した。
    SwapDiscarded {
        job: JobId,
    },
    /// 他のプロセスのスワップの本文を読んだ。
    SwapRead {
        job: JobId,
        body: crate::SwapBody,
    },
    /// 他のプロセスのスワップを消した。
    SwapRemoved {
        job: JobId,
    },
    /// 置換途中の痕跡の操作を終えた。
    LeftoverDone {
        job: JobId,
    },
    /// 単発の検索の結果。`found` は (一致, 文書の端を越えたか)。
    FindNextDone {
        job: JobId,
        found: Option<(eve_text::ByteRange, bool)>,
    },
    /// 改行コードの変換の ChangeSet ができた。
    ConvertEolDone {
        job: JobId,
        changes: eve_text::ChangeSet,
    },
    /// すべて置換の ChangeSet ができた (`count` は置換する件数)。
    ReplaceAllDone {
        job: JobId,
        changes: eve_text::ChangeSet,
        count: u64,
    },
    /// 読込の進捗 (03 §9.2 手順 3)。開く操作から 50 ms 以内に最初の 1 件を送り、以後は 10 Hz 以下。終わりではない。
    Progress {
        job: JobId,
        /// 読み終えたバイト数。
        bytes: u64,
        /// ファイルの大きさ (metadata の len)。
        total: u64,
    },
    /// 検索の途中経過。一致を走査した順に届く (終わりではない)。
    SearchBatch {
        job: JobId,
        matches: Vec<eve_text::ByteRange>,
    },
    /// 検索の終わり。`total` は数えた件数、`truncated` は保持の上限を超えた、`count_limited` は数える上限で打ち切った。
    SearchDone {
        job: JobId,
        total: u64,
        count_limited: bool,
        truncated: bool,
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
            | JobResult::ConfigRead { job, .. }
            | JobResult::Saved { job, .. }
            | JobResult::RecoveryWritten { job, .. }
            | JobResult::SwapWritten { job }
            | JobResult::SwapDiscarded { job }
            | JobResult::SwapRead { job, .. }
            | JobResult::SwapRemoved { job }
            | JobResult::LeftoverDone { job }
            | JobResult::UnmappableReplacement { job, .. }
            | JobResult::Unmappable { job, .. }
            | JobResult::ReplaceAllDone { job, .. }
            | JobResult::ConvertEolDone { job, .. }
            | JobResult::Progress { job, .. }
            | JobResult::FindNextDone { job, .. }
            | JobResult::SearchBatch { job, .. }
            | JobResult::SearchDone { job, .. }
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
            swaps: ports.fs.list_swaps().unwrap_or_default(),
            leftovers: crate::leftover::scan(&*ports.fs),
        },
        JobKind::Open { path, opts } => run_open(req.id, path, *opts, ports, cancel, emit),
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
        JobKind::ReplaceUnmappable {
            snapshot,
            encoding,
            pua,
        } => match crate::unmap::build_replacement(snapshot, *encoding, *pua, cancel) {
            Ok(Some((changes, targets, counts))) => JobResult::UnmappableReplacement {
                job: req.id,
                changes,
                targets,
                counts,
            },
            // 対象が無い (文書が変わって表現不能が消えた等): 空の置換として返す
            Ok(None) => JobResult::UnmappableReplacement {
                job: req.id,
                changes: eve_text::ChangeSet::identity(eve_text::TextRead::len_bytes(snapshot)),
                targets: Vec::new(),
                counts: crate::unmap::ReplaceCounts::default(),
            },
            Err(()) => JobResult::Failed {
                job: req.id,
                err: JobError::Cancelled,
            },
        },
        JobKind::FindNext {
            snapshot,
            query,
            from,
            forward,
        } => match crate::search::find_next(snapshot, query, *from, *forward, cancel) {
            Some(found) => JobResult::FindNextDone { job: req.id, found },
            None => JobResult::Failed {
                job: req.id,
                err: JobError::Cancelled,
            },
        },
        JobKind::ConvertEol { snapshot, eol } => {
            match crate::eol::build_eol_conversion(snapshot, *eol, cancel) {
                Some(changes) => JobResult::ConvertEolDone {
                    job: req.id,
                    changes,
                },
                None => JobResult::Failed {
                    job: req.id,
                    err: JobError::Cancelled,
                },
            }
        }
        JobKind::ReplaceAll {
            snapshot,
            query,
            replacement,
        } => match crate::search::build_replace_all(snapshot, query, replacement, cancel) {
            Some((changes, count)) => JobResult::ReplaceAllDone {
                job: req.id,
                changes,
                count,
            },
            None => JobResult::Failed {
                job: req.id,
                err: JobError::Cancelled,
            },
        },
        JobKind::Search {
            snapshot,
            query,
            from,
            max_matches,
            count_limit,
        } => {
            crate::search::run_search(
                crate::search::SearchJob {
                    job: req.id,
                    snapshot,
                    query,
                    from: *from,
                    max_matches: *max_matches,
                    count_limit: *count_limit,
                },
                &*ports.clock,
                cancel,
                emit,
            );
            return;
        }
        JobKind::WriteRecovery { doc } => match ports.fs.write_recovery(doc) {
            Ok(path) => JobResult::RecoveryWritten { job: req.id, path },
            Err(err) => JobResult::Failed {
                job: req.id,
                err: JobError::Fs(err),
            },
        },
        JobKind::Swap { doc } => match ports.fs.write_swap(doc) {
            Ok(()) => JobResult::SwapWritten { job: req.id },
            Err(err) => JobResult::Failed {
                job: req.id,
                err: JobError::Fs(err),
            },
        },
        JobKind::SwapRead { entry } => match ports.fs.read_swap(entry) {
            Ok(body) => JobResult::SwapRead { job: req.id, body },
            Err(err) => JobResult::Failed {
                job: req.id,
                err: JobError::Fs(err),
            },
        },
        JobKind::SwapRemove { entry } => match ports.fs.remove_swap(entry) {
            Ok(()) => JobResult::SwapRemoved { job: req.id },
            Err(err) => JobResult::Failed {
                job: req.id,
                err: JobError::Fs(err),
            },
        },
        JobKind::LeftoverAct { leftover, op } => {
            match crate::leftover::act(&*ports.fs, leftover, *op) {
                Ok(()) => JobResult::LeftoverDone { job: req.id },
                Err(err) => JobResult::Failed {
                    job: req.id,
                    err: JobError::Fs(err),
                },
            }
        }
        JobKind::SwapDiscard { doc, path } => match ports.fs.discard_swap(*doc, path.as_ref()) {
            Ok(()) => JobResult::SwapDiscarded { job: req.id },
            Err(err) => JobResult::Failed {
                job: req.id,
                err: JobError::Fs(err),
            },
        },
        JobKind::ReadConfig { dir } => JobResult::ConfigRead {
            job: req.id,
            config: read_config_file(&*ports.fs, &dir.join("config.toml")),
            keymap: read_config_file(&*ports.fs, &dir.join("keymap.toml")),
        },
        JobKind::Metadata { path, purpose } => match ports.fs.metadata(path) {
            Ok(meta) => {
                if cancel.is_cancelled() {
                    JobResult::Failed {
                        job: req.id,
                        err: JobError::Cancelled,
                    }
                } else {
                    // 未変更の保存の可否は、16 MiB 以下なら内容ハッシュでも照合する (05 §5.5)。読めなければ付けない
                    let hash = if *purpose == MetaPurpose::UnchangedSave {
                        crate::content_hash::hash_file(ports.fs.as_ref(), path, meta.len)
                            .ok()
                            .flatten()
                    } else {
                        None
                    };
                    let stamp = FileStamp::from_meta(&meta, hash);
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

/// 設定ファイル 1 つの読込の結果。無ければ `Ok(None)`、読めなければ理由。
pub type ConfigFile = Result<Option<Vec<u8>>, String>;

/// 設定ファイルの大きさの上限 (これを超えるものは設定ではないとみなして読まない)。
const CONFIG_FILE_MAX: usize = 4 << 20;

fn read_config_file(fs: &dyn crate::fs::FileSystem, path: &FsPath) -> ConfigFile {
    let mut r = match fs.open_read(path) {
        Ok(r) => r,
        Err(e) if e.kind == crate::fs::FsErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{:?}", e.kind)),
    };
    let mut out = Vec::new();
    loop {
        match r.next_chunk() {
            Ok(Some(c)) => {
                out.extend_from_slice(c);
                if out.len() > CONFIG_FILE_MAX {
                    return Err("too large".to_owned());
                }
            }
            Ok(None) => return Ok(Some(out)),
            Err(e) => return Err(format!("{e:?}")),
        }
    }
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
