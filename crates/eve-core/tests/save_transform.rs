//! 保存時の変換 (末尾空白の除去・末尾の改行の付与。docs/03 §9.3.1 の SVT-1〜SVT-9、§15.9.1 の L3、01 FR-216 AC-1〜AC-3。
//! v0.5 の M34 の段 3)。
//!
//! 保存の連鎖の各入口 (保存・名前を付けて保存・全保存・終了と閉じる前の保存・文字コードを指定して保存) で効くこと、効かない
//! 保存 (未編集・パスが変わる・値が届く前・1 MiB 超)、確認の後の投入し直しで二度当てないこと、書けずに終わっても本文に残り
//! 1 回の Undo で戻り、失敗・中止のメッセージに付記が出ること、承認の引き継ぎ、区分 B / D の往復を確かめる。

use eve_config::{Conditions, Config, DefaultLineEnding, Frontend, Os};
use eve_core::{
    Args, ConfigSources, Encoding, FsErrorKind, FsPath, InitialFile, JobOutput, JobResult,
    OpenOptions, StartupConfig,
};
use eve_encoding::PreserveStatus;
use eve_testkit::{FsOp, Harness};
use eve_text::{ByteOff, Selection, TextRead};
use eve_view::{Answer, EditorEvent, OriginTag, OverlayModel, Query};

const PATH: &str = "/w/a.txt";

fn config(trim: bool, final_newline: bool) -> Config {
    Config {
        trim_trailing_whitespace_on_save: trim,
        insert_final_newline_on_save: final_newline,
        ..Config::default()
    }
}

/// `files` を置き、設定 `cfg` で起動して `open` を開いた Harness (画面は付記が切れないように広くする)。
fn start_with(cfg: Config, files: &[(&str, &[u8])], open: &str) -> Harness {
    let mut b = Harness::builder()
        .defer_start()
        .size(400, 20)
        .config(cfg)
        .dir("/w");
    for (p, d) in files {
        b = b.file(p, d);
    }
    let mut h = b.build();
    h.editor
        .open_initial_files(&[InitialFile::path(open)], false, None);
    h.started();
    h.run_jobs();
    h
}

fn start(cfg: Config, text: &str) -> Harness {
    start_with(cfg, &[(PATH, text.as_bytes())], PATH)
}

/// 文書の先頭に `s` を打って dirty にする (Undo の単位を分けるため、打つ前に時計を進める)。
fn type_at_start(h: &mut Harness, s: &str) {
    h.advance_ms(5_000);
    h.keys("ctrl+home");
    h.text(s);
    h.advance_ms(5_000);
}

fn save(h: &mut Harness) {
    h.cmd("file.save");
    h.run_jobs();
}

fn disk(h: &Harness, path: &str) -> String {
    String::from_utf8(h.fs.read(path).expect("ファイルがある")).expect("UTF-8")
}

fn undo(h: &mut Harness) {
    h.cmd("edit.undo");
}

fn revision(h: &Harness) -> eve_view::Revision {
    let id = h.editor.state().active_document().expect("文書");
    h.editor.state().document(id).expect("文書").revision()
}

/// 今のメッセージの本文。
fn message(h: &mut Harness) -> String {
    let f = h.frame();
    f.messages
        .current
        .map(|m| m.text.spans.iter().map(|s| s.text.as_str()).collect())
        .unwrap_or_default()
}

/// 出ている確認の (題, 本文, details)。確認が無ければ `None`。
fn dialog(h: &Harness) -> Option<(String, String, Vec<String>)> {
    let window = h.editor.state().first_window()?;
    let text =
        |c: &eve_view::CellText| -> String { c.spans.iter().map(|sp| sp.text.as_str()).collect() };
    match h.query(&Query::Overlay { window }).ok()? {
        Answer::Overlay(Some(OverlayModel::Confirm {
            title,
            message,
            details,
            ..
        })) => Some((
            text(&title),
            text(&message),
            details.iter().flatten().map(text).collect(),
        )),
        _ => None,
    }
}

/// 変換が本文に残った旨 (`info.save.suffix.transform_kept`) の文言の頭。
const KEPT: &str = "保存時の変換 (行末の空白・末尾の改行) は本文に残っています";
/// `info.save.suffix.transform_kept` の全体 (変換のノードが今の位置のとき)。
const KEPT_UNDO: &str =
    "保存時の変換 (行末の空白・末尾の改行) は本文に残っています。Ctrl+Z で戻せます";
/// `info.save.suffix.transform_reverted`。
const REVERTED: &str = "保存時の変換 (行末の空白・末尾の改行) は戻しました";

/// これまでに保存時の変換の Transaction (`Origin::Command`、Undo でない) が当たった回数 (`clear_log` から)。
fn transforms(h: &Harness) -> usize {
    h.events()
        .iter()
        .filter(|e| {
            matches!(e, EditorEvent::DocumentChanged { origin_kind, .. }
                if origin_kind.origin == OriginTag::Command && !origin_kind.is_undo)
        })
        .count()
}

// ---------- AC-1: 変換の中身・知らせ・Undo ----------

// verifies: AC-FR-216-1
#[test]
fn 保存で末尾の空白を除き末尾の改行を足し_未変更になり_付記を出し_1_回の_undo_で戻る() {
    let mut h = start(config(true, true), "a  \nb\t\n c \u{3000}\n\t\nd");
    type_at_start(&mut h, "x");
    let before = h.doc_text();
    h.clear_log();
    save(&mut h);
    // 全角の空白は残す。空白だけの行は空の行になる。CR と LF の例外の無い文書
    let want = "xa\nb\n c \u{3000}\n\nd\n";
    assert_eq!(disk(&h, PATH), want);
    assert_eq!(h.doc_text(), want, "保存した内容とバッファが一致する");
    assert!(!h.is_dirty(), "未変更の表示になる");
    assert_eq!(
        message(&mut h),
        "/w/a.txt を保存しました (行末の空白を 3 行で除きました、末尾に改行を足しました)"
    );
    assert_eq!(
        h.messages().last().map(String::as_str),
        Some("info.save.done")
    );
    // 1 回の Undo で変換が戻り、dirty になる。もう 1 回で打った文字が戻る
    undo(&mut h);
    assert_eq!(h.doc_text(), before);
    assert!(h.is_dirty());
    undo(&mut h);
    assert_eq!(h.doc_text(), "a  \nb\t\n c \u{3000}\n\t\nd");
}

