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

// verifies: AC-FR-104-3
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

// verifies: AC-FR-204-2
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
    // 失敗の text は保存の失敗の確認の本文に入る (別名で保存を案内する)
    assert!(dialog_open(&s));
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
    assert!(s.messages().contains(&"msg.open.failed".to_owned()));
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
fn clean_な文書は置き換わり_同じファイルは既に開いている() {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"aaa");
    s.fs.add_file("/w/b.txt", b"bbb");
    s.open("/w/a.txt");
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
fn 新規文書の保存は名前の入力プロンプトを出す() {
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
        CommandResult::Done,
        "パスがなければ名前を尋ねる"
    );
    assert_eq!(s.e.state().overlay_kind(), "prompt");
    s.run("file.save_as", Args::new().with_str("path", "memo.txt"));
    let _ = (RejectReason::NeedsInput, TextFormat::new_document());
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

#[test]
fn 読込を待つ間は今の文書を編集できず_重ねた置き換えも拒否する() {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"aaa");
    s.fs.add_file("/w/b.txt", b"bbb");
    s.fs.add_file("/w/c.txt", b"ccc");
    s.open("/w/a.txt");
    // b を開き始める (ジョブはまだ実行しない)
    let r = s.e.execute(
        CommandInvocation::new("file.open", InvocationSource::Keymap)
            .with_args(Args::new().with_str("path", "/w/b.txt")),
    );
    assert!(matches!(r, CommandResult::Pending(_)));
    s.type_text("x"); // 読込の完了で捨てられる編集は受け付けない
    assert_eq!(s.text(), "aaa");
    for (id, args) in [
        ("file.open", Args::new().with_str("path", "/w/c.txt")),
        ("file.new", Args::new()),
    ] {
        let r =
            s.e.execute(CommandInvocation::new(id, InvocationSource::Keymap).with_args(args));
        assert_eq!(
            r,
            CommandResult::Rejected(RejectReason::NotAvailable),
            "{id}"
        );
    }
    s.pump();
    assert_eq!(s.text(), "bbb");
    assert_eq!(s.e.state().documents_len(), 1, "孤立した文書はない");
    s.type_text("y");
    assert_eq!(s.text(), "ybbb", "完了後は編集できる");
}

#[test]
fn 読み取り専用の文書を同じファイルへ名前を付けて保存しない() {
    let mut s = Env::new();
    let src: &[u8] = b"ok \x9A\n";
    s.fs.add_file("/w/bad.txt", src);
    s.open("/w/bad.txt");
    s.messages();
    let r = s.run("file.save_as", Args::new().with_str("path", "/w/bad.txt"));
    assert_eq!(r, CommandResult::Rejected(RejectReason::ReadOnly));
    assert_eq!(
        s.fs.read("/w/bad.txt").unwrap(),
        src,
        "元のバイト列を損なわない"
    );
    assert!(
        s.messages()
            .contains(&"msg.save_as.same_file_read_only".to_owned())
    );
}

#[test]
fn 名前を付けて保存の失敗はパスと_stamp_を変えない() {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"abc");
    s.fs.add_file("/w/other.txt", b"EXIST");
    s.open("/w/a.txt");
    let stamp = s.doc().1.disk_stamp;
    s.type_text("x");
    s.run("file.save_as", Args::new().with_str("path", "/w/other.txt")); // 既存 → Conflict
    assert_eq!(s.fs.read("/w/other.txt").unwrap(), b"EXIST");
    let (_, d) = s.doc();
    assert_eq!(
        d.path,
        Some(FsPath::from("/w/a.txt")),
        "失敗したのでパスは元のまま"
    );
    assert_eq!(d.disk_stamp, stamp);
    assert!(d.is_dirty());
    // 元のファイルへはそのまま保存できる
    s.run("file.save", Args::new());
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"xabc");
}

#[test]
fn 保存の待ちの間の名前を付けて保存は無視する() {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"abc");
    s.open("/w/a.txt");
    s.type_text("d");
    s.e.execute(CommandInvocation::new(
        "file.save",
        InvocationSource::Keymap,
    ));
    s.messages();
    let r = s.e.execute(
        CommandInvocation::new("file.save_as", InvocationSource::Keymap)
            .with_args(Args::new().with_str("path", "/w/a.txt")),
    );
    assert_eq!(r, CommandResult::Done);
    assert!(s.messages().contains(&"info.save.in_progress".to_owned()));
    s.pump();
    assert_eq!(s.jobs.pending(), 0);
}

