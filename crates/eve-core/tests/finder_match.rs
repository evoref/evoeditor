//! ファインダの照合 `finder_match` (docs/03 §12.7.4 の「照らす対象」「採点」「件数」、§12.7.8 の L0 / L1 と proptest)。
//!
//! 採点の性質 (a)〜(c) と並びの規則を固定し、任意の検索語と候補で、一致が「語の部分列の AND」と同じで、並びが候補の順や
//! 塊の切り方に依らないことを確かめる。

use std::sync::Arc;

use eve_core::{CancelToken, Candidate, FINDER_TOP, finder_match};
use proptest::prelude::*;

fn chunks_of(rels: &[&str], size: usize) -> Vec<Arc<[Candidate]>> {
    rels.chunks(size.max(1))
        .map(|c| {
            c.iter()
                .map(|r| Candidate::new(r))
                .collect::<Vec<_>>()
                .into()
        })
        .collect()
}

/// 上位の相対パス (並びの順) と一致の件数。
fn ranked(rels: &[&str], query: &str) -> (Vec<String>, usize) {
    let chunks = chunks_of(rels, 3);
    let m =
        finder_match(&chunks, query, FINDER_TOP, &CancelToken::new()).expect("取り消していない");
    let top = m
        .top
        .iter()
        .map(|h| chunks[h.chunk as usize][h.index as usize].rel().to_owned())
        .collect();
    (top, m.matched)
}

// verifies: AC-FR-125-1
#[test]
fn 性質_a_ファイル名の中だけで一致する候補はディレクトリにまたがる一致より上() {
    // 点だけで比べると `a/b/x.rs` (境目の加点 2 つ) が `dir/xaxb.rs` (語の途中・間あり) より高い。ファイル名の中の語の数を先に比べる
    let (top, _) = ranked(&["a/b/x.rs", "dir/xaxb.rs"], "ab");
    assert_eq!(top, ["dir/xaxb.rs", "a/b/x.rs"]);
    let (top, _) = ranked(&["ma/in/x.rs", "src/main.rs"], "main");
    assert_eq!(top, ["src/main.rs", "ma/in/x.rs"]);
}

// verifies: AC-FR-125-1
#[test]
fn ディレクトリ名にも語がある候補はファイル名の中で当てて数える() {
    let (top, _) = ranked(
        &["lib/domain.rs", "m/a/i/n.rs", "src/main/java/Main.java"],
        "main",
    );
    assert_eq!(top[0], "src/main/java/Main.java", "{top:?}");
    let (top, _) = ranked(&["pkg/observer.go", "cmd/server/server.go"], "server");
    assert_eq!(top, ["cmd/server/server.go", "pkg/observer.go"]);
    // 強調はファイル名の中の窓 (書記素 14〜17 の `Main`)
    let chunks = chunks_of(&["src/main/java/Main.java"], 1);
    let m = finder_match(&chunks, "main", 10, &CancelToken::new()).expect("一致");
    assert_eq!(m.top[0].ranges.len(), 1);
    assert_eq!(m.top[0].ranges.first(), Some(&(14..18)));
}

#[test]
fn 性質_b_連続した一致は飛び飛びの一致より上() {
    let (top, _) = ranked(&["xaxbxc.rs", "xabc.rs"], "abc");
    assert_eq!(top, ["xabc.rs", "xaxbxc.rs"]);
    let (top, _) = ranked(&["d/qaxbxcq", "d/qabcqqq"], "abc");
    assert_eq!(top, ["d/qabcqqq", "d/qaxbxcq"], "同じ長さでも連続が上");
}

#[test]
fn 性質_c_語頭での一致は語の途中での一致より上() {
    let (top, _) = ranked(&["src/xfoo.rs", "src/foo_.rs"], "foo");
    assert_eq!(top, ["src/foo_.rs", "src/xfoo.rs"]);
    // camelCase の境目も語頭
    let (top, _) = ranked(&["a/xbarz.rs", "a/xBarz.rs"], "bar");
    assert_eq!(top, ["a/xBarz.rs", "a/xbarz.rs"]);
}

#[test]
fn 同点は短い順_それも同じならバイト列の順() {
    let (top, _) = ranked(&["b/a.txt", "a/a.txt", "aa/a.txt"], "a.txt");
    assert_eq!(top, ["a/a.txt", "b/a.txt", "aa/a.txt"]);
}

