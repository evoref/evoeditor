//! `FileSystem` Port とその型 (docs/02-architecture.md §4.5 が **唯一の正本**)。eve-core が定義し、本番は eve-platform、
//! テストは `MemFs` (eve-testkit、障害注入つき) が実装する。**全メソッドはジョブの中からだけ呼ぶ** (ADR-0020: メインスレッドで I/O をしない)。
//!
//! 「trait は最初の利用者と同時に定義する」(ADR-0028) ので、今は M1 (開く・保存を `MemFs` で) に要る操作
//! (`metadata` / `canonicalize` / `open_read` / `write_atomic`) だけを置く。直接上書き (`write_in_place`)・退避ディレクトリ・
//! 保存中フラグ (`inflight_*`) は、M3 (G1c) で保存・退避を本実装するときに足す。

use std::fmt;
use std::io;

use crate::fspath::FsPath;

// ---------- trait ----------

/// ファイルシステムの Port。`Send + Sync` を要求する (ジョブのワーカーが呼ぶため)。
pub trait FileSystem: Send + Sync {
    /// `stat` 相当。シンボリックリンクは辿る (リンク自体の情報は `FileMeta::link`)。
    /// パスが無いとき、親フォルダも無ければ `NotFound` + `ParentMissing { dir }`、親があれば `NotFound` (detail なし)。
    fn metadata(&self, path: &FsPath) -> Result<FileMeta, FsError>;

    fn canonicalize(&self, path: &FsPath) -> Result<FsPath, FsError>;

    /// チャンク読込 (既定 1 MiB)。キャンセルは、ジョブが `next_chunk` の間で `CancelToken` を見る。
    /// `NotFound` の区別は `metadata` と同じ。
    fn open_read(&self, path: &FsPath) -> Result<Box<dyn ReadChunks + Send>, FsError>;

    /// 1 パス分の保存中フラグ (`recovery/inflight/<hash(path)>/<pid>-<seq>.toml`、05 §7.5) を読む。開くジョブが開くたびに 1 回呼ぶ
    /// (存在するファイルでも `NotFound` でも)。キーは開くパスと、シンボリックリンクならリンク先の 2 つまで。ディレクトリを列挙しないので O(1)。
    /// フラグのディレクトリが無ければ空の Vec。状態ディレクトリを持たない実装は常に空を返す。
    fn inflight_for(&self, _path: &FsPath) -> Result<Vec<InflightRecord>, FsError> {
        Ok(Vec::new())
    }

    /// 起動時の走査 (v0.2、03 §10.3 手順 1b、05 §4.6): `recovery/inflight/*/*.toml` の保存中フラグをすべて読む。
    /// 書いたプロセスが今も動いているもの (pid と起動時刻が一致。自分のプロセスを含む) は除く (保存中のフラグを孤児にしない)。
    /// 解析できないフラグは `malformed = true` で返す (その `target` は空)。ディレクトリが無ければ空。
    /// 状態ディレクトリを持たない実装は常に空を返す。
    fn list_inflight(&self) -> Result<Vec<InflightRecord>, FsError> {
        Ok(Vec::new())
    }

    /// `from` を `to` の名前に変える。**`to` が既に在れば失敗する** (置き換えない。M6e の「退避名を元の名前に戻す」、05 §4.6)。
    /// 同じボリューム内の移動だけ。実装していない Port は `Unsupported`。
    fn rename_no_replace(&self, _from: &FsPath, _to: &FsPath) -> Result<(), FsError> {
        Err(FsError::new(FsErrorKind::Unsupported))
    }

    /// 起動時の走査 (v0.2、05 §4.6): `state_dir/backup/` の直下にある、in-place の事前バックアップ (`*.bak` / `*.bak.tmp`、05 §7.5)
    /// を列挙する。名前の pid のプロセスが動いているもの、`fr119/` などのサブディレクトリ、形に合わない名前は含めない。
    /// どのフラグが記録したものかは見ない (照合は core)。場所が無ければ空。状態ディレクトリを持たない実装は常に空。
    fn list_orphan_backups(&self) -> Result<Vec<OrphanBackup>, FsError> {
        Ok(Vec::new())
    }

