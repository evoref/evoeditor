//! ファイルを開く・保存するシナリオ (docs/03 §9.2 / §9.3、01 の FR-104 / FR-124 / FR-204 / NFR-303)。
//! 保存先は `MemFs`、ジョブは `ManualJobSpawner` (テストが `pump` で実行する)。

use std::rc::Rc;
use std::sync::Arc;

use eve_core::{
    Args, CommandInvocation, CommandResult, Document, Editor, FsPath, InvocationSource, Ports,
    RejectReason, TextFormat,
};
use eve_testkit::{FixedPaths, ManualClock, ManualJobSpawner, MemFs};
use eve_view::{
    Capabilities, DocumentId, EditorEvent, InputEvent, KeyCode, KeyEvent, Modifiers, WindowId,
};

struct Env {
    e: Editor,
    fs: Arc<MemFs>,
    jobs: Rc<ManualJobSpawner>,
    clock: ManualClock,
}

impl Env {
    fn new() -> Env {
        let clock = ManualClock::new();
        let fs = Arc::new(MemFs::new(Arc::new(clock.clone())));
        fs.add_dir("/w");
        let paths = Arc::new(FixedPaths::default());
        let jp = eve_core::JobPorts {
            fs: fs.clone(),
            clock: Arc::new(clock.clone()),
            paths: paths.clone(),
        };
        let jobs = ManualJobSpawner::new(jp);
        let ports = Ports {
            fs: fs.clone(),
            clock: Arc::new(clock.clone()),
            jobs: Rc::clone(&jobs) as Rc<dyn eve_core::JobSpawner>,
            paths,
        };
        Env {
            e: Editor::new(ports, Capabilities::default()),
            fs,
            jobs,
            clock,
        }
    }

    /// ジョブを実行して結果を Editor に渡す。
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

    fn run(&mut self, id: &str, args: Args) -> CommandResult {
        let r = self
            .e
            .execute(CommandInvocation::new(id, InvocationSource::Keymap).with_args(args));
        self.pump();
        r
    }

    fn open(&mut self, path: &str) -> CommandResult {
        self.run("file.open", Args::new().with_str("path", path))
    }

    fn doc(&self) -> (DocumentId, &Document) {
        let id = self.e.state().active_document().expect("文書がある");
        (id, self.e.state().document(id).unwrap())
    }

    fn text(&self) -> String {
        self.doc().1.to_string_lossy()
    }

    fn win(&self) -> WindowId {
        self.e.state().first_window().unwrap()
    }

    fn type_text(&mut self, s: &str) {
        self.e.handle_input(self.win(), InputEvent::Text(s.into()));
    }

    fn key(&mut self, code: KeyCode, mods: Modifiers) {
        self.e
            .handle_input(self.win(), InputEvent::Key(KeyEvent::press(code, mods)));
    }

    fn messages(&mut self) -> Vec<String> {
        self.e
            .drain_events()
            .into_iter()
            .filter_map(|ev| match ev {
                EditorEvent::MessageShown { key, .. } => Some(key),
                _ => None,
            })
            .collect()
    }
}

#[test]
fn utf8_のファイルを開く() {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", "こんにちは\nworld\n".as_bytes());
    assert!(matches!(s.open("/w/a.txt"), CommandResult::Pending(_)));
    assert_eq!(s.text(), "こんにちは\nworld\n");
    let (_, d) = s.doc();
    assert_eq!(d.path, Some(FsPath::from("/w/a.txt")));
    assert!(!d.is_dirty() && d.is_editable());
    assert!(d.disk_stamp.is_some());
    assert!(!d.format.has_bom);
}

#[test]
fn bom_と_crlf_を保持して保存する() {
    let mut s = Env::new();
    let src = b"\xEF\xBB\xBFab\r\ncd\r\n";
    s.fs.add_file("/w/a.txt", src);
    s.open("/w/a.txt");
    let (_, d) = s.doc();
    assert_eq!(d.to_string_lossy(), "ab\r\ncd\r\n");
    assert!(d.format.has_bom);
    assert_eq!(d.format.default_eol, eve_text::LineEnding::Crlf);
    assert_eq!(d.format.eol_stats.crlf, 2);
    // 末尾に入力して保存: BOM と CRLF がそのまま残る。Enter は CRLF を入れる
    s.e.execute(CommandInvocation::new(
        "cursor.doc_end",
        InvocationSource::Keymap,
    ));
    s.type_text("x");
    s.key(KeyCode::Enter, Modifiers::NONE);
    s.run("file.save", Args::new());
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        b"\xEF\xBB\xBFab\r\ncd\r\nx\r\n"
    );
}

