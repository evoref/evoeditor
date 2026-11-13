//! ジョブの結果と失敗の型 (docs/02 §4.4)。

pub use eve_encoding::EscapeMergeAt;
use eve_encoding::{Encoding, UnsupportedBom};
use eve_text::Text;
use eve_view::JobId;

use crate::document::{PreservedMap, ReadOnlyReasons, TextFormat};
use crate::io::fs::{FileMeta, FileStamp, FsError, InterruptedSave, RecoveryDirEntry, SaveReport};
use crate::io::fspath::FsPath;

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

/// エスケープ文字のつながりの承認 (「このまま保存」、03 §8.3.1 の「承認」)。承認した結果 (件数・先頭 100 件・全件の
/// ダイジェスト) を持ち、保存のジョブ (`JobKind::Save` の `escape_merge_ok`) は結果がこれと同じときだけ止めない。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EscapeMergeApproval {
    /// 承認したつながりの件数。
    pub total: u64,
    /// 承認したつながりの先頭 100 件。
    pub recorded: Vec<EscapeMergeAt>,
    /// 承認したつながりの全件 (位置・強調の範囲・局所の復号) のダイジェスト (`JobResult::EscapeMerge` の `digest`)。
    /// 先頭 100 件より後ろの違いも見る。
    pub digest: u64,
}

/// 読込の完了に添える通知 (info メッセージの材料)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OpenNotes {
    /// 先頭の判定が外れて、判定し直して読み直した (03 §8.2.1。`info.open.redetected`)。
    pub redetected: Option<Encoding>,
    /// chardetng が 7 種にない文字コードを推定した (その名前。`info.open.guess_unsupported`)。
    pub guess_unsupported: Option<&'static str>,
}

/// ジョブの結果と、それが属するジョブ (02 §4.4)。受け取る側は投入時の Revision で古い結果を捨てる (ADR-0020)。
#[derive(Debug)]
pub struct JobOutput {
    /// 結果が属するジョブ。
    pub job: JobId,
    /// 結果の中身。
    pub body: JobResult,
}

