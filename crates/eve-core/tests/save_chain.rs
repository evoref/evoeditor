//! 保存の連鎖 (docs/03 §9.7) の 3 つの規則: 保存中の `Esc` で後続を取り消す (`cancel_origin = User`、#646)、中止で
//! `info.save.aborted` を `{path}` 付きで出す (#647)、保存中の `file.save` の深さ 1 の待ち行列 (03 §9.3 手順 4、#648)。
//! 保存ジョブは `ManualJobSpawner` が `run_jobs` まで止めておくので、「保存を待つ間」を再現できる。取消しが間に合わずに
//! 保存できた場合は、ジョブを先に走らせて結果を取り置き (`run_one_job_held`)、取消しの後に渡して再現する。

use eve_config::Config;
use eve_core::{Args, FsErrorKind, FsPath, InitialFile};
use eve_testkit::{FsOp, Harness};
use eve_text::Selection;
use eve_view::Effect;

const PATH: &str = "/w/a.txt";

/// `info.save.aborted` の a.txt の文言。
const ABORTED: &str = "保存を中止しました (/w/a.txt は書き換えていません)";

/// a.txt (`abc`) を開いて先頭に `x` を打った Harness。
fn edited() -> Harness {
    let mut h = Harness::new().with_file(PATH, b"abc");
    h.open(PATH);
    h.run_jobs();
    h.text("x");
    h
}

/// 今のメッセージの本文。
fn message(h: &mut Harness) -> String {
    let f = h.frame();
    f.messages
        .current
        .map(|m| m.text.spans.iter().map(|s| s.text.as_str()).collect())
        .unwrap_or_default()
}

fn disk(h: &Harness, path: &str) -> String {
    String::from_utf8(h.fs.read(path).expect("ファイルがある")).expect("UTF-8")
}

fn quitted(h: &Harness) -> bool {
    h.effects().iter().any(|e| matches!(e, Effect::Quit { .. }))
}

fn selection(h: &Harness) -> Selection {
    let view = h
        .editor
        .state()
        .window_view(h.window())
        .expect("View がある");
    h.editor
        .state()
        .view(view)
        .expect("View がある")
        .selection
        .clone()
}

/// 終了の未保存の確認で「保存」を選び、保存ジョブを待っている状態 (後続 = 終了)。
fn quit_saving() -> Harness {
    let mut h = edited();
    h.cmd("app.quit");
    assert_eq!(h.overlay(), "dialog", "未保存の確認");
    h.keys("s");
    assert_eq!(h.pending_jobs(), 1, "保存ジョブが実行待ち");
    h
}

// ---------- #646: 保存中の `Esc` で後続を取り消す ----------

// spec: 03 §9.7 (全状態の遷移表の `Saving` (`cancel_origin = None`、`cont` が Some) と `cancel_origin = User` の行)、04 §7.5
// verifies: FR-107
#[test]
fn 後続のある保存の_esc_は取消しを送り_間に合えば中止して後続を行わない() {
    let mut h = quit_saving();
    h.clear_log();
    h.keys("esc");
    assert_eq!(h.pending_jobs(), 0, "保存ジョブに取消しを送る");
    h.run_jobs();
    assert!(!quitted(&h), "取り消した終了は行わない");
    assert_eq!(disk(&h, PATH), "abc", "何も書いていない");
    assert!(h.is_dirty());
    assert_eq!(message(&mut h), ABORTED);
    // 保存中の終了の待ちも止めている (確認を後から出さない)
    h.advance_ms(10_000);
    assert_eq!(h.overlay(), "");
    assert!(!quitted(&h));
}

// spec: 03 §9.7 (`cancel_origin = User` の行の `Saved`)、02 §9.3 の採否表
// verifies: FR-107
#[test]
fn 取消しが間に合わずに保存できたら採用し_後続は行わない() {
    let mut h = quit_saving();
    // ジョブは置換を始めた後で、取消しを見ずに完了まで進んだ
    let held = h.run_one_job_held();
    h.clear_log();
    h.keys("esc");
    h.deliver(held);
    assert_eq!(disk(&h, PATH), "xabc", "保存できたことを正とする");
    assert!(!h.is_dirty());
    assert!(!quitted(&h), "後続 (終了) は捨てる");
    assert_eq!(
        h.messages().last().map(String::as_str),
        Some("info.save.done")
    );
    assert_eq!(h.overlay(), "");
}

