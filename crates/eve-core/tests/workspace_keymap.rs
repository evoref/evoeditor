//! ワークスペース層のキーの割当 (docs/07 §7.4 の WKEY-1〜WKEY-3、01 FR-705 AC-1): `.eve/keymap.toml` の割当は
//! 追加だけで、使わない割当は割当ごとに情報の診断を出し、同じファイルのほかの割当は使う。

use eve_config::{
    DiagKind, Diagnostic, KeyRejectReason, KeymapFile, KeymapProblem, Severity, WorkspaceProblem,
    load_keymap_str,
};
use eve_core::resolve_keymap;
use eve_testkit::Harness;
use eve_testkit::helpers::cell_text;

/// `.eve/keymap.toml` の本文をワークスペース層として読ませて、診断を返す。
fn workspace(h: &mut Harness, text: &str) -> Vec<Diagnostic> {
    h.editor.set_workspace_keymap(Some(&load_keymap_str(text)))
}

/// 使わなかった割当の (書かれたキー, 理由)。
fn rejected(d: &[Diagnostic]) -> Vec<(String, KeyRejectReason)> {
    d.iter()
        .filter_map(|d| match d.kind {
            DiagKind::Workspace(WorkspaceProblem::KeyRejected(r)) => {
                assert_eq!(d.severity, Severity::Info, "{d:?}");
                Some((d.key.clone(), r))
            }
            _ => None,
        })
        .collect()
}

/// ワークスペース層の割当の (コンテキスト, コマンド)。
fn workspace_bindings(h: &Harness) -> Vec<(&'static str, String)> {
    h.editor
        .keymap()
        .listed_bindings()
        .into_iter()
        .filter(|(layer, _)| *layer == "workspace")
        .map(|(_, b)| (b.context, b.command.as_str().to_owned()))
        .collect()
}

// verifies: AC-FR-705-1
#[test]
fn 未割当のキーへの追加は使う() {
    let mut h = Harness::new();
    let d = workspace(
        &mut h,
        "[keys.editor]\n\"ctrl+alt+j\" = \"edit.duplicate_line\"\n",
    );
    assert!(d.is_empty(), "{d:?}");
    h.paste("a\nb\n");
    h.keys("ctrl+home ctrl+alt+j");
    assert_eq!(h.doc_text(), "a\na\nb\n");
    assert_eq!(
        workspace_bindings(&h),
        [("editor", "edit.duplicate_line".to_owned())]
    );
    // 外すと効かない
    let d = h.editor.set_workspace_keymap(None);
    assert!(d.is_empty(), "{d:?}");
    let w = workspace_bindings(&h);
    assert!(w.is_empty(), "{w:?}");
    h.keys("ctrl+alt+j");
    assert_eq!(h.doc_text(), "a\na\nb\n");
}

// verifies: AC-FR-705-1
#[test]
fn 下の層に割当のあるキーは上書きも解除もできない() {
    let mut h = Harness::new();
    let d = workspace(
        &mut h,
        concat!(
            "[keys.editor]\n",
            "\"ctrl+z\" = \"select.all\"\n",
            "\"ctrl+alt+j\" = \"edit.duplicate_line\"\n",
            "[keys.global]\n",
            // 既定層の `editor` の割当 (`global` と同時に参照される)
            "\"alt+z\" = \"edit.duplicate_line\"\n",
            "[keys.search]\n",
            // 検索バーは `editor` を参照しないので、`editor` だけの割当とは出会わない
            "\"alt+z\" = \"edit.duplicate_line\"\n",
        ),
    );
    assert_eq!(
        rejected(&d),
        [
            ("keys.editor.ctrl+z".to_owned(), KeyRejectReason::Bound),
            ("keys.global.alt+z".to_owned(), KeyRejectReason::Bound),
        ]
    );
    // 位置は割当を書いた所
    assert_eq!((d[0].line, d[1].line), (2, 5), "{d:?}");
    h.text("abc");
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "", "既定の Undo のまま");
    assert_eq!(
        workspace_bindings(&h),
        [
            ("editor", "edit.duplicate_line".to_owned()),
            ("search", "edit.duplicate_line".to_owned()),
        ],
        "同じファイルのほかの割当は使う"
    );
}

