//! 移動・選択コマンドのシナリオテスト (docs/03 §5.1〜§5.6、01 の FR-301〜FR-306)。
//! 文書は `|` (カーソル) または `«` `»` (選択。`«` 側が anchor) を含む文字列で書く。

use std::rc::Rc;
use std::sync::Arc;

use eve_core::{
    Args, CommandInvocation, CommandResult, Document, Editor, InvocationSource, Ports,
    RejectReason, TextFormat,
};
use eve_testkit::{FixedPaths, ManualClock, ManualJobSpawner, MemFs};
use eve_text::{ByteOff, Range, Selection, Text};
use eve_view::{Capabilities, InputEvent, KeyCode, KeyEvent, Modifiers, ViewId, WindowId};

struct Scene {
    e: Editor,
    doc: eve_view::DocumentId,
    view: ViewId,
    win: WindowId,
}

impl Scene {
    fn new(marked: &str) -> Scene {
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
        let (doc, view, win) = e.open_document(Document::from_text(
            Text::from(text.as_str()),
            TextFormat::new_document(),
        ));
        e.state_mut().set_selection(view, sel);
        Scene { e, doc, view, win }
    }

    fn run(&mut self, id: &str) -> CommandResult {
        self.e
            .execute(CommandInvocation::new(id, InvocationSource::Keymap))
    }

    fn run_args(&mut self, id: &str, args: Args) -> CommandResult {
        self.e
            .execute(CommandInvocation::new(id, InvocationSource::Keymap).with_args(args))
    }

    fn key(&mut self, code: KeyCode, mods: Modifiers) {
        self.e
            .handle_input(self.win, InputEvent::Key(KeyEvent::press(code, mods)));
    }

    fn shown(&self) -> String {
        let d = self.e.state().document(self.doc).unwrap();
        let text = d.to_string_lossy();
        let r = *self.e.state().view(self.view).unwrap().selection.primary();
        render(&text, r)
    }

    fn goal(&self) -> Option<u32> {
        self.e.state().view(self.view).unwrap().goal_dcol
    }

    fn top_line(&self) -> usize {
        let v = self.e.state().view(self.view).unwrap();
        let d = self.e.state().document(self.doc).unwrap();
        use eve_text::TextRead;
        d.text().byte_to_line(v.scroll.top).0
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
        _ => panic!("| か «» が要る"),
    };
    (text, sel)
}

fn render(text: &str, r: Range) -> String {
    if r.is_empty() {
        format!("{}|{}", &text[..r.head.0], &text[r.head.0..])
    } else {
        // anchor 側を « に、head 側を » にする
        let (a, h) = (r.anchor.0, r.head.0);
        let (s, e) = (a.min(h), a.max(h));
        if a <= h {
            format!("{}«{}»{}", &text[..s], &text[s..e], &text[e..])
        } else {
            format!("{}»{}«{}", &text[..s], &text[s..e], &text[e..])
        }
    }
}

fn check(before: &str, cmd: &str, after: &str) {
    let mut s = Scene::new(before);
    assert_eq!(s.run(cmd), CommandResult::Done, "{cmd}");
    assert_eq!(s.shown(), after, "{cmd}: {before:?}");
}

#[test]
fn 左右は書記素単位で_行をまたぐ() {
    check("a|b", "cursor.move_left", "|ab");
    check("|ab", "cursor.move_left", "|ab"); // 文書先頭では動かない
    check("a|b", "cursor.move_right", "ab|");
    check("ab|", "cursor.move_right", "ab|");
    check("a|\r\nb", "cursor.move_right", "a\r\n|b"); // CRLF は 1 ステップ
    check("a\r\n|b", "cursor.move_left", "a|\r\nb");
    check("e|\u{301}x", "cursor.move_right", "e\u{301}|x");
    check(
        "|\u{1F468}\u{200D}\u{1F469}x",
        "cursor.move_right",
        "\u{1F468}\u{200D}\u{1F469}|x",
    );
}

#[test]
fn 選択ありの左右は端へ潰れて動かない() {
    check("a«bc»d", "cursor.move_left", "a|bcd");
    check("a«bc»d", "cursor.move_right", "abc|d");
    check("a»bc«d", "cursor.move_left", "a|bcd"); // 向きによらず start
}

