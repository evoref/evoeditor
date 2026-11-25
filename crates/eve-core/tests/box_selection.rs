//! 矩形 (ボックス) 選択と編集 (FR-212、docs/03 §4.8 の BOX-1〜BOX-9、01 §5.5 の FR-212 AC-1 / AC-2) のシナリオ (03 §15.9.1 の
//! L3。M34 の段 7)。文書と Range の印は `marked` (`multi_cursor.rs` と共有): `|` は空のカーソル、`«` `»` は前向きの選択
//! (`»` が head)、`‹` `›` は後ろ向きの選択 (`‹` が head)、`*` は主の Range。

mod marked;

use eve_core::{CommandResult, WrapMode, WrapSettings};
use eve_testkit::Harness;
use eve_testkit::helpers::opened;
use eve_text::{ByteOff, Range, Selection};
use eve_view::{ClipboardText, Effect, Modifiers, MouseButton, MouseKind, StatusKind};
use marked::{render, scene, selection, set_selection, shown, view};

/// 矩形選択のモードを入れる (既定キー `ctrl+k b`)。
fn box_on(h: &mut Harness) {
    h.keys("ctrl+k b");
    assert!(h.editor.state().box_mode(view(h)), "モードが入る");
}

/// ステータスバーの選択情報 (long, short)。
fn status_selection(h: &mut Harness) -> Option<(String, Option<String>)> {
    let f = h.frame();
    f.status
        .items
        .iter()
        .find(|i| i.kind == StatusKind::Selection)
        .map(|i| {
            let t = |c: &eve_view::CellText| eve_testkit::helpers::cell_text(c);
            (t(&i.long), i.short.as_ref().map(t))
        })
}

/// フォーカスのペインのガターの幅。
fn gutter(h: &mut Harness) -> i32 {
    i32::from(h.frame().panes[0].gutter_width)
}

/// システムのクリップボードへ渡した文字列 (最後のもの)。
fn last_clipboard(h: &Harness) -> Option<String> {
    h.effects().iter().rev().find_map(|e| match e {
        Effect::SetClipboard {
            content: ClipboardText::Inline(t),
            ..
        } => Some(t.clone()),
        _ => None,
    })
}

/// `edit.paste` を実行し、システムのクリップボードの応答として `text` を返す。
fn paste_via_command(h: &mut Harness, text: Option<&str>) {
    h.cmd("edit.paste");
    let token = h
        .effects()
        .iter()
        .rev()
        .find_map(|e| match e {
            Effect::RequestClipboard { token, .. } => Some(*token),
            _ => None,
        })
        .expect("RequestClipboard");
    h.clipboard_content(token, text);
}

fn mouse(h: &mut Harness, kind: MouseKind, mods: Modifiers, row: i32, col: i32) {
    h.mouse(kind, mods, row, col);
}

/// 左のドラッグ (押す前に時計を多重クリックの間隔より進め、押して、動かして、離す)。
fn drag(h: &mut Harness, from: (i32, i32), to: (i32, i32)) {
    h.advance_ms(eve_core::DOUBLE_CLICK_MS.0 + 1);
    mouse(
        h,
        MouseKind::Down(MouseButton::Left),
        Modifiers::NONE,
        from.0,
        from.1,
    );
    mouse(
        h,
        MouseKind::Drag(MouseButton::Left),
        Modifiers::NONE,
        to.0,
        to.1,
    );
    mouse(
        h,
        MouseKind::Up(MouseButton::Left),
        Modifiers::NONE,
        to.0,
        to.1,
    );
}

/// `Shift` 付きの左のクリック。
fn shift_click(h: &mut Harness, row: i32, col: i32) {
    mouse(
        h,
        MouseKind::Down(MouseButton::Left),
        Modifiers::SHIFT,
        row,
        col,
    );
    mouse(
        h,
        MouseKind::Up(MouseButton::Left),
        Modifiers::SHIFT,
        row,
        col,
    );
}

fn box_size(h: &Harness) -> Option<(usize, usize)> {
    h.editor.state().box_size(view(h))
}

// ---------- 01 FR-212 AC-1 ----------

// verifies: AC-FR-212-1
// spec: 03 §4.8 (BOX-1, BOX-2, BOX-3, BOX-4, BOX-7)
#[test]
fn ac1_モードでは_shift_矢印が表示の列で揃えた矩形を選び_入力と_1_回の_undo_ができる() {
    let mut h = scene("01|23456789\nab\nあいうえ\n\tbcdefg\n");
    box_on(&mut h);
    assert_eq!(
        shown(&h),
        "01|23456789\nab\nあいうえ\n\tbcdefg\n",
        "入れても選択は変えない"
    );
    assert_eq!(
        status_selection(&mut h),
        Some(("(矩形選択)".to_owned(), Some("矩形".to_owned())))
    );
    h.keys("shift+right shift+right shift+right shift+down shift+down shift+down");
    // 列 2〜5: `ab` (幅 2) は左の列にちょうど届くので行末の空のカーソル、全角は掛かる文字を含め、タブの途中から始まる行は
    // タブを含める
    assert_eq!(shown(&h), "01«234»56789\nab|\nあ«いう»え\n«\tb»cdefg\n");
    assert_eq!(selection(&h).primary_index(), 3, "主は head の行");
    assert_eq!(
        status_selection(&mut h),
        Some(("(矩形 4行 3列)".to_owned(), Some("矩形4x3".to_owned())))
    );
    h.text("X");
    assert_eq!(h.doc_text(), "01X56789\nabX\nあXえ\nXcdefg\n");
    assert_eq!(box_size(&h), None, "編集の後は矩形が消える (モードは残る)");
    assert!(h.editor.state().box_mode(view(&h)));
    assert_eq!(selection(&h).len(), 4, "Range は複数カーソルとして残る");
    h.keys("ctrl+z");
    assert_eq!(
        h.doc_text(),
        "0123456789\nab\nあいうえ\n\tbcdefg\n",
        "1 回の Undo で戻る"
    );
}

