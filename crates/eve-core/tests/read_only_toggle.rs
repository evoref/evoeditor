//! 読み取り専用の切替 `file.toggle_read_only` と「読み取り専用を解除して保存」(FR-116 / FR-106、docs/03 §9.6・§9.3 手順 1)。

use eve_core::{CommandResult, ReadOnlyReason};
use eve_testkit::Harness;

fn reasons(h: &Harness) -> Vec<ReadOnlyReason> {
    let id = h.doc_id();
    h.editor
        .state()
        .document(id)
        .unwrap()
        .read_only
        .iter()
        .collect()
}

fn readonly_attr(h: &Harness, path: &str) -> bool {
    eve_core::FileSystem::metadata(h.fs.as_ref(), &path.into())
        .unwrap()
        .readonly
}

// verifies: AC-FR-116-2, FR-106
#[test]
fn 読み取り専用属性を解除して編集し_確認を経て保存すると属性は保存前のまま() {
    let mut h = Harness::new().with_file("/w/ro.txt", b"abc\n");
    h.fs.set_readonly("/w/ro.txt", true);
    h.open("/w/ro.txt");
    h.run_jobs();
    assert_eq!(reasons(&h), [ReadOnlyReason::FileAttribute]);
    assert_eq!(h.cmd("file.toggle_read_only"), CommandResult::Done);
    assert!(reasons(&h).is_empty());
    assert!(h.messages().contains(&"info.read_only.released".to_owned()));
    h.text("x");
    assert_eq!(h.doc_text(), "xabc\n");
    // 上書き保存は確認を経る。既定は別名で保存
    h.keys("ctrl+s");
    assert_eq!(h.overlay(), "dialog");
    h.keys("w");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_eq!(h.fs_read("/w/ro.txt").unwrap(), b"xabc\n");
    assert!(readonly_attr(&h, "/w/ro.txt"), "属性は保存前と同じ");
    assert!(!h.is_dirty());
    // 編集したら、次の保存でも確認する
    h.text("y");
    h.keys("ctrl+s");
    assert_eq!(h.overlay(), "dialog");
}

// verifies: AC-FR-116-2
#[test]
fn 確認で別名で保存を選ぶと名前の入力になり_元のファイルは変わらない() {
    let mut h = Harness::new().with_file("/w/ro.txt", b"abc\n");
    h.fs.set_readonly("/w/ro.txt", true);
    h.open("/w/ro.txt");
    h.run_jobs();
    h.cmd("file.toggle_read_only");
    h.text("x");
    h.keys("ctrl+s");
    h.keys("enter");
    assert_eq!(h.overlay(), "prompt");
    h.keys("esc");
    h.run_jobs();
    assert_eq!(h.fs_read("/w/ro.txt").unwrap(), b"abc\n");
}

// verifies: AC-FR-116-2
#[test]
fn readonly_で開いた文書は解除でき_確認なしで保存する_もう一度で読み取り専用に戻る() {
    let mut h = Harness::new().with_file("/w/a.txt", b"abc\n");
    h.editor.state_mut().open_path(
        "/w/a.txt",
        eve_core::OpenOptions {
            user_read_only: true,
            ..Default::default()
        },
    );
    h.run_jobs();
    assert_eq!(reasons(&h), [ReadOnlyReason::UserRequested]);
    h.cmd("file.toggle_read_only");
    h.text("x");
    h.keys("ctrl+s");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_eq!(h.fs_read("/w/a.txt").unwrap(), b"xabc\n");
    // 編集できる文書では `UserRequested` を付ける
    h.cmd("file.toggle_read_only");
    assert_eq!(reasons(&h), [ReadOnlyReason::UserRequested]);
    assert!(h.messages().contains(&"info.read_only.set".to_owned()));
    h.text("y");
    assert_eq!(h.doc_text(), "xabc\n");
}

// verifies: AC-FR-116-2
#[test]
fn 解除できない理由ではコマンドが実行できない() {
    // 長大行 (理由 LongLine) は解除できない
    let line = "x".repeat((1 << 20) + 10);
    let mut h = Harness::new().with_file("/w/long.txt", line.as_bytes());
    h.open("/w/long.txt");
    h.run_jobs();
    assert_eq!(reasons(&h), [ReadOnlyReason::LongLine]);
    assert!(!matches!(
        h.cmd("file.toggle_read_only"),
        CommandResult::Done
    ));
    assert_eq!(reasons(&h), [ReadOnlyReason::LongLine]);
}
