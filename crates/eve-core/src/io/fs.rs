//! `FileSystem` Port とその型 (docs/02-architecture.md §4.5 が **唯一の正本**)。eve-core が定義し、本番は eve-platform、
//! テストは `MemFs` (eve-testkit、障害注入つき) が実装する。**全メソッドはジョブの中からだけ呼ぶ** (ADR-0020: メインスレッドで I/O をしない)。
//!
//! 「trait は最初の利用者と同時に定義する」(ADR-0028) ので、使う操作だけを置く。読み取り (`metadata` / `canonicalize` / `open_read`)、
//! 原子的保存 (`write_atomic`)、直接上書き (`write_in_place`)、保存中フラグ (`inflight_*`)、スワップ、退避名の操作などがある。

use std::fmt;
use std::io;

use crate::io::fspath::FsPath;

// ---------- trait ----------

/// ファイルシステムの Port。`Send + Sync` を要求する (ジョブのワーカーが呼ぶため)。
pub trait FileSystem: Send + Sync {
    /// `stat` 相当。シンボリックリンクは辿る (リンク自体の情報は `FileMeta::link`)。
    ///
    /// # Errors
    ///
    /// パスが無いとき、親フォルダも無ければ `NotFound` + `ParentMissing { dir }`、親があれば `NotFound` (detail なし)。
    /// ほかに権限・パスの不正・ネットワークなどの失敗を `FsErrorKind` で返す。
    fn metadata(&self, path: &FsPath) -> Result<FileMeta, FsError>;

    /// シンボリックリンクと `.` / `..` を解決した絶対パスを返す (同じファイルの判定・表示に使う)。
    ///
    /// # Errors
    ///
    /// パスが無い・辿れない (`NotFound` など)。区別は `metadata` と同じ。
    fn canonicalize(&self, path: &FsPath) -> Result<FsPath, FsError>;

    /// チャンク読込 (既定 1 MiB)。キャンセルは、ジョブが `next_chunk` の間で `CancelToken` を見る。
    ///
    /// # Errors
    ///
    /// 開けない (`NotFound` / `PermissionDenied` / `Busy` など)。`NotFound` の区別は `metadata` と同じ。
    fn open_read(&self, path: &FsPath) -> Result<Box<dyn ReadChunks + Send>, FsError>;

    /// 1 パス分の保存中フラグ (`recovery/inflight/<hash(path)>/<pid>-<seq>.toml`、05 §7.5) を読む。開くジョブが開くたびに 1 回呼ぶ
    /// (存在するファイルでも `NotFound` でも)。キーは開くパスと、シンボリックリンクならリンク先の 2 つまで。ディレクトリを列挙しないので O(1)。
    /// フラグのディレクトリが無ければ空の Vec。状態ディレクトリを持たない実装は常に空を返す。
    ///
    /// # Errors
    ///
    /// フラグのディレクトリを読めない (権限など)。
    fn inflight_for(&self, _path: &FsPath) -> Result<Vec<InflightRecord>, FsError> {
        Ok(Vec::new())
    }

    /// 起動時の走査 (v0.2、03 §10.3 手順 1b、05 §4.6): `recovery/inflight/*/*.toml` の保存中フラグをすべて読む。
    /// 書いたプロセスが今も動いているもの (pid と起動時刻が一致。自分のプロセスを含む) は除く (保存中のフラグを孤児にしない)。
    /// 解析できないフラグは `malformed = true` で返す (その `target` は空)。ディレクトリが無ければ空。
    /// 状態ディレクトリを持たない実装は常に空を返す。
    ///
    /// # Errors
    ///
    /// フラグのディレクトリを列挙できない (権限など)。
    fn list_inflight(&self) -> Result<Vec<InflightRecord>, FsError> {
        Ok(Vec::new())
    }

    /// `from` を `to` の名前に変える。**`to` が既に在れば失敗する** (置き換えない。「退避名を元の名前に戻す」、05 §4.6)。
    /// 同じボリューム内の移動だけ。実装していない Port は `Unsupported`。
    /// Unix の退路 (`link` + `unlink`) で link の後に `from` を消せなかったときは `Err` を返す。`to` は作られ、`from` も残る
    /// (呼び側は `to` の有無で確かめる。docs/review/v0.3-findings.md の SAF-40)。
    ///
    /// # Errors
    ///
    /// `to` が既に在る、`from` が無い、別のボリューム、権限などの失敗。実装していない Port は `Unsupported`。
    fn rename_no_replace(&self, _from: &FsPath, _to: &FsPath) -> Result<(), FsError> {
        Err(FsError::new(FsErrorKind::Unsupported))
    }

