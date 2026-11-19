//! 開く前の警告 (AC の棚卸しの穴埋め): 開く前の警告 (巨大ファイル・不正バイトの比率が高いファイル)。

use eve_core::{Args, Encoding, OpenOptions, ReadOnlyReason};
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

// verifies: FR-127
#[test]
fn 警告の閾値は_file_huge_file_warn_bytes_で変えられる() {
    let mut h = Harness::new()
        .with_size(80, 6)
        .with_file("/w/a.txt", b"abc\n")
        .with_file("/w/b.txt", b"abc\n");
    h.editor.state_mut().set_config(eve_config::Config {
        huge_file_warn_bytes: 2 * 1024 * 1024,
        ..eve_config::Config::default()
    });
    h.fs.set_fake_len("/w/a.txt", Some(2 * 1024 * 1024));
    h.open("/w/a.txt");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog", "設定した閾値で警告する");
    h.keys("esc");
    h.run_jobs();
    h.fs.set_fake_len("/w/b.txt", Some(2 * 1024 * 1024 - 1));
    h.open("/w/b.txt");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_eq!(h.doc_path().as_deref(), Some("/w/b.txt"));
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

// verifies: NFR-203
// spec: 03 §9.2 (読込の確保は try_reserve。失敗は msg.open.out_of_memory で、プロセスは続ける)
#[test]
fn 読込の確保に失敗したら落ちずに中止して知らせる() {
    let mut h = Harness::new()
        .with_size(80, 6)
        .with_file("/w/big.txt", b"abc\n");
    // 中身は小さいまま、確保できない大きさ (1 EiB) に見せる
    h.fs.set_fake_len("/w/big.txt", Some(1 << 60));
    h.open("/w/big.txt");
    h.run_jobs();
    h.keys("r"); // 巨大ファイルの確認: 読み取り専用で開く
    h.run_jobs();
    assert!(
        h.messages().contains(&"msg.open.out_of_memory".to_owned()),
        "{:?}",
        h.messages()
    );
    assert_ne!(h.doc_path().as_deref(), Some("/w/big.txt"), "開かない");
    assert!(h.frame_contains("メモリが足りない"));
}

// verifies: NFR-203
// spec: 03 §9.5 手順 4 (再読込の失敗は旧本文・履歴をそのまま保つ)
#[test]
fn 再読込の確保に失敗したら旧本文のまま知らせる() {
    let mut h = Harness::new()
        .with_size(80, 6)
        .with_file("/w/a.txt", b"old\n");
    h.open("/w/a.txt");
    h.run_jobs();
    h.fs.set_fake_len("/w/a.txt", Some(1 << 60));
    h.cmd("file.reload");
    h.run_jobs();
    h.keys("r"); // 巨大ファイルの確認
    h.run_jobs();
    assert!(
        h.messages().contains(&"msg.open.out_of_memory".to_owned()),
        "{:?}",
        h.messages()
    );
    assert_eq!(h.doc_text(), "old\n");
    assert_eq!(h.doc_path().as_deref(), Some("/w/a.txt"));
    h.text("x");
    assert_eq!(h.doc_text(), "xold\n", "読込中の状態が残らず、編集できる");
}

// ---------- 確認で「読み取り専用で開く」を選んだときの理由 (03 §9.2 の「確認からの再投入」の表、§9.6) ----------

/// 今の文書の読み取り専用の理由 (表示の優先順)。
fn reasons(h: &Harness) -> Vec<ReadOnlyReason> {
    let id = h.doc_id();
    h.editor
        .state()
        .document(id)
        .unwrap()
        .read_only
        .iter()
        .collect()
}

/// `Shift_JIS` の「あいう」と改行。UTF-8 として読むと 7 バイトのうち 6 バイトが不正 (比率の確認が出る)。
const SJIS_AIU: &[u8] = b"\x82\xA0\x82\xA2\x82\xA4\n";

/// 文字コードを指定して開く (`--encoding` と同じ入口)。`user_read_only` は `--readonly`。
fn open_forced(h: &mut Harness, path: &str, enc: Encoding, user_read_only: bool) {
    h.editor.state_mut().open_path(
        path,
        OpenOptions {
            forced: Some(enc),
            user_read_only,
            ..OpenOptions::default()
        },
    );
    h.run_jobs();
}

// verifies: FR-127, FR-111
#[test]
fn 不正バイトの比率の確認で読み取り専用で開いた文書の理由は_invalid_bytes_だけで_正しい文字コードで開き直すと編集できる()
 {
    let mut h = Harness::new()
        .with_size(80, 6)
        .with_file("/w/a.txt", SJIS_AIU);
    open_forced(&mut h, "/w/a.txt", Encoding::Utf8, false);
    assert_eq!(h.overlay(), "dialog", "不正バイトの比率の確認");
    h.keys("r");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    // `UserRequested` は `--readonly` だけの理由なので付かない
    assert_eq!(reasons(&h), [ReadOnlyReason::InvalidBytes]);
    let id = h.doc_id();
    assert_eq!(
        h.editor.state().document(id).unwrap().read_only.primary(),
        Some(ReadOnlyReason::InvalidBytes),
        "表示の理由 (ステータスバーの理由) も InvalidBytes"
    );
    // 正しい文字コードで開き直すと不正バイトが 0 になり、読み取り専用が外れる (03 §8.8 の「読み取り専用の理由の差し替え」)
    h.cmd_with(
        "file.reopen_with_encoding",
        Args::new().with_str("encoding", "shift_jis"),
    );
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_eq!(h.doc_text(), "あいう\n");
    let id = h.doc_id();
    let d = h.editor.state().document(id).unwrap();
    assert_eq!(d.format.encoding, Encoding::ShiftJis);
    assert_eq!(d.format.invalid_bytes, 0);
    assert!(reasons(&h).is_empty(), "{:?}", reasons(&h));
    h.text("x");
    assert_eq!(h.doc_text(), "xあいう\n", "編集できる");
}

// verifies: FR-127, FR-111
#[test]
fn readonly_で開いた文書は比率の確認を経ても_user_requested_を持ち_開き直しでも保つ() {
    let mut h = Harness::new()
        .with_size(80, 6)
        .with_file("/w/a.txt", SJIS_AIU);
    open_forced(&mut h, "/w/a.txt", Encoding::Utf8, true);
    assert_eq!(h.overlay(), "dialog", "--readonly でも確認は通常どおり出す");
    h.keys("r");
    h.run_jobs();
    assert_eq!(
        reasons(&h),
        [ReadOnlyReason::UserRequested, ReadOnlyReason::InvalidBytes]
    );
    h.cmd_with(
        "file.reopen_with_encoding",
        Args::new().with_str("encoding", "shift_jis"),
    );
    h.run_jobs();
    assert_eq!(h.doc_text(), "あいう\n");
    // 不正バイトの理由は外れ、利用者の指定は保つ
    assert_eq!(reasons(&h), [ReadOnlyReason::UserRequested]);
    h.text("x");
    assert_eq!(h.doc_text(), "あいう\n", "読み取り専用のまま");
}

// verifies: FR-115
#[test]
fn バイナリの確認で読み取り専用で開いた文書の理由は_binary_だけ() {
    let mut h = Harness::new()
        .with_size(80, 6)
        .with_file("/w/bin.dat", b"a\0b\0c\0\n");
    h.open("/w/bin.dat");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog", "バイナリの確認");
    h.keys("r");
    h.run_jobs();
    assert_eq!(reasons(&h), [ReadOnlyReason::Binary]);
}

// verifies: FR-115
#[test]
fn utf32_の_bom_の確認で読み取り専用で開いた文書に_user_requested_は付かない() {
    let mut h = Harness::new()
        .with_size(80, 6)
        .with_file("/w/a.txt", b"\xFF\xFE\x00\x00a\x00\x00\x00");
    h.open("/w/a.txt");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog", "UTF-32 の BOM の確認");
    h.keys("r");
    h.run_jobs();
    // 確認の理由は `Binary`。UTF-8 として読んだ BOM のバイト (FF FE) は不正バイトなので、読込の結果 (`Complete.detected`)
    // が `InvalidBytes` も付ける (03 §9.2 の「付く読み取り専用の理由の決め方」)
    assert_eq!(
        reasons(&h),
        [ReadOnlyReason::InvalidBytes, ReadOnlyReason::Binary]
    );
}

// verifies: FR-127
#[test]
fn 巨大ファイルの確認で読み取り専用で開いた文書の理由は_large_file_viewer_だけ() {
    let mut h = Harness::new()
        .with_size(80, 6)
        .with_file("/w/big.txt", b"abc\n");
    h.fs.set_fake_len("/w/big.txt", Some(512 * 1024 * 1024));
    h.open("/w/big.txt");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog", "巨大ファイルの確認");
    h.keys("r");
    h.run_jobs();
    assert_eq!(reasons(&h), [ReadOnlyReason::LargeFileViewer]);
}
