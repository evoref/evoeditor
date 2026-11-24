//! ダブル / トリプルクリック (FR-311、docs/03 §5.8.2 の CLK-1〜CLK-7、01 §5.5 の FR-311 AC-1) のシナリオ (03 §15.9.1 の
//! L3。M34 の段 6)。回数は core が押下の時刻 (手動の時計) と位置で数える。文書と Range の印は `marked` (`multi_cursor.rs` と
//! 共有)。

mod marked;

use eve_core::{CommandResult, DOUBLE_CLICK_MS, WrapMode, WrapSettings};
use eve_testkit::Harness;
use eve_testkit::helpers::opened;
use eve_text::{ByteOff, Range, Selection};
use eve_view::{Answer, FocusTarget, Modifiers, MouseButton, MouseKind, Query, ViewId};
use marked::{render, scene, selection, set_selection, shown, view};

/// ガターの幅 (行数 2 桁 + 区切り)。
const GW: i32 = 3;

/// 左の押下 (修飾なし)。時計は進めない。
fn down(h: &mut Harness, row: i32, col: i32) {
    h.mouse(
        MouseKind::Down(MouseButton::Left),
        Modifiers::NONE,
        row,
        col,
    );
}

/// 左のドラッグ。
fn drag(h: &mut Harness, row: i32, col: i32) {
    h.mouse(
        MouseKind::Drag(MouseButton::Left),
        Modifiers::NONE,
        row,
        col,
    );
}

/// 左を離す。
fn up(h: &mut Harness, row: i32, col: i32) {
    h.mouse(MouseKind::Up(MouseButton::Left), Modifiers::NONE, row, col);
}

/// 左の押下と離す (時計は進めない。直前の押下と続けて数えられる)。
fn press(h: &mut Harness, row: i32, col: i32) {
    down(h, row, col);
    up(h, row, col);
}

/// 本文 `body` を入れ、カーソルを先頭に置いた Harness (`cols` × `rows`)。
fn doc_sized(body: &str, cols: u16, rows: u16) -> Harness {
    let mut h = Harness::new().with_size(cols, rows);
    h.paste(body);
    h.keys("ctrl+home");
    h.clear_log();
    h
}

/// 本文 `body` を入れ、カーソルを先頭に置いた Harness (80 × 24)。
fn doc(body: &str) -> Harness {
    doc_sized(body, 80, 24)
}

/// 幅 30 (ガター 3 + 本文 27、折り返し幅 26)・高さ 8 で、単語の折り返しを ON にした Harness。
fn wrapped(body: &str) -> Harness {
    let mut h = doc_sized(body, 30, 8);
    let v = view(&h);
    h.editor.state_mut().set_wrap(
        v,
        WrapSettings {
            mode: WrapMode::Word,
            ..WrapSettings::default()
        },
    );
    h
}

/// View `v` の本文と全 Range の様子。
fn shown_in(h: &Harness, v: ViewId) -> String {
    let sel = h.editor.state().view(v).expect("View").selection.clone();
    render(&h.doc_text(), &sel)
}

/// ペインを木の葉の順に、表示中の View で並べたものと、フォーカスのペインの View (`Query::Windows`)。
fn panes(h: &Harness) -> (Vec<ViewId>, ViewId) {
    let Ok(Answer::Windows(ws)) = h.query(&Query::Windows) else {
        panic!("Window の一覧");
    };
    let w = &ws[0];
    let FocusTarget::Pane(f) = w.focus else {
        panic!("フォーカスはペイン: {:?}", w.focus);
    };
    (w.views.clone(), f)
}

/// View `v` のペインへの左の押下と離す (時計は進めない)。
fn press_in(h: &mut Harness, v: ViewId, row: i32, col: i32) {
    h.mouse_in(
        v,
        MouseKind::Down(MouseButton::Left),
        Modifiers::NONE,
        row,
        col,
    );
    h.mouse_in(
        v,
        MouseKind::Up(MouseButton::Left),
        Modifiers::NONE,
        row,
        col,
    );
}

/// 左右に分けた同じ文書の 2 つのペイン (左, 右)。フォーカスは右。
fn split(body: &str) -> (Harness, ViewId, ViewId) {
    let mut h = doc(body);
    let left = panes(&h).1;
    assert_eq!(h.cmd("pane.split_right"), CommandResult::Done);
    let right = panes(&h).1;
    assert_ne!(left, right);
    (h, left, right)
}

// ---------- 01 FR-311 AC-1 ----------

