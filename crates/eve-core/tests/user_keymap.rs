//! ユーザーのキーマップ (docs/07 §7、01 FR-604 / FR-608): `keymap.toml` の単一ストロークとチョードの上書き・追加・
//! 解除と、その診断 (衝突の検出を含む)。

use eve_config::{DiagKind, KeymapProblem, load_keymap_str};
use eve_testkit::Harness;
use eve_testkit::helpers::cell_text;

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
            .find(|l| l.kind == eve_view::HelpLineKind::Entry && cell_text(&l.text) == title)
            .and_then(|l| l.key.as_ref().map(cell_text))
    };
    assert!(key_of("名前を付けて保存").unwrap().contains("Ctrl+S"));
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

#[test]
fn 脱出キーの診断は書かれた行と桁とキーの表記で出す() {
    let mut h = Harness::new();
    let f =
        load_keymap_str("[keys.editor]\n\"f6\" = \"edit.delete_line\"\n  \"ctrl+q\" = \"none\"\n");
    let d = h.editor.set_user_keymap(&f);
    let e = d
        .iter()
        .find(|d| d.kind == DiagKind::Keymap(KeymapProblem::EscapeKey))
        .unwrap_or_else(|| panic!("{d:?}"));
    // 整形した表記 (Ctrl+Q) ではなく、書かれたとおり
    assert_eq!(e.key, "keys.editor.ctrl+q");
    assert_eq!((e.line, e.column), (3, 3));
}

#[test]
fn when_に知らない変数がある割当は警告して使わない() {
    let mut h = Harness::new();
    let f = load_keymap_str(concat!(
        "[keys.editor]
",
        "\"f6\" = { command = \"edit.delete_line\", when = \"!selection.empty || doc.dirty\" }
",
        "\"f7\" = { command = \"edit.duplicate_line\", when = \"doc.modified\" }
",
    ));
    let d = h.editor.set_user_keymap(&f);
    let unknown: Vec<(&str, &str)> = d
        .iter()
        .filter(|d| d.kind == DiagKind::Keymap(KeymapProblem::UnknownWhenVar))
        .map(|d| (d.key.as_str(), d.value.as_str()))
        .collect();
    assert_eq!(unknown, [("keys.editor.f6", "doc.dirty")], "{d:?}");
    // 知らない変数の割当は使わない (既定の f6 も無いので何も起きない)
    h.paste(
        "a
b
",
    );
    h.keys("ctrl+home f6");
    assert_eq!(
        h.doc_text(),
        "a
b
"
    );
    // 07 §7.3 の `doc.modified` は使える (貼り付けたので未保存の変更がある)
    h.keys("f7");
    assert_eq!(
        h.doc_text(),
        "a
a
b
"
    );
}

// バッファ一覧の参照順は list → prompt → global (07 §7.1)。`[keys.palette]` の割当はバッファ一覧に届かない
#[test]
fn palette_の割当はバッファ一覧に漏れず_list_の割当は両方に効く() {
    let mut h = Harness::new();
    let d = load(
        &mut h,
        "[keys.palette]\n\"f6\" = \"ui.list_next\"\n[keys.list]\n\"f7\" = \"ui.list_next\"\n",
    );
    assert!(d.is_empty(), "{d:?}");
    let selected = |h: &mut Harness| {
        let f = h.frame();
        let eve_view::OverlayModel::List { selected, .. } = &f.overlays[0] else {
            panic!("一覧ではない")
        };
        *selected
    };
    h.keys("ctrl+p");
    h.keys("f6");
    assert_eq!(selected(&mut h), 1, "パレットでは palette の割当が効く");
    h.keys("f7");
    assert_eq!(selected(&mut h), 2, "パレットでは list の割当も効く");
    h.keys("esc");
    h.cmd("file.new");
    h.cmd("file.new");
    h.cmd("buffer.list");
    assert_eq!(h.overlay(), "buffer_list");
    let before = selected(&mut h);
    h.keys("f6");
    assert_eq!(
        selected(&mut h),
        before,
        "palette の割当はバッファ一覧に届かない"
    );
    h.keys("f7");
    assert_eq!(selected(&mut h), before + 1, "list の割当は届く");
}

// ---------- v0.5 の M29: ユーザー定義のチョード (FR-604) と衝突の検出 (FR-608) ----------

