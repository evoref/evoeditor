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
fn 削除されたら尋ね_新規として保持すると次の保存で確認なしに作り直す() {
    let mut h = opened();
    h.fs.remove(&"/w/a.txt".into());
    focus(&mut h);
    assert_eq!(h.overlay(), "dialog");
    h.keys("k");
    assert_eq!(h.overlay(), "");
    assert_eq!(
        h.doc_text(),
        "abc
",
        "内容は保つ"
    );
    focus(&mut h);
    assert_eq!(h.overlay(), "", "新規ファイル扱いなので再び尋ねない");
    // 未編集でも、保存はファイルを作り直す (保存時の照合の確認は出ない)
    h.keys("ctrl+s");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_eq!(
        h.fs_read("/w/a.txt").unwrap(),
        b"abc
"
    );
    // 編集して保存しても確認は出ない
    h.text("x");
    h.keys("ctrl+s");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_eq!(
        h.fs_read("/w/a.txt").unwrap(),
        b"xabc
"
    );
}

// verifies: FR-117
#[test]
fn 削除されて閉じるを選ぶと_未編集なら閉じ_編集中なら未保存の確認を出す() {
    let mut h = opened();
    h.fs.remove(&"/w/a.txt".into());
    focus(&mut h);
    h.keys("c");
    assert_eq!(h.overlay(), "");
    assert_ne!(h.doc_path().as_deref(), Some("/w/a.txt"));

    let mut h = opened();
    h.text("x");
    h.fs.remove(&"/w/a.txt".into());
    focus(&mut h);
    h.keys("c");
    assert_eq!(h.overlay(), "dialog", "未保存の確認 (FR-107)");
    assert_eq!(
        h.doc_path().as_deref(),
        Some("/w/a.txt"),
        "答えるまで閉じない"
    );
    // キャンセルすれば内容は残る
    h.keys("esc");
    assert_eq!(h.overlay(), "");
    assert_eq!(
        h.doc_text(),
        "xabc
"
    );
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

fn has_external_status(h: &mut Harness) -> bool {
    h.frame()
        .status
        .items
        .iter()
        .any(|i| i.kind == eve_view::StatusKind::ExternalChange)
}

// verifies: FR-117
#[test]
fn 無視した外部変更はステータスに出し_保存か再読込で消す() {
    let mut h = opened();
    h.text("x");
    assert!(!has_external_status(&mut h));
    h.fs.add_file("/w/a.txt", b"changed\n");
    focus(&mut h);
    // 尋ねている間は出さない (まだ選んでいない)
    assert_eq!(h.overlay(), "dialog");
    assert!(!has_external_status(&mut h));
    h.keys("enter");
    assert!(has_external_status(&mut h));
    assert!(h.frame_contains("[外部で変更]"), "{}", h.frame_text());
    // 上書き保存 (保存時の照合の確認で上書き) で消える
    h.keys("ctrl+s");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog");
    h.keys("o");
    h.run_jobs();
    assert!(!h.is_dirty());
    assert!(!has_external_status(&mut h));

    // 再読込でも消える
    h.text("y");
    h.fs.add_file("/w/a.txt", b"again!\n");
    focus(&mut h);
    h.keys("enter");
    assert!(has_external_status(&mut h));
    h.cmd("file.reload");
    // 未保存の変更を捨てる確認
    assert_eq!(h.overlay(), "dialog");
    h.keys("l");
    h.run_jobs();
    assert_eq!(h.doc_text(), "again!\n");
    assert!(!has_external_status(&mut h));
}

// verifies: FR-117
#[test]
fn 未編集で読み直したときと_削除を新規として保持したときはステータスに出さない() {
    let mut h = opened();
    h.fs.add_file("/w/a.txt", b"changed\n");
    focus(&mut h);
    assert_eq!(h.doc_text(), "changed\n");
    assert!(!has_external_status(&mut h));
    h.fs.remove(&"/w/a.txt".into());
    focus(&mut h);
    assert_eq!(h.overlay(), "dialog");
    h.keys("k");
    assert!(!has_external_status(&mut h));
}