#[test]
fn 無編集の上書き保存は書き込まず_変更なしと示す() {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"abc");
    s.open("/w/a.txt");
    s.messages();
    let before = s.fs.stamp("/w/a.txt");
    s.clock.advance(eve_core::Millis(5000));
    s.run("file.save", Args::new());
    assert_eq!(
        s.fs.stamp("/w/a.txt"),
        before,
        "mtime も内容も変わらない (FR-104 AC-3)"
    );
    assert!(s.messages().contains(&"info.save.unchanged".to_owned()));
    assert!(s.fs.steps().is_empty(), "書込のステップに入らない");
}

#[test]
fn ディスクが変わっていれば未編集でも保存へ進む() {
    // 未編集でディスク上のファイルが外部で変わった: 上書きは Conflict になり、何も書かない
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"abc");
    s.open("/w/a.txt");
    s.messages();
    s.clock.advance(eve_core::Millis(10));
    s.fs.set_file(&FsPath::from("/w/a.txt"), b"EXTERNAL");
    s.run("file.save", Args::new());
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        b"EXTERNAL",
        "外部の変更を上書きしない"
    );
    assert!(!s.messages().contains(&"info.save.unchanged".to_owned()));
}

#[test]
fn 編集して保存すると履歴が残り_undo_で_dirty_に戻る() {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"abc");
    s.open("/w/a.txt");
    s.e.execute(CommandInvocation::new(
        "cursor.doc_end",
        InvocationSource::Keymap,
    ));
    s.type_text("d");
    assert!(s.doc().1.is_dirty());
    s.messages();
    s.run("file.save", Args::new());
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"abcd");
    assert!(!s.doc().1.is_dirty());
    let evs = s.e.drain_events();
    assert!(
        evs.iter()
            .any(|e| matches!(e, EditorEvent::DocumentSaved { path, .. } if path == "/w/a.txt"))
    );
    // 保存で履歴を消さない (FR-204): Undo で保存前の内容に戻り、dirty になる
    s.run("edit.undo", Args::new());
    assert_eq!(s.text(), "abc");
    assert!(s.doc().1.is_dirty());
    s.run("edit.redo", Args::new());
    assert!(!s.doc().1.is_dirty(), "保存時点に戻れば未変更");
}

#[test]
fn 保存中の入力は新しいノードになり_dirty_のまま残る() {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"abc");
    s.open("/w/a.txt");
    s.e.execute(CommandInvocation::new(
        "cursor.doc_end",
        InvocationSource::Keymap,
    ));
    s.type_text("d");
    // ジョブを実行せずに保存を要求し、その間に入力する
    s.e.execute(CommandInvocation::new(
        "file.save",
        InvocationSource::Keymap,
    ));
    s.type_text("e");
    s.pump();
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        b"abcd",
        "保存したのは要求時の Snapshot"
    );
    assert_eq!(s.text(), "abcde");
    assert!(s.doc().1.is_dirty(), "保存中の編集は dirty のまま");
}

#[test]
fn 保存中の二重の保存要求は無視する() {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"abc");
    s.open("/w/a.txt");
    s.type_text("d");
    s.messages();
    s.e.execute(CommandInvocation::new(
        "file.save",
        InvocationSource::Keymap,
    ));
    let second = s.e.execute(CommandInvocation::new(
        "file.save",
        InvocationSource::Keymap,
    ));
    assert_eq!(second, CommandResult::Done);
    assert!(s.messages().contains(&"info.save.in_progress".to_owned()));
    s.pump();
    assert_eq!(s.jobs.pending(), 0);
}

#[test]
fn 外部変更があると保存は_conflict_で止まる() {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"abc");
    s.open("/w/a.txt");
    s.type_text("X");
    s.messages();
    s.clock.advance(eve_core::Millis(10));
    s.fs.set_file(&FsPath::from("/w/a.txt"), b"theirs");
    s.run("file.save", Args::new());
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"theirs");
    assert!(s.doc().1.is_dirty(), "保存できていないので dirty のまま");
    assert!(s.messages().contains(&"msg.save.failed".to_owned()));
}

#[test]
fn 存在しないパスは空の新規文書で_保存すると作成する() {
    let mut s = Env::new();
    s.open("/w/new.txt");
    assert_eq!(s.text(), "");
    assert!(!s.doc().1.is_dirty());
    assert!(s.messages().contains(&"info.open.new_file".to_owned()));
    s.type_text("hi");
    s.run("file.save", Args::new());
    assert_eq!(s.fs.read("/w/new.txt").unwrap(), b"hi");
    assert!(!s.doc().1.is_dirty());
}

#[test]
fn 親フォルダがないパスは警告し_保存はエラー() {
    let mut s = Env::new();
    s.open("/nodir/x.txt");
    assert!(s.messages().contains(&"msg.open.parent_missing".to_owned()));
    s.type_text("a");
    s.run("file.save", Args::new());
    assert!(!s.fs.exists("/nodir/x.txt"), "フォルダは作らない");
    assert!(s.messages().contains(&"msg.save.parent_missing".to_owned()));
}

