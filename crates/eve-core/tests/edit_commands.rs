//! 編集コマンドのシナリオテスト (docs/03 §5.3 / §15.3、01 の FR-201 / FR-202 / FR-203)。
//! 文書は `|` (カーソル) または `«` `»` (選択) を含む文字列で書く。Harness (C5 以降) ができるまでの小さな代用。

use std::rc::Rc;
use std::sync::Arc;

use eve_core::{
    Args, CommandInvocation, CommandResult, Document, Editor, InvocationSource, Permission, Ports,
    RejectReason, TextFormat, Tick,
};
use eve_testkit::{FixedPaths, ManualClock, ManualJobSpawner, MemFs};
use eve_text::{ByteOff, Range, Selection, Text};
use eve_view::{Capabilities, ViewId};

struct Scene {
    e: Editor,
    doc: eve_view::DocumentId,
    view: ViewId,
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
        let (doc, view, _) = e.open_document(Document::from_text(
            Text::from(text.as_str()),
            TextFormat::new_document(),
        ));
        e.state_mut().set_selection(view, sel);
        Scene { e, doc, view }
    }

    fn run(&mut self, id: &str) -> CommandResult {
        self.e
            .execute(CommandInvocation::new(id, InvocationSource::Keymap))
    }

    fn type_text(&mut self, s: &str) -> CommandResult {
        self.e.execute(
            CommandInvocation::new("edit.insert_text", InvocationSource::Keymap)
                .with_args(Args::new().with_str("text", s)),
        )
    }

    fn at(&mut self, t: u64) {
        self.e.tick(Tick(t));
    }

    fn shown(&self) -> String {
        let d = self.e.state().document(self.doc).unwrap();
        let text = d.to_string_lossy();
        let r = *self.e.state().view(self.view).unwrap().selection.primary();
        render(&text, r)
    }

    fn revision(&self) -> u64 {
        self.e.state().document(self.doc).unwrap().revision().0
    }
}

fn parse(marked: &str) -> (String, Selection) {
    if let (Some(a), Some(b)) = (marked.find('«'), marked.find('»')) {
        let text = marked.replace(['«', '»'], "");
        let end = b - '«'.len_utf8();
        return (
            text,
            Selection::single(Range::new(ByteOff(a), ByteOff(end))),
        );
    }
    let p = marked.find('|').expect("| か «» が要る");
    (marked.replace('|', ""), Selection::point(ByteOff(p)))
}

fn render(text: &str, r: Range) -> String {
    if r.is_empty() {
        let p = r.head.0;
        format!("{}|{}", &text[..p], &text[p..])
    } else {
        let (s, e) = (r.start().0, r.end().0);
        format!("{}«{}»{}", &text[..s], &text[s..e], &text[e..])
    }
}

#[test]
fn 文字入力とカーソル() {
    let mut s = Scene::new("ab|c");
    assert_eq!(s.type_text("XY"), CommandResult::Done);
    assert_eq!(s.shown(), "abXY|c");
    assert_eq!(s.type_text("あ"), CommandResult::Done);
    assert_eq!(s.shown(), "abXYあ|c");
}

#[test]
fn 選択への入力は置換し_undo_で選択ごと戻る() {
    let mut s = Scene::new("a«bc»d");
    s.type_text("X");
    assert_eq!(s.shown(), "aX|d");
    s.run("edit.undo");
    assert_eq!(
        s.shown(),
        "a«bc»d",
        "1 回の Undo で選択した文字列と選択が戻る (03 §15.3)"
    );
    s.run("edit.redo");
    assert_eq!(s.shown(), "aX|d");
}

// verifies: AC-FR-203-1
#[test]
fn 連続入力は_1_回の_undo_で戻り_時間窓で区切れる() {
    let mut s = Scene::new("|");
    for (i, ch) in ["a", "b", "c"].iter().enumerate() {
        s.at(100 * i as u64);
        s.type_text(ch);
    }
    s.at(1500);
    s.type_text("d"); // 窓 (1000 ms) を超えた
    assert_eq!(s.shown(), "abcd|");
    s.run("edit.undo");
    assert_eq!(s.shown(), "abc|");
    s.run("edit.undo");
    assert_eq!(s.shown(), "|", "abc が 1 つの単位");
    assert_eq!(
        s.run("edit.undo"),
        CommandResult::Done,
        "これ以上戻せなくてもエラーにしない"
    );
}

#[test]
fn カーソル移動は_undo_の結合を封印する() {
    let mut s = Scene::new("|");
    s.type_text("a");
    s.at(10);
    s.type_text("b");
    s.at(20);
    s.run("select.all"); // Motion: 封印
    s.e.state_mut()
        .set_selection(s.view, Selection::point(ByteOff(2)));
    s.at(30);
    s.type_text("c");
    s.run("edit.undo");
    assert_eq!(s.shown(), "ab|");
    s.run("edit.undo");
    assert_eq!(s.shown(), "|");
}

