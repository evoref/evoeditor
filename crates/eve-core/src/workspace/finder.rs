//! ファインダ (docs/03 §12.7.4、04 §8.11、01 FR-125。v0.5 の M31 の段 6) の主スレッドの側: 候補の一覧の寿命 (§12.7.2 の
//! 「一覧の寿命」)、走査と照合のジョブの投入と結果の採否、一覧のオーバーレイ (`overlay.active == "finder"`) のモデル。
//!
//! - **一覧** ([`FinderList`]): 塊 (`Arc<[Candidate]>`) の列。最初に開いたときに走査を始め、閉じた後も持つ。開くたびに、前の
//!   一覧を見せたまま新しい走査を始め、終わった時点で差し替える (最初の走査の間だけは、届いた塊をそのまま足す)。閉じてから
//!   [`FINDER_LIST_KEEP`] 開かなければ手放す。
//! - **解放**: 主スレッドで大きな一覧を drop しない。手放す塊は [`EditorState::release_chunks`] で `ReleaseCandidates` に渡す
//!   (一覧がまだ持っている塊は、主スレッドで参照を外しても最後の参照にならないので、そのまま外す)。
//! - **照合**: 入力欄が変わるたびに、走っている照合を取り消して `FinderMatch` を投げ直す。結果は世代で照らし、古いものを捨てる。
//!   最初の走査の間は、新しい候補が届いていて前の照合から [`FINDER_RESCORE_INTERVAL`] 経っていれば照合し直す (最後の束の後は
//!   必ず 1 回)。
//!
//! spec: 03 §12.7.2, 03 §12.7.4, 03 §12.7.7, 04 §8.11, 07 §7.1, 07 §11.6, 01 FR-125

use std::ops::Range;
use std::sync::Arc;

use eve_view::{CutPolicy, InputFieldModel, JobId, ListItem, MessageLevel, OverlayModel};

use super::{
    Candidate, ExcludeConfig, FINDER_LIST_KEEP, FINDER_RESCORE_INTERVAL, FINDER_TOP, FinderHit,
    RootSource, WORKSPACE_MAX_FILES,
};
use crate::command::{CommandCtx, CommandKind, CommandRegistry, CommandResult, Permission, ids};
use crate::editor::state::EditorState;
use crate::io::error_keys::{fs_reason, reason_label};
use crate::io::io_jobs::OpenOptions;
use crate::io::pending::Pending;
use crate::jobs::{JobError, JobKind, JobResult};
use crate::layout::{cell_text, cell_text_indexed};
use crate::time::Tick;
use crate::ui_model::keys;
use crate::ui_model::overlay::{Overlay, field_selection};
use crate::ui_model::palette::WINDOW;

/// `workspace.exclude` の既定 (07 §5.9)。設定のキーはまだ公開していない (予約のまま、03 §12.7.1 の段 1 の細目 (1))。公開する
/// までは、この値で除外する。
pub(crate) const DEFAULT_WORKSPACE_EXCLUDE: [&str; 3] = ["**/.git", "**/node_modules", "**/target"];

/// 塊の列 (一覧・走査の途中の塊・照合に渡した写し)。
pub(crate) type Chunks = Vec<Arc<[Candidate]>>;

/// 走っている走査 (03 §12.7.2)。
#[derive(Debug)]
struct ScanRun {
    job: JobId,
    generation: u64,
    /// 最初の走査 (一覧が無かった): 届いた塊をそのまま一覧に足す。`false` なら終わるまで `chunks` に溜めて、終わった時点で
    /// 一覧を差し替える (走査の途中の新しい候補を前の一覧に混ぜない)。
    fresh: bool,
    /// 差し替え用に溜めた塊 (`fresh` のときは使わない)。
    chunks: Chunks,
    /// 差し替え用の候補の数。
    total: u64,
    /// その時点までに読めなかったディレクトリの数。
    unreadable: u64,
}

