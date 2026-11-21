//! ワークスペースのルートの検出と、起動の引数のディレクトリ (docs/03 §12.7.1、07 §12.1 / §18.2 / §18.3。v0.5 の M31 の段 1)。
//!
//! 規則は前から順に `--workspace` → 位置引数の最初のディレクトリ (`DIR`) → 目印 → カレントディレクトリ。偽 FS の
//! カレントディレクトリは `/w` (`FixedPaths`)。

use eve_core::{FsPath, InitialFile, JobResult, RootSource, StartupConfig};
use eve_testkit::Harness;

/// 起動の前の Harness (何も開いていない)。`dirs` と `files` を偽 FS に置く。
fn env(dirs: &[&str], files: &[&str]) -> Harness {
    let mut b = Harness::builder().defer_start();
    for d in dirs {
        b = b.dir(d);
    }
    for f in files {
        b = b.file(f, b"text\n");
    }
    b.build()
}

/// `--workspace` を渡す (Host が `HostInit.workspace` から渡すのと同じ経路)。
fn with_workspace(h: &mut Harness, raw: &str) {
    h.editor.configure(StartupConfig {
        workspace: Some(raw.to_owned()),
        ..Default::default()
    });
}

/// 起動して、積まれたジョブをすべて走らせる。
fn start(h: &mut Harness, paths: &[&str]) {
    let files: Vec<InitialFile> = paths.iter().map(|p| InitialFile::path(*p)).collect();
    h.editor.open_initial_files(&files, false, None);
    h.started();
    h.run_jobs();
}

fn root(h: &Harness) -> (FsPath, RootSource) {
    let w = h
        .editor
        .state()
        .workspace()
        .expect("ルートの検出の結果が届いている");
    (w.root().clone(), w.source())
}

fn count(h: &Harness, key: &str) -> usize {
    h.messages().iter().filter(|k| *k == key).count()
}

const IGNORED: &str = "msg.cli.directory_ignored";
const NOT_A_FILE: &str = "msg.open.not_a_file";
const ROOT_INVALID: &str = "msg.workspace.root_invalid";

// ---- 4 つの規則 (07 §12.1) ----

// verifies: AC-FR-125-3
#[test]
fn 規則_1_workspace_で明示したディレクトリがルートになる() {
    let mut h = env(&["/w/ws", "/w/repo/.git"], &["/w/repo/a.txt"]);
    with_workspace(&mut h, "/w/ws");
    start(&mut h, &["/w/repo/a.txt"]);
    assert_eq!(root(&h), (FsPath::from("/w/ws"), RootSource::Explicit));
    assert!(h.messages().is_empty(), "{:?}", h.messages());
    assert_eq!(
        h.doc_path().as_deref(),
        Some("/w/repo/a.txt"),
        "ファイルは開く"
    );
}

// verifies: AC-FR-125-3
#[test]
fn 規則_1_相対パスはカレントディレクトリから() {
    let mut h = env(&["/w/ws"], &[]);
    with_workspace(&mut h, "ws");
    start(&mut h, &[]);
    assert_eq!(root(&h), (FsPath::from("/w/ws"), RootSource::Explicit));
}

// verifies: AC-FR-125-3
#[test]
fn 規則_2_位置引数の最初のディレクトリがルートになり_知らせは出さない() {
    let mut h = env(&["/w/proj/.git", "/w/proj/sub"], &[]);
    start(&mut h, &["/w/proj/sub"]);
    assert_eq!(root(&h), (FsPath::from("/w/proj/sub"), RootSource::Dir));
    assert_eq!(count(&h, IGNORED), 0);
    assert_eq!(
        count(&h, NOT_A_FILE),
        0,
        "v0.4 までの not_a_file は出さない"
    );
    // DIR だけなら無題の文書 1 つ
    assert_eq!(h.editor.state().documents_len(), 1);
    assert_eq!(h.doc_path(), None, "無題");
}

