//! `PreservedMap` の性質テスト (03 §22.1 の保持方式の (c)(d))。
//!
//! 文書は `Shift_JIS` / EUC-JP (区分 B の文字を多く含む) と ISO-2022-JP (旧 JIS・ローマ字・半角カナ・冗長なエスケープの
//! 行を含む) のバイト列を復号して作り、記録は読込の検査 (`check_roundtrip_with_records`) で集めて `from_records` で表にする。
//! 任意の編集・Undo・Redo (ノードの結合と粗い逆変換を含む) の列について、次を確かめる。
//!
//! - (c) どの時点でも、表の各記録が不変条件 (範囲の文字列 = 元のバイト列を初期状態から復号したもの) を満たす。
//! - 素朴な実装 (記録の `Vec` を持ち、1 件ずつ `ChangeSet::map_range` で写す) と、表の中身・外した記録が一致する。
//! - (d) Undo / Redo の後の表は、そのノードの前・後の時点の表と同じ。最後に全部 Undo すると読込の直後の表に戻り、
//!   全部 Redo すると最後の表に戻る。
//! - 複製した版は、元の表をその後に編集しても変わらない。

use eve_encoding::{Decoder, Encoding, check_roundtrip_with_records};
use eve_text::{
    ByteOff, ByteRange, ChangeSet, CharIdx, Edit, OverlapKind, RangeMap, Text, TextRead,
};
use proptest::prelude::*;

use super::tests::all;
use super::*;

/// `Shift_JIS` の部品: ASCII、ふつうの漢字・かな、重複符号化 (IBM 拡張・NEC 選定 IBM 拡張・NEC 特殊文字)、
/// 互換文字・外字 (記録にならない)、半角カナ、不正バイト。
const SJIS: &[&[u8]] = &[
    b"a",
    b"Z\\ ",
    b"\x93\xFA",
    b"\x82\xA0",
    b"\xFA\x54",
    b"\xFA\x55",
    b"\xFA\x54",
    b"\xED\x40",
    b"\xEE\xEF",
    b"\xEE\xFC",
    b"\x87\x90",
    b"\x87\x9B",
    b"\x81\xCA",
    b"\xF0\x40",
    b"\xB1",
    b"\xFD",
];

/// EUC-JP の部品: ASCII、漢字・半角カナ、補助漢字 (JIS X 0212。復号専用)、別のコードに写るもの、NEC 特殊文字・IBM 拡張の
/// 重複、互換文字、不正バイト。
const EUC: &[&[u8]] = &[
    b"a",
    b"Z\\ ",
    b"\xC6\xFC",
    b"\xA4\xA2",
    b"\x8E\xB1",
    b"\x8F\xB0\xA1",
    b"\x8F\xA2\xB7",
    b"\x8F\xA2\xC3",
    b"\x8F\xF3\xF3",
    b"\xAD\xA1",
    b"\xAD\xE2",
    b"\xFC\xFC",
    b"\xFC\xEE",
    b"\xA1\xC1",
    b"\xFF",
];

/// ISO-2022-JP の行の途中の部品 (eve-encoding の性質テストと同じ。出力する文字で始める)。
const ISO_MID: &[&[u8]] = &[
    b"ab",
    b"\\~",
    b"x\x1B$B$\"\x1B(B",
    b"x\x1B$@$\"\x1B(B",
    b"x\x1B(J\\a",
    b"x\x1B(Ba",
    b"x\x1B(I12\x1B(B",
    b"x\x1B$B$\"\x1B(J\\",
    b"x\x1B$BF|K\\\x1B(B",
];

/// ISO-2022-JP の行頭の部品 (空・冗長な `ESC ( B` だけの行は、内容が空の記録になる)。
const ISO_START: &[&[u8]] = &[
    b"",
    b"",
    b"\x1B(B",
    b"\x1B(B",
    b"\x1B$@$\"\x1B(B",
    b"\x1B(Ba",
    b"\x1B(B",
    b"\x1B(J\\",
    b"\x1B$B$\"\x1B(B",
    b"\x1B(I1\x1B(B",
];

/// ISO-2022-JP の行末の部品 (空、または ASCII 状態に戻す)。
const ISO_END: &[&[u8]] = &[b"", b"", b"z\x1B(B"];

const EOLS: [&[u8]; 3] = [b"\n", b"\r\n", b"\r"];

/// 挿入する文字列 (区分 B の文字そのものを打ち直しても、記録は戻らない)。
const INSERTS: &[&str] = &[
    "", "a", "x\ny", "\n", "\r", "\r\n", "¬", "あ", "\u{FF71}", "\\", "纊",
];

