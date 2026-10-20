//! `ChangeSet` のプロパティテスト (docs/03-core-design.md §6.2 の CS1〜CS6、§6.4 の位置写像、§2.5 の INV-T1 / INV-T3)。
//!
//! 文字列の素朴なモデル (`&str` を直接組み立てる) と、`Text::apply` の結果を突き合わせる。
//! Undo 相当として、適用のたびに `invert` を積み、逆順に適用して元に戻ることも確かめる。

use eve_text::{
    Bias, ByteOff, ByteRange, ChangeSet, ChangeSetError, ComposeError, Edit, EditError, InsertText,
    Op, OverlapKind, RangeMap, Snapshot, Text, TextRead,
};
use proptest::prelude::*;
use std::sync::Arc;

// ---------- 生成 ----------

fn any_char() -> impl Strategy<Value = char> {
    prop_oneof![
        3 => Just('\r'),
        3 => Just('\n'),
        6 => proptest::char::range('a', 'z'),
        1 => Just('\u{3042}'),
        1 => Just('é'),
        1 => Just('\u{1F600}'),
    ]
}

fn any_string(max: usize) -> impl Strategy<Value = String> {
    proptest::collection::vec(any_char(), 0..max).prop_map(|v| v.into_iter().collect())
}

/// 短いテキストが中心で、ときどき複数チャンクにまたがる長さ。
fn any_text() -> impl Strategy<Value = String> {
    prop_oneof![
        9 => any_string(120),
        1 => proptest::collection::vec(any_char(), 1500..4000).prop_map(|v| v.into_iter().collect::<String>()),
    ]
}

/// 置換の素: 位置 2 つ (文書の長さに対する千分率。使うときに実際の位置にして char 境界へ丸める) と挿入文字列。
type RawEdit = (u16, u16, String);

fn raw_edits() -> impl Strategy<Value = Vec<RawEdit>> {
    // 一様な位置に加えて、決まった位置 (同じ位置の複数の挿入・接する編集) が出やすいようにする
    let pos = prop_oneof![
        4 => 0u16..=1000,
        1 => proptest::sample::select(vec![0u16, 250, 500, 500, 750, 1000]),
    ];
    proptest::collection::vec((pos.clone(), pos, any_string(8)), 0..5)
}

fn config() -> ProptestConfig {
    ProptestConfig {
        cases: 128,
        ..ProptestConfig::default()
    }
}

/// `pos` を `s` の char 境界まで進める。
fn snap(s: &str, mut pos: usize) -> usize {
    pos = pos.min(s.len());
    while !s.is_char_boundary(pos) {
        pos += 1;
    }
    pos
}

/// 千分率から、`s` の中の char 境界の位置を作る。
fn at(s: &str, permille: u16) -> usize {
    snap(s, s.len() * usize::from(permille) / 1000)
}

/// 素の編集から、昇順で重ならない置換の列を作る。
fn sorted_edits(s: &str, raw: &[RawEdit]) -> Vec<(usize, usize, String)> {
    let mut points: Vec<usize> = raw
        .iter()
        .flat_map(|(a, b, _)| [at(s, *a), at(s, *b)])
        .collect();
    points.sort_unstable();
    points
        .chunks(2)
        .zip(raw)
        .map(|(p, (_, _, ins))| (p[0], p[1], ins.clone()))
        .collect()
}

fn make_cs(text: &Text, s: &str, raw: &[RawEdit]) -> (ChangeSet, Vec<(usize, usize, String)>) {
    let edits = sorted_edits(s, raw);
    let cs = ChangeSet::from_sorted_edits(
        text,
        edits.iter().map(|(a, b, ins)| Edit {
            range: ByteRange::new(ByteOff(*a), ByteOff(*b)),
            insert: Arc::from(ins.as_str()),
        }),
    )
    .expect("昇順で重ならず、char 境界の編集は受け付けられる");
    (cs, edits)
}