/// ジョブの結果 (02 §4.4)。結果が属するジョブは [`JobOutput`] が持つ。受け取る側は投入時の Revision で古い結果を捨てる (ADR-0020)。
#[derive(Debug)]
#[non_exhaustive]
#[allow(
    clippy::large_enum_variant,
    reason = "読込の完了 (`Complete`) は 1 つのジョブで 1 回だけ運ぶ。v0.4 で保持方式の状態と表が増えて閾値を超えたが、箱に入れても得が無い"
)]
pub enum JobResult {
    /// `Metadata` の結果。
    Meta {
        /// ファイルの属性。
        meta: FileMeta,
        /// 外部変更の検出に使う Stamp。
        stamp: FileStamp,
    },
    /// 読込の完了。
    Complete {
        /// 読み込んだ本文。
        text: Text,
        /// 判定した形式 (文字コード・BOM・改行コードなど)。保持方式の状態は `format.roundtrip.preserve`。
        format: TextFormat,
        /// 保持方式の記録の表 (03 §8.7.1、02 §9.3)。ジョブが本文と照合済み (照合に失敗したら空で、状態は「保持なし」)。
        preserved: PreservedMap,
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
        /// 読もうとしたテーマ名。
        name: String,
        /// 組み立てたテーマ。
        theme: Option<eve_config::theme::Theme>,
        /// ファイルごとの診断 (ファイルの名前と診断の組)。
        reports: Vec<(String, Vec<eve_config::Diagnostic>)>,
    },
    /// `ReadConfig` の結果。
    ConfigRead {
        /// `files` と同じ順。
        config: Vec<ConfigFile>,
        /// `keymap` を読まない (`--no-config`) ときは `None`。
        keymap: Option<ConfigFile>,
    },
    /// 起動時の退避ディレクトリの走査の結果 (新しい順ではなく列挙した順。main が整えて案内する)。
    RecoveryFound {
        /// 見つけた退避ディレクトリ。
        dirs: Vec<RecoveryDirEntry>,
        /// 他のプロセスが残したスワップ (復旧の候補、03 §10.3)。
        swaps: Vec<crate::SwapEntry>,
        /// 置換途中の痕跡の提案 (05 §4.6)。新しい順。
        leftovers: Vec<crate::Leftover>,
    },
    /// 存在しないパスを開いた (新規ファイル)。フォルダも無ければ `parent_missing` にそのフォルダ。
    NewFile {
        /// 存在しない親フォルダ (あれば)。
        parent_missing: Option<FsPath>,
        /// 残っていた保存中フラグから見つけた、中断された保存の痕跡 (新規作成の途中で中断した、置換の途中で退避名に残った、等)。
        interrupted_save: Option<InterruptedSave>,
    },
    /// 続ける前に利用者の確認が要る (03 §9.2)。回答を得た main が再投入する。
    NeedsConfirm {
        /// 確認が要る理由。
        ask: ConfirmAsk,
    },
    /// 保存の完了。
    Saved {
        /// 保存の報告。
        report: SaveReport,
    },
    /// 保存先の文字コードで表現できない文字がある (03 §8.6 E1)。元のファイルには何も書いていない。
    Unmappable {
        /// 総数。
        total: u64,
        /// 先頭 100 件。
        recorded: Vec<eve_encoding::Unmappable>,
        /// 総数のうち、§8.3 のエスケープ文字の数 (全件で数える)。1 以上なら E3 の選択肢 1・2 を出さない (03 §8.6)。
        escapes_unmappable: u64,
        /// この保存で元のバイトに戻したエスケープ文字の数 (v0.4 の M23)。1 以上なら E3 の「UTF-8 で保存」を出さない
        /// (UTF-8 にするとエスケープ文字が表現不能になり、もう一度 E3 になる。03 §8.3.1 の「E3 との関係」)。
        escapes_restored: u64,
        /// 保存先の文字コード。
        encoding: Encoding,
    },
    /// エスケープ文字を元のバイトに戻すと隣とつながり、開き直すと別の文字になる箇所がある (03 §8.3.1 の「つながりの
    /// 検出」。文書の先頭の BOM を含む)。承認 (`escape_merge_ok`) が無い、または結果が承認と違う保存で、乾式走査の段階で
    /// 見つけたときだけ返す。何も書いていない (一時ファイルも作らない)。
    EscapeMerge {
        /// つながりの件数。
        total: u64,
        /// 先頭 100 件。
        recorded: Vec<EscapeMergeAt>,
        /// 全件 (位置・強調の範囲・局所の復号) のダイジェスト (承認に写す)。
        digest: u64,
        /// 開き直すと変わる範囲の合計 (本文の文字数)。
        changed_chars: u64,
        /// 開き直すと変わる範囲の合計 (本文の行数)。
        changed_lines: u64,
        /// 変わる範囲が文書の末尾まで戻らない。
        to_eof: bool,
    },
    /// 表現不能の文字の置換の `ChangeSet` ができた (`targets` は先頭 100 件。総数は `counts` の合計)。
    UnmappableReplacement {
        /// 置換の `ChangeSet` (投入時の Revision に対するもの)。
        changes: eve_text::ChangeSet,
        /// 置換する箇所 (先頭 100 件)。
        targets: Vec<crate::io::unmap::ReplaceTarget>,
        /// 置換の種類ごとの件数。
        counts: crate::io::unmap::ReplaceCounts,
    },
    /// 「退避先に保存」の成功。`path` は実際に書いたディレクトリ (代替の退避先を含む)。
    RecoveryWritten {
        /// 実際に書いた退避ディレクトリ。
        path: FsPath,
    },
    /// スワップを書いた。
    SwapWritten,
    /// スワップを消した。
    SwapDiscarded,
    /// 他のプロセスのスワップの本文を読んだ。
    SwapRead {
        /// 読んだスワップの本文。
        body: crate::SwapBody,
    },
    /// 他のプロセスのスワップを消した。
    SwapRemoved,
    /// 置換途中の痕跡の操作を終えた。
    LeftoverDone,
    /// 前回起動した版との比較の結果 (`LastVersion`)。
    LastVersionChecked {
        /// 比較の結果。
        outcome: crate::LastVersionOutcome,
    },
    /// 単発の検索の結果。`found` は (一致, 文書の端を越えたか)。
    FindNextDone {
        /// 見つけた一致 (バイト範囲) と、文書の端を越えて折り返したか。見つからなければ `None`。
        found: Option<(eve_text::ByteRange, bool)>,
    },
    /// 改行コードの変換の `ChangeSet` ができた。
    ConvertEolDone {
        /// 変換の `ChangeSet` (投入時の Revision に対するもの)。
        changes: eve_text::ChangeSet,
    },
    /// すべて置換の `ChangeSet` ができた (`count` は置換する件数)。
    ReplaceAllDone {
        /// 置換の `ChangeSet` (投入時の Revision に対するもの)。
        changes: eve_text::ChangeSet,
        /// 置換する件数。
        count: u64,
        /// そのうち CP932 互換文字の同一視で当たった件数 (03 §12.4.1。知らせを分ける)。
        folded: u64,
    },
    /// 読込の進捗 (03 §9.2 手順 3)。開く操作から 50 ms 以内に最初の 1 件を送り、以後は 10 Hz 以下。終わりではない。
    Progress {
        /// 読み終えたバイト数。
        bytes: u64,
        /// ファイルの大きさ (metadata の len)。
        total: u64,
    },
    /// 先行表示の先頭 (03 §9.2.1。v0.4 の M25)。終わりではない (この後も `Progress` と終端の結果が届く)。
    /// `OpenOptions.partial` が立った `Open` だけが、読込が 200 ms を過ぎても終わらないとき 1 回送る (判定のやり直しの後は
    /// 送り直す)。
    Partial {
        /// 固定の先頭 (復号した本文の先頭 1 MiB まで、行の区切りで切る)。全文 (`Complete.text`) の接頭辞。
        head: Text,
        /// 先頭の時点の形式 (表示にだけ使う。改行の統計・不正バイトの数・長大行の数は先頭の分。往復の要約は空)。
        format: TextFormat,
    },
    /// 検索の途中経過。一致を走査した順に届く (終わりではない)。
    SearchBatch {
        /// 今回の一致 (バイト範囲、走査した順)。
        matches: Vec<eve_text::ByteRange>,
        /// 起点 (`Search` の `from`) 以降に始まる最初の一致。行をまたぐ検索の規則 (全件を文書の先頭から走査する、03 §12.3.1)
        /// で、見つけたときに保持の上限に依らず 1 回だけ送る。それ以外は `None`。
        first_after: Option<eve_text::ByteRange>,
    },
    /// 検索の終わり。`total` は数えた件数、`truncated` は保持の上限を超えた、`count_limited` は数える上限で打ち切った。
    SearchDone {
        /// 数えた一致の件数。
        total: u64,
        /// 数える上限 (`count_limit`) で打ち切った。
        count_limited: bool,
        /// 保持の上限 (`max_matches`) を超えた。
        truncated: bool,
        /// 再開の位置 (正本の走査がそこから続く位置、1 MiB ごと、昇順。行をまたぐ検索の規則のときだけ。03 §12.3.1)。
        resume: Vec<eve_text::ByteOff>,
        /// 探さなかった・重すぎて打ち切った理由 (03 §12.3.1)。あれば件数は 0 で、それまでに届いた一致は捨てる。
        limit: Option<crate::search::ScanLimit>,
    },
    /// 1 件置換の対象の確かめ (`ReplaceOne`) の結果。`insert` は置換文字列 (対象が一致でなければ `None`)。
    ReplaceOneDone {
        /// 置換文字列。`range` が一致でなければ `None`。
        insert: Option<String>,
    },
    /// 単発の検索・すべて置換・1 件置換の確かめが、探さなかった・重すぎて打ち切った (03 §12.3.1 の「探さない場合」と「最悪の入力の上限」)。
    /// 何も移らず、何も置き換えない。
    SearchLimited {
        /// 理由。
        limit: crate::search::ScanLimit,
    },
    /// ジョブの失敗 (キャンセルを含む)。
    Failed {
        /// 失敗の理由。
        err: JobError,
    },
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
    /// 保持方式の記録を使う保存の確かめに合わない (03 §8.6 E2): 表の記録が確認 (i) に合わない・表の不変条件に反する、
    /// または出力の全体を同じ文字コードで復号しても本文に一致しない (確認 (ii))。実装の誤りへの備えで、ふつうは起きない。
    /// 何も書いていない (乾式走査で見つけて Port を呼ばない。本番の書き込みで見つけたら、原子的保存は一時ファイルを
    /// 捨てる)。core は文書の記録を全部捨てて「保持なし」にする。
    VerifyFailed,
}

/// 設定ファイル 1 つの読込の結果。無ければ `Ok(None)`、読めなければ理由。
pub type ConfigFile = Result<Option<Vec<u8>>, String>;
