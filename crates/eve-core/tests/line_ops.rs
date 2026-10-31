//! 行単位の編集 (docs/03 §15.3・§15.4、FR-207 / FR-208 / FR-201): 選択範囲のインデント / アンインデント、
//! 行の複製・削除・上下移動、Backspace の単位。

use eve_config::{BackspaceUnit, Config};
use eve_testkit::Harness;

/// 本文を入れた無題の文書。カーソルは文書の先頭。
fn doc(text: &str) -> Harness {
    let mut h = Harness::new();
    h.paste(text);
    h.keys("ctrl+home");
    h
}

/// 行 `line` (0 始まり) の先頭へ移る。
fn at_line(h: &mut Harness, line: usize) {
    h.keys("ctrl+home");
    for _ in 0..line {
        h.keys("down");
    }
}

// ---------- インデント ----------

// verifies: FR-208
#[test]
fn 選択した各行にインデントを足し_空行は飛ばして_undo_は_1_回() {
    let mut h = doc("a\n\nb\n");
    h.keys("ctrl+a");
    h.cmd("edit.indent");
    assert_eq!(h.doc_text(), "    a\n\n    b\n");
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "a\n\nb\n", "1 回の Undo で戻る");
}

#[test]
fn 行頭で終わる選択は最後の行を含めない() {
    let mut h = doc("a\nb\nc\n");
    h.keys("shift+down shift+down");
    h.cmd("edit.indent");
    assert_eq!(h.doc_text(), "    a\n    b\nc\n");
}

// verifies: FR-208
#[test]
fn tab_と_shift_tab_は選択があれば行単位() {
    let mut h = doc("a\nb\n");
    h.keys("shift+down");
    h.keys("tab");
    assert_eq!(h.doc_text(), "    a\nb\n");
    h.keys("shift+tab");
    assert_eq!(h.doc_text(), "a\nb\n");
}

// verifies: FR-208
#[test]
fn アンインデントは_width_以内に収まる行頭空白だけを各行から取る() {
    let mut h = doc("      a\n  \tb\n  c\n");
    h.keys("ctrl+a");
    h.cmd("edit.outdent");
    assert_eq!(h.doc_text(), "  a\nb\nc\n");
}

#[test]
fn インデントの後も行全体を選んだまま() {
    let mut h = doc("a\nb\n");
    h.keys("shift+down shift+down");
    h.cmd("edit.indent");
    // 選択はまだ 2 行にまたがっている: もう一度インデントすると 2 行とも 8 桁になる
    h.cmd("edit.indent");
    assert_eq!(h.doc_text(), "        a\n        b\n");
}

#[test]
fn 選択なしの_tab_は従来どおり() {
    let mut h = doc("ab");
    h.keys("tab");
    assert_eq!(h.doc_text(), "    ab");
}

// ---------- 行の複製 ----------

#[test]
fn 行を下に複製し_カーソルは複製へ移る() {
    let mut h = doc("a\nb\n");
    h.cmd("edit.duplicate_line");
    assert_eq!(h.doc_text(), "a\na\nb\n");
    h.text("X");
    assert_eq!(h.doc_text(), "a\nXa\nb\n", "カーソルは複製の同じ位置");
}

#[test]
fn 最後の行に改行が無ければ複製の前に改行を足す() {
    let mut h = doc("a\nb");
    at_line(&mut h, 1);
    h.cmd("edit.duplicate_line");
    assert_eq!(h.doc_text(), "a\nb\nb");
}

#[test]
fn 選択した複数の行をまとめて複製する() {
    let mut h = doc("a\nb\nc\n");
    h.keys("shift+down shift+down");
    h.cmd("edit.duplicate_line");
    assert_eq!(h.doc_text(), "a\nb\na\nb\nc\n");
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "a\nb\nc\n");
}

// ---------- 行の削除 ----------

#[test]
fn 行を削除する() {
    let mut h = doc("a\nb\nc\n");
    at_line(&mut h, 1);
    h.cmd("edit.delete_line");
    assert_eq!(h.doc_text(), "a\nc\n");
    h.text("X");
    assert_eq!(h.doc_text(), "a\nXc\n", "カーソルは詰めた行の先頭");
}