/// モデル: 編集を適用した文字列。
fn model_apply(s: &str, edits: &[(usize, usize, String)]) -> String {
    let mut out = String::new();
    let mut pos = 0;
    for (a, b, ins) in edits {
        out.push_str(&s[pos..*a]);
        out.push_str(ins);
        pos = *b;
    }
    out.push_str(&s[pos..]);
    out
}

fn contents(t: &impl TextRead) -> String {
    t.slice_to_string(ByteRange::all(t.len_bytes()))
}

/// 編集が文書を何も変えないか (すべて、範囲が空で挿入も空)。
fn edits_are_noop(edits: &[(usize, usize, String)]) -> bool {
    edits.iter().all(|(a, b, ins)| a == b && ins.is_empty())
}

/// モデル: 触れ合う編集 (前の終わり == 次の始まり) を 1 つの置換のまとまりにする。
/// 戻り値は (始まり, 終わり, 挿入の長さの合計, そのまとまりより前の差分の合計)。
fn model_groups(edits: &[(usize, usize, String)]) -> Vec<(usize, usize, usize, i64)> {
    let mut groups: Vec<(usize, usize, usize, i64)> = Vec::new();
    let mut delta = 0i64;
    for (a, b, ins) in edits {
        match groups.last_mut() {
            Some(g) if g.1 == *a => {
                g.1 = *b;
                g.2 += ins.len();
            }
            _ => groups.push((*a, *b, ins.len(), delta)),
        }
        delta += ins.len() as i64 - (*b - *a) as i64;
    }
    // 空のまとまり (範囲が空で挿入も空) は、変更ではない
    groups.retain(|g| g.0 != g.1 || g.2 != 0);
    groups
}

/// モデル: 位置 `p` の写像 (Before, After)。03 §6.4 の表。
fn model_map(groups: &[(usize, usize, usize, i64)], p: usize) -> (usize, usize) {
    let mut shift = 0i64; // p より前のまとまりの差分の合計
    for &(a, b, ins, _) in groups {
        if p < a {
            break;
        }
        if p <= b {
            // 挿入位置 (a == b) または削除の内側・端。Before は始まり、After は挿入の後ろ
            let start = (a as i64 + shift) as usize;
            return (start, start + ins);
        }
        shift += ins as i64 - (b - a) as i64;
    }
    let m = (p as i64 + shift) as usize;
    (m, m)
}

/// 正規形か (CS6): 0 長の操作がなく、隣接する同種は結合され、Insert の直後に Delete が来ない。
fn is_normal(cs: &ChangeSet) -> bool {
    let ops = cs.ops();
    let zero = ops.iter().any(|op| match op {
        Op::Retain(n) | Op::Delete(n) => *n == 0,
        Op::Insert(s) => s.is_empty(),
    });
    let adjacent = ops.windows(2).any(|w| {
        matches!(
            (&w[0], &w[1]),
            (Op::Retain(_), Op::Retain(_))
                | (Op::Delete(_), Op::Delete(_))
                | (Op::Insert(_), Op::Insert(_))
                | (Op::Insert(_), Op::Delete(_))
        )
    });
    !zero && !adjacent
}

// ---------- 性質 ----------

