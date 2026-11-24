//! 選択の拡張 (FR-309、docs/03 §5.7 の SEL-1〜SEL-7、07 §11.6 の「編集の拡張」、01 §5.5 の FR-309 AC-1) のシナリオ
//! (03 §15.9.1 の L3。M34 の段 5)。文書と Range の印は `marked` (`multi_cursor.rs` と共有)。

mod marked;

use eve_core::CommandResult;
use eve_testkit::Harness;
use eve_testkit::helpers::opened;
use eve_text::{ByteOff, Range, Selection};
use marked::{render, scene, selection, set_selection, shown, view};
use proptest::prelude::*;

/// `id` を `n` 回実行する。
fn repeat(h: &mut Harness, id: &str, n: usize) {
    for _ in 0..n {
        assert_eq!(h.cmd(id), CommandResult::Done, "{id}");
    }
}

/// `marked` の場面で `id` を 1 回実行した様子。
fn after(marked: &str, id: &str) -> String {
    let mut h = scene(marked);
    h.cmd(id);
    shown(&h)
}

/// `marked` の場面で `select.expand` を順に実行した各段の様子。
fn expansions(marked: &str, n: usize) -> Vec<String> {
    let mut h = scene(marked);
    (0..n)
        .map(|_| {
            h.cmd("select.expand");
            shown(&h)
        })
        .collect()
}

// ---------- 01 FR-309 AC-1 ----------

// verifies: AC-FR-309-1
#[test]
fn 行を選び_続けると次の行を足す() {
    let mut h = scene("ab|c\ndef\nghi");
    h.keys("ctrl+l");
    assert_eq!(shown(&h), "«abc\n»def\nghi");
    h.keys("ctrl+l");
    assert_eq!(shown(&h), "«abc\ndef\n»ghi");
    h.keys("ctrl+l");
    assert_eq!(
        shown(&h),
        "«abc\ndef\nghi»",
        "区切りの無い最後の行は文書の末尾まで"
    );
    h.keys("ctrl+l");
    assert_eq!(
        shown(&h),
        "«abc\ndef\nghi»",
        "文書の末尾まで選んでいれば変えない"
    );
    assert!(h.messages().is_empty(), "知らせない: {:?}", h.messages());
}

// verifies: AC-FR-309-1
#[test]
fn 単語_括弧の中_括弧を含む_行の中身_行の全体_文書の順に広げ_縮めると_1_段ずつ戻る() {
    let start = "fn f() {\n    let x = foo(bar, b|az);\n}\n";
    let want = [
        "fn f() {\n    let x = foo(bar, «baz»);\n}\n",
        "fn f() {\n    let x = foo(«bar, baz»);\n}\n",
        "fn f() {\n    let x = foo«(bar, baz)»;\n}\n",
        "fn f() {\n    «let x = foo(bar, baz);»\n}\n",
        "fn f() {\n«    let x = foo(bar, baz);\n»}\n",
        "fn f() {«\n    let x = foo(bar, baz);\n»}\n",
        "fn f() «{\n    let x = foo(bar, baz);\n}»\n",
        "«fn f() {\n    let x = foo(bar, baz);\n}»\n",
        "«fn f() {\n    let x = foo(bar, baz);\n}\n»",
    ];
    let mut h = scene(start);
    for w in want {
        h.keys("alt+u");
        assert_eq!(shown(&h), w);
    }
    h.keys("alt+u");
    assert_eq!(shown(&h), want[want.len() - 1], "文書の全体からは変えない");
    for w in want.iter().rev().skip(1) {
        h.keys("alt+i");
        assert_eq!(shown(&h), *w);
    }
    h.keys("alt+i");
    assert_eq!(shown(&h), start);
    h.keys("alt+i");
    assert_eq!(shown(&h), start, "履歴が尽きたら何もしない");
    assert!(h.messages().is_empty(), "知らせない: {:?}", h.messages());
}

// verifies: AC-FR-309-1
#[test]
fn チョードの代替でも広げて縮める() {
    let mut h = scene("a (b|c) d");
    h.keys("ctrl+k u");
    assert_eq!(shown(&h), "a («bc») d");
    h.keys("ctrl+k u");
    assert_eq!(shown(&h), "a «(bc)» d");
    h.keys("ctrl+k i");
    assert_eq!(shown(&h), "a («bc») d");
    h.keys("ctrl+k i");
    assert_eq!(shown(&h), "a (b|c) d");
}

