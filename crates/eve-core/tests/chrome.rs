//! クローム (ステータスバー・ヒントバー・メッセージ行) と文言の解決のシナリオ (docs/04 §8.7〜§8.9、07 §17)。

use std::rc::Rc;
use std::sync::Arc;

use eve_core::{Args, CommandInvocation, Document, Editor, InvocationSource, Ports, TextFormat};
use eve_testkit::{AsciiOptions, FixedPaths, ManualClock, ManualJobSpawner, MemFs, render_ascii};
use eve_text::{ByteOff, Selection, Text};
use eve_view::{
    Capabilities, Frame, InputEvent, KeyCode, KeyEvent, MessageLevel, Modifiers, WindowId,
};

struct Env {
    e: Editor,
    fs: Arc<MemFs>,
    jobs: Rc<ManualJobSpawner>,
    win: Option<WindowId>,
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
        let win = e.state().first_window();
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

    fn frame(&mut self) -> Frame {
        self.e.frame(self.win.unwrap()).unwrap()
    }

    fn chrome(&mut self) -> String {
        let f = self.frame();
        render_ascii(
            &f,
            AsciiOptions {
                show_chrome: true,
                show_cursor: false,
                mark_selection: false,
                ..AsciiOptions::default()
            },
        )
        .lines()
        .skip_while(|l| !l.starts_with("status:"))
        .collect::<Vec<_>>()
        .join("\n")
    }

    fn key(&mut self, code: KeyCode, mods: Modifiers) {
        self.e.handle_input(
            self.win.unwrap(),
            InputEvent::Key(KeyEvent::press(code, mods)),
        );
    }

    fn type_text(&mut self, s: &str) {
        self.e
            .handle_input(self.win.unwrap(), InputEvent::Text(s.into()));
    }
}

#[test]
fn 無題の文書のステータス() {
    let mut s = Env::new();
    let c = s.chrome();
    assert!(
        c.starts_with(
            "status: 無題-1 | 行 1, 桁 1 UTF-8 LF
hints: "
        ),
        "{c}"
    );
}

#[test]
fn 変更と位置と選択のステータス() {
    let mut s = Env::new();
    s.type_text("あいう\nえお");
    let c = s.chrome();
    assert!(
        c.starts_with("status: 無題-1 [+] | 行 2, 桁 3 UTF-8 LF"),
        "{c}"
    );
    // 選択: 「(N 文字選択)」
    s.run("select.all", Args::new());
    let c = s.chrome();
    assert!(c.contains("(6 文字選択)"), "{c}");
}

#[test]
fn ファイルの状態のステータス() {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"\xEF\xBB\xBFabc\r\ndef\r\n");
    s.run("file.open", Args::new().with_str("path", "/w/a.txt"));
    let c = s.chrome();
    assert!(
        c.starts_with("status: a.txt | 行 1, 桁 1 UTF-8 BOM CRLF"),
        "{c}"
    );
    // 改行が混在
    s.fs.add_file("/w/m.txt", b"a\nb\r\nc\n");
    s.run("file.open", Args::new().with_str("path", "/w/m.txt"));
    assert!(s.chrome().contains("混在"), "{}", s.chrome());
    // 読み取り専用
    s.fs.add_file("/w/bad.txt", b"x\x9A");
    s.run("file.open", Args::new().with_str("path", "/w/bad.txt"));
    let c = s.chrome();
    assert!(c.starts_with("status: bad.txt [RO] |"), "{c}");
}

#[test]
fn 短い形と優先度は_core_が決めて渡す() {
    let mut s = Env::new();
    let f = s.frame();
    let pos = f
        .status
        .items
        .iter()
        .find(|i| i.kind == eve_view::StatusKind::Position)
        .unwrap();
    assert_eq!(pos.priority, 4);
    assert_eq!(pos.short.as_ref().map(|t| t.width), Some(3), "1:1");
    let long: u16 = pos.long.spans.iter().map(|sp| sp.width).sum();
    assert_eq!(pos.long.width, long, "幅は core が計算済み");
    let name = &f.status.items[0];
    assert_eq!(name.kind, eve_view::StatusKind::Path);
    assert_eq!(name.long.cut, eve_view::CutPolicy::Middle);
}

#[test]
fn 保存の成功は情報メッセージで_次のキーで消える() {
    let mut s = Env::new();
    s.type_text("hi");
    s.run("file.save_as", Args::new().with_str("path", "/w/n.txt"));
    let f = s.frame();
    let m = f.messages.current.expect("メッセージ");
    assert_eq!(m.level, MessageLevel::Info);
    let text: String = m.text.spans.iter().map(|sp| sp.text.as_str()).collect();
    assert_eq!(text, "/w/n.txt を保存しました");
    assert!(m.hint.is_none());
    assert!(f.messages.unread >= 1);
    s.key(KeyCode::Right, Modifiers::NONE);
    assert!(
        s.frame().messages.current.is_none(),
        "次のキー入力で消える (Info)"
    );
}

