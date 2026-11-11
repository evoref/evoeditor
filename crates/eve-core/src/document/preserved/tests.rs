//! `PreservedMap` の個別の例 (03 §8.7.1)。性質テストは `props.rs`。

use eve_encoding::{
    Decoder, Encoding, PreserveReason, PreserveState, PreservedRecord, PreservedRecords,
    check_roundtrip_with_records,
};
use eve_text::{ByteOff, ByteRange, ChangeSet, Edit, Text, TextRead};

use super::*;

/// バイト列を復号した本文と、読込の検査で集めた記録から作った表。
pub(super) fn load(enc: Encoding, bytes: &[u8]) -> (Text, PreservedMap) {
    let mut d = Decoder::new(enc, false);
    let mut s = String::new();
    d.decode(bytes, true, &mut s);
    let (_, recs) = check_roundtrip_with_records(enc, bytes);
    let text = Text::from(s.as_str());
    let map = PreservedMap::from_records(&recs, enc, &text).expect("照合が通る");
    (text, map)
}

/// `(始まり, 終わり, 挿入する文字列)` の列から作る `ChangeSet`。
pub(super) fn cs(text: &dyn TextRead, edits: &[(usize, usize, &str)]) -> ChangeSet {
    ChangeSet::from_sorted_edits(
        text,
        edits.iter().map(|&(s, e, x)| Edit {
            range: ByteRange::new(ByteOff(s), ByteOff(e)),
            insert: x.into(),
        }),
    )
    .expect("妥当な編集")
}

/// 本文の全体。
pub(super) fn all(t: &dyn TextRead) -> String {
    t.slice_to_string(ByteRange::all(t.len_bytes()))
}

/// 本文の複製 (`Text` は複製できないので作り直す)。
pub(super) fn dup(t: &Text) -> Text {
    Text::from(all(t).as_str())
}

/// 外した記録が無い。
const NONE: [PreservedSpan; 0] = [];

fn ranges(map: &PreservedMap) -> Vec<(usize, usize)> {
    map.iter()
        .map(|r| (r.range.start.0, r.range.end.0))
        .collect()
}

/// 編集を本文と表に適用して、外した記録を返す。
fn edit(
    text: &mut Text,
    map: &mut PreservedMap,
    edits: &[(usize, usize, &str)],
) -> Vec<PreservedSpan> {
    let c = cs(text, edits);
    let removed = map.apply_edit(&c);
    text.apply(&c).expect("適用できる");
    removed
}

#[test]
fn 端に接する挿入で文字の記録は残り行の記録は外れる() {
    // Shift_JIS: a ￢(0xFA54。IBM 拡張の重複。U+FFE2) b
    let (mut text, mut map) = load(Encoding::ShiftJis, b"a\xFA\x54b");
    assert_eq!(all(&text), "a\u{FFE2}b");
    assert_eq!(ranges(&map), [(1, 4)]);
    assert!(!map.iter().next().unwrap().line);
    // 始まりに接する挿入: 範囲は挿入した文字を含まない
    assert_eq!(edit(&mut text, &mut map, &[(1, 1, "X")]), NONE);
    assert_eq!(ranges(&map), [(2, 5)]);
    // 終わりに接する挿入
    assert_eq!(edit(&mut text, &mut map, &[(5, 5, "Y")]), NONE);
    assert_eq!(ranges(&map), [(2, 5)]);
    // 端に接する置換 (前の文字を置き換える)
    assert_eq!(edit(&mut text, &mut map, &[(1, 2, "ZZ")]), NONE);
    assert_eq!(ranges(&map), [(3, 6)]);
    map.check(&text, Encoding::ShiftJis).unwrap();

    // ISO-2022-JP: 旧 JIS (ESC $ @) で書いた行は行の記録になる
    for at in [0usize, 3] {
        let (mut text, mut map) = load(Encoding::Iso2022Jp, b"\x1B$@$\"\x1B(B\nabc");
        assert_eq!(ranges(&map), [(0, 3)]);
        let r = map.iter().next().unwrap();
        assert!(r.line);
        assert_eq!(r.reason, PreserveReason::StatefulEscape);
        let removed = edit(&mut text, &mut map, &[(at, at, "x")]);
        assert_eq!(removed.len(), 1, "行の内容が変わるので外す ({at})");
        assert_eq!(removed[0].range, ByteRange::new(ByteOff(0), ByteOff(3)));
        assert!(map.is_empty());
    }
}

