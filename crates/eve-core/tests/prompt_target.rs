//! 入力・確認の対象は、開いた時点の文書に固定する (docs/03 §9.6・§9.8 の「入力・確認の対象」)。
//! 名前を付けて保存・文字コードの入力の間に、ほかの文書の読込が完了して表示が切り替わっても、確定では開いた時点の
//! 文書に効かせる (表示中の別の文書を書かない・変えない)。確定の時点で対象が閉じられていたら、何もせずに知らせる。

use eve_core::{Document, DocumentId, OpenOptions};
use eve_encoding::Encoding;
use eve_testkit::Harness;

fn doc(h: &Harness, id: DocumentId) -> &Document {
    h.editor.state().document(id).expect("文書がある")
}

fn path_of(h: &Harness, id: DocumentId) -> Option<String> {
    doc(h, id).path.as_ref().map(|p| p.display())
}

fn disk(h: &Harness, path: &str) -> Option<String> {
    h.fs_read(path)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// a.txt を開いて先頭に `x` を入力し (dirty)、b.txt を開く (読込はまだ)。表示は a のまま。
fn a_dirty_b_loading() -> (Harness, DocumentId) {
    let mut h = Harness::new()
        .with_file("/w/a.txt", b"alpha\n")
        .with_file("/w/b.txt", b"beta\n")
        .with_file("/w/c.txt", b"gamma\n");
    h.open("/w/a.txt");
    h.run_jobs();
    h.text("x");
    let a = h.doc_id();
    assert!(h.is_dirty());
    h.open("/w/b.txt");
    assert_eq!(h.doc_id(), a, "読込の間は a を表示したまま");
    (h, a)
}

#[test]
fn 名前を付けて保存の入力中に表示が切り替わっても入力を開いた文書を保存する() {
    let (mut h, a) = a_dirty_b_loading();
    // a の名前を付けて保存の入力 (初期値は a のパス)
    h.cmd("file.save_as");
    assert_eq!(h.overlay(), "prompt");
    // ここで b の読込が完了して、表示が b に切り替わる
    h.run_jobs();
    let b = h.doc_id();
    assert_ne!(b, a, "表示は b に切り替わる (この PR では切替を保留しない)");
    assert_eq!(h.overlay(), "prompt");
    // 利用者は a を保存するつもりで確定する
    h.keys("enter");
    h.run_jobs();
    assert_eq!(
        h.overlay(),
        "",
        "確認は出ない (a を a.txt へ書くので照合が通る)"
    );
    assert_eq!(disk(&h, "/w/a.txt").as_deref(), Some("xalpha\n"));
    assert!(!doc(&h, a).is_dirty(), "a は保存された");
    assert_eq!(
        disk(&h, "/w/b.txt").as_deref(),
        Some("beta\n"),
        "b.txt は変わらない"
    );
    assert_eq!(path_of(&h, b).as_deref(), Some("/w/b.txt"));
    assert!(!doc(&h, b).is_dirty());
    assert_eq!(h.doc_id(), b, "表示は b のまま");
}

#[test]
fn 名前を付けて保存の確定の前に対象の文書が閉じられたら保存しない() {
    let (mut h, a) = a_dirty_b_loading();
    h.cmd("file.save_as");
    assert_eq!(h.overlay(), "prompt");
    // b の読込が完了して表示が b に切り替わる
    h.run_jobs();
    let b = h.doc_id();
    assert_ne!(a, b);
    // 入力の間に対象の a が閉じられる (入力はモーダルなので、コマンドではなく状態を直接変えて模す)
    h.editor.state_mut().close_document(a);
    assert_eq!(h.overlay(), "prompt");
    h.clear_log();
    h.keys("enter");
    h.run_jobs();
    assert_eq!(h.overlay(), "", "入力は閉じる (戻さない)");
    assert_eq!(
        disk(&h, "/w/a.txt").as_deref(),
        Some("alpha\n"),
        "a.txt は変わらない"
    );
    assert_eq!(
        disk(&h, "/w/b.txt").as_deref(),
        Some("beta\n"),
        "表示中の b を代わりに保存しない"
    );
    assert_eq!(path_of(&h, b).as_deref(), Some("/w/b.txt"));
    assert_eq!(
        h.messages(),
        ["info.prompt.target_closed"],
        "閉じられたことを知らせる"
    );
}

#[test]
fn 文字コードを指定して保存の入力中に表示が切り替わっても入力を開いた文書を書く() {
    let mut h = Harness::new()
        .with_file("/w/a.txt", "あ\n".as_bytes())
        .with_file("/w/b.txt", "い\n".as_bytes());
    h.open("/w/a.txt");
    h.run_jobs();
    let a = h.doc_id();
    h.open("/w/b.txt");
    // 引数なしの file.set_encoding は文字コードを尋ねる (対象は a)
    h.cmd("file.set_encoding");
    assert_eq!(h.overlay(), "prompt");
    h.run_jobs();
    let b = h.doc_id();
    assert_ne!(a, b, "表示は b に切り替わる");
    h.text("shift_jis");
    h.keys("enter");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_eq!(
        h.fs_read("/w/a.txt").as_deref(),
        Some(&[0x82, 0xA0, b'\n'][..]),
        "a.txt を Shift_JIS で書く"
    );
    assert_eq!(doc(&h, a).format.encoding, Encoding::ShiftJis);
    assert_eq!(
        h.fs_read("/w/b.txt").as_deref(),
        Some("い\n".as_bytes()),
        "b.txt は変わらない"
    );
    assert_eq!(doc(&h, b).format.encoding, Encoding::Utf8);
}

#[test]
fn 文字コードを指定して開き直す入力中に表示が切り替わっても入力を開いた文書を開き直す() {
    let mut h = Harness::new()
        .with_file("/w/a.txt", b"alpha\n")
        .with_file("/w/b.txt", b"beta\n");
    h.open("/w/a.txt");
    h.run_jobs();
    let a = h.doc_id();
    // 引数なしの file.reopen_with_encoding は文字コードを尋ねる (対象は a)
    h.cmd("file.reopen_with_encoding");
    assert_eq!(h.overlay(), "prompt");
    // 入力の間に b の読込が始まって完了し、表示が b に切り替わる (入力はモーダルなので、読込は状態から直接始める)
    h.editor
        .state_mut()
        .open_path("/w/b.txt", OpenOptions::default());
    h.run_jobs();
    let b = h.doc_id();
    assert_ne!(a, b, "表示は b に切り替わる");
    assert_eq!(h.overlay(), "prompt");
    h.text("shift_jis");
    h.keys("enter");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_eq!(
        doc(&h, a).format.encoding,
        Encoding::ShiftJis,
        "a を開き直す"
    );
    assert_eq!(
        doc(&h, b).format.encoding,
        Encoding::Utf8,
        "b は開き直さない"
    );
}

#[test]
fn 編集を許可の確認から開き直すのは確認を出した文書() {
    // 不正バイト (UTF-8 として読むと 82 A0 が不正) を含む a。比率の確認が出ないよう後ろを詰める
    let pad = "0123456789".repeat(40);
    let a_bytes = [b"a\x82\xA0b\n".as_slice(), pad.as_bytes()].concat();
    let mut h = Harness::new()
        .with_file("/w/a.txt", &a_bytes)
        .with_file("/w/b.txt", b"beta\n");
    h.open("/w/a.txt");
    h.run_jobs();
    let a = h.doc_id();
    assert!(!doc(&h, a).is_editable(), "不正バイトで読み取り専用");
    h.open("/w/b.txt");
    // a の「編集を許可」の確認 (編集を許可 / 文字コードを指定して開き直す / キャンセル)
    h.cmd("file.toggle_read_only");
    assert_eq!(h.overlay(), "dialog");
    h.run_jobs();
    let b = h.doc_id();
    assert_ne!(a, b, "表示は b に切り替わる");
    // 「文字コードを指定して開き直す」: 確認を出した a の文字コードを尋ねる
    h.text("e");
    assert_eq!(h.overlay(), "prompt");
    h.text("shift_jis");
    h.keys("enter");
    h.run_jobs();
    assert_eq!(
        doc(&h, a).format.encoding,
        Encoding::ShiftJis,
        "a を開き直す"
    );
    assert!(doc(&h, a).is_editable(), "Shift_JIS では 82 A0 は「あ」");
    assert_eq!(
        doc(&h, b).format.encoding,
        Encoding::Utf8,
        "b は開き直さない"
    );
}

#[test]
fn 保存の確認から開いた名前の入力も確認の文書を保存する() {
    let (mut h, a) = a_dirty_b_loading();
    // a を既存の c.txt へ名前を付けて保存する: 保存先が後から作られていた確認 (上書き / 別名で保存 (既定))
    h.cmd_with(
        "file.save_as",
        eve_core::Args::new().with_str("path", "/w/c.txt"),
    );
    // 保存の照合と b の読込が完了する (表示は b に切り替わる)
    h.run_jobs();
    let b = h.doc_id();
    assert_ne!(a, b, "表示は b に切り替わる");
    assert_eq!(h.overlay(), "dialog");
    // 「別名で保存」で名前の入力へ (対象は確認の文書 a)
    h.answer(1);
    assert_eq!(h.overlay(), "prompt");
    h.text("/w/d.txt");
    h.keys("enter");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_eq!(
        disk(&h, "/w/d.txt").as_deref(),
        Some("xalpha\n"),
        "a を書く"
    );
    assert_eq!(path_of(&h, a).as_deref(), Some("/w/d.txt"));
    assert!(!doc(&h, a).is_dirty());
    assert_eq!(disk(&h, "/w/c.txt").as_deref(), Some("gamma\n"));
    assert_eq!(
        path_of(&h, b).as_deref(),
        Some("/w/b.txt"),
        "b は名前を変えない"
    );
}
