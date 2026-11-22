//! 横断検索のジョブ `JobKind::WorkspaceSearch` (docs/03 §12.7.5 / §12.7.6 の 3 / §12.7.7、01 FR-405)。
//!
//! [`WorkspaceWalk`] でファイルを見つけ、見つけた順に 1 つずつ探す。1 区切りは「ディレクトリ 16 個 (走査の 1 区切り)・
//! 探したファイル 64 個・探した中身の合計 1 MiB・区切りの始めから 8 ms」のどれか早いほうで、続きを `JobStep::Continue` で
//! 返す (1 つのファイルは途中で区切らない)。各ファイルは読込と同じ規則で判定して復号し (`decode_for_search`)、エディタ内の
//! 検索の照合器 (`Matcher`) で探す。開いている文書と同じファイルは、ディスクではなく検索を始めた時点の Snapshot を探す。
//!
//! spec: 03 §12.7.5, 03 §12.7.6, 03 §12.7.7, 01 FR-405

use std::collections::VecDeque;
use std::fmt;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

use eve_text::{ByteOff, ByteRange, LineIdx, Snapshot, TextRead, is_grapheme_boundary};
use eve_view::JobId;

use crate::io::fs::{FileId, FileSystem};
use crate::io::fspath::FsPath;
use crate::io::io_jobs::{SearchDecoded, decode_for_search};
use crate::io::same_file::{CanonicalSource, IdentityRef, canonical_key, canonical_of};
use crate::jobs::cancel::CancelToken;
use crate::jobs::{JobError, JobKind, JobOutput, JobPorts, JobResult, JobStep};
use crate::search::{Matcher, ScanCtx, ScanEnd, SearchQuery, scan_range};
use crate::time::{Clock, Millis, Tick};

use super::{Candidate, EntryFacts, ExcludeConfig, WALK_STEP_TIME, WalkStep, WorkspaceWalk};

/// 結果の行の上限 (`WS_MAX_RESULT_LINES`。`search.max_matches` の既定と同じ桁)。達したら走査もやめて打ち切りの印を立てる。
pub const WS_MAX_RESULT_LINES: usize = 10_000;
/// 探すファイルの大きさの上限の既定 (`search.workspace.max_file_size` の既定、07 §5.5。設定のキーは予約のまま)。
pub const WS_MAX_FILE_SIZE: u64 = 2 * 1024 * 1024;
/// 1 つのファイルの読込の上限 (`max_file_size` に依らない。復号で膨らんだ 1 ファイルで `bg` が数 GB を持たない)。
pub const WS_MAX_DECODE_BYTES: u64 = 64 * 1024 * 1024;
/// 1 区切りで探すファイルの数。
pub const WS_FILES_PER_STEP: usize = 64;
/// 1 区切りで探す中身の合計 (バイト)。
pub const WS_BYTES_PER_STEP: u64 = 1024 * 1024;
/// 1 行の結果に持つ一致の範囲の数 (抜粋の中のもの)。
pub const WS_MAX_RANGES_PER_LINE: usize = 16;
/// 抜粋の長さの上限 (バイト。最初の一致の前後を合わせて)。
pub const WS_EXCERPT_BYTES: usize = 256;
/// 抜粋で、最初の一致の前に置く長さの上限 (バイト)。残りは一致とその後ろに使う。
pub const WS_EXCERPT_BEFORE: usize = 64;
/// 一致の無い区切りが続くとき、進捗 (探したファイルの数) を送る間隔。
pub const WS_PROGRESS_INTERVAL: Millis = Millis(100);