#[test]
fn 端に接する削除では残る() {
    let (mut text, mut map) = load(Encoding::ShiftJis, b"ab\xFA\x54cd");
    assert_eq!(ranges(&map), [(2, 5)]);
    assert_eq!(edit(&mut text, &mut map, &[(1, 2, "")]), NONE);
    assert_eq!(ranges(&map), [(1, 4)]);
    assert_eq!(edit(&mut text, &mut map, &[(4, 5, "")]), NONE);
    assert_eq!(ranges(&map), [(1, 4)]);
    // 範囲と同じ削除 (Inside)・範囲をまたぐ削除 (Straddle) は外す
    let (mut t2, mut m2) = (dup(&text), map.clone());
    assert_eq!(edit(&mut t2, &mut m2, &[(1, 4, "")]).len(), 1);
    assert_eq!(edit(&mut text, &mut map, &[(0, 4, "")]).len(), 1);
    assert!(map.is_empty());

    // 行の記録も、端に接する削除 (前の行の終端を消す) では残す (保存のときに行頭に無ければ使わない、03 §8.6 E2 (d))
    let (mut text, mut map) = load(Encoding::Iso2022Jp, b"x\n\x1B$@$\"\x1B(B\nyz");
    assert_eq!(ranges(&map), [(2, 5)]);
    assert_eq!(edit(&mut text, &mut map, &[(1, 2, "")]), NONE);
    assert_eq!(ranges(&map), [(1, 4)]);
    map.check(&text, Encoding::Iso2022Jp).unwrap();
}

#[test]
fn 長さ_0_の行の記録() {
    // 2 行目は冗長な ESC ( B だけの行 (内容が空の記録)
    let bytes = b"a\n\x1B(B\nb";
    let (text0, map0) = load(Encoding::Iso2022Jp, bytes);
    assert_eq!(ranges(&map0), [(2, 2)]);
    assert_eq!(map0.total_bytes(), 3);
    // 離れた編集では位置をずらして残す
    let (mut text, mut map) = (dup(&text0), map0.clone());
    assert_eq!(edit(&mut text, &mut map, &[(0, 0, "zz")]), NONE);
    assert_eq!(ranges(&map), [(4, 4)]);
    map.check(&text, Encoding::Iso2022Jp).unwrap();
    // 位置に接する挿入・削除 (前の行の終端を消す・後ろの行の終端を消す) は外す
    for e in [(2usize, 2usize, "x"), (1, 2, ""), (2, 3, ""), (1, 3, "q")] {
        let (mut text, mut map) = (dup(&text0), map0.clone());
        let c = cs(&text, &[e]);
        let before = text.snapshot();
        let removed = map.apply_edit(&c);
        assert_eq!(removed.len(), 1, "{e:?}");
        assert!(map.is_empty());
        text.apply(&c).unwrap();
        // Undo で同じ位置に戻る
        let inv = c.invert(&before);
        assert_eq!(map.apply_undo(&inv, &removed), NONE);
        text.apply(&inv).unwrap();
        assert_eq!(map, map0, "{e:?}");
        // Redo で再び外れる
        assert_eq!(map.apply_redo(&c, &removed), NONE);
        assert!(map.is_empty());
    }
}

