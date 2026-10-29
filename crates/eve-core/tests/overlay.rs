//! オーバーレイ (確認ダイアログ・入力プロンプト) のシナリオ (docs/04 §8、01 FR-107、03 §9.7)。

use std::rc::Rc;
use std::sync::Arc;

use eve_core::{Args, CommandInvocation, CommandResult, Editor, InvocationSource, Ports};
use eve_testkit::{FixedPaths, ManualClock, ManualJobSpawner, MemFs};
use eve_view::{
    Capabilities, Effect, FocusTarget, Frame, InputEvent, KeyCode, KeyEvent, Modifiers,
    OverlayModel, WindowId,
};

struct Env {
    e: Editor,
    fs: Arc<MemFs>,
    jobs: Rc<ManualJobSpawner>,
    win: WindowId,
}

impl Env {
    fn new() -> Env {
        let clock = ManualClock::new();
        let fs = Arc::new(MemFs::new(Arc::new(clock.clone())));
        fs.add_dir("/w");
        let paths = Arc::new(FixedPaths::default());
        let jp = eve_core::JobPorts {
            fs: fs.clone(),
            clock: Arc::new(clock.clone()),
            paths: paths.clone(),
        };
        let jobs = ManualJobSpawner::new(jp);
        let ports = Ports {
            fs: fs.clone(),
            clock: Arc::new(clock),
            jobs: Rc::clone(&jobs) as Rc<dyn eve_core::JobSpawner>,
            paths,
        };
        let mut e = Editor::new(ports, Capabilities::default());
        e.open_initial(None, false);
        let win = e.state().first_window().unwrap();
        Env { e, fs, jobs, win }
    }

    fn pump(&mut self) {
        loop {
            self.jobs.run_jobs();
            let rs = self.jobs.take_results();
            if rs.is_empty() {
                break;
            }
            for r in rs {
                self.e.on_job_result(r);
            }
        }
    }

    fn run(&mut self, id: &str, args: Args) -> CommandResult {
        let r = self
            .e
            .execute(CommandInvocation::new(id, InvocationSource::Keymap).with_args(args));
        self.pump();
        r
    }

    fn open(&mut self, path: &str) {
        self.run("file.open", Args::new().with_str("path", path));
    }

    fn key(&mut self, code: KeyCode, mods: Modifiers) {
        self.e
            .handle_input(self.win, InputEvent::Key(KeyEvent::press(code, mods)));
        self.pump();
    }

    fn text(&mut self, s: &str) {
        self.e.handle_input(self.win, InputEvent::Text(s.into()));
        self.pump();
    }

    fn frame(&mut self) -> Frame {
        self.e.frame(self.win).unwrap()
    }

    fn doc_text(&self) -> String {
        let id = self.e.state().active_document().unwrap();
        self.e.state().document(id).unwrap().to_string_lossy()
    }

    fn doc_path(&self) -> Option<String> {
        let id = self.e.state().active_document().unwrap();
        self.e
            .state()
            .document(id)
            .unwrap()
            .path
            .as_ref()
            .map(|p| p.display())
    }

    fn overlay(&self) -> &'static str {
        self.e.state().overlay_kind()
    }

    fn quit_requested(&mut self) -> bool {
        self.e
            .drain_effects()
            .iter()
            .any(|e| matches!(e, Effect::Quit { .. }))
    }
}

fn ct(t: &eve_view::CellText) -> String {
    t.spans.iter().map(|s| s.text.as_str()).collect()
}

/// 確認を起こす操作 (dirty な文書を閉じる。v0.2 は `file.open` / `file.new` が確認を出さないので、`buffer.close` を使う)。
fn ask(s: &mut Env) {
    s.run("buffer.close", Args::new());
}

/// a.txt を開いて 1 文字入力した (dirty な) 状態。
fn dirty_env() -> Env {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"aaa");
    s.fs.add_file("/w/b.txt", b"bbb");
    s.open("/w/a.txt");
    s.text("x");
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
    assert_eq!(ct(title), "未保存の変更");
    assert_eq!(ct(message), "/w/a.txt には保存されていない変更があります");
    let labels: Vec<String> = choices.iter().map(|c| ct(&c.label)).collect();
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
    s.key(KeyCode::Esc, Modifiers::NONE);
    assert_eq!(s.overlay(), "");
    assert_eq!(s.doc_text(), "xaaa");
    assert_eq!(s.jobs.pending(), 0);
    assert_eq!(s.doc_path().as_deref(), Some("/w/a.txt"));
}

