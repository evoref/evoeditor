//! 複数ペイン (docs/03 §9.9、01 §5.5 の FR-509) のシナリオ (09 §3.4 の計画の (2)・(3))。v0.5 の M30 の段 2a: core の木の
//! 操作と PANE-4〜PANE-9 を、`execute` とパレットから実行する。段 2b: 見出し・区切り・フォーカスの強調 (Frame)・既定キー・
//! 見出しのクリック (`ChromeMouse`)・ASCII の Frame のゴールデン (`tests/golden/panes_*.frame.txt`)。

use eve_core::{Args, CommandResult, RejectReason};
use eve_testkit::Harness;
use eve_testkit::helpers::cell_text;
use eve_view::{
    Answer, DocumentId, FocusTarget, Modifiers, MouseButton, MouseKind, OverlayModel, Query,
    StatusKind, ViewId,
};

/// 12 バイトの行を `n` 行 (`line 000000\n` …)。
fn lines(n: usize) -> Vec<u8> {
    (0..n)
        .flat_map(|i| format!("line {i:06}\n").into_bytes())
        .collect()
}

/// `files` を置き、順に開いた Harness (最後に開いた文書を表示する)。
fn opened(files: &[(&str, &[u8])]) -> Harness {
    let mut b = Harness::builder();
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

fn focused(h: &Harness) -> ViewId {
    panes(h).1
}

fn doc_of_view(h: &Harness, v: ViewId) -> DocumentId {
    h.editor.state().view(v).expect("View").doc
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

fn head(h: &Harness, v: ViewId) -> usize {
    h.editor
        .state()
        .view(v)
        .expect("View")
        .selection
        .primary()
        .head
        .0
}

fn check(h: &Harness) {
    if let Err(e) = h.editor.state().check_panes() {
        panic!("ペインの不変条件: {e}");
    }
}

fn run(h: &mut Harness, id: &str) {
    assert_eq!(h.cmd(id), CommandResult::Done, "{id}");
    check(h);
}

/// パレットで `query` を入れて、先頭の候補を実行する。
fn via_palette(h: &mut Harness, query: &str) {
    h.cmd("ui.palette_open");
    h.text(query);
    h.keys("enter");
    check(h);
}

/// パレットで `query` を入れたときの先頭の候補の、実行できない理由 (実行できれば `None`)。
fn palette_reason(h: &mut Harness, query: &str) -> Option<String> {
    h.cmd("ui.palette_open");
    h.text(query);
    let f = h.frame();
    let reason = f.overlays.iter().find_map(|o| match o {
        OverlayModel::List { items, .. } => {
            Some(items.first()?.disabled_reason.as_ref().map(cell_text))
        }
        _ => None,
    });
    h.keys("escape");
    reason.flatten()
}

// verifies: AC-FR-509-1
#[test]
// 分割・移動と、全ペインが Frame に出ること。見出し・区切り・フォーカスの強調・ASCII の Frame・既定キーは下の段 2b の試験
fn 右と下に分割し_次_前と方向でフォーカスを移せる() {
    let mut h = opened(&[("/w/a.txt", &lines(50))]);
    let left = focused(&h);
    run(&mut h, "pane.split_right");
    let (v, right) = panes(&h);
    assert_eq!(v, vec![left, right], "右に新しいペイン、フォーカスはそこへ");
    run(&mut h, "pane.split_down");
    let (v, bottom) = panes(&h);
    assert_eq!(v, vec![left, right, bottom], "右のペインの下に新しいペイン");
    let f = h.frame();
    assert_eq!(f.panes.len(), 3, "フォーカスの無いペインも描く");
    let (l, r, b) = (f.panes[0].rect, f.panes[1].rect, f.panes[2].rect);
    assert!(
        r.col > l.col + l.cols && r.col == b.col,
        "{l:?} {r:?} {b:?}"
    );
    assert!(
        b.row > r.row + r.rows,
        "下のペインは見出しの行の下: {r:?} {b:?}"
    );
    // 次 / 前は木の葉の順で、端から反対の端へ巡る
    run(&mut h, "pane.focus_next");
    assert_eq!(focused(&h), left);
    run(&mut h, "pane.focus_prev");
    assert_eq!(focused(&h), bottom);
    run(&mut h, "pane.focus_prev");
    assert_eq!(focused(&h), right);
    // 方向: 隣り合うペインへ。隣が無ければ何もしない (巡らない)
    run(&mut h, "pane.focus_down");
    assert_eq!(focused(&h), bottom);
    run(&mut h, "pane.focus_up");
    assert_eq!(focused(&h), right);
    run(&mut h, "pane.focus_up");
    assert_eq!(focused(&h), right, "上に隣は無い");
    run(&mut h, "pane.focus_left");
    assert_eq!(focused(&h), left);
    // 左のペインの主カーソルは先頭の行なので、右の上のペインへ
    run(&mut h, "pane.focus_right");
    assert_eq!(focused(&h), right);
    // 左のペインの主カーソルを下半分の行へ動かすと、右の下のペインへ
    run(&mut h, "pane.focus_left");
    h.cmd_with("cursor.move_down", Args::new().with_int("count", 18));
    run(&mut h, "pane.focus_right");
    assert_eq!(focused(&h), bottom);
}

// verifies: AC-FR-509-1
#[test]
// パレットからの実行。既定キーからの実行は下の段 2b の試験
fn パレットから実行したペインのコマンドはフォーカスのペインに効く() {
    let mut h = opened(&[("/w/a.txt", &lines(10))]);
    let left = focused(&h);
    via_palette(&mut h, "pane.split_right");
    let (v, right) = panes(&h);
    assert_eq!(v, vec![left, right]);
    via_palette(&mut h, "pane.focus_next");
    assert_eq!(focused(&h), left);
    // パレットから開いた文書は、フォーカスのペインに表示する (03 §9.9 の PANE-6 (d))
    h.cmd("file.new");
    let (v, f) = panes(&h);
    assert_eq!(v[1], right, "ほかのペインの表示は変えない");
    assert_ne!(doc_of_view(&h, f), doc_of_view(&h, right));
}

// verifies: AC-FR-509-2
#[test]
fn 同じ文書を_2_つのペインで編集すると両方に反映し_もう片方の選択は写像され_undo_はどちらからでも同じ本文()
 {
    let mut h = opened(&[("/w/a.txt", b"one\ntwo\nthree\n")]);
    let left = focused(&h);
    h.keys("down");
    assert_eq!(head(&h, left), 4);
    run(&mut h, "pane.split_right");
    let right = focused(&h);
    assert_eq!(head(&h, right), 4, "分割で作ったペインは同じ位置で始まる");
    // 右で先頭に入力すると、左の選択は写像される
    h.keys("ctrl+home");
    h.text("X");
    assert_eq!(h.doc_text(), "Xone\ntwo\nthree\n");
    assert_eq!(head(&h, left), 5, "左のカーソルは編集に合わせてずれる");
    assert_eq!(head(&h, right), 1);
    // 左で入力すると右にも反映する
    run(&mut h, "pane.focus_prev");
    h.text("Y");
    assert_eq!(h.doc_text(), "Xone\nYtwo\nthree\n");
    assert_eq!(head(&h, right), 1, "右のカーソルは前の位置のまま");
    // Undo は文書に 1 つ: 右から戻しても左から戻しても同じ本文。右で左の編集を戻すと、選択を戻すのは発行した左の
    // View (表示中) で、右の選択は写像だけ (03 §7.6、§9.9 の PANE-2)
    run(&mut h, "pane.focus_next");
    h.cmd("edit.undo");
    let from_right = h.doc_text();
    assert_eq!(head(&h, right), 1, "実行した右のカーソルは飛ばない");
    assert_eq!(head(&h, left), 5, "発行した左の View の選択を戻す");
    h.cmd("edit.redo");
    assert_eq!(head(&h, right), 1);
    assert_eq!(head(&h, left), 6);
    run(&mut h, "pane.focus_prev");
    h.cmd("edit.undo");
    assert_eq!(h.doc_text(), from_right);
    assert_eq!(from_right, "Xone\ntwo\nthree\n");
    h.cmd("edit.undo");
    assert_eq!(h.doc_text(), "one\ntwo\nthree\n");
}

// verifies: AC-FR-509-2
#[test]
fn ペインで別の文書に切り替えて戻すと_そのペインでの位置が保たれる() {
    let mut h = opened(&[("/w/a.txt", &lines(40)), ("/w/b.txt", b"bee\n")]);
    h.open("/w/a.txt");
    let left = focused(&h);
    h.cmd_with("cursor.goto_line", Args::new().with_int("line", 30));
    let at = head(&h, left);
    run(&mut h, "pane.split_right");
    let right = focused(&h);
    h.keys("ctrl+home");
    // 右のペインで b に切り替えて戻す
    h.open("/w/b.txt");
    assert_eq!(doc_of_view(&h, focused(&h)), doc_of(&h, "/w/b.txt"));
    h.open("/w/a.txt");
    assert_eq!(focused(&h), right, "そのペインの表の View に戻る");
    assert_eq!(head(&h, right), 0, "右のペインでの位置");
    assert_eq!(head(&h, left), at, "左のペインの位置は変わらない");
    check(&h);
}

// verifies: AC-FR-509-3
#[test]
fn dirty_な文書のペインを閉じても確認は出ず文書は残り_最後のペインは閉じない() {
    let mut h = opened(&[("/w/a.txt", b"abc\n")]);
    h.text("x");
    assert!(h.is_dirty());
    run(&mut h, "pane.split_right");
    run(&mut h, "pane.close");
    assert_eq!(h.overlay(), "", "確認を出さない");
    assert_eq!(panes(&h).0.len(), 1);
    assert_eq!(h.editor.state().documents_len(), 1, "文書は閉じない");
    assert!(h.is_dirty());
    // 最後のペインは閉じない: キーでは理由を示し、パレットでは淡色にする
    h.clear_log();
    assert_eq!(
        h.cmd("pane.close"),
        CommandResult::Rejected(RejectReason::NotAvailable)
    );
    assert!(
        h.messages().contains(&"info.pane.single_pane".to_owned()),
        "{:?}",
        h.messages()
    );
    assert_eq!(
        palette_reason(&mut h, "pane.close").as_deref(),
        Some("現在は実行できません: ペインが 1 つだけです")
    );
    assert_eq!(palette_reason(&mut h, "pane.split_right"), None);
    check(&h);
}

// verifies: AC-FR-509-3
#[test]
fn _2_つのペインで表示中の文書を閉じると各ペインがそれぞれ最近使った文書を出す() {
    let mut h = opened(&[
        ("/w/a.txt", b"a\n"),
        ("/w/b.txt", b"b\n"),
        ("/w/c.txt", b"c\n"),
    ]);
    // 左: a → b → c、右 (左を写して作る): c → a → c
    run(&mut h, "pane.split_right");
    h.open("/w/a.txt");
    h.open("/w/c.txt");
    let c = doc_of(&h, "/w/c.txt");
    run(&mut h, "buffer.close");
    let (v, _) = panes(&h);
    assert_eq!(v.len(), 2, "ペインは閉じない");
    assert_eq!(doc_of_view(&h, v[0]), doc_of(&h, "/w/b.txt"), "左の直近");
    assert_eq!(doc_of_view(&h, v[1]), doc_of(&h, "/w/a.txt"), "右の直近");
    assert!(h.editor.state().document(c).is_none());
}

// verifies: AC-FR-509-3
#[test]
fn 最後の文書を閉じると表示していた全ペインが同じ新しい無題の文書を出す() {
    let mut h = opened(&[("/w/a.txt", b"a\n")]);
    run(&mut h, "pane.split_right");
    run(&mut h, "pane.split_down");
    let a = doc_of(&h, "/w/a.txt");
    run(&mut h, "buffer.close");
    let (v, _) = panes(&h);
    assert_eq!(v.len(), 3);
    let blank = doc_of_view(&h, v[0]);
    assert_ne!(blank, a);
    assert!(v.iter().all(|x| doc_of_view(&h, *x) == blank), "同じ無題");
    assert_eq!(h.editor.state().documents_len(), 1);
}

// verifies: AC-FR-509-3
#[test]
fn 分割した後に最小が入らなければ分割せずに知らせる() {
    // 40 列: 左右に分けると 20 + 区切り 1 + 20 = 41 列が要る
    let mut h = Harness::builder().size(40, 24).build();
    run(&mut h, "pane.split_right");
    assert_eq!(panes(&h).0.len(), 1);
    assert!(h.messages().contains(&"info.pane.no_room".to_owned()));
    // 上下 (4 + 4 行) は入る
    run(&mut h, "pane.split_down");
    assert_eq!(panes(&h).0.len(), 2);
    // 7 行の面では上下にも分けられない
    let mut h = Harness::builder().size(80, 7).build();
    run(&mut h, "pane.split_down");
    assert_eq!(panes(&h).0.len(), 1);
}

// verifies: AC-FR-509-3
#[test]
fn 端末が縮むとフォーカスのペインだけを出し_入力が届き_戻すと元の配置に戻る() {
    let mut h = opened(&[("/w/a.txt", &lines(30))]);
    let left = focused(&h);
    run(&mut h, "pane.split_right");
    let right = focused(&h);
    let before: Vec<_> = h.frame().panes.iter().map(|p| (p.view, p.rect)).collect();
    h.clear_log();
    h.resize(30, 24);
    check(&h);
    let f = h.frame();
    assert_eq!(f.panes.len(), 1, "縮退表示");
    assert_eq!(f.panes[0].view, right);
    assert_eq!((f.panes[0].rect.cols, f.panes[0].rect.rows), (30, 24));
    assert!(h.messages().contains(&"info.pane.collapsed".to_owned()));
    let degraded: Vec<String> = f
        .status
        .items
        .iter()
        .filter(|i| i.kind == StatusKind::Degraded)
        .map(|i| cell_text(&i.long))
        .collect();
    assert_eq!(degraded, vec!["[ペイン 2/2 だけ表示]".to_owned()]);
    // 入力は届き、移動・閉じるは木に対して働く
    h.text("z");
    assert!(h.doc_text().starts_with('z'));
    run(&mut h, "pane.focus_prev");
    assert_eq!(
        h.frame().panes[0].view,
        left,
        "フォーカスを移すと表示するペインが替わる"
    );
    // 戻すと元の配置
    h.clear_log();
    h.resize(80, 24);
    check(&h);
    let after: Vec<_> = h.frame().panes.iter().map(|p| (p.view, p.rect)).collect();
    assert_eq!(after, before);
    assert!(!h.messages().contains(&"info.pane.collapsed".to_owned()));
}

// verifies: AC-FR-509-4
#[test]
// core のマウス・大きさ・検索バーとオーバーレイ。見出しのクリック (`ChromeMouse`)・既定キーは下の段 2b の試験、TUI の変換は
// eve-tui の `mouse` の単体試験
fn フォーカスの無いペインのクリックでフォーカスとカーソルが移り_ホイールはポインタの下のペインだけを動かす()
 {
    let mut h = opened(&[("/w/a.txt", &lines(100))]);
    let left = focused(&h);
    run(&mut h, "pane.split_right");
    let right = focused(&h);
    h.mouse_in(
        left,
        MouseKind::Down(MouseButton::Left),
        Modifiers::NONE,
        2,
        0,
    );
    h.mouse_in(
        left,
        MouseKind::Up(MouseButton::Left),
        Modifiers::NONE,
        2,
        0,
    );
    assert_eq!(focused(&h), left, "クリックしたペインへ");
    assert_eq!(head(&h, left), 24, "3 行目の行頭へ");
    assert_eq!(head(&h, right), 0);
    // ホイールはポインタの下のペインをスクロールし、フォーカスは移さない
    h.mouse_in(right, MouseKind::WheelDown, Modifiers::NONE, 1, 1);
    assert_eq!(focused(&h), left);
    let top = |h: &Harness, v: ViewId| h.editor.state().view(v).expect("View").scroll.top.0;
    assert!(top(&h, right) > 0);
    assert_eq!(top(&h, left), 0);
    // 表示中でない View を指す入力は捨てる (ペインが別の文書に切り替わった後の古い Frame の座標、PANE-7 (g))
    h.cmd("file.new");
    h.mouse_in(
        left,
        MouseKind::Down(MouseButton::Left),
        Modifiers::NONE,
        5,
        0,
    );
    assert_eq!(head(&h, left), 24, "見えていない文書の位置として解かない");
    check(&h);
}

// verifies: AC-FR-509-4
#[test]
// core のマウス・大きさ・検索バーとオーバーレイ。見出しのクリック (`ChromeMouse`)・既定キーは下の段 2b の試験、TUI の変換は
// eve-tui の `mouse` の単体試験
fn キーでペインの大きさを変えて揃えられる() {
    let mut h = opened(&[("/w/a.txt", b"a\n")]);
    run(&mut h, "pane.split_right");
    let widths =
        |h: &mut Harness| -> Vec<u16> { h.frame().panes.iter().map(|p| p.rect.cols).collect() };
    assert_eq!(widths(&mut h), vec![40, 39]);
    // 既定は親の長さ (80) の 1/10
    run(&mut h, "pane.grow");
    assert_eq!(widths(&mut h), vec![32, 47]);
    assert_eq!(
        h.cmd_with("pane.shrink", Args::new().with_int("cells", 2)),
        CommandResult::Done
    );
    assert_eq!(widths(&mut h), vec![34, 45]);
    // 最小を割らない所で止める
    h.cmd_with("pane.grow", Args::new().with_int("cells", 100));
    assert_eq!(widths(&mut h), vec![20, 59]);
    run(&mut h, "pane.equalize");
    assert_eq!(widths(&mut h), vec![40, 39]);
    assert_eq!(
        h.cmd_with("pane.grow", Args::new().with_int("cells", 0)),
        CommandResult::Rejected(RejectReason::InvalidArgs)
    );
    check(&h);
}

// verifies: AC-FR-509-4
#[test]
// core のマウス・大きさ・検索バーとオーバーレイ。見出しのクリック (`ChromeMouse`)・既定キーは下の段 2b の試験、TUI の変換は
// eve-tui の `mouse` の単体試験
fn フォーカスのペインが替わると検索バーは閉じ_オーバーレイの間はクリックでフォーカスが変わらない() {
    let mut h = opened(&[("/w/a.txt", &lines(10))]);
    let left = focused(&h);
    run(&mut h, "pane.split_right");
    let right = focused(&h);
    h.cmd("search.open");
    assert!(h.frame().search.is_some());
    via_palette(&mut h, "pane.focus_next");
    assert_eq!(focused(&h), left);
    assert!(h.frame().search.is_none(), "検索バーを閉じる");
    // オーバーレイ (パレット) の間は、ほかのペインのクリックでフォーカスが変わらない
    h.cmd("ui.palette_open");
    h.mouse_in(
        right,
        MouseKind::Down(MouseButton::Left),
        Modifiers::NONE,
        1,
        1,
    );
    h.mouse_in(
        right,
        MouseKind::Up(MouseButton::Left),
        Modifiers::NONE,
        1,
        1,
    );
    h.keys("escape");
    assert_eq!(focused(&h), left);
}

// ---------- 読込とペイン (03 §9.9 の PANE-8 (b)) ----------

#[test]
fn 読込の完了は開いた時点のペインだけを切り替え_フォーカスは移さない() {
    let mut h = opened(&[("/w/a.txt", b"a\n")]);
    h.fs.add_file("/w/b.txt", b"b\n");
    let left = focused(&h);
    run(&mut h, "pane.split_right");
    let right = focused(&h);
    run(&mut h, "pane.focus_prev");
    h.open("/w/b.txt");
    run(&mut h, "pane.focus_next");
    h.run_jobs();
    check(&h);
    let (v, f) = panes(&h);
    assert_eq!(f, right, "フォーカスは移さない");
    assert_eq!(
        doc_of_view(&h, v[0]),
        doc_of(&h, "/w/b.txt"),
        "開いたペイン"
    );
    assert_eq!(v[1], right, "ほかのペインは変えない");
    assert_ne!(v[0], left);
}

#[test]
fn 開いた時点のペインを別の文書に切り替えた後なら_読込の完了で切り替えない() {
    let mut h = opened(&[("/w/c.txt", b"c\n"), ("/w/a.txt", b"a\n")]);
    h.fs.add_file("/w/b.txt", b"b\n");
    run(&mut h, "pane.split_right");
    h.open("/w/b.txt");
    h.open("/w/c.txt");
    h.run_jobs();
    check(&h);
    let (v, _) = panes(&h);
    assert_eq!(doc_of_view(&h, v[0]), doc_of(&h, "/w/a.txt"));
    assert_eq!(doc_of_view(&h, v[1]), doc_of(&h, "/w/c.txt"));
    assert!(
        h.editor.state().document(doc_of(&h, "/w/b.txt")).is_some(),
        "一覧に載るだけ"
    );
}

#[test]
fn 先行表示で切り替えた読込を取り消すと_開いたペインだけ開く前の文書へ戻り_ほかのペインはそのペインの直近へ()
 {
    use eve_core::Millis;
    let mut h = opened(&[("/w/a.txt", b"a\n"), ("/w/b.txt", b"b\n")]);
    h.fs.add_file("/w/d.txt", &lines(100));
    // 左は b → a、右は b
    h.open("/w/a.txt");
    run(&mut h, "pane.split_right");
    h.open("/w/b.txt");
    run(&mut h, "pane.focus_prev");
    // 左で遅い読込を開き、先頭で左を切り替える
    h.fs.set_chunk_size(64);
    h.fs.set_read_delay(Some((h.clock.clone(), Millis(100))));
    h.open("/w/d.txt");
    assert!(h.run_jobs_until_partial(), "先頭が届く");
    let d = doc_of(&h, "/w/d.txt");
    let (v, _) = panes(&h);
    assert_eq!(doc_of_view(&h, v[0]), d);
    // 右のペインでも d を出し、そこで取り消す
    run(&mut h, "pane.focus_next");
    h.open("/w/d.txt");
    assert_eq!(doc_of_view(&h, focused(&h)), d);
    h.keys("esc");
    h.run_jobs();
    check(&h);
    assert!(h.editor.state().document(d).is_none());
    let (v, _) = panes(&h);
    assert_eq!(
        doc_of_view(&h, v[0]),
        doc_of(&h, "/w/a.txt"),
        "開いたペインは開く前の文書"
    );
    assert_eq!(
        doc_of_view(&h, v[1]),
        doc_of(&h, "/w/b.txt"),
        "ほかのペインはその直近"
    );
    h.finish_held_jobs();
    check(&h);
}

// ---------- 段 2a のレビューで直した点 ----------

// verifies: AC-FR-509-2
#[test]
fn ほかのペインで行った編集を戻しても_実行したペインのカーソルは飛ばない() {
    let mut h = opened(&[("/w/a.txt", &lines(100))]);
    let left = focused(&h);
    run(&mut h, "pane.split_right");
    let right = focused(&h);
    h.keys("ctrl+end");
    let end = head(&h, right);
    run(&mut h, "pane.focus_prev");
    h.text("X");
    run(&mut h, "pane.focus_next");
    h.cmd("edit.undo");
    assert_eq!(head(&h, right), end, "右は写像だけ (末尾のまま)");
    assert_eq!(head(&h, left), 0, "左は記録した選択に戻る");
    let top = h.editor.state().view(right).expect("View").scroll.top.0;
    assert_eq!(
        top, 0,
        "戻した変更の先頭 (文書の先頭) が見えるようにスクロールする"
    );
    check(&h);
}

#[test]
fn 開き直すと同じ文書を出すほかのペインの_view_も先頭に戻る() {
    let mut h = opened(&[("/w/a.txt", &lines(200))]);
    let left = focused(&h);
    h.keys("ctrl+end");
    run(&mut h, "pane.split_right");
    h.keys("ctrl+home");
    h.fs.add_file("/w/a.txt", "短い\n".as_bytes());
    h.cmd_with(
        "file.reopen_with_encoding",
        Args::new().with_str("encoding", "utf-8"),
    );
    h.run_jobs();
    check(&h);
    assert_eq!(h.doc_text(), "短い\n");
    assert_eq!(head(&h, left), 0, "実行していないペインの View も戻す");
    run(&mut h, "pane.focus_prev");
    h.text("x");
    assert_eq!(h.doc_text(), "x短い\n", "左の入力が捨てられない");
}

#[test]
fn 表示していない_view_もペインの大きさが変わったら合わせ_戻したときにカーソルが見える() {
    let long = "x".repeat(3000) + "\n";
    let mut h = opened(&[("/w/a.txt", long.as_bytes()), ("/w/b.txt", b"b\n")]);
    h.open("/w/a.txt");
    run(&mut h, "pane.split_right");
    h.cmd_with("pane.shrink", Args::new().with_int("cells", 100));
    h.cmd("view.toggle_wrap");
    // 1 行目の行末 (文書の末尾の改行の前)
    h.keys("ctrl+end left");
    assert!(
        h.frame().panes[1].cursors.len() == 1,
        "折り返した行末のカーソルが見える"
    );
    let right = focused(&h);
    let wrap_idx = |h: &Harness| h.editor.state().view(right).expect("View").scroll.wrap_idx;
    assert!(
        wrap_idx(&h) > 100,
        "20 列で折り返した行の終わりの表示行: {}",
        wrap_idx(&h)
    );
    h.open("/w/b.txt");
    run(&mut h, "pane.equalize");
    h.cmd_with("pane.grow", Args::new().with_int("cells", 100));
    h.open("/w/a.txt");
    check(&h);
    let f = h.frame();
    assert_eq!(f.panes[1].cursors.len(), 1, "戻した a のカーソルが見える");
    assert!(f.primary_cursor.is_some());
}

// verifies: AC-FR-103-2
#[test]
fn _1_ペインで別の文書に切り替えて戻してもスクロールの位置が保たれる() {
    let mut h = opened(&[("/w/a.txt", &lines(200)), ("/w/b.txt", b"b\n")]);
    h.open("/w/a.txt");
    let a = focused(&h);
    // ホイールでカーソルから離れた所までスクロールする
    for _ in 0..10 {
        h.mouse_in(a, MouseKind::WheelDown, Modifiers::NONE, 1, 1);
    }
    let top = |h: &Harness| h.editor.state().view(a).expect("View").scroll.top.0;
    let before = top(&h);
    assert!(before > 0);
    run(&mut h, "buffer.next");
    run(&mut h, "buffer.next");
    assert_eq!(focused(&h), a, "a の View に戻る");
    assert_eq!(top(&h), before, "切り替えて戻してもスクロールを変えない");
}

// ---------- 段 2b: 見出し・区切り・既定キー・見出しのクリック・ASCII の Frame ----------

/// `keys` を押して、ペインの不変条件を確かめる。
fn press(h: &mut Harness, keys: &str) {
    h.keys(keys);
    check(h);
}

/// 見出しの項目の長い形を空白で繋いだもの (見出しが無ければ `None`)。
fn bar_text(p: &eve_view::PaneFrame) -> Option<String> {
    p.bar.as_ref().map(|b| {
        b.items
            .iter()
            .map(|i| cell_text(&i.long))
            .collect::<Vec<_>>()
            .join(" ")
    })
}

/// ペイン `p` の `line` 行目 (0 始まり) のガターの行番号が太字か。
fn line_number_bold(p: &eve_view::PaneFrame, line: u32) -> bool {
    p.rows
        .iter()
        .find(
            |r| matches!(r.source, eve_view::RowSource::Line { line: l, wrap_idx: 0 } if l == line),
        )
        .and_then(|r| r.gutter.first())
        .is_some_and(|s| s.style.mods.contains(eve_view::StyleMods::BOLD))
}

// verifies: AC-FR-509-1
#[test]
fn 見出しと区切りが出て_フォーカスのペインの見出しを強調し_ほかのペインは現在行の行番号を強調しない()
 {
    let mut h = opened(&[("/w/a.txt", &lines(30))]);
    // 1 ペインでは見出しも区切りも出さない (画面は v0.4 と同じ)
    let f = h.frame();
    assert_eq!(f.dividers, Vec::new());
    assert!(f.panes[0].bar.is_none() && f.panes[0].active);
    let left = focused(&h);
    run(&mut h, "pane.split_right");
    let right = focused(&h);
    let f = h.frame();
    assert_eq!(f.panes.len(), 2);
    let (l, r) = (&f.panes[0], &f.panes[1]);
    assert_eq!((l.view, r.view), (left, right));
    assert!(
        !l.active && r.active,
        "Window のフォーカスのペインだけが active"
    );
    assert!(!l.focused && r.focused);
    // 見出しは各ペインの最下行 (本文の直下)、区切りは左右の間の 1 列で見出しの行も通す
    let (lb, rb) = (
        l.bar.as_ref().expect("左の見出し"),
        r.bar.as_ref().expect("右の見出し"),
    );
    assert_eq!((lb.rect.row, lb.rect.rows), (l.rect.rows, 1));
    assert_eq!((lb.rect.col, lb.rect.cols), (l.rect.col, l.rect.cols));
    assert_eq!((rb.rect.col, rb.rect.cols), (r.rect.col, r.rect.cols));
    assert_eq!(f.dividers.len(), 1);
    let d = f.dividers[0].rect;
    assert_eq!((d.col, d.cols, d.row, d.rows), (40, 1, 0, 24));
    // 見出しの項目は名前 (番号を付けない)・変更・読み取り専用
    assert_eq!(bar_text(l).as_deref(), Some("a.txt"));
    h.text("x");
    let f = h.frame();
    assert_eq!(bar_text(&f.panes[0]).as_deref(), Some("a.txt [+]"));
    assert_eq!(bar_text(&f.panes[1]).as_deref(), Some("a.txt [+]"));
    let kinds: Vec<StatusKind> = f.panes[1]
        .bar
        .as_ref()
        .map(|b| b.items.iter().map(|i| i.kind).collect())
        .unwrap_or_default();
    assert_eq!(kinds, vec![StatusKind::Path, StatusKind::Dirty]);
    // フォーカスの無いペインは現在行の行番号を太字にしない (03 §9.9 の PANE-10 (d))
    assert!(line_number_bold(&f.panes[1], 0));
    assert!(!line_number_bold(&f.panes[0], 0));
    // オーバーレイを開いても active は変わらない (focused は偽になる)
    h.cmd("ui.palette_open");
    let f = h.frame();
    assert!(f.panes[1].active && !f.panes[1].focused);
    assert!(!f.panes[0].active);
    h.keys("escape");
}

// verifies: AC-FR-509-1
#[test]
fn 既定キーで分割し_次_前へ移り_大きさを変えて揃え_ペインを閉じる() {
    let mut h = opened(&[("/w/a.txt", &lines(30))]);
    let first = focused(&h);
    press(&mut h, "ctrl+\\");
    let (v, second) = panes(&h);
    assert_eq!(v, vec![first, second], "Ctrl+\\ で右に分割");
    press(&mut h, "ctrl+k -");
    let (v, third) = panes(&h);
    assert_eq!(v, vec![first, second, third], "Ctrl+K - で下に分割");
    press(&mut h, "f6");
    assert_eq!(focused(&h), first, "F6 で次 (末尾から先頭へ巡る)");
    press(&mut h, "shift+f6");
    assert_eq!(focused(&h), third, "Shift+F6 で前");
    press(&mut h, "ctrl+k tab");
    assert_eq!(focused(&h), first, "Ctrl+K Tab で次");
    press(&mut h, "ctrl+k shift+tab");
    assert_eq!(focused(&h), third, "Ctrl+K Shift+Tab で前");
    let rows =
        |h: &mut Harness| -> Vec<u16> { h.frame().panes.iter().map(|p| p.rect.rows).collect() };
    let before = rows(&mut h);
    press(&mut h, "ctrl+k ]");
    let grown = rows(&mut h);
    assert!(
        grown[2] > before[2],
        "Ctrl+K ] で広げる: {before:?} → {grown:?}"
    );
    press(&mut h, "ctrl+k [");
    press(&mut h, "ctrl+k [");
    let shrunk = rows(&mut h);
    assert!(
        shrunk[2] < grown[2],
        "Ctrl+K [ で狭める: {grown:?} → {shrunk:?}"
    );
    press(&mut h, "ctrl+k =");
    assert_eq!(rows(&mut h), before, "Ctrl+K = で揃える");
    press(&mut h, "ctrl+k q");
    assert_eq!(
        panes(&h).0,
        vec![first, second],
        "Ctrl+K Q でペインを閉じる"
    );
    assert_eq!(h.editor.state().documents_len(), 1, "文書は閉じない");
    // `\` の代替 (`Ctrl+K \` と `Ctrl+K |`)
    press(&mut h, "ctrl+k q");
    assert_eq!(panes(&h).0.len(), 1);
    press(&mut h, "ctrl+k \\");
    assert_eq!(panes(&h).0.len(), 2);
    press(&mut h, "ctrl+k |");
    assert_eq!(panes(&h).0.len(), 3);
    // 最後のペインで閉じるキーを押すと、未割当ではなく理由を知らせる
    press(&mut h, "ctrl+k q");
    press(&mut h, "ctrl+k q");
    h.clear_log();
    press(&mut h, "ctrl+k q");
    assert_eq!(panes(&h).0.len(), 1);
    assert!(
        h.messages().contains(&"info.pane.single_pane".to_owned()),
        "{:?}",
        h.messages()
    );
}

// verifies: AC-FR-509-4
#[test]
fn 見出しのクリックでフォーカスだけが移り_オーバーレイの間は無視する() {
    let mut h = opened(&[("/w/a.txt", &lines(30)), ("/w/b.txt", b"bee\n")]);
    h.open("/w/a.txt");
    let left = focused(&h);
    run(&mut h, "pane.split_right");
    let right = focused(&h);
    h.keys("down down");
    let (left_head, right_head) = (head(&h, left), head(&h, right));
    let down = MouseKind::Down(MouseButton::Left);
    // オーバーレイの間は無視する
    h.cmd("ui.palette_open");
    h.pane_bar_mouse(left, down, Modifiers::NONE, 3);
    h.keys("escape");
    assert_eq!(focused(&h), right);
    // 右ボタン・ホイールは何もしない
    h.pane_bar_mouse(
        left,
        MouseKind::Down(MouseButton::Right),
        Modifiers::NONE,
        3,
    );
    h.pane_bar_mouse(left, MouseKind::WheelDown, Modifiers::NONE, 3);
    assert_eq!(focused(&h), right);
    // 左の押下でフォーカスだけが移る (カーソルは動かない)
    h.pane_bar_mouse(left, down, Modifiers::NONE, 3);
    assert_eq!(focused(&h), left);
    assert_eq!((head(&h, left), head(&h, right)), (left_head, right_head));
    // Frame の後に右のペインが別の文書に切り替わっても、古い View がそのペインの表に残っていれば同じペインを指す
    run(&mut h, "pane.focus_next");
    h.open("/w/b.txt");
    let b_view = focused(&h);
    run(&mut h, "pane.focus_prev");
    assert_eq!(focused(&h), left);
    h.pane_bar_mouse(right, down, Modifiers::NONE, 0);
    assert_eq!(focused(&h), b_view, "右のペイン (今は b を表示) へ");
    // 閉じたペインの View を指す入力は捨てる
    run(&mut h, "pane.close");
    let only = focused(&h);
    h.pane_bar_mouse(b_view, down, Modifiers::NONE, 0);
    assert_eq!(focused(&h), only);
    check(&h);
}

// verifies: AC-FR-509-3
#[test]
fn 縮退表示の_frame_には見出しも区切りも出さない() {
    let mut h = opened(&[("/w/a.txt", &lines(30))]);
    run(&mut h, "pane.split_right");
    h.resize(30, 24);
    let f = h.frame();
    assert_eq!(f.panes.len(), 1);
    assert!(f.panes[0].bar.is_none() && f.panes[0].active);
    assert_eq!(f.dividers, Vec::new());
}

// ---------- ASCII の Frame のゴールデン (09 §3.4 の複数ペインの計画の (3)) ----------

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

/// `/w/a.txt` (12 行) を開いて 3 行目へ移った、`cols` × `rows` の Harness。
fn ascii_base(cols: u16, rows: u16) -> Harness {
    let mut b = Harness::builder().size(cols, rows);
    b = b.file("/w/a.txt", &lines(12));
    let mut h = b.build();
    h.open("/w/a.txt");
    h.run_jobs();
    h.keys("down down");
    h
}

// verifies: AC-FR-509-1
#[test]
fn ascii_の_frame_左右に並べた_2_つのペイン() {
    let mut h = ascii_base(80, 24);
    press(&mut h, "ctrl+\\");
    h.text("x");
    check_frame(&mut h, "panes_left_right");
}

// verifies: AC-FR-509-1
#[test]
fn ascii_の_frame_上下に並べた_2_つのペイン() {
    let mut h = ascii_base(80, 24);
    press(&mut h, "ctrl+k -");
    check_frame(&mut h, "panes_up_down");
}

// verifies: AC-FR-509-1
#[test]
fn ascii_の_frame_入れ子の_3_つのペインと同じ名前の区別() {
    let mut b = Harness::builder().size(80, 24);
    for p in ["/w/a.txt", "/p/a/mod.rs", "/p/b/mod.rs"] {
        b = b.file(p, &lines(12));
    }
    let mut h = b.build();
    h.open("/w/a.txt");
    h.run_jobs();
    press(&mut h, "ctrl+\\");
    h.open("/p/a/mod.rs");
    h.run_jobs();
    press(&mut h, "ctrl+k -");
    h.open("/p/b/mod.rs");
    h.run_jobs();
    press(&mut h, "f6");
    check_frame(&mut h, "panes_nested");
}

// verifies: AC-FR-509-1
#[test]
fn ascii_の_frame_最小の前後と縮退表示() {
    // 41 列: 左右に分けた最小 (20 + 区切り 1 + 20)
    let mut h = ascii_base(41, 8);
    press(&mut h, "ctrl+\\");
    assert_eq!(panes(&h).0.len(), 2);
    check_frame(&mut h, "panes_min");
    // 40 列に縮めると縮退表示 (フォーカスのペインだけ。見出しと区切りなし)
    h.resize(40, 8);
    check(&h);
    check_frame(&mut h, "panes_collapsed");
}