// verifies: AC-FR-311-1
// spec: 03 §5.8.2 (CLK-1, CLK-2, CLK-3)
#[test]
fn 間隔の内に同じ位置を_2_回押すと単語を_3_回押すと行の全体を選ぶ() {
    let mut h = doc("foo bar baz\nqux\n");
    h.double_click(0, GW + 5);
    assert_eq!(shown(&h), "foo «bar» baz\nqux\n");
    h.triple_click(0, GW + 5);
    assert_eq!(shown(&h), "«foo bar baz\n»qux\n");
    // 手動の時計で間隔を作る: 押下の間に 300 ms
    h.click(1, GW + 1);
    h.advance_ms(300);
    press(&mut h, 1, GW + 1);
    assert_eq!(shown(&h), "foo bar baz\n«qux»\n");
    h.advance_ms(300);
    press(&mut h, 1, GW + 1);
    assert_eq!(shown(&h), "foo bar baz\n«qux\n»");
    assert!(h.messages().is_empty(), "知らせない: {:?}", h.messages());
}

// verifies: AC-FR-311-1
// spec: 03 §5.8.2 (CLK-4)
#[test]
fn 続けてドラッグすると単語_行の単位で選択が伸びる() {
    let body = "foo bar baz qux\nline two\nline three\n";
    // 2 回目の押下のまま、qux の途中へ
    let mut h = doc(body);
    h.click(0, GW + 5);
    down(&mut h, 0, GW + 5);
    drag(&mut h, 0, GW + 13);
    assert_eq!(shown(&h), "foo «bar baz qux»\nline two\nline three\n");
    up(&mut h, 0, GW + 13);
    // 3 回目の押下のまま、下の行へ
    h.click(1, GW + 2);
    press(&mut h, 1, GW + 2);
    down(&mut h, 1, GW + 2);
    drag(&mut h, 2, GW + 1);
    assert_eq!(shown(&h), "foo bar baz qux\n«line two\nline three\n»");
    up(&mut h, 2, GW + 1);
}

// verifies: AC-FR-311-1
// spec: 03 §5.8.2 (CLK-1)
#[test]
fn 間にほかの入力をはさむ_位置が違う_間隔が空くと_1_回目として扱う() {
    // ほかの入力 (キー)
    let mut h = doc("foo bar\n");
    h.click(0, GW + 5);
    h.keys("right");
    h.keys("left");
    press(&mut h, 0, GW + 5);
    assert_eq!(shown(&h), "foo b|ar\n");
    // 位置が違う
    h.click(0, GW + 5);
    press(&mut h, 0, GW + 1);
    assert_eq!(shown(&h), "f|oo bar\n");
    // 間隔が空く
    h.click(0, GW + 5);
    h.advance_ms(DOUBLE_CLICK_MS.0 + 1);
    press(&mut h, 0, GW + 5);
    assert_eq!(shown(&h), "foo b|ar\n");
}

// ---------- CLK-1: 数え方 ----------

