//! `EditorEvent`: core が外へ知らせる出来事 (docs/02-architecture.md §10.1)。`Editor::drain_events` で取り出す。
//! v0.1 で発火する変種だけを置く。予約の変種 (`SelectionChanged` など) は最初の利用者の版で足す。

use crate::ids::{DocumentId, Revision, WindowId};

/// メッセージの重大度。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MessageLevel {
    /// 情報 (操作の結果の報告など)。
    Info,
    /// 警告 (操作は続くが、利用者が知るべきこと)。
    Warn,
    /// エラー (操作が失敗した)。
    Error,
}

/// core が外へ知らせる出来事。
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EditorEvent {
    /// 読込が完了した、または無題の文書を作った。
    DocumentOpened {
        /// 開いた文書。
        doc: DocumentId,
    },
    /// Transaction を適用した。
    DocumentChanged {
        /// 変わった文書。
        doc: DocumentId,
        /// 適用した後の Revision。
        revision: Revision,
        /// Transaction の由来。
        origin_kind: OriginKind,
    },
    /// 保存が完了した。`revision` は保存した Snapshot の Revision。
    DocumentSaved {
        /// 保存した文書。
        doc: DocumentId,
        /// 保存した Snapshot の Revision。
        revision: Revision,
        /// 保存先の表示用パス。
        path: String,
    },
    /// 文書を閉じた。以後この `DocumentId` は再利用されない。
    DocumentClosed {
        /// 閉じた文書。
        doc: DocumentId,
    },
    /// メッセージを示した。文言は `Frame` の `MessageModel` にある。
    MessageShown {
        /// 重大度。
        level: MessageLevel,
        /// `MessageKey` の名前 (例 `info.save.done`)。
        key: String,
    },
    /// オーバーレイが開閉した。`None` は閉じた。
    OverlayChanged {
        /// オーバーレイが開閉した Window。
        window: WindowId,
        /// 開いた後の最前面のオーバーレイの種類。`None` は閉じた。
        overlay: Option<OverlayKind>,
    },
}

/// Transaction の由来の種別 (03 §6.3 の `Origin` から中身を除いたもの)。
/// Undo / Redo は `Origin` の変種ではなく、元の Origin + `is_undo`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OriginKind {
    /// 元の Transaction の由来。
    pub origin: OriginTag,
    /// Undo / Redo による適用 (`origin` は元の Transaction のまま。取り消しとやり直しは区別しない)。
    pub is_undo: bool,
}

/// `Origin` の種別。`Origin` に変種が足されたら同時に足す。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OriginTag {
    /// キー入力による直接の編集 (文字入力・Backspace / Delete・改行など)。
    User,
    /// その他の編集コマンド (インデント・行の操作・置換・変換など)。`Origin` が持つ `CommandId` は除く。
    Command,
    /// ブラケットペースト・クリップボードからの貼り付け。
    Paste,
    /// 外部の変更の再読込。
    Reload,
    /// スワップ・緊急退避からの復旧の適用。
    Recovery,
    /// 承認済みの AI の提案の適用 (v1.L。今は発火しない予約)。
    Ai,
    /// プラグインの編集 (v1.X。今は発火しない予約)。
    Plugin,
    /// 駆動 API などの外部のクライアント (今は発火しない予約)。
    External,
    /// LSP の workspace edit (v1.L。今は発火しない予約)。
    Lsp,
    /// 利用者の選択を受けて core が組み立てる編集 (例: 確認の「置換して保存」)。
    System,
}

/// オーバーレイの種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OverlayKind {
    /// リスト選択 (`OverlayModel::List`。コマンドパレットなど)。
    List,
    /// 入力プロンプト (`OverlayModel::Prompt`)。
    Prompt,
    /// 確認ダイアログ (`OverlayModel::Confirm`)。
    Confirm,
    /// ヘルプ画面 (全画面、04 §8.6)。
    Help,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 変更の通知() {
        let ev = EditorEvent::DocumentChanged {
            doc: DocumentId(1),
            revision: Revision(3),
            origin_kind: OriginKind {
                origin: OriginTag::User,
                is_undo: false,
            },
        };
        assert_eq!(ev.clone(), ev);
        assert_ne!(
            ev,
            EditorEvent::DocumentChanged {
                doc: DocumentId(1),
                revision: Revision(3),
                origin_kind: OriginKind {
                    origin: OriginTag::User,
                    is_undo: true
                },
            }
        );
    }
}
