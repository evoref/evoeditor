//! ユーザーの言語ファイルの読込と重ね合わせの性質 (docs/09 §3.4 の (1)、07 §17.4)。任意の文字列・バイト列と、言語ファイルの
//! 断片を並べた文字列で、読込と重ね合わせが panic せず、重ねた表のどの文言も検査 4 の文字 (制御文字・書式文字など) を
//! 含まず、診断の件数が上限に収まることを確かめる。

use eve_config::{
    Catalog, Diagnostic, LOCALE_MAX_DIAGNOSTICS, check_text, forbidden_char, load_locale_file,
    sanitize_untrusted,
};
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

fn check_all(lang: &str, bytes: &[u8]) {
    let base = Catalog::base_for(lang);
    let f = load_locale_file(lang, bytes, base, LOCALE_MAX_DIAGNOSTICS);
    assert!(f.diagnostics().len() <= LOCALE_MAX_DIAGNOSTICS + 1);
    if let Ok(text) = std::str::from_utf8(bytes) {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        for d in f.diagnostics() {
            assert!(in_range(text, d), "{d:?}\n{text:?}");
        }
    }
    let c = base.overlay(&f);
    assert_eq!(c.len(), base.len());
    for k in c.keys() {
        let v = c.get(k).unwrap_or_default();
        assert!(
            v.chars().all(|ch| forbidden_char(ch).is_none()),
            "{k} = {v:?}"
        );
    }
    // 診断に埋める文字列は無害化すると、使えない文字を含まず 64 文字に収まる
    for d in f.diagnostics() {
        for s in [&d.key, &d.value] {
            let safe = sanitize_untrusted(s);
            assert!(safe.chars().all(|ch| forbidden_char(ch).is_none()));
            assert!(safe.chars().count() <= 64);
        }
    }
}

/// 言語ファイルの断片 (同梱にあるキー・無いキー・予約のキー・壊れた書き方) を並べて、検査の奥まで届く入力を作る。
fn locale_like() -> impl Strategy<Value = String> {
    let value = prop_oneof![
        Just("保存".to_owned()),
        Just("{path} を保存しました".to_owned()),
        Just("{path}{path}".to_owned()),
        Just("[S] Save".to_owned()),
        Just("〔{tmp}".to_owned()),
        Just("\\u001b[2J".to_owned()),
        Just("\\u202e".to_owned()),
        Just("\\t".to_owned()),
        Just(String::new()),
        "\\PC{0,8}".prop_map(|s| s.replace(['"', '\\'], "")),
    ];
    let piece = prop_oneof![
        Just("[meta]\nlanguage = \"fr\"\n".to_owned()),
        Just("[info.save]\n".to_owned()),
        Just("[hint.app]\n".to_owned()),
        Just("[stderr.quit]\n".to_owned()),
        Just("[status]\n".to_owned()),
        Just("[ui.confirm.unsaved]\n".to_owned()),
        value.clone().prop_map(|v| format!("done = \"{v}\"\n")),
        value.clone().prop_map(|v| format!("quit = \"{v}\"\n")),
        value.clone().prop_map(|v| format!("save = \"{v}\"\n")),
        value.prop_map(|v| format!("\"save.failed.text\" = \"{v}\"\n")),
        Just("untitled = 1\n".to_owned()),
        Just("x = [[1]]\n".to_owned()),
        Just("[editor\n".to_owned()),
        Just("= 1\n".to_owned()),
        Just("\r\n".to_owned()),
        "[ -~]{0,12}\n",
    ];
    proptest::collection::vec(piece, 0..24).prop_map(|v| v.concat())
}

/// キーの長さの上限 (256 バイト) の近くの長さの段を並べた、表の見出しと葉のキー。
fn long_keys() -> impl Strategy<Value = String> {
    let seg = (0usize..=3, 245usize..=260).prop_map(|(kind, n)| match kind {
        0 => "a".repeat(n),
        1 => format!("\"{}\"", "あ".repeat(n / 3)),
        2 => "b".repeat(n % 7),
        _ => "\"\"".to_owned(),
    });
    let path = proptest::collection::vec(seg, 1..4).prop_map(|v| v.join("."));
    let piece = prop_oneof![
        path.clone().prop_map(|p| format!("[{p}]\n")),
        path.prop_map(|p| format!("{p} = \"x\"\n")),
        Just("z = \"x\"\n".to_owned()),
        Just("[hint.app]\nquit = \"x\"\n".to_owned()),
    ];
    proptest::collection::vec(piece, 1..8).prop_map(|v| v.concat())
}

/// 文字列・注釈・括弧が混ざった、字句の走査 (インラインの表と配列を拒む) の境目を突く入力。
fn lexical_like() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        Just("\"".to_owned()),
        Just("'".to_owned()),
        Just("\"\"\"".to_owned()),
        Just("'''".to_owned()),
        Just("\\".to_owned()),
        Just("#".to_owned()),
        Just("[".to_owned()),
        Just("]".to_owned()),
        Just("{".to_owned()),
        Just("}".to_owned()),
        Just("=".to_owned()),
        Just("\n".to_owned()),
        Just(" ".to_owned()),
        Just("a".to_owned()),
        Just("hint.app.quit".to_owned()),
    ];
    proptest::collection::vec(piece, 0..40).prop_map(|v| v.concat())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn 上限の近くの長さのキーでも_panic_しない(text in long_keys()) {
        check_all("ja", text.as_bytes());
        let f = load_locale_file("ja", text.as_bytes(), Catalog::base_for("ja"), usize::MAX);
        for d in f.diagnostics() {
            prop_assert!(d.key.len() <= 256, "{}", d.key.len());
        }
    }

    #[test]
    fn 字句の境目を突く入力でも_panic_しない(text in lexical_like()) {
        check_all("ja", text.as_bytes());
    }

    #[test]
    fn 任意の文字列で読込と重ね合わせが_panic_せず使えない文字が残らない(text in "\\PC{0,200}") {
        check_all("ja", text.as_bytes());
        check_all("fr", text.as_bytes());
    }

    #[test]
    fn 任意のバイト列でも同じ(bytes in proptest::collection::vec(any::<u8>(), 0..200)) {
        check_all("en", &bytes);
    }

    #[test]
    fn 言語ファイルの断片を並べた文字列でも同じ(text in locale_like()) {
        check_all("ja", text.as_bytes());
        check_all("fr", text.as_bytes());
    }

    #[test]
    fn 任意の文言と同梱の文言で検査が_panic_しない(text in "\\PC{0,64}", builtin in "\\PC{0,64}") {
        let _ = check_text("hint.app.quit", &text, &builtin);
        let _ = check_text("msg.a.text", &text, &builtin);
    }
}