    /// 起動時の退避の案内 (`msg.recovery.found` / `found_rescue`) 用に、退避ディレクトリを列挙する (05 §7.5 の「`recovery/` の走査と
    /// 削除の規則」)。`state_dir/recovery/` 直下と `recovery/rescue/` 直下、代替の退避先の同じ位置にある、名前の形
    /// (`<UTC timestamp>-<pid>[-<n>]`) に合うディレクトリだけを返す (`inflight/` は見ない。形に合わない名前は無視。中のファイルは読まない)。
    /// 場所が無ければ空。状態ディレクトリを持たない実装は常に空。
    fn list_recovery_dirs(&self) -> Result<Vec<RecoveryDirEntry>, FsError> {
        Ok(Vec::new())
    }

    /// 「退避先に保存」(03 §9.7 (e))。`doc` を `state_dir/recovery/rescue/<UTC timestamp>-<pid>[-<n>]/` に緊急退避と同じ形式
    /// (`<n>-<basename>.txt` + `manifest.toml`、`reason = "user_save"`) で書き、書いたディレクトリを返す。`state_dir` を使えなければ
    /// 代替の退避先に書く。元のファイルには触れない。書けなければ `FsError`。
    fn write_recovery(&self, _doc: &crate::DocSnapshot) -> Result<FsPath, FsError> {
        Err(FsError::new(FsErrorKind::Other))
    }

    /// スワップ (swap-lite、03 §10.2 / FR-118) を書く。`doc` の全文を緊急退避と同じ形式で、`state_dir/swap/` の `doc` に対応する
    /// ファイルへ原子的に置き換える (配置・命名は 05 §7.5)。`doc.path` が無ければ無題の文書。実装していない Port は
    /// `FsErrorKind::Unsupported` を返し、core はそのセッションのスワップを止める。
    fn write_swap(&self, _doc: &crate::DocSnapshot) -> Result<(), FsError> {
        Err(FsError::new(FsErrorKind::Unsupported))
    }

    /// `doc` のスワップを消す (保存の成功・文書を閉じた・正常終了)。無ければ何もしない (成功)。`path` は文書のパス (無題なら `None`)。
    fn discard_swap(
        &self,
        _doc: eve_view::DocumentId,
        _path: Option<&FsPath>,
    ) -> Result<(), FsError> {
        Err(FsError::new(FsErrorKind::Unsupported))
    }

    /// 他のプロセスが残したスワップを列挙する (復旧の探索、03 §10.3 手順 1)。`state_dir/swap/*.swp` の manifest だけを読む。
    /// 自分のプロセスのものは含めない。`swap/` が無ければ空。状態ディレクトリを持たない実装は常に空。
    fn list_swaps(&self) -> Result<Vec<SwapEntry>, FsError> {
        Ok(Vec::new())
    }

    /// スワップの本文と形式を読む。`entry.readable` でないものは呼ばない。
    fn read_swap(&self, _entry: &SwapEntry) -> Result<SwapBody, FsError> {
        Err(FsError::new(FsErrorKind::Unsupported))
    }

    /// 他のプロセスが残したスワップを消す (復旧の適用の後・破棄の選択)。`swap/` の下の `.swp` 以外は消さない。無ければ成功。
    fn remove_swap(&self, _entry: &SwapEntry) -> Result<(), FsError> {
        Err(FsError::new(FsErrorKind::Unsupported))
    }

    /// ファイルを 1 つ消す (保存中フラグの掃除など)。存在しなければ `NotFound`。
    fn remove_file(&self, _path: &FsPath) -> Result<(), FsError> {
        Err(FsError::new(FsErrorKind::Other))
    }

    /// 直接上書きで保存する (ADR-0016、05 §4.4)。呼ぶ前に core が同じ Snapshot の **乾式走査** (エンコード + 照合。書込なし) を終え、
    /// 表現不能が無いことと出力のバイト長 `len` を確定していること。`data` は乾式走査と同じエンコードをもう一度流す。
    /// 取消し (`cancel`) は上書きの開始より前の段 (照合・開く・事前バックアップの書込のチャンクごと) でだけ見て、立っていれば
    /// 一時名のバックアップとフラグを消して `SourceFailed` を返す。上書きの開始以降は見ない (`data` も `Aborted` を返さない)。
    fn write_in_place(
        &self,
        _path: &FsPath,
        _data: &mut dyn ReadChunks,
        _len: u64,
        _opts: &SaveOptions,
        _cancel: &crate::cancel::CancelToken,
    ) -> Result<SaveReport, FsError> {
        Err(FsError::new(FsErrorKind::Other))
    }