#[test]
fn 破棄を選ぶと_保存せずに閉じる() {
    let mut s = dirty_env();
    ask(&mut s);
    s.text("D"); // アクセラレータ (大小を区別しない)
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
    s.key(KeyCode::Enter, Modifiers::NONE); // 既定のボタン = 保存
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
    s.text("s");
    assert_eq!(s.doc_text(), "xaaa", "今の文書がそのまま残る");
    assert_eq!(s.doc_path().as_deref(), Some("/w/a.txt"));
    assert_eq!(s.e.state().documents_len(), 1);
}

#[test]
fn フォーカスの移動と決定() {
    let mut s = dirty_env();
    ask(&mut s);
    s.key(KeyCode::Tab, Modifiers::NONE); // 破棄
    s.key(KeyCode::Tab, Modifiers::NONE); // キャンセル
    let f = s.frame();
    let OverlayModel::Confirm {
        focused, choices, ..
    } = &f.overlays[0]
    else {
        panic!()
    };
    assert_eq!(*focused, choices[2].id);
    s.key(KeyCode::Tab, Modifiers::NONE); // 折り返して保存
    s.key(KeyCode::Tab, Modifiers::SHIFT); // 戻ってキャンセル
    s.key(KeyCode::Enter, Modifiers::NONE);
    assert_eq!(s.overlay(), "");
    assert_eq!(s.doc_text(), "xaaa", "キャンセルを決定: 何も変わらない");
}

#[test]
fn ダイアログはモーダル() {
    let mut s = dirty_env();
    ask(&mut s);
    s.text("zzz"); // 複数書記素はすべて捨てる
    s.e.handle_input(s.win, InputEvent::Paste("d".into())); // 貼り付けも選択肢を実行しない
    assert_eq!(s.overlay(), "dialog");
    s.key(KeyCode::Char('q'), Modifiers::NONE); // 一致しない文字は捨てる
    assert_eq!(s.overlay(), "dialog");
    assert_eq!(s.doc_text(), "xaaa", "文書は編集されない");
    let r = s.e.execute(CommandInvocation::new(
        "edit.undo",
        InvocationSource::Keymap,
    ));
    assert_eq!(
        r,
        CommandResult::Rejected(eve_core::RejectReason::NotAvailable)
    );
    s.key(KeyCode::Char('s'), Modifiers::CTRL); // ダイアログは global を参照しない
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"aaa");
    assert_eq!(s.overlay(), "dialog");
}

#[test]
fn 無題を閉じるときは_名前の入力を経て保存してから閉じる() {
    let mut s = Env::new();
    s.text("memo");
    ask(&mut s);
    assert_eq!(s.overlay(), "dialog");
    let f = s.frame();
    let OverlayModel::Confirm { message, .. } = &f.overlays[0] else {
        panic!()
    };
    assert_eq!(ct(message), "無題-1 には保存されていない変更があります");
    s.text("s"); // 保存 → パスがないので名前を尋ねる
    assert_eq!(s.overlay(), "prompt");
    s.text("memo.txt");
    s.key(KeyCode::Enter, Modifiers::NONE);
    assert_eq!(s.fs.read("/w/memo.txt").unwrap(), b"memo");
    assert_eq!(s.doc_text(), "", "保存の後に閉じ、新しい無題の文書");
    assert_eq!(s.doc_path(), None);
    assert_eq!(s.overlay(), "");
}

#[test]
fn 終了は_dirty_なら確認し_clean_ならすぐ終わる() {
    let mut s = Env::new();
    s.run("app.quit", Args::new());
    assert!(s.quit_requested(), "clean ならすぐ終了");

    let mut s = dirty_env();
    s.run("app.quit", Args::new());
    assert_eq!(s.overlay(), "dialog");
    assert!(!s.quit_requested());
    let f = s.frame();
    let OverlayModel::Confirm { choices, .. } = &f.overlays[0] else {
        panic!()
    };
    assert_eq!(ct(&choices[0].label), "[S] 保存して終了");
    s.key(KeyCode::Esc, Modifiers::NONE);
    assert!(!s.quit_requested(), "キャンセル: 終了しない");

    s.key(KeyCode::Char('q'), Modifiers::CTRL); // 既定キー mod+q
    s.text("d");
    assert!(s.quit_requested(), "破棄して終了");

    let mut s = dirty_env();
    s.run("app.quit", Args::new());
    s.text("s");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"xaaa");
    assert!(s.quit_requested(), "保存して終了");

    // 保存に失敗したら終了しない
    let mut s = dirty_env();
    s.run("app.quit", Args::new());
    s.fs.fail_next(
        eve_testkit::FsOp::WriteAtomic,
        eve_core::FsErrorKind::DiskFull,
    );
    s.text("s");
    assert!(!s.quit_requested());
}

