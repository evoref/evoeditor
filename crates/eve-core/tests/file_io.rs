//! ファイルを開く・保存するシナリオ (docs/03 §9.2 / §9.3、01 の FR-104 / FR-124 / FR-204 / NFR-303)。
//! 保存先は `MemFs`、ジョブは `ManualJobSpawner` (テストが `run_jobs` で実行する)。

use eve_core::{Args, CommandResult, Document, FsPath, RejectReason, TextFormat};
use eve_testkit::Harness;
use eve_view::{DocumentId, EditorEvent};

/// 起動していない Harness (文書は `open` / `open_initial` で開く)。
fn env() -> Harness {
    Harness::builder().defer_start().build()
}

/// ジョブを実行して結果を Editor に渡す。
fn run(h: &mut Harness, id: &str, args: Args) -> CommandResult {
    let r = h.cmd_with(id, args);
    h.run_jobs();
    r
}

fn open(h: &mut Harness, path: &str) -> CommandResult {
    run(h, "file.open", Args::new().with_str("path", path))
}

fn doc(h: &Harness) -> (DocumentId, &Document) {
    let id = h.editor.state().active_document().expect("文書がある");
    (id, h.editor.state().document(id).unwrap())
}

fn type_text(h: &mut Harness, s: &str) {
    h.text(s);
}

fn keys(h: &mut Harness, seq: &str) {
    h.keys(seq);
}

fn frame(h: &mut Harness) -> eve_view::Frame {
    h.frame()
}

/// これまでに出たメッセージの MessageKey (読むと捨てる)。
fn messages(h: &mut Harness) -> Vec<String> {
    let m = h.messages();
    h.clear_log();
    m
}

/// これまでの `EditorEvent` (読むと捨てる)。
fn drain_events(h: &mut Harness) -> Vec<EditorEvent> {
    let e = h.events().to_vec();
    h.clear_log();
    e
}

#[test]
fn utf8_のファイルを開く() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", "こんにちは\nworld\n".as_bytes());
    assert!(matches!(
        open(&mut s, "/w/a.txt"),
        CommandResult::Pending(_)
    ));
    assert_eq!(s.doc_text(), "こんにちは\nworld\n");
    let (_, d) = doc(&s);
    assert_eq!(d.path, Some(FsPath::from("/w/a.txt")));
    assert!(!d.is_dirty() && d.is_editable());
    assert!(d.disk_stamp.is_some());
    assert!(!d.format.has_bom);
}

// verifies: AC-FR-108-1
#[test]
fn bom_と_crlf_を保持して保存する() {
    let mut s = env();
    let src = b"\xEF\xBB\xBFab\r\ncd\r\n";
    s.fs.add_file("/w/a.txt", src);
    open(&mut s, "/w/a.txt");
    let (_, d) = doc(&s);
    assert_eq!(d.to_string_lossy(), "ab\r\ncd\r\n");
    assert!(d.format.has_bom);
    assert_eq!(d.format.default_eol, eve_text::LineEnding::Crlf);
    assert_eq!(d.format.eol_stats.crlf, 2);
    // 末尾に入力して保存: BOM と CRLF がそのまま残る。Enter は CRLF を入れる
    s.cmd("cursor.doc_end");
    type_text(&mut s, "x");
    keys(&mut s, "enter");
    run(&mut s, "file.save", Args::new());
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        b"\xEF\xBB\xBFab\r\ncd\r\nx\r\n"
    );
}

// verifies: AC-FR-104-3
#[test]
fn 無編集の上書き保存は書き込まず_変更なしと示す() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc");
    open(&mut s, "/w/a.txt");
    messages(&mut s);
    let before = s.fs.stamp("/w/a.txt");
    s.clock.advance(eve_core::Millis(5000));
    run(&mut s, "file.save", Args::new());
    assert_eq!(
        s.fs.stamp("/w/a.txt"),
        before,
        "mtime も内容も変わらない (FR-104 AC-3)"
    );
    assert!(messages(&mut s).contains(&"info.save.unchanged".to_owned()));
    assert!(s.fs.steps().is_empty(), "書込のステップに入らない");
}

#[test]
fn ディスクが変わっていれば未編集でも保存へ進む() {
    // 未編集でディスク上のファイルが外部で変わった: 上書きは Conflict になり、何も書かない
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc");
    open(&mut s, "/w/a.txt");
    messages(&mut s);
    s.clock.advance(eve_core::Millis(10));
    s.fs.set_file(&FsPath::from("/w/a.txt"), b"EXTERNAL");
    run(&mut s, "file.save", Args::new());
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        b"EXTERNAL",
        "外部の変更を上書きしない"
    );
    assert!(!messages(&mut s).contains(&"info.save.unchanged".to_owned()));
}

// verifies: AC-FR-204-2
#[test]
fn 編集して保存すると履歴が残り_undo_で_dirty_に戻る() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc");
    open(&mut s, "/w/a.txt");
    s.cmd("cursor.doc_end");
    type_text(&mut s, "d");
    assert!(doc(&s).1.is_dirty());
    messages(&mut s);
    run(&mut s, "file.save", Args::new());
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"abcd");
    assert!(!doc(&s).1.is_dirty());
    let evs = drain_events(&mut s);
    assert!(
        evs.iter()
            .any(|e| matches!(e, EditorEvent::DocumentSaved { path, .. } if path == "/w/a.txt"))
    );
    // 保存で履歴を消さない (FR-204): Undo で保存前の内容に戻り、dirty になる
    run(&mut s, "edit.undo", Args::new());
    assert_eq!(s.doc_text(), "abc");
    assert!(doc(&s).1.is_dirty());
    run(&mut s, "edit.redo", Args::new());
    assert!(!doc(&s).1.is_dirty(), "保存時点に戻れば未変更");
}

#[test]
fn 保存中の入力は新しいノードになり_dirty_のまま残る() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc");
    open(&mut s, "/w/a.txt");
    s.cmd("cursor.doc_end");
    type_text(&mut s, "d");
    // ジョブを実行せずに保存を要求し、その間に入力する
    s.cmd("file.save");
    type_text(&mut s, "e");
    s.run_jobs();
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        b"abcd",
        "保存したのは要求時の Snapshot"
    );
    assert_eq!(s.doc_text(), "abcde");
    assert!(doc(&s).1.is_dirty(), "保存中の編集は dirty のまま");
}

#[test]
fn 保存中の二重の保存要求は無視する() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc");
    open(&mut s, "/w/a.txt");
    type_text(&mut s, "d");
    messages(&mut s);
    s.cmd("file.save");
    let second = s.cmd("file.save");
    assert_eq!(second, CommandResult::Done);
    assert!(messages(&mut s).contains(&"info.save.in_progress".to_owned()));
    s.run_jobs();
    assert_eq!(s.pending_jobs(), 0);
}

#[test]
fn 外部変更があると保存は_conflict_で止まる() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc");
    open(&mut s, "/w/a.txt");
    type_text(&mut s, "X");
    messages(&mut s);
    s.clock.advance(eve_core::Millis(10));
    s.fs.set_file(&FsPath::from("/w/a.txt"), b"theirs");
    run(&mut s, "file.save", Args::new());
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"theirs");
    assert!(doc(&s).1.is_dirty(), "保存できていないので dirty のまま");
    // ダイアログを出し、Esc で中止すると何も書かない
    assert!(dialog_open(&s));
    keys(&mut s, "esc");
    assert!(!dialog_open(&s));
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"theirs");
}

// verifies: AC-FR-101-2
#[test]
fn 存在しないパスは空の新規文書で_保存すると作成する() {
    let mut s = env();
    open(&mut s, "/w/new.txt");
    assert_eq!(s.doc_text(), "");
    assert!(!doc(&s).1.is_dirty());
    assert!(messages(&mut s).contains(&"info.open.new_file".to_owned()));
    type_text(&mut s, "hi");
    run(&mut s, "file.save", Args::new());
    assert_eq!(s.fs.read("/w/new.txt").unwrap(), b"hi");
    assert!(!doc(&s).1.is_dirty());
}

// verifies: AC-FR-101-2
#[test]
fn 親フォルダがないパスは警告し_保存はエラー() {
    let mut s = env();
    open(&mut s, "/nodir/x.txt");
    assert!(messages(&mut s).contains(&"msg.open.parent_missing".to_owned()));
    type_text(&mut s, "a");
    run(&mut s, "file.save", Args::new());
    assert!(!s.fs.exists("/nodir/x.txt"), "フォルダは作らない");
    // 失敗の text は保存の失敗の確認の本文に入る (別名で保存を案内する)
    assert!(dialog_open(&s));
}

