//! ジャンプ履歴 (FR-310、docs/03 §5.9 の JMP-1〜JMP-7、07 §11.6 の「編集の拡張」、01 §5.5 の FR-310 AC-1) のシナリオ
//! (03 §15.9.1 の L3。M34 の段 8)。

use eve_core::{Args, CommandResult, InitialFile};
use eve_testkit::Harness;
use eve_text::{ByteOff, Range, Selection, TextRead};
use eve_view::{Answer, DocumentId, Modifiers, MouseKind, Query};

const NO_BACK: &str = "info.jump.no_back";
const NO_FORWARD: &str = "info.jump.no_forward";

/// `n` 行の本文 (`l001` …。各行 5 バイト、最後の行も改行で終わる)。
fn numbered(n: usize) -> String {
    (1..=n).map(|i| format!("l{i:03}\n")).collect()
}

/// 無題の文書に `text` を入れ、カーソルを先頭に置いた Harness (ログは空)。
fn scene(text: &str) -> Harness {
    let mut h = Harness::new();
    h.paste(text);
    h.set_selection(Selection::point(ByteOff(0)));
    h.clear_log();
    h
}

/// `files` を置き、順に開いた Harness (最後に開いた文書を表示する)。
fn opened(files: &[(&str, &str)]) -> Harness {
    let mut b = Harness::builder();
    for (p, d) in files {
        b = b.file(p, d.as_bytes());
    }
    let mut h = b.build();
    for (p, _) in files {
        h.open(p);
        h.run_jobs();
    }
    h.clear_log();
    h
}

/// フォーカスのペインの (文書, 主の Range の head)。
fn point(h: &Harness) -> (DocumentId, usize) {
    let st = h.editor.state();
    let v = st
        .view(st.window_view(h.window()).expect("View"))
        .expect("View");
    (v.doc, v.selection.primary().head.0)
}

/// フォーカスのペインの主のカーソルの行 (1 始まり)。
fn line(h: &Harness) -> usize {
    let (doc, head) = point(h);
    let st = h.editor.state();
    st.document(doc)
        .expect("文書")
        .text()
        .byte_to_line(ByteOff(head))
        .0
        + 1
}

/// フォーカスのペインの (表示中の文書のパスの末尾の名前, 行)。無題は `-`。
fn place(h: &Harness) -> (String, usize) {
    let name = h
        .doc_path()
        .map(|p| p.replace('\\', "/"))
        .and_then(|p| p.rsplit('/').next().map(str::to_owned))
        .unwrap_or_else(|| "-".to_owned());
    (name, line(h))
}

/// パスの末尾が `name` の文書。
fn doc_of(h: &Harness, name: &str) -> DocumentId {
    let Ok(Answer::Documents(ds)) = h.query(&Query::Documents) else {
        panic!("文書の一覧");
    };
    ds.iter()
        .find(|d| d.path.as_deref().is_some_and(|p| p.ends_with(name)))
        .unwrap_or_else(|| panic!("{name} の文書"))
        .doc
}

/// バッファ一覧で `name` を選んで閉じる (表示していない文書も閉じられる)。
fn close_from_list(h: &mut Harness, name: &str) {
    h.keys("ctrl+e");
    assert_eq!(h.overlay(), "buffer_list");
    h.text(name);
    h.keys("ctrl+w");
    if h.overlay() == "buffer_list" {
        h.keys("esc");
    }
    h.run_jobs();
}

fn goto(h: &mut Harness, line: i64) {
    let r = h.cmd_with("cursor.goto_line", Args::new().with_int("line", line));
    assert_eq!(r, CommandResult::Done);
}

fn goto_col(h: &mut Harness, line: i64, col: i64) {
    let r = h.cmd_with(
        "cursor.goto_line",
        Args::new().with_int("line", line).with_int("col", col),
    );
    assert_eq!(r, CommandResult::Done);
}

/// `id` (`cursor.jump_back` / `cursor.jump_forward`) を 1 回実行し、移ったか (端の知らせが出なかったか)。
fn step(h: &mut Harness, id: &str) -> bool {
    h.clear_log();
    assert_eq!(h.cmd(id), CommandResult::Done);
    let m = h.messages();
    assert!(
        m.iter().all(|k| k == NO_BACK || k == NO_FORWARD) && m.len() <= 1,
        "{m:?}"
    );
    m.is_empty()
}

fn back(h: &mut Harness) -> bool {
    step(h, "cursor.jump_back")
}

fn forward(h: &mut Harness) -> bool {
    step(h, "cursor.jump_forward")
}

/// 戻れる限り戻り、各段の行を返す (端の知らせが出たら止める)。
fn back_lines(h: &mut Harness) -> Vec<usize> {
    let mut out = Vec::new();
    while back(h) {
        out.push(line(h));
        assert!(out.len() <= 200, "戻り続ける");
    }
    assert_eq!(h.messages(), [NO_BACK]);
    out
}