// spec: 03 §9.7 (`Saving` (`cancel_origin = None`、`cont` が None) の行の `ui.cancel`)、04 §7.5
#[test]
fn 後続の無い保存の_esc_は取り消さずに次の段へ通す() {
    let mut h = edited();
    h.keys("shift+left");
    assert!(!selection(&h).primary().is_empty(), "選択がある");
    h.cmd("file.save");
    h.keys("esc");
    assert_eq!(h.pending_jobs(), 1, "単独の保存は取り消さない");
    assert!(
        selection(&h).primary().is_empty(),
        "次の段 (選択の解除) へ通す"
    );
    h.run_jobs();
    assert_eq!(disk(&h, PATH), "xabc");
    assert!(!h.is_dirty());
}

// spec: 03 §9.7 (`cancel_origin = User` の行の `ui.cancel` と `app.quit`)
// verifies: FR-107
#[test]
fn 取り消した後の_esc_は素通りし_app_quit_は後続を終了にし直す() {
    // 取消しが間に合った: 中止になり、終了しない (終了するには `app.quit` をもう一度)
    let mut h = quit_saving();
    h.keys("esc");
    h.keys("shift+left");
    h.keys("esc");
    assert!(
        selection(&h).primary().is_empty(),
        "取消しは送り済みなので、次の段へ通す"
    );
    h.cmd("app.quit");
    h.run_jobs();
    assert!(!quitted(&h), "取消しが間に合ったので終了しない");
    assert!(h.is_dirty());
    assert_eq!(message(&mut h), ABORTED);
    // 取消しが間に合わずに保存できた: 終了にし直した後続を行う
    let mut h = quit_saving();
    let held = h.run_one_job_held();
    h.keys("esc");
    h.cmd("app.quit");
    h.deliver(held);
    assert_eq!(disk(&h, PATH), "xabc");
    assert!(quitted(&h), "保存できたので終了する");
}

// spec: 03 §9.7 (`cancel_origin = User` の行の「その他の `Failed`」)
#[test]
fn 取り消した保存のほかの失敗は単独の保存として選ばせる() {
    let mut h = quit_saving();
    h.fs.fail_next(FsOp::WriteAtomic, FsErrorKind::Busy);
    let held = h.run_one_job_held();
    h.keys("esc");
    h.deliver(held);
    assert_eq!(
        h.overlay(),
        "dialog",
        "保存の失敗の確認 (単独の保存の (e) の表)"
    );
    h.keys("t");
    h.run_jobs();
    assert_eq!(disk(&h, PATH), "xabc", "再試行で保存できた");
    assert!(!quitted(&h), "後続は捨てたので終了しない");
}

// spec: 03 §9.7 (`Saving` の行の `ui.cancel`。後続 = 閉じる、03 §9.8 の `Continuation::Close`)
// verifies: FR-107
#[test]
fn 閉じる前の保存も_esc_で取り消し_文書を閉じない() {
    let mut h = edited();
    let doc = h.doc_id();
    h.cmd("buffer.close");
    assert_eq!(h.overlay(), "dialog");
    h.keys("s");
    h.keys("esc");
    h.run_jobs();
    assert!(h.editor.state().document(doc).is_some(), "閉じない");
    assert_eq!(h.doc_text(), "xabc");
    assert_eq!(message(&mut h), ABORTED);
}

