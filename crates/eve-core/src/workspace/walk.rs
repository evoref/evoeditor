//! ワークスペースの走査の状態 `WorkspaceWalk` (docs/03 §12.7.2)。
//!
//! 深さ優先で、各ディレクトリの項目を名前のバイト列の順に並べてから処理する。1 回の [`WorkspaceWalk::step`] (1 区切り) は、
//! ディレクトリを [`WALK_DIRS_PER_STEP`] 個読むか、区切りの始めから [`WALK_STEP_TIME`] を過ぎるか、項目を
//! [`WALK_ENTRIES_PER_STEP`] 個処理するかの早いほうで戻る (続きはジョブが自分のキューの末尾へ積み直す。02 §9.3 の
//! `JobStep::Continue`)。I/O は `FileSystem` の `read_dir`・`open_read`・`metadata`・`canonicalize` だけを使う。

use std::collections::HashSet;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use crate::gitignore::{ExcludeChain, IgnoreRules, PathKind};
use crate::io::fs::{DirEntry, EntryKind, FileId, FileKind, FileSystem, FsError};
use crate::io::fspath::FsPath;
use crate::time::{Clock, Tick};

use super::candidate::Candidate;
use super::git::{
    DotGit, find_repo_upward, is_dot_git, is_gitignore, load_repo, read_ancestor_gitignore,
    read_gitignore, rel_bytes,
};
use super::{
    CANDIDATE_CHUNK, ExcludeConfig, SCAN_FLUSH_INTERVAL, WALK_DIRS_PER_STEP,
    WALK_ENTRIES_PER_DIR_MAX, WALK_ENTRIES_PER_STEP, WALK_STEP_TIME, WORKSPACE_MAX_DEPTH,
    WORKSPACE_MAX_ENTRIES, WORKSPACE_MAX_FILES,
};

/// Linux でルートが `/` のときに降りない疑似 FS (03 §12.7.2 の 5)。
const LINUX_PSEUDO_FS: &[&[u8]] = &[b"proc", b"sys", b"dev", b"run"];

/// 走査の途中で、候補にならなかった項目と降りたディレクトリを知らせる口 (git との差分試験が、除外した項目・ディレクトリ・
/// 入れ子のリポジトリを集めるのに使う)。
/// 既定はどれも何もしない。`rel` はワークスペースのルートからの相対パス (区切りは `/`)。
pub trait WalkObserver {
    /// 除外に当たって候補にしなかった (ディレクトリなら降りなかった) 項目。
    fn excluded(&mut self, rel: &[u8], kind: PathKind) {
        let _ = (rel, kind);
    }

    /// 除外に当たらないディレクトリに降りる (積んだ時点で知らせる。読めないディレクトリも含む)。
    fn entered_dir(&mut self, rel: &[u8]) {
        let _ = rel;
    }

    /// 入れ子のリポジトリの境目のディレクトリに入った (中は内側のリポジトリの規則で照らす)。
    fn nested_repo(&mut self, rel: &[u8]) {
        let _ = rel;
    }
}

/// 何もしない [`WalkObserver`]。
#[derive(Debug, Default, Clone, Copy)]
pub struct NoObserver;

impl WalkObserver for NoObserver {}

/// 1 区切りの終わり方。
#[derive(Debug)]
pub enum WalkStep {
    /// 区切りの上限に達した。続きがある。
    Yield,
    /// 走査が終わった (上限による打ち切りを含む。[`WorkspaceWalk::truncated`])。
    Done,
    /// ルートそのものを読めなかった (`msg.workspace.scan_failed`、03 §12.7.2 の 7)。
    Failed(FsError),
}

/// 降りたディレクトリを 2 度辿らないための鍵 (`follow_symlinks = true` のとき。`FileId` が取れない FS は canonicalize したパス)。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum VisitKey {
    Id(FileId),
    Path(FsPath),
}

/// 未処理のディレクトリ 1 つ。
#[derive(Clone)]
struct Frame {
    abs: FsPath,
    /// 走査の最上位 (リポジトリのルートがワークスペースのルートより上ならリポジトリのルート) からの相対パス。照合に使う。
    mrel: Vec<u8>,
    /// ワークスペースのルートからの相対パスの OS の形 (名前が UTF-8 で表せない候補の元の形を作るのに使う)。
    os_rel: PathBuf,
    depth: usize,
    /// このディレクトリの項目に効く除外の連鎖 (列挙の後は、このディレクトリの `.gitignore` と入れ子のリポジトリを含む)。
    chain: ExcludeChain,
    /// 並べた項目の残り。読む前は `None`。
    listing: Option<std::vec::IntoIter<DirEntry>>,
}