// verifies: AC-NFR-304-1
#[test]
fn 不正バイトは読み取り専用で開き_別名保存でバイト一致する() {
    let mut s = env();
    let src: &[u8] = b"ok \x9A\xFF end\n";
    s.fs.add_file("/w/bad.txt", src);
    open(&mut s, "/w/bad.txt");
    // 不正バイトが多いので確認が出る (03 §8.3): 読み取り専用で開く
    type_text(&mut s, "r");
    s.run_jobs();
    let (_, d) = doc(&s);
    assert_eq!(d.format.invalid_bytes, 2);
    assert!(d.read_only.contains(eve_core::ReadOnlyReason::InvalidBytes));
    type_text(&mut s, "x");
    assert!(s.doc_text().starts_with("ok "), "編集は拒否される");
    assert_eq!(
        s.doc_text()
            .chars()
            .filter(|c| eve_encoding::unescape_char(*c).is_some())
            .count(),
        2
    );
    assert_eq!(
        s.cmd("file.save"),
        CommandResult::Rejected(RejectReason::NotAvailable)
    );
    // 名前を付けて保存は読み取り専用でも実行できる: 元のバイト列がそのまま書かれる (NFR-303)
    run(
        &mut s,
        "file.save_as",
        Args::new().with_str("path", "/w/copy.txt"),
    );
    assert_eq!(s.fs.read("/w/copy.txt").unwrap(), src);
}

// verifies: AC-FR-115-1
#[test]
fn バイナリは確認が要るので開かず_今の文書を残す() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc");
    s.fs.add_file("/w/bin.dat", b"\x00\x01\x02\x03");
    open(&mut s, "/w/a.txt");
    let (id, _) = doc(&s);
    messages(&mut s);
    open(&mut s, "/w/bin.dat");
    assert!(dialog_open(&s), "確認が出る");
    assert_eq!(doc(&s).0, id, "確認の間は今の文書がそのまま表示される");
    keys(&mut s, "esc");
    s.run_jobs();
    assert_eq!(
        doc(&s).0,
        id,
        "中止すると今の文書がそのまま残る (FR-107 AC-3)"
    );
    assert!(messages(&mut s).contains(&"info.open.cancelled".to_owned()));
    assert_eq!(s.editor.state().documents_len(), 1, "読込中の文書は捨てる");
}

// verifies: AC-FR-115-1
#[test]
fn バイナリを読み取り専用で開く() {
    let mut s = env();
    s.fs.add_file("/w/bin.dat", b" ");
    open(&mut s, "/w/bin.dat");
    assert!(dialog_open(&s));
    type_text(&mut s, "r");
    s.run_jobs();
    let (_, d) = doc(&s);
    assert!(!d.is_editable() && !d.read_only.is_empty(), "読み取り専用");
    assert!(
        d.read_only.contains(eve_core::ReadOnlyReason::Binary),
        "理由は Binary"
    );
}

#[test]
fn 文字コードを指定して開く() {
    let mut s = env();
    // Shift_JIS の「あ」(82 A0) の前に NUL: 判定ではバイナリ。UTF-8 を指定すれば NUL も本文として読める
    s.fs.add_file("/w/bin.dat", b" abc");
    open(&mut s, "/w/bin.dat");
    type_text(&mut s, "e");
    assert_eq!(s.overlay(), "prompt");
    // 不明な名前は入力欄に戻る
    type_text(&mut s, "nonsense");
    keys(&mut s, "enter");
    s.run_jobs();
    assert_eq!(s.overlay(), "prompt");
    for _ in 0.."nonsense".len() {
        keys(&mut s, "backspace");
    }
    type_text(&mut s, "utf-8");
    keys(&mut s, "enter");
    s.run_jobs();
    assert!(!dialog_open(&s) && s.overlay() != "prompt");
    assert_eq!(doc(&s).1.format.encoding, eve_core::Encoding::Utf8);
    assert!(s.doc_text().ends_with("abc"));
}

#[test]
fn 文字コードの入力で_esc_は開くのを中止する() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc");
    s.fs.add_file("/w/bin.dat", b" ");
    open(&mut s, "/w/a.txt");
    open(&mut s, "/w/bin.dat");
    type_text(&mut s, "e");
    keys(&mut s, "esc");
    s.run_jobs();
    assert_eq!(s.doc_text(), "abc");
    assert_eq!(s.overlay(), "");
    assert_eq!(s.editor.state().documents_len(), 1);
}

#[test]
fn 不正バイトが多いファイルは小さくても確認する() {
    let mut s = env();
    s.fs.add_file("/w/bad.txt", b"ab\x9A\x9Bcd");
    open(&mut s, "/w/bad.txt");
    assert!(
        dialog_open(&s),
        "ManyInvalidBytes の確認 (大きさによらず、03 §8.3)"
    );
    type_text(&mut s, "c");
    s.run_jobs();
    assert!(!dialog_open(&s));
    assert_eq!(s.editor.state().documents_len(), 1);
}

#[test]
fn フォルダは開けない() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc");
    open(&mut s, "/w/a.txt");
    messages(&mut s);
    open(&mut s, "/w");
    assert_eq!(s.doc_text(), "abc");
    assert!(messages(&mut s).contains(&"msg.open.not_a_file".to_owned()));
}

#[test]
fn 別のファイルを開くとバッファが増え_同じファイルは既に開いている() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"aaa");
    s.fs.add_file("/w/b.txt", b"bbb");
    open(&mut s, "/w/a.txt");
    assert_eq!(
        s.editor.state().documents_len(),
        1,
        "何も入力していない無題のバッファは、開いたファイルで置き換わる"
    );
    drain_events(&mut s);
    open(&mut s, "/w/b.txt");
    assert_eq!(
        s.doc_text(),
        "bbb",
        "読込が終わると新しいバッファへ切り替わる"
    );
    assert!(
        !drain_events(&mut s)
            .iter()
            .any(|e| matches!(e, EditorEvent::DocumentClosed { .. })),
        "今の文書は閉じない"
    );
    assert_eq!(s.editor.state().documents_len(), 2, "a.txt が残る");
    // 表示中のファイルを開き直そうとすると既に開いている
    messages(&mut s);
    open(&mut s, "/w/b.txt");
    assert!(messages(&mut s).contains(&"info.open.already_open".to_owned()));
    // 別のバッファで開いているファイルは、そのバッファへ切り替わる
    open(&mut s, "/w/a.txt");
    assert_eq!(s.doc_text(), "aaa");
    assert_eq!(s.editor.state().documents_len(), 2);
}

#[test]
fn 新規文書の保存は名前の入力プロンプトを出す() {
    let mut s = env();
    s.editor.open_initial(None, false);
    assert_eq!(s.doc_text(), "");
    assert!(!doc(&s).1.is_dirty());
    type_text(&mut s, "memo");
    assert_eq!(
        s.cmd("file.save"),
        CommandResult::Done,
        "パスがなければ名前を尋ねる"
    );
    assert_eq!(s.overlay(), "prompt");
    run(
        &mut s,
        "file.save_as",
        Args::new().with_str("path", "memo.txt"),
    );
    let _ = (RejectReason::NeedsInput, TextFormat::new_document());
}

#[test]
fn 読み取り専用の指定で開く() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc");
    s.editor.open_initial(Some("/w/a.txt"), true);
    s.run_jobs();
    assert!(
        doc(&s)
            .1
            .read_only
            .contains(eve_core::ReadOnlyReason::UserRequested)
    );
    type_text(&mut s, "x");
    assert_eq!(s.doc_text(), "abc");
}

#[test]
fn 読込中は編集できず_完了すると編集できる() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc");
    s.editor.open_initial(Some("/w/a.txt"), false);
    assert_eq!(doc(&s).1.load_state, eve_core::LoadState::Loading);
    type_text(&mut s, "x"); // 読込中: 拒否される
    s.run_jobs();
    assert_eq!(s.doc_text(), "abc");
    type_text(&mut s, "x");
    assert_eq!(s.doc_text(), "xabc");
}

