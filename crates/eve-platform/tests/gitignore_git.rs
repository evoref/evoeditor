//! gitignore の照合 (eve-core の `ExcludeChain`) と git の差分試験 (docs/09 §3.4 の「ワークスペースの試験」の (1)、03 §12.7.8、
//! 01 FR-125 AC-1)。
//!
//! §4 の「ワークスペースの木」を実 FS に作り、本番の走査 (`WorkspaceWalk`、03 §12.7.2。M31 の段 4 で試験の中の参照の走査を
//! 置き換えた) の結果を、git の出力と比べて差 0 を確かめる。除外したディレクトリには走査が降りないので、その中の項目
//! (git の `--ignored` と `check-ignore` が 1 件ずつ出すもの) だけは試験が std で列挙する:
//!
//! - 残るファイル: `git ls-files -z --others --exclude-standard`
//! - 除外されるファイル: 同じに `--ignored` を足したもの
//! - 全パス (ファイルとディレクトリ) の除外の判定: `git check-ignore --stdin -z --no-index`
//!
//! git はシステム・利用者の設定とグローバルの excludesFile を読まない形で走らせる (`GIT_CONFIG_NOSYSTEM=1`・
//! `GIT_CONFIG_GLOBAL` に空のファイル・`-c core.excludesFile=<空のファイル>`・`-c core.quotePath=false`・
//! `-c core.longpaths=true`)。`core.ignorecase` は各リポジトリの `config` に `true` と `false` を書いて両方で比べる (走査は
//! その `config` を読む)。入れ子のリポジトリと worktree (`.git` がファイル) は、それぞれのルートで
//! 走らせた git の結果と比べる。git が無い・古い (2.32 より前。`GIT_CONFIG_GLOBAL` が効かない) 環境では失敗にする (飛ばさない)。
//!
//! 小さい木 (約 2,000 ファイル) と、ランダムに組んだ規則の木 (1 行の規則を置いたディレクトリ 200 個) は毎回。10 万ファイルの木は
//! `#[ignore]` で、手元と夜間 (`cargo xtask nightly` の `gitignore` の段) に `--ignored` で回す。

#![allow(clippy::print_stderr)] // 木の大きさと件数を記録に残す (--nocapture)

#[path = "common/workspace_tree.rs"]
mod workspace_tree;

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use std::sync::Arc;

use eve_core::{
    CaseFold, Clock, ExcludeConfig, FileSystem, FsPath, PathKind, Tick, WalkObserver, WalkStep,
    WorkspaceWalk,
};
use workspace_tree::{Git, TreeSize};

// ---------- 本番の走査 (WorkspaceWalk) の結果を、リポジトリごとに分ける ----------

/// 1 つのリポジトリの結果 (パスは木のルートからの相対パス)。
#[derive(Default)]
struct RepoResult {
    /// リポジトリのルート (木のルートからの相対パス。木のルートは空)。
    root: Vec<u8>,
    /// 残るファイル (除外されない入れ子のリポジトリは `<パス>/` で 1 件。git と同じ)。
    remaining: BTreeSet<Vec<u8>>,
    /// 除外されるファイル (除外したディレクトリの中のものを含む)。
    ignored: BTreeSet<Vec<u8>>,
    /// 全パス (ファイルとディレクトリ) と、除外されるか。
    all: Vec<(Vec<u8>, bool)>,
}

/// 走査が知らせる、候補にならなかった項目と降りたディレクトリ。
#[derive(Default)]
struct Events {
    excluded: Vec<(Vec<u8>, PathKind)>,
    dirs: Vec<Vec<u8>>,
    nested: Vec<Vec<u8>>,
}

impl WalkObserver for Events {
    fn excluded(&mut self, rel: &[u8], kind: PathKind) {
        self.excluded.push((rel.to_vec(), kind));
    }
    fn entered_dir(&mut self, rel: &[u8]) {
        self.dirs.push(rel.to_vec());
    }
    fn nested_repo(&mut self, rel: &[u8]) {
        self.nested.push(rel.to_vec());
    }
}

fn name_bytes(name: &std::ffi::OsStr) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        name.as_bytes().to_vec()
    }
    #[cfg(not(unix))]
    {
        name.to_str().expect("木の名前は UTF-8").as_bytes().to_vec()
    }
}

fn join(dir: &[u8], name: &[u8]) -> Vec<u8> {
    if dir.is_empty() {
        name.to_vec()
    } else {
        [dir, b"/", name].concat()
    }
}