// verifies: AC-FR-309-1
#[test]
fn 編集と移動の後は縮めても戻らない() {
    // 移動
    let mut h = scene("foo(b|ar)");
    repeat(&mut h, "select.expand", 2);
    h.keys("right");
    let moved = shown(&h);
    h.cmd("select.shrink");
    assert_eq!(shown(&h), moved, "移動の後");
    // 編集 (選択を置き換える)
    let mut h = scene("foo(b|ar)");
    h.cmd("select.expand");
    h.text("x");
    assert_eq!(h.doc_text(), "foo(x)");
    let typed = shown(&h);
    h.cmd("select.shrink");
    assert_eq!(shown(&h), typed, "編集の後");
    // Undo (Revision が変わる)
    h.cmd("edit.undo");
    let undone = shown(&h);
    h.cmd("select.shrink");
    assert_eq!(shown(&h), undone, "Undo の後");
    // ほかの選択の操作 (select.line は履歴に積まず、履歴を捨てる)
    let mut h = scene("foo(b|ar)\nx");
    h.cmd("select.expand");
    h.cmd("select.line");
    assert_eq!(shown(&h), "«foo(bar)\n»x");
    h.cmd("select.shrink");
    assert_eq!(shown(&h), "«foo(bar)\n»x", "select.line の後");
}

// verifies: AC-FR-309-1
#[test]
fn 複数カーソルでは各カーソルで広がり_重なれば_1_つになり_縮めると併合の前に戻る() {
    let mut h = scene("f|oo b*|ar\nx|yz\n");
    h.cmd("select.expand");
    assert_eq!(shown(&h), "«foo» «bar»\n«xyz»\n");
    assert_eq!(selection(&h).primary_index(), 1, "主の Range は bar のまま");
    h.cmd("select.expand");
    assert_eq!(
        shown(&h),
        "«foo bar»\n«xyz\n»",
        "同じ行の 2 つは行の中身で重なって 1 つ。xyz は行の中身と同じなので行の全体"
    );
    assert_eq!(selection(&h).len(), 2);
    assert_eq!(selection(&h).primary_index(), 0, "併合した Range が主");
    h.cmd("select.shrink");
    assert_eq!(shown(&h), "«foo» «bar»\n«xyz»\n", "併合の前の 3 つの Range");
    assert_eq!(selection(&h).primary_index(), 1);
    h.cmd("select.shrink");
    assert_eq!(shown(&h), "f|oo b|ar\nx|yz\n");
    assert_eq!(selection(&h).primary_index(), 1);
}

// ---------- SEL-1 (select.line) ----------

// verifies: FR-309
#[test]
fn 行の選択は触れる行の全体を前向きに選ぶ() {
    assert_eq!(
        after("a‹b\nc›d\ne", "select.line"),
        "«ab\ncd\n»e",
        "後ろ向きも前向きに"
    );
    // 前の行から始まって行頭で終わる選択は、その最後の行を含めない (03 §15.4)
    assert_eq!(after("a«b\n»cd\ne", "select.line"), "«ab\n»cd\ne");
    // 区切りで終わる文書の最後の空の行 (§2.4) は行の全体が空なので変わらない
    assert_eq!(after("ab\n|", "select.line"), "ab\n|");
    assert_eq!(after("|", "select.line"), "|", "空の文書");
    // 複数カーソル: 同じ行の 2 つは 1 つになる
    assert_eq!(after("a|b|c\nd|e", "select.line"), "«abc\nde»");
    assert_eq!(after("a|bc\nde\nf|g", "select.line"), "«abc\n»de\n«fg»");
    // 全角・結合文字
    assert_eq!(
        after("日本|語\nか\u{3099}", "select.line"),
        "«日本語\n»か\u{3099}"
    );
}

// verifies: FR-309
#[test]
fn 行の選択は_crlf_と_cr_の区切りの後ろまで() {
    let mut h = opened("/w/crlf.txt", b"ab\r\ncd\r\nef");
    set_selection(&mut h, vec![Range::point(ByteOff(5))], 0);
    h.cmd("select.line");
    assert_eq!(shown(&h), "ab\r\n«cd\r\n»ef");
    h.cmd("select.line");
    assert_eq!(shown(&h), "ab\r\n«cd\r\nef»");
    let mut h = opened("/w/cr.txt", b"ab\rcd\ref");
    set_selection(&mut h, vec![Range::point(ByteOff(1))], 0);
    h.cmd("select.line");
    assert_eq!(shown(&h), "«ab\r»cd\ref");
    h.cmd("select.line");
    assert_eq!(shown(&h), "«ab\rcd\r»ef");
}