#[test]
fn 拡張は_anchor_を動かさない() {
    check("a|bc", "select.extend_right", "a«b»c");
    check("a|bc", "select.extend_left", "»a«bc");
    check("a«b»c", "select.extend_right", "a«bc»");
    check("a»b«c", "select.extend_right", "ab|c"); // 後ろ向きの選択が縮んで空になる (anchor == head)
    check("a|bc def", "select.extend_word_right", "a«bc »def");
    check("ab|c", "select.extend_home", "»ab«c");
    check("a|bc", "select.extend_end", "a«bc»");
    check("a|bc", "select.extend_to_doc_end", "a«bc»");
    check("a|bc", "select.extend_to_doc_start", "»a«bc");
}

#[test]
fn 単語移動() {
    check("|foo bar", "cursor.move_word_right", "foo |bar");
    check("foo| bar", "cursor.move_word_right", "foo |bar");
    check("foo bar|", "cursor.move_word_left", "foo |bar");
    check("日本語を|勉強", "cursor.move_word_left", "日本語|を勉強");
    check("foo|\nbar", "cursor.move_word_right", "foo\n|bar");
}

// verifies: AC-FR-303-1 AC-FR-303-2
#[test]
fn スマートホームとエンド() {
    check("    fo|o", "cursor.home", "    |foo"); // インデント終端へ
    check("    |foo", "cursor.home", "|    foo"); // 終端にいれば行頭へ
    check("  | foo", "cursor.home", "   |foo"); // インデントの途中 → 終端
    check("|    foo", "cursor.home", "    |foo"); // 行頭 → 終端 (トグル)
    check("fo|o", "cursor.home", "|foo"); // インデントが無ければ常に行頭
    check("a\n    |", "cursor.home", "a\n|    "); // 空白だけの行: 終端 == 行末 → 行頭
    check("f|oo\nbar", "cursor.end", "foo|\nbar");
    check("foo|\nbar", "cursor.end", "foo|\nbar");
    check("a|\r\nb", "cursor.end", "a|\r\nb");
}

#[test]
fn 文書の端() {
    check("a\nb|c\nd", "cursor.doc_start", "|a\nbc\nd");
    check("a\nb|c\nd", "cursor.doc_end", "a\nbc\nd|");
}

#[test]
fn 上下移動は目標列を維持する() {
    let mut s = Scene::new("abcde|f\nab\nabcdef");
    s.run("cursor.move_down");
    assert_eq!(s.shown(), "abcdef\nab|\nabcdef", "短い行は行末");
    assert_eq!(s.goal(), Some(5));
    s.run("cursor.move_down");
    assert_eq!(s.shown(), "abcdef\nab\nabcde|f", "目標列 5 に戻る");
    s.run("cursor.move_up");
    s.run("cursor.move_up");
    assert_eq!(s.shown(), "abcde|f\nab\nabcdef");
    // 水平移動は目標列をクリアする
    s.run("cursor.move_down");
    s.run("cursor.move_left");
    assert_eq!(s.goal(), None);
}

#[test]
fn 全角の右半分は前に寄せる() {
    // 上の行 "ab" の列 2 から、下の行 "あいう" へ: 列 2 は「い」の前
    let mut s = Scene::new("ab|\nあいう");
    s.run("cursor.move_down");
    assert_eq!(s.shown(), "ab\nあ|いう");
    // 列 3 (「い」の途中) を目標にする: "abc|" から
    let mut s = Scene::new("abc|\nあいう");
    s.run("cursor.move_down");
    assert_eq!(s.shown(), "abc\nあ|いう", "セルの途中は前の書記素境界");
}

#[test]
fn 先頭行で上と最終行で下は文書の端へ() {
    check("ab|c\nd", "cursor.move_up", "|abc\nd");
    check("a\nb|c", "cursor.move_down", "a\nbc|");
    check("ab|c", "cursor.move_down", "abc|");
}

#[test]
fn 選択ありでも垂直移動は_head_から動く() {
    let mut s = Scene::new("ab«cd\nef»gh");
    s.run("cursor.move_up");
    assert_eq!(
        s.shown(),
        "ab|cd\negh".replace("egh", "efgh"),
        "head を起点に移動して潰す"
    );
}

