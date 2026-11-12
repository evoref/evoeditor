//! 行をまたぐ検索の呼び出し側 (docs/03 §12.3.1 の「呼び出し側」、08 §2.5 の M24 の (3))。
//!
//! 行をまたぐ検索は M24 の (4) まで利用者に開かないので、eve-core の内部の設定 (`SearchInternal`) で開いて確かめる。全件の
//! 走査は文書の先頭から選んだ正本で、件数・次 / 前・すべて置換・1 件置換は正本に従う。一致の集合・次 / 前の繰り返し・すべて
//! 置換の参照との突き合わせは `fuzz_regex_search.rs` (09 §3.3) が受け持ち、ここは検索バーとエディタの状態の振る舞いを見る。

// 内部の設定と検索の状態を直接使う (試験だけに許す Editor::state_mut、STR-B3)
#![allow(clippy::disallowed_methods)]

use eve_core::{
    CancelToken, Clock, FindJob, JobOutput, JobResult, LineSpan, ScanCtx, ScanEnd, SearchCount,
    SearchInternal, SearchJob, SearchQuery, SearchStep, Tick, find_next, run_search, scan_range,
};
use eve_testkit::Harness;
use eve_testkit::helpers::cell_text;
use eve_text::{ByteOff, ByteRange, LineIdx, Text, TextRead};
use eve_view::{Frame, StyleMods};

/// 行をまたぐ検索を開いた設定 (核を小さくして、窓の境目と再開の位置を多く作る)。
fn internal() -> SearchInternal {
    SearchInternal {
        core_bytes: 4,
        ..SearchInternal::multiline()
    }
}

/// `doc` を開き、内部の設定で行をまたぐ検索を開いた状態。カーソルは文書の先頭。
fn open(doc: &str) -> Harness {
    let mut h = Harness::new()
        .with_size(40, 12)
        .with_file("/w/a.txt", doc.as_bytes());
    h.open("/w/a.txt");
    h.run_jobs();
    h.editor.state_mut().set_search_internal(internal());
    h.keys("ctrl+home");
    h
}

/// 正規表現の検索語 (内部の設定で開いたもの)。
fn regex(p: &str) -> SearchQuery {
    SearchQuery::literal(p)
        .with_regex(true)
        .with_internal(internal())
}

/// 検索バーを開き、正規表現で `p` を入れる (ジョブは走らせない)。
fn bar_regex(h: &mut Harness, p: &str) {
    h.keys("ctrl+f");
    h.keys("alt+r");
    h.text(p);
}

fn count(h: &mut Harness) -> Option<String> {
    h.frame()
        .search
        .and_then(|b| b.count_text)
        .map(|t| cell_text(&t))
}

fn error(h: &mut Harness) -> Option<String> {
    h.frame()
        .search
        .and_then(|b| b.error)
        .map(|t| cell_text(&t))
}

/// 描かれた行の中で、`mods` を持つセルの文字列 (行の区切りは `|`)。
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
    out.trim_end_matches('|').to_owned()
}

fn step(h: &mut Harness, forward: bool) -> SearchStep {
    h.editor.state_mut().search_step(forward)
}

// ---- 全件の走査と件数 (正本) ----

#[test]
fn 一致の集合は文書の先頭から選び_件数と現在の一致が正本に従う() {
    // `a\na` を `a\na\na\na\na` で探すと、先頭からは [0,3) と [4,7) の 2 件 (2 行目の行頭から選ぶと [2,5) と [6,9))
    let mut h = open("a\na\na\na\na");
    h.keys("down down");
    assert_eq!(h.selection_range(), (4, 4));
    bar_regex(&mut h, r"a\na");
    h.run_jobs();
    assert_eq!(
        count(&mut h).as_deref(),
        Some("2/2"),
        "起点 4 以降の最初の一致 [4,7) が現在の一致"
    );
    assert_eq!(
        h.editor.state().search_count(),
        SearchCount::Exact {
            current: None,
            total: 2
        }
    );
    // 次へ: 現在の一致を選び、次は端を越えて先頭の一致へ
    h.keys("enter");
    assert_eq!(h.selection_range(), (4, 7));
    h.keys("enter");
    assert_eq!(h.selection_range(), (0, 3));
    assert!(
        h.messages()
            .iter()
            .any(|k| k == "info.search.wrapped_forward")
    );
    // 前へ: 先頭を越えて末尾の一致へ
    h.keys("up");
    assert_eq!(h.selection_range(), (4, 7));
    h.keys("up");
    assert_eq!(h.selection_range(), (0, 3));
}

#[test]
fn 既定では行をまたぐ検索語は今までどおり未対応() {
    let mut h = Harness::new().with_size(40, 12);
    h.text("a\na");
    h.keys("ctrl+home");
    bar_regex(&mut h, r"a\na");
    h.run_jobs();
    assert!(error(&mut h).is_some_and(|e| e.contains("未対応")));
}