// verifies: AC-FR-705-1
#[test]
fn 下の層の割当を使えなくするチョードと_下の層のチョードの先頭は使わない() {
    let mut h = Harness::new();
    let d = workspace(
        &mut h,
        concat!(
            "[keys.editor]\n",
            // 先頭の `ctrl+s` が既定の割当 (保存) のキー
            "\"ctrl+s x\" = \"edit.duplicate_line\"\n",
            // 既定のチョード `ctrl+k …` の先頭
            "\"ctrl+k\" = \"edit.duplicate_line\"\n",
            // 既定のチョードを延ばす列 (`ctrl+k d` は行の複製)
            "\"ctrl+k d x\" = \"select.all\"\n",
            "\"ctrl+alt+j x\" = \"edit.duplicate_line\"\n",
        ),
    );
    assert_eq!(
        rejected(&d),
        [
            ("keys.editor.ctrl+s x".to_owned(), KeyRejectReason::Shadows),
            ("keys.editor.ctrl+k".to_owned(), KeyRejectReason::Shadows),
            (
                "keys.editor.ctrl+k d x".to_owned(),
                KeyRejectReason::Shadows
            ),
        ]
    );
    assert!(
        h.editor.keymap().disabled_bindings().is_empty(),
        "下の層の割当は使えなくならない"
    );
    h.paste("a\n");
    h.keys("ctrl+home ctrl+alt+j");
    h.text("x");
    assert_eq!(h.doc_text(), "a\na\n", "ほかのチョードは使う");
}

// verifies: AC-FR-705-1
#[test]
fn 文字入力と取消し確定のキーで始まる割当は使わない() {
    let mut h = Harness::new();
    let d = workspace(
        &mut h,
        concat!(
            "[keys.editor]\n",
            "\"q\" = \"select.all\"\n",
            "\"Q\" = \"select.all\"\n",
            "\"space x\" = \"select.all\"\n",
            "\"enter\" = \"select.all\"\n",
            "\"tab x\" = \"select.all\"\n",
            "\"backspace x\" = \"select.all\"\n",
            "\"delete x\" = \"select.all\"\n",
            "\"esc x\" = \"select.all\"\n",
            "\"ctrl+alt+j\" = \"edit.duplicate_line\"\n",
        ),
    );
    let r = rejected(&d);
    assert_eq!(r.len(), 8, "{d:?}");
    assert!(
        r.iter().all(|(_, why)| *why == KeyRejectReason::Text),
        "{r:?}"
    );
    assert!(
        d.iter()
            .all(|d| d.kind != DiagKind::Keymap(KeymapProblem::PrintableKey)),
        "ユーザー層の警告は重ねて出さない: {d:?}"
    );
    h.text("q");
    assert_eq!(h.doc_text(), "q", "文字は本文に入る");
    assert_eq!(workspace_bindings(&h).len(), 1);
}

// verifies: AC-FR-705-1
#[test]
fn 引数と解除の割当は使わない() {
    let mut h = Harness::new();
    let d = workspace(
        &mut h,
        concat!(
            "[keys.editor]\n",
            "\"ctrl+alt+j\" = { command = \"edit.insert_text\", args = { text = \"x\" } }\n",
            "\"ctrl+alt+u\" = \"none\"\n",
        ),
    );
    assert_eq!(
        rejected(&d),
        [
            ("keys.editor.ctrl+alt+j".to_owned(), KeyRejectReason::Args),
            ("keys.editor.ctrl+alt+u".to_owned(), KeyRejectReason::None),
        ]
    );
    let w = workspace_bindings(&h);
    assert!(w.is_empty(), "{w:?}");
}

// verifies: AC-FR-705-1
#[test]
fn ファイルの操作と_app_のコマンドは付けられない() {
    // (f) の `perm` が `exec` のコマンドは v0.5 に無い (権限の網羅の match で、足したときに見直す)
    let mut h = Harness::new();
    let d = workspace(
        &mut h,
        concat!(
            "[keys.global]\n",
            "\"ctrl+alt+j\" = \"file.save_all\"\n",
            "\"ctrl+alt+u\" = \"file.toggle_read_only\"\n",
            "\"ctrl+alt+y\" = \"app.save_and_quit\"\n",
            "\"ctrl+alt+i\" = \"ui.palette_open\"\n",
            "\"ctrl+alt+o\" = \"cursor.doc_end\"\n",
        ),
    );
    assert_eq!(
        rejected(&d),
        [
            (
                "keys.global.ctrl+alt+j".to_owned(),
                KeyRejectReason::Command
            ),
            (
                "keys.global.ctrl+alt+u".to_owned(),
                KeyRejectReason::Command
            ),
            (
                "keys.global.ctrl+alt+y".to_owned(),
                KeyRejectReason::Command
            ),
        ]
    );
    assert_eq!(
        workspace_bindings(&h),
        [
            ("global", "ui.palette_open".to_owned()),
            ("global", "cursor.doc_end".to_owned()),
        ],
        "Ui と Motion のコマンドは付けられる"
    );
}