const KEYMAP: &str = "/evo/config/keymap.toml";

/// `keymap.toml` の本文を読ませて、診断を返す。
fn diagnostics(h: &mut Harness, text: &str) -> Vec<eve_config::Diagnostic> {
    h.editor.set_user_keymap(&load_keymap_str(text))
}

/// `p` の診断の (キー, 値, 行)。
fn of_kind(d: &[eve_config::Diagnostic], p: KeymapProblem) -> Vec<(String, String, u32)> {
    d.iter()
        .filter(|d| d.kind == DiagKind::Keymap(p))
        .map(|d| (d.key.clone(), d.value.clone(), d.line))
        .collect()
}

/// ヘルプのキー一覧 (`help.keys`) の (タイトル → キーの列) と本文の行。
fn help_keys(h: &mut Harness) -> (Vec<(String, String)>, Vec<String>) {
    h.cmd("help.keys");
    let f = h.frame();
    let eve_view::OverlayModel::Help { lines, .. } = &f.overlays[0] else {
        panic!("ヘルプではない")
    };
    let entries = lines
        .iter()
        .filter(|l| l.kind == eve_view::HelpLineKind::Entry)
        .filter_map(|l| Some((cell_text(&l.text), cell_text(l.key.as_ref()?))))
        .collect();
    let body = lines
        .iter()
        .filter(|l| l.kind == eve_view::HelpLineKind::Body)
        .map(|l| cell_text(&l.text))
        .collect();
    (entries, body)
}

// verifies: AC-FR-604-1
#[test]
fn ユーザーのチョードを解決し_ストロークの数に上限は無い() {
    let mut h = Harness::new();
    let d = load(
        &mut h,
        "[keys.editor]\n\"ctrl+j d\" = \"edit.duplicate_line\"\n\"ctrl+j ctrl+j ctrl+j l\" = \"edit.delete_line\"\n",
    );
    assert!(d.is_empty(), "{d:?}");
    h.paste("a\nb\n");
    h.keys("ctrl+home ctrl+j");
    // 2 打目の文字は端末から Text で届く (K1)
    h.text("d");
    assert_eq!(h.doc_text(), "a\na\nb\n");
    h.keys("ctrl+j ctrl+j ctrl+j");
    assert_eq!(h.doc_text(), "a\na\nb\n", "途中では何もしない");
    h.text("l");
    assert_eq!(h.doc_text(), "a\nb\n");
}

// verifies: AC-FR-604-1
#[test]
fn 割当の_args_はコマンドに渡る() {
    let mut h = Harness::new();
    let d = load(
        &mut h,
        concat!(
            "[keys.editor]\n",
            "\"ctrl+j i\" = { command = \"edit.insert_text\", args = { text = \"hello\" } }\n",
            "\"f6\" = { command = \"cursor.goto_line\", args = { line = 2 } }\n",
            "\"f7\" = { command = \"cursor.goto_line\", args = { line = \"$\" } }\n",
        ),
    );
    assert!(d.is_empty(), "{d:?}");
    h.keys("ctrl+j");
    h.text("i");
    assert_eq!(h.doc_text(), "hello");
    h.paste("\nx\ny");
    h.keys("f6");
    assert_eq!(h.selection_range(), (6, 6), "2 行目の先頭");
    h.keys("f7");
    assert_eq!(h.selection_range().0, 8, "最終行");
}

// verifies: AC-FR-604-1
#[test]
fn none_はチョードの割当も解除し_ほかのチョードは残る() {
    let mut h = Harness::new();
    let d = load(&mut h, "[keys.editor]\n\"ctrl+k d\" = \"none\"\n");
    assert!(d.is_empty(), "{d:?}");
    h.paste("a\n");
    h.keys("ctrl+home ctrl+k");
    h.text("d");
    assert_eq!(h.doc_text(), "a\n", "複製されず、d も入らない");
    h.keys("ctrl+k");
    h.text("l");
    assert_eq!(h.doc_text(), "", "ほかの既定のチョード (行の削除) は効く");
}

