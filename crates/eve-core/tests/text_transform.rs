//! テキスト変換 (docs/03 §15.8 の TXT-1〜TXT-7、§15.9.1、01 FR-214 AC-1 / AC-2。v0.5 の M34 の段 2)。
//!
//! 表の試験で TXT-1〜TXT-7 を、proptest で 03 §15.9.1 の性質を確かめる:
//!
//! - (a) 並べ替えの結果が参照モデル「ブロックの行の中身をコードポイントの順に安定に並べ替え、区切りは動かさない (CR と
//!   LF がつながるならブロックを変えない)」と一致する。行の中身の多重集合と区切りの並び (種類と位置の順) が元と同じ。
//! - (b) 末尾空白の除去の結果が参照モデル「行ごとに末尾の U+0020 / U+0009 を落とす (CR と LF がつながる行は落とさない)」
//!   と一致し、行の数が変わらず、`ChangeSet` は消す範囲だけで、2 回目は何も変えない (冪等)。
//! - (e) 対象が上限を超えてジョブで作った結果 (本文・選択・Undo の後の本文) が、メインスレッドで作ったものと同じ
//!   (`TRANSFORM_SYNC_BYTES` を試験の値で小さくして両方の経路を通す)。

// 参照の文字列に `ChangeSet` を当てる (`eve_text::Text::apply`。文書の外の参照の本文)
#![allow(clippy::disallowed_methods)]

use eve_config::{Config, ExternalChangePolicy};
use eve_core::{
    CancelToken, CommandResult, DocumentId, InitialFile, JobError, JobResult, Origin,
    TRANSFORM_SYNC_BYTES, Tick, Transaction, TransformOp, WATCH_SETTLE, build_text_transform,
};
use eve_testkit::{FakeWatcher, Harness};
use eve_text::{
    ByteOff, ByteRange, ChangeSet, Edit, Op, Range, Selection, Snapshot, Text, TextRead,
};
use proptest::prelude::*;

const UPPER: &str = "edit.transform_upper";
const LOWER: &str = "edit.transform_lower";
const SORT_ASC: &str = "edit.sort_lines_ascending";
const SORT_DESC: &str = "edit.sort_lines_descending";
const TRIM: &str = "edit.trim_trailing_whitespace";
const ALL_IDS: [&str; 5] = [UPPER, LOWER, SORT_ASC, SORT_DESC, TRIM];

const PATH: &str = "/w/a.txt";

/// `text` のファイルを開いた Harness (改行はファイルのまま)。
fn open_with(text: &str, read_only: bool) -> Harness {
    let mut h = Harness::builder()
        .defer_start()
        .file(PATH, text.as_bytes())
        .build();
    h.editor
        .open_initial_files(&[InitialFile::path(PATH)], read_only, None);
    h.started();
    h.run_jobs();
    assert_eq!(h.doc_text(), text, "本文がそのまま開ける");
    h
}

fn open(text: &str) -> Harness {
    open_with(text, false)
}

/// 上限を `bytes` にした Harness (0 なら、空でない対象はどれもジョブで作る)。
fn open_job(text: &str, bytes: usize) -> Harness {
    let mut h = open(text);
    h.editor.state_mut().set_transform_sync_bytes(bytes);
    h
}

fn view(h: &Harness) -> eve_view::ViewId {
    h.editor
        .state()
        .window_view(h.window())
        .expect("View がある")
}

fn selection(h: &Harness) -> Selection {
    h.editor
        .state()
        .view(view(h))
        .expect("View がある")
        .selection
        .clone()
}

/// Range (`(anchor, head)` のバイト位置) を置く。最初の Range が主。
fn select(h: &mut Harness, ranges: &[(usize, usize)]) {
    let rs = ranges
        .iter()
        .map(|&(a, b)| Range::new(ByteOff(a), ByteOff(b)))
        .collect();
    let v = view(h);
    h.editor
        .state_mut()
        .set_selection(v, Selection::new(rs, 0).expect("Selection"));
}

fn ranges(h: &Harness) -> Vec<(usize, usize)> {
    selection(h)
        .iter()
        .map(|r| (r.anchor.0, r.head.0))
        .collect()
}

fn revision(h: &Harness) -> eve_view::Revision {
    h.editor
        .state()
        .document(h.doc_id())
        .expect("文書")
        .revision()
}

/// 文書を外から (利用者の入力として) 置き換える。`[from, to)` を `insert` にする。
fn edit(h: &mut Harness, from: usize, to: usize, insert: &str) {
    let doc = h.doc_id();
    let cs = {
        let t = h.editor.state().document(doc).expect("文書").text();
        ChangeSet::from_sorted_edits(
            t,
            [Edit {
                range: ByteRange::new(ByteOff(from), ByteOff(to)),
                insert: insert.into(),
            }],
        )
        .expect("編集")
    };
    h.editor
        .state_mut()
        .apply_transaction(doc, Transaction::new(cs, Origin::User, Tick(0)))
        .expect("適用");
}

/// 最後に出た文言のキー。
fn last_message(h: &Harness) -> Option<String> {
    h.messages().last().cloned()
}

/// 参照の本文に `ChangeSet` を当てた結果。
fn applied(text: &str, cs: &ChangeSet) -> String {
    let mut t = Text::from(text);
    t.apply(cs).expect("当てられる");
    t.slice_to_string(ByteRange::all(t.len_bytes()))
}

/// 文書の全体を対象にした変換の結果 (メインスレッドの経路と同じ関数)。
fn transformed(text: &str, op: TransformOp) -> String {
    let s = Snapshot::from(text);
    let cs = build_text_transform(
        &s,
        op,
        &[ByteRange::all(s.len_bytes())],
        &CancelToken::new(),
    )
    .expect("作れる")
    .changes;
    applied(text, &cs)
}

// ---------- AC-1: 大文字・小文字 (TXT-1 (a)・TXT-2・TXT-5) ----------

