//! ワークスペースの除外の照合 (gitignore。docs/03 §12.7.3、06 X10)。
//!
//! I/O をしない純粋関数の集まり。ignore のファイル・`info/exclude`・git の `config`・`.git` のファイルの中身は、呼び出し側
//! (ワークスペースの走査、03 §12.7.2) が `FileSystem` で読んで渡す。
//!
//! - [`IgnoreRules`]: 除外の源 1 つ (`.gitignore` 1 つ、`info/exclude`、`workspace.exclude` の行の並び) を解釈したもの。
//!   置き場所 (`base`) を持ち、そこからの相対パスに当てる。各源の中は後ろの行から当て、最初に当たった行が決める。
//! - [`ExcludeChain`]: 1 つのディレクトリで効く源の連鎖。`workspace.exclude` → 内側の `.gitignore` から外側へ →
//!   `info/exclude` の順に見て、最初に当たった源が決める。入れ子のリポジトリに入ると `.gitignore` の連鎖と `info/exclude` を
//!   替える (外側の規則は効かない)。複製は `Arc` の複製だけ。
//! - [`parse_core_ignorecase`] / [`parse_gitdir_file`] / [`parse_commondir_file`]: リポジトリの設定と `.git` のファイルの解釈。
//!
//! パスは、走査の「最上位」(リポジトリのルートがワークスペースのルートより上ならリポジトリのルート、そうでなければワークスペース
//! のルート) からの相対パスのバイト列で、区切りは `/` (Windows も)。名前が UTF-8 で表せないときは、Unix は元のバイト列、
//! Windows は WTF-8 のバイト列にする。
//!
//! 実装は gitignore(5) の記述と git の実行結果との差分試験 (docs/09 §3.4) だけから書いた。git のソース (`dir.c`・
//! `wildmatch.c`) と git の試験のデータは写していない (GPL-2.0。03 §12.7.3 の「ライセンス」)。
//!
//! spec: 03 §12.7.3, 03 §12.7.8, 09 §3.3, 09 §3.4

pub mod fuzzing;
mod pattern;
mod repo;

use std::sync::Arc;

pub use pattern::CaseFold;
use pattern::{Mode, Program, step_bound};
pub use repo::{GitPath, parse_commondir_file, parse_core_ignorecase, parse_gitdir_file};

/// 1 つの ignore のファイルの大きさの上限 (1 MiB。超えたらそのファイルを読まない、03 §12.7.3 の「資源の上限」)。
/// 呼び出し側は上限 + 1 バイトまで読んで渡す。
pub const IGNORE_FILE_MAX_BYTES: usize = 1 << 20;
/// 1 行の長さの上限 (超えた行は飛ばす)。
pub const IGNORE_LINE_MAX_BYTES: usize = 4096;
/// 1 つの源のパターンの数の上限 (超えた分の行は読まない)。
pub const IGNORE_FILE_MAX_PATTERNS: usize = 10_000;
/// 1 つの項目に効く連鎖のパターンの数の合計の上限 (超えた分の行は読まない)。
pub const IGNORE_CHAIN_MAX_PATTERNS: usize = 50_000;

/// 照らす項目の種類 (ディレクトリだけに当てるパターン `foo/` のため)。シンボリックリンクは、リンク先に依らず `File`
/// (git と同じ)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PathKind {
    /// ファイル・シンボリックリンク・その他。
    File,
    /// ディレクトリ。
    Dir,
}

/// 1 行のパターン。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Rule {
    program: Program,
    mode: Mode,
    negated: bool,
    dir_only: bool,
    /// 源の中の行番号 (1 始まり。`workspace.exclude` は要素の番号 (1 始まり))。
    line: u32,
}