#[test]
fn 全件ジョブは起点以降の最初の一致を_first_after_で知らせ_届くまで現在の一致を出さない() {
    let mut h = open("x\nab\ncd\nab\ncd\n");
    h.keys("down down");
    bar_regex(&mut h, r"b\nc");
    // 照合器も現在の一致もまだ無い (同期検索をしない)
    assert!(h.editor.state().search_query().is_some());
    let f = h.frame();
    assert_eq!(marked(&f, StyleMods::UNDERLINE), "");
    h.run_jobs();
    // 起点 (5) 以降の最初の一致は [9,12)。現在の一致として強調する (反転 + 太字 + 下線)
    let f = h.frame();
    assert_eq!(marked(&f, StyleMods::UNDERLINE), "|b|c|b|c");
    assert_eq!(marked(&f, StyleMods::UNDERLINE | StyleMods::BOLD), "|||b|c");
    assert_eq!(count(&mut h).as_deref(), Some("2/2"));
}

// ---- 次 / 前 ----

#[test]
fn 全件ジョブの走査中は単発のジョブを投げずに結果を待って移る() {
    let mut h = open("x\nab\ncd\nab\ncd\n");
    h.editor.state_mut().search_start(regex(r"b\nc"));
    assert_eq!(h.pending_jobs(), 1);
    assert_eq!(step(&mut h, true), SearchStep::NotYet);
    assert_eq!(h.pending_jobs(), 1, "単発のジョブは投げない");
    // 待っている間に同じ向きを押した回数は数え、届いて移った後に続ける
    assert_eq!(step(&mut h, true), SearchStep::NotYet);
    h.run_jobs();
    assert_eq!(h.selection_range(), (9, 12), "2 回分移った");
}

/// 文書を 1 回編集しておく (検索バーを開いた後に `edit.undo` で編集し、保持を無効にするため)。
fn pre_edit(h: &mut Harness) {
    h.keys("ctrl+end");
    h.text("z");
    h.keys("ctrl+home");
}

#[test]
fn 待ちが_100_ms_を過ぎたら件数の位置を待ちの表示にする() {
    let mut h = open("x\nab\ncd\nab\ncd\n");
    pre_edit(&mut h);
    bar_regex(&mut h, r"b\nc");
    h.run_jobs();
    assert_eq!(count(&mut h).as_deref(), Some("1/2"));
    // 編集で保持が無効になったので、次は単発のジョブで探す
    h.cmd("edit.undo");
    h.keys("ctrl+home");
    assert_eq!(count(&mut h).as_deref(), Some("約 2"));
    assert_eq!(step(&mut h, true), SearchStep::NotYet);
    assert_eq!(
        count(&mut h).as_deref(),
        Some("約 2"),
        "100 ms までは変えない"
    );
    h.advance_ms(150);
    assert_eq!(count(&mut h).as_deref(), Some("2/…"));
    h.run_jobs();
    assert_eq!(h.selection_range(), (3, 6));
    assert_eq!(count(&mut h).as_deref(), Some("約 2"));
}

#[test]
fn 編集の後の次_前は再開の位置と今の一致の終端から単発のジョブで探す() {
    // x を 5 行、`a` を 3 行、y を 5 行。`a\na` の正本は [10,13) の 1 件 ([12,15) は [10,13) と重なるので採らない)
    let doc = format!("{}a\na\na\n{}", "x\n".repeat(5), "y\n".repeat(5));
    let mut h = open(&doc);
    h.editor.state_mut().search_start(regex(r"a\na"));
    h.run_jobs();
    assert_eq!(
        h.editor.state().search_count(),
        SearchCount::Exact {
            current: None,
            total: 1
        }
    );
    // 末尾を編集する (保持は無効になる。編集した行より前の再開の位置は残る)
    h.keys("ctrl+end");
    h.text("z");
    h.keys("ctrl+home");
    assert_eq!(step(&mut h, true), SearchStep::NotYet, "単発のジョブで探す");
    h.run_jobs();
    assert_eq!(h.selection_range(), (10, 13));
    // 今の一致の終端 (13) から探す: 行頭 (12) から探すと [12,15) になるが、正本の次は無いので折り返す
    assert_eq!(step(&mut h, true), SearchStep::NotYet);
    h.run_jobs();
    assert_eq!(h.selection_range(), (10, 13));
    assert!(
        h.messages()
            .iter()
            .any(|k| k == "info.search.wrapped_forward")
    );
    // 前へ: 起点より前の一致は無いので、末尾から (折り返し)
    assert_eq!(step(&mut h, false), SearchStep::NotYet);
    h.run_jobs();
    assert_eq!(h.selection_range(), (10, 13));
}

