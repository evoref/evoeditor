//! Frame 生成のシナリオテスト (docs/02 §8、docs/03 §14)。ASCII の Frame (`render_ascii`) で期待を書く。

use std::rc::Rc;
use std::sync::Arc;

use eve_core::{Args, CommandInvocation, Document, Editor, InvocationSource, Ports, TextFormat};
use eve_testkit::{AsciiOptions, FixedPaths, ManualClock, ManualJobSpawner, MemFs, render_ascii};
use eve_text::{ByteOff, Range, Selection, Text};
use eve_view::{Capabilities, Frame, InputEvent, RowSource, WindowId};

struct Scene {
    e: Editor,
    win: WindowId,
    view: eve_view::ViewId,
}

impl Scene {
    /// `|` がカーソル、`«` `»` が選択。画面は `cols` x `rows`。
    fn new(marked: &str, cols: u16, rows: u16) -> Scene {
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
        let (text, sel) = parse(marked);
        let (_, view, win) = e.open_document(Document::from_text(
            Text::from(text.as_str()),
            TextFormat::new_document(),
        ));
        e.state_mut().set_selection(view, sel);
        e.handle_input(win, InputEvent::Resize { cols, rows });
        e.state_mut().ensure_cursor_visible(view);
        Scene { e, win, view }
    }

    fn frame(&mut self) -> Frame {
        self.e.frame(self.win).expect("Frame")
    }

    fn ascii(&mut self) -> String {
        render_ascii(&self.frame(), AsciiOptions::default())
    }

    fn ascii_with_gutter(&mut self) -> String {
        render_ascii(
            &self.frame(),
            AsciiOptions {
                show_gutter: true,
                ..AsciiOptions::default()
            },
        )
    }

    fn run(&mut self, id: &str) {
        self.e
            .execute(CommandInvocation::new(id, InvocationSource::Keymap));
    }
}

fn parse(marked: &str) -> (String, Selection) {
    let (mut text, mut anchor, mut head, mut cursor) = (String::new(), None, None, None);
    for c in marked.chars() {
        match c {
            '«' => anchor = Some(text.len()),
            '»' => head = Some(text.len()),
            '|' => cursor = Some(text.len()),
            c => text.push(c),
        }
    }
    let sel = match (anchor, head, cursor) {
        (Some(a), Some(h), _) => Selection::single(Range::new(ByteOff(a), ByteOff(h))),
        (_, _, Some(p)) => Selection::point(ByteOff(p)),
        _ => Selection::point(ByteOff(0)),
    };
    (text, sel)
}

#[test]
fn 行番号と本文とカーソル() {
    let mut s = Scene::new("ab\ncd|e\n", 20, 4);
    assert_eq!(
        s.ascii_with_gutter(),
        " 1 ab\n 2 cde\n 3\n\ncursor: (5,1)\n"
    );
    // 文書の最後より後ろの行は空 (Virtual)
    let f = s.frame();
    assert_eq!(f.panes[0].rows[3].source, RowSource::Virtual);
    assert_eq!(
        f.panes[0].rows[1].source,
        RowSource::Line {
            line: 1,
            wrap_idx: 0
        }
    );
    assert_eq!(f.panes[0].gutter_width, 3);
    assert_eq!(f.panes[0].cursors.len(), 1);
}

#[test]
fn タブと全角スペースは記号で示し_幅は変えない() {
    let mut s = Scene::new("a\tb\n\u{3000}x|", 20, 3);
    assert_eq!(s.ascii(), "a>  b\n[]x\n\ncursor: (6,1)\n");
}

#[test]
fn 全角と結合文字と絵文字の幅() {
    let mut s = Scene::new("あe\u{301}😀|z", 20, 1);
    let f = s.frame();
    // あ(2) é(1) 😀(2) の後ろ = 列 5
    assert_eq!(f.primary_cursor.unwrap().pos.col, 3 + 5);
    let row = &f.panes[0].rows[0];
    let widths: Vec<u16> = row.spans.iter().map(|sp| sp.width).collect();
    assert_eq!(widths.iter().sum::<u16>(), 2 + 1 + 2 + 1);
    assert!(row.width_risky, "絵文字は幅危険書記素");
}

