//! すべて置換 (docs/03 §12.5、01 FR-403、04 §8.4): 置換欄・1 Undo 単位・大きな文書のジョブ。

use eve_testkit::Harness;
use eve_view::BarFocus;

fn text_of(t: &eve_view::CellText) -> String {
    t.spans.iter().map(|s| s.text.to_string()).collect()
}

fn message(h: &mut Harness) -> String {
    h.frame()
        .messages
        .current
        .map(|m| text_of(&m.text))
        .unwrap_or_default()
}

/// 検索語と置換文字列を入れた状態にする (検索欄 → Tab → 置換欄)。
fn setup(h: &mut Harness, find: &str, with: &str) {
    h.keys("ctrl+r");
    h.text(find);
    h.keys("tab");
    h.text(with);
}

#[test]
fn ctrl_r_で置換欄つきの検索バーが開き_編集面が_2_行縮む() {
    let mut h = Harness::new().with_size(40, 10);
    let rows = h.frame().size.rows;
    h.keys("ctrl+r");
    let f = h.frame();
    assert_eq!(f.size.rows, rows - 2);
    let b = f.search.unwrap();
    assert!(b.replace.is_some());
    assert_eq!(b.focus, BarFocus::Search, "検索語が空なら検索欄");
    h.text("a");
    h.keys("ctrl+r");
    assert_eq!(h.frame().search.unwrap().focus, BarFocus::Replace);
}

#[test]
fn tab_で_検索欄_置換欄_ボタンを巡回する() {
    let mut h = Harness::new().with_size(40, 10);
    h.keys("ctrl+r");
    let focus = |h: &mut Harness| h.frame().search.unwrap().focus;
    h.keys("tab");
    assert_eq!(focus(&mut h), BarFocus::Replace);
    h.keys("tab");
    assert_eq!(focus(&mut h), BarFocus::Button);
    h.keys("tab");
    assert_eq!(focus(&mut h), BarFocus::Search);
    h.keys("shift+tab");
    assert_eq!(focus(&mut h), BarFocus::Button);
}

#[test]
fn すべて置換は一度の_undo_で戻り_件数を知らせる() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("foo bar foo\nfoo");
    setup(&mut h, "foo", "X");
    h.keys("alt+a");
    assert_eq!(h.doc_text(), "X bar X\nX");
    assert_eq!(message(&mut h), "3 件を置換しました (元に戻すで戻せます)");
    assert!(h.is_dirty());
    h.keys("esc");
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "foo bar foo\nfoo", "1 回の Undo で全部戻る");
}

#[test]
fn ボタンで_enter_でも置換する() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("aa");
    setup(&mut h, "a", "b");
    h.keys("tab enter");
    assert_eq!(h.doc_text(), "bb");
}

#[test]
fn 置換文字列は展開せず_空なら削除し_日本語も扱う() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("日本語 日本");
    setup(&mut h, "日本", "$1\\n");
    h.keys("alt+a");
    assert_eq!(h.doc_text(), "$1\\n語 $1\\n");
    let mut h = Harness::new().with_size(60, 10);
    h.text("a-b-c");
    setup(&mut h, "-", "");
    h.keys("alt+a");
    assert_eq!(h.doc_text(), "abc");
}

#[test]
fn 大小の区別は検索バーのトグルに従う() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("Foo foo");
    setup(&mut h, "foo", "x");
    h.keys("alt+a");
    assert_eq!(h.doc_text(), "x x", "smart + 小文字は区別しない");
    let mut h = Harness::new().with_size(60, 10);
    h.text("Foo foo");
    h.keys("ctrl+r alt+c alt+c");
    h.keys("alt+c");
    h.keys("alt+c");
    h.text("Foo");
    h.keys("tab");
    h.text("x");
    h.keys("alt+a");
    assert_eq!(h.doc_text(), "x foo", "smart + 大文字を含めば区別する");
}

#[test]
fn 検索語が空なら案内し_一致が無ければ文書を変えない() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("abc");
    h.keys("ctrl+r tab");
    h.text("x");
    h.keys("alt+a");
    assert_eq!(message(&mut h), "検索する文字列を入力してください");
    assert_eq!(h.doc_text(), "abc");
    let mut h = Harness::new().with_size(60, 10);
    h.text("abc");
    setup(&mut h, "zzz", "x");
    h.keys("alt+a");
    assert_eq!(message(&mut h), "置換する箇所はありません");
    assert_eq!(h.doc_text(), "abc");
}

#[test]
fn 置換欄が開いていなければ_alt_a_は何もしない() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("aa");
    h.keys("ctrl+f");
    h.text("a");
    h.keys("alt+a");
    assert_eq!(h.doc_text(), "aa");
}

#[test]
fn 置換の後も検索バーは開いたままで_再検索する() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("ab ab");
    setup(&mut h, "ab", "abab");
    h.keys("alt+a");
    h.run_jobs();
    assert_eq!(h.doc_text(), "abab abab");
    let b = h.frame().search.unwrap();
    assert!(text_of(&b.count_text.unwrap()).ends_with("/4"));
}

fn big_doc() -> String {
    // 1 MiB を超える
    "foo bar baz\n".repeat(90_000)
}

#[test]
fn 大きな文書はジョブで置換し_入力は止めない() {
    let mut h = Harness::new().with_size(60, 10);
    h.paste(&big_doc());
    setup(&mut h, "foo", "FOOO");
    h.keys("alt+a");
    assert!(h.doc_text().starts_with("foo bar"), "まだ置換していない");
    h.run_jobs();
    assert!(h.doc_text().starts_with("FOOO bar baz\nFOOO"));
    assert_eq!(
        message(&mut h),
        "90000 件を置換しました (元に戻すで戻せます)"
    );
    h.keys("esc ctrl+z");
    assert!(h.doc_text().starts_with("foo bar baz\nfoo"));
}