#[test]
fn 改行の無い最後の行を消すと手前の改行も取り除く() {
    let mut h = doc("a\nb");
    at_line(&mut h, 1);
    h.cmd("edit.delete_line");
    assert_eq!(h.doc_text(), "a");
    let mut h = doc("only");
    h.cmd("edit.delete_line");
    assert_eq!(h.doc_text(), "");
}

#[test]
fn 選択した複数の行を削除する() {
    let mut h = doc("a\nb\nc\nd\n");
    at_line(&mut h, 1);
    h.keys("shift+down shift+down"); // b, c (d の行頭で終わる)
    h.cmd("edit.delete_line");
    assert_eq!(h.doc_text(), "a\nd\n");
}

// ---------- 行の上下移動 ----------

#[test]
fn 行を上下へ動かし_選択はついていく() {
    let mut h = doc("a\nb\nc\n");
    at_line(&mut h, 1);
    h.cmd("edit.move_line_up");
    assert_eq!(h.doc_text(), "b\na\nc\n");
    h.text("X");
    assert_eq!(
        h.doc_text(),
        "Xb\na\nc\n",
        "カーソルは動かした行についていく"
    );
    h.cmd("edit.move_line_down");
    assert_eq!(h.doc_text(), "a\nXb\nc\n");
}

#[test]
fn 端では何もしない() {
    let mut h = doc("a\nb\n");
    h.cmd("edit.move_line_up");
    assert_eq!(h.doc_text(), "a\nb\n");
    at_line(&mut h, 1);
    h.keys("down"); // 最後の空の行
    h.cmd("edit.move_line_down");
    assert_eq!(h.doc_text(), "a\nb\n");
}

#[test]
fn 改行の種類と改行の無い最後の行は位置のまま() {
    let mut h = doc("a\nb");
    h.cmd("edit.move_line_down");
    assert_eq!(h.doc_text(), "b\na", "最後の行に改行は付かない");
    // 改行の位置は動かさない (内容だけを入れ替える)。CRLF の文書はファイルから開く (貼り付けは改行を正規化する)
    let mut h = Harness::new().with_file("/w/crlf.txt", b"a\r\nb\r\nc");
    h.open("/w/crlf.txt");
    h.run_jobs();
    h.cmd("edit.move_line_down");
    assert_eq!(h.doc_text(), "b\r\na\r\nc");
}

#[test]
fn 選択した複数の行を動かす() {
    let mut h = doc("a\nb\nc\nd\n");
    h.keys("shift+down shift+down"); // a, b
    h.cmd("edit.move_line_down");
    assert_eq!(h.doc_text(), "c\na\nb\nd\n");
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "a\nb\nc\nd\n");
}

#[test]
fn 既定のキーで動く() {
    let mut h = doc("a\nb\n");
    h.keys("ctrl+k down");
    assert_eq!(h.doc_text(), "b\na\n");
    h.keys("ctrl+k up");
    assert_eq!(h.doc_text(), "a\nb\n");
    h.keys("ctrl+k d");
    assert_eq!(h.doc_text(), "a\na\nb\n");
    h.keys("ctrl+k l");
    assert_eq!(h.doc_text(), "a\nb\n");
}

// ---------- Backspace の単位 ----------

#[test]
fn backspace_は書記素単位が既定で_codepoint_に切り替えられる() {
    // "e" + 結合アクセント (2 コードポイント、1 書記素)
    let mut h = doc("e\u{0301}");
    h.keys("end");
    h.keys("backspace");
    assert_eq!(h.doc_text(), "", "既定は書記素単位");

    let mut h = doc("e\u{0301}");
    h.editor.state_mut().set_config(Config {
        backspace_unit: BackspaceUnit::Codepoint,
        ..Config::default()
    });
    h.keys("end");
    h.keys("backspace");
    assert_eq!(
        h.doc_text(),
        "e",
        "コードポイント単位なら結合文字だけを消す"
    );
}
