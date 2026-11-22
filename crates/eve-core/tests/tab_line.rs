//! タブライン (FR-510、docs/03 §9.9 の PANE-12、01 §5.5 の FR-510 AC-1 / AC-2) のシナリオ (03 §4.7.1 の L3)。core が出す
//! `Frame.tab_line` (開いた順・アクティブ・PANE-11 の名前・印)・行の確保 (`ui.tab_line` の 3 値と編集面の 3 行の条件)・
//! タブのクリック (`ChromeTarget::Tab`)・ASCII の Frame のゴールデン (`tests/golden/tab_line_*.frame.txt`)。

use eve_config::TabLineMode;
use eve_testkit::Harness;
use eve_testkit::helpers::cell_text;
use eve_view::{
    Answer, CellSize, DocumentId, FocusTarget, Frame, Modifiers, MouseButton, MouseKind, Query,
    StatusKind, ViewId,
};

/// 12 バイトの行を `n` 行 (`line 000000\n` …)。
fn lines(n: usize) -> Vec<u8> {
    (0..n)
        .flat_map(|i| format!("line {i:06}\n").into_bytes())
        .collect()
}

/// `files` を置き、順に開いた `cols` × `rows` の Harness (最後に開いた文書を表示する。最初の文書は起動時の無題の文書を
/// 置き換える)。
fn opened_sized(cols: u16, rows: u16, files: &[(&str, &[u8])]) -> Harness {
    let mut b = Harness::builder().size(cols, rows);
    for (p, d) in files {
        b = b.file(p, d);
    }
    let mut h = b.build();
    for (p, _) in files {
        h.open(p);
        h.run_jobs();
    }
    h
}

fn opened(files: &[(&str, &[u8])]) -> Harness {
    opened_sized(80, 24, files)
}

fn doc_of(h: &Harness, path: &str) -> DocumentId {
    let Ok(Answer::Documents(ds)) = h.query(&Query::Documents) else {
        panic!("文書の一覧");
    };
    ds.iter()
        .find(|d| d.path.as_deref() == Some(path))
        .unwrap_or_else(|| panic!("{path} の文書"))
        .doc
}

fn active_doc(h: &Harness) -> DocumentId {
    h.editor
        .state()
        .active_document()
        .expect("アクティブな文書")
}

/// フォーカスのペインの View (`Query::Windows`)。
fn focused(h: &Harness) -> ViewId {
    let Ok(Answer::Windows(ws)) = h.query(&Query::Windows) else {
        panic!("Window の一覧");
    };
    let FocusTarget::Pane(f) = ws[0].focus else {
        panic!("フォーカスはペイン");
    };
    f
}

fn doc_of_view(h: &Harness, v: ViewId) -> DocumentId {
    h.editor.state().view(v).expect("View").doc
}

/// タブの (文書, 項目の長い形を空白で繋いだもの) の並び。
type Tabs = Vec<(DocumentId, String)>;

/// タブの並びと、アクティブの添字。
fn tabs(f: &Frame) -> Option<(Tabs, Option<usize>)> {
    f.tab_line.as_ref().map(|t| {
        let tabs = t
            .tabs
            .iter()
            .map(|x| {
                let text: Vec<String> = x.items.iter().map(|i| cell_text(&i.long)).collect();
                (x.doc, text.join(" "))
            })
            .collect();
        (tabs, t.active)
    })
}

fn names(f: &Frame) -> Vec<String> {
    tabs(f)
        .expect("タブライン")
        .0
        .into_iter()
        .map(|(_, n)| n)
        .collect()
}

fn left_down() -> MouseKind {
    MouseKind::Down(MouseButton::Left)
}