/// 押下の間を `gap` ms 空けて 2 回押した後の様子。
fn after_gap(gap: u64) -> String {
    let mut h = doc("foo bar\n");
    h.click(0, GW + 5);
    h.advance_ms(gap);
    press(&mut h, 0, GW + 5);
    shown(&h)
}

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-1)
#[test]
fn 間隔は_500_ms_ちょうどまで続けて数え_501_ms_で数え直す() {
    assert_eq!(DOUBLE_CLICK_MS.0, 500);
    assert_eq!(after_gap(0), "foo «bar»\n");
    assert_eq!(after_gap(499), "foo «bar»\n");
    assert_eq!(after_gap(500), "foo «bar»\n", "ちょうど 500 ms は続ける");
    assert_eq!(after_gap(501), "foo b|ar\n", "501 ms は 1 回目");
}

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-1)
#[test]
fn 間隔は直前の押下から測る() {
    // 400 ms ずつ 3 回: 1 回目から 800 ms でも、直前から 400 ms なので 3 回目
    let mut h = doc("foo bar\n");
    h.click(0, GW + 5);
    h.advance_ms(400);
    press(&mut h, 0, GW + 5);
    h.advance_ms(400);
    press(&mut h, 0, GW + 5);
    assert_eq!(shown(&h), "«foo bar\n»");
    // 2 回目から 501 ms 空くと 1 回目
    h.advance_ms(501);
    press(&mut h, 0, GW + 5);
    assert_eq!(shown(&h), "foo b|ar\n");
}

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-1)
#[test]
fn 別の表示行と離れたセルは数え直す() {
    let mut h = doc("foo bar\nfoo bar\n");
    // 別の行の同じ列
    h.click(0, GW + 5);
    press(&mut h, 1, GW + 5);
    assert_eq!(shown(&h), "foo bar\nfoo b|ar\n");
    // 同じ行の隣のセル (別の位置)
    h.click(0, GW + 5);
    press(&mut h, 0, GW + 6);
    assert_eq!(shown(&h), "foo ba|r\nfoo bar\n");
    // 離れたセルから戻っても、直前の押下と比べるので 1 回目
    press(&mut h, 0, GW + 5);
    assert_eq!(shown(&h), "foo b|ar\nfoo bar\n");
}

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-1, CLK-2)
#[test]
fn 行末より右の別のセルでも位置が同じなら続けて数え_直前の書記素の単位を選ぶ() {
    let mut h = doc("foo bar\nx\n");
    h.click(0, GW + 20);
    assert_eq!(shown(&h), "foo bar|\nx\n");
    press(&mut h, 0, GW + 30);
    assert_eq!(shown(&h), "foo «bar»\nx\n", "行末は直前の書記素の単位");
    press(&mut h, 0, GW + 9);
    assert_eq!(shown(&h), "«foo bar\n»x\n");
}

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-1)
#[test]
fn 全角の左右の半分は同じ位置なので続けて数える() {
    let mut h = doc("日本語のテキスト\n");
    h.click(0, GW + 2);
    press(&mut h, 0, GW + 3);
    assert_eq!(shown(&h), "«日本語»のテキスト\n", "本 の左半分と右半分");
}

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-1)
#[test]
fn キー_文字_貼り付け_ホイール_ほかのボタン_修飾付きの押下は数え直す() {
    type Between = fn(&mut Harness);
    let cases: [(&str, Between); 9] = [
        ("キー", |h| h.keys("ctrl+home")),
        ("文字", |h| h.text("")),
        ("貼り付け", |h| h.paste("")),
        ("ホイール", |h| {
            h.mouse(MouseKind::WheelDown, Modifiers::NONE, 0, GW + 5);
        }),
        ("右のボタン", |h| {
            h.mouse(
                MouseKind::Down(MouseButton::Right),
                Modifiers::NONE,
                0,
                GW + 5,
            );
            h.mouse(
                MouseKind::Up(MouseButton::Right),
                Modifiers::NONE,
                0,
                GW + 5,
            );
        }),
        ("中のボタン", |h| {
            h.mouse(
                MouseKind::Down(MouseButton::Middle),
                Modifiers::NONE,
                0,
                GW + 5,
            );
        }),
        ("Ctrl 付きの押下", |h| {
            h.mouse(
                MouseKind::Down(MouseButton::Left),
                Modifiers::CTRL,
                0,
                GW + 5,
            );
        }),
        ("Alt 付きの押下", |h| {
            h.mouse(
                MouseKind::Down(MouseButton::Left),
                Modifiers::ALT,
                0,
                GW + 5,
            );
        }),
        ("コマンド", |h| {
            assert_eq!(h.cmd("view.toggle_wrap"), CommandResult::Done);
            assert_eq!(h.cmd("view.toggle_wrap"), CommandResult::Done);
        }),
    ];
    for (what, between) in cases {
        let mut h = doc("foo bar\n");
        h.click(0, GW + 5);
        between(&mut h);
        press(&mut h, 0, GW + 5);
        assert_eq!(shown(&h), "foo b|ar\n", "{what} の後は 1 回目");
        press(&mut h, 0, GW + 5);
        assert_eq!(shown(&h), "foo «bar»\n", "{what} の後の 2 回目");
    }
}

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-1)
#[test]
fn shift_付きの押下は数えず_数え直す() {
    let mut h = doc("foo bar\n");
    h.click(0, GW + 5);
    // `Shift` 付きの押下は 2 回目に数えない (単語を選ばず、今の選択を伸ばす)
    h.mouse(
        MouseKind::Down(MouseButton::Left),
        Modifiers::SHIFT,
        0,
        GW + 5,
    );
    h.mouse(
        MouseKind::Up(MouseButton::Left),
        Modifiers::SHIFT,
        0,
        GW + 5,
    );
    assert_eq!(shown(&h), "foo b|ar\n");
    // 続く修飾なしの押下は 1 回目
    press(&mut h, 0, GW + 5);
    assert_eq!(shown(&h), "foo b|ar\n");
    press(&mut h, 0, GW + 5);
    assert_eq!(shown(&h), "foo «bar»\n");
}

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-1)
#[test]
fn ドラッグと離すは回数を変えない() {
    let mut h = doc("foo bar baz\n");
    h.click(0, GW + 5);
    down(&mut h, 0, GW + 5);
    // 2 回目の押下からドラッグして離す
    drag(&mut h, 0, GW + 9);
    up(&mut h, 0, GW + 9);
    assert_eq!(shown(&h), "foo «bar baz»\n");
    // 同じ位置の 3 回目は行
    press(&mut h, 0, GW + 5);
    assert_eq!(shown(&h), "«foo bar baz\n»");
}

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-1)
#[test]
fn オーバーレイと検索バーの開閉は数え直す() {
    let cases: [(&str, &str); 2] = [("ui.palette_open", "パレット"), ("search.open", "検索バー")];
    for (id, what) in cases {
        let mut h = doc("foo bar\n");
        h.click(0, GW + 5);
        h.cmd(id);
        // 開いている間の押下は無視する
        press(&mut h, 0, GW + 5);
        assert_eq!(shown(&h), "foo b|ar\n", "{what} の間");
        h.keys("escape");
        press(&mut h, 0, GW + 5);
        assert_eq!(shown(&h), "foo b|ar\n", "{what} を閉じた後は 1 回目");
        press(&mut h, 0, GW + 5);
        assert_eq!(shown(&h), "foo «bar»\n", "{what} を閉じた後の 2 回目");
    }
}

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-1, CLK-5), 03 §9.9 (PANE-7)
#[test]
fn 別の_view_は数え直し_押下によるフォーカスの移動は数え直さない() {
    let (mut h, left, right) = split("foo bar\n");
    // 右 (フォーカス) を押してから、左の同じ行と列を押す: 別の View なので 1 回目。押下でフォーカスが左へ移る
    h.advance_ms(DOUBLE_CLICK_MS.0 + 1);
    press_in(&mut h, right, 0, GW + 5);
    press_in(&mut h, left, 0, GW + 5);
    assert_eq!(panes(&h).1, left);
    assert_eq!(shown_in(&h, left), "foo b|ar\n", "別の View は 1 回目");
    // 押下によるフォーカスの移動 (1 回目) に続く 2 回目は単語
    press_in(&mut h, left, 0, GW + 5);
    assert_eq!(shown_in(&h, left), "foo «bar»\n");
    // フォーカスの無いペインの 1 回目でフォーカスが移り、2 回目で単語・3 回目で行 (CLK-5)
    h.advance_ms(DOUBLE_CLICK_MS.0 + 1);
    press_in(&mut h, right, 0, GW + 1);
    assert_eq!(panes(&h).1, right);
    assert_eq!(shown_in(&h, right), "f|oo bar\n");
    press_in(&mut h, right, 0, GW + 1);
    assert_eq!(shown_in(&h, right), "«foo» bar\n");
    press_in(&mut h, right, 0, GW + 1);
    assert_eq!(shown_in(&h, right), "«foo bar\n»");
    assert_eq!(shown_in(&h, left), "foo «bar»\n", "左の選択は変えない");
}

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-1)
#[test]
fn 押下によらないペインのフォーカスの移動と見出しの押下は数え直す() {
    let (mut h, left, right) = split("foo bar\n");
    // `pane.focus_*` で離れて戻る
    h.advance_ms(DOUBLE_CLICK_MS.0 + 1);
    press_in(&mut h, right, 0, GW + 5);
    h.cmd("pane.focus_next");
    h.cmd("pane.focus_next");
    assert_eq!(panes(&h).1, right);
    press_in(&mut h, right, 0, GW + 5);
    assert_eq!(
        shown_in(&h, right),
        "foo b|ar\n",
        "pane.focus_* の後は 1 回目"
    );
    // ペインの見出しの押下 (ChromeMouse) をはさむ
    press_in(&mut h, right, 0, GW + 5);
    assert_eq!(shown_in(&h, right), "foo «bar»\n");
    h.advance_ms(DOUBLE_CLICK_MS.0 + 1);
    press_in(&mut h, left, 0, GW + 5);
    h.pane_bar_mouse(left, MouseKind::Down(MouseButton::Left), Modifiers::NONE, 1);
    press_in(&mut h, left, 0, GW + 5);
    assert_eq!(
        shown_in(&h, left),
        "foo b|ar\n",
        "見出しの押下の後は 1 回目"
    );
}

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-1)
#[test]
fn 自動スクロールで同じセルが別の位置を指すと数え直す() {
    // 30 行の文書を 8 行の面に。各行 "wNN xx" (7 バイト)
    let body: Vec<String> = (0..30).map(|i| format!("w{i:02} xx")).collect();
    let mut h = doc_sized(&body.join("\n"), 30, 8);
    h.click(7, GW + 1);
    // 同じセルを押したまま面の下へ出し、1 行スクロールさせてから離す
    down(&mut h, 7, GW + 1);
    drag(&mut h, 9, GW + 1);
    h.advance_ms(50);
    up(&mut h, 9, GW + 1);
    // 同じセル (7 行目、1 列目) は 1 行下の文書の行を指す: 位置が違うので 1 回目
    press(&mut h, 7, GW + 1);
    assert_eq!(h.selection_range(), (8 * 7 + 1, 8 * 7 + 1));
    press(&mut h, 7, GW + 1);
    assert_eq!(
        h.selection_range(),
        (8 * 7, 8 * 7 + 3),
        "続けて押せば 2 回目"
    );
}

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-1)
#[test]
fn 大きさの変化と端末のフォーカスの変化は数え直す() {
    type Between = fn(&mut Harness);
    let cases: [(&str, Between); 3] = [
        ("大きさの変化", |h| h.resize(80, 24)),
        ("フォーカスを失う", |h| h.focus(false)),
        ("フォーカスを得る", |h| h.focus(true)),
    ];
    for (what, between) in cases {
        let mut h = doc("foo bar\n");
        h.click(0, GW + 5);
        between(&mut h);
        press(&mut h, 0, GW + 5);
        assert_eq!(shown(&h), "foo b|ar\n", "{what} の後は 1 回目");
        press(&mut h, 0, GW + 5);
        assert_eq!(shown(&h), "foo «bar»\n", "{what} の後の 2 回目");
    }
}