// spec: 03 §9.3.1 の SVT-8 (後続の取消し)、03 §9.7 の「中止」
// verifies: AC-FR-216-2
#[test]
fn 保存時の変換を当てた後に後続を取り消したら_中止のメッセージに付記する() {
    let mut h = Harness::builder()
        .defer_start()
        .size(400, 20)
        .config(Config {
            trim_trailing_whitespace_on_save: true,
            ..Config::default()
        })
        .dir("/w")
        .file(PATH, b"a  ")
        .build();
    h.editor
        .open_initial_files(&[InitialFile::path(PATH)], false, None);
    h.started();
    h.run_jobs();
    h.text("x");
    h.cmd("app.quit");
    h.keys("s");
    assert_eq!(h.doc_text(), "xa", "投入の直前に変換を当てた");
    h.keys("esc");
    h.run_jobs();
    assert!(!quitted(&h));
    assert_eq!(
        message(&mut h),
        format!(
            "{ABORTED} 保存時の変換 (行末の空白・末尾の改行) は本文に残っています。Ctrl+Z で戻せます"
        )
    );
}

// ---------- #647: 中止で `info.save.aborted` (パス付き) を出す ----------

/// 保存して確認を出し、`Esc` でキャンセルしたときのメッセージ。
fn cancel_confirm(h: &mut Harness) -> String {
    h.cmd("file.save");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog", "確認が出る");
    h.keys("esc");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert!(h.is_dirty(), "保存していない");
    message(h)
}

// spec: 03 §9.7 の「中止」の行と (e) の表の「中止」「キャンセル」、04 §15.2 の `info.save.aborted`
#[test]
fn 保存の確認のキャンセルはどれも中止を示し_パスを埋める() {
    // 外部変更
    let mut h = edited();
    h.fs.set_file(&FsPath::new(PATH), b"other");
    assert_eq!(cancel_confirm(&mut h), ABORTED, "外部変更");
    // 原子的置換ができない (直接上書きの確認)
    let mut h = edited();
    h.fs.fail_next(FsOp::WriteAtomic, FsErrorKind::AtomicUnsupported);
    assert_eq!(cancel_confirm(&mut h), ABORTED, "原子的置換不可");
    // ハードリンク
    let mut h = edited();
    h.fs.set_nlink(PATH, 2);
    assert_eq!(cancel_confirm(&mut h), ABORTED, "ハードリンク");
    // バックアップを作れない (直接上書きを選んだ後)
    let mut h = edited();
    h.fs.fail_next(FsOp::WriteAtomic, FsErrorKind::AtomicUnsupported);
    h.fs.fail_next(FsOp::WriteInPlace, FsErrorKind::BackupFailed);
    h.cmd("file.save");
    h.run_jobs();
    h.text("o");
    assert_eq!(
        cancel_confirm_after_choice(&mut h),
        ABORTED,
        "バックアップなし"
    );
    // 保存の失敗 (Busy)
    let mut h = edited();
    h.fs.fail_next(FsOp::WriteAtomic, FsErrorKind::Busy);
    assert_eq!(cancel_confirm(&mut h), ABORTED, "保存の失敗");
}

/// 選択肢を選んだ後に出た確認を `Esc` でキャンセルしたときのメッセージ。
fn cancel_confirm_after_choice(h: &mut Harness) -> String {
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog", "確認が出る");
    h.keys("esc");
    assert_eq!(h.overlay(), "");
    message(h)
}

// spec: 03 §9.7 の「中止」、(e) の表の `DanglingLink` (v0.2 の「キャンセル」)
// verifies: FR-106
#[test]
fn ダングリングのリンクの確認のキャンセルも中止を示す() {
    let mut h = Harness::new();
    h.fs.add_link("/w/link.txt", "/w/real.txt");
    h.open("/w/link.txt");
    h.run_jobs();
    h.text("x");
    h.cmd("file.save");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog");
    h.keys("esc");
    assert_eq!(
        message(&mut h),
        "保存を中止しました (/w/link.txt は書き換えていません)"
    );
}

// spec: 03 §9.7 の「中止」、§9.3 手順 1 (読み取り専用を解除して保存の確認)
// verifies: FR-116
#[test]
fn 読み取り専用を解除して保存の確認のキャンセルも中止を示す() {
    let mut h = Harness::new().with_file(PATH, b"abc");
    h.fs.set_readonly(PATH, true);
    h.open(PATH);
    h.run_jobs();
    h.cmd("file.toggle_read_only");
    h.text("x");
    h.cmd("file.save");
    assert_eq!(h.overlay(), "dialog");
    h.keys("esc");
    assert_eq!(message(&mut h), ABORTED);
}

