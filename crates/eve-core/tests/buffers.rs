//! バッファ (複数文書) の切替と閉じる (docs/03 §9.2 の閉じる手順、§9.8、01 FR-103 / FR-107)。

use eve_core::{Document, DocumentId, FsPath, TextFormat};
use eve_testkit::Harness;
use eve_testkit::helpers::cell_text;
use eve_text::Text;
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

fn quit_requested(h: &Harness) -> bool {
    h.quit_code().is_some()
}

fn message(h: &mut Harness) -> String {
    let f = h.frame();
    let OverlayModel::Confirm { message, .. } = &f.overlays[0] else {
        panic!("確認が出ていない")
    };
    cell_text(message)
}

fn exists(h: &Harness, doc: DocumentId) -> bool {
    h.editor.state().document(doc).is_some()
}

/// 開いた順に a, b, c の 3 文書 (a が表示中)。
fn three() -> (Harness, [DocumentId; 3]) {
    let mut s = Harness::new();
    let a = s.doc_id();
    let add = |s: &mut Harness, t: &str| {
        s.editor
            .state_mut()
            .add_document(Document::from_text(
                Text::from(t),
                TextFormat::new_document(),
            ))
            .0
    };
    let b = add(&mut s, "bbb");
    let c = add(&mut s, "ccc");
    (s, [a, b, c])
}

#[test]
fn next_と_prev_は開いた順で循環する() {
    let (mut s, [a, b, c]) = three();
    run(&mut s, "buffer.next");
    assert_eq!(s.doc_id(), b);
    run(&mut s, "buffer.next");
    assert_eq!(s.doc_id(), c);
    run(&mut s, "buffer.next");
    assert_eq!(s.doc_id(), a, "末尾の次は先頭");
    run(&mut s, "buffer.prev");
    assert_eq!(s.doc_id(), c, "先頭の前は末尾");
    run(&mut s, "buffer.prev");
    assert_eq!(s.doc_id(), b);
}

#[test]
fn 既定キーで切り替わり_文書が_1_件なら何も起きない() {
    let (mut s, [a, b, _c]) = three();
    keys(&mut s, "ctrl+pagedown");
    assert_eq!(s.doc_id(), b);
    keys(&mut s, "ctrl+pageup");
    assert_eq!(s.doc_id(), a);
    keys(&mut s, "ctrl+k");
    text(&mut s, "n");
    assert_eq!(s.doc_id(), b, "ctrl+k n");

    let mut one = Harness::new();
    let only = one.doc_id();
    run(&mut one, "buffer.next");
    run(&mut one, "buffer.prev");
    assert_eq!(one.doc_id(), only);
}

#[test]
fn 切替の後も編集と_undo_の履歴が保たれる() {
    let (mut s, [a, b, _c]) = three();
    text(&mut s, "x");
    assert_eq!(s.text_of(a), "x");
    run(&mut s, "buffer.next");
    text(&mut s, "y");
    run(&mut s, "buffer.prev");
    assert_eq!(s.doc_id(), a);
    assert_eq!(s.text_of(a), "x");
    assert_eq!(s.text_of(b), "ybbb");
    run(&mut s, "edit.undo");
    assert_eq!(s.text_of(a), "", "a の履歴は b の編集と混ざらない");
    assert_eq!(s.text_of(b), "ybbb");
}

#[test]
fn 閉じた後は最近使った順で次の文書を表示する() {
    let (mut s, [a, b, c]) = three();
    run(&mut s, "buffer.next"); // b
    run(&mut s, "buffer.next"); // c
    run(&mut s, "buffer.close");
    assert!(!exists(&s, c));
    assert_eq!(
        s.doc_id(),
        b,
        "直前に使っていた b (開いた順なら a ではない)"
    );
    run(&mut s, "buffer.close");
    assert_eq!(s.doc_id(), a);
    assert!(!exists(&s, b));
}