#[test]
fn count_引数で繰り返す() {
    let mut s = Scene::new("|abcdef");
    s.run_args("cursor.move_right", Args::new().with_int("count", 3));
    assert_eq!(s.shown(), "abc|def");
    assert_eq!(
        s.run_args("cursor.move_right", Args::new().with_int("count", 0)),
        CommandResult::Rejected(RejectReason::InvalidArgs)
    );
    let mut s = Scene::new("a«bc»d");
    s.run_args("cursor.move_left", Args::new().with_int("count", 2));
    assert_eq!(s.shown(), "|abcd", "1 回目で start へ、2 回目で 1 つ戻る");
}

#[test]
fn ページ移動はスクロールも動かし_目標列を保つ() {
    let text: String = (0..20).map(|i| format!("line{i:02}\n")).collect::<String>();
    let mut s = Scene::new(&format!("li|{}", &text[2..]));
    s.e.handle_input(s.win, InputEvent::Resize { cols: 80, rows: 6 });
    s.run("cursor.page_down"); // 5 行 (6 − 1) 進む
    assert_eq!(s.shown().lines().nth(5).unwrap(), "li|ne05");
    assert_eq!(
        s.top_line(),
        5,
        "スクロールも同量動く (カーソルの相対位置を保つ)"
    );
    assert_eq!(s.goal(), Some(2));
    s.run("cursor.page_up");
    assert_eq!(s.shown().lines().next().unwrap(), "li|ne00");
    assert_eq!(s.top_line(), 0);
    s.run("cursor.page_up");
    assert_eq!(
        s.shown().lines().next().unwrap(),
        "|line00",
        "先頭では文書先頭へ"
    );
}

#[test]
fn 範囲外へ動くとスクロールして見える位置に保つ() {
    let text: String = (0..30).map(|i| format!("{i}\n")).collect();
    let mut s = Scene::new(&format!("|{text}"));
    s.e.handle_input(s.win, InputEvent::Resize { cols: 80, rows: 10 });
    for _ in 0..12 {
        s.run("cursor.move_down");
    }
    assert_eq!(
        s.top_line(),
        6,
        "12 行目のカーソルは下から 3 行の余白を残す (rows = 10)"
    );
    s.run("cursor.doc_start");
    assert_eq!(s.top_line(), 0);
}

#[test]
fn 指定行へ移動() {
    let t = "aa\nbb\ncc\ndd";
    let go = |args: Args| {
        let mut s = Scene::new(&format!("|{t}"));
        assert_eq!(s.run_args("cursor.goto_line", args), CommandResult::Done);
        s.shown()
    };
    assert_eq!(go(Args::new().with_int("line", 3)), "aa\nbb\n|cc\ndd");
    assert_eq!(
        go(Args::new().with_int("line", 3).with_int("col", 2)),
        "aa\nbb\nc|c\ndd"
    );
    assert_eq!(go(Args::new().with_str("line", "$")), "aa\nbb\ncc\n|dd");
    assert_eq!(
        go(Args::new().with_int("line", 99)),
        "aa\nbb\ncc\n|dd",
        "範囲外は最終行へ丸める"
    );
    for bad in [0, -3] {
        let mut s = Scene::new("aa\n|bb\ncc\ndd");
        assert_eq!(
            s.run_args("cursor.goto_line", Args::new().with_int("line", bad)),
            CommandResult::Rejected(eve_core::RejectReason::InvalidArgs),
            "1 未満は不正な入力 (03 §5.6)"
        );
        assert_eq!(s.shown(), "aa\n|bb\ncc\ndd", "動かない");
    }
    assert_eq!(
        go(Args::new().with_int("line", 2).with_int("col", 99)),
        "aa\nbb|\ncc\ndd",
        "桁は行末に丸める"
    );
    assert_eq!(
        go(Args::new()),
        "|aa
bb
cc
dd",
        "line がなければ何もしない"
    );
    assert_eq!(
        go(Args::new().with_int("line", 2).with_int("col", 0)),
        "aa
|bb
cc
dd",
        "col も範囲外は丸める"
    );
    let mut s = Scene::new("|a");
    assert_eq!(
        s.run_args("cursor.goto_line", Args::new().with_str("line", "x")),
        CommandResult::Rejected(RejectReason::InvalidArgs)
    );
    // 書記素単位の桁 (結合文字は 1 桁)
    let mut s = Scene::new("|e\u{301}x\ny");
    s.run_args(
        "cursor.goto_line",
        Args::new().with_int("line", 1).with_int("col", 2),
    );
    assert_eq!(s.shown(), "e\u{301}|x\ny");
}