// spec: 03 §9.7 の状態表の `AskName` の行 (キャンセル → 中止)、04 §15.2 の `info.save.aborted`
#[test]
fn 名前の入力のキャンセルは中止を示し_無題の文書は名前で埋める() {
    // 無題の文書の保存の名前の入力
    let mut h = Harness::new();
    h.text("x");
    h.cmd("file.save");
    assert_eq!(h.overlay(), "prompt");
    h.keys("esc");
    assert_eq!(
        message(&mut h),
        "保存を中止しました (無題-1 は書き換えていません)"
    );
    // 名前を付けて保存の名前の入力
    let mut h = edited();
    h.cmd("file.save_as");
    assert_eq!(h.overlay(), "prompt");
    h.keys("esc");
    assert_eq!(message(&mut h), ABORTED);
}

// spec: 03 §9.7 の「中止」の行の例外 (`AskUnsaved` のキャンセルでは `info.save.aborted` を出さない。保存を始めていない)
// verifies: FR-107
#[test]
fn 未保存の確認のキャンセルは中止を示さない() {
    // 閉じる前の未保存の確認
    let mut h = edited();
    h.cmd("buffer.close");
    assert_eq!(h.overlay(), "dialog");
    h.clear_log();
    h.keys("esc");
    assert!(h.messages().is_empty(), "{:?}", h.messages());
    assert_eq!(h.doc_text(), "xabc", "閉じない");
    // 終了の未保存の確認
    let mut h = edited();
    h.cmd("app.quit");
    assert_eq!(h.overlay(), "dialog");
    h.clear_log();
    h.keys("esc");
    assert!(h.messages().is_empty(), "{:?}", h.messages());
    assert!(!quitted(&h));
    // 開き直しの前の未保存の確認
    let mut h = edited();
    h.cmd_with(
        "file.reopen_with_encoding",
        Args::new().with_str("encoding", "shift_jis"),
    );
    assert_eq!(h.overlay(), "dialog");
    h.clear_log();
    h.keys("esc");
    assert!(h.messages().is_empty(), "{:?}", h.messages());
    assert_eq!(h.doc_text(), "xabc");
}

// spec: 03 §9.7 の状態表の `ConfirmOverwrite` の行 (`Esc` は `AskName` に戻る。中止ではない)
#[test]
fn 上書きの確認の_esc_は名前の入力に戻り_中止を示さない() {
    let mut h = edited();
    h.fs.add_file("/w/b.txt", b"other");
    h.cmd_with("file.save_as", Args::new().with_str("path", "/w/b.txt"));
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog", "上書きの確認");
    h.clear_log();
    h.keys("esc");
    assert_eq!(h.overlay(), "prompt", "名前の入力に戻る");
    assert!(
        !h.messages().iter().any(|m| m == "info.save.aborted"),
        "{:?}",
        h.messages()
    );
}

// ---------- #648: 保存中の `file.save` の深さ 1 の待ち行列 ----------

// spec: 03 §9.3 手順 4 (深さ 1、再実行の `Expect::Stamp` は直前の `report.new_stamp`)、§9.7 の `Saving` (`cont` が None) の
// `file.save` の列
// verifies: FR-104
#[test]
fn 単独の保存中の_file_save_は積み_保存の後に最新の本文で再実行する() {
    let mut h = edited();
    h.cmd("file.save");
    h.text("y");
    h.clear_log();
    h.cmd("file.save");
    assert!(
        !h.messages().iter().any(|m| m == "info.save.in_progress"),
        "積んだ要求は知らせない: {:?}",
        h.messages()
    );
    assert_eq!(h.pending_jobs(), 1, "前の保存の完了を待つ");
    h.run_jobs();
    assert_eq!(disk(&h, PATH), "xyabc", "最新の本文で再実行した");
    assert!(!h.is_dirty());
    assert_eq!(h.overlay(), "", "自分の保存を外部変更と誤らない");
    let done = h
        .messages()
        .iter()
        .filter(|m| *m == "info.save.done")
        .count();
    assert_eq!(done, 2);
}