/// 解釈で飛ばしたものの数 (呼び出し側がログに残す。画面には出さない)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ParseReport {
    /// 本文が [`IGNORE_FILE_MAX_BYTES`] を超えたので、何も読まなかった。
    pub too_large: bool,
    /// [`IGNORE_LINE_MAX_BYTES`] を超えて飛ばした行の数。
    pub long_lines: u32,
    /// パターンの数の上限 ([`IGNORE_FILE_MAX_PATTERNS`]・[`IGNORE_CHAIN_MAX_PATTERNS`]) を超えて読まなかった行の数。
    pub dropped_patterns: u32,
}

impl ParseReport {
    /// 何かを飛ばしたか。
    #[must_use]
    pub fn is_clean(&self) -> bool {
        !self.too_large && self.long_lines == 0 && self.dropped_patterns == 0
    }
}

/// 除外の源 1 つを解釈したもの。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IgnoreRules {
    /// 置き場所 (最上位からの相対パス。区切りは `/`、前後の `/` は無い。最上位なら空)。
    base: Box<[u8]>,
    rules: Vec<Rule>,
}

/// 1 つの項目に当たった行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IgnoreHit {
    /// 当たった源。
    pub source: IgnoreSource,
    /// 源の中の行番号 (1 始まり。`workspace.exclude` は要素の番号)。
    pub line: u32,
    /// 除外するか (否定 `!` の行に当たったなら `false`)。
    pub excluded: bool,
}

/// 当たった源の種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IgnoreSource {
    /// `workspace.exclude` (07 §5.9)。
    Workspace,
    /// `.gitignore` (内側から数えて何番目か。0 が最も内側)。
    GitIgnore(u32),
    /// リポジトリの `info/exclude`。
    InfoExclude,
}

impl IgnoreRules {
    /// ignore のファイルの本文 `text` を、置き場所 `base` (最上位からの相対パス) の源として解釈する。
    ///
    /// gitignore(5) の書式: UTF-8 の BOM を飛ばす。行末の CR を 1 つ落とす。空行と `#` で始まる行は飛ばす。行頭の `!` は
    /// 否定。逃がしていない末尾の空白を落とす。末尾の `/` はディレクトリだけに当てる印。先頭か途中に `/` があれば置き場所からの
    /// 相対パスに、無ければ名前に当てる。行の中の NUL から後ろは読まない (git と同じ)。
    #[must_use]
    pub fn parse(base: &[u8], text: &[u8]) -> (IgnoreRules, ParseReport) {
        let mut report = ParseReport::default();
        let mut out = IgnoreRules {
            base: trim_slashes(base).into(),
            rules: Vec::new(),
        };
        if text.len() > IGNORE_FILE_MAX_BYTES {
            report.too_large = true;
            return (out, report);
        }
        let body = text.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(text);
        for (idx, raw) in body.split(|b| *b == b'\n').enumerate() {
            if raw.len() > IGNORE_LINE_MAX_BYTES {
                report.long_lines += 1;
                continue;
            }
            let line_no = u32::try_from(idx + 1).unwrap_or(u32::MAX);
            out.push_line(raw, line_no, &mut report);
        }
        (out, report)
    }

    /// `workspace.exclude` の各要素を、置き場所 `base` (ワークスペースのルート) の `.gitignore` の 1 行として解釈する
    /// (03 §12.7.3 の「`workspace.exclude` の書き方」)。
    #[must_use]
    pub fn from_lines<S: AsRef<str>>(base: &[u8], lines: &[S]) -> (IgnoreRules, ParseReport) {
        let mut report = ParseReport::default();
        let mut out = IgnoreRules {
            base: trim_slashes(base).into(),
            rules: Vec::new(),
        };
        for (idx, line) in lines.iter().enumerate() {
            let raw = line.as_ref().as_bytes();
            if raw.len() > IGNORE_LINE_MAX_BYTES {
                report.long_lines += 1;
                continue;
            }
            let line_no = u32::try_from(idx + 1).unwrap_or(u32::MAX);
            out.push_line(raw, line_no, &mut report);
        }
        (out, report)
    }

