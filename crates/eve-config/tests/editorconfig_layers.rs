//! `.editorconfig` の重ね順の性質 (docs/07 §10.3 の EC-5、§10.1 の `unset`・誤った値・写せない `charset`)。任意の複数の
//! ファイル・セクション・プロパティの並びについて、`resolve_editorconfig` の結果が参照モデル「遠いファイルから近いファイルの
//! 順に、各ファイルの中は書いた順に、当たるセクションの値を後勝ちで重ね、`unset`・誤った値・写せない値は取り消す」と
//! 一致する。件数は `PROPTEST_CASES` で増やせる。

use eve_config::{
    DefaultEncoding, EcWidth, EditorConfigFile, EditorConfigSource, parse_editorconfig,
    resolve_editorconfig,
};
use proptest::prelude::*;

/// `indent_size` の値の候補 (正しい値・`tab`・`unset`・範囲の外・知らない値)。
const SIZES: &[&str] = &["1", "2", "16", "tab", "TAB", "unset", "0", "17", "x"];
/// `charset` の値の候補 (写す値・写せない値・`unset`・知らない値)。
const CHARSETS: &[&str] = &[
    "utf-8",
    "utf-8-bom",
    "UTF-16LE",
    "utf-16be",
    "latin1",
    "unset",
    "bogus",
];

/// 1 つのセクション: 当たるか、(`indent_size` なら true, 値) の並び。
type Section = (bool, Vec<(bool, &'static str)>);

fn section() -> impl Strategy<Value = Section> {
    let prop = prop_oneof![
        prop::sample::select(SIZES).prop_map(|v| (true, v)),
        prop::sample::select(CHARSETS).prop_map(|v| (false, v)),
    ];
    (any::<bool>(), prop::collection::vec(prop, 0..4))
}

fn text_of(sections: &[Section]) -> String {
    let mut t = String::new();
    for (hit, props) in sections {
        t.push_str(if *hit { "[*]\n" } else { "[nomatch]\n" });
        for (size, v) in props {
            let key = if *size { "indent_size" } else { "charset" };
            t.push_str(&format!("{key} = {v}\n"));
        }
    }
    t
}

fn model_size(v: &str) -> Option<EcWidth> {
    match v.to_ascii_lowercase().as_str() {
        "tab" => Some(EcWidth::Tab),
        n => n
            .parse::<u8>()
            .ok()
            .filter(|n| (1..=16).contains(n))
            .map(EcWidth::Width),
    }
}

fn model_charset(v: &str) -> Option<DefaultEncoding> {
    match v.to_ascii_lowercase().as_str() {
        "utf-8" => Some(DefaultEncoding::Utf8),
        "utf-8-bom" => Some(DefaultEncoding::Utf8Bom),
        "utf-16le" => Some(DefaultEncoding::Utf16Le),
        "utf-16be" => Some(DefaultEncoding::Utf16Be),
        _ => None,
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(256),
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    // 遠い順のファイルの並び (各ファイルはセクションの並び)
    #[test]
    fn 重ね順と取り消しが参照モデルと一致する(
        files in prop::collection::vec(prop::collection::vec(section(), 0..4), 1..5)
    ) {
        let names: Vec<String> = (0..files.len()).map(|i| format!("/{i}/.editorconfig")).collect();
        let parsed: Vec<EditorConfigFile> = files.iter().map(|f| parse_editorconfig(&text_of(f))).collect();
        let sources: Vec<EditorConfigSource<'_>> = names
            .iter()
            .zip(&parsed)
            .map(|(n, f)| (n.as_str(), f, "x"))
            .collect();
        let got = resolve_editorconfig(&sources);

        // 参照モデル: (値, 書いたファイルの番号)
        let mut size: Option<(EcWidth, usize)> = None;
        let mut charset: Option<(DefaultEncoding, usize)> = None;
        for (i, f) in files.iter().enumerate() {
            for (hit, props) in f {
                if !hit {
                    continue;
                }
                for (is_size, v) in props {
                    if *is_size {
                        size = model_size(v).map(|w| (w, i));
                    } else {
                        charset = model_charset(v).map(|e| (e, i));
                    }
                }
            }
        }
        let got_size = got.indent_width.as_ref().map(|(w, o)| (*w, o.file.clone()));
        prop_assert_eq!(got_size, size.map(|(w, i)| (w, names[i].clone())));
        let got_charset = got.encoding.as_ref().map(|(e, o)| (*e, o.file.clone()));
        prop_assert_eq!(got_charset, charset.map(|(e, i)| (e, names[i].clone())));
        // `tab_width` を書かなければ、整数の `indent_size` の値 (`tab` と取り消しでは無い)
        let want_tab = match size {
            Some((EcWidth::Width(n), _)) => Some(n),
            _ => None,
        };
        prop_assert_eq!(got.tab_width.as_ref().map(|(n, _)| *n), want_tab);
    }
}