#[test]
fn 読込を待つ間も今の文書を編集でき_重ねて開いてもよい() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"aaa");
    s.fs.add_file("/w/b.txt", b"bbb");
    s.fs.add_file("/w/c.txt", b"ccc");
    open(&mut s, "/w/a.txt");
    // b を開き始める (ジョブはまだ実行しない)
    let r = s.cmd_with("file.open", Args::new().with_str("path", "/w/b.txt"));
    assert!(matches!(r, CommandResult::Pending(_)));
    type_text(&mut s, "x"); // 何も置き換えないので、読込の間も今の文書を編集できる
    assert_eq!(s.doc_text(), "xaaa");
    let r = s.cmd_with("file.open", Args::new().with_str("path", "/w/c.txt"));
    assert!(matches!(r, CommandResult::Pending(_)), "重ねて開いてもよい");
    s.run_jobs();
    assert_eq!(s.editor.state().documents_len(), 3, "a・b・c の 3 つ");
    assert_eq!(s.doc_text(), "bbb", "最初に読み終えた b へ切り替わる");
    type_text(&mut s, "y");
    assert_eq!(s.doc_text(), "ybbb", "完了後は編集できる");
}

#[test]
fn 読み取り専用の文書を同じファイルへ名前を付けて保存しない() {
    let mut s = env();
    let src: &[u8] = b"ok \x9A\n";
    s.fs.add_file("/w/bad.txt", src);
    open(&mut s, "/w/bad.txt");
    type_text(&mut s, "r");
    s.run_jobs();
    messages(&mut s);
    let r = run(
        &mut s,
        "file.save_as",
        Args::new().with_str("path", "/w/bad.txt"),
    );
    assert_eq!(r, CommandResult::Rejected(RejectReason::ReadOnly));
    assert_eq!(
        s.fs.read("/w/bad.txt").unwrap(),
        src,
        "元のバイト列を損なわない"
    );
    assert!(messages(&mut s).contains(&"msg.save_as.same_file_read_only".to_owned()));
}

#[test]
fn 名前を付けて保存の失敗はパスと_stamp_を変えない() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc");
    s.fs.add_file("/w/other.txt", b"EXIST");
    open(&mut s, "/w/a.txt");
    let stamp = doc(&s).1.disk_stamp;
    type_text(&mut s, "x");
    run(
        &mut s,
        "file.save_as",
        Args::new().with_str("path", "/w/other.txt"),
    ); // 既存 → Conflict
    assert_eq!(s.fs.read("/w/other.txt").unwrap(), b"EXIST");
    let (_, d) = doc(&s);
    assert_eq!(
        d.path,
        Some(FsPath::from("/w/a.txt")),
        "失敗したのでパスは元のまま"
    );
    assert_eq!(d.disk_stamp, stamp);
    assert!(d.is_dirty());
    // 競合の確認は中止する (Esc)。元のファイルへはそのまま保存できる
    assert!(dialog_open(&s));
    keys(&mut s, "esc");
    run(&mut s, "file.save", Args::new());
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"xabc");
}

#[test]
fn 保存の待ちの間の名前を付けて保存は無視する() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc");
    open(&mut s, "/w/a.txt");
    type_text(&mut s, "d");
    s.cmd("file.save");
    messages(&mut s);
    let r = s.cmd_with("file.save_as", Args::new().with_str("path", "/w/a.txt"));
    assert_eq!(r, CommandResult::Done);
    assert!(messages(&mut s).contains(&"info.save.in_progress".to_owned()));
    s.run_jobs();
    assert_eq!(s.pending_jobs(), 0);
}

#[test]
fn 読込中の文書は名前を付けて保存できない() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc");
    s.editor.open_initial(Some("/w/a.txt"), false);
    let r = s.cmd_with("file.save_as", Args::new().with_str("path", "/w/zz.txt"));
    assert_eq!(r, CommandResult::Rejected(RejectReason::NotAvailable));
    s.run_jobs();
    assert!(!s.fs.exists("/w/zz.txt"));
}

#[test]
fn 最初の読込の失敗は無題で始まり_再試行できる() {
    let mut s = env();
    s.fs.add_file("/w/bin.dat", b"\x00\x01\x02");
    s.editor.open_initial(Some("/w/bin.dat"), false);
    s.run_jobs();
    assert!(dialog_open(&s));
    keys(&mut s, "esc");
    s.run_jobs();
    let (_, d) = doc(&s);
    assert!(
        d.path.is_none() && d.is_editable(),
        "失敗した文書は閉じ、無題の文書で始まる"
    );
    assert_eq!(s.editor.state().documents_len(), 1);
    // 同じパスを開き直せる (already_open にならない)
    messages(&mut s);
    s.fs.add_file("/w/bin.dat", b"text now");
    open(&mut s, "/w/bin.dat");
    assert_eq!(s.doc_text(), "text now");
}

#[test]
fn readonly_の指定でもバイナリの確認は出す() {
    let mut s = env();
    s.fs.add_file("/w/bin.dat", b"\x00\x01\x02");
    s.editor.open_initial(Some("/w/bin.dat"), true);
    s.run_jobs();
    assert!(dialog_open(&s), "--readonly でもバイナリの確認は出る");
}

// ---- 中断された保存の痕跡 (02 §4.5「中断の痕跡の写像」、05 §4.6) ----

fn flag(
    target: &str,
    method: eve_core::SaveMethod,
    recorded: bool,
    complete: bool,
) -> eve_core::InflightRecord {
    eve_core::InflightRecord {
        target: FsPath::from(target),
        pid: 100,
        started_at: None,
        method,
        tmp: Some(FsPath::from("/w/.a.txt.eve-tmp-100-aaaaaaaa")),
        backup: Some(FsPath::from("/w/.a.txt.eve-bak-100-bbbbbbbb")),
        backup_recorded: recorded,
        backup_complete: complete,
        flag: FsPath::from("/state/inflight/k/100-1.toml"),
        malformed: false,
    }
}

#[test]
fn 置換の途中で中断した保存は退避名を案内して_文書は空の新規として開く() {
    let mut s = env();
    s.fs.add_file("/w/.a.txt.eve-bak-100-bbbbbbbb", b"original");
    s.fs.add_inflight(flag("/w/a.txt", eve_core::SaveMethod::Atomic, false, false));
    messages(&mut s);
    open(&mut s, "/w/a.txt");
    assert!(messages(&mut s).contains(&"msg.open.interrupted_save".to_owned()));
    assert_eq!(
        s.doc_text(),
        "",
        "T が無いので空の新規文書 (黙って開かず、場所を示す)"
    );
    assert_eq!(s.fs.inflight_records().len(), 1, "報告した記録は消さない");
}

#[test]
fn 新規作成の途中で中断した保存を案内する() {
    let mut s = env();
    s.fs.add_file("/w/.a.txt.eve-tmp-100-aaaaaaaa", b"partial");
    s.fs.add_inflight(flag(
        "/w/a.txt",
        eve_core::SaveMethod::NewFile,
        false,
        false,
    ));
    messages(&mut s);
    open(&mut s, "/w/a.txt");
    assert!(messages(&mut s).contains(&"msg.open.interrupted_new".to_owned()));
}

#[test]
fn 直接上書きの中断はファイルがあっても案内する() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"half-written");
    s.fs.add_file("/w/.a.txt.eve-bak-100-bbbbbbbb", b"original");
    s.fs.add_inflight(flag("/w/a.txt", eve_core::SaveMethod::InPlace, true, true));
    messages(&mut s);
    open(&mut s, "/w/a.txt");
    assert_eq!(s.doc_text(), "half-written", "文書は通常どおり開く");
    assert!(messages(&mut s).contains(&"msg.open.interrupted_save".to_owned()));
}

#[test]
fn t_が在る原子的保存の痕跡は報告せずフラグを掃除する() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"fine");
    s.fs.add_inflight(flag("/w/a.txt", eve_core::SaveMethod::Atomic, false, false));
    messages(&mut s);
    open(&mut s, "/w/a.txt");
    let m = messages(&mut s);
    assert!(
        !m.iter().any(|k| k.starts_with("msg.open.interrupted")),
        "{m:?}"
    );
    assert!(
        s.fs.inflight_records().is_empty(),
        "痕跡の無いフラグは消える"
    );
}

#[test]
fn 痕跡が実在する記録は報告しなくてもフラグを残す() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"fine");
    s.fs.add_file("/w/.a.txt.eve-tmp-100-aaaaaaaa", b"leftover");
    s.fs.add_inflight(flag("/w/a.txt", eve_core::SaveMethod::Atomic, false, false));
    open(&mut s, "/w/a.txt");
    assert_eq!(s.fs.inflight_records().len(), 1);
}