// verifies: AC-FR-212-1
// spec: 03 §4.8 (BOX-6)
#[test]
fn ac1_ドラッグと_shift_クリックが矩形を選び_行末より右にも広げられる() {
    let mut h = scene("|abcdef\nab\nabcdef\n");
    box_on(&mut h);
    let gw = gutter(&mut h);
    drag(&mut h, (0, gw + 1), (2, gw + 4));
    assert_eq!(shown(&h), "a«bcd»ef\na«b»\na«bcd»ef\n");
    // 行末より右へドラッグ (仮想の列。空白は詰めない)
    drag(&mut h, (0, gw + 1), (1, gw + 9));
    assert_eq!(shown(&h), "a«bcdef»\na«b»\nabcdef\n");
    assert_eq!(box_size(&h), Some((2, 8)));
    // `Shift` 付きのクリックで head を動かす (anchor は保つ)
    shift_click(&mut h, 2, gw + 3);
    assert_eq!(shown(&h), "a«bc»def\na«b»\na«bc»def\n");
    // ドラッグしないクリックは矩形を消して 1 つのカーソル
    h.click(1, gw);
    assert_eq!(shown(&h), "abcdef\n|ab\nabcdef\n");
    assert_eq!(box_size(&h), None);
    assert!(h.editor.state().box_mode(view(&h)), "モードは残る");
}

// verifies: AC-FR-212-1
// spec: 03 §4.8 (BOX-4)
#[test]
fn ac1_矩形のコピーは短い行を空の行にし_同じ高さの矩形へ_1_行ずつ戻る() {
    let mut h = scene("ab|cd\na\nabcd\n");
    box_on(&mut h);
    h.keys("shift+right shift+right shift+down shift+down");
    assert_eq!(shown(&h), "ab«cd»\na\nab«cd»\n", "短い行には Range が無い");
    h.cmd("clipboard.copy");
    assert_eq!(
        last_clipboard(&h).as_deref(),
        Some("cd\n\ncd"),
        "高さ 3 の片"
    );
    // 幅 0 の高さ 3 の矩形へ貼る (内部レジスタと一致する)
    h.keys("ctrl+home");
    h.keys("shift+down shift+down");
    assert_eq!(shown(&h), "|abcd\n|a\n|abcd\n");
    paste_via_command(&mut h, Some("cd\n\ncd"));
    assert_eq!(h.doc_text(), "cdabcd\na\ncdabcd\n");
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "abcd\na\nabcd\n", "1 回の Undo で戻る");
}

// verifies: AC-FR-212-1
// spec: 03 §4.8 (BOX-4)
#[test]
fn ac1_矩形の切り取りは_range_だけを消し_貼り付けで戻る() {
    let mut h = scene("ab|cd\na\nabcd\n");
    box_on(&mut h);
    h.keys("shift+right shift+right shift+down shift+down");
    h.cmd("clipboard.cut");
    assert_eq!(h.doc_text(), "ab\na\nab\n", "Range の無い短い行は変えない");
    assert_eq!(last_clipboard(&h).as_deref(), Some("cd\n\ncd"));
    // 同じ位置に高さ 3 の矩形を作り直して貼る (読み取れなかったときは内部レジスタ)
    h.keys("ctrl+home end shift+down shift+down");
    paste_via_command(&mut h, None);
    assert_eq!(h.doc_text(), "abcd\na\nabcd\n");
}

// verifies: AC-FR-212-1
// spec: 03 §4.8 (BOX-1, MCUR-5)
#[test]
fn ac1_同じコマンドか_esc_の層の最後でモードを切る() {
    let mut h = scene("ab|cd\nabcd\nabcd\n");
    box_on(&mut h);
    h.keys("shift+right shift+down shift+down");
    assert_eq!(selection(&h).len(), 3);
    // 1 回目: 主の Range だけを残す (矩形は消える)
    h.keys("escape");
    assert_eq!(shown(&h), "abcd\nabcd\nab«c»d\n");
    assert_eq!(
        status_selection(&mut h).map(|s| s.0).as_deref(),
        Some("(矩形選択)")
    );
    // 2 回目: 選択を解く
    h.keys("escape");
    assert_eq!(shown(&h), "abcd\nabcd\nabc|d\n");
    assert!(h.editor.state().box_mode(view(&h)));
    // 3 回目: モードを切る
    h.keys("escape");
    assert!(!h.editor.state().box_mode(view(&h)));
    assert_eq!(status_selection(&mut h), None);
    // 4 回目: 何もしない
    h.keys("escape");
    assert_eq!(shown(&h), "abcd\nabcd\nabc|d\n");
    // もう一度のコマンドでも切れる。切ると今の Range を複数カーソルとして残す
    box_on(&mut h);
    h.keys("shift+left shift+up");
    assert_eq!(selection(&h).len(), 2);
    h.keys("ctrl+k b");
    assert!(!h.editor.state().box_mode(view(&h)));
    assert_eq!(shown(&h), "abcd\nab‹c›d\nab‹c›d\n");
    assert_eq!(
        status_selection(&mut h).map(|s| s.0).as_deref(),
        Some("カーソル 2 個"),
        "複数カーソルとして残る"
    );
}