// verifies: AC-FR-604-2, AC-FR-608-1
#[test]
fn ユーザーのチョードが既定の単体を無効にしたら_チョードが勝ち_警告で名指しする() {
    let mut h = Harness::new();
    let d = diagnostics(
        &mut h,
        "[keys.editor]\n\"f6\" = \"edit.delete_line\"\n\"ctrl+s x\" = \"edit.duplicate_line\"\n",
    );
    let shadow = of_kind(&d, KeymapProblem::ChordShadowsDefault);
    assert_eq!(
        shadow,
        [(
            "keys.editor.ctrl+s x".to_owned(),
            "editor: Ctrl+S = file.save".to_owned(),
            3
        )],
        "{d:?}"
    );
    assert!(
        d.iter()
            .all(|d| d.severity == eve_config::Severity::Warning)
    );
    h.text("a");
    h.keys("ctrl+s");
    assert_eq!(h.overlay(), "", "保存 (名前を尋ねる) にならず、続きを待つ");
    h.text("x");
    assert_eq!(h.doc_text(), "a\na");
}

// verifies: AC-FR-608-1
#[test]
fn ユーザーの割当どうしの衝突は短い側を名指しして警告し_チョードが勝つ() {
    let mut h = Harness::new();
    let d = diagnostics(
        &mut h,
        "[keys.editor]\n\"ctrl+j x\" = \"edit.duplicate_line\"\n\"ctrl+j\" = \"edit.delete_line\"\n",
    );
    assert_eq!(
        of_kind(&d, KeymapProblem::UserChordPrefix),
        [("keys.editor.ctrl+j".to_owned(), "ctrl+j x".to_owned(), 3)],
        "{d:?}"
    );
    h.paste("a\n");
    h.keys("ctrl+home ctrl+j");
    assert_eq!(h.doc_text(), "a\n", "単体 (行の削除) は使わない");
    h.text("x");
    assert_eq!(h.doc_text(), "a\na\n");
}

// verifies: AC-FR-608-1
#[test]
fn 既定のチョードの先頭と同じ単体は_global_でも警告して使わない() {
    let mut h = Harness::new();
    let d = diagnostics(&mut h, "[keys.prompt]\n\"ctrl+k\" = \"select.all\"\n");
    assert_eq!(
        of_kind(&d, KeymapProblem::ChordPrefix),
        [("keys.prompt.ctrl+k".to_owned(), "ctrl+k".to_owned(), 2)],
        "prompt は global (Ctrl+K R など) と同時に参照される: {d:?}"
    );
}

// verifies: AC-FR-608-1
#[test]
fn 別の書き方の同じキーは後の行を使い_情報の診断を出す() {
    let mut h = Harness::new();
    let d = diagnostics(
        &mut h,
        "[keys.editor]\n\"ctrl+j\" = \"edit.delete_line\"\n\"mod+j\" = \"edit.duplicate_line\"\n",
    );
    assert_eq!(
        of_kind(&d, KeymapProblem::DuplicateKey),
        [("keys.editor.mod+j".to_owned(), "ctrl+j".to_owned(), 3)],
        "{d:?}"
    );
    assert_eq!(d[0].severity, eve_config::Severity::Info);
    h.paste("a\n");
    h.keys("ctrl+home ctrl+j");
    assert_eq!(h.doc_text(), "a\na\n", "後の行 (複製) を使う");
    // 表のキーの順 (ctrl+j が先) と行の順が逆でも、後の行を使う
    let mut h = Harness::new();
    let d = diagnostics(
        &mut h,
        "[keys.editor]\n\"mod+j\" = \"edit.duplicate_line\"\n\"ctrl+j\" = \"edit.delete_line\"\n",
    );
    assert_eq!(
        of_kind(&d, KeymapProblem::DuplicateKey),
        [("keys.editor.ctrl+j".to_owned(), "mod+j".to_owned(), 3)],
        "{d:?}"
    );
    h.paste("a\nb\n");
    h.keys("ctrl+home ctrl+j");
    assert_eq!(h.doc_text(), "b\n", "後の行 (削除) を使う");
}