// ---- 起動時の退避の案内 (02 §9.3 の RecoveryScan、05 §7.5) ----

#[test]
fn 起動時に退避が残っていれば最も新しいものの場所を案内する() {
    let mut s = env();
    s.fs.add_recovery_dir("/s/recovery/20261001T000000Z-100", false);
    s.fs.add_recovery_dir("/s/recovery/20261021T120000Z-200", false);
    s.fs.add_recovery_dir("/s/recovery/20261021T120000Z-200-2", false);
    s.fs.add_recovery_dir("/s/recovery/rescue/20261022T000000Z-300", true);
    s.editor.open_initial(None, false);
    s.run_jobs();
    let keys = messages(&mut s);
    assert_eq!(
        keys.last().map(String::as_str),
        Some("msg.recovery.found"),
        "異常終了の退避があるので found (rescue ではない): {keys:?}"
    );
    let frame = frame(&mut s);
    let text: String = frame
        .messages
        .current
        .expect("現在のメッセージ")
        .text
        .spans
        .iter()
        .map(|sp| sp.text.as_str())
        .collect();
    assert!(
        text.contains("20261021T120000Z-200-2"),
        "同じ時刻は -n の大きい方が新しい: {text}"
    );
    assert!(text.contains('3'), "異常終了の退避は 3 件: {text}");
}

#[test]
fn 退避先に保存の退避だけなら文言を分ける() {
    let mut s = env();
    s.fs.add_recovery_dir("/s/recovery/rescue/20261022T000000Z-300", true);
    s.editor.open_initial(None, false);
    s.run_jobs();
    assert!(messages(&mut s).contains(&"msg.recovery.found_rescue".to_owned()));
}

#[test]
fn 退避が無ければ案内しない() {
    let mut s = env();
    s.editor.open_initial(None, false);
    s.run_jobs();
    assert!(
        !messages(&mut s)
            .iter()
            .any(|k| k.starts_with("msg.recovery"))
    );
}

/// 編集済みの文書を開き、原子的保存が `kind` で失敗する状態にして保存する。
fn edited_save_fails(s: &mut Harness, kind: eve_core::FsErrorKind) {
    s.fs.add_file("/w/a.txt", b"abc");
    open(s, "/w/a.txt");
    s.cmd("cursor.doc_end");
    type_text(s, "d");
    messages(s);
    s.fs.fail_next(eve_testkit::FsOp::WriteAtomic, kind);
    run(s, "file.save", Args::new());
}

fn dialog_open(s: &Harness) -> bool {
    s.overlay() == "dialog"
}

// verifies: AC-FR-105-2
#[test]
fn 原子的置換ができなければ確認を出し_直接上書きで保存する() {
    let mut s = env();
    edited_save_fails(&mut s, eve_core::FsErrorKind::AtomicUnsupported);
    assert!(dialog_open(&s), "確認が出る");
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        b"abc",
        "確認の間は何も書かない"
    );
    type_text(&mut s, "o");
    s.run_jobs();
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"abcd");
    assert!(!doc(&s).1.is_dirty());
    assert!(messages(&mut s).contains(&"info.save.in_place".to_owned()));
}

#[test]
fn 原子的置換の確認のキャンセルでは何も書かず_dirty_のまま() {
    let mut s = env();
    edited_save_fails(&mut s, eve_core::FsErrorKind::AtomicUnsupported);
    keys(&mut s, "esc");
    s.run_jobs();
    assert!(!dialog_open(&s));
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"abc");
    assert!(doc(&s).1.is_dirty());
}

#[test]
fn 原子的置換の確認の別名で保存は名前の入力へ進む() {
    let mut s = env();
    edited_save_fails(&mut s, eve_core::FsErrorKind::AtomicUnsupported);
    type_text(&mut s, "a");
    s.run_jobs();
    assert_eq!(s.overlay(), "prompt");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"abc");
}

#[test]
fn バックアップを作れなければ確認し_承認するとバックアップなしで上書きする() {
    let mut s = env();
    edited_save_fails(&mut s, eve_core::FsErrorKind::AtomicUnsupported);
    s.fs.fail_next(
        eve_testkit::FsOp::WriteInPlace,
        eve_core::FsErrorKind::BackupFailed,
    );
    type_text(&mut s, "o");
    s.run_jobs();
    assert!(dialog_open(&s), "バックアップなしの確認が続く");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"abc");
    type_text(&mut s, "o");
    s.run_jobs();
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"abcd");
    assert!(!doc(&s).1.is_dirty());
}

// verifies: AC-FR-106-3
#[test]
fn ハードリンクはリンクを保つか切り離すかを選ぶ() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc");
    s.fs.set_nlink("/w/a.txt", 2);
    open(&mut s, "/w/a.txt");
    s.cmd("cursor.doc_end");
    type_text(&mut s, "d");
    run(&mut s, "file.save", Args::new());
    assert!(dialog_open(&s));
    // 切り離して保存: 原子的置換で書き、nlink は 1 になる
    type_text(&mut s, "b");
    s.run_jobs();
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"abcd");
    assert!(!doc(&s).1.is_dirty());
}

#[test]
fn 保存の失敗は確認を出し_退避先に保存すると本文を書いて_dirty_のまま() {
    let mut s = env();
    edited_save_fails(&mut s, eve_core::FsErrorKind::PermissionDenied);
    assert!(dialog_open(&s), "失敗の後の選択が出る");
    type_text(&mut s, "r");
    s.run_jobs();
    assert_eq!(s.fs.recovery_written(), vec![b"abcd".to_vec()]);
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        b"abc",
        "元のファイルは変えない"
    );
    assert!(doc(&s).1.is_dirty());
    assert!(messages(&mut s).contains(&"info.save.to_recovery".to_owned()));
}

#[test]
fn 退避先に保存が失敗するとそれを示す() {
    let mut s = env();
    edited_save_fails(&mut s, eve_core::FsErrorKind::DiskFull);
    s.fs.fail_next(
        eve_testkit::FsOp::WriteRecovery,
        eve_core::FsErrorKind::DiskFull,
    );
    type_text(&mut s, "r");
    s.run_jobs();
    let actual = s.fs.recovery_written();
    assert!(actual.is_empty(), "{actual:?}");
    assert!(messages(&mut s).contains(&"msg.save.recovery_failed".to_owned()));
    assert!(doc(&s).1.is_dirty());
}

#[test]
fn 保存の失敗の確認の別名で保存とキャンセル() {
    let mut s = env();
    edited_save_fails(&mut s, eve_core::FsErrorKind::DiskFull);
    type_text(&mut s, "a");
    s.run_jobs();
    assert_eq!(s.overlay(), "prompt");
    keys(&mut s, "esc");
    // 権限エラーだと文書が読み取り専用になり file.save が無効になるので、失敗はディスクの空きで起こす
    messages(&mut s);
    s.fs.fail_next(
        eve_testkit::FsOp::WriteAtomic,
        eve_core::FsErrorKind::DiskFull,
    );
    run(&mut s, "file.save", Args::new());
    assert!(dialog_open(&s));
    keys(&mut s, "esc");
    assert!(!dialog_open(&s));
    assert!(messages(&mut s).contains(&"info.save.aborted".to_owned()));
    let actual = s.fs.recovery_written();
    assert!(actual.is_empty(), "{actual:?}");
}

#[test]
fn busy_のときだけ再試行でき_照合を通って保存される() {
    let mut s = env();
    edited_save_fails(&mut s, eve_core::FsErrorKind::Busy);
    assert!(dialog_open(&s));
    type_text(&mut s, "t");
    s.run_jobs();
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"abcd");
    assert!(!doc(&s).1.is_dirty());
    // Busy 以外には再試行が無い (t は無視される)
    let mut s = env();
    edited_save_fails(&mut s, eve_core::FsErrorKind::PermissionDenied);
    type_text(&mut s, "t");
    s.run_jobs();
    assert!(dialog_open(&s));
}