#[test]
fn 不正バイトは読み取り専用で開き_別名保存でバイト一致する() {
    let mut s = Env::new();
    let src: &[u8] = b"ok \x9A\xFF end\n";
    s.fs.add_file("/w/bad.txt", src);
    s.open("/w/bad.txt");
    let (_, d) = s.doc();
    assert_eq!(d.format.invalid_bytes, 2);
    assert!(d.read_only.contains(eve_core::ReadOnlyReason::InvalidBytes));
    s.type_text("x");
    assert!(s.text().starts_with("ok "), "編集は拒否される");
    assert_eq!(
        s.text()
            .chars()
            .filter(|c| eve_encoding::unescape_char(*c).is_some())
            .count(),
        2
    );
    assert_eq!(
        s.e.execute(CommandInvocation::new(
            "file.save",
            InvocationSource::Keymap
        )),
        CommandResult::Rejected(RejectReason::NotAvailable)
    );
    // 名前を付けて保存は読み取り専用でも実行できる: 元のバイト列がそのまま書かれる (NFR-303)
    s.run("file.save_as", Args::new().with_str("path", "/w/copy.txt"));
    assert_eq!(s.fs.read("/w/copy.txt").unwrap(), src);
}

#[test]
fn バイナリは確認が要るので開かず_今の文書を残す() {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"abc");
    s.fs.add_file("/w/bin.dat", b"\x00\x01\x02\x03");
    s.open("/w/a.txt");
    let (id, _) = s.doc();
    s.messages();
    s.open("/w/bin.dat");
    assert_eq!(s.doc().0, id, "今の文書がそのまま残る (FR-107 AC-3)");
    assert!(s.messages().contains(&"ui.confirm.open.binary".to_owned()));
}

#[test]
fn フォルダは開けない() {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"abc");
    s.open("/w/a.txt");
    s.messages();
    s.open("/w");
    assert_eq!(s.text(), "abc");
    assert!(s.messages().contains(&"msg.open.not_a_file".to_owned()));
}

#[test]
fn dirty_な文書の置き換えは入力が要るので拒否し_clean_なら置き換わる() {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"aaa");
    s.fs.add_file("/w/b.txt", b"bbb");
    s.open("/w/a.txt");
    s.type_text("x");
    assert_eq!(
        s.open("/w/b.txt"),
        CommandResult::Rejected(RejectReason::NeedsInput)
    );
    assert_eq!(
        s.run("file.new", Args::new()),
        CommandResult::Rejected(RejectReason::NeedsInput)
    );
    assert!(s.text().contains("aaa"));
    // 保存して clean にすれば置き換えられる
    s.run("file.save", Args::new());
    s.e.drain_events();
    s.open("/w/b.txt");
    assert_eq!(s.text(), "bbb");
    assert!(
        s.e.drain_events()
            .iter()
            .any(|e| matches!(e, EditorEvent::DocumentClosed { .. }))
    );
    assert_eq!(s.e.state().documents_len(), 1, "文書は 1 つだけ");
    // 同じファイルを開き直そうとすると既に開いている
    s.messages();
    s.open("/w/b.txt");
    assert!(s.messages().contains(&"info.open.already_open".to_owned()));
}

#[test]
fn 新規文書と名前の入力が要る保存() {
    let mut s = Env::new();
    s.e.open_initial(None, false);
    assert_eq!(s.text(), "");
    assert!(!s.doc().1.is_dirty());
    s.type_text("memo");
    assert_eq!(
        s.e.execute(CommandInvocation::new(
            "file.save",
            InvocationSource::Keymap
        )),
        CommandResult::Rejected(RejectReason::NeedsInput),
        "パスがなければ名前の入力が要る (プロンプトはオーバーレイと同時)"
    );
    s.run("file.save_as", Args::new().with_str("path", "memo.txt"));
    assert_eq!(
        s.fs.read("/w/memo.txt").unwrap(),
        b"memo",
        "相対パスは作業ディレクトリ基準"
    );
    assert!(!s.doc().1.is_dirty());
    let _ = TextFormat::new_document();
}

#[test]
fn 読み取り専用の指定で開く() {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"abc");
    s.e.open_initial(Some("/w/a.txt"), true);
    s.pump();
    assert!(
        s.doc()
            .1
            .read_only
            .contains(eve_core::ReadOnlyReason::UserRequested)
    );
    s.type_text("x");
    assert_eq!(s.text(), "abc");
}

#[test]
fn 読込中は編集できず_完了すると編集できる() {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"abc");
    s.e.open_initial(Some("/w/a.txt"), false);
    assert_eq!(s.doc().1.load_state, eve_core::LoadState::Loading);
    s.type_text("x"); // 読込中: 拒否される
    s.pump();
    assert_eq!(s.text(), "abc");
    s.type_text("x");
    assert_eq!(s.text(), "xabc");
}
