//! オーバーレイ (確認ダイアログ・入力プロンプト) のシナリオ (docs/04 §8、01 FR-107、03 §9.7)。

use eve_core::{Args, CommandResult};
use eve_testkit::Harness;
use eve_testkit::helpers::cell_text;
use eve_view::{FocusTarget, InputEvent, OverlayModel};

fn run(h: &mut Harness, id: &str, args: Args) -> CommandResult {
    let r = h.cmd_with(id, args);
    h.run_jobs();
    r
}

fn open(h: &mut Harness, path: &str) {
    run(h, "file.open", Args::new().with_str("path", path));
}

fn keys(h: &mut Harness, seq: &str) {
    h.keys(seq);
    h.run_jobs();
}

fn text(h: &mut Harness, s: &str) {
    h.text(s);
    h.run_jobs();
}

fn quit_requested(h: &Harness) -> bool {
    h.quit_code().is_some()
}

/// 確認を起こす操作 (dirty な文書を閉じる。v0.2 は `file.open` / `file.new` が確認を出さないので、`buffer.close` を使う)。
fn ask(s: &mut Harness) {
    run(s, "buffer.close", Args::new());
}

/// a.txt を開いて 1 文字入力した (dirty な) 状態。
fn dirty_env() -> Harness {
    let mut s = Harness::new();
    s.fs.add_file("/w/a.txt", b"aaa");
    s.fs.add_file("/w/b.txt", b"bbb");
    open(&mut s, "/w/a.txt");
    text(&mut s, "x");
    s
}

// verifies: AC-FR-107-3
#[test]
fn dirty_な文書を閉じると確認が出て_キャンセルで何も変わらない() {
    let mut s = dirty_env();
    ask(&mut s);
    assert_eq!(s.overlay(), "dialog");
    let f = s.frame();
    assert_eq!(f.focus, FocusTarget::Overlay(0));
    assert!(f.primary_cursor.is_none() && !f.panes[0].focused);
    let OverlayModel::Confirm {
        title,
        message,
        choices,
        default,
        focused,
        ..
    } = &f.overlays[0]
    else {
        panic!()
    };
    assert_eq!(cell_text(title), "未保存の変更");
    assert_eq!(
        cell_text(message),
        "/w/a.txt には保存されていない変更があります"
    );
    let labels: Vec<String> = choices.iter().map(|c| cell_text(&c.label)).collect();
    assert_eq!(
        labels,
        [
            "[S] 保存して閉じる",
            "[D] 破棄して閉じる",
            "[Esc] キャンセル"
        ]
    );
    assert_eq!(
        (choices[0].key, choices[1].key, choices[2].key),
        (Some('s'), Some('d'), None)
    );
    assert!(choices[1].destructive && !choices[0].destructive);
    assert_eq!((*default, *focused), (choices[0].id, choices[0].id));
    // Esc で閉じる: 文書も dirty も変わらず、読込も始まらない
    keys(&mut s, "esc");
    assert_eq!(s.overlay(), "");
    assert_eq!(s.doc_text(), "xaaa");
    assert_eq!(s.pending_jobs(), 0);
    assert_eq!(s.doc_path().as_deref(), Some("/w/a.txt"));
}

#[test]
fn 破棄を選ぶと_保存せずに閉じる() {
    let mut s = dirty_env();
    ask(&mut s);
    text(&mut s, "D"); // アクセラレータ (大小を区別しない)
    assert_eq!(s.overlay(), "");
    assert_eq!(s.doc_text(), "");
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        b"aaa",
        "破棄したので a.txt は変わらない"
    );
}

#[test]
fn 保存を選ぶと_保存してから閉じる() {
    let mut s = dirty_env();
    ask(&mut s);
    keys(&mut s, "enter"); // 既定のボタン = 保存
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"xaaa");
    assert_eq!(s.doc_text(), "");
    assert_eq!(s.overlay(), "");
}

#[test]
fn 保存が失敗したら_続きの操作はしない() {
    let mut s = dirty_env();
    ask(&mut s);
    s.fs.fail_next(
        eve_testkit::FsOp::WriteAtomic,
        eve_core::FsErrorKind::PermissionDenied,
    );
    text(&mut s, "s");
    assert_eq!(s.doc_text(), "xaaa", "今の文書がそのまま残る");
    assert_eq!(s.doc_path().as_deref(), Some("/w/a.txt"));
    assert_eq!(s.editor.state().documents_len(), 1);
}