#[test]
fn 単発のジョブを待つ間の同じ向きの連打はまとめて_届いた後に続ける() {
    let doc = "ab\ncd\nab\ncd\nab\ncd\nab\ncd\n";
    let mut h = open(doc);
    h.editor.state_mut().search_start(regex(r"b\nc"));
    h.run_jobs();
    h.keys("ctrl+end");
    h.text("z");
    h.keys("ctrl+home");
    assert_eq!(step(&mut h, true), SearchStep::NotYet);
    assert_eq!(step(&mut h, true), SearchStep::NotYet);
    assert_eq!(step(&mut h, true), SearchStep::NotYet);
    assert_eq!(
        h.pending_jobs(),
        1,
        "同じ向きの単発は取り消さず、新しく投げない"
    );
    h.run_jobs();
    assert_eq!(h.selection_range(), (13, 16), "3 回分 (3 番目の一致)");
    // 違う向きを押したら、待っていた単発を取り消して新しく投げる
    assert_eq!(step(&mut h, true), SearchStep::NotYet);
    assert_eq!(step(&mut h, false), SearchStep::NotYet);
    assert_eq!(h.pending_jobs(), 1);
    h.run_jobs();
    assert_eq!(h.selection_range(), (7, 10));
}

// ---- 可視範囲の強調 ----

#[test]
fn 可視範囲の強調は保持した一致を使い_編集の後は窓を同期検索する() {
    let mut h = open("ab\ncd\nxx\nab\ncd\n");
    pre_edit(&mut h);
    bar_regex(&mut h, r"b\nc");
    h.run_jobs();
    let f = h.frame();
    assert_eq!(marked(&f, StyleMods::UNDERLINE), "b|c||b|c");
    // 編集すると保持は無効になる。重くない検索語で窓が 64 KiB 以下なので、窓を同期検索して強調する
    h.cmd("edit.undo");
    assert_eq!(
        count(&mut h).as_deref().map(|c| c.starts_with('約')),
        Some(true)
    );
    let f = h.frame();
    assert_eq!(marked(&f, StyleMods::UNDERLINE), "b|c||b|c");
}

#[test]
fn 重い検索語は可視範囲を同期検索せず_保持した一致だけを強調する() {
    // 上限のある繰り返しの上限が 1000 を超える検索語は重い (メインスレッドで同期検索をしない)
    let mut h = open("ab\ncd\nxx\nab\ncd\n");
    pre_edit(&mut h);
    bar_regex(&mut h, r"b\nc[^\n]{0,2000}");
    h.run_jobs();
    let f = h.frame();
    assert_eq!(marked(&f, StyleMods::UNDERLINE), "b|cd||b|cd");
    h.cmd("edit.undo");
    let f = h.frame();
    assert_eq!(
        marked(&f, StyleMods::UNDERLINE),
        "",
        "編集の後は保持が無効で、同期検索もしない (編集の前の入力中の現在の一致も消す)"
    );
}

// ---- 単語単位・選択範囲内 ----

#[test]
fn 単語単位は一致の両端をそれぞれの行で調べる() {
    let mut h = open("foo\nbar o\nb");
    bar_regex(&mut h, r"o\nb");
    h.run_jobs();
    assert_eq!(count(&mut h).as_deref(), Some("1/2"));
    h.keys("alt+w");
    h.run_jobs();
    // `foo\nbar` の `o\nb` は始端が単語の途中。`o\nb` (文書の末尾) だけが両端とも単語境界
    assert_eq!(count(&mut h).as_deref(), Some("1/1"));
    h.keys("enter");
    assert_eq!(h.selection_range(), (8, 11));
}

#[test]
fn 選択範囲内は文書全体の正本から範囲に収まる一致だけを採る() {
    // 正本は [0,3) と [4,7)。範囲 [2,7) に収まるのは [4,7) だけ ([2,5) は正本ではない)
    let mut h = open("a\na\na\na");
    h.keys("down shift+ctrl+end");
    assert_eq!(h.selection_range(), (2, 7));
    bar_regex(&mut h, r"a\na");
    h.keys("alt+l");
    h.run_jobs();
    let st = h.editor.state();
    assert_eq!(
        st.search_count(),
        SearchCount::Exact {
            current: None,
            total: 1
        }
    );
    h.keys("enter");
    assert_eq!(h.selection_range(), (4, 7));
}

// ---- 置換 ----

/// 置換欄つきで、正規表現 `find` と置換の文字列 `with` を入れる。
fn setup_replace(h: &mut Harness, find: &str, with: &str) {
    h.keys("ctrl+r");
    h.keys("alt+r");
    h.text(find);
    h.keys("tab");
    h.text(with);
}

#[test]
fn すべて置換は正本の一致を置き換え_キャプチャと_crlf_を保つ() {
    // CRLF の文書。`(\w)\n(\w)` の正本は [1,5) と [5,9) (CRLF は 1 つの改行)
    let mut h = open("ab\r\ncd\r\nef");
    setup_replace(&mut h, r"(\w)\n(\w)", "$2$1");
    h.keys("alt+a");
    h.run_jobs();
    assert_eq!(h.doc_text(), "acbedf");
    assert!(h.messages().iter().any(|k| k == "info.search.replaced"));
}

