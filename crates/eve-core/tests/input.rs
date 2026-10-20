//! `Editor::handle_input` のテスト (docs/02 §7.1 / §7.2): キー → キーマップ → コマンド、Text、Paste、チョードの保留。

use std::rc::Rc;
use std::sync::Arc;

use eve_core::{
    CommandCtx, CommandKind, CommandResult, CommandSpec, Document, Editor, Keymap, Layer,
    Permission, Ports, ReadOnlyReason, TextFormat,
};
use eve_testkit::{FixedPaths, ManualClock, ManualJobSpawner, MemFs};
use eve_text::{ByteOff, Selection, Text};
use eve_view::{
    Capabilities, DocumentId, InputEvent, KeyCode, KeyEvent, KeyKind, Modifiers, WindowId,
};

struct Scene {
    e: Editor,
    doc: DocumentId,
    win: WindowId,
}

impl Scene {
    fn new(initial: &str) -> Scene {
        let clock = ManualClock::new();
        let fs = Arc::new(MemFs::new(Arc::new(clock.clone())));
        let paths = Arc::new(FixedPaths::default());
        let jp = eve_core::JobPorts {
            fs: fs.clone(),
            clock: Arc::new(clock.clone()),
            paths: paths.clone(),
        };
        let jobs = ManualJobSpawner::new(jp);
        let ports = Ports {
            fs,
            clock: Arc::new(clock),
            jobs: Rc::clone(&jobs) as Rc<dyn eve_core::JobSpawner>,
            paths,
        };
        let mut e = Editor::new(ports, Capabilities::default());
        let (doc, view, win) = e.open_document(Document::from_text(
            Text::from(initial),
            TextFormat::new_document(),
        ));
        e.state_mut()
            .set_selection(view, Selection::point(ByteOff(initial.len())));
        Scene { e, doc, win }
    }

    fn key(&mut self, code: KeyCode, mods: Modifiers) {
        self.e
            .handle_input(self.win, InputEvent::Key(KeyEvent::press(code, mods)));
    }

    fn text(&self) -> String {
        self.e.state().document(self.doc).unwrap().to_string_lossy()
    }
}

#[test]
fn text_は_insert_text_になる() {
    let mut s = Scene::new("");
    s.e.handle_input(s.win, InputEvent::Text("ab".into()));
    s.e.handle_input(s.win, InputEvent::Text("あ".into()));
    assert_eq!(s.text(), "abあ");
}

#[test]
fn キーはキーマップを通してコマンドになる() {
    let mut s = Scene::new("ab");
    s.key(KeyCode::Enter, Modifiers::NONE);
    assert_eq!(s.text(), "ab\n");
    s.key(KeyCode::Backspace, Modifiers::NONE);
    assert_eq!(s.text(), "ab");
    s.key(KeyCode::Char('z'), Modifiers::CTRL); // mod+z = undo
    s.key(KeyCode::Char('z'), Modifiers::CTRL);
    assert_eq!(
        s.text(),
        "ab",
        "Undo は 1 回で Enter か Backspace のどちらかが戻る (結合の窓の外では別ノード)"
    );
}

#[test]
fn 割当のない印字可能キーは文字として入る() {
    let mut s = Scene::new("");
    s.key(KeyCode::Char('x'), Modifiers::NONE);
    assert_eq!(s.text(), "x");
    // 修飾付きの割当なしキーは無視する
    s.key(KeyCode::Char('x'), Modifiers::CTRL);
    s.key(KeyCode::F(5), Modifiers::NONE);
    assert_eq!(s.text(), "x");
}

#[test]
fn release_は無視する() {
    let mut s = Scene::new("");
    let ev = KeyEvent {
        code: KeyCode::Char('q'),
        mods: Modifiers::NONE,
        kind: KeyKind::Release,
    };
    s.e.handle_input(s.win, InputEvent::Key(ev));
    assert_eq!(s.text(), "");
}

