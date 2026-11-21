//! ワークスペースの走査 `WorkspaceWalk` とジョブ `WorkspaceScan` / `ReleaseCandidates` の試験 (docs/03 §12.7.2 / §12.7.7 /
//! §12.7.8、09 §3.4 の「ワークスペースの試験」の (3))。`MemFs` の `read_dir` は名前の逆順で返すので、走査が自分で並べることも
//! 確かめられる。

use std::sync::Arc;

use eve_core::{
    CANDIDATE_CHUNK, Candidate, ExcludeConfig, FileSystem, FsErrorKind, FsPath, JobError, JobKind,
    JobOutput, JobPorts, JobResult, JobSpawner, Millis, PathKind, WALK_DIRS_PER_STEP,
    WALK_ENTRIES_PER_STEP, WORKSPACE_MAX_DEPTH, WORKSPACE_MAX_FILES, WalkObserver, WalkStep,
    WorkspaceWalk,
};
use eve_testkit::{FixedPaths, ManualClock, ManualJobSpawner, MemFs};
use eve_view::Revision;

fn fs() -> (Arc<MemFs>, ManualClock) {
    let clock = ManualClock::new();
    (Arc::new(MemFs::new(Arc::new(clock.clone()))), clock)
}

fn config(exclude: &[&str]) -> Arc<ExcludeConfig> {
    Arc::new(ExcludeConfig {
        exclude: exclude.iter().map(|s| (*s).to_owned()).collect(),
        ..ExcludeConfig::new()
    })
}

/// 走査で集めたもの。
#[derive(Default)]
struct Seen {
    excluded: Vec<(String, PathKind)>,
    nested: Vec<String>,
}

impl WalkObserver for Seen {
    fn excluded(&mut self, rel: &[u8], kind: PathKind) {
        self.excluded
            .push((String::from_utf8_lossy(rel).into_owned(), kind));
    }
    fn nested_repo(&mut self, rel: &[u8]) {
        self.nested.push(String::from_utf8_lossy(rel).into_owned());
    }
}

/// 終わりまで走らせ、候補の相対パス (見つけた順) と区切りの数を返す。
fn walk_all(
    fs: &MemFs,
    clock: &ManualClock,
    root: &str,
    cfg: Arc<ExcludeConfig>,
) -> (Vec<String>, WorkspaceWalk, Seen, usize) {
    let mut w = WorkspaceWalk::new(FsPath::from(root), cfg);
    let mut seen = Seen::default();
    let mut steps = 0;
    let mut out = Vec::new();
    loop {
        steps += 1;
        let step = w.step_observed(fs, clock, &mut seen);
        out.extend(w.take_pending().iter().map(|c| c.rel().to_owned()));
        match step {
            WalkStep::Yield => {}
            WalkStep::Done => break,
            WalkStep::Failed(e) => panic!("走査に失敗: {:?}", e.kind),
        }
        assert!(steps < 100_000, "終わらない");
    }
    (out, w, seen, steps)
}

fn sorted(v: &[String]) -> Vec<String> {
    let mut v = v.to_vec();
    v.sort();
    v
}

#[test]
fn 深さ優先で名前のバイト列の順に並べる() {
    let (fs, clock) = fs();
    for p in [
        "/w/b.txt",
        "/w/a/z.rs",
        "/w/a/b/c.rs",
        "/w/a/B.rs",
        "/w/A.md",
        "/w/c/x",
        "/w/.env",
    ] {
        fs.add_file(p, b"x");
    }
    fs.add_dir("/w/empty");
    let (got, w, _, _) = walk_all(&fs, &clock, "/w", config(&[]));
    // MemFs は逆順に返す: 走査が並べ直している
    assert_eq!(
        got,
        [
            ".env", "A.md", "a/B.rs", "a/b/c.rs", "a/z.rs", "b.txt", "c/x"
        ],
        "深さ優先・バイト列の順 (`.` で始まる名前も出す)"
    );
    assert_eq!(w.found(), 7);
    assert!(!w.truncated());
    assert_eq!(w.unreadable_dirs(), 0);
    assert!(w.is_finished());
}

#[test]
fn git_と保存の一時ファイルとその他の項目は候補にしない() {
    let (fs, clock) = fs();
    fs.add_file("/w/.git/HEAD", b"ref");
    fs.add_file("/w/a.txt", b"x");
    fs.add_file("/w/.a.txt.eve-tmp-12-abcdefgh", b"x");
    fs.add_file("/w/.a.txt.eve-bak-12-abcdefgh", b"x");
    fs.add_special("/w/fifo");
    let (got, ..) = walk_all(&fs, &clock, "/w", config(&[]));
    assert_eq!(got, ["a.txt"]);
}