fn abs(top: &Path, rel: &[u8]) -> PathBuf {
    let s = std::str::from_utf8(rel).expect("木のパスは UTF-8");
    if s.is_empty() {
        top.to_path_buf()
    } else {
        top.join(s)
    }
}

/// 除外したディレクトリの中のすべて (git の `--ignored` は中のファイルを 1 件ずつ出す)。本番の走査は除外したディレクトリに
/// 降りないので、ここだけ std で列挙する (`.git` は git と同じく飛ばす)。
fn all_excluded(top: &Path, dir: &[u8], out: &mut Vec<(Vec<u8>, PathKind)>) {
    let d = abs(top, dir);
    let mut v: Vec<(Vec<u8>, bool)> = fs::read_dir(&d)
        .unwrap_or_else(|e| panic!("{}: {e}", d.display()))
        .map(|e| {
            let e = e.expect("項目");
            let is_dir = e.file_type().expect("種類").is_dir();
            (name_bytes(&e.file_name()), is_dir)
        })
        .filter(|(n, _)| n.as_slice() != b".git")
        .collect();
    v.sort();
    for (name, is_dir) in v {
        let rel = join(dir, &name);
        if is_dir {
            out.push((rel.clone(), PathKind::Dir));
            all_excluded(top, &rel, out);
        } else {
            out.push((rel, PathKind::File));
        }
    }
}

/// 木のルート (リポジトリのルート) を本番の走査 (`WorkspaceWalk`。`workspace.exclude = []`) で最後まで走らせ、結果を
/// リポジトリごとに分ける。
fn walk_repos(top: &Path) -> Vec<RepoResult> {
    let fs = real_fs();
    let mut walk = WorkspaceWalk::new(FsPath::new(top), Arc::new(ExcludeConfig::new()));
    let mut ev = Events::default();
    let mut remaining: Vec<Vec<u8>> = Vec::new();
    loop {
        let step = walk.step_observed(&*fs, &StoppedClock, &mut ev);
        remaining.extend(
            walk.take_pending()
                .iter()
                .map(|c| c.rel().as_bytes().to_vec()),
        );
        match step {
            WalkStep::Yield => {}
            WalkStep::Done => break,
            WalkStep::Failed(e) => panic!("走査に失敗: {:?}", e.kind),
        }
    }
    assert!(!walk.truncated(), "木は上限に当たらない");
    assert_eq!(walk.unreadable_dirs(), 0);
    let roots: Vec<Vec<u8>> = std::iter::once(Vec::new())
        .chain(ev.nested.iter().cloned())
        .collect();
    let mut repos: Vec<RepoResult> = roots
        .iter()
        .map(|r| RepoResult {
            root: r.clone(),
            ..RepoResult::default()
        })
        .collect();
    // パスを含む最も内側のリポジトリ (入れ子のリポジトリのルートのディレクトリ自身は外側に属する)
    let owner = |p: &[u8]| -> usize {
        roots
            .iter()
            .enumerate()
            .filter(|(_, r)| r.is_empty() || p.starts_with(&[&r[..], b"/"].concat()))
            .max_by_key(|(_, r)| r.len())
            .map_or(0, |(i, _)| i)
    };
    let mut excluded: Vec<(Vec<u8>, PathKind)> = Vec::new();
    for (p, kind) in &ev.excluded {
        excluded.push((p.clone(), *kind));
        if *kind == PathKind::Dir {
            all_excluded(top, p, &mut excluded);
        }
    }
    let owners: Vec<usize> = excluded.iter().map(|(p, _)| owner(p)).collect();
    for ((p, kind), i) in excluded.into_iter().zip(owners) {
        if kind == PathKind::File {
            repos[i].ignored.insert(p.clone());
        }
        repos[i].all.push((p, true));
    }
    for p in remaining {
        let i = owner(&p);
        repos[i].remaining.insert(p.clone());
        repos[i].all.push((p, false));
    }
    for d in ev.dirs {
        let i = owner(&d);
        repos[i].all.push((d, false));
    }
    for n in ev.nested.clone() {
        let i = owner(&n);
        repos[i].remaining.insert([&n[..], b"/"].concat());
    }
    repos
}

fn real_fs() -> Box<dyn FileSystem> {
    #[cfg(windows)]
    return Box::new(eve_platform::fs::WinFs::default());
    #[cfg(unix)]
    return Box::new(eve_platform::unix_fs::UnixFs::default());
}