// verifies: AC-FR-216-1
#[test]
fn 末尾の改行は区切りで終わる文書と空の文書には足さず_取り除かない() {
    for (text, trim, fin, want) in [
        // 除去の後に足すかを決める (`a⏎··` は除去で `a⏎` になり、足さない)
        ("a\n  ", true, true, "a\n"),
        ("a\r", false, true, "a\r"),
        ("a\r\n", false, true, "a\r\n"),
        // 設定が偽でも末尾の改行は取り除かない
        ("a\n\n", true, false, "a\n\n"),
    ] {
        let mut h = start(config(trim, fin), text);
        type_at_start(&mut h, "x");
        save(&mut h);
        assert_eq!(disk(&h, PATH), format!("x{want}"), "{text:?}");
    }
    // 空白だけの文書は空になり、改行を足さない
    let mut h = start(config(true, true), "");
    h.text("   ");
    save(&mut h);
    assert_eq!(disk(&h, PATH), "");
}

// verifies: AC-FR-216-1
#[test]
fn 足す改行は文書の既定の改行で_区切りの無い文書は_default_line_ending_の値() {
    let mut h = start(config(false, true), "a\r\nb");
    type_at_start(&mut h, "x");
    save(&mut h);
    assert_eq!(disk(&h, PATH), "xa\r\nb\r\n");
    let cfg = Config {
        default_line_ending: DefaultLineEnding::Crlf,
        ..config(false, true)
    };
    let mut h = start(cfg, "abc");
    type_at_start(&mut h, "x");
    save(&mut h);
    assert_eq!(disk(&h, PATH), "xabc\r\n");
}

// verifies: AC-FR-216-1
#[test]
fn 文書の末尾のカーソルは足した改行の前に留まり_続けて打つ文字は同じ行に入る() {
    let mut h = start(config(true, true), "abc  ");
    h.keys("ctrl+end");
    h.text("d");
    save(&mut h);
    assert_eq!(disk(&h, PATH), "abc  d\n");
    h.text("e");
    assert_eq!(h.doc_text(), "abc  de\n");
    // 消した空白の中のカーソルは行末へ寄る
    let mut h = start(config(true, true), "ab   \ncd");
    type_at_start(&mut h, "x");
    let v = h
        .editor
        .state()
        .first_window()
        .and_then(|w| h.editor.state().window_view(w))
        .expect("View");
    h.editor
        .state_mut()
        .set_selection(v, Selection::point(ByteOff(5)));
    save(&mut h);
    assert_eq!(disk(&h, PATH), "xab\ncd\n");
    h.text("y");
    assert_eq!(h.doc_text(), "xaby\ncd\n");
}

// verifies: AC-FR-216-1
#[test]
fn 改行の混在で_cr_と_lf_がつながる行は除かず_行の数が変わらない() {
    let mut h = start(config(true, false), "a\r \nb ");
    type_at_start(&mut h, "x");
    let lines = h
        .editor
        .state()
        .document(h.doc_id())
        .expect("文書")
        .text()
        .len_lines();
    save(&mut h);
    assert_eq!(disk(&h, PATH), "xa\r \nb");
    let d = h.editor.state().document(h.doc_id()).expect("文書");
    assert_eq!(d.text().len_lines(), lines);
}

// verifies: AC-FR-216-1
#[test]
fn 最後の文字が不正バイトなら改行を足さない() {
    // 不正バイトの比率で読み取り専用 (バイナリ) にならないよう、前に長い行を置く
    let pad = "0123456789".repeat(20) + "\n";
    let bytes = [pad.as_bytes(), b"abc\xFF"].concat();
    let mut h = start_with(config(false, true), &[(PATH, &bytes)], PATH);
    // 不正バイトを含む文書の編集を許可してから編集する (03 §8.3.1)
    h.cmd("file.toggle_read_only");
    assert_eq!(h.overlay(), "dialog");
    h.text("a");
    type_at_start(&mut h, "x");
    save(&mut h);
    assert_eq!(
        h.fs.read(PATH).expect("ある"),
        [b"x", bytes.as_slice()].concat()
    );
}

// verifies: AC-FR-216-1
#[test]
fn 言語ごとに替わり_組込みの_markdown_と_diff_は末尾の空白を除かない() {
    for (path, want) in [
        ("/w/a.md", "xa  \n"),
        ("/w/a.diff", "xa  \n"),
        ("/w/a.rs", "xa\n"),
    ] {
        let mut h = start_with(config(true, true), &[(path, b"a  ")], path);
        type_at_start(&mut h, "x");
        save(&mut h);
        assert_eq!(disk(&h, path), want, "{path}");
    }
}

// verifies: AC-FR-216-1
#[test]
fn 設定が偽なら_v0_4_と同じバイト列を書き_付記を出さない() {
    let mut h = start(Config::default(), "a  \nb\t");
    type_at_start(&mut h, "x");
    let rev = revision(&h);
    save(&mut h);
    assert_eq!(disk(&h, PATH), "xa  \nb\t");
    assert_eq!(revision(&h), rev, "Transaction を作らない");
    assert_eq!(message(&mut h), "/w/a.txt を保存しました");
    // 変える所が無ければ Transaction を作らない
    let mut h = start(config(true, true), "a\n");
    type_at_start(&mut h, "x");
    let rev = revision(&h);
    save(&mut h);
    assert_eq!(revision(&h), rev);
    assert_eq!(message(&mut h), "/w/a.txt を保存しました");
}

