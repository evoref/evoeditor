//! 全保存 `file.save_all` (docs/03 §9.8、01 FR-104 AC-2): dirty な全文書を保存し、失敗した文書を一覧で示す。

use eve_core::{
    Document, DocumentId, FsPath, Origin, ReadOnlyReason, TextFormat, Tick, Transaction,
};
use eve_testkit::Harness;
use eve_testkit::helpers::cell_text;
use eve_text::{ByteRange, ChangeSet, Edit, Text};
use eve_view::OverlayModel;

fn run(h: &mut Harness, id: &str) {
    h.cmd(id);
    h.run_jobs();
}

fn keys(h: &mut Harness, seq: &str) {
    h.keys(seq);
    h.run_jobs();
}

fn text(h: &mut Harness, s: &str) {
    h.text(s);
    h.run_jobs();
}

/// 文書を足す。`path` があれば文書のパスにする (ディスクには無い = 新規のファイル)。dirty にして返す。
fn add_dirty(h: &mut Harness, path: Option<&str>, body: &str) -> DocumentId {
    let (id, _) = h.editor.state_mut().add_document(Document::from_text(
        Text::from(body),
        TextFormat::new_document(),
    ));
    let d = h.editor.state_mut().document_mut(id).unwrap();
    d.path = path.map(FsPath::new);
    let len = d.len_bytes();
    let cs = ChangeSet::from_sorted_edits(
        &d.snapshot(),
        [Edit {
            range: ByteRange::from(len..len),
            insert: "!".into(),
        }],
    )
    .unwrap();
    d.apply(&Transaction::new(cs, Origin::User, Tick(0)))
        .unwrap();
    id
}

fn dirty(h: &Harness, doc: DocumentId) -> bool {
    h.editor.state().document(doc).unwrap().is_dirty()
}

/// 確認ダイアログの (本文, details)。
fn dialog(h: &mut Harness) -> (String, Vec<String>) {
    let f = h.frame();
    let OverlayModel::Confirm {
        message, details, ..
    } = &f.overlays[0]
    else {
        panic!("確認が出ていない")
    };
    (
        cell_text(message),
        details
            .iter()
            .flatten()
            .map(cell_text)
            .collect::<Vec<String>>(),
    )
}

// verifies: AC-FR-104-1
#[test]
fn dirty_な全文書を保存し_名前の無い文書と読み取り専用の文書は飛ばす() {
    let mut s = Harness::new();
    let active = s.editor.state().active_document().unwrap();
    let a = add_dirty(&mut s, Some("/w/a.txt"), "aaa");
    let b = add_dirty(&mut s, Some("/w/b.txt"), "bbb");
    let untitled = add_dirty(&mut s, None, "uuu");
    let ro = add_dirty(&mut s, Some("/w/ro.txt"), "rrr");
    s.editor
        .state_mut()
        .document_mut(ro)
        .unwrap()
        .read_only
        .insert(ReadOnlyReason::UserRequested);
    // 表示中の文書は変えない
    run(&mut s, "file.save_all");

    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"aaa!");
    assert_eq!(s.fs.read("/w/b.txt").unwrap(), b"bbb!");
    assert!(!s.fs.exists("/w/ro.txt"), "読み取り専用は書かない");
    assert!(!dirty(&s, a) && !dirty(&s, b));
    assert!(
        dirty(&s, untitled) && dirty(&s, ro),
        "飛ばした文書は dirty のまま"
    );
    assert_eq!(s.overlay(), "", "失敗が無いので確認は出ない");
    assert_eq!(s.editor.state().active_document().unwrap(), active);
}

// verifies: AC-FR-104-1
#[test]
fn 既定キーで実行でき_dirty_が無ければ何も書かない() {
    let mut s = Harness::new();
    keys(&mut s, "ctrl+k");
    text(&mut s, "a");
    assert_eq!(s.overlay(), "");
    let a = add_dirty(&mut s, Some("/w/a.txt"), "aaa");
    keys(&mut s, "ctrl+k");
    text(&mut s, "a");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"aaa!");
    assert!(!dirty(&s, a));
}

// verifies: AC-FR-104-1
#[test]
fn 失敗した文書は確認を出さずに記録し_最後に一覧で示す() {
    let mut s = Harness::new();
    let a = add_dirty(&mut s, Some("/w/a.txt"), "aaa");
    let b = add_dirty(&mut s, Some("/w/b.txt"), "bbb");
    let c = add_dirty(&mut s, Some("/w/c.txt"), "ccc");
    // b は保存の間に別のプロセスが同名のファイルを作った (外部変更の確認が要る保存)
    s.fs.add_file("/w/b.txt", b"external");
    run(&mut s, "file.save_all");

    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"aaa!");
    assert_eq!(s.fs.read("/w/c.txt").unwrap(), b"ccc!");
    assert_eq!(s.fs.read("/w/b.txt").unwrap(), b"external", "上書きしない");
    assert!(!dirty(&s, a) && !dirty(&s, c));
    assert!(dirty(&s, b), "失敗した文書は dirty のまま");

    // 一覧の確認 1 つだけ (外部変更の確認は出ない)
    assert_eq!(s.overlay(), "dialog");
    let (message, details) = dialog(&mut s);
    assert!(message.starts_with("1 件"), "{message}");
    assert_eq!(details.len(), 1, "{details:?}");
    assert!(details[0].starts_with("/w/b.txt: "), "{details:?}");

    keys(&mut s, "esc");
    assert_eq!(s.overlay(), "");
    assert!(dirty(&s, b));
}

// verifies: FR-104
#[test]
fn 保存の失敗は他の文書の保存を止めない() {
    let mut s = Harness::new();
    let a = add_dirty(&mut s, Some("/w/a.txt"), "aaa");
    let b = add_dirty(&mut s, Some("/w/b.txt"), "bbb");
    s.fs.fail_next(
        eve_testkit::FsOp::WriteAtomic,
        eve_core::FsErrorKind::DiskFull,
    );
    run(&mut s, "file.save_all");
    // 1 件目が失敗し、2 件目は保存される
    assert!(dirty(&s, a) && !dirty(&s, b));
    assert_eq!(s.fs.read("/w/b.txt").unwrap(), b"bbb!");
    assert_eq!(s.overlay(), "dialog");
}

// NFR-301 AC-2: 元のファイルが退避名に移った失敗は、一覧に退避名を出す (docs/review/v0.3-findings.md の SAF-16)
#[test]
fn 置換の途中で失敗した文書は_一覧に退避名を出す() {
    let mut s = Harness::new();
    add_dirty(&mut s, Some("/w/a.txt"), "aaa");
    s.fs.fail_next(
        eve_testkit::FsOp::WriteAtomic,
        eve_core::FsErrorKind::ReplacedMovedToBackup {
            backup: "/w/.a.txt.eve-bak-1".into(),
        },
    );
    run(&mut s, "file.save_all");
    assert_eq!(s.overlay(), "dialog");
    let (_, details) = dialog(&mut s);
    assert_eq!(details.len(), 1, "{details:?}");
    assert!(
        details[0].starts_with("/w/a.txt: ") && details[0].contains("/w/.a.txt.eve-bak-1"),
        "{details:?}"
    );
}
