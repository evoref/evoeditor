//! `Range` / `Selection` (docs/03-core-design.md §4)。純データと、ChangeSet による写像。
//!
//! N1 (書記素境界へのスナップ) は `normalized`、N2〜N6 は `merged`。写像は `map`。

use crate::changeset::{Bias, ChangeSet};
use crate::grapheme::{snap_to_grapheme, snap_to_nearest_grapheme};
use crate::text::TextRead;
use crate::types::{ByteOff, Pos};

/// 選択範囲。`anchor == head` なら選択なしのカーソル。半開区間 `[start, end)` が選択内容で、向きを保持する。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Range {
    pub anchor: Pos,
    pub head: Pos,
}

impl Range {
    pub fn new(anchor: Pos, head: Pos) -> Range {
        Range { anchor, head }
    }

    /// 選択なしのカーソル。
    pub fn point(pos: Pos) -> Range {
        Range {
            anchor: pos,
            head: pos,
        }
    }

    pub fn start(&self) -> Pos {
        self.anchor.min(self.head)
    }

    pub fn end(&self) -> Pos {
        self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// `head >= anchor`。
    pub fn is_forward(&self) -> bool {
        self.head >= self.anchor
    }

    /// 選択拡張 (Shift + 移動): anchor は動かさない。
    pub fn with_head(self, head: Pos) -> Range {
        Range {
            anchor: self.anchor,
            head,
        }
    }

    /// 移動 (Shift なし): head に潰す。
    pub fn collapse_to_head(self) -> Range {
        Range::point(self.head)
    }

    /// `ChangeSet` による写像。anchor と head を同じ `bias` で写す。
    pub fn map(&self, cs: &ChangeSet, bias: Bias) -> Range {
        Range {
            anchor: cs.map_pos(self.anchor, bias),
            head: cs.map_pos(self.head, bias),
        }
    }
}

/// `Selection::map` の方針 (03 §6.4)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapPolicy {
    /// 自分の編集: 挿入位置に一致する点は挿入の後ろ (`Bias::After`)。
    Own,
    /// 他者の編集 (他 View・外部変更): 挿入位置に一致する点は前に留まる (`Bias::Before`)。
    Other,
}

impl MapPolicy {
    fn bias(self) -> Bias {
        match self {
            MapPolicy::Own => Bias::After,
            MapPolicy::Other => Bias::Before,
        }
    }
}

/// 複数の `Range`。非空で、`primary` はそのうちの 1 つを指す。
/// v0.1〜v0.3 は 1 つに制限されるが、編集コマンドは複数を前提に書く (03 §4.5)。
/// 内部は `Vec` (仕様の `SmallVec` は依存を増やすので使わない。1 要素の確保は View ごとの低頻度)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    ranges: Vec<Range>,
    primary: usize,
}

impl Selection {
    pub fn single(range: Range) -> Selection {
        Selection {
            ranges: vec![range],
            primary: 0,
        }
    }

    pub fn point(pos: Pos) -> Selection {
        Selection::single(Range::point(pos))
    }

    /// `ranges` が空、または `primary` が範囲外なら `None` (N5: 空の Selection は作れない)。
    /// 並べ替えや併合はしない (`merged` を通す)。
    pub fn new(ranges: Vec<Range>, primary: usize) -> Option<Selection> {
        (primary < ranges.len()).then_some(Selection { ranges, primary })
    }

    pub fn primary(&self) -> &Range {
        &self.ranges[self.primary]
    }

    pub fn primary_index(&self) -> usize {
        self.primary
    }

    /// 並べ替え・併合の後は `start` 昇順。
    pub fn iter(&self) -> impl Iterator<Item = &Range> {
        self.ranges.iter()
    }

    pub fn ranges(&self) -> &[Range] {
        &self.ranges
    }

    /// 常に 1 以上。
    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    /// N5 により常に `false` (clippy の `len_without_is_empty` のために置く)。
    pub fn is_empty(&self) -> bool {
        false
    }