// verifies: AC-FR-510-1
#[test]
fn 開いた順に並び_フォーカスのペインの文書がアクティブで_印が付く() {
    let mut h = opened(&[
        ("/w/a.txt", b"a\n"),
        ("/w/b.txt", b"b\n"),
        ("/w/c.txt", b"c\n"),
    ]);
    let (a, b, c) = (
        doc_of(&h, "/w/a.txt"),
        doc_of(&h, "/w/b.txt"),
        doc_of(&h, "/w/c.txt"),
    );
    let (t, active) = tabs(&h.frame()).expect("文書が 3 つならタブラインを出す");
    assert_eq!(
        t,
        vec![
            (a, "a.txt".to_owned()),
            (b, "b.txt".to_owned()),
            (c, "c.txt".to_owned())
        ]
    );
    assert_eq!(active, Some(2), "最後に開いた c を表示している");
    // `buffer.next` と同じ順: 次は先頭 (巡る)
    h.cmd("buffer.next");
    assert_eq!(active_doc(&h), a);
    assert_eq!(tabs(&h.frame()).expect("タブライン").1, Some(0));
    // 変更あり・読み取り専用の印 (ステータスバーの左群と同じ文言。読み取り専用は短い形も持つ)
    h.text("x");
    h.cmd("buffer.next");
    h.cmd("file.toggle_read_only");
    let f = h.frame();
    assert_eq!(names(&f), vec!["a.txt [+]", "b.txt [RO]", "c.txt"]);
    let tl = f.tab_line.as_ref().expect("タブライン");
    let kinds: Vec<Vec<StatusKind>> = tl
        .tabs
        .iter()
        .map(|t| t.items.iter().map(|i| i.kind).collect())
        .collect();
    assert_eq!(
        kinds,
        vec![
            vec![StatusKind::Path, StatusKind::Dirty],
            vec![StatusKind::Path, StatusKind::ReadOnly],
            vec![StatusKind::Path],
        ]
    );
    let ro = &tl.tabs[1].items[1];
    assert_eq!(ro.short.as_ref().map(cell_text).as_deref(), Some("RO"));
    // 名前は中間で切ってよい (frontend が 32 セルまでに縮める)
    assert_eq!(tl.tabs[0].items[0].long.cut, eve_view::CutPolicy::Middle);
}

// verifies: AC-FR-510-1
#[test]
fn 同じ名前のファイルは親のディレクトリで区別し_無題は通し番号のまま() {
    let mut h = opened(&[
        ("/p/a/mod.rs", b"a\n"),
        ("/p/b/mod.rs", b"b\n"),
        ("/w/lib.rs", b"l\n"),
    ]);
    h.cmd("file.new");
    assert_eq!(
        names(&h.frame()),
        vec!["mod.rs (a)", "mod.rs (b)", "lib.rs", "無題-2"]
    );
}

// verifies: AC-FR-510-1
#[test]
fn フォーカスのペインの文書がアクティブで_ペインを移るとアクティブも替わる() {
    let mut h = opened(&[("/w/a.txt", &lines(30)), ("/w/b.txt", b"b\n")]);
    let (a, b) = (doc_of(&h, "/w/a.txt"), doc_of(&h, "/w/b.txt"));
    h.cmd("pane.split_right");
    h.open("/w/a.txt");
    // 右は a、左は b
    let f = h.frame();
    assert_eq!(tabs(&f).expect("タブライン").1, Some(0));
    h.cmd("pane.focus_prev");
    assert_eq!(active_doc(&h), b);
    assert_eq!(tabs(&h.frame()).expect("タブライン").1, Some(1));
    // タブは文書ごとに 1 つ (2 つのペインで同じ文書を出しても増えない)
    h.open("/w/a.txt");
    let f = h.frame();
    let (t, active) = tabs(&f).expect("タブライン");
    assert_eq!(t.iter().map(|x| x.0).collect::<Vec<_>>(), vec![a, b]);
    assert_eq!(active, Some(0));
}

// verifies: AC-FR-510-2
#[test]
fn ui_tab_line_の_3_値で出し入れし_出すと編集面が_1_行減る() {
    let mut h = opened(&[("/w/a.txt", &lines(5))]);
    let f = h.frame();
    assert!(f.tab_line.is_none(), "auto: 文書が 1 つなら出さない");
    assert_eq!(f.size, CellSize { cols: 80, rows: 24 });
    h.set_tab_line(TabLineMode::Always);
    let f = h.frame();
    assert_eq!(names(&f), vec!["a.txt"], "always: 1 つでも出す");
    assert_eq!(f.size.rows, 23);
    h.set_tab_line(TabLineMode::Never);
    h.cmd("file.new");
    let f = h.frame();
    assert!(f.tab_line.is_none(), "never: 2 つでも出さない");
    assert_eq!(f.size.rows, 24);
    h.set_tab_line(TabLineMode::Auto);
    let f = h.frame();
    assert_eq!(names(&f).len(), 2, "auto: 2 つなら出す");
    assert_eq!(f.size.rows, 23);
    // 閉じて 1 つになれば消える
    h.cmd("buffer.close");
    let f = h.frame();
    assert!(f.tab_line.is_none());
    assert_eq!(f.size.rows, 24);
}