// verifies: AC-FR-216-1
#[test]
fn 同じファイルへの名前を付けて保存と文字コードを指定して保存でも効く() {
    let mut h = start(config(true, true), "a  ");
    type_at_start(&mut h, "x");
    h.cmd_with("file.save_as", Args::new().with_str("path", PATH));
    h.run_jobs();
    assert_eq!(disk(&h, PATH), "xa\n");
    let mut h = start(config(true, true), "a  ");
    type_at_start(&mut h, "x");
    h.cmd_with(
        "file.set_encoding",
        Args::new().with_str("encoding", "utf-16le"),
    );
    h.run_jobs();
    let want: Vec<u8> = [0xFF_u8, 0xFE]
        .into_iter()
        .chain("xa\n".encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    assert_eq!(h.fs.read(PATH).expect("ある"), want);
}

// verifies: AC-FR-216-1
#[test]
fn 全保存は保存する文書ごとに当て_要約に件数を付ける() {
    let mut h = start_with(
        config(true, false),
        &[
            ("/w/a.txt", b"a  "),
            ("/w/b.txt", b"b"),
            ("/w/c.txt", b"c "),
        ],
        "/w/a.txt",
    );
    type_at_start(&mut h, "x");
    h.open("/w/b.txt");
    h.run_jobs();
    type_at_start(&mut h, "y");
    // c は開くだけ (dirty でない文書には当てない)
    h.open("/w/c.txt");
    h.run_jobs();
    h.cmd("file.save_all");
    h.run_jobs();
    assert_eq!(disk(&h, "/w/a.txt"), "xa");
    assert_eq!(disk(&h, "/w/b.txt"), "yb");
    assert_eq!(disk(&h, "/w/c.txt"), "c ");
    assert_eq!(
        message(&mut h),
        "2 件の文書を保存しました (うち 1 件は保存時に行末の空白・末尾の改行を整えました)"
    );
}

// verifies: AC-FR-216-1
#[test]
fn 終了の連鎖と閉じる前の保存でも効く() {
    let mut h = start_with(
        config(true, false),
        &[("/w/a.txt", b"a  "), ("/w/b.txt", b"b  ")],
        "/w/a.txt",
    );
    type_at_start(&mut h, "x");
    // 閉じる前の保存 (`Continuation::Close`)
    h.cmd("buffer.close");
    assert_eq!(h.overlay(), "dialog");
    let a = h.doc_id();
    h.keys("s");
    h.run_jobs();
    assert_eq!(disk(&h, "/w/a.txt"), "xa");
    assert!(h.editor.state().document(a).is_none(), "保存の後に閉じた");
    // 終了の連鎖の保存
    h.open("/w/b.txt");
    h.run_jobs();
    type_at_start(&mut h, "y");
    h.cmd("app.quit");
    h.keys("s");
    h.run_jobs();
    assert_eq!(disk(&h, "/w/b.txt"), "yb");
    assert!(h.quit_code().is_some());
}

// ---------- AC-2: 効かない保存・二度当てない・書けずに終わったとき ----------

// verifies: AC-FR-216-2
#[test]
fn 未編集の保存には効かせない() {
    let mut h = start(config(true, true), "a  ");
    save(&mut h);
    assert!(h.messages().contains(&"info.save.unchanged".to_owned()));
    // 未編集の名前を付けて保存 (別のファイル・同じファイル) と文字コードを指定して保存
    h.cmd_with("file.save_as", Args::new().with_str("path", "/w/b.txt"));
    h.run_jobs();
    assert_eq!(disk(&h, "/w/b.txt"), "a  ");
    h.cmd_with(
        "file.set_encoding",
        Args::new().with_str("encoding", "utf-8"),
    );
    h.run_jobs();
    assert_eq!(disk(&h, "/w/b.txt"), "a  ");
    assert_eq!(h.doc_text(), "a  ");
    // 未編集で、照合が一致せずに書く (`CheckingUnchanged` の不一致) 文書にも当てない
    let mut h = start(config(true, true), "a  ");
    h.fs.set_file(&FsPath::new(PATH), b"other");
    save(&mut h);
    // 外部変更の確認で上書き
    assert!(dialog(&h).is_some());
    h.answer(0);
    h.run_jobs();
    assert_eq!(disk(&h, PATH), "a  ");
}

// verifies: AC-FR-216-2
#[test]
fn パスの変わる保存には効かせない() {
    // 別のファイルへの名前を付けて保存
    let mut h = start(config(true, true), "a  ");
    type_at_start(&mut h, "x");
    h.cmd_with("file.save_as", Args::new().with_str("path", "/w/b.txt"));
    h.run_jobs();
    assert_eq!(disk(&h, "/w/b.txt"), "xa  ");
    // 次の保存 (2 回目) からかかる
    h.text("y");
    save(&mut h);
    assert_eq!(disk(&h, "/w/b.txt"), "xya\n");
    // 無題の文書の最初の保存
    let mut h = start(config(true, true), "");
    h.cmd("file.new");
    h.text("u  ");
    h.cmd("file.save");
    h.text("/w/u.txt");
    h.keys("enter");
    h.run_jobs();
    assert_eq!(disk(&h, "/w/u.txt"), "u  ");
}

// verifies: AC-FR-216-2
#[test]
fn 確認の後の投入し直しでは二度当てない() {
    // Busy の再試行
    let mut h = start(config(true, true), "a  ");
    type_at_start(&mut h, "x");
    h.fs.fail_next(FsOp::WriteAtomic, FsErrorKind::Busy);
    save(&mut h);
    let rev = revision(&h);
    let (_, body, _) = dialog(&h).expect("保存の失敗の確認");
    assert!(body.contains(KEPT), "{body}");
    h.keys("t");
    h.run_jobs();
    assert_eq!(disk(&h, PATH), "xa\n");
    assert_eq!(revision(&h), rev, "もう一度は変換しない");
    // 外部変更の確認で上書き
    let mut h = start(config(true, true), "a  ");
    type_at_start(&mut h, "x");
    h.fs.set_file(&FsPath::new(PATH), b"other");
    save(&mut h);
    let rev = revision(&h);
    assert!(dialog(&h).is_some());
    h.answer(0);
    h.run_jobs();
    assert_eq!(disk(&h, PATH), "xa\n");
    assert_eq!(revision(&h), rev);
    undo(&mut h);
    assert_eq!(h.doc_text(), "xa  ", "変換は 1 つの Undo のノード");
}

// verifies: AC-FR-216-2
#[test]
fn 別名で保存へ移ったら前のパスの値で当てた変換のまま新しいパスへ書く() {
    let mut h = start_with(config(true, true), &[(PATH, b"a  ")], PATH);
    type_at_start(&mut h, "x");
    h.fs.fail_next(FsOp::WriteAtomic, FsErrorKind::Busy);
    save(&mut h);
    let rev = revision(&h);
    // 別名で保存 (既定) → 名前の入力 (markdown は末尾の空白を除かない言語だが、判定し直さない)
    h.keys("a");
    assert_eq!(h.overlay(), "prompt");
    h.text("/w/b.md");
    h.keys("enter");
    h.run_jobs();
    assert_eq!(disk(&h, "/w/b.md"), "xa\n");
    assert_eq!(revision(&h), rev);
}

// verifies: AC-FR-216-2
#[test]
fn 書けずに終わっても変換は本文に残り_1_回の_undo_で戻り_中止のメッセージに付記する() {
    // Busy で中止
    let mut h = start(config(true, true), "a  ");
    type_at_start(&mut h, "x");
    h.fs.fail_next(FsOp::WriteAtomic, FsErrorKind::Busy);
    save(&mut h);
    h.keys("esc");
    assert_eq!(h.doc_text(), "xa\n");
    assert!(h.is_dirty());
    assert_eq!(
        message(&mut h),
        format!("保存を中止しました (/w/a.txt は書き換えていません) {KEPT_UNDO}")
    );
    undo(&mut h);
    assert_eq!(h.doc_text(), "xa  ");
    assert_eq!(disk(&h, PATH), "a  ");
    // 外部変更で中止 (確認のキャンセルは中止。03 §9.7 の「中止」)
    let mut h = start(config(true, true), "a  ");
    type_at_start(&mut h, "x");
    h.fs.set_file(&FsPath::new(PATH), b"other");
    save(&mut h);
    h.keys("esc");
    assert_eq!(
        message(&mut h),
        format!("保存を中止しました (/w/a.txt は書き換えていません) {KEPT_UNDO}")
    );
    assert_eq!(h.doc_text(), "xa\n");
    // 変換が無ければ (設定が偽) 外部変更のキャンセルは中止のメッセージだけを出し、付記を付けない (#647 で、確認の
    // キャンセルでも `info.save.aborted` を出すように直した。v0.4 までは何も出さなかった)
    let mut h = start(Config::default(), "a  ");
    type_at_start(&mut h, "x");
    h.fs.set_file(&FsPath::new(PATH), b"other");
    save(&mut h);
    h.clear_log();
    h.keys("esc");
    assert_eq!(h.messages(), ["info.save.aborted"]);
    assert_eq!(
        message(&mut h),
        "保存を中止しました (/w/a.txt は書き換えていません)"
    );
    // 退避先に保存 (容量不足。書込権限なしは変換を戻す、下の試験)
    let mut h = start(config(true, true), "a  ");
    type_at_start(&mut h, "x");
    h.fs.fail_next(FsOp::WriteAtomic, FsErrorKind::DiskFull);
    save(&mut h);
    let (_, body, _) = dialog(&h).expect("保存の失敗の確認");
    assert!(body.contains(KEPT), "{body}");
    h.keys("r");
    h.run_jobs();
    let m = message(&mut h);
    assert!(m.starts_with("退避先"), "{m}");
    assert!(m.contains(KEPT), "{m}");
    assert_eq!(h.fs.recovery_written().len(), 1);
}

// verifies: AC-FR-216-2
#[test]
fn 終了の連鎖の保存が取り消されて中止したときも変換は残り付記し_終了しない() {
    let mut h = start(config(true, true), "a  ");
    type_at_start(&mut h, "x");
    h.cmd("app.quit");
    h.keys("s");
    // 保存のジョブの取消し (`JobError::Cancelled`。何も書いていない)
    h.cancel_jobs();
    h.run_jobs();
    assert!(h.quit_code().is_none(), "後続 (終了) は行わない");
    assert_eq!(h.doc_text(), "xa\n");
    assert!(h.is_dirty());
    assert_eq!(disk(&h, PATH), "a  ");
    assert_eq!(
        message(&mut h),
        format!("保存を中止しました (/w/a.txt は書き換えていません) {KEPT_UNDO}")
    );
    undo(&mut h);
    assert_eq!(h.doc_text(), "xa  ");
}

// verifies: AC-FR-216-2
#[test]
fn 全保存の失敗の一覧の理由に変換が残った旨を付ける() {
    let mut h = start(config(true, false), "a  ");
    type_at_start(&mut h, "x");
    h.fs.fail_next(FsOp::WriteAtomic, FsErrorKind::Busy);
    h.cmd("file.save_all");
    h.run_jobs();
    let (_, _, details) = dialog(&h).expect("失敗の一覧");
    assert_eq!(details.len(), 1);
    assert!(details[0].contains(KEPT), "{details:?}");
    assert_eq!(h.doc_text(), "xa");
    assert!(h.is_dirty());
    h.keys("esc");
    undo(&mut h);
    assert_eq!(h.doc_text(), "xa  ");
}

// ---------- AC-3: 1 MiB 超 ----------

// verifies: AC-FR-216-3
#[test]
fn 上限を超える文書は変換せずに保存し_完了のメッセージで知らせる() {
    let mut h = start(config(true, true), "a  \nb  ");
    h.editor.state_mut().set_transform_sync_bytes(4);
    type_at_start(&mut h, "x");
    save(&mut h);
    assert_eq!(disk(&h, PATH), "xa  \nb  ");
    assert_eq!(
        message(&mut h),
        "/w/a.txt を保存しました (1 MiB を超える文書なので、行末の空白の除去と末尾の改行はしていません)"
    );
    // 設定が偽なら知らせない
    let mut h = start(Config::default(), "a  \nb  ");
    h.editor.state_mut().set_transform_sync_bytes(4);
    type_at_start(&mut h, "x");
    save(&mut h);
    assert_eq!(message(&mut h), "/w/a.txt を保存しました");
}

// ---------- SVT-5: 承認の引き継ぎ ----------

/// 旧 JIS の行 (記録) と、ASCII の行と、保持できなかった最後の行 (区分 D の確認が出る文書。`preserve_save.rs` と同じ)。
const ISO_PARTIAL: &[u8] = b"\x1B$@$\"\x1B(B\nabc\n\x1B(J\\\n";

fn open_as(cfg: Config, bytes: &[u8], enc: Encoding) -> Harness {
    let mut h = Harness::builder()
        .size(400, 20)
        .config(cfg)
        .file(PATH, bytes)
        .build();
    h.editor.state_mut().open_path(
        PATH,
        OpenOptions {
            forced: Some(enc),
            ..OpenOptions::default()
        },
    );
    h.run_jobs();
    h
}

// verifies: AC-FR-216-2
#[test]
fn 区分_d_の確認の承認を変換の後に引き継ぎ_busy_の再試行で確認を出し直さない() {
    let mut h = open_as(config(true, false), ISO_PARTIAL, Encoding::Iso2022Jp);
    // 2 行目の末尾に空白を打つ
    h.keys("ctrl+home");
    h.keys("down");
    h.keys("end");
    h.text("  ");
    h.fs.fail_next(FsOp::WriteAtomic, FsErrorKind::Busy);
    h.cmd("file.save");
    let (title, _, _) = dialog(&h).expect("保存の前の確認");
    assert_eq!(title, "エスケープの書き方が変わります");
    h.keys("s");
    h.run_jobs();
    // 変換の後の Busy: 保存の失敗の確認
    let (_, body, _) = dialog(&h).expect("保存の失敗の確認");
    assert!(body.contains(KEPT), "{body}");
    h.keys("t");
    h.run_jobs();
    assert!(dialog(&h).is_none(), "区分 D の確認を出し直さない");
    assert!(!h.is_dirty());
    let saved = h.fs.read(PATH).expect("ある");
    assert_eq!(saved, b"\x1B$@$\"\x1B(B\nabc\n\x1B(J\\\x1B(B\n");
}

// ---------- SVT-7: 区分 B / D の往復 ----------

fn sjis(s: &str) -> Vec<u8> {
    encoding_rs::SHIFT_JIS.encode(s).0.into_owned()
}

fn euc(s: &str) -> Vec<u8> {
    encoding_rs::EUC_JP.encode(s).0.into_owned()
}

// verifies: AC-FR-216-3
#[test]
fn 区分_b_の記録と変換しない行のバイトは保たれる() {
    // Shift_JIS の区分 B (NEC 特殊文字・IBM 拡張・NEC 選定 IBM 拡張) を、空白を除く行と除かない行に置く
    let l1 = [
        sjis("B: "),
        b"\x87\x90\xFA\x54\xED\x40".to_vec(),
        b"  \r\n".to_vec(),
    ]
    .concat();
    let l2 = [b"x".to_vec(), b"\xEE\xEF".to_vec(), sjis("おわり\r\n")].concat();
    let bytes = [l1.clone(), l2.clone()].concat();
    let mut h = open_as(config(true, true), &bytes, Encoding::ShiftJis);
    assert!(matches!(
        h.editor
            .state()
            .document(h.doc_id())
            .expect("文書")
            .format
            .roundtrip
            .preserve,
        PreserveStatus::Active { unpreserved: 0 }
    ));
    // 2 行目の末尾に打って dirty にする (1 行目は空白を除くだけ)
    h.keys("ctrl+end");
    h.keys("up");
    h.keys("end");
    h.text("!");
    h.cmd("file.save");
    h.run_jobs();
    assert!(dialog(&h).is_none());
    let want = [
        sjis("B: "),
        b"\x87\x90\xFA\x54\xED\x40".to_vec(),
        b"\r\n".to_vec(),
        b"x".to_vec(),
        b"\xEE\xEF".to_vec(),
        sjis("おわり!\r\n"),
    ]
    .concat();
    assert_eq!(h.fs.read(PATH).expect("ある"), want);
    // EUC-JP の補助漢字 (3 バイト) も同じ
    let bytes = [
        euc("補: "),
        b"\x8F\xB0\xA1".to_vec(),
        b" \t\n".to_vec(),
        euc("b"),
    ]
    .concat();
    let mut h = open_as(config(true, true), &bytes, Encoding::EucJp);
    h.keys("ctrl+end");
    h.text("!");
    h.cmd("file.save");
    h.run_jobs();
    let want = [
        euc("補: "),
        b"\x8F\xB0\xA1".to_vec(),
        b"\n".to_vec(),
        euc("b!\n"),
    ]
    .concat();
    assert_eq!(h.fs.read(PATH).expect("ある"), want);
}

// verifies: AC-FR-216-3
#[test]
fn iso_2022_jp_の変換した行は正規形で書き直り_ほかの行は保たれる() {
    // 1 行目は旧 JIS の記録の行の末尾に空白、2 行目は記録の行のまま、3 行目を編集する
    let bytes = b"\x1B$@$\"\x1B(B  \n\x1B$@$$\x1B(B\nabc\n";
    let mut h = open_as(config(true, false), bytes, Encoding::Iso2022Jp);
    h.keys("ctrl+end");
    h.keys("up");
    h.keys("end");
    h.text("d");
    h.cmd("file.save");
    h.run_jobs();
    assert!(dialog(&h).is_none());
    assert_eq!(
        h.fs.read(PATH).expect("ある"),
        b"\x1B$B$\"\x1B(B\n\x1B$@$$\x1B(B\nabcd\n"
    );
}

// ---------- SVT-1 (b): 値が届く前 ----------

const CONFIG: &str = "/evo/config/config.toml";
const REPO_A: &str = "/w/repo/a.txt";

fn linux() -> Conditions {
    Conditions {
        os: Some(Os::Linux),
        frontend: Frontend::Tui,
        ..Conditions::default()
    }
}

/// `.editorconfig` とワークスペース層を読む起動 (editorconfig.rs と同じ形)。`hold` に当たるジョブの結果は渡さずに返す。
fn start_project(
    user: &str,
    editorconfig: &str,
    hold: impl Fn(&JobResult) -> bool,
) -> (Harness, Vec<JobOutput>) {
    start_project_with(user, Some(editorconfig), None, hold)
}

/// `start_project` に、ワークスペース層 (`.eve/config.toml`) を置く版。結果はどれも渡す。
fn start_project_ws(user: &str, editorconfig: &str, ws: Option<&str>) -> (Harness, Vec<JobOutput>) {
    start_project_with(user, Some(editorconfig), ws, |_| false)
}

fn start_project_with(
    user: &str,
    editorconfig: Option<&str>,
    ws: Option<&str>,
    hold: impl Fn(&JobResult) -> bool,
) -> (Harness, Vec<JobOutput>) {
    let mut b = Harness::builder()
        .defer_start()
        .size(400, 20)
        .dir("/evo/config")
        .dir("/w/repo/.git")
        .file(CONFIG, user.as_bytes())
        .file(REPO_A, b"a  ");
    if let Some(t) = editorconfig {
        b = b.file("/w/repo/.editorconfig", t.as_bytes());
    }
    if let Some(t) = ws {
        b = b.file("/w/repo/.eve/config.toml", t.as_bytes());
    }
    let mut h = b.build();
    let r = eve_config::resolve(
        &[eve_config::Source {
            name: "config.toml".to_owned(),
            text: Ok(Some(user.to_owned())),
        }],
        &[],
        linux(),
    );
    h.editor.configure(StartupConfig {
        config: Some(r.config),
        config_origins: Some(r.origins),
        config_user: Some(r.user),
        config_languages: Some(r.languages),
        config_sources: Some(ConfigSources {
            files: vec![("config.toml".to_owned(), FsPath::new(CONFIG))],
            keymap: None,
            sets: Vec::new(),
            themes: None,
            languages: None,
            locales: None,
            conditions: linux(),
            no_workspace_config: false,
            home: None,
        }),
        workspace: Some("/w/repo".to_owned()),
        ..Default::default()
    });
    h.editor
        .open_initial_files(&[InitialFile::path(REPO_A)], false, None);
    h.started();
    let mut held = Vec::new();
    while h.pending_jobs() > 0 {
        for o in h.run_one_job_held() {
            if hold(&o.body) {
                held.push(o);
            } else {
                h.deliver(vec![o]);
            }
        }
    }
    (h, held)
}

// verifies: AC-FR-216-1, AC-FR-216-2
#[test]
fn editorconfig_の_2_つのプロパティで替わり_届く前は変換しない() {
    let ec = "[*.txt]\ntrim_trailing_whitespace = true\ninsert_final_newline = true\n";
    // 届いた後: ユーザー層が偽でも `.editorconfig` の値で変換する
    let (mut h, _) = start_project("", ec, |_| false);
    type_at_start(&mut h, "x");
    save(&mut h);
    assert_eq!(disk(&h, REPO_A), "xa\n");
    // 届く前: 値が決まっていないので変換しない (ユーザー層が真でも)
    let user = "[editor]\ntrim_trailing_whitespace_on_save = true\n";
    let (mut h, held) = start_project(user, "[*]\ntrim_trailing_whitespace = false\n", |r| {
        matches!(r, JobResult::EditorConfigRead(_))
    });
    assert_eq!(held.len(), 1);
    type_at_start(&mut h, "x");
    save(&mut h);
    assert_eq!(disk(&h, REPO_A), "xa  ");
    // 届いた後は `.editorconfig` の false が勝つ
    h.deliver(held);
    h.text("y");
    save(&mut h);
    assert_eq!(disk(&h, REPO_A), "xya  ");
    // insert_final_newline = false は末尾の改行を取り除かない
    let (mut h, _) = start_project(
        "[editor]\ninsert_final_newline_on_save = true\n",
        "[*]\ninsert_final_newline = false\n",
        |_| false,
    );
    h.keys("ctrl+end");
    h.text("\n");
    save(&mut h);
    assert_eq!(disk(&h, REPO_A), "a  \n");
}

// verifies: AC-FR-216-2
#[test]
fn ワークスペース層の最初の読込を待つ間とルートの検出の前は変換しない() {
    let user = "[editor]\ntrim_trailing_whitespace_on_save = true\n";
    let (mut h, held) = start_project(user, "", |r| matches!(r, JobResult::WorkspaceRoot { .. }));
    assert_eq!(held.len(), 1);
    type_at_start(&mut h, "x");
    save(&mut h);
    assert_eq!(disk(&h, REPO_A), "xa  ", "ルートの検出の結果を待っている");
    let (mut h, held) = start_project(user, "", |r| matches!(r, JobResult::WorkspaceConfigRead(_)));
    assert_eq!(held.len(), 1);
    type_at_start(&mut h, "x");
    save(&mut h);
    assert_eq!(
        disk(&h, REPO_A),
        "xa  ",
        "ワークスペース層の最初の読込を待っている"
    );
    // 届いた後はかかる
    h.deliver(held);
    h.run_jobs();
    h.text("y");
    save(&mut h);
    assert_eq!(disk(&h, REPO_A), "xya");
}

// verifies: AC-FR-216-1
#[test]
fn 設定の読み直しの間は前に当てた値で変換する() {
    let (mut h, _) = start_project("", "[*]\ntrim_trailing_whitespace = true\n", |_| false);
    h.cmd("config.reload");
    // ReadConfig → ReadWorkspaceConfig を当て、`.editorconfig` の読込を積むところまで進める
    assert!(h.run_one_job());
    assert!(h.run_one_job());
    type_at_start(&mut h, "x");
    h.cmd("file.save");
    // `.editorconfig` の読込の結果より前に保存のジョブを走らせる
    h.run_jobs();
    assert_eq!(disk(&h, REPO_A), "xa");
}

// ---------- 入口と確認の後の投入し直し (自己点検の 2・3) ----------

// verifies: AC-FR-216-1
#[test]
fn 保存して終了と開き直しの前の保存でも効く() {
    let mut h = start(config(true, false), "a  ");
    type_at_start(&mut h, "x");
    h.cmd("app.save_and_quit");
    h.run_jobs();
    assert_eq!(disk(&h, PATH), "xa");
    assert_eq!(h.quit_code(), Some(0));
    // 文字コードを指定して開き直す (dirty なので「保存」を尋ねる)
    let mut h = start(config(true, false), "a  ");
    type_at_start(&mut h, "x");
    h.cmd_with(
        "file.reopen_with_encoding",
        Args::new().with_str("encoding", "utf-8"),
    );
    assert_eq!(h.overlay(), "dialog");
    h.keys("s");
    h.run_jobs();
    assert_eq!(disk(&h, PATH), "xa");
}

// verifies: AC-FR-216-2
#[test]
fn 表現不能の確認で_utf_8_で保存しても二度当てない() {
    let mut h = open_as(config(true, false), &sjis("a  \n"), Encoding::ShiftJis);
    h.keys("ctrl+end");
    // Shift_JIS で表せない文字と、行末の空白
    h.text("\u{1F600}  ");
    h.cmd("file.save");
    h.run_jobs();
    let rev = revision(&h);
    let (title, _, _) = dialog(&h).expect("表現不能の確認");
    assert_ne!(title, "");
    h.keys("u");
    h.run_jobs();
    assert_eq!(revision(&h), rev, "もう一度は変換しない");
    assert_eq!(disk(&h, PATH), "a\n\u{1F600}");
}

// verifies: AC-FR-216-2
#[test]
fn 読み取り専用を解除して保存する流れでは解除の後に当て_再試行で確認を出し直さない() {
    let mut h = start(config(true, false), "a  ");
    h.fs.set_readonly(PATH, true);
    h.cmd_with(
        "file.reopen_with_encoding",
        Args::new().with_str("encoding", "utf-8"),
    );
    h.run_jobs();
    h.cmd("file.toggle_read_only");
    type_at_start(&mut h, "x");
    h.fs.fail_next(FsOp::WriteAtomic, FsErrorKind::Busy);
    h.cmd("file.save");
    let (title, _, _) = dialog(&h).expect("読み取り専用を解除して保存の確認");
    assert!(title.contains("読み取り専用"), "{title}");
    // 確認の間は変換していない
    assert_eq!(h.doc_text(), "xa  ");
    h.keys("w");
    h.run_jobs();
    assert_eq!(h.doc_text(), "xa", "解除の後なのでかける");
    // Busy の再試行: 読み取り専用の解除の確認は出し直さない (承認を引き継ぐ)
    h.keys("t");
    h.run_jobs();
    assert!(dialog(&h).is_none());
    assert_eq!(disk(&h, PATH), "xa");
}

// verifies: AC-FR-216-1
#[test]
fn ワークスペース層の値で替わる() {
    let (mut h, _) = start_project_ws(
        "",
        "",
        Some("[editor]\ntrim_trailing_whitespace_on_save = true\n"),
    );
    type_at_start(&mut h, "x");
    save(&mut h);
    assert_eq!(disk(&h, REPO_A), "xa");
}

// ---------- レビューで足した試験 (段 3) ----------

// verifies: AC-FR-216-2
#[test]
fn 上書き保存が書込権限なしで失敗したら_読み取り専用にする前に変換を戻し_利用者の編集は残す() {
    let mut h = start(config(true, true), "a  ");
    type_at_start(&mut h, "x");
    h.fs.fail_next(FsOp::WriteAtomic, FsErrorKind::PermissionDenied);
    save(&mut h);
    assert_eq!(h.doc_text(), "xa  ", "変換だけを戻し、打った文字は残る");
    let d = h.editor.state().document(h.doc_id()).expect("文書");
    assert!(!d.is_editable(), "読み取り専用 (書込権限なし) になる");
    assert!(!d.history().can_redo(), "Redo に残さない");
    let (_, body, _) = dialog(&h).expect("保存の失敗の確認");
    assert!(body.ends_with(REVERTED), "{body}");
    // 全保存も同じ
    let mut h = start(config(true, true), "a  ");
    type_at_start(&mut h, "x");
    h.fs.fail_next(FsOp::WriteAtomic, FsErrorKind::PermissionDenied);
    h.cmd("file.save_all");
    h.run_jobs();
    assert_eq!(h.doc_text(), "xa  ");
    let (_, _, details) = dialog(&h).expect("失敗の一覧");
    assert!(details[0].ends_with(REVERTED), "{details:?}");
    // 保存の間に編集して変換のノードが今の位置でなければ、戻さず、Undo の案内もしない
    let mut h = start(config(true, true), "a  ");
    type_at_start(&mut h, "x");
    h.fs.fail_next(FsOp::WriteAtomic, FsErrorKind::PermissionDenied);
    h.cmd("file.save_all");
    h.advance_ms(5_000);
    h.keys("ctrl+end");
    h.text("y");
    h.run_jobs();
    assert_eq!(h.doc_text(), "xa\ny");
    let (_, _, details) = dialog(&h).expect("失敗の一覧");
    assert!(details[0].ends_with(KEPT), "{details:?}");
    assert!(!details[0].contains("Ctrl+Z"), "{details:?}");
}

// verifies: AC-FR-216-2
#[test]
fn undo_の途中で_redo_があるときは変換せず_redo_を捨てない() {
    let mut h = start(config(true, true), "a  ");
    type_at_start(&mut h, "x");
    type_at_start(&mut h, "y");
    undo(&mut h);
    assert_eq!(h.doc_text(), "xa  ");
    save(&mut h);
    assert_eq!(disk(&h, PATH), "xa  ");
    h.cmd("edit.redo");
    assert_eq!(h.doc_text(), "yxa  ", "Redo が残っている");
}

// verifies: AC-FR-216-1
#[test]
fn editorconfig_の無い文書は読み直しの間も変換する() {
    let user = "[editor]\ntrim_trailing_whitespace_on_save = true\n";
    let (mut h, _) = start_project_with(user, None, None, |_| false);
    h.cmd("config.reload");
    // ReadConfig → ReadWorkspaceConfig を当て、`.editorconfig` の読込を積むところまで進める
    assert!(h.run_one_job());
    assert!(h.run_one_job());
    type_at_start(&mut h, "x");
    h.cmd("file.save");
    h.run_jobs();
    assert_eq!(disk(&h, REPO_A), "xa");
}

// verifies: AC-FR-216-2
#[test]
fn つながりの確認で_このまま保存しても二度当てない() {
    let pad = "0123456789".repeat(20) + "\n";
    let bytes = [b"a  \n".as_slice(), b"x\x81\n@yz\n", pad.as_bytes()].concat();
    let mut h = open_as(config(true, false), &bytes, Encoding::ShiftJis);
    h.cmd("file.toggle_read_only");
    h.text("a");
    // 不正バイトの後ろの改行を消して、`@` とつなげる
    let at = h.doc_text().find("\u{10FF81}").expect("エスケープ文字") + '\u{10FF81}'.len_utf8();
    let v = h
        .editor
        .state()
        .first_window()
        .and_then(|w| h.editor.state().window_view(w))
        .expect("View");
    h.editor
        .state_mut()
        .set_selection(v, Selection::point(ByteOff(at)));
    h.keys("delete");
    h.clear_log();
    h.cmd("file.save");
    h.run_jobs();
    let (title, _, _) = dialog(&h).expect("つながりの確認");
    assert!(title.contains("開き直す"), "{title}");
    h.text("s");
    h.run_jobs();
    assert_eq!(transforms(&h), 1, "もう一度は変換しない");
    assert_eq!(
        h.fs.read(PATH).expect("ある"),
        [b"a\n".as_slice(), b"x\x81@yz\n", pad.as_bytes()].concat()
    );
    let m = message(&mut h);
    assert!(m.ends_with("(行末の空白を 1 行で除きました)"), "{m}");
}

// verifies: AC-FR-216-2
#[test]
fn 直接上書き_ハードリンク_バックアップなし_置換の後の投入し直しでも二度当てない() {
    // 原子的置換ができない → 直接上書き。完了のメッセージは `info.save.in_place` が最後で、付記はそこに 1 つだけ
    let mut h = start(config(true, true), "a  ");
    type_at_start(&mut h, "x");
    h.clear_log();
    h.fs.fail_next(FsOp::WriteAtomic, FsErrorKind::AtomicUnsupported);
    save(&mut h);
    h.text("o");
    h.run_jobs();
    assert_eq!(disk(&h, PATH), "xa\n");
    assert_eq!(transforms(&h), 1);
    assert_eq!(
        message(&mut h),
        "/w/a.txt を直接上書きで保存しました (行末の空白を 1 行で除きました、末尾に改行を足しました)"
    );
    assert_eq!(
        h.messages().last().map(String::as_str),
        Some("info.save.in_place")
    );
    // 直接上書きのバックアップを作れない → バックアップなしで上書き
    let mut h = start(config(true, true), "a  ");
    type_at_start(&mut h, "x");
    h.clear_log();
    h.fs.fail_next(FsOp::WriteAtomic, FsErrorKind::AtomicUnsupported);
    h.fs.fail_next(FsOp::WriteInPlace, FsErrorKind::BackupFailed);
    save(&mut h);
    h.text("o");
    h.run_jobs();
    let (title, _, _) = dialog(&h).expect("バックアップなしの確認");
    assert!(title.contains("バックアップ"), "{title}");
    h.text("o");
    h.run_jobs();
    assert_eq!(disk(&h, PATH), "xa\n");
    assert_eq!(transforms(&h), 1);
    // ハードリンク → 切り離して保存
    let mut h = start(config(true, true), "a  ");
    h.fs.set_nlink(PATH, 2);
    type_at_start(&mut h, "x");
    h.clear_log();
    save(&mut h);
    assert!(dialog(&h).is_some(), "ハードリンクの確認");
    h.text("b");
    h.run_jobs();
    assert_eq!(disk(&h, PATH), "xa\n");
    assert_eq!(transforms(&h), 1);
    // 表現不能 → 置換して保存 (置換の Transaction の後の投入し直し)
    let mut h = open_as(config(true, false), &sjis("a  \n"), Encoding::ShiftJis);
    h.keys("ctrl+end");
    h.text("\u{1F600}");
    h.clear_log();
    h.cmd("file.save");
    h.run_jobs();
    h.text("r");
    h.run_jobs();
    let (_, _, _) = dialog(&h).expect("置換の一覧の確認");
    h.text("r");
    h.run_jobs();
    assert_eq!(transforms(&h), 1, "置換の後の投入し直しで変換しない");
    let saved = h.fs.read(PATH).expect("ある");
    assert!(saved.starts_with(b"a\n"), "{saved:?}");
}

// verifies: AC-FR-216-2
#[test]
fn 保存のジョブを待つ間は印を変えず_再試行で後から打った空白は除かない() {
    let mut h = start(config(true, false), "a  ");
    type_at_start(&mut h, "x");
    h.fs.fail_next(FsOp::WriteAtomic, FsErrorKind::Busy);
    h.cmd("file.save");
    assert_eq!(h.doc_text(), "xa");
    // 保存のジョブを待つ間に行末に空白を打ち、もう一度保存する (深さ 1 の待ち行列に積むが、前の保存が失敗したので
    // 捨てる。続きは確認の選択で決める、03 §9.3 手順 4)
    h.advance_ms(5_000);
    h.keys("ctrl+end");
    h.text("  ");
    h.cmd("file.save");
    h.run_jobs();
    // Busy の再試行: 同じ連鎖なので、新しい空白には変換を当てない
    h.text("t");
    h.run_jobs();
    assert_eq!(disk(&h, PATH), "xa  ");
}

// verifies: AC-FR-216-2
#[test]
fn 保存の間の編集で出し直した未保存の確認の保存は新しい連鎖として変換する() {
    let mut h = start(config(true, false), "a  ");
    type_at_start(&mut h, "x");
    h.cmd("app.quit");
    h.text("s");
    assert_eq!(h.doc_text(), "xa");
    // 保存のジョブを待つ間に行末に空白を打つ
    h.advance_ms(5_000);
    h.keys("ctrl+end");
    h.text("  ");
    h.run_jobs();
    assert_eq!(disk(&h, PATH), "xa");
    assert!(
        h.quit_code().is_none(),
        "保存の間の編集で dirty なので終了しない"
    );
    assert_eq!(h.overlay(), "dialog", "未保存の確認を出し直す");
    h.text("s");
    h.run_jobs();
    assert_eq!(
        disk(&h, PATH),
        "xa",
        "新しい連鎖として後から打った空白も除く"
    );
    assert!(h.quit_code().is_some());
}

// verifies: AC-FR-216-1
#[test]
fn 起動時の言語定義の読込を待つ間は変換しない() {
    let user = "[editor]\ntrim_trailing_whitespace_on_save = true\n";
    let (mut h, held) = start_project_with(user, None, None, |r| {
        matches!(
            r,
            JobResult::LanguagesRead { .. } | JobResult::LanguagesPrecompiled { .. }
        )
    });
    assert!(!held.is_empty(), "起動時の言語定義の読込を待たせている");
    type_at_start(&mut h, "x");
    save(&mut h);
    assert_eq!(disk(&h, REPO_A), "xa  ", "言語定義の読込を待っている");
    // 届いた後はかかる
    h.deliver(held);
    h.run_jobs();
    h.text("y");
    save(&mut h);
    assert_eq!(disk(&h, REPO_A), "xya");
}