/// 横断検索の結果 1 行 (03 §12.7.5 の「結果」)。同じ行の一致は 1 件にまとめる。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WsHit {
    /// ファイルの絶対パス (ルート + 相対パス。開くときに使う)。
    pub path: FsPath,
    /// ルートからの相対パス (区切りは `/`。UTF-8 で表せない部分は U+FFFD)。同じファイルの行で共有する。
    pub rel: Arc<str>,
    /// 行 (0 始まり)。
    pub line: u32,
    /// 一致の範囲 (その行の中のバイト位置。復号した UTF-8 の行。行をまたぐ一致は行末まで)。抜粋に掛かるものを
    /// [`WS_MAX_RANGES_PER_LINE`] 個まで。最初の要素はその行の最初の一致 (ジャンプの位置)。
    pub ranges: Vec<Range<u32>>,
    /// 行の抜粋 (最初の一致の前後を合わせて [`WS_EXCERPT_BYTES`] まで。書記素の境界で切り、先頭の空白は詰める)。
    pub excerpt: Box<str>,
    /// 抜粋の始まりの、行の中のバイト位置 (書記素の境界)。
    pub excerpt_start: u32,
    /// 開いている文書の Snapshot を探した結果。
    pub from_open_doc: bool,
}

/// 横断検索の進捗 (03 §12.7.5 の「結果」)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WsProgress {
    /// 探したファイル (開いている文書を含む)。
    pub files: u64,
    /// 一致のあったファイル。
    pub hit_files: u64,
    /// 大きさの上限を超えて飛ばしたファイル。
    pub skipped_large: u64,
    /// バイナリ (と対応していない BOM) で飛ばしたファイル。
    pub skipped_binary: u64,
    /// 読めない・オフライン・最悪の入力の上限で打ち切ったファイル。
    pub skipped_other: u64,
    /// 読めなかったディレクトリ。
    pub unreadable_dirs: u64,
}

impl WsProgress {
    /// 飛ばしたファイルの合計。
    #[must_use]
    pub fn skipped(&self) -> u64 {
        self.skipped_large + self.skipped_binary + self.skipped_other
    }
}

/// 開いている文書 1 つ (03 §12.7.6 の 3 の突き合わせに使う)。主スレッドが、ルートの下にある文書だけを渡す。
#[derive(Clone, Debug)]
pub struct OpenDocRef {
    /// 文書のパス (古い識別を確かめるときに `metadata` し直す)。
    pub path: FsPath,
    /// 文書の canonical のパス (取れなかった文書は正規化したパス)。
    pub canonical: FsPath,
    /// 文書の識別 (`disk_stamp` の値)。
    pub file_id: Option<FileId>,
    /// 識別を判定に使ってよい。
    pub file_id_unique: bool,
    /// 探す中身。
    pub body: OpenDocBody,
}

/// 開いている文書の、横断検索で探す中身。
#[derive(Clone, Debug)]
pub enum OpenDocBody {
    /// 検索を始めた時点の本文 (保存していない編集を含む)。
    Text(Snapshot),
    /// 本文の UTF-8 のバイト数が上限を超える (飛ばす。ディスクも探さない)。
    TooLarge,
    /// バイナリとして読み取り専用で開いた文書 (飛ばす)。
    Binary,
}

/// 横断検索の続き (積み直しの間に持つ状態)。
#[derive(Clone)]
pub struct WorkspaceSearchState {
    walk: WorkspaceWalk,
    queue: VecDeque<(Candidate, EntryFacts)>,
    progress: WsProgress,
    /// 送った結果の行の数。
    lines: usize,
    /// 開いている文書の canonical のパスの比べる形 (`open_docs` と同じ順)。
    doc_keys: Vec<PathBuf>,
    /// 前に結果か進捗を送った時刻。
    last_sent: Option<Tick>,
    /// 前に送った進捗。
    sent: WsProgress,
    /// 読むためのルート (Windows は verbatim の形。区切りのたびに作り直さない)。
    read_root: FsPath,
}

impl fmt::Debug for WorkspaceSearchState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkspaceSearchState")
            .field("walk", &self.walk)
            .field("queue", &self.queue.len())
            .field("progress", &self.progress)
            .field("lines", &self.lines)
            .finish_non_exhaustive()
    }
}

/// 横断検索のジョブの入力 (`JobKind::WorkspaceSearch` の中身)。
pub(crate) struct WsSearchJob {
    pub root: FsPath,
    pub generation: u64,
    pub query: SearchQuery,
    pub exclude: Arc<ExcludeConfig>,
    pub open_docs: Vec<OpenDocRef>,
    pub max_file_size: u64,
    pub max_lines: usize,
    pub state: Option<Box<WorkspaceSearchState>>,
}

