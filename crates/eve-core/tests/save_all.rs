//! 全保存 `file.save_all` (docs/03 §9.8、01 FR-104 AC-2): dirty な全文書を保存し、失敗した文書を一覧で示す。

use std::rc::Rc;
use std::sync::Arc;

use eve_core::{
    Args, CommandInvocation, CommandResult, Document, DocumentId, Editor, FsPath, InvocationSource,
    Origin, Ports, ReadOnlyReason, TextFormat, Tick, Transaction,
};
use eve_testkit::{FixedPaths, ManualClock, ManualJobSpawner, MemFs};
use eve_text::{ByteRange, ChangeSet, Edit, Text};
use eve_view::{Capabilities, InputEvent, KeyCode, KeyEvent, Modifiers, OverlayModel, WindowId};

struct Env {
    e: Editor,
    fs: Arc<MemFs>,
    jobs: Rc<ManualJobSpawner>,
    win: WindowId,
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
            clock: Arc::new(clock),
            jobs: Rc::clone(&jobs) as Rc<dyn eve_core::JobSpawner>,
            paths,
        };
        let mut e = Editor::new(ports, Capabilities::default());
        e.open_initial(None, false);
        let win = e.state().first_window().unwrap();
        Env { e, fs, jobs, win }
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

    fn run(&mut self, id: &str) -> CommandResult {
        let r = self
            .e
            .execute(CommandInvocation::new(id, InvocationSource::Keymap).with_args(Args::new()));
        self.pump();
        r
    }

    fn key(&mut self, code: KeyCode, mods: Modifiers) {
        self.e
            .handle_input(self.win, InputEvent::Key(KeyEvent::press(code, mods)));
        self.pump();
    }

    fn text(&mut self, s: &str) {
        self.e.handle_input(self.win, InputEvent::Text(s.into()));
        self.pump();
    }

    fn overlay(&self) -> &'static str {
        self.e.state().overlay_kind()
    }

    /// 文書を足す。`path` があれば文書のパスにする (ディスクには無い = 新規のファイル)。dirty にして返す。
    fn add_dirty(&mut self, path: Option<&str>, body: &str) -> DocumentId {
        let (id, _) = self.e.state_mut().add_document(Document::from_text(
            Text::from(body),
            TextFormat::new_document(),
        ));
        let d = self.e.state_mut().document_mut(id).unwrap();
        d.path = path.map(FsPath::new);
        let len = d.len_bytes();
        let cs = ChangeSet::from_sorted_edits(
            &d.snapshot(),
            [Edit {
                range: ByteRange::from(len..len),
                insert: "!".into(),
            }],
        )
        .unwrap();
        d.apply(&Transaction::new(cs, Origin::User, Tick(0)))
            .unwrap();
        id
    }

    fn dirty(&self, doc: DocumentId) -> bool {
        self.e.state().document(doc).unwrap().is_dirty()
    }

    /// 確認ダイアログの (本文, details)。
    fn dialog(&mut self) -> (String, Vec<String>) {
        let f = self.e.frame(self.win).unwrap();
        let OverlayModel::Confirm {
            message, details, ..
        } = &f.overlays[0]
        else {
            panic!("確認が出ていない")
        };
        let join = |t: &eve_view::CellText| t.spans.iter().map(|s| s.text.as_str()).collect();
        (
            join(message),
            details.iter().flatten().map(join).collect::<Vec<String>>(),
        )
    }
}

// verifies: AC-FR-104-1
#[test]
fn dirty_な全文書を保存し_名前の無い文書と読み取り専用の文書は飛ばす() {
    let mut s = Env::new();
    let active = s.e.state().active_document().unwrap();
    let a = s.add_dirty(Some("/w/a.txt"), "aaa");
    let b = s.add_dirty(Some("/w/b.txt"), "bbb");
    let untitled = s.add_dirty(None, "uuu");
    let ro = s.add_dirty(Some("/w/ro.txt"), "rrr");
    s.e.state_mut()
        .document_mut(ro)
        .unwrap()
        .read_only
        .insert(ReadOnlyReason::UserRequested);
    // 表示中の文書は変えない
    s.run("file.save_all");

    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"aaa!");
    assert_eq!(s.fs.read("/w/b.txt").unwrap(), b"bbb!");
    assert!(!s.fs.exists("/w/ro.txt"), "読み取り専用は書かない");
    assert!(!s.dirty(a) && !s.dirty(b));
    assert!(
        s.dirty(untitled) && s.dirty(ro),
        "飛ばした文書は dirty のまま"
    );
    assert_eq!(s.overlay(), "", "失敗が無いので確認は出ない");
    assert_eq!(s.e.state().active_document().unwrap(), active);
}

// verifies: AC-FR-104-1
#[test]
fn 既定キーで実行でき_dirty_が無ければ何も書かない() {
    let mut s = Env::new();
    s.key(KeyCode::Char('k'), Modifiers::CTRL);
    s.text("a");
    assert_eq!(s.overlay(), "");
    let a = s.add_dirty(Some("/w/a.txt"), "aaa");
    s.key(KeyCode::Char('k'), Modifiers::CTRL);
    s.text("a");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"aaa!");
    assert!(!s.dirty(a));
}

// verifies: AC-FR-104-1
#[test]
fn 失敗した文書は確認を出さずに記録し_最後に一覧で示す() {
    let mut s = Env::new();
    let a = s.add_dirty(Some("/w/a.txt"), "aaa");
    let b = s.add_dirty(Some("/w/b.txt"), "bbb");
    let c = s.add_dirty(Some("/w/c.txt"), "ccc");
    // b は保存の間に別のプロセスが同名のファイルを作った (外部変更の確認が要る保存)
    s.fs.add_file("/w/b.txt", b"external");
    s.run("file.save_all");

    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"aaa!");
    assert_eq!(s.fs.read("/w/c.txt").unwrap(), b"ccc!");
    assert_eq!(s.fs.read("/w/b.txt").unwrap(), b"external", "上書きしない");
    assert!(!s.dirty(a) && !s.dirty(c));
    assert!(s.dirty(b), "失敗した文書は dirty のまま");

    // 一覧の確認 1 つだけ (外部変更の確認は出ない)
    assert_eq!(s.overlay(), "dialog");
    let (message, details) = s.dialog();
    assert!(message.starts_with("1 件"), "{message}");
    assert_eq!(details.len(), 1, "{details:?}");
    assert!(details[0].starts_with("/w/b.txt: "), "{details:?}");

    s.key(KeyCode::Esc, Modifiers::NONE);
    assert_eq!(s.overlay(), "");
    assert!(s.dirty(b));
}

#[test]
fn 保存の失敗は他の文書の保存を止めない() {
    let mut s = Env::new();
    let a = s.add_dirty(Some("/w/a.txt"), "aaa");
    let b = s.add_dirty(Some("/w/b.txt"), "bbb");
    s.fs.fail_next(
        eve_testkit::FsOp::WriteAtomic,
        eve_core::FsErrorKind::DiskFull,
    );
    s.run("file.save_all");
    // 1 件目が失敗し、2 件目は保存される
    assert!(s.dirty(a) && !s.dirty(b));
    assert_eq!(s.fs.read("/w/b.txt").unwrap(), b"bbb!");
    assert_eq!(s.overlay(), "dialog");
}
