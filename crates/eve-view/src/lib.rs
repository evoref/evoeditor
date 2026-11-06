//! core と frontend の **境界型** だけを持つ crate (docs/02-architecture.md §4.1、§8.4、§12.2)。外部依存はない (R4)。
//!
//! 境界型は **契約** として扱う (ADR-0030)。現在の段階は C2 (追加のみ、docs/02-architecture.md §8.4)。`CONTRACT_VERSION` は C3 まで 0 に固定する。
//!
//! - enum は `#[non_exhaustive]` (frontend の `match` に既定の腕を強制し、変種の追加を非破壊にする)。
//!   struct への付与は C3 から (§8.4、OQ-177)。
//! - frontend 固有の挙動は `Capabilities` で表し、core に「TUI なら」の分岐を書かない。
//!
//! 持つもの: ID 型・入力・能力・`Effect`・`EditorEvent`・`Query`/`Answer`・`Frame` と意味的 UI モデル。

#![forbid(unsafe_code)]

mod caps;
mod command;
mod effect;
mod events;
mod frame;
mod ids;
mod input;
mod query;
mod term;

pub use caps::{Capabilities, CellSize, ColorDepth};
pub use command::CommandId;
pub use effect::{ClipboardTarget, ClipboardText, Effect, QuitRescue, StderrLine};
pub use events::{EditorEvent, MessageLevel, OriginKind, OriginTag, OverlayKind};
pub use frame::{
    BarFocus, CaseMode, CellPos, CellRect, CellText, Choice, ChoiceId, ChromeTexts, Color,
    CursorPlacement, CursorShape, CutPolicy, Frame, HelpLine, HelpLineKind, HintContext, HintItem,
    HintModel, InputFieldModel, KeyLabel, ListItem, Message, MessageModel, OverlayModel, PaneFrame,
    RenderRow, ReplaceRowModel, RoleStyles, RowSource, SearchBarModel, Span, StatusAlign,
    StatusItem, StatusKind, StatusModel, Style, StyleMods, WhichKeyItem, WhichKeyModel,
};
pub use ids::{
    ClipboardHandleId, ClipboardToken, DocumentId, JobId, Revision, ViewId, ViewRevision, WindowId,
};
pub use input::{
    InputEvent, KeyCode, KeyEvent, KeyKind, KeyStroke, Modifiers, MouseAt, MouseButton, MouseEvent,
    MouseKind,
};
pub use query::{
    Answer, ByteRange, DocumentInfo, FocusTarget, LoadStateView, Query, QueryError,
    ReadOnlyReasonView, SelRange, WindowInfo,
};
pub use term::{RawTermHandle, SavedTermMode, TermRestoredFlag};

/// 契約の版。境界型を変えるごとに上げる (C1 から)。Frame JSON と駆動 API の `initialize` に含める。
/// C3 まで 0 のまま。契約版 1 の固定は TUI 完成後のレビュー (C3)。
pub const CONTRACT_VERSION: u32 = 0;