/// 1 区切りを進める。区切りの間の結果を `WorkspaceHits` で送り、続きがあれば `JobStep::Continue` を返す。終われば
/// `WorkspaceSearchDone`、ルートを読めなければ `Failed { Fs }`、取り消されたら `Failed { Cancelled }`。
pub(crate) fn run_workspace_search(
    job: JobId,
    p: WsSearchJob,
    ports: &JobPorts,
    cancel: &CancelToken,
    emit: &mut dyn FnMut(JobOutput),
) -> JobStep {
    let fs = &*ports.fs;
    let clock = &*ports.clock;
    let start = clock.now();
    let mut st = p.state.unwrap_or_else(|| {
        Box::new(WorkspaceSearchState {
            walk: WorkspaceWalk::new(p.root.clone(), p.exclude.clone()).with_entry_facts(),
            queue: VecDeque::new(),
            progress: WsProgress::default(),
            lines: 0,
            doc_keys: p
                .open_docs
                .iter()
                .map(|d| canonical_key(&d.canonical))
                .collect(),
            last_sent: None,
            sent: WsProgress::default(),
            read_root: verbatim_root(&p.root),
        })
    });
    let fail = |emit: &mut dyn FnMut(JobOutput), err| {
        emit(JobOutput {
            job,
            body: JobResult::Failed { err },
        });
        JobStep::Done
    };
    // 照合器はここ (`bg`) で作る。作れない (空・誤り) ときは何も探さずに終える (誤りは主スレッドが `known_error` で示す)
    let Some(matcher) = p.query.matcher() else {
        emit(JobOutput {
            job,
            body: JobResult::WorkspaceSearchDone {
                generation: p.generation,
                progress: st.progress,
                truncated: false,
            },
        });
        return JobStep::Done;
    };
    let limit = p.max_file_size.min(WS_MAX_DECODE_BYTES);
    let ctx = FileCtx {
        fs,
        clock,
        cancel,
        root: &p.root,
        read_root: &st.read_root,
        matcher: &matcher,
        open_docs: &p.open_docs,
        doc_keys: &st.doc_keys,
        limit,
    };
    let mut hits: Vec<WsHit> = Vec::new();
    let (mut files, mut bytes) = (0usize, 0u64);
    let mut walked = false;
    let mut capped = false;
    // 区切りの最初の 1 回 (走査かファイル 1 つ) は上限を見ずに進める (どの区切りも少しは進む)
    let mut first = true;
    loop {
        if cancel.is_cancelled() {
            return fail(emit, JobError::Cancelled);
        }
        let elapsed = clock.now().saturating_sub(start);
        let over =
            files >= WS_FILES_PER_STEP || bytes >= WS_BYTES_PER_STEP || elapsed >= WALK_STEP_TIME;
        if !first && over {
            break;
        }
        first = false;
        if st.queue.is_empty() {
            // 走査は 1 区切りに 1 回まで。ファイルを探した残りの時間で走査する (区切り全体で 8 ms)
            if walked || st.walk.is_finished() {
                break;
            }
            walked = true;
            let budget = Millis(WALK_STEP_TIME.0.saturating_sub(elapsed.0));
            if let WalkStep::Failed(e) = st.walk.step_within(fs, clock, budget) {
                return fail(emit, JobError::Fs(e));
            }
            let found = st.walk.take_found();
            st.queue.extend(found);
            st.progress.unreadable_dirs = st.walk.unreadable_dirs();
            continue;
        }
        let Some((cand, facts)) = st.queue.pop_front() else {
            break;
        };
        let remaining = p.max_lines.saturating_sub(st.lines);
        let before = hits.len();
        match search_candidate(&ctx, &cand, facts, remaining, &mut hits) {
            Outcome::Searched { size, capped: c } => {
                files += 1;
                bytes = bytes.saturating_add(size);
                st.progress.files += 1;
                if hits.len() > before {
                    st.progress.hit_files += 1;
                }
                st.lines += hits.len() - before;
                // 上限を「超える」行を見つけたときだけ打ち切る (ちょうど上限の行数で終わる検索は打ち切りの印を立てない)
                if c {
                    capped = true;
                    break;
                }
            }
            Outcome::Skipped(kind, read) => {
                files += 1;
                // 飛ばす前に読んだ量も 1 区切りの中身の合計に数える (上限 + 1 バイトまで読んで飛ばすファイルが続いても区切る)
                bytes = bytes.saturating_add(read);
                match kind {
                    Skip::Large => st.progress.skipped_large += 1,
                    Skip::Binary => st.progress.skipped_binary += 1,
                    Skip::Other => st.progress.skipped_other += 1,
                }
            }
            Outcome::NotCounted => {}
            Outcome::Cancelled => return fail(emit, JobError::Cancelled),
        }
    }
    let finished = capped || (st.queue.is_empty() && st.walk.is_finished());
    let now = clock.now();
    let due = st
        .last_sent
        .is_none_or(|t| now.saturating_sub(t) >= WS_PROGRESS_INTERVAL);
    if !hits.is_empty() || (!finished && due && st.progress != st.sent) {
        st.last_sent = Some(now);
        st.sent = st.progress;
        emit(JobOutput {
            job,
            body: JobResult::WorkspaceHits {
                generation: p.generation,
                hits,
                progress: st.progress,
            },
        });
    }
    if finished {
        emit(JobOutput {
            job,
            body: JobResult::WorkspaceSearchDone {
                generation: p.generation,
                progress: st.progress,
                truncated: capped || st.walk.truncated(),
            },
        });
        return JobStep::Done;
    }
    JobStep::Continue(JobKind::WorkspaceSearch {
        root: p.root,
        generation: p.generation,
        query: p.query,
        exclude: p.exclude,
        open_docs: p.open_docs,
        max_file_size: p.max_file_size,
        max_lines: p.max_lines,
        state: Some(st),
    })
}