// verifies: AC-FR-214-1
#[test]
fn txt2_選択を大文字_小文字にし_変換した文字列の全体を向きを保って選ぶ() {
    let mut h = open("hello World");
    select(&mut h, &[(0, 5)]);
    assert_eq!(h.cmd(UPPER), CommandResult::Done);
    assert_eq!(h.doc_text(), "HELLO World");
    assert_eq!(ranges(&h), vec![(0, 5)]);
    // 逆向きの選択は逆向きのまま
    select(&mut h, &[(11, 6)]);
    h.cmd(LOWER);
    assert_eq!(h.doc_text(), "HELLO world");
    assert_eq!(ranges(&h), vec![(11, 6)]);
}

// verifies: AC-FR-214-1
#[test]
fn txt2_長さの変わる文字と全角の英字は変わり_漢字とかなは変わらない() {
    let mut h = open("straße ａｂｃ 漢字かな");
    let len = h.doc_text().len();
    select(&mut h, &[(0, len)]);
    h.cmd(UPPER);
    assert_eq!(h.doc_text(), "STRASSE ＡＢＣ 漢字かな");
    assert_eq!(ranges(&h), vec![(0, len)], "変換した文字列の全体を選ぶ");
    h.cmd(LOWER);
    assert_eq!(h.doc_text(), "strasse ａｂｃ 漢字かな");
    // 大文字・小文字の無い文字だけなら本文を変えずに知らせる
    let mut h = open("漢字かな");
    select(&mut h, &[(0, 12)]);
    let rev = revision(&h);
    h.cmd(UPPER);
    assert_eq!(h.doc_text(), "漢字かな");
    assert_eq!(revision(&h), rev, "Transaction を作らない");
    assert_eq!(
        last_message(&h).as_deref(),
        Some("info.transform.unchanged")
    );
}

// verifies: AC-FR-214-1
#[test]
fn txt2_不正バイトのエスケープ文字と結合文字は変わらない() {
    assert_eq!(
        transformed("a\u{10FF61}b\u{10FFE9}", TransformOp::Upper),
        "A\u{10FF61}B\u{10FFE9}"
    );
    assert_eq!(
        transformed("E\u{301}\u{10FF41}", TransformOp::Lower),
        "e\u{301}\u{10FF41}"
    );
}

// verifies: AC-FR-214-1
#[test]
fn txt1_選択が無ければ各カーソルの単語_単語の無いカーソルは飛ばす() {
    // 8 は 2 つの空白の間 (単語が無い)。同じ単語の 2 つのカーソルは 1 回だけ変える
    let mut h = open("foo bar  baz");
    select(&mut h, &[(1, 1), (8, 8), (10, 10), (11, 11)]);
    h.cmd(UPPER);
    assert_eq!(h.doc_text(), "FOO bar  BAZ");
    assert_eq!(ranges(&h), vec![(1, 1), (8, 8), (10, 10), (11, 11)]);
}

// verifies: AC-FR-214-1
#[test]
fn txt1_どのカーソルにも単語が無ければ文書を変えずに知らせる() {
    let mut h = open("a  b");
    select(&mut h, &[(2, 2)]);
    let rev = revision(&h);
    h.cmd(UPPER);
    assert_eq!(h.doc_text(), "a  b");
    assert_eq!(revision(&h), rev);
    assert_eq!(last_message(&h).as_deref(), Some("info.select.no_word"));
}

// verifies: AC-FR-214-1
#[test]
fn txt5_単語のカーソルは単語の始まりからの書記素の数を保つ() {
    // "straß|e": 5 書記素の後ろ。大文字にすると "STRAS|SE"
    let mut h = open("straße x");
    select(&mut h, &[(6, 6)]);
    h.cmd(UPPER);
    assert_eq!(h.doc_text(), "STRASSE x");
    assert_eq!(ranges(&h), vec![(5, 5)]);
    // 結合文字: "é|t" (e + U+0301 の 1 書記素の後ろ)
    let mut h = open("e\u{301}t");
    select(&mut h, &[(3, 3)]);
    h.cmd(UPPER);
    assert_eq!(h.doc_text(), "E\u{301}T");
    assert_eq!(ranges(&h), vec![(3, 3)]);
}

// verifies: AC-FR-214-1
#[test]
fn txt1_空の_range_と空でない_range_が混ざれば空の_range_は飛ばす() {
    let mut h = open("ab cd ef");
    // 主は空のカーソル (cd の中)。空でない Range だけを変える
    select(&mut h, &[(4, 4), (0, 2), (8, 6)]);
    h.cmd(UPPER);
    assert_eq!(h.doc_text(), "AB cd EF");
    let mut got = ranges(&h);
    got.sort_unstable();
    assert_eq!(got, vec![(0, 2), (4, 4), (8, 6)]);
}

// ---------- AC-1: 並べ替え (TXT-1 (b)・TXT-3・TXT-5) ----------

// verifies: AC-FR-214-1
#[test]
fn txt3_文書の全体をコードポイントの順に並べ替え_区切りは動かさない() {
    // 行: b (CRLF), 10 (LF), 9 (CR), B (LF), a (LF)。大小を区別し、数を数として比べない
    let text = "b\r\n10\n9\rB\na\n";
    let mut h = open(text);
    h.cmd(SORT_ASC);
    assert_eq!(h.doc_text(), "10\r\n9\nB\ra\nb\n");
    h.cmd(SORT_DESC);
    assert_eq!(h.doc_text(), "b\r\na\nB\r9\n10\n");
}