#[test]
fn gitignore_は_git_のあるときだけ効く() {
    let (fs, clock) = fs();
    fs.add_file("/w/.gitignore", b"*.log\nbuild/\n!keep.log\n");
    fs.add_file("/w/a.log", b"x");
    fs.add_file("/w/keep.log", b"x");
    fs.add_file("/w/build/out.o", b"x");
    fs.add_file("/w/src/build", b"file named build");
    fs.add_file("/w/src/main.rs", b"x");
    // リポジトリが無い: .gitignore は効かない
    let (got, ..) = walk_all(&fs, &clock, "/w", config(&[]));
    assert_eq!(
        sorted(&got),
        [
            ".gitignore",
            "a.log",
            "build/out.o",
            "keep.log",
            "src/build",
            "src/main.rs"
        ]
    );
    // リポジトリがある
    fs.add_dir("/w/.git");
    let (got, _, seen, _) = walk_all(&fs, &clock, "/w", config(&[]));
    assert_eq!(
        sorted(&got),
        [".gitignore", "keep.log", "src/build", "src/main.rs"],
        "ディレクトリだけのパターンはファイルの build に当たらない"
    );
    assert!(seen.excluded.contains(&("build".to_owned(), PathKind::Dir)));
    assert!(
        seen.excluded
            .contains(&("a.log".to_owned(), PathKind::File))
    );
    assert!(
        !seen.excluded.iter().any(|(p, _)| p.starts_with("build/")),
        "除外したディレクトリには降りない"
    );
}

#[test]
fn workspace_exclude_は_gitignore_より先でリポジトリが無くても効く() {
    let (fs, clock) = fs();
    fs.add_file("/w/node_modules/x/index.js", b"x");
    fs.add_file("/w/sub/node_modules/y.js", b"x");
    fs.add_file("/w/.env", b"x");
    fs.add_file("/w/.env.example", b"x");
    fs.add_file("/w/a.rs", b"x");
    let cfg = config(&["**/node_modules", ".env*", "!.env.example"]);
    let (got, ..) = walk_all(&fs, &clock, "/w", cfg.clone());
    assert_eq!(sorted(&got), [".env.example", "a.rs"]);
    // gitignore が除外したものを `!` で戻せる
    fs.add_dir("/w/.git");
    fs.add_file("/w/.gitignore", b"*.rs\n");
    let cfg = config(&["!a.rs"]);
    let (got, ..) = walk_all(&fs, &clock, "/w", cfg);
    assert!(got.contains(&"a.rs".to_owned()), "{got:?}");
}

#[test]
fn respect_gitignore_を切ると_gitignore_と_info_exclude_を読まない() {
    let (fs, clock) = fs();
    fs.add_file("/w/.git/info/exclude", b"*.tmp\n");
    fs.add_file("/w/.gitignore", b"*.log\n");
    fs.add_file("/w/a.log", b"x");
    fs.add_file("/w/b.tmp", b"x");
    fs.add_file("/w/c.bak", b"x");
    let cfg = Arc::new(ExcludeConfig {
        exclude: vec!["*.bak".to_owned()],
        respect_gitignore: false,
        follow_symlinks: false,
    });
    let (got, ..) = walk_all(&fs, &clock, "/w", cfg);
    assert_eq!(sorted(&got), [".gitignore", "a.log", "b.tmp"]);
    let (got, ..) = walk_all(&fs, &clock, "/w", config(&[]));
    assert_eq!(sorted(&got), [".gitignore", "c.bak"]);
}

#[test]
fn core_ignorecase_に従って_ascii_を畳む() {
    let (fs, clock) = fs();
    fs.add_file("/w/.gitignore", b"*.log\n");
    fs.add_file("/w/FOO.LOG", b"x");
    fs.add_file("/w/a.log", b"x");
    fs.add_file("/w/.git/config", b"[core]\n\tignorecase = true\n");
    let (got, ..) = walk_all(&fs, &clock, "/w", config(&[]));
    assert_eq!(sorted(&got), [".gitignore"]);
    fs.add_file("/w/.git/config", b"[Core]\n\tIgnoreCase = false\n");
    let (got, ..) = walk_all(&fs, &clock, "/w", config(&[]));
    assert_eq!(sorted(&got), [".gitignore", "FOO.LOG"]);
    // キーが無ければ区別する (git の既定)
    fs.add_file("/w/.git/config", b"[core]\n\tbare = false\n");
    let (got, ..) = walk_all(&fs, &clock, "/w", config(&[]));
    assert_eq!(sorted(&got), [".gitignore", "FOO.LOG"]);
}