    /// 原子的に保存する (一時ファイル + 置換、または新規作成)。`data` の失敗 (`ChunkError`) で書込を止め、
    /// 一時ファイルを消して `SourceFailed` を返す。
    fn write_atomic(
        &self,
        path: &FsPath,
        data: &mut dyn ReadChunks,
        opts: &SaveOptions,
    ) -> Result<SaveReport, FsError>;
}

/// チャンク供給。読込では Port が、保存では core のエンコーダが供給する。
pub trait ReadChunks {
    fn next_chunk(&mut self) -> Result<Option<&[u8]>, ChunkError>;
}

/// `ReadChunks` の失敗。
#[derive(Debug)]
#[non_exhaustive]
pub enum ChunkError {
    /// 読込側: 読込の途中の I/O 失敗。
    Io(FsError),
    /// 保存側: エンコード・照合で表現不能 (黙示置換を含む) を検出したので書込を打ち切る。
    Unmappable,
    /// 保存側: 置換の前の段で `CancelToken` により中断した。
    Aborted,
    /// 保存側: エンコーダのバッファの `try_reserve` が失敗した。
    OutOfMemory { requested: u64 },
}

// ---------- メタ情報 ----------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    File,
    Dir,
    /// デバイス・FIFO・ソケットなど。
    Other,
}

/// ファイルの更新時刻 (UNIX 紀元からの経過)。OS の値をそのまま写し、**等しいかどうかの比較にだけ**使う
/// (`Clock` とは無関係。表示もしない)。FS が持たない精度は 0。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FileTime {
    pub secs: i64,
    pub nanos: u32,
}

/// ファイルの識別 (Windows はボリューム番号とファイルインデックス、Unix は dev と ino)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileId {
    pub volume: u64,
    pub index: u128,
}

/// 内容ハッシュ (XXH3-64)。16 MiB 以下のファイルだけ、読込ジョブ・保存ジョブの中で計算する。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ContentHash(pub [u8; 8]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FsKind {
    Local,
    Network,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LinkKind {
    Symlink,
    Junction,
}

/// リンクの情報 (ダングリングを含む)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkInfo {
    pub kind: LinkKind,
    pub target: Option<FsPath>,
    pub dangling: bool,
}

/// `metadata` の結果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileMeta {
    pub kind: FileKind,
    pub len: u64,
    pub mtime: FileTime,
    pub readonly: bool,
    pub file_id: Option<FileId>,
    pub nlink: u32,
    pub link: Option<LinkInfo>,
    pub fs_kind: FsKind,
    /// 他ユーザー所有の事前判定。判定できない OS・FS は `None`。
    pub owner_is_me: Option<bool>,
}

/// 外部変更の比較キー (計算規則は 05 §5.5)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStamp {
    pub mtime: FileTime,
    pub len: u64,
    pub file_id: Option<FileId>,
    pub hash: Option<ContentHash>,
}

impl FileStamp {
    /// メタ情報から作る (ハッシュは読込・保存ジョブの中で別に計算して付ける)。
    pub fn from_meta(meta: &FileMeta, hash: Option<ContentHash>) -> FileStamp {
        FileStamp {
            mtime: meta.mtime,
            len: meta.len,
            file_id: meta.file_id,
            hash,
        }
    }
}

/// 退避ディレクトリ 1 つ (起動時の案内用。中身は読まない)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryDirEntry {
    pub path: FsPath,
    /// 「退避先に保存」の退避 (`recovery/rescue/` の下)。異常終了の退避ではない。
    pub rescue: bool,
}

// ---------- スワップの候補 (復旧の材料、03 §10.3、05 §7.5) ----------

/// スワップの書き手 (プロセス) の状態。pid の生存確認 (05 §5.3: `OpenProcess` + 起動時刻の一致) で決める。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwapOwner {
    /// 書いたプロセスがまだ動いている。他のインスタンスが同じ文書を開いているということ。
    Alive,
    /// 書いたプロセスはもういない (異常終了の残骸)。復旧の対象。
    Stale,
}