// verifies: AC-FR-212-1
// spec: 03 §4.8 (BOX-2)
#[test]
fn ac1_キーボードでは最も長い行の幅_1_列で止まり_行末までは短い行で仮想の列になる() {
    let mut h = scene("|ab\nabcdef\nabc\n");
    box_on(&mut h);
    h.keys("shift+down shift+end");
    assert_eq!(shown(&h), "«ab»\n«abcdef»\nabc\n");
    assert_eq!(box_size(&h), Some((2, 6)), "最も長い行の幅まで");
    h.keys("shift+right shift+right shift+right");
    assert_eq!(box_size(&h), Some((2, 7)), "最も長い行の幅 + 1 列で止まる");
    h.keys("shift+left");
    assert_eq!(box_size(&h), Some((2, 6)));
}

// ---------- 01 FR-212 AC-2 ----------

// verifies: AC-FR-212-2
// spec: 03 §4.8 (BOX-3, BOX-9), 03 §4.5.1 (MCUR-6)
#[test]
fn ac2_行の数が_10000_を超える矩形は断り_その旨を示す() {
    let mut h = scene(&format!("|{}", "x\n".repeat(10_001)));
    box_on(&mut h);
    h.clear_log();
    // 10,002 行の矩形: 断る (矩形も Selection も変えない)
    h.keys("ctrl+shift+end");
    assert_eq!(h.messages(), ["info.cursor.limit"]);
    assert_eq!(selection(&h), Selection::point(ByteOff(0)));
    assert_eq!(box_size(&h), None);
    // ちょうど 10,000 行は作れる
    h.cmd_with(
        "select.extend_down",
        eve_core::Args::new().with_int("count", 9_999),
    );
    assert_eq!(selection(&h).len(), 10_000);
    assert_eq!(box_size(&h), Some((10_000, 0)));
    // 1 行増やすと断り、直前の矩形と Selection のまま
    h.clear_log();
    let before = selection(&h);
    h.keys("shift+down");
    assert_eq!(h.messages(), ["info.cursor.limit"]);
    assert_eq!(selection(&h), before);
    assert_eq!(box_size(&h), Some((10_000, 0)));
}

// verifies: AC-FR-212-2
// spec: 03 §4.8 (BOX-1)
#[test]
fn ac2_モードを入れなければ_v0_4_と同じ選択になる() {
    let mut h = scene("ab|cd\nabcd\n");
    h.keys("shift+right shift+down");
    assert_eq!(shown(&h), "ab«cd\nabc»d\n");
    assert_eq!(box_size(&h), None);
    // ドラッグ・Shift 付きのクリックも今のまま
    let gw = gutter(&mut h);
    drag(&mut h, (0, gw + 1), (1, gw + 2));
    assert_eq!(shown(&h), "a«bcd\nab»cd\n");
    shift_click(&mut h, 1, gw + 3);
    assert_eq!(shown(&h), "a«bcd\nabc»d\n");
}

// ---------- BOX-1: モード ----------

// spec: 03 §4.8 (BOX-1)
#[test]
fn モードの切り替えは文書を変えず_undo_の履歴に残らず_読み取り専用でも使える() {
    let mut h = opened("/w/a.txt", b"abcd\nabcd\n");
    let rev = h.editor.state().document(h.doc_id()).map(|d| d.revision());
    h.cmd("file.toggle_read_only");
    box_on(&mut h);
    h.keys("shift+right shift+down");
    assert_eq!(shown(&h), "«a»bcd\n«a»bcd\n");
    assert_eq!(
        h.editor.state().document(h.doc_id()).map(|d| d.revision()),
        rev
    );
    assert!(!h.is_dirty());
}

// spec: 03 §4.8 (BOX-1), 03 §9.9 (PANE-1, PANE-2, PANE-5)
#[test]
fn モードは_view_ごとで_分割で作る_view_は切った状態_文書を切り替えて戻ると戻る() {
    let mut h = scene("ab|cd\nabcd\n");
    box_on(&mut h);
    let left = view(&h);
    assert_eq!(h.cmd("pane.split_right"), CommandResult::Done);
    let right = view(&h);
    assert_ne!(left, right);
    assert!(
        !h.editor.state().box_mode(right),
        "分割で作る View は切った状態"
    );
    assert!(h.editor.state().box_mode(left));
    // 右のペインで別の文書に切り替えて戻る
    box_on(&mut h);
    h.cmd("file.new");
    assert!(
        !h.editor.state().box_mode(view(&h)),
        "初めて出す文書の View"
    );
    h.cmd("buffer.prev");
    assert_eq!(view(&h), right);
    assert!(
        h.editor.state().box_mode(right),
        "戻るとその View のモードが戻る"
    );
}

// ---------- BOX-2: 矩形の作り方と動かし方 ----------

// spec: 03 §4.8 (BOX-2)
#[test]
fn 矩形が無ければ主の_range_の_head_から作り_ほかの_range_は捨てる() {
    let mut h = scene("a|bcd\nab«cd»\nabcd\n");
    box_on(&mut h);
    h.keys("shift+down");
    // 主 (後の Range) の head は 1 行目の列 4。anchor も同じ所から
    assert_eq!(shown(&h), "abcd\nabcd|\nabcd|\n");
    assert_eq!(box_size(&h), Some((2, 0)));
}

// spec: 03 §4.8 (BOX-2)
#[test]
fn 移動と全選択とクリックは矩形を消し_モードは残る() {
    let mut h = scene("ab|cd\nabcd\n");
    box_on(&mut h);
    h.keys("shift+right shift+down");
    assert_eq!(box_size(&h), Some((2, 1)));
    h.keys("right");
    assert_eq!(box_size(&h), None);
    assert_eq!(shown(&h), "abc|d\nabc|d\n", "移動は今までどおり");
    assert!(h.editor.state().box_mode(view(&h)));
    h.keys("shift+down");
    assert_eq!(box_size(&h), Some((2, 0)), "続けると主の head から作る");
    h.keys("ctrl+a");
    assert_eq!(box_size(&h), None);
    assert_eq!(shown(&h), "«abcd\nabcd\n»");
}