    /// N2〜N4 と N6: 位置を `len_bytes` に収め、`start` 昇順に並べ、重なる Range を併合する。
    /// 書記素境界へのスナップ (N1) は含まない。
    ///
    /// - 併合するのは、重なる Range、両方が非空で接する Range、同じ位置の空のカーソル同士 (N3)。
    /// - 空のカーソルが非空の Range の `[start, end]` (両端を含む) にあれば捨てる (N4)。
    ///   `end` の位置も含めるのは、選択の端に重なるカーソルは見た目に冗長だから (03 §4.4 N4 の「同じ位置」の解釈)。
    ///   primary の空のカーソルは、Range の内側にあれば Range に併合し (その Range が primary になる)、端なら残す。
    /// - 併合後の向きは primary を含むときは primary の向き、含まないときは先頭の向き。
    ///   空の Range だけの併合は空のカーソル。
    pub fn merged(self, len_bytes: usize) -> Selection {
        let clamp = |p: Pos| ByteOff(p.0.min(len_bytes));
        let (mut full, mut points): (Vec<Tagged>, Vec<Tagged>) = self
            .ranges
            .iter()
            .enumerate()
            .map(|(i, r)| {
                (
                    Range::new(clamp(r.anchor), clamp(r.head)),
                    i == self.primary,
                )
            })
            .partition(|(r, _)| !r.is_empty());

        // 1. 非空の Range: 重なるもの、接するものを併合する (安定ソートで、同じ start なら入力順)
        full.sort_by_key(|(r, _)| (r.start(), r.end()));
        let mut merged: Vec<(Range, bool)> = Vec::with_capacity(full.len());
        for (r, is_primary) in full {
            match merged.last_mut() {
                Some((cur, cur_primary)) if r.start() <= cur.end() => {
                    let forward = if *cur_primary || !is_primary {
                        cur.is_forward()
                    } else {
                        r.is_forward()
                    };
                    *cur = union(cur, &r, forward);
                    *cur_primary |= is_primary;
                }
                _ => merged.push((r, is_primary)),
            }
        }

        // 2. 空のカーソル: 同じ位置は 1 つに、非空の Range の `[start, end]` にあれば捨てる (N4)。
        //    primary のカーソルは、内側 (`start < p < end`) なら Range に併合し、端なら残す。
        points.sort_by_key(|(r, _)| r.start());
        let mut cursors: Vec<(Range, bool)> = Vec::new();
        for (r, is_primary) in points {
            let p = r.start();
            let k = merged.partition_point(|(m, _)| m.end() < p);
            match merged.get_mut(k) {
                Some((m, m_primary)) if m.start() <= p => {
                    if is_primary && m.start() < p && p < m.end() {
                        *m_primary = true;
                    } else if is_primary {
                        cursors.push((r, true));
                    }
                }
                _ => match cursors.last_mut() {
                    Some((c, c_primary)) if c.start() == p => *c_primary |= is_primary,
                    _ => cursors.push((r, is_primary)),
                },
            }
        }

        // 3. 位置順に並べる。非空と空は、start の小さい方 (同じ start なら空が先) に並ぶ
        merged.extend(cursors);
        merged.sort_by_key(|(r, _)| (r.start(), r.end()));
        let primary = merged.iter().position(|&(_, p)| p).unwrap_or(0);
        Selection {
            ranges: merged.into_iter().map(|(r, _)| r).collect(),
            primary,
        }
    }

    /// N1〜N6 のすべて: 各 Range の anchor / head を近い書記素境界へ寄せ (同じ距離なら前)、`merged` を通す。
    /// 移動の向きを持つスナップ (head を移動方向の `Bias` に寄せる) は `normalized_with_head_bias` を使う。
    pub fn normalized(self, text: &dyn TextRead) -> Selection {
        let snap = |r: &Range| {
            Range::new(
                snap_to_nearest_grapheme(text, r.anchor),
                snap_to_nearest_grapheme(text, r.head),
            )
        };
        let ranges = self.ranges.iter().map(snap).collect();
        Selection {
            ranges,
            primary: self.primary,
        }
        .merged(text.len_bytes())
    }

    /// `normalized` と同じだが、head は `head_bias` の側の書記素境界へ寄せる (anchor は近い方)。
    pub fn normalized_with_head_bias(self, text: &dyn TextRead, head_bias: Bias) -> Selection {
        let snap = |r: &Range| {
            Range::new(
                snap_to_nearest_grapheme(text, r.anchor),
                snap_to_grapheme(text, r.head, head_bias),
            )
        };
        let ranges = self.ranges.iter().map(snap).collect();
        Selection {
            ranges,
            primary: self.primary,
        }
        .merged(text.len_bytes())
    }