#[test]
fn 読込中の文書は名前を付けて保存できない() {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"abc");
    s.e.open_initial(Some("/w/a.txt"), false);
    let r = s.e.execute(
        CommandInvocation::new("file.save_as", InvocationSource::Keymap)
            .with_args(Args::new().with_str("path", "/w/zz.txt")),
    );
    assert_eq!(r, CommandResult::Rejected(RejectReason::NotAvailable));
    s.pump();
    assert!(!s.fs.exists("/w/zz.txt"));
}

#[test]
fn 最初の読込の失敗は無題で始まり_再試行できる() {
    let mut s = Env::new();
    s.fs.add_file("/w/bin.dat", b"\x00\x01\x02");
    s.e.open_initial(Some("/w/bin.dat"), false);
    s.pump();
    let (_, d) = s.doc();
    assert!(
        d.path.is_none() && d.is_editable(),
        "失敗した文書は閉じ、無題の文書で始まる"
    );
    assert_eq!(s.e.state().documents_len(), 1);
    // 同じパスを開き直せる (already_open にならない)
    s.messages();
    s.fs.add_file("/w/bin.dat", b"text now");
    s.open("/w/bin.dat");
    assert_eq!(s.text(), "text now");
}

#[test]
fn readonly_の指定でもバイナリの確認は出す() {
    let mut s = Env::new();
    s.fs.add_file("/w/bin.dat", b"\x00\x01\x02");
    s.e.open_initial(Some("/w/bin.dat"), true);
    s.pump();
    assert!(s.messages().contains(&"msg.open.failed".to_owned()));
}

// ---- 中断された保存の痕跡 (02 §4.5「中断の痕跡の写像」、05 §4.6) ----

fn flag(
    target: &str,
    method: eve_core::SaveMethod,
    recorded: bool,
    complete: bool,
) -> eve_core::InflightRecord {
    eve_core::InflightRecord {
        target: FsPath::from(target),
        pid: 100,
        started_at: None,
        method,
        tmp: Some(FsPath::from("/w/.a.txt.eve-tmp-100-aaaaaaaa")),
        backup: Some(FsPath::from("/w/.a.txt.eve-bak-100-bbbbbbbb")),
        backup_recorded: recorded,
        backup_complete: complete,
        flag: FsPath::from("/state/inflight/k/100-1.toml"),
        malformed: false,
    }
}

#[test]
fn 置換の途中で中断した保存は退避名を案内して_文書は空の新規として開く() {
    let mut s = Env::new();
    s.fs.add_file("/w/.a.txt.eve-bak-100-bbbbbbbb", b"original");
    s.fs.add_inflight(flag("/w/a.txt", eve_core::SaveMethod::Atomic, false, false));
    s.messages();
    s.open("/w/a.txt");
    assert!(
        s.messages()
            .contains(&"msg.open.interrupted_save".to_owned())
    );
    assert_eq!(
        s.text(),
        "",
        "T が無いので空の新規文書 (黙って開かず、場所を示す)"
    );
    assert_eq!(s.fs.inflight_records().len(), 1, "報告した記録は消さない");
}

#[test]
fn 新規作成の途中で中断した保存を案内する() {
    let mut s = Env::new();
    s.fs.add_file("/w/.a.txt.eve-tmp-100-aaaaaaaa", b"partial");
    s.fs.add_inflight(flag(
        "/w/a.txt",
        eve_core::SaveMethod::NewFile,
        false,
        false,
    ));
    s.messages();
    s.open("/w/a.txt");
    assert!(
        s.messages()
            .contains(&"msg.open.interrupted_new".to_owned())
    );
}

#[test]
fn 直接上書きの中断はファイルがあっても案内する() {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"half-written");
    s.fs.add_file("/w/.a.txt.eve-bak-100-bbbbbbbb", b"original");
    s.fs.add_inflight(flag("/w/a.txt", eve_core::SaveMethod::InPlace, true, true));
    s.messages();
    s.open("/w/a.txt");
    assert_eq!(s.text(), "half-written", "文書は通常どおり開く");
    assert!(
        s.messages()
            .contains(&"msg.open.interrupted_save".to_owned())
    );
}

