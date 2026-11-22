//! 複数カーソルでの既存の編集とクリップボード (FR-211、docs/03 §4.5〜§4.5.5、01 §5.5 の FR-211 AC-1 / AC-2) のシナリオ
//! (03 §4.7.1 の L3。M32 の段 2)。カーソルを足すコマンドは段 3 なので、試験は View に複数の Range を直接与える。
//!
//! 文書は `|` (空のカーソル) と `«` `»` (前向きの選択。`»` が head) と `‹` `›` (後ろ向きの選択。`‹` が head) を含む文字列で
//! 書く。主の Range は `*` を直前に置いたもの (無ければ最後の Range)。

use eve_testkit::Harness;
use eve_text::{ByteOff, MAX_CURSORS, Range, Selection};
use eve_view::{ClipboardText, Effect, StatusKind, ViewId};

/// 印を読んで (本文, Range の並び, 主の番号) にする。
fn parse(marked: &str) -> (String, Vec<Range>, usize) {
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
fn render(text: &str, sel: &Selection) -> String {
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

fn view(h: &Harness) -> ViewId {
    let st = h.editor.state();
    st.window_view(h.window()).expect("View がある")
}

fn selection(h: &Harness) -> Selection {
    h.editor
        .state()
        .view(view(h))
        .expect("View がある")
        .selection
        .clone()
}

fn set_selection(h: &mut Harness, ranges: Vec<Range>, primary: usize) {
    let v = view(h);
    let sel = Selection::new(ranges, primary).expect("Selection を作れる");
    h.editor.state_mut().set_selection(v, sel);
}

/// 無題の文書に本文を入れ、印の Range を置いた Harness。
fn scene(marked: &str) -> Harness {
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
fn shown(h: &Harness) -> String {
    render(&h.doc_text(), &selection(h))
}

fn set_clipboard(h: &Harness) -> Vec<ClipboardText> {
    h.effects()
        .iter()
        .filter_map(|e| match e {
            Effect::SetClipboard { content, .. } => Some(content.clone()),
            _ => None,
        })
        .collect()
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

fn status_selection(h: &mut Harness) -> Option<(String, Option<String>, u8)> {
    let f = h.frame();
    f.status
        .items
        .iter()
        .find(|i| i.kind == StatusKind::Selection)
        .map(|i| {
            let t = |c: &eve_view::CellText| eve_testkit::helpers::cell_text(c);
            (t(&i.long), i.short.as_ref().map(t), i.priority)
        })
}

#[test]
fn 印の読み書きは往復する() {
    let (text, ranges, primary) = parse("a|b«cd»e‹fg›*h|");
    assert_eq!(text, "abcdefgh");
    assert_eq!(primary, 3);
    let sel = Selection::new(ranges, primary).unwrap();
    assert_eq!(render(&text, &sel), "a|b«cd»e‹fg›h|");
}

// ---------- 入力・削除・改行・インデント (AC-1) ----------

// verifies: AC-FR-211-1
#[test]
fn 文字の入力は全カーソルに入り_1_回の_undo_で本文と複数の選択が戻る() {
    let mut h = scene("a|b\nc«de»f\n|g");
    h.text("X");
    assert_eq!(shown(&h), "aX|b\ncX|f\nX|g");
    h.keys("ctrl+z");
    assert_eq!(
        shown(&h),
        "a|b\nc«de»f\n|g",
        "1 回の Undo で全カーソルの編集と複数の選択が戻る"
    );
    h.keys("ctrl+y");
    assert_eq!(shown(&h), "aX|b\ncX|f\nX|g", "Redo で編集の後の選択に戻る");
}

// verifies: AC-FR-211-1
#[test]
fn 続けて打った文字は全カーソルで_1_つの_undo_にまとまる() {
    let mut h = scene("|a\n|b");
    h.text("x");
    h.text("y");
    h.text("z");
    assert_eq!(shown(&h), "xyz|a\nxyz|b");
    h.keys("ctrl+z");
    assert_eq!(shown(&h), "|a\n|b", "R7 の結合は全 Range で条件を満たす");
}

#[test]
fn range_の数が変わったら続けて消しても別の_undo_になる() {
    // 先頭のカーソルの後退は何も消さないので、変更のまとまりは 1 つ (Range は 2 つ)
    let mut h = scene("|ab|");
    h.keys("backspace");
    assert_eq!(shown(&h), "|a|");
    // 主だけを残す (Range の数が 2 → 1。まとまりの数は同じ 1 つ)
    let primary = *selection(&h).primary();
    set_selection(&mut h, vec![primary], 0);
    h.keys("backspace");
    assert_eq!(h.doc_text(), "");
    h.keys("ctrl+z");
    assert_eq!(
        h.doc_text(),
        "a",
        "まとまりの数が同じでも、Range の数が違えば新しいノード (MCUR-7)"
    );
}

#[test]
fn 併合で_range_が減った後の続けての削除は_1_つの_undo_にまとまる() {
    // 2 つのカーソルの後退で消す範囲が接し、カーソルが 1 つにまとまる。記録する編集の後の選択も 1 つなので、次の後退と結合する
    let mut h = scene("ab|c|d");
    h.keys("backspace");
    assert_eq!(shown(&h), "a|d");
    h.keys("backspace");
    assert_eq!(shown(&h), "|d");
    h.keys("ctrl+z");
    assert_eq!(shown(&h), "ab|c|d", "1 回の Undo で 2 回の後退が戻る");
}

#[test]
fn 結合の_4_kib_は_range_ごとに数える() {
    // 2 つのカーソルで 3 KiB ずつ打つ: 合計 (6 KiB) は 4 KiB を超えるが、Range ごとは超えないので結合する
    let mut h = scene("|\n|");
    let chunk = "a".repeat(1024);
    for _ in 0..3 {
        h.text(&chunk);
    }
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "\n", "Range ごとに 3 KiB なので 1 つのノード");
    // Range ごとに 4 KiB を超えると区切る
    let mut h = scene("|\n|");
    for _ in 0..5 {
        h.text(&chunk);
    }
    h.keys("ctrl+z");
    assert_eq!(
        h.doc_text(),
        format!("{0}\n{0}", "a".repeat(4096)),
        "5 回目は Range ごとに 4 KiB を超えるので新しいノード"
    );
}

// verifies: AC-FR-211-1
#[test]
fn 選択のある_range_は入力で置き換え_空のカーソルには挿入する() {
    let mut h = scene("«ab» cd |ef ‹gh›");
    h.text("Z");
    assert_eq!(shown(&h), "Z| cd Z|ef Z|");
}

// verifies: AC-FR-211-1
#[test]
fn 後退と前方の削除は各カーソルで行い_重なったカーソルは_1_つになる() {
    let mut h = scene("ab|cd|ef");
    h.keys("backspace");
    assert_eq!(shown(&h), "a|c|ef");
    // 隣り合う 2 つのカーソルの後退: 消す範囲が接し、カーソルが同じ位置で 1 つになる
    let mut h = scene("a|b|c");
    h.keys("backspace");
    assert_eq!(shown(&h), "|c");
    assert_eq!(selection(&h).len(), 1, "重なったカーソルは 1 つにまとまる");
    h.keys("ctrl+z");
    assert_eq!(shown(&h), "a|b|c", "Undo で 2 つの Range が戻る");
    let mut h = scene("|ab|cd");
    h.keys("delete");
    assert_eq!(shown(&h), "|b|d");
}

#[test]
fn 単語の削除が先行の範囲に重なると後続を切り詰める() {
    // 1 つ目は選択 (cd を消す)、2 つ目のカーソルの単語の削除は単語の先頭 (a) まで届くが、1 つ目の後ろから消す
    let mut h = scene("ab«cd»ef|gh");
    h.keys("ctrl+backspace");
    assert_eq!(shown(&h), "ab|gh");
    assert_eq!(selection(&h).len(), 1);
    h.keys("ctrl+z");
    assert_eq!(shown(&h), "ab«cd»ef|gh");
}

// verifies: AC-FR-211-1
#[test]
fn 改行は各カーソルの行の行頭空白を複製する() {
    let mut h = scene("  a|b\n\tc|d\ne|");
    h.keys("enter");
    assert_eq!(shown(&h), "  a\n  |b\n\tc\n\t|d\ne\n|");
    h.keys("ctrl+z");
    assert_eq!(shown(&h), "  a|b\n\tc|d\ne|");
}

// verifies: AC-FR-211-1
#[test]
fn インデントは全部空なら各カーソルに_どれかに選択があれば行の和集合に_1_回() {
    let mut h = scene("a|b\nc|d|e");
    h.keys("tab");
    // 各カーソルの表示列から次の停止位置まで (1 → 4、1 → 4、2 → 4)
    assert_eq!(shown(&h), "a   |b\nc   |d  |e");
    // 選択があれば、触れる行の和集合を行単位で (同じ行は 1 回)
    let mut h = scene("«a»b\nc|d|e\nf");
    h.keys("tab");
    assert_eq!(h.doc_text(), "    ab\n    cde\nf");
    h.keys("ctrl+z");
    assert_eq!(shown(&h), "«a»b\nc|d|e\nf");
}

// verifies: AC-FR-211-1
#[test]
fn 逆インデントは全部空なら各カーソルの行を_同じ行は_1_回() {
    let mut h = scene("        a|b|c\n    d|");
    h.keys("shift+tab");
    assert_eq!(h.doc_text(), "    abc\nd");
    assert_eq!(selection(&h).len(), 3, "カーソルは行頭空白の削除を追う");
}

// ---------- 行の操作 (§4.5.2、§15.4) ----------

#[test]
fn 行の複製はブロックごとに行い_各_range_は自分のブロックの複製に移る() {
    // 2 つ目の選択は次の行の行頭で終わる (その行を含めない。終わりの行頭も、選択の始まりのブロックの複製に付いていく)
    let mut h = scene("a|\nb\n«c\n»d\ne");
    h.cmd("edit.duplicate_line");
    assert_eq!(h.doc_text(), "a\na\nb\nc\nc\nd\ne");
    assert_eq!(shown(&h), "a\na|\nb\nc\n«c\n»d\ne");
}

#[test]
fn 行の削除はブロックごとに_1_回() {
    let mut h = scene("a|\nb|\nc\nd|");
    h.cmd("edit.delete_line");
    assert_eq!(shown(&h), "|c|", "各ブロックの削除の位置に 1 つずつ");
    h.keys("ctrl+z");
    assert_eq!(shown(&h), "a|\nb|\nc\nd|");
}

#[test]
fn 行の移動は動けないブロックが_1_つでもあれば全体を何もしない() {
    let mut h = scene("|a\nb\n|c\nd");
    let rev = h.editor.state().document(h.doc_id()).unwrap().revision();
    h.cmd("edit.move_line_up");
    assert_eq!(shown(&h), "|a\nb\n|c\nd", "先頭の行のブロックが動けない");
    assert_eq!(
        h.editor.state().document(h.doc_id()).unwrap().revision(),
        rev,
        "文書を変えない"
    );
    h.cmd("edit.move_line_down");
    assert_eq!(
        shown(&h),
        "b\n|a\nd\n|c",
        "どれも動けるなら各ブロックを動かす"
    );
}

// ---------- クリップボード (AC-2) ----------

// verifies: AC-FR-211-2
#[test]
fn コピーは空でない_range_を文書の順に既定の改行で連結し_空のカーソルは数えない() {
    let mut h = scene("«ab» c| «d\ne»");
    h.keys("ctrl+c");
    assert_eq!(
        set_clipboard(&h),
        vec![ClipboardText::Inline("ab\nd\ne".to_owned())],
        "末尾に改行を足さない"
    );
    // 全部空なら何もしない
    let mut h = scene("a|b|c");
    h.keys("ctrl+c");
    assert_eq!(set_clipboard(&h), Vec::<ClipboardText>::new());
}

// verifies: AC-FR-211-2
#[test]
fn crlf_の文書のコピーは_crlf_で連結する() {
    let mut h = eve_testkit::helpers::opened("/w/a.txt", b"ab\r\ncd\r\n");
    set_selection(
        &mut h,
        vec![
            Range::new(ByteOff(0), ByteOff(2)),
            Range::new(ByteOff(4), ByteOff(6)),
        ],
        1,
    );
    h.clear_log();
    h.keys("ctrl+c");
    assert_eq!(
        set_clipboard(&h),
        vec![ClipboardText::Inline("ab\r\ncd".to_owned())]
    );
}

// verifies: AC-FR-211-2
#[test]
fn 切り取りは空でない_range_を全部消し_1_回の_undo_で戻る() {
    let mut h = scene("«ab»c|d«ef»");
    h.keys("ctrl+x");
    assert_eq!(
        set_clipboard(&h),
        vec![ClipboardText::Inline("ab\nef".to_owned())]
    );
    assert_eq!(shown(&h), "|c|d|");
    h.keys("ctrl+z");
    assert_eq!(shown(&h), "«ab»c|d«ef»");
}

// verifies: AC-FR-211-2
#[test]
fn 同じ数のカーソルでのコピーと貼り付けは片を_1_つずつ戻し_改行を含む片も戻る() {
    let mut h = scene("«a\nb» x «c» y");
    h.keys("ctrl+c");
    let copied = "a\nb\nc".to_owned();
    // 別の 2 つのカーソルへ: 行の数 (3) とカーソルの数 (2) は違うが、内部レジスタの片 (2) が当たる
    set_selection(
        &mut h,
        vec![Range::point(ByteOff(3)), Range::point(ByteOff(7))],
        0,
    );
    paste_via_command(&mut h, Some(&copied));
    assert_eq!(shown(&h), "a\nba\nb| x cc| y");
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "a\nb x c y", "1 回の Undo で戻る");
}

// verifies: AC-FR-211-2
#[test]
fn 端末の貼り付けでも_改行が_cr_になっても片の照合が当たる() {
    let mut h = scene("«a\nb» x «c» y");
    h.keys("ctrl+c");
    set_selection(
        &mut h,
        vec![Range::point(ByteOff(3)), Range::point(ByteOff(7))],
        0,
    );
    // Windows Terminal は貼り付けの改行を CR にして送る (04 §6.6)
    h.paste("a\rb\rc");
    assert_eq!(h.doc_text(), "a\nba\nb x cc y");
}

// verifies: AC-FR-211-2
#[test]
fn 読み取れなかったときは内部レジスタの片を使う() {
    let mut h = scene("«ab» «cd»");
    h.keys("ctrl+c");
    set_selection(
        &mut h,
        vec![Range::point(ByteOff(0)), Range::point(ByteOff(5))],
        0,
    );
    paste_via_command(&mut h, None);
    assert_eq!(h.doc_text(), "abab cdcd");
}

// verifies: AC-FR-211-2
#[test]
fn 中身が変わっていれば片を使わず_行の数がカーソルの数と同じなら_1_行ずつ() {
    let mut h = scene("«ab» «cd»");
    h.keys("ctrl+c");
    set_selection(
        &mut h,
        vec![Range::point(ByteOff(0)), Range::point(ByteOff(3))],
        1,
    );
    // ほかのアプリでコピーした 2 行 (末尾の改行 1 つは行を増やさない)
    paste_via_command(&mut h, Some("X\r\nY\r\n"));
    assert_eq!(shown(&h), "X|ab Y|cd");
}

// verifies: AC-FR-211-2
#[test]
fn 行の数がカーソルの数と違えば全文を各カーソルへ() {
    let mut h = scene("|a |b");
    h.paste("X\n\nY");
    assert_eq!(
        h.doc_text(),
        "X\n\nYa X\n\nYb",
        "`X⏎⏎Y` は 3 行なので 2 つのカーソルには全文"
    );
    let mut h = scene("|a |b |c");
    h.paste("X\n\nY");
    assert_eq!(
        shown(&h),
        "X|a |b Y|c",
        "3 行と 3 つのカーソル (空の行も 1 行)"
    );
}

#[test]
fn 空の片を入れたカーソルも文書の順のまま先行の片の後ろに置く() {
    // 主の空のカーソルが選択の端に残る (N4)。2 行 (`X` と空の行) を 1 行ずつ: 選択は `X` に、カーソルは空の行 (何も
    // 入れない) で、カーソルは選択に入れた `X` の後ろ (前に回り込まない)
    let mut h = scene("«ab»*|c");
    h.paste("X\n\n");
    assert_eq!(shown(&h), "X|c");
}

#[test]
fn カーソルが_1_つなら貼り付けは今と同じ() {
    let mut h = scene("a|b");
    h.paste("X\nY\n");
    assert_eq!(shown(&h), "aX\nY\n|b");
}

// verifies: AC-FR-211-2
#[test]
fn 入れる合計が上限を超えたら何もせずに知らせる() {
    let limit = eve_core::MULTI_PASTE_MAX_BYTES;
    let mut h = scene("|a|");
    let rev = h.editor.state().document(h.doc_id()).unwrap().revision();
    // 全文 × 2 = 上限 + 2
    h.paste(&"x".repeat(limit / 2 + 1));
    assert_eq!(h.doc_text(), "a", "何も入れない");
    assert_eq!(
        h.editor.state().document(h.doc_id()).unwrap().revision(),
        rev
    );
    assert!(
        h.messages()
            .iter()
            .any(|k| k == "info.paste.multi_too_large"),
        "知らせる: {:?}",
        h.messages()
    );
}

// verifies: AC-FR-211-2
#[test]
fn 入れる合計が上限ちょうどなら入れる() {
    let limit = eve_core::MULTI_PASTE_MAX_BYTES;
    let mut h = scene("|a|");
    h.paste(&"x".repeat(limit / 2));
    assert_eq!(h.doc_text().len(), limit + 1);
}

// ---------- ステータスバー・when・ほかの機能 ----------

#[test]
fn ステータスバーはカーソルが_2_つ以上ならカーソルの数を出す() {
    let mut h = scene("«ab» |c");
    assert_eq!(
        status_selection(&mut h),
        Some(("カーソル 2 個".to_owned(), Some("2カーソル".to_owned()), 4)),
        "選択情報の代わりにカーソルの数 (優先度 4)"
    );
    let mut h = scene("«ab» c");
    assert_eq!(
        status_selection(&mut h),
        Some(("(2 文字選択)".to_owned(), None, 2)),
        "1 つなら今と同じ選択情報"
    );
}

#[test]
fn selection_empty_は全_range_が空のときだけ真() {
    let mut h = scene("a| «b»");
    assert_eq!(h.cmd("clipboard.copy"), eve_core::CommandResult::Done);
    let mut h = scene("a| |b");
    assert_eq!(
        h.cmd("clipboard.copy"),
        eve_core::CommandResult::Rejected(eve_core::RejectReason::NotAvailable)
    );
}

#[test]
fn 一つにするコマンドは実行の後に_range_が_1_つ() {
    let mut h = scene("a|b|c");
    h.keys("ctrl+a");
    assert_eq!(selection(&h).len(), 1, "select.all");
    let mut h = scene("a|b\nc|d");
    h.cmd_with(
        "cursor.goto_line",
        eve_core::Args::new().with_int("line", 2),
    );
    assert_eq!(shown(&h), "ab\n|cd", "cursor.goto_line");
}

#[test]
fn 移動は各_range_を動かして併合する() {
    let mut h = scene("a|b|c");
    h.keys("right");
    assert_eq!(shown(&h), "ab|c|");
    h.keys("right");
    assert_eq!(shown(&h), "abc|", "同じ位置になったカーソルは 1 つ");
}

#[test]
fn 検索バーは選択範囲内の範囲に全_range_を使い_閉じれば複数の_range_が残り_次へで_1_つになる() {
    let marked = "«foo 1\n»foo 2\n«foo 3\n»foo 4\n";
    let mut h = scene(marked);
    h.keys("ctrl+f");
    h.text("foo");
    h.keys("alt+l");
    h.run_jobs();
    let count = h
        .frame()
        .search
        .and_then(|b| b.count_text)
        .map(|t| eve_testkit::helpers::cell_text(&t))
        .unwrap_or_default();
    assert!(
        count.ends_with('2'),
        "範囲は開いたときの空でない Range の全部: {count}"
    );
    h.keys("escape");
    assert_eq!(
        shown(&h),
        marked,
        "強調は選択を変えないので、閉じれば複数の Range が残る"
    );
    h.keys("ctrl+f");
    h.text("foo");
    h.keys("enter");
    assert_eq!(selection(&h).len(), 1, "次の一致へ移ると 1 つ (§4.5.2)");
}

#[test]
fn すべて置換は全_range_を写して保つ() {
    let mut h = scene("a|b foo c|d foo");
    h.keys("ctrl+r");
    h.text("foo");
    h.keys("tab");
    h.text("XY");
    h.keys("alt+a");
    h.run_jobs();
    assert_eq!(shown(&h), "a|b XY c|d XY");
}

#[test]
fn すべて置換は一致の始まりの空のカーソルを置換の後ろへ写す() {
    // 発行 View の写像 (MapPolicy::Own): 置き換えた範囲の始まりの点は、入れた文字列の後ろへ。Other なら前に留まる
    let mut h = scene("a |foo b |foo");
    h.keys("ctrl+r");
    h.text("foo");
    h.keys("tab");
    h.text("XY");
    h.keys("alt+a");
    h.run_jobs();
    assert_eq!(shown(&h), "a XY| b XY|");
}

// verifies: AC-FR-211-2
#[test]
fn 切り取った片は同じ数のカーソルへ_1_つずつ戻る() {
    let mut h = scene("«a\nb» x «c»");
    h.keys("ctrl+x");
    assert_eq!(shown(&h), "| x |");
    paste_via_command(&mut h, Some("a\nb\nc"));
    assert_eq!(shown(&h), "a\nb| x c|", "改行を含む片も 1 段目で戻る");
}

#[test]
fn 読み取り専用の文書への端末の貼り付けは上限を調べずに断る() {
    let mut h = Harness::new().with_file("/w/ro.txt", b"ab\n");
    h.fs.set_readonly("/w/ro.txt", true);
    h.open("/w/ro.txt");
    h.run_jobs();
    set_selection(
        &mut h,
        vec![Range::point(ByteOff(0)), Range::point(ByteOff(2))],
        0,
    );
    h.clear_log();
    h.paste(&"x".repeat(eve_core::MULTI_PASTE_MAX_BYTES / 2 + 1));
    assert_eq!(h.doc_text(), "ab\n");
    assert!(
        !h.messages()
            .iter()
            .any(|k| k == "info.paste.multi_too_large"),
        "読み取り専用なので量の知らせは出さない: {:?}",
        h.messages()
    );
}

#[test]
fn 長い行の多数のカーソルでも移動とインデントと再読込が各カーソルの位置になる() {
    // 1 行 (長い行の表示単位を覚える 4 KiB 以上) に 1,000 個のカーソル
    let line = "ab".repeat(4_000);
    let mut h = eve_testkit::helpers::opened("/w/long.txt", format!("{line}\n{line}\n").as_bytes());
    let ranges: Vec<Range> = (0..1_000).map(|i| Range::point(ByteOff(i * 8))).collect();
    set_selection(&mut h, ranges.clone(), 0);
    h.keys("down");
    let heads: Vec<usize> = selection(&h).iter().map(|r| r.head.0).collect();
    let want: Vec<usize> = (0..1_000).map(|i| 8_001 + i * 8).collect();
    assert_eq!(heads, want, "同じ表示列の次の行へ");
    h.keys("up");
    h.keys("tab");
    let heads: Vec<usize> = selection(&h).iter().map(|r| r.head.0).collect();
    let want: Vec<usize> = (0..1_000).map(|i| i * 8 + (i + 1) * 4).collect();
    assert_eq!(
        heads, want,
        "各カーソルの表示列から次の停止位置 (4 の倍数) まで"
    );
    // 再読込: 各 head の行:桁
    set_selection(&mut h, ranges, 0);
    h.fs.add_file("/w/long.txt", format!("{line}\n").as_bytes());
    h.cmd("file.reload");
    h.run_jobs();
    let heads: Vec<usize> = selection(&h).iter().map(|r| r.head.0).collect();
    assert_eq!(heads, (0..1_000).map(|i| i * 8).collect::<Vec<_>>());
}

#[test]
fn ほかのペインからの編集は全_range_を写して保つ() {
    let mut h = scene("ab|cd|ef");
    let first = view(&h);
    h.cmd("pane.split_right");
    let second = view(&h);
    assert_ne!(first, second);
    h.editor
        .state_mut()
        .set_selection(second, Selection::point(ByteOff(0)));
    h.text("XY");
    let sel = h.editor.state().view(first).unwrap().selection.clone();
    assert_eq!(
        render(&h.doc_text(), &sel),
        "XYab|cd|ef",
        "ほかの View の編集は MapPolicy::Other で写す"
    );
}

#[test]
fn 外部変更の再読込は各カーソルを同じ行と桁に写す() {
    let mut h = eve_testkit::helpers::opened("/w/a.txt", b"abc\ndef\nghi\n");
    set_selection(
        &mut h,
        vec![Range::point(ByteOff(1)), Range::new(ByteOff(4), ByteOff(6))],
        0,
    );
    h.fs.add_file("/w/a.txt", b"ABCD\nEFGH\n");
    h.cmd("file.reload");
    h.run_jobs();
    assert_eq!(shown(&h), "A|BCD\nEF|GH\n", "head の行:桁の空のカーソル");
    h.keys("ctrl+z");
    assert_eq!(
        shown(&h),
        "a|bc\n«de»f\nghi\n",
        "Undo で元の複数の Range に戻る"
    );
}

#[test]
fn 上限ちょうどのカーソルで入力でき_undo_で戻る() {
    let text = "a\n".repeat(MAX_CURSORS);
    let mut h = Harness::new();
    h.paste(&text);
    let ranges = (0..MAX_CURSORS)
        .map(|i| Range::point(ByteOff(i * 2)))
        .collect();
    set_selection(&mut h, ranges, MAX_CURSORS - 1);
    assert_eq!(selection(&h).len(), MAX_CURSORS);
    h.text("x");
    assert_eq!(h.doc_text(), "xa\n".repeat(MAX_CURSORS));
    assert_eq!(selection(&h).len(), MAX_CURSORS);
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), text);
    assert_eq!(selection(&h).len(), MAX_CURSORS);
}