    /// 起動時の走査 (v0.2、05 §4.6): `state_dir/backup/` の直下にある、in-place の事前バックアップ (`*.bak` / `*.bak.tmp`、05 §7.5)
    /// を列挙する。名前の pid のプロセスが動いているもの、`fr119/` などのサブディレクトリ、形に合わない名前は含めない。
    /// どのフラグが記録したものかは見ない (照合は core)。場所が無ければ空。状態ディレクトリを持たない実装は常に空。
    ///
    /// # Errors
    ///
    /// `state_dir/backup/` を列挙できない (権限など)。
    fn list_orphan_backups(&self) -> Result<Vec<OrphanBackup>, FsError> {
        Ok(Vec::new())
    }

    /// 起動時の退避の案内 (`msg.recovery.found` / `found_rescue`) 用に、退避ディレクトリを列挙する (05 §7.5 の「`recovery/` の走査と
    /// 削除の規則」)。`state_dir/recovery/` 直下と `recovery/rescue/` 直下、代替の退避先の同じ位置にある、名前の形
    /// (`<UTC timestamp>-<pid>[-<n>]`) に合うディレクトリだけを返す (`inflight/` は見ない。形に合わない名前は無視。中のファイルは読まない)。
    /// 場所が無ければ空。状態ディレクトリを持たない実装は常に空。
    ///
    /// # Errors
    ///
    /// 退避の場所を列挙できない (権限など)。
    fn list_recovery_dirs(&self) -> Result<Vec<RecoveryDirEntry>, FsError> {
        Ok(Vec::new())
    }

    /// 「退避先に保存」(03 §9.7 (e))。`doc` を `state_dir/recovery/rescue/<UTC timestamp>-<pid>[-<n>]/` に緊急退避と同じ形式
    /// (`<n>-<basename>.txt` + `manifest.toml`、`reason = "user_save"`) で書き、書いたディレクトリを返す。`state_dir` を使えなければ
    /// 代替の退避先に書く。元のファイルには触れない。
    ///
    /// # Errors
    ///
    /// `state_dir` にも代替の退避先にも書けない (容量・権限など)。既定の実装は `Other`。
    fn write_recovery(&self, _doc: &crate::DocSnapshot) -> Result<FsPath, FsError> {
        Err(FsError::new(FsErrorKind::Other))
    }

    /// スワップ (swap-lite、03 §10.2 / FR-118) を書く。`doc` の全文を緊急退避と同じ形式で、`state_dir/swap/` の `doc` に対応する
    /// ファイルへ原子的に置き換える (配置・命名は 05 §7.5)。`doc.path` が無ければ無題の文書。実装していない Port は
    /// `FsErrorKind::Unsupported` を返し、core はそのセッションのスワップを止める。
    ///
    /// # Errors
    ///
    /// 書けない (容量・権限など)。実装していない Port は `Unsupported`。
    fn write_swap(&self, _doc: &crate::DocSnapshot) -> Result<(), FsError> {
        Err(FsError::new(FsErrorKind::Unsupported))
    }

    /// `doc` のスワップを消す (保存の成功・文書を閉じた・正常終了)。無ければ何もしない (成功)。`path` は文書のパス (無題なら `None`)。
    ///
    /// # Errors
    ///
    /// 消せない (権限・共有違反など)。実装していない Port は `Unsupported`。
    fn discard_swap(
        &self,
        _doc: eve_view::DocumentId,
        _path: Option<&FsPath>,
    ) -> Result<(), FsError> {
        Err(FsError::new(FsErrorKind::Unsupported))
    }