// spec: 03 §4.8 (BOX-2)
#[test]
fn 単語の移動とホームとページと文書の端() {
    let mut h = scene("  foo bar|\n  baz\nqux quux\nend\n").with_size(80, 5);
    box_on(&mut h);
    h.keys("ctrl+shift+left");
    assert_eq!(
        shown(&h),
        "  foo ‹bar›\n  baz\nqux quux\nend\n",
        "単語の移動は head の行の中"
    );
    h.keys("shift+home");
    assert_eq!(
        shown(&h),
        "  ‹foo bar›\n  baz\nqux quux\nend\n",
        "字下げの終わり"
    );
    h.keys("shift+down");
    assert_eq!(shown(&h), "  ‹foo bar›\n  ‹baz›\nqux quux\nend\n");
    // ページ移動は行を表示の行数 - 1 動かす (列は保つ)
    let page = usize::from(h.editor.state().viewport().1)
        .saturating_sub(1)
        .max(1);
    h.keys("shift+pagedown");
    assert_eq!(box_size(&h), Some(((1 + page).min(4) + 1, 7)));
    h.keys("ctrl+shift+home");
    assert_eq!(box_size(&h), Some((1, 7)));
    assert_eq!(shown(&h), "  ‹foo bar›\n  baz\nqux quux\nend\n");
    h.keys("ctrl+shift+end");
    assert_eq!(box_size(&h), Some((5, 7)), "最後の (空の) 行まで");
}

// spec: 03 §4.8 (BOX-2), 03 §4.5.2
#[test]
fn 編集の後に選択を伸ばすと主の_head_から新しい矩形を作る() {
    let mut h = scene("|abcd\nabcd\nabcd\n");
    box_on(&mut h);
    h.keys("shift+down shift+down");
    h.text("X");
    assert_eq!(shown(&h), "X|abcd\nX|abcd\nX|abcd\n");
    h.keys("shift+right");
    assert_eq!(shown(&h), "Xabcd\nXabcd\nX«a»bcd\n");
    assert_eq!(box_size(&h), Some((1, 1)));
}

// ---------- BOX-3 / BOX-5: Selection の作り方・仮想の空白 ----------

// spec: 03 §4.8 (BOX-3, BOX-5)
#[test]
fn 左の列に届かない行には_range_を作らず_打った文字は同じ列に入り空白を詰めない() {
    let mut h = scene("abc|d\na\n\nabcdef\nab\n");
    box_on(&mut h);
    h.keys("shift+down shift+down shift+down shift+down");
    assert_eq!(
        shown(&h),
        "abc|d\na\n\nabc|def\nab|\n",
        "head の行は届かなければ行末"
    );
    h.text("_");
    assert_eq!(h.doc_text(), "abc_d\na\n\nabc_def\nab_\n");
}

// spec: 03 §4.8 (BOX-3)
#[test]
fn 後ろ向きの矩形と全角とタブと結合文字() {
    let mut h = scene("abcde|\nあいう\n\tz\ne\u{301}e\u{301}e\u{301}e\u{301}x\n");
    box_on(&mut h);
    h.keys("shift+left shift+left shift+left shift+down shift+down shift+down");
    // 列 2〜5 (head が左なので後ろ向き)
    assert_eq!(
        shown(&h),
        "ab‹cde›\nあ‹いう›\n‹\tz›\ne\u{301}e\u{301}‹e\u{301}e\u{301}x›\n"
    );
}

// spec: 03 §4.8 (BOX-3)
#[test]
fn crlf_の文書では区切りの前までを選び_コピーは文書の改行で連結する() {
    let mut h = opened("/w/crlf.txt", b"abc\r\nab\r\nabcd\r\n");
    box_on(&mut h);
    h.keys("right shift+right shift+right shift+right shift+right shift+down shift+down");
    assert_eq!(h.doc_text(), "abc\r\nab\r\nabcd\r\n");
    assert_eq!(box_size(&h), Some((3, 3)));
    assert_eq!(shown(&h), "a«bc»\r\na«b»\r\na«bcd»\r\n");
    h.cmd("clipboard.copy");
    assert_eq!(last_clipboard(&h).as_deref(), Some("bc\r\nb\r\nbcd"));
}

// ---------- BOX-4: 編集とクリップボード ----------

// spec: 03 §4.8 (BOX-4), 03 §4.5.3 (MCUR-10)
#[test]
fn 高さの違う矩形への貼り付けは_mcur_10_の_3_段() {
    let mut h = scene("ab|cd\na\nabcd\n");
    box_on(&mut h);
    h.keys("shift+right shift+right shift+down shift+down");
    h.cmd("clipboard.copy");
    // 高さ 2 の矩形 (Range 2 つ) へ、内部レジスタ (片 3 つ) と一致する文字列 → 行の数 (3) も合わないので全文
    h.keys("ctrl+home shift+down");
    paste_via_command(&mut h, Some("cd\n\ncd"));
    assert_eq!(h.doc_text(), "cd\n\ncdabcd\ncd\n\ncda\nabcd\n");
    h.keys("ctrl+z");
    // ほかのアプリからの 2 行は、2 つの Range へ 1 行ずつ (2 段目)
    h.keys("ctrl+home shift+down");
    paste_via_command(&mut h, Some("X\nY"));
    assert_eq!(h.doc_text(), "Xabcd\nYa\nabcd\n");
}