// verifies: AC-FR-510-2
#[test]
fn 設定の再読込で_ui_tab_line_が即時に効く() {
    let mut h = Harness::builder().file("/w/a.txt", b"a\n").build();
    h.open("/w/a.txt");
    h.run_jobs();
    assert!(h.frame().tab_line.is_none());
    let loaded = eve_config::load_str("[ui]\ntab_line = \"always\"\n");
    assert_eq!(loaded.diagnostics, Vec::new());
    h.editor.state_mut().set_config(loaded.config);
    assert!(h.frame().tab_line.is_some());
}

// verifies: AC-FR-510-2
#[test]
fn 出し入れでカーソルが見える範囲に保たれ_選択は変わらない() {
    // 10 行の編集面で、カーソルを下へ動かして画面の下の余白 (`view.scroll_margin` = 3) の端に置く
    let mut h = opened_sized(80, 10, &[("/w/a.txt", &lines(40))]);
    for _ in 0..12 {
        h.keys("down");
    }
    let sel = |h: &Harness| {
        let v = h.editor.state().view(focused(h)).expect("View");
        (v.selection.clone(), v.scroll.top)
    };
    let (sel0, top0) = sel(&h);
    assert_eq!(
        h.frame().primary_cursor.expect("カーソル").pos.row,
        6,
        "10 行の下の余白 3 行の上"
    );
    // 出す: 編集面が 9 行になり、カーソルが余白に入るので 1 行だけスクロールする
    h.set_tab_line(TabLineMode::Always);
    let f = h.frame();
    assert_eq!(f.size.rows, 9);
    assert_eq!(f.primary_cursor.expect("カーソル").pos.row, 5);
    let (sel1, top1) = sel(&h);
    assert_eq!(sel1, sel0, "選択は変えない");
    assert!(top1 > top0, "前へスクロールした");
    // 消す: 編集面が 10 行に戻る。カーソルは見えているのでスクロールしない
    h.set_tab_line(TabLineMode::Never);
    let f = h.frame();
    assert_eq!(f.size.rows, 10);
    let (sel2, top2) = sel(&h);
    assert_eq!((sel2, top2), (sel0, top1));
    assert_eq!(f.primary_cursor.expect("カーソル").pos.row, 5);
    // カーソルが先頭にあるときは、出してもスクロールしない
    h.keys("ctrl+home");
    let (_, top3) = sel(&h);
    h.set_tab_line(TabLineMode::Always);
    assert_eq!(sel(&h).1, top3);
}

// verifies: AC-FR-510-2
#[test]
fn 出すと編集面が_3_行を割るときは出さない() {
    // 編集面 4 行: 出すと 3 行 (出せる)
    let mut h = opened_sized(40, 4, &[("/w/a.txt", b"a\n"), ("/w/b.txt", b"b\n")]);
    let f = h.frame();
    assert!(f.tab_line.is_some());
    assert_eq!(f.size.rows, 3);
    // 編集面 3 行: 出すと 2 行 (出さない)
    h.resize(40, 3);
    let f = h.frame();
    assert!(f.tab_line.is_none());
    assert_eq!(f.size.rows, 3);
    h.set_tab_line(TabLineMode::Always);
    assert!(h.frame().tab_line.is_none(), "always でも 3 行の条件は同じ");
    // 検索バーの行を引いた後で判定する (編集面 5 行 − 検索バー 1 行 = 4 行: 出す)
    h.resize(40, 5);
    h.cmd("search.open");
    let f = h.frame();
    assert!(f.tab_line.is_some());
    assert_eq!(f.size.rows, 3);
    // 置換欄も開くと 5 − 2 = 3 行: 出さない
    h.cmd("search.replace_open");
    let f = h.frame();
    assert!(f.tab_line.is_none());
    assert_eq!(f.size.rows, 3);
}

