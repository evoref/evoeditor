//! バッファ (複数文書) の切替と閉じる (docs/03 §9.2 の閉じる手順、§9.8、01 FR-103 / FR-107)。

use std::rc::Rc;
use std::sync::Arc;

use eve_core::{
    Args, CommandInvocation, CommandResult, Document, DocumentId, Editor, InvocationSource, Ports,
    TextFormat,
};
use eve_testkit::{FixedPaths, ManualClock, ManualJobSpawner, MemFs};
use eve_text::Text;
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

    fn active(&self) -> DocumentId {
        self.e.state().active_document().unwrap()
    }

    fn text_of(&self, doc: DocumentId) -> String {
        self.e.state().document(doc).unwrap().to_string_lossy()
    }

    fn overlay(&self) -> &'static str {
        self.e.state().overlay_kind()
    }

    fn quit_requested(&mut self) -> bool {
        self.e
            .drain_effects()
            .iter()
            .any(|e| matches!(e, Effect::Quit { .. }))
    }

    fn message(&mut self) -> String {
        let f = self.e.frame(self.win).unwrap();
        let OverlayModel::Confirm { message, .. } = &f.overlays[0] else {
            panic!("確認が出ていない")
        };
        message.spans.iter().map(|s| s.text.as_str()).collect()
    }

    fn exists(&self, doc: DocumentId) -> bool {
        self.e.state().document(doc).is_some()
    }
}

/// 開いた順に a, b, c の 3 文書 (a が表示中)。
fn three() -> (Env, [DocumentId; 3]) {
    let mut s = Env::new();
    let a = s.active();
    let add = |s: &mut Env, t: &str| {
        s.e.state_mut()
            .add_document(Document::from_text(
                Text::from(t),
                TextFormat::new_document(),
            ))
            .0
    };
    let b = add(&mut s, "bbb");
    let c = add(&mut s, "ccc");
    (s, [a, b, c])
}

#[test]
fn next_と_prev_は開いた順で循環する() {
    let (mut s, [a, b, c]) = three();
    s.run("buffer.next");
    assert_eq!(s.active(), b);
    s.run("buffer.next");
    assert_eq!(s.active(), c);
    s.run("buffer.next");
    assert_eq!(s.active(), a, "末尾の次は先頭");
    s.run("buffer.prev");
    assert_eq!(s.active(), c, "先頭の前は末尾");
    s.run("buffer.prev");
    assert_eq!(s.active(), b);
}

#[test]
fn 既定キーで切り替わり_文書が_1_件なら何も起きない() {
    let (mut s, [a, b, _c]) = three();
    s.key(KeyCode::PageDown, Modifiers::CTRL);
    assert_eq!(s.active(), b);
    s.key(KeyCode::PageUp, Modifiers::CTRL);
    assert_eq!(s.active(), a);
    s.key(KeyCode::Char('k'), Modifiers::CTRL);
    s.text("n");
    assert_eq!(s.active(), b, "ctrl+k n");

    let mut one = Env::new();
    let only = one.active();
    one.run("buffer.next");
    one.run("buffer.prev");
    assert_eq!(one.active(), only);
}

#[test]
fn 切替の後も編集と_undo_の履歴が保たれる() {
    let (mut s, [a, b, _c]) = three();
    s.text("x");
    assert_eq!(s.text_of(a), "x");
    s.run("buffer.next");
    s.text("y");
    s.run("buffer.prev");
    assert_eq!(s.active(), a);
    assert_eq!(s.text_of(a), "x");
    assert_eq!(s.text_of(b), "ybbb");
    s.run("edit.undo");
    assert_eq!(s.text_of(a), "", "a の履歴は b の編集と混ざらない");
    assert_eq!(s.text_of(b), "ybbb");
}

