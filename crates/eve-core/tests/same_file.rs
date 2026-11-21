//! 同じファイルの判定 (SAF-28、docs/03 §12.7.6): 別の書き方のパス (シンボリックリンク・ハードリンク・`..`) で同じファイルを
//! 開くと 1 つの文書になる (03 §9.2 手順 2b)。名前を付けて保存の保存先の照合 (`CheckTarget`、03 §9.7 (a)) の表の各行。
//! 識別 (`FileId`) を信頼しない FS (`exFAT`・ネットワーク。`MemFs::set_file_id_unique(false)`) では canonical のパスで比べる。
//! 実 FS (大文字小文字・NTFS / `exFAT`・Linux) の試験は eve-platform の `tests/same_file.rs`。

use eve_core::{Args, DocumentId};
use eve_testkit::Harness;

fn docs(h: &Harness) -> usize {
    h.editor.state().documents_len()
}

/// 開いている文書のパス (表示の形、並びは文書の順)。
fn paths(h: &Harness) -> Vec<String> {
    let Ok(eve_view::Answer::Documents(ds)) = h.query(&eve_view::Query::Documents) else {
        panic!("文書の一覧");
    };
    ds.iter().filter_map(|d| d.path.clone()).collect()
}

fn opened(files: &[(&str, &[u8])]) -> Harness {
    let h = Harness::new();
    for (p, data) in files {
        h.fs.add_file(p, data);
    }
    h
}

fn save_as(h: &mut Harness, path: &str) {
    h.cmd_with("file.save_as", Args::new().with_str("path", path));
    h.run_jobs();
}

fn disk(h: &Harness, path: &str) -> Option<String> {
    h.fs_read(path)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
}

fn has_msg(h: &Harness, key: &str) -> bool {
    h.messages().iter().any(|m| m == key)
}

// ---------- 開く (03 §9.2 手順 2b) ----------

// verifies: FR-103
#[test]
fn シンボリックリンクで同じファイルを開くと既存の文書に切り替える() {
    let mut h = opened(&[("/w/a.txt", b"alpha\n")]);
    h.fs.add_link("/w/link.txt", "/w/a.txt");
    h.open("/w/a.txt");
    h.run_jobs();
    let a = h.doc_id();
    h.open("/w/link.txt");
    h.run_jobs();
    assert_eq!(docs(&h), 1, "1 つの文書: {:?}", paths(&h));
    assert_eq!(h.doc_id(), a);
    assert!(
        has_msg(&h, "info.open.already_open"),
        "今の文書なので「既に開いています」"
    );
}

// verifies: FR-103
#[test]
fn ハードリンクで同じファイルを開くと既存の文書を出す() {
    let mut h = opened(&[("/w/a.txt", b"alpha\n"), ("/w/b.txt", b"beta\n")]);
    h.fs.add_hardlink("/w/hard.txt", "/w/a.txt");
    h.open("/w/a.txt");
    h.run_jobs();
    let a = h.doc_id();
    h.open("/w/b.txt");
    h.run_jobs();
    assert_ne!(h.doc_id(), a);
    // 今の文書 (b) と違う既存の文書 (a) を、開いた時点のペインに出す
    h.open("/w/hard.txt");
    h.run_jobs();
    assert_eq!(docs(&h), 2, "a と b だけ: {:?}", paths(&h));
    assert_eq!(h.doc_id(), a, "既存の a を出す");
    assert!(
        !has_msg(&h, "info.open.already_open"),
        "今の文書ではないので知らせない"
    );
}

// verifies: FR-103
#[test]
fn 識別を信頼しない_fs_ではハードリンクは別の文書になる() {
    // exFAT・ネットワーク: 識別が使い回され・重なり得るので使わない (05 §5.4)。canonical のパスは違う
    let mut h = opened(&[("/w/a.txt", b"alpha\n")]);
    h.fs.set_file_id_unique(false);
    h.fs.add_hardlink("/w/hard.txt", "/w/a.txt");
    h.open("/w/a.txt");
    h.run_jobs();
    h.open("/w/hard.txt");
    h.run_jobs();
    assert_eq!(docs(&h), 2, "{:?}", paths(&h));
}

// verifies: FR-103
#[test]
fn 識別を信頼しない_fs_でも_canonical_のパスが同じなら_1_つの文書() {
    let mut h = opened(&[("/w/a.txt", b"alpha\n")]);
    h.fs.set_file_id_unique(false);
    h.fs.add_link("/w/link.txt", "/w/a.txt");
    h.open("/w/a.txt");
    h.run_jobs();
    h.open("/w/link.txt");
    h.run_jobs();
    assert_eq!(docs(&h), 1, "{:?}", paths(&h));
}

