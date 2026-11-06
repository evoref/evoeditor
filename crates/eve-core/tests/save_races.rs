//! 保存・開き直し・すべて置換が、閉じる・切り替える・別のジョブと重なったときの回帰試験
//! (docs/review/v0.3-findings.md の §2.7 の 1・3・5・12。SAF-01 / 03 / 05 / 06 / 17 は M12 で直した)。

use eve_core::{Args, SaveWarning};
use eve_testkit::Harness;
use eve_testkit::helpers::cell_text;

fn message(h: &mut Harness) -> String {
    h.frame()
        .messages
        .current
        .map(|m| cell_text(&m.text))
        .unwrap_or_default()
}

/// a.txt と b.txt を開き、どちらも未保存の編集をした状態 (Harness, a, b)。今の文書は a。
fn two_dirty() -> (Harness, eve_view::DocumentId, eve_view::DocumentId) {
    let mut h = Harness::new()
        .with_file("/w/a.txt", b"aaa\n")
        .with_file("/w/b.txt", b"bbb\n");
    h.open("/w/a.txt");
    h.run_jobs();
    let a = h.doc_id();
    h.text("x");
    h.open("/w/b.txt");
    h.run_jobs();
    let b = h.doc_id();
    h.text("y");
    h.cmd("buffer.prev");
    assert_eq!(h.doc_id(), a);
    (h, a, b)
}

// SAF-01: 開き直しの「保存」の完了を待つ間に、その文書を閉じても、他の文書は開き直さない
#[test]
fn 開き直しの保存中にその文書を閉じても_他の文書の本文と履歴は変わらない() {
    let (mut h, _a, b) = two_dirty();
    h.cmd_with(
        "file.reopen_with_encoding",
        Args::new().with_str("encoding", "shift_jis"),
    );
    assert_eq!(h.overlay(), "dialog", "未保存の確認");
    h.text("s"); // 保存してから開き直す
    assert_eq!(h.pending_jobs(), 1, "保存ジョブが実行待ち");
    h.cmd("buffer.close");
    h.run_jobs();
    // 閉じる確認が出たら閉じずに戻す (どちらの経路でも、他の文書が開き直されないこと)
    if h.overlay() != "" {
        h.keys("esc");
    }
    let bd = h.editor.state().document(b).unwrap();
    assert_eq!(bd.to_string_lossy(), "ybbb\n", "b の未保存の編集が残る");
    assert!(bd.is_dirty());
    assert_eq!(
        bd.format.encoding,
        eve_core::Encoding::Utf8,
        "b は開き直さない"
    );
    assert_eq!(
        h.fs_read("/w/a.txt").as_deref(),
        Some(&b"xaaa\n"[..]),
        "a は保存された"
    );
    assert_eq!(h.fs_read("/w/b.txt").as_deref(), Some(&b"bbb\n"[..]));
}

// SAF-03: 保存の途中で閉じた (破棄を選んだ) 文書のスワップは、保存が終わって保存中の Snapshot を手放すまで残す
#[test]
fn 保存中に閉じた文書のスワップは保存が終わるまで残す() {
    let mut h = Harness::new().with_file("/w/a.txt", b"x");
    h.open("/w/a.txt");
    h.run_jobs();
    h.text("y");
    h.advance_ms(0); // 最後の入力の時刻を Editor に知らせる (swap.rs の typed と同じ)
    h.advance_ms(2000);
    h.run_jobs();
    assert_eq!(h.fs.swap_count(), 1, "スワップが書かれた");
    h.cmd("file.save");
    assert_eq!(h.pending_jobs(), 1, "保存ジョブが実行待ち");
    h.cmd("buffer.close");
    assert_eq!(h.overlay(), "dialog", "未保存の確認");
    h.text("d"); // 破棄して閉じる
    assert_eq!(
        h.editor.state().documents_len(),
        1,
        "閉じて無題の文書だけになった"
    );
    // 保存がまだ終わっていない間は、tick を進めてもスワップを消さない (強制終了・電源断に備える)
    h.advance_ms(60_000);
    assert_eq!(h.fs.swap_count(), 1, "保存中は残す");
    h.run_jobs();
    h.advance_ms(0);
    h.run_jobs();
    assert_eq!(
        h.fs_read("/w/a.txt").as_deref(),
        Some(&b"yx"[..]),
        "保存は完了した"
    );
    assert_eq!(h.fs.swap_count(), 0, "保存が終わって手放したら消す");
}

// SAF-05: 保存の警告 `ChangedAfterSave` を完了の後に知らせる
#[test]
fn 保存の直後に他のプログラムが変えた可能性を知らせる() {
    let mut h = Harness::new().with_file("/w/a.txt", b"x");
    h.open("/w/a.txt");
    h.run_jobs();
    h.text("y");
    h.fs.warn_next(vec![SaveWarning::ChangedAfterSave]);
    h.cmd("file.save");
    h.run_jobs();
    let m = message(&mut h);
    assert!(!h.is_dirty());
    assert!(
        m.contains("他のプログラムが変更した可能性"),
        "警告を出す: {m}"
    );
}

// SAF-40: Unix の新規ファイルの退路で一時ファイルを消せなかった (`TempNotRemoved`) ことを、そのパスとともに知らせる
#[test]
fn 一時ファイルを消せなかったことをパスとともに知らせる() {
    let mut h = Harness::new().with_file("/w/a.txt", b"x");
    h.open("/w/a.txt");
    h.run_jobs();
    h.text("y");
    // fsync の警告と重なっても隠れない (重さの順の先頭)
    h.fs.warn_next(vec![
        SaveWarning::FsyncFailed,
        SaveWarning::TempNotRemoved {
            path: eve_core::FsPath::from("/w/.a.txt.eve-tmp-1-00000000"),
        },
    ]);
    h.cmd("file.save");
    h.run_jobs();
    let m = message(&mut h);
    assert!(!h.is_dirty());
    assert!(
        m.contains("一時ファイル") && m.contains(".a.txt.eve-tmp-1-00000000"),
        "警告を出す: {m}"
    );
}

// SAF-06 / 17: すべて置換のジョブの最中に開き直すと、古い結果を適用せず、Revision も戻らない
#[test]
fn すべて置換のジョブの最中に開き直すと_古い結果を適用しない() {
    let big = "foo bar baz\n".repeat(90_000);
    let mut h = Harness::new()
        .with_size(60, 10)
        .with_file("/w/big.txt", big.as_bytes());
    h.open("/w/big.txt");
    h.run_jobs();
    let doc = h.doc_id();
    let before = h.editor.state().document(doc).unwrap().revision();
    h.keys("ctrl+r");
    h.text("foo");
    h.keys("tab");
    h.text("x");
    h.keys("alt+a");
    assert!(h.pending_jobs() >= 1, "置換はジョブで走る");
    h.keys("esc");
    h.cmd_with(
        "file.reopen_with_encoding",
        Args::new().with_str("encoding", "utf-8"),
    );
    if h.overlay() != "" {
        h.text("d"); // 確認が出たら破棄して開き直す
    }
    h.run_jobs();
    let d = h.editor.state().document(h.doc_id()).unwrap();
    assert!(
        d.to_string_lossy().starts_with("foo bar baz\nfoo"),
        "開き直した内容のまま (置換の古い結果を当てない)"
    );
    assert!(d.revision() >= before, "Revision は戻らない");
    assert!(!d.is_dirty());
}