// verifies: AC-FR-510-2
#[test]
fn タブラインを出すと縮退表示に入ることがある() {
    // 上下に 2 つのペインの最小は 8 行 (本文 3 + 見出し 1 が 2 つ。03 §9.9 の PANE-4)
    let mut h = opened_sized(40, 8, &[("/w/a.txt", &lines(20))]);
    h.cmd("pane.split_down");
    let f = h.frame();
    assert_eq!(f.panes.len(), 2);
    // 文書を足すとタブラインの 1 行で編集面が 7 行になり、木の最小を割るので縮退表示
    h.cmd("file.new");
    let f = h.frame();
    assert!(f.tab_line.is_some());
    assert_eq!(f.size.rows, 7);
    assert_eq!(f.panes.len(), 1, "フォーカスのペインだけ");
    assert!(f.panes[0].bar.is_none());
    // タブラインを消すと元の配置に戻る
    h.set_tab_line(TabLineMode::Never);
    assert_eq!(h.frame().panes.len(), 2);
}

// verifies: AC-FR-510-1
#[test]
fn 置き換えで読込中の文書はタブに出さない() {
    let mut h = Harness::builder().file("/w/a.txt", &lines(3)).build();
    // 起動時の無題の文書を置き換えて開く間は、文書が 2 つでもタブラインを出さない (ステータスバーの (n/total) と同じ)
    h.open("/w/a.txt");
    assert_eq!(h.editor.state().documents_len(), 2);
    assert!(h.frame().tab_line.is_none());
    h.run_jobs();
    assert!(h.frame().tab_line.is_none());
}

// verifies: AC-FR-510-2
#[test]
fn タブのクリックでフォーカスのペインに表示し_ほかのペインは変えない() {
    let mut h = opened(&[
        ("/w/a.txt", b"a\n"),
        ("/w/b.txt", b"b\n"),
        ("/w/c.txt", b"c\n"),
    ]);
    let (a, b, c) = (
        doc_of(&h, "/w/a.txt"),
        doc_of(&h, "/w/b.txt"),
        doc_of(&h, "/w/c.txt"),
    );
    let left = focused(&h);
    h.cmd("pane.split_right");
    h.tab_mouse(a, left_down(), Modifiers::NONE);
    assert_eq!(active_doc(&h), a);
    assert_ne!(focused(&h), left, "フォーカスは右のペインのまま");
    assert_eq!(doc_of_view(&h, focused(&h)), a, "右のペインに a を出す");
    assert_eq!(doc_of_view(&h, left), c, "左のペインは c のまま");
    // 修飾キーを問わず左の押下で反応する
    h.tab_mouse(b, left_down(), Modifiers::CTRL);
    assert_eq!(active_doc(&h), b);
}

// verifies: AC-FR-510-2
#[test]
fn タブのクリックは_buffer_next_と同じく_undo_を封印し_最近使った順に記録する() {
    let mut h = opened(&[
        ("/w/a.txt", b"\n"),
        ("/w/b.txt", b"\n"),
        ("/w/c.txt", b"\n"),
    ]);
    let (a, c) = (doc_of(&h, "/w/a.txt"), doc_of(&h, "/w/c.txt"));
    h.text("x");
    // c → a → c と行き来してから続けて打つ: 戻すと後の 1 文字だけが消える (封印した)
    h.click_tab(a);
    h.click_tab(c);
    h.text("y");
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "x\n");
    // 最近使った順の記録: a を出してから a を閉じると、直前の c に戻る (記録が無ければ開いた順で b)
    h.click_tab(a);
    h.cmd("buffer.close");
    assert_eq!(active_doc(&h), c);
}

// verifies: AC-FR-510-2
#[test]
fn アクティブなタブのクリックは何もせず_undo_も封印しない() {
    let mut h = opened(&[("/w/a.txt", b"\n"), ("/w/b.txt", b"\n")]);
    let b = doc_of(&h, "/w/b.txt");
    h.text("x");
    h.click_tab(b);
    h.text("y");
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "\n", "x と y は 1 回で戻る");
}