fn stateless_bytes(atoms: &[&[u8]], picks: &[(u8, u8)]) -> Vec<u8> {
    let mut b = Vec::new();
    for &(k, v) in picks {
        if k < 30 {
            b.extend_from_slice(EOLS[usize::from(v) % 3]);
        } else {
            b.extend_from_slice(atoms[usize::from(v) % atoms.len()]);
        }
    }
    b
}

fn iso_bytes(lines: &[(u8, Vec<u8>, u8, u8)]) -> Vec<u8> {
    let mut b = Vec::new();
    for (start, mids, end, eol) in lines {
        b.extend_from_slice(ISO_START[usize::from(*start) % ISO_START.len()]);
        for &m in mids {
            b.extend_from_slice(ISO_MID[usize::from(m) % ISO_MID.len()]);
        }
        b.extend_from_slice(ISO_END[usize::from(*end) % ISO_END.len()]);
        if *eol > 0 {
            b.extend_from_slice(EOLS[usize::from(*eol) % 3]);
        }
    }
    b
}

/// (文字コード, バイト列)。
fn doc() -> impl Strategy<Value = (Encoding, Vec<u8>)> {
    let picks = proptest::collection::vec((any::<u8>(), any::<u8>()), 0..48);
    let iso = proptest::collection::vec(
        (
            any::<u8>(),
            proptest::collection::vec(any::<u8>(), 0..4),
            any::<u8>(),
            0u8..40,
        ),
        0..10,
    );
    prop_oneof![
        picks
            .clone()
            .prop_map(|p| (Encoding::ShiftJis, stateless_bytes(SJIS, &p))),
        picks.prop_map(|p| (Encoding::EucJp, stateless_bytes(EUC, &p))),
        iso.prop_map(|l| (Encoding::Iso2022Jp, iso_bytes(&l))),
    ]
}

/// 1 つの操作: (種類, 編集の部品の列, 付け足し)。
type Action = (u8, Vec<(u8, u16, u8, u8)>, u8);

fn actions() -> impl Strategy<Value = Vec<Action>> {
    proptest::collection::vec(
        (
            any::<u8>(),
            proptest::collection::vec((any::<u8>(), any::<u16>(), any::<u8>(), any::<u8>()), 1..4),
            any::<u8>(),
        ),
        0..16,
    )
}

/// 素朴な実装: 記録の列を 1 件ずつ `map_range` で写す。(残した記録, 外した記録)。
fn naive_edit(spans: &[PreservedSpan], cs: &ChangeSet) -> (Vec<PreservedSpan>, Vec<PreservedSpan>) {
    let hunks: Vec<(ByteRange, ByteRange)> = cs.changed_ranges().collect();
    let (mut kept, mut removed) = (Vec::new(), Vec::new());
    for sp in spans {
        // 長さ 0 の記録は、位置に接する変更 (削除の端を含む) でも外す (モジュールの説明)
        let zero_hit = sp.range.is_empty()
            && hunks
                .iter()
                .any(|(o, _)| o.start <= sp.range.start && sp.range.start <= o.end);
        let to = match cs.map_range(sp.range) {
            RangeMap::Mapped(r) if !zero_hit => Some(r),
            RangeMap::Overlapped {
                mapped: Some(r),
                kind: OverlapKind::Touch,
            } if !sp.line && !zero_hit => Some(r),
            _ => None,
        };
        match to {
            Some(r) => {
                assert_eq!(r.len(), sp.range.len(), "残す記録の長さは変わらない");
                kept.push(PreservedSpan {
                    range: r,
                    ..sp.clone()
                });
            }
            None => removed.push(sp.clone()),
        }
    }
    (kept, removed)
}

/// 粗い逆変換のノードが持つもの: 編集の後の座標の範囲、編集の前の範囲の長さ、編集の前のその範囲の記録。
struct Coarse {
    hull_new: ByteRange,
    old_len: usize,
    before: Vec<PreservedSpan>,
}

struct Node {
    redo: ChangeSet,
    undo: ChangeSet,
    removed: Vec<PreservedSpan>,
    coarse: Option<Coarse>,
}

struct World {
    enc: Encoding,
    text: Text,
    map: PreservedMap,
    undo: Vec<Node>,
    redo: Vec<Node>,
    /// `timeline[i]` = ノードを i 個適用した時点の、素朴な実装の記録。今は `timeline[undo.len()]`。
    timeline: Vec<Vec<PreservedSpan>>,
    /// 複製した版と、複製した時点の中身。
    copies: Vec<(PreservedMap, Vec<PreservedSpan>)>,
}