#[test]
fn 最後の_1_件を閉じると無題の文書を作り_終了しない() {
    let mut s = Harness::new();
    let only = s.doc_id();
    run(&mut s, "buffer.close");
    assert!(!quit_requested(&s));
    assert!(!exists(&s, only));
    assert_eq!(s.editor.state().documents_len(), 1);
    assert_eq!(s.text_of(s.doc_id()), "");
}

// verifies: AC-FR-107-1
#[test]
fn dirty_な文書を閉じると確認が出て_キャンセルでも破棄でも他の文書は残る() {
    let (mut s, [a, b, _c]) = three();
    text(&mut s, "x");
    run(&mut s, "buffer.close");
    assert_eq!(s.overlay(), "dialog");
    assert!(
        message(&mut s).contains("保存されていない変更"),
        "{}",
        message(&mut s)
    );
    keys(&mut s, "esc");
    assert!(exists(&s, a) && exists(&s, b), "キャンセルで何も変わらない");
    assert_eq!(s.doc_id(), a);
    assert_eq!(s.text_of(a), "x");

    run(&mut s, "buffer.close");
    text(&mut s, "d");
    assert!(!exists(&s, a), "破棄して閉じる");
    assert!(exists(&s, b));
    assert_eq!(s.overlay(), "");
}

/// /w/a.txt を開いて 1 文字入力した (dirty な) 状態に、表示していない文書を 1 件足す。
fn dirty_file_and_other() -> (Harness, DocumentId, DocumentId) {
    let mut s = Harness::new();
    s.fs.add_file("/w/a.txt", b"aaa");
    s.open("/w/a.txt");
    s.run_jobs();
    let a = s.doc_id();
    text(&mut s, "x");
    let b = s
        .editor
        .state_mut()
        .add_document(Document::from_text(
            Text::from("bbb"),
            TextFormat::new_document(),
        ))
        .0;
    (s, a, b)
}

#[test]
fn 保存して閉じる() {
    let (mut s, a, b) = dirty_file_and_other();
    run(&mut s, "buffer.close");
    text(&mut s, "s");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"xaaa");
    assert!(!exists(&s, a), "保存できたので閉じる");
    assert_eq!(s.doc_id(), b);
}

#[test]
fn 保存に失敗したら閉じない() {
    let (mut s, a, _b) = dirty_file_and_other();
    run(&mut s, "buffer.close");
    s.fs.fail_next(
        eve_testkit::FsOp::WriteAtomic,
        eve_core::FsErrorKind::DiskFull,
    );
    text(&mut s, "s");
    assert!(exists(&s, a), "保存に失敗したら閉じない");
    assert_eq!(s.doc_id(), a);
}

fn title(h: &mut Harness) -> String {
    h.frame().title
}

/// ステータスバーのファイル名の項目。
fn status_name(h: &mut Harness) -> String {
    let f = h.frame();
    let name = &f.status.items[0];
    assert_eq!(name.kind, eve_view::StatusKind::Path);
    cell_text(&name.long)
}

#[test]
fn ステータスバーは文書が_2_件以上のとき何番目かを出す() {
    let mut one = Harness::new();
    assert_eq!(status_name(&mut one), "無題-1", "1 件なら出さない");

    let (mut s, _) = three();
    assert_eq!(status_name(&mut s), "無題-1 (1/3)");
    run(&mut s, "buffer.next");
    assert_eq!(status_name(&mut s), "無題-1 (2/3)");
    run(&mut s, "buffer.prev");
    run(&mut s, "buffer.prev");
    assert_eq!(status_name(&mut s), "無題-1 (3/3)");
    run(&mut s, "buffer.close");
    assert_eq!(
        status_name(&mut s),
        "無題-1 (1/2)",
        "閉じると数が減り、最近使った a を表示する"
    );
}

#[test]
fn 端末のタイトルにも何番目かを出す() {
    let mut one = Harness::new();
    assert_eq!(title(&mut one), "無題 - evoeditor", "1 件なら出さない");
    let (mut s, _) = three();
    assert_eq!(title(&mut s), "無題 (1/3) - evoeditor");
    run(&mut s, "buffer.next");
    assert_eq!(title(&mut s), "無題 (2/3) - evoeditor");
}