    fn push_line(&mut self, raw: &[u8], line_no: u32, report: &mut ParseReport) {
        let mut line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if let Some(nul) = line.iter().position(|b| *b == 0) {
            line = &line[..nul];
        }
        // 空行と `#` の行 (CR を落とす前の行で見る。git と同じ)
        if raw.is_empty() || raw[0] == b'#' {
            return;
        }
        let mut pat = trim_trailing_spaces(line);
        let negated = pat.first() == Some(&b'!');
        if negated {
            pat = &pat[1..];
        }
        let dir_only = pat.last() == Some(&b'/');
        if dir_only {
            pat = &pat[..pat.len() - 1];
        }
        let mode = if pat.contains(&b'/') {
            Mode::Path
        } else {
            Mode::Basename
        };
        if mode == Mode::Path && pat.first() == Some(&b'/') {
            pat = &pat[1..];
        }
        if pat.is_empty() {
            // 空のパターン (`!`・`/` だけの行など) は何にも当たらない
            return;
        }
        if self.rules.len() >= IGNORE_FILE_MAX_PATTERNS {
            report.dropped_patterns += 1;
            return;
        }
        self.rules.push(Rule {
            program: Program::compile(pat, mode),
            mode,
            negated,
            dir_only,
            line: line_no,
        });
    }

    /// 置き場所 (最上位からの相対パス)。
    #[must_use]
    pub fn base(&self) -> &[u8] {
        &self.base
    }

    /// パターンの数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// パターンが無いか。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// 先頭の `n` 個だけを残す (連鎖の上限、[`IGNORE_CHAIN_MAX_PATTERNS`])。
    fn truncate(&mut self, n: usize) {
        self.rules.truncate(n);
    }

    /// `path` (最上位からの相対パス) にこの源の行が当たるかを、後ろの行から見て最初に当たった行で返す。置き場所の外の
    /// パスと、どの行にも当たらないパスは `None`。戻り値の `source` は [`IgnoreSource::GitIgnore`]`(0)`。
    #[must_use]
    pub fn decide(&self, path: &[u8], kind: PathKind, fold: CaseFold) -> Option<IgnoreHit> {
        let mut steps = 0;
        self.decide_counted(path, kind, fold, &mut steps)
            .map(|(line, excluded)| IgnoreHit {
                source: IgnoreSource::GitIgnore(0),
                line,
                excluded,
            })
    }

    /// `decide` の本体。`steps` に照合の手数を足す。返すのは (行番号, 除外するか)。
    fn decide_counted(
        &self,
        path: &[u8],
        kind: PathKind,
        fold: CaseFold,
        steps: &mut u64,
    ) -> Option<(u32, bool)> {
        let rel = relative_to(&self.base, path)?;
        let name_start = rel.iter().rposition(|b| *b == b'/').map_or(0, |i| i + 1);
        let name = &rel[name_start..];
        self.rules.iter().rev().find_map(|r| {
            if r.dir_only && kind != PathKind::Dir {
                return None;
            }
            let subject = match r.mode {
                Mode::Basename => name,
                Mode::Path => rel,
            };
            r.program
                .matches(subject, fold, steps)
                .then_some((r.line, !r.negated))
        })
    }

    /// 1 回の `decide` の手数の上限 (各行の「(字句の数 + 1) × (パスの長さ + 1)」の和。fuzz の性質、docs/09 §3.3)。
    fn step_bound(&self, path_len: usize) -> u64 {
        self.rules
            .iter()
            .map(|r| step_bound(r.program.len(), path_len))
            .fold(0u64, u64::saturating_add)
    }
}

/// 前後の `/` を外す。
fn trim_slashes(mut s: &[u8]) -> &[u8] {
    while let [b'/', rest @ ..] = s {
        s = rest;
    }
    while let [rest @ .., b'/'] = s {
        s = rest;
    }
    s
}