/// 進める限り進み、各段の行を返す。
fn forward_lines(h: &mut Harness) -> Vec<usize> {
    let mut out = Vec::new();
    while forward(h) {
        out.push(line(h));
        assert!(out.len() <= 200, "進み続ける");
    }
    assert_eq!(h.messages(), [NO_FORWARD]);
    out
}

fn check(h: &Harness) {
    if let Err(e) = h.editor.state().check_panes() {
        panic!("ペインとジャンプ履歴の不変条件: {e}");
    }
}

/// 10 行目の先頭が `(`、20 行目の先頭が `)` の 100 行の本文。
fn bracketed() -> String {
    (1..=100)
        .map(|i| match i {
            10 => "(a\n".to_owned(),
            20 => ")b\n".to_owned(),
            _ => format!("l{i:03}\n"),
        })
        .collect()
}

// ---------- 01 FR-310 AC-1 ----------

// verifies: AC-FR-310-1
#[test]
fn 行へ移動_文書の端_対応括弧の前の位置へ戻り_進める() {
    let mut h = scene(&bracketed());
    goto(&mut h, 50);
    h.keys("ctrl+end");
    assert_eq!(line(&h), 101);
    h.keys("ctrl+home");
    goto(&mut h, 10);
    h.keys("ctrl+]");
    assert_eq!(line(&h), 20, "対応括弧へ移る");
    check(&h);
    // 戻ると、今の位置 (20 行目) を末尾に記録してから、各移動の前の位置へ順に戻る
    assert_eq!(back_lines(&mut h), [10, 1, 101, 50, 1]);
    // 進むと逆の順で、最後は戻り始めた位置
    assert_eq!(forward_lines(&mut h), [50, 101, 1, 10, 20]);
    check(&h);
}

// verifies: AC-FR-310-1
#[test]
fn 検索バーを開いてから閉じるまでの続けた移動は最初の移動の前の_1_項目だけ() {
    let text: String = (1..=50)
        .map(|i| {
            if i % 10 == 0 {
                format!("aa hit {i}\n")
            } else {
                format!("l{i:03}\n")
            }
        })
        .collect();
    let mut h = scene(&text);
    h.keys("ctrl+f");
    h.text("hit");
    h.keys("enter");
    assert_eq!(line(&h), 10);
    h.keys("enter");
    h.keys("enter");
    h.keys("up");
    h.keys("enter");
    assert_eq!(line(&h), 30);
    h.keys("esc");
    // 検索を始める前の位置へ 1 回で戻る
    assert_eq!(back_lines(&mut h), [1]);
    assert_eq!(forward_lines(&mut h), [30]);
    // 閉じて開き直すと、新しい検索の最初の移動の前の位置を記録する
    h.keys("ctrl+f");
    h.text("hit");
    h.keys("enter");
    assert_eq!(line(&h), 40);
    h.keys("enter enter");
    assert_eq!(line(&h), 10, "端を越えて先頭の一致へ");
    h.keys("esc");
    assert_eq!(back_lines(&mut h), [30, 1]);
}

// verifies: AC-FR-310-1
#[test]
fn 文書の切り替えの前の位置へ別の文書を含めて戻る() {
    let a = numbered(20);
    let b = numbered(30);
    let mut h = opened(&[("/w/a.txt", &a), ("/w/b.txt", &b)]);
    assert_eq!(place(&h), ("b.txt".to_owned(), 1));
    goto(&mut h, 7);
    assert_eq!(h.cmd("buffer.prev"), CommandResult::Done);
    assert_eq!(place(&h), ("a.txt".to_owned(), 1));
    goto(&mut h, 5);
    let mut seen = Vec::new();
    while back(&mut h) {
        seen.push(place(&h));
    }
    let want = |v: &[(&str, usize)]| -> Vec<(String, usize)> {
        v.iter().map(|(n, l)| ((*n).to_owned(), *l)).collect()
    };
    assert_eq!(
        seen,
        want(&[("a.txt", 1), ("b.txt", 7), ("b.txt", 1), ("a.txt", 1)]),
        "行へ移動・バッファの切り替え・開いた文書の切り替えの前の位置"
    );
    let mut seen = Vec::new();
    while forward(&mut h) {
        seen.push(place(&h));
    }
    assert_eq!(
        seen,
        want(&[("b.txt", 1), ("b.txt", 7), ("a.txt", 1), ("a.txt", 5)])
    );
    check(&h);
}