// verifies: AC-FR-214-1
#[test]
fn txt3_最後の行の区切りの有無を保ち_空の文書と_1_行は変えない() {
    let mut h = open("b\na");
    h.cmd(SORT_ASC);
    assert_eq!(h.doc_text(), "a\nb", "最後の行に区切りが無いまま");
    let mut h = open("b\na\n");
    h.cmd(SORT_ASC);
    assert_eq!(h.doc_text(), "a\nb\n", "最後の空の行は先頭に来ない");
    for text in ["", "only", "only\n"] {
        let mut h = open(text);
        h.cmd(SORT_ASC);
        assert_eq!(h.doc_text(), text);
        assert_eq!(
            last_message(&h).as_deref(),
            Some("info.transform.unchanged")
        );
    }
}

// verifies: AC-FR-214-1
#[test]
fn txt3_エスケープ文字はコードポイントとして後ろに並び_全角はその前() {
    assert_eq!(
        transformed("\u{10FF41}\nｂ\nb\n", TransformOp::SortAscending),
        "b\nｂ\n\u{10FF41}\n"
    );
}

// verifies: AC-FR-214-1
#[test]
fn txt3_cr_と_lf_がつながる並びになるブロックは並べ替えない() {
    // 行: "" (CR), "x" (LF), "" (LF)。昇順では 2 行目が空になり、CR と LF が 1 つの CRLF になる
    let mut h = open("\rx\n\n");
    h.cmd(SORT_ASC);
    assert_eq!(h.doc_text(), "\rx\n\n");
    assert_eq!(
        last_message(&h).as_deref(),
        Some("info.transform.sort_skipped_cr_lf")
    );
}

// verifies: AC-FR-214-1
#[test]
fn txt3_つながるブロックだけを飛ばし_ほかのブロックは並べて飛ばした旨を付記する() {
    // ブロック 1 は行 0〜2 ("" (CR), "x" (LF), "" (LF))、ブロック 2 は行 4〜5。行 3 は対象の外
    let text = "\rx\n\nz\nb\na\n";
    let mut h = open(text);
    select(&mut h, &[(0, 4), (6, 9)]);
    h.cmd(SORT_ASC);
    assert_eq!(h.doc_text(), "\rx\n\nz\na\nb\n");
    assert_eq!(
        last_message(&h).as_deref(),
        Some("info.transform.sort_skipped_cr_lf")
    );
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), text);
}

// verifies: AC-FR-214-1
#[test]
fn txt1_選んだ行をブロックごとに並べ替え_行頭で終わる選択は最後の行を含めない() {
    // 行 0〜1 (行 2 の行頭で終わる) と、行 4〜5 の 2 つのブロック。行 2〜3 は変えない
    let mut h = open("d\nc\nb\na\nz\ny\n");
    select(&mut h, &[(0, 4), (8, 11)]);
    h.cmd(SORT_ASC);
    assert_eq!(h.doc_text(), "c\nd\nb\na\ny\nz\n");
    assert_eq!(
        ranges(&h),
        vec![(0, 3), (8, 11)],
        "各ブロックの全体を前向きに"
    );
}

// verifies: AC-FR-214-1
#[test]
fn txt1_並べ替えでは空のカーソルを数えず_ブロックの外のカーソルは写す() {
    let mut h = open("b\na\nd\nc\n");
    // 空でない Range は行 0〜1、空のカーソルは行 3
    select(&mut h, &[(7, 7), (0, 3)]);
    h.cmd(SORT_ASC);
    assert_eq!(h.doc_text(), "a\nb\nd\nc\n");
    let mut got = ranges(&h);
    got.sort_unstable();
    assert_eq!(got, vec![(0, 3), (7, 7)]);
}

// verifies: AC-FR-214-1
#[test]
fn txt5_文書の全体を並べ替えたら主のカーソルを同じ行と桁の_1_つのカーソルにする() {
    let mut h = open("bb\naaa\nc\n");
    // 主は行 1 の桁 2 (書記素)。ほかのカーソルは消える
    select(&mut h, &[(5, 5), (0, 0)]);
    h.cmd(SORT_ASC);
    assert_eq!(h.doc_text(), "aaa\nbb\nc\n");
    assert_eq!(ranges(&h), vec![(6, 6)], "行 1 の桁 2");
    // 桁が足りなければ行末
    let mut h = open("bb\nazzz\n");
    select(&mut h, &[(7, 7)]);
    h.cmd(SORT_DESC);
    assert_eq!(h.doc_text(), "bb\nazzz\n", "降順でも変わらない");
    let mut h = open("a\nbbbb\n");
    select(&mut h, &[(6, 6)]);
    h.cmd(SORT_DESC);
    assert_eq!(h.doc_text(), "bbbb\na\n");
    assert_eq!(ranges(&h), vec![(6, 6)], "行 1 の行末");
}

// ---------- AC-1: 末尾空白の除去 (TXT-4・TXT-5) ----------

// verifies: AC-FR-214-1
#[test]
fn txt4_ascii_の空白とタブだけを消し_全角の空白と空白の後の文字は残す() {
    let mut h = open("a  \nb\t\n  \nc \u{3000}\nd");
    // カーソルは消した空白の中 (行 0 の桁 2)
    select(&mut h, &[(2, 2)]);
    h.cmd(TRIM);
    assert_eq!(h.doc_text(), "a\nb\n\nc \u{3000}\nd");
    assert_eq!(
        ranges(&h),
        vec![(1, 1)],
        "消した空白の中のカーソルは行末へ寄る"
    );
}

// verifies: AC-FR-214-1
#[test]
fn txt4_消すと_cr_と_lf_がつながる行は消さず_行の数が変わらない() {
    // a⏎(CR)␠⏎(LF)b
    let mut h = open("a\r \nb  ");
    h.cmd(TRIM);
    assert_eq!(h.doc_text(), "a\r \nb", "真ん中の行は残し、最後の行は消す");
    let mut h = open("a\r \nb");
    h.cmd(TRIM);
    assert_eq!(h.doc_text(), "a\r \nb");
    assert_eq!(
        last_message(&h).as_deref(),
        Some("info.transform.unchanged")
    );
}

