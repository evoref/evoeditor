//! 退避用の公開物 (`DocSnapshot`、docs/02 §5.4 / §11.2.1)。Host が変更のあった文書ごとにスロットへ公開し、
//! 緊急退避の書き手が Editor に触れずに読む。

use eve_text::Snapshot;
use eve_view::{DocumentId, Revision};

use crate::document::TextFormat;
use crate::io::fspath::FsPath;

/// 1 つの文書の退避用の公開物。`Send + Sync`。
#[derive(Clone, Debug)]
pub struct DocSnapshot {
    /// 文書の ID。
    pub doc: DocumentId,
    /// O(1) の clone。
    pub snapshot: Snapshot,
    /// 文書の形式 (退避の manifest に書く文字コード・BOM・改行)。
    pub format: TextFormat,
    /// 公開した時点の Revision。
    pub revision: Revision,
    /// 保存点がある (= ディスク上に保存済みの版がある)。復旧の案内に使う。
    pub saved_node_exists: bool,
    /// 秘密ファイルのパターンに一致する (パターンの正本は 07。`is_secret_path` で判定する)、または標準入力から読んだ文書
    /// (03 §9.2.2)。
    pub sensitive: bool,
    /// ファイルのパス。無題の文書は `None`。
    pub path: Option<FsPath>,
    /// 無題の文書の表示名。パスのある文書は `None`。
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

/// 公開物を作る文書の範囲 (`EditorState::rescue_snapshot`)。読込中・閉じた文書はどの範囲でも作らない。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotScope {
    /// 緊急退避 (docs/03 §10.1 の「対象」): dirty な文書と、dirty でなくても内容がほかに残っていない文書
    /// (`Document::stdin_unsaved`。`--readonly` で開いた標準入力の文書。03 §9.2.2 の「スワップと緊急退避」)。
    Rescue,
    /// スワップ (03 §10.2): dirty な文書だけ。
    Dirty,
    /// 「退避先に保存」: 保存に失敗した文書 (dirty でないこともある)。
    Any,
}

impl crate::editor::state::EditorState {
    /// 文書の公開物を作る。`scope` に入らない文書は `None`。読込中・閉じた文書は常に `None`。
    pub(crate) fn rescue_snapshot(
        &self,
        doc: DocumentId,
        scope: SnapshotScope,
    ) -> Option<DocSnapshot> {
        let Some(d) = self.document(doc) else {
            // 閉じたが保存ジョブが実行中の文書は、保存の結果が届くまで公開物を残す
            return self.io.closing_snapshots.get(&doc).cloned();
        };
        let in_scope = match scope {
            SnapshotScope::Rescue => {
                d.is_dirty() || (d.stdin_unsaved() && !self.io.rescue_dirty_only)
            }
            SnapshotScope::Dirty => d.is_dirty(),
            SnapshotScope::Any => true,
        };
        if !in_scope || d.load_state != crate::document::LoadState::Ready {
            return None;
        }
        Some(DocSnapshot {
            doc,
            snapshot: d.snapshot(),
            format: d.format.clone(),
            revision: d.revision(),
            saved_node_exists: d.history().saved().is_some(),
            // 秘密ファイルパターン (docs/07 §5.12.2) に一致する文書と、標準入力から読んだ文書 (常に。03 §9.2.2 の
            // 「スワップと緊急退避」、OQ-219 の 5)。退避の manifest の `sensitive` と、終了時の削除の案内になる
            sensitive: d.stdin.is_some() || d.path.as_ref().is_some_and(crate::is_secret_path),
            path: d.path.clone(),
            untitled_name: d.path.is_none().then(|| self.untitled_label(d)),
        })
    }
}