/// ファインダの候補の一覧と走査の状態 (`WorkspaceState` が 1 つ持つ)。
#[derive(Debug, Default)]
pub(crate) struct FinderList {
    /// 一覧 (照合に渡す塊の列)。
    chunks: Chunks,
    /// 一覧の候補の数。
    total: u64,
    /// 一覧を作った走査が上限か列挙の途中の失敗で打ち切った。
    truncated: bool,
    /// 一覧を作った走査で読めなかったディレクトリの数。
    unreadable: u64,
    /// 一覧を作る走査が終わった (一覧がある)。
    complete: bool,
    /// 一覧を作った (最初の) 走査が失敗した (ルートを読めない・パニック)。一覧が欠けているので、件数の欄を完了に見せない
    /// (`ui.finder.footer_incomplete`、03 §12.7.4 の段 6 の細目 (12))。
    failed: bool,
    /// 走っている走査。
    scan: Option<ScanRun>,
    /// 次の走査の世代。
    next_scan_generation: u64,
    /// 次の照合の世代。
    next_match_generation: u64,
    /// ファインダを閉じた時刻 (開いている間・一覧を手放した後は `None`)。
    closed_at: Option<Tick>,
}

impl FinderList {
    /// 一覧の候補の数 (試験用)。
    pub(crate) fn total(&self) -> u64 {
        self.total
    }

    /// 走査が走っている。
    pub(crate) fn scanning(&self) -> bool {
        self.scan.is_some()
    }

    /// 一覧か走査の途中の塊が持っている塊か。
    fn holds(&self, c: &Arc<[Candidate]>) -> bool {
        let same = |x: &Arc<[Candidate]>| Arc::ptr_eq(x, c);
        self.chunks.iter().any(same)
            || self
                .scan
                .as_ref()
                .is_some_and(|s| s.chunks.iter().any(same))
    }
}

/// 一覧に出している上位の行 1 つ。塊の `Arc` を持つので、一覧を差し替えた後も行は有効 (手放すときは `release_chunks`)。
#[derive(Clone, Debug)]
pub(crate) struct FinderRow {
    chunk: Arc<[Candidate]>,
    index: u32,
    /// 一致した位置 (相対パスの書記素の番号の半開区間)。
    ranges: Vec<Range<u32>>,
}

impl FinderRow {
    fn candidate(&self) -> &Candidate {
        &self.chunk[self.index as usize]
    }
}

/// ファインダのオーバーレイの状態 (`Overlay::Finder`)。
#[derive(Clone, Debug, Default)]
pub(crate) struct FinderOverlay {
    pub text: String,
    /// `text` 内のカーソル (バイト。書記素境界)。
    pub cursor: usize,
    /// 欄内全選択 (`select.all`) の最中。
    pub all_selected: bool,
    /// 上位の行 (最後に届いた照合の結果)。
    rows: Vec<FinderRow>,
    /// 選択中の行。
    selected: usize,
    /// 最後に届いた照合で一致した件数。
    matched: usize,
    /// 走っている照合のジョブ。
    match_job: Option<JobId>,
    /// 最後に投げた照合の世代。
    generation: u64,
    /// `rows` を作った照合の世代 (`generation` と違えば、行は前の検索語のもの)。
    shown_generation: Option<u64>,
    /// 最後に照合を投げた時刻。
    last_match_at: Option<Tick>,
    /// 前の照合の後に新しい候補が届いた (最初の走査の間。間隔が過ぎたら照合し直す)。
    rematch_due: bool,
    /// 照合を待つ間に `Enter` を押した: 結果が届いたら、その先頭 (選択) を開く。
    confirm_waiting: bool,
    /// 入力欄が変わった: 次に届いた結果で先頭を選ぶ (選択を保つのは同じ検索語の照合し直しだけ、03 §12.7.4)。
    reset_selection: bool,
}

/// 選択中の行 selected が入る表示の窓の先頭の行。
fn window_first(selected: usize) -> usize {
    if selected < WINDOW {
        0
    } else {
        selected + 1 - WINDOW
    }
}