/// `base` の下のパスなら、`base` からの相対パスを返す。
fn relative_to<'p>(base: &[u8], path: &'p [u8]) -> Option<&'p [u8]> {
    if base.is_empty() {
        return Some(path);
    }
    let rest = path.strip_prefix(base)?;
    rest.strip_prefix(b"/")
}

/// 逃がしていない末尾の空白 (` `) を落とす (`\ ` は残す。タブは落とさない)。
fn trim_trailing_spaces(line: &[u8]) -> &[u8] {
    let mut end = line.len();
    let mut space_from: Option<usize> = None;
    let mut i = 0;
    while i < line.len() {
        match line[i] {
            b' ' => {
                space_from.get_or_insert(i);
            }
            b'\\' => {
                space_from = None;
                i += 1;
            }
            _ => space_from = None,
        }
        i += 1;
    }
    if let Some(s) = space_from {
        end = s;
    }
    &line[..end]
}

/// リポジトリの除外の源 (`info/exclude` と `core.ignorecase`)。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepoIgnore {
    info_exclude: IgnoreRules,
    fold: CaseFold,
}

impl RepoIgnore {
    /// リポジトリの `info/exclude` (置き場所はリポジトリのルート。読めなかったときは空の源) と、`core.ignorecase` の畳み方。
    #[must_use]
    pub fn new(info_exclude: IgnoreRules, fold: CaseFold) -> RepoIgnore {
        RepoIgnore { info_exclude, fold }
    }

    /// `core.ignorecase` の畳み方。
    #[must_use]
    pub fn fold(&self) -> CaseFold {
        self.fold
    }
}

/// `.gitignore` の連鎖の 1 つ (内側から外側へ)。
#[derive(Debug)]
struct DirLink {
    rules: IgnoreRules,
    parent: Option<Arc<DirLink>>,
}

/// 1 つのディレクトリで効く除外の源の連鎖 (03 §12.7.3 の「優先」)。走査はディレクトリごとに 1 つ持ち、子のディレクトリに
/// 降りるときに [`ExcludeChain::with_gitignore`] で `.gitignore` を足す。複製は `Arc` の複製だけ。
#[derive(Debug, Clone, Default)]
pub struct ExcludeChain {
    workspace: Option<Arc<IgnoreRules>>,
    repo: Option<Arc<RepoIgnore>>,
    dirs: Option<Arc<DirLink>>,
    /// 連鎖のパターンの数の合計。
    patterns: usize,
}

impl ExcludeChain {
    /// `workspace.exclude` だけの連鎖 (リポジトリの外。大文字小文字を区別する)。
    #[must_use]
    pub fn new(workspace: IgnoreRules) -> ExcludeChain {
        let patterns = workspace.len();
        ExcludeChain {
            workspace: (!workspace.is_empty()).then(|| Arc::new(workspace)),
            repo: None,
            dirs: None,
            patterns,
        }
    }

    /// リポジトリに入る (ルートの、または入れ子のリポジトリの境目)。外側の `.gitignore` と `info/exclude` は効かなくなり、
    /// このリポジトリの `info/exclude` と `core.ignorecase` が効く。`workspace.exclude` はそのまま効く。返す数は上限で
    /// 読まなかったパターンの数。
    #[must_use]
    pub fn enter_repo(&self, repo: RepoIgnore) -> (ExcludeChain, u32) {
        let base = self.workspace.as_ref().map_or(0, |w| w.len());
        let mut repo = repo;
        let dropped = cap(&mut repo.info_exclude, base);
        let patterns = base + repo.info_exclude.len();
        (
            ExcludeChain {
                workspace: self.workspace.clone(),
                repo: Some(Arc::new(repo)),
                dirs: None,
                patterns,
            },
            dropped,
        )
    }