/// 1 つのファイルの探し方の文脈。
struct FileCtx<'a> {
    fs: &'a dyn FileSystem,
    clock: &'a dyn Clock,
    cancel: &'a CancelToken,
    root: &'a FsPath,
    /// 読むためのルート (Windows は verbatim の形)。
    read_root: &'a FsPath,
    matcher: &'a Matcher,
    open_docs: &'a [OpenDocRef],
    doc_keys: &'a [PathBuf],
    /// 読む大きさの上限 (`max_file_size` と [`WS_MAX_DECODE_BYTES`] の小さいほう)。
    limit: u64,
}

/// 1 つのファイルの終わり方。
enum Outcome {
    /// 探した (`size` は探した中身のバイト数。`capped` は結果の行の上限に達した)。
    Searched { size: u64, capped: bool },
    /// 飛ばした (2 つ目は飛ばすまでに読んだバイト数)。
    Skipped(Skip, u64),
    /// 数えない (辿らないシンボリックリンク)。
    NotCounted,
    /// 取り消された。
    Cancelled,
}

/// 飛ばした理由。
#[derive(Clone, Copy)]
enum Skip {
    Large,
    Binary,
    Other,
}

/// 候補 1 つを探す (03 §12.7.5 の「1 つのファイルの扱い」)。
fn search_candidate(
    ctx: &FileCtx<'_>,
    cand: &Candidate,
    facts: EntryFacts,
    remaining: usize,
    out: &mut Vec<WsHit>,
) -> Outcome {
    if facts.unfollowed_link {
        return Outcome::NotCounted;
    }
    let path = cand.path(ctx.root);
    // 1. 開いている文書: ディスクではなく Snapshot を探す
    if let Some(doc) = open_doc_for(ctx, &path) {
        return match &doc.body {
            OpenDocBody::Text(s) => search_text(ctx, s, &path, cand, true, remaining, out),
            OpenDocBody::TooLarge => Outcome::Skipped(Skip::Large, 0),
            OpenDocBody::Binary => Outcome::Skipped(Skip::Binary, 0),
        };
    }
    // 2. 大きさとオフライン (開かずに飛ばす)
    if facts.offline {
        return Outcome::Skipped(Skip::Other, 0);
    }
    if facts.len.is_some_and(|n| n > ctx.limit) {
        return Outcome::Skipped(Skip::Large, 0);
    }
    // 3. 判定と復号
    let chunks = match read_limited(ctx, &cand.path(ctx.read_root)) {
        Read::Data(c) => c,
        Read::TooLarge(n) => return Outcome::Skipped(Skip::Large, n),
        Read::Failed(n) => return Outcome::Skipped(Skip::Other, n),
        Read::Cancelled => return Outcome::Cancelled,
    };
    let read: u64 = chunks.iter().map(|c| c.len() as u64).sum();
    let text = match decode_for_search(&chunks, ctx.cancel) {
        SearchDecoded::Text(t) => t,
        SearchDecoded::Binary => return Outcome::Skipped(Skip::Binary, read),
        SearchDecoded::Cancelled => return Outcome::Cancelled,
    };
    drop(chunks);
    // 4. 照合
    search_text(ctx, &text, &path, cand, false, remaining, out)
}