/// 照合の結果の行を、記法に直した後の `label` の Span の番号に写す (`cell_text_indexed` の写し)。
fn map_ranges(ranges: &[Range<u32>], map: &[Option<usize>]) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    for r in ranges {
        for g in r.start..r.end {
            let Some(Some(i)) = map.get(g as usize) else {
                continue;
            };
            match out.last_mut() {
                Some(last) if last.end == *i => last.end = i + 1,
                _ => out.push(*i..i + 1),
            }
        }
    }
    out
}

fn workspace_open_file(ctx: &mut CommandCtx<'_>) -> CommandResult {
    let query = ctx.args.str("query").map(str::to_owned);
    ctx.state.open_finder(query)
}

/// `workspace.open_file` (07 §11.6)。
pub(crate) fn register(r: &mut CommandRegistry) {
    r.add(
        ids::WORKSPACE_OPEN_FILE,
        CommandKind::Ui,
        Permission::Ui,
        "",
        workspace_open_file,
    );
}

impl EditorState {
    /// `workspace.open_file`: ファインダを開く (03 §12.7.4)。`query` があれば入力欄に入れる。ルートが決まっていれば走査を
    /// 始め (走っていればそのまま使う)、今の一覧で照合する。決まる前なら「調べています」を出し、決まった時点で始める。
    pub(crate) fn open_finder(&mut self, query: Option<String>) -> CommandResult {
        let text = query.unwrap_or_default();
        let cursor = text.len();
        self.set_overlay(Some(Overlay::Finder(Box::new(FinderOverlay {
            text,
            cursor,
            ..FinderOverlay::default()
        }))));
        self.workspace.finder.closed_at = None;
        if self.workspace().is_some() {
            self.finder_start_scan();
            self.finder_rematch();
        }
        self.mark_dirty_render();
        CommandResult::Done
    }

    /// ルートの検出の結果が届いた (03 §12.7.1): ファインダが「調べています」で待っていれば走査を始める。ルートが `DIR` で
    /// 決まったら、ほかのオーバーレイが出ていなければファインダを開く (07 §18.2)。
    pub(crate) fn finder_on_root(&mut self, source: RootSource) {
        if self.workspace().is_none() {
            return;
        }
        if matches!(self.ui.overlay, Some(Overlay::Finder(_))) {
            self.finder_start_scan();
            self.finder_rematch();
            self.mark_dirty_render();
        } else if source == RootSource::Dir && self.ui.overlay.is_none() {
            self.open_finder(None);
        }
    }

    /// 走査を始める (走っていれば何もしない。走っている走査が終わると新しい一覧になる)。
    fn finder_start_scan(&mut self) {
        let Some(root) = self.workspace().map(|w| w.root().clone()) else {
            return;
        };
        let list = &mut self.workspace.finder;
        if list.scan.is_some() {
            return;
        }
        let generation = list.next_scan_generation;
        list.next_scan_generation += 1;
        // 一覧が空 (最初・手放した後・前の走査が候補を出さずに失敗した) なら、届いた束をそのまま足す (03 §12.7.4 の段 6 の
        // 細目 (6))
        let fresh = list.chunks.is_empty();
        let exclude = Arc::new(ExcludeConfig {
            exclude: DEFAULT_WORKSPACE_EXCLUDE.map(str::to_owned).to_vec(),
            ..ExcludeConfig::new()
        });
        let job = self.jobs.spawn(
            None,
            eve_view::Revision::INITIAL,
            JobKind::WorkspaceScan {
                root,
                exclude,
                generation,
                walk: None,
            },
        );
        self.workspace.finder.scan = Some(ScanRun {
            job,
            generation,
            fresh,
            chunks: Vec::new(),
            total: 0,
            unreadable: 0,
        });
        self.io
            .pending
            .insert(job, Pending::WorkspaceScan { generation });
    }