// verifies: AC-FR-604-2, AC-FR-608-1
#[test]
fn 脱出キーを先頭にするチョードはエラーにして外し_既定を残す() {
    let mut h = Harness::new();
    let d = diagnostics(
        &mut h,
        "[keys.global]\n\"ctrl+q x\" = \"edit.delete_line\"\n",
    );
    let e = d
        .iter()
        .find(|d| d.kind == DiagKind::Keymap(KeymapProblem::EscapeKey))
        .unwrap_or_else(|| panic!("{d:?}"));
    assert_eq!(
        (e.key.as_str(), e.value.as_str(), e.severity),
        (
            "keys.global.ctrl+q x",
            "app.quit",
            eve_config::Severity::Error
        )
    );
    h.text("x");
    h.keys("ctrl+q");
    assert_eq!(h.overlay(), "dialog", "終了の確認が出る");
}

// verifies: AC-FR-608-1
#[test]
fn 一打目が文字キーのチョードは使わず_二打目以降の文字は使える() {
    let mut h = Harness::new();
    let d = diagnostics(
        &mut h,
        "[keys.editor]\n\"a b\" = \"edit.delete_line\"\n\"f6 b\" = \"edit.duplicate_line\"\n",
    );
    assert_eq!(
        of_kind(&d, KeymapProblem::PrintableKey),
        [("keys.editor.a b".to_owned(), "a b".to_owned(), 2)]
    );
    h.text("a");
    h.text("b");
    assert_eq!(h.doc_text(), "ab", "文字入力は奪われない");
    h.keys("f6");
    h.text("b");
    assert_eq!(h.doc_text(), "ab\nab");
}

// verifies: AC-FR-608-2
#[test]
fn 合わない割当はそれだけを使わずに警告し_同じファイルの他の割当は効く() {
    let mut h = Harness::new();
    let d = diagnostics(
        &mut h,
        concat!(
            "[keys.editor]\n",
            "\"f6\" = { command = \"cursor.goto_line\", args = { row = 1 } }\n",
            "\"f7\" = { command = \"cursor.goto_line\", args = { line = \"x\" } }\n",
            "\"f8\" = { command = \"file.set_line_ending\", args = { eol = \"lf2\" } }\n",
            "\"f9\" = { command = \"file.set_line_ending\", args = {} }\n",
            "\"f10\" = { command = \"edit.duplicate_line\", args = { a = 1 } }\n",
            "\"f11\" = \"edit.duplicate_line\"\n",
            "\"f12\" = \"no.such_command\"\n",
            "\"ctrl+j\" = { command = \"edit.delete_line\", when = \"((\" }\n",
            "\"ctrl+j x\" = { command = \"edit.delete_line\", when = \"doc.no_such\" }\n",
        ),
    );
    // 診断は表のキーの順に並ぶので、行の順に並べ替えて比べる
    let mut bad = of_kind(&d, KeymapProblem::BadArgs);
    bad.sort_by_key(|(_, _, line)| *line);
    let bad: Vec<(String, String)> = bad.into_iter().map(|(k, v, _)| (k, v)).collect();
    let want: Vec<(String, String)> = [
        ("f6", "row"),
        ("f7", "line"),
        ("f8", "eol"),
        ("f9", "eol"),
        ("f10", "a"),
    ]
    .iter()
    .map(|(k, v)| (format!("keys.editor.{k}"), (*v).to_owned()))
    .collect();
    assert_eq!(bad, want, "{d:?}");
    for p in [
        KeymapProblem::UnknownCommand,
        KeymapProblem::BadWhen,
        KeymapProblem::UnknownWhenVar,
    ] {
        assert_eq!(of_kind(&d, p).len(), 1, "{p:?} {d:?}");
    }
    assert!(
        d.iter()
            .all(|d| d.severity == eve_config::Severity::Warning),
        "どれも起動を止めない警告: {d:?}"
    );
    h.paste("a");
    h.keys("f6 f10 f11");
    assert_eq!(h.doc_text(), "a\na", "f11 だけが効く");
}