// verifies: FR-103
#[test]
fn 同時に開いた同じファイルは後から来たほうを閉じる() {
    let mut h = opened(&[("/w/a.txt", b"alpha\n")]);
    h.fs.add_link("/w/link.txt", "/w/a.txt");
    h.open("/w/a.txt");
    h.open("/w/link.txt");
    h.run_jobs();
    assert_eq!(docs(&h), 1, "{:?}", paths(&h));
    assert_eq!(paths(&h), ["/w/a.txt"]);
    assert_eq!(h.doc_text(), "alpha\n");
}

// verifies: FR-103, FR-127
#[test]
fn 確認を待つ文書も識別を持つ() {
    // 巨大ファイルの確認を待つ間 (読込中) に、別の書き方のパスで同じファイルを開いても 2 つ目の文書を作らない
    let mut h = opened(&[("/w/big.txt", b"x\n")]);
    h.fs.set_fake_len("/w/big.txt", Some(1 << 40));
    h.fs.add_link("/w/link.txt", "/w/big.txt");
    h.open("/w/big.txt");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog", "巨大ファイルの確認");
    h.keys("esc");
    // 確認を閉じた (中止) ので文書は閉じている。もう一度開いて確認を待つ間に、リンクを開く
    h.open("/w/big.txt");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog");
    let before = docs(&h);
    h.editor
        .state_mut()
        .open_path("/w/link.txt", eve_core::OpenOptions::default());
    h.run_jobs();
    assert_eq!(
        docs(&h),
        before,
        "確認を待つ文書と同じファイル: {:?}",
        paths(&h)
    );
}

// verifies: FR-103
#[test]
fn 新規のファイルも親の_canonical_とファイル名で比べる() {
    // まだ無いファイルを 2 通りの書き方 (リンクのディレクトリ経由) で開く
    let mut h = Harness::new();
    h.fs.add_dir("/w/real");
    h.fs.add_link("/w/alias", "/w/real");
    h.open("/w/real/new.txt");
    h.run_jobs();
    h.open("/w/alias/new.txt");
    h.run_jobs();
    let news: Vec<_> = paths(&h)
        .into_iter()
        .filter(|p| p.ends_with("new.txt"))
        .collect();
    assert_eq!(news.len(), 1, "{news:?}");
}

// verifies: FR-103
#[test]
fn 別のファイルは別の文書のまま() {
    let mut h = opened(&[("/w/a.txt", b"alpha\n"), ("/w/b.txt", b"beta\n")]);
    h.open("/w/a.txt");
    h.run_jobs();
    h.open("/w/b.txt");
    h.run_jobs();
    assert_eq!(docs(&h), 2);
}

// ---------- 名前を付けて保存 (03 §9.7 (a)) ----------

/// a.txt を開いて編集し (dirty)、b.txt も開いて表示を a に戻す。
fn a_dirty_and_b() -> (Harness, DocumentId, DocumentId) {
    let mut h = opened(&[("/w/a.txt", b"alpha\n"), ("/w/b.txt", b"beta\n")]);
    h.open("/w/b.txt");
    h.run_jobs();
    let b = h.doc_id();
    h.open("/w/a.txt");
    h.run_jobs();
    let a = h.doc_id();
    h.text("x");
    (h, a, b)
}

// verifies: FR-104
#[test]
fn 表の_1_今の文書と同じファイルへの名前を付けて保存は上書き保存になる() {
    let (mut h, a, _) = a_dirty_and_b();
    h.fs.add_link("/w/link.txt", "/w/a.txt");
    save_as(&mut h, "/w/link.txt");
    assert_eq!(h.overlay(), "", "上書きの確認を出さない");
    assert_eq!(disk(&h, "/w/a.txt").as_deref(), Some("xalpha\n"));
    assert_eq!(h.doc_id(), a);
    assert_eq!(h.doc_path().as_deref(), Some("/w/a.txt"), "パスは変えない");
    assert!(!h.is_dirty());
}

// verifies: FR-104
#[test]
fn 表の_1_ハードリンクも今の文書と同じファイル() {
    let (mut h, _, _) = a_dirty_and_b();
    h.fs.add_hardlink("/w/hard.txt", "/w/a.txt");
    save_as(&mut h, "/w/hard.txt");
    // 上書きの確認ではなく、今の文書 (a) の上書き保存のハードリンクの確認 (03 §9.3 手順 7)
    assert_eq!(h.overlay(), "dialog");
    h.answer(0); // リンクを保って直接上書き
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_eq!(disk(&h, "/w/a.txt").as_deref(), Some("xalpha\n"));
    assert_eq!(h.doc_path().as_deref(), Some("/w/a.txt"));
}