    /// 走査の結果 (`Pending::WorkspaceScan`)。`ScanBatch` は終わりではないので、待ちの記録を残す (呼び側は `true` なら
    /// 記録を外さない)。
    pub(crate) fn on_scan_result(&mut self, generation: u64, r: JobResult) -> bool {
        let current = self
            .workspace
            .finder
            .scan
            .as_ref()
            .is_some_and(|s| s.generation == generation);
        match r {
            JobResult::ScanBatch {
                chunk,
                unreadable_dirs,
                ..
            } => {
                if !current {
                    // 取り消した走査の束 (取消しの前に送られていた): 主スレッドで手放さない
                    self.release_chunks(vec![chunk]);
                    return true;
                }
                self.finder_take_batch(chunk, unreadable_dirs);
                true
            }
            JobResult::ScanDone {
                total, truncated, ..
            } if current => {
                self.finder_scan_done(total, truncated);
                false
            }
            JobResult::Failed { err } if current => {
                let scan = self.workspace.finder.scan.take();
                let cancelled = matches!(err, JobError::Cancelled);
                if let Some(s) = scan {
                    self.release_chunks(s.chunks);
                    // 最初の走査の失敗: 一覧は欠けている (件数の欄を完了に見せない)。走査し直しの失敗は前の一覧のまま
                    if s.fresh && !cancelled {
                        self.workspace.finder.complete = true;
                        self.workspace.finder.failed = true;
                    }
                }
                // ルートを読めない・走査のジョブのパニック (03 §12.7.4 の段 6 の細目 (12))
                let reason = match &err {
                    JobError::Fs(e) => Some(fs_reason(e)),
                    JobError::Cancelled => None,
                    other => Some(reason_label(other)),
                };
                if let Some(reason) = reason {
                    let root = self
                        .workspace()
                        .map(|w| w.root().display())
                        .unwrap_or_default();
                    self.show_message(
                        MessageLevel::Error,
                        keys::MSG_WORKSPACE_SCAN_FAILED,
                        &[("root", &root), ("reason", &reason)],
                    );
                }
                self.mark_dirty_render();
                false
            }
            _ => false,
        }
    }

    fn finder_take_batch(&mut self, chunk: Arc<[Candidate]>, unreadable: u64) {
        let list = &mut self.workspace.finder;
        let Some(scan) = list.scan.as_mut() else {
            return;
        };
        scan.unreadable = unreadable;
        let added = !chunk.is_empty();
        if scan.fresh {
            list.unreadable = unreadable;
            if added {
                list.total += chunk.len() as u64;
                list.chunks.push(chunk);
            }
        } else if added {
            scan.total += chunk.len() as u64;
            scan.chunks.push(chunk);
        }
        let fresh = scan.fresh;
        if fresh {
            // 最初の走査の間は、それまでに見つけたファイルで絞り込める (01 FR-125 AC-3)。間隔を置いて照合し直す
            if added && let Some(Overlay::Finder(f)) = &mut self.ui.overlay {
                f.rematch_due = true;
            }
            self.finder_rematch_if_due();
            self.mark_dirty_render();
        }
    }

    fn finder_scan_done(&mut self, total: u64, truncated: bool) {
        let Some(scan) = self.workspace.finder.scan.take() else {
            return;
        };
        let list = &mut self.workspace.finder;
        list.truncated = truncated;
        list.unreadable = scan.unreadable;
        list.complete = true;
        list.failed = false;
        if scan.fresh {
            debug_assert_eq!(
                list.total, total,
                "最初の走査で足した数と走査の数が合わない"
            );
        } else {
            // 新しい一覧に差し替え、古い一覧は `bg` で手放す (出している行が古い塊を持っていても、行は有効なまま)
            let old = std::mem::replace(&mut list.chunks, scan.chunks);
            list.total = scan.total;
            self.release_chunks(old);
        }
        // 最後の束の後は必ず 1 回照合する
        if matches!(self.ui.overlay, Some(Overlay::Finder(_))) {
            self.finder_rematch();
        }
        self.mark_dirty_render();
    }

    /// 最初の走査の間の照合し直し: 新しい候補が届いていて、前の照合から間隔が過ぎていれば投げる。
    fn finder_rematch_if_due(&mut self) {
        let now = self.now;
        let due = match &self.ui.overlay {
            Some(Overlay::Finder(f)) => {
                f.rematch_due
                    && f.last_match_at
                        .is_none_or(|t| now >= t + FINDER_RESCORE_INTERVAL)
            }
            _ => false,
        };
        if due {
            self.finder_rematch();
        }
    }