    /// 各 Range に移動 `f` を適用して `normalized` する。
    pub fn transform(&self, text: &dyn TextRead, f: impl Fn(Range) -> Range) -> Selection {
        let ranges = self.ranges.iter().map(|r| f(*r)).collect();
        Selection {
            ranges,
            primary: self.primary,
        }
        .normalized(text)
    }

    /// `ChangeSet` による写像 (03 §6.4)。元の `ChangeSet` を順に使う (合成したものは使わない)。
    /// 結果は N2〜N6 を満たす。
    pub fn map(&self, cs: &ChangeSet, policy: MapPolicy) -> Selection {
        let bias = policy.bias();
        let ranges = self.ranges.iter().map(|r| r.map(cs, bias)).collect();
        Selection {
            ranges,
            primary: self.primary,
        }
        .merged(cs.len_after())
    }
}

/// primary かどうかの印を付けた Range (併合の作業用)。
type Tagged = (Range, bool);

fn union(a: &Range, b: &Range, forward: bool) -> Range {
    let start = a.start().min(b.start());
    let end = a.end().max(b.end());
    if start == end {
        Range::point(start)
    } else if forward {
        Range::new(start, end)
    } else {
        Range::new(end, start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changeset::{ChangeSet, Edit};
    use crate::text::Text;
    use crate::types::{ByteOff, ByteRange};

    fn r(a: usize, h: usize) -> Range {
        Range::new(ByteOff(a), ByteOff(h))
    }

    fn sel(rs: &[(usize, usize)], primary: usize) -> Selection {
        Selection::new(rs.iter().map(|&(a, h)| r(a, h)).collect(), primary).unwrap()
    }

    fn pairs(s: &Selection) -> Vec<(usize, usize)> {
        s.iter().map(|r| (r.anchor.0, r.head.0)).collect()
    }

    #[test]
    fn range_の基本() {
        let x = r(5, 2);
        assert_eq!((x.start(), x.end()), (ByteOff(2), ByteOff(5)));
        assert!(!x.is_forward() && !x.is_empty());
        assert_eq!(x.with_head(ByteOff(9)), r(5, 9));
        assert_eq!(x.collapse_to_head(), Range::point(ByteOff(2)));
        assert!(Range::point(ByteOff(3)).is_forward());
    }

    #[test]
    fn 空の_selection_は作れない() {
        assert!(Selection::new(vec![], 0).is_none());
        assert!(Selection::new(vec![r(0, 0)], 1).is_none());
    }

    #[test]
    fn 並べ替えても_primary_は同じ_range_を指す() {
        let s = sel(&[(10, 12), (0, 3)], 0).merged(100);
        assert_eq!(pairs(&s), [(0, 3), (10, 12)]);
        assert_eq!(*s.primary(), r(10, 12));
    }

    #[test]
    fn 重なる_range_と接する非空は併合する() {
        assert_eq!(pairs(&sel(&[(0, 5), (3, 8)], 0).merged(100)), [(0, 8)]);
        assert_eq!(
            pairs(&sel(&[(0, 5), (5, 8)], 0).merged(100)),
            [(0, 8)],
            "両方非空で接する"
        );
        assert_eq!(
            pairs(&sel(&[(0, 5), (6, 8)], 0).merged(100)),
            [(0, 5), (6, 8)],
            "離れている"
        );
        assert_eq!(
            pairs(&sel(&[(1, 9), (3, 4)], 0).merged(100)),
            [(1, 9)],
            "包含"
        );
    }

    #[test]
    fn 併合後の向きは_primary_を優先() {
        // primary (後ろ向き 8→3) と先頭 (前向き)
        let s = sel(&[(0, 5), (8, 3)], 1).merged(100);
        assert_eq!(pairs(&s), [(8, 0)], "primary の向き (後ろ向き)");
        let s = sel(&[(0, 5), (8, 3)], 0).merged(100);
        assert_eq!(pairs(&s), [(0, 8)], "primary は先頭 (前向き)");
        // primary を含まない併合は先頭の向き
        let s = sel(&[(5, 0), (3, 8), (20, 21)], 2).merged(100);
        assert_eq!(pairs(&s), [(8, 0), (20, 21)]);
        assert_eq!(*s.primary(), r(20, 21));
    }

    #[test]
    fn 同じ位置の空のカーソルは_1_つになる() {
        let s = sel(&[(4, 4), (4, 4), (7, 7)], 1).merged(100);
        assert_eq!(pairs(&s), [(4, 4), (7, 7)]);
        assert_eq!(s.primary_index(), 0, "primary を含む側に付く");
    }

    #[test]
    fn 空のカーソルは非空の_range_の端と内側で捨てる() {
        assert_eq!(
            pairs(&sel(&[(2, 8), (5, 5)], 0).merged(100)),
            [(2, 8)],
            "内側"
        );
        assert_eq!(
            pairs(&sel(&[(2, 8), (8, 8)], 0).merged(100)),
            [(2, 8)],
            "end"
        );
        assert_eq!(
            pairs(&sel(&[(2, 8), (2, 2)], 0).merged(100)),
            [(2, 8)],
            "start"
        );
        assert_eq!(
            pairs(&sel(&[(2, 8), (9, 9)], 0).merged(100)),
            [(2, 8), (9, 9)],
            "離れている"
        );
        // primary が空のカーソルなら残す
        let s = sel(&[(2, 8), (8, 8)], 1).merged(100);
        assert_eq!(*s.primary(), r(8, 8));
    }

    #[test]
    fn 位置は文書の長さに収める() {
        let s = sel(&[(5, 50)], 0).merged(10);
        assert_eq!(pairs(&s), [(5, 10)]);
        let s = sel(&[(50, 60)], 0).merged(10);
        assert_eq!(pairs(&s), [(10, 10)], "潰れて空のカーソルになる");
    }

    #[test]
    fn 自分の入力は後ろに_他者の入力は前に写る() {
        let base = Text::from("abcdef");
        let snap = base.snapshot();
        let cs = ChangeSet::from_sorted_edits(
            &snap,
            [Edit {
                range: ByteRange::from(3..3),
                insert: "XY".into(),
            }],
        )
        .unwrap();
        let s = Selection::point(ByteOff(3));
        assert_eq!(s.map(&cs, MapPolicy::Own).primary().head, ByteOff(5));
        assert_eq!(s.map(&cs, MapPolicy::Other).primary().head, ByteOff(3));
    }

    #[test]
    fn 選択範囲への入力は入力した文字の後ろへ潰れる() {
        let base = Text::from("0123456789");
        let snap = base.snapshot();
        let cs = ChangeSet::from_sorted_edits(
            &snap,
            [Edit {
                range: ByteRange::from(2..5),
                insert: "X".into(),
            }],
        )
        .unwrap();
        let s = Selection::single(r(2, 5));
        let m = s.map(&cs, MapPolicy::Own);
        assert_eq!(pairs(&m), [(3, 3)], "03 §6.4 の例: head = 5 は 3 に写る");
    }

    #[test]
    fn 削除で重なった_range_は併合される() {
        let base = Text::from("0123456789");
        let snap = base.snapshot();
        let cs = ChangeSet::from_sorted_edits(
            &snap,
            [Edit {
                range: ByteRange::from(2..8),
                insert: "".into(),
            }],
        )
        .unwrap();
        let s = sel(&[(3, 4), (6, 7), (9, 10)], 1);
        let m = s.map(&cs, MapPolicy::Other);
        assert_eq!(
            pairs(&m),
            [(2, 2), (3, 4)],
            "(3,4) と (6,7) は (2,2) に潰れて 1 つ、(9,10) は 3,4 へ"
        );
        assert_eq!(m.primary_index(), 0);
    }

    #[test]
    fn 正規化は書記素境界に寄せて併合する() {
        let base = Text::from("a\r\nb");
        let s = sel(&[(2, 4)], 0).normalized(&base);
        assert_eq!(pairs(&s), [(1, 4)], "CR と LF の間は前 (同じ距離) に寄る");
        // 寄せた結果、重なる Range は併合される
        let t = Text::from("e\u{301}e\u{301}");
        let s = sel(&[(0, 4), (3, 5)], 0).normalized(&t);
        assert_eq!(
            pairs(&s),
            [(0, 6)],
            "(0,3) と (3,6) に寄って接し、1 つになる"
        );
        let s = sel(&[(1, 1)], 0).normalized_with_head_bias(&t, Bias::After);
        assert_eq!(
            pairs(&s),
            [(0, 3)],
            "anchor は近い方 (0)、head は After (3)"
        );
    }

    #[test]
    fn 変換は移動の後に正規化する() {
        let base = Text::from("a\r\nb");
        let s = Selection::point(ByteOff(0))
            .transform(&base, |r| r.with_head(ByteOff(2)).collapse_to_head());
        assert_eq!(pairs(&s), [(1, 1)]);
    }
}