// 03 §9.7: 開き直しの「保存」の後続は、保存した文書に属する。保存の完了までに別の dirty な文書へ
// 切り替えても、その文書を開き直さない (未保存の編集と履歴を失わない)。docs/review/v0.3-findings.md の SAF-01
#[test]
fn 開き直しの保存中に別の文書へ切り替えても_その文書は開き直さない() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"aaa\n");
    s.fs.add_file("/w/b.txt", b"bbb\n");
    open(&mut s, "/w/a.txt");
    let (a, _) = doc(&s);
    type_text(&mut s, "x");
    open(&mut s, "/w/b.txt");
    let (b, _) = doc(&s);
    assert_ne!(a, b);
    type_text(&mut s, "y");
    let b_text = s.doc_text();
    run(&mut s, "buffer.prev", Args::new());
    assert_eq!(doc(&s).0, a);
    // A を開き直す → 未保存の確認で「保存」(s)。保存ジョブはまだ走らせない
    s.cmd_with(
        "file.reopen_with_encoding",
        Args::new().with_str("encoding", "shift_jis"),
    );
    assert!(dialog_open(&s), "未保存の確認 (ui.confirm.unsaved_reopen)");
    type_text(&mut s, "s");
    // 保存の完了の前に B へ切り替える
    s.cmd("buffer.next");
    assert_eq!(doc(&s).0, b);
    s.run_jobs();
    // B は開き直されず、未保存の編集と履歴が残る
    let bd = s.editor.state().document(b).unwrap();
    assert!(bd.is_dirty(), "B の未保存の編集が残る");
    assert_eq!(bd.to_string_lossy(), b_text);
    assert!(!dialog_open(&s));
    run(&mut s, "edit.undo", Args::new());
    assert_eq!(s.doc_text(), "bbb\n", "B の履歴が残る");
    // A は保存されてから Shift_JIS で開き直された
    let ad = s.editor.state().document(a).unwrap();
    assert_eq!(ad.format.encoding, eve_core::Encoding::ShiftJis);
    assert_eq!(ad.to_string_lossy(), "xaaa\n");
    assert!(!ad.is_dirty());
}

fn sjis_bytes(s: &str) -> Vec<u8> {
    let (b, _, _) = encoding_rs::SHIFT_JIS.encode(s);
    b.into_owned()
}

// verifies: AC-NFR-304-3
#[test]
fn 文字コードを指定して開き直す() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc\ndef\n");
    open(&mut s, "/w/a.txt");
    let (id, _) = doc(&s);
    // 履歴を作ってから保存し (dirty でない)、開き直す
    s.cmd("cursor.doc_end");
    type_text(&mut s, "x");
    run(&mut s, "file.save", Args::new());
    messages(&mut s);
    let r = run(
        &mut s,
        "file.reopen_with_encoding",
        Args::new().with_str("encoding", "shift_jis"),
    );
    assert!(matches!(r, CommandResult::Pending(_)));
    let (id2, d) = doc(&s);
    assert_eq!(id2, id, "同じ DocumentId");
    assert_eq!(d.format.encoding, eve_core::Encoding::ShiftJis);
    assert!(!d.is_dirty());
    assert!(messages(&mut s).contains(&"info.reopen.history_cleared".to_owned()));
    // 開き直しは Undo できない
    run(&mut s, "edit.undo", Args::new());
    assert_eq!(s.doc_text(), "abc\ndef\nx");
}

// verifies: AC-FR-111-2
#[test]
fn 開き直しは未保存なら確認し_破棄すると読み直す() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", &sjis_bytes("日本語のテキストです。"));
    open(&mut s, "/w/a.txt");
    assert_eq!(doc(&s).1.format.encoding, eve_core::Encoding::ShiftJis);
    type_text(&mut s, "x");
    run(
        &mut s,
        "file.reopen_with_encoding",
        Args::new().with_str("encoding", "utf-8"),
    );
    assert!(dialog_open(&s), "未保存の確認 (ui.confirm.unsaved_reopen)");
    type_text(&mut s, "d");
    s.run_jobs();
    // Shift_JIS のバイトを UTF-8 として読むと不正バイトが多い: 続けて確認が出る
    assert!(dialog_open(&s));
    type_text(&mut s, "r");
    s.run_jobs();
    let (_, d) = doc(&s);
    assert_eq!(d.format.encoding, eve_core::Encoding::Utf8);
    assert!(d.format.invalid_bytes > 0 && !d.is_dirty());
    assert!(d.read_only.contains(eve_core::ReadOnlyReason::InvalidBytes));
}

// verifies: AC-FR-111-2
#[test]
fn 開き直しの確認で中止すると旧本文と書式のまま() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", &sjis_bytes("日本語のテキストです。"));
    open(&mut s, "/w/a.txt");
    let before = s.doc_text();
    run(
        &mut s,
        "file.reopen_with_encoding",
        Args::new().with_str("encoding", "utf-8"),
    );
    assert!(dialog_open(&s), "不正バイトの確認");
    keys(&mut s, "esc");
    s.run_jobs();
    let (_, d) = doc(&s);
    assert_eq!(d.format.encoding, eve_core::Encoding::ShiftJis);
    assert_eq!(d.load_state, eve_core::LoadState::Ready);
    assert_eq!(s.doc_text(), before);
}

#[test]
fn 開き直しは引数が無ければ文字コードを尋ねる() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc");
    open(&mut s, "/w/a.txt");
    run(&mut s, "file.reopen_with_encoding", Args::new());
    assert_eq!(s.overlay(), "prompt");
    type_text(&mut s, "euc-jp");
    keys(&mut s, "enter");
    s.run_jobs();
    assert_eq!(doc(&s).1.format.encoding, eve_core::Encoding::EucJp);
}

#[test]
fn 再読込は外部の変更を取り込み_undo_で戻せる() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc");
    open(&mut s, "/w/a.txt");
    s.fs.set_file(&FsPath::from("/w/a.txt"), b"XYZ\n");
    messages(&mut s);
    let r = run(&mut s, "file.reload", Args::new());
    assert!(matches!(r, CommandResult::Pending(_)));
    assert_eq!(s.doc_text(), "XYZ\n");
    assert!(!doc(&s).1.is_dirty(), "読み直した内容がディスクの状態");
    assert!(messages(&mut s).contains(&"info.reload.done".to_owned()));
    // 元に戻すで読み込む前の内容に戻り、保存点から外れる (dirty)
    run(&mut s, "edit.undo", Args::new());
    assert_eq!(s.doc_text(), "abc");
    assert!(doc(&s).1.is_dirty());
    run(&mut s, "edit.redo", Args::new());
    assert_eq!(s.doc_text(), "XYZ\n");
    assert!(!doc(&s).1.is_dirty());
}

// spec: 03 §9.5 (再読込の後のカーソルは同じ行:桁 (書記素) を目安にする)
#[test]
fn 再読込の後のカーソルは同じ行と桁に置く() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"aaa\nbbbb\nccc");
    open(&mut s, "/w/a.txt");
    keys(&mut s, "ctrl+end left"); // 3 行目の 3 桁 (cc|c)
    assert_eq!(s.selection_range(), (11, 11));
    // 外部で前の方に行が足され、3 行目は全角の行になった
    s.fs.set_file(
        &FsPath::from("/w/a.txt"),
        "x\naaa\n日本語です\nccc".as_bytes(),
    );
    run(&mut s, "file.reload", Args::new());
    let p = "x\naaa\n日本".len();
    assert_eq!(s.selection_range(), (p, p), "3 行目の 3 桁 (日本|語)");
    // 行が足りなければ最後の行、桁が足りなければ行末
    s.fs.set_file(&FsPath::from("/w/a.txt"), b"z");
    run(&mut s, "file.reload", Args::new());
    assert_eq!(s.selection_range(), (1, 1));
}

#[test]
fn 再読込の書式の変更は_undo_で本文と一緒に戻る() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"ab\nc");
    open(&mut s, "/w/a.txt");
    assert!(!doc(&s).1.format.has_bom);
    // 外部で BOM と CRLF が付いた
    s.fs.set_file(&FsPath::from("/w/a.txt"), b"\xEF\xBB\xBFab\r\nc");
    run(&mut s, "file.reload", Args::new());
    let f = doc(&s).1.format.clone();
    assert!(f.has_bom && f.default_eol == eve_text::LineEnding::Crlf);
    run(&mut s, "edit.undo", Args::new());
    let f = doc(&s).1.format.clone();
    assert!(
        !f.has_bom && f.default_eol == eve_text::LineEnding::Lf,
        "書式も戻る"
    );
    assert_eq!(s.doc_text(), "ab\nc");
    run(&mut s, "edit.redo", Args::new());
    assert!(doc(&s).1.format.has_bom);
}

