//! 設定の適用 (docs/07 §5.0): 表示幅・インデント・新規ファイルの既定値・起動時の診断の表示。

use eve_config::{Config, DefaultEncoding, DefaultLineEnding, IndentUnit, load_str};
use eve_core::Encoding;
use eve_testkit::Harness;

fn with(h: &mut Harness, f: impl FnOnce(&mut Config)) {
    let mut c = Config::default();
    f(&mut c);
    h.editor.state_mut().set_config(c);
}

#[test]
fn tab_キーは_indent_width_の空白を入れる() {
    let mut h = Harness::new();
    with(&mut h, |c| c.indent_width = 2);
    h.keys("tab");
    assert_eq!(h.doc_text(), "  ");
    h.text("a");
    h.keys("tab");
    assert_eq!(h.doc_text(), "  a ", "次のインデント停止位置 (幅 2) まで");
}

#[test]
fn indent_unit_が_tab_ならタブ文字を入れる() {
    let mut h = Harness::new();
    with(&mut h, |c| c.indent_unit = IndentUnit::Tab);
    h.keys("tab");
    assert_eq!(h.doc_text(), "\t");
}

#[test]
fn shift_tab_は_indent_width_ぶん戻す() {
    let mut h = Harness::new();
    with(&mut h, |c| c.indent_width = 2);
    h.text("      x");
    h.keys("home shift+tab");
    assert_eq!(h.doc_text(), "    x");
}

// verifies: AC-FR-701-1
#[test]
fn タブ文字の表示幅は_tab_width() {
    let mut h = Harness::new();
    h.text("\tx");
    let w4 = h.frame_text();
    with(&mut h, |c| c.tab_width = 8);
    let w8 = h.frame_text();
    assert_ne!(w4, w8, "タブの幅が変わると描画も変わる");
}

#[test]
fn 新規文書は_file_default_で作る() {
    let mut h = Harness::new();
    with(&mut h, |c| {
        c.default_line_ending = DefaultLineEnding::Crlf;
        c.default_encoding = DefaultEncoding::ShiftJis;
    });
    h.cmd("file.new");
    h.text("a");
    h.keys("enter");
    h.text("b");
    assert_eq!(h.doc_text(), "a\r\nb");
    let id = h.doc_id();
    let doc = h.editor.state().document(id).unwrap();
    assert_eq!(doc.format.encoding, Encoding::ShiftJis);
}

// verifies: AC-FR-703-1
#[test]
fn 診断が_1_件ならその内容を_2_件以上なら要約を出す() {
    let mut h = Harness::new();
    let l = load_str("[editor]\ntab_width = \"4\"\n");
    h.editor
        .state_mut()
        .report_config("config.toml", &l.diagnostics);
    let t = h.frame_text();
    assert!(t.contains("config.toml:2"), "{t}");
    assert!(t.contains("editor.tab_width"), "{t}");

    let l = load_str("[editor]\ntab_width = 0\nindent_width = 99\n");
    assert_eq!(l.diagnostics.len(), 2);
    h.editor
        .state_mut()
        .report_config("config.toml", &l.diagnostics);
    let t = h.frame_text();
    assert!(t.contains("2 件の問題"), "{t}");
}

#[test]
fn 診断が無ければ何も出さない() {
    let mut h = Harness::new();
    h.clear_log();
    h.editor.state_mut().report_config("config.toml", &[]);
    assert!(h.messages().is_empty());
}

#[test]
fn system_は_os_のヒントの文字コードで新規文書を作る() {
    let mut h = Harness::new();
    with(&mut h, |c| c.default_encoding = DefaultEncoding::System);
    h.editor
        .state_mut()
        .set_system_encoding(Some(Encoding::ShiftJis));
    h.cmd("file.new");
    let id = h.doc_id();
    assert_eq!(
        h.editor.state().document(id).unwrap().format.encoding,
        Encoding::ShiftJis
    );
    // ヒントが無ければ UTF-8
    h.editor.state_mut().set_system_encoding(None);
    h.cmd("file.new");
    let id = h.doc_id();
    assert_eq!(
        h.editor.state().document(id).unwrap().format.encoding,
        Encoding::Utf8
    );
}

#[test]
fn 空の既存ファイルと改行のないファイルの改行は設定に従う() {
    for (bytes, name) in [(&b""[..], "空"), (&b"abc"[..], "改行なし")] {
        let mut h = Harness::new().with_file("/w/e.txt", bytes);
        with(&mut h, |c| c.default_line_ending = DefaultLineEnding::Crlf);
        h.open("/w/e.txt");
        h.run_jobs();
        h.keys("ctrl+end");
        h.keys("enter");
        h.text("x");
        assert!(
            h.doc_text().ends_with("\r\nx") || h.doc_text().ends_with("\nx"),
            "{name}"
        );
        let id = h.doc_id();
        assert_eq!(
            h.editor.state().document(id).unwrap().format.default_eol,
            eve_text::LineEnding::Crlf,
            "{name}"
        );
    }
    // 改行のあるファイルは多数派を維持する
    let mut h = Harness::new().with_file("/w/l.txt", b"a\nb\n");
    with(&mut h, |c| c.default_line_ending = DefaultLineEnding::Crlf);
    h.open("/w/l.txt");
    h.run_jobs();
    let id = h.doc_id();
    assert_eq!(
        h.editor.state().document(id).unwrap().format.default_eol,
        eve_text::LineEnding::Lf
    );
}

// verifies: AC-FR-701-3
#[test]
fn 未対応の情報診断は画面に出さない() {
    let mut h = Harness::new();
    let l = load_str("[editor]\nauto_indent = false\n[search]\ncase = \"smart\"\n");
    assert_eq!(l.diagnostics.len(), 2);
    h.clear_log();
    h.editor
        .state_mut()
        .report_config("config.toml", &l.diagnostics);
    assert!(h.messages().is_empty());
    // 警告が 1 件混じれば、その 1 件だけを数える
    let l = load_str("[editor]\nauto_indent = false\ntab_width = 0\n");
    h.editor
        .state_mut()
        .report_config("config.toml", &l.diagnostics);
    let t = h.frame_text();
    assert!(t.contains("config.toml:3"), "{t}");
}

// verifies: AC-NFR-603-8
#[test]
fn ui_locale_は未対応の情報診断になり_画面に出ない() {
    let mut h = Harness::new();
    let l = load_str("[ui]\nlocale = \"en\"\n");
    assert_eq!(l.diagnostics.len(), 1);
    assert!(
        l.diagnostics[0].key.starts_with("ui"),
        "{:?}",
        l.diagnostics[0]
    );
    assert!(matches!(
        l.diagnostics[0].kind,
        eve_config::DiagKind::Unsupported
    ));
    h.clear_log();
    h.editor
        .state_mut()
        .report_config("config.toml", &l.diagnostics);
    assert!(h.messages().is_empty());
}
