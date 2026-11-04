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

impl DocSnapshot {
    /// 本文のバイト列 (内部表現のまま。テスト・小さな文書向け。大きな文書は `snapshot.chunks` で流す)。
    pub fn to_vec(&self) -> Vec<u8> {
        use eve_text::TextRead;
        let mut out = Vec::with_capacity(self.snapshot.len_bytes());
        for c in self
            .snapshot
            .chunks(eve_text::ByteRange::all(self.snapshot.len_bytes()))
        {
            out.extend_from_slice(c.as_bytes());
        }
        out
    }
}

impl super::state::EditorState {
    /// 文書の公開物を作る。`dirty_only` なら dirty でない文書は `None` (緊急退避)。読込中・閉じた文書は常に `None`。
    /// 「退避先に保存」は保存に失敗した文書 (dirty でないこともある) を `dirty_only = false` で作る。
    pub(crate) fn rescue_snapshot(&self, doc: DocumentId, dirty_only: bool) -> Option<DocSnapshot> {
        let Some(d) = self.document(doc) else {
            // 閉じたが保存ジョブが実行中の文書は、保存の結果が届くまで公開物を残す
            return self.closing_snapshots.get(&doc).cloned();
        };
        if (dirty_only && !d.is_dirty()) || d.load_state != crate::document::LoadState::Ready {
            return None;
        }
        Some(DocSnapshot {
            doc,
            snapshot: d.snapshot(),
            format: d.format.clone(),
            revision: d.revision(),
            saved_node_exists: d.history().saved().is_some(),
            // 秘密ファイルパターン (docs/07 §5.12.2) に一致する文書。退避の manifest の `sensitive` と、終了時の削除の案内になる
            sensitive: d.path.as_ref().is_some_and(crate::is_secret_path),
            path: d.path.clone(),
            untitled_name: d.path.is_none().then(|| {
                self.catalog()
                    .render("status.untitled", &[("n", "1")], &|_| None)
            }),
        })
    }
}
