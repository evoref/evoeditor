//! 横断検索 (docs/03 §12.7.5、04 §8.12、01 FR-405。v0.5 の M31 の段 7) の主スレッドの側: 一覧のオーバーレイ
//! (`overlay.active == "workspace_search"`) の状態とモデル、検索のジョブの投入と結果の採否、結果へのジャンプ。
//!
//! - **検索を始める時機**: 入力欄・トグルが変わるたびに、走っている検索を取り消し、出している結果を `bg` で手放してから
//!   新しい検索を始める (待ち時間を置かない)。結果は世代で照らし、古い世代の束は主スレッドで手放さずに `bg` に渡す。
//! - **結果を保つ**: 閉じても (ジャンプ・`Esc`) 検索語・トグル・結果・選択を [`WsSearchStore`] に持ち、次に開いたときに
//!   そのまま出す (探し直さない)。閉じたときに走っていた検索は取り消し、次に開いたときに同じ検索語で探し直す。
//! - **ジャンプ**: オーバーレイを閉じて結果のファイルを開き (開いていればその文書へ切り替え)、読込が終わった時点でその
//!   文書がまだ開いた時点のペインに出ていれば、一致の位置へ移る (`WsJump`)。
//!
//! spec: 03 §12.7.5, 03 §12.7.6, 04 §8.12, 07 §7.1, 07 §11.6, 01 FR-405

use std::ops::Range;

use eve_text::{ByteOff, LineIdx, Selection, TextRead, grapheme_indices, is_grapheme_boundary};
use eve_view::{
    CutPolicy, DocumentId, InputFieldModel, JobId, ListItem, MessageLevel, OverlayModel,
};

use super::finder::map_ranges;
use super::{OpenDocBody, OpenDocRef, WS_MAX_RESULT_LINES, WsHit, WsProgress};
use crate::command::{CommandCtx, CommandKind, CommandRegistry, CommandResult, Permission, ids};
use crate::document::{LoadState, ReadOnlyReason};
use crate::editor::state::EditorState;
use crate::editor::window::PaneRef;
use crate::io::error_keys::{fs_reason, reason_label};
use crate::io::fspath::FsPath;
use crate::io::io_jobs::OpenOptions;
use crate::io::pending::Pending;
use crate::io::same_file::canonical_key;
use crate::jobs::{JobError, JobKind, JobResult};
use crate::layout::{cell_text, cell_text_indexed};
use crate::search::{Case, QueryError, ScanLimit, SearchQuery};
use crate::ui_model::keys;
use crate::ui_model::overlay::{Overlay, field_selection};
use crate::ui_model::palette::WINDOW;
use crate::ui_model::search_bar::{invalid_regex_reason, limit_message};

/// 編集面の選択を初期値にする上限 (バイト。1 行の中で、これ以下のときだけ、03 §12.7.5 の「開く」)。
pub(crate) const WS_SELECTION_QUERY_MAX: usize = 256;

/// 検索の状態。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Status {
    /// 検索語が空 (探さない。結果は空)。
    #[default]
    Empty,
    /// ルートの検出を待っている (`ui.finder.footer_waiting`)。
    WaitingRoot,
    /// ルートの検出が失敗した (`ui.finder.footer_no_root`)。
    NoRoot,
    /// 正規表現の誤り (件数の位置に検索バーと同じ文言。探さない)。
    Invalid(QueryError),
    /// どのファイルでも探さない検索語 (上限の無い重い検索語、03 §12.3.1)。
    Refused(ScanLimit),
    /// 検索中。
    Running { job: JobId },
    /// 終わった。
    Done { truncated: bool },
    /// 失敗した (ルートを読めない・ジョブのパニック)。届いていた結果は残す。
    Failed,
    /// 閉じたときに取り消した (次に開いたときに探し直す)。
    Interrupted,
}

/// 横断検索のオーバーレイの状態 (`Overlay::WorkspaceSearch`)。閉じている間は [`WsSearchStore`] が持つ。
#[derive(Clone, Debug)]
pub(crate) struct WsSearchOverlay {
    pub text: String,
    /// `text` 内のカーソル (バイト。書記素境界)。
    pub cursor: usize,
    /// 欄内全選択 (`select.all`) の最中。
    pub all_selected: bool,
    case: Case,
    regex: bool,
    whole_word: bool,
    /// 今の結果を探した検索の条件 (コンパイルの誤りを `known_error` で見る)。
    query: Option<SearchQuery>,
    /// 届いた結果 (今の世代のもの)。
    hits: Vec<WsHit>,
    progress: WsProgress,
    status: Status,
    /// 選択中の行。
    selected: usize,
    /// 今の検索の世代。
    generation: u64,
}

impl Default for WsSearchOverlay {
    fn default() -> WsSearchOverlay {
        WsSearchOverlay {
            text: String::new(),
            cursor: 0,
            all_selected: false,
            case: Case::Smart,
            regex: false,
            whole_word: false,
            query: None,
            hits: Vec::new(),
            progress: WsProgress::default(),
            status: Status::Empty,
            selected: 0,
            generation: 0,
        }
    }
}