#[test]
fn 入れ子のリポジトリには外側の規則を効かせない() {
    let (fs, clock) = fs();
    fs.add_file("/w/.git/info/exclude", b"*.tmp\n");
    fs.add_file("/w/.gitignore", b"*.log\n");
    fs.add_file("/w/a.log", b"x");
    fs.add_file("/w/sub/.git/HEAD", b"x");
    fs.add_file("/w/sub/.gitignore", b"*.txt\n");
    fs.add_file("/w/sub/x.log", b"x");
    fs.add_file("/w/sub/y.tmp", b"x");
    fs.add_file("/w/sub/z.txt", b"x");
    fs.add_file("/w/sub/deep/w.txt", b"x");
    let cfg = config(&["*.md"]);
    fs.add_file("/w/sub/r.md", b"x");
    let (got, _, seen, _) = walk_all(&fs, &clock, "/w", cfg);
    assert_eq!(
        sorted(&got),
        [".gitignore", "sub/.gitignore", "sub/x.log", "sub/y.tmp"],
        "外側の .gitignore と info/exclude は境目で効かなくなる。workspace.exclude は全体に効く"
    );
    assert_eq!(seen.nested, ["sub"]);
}

#[test]
fn git_がファイルの_worktree_は共通の_git_ディレクトリの_info_exclude_を読む() {
    let (fs, clock) = fs();
    fs.add_file("/r/.git/info/exclude", b"*.secret\n");
    fs.add_file("/r/.git/worktrees/wt/commondir", b"../..\n");
    fs.add_file("/r/.git/worktrees/wt/HEAD", b"x");
    fs.add_file("/r/wt/.git", b"gitdir: ../.git/worktrees/wt\n");
    fs.add_file("/r/wt/a.secret", b"x");
    fs.add_file("/r/wt/b.txt", b"x");
    let (got, ..) = walk_all(&fs, &clock, "/r/wt", config(&[]));
    assert_eq!(sorted(&got), ["b.txt"]);
    // UNC の行き先は辿らない (源を飛ばす)
    fs.add_file("/r/wt/.git", b"gitdir: //server/share/x\n");
    let (got, ..) = walk_all(&fs, &clock, "/r/wt", config(&[]));
    assert_eq!(sorted(&got), ["a.secret", "b.txt"]);
}

#[test]
fn ルートがリポジトリの中のフォルダなら祖先の_gitignore_も読む() {
    let (fs, clock) = fs();
    fs.add_dir("/r/.git");
    fs.add_file("/r/.gitignore", b"*.bak\n/ws/gen/\n");
    fs.add_file("/r/ws/.gitignore", b"*.tmp\n");
    fs.add_file("/r/ws/a.bak", b"x");
    fs.add_file("/r/ws/b.tmp", b"x");
    fs.add_file("/r/ws/gen/c.rs", b"x");
    fs.add_file("/r/ws/build/d.o", b"x");
    fs.add_file("/r/ws/e.rs", b"x");
    // workspace.exclude の置き場所はワークスペースのルート
    let (got, ..) = walk_all(&fs, &clock, "/r/ws", config(&["/build/"]));
    assert_eq!(sorted(&got), [".gitignore", "e.rs"]);
}

#[test]
fn シンボリックリンクは既定で降りず候補にする() {
    let (fs, clock) = fs();
    fs.add_file("/w/real/a.txt", b"x");
    fs.add_link("/w/link", "/w/real");
    fs.add_link("/w/loop", "/w");
    let (got, ..) = walk_all(&fs, &clock, "/w", config(&[]));
    assert_eq!(got, ["link", "loop", "real/a.txt"]);
    // follow_symlinks = true: ディレクトリなら降りる。同じディレクトリに 2 度降りない (ルートを指すリンクで木を 2 回走査しない)
    let cfg = Arc::new(ExcludeConfig {
        follow_symlinks: true,
        ..ExcludeConfig::new()
    });
    let (got, ..) = walk_all(&fs, &clock, "/w", cfg);
    assert_eq!(
        got,
        ["link/a.txt", "real/a.txt"],
        "ルートを指す loop には降りない。覚えるのはリンクで辿ったディレクトリだけ"
    );
}