// ---------- SEL-2 (select.expand の候補の順) ----------

// verifies: FR-309
#[test]
fn 候補の順は_foo_bar_baz_の各位置で真に含む最小のもの() {
    // 03 §15.9.1 の `foo(bar baz)` の各位置
    let cases = [
        ("|foo(bar baz)", "«foo»(bar baz)"),
        ("f|oo(bar baz)", "«foo»(bar baz)"),
        ("foo|(bar baz)", "foo«(»bar baz)"),
        ("foo(|bar baz)", "foo(«bar» baz)"),
        ("foo(bar| baz)", "foo(«bar» baz)"),
        ("foo(bar |baz)", "foo(bar «baz»)"),
        ("foo(bar baz|)", "foo(bar baz«)»"),
        ("foo(bar baz)|", "foo(bar baz«)»"),
        ("foo(«bar» baz)", "foo(«bar baz»)"),
        ("foo(«bar baz»)", "foo«(bar baz)»"),
        ("foo«(bar baz)»", "«foo(bar baz)»"),
        ("«foo(bar baz)»", "«foo(bar baz)»"),
        ("foo(«bar ba»z)", "foo(«bar baz»)"),
    ];
    for (from, want) in cases {
        assert_eq!(after(from, "select.expand"), want, "{from}");
    }
}

// verifies: FR-309
#[test]
fn 複数行の括弧と括弧の無い行と範囲を超えた括弧() {
    // 括弧の無い行: 単語 → 行の中身 → 行の全体 → 文書
    assert_eq!(
        expansions("  a|b cd\nef", 4),
        [
            "  «ab» cd\nef",
            "  «ab cd»\nef",
            "«  ab cd\n»ef",
            "«  ab cd\nef»"
        ]
    );
    // 複数行の括弧
    assert_eq!(
        expansions("[\n  a|b,\n]", 5),
        [
            "[\n  «ab»,\n]",
            "[\n  «ab,»\n]",
            "[\n«  ab,\n»]",
            "[«\n  ab,\n»]",
            "«[\n  ab,\n»]"
        ],
        "括弧の中の次は、括弧を含む範囲より短い行の全体"
    );
    // 範囲を超えた括弧 (対応が 1 MiB の外) は候補にしない
    let far = format!("(a|b {})", "x".repeat(1 << 20));
    let mut h = scene(&far);
    repeat(&mut h, "select.expand", 2);
    let r = *selection(&h).primary();
    assert_eq!(
        (r.start().0, r.end().0),
        (0, far.len() - 1),
        "単語の次は行の中身 (文書の全体と同じ)"
    );
}

// verifies: FR-309
#[test]
fn 括弧の入れ子と対応しない括弧とほかの組() {
    assert_eq!(
        expansions("((a) (|b) c)", 4),
        [
            "((a) («b») c)",
            "((a) «(b)» c)",
            "(«(a) (b) c»)",
            "«((a) (b) c)»"
        ]
    );
    // 対応しない括弧は飛ばして外側の組
    assert_eq!(expansions("{ ( a|b }", 2), ["{ ( «ab» }", "{« ( ab »}"]);
    // 閉じ括弧だけがある: 括弧の候補は無い
    assert_eq!(expansions("a|b) c", 2), ["«ab») c", "«ab) c»"]);
    // 全角の括弧 (10 組)。交差する組は、近い開き括弧の組から候補にする (ほかの組は数えない、BRK-3)
    assert_eq!(
        expansions("「（あ|い」）", 3),
        ["「（«あい»」）", "「（«あい」»）", "「«（あい」）»"]
    );
}

// verifies: FR-309
#[test]
fn 文字列とコメントと段落は候補にしない() {
    // `"a" + "b"` の真ん中を文字列と誤らない (SEL-6)
    assert_eq!(
        expansions("f(\"a\" + \"b|c\")", 2),
        ["f(\"a\" + \"«bc»\")", "f(«\"a\" + \"bc\"»)"]
    );
    assert_eq!(expansions("x // c|d e", 2), ["x // «cd» e", "«x // cd e»"]);
    // 段落 (空行で区切った塊) は候補にしない: 行の全体の次は文書
    assert_eq!(
        expansions("a\nb|\n\nc", 4),
        ["a\n«b»\n\nc", "a\n«b\n»\nc", "«a\nb\n\nc»", "«a\nb\n\nc»"]
    );
}