/// 止まった時計 (区切りは数だけで決まる)。
struct StoppedClock;

impl Clock for StoppedClock {
    fn now(&self) -> Tick {
        Tick(0)
    }
}

// ---------- git ----------

/// git の版を確かめる (無い・2.32 より前なら失敗)。
fn require_git(git: &Git) {
    let out = git
        .command(git.empty.parent().expect("親"))
        .arg("--version")
        .output()
        .unwrap_or_else(|e| {
            panic!("git を実行できません (差分試験には git 2.32 以降が要ります): {e}")
        });
    let text = String::from_utf8_lossy(&out.stdout);
    let ver: Vec<u32> = text
        .trim()
        .trim_start_matches("git version ")
        .split(|c: char| !c.is_ascii_digit())
        .take(2)
        .map(|s| s.parse().unwrap_or(0))
        .collect();
    assert!(
        ver.len() == 2 && (ver[0], ver[1]) >= (2, 32),
        "git が古い ({}): 差分試験には 2.32 以降 (GIT_CONFIG_GLOBAL・リンクの .gitignore を辿らない) が要ります",
        text.trim()
    );
}

fn git_list(git: &Git, cwd: &Path, args: &[&str]) -> BTreeSet<Vec<u8>> {
    let empty = git.empty.to_string_lossy().into_owned();
    let out = git
        .command(cwd)
        .args(["-c", &format!("core.excludesFile={empty}")])
        .args(args)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(<[u8]>::to_vec)
        .collect()
}

/// `git check-ignore` に `paths` を渡す。`verbose` なら `-v -n` の 4 つ組 (源・行・パターン・パス) を返す。
fn check_ignore(git: &Git, cwd: &Path, paths: &[Vec<u8>], verbose: bool) -> Vec<Vec<u8>> {
    let empty = git.empty.to_string_lossy().into_owned();
    let mut cmd = git.command(cwd);
    cmd.args(["-c", &format!("core.excludesFile={empty}")])
        .args(["check-ignore", "--stdin", "-z", "--no-index"]);
    if verbose {
        cmd.args(["-v", "-n"]);
    }
    // 入力はファイルから渡す (パイプに書く間に出力のパイプが詰まらないように。試験でスレッドを作らない、02 §4.3 R11)
    let mut input = Vec::new();
    for p in paths {
        input.extend_from_slice(p);
        input.push(0);
    }
    let input_file = git.empty.with_file_name("check-ignore-input");
    fs::write(&input_file, &input).expect("入力のファイル");
    let out = cmd
        .stdin(Stdio::from(fs::File::open(&input_file).expect("入力")))
        .output()
        .expect("git check-ignore");
    // 何も除外しないときは 1 で終わる
    assert!(
        out.status.code().is_some_and(|c| c <= 1),
        "git check-ignore: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
        .split(|b| *b == 0)
        .map(<[u8]>::to_vec)
        .collect::<Vec<_>>()
}

fn set_ignorecase(git: &Git, repo: &Path, fold: CaseFold) {
    let v = if fold == CaseFold::Ascii {
        "true"
    } else {
        "false"
    };
    git.run(repo, &["config", "core.ignorecase", v])
        .expect("git config");
}

fn strip_root<'p>(root: &[u8], p: &'p [u8]) -> &'p [u8] {
    if root.is_empty() {
        p
    } else {
        p.strip_prefix(root)
            .and_then(|r| r.strip_prefix(b"/"))
            .expect("リポジトリの中のパス")
    }
}

fn show(p: &[u8]) -> String {
    String::from_utf8_lossy(p).into_owned()
}

