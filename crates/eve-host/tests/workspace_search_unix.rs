//! 横断検索で、名前が UTF-8 で表せないファイルの一致を出してジャンプする (Unix の実 FS。docs/03 §12.7.5 の「1 つのファイルの
//! 扱い」と「ジャンプ」)。読むパスは候補の元の `OsString` から作り、表示は置換文字、開くときは元の名前を使う。辿らない
//! シンボリックリンクの先 (ワークスペースの外) は読まない。
#![cfg(unix)]

use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use eve_core::{Args, CommandInvocation, Editor, InvocationSource, JobPorts, Ports, StartupConfig};
use eve_platform::unix_fs::UnixFs;
use eve_testkit::{FixedPaths, ManualClock, ManualJobSpawner};

/// 試験ごとの作業ディレクトリ (終わったら消す)。
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run_all(editor: &mut Editor, jobs: &ManualJobSpawner) {
    loop {
        jobs.run_jobs();
        let results = jobs.take_results();
        if results.is_empty() {
            return;
        }
        for r in results {
            editor.on_job_result(r);
        }
    }
}

// verifies: AC-FR-405-1
#[test]
fn utf8_で表せない名前のファイルの一致へ移り_リンクの先は読まない() {
    let dir =
        TempDir(std::env::temp_dir().join(format!("eve-wsearch-unix-{}", std::process::id())));
    let ws = dir.0.join("ws");
    let outside = dir.0.join("outside");
    std::fs::create_dir_all(&ws).expect("作業ディレクトリ");
    std::fs::create_dir_all(&outside).expect("外のディレクトリ");
    let name = std::ffi::OsStr::from_bytes(b"bad\xffname.txt");
    std::fs::write(ws.join(name), b"first\nsecond needle\n").expect("名前が UTF-8 でないファイル");
    std::fs::write(outside.join("secret.txt"), b"needle outside\n").expect("外のファイル");
    std::os::unix::fs::symlink(outside.join("secret.txt"), ws.join("link.txt")).expect("リンク");

    let clock = ManualClock::new();
    let fs = Arc::new(UnixFs::new(dir.0.join("state")));
    let paths = Arc::new(FixedPaths::default());
    let jobs = ManualJobSpawner::new(JobPorts {
        fs: fs.clone(),
        clock: Arc::new(clock.clone()),
        paths: paths.clone(),
        stdin: None,
    });
    let mut editor = Editor::new(
        Ports {
            watcher: None,
            fs,
            clock: Arc::new(clock),
            jobs: Rc::clone(&jobs) as Rc<dyn eve_core::JobSpawner>,
            paths,
        },
        eve_view::Capabilities::default(),
    );
    editor.configure(StartupConfig {
        workspace: Some(ws.to_str().expect("一時ディレクトリは UTF-8").to_owned()),
        ..Default::default()
    });
    editor.open_initial(None, false);
    run_all(&mut editor, &jobs);

    editor.execute(
        CommandInvocation::new("search.workspace", InvocationSource::Keymap)
            .with_args(Args::new().with_str("query", "needle")),
    );
    run_all(&mut editor, &jobs);
    let (_, rows, _, progress) = editor
        .state()
        .ws_search_rows()
        .expect("横断検索が開いている");
    assert_eq!(
        rows,
        ["bad\u{FFFD}name.txt:2"],
        "表示は置換文字。リンクの先は読まない"
    );
    assert_eq!(
        progress.files, 1,
        "リンクは探したファイルに数えない: {progress:?}"
    );
    editor.execute(CommandInvocation::new(
        "ui.confirm",
        InvocationSource::Keymap,
    ));
    run_all(&mut editor, &jobs);

    let state = editor.state();
    assert_eq!(state.overlay_kind(), "", "開いたら閉じる");
    let doc = state
        .active_document()
        .and_then(|id| state.document(id))
        .expect("文書");
    let path = doc.path.as_ref().expect("パスのある文書");
    assert_eq!(
        path.as_path().file_name().map(|n| n.as_bytes().to_vec()),
        Some(b"bad\xffname.txt".to_vec()),
        "元の名前で開く"
    );
    assert_eq!(doc.to_string_lossy(), "first\nsecond needle\n");
    let view = match editor.query(&eve_view::Query::Windows).expect("Window") {
        eve_view::Answer::Windows(ws) => ws[0].views[0],
        a => panic!("{a:?}"),
    };
    match editor
        .query(&eve_view::Query::Selection { view })
        .expect("選択")
    {
        eve_view::Answer::Selection {
            ranges, primary, ..
        } => assert_eq!(
            (ranges[primary].anchor, ranges[primary].head),
            (13, 19),
            "2 行目の needle を選ぶ"
        ),
        a => panic!("{a:?}"),
    }
}