// verifies: AC-FR-202-1
// verifies: AC-FR-202-2
#[test]
fn 改行の自動インデント() {
    // 03 §15.3 の期待例。· は空白、→ はタブ
    let cases = [
        ("····foo|", "····foo\n····|"),
        ("→foo|", "→foo\n→|"),
        ("····foo|bar", "····foo\n····|bar"),
        ("|····foo", "\n|····foo"),
        ("··|··foo", "··\n··|··foo"),
        ("····|", "····\n····|"),
    ];
    for (before, after) in cases {
        let norm = |s: &str| s.replace('·', " ").replace('→', "\t");
        let mut s = Scene::new(&norm(before));
        s.run("edit.newline");
        assert_eq!(s.shown(), norm(after), "{before}");
    }
}

#[test]
fn 改行は文書の既定の改行を使い_選択を置換する() {
    let mut s = Scene::new("  a«bc»d");
    s.e.state_mut()
        .document_mut(s.doc)
        .unwrap()
        .format
        .default_eol = eve_text::LineEnding::Crlf;
    s.run("edit.newline");
    assert_eq!(
        s.shown(),
        "  a\r\n  |d",
        "参照行は start の行、複製するのは start より前の行頭空白"
    );
}

#[test]
fn バックスペースとデリートは書記素単位() {
    let mut s = Scene::new("a\r\n|b");
    s.run("edit.delete_backward");
    assert_eq!(s.shown(), "a|b", "CRLF は 1 つ");
    let mut s = Scene::new("a|\r\nb");
    s.run("edit.delete_forward");
    assert_eq!(s.shown(), "a|b");
    let mut s = Scene::new("e\u{301}|x");
    s.run("edit.delete_backward");
    assert_eq!(s.shown(), "|x", "結合文字は基底ごと");
    let mut s = Scene::new("a«bc»d");
    s.run("edit.delete_backward");
    assert_eq!(s.shown(), "a|d", "選択は 1 書記素を足さずに削除");
}

#[test]
fn 文書の端での削除は何も変えない() {
    let mut s = Scene::new("|abc");
    let rev = s.revision();
    s.run("edit.delete_backward");
    assert_eq!(
        (s.shown(), s.revision()),
        ("|abc".to_owned(), rev),
        "恒等: Revision も進まない"
    );
    let mut s = Scene::new("abc|");
    let rev = s.revision();
    s.run("edit.delete_forward");
    assert_eq!(s.revision(), rev);
    assert!(!s.e.state().document(s.doc).unwrap().is_dirty());
}

#[test]
fn 単語単位の削除() {
    // 03 §5.3 の表
    let back = [
        ("foo   |", "|"),
        ("foo bar|", "foo |"),
        ("foo\n|bar", "foo|bar"),
    ];
    for (before, after) in back {
        let mut s = Scene::new(before);
        s.run("edit.delete_word_backward");
        assert_eq!(s.shown(), after, "backward {before:?}");
    }
    let fwd = [
        ("|foo bar", "|bar"),
        ("foo| bar", "foo|bar"),
        ("foo|   \nx", "foo|\nx"),
        ("foo|\nbar", "foo|bar"),
    ];
    for (before, after) in fwd {
        let mut s = Scene::new(before);
        s.run("edit.delete_word_forward");
        assert_eq!(s.shown(), after, "forward {before:?}");
    }
    let mut s = Scene::new("a«bc d»e");
    s.run("edit.delete_word_backward");
    assert_eq!(s.shown(), "a|e", "選択ありは選択だけを削除");
    let mut s = Scene::new("|abc");
    let rev = s.revision();
    s.run("edit.delete_word_backward");
    assert_eq!(
        s.revision(),
        rev,
        "何も消えないときは Transaction を作らない"
    );
}

#[test]
fn 連打した_backspace_は_1_回の_undo() {
    let mut s = Scene::new("abcdef|");
    for i in 0..3 {
        s.at(10 * i);
        s.run("edit.delete_backward");
    }
    assert_eq!(s.shown(), "abc|");
    s.run("edit.undo");
    assert_eq!(s.shown(), "abcdef|");
}

#[test]
fn tab_とシフトタブ() {
    let mut s = Scene::new("a|b");
    s.run("edit.indent");
    assert_eq!(s.shown(), "a   |b", "次のインデント停止位置 (4) まで");
    let mut s = Scene::new("|x");
    s.run("edit.indent");
    assert_eq!(s.shown(), "    |x");
    // Shift+Tab: 表示幅 4 以内に収まる行頭空白を削除 (03 §15.3 の例)
    for (before, after) in [("  \tx|", "x|"), (" \t\tx|", "\tx|"), ("      x|", "  x|")] {
        let mut s = Scene::new(before);
        s.run("edit.outdent");
        assert_eq!(s.shown(), after, "{before:?}");
    }
    let mut s = Scene::new("x|");
    let rev = s.revision();
    s.run("edit.outdent");
    assert_eq!(s.revision(), rev, "行頭空白がなければ何もしない");
}