// spec: 03 §4.8 (BOX-4)
#[test]
fn 矩形が無い複数カーソルへの貼り付けは_mcur_8_から_mcur_10_のまま() {
    let mut h = scene("ab|cd\na\nabcd\n");
    box_on(&mut h);
    h.keys("shift+right shift+right shift+down shift+down");
    h.cmd("clipboard.copy");
    // 矩形を消して (Range 2 つは複数カーソルとして残る) から貼る: 片 3 つと Range 2 つは合わないので全文
    h.keys("escape escape");
    set_selection(
        &mut h,
        vec![Range::point(ByteOff(0)), Range::point(ByteOff(7))],
        1,
    );
    paste_via_command(&mut h, None);
    assert_eq!(h.doc_text(), "cd\n\ncdabcd\na\ncd\n\ncdabcd\n");
}

// spec: 03 §4.8 (BOX-4), 03 §5.7 (SEL-7)
#[test]
fn 選択の拡張はモードの間は矩形を消してから広げる() {
    let mut h = scene("|foo bar\nbaz qux\n");
    box_on(&mut h);
    h.keys("shift+right shift+down");
    assert_eq!(box_size(&h), Some((2, 1)));
    h.cmd("select.expand");
    assert_eq!(shown(&h), "«foo» bar\n«baz» qux\n");
    assert_eq!(box_size(&h), None);
    assert!(h.editor.state().box_mode(view(&h)));
}

// ---------- BOX-6: マウス ----------

// spec: 03 §4.8 (BOX-6), 03 §5.8.2 (CLK-5)
#[test]
fn ダブル_トリプルクリックはモードの間も単語と行を選ぶ() {
    let mut h = scene("|foo bar\nbaz\n");
    box_on(&mut h);
    let gw = gutter(&mut h);
    h.double_click(0, gw + 5);
    assert_eq!(shown(&h), "foo «bar»\nbaz\n");
    assert_eq!(box_size(&h), None);
    h.triple_click(1, gw + 1);
    assert_eq!(shown(&h), "foo bar\n«baz\n»");
    assert!(h.editor.state().box_mode(view(&h)));
}

// spec: 03 §4.8 (BOX-6)
#[test]
fn マウスの列は全角とタブの途中のセルをその文字の始まりにする() {
    let mut h = scene("|あいう\n\tabc\n");
    box_on(&mut h);
    let gw = gutter(&mut h);
    // あ の右半分 (列 1) から う の右半分 (列 5) へ: 列 0〜4 → あ い
    drag(&mut h, (0, gw + 1), (0, gw + 5));
    assert_eq!(shown(&h), "«あい»う\n\tabc\n");
    // タブの途中 (列 2) から 2 行目の列 5 まで
    drag(&mut h, (0, gw + 2), (1, gw + 5));
    assert_eq!(shown(&h), "あ«いう»\n«\ta»bc\n");
}

// spec: 03 §4.8 (BOX-6, BOX-8), 01 §4.6
#[test]
fn 折り返し_on_の継続行の押下は論理行の行頭からの表示列で_行末より右は仮想の列() {
    // 行番号なし・幅 81 (折り返し幅 80)・文字の単位で折り返し、継続行は字下げしない
    let mut h = Harness::builder()
        .size(81, 10)
        .config(eve_config::Config {
            line_numbers: eve_config::LineNumberMode::Off,
            ..eve_config::Config::default()
        })
        .build();
    h.paste(&format!("{}\n{}\n", "x".repeat(100), "y".repeat(100)));
    h.keys("ctrl+home");
    let v = view(&h);
    h.editor.state_mut().set_wrap(
        v,
        WrapSettings {
            mode: WrapMode::Soft,
            indent_same: false,
            ..WrapSettings::default()
        },
    );
    box_on(&mut h);
    // 1 行目の継続行 (面の 1 行目。表示列 80〜99) の 30 セル目: 表示行の末尾 100 + はみ出し 10 = 110
    shift_click(&mut h, 1, 30);
    assert_eq!(box_size(&h), Some((1, 110)));
    // 継続行どうしのドラッグも論理行の行頭からの表示列 (見た目の矩形にならない、01 §4.6 の既知の制約)
    drag(&mut h, (1, 5), (3, 10));
    assert_eq!(box_size(&h), Some((2, 5)));
    let sel = selection(&h);
    let spans: Vec<(usize, usize)> = sel.iter().map(|r| (r.start().0, r.end().0)).collect();
    assert_eq!(spans, vec![(85, 90), (101 + 85, 101 + 90)]);
}

// spec: 03 §4.8 (BOX-6)
#[test]
fn 横スクロールの間のドラッグはスクロールの分を足した列() {
    let line = "0123456789".repeat(10);
    let mut h = Harness::new().with_size(40, 10);
    h.paste(&format!("{line}\n{line}\n"));
    h.keys("ctrl+home");
    box_on(&mut h);
    let gw = gutter(&mut h);
    let v = view(&h);
    let mut scroll = h.editor.state().view(v).expect("View").scroll;
    scroll.left_col = 50;
    h.editor.state_mut().set_scroll(v, scroll);
    drag(&mut h, (0, gw + 2), (1, gw + 5));
    let spans: Vec<(usize, usize)> = selection(&h)
        .iter()
        .map(|r| (r.start().0, r.end().0))
        .collect();
    assert_eq!(spans, vec![(52, 55), (101 + 52, 101 + 55)]);
}

// ---------- BOX-7: 表示 ----------

// spec: 03 §4.8 (BOX-7), 04 §8.7
#[test]
fn ステータスの文言は_en_でも丸括弧で_モードだけのときは矩形選択() {
    let mut h = scene("|abc\nabc\n").with_locale(eve_config::Locale::En);
    box_on(&mut h);
    assert_eq!(
        status_selection(&mut h),
        Some(("(Box mode)".to_owned(), Some("Box".to_owned())))
    );
    h.keys("shift+right shift+right shift+down");
    assert_eq!(
        status_selection(&mut h),
        Some(("(Box 2x2)".to_owned(), Some("Box 2x2".to_owned())))
    );
}