#[test]
fn ジョブの間に文書が変わったら適用せず中止を知らせる() {
    use eve_core::{Origin, Transaction};
    use eve_text::{ByteOff, ByteRange, ChangeSet, Edit, TextRead};
    let mut h = Harness::new().with_size(60, 10);
    h.paste(&big_doc());
    setup(&mut h, "foo", "x");
    h.keys("alt+a");
    // ジョブが走る前に、別の経路 (ここでは API) で文書を変える
    let doc = h.doc_id();
    let cs = {
        let st = h.editor.state();
        let t = st.document(doc).unwrap().text();
        assert!(t.len_bytes() > 1 << 20);
        ChangeSet::from_sorted_edits(
            t,
            [Edit {
                range: ByteRange::new(ByteOff(0), ByteOff(0)),
                insert: "Z".into(),
            }],
        )
        .unwrap()
    };
    h.editor
        .state_mut()
        .apply_transaction(doc, Transaction::new(cs, Origin::User, eve_core::Tick(0)))
        .unwrap();
    h.run_jobs();
    assert!(h.doc_text().starts_with("Zfoo bar"), "置換していない");
    assert_eq!(
        message(&mut h),
        "編集が続いたため、すべて置換を中止しました"
    );
}

#[test]
fn バーを閉じると置換のジョブは取り消される() {
    let mut h = Harness::new().with_size(60, 10);
    h.paste(&big_doc());
    setup(&mut h, "foo", "x");
    h.keys("alt+a");
    h.keys("esc");
    h.run_jobs();
    assert!(h.doc_text().starts_with("foo bar"));
}

// ---- v0.2: 正規表現・単語単位の置換、1 件置換 (03 §12.5) ----

#[test]
fn 正規表現の置換は_キャプチャを展開する() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("foo=1 bar=2");
    h.keys("ctrl+r");
    h.keys("alt+r");
    h.text(r"(\w+)=(\d)");
    h.keys("tab");
    h.text("$2:${1}!");
    h.keys("alt+a");
    assert_eq!(h.doc_text(), "1:foo! 2:bar!");
    assert_eq!(message(&mut h), "2 件を置換しました (元に戻すで戻せます)");
    h.keys("esc");
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "foo=1 bar=2", "1 回の Undo で戻る");
}

#[test]
fn 正規表現の置換は_改行と_ドルを扱う() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("a,b,c");
    h.keys("ctrl+r");
    h.keys("alt+r");
    h.text(",");
    h.keys("tab");
    h.text(r"\n");
    h.keys("alt+a");
    assert_eq!(h.doc_text(), "a\nb\nc", "\n は既定の改行 (LF)");

    let mut h = Harness::new().with_size(60, 10);
    h.text("price 5");
    h.keys("ctrl+r");
    h.keys("alt+r");
    h.text(r"\d");
    h.keys("tab");
    h.text("$$ 100");
    h.keys("alt+a");
    assert_eq!(h.doc_text(), "price $ 100", "$$ は $");

    let mut h = Harness::new().with_size(60, 10);
    h.text("a-b");
    h.keys("ctrl+r");
    h.keys("alt+r");
    h.text("-");
    h.keys("tab");
    h.text(r"\\n"); // バックスラッシュ 2 つと n
    h.keys("alt+a");
    assert_eq!(
        h.doc_text(),
        r"a\nb",
        "バックスラッシュ 2 つは 1 つになり、n は文字のまま"
    );
}

#[test]
fn 単語単位のすべて置換() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("foo foobar barfoo foo");
    h.keys("ctrl+r");
    h.keys("alt+w");
    h.text("foo");
    h.keys("tab");
    h.text("X");
    h.keys("alt+a");
    assert_eq!(h.doc_text(), "X foobar barfoo X");
}

#[test]
fn 置換欄の_enter_は_1_件置換で_次の一致へ進み_置換ごとに_undo_できる() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("a a a");
    h.keys("ctrl+home");
    setup(&mut h, "a", "b");
    h.keys("enter");
    assert_eq!(h.doc_text(), "b a a");
    h.keys("enter");
    assert_eq!(h.doc_text(), "b b a");
    h.keys("enter");
    assert_eq!(h.doc_text(), "b b b");
    assert_eq!(h.frame().search.unwrap().focus, BarFocus::Replace);
    h.keys("enter");
    assert_eq!(message(&mut h), "置換する箇所はありません");
    h.keys("esc");
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "b b a", "1 件ずつ戻る");
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "b a a");
}

#[test]
fn 検索欄の_enter_は従来どおり次の一致へ移るだけ() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("a a");
    h.keys("ctrl+home");
    setup(&mut h, "a", "b");
    h.keys("shift+tab");
    h.keys("enter");
    assert_eq!(h.doc_text(), "a a", "検索欄では置換しない");
}

#[test]
fn 選択した一致を置換し_正規表現ではキャプチャを展開する() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("k1=v1 k2=v2");
    h.keys("ctrl+home");
    h.keys("ctrl+r");
    h.keys("alt+r");
    h.text(r"(\w+)=(\w+)");
    h.keys("tab");
    h.text("$2=$1");
    h.keys("shift+tab");
    h.keys("enter"); // 1 つ目の一致を選択
    h.keys("enter"); // 2 つ目へ (選択)
    h.keys("tab");
    h.keys("enter"); // 選択している 2 つ目を置換
    assert_eq!(h.doc_text(), "k1=v1 v2=k2");
}