// verifies: AC-FR-125-3
#[test]
fn 規則_2_ファイルと一緒に書ける() {
    let mut h = env(&["/w/proj"], &["/w/proj/src/main.rs"]);
    start(&mut h, &["/w/proj", "/w/proj/src/main.rs"]);
    assert_eq!(root(&h), (FsPath::from("/w/proj"), RootSource::Dir));
    assert_eq!(h.editor.state().documents_len(), 1);
    assert_eq!(
        h.doc_path().as_deref(),
        Some("/w/proj/src/main.rs"),
        "ファイルを開いて表示する"
    );
    assert!(h.messages().is_empty(), "{:?}", h.messages());
}

// verifies: AC-FR-125-3
#[test]
fn 規則_3_ファイルの親から上へ目印を探す() {
    let mut h = env(&["/w/repo/.git"], &["/w/repo/src/deep/a.txt"]);
    start(&mut h, &["/w/repo/src/deep/a.txt"]);
    assert_eq!(root(&h), (FsPath::from("/w/repo"), RootSource::Marker));
}

#[test]
fn 規則_3_深いほうの目印が勝ち_ファイルの目印も数える() {
    // `git worktree` とサブモジュールの `.git` はファイル。`.eve` と `.git` の順は問わない (深いほうが勝つ)
    let mut h = env(
        &["/w/proj/.git"],
        &["/w/proj/sub/.git", "/w/proj/sub/x/a.txt"],
    );
    start(&mut h, &["/w/proj/sub/x/a.txt"]);
    assert_eq!(root(&h), (FsPath::from("/w/proj/sub"), RootSource::Marker));
}

#[test]
fn 規則_3_起点はファイルの親の共通の祖先() {
    // a と b の共通の祖先 /w/repo/src から上へ探す (/w/repo/src/a の中の目印は見ない)
    let mut h = env(
        &["/w/repo/.hg", "/w/repo/src/a/.svn"],
        &["/w/repo/src/a/x.txt", "/w/repo/src/b/y.txt"],
    );
    start(&mut h, &["/w/repo/src/a/x.txt", "/w/repo/src/b/y.txt"]);
    assert_eq!(root(&h), (FsPath::from("/w/repo"), RootSource::Marker));
}

#[test]
fn 規則_3_ファイルが無ければカレントディレクトリから探す() {
    // カレントディレクトリ /w の上の / に目印がある
    let mut h = env(&["/.eve"], &[]);
    start(&mut h, &[]);
    assert_eq!(root(&h), (FsPath::from("/"), RootSource::Marker));
}

#[test]
fn 規則_3_新規のファイルでも親から探す() {
    let mut h = env(&["/w/repo/.git", "/w/repo/src"], &[]);
    start(&mut h, &["/w/repo/src/new.txt"]);
    assert_eq!(root(&h), (FsPath::from("/w/repo"), RootSource::Marker));
}

// verifies: AC-FR-125-3
#[test]
fn 規則_4_目印が無ければカレントディレクトリ() {
    let mut h = env(&["/x/y"], &["/x/y/a.txt"]);
    start(&mut h, &["/x/y/a.txt"]);
    assert_eq!(root(&h), (FsPath::from("/w"), RootSource::Cwd));

    let mut h = env(&[], &[]);
    start(&mut h, &[]);
    assert_eq!(root(&h), (FsPath::from("/w"), RootSource::Cwd), "引数なし");
}

// ---- ディレクトリの引数が 2 つ以上 (07 §18.2) ----

// verifies: AC-FR-125-3
#[test]
fn ディレクトリが_2_つなら最初をルートにし_2_つ目は知らせて開かない() {
    let mut h = env(&["/w/d1", "/w/d2"], &[]);
    start(&mut h, &["/w/d1", "/w/d2"]);
    assert_eq!(root(&h), (FsPath::from("/w/d1"), RootSource::Dir));
    assert_eq!(count(&h, IGNORED), 1, "{:?}", h.messages());
    assert_eq!(count(&h, NOT_A_FILE), 0);
    assert_eq!(h.editor.state().documents_len(), 1, "無題の文書 1 つ");
    assert_eq!(h.doc_path(), None);
    assert!(
        h.frame_contains("d2"),
        "知らせは 2 つ目のディレクトリを示す"
    );
}

