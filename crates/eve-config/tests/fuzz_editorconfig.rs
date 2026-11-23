//! `fuzz_editorconfig` の本体を proptest で回す (docs/09 §3.3。Windows の CI では libFuzzer が使えないので、同じ関数をここで
//! 検査する。`fuzz_gitignore` と同じ形)。性質は `eve_config::editorconfig_fuzzing` の説明のとおり (パニックしない・照合の
//! 手数の上限・決定的・由来の位置・選択肢の展開・使わないセクションの影響なし)。件数は `PROPTEST_CASES` で増やせる (夜間は
//! 10,000)。

use eve_config::editorconfig_fuzzing::{EditorConfigCase, check, decode};
use proptest::prelude::*;

fn run(c: &EditorConfigCase) -> Result<(), TestCaseError> {
    check(c).map_err(TestCaseError::fail)
}

/// 本文に効く部品 (見出し・glob の記号・逃がし・数の範囲・キーと値・注釈・BOM・CR)。
const PIECES: &[&str] = &[
    "\n",
    "\r\n",
    "[",
    "]",
    "[*]",
    "[*.rs]",
    "[src/**]",
    "[/a/*]",
    "[{a,b}]",
    "[{1..5}]",
    "[{-99999999999..99999999999}]",
    "[!a]",
    "{",
    "}",
    ",",
    "..",
    "*",
    "**",
    "?",
    "\\",
    "/",
    "a",
    "b",
    "1",
    "-",
    "=",
    " ",
    "#",
    ";",
    "\u{feff}",
    "root = true",
    "indent_style = tab",
    "indent_style = space",
    "indent_size = 2",
    "indent_size = tab",
    "indent_size = 0",
    "tab_width = 8",
    "end_of_line = crlf",
    "charset = utf-8-bom",
    "charset = latin1",
    "indent_size = unset",
    "max_line_length = 80",
];

/// パスに使う部品 (`/` は区切り)。
const PATH_PIECES: &[&str] = &[
    "a", "b", "A", "src", "x.rs", "1", "12", "-3", "[", "]", "{", "}", ",", "*", "?", "\\", "/",
    "あ",
];

fn pieces(set: &'static [&'static str], max: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(set), 0..max).prop_map(|v| v.concat())
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(256),
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    // 任意のバイト列 (fuzz の入力と同じ読み替え)
    #[test]
    fn 任意の入力で性質を満たす(data in prop::collection::vec(any::<u8>(), 0..300)) {
        run(&decode(&data))?;
    }

    // 部品を組んだ本文とパス
    #[test]
    fn 部品を組んだ入力で性質を満たす(text in pieces(PIECES, 60), path in pieces(PATH_PIECES, 12)) {
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let path = if segments.is_empty() { "a".to_owned() } else { segments.join("/") };
        run(&EditorConfigCase { text, path })?;
    }
}

/// 後戻りの多い glob・深い入れ子・大きな数の範囲と長いパスでも、手数の上限に収まる (時間が爆発しない)。
#[test]
fn 星の多い_glob_と長いパスでも手数は上限に収まる() {
    let text = format!(
        "[{}]\nindent_size = 2\n[{}]\nindent_size = 3\n[{}x]\nindent_size = 4\n[{}]\nindent_size = 5\n",
        "*a".repeat(500),
        "{a,".repeat(8) + &"}".repeat(8),
        "**".repeat(200),
        "{-99999999999..99999999999}".repeat(20),
    );
    let path = format!("{}/{}", "a".repeat(2000), "1".repeat(1000));
    check(&EditorConfigCase { text, path }).unwrap();
}