#[test]
fn 貼り付けは改行を正規化し_制御文字を実行しない() {
    let mut s = Scene::new("");
    s.e.handle_input(s.win, InputEvent::Paste("a\r\nb\rc\nd\u{13}e".into()));
    assert_eq!(
        s.text(),
        "a\nb\nc\nd\u{13}e",
        "default_eol (LF) に統一。0x13 は文字として入る"
    );
    // 1 回の Undo で貼り付け全体が戻る
    s.key(KeyCode::Char('z'), Modifiers::CTRL);
    assert_eq!(s.text(), "");
    // CRLF の文書へは CRLF で入る
    let mut s = Scene::new("");
    s.e.state_mut()
        .document_mut(s.doc)
        .unwrap()
        .format
        .default_eol = eve_text::LineEnding::Crlf;
    s.e.handle_input(s.win, InputEvent::Paste("x\ny".into()));
    assert_eq!(s.text(), "x\r\ny");
}

#[test]
fn 読み取り専用では貼り付けも入力も何も変えない() {
    let mut s = Scene::new("keep");
    s.e.state_mut()
        .document_mut(s.doc)
        .unwrap()
        .read_only
        .insert(ReadOnlyReason::UserRequested);
    s.e.handle_input(s.win, InputEvent::Paste("x".into()));
    s.e.handle_input(s.win, InputEvent::Text("y".into()));
    s.key(KeyCode::Enter, Modifiers::NONE);
    assert_eq!(s.text(), "keep");
}

fn mark_handler(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.push_event(eve_view::EditorEvent::MessageShown {
        level: eve_view::MessageLevel::Info,
        key: "test.chord".to_owned(),
    });
    CommandResult::Done
}

#[test]
fn チョードの保留と_text_での_2_打目() {
    let mut s = Scene::new("");
    let mut l = Layer::new("test");
    l.bind("editor", "ctrl+k s", "t.chord", "").unwrap();
    s.e.set_keymap(Keymap::new(vec![l]).unwrap());
    s.e.register(
        CommandSpec::new(
            "t.chord",
            CommandKind::Ui,
            Permission::None,
            "",
            mark_handler,
        )
        .unwrap(),
    );
    s.e.drain_events();

    // 1 打目 (保留) → 2 打目が Text("sXYZ") で届く: 先頭の書記素だけを使い、残りは捨てる (K1)
    s.key(KeyCode::Char('k'), Modifiers::CTRL);
    s.e.handle_input(s.win, InputEvent::Text("sXYZ".into()));
    assert_eq!(s.text(), "", "残りは本文に混ざらない");
    assert!(
        s.e.drain_events().iter().any(|ev| matches!(ev, eve_view::EditorEvent::MessageShown { key, .. } if key == "test.chord")),
        "チョードが実行された"
    );

    // 保留中の Esc は保留だけを取り消し、本文は変わらない
    s.key(KeyCode::Char('k'), Modifiers::CTRL);
    s.key(KeyCode::Esc, Modifiers::NONE);
    s.e.handle_input(s.win, InputEvent::Text("a".into()));
    assert_eq!(s.text(), "a", "取り消した後は通常の入力");

    // 保留中に一致しないキー: 保留を破棄し、そのキーは捨てる
    s.key(KeyCode::Char('k'), Modifiers::CTRL);
    s.key(KeyCode::Char('z'), Modifiers::NONE);
    assert_eq!(s.text(), "a", "z は文字として入らない");
    s.e.handle_input(s.win, InputEvent::Text("b".into()));
    assert_eq!(s.text(), "ab");

    // 保留中の貼り付けは保留を破棄して本文を入れる
    s.key(KeyCode::Char('k'), Modifiers::CTRL);
    s.e.handle_input(s.win, InputEvent::Paste("P".into()));
    assert_eq!(s.text(), "abP");
    s.e.handle_input(s.win, InputEvent::Text("c".into()));
    assert_eq!(s.text(), "abPc");
}

#[test]
fn resize_は編集面の大きさを記録する() {
    let mut s = Scene::new("");
    s.e.drain_events();
    s.e.handle_input(
        s.win,
        InputEvent::Resize {
            cols: 100,
            rows: 30,
        },
    );
    assert_eq!(s.e.state().viewport(), (100, 30));
}