// verifies: AC-FR-214-1
#[test]
fn txt4_選んだ行だけを対象にする() {
    let mut h = open("a \nb \nc ");
    select(&mut h, &[(0, 1)]);
    h.cmd(TRIM);
    assert_eq!(h.doc_text(), "a\nb \nc ");
}

// ---------- AC-1: Transaction (TXT-5) ----------

// verifies: AC-FR-214-1
#[test]
fn txt5_どの変換も_1_回の_undo_で本文と選択が戻る() {
    // 行 0〜2 (逆向きの選択。最後の行の行頭で終わる)。どの変換でも変わる並び
    let text = "Bb \nCc \r\nAa \nz";
    for id in ALL_IDS {
        let mut h = open(text);
        select(&mut h, &[(13, 0)]);
        let before = ranges(&h);
        let rev = revision(&h);
        h.cmd(id);
        assert_ne!(h.doc_text(), text, "{id}");
        h.keys("ctrl+z");
        assert_eq!(h.doc_text(), text, "{id}: 本文が戻る");
        assert_eq!(ranges(&h), before, "{id}: 選択が戻る");
        h.keys("ctrl+y");
        assert_ne!(h.doc_text(), text, "{id}: やり直せる");
        assert!(revision(&h) > rev);
    }
}

// verifies: AC-FR-214-1
#[test]
fn txt5_直前の入力と_undo_の単位をつながない() {
    let mut h = open("");
    h.text("abc");
    h.keys("ctrl+a");
    h.cmd(UPPER);
    assert_eq!(h.doc_text(), "ABC");
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "abc", "変換だけが戻る");
}

// ---------- TXT-7: 断る場合 ----------

// verifies: AC-FR-214-1
#[test]
fn txt7_読み取り専用の文書では実行できない() {
    let mut h = open_with("b\na\n", true);
    for id in ALL_IDS {
        assert!(matches!(h.cmd(id), CommandResult::Rejected(_)), "{id}");
    }
    assert_eq!(h.doc_text(), "b\na\n");
}

// verifies: AC-FR-214-1
#[test]
fn txt7_再読込の読込中は実行できない() {
    let mut h = open("b\na\n");
    h.cmd("file.reload");
    assert!(matches!(h.cmd(SORT_ASC), CommandResult::Rejected(_)));
    h.run_jobs();
    assert_eq!(h.doc_text(), "b\na\n");
    h.cmd(SORT_ASC);
    assert_eq!(h.doc_text(), "a\nb\n", "読み終えたら実行できる");
}

// ---------- AC-2: 大きい対象はジョブ (TXT-6) ----------

// verifies: AC-FR-214-2
#[test]
fn txt6_1_mib_を超える対象はジョブで作り_待つ間は変換中と出して入力を受け付ける() {
    let text = "bbbbbbbb\naaaaaaaa\n".repeat(60_000);
    assert!(text.len() > TRANSFORM_SYNC_BYTES);
    let mut h = open(&text);
    h.keys("ctrl+end");
    h.cmd(SORT_ASC);
    assert_eq!(h.doc_text(), text, "ジョブの結果を待つ間は変えない");
    assert_eq!(h.pending_jobs(), 1);
    assert!(h.frame_contains("[変換中]"), "{}", h.frame_text());
    // 待つ間の入力 (文書の末尾の空の行)
    h.text("c");
    assert!(h.doc_text().ends_with("\nc"), "入力を受け付ける");
    h.run_jobs();
    let want = format!(
        "{}{}c",
        "aaaaaaaa\n".repeat(60_000),
        "bbbbbbbb\n".repeat(60_000)
    );
    assert!(h.doc_text() == want, "作り直して適用した");
    assert!(!h.frame_contains("[変換中]"));
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), format!("{text}c"), "1 回の Undo で戻る");
}

// verifies: AC-FR-214-2
#[test]
fn txt6_上限ちょうどは同期_超えればジョブで_結果は同じ() {
    // 文書の全体のブロックの長さは "b\na" の 3 バイト (最後の区切りを数えない)
    let text = "b\na\n";
    let mut sync = open_job(text, 3);
    sync.cmd(SORT_ASC);
    assert_eq!(sync.pending_jobs(), 0, "ちょうどはメインスレッド");
    let mut job = open_job(text, 2);
    job.cmd(SORT_ASC);
    assert_eq!(job.pending_jobs(), 1, "超えればジョブ");
    job.run_jobs();
    assert_eq!(sync.doc_text(), "a\nb\n");
    assert_eq!(job.doc_text(), sync.doc_text());
    assert_eq!(ranges(&job), ranges(&sync));
    // 大文字・小文字は Range の長さの和
    let mut sync = open_job("ab cd", 4);
    select(&mut sync, &[(0, 2), (3, 5)]);
    sync.cmd(UPPER);
    assert_eq!(sync.pending_jobs(), 0);
    let mut job = open_job("ab cd", 3);
    select(&mut job, &[(0, 2), (3, 5)]);
    job.cmd(UPPER);
    assert_eq!(job.pending_jobs(), 1);
    job.run_jobs();
    assert_eq!(job.doc_text(), "AB CD");
    assert_eq!(job.doc_text(), sync.doc_text());
    assert_eq!(ranges(&job), ranges(&sync));
}

// verifies: AC-FR-214-2
#[test]
fn txt6_待つ間の編集は投入したときの範囲を写像して作り直し_今の選択は読まない() {
    let mut h = open_job("hello world", 0);
    select(&mut h, &[(6, 11)]);
    h.cmd(UPPER);
    // 範囲の前に足し、範囲の直後にも足す (端に足した文字は対象に入れない)。選択も動かす
    edit(&mut h, 0, 0, "XX ");
    edit(&mut h, 14, 14, "!");
    select(&mut h, &[(0, 0)]);
    h.run_jobs();
    assert_eq!(h.doc_text(), "XX hello WORLD!");
    assert!(
        h.messages()
            .iter()
            .all(|m| !m.starts_with("msg.transform."))
    );
}