// verifies: FR-309
#[test]
fn 空の範囲と逆向きと文書の端と空の文書() {
    // 空白の上は単語が無い (MCUR-2 (a))
    assert_eq!(after("ab  |  cd", "select.expand"), "«ab    cd»");
    // 逆向きは保つ
    assert_eq!(after("(‹ab› cd)", "select.expand"), "(‹ab cd›)");
    assert_eq!(after("(‹ab cd›)", "select.expand"), "‹(ab cd)›");
    // 文書の先頭・末尾
    assert_eq!(after("|ab", "select.expand"), "«ab»");
    assert_eq!(after("ab|", "select.expand"), "«ab»");
    // 区切りで終わる文書の最後の空の行からは文書の全体
    assert_eq!(after("ab\n|", "select.expand"), "«ab\n»");
    // 空の文書は変わらない
    assert_eq!(after("|", "select.expand"), "|");
    // 全角と結合文字
    assert_eq!(after("「漢|字かな」", "select.expand"), "「«漢字»かな」");
    assert_eq!(
        after("e\u{301}|t\u{301}e x", "select.expand"),
        "«e\u{301}t\u{301}e» x"
    );
}

// verifies: FR-309
#[test]
fn crlf_の文書の行の中身と行の全体() {
    let mut h = opened("/w/crlf.txt", b"a\r\n  b c\r\nd");
    set_selection(&mut h, vec![Range::point(ByteOff(5))], 0);
    let mut seen = Vec::new();
    for _ in 0..3 {
        h.cmd("select.expand");
        seen.push(shown(&h));
    }
    assert_eq!(
        seen,
        [
            "a\r\n  «b» c\r\nd",
            "a\r\n  «b c»\r\nd",
            "a\r\n«  b c\r\n»d"
        ]
    );
}

// verifies: FR-309
#[test]
fn cr_だけの改行の文書の行の中身と行の全体() {
    let mut h = opened("/w/cr.txt", b"a\r  b c\rd");
    set_selection(&mut h, vec![Range::point(ByteOff(4))], 0);
    let mut seen = Vec::new();
    for _ in 0..4 {
        h.cmd("select.expand");
        seen.push(shown(&h));
    }
    assert_eq!(
        seen,
        [
            "a\r  «b» c\rd",
            "a\r  «b c»\rd",
            "a\r«  b c\r»d",
            "«a\r  b c\rd»"
        ]
    );
}

// verifies: FR-309
#[test]
fn 閉じ括弧を選んだ後は_その組を含む範囲へ広げる() {
    // 直後の記号は 1 書記素の単語 (MCUR-2 (a))。閉じ括弧を含む Range も、その組に囲まれる (SEL-2)
    assert_eq!(expansions("foo(|)", 3), ["foo(«)»", "foo«()»", "«foo()»"]);
    assert_eq!(
        expansions("x = foo(bar baz|);", 3),
        [
            "x = foo(bar baz«)»;",
            "x = foo«(bar baz)»;",
            "«x = foo(bar baz);»"
        ]
    );
    assert_eq!(
        expansions("f(「あ|」)", 3),
        ["f(「あ«」»)", "f(«「あ」»)", "f«(「あ」)»"]
    );
}

// verifies: FR-309
#[test]
fn 開き括弧を選んだ後は_その組を含む範囲へ広げる() {
    // 直後の記号 `(` は 1 書記素の単語 (MCUR-2 (a))。開き括弧を含む Range も、その組に囲まれる (SEL-2)
    assert_eq!(
        expansions("x = foo|(bar);", 3),
        ["x = foo«(»bar);", "x = foo«(bar)»;", "«x = foo(bar);»"]
    );
    assert_eq!(after("x = «(ba»r);", "select.expand"), "x = «(bar)»;");
    // 入れ子: 外側の `(` なら外側の組、内側の `(` なら内側の組
    assert_eq!(after("«(»(a))", "select.expand"), "«((a))»");
    assert_eq!(after("(«(»a))", "select.expand"), "(«(a)»)");
    assert_eq!(
        expansions("f「|「あ」」", 3),
        ["f「«「»あ」」", "f「«「あ」»」", "f«「「あ」」»"]
    );
}

