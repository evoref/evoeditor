//! `Harness` 自身の語彙の検査 (docs/09 §3.4 の例を含む)。M1 の AC のシナリオは D2b。

use eve_testkit::Harness;

// verifies: AC-NFR-701-1
#[test]
fn 例_開いて選択を消して保存する() {
    let mut h = Harness::new().with_file("/w/a.txt", b"hello\r\nworld");
    h.open("/w/a.txt");
    h.run_jobs();
    assert_eq!(h.doc_text(), "hello\r\nworld");
    h.keys("ctrl+end shift+home");
    h.cmd("edit.delete_backward");
    h.advance_ms(3000);
    h.cmd("file.save");
    assert_eq!(h.pending_jobs(), 1, "保存は run_jobs まで進まない");
    h.run_jobs();
    assert_eq!(h.fs_read("/w/a.txt").unwrap(), b"hello\r\n");
    assert!(!h.is_dirty());
    assert_eq!(h.doc_path().as_deref(), Some("/w/a.txt"));
}

#[test]
fn 入力と_frame_の_ascii() {
    let mut h = Harness::new().with_size(20, 5);
    h.text("ab");
    h.keys("enter");
    h.paste("日本");
    let f = h.frame_text();
    assert!(f.starts_with("ab\n日本\n"), "{f}");
    assert!(h.frame_contains("行 2"));
    assert!(h.is_dirty());
}

#[test]
fn 確認ダイアログと終了() {
    let mut h = Harness::new();
    h.text("x");
    h.keys("ctrl+q");
    assert_eq!(h.overlay(), "dialog");
    assert_eq!(h.quit_code(), None);
    h.answer(1);
    assert_eq!(h.overlay(), "", "{:?}", h.messages());
}

#[test]
#[should_panic(expected = "キー表記")]
fn 不正なキー表記は_panic() {
    Harness::new().keys("ctrl+nosuchkey");
}