#[test]
fn t_が在る原子的保存の痕跡は報告せずフラグを掃除する() {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"fine");
    s.fs.add_inflight(flag("/w/a.txt", eve_core::SaveMethod::Atomic, false, false));
    s.messages();
    s.open("/w/a.txt");
    let m = s.messages();
    assert!(
        !m.iter().any(|k| k.starts_with("msg.open.interrupted")),
        "{m:?}"
    );
    assert!(
        s.fs.inflight_records().is_empty(),
        "痕跡の無いフラグは消える"
    );
}

#[test]
fn 痕跡が実在する記録は報告しなくてもフラグを残す() {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"fine");
    s.fs.add_file("/w/.a.txt.eve-tmp-100-aaaaaaaa", b"leftover");
    s.fs.add_inflight(flag("/w/a.txt", eve_core::SaveMethod::Atomic, false, false));
    s.open("/w/a.txt");
    assert_eq!(s.fs.inflight_records().len(), 1);
}

// ---- 起動時の退避の案内 (02 §9.3 の RecoveryScan、05 §7.5) ----

#[test]
fn 起動時に退避が残っていれば最も新しいものの場所を案内する() {
    let mut s = Env::new();
    s.fs.add_recovery_dir("/s/recovery/20261001T000000Z-100", false);
    s.fs.add_recovery_dir("/s/recovery/20261021T120000Z-200", false);
    s.fs.add_recovery_dir("/s/recovery/20261021T120000Z-200-2", false);
    s.fs.add_recovery_dir("/s/recovery/rescue/20261022T000000Z-300", true);
    s.e.open_initial(None, false);
    s.pump();
    let keys = s.messages();
    assert_eq!(
        keys.last().map(String::as_str),
        Some("msg.recovery.found"),
        "異常終了の退避があるので found (rescue ではない): {keys:?}"
    );
    let frame = s.e.frame(s.win()).unwrap();
    let text: String = frame
        .messages
        .current
        .expect("現在のメッセージ")
        .text
        .spans
        .iter()
        .map(|sp| sp.text.as_str())
        .collect();
    assert!(
        text.contains("20261021T120000Z-200-2"),
        "同じ時刻は -n の大きい方が新しい: {text}"
    );
    assert!(text.contains('3'), "異常終了の退避は 3 件: {text}");
}

#[test]
fn 退避先に保存の退避だけなら文言を分ける() {
    let mut s = Env::new();
    s.fs.add_recovery_dir("/s/recovery/rescue/20261022T000000Z-300", true);
    s.e.open_initial(None, false);
    s.pump();
    assert!(
        s.messages()
            .contains(&"msg.recovery.found_rescue".to_owned())
    );
}

#[test]
fn 退避が無ければ案内しない() {
    let mut s = Env::new();
    s.e.open_initial(None, false);
    s.pump();
    assert!(!s.messages().iter().any(|k| k.starts_with("msg.recovery")));
}

/// 編集済みの文書を開き、原子的保存が `kind` で失敗する状態にして保存する。
fn edited_save_fails(s: &mut Env, kind: eve_core::FsErrorKind) {
    s.fs.add_file("/w/a.txt", b"abc");
    s.open("/w/a.txt");
    s.e.execute(CommandInvocation::new(
        "cursor.doc_end",
        InvocationSource::Keymap,
    ));
    s.type_text("d");
    s.messages();
    s.fs.fail_next(eve_testkit::FsOp::WriteAtomic, kind);
    s.run("file.save", Args::new());
}

fn dialog_open(s: &Env) -> bool {
    s.e.state().overlay_kind() == "dialog"
}

#[test]
fn 原子的置換ができなければ確認を出し_直接上書きで保存する() {
    let mut s = Env::new();
    edited_save_fails(&mut s, eve_core::FsErrorKind::AtomicUnsupported);
    assert!(dialog_open(&s), "確認が出る");
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        b"abc",
        "確認の間は何も書かない"
    );
    s.type_text("o");
    s.pump();
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"abcd");
    assert!(!s.doc().1.is_dirty());
    assert!(s.messages().contains(&"info.save.in_place".to_owned()));
}

#[test]
fn 原子的置換の確認のキャンセルでは何も書かず_dirty_のまま() {
    let mut s = Env::new();
    edited_save_fails(&mut s, eve_core::FsErrorKind::AtomicUnsupported);
    s.key(KeyCode::Esc, Modifiers::NONE);
    s.pump();
    assert!(!dialog_open(&s));
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"abc");
    assert!(s.doc().1.is_dirty());
}