// verifies: AC-FR-310-1
#[test]
fn 編集の後も位置は本文に合わせて動き_閉じた文書の位置は消える() {
    let mut h = opened(&[("/w/a.txt", &numbered(80)), ("/w/b.txt", &numbered(80))]);
    // b: 30 行目と 50 行目を記録する
    goto(&mut h, 30);
    goto(&mut h, 50);
    goto(&mut h, 60);
    // 先頭に 5 行を足す (記録しない移動で先頭へ)
    h.set_selection(Selection::point(ByteOff(0)));
    h.text("x\nx\nx\nx\nx\n");
    assert_eq!(line(&h), 6);
    let mut seen = Vec::new();
    while back(&mut h) {
        seen.push(place(&h));
    }
    let want: Vec<(String, usize)> = [("b.txt", 55), ("b.txt", 35), ("b.txt", 1), ("a.txt", 1)]
        .iter()
        .map(|(n, l)| ((*n).to_owned(), *l))
        .collect();
    assert_eq!(seen, want, "足した 5 行の分だけ下へ");
    // 表示している a を閉じる: a の項目は消える
    close_from_list(&mut h, "a.txt");
    assert_eq!(h.editor.state().documents_len(), 1);
    assert_eq!(place(&h).0, "b.txt");
    check(&h);
    let mut seen = Vec::new();
    while forward(&mut h) {
        seen.push(place(&h));
    }
    while back(&mut h) {
        seen.push(place(&h));
    }
    // a の項目 (先頭) を消して詰めたので、今の位置は b の 1 行目の項目。b の項目だけを進み、戻る
    let want: Vec<(String, usize)> = [35, 55, 6, 55, 35, 1]
        .iter()
        .map(|l| ("b.txt".to_owned(), *l))
        .collect();
    assert_eq!(seen, want, "閉じた文書へは戻らない");
    assert_eq!(h.messages(), [NO_BACK], "端ではその旨を示す");
}

// ---------- JMP-1: 記録する移動・しない移動 ----------

#[test]
fn jmp1_矢印_単語_ページ_クリック_スクロール_選択の拡張と戻る自身は記録しない() {
    let mut h = scene(&numbered(200));
    h.keys("down down right ctrl+right pagedown pagedown pageup");
    h.click(5, 2);
    h.mouse(MouseKind::WheelDown, Modifiers::NONE, 3, 3);
    h.keys("shift+down ctrl+shift+end ctrl+shift+home");
    h.keys("esc");
    h.cmd("select.expand");
    h.cmd("cursor.jump_back");
    h.cmd("cursor.jump_forward");
    h.text("abc");
    h.keys("ctrl+z");
    check(&h);
    assert!(!back(&mut h), "どれもジャンプにしない");
    assert!(!forward(&mut h));
}

#[test]
fn jmp1_位置が変わらない移動は記録しない() {
    let mut h = scene(&bracketed());
    goto(&mut h, 1);
    h.keys("ctrl+home");
    // 括弧の無い位置の対応括弧 (動かない)
    h.keys("ctrl+]");
    assert!(h.messages().contains(&"info.bracket.none".to_owned()));
    assert!(!back(&mut h));
    // 文書の末尾にいて文書の末尾へ
    h.keys("ctrl+end");
    h.keys("ctrl+end");
    assert_eq!(back_lines(&mut h), [1]);
}

/// 10 行目・20 行目・30 行目が `aa foo N` (ほかは `lNNN`) の本文。10 行目の行頭は 45、20 行目の行頭は 100。
fn foo_lines() -> String {
    (1..=30)
        .map(|i| {
            if i % 10 == 0 {
                format!("aa foo {i}\n")
            } else {
                format!("l{i:03}\n")
            }
        })
        .collect()
}

#[test]
fn jmp1_検索の移動なしの置換は置換の前の位置を記録する() {
    let mut h = scene(&foo_lines());
    h.keys("ctrl+r");
    h.text("foo");
    h.keys("tab");
    h.text("X");
    // 入力中の一致 (10 行目) を置き換え、カーソルは置き換えた文字列の後ろ
    h.keys("enter");
    assert!(h.doc_text().contains("aa X 10\n"), "{}", h.doc_text());
    assert_eq!(point(&h).1, 45 + 4);
    // 続けた置換は記録しない
    h.keys("enter");
    assert!(h.doc_text().contains("aa X 20\n"), "{}", h.doc_text());
    h.keys("esc");
    assert_eq!(line(&h), 20);
    assert!(back(&mut h));
    assert_eq!(point(&h).1, 0, "置換の前の位置 (先頭)");
    assert!(!back(&mut h));
}

#[test]
fn jmp1_検索の移動の後の置換は検索の移動と合わせて_1_項目() {
    let mut h = scene(&foo_lines());
    h.keys("ctrl+r");
    h.text("foo");
    h.keys("enter");
    assert_eq!(line(&h), 10, "検索の移動");
    h.keys("enter");
    assert_eq!(line(&h), 20);
    h.keys("tab");
    h.text("X");
    h.keys("enter");
    assert!(h.doc_text().contains("aa X 20\n"), "{}", h.doc_text());
    assert!(h.doc_text().contains("aa foo 10\n"), "{}", h.doc_text());
    assert_eq!(point(&h).1, 100 + 4);
    h.keys("esc");
    assert!(back(&mut h));
    assert_eq!(point(&h).1, 0, "検索を始める前の位置");
    assert!(!back(&mut h));
}