#[test]
fn dirty_な再読込は確認し_キャンセルでは何も変わらない() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc");
    open(&mut s, "/w/a.txt");
    type_text(&mut s, "x");
    s.fs.set_file(&FsPath::from("/w/a.txt"), b"NEW");
    run(&mut s, "file.reload", Args::new());
    assert!(dialog_open(&s));
    keys(&mut s, "esc");
    s.run_jobs();
    assert!(
        s.doc_text().contains('x') && doc(&s).1.is_dirty(),
        "キャンセルでは何も変わらない"
    );
    run(&mut s, "file.reload", Args::new());
    type_text(&mut s, "l");
    s.run_jobs();
    assert_eq!(s.doc_text(), "NEW");
    assert!(!doc(&s).1.is_dirty());
    // 破棄した編集も元に戻すで取り戻せる (P1)
    run(&mut s, "edit.undo", Args::new());
    assert!(s.doc_text().contains('x'));
}

#[test]
fn ファイルが無ければ再読込は文書を変えずに知らせる() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc");
    open(&mut s, "/w/a.txt");
    s.fs.remove(&FsPath::from("/w/a.txt"));
    messages(&mut s);
    run(&mut s, "file.reload", Args::new());
    assert_eq!(s.doc_text(), "abc");
    assert_eq!(doc(&s).1.load_state, eve_core::LoadState::Ready);
    assert!(messages(&mut s).contains(&"msg.reload.missing".to_owned()));
}

#[test]
fn 再読込の後に読み取り専用なら_undo_できない() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc");
    open(&mut s, "/w/a.txt");
    // 外部で不正バイトが入った: 開くときと同じ確認が出る
    s.fs.set_file(&FsPath::from("/w/a.txt"), b"ab\x9A\x9Bcd");
    messages(&mut s);
    run(&mut s, "file.reload", Args::new());
    assert!(dialog_open(&s));
    type_text(&mut s, "r");
    s.run_jobs();
    assert!(!doc(&s).1.read_only.is_empty());
    assert!(messages(&mut s).contains(&"info.reload.done_read_only".to_owned()));
    let r = run(&mut s, "edit.undo", Args::new());
    assert!(
        matches!(r, CommandResult::Rejected(_)),
        "読み取り専用では Undo できない"
    );
}

// verifies: AC-FR-111-3
#[test]
fn 文字コードを指定して保存すると_書式は成功のときだけ変わる() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", "日本語\n".as_bytes());
    open(&mut s, "/w/a.txt");
    let before = s.fs.stamp("/w/a.txt");
    let r = run(
        &mut s,
        "file.set_encoding",
        Args::new().with_str("encoding", "shift_jis"),
    );
    assert!(matches!(r, CommandResult::Pending(_)), "未編集でも書く");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), sjis_bytes("日本語\n"));
    assert_ne!(s.fs.stamp("/w/a.txt"), before);
    let d = doc(&s).1;
    assert_eq!(d.format.encoding, eve_core::Encoding::ShiftJis);
    assert!(!d.is_dirty(), "dirty にならない (03 §9.3)");
    assert_eq!(s.doc_text(), "日本語\n");
}

#[test]
fn utf16_を指定すると_bom_を付けて書く() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"ab");
    open(&mut s, "/w/a.txt");
    run(
        &mut s,
        "file.set_encoding",
        Args::new().with_str("encoding", "utf-16le"),
    );
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        vec![0xFF, 0xFE, b'a', 0, b'b', 0]
    );
    assert!(doc(&s).1.format.has_bom);
}

#[test]
fn 表現できない文字があれば書かず_書式も変えない() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", "a😀".as_bytes());
    open(&mut s, "/w/a.txt");
    run(
        &mut s,
        "file.set_encoding",
        Args::new().with_str("encoding", "shift_jis"),
    );
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), "a😀".as_bytes());
    assert_eq!(doc(&s).1.format.encoding, eve_core::Encoding::Utf8);
}

// verifies: AC-FR-111-3
#[test]
fn 保存が失敗したら書式は変わらない() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc");
    open(&mut s, "/w/a.txt");
    s.fs.fail_next(
        eve_testkit::FsOp::WriteAtomic,
        eve_core::FsErrorKind::PermissionDenied,
    );
    run(
        &mut s,
        "file.set_encoding",
        Args::new().with_str("encoding", "euc-jp"),
    );
    assert!(dialog_open(&s));
    assert_eq!(doc(&s).1.format.encoding, eve_core::Encoding::Utf8);
    // 退避先に保存でも書式は変えない
    type_text(&mut s, "r");
    s.run_jobs();
    assert_eq!(doc(&s).1.format.encoding, eve_core::Encoding::Utf8);
}

#[test]
fn 無題の文書は名前を尋ねて新しい書式で保存する() {
    let mut s = env();
    s.editor.open_initial(None, false);
    s.run_jobs();
    type_text(&mut s, "ab");
    run(
        &mut s,
        "file.set_encoding",
        Args::new().with_str("encoding", "utf-16be"),
    );
    assert_eq!(s.overlay(), "prompt");
    type_text(&mut s, "/w/n.txt");
    keys(&mut s, "enter");
    s.run_jobs();
    assert_eq!(
        s.fs.read("/w/n.txt").unwrap(),
        vec![0xFE, 0xFF, 0, b'a', 0, b'b']
    );
    assert_eq!(doc(&s).1.format.encoding, eve_core::Encoding::Utf16Be);
}

#[test]
fn 文字コードの引数が無ければ尋ねる() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc");
    open(&mut s, "/w/a.txt");
    run(&mut s, "file.set_encoding", Args::new());
    assert_eq!(s.overlay(), "prompt");
    type_text(&mut s, "shift_jis");
    keys(&mut s, "enter");
    s.run_jobs();
    assert_eq!(doc(&s).1.format.encoding, eve_core::Encoding::ShiftJis);
}

#[test]
fn 改行コードを変換すると全区切りが揃い_undo_で本文と書式が戻る() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"a\nb\r\nc\rd");
    open(&mut s, "/w/a.txt");
    assert!(doc(&s).1.format.eol_stats.is_mixed());
    messages(&mut s);
    let r = run(
        &mut s,
        "file.set_line_ending",
        Args::new().with_str("eol", "crlf"),
    );
    assert_eq!(r, CommandResult::Done);
    assert_eq!(s.doc_text(), "a\r\nb\r\nc\r\nd");
    let d = doc(&s).1;
    assert!(d.is_dirty());
    assert_eq!(d.format.default_eol, eve_text::LineEnding::Crlf);
    assert!(!d.format.eol_stats.is_mixed() && d.format.eol_stats.crlf == 3);
    assert!(messages(&mut s).contains(&"info.eol.converted".to_owned()));
    // 1 回の Undo で本文と default_eol が戻る
    run(&mut s, "edit.undo", Args::new());
    assert_eq!(s.doc_text(), "a\nb\r\nc\rd");
    let d = doc(&s).1;
    assert_eq!(d.format.default_eol, eve_text::LineEnding::Lf);
    assert!(d.format.eol_stats.is_mixed());
    assert!(!d.is_dirty());
    run(&mut s, "edit.redo", Args::new());
    assert_eq!(s.doc_text(), "a\r\nb\r\nc\r\nd");
}

#[test]
fn 変換済みなら本文を変えず_履歴も増えない() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"a\nb\n");
    open(&mut s, "/w/a.txt");
    messages(&mut s);
    run(
        &mut s,
        "file.set_line_ending",
        Args::new().with_str("eol", "lf"),
    );
    assert!(!doc(&s).1.is_dirty());
    assert!(messages(&mut s).contains(&"info.eol.unchanged".to_owned()));
    assert_eq!(doc(&s).1.history().undo_len(), 0);
    // 改行が無い文書では default_eol だけが変わる (メタデータ。Undo の対象外)
    s.fs.add_file("/w/b.txt", b"abc");
    open(&mut s, "/w/b.txt");
    run(
        &mut s,
        "file.set_line_ending",
        Args::new().with_str("eol", "cr"),
    );
    let d = doc(&s).1;
    assert_eq!(d.format.default_eol, eve_text::LineEnding::Cr);
    assert!(!d.is_dirty() && d.history().undo_len() == 0);
}