// verifies: AC-FR-214-2
#[test]
fn txt6_文書の全体の作り直しは今の文書の全体() {
    let mut h = open_job("b\na\n", 0);
    h.cmd(SORT_ASC);
    edit(&mut h, 0, 0, "c\n");
    h.run_jobs();
    assert_eq!(h.doc_text(), "a\nb\nc\n");
}

// verifies: AC-FR-214-2
#[test]
fn txt6_範囲が編集で消えたら変換せずに知らせる() {
    let mut h = open_job("hello world", 0);
    select(&mut h, &[(6, 11)]);
    h.cmd(UPPER);
    edit(&mut h, 5, 11, "");
    h.run_jobs();
    assert_eq!(h.doc_text(), "hello");
    assert_eq!(
        last_message(&h).as_deref(),
        Some("msg.transform.target_gone")
    );
    // 行のブロック: 選んだ行を区切りごと消した
    let mut h = open_job("x\nb\na\ny\n", 0);
    select(&mut h, &[(2, 5)]);
    h.cmd(SORT_ASC);
    edit(&mut h, 2, 6, "");
    h.run_jobs();
    assert_eq!(h.doc_text(), "x\ny\n");
    assert_eq!(
        last_message(&h).as_deref(),
        Some("msg.transform.target_gone")
    );
    // 行のブロック: 行の中身を消して区切りだけが残った (消えたとみなす)
    let mut h = open_job("x\nb\na\ny\n", 0);
    select(&mut h, &[(2, 5)]);
    h.cmd(TRIM);
    edit(&mut h, 2, 5, "");
    h.run_jobs();
    assert_eq!(h.doc_text(), "x\n\ny\n");
    assert_eq!(
        last_message(&h).as_deref(),
        Some("msg.transform.target_gone")
    );
}

// verifies: AC-FR-214-2
#[test]
fn txt6_範囲の_1_つでも消えたら全体を諦める() {
    let mut h = open_job("ab cd ef", 0);
    select(&mut h, &[(0, 2), (6, 8)]);
    h.cmd(UPPER);
    edit(&mut h, 6, 8, "");
    h.run_jobs();
    assert_eq!(h.doc_text(), "ab cd ", "残った範囲も変えない");
    assert_eq!(
        last_message(&h).as_deref(),
        Some("msg.transform.target_gone")
    );
}

// verifies: AC-FR-214-2
#[test]
fn txt6_待つ間に再読込を始めたら変換を取り消し_読み直した本文を変えない() {
    for whole in [true, false] {
        let mut h = open_job("b\na\n", 0);
        if !whole {
            select(&mut h, &[(0, 3)]);
        }
        h.cmd(SORT_ASC);
        h.fs.add_file(PATH, b"d\nc\n");
        h.cmd("file.reload");
        assert_eq!(
            last_message(&h).as_deref(),
            Some("msg.transform.target_gone"),
            "{whole}"
        );
        assert!(!h.frame_contains("[変換中]"));
        // 読込が先に終わっても、取り消した変換は当てない
        h.run_jobs();
        assert_eq!(h.doc_text(), "d\nc\n", "{whole}");
    }
}

// verifies: AC-FR-214-2
#[test]
fn txt6_待つ間の_undo_では今の文書の全体で作り直す() {
    let mut h = open_job("b\na\n", 0);
    h.cmd(SORT_ASC);
    edit(&mut h, 0, 0, "c\n");
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "b\na\n");
    h.run_jobs();
    assert_eq!(h.doc_text(), "a\nb\n");
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "b\na\n", "変換だけが 1 回の Undo で戻る");
}

// verifies: AC-FR-214-2
#[test]
fn txt6_待つ間に選択を広げたら_広げた選択を写す() {
    let mut h = open_job("hello world!", 0);
    select(&mut h, &[(6, 11)]);
    h.cmd(UPPER);
    h.keys("shift+right");
    assert_eq!(ranges(&h), vec![(6, 12)]);
    h.run_jobs();
    assert_eq!(h.doc_text(), "hello WORLD!");
    assert_eq!(
        ranges(&h),
        vec![(6, 12)],
        "始まりは前寄り・終わりは後ろ寄り"
    );
}

// verifies: AC-FR-214-2
#[test]
fn txt6_保存中と変換中が重なれば保存中を出す() {
    let mut h = open_job("b\na\n", 0);
    h.text("z");
    h.keys("ctrl+s");
    h.cmd(SORT_ASC);
    assert!(h.frame_contains("[保存中]"), "{}", h.frame_text());
    assert!(!h.frame_contains("[変換中]"));
    h.run_jobs();
    assert_eq!(h.doc_text(), "a\nzb\n");
}

// verifies: AC-FR-214-2
#[test]
fn txt6_作り直しも古ければ適用せずに知らせる() {
    let mut h = open_job("b\na\n", 0);
    h.cmd(SORT_ASC);
    edit(&mut h, 0, 0, "z");
    // 最初の結果を受けた直後 (作り直しのジョブが走る前) にもう一度変える
    h.run_jobs_once();
    edit(&mut h, 0, 0, "z");
    h.run_jobs();
    assert_eq!(h.doc_text(), "zzb\na\n", "変換していない");
    assert_eq!(last_message(&h).as_deref(), Some("msg.transform.stale"));
}

// verifies: AC-FR-214-2
#[test]
fn txt6_ジョブが失敗したら文書を変えずに知らせる() {
    let errors = || {
        [
            JobError::OutOfMemory { requested: 1 << 30 },
            JobError::Panic,
        ]
    };
    for (i, err) in errors().into_iter().enumerate() {
        let mut h = open_job("b\na\n", 0);
        h.cmd(SORT_ASC);
        let mut held = h.run_one_job_held();
        assert_eq!(held.len(), 1);
        held[0].body = JobResult::Failed { err };
        let err = &errors()[i];
        h.deliver(held);
        assert_eq!(h.doc_text(), "b\na\n");
        assert_eq!(
            last_message(&h).as_deref(),
            Some("msg.transform.failed"),
            "{err:?}"
        );
        assert!(!h.frame_contains("[変換中]"));
    }
}