#[test]
fn jmp1_バッファ一覧の選択_タブのクリック_新しい文書は切り替える前の位置を記録する() {
    let mut h = opened(&[("/w/a.txt", &numbered(20)), ("/w/b.txt", &numbered(20))]);
    goto(&mut h, 3);
    // バッファ一覧で a を選ぶ
    h.keys("ctrl+e");
    assert_eq!(h.overlay(), "buffer_list");
    h.text("a.txt");
    h.keys("enter");
    assert_eq!(place(&h).0, "a.txt");
    goto(&mut h, 4);
    // タブのクリックで b へ
    h.set_tab_line(eve_config::TabLineMode::Always);
    let b = doc_of(&h, "b.txt");
    h.click_tab(b);
    assert_eq!(place(&h), ("b.txt".to_owned(), 3));
    // 新しい無題の文書を出す
    h.keys("ctrl+n");
    assert_eq!(place(&h).0, "-");
    let mut seen = Vec::new();
    while back(&mut h) {
        seen.push(place(&h));
    }
    let want: Vec<(String, usize)> = [("b.txt", 3), ("a.txt", 4), ("a.txt", 1), ("b.txt", 3)]
        .iter()
        .map(|(n, l)| ((*n).to_owned(), *l))
        .collect();
    assert_eq!(seen[..4], want[..]);
}

/// `/w/proj` (`.git` のあるリポジトリ) に `files` を置き、`/w/proj/README.md` を開いて起動する。
fn repo(files: &[(&str, &[u8])]) -> Harness {
    let mut b = Harness::builder().defer_start().dir("/w/proj/.git");
    b = b.file("/w/proj/README.md", b"readme\nr2\nr3\n");
    for (p, data) in files {
        b = b.file(p, data);
    }
    let mut h = b.build();
    h.editor
        .open_initial_files(&[InitialFile::path("/w/proj/README.md")], false, None);
    h.started();
    h.run_jobs();
    h.clear_log();
    h
}

#[test]
fn jmp1_ファインダと横断検索の結果へのジャンプは前の位置を記録する() {
    let mut h = repo(&[
        ("/w/proj/a.txt", b"one\ntwo\nthree hit\n"),
        ("/w/proj/b.txt", b"b1\nb2\n"),
    ]);
    goto(&mut h, 2);
    // ファインダで b.txt を開く (読込の完了で切り替える)
    h.keys("ctrl+o");
    h.run_jobs();
    h.text("b.txt");
    h.run_jobs();
    h.keys("enter");
    h.run_jobs();
    assert_eq!(place(&h), ("b.txt".to_owned(), 1));
    goto(&mut h, 2);
    // 横断検索で a.txt の一致へ (新しく開く)
    h.keys("ctrl+shift+f");
    h.text("hit");
    h.run_jobs();
    h.keys("enter");
    h.run_jobs();
    assert_eq!(place(&h), ("a.txt".to_owned(), 3));
    // 今の文書の中の一致へ (開き直さずに移る)
    h.keys("ctrl+home");
    h.keys("ctrl+shift+f enter");
    h.run_jobs();
    assert_eq!(place(&h), ("a.txt".to_owned(), 3));
    let mut seen = Vec::new();
    while back(&mut h) {
        seen.push(place(&h));
    }
    let want: Vec<(String, usize)> = [
        ("a.txt", 1),
        ("a.txt", 3),
        ("b.txt", 2),
        ("b.txt", 1),
        ("README.md", 2),
        ("README.md", 1),
    ]
    .iter()
    .map(|(n, l)| ((*n).to_owned(), *l))
    .collect();
    assert_eq!(seen, want);
    check(&h);
}

// ---------- JMP-2: 持ち方 ----------

#[test]
fn jmp2_上限は_100_項目で古いものから捨てる() {
    let mut h = scene(&numbered(300));
    for l in 2..=106 {
        goto(&mut h, l);
    }
    let lines = back_lines(&mut h);
    assert_eq!(lines.len(), 99, "末尾の今の位置を含めて 100 項目");
    assert_eq!(lines.first(), Some(&105));
    assert_eq!(lines.last(), Some(&7));
    check(&h);
}

#[test]
fn jmp2_同じ行は重ねず_戻った後の新しい記録で先を捨てる() {
    let mut h = scene(&numbered(100));
    goto(&mut h, 10);
    goto_col(&mut h, 10, 3);
    goto(&mut h, 20);
    // 10 行目の 2 項目 (1 桁目・3 桁目) は 1 つになり、後の位置 (3 桁目) が残る
    assert!(back(&mut h));
    assert_eq!((line(&h), point(&h).1), (10, 9 * 5 + 2));
    assert!(back(&mut h));
    assert_eq!(line(&h), 1);
    assert!(!back(&mut h));
    // 1 行目に戻った後に移ると、先 (10 行目・20 行目) を捨てる
    goto(&mut h, 40);
    assert_eq!(back_lines(&mut h), [1]);
    assert_eq!(forward_lines(&mut h), [40]);
}

#[test]
fn jmp2_jmp3_今と同じ行の項目は飛ばす() {
    let mut h = scene(&numbered(100));
    goto(&mut h, 20);
    goto(&mut h, 30);
    // 記録しない移動で 20 行目へ戻ってから戻る: 20 行目の項目を飛ばして 1 行目へ
    for _ in 0..10 {
        h.keys("up");
    }
    h.keys("right");
    assert_eq!(line(&h), 20);
    assert!(back(&mut h));
    assert_eq!(line(&h), 1);
    // 進むと戻り始めた位置 (20 行目の 2 桁目)
    assert!(forward(&mut h));
    assert_eq!((line(&h), point(&h).1), (20, 19 * 5 + 1));
    assert!(!forward(&mut h));
}