// spec: 03 §9.3 手順 4 (深さ 1)
#[test]
fn 保存の間に編集が無ければ積んだ_file_save_は書かない() {
    let mut h = edited();
    h.cmd("file.save");
    h.cmd("file.save");
    h.cmd("file.save");
    h.clear_log();
    h.run_jobs();
    assert_eq!(disk(&h, PATH), "xabc");
    assert_eq!(h.messages(), ["info.save.done"], "1 回だけ保存した");
}

// spec: 03 §9.3 手順 4 (前の保存が失敗したら積んだ要求は破棄し、`AfterFailure` の選択に任せる)
#[test]
fn 前の保存が失敗したら積んだ要求を捨て_確認の選択に任せる() {
    let mut h = edited();
    h.fs.fail_next(FsOp::WriteAtomic, FsErrorKind::Busy);
    h.cmd("file.save");
    h.text("y");
    h.cmd("file.save");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog", "保存の失敗の確認");
    assert_eq!(disk(&h, PATH), "abc");
    h.keys("esc");
    h.run_jobs();
    assert_eq!(h.pending_jobs(), 0, "積んだ要求は再実行しない");
    assert_eq!(disk(&h, PATH), "abc");
    assert!(h.is_dirty());
    assert_eq!(message(&mut h), ABORTED);
    // 確認で再試行を選べば、その時点の最新の本文 (積んだ要求の内容も含む) で書く
    let mut h = edited();
    h.fs.fail_next(FsOp::WriteAtomic, FsErrorKind::Busy);
    h.cmd("file.save");
    h.text("y");
    h.cmd("file.save");
    h.run_jobs();
    h.keys("t");
    h.run_jobs();
    assert_eq!(disk(&h, PATH), "xyabc");
    assert!(!h.is_dirty());
}

// spec: 03 §9.7 の全状態の遷移表 (`Saving` の `cont` が Some / `cancel_origin = User` の行と `WritingRecovery` の行の
// `file.save` は無視)、04 §15.2 の `info.save.in_progress`
#[test]
fn 後続のある保存_取り消した保存_退避先への保存の間の_file_save_は無視して知らせる() {
    // 後続のある保存
    let mut h = quit_saving();
    h.clear_log();
    h.cmd("file.save");
    assert_eq!(h.messages(), ["info.save.in_progress"]);
    // 取り消した保存 (`cancel_origin = User`)
    let mut h = quit_saving();
    let held = h.run_one_job_held();
    h.keys("esc");
    h.text("y");
    h.clear_log();
    h.cmd("file.save");
    assert_eq!(h.messages(), ["info.save.in_progress"]);
    h.deliver(held);
    h.run_jobs();
    assert_eq!(disk(&h, PATH), "xabc", "積んでいないので再実行しない");
    assert!(h.is_dirty());
    // 退避先への保存
    let mut h = edited();
    h.fs.fail_next(FsOp::WriteAtomic, FsErrorKind::DiskFull);
    h.cmd("file.save");
    h.run_jobs();
    h.keys("r");
    assert_eq!(h.pending_jobs(), 1, "退避先への保存を待つ");
    h.clear_log();
    h.cmd("file.save");
    assert_eq!(h.messages(), ["info.save.in_progress"]);
    h.run_jobs();
    assert_eq!(disk(&h, PATH), "abc", "保存しない");
    assert_eq!(h.fs.recovery_written().len(), 1);
}

// spec: 03 §9.3 手順 4、§9.8 の安全の不変条件 4 (閉じた文書の保存の完了)
#[test]
fn 積んだ後に文書を閉じたら再実行しない() {
    let mut h = edited();
    h.cmd("file.save");
    h.text("y");
    h.cmd("file.save");
    h.cmd("buffer.close");
    assert_eq!(h.overlay(), "dialog", "保存の間の編集で dirty");
    h.keys("d");
    h.run_jobs();
    assert_eq!(disk(&h, PATH), "xabc", "閉じた文書は再実行しない");
}

