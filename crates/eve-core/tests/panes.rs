//! 複数ペイン (docs/03 §9.9、01 §5.5 の FR-509) のシナリオ (09 §3.4 の計画の (2))。v0.5 の M30 の段 2a: core の木の
//! 操作と PANE-4〜PANE-9 を、`execute` とパレットから実行する。見出し・区切りの描画・既定キー・見出しのクリック・
//! ASCII の Frame は段 2b で確かめる。

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
fn 右と下に分割し_次_前と方向でフォーカスを移せる() {
    // 見出し・区切り・フォーカスの強調の描画は段 2b (ここでは木とフォーカスと、全ペインが Frame に出ることを見る)
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
    // Undo は文書に 1 つ: 右から戻しても左から戻しても同じ本文
    run(&mut h, "pane.focus_next");
    h.cmd("edit.undo");
    let from_right = h.doc_text();
    h.cmd("edit.redo");
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
