//! AC の棚卸し (M5) の穴埋め その 3: 開く前の警告 (巨大ファイル・不正バイトの比率が高いファイル)。

use eve_testkit::Harness;
use eve_view::OverlayModel;

fn ct(t: &eve_view::CellText) -> String {
    t.spans.iter().map(|s| s.text.as_str()).collect()
}

/// 開くときの確認ダイアログの (メッセージ, 選択肢のラベル)。
fn dialog(h: &mut Harness) -> (String, Vec<String>) {
    assert_eq!(h.overlay(), "dialog");
    let f = h.frame();
    let OverlayModel::Confirm {
        message, choices, ..
    } = &f.overlays[0]
    else {
        panic!("確認ダイアログではない");
    };
    (ct(message), choices.iter().map(|c| ct(&c.label)).collect())
}

// verifies: AC-FR-127-1
#[test]
fn 巨大なファイルは読む前に見込みを示して_読み取り専用で開くか中止かを選ばせる() {
    let mut h = Harness::new()
        .with_size(80, 6)
        .with_file("/w/big.txt", b"abc\n");
    // 中身は小さいまま、メタデータの長さだけ 512 MiB にする
    h.fs.set_fake_len("/w/big.txt", Some(512 * 1024 * 1024));
    h.open("/w/big.txt");
    h.run_jobs();
    let (message, labels) = dialog(&mut h);
    assert!(
        message.contains("512"),
        "メモリ使用量の見込み (サイズ) を示す: {message}"
    );
    assert_eq!(
        labels,
        ["[R] 読み取り専用で開く", "[Esc] キャンセル"],
        "文字コードの指定は選択肢にない"
    );
    // 中止: 何も開かれない
    h.keys("esc");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_ne!(h.doc_path().as_deref(), Some("/w/big.txt"));
    // 読み取り専用で開く
    h.open("/w/big.txt");
    h.run_jobs();
    h.keys("r");
    h.run_jobs();
    assert_eq!(h.doc_path().as_deref(), Some("/w/big.txt"));
    assert_eq!(h.doc_text(), "abc\n");
    h.text("x");
    assert_eq!(
        h.doc_text(),
        "abc
",
        "読み取り専用なので編集は効かない"
    );
}

// verifies: AC-FR-127-1
#[test]
fn 閾値未満のファイルは警告なしで開く() {
    let mut h = Harness::new()
        .with_size(80, 6)
        .with_file("/w/a.txt", b"abc\n");
    h.fs.set_fake_len("/w/a.txt", Some(512 * 1024 * 1024 - 1));
    h.open("/w/a.txt");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_eq!(h.doc_path().as_deref(), Some("/w/a.txt"));
}

// verifies: AC-FR-127-2
#[test]
fn 不正バイトが_1_パーセントを超えると見込みを示して_3_択を出す() {
    // 約 2% (100 バイトに 2 バイト) が UTF-8 として不正
    let mut body = Vec::new();
    for _ in 0..50 {
        body.extend_from_slice(b"abcdefghi\xff");
        body.extend_from_slice(b"abcdefghij");
    }
    let mut h = Harness::new().with_size(80, 6).with_file("/w/a.txt", &body);
    h.open("/w/a.txt");
    h.run_jobs();
    let (message, labels) = dialog(&mut h);
    assert!(
        message.contains('%') && message.contains("倍"),
        "割合とメモリ使用量の見込みを示す: {message}"
    );
    assert_eq!(
        labels,
        [
            "[R] 読み取り専用で開く",
            "[E] 文字コードを指定して開く",
            "[C] 中止"
        ]
    );
    // Esc も同じ中止 (04 §15.2): 何も開かれない
    h.keys("esc");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_ne!(h.doc_path().as_deref(), Some("/w/a.txt"));
}