// verifies: FR-104, FR-116
#[test]
fn 表の_1_読み取り専用の文書は同じファイルに保存しない() {
    let mut h = opened(&[("/w/a.txt", b"alpha\n")]);
    h.fs.add_link("/w/link.txt", "/w/a.txt");
    h.open("/w/a.txt");
    h.run_jobs();
    h.cmd("file.toggle_read_only");
    save_as(&mut h, "/w/link.txt");
    assert!(has_msg(&h, "msg.save_as.same_file_read_only"));
    assert_eq!(h.overlay(), "prompt", "名前の入力に戻る");
    assert_eq!(disk(&h, "/w/a.txt").as_deref(), Some("alpha\n"));
}

// verifies: FR-104
#[test]
fn 表の_1b_ほかの開いている文書と同じファイルには保存しない() {
    let (mut h, a, _) = a_dirty_and_b();
    h.fs.add_link("/w/to-b.txt", "/w/b.txt");
    save_as(&mut h, "/w/to-b.txt");
    assert!(has_msg(&h, "msg.save_as.open_in_other"));
    assert_eq!(h.overlay(), "prompt", "名前の入力に戻る");
    assert_eq!(disk(&h, "/w/b.txt").as_deref(), Some("beta\n"), "書かない");
    assert_eq!(h.doc_id(), a);
    assert!(h.is_dirty());
    // 構文的に同じパスでも同じ (照合のジョブを経る)
    h.keys("esc");
    save_as(&mut h, "/w/b.txt");
    assert!(has_msg(&h, "msg.save_as.open_in_other"));
    assert_eq!(disk(&h, "/w/b.txt").as_deref(), Some("beta\n"));
}

// verifies: FR-104
#[test]
fn 表の_1b_識別を信頼しない_fs_でもリンクは_canonical_で見つける() {
    let (mut h, _, _) = a_dirty_and_b();
    h.fs.set_file_id_unique(false);
    h.fs.add_link("/w/to-b.txt", "/w/b.txt");
    save_as(&mut h, "/w/to-b.txt");
    assert!(has_msg(&h, "msg.save_as.open_in_other"));
    assert_eq!(disk(&h, "/w/b.txt").as_deref(), Some("beta\n"));
}

// verifies: FR-104
#[test]
fn 表の_1b_まだ無いファイルとして開いた文書とも比べる() {
    let (mut h, _, _) = a_dirty_and_b();
    h.open("/w/new.txt");
    h.run_jobs();
    h.cmd("buffer.prev");
    save_as(&mut h, "/w/new.txt");
    assert!(has_msg(&h, "msg.save_as.open_in_other"));
    assert!(disk(&h, "/w/new.txt").is_none(), "作らない");
}

// verifies: FR-104
#[test]
fn 表の_2_無いファイルには保存して文書のパスを替える() {
    let (mut h, a, _) = a_dirty_and_b();
    save_as(&mut h, "/w/c.txt");
    assert_eq!(h.overlay(), "");
    assert_eq!(disk(&h, "/w/c.txt").as_deref(), Some("xalpha\n"));
    assert_eq!(h.doc_id(), a);
    assert_eq!(h.doc_path().as_deref(), Some("/w/c.txt"));
    // 保存した後は新しいパスで同じファイルと判定する (canonical を替えた)。古いパスは別のファイル
    h.fs.add_link("/w/to-c.txt", "/w/c.txt");
    h.open("/w/to-c.txt");
    h.run_jobs();
    assert_eq!(h.doc_id(), a, "c の文書に切り替える");
    h.open("/w/a.txt");
    h.run_jobs();
    assert_ne!(h.doc_id(), a, "a.txt は別の文書として開く");
}

// verifies: FR-106
#[test]
fn 表の_3_ダングリングのリンクには保存しない() {
    let (mut h, _, _) = a_dirty_and_b();
    h.fs.add_link("/w/dangling.txt", "/w/nowhere.txt");
    save_as(&mut h, "/w/dangling.txt");
    assert!(has_msg(&h, "msg.save.dangling_link"));
    assert_eq!(h.overlay(), "prompt");
    assert!(disk(&h, "/w/nowhere.txt").is_none());
}

// verifies: FR-104
#[test]
fn 表の_4_フォルダには保存しない() {
    let (mut h, _, _) = a_dirty_and_b();
    h.fs.add_dir("/w/dir");
    save_as(&mut h, "/w/dir");
    assert!(has_msg(&h, "msg.save_as.not_a_file"));
    assert_eq!(h.overlay(), "prompt");
}

// verifies: FR-104, FR-116
#[test]
fn 表の_5_読み取り専用属性のファイルは上書きの確認の前に拒む() {
    let (mut h, _, _) = a_dirty_and_b();
    h.fs.add_file("/w/ro.txt", b"keep\n");
    h.fs.set_readonly("/w/ro.txt", true);
    save_as(&mut h, "/w/ro.txt");
    assert!(has_msg(&h, "msg.save_as.target_read_only"));
    assert_eq!(h.overlay(), "prompt");
    assert_eq!(disk(&h, "/w/ro.txt").as_deref(), Some("keep\n"));
}