/// 木のルート `top` のすべてのリポジトリで、参照の走査と git を比べる。差の説明を返す (差 0 なら空)。
fn compare(git: &Git, top: &Path, fold: CaseFold) -> String {
    let repos = walk_repos(top);
    for r in &repos {
        set_ignorecase(git, &top.join(show(&r.root)), fold);
    }
    // 設定を書いた後に読み直す
    let repos = walk_repos(top);
    let mut report = String::new();
    for r in &repos {
        let cwd = top.join(show(&r.root));
        let rel_set = |s: &BTreeSet<Vec<u8>>| -> BTreeSet<Vec<u8>> {
            s.iter().map(|p| strip_root(&r.root, p).to_vec()).collect()
        };
        let remaining = rel_set(&r.remaining);
        let ignored = rel_set(&r.ignored);
        let git_remaining = git_list(
            git,
            &cwd,
            &["ls-files", "-z", "--others", "--exclude-standard"],
        );
        let git_ignored = git_list(
            git,
            &cwd,
            &[
                "ls-files",
                "-z",
                "--others",
                "--ignored",
                "--exclude-standard",
            ],
        );
        let all: Vec<Vec<u8>> = r
            .all
            .iter()
            .map(|(p, _)| strip_root(&r.root, p).to_vec())
            .collect();
        let want_ci: BTreeSet<Vec<u8>> = r
            .all
            .iter()
            .filter(|(_, e)| *e)
            .map(|(p, _)| strip_root(&r.root, p).to_vec())
            .collect();
        let git_ci: BTreeSet<Vec<u8>> = check_ignore(git, &cwd, &all, false)
            .into_iter()
            .filter(|p| !p.is_empty())
            .collect();
        let mut diffs: Vec<Vec<u8>> = Vec::new();
        for (label, ours, theirs) in [
            (
                "残るファイル (ls-files --others)",
                &remaining,
                &git_remaining,
            ),
            (
                "除外されるファイル (ls-files --others --ignored)",
                &ignored,
                &git_ignored,
            ),
            ("除外の判定 (check-ignore)", &want_ci, &git_ci),
        ] {
            let only_ours: Vec<_> = ours.difference(theirs).collect();
            let only_git: Vec<_> = theirs.difference(ours).collect();
            if only_ours.is_empty() && only_git.is_empty() {
                continue;
            }
            let _ = writeln!(
                report,
                "[{} / {fold:?}] {label}: 自作だけ {} 件、git だけ {} 件",
                show(&r.root),
                only_ours.len(),
                only_git.len()
            );
            for p in only_ours.iter().take(20) {
                let _ = writeln!(report, "  自作だけ: {}", show(p));
                diffs.push((*p).clone());
            }
            for p in only_git.iter().take(20) {
                let _ = writeln!(report, "  git だけ: {}", show(p));
                diffs.push((*p).clone());
            }
        }
        if !diffs.is_empty() {
            let _ = writeln!(report, "  git check-ignore -v:");
            for chunk in check_ignore(git, &cwd, &diffs, true).chunks(4) {
                let fields: Vec<String> = chunk.iter().map(|f| show(f)).collect();
                let _ = writeln!(report, "    {}", fields.join(" | "));
            }
        }
        let _ = (remaining.len(), ignored.len());
    }
    report
}

fn counts(top: &Path) -> (usize, usize) {
    let repos = walk_repos(top);
    (
        repos.iter().map(|r| r.remaining.len()).sum(),
        repos.iter().map(|r| r.ignored.len()).sum(),
    )
}

/// 一時ディレクトリの木 (終わりに消す)。
struct TmpTree(PathBuf);

impl TmpTree {
    fn new(tag: &str) -> TmpTree {
        let p = std::env::temp_dir().join(format!("eve-gitignore-{tag}-{}", std::process::id()));
        let _ = workspace_tree::remove(&p);
        TmpTree(p)
    }
}

impl Drop for TmpTree {
    fn drop(&mut self) {
        let _ = workspace_tree::remove(&self.0);
    }
}

fn git_for(tree: &Path) -> Git {
    Git {
        empty: workspace_tree::gitdirs_of(tree).join("empty-config"),
    }
}

fn run_tree(size: TreeSize, tag: &str) {
    let tmp = TmpTree::new(tag);
    let tree = workspace_tree::generate(&tmp.0, size, 42).expect("木を作る");
    let git = git_for(&tree.root);
    require_git(&git);
    let mut report = String::new();
    for fold in [CaseFold::Sensitive, CaseFold::Ascii] {
        report.push_str(&compare(&git, &tree.root, fold));
        let (remaining, ignored) = counts(&tree.root);
        eprintln!(
            "[{tag} / {fold:?}] ファイル {}・ディレクトリ {}: 残る {remaining}、除外 {ignored}",
            tree.files, tree.dirs
        );
        assert!(remaining > 0 && ignored > 0);
    }
    assert!(report.is_empty(), "git との差:\n{report}");
}

// verifies: AC-FR-125-1
#[test]
fn 小さい木で_git_と一致する() {
    run_tree(TreeSize::Small, "small");
}