    /// 他のプロセスが残したスワップを列挙する (復旧の探索、03 §10.3 手順 1)。`state_dir/swap/*.swp` の manifest だけを読む。
    /// 自分のプロセスのものは含めない。`swap/` が無ければ空。状態ディレクトリを持たない実装は常に空。
    ///
    /// # Errors
    ///
    /// `swap/` を列挙できない (権限など)。
    fn list_swaps(&self) -> Result<Vec<SwapEntry>, FsError> {
        Ok(Vec::new())
    }

    /// スワップの本文と形式を読む。`entry.readable` でないものは呼ばない。
    ///
    /// # Errors
    ///
    /// 読めない・本文が壊れている。実装していない Port は `Unsupported`。
    fn read_swap(&self, _entry: &SwapEntry) -> Result<SwapBody, FsError> {
        Err(FsError::new(FsErrorKind::Unsupported))
    }

    /// 他のプロセスが残したスワップを消す (復旧の適用の後・破棄の選択)。`swap/` の下の `.swp` 以外は消さない。無ければ成功。
    ///
    /// # Errors
    ///
    /// 消せない (権限・共有違反など)。実装していない Port は `Unsupported`。
    fn remove_swap(&self, _entry: &SwapEntry) -> Result<(), FsError> {
        Err(FsError::new(FsErrorKind::Unsupported))
    }

    /// `state_dir` の直下の小さな状態ファイル (`last_version.toml` など、05 §7.1) を読む。
    ///
    /// # Errors
    ///
    /// 無ければ `NotFound`、`state_dir` を使えなければ `Unsupported`。ほかに読込の失敗。
    fn read_state_file(&self, _name: &str) -> Result<Vec<u8>, FsError> {
        Err(FsError::new(FsErrorKind::Unsupported))
    }

    /// `state_dir` の直下に小さな状態ファイルを、所有者のみ・原子的な置換で書く (NFR-505、05 §7.5 (b)。最終書込勝ち)。
    /// `state_dir` が無ければ所有者のみで作る (起動経路では呼ばない)。
    ///
    /// # Errors
    ///
    /// 書けない (容量・権限など)。`state_dir` を使えなければ `Unsupported`。
    fn write_state_file(&self, _name: &str, _data: &[u8]) -> Result<(), FsError> {
        Err(FsError::new(FsErrorKind::Unsupported))
    }

    /// 未知の書式の版の状態ファイルを `<name>.unsupported-<ts>` へ移して退避する (05 §7.5 (a)。消さない)。
    ///
    /// # Errors
    ///
    /// 移せない (権限など)。`state_dir` を使えなければ `Unsupported`。
    fn set_aside_state_file(&self, _name: &str) -> Result<(), FsError> {
        Err(FsError::new(FsErrorKind::Unsupported))
    }

    /// ファイルを 1 つ消す (保存中フラグの掃除など)。
    ///
    /// # Errors
    ///
    /// 存在しなければ `NotFound`。ほかに権限・共有違反などの失敗。既定の実装は `Other`。
    fn remove_file(&self, _path: &FsPath) -> Result<(), FsError> {
        Err(FsError::new(FsErrorKind::Other))
    }

    /// 直接上書きで保存する (ADR-0016、05 §4.4)。呼ぶ前に core が同じ Snapshot の **乾式走査** (エンコード + 照合。書込なし) を終え、
    /// 表現不能が無いことと出力のバイト長 `len` を確定していること。`data` は乾式走査と同じエンコードをもう一度流す。
    /// 取消し (`cancel`) は上書きの開始より前の段 (照合・開く・事前バックアップの書込のチャンクごと) でだけ見て、立っていれば
    /// 一時名のバックアップとフラグを消して `SourceFailed` を返す。上書きの開始以降は見ない (`data` も `Aborted` を返さない)。
    ///
    /// # Errors
    ///
    /// 照合の不一致は `Conflict`、事前バックアップか保存中フラグを書けなければ `BackupFailed` (何も書いていない)、
    /// 上書きの途中の失敗は `InPlaceFailed` (元ファイルは壊れている可能性がある)、取消し・供給の失敗は `SourceFailed`。
    fn write_in_place(
        &self,
        _path: &FsPath,
        _data: &mut dyn ReadChunks,
        _len: u64,
        _opts: &SaveOptions,
        _cancel: &crate::jobs::cancel::CancelToken,
    ) -> Result<SaveReport, FsError> {
        Err(FsError::new(FsErrorKind::Other))
    }

