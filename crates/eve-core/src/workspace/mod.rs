//! ワークスペース (docs/03 §12.7、07 §12.1。v0.5 の M31)。
//!
//! - [`root`]: ルートの検出 (`JobKind::WorkspaceRoot`、03 §12.7.1) と、core が持つワークスペースの状態、起動の引数の
//!   ディレクトリの知らせの保留 (段 1)。
//! - [`WorkspaceWalk`][]: 走査の状態 (段 4)。深さ優先で、項目を名前のバイト列の順に並べ、除外の連鎖
//!   ([`ExcludeChain`](crate::ExcludeChain)) で照らす。区切りはディレクトリ 16 個・項目 4,096 個・8 ms の早いほう。
//! - [`Candidate`][]: ファインダの候補。走査の側 (`bg`) で作り、4,096 件ごとの不変の塊 (`Arc<[Candidate]>`) で送る。
//! - ジョブ `JobKind::WorkspaceScan` (区切りごとに `JobStep::Continue` で積み直す) と `JobKind::ReleaseCandidates`
//!   (古い一覧を `bg` で解放する。主スレッドで大きな一覧を drop しない)。
//!
//! 走査はルートを引数で受け取る (ファインダへのつなぎ込みは段 6)。
//!
//! spec: 03 §12.7.1, 03 §12.7.2, 03 §12.7.7, 03 §12.7.8, 02 §9.3, 07 §12.1, 07 §18.2, 07 §18.3

mod candidate;
pub(crate) mod finder;
mod finder_match;
mod git;
pub(crate) mod root;
mod scan_job;
mod walk;

pub use candidate::{Candidate, char_bit};
pub use finder_match::{
    BONUS_BOUNDARY, BONUS_CAMEL, BONUS_CONSECUTIVE, BONUS_FILENAME_PER_CHAR, FINDER_CANCEL_EVERY,
    FINDER_TOP, FIRST_CHAR_MULTIPLIER, FinderHit, FinderMatches, FinderQuery, MATCH_SCORE,
    PENALTY_GAP_EXTENSION, PENALTY_GAP_START, finder_match,
};
pub use root::{RootSource, Workspace};
pub(crate) use scan_job::run_scan;
pub use walk::{NoObserver, WalkObserver, WalkStep, WorkspaceWalk};

use crate::time::Millis;

/// 1 区切りで読むディレクトリの数 (S-26 の決める点 3)。
pub const WALK_DIRS_PER_STEP: usize = 16;
/// 1 区切りで処理する項目の数 (1 つのディレクトリの項目が多いときも、照合と候補の用意をここで区切る)。
pub const WALK_ENTRIES_PER_STEP: usize = 4096;
/// 1 区切りの時間の目安 (区切りの始めから過ぎたら、読んだディレクトリの後で区切る)。
pub const WALK_STEP_TIME: Millis = Millis(8);
/// `read_dir` に渡す 1 つのディレクトリの項目の上限 (残りの件数は渡さない。結果を決定的にするため、03 §12.7.2 の 3)。
pub const WALK_ENTRIES_PER_DIR_MAX: usize = 200_000;
/// 候補 (除外に当たらないファイル) の数の上限。超えるときは走査をやめて打ち切りの印を立てる。
pub const WORKSPACE_MAX_FILES: u64 = 200_000;
/// 列挙した項目の総数の上限 (除外される項目とディレクトリばかりの木に備える)。
pub const WORKSPACE_MAX_ENTRIES: u64 = 2_000_000;
/// 降りる深さの上限 (ルートの直下が 1)。それより深いディレクトリには降りず、打ち切りの印を立てる。
pub const WORKSPACE_MAX_DEPTH: usize = 256;
/// 候補の塊の大きさ (主スレッドへ送る単位)。
pub const CANDIDATE_CHUNK: usize = 4096;
/// 塊が [`CANDIDATE_CHUNK`] 件に満たなくても、前に送ってからこれだけ過ぎたら区切りの終わりで送る (ファインダの照合の
/// やり直しの間隔と同じ 100 ms、03 §12.7.4)。
pub const SCAN_FLUSH_INTERVAL: Millis = Millis(100);

/// ファインダを閉じてから一覧を手放すまでの時間 (03 §12.7.2 の「一覧の寿命」の `FINDER_LIST_KEEP`)。10 万件で数十 MB を
/// 持ち続けない。
pub const FINDER_LIST_KEEP: Millis = Millis(5 * 60 * 1000);
/// 最初の走査の間に、届いた新しい候補で照合し直す間隔 (03 §12.7.4 の「取消しと投げ直し」)。前の照合を始めてからこれだけ
/// 経っていなければ、経つまで待つ (束が届くたびに投げ直すと `bg` に照合が積み上がる)。
pub const FINDER_RESCORE_INTERVAL: Millis = Millis(100);

/// 除外の設定 (走査を始める時点の値。走査の途中で設定が変わっても、次の走査から効く、03 §12.7.2)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExcludeConfig {
    /// `workspace.exclude` の各要素 (ルートに置いた `.gitignore` の 1 行と同じ書式、07 §5.9)。
    pub exclude: Vec<String>,
    /// `search.workspace.respect_gitignore` (`.gitignore` と `info/exclude` を読む)。
    pub respect_gitignore: bool,
    /// `search.workspace.follow_symlinks` (シンボリックリンクのディレクトリに降りる)。
    pub follow_symlinks: bool,
}

impl ExcludeConfig {
    /// 既定 (`workspace.exclude` は空、`.gitignore` を読む、シンボリックリンクに降りない)。`workspace.exclude` の既定の値は
    /// 設定 (07 §5.9) が持つ。
    #[must_use]
    pub fn new() -> ExcludeConfig {
        ExcludeConfig {
            exclude: Vec::new(),
            respect_gitignore: true,
            follow_symlinks: false,
        }
    }
}

impl Default for ExcludeConfig {
    fn default() -> ExcludeConfig {
        ExcludeConfig::new()
    }
}