// verifies: AC-FR-214-2
#[test]
fn txt6_同じ文書に新しい変換を頼んだら前のジョブを取り消す() {
    let mut h = open_job("Ab", 0);
    select(&mut h, &[(0, 2)]);
    h.cmd(UPPER);
    h.cmd(LOWER);
    h.run_jobs();
    assert_eq!(h.doc_text(), "ab", "後の変換だけを当てる");
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "Ab", "当てたのは 1 回");
}

// verifies: AC-FR-214-2
#[test]
fn txt6_待つ間に読み取り専用にした文書には当てない() {
    let mut h = open_job("b\na\n", 0);
    h.cmd(SORT_ASC);
    h.cmd("file.toggle_read_only");
    let d = h.editor.state().document(h.doc_id()).expect("文書");
    assert!(!d.is_editable());
    h.run_jobs();
    assert_eq!(h.doc_text(), "b\na\n");
    assert!(
        last_message(&h).is_some_and(|m| m.starts_with("msg.edit.read_only")),
        "{:?}",
        h.messages()
    );
}

// verifies: AC-FR-214-2
#[test]
fn txt6_前に出ていない文書が読み取り専用になっていても読み取り専用と知らせる() {
    let mut h = Harness::builder()
        .file(PATH, b"b\na\n")
        .file("/w/other.txt", b"x\n")
        .build();
    h.open(PATH);
    h.run_jobs();
    let a = h.doc_id();
    h.editor.state_mut().set_transform_sync_bytes(0);
    h.cmd(SORT_ASC);
    h.cmd("file.toggle_read_only");
    // 変換の結果を取り置き、ほかの文書を前に出してから渡す
    let held = h.run_one_job_held();
    h.open("/w/other.txt");
    h.run_jobs();
    assert_ne!(h.doc_id(), a);
    h.clear_log();
    h.deliver(held);
    assert_eq!(text_of(&h, a), "b\na\n");
    assert!(
        last_message(&h).is_some_and(|m| m.starts_with("msg.edit.read_only")),
        "{:?}",
        h.messages()
    );
}

// verifies: AC-FR-214-2
#[test]
fn txt6_待つ間にペインを閉じても文書に当てる() {
    let mut h = open_job("b\na\n", 0);
    h.cmd("pane.split_right");
    h.cmd(SORT_ASC);
    h.cmd("pane.close");
    h.run_jobs();
    assert_eq!(h.doc_text(), "a\nb\n");
}

// verifies: AC-FR-214-2
#[test]
fn txt6_文書を閉じたらジョブの結果を捨てる() {
    let mut h = open_job("b\na\n", 0);
    h.cmd(SORT_ASC);
    h.cmd("buffer.close");
    h.run_jobs();
    assert!(
        h.messages()
            .iter()
            .all(|m| !m.starts_with("msg.transform."))
    );
}

/// 監視 (登録にすぐ `Started` を返す) と `/w/a.txt` を開いた Harness。
fn watched(text: &str) -> (Harness, FakeWatcher) {
    let fake = FakeWatcher::new().auto_started();
    let mut h = Harness::builder()
        .watcher(fake.clone())
        .file(PATH, text.as_bytes())
        .build();
    h.editor.state_mut().set_config(Config {
        watch_enabled: true,
        reload_on_external_change: ExternalChangePolicy::AutoIfUnmodified,
        ..Config::default()
    });
    h.open(PATH);
    h.run_jobs();
    h.pump_fs();
    (h, fake)
}

fn text_of(h: &Harness, doc: DocumentId) -> String {
    h.editor
        .state()
        .document(doc)
        .expect("文書")
        .to_string_lossy()
}

// verifies: AC-FR-214-2
#[test]
fn txt6_変換のジョブの間は監視の照合と読み直しを延ばす() {
    let (mut h, fake) = watched("b\na\n");
    let d = h.doc_id();
    h.editor.state_mut().set_transform_sync_bytes(0);
    h.cmd(SORT_ASC);
    assert_eq!(h.pending_jobs(), 1, "変換のジョブ");
    h.fs.add_file(PATH, b"changed\n");
    fake.changed(fake.id_of("/w").expect("監視している"), "a.txt");
    h.pump_fs();
    h.advance_ms(WATCH_SETTLE.0);
    assert_eq!(h.pending_jobs(), 1, "変換の間は照合を積まない");
    h.run_jobs_once();
    assert_eq!(text_of(&h, d), "a\nb\n", "変換を当てる");
    h.run_jobs();
    assert!(
        h.editor
            .state()
            .document(d)
            .expect("文書")
            .external_pending
            .is_some(),
        "変換の後に照合して、編集した文書には印を付ける (読み直さない)"
    );
    assert_eq!(text_of(&h, d), "a\nb\n");
}

// ---------- proptest (03 §15.9.1) ----------

/// 行の中身に使う文字 (大小・数・全角・結合文字・空白・タブ・全角の空白・エスケープ文字 (空白の 0x20 のものを含む))。
const CHARS: &[char] = &[
    'a',
    'b',
    'B',
    '1',
    '9',
    ' ',
    '\t',
    '\u{3000}',
    'é',
    'e',
    '\u{301}',
    'ｂ',
    '漢',
    '\u{10FF41}',
    '\u{10FF20}',
];