#[test]
fn 制御文字と不可視文字と不正バイトを記号にする() {
    // ESC → ^[、ZWSP → <ZWSP>、PUA エスケープ → <0x9A>
    let src = format!("a\u{1B}b\u{200B}c{}d|", eve_encoding::escape_byte(0x9A));
    let mut s = Scene::new(&src, 40, 1);
    assert_eq!(
        s.ascii(),
        "a^[b<ZWSP>c<0x9A>d
cursor: (21,0)
"
    );
    // INV-F1: Frame に制御文字・PUA エスケープ文字は残らない
    let f = s.frame();
    for sp in &f.panes[0].rows[0].spans {
        assert!(
            sp.text
                .chars()
                .all(|c| !c.is_control() && eve_encoding::unescape_char(c).is_none()),
            "{:?}",
            sp.text
        );
    }
}

#[test]
fn 選択は反転で_行末の改行も示す() {
    let mut s = Scene::new("ab«c\nde»f", 20, 2);
    assert_eq!(
        s.ascii(),
        "ab«c »
«de»f
cursor: (5,1)
"
    );
}

#[test]
fn 長い行は水平スクロールし_端の全角は空白で埋める() {
    let line = "0123456789".repeat(3); // 30 桁
    let mut s = Scene::new(&format!("{line}|"), 14, 1); // ガター 3 + 本文 11 桁
    let f = s.frame();
    let row = &f.panes[0].rows[0];
    assert!(
        row.clipped_left && !row.clipped_right,
        "カーソルが行末: 左が切れる"
    );
    assert_eq!(
        f.panes[0].cursors[0].col,
        3 + 10,
        "カーソルは本文の最後のセル"
    );
    // 全角が左端にかかる: 「あ」の右半分から始まる位置は空白で埋める
    let mut s = Scene::new("あいうえおかきくけこ|", 10, 1); // 本文 7 桁、20 桁の行
    let f = s.frame();
    let w: u16 = f.panes[0].rows[0].spans.iter().map(|sp| sp.width).sum();
    assert!(w <= 7, "本文の幅を超えない: {w}");
    assert!(f.panes[0].rows[0].clipped_left);
}

#[test]
fn 水平スクロールは戻る() {
    let line = "x".repeat(40);
    let mut s = Scene::new(&format!("{line}|"), 14, 1);
    assert!(s.frame().panes[0].rows[0].clipped_left);
    s.run("cursor.home");
    let f = s.frame();
    assert!(
        !f.panes[0].rows[0].clipped_left,
        "行頭に戻ればスクロールも戻る"
    );
    assert_eq!(f.panes[0].cursors[0].col, 3);
}

#[test]
fn 縦にスクロールした範囲だけを作る() {
    let text: String = (0..100).map(|i| format!("line{i}\n")).collect();
    let mut s = Scene::new(&format!("|{text}"), 20, 5);
    for _ in 0..50 {
        s.run("cursor.move_down");
    }
    let f = s.frame();
    assert_eq!(f.panes[0].rows.len(), 5, "可視行だけ");
    let first = match f.panes[0].rows[0].source {
        RowSource::Line { line, .. } => line,
        _ => panic!(),
    };
    assert!(
        (46..=50).contains(&first),
        "カーソルの行 (50) が見える範囲: 先頭 {first}"
    );
    assert!(f.panes[0].cursors.len() == 1);
}

#[test]
fn 画面の外のカーソルは含めない() {
    let text: String = (0..30).map(|i| format!("{i}\n")).collect();
    let mut s = Scene::new(&format!("|{text}"), 20, 5);
    s.run("cursor.doc_end");
    s.run("cursor.doc_start");
    // 行 0 に戻り、先頭が見える
    let f = s.frame();
    assert_eq!(
        f.panes[0].cursors,
        vec![eve_view::CellPos { col: 3, row: 0 }]
    );
    assert!(f.primary_cursor.is_some());
}

#[test]
fn タイトルと_revision() {
    let mut s = Scene::new("|", 20, 2);
    let t = s.frame().title;
    assert_eq!(t, "無題 - evoeditor");
    s.e.execute(
        CommandInvocation::new("edit.insert_text", InvocationSource::Keymap)
            .with_args(Args::new().with_str("text", "x")),
    );
    let f = s.frame();
    assert_eq!(f.title, "* 無題 - evoeditor");
    assert_eq!(f.panes[0].revision.0.0, 1);
    let _ = s.view;
}

#[test]
fn 絵文字の列は_zwj_と_vs16_を残して_幅のとおりに出す() {
    let mut s = Scene::new("👨\u{200D}👩\u{200D}👧❤\u{FE0F}x|", 20, 1);
    let f = s.frame();
    let row = &f.panes[0].rows[0];
    let all: String = row.spans.iter().map(|sp| sp.text.as_str()).collect();
    assert!(
        all.contains("👨\u{200D}👩\u{200D}👧"),
        "ZWJ 列を壊さない: {all:?}"
    );
    assert!(all.contains("❤\u{FE0F}"), "VS16 を残す: {all:?}");
    // 1 書記素 = 幅 2 の Span (3 つの文字を 1 つの Span に入れても幅は 2)
    let family = row
        .spans
        .iter()
        .find(|sp| sp.text.starts_with('👨'))
        .unwrap();
    assert_eq!(family.width, 2);
}