#[test]
fn 範囲の入れ替え_粗い逆変換() {
    // 記録が 4 件: 0xFA54 / 0xED40 / 0xEEEF / 0x8790 (どれも再符号化で別のバイト列になる)
    let bytes = b"aa\xFA\x54bb\xED\x40cc\xEE\xEFdd\x87\x90ee";
    let (mut text, mut map) = load(Encoding::ShiftJis, bytes);
    let map0 = map.clone();
    let text0 = dup(&text);
    let r0 = ranges(&map);
    assert_eq!(r0.len(), 4);
    // 2 件目をまたぐ削除と、3 件目の後ろへの挿入 (記録の間の変わらない範囲も粗い逆変換で置き換わる)
    let (b1, e1) = r0[1];
    let (_, e2) = r0[2];
    let c = cs(&text, &[(b1 - 1, e1, ""), (e2 + 1, e2 + 1, "XYZ")]);
    let old = ByteRange::new(ByteOff(b1 - 1), ByteOff(e2 + 1));
    let hull = map.affected_hull(old);
    assert_eq!(hull, ByteRange::new(ByteOff(b1 - 1), ByteOff(e2 + 1)));
    assert!(e1 < e2);
    let before = map.spans_in(hull);
    assert_eq!(before.len(), 2, "2 件目 (外れる) と 3 件目 (残る)");
    let snap = text.snapshot();
    let removed = map.apply_edit(&c);
    assert_eq!(removed.len(), 1);
    text.apply(&c).unwrap();
    let after_map = map.clone();
    let inv = crate::document::coarse_inverse(&c, &snap, &text).expect("粗い逆変換");
    assert_eq!(inv.changed_ranges().count(), 1);
    // Undo: 置き換えた範囲の記録を、編集の前の記録に入れ替える
    let hull_new = ByteRange::new(
        hull.start,
        ByteOff(hull.end.0 + c.len_after() - c.len_before()),
    );
    let out = map.replace_range(hull_new, hull.len(), &before);
    assert_eq!(out.len(), 1, "3 件目 (編集の後の位置)");
    text.apply(&inv).unwrap();
    assert_eq!(all(&text), all(&text0));
    assert_eq!(map, map0);
    map.check(&text, Encoding::ShiftJis).unwrap();
    // Redo: 同じ記録を外して写す
    assert_eq!(map.apply_redo(&c, &removed), NONE);
    assert_eq!(map, after_map);

    // 文書全体の置き換え (再読込の形): 全部を新しい記録に入れ替える
    let mut m = map0.clone();
    let all = ByteRange::all(text0.len_bytes());
    let new_spans = vec![PreservedSpan {
        range: ByteRange::new(ByteOff(1), ByteOff(3)),
        bytes: Box::from(&b"\xFA\x54"[..]),
        reason: PreserveReason::DuplicateEncoding,
        line: false,
    }];
    let old_spans = m.replace_range(all, 10, &new_spans);
    assert_eq!(old_spans.len(), 4);
    assert_eq!(ranges(&m), [(1, 3)]);
    assert_eq!(m.total_bytes(), 2);
    m.replace_range(ByteRange::all(10), text0.len_bytes(), &old_spans);
    assert_eq!(m, map0);
}

#[test]
fn 塊の境をまたぐ編集() {
    // 記録が 12 件 (￢ と a を交互に)
    let bytes: Vec<u8> = b"\xFA\x54a".repeat(12);
    let (mut text, mut map) = load(Encoding::ShiftJis, &bytes);
    assert_eq!(map.len(), 12);
    map.rechunk(ChunkLimits {
        max: 2,
        max_bytes: 1024,
    });
    assert_eq!(map.chunk_count(), 6);
    let map0 = map.clone();
    // 2 件目〜5 件目 (2 つの塊をまたぐ) を消す削除と、10 件目の後ろへの挿入
    let r = ranges(&map);
    let snap = text.snapshot();
    let c = cs(&text, &[(r[1].0, r[4].1, ""), (r[9].1, r[9].1, "あ")]);
    let removed = map.apply_edit(&c);
    assert_eq!(
        removed.iter().map(|s| s.range.start.0).collect::<Vec<_>>(),
        [r[1].0, r[2].0, r[3].0, r[4].0]
    );
    text.apply(&c).unwrap();
    assert_eq!(map.len(), 8);
    map.check(&text, Encoding::ShiftJis).unwrap();
    assert!(map.chunk_count() <= 6);
    // 塊の大きさは上限を超えない (check_structure)。Undo で元に戻る
    let inv = c.invert(&snap);
    assert_eq!(map.apply_undo(&inv, &removed), NONE);
    text.apply(&inv).unwrap();
    assert_eq!(map, map0);
    map.check(&text, Encoding::ShiftJis).unwrap();
    // 元の表 (複製) は編集の影響を受けていない
    assert_eq!(ranges(&map0), r);

    // 元のバイト列の大きさで塊を分ける
    map.rechunk(ChunkLimits {
        max: 8,
        max_bytes: 3,
    });
    assert_eq!(map.chunk_count(), 12);
    map.check(&text, Encoding::ShiftJis).unwrap();
}