/// ワークスペースの走査の状態 (03 §12.7.2)。ファインダの走査 (`JobKind::WorkspaceScan`) と横断検索が使う。
///
/// 状態は、未処理のディレクトリのスタック (各ディレクトリの除外の連鎖を `Arc` で持つ)、見つけた件数、打ち切りの印、読めなかった
/// ディレクトリの数と、まだ塊にしていない候補。
#[derive(Clone)]
pub struct WorkspaceWalk {
    root: FsPath,
    config: Arc<ExcludeConfig>,
    started: bool,
    finished: bool,
    stack: Vec<Frame>,
    /// 走査の最上位からのワークスペースのルートの相対パス (空なら同じ)。候補の相対パスは照合のパスからこれを外したもの。
    prefix_len: usize,
    visited: HashSet<VisitKey>,
    pending: Vec<Candidate>,
    found: u64,
    entries: u64,
    unreadable_dirs: u64,
    truncated: bool,
    /// Linux でルートが `/` (疑似 FS に降りない)。
    root_is_fs_root: bool,
    last_flush: Option<Tick>,
    reported_unreadable: u64,
}

impl fmt::Debug for WorkspaceWalk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkspaceWalk")
            .field("root", &self.root)
            .field("stack", &self.stack.len())
            .field("pending", &self.pending.len())
            .field("found", &self.found)
            .field("entries", &self.entries)
            .field("unreadable_dirs", &self.unreadable_dirs)
            .field("truncated", &self.truncated)
            .field("finished", &self.finished)
            .finish_non_exhaustive()
    }
}

impl WorkspaceWalk {
    /// ルート `root` (絶対パス) を除外の設定 `config` (走査を始める時点の値) で走査する状態を作る。I/O は最初の
    /// [`WorkspaceWalk::step`] で始める。
    #[must_use]
    pub fn new(root: FsPath, config: Arc<ExcludeConfig>) -> WorkspaceWalk {
        let root_is_fs_root =
            cfg!(target_os = "linux") && root.as_path() == std::path::Path::new("/");
        WorkspaceWalk {
            root,
            config,
            started: false,
            finished: false,
            stack: Vec::new(),
            prefix_len: 0,
            visited: HashSet::new(),
            pending: Vec::new(),
            found: 0,
            entries: 0,
            unreadable_dirs: 0,
            truncated: false,
            root_is_fs_root,
            last_flush: None,
            reported_unreadable: 0,
        }
    }

    /// ルート。
    #[must_use]
    pub fn root(&self) -> &FsPath {
        &self.root
    }

    /// 見つけた候補の数 (塊にしたものを含む)。
    #[must_use]
    pub fn found(&self) -> u64 {
        self.found
    }

    /// 読めなかったディレクトリの数 (`read_dir` の失敗と、列挙が途中で失敗したもの)。
    #[must_use]
    pub fn unreadable_dirs(&self) -> u64 {
        self.unreadable_dirs
    }

    /// 上限 (候補の数・列挙した項目の数・深さ・1 つのディレクトリの項目の数) か、列挙の途中の失敗で、一覧が欠けている。
    #[must_use]
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// 走査が終わった。
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// まだ塊にしていない候補をすべて取り出す (試験と横断検索が使う)。
    pub fn take_pending(&mut self) -> Vec<Candidate> {
        std::mem::take(&mut self.pending)
    }

