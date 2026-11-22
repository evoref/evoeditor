//! カーソルを足す操作・`Esc`・既定キー (FR-211、docs/03 §4.5.1・§4.5.5、07 §11.6 の「複数カーソルと対応括弧」、01 §5.5 の
//! FR-211 AC-1 / AC-3) のシナリオ (03 §4.7.1 の L3。M32 の段 3)。文書と Range の印は `marked` (`multi_cursor.rs` と共有)。

mod marked;

use eve_core::CtxValue;
use eve_testkit::Harness;
use eve_testkit::helpers::opened;
use eve_text::{ByteOff, MAX_CURSORS, Range, Selection};
use eve_view::Effect;
use marked::{scene, selection, set_selection, shown, view};

/// メッセージが出なかった。
const NO_MESSAGE: [&str; 0] = [];

/// メッセージの記録を消してから `keys` を押し、出たメッセージのキー。
fn press(h: &mut Harness, keys: &str) -> Vec<String> {
    h.clear_log();
    h.keys(keys);
    h.messages()
}

fn context(h: &Harness, name: &str) -> Option<CtxValue> {
    h.editor.state().context_value(name)
}

// ---------- select.add_next_match (MCUR-2) ----------

// verifies: AC-FR-211-1
#[test]
fn ctrl_d_はカーソルの単語を選び_次の一致を足して主にし_末尾から先頭へ回る() {
    let mut h = scene("foo bar fo*|o baz foo");
    assert_eq!(press(&mut h, "ctrl+d"), NO_MESSAGE);
    assert_eq!(
        shown(&h),
        "foo bar «foo» baz foo",
        "(a) 単語を選ぶ (Range は足さない)"
    );
    press(&mut h, "ctrl+d");
    assert_eq!(shown(&h), "foo bar «foo» baz «foo»");
    assert_eq!(
        selection(&h).primary_index(),
        1,
        "足した Range が主 (MCUR-1)"
    );
    press(&mut h, "ctrl+d");
    assert_eq!(
        shown(&h),
        "«foo» bar «foo» baz «foo»",
        "文書の末尾から先頭へ回る"
    );
    assert_eq!(selection(&h).primary_index(), 0);
    // 全部選んである: 何も足さずに知らせる
    let before = selection(&h);
    assert_eq!(press(&mut h, "ctrl+d"), ["info.select.no_more_matches"]);
    assert_eq!(selection(&h), before);
}

// verifies: AC-FR-211-1
#[test]
fn 単語から始めた一致は単語単位_選択から始めた一致は単語単位でない() {
    let mut h = scene("*|foo foobar foo");
    h.keys("ctrl+d ctrl+d");
    assert_eq!(
        shown(&h),
        "«foo» foobar «foo»",
        "単語の一致の印: foobar を飛ばす"
    );
    // 選択から始めると (印なし) 単語の中の一致も足す
    let mut h = scene("*«foo» foobar foo");
    h.keys("ctrl+d");
    assert_eq!(shown(&h), "«foo» «foo»bar foo");
}

// verifies: AC-FR-211-1
#[test]
fn 大小を区別し_検索バーのトグルに依らない() {
    let mut h = scene("Foo foo FOO f*|oo");
    h.keys("ctrl+d ctrl+d");
    assert_eq!(shown(&h), "Foo «foo» FOO «foo»");
    assert_eq!(press(&mut h, "ctrl+d"), ["info.select.no_more_matches"]);
}

// verifies: AC-FR-211-1
#[test]
fn 日本語の単語は文字種の境界で決まる() {
    // 03 §4.5.1 の MCUR-2 (d) の例: `設定` から足すと `設定ファイル` の `設定` に当たり、`再設定` の `設定` には当たらない
    let mut h = scene("設*|定 再設定 設定ファイル");
    h.keys("ctrl+d ctrl+d");
    assert_eq!(shown(&h), "«設定» 再設定 «設定»ファイル");
    assert_eq!(press(&mut h, "ctrl+d"), ["info.select.no_more_matches"]);
}