#[test]
fn キーからの移動() {
    let mut s = Scene::new("a|bc\nde");
    s.key(KeyCode::Right, Modifiers::NONE);
    assert_eq!(s.shown(), "ab|c\nde");
    s.key(KeyCode::Down, Modifiers::NONE);
    assert_eq!(s.shown(), "abc\nde|");
    s.key(KeyCode::Home, Modifiers::NONE);
    assert_eq!(s.shown(), "abc\n|de");
    s.key(KeyCode::Right, Modifiers::SHIFT);
    assert_eq!(s.shown(), "abc\n«d»e");
    s.key(KeyCode::Left, Modifiers::CTRL);
    assert_eq!(s.shown(), "abc\n|de");
    s.key(KeyCode::End, Modifiers::CTRL | Modifiers::SHIFT);
    assert_eq!(s.shown(), "abc\n«de»");
    // チョードの代替キー: ctrl+k left = cursor.home
    s.key(KeyCode::Char('k'), Modifiers::CTRL);
    s.key(KeyCode::Left, Modifiers::NONE);
    assert_eq!(s.shown(), "abc\n|de");
}

#[test]
fn 移動は文書を変えず_読み取り専用でも使える() {
    let mut s = Scene::new("a|bc");
    s.e.state_mut()
        .document_mut(s.doc)
        .unwrap()
        .read_only
        .insert(eve_core::ReadOnlyReason::InvalidBytes);
    let rev = s.e.state().document(s.doc).unwrap().revision();
    assert_eq!(s.run("cursor.move_right"), CommandResult::Done);
    assert_eq!(s.shown(), "ab|c");
    assert_eq!(s.e.state().document(s.doc).unwrap().revision(), rev);
    assert!(!s.e.state().document(s.doc).unwrap().is_dirty());
}

#[test]
fn 外部の主体は移動を呼べる() {
    let mut s = Scene::new("|ab");
    let r = s.e.execute(CommandInvocation::new(
        "cursor.move_right",
        InvocationSource::External,
    ));
    assert_eq!(r, CommandResult::Done, "read は可");
}

#[test]
fn 複数のカーソルの上下移動は_primary_の目標列_他は各自の列() {
    let mut s = Scene::new("|abcdef\nabcdef\nabcdef");
    let r = |p: usize| Range::point(ByteOff(p));
    // 2 つのカーソル: 列 2 (primary でない) と 列 5 (primary)
    s.e.state_mut()
        .set_selection(s.view, Selection::new(vec![r(2), r(5)], 1).unwrap());
    s.run("cursor.move_down");
    let v = s.e.state().view(s.view).unwrap();
    let heads: Vec<usize> = v.selection.iter().map(|r| r.head.0).collect();
    assert_eq!(heads, [9, 12], "それぞれの列 (2 と 5) のまま次の行へ");
    assert_eq!(v.goal_dcol, Some(5), "目標列は primary のもの");
}

#[test]
fn 大きな_count_でも止まり_ページ移動の量が溢れない() {
    let mut s = Scene::new("|abc");
    s.run_args("cursor.move_right", Args::new().with_int("count", i64::MAX));
    assert_eq!(s.shown(), "abc|", "動けなくなったら打ち切る");
    s.run_args("cursor.page_down", Args::new().with_int("count", i64::MAX));
}

#[test]
fn ページ移動で短い文書を画面の外へ押し出さない() {
    let text: String = (0..20).map(|i| format!("l{i}\n")).collect();
    let mut s = Scene::new(&format!("|{text}"));
    s.e.handle_input(s.win, InputEvent::Resize { cols: 80, rows: 24 });
    s.run("cursor.page_down");
    assert_eq!(s.top_line(), 0, "21 行 (末尾の空行を含む) は 24 行に収まる");
}

#[test]
fn 指定行へ移動は行をビューポートの中央に置く() {
    let text: String = (0..50).map(|i| format!("{i}\n")).collect();
    let mut s = Scene::new(&format!("|{text}"));
    s.e.handle_input(s.win, InputEvent::Resize { cols: 80, rows: 10 });
    s.run_args("cursor.goto_line", Args::new().with_int("line", 30));
    assert_eq!(
        s.top_line(),
        29 - 5,
        "29 行目 (0 始まり) を中央 (rows / 2) に"
    );
}