#[test]
fn 原子的置換の確認の別名で保存は名前の入力へ進む() {
    let mut s = Env::new();
    edited_save_fails(&mut s, eve_core::FsErrorKind::AtomicUnsupported);
    s.type_text("a");
    s.pump();
    assert_eq!(s.e.state().overlay_kind(), "prompt");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"abc");
}

#[test]
fn バックアップを作れなければ確認し_承認するとバックアップなしで上書きする() {
    let mut s = Env::new();
    edited_save_fails(&mut s, eve_core::FsErrorKind::AtomicUnsupported);
    s.fs.fail_next(
        eve_testkit::FsOp::WriteInPlace,
        eve_core::FsErrorKind::BackupFailed,
    );
    s.type_text("o");
    s.pump();
    assert!(dialog_open(&s), "バックアップなしの確認が続く");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"abc");
    s.type_text("o");
    s.pump();
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"abcd");
    assert!(!s.doc().1.is_dirty());
}

#[test]
fn ハードリンクはリンクを保つか切り離すかを選ぶ() {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"abc");
    s.fs.set_nlink("/w/a.txt", 2);
    s.open("/w/a.txt");
    s.e.execute(CommandInvocation::new(
        "cursor.doc_end",
        InvocationSource::Keymap,
    ));
    s.type_text("d");
    s.run("file.save", Args::new());
    assert!(dialog_open(&s));
    // 切り離して保存: 原子的置換で書き、nlink は 1 になる
    s.type_text("b");
    s.pump();
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"abcd");
    assert!(!s.doc().1.is_dirty());
}

#[test]
fn 保存の失敗は確認を出し_退避先に保存すると本文を書いて_dirty_のまま() {
    let mut s = Env::new();
    edited_save_fails(&mut s, eve_core::FsErrorKind::PermissionDenied);
    assert!(dialog_open(&s), "失敗の後の選択が出る");
    s.type_text("r");
    s.pump();
    assert_eq!(s.fs.recovery_written(), vec![b"abcd".to_vec()]);
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        b"abc",
        "元のファイルは変えない"
    );
    assert!(s.doc().1.is_dirty());
    assert!(s.messages().contains(&"info.save.to_recovery".to_owned()));
}

#[test]
fn 退避先に保存が失敗するとそれを示す() {
    let mut s = Env::new();
    edited_save_fails(&mut s, eve_core::FsErrorKind::DiskFull);
    s.fs.fail_next(
        eve_testkit::FsOp::WriteRecovery,
        eve_core::FsErrorKind::DiskFull,
    );
    s.type_text("r");
    s.pump();
    assert!(s.fs.recovery_written().is_empty());
    assert!(
        s.messages()
            .contains(&"msg.save.recovery_failed".to_owned())
    );
    assert!(s.doc().1.is_dirty());
}

#[test]
fn 保存の失敗の確認の別名で保存とキャンセル() {
    let mut s = Env::new();
    edited_save_fails(&mut s, eve_core::FsErrorKind::PermissionDenied);
    s.type_text("a");
    s.pump();
    assert_eq!(s.e.state().overlay_kind(), "prompt");
    s.key(KeyCode::Esc, Modifiers::NONE);
    s.messages();
    s.fs.fail_next(
        eve_testkit::FsOp::WriteAtomic,
        eve_core::FsErrorKind::PermissionDenied,
    );
    s.run("file.save", Args::new());
    assert!(dialog_open(&s));
    s.key(KeyCode::Esc, Modifiers::NONE);
    assert!(!dialog_open(&s));
    assert!(s.messages().contains(&"info.save.aborted".to_owned()));
    assert!(s.fs.recovery_written().is_empty());
}

#[test]
fn busy_のときだけ再試行でき_照合を通って保存される() {
    let mut s = Env::new();
    edited_save_fails(&mut s, eve_core::FsErrorKind::Busy);
    assert!(dialog_open(&s));
    s.type_text("t");
    s.pump();
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"abcd");
    assert!(!s.doc().1.is_dirty());
    // Busy 以外には再試行が無い (t は無視される)
    let mut s = Env::new();
    edited_save_fails(&mut s, eve_core::FsErrorKind::PermissionDenied);
    s.type_text("t");
    s.pump();
    assert!(dialog_open(&s));
}