proptest! {
    #![proptest_config(config())]

    // spec: 03 §6.1 / §6.2 (from_sorted_edits と Text::apply)
    #[test]
    fn 適用の結果はモデルと一致する(s in any_text(), raw in raw_edits()) {
        let mut t = Text::from(s.as_str());
        let (cs, edits) = make_cs(&t, &s, &raw);
        prop_assert!(is_normal(&cs));
        prop_assert_eq!(cs.len_before(), s.len());
        let expected = model_apply(&s, &edits);
        prop_assert_eq!(cs.len_after(), expected.len());
        let before: Snapshot = t.snapshot();
        t.apply(&cs).unwrap();
        prop_assert_eq!(contents(&t), expected);
        // INV-T3: apply の後も、既存のスナップショットは旧内容を保つ
        prop_assert_eq!(contents(&before), s);
    }

    // spec: 03 §6.2 CS1: apply(apply(t, a), b) == apply(t, a.compose(b))
    #[test]
    fn cs1_合成して適用するのと順に適用するのは同じ(s in any_text(), ra in raw_edits(), rb in raw_edits()) {
        let mut t = Text::from(s.as_str());
        let (a, ea) = make_cs(&t, &s, &ra);
        let s1 = model_apply(&s, &ea);
        let mut t1 = Text::from(s1.as_str());
        let (b, eb) = make_cs(&t1, &s1, &rb);
        let composed = a.compose(&b).unwrap();
        prop_assert!(is_normal(&composed));
        prop_assert_eq!(composed.len_before(), a.len_before());
        prop_assert_eq!(composed.len_after(), b.len_after());
        t.apply(&composed).unwrap();
        t1.apply(&b).unwrap();
        prop_assert_eq!(contents(&t), model_apply(&s1, &eb));
        prop_assert_eq!(contents(&t), contents(&t1));
    }

    // spec: 03 §6.2 CS2: apply(apply(t, a), a.invert(t)) == t
    #[test]
    fn cs2_逆変換を適用すると元に戻る(s in any_text(), raw in raw_edits()) {
        let mut t = Text::from(s.as_str());
        let (cs, _) = make_cs(&t, &s, &raw);
        let inv = cs.invert(&t.snapshot());
        prop_assert!(is_normal(&inv));
        prop_assert_eq!(inv.len_before(), cs.len_after());
        prop_assert_eq!(inv.len_after(), cs.len_before());
        t.apply(&cs).unwrap();
        t.apply(&inv).unwrap();
        prop_assert_eq!(contents(&t), s);
    }

    // spec: 03 §6.2 CS3: a.invert(t).invert(apply(t, a)) == a (正規形で比較)
    #[test]
    fn cs3_逆変換の逆変換は元に戻る(s in any_text(), raw in raw_edits()) {
        let mut t = Text::from(s.as_str());
        let (cs, _) = make_cs(&t, &s, &raw);
        let inv = cs.invert(&t.snapshot());
        t.apply(&cs).unwrap();
        prop_assert_eq!(inv.invert(&t.snapshot()), cs);
    }

    // spec: 03 §6.2 CS5: identity.compose(a) == a == a.compose(identity)
    #[test]
    fn cs5_恒等は合成の単位元(s in any_text(), raw in raw_edits()) {
        let t = Text::from(s.as_str());
        let (cs, edits) = make_cs(&t, &s, &raw);
        let left = ChangeSet::identity(cs.len_before()).compose(&cs).unwrap();
        let right = cs.compose(&ChangeSet::identity(cs.len_after())).unwrap();
        prop_assert_eq!(&left, &cs);
        prop_assert_eq!(&right, &cs);
        prop_assert_eq!(cs.is_identity(), edits_are_noop(&edits));
    }

    // spec: 03 §6.2 CS6: 正規形。どの経路で作っても正規形になる
    #[test]
    fn cs6_任意の操作列から作っても正規形になる(
        ops in proptest::collection::vec(
            prop_oneof![
                (0usize..5).prop_map(Op::Retain),
                (0usize..5).prop_map(Op::Delete),
                any_string(4).prop_map(|s| Op::Insert(InsertText::from(s.as_str()))),
            ],
            0..12,
        )
    ) {
        let cs = ChangeSet::from_ops(ops.clone());
        prop_assert!(is_normal(&cs), "{:?} -> {:?}", ops, cs);
        // 正規化は冪等
        prop_assert_eq!(ChangeSet::from_ops(cs.ops().to_vec()), cs.clone());
        // 長さは操作列の合計と一致する
        let before: usize = ops.iter().map(|o| match o { Op::Retain(n) | Op::Delete(n) => *n, Op::Insert(_) => 0 }).sum();
        let after: usize = ops.iter().map(|o| match o { Op::Retain(n) => *n, Op::Delete(_) => 0, Op::Insert(s) => s.len() }).sum();
        prop_assert_eq!((cs.len_before(), cs.len_after()), (before, after));
    }

    // spec: 03 §6.4 CS4: map_pos は単調非減少。Before <= After。範囲は新しい文書の中
    #[test]
    fn cs4_位置写像は単調で範囲に収まる(s in any_text(), raw in raw_edits()) {
        let t = Text::from(s.as_str());
        let (cs, _) = make_cs(&t, &s, &raw);
        for bias in [Bias::Before, Bias::After] {
            let mut prev = 0;
            for p in 0..=s.len() {
                let m = cs.map_pos(ByteOff(p), bias).0;
                prop_assert!(m >= prev, "p={} で単調でない ({:?})", p, bias);
                prop_assert!(m <= cs.len_after());
                prev = m;
            }
        }
        for p in 0..=s.len() {
            prop_assert!(cs.map_pos(ByteOff(p), Bias::Before) <= cs.map_pos(ByteOff(p), Bias::After));
        }
        // 文書の末尾より後ろは、新しい文書の末尾に丸める
        prop_assert_eq!(cs.map_pos(ByteOff(usize::MAX), Bias::After), ByteOff(cs.len_after()));
    }

    // spec: 03 §6.4 の表 (挿入位置・削除の内側と端・置換・変更の前後) を、編集をグループ化したモデルと突き合わせる
    #[test]
    fn 位置写像は表のとおり(s in any_text(), raw in raw_edits()) {
        let t = Text::from(s.as_str());
        let (cs, edits) = make_cs(&t, &s, &raw);
        let groups = model_groups(&edits);
        for p in 0..=s.len() {
            let (before, after) = model_map(&groups, p);
            prop_assert_eq!(cs.map_pos(ByteOff(p), Bias::Before).0, before, "p={} Before", p);
            prop_assert_eq!(cs.map_pos(ByteOff(p), Bias::After).0, after, "p={} After", p);
        }
    }

    // spec: 03 §6.4 (map_range): Mapped なら、範囲の内容は変わらない
    #[test]
    fn map_range_が_mapped_なら範囲の内容は変わらない(s in any_text(), raw in raw_edits(), a in 0u16..=1000, b in 0u16..=1000) {
        let mut t = Text::from(s.as_str());
        let (cs, _) = make_cs(&t, &s, &raw);
        let (lo, hi) = (at(&s, a.min(b)), at(&s, a.max(b)));
        let range = ByteRange::new(ByteOff(lo), ByteOff(hi));
        let verdict = cs.map_range(range);
        t.apply(&cs).unwrap();
        if let RangeMap::Mapped(m) = verdict {
            prop_assert_eq!(t.slice_to_string(m), s[lo..hi].to_owned(), "範囲 {:?} -> {:?}", range, m);
        }
        // Straddle は写像を持たない
        if let RangeMap::Overlapped { mapped, kind } = verdict {
            prop_assert_eq!(mapped.is_none(), kind == OverlapKind::Straddle);
        }
    }

    // spec: 03 §6.2 changed_ranges: 昇順で重ならず、長さの収支が合う
    #[test]
    fn 変更されたまとまりは昇順で長さの収支が合う(s in any_text(), raw in raw_edits()) {
        let t = Text::from(s.as_str());
        let (cs, _) = make_cs(&t, &s, &raw);
        let groups: Vec<_> = cs.changed_ranges().collect();
        let mut last_old = 0;
        let mut last_new = 0;
        let (mut removed, mut added) = (0usize, 0usize);
        for (before, after) in &groups {
            prop_assert!(before.start.0 >= last_old && after.start.0 >= last_new);
            last_old = before.end.0;
            last_new = after.end.0;
            removed += before.len();
            added += after.len();
            prop_assert!(!(before.is_empty() && after.is_empty()), "空のまとまり");
        }
        prop_assert_eq!(cs.len_before() - removed + added, cs.len_after());
    }

    // spec: 03 §2.5 INV-T1 / 03 §6.2: char 境界でない操作は apply が拒否し、何も変更しない
    #[test]
    fn char_境界でない操作は拒否されて何も変わらない(s in any_text(), pick in 0usize..10_000) {
        let mut t = Text::from(s.as_str());
        let before = contents(&t);
        // 文字の途中を指す位置 (マルチバイト文字の先頭バイトの次) を 1 つ選ぶ
        let mids: Vec<usize> = s.char_indices().filter(|(_, c)| c.len_utf8() > 1).map(|(i, _)| i + 1).collect();
        if !mids.is_empty() {
            let m = mids[pick % mids.len()];
            // Retain の端が途中、Delete の端が途中、挿入位置が途中、の 3 通り
            let tail = s.len() - m;
            let cases = [
                vec![Op::Retain(m), Op::Retain(tail)],
                vec![Op::Retain(m - 1), Op::Delete(1), Op::Retain(tail)],
                vec![Op::Retain(m), Op::Insert(InsertText::from("x")), Op::Retain(tail)],
            ];
            for ops in cases {
                let cs = ChangeSet::from_ops(ops);
                // 1 つ目 (Retain だけ) は恒等に正規化され、境界の検査を受けない
                if cs.is_identity() {
                    continue;
                }
                prop_assert_eq!(t.apply(&cs), Err(ChangeSetError::NotCharBoundary { pos: ByteOff(m) }), "{:?}", cs);
                prop_assert_eq!(contents(&t), before.clone());
            }
            let e = ChangeSet::from_sorted_edits(&t, [Edit { range: ByteRange::new(ByteOff(m), ByteOff(m)), insert: Arc::from("x") }]);
            prop_assert_eq!(e, Err(EditError::NotCharBoundary { pos: ByteOff(m) }));
            let e = ChangeSet::from_sorted_edits(&t, [Edit { range: ByteRange::new(ByteOff(0), ByteOff(m)), insert: Arc::from("") }]);
            prop_assert_eq!(e, Err(EditError::NotCharBoundary { pos: ByteOff(m) }));
        }
        // 長さが合わない ChangeSet
        let cs = ChangeSet::identity(s.len() + 1);
        prop_assert_eq!(t.apply(&cs), Err(ChangeSetError::LenMismatch { expected: s.len() + 1, actual: s.len() }));
        prop_assert_eq!(contents(&t), before);
    }

    // spec: ChangeSet・Undo のプロパティテスト (08 §2.2 の (1))。適用のたびに invert を積み、逆順に適用すると元に戻る
    #[test]
    fn 逆変換を積んで巻き戻すと元に戻り_やり直すと同じ結果になる(s in any_text(), steps in proptest::collection::vec(raw_edits(), 1..6)) {
        let mut t = Text::from(s.as_str());
        let mut cur = s.clone();
        let mut undo: Vec<ChangeSet> = Vec::new();
        let mut redo: Vec<ChangeSet> = Vec::new();
        let mut composed = ChangeSet::identity(s.len());
        for raw in &steps {
            let (cs, edits) = make_cs(&t, &cur, raw);
            undo.push(cs.invert(&t.snapshot()));
            composed = composed.compose(&cs).unwrap();
            redo.push(cs.clone());
            t.apply(&cs).unwrap();
            cur = model_apply(&cur, &edits);
            prop_assert_eq!(contents(&t), cur.clone());
        }
        let final_text = cur.clone();
        // 全体を 1 つに合成したものを、元の文書に適用しても同じ
        let mut direct = Text::from(s.as_str());
        direct.apply(&composed).unwrap();
        prop_assert_eq!(contents(&direct), final_text.clone());
        // Undo: 逆順に適用
        for inv in undo.iter().rev() {
            t.apply(inv).unwrap();
        }
        prop_assert_eq!(contents(&t), s.clone());
        // Redo: 元の ChangeSet を順に適用
        for cs in &redo {
            t.apply(cs).unwrap();
        }
        prop_assert_eq!(contents(&t), final_text);
    }
}

