//! 選択範囲内の検索と置換 (docs/03 §12.5 の `scope: Selection`、04 §8.4 の `search.toggle_selection`)。

use eve_testkit::Harness;

fn text_of(t: &eve_view::CellText) -> String {
    t.spans.iter().map(|s| s.text.to_string()).collect()
}

fn count(h: &mut Harness) -> String {
    h.run_jobs();
    let bar = h.frame().search.expect("検索バーが開いている");
    bar.count_text.map(|t| text_of(&t)).unwrap_or_default()
}

fn sel(h: &Harness) -> (usize, usize) {
    let st = h.editor.state();
    let w = st.first_window().unwrap();
    let r = *st
        .view(st.window_view(w).unwrap())
        .unwrap()
        .selection
        .primary();
    (r.start().0, r.end().0)
}

const BODY: &str = "foo 1\nfoo 2\nfoo 3\nfoo 4\n";

/// 2・3 行目 ("foo 2\nfoo 3\n") を選んでから置換欄つきで検索バーを開き、`foo` を探す。
fn selected_two_lines() -> Harness {
    let mut h = Harness::new().with_size(40, 12);
    h.paste(BODY);
    h.keys("ctrl+home down shift+down shift+down");
    assert_eq!(sel(&h), (6, 18));
    h.keys("ctrl+r");
    h.text("foo");
    h
}

#[test]
fn トグルを入れると件数と移動が範囲内に限られる() {
    let mut h = selected_two_lines();
    assert!(count(&mut h).ends_with('4'), "文書全体では 4 件");
    h.keys("alt+l");
    let bar = h.frame().search.unwrap();
    assert!(bar.in_selection);
    assert!(count(&mut h).ends_with('2'), "{}", count(&mut h));
    // 次へは範囲内の一致だけを巡る
    h.keys("enter");
    assert_eq!(sel(&h), (6, 9));
    h.keys("enter");
    assert_eq!(sel(&h), (12, 15));
    h.keys("enter");
    assert_eq!(sel(&h), (6, 9), "範囲の端を越えると範囲の先頭へ");
    // もう一度で文書全体に戻る
    h.keys("alt+l");
    assert!(!h.frame().search.unwrap().in_selection);
    assert!(count(&mut h).ends_with('4'));
}

#[test]
fn すべて置換は範囲内だけを置き換え_範囲は編集に合わせて動く() {
    let mut h = selected_two_lines();
    h.keys("alt+l tab");
    h.text("barbaz");
    h.keys("alt+a");
    assert_eq!(h.doc_text(), "foo 1\nbarbaz 2\nbarbaz 3\nfoo 4\n");
    // 置換で長さが変わっても、範囲は置き換えた 2 行を指したまま
    h.keys("shift+tab backspace backspace backspace");
    h.text("barbaz");
    h.keys("tab");
    for _ in 0.."barbaz".len() {
        h.keys("backspace");
    }
    h.text("q");
    h.keys("alt+a");
    assert_eq!(h.doc_text(), "foo 1\nq 2\nq 3\nfoo 4\n");
}

#[test]
fn 一件置換も範囲外の一致は対象にしない() {
    let mut h = selected_two_lines();
    h.keys("alt+l tab");
    h.text("X");
    // 範囲内の 2 件を置換したら、それ以上は置換しない
    h.keys("enter enter enter");
    assert_eq!(h.doc_text(), "foo 1\nX 2\nX 3\nfoo 4\n");
}

#[test]
fn 選択が無ければ切り替えずに知らせる() {
    let mut h = Harness::new().with_size(40, 12);
    h.paste(BODY);
    h.keys("ctrl+f");
    h.text("foo");
    h.keys("alt+l");
    assert!(!h.frame().search.unwrap().in_selection);
    assert!(
        h.messages().iter().any(|m| m == "info.search.no_selection"),
        "{:?}",
        h.messages()
    );
}
