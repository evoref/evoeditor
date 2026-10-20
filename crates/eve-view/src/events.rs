//! `EditorEvent`: core が外へ知らせる出来事 (docs/02-architecture.md §10.1)。`Editor::drain_events` で取り出す。
//! v0.1 で発火する変種だけを置く。予約の変種 (`SelectionChanged` など) は最初の利用者の版で足す。

use crate::ids::{DocumentId, Revision, WindowId};

/// メッセージの重大度。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MessageLevel {
    Info,
    Warn,
    Error,
}

/// core が外へ知らせる出来事。
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EditorEvent {
    /// 読込が完了した、または無題の文書を作った。
    DocumentOpened {
        doc: DocumentId,
    },
    /// Transaction を適用した。
    DocumentChanged {
        doc: DocumentId,
        revision: Revision,
        origin_kind: OriginKind,
    },
    /// 保存が完了した。`revision` は保存した Snapshot の Revision。
    DocumentSaved {
        doc: DocumentId,
        revision: Revision,
        /// 保存先の表示用パス。
        path: String,
    },
    DocumentClosed {
        doc: DocumentId,
    },
    /// メッセージを示した。`key` は MessageKey の名前 (例 `info.save.done`)。文言は Frame の MessageModel にある。
    MessageShown {
        level: MessageLevel,
        key: String,
    },
    /// オーバーレイが開閉した。`None` は閉じた。
    OverlayChanged {
        window: WindowId,
        overlay: Option<OverlayKind>,
    },
}

/// Transaction の由来の種別 (03 §6.3 の `Origin` から中身を除いたもの)。
/// Undo / Redo は `Origin` の変種ではなく、元の Origin + `is_undo`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OriginKind {
    pub origin: OriginTag,
    pub is_undo: bool,
}

/// `Origin` の種別。`Origin` に変種が足されたら同時に足す。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OriginTag {
    User,
    Command,
    Paste,
    Reload,
    Recovery,
    Ai,
    Plugin,
    External,
    Lsp,
    System,
}

/// オーバーレイの種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OverlayKind {
    List,
    Prompt,
    Confirm,
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