// spec: 03 §9.7 (f) 1 (単独の保存の間の `app.quit` は `cont = Quit` の連鎖を作る)、§9.3 手順 4
#[test]
fn 積んだ後に終了を求めたら_積んだ要求は再実行せず完了の規則に従う() {
    let mut h = edited();
    h.cmd("file.save");
    h.text("y");
    h.cmd("file.save");
    h.cmd("app.quit");
    h.run_jobs();
    assert_eq!(disk(&h, PATH), "xabc");
    assert!(!quitted(&h), "保存の間の編集で dirty なので終了しない");
    assert_eq!(h.overlay(), "dialog", "未保存の確認を出し直す");
}

// spec: 03 §9.3 手順 4、§9.7 の表の注 (`file.save` は `file.save_as` / `file.set_encoding` を含む)
#[test]
fn 保存中の文字コードを指定して保存と名前を付けて保存も積んで再実行する() {
    let mut h = edited();
    h.cmd("file.save");
    h.clear_log();
    h.cmd_with(
        "file.set_encoding",
        Args::new().with_str("encoding", "utf-16le"),
    );
    assert!(h.messages().is_empty(), "{:?}", h.messages());
    h.run_jobs();
    assert_eq!(
        h.fs.read(PATH).expect("ある"),
        b"\xFF\xFEx\0a\0b\0c\0",
        "未編集でも新しい文字コードで書く"
    );
    let mut h = edited();
    h.cmd("file.save");
    h.cmd_with("file.save_as", Args::new().with_str("path", "/w/b.txt"));
    h.run_jobs();
    assert_eq!(disk(&h, PATH), "xabc");
    assert_eq!(disk(&h, "/w/b.txt"), "xabc");
    assert_eq!(h.doc_path().as_deref(), Some("/w/b.txt"));
}

// spec: 03 §9.3.1 の SVT-4 (c) (待ち行列で再実行する保存は新しい要求として変換を判定する)
// verifies: AC-FR-216-1
#[test]
fn 再実行する保存は新しい連鎖として保存時の変換を判定する() {
    let mut h = Harness::builder()
        .defer_start()
        .config(Config {
            trim_trailing_whitespace_on_save: true,
            ..Config::default()
        })
        .dir("/w")
        .file(PATH, b"a  ")
        .build();
    h.editor
        .open_initial_files(&[InitialFile::path(PATH)], false, None);
    h.started();
    h.run_jobs();
    h.text("x");
    h.cmd("file.save");
    assert_eq!(h.doc_text(), "xa");
    // 保存を待つ間に行末へ空白を打ち、もう一度保存する
    h.keys("ctrl+end");
    h.text("b  ");
    h.cmd("file.save");
    h.run_jobs();
    assert_eq!(
        disk(&h, PATH),
        "xab",
        "再実行の保存で後から打った空白も除く"
    );
    assert!(!h.is_dirty());
}

// ---------- レビューの指摘の再現と、試験の抜け ----------

/// `Shift_JIS` で開いた a.txt (`a\n`)。
fn open_sjis() -> Harness {
    let mut h = Harness::builder().file(PATH, b"a\n").build();
    h.editor.state_mut().open_path(
        PATH,
        eve_core::OpenOptions {
            forced: Some(eve_core::Encoding::ShiftJis),
            ..eve_core::OpenOptions::default()
        },
    );
    h.run_jobs();
    h
}

// spec: 03 §9.7 の全状態の遷移表 (`BuildingReplacement` の行の `file.save` は無視)
#[test]
fn 置換の一覧を作る間の保存要求は無視して知らせる() {
    let mut h = open_sjis();
    h.keys("ctrl+end");
    h.text("\u{1F600}");
    h.cmd("file.save");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog", "表現不能の確認 (E3)");
    h.text("r");
    assert_eq!(h.pending_jobs(), 1, "置換の一覧を作るジョブを待つ");
    h.clear_log();
    h.cmd("file.save");
    assert_eq!(h.pending_jobs(), 1, "保存を投入しない");
    assert_eq!(h.messages(), ["info.save.in_progress"]);
}