/// 読むためのルート: Windows のドライブ文字・UNC の絶対パスは verbatim の形にする (そこに名前を足すので、末尾のドット・空白・
/// 予約名の名前を、別のファイルやデバイスに読み替えない、03 §12.7.5 の 3)。ほかはそのまま。ジョブの状態に 1 回だけ作る。
fn verbatim_root(root: &FsPath) -> FsPath {
    if !cfg!(windows) {
        return root.clone();
    }
    let Some(s) = root.as_path().to_str() else {
        return root.clone();
    };
    let b = s.as_bytes();
    if b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\' {
        FsPath::from(format!(r"\\?\{s}").as_str())
    } else if let Some(unc) = s.strip_prefix(r"\\").filter(|r| !r.starts_with(['?', '.'])) {
        FsPath::from(format!(r"\\?\UNC\{unc}").as_str())
    } else {
        root.clone()
    }
}

/// 読込の結果 (`TooLarge` / `Failed` は、それまでに読んだバイト数)。
enum Read {
    Data(Vec<Vec<u8>>),
    TooLarge(u64),
    Failed(u64),
    Cancelled,
}

/// ファイルを上限 + 1 バイトまで読む (大きさが列挙で分からないとき、`metadata` を 1 件ずつ呼ばずに読んで確かめる)。
fn read_limited(ctx: &FileCtx<'_>, path: &FsPath) -> Read {
    let Ok(mut reader) = ctx.fs.open_read(path) else {
        return Read::Failed(0);
    };
    let mut chunks: Vec<Vec<u8>> = Vec::new();
    let mut total = 0u64;
    loop {
        if ctx.cancel.is_cancelled() {
            return Read::Cancelled;
        }
        match reader.next_chunk() {
            Ok(Some(c)) => {
                total = total.saturating_add(c.len() as u64);
                if total > ctx.limit {
                    return Read::TooLarge(total);
                }
                chunks.push(c.to_vec());
            }
            Ok(None) => return Read::Data(chunks),
            Err(_) => return Read::Failed(total),
        }
    }
}