/// 10 万ファイルの木 (手元と夜間。`cargo test -p eve-platform --test gitignore_git -- --ignored`)。
// verifies: AC-FR-125-1
#[test]
#[ignore = "10 万ファイルの木。夜間 (cargo xtask nightly の gitignore の段) と手元で --ignored を付けて回す"]
fn 大きい木で_git_と一致する() {
    run_tree(TreeSize::Large, "large");
}

// ---------- ランダムに組んだ規則 ----------

/// splitmix64。
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n as u64).expect("n 未満")
    }

    fn pick<'a>(&mut self, v: &[&'a str]) -> &'a str {
        v[self.below(v.len())]
    }
}

/// 各ディレクトリに置く、照らすファイル (大文字小文字だけが違う名前を同じディレクトリに置かない)。
const PROBES: &[&str] = &[
    "a",
    "B",
    "ab",
    "Xy",
    "x1",
    "9z",
    "c.txt",
    "D.TXT",
    "a.b.c",
    "[x]",
    "!x",
    "#x",
    "-",
    "^",
    "]",
    "a b",
    "あ",
    "aあ",
    "é.md",
    "ab-c",
    "abc/def",
    "abc/DEF2/g",
    "abd/x/y/z",
    "d/a",
    "d/B",
    "d/e/f.txt",
    "d/E2/G",
    "a2/b/c",
    "Q/q.txt",
    "x y/z",
];

/// Unix だけで作れる名前 (制御文字・`\`。POSIX の文字の集合 `space` / `cntrl` / `blank` / `print` / `graph` の境目を見る)。
#[cfg(unix)]
const UNIX_PROBES: &[&str] = &[
    "t\tz", "v\x0bz", "f\x0cz", "n\nz", "r\rz", "c\x01z", "d\x7fz", "b\\z", "s z",
];
#[cfg(not(unix))]
const UNIX_PROBES: &[&str] = &[];

/// パターンの部品 (ワイルドカード・集合・POSIX の集合・エスケープ・区切り)。
const ATOMS: &[&str] = &[
    "a",
    "b",
    "B",
    "c",
    "d",
    "x",
    "X",
    "y",
    "z",
    "e",
    "f",
    "g",
    "q",
    "1",
    "9",
    ".",
    "-",
    " ",
    "あ",
    "txt",
    "TXT",
    "*",
    "*",
    "**",
    "?",
    "/",
    "/",
    "[a-c]",
    "[!a]",
    "[^x]",
    "[A-Z]",
    "[a-z]",
    "[]a]",
    "[!]]",
    "[ab]",
    "[A]",
    "[B-b]",
    "[[:alpha:]]",
    "[[:upper:]]",
    "[[:lower:]]",
    "[[:digit:]]",
    "[[:alnum:]]",
    "[[:punct:]]",
    "[[:space:]]",
    "[[:xdigit:]]",
    "[[:bogus:]]",
    "[a-]",
    "[-a]",
    "[\\]]",
    "[[:a]",
    "[",
    "]",
    "\\*",
    "\\a",
    "\\!",
    "\\#",
    "\\ ",
    "\\[",
    "\\\\",
    "**/",
    "/**",
    "/**/",
    "***",
    "\\/",
];

fn random_line(rng: &mut Rng) -> String {
    let mut s = String::new();
    match rng.below(10) {
        0 => s.push('!'),
        1 => s.push('/'),
        2 => s.push_str("\\!"),
        _ => {}
    }
    let n = 1 + rng.below(5);
    for _ in 0..n {
        s.push_str(rng.pick(ATOMS));
    }
    match rng.below(8) {
        0 => s.push('/'),
        1 => s.push_str("  "),
        2 => s.push('\\'),
        _ => {}
    }
    s
}

/// ランダムな 1〜2 行の規則を `dirs` 個作る。
fn random_rules(seed: u64, dirs: usize) -> Vec<String> {
    let mut rng = Rng(seed);
    (0..dirs)
        .map(|_| {
            let mut text = random_line(&mut rng);
            if rng.below(3) == 0 {
                text.push('\n');
                text.push_str(&random_line(&mut rng));
            }
            text
        })
        .collect()
}