/// `swap/` の下の 1 つのスワップ (manifest だけを読んだ要約)。自分のプロセスのものは含まない。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwapEntry {
    /// スワップのディレクトリ (`swap/<名前>.swp`)。
    pub dir: FsPath,
    /// 元のファイルのパス。無題の文書は `None`。
    pub path: Option<FsPath>,
    /// 無題の文書の表示名。
    pub untitled_name: Option<String>,
    /// 書いた時点の文書の Revision。
    pub revision: u64,
    /// 本文のバイト数 (manifest の `len`)。
    pub len: u64,
    /// 書いたプロセスの pid。
    pub pid: u32,
    pub owner: SwapOwner,
    /// 秘密ファイルパターンに一致した文書 (復旧の案内で削除を促す)。
    pub sensitive: bool,
    /// スワップを最後に書いた時刻 (manifest の更新時刻)。元のファイルがその後に変わったかの判定に使う (比べるだけ)。
    pub modified: Option<FileTime>,
    /// 本文を読める。未知の `format_version`・壊れた manifest・書きかけ (`written = false`) は `false`
    /// (その場合 core は適用を提案せず、場所だけを示す。移動も削除もしない、05 §7.5)。
    pub readable: bool,
}

/// スワップの本文と、元の形式 (無題の文書に適用するとき、開いている文書に写す)。本文は内部の UTF-8 のまま (PUA エスケープ保持)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwapBody {
    pub text: String,
    pub encoding: crate::Encoding,
    pub has_bom: bool,
    pub default_eol: eve_text::LineEnding,
}

// ---------- 保存中フラグと中断の痕跡 (02 §4.5、05 §4.6 / §7.5) ----------

/// 保存中フラグ 1 つ分。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InflightRecord {
    /// 保存先のパス (フラグのキー)。
    pub target: FsPath,
    /// 書いたプロセス。
    pub pid: u32,
    /// 書いたプロセスの起動時刻 (OS のプロセス作成時刻。v0.1 は書くだけで、写像には使わない)。
    pub started_at: Option<FileTime>,
    pub method: SaveMethod,
    /// 一時ファイル (`.<name>.eve-tmp-*`)。
    pub tmp: Option<FsPath>,
    /// 置換の退避名 (`.<name>.eve-bak-*`)、または直接上書きの事前バックアップの最終名。
    pub backup: Option<FsPath>,
    /// 直接上書きで事前バックアップを作る記録か (`Required` なら true、`SkipConfirmed` なら false)。
    pub backup_recorded: bool,
    /// 直接上書き: 事前バックアップを最終名に変えた後に true。原子的保存では常に false。
    pub backup_complete: bool,
    /// フラグのファイル自身 (後始末で消す)。
    pub flag: FsPath,
    /// フラグを解析できなかった (未知の版・必須キーの欠落・値の不正)。true のとき意味を持つのは `target`・`pid`・`flag` と読めた `tmp` だけ。
    pub malformed: bool,
}

/// `state_dir/backup/` の直下に残る、事前バックアップのファイル (`list_orphan_backups` の要素)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrphanBackup {
    pub path: FsPath,
    /// ファイル名の末尾の pid (書いたプロセス。動いていないものだけが列挙される)。
    pub pid: u32,
    /// `.bak.tmp` (書込中に中断した、完全でない写し) なら true。`.bak` は完全な写し。
    pub partial: bool,
    pub len: u64,
}

/// 開くジョブが見つけた中断された保存の痕跡 (`JobResult::Complete` / `NewFile` の `interrupted_save`)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterruptedSave {
    pub method: SaveMethod,
    /// 実在するバックアップのパス。`complete` なら完全なもの (直接上書きは最終名、原子的保存は退避名)。
    /// `complete = false` なら最終名ではなく実在する一時名 (無ければ `None`)。
    pub backup: Option<FsPath>,
    /// `InflightRecord.tmp` のうち実在するもの。
    pub tmp: Option<FsPath>,
    pub complete: bool,
}

// ---------- 保存 ----------

/// 保存前に期待する保存先の状態。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expect {
    /// 新規作成: 存在しないことを期待する。
    Absent,
    /// 書き始める前と置換の直前の 2 回照合する。
    Stamp(FileStamp),
    /// 照合しない。evoeditor 自身だけが書く状態ファイルのための値で、利用者のファイルの保存では使わない。
    Any,
}

/// `fsync` の水準。既定は `Full` (P1 原則。07 §5 の `file.fsync`、OQ-77)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FsyncLevel {
    None,
    FileOnly,
    #[default]
    Full,
}

/// `write_in_place` の事前バックアップの扱い (直接上書きは M3 で足す。型だけ先に置く)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InPlaceBackup {
    #[default]
    Required,
    SkipConfirmed,
}

