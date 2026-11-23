//! ジョブの種類 (投入する側が作る値、docs/02 §4.4)。

use eve_encoding::{Encoding, EscapePolicy};
use eve_text::Snapshot;
use eve_view::{DocumentId, JobId, Revision};

use crate::document::PreservedMap;
use crate::io::fs::{Expect, HardlinkPolicy, InPlaceBackup};
use crate::io::fspath::FsPath;
use crate::io::io_jobs::OpenOptions;
use crate::jobs::EscapeMergeApproval;

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
        /// `theme.name` を書いた所 (テーマが無いときの診断の場所、07 §8.7)。分からなければ `None`。
        origin: Option<eve_config::KeyOrigin>,
    },
    /// 設定ファイル (ユーザー層の `config.toml` を順に) と `keymap.toml` を読む (`config.reload`、07 §6)。
    ReadConfig {
        /// 読む設定ファイル。層の低い順。
        files: Vec<FsPath>,
        /// 読む `keymap.toml`。`--no-config` のときは `None`。
        keymap: Option<FsPath>,
        /// 一覧する `locales/` と選んだ言語 (候補と、選んだ言語のファイルの大きさと更新日時だけを調べる。中身は読まない、
        /// 07 §17.4)。`None` は調べない。
        locales: Option<(FsPath, eve_config::Locale)>,
    },
    /// 言語定義を読んで重ねる (07 §9.6 の LANG-2、02 §9.3 の `ReadLanguages`)。起動時は、起動時のパスを開くジョブより前に
    /// 積む (`io` は直列なので、起動時の文書は定義が届いた後に開き終わる)。`config.reload` でも積む。`dir` の
    /// `*.toml` を名前の順に読み、`config` (設定ファイルの `[[language]]`) の後に重ねる。`first_line` の正規表現は、起動時の
    /// 1 回は組まず (組むのは `PrecompileLanguages`。組む時間で起動時の文書を開くのを待たせない)、ほか (`config.reload`・
    /// ワークスペース層が届いた契機) はジョブの中で組む (`precompile`)。結果は `JobResult::LanguagesRead`。
    ReadLanguages {
        /// `<config_dir>/languages`。`None` は読まない (`--no-config`)。
        dir: Option<FsPath>,
        /// 設定ファイルの `[[language]]` の定義 (重ねる順)。
        config: Vec<eve_config::LanguageDef>,
        /// ワークスペース層の `[[language]]` の定義 (`.eve/config.toml`、07 §9.6 の LANG-2)。空でなければ、ユーザー層の後に
        /// 重ねた集まり (ルートの下の文書の判定に使う) も作る。
        workspace: Vec<eve_config::LanguageDef>,
        /// ジョブの中で `first_line` の正規表現まで組む (起動時の 1 回のほか。差し替えた集まりで主スレッドが判定するときに
        /// 組まない)。
        precompile: bool,
        /// ワークスペース層のファイルの表示名 (ワークスペース層の集まりの診断は、このファイルの分だけ返す)。
        workspace_files: Vec<String>,
    },
    /// ワークスペース層 (`<root>/.eve/config.toml`・`.eve/keymap.toml`) を所有者の検査つきで読み、解析・分類する (07 §12.5 の
    /// WSC-1 / WSC-2、02 §9.3 の `ReadWorkspaceConfig`)。起動時はルートが決まったとき、`config.reload` はユーザー層を
    /// 読み直した後に積む。結果は `JobResult::WorkspaceConfigRead`。
    ReadWorkspaceConfig {
        /// ワークスペースのルート。
        root: FsPath,
        /// 条件付きセクションの条件の値 (07 §3.4 の COND-2)。
        conditions: eve_config::Conditions,
        /// `workspace.safe_directories` (07 §12.1 の OWN-5)。
        safe_dirs: Vec<FsPath>,
    },
    /// 言語定義の `first_line` の正規表現を組む (07 §9.6 の段 2 の細目)。`ReadLanguages` の結果が届いたときに積む (起動時は
    /// 起動時のパスを開くジョブの後ろになる)。結果は `JobResult::LanguagesPrecompiled`。
    PrecompileLanguages {
        /// 組む定義の集まり (主スレッドと共有する。組んだ正規表現はそのまま判定に使う)。
        set: std::sync::Arc<eve_config::LanguageSet>,
        /// ワークスペース層の定義も重ねた集まり (あれば。ルートの下の文書の判定に使う。主スレッドで組まない)。
        workspace: Option<std::sync::Arc<eve_config::LanguageSet>>,
        /// ワークスペース層のファイルの表示名 (`workspace` の診断はこのファイルの分だけ返す。ユーザー層の分は `set` と同じ)。
        workspace_files: Vec<String>,
    },
    /// ファイルの `metadata` と Stamp を調べる。
    Metadata {
        /// 調べるパス。
        path: FsPath,
        /// 結果を受けた側の分岐に使う目的。
        purpose: MetaPurpose,
        /// 古い識別の確かめに使う、投入した時点の文書の識別 (`MetaPurpose::SaveAsTarget` だけが入れる。03 §12.7.6 の 0)。
        known_ids: Vec<KnownFileId>,
    },
    /// 起動時の退避ディレクトリの走査 (02 §9.3 の `RecoveryScan`。メインスレッドでは列挙しない)。
    RecoveryScan,
    /// 前回起動した版の記録を読み、今の版 `current` と比べて書き直す (FR-518、07 §18.8)。
    LastVersion {
        /// 今の版 (`CARGO_PKG_VERSION` の形の文字列)。
        current: String,
    },
    /// ワークスペースのルートの検出 (03 §12.7.1、規則は 07 §12.1。v0.5 の M31)。起動時に 1 回、起動時のパスを開くジョブの
    /// 後ろに積む (`io`)。結果は `JobResult::WorkspaceRoot`。
    WorkspaceRoot {
        /// `--workspace` / `-w` の値 (正規化したパス)。
        explicit: Option<FsPath>,
        /// 位置引数 (正規化したパス。`-` を除く)。
        args: Vec<FsPath>,
        /// カレントディレクトリ。
        cwd: FsPath,
        /// 目印の名前 (`workspace.root_markers`、07 §5.9)。
        markers: Vec<String>,
    },
    /// ファイルを開く (03 §9.2)。文字コードは判定する (03 §8.2) か、`opts.forced` で指定する。
    Open {
        /// 開くパス。
        path: FsPath,
        /// 開くときの指定 (文字コードの強制など)。
        opts: OpenOptions,
        /// 古い識別の確かめに使う、投入した時点の文書の識別 (03 §12.7.6 の 0)。再読込・開き直しは空。
        known_ids: Vec<KnownFileId>,
    },
    /// 標準入力を読んで開く (`evoeditor -`、03 §9.2.2。v0.4 の M25)。`JobPorts.stdin` から `ReadChunks` を取って読む。
    /// 走らせるのは `io` でも `bg` でもない専用のワーカー `eve-stdin-job` (02 §9.1。OQ-219 の 8)。確認を出さない
    /// (`read_only_requested` を立てたのと同じ)。`opts` は `forced`・`user_read_only`・`default_eol`・
    /// `huge_file_warn_bytes` (上限)・`long_line_threshold`・`pending` だけを使う (`partial` は見ない)。取消し (`CancelToken`)
    /// は「読むのをやめて、読んだところまでを `StdinDone` で返す」意味 (`Failed { Cancelled }` にしない)。
    OpenStdin {
        /// 開くときの指定。
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
        /// エスケープ文字のつながり (03 §8.3.1) の承認 (「このまま保存」)。ジョブは承認があってもつながりを調べ、結果
        /// (件数・先頭 100 件・全件のダイジェスト) が承認と同じときだけ止めずに書く (違えば何も書かずに
        /// `JobResult::EscapeMerge`)。承認しても区切った確認 (ii) は行う。
        escape_merge_ok: Option<EscapeMergeApproval>,
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
        /// 正本の走査を始められる位置 (再開の位置と今の一致の終端、昇順。行をまたぐ検索の規則だけが使う、03 §12.3.1 の
        /// 「次 / 前」)。空なら文書の先頭から。
        resume: Vec<eve_text::ByteOff>,
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
    /// 1 件置換の対象の確かめとキャプチャの取り直し (03 §12.3.1 の「置換」。重い検索語と上限の無い検索語。メインスレッドで
    /// 照合しない)。`from` から走査の入口で選び始め、`range.start` で始まる一致が `range` そのものなら、その置換文字列を返す。
    ReplaceOne {
        /// 置換の対象の本文。
        snapshot: Snapshot,
        /// 検索の条件。
        query: crate::search::SearchQuery,
        /// 選び始める正本の位置。
        from: eve_text::ByteOff,
        /// 置き換える一致。
        range: eve_text::ByteRange,
        /// 置換の文字列 (`$1` などを展開する前)。
        replacement: String,
    },
    /// ワークスペースの候補の走査 (`bg`。03 §12.7.2 / §12.7.7)。1 区切り (ディレクトリ 16 個・項目 4,096 個・8 ms の早いほう)
    /// ごとに、溜まった塊を `JobResult::ScanBatch` で送り、続き (`walk` に状態を持つ) を `JobStep::Continue` で返す。
    /// 終われば `JobResult::ScanDone`、ルートを読めなければ `JobResult::Failed { Fs }`。`doc` は `None`。
    WorkspaceScan {
        /// ワークスペースのルート (絶対パス)。
        root: FsPath,
        /// 除外の設定 (走査を始める時点の値)。
        exclude: std::sync::Arc<crate::workspace::ExcludeConfig>,
        /// 走査の世代 (結果を照らして古いものを捨てる、03 §12.7.2 の 8)。
        generation: u64,
        /// 積み直しの続き (最初の投入は `None`)。
        walk: Option<Box<crate::workspace::WorkspaceWalk>>,
    },
    /// ファインダの照合 (`bg`。03 §12.7.4)。`chunks` の全候補を `query` で照らし、一致した件数と上位 `top` 件を
    /// `JobResult::FinderMatched` で返す。区切らない (照合 1 回)。候補 4,096 件ごとに取消しを見る。`doc` は `None`。
    FinderMatch {
        /// 照らす候補の塊 (主スレッドの一覧の `Arc` の写し)。
        chunks: Vec<std::sync::Arc<[crate::workspace::Candidate]>>,
        /// 検索語 (入力欄の文字列のまま)。
        query: std::sync::Arc<str>,
        /// 上位の件数 (`FINDER_TOP`)。
        top: usize,
        /// 照合の世代 (結果を照らして古いものを捨てる)。
        generation: u64,
    },
    /// 古い候補の一覧と横断検索の結果を `bg` で解放する (主スレッドで大きな一覧を drop しない、03 §12.7.2 / §12.7.5)。
    /// 終端の結果は `JobResult::Released` だけ。取り消さない。
    ReleaseCandidates {
        /// 手放す塊 (照合のジョブが同じ塊を持っていれば、最後の参照はジョブの側で外れる)。
        chunks: Vec<std::sync::Arc<[crate::workspace::Candidate]>>,
        /// 手放す横断検索の結果。
        hits: Vec<crate::workspace::WsHit>,
    },
    /// 横断検索 (`bg`。03 §12.7.5 / §12.7.7)。`WorkspaceWalk` でファイルを見つけ、見つけた順に読込と同じ規則で復号して
    /// 探す。1 区切り (ディレクトリ 16 個・ファイル 64 個・中身 1 MiB・8 ms の早いほう) ごとに `JobResult::WorkspaceHits` を
    /// 送り、続き (`state`) を `JobStep::Continue` で返す。終われば `JobResult::WorkspaceSearchDone`、ルートを読めなければ
    /// `JobResult::Failed { Fs }`。`doc` は `None`。
    WorkspaceSearch {
        /// ワークスペースのルート (絶対パス)。
        root: FsPath,
        /// 検索の世代 (結果を照らして古いものを捨てる)。
        generation: u64,
        /// 検索の条件 (エディタ内の検索と同じ)。
        query: crate::search::SearchQuery,
        /// 除外の設定 (走査を始める時点の値)。
        exclude: std::sync::Arc<crate::workspace::ExcludeConfig>,
        /// ルートの下にある開いている文書 (03 §12.7.6 の 3)。
        open_docs: Vec<crate::workspace::OpenDocRef>,
        /// 探すファイルの大きさの上限 (バイト)。
        max_file_size: u64,
        /// 結果の行の上限。
        max_lines: usize,
        /// 積み直しの続き (最初の投入は `None`)。
        state: Option<Box<crate::workspace::WorkspaceSearchState>>,
    },
    /// 文書内の全件検索 (03 §12.2)。`from` の行から末尾まで、続けて先頭から走査し (行をまたぐ検索の規則では文書の先頭から
    /// 末尾まで、03 §12.3.1)、一致を `SearchBatch` で順に返して `SearchDone` で終わる。保持するのは `max_matches` 件まで、
    /// 数えるのは `count_limit` 件まで。
    Search {
        /// 検索する本文。
        snapshot: Snapshot,
        /// 検索の条件。
        query: crate::search::SearchQuery,
        /// 起点 (バイト。この位置のある行から走査する。行をまたぐ検索の規則では、この位置以降の最初の一致を `first_after` で知らせる)。
        from: eve_text::ByteOff,
        /// 保持する一致の上限 (件)。
        max_matches: usize,
        /// 数える一致の上限 (件)。
        count_limit: u64,
    },
}