/// 規則の木を作る (ルートのリポジトリの下に `rand/pNNN/` を規則の数だけ。各ディレクトリに照らすファイルと、その規則の
/// `.gitignore`)。
fn rules_tree(root: &Path, rules: &[String]) {
    let gitdirs = workspace_tree::gitdirs_of(root);
    fs::create_dir_all(root).expect("木");
    fs::create_dir_all(&gitdirs).expect("gitdirs");
    fs::write(gitdirs.join("empty-config"), b"").expect("空の設定");
    let git = git_for(root);
    require_git(&git);
    git.run(root, &["init", "-q", "."]).expect("git init");
    for (d, text) in rules.iter().enumerate() {
        let dir = root.join(format!("rand/p{d:03}"));
        for p in PROBES.iter().chain(UNIX_PROBES) {
            let path = dir.join(p);
            fs::create_dir_all(path.parent().expect("親")).expect("ディレクトリ");
            fs::write(&path, b"p\n").expect("ファイル");
        }
        fs::write(dir.join(".gitignore"), format!("{text}\n")).expect(".gitignore");
    }
}

/// 規則の木を作って、`core.ignorecase` の両方で git と比べる。
fn run_rules(tag: &str, rules: &[String]) -> Result<(), String> {
    let tmp = TmpTree::new(tag);
    rules_tree(&tmp.0, rules);
    let git = git_for(&tmp.0);
    let mut report = String::new();
    for fold in [CaseFold::Sensitive, CaseFold::Ascii] {
        report.push_str(&compare(&git, &tmp.0, fold));
    }
    if report.is_empty() {
        return Ok(());
    }
    // 差のあったディレクトリの規則を添える
    let mut lines = String::new();
    for (d, l) in rules.iter().enumerate() {
        if report.contains(&format!("rand/p{d:03}/")) {
            let _ = writeln!(lines, "  rand/p{d:03}/.gitignore: {l:?}");
        }
    }
    Err(format!("{report}\n規則:\n{lines}"))
}

/// 実装で git に揃えた細目 (集合の大文字小文字・`**` の境目・字の並びの直後の `**`・エスケープ・閉じていない集合・末尾の
/// 空白) を、1 行ずつ git と突き合わせる。
// verifies: AC-FR-125-1
#[test]
fn 細目の規則で_git_と一致する() {
    let rules: Vec<String> = [
        "[A]",
        "[B-b]",
        "[[:upper:]]",
        "[[:lower:]]",
        "[A-Z]",
        "[a-z]",
        "/abc**",
        "/abc/d**",
        "a**/z",
        "/ab**/y",
        "abd/**/z",
        "abd/**/y/z",
        "**\\/z",
        "**/f.txt",
        "d/**",
        "d/**/G",
        "abc\\",
        "[a-\\",
        "[[:a]",
        "[!]]",
        "[]a]",
        "[[:bogus:]]",
        "[a",
        "a b  ",
        "a\\ b",
        "x y/",
        "/x y/z",
        "\\#x",
        "\\!x",
        "!x",
        "#x",
        "x1/",
        "**",
        "*",
        "d/*",
        "*/",
        "/*",
        "[[:space:]]",
        "[[:punct:]]x",
        "?",
        "??",
        "a?",
        "*.TXT",
        "*.[tT][xX][tT]",
        "é.md",
        "あ",
        "a[[:alpha:]]",
        "[[:digit:]]?",
        "[^a-c]*",
        "abc/*",
        "abc/**/g",
        "/d/e/",
        "e/",
        "[B]",
        "[Q]",
        "[b]",
        "?[[:space:]]z",
        "?[[:cntrl:]]z",
        "?[[:blank:]]z",
        "?[[:print:]]z",
        "?[[:graph:]]z",
        "?[[:punct:]]z",
        "b\\z",
        "b\\\\z",
        "b[\\\\]z",
        "b[\\]z",
        "?[!a-z]z",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    if let Err(e) = run_rules("fixed", &rules) {
        panic!("git との差:\n{e}");
    }
}

/// ランダムに組んだ 1〜2 行の規則で、git の判定と一致する (集合・POSIX の集合・エスケープ・`**` の位置・大文字小文字の
/// 畳み方の細目を、git の実行結果と突き合わせる)。種と数は `EVE_GITIGNORE_SEED` / `EVE_GITIGNORE_DIRS` で変えられる。
// verifies: AC-FR-125-1
#[test]
fn ランダムな規則で_git_と一致する() {
    let seed = std::env::var("EVE_GITIGNORE_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20_261_121);
    let dirs = std::env::var("EVE_GITIGNORE_DIRS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200);
    if let Err(e) = run_rules("random", &random_rules(seed, dirs)) {
        panic!("git との差 (種 {seed}):\n{e}");
    }
}