// verifies: FR-309
#[test]
fn 後ろ向きの文書の末尾までの行の全体は_select_line_で変えない() {
    assert_eq!(after("‹ab\ncd›", "select.line"), "‹ab\ncd›");
    assert_eq!(after("ab\n‹cd›", "select.line"), "ab\n‹cd›");
    assert_eq!(after("‹ab\ncd\n›", "select.line"), "‹ab\ncd\n›");
    // 文書の末尾でなければ次の行を足して前向き
    assert_eq!(after("‹ab\n›cd", "select.line"), "«ab\ncd»");
}

// verifies: FR-309
#[test]
fn 何も変わらないときは目標の列と単語の一致の印を消さない() {
    // select.expand: 文書の全体を選び、目標の列 (3) を持つ (Shift+End の後に Shift+↓)
    let mut h = scene("|abc\nx");
    h.keys("shift+end shift+down");
    assert_eq!(shown(&h), "«abc\nx»");
    h.cmd("select.expand");
    assert_eq!(shown(&h), "«abc\nx»", "文書の全体からは変えない");
    h.keys("shift+up");
    assert_eq!(shown(&h), "«abc»\nx", "目標の列 3 で上の行へ");
    // select.line: 区切りで終わる文書の最後の空の行 (目標の列 3 で ↓ した)
    let mut h = scene("abc|\n");
    h.keys("down");
    assert_eq!(shown(&h), "abc\n|");
    h.cmd("select.line");
    assert_eq!(shown(&h), "abc\n|");
    h.keys("up");
    assert_eq!(shown(&h), "abc|\n", "目標の列が残る");
    // select.line: 文書の末尾までの行の全体を Ctrl+D (単語の一致の印) で選んでいる
    let mut h = scene("abc\na|b");
    h.keys("ctrl+d");
    assert_eq!(shown(&h), "abc\n«ab»");
    h.cmd("select.line");
    assert_eq!(shown(&h), "abc\n«ab»", "行の全体で文書の末尾まで");
    h.clear_log();
    h.keys("ctrl+d");
    assert_eq!(
        shown(&h),
        "abc\n«ab»",
        "印が残るので単語単位で探し、abc の中の ab には当たらない"
    );
    assert_eq!(h.messages(), ["info.select.no_more_matches"]);
}

// verifies: FR-309
#[test]
fn 検索バーの逐次検索の後も拡張の履歴が残る() {
    let mut h = scene("foo(b|ar) bar");
    repeat(&mut h, "select.expand", 2);
    assert_eq!(shown(&h), "foo«(bar)» bar");
    h.keys("ctrl+f");
    h.text("b");
    h.keys("escape");
    assert_eq!(shown(&h), "foo«(bar)» bar", "検索バーは選択を変えない");
    h.cmd("select.shrink");
    assert_eq!(shown(&h), "foo(«bar») bar");
    h.cmd("select.shrink");
    assert_eq!(shown(&h), "foo(b|ar) bar");
}

// ---------- SEL-3 / SEL-4 (縮める・履歴) ----------

// verifies: FR-309
#[test]
fn 履歴は_64_個までで古いものから捨てる() {
    // 70 段の入れ子: 単語の後、1 段ずつ外側の括弧を含む範囲へ広がる
    let depth = 70;
    let text = format!("{}x{}", "(".repeat(depth), ")".repeat(depth));
    let mut h = scene(&format!("{}|x{}", "(".repeat(depth), ")".repeat(depth)));
    let mut seen = vec![shown(&h)];
    for _ in 0..depth + 1 {
        h.cmd("select.expand");
        seen.push(shown(&h));
    }
    assert_eq!(seen[depth + 1], format!("«{text}»"));
    repeat(&mut h, "select.shrink", depth + 1);
    // 71 回広げて、残る履歴は最後の 64 個: 7 回目の後の選択までしか戻らない
    assert_eq!(shown(&h), seen[depth + 1 - 64]);
}

