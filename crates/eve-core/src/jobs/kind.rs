//! ジョブの種類 (投入する側が作る値、docs/02 §4.4)。

use eve_encoding::{Encoding, EscapePolicy};
use eve_text::Snapshot;
use eve_view::{DocumentId, JobId, Revision};

use crate::document::PreservedMap;
use crate::io::fs::{Expect, HardlinkPolicy, InPlaceBackup};
use crate::io::fspath::FsPath;
use crate::io::io_jobs::OpenOptions;

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
        /// 保持方式の記録の、保存を始めた時点の版 (03 §8.6 E2 / §8.7.1、v0.4。渡した後の編集の影響を受けない)。
        /// 書く文字コードが文書の文字コードと同じで、保持が効いている (`PreserveStatus::Active`) ときだけ渡し、ほかは空。
        /// 空なら記録を使わない (v0.3 と同じ出力)。
        preserved: PreservedMap,
    },
    /// 表現不能の文字を置換する `ChangeSet` を作る (E3 の選択肢 2、03 §8.6)。`snapshot` は投入時の Revision のもの。
    ReplaceUnmappable {
        /// 置換の対象の本文。
        snapshot: Snapshot,
        /// 表現できるかを調べる文字コード。
        encoding: Encoding,
        /// 不正バイトのエスケープ文字の扱い (03 §8.3)。
        escape: EscapePolicy,
        /// 保持方式の記録の版 (`Save` と同じ条件で渡す)。保存と同じ計画で元のバイト列を書く範囲の文字は、置換の対象に
        /// しない (03 §8.6 E2)。
        preserved: PreservedMap,
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