#[test]
fn 開くプロンプト() {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"aaa");
    s.run("file.open", Args::new());
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
    assert_eq!(ct(title), "開く");
    assert_eq!((input.len, input.cursor, error.is_none()), (0, 0, true));
    // 空のまま確定: エラーを出して閉じない
    s.key(KeyCode::Enter, Modifiers::NONE);
    assert_eq!(s.overlay(), "prompt");
    let f = s.frame();
    let OverlayModel::Prompt { error, .. } = &f.overlays[0] else {
        panic!()
    };
    assert_eq!(ct(error.as_ref().unwrap()), "パスを入力してください");
    // 入力するとエラーは消える。編集: 途中への挿入、Backspace、カーソル移動
    s.text("/w/ab.txt");
    s.key(KeyCode::Left, Modifiers::NONE);
    s.key(KeyCode::Left, Modifiers::NONE);
    s.key(KeyCode::Left, Modifiers::NONE);
    s.key(KeyCode::Left, Modifiers::NONE);
    s.key(KeyCode::Backspace, Modifiers::NONE);
    let f = s.frame();
    let OverlayModel::Prompt { input, error, .. } = &f.overlays[0] else {
        panic!()
    };
    assert!(error.is_none());
    let typed: String = input.cells.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(typed, "/w/a.txt", "カーソルの前の文字 (b) が消える");
    assert_eq!(input.cursor, 4);
    s.key(KeyCode::Home, Modifiers::NONE);
    s.key(KeyCode::End, Modifiers::NONE);
    // 貼り付けの改行は取り除く
    s.e.handle_input(s.win, InputEvent::Paste("x\ny".into()));
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
    s.key(KeyCode::Esc, Modifiers::NONE);
    s.run("file.open", Args::new());
    s.text("/w/a.txt");
    s.key(KeyCode::Enter, Modifiers::NONE);
    assert_eq!(s.overlay(), "");
    assert_eq!(s.doc_text(), "aaa");
}

#[test]
fn 行へ移動のプロンプト() {
    let mut s = Env::new();
    s.text("l1\nl2\nl3\n");
    s.key(KeyCode::Char('g'), Modifiers::CTRL); // 既定キー mod+g
    assert_eq!(s.overlay(), "prompt");
    let f = s.frame();
    let OverlayModel::Prompt { title, hint, .. } = &f.overlays[0] else {
        panic!()
    };
    assert_eq!(ct(title), "行へ移動");
    assert_eq!(ct(hint.as_ref().unwrap()), "行、行:桁、$ (最終行)");
    s.text("abc");
    s.key(KeyCode::Enter, Modifiers::NONE);
    assert_eq!(s.overlay(), "prompt", "不正な入力: 閉じない");
    let f = s.frame();
    let OverlayModel::Prompt { error, .. } = &f.overlays[0] else {
        panic!()
    };
    assert_eq!(
        ct(error.as_ref().unwrap()),
        "行番号を入力してください (例: 12、12:5、$)"
    );
    for _ in 0..3 {
        s.key(KeyCode::Backspace, Modifiers::NONE);
    }
    s.text("2:2");
    s.key(KeyCode::Enter, Modifiers::NONE);
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
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"aaa");
    s.open("/w/a.txt");
    s.run("file.save_as", Args::new());
    let f = s.frame();
    let OverlayModel::Prompt { title, input, .. } = &f.overlays[0] else {
        panic!()
    };
    assert_eq!(ct(title), "名前を付けて保存");
    assert_eq!(
        input
            .cells
            .iter()
            .map(|c| c.text.as_str())
            .collect::<String>(),
        "/w/a.txt"
    );
    assert_eq!(input.cursor, input.len, "カーソルは末尾");
    s.key(KeyCode::Esc, Modifiers::NONE);
    assert_eq!(s.overlay(), "");
}