/// 候補と同じファイルの開いている文書 (03 §12.7.6 の 3)。(i) 文書の canonical と、ルート + 相対パスを比べる。(ii) 一致
/// しない文書のうちファイル名が同じものがあるときだけ、候補に `metadata` を呼んで §12.7.6 の 0 の比べ方で確かめる
/// (文書の識別が古ければ識別では比べない)。
fn open_doc_for<'d>(ctx: &FileCtx<'d>, path: &FsPath) -> Option<&'d OpenDocRef> {
    if ctx.open_docs.is_empty() {
        return None;
    }
    let key = canonical_key(path);
    if let Some(i) = ctx.doc_keys.iter().position(|k| *k == key) {
        return ctx.open_docs.get(i);
    }
    let name = path.as_path().file_name()?;
    // (ii) は識別で比べるためのもの: 信頼できる識別の無い文書は (i) の canonical の比べ方だけにする。識別を信頼できる FS
    // (NTFS など) では、開いている文書と同じ名前の候補ごとに `metadata` と canonicalize を呼ぶ (今これが当たるのは、ルートの
    // 下の別の場所にある同じ名前のハードリンクだけ。03 §12.7.6 の 3)
    let same_name: Vec<&OpenDocRef> = ctx
        .open_docs
        .iter()
        .filter(|d| d.file_id_unique && d.file_id.is_some())
        .filter(|d| {
            d.canonical
                .as_path()
                .file_name()
                .is_some_and(|n| same_file_name(n, name))
        })
        .collect();
    if same_name.is_empty() {
        return None;
    }
    let meta = ctx.fs.metadata(path).ok()?;
    let (canonical, source) = canonical_of(ctx.fs, path);
    let file_id_unique = meta.file_id_unique && source == CanonicalSource::Direct;
    let mine = IdentityRef {
        file_id: meta.file_id,
        file_id_unique,
        canonical: canonical.as_ref(),
    };
    same_name.into_iter().find(|d| {
        // 古い識別の確かめ: 識別が候補と等しい文書は、文書のパスを `metadata` し直し、今も同じ識別でなければ識別では比べない
        let trusted = d.file_id_unique
            && match (d.file_id, meta.file_id) {
                (Some(a), Some(b)) if a == b => {
                    ctx.fs.metadata(&d.path).is_ok_and(|m| m.file_id == Some(a))
                }
                _ => true,
            };
        let theirs = IdentityRef {
            file_id: d.file_id,
            file_id_unique: trusted,
            canonical: Some(&d.canonical),
        };
        mine.same_file(theirs)
    })
}