/// 1 回の `run_job` の終わり方 (02 §9.3)。区切って譲るジョブ (`WorkspaceScan`) は、1 区切りを終えると終端の結果を送らずに
/// 続きを返し、ワーカーのレーンが **同じ `JobId`・同じ `CancelToken` のまま** 自分のキューの末尾へ積む (間に投入された
/// ジョブが先に走る)。ほかのジョブは `Done` だけを返す。
#[derive(Debug)]
#[must_use = "Continue を積み直さないと終端の結果が届かない"]
#[expect(
    clippy::large_enum_variant,
    reason = "run_job の戻り値として 1 回ずつ動かすだけで、並べて持たない (続きは JobRequest の kind に戻す)"
)]
pub enum JobStep {
    /// 終端の結果を送った (または送らないジョブが終わった)。
    Done,
    /// 続きがある。`JobRequest` の `kind` をこれに替えて積み直す (`id`・`doc`・`base` はそのまま)。
    Continue(JobKind),
}

impl JobStep {
    /// 終わった (続きが無い)。区切らないジョブを直に走らせる試験・ベンチが確かめるのに使う。
    #[must_use]
    pub fn is_done(&self) -> bool {
        matches!(self, JobStep::Done)
    }

    /// 続きの積み直しの規則 (本番のレーンと `ManualJobSpawner` が共通に使う、02 §9.3)。`Done` なら `None`。`Continue` なら、
    /// `cancel` が立っていれば続きを手放して (呼び出し側のスレッド = ワーカー) `Failed { Cancelled }` を終端として `emit` し、
    /// `None`。立っていなければ、同じ `id`・`doc`・`base` の依頼を返す (呼び出し側が同じ `cancel` と一緒に末尾へ積む)。
    pub fn requeue(
        self,
        id: JobId,
        doc: Option<DocumentId>,
        base: Revision,
        cancel: &crate::CancelToken,
        emit: &mut dyn FnMut(super::JobOutput),
    ) -> Option<JobRequest> {
        let JobStep::Continue(kind) = self else {
            return None;
        };
        if cancel.is_cancelled() {
            drop(kind);
            emit(super::JobOutput {
                job: id,
                body: super::JobResult::Failed {
                    err: super::JobError::Cancelled,
                },
            });
            return None;
        }
        Some(JobRequest {
            id,
            doc,
            base,
            kind,
        })
    }
}

/// 投入した時点の文書の、同じファイルの判定に使ってよい識別 (03 §12.7.6 の「古い識別の確かめ」)。ジョブは、調べたファイルの
/// 識別がこれと等しければ `path` を `metadata` し直し、今も同じ識別かを確かめる。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownFileId {
    /// 文書のパス。
    pub path: FsPath,
    /// 文書の識別 (`disk_stamp` の値。ファイルが外で作り直されると古い)。
    pub file_id: crate::io::fs::FileId,
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