// spec: 03 §4.8 (BOX-7, LNO-6 の形), 04 §8.7
#[test]
fn ascii_の_frame_矩形選択のモードのステータスバー() {
    let mut h = scene("|abcdef\nab\nあいうえ\n\tbcdefg\n").with_size(60, 8);
    box_on(&mut h);
    h.keys("right shift+right shift+right shift+right shift+down shift+down shift+down");
    let text = h.frame_text_with(eve_testkit::AsciiOptions {
        show_gutter: true,
        show_chrome: true,
        ..eve_testkit::AsciiOptions::default()
    });
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/box_selection_status.frame.txt");
    eve_testkit::golden::check(&path, &text);
}

// ---------- 複数ペイン・長大行 ----------

// spec: 03 §4.8 (BOX-4), 03 §4.6, 03 §9.9 (PANE-2)
#[test]
fn ほかのペインの編集で矩形は消え_range_は写像して残る() {
    let mut h = scene("ab|cd\nabcd\n");
    let left = view(&h);
    h.cmd("pane.split_right");
    let right = view(&h);
    h.cmd("pane.focus_next");
    assert_eq!(view(&h), left);
    box_on(&mut h);
    h.keys("shift+right shift+down");
    assert_eq!(h.editor.state().box_size(left), Some((2, 1)));
    // 右のペインで先頭に文字を入れる
    h.cmd("pane.focus_next");
    assert_eq!(view(&h), right);
    h.keys("ctrl+home");
    h.text("Z");
    assert_eq!(
        h.editor.state().box_size(left),
        None,
        "写像の後は矩形が消える"
    );
    let sel = h.editor.state().view(left).expect("View").selection.clone();
    assert_eq!(render(&h.doc_text(), &sel), "Zab«c»d\nab«c»d\n");
    assert!(h.editor.state().box_mode(left));
    assert!(!h.editor.state().box_mode(right));
}

// spec: 03 §4.8 (BOX-8), 03 §14.4
#[test]
fn 長大行では列の索引で表示列から位置を求める() {
    let long: String = "aあ\t".repeat(30_000);
    let mut data = long.clone().into_bytes();
    data.extend_from_slice(b"\nshort\n");
    data.extend_from_slice(long.as_bytes());
    data.push(b'\n');
    let mut h = Harness::new()
        .with_file("/w/long.txt", &data)
        .with_size(80, 10);
    h.editor.state_mut().set_config(eve_config::Config {
        long_line_threshold: 65_536,
        ..eve_config::Config::default()
    });
    h.open("/w/long.txt");
    h.run_jobs();
    let st = h.editor.state();
    let v = view(&h);
    let d = st.document(st.view(v).expect("View").doc).expect("文書");
    assert_eq!(d.format.long_lines, 2, "長大行の文書");
    // 1 つの単位 `aあ\t` は表示の 4 列・5 バイト。列 40000〜40006 は 10000 個目の単位の始まりから
    let at = |ls: usize, col: usize| ls + col / 4 * 5;
    let sel = Selection::new(vec![Range::point(ByteOff(0))], 0).expect("Selection");
    h.editor.state_mut().set_selection(v, sel);
    box_on(&mut h);
    h.cmd_with(
        "select.extend_right",
        eve_core::Args::new().with_int("count", 3 * 10_000),
    );
    h.cmd_with(
        "select.extend_down",
        eve_core::Args::new().with_int("count", 2),
    );
    let (rows, cols) = box_size(&h).expect("矩形");
    assert_eq!((rows, cols), (3, 40_000));
    let spans: Vec<(usize, usize)> = selection(&h)
        .iter()
        .map(|r| (r.start().0, r.end().0))
        .collect();
    let second = long.len() + 1 + "short\n".len();
    // `short` は幅 5 で列 0〜40000 を末尾まで選ぶ
    assert_eq!(
        spans,
        vec![
            (0, at(0, 40_000)),
            (long.len() + 1, long.len() + 6),
            (second, at(second, 40_000))
        ]
    );
}

// ---------- レビューで足した試験 ----------

/// 押す (時計を多重クリックの間隔より進めてから、離さない)。
fn press_down(h: &mut Harness, row: i32, col: i32) {
    h.advance_ms(eve_core::DOUBLE_CLICK_MS.0 + 1);
    mouse(
        h,
        MouseKind::Down(MouseButton::Left),
        Modifiers::NONE,
        row,
        col,
    );
}

fn drag_to(h: &mut Harness, row: i32, col: i32) {
    mouse(
        h,
        MouseKind::Drag(MouseButton::Left),
        Modifiers::NONE,
        row,
        col,
    );
}

fn limit_messages(h: &Harness) -> usize {
    h.messages()
        .iter()
        .filter(|m| *m == "info.cursor.limit")
        .count()
}

/// 行 `line` の行頭を先頭の行にスクロールする。
fn scroll_to_line(h: &mut Harness, line: usize) {
    let v = view(h);
    h.editor.state_mut().set_scroll_top(v, ByteOff(line * 2));
}