// ---------- CLK-2: 2 回 (単位) ----------

/// `body` の (行, 列) をダブルクリックした様子。
fn double_at(body: &str, row: i32, col: i32) -> String {
    let mut h = doc(body);
    h.double_click(row, col);
    shown(&h)
}

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-2), 03 §13.3
#[test]
fn ダブルクリックは位置の直後の書記素の単位を前向きに選ぶ() {
    // 単語・空白のラン・記号 1 つ
    assert_eq!(double_at("foo_bar  x+=1\n", 0, GW + 2), "«foo_bar»  x+=1\n");
    assert_eq!(double_at("foo_bar  x+=1\n", 0, GW + 8), "foo_bar«  »x+=1\n");
    assert_eq!(
        double_at("foo_bar  x+=1\n", 0, GW + 10),
        "foo_bar  x«+»=1\n"
    );
    // 単語の先頭 (直後の書記素の単位)
    assert_eq!(double_at("foo bar\n", 0, GW + 4), "foo «bar»\n");
    // 文字種の境 (漢字・かな・カタカナ・英数)
    assert_eq!(
        double_at("日本語のテキストabc\n", 0, GW + 6),
        "日本語«の»テキストabc\n"
    );
    assert_eq!(
        double_at("日本語のテキストabc\n", 0, GW + 9),
        "日本語の«テキスト»abc\n",
        "全角の右半分はその文字の前"
    );
    assert_eq!(
        double_at("日本語のテキストabc\n", 0, GW + 16),
        "日本語のテキスト«abc»\n"
    );
}

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-2)
#[test]
fn タブの上は空白のランを選ぶ() {
    // `a` は列 0、タブは 1〜3 と 4〜7、`b` は 8 (タブ幅 4)
    for col in [1, 2, 3, 4, 5, 7] {
        assert_eq!(double_at("a\t\tb\n", 0, GW + col), "a«\t\t»b\n", "列 {col}");
    }
    assert_eq!(double_at("a\t\tb\n", 0, GW + 8), "a\t\t«b»\n");
}

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-2)
#[test]
fn 行末より右は直前の書記素の単位で_空の行と文書の末尾の空の行では何も選ばない() {
    assert_eq!(double_at("foo bar\n", 0, GW + 40), "foo «bar»\n");
    assert_eq!(
        double_at("foo \n", 0, GW + 40),
        "foo« »\n",
        "空白で終わる行"
    );
    assert_eq!(double_at("foo\n\nbar", 1, GW + 3), "foo\n|\nbar", "空の行");
    assert_eq!(double_at("foo\n", 1, GW), "foo\n|", "文書の末尾の空の行");
    assert_eq!(
        double_at("foo\n", 5, GW),
        "foo\n|",
        "最終行より下は文書の末尾"
    );
}

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-2, CLK-3), 03 §14.3.1
#[test]
fn 折り返し_on_では表示行の位置の単位と論理行の全体を選ぶ() {
    // 1 行目の表示行: "aaaa bbbb cccc dddd eeee " / "ffff gggg hhhh"
    let body = "aaaa bbbb cccc dddd eeee ffff gggg hhhh\nnext";
    let mut h = wrapped(body);
    h.double_click(1, GW + 6);
    assert_eq!(shown(&h), "aaaa bbbb cccc dddd eeee ffff «gggg» hhhh\nnext");
    // 継続行の末尾より右 (論理行の行末): 直前の書記素の単位
    h.double_click(1, GW + 20);
    assert_eq!(shown(&h), "aaaa bbbb cccc dddd eeee ffff gggg «hhhh»\nnext");
    // 継続行のトリプルクリックも論理行の全体
    h.triple_click(1, GW + 1);
    assert_eq!(shown(&h), "«aaaa bbbb cccc dddd eeee ffff gggg hhhh\n»next");
}

