//! クローム (ステータスバー・ヒントバー・メッセージ行) と文言の解決のシナリオ (docs/04 §8.7〜§8.9、07 §17)。

use eve_core::{Args, Document, TextFormat};
use eve_testkit::{AsciiOptions, Harness};
use eve_text::{ByteOff, Selection, Text};
use eve_view::{Frame, KeyCode, MessageLevel, Modifiers};

fn run(h: &mut Harness, id: &str, args: Args) {
    h.cmd_with(id, args);
    h.run_jobs();
}

fn chrome(h: &mut Harness) -> String {
    h.frame_text_with(AsciiOptions {
        show_chrome: true,
        show_cursor: false,
        mark_selection: false,
        ..AsciiOptions::default()
    })
    .lines()
    .skip_while(|l| !l.starts_with("status:"))
    .collect::<Vec<_>>()
    .join(
        "
",
    )
}

// verifies: AC-FR-503-1
#[test]
fn 無題の文書のステータス() {
    let mut s = Harness::new();
    let c = chrome(&mut s);
    assert!(
        c.starts_with(
            "status: 無題-1 | 行 1, 桁 1 UTF-8 LF
hints: "
        ),
        "{c}"
    );
}

// verifies: AC-FR-503-1
#[test]
fn 変更と位置と選択のステータス() {
    let mut s = Harness::new();
    s.text("あいう\nえお");
    let c = chrome(&mut s);
    assert!(
        c.starts_with("status: 無題-1 [+] | 行 2, 桁 3 UTF-8 LF"),
        "{c}"
    );
    // 選択: 「(N 文字選択)」
    run(&mut s, "select.all", Args::new());
    let c = chrome(&mut s);
    assert!(c.contains("(6 文字選択)"), "{c}");
}

// verifies: AC-FR-503-1
#[test]
fn ファイルの状態のステータス() {
    let mut s = Harness::new();
    s.fs.add_file("/w/a.txt", b"\xEF\xBB\xBFabc\r\ndef\r\n");
    run(
        &mut s,
        "file.open",
        Args::new().with_str("path", "/w/a.txt"),
    );
    let c = chrome(&mut s);
    assert!(
        c.starts_with("status: a.txt | 行 1, 桁 1 UTF-8 BOM CRLF"),
        "{c}"
    );
    // 改行が混在
    s.fs.add_file("/w/m.txt", b"a\nb\r\nc\n");
    run(
        &mut s,
        "file.open",
        Args::new().with_str("path", "/w/m.txt"),
    );
    assert!(chrome(&mut s).contains("混在"), "{}", chrome(&mut s));
    // 読み取り専用
    s.fs.add_file("/w/bad.txt", b"x\x9A");
    run(
        &mut s,
        "file.open",
        Args::new().with_str("path", "/w/bad.txt"),
    );
    // 不正バイトが多いので確認が出る: 読み取り専用で開く
    s.text("r");
    s.run_jobs();
    let c = chrome(&mut s);
    assert!(c.starts_with("status: bad.txt (3/3) [RO] |"), "{c}");
}

#[test]
fn 短い形と優先度は_core_が決めて渡す() {
    let mut s = Harness::new();
    let f = s.frame();
    let pos = f
        .status
        .items
        .iter()
        .find(|i| i.kind == eve_view::StatusKind::Position)
        .unwrap();
    assert_eq!(pos.priority, 4);
    assert_eq!(pos.short.as_ref().map(|t| t.width), Some(3), "1:1");
    let long: u16 = pos.long.spans.iter().map(|sp| sp.width).sum();
    assert_eq!(pos.long.width, long, "幅は core が計算済み");
    let name = &f.status.items[0];
    assert_eq!(name.kind, eve_view::StatusKind::Path);
    assert_eq!(name.long.cut, eve_view::CutPolicy::Middle);
}

#[test]
fn 保存の成功は情報メッセージで_次のキーで消える() {
    let mut s = Harness::new();
    s.text("hi");
    run(
        &mut s,
        "file.save_as",
        Args::new().with_str("path", "/w/n.txt"),
    );
    let f = s.frame();
    let m = f.messages.current.expect("メッセージ");
    assert_eq!(m.level, MessageLevel::Info);
    let text: String = m.text.spans.iter().map(|sp| sp.text.as_str()).collect();
    assert_eq!(text, "/w/n.txt を保存しました");
    assert!(m.hint.is_none());
    assert_eq!(f.messages.unread, 0, "他に未読はない");
    s.keys("right");
    assert!(
        s.frame().messages.current.is_none(),
        "次のキー入力で消える (Info)"
    );
}