#[test]
fn 空の表() {
    let mut map = PreservedMap::new();
    let text = Text::from("abc");
    assert!(map.is_empty());
    assert_eq!(map.len(), 0);
    assert_eq!(map.total_bytes(), 0);
    assert_eq!(map.iter().count(), 0);
    assert_eq!(map.iter_from(ByteOff(1)).count(), 0);
    assert_eq!(map.apply_edit(&cs(&text, &[(1, 2, "x")])), NONE);
    let r = ByteRange::new(ByteOff(1), ByteOff(2));
    assert_eq!(map.affected_hull(r), r);
    assert_eq!(map.spans_in(r), NONE);
    map.check(&text, Encoding::ShiftJis).unwrap();
    // 空の表に戻す
    let span = PreservedSpan {
        range: ByteRange::new(ByteOff(1), ByteOff(3)),
        bytes: Box::from(&b"\xFA\x54"[..]),
        reason: PreserveReason::DuplicateEncoding,
        line: false,
    };
    let identity = cs(&text, &[(1, 1, "")]);
    assert_eq!(map.apply_undo(&identity, std::slice::from_ref(&span)), NONE);
    assert_eq!(map.to_spans(), [span]);
    assert_eq!(map.total_bytes(), 2);
    // 状態が Collected でない記録からは空の表
    let recs = PreservedRecords {
        state: PreserveState::Overflow,
        ..PreservedRecords::default()
    };
    let m = PreservedMap::from_records(&recs, Encoding::ShiftJis, &text).unwrap();
    assert!(m.is_empty());
}

fn record(line: u64, col: u64, len: u32, bytes: std::ops::Range<u32>) -> PreservedRecord {
    PreservedRecord {
        line,
        col,
        len,
        bytes,
        reason: PreserveReason::DuplicateEncoding,
    }
}

#[test]
fn 照合に合わない記録があれば作らない() {
    let text = Text::from("a\u{FFE2}b\nc");
    let recs = |r: Vec<PreservedRecord>, arena: &[u8]| PreservedRecords {
        records: r,
        arena: arena.to_vec(),
        state: PreserveState::Collected,
        unpreserved: 0,
    };
    let build = |r: &PreservedRecords, enc| PreservedMap::from_records(r, enc, &text);
    // 正しい記録
    let ok = recs(vec![record(0, 1, 3, 0..2)], b"\xFA\x54");
    assert_eq!(build(&ok, Encoding::ShiftJis).unwrap().len(), 1);
    // 元のバイト列が別の文字に復号される
    let bad = recs(vec![record(0, 1, 3, 0..2)], b"\x82\xA0");
    assert_eq!(
        build(&bad, Encoding::ShiftJis).unwrap_err(),
        PreservedError {
            index: 0,
            kind: PreservedErrorKind::Mismatch
        }
    );
    // 不正バイト (先頭バイトだけ)
    let bad = recs(vec![record(0, 1, 3, 0..1)], b"\xFA");
    assert_eq!(
        build(&bad, Encoding::ShiftJis).unwrap_err().kind,
        PreservedErrorKind::Mismatch
    );
    // 2 件目だけが合わない
    let bad = recs(
        vec![record(0, 1, 3, 0..2), record(1, 0, 1, 2..4)],
        b"\xFA\x54\xFA\x54",
    );
    assert_eq!(
        build(&bad, Encoding::ShiftJis).unwrap_err(),
        PreservedError {
            index: 1,
            kind: PreservedErrorKind::Mismatch
        }
    );
    // 行の外・行番号の外・char 境界でない・昇順でない
    for (r, kind) in [
        (record(0, 3, 3, 0..2), PreservedErrorKind::OutOfRange),
        (record(2, 0, 1, 0..2), PreservedErrorKind::OutOfRange),
        (record(0, 2, 1, 0..2), PreservedErrorKind::NotCharBoundary),
        (record(0, 1, 3, 0..3), PreservedErrorKind::OutOfRange),
    ] {
        let bad = recs(vec![r], b"\xFA\x54");
        assert_eq!(build(&bad, Encoding::ShiftJis).unwrap_err().kind, kind);
    }
    let bad = recs(
        vec![record(1, 0, 1, 0..1), record(0, 1, 3, 1..3)],
        b"c\xFA\x54",
    );
    assert_eq!(
        build(&bad, Encoding::ShiftJis).unwrap_err().kind,
        PreservedErrorKind::NotSorted
    );

    // ISO-2022-JP: ASCII 状態で終わらない (ローマ字状態のまま) 記録は合わない
    let text = Text::from("¥\nx");
    let iso = |arena: &[u8], len: u32| PreservedRecords {
        records: vec![PreservedRecord {
            line: 0,
            col: 0,
            len,
            bytes: 0..u32::try_from(arena.len()).unwrap(),
            reason: PreserveReason::StatefulEscape,
        }],
        arena: arena.to_vec(),
        state: PreserveState::Collected,
        unpreserved: 0,
    };
    let ok = iso(b"\x1B(J\\\x1B(B", 2);
    assert_eq!(
        PreservedMap::from_records(&ok, Encoding::Iso2022Jp, &text)
            .unwrap()
            .len(),
        1
    );
    let bad = iso(b"\x1B(J\\", 2);
    assert_eq!(
        PreservedMap::from_records(&bad, Encoding::Iso2022Jp, &text)
            .unwrap_err()
            .kind,
        PreservedErrorKind::Mismatch
    );
    // 行の記録が行の内容の全体でない
    let text = Text::from("¥a\nx");
    let bad = iso(b"\x1B(J\\\x1B(B", 2);
    assert_eq!(
        PreservedMap::from_records(&bad, Encoding::Iso2022Jp, &text)
            .unwrap_err()
            .kind,
        PreservedErrorKind::WrongUnit
    );
}

