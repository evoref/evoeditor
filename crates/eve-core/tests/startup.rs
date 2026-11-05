//! 起動時に複数のファイルを開く・開いた直後の位置への移動 (docs/07 §18.2、01 FR-101 / FR-102)。

use eve_core::InitialFile;
use eve_testkit::Harness;
use eve_testkit::helpers::cell_text;
use eve_view::StatusKind;

/// ファイルを置いた Harness (まだ何も開いていない)。
fn env() -> Harness {
    Harness::builder()
        .file(
            "/w/a.txt",
            b"aaa
bbb
ccc
",
        )
        .file(
            "/w/b.txt",
            b"b1
b2
b3
",
        )
        .file(
            "/w/c.txt",
            "日本語
あいうえお
"
            .as_bytes(),
        )
        .defer_start()
        .build()
}

fn start(h: &mut Harness, files: &[InitialFile], read_only: bool) {
    h.editor.open_initial_files(files, read_only, None);
    h.started();
    h.run_jobs();
}

fn status(h: &mut Harness, kind: StatusKind) -> String {
    let f = h.frame();
    let item = f.status.items.iter().find(|i| i.kind == kind).unwrap();
    cell_text(&item.long)
}

fn file(path: &str) -> InitialFile {
    InitialFile {
        path: path.to_owned(),
        line: None,
        col: None,
    }
}

fn at(path: &str, line: usize, col: Option<usize>) -> InitialFile {
    InitialFile {
        path: path.to_owned(),
        line: Some(line),
        col,
    }
}

#[test]
fn 複数のパスを渡すと_その数のバッファが開き_最初のものが表示される() {
    let mut s = env();
    start(
        &mut s,
        &[file("/w/a.txt"), file("/w/b.txt"), file("/w/c.txt")],
        false,
    );
    assert_eq!(s.editor.state().documents_len(), 3);
    assert_eq!(status(&mut s, StatusKind::Path), "a.txt (1/3)");
    s.cmd("buffer.next");
    assert_eq!(status(&mut s, StatusKind::Path), "b.txt (2/3)");
    s.cmd("buffer.next");
    assert_eq!(status(&mut s, StatusKind::Path), "c.txt (3/3)");
    // 全部読み終わっていて、編集できる
    let id = s.editor.state().active_document().unwrap();
    assert_eq!(
        s.editor.state().document(id).unwrap().to_string_lossy(),
        "日本語\nあいうえお\n"
    );
}

#[test]
fn 読込が終わる前でも_最初のファイルを表示し_追加のバッファへ切り替えられる() {
    let mut s = env();
    s.editor
        .open_initial_files(&[file("/w/a.txt"), file("/w/b.txt")], false, None);
    s.started();
    // ジョブはまだ走っていない
    assert_eq!(s.editor.state().documents_len(), 2);
    s.cmd("buffer.next");
    s.cmd("buffer.prev");
    assert_eq!(s.editor.state().documents_len(), 2, "読込中でも切替できる");
    s.run_jobs();
    assert_eq!(status(&mut s, StatusKind::Path), "a.txt (1/2)");
}

#[test]
fn 同じパスは_1_つのバッファだけ開く() {
    let mut s = env();
    start(
        &mut s,
        &[file("/w/a.txt"), file("/w/b.txt"), file("/w/b.txt")],
        false,
    );
    assert_eq!(s.editor.state().documents_len(), 2);
}

#[test]
fn readonly_は全パスに掛かる() {
    let mut s = env();
    start(&mut s, &[file("/w/a.txt"), file("/w/b.txt")], true);
    for _ in 0..2 {
        let id = s.editor.state().active_document().unwrap();
        assert!(
            !s.editor.state().document(id).unwrap().read_only.is_empty(),
            "読み取り専用"
        );
        s.cmd("buffer.next");
    }
}

#[test]
fn 開いた直後に指定の行と桁へ移る() {
    let mut s = env();
    start(
        &mut s,
        &[
            at("/w/a.txt", 2, Some(2)),
            at("/w/b.txt", 3, None),
            at("/w/c.txt", 2, Some(3)),
        ],
        false,
    );
    assert_eq!(status(&mut s, StatusKind::Position), "行 2, 桁 2");
    s.cmd("buffer.next");
    assert_eq!(
        status(&mut s, StatusKind::Position),
        "行 3, 桁 1",
        "桁を省くと 1"
    );
    s.cmd("buffer.next");
    assert_eq!(
        status(&mut s, StatusKind::Position),
        "行 2, 桁 3",
        "桁は書記素の数 (あい|う)"
    );
}

#[test]
fn 範囲外の行と桁は末尾に丸める() {
    let mut s = env();
    start(&mut s, &[at("/w/a.txt", 99, Some(99))], false);
    // 末尾の改行の後の空行 (4 行目) の先頭
    assert_eq!(status(&mut s, StatusKind::Position), "行 4, 桁 1");

    let mut s = env();
    start(&mut s, &[at("/w/a.txt", 1, Some(99))], false);
    assert_eq!(status(&mut s, StatusKind::Position), "行 1, 桁 4", "行末");
}

#[test]
fn 追加のファイルが開けなくても_表示中のバッファはそのまま() {
    let mut s = env();
    // フォルダは開けない
    start(&mut s, &[file("/w/a.txt"), file("/w")], false);
    assert_eq!(
        s.editor.state().documents_len(),
        1,
        "開けなかったものは閉じる"
    );
    assert_eq!(status(&mut s, StatusKind::Path), "a.txt");
}