    /// 原子的に保存する (一時ファイル + 置換、または新規作成)。
    ///
    /// # Errors
    ///
    /// `data` の失敗 (`ChunkError`) で書込を止め、一時ファイルを消して `SourceFailed` を返す。照合の不一致は `Conflict`、
    /// 原子的置換ができなければ `AtomicUnsupported` (何も書いていない)、置換の途中の失敗は `ReplacedMovedToBackup` など。
    fn write_atomic(
        &self,
        path: &FsPath,
        data: &mut dyn ReadChunks,
        opts: &SaveOptions,
    ) -> Result<SaveReport, FsError>;
}

/// チャンク供給。読込では Port が、保存では core のエンコーダが供給する。
pub trait ReadChunks {
    /// 次のチャンクを返す。終わりなら `Ok(None)`。返したスライスは次の呼び出しまで有効。
    ///
    /// # Errors
    ///
    /// 読込側は I/O の失敗 (`Io`)、保存側は表現不能 (`Unmappable`)・中断 (`Aborted`)・確保の失敗 (`OutOfMemory`)。
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
    OutOfMemory {
        /// 確保しようとした量 (バイト)。
        requested: u64,
    },
}

// ---------- メタ情報 ----------

/// パスが指すものの種類 (リンクは辿った先)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    /// 通常のファイル。
    File,
    /// ディレクトリ。
    Dir,
    /// デバイス・FIFO・ソケットなど。
    Other,
}

/// ファイルの更新時刻 (UNIX 紀元からの経過)。OS の値をそのまま写し、**等しいかどうかの比較にだけ**使う
/// (`Clock` とは無関係。表示もしない)。FS が持たない精度は 0。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FileTime {
    /// UNIX 紀元からの秒。紀元より前は負。
    pub secs: i64,
    /// 秒未満のナノ秒。
    pub nanos: u32,
}

/// ファイルの識別 (Windows はボリューム番号とファイルインデックス、Unix は dev と ino)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileId {
    /// ボリューム番号 (Unix は `st_dev`)。
    pub volume: u64,
    /// ボリューム内のファイルインデックス (Unix は `st_ino`)。
    pub index: u128,
}

/// 内容ハッシュ (XXH3-64)。16 MiB 以下のファイルだけ、読込ジョブ・保存ジョブの中で計算する。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ContentHash(pub [u8; 8]);

/// ファイルを置く FS の種類 (保存の方式や警告の判断に使う)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FsKind {
    /// ローカルのディスク。
    Local,
    /// ネットワーク上の FS (SMB・NFS など)。
    Network,
    /// 判定できない。
    Unknown,
}

/// リンクの種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LinkKind {
    /// シンボリックリンク。
    Symlink,
    /// Windows のジャンクション。
    Junction,
}

/// リンクの情報 (ダングリングを含む)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkInfo {
    /// リンクの種類。
    pub kind: LinkKind,
    /// リンク先 (読めなければ `None`)。
    pub target: Option<FsPath>,
    /// リンク先が存在しない。
    pub dangling: bool,
}

/// `metadata` の結果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileMeta {
    /// 種類 (リンクは辿った先)。
    pub kind: FileKind,
    /// 大きさ (バイト)。
    pub len: u64,
    /// 更新時刻。
    pub mtime: FileTime,
    /// 読み取り専用 (属性または権限)。
    pub readonly: bool,
    /// ファイルの識別。取れない FS は `None`。
    pub file_id: Option<FileId>,
    /// ハードリンクの数。
    pub nlink: u32,
    /// パス自体がリンクなら、その情報。
    pub link: Option<LinkInfo>,
    /// ファイルを置く FS の種類。
    pub fs_kind: FsKind,
    /// 他ユーザー所有の事前判定。判定できない OS・FS は `None`。
    pub owner_is_me: Option<bool>,
}