// ---------- 個別の例 ----------

fn edit(a: usize, b: usize, ins: &str) -> Edit {
    Edit {
        range: ByteRange::new(ByteOff(a), ByteOff(b)),
        insert: Arc::from(ins),
    }
}

#[test]
fn 位置写像の表_03_6_4() {
    let t = Text::from("0123456789");
    // 挿入: 位置 3 に "XY"
    let ins = ChangeSet::from_sorted_edits(&t, [edit(3, 3, "XY")]).unwrap();
    assert_eq!(
        ins.map_pos(ByteOff(3), Bias::Before),
        ByteOff(3),
        "挿入位置: Before は挿入の前"
    );
    assert_eq!(
        ins.map_pos(ByteOff(3), Bias::After),
        ByteOff(5),
        "挿入位置: After は挿入の後"
    );
    assert_eq!(
        ins.map_pos(ByteOff(2), Bias::After),
        ByteOff(2),
        "変更より前"
    );
    assert_eq!(
        ins.map_pos(ByteOff(4), Bias::Before),
        ByteOff(6),
        "変更より後"
    );
    // 削除 [2, 5)
    let del = ChangeSet::from_sorted_edits(&t, [edit(2, 5, "")]).unwrap();
    for bias in [Bias::Before, Bias::After] {
        assert_eq!(
            del.map_pos(ByteOff(2), bias),
            ByteOff(2),
            "削除の端 (start)"
        );
        assert_eq!(del.map_pos(ByteOff(3), bias), ByteOff(2), "削除の内側");
        assert_eq!(del.map_pos(ByteOff(5), bias), ByteOff(2), "削除の端 (end)");
        assert_eq!(del.map_pos(ByteOff(6), bias), ByteOff(3), "削除より後");
    }
    // 置換 [2, 5) -> "X": 内側・端は Before が s、After が挿入の後
    let rep = ChangeSet::from_sorted_edits(&t, [edit(2, 5, "X")]).unwrap();
    assert_eq!(rep.map_pos(ByteOff(3), Bias::Before), ByteOff(2));
    assert_eq!(rep.map_pos(ByteOff(3), Bias::After), ByteOff(3));
    assert_eq!(
        rep.map_pos(ByteOff(5), Bias::After),
        ByteOff(3),
        "置換の終わりの端: After は挿入の後"
    );
    // 選択 [2, 5) の上で X を入力すると、head = 5 は After で入力した文字の後に写る (03 §6.4 の例)
    assert_eq!(rep.map_pos(ByteOff(5), Bias::After), ByteOff(3));
}