#[test]
fn すべて置換で行をまたぐキャプチャの行の終端は文書の種類のまま入る() {
    let mut h = open("k=1\r\nv\r\nk=2\r\nv\r\n");
    setup_replace(&mut h, r"k=(\d\nv)", "[$1]");
    h.keys("alt+a");
    h.run_jobs();
    assert_eq!(h.doc_text(), "[1\r\nv]\r\n[2\r\nv]\r\n");
}

#[test]
fn 一件置換は正本の現在の一致を置き換え_待っている間の_enter_は何もしない() {
    let mut h = open("ab\ncd\nab\ncd\n");
    setup_replace(&mut h, r"(b)\n(c)", "$2-$1");
    // 照合器と現在の一致を待っている間の置換欄の Enter は何もしない (回数も数えない)
    h.keys("enter");
    h.keys("enter");
    assert_eq!(h.doc_text(), "ab\ncd\nab\ncd\n");
    h.run_jobs();
    h.keys("enter");
    assert_eq!(h.doc_text(), "ac-bd\nab\ncd\n");
    // 置き換えた後は再検索する。届く前の Enter は何もしない
    h.keys("enter");
    assert_eq!(h.doc_text(), "ac-bd\nab\ncd\n");
    h.run_jobs();
    h.keys("enter");
    assert_eq!(h.doc_text(), "ac-bd\nac-bd\n");
}

#[test]
fn 一件置換で選択が正本の一致でなければ入力中の現在の一致を置き換える() {
    // 正本は [0,3) と [4,7)。選択 [2,5) は一致の形だが正本ではない
    let mut h = open("a\na\na\na");
    h.editor.state_mut().search_start(regex(r"a\na"));
    h.keys("down shift+down");
    let sel = h.selection_range();
    assert_eq!(sel, (2, 4));
    h.keys("shift+right");
    assert_eq!(h.selection_range(), (2, 5));
    setup_replace(&mut h, r"a\na", "X");
    h.run_jobs();
    // 入力中の現在の一致 (起点 2 以降の最初の一致 [4,7)) を置き換える
    h.keys("enter");
    assert_eq!(h.doc_text(), "a\na\nX");
}

#[test]
fn 一件置換は窓の端の表明で正本の一致を取りこぼさない() {
    // 正本の一致は短い方の選択肢 (`a\n` や `a\nb`)。取り直しの窓を一致の後ろの 1 文字で切ると、窓の端で `\b`・`$`・`\z` が
    // 偽って成り立ち、優先の高い長い選択肢が当たって「対象でない」と判定してしまう (走査と同じ窓で取り直す)
    for (doc, pat, want) in [
        ("a\nbc", r"a\nb\b|a\n", "Xbc"),
        ("a\nbc", r"a\n(?:b$)?", "Xbc"),
        ("a\nbcd", r"(?s)a\nb.\z|a\nb", "Xcd"),
    ] {
        let mut h = open(doc);
        setup_replace(&mut h, pat, "X");
        h.run_jobs();
        h.keys("enter");
        h.run_jobs();
        assert_eq!(h.doc_text(), want, "{pat:?}");
        // すべて置換と同じ結果
        let mut h = open(doc);
        setup_replace(&mut h, pat, "X");
        h.keys("alt+a");
        h.run_jobs();
        assert_eq!(h.doc_text(), want, "すべて置換 {pat:?}");
    }
}

#[test]
fn 上限の無い検索語の一件置換はジョブで確かめる() {
    let mut h = open("a\nb\na\nb\n");
    setup_replace(&mut h, r"(?s)a.*?b", "[$0]");
    h.run_jobs();
    h.keys("enter");
    assert_eq!(h.doc_text(), "a\nb\na\nb\n", "メインスレッドで照合しない");
    assert_eq!(h.pending_jobs(), 1, "確かめのジョブ");
    // 待っている間の Enter は何もしない
    h.keys("enter");
    assert_eq!(h.pending_jobs(), 1);
    h.run_jobs();
    assert_eq!(h.doc_text(), "[a\nb]\na\nb\n");
}

#[test]
fn 次へを待つ間の置換欄の_enter_は飛ばしたつもりの一致を置き換えない() {
    let doc = "ab\ncd\nab\ncd\nab\ncd\n";
    let mut h = open(doc);
    pre_edit(&mut h);
    setup_replace(&mut h, r"b\nc", "X");
    h.run_jobs();
    // 入力中の現在の一致 [1,4) を選ぶ (正本の一致として記録される)
    h.cmd("search.next");
    assert_eq!(h.selection_range(), (1, 4));
    // 編集で保持が無効になる (Undo で選択は文書の末尾に戻る)。次へは単発のジョブで探し、折り返して [1,4) へ
    h.cmd("edit.undo");
    h.cmd("search.next");
    assert_eq!(h.pending_jobs(), 1, "単発のジョブ");
    h.run_jobs();
    assert_eq!(h.selection_range(), (1, 4));
    // もう一度次へ (単発のジョブを待つ)。届く前に置換欄で Enter: 飛ばしたつもりの [1,4) を置き換えない
    h.cmd("search.next");
    assert_eq!(h.pending_jobs(), 1);
    h.keys("enter");
    assert_eq!(h.doc_text(), doc);
    h.run_jobs();
    assert_eq!(h.selection_range(), (7, 10));
    h.keys("enter");
    assert_eq!(h.doc_text(), "ab\ncd\naXd\nab\ncd\n");
}

