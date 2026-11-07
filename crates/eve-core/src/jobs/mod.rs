//! ジョブの型と `JobSpawner` Port (docs/02-architecture.md §4.4 / §9.3)。
//!
//! ジョブはワーカーで動き、結果を `JobResult` で返す。結果は `base` (投入時の Revision) を持ち、受け取る側が
//! 古い結果を捨てる (ADR-0020)。変種は `JobKind` を参照。
//!
//! spec: 02 §4.4, 02 §9.3

pub(crate) mod cancel;

use std::rc::Rc;
use std::sync::Arc;

use eve_view::{DocumentId, JobId, Revision};

use crate::jobs::cancel::CancelToken;
use eve_encoding::{Encoding, EscapePolicy, UnsupportedBom};
use eve_text::{Snapshot, Text};

use crate::document::{ReadOnlyReasons, TextFormat};
use crate::io::fs::{
    Expect, FileMeta, FileStamp, FileSystem, FsError, HardlinkPolicy, InPlaceBackup,
    InterruptedSave, RecoveryDirEntry, SaveReport,
};
use crate::io::fspath::FsPath;
use crate::io::io_jobs::{OpenOptions, SaveParams, run_open, run_save};
use crate::io::paths::Paths;
use crate::time::Clock;

/// 保存の方式 (03 §9.3)。既定は原子的保存。確認 (`AtomicUnsupported` / ハードリンク / `BackupFailed`) の後で直接上書きにする。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveStrategy {
    /// 一時ファイル + 置換 (`write_atomic`)。`hardlink` は `nlink > 1` の方針。
    Atomic {
        /// 保存先のハードリンク (`nlink > 1`) の扱い (03 §9.3)。
        hardlink: HardlinkPolicy,
    },
    /// 直接上書き (`write_in_place`)。`backup = SkipConfirmed` は「バックアップなしで上書き」を承認した再投入。
    InPlace {
        /// 直接上書きの前のバックアップの扱い (03 §9.3)。
        backup: InPlaceBackup,
    },
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
    /// このジョブの識別子 (`JobSpawner::spawn` が振る)。
    pub id: JobId,
    /// 結果の宛先の文書。文書に紐づかないジョブは `None`。
    pub doc: Option<DocumentId>,
    /// 投入時の文書の Revision。古い結果の判定に使う。
    pub base: Revision,
    /// ジョブの種類と入力。
    pub kind: JobKind,
}