#[test]
fn 選択があるときの_tab_は何も変えない() {
    for id in ["edit.indent", "edit.outdent"] {
        let mut s = Scene::new("  «ab»c");
        let rev = s.revision();
        assert_eq!(s.run(id), CommandResult::Done);
        assert_eq!(
            (s.shown(), s.revision()),
            ("  «ab»c".to_owned(), rev),
            "{id}"
        );
        assert!(!s.e.state().document(s.doc).unwrap().is_dirty());
    }
}

#[test]
fn 全選択して入力すると全体を置換する() {
    let mut s = Scene::new("ab|c");
    s.run("select.all");
    assert_eq!(s.shown(), "«abc»");
    s.type_text("Z");
    assert_eq!(s.shown(), "Z|");
}

#[test]
fn 読み取り専用では編集コマンドを拒否し_全選択は通る() {
    let mut s = Scene::new("ab|c");
    s.e.state_mut()
        .document_mut(s.doc)
        .unwrap()
        .read_only
        .insert(eve_core::ReadOnlyReason::UserRequested);
    assert_eq!(
        s.type_text("x"),
        CommandResult::Rejected(RejectReason::NotAvailable)
    );
    assert_eq!(
        s.run("edit.delete_backward"),
        CommandResult::Rejected(RejectReason::NotAvailable)
    );
    assert_eq!(
        s.run("edit.undo"),
        CommandResult::Rejected(RejectReason::NotAvailable)
    );
    assert_eq!(s.run("select.all"), CommandResult::Done);
    assert_eq!(s.shown(), "«abc»");
}

#[test]
fn 権限_edit_は外部から呼べ_undo_は呼べない() {
    let mut s = Scene::new("|");
    let ext = |e: &mut Editor, id: &str| {
        e.execute(CommandInvocation::new(id, InvocationSource::External))
    };
    assert_eq!(
        ext(&mut s.e, "edit.undo"),
        CommandResult::Rejected(RejectReason::PermissionDenied)
    );
    assert_eq!(
        ext(&mut s.e, "edit.redo"),
        CommandResult::Rejected(RejectReason::PermissionDenied)
    );
    assert_eq!(
        ext(&mut s.e, "edit.newline"),
        CommandResult::Done,
        "propose は可"
    );
    let _ = Permission::Propose;
}

#[test]
fn text_引数がない入力は拒否() {
    let mut s = Scene::new("|");
    assert_eq!(
        s.run("edit.insert_text"),
        CommandResult::Rejected(RejectReason::InvalidArgs)
    );
}

fn scene_with(text: &str, sel: Vec<(usize, usize)>, primary: usize) -> Scene {
    let mut s = Scene::new(&format!("{text}|"));
    let ranges = sel
        .into_iter()
        .map(|(a, h)| Range::new(ByteOff(a), ByteOff(h)))
        .collect();
    let selection = Selection::new(ranges, primary).unwrap();
    // 複数の Range は、正規化を通る `set_selection` で作る (併合されないものだけを渡す)
    s.e.state_mut().set_selection(s.view, selection);
    s
}

fn all(s: &Scene) -> (String, Vec<(usize, usize)>) {
    let d = s.e.state().document(s.doc).unwrap();
    let v = s.e.state().view(s.view).unwrap();
    (
        d.to_string_lossy(),
        v.selection.iter().map(|r| (r.anchor.0, r.head.0)).collect(),
    )
}

#[test]
fn 複数のカーソルでの重なる削除は先行を優先して切り詰める() {
    // "foo bar" のカーソル 5 と 7 で単語削除 (後方): [4,5) と [4,7) → 先行を優先、後続は重なりを切り詰める
    let mut s = scene_with("foo bar", vec![(5, 5), (7, 7)], 0);
    assert_eq!(s.run("edit.delete_word_backward"), CommandResult::Done);
    let (text, sel) = all(&s);
    assert_eq!(text, "foo ");
    assert_eq!(
        sel,
        vec![(4, 4)],
        "2 つのカーソルは同じ位置になり 1 つに併合される"
    );
}

#[test]
fn 複数のカーソルで同じ行をシフトタブしても_1_回だけ() {
    let mut s = scene_with("    ab", vec![(5, 5), (6, 6)], 0);
    assert_eq!(s.run("edit.outdent"), CommandResult::Done);
    assert_eq!(all(&s).0, "ab");
}

#[test]
fn 編集のない_range_は他の挿入位置で前に留まる() {
    // 選択 [0,4) と、その端の primary カーソル 4 (N4 で残る)。Tab はカーソルだけが挿入する
    let mut s = scene_with("abcdefgh", vec![(0, 4), (4, 4)], 1);
    assert_eq!(s.run("edit.indent"), CommandResult::Done);
    let (text, sel) = all(&s);
    assert_eq!(text, "abcd    efgh");
    assert_eq!(sel[0], (0, 4), "選択は挿入した空白を覆わない");
}