#[test]
fn jmp2_分割したペインは元の履歴を写し_ペインごとに別に記録し_閉じたペインの履歴は捨てる() {
    let mut h = scene(&numbered(100));
    goto(&mut h, 10);
    goto(&mut h, 20);
    assert_eq!(h.cmd("pane.split_right"), CommandResult::Done);
    // 新しいペインでの移動は、元のペインの履歴に入らない
    goto(&mut h, 70);
    goto(&mut h, 80);
    check(&h);
    assert_eq!(
        back_lines(&mut h),
        [70, 20, 10, 1],
        "写した履歴に続けて記録する"
    );
    h.cmd("pane.focus_next");
    assert_eq!(line(&h), 20);
    assert_eq!(back_lines(&mut h), [10, 1], "元のペインの履歴");
    // 新しいペインを閉じても、元のペインの履歴は変わらない
    h.cmd("pane.focus_next");
    assert_eq!(h.cmd("pane.close"), CommandResult::Done);
    check(&h);
    assert_eq!(line(&h), 1);
    assert_eq!(forward_lines(&mut h), [10, 20]);
}

// ---------- JMP-3: 戻る・進む ----------

#[test]
fn jmp3_戻ると選択を解いた_1_つのカーソルを置き_行を中央へ置く() {
    let mut h = scene(&numbered(300));
    goto(&mut h, 150);
    let centered = {
        let st = h.editor.state();
        st.view(st.window_view(h.window()).expect("View"))
            .expect("View")
            .scroll
            .top
    };
    goto(&mut h, 280);
    goto(&mut h, 2);
    // 複数のカーソルと選択
    h.set_selection(
        Selection::new(
            vec![
                Range::new(ByteOff(5), ByteOff(8)),
                Range::new(ByteOff(20), ByteOff(22)),
            ],
            0,
        )
        .expect("Selection"),
    );
    assert!(back(&mut h));
    assert_eq!(line(&h), 280);
    assert!(back(&mut h));
    assert_eq!(line(&h), 150);
    let st = h.editor.state();
    let v = st
        .view(st.window_view(h.window()).expect("View"))
        .expect("View");
    assert_eq!(v.selection.len(), 1, "複数カーソルは 1 つにする");
    assert!(v.selection.primary().is_empty(), "選択なし");
    assert_eq!(v.scroll.top, centered, "行へ移動と同じく中央へ置く");
}

#[test]
fn jmp3_空の履歴と端では何もせず知らせる() {
    let mut h = scene("abc\n");
    let before = point(&h);
    assert!(!back(&mut h));
    assert_eq!(h.messages(), [NO_BACK]);
    assert!(!forward(&mut h));
    assert_eq!(h.messages(), [NO_FORWARD]);
    assert_eq!(point(&h), before);
    goto(&mut h, 2);
    assert!(!forward(&mut h), "戻っていなければ進めない");
    assert!(back(&mut h));
    assert!(!back(&mut h));
    assert_eq!(point(&h), before);
}

// ---------- JMP-4: 位置の写像 ----------

#[test]
fn jmp4_ほかのペインの編集と範囲の削除と書記素の境界でも位置を写す() {
    let mut h = scene(&numbered(100));
    goto(&mut h, 40);
    goto(&mut h, 60);
    goto(&mut h, 90);
    h.cmd("pane.split_right");
    // 新しいペインで 1 行目に 3 行を足す (元のペインの項目も写る)
    h.set_selection(Selection::point(ByteOff(0)));
    h.text("y\ny\ny\n");
    h.cmd("pane.focus_next");
    assert_eq!(line(&h), 93, "元のペインのカーソルも写る");
    // 項目 (63 行目) を含む範囲 (50〜69 行目) を消す: 消した範囲の始まり (50 行目) へ寄る。行の始まりは、足した 3 行が
    // 2 バイトずつ、ほかが 5 バイトずつ
    let start = |l: usize| 3 * 2 + (l - 4) * 5;
    h.set_selection(Selection::single(Range::new(
        ByteOff(start(50)),
        ByteOff(start(70)),
    )));
    h.keys("delete");
    check(&h);
    h.keys("down down");
    assert_eq!(back_lines(&mut h), [50, 43, 1]);
    // 書記素の境界: 項目の位置に結合文字を足しても、境界に収める
    let mut h = scene("ab\ncd\nef\n");
    goto_col(&mut h, 2, 2);
    goto(&mut h, 3);
    h.set_selection(Selection::point(ByteOff(4)));
    h.text("\u{0301}");
    assert_eq!(h.doc_text(), "ab\nc\u{0301}d\nef\n");
    check(&h);
    h.keys("down");
    assert_eq!(line(&h), 3);
    assert!(back(&mut h));
    let (_, head) = point(&h);
    assert!(head == 3 || head == 6, "書記素の境界: {head}");
}

