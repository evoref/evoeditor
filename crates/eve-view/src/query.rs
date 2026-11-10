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
    /// 先頭 (含む)。
    pub start: u64,
    /// 末尾 (含まない)。
    pub end: u64,
}

impl ByteRange {
    /// `[start, end)` を作る。順序と範囲は検査しない (逆順の範囲は空として扱われる)。
    pub fn new(start: u64, end: u64) -> ByteRange {
        ByteRange { start, end }
    }

    /// バイト数。逆順の範囲は 0。
    pub fn len(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }

    /// 空の範囲か (`start >= end`。逆順も空)。
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
        /// 対象の文書。
        doc: DocumentId,
        /// 取り出す範囲 (UTF-8 の境界)。
        range: Option<ByteRange>,
    },
    /// View の Selection。
    Selection {
        /// 対象の View。
        view: ViewId,
    },
    /// core が今持っている frontend の能力。
    Capabilities,
    /// Window のステータスバーのモデル。
    Status {
        /// 対象の Window。
        window: WindowId,
    },
    /// Window の最前面のオーバーレイ (なければ `None`)。
    Overlay {
        /// 対象の Window。
        window: WindowId,
    },
}

/// 問い合わせの応答。
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Answer {
    /// `Query::Documents` の応答 (開いているすべての文書)。
    Documents(Vec<DocumentInfo>),
    /// `Query::Windows` の応答。
    Windows(Vec<WindowInfo>),
    /// `Query::DocumentText` の応答。
    DocumentText {
        /// 対象の文書。
        doc: DocumentId,
        /// 本文を取り出した時点の Revision。
        revision: Revision,
        /// 取り出した範囲 (全文なら `[0, 文書のバイト数)`)。
        range: ByteRange,
        /// 改行は文書のまま (非正規化)、不正バイトは PUA エスケープのまま。
        text: String,
    },
    /// `Query::Selection` の応答。
    Selection {
        /// 対象の View。
        view: ViewId,
        /// View が表示している文書。
        doc: DocumentId,
        /// `ranges` の位置が指す Revision。
        revision: Revision,
        /// Selection の Range (1 つ以上)。
        ranges: Vec<SelRange>,
        /// `ranges` の中の主 Range の添字。
        primary: usize,
    },
    /// `Query::Capabilities` の応答。
    Capabilities(Capabilities),
    /// `Query::Status` の応答。
    Status(StatusModel),
    /// `Query::Overlay` の応答。`None` はオーバーレイが無い。
    Overlay(Option<OverlayModel>),
}

/// Selection の Range 1 つ (バイトオフセット)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SelRange {
    /// 選択を始めた端 (動かない側)。
    pub anchor: u64,
    /// カーソルのある端 (動く側)。`anchor == head` なら選択なしのカーソル。
    pub head: u64,
}

/// 問い合わせのエラー。
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum QueryError {
    /// その ID の文書が無い (閉じた、または作られていない)。
    UnknownDocument(DocumentId),
    /// その ID の View が無い。
    UnknownView(ViewId),
    /// その ID の Window が無い。
    UnknownWindow(WindowId),
    /// 読込が完了していない (`LoadStateView` が `Ready` でない。`DocumentText` / `Selection`)。
    NotLoaded(DocumentId),
    /// 範囲が文書の外、または UTF-8 の境界でない。
    RangeOutOfBounds {
        /// 文書の今のバイト数。
        len: u64,
    },
    /// 予約の変種 (この版では応答しない)。
    Unsupported,
}

/// 文書の情報。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentInfo {
    /// 文書の ID。
    pub doc: DocumentId,
    /// 表示用のパス (05 §3.3)。無題の文書は `None`。
    pub path: Option<String>,
    /// 無題の文書の表示名 (`無題-1` など)。パスのある文書は `None`。
    pub untitled_name: Option<String>,
    /// 今の Revision。
    pub revision: Revision,
    /// 最後の保存 (または読込) の後に変更がある。
    pub dirty: bool,
    /// 読込の状態。
    pub load_state: LoadStateView,
    /// 読み取り専用の理由 (優先順。空なら理由なし)。読込中かどうかは `load_state` で見る。
    pub read_only: Vec<ReadOnlyReasonView>,
    /// この文書を表示している View (v0.1 は 1 つ)。
    pub views: Vec<ViewId>,
}

/// 文書の読込の状態 (eve-core の `load_state` を写したもの)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoadStateView {
    /// 読込中。`Query::DocumentText` / `Query::Selection` は `QueryError::NotLoaded` になる。
    Loading,
    /// 読込が完了した (無題の文書も含む)。本文と Selection を問い合わせられるのはこの状態だけ。
    Ready,
    /// 読込に失敗した。`Loading` と同じく本文と Selection は問い合わせられない。
    Failed,
}

/// 読み取り専用の理由 (eve-core の `ReadOnlyReason` (03 §9.6) を写したもの。宣言順が表示の優先順)。
/// eve-view は eve-core に依存しないので、同じ変種を持つ別の型にし、変換は eve-core が行う (02 §12.2)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ReadOnlyReasonView {
    /// ファイルの読み取り専用属性 (`file.toggle_read_only` で解除できる)。
    FileAttribute,
    /// 利用者の指定 (`--readonly`・`file.toggle_read_only`)。
    UserRequested,
    /// 書込み権限が無い。
    NoWritePermission,
    /// 不正なバイトを含む。
    InvalidBytes,
    /// 状態を持つ文字コード (ISO-2022-JP など) で、不正なバイトを含む。
    StatefulEncodingWithInvalidBytes,
    /// バイナリとして開いた。
    Binary,
    /// 巨大ファイルの閲覧モード。
    LargeFileViewer,
    /// 読込を最後まで終えていない。
    LoadIncomplete,
}

/// Window の情報。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowInfo {
    /// Window の ID。
    pub window: WindowId,
    /// この Window が表示している View (v0.1 は 1 つ)。
    pub views: Vec<ViewId>,
    /// 今の入力の行き先。
    pub focus: FocusTarget,
}

/// 入力の行き先。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FocusTarget {
    /// 編集面 (View)。
    Pane(ViewId),
    /// オーバーレイ (`Frame.overlays` の添字)。
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
