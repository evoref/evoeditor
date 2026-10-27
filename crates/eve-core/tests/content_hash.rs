//! 内容ハッシュ (XXH3-64) による外部変更の検知 (docs/05 §5.5、01 FR-117): mtime と長さが同じ書き換えを見つけ、
//! 内容が同じで mtime だけが違うものを変更とみなさない。16 MiB 以下のファイルだけ。

use eve_testkit::Harness;

fn opened(body: &[u8]) -> Harness {
    let mut h = Harness::new().with_size(60, 6).with_file("/w/a.txt", body);
    h.open("/w/a.txt");
    h.run_jobs();
    h
}

fn stamp_hash(h: &Harness) -> Option<eve_core::ContentHash> {
    h.editor
        .state()
        .document(h.doc_id())
        .unwrap()
        .disk_stamp
        .unwrap()
        .hash
}

// verifies: AC-FR-117-1
#[test]
fn 開くと内容ハッシュが付き_16_mib_を超えると付かない() {
    let h = opened(b"abc\n");
    assert!(stamp_hash(&h).is_some());
    let mut h = Harness::new()
        .with_size(60, 6)
        .with_file("/w/big.txt", b"abc\n");
    h.fs.set_fake_len("/w/big.txt", Some(20 * 1024 * 1024));
    h.open("/w/big.txt");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert!(stamp_hash(&h).is_none(), "16 MiB 超はハッシュを使わない");
}

// verifies: AC-FR-117-1
#[test]
fn mtime_も長さも同じ書き換えは保存の前に見つけて尋ねる() {
    let mut h = opened(b"abc\n");
    h.fs.set_data_keep_mtime("/w/a.txt", b"xyz\n");
    h.text("Q");
    h.cmd("file.save");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog", "外部で変更されています");
    assert_eq!(h.fs_read("/w/a.txt").unwrap(), b"xyz\n", "まだ書いていない");
}

// verifies: AC-FR-117-1
#[test]
fn 内容が同じで_mtime_だけが違うなら尋ねずに保存する() {
    let mut h = opened(b"abc\n");
    h.fs.bump_mtime("/w/a.txt", 30);
    h.text("Q");
    h.cmd("file.save");
    h.run_jobs();
    assert_eq!(h.overlay(), "", "確認は出ない");
    assert_eq!(h.fs_read("/w/a.txt").unwrap(), b"Qabc\n");
    // 続けてもう一度編集して保存できる (新しい mtime の Stamp が引き継がれている)
    h.text("R");
    h.cmd("file.save");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_eq!(h.fs_read("/w/a.txt").unwrap(), b"QRabc\n");
}

// verifies: AC-FR-117-1
#[test]
fn 保存した内容のハッシュで_次の保存でも同じ書き換えを見つける() {
    let mut h = opened(b"abc\n");
    h.text("Q");
    h.cmd("file.save");
    h.run_jobs();
    assert_eq!(h.fs_read("/w/a.txt").unwrap(), b"Qabc\n");
    assert!(
        stamp_hash(&h).is_some(),
        "保存した内容のハッシュが Stamp に入る"
    );
    // 保存の後、同じ mtime・同じ長さで別の内容に書き換えられた
    h.fs.set_data_keep_mtime("/w/a.txt", b"Zabc\n");
    h.text("R");
    h.cmd("file.save");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog");
    assert_eq!(h.fs_read("/w/a.txt").unwrap(), b"Zabc\n");
}

// verifies: AC-FR-117-1
#[test]
fn 未編集の保存は_mtime_だけの変化なら変更なしとして書かない() {
    let mut h = opened(b"abc\n");
    h.fs.bump_mtime("/w/a.txt", 30);
    h.clear_log();
    h.cmd("file.save");
    h.run_jobs();
    assert!(h.messages().contains(&"info.save.unchanged".to_owned()));
    assert_eq!(h.overlay(), "");
}

// verifies: AC-FR-117-1
#[test]
fn 未編集の保存でも_mtime_が同じで中身が違えば尋ねる() {
    let mut h = opened(b"abc\n");
    h.fs.set_data_keep_mtime("/w/a.txt", b"xyz\n");
    h.cmd("file.save");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog");
    assert_eq!(h.fs_read("/w/a.txt").unwrap(), b"xyz\n");
}