#[test]
fn 閉じた後は最近使った順で次の文書を表示する() {
    let (mut s, [a, b, c]) = three();
    s.run("buffer.next"); // b
    s.run("buffer.next"); // c
    s.run("buffer.close");
    assert!(!s.exists(c));
    assert_eq!(
        s.active(),
        b,
        "直前に使っていた b (開いた順なら a ではない)"
    );
    s.run("buffer.close");
    assert_eq!(s.active(), a);
    assert!(!s.exists(b));
}

#[test]
fn 最後の_1_件を閉じると無題の文書を作り_終了しない() {
    let mut s = Env::new();
    let only = s.active();
    s.run("buffer.close");
    assert!(!s.quit_requested());
    assert!(!s.exists(only));
    assert_eq!(s.e.state().documents_len(), 1);
    assert_eq!(s.text_of(s.active()), "");
}

// verifies: AC-FR-107-1
#[test]
fn dirty_な文書を閉じると確認が出て_キャンセルでも破棄でも他の文書は残る() {
    let (mut s, [a, b, _c]) = three();
    s.text("x");
    s.run("buffer.close");
    assert_eq!(s.overlay(), "dialog");
    assert!(
        s.message().contains("保存されていない変更"),
        "{}",
        s.message()
    );
    s.key(KeyCode::Esc, Modifiers::NONE);
    assert!(s.exists(a) && s.exists(b), "キャンセルで何も変わらない");
    assert_eq!(s.active(), a);
    assert_eq!(s.text_of(a), "x");

    s.run("buffer.close");
    s.text("d");
    assert!(!s.exists(a), "破棄して閉じる");
    assert!(s.exists(b));
    assert_eq!(s.overlay(), "");
}

/// /w/a.txt を開いて 1 文字入力した (dirty な) 状態に、表示していない文書を 1 件足す。
fn dirty_file_and_other() -> (Env, DocumentId, DocumentId) {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"aaa");
    s.e.execute(
        CommandInvocation::new("file.open", InvocationSource::Keymap)
            .with_args(Args::new().with_str("path", "/w/a.txt")),
    );
    s.pump();
    let a = s.active();
    s.text("x");
    let b =
        s.e.state_mut()
            .add_document(Document::from_text(
                Text::from("bbb"),
                TextFormat::new_document(),
            ))
            .0;
    (s, a, b)
}

#[test]
fn 保存して閉じる() {
    let (mut s, a, b) = dirty_file_and_other();
    s.run("buffer.close");
    s.text("s");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"xaaa");
    assert!(!s.exists(a), "保存できたので閉じる");
    assert_eq!(s.active(), b);
}

#[test]
fn 保存に失敗したら閉じない() {
    let (mut s, a, _b) = dirty_file_and_other();
    s.run("buffer.close");
    s.fs.fail_next(
        eve_testkit::FsOp::WriteAtomic,
        eve_core::FsErrorKind::DiskFull,
    );
    s.text("s");
    assert!(s.exists(a), "保存に失敗したら閉じない");
    assert_eq!(s.active(), a);
}

impl Env {
    /// ステータスバーのファイル名の項目。
    fn status_name(&mut self) -> String {
        let f = self.e.frame(self.win).unwrap();
        let name = &f.status.items[0];
        assert_eq!(name.kind, eve_view::StatusKind::Path);
        name.long.spans.iter().map(|s| s.text.as_str()).collect()
    }
}

#[test]
fn ステータスバーは文書が_2_件以上のとき何番目かを出す() {
    let mut one = Env::new();
    assert_eq!(one.status_name(), "無題-1", "1 件なら出さない");

    let (mut s, _) = three();
    assert_eq!(s.status_name(), "無題-1 (1/3)");
    s.run("buffer.next");
    assert_eq!(s.status_name(), "無題-1 (2/3)");
    s.run("buffer.prev");
    s.run("buffer.prev");
    assert_eq!(s.status_name(), "無題-1 (3/3)");
    s.run("buffer.close");
    assert_eq!(
        s.status_name(),
        "無題-1 (1/2)",
        "閉じると数が減り、最近使った a を表示する"
    );
}