#[test]
fn jmp4_外部変更の再読込とその_undo_redo_は同じ行と桁へ写す() {
    let mut h = opened(&[("/w/a.txt", "a1\na2\na3 xyz\na4\n")]);
    // 3 行目の 4 桁目 (9) を記録し、4 行目 (13) にいる
    goto_col(&mut h, 3, 4);
    goto(&mut h, 4);
    // 先頭の 2 行が長くなった内容で読み直す: 項目は同じ行:桁 (3 行目の 4 桁目 = 8 + 3)
    h.fs.add_file("/w/a.txt", b"nn1\nnn2\nbb3 qrs\nb4\nb5\n");
    h.cmd("file.reload");
    h.run_jobs();
    assert_eq!(h.doc_text(), "nn1\nnn2\nbb3 qrs\nb4\nb5\n");
    check(&h);
    // 再読込の Undo (全置換) も同じ行:桁で写す (先頭に潰れない)
    h.cmd("edit.undo");
    assert_eq!(h.doc_text(), "a1\na2\na3 xyz\na4\n");
    check(&h);
    assert_eq!(point(&h).1, 9 + 4, "カーソルは再読込の前 (4 行目)");
    assert!(back(&mut h));
    assert_eq!((line(&h), point(&h).1), (3, 9), "Undo の後の項目");
    assert!(forward(&mut h));
    assert_eq!(point(&h).1, 13);
    // Redo も同じ
    h.cmd("edit.redo");
    assert_eq!(h.doc_text(), "nn1\nnn2\nbb3 qrs\nb4\nb5\n");
    check(&h);
    assert_eq!(line(&h), 4);
    assert!(back(&mut h));
    assert_eq!((line(&h), point(&h).1), (3, 8 + 3), "Redo の後の項目");
    assert!(back(&mut h));
    assert_eq!(point(&h).1, 0);
}

#[test]
fn jmp4_開き直しも同じ行と桁へ写す() {
    // `é` (UTF-8 で 2 バイト) を Shift_JIS で開き直すと半角カナ 2 文字 (UTF-8 で 3 バイトずつ) になる
    let mut h = opened(&[("/w/a.txt", "l1\n\u{e9}\u{e9} x\nl3\nl4\n")]);
    goto_col(&mut h, 2, 4);
    assert_eq!(point(&h).1, 3 + 5, "2 行目の 4 桁目 (x)");
    goto(&mut h, 4);
    h.cmd_with(
        "file.reopen_with_encoding",
        Args::new().with_str("encoding", "shift_jis"),
    );
    h.run_jobs();
    assert_eq!(
        h.doc_text(),
        "l1\n\u{ff83}\u{ff69}\u{ff83}\u{ff69} x\nl3\nl4\n"
    );
    check(&h);
    assert_eq!(point(&h).1, 0, "View は先頭に戻る (PANE-8 (e))");
    assert!(back(&mut h));
    assert_eq!(
        (line(&h), point(&h).1),
        (2, 3 + 9),
        "2 行目の 4 桁目 (2 つ目の `ｩ`)"
    );
}

#[test]
fn jmp4_普通の編集の_undo_と_redo_でも位置を写す() {
    for redo in [false, true] {
        let mut h = scene(&numbered(100));
        goto(&mut h, 30);
        goto(&mut h, 50);
        h.set_selection(Selection::point(ByteOff(0)));
        h.text("xy\n");
        h.cmd("edit.undo");
        let mut want = vec![30, 1];
        if redo {
            h.cmd("edit.redo");
            want[0] = 31;
            assert_eq!(line(&h), 2);
        } else {
            assert_eq!(line(&h), 1);
        }
        check(&h);
        assert_eq!(back_lines(&mut h), want, "redo {redo}");
    }
}

// ---------- JMP-5: 閉じた文書 ----------

#[test]
fn jmp5_閉じた文書の項目は全ペインから消え_今の位置を詰める() {
    let mut h = opened(&[
        ("/w/a.txt", &numbered(20)),
        ("/w/b.txt", &numbered(20)),
        ("/w/c.txt", &numbered(20)),
    ]);
    // c → a (5 行目) → b (6 行目) → c (7 行目)
    h.cmd("buffer.next");
    assert_eq!(place(&h).0, "a.txt");
    goto(&mut h, 5);
    h.cmd("buffer.next");
    goto(&mut h, 6);
    h.cmd("buffer.next");
    goto(&mut h, 7);
    h.cmd("pane.split_right");
    h.cmd("pane.focus_next");
    // 元のペインで 2 回戻る (b の 6 行目の項目へ)
    assert!(back(&mut h));
    assert_eq!(place(&h), ("c.txt".to_owned(), 1));
    assert!(back(&mut h));
    assert_eq!(place(&h), ("b.txt".to_owned(), 6));
    // a へ切り替えて b を閉じる (切り替えの記録で先を捨てる)
    h.cmd("buffer.prev");
    assert_eq!(place(&h).0, "a.txt");
    // 進む先は無い
    assert!(!forward(&mut h));
    close_from_list(&mut h, "b.txt");
    assert_eq!(h.editor.state().documents_len(), 2);
    check(&h);
    let mut seen = Vec::new();
    while back(&mut h) {
        seen.push(place(&h).0);
    }
    assert!(seen.iter().all(|n| n != "b.txt"), "{seen:?}");
    h.cmd("pane.focus_next");
    let mut seen = Vec::new();
    while back(&mut h) {
        seen.push(place(&h).0);
    }
    assert!(
        seen.iter().all(|n| n != "b.txt"),
        "ほかのペインからも消える: {seen:?}"
    );
    check(&h);
}