/// 横断検索の閉じている間の状態と世代 (`WorkspaceState` が 1 つ持つ)。
#[derive(Debug, Default)]
pub(crate) struct WsSearchStore {
    /// 閉じたときの状態 (開いている間は `None`)。
    saved: Option<Box<WsSearchOverlay>>,
    /// 次の検索の世代。
    next_generation: u64,
}

/// 横断検索の結果へのジャンプ (読込が終わった時点で一致の位置へ移る、03 §12.7.5 の「ジャンプ」)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WsJump {
    /// 開いた時点のペイン (読込の後にこのペインに出ていなければ移らない)。
    pane: Option<PaneRef>,
    /// 行 (0 始まり)。
    line: u32,
    /// 最初の一致 (行の中のバイト位置)。
    range: Range<u32>,
}

/// トグルの種類 (`search.toggle_case` / `_regex` / `_word`)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WsToggle {
    Case,
    Regex,
    Word,
}

/// 文言を組む関数 (キーと引数から文字列)。
type Render<'a> = dyn Fn(&str, &[(&str, &str)]) -> String + 'a;

/// 選択中の行 selected が入る表示の窓の先頭の行。
fn window_first(selected: usize) -> usize {
    if selected < WINDOW {
        0
    } else {
        selected + 1 - WINDOW
    }
}