// verifies: AC-FR-705-1
#[test]
fn ほかの検証はユーザー層と同じ診断() {
    let mut h = Harness::new();
    let d = workspace(
        &mut h,
        concat!(
            "[keys.nowhere]\n",
            "\"ctrl+alt+j\" = \"edit.duplicate_line\"\n",
            "[keys.editor]\n",
            "\"ctrl+alt+u\" = \"no.such_command\"\n",
            "\"ctrl+shift+nope\" = \"edit.duplicate_line\"\n",
            "\"ctrl+alt+y\" = { command = \"edit.duplicate_line\", when = \"((\" }\n",
            "\"ctrl+alt+i\" = { command = \"edit.duplicate_line\", when = \"no.such_var\" }\n",
            "\"ctrl+alt+o\" = \"edit.duplicate_line\"\n",
        ),
    );
    let kinds: Vec<DiagKind> = d.iter().map(|d| d.kind.clone()).collect();
    assert_eq!(
        kinds,
        [
            DiagKind::Keymap(KeymapProblem::UnknownContext),
            DiagKind::Keymap(KeymapProblem::UnknownCommand),
            DiagKind::Keymap(KeymapProblem::BadKey),
            DiagKind::Keymap(KeymapProblem::BadWhen),
            DiagKind::Keymap(KeymapProblem::UnknownWhenVar),
        ]
    );
    assert!(d.iter().all(|d| d.severity == Severity::Warning), "{d:?}");
    assert_eq!(
        workspace_bindings(&h),
        [("editor", "edit.duplicate_line".to_owned())]
    );
}

// verifies: AC-FR-705-1
#[test]
fn 割当は_1_000_件まで読み_超えた分は_1_件の警告にする() {
    // 3 打のチョードで、互いに先頭にならない別のキーを 1,002 件作る
    let chars: Vec<char> = ('a'..='z').chain('0'..='9').collect();
    let keys: Vec<String> = chars
        .iter()
        .flat_map(|a| chars.iter().map(move |b| format!("ctrl+alt+j {a} {b}")))
        .take(1_002)
        .collect();
    let mut text = String::from("[keys.editor]\n");
    for k in &keys {
        text.push_str(&format!("\"{k}\" = \"edit.duplicate_line\"\n"));
    }
    let mut h = Harness::new();
    let d = workspace(&mut h, &text);
    assert_eq!(d.len(), 1, "{:?}", &d[..d.len().min(3)]);
    assert_eq!(
        d[0].kind,
        DiagKind::Workspace(WorkspaceProblem::TooManyKeys)
    );
    assert_eq!(d[0].severity, Severity::Warning);
    assert_eq!(d[0].value, "1000");
    assert_eq!(d[0].line, 1_002, "1,001 件目の割当の行");
    assert_eq!(workspace_bindings(&h).len(), 1_000);
    // 1,000 件目は効き、1,001 件目は読まない
    let press = |h: &mut Harness, k: &str| {
        let mut it = k.split(' ');
        h.keys(it.next().expect("先頭"));
        for c in it {
            h.text(c);
        }
    };
    h.paste("a\n");
    h.keys("ctrl+home");
    press(&mut h, &keys[999]);
    assert_eq!(h.doc_text(), "a\na\n");
    press(&mut h, &keys[1_000]);
    assert_eq!(h.doc_text(), "a\na\n");
}

