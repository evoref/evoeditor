//! Editor ファサード、Document / View、Transaction / Undo、コマンド、検索、Job、Frame 生成、Ports の定義 (docs/02 §4.1、docs/03)。
//!
//! 入口は `Editor` (`editor`)。文書は `document`、検索は `search`、Ports とジョブの型は `fs`・`jobs`・`paths`・`time` などにある
//! (docs/02 §4.4 / §4.5 / §9.3)。

#![forbid(unsafe_code)]
// FsError は詳細 (Conflict の Stamp など) を持つ大きな型。エラーは稀な経路なので、Box で包まず値で返す。
#![allow(clippy::result_large_err)]

mod command;
mod document;
mod editor;
mod io;
mod jobs;
mod keymap;
mod layout;
mod time;
mod ui_model;
mod view;

pub use command::clipboard::ClipboardFallbackReason;
pub use jobs::cancel::CancelToken;
mod search;
pub use document::eol::build_eol_conversion;
pub use document::{
    Applied, AppliedTx, ApplyError, DisplaySettings, Document, EolStats, HistoryStep,
    LONG_LINE_THRESHOLD, LoadState, NodeId, Origin, ReadOnlyReason, ReadOnlyReasons, RecordOutcome,
    TextFormat, Transaction, TransactionMeta, UndoBudget, UndoGroupHint, UndoHistory,
    count_long_lines, trim_to_total_budget,
};
pub use editor::{
    Affinity, ArgValue, Args, Binding, CONFIG_FILE, CONTEXT_VARS, CommandCtx, CommandInvocation,
    CommandKind, CommandRegistry, CommandResult, CommandSpec, ConfigSources, ContextExpr, CtxValue,
    DocSnapshot, Editor, EditorState, Handler, HostReport, InitialFile, InvocationSource,
    KEYMAP_FILE, KeyParseError, Keymap, Layer, ParseError, Permission, RejectReason, Resolution,
    ScrollPos, SearchCount, SearchStep, StartupConfig, View, Window, WrapMode, WrapSettings,
    config_diagnostic_texts, format_stroke, normalize_eol, parse_sequence, parse_stroke,
    resolve_theme,
};
pub use eve_encoding::Encoding;
pub use eve_view::DocumentId;
pub use io::fs::*;
pub use io::fspath::FsPath;
pub use io::io_jobs::OpenOptions;
pub use io::last_version::{LAST_VERSION_FILE, LastVersionOutcome};
pub use io::leftover::{Leftover, LeftoverKind, LeftoverOp};
pub use io::paths::Paths;
pub use io::replacing::{ReplacingCount, ReplacingGuard};
pub use io::rescue::{RescueOutcome, RescueReason, RescueWriter};
pub use io::secret::is_secret_path;
pub use io::unmap::{ReplaceCounts, ReplaceTarget};
pub use jobs::{
    ConfirmAsk, JobError, JobKind, JobPorts, JobRequest, JobResult, JobSpawner, MetaPurpose,
    OpenNotes, Ports, SaveStrategy, run_job,
};
pub use search::{
    Case, Matcher, QueryError, SearchJob, SearchQuery, build_replace_all, find_next,
    replacement_for, run_search, scan_bytes, unescape_replacement,
};
pub use time::{Clock, Millis, Tick};
