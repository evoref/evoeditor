//! 終了の連鎖 (docs/03 §9.8、01 FR-107): dirty な文書が複数あるとき、全部について確認してから終了する。

use eve_core::{Args, Document, DocumentId, Origin, TextFormat, Tick, Transaction};
use eve_testkit::Harness;
use eve_testkit::helpers::cell_text;
use eve_text::{ByteRange, ChangeSet, Edit, Text};
use eve_view::OverlayModel;

fn run(h: &mut Harness, id: &str, args: Args) {
    h.cmd_with(id, args);
    h.run_jobs();
}

fn esc(h: &mut Harness) {
    h.keys("esc");
    h.run_jobs();
}

fn text(h: &mut Harness, s: &str) {
    h.text(s);
    h.run_jobs();
}

fn quit_requested(h: &Harness) -> bool {
    h.quit_code().is_some()
}

/// 確認ダイアログの本文。
fn message(h: &mut Harness) -> String {
    let f = h.frame();
    let OverlayModel::Confirm { message, .. } = &f.overlays[0] else {
        panic!("確認が出ていない")
    };
    cell_text(message)
}

/// 文書の末尾に `s` を足す (dirty にする)。
fn append(h: &mut Harness, doc: DocumentId, s: &str) {
    let d = h.editor.state_mut().document_mut(doc).unwrap();
    let len = d.len_bytes();
    let cs = ChangeSet::from_sorted_edits(
        &d.snapshot(),
        [Edit {
            range: ByteRange::from(len..len),
            insert: s.into(),
        }],
    )
    .unwrap();
    d.apply(&Transaction::new(cs, Origin::User, Tick(0)))
        .unwrap();
}

fn dirty(h: &Harness, doc: DocumentId) -> bool {
    h.editor.state().document(doc).unwrap().is_dirty()
}

/// a.txt (dirty、アクティブ) と、表示していない無題の文書 (dirty) の 2 件。
fn two_dirty() -> (Harness, DocumentId, DocumentId) {
    let mut s = Harness::new();
    s.fs.add_file("/w/a.txt", b"aaa");
    run(
        &mut s,
        "file.open",
        Args::new().with_str("path", "/w/a.txt"),
    );
    let a = s.doc_id();
    text(&mut s, "x");
    let (b, _) = s.editor.state_mut().add_document(Document::from_text(
        Text::from("bbb"),
        TextFormat::new_document(),
    ));
    append(&mut s, b, "y");
    (s, a, b)
}

// verifies: AC-FR-107-1
#[test]
fn 全_dirty_文書を何件目かつきで確認し_全部破棄したら終了する() {
    let (mut s, a, b) = two_dirty();
    run(&mut s, "app.quit", Args::new());
    assert_eq!(s.overlay(), "dialog");
    assert_eq!(s.doc_id(), a, "アクティブな文書が先頭");
    assert!(message(&mut s).starts_with("(1/2) "), "{}", message(&mut s));

    text(&mut s, "d");
    assert!(!quit_requested(&s), "まだ 1 件残っている");
    assert_eq!(s.doc_id(), b, "次の文書をアクティブにして尋ねる");
    assert!(message(&mut s).starts_with("(2/2) "), "{}", message(&mut s));

    text(&mut s, "d");
    assert!(quit_requested(&s));
    assert!(
        dirty(&s, a) && dirty(&s, b),
        "破棄しても終了が確定するまで内容は残す"
    );
}

// verifies: AC-FR-107-1
#[test]
fn 後の文書でキャンセルすると終了せず_すべての文書が元のまま残る() {
    let (mut s, a, b) = two_dirty();
    run(&mut s, "app.quit", Args::new());
    text(&mut s, "d");
    esc(&mut s);
    assert!(!quit_requested(&s));
    assert_eq!(s.overlay(), "");
    assert_eq!(s.text_of(a), "xaaa");
    assert_eq!(s.text_of(b), "bbby");
    assert!(dirty(&s, a) && dirty(&s, b));

    // 破棄の印は無効: もう一度終了すると、全部をもう一度尋ねる
    run(&mut s, "app.quit", Args::new());
    assert!(message(&mut s).starts_with("(1/2) "), "{}", message(&mut s));
}

#[test]
fn 保存した文書は次へ進み_保存の続きは保存した文書に属する() {
    let (mut s, a, b) = two_dirty();
    run(&mut s, "app.quit", Args::new());
    text(&mut s, "s");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"xaaa");
    assert!(!dirty(&s, a));
    assert!(!quit_requested(&s), "b が残っている");
    assert_eq!(s.doc_id(), b);
    assert_eq!(s.overlay(), "dialog");
    assert!(message(&mut s).starts_with("(2/2) "), "{}", message(&mut s));

    text(&mut s, "d");
    assert!(quit_requested(&s));
}

#[test]
fn 保存に失敗したら連鎖を止めて終了しない() {
    let (mut s, a, _b) = two_dirty();
    run(&mut s, "app.quit", Args::new());
    s.fs.fail_next(
        eve_testkit::FsOp::WriteAtomic,
        eve_core::FsErrorKind::DiskFull,
    );
    text(&mut s, "s");
    assert!(!quit_requested(&s));
    assert!(dirty(&s, a), "保存できていない");
}

#[test]
fn clean_な文書は尋ねず_dirty_が_1_件なら件数を前置しない() {
    let mut s = Harness::new();
    s.fs.add_file("/w/a.txt", b"aaa");
    run(
        &mut s,
        "file.open",
        Args::new().with_str("path", "/w/a.txt"),
    );
    // 表示していない clean な文書
    s.editor.state_mut().add_document(Document::from_text(
        Text::from("ccc"),
        TextFormat::new_document(),
    ));
    text(&mut s, "x");
    run(&mut s, "app.quit", Args::new());
    assert!(!message(&mut s).starts_with('('), "{}", message(&mut s));
    text(&mut s, "d");
    assert!(quit_requested(&s));
}