/// View の今のカーソルの帰属と、Frame の主カーソルの表示行。
fn cursor_place(h: &mut Harness) -> (eve_core::Affinity, u16) {
    let v = view(h);
    let aff = h.editor.state().view(v).expect("View").cursor_affinity;
    let row = h.frame().primary_cursor.expect("主カーソル").pos.row;
    (aff, row)
}

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-2, CLK-4), 03 §14.3
#[test]
fn 折り返しの右の余白は上の表示行の末尾の単位で_次の表示行の単語を選ばない() {
    // 1 行目の表示行: "aaaa bbbb cccc dddd eeee " / "ffff gggg hhhh"。0 行目の右の余白は折り返し位置 (Before)
    let body = "aaaa bbbb cccc dddd eeee ffff gggg hhhh\nnext";
    let mut h = wrapped(body);
    h.double_click(0, GW + 26);
    assert_eq!(
        shown(&h),
        "aaaa bbbb cccc dddd eeee« »ffff gggg hhhh\nnext",
        "上の表示行の最後の書記素 (空白) の単位"
    );
    assert_eq!(
        cursor_place(&mut h),
        (eve_core::Affinity::Before, 0),
        "前向きの head は上の表示行の末尾"
    );
    // 単位のドラッグで右の余白へ: 次の表示行の ffff まで伸ばさない
    let mut h = wrapped(body);
    h.click(0, GW + 16);
    down(&mut h, 0, GW + 16);
    drag(&mut h, 0, GW + 28);
    assert_eq!(shown(&h), "aaaa bbbb cccc «dddd eeee »ffff gggg hhhh\nnext");
    assert_eq!(cursor_place(&mut h), (eve_core::Affinity::Before, 0));
    up(&mut h, 0, GW + 28);
    // 次の表示行の先頭のセルは ffff (After)
    h.double_click(1, GW);
    assert_eq!(shown(&h), "aaaa bbbb cccc dddd eeee «ffff» gggg hhhh\nnext");
}

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-4), 03 §14.3
#[test]
fn 単語の後ろ向きの_head_が折り返し位置なら下の表示行の先頭() {
    let body = "aaaa bbbb cccc dddd eeee ffff gggg hhhh\nnext";
    let mut h = wrapped(body);
    // gggg で 2 回押して、同じ表示行の ffff へ戻す: head は ffff の始まり (折り返し位置) で後ろ向き
    h.click(1, GW + 6);
    down(&mut h, 1, GW + 6);
    drag(&mut h, 1, GW + 1);
    assert_eq!(shown(&h), "aaaa bbbb cccc dddd eeee ‹ffff gggg› hhhh\nnext");
    assert_eq!(
        cursor_place(&mut h),
        (eve_core::Affinity::After, 1),
        "後ろ向きの head は下の表示行の先頭"
    );
    up(&mut h, 1, GW + 1);
    // 行の単位の head は After のまま
    h.triple_click(0, GW + 1);
    assert_eq!(cursor_place(&mut h).0, eve_core::Affinity::After);
}

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-2, CLK-3)
#[test]
fn crlf_の文書は行の全体が区切りまでで_行末より右は_cr_を選ばない() {
    let mut h = opened("/w/crlf.txt", b"foo bar\r\nbaz\r\n");
    h.triple_click(0, GW + 1);
    assert_eq!(shown(&h), "«foo bar\r\n»baz\r\n");
    h.double_click(0, GW + 20);
    assert_eq!(shown(&h), "foo «bar»\r\nbaz\r\n");
    h.double_click(1, GW + 20);
    assert_eq!(shown(&h), "foo bar\r\n«baz»\r\n");
    h.triple_click(2, GW);
    assert_eq!(
        shown(&h),
        "foo bar\r\nbaz\r\n|",
        "区切りで終わる文書の最後の空の行"
    );
}

