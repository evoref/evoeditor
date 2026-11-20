//! `cargo xtask gen-locales`: 同梱の言語の表 `crates/eve-config/src/locales_gen.rs` の生成と検査 (docs/07 §17.1)。
//!
//! `locales/<言語>.toml` (同梱の言語ファイル) を `eve_config::Catalog::parse` (実行時のユーザーの言語ファイルと
//! 同じ平らにする規則。`[meta]` を除き、入れ子の表を `.` でつなぐ) で読み、言語ごとに
//! キー → 文字列の静的な配列 (キーの昇順) を Rust のソースとして書く。起動時の TOML の解析をなくすため
//! (BM-102a、07 §17.1 の「サイズと起動時間」)。eve-config は build.rs を使わない (07 §4.1) ので、
//! 生成物はコミットし、`--check` (書き換えずに、生成した結果が今のファイルと一致するか) を `cargo xtask check` で回す。

use std::fmt::Write as _;
use std::path::Path;

use eve_config::Catalog;

use crate::util::{read_text, workspace_root};

/// 同梱の言語ファイルの置き場所。
const LOCALES: &str = "locales";

/// 生成物。
const OUT: &str = "crates/eve-config/src/locales_gen.rs";

/// 同梱の言語ファイル `(言語, 原文)` を言語の名前の順に集める。名前は `^[a-z]{2,3}\.toml$` (07 §17.4 の候補の形と同じ)。
fn sources(root: &Path) -> Result<Vec<(String, String)>, String> {
    let dir = root.join(LOCALES);
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let name = entry.file_name();
        let Some(lang) = name.to_str().and_then(|n| n.strip_suffix(".toml")) else {
            continue;
        };
        if !(2..=3).contains(&lang.len()) || !lang.bytes().all(|b| b.is_ascii_lowercase()) {
            return Err(format!(
                "{LOCALES}/{}: 同梱の言語ファイルの名前は `<小文字 2〜3 字>.toml` にしてください",
                name.to_string_lossy()
            ));
        }
        out.push((lang.to_owned(), read_text(&entry.path())?));
    }
    out.sort();
    if out.is_empty() {
        return Err(format!("{LOCALES}/ に言語ファイルがありません"));
    }
    Ok(out)
}

/// Rust の文字列リテラル。`\` `"` と制御文字、ソースに置くと警告・誤読のもとになる不可視の文字・書字方向の制御文字は
/// エスケープし、ほかはそのまま書く (Rust の版で結果が変わらないよう、`{:?}` の印字可能の判定は使わない)。
fn literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() || invisible(c) => {
                write!(out, "\\u{{{:04x}}}", u32::from(c)).expect("String への書込みは失敗しない");
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// ソースにそのまま置かない不可視の文字 (ソフトハイフン・ゼロ幅の文字・書字方向の制御・BOM など)。
fn invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00ad}'
            | '\u{061c}'
            | '\u{180e}'
            | '\u{200b}'..='\u{200f}'
            | '\u{2028}'..='\u{202e}'
            | '\u{2060}'..='\u{206f}'
            | '\u{feff}'
    )
}

/// 生成物の本文。
fn render(langs: &[(String, String)]) -> Result<String, String> {
    let mut out = String::from(
        "//! 同梱の言語の表 (**生成物。手で編集しない**)。\n\
         //!\n\
         //! `locales/*.toml` から `cargo xtask gen-locales` で生成する (docs/07 §17.1)。言語ファイルを直したら\n\
         //! 作り直してコミットする (CI の `cargo xtask check` が `cargo xtask gen-locales --check` で原文との一致を確かめる)。\n\
         //! 各表はキー → 文字列の、キーの昇順の配列 (`[meta]` を含まない。入れ子の表は `.` でつないだ平らなキー)。\n\
         \n\
         use std::borrow::Cow::Borrowed as B;\n\
         \n\
         use crate::catalog::Entry;\n",
    );
    for (lang, src) in langs {
        let cat = Catalog::parse(src).map_err(|e| format!("{LOCALES}/{lang}.toml: {e}"))?;
        let name = lang.to_ascii_uppercase();
        write!(
            out,
            "\n/// `locales/{lang}.toml` の文言 ({} 件)。\n#[rustfmt::skip]\npub(crate) static {name}: &[Entry] = &[\n",
            cat.len()
        )
        .expect("String への書込みは失敗しない");
        for key in cat.keys() {
            let text = cat.get(key).expect("keys() のキーは引ける");
            writeln!(out, "    (B({}), B({})),", literal(key), literal(text))
                .expect("String への書込みは失敗しない");
        }
        out.push_str("];\n");
    }
    Ok(out)
}

pub fn run(args: &[String]) -> Result<(), String> {
    let check = args.iter().any(|a| a == "--check");
    println!("▶ gen-locales{}", if check { " --check" } else { "" });
    let root = workspace_root();
    let langs = sources(&root)?;
    let want = render(&langs)?;
    let path = root.join(OUT);
    let names: Vec<&str> = langs.iter().map(|(l, _)| l.as_str()).collect();
    if check {
        let have = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        if have != want {
            return Err(format!(
                "{OUT} が {LOCALES}/*.toml と違います: `cargo xtask gen-locales` で作り直してコミットしてください"
            ));
        }
        println!("  OK ({})", names.join(", "));
    } else {
        std::fs::write(&path, want).map_err(|e| format!("{OUT}: {e}"))?;
        println!("  {OUT} を書きました ({})", names.join(", "));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn リテラルのエスケープ() {
        assert_eq!(literal("a"), "\"a\"");
        assert_eq!(literal("保存 {path}"), "\"保存 {path}\"");
        assert_eq!(literal("a\\b\"c\nd\te"), "\"a\\\\b\\\"c\\nd\\te\"");
        assert_eq!(
            literal("\u{1b}x\u{200b}\u{202e}"),
            "\"\\u{001b}x\\u{200b}\\u{202e}\""
        );
    }

    #[test]
    fn 表はキーの昇順で_meta_を除く() {
        let src = "[meta]\nlanguage = \"xx\"\n[b]\nz = \"2\"\n[a.c]\nd = \"1 \\\"q\\\"\"\n";
        let text = render(&[("xx".to_owned(), src.to_owned())]).unwrap();
        assert!(
            text.contains("pub(crate) static XX: &[Entry] = &[\n"),
            "{text}"
        );
        let a = text
            .find("(B(\"a.c.d\"), B(\"1 \\\"q\\\"\")),")
            .expect("a.c.d");
        let b = text.find("(B(\"b.z\"), B(\"2\")),").expect("b.z");
        assert!(a < b);
        assert!(!text.contains("language"), "{text}");
    }

    #[test]
    fn 読めない原文は失敗() {
        assert!(render(&[("xx".to_owned(), "a = 1\n".to_owned())]).is_err());
    }
}