#[test]
fn 区切りはディレクトリ_16_個ごと() {
    let (fs, clock) = fs();
    for i in 0..40 {
        fs.add_file(&format!("/w/d{i:02}/f.txt"), b"x");
    }
    let mut w = WorkspaceWalk::new(FsPath::from("/w"), config(&[]));
    assert!(matches!(w.step(&*fs, &clock), WalkStep::Yield));
    // ルート + 15 個のディレクトリを読んだ。読んだディレクトリのファイルのうち、処理し終えたもの
    assert_eq!(w.take_pending().len(), WALK_DIRS_PER_STEP - 2);
    let mut steps = 1;
    let mut rest = 0;
    loop {
        steps += 1;
        let s = w.step(&*fs, &clock);
        rest += w.take_pending().len();
        if matches!(s, WalkStep::Done) {
            break;
        }
    }
    assert_eq!(rest, 40 - (WALK_DIRS_PER_STEP - 2));
    assert_eq!(w.found(), 40);
    assert_eq!(steps, 3, "41 個のディレクトリを 16 個ずつ");
}

#[test]
fn 区切りの時間を過ぎたらそのディレクトリの後で区切る() {
    let (fs, clock) = fs();
    for i in 0..5 {
        fs.add_file(&format!("/w/d{i}/.gitignore"), b"*.o\n");
        fs.add_file(&format!("/w/d{i}/a.rs"), b"x");
    }
    fs.add_dir("/w/.git");
    // .gitignore を読むたびに時計が 10 ms 進む
    fs.set_read_delay(Some((clock.clone(), Millis(10))));
    let (got, _, _, steps) = walk_all(&fs, &clock, "/w", config(&[]));
    assert_eq!(got.len(), 10);
    assert_eq!(
        steps, 6,
        "ルートの後と、.gitignore のある各ディレクトリの後で区切る"
    );
}

#[test]
fn 大きなディレクトリは_4096_項目ごとに区切る() {
    let (fs, clock) = fs();
    let n = WALK_ENTRIES_PER_STEP * 2 + 100;
    for i in 0..n {
        fs.add_file(&format!("/w/f{i:05}"), b"");
    }
    let mut w = WorkspaceWalk::new(FsPath::from("/w"), config(&[]));
    let mut sizes = Vec::new();
    loop {
        let s = w.step(&*fs, &clock);
        sizes.push(w.take_pending().len());
        if matches!(s, WalkStep::Done) {
            break;
        }
    }
    assert_eq!(
        sizes,
        [WALK_ENTRIES_PER_STEP, WALK_ENTRIES_PER_STEP, 100],
        "1 回目はルートを読んで 4,096 項目まで"
    );
}

// verifies: AC-FR-125-3
#[test]
fn 読めないディレクトリは数えて走査を続ける() {
    let (fs, clock) = fs();
    fs.add_file("/w/a/x.txt", b"x");
    fs.add_file("/w/b/y.txt", b"x");
    fs.add_file("/w/c/z.txt", b"x");
    fs.set_unreadable("/w/b", true);
    let (got, w, ..) = walk_all(&fs, &clock, "/w", config(&[]));
    assert_eq!(got, ["a/x.txt", "c/z.txt"]);
    assert_eq!(w.unreadable_dirs(), 1);
    assert!(!w.truncated(), "読めないだけでは打ち切りの印を立てない");
}

#[test]
fn ルートが読めなければ失敗にする() {
    let (fs, clock) = fs();
    fs.add_file("/w/a.txt", b"x");
    fs.set_unreadable("/w", true);
    let mut w = WorkspaceWalk::new(FsPath::from("/w"), config(&[]));
    match w.step(&*fs, &clock) {
        WalkStep::Failed(e) => assert_eq!(e.kind, FsErrorKind::PermissionDenied),
        other => panic!("{other:?}"),
    }
    assert!(w.is_finished());
}

// verifies: AC-FR-125-3
#[test]
fn 列挙が途中で失敗したディレクトリは返った項目で続け_数えて打ち切りの印を立てる() {
    let (fs, clock) = fs();
    for n in ["a", "b", "c", "d"] {
        fs.add_file(&format!("/w/d/{n}"), b"x");
    }
    fs.add_file("/w/e.txt", b"x");
    // MemFs は逆順 (d, c, b, a) に返すので、2 つ目の後で失敗すると d と c だけが返る
    fs.set_read_dir_fail_after("/w/d", Some(2));
    let (got, w, ..) = walk_all(&fs, &clock, "/w", config(&[]));
    assert_eq!(
        got,
        ["d/c", "d/d", "e.txt"],
        "返った項目も並べてから処理する"
    );
    assert_eq!(w.unreadable_dirs(), 1);
    assert!(w.truncated());
}

