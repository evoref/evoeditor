//! 外部変更の確認 (FR-117、docs/03 §9.5 の「M9d の実装」): フォーカス復帰と `file.check_external_changes`。

use eve_config::{Config, ExternalChangePolicy};
use eve_testkit::Harness;

fn opened() -> Harness {
    let mut h = Harness::new().with_file("/w/a.txt", b"abc\n");
    h.open("/w/a.txt");
    h.run_jobs();
    h
}

fn focus(h: &mut Harness) {
    h.focus(false);
    h.focus(true);
    h.run_jobs();
}

fn policy(h: &mut Harness, p: ExternalChangePolicy) {
    h.editor.state_mut().set_config(Config {
        reload_on_external_change: p,
        ..Config::default()
    });
}

// verifies: FR-117
#[test]
fn 未編集なら変更を確認なしで読み直す() {
    let mut h = opened();
    h.fs.add_file("/w/a.txt", b"changed\n");
    focus(&mut h);
    assert_eq!(h.overlay(), "");
    assert_eq!(h.doc_text(), "changed\n");
    assert!(!h.is_dirty());
    // Undo で読み直す前に戻せる
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "abc\n");
}

// verifies: FR-117
#[test]
fn 変わっていなければ何もしない() {
    let mut h = opened();
    h.clear_log();
    focus(&mut h);
    assert_eq!(h.overlay(), "");
    assert!(h.messages().is_empty(), "{:?}", h.messages());
    // コマンドでは変わっていないことを知らせる
    h.cmd("file.check_external_changes");
    h.run_jobs();
    assert!(h.messages().contains(&"info.external.unchanged".to_owned()));
}

// verifies: FR-117
#[test]
fn 編集中なら尋ね_無視した変更はフォーカス復帰で再び尋ねない() {
    let mut h = opened();
    h.text("x");
    h.fs.add_file("/w/a.txt", b"changed\n");
    focus(&mut h);
    assert_eq!(h.overlay(), "dialog");
    // 既定は無視
    h.keys("enter");
    assert_eq!(h.overlay(), "");
    assert_eq!(h.doc_text(), "xabc\n");
    focus(&mut h);
    assert_eq!(h.overlay(), "", "同じ変更では尋ねない");
    // コマンドでは尋ねる
    h.cmd("file.check_external_changes");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog");
    h.keys("esc");
    // 新しい変更ではまた尋ね、再読込を選ぶと自分の変更を捨てて読み直す
    h.fs.add_file("/w/a.txt", b"again!\n");
    focus(&mut h);
    assert_eq!(h.overlay(), "dialog");
    h.keys("l");
    h.run_jobs();
    assert_eq!(h.doc_text(), "again!\n");
    assert!(!h.is_dirty());
}

// verifies: FR-117
#[test]
fn 削除されたら新規として保持するか閉じるかを尋ねる() {
    let mut h = opened();
    h.fs.remove(&"/w/a.txt".into());
    focus(&mut h);
    assert_eq!(h.overlay(), "dialog");
    h.keys("k");
    assert_eq!(h.overlay(), "");
    assert_eq!(h.doc_text(), "abc\n", "内容は保つ");
    focus(&mut h);
    assert_eq!(h.overlay(), "", "同じ状態では尋ねない");
    // 閉じる: 未編集なのでそのまま閉じる
    h.cmd("file.check_external_changes");
    h.run_jobs();
    h.keys("c");
    assert_ne!(h.doc_path().as_deref(), Some("/w/a.txt"));
}

// verifies: FR-117
#[test]
fn 設定_ask_なら未編集でも尋ね_never_ならフォーカス復帰では確かめない() {
    let mut h = opened();
    policy(&mut h, ExternalChangePolicy::Ask);
    h.fs.add_file("/w/a.txt", b"changed\n");
    focus(&mut h);
    assert_eq!(h.overlay(), "dialog");
    h.keys("esc");
    policy(&mut h, ExternalChangePolicy::Never);
    focus(&mut h);
    assert_eq!(h.overlay(), "");
    assert_eq!(h.doc_text(), "abc\n");
    // コマンドは設定に関わらず確かめる
    h.cmd("file.check_external_changes");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog");
}

// verifies: FR-117
#[test]
fn 確認などが出ている間は確かめない() {
    let mut h = opened();
    h.fs.add_file("/w/a.txt", b"changed\n");
    h.cmd("ui.palette_open");
    focus(&mut h);
    assert_eq!(h.overlay(), "palette");
    assert_eq!(h.doc_text(), "abc\n");
}
