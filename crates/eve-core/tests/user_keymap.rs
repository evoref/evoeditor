//! ユーザーのキーマップ (docs/07 §7、01 FR-604): `keymap.toml` の単一ストロークの上書き・追加・解除と、その診断。

use eve_config::{DiagKind, KeymapProblem, load_keymap_str};
use eve_testkit::Harness;

/// `keymap.toml` の本文を読ませて、診断の種類の列を返す。
fn load(h: &mut Harness, text: &str) -> Vec<(String, DiagKind)> {
    let f = load_keymap_str(text);
    h.editor
        .set_user_keymap(&f)
        .into_iter()
        .map(|d| (d.key, d.kind))
        .collect()
}

fn problems(d: &[(String, DiagKind)]) -> Vec<KeymapProblem> {
    d.iter()
        .filter_map(|(_, k)| match k {
            DiagKind::Keymap(p) => Some(*p),
            _ => None,
        })
        .collect()
}

#[test]
fn 未割当のキーにコマンドを追加できる() {
    let mut h = Harness::new();
    let d = load(&mut h, "[keys.editor]\n\"f6\" = \"edit.duplicate_line\"\n");
    assert!(d.is_empty(), "{d:?}");
    h.paste("a\nb\n");
    h.keys("ctrl+home f6");
    assert_eq!(h.doc_text(), "a\na\nb\n");
}

#[test]
fn 既定の割当を別のコマンドに付け替えると_ラベルとヘルプも追従する() {
    let mut h = Harness::new();
    let d = load(&mut h, "[keys.editor]\n\"ctrl+s\" = \"file.save_as\"\n");
    assert!(d.is_empty(), "{d:?}");
    h.keys("ctrl+s");
    assert_eq!(h.overlay(), "prompt", "名前を付けて保存の入力が開く");
    h.keys("esc");
    h.keys("ctrl+k h");
    let f = h.frame();
    let eve_view::OverlayModel::Help { lines, .. } = &f.overlays[0] else {
        panic!("ヘルプではない")
    };
    let key_of = |title: &str| {
        lines
            .iter()
            .find(|l| l.kind == eve_view::HelpLineKind::Entry && text_of(&l.text) == title)
            .and_then(|l| l.key.as_ref().map(text_of))
    };
    assert!(key_of("名前を付けて保存").unwrap().contains("Ctrl+S"));
}

fn text_of(t: &eve_view::CellText) -> String {
    t.spans.iter().map(|s| s.text.to_string()).collect()
}

#[test]
fn none_は既定の割当を遮蔽する() {
    let mut h = Harness::new();
    load(&mut h, "[keys.editor]\n\"ctrl+z\" = \"none\"\n");
    h.text("abc");
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "abc", "Undo されない");
}

#[test]
fn 配列は_when_の先頭から真の最初を採る() {
    let mut h = Harness::new();
    load(
        &mut h,
        "[keys.editor]\n\"f6\" = [ { command = \"edit.delete_line\", when = \"selection.empty\" }, \"edit.duplicate_line\" ]\n",
    );
    h.paste("a\nb\n");
    h.keys("ctrl+home f6");
    assert_eq!(h.doc_text(), "b\n", "選択が空なら先頭の割当");
}

#[test]
fn ユーザー層の_global_は既定層の_editor_に勝つ() {
    let mut h = Harness::new();
    load(&mut h, "[keys.global]\n\"ctrl+z\" = \"edit.select_all\"\n");
    h.text("abc");
    h.keys("ctrl+z");
    h.text("X");
    assert_eq!(h.doc_text(), "X");
}

#[test]
fn 問題のある割当だけを無効にして診断を返す() {
    let mut h = Harness::new();
    let d = load(
        &mut h,
        r#"
[keys.nowhere]
"f6" = "edit.delete_line"
[keys.editor]
"f7" = "no.such_command"
"ctrl+shift+nope" = "edit.delete_line"
"f8" = { command = "edit.delete_line", when = "((" }
"a" = "edit.delete_line"
"ctrl+k" = "edit.delete_line"
"f9" = "edit.duplicate_line"
"#,
    );
    for p in [
        KeymapProblem::UnknownContext,
        KeymapProblem::UnknownCommand,
        KeymapProblem::BadKey,
        KeymapProblem::BadWhen,
        KeymapProblem::PrintableKey,
        KeymapProblem::ChordPrefix,
    ] {
        assert!(problems(&d).contains(&p), "{p:?} {d:?}");
    }
    // 正しい割当は使える
    h.paste("a\n");
    h.keys("ctrl+home f9");
    assert_eq!(h.doc_text(), "a\na\n");
    // 文字入力は奪われない
    h.keys("ctrl+home");
    h.text("a");
    assert!(h.doc_text().starts_with("aa"));
}

#[test]
fn 二ストロークと_args_は未対応の情報診断() {
    let mut h = Harness::new();
    let d = load(
        &mut h,
        "[keys.editor]\n\"ctrl+k ctrl+c\" = \"edit.delete_line\"\n\"f6\" = { command = \"edit.delete_line\", args = { a = 1 } }\n",
    );
    assert_eq!(d.len(), 2);
    assert!(d.iter().all(|(_, k)| *k == DiagKind::Unsupported));
}

#[test]
fn 脱出キーに届かなくなる割当は外して既定を残す() {
    let mut h = Harness::new();
    let d = load(
        &mut h,
        "[keys.editor]\n\"ctrl+q\" = \"none\"\n[keys.global]\n\"ctrl+p\" = \"none\"\n",
    );
    assert!(problems(&d).contains(&KeymapProblem::EscapeKey), "{d:?}");
    // ctrl+q は none にされたが、app.quit はほかの既定 (ctrl+q 以外があれば) か復活した既定で届く
    h.text("x");
    h.keys("ctrl+q");
    assert_eq!(h.overlay(), "dialog", "終了の確認が出る");
}