#[test]
fn 空の検索語は短い順の上位_100_件と全件の数() {
    let names: Vec<String> = (0..150)
        .map(|i| format!("{}{i}", "d/".repeat(i % 3)))
        .collect();
    let rels: Vec<&str> = names.iter().map(String::as_str).collect();
    let (top, matched) = ranked(&rels, "  ");
    assert_eq!(matched, 150);
    assert_eq!(top.len(), FINDER_TOP);
    for w in top.windows(2) {
        assert!(
            (w[0].len(), &w[0]) < (w[1].len(), &w[1]),
            "短い順・バイト列の順: {w:?}"
        );
    }
}

#[test]
fn 空白で区切った語のすべてが一致する候補だけ() {
    let (top, matched) = ranked(&["src/main.rs", "src/lib.rs", "docs/main.md"], "main rs");
    assert_eq!(top, ["src/main.rs"]);
    assert_eq!(matched, 1);
    let (_, matched) = ranked(&["src/main.rs"], "MAIN");
    assert_eq!(matched, 1, "ASCII の英字は畳む");
}

#[cfg(windows)]
#[test]
fn windows_では検索語の円記号を区切りとして照らす() {
    let (top, _) = ranked(&["src/main.rs", "srcmain.rs"], r"src\main");
    assert_eq!(top, ["src/main.rs"]);
}

#[test]
fn 取り消していれば結果を返さない() {
    let chunks = chunks_of(&["a", "b"], 1);
    let c = CancelToken::new();
    c.cancel();
    assert!(finder_match(&chunks, "a", 10, &c).is_none());
}

#[test]
fn 強調の位置は相対パスの書記素の番号() {
    let chunks = chunks_of(&["日本/main.rs"], 1);
    let m = finder_match(&chunks, "本ma", 10, &CancelToken::new()).expect("一致");
    // 書記素: 日 本 / m a i n . r s
    assert_eq!(m.top[0].ranges, [1..2, 3..5]);
}

/// 参照の実装: 語ごとに、ASCII を小文字にした文字の部分列か。
fn reference_matches(rel: &str, query: &str) -> bool {
    let query = if cfg!(windows) {
        query.replace('\\', "/")
    } else {
        query.to_owned()
    };
    let hay: Vec<char> = rel.chars().map(|c| c.to_ascii_lowercase()).collect();
    query.split_whitespace().all(|w| {
        let mut it = hay.iter();
        w.chars()
            .map(|c| c.to_ascii_lowercase())
            .all(|c| it.any(|&h| h == c))
    })
}

fn rel_strategy() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        prop_oneof![
            Just("a"),
            Just("b"),
            Just("A"),
            Just("/"),
            Just("_"),
            Just("."),
            Just("1"),
            Just("あ"),
            Just("い"),
            Just("Ｂ")
        ],
        1..12,
    )
    .prop_map(|v| v.concat())
}

fn query_strategy() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        prop_oneof![
            Just("a"),
            Just("B"),
            Just("/"),
            Just("."),
            Just("1"),
            Just(" "),
            Just("あ"),
            Just("い"),
            Just("ｂ"),
            Just("\\")
        ],
        0..5,
    )
    .prop_map(|v| v.concat())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    // verifies: AC-FR-125-1
    #[test]
    fn 一致は語の部分列の_and_と同じで_並びは候補の順と塊に依らない(
        rels in proptest::collection::vec(rel_strategy(), 0..40),
        query in query_strategy(),
        size in 1usize..7,
        top in 0usize..12,
    ) {
        let refs: Vec<&str> = rels.iter().map(String::as_str).collect();
        let want = refs.iter().filter(|r| reference_matches(r, &query)).count();
        let run = |rels: &[&str], size: usize| {
            let chunks = chunks_of(rels, size);
            let m = finder_match(&chunks, &query, top, &CancelToken::new()).expect("取り消していない");
            let top: Vec<String> = m
                .top
                .iter()
                .map(|h| chunks[h.chunk as usize][h.index as usize].rel().to_owned())
                .collect();
            for h in &m.top {
                let rel = chunks[h.chunk as usize][h.index as usize].rel();
                prop_assert!(reference_matches(rel, &query) || query.trim().is_empty());
            }
            Ok((m.matched, top))
        };
        let (matched, a) = run(&refs, size)?;
        prop_assert_eq!(matched, want);
        prop_assert_eq!(a.len(), want.min(top));
        let mut rev = refs.clone();
        rev.reverse();
        let (_, b) = run(&rev, size % 3 + 1)?;
        prop_assert_eq!(a, b, "並びが候補の順・塊の切り方で変わった");
    }
}