// ---------- CLK-3: 3 回 (行の全体) と 4 回目より後 ----------

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-3)
#[test]
fn トリプルクリックは行の全体を選び_最後の行は文書の末尾まで_空の行は区切りまで() {
    let mut h = doc("ab\n\ncd");
    h.triple_click(0, GW + 1);
    assert_eq!(shown(&h), "«ab\n»\ncd");
    h.triple_click(1, GW + 5);
    assert_eq!(shown(&h), "ab\n«\n»cd", "空の行は区切りまで");
    h.triple_click(2, GW + 9);
    assert_eq!(shown(&h), "ab\n\n«cd»", "最後の行は文書の末尾まで");
    // 区切りで終わる文書の最後の空の行は空
    let mut h = doc("ab\n");
    h.triple_click(1, GW);
    assert_eq!(shown(&h), "ab\n|");
}

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-3)
#[test]
fn 四回目より後も行の全体を選び直す() {
    let mut h = doc("foo bar\nbaz\n");
    h.triple_click(0, GW + 5);
    for n in 4..=7 {
        press(&mut h, 0, GW + 5);
        assert_eq!(shown(&h), "«foo bar\n»baz\n", "{n} 回目");
    }
    // 4 回目より後のドラッグも行の単位
    down(&mut h, 0, GW + 5);
    drag(&mut h, 1, GW + 1);
    assert_eq!(shown(&h), "«foo bar\nbaz\n»");
    up(&mut h, 1, GW + 1);
    // 間隔が空けば 1 回目に戻る
    h.advance_ms(DOUBLE_CLICK_MS.0 + 1);
    press(&mut h, 0, GW + 5);
    assert_eq!(shown(&h), "foo b|ar\nbaz\n");
}