    /// 送る塊を取り出す: [`CANDIDATE_CHUNK`] 件ごとの塊と、`finish` なら残りすべて、そうでなければ前に送ってから
    /// [`SCAN_FLUSH_INTERVAL`] を過ぎていれば残りすべて (少ないファイルの遅い FS でも、走査の途中の候補が届く)。
    /// 返す数は、塊ごとの「その時点の読めなかったディレクトリの数」と組にする。`finish` で、塊が無くても読めなかった
    /// ディレクトリの数が前に送った値から増えていれば、空の塊を 1 つ返す (数を届けるため)。
    pub fn take_chunks(&mut self, now: Tick, finish: bool) -> Vec<Arc<[Candidate]>> {
        let last = *self.last_flush.get_or_insert(now);
        let mut out: Vec<Arc<[Candidate]>> = Vec::new();
        while self.pending.len() >= CANDIDATE_CHUNK {
            let rest = self.pending.split_off(CANDIDATE_CHUNK);
            out.push(Arc::from(std::mem::replace(&mut self.pending, rest)));
        }
        let due = now.saturating_sub(last) >= SCAN_FLUSH_INTERVAL;
        if !self.pending.is_empty() && (finish || due) {
            out.push(Arc::from(std::mem::take(&mut self.pending)));
        }
        if out.is_empty() && finish && self.unreadable_dirs != self.reported_unreadable {
            out.push(Arc::from(Vec::new()));
        }
        if !out.is_empty() {
            self.last_flush = Some(now);
            self.reported_unreadable = self.unreadable_dirs;
        }
        out
    }

    /// 1 区切りを進める (候補は内に溜め、[`WorkspaceWalk::take_chunks`] で取り出す)。
    pub fn step(&mut self, fs: &dyn FileSystem, clock: &dyn Clock) -> WalkStep {
        self.step_observed(fs, clock, &mut NoObserver)
    }

    /// [`WorkspaceWalk::step`] と同じ。候補にならなかった項目を `obs` に知らせる。
    pub fn step_observed(
        &mut self,
        fs: &dyn FileSystem,
        clock: &dyn Clock,
        obs: &mut dyn WalkObserver,
    ) -> WalkStep {
        if self.finished {
            return WalkStep::Done;
        }
        let start = clock.now();
        if !self.started {
            self.started = true;
            self.begin(fs);
        }
        let mut dirs_read = 0usize;
        let mut entries_done = 0usize;
        loop {
            let Some(top) = self.stack.last_mut() else {
                self.finished = true;
                return WalkStep::Done;
            };
            if top.listing.is_none() {
                if let Err(e) = self.read_top(fs, obs) {
                    self.stack.clear();
                    self.finished = true;
                    return WalkStep::Failed(e);
                }
                dirs_read += 1;
                if dirs_read >= WALK_DIRS_PER_STEP
                    || clock.now().saturating_sub(start) >= WALK_STEP_TIME
                {
                    return WalkStep::Yield;
                }
                continue;
            }
            // 項目を処理する (子のディレクトリに降りるときは、子を積んで外の繰り返しに戻る)
            loop {
                if entries_done >= WALK_ENTRIES_PER_STEP {
                    return WalkStep::Yield;
                }
                let Some(top) = self.stack.last_mut() else {
                    break;
                };
                let Some(entry) = top.listing.as_mut().and_then(Iterator::next) else {
                    self.stack.pop();
                    break;
                };
                entries_done += 1;
                match self.visit(fs, entry, obs) {
                    Visit::Next => {}
                    Visit::Descend => break,
                    Visit::Stop => {
                        self.truncated = true;
                        self.stack.clear();
                        self.finished = true;
                        return WalkStep::Done;
                    }
                }
            }
        }
    }

    /// 最初の区切りの始め: リポジトリを探し (ルートから上へ)、ルートより上の祖先の `.gitignore` を読んで、ルートを積む。
    fn begin(&mut self, fs: &dyn FileSystem) {
        let respect = self.config.respect_gitignore;
        let repo = find_repo_upward(fs, &self.root);
        let prefix = repo
            .as_ref()
            .and_then(|(repo_root, _)| self.root.as_path().strip_prefix(repo_root.as_path()).ok())
            .map(rel_bytes)
            .unwrap_or_default();
        // `workspace.exclude` の置き場所はワークスペースのルート (最上位からの相対パス)
        let (workspace_rules, _) = IgnoreRules::from_lines(&prefix, &self.config.exclude);
        let mut chain = ExcludeChain::new(workspace_rules);
        if let Some((repo_root, dotgit)) = repo {
            chain = chain
                .enter_repo(load_repo(fs, &repo_root, dotgit, b"", respect))
                .0;
            if respect && !prefix.is_empty() {
                chain = ancestor_chain(fs, &repo_root, &self.root, chain);
            }
        }
        self.prefix_len = if prefix.is_empty() {
            0
        } else {
            prefix.len() + 1
        };
        if self.config.follow_symlinks
            && let Some(key) = visit_key(fs, &self.root)
        {
            self.visited.insert(key);
        }
        self.stack.push(Frame {
            abs: self.root.clone(),
            mrel: prefix,
            os_rel: PathBuf::new(),
            depth: 0,
            chain,
            listing: None,
        });
    }