#[test]
fn フォーカスの移動と決定() {
    let mut s = dirty_env();
    ask(&mut s);
    keys(&mut s, "tab"); // 破棄
    keys(&mut s, "tab"); // キャンセル
    let f = s.frame();
    let OverlayModel::Confirm {
        focused, choices, ..
    } = &f.overlays[0]
    else {
        panic!()
    };
    assert_eq!(*focused, choices[2].id);
    keys(&mut s, "tab"); // 折り返して保存
    keys(&mut s, "shift+tab"); // 戻ってキャンセル
    keys(&mut s, "enter");
    assert_eq!(s.overlay(), "");
    assert_eq!(s.doc_text(), "xaaa", "キャンセルを決定: 何も変わらない");
}

#[test]
fn ダイアログはモーダル() {
    let mut s = dirty_env();
    ask(&mut s);
    text(&mut s, "zzz"); // 複数書記素はすべて捨てる
    s.editor
        .handle_input(s.window(), InputEvent::Paste("d".into())); // 貼り付けも選択肢を実行しない
    assert_eq!(s.overlay(), "dialog");
    keys(&mut s, "q"); // 一致しない文字は捨てる
    assert_eq!(s.overlay(), "dialog");
    assert_eq!(s.doc_text(), "xaaa", "文書は編集されない");
    let r = s.cmd("edit.undo");
    assert_eq!(
        r,
        CommandResult::Rejected(eve_core::RejectReason::NotAvailable)
    );
    keys(&mut s, "ctrl+s"); // ダイアログは global を参照しない
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"aaa");
    assert_eq!(s.overlay(), "dialog");
}

#[test]
fn 無題を閉じるときは_名前の入力を経て保存してから閉じる() {
    let mut s = Harness::new();
    text(&mut s, "memo");
    ask(&mut s);
    assert_eq!(s.overlay(), "dialog");
    let f = s.frame();
    let OverlayModel::Confirm { message, .. } = &f.overlays[0] else {
        panic!()
    };
    assert_eq!(
        cell_text(message),
        "無題-1 には保存されていない変更があります"
    );
    text(&mut s, "s"); // 保存 → パスがないので名前を尋ねる
    assert_eq!(s.overlay(), "prompt");
    text(&mut s, "memo.txt");
    keys(&mut s, "enter");
    assert_eq!(s.fs.read("/w/memo.txt").unwrap(), b"memo");
    assert_eq!(s.doc_text(), "", "保存の後に閉じ、新しい無題の文書");
    assert_eq!(s.doc_path(), None);
    assert_eq!(s.overlay(), "");
}

#[test]
fn 終了は_dirty_なら確認し_clean_ならすぐ終わる() {
    let mut s = Harness::new();
    run(&mut s, "app.quit", Args::new());
    assert!(quit_requested(&s), "clean ならすぐ終了");

    let mut s = dirty_env();
    run(&mut s, "app.quit", Args::new());
    assert_eq!(s.overlay(), "dialog");
    assert!(!quit_requested(&s));
    let f = s.frame();
    let OverlayModel::Confirm { choices, .. } = &f.overlays[0] else {
        panic!()
    };
    assert_eq!(cell_text(&choices[0].label), "[S] 保存して終了");
    keys(&mut s, "esc");
    assert!(!quit_requested(&s), "キャンセル: 終了しない");

    keys(&mut s, "ctrl+q"); // 既定キー mod+q
    text(&mut s, "d");
    assert!(quit_requested(&s), "破棄して終了");

    let mut s = dirty_env();
    run(&mut s, "app.quit", Args::new());
    text(&mut s, "s");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"xaaa");
    assert!(quit_requested(&s), "保存して終了");

    // 保存に失敗したら終了しない
    let mut s = dirty_env();
    run(&mut s, "app.quit", Args::new());
    s.fs.fail_next(
        eve_testkit::FsOp::WriteAtomic,
        eve_core::FsErrorKind::DiskFull,
    );
    text(&mut s, "s");
    assert!(!quit_requested(&s));
}

