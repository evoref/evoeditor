//! 空白の可視化の設定 (docs/03 §13.2、07 §5.3 の `view.show_whitespace`、01 FR-508)。

use eve_config::{Config, Severity, ShowWhitespace, load_str};
use eve_testkit::Harness;

fn row0(h: &mut Harness) -> String {
    let f = h.frame();
    f.panes[0].rows[0]
        .spans
        .iter()
        .map(|s| s.text.as_str())
        .collect::<String>()
}

#[test]
fn 既定は全角スペースとタブを記号で示す() {
    let mut h = Harness::new().with_size(40, 6);
    h.paste("a\tb\u{3000}c");
    assert_eq!(row0(&mut h).trim_end(), "a>  b[]c");
}

#[test]
fn off_では記号を出さず_幅は変えない() {
    let mut h = Harness::new().with_size(40, 6);
    h.editor.state_mut().set_config(Config {
        show_whitespace: ShowWhitespace::Off,
        ..Config::default()
    });
    h.paste("a\tb\u{3000}c");
    assert_eq!(
        row0(&mut h).trim_end(),
        "a   b  c",
        "タブは 4 桁目まで、全角スペースは 2 セルの空白"
    );
    // カーソルの位置 (表示列) も同じ
    h.keys("end");
    let f = h.frame();
    let gw = f.panes[0].gutter_width;
    assert_eq!(f.primary_cursor.unwrap().pos.col - gw, 8);
    // 文書の内容は変わらない
    assert_eq!(h.doc_text(), "a\tb\u{3000}c");
}

#[test]
fn 設定の値を読み_後の版の値は未対応の情報診断にする() {
    let l = load_str("[view]\nshow_whitespace = \"off\"\n");
    assert!(l.diagnostics.is_empty(), "{:?}", l.diagnostics);
    assert_eq!(l.config.show_whitespace, ShowWhitespace::Off);
    for v in ["ideographic", "trailing", "all"] {
        let l = load_str(&format!("[view]\nshow_whitespace = \"{v}\"\n"));
        assert_eq!(l.config.show_whitespace, ShowWhitespace::Minimal, "{v}");
        assert_eq!(l.diagnostics.len(), 1, "{v}");
        assert_eq!(l.diagnostics[0].severity, Severity::Info, "{v}");
    }
    let l = load_str("[view]\nshow_whitespace = \"none\"\n");
    assert_eq!(l.config.show_whitespace, ShowWhitespace::Minimal);
    assert_eq!(l.diagnostics[0].severity, Severity::Warning);
}