/// a.txt, b.txt, c.txt の 3 文書 (開いた順。a が表示中)。
fn named_three() -> (Harness, [DocumentId; 3]) {
    let (mut s, ids) = three();
    for (id, name) in ids.iter().zip(["a.txt", "b.txt", "c.txt"]) {
        s.editor.state_mut().document_mut(*id).unwrap().path =
            Some(FsPath::new(format!("/w/{name}")));
    }
    (s, ids)
}

/// バッファ一覧の (行のラベル, 選択中の番号)。
fn list_rows(h: &mut Harness) -> (Vec<String>, usize) {
    let f = h.frame();
    let OverlayModel::List {
        items, selected, ..
    } = &f.overlays[0]
    else {
        panic!("一覧が出ていない")
    };
    let rows = items.iter().map(|i| cell_text(&i.label)).collect();
    (rows, *selected)
}

#[test]
fn 一覧は最近使った順で_直前の文書が_2_番目に選ばれている() {
    let (mut s, _) = named_three();
    run(&mut s, "buffer.next"); // b。直前は a
    run(&mut s, "buffer.list");
    assert_eq!(s.overlay(), "buffer_list");
    let (rows, selected) = list_rows(&mut s);
    assert_eq!(rows, ["    b.txt", "    a.txt", "    c.txt"]);
    assert_eq!(selected, 1, "直前の a が選ばれている");
}

#[test]
fn ctrl_e_enter_で直前の文書と往復できる() {
    let (mut s, [a, b, _c]) = named_three();
    keys(&mut s, "ctrl+e");
    assert_eq!(s.overlay(), "buffer_list");
    keys(&mut s, "enter");
    assert_eq!(s.overlay(), "");
    assert_eq!(s.doc_id(), b, "a の直前は無いので 2 番目 (開いた順で b)");
    keys(&mut s, "ctrl+e");
    keys(&mut s, "enter");
    assert_eq!(s.doc_id(), a, "もう一度で元の文書");
}

#[test]
fn 一覧は入力で絞り込め_dirty_を印で示し_esc_で何も変えずに閉じる() {
    let (mut s, [a, _b, _c]) = named_three();
    text(&mut s, "x"); // a を dirty に
    run(&mut s, "buffer.list");
    let (rows, _) = list_rows(&mut s);
    assert_eq!(rows[0], "[+] a.txt", "変更マーカー");
    text(&mut s, "c.t");
    let (rows, selected) = list_rows(&mut s);
    assert_eq!(rows, ["    c.txt"]);
    assert_eq!(selected, 0, "入力が変わると先頭を選ぶ");
    keys(&mut s, "esc");
    assert_eq!(s.overlay(), "");
    assert_eq!(s.doc_id(), a);
    assert_eq!(s.editor.state().documents_len(), 3);
}

#[test]
fn 一覧の中の_ctrl_w_は選んでいる文書を閉じる() {
    let (mut s, [a, b, c]) = named_three();
    run(&mut s, "buffer.list");
    keys(&mut s, "down"); // 2 番目 (b) → 3 番目 (c)
    keys(&mut s, "ctrl+w");
    assert!(!exists(&s, c), "選んでいた c を閉じる");
    assert!(exists(&s, a) && exists(&s, b));
    assert_eq!(s.overlay(), "buffer_list", "一覧は開いたまま");
    assert_eq!(s.doc_id(), a, "表示中の文書は変わらない");
    let (rows, _) = list_rows(&mut s);
    assert_eq!(rows, ["    a.txt", "    b.txt"]);
}

#[test]
fn 一覧の中で_dirty_な文書を閉じると確認が出る() {
    let (mut s, [a, b, _c]) = named_three();
    text(&mut s, "x"); // a
    run(&mut s, "buffer.next"); // b
    run(&mut s, "buffer.list"); // [b, a, c]、選択は a
    keys(&mut s, "ctrl+w");
    assert_eq!(s.overlay(), "dialog", "a は dirty なので確認");
    text(&mut s, "d");
    assert!(!exists(&s, a) && exists(&s, b));
    assert_eq!(s.doc_id(), b);
}