#[test]
fn map_range_の分類() {
    let t = Text::from("0123456789");
    let r = ByteRange::from(3..7);
    let map = |e: Edit| ChangeSet::from_sorted_edits(&t, [e]).unwrap().map_range(r);
    assert_eq!(
        map(edit(0, 0, "ab")),
        RangeMap::Mapped(ByteRange::from(5..9)),
        "範囲より前の挿入"
    );
    assert_eq!(
        map(edit(8, 8, "ab")),
        RangeMap::Mapped(ByteRange::from(3..7)),
        "範囲より後の挿入"
    );
    assert_eq!(
        map(edit(0, 3, "")),
        RangeMap::Mapped(ByteRange::from(0..4)),
        "範囲の直前までの削除は交差しない"
    );
    assert!(
        matches!(
            map(edit(3, 3, "ab")),
            RangeMap::Overlapped {
                kind: OverlapKind::Touch,
                ..
            }
        ),
        "start に接する挿入"
    );
    assert!(
        matches!(
            map(edit(7, 7, "ab")),
            RangeMap::Overlapped {
                kind: OverlapKind::Touch,
                ..
            }
        ),
        "end に接する挿入"
    );
    assert!(
        matches!(
            map(edit(5, 5, "ab")),
            RangeMap::Overlapped {
                kind: OverlapKind::Inside,
                ..
            }
        ),
        "内側の挿入"
    );
    assert!(
        matches!(
            map(edit(4, 6, "")),
            RangeMap::Overlapped {
                kind: OverlapKind::Inside,
                ..
            }
        ),
        "範囲に収まる削除"
    );
    assert!(
        matches!(
            map(edit(3, 7, "")),
            RangeMap::Overlapped {
                kind: OverlapKind::Inside,
                ..
            }
        ),
        "範囲全体の削除"
    );
    assert_eq!(
        map(edit(2, 5, "")),
        RangeMap::Overlapped {
            mapped: None,
            kind: OverlapKind::Straddle
        },
        "境界をまたぐ削除"
    );
    assert_eq!(
        map(edit(5, 9, "")),
        RangeMap::Overlapped {
            mapped: None,
            kind: OverlapKind::Straddle
        }
    );
    assert_eq!(
        map(edit(0, 9, "")),
        RangeMap::Overlapped {
            mapped: None,
            kind: OverlapKind::Straddle
        },
        "範囲を覆う削除"
    );
}