// verifies: FR-104
#[test]
fn 表の_6_既存のファイルは上書きを確かめる() {
    let (mut h, a, _) = a_dirty_and_b();
    h.fs.add_file("/w/c.txt", b"old\n");
    save_as(&mut h, "/w/c.txt");
    assert_eq!(h.overlay(), "dialog", "上書きの確認");
    assert_eq!(
        disk(&h, "/w/c.txt").as_deref(),
        Some("old\n"),
        "確認の前は書かない"
    );
    // 既定は「別の名前を入力」: Enter で名前の入力に戻る (入力した名前は残す)
    h.keys("enter");
    assert_eq!(h.overlay(), "prompt");
    assert_eq!(disk(&h, "/w/c.txt").as_deref(), Some("old\n"));
    // Esc も名前の入力に戻る
    h.keys("enter");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog");
    h.keys("esc");
    assert_eq!(h.overlay(), "prompt");
    // 上書き
    h.keys("enter");
    h.run_jobs();
    h.keys("o");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_eq!(disk(&h, "/w/c.txt").as_deref(), Some("xalpha\n"));
    assert_eq!(h.doc_id(), a);
    assert_eq!(h.doc_path().as_deref(), Some("/w/c.txt"));
}

// verifies: FR-104, FR-117
#[test]
fn 表の_6_確認の間に変わったファイルは上書きせずに尋ねる() {
    let (mut h, _, _) = a_dirty_and_b();
    h.fs.add_file("/w/c.txt", b"old\n");
    save_as(&mut h, "/w/c.txt");
    assert_eq!(h.overlay(), "dialog");
    // 確認の間に他のプログラムが書き換えた
    h.advance_ms(10);
    h.fs.add_file("/w/c.txt", b"changed by other\n");
    h.keys("o");
    h.run_jobs();
    assert_eq!(
        disk(&h, "/w/c.txt").as_deref(),
        Some("changed by other\n"),
        "照合で止めて書かない"
    );
    assert_eq!(h.overlay(), "dialog", "外部変更の確認");
}

// verifies: FR-104
#[test]
fn 表の_7_調べられない保存先には保存しない() {
    let (mut h, _, _) = a_dirty_and_b();
    h.fs.fail_next(
        eve_testkit::FsOp::Metadata,
        eve_core::FsErrorKind::PermissionDenied,
    );
    save_as(&mut h, "/w/c.txt");
    assert!(has_msg(&h, "msg.save.permission_denied"));
    assert_eq!(h.overlay(), "prompt");
    assert!(disk(&h, "/w/c.txt").is_none());
}

// verifies: FR-104
#[test]
fn 照合を待つ間の_esc_は名前の入力に戻る() {
    let (mut h, _, _) = a_dirty_and_b();
    h.cmd_with("file.save_as", Args::new().with_str("path", "/w/c.txt"));
    assert_eq!(
        h.overlay(),
        "",
        "照合のジョブを待つ間はオーバーレイを出さない"
    );
    h.keys("esc");
    assert_eq!(h.overlay(), "prompt");
    h.run_jobs();
    assert!(disk(&h, "/w/c.txt").is_none(), "照合の結果は捨てる");
    assert_eq!(h.overlay(), "prompt");
}

// verifies: FR-104
#[test]
fn 照合を待つ間の上書き保存は無視する() {
    let (mut h, _, _) = a_dirty_and_b();
    h.cmd_with("file.save_as", Args::new().with_str("path", "/w/c.txt"));
    h.cmd("file.save");
    assert!(has_msg(&h, "info.save.in_progress"));
    h.run_jobs();
    assert_eq!(
        disk(&h, "/w/a.txt").as_deref(),
        Some("alpha\n"),
        "a は書かない"
    );
    assert_eq!(disk(&h, "/w/c.txt").as_deref(), Some("xalpha\n"));
}

// verifies: FR-104
#[test]
fn 照合から保存の完了までに同じファイルの文書が開かれたら知らせて両方残す() {
    let (mut h, a, _) = a_dirty_and_b();
    // 照合 (c は無い) の結果が届く前に c を開く: 開く読込の識別 (新規のファイル) は保存の前に届くので、その時点では
    // 別のファイル。保存が終わった時点で同じファイルになる
    h.cmd_with("file.save_as", Args::new().with_str("path", "/w/c.txt"));
    h.editor
        .state_mut()
        .open_path("/w/c.txt", eve_core::OpenOptions::default());
    h.run_jobs();
    assert_eq!(disk(&h, "/w/c.txt").as_deref(), Some("xalpha\n"));
    assert!(
        has_msg(&h, "msg.save_as.open_in_other"),
        "{:?}",
        h.messages()
    );
    assert!(
        h.editor.state().document(a).is_some(),
        "保存した文書は閉じない"
    );
}