/// 任意の文書: 行の中身と区切り (LF・CRLF・CR) の列。最後の行の区切りは有っても無くてもよい。
fn doc_strategy() -> impl Strategy<Value = String> {
    let line = (
        prop::collection::vec(prop::sample::select(CHARS), 0..5),
        prop::sample::select(&["\n", "\r\n", "\r"][..]),
    );
    (prop::collection::vec(line, 0..8), any::<bool>()).prop_map(|(lines, last_eol)| {
        let mut s = String::new();
        let n = lines.len();
        for (i, (chars, eol)) in lines.into_iter().enumerate() {
            s.extend(chars);
            if i + 1 < n || last_eol {
                s.push_str(eol);
            }
        }
        s
    })
}

/// 本文を行 (中身, 区切り) に分ける (03 §2.4: `\r\n` は 1 つの区切り)。最後の行の区切りは空。
fn split_lines(s: &str) -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\r' if it.peek() == Some(&'\n') => {
                it.next();
                out.push((std::mem::take(&mut cur), "\r\n"));
            }
            '\r' => out.push((std::mem::take(&mut cur), "\r")),
            '\n' => out.push((std::mem::take(&mut cur), "\n")),
            _ => cur.push(c),
        }
    }
    out.push((cur, ""));
    out
}

fn join_lines(lines: &[(String, &str)]) -> String {
    lines.iter().map(|(c, t)| format!("{c}{t}")).collect()
}

/// 文書の全体のブロックの行の数 (最後の区切りの後ろの空の行を除く)。
fn block_len(lines: &[(String, &str)]) -> usize {
    if lines.len() > 1 && lines.last().is_some_and(|(c, _)| c.is_empty()) {
        lines.len() - 1
    } else {
        lines.len()
    }
}

/// 行 `i` の中身が空になると、前の行の単独の CR とこの行の単独の LF がつながる。
fn joins(lines: &[(String, &str)], i: usize) -> bool {
    i > 0 && lines[i - 1].1 == "\r" && lines[i].1 == "\n"
}

/// 並べ替えの参照モデル (TXT-3)。
fn sort_reference(text: &str, descending: bool) -> String {
    let lines = split_lines(text);
    let n = block_len(&lines);
    let mut contents: Vec<String> = lines[..n].iter().map(|(c, _)| c.clone()).collect();
    // 安定な並べ替え (等しい行は元の順)
    if descending {
        contents.sort_by(|a, b| b.cmp(a));
    } else {
        contents.sort();
    }
    let mut out = lines.clone();
    for (i, c) in contents.into_iter().enumerate() {
        out[i].0 = c;
    }
    if (0..n).any(|i| out[i].0.is_empty() && joins(&lines, i)) {
        return text.to_owned();
    }
    join_lines(&out)
}

/// 末尾空白の除去の参照モデル (TXT-4・SVT-2)。
fn trim_reference(text: &str) -> String {
    let lines = split_lines(text);
    let out: Vec<(String, &str)> = lines
        .iter()
        .enumerate()
        .map(|(i, (c, t))| {
            let trimmed = c.trim_end_matches([' ', '\t']);
            if trimmed.is_empty() && !c.is_empty() && joins(&lines, i) {
                (c.clone(), *t)
            } else {
                (trimmed.to_owned(), *t)
            }
        })
        .collect();
    join_lines(&out)
}

/// 選択から作るブロック (TXT-1 (b)) の参照モデル。`which` は 0 = 昇順、1 = 降順、2 = 末尾空白の除去。
fn block_reference(text: &str, sel: &[(usize, usize)], which: usize) -> String {
    let lines = split_lines(text);
    // 各行の始まりのバイト位置
    let mut starts = Vec::new();
    let mut pos = 0;
    for (c, t) in &lines {
        starts.push(pos);
        pos += c.len() + t.len();
    }
    let line_of = |p: usize| starts.iter().rposition(|&s| s <= p).unwrap_or(0);
    let n = block_len(&lines);
    let mut blocks: Vec<(usize, usize)> = sel
        .iter()
        .filter(|(a, b)| a != b)
        .filter_map(|&(a, b)| {
            let (s, e) = (a.min(b), a.max(b));
            let first = line_of(s);
            let mut last = line_of(e);
            if last > first && starts[last] == e {
                last -= 1;
            }
            // 最後の区切りの後ろの空の行は含めない
            if last >= n {
                if first >= n {
                    return None;
                }
                last = n - 1;
            }
            Some((first, last))
        })
        .collect();
    if sel.iter().all(|(a, b)| a == b) {
        blocks = vec![(0, n - 1)];
    }
    blocks.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (a, b) in blocks {
        match merged.last_mut() {
            Some(p) if a <= p.1 + 1 => p.1 = p.1.max(b),
            _ => merged.push((a, b)),
        }
    }
    let mut out = lines.clone();
    for (first, last) in merged {
        if which == 2 {
            for i in first..=last {
                let c = &lines[i].0;
                let trimmed = c.trim_end_matches([' ', '\t']);
                if !(trimmed.is_empty() && !c.is_empty() && joins(&lines, i)) {
                    out[i].0 = trimmed.to_owned();
                }
            }
            continue;
        }
        let mut contents: Vec<String> =
            lines[first..=last].iter().map(|(c, _)| c.clone()).collect();
        if which == 1 {
            contents.sort_by(|a, b| b.cmp(a));
        } else {
            contents.sort();
        }
        if contents
            .iter()
            .enumerate()
            .any(|(k, c)| c.is_empty() && joins(&lines, first + k))
        {
            continue;
        }
        for (k, c) in contents.into_iter().enumerate() {
            out[first + k].0 = c;
        }
    }
    join_lines(&out)
}