/// 外部変更の比較キー (計算規則は 05 §5.5)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStamp {
    /// 更新時刻。
    pub mtime: FileTime,
    /// 大きさ (バイト)。
    pub len: u64,
    /// ファイルの識別 (取れない FS は `None`)。
    pub file_id: Option<FileId>,
    /// 内容ハッシュ (16 MiB 以下のファイルだけ。それ以外は `None`)。
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
    /// 退避ディレクトリのパス。
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
    /// 書いたプロセスが今も動いているか。
    pub owner: SwapOwner,
    /// 秘密ファイルパターンに一致した文書 (復旧の案内で削除を促す)。
    pub sensitive: bool,
    /// スワップを最後に書いた時刻 (manifest の更新時刻)。元のファイルがその後に変わったかの判定に使う (比べるだけ)。
    pub modified: Option<FileTime>,
    /// 本文を読める。未知の `format_version`・壊れた manifest・書きかけ (`written = false`) は `false`
    /// (その場合 core は適用を提案せず、場所だけを示す。移動も削除もしない、05 §7.5)。
    pub readable: bool,
    /// manifest の `encoding` (書いた時点の文書の文字コード)。読めなければ `None`。理由が `InvalidBytes` だけの読み取り
    /// 専用の文書には、これが文書の文字コードと同じときだけ復旧を提案する (03 §10.3、v0.4 の M23)。
    pub encoding: Option<crate::Encoding>,
}

/// スワップの本文と、元の形式 (無題の文書に適用するとき、開いている文書に写す)。本文は内部の UTF-8 のまま (PUA エスケープ保持)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwapBody {
    /// 本文 (内部の UTF-8)。
    pub text: String,
    /// 元の文書の文字コード。
    pub encoding: crate::Encoding,
    /// 元の文書が BOM を持っていたか。
    pub has_bom: bool,
    /// 元の文書の既定の改行コード。
    pub default_eol: eve_text::LineEnding,
    /// manifest の `pua_escaped` (02 §11.2.1): 本文のエスケープ文字が `encoding` のバイトか (書いた文書の
    /// `escape_encoding.is_some()`)。復旧の適用で `escape_encoding` を決めるのに使う (03 §8.3.1)。
    pub pua_escaped: bool,
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
    /// 保存の方式。
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
    /// バックアップのファイルのパス。
    pub path: FsPath,
    /// ファイル名の末尾の pid (書いたプロセス。動いていないものだけが列挙される)。
    pub pid: u32,
    /// `.bak.tmp` (書込中に中断した、完全でない写し) なら true。`.bak` は完全な写し。
    pub partial: bool,
    /// ファイルの大きさ (バイト)。
    pub len: u64,
}

/// 開くジョブが見つけた中断された保存の痕跡 (`JobResult::Complete` / `NewFile` の `interrupted_save`)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterruptedSave {
    /// 中断された保存の方式。
    pub method: SaveMethod,
    /// 実在するバックアップのパス。`complete` なら完全なもの (直接上書きは最終名、原子的保存は退避名)。
    /// `complete = false` なら最終名ではなく実在する一時名 (無ければ `None`)。
    pub backup: Option<FsPath>,
    /// `InflightRecord.tmp` のうち実在するもの。
    pub tmp: Option<FsPath>,
    /// `backup` が完全な写し (直接上書きは事前バックアップを最終名に変えた後)。
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
    /// `fsync` しない (設定で明示したときだけ。遅いネットワークドライブ用)。
    None,
    /// 一時ファイルと、置換したディレクトリ (Unix のみ) を `fsync` する (05 §4.1 手順 3・6)。
    ///
    /// 今の動作は `Full` と同じ。値を分けているのは OQ-77 (ネットワークの FS の既定) の検討のため。
    FileOnly,
    /// 一時ファイルと、置換したディレクトリ (Unix のみ。Windows はディレクトリを flush しない) を `fsync` する
    /// (05 §4.1 手順 3・6)。
    #[default]
    Full,
}

/// `write_in_place` の事前バックアップの扱い。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InPlaceBackup {
    /// 上書きの前に事前バックアップを作る。作れなければ `BackupFailed`。
    #[default]
    Required,
    /// 利用者が「バックアップなしで上書き」を承認した。事前バックアップを作らない。
    SkipConfirmed,
}

/// `nlink > 1` のときの方針。既定は `Ask` (OQ-73)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HardlinkPolicy {
    /// 書かずに `AtomicUnsupported` + `Hardlink` を返し、利用者に尋ねる。
    #[default]
    Ask,
    /// 置換してリンクを切る (保存先だけが新しい内容になる)。
    BreakLink,
    /// 直接上書きにしてリンクを保つ。
    WriteInPlace,
}

