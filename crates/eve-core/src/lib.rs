//! Editor ファサード、Document / View、Transaction / Undo、コマンド、検索、Job、Frame 生成、Ports の定義 (docs/02 §4.1、docs/03)。
//!
//! 入口は `Editor` (`editor`)。文書は `document`、検索は `search`、Ports とジョブの型は `fs`・`jobs`・`paths`・`time` などにある
//! (docs/02 §4.4 / §4.5 / §9.3)。

#![forbid(unsafe_code)]
// 公開 crate は公開面のすべてに文書を付ける (docs/09 §8.1、NFR-705。v0.3 から)。
#![deny(missing_docs)]
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

pub use command::arg_schema::{ArgSpec, ArgType, command_args};
pub use command::clipboard::ClipboardFallbackReason;
pub use editor::builtin_registry;
pub use jobs::cancel::CancelToken;
pub use keymap::{KEYMAP_CONTEXTS, resolve_keymap};
pub use ui_model::palette::hidden as command_hidden;
mod search;
pub use document::eol::build_eol_conversion;
pub use document::{
    Applied, AppliedTx, ApplyError, DisplaySettings, Document, EolStats, HistoryStep,
    LONG_LINE_THRESHOLD, LoadState, NodeId, Origin, PreservedError, PreservedMap, ReadOnlyReason,
    ReadOnlyReasons, RecordOutcome, StdinDoc, TextFormat, Transaction, TransactionMeta, UndoBudget,
    UndoGroupHint, UndoHistory, count_long_lines, trim_to_total_budget,
};
pub use editor::{
    Affinity, ArgValue, Args, Binding, CONFIG_FILE, CONTEXT_VARS, CommandCtx, CommandInvocation,
    CommandKind, CommandRegistry, CommandResult, CommandSpec, ConfigSources, ContextExpr, CtxValue,
    DocSnapshot, Editor, EditorState, Handler, HostReport, InitialFile, InitialTarget,
    InvocationSource, KEYMAP_FILE, KeyParseError, Keymap, LOCALE_CLI_SOURCE, LOCALE_ENV_SOURCE,
    LOCALES_PREFIX, Layer, LocaleFiles, LocaleScan, ParseError, Permission, RejectReason,
    Resolution, ScrollPos, SearchCount, SearchStep, StartupConfig, View, Window, WrapMode,
    WrapSettings, config_diagnostic_kind, config_diagnostic_texts, config_diagnostic_texts_long,
    config_problem_count, format_stroke, normalize_eol, parse_sequence, parse_stroke,
    resolve_theme, scan_locales,
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
pub use io::rescue::{RescueError, RescueOutcome, RescueReason, RescueWriter};
pub use io::secret::is_secret_path;
pub use io::unmap::{ReplaceCounts, ReplaceTarget};
pub use jobs::{
    ConfirmAsk, EscapeMergeApproval, EscapeMergeAt, JobError, JobKind, JobOutput, JobPorts,
    JobRequest, JobResult, JobSpawner, MetaPurpose, OpenNotes, Ports, SaveStrategy, StdinEnd,
    StdinSource, run_job,
};
/// 行の窓の核の大きさと連結の上限を試験から小さくする口 (feature `test-hooks` のときだけ。配布ビルドには無い、03 §12.3.1)。
#[cfg(feature = "test-hooks")]
#[doc(hidden)]
pub use search::SearchTuning;
/// `fuzz_regex_search` の本体 (docs/09 §3.3)。cargo-fuzz のターゲットと、CI の proptest が同じ関数を使う。
#[doc(hidden)]
pub use search::fuzzing as search_fuzzing;
pub use search::{
    Case, FindJob, LineSpan, Matcher, QueryError, QueryPlan, ReplaceAllBuilt,
    SEARCH_CONCAT_MAX_BYTES, SEARCH_WINDOW_CORE_BYTES, ScanCtx, ScanEnd, ScanLimit, SearchJob,
    SearchQuery, build_replace_all, build_replace_all_in, find_next, replace_one, replacement_for,
    run_search, scan_bytes, scan_range, unescape_replacement,
};
pub use time::{Clock, Millis, Tick};