fn floor_boundary(t: &dyn TextRead, mut p: usize) -> usize {
    p = p.min(t.len_bytes());
    while !t.is_char_boundary(ByteOff(p)) {
        p -= 1;
    }
    p
}

impl World {
    fn check(&self) -> Result<(), TestCaseError> {
        // (c) 不変条件
        prop_assert_eq!(self.map.check(&self.text, self.enc), Ok(()));
        // 素朴な実装と同じ中身
        prop_assert_eq!(&self.map.to_spans(), &self.timeline[self.undo.len()]);
        Ok(())
    }

    /// 編集の部品から `ChangeSet` を作る (位置は、任意の位置・記録の始まり・終わり・文書の先頭と末尾から選び、char 境界に
    /// 丸める)。恒等なら `None`。
    fn make_cs(&self, parts: &[(u8, u16, u8, u8)]) -> Option<ChangeSet> {
        let t = &self.text;
        let len = t.len_bytes();
        let recs: Vec<(usize, usize)> = self
            .map
            .iter()
            .map(|r| (r.range.start.0, r.range.end.0))
            .collect();
        let mut edits: Vec<(usize, usize, &str)> = parts
            .iter()
            .map(|&(anchor, sel, del, ins)| {
                let sel = usize::from(sel);
                let p = match (anchor % 5, recs.is_empty()) {
                    (1, false) => recs[sel % recs.len()].0,
                    (2, false) => recs[sel % recs.len()].1,
                    (3, _) => {
                        if sel % 2 == 0 {
                            0
                        } else {
                            len
                        }
                    }
                    _ => sel % (len + 1),
                };
                let s = floor_boundary(t, p);
                // 削除は 0〜3 文字 (前へ広げる場合も含む)
                let n = usize::from(del % 4);
                let (s, e) = if del & 0x80 != 0 && n > 0 {
                    let c = t.byte_to_char(ByteOff(s)).0;
                    (t.char_to_byte(CharIdx(c.saturating_sub(n))).0, s)
                } else {
                    let c = t.byte_to_char(ByteOff(s)).0;
                    (s, t.char_to_byte(CharIdx(c + n)).0.min(len))
                };
                (s, e, INSERTS[usize::from(ins) % INSERTS.len()])
            })
            .collect();
        edits.sort_by_key(|e| (e.0, e.1));
        // 重なる編集は捨てる (接するものは残す)
        let mut ok: Vec<(usize, usize, &str)> = Vec::new();
        for e in edits {
            if ok.last().is_none_or(|l| e.0 >= l.1) {
                ok.push(e);
            }
        }
        let cs = ChangeSet::from_sorted_edits(
            t,
            ok.iter().map(|&(s, e, x)| Edit {
                range: ByteRange::new(ByteOff(s), ByteOff(e)),
                insert: x.into(),
            }),
        )
        .expect("妥当な編集");
        (!cs.is_identity()).then_some(cs)
    }

    fn edit(&mut self, cs: &ChangeSet, merge: bool, coarse: bool) -> Result<(), TestCaseError> {
        let pos = self.undo.len();
        let (naive_after, naive_removed) = naive_edit(&self.timeline[pos], cs);
        let hunks: Vec<_> = cs.changed_ranges().collect();
        let coarse_old = (coarse && hunks.len() >= 2)
            .then(|| ByteRange::new(hunks[0].0.start, hunks[hunks.len() - 1].0.end));
        let hull = coarse_old.map(|old| self.map.affected_hull(old));
        let before_spans = hull.map(|h| self.map.spans_in(h));
        let snap = self.text.snapshot();
        let removed = self.map.apply_edit(cs);
        prop_assert_eq!(&removed, &naive_removed, "外した記録が素朴な実装と同じ");
        self.text.apply(cs).expect("適用できる");
        let coarse_inv = hull
            .is_some()
            .then(|| crate::document::coarse_inverse(cs, &snap, &self.text))
            .flatten();
        let inverse = coarse_inv.clone().unwrap_or_else(|| cs.invert(&snap));
        let coarse = match (coarse_inv, hull, before_spans) {
            (Some(_), Some(h), Some(before)) => Some(Coarse {
                hull_new: ByteRange::new(
                    h.start,
                    ByteOff(h.end.0 + cs.len_after() - cs.len_before()),
                ),
                old_len: h.len(),
                before,
            }),
            _ => None,
        };
        // 新しい編集は redo を捨てる
        self.redo.clear();
        self.timeline.truncate(pos + 1);
        let can_merge =
            merge && coarse.is_none() && self.undo.last().is_some_and(|n| n.coarse.is_none());
        if can_merge && let Some(last) = self.undo.last_mut() {
            // ノードの結合 (03 §7.3): redo = redo.compose(new)、undo = new_inverse.compose(undo)、外した記録も合わせる
            let redo = last.redo.compose(cs).expect("合成できる");
            let undo = inverse.compose(&last.undo).expect("合成できる");
            let older = std::mem::take(&mut last.removed);
            last.removed = merge_removed(older, removed, &last.undo);
            last.redo = redo;
            last.undo = undo;
            self.timeline[pos] = naive_after;
        } else {
            self.undo.push(Node {
                redo: cs.clone(),
                undo: inverse,
                removed,
                coarse,
            });
            self.timeline.push(naive_after);
        }
        self.check()
    }