    /// 子のディレクトリの `.gitignore` を内側に足した連鎖。リポジトリの外では足さない (gitignore の源は `.git` があるときだけ
    /// 効く)。返す数は上限で読まなかったパターンの数。
    #[must_use]
    pub fn with_gitignore(&self, rules: IgnoreRules) -> (ExcludeChain, u32) {
        if self.repo.is_none() || rules.is_empty() {
            return (self.clone(), 0);
        }
        let mut rules = rules;
        let dropped = cap(&mut rules, self.patterns);
        let patterns = self.patterns + rules.len();
        (
            ExcludeChain {
                workspace: self.workspace.clone(),
                repo: self.repo.clone(),
                dirs: Some(Arc::new(DirLink {
                    rules,
                    parent: self.dirs.clone(),
                })),
                patterns,
            },
            dropped,
        )
    }

    /// リポジトリの中か (`.gitignore` を読むか)。
    #[must_use]
    pub fn in_repo(&self) -> bool {
        self.repo.is_some()
    }

    /// 照合の畳み方 (リポジトリの `core.ignorecase`。リポジトリの外は区別する)。`workspace.exclude` も同じ畳み方で照らす。
    #[must_use]
    pub fn fold(&self) -> CaseFold {
        self.repo.as_ref().map_or(CaseFold::Sensitive, |r| r.fold)
    }

    /// 連鎖のパターンの数の合計。
    #[must_use]
    pub fn pattern_count(&self) -> usize {
        self.patterns
    }

    /// `path` (最上位からの相対パス) を除外するか。
    #[must_use]
    pub fn is_excluded(&self, path: &[u8], kind: PathKind) -> bool {
        self.decide(path, kind).is_some_and(|h| h.excluded)
    }

    /// `path` に当たった行 (源の優先と各源の中の後ろからの順で、最初に当たったもの)。どれにも当たらなければ `None`。
    #[must_use]
    pub fn decide(&self, path: &[u8], kind: PathKind) -> Option<IgnoreHit> {
        let mut steps = 0;
        self.decide_counted(path, kind, &mut steps)
    }

    pub(crate) fn decide_counted(
        &self,
        path: &[u8],
        kind: PathKind,
        steps: &mut u64,
    ) -> Option<IgnoreHit> {
        let fold = self.fold();
        let hit = |source, (line, excluded)| IgnoreHit {
            source,
            line,
            excluded,
        };
        if let Some(w) = &self.workspace
            && let Some(h) = w.decide_counted(path, kind, fold, steps)
        {
            return Some(hit(IgnoreSource::Workspace, h));
        }
        let mut link = self.dirs.as_deref();
        let mut depth = 0u32;
        while let Some(l) = link {
            if let Some(h) = l.rules.decide_counted(path, kind, fold, steps) {
                return Some(hit(IgnoreSource::GitIgnore(depth), h));
            }
            depth += 1;
            link = l.parent.as_deref();
        }
        let repo = self.repo.as_deref()?;
        repo.info_exclude
            .decide_counted(path, kind, fold, steps)
            .map(|h| hit(IgnoreSource::InfoExclude, h))
    }

    /// 1 回の `decide` の手数の上限 (fuzz の性質)。
    pub(crate) fn step_bound(&self, path_len: usize) -> u64 {
        let mut total = self
            .workspace
            .as_ref()
            .map_or(0, |w| w.step_bound(path_len));
        let mut link = self.dirs.as_deref();
        while let Some(l) = link {
            total = total.saturating_add(l.rules.step_bound(path_len));
            link = l.parent.as_deref();
        }
        if let Some(r) = &self.repo {
            total = total.saturating_add(r.info_exclude.step_bound(path_len));
        }
        total
    }
}

/// 連鎖の合計が上限を超えないように、`rules` の後ろを落とす。落とした数を返す。
fn cap(rules: &mut IgnoreRules, used: usize) -> u32 {
    let room = IGNORE_CHAIN_MAX_PATTERNS.saturating_sub(used);
    if rules.len() <= room {
        return 0;
    }
    let dropped = rules.len() - room;
    rules.truncate(room);
    u32::try_from(dropped).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests;
