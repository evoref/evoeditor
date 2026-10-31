//! ソフトラップの移動とスクロール (docs/03 §5.2 / §5.4 / §5.5 / §14.3.1): 表示行単位の上下移動、Home / End と
//! affinity、ページ移動、カーソルへのスクロールの追従。

use eve_core::{Affinity, WrapMode, WrapSettings};
use eve_testkit::Harness;
use eve_view::Frame;

fn wrap_on(h: &mut Harness) {
    let v = view(h);
    h.editor.state_mut().set_wrap(
        v,
        WrapSettings {
            mode: WrapMode::Word,
            ..WrapSettings::default()
        },
    );
}

fn view(h: &Harness) -> eve_view::ViewId {
    let w = h.editor.state().first_window().unwrap();
    h.editor.state().window_view(w).unwrap()
}

/// 幅 20 (ガター 3 + 本文 17、折り返し幅 16)、高さ 8 の画面。
fn doc(body: &str) -> Harness {
    let mut h = Harness::new().with_size(20, 8);
    h.paste(body);
    h.keys("ctrl+home");
    wrap_on(&mut h);
    h
}

fn head(h: &Harness) -> usize {
    let v = view(h);
    h.editor.state().view(v).unwrap().selection.primary().head.0
}

fn affinity(h: &Harness) -> Affinity {
    let v = view(h);
    h.editor.state().view(v).unwrap().cursor_affinity
}

/// 主カーソルの (画面の行, 本文内の列)。
fn cursor(f: &Frame) -> (u16, u16) {
    let p = f.primary_cursor.expect("カーソルが見える").pos;
    (p.row, p.col - f.panes[0].gutter_width)
}

fn scroll(h: &Harness) -> (usize, u32) {
    let v = view(h);
    let s = h.editor.state().view(v).unwrap().scroll;
    (s.top.0, s.wrap_idx)
}

/// 1 行目は 2 表示行 ("aaaa bbbb cccc " / "dddd eeee ffff")、2 行目は短い、3 行目は 2 表示行。
const THREE: &str = "aaaa bbbb cccc dddd eeee ffff\nxy\nlonger line here ok then more";

// verifies: FR-506
#[test]
fn 下矢印は表示行を_1_つ動き_目標列を保つ() {
    let mut h = doc(THREE);
    h.keys("right right right right right right"); // "aaaa b|bbb"
    assert_eq!(cursor(&h.frame()), (0, 6));
    h.keys("down");
    assert_eq!(
        cursor(&h.frame()),
        (1, 6),
        "同じ論理行の次の表示行 'dddd e|eee'"
    );
    assert_eq!(head(&h), 15 + 6);
    h.keys("down");
    assert_eq!(cursor(&h.frame()), (2, 2), "短い行は行末に寄る");
    h.keys("down");
    assert_eq!(cursor(&h.frame()), (3, 6), "目標列 6 に戻る");
    h.keys("up up");
    assert_eq!(cursor(&h.frame()), (1, 6));
}

#[test]
fn 先頭の表示行で上は文書先頭_最後の表示行で下は文書末尾() {
    let mut h = doc(THREE);
    h.keys("right right down up");
    assert_eq!(head(&h), 2);
    h.keys("up");
    assert_eq!(head(&h), 0, "最初の表示行で上 → 文書先頭");
    h.keys("ctrl+end");
    let end = head(&h);
    h.keys("down");
    assert_eq!(head(&h), end);
    h.keys("up");
    assert!(head(&h) < end, "最後の論理行の最初の表示行へ上がる");
}

#[test]
fn end_はまず表示行末_次に論理行末_home_は表示行の先頭から論理行へ() {
    let mut h = doc("aaaa bbbb cccc dddd eeee ffff");
    h.keys("end");
    assert_eq!(head(&h), 15, "折り返し位置");
    assert_eq!(affinity(&h), Affinity::Before);
    assert_eq!(cursor(&h.frame()), (0, 15), "上の表示行の末尾に描く");
    h.keys("end");
    assert_eq!(head(&h), 29, "すでに表示行末 → 論理行末");
    assert_eq!(cursor(&h.frame()), (1, 14));
    h.keys("home");
    assert_eq!(head(&h), 15, "継続行ではまずその表示行の先頭");
    assert_eq!(affinity(&h), Affinity::After);
    assert_eq!(cursor(&h.frame()), (1, 0));
    h.keys("home");
    assert_eq!(
        head(&h),
        0,
        "表示行の先頭にいれば論理行の先頭 (スマートホーム)"
    );
}

#[test]
fn 折り返し位置から下へ動くと表示行末の_before_を引き継ぐ() {
    let mut h = doc("aaaa bbbb cccc dddd eeee ffff\nxyz");
    h.keys("end down");
    assert_eq!(cursor(&h.frame()).0, 1, "Before の列 15 から次の表示行へ");
    assert_eq!(affinity(&h), Affinity::After);
}