#[test]
fn 不正バイトのエスケープは結合文字と同じ書記素でも_記号になる() {
    let esc = eve_encoding::escape_byte(0x80);
    let src = format!("{esc}\u{301}z|");
    let mut s = Scene::new(&src, 30, 1);
    let f = s.frame();
    for sp in &f.panes[0].rows[0].spans {
        assert!(
            sp.text
                .chars()
                .all(|c| eve_encoding::unescape_char(c).is_none()),
            "{:?}",
            sp.text
        );
    }
    assert!(s.ascii().starts_with("<0x80>"), "{}", s.ascii());
}

#[test]
fn 入力で右端を越えたらスクロールしてカーソルが見える() {
    let mut s = Scene::new("|", 14, 2); // 本文 11 桁
    for _ in 0..30 {
        s.e.execute(
            CommandInvocation::new("edit.insert_text", InvocationSource::Keymap)
                .with_args(Args::new().with_str("text", "x")),
        );
    }
    let f = s.frame();
    assert!(f.primary_cursor.is_some(), "カーソルが見える");
    assert!(f.panes[0].rows[0].clipped_left);
    // 行を短くしても (Backspace)、カーソルが見える
    for _ in 0..25 {
        s.run("edit.delete_backward");
    }
    let f = s.frame();
    assert!(f.primary_cursor.is_some());
    assert!(!f.panes[0].rows[0].clipped_left);
    // 画面を狭くしても見える
    s.e.handle_input(s.win, InputEvent::Resize { cols: 8, rows: 2 });
    for _ in 0..10 {
        s.e.execute(
            CommandInvocation::new("edit.insert_text", InvocationSource::Keymap)
                .with_args(Args::new().with_str("text", "y")),
        );
    }
    assert!(s.frame().primary_cursor.is_some());
}

#[test]
fn 幅のある単位が右端にかかるときは全体が見える位置までスクロールする() {
    let mut s = Scene::new("0123456789\t|x", 14, 1); // 本文 11 桁。タブは 10 桁目から 12 桁目まで
    s.run("cursor.move_left");
    let f = s.frame();
    let c = f.primary_cursor.expect("カーソル").pos;
    let w: u16 = f.panes[0].rows[0].spans.iter().map(|sp| sp.width).sum();
    assert!(c.col - f.panes[0].gutter_width < w + 1);
    assert!(c.col < 14);
}

#[test]
fn 画面が狭いときはガターを出さない() {
    let mut s = Scene::new("ab|", 3, 1);
    let f = s.frame();
    assert_eq!(f.panes[0].gutter_width, 0);
    assert!(f.panes[0].rows[0].gutter.is_empty());
    let w: u16 = f.panes[0].rows[0].spans.iter().map(|sp| sp.width).sum();
    assert!(w <= 3);
}

#[test]
fn 縦移動の目標列は表示の幅で数える() {
    // 不正バイト (表示は 6 桁) の次の `a` は表示の 6 桁目。次の行に移っても表示の列を保つ
    let esc = eve_encoding::escape_byte(0x80);
    let mut s = Scene::new(&format!("{esc}a|bc\nxxxxxxxxxx"), 30, 2);
    s.run("cursor.move_down");
    let f = s.frame();
    // 上の行: <0x80>a の後ろ = 表示列 7。下の行も表示列 7
    assert_eq!(
        f.panes[0].cursors,
        vec![eve_view::CellPos { col: 3 + 7, row: 1 }]
    );
}

#[test]
fn 多数のカーソルでも可視範囲の分だけを作る() {
    let text: String = (0..2000).map(|i| format!("{i}\n")).collect();
    let mut s = Scene::new(&format!("|{text}"), 20, 5);
    let ranges: Vec<Range> = (0..2000).map(|i| Range::point(ByteOff(i * 2))).collect();
    // 先頭付近の 5 行にだけカーソルがある範囲を可視にする (バイト位置は 0, 2, 4 ... でおおむね各行の先頭)
    s.e.state_mut()
        .set_selection(s.view, Selection::new(ranges, 0).unwrap());
    let f = s.frame();
    assert_eq!(f.panes[0].rows.len(), 5);
    assert!(
        f.panes[0].cursors.len() <= 5 + 5,
        "可視行にあるカーソルだけ"
    );
}
