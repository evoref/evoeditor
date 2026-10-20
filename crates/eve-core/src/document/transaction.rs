//! `Transaction` と `Origin` (docs/03-core-design.md §6.3)。文書を変える唯一の単位。

use eve_text::{ChangeSet, Selection};
use eve_view::{CommandId, OriginTag, ViewId};

use crate::time::Tick;

/// Transaction の由来。Undo の結合可否・監査・変更通知の識別に使う。**列挙の正本は 03 §6.3**。
/// `Ai` / `Plugin` / `External` は、それぞれの ID 型を持つ版で足す (`#[non_exhaustive]`)。
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Origin {
    /// キー入力による直接編集 (文字入力・Backspace / Delete・単語削除・改行・Tab)。
    User,
    /// その他の編集コマンド (インデント、行操作、置換、変換 …)。
    Command(CommandId),
    /// ブラケットペースト / クリップボード貼り付け。
    Paste,
    /// 外部変更の再読込。
    Reload,
    Lsp,
    /// スワップ・緊急退避からの復旧適用。
    Recovery,
    /// 利用者の選択を受けて core が組み立てる編集。Undo 可、前後と結合しない。
    System,
}

impl Origin {
    /// 外向きの通知 (`EditorEvent::DocumentChanged`) に載せる種別。
    pub fn tag(&self) -> OriginTag {
        match self {
            Origin::User => OriginTag::User,
            Origin::Command(_) => OriginTag::Command,
            Origin::Paste => OriginTag::Paste,
            Origin::Reload => OriginTag::Reload,
            Origin::Lsp => OriginTag::Lsp,
            Origin::Recovery => OriginTag::Recovery,
            Origin::System => OriginTag::System,
        }
    }
}

/// Undo の結合の指示 (03 §7.3)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UndoGroupHint {
    /// 結合規則 (R3〜R9) で決める。
    #[default]
    Auto,
    /// 直前のノードに結合する (1 つの操作が複数の Transaction に分かれるときの 2 つ目以降)。
    Merge,
    /// 必ず新ノードにする。
    Break,
}

/// 拡張メタデータ欄 (06 が provider / model / ハンク一覧等に使う)。v0.1 は空。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct TransactionMeta {}

#[derive(Clone, Debug)]
pub struct Transaction {
    pub changes: ChangeSet,
    /// 発行 View の Selection (Undo 時の復元用)。他 View 由来や Reload では `None`。
    pub selection_before: Option<Selection>,
    /// 適用後に発行 View へ設定する Selection。`None` なら `changes` で写像する。
    pub selection_after: Option<Selection>,
    pub origin: Origin,
    /// Undo / Redo による適用 (Origin は元のまま)。
    pub is_undo: bool,
    pub view: Option<ViewId>,
    /// Editor の「今」 (Host が各メッセージの処理の前に渡した Tick)。コマンドの実行中に Clock を読み直さない。
    pub timestamp: Tick,
    pub undo_group: UndoGroupHint,
    pub meta: TransactionMeta,
}

impl Transaction {
    pub fn new(changes: ChangeSet, origin: Origin, timestamp: Tick) -> Transaction {
        Transaction {
            changes,
            selection_before: None,
            selection_after: None,
            origin,
            is_undo: false,
            view: None,
            timestamp,
            undo_group: UndoGroupHint::Auto,
            meta: TransactionMeta::default(),
        }
    }

    pub fn with_selections(
        mut self,
        before: Option<Selection>,
        after: Option<Selection>,
    ) -> Transaction {
        self.selection_before = before;
        self.selection_after = after;
        self
    }

    pub fn with_view(mut self, view: ViewId) -> Transaction {
        self.view = Some(view);
        self
    }

    pub fn with_undo_group(mut self, hint: UndoGroupHint) -> Transaction {
        self.undo_group = hint;
        self
    }
}
