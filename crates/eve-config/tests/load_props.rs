//! 設定・キーマップ・テーマの読込の性質 (docs/review/v0.3-findings.md の STR-T8)。任意の文字列と、TOML の断片を並べた文字列で、
//! 読込が panic せず、診断の位置 (行・桁) が本文の範囲に収まることを確かめる。

use eve_config::theme_file::load_theme;
use eve_config::{Diagnostic, load_keymap_str, load_str};
use proptest::prelude::*;

/// 診断の位置が `text` の範囲に収まる (位置が無いときは 0:0)。
fn in_range(text: &str, d: &Diagnostic) -> bool {
    if d.line == 0 {
        return d.column == 0;
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let Some(l) = lines.get(d.line as usize - 1) else {
        return false;
    };
    d.column >= 1 && d.column as usize <= l.chars().count() + 1
}

fn check_all(text: &str) {
    let loaded = load_str(text);
    for d in &loaded.diagnostics {
        assert!(in_range(text, d), "設定: {d:?}\n{text:?}");
    }
    let km = load_keymap_str(text);
    for d in &km.diagnostics {
        assert!(in_range(text, d), "キーマップ: {d:?}\n{text:?}");
    }
    let read =
        |n: &str| -> Result<Option<String>, String> { Ok((n == "t").then(|| text.to_owned())) };
    let (_, reports) = load_theme("t", &read);
    for (file, diags) in &reports {
        for d in diags {
            assert!(in_range(text, d), "テーマ {file}: {d:?}\n{text:?}");
        }
    }
}

/// TOML の断片 (節・キー・値・壊れた書き方) を並べて、読込の奥まで届く入力を作る。
fn toml_like() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        Just("[editor]\n".to_owned()),
        Just("[file]\n".to_owned()),
        Just("[view]\n".to_owned()),
        Just("[theme]\n".to_owned()),
        Just("[ui]\n".to_owned()),
        Just("[keys.editor]\n".to_owned()),
        Just("[keys.nowhere]\n".to_owned()),
        Just("[meta]\ninherits = \"t\"\n".to_owned()),
        Just("[syntax]\n".to_owned()),
        Just("tab_width = 4\n".to_owned()),
        Just("tab_width = \"4\"\n".to_owned()),
        Just("tab_width = 99999999999999999999\n".to_owned()),
        Just("backspace_unit = \"word\"\n".to_owned()),
        Just("selection = { fg = \"#123\", bg = \"nope\" }\n".to_owned()),
        Just("\"ctrl+s\" = \"file.save\"\n".to_owned()),
        Just("\"ctrl+\" = { command = \"x\", when = \"a &&\" }\n".to_owned()),
        Just("\"f5\" = [ \"a.b\", { command = \"c.d\" } ]\n".to_owned()),
        Just("[editor\n".to_owned()),
        Just("= 1\n".to_owned()),
        Just("\"\u{3042}\u{1F600}\" = \"\u{65E5}\u{672C}\"\n".to_owned()),
        Just("x = '''\n".to_owned()),
        Just("\r\n".to_owned()),
        "[ -~]{0,12}\n",
    ];
    proptest::collection::vec(piece, 0..24).prop_map(|v| v.concat())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn 任意の文字列で読込が_panic_せず診断の位置は本文の中(text in "\\PC{0,200}") {
        check_all(&text);
    }

    #[test]
    fn toml_の断片を並べた文字列でも同じ(text in toml_like()) {
        check_all(&text);
    }
}