// verifies: AC-FR-211-1
#[test]
fn 主のカーソルに単語が無ければ何もせずに知らせる() {
    let mut h = scene("foo  *|  bar");
    assert_eq!(press(&mut h, "ctrl+d"), ["info.select.no_word"]);
    assert_eq!(shown(&h), "foo  |  bar");
}

// verifies: AC-FR-211-1
#[test]
fn 全部が空なら各カーソルの単語を選び_主が空でほかに選択があれば主だけ() {
    let mut h = scene("a|lpha be*|ta ga|mma");
    h.keys("ctrl+d");
    assert_eq!(shown(&h), "«alpha» «beta» «gamma»");
    assert_eq!(selection(&h).primary_index(), 1, "主は同じ Range");
    let mut h = scene("«alpha» be*|ta gamma");
    h.keys("ctrl+d");
    assert_eq!(
        shown(&h),
        "«alpha» «beta» gamma",
        "(c) 主のカーソルの単語だけ"
    );
}

// verifies: AC-FR-211-1
#[test]
fn 既にある選択と重なる_接する一致は飛ばす() {
    // `aaa` の `a`: 次の `a` は今の選択に接するので飛ばし (足すと併合されて検索語が変わる)、その次を足す
    let mut h = scene("*«a»aa");
    h.keys("ctrl+d");
    assert_eq!(shown(&h), "«a»a«a»");
    assert_eq!(press(&mut h, "ctrl+d"), ["info.select.no_more_matches"]);
}

// verifies: AC-FR-211-1
#[test]
fn 単語の一致の印は移動で消え_足す操作と_esc_では残る() {
    let mut h = scene("*|foo foobar foo");
    h.keys("ctrl+d");
    // 移動して同じ選択に戻しても、印は消えている (単語単位でなくなる)
    h.keys("shift+left shift+right");
    assert_eq!(shown(&h), "«foo» foobar foo");
    h.keys("ctrl+d");
    assert_eq!(shown(&h), "«foo» «foo»bar foo");
    // 足す操作と Esc では残る
    let mut h = scene("*|foo foobar foo foo");
    h.keys("ctrl+d ctrl+d esc");
    assert_eq!(shown(&h), "foo foobar «foo» foo", "Esc で主の Range だけ");
    h.keys("ctrl+d");
    assert_eq!(
        shown(&h),
        "foo foobar «foo» «foo»",
        "印が残り、単語単位のまま"
    );
}

// verifies: AC-FR-211-3
#[test]
fn 足す一致で上限を超えるなら選択を変えずに知らせる() {
    let text = format!("*|{}", "ab ".repeat(MAX_CURSORS + 1));
    let mut h = scene(&text);
    let ranges: Vec<Range> = (0..MAX_CURSORS)
        .map(|i| Range::new(ByteOff(i * 3), ByteOff(i * 3 + 2)))
        .collect();
    set_selection(&mut h, ranges, MAX_CURSORS - 1);
    let before = selection(&h);
    assert_eq!(press(&mut h, "ctrl+d"), ["info.cursor.limit"]);
    assert_eq!(selection(&h), before);
}

// verifies: AC-FR-211-1
#[test]
fn 同期検索で見つからない一致は単発のジョブで探して足す() {
    // 主の後ろ 600 KiB に一致が無い: 同期検索 (256 KiB) で決まらず、ジョブの結果で足す
    let mut body = String::from("foo\n");
    body.push_str(&format!("{}\n", "x".repeat(1023)).repeat(600));
    body.push_str("foo");
    let mut h = opened("/w/big.txt", body.as_bytes());
    let len = body.len();
    let v = view(&h);
    h.editor
        .state_mut()
        .set_selection(v, Selection::single(Range::new(ByteOff(0), ByteOff(3))));
    h.keys("ctrl+d");
    assert_eq!(selection(&h).len(), 1, "ジョブの結果を待つ");
    h.run_jobs();
    let s = selection(&h);
    assert_eq!(s.len(), 2, "{s:?}");
    assert_eq!(*s.primary(), Range::new(ByteOff(len - 3), ByteOff(len)));
    // 結果が届く前に選択を変えたら、結果は捨てる
    h.editor
        .state_mut()
        .set_selection(v, Selection::single(Range::new(ByteOff(0), ByteOff(3))));
    h.keys("ctrl+d");
    h.keys("right");
    h.run_jobs();
    assert_eq!(selection(&h).len(), 1, "投入時と選択が違うので足さない");
}

