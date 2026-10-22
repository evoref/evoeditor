//! 検索バー (docs/04 §8.4、01 FR-401〜404): 開閉・入力による検索・確定・大小の切替・強調。

use eve_testkit::Harness;
use eve_view::{CaseMode, FocusTarget, Frame, StyleMods};

fn text_of(t: &eve_view::CellText) -> String {
    t.spans.iter().map(|s| s.text.to_string()).collect()
}

fn bar(h: &mut Harness) -> eve_view::SearchBarModel {
    h.frame().search.expect("検索バーが開いている")
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

/// 描かれた行の中で、`mods` を持つセルの文字列。
fn marked(f: &Frame, mods: StyleMods) -> String {
    let mut out = String::new();
    for row in &f.panes[0].rows {
        for sp in &row.spans {
            if sp.style.mods.contains(mods) {
                out.push_str(&sp.text);
            }
        }
        out.push('|');
    }
    out
}

#[test]
fn ctrl_f_で開き_編集面が_1_行縮み_esc_で戻る() {
    let mut h = Harness::new().with_size(40, 10);
    let rows = h.frame().size.rows;
    assert!(h.frame().search.is_none());
    h.keys("ctrl+f");
    let f = h.frame();
    assert!(f.search.is_some());
    assert_eq!(f.size.rows, rows - 1);
    assert_eq!(f.focus, FocusTarget::SearchBar);
    assert!(f.primary_cursor.is_none(), "カーソルは検索バーにある");
    assert_eq!(text_of(&f.search.unwrap().label), "検索:");
    h.keys("esc");
    let f = h.frame();
    assert!(f.search.is_none());
    assert_eq!(f.size.rows, rows);
    assert!(matches!(f.focus, FocusTarget::Pane(_)));
}

#[test]
fn 入力すると件数が出て_現在の一致は強調するだけで_選択しない() {
    let mut h = Harness::new().with_size(40, 10);
    h.text("foo bar foo");
    h.keys("ctrl+home");
    let before = sel(&h);
    h.keys("ctrl+f");
    h.text("foo");
    h.run_jobs();
    assert_eq!(sel(&h), before, "入力では選択しない");
    let f = h.frame();
    let b = f.search.clone().unwrap();
    assert_eq!(
        text_of(
            &b.input
                .cells
                .iter()
                .fold(eve_view::CellText::default(), |mut a, s| {
                    a.spans.push(s.clone());
                    a
                })
        ),
        "foo"
    );
    assert_eq!(text_of(&b.count_text.unwrap()), "1/2");
    // 一致は下線、現在の一致は反転 + 太字 + 下線
    assert_eq!(
        marked(&f, StyleMods::UNDERLINE)
            .trim_end_matches('|')
            .split('|')
            .next()
            .unwrap(),
        "foofoo"
    );
    let cur = marked(&f, StyleMods::REVERSE | StyleMods::BOLD);
    assert!(cur.starts_with("foo"), "{cur}");
}

#[test]
fn enter_で現在の一致を選択し_次の_enter_で次へ_esc_で選択は残る() {
    let mut h = Harness::new().with_size(40, 10);
    h.text("foo bar foo");
    h.keys("ctrl+home");
    h.keys("ctrl+f");
    h.text("foo");
    h.run_jobs();
    h.keys("enter");
    assert_eq!(sel(&h), (0, 3), "最初の Enter は強調中の一致を確定する");
    h.keys("enter");
    assert_eq!(sel(&h), (8, 11));
    h.keys("up");
    assert_eq!(sel(&h), (0, 3));
    h.keys("f3");
    assert_eq!(sel(&h), (8, 11));
    h.keys("esc");
    assert_eq!(sel(&h), (8, 11), "確定した選択は残る");
}

#[test]
fn esc_で閉じると未確定の一致は選択に残らない() {
    let mut h = Harness::new().with_size(40, 10);
    h.text("foo bar foo");
    h.keys("ctrl+home");
    let before = sel(&h);
    h.keys("ctrl+f");
    h.text("bar");
    h.run_jobs();
    h.keys("esc");
    assert_eq!(sel(&h), before);
}

#[test]
fn alt_c_で大小の区別が巡回し_再検索する() {
    let mut h = Harness::new().with_size(40, 10);
    h.text("Foo foo");
    h.keys("ctrl+home ctrl+f");
    h.text("foo");
    h.run_jobs();
    let b = bar(&mut h);
    assert_eq!((b.case, b.case_on), (CaseMode::Smart, false));
    assert_eq!(text_of(&b.count_text.unwrap()), "1/2");
    h.keys("alt+c");
    h.run_jobs();
    let b = bar(&mut h);
    assert_eq!((b.case, b.case_on), (CaseMode::Sensitive, true));
    assert_eq!(text_of(&b.count_text.unwrap()), "1/1");
    h.keys("alt+c");
    assert_eq!(bar(&mut h).case, CaseMode::Insensitive);
    h.keys("alt+c");
    assert_eq!(bar(&mut h).case, CaseMode::Smart);
}

#[test]
fn smart_で検索語に大文字があれば_区別を示す() {
    let mut h = Harness::new().with_size(40, 10);
    h.text("Foo foo");
    h.keys("ctrl+f");
    h.text("Foo");
    assert!(bar(&mut h).case_on);
}

#[test]
fn 一致が無ければ_0_件で_欄を消すと件数も消える() {
    let mut h = Harness::new().with_size(40, 10);
    h.text("abc");
    h.keys("ctrl+f");
    h.text("zzz");
    h.run_jobs();
    assert_eq!(text_of(&bar(&mut h).count_text.unwrap()), "0 件");
    h.keys("backspace backspace backspace");
    assert!(bar(&mut h).count_text.is_none());
}

#[test]
fn 日本語を入力して検索できる() {
    let mut h = Harness::new().with_size(40, 10);
    h.text("日本語の文書、日本");
    h.keys("ctrl+home ctrl+f");
    h.text("日本");
    h.run_jobs();
    assert_eq!(text_of(&bar(&mut h).count_text.unwrap()), "1/2");
    h.keys("enter");
    assert_eq!(sel(&h), (0, 6));
}

#[test]
fn 検索バーが開いていても保存などのキーは効く() {
    let mut h = Harness::new().with_size(40, 10);
    h.text("x");
    h.keys("ctrl+f");
    h.keys("ctrl+shift+s");
    assert_eq!(h.overlay(), "prompt", "名前を付けて保存の入力が開く");
}

#[test]
fn 検索を開いたまま編集面の行数に合わせて強調する範囲は可視行だけ() {
    let mut h = Harness::new().with_size(40, 6);
    let doc: String = (0..100).map(|i| format!("line{i} foo\n")).collect();
    h.paste(&doc);
    h.keys("ctrl+home ctrl+f");
    h.text("foo");
    h.run_jobs();
    let f = h.frame();
    let n = marked(&f, StyleMods::UNDERLINE)
        .split('|')
        .filter(|s| s.contains("foo"))
        .count();
    assert!(n <= f.size.rows as usize, "{n}");
    assert!(n >= 1);
}

fn paste_into_field(h: &mut Harness, content: &str) {
    h.keys("ctrl+v");
    let token = h
        .effects()
        .iter()
        .rev()
        .find_map(|e| match e {
            eve_view::Effect::RequestClipboard { token, .. } => Some(*token),
            _ => None,
        })
        .expect("RequestClipboard");
    h.clipboard_content(token, Some(content));
}

#[test]
fn クリップボードの最初の行を検索欄へ貼り付ける() {
    let mut h = Harness::new().with_size(40, 10);
    h.text("foo bar");
    h.keys("ctrl+home ctrl+f");
    paste_into_field(&mut h, "bar\nignored");
    h.run_jobs();
    let b = bar(&mut h);
    assert_eq!(text_of(&b.count_text.unwrap()), "1/1");
    assert_eq!(h.doc_text(), "foo bar", "文書には貼り付けない");
}

#[test]
fn プロンプトにも貼り付けられる() {
    let mut h = Harness::new().with_size(40, 10);
    h.keys("ctrl+o");
    paste_into_field(&mut h, "/w/x.txt");
    let f = h.frame();
    let eve_view::OverlayModel::Prompt { input, .. } = &f.overlays[0] else {
        panic!()
    };
    let s: String = input.cells.iter().map(|c| c.text.to_string()).collect();
    assert_eq!(s, "/w/x.txt");
}

#[test]
fn 編集で結果が無効になっても_300_ms_待って自動で再検索する() {
    use eve_core::{Origin, Tick, Transaction};
    use eve_text::{ByteOff, ByteRange, ChangeSet, Edit};
    let mut h = Harness::new().with_size(40, 10);
    h.text("ab");
    h.keys("ctrl+home ctrl+f");
    h.text("ab");
    h.run_jobs();
    assert_eq!(text_of(&bar(&mut h).count_text.unwrap()), "1/1");
    // 検索バーを通さずに文書を変える
    let doc = h.doc_id();
    let cs = {
        let t = h.editor.state().document(doc).unwrap().text();
        ChangeSet::from_sorted_edits(
            t,
            [Edit {
                range: ByteRange::new(ByteOff(0), ByteOff(0)),
                insert: "ab ".into(),
            }],
        )
        .unwrap()
    };
    h.editor
        .state_mut()
        .apply_transaction(doc, Transaction::new(cs, Origin::User, Tick(0)))
        .unwrap();
    h.advance_ms(1);
    assert!(
        text_of(&bar(&mut h).count_text.unwrap()).starts_with('約'),
        "約表示"
    );
    assert!(h.editor.next_deadline().is_some(), "期限を Host に知らせる");
    h.advance_ms(299);
    assert!(text_of(&bar(&mut h).count_text.unwrap()).starts_with('約'));
    h.advance_ms(1);
    h.run_jobs();
    assert_eq!(text_of(&bar(&mut h).count_text.unwrap()), "1/2");
    assert!(h.editor.next_deadline().is_none());
}