#[test]
fn オーバーレイ中のヒントは決定と取消() {
    let mut s = dirty_env();
    ask(&mut s);
    let f = s.frame();
    assert_eq!(f.hints.context, eve_view::HintContext::Overlay);
    let words: Vec<String> = f.hints.items.iter().map(|h| ct(&h.text)).collect();
    assert_eq!(words, ["取消", "決定", "次へ"]);
    s.key(KeyCode::Esc, Modifiers::NONE);
    assert_eq!(s.frame().hints.context, eve_view::HintContext::Normal);
}

#[test]
fn 読み取り専用の文書でもプロンプトは入力できる() {
    let mut s = Env::new();
    s.fs.add_file("/w/bad.txt", b"x\x9A");
    s.open("/w/bad.txt");
    s.text("r"); // 不正バイトの確認: 読み取り専用で開く
    s.pump();
    s.run("file.open", Args::new());
    s.text("/w/q");
    let f = s.frame();
    let OverlayModel::Prompt { input, .. } = &f.overlays[0] else {
        panic!()
    };
    assert_eq!(input.len, 4);
}

#[test]
fn 保存の間に編集したら_続きの前にもう一度尋ねる() {
    let mut s = dirty_env();
    s.run("app.quit", Args::new());
    // 「保存」を選ぶ (ジョブはまだ実行しない) → 保存中に入力する
    s.e.handle_input(s.win, InputEvent::Text("s".into()));
    s.e.handle_input(s.win, InputEvent::Text("y".into()));
    s.pump();
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        b"xaaa",
        "保存したのは選択時の内容"
    );
    assert!(
        !s.quit_requested(),
        "保存中に編集された: そのまま終了しない"
    );
    assert_eq!(s.overlay(), "dialog", "もう一度尋ねる");
    assert_eq!(s.doc_text(), "xyaaa");
}

#[test]
fn 明示の保存の最中に破棄で閉じても_保存は完了する() {
    let mut s = dirty_env();
    s.e.execute(CommandInvocation::new(
        "file.save",
        InvocationSource::Keymap,
    )); // ジョブはまだ
    ask(&mut s); // 保存中は dirty のまま → 確認
    assert_eq!(s.overlay(), "dialog");
    s.e.handle_input(s.win, InputEvent::Text("d".into()));
    s.pump();
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
    s.e.execute(CommandInvocation::new(
        "file.save",
        InvocationSource::Keymap,
    ));
    ask(&mut s);
    s.e.handle_input(s.win, InputEvent::Text("s".into()));
    assert_eq!(s.jobs.pending(), 1, "保存ジョブは 1 つだけ");
    s.pump();
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
    s.text("Ｄ"); // 全角英字は半角に畳む
    assert_eq!(s.doc_text(), "");
    let mut s = dirty_env();
    ask(&mut s);
    s.key(KeyCode::Char('d'), Modifiers::SHIFT); // 大文字は shift + 小文字で届く
    assert_eq!(s.doc_text(), "");
}

#[test]
fn オーバーレイを閉じる_esc_はメッセージを消さない() {
    let mut s = dirty_env();
    ask(&mut s);
    s.e.state_mut().show_message(
        eve_view::MessageLevel::Error,
        "msg.save.failed",
        &[("path", "p"), ("reason", "r")],
    );
    s.key(KeyCode::Esc, Modifiers::NONE);
    assert_eq!(s.overlay(), "");
    assert!(
        s.frame().messages.current.is_some(),
        "1 回の Esc は 1 つのことだけ"
    );
}

#[test]
fn 名前を付けて保存が拒否されたら_プロンプトに戻る() {
    let mut s = Env::new();
    s.fs.add_file("/w/bad.txt", b"x\x9A");
    s.open("/w/bad.txt"); // 不正バイト → 確認 → 読み取り専用
    s.text("r");
    s.pump();
    s.run("file.save_as", Args::new());
    assert_eq!(s.overlay(), "prompt");
    s.key(KeyCode::Enter, Modifiers::NONE); // 同じファイル → 拒否
    assert_eq!(s.overlay(), "prompt", "拒否されたら名前の入力に戻る");
    s.key(KeyCode::Esc, Modifiers::NONE);
    assert_eq!(s.fs.read("/w/bad.txt").unwrap(), b"x\x9A");
}
