//! 複数カーソルの試験の印の読み書きと場面の用意 (`multi_cursor.rs`・`multi_cursor_commands.rs` で共有する)。
//!
//! 文書は `|` (空のカーソル) と `«` `»` (前向きの選択。`»` が head) と `‹` `›` (後ろ向きの選択。`‹` が head) を含む文字列で
//! 書く。主の Range は `*` を直前に置いたもの (無ければ最後の Range)。

use eve_testkit::Harness;
use eve_text::{ByteOff, Range, Selection};
use eve_view::ViewId;

/// 印を読んで (本文, Range の並び, 主の番号) にする。
pub fn parse(marked: &str) -> (String, Vec<Range>, usize) {
    let mut text = String::new();
    let mut ranges = Vec::new();
    let mut primary = None;
    let mut open: Option<(usize, char)> = None;
    for c in marked.chars() {
        let p = text.len();
        match c {
            '*' => primary = Some(ranges.len()),
            '|' => ranges.push(Range::point(ByteOff(p))),
            '«' | '‹' => open = Some((p, c)),
            '»' => {
                let (a, _) = open.take().expect("« の後の »");
                ranges.push(Range::new(ByteOff(a), ByteOff(p)));
            }
            '›' => {
                let (a, _) = open.take().expect("‹ の後の ›");
                ranges.push(Range::new(ByteOff(p), ByteOff(a)));
            }
            c => text.push(c),
        }
    }
    assert!(!ranges.is_empty(), "カーソルが要る: {marked}");
    let primary = primary.unwrap_or(ranges.len() - 1);
    (text, ranges, primary)
}

/// 本文と全 Range を印の文字列にする (`parse` の逆。主には `*` を付けない)。
pub fn render(text: &str, sel: &Selection) -> String {
    let mut marks: Vec<(usize, u8, &str)> = Vec::new();
    for r in sel.iter() {
        if r.is_empty() {
            marks.push((r.head.0, 1, "|"));
        } else if r.is_forward() {
            marks.push((r.start().0, 2, "«"));
            marks.push((r.end().0, 0, "»"));
        } else {
            marks.push((r.start().0, 2, "‹"));
            marks.push((r.end().0, 0, "›"));
        }
    }
    marks.sort_by_key(|&(p, order, _)| (p, order));
    let mut out = String::new();
    let mut at = 0;
    for (p, _, m) in marks {
        out.push_str(&text[at..p]);
        out.push_str(m);
        at = p;
    }
    out.push_str(&text[at..]);
    out
}

pub fn view(h: &Harness) -> ViewId {
    let st = h.editor.state();
    st.window_view(h.window()).expect("View がある")
}

pub fn selection(h: &Harness) -> Selection {
    h.editor
        .state()
        .view(view(h))
        .expect("View がある")
        .selection
        .clone()
}

pub fn set_selection(h: &mut Harness, ranges: Vec<Range>, primary: usize) {
    let v = view(h);
    let sel = Selection::new(ranges, primary).expect("Selection を作れる");
    h.editor.state_mut().set_selection(v, sel);
}

/// 無題の文書に本文を入れ、印の Range を置いた Harness。
pub fn scene(marked: &str) -> Harness {
    let (text, ranges, primary) = parse(marked);
    let mut h = Harness::new();
    if !text.is_empty() {
        h.paste(&text);
    }
    assert_eq!(h.doc_text(), text, "本文が入る");
    set_selection(&mut h, ranges, primary);
    h.clear_log();
    h
}

/// 本文と全 Range の今の様子。
pub fn shown(h: &Harness) -> String {
    render(&h.doc_text(), &selection(h))
}
