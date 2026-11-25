//! 検索履歴 (FR-407、docs/03 §12.8 の SHI-1〜SHI-6、07 §11.6 の「編集の拡張」、01 §5.5 の FR-407 AC-1) のシナリオ
//! (03 §15.9.1 の L3。M34 の段 9)。

use eve_core::{CommandResult, InitialFile};
use eve_testkit::Harness;
use eve_testkit::helpers::cell_text;
use eve_text::{ByteOff, Selection};
use eve_view::{BarFocus, OverlayModel, Span};

/// 無題の文書に `text` を入れ、カーソルを先頭に置いた Harness (ログは空)。
fn scene(text: &str) -> Harness {
    let mut h = Harness::new().with_size(60, 12);
    h.paste(text);
    h.set_selection(Selection::point(ByteOff(0)));
    h.clear_log();
    h
}

/// `/w/proj` (`.git` のあるリポジトリ) に `files` を置き、`/w/proj/README.md` を開いて起動する。
fn repo(files: &[(&str, &[u8])]) -> Harness {
    let mut b = Harness::builder()
        .defer_start()
        .dir("/w/proj/.git")
        .size(60, 12);
    b = b.file("/w/proj/README.md", b"readme\n");
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

/// 入力欄のセルの文字列。
fn spans(cells: &[Span]) -> String {
    cells.iter().map(|s| s.text.as_str()).collect()
}

/// 検索バーの検索欄の文字列。
fn field(h: &mut Harness) -> String {
    let bar = h.frame().search.expect("検索バーが開いている");
    spans(&bar.input.cells)
}

/// 検索バーの置換欄の文字列。
fn replace_field(h: &mut Harness) -> String {
    let bar = h.frame().search.expect("検索バーが開いている");
    spans(&bar.replace.expect("置換欄がある").input.cells)
}

/// 検索バーのフォーカスのある欄のカーソルの位置 (書記素の番号) と、欄内全選択があるか。
fn bar_cursor(h: &mut Harness) -> (usize, bool) {
    let bar = h.frame().search.expect("検索バーが開いている");
    let input = match bar.focus {
        BarFocus::Replace => bar.replace.expect("置換欄").input,
        _ => bar.input,
    };
    (input.cursor, input.selection.is_some())
}

/// 検索バーの件数の欄 (逐次検索の結果)。
fn count(h: &mut Harness) -> String {
    h.frame()
        .search
        .and_then(|b| b.count_text)
        .map(|c| cell_text(&c))
        .unwrap_or_default()
}

/// 横断検索のオーバーレイの入力欄の文字列。
fn ws_field(h: &mut Harness) -> String {
    let f = h.frame();
    let Some(OverlayModel::List { input, .. }) = f.overlays.first() else {
        panic!("一覧のオーバーレイが無い: {:?}", f.overlays);
    };
    spans(&input.cells)
}

/// 横断検索の結果の行 (`相対パス:行`)。
fn ws_rows(h: &Harness) -> Vec<String> {
    h.editor
        .state()
        .ws_search_rows()
        .expect("横断検索が開いている")
        .1
}

fn terms(h: &Harness) -> Vec<String> {
    h.editor.state().search_term_history()
}

fn replacements(h: &Harness) -> Vec<String> {
    h.editor.state().replace_history()
}

/// フォーカスのある欄を空にする (`select.all` → `Backspace`)。
fn clear(h: &mut Harness) {
    h.keys("ctrl+a backspace");
}

fn prev(h: &mut Harness) {
    h.keys("alt+up");
}

fn next(h: &mut Harness) {
    h.keys("alt+down");
}

/// 検索バーで `words` を順に打ち、それぞれ `Enter` で次の一致へ移る (検索語の履歴に記録する)。
fn search_words(h: &mut Harness, words: &[&str]) {
    h.keys("ctrl+f");
    for w in words {
        clear(h);
        h.text(w);
        h.run_jobs();
        h.keys("enter");
        h.run_jobs();
    }
    h.keys("esc");
}

// ---------- AC-1 ----------

// verifies: AC-FR-407-1
#[test]
fn 検索バーで実行した語が記録され_alt_上下で古い語と新しい語と下書きを呼び出せる() {
    let mut h = scene("foo bar baz foo\n");
    h.keys("ctrl+f");
    h.text("foo");
    h.keys("enter");
    clear(&mut h);
    h.text("bar");
    h.keys("down");
    clear(&mut h);
    h.text("baz");
    h.keys("up");
    h.keys("esc");
    assert_eq!(terms(&h), ["foo", "bar", "baz"]);
    h.clear_log();
    // 打ちかけの語を下書きとして覚え、古い側へたどる
    h.keys("ctrl+f");
    h.text("dr");
    prev(&mut h);
    assert_eq!(field(&mut h), "baz");
    prev(&mut h);
    assert_eq!(field(&mut h), "bar");
    prev(&mut h);
    assert_eq!(field(&mut h), "foo");
    prev(&mut h);
    assert_eq!(field(&mut h), "foo", "最も古い項目の先は何もしない");
    next(&mut h);
    assert_eq!(field(&mut h), "bar");
    next(&mut h);
    assert_eq!(field(&mut h), "baz");
    next(&mut h);
    assert_eq!(field(&mut h), "dr", "最新より新しい側は下書き");
    next(&mut h);
    assert_eq!(field(&mut h), "dr", "下書きの先は何もしない");
    assert_eq!(h.messages(), Vec::<String>::new(), "端でも知らせない");
}

// verifies: AC-FR-407-1
#[test]
fn 代替の_ctrl_k_上下でも呼び出せる() {
    let mut h = scene("foo bar\n");
    search_words(&mut h, &["foo", "bar"]);
    h.keys("ctrl+f");
    h.keys("ctrl+k up");
    assert_eq!(field(&mut h), "bar");
    h.keys("ctrl+k up");
    assert_eq!(field(&mut h), "foo");
    h.keys("ctrl+k down");
    assert_eq!(field(&mut h), "bar");
    h.keys("ctrl+k down");
    assert_eq!(field(&mut h), "");
}

// verifies: AC-FR-407-1
#[test]
fn 置換欄では置換の文字列の履歴を使い_検索語の履歴と分ける() {
    let mut h = scene("foo foo foo foo\n");
    h.keys("ctrl+r");
    h.text("foo");
    h.keys("tab");
    h.text("X");
    // 置換欄の Enter は 1 件置換
    h.keys("enter");
    h.run_jobs();
    clear(&mut h);
    h.text("Y");
    h.keys("alt+a");
    h.run_jobs();
    assert_eq!(h.doc_text(), "X Y Y Y\n");
    assert_eq!(terms(&h), ["foo"]);
    assert_eq!(replacements(&h), ["X", "Y"]);
    h.keys("esc");
    // 開き直すと位置は最新から。置換欄は置換の文字列、検索欄は検索語
    h.keys("ctrl+r");
    assert_eq!(
        bar_focus(&mut h),
        BarFocus::Search,
        "検索語が空なら検索欄から"
    );
    prev(&mut h);
    assert_eq!(field(&mut h), "foo");
    h.keys("tab");
    prev(&mut h);
    assert_eq!(replace_field(&mut h), "Y");
    prev(&mut h);
    assert_eq!(replace_field(&mut h), "X");
    prev(&mut h);
    assert_eq!(replace_field(&mut h), "X", "端");
    next(&mut h);
    next(&mut h);
    assert_eq!(replace_field(&mut h), "", "下書き (空の置換欄)");
    assert_eq!(field(&mut h), "foo", "置換欄の呼び出しは検索欄を変えない");
}

fn bar_focus(h: &mut Harness) -> BarFocus {
    h.frame().search.expect("検索バーが開いている").focus
}

// verifies: AC-FR-407-1
#[test]
fn 横断検索で実行した語を検索バーと共有し_横断検索の入力欄でも呼び出せる() {
    let mut h = repo(&[
        ("/w/proj/a.txt", b"alpha hit\nfoo\n"),
        ("/w/proj/b.txt", b"beta foo\n"),
    ]);
    // 横断検索で打った語は、閉じるときに 1 つだけ記録する (打っている途中の語は記録しない)
    h.keys("ctrl+shift+f");
    h.text("h");
    h.run_jobs();
    h.text("it");
    h.run_jobs();
    h.keys("esc");
    assert_eq!(terms(&h), ["hit"]);
    // 検索バーで呼び出せる (共有)
    h.keys("ctrl+f");
    prev(&mut h);
    assert_eq!(field(&mut h), "hit");
    clear(&mut h);
    h.text("foo");
    h.keys("enter");
    h.keys("esc");
    assert_eq!(terms(&h), ["hit", "foo"]);
    // 横断検索の入力欄 (前回の検索語 "hit" で開く) でも呼び出し、結果を探し直す
    h.keys("ctrl+shift+f");
    h.run_jobs();
    assert_eq!(ws_field(&mut h), "hit");
    prev(&mut h);
    assert_eq!(ws_field(&mut h), "foo");
    h.run_jobs();
    assert_eq!(ws_rows(&h), ["a.txt:2", "b.txt:1"]);
    prev(&mut h);
    assert_eq!(ws_field(&mut h), "hit", "欄 (foo) と違う項目なので出す");
    next(&mut h);
    next(&mut h);
    assert_eq!(ws_field(&mut h), "hit", "下書きに戻る");
    h.run_jobs();
    assert_eq!(ws_rows(&h), ["a.txt:1"]);
    // 呼び出した語の結果へのジャンプでも記録する (最新へ移す)
    prev(&mut h);
    prev(&mut h);
    assert_eq!(ws_field(&mut h), "hit");
    h.run_jobs();
    h.keys("enter");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_eq!(terms(&h), ["foo", "hit"]);
}

// ---------- SHI-1: 履歴の種類 ----------

// spec: 03 §12.8 (SHI-1)
#[test]
fn shi1_履歴は文書とペインに依らずプロセスに_1_つ() {
    let mut h = scene("foo\n");
    search_words(&mut h, &["foo"]);
    // 別の文書・分割したペインでも同じ履歴
    h.cmd("file.new");
    h.cmd("pane.split_right");
    h.keys("ctrl+f");
    prev(&mut h);
    assert_eq!(field(&mut h), "foo");
}

// spec: 03 §12.8 (SHI-1, SHI-6)
#[test]
fn shi1_shi6_ほかの入力欄では効かず_キーは下の層へ進む() {
    let mut h = scene("a\nb\nc\n");
    search_words(&mut h, &["a"]);
    // 行へ移動のプロンプト: 欄は変わらない
    h.keys("ctrl+g");
    assert_eq!(h.overlay(), "prompt");
    h.text("2");
    prev(&mut h);
    h.keys("ctrl+k up");
    let f = h.frame();
    let text = match f.overlays.first() {
        Some(OverlayModel::Prompt { input, .. }) => spans(&input.cells),
        other => panic!("プロンプト: {other:?}"),
    };
    assert_eq!(text, "2");
    h.keys("esc");
    // パレット: 絞り込みの欄は変わらない
    h.keys("ctrl+p");
    h.text("save");
    prev(&mut h);
    let f = h.frame();
    let text = match f.overlays.first() {
        Some(OverlayModel::List { input, .. }) => spans(&input.cells),
        other => panic!("パレット: {other:?}"),
    };
    assert_eq!(text, "save");
    h.keys("esc");
    // 編集面では従来どおり行の上下移動 (editor の割当)
    h.keys("ctrl+end up");
    prev(&mut h);
    assert_eq!(h.doc_text(), "a\nc\nb\n");
    h.keys("ctrl+k up");
    assert_eq!(
        h.doc_text(),
        "c\na\nb\n",
        "編集面の ctrl+k up は行の移動のまま"
    );
    // パレットに出さない
    assert!(eve_core::command_hidden("ui.history_prev"));
    assert!(eve_core::command_hidden("ui.history_next"));
    // 検索バーが無いときにコマンドを直に実行しても何も変えない
    assert_eq!(
        h.cmd("ui.history_prev"),
        CommandResult::Rejected(eve_core::RejectReason::NotAvailable)
    );
}

// ---------- SHI-2: 記録する時期 ----------

// spec: 03 §12.8 (SHI-2)
#[test]
fn shi2_次_前_置換_すべて置換で記録し_一致の有無に依らない() {
    let mut h = scene("foo foo\n");
    h.keys("ctrl+f");
    h.text("nomatch");
    h.keys("enter");
    clear(&mut h);
    h.text("foo");
    h.keys("up");
    h.keys("esc");
    assert_eq!(terms(&h), ["nomatch", "foo"]);
    // 置換は検索語と置換の文字列を記録する (検索欄からの alt+a も)
    h.keys("ctrl+r");
    clear(&mut h);
    h.text("o");
    h.keys("tab");
    h.text("0");
    h.keys("alt+a");
    h.run_jobs();
    assert_eq!(h.doc_text(), "f00 f00\n");
    assert_eq!(terms(&h), ["nomatch", "foo", "o"]);
    assert_eq!(replacements(&h), ["0"]);
    // ボタンの Enter (すべて置換) でも記録する: 置換の文字列と検索語を替えてから押す
    clear(&mut h);
    h.text("1");
    h.keys("shift+tab");
    clear(&mut h);
    h.text("f");
    h.keys("tab tab");
    assert_eq!(bar_focus(&mut h), BarFocus::Button);
    h.keys("enter");
    h.run_jobs();
    assert_eq!(h.doc_text(), "100 100\n");
    assert_eq!(terms(&h), ["nomatch", "foo", "o", "f"]);
    assert_eq!(replacements(&h), ["0", "1"]);
}

// spec: 03 §12.8 (SHI-2)、03 §12.8.1 (読み取り専用で断る置換)
#[test]
fn shi2_読み取り専用の文書で断る置換は経路に依らず記録しない() {
    let mut h = scene("foo foo\n");
    h.keys("ctrl+r");
    h.text("foo");
    h.keys("tab");
    h.text("bar");
    assert_eq!(h.cmd("file.toggle_read_only"), CommandResult::Done);
    // 置換欄の Enter (search.replace) と alt+a (search.replace_all) は `when` で断る
    h.keys("enter");
    h.keys("alt+a");
    // ボタンの Enter も search.replace_all として実行し、`when` で断る (04 §8.4、Issue #655)
    h.keys("tab");
    assert_eq!(bar_focus(&mut h), BarFocus::Button);
    h.keys("enter");
    h.run_jobs();
    assert_eq!(h.doc_text(), "foo foo\n");
    assert_eq!(terms(&h), Vec::<String>::new());
    assert_eq!(replacements(&h), Vec::<String>::new());
    // 断ったボタンの Enter は search.next に落ちない (次 / 前として検索語を記録しない)
    h.clear_log();
    h.keys("enter");
    assert_eq!(terms(&h), Vec::<String>::new());
    assert_eq!(h.messages(), ["msg.edit.read_only"]);
}

// spec: 03 §12.8 (SHI-2)、04 §8.4 (ボタンにフォーカスがあっても `search.next` は次へ移るだけ)
#[test]
fn shi2_ボタンにフォーカスがあるときの_search_next_は検索語だけを記録する() {
    let mut h = scene("foo foo\n");
    h.keys("ctrl+r");
    h.text("foo");
    h.keys("tab");
    h.text("bar");
    h.keys("tab");
    assert_eq!(bar_focus(&mut h), BarFocus::Button);
    assert_eq!(h.cmd("search.next"), CommandResult::Done);
    h.run_jobs();
    assert_eq!(h.doc_text(), "foo foo\n");
    assert_eq!(terms(&h), ["foo"]);
    assert_eq!(
        replacements(&h),
        Vec::<String>::new(),
        "置換しないので置換の文字列は記録しない"
    );
}

// spec: 03 §12.8 (SHI-2)
#[test]
fn shi2_打っただけの語_閉じた語_空の語は記録しない() {
    let mut h = scene("foo\n");
    // 逐次検索の途中の語と、Esc で閉じた語は記録しない
    h.keys("ctrl+f");
    h.text("f");
    h.run_jobs();
    h.text("oo");
    h.run_jobs();
    h.keys("esc");
    assert_eq!(terms(&h), Vec::<String>::new());
    // 空の検索語で次へ (何もしない) も記録しない
    h.keys("ctrl+f enter");
    h.keys("esc");
    assert_eq!(terms(&h), Vec::<String>::new());
    // 置換の文字列が空 (削除の置換) なら置換の履歴に入れない
    h.keys("ctrl+r");
    h.text("o");
    h.keys("tab");
    h.keys("alt+a");
    h.run_jobs();
    assert_eq!(h.doc_text(), "f\n");
    assert_eq!(terms(&h), ["o"]);
    assert_eq!(replacements(&h), Vec::<String>::new());
}

// spec: 03 §12.8 (SHI-2)
#[test]
fn shi2_横断検索は走査を始めた語だけを閉じるときに記録する() {
    let mut h = repo(&[("/w/proj/a.txt", b"hit\n")]);
    // 空のまま閉じる・正規表現の誤りで探さなかった語は記録しない
    h.keys("ctrl+shift+f esc");
    h.keys("ctrl+shift+f alt+r");
    h.text("(");
    h.run_jobs();
    h.keys("esc");
    assert_eq!(terms(&h), Vec::<String>::new());
    // 走査を始めた語は記録する
    h.keys("ctrl+shift+f");
    clear(&mut h);
    h.text("hit");
    h.run_jobs();
    h.keys("esc");
    assert_eq!(terms(&h), ["hit"]);
}

// spec: 03 §12.8 (SHI-2)、03 §12.8.1 (横断検索で記録する時期)
#[test]
fn shi2_横断検索は呼び出しで眺めた語と出し直しただけの語を記録せず_ジャンプと編集とトグルは記録する()
 {
    let mut h = repo(&[("/w/proj/a.txt", b"hit\nfoo\n")]);
    search_words(&mut h, &["hit", "foo"]);
    // 呼び出しで眺めて閉じる: 記録しない (hit を最新へ移さない)
    h.keys("ctrl+shift+f");
    assert_eq!(ws_field(&mut h), "");
    prev(&mut h);
    h.run_jobs();
    prev(&mut h);
    h.run_jobs();
    assert_eq!(ws_field(&mut h), "hit");
    assert_eq!(ws_rows(&h), ["a.txt:1"]);
    h.keys("esc");
    assert_eq!(terms(&h), ["hit", "foo"]);
    // 前回の結果を出し直しただけで閉じる: 記録しない
    h.keys("ctrl+shift+f");
    h.run_jobs();
    assert_eq!(ws_field(&mut h), "hit");
    h.keys("esc");
    assert_eq!(terms(&h), ["hit", "foo"]);
    // 出し直した結果へのジャンプは記録する
    h.keys("ctrl+shift+f");
    h.run_jobs();
    h.keys("enter");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_eq!(terms(&h), ["foo", "hit"]);
    // 呼び出した後に編集して閉じると、編集した語を記録する
    h.keys("ctrl+shift+f");
    prev(&mut h);
    prev(&mut h);
    assert_eq!(ws_field(&mut h), "foo");
    h.keys("backspace");
    h.run_jobs();
    h.keys("esc");
    assert_eq!(terms(&h), ["foo", "hit", "fo"]);
    // トグルで探し直して閉じても記録する
    h.keys("ctrl+shift+f");
    prev(&mut h);
    prev(&mut h);
    assert_eq!(ws_field(&mut h), "hit");
    h.keys("alt+c");
    h.run_jobs();
    h.keys("esc");
    assert_eq!(terms(&h), ["foo", "fo", "hit"]);
}

// spec: 03 §12.8 (SHI-2)
#[test]
fn shi2_トグルは記録せず_同じ語は大小を区別して最新へ移す() {
    let mut h = scene("Foo foo\n");
    h.keys("ctrl+f");
    h.text("foo");
    h.keys("alt+c enter");
    h.keys("alt+r enter");
    h.keys("esc");
    assert_eq!(terms(&h), ["foo"], "トグルを変えても同じ語は 1 つ");
    search_words(&mut h, &["Foo", "bar", "foo"]);
    assert_eq!(terms(&h), ["Foo", "bar", "foo"]);
    // 呼び出しても記録しない (確定したときだけ)
    h.keys("ctrl+f");
    prev(&mut h);
    prev(&mut h);
    assert_eq!(field(&mut h), "bar");
    h.keys("esc");
    assert_eq!(terms(&h), ["Foo", "bar", "foo"]);
}

// ---------- SHI-3: 上限 ----------

// spec: 03 §12.8 (SHI-3, SHI-2)
#[test]
fn shi3_上限の_100_件で古いものから捨て_4_kib_を超える語は記録しない() {
    let mut h = scene("x\n");
    let words: Vec<String> = (0..=100).map(|i| format!("w{i}")).collect();
    let refs: Vec<&str> = words.iter().map(String::as_str).collect();
    search_words(&mut h, &refs);
    let t = terms(&h);
    assert_eq!(t.len(), 100);
    assert_eq!(t.first().map(String::as_str), Some("w1"));
    assert_eq!(t.last().map(String::as_str), Some("w100"));
    // 4 KiB ちょうどは記録し、超える語は記録しない
    let ok = "a".repeat(4096);
    let over = "b".repeat(4097);
    search_words(&mut h, &[&ok, &over]);
    let t = terms(&h);
    assert_eq!(t.last(), Some(&ok));
    assert!(!t.contains(&over));
    // 多バイトの語もバイトで数える ("あ" は 3 バイト。1365 個 = 4095 バイトは記録し、1366 個 = 4098 バイトは記録しない)
    let ja_ok = "あ".repeat(1365);
    let ja_over = "あ".repeat(1366);
    search_words(&mut h, &[&ja_ok, &ja_over]);
    let t = terms(&h);
    assert_eq!(t.last(), Some(&ja_ok));
    assert!(!t.contains(&ja_over));
    // 置換の文字列も同じ上限
    h.keys("ctrl+r");
    h.text("x");
    for i in 0..=100 {
        h.keys("tab");
        clear(&mut h);
        h.text(&format!("r{i}"));
        h.keys("alt+a");
        h.run_jobs();
        h.keys("shift+tab");
    }
    let r = replacements(&h);
    assert_eq!(r.len(), 100);
    assert_eq!(r.first().map(String::as_str), Some("r1"));
}

// ---------- SHI-4: 呼び出し ----------

// spec: 03 §12.8 (SHI-4)
#[test]
fn shi4_欄の全体を置き換え_カーソルを末尾へ置き_欄内全選択を解く() {
    let mut h = scene("abc\n");
    search_words(&mut h, &["abc"]);
    h.keys("ctrl+f");
    h.text("zz");
    h.keys("left left");
    h.keys("ctrl+a");
    assert!(bar_cursor(&mut h).1);
    prev(&mut h);
    assert_eq!(field(&mut h), "abc");
    assert_eq!(bar_cursor(&mut h), (3, false));
    // 下書きに戻すときもカーソルは末尾
    next(&mut h);
    assert_eq!(field(&mut h), "zz");
    assert_eq!(bar_cursor(&mut h), (2, false));
}

// spec: 03 §12.8 (SHI-4)
#[test]
fn shi4_呼び出した語は打ったときと同じく逐次検索に効く() {
    let mut h = scene("foo bar foo\n");
    // 打ったときの件数
    h.keys("ctrl+f");
    h.text("foo");
    h.run_jobs();
    let typed = count(&mut h);
    assert_ne!(typed, "");
    h.keys("enter esc");
    h.set_selection(Selection::point(ByteOff(0)));
    h.keys("ctrl+f");
    prev(&mut h);
    h.run_jobs();
    assert_eq!(count(&mut h), typed);
    // 呼び出した語で Enter すると一致へ移る
    h.keys("enter");
    assert_eq!(h.selection_range(), (0, 3));
}

// spec: 03 §12.8 (SHI-4)
#[test]
fn shi4_呼び出した後に編集して確定すると編集した語を記録し_編集した語が次の下書きになる() {
    let mut h = scene("foo foox\n");
    search_words(&mut h, &["bar", "foo"]);
    h.keys("ctrl+f");
    prev(&mut h);
    assert_eq!(field(&mut h), "foo");
    h.text("x");
    // 編集すると呼び出しを止め、次の前は編集した語を下書きにして最新から
    prev(&mut h);
    assert_eq!(field(&mut h), "foo");
    next(&mut h);
    assert_eq!(field(&mut h), "foox", "編集した語が下書き");
    h.keys("enter esc");
    assert_eq!(
        terms(&h),
        ["bar", "foo", "foox"],
        "呼び出した項目は変えず、編集した語を足す"
    );
}

// spec: 03 §12.8 (SHI-4)
#[test]
fn shi4_開き直すと最新から数え直し_下書きを捨てる() {
    let mut h = scene("a b\n");
    search_words(&mut h, &["a", "b"]);
    h.keys("ctrl+f");
    h.text("draft");
    prev(&mut h);
    prev(&mut h);
    assert_eq!(field(&mut h), "a");
    h.keys("esc ctrl+f");
    assert_eq!(field(&mut h), "");
    prev(&mut h);
    assert_eq!(field(&mut h), "b", "最新から");
    next(&mut h);
    assert_eq!(field(&mut h), "", "前の下書き (draft) は捨てた");
    // 確定した直後は、欄と同じ最新の項目を飛ばして 1 つ前を出す
    clear(&mut h);
    h.text("b");
    h.keys("enter");
    prev(&mut h);
    assert_eq!(field(&mut h), "a");
}

// spec: 03 §12.8 (SHI-4)
#[test]
fn shi4_空の履歴とボタンでは何もしない() {
    let mut h = scene("foo\n");
    h.keys("ctrl+f");
    h.text("q");
    prev(&mut h);
    next(&mut h);
    assert_eq!(field(&mut h), "q");
    h.keys("esc");
    search_words(&mut h, &["foo"]);
    h.keys("ctrl+r");
    h.keys("tab tab");
    assert_eq!(bar_focus(&mut h), BarFocus::Button);
    h.clear_log();
    assert_eq!(h.cmd("ui.history_prev"), CommandResult::Done);
    assert_eq!(field(&mut h), "");
    assert_eq!(replace_field(&mut h), "");
    assert_eq!(h.messages(), Vec::<String>::new());
}

// spec: 03 §12.8 (SHI-4)、03 §12.8.1 (欄ごとの位置)
#[test]
fn shi4_検索バーと横断検索の入力欄は呼び出しの位置を別に持つ() {
    let mut h = repo(&[("/w/proj/a.txt", b"a b c\n")]);
    search_words(&mut h, &["a", "b", "c"]);
    h.keys("ctrl+f");
    prev(&mut h);
    prev(&mut h);
    assert_eq!(field(&mut h), "b");
    // 検索バーから横断検索を開いて呼び出す (横断検索の欄は検索バーの語 "b" で始まり、最新から数える)
    h.keys("ctrl+shift+f");
    assert_eq!(ws_field(&mut h), "b");
    prev(&mut h);
    assert_eq!(ws_field(&mut h), "c");
    h.keys("esc");
    // 検索バーに戻ると、バーの位置の続きから (横断検索の呼び出しはバーの位置を動かさない)
    assert_eq!(h.overlay(), "");
    prev(&mut h);
    assert_eq!(field(&mut h), "a");
    // 横断検索で語を探して閉じると履歴が変わるので、バーは最新から数え直す
    h.keys("ctrl+shift+f");
    assert_eq!(ws_field(&mut h), "a");
    h.text("x");
    h.run_jobs();
    h.keys("esc");
    assert_eq!(terms(&h), ["a", "b", "c", "ax"]);
    prev(&mut h);
    assert_eq!(field(&mut h), "ax");
}

// spec: 03 §12.8 (SHI-6)
#[test]
fn shi6_ファインダでは効かず_横断検索ではチョードの代替でも効く() {
    let mut h = repo(&[("/w/proj/a.txt", b"a\n")]);
    search_words(&mut h, &["readme"]);
    h.keys("ctrl+o");
    h.run_jobs();
    assert_eq!(h.overlay(), "finder");
    h.text("rea");
    prev(&mut h);
    h.keys("ctrl+k up");
    assert_eq!(ws_field(&mut h), "rea");
    h.keys("esc");
    // 選択を解いてから開く (選択が初期値にならないように)
    h.keys("ctrl+home");
    h.keys("ctrl+shift+f");
    assert_eq!(h.overlay(), "workspace_search");
    assert_eq!(ws_field(&mut h), "");
    h.keys("ctrl+k up");
    assert_eq!(ws_field(&mut h), "readme");
    h.keys("ctrl+k down");
    assert_eq!(ws_field(&mut h), "");
}

// ---------- SHI-5: 保存しない ----------

// spec: 03 §12.8 (SHI-5)
#[test]
fn shi5_履歴をディスクに書かない() {
    const SECRET: &str = "s3cr3t-token-for-history";
    let mut h = Harness::new().with_file("/w/a.txt", b"hello\n");
    h.open("/w/a.txt");
    h.run_jobs();
    h.keys("ctrl+r");
    h.text(SECRET);
    h.keys("enter tab");
    h.text(SECRET);
    h.keys("alt+a");
    h.run_jobs();
    h.keys("esc");
    // 文書を変えてスワップを書かせる (最後の入力から 2 秒後)
    h.text("edit");
    h.advance_ms(0);
    h.advance_ms(2_000);
    h.run_jobs();
    assert_eq!(terms(&h), [SECRET]);
    assert_eq!(replacements(&h), [SECRET]);
    assert!(h.fs.swap_writes() >= 1, "スワップを書いた");
    let doc = h.doc_id();
    let swap = h.fs.swap_of(doc).expect("スワップ");
    assert!(!String::from_utf8_lossy(&swap).contains(SECRET));
    for p in h.fs.paths() {
        let data = h.fs.read(&p).unwrap_or_default();
        assert!(
            !String::from_utf8_lossy(&data).contains(SECRET),
            "{p} に検索語が書かれた"
        );
    }
    // 緊急退避の公開物にも入らない
    let snap = h.editor.emergency_snapshot(doc);
    assert!(!format!("{snap:?}").contains(SECRET));
}
