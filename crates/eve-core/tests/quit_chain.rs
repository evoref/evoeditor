//! 終了の連鎖 (docs/03 §9.8、01 FR-107): dirty な文書が複数あるとき、全部について確認してから終了する。

use std::rc::Rc;
use std::sync::Arc;

use eve_core::{
    Args, CommandInvocation, Document, DocumentId, Editor, InvocationSource, Origin, Ports,
    TextFormat, Tick, Transaction,
};
use eve_testkit::{FixedPaths, ManualClock, ManualJobSpawner, MemFs};
use eve_text::{ByteRange, ChangeSet, Edit, Text};
use eve_view::{
    Capabilities, Effect, InputEvent, KeyCode, KeyEvent, Modifiers, OverlayModel, WindowId,
};

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

    fn run(&mut self, id: &str, args: Args) {
        self.e
            .execute(CommandInvocation::new(id, InvocationSource::Keymap).with_args(args));
        self.pump();
    }

    fn key(&mut self, code: KeyCode) {
        self.e.handle_input(
            self.win,
            InputEvent::Key(KeyEvent::press(code, Modifiers::NONE)),
        );
        self.pump();
    }

    fn text(&mut self, s: &str) {
        self.e.handle_input(self.win, InputEvent::Text(s.into()));
        self.pump();
    }

    fn quit_requested(&mut self) -> bool {
        self.e
            .drain_effects()
            .iter()
            .any(|e| matches!(e, Effect::Quit { .. }))
    }

    fn active(&self) -> DocumentId {
        self.e.state().active_document().unwrap()
    }

    fn overlay(&self) -> &'static str {
        self.e.state().overlay_kind()
    }

    /// 確認ダイアログの本文。
    fn message(&mut self) -> String {
        let f = self.e.frame(self.win).unwrap();
        let OverlayModel::Confirm { message, .. } = &f.overlays[0] else {
            panic!("確認が出ていない")
        };
        message.spans.iter().map(|s| s.text.as_str()).collect()
    }

    /// 文書の末尾に `s` を足す (dirty にする)。
    fn append(&mut self, doc: DocumentId, s: &str) {
        let d = self.e.state_mut().document_mut(doc).unwrap();
        let len = d.len_bytes();
        let cs = ChangeSet::from_sorted_edits(
            &d.snapshot(),
            [Edit {
                range: ByteRange::from(len..len),
                insert: s.into(),
            }],
        )
        .unwrap();
        d.apply(&Transaction::new(cs, Origin::User, Tick(0)))
            .unwrap();
    }

    fn text_of(&self, doc: DocumentId) -> String {
        self.e.state().document(doc).unwrap().to_string_lossy()
    }

    fn dirty(&self, doc: DocumentId) -> bool {
        self.e.state().document(doc).unwrap().is_dirty()
    }
}

/// a.txt (dirty、アクティブ) と、表示していない無題の文書 (dirty) の 2 件。
fn two_dirty() -> (Env, DocumentId, DocumentId) {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"aaa");
    s.run("file.open", Args::new().with_str("path", "/w/a.txt"));
    let a = s.active();
    s.text("x");
    let (b, _) = s.e.state_mut().add_document(Document::from_text(
        Text::from("bbb"),
        TextFormat::new_document(),
    ));
    s.append(b, "y");
    (s, a, b)
}

// verifies: AC-FR-107-1
#[test]
fn 全_dirty_文書を何件目かつきで確認し_全部破棄したら終了する() {
    let (mut s, a, b) = two_dirty();
    s.run("app.quit", Args::new());
    assert_eq!(s.overlay(), "dialog");
    assert_eq!(s.active(), a, "アクティブな文書が先頭");
    assert!(s.message().starts_with("(1/2) "), "{}", s.message());

    s.text("d");
    assert!(!s.quit_requested(), "まだ 1 件残っている");
    assert_eq!(s.active(), b, "次の文書をアクティブにして尋ねる");
    assert!(s.message().starts_with("(2/2) "), "{}", s.message());

    s.text("d");
    assert!(s.quit_requested());
    assert!(
        s.dirty(a) && s.dirty(b),
        "破棄しても終了が確定するまで内容は残す"
    );
}

// verifies: AC-FR-107-1
#[test]
fn 後の文書でキャンセルすると終了せず_すべての文書が元のまま残る() {
    let (mut s, a, b) = two_dirty();
    s.run("app.quit", Args::new());
    s.text("d");
    s.key(KeyCode::Esc);
    assert!(!s.quit_requested());
    assert_eq!(s.overlay(), "");
    assert_eq!(s.text_of(a), "xaaa");
    assert_eq!(s.text_of(b), "bbby");
    assert!(s.dirty(a) && s.dirty(b));

    // 破棄の印は無効: もう一度終了すると、全部をもう一度尋ねる
    s.run("app.quit", Args::new());
    assert!(s.message().starts_with("(1/2) "), "{}", s.message());
}

#[test]
fn 保存した文書は次へ進み_保存の続きは保存した文書に属する() {
    let (mut s, a, b) = two_dirty();
    s.run("app.quit", Args::new());
    s.text("s");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"xaaa");
    assert!(!s.dirty(a));
    assert!(!s.quit_requested(), "b が残っている");
    assert_eq!(s.active(), b);
    assert_eq!(s.overlay(), "dialog");
    assert!(s.message().starts_with("(2/2) "), "{}", s.message());

    s.text("d");
    assert!(s.quit_requested());
}

#[test]
fn 保存に失敗したら連鎖を止めて終了しない() {
    let (mut s, a, _b) = two_dirty();
    s.run("app.quit", Args::new());
    s.fs.fail_next(
        eve_testkit::FsOp::WriteAtomic,
        eve_core::FsErrorKind::DiskFull,
    );
    s.text("s");
    assert!(!s.quit_requested());
    assert!(s.dirty(a), "保存できていない");
}

#[test]
fn clean_な文書は尋ねず_dirty_が_1_件なら件数を前置しない() {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"aaa");
    s.run("file.open", Args::new().with_str("path", "/w/a.txt"));
    // 表示していない clean な文書
    s.e.state_mut().add_document(Document::from_text(
        Text::from("ccc"),
        TextFormat::new_document(),
    ));
    s.text("x");
    s.run("app.quit", Args::new());
    assert!(!s.message().starts_with('('), "{}", s.message());
    s.text("d");
    assert!(s.quit_requested());
}