#[test]
fn from_sorted_edits_のエラー() {
    let t = Text::from("あいう"); // 9 バイト
    assert_eq!(
        ChangeSet::from_sorted_edits(&t, [edit(3, 6, ""), edit(5, 7, "")]),
        Err(EditError::Overlap)
    );
    assert_eq!(
        ChangeSet::from_sorted_edits(&t, [edit(6, 9, ""), edit(0, 3, "")]),
        Err(EditError::Overlap),
        "昇順でない"
    );
    assert_eq!(
        ChangeSet::from_sorted_edits(&t, [edit(0, 10, "")]),
        Err(EditError::OutOfRange)
    );
    assert_eq!(
        ChangeSet::from_sorted_edits(&t, [edit(1, 3, "")]),
        Err(EditError::NotCharBoundary { pos: ByteOff(1) })
    );
    // 接する編集と、同じ位置の複数の挿入は受け付ける
    let cs = ChangeSet::from_sorted_edits(
        &t,
        [
            edit(0, 3, "a"),
            edit(3, 6, "b"),
            edit(6, 6, "c"),
            edit(6, 6, "d"),
        ],
    )
    .unwrap();
    let mut t = t;
    t.apply(&cs).unwrap();
    assert_eq!(contents(&t), "abcdう");
}

#[test]
fn 合成のエラー() {
    let a = ChangeSet::identity(3);
    let b = ChangeSet::identity(4);
    assert_eq!(
        a.compose(&b),
        Err(ComposeError::LenMismatch {
            left_after: 3,
            right_before: 4
        })
    );
    // 前が挿入した文字列の char の途中で次が切る
    let t = Text::from("");
    let ins = ChangeSet::from_sorted_edits(&t, [edit(0, 0, "あ")]).unwrap(); // 0 -> 3
    let cut = ChangeSet::from_ops([Op::Retain(1), Op::Delete(2)]); // 3 -> 1
    assert_eq!(ins.compose(&cut), Err(ComposeError::SplitInsideChar));
}