// spec: 03 §9.3 手順 1 (読み取り専用の文書の `file.save` は無効)、§9.3 手順 4 (再実行は新しい要求)
#[test]
fn 積んだ後に読み取り専用にした文書は再実行で書かない() {
    let mut h = edited();
    h.cmd("file.save");
    h.text("y");
    h.cmd("file.save");
    h.cmd("file.toggle_read_only");
    h.run_jobs();
    assert_eq!(
        disk(&h, PATH),
        "xabc",
        "読み取り専用にした後の本文は書かない"
    );
    assert!(h.is_dirty());
}

// spec: 03 §9.7 の全状態の遷移表 (`Saving` (`cont` が None) の `file.save` は積む。無題の文書の最初の保存も同じ)
#[test]
fn 無題の文書の最初の保存の間の_file_save_は積み_名前を尋ねない() {
    let mut h = Harness::new();
    h.run_jobs();
    h.text("x");
    h.cmd_with("file.save_as", Args::new().with_str("path", "/w/n.txt"));
    h.run_one_job();
    assert_eq!(h.pending_jobs(), 1, "保存ジョブを待つ");
    h.text("y");
    h.cmd("file.save");
    assert_eq!(h.overlay(), "", "名前を尋ねない");
    h.run_jobs();
    assert_eq!(disk(&h, "/w/n.txt"), "xy", "最新の本文で再実行した");
    assert!(!h.is_dirty());
}

/// a.txt と b.txt を開いて、両方を編集した Harness (b.txt を表示している)。
fn two_edited() -> Harness {
    let mut h = Harness::new()
        .with_file(PATH, b"abc")
        .with_file("/w/b.txt", b"def");
    h.open(PATH);
    h.run_jobs();
    h.text("x");
    h.open("/w/b.txt");
    h.run_jobs();
    h.text("y");
    h
}

// spec: 03 §9.3 手順 4 (後続のある保存では積んだ要求を捨てる)、§9.7 の「保存中の `Esc` の細目」
#[test]
fn 終了を付けた保存の積んだ要求は_ほかの文書の_esc_で終了を外されても再実行しない() {
    let mut h = two_edited();
    h.open(PATH);
    h.cmd("file.save");
    h.text("z");
    h.cmd("file.save");
    h.open("/w/b.txt");
    h.cmd("file.save");
    h.cmd("app.quit");
    h.keys("esc");
    h.run_jobs();
    assert_eq!(
        disk(&h, PATH),
        "xabc",
        "積んだ要求は終了を付けたときに捨てた"
    );
    assert!(!quitted(&h));
}

// spec: 03 §9.7 の「保存中の `Esc` の細目」(表示中の文書の保存を先に取り消し、後続が終了ならほかの保存からも終了を外す)
// verifies: FR-107
#[test]
fn 複数の文書の保存に終了が付いているときの_esc_は表示中の文書の保存を取り消し_終了をやめる() {
    let mut h = two_edited();
    h.open(PATH);
    h.cmd("file.save");
    h.open("/w/b.txt");
    h.cmd("file.save");
    h.cmd("app.quit");
    assert_eq!(h.pending_jobs(), 2, "両方の保存を待つ");
    h.keys("esc");
    assert_eq!(
        h.pending_jobs(),
        1,
        "表示中の b.txt の保存だけに取消しを送る"
    );
    h.run_jobs();
    assert!(!quitted(&h), "終了しない");
    assert_eq!(disk(&h, PATH), "xabc", "a.txt は単独の保存として続けた");
    assert_eq!(disk(&h, "/w/b.txt"), "def", "b.txt は取り消した");
    h.advance_ms(10_000);
    assert_eq!(h.overlay(), "", "保存中の終了の確認を後から出さない");
    assert!(!quitted(&h));
}

// spec: 03 §9.7 の「保存中の `Esc` の細目」(`AskUnsaved` の「保存」で後続を付け直すと `User` の印を外す)
// verifies: FR-107
#[test]
fn 取り消した保存に閉じる前の保存で後続を付け直すと_保存できれば閉じる() {
    let mut h = quit_saving();
    let doc = h.doc_id();
    let held = h.run_one_job_held();
    h.keys("esc");
    h.cmd("buffer.close");
    assert_eq!(h.overlay(), "dialog", "まだ dirty なので未保存の確認");
    h.keys("s");
    h.deliver(held);
    assert_eq!(disk(&h, PATH), "xabc");
    assert!(
        h.editor.state().document(doc).is_none(),
        "付け直した後続 (閉じる) を行う"
    );
    assert!(!quitted(&h), "取り消した終了は行わない");
}

