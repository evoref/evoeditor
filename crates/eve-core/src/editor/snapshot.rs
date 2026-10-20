//! 退避用の公開物 (`DocSnapshot`、docs/02 §5.4 / §11.2.1)。Host が変更のあった文書ごとにスロットへ公開し、
//! 緊急退避の書き手が Editor に触れずに読む。

use eve_text::Snapshot;
use eve_view::{DocumentId, Revision};

use crate::document::TextFormat;
use crate::fspath::FsPath;

/// 1 つの文書の退避用の公開物。`Send + Sync`。
#[derive(Clone, Debug)]
pub struct DocSnapshot {
    pub doc: DocumentId,
    /// O(1) の clone。
    pub snapshot: Snapshot,
    pub format: TextFormat,
    pub revision: Revision,
    /// 保存点がある (= ディスク上に保存済みの版がある)。復旧の案内に使う。
    pub saved_node_exists: bool,
    /// 秘密ファイルのパターンに一致する (パターンの正本は 07。v0.1 は未実装で常に `false`)。
    pub sensitive: bool,
    pub path: Option<FsPath>,
    pub untitled_name: Option<String>,
}