#[test]
fn 開くプロンプト() {
    let mut s = Harness::new();
    s.fs.add_file("/w/a.txt", b"aaa");
    run(&mut s, "file.open", Args::new());
    assert_eq!(s.overlay(), "prompt");
    let f = s.frame();
    let OverlayModel::Prompt {
        title,
        input,
        error,
        ..
    } = &f.overlays[0]
    else {
        panic!()
    };
    assert_eq!(cell_text(title), "開く");
    assert_eq!((input.len, input.cursor, error.is_none()), (0, 0, true));
    // 空のまま確定: エラーを出して閉じない
    keys(&mut s, "enter");
    assert_eq!(s.overlay(), "prompt");
    let f = s.frame();
    let OverlayModel::Prompt { error, .. } = &f.overlays[0] else {
        panic!()
    };
    assert_eq!(cell_text(error.as_ref().unwrap()), "パスを入力してください");
    // 入力するとエラーは消える。編集: 途中への挿入、Backspace、カーソル移動
    text(&mut s, "/w/ab.txt");
    keys(&mut s, "left");
    keys(&mut s, "left");
    keys(&mut s, "left");
    keys(&mut s, "left");
    keys(&mut s, "backspace");
    let f = s.frame();
    let OverlayModel::Prompt { input, error, .. } = &f.overlays[0] else {
        panic!()
    };
    assert!(error.is_none());
    let typed: String = input.cells.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(typed, "/w/a.txt", "カーソルの前の文字 (b) が消える");
    assert_eq!(input.cursor, 4);
    keys(&mut s, "home");
    keys(&mut s, "end");
    // 貼り付けの改行は取り除く
    s.editor
        .handle_input(s.window(), InputEvent::Paste("x\ny".into()));
    let f = s.frame();
    let OverlayModel::Prompt { input, .. } = &f.overlays[0] else {
        panic!()
    };
    assert_eq!(
        input
            .cells
            .iter()
            .map(|c| c.text.as_str())
            .collect::<String>(),
        "/w/a.txtxy"
    );
    // 作り直して a.txt を開く
    keys(&mut s, "esc");
    run(&mut s, "file.open", Args::new());
    text(&mut s, "/w/a.txt");
    keys(&mut s, "enter");
    assert_eq!(s.overlay(), "");
    assert_eq!(s.doc_text(), "aaa");
}

#[test]
fn 行へ移動のプロンプト() {
    let mut s = Harness::new();
    text(&mut s, "l1\nl2\nl3\n");
    keys(&mut s, "ctrl+g"); // 既定キー mod+g
    assert_eq!(s.overlay(), "prompt");
    let f = s.frame();
    let OverlayModel::Prompt { title, hint, .. } = &f.overlays[0] else {
        panic!()
    };
    assert_eq!(cell_text(title), "行へ移動");
    assert_eq!(cell_text(hint.as_ref().unwrap()), "行、行:桁、$ (最終行)");
    text(&mut s, "abc");
    keys(&mut s, "enter");
    assert_eq!(s.overlay(), "prompt", "不正な入力: 閉じない");
    let f = s.frame();
    let OverlayModel::Prompt { error, .. } = &f.overlays[0] else {
        panic!()
    };
    assert_eq!(
        cell_text(error.as_ref().unwrap()),
        "行番号を入力してください (例: 12、12:5、$)"
    );
    for _ in 0..3 {
        keys(&mut s, "backspace");
    }
    text(&mut s, "2:2");
    keys(&mut s, "enter");
    assert_eq!(s.overlay(), "");
    let f = s.frame();
    let status: String = f
        .status
        .items
        .iter()
        .flat_map(|i| i.long.spans.iter())
        .map(|sp| sp.text.as_str())
        .collect();
    assert!(status.contains("行 2, 桁 2"), "{status}");
}

#[test]
fn 名前を付けて保存のプロンプトは今のパスが初期値() {
    let mut s = Harness::new();
    s.fs.add_file("/w/a.txt", b"aaa");
    open(&mut s, "/w/a.txt");
    run(&mut s, "file.save_as", Args::new());
    let f = s.frame();
    let OverlayModel::Prompt { title, input, .. } = &f.overlays[0] else {
        panic!()
    };
    assert_eq!(cell_text(title), "名前を付けて保存");
    assert_eq!(
        input
            .cells
            .iter()
            .map(|c| c.text.as_str())
            .collect::<String>(),
        "/w/a.txt"
    );
    assert_eq!(input.cursor, input.len, "カーソルは末尾");
    keys(&mut s, "esc");
    assert_eq!(s.overlay(), "");
}

