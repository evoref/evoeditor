//! 折り返しの設定キーと `view.toggle_wrap` (docs/07 §5.3・§11.6、04 §7.2)。

use eve_config::{Config, WrapIndent, WrapMode as CfgMode, load_str};
use eve_core::WrapMode;
use eve_testkit::Harness;

fn view_wrap(h: &Harness) -> eve_core::WrapSettings {
    let w = h.editor.state().first_window().unwrap();
    let v = h.editor.state().window_view(w).unwrap();
    h.editor.state().view(v).unwrap().wrap
}

#[test]
fn 設定キーを読む() {
    let l = load_str("[view]\nwrap = \"word\"\nwrap_indent = \"none\"\nwrap_kinsoku = false\n");
    assert!(l.diagnostics.is_empty(), "{:?}", l.diagnostics);
    assert_eq!(l.config.wrap, CfgMode::Word);
    assert_eq!(l.config.wrap_indent, WrapIndent::None);
    assert!(!l.config.wrap_kinsoku);
    let d = Config::default();
    assert_eq!(
        (d.wrap, d.wrap_indent, d.wrap_kinsoku),
        (CfgMode::Off, WrapIndent::Same, true)
    );
}

#[test]
fn 不正な値は警告して既定値() {
    let l = load_str("[view]\nwrap = \"hard\"\n");
    assert_eq!(l.config.wrap, CfgMode::Off);
    assert_eq!(l.diagnostics.len(), 1);
}

#[test]
fn 設定は_view_に写り_toggle_で切り替わる() {
    let mut h = Harness::new().with_size(20, 8);
    assert!(!view_wrap(&h).is_on());
    h.editor.state_mut().set_config(Config {
        wrap: CfgMode::Soft,
        wrap_indent: WrapIndent::None,
        ..Config::default()
    });
    let s = view_wrap(&h);
    assert_eq!(s.mode, WrapMode::Soft);
    assert!(!s.indent_same);

    h.keys("alt+z");
    assert!(!view_wrap(&h).is_on(), "ON → OFF");
    h.keys("alt+z");
    assert_eq!(view_wrap(&h).mode, WrapMode::Soft, "設定の方式で ON に戻る");
    h.keys("alt+z ctrl+k o");
    assert!(view_wrap(&h).is_on(), "Ctrl+K O でも切り替わる");
}

#[test]
fn 設定が_off_なら_toggle_は_word_で_on_にする() {
    let mut h = Harness::new().with_size(20, 8);
    h.cmd("view.toggle_wrap");
    assert_eq!(view_wrap(&h).mode, WrapMode::Word);
    let v = h.editor.state().context_value("view.wrapped");
    assert!(matches!(v, Some(eve_core::CtxValue::Bool(true))), "{v:?}");
}
