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
    for w in rs.windows(2) {
        let (a, b) = (w[0], w[1]);
        assert!(a.start() <= b.start(), "N2: {a:?} {b:?}");
        match (a.is_empty(), b.is_empty()) {
            (false, false) => assert!(a.end() < b.start(), "N3 非空同士: {a:?} {b:?}"),
            (true, true) => assert!(a.start() < b.start(), "N3 空同士: {a:?} {b:?}"),
            // N4: 空のカーソルは非空の Range の端にも残らない (主のカーソルも併合される)
            (true, false) => {
                assert!(
                    a.start() < b.start(),
                    "N4: 空のカーソルが非空の端にある: {a:?} {b:?}"
                );
            }
            (false, true) => {
                assert!(
                    a.end() < b.start(),
                    "N4: 空のカーソルが非空の端にある: {a:?} {b:?}"
                );
            }
        }
    }
}

/// `merged` の参照モデル (03 §4.4 の N2〜N6 をそのまま書いたもの。実装の掃き出しとは別の、組を繰り返し併合する形)。
/// 結果は (start 昇順の (anchor, head) の並び, 主の番号)。
fn reference_merged(
    raw: &[(usize, usize)],
    primary: usize,
    len: usize,
) -> (Vec<(usize, usize)>, usize) {
    let clamp = |p: usize| p.min(len);
    // 非空の Range の組: (start, end, 元の番号の並び)。重なる・接するものを、変わらなくなるまで併合する (N3)
    let mut groups: Vec<(usize, usize, Vec<usize>)> = raw
        .iter()
        .enumerate()
        .map(|(i, &(a, h))| (clamp(a.min(h)), clamp(a.max(h)), vec![i]))
        .filter(|(s, e, _)| s < e)
        .collect();
    loop {
        let pair = (0..groups.len()).find_map(|i| {
            (i + 1..groups.len())
                .find(|&j| groups[i].0 <= groups[j].1 && groups[j].0 <= groups[i].1)
                .map(|j| (i, j))
        });
        let Some((i, j)) = pair else { break };
        let (s, e, members) = groups.remove(j);
        let g = &mut groups[i];
        g.0 = g.0.min(s);
        g.1 = g.1.max(e);
        g.2.extend(members);
    }
    groups.sort_by_key(|g| (g.0, g.1));
    // 向き: 主を含めば主の向き、含まなければ元の Range のうち (start, end) の最小のもの (同じなら入力の順) の向き
    let forward = |i: usize| raw[i].1 >= raw[i].0;
    let mut out: Vec<((usize, usize), bool)> = groups
        .iter()
        .map(|(s, e, members)| {
            let lead = if members.contains(&primary) {
                primary
            } else {
                *members
                    .iter()
                    .min_by_key(|&&m| {
                        (
                            clamp(raw[m].0.min(raw[m].1)),
                            clamp(raw[m].0.max(raw[m].1)),
                            m,
                        )
                    })
                    .expect("組は空でない")
            };
            let r = if forward(lead) { (*s, *e) } else { (*e, *s) };
            (r, members.contains(&primary))
        })
        .collect();
    // 空のカーソル (N4): 非空の `[start, end]` にあれば捨てる。主なら、その Range を主にし、端ならカーソルの側を head に
    let mut points: Vec<(usize, bool)> = Vec::new();
    for (i, &(a, h)) in raw.iter().enumerate() {
        let p = clamp(a);
        if clamp(h) != p {
            continue;
        }
        let is_primary = i == primary;
        if let Some((r, prim)) = out
            .iter_mut()
            .find(|((x, y), _)| (*x).min(*y) <= p && p <= (*x).max(*y))
        {
            if is_primary {
                let (s, e) = (r.0.min(r.1), r.0.max(r.1));
                *prim = true;
                if p == e {
                    *r = (s, e);
                } else if p == s {
                    *r = (e, s);
                }
            }
            continue;
        }
        match points.iter_mut().find(|(q, _)| *q == p) {
            Some((_, prim)) => *prim |= is_primary,
            None => points.push((p, is_primary)),
        }
    }
    out.extend(points.into_iter().map(|(p, prim)| ((p, p), prim)));
    out.sort_by_key(|((a, h), _)| ((*a).min(*h), (*a).max(*h)));
    let primary = out.iter().position(|(_, p)| *p).unwrap_or(0);
    (out.into_iter().map(|(r, _)| r).collect(), primary)
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
    fn 併合は参照モデルと一致する(raw in raw_selection()) {
        let (rs, primary) = raw;
        let s = build(&rs, primary).merged(LEN);
        let got: Vec<(usize, usize)> = s.iter().map(|r| (r.anchor.0, r.head.0)).collect();
        prop_assert_eq!((got, s.primary_index()), reference_merged(&rs, primary, LEN), "{:?} 主 {}", rs, primary);
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