// verifies: AC-FR-510-2
#[test]
fn オーバーレイと検索バーの間_閉じた文書_左の押下でない操作は捨てる() {
    let mut h = opened(&[("/w/a.txt", b"a\n"), ("/w/b.txt", b"b\n")]);
    let (a, b) = (doc_of(&h, "/w/a.txt"), doc_of(&h, "/w/b.txt"));
    // オーバーレイ (パレット) の間は捨てる (PANE-6 (c))
    h.cmd("ui.palette_open");
    h.tab_mouse(a, left_down(), Modifiers::NONE);
    assert_eq!(h.overlay(), "palette");
    h.keys("escape");
    assert_eq!(active_doc(&h), b);
    // 検索バーの間も捨てる (検索バーは開いたまま)
    h.cmd("search.open");
    h.tab_mouse(a, left_down(), Modifiers::NONE);
    assert_eq!(active_doc(&h), b);
    assert!(h.frame().search.is_some());
    h.keys("escape");
    // 離す・ドラッグ・ホイール・右と中のボタンは捨てる (PANE-7 (f))
    for kind in [
        MouseKind::Up(MouseButton::Left),
        MouseKind::Drag(MouseButton::Left),
        MouseKind::WheelDown,
        MouseKind::Down(MouseButton::Right),
        MouseKind::Down(MouseButton::Middle),
    ] {
        h.tab_mouse(a, kind, Modifiers::NONE);
        assert_eq!(active_doc(&h), b, "{kind:?}");
    }
    // 閉じた文書・無い文書のタブは捨てる
    h.cmd("file.new");
    let gone = active_doc(&h);
    h.cmd("buffer.close");
    let now = active_doc(&h);
    h.tab_mouse(gone, left_down(), Modifiers::NONE);
    h.tab_mouse(DocumentId(9_999), left_down(), Modifiers::NONE);
    assert_eq!(active_doc(&h), now);
    assert_eq!(h.editor.state().documents_len(), 2);
}

// ---------- ASCII の Frame のゴールデン (03 §4.7.1 のゴールデン、PANE-12 (h)) ----------

fn golden_path(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.frame.txt"))
}

/// ガター・クローム・カーソルつきの ASCII の Frame を、ゴールデン `name` と比べる。
fn check_frame(h: &mut Harness, name: &str) {
    let text = h.frame_text_with(eve_testkit::AsciiOptions {
        show_gutter: true,
        show_chrome: true,
        ..eve_testkit::AsciiOptions::default()
    });
    eve_testkit::golden::check(&golden_path(name), &text);
}

// verifies: AC-FR-510-1
#[test]
fn ascii_の_frame_80_桁で全部入るタブライン() {
    let mut h = opened_sized(
        80,
        8,
        &[
            ("/w/main.rs", &lines(3)),
            ("/w/lib.rs", &lines(3)),
            ("/p/a/mod.rs", &lines(3)),
            ("/p/b/mod.rs", &lines(3)),
        ],
    );
    h.click_tab(doc_of(&h, "/w/main.rs"));
    h.text("x");
    check_frame(&mut h, "tab_line_fits");
}

// verifies: AC-FR-510-1
#[test]
fn ascii_の_frame_幅を超えるタブラインはアクティブを含む範囲と省いた側の印() {
    let files: Vec<(String, Vec<u8>)> = (1..=8)
        .map(|i| (format!("/w/file-{i}.txt"), lines(2)))
        .collect();
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(p, d)| (p.as_str(), d.as_slice()))
        .collect();
    let mut h = opened_sized(60, 6, &refs);
    // 真ん中のタブをアクティブにする: 左右とも省く
    h.click_tab(doc_of(&h, "/w/file-4.txt"));
    check_frame(&mut h, "tab_line_overflow");
    // 32 セルを超える名前は中間を省略し、アクティブだけでも入らない幅では名前をさらに縮める
    let long = format!("/w/{}.txt", "a-very-long-file-name-".repeat(3));
    let mut h = opened_sized(30, 6, &[("/w/x.txt", b"x\n"), (long.as_str(), b"y\n")]);
    h.text("z");
    check_frame(&mut h, "tab_line_squeeze");
}