#[test]
fn 文字入力とクリック相当の選択で_affinity_は_after_に戻る() {
    let mut h = doc("aaaa bbbb cccc dddd eeee ffff");
    h.keys("end");
    assert_eq!(affinity(&h), Affinity::Before);
    h.keys("left");
    assert_eq!(affinity(&h), Affinity::After);
    h.keys("end");
    h.text("X");
    assert_eq!(affinity(&h), Affinity::After);
}

#[test]
fn 折り返し_off_の上下移動は論理行のまま() {
    let mut h = Harness::new().with_size(20, 8);
    h.paste(THREE);
    h.keys("ctrl+home down");
    assert_eq!(head(&h), 30, "2 行目の先頭");
}

/// 各行が 2 表示行になる 20 行の文書。
fn tall() -> Harness {
    let body: Vec<String> = (0..20)
        .map(|i| format!("line{i:02} aaaa bbbb cccc dddd"))
        .collect();
    doc(&body.join("\n"))
}

fn frame_rows(f: &Frame) -> Vec<String> {
    f.panes[0]
        .rows
        .iter()
        .map(|r| {
            r.spans
                .iter()
                .map(|s| s.text.as_str())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect()
}

#[test]
fn 下へ動き続けてもカーソルは見え_余白を保ってスクロールする() {
    let mut h = tall();
    let mut prev_top = (0, 0);
    for _ in 0..39 {
        h.keys("down");
        let f = h.frame();
        let (row, _) = cursor(&f);
        assert!((row as usize) < f.panes[0].rows.len());
        let top = scroll(&h);
        assert!(top >= prev_top, "下へ動くだけなので先頭は戻らない");
        prev_top = top;
    }
    let f = h.frame();
    let rows = frame_rows(&f);
    assert!(rows.iter().any(|r| r.contains("line19")), "{rows:?}");
    // 逆向きにも同様
    for _ in 0..39 {
        h.keys("up");
        let f = h.frame();
        assert!(f.primary_cursor.is_some());
    }
    assert_eq!(scroll(&h), (0, 0));
}

#[test]
fn 先頭が継続行のとき_その表示行から描く() {
    let mut h = tall();
    for _ in 0..13 {
        h.keys("down");
    }
    let (_, widx) = scroll(&h);
    let f = h.frame();
    let rows = frame_rows(&f);
    if widx > 0 {
        assert!(!rows[0].contains("line"), "継続行から始まる: {rows:?}");
    }
    assert!(f.primary_cursor.is_some());
}

#[test]
fn ページ移動は表示行で数え_カーソルの画面上の位置を保つ() {
    let mut h = tall();
    h.keys("down down");
    let before = cursor(&h.frame()).0;
    h.keys("pagedown");
    let f = h.frame();
    assert_eq!(cursor(&f).0, before, "画面上の相対位置を保つ");
    // 1 行は 26 文字 + 改行 = 27 バイトで、表示行は 17 バイト + 9 文字。
    // 2 表示行目 (行 1 の先頭) から 7 表示行下 = 行 4 の 2 本目の表示行
    assert_eq!(head(&h), 4 * 27 + 17);
    h.keys("pageup");
    assert_eq!(head(&h), 27);
    assert_eq!(scroll(&h), (0, 0));
}

#[test]
fn 文書末尾では最後の表示行が画面の最下行に来る() {
    let mut h = tall();
    h.keys("ctrl+end");
    let f = h.frame();
    let rows = frame_rows(&f);
    let last = rows.last().unwrap();
    assert!(
        last.contains("cccc dddd") || last.contains("dddd"),
        "{rows:?}"
    );
    assert!(rows.iter().any(|r| r.contains("line19")), "{rows:?}");
    assert!(f.primary_cursor.is_some());
}

#[test]
fn 指定行へ移動は遠くても中央に置く() {
    let mut h = tall();
    h.cmd_with(
        "cursor.goto_line",
        eve_core::Args::new().with_int("line", 15),
    );
    let f = h.frame();
    let rows = frame_rows(&f);
    assert!(rows.iter().any(|r| r.contains("line14")), "{rows:?}");
    assert!(f.primary_cursor.is_some());
}

#[test]
fn 編集で先頭の論理行が短くなっても_先頭の表示行の番号は収まる() {
    let mut h = tall();
    for _ in 0..13 {
        h.keys("down");
    }
    // 先頭の論理行を探して、その行を 1 表示行に縮める (行頭から 20 文字削る)
    let (top, widx) = scroll(&h);
    assert!(widx <= 1);
    let v = view(&h);
    let sel = eve_text::Selection::point(eve_text::ByteOff(top + 20));
    h.editor.state_mut().set_selection(v, sel);
    h.keys("shift+home delete");
    let f = h.frame();
    assert!(f.primary_cursor.is_some());
    let (_, w2) = scroll(&h);
    assert_eq!(w2, 0, "行が 1 表示行になったので先頭の表示行は 0");
}

#[test]
fn リサイズで先頭の表示行は行頭に戻る() {
    let mut h = tall();
    for _ in 0..13 {
        h.keys("down");
    }
    h.resize(30, 8);
    assert_eq!(scroll(&h).1, 0);
    assert!(h.frame().primary_cursor.is_some());
}