    /// スタックの先頭のディレクトリを列挙して並べ、入れ子のリポジトリと `.gitignore` を連鎖に足す。読めなければ数えて外す
    /// (ルートだけは失敗にする)。
    fn read_top(&mut self, fs: &dyn FileSystem, obs: &mut dyn WalkObserver) -> Result<(), FsError> {
        let respect = self.config.respect_gitignore;
        let prefix_len = self.prefix_len;
        let top = self.stack.last_mut().expect("先頭がある");
        let listing = match fs.read_dir(&top.abs, WALK_ENTRIES_PER_DIR_MAX) {
            Ok(l) => l,
            Err(e) if top.depth == 0 => return Err(e),
            Err(_) => {
                self.unreadable_dirs += 1;
                self.stack.pop();
                return Ok(());
            }
        };
        if listing.truncated {
            self.truncated = true;
        }
        if listing.incomplete {
            self.unreadable_dirs += 1;
            self.truncated = true;
        }
        let mut entries = listing.entries;
        // 名前のバイト列の順 (OS と FS で並びが違うので、並べて結果を決定的にする)
        entries.sort_by(|a, b| a.name.as_encoded_bytes().cmp(b.name.as_encoded_bytes()));
        if top.depth > 0
            && let Some(dg) = entries
                .iter()
                .find(|e| is_dot_git(e.name.as_encoded_bytes()))
        {
            // 入れ子のリポジトリ: 外側の `.gitignore` と `info/exclude` はここで効かなくなる
            let repo = load_repo(
                fs,
                &top.abs,
                DotGit::from_entry(dg.kind),
                &top.mrel,
                respect,
            );
            top.chain = top.chain.enter_repo(repo).0;
            obs.nested_repo(top.mrel.get(prefix_len..).unwrap_or_default());
        }
        if respect
            && top.chain.in_repo()
            && let Some(gi) = entries
                .iter()
                .find(|e| e.kind == EntryKind::File && is_gitignore(e.name.as_encoded_bytes()))
            && let Some(rules) = read_gitignore(fs, &top.abs.join(&gi.name), &top.mrel)
        {
            top.chain = top.chain.with_gitignore(rules).0;
        }
        top.listing = Some(entries.into_iter());
        Ok(())
    }

    /// 項目 1 つを処理する。
    fn visit(&mut self, fs: &dyn FileSystem, entry: DirEntry, obs: &mut dyn WalkObserver) -> Visit {
        if self.entries >= WORKSPACE_MAX_ENTRIES {
            return Visit::Stop;
        }
        self.entries += 1;
        let top = self.stack.last().expect("処理中のディレクトリがある");
        let name = entry.name.as_encoded_bytes();
        if is_dot_git(name) || is_save_temp(name) {
            return Visit::Next;
        }
        if self.root_is_fs_root && top.depth == 0 && LINUX_PSEUDO_FS.contains(&name) {
            return Visit::Next;
        }
        let mut mrel = top.mrel.clone();
        if !mrel.is_empty() {
            mrel.push(b'/');
        }
        mrel.extend_from_slice(name);
        let kind = match entry.kind {
            EntryKind::File => Some(PathKind::File),
            EntryKind::Dir => Some(PathKind::Dir),
            EntryKind::Other => None,
            EntryKind::Symlink if !self.config.follow_symlinks => Some(PathKind::File),
            EntryKind::Symlink => match fs.metadata(&top.abs.join(&entry.name)) {
                Ok(m) if m.kind == FileKind::Dir => Some(PathKind::Dir),
                Ok(m) if m.kind == FileKind::Other => None,
                // ファイルとダングリングのリンクは候補にする (開くときにリンク先を開く・失敗を示す)
                _ => Some(PathKind::File),
            },
        };
        let Some(kind) = kind else {
            return Visit::Next;
        };
        let rel_start = self.prefix_len.min(mrel.len());
        if top.chain.is_excluded(&mrel, kind) {
            obs.excluded(&mrel[rel_start..], kind);
            return Visit::Next;
        }
        match kind {
            PathKind::File => {
                if self.found >= WORKSPACE_MAX_FILES {
                    return Visit::Stop;
                }
                let os_rel = &top.os_rel;
                self.pending
                    .push(Candidate::from_walk(&mrel[rel_start..], || {
                        os_rel.join(&entry.name)
                    }));
                self.found += 1;
                Visit::Next
            }
            PathKind::Dir => {
                let depth = top.depth + 1;
                if depth > WORKSPACE_MAX_DEPTH {
                    self.truncated = true;
                    return Visit::Next;
                }
                let abs = top.abs.join(&entry.name);
                if entry.kind == EntryKind::Symlink
                    && let Some(key) = visit_key(fs, &abs)
                    && !self.visited.insert(key)
                {
                    return Visit::Next;
                }
                obs.entered_dir(&mrel[rel_start..]);
                let frame = Frame {
                    os_rel: top.os_rel.join(&entry.name),
                    chain: top.chain.clone(),
                    abs,
                    mrel,
                    depth,
                    listing: None,
                };
                self.stack.push(frame);
                Visit::Descend
            }
        }
    }
}