    /// 今の入力欄と一覧で照合を投げ直す (走っている照合は取り消す)。一覧が空なら、ジョブを投げずに 0 件にする。
    pub(crate) fn finder_rematch(&mut self) {
        let generation = self.workspace.finder.next_match_generation;
        self.workspace.finder.next_match_generation += 1;
        let chunks = self.workspace.finder.chunks.clone();
        let now = self.now;
        let Some(Overlay::Finder(f)) = &mut self.ui.overlay else {
            return;
        };
        let old_job = f.match_job.take();
        f.generation = generation;
        f.rematch_due = false;
        // 照らす候補が無ければ照合を始めない (最初の束が届いたらすぐに照合する)
        f.last_match_at = (!chunks.is_empty()).then_some(now);
        let query: Arc<str> = Arc::from(f.text.as_str());
        if let Some(job) = old_job {
            self.jobs.cancel(job);
        }
        if chunks.is_empty() {
            // 照らす候補が無い: 0 件 (行は無いので、解放するものも無い)
            let Some(Overlay::Finder(f)) = &mut self.ui.overlay else {
                return;
            };
            let old = std::mem::take(&mut f.rows);
            f.matched = 0;
            f.selected = 0;
            f.shown_generation = Some(generation);
            let waiting = std::mem::take(&mut f.confirm_waiting);
            self.release_rows(old);
            if waiting {
                self.finder_confirm();
            }
            return;
        }
        let job = self.jobs.spawn(
            None,
            eve_view::Revision::INITIAL,
            JobKind::FinderMatch {
                chunks: chunks.clone(),
                query,
                top: FINDER_TOP,
                generation,
            },
        );
        if let Some(Overlay::Finder(f)) = &mut self.ui.overlay {
            f.match_job = Some(job);
        }
        self.io
            .pending
            .insert(job, Pending::FinderMatch { generation, chunks });
    }

    /// 照合の結果 (`Pending::FinderMatch`)。今の世代の結果だけを行にする。選択中の候補のパスが新しい上位に残っていれば
    /// 選択を保ち、残っていなければ先頭を選ぶ。
    pub(crate) fn finish_finder_match(&mut self, generation: u64, chunks: Chunks, r: JobResult) {
        let current =
            matches!(&self.ui.overlay, Some(Overlay::Finder(f)) if f.generation == generation);
        let JobResult::FinderMatched { matched, top, .. } = r else {
            // 取り消した・失敗した照合: 写しの塊を手放す。今の世代の失敗なら `Enter` の待ちも解く (03 §12.7.4 の段 6 の細目 (4))
            if current && let Some(Overlay::Finder(f)) = &mut self.ui.overlay {
                f.match_job = None;
                f.confirm_waiting = false;
            }
            self.release_chunks(chunks);
            return;
        };
        if !current {
            self.release_chunks(chunks);
            return;
        }
        let rows: Vec<FinderRow> = top
            .into_iter()
            .filter_map(
                |FinderHit {
                     chunk,
                     index,
                     ranges,
                     ..
                 }| {
                    let c = chunks.get(chunk as usize)?;
                    (c.len() > index as usize).then(|| FinderRow {
                        chunk: c.clone(),
                        index,
                        ranges,
                    })
                },
            )
            .collect();
        let Some(Overlay::Finder(f)) = &mut self.ui.overlay else {
            return;
        };
        // 選択を保つのは同じ検索語の照合し直しだけ。入力欄が変わった後の最初の結果では先頭を選ぶ
        let keep = (!std::mem::take(&mut f.reset_selection))
            .then(|| {
                f.rows
                    .get(f.selected)
                    .map(|r| r.candidate().rel().to_owned())
            })
            .flatten();
        f.selected = keep
            .and_then(|rel| rows.iter().position(|r| r.candidate().rel() == rel))
            .unwrap_or(0);
        f.matched = matched;
        f.match_job = None;
        f.shown_generation = Some(generation);
        let old = std::mem::replace(&mut f.rows, rows);
        let waiting = std::mem::take(&mut f.confirm_waiting);
        self.release_rows(old);
        self.release_chunks(chunks);
        self.mark_dirty_render();
        if waiting {
            self.finder_confirm();
        }
    }