#[test]
fn 編集の後の古い現在の一致を正本として置き換えない() {
    // "zz" を打った文書で探すと入力中の現在の一致は [2,5)。Undo で消すと、[2,5) は今の文書の正本ではない
    // (`a\na\na\na\na` の正本は [0,3) と [4,7))
    let mut h = open("a\na\na\na\na");
    h.text("zz");
    setup_replace(&mut h, r"a\na", "X");
    h.run_jobs();
    h.cmd("edit.undo");
    assert_eq!(h.doc_text(), "a\na\na\na\na");
    let f = h.frame();
    assert_eq!(
        marked(&f, StyleMods::UNDERLINE | StyleMods::BOLD),
        "",
        "古い現在の一致は消す"
    );
    // 300 ms の再検索の前に置換欄で Enter を 2 回: 対象が無いので次を探す (2 回目は待っている間なので何もしない)
    h.keys("enter");
    h.keys("enter");
    assert_eq!(h.doc_text(), "a\na\na\na\na");
    h.run_jobs();
    assert_eq!(h.selection_range(), (0, 3), "正本の一致へ移る");
    h.keys("enter");
    assert_eq!(h.doc_text(), "X\na\na\na");
}

#[test]
fn 編集の後の古い現在の一致の終端から次を探さない() {
    let mut h = open("a\na\na\na\na");
    h.text("zz");
    bar_regex(&mut h, r"a\na");
    h.run_jobs();
    h.cmd("edit.undo");
    for _ in 0..3 {
        h.editor.state_mut().search_move(true);
        h.run_jobs();
        let sel = h.selection_range();
        assert!(
            [(0, 3), (4, 7)].contains(&sel),
            "正本でない {sel:?} へ移った"
        );
    }
}

#[test]
fn 編集の後に届いた古いジョブの現在の一致を採らない() {
    let mut h = open("a\na\na");
    h.keys("shift+right shift+right");
    h.text("xy");
    assert_eq!(h.doc_text(), "xya\na");
    setup_replace(&mut h, r"a\na", "X");
    // ジョブが届く前に Undo: "a\na\na" の正本は [0,3) だけ ([2,5) は一致の形だが正本でない)
    h.cmd("edit.undo");
    h.run_jobs();
    let f = h.frame();
    assert_eq!(marked(&f, StyleMods::UNDERLINE | StyleMods::BOLD), "");
    // 入力中の現在の一致を待っているので、置換欄の Enter は何もしない
    h.keys("enter");
    h.keys("enter");
    assert_eq!(h.doc_text(), "a\na\na");
    // 300 ms の再検索で今の文書の正本から求め直す
    h.advance_ms(10);
    h.advance_ms(300);
    h.run_jobs();
    h.keys("enter");
    assert_eq!(h.doc_text(), "X\na");
}

#[test]
fn すべて置換のジョブを待つ間に検索語を変えたら取り消す() {
    let doc = "ab\ncd\nab\ncd\n";
    let mut h = open(doc);
    setup_replace(&mut h, r"b\nc", "X");
    // 照合器がまだ無いので、すべて置換はジョブで作る
    h.keys("alt+a");
    h.keys("shift+tab");
    h.text("ZZZ");
    h.run_jobs();
    assert_eq!(h.doc_text(), doc, "古い検索語の結果を当てない");
    // 置換の文字列を変えたときも取り消す
    let mut h = open(doc);
    setup_replace(&mut h, r"b\nc", "X");
    h.keys("alt+a");
    h.text("Y");
    h.run_jobs();
    assert_eq!(h.doc_text(), doc);
}

#[test]
fn 選択範囲内のすべて置換は範囲をまたぐ一致を置き換えない() {
    let mut h = open("a\na\na\na");
    h.keys("down shift+ctrl+end");
    assert_eq!(h.selection_range(), (2, 7));
    setup_replace(&mut h, r"a\na", "X");
    h.keys("alt+l");
    h.run_jobs();
    h.keys("alt+a");
    h.run_jobs();
    // 正本 [0,3) は範囲をまたぐので採らず、[4,7) だけを置き換える
    assert_eq!(h.doc_text(), "a\na\nX");
}

#[test]
fn crlf_の文書で_r_のすべて置換は何もしない() {
    let doc = "a\r\nb\r\nc";
    let mut h = open(doc);
    setup_replace(&mut h, r"\r", "");
    h.keys("alt+a");
    h.run_jobs();
    assert_eq!(h.doc_text(), doc);
}