/// 置換で保つ属性。既定はすべて `true`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreserveAttrs {
    /// Windows の READONLY / HIDDEN / SYSTEM の属性。
    pub file_attributes: bool,
    /// Windows の ACL、Unix のモード・所有者。
    pub security: bool,
    /// Windows の ADS、Unix の xattr・POSIX ACL。
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

/// 予約 (v0.4、FR-119)。v0.1 / v0.2 / v0.3 は常に `None`。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum BackupPolicy {
    /// バックアップを作らない。
    #[default]
    None,
}

/// `write_atomic` / `write_in_place` に渡す保存の指定 (02 §4.5、05 §4)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SaveOptions {
    /// 書き始める前と置換の直前の 2 回照合する。不一致なら `FsErrorKind::Conflict`。
    pub expected: Expect,
    /// `write_in_place` の事前バックアップの扱い。
    pub in_place_backup: InPlaceBackup,
    /// `fsync` の水準。
    pub fsync: FsyncLevel,
    /// 新しく作るファイルを所有者のみで作る。意味はこれだけ。
    pub private: bool,
    /// evoeditor 自身の状態ファイルの書込で、保存中フラグを書かない。利用者のファイルの保存では常に `false`。
    pub state_file: bool,
    /// 置換で保つ属性。
    pub preserve: PreserveAttrs,
    /// `nlink > 1` のときの方針。
    pub hardlink: HardlinkPolicy,
    /// 予約 (FR-119)。
    pub backup: BackupPolicy,
    /// 「読み取り専用を解除して保存」(FR-116 / FR-106): 保存先の読み取り専用属性を置換の直前に外し、置換の後に付け直す (05 §4.2 の手順 5b)。
    pub clear_readonly: bool,
    /// 保存先がダングリングのシンボリックリンクなら、リンク先に新規作成する (リンクは保つ。FR-106、05 §4.1)。確認を経た保存だけが立てる。
    pub create_link_target: bool,
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
            create_link_target: false,
        }
    }
}

/// 実際に使った保存の方式。
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
    AttrsNotPreserved {
        /// 保てなかった属性。
        what: Vec<AttrKind>,
    },
    /// 置換で他の利用者からの権限が元より狭くなった。
    PermissionNarrowed,
    /// 一時ファイル・ディレクトリの `fsync` を FS が受け付けなかった (ネットワーク FS・リムーバブル)。保存は続けた (05 §4.1 手順 3 / 6)。
    /// 一時ファイルの `fsync` の EIO・ENOSPC 等 (Unix) は警告ではなくエラー (docs/review/v0.3-findings.md の SAF-46)。
    FsyncFailed,
    /// 置換の直後に他のプログラムが保存先を変更した可能性がある (`FileId` が一致しない)。次の保存は照合で Conflict になる (05 §4.1 手順 7)。
    ChangedAfterSave,
    /// 退避名 (`.<name>.eve-bak-*`) を消せなかった。保存先は新しい内容で完全 (05 §4.1 手順 7)。
    BackupNotRemoved {
        /// 残った退避名のパス。
        path: FsPath,
    },
    /// 一時ファイル (`.<name>.eve-tmp-*`) を消せなかった。保存先は新しい内容で完全で、一時ファイルは保存先と同じ実体
    /// (Unix の新規ファイルの `link` + `unlink` の退路。消すまでは次の保存がハードリンクの確認になる。05 §4.1「新規ファイル」、
    /// docs/review/v0.3-findings.md の SAF-40)。
    TempNotRemoved {
        /// 残った一時ファイルのパス。
        path: FsPath,
    },
}

/// 置換で保つ属性の種類 (`PreserveAttrs` の各項目に対応する)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AttrKind {
    /// `PreserveAttrs::file_attributes`。
    FileAttributes,
    /// `PreserveAttrs::security`。
    Security,
    /// `PreserveAttrs::streams_xattr`。
    StreamsXattr,
}