// ---------- JMP-6 / JMP-7 ----------

#[test]
fn jmp6_jmp7_文書と_undo_を変えず_読み取り専用でも使え_ディスクに書かない() {
    let mut h = opened(&[("/w/a.txt", &numbered(50))]);
    let files = h.fs.paths();
    goto(&mut h, 30);
    goto(&mut h, 40);
    assert!(back(&mut h));
    assert!(back(&mut h));
    assert!(forward(&mut h));
    assert_eq!(h.fs.paths(), files, "履歴をディスクに書かない");
    assert_eq!(h.doc_text(), numbered(50), "文書を変えない");
    // Undo は打った文字を戻す (戻る・進むは Undo の履歴に残らない)
    h.text("zz");
    let text = h.doc_text();
    goto(&mut h, 10);
    assert!(back(&mut h));
    assert!(forward(&mut h));
    assert_eq!(h.doc_text(), text);
    h.cmd("edit.undo");
    assert_eq!(h.doc_text(), numbered(50));
    assert!(!h.is_dirty());
    // 読み取り専用の文書でも使える
    assert_eq!(h.cmd("file.toggle_read_only"), CommandResult::Done);
    assert!(back(&mut h));
    assert!(forward(&mut h));
}

/// 記録しない移動で、`numbered` の本文の `n` 行目の行頭へカーソルを置く。
fn put_at_line(h: &mut Harness, n: usize) {
    h.set_selection(Selection::point(ByteOff((n - 1) * 5)));
}

fn places_forward(h: &mut Harness) -> Vec<(String, usize)> {
    let mut seen = Vec::new();
    while forward(h) {
        seen.push(place(h));
    }
    seen
}

#[test]
fn jmp5_戻った先の文書を閉じても_代わりに出した文書の位置から進める() {
    let mut h = opened(&[
        ("/w/a.txt", &numbered(40)),
        ("/w/b.txt", &numbered(40)),
        ("/w/c.txt", &numbered(40)),
    ]);
    // A10・B20・C30 (切り替えの前の位置だけを記録する)
    h.cmd("buffer.next");
    assert_eq!(place(&h).0, "a.txt");
    put_at_line(&mut h, 10);
    h.cmd("buffer.next");
    put_at_line(&mut h, 20);
    h.cmd("buffer.next");
    put_at_line(&mut h, 30);
    assert_eq!(place(&h), ("c.txt".to_owned(), 30));
    // B20 → A10 と戻り、B20 へ進む
    assert!(back(&mut h));
    assert_eq!(place(&h), ("b.txt".to_owned(), 20));
    assert!(back(&mut h));
    assert_eq!(place(&h), ("a.txt".to_owned(), 10));
    assert!(forward(&mut h));
    assert_eq!(place(&h), ("b.txt".to_owned(), 20));
    // B を閉じる: ペインには A が出て、B20 の場所は A の今の位置になる
    assert_eq!(h.cmd("buffer.close"), CommandResult::Done);
    assert_eq!(place(&h), ("a.txt".to_owned(), 10));
    check(&h);
    let want: Vec<(String, usize)> = vec![("c.txt".to_owned(), 30)];
    assert_eq!(places_forward(&mut h), want, "閉じた後も C30 へ進める");
    let mut seen = Vec::new();
    while back(&mut h) {
        seen.push(place(&h));
    }
    assert!(seen.iter().all(|(n, _)| n != "b.txt"), "{seen:?}");
}

#[test]
fn jmp1_先行表示の文書でも記録し_読込の取消しで戻す切り替えは記録しない() {
    let mut h = Harness::new().with_file(
        "/w/a.txt",
        &(0..200)
            .flat_map(|i| format!("line {i:06}\n").into_bytes())
            .collect::<Vec<u8>>(),
    );
    h.paste(&numbered(20));
    put_at_line(&mut h, 5);
    h.fs.set_chunk_size(64);
    h.fs.set_read_delay(Some((h.clock.clone(), eve_core::Millis(100))));
    h.open("/w/a.txt");
    assert!(h.run_jobs_until_partial(), "先頭が届く");
    assert_eq!(place(&h), ("a.txt".to_owned(), 1));
    h.keys("down down");
    // 先行表示の文書から切り替える (記録する)
    h.cmd("buffer.prev");
    assert_eq!(place(&h), ("-".to_owned(), 5));
    goto(&mut h, 12);
    // 無題 12 行目 → (戻る) 無題 5 行目 → (戻る) a の 3 行目 (先行表示)
    assert!(back(&mut h));
    assert_eq!(place(&h), ("-".to_owned(), 5));
    assert!(back(&mut h));
    assert_eq!(place(&h), ("a.txt".to_owned(), 3));
    // 読込を取り消す: a を閉じて無題へ戻す切り替えは記録しない (進む側の項目を捨てない)
    h.keys("esc");
    h.run_jobs();
    assert_eq!(h.editor.state().documents_len(), 1);
    assert_eq!(place(&h).0, "-");
    check(&h);
    let fwd = places_forward(&mut h);
    assert_eq!(fwd.last(), Some(&("-".to_owned(), 12)), "{fwd:?}");
}