#[test]
fn オーバーレイ中のヒントは決定と取消() {
    let mut s = dirty_env();
    ask(&mut s);
    let f = s.frame();
    assert_eq!(f.hints.context, eve_view::HintContext::Overlay);
    let words: Vec<String> = f.hints.items.iter().map(|h| cell_text(&h.text)).collect();
    assert_eq!(words, ["取消", "決定", "次へ"]);
    keys(&mut s, "esc");
    assert_eq!(s.frame().hints.context, eve_view::HintContext::Normal);
}

#[test]
fn 読み取り専用の文書でもプロンプトは入力できる() {
    let mut s = Harness::new();
    s.fs.add_file("/w/bad.txt", b"x\x9A");
    open(&mut s, "/w/bad.txt");
    text(&mut s, "r"); // 不正バイトの確認: 読み取り専用で開く
    s.run_jobs();
    run(&mut s, "file.open", Args::new());
    text(&mut s, "/w/q");
    let f = s.frame();
    let OverlayModel::Prompt { input, .. } = &f.overlays[0] else {
        panic!()
    };
    assert_eq!(input.len, 4);
}

#[test]
fn 保存の間に編集したら_続きの前にもう一度尋ねる() {
    let mut s = dirty_env();
    run(&mut s, "app.quit", Args::new());
    // 「保存」を選ぶ (ジョブはまだ実行しない) → 保存中に入力する
    s.editor
        .handle_input(s.window(), InputEvent::Text("s".into()));
    s.editor
        .handle_input(s.window(), InputEvent::Text("y".into()));
    s.run_jobs();
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        b"xaaa",
        "保存したのは選択時の内容"
    );
    assert!(
        !quit_requested(&s),
        "保存中に編集された: そのまま終了しない"
    );
    assert_eq!(s.overlay(), "dialog", "もう一度尋ねる");
    assert_eq!(s.doc_text(), "xyaaa");
}

#[test]
fn 明示の保存の最中に破棄で閉じても_保存は完了する() {
    let mut s = dirty_env();
    s.cmd("file.save"); // ジョブはまだ
    ask(&mut s); // 保存中は dirty のまま → 確認
    assert_eq!(s.overlay(), "dialog");
    s.editor
        .handle_input(s.window(), InputEvent::Text("d".into()));
    s.run_jobs();
    assert_eq!(s.doc_text(), "");
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        b"xaaa",
        "求められた保存は中断されない"
    );
}

#[test]
fn 保存中に確認で保存を選んでも_二重に保存しない() {
    let mut s = dirty_env();
    s.cmd("file.save");
    ask(&mut s);
    s.editor
        .handle_input(s.window(), InputEvent::Text("s".into()));
    assert_eq!(s.pending_jobs(), 1, "保存ジョブは 1 つだけ");
    s.run_jobs();
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"xaaa");
    assert_eq!(
        s.doc_text(),
        "",
        "その保存の完了後に続きの操作 (閉じて無題の文書)"
    );
}

#[test]
fn 全角のアクセラレータと_shift_付きのキー() {
    let mut s = dirty_env();
    ask(&mut s);
    text(&mut s, "Ｄ"); // 全角英字は半角に畳む
    assert_eq!(s.doc_text(), "");
    let mut s = dirty_env();
    ask(&mut s);
    keys(&mut s, "shift+d"); // 大文字は shift + 小文字で届く
    assert_eq!(s.doc_text(), "");
}

#[test]
fn オーバーレイを閉じる_esc_はメッセージを消さない() {
    let mut s = dirty_env();
    ask(&mut s);
    s.editor.state_mut().show_message(
        eve_view::MessageLevel::Error,
        "msg.save.failed",
        &[("path", "p"), ("reason", "r")],
    );
    keys(&mut s, "esc");
    assert_eq!(s.overlay(), "");
    assert!(
        s.frame().messages.current.is_some(),
        "1 回の Esc は 1 つのことだけ"
    );
}

#[test]
fn 名前を付けて保存が拒否されたら_プロンプトに戻る() {
    let mut s = Harness::new();
    s.fs.add_file("/w/bad.txt", b"x\x9A");
    open(&mut s, "/w/bad.txt"); // 不正バイト → 確認 → 読み取り専用
    text(&mut s, "r");
    s.run_jobs();
    run(&mut s, "file.save_as", Args::new());
    assert_eq!(s.overlay(), "prompt");
    keys(&mut s, "enter"); // 同じファイル → 拒否
    assert_eq!(s.overlay(), "prompt", "拒否されたら名前の入力に戻る");
    keys(&mut s, "esc");
    assert_eq!(s.fs.read("/w/bad.txt").unwrap(), b"x\x9A");
}
