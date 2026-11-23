//! ファインダで、名前が UTF-8 で表せないファイルを選んで開く (Unix の実 FS。docs/03 §12.7.2 の「候補の持ち方と解放」、
//! §12.7.4 の「開く」)。候補は表示用の文字列 (置換文字) と元の `OsString` の両方を持ち、開くときは元の名前を使う。
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

// verifies: AC-FR-125-1
#[test]
fn utf8_で表せない名前のファイルを選んで開く() {
    let dir = TempDir(std::env::temp_dir().join(format!("eve-finder-unix-{}", std::process::id())));
    let ws = dir.0.join("ws");
    std::fs::create_dir_all(&ws).expect("作業ディレクトリ");
    let name = std::ffi::OsStr::from_bytes(b"bad\xffname.txt");
    let file = ws.join(name);
    std::fs::write(&file, b"raw bytes name\n").expect("名前が UTF-8 でないファイル");

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
        CommandInvocation::new("workspace.open_file", InvocationSource::Keymap)
            .with_args(Args::new().with_str("query", "badname")),
    );
    run_all(&mut editor, &jobs);
    let (_, _, rows, _) = editor
        .state()
        .finder_rows()
        .expect("ファインダが開いている");
    assert_eq!(rows, ["bad\u{FFFD}name.txt"], "表示は置換文字");
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
        "元の名前で開く (置換文字の名前で新規の文書にしない)"
    );
    assert_eq!(doc.to_string_lossy(), "raw bytes name\n");
}