// verifies: FR-309
#[test]
fn ほかのペインの編集で写像された後は縮めても戻らない() {
    let mut h = opened("/w/p.txt", b"foo(bar)\nzzz");
    let v = view(&h);
    let first = h.editor.state().view(v).expect("View").selection.clone();
    assert_eq!(first, Selection::point(ByteOff(0)));
    set_selection(&mut h, vec![Range::point(ByteOff(5))], 0);
    repeat(&mut h, "select.expand", 2);
    assert_eq!(shown(&h), "foo«(bar)»\nzzz");
    // 別のペインで文書の末尾を編集する (このペインの選択は写像で変わらない)
    h.cmd("pane.split_right");
    h.keys("ctrl+end");
    h.text("!");
    h.cmd("pane.focus_prev");
    assert_eq!(view(&h), v);
    assert_eq!(shown(&h), "foo«(bar)»\nzzz!");
    h.cmd("select.shrink");
    assert_eq!(
        shown(&h),
        "foo«(bar)»\nzzz!",
        "Revision が変わったので履歴は無い"
    );
}

// ---------- SEL-7 (Motion) ----------

// verifies: FR-309
#[test]
fn 文書を変えず_undo_の結合を封印し_読み取り専用でも使える() {
    let mut h = Harness::new();
    h.text("ab");
    h.cmd("select.expand");
    h.cmd("select.shrink");
    h.cmd("select.line");
    h.keys("right");
    h.text("c");
    assert_eq!(h.doc_text(), "abc");
    h.cmd("edit.undo");
    assert_eq!(h.doc_text(), "ab", "Motion が Undo の結合を封印する");
    // 読み取り専用の文書
    let mut h = opened("/w/ro.txt", b"foo(bar)");
    assert_eq!(h.cmd("file.toggle_read_only"), CommandResult::Done);
    set_selection(&mut h, vec![Range::point(ByteOff(5))], 0);
    assert_eq!(h.cmd("select.expand"), CommandResult::Done);
    assert_eq!(shown(&h), "foo(«bar»)");
    assert_eq!(h.cmd("select.line"), CommandResult::Done);
    assert_eq!(shown(&h), "«foo(bar)»");
    assert_eq!(h.cmd("select.shrink"), CommandResult::Done);
    assert!(!h.is_dirty());
}

// verifies: FR-309
#[test]
fn 目標の列と単語の一致の印を消す() {
    // 目標の列: 下へ移って短い行で広げて縮めると、次の下への移動は今の列から
    let mut h = scene("abcdef|\nx\nabcdef");
    h.keys("down");
    assert_eq!(shown(&h), "abcdef\nx|\nabcdef");
    h.cmd("select.expand");
    h.cmd("select.shrink");
    h.keys("down");
    assert_eq!(shown(&h), "abcdef\nx\na|bcdef", "目標の列 (6) を消した");
    // 単語の一致の印: Ctrl+D で単語を選んだ後に広げて縮めると、次の一致は単語単位でなくなる
    let mut h = scene("f|oo foobar");
    h.keys("ctrl+d");
    assert_eq!(shown(&h), "«foo» foobar");
    h.cmd("select.expand");
    h.cmd("select.shrink");
    assert_eq!(shown(&h), "«foo» foobar");
    h.keys("ctrl+d");
    assert_eq!(shown(&h), "«foo» «foo»bar", "印が無いので部分にも一致する");
}

// ---------- 性質 (03 §15.9.1 の proptest (a)) ----------

const PIECES: &[&str] = &[
    "foo", "ba_r", " ", "  ", "\t", "\n", "\r\n", "\r", "(", ")", "[", "]", "{", "}", "「", "」",
    "（", "）", "\"", ",", ".", "日本", "かな", "カナ", "\u{3000}", "e\u{301}", "    ",
];

fn doc_text() -> impl Strategy<Value = String> {
    proptest::collection::vec(proptest::sample::select(PIECES.to_vec()), 0..30)
        .prop_map(|v| v.concat())
}

/// 文字列の任意の位置の印を付けた場面 (Range 1〜4 個)。
fn setup(text: &str, raw: &[(u16, u16)], primary: usize) -> Harness {
    let mut h = opened("/w/p.txt", text.as_bytes());
    let len = text.len() + 1;
    let fix = |p: u16| {
        let mut p = usize::from(p) % len;
        while !text.is_char_boundary(p) {
            p -= 1;
        }
        ByteOff(p)
    };
    let rs: Vec<Range> = raw
        .iter()
        .map(|&(a, b)| Range::new(fix(a), fix(b)))
        .collect();
    let n = rs.len();
    set_selection(&mut h, rs, primary % n);
    h
}