// verifies: AC-FR-705-1
#[test]
fn ユーザー層を替えると検証し直す() {
    let mut h = Harness::new();
    let d = workspace(
        &mut h,
        "[keys.editor]\n\"ctrl+alt+j\" = \"edit.duplicate_line\"\n",
    );
    assert!(d.is_empty(), "{d:?}");
    // ユーザー層が同じキーを使うと、ワークスペースの割当は使わない (ユーザー層の診断には出さない)
    let u = h.editor.set_user_keymap(&load_keymap_str(
        "[keys.global]\n\"ctrl+alt+j\" = \"select.all\"\n",
    ));
    assert!(u.is_empty(), "{u:?}");
    let w = workspace_bindings(&h);
    assert!(w.is_empty(), "{w:?}");
    h.paste("a\nb\n");
    h.keys("ctrl+alt+j");
    h.text("X");
    assert_eq!(h.doc_text(), "X", "ユーザーの割当 (全選択) が効く");
    // ユーザー層から外すと、ワークスペースの割当がまた使われる
    h.editor.set_user_keymap(&KeymapFile::default());
    assert_eq!(
        workspace_bindings(&h),
        [("editor", "edit.duplicate_line".to_owned())]
    );
    h.keys("ctrl+alt+j");
    assert_eq!(h.doc_text(), "X\nX");
    // 再読込でワークスペースを読み直したときは、その時点のユーザー層に照らした診断を返す
    h.editor.set_user_keymap(&load_keymap_str(
        "[keys.editor]\n\"ctrl+alt+j\" = \"select.all\"\n",
    ));
    let d = workspace(
        &mut h,
        "[keys.editor]\n\"ctrl+alt+j\" = \"edit.duplicate_line\"\n",
    );
    assert_eq!(
        rejected(&d),
        [("keys.editor.ctrl+alt+j".to_owned(), KeyRejectReason::Bound)]
    );
}

// verifies: AC-FR-705-1
#[test]
fn ワークスペースの中のチョードの衝突はユーザー層と同じ警告にする() {
    let mut h = Harness::new();
    let d = workspace(
        &mut h,
        concat!(
            "[keys.editor]\n",
            "\"ctrl+alt+j\" = \"select.all\"\n",
            "\"ctrl+alt+j x\" = \"edit.duplicate_line\"\n",
        ),
    );
    let kinds: Vec<(String, DiagKind)> =
        d.iter().map(|d| (d.key.clone(), d.kind.clone())).collect();
    assert_eq!(
        kinds,
        [(
            "keys.editor.ctrl+alt+j".to_owned(),
            DiagKind::Keymap(KeymapProblem::UserChordPrefix)
        )]
    );
    h.paste("a\n");
    h.keys("ctrl+home ctrl+alt+j");
    h.text("x");
    assert_eq!(h.doc_text(), "a\na\n", "チョードが勝つ (02 §7.2 K3)");
}

// verifies: AC-FR-705-1
#[test]
fn dump_keymap_の解決にはワークスペース層を含めない() {
    let mut h = Harness::new();
    workspace(
        &mut h,
        "[keys.editor]\n\"ctrl+alt+j\" = \"edit.duplicate_line\"\n",
    );
    let (km, _) = resolve_keymap(&KeymapFile::default());
    let listed = km.listed_bindings();
    assert!(listed.iter().all(|(layer, _)| *layer != "workspace"));
    let ws = h.editor.keymap().listed_bindings();
    let (_, added) = ws
        .iter()
        .find(|(layer, _)| *layer == "workspace")
        .expect("エディタにはワークスペース層がある");
    assert!(
        listed.iter().all(|(_, b)| b.sequence != added.sequence),
        "ワークスペースの割当は出さない"
    );
}

/// ヘルプのキー一覧 (`help.keys`) の (タイトル, キーの列)。
fn help_entries(h: &mut Harness) -> Vec<(String, String)> {
    h.cmd("help.keys");
    let f = h.frame();
    let eve_view::OverlayModel::Help { lines, .. } = &f.overlays[0] else {
        panic!("ヘルプではない")
    };
    lines
        .iter()
        .filter(|l| l.kind == eve_view::HelpLineKind::Entry)
        .filter_map(|l| Some((cell_text(&l.text), cell_text(l.key.as_ref()?))))
        .collect()
}

// verifies: AC-FR-705-1
#[test]
fn ヘルプのキー一覧はワークスペース層の割当の由来を_workspace_と示す() {
    let mut h = Harness::new();
    workspace(
        &mut h,
        "[keys.editor]\n\"ctrl+alt+j\" = \"edit.duplicate_line\"\n",
    );
    let entries = help_entries(&mut h);
    let key_of = |title: &str| {
        entries
            .iter()
            .find(|(t, _)| t == title)
            .map(|(_, k)| k.clone())
            .unwrap_or_else(|| panic!("{title}: {entries:?}"))
    };
    let dup = key_of("行を複製");
    assert_eq!(
        dup, "Ctrl+Alt+J (workspace), Ctrl+K D",
        "下の層の割当には付けない"
    );
    let marked = entries
        .iter()
        .filter(|(_, k)| k.contains("(workspace)"))
        .count();
    assert_eq!(marked, 1, "{entries:?}");
}
