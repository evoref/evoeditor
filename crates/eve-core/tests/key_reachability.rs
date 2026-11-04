//! この端末で届く割当 (docs/04 §7.8・§7.3a・§8.6): キーの表記は届く割当から選び、ヘルプのキー一覧は届かない割当に印を付ける。

use eve_testkit::Harness;
use eve_view::{Capabilities, HelpLine, HelpLineKind, KeyCode, KeyStroke, Modifiers, OverlayModel};

fn text_of(t: &eve_view::CellText) -> String {
    t.spans.iter().map(|s| s.text.to_string()).collect()
}

fn stroke(mods: Modifiers, key: KeyCode) -> KeyStroke {
    KeyStroke { mods, key }
}

/// Windows Terminal の横取り (04 §7.3a) のうち、試験に使う一部。
fn wt_caps() -> Capabilities {
    Capabilities {
        intercepted_keys: vec![
            stroke(Modifiers::CTRL | Modifiers::SHIFT, KeyCode::Char('p')),
            stroke(Modifiers::CTRL | Modifiers::SHIFT, KeyCode::End),
            stroke(Modifiers::ALT, KeyCode::Up),
        ],
        ..Capabilities::default()
    }
}

fn help_lines(h: &mut Harness) -> Vec<HelpLine> {
    h.keys("ctrl+k h");
    let f = h.frame();
    let OverlayModel::Help { lines, .. } = f.overlays.into_iter().next().expect("ヘルプ") else {
        panic!("ヘルプではない")
    };
    lines
}

fn entry_keys(lines: &[HelpLine], title: &str) -> String {
    entry_keys_with(lines, title, "")
}

/// 題が `title` で、キーに `has` を含む行のキー (同じ題の行が複数あるとき)。
fn entry_keys_with(lines: &[HelpLine], title: &str, has: &str) -> String {
    lines
        .iter()
        .filter(|l| l.kind == HelpLineKind::Entry && text_of(&l.text) == title)
        .filter_map(|l| l.key.as_ref().map(text_of))
        .find(|k| k.contains(has))
        .unwrap_or_else(|| panic!("{title} の行"))
}

#[test]
fn ヘルプのキー一覧は届かない割当に印を付け_注記を出す() {
    let mut h = Harness::new().with_size(100, 30).with_caps(wt_caps());
    let lines = help_lines(&mut h);
    let palette = entry_keys(&lines, "コマンドパレット");
    assert!(palette.contains("Ctrl+P"), "{palette}");
    assert!(palette.contains("Ctrl+Shift+P*"), "横取りされる: {palette}");
    assert!(!palette.contains("Ctrl+P*"), "{palette}");
    assert!(
        lines
            .iter()
            .any(|l| text_of(&l.text).starts_with("* の付いたキーは")),
        "注記がある"
    );
}

#[test]
fn 拡張キーが要る割当は拡張キーが無ければ届かない() {
    let mut h = Harness::new().with_size(100, 30);
    let lines = help_lines(&mut h);
    let replace = entry_keys_with(&lines, "置換", "Ctrl+R");
    assert!(replace.contains("Ctrl+H*"), "[E]: {replace}");
    assert!(replace.contains("Ctrl+R") && !replace.contains("Ctrl+R*"));
    // 拡張キーが有効なら届く
    let mut h = Harness::new().with_size(100, 30).with_caps(Capabilities {
        enhanced_keys: true,
        ..Capabilities::default()
    });
    let lines = help_lines(&mut h);
    let replace = entry_keys_with(&lines, "置換", "Ctrl+R");
    assert!(!replace.contains('*'), "{replace}");
    assert!(
        !lines
            .iter()
            .any(|l| text_of(&l.text).starts_with("* の付いたキーは")),
        "届かない割当が無ければ注記も出さない"
    );
}

#[test]
fn パレットのキー表記は届く割当から選ぶ() {
    let keys_of = |h: &mut Harness, id: &str| -> Vec<Vec<KeyStroke>> {
        h.keys("ctrl+p");
        h.text(id);
        let f = h.frame();
        let OverlayModel::List { items, .. } = f.overlays.into_iter().next().expect("パレット")
        else {
            panic!("一覧ではない")
        };
        h.keys("esc");
        items[0].keys.iter().map(|k| k.strokes.clone()).collect()
    };
    // 横取りが無ければ第一の割当 (Alt+↑)
    let mut h = Harness::new().with_size(100, 30);
    let k = keys_of(&mut h, "edit.move_line_up");
    assert_eq!(k[0], vec![stroke(Modifiers::ALT, KeyCode::Up)]);
    // Alt+↑ が横取りされる端末では、代替のチョードを出す
    let mut h = Harness::new().with_size(100, 30).with_caps(wt_caps());
    let k = keys_of(&mut h, "edit.move_line_up");
    assert_ne!(k[0], vec![stroke(Modifiers::ALT, KeyCode::Up)], "{k:?}");
    assert_eq!(k[0].len(), 2, "Ctrl+K のチョード: {k:?}");
}
