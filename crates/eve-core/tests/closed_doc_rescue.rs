//! 閉じた文書の保存ジョブが実行中の間は、内容を緊急退避の対象に残す (03 §9.2 の閉じる手順 (a)、§9.8 の不変条件 4)。
//! 保存が成功すれば外し、失敗すれば退避先に書き終えるまで残す。

use eve_core::FsErrorKind;
use eve_testkit::FsOp;
use eve_testkit::Harness;

/// 編集して保存を始め (ジョブは未実行)、その文書を閉じた状態。
fn closed_while_saving() -> (Harness, eve_view::DocumentId) {
    let mut h = Harness::new().with_file("/w/a.txt", b"hello\n");
    h.open("/w/a.txt");
    h.run_jobs();
    h.text("X");
    let doc = h.doc_id();
    h.cmd("file.save");
    assert_eq!(h.pending_jobs(), 1, "保存ジョブが実行待ち");
    h.editor.state_mut().close_document(doc);
    (h, doc)
}

#[test]
fn 保存中に閉じた文書は_保存の結果が届くまで緊急退避の対象に残る() {
    let (mut h, doc) = closed_while_saving();
    let snap = h
        .editor
        .emergency_snapshot(doc)
        .expect("保存の完了まで公開物を残す");
    assert!(String::from_utf8(snap.to_vec()).unwrap().contains('X'));
    assert!(
        h.editor.take_snapshot_updates().contains(&doc),
        "更新の通知は出る (公開物を差し替える)"
    );
}

#[test]
fn 保存が成功したら公開物を外す() {
    let (mut h, doc) = closed_while_saving();
    h.run_jobs();
    assert_eq!(h.fs_read("/w/a.txt").as_deref(), Some(&b"Xhello\n"[..]));
    assert!(h.editor.emergency_snapshot(doc).is_none());
    assert!(h.editor.take_snapshot_updates().contains(&doc));
}

#[test]
fn 保存が失敗したら退避先に書き終えるまで残し_書いたら外す() {
    let (mut h, doc) = closed_while_saving();
    h.fs.fail_next(FsOp::WriteAtomic, FsErrorKind::PermissionDenied);
    // 1 周目: 保存の失敗。確認は出さず、退避先への書込が始まる (その間も公開物は残る)
    h.run_jobs_once();
    assert_eq!(h.overlay(), "", "閉じた文書に確認は出さない");
    assert!(
        h.editor.emergency_snapshot(doc).is_some(),
        "退避先に書き終えるまで残す"
    );
    // 2 周目: 退避先に書く
    h.run_jobs();
    let written = h.fs.recovery_written();
    assert_eq!(written.len(), 1, "退避先に 1 件");
    assert!(String::from_utf8_lossy(&written[0]).contains("Xhello"));
    assert!(h.editor.emergency_snapshot(doc).is_none());
}

#[test]
fn 保存中でない文書を閉じれば公開物は直ちに外れる() {
    let mut h = Harness::new().with_file("/w/a.txt", b"hello\n");
    h.open("/w/a.txt");
    h.run_jobs();
    h.text("X");
    let doc = h.doc_id();
    h.editor.state_mut().close_document(doc);
    assert!(
        h.editor.emergency_snapshot(doc).is_none(),
        "破棄した内容を緊急退避に残さない"
    );
}
