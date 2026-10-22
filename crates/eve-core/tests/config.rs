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