#[test]
fn 変更なしの保存の文言() {
    let mut s = Harness::new();
    s.fs.add_file("/w/a.txt", b"abc");
    run(
        &mut s,
        "file.open",
        Args::new().with_str("path", "/w/a.txt"),
    );
    run(&mut s, "file.save", Args::new());
    let c = chrome(&mut s);
    assert!(
        c.contains("message[Info]: 変更はありません (ファイルは書き換えていません)"),
        "{c}"
    );
}

// verifies: AC-FR-505-1
#[test]
fn エラーは_text_と_hint_の_2_部でキー表記を解決する() {
    let mut s = Harness::new();
    // 親フォルダが無いパスは、新しいファイルとして開いて警告する (確認は出さない)
    run(
        &mut s,
        "file.open",
        Args::new().with_str("path", "/nodir/a.txt"),
    );
    s.run_jobs();
    let f = s.frame();
    let m = f.messages.current.expect("メッセージ");
    assert_eq!(m.level, MessageLevel::Warn);
    let text: String = m.text.spans.iter().map(|sp| sp.text.as_str()).collect();
    assert!(text.starts_with("/nodir/a.txt のフォルダ "), "{text}");
    let hint: String = m
        .hint
        .expect("hint")
        .spans
        .iter()
        .map(|sp| sp.text.as_str())
        .collect();
    assert_eq!(
        hint,
        "フォルダを作ってから保存するか、別名で保存 (Ctrl+Shift+S) してください"
    );
    // Warn は Esc で消える。カーソル移動では消えない
    s.keys("left");
    assert!(s.frame().messages.current.is_some());
    s.keys("esc");
    assert!(s.frame().messages.current.is_none());
}

// verifies: AC-FR-504-1
#[test]
fn ヒントバーは割当のある項目を優先度の降順で出す() {
    let mut s = Harness::new();
    let f = s.frame();
    let words: Vec<String> = f
        .hints
        .items
        .iter()
        .map(|h| h.text.spans.iter().map(|sp| sp.text.as_str()).collect())
        .collect();
    assert_eq!(
        words,
        [
            "終了",
            "コマンド",
            "保存",
            "検索",
            "開く",
            "元に戻す",
            "行へ移動"
        ]
    );
    assert!(
        f.hints
            .items
            .windows(2)
            .all(|w| w[0].priority >= w[1].priority)
    );
    // キー: file.save は Ctrl+S
    let k = &f.hints.items[2].keys[0].strokes[0];
    assert_eq!((k.key, k.mods), (KeyCode::Char('s'), Modifiers::CTRL));
}

#[test]
fn チョードの保留中は取消だけを出す() {
    let mut s = Harness::new();
    s.keys("ctrl+k");
    let f = s.frame();
    assert_eq!(f.hints.context, eve_view::HintContext::Overlay);
    assert_eq!(f.hints.items.len(), 1);
    let word: String = f.hints.items[0]
        .text
        .spans
        .iter()
        .map(|sp| sp.text.as_str())
        .collect();
    assert_eq!(word, "取消");
    assert_eq!(f.hints.items[0].keys[0].strokes[0].key, KeyCode::Esc);
    let chord = f
        .status
        .items
        .iter()
        .find(|i| i.kind == eve_view::StatusKind::Chord)
        .expect("チョード");
    let t: String = chord.long.spans.iter().map(|sp| sp.text.as_str()).collect();
    assert_eq!(t, "Ctrl+K …");
    s.keys("esc");
    assert_eq!(s.frame().hints.context, eve_view::HintContext::Normal);
}

#[test]
fn 小さすぎる端末の文言と_制御文字を含まない文言() {
    let mut s = Harness::new();
    let f = s.frame();
    let t: String = f
        .texts
        .too_small
        .spans
        .iter()
        .map(|sp| sp.text.as_str())
        .collect();
    assert_eq!(t, "端末が小さすぎます (20x4 以上が必要)");
    // INV-F1: パスの制御文字は記号になる
    s.fs.add_file("/w/a\u{1B}b.txt", b"x");
    run(
        &mut s,
        "file.open",
        Args::new().with_str("path", "/w/a\u{1B}b.txt"),
    );
    let f = s.frame();
    let all: String = f
        .status
        .items
        .iter()
        .flat_map(|i| i.long.spans.iter())
        .map(|sp| sp.text.as_str())
        .collect();
    assert!(all.chars().all(|c| !c.is_control()), "{all:?}");
    let _ = (
        Document::new_untitled(),
        TextFormat::new_document(),
        Text::new(),
        ByteOff(0),
        Selection::point(ByteOff(0)),
    );
}