// ---------- CLK-4: 単位のドラッグ ----------

/// `body` の (行, 列) で `n` 回目の押下をして、そのまま `to` の位置へドラッグした様子。
fn unit_drag(body: &str, n: usize, at: (i32, i32), to: (i32, i32)) -> String {
    let mut h = doc(body);
    h.advance_ms(DOUBLE_CLICK_MS.0 + 1);
    for _ in 1..n {
        press(&mut h, at.0, at.1);
    }
    down(&mut h, at.0, at.1);
    drag(&mut h, to.0, to.1);
    up(&mut h, to.0, to.1);
    shown(&h)
}

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-4)
#[test]
fn 単語のドラッグは起点の単語を含めて位置の単語の端まで選び_head_は位置の側() {
    let body = "foo bar baz qux\n";
    assert_eq!(
        unit_drag(body, 2, (0, GW + 5), (0, GW + 9)),
        "foo «bar baz» qux\n",
        "後ろへ: 位置の単語の終わりまで前向き"
    );
    assert_eq!(
        unit_drag(body, 2, (0, GW + 9), (0, GW + 1)),
        "‹foo bar baz› qux\n",
        "前へ: 位置の単語の始まりまで後ろ向き (起点の単語を含む)"
    );
    assert_eq!(
        unit_drag(body, 2, (0, GW + 9), (0, GW + 10)),
        "foo bar «baz» qux\n",
        "起点の単語の中"
    );
    assert_eq!(
        unit_drag(body, 2, (0, GW + 5), (0, GW + 7)),
        "foo «bar »baz qux\n",
        "単語の後ろの空白のセル: 空白のランまで"
    );
    assert_eq!(
        unit_drag(body, 2, (0, GW + 5), (0, GW + 40)),
        "foo «bar baz qux»\n",
        "行末より右: 直前の書記素の単位"
    );
}

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-4)
#[test]
fn 単語のドラッグは行をまたぎ_逆向きでも起点の単語を含む() {
    let body = "one two\nthree four\nfive six\n";
    assert_eq!(
        unit_drag(body, 2, (1, GW + 7), (2, GW + 1)),
        "one two\nthree «four\nfive» six\n"
    );
    assert_eq!(
        unit_drag(body, 2, (1, GW + 1), (0, GW + 5)),
        "one ‹two\nthree› four\nfive six\n",
        "逆向きでも起点の単語 (three) の終わりが anchor"
    );
    // 空の行へ・空の行から
    let body = "foo\n\nbar\n";
    assert_eq!(
        unit_drag(body, 2, (0, GW + 1), (1, GW + 3)),
        "«foo\n»\nbar\n",
        "空の行の位置の単位は空"
    );
    assert_eq!(
        unit_drag(body, 2, (1, GW), (2, GW + 1)),
        "foo\n«\nbar»\n",
        "空の行で 2 回押してから後ろへ"
    );
}

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-4)
#[test]
fn 行のドラッグは起点の行を含めて位置の行の全体を選ぶ() {
    let body = "l0\nl1\nl2\nl3";
    assert_eq!(
        unit_drag(body, 3, (1, GW + 1), (3, GW)),
        "l0\n«l1\nl2\nl3»",
        "後ろへ: 最後の行は文書の末尾まで"
    );
    assert_eq!(
        unit_drag(body, 3, (2, GW + 1), (0, GW + 1)),
        "‹l0\nl1\nl2\n›l3",
        "前へ: 起点の行の区切りの後ろが anchor"
    );
    assert_eq!(
        unit_drag(body, 3, (1, GW + 1), (1, GW + 9)),
        "l0\n«l1\n»l2\nl3",
        "同じ行の中"
    );
}

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-4)
#[test]
fn 単位のドラッグは全角と折り返し_on_でも単位の端に止まる() {
    // 全角: 「日本語」から「テキスト」の右半分へ
    assert_eq!(
        unit_drag("日本語のテキスト\n", 2, (0, GW + 1), (0, GW + 11)),
        "«日本語のテキスト»\n"
    );
    // 折り返し ON: 1 行目の bbbb から継続行の gggg へ
    let body = "aaaa bbbb cccc dddd eeee ffff gggg hhhh\nnext";
    let mut h = wrapped(body);
    h.click(0, GW + 6);
    down(&mut h, 0, GW + 6);
    drag(&mut h, 1, GW + 6);
    assert_eq!(shown(&h), "aaaa «bbbb cccc dddd eeee ffff gggg» hhhh\nnext");
    up(&mut h, 1, GW + 6);
}

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-4), 03 §5.8.1
#[test]
fn 面の外の自動スクロールも単位で伸ばす() {
    // 30 行の文書を 8 行の面に。各行 "wNN xx" (7 バイト)
    let body: Vec<String> = (0..30).map(|i| format!("w{i:02} xx")).collect();
    let body = body.join("\n");
    let mut h = doc_sized(&body, 30, 8);
    h.click(2, GW + 5);
    down(&mut h, 2, GW + 5);
    assert_eq!(h.selection_range(), (2 * 7 + 4, 2 * 7 + 6));
    // 面の下の外へ (面は 8 行)。位置は最下行の同じ列 → その行の xx の終わりまで
    drag(&mut h, 9, GW + 5);
    assert_eq!(h.selection_range(), (2 * 7 + 4, 7 * 7 + 6));
    h.advance_ms(50);
    assert_eq!(
        h.selection_range(),
        (2 * 7 + 4, 8 * 7 + 6),
        "1 行スクロールして次の行の単位の終わりまで"
    );
    // 行の単位
    let mut h = doc_sized(&body, 30, 8);
    h.click(2, GW + 1);
    press(&mut h, 2, GW + 1);
    down(&mut h, 2, GW + 1);
    drag(&mut h, 9, GW + 1);
    h.advance_ms(50);
    assert_eq!(
        h.selection_range(),
        (2 * 7, 9 * 7),
        "スクロールした最下行 (8 行目) の全体まで"
    );
}