    /// 入力欄が変わった (03 §12.7.4 の「取消しと投げ直し」): 待ち時間を置かずに照合し直す。
    pub(crate) fn finder_edited(&mut self) {
        if let Some(Overlay::Finder(f)) = &mut self.ui.overlay {
            f.confirm_waiting = false;
            f.reset_selection = true;
        }
        if self.workspace().is_some() {
            self.finder_rematch();
        }
    }

    /// `ui.list_next` / `ui.list_prev` (折り返す)。
    pub(crate) fn finder_move(&mut self, forward: bool) -> CommandResult {
        let Some(Overlay::Finder(f)) = &mut self.ui.overlay else {
            return CommandResult::Rejected(crate::command::RejectReason::NotAvailable);
        };
        let n = f.rows.len();
        if n > 0 {
            f.selected = if forward {
                (f.selected + 1) % n
            } else {
                (f.selected + n - 1) % n
            };
        }
        self.mark_dirty_render();
        CommandResult::Done
    }

    /// `ui.list_page_down` / `ui.list_page_up` (端で止まる)。
    pub(crate) fn finder_page(&mut self, forward: bool) -> CommandResult {
        let Some(Overlay::Finder(f)) = &mut self.ui.overlay else {
            return CommandResult::Rejected(crate::command::RejectReason::NotAvailable);
        };
        let n = f.rows.len();
        if n > 0 {
            f.selected = if forward {
                (f.selected + WINDOW).min(n - 1)
            } else {
                f.selected.saturating_sub(WINDOW)
            };
        }
        self.mark_dirty_render();
        CommandResult::Done
    }

    /// `ui.confirm` (`Enter`): 選んだ候補を、オーバーレイを閉じてからフォーカスのペインで開く (`file.open` と同じ経路、
    /// `must_exist`)。行が前の検索語のもの (今の入力欄の照合を待っている) なら、結果が届くまで待ってから開く。
    pub(crate) fn finder_confirm(&mut self) -> CommandResult {
        let root = self.workspace().map(|w| w.root().clone());
        let Some(Overlay::Finder(f)) = &mut self.ui.overlay else {
            return CommandResult::Rejected(crate::command::RejectReason::NotAvailable);
        };
        if f.shown_generation != Some(f.generation) {
            // 行は前の検索語のもの: 照合が走っていれば結果を待って開き、走っていなければ (照合の失敗の後) 何もしない
            if f.match_job.is_some() {
                f.confirm_waiting = true;
            }
            return CommandResult::Done;
        }
        let Some(row) = f.rows.get(f.selected) else {
            // 候補が無い (0 件・ルートを待っている): 閉じない
            return CommandResult::Done;
        };
        let Some(root) = root else {
            return CommandResult::Done;
        };
        let path = row.candidate().path(&root);
        self.take_overlay();
        self.mark_dirty_render();
        // 名前を付けて保存の照合の間は `file.open` と同じく無視する (03 §12.7.6 の細目 (8))
        if self.ignore_while_save_as_check() {
            return CommandResult::Done;
        }
        self.open_fs_path(
            path,
            OpenOptions {
                must_exist: true,
                ..OpenOptions::default()
            },
        )
    }

    /// ファインダのオーバーレイを閉じた (`set_overlay` / `take_overlay` が呼ぶ): 照合を取り消し、行の塊を手放し、閉じた時刻を
    /// 覚える (一覧は持ち続ける)。
    pub(crate) fn finder_closed(&mut self, f: &mut FinderOverlay) {
        if let Some(job) = f.match_job.take() {
            self.jobs.cancel(job);
        }
        self.release_rows(std::mem::take(&mut f.rows));
        self.workspace.finder.closed_at = Some(self.now);
    }