/// 1 KiB の行を `n` 行 (同期検索の予算を使い切る長さ)。
fn filler(n: usize) -> String {
    format!("{}\n", "x".repeat(1023)).repeat(n)
}

/// View `v` の選択。
fn selection_of(h: &Harness, v: eve_view::ViewId) -> Selection {
    h.editor
        .state()
        .view(v)
        .expect("View がある")
        .selection
        .clone()
}

// verifies: AC-FR-211-1
#[test]
fn 単発のジョブの結果は_revision_が変わっていたら捨てる() {
    // 右のペインで投げ、左のペインで後ろに書き足す (右のペインの選択は写像で変わらない。Revision だけが違う)
    let body = format!("foo\n{}foo", filler(600));
    let mut h = opened("/w/big.txt", body.as_bytes());
    let v = view(&h);
    h.editor
        .state_mut()
        .set_selection(v, Selection::single(Range::new(ByteOff(0), ByteOff(3))));
    h.cmd("pane.split_right");
    let right = view(&h);
    h.keys("ctrl+d");
    h.cmd("pane.focus_next");
    h.keys("ctrl+end");
    h.text("z");
    let before = selection_of(&h, right);
    h.run_jobs();
    assert_eq!(before.len(), 1);
    assert_eq!(
        selection_of(&h, right),
        before,
        "Revision が違うので足さない"
    );
}

// verifies: AC-FR-211-1
#[test]
fn 単発のジョブの結果が飛ばす一致なら_その後ろから続けて足す() {
    // 主 (先頭) と、間の `foo` を選んである。間の `foo` は飛ばし (同期検索とジョブを 2 回)、最後の `foo` を足す
    let body = format!("foo\n{}foo\n{}foo", filler(600), filler(600));
    let len = body.len();
    let mid = 4 + 600 * 1024;
    let mut h = opened("/w/big.txt", body.as_bytes());
    let v = view(&h);
    let sel = Selection::new(
        vec![
            Range::new(ByteOff(0), ByteOff(3)),
            Range::new(ByteOff(mid), ByteOff(mid + 3)),
        ],
        0,
    )
    .expect("Selection");
    h.editor.state_mut().set_selection(v, sel);
    h.keys("ctrl+d");
    assert_eq!(selection(&h).len(), 2, "ジョブの結果を待つ");
    h.clear_log();
    h.run_jobs();
    let s = selection(&h);
    assert_eq!(s.len(), 3, "{:?}", s.ranges());
    assert_eq!(*s.primary(), Range::new(ByteOff(len - 3), ByteOff(len)));
    assert!(h.messages().is_empty(), "{:?}", h.messages());
}

// verifies: AC-FR-211-1
#[test]
fn 飛ばす一致の後ろから続ける同期検索は_1_回分の予算を分け合う() {
    // 間の `foo` (選んである) まで 150 KiB、最後の `foo` まで更に 100 KiB: 続けた同期検索の合計は予算 (256 KiB) を超えるので、
    // その場では足さずにジョブに回す (呼び出しごとに予算を新しくすると、メインスレッドで 250 KiB を読んで足してしまう)
    let body = format!("foo\n{}foo\n{}foo", filler(150), filler(100));
    let mid = 4 + 150 * 1024;
    let mut h = opened("/w/big.txt", body.as_bytes());
    let v = view(&h);
    let sel = Selection::new(
        vec![
            Range::new(ByteOff(0), ByteOff(3)),
            Range::new(ByteOff(mid), ByteOff(mid + 3)),
        ],
        0,
    )
    .expect("Selection");
    h.editor.state_mut().set_selection(v, sel);
    h.keys("ctrl+d");
    assert_eq!(selection(&h).len(), 2, "予算を使い切ってジョブに回る");
    h.run_jobs();
    assert_eq!(selection(&h).len(), 3, "ジョブの結果で足す");
}