fn whole_cs(text: &str, op: TransformOp) -> ChangeSet {
    let s = Snapshot::from(text);
    build_text_transform(
        &s,
        op,
        &[ByteRange::all(s.len_bytes())],
        &CancelToken::new(),
    )
    .expect("作れる")
    .changes
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// (a) 並べ替え: 参照モデルと一致し、行の中身の多重集合と区切りの並びが元と同じ。昇順・降順とも隣り合う行が順に並ぶ
    /// (CR と LF がつながるブロックは変えない)。
    #[test]
    fn prop_a_並べ替えは中身の多重集合と区切りの並びを保ち_安定に並ぶ(text in doc_strategy()) {
        for (op, desc) in [(TransformOp::SortAscending, false), (TransformOp::SortDescending, true)] {
            let got = applied(&text, &whole_cs(&text, op));
            prop_assert_eq!(&got, &sort_reference(&text, desc));
            let (before, after) = (split_lines(&text), split_lines(&got));
            prop_assert_eq!(before.len(), after.len(), "行の数");
            let terms = |l: &[(String, &str)]| l.iter().map(|(_, t)| t.to_string()).collect::<Vec<_>>();
            prop_assert_eq!(terms(&before), terms(&after), "区切りの並び");
            let mut a: Vec<String> = before.iter().map(|(c, _)| c.clone()).collect();
            let mut b: Vec<String> = after.iter().map(|(c, _)| c.clone()).collect();
            a.sort();
            b.sort();
            prop_assert_eq!(a, b, "中身の多重集合");
            if got != text {
                let n = block_len(&after);
                for w in after[..n].windows(2) {
                    if desc {
                        prop_assert!(w[0].0 >= w[1].0);
                    } else {
                        prop_assert!(w[0].0 <= w[1].0);
                    }
                }
            }
        }
    }

    /// (b) 末尾空白の除去: 参照モデルと一致し、行の数が変わらず、消す範囲だけの `ChangeSet` で、2 回目は何も変えない。
    #[test]
    fn prop_b_末尾空白の除去は行の数を変えず_消す範囲だけで冪等(text in doc_strategy()) {
        let cs = whole_cs(&text, TransformOp::TrimTrailing);
        prop_assert!(
            cs.ops().iter().all(|op| !matches!(op, Op::Insert(s) if !s.is_empty())),
            "挿入を含まない"
        );
        let got = applied(&text, &cs);
        prop_assert_eq!(&got, &trim_reference(&text));
        let (before, after) = (split_lines(&text), split_lines(&got));
        prop_assert_eq!(before.len(), after.len(), "行の数");
        for (i, (c, _)) in after.iter().enumerate() {
            let kept = c.ends_with([' ', '\t']);
            prop_assert!(!kept || (c.trim_end_matches([' ', '\t']).is_empty() && joins(&after, i)), "行 {}: {:?}", i, c);
        }
        prop_assert!(whole_cs(&got, TransformOp::TrimTrailing).is_identity(), "冪等");
    }

    /// (e) ジョブで作った結果 (本文・選択・Undo の後の本文) が、メインスレッドで作ったものと同じ。
    #[test]
    fn prop_e_ジョブと同期の結果が同じ(
        text in doc_strategy().prop_filter("エスケープ文字は外から開けない", |t| !t.contains(['\u{10FF41}', '\u{10FF20}'])),
        picks in prop::collection::vec((any::<u16>(), any::<u16>()), 1..4),
        op in 0usize..5,
    ) {
        let id = ALL_IDS[op];
        let len = text.len();
        let at = |p: u16| {
            // 書記素の境界に丸める (Selection の正規化に任せず、両方に同じ位置を置く)
            let mut p = usize::from(p) % (len + 1);
            while !text.is_char_boundary(p) {
                p -= 1;
            }
            p
        };
        let sel: Vec<(usize, usize)> = picks.iter().map(|&(a, b)| (at(a), at(b))).collect();
        let mut sync = open(&text);
        let mut job = open_job(&text, 0);
        select(&mut sync, &sel);
        select(&mut job, &sel);
        sync.cmd(id);
        job.cmd(id);
        job.run_jobs();
        prop_assert_eq!(job.doc_text(), sync.doc_text());
        prop_assert_eq!(ranges(&job), ranges(&sync));
        prop_assert_eq!(job.messages(), sync.messages());
        sync.keys("ctrl+z");
        job.keys("ctrl+z");
        prop_assert_eq!(job.doc_text(), sync.doc_text());
        prop_assert_eq!(sync.doc_text(), text);
    }

    /// 選択から作るブロック (TXT-1 (b)): 任意の選択で並べ替え・末尾空白の除去をした本文が、参照モデル「空でない Range が
    /// 触れる行 (行頭で終わる選択の最後の行と、最後の区切りの後ろの空の行を除く) を隣り合う行ごとにまとめ、ブロックごとに
    /// 変換する (空でない Range が無ければ文書の全体)」と一致する。
    #[test]
    fn prop_選択から作るブロックごとの変換が参照モデルと一致する(
        text in doc_strategy().prop_filter("外から開けない文字・書記素の途中の位置を避ける", |t| {
            !t.contains(['\u{10FF41}', '\u{10FF20}', '\u{301}'])
        }),
        picks in prop::collection::vec((any::<u16>(), any::<u16>()), 1..4),
        op in 0usize..3,
    ) {
        let (id, which) = [(SORT_ASC, 0), (SORT_DESC, 1), (TRIM, 2)][op];
        let len = text.len();
        let at = |p: u16| {
            let mut p = usize::from(p) % (len + 1);
            while !text.is_char_boundary(p) {
                p -= 1;
            }
            // CRLF の間には置けない (Selection の正規化と同じ位置にする)
            if p > 0 && text.as_bytes()[p - 1] == b'\r' && text.as_bytes().get(p) == Some(&b'\n') {
                p -= 1;
            }
            p
        };
        let sel: Vec<(usize, usize)> = picks.iter().map(|&(a, b)| (at(a), at(b))).collect();
        let mut h = open(&text);
        select(&mut h, &sel);
        h.cmd(id);
        prop_assert_eq!(h.doc_text(), block_reference(&text, &sel, which));
    }
}