/// `nlink > 1` のときの方針。既定は `Ask` (OQ-73)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HardlinkPolicy {
    #[default]
    Ask,
    BreakLink,
    WriteInPlace,
}

/// 置換で保つ属性。既定はすべて `true`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreserveAttrs {
    pub file_attributes: bool,
    pub security: bool,
    pub streams_xattr: bool,
}

impl Default for PreserveAttrs {
    fn default() -> PreserveAttrs {
        PreserveAttrs {
            file_attributes: true,
            security: true,
            streams_xattr: true,
        }
    }
}

/// 予約 (v0.3、FR-119)。v0.1 / v0.2 は常に `None`。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum BackupPolicy {
    #[default]
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SaveOptions {
    /// 書き始める前と置換の直前の 2 回照合する。不一致なら `FsErrorKind::Conflict`。
    pub expected: Expect,
    pub in_place_backup: InPlaceBackup,
    pub fsync: FsyncLevel,
    /// 新しく作るファイルを所有者のみで作る。意味はこれだけ。
    pub private: bool,
    /// evoeditor 自身の状態ファイルの書込で、保存中フラグを書かない。利用者のファイルの保存では常に `false`。
    pub state_file: bool,
    pub preserve: PreserveAttrs,
    pub hardlink: HardlinkPolicy,
    pub backup: BackupPolicy,
    /// 「読み取り専用を解除して保存」(FR-116 / FR-106): 保存先の読み取り専用属性を置換の直前に外し、置換の後に付け直す (05 §4.2 の手順 5b)。
    pub clear_readonly: bool,
}