#[test]
fn workspace_があれば位置引数のディレクトリはどれも知らせて開かない() {
    let mut h = env(&["/w/ws", "/w/d1", "/w/d2"], &["/w/a.txt"]);
    with_workspace(&mut h, "/w/ws");
    start(&mut h, &["/w/d1", "/w/a.txt", "/w/d2"]);
    assert_eq!(root(&h), (FsPath::from("/w/ws"), RootSource::Explicit));
    assert_eq!(count(&h, IGNORED), 2, "{:?}", h.messages());
    assert_eq!(h.editor.state().documents_len(), 1);
    assert_eq!(h.doc_path().as_deref(), Some("/w/a.txt"));
}

// ---- `--workspace` を使えないとき (07 §18.3) ----

#[test]
fn workspace_がファイルなら知らせて次の規則で決める() {
    let mut h = env(&["/w/d1"], &["/w/file.txt"]);
    with_workspace(&mut h, "/w/file.txt");
    start(&mut h, &["/w/d1"]);
    assert_eq!(root(&h), (FsPath::from("/w/d1"), RootSource::Dir));
    assert_eq!(count(&h, ROOT_INVALID), 1);
    assert_eq!(count(&h, IGNORED), 0, "ルートにした DIR には出さない");
    assert_eq!(
        h.messages().last().map(String::as_str),
        Some(ROOT_INVALID),
        "利用者が明示した指定の知らせを最後に出す"
    );
    assert!(h.frame_contains("not a directory"), "理由を示す");
}

#[test]
fn workspace_が無ければ知らせて目印かカレントディレクトリで決める() {
    let mut h = env(&["/w/repo/.git"], &["/w/repo/a.txt"]);
    with_workspace(&mut h, "/w/none");
    start(&mut h, &["/w/repo/a.txt"]);
    assert_eq!(root(&h), (FsPath::from("/w/repo"), RootSource::Marker));
    assert_eq!(count(&h, ROOT_INVALID), 1);
    assert_eq!(h.doc_path().as_deref(), Some("/w/repo/a.txt"));
}

// ---- 最初の Frame を遅らせない (01 FR-125 AC-3、03 §12.7.1) ----

#[test]
fn ルートの検出は起動時のパスを開くジョブの後ろのジョブで行う() {
    let mut h = env(&["/w/proj"], &["/w/a.txt", "/w/b.txt"]);
    let files = [
        InitialFile::path("/w/a.txt"),
        InitialFile::path("/w/proj"),
        InitialFile::path("/w/b.txt"),
    ];
    h.editor.open_initial_files(&files, false, None);
    h.started();
    // 起動の中では決めない (ジョブで決める)。最初の Frame は描ける
    assert!(h.editor.state().workspace().is_none());
    assert!(h.frame_contains("a.txt"));
    // 結果の届く順: 開くジョブ 3 つ → ルートの検出
    let mut order = Vec::new();
    h.run_jobs_with(|out| match &out.body {
        JobResult::Complete { .. } | JobResult::Failed { .. } => order.push("open"),
        JobResult::WorkspaceRoot { .. } => order.push("root"),
        _ => {}
    });
    assert_eq!(order, ["open", "open", "open", "root"]);
    assert_eq!(root(&h), (FsPath::from("/w/proj"), RootSource::Dir));
}

#[test]
fn 標準入力はルートの検出の引数にしない() {
    let mut h = env(&["/w/repo/.git"], &[]);
    let files = [InitialFile {
        target: eve_core::InitialTarget::Stdin(eve_core::StdinKind::Terminal),
        line: None,
        col: None,
    }];
    h.editor.open_initial_files(&files, false, None);
    h.started();
    h.run_jobs();
    assert_eq!(root(&h), (FsPath::from("/w"), RootSource::Cwd));
}

#[test]
fn 起動の後にファイルを開く操作でフォルダを指定したときは今までどおり() {
    let mut h = env(&["/w/d1"], &["/w/a.txt"]);
    start(&mut h, &["/w/a.txt"]);
    h.clear_log();
    h.open("/w/d1");
    h.run_jobs();
    assert_eq!(count(&h, NOT_A_FILE), 1);
    assert_eq!(count(&h, IGNORED), 0);
}