// verifies: AC-FR-211-1
#[test]
fn ジョブを経た末尾から先頭への回り込みは主の始まりで止まる() {
    // 主は最後の `foo`、先頭の `foo` も選んである。先頭へ回って先頭の `foo` を飛ばした後、同期検索の予算が尽きてジョブに
    // 回り、ジョブは主 (回った後の主の始まり) を見つけて止める
    let body = format!("foo\n{}foo", filler(600));
    let len = body.len();
    let mut h = opened("/w/big.txt", body.as_bytes());
    let v = view(&h);
    let sel = Selection::new(
        vec![
            Range::new(ByteOff(0), ByteOff(3)),
            Range::new(ByteOff(len - 3), ByteOff(len)),
        ],
        1,
    )
    .expect("Selection");
    h.editor.state_mut().set_selection(v, sel.clone());
    h.clear_log();
    h.keys("ctrl+d");
    assert!(h.messages().is_empty(), "同期検索では決まらない");
    h.run_jobs();
    assert_eq!(h.messages(), ["info.select.no_more_matches"]);
    assert_eq!(selection(&h), sel);
}

// verifies: AC-FR-211-1
#[test]
fn 検索バーの逐次検索の後も単語の一致の印は残る() {
    let mut h = scene("*|foo foobar foo");
    h.keys("ctrl+d");
    // 検索バーを開いて打つ (一致へスクロールするために選択を置き直して戻す) → 閉じる
    h.keys("ctrl+f");
    h.text("bar");
    h.keys("esc");
    assert_eq!(shown(&h), "«foo» foobar foo", "選択は変わらない");
    h.keys("ctrl+d");
    assert_eq!(
        shown(&h),
        "«foo» foobar «foo»",
        "単語単位のまま (foobar を飛ばす)"
    );
}

// verifies: AC-FR-211-1
#[test]
fn 長すぎる選択と文書の全体からは探さない() {
    let long = "x".repeat(eve_core_query_max() + 1);
    let mut h = scene(&format!("*«{long}» {long}"));
    assert_eq!(press(&mut h, "ctrl+d"), ["info.select.query_too_long"]);
    assert_eq!(press(&mut h, "ctrl+k e"), ["info.select.query_too_long"]);
    assert_eq!(selection(&h).len(), 1);
    // 上限ちょうどは探す
    let ok = "x".repeat(eve_core_query_max());
    let mut h = scene(&format!("*«{ok}» {ok}"));
    assert_eq!(press(&mut h, "ctrl+d"), NO_MESSAGE);
    assert_eq!(selection(&h).len(), 2);
    // 文書の全体 (`select.all` の後) は何もしない (知らせない)
    let mut h = scene("*|foo foo");
    h.keys("ctrl+a");
    let before = selection(&h);
    assert_eq!(press(&mut h, "ctrl+d"), NO_MESSAGE);
    assert_eq!(press(&mut h, "ctrl+k e"), NO_MESSAGE);
    assert_eq!(selection(&h), before);
}

/// 検索語の長さの上限 (03 §4.5.1 の MCUR-2 (k) の 64 KiB)。
fn eve_core_query_max() -> usize {
    64 * 1024
}

// ---------- select.all_matches (MCUR-3) ----------