// verifies: AC-FR-604-3
#[test]
fn ヘルプのキー一覧にユーザーのチョードと無効にした単体の割当が出る() {
    let mut h = Harness::new();
    load(
        &mut h,
        "[keys.editor]\n\"ctrl+z x\" = \"edit.duplicate_line\"\n",
    );
    let (entries, body) = help_keys(&mut h);
    let key_of = |title: &str| {
        entries
            .iter()
            .find(|(t, _)| t == title)
            .map(|(_, k)| k.clone())
            .unwrap_or_else(|| panic!("{title}: {entries:?}"))
    };
    assert!(key_of("行を複製").contains("Ctrl+Z X"), "{entries:?}");
    assert_eq!(key_of("元に戻す"), "Ctrl+Z!", "{entries:?}");
    assert!(
        body.iter().any(|l| l.starts_with("! の付いたキーは")),
        "{body:?}"
    );
    // 衝突が無ければ印も注記も出ない
    let mut h = Harness::new();
    let (entries, body) = help_keys(&mut h);
    assert!(entries.iter().all(|(_, k)| !k.contains('!')), "{entries:?}");
    assert!(body.iter().all(|l| !l.starts_with("! の付いたキーは")));
}

/// 一覧 (`config.show_diagnostics`) の (各行の本文, 各行の右の列)。
fn diagnostics_list(h: &mut Harness) -> (Vec<String>, Vec<String>) {
    h.cmd("config.show_diagnostics");
    let f = h.frame();
    let eve_view::OverlayModel::List { items, .. } = &f.overlays[0] else {
        panic!("一覧が出ていない")
    };
    (
        items.iter().map(|i| cell_text(&i.label)).collect(),
        items
            .iter()
            .map(|i| i.detail.as_ref().map(cell_text).unwrap_or_default())
            .collect(),
    )
}

// verifies: AC-FR-608-1
#[test]
fn 衝突の診断は再読込でも出て_一覧にファイルと行で並ぶ() {
    let mut h = Harness::new();
    h.fs.add_file(
        KEYMAP,
        b"[keys.editor]\n\"ctrl+j\" = \"edit.delete_line\"\n\"mod+j\" = \"edit.duplicate_line\"\n\"ctrl+s x\" = \"edit.delete_line\"\n",
    );
    h.cmd("config.reload");
    h.run_jobs();
    let (rows, places) = diagnostics_list(&mut h);
    let mut found: Vec<(&str, &str)> = places
        .iter()
        .map(String::as_str)
        .zip(rows.iter().map(String::as_str))
        .collect();
    found.sort_unstable();
    assert_eq!(found.len(), 2, "{rows:?}");
    assert_eq!(found[0].0, "keymap.toml:3");
    assert!(
        found[0].1.starts_with("情報: ") && found[0].1.contains("ctrl+j"),
        "{rows:?}"
    );
    assert_eq!(found[1].0, "keymap.toml:4");
    assert!(
        found[1].1.starts_with("警告: ") && found[1].1.contains("Ctrl+S = file.save"),
        "{rows:?}"
    );
    // 起動は止まらず、チョードは効く
    h.keys("esc");
    h.paste("a");
    h.keys("ctrl+s");
    h.text("x");
    assert_eq!(h.doc_text(), "");
}

// ---------- PR #600 のレビューで直した点 ----------

// verifies: AC-FR-604-2
#[test]
fn 確認ダイアログでもチョードの保留中の文字はアクセラレータにしない() {
    let mut h = Harness::new();
    let d = diagnostics(&mut h, "[keys.dialog]\n\"ctrl+k d\" = \"ui.cancel\"\n");
    assert!(d.is_empty(), "{d:?}");
    h.text("x");
    h.keys("ctrl+q");
    assert_eq!(h.overlay(), "dialog", "未保存の終了の確認");
    h.keys("ctrl+k");
    // 「保存しない」のアクセラレータと同じ文字。保留の 2 打目として照合する
    h.text("d");
    assert_eq!(h.quit_code(), None, "変更を捨てて終了しない");
    assert_eq!(h.overlay(), "", "チョード (ui.cancel) で閉じる");
    assert_eq!(h.doc_text(), "x");
    // 複数書記素でも先頭だけを照合し、残りは捨てる
    h.keys("ctrl+q ctrl+k");
    h.text("dd");
    assert_eq!((h.quit_code(), h.overlay()), (None, ""));
    // 保留中の貼り付けは保留を捨て、ダイアログでは選択肢にしない
    h.keys("ctrl+q ctrl+k");
    h.paste("d");
    assert_eq!((h.quit_code(), h.overlay()), (None, "dialog"));
    // 保留が無ければ、文字は今までどおりアクセラレータ
    h.text("d");
    assert_eq!(h.quit_code(), Some(0), "保存しないで終了");
}