/// 項目 1 つの処理の結果。
enum Visit {
    Next,
    Descend,
    /// 上限 (候補の数・列挙した項目の数) に達した。
    Stop,
}

/// リポジトリのルートからワークスペースのルートの親までの祖先の `.gitignore` を連鎖に足す (03 §12.7.3 の「リポジトリ」。結果が、
/// そのフォルダで `git ls-files` を走らせたときと同じになるように)。
fn ancestor_chain(
    fs: &dyn FileSystem,
    repo_root: &FsPath,
    root: &FsPath,
    mut chain: ExcludeChain,
) -> ExcludeChain {
    let Ok(rest) = root.as_path().strip_prefix(repo_root.as_path()) else {
        return chain;
    };
    let mut dir = repo_root.clone();
    let mut base: Vec<u8> = Vec::new();
    let parts: Vec<_> = rest.components().collect();
    // 最後の要素 (ワークスペースのルート自身) の `.gitignore` は走査が読む
    for (i, part) in parts.iter().enumerate() {
        if let Some(rules) = read_ancestor_gitignore(fs, &dir, &base) {
            chain = chain.with_gitignore(rules).0;
        }
        if i + 1 == parts.len() {
            break;
        }
        dir = dir.join(part);
        if !base.is_empty() {
            base.push(b'/');
        }
        base.extend_from_slice(part.as_os_str().as_encoded_bytes());
    }
    chain
}

/// `follow_symlinks = true` のときの、降りたディレクトリの鍵 (`FileId`、取れなければ canonicalize したパス)。
fn visit_key(fs: &dyn FileSystem, path: &FsPath) -> Option<VisitKey> {
    if let Some(id) = fs.metadata(path).ok().and_then(|m| m.file_id) {
        return Some(VisitKey::Id(id));
    }
    fs.canonicalize(path).ok().map(VisitKey::Path)
}

/// evoeditor 自身の保存の一時ファイルと退避名 (`.<name>.eve-tmp-*` / `.<name>.eve-bak-*`、05 §4.6)。候補にせず読まない。
fn is_save_temp(name: &[u8]) -> bool {
    let Some(rest) = name.strip_prefix(b".") else {
        return false;
    };
    [b".eve-tmp-".as_slice(), b".eve-bak-".as_slice()]
        .iter()
        .any(|m| {
            rest.windows(m.len())
                .position(|w| w == *m)
                .is_some_and(|i| i > 0)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 保存の一時ファイルと退避名を見分ける() {
        assert!(is_save_temp(b".a.txt.eve-tmp-12-abcdefgh"));
        assert!(is_save_temp(b".a.txt.eve-bak-12-abcdefgh"));
        assert!(!is_save_temp(b"a.txt.eve-tmp-12"), "先頭が . でない");
        assert!(!is_save_temp(b"..eve-tmp-1"), "名前の部分が空");
        assert!(!is_save_temp(b".env"));
    }

    #[test]
    fn 走査の状態は複製してスレッドへ渡せる() {
        fn assert_send_clone<T: Send + Sync + Clone>() {}
        assert_send_clone::<WorkspaceWalk>();
    }
}
