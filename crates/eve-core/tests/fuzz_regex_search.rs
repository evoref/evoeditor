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
        compat_fold: true,
        compat_ref: pattern.to_owned(),
        scope: None,
        core_bytes: 1,
        concat_max: SEARCH_CONCAT_MAX_BYTES,
        splits: vec![3, 7],
        clock_step: 0,
    }
}

/// 固定の例: 行をまたぐ一致・CRLF・空の一致・書記素の境界・単語単位・選択範囲・上限なし・`\A` / `\z`・`(?-R)` の `^` / `$`。核の大きさを 1〜40 に
/// 変えても、どれも参照と同じ。
// verifies: FR-402
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

/// 固定の例 (CP932 互換文字の同一視、03 §12.4.1): 互換文字の部品を含む検索語が、同一視 ON / OFF のどちらでも、核の大きさに
/// 依らず 1 つ目の参照 (全文の連結) と同じで、ON では 2 つ目の参照 (組をそろえた文書で `compat_reference` の形の検索語を
/// 同一視 OFF で探したもの) とも同じ。
// verifies: FR-401
#[test]
fn 互換文字の同一視の固定の例は参照と一致する() {
    use eve_core::search_fuzzing::compat_reference;
    // (検索語, 互換文字の部品) の組。2 つ目の参照の検索語は、部品を `compat_reference` の形に置き換えて作る
    let pats: [(&str, &[&str]); 8] = [
        ("〜", &["〜"]),
        ("a～", &["～"]),
        (r"[^～]\n", &["[^～]"]),
        (r"\p{Sm}+", &[r"\p{Sm}"]),
        (r"\P{Sm}\n\P{Sm}", &[r"\P{Sm}"]),
        (r"(〜)\n(?P<g1>―)", &["〜", "―"]),
        (r"\x{FF0D}|¢", &[r"\x{FF0D}", "¢"]),
        (r"\b−\w", &["−"]),
    ];
    let docs = [
        "a〜\n～b",
        "a～\r\n―b\n〜\r\n—",
        "−1 －2\n¢ ￠",
        "¥￥‾￣\\~\n～〜",
        "〜\u{301}～",
    ];
    let mut compared = 0;
    for (p, atoms) in pats {
        let mut r = p.to_owned();
        for a in atoms {
            r = r.replace(a, &compat_reference(a));
        }
        for d in docs {
            for core in [1, 2, 3, 7, 40, 1 << 20] {
                for (fold, ww) in [(true, false), (false, false), (true, true)] {
                    let mut c = case(p, d);
                    c.core_bytes = core;
                    c.compat_fold = fold;
                    c.compat_ref = r.clone();
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
    assert_eq!(compared, 8 * 5 * 6 * 3);
    // 部品の参照の形 (そろえた文書の上での意味)
    assert_eq!(compat_reference("～"), "(?:～|[〜])");
    assert_eq!(compat_reference("〜"), "〜");
    assert_eq!(compat_reference("[^～]"), "[^～〜]");
    assert_eq!(compat_reference(r"\p{Sm}"), r"(?:\p{Sm}|[〜‖])");
    assert_eq!(compat_reference(r"\P{Sm}"), r"[^\p{Sm}〜‖]");
    assert_eq!(compat_reference("¥"), "¥");
}

/// 回帰: 単語単位の `([\s\n])` で、行の途中の正本の位置 (一致の終端 8) から探すと、窓が行頭へ戻って次の行まで含み、
/// 上限 (21 バイト) を超えて「探さない」になっていた (正本の走査は探せる。末尾からの次 (折り返し) が `None` だった)。
/// proptest が見つけた入力 (バイト列) と、同じ文書で単語単位・糖衣・核の大きさ・上限を変えたもの。
// verifies: FR-402
#[test]
fn 行の途中の正本の位置からの次と前は窓の上限で探さないとしない() {
    // proptest が見つけた入力を decode した値を直接書く (M24 の (5) で入力の読み方が変わり、同じバイト列では同じ例に
    // ならないため)
    let mut c = case(r"([\s\n])", "é\u{212A}a1\t\ra\u{212A}σaé\r\n\r\n");
    (c.case, c.whole_word, c.sugar, c.core_bytes, c.concat_max) = (Case::Smart, true, false, 2, 21);
    c.splits = vec![];
    assert_eq!(check(&c), Ok(Checked::Compared));
    for core in [1, 2, 3, 5, 8, 13, 40] {
        for ww in [false, true] {
            for sugar in [false, true] {
                for concat_max in [12, 16, 21, 24] {
                    let mut v = c.clone();
                    (v.core_bytes, v.whole_word, v.sugar, v.concat_max) =
                        (core, ww, sugar, concat_max);
                    if let Err(e) = check(&v) {
                        panic!("{e}");
                    }
                }
            }
        }
    }
}
