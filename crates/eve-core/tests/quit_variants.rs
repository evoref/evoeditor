//! 保存して終了 (`app.save_and_quit`、docs/07 §18.6) と失敗として終了 (`app.quit_with_error`、07 §18.4 の終了コード 4)。
//! どちらも終了の確認は `app.quit` と同じ終了の連鎖 (03 §9.8、01 FR-107) を通る。

use eve_core::{Args, Document, DocumentId, FsErrorKind, Origin, TextFormat, Tick, Transaction};
use eve_testkit::FsOp;
use eve_testkit::Harness;
use eve_text::{ByteRange, ChangeSet, Edit, Text};
use eve_view::{Effect, QuitRescue};

fn run(h: &mut Harness, id: &str) {
    h.cmd(id);
    h.run_jobs();
}

fn text(h: &mut Harness, s: &str) {
    h.text(s);
    h.run_jobs();
}

fn open(h: &mut Harness, path: &str, bytes: &[u8]) -> DocumentId {
    h.fs.add_file(path, bytes);
    h.cmd_with("file.open", Args::new().with_str("path", path));
    h.run_jobs();
    h.doc_id()
}

/// 表示していない無題の文書 (dirty) を足す。
fn add_dirty_untitled(h: &mut Harness) -> DocumentId {
    let (b, _) = h.editor.state_mut().add_document(Document::from_text(
        Text::from("bbb"),
        TextFormat::new_document(),
    ));
    let d = h.editor.state_mut().document_mut(b).unwrap();
    let len = d.len_bytes();
    let cs = ChangeSet::from_sorted_edits(
        &d.snapshot(),
        [Edit {
            range: ByteRange::from(len..len),
            insert: "y".into(),
        }],
    )
    .unwrap();
    d.apply(&Transaction::new(cs, Origin::User, Tick(0)))
        .unwrap();
    b
}

fn rescue(h: &Harness) -> Option<QuitRescue> {
    h.effects().iter().find_map(|e| match e {
        Effect::Quit { rescue, .. } => Some(*rescue),
        _ => None,
    })
}

// ---------- app.save_and_quit ----------

#[test]
fn 保存して終了は表示中の文書を保存してから終了コード_0_で終わる() {
    let mut h = Harness::new();
    open(&mut h, "/w/COMMIT_EDITMSG", b"\n");
    text(&mut h, "fix");
    run(&mut h, "app.save_and_quit");
    assert_eq!(h.fs_read("/w/COMMIT_EDITMSG").unwrap(), b"fix\n");
    assert_eq!(h.quit_code(), Some(0));
    assert_eq!(rescue(&h), Some(QuitRescue::None));
}

#[test]
fn 保存して終了は_dirty_でなければ書かずに終了する() {
    let mut h = Harness::new();
    open(&mut h, "/w/a.txt", b"aaa");
    h.clear_log();
    run(&mut h, "app.save_and_quit");
    assert_eq!(h.quit_code(), Some(0));
    assert!(
        !h.messages().iter().any(|m| m == "info.save.done"),
        "書かない: {:?}",
        h.messages()
    );
}

#[test]
fn 保存して終了は無題の文書なら名前を尋ね_保存できたら終了する() {
    let mut h = Harness::new();
    text(&mut h, "hello");
    run(&mut h, "app.save_and_quit");
    assert_eq!(h.overlay(), "prompt", "名前を尋ねる");
    assert_eq!(h.quit_code(), None);
    text(&mut h, "/w/new.txt");
    h.keys("enter");
    h.run_jobs();
    assert_eq!(h.fs_read("/w/new.txt").unwrap(), b"hello");
    assert_eq!(h.quit_code(), Some(0));
}

#[test]
fn 保存して終了で名前の入力を取り消すと終了しない() {
    let mut h = Harness::new();
    text(&mut h, "hello");
    run(&mut h, "app.save_and_quit");
    h.keys("esc");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_eq!(h.quit_code(), None);
    assert!(h.is_dirty());
}