impl SaveOptions {
    /// 利用者のファイルの保存の既定 (`expected` だけを指定する)。
    pub fn user_file(expected: Expect) -> SaveOptions {
        SaveOptions {
            expected,
            in_place_backup: InPlaceBackup::default(),
            fsync: FsyncLevel::default(),
            private: false,
            state_file: false,
            preserve: PreserveAttrs::default(),
            hardlink: HardlinkPolicy::default(),
            backup: BackupPolicy::default(),
            clear_readonly: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveMethod {
    /// 一時ファイル + 置換。
    Atomic,
    /// 直接上書き。
    InPlace,
    /// `Expect::Absent` で新規に作成 (置換なし)。
    NewFile,
}

/// 保存は成功したが知らせること。
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SaveWarning {
    /// 保存中フラグを書けなかった (警告して保存は続けた)。
    InflightFlagNotWritten,
    /// 求めた属性の一部を保てなかった。
    AttrsNotPreserved { what: Vec<AttrKind> },
    /// 置換で他の利用者からの権限が元より狭くなった。
    PermissionNarrowed,
    /// `fsync` に失敗した (ネットワーク FS・リムーバブル)。保存は続けた (05 §4.1 手順 3)。
    FsyncFailed,
    /// 置換の直後に他のプログラムが保存先を変更した可能性がある (FileId が一致しない)。次の保存は照合で Conflict になる (05 §4.1 手順 7)。
    ChangedAfterSave,
    /// 退避名 (`.<name>.eve-bak-*`) を消せなかった。保存先は新しい内容で完全 (05 §4.1 手順 7)。
    BackupNotRemoved { path: FsPath },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AttrKind {
    FileAttributes,
    Security,
    StreamsXattr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveReport {
    pub method: SaveMethod,
    pub new_stamp: FileStamp,
    pub warnings: Vec<SaveWarning>,
}

// ---------- エラー ----------

/// `FileSystem` のエラー。キャンセルの変種は持たない (キャンセルは `CancelToken` と `JobError::Cancelled`)。
/// OS のエラーは `source` に入れる (ログ用。本文を含まない)。
#[derive(Debug)]
pub struct FsError {
    pub kind: FsErrorKind,
    pub detail: Option<FsErrorDetail>,
    pub source: Option<io::Error>,
}

impl FsError {
    pub fn new(kind: FsErrorKind) -> FsError {
        FsError {
            kind,
            detail: None,
            source: None,
        }
    }

    pub fn with_detail(kind: FsErrorKind, detail: FsErrorDetail) -> FsError {
        FsError {
            kind,
            detail: Some(detail),
            source: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FsErrorKind {
    // ── 開く・調べる・一般 ──
    NotFound,
    PermissionDenied,
    /// 共有違反・ロック。
    Busy,
    /// 構文・予約名・循環。
    InvalidPath,
    DiskFull,
    ReadOnlyFs,
    Other,
    /// この Port はその操作を実装していない (スワップの `write_swap` など)。呼び側は以後その機能を使わない。
    Unsupported,
    // ── 保存 ──
    /// `SaveOptions::expected` と不一致。何も書いていない。`current` は照合の時点の保存先の Stamp で、
    /// `None` は保存先が存在しない (開いた後に削除された)。
    Conflict {
        current: Option<FileStamp>,
    },
    /// 原子的置換ができない・意味が変わる。何も書いていない。
    AtomicUnsupported,
    /// `ReadChunks` の供給側の失敗。元ファイルは無傷。
    SourceFailed,
    /// Windows の置換途中失敗: 元ファイルは退避名に完全に残る。
    ReplacedMovedToBackup {
        backup: FsPath,
    },
    /// `write_in_place` の事前バックアップ、または保存中フラグの書込に失敗した (後者は `detail = InflightFlag`)。何も書いていない
    /// (元ファイルは無傷)。core は「バックアップ無しで上書きしますか」を確認し、承認されたら `SkipConfirmed` で再投入する。
    BackupFailed,
    /// `write_in_place` の書込途中の失敗。元ファイルは壊れている可能性がある。`Some` なら事前バックアップは完全で、
    /// core はそのパスを必ず示す。`None` は「バックアップなしで上書き」を承認していた (元の内容は取り戻せない)。
    InPlaceFailed {
        backup: Option<FsPath>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FsErrorDetail {
    /// Windows の予約名 (CON・NUL など)。`kind` は `InvalidPath`。
    ReservedName,
    /// リンク先が存在しない。`kind` は `NotFound`。
    DanglingLink { target: FsPath },
    /// 親フォルダが無い。`kind` は `NotFound`。
    ParentMissing { dir: FsPath },
    /// ハードリンクの `Ask` による `AtomicUnsupported`。
    Hardlink { nlink: u32 },
    /// デバイス・FIFO・ソケット。`kind` は `InvalidPath`。
    NotAFile,
    /// ネットワーク上のファイルに到達できない。`kind` は `Other`。
    Network,
    /// 保存中フラグを書けなかった。`kind` は `BackupFailed` (`write_in_place` のみ。`write_atomic` は警告で続ける)。
    InflightFlag,
}

impl fmt::Display for FsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ファイルシステムのエラー: {:?}", self.kind)?;
        if let Some(d) = &self.detail {
            write!(f, " ({d:?})")?;
        }
        Ok(())
    }
}

impl std::error::Error for FsError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 保存の既定の方針() {
        let o = SaveOptions::user_file(Expect::Absent);
        assert_eq!(o.fsync, FsyncLevel::Full, "既定は Full (OQ-77)");
        assert_eq!(o.hardlink, HardlinkPolicy::Ask, "既定は Ask (OQ-73)");
        assert!(!o.private && !o.state_file, "利用者のファイルの保存");
        assert!(o.preserve.file_attributes && o.preserve.security && o.preserve.streams_xattr);
    }

    #[test]
    fn スタンプはメタ情報から作る() {
        let meta = FileMeta {
            kind: FileKind::File,
            len: 10,
            mtime: FileTime { secs: 1, nanos: 2 },
            readonly: false,
            file_id: None,
            nlink: 1,
            link: None,
            fs_kind: FsKind::Local,
            owner_is_me: Some(true),
        };
        let s = FileStamp::from_meta(&meta, None);
        assert_eq!((s.len, s.mtime), (10, FileTime { secs: 1, nanos: 2 }));
        assert_eq!(s, FileStamp::from_meta(&meta, None));
        assert_ne!(s, FileStamp::from_meta(&meta, Some(ContentHash([1; 8]))));
    }

    #[test]
    fn エラーの組み立て() {
        let e = FsError::with_detail(
            FsErrorKind::NotFound,
            FsErrorDetail::ParentMissing {
                dir: FsPath::from("/secretdir/x"),
            },
        );
        assert_eq!(e.kind, FsErrorKind::NotFound);
        assert!(matches!(
            e.detail,
            Some(FsErrorDetail::ParentMissing { .. })
        ));
        assert!(e.to_string().contains("NotFound"));
        assert!(
            !e.to_string().contains("secretdir"),
            "エラーの表示にパス全体を出さない"
        );
    }
}