// verifies: AC-FR-604-1
#[test]
fn 二打目以降の大文字と_shift_付きの文字は端末の_text_で届く() {
    let mut h = Harness::new();
    let d = diagnostics(
        &mut h,
        "[keys.editor]\n\"ctrl+j A\" = \"edit.duplicate_line\"\n\"ctrl+j shift+b\" = \"edit.delete_line\"\n",
    );
    assert!(d.is_empty(), "{d:?}");
    h.paste("a\nb\n");
    h.keys("ctrl+home ctrl+j");
    h.text("A");
    assert_eq!(h.doc_text(), "a\na\nb\n");
    h.keys("ctrl+j");
    h.text("B");
    assert_eq!(h.doc_text(), "a\nb\n");
    // 小文字は別のキー (割当が無いので何もしない)
    h.keys("ctrl+j");
    h.text("a");
    assert_eq!(h.doc_text(), "a\nb\n");
}

// verifies: AC-FR-608-1
#[test]
fn 脱出の検査はフォーカスごと_esc_を先頭にするチョードはエラーにして外す() {
    let mut h = Harness::new();
    let d = diagnostics(
        &mut h,
        "[keys.global]\n\"esc x\" = \"edit.duplicate_line\"\n",
    );
    assert_eq!(
        d.iter()
            .filter(|d| d.kind == DiagKind::Keymap(KeymapProblem::EscapeKey))
            .map(|d| (d.key.as_str(), d.value.as_str()))
            .collect::<Vec<_>>(),
        [("keys.global.esc x", "ui.cancel")],
        "{d:?}"
    );
    // どのフォーカスでも Esc で取り消せる
    h.keys("ctrl+p");
    assert_eq!(h.overlay(), "palette");
    h.keys("esc");
    assert_eq!(h.overlay(), "");
    h.keys("ctrl+k h");
    assert_eq!(h.overlay(), "help");
    h.keys("esc");
    assert_eq!(h.overlay(), "");
}

// verifies: AC-FR-608-1
#[test]
fn when_付きで脱出キーを奪い得る割当もエラーにして外す() {
    let mut h = Harness::new();
    let d = diagnostics(
        &mut h,
        "[keys.global]\n\"ctrl+q\" = { command = \"file.save\", when = \"doc.modified\" }\n",
    );
    assert!(
        d.iter()
            .any(|d| d.kind == DiagKind::Keymap(KeymapProblem::EscapeKey) && d.value == "app.quit"),
        "{d:?}"
    );
    h.text("x");
    h.keys("ctrl+q");
    assert_eq!(
        h.overlay(),
        "dialog",
        "未保存でも終了の確認が出る (保存にならない)"
    );
}

// verifies: AC-FR-608-1
#[test]
fn 一部のフォーカスでだけ奪う衝突は使えない割当にせず_そうと分かる警告にする() {
    let mut h = Harness::new();
    let d = diagnostics(
        &mut h,
        concat!(
            "[keys.search]\n\"ctrl+a x\" = \"edit.duplicate_line\"\n\"ctrl+j x\" = \"edit.duplicate_line\"\n",
            "[keys.global]\n\"ctrl+j\" = \"edit.delete_line\"\n",
        ),
    );
    assert_eq!(
        of_kind(&d, KeymapProblem::ChordShadowsDefaultPartly),
        [(
            "keys.search.ctrl+a x".to_owned(),
            "prompt: Ctrl+A = select.all".to_owned(),
            2
        )],
        "{d:?}"
    );
    assert_eq!(
        of_kind(&d, KeymapProblem::ChordPrefixPartly),
        [(
            "keys.global.ctrl+j".to_owned(),
            "search: Ctrl+J X".to_owned(),
            5
        )],
        "{d:?}"
    );
    assert!(
        d.iter()
            .all(|d| d.severity == eve_config::Severity::Warning)
    );
    assert!(
        h.editor.keymap().disabled_bindings().is_empty(),
        "使えない割当にはしない"
    );
    // 編集面では global の ctrl+j が効く
    h.paste("a\nb\n");
    h.keys("ctrl+home ctrl+j");
    assert_eq!(h.doc_text(), "b\n");
    let (entries, _) = help_keys(&mut h);
    assert!(entries.iter().all(|(_, k)| !k.contains('!')), "{entries:?}");
}
