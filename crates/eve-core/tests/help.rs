//! ヘルプ画面 (docs/04 §8.6、01 FR-514): 節を縦に並べた 1 本のスクロール、キー一覧とコマンド一覧は現在のキーマップとコマンドから作る。

use eve_testkit::Harness;
use eve_view::{HelpLine, HelpLineKind, OverlayModel};

fn text_of(t: &eve_view::CellText) -> String {
    t.spans.iter().map(|s| s.text.to_string()).collect()
}

/// (全行, 先頭に描く行の番号)。
fn help(h: &mut Harness) -> (Vec<HelpLine>, usize) {
    let f = h.frame();
    let OverlayModel::Help { lines, scroll, .. } =
        f.overlays.into_iter().next().expect("ヘルプが出ている")
    else {
        panic!("ヘルプではない")
    };
    (lines, scroll)
}

fn find<'a>(lines: &'a [HelpLine], kind: HelpLineKind, text: &str) -> Option<&'a HelpLine> {
    lines
        .iter()
        .find(|l| l.kind == kind && text_of(&l.text) == text)
}

fn key_of(l: &HelpLine) -> String {
    l.key.as_ref().map(text_of).unwrap_or_default()
}

#[test]
fn ctrl_k_h_でヘルプが開き_先頭は_はじめに() {
    let mut h = Harness::new().with_size(80, 20);
    h.keys("ctrl+k h");
    assert_eq!(h.editor.state().overlay_kind(), "help");
    let (lines, scroll) = help(&mut h);
    assert_eq!(scroll, 0);
    assert_eq!(lines[0].kind, HelpLineKind::Heading);
    assert_eq!(text_of(&lines[0].text), "はじめに");
    let intro = text_of(&lines[1].text);
    assert!(
        intro.contains("Ctrl+S"),
        "{{key:…}} がキーマップから解決される: {intro}"
    );
    for sec in ["キー一覧", "コマンド一覧", "端末の設定"] {
        assert!(find(&lines, HelpLineKind::Heading, sec).is_some(), "{sec}");
    }
}

#[test]
fn キー一覧は割当のあるコマンドをカテゴリ別に並べ_複数の割当を並べる() {
    let mut h = Harness::new().with_size(80, 20);
    h.keys("ctrl+k h");
    let (lines, _) = help(&mut h);
    let save = find(&lines, HelpLineKind::Entry, "保存").expect("保存");
    assert_eq!(key_of(save), "Ctrl+S");
    let save_as = find(&lines, HelpLineKind::Entry, "名前を付けて保存").expect("名前を付けて保存");
    let k = key_of(save_as);
    assert!(k.contains("Ctrl+Shift+S") && k.contains("Ctrl+K S"), "{k}");
    assert!(find(&lines, HelpLineKind::Subheading, "ファイル").is_some());
    // 割当のないコマンド (バッファ一覧の `list` など以外) はキー一覧に出ない: パレットだけのコマンド
    let keys_start = lines
        .iter()
        .position(|l| text_of(&l.text) == "キー一覧")
        .unwrap();
    let cmds_start = lines
        .iter()
        .position(|l| text_of(&l.text) == "コマンド一覧")
        .unwrap();
    assert!(
        lines[keys_start..cmds_start]
            .iter()
            .all(|l| l.kind != HelpLineKind::Entry || l.key.is_some()),
        "キー一覧の行はすべてキーを持つ"
    );
}

#[test]
fn コマンド一覧は_commandid_とタイトルと説明を出す() {
    let mut h = Harness::new().with_size(80, 20);
    h.keys("ctrl+k h");
    let (lines, _) = help(&mut h);
    let cmds_start = lines
        .iter()
        .position(|l| text_of(&l.text) == "コマンド一覧")
        .unwrap();
    let e = lines[cmds_start..]
        .iter()
        .find(|l| l.key.as_ref().is_some_and(|k| text_of(k) == "file.save"))
        .expect("file.save");
    let t = text_of(&e.text);
    assert!(t.starts_with("保存"), "{t}");
    assert!(t.contains('—'), "説明が続く: {t}");
    // 専用の内部コマンドは出ない
    assert!(!lines.iter().any(|l| {
        l.key
            .as_ref()
            .is_some_and(|k| text_of(k).starts_with("internal."))
    }));
}

#[test]
fn 矢印とページ送りでスクロールし_端で止まり_esc_で閉じる() {
    let mut h = Harness::new().with_size(80, 12);
    h.keys("ctrl+k h");
    let (lines, _) = help(&mut h);
    let view = 12 - 2;
    let max = lines.len() - view;
    h.keys("down");
    assert_eq!(help(&mut h).1, 1);
    h.keys("up");
    h.keys("up");
    assert_eq!(help(&mut h).1, 0, "先頭で止まる");
    h.keys("pagedown");
    assert_eq!(help(&mut h).1, view - 1, "1 画面 (重なりを 1 行残す)");
    h.keys("pageup");
    assert_eq!(help(&mut h).1, 0);
    for _ in 0..50 {
        h.keys("pagedown");
    }
    assert_eq!(help(&mut h).1, max, "末尾で止まる");
    h.keys("esc");
    assert_eq!(h.editor.state().overlay_kind(), "");
}

#[test]
fn ヘルプの間は文書を編集できない() {
    let mut h = Harness::new().with_size(80, 20);
    h.keys("ctrl+k h");
    h.text("abc");
    h.keys("enter");
    h.keys("esc");
    assert_eq!(h.doc_text(), "");
}

#[test]
fn キー一覧とコマンド一覧は該当の節へスクロールして開く() {
    let mut h = Harness::new().with_size(80, 12);
    h.editor.execute(eve_core::CommandInvocation::new(
        "help.keys",
        eve_core::InvocationSource::Palette,
    ));
    let (lines, scroll) = help(&mut h);
    assert_eq!(text_of(&lines[scroll].text), "キー一覧");
    h.keys("esc");
    h.editor.execute(eve_core::CommandInvocation::new(
        "help.commands",
        eve_core::InvocationSource::Palette,
    ));
    let (lines, scroll) = help(&mut h);
    assert_eq!(text_of(&lines[scroll].text), "コマンド一覧");
}

#[test]
fn パレットから開ける() {
    let mut h = Harness::new().with_size(80, 20);
    h.keys("ctrl+p");
    h.text("help.open");
    h.keys("enter");
    assert_eq!(h.editor.state().overlay_kind(), "help");
}