#[test]
fn 改行コードの変換は読み取り専用では拒否し_引数の誤りも拒否する() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"a\nb");
    open(&mut s, "/w/a.txt");
    let r = run(
        &mut s,
        "file.set_line_ending",
        Args::new().with_str("eol", "xx"),
    );
    assert_eq!(r, CommandResult::Rejected(RejectReason::InvalidArgs));
    let r = run(&mut s, "file.set_line_ending", Args::new());
    assert_eq!(r, CommandResult::Rejected(RejectReason::InvalidArgs));
    s.fs.add_file("/w/ro.txt", b"x\n");
    s.fs.set_readonly("/w/ro.txt", true);
    open(&mut s, "/w/ro.txt");
    let r = run(
        &mut s,
        "file.set_line_ending",
        Args::new().with_str("eol", "crlf"),
    );
    assert!(matches!(r, CommandResult::Rejected(_)));
    assert_eq!(s.doc_text(), "x\n");
}

#[test]
fn 変換した文書の保存は区切りをそのまま書く() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"a\nb\n");
    open(&mut s, "/w/a.txt");
    run(
        &mut s,
        "file.set_line_ending",
        Args::new().with_str("eol", "crlf"),
    );
    run(&mut s, "file.save", Args::new());
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"a\r\nb\r\n");
}

// verifies: AC-FR-112-1
#[test]
fn 表現できない文字の保存は確認し_utf8_で保存できる() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", &sjis_bytes("日本語のテキストです。\n"));
    open(&mut s, "/w/a.txt");
    assert_eq!(doc(&s).1.format.encoding, eve_core::Encoding::ShiftJis);
    s.cmd("cursor.doc_end");
    type_text(&mut s, "😀");
    run(&mut s, "file.save", Args::new());
    assert!(dialog_open(&s), "E3 の選択肢が出る");
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        sjis_bytes("日本語のテキストです。\n"),
        "何も書かない"
    );
    type_text(&mut s, "u");
    s.run_jobs();
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        "日本語のテキストです。\n😀".as_bytes()
    );
    let d = doc(&s).1;
    assert_eq!(d.format.encoding, eve_core::Encoding::Utf8);
    assert!(!d.format.has_bom && !d.is_dirty());
}

#[test]
fn 表現できない文字の確認のキャンセルは最初の位置へ移り_何も変えない() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc");
    open(&mut s, "/w/a.txt");
    run(
        &mut s,
        "file.set_encoding",
        Args::new().with_str("encoding", "shift_jis"),
    );
    // 表現できる文字だけなので保存できた。ここから絵文字を足して、改めて失敗させる
    type_text(&mut s, "x😀y");
    run(&mut s, "file.save", Args::new());
    assert!(dialog_open(&s));
    keys(&mut s, "esc");
    s.run_jobs();
    assert!(!dialog_open(&s));
    assert!(doc(&s).1.is_dirty(), "文書は変わらない (dirty のまま)");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"abc");
    assert_eq!(doc(&s).1.format.encoding, eve_core::Encoding::ShiftJis);
    // カーソルは最初の表現不能の文字 (絵文字) の位置
    let (id, _) = doc(&s);
    let view = s
        .editor
        .state()
        .first_window()
        .and_then(|w| s.editor.state().window_view(w))
        .unwrap();
    let head = s
        .editor
        .state()
        .view(view)
        .unwrap()
        .selection
        .primary()
        .head
        .0;
    assert_eq!(s.doc_text().find('😀'), Some(head), "{id:?}");
}

// verifies: AC-FR-109-4
#[test]
fn 置換して保存は一覧を示し_確認の後に置換して書く() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", &sjis_bytes("日本語のテキストです。\n"));
    open(&mut s, "/w/a.txt");
    s.cmd("cursor.doc_end");
    // 絵文字は表現不能 (`?` へ)、U+2212 は黙って別の文字に写される (互換文字へ)
    type_text(&mut s, "😀\u{2212}");
    run(&mut s, "file.save", Args::new());
    assert!(dialog_open(&s));
    type_text(&mut s, "r");
    s.run_jobs();
    // 置換の一覧の確認 (details に対象が並ぶ)
    assert!(dialog_open(&s), "置換の確認");
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        sjis_bytes("日本語のテキストです。\n"),
        "確認の間は何も書かない"
    );
    type_text(&mut s, "r");
    s.run_jobs();
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        sjis_bytes("日本語のテキストです。\n?\u{FF0D}")
    );
    assert_eq!(s.doc_text(), "日本語のテキストです。\n?\u{FF0D}");
    assert!(!doc(&s).1.is_dirty());
    // 置換は 1 つの Transaction (Undo で戻せる)
    run(&mut s, "edit.undo", Args::new());
    assert!(s.doc_text().ends_with("😀\u{2212}"));
}

// verifies: AC-FR-109-4
#[test]
fn iso_2022_jp_の半角カナは全角カナへ置換する() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", "ｱｲｳ\n".as_bytes());
    open(&mut s, "/w/a.txt");
    run(
        &mut s,
        "file.set_encoding",
        Args::new().with_str("encoding", "iso-2022-jp"),
    );
    assert!(dialog_open(&s));
    type_text(&mut s, "r");
    s.run_jobs();
    type_text(&mut s, "r");
    s.run_jobs();
    let (expected, _, _) = encoding_rs::ISO_2022_JP.encode("アイウ\n");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), expected.as_ref());
    assert_eq!(doc(&s).1.format.encoding, eve_core::Encoding::Iso2022Jp);
}

#[test]
fn 置換の確認のキャンセルは何も変えず_最初の位置へ移る() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", &sjis_bytes("日本語のテキストです。\n"));
    open(&mut s, "/w/a.txt");
    s.cmd("cursor.doc_end");
    type_text(&mut s, "😀");
    run(&mut s, "file.save", Args::new());
    type_text(&mut s, "r");
    s.run_jobs();
    assert!(dialog_open(&s));
    keys(&mut s, "esc");
    s.run_jobs();
    assert!(!dialog_open(&s));
    assert!(s.doc_text().ends_with('😀'), "置換していない");
    assert!(doc(&s).1.is_dirty());
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        sjis_bytes("日本語のテキストです。\n")
    );
}

// verifies: AC-FR-116-3
#[test]
fn 上書き保存が権限エラーなら読み取り専用に切り替わり_別名で保存できる() {
    let mut s = env();
    edited_save_fails(&mut s, eve_core::FsErrorKind::PermissionDenied);
    // 失敗の確認を閉じる
    keys(&mut s, "esc");
    s.run_jobs();
    assert!(
        doc(&s)
            .1
            .read_only
            .contains(eve_core::ReadOnlyReason::NoWritePermission),
        "書込権限なしに切り替わる"
    );
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        b"abc",
        "ファイルは変わらない"
    );
    assert_eq!(s.doc_text(), "abcd", "編集した内容は失われない");
    // 以後の編集は拒否される
    type_text(&mut s, "x");
    assert_eq!(s.doc_text(), "abcd");
    // 名前を付けて保存で書けて、書けた先の文書は編集できる
    run(
        &mut s,
        "file.save_as",
        Args::new().with_str("path", "/w/b.txt"),
    );
    assert_eq!(s.fs.read("/w/b.txt").unwrap(), b"abcd");
    assert!(doc(&s).1.is_editable());
    type_text(&mut s, "x");
    assert_ne!(s.doc_text(), "abcd", "編集できる");
}

// verifies: AC-FR-116-3
#[test]
fn 名前を付けて保存の権限エラーでは今の文書を読み取り専用にしない() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc");
    open(&mut s, "/w/a.txt");
    type_text(&mut s, "d");
    s.fs.fail_next(
        eve_testkit::FsOp::WriteAtomic,
        eve_core::FsErrorKind::PermissionDenied,
    );
    run(
        &mut s,
        "file.save_as",
        Args::new().with_str("path", "/w/b.txt"),
    );
    assert!(
        !doc(&s)
            .1
            .read_only
            .contains(eve_core::ReadOnlyReason::NoWritePermission)
    );
}