/// `outer` の各 Range が、`inner` の Range のどれかを含む向きの写像として、`inner` の各 Range を含む Range が `outer` にある。
fn covers(outer: &Selection, inner: &Selection) -> bool {
    inner.iter().all(|r| {
        outer
            .iter()
            .any(|o| o.start() <= r.start() && o.end() >= r.end())
    })
}

/// CI の proptest が見つけた例。文書の末尾までの後ろ向きの主は `select.line` で変わらず (03 §5.7 の細目 (d))、
/// その前の行の選択と接して併合すると、併合した Range は主の向き (後ろ向き) になる (03 §4.4 の N3)。
// spec: 03 §5.7, 03 §4.4
#[test]
fn 行の選択で後ろ向きの主に接して併合した_range_は主の向きになる() {
    let text = "\r日本\n   foo   日本ba_r\u{3000}    日本   ";
    let mut h = opened("/w/p.txt", text.as_bytes());
    let end = text.len();
    set_selection(
        &mut h,
        vec![
            Range::new(ByteOff(0), ByteOff(1)),
            Range::new(ByteOff(end), ByteOff(8)),
        ],
        1,
    );
    h.cmd("select.line");
    let after = selection(&h);
    assert_eq!(after.iter().count(), 1);
    assert_eq!(*after.primary(), Range::new(ByteOff(end), ByteOff(0)));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(160))]

    /// (a) 任意の Selection に `select.expand` を k 回、`select.shrink` を k 回行うと元の Selection に戻る。広げた選択は前の
    /// 選択を含み、変わったときは前の選択と等しくない。文書は変わらない。
    #[test]
    fn 広げて同じ回数だけ縮めると元に戻る(
        text in doc_text(),
        raw in proptest::collection::vec((any::<u16>(), any::<u16>()), 1..5),
        primary in any::<usize>(),
        k in 1usize..8,
    ) {
        let mut h = setup(&text, &raw, primary);
        let start = selection(&h);
        let mut prev = start.clone();
        for _ in 0..k {
            h.cmd("select.expand");
            let now = selection(&h);
            prop_assert!(covers(&now, &prev), "{:?}: {} → {}", text, render(&text, &prev), render(&text, &now));
            if now == prev {
                // 変わらないのは、全部の Range が文書の全体のときだけ
                prop_assert!(now.iter().all(|r| r.start().0 == 0 && r.end().0 == text.len()), "{:?} {:?}", text, now);
            }
            prev = now;
        }
        prop_assert_eq!(h.doc_text(), text.clone());
        for _ in 0..k {
            h.cmd("select.shrink");
        }
        prop_assert_eq!(selection(&h), start, "{:?}", text);
    }

    /// `select.line` の後、各 Range は行の全体 (行頭から始まり、行頭か文書の末尾で終わる) か、区切りで終わる文書の最後の空の行の
    /// 空のカーソルで、前の Range を含む。
    #[test]
    fn 行の選択は前の選択を含む行の全体(
        text in doc_text(),
        raw in proptest::collection::vec((any::<u16>(), any::<u16>()), 1..5),
        primary in any::<usize>(),
    ) {
        let mut h = setup(&text, &raw, primary);
        let before = selection(&h);
        h.cmd("select.line");
        let after = selection(&h);
        prop_assert!(covers(&after, &before), "{:?}", text);
        let line_starts: Vec<usize> = std::iter::once(0)
            .chain(text.char_indices().filter_map(|(i, c)| match c {
                '\n' => Some(i + 1),
                '\r' if !text[i + 1..].starts_with('\n') => Some(i + 1),
                _ => None,
            }))
            .collect();
        for r in after.iter() {
            prop_assert!(line_starts.contains(&r.start().0), "{:?} {:?}", text, r);
            prop_assert!(
                r.end().0 == text.len() || line_starts.contains(&r.end().0),
                "{:?} {:?}", text, r
            );
            // 前向きか、元のままの Range か、後ろ向きの元の主を含んで併合した主の Range
            // (併合した Range の向きは主に従う。03 §4.4 の N3)
            let merged_into_primary = r == after.primary() && !before.primary().is_forward();
            prop_assert!(
                r.is_forward() || before.iter().any(|b| b == r) || merged_into_primary,
                "{:?} {:?}", text, r
            );
        }
    }
}