#[test]
fn 深さの上限より深いディレクトリには降りない() {
    let (fs, clock) = fs();
    let mut p = String::from("/w");
    for _ in 0..WORKSPACE_MAX_DEPTH {
        p.push_str("/d");
    }
    fs.add_file(&format!("{p}/ok.txt"), b"x");
    fs.add_file(&format!("{p}/d/deep.txt"), b"x");
    let (got, w, ..) = walk_all(&fs, &clock, "/w", config(&[]));
    assert_eq!(got.len(), 1);
    assert!(got[0].ends_with("/ok.txt"));
    assert!(w.truncated());
}

// verifies: AC-FR-125-3
#[test]
fn 候補の数の上限で打ち切る() {
    let (fs, clock) = fs();
    let half = usize::try_from(WORKSPACE_MAX_FILES / 2).expect("小さい");
    for i in 0..half {
        fs.add_file(&format!("/w/a/{i:06}"), b"");
    }
    for i in 0..=half {
        fs.add_file(&format!("/w/b/{i:06}"), b"");
    }
    let (got, w, ..) = walk_all(&fs, &clock, "/w", config(&[]));
    assert_eq!(got.len() as u64, WORKSPACE_MAX_FILES);
    assert_eq!(w.found(), WORKSPACE_MAX_FILES);
    assert!(w.truncated());
    // ちょうど上限の数なら打ち切りではない
    fs.remove_file(&FsPath::from(format!("/w/b/{half:06}").as_str()))
        .expect("消す");
    let (_, w, ..) = walk_all(&fs, &clock, "/w", config(&[]));
    assert_eq!(w.found(), WORKSPACE_MAX_FILES);
    assert!(!w.truncated());
}

#[cfg(target_os = "linux")]
#[test]
fn ルートがファイルシステムのルートなら疑似_fs_に降りない() {
    let (fs, clock) = fs();
    for p in [
        "/proc/1/status",
        "/sys/x",
        "/dev/null0",
        "/run/a",
        "/home/u/a.txt",
        "/x/proc/b",
    ] {
        fs.add_file(p, b"x");
    }
    let (got, ..) = walk_all(&fs, &clock, "/", config(&[]));
    assert_eq!(got, ["home/u/a.txt", "x/proc/b"]);
}

// ---------- ジョブ (ManualJobSpawner。区切って譲る経路) ----------

fn spawner(fs: &Arc<MemFs>, clock: &ManualClock) -> std::rc::Rc<ManualJobSpawner> {
    ManualJobSpawner::new(JobPorts {
        stdin: None,
        fs: fs.clone(),
        clock: Arc::new(clock.clone()),
        paths: Arc::new(FixedPaths::default()),
    })
}

fn scan(generation: u64) -> JobKind {
    JobKind::WorkspaceScan {
        root: FsPath::from("/w"),
        exclude: config(&[]),
        generation,
        walk: None,
    }
}

#[test]
fn 走査のジョブは区切りごとに末尾へ積み直し_間に入ったジョブが先に走る() {
    let (fs, clock) = fs();
    for i in 0..40 {
        fs.add_file(&format!("/w/d{i:02}/f.txt"), b"x");
    }
    let sp = spawner(&fs, &clock);
    let id = sp.spawn(None, Revision(0), scan(7));
    let meta = sp.spawn(
        None,
        Revision(0),
        JobKind::Metadata {
            path: FsPath::from("/w/d00/f.txt"),
            purpose: eve_core::MetaPurpose::ExternalCheck,
        },
    );
    // 1 区切り: 続きは Metadata の後ろへ積まれる
    assert!(sp.run_one());
    assert_eq!(sp.pending(), 2);
    assert!(
        sp.take_results().is_empty(),
        "終端も束もまだ送らない (100 ms 経っていない)"
    );
    assert!(sp.run_one());
    let r = sp.take_results();
    assert!(matches!(&r[..], [JobOutput { job, body: JobResult::Meta { .. } }] if *job == meta));
    // 残りを回す
    let steps = sp.run_jobs();
    assert_eq!(steps, 2, "41 個のディレクトリを 16 個ずつで 3 区切り");
    let r = sp.take_results();
    let mut total = 0;
    for o in &r[..r.len() - 1] {
        assert_eq!(o.job, id);
        let JobResult::ScanBatch {
            generation, chunk, ..
        } = &o.body
        else {
            panic!("{:?}", o.body);
        };
        assert_eq!(*generation, 7);
        total += chunk.len();
    }
    assert!(matches!(
        r.last().map(|o| &o.body),
        Some(JobResult::ScanDone {
            generation: 7,
            total: 40,
            truncated: false
        })
    ));
    assert_eq!(total, 40);
}