#[test]
fn 上限の無い検索語のすべて置換は文書の残りを_1_件として消し_1_回の元に戻すで戻る() {
    // 知らせは件数だけ (消した行の数を足すのは M24 の (4)、08 §2.5)
    let doc = "head\nkeep 1\nkeep 2\nkeep 3\ntail\n";
    let mut h = open(doc);
    setup_replace(&mut h, r"(?s)1.*", "");
    h.keys("alt+a");
    h.run_jobs();
    assert_eq!(h.doc_text(), "head\nkeep ");
    h.cmd("edit.undo");
    assert_eq!(h.doc_text(), doc);
}

// ---- k が 0 の重い検索語 ----

#[test]
fn k_が_0_の重い検索語は同期検索せず_現在の一致と次と置換はジョブで決める() {
    // 上限のある繰り返しの上限が 1000 を超えるので重い。k は 0 (行をまたがない)
    let mut h = open("x\nab\nab\nab\n");
    pre_edit(&mut h);
    h.keys("down down");
    bar_regex(&mut h, r"a[^\n]{0,2000}");
    assert_eq!(
        marked(&h.frame(), StyleMods::UNDERLINE),
        "",
        "照合器と結果を待つ"
    );
    h.run_jobs();
    // 起点 (5) 以降の最初の一致 [5,7) が入力中の現在の一致
    assert_eq!(
        marked(&h.frame(), StyleMods::UNDERLINE | StyleMods::BOLD),
        "||ab"
    );
    assert_eq!(count(&mut h).as_deref(), Some("2/3"));
    h.keys("enter");
    assert_eq!(h.selection_range(), (5, 7));
    // 編集の後の次は、同期検索をせずに単発のジョブ (Undo で選択は文書の末尾に戻るので、折り返して先頭の一致へ)
    h.cmd("edit.undo");
    assert_eq!(step(&mut h, true), SearchStep::NotYet);
    h.run_jobs();
    assert_eq!(h.selection_range(), (2, 4));
}

#[test]
fn k_が_0_の重い検索語の一件置換はジョブで確かめる() {
    let mut h = open("x\nab\nab\n");
    setup_replace(&mut h, r"a([^\n]{0,2000})", "<$1>");
    h.run_jobs();
    h.keys("enter");
    assert_eq!(h.pending_jobs(), 1, "確かめのジョブ");
    assert_eq!(h.doc_text(), "x\nab\nab\n");
    h.run_jobs();
    assert_eq!(h.doc_text(), "x\n<b>\nab\n");
}

// ---- 単発の検索を待つ間の起点 ----

#[test]
fn 単発を待つ間にカーソルを動かしてから押した次は新しい起点から探す() {
    let mut h = open("ab\ncd\nab\ncd\nab\ncd\n");
    h.editor.state_mut().search_start(regex(r"b\nc"));
    h.run_jobs();
    h.keys("ctrl+end");
    h.text("z");
    h.keys("ctrl+home");
    assert_eq!(step(&mut h, true), SearchStep::NotYet);
    // 待つ間にカーソルを 3 行目の行頭 (9) へ動かして、もう一度「次」: 取り消して新しい起点から投げ直す
    h.keys("down down down");
    assert_eq!(h.selection_range(), (9, 9));
    assert_eq!(step(&mut h, true), SearchStep::NotYet);
    assert_eq!(h.pending_jobs(), 1);
    h.run_jobs();
    assert_eq!(h.selection_range(), (13, 16));
}

// ---- 単発の検索と再開の位置の規則 (乱数の文書) ----

struct Zero;
impl Clock for Zero {
    fn now(&self) -> Tick {
        Tick(0)
    }
}

/// 全件のジョブの (正本の一致, 再開の位置)。
fn canon(t: &Text, q: &SearchQuery) -> (Vec<(usize, usize)>, Vec<usize>) {
    let mut out: Vec<JobOutput> = Vec::new();
    run_search(
        SearchJob {
            job: eve_view::JobId(1),
            snapshot: t,
            query: q,
            from: ByteOff(0),
            max_matches: usize::MAX,
            count_limit: u64::MAX,
        },
        &Zero,
        &CancelToken::new(),
        &mut |r| out.push(r),
    );
    let (mut m, mut res) = (Vec::new(), Vec::new());
    for r in out {
        match r.body {
            JobResult::SearchBatch { matches, .. } => {
                m.extend(matches.iter().map(|r| (r.start.0, r.end.0)));
            }
            JobResult::SearchDone { resume, .. } => res = resume.iter().map(|p| p.0).collect(),
            _ => {}
        }
    }
    (m, res)
}