// verifies: AC-FR-211-1
#[test]
fn すべて選ぶは一致で全部を置き換え_主は元の主と重なる一致() {
    let mut h = scene("foo bar f*|oo baz foo");
    h.keys("ctrl+shift+l");
    assert_eq!(shown(&h), "«foo» bar «foo» baz «foo»");
    assert_eq!(selection(&h).primary_index(), 1);
    // 代替キーでも同じ。選択から始めたときは単語単位でない
    let mut h = scene("*«foo» foobar");
    h.keys("ctrl+k e");
    assert_eq!(shown(&h), "«foo» «foo»bar");
    // 単語から始めたときは単語単位
    let mut h = scene("*|foo foobar foo");
    h.keys("ctrl+k e");
    assert_eq!(shown(&h), "«foo» foobar «foo»");
    assert_eq!(selection(&h).primary_index(), 0);
}

// verifies: AC-FR-211-3
#[test]
fn すべて選ぶは上限を超えるなら何も選ばずに知らせ_ちょうどなら選ぶ() {
    let mut h = scene(&format!("*|{}", "ab ".repeat(MAX_CURSORS + 1)));
    assert_eq!(press(&mut h, "ctrl+k e"), ["info.select.too_many_matches"]);
    assert_eq!(selection(&h).len(), 1, "一部だけを選ばない");
    assert!(selection(&h).primary().is_empty(), "選択を変えない");
    let mut h = scene(&format!("*|{}", "ab ".repeat(MAX_CURSORS)));
    assert_eq!(press(&mut h, "ctrl+k e"), NO_MESSAGE);
    assert_eq!(selection(&h).len(), MAX_CURSORS, "上限ちょうど");
}

// verifies: AC-FR-211-1
#[test]
fn 大きな文書のすべて選ぶは全件の検索のジョブで探す() {
    // 1 MiB を超える文書はメインスレッドで探さない
    let mut body = "foo\n".repeat(10);
    body.push_str(&format!("{}\n", "y".repeat(1023)).repeat(1024));
    body.push_str("foo");
    let mut h = opened("/w/big.txt", body.as_bytes());
    h.keys("ctrl+k e");
    assert_eq!(selection(&h).len(), 1, "ジョブの結果を待つ");
    h.run_jobs();
    let s = selection(&h);
    assert_eq!(s.len(), 11, "{:?}", s.ranges().first());
    assert_eq!(s.primary_index(), 0, "元の主 (先頭の単語) と重なる一致");
    // 結果が届く前に編集したら捨てる
    let mut h = opened("/w/big.txt", body.as_bytes());
    h.keys("ctrl+k e");
    h.text("z");
    h.run_jobs();
    assert_eq!(selection(&h).len(), 1, "Revision が違う");
}

// verifies: AC-FR-211-3
#[test]
fn 大きな文書のすべて選ぶも上限を超えるなら何も選ばない() {
    let mut body = "ab\n".repeat(MAX_CURSORS + 1);
    body.push_str(&format!("{}\n", "y".repeat(1023)).repeat(1024));
    let mut h = opened("/w/big.txt", body.as_bytes());
    h.keys("ctrl+k e");
    h.clear_log();
    h.run_jobs();
    assert_eq!(h.messages(), ["info.select.too_many_matches"]);
    assert_eq!(selection(&h).len(), 1);
}

// ---------- select.add_cursor_above / below (MCUR-4) ----------

/// `marked` の場面で、主のカーソルから `cursor.move_down` / `cursor.move_up` を `n` 回した位置 (足す操作の行き先と比べる)。
fn moved(marked: &str, keys: &str) -> usize {
    let mut h = scene(marked);
    h.keys(keys);
    selection(&h).primary().head.0
}

