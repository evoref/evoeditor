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

/// パスに使う部品 (`/` は区切り)。CR と BOM は、本文の行末の CR・行頭の BOM の扱いを名前の側からも突けるように入れる。
const PATH_PIECES: &[&str] = &[
    "a", "b", "A", "B", "z", ".", "-", "_", "あ", "é", "[", "]", "!", "#", " ", "*", "?", "\\",
    "/", "log", "LOG", "build", "Build", "\r", "\u{feff}",
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

    // `**/` + 名前の行 (速い経路) と、その名前を含むパス。速い経路と字句の模擬の結果が等しいことを含めて確かめる
    #[test]
    fn 二重の星と名前の行で性質を満たす(
        names in prop::collection::vec(pieces(PATH_PIECES, 3), 1..4),
        path in pieces(PATH_PIECES, 12),
        dir in any::<bool>(),
        fold in any::<bool>(),
        depth in 0usize..4,
    ) {
        let text: String = names.iter().map(|n| format!("**/{n}\n")).collect();
        let mut segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if let Some(n) = names.first().filter(|n| !n.is_empty() && !n.contains('/')) {
            segments.push(n);
        }
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

/// 夜間の fuzz の反例 (2026-11-22・23、Issue #612 / #623) を縮めたもの。ワイルドカードの無い行の検査が、処理した後の行を
/// `parse` に通し直して、行末の CR を 2 回落とし、行頭の BOM の並びをファイルの先頭の BOM として落としていた (検査の誤り)。
#[test]
fn 行末の_cr_と行頭の_bom_は行の処理で一度だけ扱う() {
    let cases: &[(&[u8], &[u8])] = &[
        // CR が 2 つ: パターンは `"\r"` で、名前 `"\r"` に当たる
        (b"\r\r", b"\r"),
        // CR が 3 つ: パターンは `"\r\r"` で、名前 `"\r"` に当たらない
        (b"\r\r\r", b"\r"),
        // 行末が CR でないので落とさず、NUL で切ってパターンは `"\r"`
        (b"\r\0\xD8", b"\r"),
        (b"\xFF\r\0", b"\xFF"),
        // 2 行目の行頭の BOM の並びは本文の一部 (名前 `a` に当たらない)
        (b"x\n\xEF\xBB\xBFa", b"a"),
        (b"x\n\xEF\xBB\xBFa", b"\xEF\xBB\xBFa"),
        // ファイルの先頭の BOM は 1 つだけ落とす
        (b"\xEF\xBB\xBF\xEF\xBB\xBFa", b"a"),
        (b"\xEF\xBB\xBF\xEF\xBB\xBFa", b"\xEF\xBB\xBFa"),
    ];
    for (text, path) in cases {
        for kind in [PathKind::File, PathKind::Dir] {
            for fold in [CaseFold::Sensitive, CaseFold::Ascii] {
                check(&GitignoreCase {
                    text: text.to_vec(),
                    path: path.to_vec(),
                    kind,
                    fold,
                    base_depth: 0,
                })
                .unwrap();
            }
        }
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