/// `p` から末尾まで走査した一致 (走査が最後まで進まなければ `None`)。
fn from_point(t: &Text, q: &SearchQuery, p: usize) -> Option<Vec<(usize, usize)>> {
    let m = q.matcher()?;
    let cancel = CancelToken::new();
    let mut got = Vec::new();
    let end = scan_range(
        t,
        &m,
        ByteRange::new(ByteOff(p), ByteOff(t.len_bytes())),
        &mut ScanCtx::new(&cancel),
        &mut |r| {
            got.push((r.start.0, r.end.0));
            true
        },
    );
    (end == ScanEnd::Done).then_some(got)
}

/// 決定的な擬似乱数 (xorshift)。
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n as u64).unwrap_or(0)
    }
}

#[test]
fn 任意の起点からの単発の検索は正本の次_前() {
    let pats = [
        r"a\na",
        r"a\n\na",
        r"\n\n",
        r"a[\n ]{1,3}b",
        r"(?s)a.{0,3}b",
        r"b\n\s*a",
        r"a\n+",
        r"(?s)a.*b",
    ];
    let alphabet = ['a', 'b', ' ', '\n', '\r', 'x'];
    let mut rng = Rng(0x1234_5678_9abc_def1);
    let mut checked = 0;
    for _ in 0..3000 {
        let len = 2 + rng.below(30);
        let doc: String = (0..len)
            .map(|_| alphabet[rng.below(alphabet.len())])
            .collect();
        let pat = pats[rng.below(pats.len())];
        let q = regex(pat);
        let Some(m) = q.matcher() else { continue };
        if !m.whole_doc() {
            continue;
        }
        let t = Text::from(doc.as_str());
        let (want, resume) = canon(&t, &q);
        let pts: Vec<ByteOff> = resume.iter().map(|p| ByteOff(*p)).collect();
        let from = rng.below(doc.len() + 1);
        for forward in [true, false] {
            let got = find_next(
                &FindJob {
                    snapshot: &t,
                    query: &q,
                    from: ByteOff(from),
                    forward,
                    resume: &pts,
                },
                None,
                &CancelToken::new(),
            )
            .ok()
            .flatten()
            .map(|(r, w)| ((r.start.0, r.end.0), w));
            let expect = if forward {
                want.iter()
                    .copied()
                    .find(|r| r.0 >= from)
                    .map(|r| (r, false))
                    .or(want.first().map(|r| (*r, true)))
            } else {
                want.iter()
                    .copied()
                    .rev()
                    .find(|r| r.1 <= from)
                    .map(|r| (r, false))
                    .or(want.last().map(|r| (*r, true)))
            };
            assert_eq!(
                got, expect,
                "pat {pat:?} doc {doc:?} from {from} fwd {forward}"
            );
            checked += 1;
        }
    }
    assert!(checked > 100);
}

#[test]
fn 編集した行の_k_plus_1_行より前の再開の位置は新しい文書でも正本の位置() {
    let pats = [
        r"a\na",
        r"a\n\na",
        r"\n\n",
        r"a[\n ]{1,3}b",
        r"\r",
        r"a\r\nb",
        r"(?s)a.{0,3}b",
        r"b\n\s*a",
        r"a\nb\b|a\n",
        r"\na$",
    ];
    let alphabet = ['a', 'b', ' ', '\n', '\r', 'x'];
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut checked = 0;
    for _ in 0..4000 {
        let len = 4 + rng.below(40);
        let old: String = (0..len)
            .map(|_| alphabet[rng.below(alphabet.len())])
            .collect();
        let a = rng.below(old.len() + 1);
        let b = (a + rng.below(4)).min(old.len());
        let ins: String = (0..rng.below(3))
            .map(|_| alphabet[rng.below(alphabet.len())])
            .collect();
        let new = format!("{}{}{}", &old[..a], ins, &old[b..]);
        let pat = pats[rng.below(pats.len())];
        let q = regex(pat);
        let Some(m) = q.matcher() else { continue };
        if !m.whole_doc() {
            continue;
        }
        let LineSpan::Bounded(k) = m.lines() else {
            continue;
        };
        let (t_old, t_new) = (Text::from(old.as_str()), Text::from(new.as_str()));
        let (_, resume) = canon(&t_old, &q);
        let (want_new, _) = canon(&t_new, &q);
        // 実装 (map_search_marks) と同じ境
        let first = a.min(t_new.len_bytes());
        let line = t_new.byte_to_line(ByteOff(first)).0;
        let cutoff = match line.checked_sub(k + 1) {
            Some(l) => t_new.line_to_byte(LineIdx(l)).0,
            None => 0,
        };
        for &p in resume.iter().filter(|p| **p <= cutoff) {
            let Some(rest) = from_point(&t_new, &q, p) else {
                continue;
            };
            let expect: Vec<_> = want_new.iter().copied().filter(|r| r.0 >= p).collect();
            assert_eq!(
                rest, expect,
                "pat {pat:?} old {old:?} new {new:?} p {p} cutoff {cutoff}"
            );
            checked += 1;
        }
    }
    assert!(checked > 100, "確かめた数 {checked}");
}

// ---- 探さない理由・打ち切り ----