// verifies: AC-FR-211-1
#[test]
fn 下と上にカーソルを足し_短い行をはさんでも目標の列を保つ() {
    let start = "abc*|d\nx\nabcdef\nab";
    let mut h = scene(start);
    h.keys("ctrl+alt+down");
    assert_eq!(shown(&h), "abc|d\nx|\nabcdef\nab", "短い行は行末");
    h.keys("ctrl+alt+down");
    assert_eq!(shown(&h), "abc|d\nx|\nabc|def\nab", "目標の列 (3) に戻る");
    assert_eq!(
        selection(&h).primary().head.0,
        moved(start, "down down"),
        "上下の移動と同じ行き先"
    );
    // 代替キー。最後の行からは何もしない (知らせない)
    h.keys("ctrl+k shift+down");
    assert_eq!(shown(&h), "abc|d\nx|\nabc|def\nab|");
    assert_eq!(press(&mut h, "ctrl+alt+down"), NO_MESSAGE);
    assert_eq!(selection(&h).len(), 4);
    // 上へは一番上のカーソルから。先頭の行からは何もしない
    let mut h = scene("ab\nabc\nab*|c");
    h.keys("ctrl+alt+up ctrl+k shift+up");
    assert_eq!(shown(&h), "ab|\nab|c\nab|c");
    assert_eq!(selection(&h).primary().head.0, 2, "最後に足した Range が主");
    assert_eq!(press(&mut h, "ctrl+alt+up"), NO_MESSAGE);
    assert_eq!(selection(&h).len(), 3);
}

// verifies: AC-FR-211-1
#[test]
fn 全角と折り返しでは上下の移動と同じ表示の列へ足す() {
    // 全角: 表示列 2 は `あ` の後ろ
    let start = "あ*|いう\nabcdef";
    let mut h = scene(start);
    h.keys("ctrl+alt+down");
    assert_eq!(shown(&h), "あ|いう\nab|cdef");
    assert_eq!(selection(&h).primary().head.0, moved(start, "down"));
    // 折り返し: 長い 1 行の次の表示行へ (論理行の中)
    let long = format!("{}*|", "a".repeat(5));
    let marked = format!("{long}{}\nb", "a".repeat(60));
    let mut h = scene(&marked).with_size(20, 10);
    h.cmd("view.toggle_wrap");
    let mut want = scene(&marked).with_size(20, 10);
    want.cmd("view.toggle_wrap");
    want.keys("down");
    h.keys("ctrl+alt+down");
    let s = selection(&h);
    assert_eq!(s.len(), 2);
    assert_eq!(
        s.primary().head,
        selection(&want).primary().head,
        "表示行の単位の下の移動と同じ"
    );
    assert!(s.primary().head.0 < 65, "同じ論理行の中");
}

// verifies: AC-FR-211-1
#[test]
fn 足す位置が既にある選択の中なら_1_つになり数は増えない() {
    // 一番上の Range の head (2 行目) の上は、その Range の選択の中: 併合されて数は増えない (03 §4.4 の N4)
    let mut h = scene("*«ab\ncd»");
    assert_eq!(press(&mut h, "ctrl+alt+up"), NO_MESSAGE);
    assert_eq!(shown(&h), "«ab\ncd»");
    assert_eq!(selection(&h).len(), 1);
}

// verifies: AC-FR-211-3
#[test]
fn 上下に足して上限を超えるなら選択を変えずに知らせる() {
    let text = format!("*|{}", "a\n".repeat(MAX_CURSORS + 1));
    let mut h = scene(&text);
    let ranges: Vec<Range> = (0..MAX_CURSORS)
        .map(|i| Range::point(ByteOff(i * 2)))
        .collect();
    set_selection(&mut h, ranges, 0);
    let before = selection(&h);
    assert_eq!(press(&mut h, "ctrl+alt+down"), ["info.cursor.limit"]);
    assert_eq!(selection(&h), before);
}

// verifies: AC-FR-211-1
#[test]
fn ペインごとに足し_ほかのペインの選択は変えない() {
    let mut h = scene("abc\ndef\nghi*|");
    h.cmd("pane.split_right");
    let right = view(&h);
    h.keys("ctrl+alt+up ctrl+alt+up");
    assert_eq!(selection(&h).len(), 3);
    h.cmd("pane.focus_next");
    assert_ne!(view(&h), right);
    assert_eq!(selection(&h).len(), 1, "左のペインは 1 つのまま (PANE-2)");
}