    fn undo_one(&mut self) -> Result<bool, TestCaseError> {
        let Some(node) = self.undo.pop() else {
            return Ok(false);
        };
        match &node.coarse {
            Some(c) => {
                self.map.replace_range(c.hull_new, c.old_len, &c.before);
            }
            None => {
                let dropped = self.map.apply_undo(&node.undo, &node.removed);
                prop_assert!(dropped.is_empty(), "Undo で外れる記録は無い: {:?}", dropped);
            }
        }
        self.text.apply(&node.undo).expect("適用できる");
        self.redo.push(node);
        self.check()?;
        Ok(true)
    }

    fn redo_one(&mut self) -> Result<bool, TestCaseError> {
        let Some(node) = self.redo.pop() else {
            return Ok(false);
        };
        let odd = self.map.apply_redo(&node.redo, &node.removed);
        prop_assert!(odd.is_empty(), "Redo で食い違う記録は無い: {:?}", odd);
        self.text.apply(&node.redo).expect("適用できる");
        self.undo.push(node);
        self.check()?;
        Ok(true)
    }
}

fn run(
    enc: Encoding,
    bytes: &[u8],
    limits: (u8, u8),
    acts: &[Action],
) -> Result<(), TestCaseError> {
    let mut d = Decoder::new(enc, false);
    let mut s = String::new();
    d.decode(bytes, true, &mut s);
    let (_, recs) = check_roundtrip_with_records(enc, bytes);
    let text = Text::from(s.as_str());
    let mut map = PreservedMap::from_records(&recs, enc, &text)
        .map_err(|e| TestCaseError::fail(format!("照合が通る: {e}")))?;
    prop_assert_eq!(map.len(), recs.records.len());
    // 塊を小さくして、塊の境をまたぐ編集を通す
    map.rechunk(ChunkLimits {
        max: [1usize, 2, 3, 5, 8, CHUNK_MAX][usize::from(limits.0) % 6],
        max_bytes: [3usize, 8, CHUNK_MAX_BYTES][usize::from(limits.1) % 3],
    });
    let initial_text = s;
    let initial = map.to_spans();
    let mut w = World {
        enc,
        text,
        map,
        undo: Vec::new(),
        redo: Vec::new(),
        timeline: vec![initial.clone()],
        copies: Vec::new(),
    };
    w.check()?;
    for (kind, parts, extra) in acts {
        match kind % 10 {
            0..=5 | 9 => {
                if let Some(cs) = w.make_cs(parts) {
                    w.edit(&cs, kind % 10 == 9, extra & 1 != 0)?;
                }
            }
            6 | 7 => {
                w.undo_one()?;
            }
            _ => {
                w.redo_one()?;
            }
        }
        if extra & 2 != 0 {
            w.copies.push((w.map.clone(), w.map.to_spans()));
        }
    }
    // (d) 全部 Undo すると読込の直後の表に戻る。全部 Redo すると最後の表に戻る
    let depth = w.undo.len() + w.redo.len();
    prop_assert_eq!(w.timeline.len(), depth + 1);
    while w.undo_one()? {}
    prop_assert_eq!(&w.map.to_spans(), &initial);
    prop_assert_eq!(all(&w.text), initial_text);
    while w.redo_one()? {}
    prop_assert_eq!(w.undo.len(), depth);
    prop_assert_eq!(&w.map.to_spans(), &w.timeline[depth]);
    // 複製した版は、その後の編集の影響を受けない
    for (copy, spans) in &w.copies {
        prop_assert_eq!(&copy.to_spans(), spans);
        prop_assert_eq!(copy.check_structure(), Ok(()));
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 768, ..ProptestConfig::default() })]

    #[test]
    fn 編集とundo_redoで表が素朴な実装と一致し不変条件を保つ(
        (enc, bytes) in doc(),
        limits in (any::<u8>(), any::<u8>()),
        acts in actions(),
    ) {
        run(enc, &bytes, limits, &acts)?;
    }
}