// ---------- CLK-5: Range を 1 つにする ----------

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-5), 03 §4.5.2
#[test]
fn どの回数の押下も_range_を_1_つにする() {
    for n in 1..=3 {
        let mut h = scene("|foo bar\n|baz qux\n|end");
        h.click(1, GW + 5);
        for _ in 1..n {
            press(&mut h, 1, GW + 5);
        }
        let sel = selection(&h);
        assert_eq!(sel.len(), 1, "{n} 回: {}", shown(&h));
    }
    // 主でない Range があっても単語は押した位置のもの
    let mut h = scene("|foo bar\n|baz qux\n|end");
    h.double_click(1, GW + 5);
    assert_eq!(shown(&h), "foo bar\nbaz «qux»\nend");
}

// ---------- CLK-6: Motion ----------

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-6)
#[test]
fn 文書を変えず_履歴に積まず_読み取り専用でも使える() {
    // Undo の結合の封印は、どの回数の押下も 1 回目の押下と同じ経路 (`mouse_down` の `seal_active_history`) で行う。2 回目
    // より後の押下の前には必ず 1 回目の押下 (単クリックと同じ) があるので、ここでは履歴に何も積まないことを確かめる
    let mut h = Harness::new();
    h.text("ab");
    h.double_click(0, GW + 9);
    assert_eq!(shown(&h), "«ab»");
    h.triple_click(0, GW + 1);
    assert_eq!(shown(&h), "«ab»");
    assert_eq!(h.doc_text(), "ab");
    h.cmd("edit.undo");
    assert_eq!(
        h.doc_text(),
        "",
        "押下は履歴のノードを作らない (Undo 1 回で入力が戻る)"
    );
    h.cmd("edit.redo");
    assert_eq!(h.doc_text(), "ab");
    // 読み取り専用の文書
    let mut h = opened("/w/ro.txt", b"foo bar\nbaz");
    assert_eq!(h.cmd("file.toggle_read_only"), CommandResult::Done);
    h.double_click(0, GW + 5);
    assert_eq!(shown(&h), "foo «bar»\nbaz");
    h.triple_click(1, GW + 1);
    assert_eq!(shown(&h), "foo bar\n«baz»");
    assert!(!h.is_dirty());
    assert_eq!(h.doc_text(), "foo bar\nbaz");
}

// ---------- CLK-7: 費用 ----------

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-7), 03 §4.5.1
#[test]
fn 長大行の単語は窓_64_kib_までで行の全体を読まない() {
    let n = 300 * 1024;
    let body = "a".repeat(n);
    let mut h = doc(&body);
    h.double_click(0, GW + 5);
    let sel = selection(&h);
    let r = *sel.primary();
    assert!(r.is_forward(), "前向き");
    assert!(r.start().0 <= 5 && r.end().0 > 5, "押した位置を含む: {r:?}");
    assert!(
        r.end().0 - r.start().0 <= 2 * 64 * 1024 + 8,
        "窓 (64 KiB) の中だけ: {r:?}"
    );
    assert!(r.end().0 < n, "行の全体ではない: {r:?}");
}

// verifies: FR-311
// spec: 03 §5.8.2 (CLK-5)
#[test]
fn 選択があってもダブルクリックは押した位置の単語_1_つに置き換える() {
    let mut h = scene("|foo bar");
    let v = view(&h);
    set_selection(&mut h, vec![Range::new(ByteOff(0), ByteOff(3))], 0);
    assert_eq!(shown_in(&h, v), "«foo» bar");
    h.double_click(0, GW + 5);
    assert_eq!(
        selection(&h),
        Selection::single(Range::new(ByteOff(4), ByteOff(7)))
    );
}