#[test]
fn 変更なしの保存の文言() {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"abc");
    s.run("file.open", Args::new().with_str("path", "/w/a.txt"));
    s.run("file.save", Args::new());
    let c = s.chrome();
    assert!(
        c.contains("message[Info]: 変更はありません (ファイルは書き換えていません)"),
        "{c}"
    );
}

#[test]
fn エラーは_text_と_hint_の_2_部でキー表記を解決する() {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"abc");
    s.run("file.open", Args::new().with_str("path", "/w/a.txt"));
    s.type_text("x");
    s.fs.fail_next(eve_testkit::FsOp::WriteAtomic, eve_core::FsErrorKind::Other);
    s.run("file.save", Args::new());
    let f = s.frame();
    let m = f.messages.current.expect("エラー");
    assert_eq!(m.level, MessageLevel::Error);
    let text: String = m.text.spans.iter().map(|sp| sp.text.as_str()).collect();
    assert!(
        text.starts_with("/w/a.txt を保存できませんでした: "),
        "{text}"
    );
    let hint: String = m
        .hint
        .expect("hint")
        .spans
        .iter()
        .map(|sp| sp.text.as_str())
        .collect();
    assert_eq!(
        hint,
        "別名で保存 (Ctrl+K S) するか、権限を確認してください".replace("Ctrl+K S", "Ctrl+Shift+S")
    );
    // Error は Esc で消える。カーソル移動では消えない
    s.key(KeyCode::Left, Modifiers::NONE);
    assert!(s.frame().messages.current.is_some());
    s.key(KeyCode::Esc, Modifiers::NONE);
    assert!(s.frame().messages.current.is_none());
}

#[test]
fn ヒントバーは割当のある項目を優先度の降順で出す() {
    let mut s = Env::new();
    let f = s.frame();
    let words: Vec<String> = f
        .hints
        .items
        .iter()
        .map(|h| h.text.spans.iter().map(|sp| sp.text.as_str()).collect())
        .collect();
    assert_eq!(words, ["保存", "開く", "元に戻す", "行へ移動"]);
    assert!(
        f.hints
            .items
            .windows(2)
            .all(|w| w[0].priority >= w[1].priority)
    );
    // キー: file.save は Ctrl+S
    let k = &f.hints.items[0].keys[0].strokes[0];
    assert_eq!((k.key, k.mods), (KeyCode::Char('s'), Modifiers::CTRL));
}

#[test]
fn チョードの保留中は取消だけを出す() {
    let mut s = Env::new();
    s.key(KeyCode::Char('k'), Modifiers::CTRL);
    let f = s.frame();
    assert_eq!(f.hints.context, eve_view::HintContext::Overlay);
    assert_eq!(f.hints.items.len(), 1);
    let word: String = f.hints.items[0]
        .text
        .spans
        .iter()
        .map(|sp| sp.text.as_str())
        .collect();
    assert_eq!(word, "取消");
    assert_eq!(f.hints.items[0].keys[0].strokes[0].key, KeyCode::Esc);
    let chord = f
        .status
        .items
        .iter()
        .find(|i| i.kind == eve_view::StatusKind::Chord)
        .expect("チョード");
    let t: String = chord.long.spans.iter().map(|sp| sp.text.as_str()).collect();
    assert_eq!(t, "Ctrl+K …");
    s.key(KeyCode::Esc, Modifiers::NONE);
    assert_eq!(s.frame().hints.context, eve_view::HintContext::Normal);
}

#[test]
fn 小さすぎる端末の文言と_制御文字を含まない文言() {
    let mut s = Env::new();
    let f = s.frame();
    let t: String = f
        .texts
        .too_small
        .spans
        .iter()
        .map(|sp| sp.text.as_str())
        .collect();
    assert_eq!(t, "端末が小さすぎます (20x4 以上が必要)");
    // INV-F1: パスの制御文字は記号になる
    s.fs.add_file("/w/a\u{1B}b.txt", b"x");
    s.run("file.open", Args::new().with_str("path", "/w/a\u{1B}b.txt"));
    let f = s.frame();
    let all: String = f
        .status
        .items
        .iter()
        .flat_map(|i| i.long.spans.iter())
        .map(|sp| sp.text.as_str())
        .collect();
    assert!(all.chars().all(|c| !c.is_control()), "{all:?}");
    let _ = (
        Document::new_untitled(),
        TextFormat::new_document(),
        Text::new(),
        ByteOff(0),
        Selection::point(ByteOff(0)),
    );
}

#[test]
fn 存在しない文言キーはキー名で出る() {
    let mut s = Env::new();
    s.e.state_mut()
        .show_message(MessageLevel::Info, "info.nothing.here", &[]);
    let f = s.frame();
    let t: String = f
        .messages
        .current
        .unwrap()
        .text
        .spans
        .iter()
        .map(|sp| sp.text.as_str())
        .collect();
    assert_eq!(t, "info.nothing.here");
}
