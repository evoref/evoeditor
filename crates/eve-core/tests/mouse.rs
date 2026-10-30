//! マウス (docs/03 §5.8.1、01 FR-307): クリックで移動、Shift+クリックで拡張、ドラッグで選択、ホイール、自動スクロール。

use eve_core::{WrapMode, WrapSettings};
use eve_testkit::Harness;
use eve_view::{Modifiers, MouseButton, MouseKind};

const GW: i32 = 3; // 行数 2 桁 + 区切り

fn head(h: &Harness) -> usize {
    let w = h.editor.state().first_window().unwrap();
    let v = h.editor.state().window_view(w).unwrap();
    h.editor.state().view(v).unwrap().selection.primary().head.0
}

fn range(h: &Harness) -> (usize, usize) {
    let w = h.editor.state().first_window().unwrap();
    let v = h.editor.state().window_view(w).unwrap();
    let r = *h.editor.state().view(v).unwrap().selection.primary();
    (r.anchor.0, r.head.0)
}

fn scroll(h: &Harness) -> (usize, u32, u32) {
    let w = h.editor.state().first_window().unwrap();
    let v = h.editor.state().window_view(w).unwrap();
    let s = h.editor.state().view(v).unwrap().scroll;
    (s.top.0, s.left_col, s.wrap_idx)
}

fn left(h: &mut Harness, kind: fn(MouseButton) -> MouseKind, row: i32, col: i32) {
    h.mouse(kind(MouseButton::Left), Modifiers::NONE, row, col);
}

fn doc(body: &str) -> Harness {
    let mut h = Harness::new().with_size(30, 8);
    h.paste(body);
    h.keys("ctrl+home");
    h
}

#[test]
fn クリックでカーソルがその位置に移る() {
    let mut h = doc("hello\nworld\nあいう");
    h.click(1, GW + 2);
    assert_eq!(head(&h), 6 + 2);
    h.click(0, GW + 40);
    assert_eq!(head(&h), 5, "行末より右は行末");
    h.click(5, GW);
    assert_eq!(
        head(&h),
        "hello\nworld\nあいう".len(),
        "最終行より下は文書末尾"
    );
    h.click(1, 0);
    assert_eq!(head(&h), 6, "ガターは列 0");
}

#[test]
fn 全角のどちらの半分でもその文字の前に置く() {
    let mut h = doc("hello\nworld\nあいう");
    let base = "hello\nworld\n".len();
    h.click(2, GW);
    assert_eq!(head(&h), base);
    h.click(2, GW + 1);
    assert_eq!(head(&h), base, "右半分は文字の前 (OQ-58)");
    h.click(2, GW + 2);
    assert_eq!(head(&h), base + 3, "次の文字の前");
}

#[test]
fn shift_クリックは起点を保って選択を広げる() {
    let mut h = doc("hello\nworld");
    h.click(0, GW + 1);
    h.mouse(
        MouseKind::Down(MouseButton::Left),
        Modifiers::SHIFT,
        1,
        GW + 3,
    );
    assert_eq!(range(&h), (1, 9));
}

#[test]
fn ドラッグで選択し_離したら終わる() {
    let mut h = doc("hello\nworld");
    left(&mut h, MouseKind::Down, 0, GW + 1);
    left(&mut h, MouseKind::Drag, 1, GW + 3);
    assert_eq!(range(&h), (1, 9));
    left(&mut h, MouseKind::Drag, 0, GW + 3);
    assert_eq!(range(&h), (1, 3), "戻せば縮む");
    left(&mut h, MouseKind::Up, 0, GW + 3);
    left(&mut h, MouseKind::Drag, 1, GW + 4);
    assert_eq!(range(&h), (1, 3), "離した後のドラッグは何もしない");
}

#[test]
fn 中ボタンと右ボタンと_ctrl_付きは何もしない() {
    let mut h = doc("hello\nworld");
    h.mouse(MouseKind::Down(MouseButton::Right), Modifiers::NONE, 1, GW);
    h.mouse(MouseKind::Down(MouseButton::Middle), Modifiers::NONE, 1, GW);
    h.mouse(MouseKind::Down(MouseButton::Left), Modifiers::CTRL, 1, GW);
    assert_eq!(head(&h), 0);
}