#[test]
fn 存在しない文言キーはキー名で出る() {
    let mut s = Harness::new();
    s.editor
        .state_mut()
        .show_message(MessageLevel::Info, "info.nothing.here", &[]);
    let f = s.frame();
    let t: String = f
        .messages
        .current
        .unwrap()
        .text
        .spans
        .iter()
        .map(|sp| sp.text.as_str())
        .collect();
    assert_eq!(t, "info.nothing.here");
}

fn message_text(f: &Frame) -> Option<String> {
    f.messages
        .current
        .as_ref()
        .map(|m| m.text.spans.iter().map(|sp| sp.text.as_str()).collect())
}

#[test]
fn 警告は_esc_か_1_秒後の次のキーで消える() {
    let mut s = Harness::new();
    s.fs.add_dir("/w2");
    run(
        &mut s,
        "file.open",
        Args::new().with_str("path", "/nodir/x.txt"),
    ); // 親フォルダなし → Warn
    assert_eq!(
        s.frame().messages.current.as_ref().map(|m| m.level),
        Some(MessageLevel::Warn)
    );
    s.advance_ms(500);
    s.keys("right");
    assert!(
        s.frame().messages.current.is_some(),
        "1 秒たつまでは消えない"
    );
    s.advance_ms(1000);
    s.keys("right");
    assert!(
        s.frame().messages.current.is_none(),
        "1 秒後の次のキーで消える"
    );
}

#[test]
fn 保留中の_esc_はチョードだけを取り消しメッセージは残す() {
    let mut s = Harness::new();
    s.editor.state_mut().show_message(
        MessageLevel::Error,
        "msg.save.failed",
        &[("path", "p"), ("reason", "r")],
    );
    s.keys("ctrl+k");
    s.keys("esc");
    assert!(
        s.frame().messages.current.is_some(),
        "1 回目の Esc はチョードの取消"
    );
    s.keys("esc");
    assert!(
        s.frame().messages.current.is_none(),
        "2 回目の Esc でメッセージが消える"
    );
}

#[test]
fn text_の入力でも情報メッセージは消える() {
    let mut s = Harness::new();
    s.editor
        .state_mut()
        .show_message(MessageLevel::Info, "info.save.done", &[("path", "p")]);
    s.text("a");
    assert!(s.frame().messages.current.is_none());
}

#[test]
fn 置き換えられた未読を数える() {
    let mut s = Harness::new();
    s.editor
        .state_mut()
        .show_message(MessageLevel::Info, "info.save.done", &[("path", "a")]);
    s.editor
        .state_mut()
        .show_message(MessageLevel::Info, "info.save.done", &[("path", "b")]);
    assert_eq!(s.frame().messages.unread, 1);
    assert_eq!(
        message_text(&s.frame()).as_deref(),
        Some("b を保存しました")
    );
}

#[test]
fn 未解決のプレースホルダを出さない() {
    let mut s = Harness::new();
    run(
        &mut s,
        "file.open",
        Args::new().with_str("path", "/nodir/x.txt"),
    );
    let t = message_text(&s.frame()).unwrap();
    assert!(!t.contains('{') && t.contains("/nodir"), "{t}");
    s.text("a");
    run(&mut s, "file.save", Args::new());
    let t = message_text(&s.frame()).unwrap();
    assert!(!t.contains('{'), "{t}");
    let mut s = Harness::new();
    s.fs.add_file("/w/bin.dat", b"\x00\x01");
    run(
        &mut s,
        "file.open",
        Args::new().with_str("path", "/w/bin.dat"),
    );
    // バイナリは確認が出る。中止するとメッセージが出る
    s.keys("esc");
    s.run_jobs();
    let t = message_text(&s.frame()).unwrap();
    assert!(!t.contains('{'), "{t}");
}

#[test]
fn 不正バイトのエスケープ文字は文言でも記号になる() {
    let mut s = Harness::new();
    s.editor.state_mut().show_message(
        MessageLevel::Info,
        "info.save.done",
        &[("path", &format!("a{}b", eve_encoding::escape_byte(0x9A)))],
    );
    let t = message_text(&s.frame()).unwrap();
    assert_eq!(t, "a<0x9A>b を保存しました");
}