#[test]
fn 走査の途中の取消しは続きを外して_cancelled_だけを送る() {
    let (fs, clock) = fs();
    for i in 0..40 {
        fs.add_file(&format!("/w/d{i:02}/f.txt"), b"x");
    }
    let sp = spawner(&fs, &clock);
    let id = sp.spawn(None, Revision(0), scan(1));
    assert!(sp.run_one());
    sp.cancel(id);
    assert_eq!(sp.pending(), 0);
    assert_eq!(sp.run_jobs(), 0);
    let r = sp.take_results();
    assert!(
        matches!(&r[..], [JobOutput { job, body: JobResult::Failed { err: JobError::Cancelled } }] if *job == id),
        "{r:?}"
    );
}

#[test]
fn 候補は_4096_件の塊で送り_時間が経てば少なくても送る() {
    let (fs, clock) = fs();
    let n = CANDIDATE_CHUNK + 904;
    for i in 0..n {
        fs.add_file(&format!("/w/f{i:05}"), b"");
    }
    let sp = spawner(&fs, &clock);
    sp.spawn(None, Revision(0), scan(3));
    sp.run_jobs();
    let sizes: Vec<usize> = sp
        .take_results()
        .iter()
        .filter_map(|o| match &o.body {
            JobResult::ScanBatch { chunk, .. } => Some(chunk.len()),
            _ => None,
        })
        .collect();
    assert_eq!(sizes, [CANDIDATE_CHUNK, 904]);

    // 少ないファイルでも、前に送ってから 100 ms 過ぎた区切りの終わりで送る
    let (fs, clock) = self::fs();
    for i in 0..40 {
        fs.add_file(&format!("/w/d{i:02}/f.txt"), b"x");
    }
    let sp = spawner(&fs, &clock);
    sp.spawn(None, Revision(0), scan(4));
    assert!(sp.run_one());
    assert!(sp.take_results().is_empty());
    clock.advance(Millis(100));
    assert!(sp.run_one());
    let r = sp.take_results();
    assert!(
        matches!(&r[..], [JobOutput { body: JobResult::ScanBatch { chunk, .. }, .. }] if chunk.len() == 30),
        "{r:?}"
    );
}

#[test]
fn 走査の終わりに読めなかったディレクトリの数を届ける() {
    let (fs, clock) = fs();
    fs.add_file("/w/a/x.txt", b"x");
    fs.add_dir("/w/z");
    fs.set_unreadable("/w/z", true);
    let sp = spawner(&fs, &clock);
    sp.spawn(None, Revision(0), scan(5));
    sp.run_jobs();
    let r = sp.take_results();
    assert!(matches!(
        &r[0].body,
        JobResult::ScanBatch { chunk, unreadable_dirs: 1, .. } if chunk.len() == 1
    ));
    assert!(matches!(
        &r[1].body,
        JobResult::ScanDone {
            total: 1,
            truncated: false,
            ..
        }
    ));
}

#[test]
fn ルートが読めない走査は_fs_の失敗で終わる() {
    let (fs, clock) = fs();
    let sp = spawner(&fs, &clock);
    sp.spawn(None, Revision(0), scan(6));
    sp.run_jobs();
    let r = sp.take_results();
    assert!(
        matches!(&r[..], [JobOutput { body: JobResult::Failed { err: JobError::Fs(e) }, .. }] if e.kind == FsErrorKind::NotFound),
        "{r:?}"
    );
}

#[test]
fn 候補の一覧の解放は_released_で終わる() {
    let (fs, clock) = fs();
    let sp = spawner(&fs, &clock);
    let chunk: Arc<[Candidate]> = Arc::from(vec![Candidate::new("a.txt")]);
    let weak = Arc::downgrade(&chunk);
    let id = sp.spawn(
        None,
        Revision(0),
        JobKind::ReleaseCandidates {
            chunks: vec![chunk],
        },
    );
    sp.run_jobs();
    assert!(weak.upgrade().is_none(), "ジョブの中で手放した");
    let r = sp.take_results();
    assert!(matches!(&r[..], [JobOutput { job, body: JobResult::Released }] if *job == id));
}