// spec: 03 §9.7 の全状態の遷移表 (`cancel_origin = QuitAbort` の行の `ui.cancel` は無視。保存の連鎖としては無視し、次の段へ
// 通す)
#[test]
fn 保存を中止して終了の最中の_esc_は素通りする() {
    let mut h = edited();
    h.cmd("file.save");
    h.cmd("app.quit");
    h.advance_ms(5_000);
    assert_eq!(h.overlay(), "dialog", "保存中の終了の確認");
    h.keys("q");
    h.keys("shift+left");
    h.keys("esc");
    assert!(
        selection(&h).primary().is_empty(),
        "次の段 (選択の解除) へ通す"
    );
    h.run_jobs();
    assert!(quitted(&h), "終了は取り消せない");
}

// spec: 03 §9.7 の「深さ 1 の待ち行列の細目」(オーバーレイの間は再実行を保留する)、03 §12.7.6 の細目 (7)
#[test]
fn オーバーレイが出ている間は再実行を保留し_閉じた後に再実行する() {
    let mut h = edited();
    h.cmd("file.save");
    h.text("y");
    h.cmd("file.save");
    h.cmd("ui.palette_open");
    assert_eq!(h.overlay(), "palette");
    h.run_jobs();
    assert_eq!(disk(&h, PATH), "xabc");
    assert_eq!(h.overlay(), "palette", "利用者の開いたパレットはそのまま");
    assert_eq!(h.pending_jobs(), 0, "再実行は保留した");
    h.keys("esc");
    h.run_jobs();
    assert_eq!(disk(&h, PATH), "xyabc", "閉じた後に再実行した");
    assert!(!h.is_dirty());
}

// spec: 03 §9.7 の「深さ 1 の待ち行列の細目」(前の保存の警告が表示中なら、再実行の完了は背景の知らせにする)
#[test]
fn 前の保存の警告は再実行の完了で置き換えない() {
    let mut h = edited();
    h.fs.warn_next(vec![eve_core::SaveWarning::FsyncFailed]);
    h.cmd("file.save");
    h.text("y");
    h.cmd("file.save");
    h.run_jobs();
    assert_eq!(disk(&h, PATH), "xyabc");
    let f = h.frame();
    let current = f.messages.current.expect("メッセージがある");
    assert_ne!(
        current.level,
        eve_view::MessageLevel::Info,
        "警告が表示されたまま"
    );
}

// spec: 03 §9.3 手順 4 (再実行は新しい要求。積んだ後に変わった文書の状態で決める)、§9.7 (a) の 1、§8.8
#[test]
fn 積んだ後に読み取り専用にした文書の名前を付けて保存と文字コードの指定は_入口の規則で扱う() {
    // 同じファイルへの名前を付けて保存: 読み取り専用の文書は同じファイルに保存しない
    let mut h = edited();
    h.cmd("file.save");
    h.text("y");
    h.cmd_with("file.save_as", Args::new().with_str("path", PATH));
    h.cmd("file.toggle_read_only");
    h.clear_log();
    h.run_jobs();
    assert_eq!(disk(&h, PATH), "xabc");
    assert!(
        h.messages()
            .iter()
            .any(|m| m == "msg.save_as.same_file_read_only"),
        "{:?}",
        h.messages()
    );
    // 文字コードを指定して保存: 読み取り専用の文書は名前を尋ねる
    let mut h = edited();
    h.cmd("file.save");
    h.cmd_with(
        "file.set_encoding",
        Args::new().with_str("encoding", "utf-16le"),
    );
    h.cmd("file.toggle_read_only");
    h.run_jobs();
    assert_eq!(disk(&h, PATH), "xabc");
    assert_eq!(h.overlay(), "prompt", "名前を尋ねる");
}