#[test]
fn 走査の途中で窓が上限を超えたら_届いた一致を捨てて理由を示す() {
    // k = 1 で最大の長さが無い検索語は、核を行の境界に揃える。長い行で窓が上限 (ここでは 32 バイト) を超える
    let doc = format!("a\nb\na\nb\n{}\na\nb", "c".repeat(100));
    let mut h = Harness::new()
        .with_size(40, 12)
        .with_file("/w/a.txt", doc.as_bytes());
    h.open("/w/a.txt");
    h.run_jobs();
    h.editor.state_mut().set_search_internal(SearchInternal {
        core_bytes: 4,
        concat_max: 32,
        ..SearchInternal::multiline()
    });
    h.keys("ctrl+home");
    bar_regex(&mut h, r"a\n[^\n]*");
    h.run_jobs();
    assert!(error(&mut h).is_some(), "理由を示す");
    assert_eq!(count(&mut h), None, "件数を出さない");
    let f = h.frame();
    assert_eq!(marked(&f, StyleMods::UNDERLINE), "", "強調を残さない");
    assert_eq!(h.editor.state().search_count(), SearchCount::Idle);
    // 次 / 前・すべて置換は何もしない
    h.keys("enter");
    assert_eq!(h.selection_range(), (0, 0));
}

#[test]
fn 捨てた候補の予算で打ち切ったら_届いた一致を捨てて理由を示す() {
    // 単語単位の `b[^\n]*` は、`abab…` の行で候補ごとに行の末尾まで読んで捨てる (O(n²))。予算 (1 MiB) を超えて打ち切る
    let doc = format!("b x\n{}\n", "ab".repeat(2048));
    let mut h = open(&doc);
    bar_regex(&mut h, r"b[^\n]*");
    h.keys("alt+w");
    h.run_jobs();
    assert!(error(&mut h).is_some(), "理由を示す");
    assert_eq!(count(&mut h), None);
    let f = h.frame();
    assert_eq!(marked(&f, StyleMods::UNDERLINE), "");
}

#[test]
fn 上限の無い検索語は大きな文書では探さず_ジョブを投げない() {
    let mut h = Harness::new()
        .with_size(40, 12)
        .with_file("/w/a.txt", "a\nb\n".repeat(20).as_bytes());
    h.open("/w/a.txt");
    h.run_jobs();
    h.editor.state_mut().set_search_internal(SearchInternal {
        concat_max: 16,
        ..SearchInternal::multiline()
    });
    h.keys("ctrl+r");
    h.keys("alt+r");
    h.text(r"a\n+");
    assert_eq!(h.pending_jobs(), 0, "ジョブを投げない");
    assert!(error(&mut h).is_some());
    // すべて置換も何もしない (メッセージ行に理由を出す)
    h.keys("tab");
    h.text("x");
    h.keys("alt+a");
    h.run_jobs();
    assert_eq!(h.doc_text(), "a\nb\n".repeat(20));
    assert!(
        h.messages()
            .iter()
            .any(|k| k == "msg.search.replace_invalid_regex")
    );
}

#[test]
fn 探さない理由があるとき次_前はメッセージ行に理由を出す() {
    let mut h = Harness::new()
        .with_size(40, 12)
        .with_file("/w/a.txt", "a\nb\n".repeat(20).as_bytes());
    h.open("/w/a.txt");
    h.run_jobs();
    h.editor.state_mut().set_search_internal(SearchInternal {
        concat_max: 16,
        ..SearchInternal::multiline()
    });
    bar_regex(&mut h, r"a\n+");
    h.keys("enter");
    assert!(h.messages().iter().any(|k| k == "msg.search.invalid_regex"));
    assert_eq!(h.selection_range(), (0, 0));
}

#[test]
fn 始める前に探さないと決めた検索語も_編集で条件を満たせば再検索で探す() {
    // 上限 40 バイトで、48 バイトの文書では探さない。編集で 40 バイト以下にすると、300 ms 後の再検索で探す
    let mut h = Harness::new()
        .with_size(40, 12)
        .with_file("/w/a.txt", "a\nb\n".repeat(12).as_bytes());
    h.open("/w/a.txt");
    h.run_jobs();
    h.editor.state_mut().set_search_internal(SearchInternal {
        concat_max: 40,
        ..SearchInternal::multiline()
    });
    h.keys("shift+down shift+down shift+down shift+down");
    h.keys("delete");
    assert_eq!(h.doc_text().len(), 40);
    h.cmd("edit.undo");
    bar_regex(&mut h, r"a\n+");
    assert!(error(&mut h).is_some());
    h.cmd("edit.redo");
    assert_eq!(h.doc_text().len(), 40);
    h.advance_ms(10);
    h.advance_ms(300);
    h.run_jobs();
    assert_eq!(error(&mut h), None, "{:?}", count(&mut h));
    assert_eq!(count(&mut h).as_deref(), Some("1/10"));
}