fn long_doc() -> Harness {
    let body: Vec<String> = (0..30).map(|i| format!("line{i:02}")).collect();
    doc(&body.join("\n"))
}

#[test]
fn ホイールは_3_行スクロールし_カーソルは動かさない() {
    let mut h = long_doc();
    h.mouse(MouseKind::WheelDown, Modifiers::NONE, 2, GW);
    assert_eq!(scroll(&h).0, 3 * 7, "3 行下 (1 行 7 バイト)");
    assert_eq!(head(&h), 0);
    h.mouse(MouseKind::WheelUp, Modifiers::NONE, 2, GW);
    h.mouse(MouseKind::WheelUp, Modifiers::NONE, 2, GW);
    assert_eq!(scroll(&h).0, 0, "先頭より上へは行かない");
}

#[test]
fn shift_ホイールは横にスクロールする() {
    let mut h = doc(&"abcdefghij".repeat(10));
    h.mouse(MouseKind::WheelDown, Modifiers::SHIFT, 0, GW);
    assert_eq!(scroll(&h).1, 4);
    h.mouse(MouseKind::WheelUp, Modifiers::SHIFT, 0, GW);
    h.mouse(MouseKind::WheelUp, Modifiers::SHIFT, 0, GW);
    assert_eq!(scroll(&h).1, 0);
}

#[test]
fn 面の外へドラッグすると_時間ごとにスクロールして選択が伸びる() {
    let mut h = long_doc();
    left(&mut h, MouseKind::Down, 2, GW);
    left(&mut h, MouseKind::Drag, 9, GW); // 面は 8 行: 下にはみ出す
    assert!(h.editor.next_deadline().is_some(), "自動スクロールの期限");
    let before = range(&h);
    h.advance_ms(50);
    assert_eq!(scroll(&h).0, 7, "1 行スクロール");
    assert!(range(&h).1 > before.1, "選択が伸びる");
    h.advance_ms(50);
    assert_eq!(scroll(&h).0, 14);
    // 面の中へ戻せば止まる
    left(&mut h, MouseKind::Drag, 4, GW);
    h.advance_ms(200);
    assert_eq!(scroll(&h).0, 14);
}

#[test]
fn オーバーレイが開いている間はマウスを無視する() {
    let mut h = doc("hello\nworld");
    h.keys("ctrl+p");
    h.click(1, GW + 2);
    h.keys("esc");
    assert_eq!(head(&h), 0);
}

fn wrapped(body: &str) -> Harness {
    let mut h = doc(body);
    let w = h.editor.state().first_window().unwrap();
    let v = h.editor.state().window_view(w).unwrap();
    h.editor.state_mut().set_wrap(
        v,
        WrapSettings {
            mode: WrapMode::Word,
            ..WrapSettings::default()
        },
    );
    h
}

#[test]
fn 折り返し_on_では表示行の列で位置を引く() {
    // 幅 30: ガター 3 + 本文 27、折り返し幅 26
    let mut h = wrapped("aaaa bbbb cccc dddd eeee ffff gggg hhhh\nnext");
    // 1 行目: "aaaa bbbb cccc dddd eeee " / "ffff gggg hhhh"
    h.click(1, GW + 2);
    assert_eq!(head(&h), 25 + 2);
    h.click(2, GW + 1);
    assert_eq!(
        head(&h),
        "aaaa bbbb cccc dddd eeee ffff gggg hhhh\n".len() + 1
    );
    // 継続行の末尾より右: 文書の最後の表示行なので論理行末
    h.click(1, GW + 20);
    assert_eq!(head(&h), "aaaa bbbb cccc dddd eeee ffff gggg hhhh".len());
    // 折り返し位置で終わる表示行の右: 上の表示行の末尾にとどまる (次の表示行の先頭ではない)
    h.click(0, GW + 26);
    assert_eq!(head(&h), 25);
    let w = h.editor.state().first_window().unwrap();
    let v = h.editor.state().window_view(w).unwrap();
    assert_eq!(
        h.editor.state().view(v).unwrap().cursor_affinity,
        eve_core::Affinity::Before
    );
    let f = h.frame();
    let p = f.primary_cursor.unwrap().pos;
    assert_eq!(p.row, 0, "描画も上の表示行の末尾");
}