    /// 時刻が進んだ (`Editor::tick`): 最初の走査の間の照合し直しと、閉じてから [`FINDER_LIST_KEEP`] 経った一覧を手放す。
    pub(crate) fn finder_tick(&mut self) {
        self.finder_rematch_if_due();
        let list = &self.workspace.finder;
        let expired = list
            .closed_at
            .is_some_and(|t| self.now >= t + FINDER_LIST_KEEP);
        if expired && !matches!(self.ui.overlay, Some(Overlay::Finder(_))) {
            self.finder_release_list();
        }
    }

    /// 次に `finder_tick` が要る時刻。
    pub(crate) fn finder_deadline(&self) -> Option<Tick> {
        let rematch = match &self.ui.overlay {
            Some(Overlay::Finder(f)) if f.rematch_due => Some(
                f.last_match_at
                    .map_or(self.now, |t| t + FINDER_RESCORE_INTERVAL),
            ),
            _ => None,
        };
        let keep = self
            .workspace
            .finder
            .closed_at
            .map(|t| t + FINDER_LIST_KEEP);
        rematch.into_iter().chain(keep).min()
    }

    /// 一覧を手放す (走っている走査も取り消す)。次に開いたときは最初の走査からやり直す。
    fn finder_release_list(&mut self) {
        let list = &mut self.workspace.finder;
        let scan = list.scan.take();
        let chunks = std::mem::take(&mut list.chunks);
        list.total = 0;
        list.truncated = false;
        list.unreadable = 0;
        list.complete = false;
        list.failed = false;
        list.closed_at = None;
        let mut all = chunks;
        if let Some(s) = scan {
            self.jobs.cancel(s.job);
            all.extend(s.chunks);
        }
        self.release_chunks(all);
    }

    /// 行が持つ塊の参照を手放す。
    fn release_rows(&mut self, rows: Vec<FinderRow>) {
        if rows.is_empty() {
            return;
        }
        let chunks: Chunks = rows.into_iter().map(|r| r.chunk).collect();
        self.release_chunks(chunks);
    }

    /// 塊の参照を手放す (03 §12.7.2 の「主スレッドで大きな一覧を解放しない」)。一覧・走査の途中の塊が持っている塊は、ここで
    /// 外しても最後の参照にならないので外す。それ以外 (差し替えた古い一覧・取り消した走査の束など) は、重複を除いて
    /// `ReleaseCandidates` で `bg` に渡す。
    pub(crate) fn release_chunks(&mut self, chunks: Chunks) {
        let mut out: Chunks = Vec::new();
        for c in chunks {
            if self.workspace.finder.holds(&c) || out.iter().any(|x| Arc::ptr_eq(x, &c)) {
                continue;
            }
            out.push(c);
        }
        if out.is_empty() {
            return;
        }
        // 終端の結果 (`Released`) は待たない (待ちの記録を作らない)
        let _ = self.jobs.spawn(
            None,
            eve_view::Revision::INITIAL,
            JobKind::ReleaseCandidates { chunks: out },
        );
    }