/// ワーカーで動くジョブの種類と入力 (02 §4.4 / §9.3)。結果は対応する `JobResult` の変種で返る。
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum JobKind {
    /// テーマファイル `dir/<name>.toml` を `inherits` の親までたどって読み、テーマを組み立てる (07 §8.6)。
    ReadTheme {
        /// テーマファイルを置くディレクトリ。
        dir: FsPath,
        /// テーマ名 (拡張子 `.toml` を除いたファイル名)。
        name: String,
    },
    /// 設定ファイル (ユーザー層の `config.toml` を順に) と `keymap.toml` を読む (`config.reload`、07 §6)。
    ReadConfig {
        /// 読む設定ファイル。層の低い順。
        files: Vec<FsPath>,
        /// 読む `keymap.toml`。`--no-config` のときは `None`。
        keymap: Option<FsPath>,
    },
    /// ファイルの `metadata` と Stamp を調べる。
    Metadata {
        /// 調べるパス。
        path: FsPath,
        /// 結果を受けた側の分岐に使う目的。
        purpose: MetaPurpose,
    },
    /// 起動時の退避ディレクトリの走査 (02 §9.3 の `RecoveryScan`。メインスレッドでは列挙しない)。
    RecoveryScan,
    /// 前回起動した版の記録を読み、今の版 `current` と比べて書き直す (FR-518、07 §18.8)。
    LastVersion {
        /// 今の版 (`CARGO_PKG_VERSION` の形の文字列)。
        current: String,
    },
    /// ファイルを開く (03 §9.2)。文字コードは判定する (03 §8.2) か、`opts.forced` で指定する。
    Open {
        /// 開くパス。
        path: FsPath,
        /// 開くときの指定 (文字コードの強制など)。
        opts: OpenOptions,
    },
    /// Snapshot を `encoding` で原子的に保存する (03 §9.3 / §8.6)。表現不能の文字があれば何も書かず `JobResult::Unmappable`。`expected` は書く前と置換の直前に照合する。
    Save {
        /// 保存先のパス。
        path: FsPath,
        /// 保存する本文 (投入時の Revision のもの)。
        snapshot: Snapshot,
        /// 保存先の文字コード。
        encoding: Encoding,
        /// 先頭に BOM を書くか。
        bom: bool,
        /// 不正バイトのエスケープ文字の扱い。core が `TextFormat::save_escape_policy` で決める (03 §8.3)。
        escape: EscapePolicy,
        /// 保存先の期待する状態 (外部変更の検出に使う)。
        expected: Expect,
        /// 原子的保存か直接上書きか。
        strategy: SaveStrategy,
        /// 「読み取り専用を解除して保存」(FR-116)。保存先の読み取り専用属性を外して書き、書いた後に付け直す。
        clear_readonly: bool,
        /// ダングリングのシンボリックリンクのリンク先に新規作成する (FR-106。確認を経た保存だけ)。
        create_link_target: bool,
    },
    /// 表現不能の文字を置換する `ChangeSet` を作る (E3 の選択肢 2、03 §8.6)。`snapshot` は投入時の Revision のもの。
    ReplaceUnmappable {
        /// 置換の対象の本文。
        snapshot: Snapshot,
        /// 表現できるかを調べる文字コード。
        encoding: Encoding,
        /// 不正バイトのエスケープ文字の扱い (03 §8.3)。
        escape: EscapePolicy,
    },
    /// 保存の失敗の後の「退避先に保存」(03 §9.7 (e))。`doc` の公開物を退避ディレクトリへ書く。
    WriteRecovery {
        /// 退避する文書の公開物。
        doc: std::sync::Arc<crate::DocSnapshot>,
    },
    /// スワップを書く (03 §10.2)。`doc` は投入時の公開物で、書込中の文書は変わらない。同じ文書の Swap は同時に 1 つだけ。
    Swap {
        /// スワップに書く文書の公開物。
        doc: std::sync::Arc<crate::DocSnapshot>,
    },
    /// 他のプロセスが残したスワップの本文を読む (復旧の適用、03 §10.3)。
    SwapRead {
        /// 読むスワップ。
        entry: crate::SwapEntry,
    },
    /// 他のプロセスが残したスワップを消す (復旧の破棄・適用後)。
    SwapRemove {
        /// 消すスワップ。
        entry: crate::SwapEntry,
    },
    /// 置換途中の痕跡の提案への答え (退避名を戻す・退避名や一時ファイルを消す、05 §4.6)。
    LeftoverAct {
        /// 対象の痕跡。
        leftover: Box<crate::Leftover>,
        /// 利用者が選んだ操作。
        op: crate::LeftoverOp,
    },
    /// スワップを消す (03 §10.2「削除」)。
    SwapDiscard {
        /// スワップの持ち主の文書。
        doc: DocumentId,
        /// 文書のパス (無題の文書は `None`)。
        path: Option<FsPath>,
    },
    /// 単発の検索: `from` から先 (`forward`) か前の最初の一致 (03 §12.2「`search.next` / `search.prev`」)。
    FindNext {
        /// 検索する本文。
        snapshot: Snapshot,
        /// 検索の条件。
        query: crate::search::SearchQuery,
        /// 探し始める位置 (バイト)。
        from: eve_text::ByteOff,
        /// `true` なら後ろへ、`false` なら前へ探す。
        forward: bool,
    },
    /// 改行コードの変換の `ChangeSet` を作る (03 §8.4)。`snapshot` は投入時の Revision のもの。
    ConvertEol {
        /// 変換の対象の本文。
        snapshot: Snapshot,
        /// 変換後の改行コード。
        eol: eve_text::LineEnding,
    },
    /// すべて置換の `ChangeSet` を作る (03 §12.5)。`snapshot` は投入時の Revision のもの。
    ReplaceAll {
        /// 置換の対象の本文。
        snapshot: Snapshot,
        /// 検索の条件。
        query: crate::search::SearchQuery,
        /// 置換後の文字列。
        replacement: String,
    },
    /// 文書内の全件検索 (03 §12.2)。`from` の行から末尾まで、続けて先頭から走査し、一致を `SearchBatch` で順に返して
    /// `SearchDone` で終わる。保持するのは `max_matches` 件まで、数えるのは `count_limit` 件まで。
    Search {
        /// 検索する本文。
        snapshot: Snapshot,
        /// 検索の条件。
        query: crate::search::SearchQuery,
        /// 走査を始める位置 (バイト)。
        from: eve_text::ByteOff,
        /// 保持する一致の上限 (件)。
        max_matches: usize,
        /// 数える一致の上限 (件)。
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
    UnsupportedBom {
        /// 見つかった BOM の種類。
        kind: UnsupportedBom,
    },
    /// 不正バイトが多い (sniff 範囲で 1% 超、03 §8.3)。`permille` は千分率 (本文の構築を始める前に判定する)。
    ManyInvalidBytes {
        /// 判定した文字コード。
        encoding: Encoding,
        /// sniff 範囲の不正バイトの割合 (千分率)。
        permille: u32,
    },
    /// 巨大ファイル (`file.huge_file_warn_bytes`、FR-127)。開くと多くのメモリを使う。
    HugeFile {
        /// ファイルの大きさ (バイト)。
        len: u64,
    },
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

/// ジョブの結果 (02 §4.4)。どの変種も `job` (結果が属するジョブ) を持つ。受け取る側は投入時の Revision で古い結果を捨てる (ADR-0020)。
#[derive(Debug)]
#[non_exhaustive]
pub enum JobResult {
    /// `Metadata` の結果。
    Meta {
        /// 結果が属するジョブ。
        job: JobId,
        /// ファイルの属性。
        meta: FileMeta,
        /// 外部変更の検出に使う Stamp。
        stamp: FileStamp,
    },
    /// 読込の完了。
    Complete {
        /// 結果が属するジョブ。
        job: JobId,
        /// 読み込んだ本文。
        text: Text,
        /// 判定した形式 (文字コード・BOM・改行コードなど)。
        format: TextFormat,
        /// 読んだファイルの属性。
        meta: FileMeta,
        /// 読んだ時点の Stamp。
        stamp: FileStamp,
        /// 読んでいる間にファイルが変わった、または読む量の上限で打ち切った。
        changed_during_load: bool,
        /// 内容・属性から決まる読み取り専用の理由。
        detected: ReadOnlyReasons,
        /// info メッセージの材料。
        notes: OpenNotes,
        /// 残っていた保存中フラグから見つけた、中断された保存の痕跡 (直接上書きの中断など、存在するファイルでも返す)。
        interrupted_save: Option<InterruptedSave>,
    },
    /// `ReadTheme` の結果。`theme` が `None` なら読めなかった (組込みの `default` で動く)。
    ThemeRead {
        /// 結果が属するジョブ。
        job: JobId,
        /// 読もうとしたテーマ名。
        name: String,
        /// 組み立てたテーマ。
        theme: Option<eve_config::theme::Theme>,
        /// ファイルごとの診断 (ファイルの名前と診断の組)。
        reports: Vec<(String, Vec<eve_config::Diagnostic>)>,
    },
    /// `ReadConfig` の結果。
    ConfigRead {
        /// 結果が属するジョブ。
        job: JobId,
        /// `files` と同じ順。
        config: Vec<ConfigFile>,
        /// `keymap` を読まない (`--no-config`) ときは `None`。
        keymap: Option<ConfigFile>,
    },
    /// 起動時の退避ディレクトリの走査の結果 (新しい順ではなく列挙した順。main が整えて案内する)。
    RecoveryFound {
        /// 結果が属するジョブ。
        job: JobId,
        /// 見つけた退避ディレクトリ。
        dirs: Vec<RecoveryDirEntry>,
        /// 他のプロセスが残したスワップ (復旧の候補、03 §10.3)。
        swaps: Vec<crate::SwapEntry>,
        /// 置換途中の痕跡の提案 (05 §4.6)。新しい順。
        leftovers: Vec<crate::Leftover>,
    },
    /// 存在しないパスを開いた (新規ファイル)。フォルダも無ければ `parent_missing` にそのフォルダ。
    NewFile {
        /// 結果が属するジョブ。
        job: JobId,
        /// 存在しない親フォルダ (あれば)。
        parent_missing: Option<FsPath>,
        /// 残っていた保存中フラグから見つけた、中断された保存の痕跡 (新規作成の途中で中断した、置換の途中で退避名に残った、等)。
        interrupted_save: Option<InterruptedSave>,
    },
    /// 続ける前に利用者の確認が要る (03 §9.2)。回答を得た main が再投入する。
    NeedsConfirm {
        /// 結果が属するジョブ。
        job: JobId,
        /// 確認が要る理由。
        ask: ConfirmAsk,
    },
    /// 保存の完了。
    Saved {
        /// 結果が属するジョブ。
        job: JobId,
        /// 保存の報告。
        report: SaveReport,
    },
    /// 保存先の文字コードで表現できない文字がある (03 §8.6 E1)。元のファイルには何も書いていない。
    Unmappable {
        /// 結果が属するジョブ。
        job: JobId,
        /// 総数。
        total: u64,
        /// 先頭 100 件。
        recorded: Vec<eve_encoding::Unmappable>,
        /// 保存先の文字コード。
        encoding: Encoding,
    },
    /// 表現不能の文字の置換の `ChangeSet` ができた (`targets` は先頭 100 件。総数は `counts` の合計)。
    UnmappableReplacement {
        /// 結果が属するジョブ。
        job: JobId,
        /// 置換の `ChangeSet` (投入時の Revision に対するもの)。
        changes: eve_text::ChangeSet,
        /// 置換する箇所 (先頭 100 件)。
        targets: Vec<crate::io::unmap::ReplaceTarget>,
        /// 置換の種類ごとの件数。
        counts: crate::io::unmap::ReplaceCounts,
    },
    /// 「退避先に保存」の成功。`path` は実際に書いたディレクトリ (代替の退避先を含む)。
    RecoveryWritten {
        /// 結果が属するジョブ。
        job: JobId,
        /// 実際に書いた退避ディレクトリ。
        path: FsPath,
    },
    /// スワップを書いた。
    SwapWritten {
        /// 結果が属するジョブ。
        job: JobId,
    },
    /// スワップを消した。
    SwapDiscarded {
        /// 結果が属するジョブ。
        job: JobId,
    },
    /// 他のプロセスのスワップの本文を読んだ。
    SwapRead {
        /// 結果が属するジョブ。
        job: JobId,
        /// 読んだスワップの本文。
        body: crate::SwapBody,
    },
    /// 他のプロセスのスワップを消した。
    SwapRemoved {
        /// 結果が属するジョブ。
        job: JobId,
    },
    /// 置換途中の痕跡の操作を終えた。
    LeftoverDone {
        /// 結果が属するジョブ。
        job: JobId,
    },
    /// 前回起動した版との比較の結果 (`LastVersion`)。
    LastVersionChecked {
        /// 結果が属するジョブ。
        job: JobId,
        /// 比較の結果。
        outcome: crate::LastVersionOutcome,
    },
    /// 単発の検索の結果。`found` は (一致, 文書の端を越えたか)。
    FindNextDone {
        /// 結果が属するジョブ。
        job: JobId,
        /// 見つけた一致 (バイト範囲) と、文書の端を越えて折り返したか。見つからなければ `None`。
        found: Option<(eve_text::ByteRange, bool)>,
    },
    /// 改行コードの変換の `ChangeSet` ができた。
    ConvertEolDone {
        /// 結果が属するジョブ。
        job: JobId,
        /// 変換の `ChangeSet` (投入時の Revision に対するもの)。
        changes: eve_text::ChangeSet,
    },
    /// すべて置換の `ChangeSet` ができた (`count` は置換する件数)。
    ReplaceAllDone {
        /// 結果が属するジョブ。
        job: JobId,
        /// 置換の `ChangeSet` (投入時の Revision に対するもの)。
        changes: eve_text::ChangeSet,
        /// 置換する件数。
        count: u64,
    },
    /// 読込の進捗 (03 §9.2 手順 3)。開く操作から 50 ms 以内に最初の 1 件を送り、以後は 10 Hz 以下。終わりではない。
    Progress {
        /// 結果が属するジョブ。
        job: JobId,
        /// 読み終えたバイト数。
        bytes: u64,
        /// ファイルの大きさ (metadata の len)。
        total: u64,
    },
    /// 検索の途中経過。一致を走査した順に届く (終わりではない)。
    SearchBatch {
        /// 結果が属するジョブ。
        job: JobId,
        /// 今回の一致 (バイト範囲、走査した順)。
        matches: Vec<eve_text::ByteRange>,
    },
    /// 検索の終わり。`total` は数えた件数、`truncated` は保持の上限を超えた、`count_limited` は数える上限で打ち切った。
    SearchDone {
        /// 結果が属するジョブ。
        job: JobId,
        /// 数えた一致の件数。
        total: u64,
        /// 数える上限 (`count_limit`) で打ち切った。
        count_limited: bool,
        /// 保持の上限 (`max_matches`) を超えた。
        truncated: bool,
    },
    /// ジョブの失敗 (キャンセルを含む)。
    Failed {
        /// 結果が属するジョブ。
        job: JobId,
        /// 失敗の理由。
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
            | JobResult::ThemeRead { job, .. }
            | JobResult::Saved { job, .. }
            | JobResult::RecoveryWritten { job, .. }
            | JobResult::SwapWritten { job }
            | JobResult::SwapDiscarded { job }
            | JobResult::SwapRead { job, .. }
            | JobResult::SwapRemoved { job }
            | JobResult::LeftoverDone { job }
            | JobResult::LastVersionChecked { job, .. }
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

/// ジョブの失敗の理由。
#[derive(Debug)]
#[non_exhaustive]
pub enum JobError {
    /// ファイルシステムの操作の失敗。
    Fs(FsError),
    /// `CancelToken` でキャンセルされた。
    Cancelled,
    /// ワーカー内のパニック (eve-host が捕まえて変換する)。
    Panic,
    /// メモリを確保できない (INV-IO4、02 §11.2.2)。プロセスは続ける。
    OutOfMemory {
        /// 確保しようとした量 (バイト)。
        requested: u64,
    },
}

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
        JobKind::LastVersion { current } => JobResult::LastVersionChecked {
            job: req.id,
            outcome: crate::io::last_version::check_and_record(&*ports.fs, current),
        },
        JobKind::RecoveryScan => JobResult::RecoveryFound {
            job: req.id,
            dirs: ports.fs.list_recovery_dirs().unwrap_or_default(),
            swaps: ports.fs.list_swaps().unwrap_or_default(),
            leftovers: crate::io::leftover::scan(&*ports.fs),
        },
        JobKind::Open { path, opts } => run_open(req.id, path, *opts, ports, cancel, emit),
        JobKind::Save {
            path,
            snapshot,
            encoding,
            bom,
            escape,
            expected,
            strategy,
            clear_readonly,
            create_link_target,
        } => run_save(
            req.id,
            SaveParams {
                path,
                snapshot,
                encoding: *encoding,
                bom: *bom,
                escape: *escape,
                expected: *expected,
                strategy: *strategy,
                clear_readonly: *clear_readonly,
                create_link_target: *create_link_target,
            },
            ports,
            cancel,
        ),
        JobKind::ReplaceUnmappable {
            snapshot,
            encoding,
            escape,
        } => match crate::io::unmap::build_replacement(snapshot, *encoding, *escape, cancel) {
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
                counts: crate::io::unmap::ReplaceCounts::default(),
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
            match crate::document::eol::build_eol_conversion(snapshot, *eol, cancel) {
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
            match crate::io::leftover::act(&*ports.fs, leftover, *op) {
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
        JobKind::ReadTheme { dir, name } => {
            let read = |n: &str| -> Result<Option<String>, String> {
                match read_config_file(&*ports.fs, &dir.join(format!("{n}.toml"))) {
                    Ok(Some(b)) => String::from_utf8(b)
                        .map(Some)
                        .map_err(|_| "UTF-8 として読めません".to_owned()),
                    Ok(None) => Ok(None),
                    Err(e) => Err(e),
                }
            };
            let (theme, reports) = eve_config::theme_file::load_theme(name, &read);
            JobResult::ThemeRead {
                job: req.id,
                name: name.clone(),
                theme,
                reports,
            }
        }
        JobKind::ReadConfig { files, keymap } => JobResult::ConfigRead {
            job: req.id,
            config: files
                .iter()
                .map(|f| read_config_file(&*ports.fs, f))
                .collect(),
            keymap: keymap.as_ref().map(|k| read_config_file(&*ports.fs, k)),
        },
        JobKind::Metadata { path, purpose } => match ports.fs.metadata(path) {
            Ok(meta) => {
                if cancel.is_cancelled() {
                    JobResult::Failed {
                        job: req.id,
                        err: JobError::Cancelled,
                    }
                } else {
                    // 未変更の保存の可否と外部変更の確認は、16 MiB 以下なら内容ハッシュでも照合する (05 §5.5)。読めなければ付けない
                    let hash = if matches!(
                        purpose,
                        MetaPurpose::UnchangedSave | MetaPurpose::ExternalCheck
                    ) {
                        crate::io::content_hash::hash_file(ports.fs.as_ref(), path, meta.len)
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

fn read_config_file(fs: &dyn crate::io::fs::FileSystem, path: &FsPath) -> ConfigFile {
    let mut r = match fs.open_read(path) {
        Ok(r) => r,
        Err(e) if e.kind == crate::io::fs::FsErrorKind::NotFound => return Ok(None),
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
    use crate::io::fs::*;
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

    /// `write_atomic` の途中で取消しが届き (利用者が「保存を中止して終了」を選んだ)、そのあと Port が `kind` で失敗する FS。
    struct CancelledMidSave {
        cancel: CancelToken,
        kind: FsErrorKind,
    }
    impl FileSystem for CancelledMidSave {
        fn metadata(&self, _: &FsPath) -> Result<FileMeta, FsError> {
            Ok(meta())
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
            self.cancel.cancel();
            Err(FsError::new(self.kind.clone()))
        }
    }

    fn save_cancelled_mid(kind: FsErrorKind) -> JobResult {
        let cancel = CancelToken::new();
        let p = JobPorts {
            fs: Arc::new(CancelledMidSave {
                cancel: cancel.clone(),
                kind,
            }),
            clock: Arc::new(StubClock),
            paths: Arc::new(StubPaths),
        };
        let r = JobRequest {
            id: JobId(9),
            doc: None,
            base: Revision(0),
            kind: JobKind::Save {
                path: FsPath::from("/a"),
                snapshot: Text::from("abc").snapshot(),
                encoding: Encoding::Utf8,
                bom: false,
                escape: EscapePolicy::Restore,
                expected: Expect::Any,
                strategy: SaveStrategy::default(),
                clear_readonly: false,
                create_link_target: false,
            },
        };
        let mut out = Vec::new();
        run_job(&r, &p, &cancel, &mut |x| out.push(x));
        out.pop().expect("終端の結果")
    }

    // 05 §4.1 手順 5、NFR-301 AC-2 (docs/review/v0.3-findings.md の SAF-04)
    #[test]
    fn 置換を始めた後の失敗は_取消しが届いていても失敗のまま返す() {
        let backup = FsPath::from("/.a.eve-bak-1");
        let r = save_cancelled_mid(FsErrorKind::ReplacedMovedToBackup {
            backup: backup.clone(),
        });
        assert!(
            matches!(&r, JobResult::Failed { err: JobError::Fs(e), .. }
                if e.kind == FsErrorKind::ReplacedMovedToBackup { backup: backup.clone() }),
            "{r:?}"
        );
        let r = save_cancelled_mid(FsErrorKind::InPlaceFailed { backup: None });
        assert!(
            matches!(&r, JobResult::Failed { err: JobError::Fs(e), .. }
                if matches!(e.kind, FsErrorKind::InPlaceFailed { .. })),
            "{r:?}"
        );
    }

    #[test]
    fn 何も書いていない失敗は_取消しが届いていれば取消しとして返す() {
        let r = save_cancelled_mid(FsErrorKind::Conflict { current: None });
        assert!(
            matches!(
                r,
                JobResult::Failed {
                    err: JobError::Cancelled,
                    ..
                }
            ),
            "{r:?}"
        );
    }
}
