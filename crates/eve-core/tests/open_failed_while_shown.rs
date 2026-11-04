//! 読込中に表示へ切り替えたバッファの読込が失敗したとき、表示を開く前の文書へ戻す (docs/03 §9.2 の「失敗」・§9.8)。
//! fuzz_editor で見つけた不具合の回帰試験: 失敗したバッファを閉じるだけだと、Window が消えた View を指したままになり Frame を作れなかった。

use eve_core::FsErrorKind;
use eve_testkit::{FsOp, Harness};

// verifies: FR-103
#[test]
fn 読込中に表示したバッファの読込が失敗すると_開く前の文書を表示する() {
    let mut h = Harness::new()
        .with_file("/w/a.txt", b"alpha\n")
        .with_file("/w/b.txt", b"beta\n");
    h.open("/w/a.txt");
    h.run_jobs();
    h.text("x");
    let a = h.doc_id();
    // b を開き、読み終わる前にそのバッファへ切り替える
    h.open("/w/b.txt");
    h.cmd("buffer.next");
    assert_ne!(h.doc_id(), a, "読込中のバッファを表示している");
    h.fs.fail_next(FsOp::OpenRead, FsErrorKind::PermissionDenied);
    h.run_jobs();
    // 開く前の文書 (編集中の a) が表示され、Frame を作れる
    assert_eq!(h.doc_id(), a);
    assert_eq!(h.editor.state().documents_len(), 1);
    assert!(h.frame_text().contains("xalpha"));
}