#[test]
fn 恒等の判定と空の文書() {
    assert!(ChangeSet::identity(0).is_identity());
    assert!(ChangeSet::identity(5).is_identity());
    let t = Text::from("");
    let cs = ChangeSet::from_sorted_edits(&t, []).unwrap();
    assert!(cs.is_identity());
    assert_eq!(cs.len_before(), 0);
}

#[test]
fn デバッグ出力に挿入した本文を含めない() {
    let cs = ChangeSet::from_ops([Op::Insert(InsertText::from("秘密の本文"))]);
    let s = format!("{cs:?}");
    assert!(!s.contains("秘密"), "{s}");
}

#[test]
fn 置換は_delete_が_insert_の前に正規化される() {
    let cs = ChangeSet::from_ops([
        Op::Retain(1),
        Op::Insert(InsertText::from("x")),
        Op::Delete(2),
        Op::Retain(1),
    ]);
    assert!(
        matches!(
            cs.ops(),
            [Op::Retain(1), Op::Delete(2), Op::Insert(_), Op::Retain(1)]
        ),
        "{:?}",
        cs.ops()
    );
}

#[test]
fn 空の範囲が削除や置換の内側にあるときは_straddle() {
    // レビューで見つかった: 位置 (空の範囲) が削除・置換の内側にあるのに Mapped になっていた
    let t = Text::from("0123456789");
    let pos = ByteRange::from(3..3);
    let map = |e: Edit| {
        ChangeSet::from_sorted_edits(&t, [e])
            .unwrap()
            .map_range(pos)
    };
    let straddle = RangeMap::Overlapped {
        mapped: None,
        kind: OverlapKind::Straddle,
    };
    assert_eq!(map(edit(2, 5, "")), straddle, "削除の内側");
    assert_eq!(map(edit(2, 5, "X")), straddle, "置換の内側");
    // 端に接する削除は、位置を消さない
    assert_eq!(
        map(edit(3, 5, "")),
        RangeMap::Mapped(ByteRange::from(3..3)),
        "削除の始まりに接する"
    );
    assert_eq!(
        map(edit(1, 3, "")),
        RangeMap::Mapped(ByteRange::from(1..1)),
        "削除の終わりに接する"
    );
    // 端に接する挿入は Touch
    assert!(matches!(
        map(edit(3, 3, "ab")),
        RangeMap::Overlapped {
            kind: OverlapKind::Touch,
            ..
        }
    ));
}

