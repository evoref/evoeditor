//! `Selection` のプロパティテスト (docs/03-core-design.md §4.4 の N2〜N6、§6.4 の写像)。

use eve_text::{ByteOff, ByteRange, ChangeSet, Edit, MapPolicy, Range, Selection, Text};
use proptest::prelude::*;

const LEN: usize = 40;

fn raw_selection() -> impl Strategy<Value = (Vec<(usize, usize)>, usize)> {
    // 長さを少し超える位置 (N6 の収め込み) も出す。小さい範囲に偏らせて、重なりと接触を起こしやすくする
    let pos = prop_oneof![4 => 0usize..=LEN, 1 => 0usize..=LEN + 10];
    proptest::collection::vec((pos.clone(), pos), 1..8).prop_flat_map(|v| {
        let n = v.len();
        (Just(v), 0..n)
    })
}

fn build(raw: &[(usize, usize)], primary: usize) -> Selection {
    let ranges = raw
        .iter()
        .map(|&(a, h)| Range::new(ByteOff(a), ByteOff(h)))
        .collect();
    Selection::new(ranges, primary).unwrap()
}

/// N2〜N6 を検査する。
fn check_invariants(s: &Selection, len: usize) {
    assert!(!s.is_empty() && s.primary_index() < s.len(), "N5");
    let rs = s.ranges();
    for r in rs {
        assert!(r.anchor.0 <= len && r.head.0 <= len, "N6: {r:?} > {len}");
    }
    for (i, w) in rs.windows(2).enumerate() {
        let (a, b) = (w[0], w[1]);
        assert!(a.start() <= b.start(), "N2: {a:?} {b:?}");
        match (a.is_empty(), b.is_empty()) {
            (false, false) => assert!(a.end() < b.start(), "N3 非空同士: {a:?} {b:?}"),
            (true, true) => assert!(a.start() < b.start(), "N3 空同士: {a:?} {b:?}"),
            (true, false) => {
                assert!(
                    a.start() < b.start() || i == s.primary_index(),
                    "N4: 空のカーソルが非空の端にある: {a:?} {b:?}"
                );
            }
            (false, true) => {
                assert!(
                    a.end() < b.start() || i + 1 == s.primary_index(),
                    "N4: 空のカーソルが非空の端にある: {a:?} {b:?}"
                );
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    #[test]
    fn 併合後は正規形を満たし_冪等(raw in raw_selection()) {
        let (rs, primary) = raw;
        let s = build(&rs, primary).merged(LEN);
        check_invariants(&s, LEN);
        prop_assert_eq!(s.clone().merged(LEN), s);
    }

    #[test]
    fn 併合は元の範囲を覆い_primary_を保つ(raw in raw_selection()) {
        let (rs, primary) = raw;
        let clamp = |p: usize| p.min(LEN);
        let s = build(&rs, primary).merged(LEN);
        // primary は、元の primary を含む Range を指す
        let (pa, ph) = rs[primary];
        let (ps, pe) = (clamp(pa.min(ph)), clamp(pa.max(ph)));
        let np = s.primary();
        prop_assert!(np.start().0 <= ps && pe <= np.end().0, "primary {:?} が {:?} に含まれない", (ps, pe), np);
        // 非空の元の Range はどれかの結果に含まれる
        for &(a, h) in &rs {
            let (st, en) = (clamp(a.min(h)), clamp(a.max(h)));
            if st < en {
                prop_assert!(
                    s.iter().any(|r| r.start().0 <= st && en <= r.end().0),
                    "{:?} がどの結果にも含まれない: {:?}", (st, en), s
                );
            }
        }
    }

    #[test]
    fn 写像の結果は正規形を満たす(
        raw in raw_selection(),
        edits in proptest::collection::vec((0usize..=LEN, 0usize..=LEN, "[a-z]{0,4}"), 0..4),
        own in any::<bool>(),
    ) {
        let (rs, primary) = raw;
        let text = Text::from("x".repeat(LEN).as_str());
        let snap = text.snapshot();
        // 位置を昇順にして重ならない編集を作る
        let mut cuts: Vec<usize> = edits.iter().flat_map(|e| [e.0, e.1]).collect();
        cuts.sort_unstable();
        let es: Vec<Edit> = cuts
            .as_chunks::<2>().0.iter()
            .zip(edits.iter())
            .map(|(c, e)| Edit { range: ByteRange::from(c[0]..c[1]), insert: e.2.as_str().into() })
            .collect();
        let cs = ChangeSet::from_sorted_edits(&snap, es).unwrap();
        let policy = if own { MapPolicy::Own } else { MapPolicy::Other };
        let before = build(&rs, primary).merged(LEN);
        let after = before.map(&cs, policy);
        check_invariants(&after, cs.len_after());
    }
}
