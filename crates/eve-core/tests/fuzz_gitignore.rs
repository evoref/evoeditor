//! `fuzz_gitignore` の本体を proptest で回す (docs/09 §3.3。Windows の CI では libFuzzer が使えないので、同じ関数をここで
//! 検査する)。性質は `eve_core::gitignore_fuzzing` の説明のとおり (パニックしない・手数の上限・決定的・ワイルドカードの無い行は
//! 完全一致・文字の集合の無い本文の畳み方)。件数は `PROPTEST_CASES` で増やせる (夜間は 10,000)。

use eve_core::gitignore_fuzzing::{GitignoreCase, check, decode};
use eve_core::{CaseFold, PathKind};
use proptest::prelude::*;

fn run(c: &GitignoreCase) -> Result<(), TestCaseError> {
    check(c).map_err(TestCaseError::fail)
}

/// パターンに効く部品 (ワイルドカード・集合・POSIX の集合・エスケープ・区切り・否定・注釈・空白・CR・BOM)。
const PIECES: &[&str] = &[
    "a",
    "b",
    "A",
    "B",
    "z",
    ".",
    "-",
    "_",
    "あ",
    "é",
    "*",
    "**",
    "?",
    "[",
    "]",
    "[!",
    "[^",
    "[a-c]",
    "[A-Z]",
    "[[:alpha:]]",
    "[[:digit:]]",
    "[[:upper:]]",
    "[[:lower:]]",
    "[[:space:]]",
    "[[:punct:]]",
    "[[:bogus:]]",
    "[[:",
    ":]",
    "\\",
    "\\*",
    "\\ ",
    "/",
    "**/",
    "/**",
    "/**/",
    "!",
    "#",
    " ",
    "\t",
    "\r",
    "\n",
    "\u{feff}",
];

/// パスに使う部品 (`/` は区切り)。
const PATH_PIECES: &[&str] = &[
    "a", "b", "A", "B", "z", ".", "-", "_", "あ", "é", "[", "]", "!", "#", " ", "*", "?", "\\",
    "/", "log", "LOG", "build", "Build",
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
    fn 任意の入力で性質を満たす(data in prop::collection::vec(any::<u8>(), 0..200)) {
        run(&decode(&data))?;
    }

    // 部品を組んだ本文とパス
    #[test]
    fn 部品を組んだ入力で性質を満たす(
        text in pieces(PIECES, 40),
        path in pieces(PATH_PIECES, 12),
        dir in any::<bool>(),
        fold in any::<bool>(),
        depth in 0usize..4,
    ) {
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let path = if segments.is_empty() { "a".to_owned() } else { segments.join("/") };
        let slashes = path.matches('/').count();
        run(&GitignoreCase {
            text: text.into_bytes(),
            path: path.into_bytes(),
            kind: if dir { PathKind::Dir } else { PathKind::File },
            fold: if fold { CaseFold::Ascii } else { CaseFold::Sensitive },
            base_depth: depth % (slashes + 1),
        })?;
    }
}

/// 後戻りの多いパターンと長いパスでも、手数の上限に収まる (時間が爆発しない)。
#[test]
fn 星の多いパターンでも手数は上限に収まる() {
    let text = format!(
        "{}\n**/{}\n{}/**/x\n",
        "*a".repeat(60),
        "a*".repeat(60),
        "*".repeat(100)
    );
    let path = format!("{}/{}", "a".repeat(2000), "a".repeat(1000));
    for fold in [CaseFold::Sensitive, CaseFold::Ascii] {
        check(&GitignoreCase {
            text: text.clone().into_bytes(),
            path: path.clone().into_bytes(),
            kind: PathKind::File,
            fold,
            base_depth: 0,
        })
        .unwrap();
    }
}