#[test]
fn cr_と_crlf_の文書で行番号を位置に直す() {
    // a CRLF ￢ CR b￢ LF ￢
    let bytes = b"a\r\n\xFA\x54\rb\xFA\x54\n\xFA\x54";
    let (text, map) = load(Encoding::ShiftJis, bytes);
    assert_eq!(all(&text), "a\r\n\u{FFE2}\rb\u{FFE2}\n\u{FFE2}");
    assert_eq!(ranges(&map), [(3, 6), (8, 11), (12, 15)]);
    map.check(&text, Encoding::ShiftJis).unwrap();
    // ISO-2022-JP も同じ (CR で区切った行の記録)
    let (text, map) = load(Encoding::Iso2022Jp, b"x\r\x1B$@$\"\x1B(B\r\ny");
    assert_eq!(ranges(&map), [(2, 5)]);
    map.check(&text, Encoding::Iso2022Jp).unwrap();
}

#[test]
fn ある位置以降の最初の記録() {
    let bytes: Vec<u8> = b"\xFA\x54a".repeat(7);
    let (_, mut map) = load(Encoding::ShiftJis, &bytes);
    map.rechunk(ChunkLimits {
        max: 3,
        max_bytes: 1024,
    });
    let all = ranges(&map);
    for pos in 0..=bytes.len() + 4 {
        let want: Vec<_> = all.iter().copied().filter(|r| r.0 >= pos).collect();
        let got: Vec<_> = map
            .iter_from(ByteOff(pos))
            .map(|r| (r.range.start.0, r.range.end.0))
            .collect();
        assert_eq!(got, want, "{pos}");
    }
}

#[test]
fn 複製は後の編集の影響を受けない() {
    let bytes: Vec<u8> = b"\xFA\x54a".repeat(20);
    let (mut text, mut map) = load(Encoding::ShiftJis, &bytes);
    map.rechunk(ChunkLimits {
        max: 4,
        max_bytes: 1024,
    });
    let copy = map.clone();
    let spans = copy.to_spans();
    edit(&mut text, &mut map, &[(0, 0, "x"), (8, 12, "")]);
    assert_ne!(map, copy);
    assert_eq!(copy.to_spans(), spans);
    copy.check_structure().unwrap();
}