/// `label` の中で、引数 `text` を入れた位置 (バイト)。`empty` は同じ文言を `text` を空にして組んだもの。言語ファイルの
/// 文言で `{text}` の位置が変わっても強調を正しい位置に付ける。見つからなければ `None` (強調しない)。
fn arg_position(label: &str, empty: &str, text: &str) -> Option<usize> {
    if text.is_empty() || label.len() != empty.len() + text.len() {
        return None;
    }
    // 前の共通部分の長さ `p` と後ろの共通部分の長さ `s` から、入れた位置 `k` の範囲は `empty.len() - s ..= p` に限られる。
    // 範囲に 2 つ以上あり得る (抜粋が文言の前後の文字と同じ文字で始まる・終わる) ときは、後ろのもの (同梱の文言は `{text}`
    // が末尾) を採る
    let (l, e) = (label.as_bytes(), empty.as_bytes());
    let p = l.iter().zip(e).take_while(|(a, b)| a == b).count();
    let s = l
        .iter()
        .rev()
        .zip(e.iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let lo = empty.len().saturating_sub(s);
    let hi = p.min(empty.len());
    (lo..=hi).rev().find(|&k| {
        empty.is_char_boundary(k)
            && label.get(k..k + text.len()) == Some(text)
            && label.get(..k) == empty.get(..k)
            && label.get(k + text.len()..) == empty.get(k..)
    })
}

/// 抜粋の中の一致を、`label` の書記素の番号の区間にする (`text_at` は抜粋を入れた位置)。抜粋の外に出る部分は切り落とす。
fn hit_grapheme_ranges(label: &str, text_at: usize, h: &WsHit) -> Vec<Range<u32>> {
    let len = h.excerpt.len();
    let graphemes = grapheme_indices(label);
    let mut out: Vec<Range<u32>> = Vec::new();
    for r in &h.ranges {
        let a = (r.start.saturating_sub(h.excerpt_start) as usize).min(len);
        let b = (r.end.saturating_sub(h.excerpt_start) as usize).min(len);
        if a >= b {
            continue;
        }
        let (a, b) = (text_at + a, text_at + b);
        let mut first: Option<u32> = None;
        let mut last = 0u32;
        for (i, (start, g)) in graphemes.iter().enumerate() {
            let end = start + g.len();
            if *start < b && end > a {
                let i = u32::try_from(i).unwrap_or(u32::MAX);
                first.get_or_insert(i);
                last = i + 1;
            }
        }
        if let Some(f) = first {
            out.push(f..last);
        }
    }
    out
}

/// 見出しのトグルの記号 (04 §8.12。検索バーと同じ `[Aa~]` / `[Aa✓]` / `[Aa]`・`[.*]`・`[ab]`。ASCII の端末は `✓` を `*`)。
fn toggle_marks(case: Case, regex: bool, word: bool, ascii: bool) -> String {
    let on = if ascii { "*" } else { "\u{2713}" };
    let case = match case {
        Case::Smart => "~",
        Case::Sensitive => on,
        Case::Insensitive => "",
    };
    let regex = if regex { on } else { "" };
    let word = if word { on } else { "" };
    format!("[Aa{case}] [.*{regex}] [ab{word}]")
}

/// `p` (比べる形の canonical のパス) がルート `root` の下にあるか。要素ごとに比べる (区切りの書き方に依らない。Windows・
/// macOS は ASCII の大文字小文字を畳む)。
fn under_root(root: &std::path::Path, p: &std::path::Path) -> bool {
    let fold = cfg!(any(windows, target_os = "macos"));
    let mut ps = p.components();
    for r in root.components() {
        let Some(c) = ps.next() else {
            return false;
        };
        let (a, b) = (
            r.as_os_str().as_encoded_bytes(),
            c.as_os_str().as_encoded_bytes(),
        );
        if !(a == b || (fold && a.eq_ignore_ascii_case(b))) {
            return false;
        }
    }
    ps.next().is_some()
}

fn search_workspace(ctx: &mut CommandCtx<'_>) -> CommandResult {
    let query = ctx.args.str("query").map(str::to_owned);
    ctx.state.open_workspace_search(query)
}

/// `search.workspace` (07 §11.6)。
pub(crate) fn register(r: &mut CommandRegistry) {
    r.add(
        ids::SEARCH_WORKSPACE,
        CommandKind::Ui,
        Permission::Ui,
        "",
        search_workspace,
    );
}

impl EditorState {
    /// `search.workspace`: 横断検索のオーバーレイを開く (03 §12.7.5 の「開く」)。入力欄の初期値は、引数 `query`、検索バーが
    /// 開いていて検索語が空でなければその検索語 (トグルも引き継ぐ)、編集面の選択が 1 行の中の 256 バイト以下ならその文字列、それ以外は前回の
    /// 検索語。前回と同じ検索語 (検索バーから開いたときはトグルも同じ) なら、前回の結果・選択をそのまま出す。
    pub(crate) fn open_workspace_search(&mut self, query: Option<String>) -> CommandResult {
        let bar = self
            .find
            .search_bar
            .as_ref()
            .filter(|b| !b.text.is_empty())
            .map(|b| (b.text.clone(), b.case, b.regex, b.whole_word));
        let saved = self.workspace.search.saved.take();
        let (text, toggles) = if let Some(q) = query {
            (q, None)
        } else if let Some((t, case, regex, word)) = bar {
            (t, Some((case, regex, word)))
        } else if let Some(sel) = self.selection_query() {
            (sel, None)
        } else {
            (
                saved.as_ref().map(|s| s.text.clone()).unwrap_or_default(),
                None,
            )
        };
        let restore = saved.as_ref().is_some_and(|s| {
            s.text == text && toggles.is_none_or(|t| t == (s.case, s.regex, s.whole_word))
        });
        let mut state = match saved {
            Some(s) if restore => s,
            other => {
                if let Some(old) = other {
                    self.release_hits(old.hits);
                }
                let (case, regex, whole_word) = toggles.unwrap_or_else(|| {
                    (
                        match self.config().search_case {
                            eve_config::SearchCase::Smart => Case::Smart,
                            eve_config::SearchCase::Sensitive => Case::Sensitive,
                            eve_config::SearchCase::Insensitive => Case::Insensitive,
                        },
                        self.config().regex_default,
                        false,
                    )
                });
                Box::new(WsSearchOverlay {
                    case,
                    regex,
                    whole_word,
                    status: Status::Interrupted,
                    ..WsSearchOverlay::default()
                })
            }
        };
        state.cursor = text.len();
        state.text = text;
        state.all_selected = false;
        // 前回の結果をそのまま出すのは、検索が終わっていた (失敗を含む) か探さない状態のときだけ。取り消した・ルートを
        // 待っていた検索は探し直す
        let start = matches!(
            state.status,
            Status::Interrupted | Status::WaitingRoot | Status::NoRoot
        );
        self.set_overlay(Some(Overlay::WorkspaceSearch(state)));
        if start {
            self.ws_search_start();
        }
        self.mark_dirty_render();
        CommandResult::Done
    }

    /// 編集面の主の選択が 1 行の中の 256 バイト以下なら、その文字列 (横断検索の初期値)。
    fn selection_query(&self) -> Option<String> {
        let view = self.focused_view()?;
        let v = self.view(view)?;
        let r = v.selection.primary();
        if r.is_empty() || r.end().0 - r.start().0 > WS_SELECTION_QUERY_MAX {
            return None;
        }
        let d = self.docs.documents.get(&v.doc)?;
        let s = d
            .text()
            .slice_to_string(eve_text::ByteRange::new(r.start(), r.end()));
        (!s.contains(['\n', '\r'])).then_some(s)
    }

    /// 今の入力欄とトグルで検索を始め直す (03 §12.7.5 の「検索を始める時機」)。走っている検索を取り消し、出している結果を
    /// `bg` で手放す。検索語が空・誤り・ルートが無いときは探さない。
    pub(crate) fn ws_search_start(&mut self) {
        let generation = self.workspace.search.next_generation;
        self.workspace.search.next_generation += 1;
        let root = self.workspace().map(|w| w.root().clone());
        let root_failed = self.workspace.root_failed();
        let tuning = self.find.tuning;
        let sugar = self.config().newline_sugar;
        let fold = self.config().cp932_compat_fold;
        let Some(Overlay::WorkspaceSearch(w)) = &mut self.ui.overlay else {
            return;
        };
        let old_job = match w.status {
            Status::Running { job } => Some(job),
            _ => None,
        };
        let old = std::mem::take(&mut w.hits);
        w.progress = WsProgress::default();
        w.selected = 0;
        w.generation = generation;
        let query = SearchQuery::literal(&w.text)
            .with_case(w.case)
            .with_regex(w.regex)
            .with_whole_word(w.whole_word)
            .with_newline_sugar(sugar)
            .with_compat_fold(fold)
            .tuned(tuning);
        w.query = Some(query.clone());
        w.status = if w.text.is_empty() {
            Status::Empty
        } else if let Some(e) = query.known_error() {
            Status::Invalid(e)
        } else if query.shape().is_some_and(|s| s.too_heavy()) {
            Status::Refused(ScanLimit::TooHeavy)
        } else if root.is_none() {
            if root_failed {
                Status::NoRoot
            } else {
                Status::WaitingRoot
            }
        } else {
            Status::Interrupted
        };
        let run = w.status == Status::Interrupted;
        if let Some(job) = old_job {
            self.jobs.cancel(job);
        }
        self.release_hits(old);
        self.mark_dirty_render();
        let (true, Some(root)) = (run, root) else {
            return;
        };
        let open_docs = self.ws_open_docs(&root);
        let exclude = std::sync::Arc::new(self.exclude_config());
        let job = self.jobs.spawn(
            None,
            eve_view::Revision::INITIAL,
            JobKind::WorkspaceSearch {
                root,
                generation,
                query,
                exclude,
                open_docs,
                max_file_size: self.settings.config.ws_max_file_size,
                max_lines: WS_MAX_RESULT_LINES,
                state: None,
            },
        );
        self.io
            .pending
            .insert(job, Pending::WorkspaceSearch { generation });
        if let Some(Overlay::WorkspaceSearch(w)) = &mut self.ui.overlay {
            w.status = Status::Running { job };
        }
    }

    /// ルートの下にある、読込を終えた文書 (03 §12.7.5 の 1、§12.7.6 の 3)。canonical の無い文書・読込中の文書はディスクを
    /// 探す。本文が上限を超える文書とバイナリとして開いた文書は、飛ばす印だけを渡す (ディスクも探さない)。
    fn ws_open_docs(&self, root: &FsPath) -> Vec<OpenDocRef> {
        let root_key = canonical_key(root);
        self.docs
            .documents
            .values()
            .filter(|d| d.load_state == LoadState::Ready)
            .filter_map(|d| {
                let canonical = d.canonical.clone()?;
                if !under_root(&root_key, &canonical_key(&canonical)) {
                    return None;
                }
                let ident = d.identity_ref();
                let body = if d.read_only.contains(ReadOnlyReason::Binary) {
                    OpenDocBody::Binary
                } else if d.text().len_bytes() as u64 > self.settings.config.ws_max_file_size {
                    OpenDocBody::TooLarge
                } else {
                    OpenDocBody::Text(d.snapshot())
                };
                Some(OpenDocRef {
                    path: d.path.clone()?,
                    canonical,
                    file_id: ident.file_id,
                    file_id_unique: ident.file_id_unique,
                    body,
                })
            })
            .collect()
    }

    /// ルートの検出の結果が届いた (03 §12.7.1): 横断検索がルートを待っていれば始める (ルートが決まらなかったら
    /// `ui.finder.footer_no_root` にする)。
    pub(crate) fn ws_search_on_root(&mut self) {
        if matches!(
            &self.ui.overlay,
            Some(Overlay::WorkspaceSearch(w)) if w.status == Status::WaitingRoot
        ) {
            self.ws_search_start();
        }
    }

    /// 横断検索の結果 (`Pending::WorkspaceSearch`)。今の世代の結果だけを採り、古い世代の束は `bg` で手放す。`WorkspaceHits`
    /// は終わりではないので `true` (待ちの記録を残す)。
    pub(crate) fn on_ws_search_result(&mut self, generation: u64, r: JobResult) -> bool {
        let current = matches!(
            &self.ui.overlay,
            Some(Overlay::WorkspaceSearch(w))
                if w.generation == generation && matches!(w.status, Status::Running { .. })
        );
        match r {
            JobResult::WorkspaceHits { hits, progress, .. } => {
                if let (true, Some(Overlay::WorkspaceSearch(w))) = (current, &mut self.ui.overlay) {
                    w.hits.extend(hits);
                    w.progress = progress;
                    self.mark_dirty_render();
                } else {
                    // 取り消した検索の束 (取消しの前に送られていた): 主スレッドで手放さない
                    self.release_hits(hits);
                }
                true
            }
            JobResult::WorkspaceSearchDone {
                progress,
                truncated,
                ..
            } => {
                if let (true, Some(Overlay::WorkspaceSearch(w))) = (current, &mut self.ui.overlay) {
                    w.progress = progress;
                    // コンパイルで分かった誤り (大きさの上限) は、照合器が作れずに何も探さなかったので誤りとして示す
                    let error = w.query.as_ref().and_then(SearchQuery::known_error);
                    w.status = match error {
                        Some(e) if w.hits.is_empty() => Status::Invalid(e),
                        _ => Status::Done { truncated },
                    };
                    self.mark_dirty_render();
                }
                false
            }
            JobResult::Failed { err } => {
                if !current {
                    return false;
                }
                if let Some(Overlay::WorkspaceSearch(w)) = &mut self.ui.overlay {
                    w.status = if matches!(err, JobError::Cancelled) {
                        Status::Interrupted
                    } else {
                        Status::Failed
                    };
                }
                match &err {
                    JobError::Cancelled => {}
                    // ルートを読めない (03 §12.7.2 の 7)
                    JobError::Fs(e) => {
                        let root = self
                            .workspace()
                            .map(|w| w.root().display())
                            .unwrap_or_default();
                        self.show_message(
                            MessageLevel::Error,
                            keys::MSG_WORKSPACE_SCAN_FAILED,
                            &[("root", &root), ("reason", &fs_reason(e))],
                        );
                    }
                    // ジョブのパニックなど: 利用者に分かる知らせを出す (検索語を変えると探し直す)
                    other => {
                        self.show_message(
                            MessageLevel::Error,
                            keys::MSG_WORKSPACE_SEARCH_FAILED,
                            &[("reason", &reason_label(other))],
                        );
                    }
                }
                self.mark_dirty_render();
                false
            }
            _ => false,
        }
    }

    /// 入力欄が変わった: 待ち時間を置かずに探し直す。
    pub(crate) fn ws_search_edited(&mut self) {
        self.ws_search_start();
    }

    /// 横断検索のオーバーレイが開いていればトグルを切り替えて探し直す (`search.toggle_*` は、オーバーレイが開いていれば
    /// オーバーレイの検索語に効く。03 §12.7.5)。開いていなければ `None`。
    pub(crate) fn ws_search_toggle(&mut self, t: WsToggle) -> Option<CommandResult> {
        let Some(Overlay::WorkspaceSearch(w)) = &mut self.ui.overlay else {
            return None;
        };
        match t {
            WsToggle::Case => w.case = w.case.next(),
            WsToggle::Regex => w.regex = !w.regex,
            WsToggle::Word => w.whole_word = !w.whole_word,
        }
        self.ws_search_start();
        Some(CommandResult::Done)
    }

    /// `ui.list_next` / `ui.list_prev` (折り返す)。
    pub(crate) fn ws_search_move(&mut self, forward: bool) -> CommandResult {
        let Some(Overlay::WorkspaceSearch(w)) = &mut self.ui.overlay else {
            return CommandResult::Rejected(crate::command::RejectReason::NotAvailable);
        };
        let n = w.hits.len();
        if n > 0 {
            w.selected = if forward {
                (w.selected + 1) % n
            } else {
                (w.selected + n - 1) % n
            };
        }
        self.mark_dirty_render();
        CommandResult::Done
    }

    /// `ui.list_page_down` / `ui.list_page_up` (端で止まる)。
    pub(crate) fn ws_search_page(&mut self, forward: bool) -> CommandResult {
        let Some(Overlay::WorkspaceSearch(w)) = &mut self.ui.overlay else {
            return CommandResult::Rejected(crate::command::RejectReason::NotAvailable);
        };
        let n = w.hits.len();
        if n > 0 {
            w.selected = if forward {
                (w.selected + WINDOW).min(n - 1)
            } else {
                w.selected.saturating_sub(WINDOW)
            };
        }
        self.mark_dirty_render();
        CommandResult::Done
    }

    /// `ui.confirm` (`Enter`): オーバーレイを閉じ、選んだ結果のファイルをフォーカスのペインで開いて (開いていればその文書へ
    /// 切り替えて)、一致の位置へ移る (03 §12.7.5 の「ジャンプ」)。結果が無ければ何もしない。
    pub(crate) fn ws_search_confirm(&mut self) -> CommandResult {
        let Some(Overlay::WorkspaceSearch(w)) = &self.ui.overlay else {
            return CommandResult::Rejected(crate::command::RejectReason::NotAvailable);
        };
        let Some(h) = w.hits.get(w.selected.min(w.hits.len().saturating_sub(1))) else {
            return CommandResult::Done;
        };
        let path = h.path.clone();
        let jump = WsJump {
            pane: self.focused_pane(),
            line: h.line,
            range: h.ranges.first().cloned().unwrap_or(0..0),
        };
        self.take_overlay();
        self.mark_dirty_render();
        // 名前を付けて保存の照合の間は `file.open` と同じく無視する (03 §12.7.6 の細目 (8))
        if self.ignore_while_save_as_check() {
            return CommandResult::Done;
        }
        // 検索バーが開いたままだと、読込の完了の表示の切り替えが保留されて一致へ移れない (03 §9.2.1 の「表示の切り替え」)。
        // 結果を選んだら閉じる (フォーカスのペインに別の文書を出すと閉じるのと同じ)
        self.search_close();
        // 開いている文書 (パスの文字列か canonical のパスが同じ。別の書き方のパスで開いた文書を含む) の一致: 開くジョブを
        // 走らせずに、その文書へ切り替えて移る (「既に開いています」「…として開いています」を出さない)。読込中なら、読込の
        // 完了で移る
        if let Some((id, ready)) = self.ws_open_doc_of(&path) {
            if self.active_document() != Some(id) {
                self.activate_document(id);
            }
            if ready {
                if self.active_document() == Some(id)
                    && let Some(view) = self.focused_view()
                {
                    self.apply_ws_jump(view, id, &jump);
                }
            } else {
                self.io.ws_jumps.insert(id, jump);
            }
            self.mark_dirty_render();
            return CommandResult::Done;
        }
        let res = self.open_fs_path(
            path.clone(),
            OpenOptions {
                must_exist: true,
                ..OpenOptions::default()
            },
        );
        // 新しく開いた文書: 読込が終わった時点で移る (`complete_open`)
        if let Some(id) = self
            .docs
            .documents
            .iter()
            .find(|(_, d)| d.path.as_ref() == Some(&path))
            .map(|(id, _)| *id)
        {
            self.io.ws_jumps.insert(id, jump);
        }
        res
    }

    /// 結果のパス `path` の、開いている文書 (パスの文字列が同じか、canonical のパスの比べる形が同じ) と、読込を終えているか。
    fn ws_open_doc_of(&self, path: &FsPath) -> Option<(DocumentId, bool)> {
        let key = canonical_key(path);
        self.docs
            .documents
            .iter()
            .filter(|(_, d)| d.load_state != LoadState::Failed)
            .find(|(_, d)| {
                d.path.as_ref() == Some(path)
                    || d.canonical
                        .as_ref()
                        .is_some_and(|c| canonical_key(c) == key)
            })
            .map(|(id, d)| (*id, d.load_state == LoadState::Ready))
    }

    /// 読込が終わった文書 `doc` の横断検索のジャンプ: 開いた時点のペインがまだその文書を出していれば移る。読込の完了の表示の
    /// 切り替えが保留されている (オーバーレイ・検索バーの間、03 §9.2.1) ときは、切り替えた時点 (`try_open_switch`) まで待つ。
    pub(crate) fn finish_ws_jump(&mut self, doc: DocumentId) {
        let Some(jump) = self.io.ws_jumps.remove(&doc) else {
            return;
        };
        if !self
            .document(doc)
            .is_some_and(|d| d.load_state == LoadState::Ready)
        {
            self.io.ws_jumps.insert(doc, jump);
            return;
        }
        let Some(pane) = jump.pane.filter(|p| self.panes_showing(doc).contains(p)) else {
            if self.io.open_switch.get(&doc).is_some_and(|s| s.deferred) {
                self.io.ws_jumps.insert(doc, jump);
            }
            return;
        };
        if let Some(view) = self.pane_view(pane) {
            self.apply_ws_jump(view, doc, &jump);
        }
    }

    /// 二重オープンで閉じた文書 `from` のジャンプを、既存の文書 `to` に移す (03 §12.7.6 の 1)。`to` が読込を終えていれば、
    /// 出しているペインですぐに移る。
    pub(crate) fn transfer_ws_jump(&mut self, from: DocumentId, to: DocumentId) {
        let Some(jump) = self.io.ws_jumps.remove(&from) else {
            return;
        };
        let ready = self
            .document(to)
            .is_some_and(|d| d.load_state == LoadState::Ready);
        if !ready {
            self.io.ws_jumps.insert(to, jump);
            return;
        }
        // 開いた時点のペインに出ていなければ移らない (ほかのペインのカーソルは動かさない)
        let pane = jump.pane.filter(|p| self.panes_showing(to).contains(p));
        if let Some(view) = pane.and_then(|p| self.pane_view(p)) {
            self.apply_ws_jump(view, to, &jump);
        }
    }

    /// 一致の位置 `(line, range)` を今の本文に当てる: 行が無ければ最後の行、行の中の位置が行より長ければ行末、書記素の
    /// 境界でなければ前の境界に丸める。丸めずに当てられたら一致の範囲を主の選択にし、丸めたらカーソルだけを置く。中央に寄せる。
    fn apply_ws_jump(&mut self, view: eve_view::ViewId, doc: DocumentId, jump: &WsJump) {
        let Some(d) = self.docs.documents.get(&doc) else {
            return;
        };
        let t = d.text();
        let last = t.len_lines().saturating_sub(1);
        let line = jump.line as usize;
        let mut exact = line <= last;
        let line = LineIdx(line.min(last));
        let ls = t.line_to_byte(line).0;
        let le = t.line_end_byte(line).0;
        let mut start = ls + jump.range.start as usize;
        if start > le {
            start = le;
            exact = false;
        }
        if !is_grapheme_boundary(t, ByteOff(start)) {
            start = eve_text::prev_grapheme_boundary(t, ByteOff(start)).0;
            exact = false;
        }
        // 範囲の終わりも行の中に収まり、書記素の境界のときだけ範囲を選ぶ (ファイルが変わって行が縮んでいれば、改行を越えて
        // 次の行まで選ばない)
        let end = ls + jump.range.end as usize;
        let sel = if exact && end >= start && end <= le && is_grapheme_boundary(t, ByteOff(end)) {
            Selection::single(eve_text::Range::new(ByteOff(start), ByteOff(end)))
        } else {
            Selection::point(ByteOff(start))
        };
        self.set_selection(view, sel);
        self.center_cursor(view);
        self.mark_dirty_render();
    }

    /// 横断検索のオーバーレイを閉じた (`set_overlay` / `take_overlay` が呼ぶ): 走っている検索を取り消し (次に開いたときに
    /// 探し直す)、状態を持っておく。
    pub(crate) fn ws_search_closed(&mut self, mut w: WsSearchOverlay) {
        match w.status {
            Status::Running { job } => {
                self.jobs.cancel(job);
                w.status = Status::Interrupted;
            }
            Status::WaitingRoot => w.status = Status::Interrupted,
            _ => {}
        }
        w.all_selected = false;
        if let Some(old) = self.workspace.search.saved.replace(Box::new(w)) {
            self.release_hits(old.hits);
        }
    }

    /// 横断検索の結果を手放す (`bg` の `ReleaseCandidates`。10,000 行の抜粋を主スレッドで解放しない、03 §12.7.5)。
    pub(crate) fn release_hits(&mut self, hits: Vec<WsHit>) {
        if hits.is_empty() {
            return;
        }
        // 終端の結果 (`Released`) は待たない (待ちの記録を作らない)
        let _ = self.jobs.spawn(
            None,
            eve_view::Revision::INITIAL,
            JobKind::ReleaseCandidates {
                chunks: Vec::new(),
                hits,
            },
        );
    }

    /// 横断検索の一覧のオーバーレイのモデル (04 §8.12)。
    pub(crate) fn ws_search_model(&self, w: &WsSearchOverlay) -> OverlayModel {
        let render = |key: &str, args: &[(&str, &str)]| {
            self.catalog().render(key, args, &|c| self.key_label(c))
        };
        let selected = w.selected.min(w.hits.len().saturating_sub(1));
        let first = window_first(selected);
        let items: Vec<ListItem> = w
            .hits
            .iter()
            .skip(first)
            .take(WINDOW)
            .map(|h| {
                let line = (u64::from(h.line) + 1).to_string();
                let label = render(
                    keys::UI_WORKSPACE_SEARCH_ITEM,
                    &[("path", &h.rel), ("line", &line), ("text", &h.excerpt)],
                );
                let empty = render(
                    keys::UI_WORKSPACE_SEARCH_ITEM,
                    &[("path", &h.rel), ("line", &line), ("text", "")],
                );
                let ranges = arg_position(&label, &empty, &h.excerpt)
                    .map(|k| hit_grapheme_ranges(&label, k, h))
                    .unwrap_or_default();
                let (label, map) = cell_text_indexed(&label, CutPolicy::End);
                ListItem {
                    label,
                    detail: None,
                    match_ranges: map_ranges(&ranges, &map),
                    keys: Vec::new(),
                    disabled_reason: None,
                }
            })
            .collect();
        let toggles = toggle_marks(w.case, w.regex, w.whole_word, self.term.caps.ascii_only);
        let title = render(keys::UI_WORKSPACE_SEARCH_TITLE, &[("toggles", &toggles)]);
        let footer = self.ws_search_footer(w, &render);
        let no_match = match w.status {
            Status::Done { .. } | Status::Failed if w.hits.is_empty() => {
                render(keys::UI_WORKSPACE_SEARCH_NO_MATCH, &[])
            }
            _ => String::new(),
        };
        let cells = cell_text(&w.text, CutPolicy::End).spans;
        let cursor = cell_text(&w.text[..w.cursor], CutPolicy::End).spans.len();
        let selection = field_selection(w.all_selected, cells.len());
        let placeholder = w.text.is_empty().then(|| {
            cell_text(
                &render(keys::UI_WORKSPACE_SEARCH_PLACEHOLDER, &[]),
                CutPolicy::End,
            )
        });
        OverlayModel::List {
            title: cell_text(&title, CutPolicy::End),
            input: InputFieldModel {
                len: cells.len(),
                cells,
                first: 0,
                cursor,
                selection,
                placeholder,
            },
            items,
            first,
            selected,
            footer: cell_text(&footer, CutPolicy::Never),
            no_match: cell_text(&no_match, CutPolicy::End),
        }
    }

    /// 件数の欄 (04 §8.12)。
    fn ws_search_footer(&self, w: &WsSearchOverlay, render: &Render<'_>) -> String {
        let p = &w.progress;
        let lines = w.hits.len().to_string();
        let files = p.files.to_string();
        let hit_files = p.hit_files.to_string();
        let mut s = match &w.status {
            Status::Empty => return String::new(),
            Status::WaitingRoot => return render(keys::UI_FINDER_FOOTER_WAITING, &[]),
            Status::NoRoot => return render(keys::UI_FINDER_FOOTER_NO_ROOT, &[]),
            Status::Invalid(e) => {
                return render(
                    keys::MSG_SEARCH_INVALID_REGEX_TEXT,
                    &[("reason", invalid_regex_reason(e))],
                );
            }
            Status::Refused(l) => {
                let (_, key, args) = limit_message(*l);
                let args: Vec<(&str, &str)> = args.iter().map(|(k, v)| (*k, v.as_str())).collect();
                return render(key, &args);
            }
            Status::Running { .. } | Status::Interrupted => render(
                keys::UI_WORKSPACE_SEARCH_FOOTER_RUNNING,
                &[("lines", &lines), ("files", &files)],
            ),
            Status::Done { truncated: false } => render(
                keys::UI_WORKSPACE_SEARCH_FOOTER_DONE,
                &[
                    ("lines", &lines),
                    ("hit_files", &hit_files),
                    ("files", &files),
                ],
            ),
            Status::Done { truncated: true } => render(
                keys::UI_WORKSPACE_SEARCH_FOOTER_TRUNCATED,
                &[("lines", &lines), ("hit_files", &hit_files)],
            ),
            Status::Failed => render(
                keys::UI_WORKSPACE_SEARCH_FOOTER_FAILED,
                &[("lines", &lines)],
            ),
        };
        if p.skipped() > 0 {
            s.push_str(&render(
                keys::UI_WORKSPACE_SEARCH_SKIPPED_SUFFIX,
                &[("skipped", &p.skipped().to_string())],
            ));
        }
        if p.unreadable_dirs > 0 {
            s.push_str(&render(
                keys::UI_FINDER_UNREADABLE_SUFFIX,
                &[("dirs", &p.unreadable_dirs.to_string())],
            ));
        }
        s
    }

    /// 横断検索の状態 (試験用): (検索中か, 出している行 `相対パス:行` (1 始まり), 選択中の行, 進捗)。
    #[doc(hidden)]
    pub fn ws_search_rows(&self) -> Option<(bool, Vec<String>, usize, WsProgress)> {
        let Some(Overlay::WorkspaceSearch(w)) = &self.ui.overlay else {
            return None;
        };
        Some((
            matches!(w.status, Status::Running { .. }),
            w.hits
                .iter()
                .map(|h| format!("{}:{}", h.rel, u64::from(h.line) + 1))
                .collect(),
            w.selected,
            w.progress,
        ))
    }

    /// 横断検索の今の結果 (試験用)。
    #[doc(hidden)]
    pub fn ws_search_hits(&self) -> Option<&[WsHit]> {
        match &self.ui.overlay {
            Some(Overlay::WorkspaceSearch(w)) => Some(&w.hits),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 引数の位置を空の文言との差から求める() {
        assert_eq!(arg_position("a.rs:3: foo", "a.rs:3: ", "foo"), Some(8));
        assert_eq!(arg_position("[foo] a.rs", "[] a.rs", "foo"), Some(1));
        // 抜粋が後ろの文言と同じ文字で始まっても、空の文言と揃う位置だけを採る
        assert_eq!(arg_position("x:yy:", "x::", "yy"), Some(2));
        assert_eq!(arg_position("abc", "abc", "z"), None, "長さが合わない");
        assert_eq!(arg_position("abc", "ab", ""), None);
    }

    #[test]
    fn 抜粋の外の一致は切り落として書記素の番号にする() {
        let h = WsHit {
            path: FsPath::from("/w/a.rs"),
            rel: "a.rs".into(),
            line: 0,
            ranges: vec![2..5, 7..20],
            excerpt: "aあbcd".into(),
            excerpt_start: 1,
            from_open_doc: false,
        };
        // 抜粋 "aあbcd" は行の 1 バイト目から。一致 2..5 は抜粋の 1..4 (あ)、7..20 は抜粋の 6.. (d と、抜粋の外)
        let label = "p:1: aあbcd";
        let ranges = hit_grapheme_ranges(label, 5, &h);
        assert_eq!(ranges, [6..7, 9..10]);
    }

    #[test]
    fn トグルの記号() {
        assert_eq!(
            toggle_marks(Case::Smart, false, false, false),
            "[Aa~] [.*] [ab]"
        );
        assert_eq!(
            toggle_marks(Case::Sensitive, true, true, false),
            "[Aa\u{2713}] [.*\u{2713}] [ab\u{2713}]"
        );
        assert_eq!(
            toggle_marks(Case::Insensitive, true, false, true),
            "[Aa] [.**] [ab]"
        );
    }

    #[test]
    fn ルートの下か() {
        use std::path::Path;
        assert!(under_root(Path::new("/w"), Path::new("/w/a.txt")));
        assert!(!under_root(Path::new("/w"), Path::new("/wx/a.txt")));
        assert!(!under_root(Path::new("/w"), Path::new("/w")));
        assert!(under_root(Path::new("/"), Path::new("/a")));
        assert_eq!(
            under_root(Path::new("/W"), Path::new("/w/a")),
            cfg!(any(windows, target_os = "macos"))
        );
    }
}