/// 保存の成功の報告。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveReport {
    /// 実際に使った保存の方式。
    pub method: SaveMethod,
    /// 書いた後の保存先の Stamp (次の保存の照合に使う)。
    pub new_stamp: FileStamp,
    /// 保存は成功したが知らせること。
    pub warnings: Vec<SaveWarning>,
}

// ---------- エラー ----------

/// `FileSystem` のエラー。キャンセルの変種は持たない (キャンセルは `CancelToken` と `JobError::Cancelled`)。
/// OS のエラーは `source` に入れる (ログ用。本文を含まない)。
#[derive(Debug)]
pub struct FsError {
    /// エラーの種類。
    pub kind: FsErrorKind,
    /// 種類を補う詳細 (メッセージの出し分けに使う)。
    pub detail: Option<FsErrorDetail>,
    /// 元の OS のエラー (ログ用)。
    pub source: Option<io::Error>,
}

impl FsError {
    /// 詳細と元のエラーを持たないエラーを作る。
    pub fn new(kind: FsErrorKind) -> FsError {
        FsError {
            kind,
            detail: None,
            source: None,
        }
    }

    /// 詳細つきのエラーを作る (元のエラーは持たない)。
    pub fn with_detail(kind: FsErrorKind, detail: FsErrorDetail) -> FsError {
        FsError {
            kind,
            detail: Some(detail),
            source: None,
        }
    }
}

/// `FsError` の種類。保存の変種は、元ファイルが無傷か (何も書いていないか) を区別する (05 §4)。
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FsErrorKind {
    // ── 開く・調べる・一般 ──
    /// パスが存在しない。
    NotFound,
    /// 権限が無い。
    PermissionDenied,
    /// 共有違反・ロック。
    Busy,
    /// 構文・予約名・循環。
    InvalidPath,
    /// ディスクの容量が足りない。
    DiskFull,
    /// 読み取り専用の FS。
    ReadOnlyFs,
    /// その他の失敗。
    Other,
    /// この Port はその操作を実装していない (スワップの `write_swap` など)。呼び側は以後その機能を使わない。
    Unsupported,
    // ── 保存 ──
    /// `SaveOptions::expected` と不一致。何も書いていない。`current` は照合の時点の保存先の Stamp で、
    /// `None` は保存先が存在しない (開いた後に削除された)。
    Conflict {
        /// 照合の時点の保存先の Stamp。保存先が存在しなければ `None`。
        current: Option<FileStamp>,
    },
    /// 原子的置換ができない・意味が変わる。何も書いていない。
    AtomicUnsupported,
    /// `ReadChunks` の供給側の失敗。元ファイルは無傷。
    SourceFailed,
    /// Windows の置換途中失敗: 元ファイルは退避名に完全に残る。
    ReplacedMovedToBackup {
        /// 元ファイルが残っている退避名のパス。
        backup: FsPath,
    },
    /// `write_in_place` の事前バックアップ、または保存中フラグの書込に失敗した (後者は `detail = InflightFlag`)。何も書いていない
    /// (元ファイルは無傷)。core は「バックアップ無しで上書きしますか」を確認し、承認されたら `SkipConfirmed` で再投入する。
    BackupFailed,
    /// `write_in_place` の書込途中の失敗。元ファイルは壊れている可能性がある。`Some` なら事前バックアップは完全で、
    /// core はそのパスを必ず示す。`None` は「バックアップなしで上書き」を承認していた (元の内容は取り戻せない)。
    InPlaceFailed {
        /// 完全な事前バックアップのパス。承認によりバックアップを作らなかったときは `None`。
        backup: Option<FsPath>,
    },
}

/// `FsError` の詳細 (`kind` だけでは出し分けられないメッセージの材料)。
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FsErrorDetail {
    /// Windows の予約名 (CON・NUL など)。`kind` は `InvalidPath`。
    ReservedName,
    /// リンク先が存在しない。`kind` は `NotFound`。
    DanglingLink {
        /// 存在しないリンク先。
        target: FsPath,
    },
    /// 親フォルダが無い。`kind` は `NotFound`。
    ParentMissing {
        /// 存在しない親フォルダ。
        dir: FsPath,
    },
    /// ハードリンクの `Ask` による `AtomicUnsupported`。
    Hardlink {
        /// 保存先のハードリンクの数。
        nlink: u32,
    },
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