#[test]
fn 多数の挿入を結合しても線形時間で済む() {
    // レビューで見つかった: 隣接する Insert を結合するたびに全体をコピーしていて、O(n^2) だった
    let n = 200_000;
    let cs = ChangeSet::from_ops((0..n).map(|_| Op::Insert(InsertText::from("y"))));
    assert_eq!(cs.ops().len(), 1);
    assert_eq!(cs.len_after(), n);
    // 長い挿入に、1 文字ずつ挿入する ChangeSet を合成しても同じ
    let a = ChangeSet::from_ops([Op::Insert(InsertText::from("a".repeat(n).as_str()))]);
    let b = ChangeSet::from_ops(
        (0..n).flat_map(|_| [Op::Retain(1), Op::Insert(InsertText::from("y"))]),
    );
    let c = a.compose(&b).unwrap();
    assert_eq!(c.ops().len(), 1);
    assert_eq!(c.len_after(), 2 * n);
}

#[test]
fn 合成は挿入の全体を使うとき_arc_を共有する() {
    let a = ChangeSet::from_ops([Op::Insert(InsertText::from("abc"))]); // 0 -> 3
    let c = a.compose(&ChangeSet::identity(3)).unwrap();
    let (Op::Insert(InsertText::Str(x)), Op::Insert(InsertText::Str(y))) =
        (&a.ops()[0], &c.ops()[0])
    else {
        panic!("挿入のはず");
    };
    assert!(
        Arc::ptr_eq(x, y),
        "全体を使うなら、文字列をコピーしない (03 §6.1)"
    );
}

#[test]
fn 巨大な長さでもパニックしない() {
    // レビューで見つかった: 加算のオーバーフロー (debug でパニック)
    let cs = ChangeSet::from_ops([Op::Retain(usize::MAX), Op::Delete(1)]);
    let _ = cs.invert(&Text::from("abc").snapshot());
    let _ = cs.changed_ranges().count();
    let _ = cs.map_pos(ByteOff(usize::MAX), Bias::After);
    let _ = cs.map_range(ByteRange::from(0..10));
    assert!(Text::from("abc").apply(&cs).is_err());
}
