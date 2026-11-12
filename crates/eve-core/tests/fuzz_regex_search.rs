//! `fuzz_regex_search` の本体を proptest で回す (docs/09 §3.3、09 §3.2 の検索の拡張の行。Windows の CI では libFuzzer が
//! 使えないので、同じ関数をここで検査する)。行の窓 (核を 1〜40 バイトにして窓の境目を多く作る) と全文の連結の経路が、全文を
//! 1 本に連結した参照と、一致・再開の位置からの続き・前 / 次の一致・置換の展開で同じになり、2 つ目の参照 (CRLF と CR を LF に
//! そろえた文書) とも同じになる。件数は `PROPTEST_CASES` で増やせる (夜間は 10,000)。

use eve_core::search_fuzzing::{Checked, RegexCase, check, decode};
use eve_core::{Case, SEARCH_CONCAT_MAX_BYTES};
use proptest::prelude::*;

fn run(c: &RegexCase) -> Result<Checked, TestCaseError> {
    check(c).map_err(TestCaseError::fail)
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(256),
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    // 任意のバイト列 (fuzz の入力と同じ読み替え)
    #[test]
    fn 任意の入力で参照と一致する(data in prop::collection::vec(any::<u8>(), 0..160)) {
        run(&decode(&data))?;
    }

    // 部品を多く含む入力 (構文木の節を多く作るバイトを重み付けする)
    #[test]
    fn 複合の検索語でも参照と一致する(
        head in prop::collection::vec(any::<u8>(), 4..5),
        body in prop::collection::vec(prop_oneof![3 => 0u8..112, 2 => 112u8..=255, 1 => 248u8..=255], 0..200),
    ) {
        let mut data = head;
        data.extend(body);
        run(&decode(&data))?;
    }
}

fn case(pattern: &str, doc: &str) -> RegexCase {
    RegexCase {
        pattern: pattern.to_owned(),
        ref2: !pattern.contains(r"\r") && !pattern.contains("(?s)") && !pattern.contains("-R"),
        doc: doc.to_owned(),
        case: Case::Sensitive,
        whole_word: false,
        sugar: true,
        scope: None,
        core_bytes: 1,
        concat_max: SEARCH_CONCAT_MAX_BYTES,
        splits: vec![3, 7],
        clock_step: 0,
    }
}

/// 固定の例: 行をまたぐ一致・CRLF・空の一致・書記素の境界・単語単位・選択範囲・上限なし・`\A` / `\z`・`(?-R)` の `^` / `$`。核の大きさを 1〜40 に
/// 変えても、どれも参照と同じ。
#[test]
fn 固定の例は核の大きさに依らず参照と一致する() {
    let pats = [
        r"a\na",
        r"\n\n",
        r"\n{2,}",
        r"[\s\n]+",
        r"(?s)a.*b",
        r"(?s).",
        r"\Aa",
        r"b\z",
        r"(?-m)^.",
        r"(?-m).$",
        r"^$",
        r"x*",
        r"e",
        r"\b\w+\n\w+\b",
        r"(\w+)\n(?P<g1>\w+)",
        r"[^\n]*\n",
        r"\r\n",
        r"\r",
        r"(?:\r\n)+",
        r"\n\s*\n",
        // `(?-R)` の `^` / `$` (CRLF を行の終端と見ない): 行の経路では文書全体の意味にならないので、k = 0 でも窓で探す
        r"a(?-R:$)",
        r"(?-R:^)b",
        r"(?-R)^b",
        r"(?-R:$)\n",
    ];
    let docs = [
        "a\na\na",
        "a\r\nb",
        "a\r\n\r\nb",
        "a\n\nb\r\rc\r\n\r\n\rd",
        "e\u{301}\ne\nxe",
        "ab\r\ncd ef\rgh\n",
        "",
        "\n",
        "\r\n",
        "😀\u{200D}😀\na😀",
        "a\rb\r\nb\na",
    ];
    let mut compared = 0;
    for p in pats {
        for d in docs {
            for core in [1, 2, 3, 5, 8, 13, 40, 1 << 20] {
                for ww in [false, true] {
                    let mut c = case(p, d);
                    c.core_bytes = core;
                    c.whole_word = ww;
                    match check(&c) {
                        Ok(Checked::Compared) => compared += 1,
                        Ok(Checked::Skipped(why)) => panic!("{p:?} / {d:?}: 比べなかった ({why})"),
                        Err(e) => panic!("{e}"),
                    }
                }
            }
        }
    }
    assert_eq!(compared, 24 * 11 * 8 * 2);
}
