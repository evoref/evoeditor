//! `Query` / `Answer`: 読み取り専用の問い合わせ (docs/02-architecture.md §10.1)。副作用がない。テストのハーネスと
//! (後の版の) 駆動 API が、状態を取り出す唯一の手段 (01 NFR-707 AC-4)。
//!
//! 予約の変種 (`Commands` / `KeymapReverse`) は、最初の利用者の版で足す。

use crate::caps::Capabilities;
use crate::frame::{OverlayModel, StatusModel};
use crate::ids::{DocumentId, Revision, ViewId, WindowId};

/// 位置の範囲 `[start, end)`。UTF-8 のバイトオフセット (ADR-0010)。
/// eve-text の同名の型 (`ByteOff` (usize) の組) とは別の型で、変換は eve-core だけが行う (02 §12.2)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ByteRange {
    pub start: u64,
    pub end: u64,
}

impl ByteRange {
    pub fn new(start: u64, end: u64) -> ByteRange {
        ByteRange { start, end }
    }

    /// バイト数。逆順の範囲は 0。
    pub fn len(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }
}

/// 問い合わせ。
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Query {
    /// 文書一覧 (各文書の `ViewId` を含む)。
    Documents,
    /// Window の一覧。宛先の ID を得る手段。
    Windows,
    /// 文書の本文。`range` が `None` なら全文。同じ Revision・同じ範囲なら同じバイト列を返す (決定性)。
    DocumentText {
        doc: DocumentId,
        range: Option<ByteRange>,
    },
    /// View の Selection。
    Selection { view: ViewId },
    /// frontend の能力。
    Capabilities,
    /// Window のステータスバーのモデル。
    Status { window: WindowId },
    /// Window の最前面のオーバーレイ (なければ `None`)。
    Overlay { window: WindowId },
}

/// 問い合わせの応答。
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Answer {
    Documents(Vec<DocumentInfo>),
    Windows(Vec<WindowInfo>),
    DocumentText {
        doc: DocumentId,
        revision: Revision,
        range: ByteRange,
        /// 改行は文書のまま (非正規化)、不正バイトは PUA エスケープのまま。
        text: String,
    },
    Selection {
        view: ViewId,
        doc: DocumentId,
        revision: Revision,
        ranges: Vec<SelRange>,
        /// `ranges` の中の主 Range の添字。
        primary: usize,
    },
    Capabilities(Capabilities),
    Status(StatusModel),
    Overlay(Option<OverlayModel>),
}

/// Selection の Range 1 つ (バイトオフセット)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SelRange {
    pub anchor: u64,
    pub head: u64,
}

/// 問い合わせのエラー。
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum QueryError {
    UnknownDocument(DocumentId),
    UnknownView(ViewId),
    UnknownWindow(WindowId),
    /// 読込が完了していない (`DocumentText` / `Selection`)。
    NotLoaded(DocumentId),
    /// 範囲が文書の外、または UTF-8 の境界でない。
    RangeOutOfBounds {
        len: u64,
    },
    /// 予約の変種 (この版では応答しない)。
    Unsupported,
}

/// 文書の情報。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentInfo {
    pub doc: DocumentId,
    /// 表示用のパス (05 §3.3)。
    pub path: Option<String>,
    pub untitled_name: Option<String>,
    pub revision: Revision,
    pub dirty: bool,
    pub load_state: LoadStateView,
    /// 読み取り専用の理由 (eve-core の `ReadOnlyReason` の変種名を PascalCase のまま、優先順に)。
    pub read_only: Vec<String>,
    /// この文書を表示している View (v0.1 は 1 つ)。
    pub views: Vec<ViewId>,
}

/// 文書の読込の状態 (eve-core の `load_state` を写したもの)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoadStateView {
    Loading,
    Ready,
    Failed,
}

/// Window の情報。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowInfo {
    pub window: WindowId,
    pub views: Vec<ViewId>,
    pub focus: FocusTarget,
}

/// 入力の行き先。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FocusTarget {
    /// 編集面 (View)。
    Pane(ViewId),
    /// オーバーレイ (`overlays` の添字)。
    Overlay(usize),
    /// 検索バー。
    SearchBar,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 範囲の長さ() {
        assert_eq!(ByteRange::new(2, 5).len(), 3);
        assert!(ByteRange::new(5, 5).is_empty());
        assert_eq!(ByteRange::new(5, 2).len(), 0, "逆順でもパニックしない");
    }

    #[test]
    fn 問い合わせと応答() {
        let q = Query::DocumentText {
            doc: DocumentId(1),
            range: None,
        };
        assert_eq!(q.clone(), q);
        let a = Answer::Selection {
            view: ViewId(1),
            doc: DocumentId(1),
            revision: Revision(0),
            ranges: vec![SelRange { anchor: 0, head: 0 }],
            primary: 0,
        };
        assert_eq!(a.clone(), a);
        assert_eq!(
            QueryError::RangeOutOfBounds { len: 3 },
            QueryError::RangeOutOfBounds { len: 3 }
        );
    }
}