// verifies: AC-FR-116-4
#[test]
fn 読み取り専用属性のファイルを別名で保存すると編集できる() {
    let mut s = env();
    s.fs.add_file("/w/ro.txt", b"abc");
    s.fs.set_readonly("/w/ro.txt", true);
    open(&mut s, "/w/ro.txt");
    assert!(
        doc(&s)
            .1
            .read_only
            .contains(eve_core::ReadOnlyReason::FileAttribute)
    );
    run(
        &mut s,
        "file.save_as",
        Args::new().with_str("path", "/w/copy.txt"),
    );
    assert_eq!(s.fs.read("/w/copy.txt").unwrap(), b"abc");
    assert!(doc(&s).1.is_editable(), "保存後は編集できる");
    // 元のファイルは変わらない
    assert_eq!(s.fs.read("/w/ro.txt").unwrap(), b"abc");
}

// verifies: AC-FR-116-4
#[test]
fn readonly_の指定と内容由来の理由は別名で保存しても残る() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc");
    s.editor.state_mut().open_path(
        "/w/a.txt",
        eve_core::OpenOptions {
            user_read_only: true,
            ..Default::default()
        },
    );
    s.run_jobs();
    run(
        &mut s,
        "file.save_as",
        Args::new().with_str("path", "/w/copy.txt"),
    );
    assert!(
        doc(&s)
            .1
            .read_only
            .contains(eve_core::ReadOnlyReason::UserRequested)
    );
    assert!(!doc(&s).1.is_editable());
}

fn conflict_title(s: &mut Harness) -> String {
    let f = frame(s);
    let eve_view::OverlayModel::Confirm { title, .. } = &f.overlays[0] else {
        panic!("確認ダイアログではありません");
    };
    title.spans.iter().map(|sp| sp.text.to_string()).collect()
}

fn externally_changed() -> Harness {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc");
    open(&mut s, "/w/a.txt");
    type_text(&mut s, "X");
    messages(&mut s);
    s.clock.advance(eve_core::Millis(10));
    s.fs.set_file(&FsPath::from("/w/a.txt"), b"theirs");
    run(&mut s, "file.save", Args::new());
    s
}

// verifies: AC-FR-117-1
#[test]
fn 外部で変更されていたら書かずに尋ね_上書きを選ぶと保存される() {
    let mut s = externally_changed();
    assert_eq!(conflict_title(&mut s), "外部で変更されています");
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        b"theirs",
        "まだ書いていない"
    );
    type_text(&mut s, "o");
    s.run_jobs();
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"Xabc");
    assert!(!doc(&s).1.is_dirty());
}

// verifies: AC-FR-117-1
#[test]
fn 外部で変更されていたら_別名で保存を選ぶと名前の入力へ進む() {
    let mut s = externally_changed();
    type_text(&mut s, "a");
    s.run_jobs();
    assert_eq!(s.overlay(), "prompt");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"theirs");
}

// verifies: AC-FR-117-1
#[test]
fn 外部で削除されていたら_上書きでファイルを作り直す() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc");
    open(&mut s, "/w/a.txt");
    type_text(&mut s, "X");
    s.fs.remove(&FsPath::from("/w/a.txt"));
    run(&mut s, "file.save", Args::new());
    assert_eq!(conflict_title(&mut s), "外部で削除されています");
    assert!(!s.fs.exists("/w/a.txt"));
    type_text(&mut s, "o");
    s.run_jobs();
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"Xabc");
}

// verifies: AC-FR-117-1
#[test]
fn 新規ファイルの保存先に後から同名のファイルができていたら尋ねる() {
    let mut s = env();
    open(&mut s, "/w/new.txt");
    type_text(&mut s, "mine");
    s.fs.add_file("/w/new.txt", b"theirs");
    run(&mut s, "file.save", Args::new());
    assert_eq!(conflict_title(&mut s), "同名のファイルがあります");
    assert_eq!(s.fs.read("/w/new.txt").unwrap(), b"theirs");
    type_text(&mut s, "o");
    s.run_jobs();
    assert_eq!(s.fs.read("/w/new.txt").unwrap(), b"mine");
}

// verifies: AC-FR-117-1
#[test]
fn 競合の確認は_esc_で中止し_何も書かない() {
    let mut s = externally_changed();
    keys(&mut s, "esc");
    assert!(!dialog_open(&s));
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"theirs");
    assert!(doc(&s).1.is_dirty());
}

// verifies: AC-NFR-301-2
#[test]
fn 置換の途中の失敗は元のファイルの退避先のパスを示す() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc");
    open(&mut s, "/w/a.txt");
    type_text(&mut s, "d");
    messages(&mut s);
    s.fs.fail_next(
        eve_testkit::FsOp::WriteAtomic,
        eve_core::FsErrorKind::ReplacedMovedToBackup {
            backup: FsPath::from("/w/.a.txt.bak123"),
        },
    );
    run(&mut s, "file.save", Args::new());
    assert!(dialog_open(&s), "続く確認で別名で保存できる");
    let f = frame(&mut s);
    let eve_view::OverlayModel::Confirm { message, .. } = &f.overlays[0] else {
        panic!()
    };
    let text: String = message.spans.iter().map(|sp| sp.text.to_string()).collect();
    assert!(text.contains(".a.txt.bak123"), "{text}");
}

// 03 §9.3 手順 6、02 §4.5: 保存は成功したが知らせること (`SaveReport.warnings`) を黙って捨てない。
// 最も重い 1 つを、保存の完了のメッセージの後に警告として出す (docs/review/v0.3-findings.md の SAF-05)
#[test]
fn 保存の警告は最も重い_1_つを完了の後に出す() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc\n");
    open(&mut s, "/w/a.txt");
    type_text(&mut s, "x");
    messages(&mut s);
    s.fs.warn_next(vec![
        eve_core::SaveWarning::AttrsNotPreserved {
            what: vec![eve_core::AttrKind::Security],
        },
        eve_core::SaveWarning::FsyncFailed,
    ]);
    run(&mut s, "file.save", Args::new());
    let m = messages(&mut s);
    assert!(!doc(&s).1.is_dirty(), "保存は成功のまま");
    let done = m.iter().position(|k| k == "info.save.done").expect("{m:?}");
    let warn = m
        .iter()
        .position(|k| k == "msg.save.fsync_failed")
        .expect("fsync の失敗を知らせる");
    assert!(done < warn, "完了の後に出す: {m:?}");
    assert!(
        !m.contains(&"msg.save.attributes_not_copied".to_owned()),
        "出すのは最も重い 1 つ: {m:?}"
    );
}

#[test]
fn 警告の無い保存は完了だけを出す() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc\n");
    open(&mut s, "/w/a.txt");
    type_text(&mut s, "x");
    messages(&mut s);
    run(&mut s, "file.save", Args::new());
    let m = messages(&mut s);
    assert_eq!(m, vec!["info.save.done".to_owned()], "{m:?}");
}

// 03 §17.1: Revision は文書ごとに単調増加。開き直し (同じ DocumentId の差し替え) で 0 に戻さない。戻ると、開き直しの
// 前に投入したジョブ (すべて置換など) の古い結果を新しいと取り違え得る (docs/review/v0.3-findings.md の SAF-06)
#[test]
fn 開き直しても_revision_は戻らない() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc\n");
    open(&mut s, "/w/a.txt");
    type_text(&mut s, "x");
    type_text(&mut s, "y");
    run(&mut s, "file.save", Args::new());
    let (id, d) = doc(&s);
    let before = d.revision();
    assert!(before.0 > 0);
    run(
        &mut s,
        "file.reopen_with_encoding",
        Args::new().with_str("encoding", "shift_jis"),
    );
    let (id2, d) = doc(&s);
    assert_eq!(id2, id);
    assert!(
        d.revision() > before,
        "開き直しの後の Revision ({:?}) は前 ({before:?}) より大きい",
        d.revision()
    );
}

// 03 §9.7 の AskUnsaved の行: 読み取り専用の dirty な文書の「保存」は同じパスへ書かず、名前を尋ねる
// (docs/review/v0.3-findings.md の SAF-18)
#[test]
fn 読み取り専用の文書の未保存の確認で保存を選ぶと_同じパスに書かずに名前を尋ねる() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc\n");
    open(&mut s, "/w/a.txt");
    type_text(&mut s, "x");
    run(&mut s, "file.toggle_read_only", Args::new());
    assert!(!doc(&s).1.read_only.is_empty());
    run(&mut s, "app.quit", Args::new());
    assert!(dialog_open(&s), "未保存の確認");
    type_text(&mut s, "s");
    s.run_jobs();
    assert_eq!(s.overlay(), "prompt", "名前を尋ねる");
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        b"abc\n",
        "同じパスに書かない"
    );
}