/// ファイル名が同じか (Windows・macOS は ASCII の大文字小文字を畳む)。
fn same_file_name(a: &std::ffi::OsStr, b: &std::ffi::OsStr) -> bool {
    let (a, b) = (a.as_encoded_bytes(), b.as_encoded_bytes());
    if cfg!(any(windows, target_os = "macos")) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// 本文 `text` を探し、一致を行ごとにまとめて `out` に足す (03 §12.7.5 の 4 と「結果」)。最悪の入力の上限で打ち切ったら、
/// そのファイルの結果を捨てて飛ばした数に数える。
fn search_text(
    ctx: &FileCtx<'_>,
    text: &Snapshot,
    path: &FsPath,
    cand: &Candidate,
    from_open_doc: bool,
    remaining: usize,
    out: &mut Vec<WsHit>,
) -> Outcome {
    // 相対パスの共有の文字列は、最初の一致の行を作るときに 1 回だけ作る (一致の無いファイルでは作らない)
    let mut rel: Option<Arc<str>> = None;
    let mut rel_of = || rel.get_or_insert_with(|| Arc::from(cand.rel())).clone();
    let len = text.len_bytes();
    if ctx.matcher.too_large_for(len) {
        return Outcome::Skipped(Skip::Other, len as u64);
    }
    let start_len = out.len();
    let mut group: Option<LineGroup> = None;
    let mut groups = 0usize;
    let mut capped = false;
    let mut scan_ctx = ScanCtx {
        cancel: ctx.cancel,
        clock: Some(ctx.clock),
        resume: None,
    };
    let end = scan_range(
        text,
        ctx.matcher,
        ByteRange::new(ByteOff(0), ByteOff(len)),
        &mut scan_ctx,
        &mut |r| {
            let line = text.byte_to_line(r.start).0;
            if group.as_ref().is_none_or(|g| g.line != line) {
                if let Some(g) = group.take() {
                    out.push(g.finish(text, path, rel_of(), from_open_doc));
                }
                if groups >= remaining {
                    capped = true;
                    return false;
                }
                groups += 1;
                group = Some(LineGroup::new(text, line, r));
            } else if let Some(g) = group.as_mut() {
                g.add(r);
            }
            true
        },
    );
    match end {
        ScanEnd::Done => {}
        ScanEnd::Stopped if capped => {}
        ScanEnd::Cancelled | ScanEnd::Stopped => {
            out.truncate(start_len);
            return Outcome::Cancelled;
        }
        ScanEnd::TooLarge | ScanEnd::TooHeavy => {
            out.truncate(start_len);
            return Outcome::Skipped(Skip::Other, len as u64);
        }
    }
    if let Some(g) = group.take() {
        out.push(g.finish(text, path, rel_of(), from_open_doc));
    }
    Outcome::Searched {
        size: len as u64,
        capped,
    }
}

/// 1 行の一致を集める途中の状態。
struct LineGroup {
    line: usize,
    /// 行の始まりと内容の終わり (文書のバイト位置)。
    line_start: usize,
    line_end: usize,
    /// 抜粋の窓 (行の中のバイト位置。最初の一致で決める)。
    window: Range<usize>,
    ranges: Vec<Range<u32>>,
}

impl LineGroup {
    /// 行 `line` の最初の一致 `first` から始める。
    fn new(text: &Snapshot, line: usize, first: ByteRange) -> LineGroup {
        let line_start = text.line_to_byte(LineIdx(line)).0;
        let line_end = text.line_end_byte(LineIdx(line)).0.max(line_start);
        let line_len = line_end - line_start;
        let fs = first.start.0.saturating_sub(line_start).min(line_len);
        // 一致の前は最大 WS_EXCERPT_BEFORE。行の終わりに近ければ、前を広げて 256 バイトを使う
        let mut ws = fs.saturating_sub(WS_EXCERPT_BEFORE);
        if line_len - ws < WS_EXCERPT_BYTES {
            ws = line_len.saturating_sub(WS_EXCERPT_BYTES).min(fs);
        }
        // 書記素の境界に揃える (始まりは後ろへ、終わりは前へ。結合文字・ZWJ の並び・国旗の途中で切ると、一覧の行で前の文言の
        // 文字と 1 つの書記素になり、強調と可視の記法がずれる)
        let boundary = |n: usize| is_grapheme_boundary(text, ByteOff(line_start + n));
        while ws < fs && !boundary(ws) {
            ws += 1;
        }
        let mut we = (ws + WS_EXCERPT_BYTES).min(line_len);
        while we > ws && !boundary(we) {
            we -= 1;
        }
        let mut g = LineGroup {
            line,
            line_start,
            line_end,
            window: ws..we,
            ranges: Vec::new(),
        };
        g.push_range(first);
        g
    }

    /// 同じ行の一致を足す (抜粋の窓に掛かるものだけ、[`WS_MAX_RANGES_PER_LINE`] 個まで)。
    fn add(&mut self, r: ByteRange) {
        if self.ranges.len() >= WS_MAX_RANGES_PER_LINE {
            return;
        }
        let start = r.start.0.saturating_sub(self.line_start);
        if start >= self.window.end && !self.ranges.is_empty() {
            return;
        }
        self.push_range(r);
    }

    fn push_range(&mut self, r: ByteRange) {
        let line_len = self.line_end - self.line_start;
        let s = r.start.0.saturating_sub(self.line_start).min(line_len);
        let e = r
            .end
            .0
            .min(self.line_end)
            .saturating_sub(self.line_start)
            .max(s);
        let to32 = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        self.ranges.push(to32(s)..to32(e));
    }

    /// 結果の行にする。抜粋の先頭の空白は、最初の一致の手前まで詰める。
    fn finish(self, text: &Snapshot, path: &FsPath, rel: Arc<str>, from_open_doc: bool) -> WsHit {
        let ws = self.window.start;
        let raw = text.slice_to_string(ByteRange::new(
            ByteOff(self.line_start + ws),
            ByteOff(self.line_start + self.window.end),
        ));
        let first = self.ranges.first().map_or(ws, |r| r.start as usize);
        let lead = (raw.len() - raw.trim_start().len()).min(first.saturating_sub(ws));
        let excerpt: Box<str> = raw.get(lead..).unwrap_or_default().into();
        let to32 = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        WsHit {
            path: path.clone(),
            rel,
            line: to32(self.line),
            ranges: self.ranges,
            excerpt,
            excerpt_start: to32(ws + lead),
            from_open_doc,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 半開区間 (要素 1 つの範囲の配列を書くための補助)。
    fn r(start: u32, end: u32) -> Range<u32> {
        start..end
    }

    fn group_of(s: &str, at: Range<usize>) -> WsHit {
        let t = Snapshot::from(s);
        let r = ByteRange::new(ByteOff(at.start), ByteOff(at.end));
        let line = t.byte_to_line(r.start).0;
        let rel: Arc<str> = Arc::from("a.txt");
        LineGroup::new(&t, line, r).finish(&t, &FsPath::from("/w/a.txt"), rel, false)
    }

    #[test]
    fn 短い行は行全体を抜粋にし_先頭の空白を詰める() {
        let h = group_of("x\n    let foo = 1;\n", 10..13);
        assert_eq!(h.line, 1);
        assert_eq!(&*h.excerpt, "let foo = 1;");
        assert_eq!(h.excerpt_start, 4);
        assert_eq!(
            h.ranges,
            [r(8, 11)],
            "範囲は行の中の位置 (foo は行の 8 バイト目から)"
        );
    }

    #[test]
    fn 空白の一致の手前までしか詰めない() {
        let h = group_of("   x", 1..2);
        assert_eq!(h.excerpt_start, 1);
        assert_eq!(&*h.excerpt, "  x");
    }

    #[test]
    fn 長い行は一致の前後_256_バイトに切り_文字の境界に揃える() {
        let line = format!("{}needle{}", "あ".repeat(100), "い".repeat(100));
        let at = 300..306;
        let h = group_of(&line, at);
        assert!(h.excerpt.len() <= WS_EXCERPT_BYTES, "{}", h.excerpt.len());
        assert!(h.excerpt.contains("needle"));
        let start = h.excerpt_start as usize;
        assert!(line.is_char_boundary(start));
        assert!(
            300 - start <= WS_EXCERPT_BEFORE,
            "前は 64 バイトまで: {start}"
        );
        assert_eq!(&line[start..start + h.excerpt.len()], &*h.excerpt);
    }

    #[test]
    fn 行末に近い一致は前を広げる() {
        let line = format!("{}needle", "a".repeat(400));
        let h = group_of(&line, 400..406);
        assert_eq!(h.excerpt.len(), WS_EXCERPT_BYTES);
        assert!(h.excerpt.ends_with("needle"));
    }

    #[test]
    fn 行をまたぐ一致は行末までにする() {
        let h = group_of("ab\ncd\n", 1..4);
        assert_eq!(h.line, 0);
        assert_eq!(h.ranges, [r(1, 2)]);
    }

    #[test]
    fn 窓の外の一致は持たず_16_個までにする() {
        let line = "x ".repeat(400);
        let t = Snapshot::from(line.as_str());
        let mut g = LineGroup::new(&t, 0, ByteRange::new(ByteOff(0), ByteOff(1)));
        for i in 1..400 {
            g.add(ByteRange::new(ByteOff(i * 2), ByteOff(i * 2 + 1)));
        }
        assert_eq!(g.ranges.len(), WS_MAX_RANGES_PER_LINE);
        let mut g = LineGroup::new(&t, 0, ByteRange::new(ByteOff(0), ByteOff(1)));
        g.add(ByteRange::new(ByteOff(700), ByteOff(701)));
        assert_eq!(g.ranges.len(), 1, "窓 (256 バイト) の外は持たない");
    }

    #[test]
    fn 同じファイル名の比べ方() {
        use std::ffi::OsStr;
        assert!(same_file_name(OsStr::new("a.txt"), OsStr::new("a.txt")));
        assert_eq!(
            same_file_name(OsStr::new("A.TXT"), OsStr::new("a.txt")),
            cfg!(any(windows, target_os = "macos"))
        );
    }

    #[test]
    fn verbatim_のルート() {
        let r = verbatim_root(&FsPath::from(r"C:\w"));
        if cfg!(windows) {
            assert_eq!(r.as_path().to_str(), Some(r"\\?\C:\w"));
            assert_eq!(
                verbatim_root(&FsPath::from(r"\\srv\sh\w"))
                    .as_path()
                    .to_str(),
                Some(r"\\?\UNC\srv\sh\w")
            );
            assert_eq!(
                verbatim_root(&FsPath::from(r"\\?\C:\w")).as_path().to_str(),
                Some(r"\\?\C:\w")
            );
        } else {
            assert_eq!(r.as_path().to_str(), Some(r"C:\w"));
        }
        assert_eq!(
            verbatim_root(&FsPath::from("/w")).as_path().to_str(),
            Some("/w")
        );
    }
}