// ---------- Esc (MCUR-5)・selection.multi・1 つにするコマンド ----------

// verifies: AC-FR-211-1
#[test]
fn esc_は主の_range_だけを残し_もう一度で選択を解く() {
    let mut h = scene("«one» *«two» three");
    h.keys("esc");
    assert_eq!(shown(&h), "one «two» three", "主の選択は残す");
    h.keys("esc");
    assert_eq!(shown(&h), "one two| three", "選択を解いて head のカーソル");
    h.keys("esc");
    assert_eq!(shown(&h), "one two| three", "それ以外は何もしない");
}

// verifies: AC-FR-211-1
#[test]
fn 検索バーやオーバーレイを開いている間の_esc_はそれだけを閉じて_range_を残す() {
    let mut h = scene("a|b *|c");
    h.keys("ctrl+f");
    h.keys("esc");
    assert_eq!(h.editor.state().overlay_kind(), "");
    assert_eq!(selection(&h).len(), 2, "検索バーを閉じるだけ");
    h.keys("ctrl+p");
    assert_ne!(h.editor.state().overlay_kind(), "");
    h.keys("esc");
    assert_eq!(h.editor.state().overlay_kind(), "");
    assert_eq!(selection(&h).len(), 2, "パレットを閉じるだけ");
    h.keys("esc");
    assert_eq!(selection(&h).len(), 1, "前の段が無ければ主だけに");
}

// verifies: AC-FR-211-1
#[test]
fn 未編集の上書き保存の照合を待つ間の_esc_は照合を中止し_range_を残す() {
    let mut h = opened("/w/a.txt", b"ab\ncd\n");
    h.keys("ctrl+alt+down");
    assert_eq!(selection(&h).len(), 2);
    h.keys("ctrl+s");
    assert_eq!(press(&mut h, "esc"), ["info.save.aborted"], "照合の中止");
    assert_eq!(selection(&h).len(), 2, "選択は減らさない");
    h.clear_log();
    h.run_jobs();
    assert!(
        !h.messages().iter().any(|k| k == "info.save.unchanged"),
        "照合の結果は捨てた: {:?}",
        h.messages()
    );
    h.keys("esc");
    assert_eq!(selection(&h).len(), 1);
}

// verifies: AC-FR-211-1
#[test]
fn すべて選ぶのジョブは検索バーの全件の走査を取り消さない() {
    let body = format!("foo\n{}foo", "y\n".repeat(600 * 1024));
    let mut h = opened("/w/big.txt", body.as_bytes());
    h.keys("ctrl+f");
    h.text("foo");
    h.cmd("select.all_matches");
    h.run_jobs();
    assert_eq!(selection(&h).len(), 2, "すべて選んだ");
    assert!(
        matches!(
            h.editor.state().search_count(),
            eve_core::SearchCount::Exact { total: 2, .. }
        ),
        "検索バーの全件の走査は最後まで数えた: {:?}",
        h.editor.state().search_count()
    );
}

// verifies: AC-FR-211-3
#[test]
fn selection_multi_は_range_が_2_つ以上のとき真() {
    let mut h = scene("a*|b");
    assert_eq!(context(&h, "selection.multi"), Some(CtxValue::Bool(false)));
    h.keys("ctrl+d");
    assert_eq!(context(&h, "selection.multi"), Some(CtxValue::Bool(false)));
    let mut h = scene("a\nb*|");
    h.keys("ctrl+alt+up");
    assert_eq!(context(&h, "selection.multi"), Some(CtxValue::Bool(true)));
    h.keys("esc");
    assert_eq!(context(&h, "selection.multi"), Some(CtxValue::Bool(false)));
}