#[test]
fn 保存して終了で保存が失敗すると終了しない() {
    let mut h = Harness::new();
    open(&mut h, "/w/a.txt", b"aaa");
    text(&mut h, "x");
    h.fs.fail_next(FsOp::WriteAtomic, FsErrorKind::PermissionDenied);
    run(&mut h, "app.save_and_quit");
    assert_eq!(h.quit_code(), None, "保存できなければ終了しない");
    assert!(h.is_dirty());
    assert_eq!(h.overlay(), "dialog", "保存の失敗の確認を出す");
}

#[test]
fn 保存して終了はほかに未保存の文書があれば終了の連鎖で確かめる() {
    let mut h = Harness::new();
    open(&mut h, "/w/a.txt", b"aaa");
    text(&mut h, "x");
    let b = add_dirty_untitled(&mut h);
    run(&mut h, "app.save_and_quit");
    assert_eq!(
        h.fs_read("/w/a.txt").unwrap(),
        b"xaaa",
        "表示中の文書を保存"
    );
    assert_eq!(h.quit_code(), None, "残りの文書を確かめる");
    assert_eq!(h.overlay(), "dialog");
    assert_eq!(h.doc_id(), b);
    // 破棄すると終了する
    text(&mut h, "d");
    assert_eq!(h.quit_code(), Some(0));
}

// ---------- app.quit_with_error ----------

#[test]
fn 失敗として終了は未保存の文書が無ければ確認なしで終了コード_4_で終わる() {
    let mut h = Harness::new();
    open(&mut h, "/w/a.txt", b"aaa");
    run(&mut h, "app.quit_with_error");
    assert_eq!(h.quit_code(), Some(4));
    assert_eq!(rescue(&h), Some(QuitRescue::None));
}

// verifies: AC-FR-107-1
#[test]
fn 失敗として終了は未保存の文書があれば確かめ_破棄すると保存せず終了コード_4() {
    let mut h = Harness::new();
    open(&mut h, "/w/COMMIT_EDITMSG", b"\n");
    text(&mut h, "wip");
    run(&mut h, "app.quit_with_error");
    assert_eq!(h.overlay(), "dialog", "app.quit と同じ確認 (FR-107)");
    assert_eq!(h.quit_code(), None);
    text(&mut h, "d");
    assert_eq!(h.quit_code(), Some(4));
    assert_eq!(
        h.fs_read("/w/COMMIT_EDITMSG").unwrap(),
        b"\n",
        "保存していない"
    );
}

#[test]
fn 失敗として終了の確認をキャンセルすると終了せず_次の_app_quit_は終了コード_0() {
    let mut h = Harness::new();
    open(&mut h, "/w/a.txt", b"aaa");
    text(&mut h, "x");
    run(&mut h, "app.quit_with_error");
    h.keys("esc");
    h.run_jobs();
    assert_eq!(h.quit_code(), None);
    assert!(h.is_dirty());
    run(&mut h, "app.quit");
    text(&mut h, "d");
    assert_eq!(h.quit_code(), Some(0), "前の要求の終了コードを持ち越さない");
}

#[test]
fn 失敗として終了の確認で保存を選んでも終了コードは_4() {
    let mut h = Harness::new();
    open(&mut h, "/w/a.txt", b"aaa");
    text(&mut h, "x");
    run(&mut h, "app.quit_with_error");
    text(&mut h, "s");
    assert_eq!(h.fs_read("/w/a.txt").unwrap(), b"xaaa");
    assert_eq!(h.quit_code(), Some(4));
}

#[test]
fn 三つのコマンドはパレットに出て既定のキーを持たない() {
    let h = Harness::new();
    let reg = h.editor.registry();
    for id in [
        "app.save_and_quit",
        "app.quit_with_error",
        "config.show_diagnostics",
    ] {
        assert!(
            reg.get(&eve_view::CommandId::new(id)).is_some(),
            "{id} を登録する"
        );
        assert!(!eve_core::command_hidden(id), "{id} はパレットに出す");
        assert!(
            h.editor
                .keymap()
                .all_bindings()
                .iter()
                .all(|b| b.command.as_str() != id),
            "{id} に既定のキーは無い (07 §11.6 の「—」)"
        );
    }
}