    /// ファインダの一覧のオーバーレイのモデル (04 §8.11)。
    pub(crate) fn finder_model(&self, f: &FinderOverlay) -> OverlayModel {
        let render = |key: &str, args: &[(&str, &str)]| {
            self.catalog().render(key, args, &|c| self.key_label(c))
        };
        let list = &self.workspace.finder;
        let root = self.workspace().map(|w| w.root().display());
        let selected = f.selected.min(f.rows.len().saturating_sub(1));
        let first = window_first(selected);

        let items: Vec<ListItem> = f
            .rows
            .iter()
            .skip(first)
            .take(WINDOW)
            .map(|r| {
                let (label, map) = cell_text_indexed(r.candidate().rel(), CutPolicy::Middle);
                ListItem {
                    label,
                    detail: None,
                    match_ranges: map_ranges(&r.ranges, &map),
                    keys: Vec::new(),
                    disabled_reason: None,
                }
            })
            .collect();
        let shown_root = root.clone().unwrap_or_else(|| {
            if self.term.caps.ascii_only {
                "...".to_owned()
            } else {
                "…".to_owned()
            }
        });
        let title = render(keys::UI_FINDER_TITLE, &[("root", &shown_root)]);
        let footer = if root.is_none() {
            render(keys::UI_FINDER_FOOTER_WAITING, &[])
        } else {
            let matched = f.matched.to_string();
            let total = list.total.to_string();
            let counts = [("matched", matched.as_str()), ("total", total.as_str())];
            let mut s = if list.scanning() {
                render(keys::UI_FINDER_FOOTER_SCANNING, &counts)
            } else if list.failed {
                render(keys::UI_FINDER_FOOTER_INCOMPLETE, &counts)
            } else if list.truncated {
                let limit = WORKSPACE_MAX_FILES.to_string();
                render(
                    keys::UI_FINDER_FOOTER_TRUNCATED,
                    &[counts[0], counts[1], ("limit", &limit)],
                )
            } else {
                render(keys::UI_PALETTE_FOOTER, &counts)
            };
            let unreadable = list
                .scan
                .as_ref()
                .filter(|s| !s.fresh)
                .map_or(list.unreadable, |s| s.unreadable.max(list.unreadable));
            if unreadable > 0 {
                s.push_str(&render(
                    keys::UI_FINDER_UNREADABLE_SUFFIX,
                    &[("dirs", &unreadable.to_string())],
                ));
            }
            s
        };
        let cells = cell_text(&f.text, CutPolicy::End).spans;
        let cursor = cell_text(&f.text[..f.cursor], CutPolicy::End).spans.len();
        let selection = field_selection(f.all_selected, cells.len());
        let placeholder = f
            .text
            .is_empty()
            .then(|| cell_text(&render(keys::UI_FINDER_PLACEHOLDER, &[]), CutPolicy::End));
        OverlayModel::List {
            title: cell_text(&title, CutPolicy::Middle),
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
            no_match: cell_text(&render(keys::UI_FINDER_NO_MATCH, &[]), CutPolicy::End),
        }
    }

    /// ファインダの状態 (試験用): (一覧の候補の数, 走査中か, 出している行の相対パス, 選択中の行)。
    #[doc(hidden)]
    pub fn finder_rows(&self) -> Option<(u64, bool, Vec<String>, usize)> {
        let Some(Overlay::Finder(f)) = &self.ui.overlay else {
            return None;
        };
        let list = &self.workspace.finder;
        Some((
            list.total(),
            list.scanning(),
            f.rows
                .iter()
                .map(|r| r.candidate().rel().to_owned())
                .collect(),
            f.selected,
        ))
    }

    /// 一覧を持っている (閉じた後も持つ。手放した後は `false`。試験用)。
    #[doc(hidden)]
    pub fn finder_list_held(&self) -> bool {
        let list = &self.workspace.finder;
        list.complete || !list.chunks.is_empty() || list.scan.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 半開区間 (単一の範囲の配列を書くための補助)。
    fn r<T>(start: T, end: T) -> Range<T> {
        start..end
    }

    #[test]
    fn 書記素の区間を_span_の番号に写す() {
        // 書記素 1 が幅 0 で落ちた (Span が無い) とき、2 以降は 1 つずれる
        let map = [Some(0), None, Some(1), Some(2), Some(3)];
        assert_eq!(map_ranges(&[r(0, 3)], &map), [r(0, 2)]);
        assert_eq!(map_ranges(&[r(3, 5)], &map), [r(2, 4)]);
        assert_eq!(map_ranges(&[0..1, 3..4], &map), [0..1, 2..3]);
        assert_eq!(map_ranges(&[r(1, 2)], &map), Vec::<Range<usize>>::new());
        assert_eq!(
            map_ranges(&[r(9, 10)], &map),
            Vec::<Range<usize>>::new(),
            "範囲の外は捨てる"
        );
    }
}
