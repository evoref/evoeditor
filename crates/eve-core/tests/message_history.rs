//! メッセージ履歴 (docs/04 §8.9、01 FR-505): `Ctrl+K M` で、これまでのメッセージを古い順に並べた全画面のスクロールを開く。

use eve_testkit::Harness;
use eve_view::{HelpLine, HelpLineKind, MessageLevel, OverlayModel};

/// メッセージを 1 件出す (文言は何でもよい)。
fn say(h: &mut Harness, level: MessageLevel) {
    h.editor
        .state_mut()
        .show_message(level, "info.save.unchanged", &[]);
}

fn text_of(t: &eve_view::CellText) -> String {
    t.spans.iter().map(|s| s.text.to_string()).collect()
}

fn history(h: &mut Harness) -> (String, Vec<HelpLine>, usize) {
    let f = h.frame();
    let OverlayModel::Help {
        title,
        lines,
        scroll,
        ..
    } = f.overlays.into_iter().next().expect("履歴が出ている")
    else {
        panic!("履歴ではない")
    };
    (text_of(&title), lines, scroll)
}

#[test]
fn 履歴が空なら案内の一行を出す() {
    let mut h = Harness::new().with_size(80, 20);
    h.keys("ctrl+k m");
    assert_eq!(h.overlay(), "help");
    let (title, lines, _) = history(&mut h);
    assert_eq!(title, "メッセージ履歴");
    assert_eq!(lines.len(), 1);
    assert_eq!(text_of(&lines[0].text), "メッセージはまだありません");
}

#[test]
fn 出したメッセージが古い順に並び_レベルと経過時間が付く() {
    let mut h = Harness::new().with_size(80, 20);
    say(&mut h, MessageLevel::Info);
    h.advance_ms(65_000);
    say(&mut h, MessageLevel::Warn);
    h.keys("ctrl+k m");
    let (_, lines, _) = history(&mut h);
    assert_eq!(lines.len(), 2);
    let k = |i: usize| text_of(lines[i].key.as_ref().unwrap());
    assert_eq!(k(0), "00:00 情報");
    assert_eq!(k(1), "01:05 警告");
    assert!(lines.iter().all(|l| l.kind == HelpLineKind::Entry));
}

#[test]
fn 置き換わって消えたメッセージも履歴に残り_開くと末尾が見える() {
    let mut h = Harness::new().with_size(80, 6);
    for _ in 0..30 {
        say(&mut h, MessageLevel::Info);
    }
    h.keys("ctrl+k m");
    let (_, lines, scroll) = history(&mut h);
    assert!(lines.len() >= 30, "{}", lines.len());
    assert_eq!(scroll, lines.len() - 4, "最新が見えるよう末尾にスクロール");
    h.keys("pageup");
    let (_, _, up) = history(&mut h);
    assert!(up < scroll);
    h.keys("esc");
    assert_eq!(h.overlay(), "");
}

#[test]
fn 履歴は千件を超えたら古いものから捨てる() {
    let mut h = Harness::new().with_size(80, 20);
    for _ in 0..1_200 {
        say(&mut h, MessageLevel::Error);
    }
    h.keys("ctrl+k m");
    let (_, lines, _) = history(&mut h);
    assert_eq!(lines.len(), 1_000);
}

#[test]
fn ヘルプは履歴ではなく従来どおり開く() {
    let mut h = Harness::new().with_size(80, 20);
    say(&mut h, MessageLevel::Info);
    h.keys("ctrl+k h");
    let (title, lines, _) = history(&mut h);
    assert_eq!(title, "ヘルプ");
    assert_eq!(text_of(&lines[0].text), "はじめに");
}
