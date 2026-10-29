//! 起動時に複数のファイルを開く・開いた直後の位置への移動 (docs/07 §18.2、01 FR-101 / FR-102)。

use std::rc::Rc;
use std::sync::Arc;

use eve_core::{Args, CommandInvocation, Editor, InitialFile, InvocationSource, Ports};
use eve_testkit::{FixedPaths, ManualClock, ManualJobSpawner, MemFs};
use eve_view::{Capabilities, StatusKind, WindowId};

struct Env {
    e: Editor,
    fs: Arc<MemFs>,
    jobs: Rc<ManualJobSpawner>,
    win: WindowId,
}

impl Env {
    /// ファイルを置いた Editor (まだ何も開いていない)。
    fn new() -> Env {
        let clock = ManualClock::new();
        let fs = Arc::new(MemFs::new(Arc::new(clock.clone())));
        fs.add_dir("/w");
        fs.add_file("/w/a.txt", b"aaa\nbbb\nccc\n");
        fs.add_file("/w/b.txt", b"b1\nb2\nb3\n");
        fs.add_file("/w/c.txt", "日本語\nあいうえお\n".as_bytes());
        let paths = Arc::new(FixedPaths::default());
        let jp = eve_core::JobPorts {
            fs: fs.clone(),
            clock: Arc::new(clock.clone()),
            paths: paths.clone(),
        };
        let jobs = ManualJobSpawner::new(jp);
        let ports = Ports {
            fs: fs.clone(),
            clock: Arc::new(clock),
            jobs: Rc::clone(&jobs) as Rc<dyn eve_core::JobSpawner>,
            paths,
        };
        let mut e = Editor::new(ports, Capabilities::default());
        // 最初の Resize の前でもウィンドウは `open_initial` で作られる
        let _ = &mut e;
        let win = WindowId::default();
        Env { e, fs, jobs, win }
    }

    fn start(&mut self, files: &[InitialFile], read_only: bool) {
        self.e.open_initial_files(files, read_only, None);
        self.win = self.e.state().first_window().unwrap();
        self.pump();
    }

    fn pump(&mut self) {
        loop {
            self.jobs.run_jobs();
            let rs = self.jobs.take_results();
            if rs.is_empty() {
                break;
            }
            for r in rs {
                self.e.on_job_result(r);
            }
        }
    }

    fn run(&mut self, id: &str) {
        self.e
            .execute(CommandInvocation::new(id, InvocationSource::Keymap).with_args(Args::new()));
        self.pump();
    }

    fn status(&mut self, kind: StatusKind) -> String {
        let f = self.e.frame(self.win).unwrap();
        let item = f.status.items.iter().find(|i| i.kind == kind).unwrap();
        item.long.spans.iter().map(|s| s.text.as_str()).collect()
    }
}

fn file(path: &str) -> InitialFile {
    InitialFile {
        path: path.to_owned(),
        line: None,
        col: None,
    }
}

fn at(path: &str, line: usize, col: Option<usize>) -> InitialFile {
    InitialFile {
        path: path.to_owned(),
        line: Some(line),
        col,
    }
}

#[test]
fn 複数のパスを渡すと_その数のバッファが開き_最初のものが表示される() {
    let mut s = Env::new();
    s.start(
        &[file("/w/a.txt"), file("/w/b.txt"), file("/w/c.txt")],
        false,
    );
    assert_eq!(s.e.state().documents_len(), 3);
    assert_eq!(s.status(StatusKind::Path), "a.txt (1/3)");
    s.run("buffer.next");
    assert_eq!(s.status(StatusKind::Path), "b.txt (2/3)");
    s.run("buffer.next");
    assert_eq!(s.status(StatusKind::Path), "c.txt (3/3)");
    // 全部読み終わっていて、編集できる
    let id = s.e.state().active_document().unwrap();
    assert_eq!(
        s.e.state().document(id).unwrap().to_string_lossy(),
        "日本語\nあいうえお\n"
    );
}

#[test]
fn 読込が終わる前でも_最初のファイルを表示し_追加のバッファへ切り替えられる() {
    let mut s = Env::new();
    s.e.open_initial_files(&[file("/w/a.txt"), file("/w/b.txt")], false, None);
    s.win = s.e.state().first_window().unwrap();
    // ジョブはまだ走っていない
    assert_eq!(s.e.state().documents_len(), 2);
    s.run("buffer.next");
    s.run("buffer.prev");
    assert_eq!(s.e.state().documents_len(), 2, "読込中でも切替できる");
    s.pump();
    assert_eq!(s.status(StatusKind::Path), "a.txt (1/2)");
}

#[test]
fn 同じパスは_1_つのバッファだけ開く() {
    let mut s = Env::new();
    s.start(
        &[file("/w/a.txt"), file("/w/b.txt"), file("/w/b.txt")],
        false,
    );
    assert_eq!(s.e.state().documents_len(), 2);
}

#[test]
fn readonly_は全パスに掛かる() {
    let mut s = Env::new();
    s.start(&[file("/w/a.txt"), file("/w/b.txt")], true);
    for _ in 0..2 {
        let id = s.e.state().active_document().unwrap();
        assert!(
            !s.e.state().document(id).unwrap().read_only.is_empty(),
            "読み取り専用"
        );
        s.run("buffer.next");
    }
}

#[test]
fn 開いた直後に指定の行と桁へ移る() {
    let mut s = Env::new();
    s.start(
        &[
            at("/w/a.txt", 2, Some(2)),
            at("/w/b.txt", 3, None),
            at("/w/c.txt", 2, Some(3)),
        ],
        false,
    );
    assert_eq!(s.status(StatusKind::Position), "行 2, 桁 2");
    s.run("buffer.next");
    assert_eq!(s.status(StatusKind::Position), "行 3, 桁 1", "桁を省くと 1");
    s.run("buffer.next");
    assert_eq!(
        s.status(StatusKind::Position),
        "行 2, 桁 3",
        "桁は書記素の数 (あい|う)"
    );
}

#[test]
fn 範囲外の行と桁は末尾に丸める() {
    let mut s = Env::new();
    s.start(&[at("/w/a.txt", 99, Some(99))], false);
    // 末尾の改行の後の空行 (4 行目) の先頭
    assert_eq!(s.status(StatusKind::Position), "行 4, 桁 1");

    let mut s = Env::new();
    s.start(&[at("/w/a.txt", 1, Some(99))], false);
    assert_eq!(s.status(StatusKind::Position), "行 1, 桁 4", "行末");
}

#[test]
fn 追加のファイルが開けなくても_表示中のバッファはそのまま() {
    let mut s = Env::new();
    // フォルダは開けない
    s.start(&[file("/w/a.txt"), file("/w")], false);
    assert_eq!(s.e.state().documents_len(), 1, "開けなかったものは閉じる");
    assert_eq!(s.status(StatusKind::Path), "a.txt");
    let _ = &s.fs;
}