// verifies: AC-FR-211-3
#[test]
fn 一つにするコマンドの後は_range_が_1_つ() {
    let mut h = scene("*|x foo foo foo");
    h.keys("ctrl+right ctrl+d ctrl+d ctrl+d");
    assert_eq!(selection(&h).len(), 3);
    h.keys("ctrl+a");
    assert_eq!(selection(&h).len(), 1, "select.all");
    let mut h = scene("*|x foo foo foo");
    h.keys("ctrl+right ctrl+k e");
    assert_eq!(selection(&h).len(), 3);
    h.cmd_with(
        "cursor.goto_line",
        eve_core::Args::new().with_int("line", 1),
    );
    assert_eq!(selection(&h).len(), 1, "cursor.goto_line");
}

// verifies: AC-FR-211-1
#[test]
fn 読み取り専用の文書でも足せる() {
    let mut h = opened("/w/ro.txt", b"foo foo\nfoo");
    h.cmd("file.toggle_read_only");
    h.keys("ctrl+d ctrl+d ctrl+alt+down");
    assert_eq!(selection(&h).len(), 3);
    h.keys("esc");
    h.keys("ctrl+k e");
    assert_eq!(selection(&h).len(), 3);
}

// ---------- AC-1 の流れ・貼り付けの代替キー ----------

// verifies: AC-FR-211-1
#[test]
fn 足したカーソルで入力し_1_回の_undo_で全部戻り_esc_で主に戻る() {
    let mut h = scene("let a = 1;\nlet b = 2;\nlet c = *|3;");
    h.keys("ctrl+alt+up ctrl+alt+up");
    let before = selection(&h);
    h.text("x");
    assert_eq!(h.doc_text(), "let a = x1;\nlet b = x2;\nlet c = x3;");
    h.keys("ctrl+z");
    assert_eq!(
        h.doc_text(),
        "let a = 1;\nlet b = 2;\nlet c = 3;",
        "1 回の Undo"
    );
    assert_eq!(selection(&h), before, "選択も複数のまま戻る");
    h.keys("delete enter tab");
    assert_eq!(
        h.doc_text(),
        "let a = \n    ;\nlet b = \n    ;\nlet c = \n    ;"
    );
    h.keys("ctrl+z");
    assert_eq!(
        h.doc_text(),
        "let a = \n;\nlet b = \n;\nlet c = \n;",
        "インデントを 1 回で戻す"
    );
    h.keys("esc");
    assert_eq!(
        shown(&h),
        "let a = \n|;\nlet b = \n;\nlet c = \n;",
        "主は最後に足したもの (一番上)"
    );
}

// verifies: AC-FR-211-2
#[test]
fn ctrl_k_v_は_edit_paste_でシステムのクリップボードを読む() {
    let mut h = scene("a|b*|");
    h.clear_log();
    h.keys("ctrl+k v");
    assert!(
        h.effects()
            .iter()
            .any(|e| matches!(e, Effect::RequestClipboard { .. })),
        "{:?}",
        h.effects()
    );
}

// ---------- ASCII の Frame のゴールデン (03 §4.5.5 の `secondary:` の行、§4.7.1 のゴールデン) ----------

// verifies: AC-FR-211-3
#[test]
fn ascii_の_frame_は主でないカーソルを_secondary_の行に出し_状態にカーソルの数を出す() {
    let mut h = scene("let a = 1;\nlet b = 2;\nlet *|c = 3;").with_size(40, 8);
    h.keys("ctrl+alt+up ctrl+alt+up shift+right");
    let text = h.frame_text_with(eve_testkit::AsciiOptions {
        show_gutter: true,
        show_chrome: true,
        ..eve_testkit::AsciiOptions::default()
    });
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/multi_cursor_secondary.frame.txt");
    eve_testkit::golden::check(&path, &text);
    // 主 (最後に足した 1 行目) は `cursor:`、ほかの 2 つは `secondary:` (行・列の順)
    assert!(text.contains("カーソル 3 個"), "{text}");
    let tail: Vec<&str> = text.lines().rev().take(2).collect();
    assert!(tail[0].starts_with("secondary: "), "{text}");
    assert!(tail[1].starts_with("cursor: "), "{text}");
    assert_eq!(tail[0].matches('(').count(), 2, "{text}");
}