#[test]
fn jmp1_終了の連鎖で未保存の文書を出す切り替えは記録する() {
    let mut h = opened(&[("/w/a.txt", &numbered(20)), ("/w/b.txt", &numbered(20))]);
    h.cmd("buffer.prev");
    h.text("x");
    h.cmd("buffer.next");
    assert_eq!(place(&h).0, "b.txt");
    goto(&mut h, 7);
    h.cmd("app.quit");
    assert_eq!(h.overlay(), "dialog");
    assert_eq!(place(&h).0, "a.txt", "未保存の a を出す");
    h.keys("esc");
    assert_eq!(h.overlay(), "");
    assert!(back(&mut h));
    assert_eq!(place(&h), ("b.txt".to_owned(), 7));
}

#[test]
fn jmp1_設定の診断の一覧からの移動は前の位置を記録する() {
    const CONFIG: &str = "/evo/config/config.toml";
    let mut h = Harness::new();
    h.fs.add_file(CONFIG, b"[editor]\n\n\ntab_width = \"4\"\n");
    h.cmd("config.reload");
    h.run_jobs();
    h.open(CONFIG);
    h.run_jobs();
    // 今の文書の中の行の移動
    h.cmd("config.show_diagnostics");
    h.keys("enter");
    h.run_jobs();
    assert_eq!(place(&h), ("config.toml".to_owned(), 4));
    assert!(back(&mut h));
    assert_eq!(place(&h), ("config.toml".to_owned(), 1));
    // 別の文書から
    h.cmd("file.new");
    h.cmd("config.show_diagnostics");
    h.keys("enter");
    h.run_jobs();
    assert_eq!(place(&h), ("config.toml".to_owned(), 4));
    assert!(back(&mut h));
    assert_eq!(place(&h).0, "-");
}

#[test]
fn jmp1_読込の完了でフォーカスの無いペインを切り替えるとそのペインの履歴に入る() {
    let mut h = Harness::new().with_file("/w/a.txt", numbered(20).as_bytes());
    h.paste(&numbered(30));
    put_at_line(&mut h, 1);
    goto(&mut h, 5);
    assert_eq!(h.cmd("pane.split_right"), CommandResult::Done);
    goto(&mut h, 8);
    // 新しいペインで開き、読込の前にフォーカスを元のペインへ移す
    h.open("/w/a.txt");
    h.cmd("pane.focus_next");
    h.run_jobs();
    assert_eq!(place(&h), ("-".to_owned(), 5), "元のペインは替わらない");
    assert_eq!(back_lines(&mut h), [1], "元のペインの履歴には入らない");
    h.cmd("pane.focus_next");
    assert_eq!(place(&h), ("a.txt".to_owned(), 1));
    assert!(back(&mut h));
    assert_eq!(place(&h), ("-".to_owned(), 8), "開いたペインの履歴");
}

#[test]
fn jmp1_起動と最後の文書を閉じた後の無題の文書は記録しない() {
    let mut h = Harness::builder()
        .defer_start()
        .file("/w/a.txt", numbered(20).as_bytes())
        .build();
    h.editor
        .open_initial_files(&[InitialFile::path("/w/a.txt")], false, None);
    h.started();
    h.run_jobs();
    assert_eq!(place(&h).0, "a.txt");
    assert!(!back(&mut h), "起動で開いた文書の表示は記録しない");
    goto(&mut h, 5);
    h.cmd("buffer.close");
    assert_eq!(place(&h).0, "-");
    check(&h);
    assert!(
        !back(&mut h),
        "閉じた文書の項目は消え、無題の文書の表示は記録しない"
    );
    assert!(!forward(&mut h));
}

#[test]
fn jmp1_複数カーソルの対応括弧で主が動かなければ記録しない() {
    let mut h = scene("(a)\nbc\n");
    // 主は 2 行目 (括弧が無い)、ほかの Range は `(` の前
    h.set_selection(
        Selection::new(vec![Range::point(ByteOff(0)), Range::point(ByteOff(5))], 1)
            .expect("Selection"),
    );
    h.keys("ctrl+]");
    let st = h.editor.state();
    let v = st
        .view(st.window_view(h.window()).expect("View"))
        .expect("View");
    assert_ne!(
        v.selection.ranges()[0].head,
        ByteOff(0),
        "ほかの Range は動く"
    );
    assert_eq!(v.selection.primary().head, ByteOff(5));
    assert!(!back(&mut h));
}