// verifies: AC-FR-212-2
// spec: 03 §4.8 (BOX-3, BOX-6, §4.8.1 の「上限の知らせ」)
#[test]
fn ドラッグで上限を超え続けても知らせは_1_回_下回って再び超えたらもう_1_回() {
    let mut h = scene(&format!("|{}", "x\n".repeat(10_001)));
    box_on(&mut h);
    let gw = gutter(&mut h);
    scroll_to_line(&mut h, 0);
    press_down(&mut h, 0, gw);
    h.clear_log();
    // 10,002 行の矩形 (断る)。最後の行までスクロールして面の下へドラッグする。位置を変えて超え続けても知らせは 1 回
    scroll_to_line(&mut h, 10_001);
    drag_to(&mut h, 1_000, gw);
    drag_to(&mut h, 1_000, gw + 1);
    drag_to(&mut h, 1_000, gw + 2);
    assert_eq!(limit_messages(&h), 1);
    assert_eq!(box_size(&h), None, "断ったので矩形を置かない");
    // 上限以下へ戻ると、押した位置を anchor にした矩形になる
    scroll_to_line(&mut h, 9_000);
    drag_to(&mut h, 0, gw);
    assert_eq!(h.editor.state().box_lines(view(&h)), Some((0, 9_000)));
    // 再び超えたらもう 1 回
    scroll_to_line(&mut h, 10_001);
    drag_to(&mut h, 1_000, gw);
    assert_eq!(limit_messages(&h), 2);
    assert_eq!(
        h.editor.state().box_lines(view(&h)),
        Some((0, 9_000)),
        "直前の矩形のまま"
    );
    mouse(
        &mut h,
        MouseKind::Up(MouseButton::Left),
        Modifiers::NONE,
        0,
        gw,
    );
}

// spec: 03 §4.8 (BOX-6)
#[test]
fn ドラッグの途中で文書が縮んでも_anchor_を最後の行に丸めて続ける() {
    let mut h = scene("|abc\nabc\nabc\nabc");
    box_on(&mut h);
    let gw = gutter(&mut h);
    press_down(&mut h, 3, gw + 1);
    // ドラッグの間に文書の全体を消す
    h.keys("ctrl+a delete");
    assert_eq!(h.doc_text(), "");
    drag_to(&mut h, 0, gw + 2);
    assert_eq!(h.editor.state().box_lines(view(&h)), Some((0, 0)));
    assert_eq!(shown(&h), "|");
}

/// 行番号なし・幅 `cols` で、折り返しを `mode` (継続行の字下げなし・禁則あり) にした Harness。
fn wrapped(body: &str, cols: u16, mode: WrapMode) -> Harness {
    let mut h = Harness::builder()
        .size(cols, 10)
        .config(eve_config::Config {
            line_numbers: eve_config::LineNumberMode::Off,
            ..eve_config::Config::default()
        })
        .build();
    h.paste(body);
    h.keys("ctrl+home");
    let v = view(&h);
    h.editor.state_mut().set_wrap(
        v,
        WrapSettings {
            mode,
            indent_same: false,
            kinsoku: true,
        },
    );
    h
}

// spec: 03 §4.8 (BOX-6)
#[test]
fn 単語の折り返しの途中の表示行の右の空きは_その表示行の末尾の列() {
    // 幅 21 (折り返し幅 20): 1 行目は `aaaa ` (列 0〜4) と `bbb…` (列 5〜) に分かれ、1 行目の右に空きができる
    let mut h = wrapped(&format!("aaaa {}\n", "b".repeat(18)), 21, WrapMode::Word);
    box_on(&mut h);
    shift_click(&mut h, 0, 15);
    assert_eq!(
        box_size(&h),
        Some((1, 5)),
        "途中の表示行の右は仮想の列にしない"
    );
    // 最後の表示行の右は仮想の列
    shift_click(&mut h, 1, 19);
    assert_eq!(box_size(&h), Some((1, 24)));
}

// spec: 03 §4.8 (BOX-6), 03 §14.3 (禁則)
#[test]
fn 禁則で送った表示行の右の空きも_その表示行の末尾の列() {
    // 幅 11 (折り返し幅 10): `。` を行頭に置かないので `お` を次の表示行へ送り、1 行目は `あいうえ` (列 0〜7)
    let mut h = wrapped("あいうえお。\n", 11, WrapMode::Soft);
    box_on(&mut h);
    shift_click(&mut h, 0, 9);
    assert_eq!(box_size(&h), Some((1, 8)));
}

// spec: 03 §4.8 (BOX-2, BOX-6), 03 §5.8.2 (CLK-4)
#[test]
fn 折り返し_on_で前向きの矩形の主の_head_が折り返し位置なら上の表示行の末尾() {
    let mut h = wrapped(&format!("{}\n", "x".repeat(30)), 21, WrapMode::Soft);
    box_on(&mut h);
    // 列 0〜20: 主の head は 20 (折り返し位置)
    shift_click(&mut h, 1, 0);
    assert_eq!(box_size(&h), Some((1, 20)));
    let v = view(&h);
    assert_eq!(
        h.editor.state().view(v).expect("View").cursor_affinity,
        eve_core::Affinity::Before
    );
    let f = h.frame();
    assert_eq!(
        f.primary_cursor.map(|c| (c.pos.row, c.pos.col)),
        Some((0, 20)),
        "上の表示行の末尾に描く"
    );
}

// spec: 03 §4.8 (BOX-4)
#[test]
fn undo_と_redo_の後は同じ_selection_に戻っても矩形は戻らない() {
    let mut h = scene("|abcd\nabcd\n");
    box_on(&mut h);
    h.keys("shift+right shift+down");
    let boxed = selection(&h);
    h.text("X");
    h.keys("ctrl+z");
    assert_eq!(selection(&h), boxed, "Undo で選択は戻る");
    assert_eq!(box_size(&h), None, "矩形は戻らない");
    h.cmd("edit.redo");
    assert_eq!(box_size(&h), None);
    assert!(h.editor.state().box_mode(view(&h)));
}

