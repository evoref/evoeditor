//! ダングリングのシンボリックリンクの保存 (FR-106、docs/05 §4.1・03 §9.7 (e)): 確認のうえリンク先に新規作成する。

use eve_testkit::Harness;

fn opened_link() -> Harness {
    let mut h = Harness::new();
    h.fs.add_link("/w/link.txt", "/w/real.txt");
    h.open("/w/link.txt");
    h.run_jobs();
    h.text("x");
    h
}

// verifies: FR-106
#[test]
fn ダングリングリンクの保存は確認のうえリンク先に作る() {
    let mut h = opened_link();
    h.keys("ctrl+s");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog", "リンク先に作るかを尋ねる");
    assert!(h.fs_read("/w/real.txt").is_none(), "確認の前は何も書かない");
    h.keys("c");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_eq!(h.fs_read("/w/real.txt").unwrap(), b"x");
    assert!(!h.is_dirty());
    // 次の保存はリンク先が在るので、確認なしでリンク先に書く
    h.text("y");
    h.keys("ctrl+s");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_eq!(h.fs_read("/w/real.txt").unwrap(), b"xy");
}

// verifies: FR-106
#[test]
fn キャンセルすると何も書かない() {
    let mut h = opened_link();
    h.keys("ctrl+s");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog");
    h.keys("esc");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert!(h.fs_read("/w/real.txt").is_none());
    assert!(h.is_dirty());
    // 承認は 1 回きり: もう一度保存しても、また尋ねる
    h.keys("ctrl+s");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog");
}

// verifies: FR-106
#[test]
fn リンク先の親が無ければ作らずに失敗を示す() {
    let mut h = Harness::new();
    h.fs.add_link("/w/link.txt", "/nodir/real.txt");
    h.open("/w/link.txt");
    h.run_jobs();
    h.text("x");
    h.keys("ctrl+s");
    h.run_jobs();
    h.keys("c");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog", "保存の失敗の選択");
    assert!(h.fs_read("/nodir/real.txt").is_none());
    assert!(h.is_dirty());
}
