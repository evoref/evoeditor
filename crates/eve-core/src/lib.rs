//! Editor ファサード、Document / View、Transaction / Undo、コマンド、検索、Job、Frame 生成、Ports の定義 (docs/02 §4.1、docs/03)。
//!
//! 現在は Ports とジョブの型 (docs/02 §4.4 / §4.5 / §9.3) だけがある。残りは docs/08-roadmap.md §2.2 の
//! 「最初の縦切り」の順に入れる。

#![forbid(unsafe_code)]
// FsError は詳細 (Conflict の Stamp など) を持つ大きな型。エラーは稀な経路なので、Box で包まず値で返す。
#![allow(clippy::result_large_err)]

mod cancel;
mod content_hash;
mod document;
mod editor;
mod fs;
mod fspath;
mod interrupted;
mod io_jobs;
mod jobs;
mod leftover;
mod paths;
mod replacing;
mod rescue;
mod secret;
mod time;
mod unmap;

pub use cancel::CancelToken;
mod eol;
mod search;
pub use document::{
    Applied, AppliedTx, ApplyError, DisplaySettings, Document, EolStats, HistoryStep,
    LONG_LINE_THRESHOLD, LONG_LINE_VISIBLE, LoadState, NodeId, Origin, ReadOnlyReason,
    ReadOnlyReasons, RecordOutcome, TextFormat, Transaction, TransactionMeta, UndoBudget,
    UndoGroupHint, UndoHistory, trim_to_total_budget,
};
pub use editor::{
    ArgValue, Args, Binding, CommandCtx, CommandInvocation, CommandKind, CommandRegistry,
    CommandResult, CommandSpec, ContextExpr, CtxValue, DocSnapshot, Editor, EditorState, Handler,
    InitialFile, InvocationSource, KeyParseError, Keymap, Layer, ParseError, Permission,
    RejectReason, Resolution, ScrollPos, SearchCount, SearchStep, View, Window, format_stroke,
    normalize_eol, parse_sequence, parse_stroke,
};
pub use eol::build_eol_conversion;
pub use eve_encoding::Encoding;
pub use eve_view::DocumentId;
pub use fs::*;
pub use fspath::FsPath;
pub use io_jobs::OpenOptions;
pub use jobs::{
    ConfirmAsk, JobError, JobKind, JobPorts, JobRequest, JobResult, JobSpawner, MetaPurpose,
    OpenNotes, Ports, SaveStrategy, run_job,
};
pub use leftover::{Leftover, LeftoverKind, LeftoverOp};
pub use paths::Paths;
pub use replacing::{ReplacingCount, ReplacingGuard};
pub use rescue::{RescueOutcome, RescueReason, RescueWriter};
pub use search::{
    Case, Matcher, QueryError, SearchJob, SearchQuery, build_replace_all, find_next,
    replacement_for, run_search, scan_bytes, scan_lines, unescape_replacement,
};
pub use secret::is_secret_path;
pub use time::{Clock, Millis, Tick};
pub use unmap::{ReplaceCounts, ReplaceTarget};