// spec: 03 §4.8 (BOX-4), 03 §9.5
#[test]
fn 再読込で矩形は消え_検索バーの逐次検索の後は残る() {
    let mut h = opened("/w/a.txt", b"abcd\nabcd\n");
    box_on(&mut h);
    h.keys("shift+right shift+down");
    h.keys("ctrl+f");
    h.text("cd");
    h.keys("escape");
    assert_eq!(
        box_size(&h),
        Some((2, 1)),
        "検索バーの逐次検索は選択を変えない"
    );
    h.fs.add_file("/w/a.txt", b"abcd\nabcd\nabcd\n");
    h.fs.bump_mtime("/w/a.txt", 1);
    h.cmd("file.reload");
    h.run_jobs();
    assert_eq!(h.doc_text(), "abcd\nabcd\nabcd\n");
    assert_eq!(box_size(&h), None);
}

// spec: 03 §4.8 (BOX-4), 03 §4.5.3 (MCUR-10)
#[test]
fn 高さが同じでも内部レジスタと一致しない貼り付けは行の数で分ける() {
    let mut h = scene("|abcd\nabcd\n");
    box_on(&mut h);
    h.keys("shift+right shift+down");
    h.cmd("clipboard.copy");
    assert_eq!(last_clipboard(&h).as_deref(), Some("a\na"));
    // 高さ 2 の矩形へ、ほかのアプリの 2 行を貼る
    paste_via_command(&mut h, Some("P\nQ"));
    assert_eq!(h.doc_text(), "Pbcd\nQbcd\n");
}

// spec: 03 §4.8 (BOX-4), 03 §4.5.3 (MCUR-10)
#[test]
fn 端末からの貼り付けも矩形の行ごとに入る() {
    let mut h = scene("ab|cd\na\nabcd\n");
    box_on(&mut h);
    h.keys("shift+right shift+right shift+down shift+down");
    h.cmd("clipboard.copy");
    h.keys("ctrl+home shift+down shift+down");
    // ブラケットペースト (`InputEvent::Paste`) は届いた文字列で照合する
    h.paste("cd\n\ncd");
    assert_eq!(h.doc_text(), "cdabcd\na\ncdabcd\n");
}

// spec: 03 §4.8 (BOX-4)
#[test]
fn 高さ_1_の矩形への貼り付けは_1_つの_range_の貼り付けと同じ() {
    let mut h = scene("|abcd\nabcd\n");
    box_on(&mut h);
    h.keys("shift+right");
    h.cmd("clipboard.copy");
    paste_via_command(&mut h, Some("X\nY"));
    assert_eq!(h.doc_text(), "X\nYbcd\nabcd\n", "全文を入れる");
}

// spec: 03 §4.8 (BOX-4)
#[test]
fn 最後の行が短い矩形のコピーは末尾が空の片() {
    let mut h = scene("|abcd\nabcd\na\n");
    box_on(&mut h);
    let gw = gutter(&mut h);
    // 3 行目の行末より右から 1 行目の列 2 へ (head は上の行)
    drag(&mut h, (2, gw + 3), (0, gw + 2));
    assert_eq!(shown(&h), "ab‹c›d\nab‹c›d\na\n");
    h.cmd("clipboard.copy");
    assert_eq!(last_clipboard(&h).as_deref(), Some("c\nc\n"));
}

// spec: 03 §4.8 (BOX-2), 03 §5.7 (SEL-7), 03 §4.5.1 (MCUR-2〜MCUR-4)
#[test]
fn 行の選択と縮めると足す操作は矩形を消す() {
    for cmd in [
        "select.line",
        "select.shrink",
        "select.add_cursor_below",
        "select.add_next_match",
        "select.all_matches",
    ] {
        let mut h = scene("|abcd\nabcd\nabcd\n");
        box_on(&mut h);
        h.keys("shift+right shift+down");
        assert_eq!(box_size(&h), Some((2, 1)));
        h.cmd(cmd);
        assert_eq!(box_size(&h), None, "{cmd}");
        assert!(h.editor.state().box_mode(view(&h)), "{cmd}: モードは残る");
    }
}

// spec: 03 §4.8 (BOX-1)
#[test]
fn range_が_1_つの高さ_2_の矩形の_esc_は選択の段の次でモードを切る() {
    let mut h = scene("|ab\nab\n");
    box_on(&mut h);
    let gw = gutter(&mut h);
    // 2 行目の列 5 から 1 行目の列 5 へ: どちらの行も届かず、Range は head の行の行末の 1 つ
    drag(&mut h, (1, gw + 5), (0, gw + 5));
    assert_eq!(box_size(&h), Some((2, 0)));
    assert_eq!(shown(&h), "ab|\nab\n");
    h.keys("escape");
    assert!(
        !h.editor.state().box_mode(view(&h)),
        "1 回の Esc でモードを切る"
    );
    assert_eq!(box_size(&h), None);
}

// spec: 03 §4.8 (BOX-1), 03 §9.9 (PANE-1)
#[test]
fn ペインに初めて出す文書の_view_は写し元のモードを写さない() {
    let mut h = scene("|abcd\n");
    let left = view(&h);
    h.cmd("pane.split_right");
    let right = view(&h);
    // 右のペインで新しい文書を作り、モードを入れる
    h.cmd("file.new");
    let other = view(&h);
    assert_ne!(other, right);
    box_on(&mut h);
    // 左のペインにその文書を出す (右のペインの View を写して作る)
    h.cmd("pane.focus_next");
    assert_eq!(view(&h), left);
    h.cmd("buffer.next");
    let copied = view(&h);
    assert_ne!(copied, other);
    assert_eq!(
        h.editor.state().view(copied).map(|v| v.doc),
        h.editor.state().view(other).map(|v| v.doc)
    );
    assert!(
        !h.editor.state().box_mode(copied),
        "写した View は切った状態"
    );
    assert!(h.editor.state().box_mode(other));
}
