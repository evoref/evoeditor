//! ペインの矩形の計算 (docs/03 §9.9 の PANE-3 / PANE-4)。状態から切り離した純関数で、レイアウトの木と編集面の大きさだけから
//! 葉の矩形・見出し・区切りを求める。
//!
//! spec: 03 §9.9 (PANE-3, PANE-4, PANE-10)

use eve_view::{CellRect, CellSize};

use crate::editor::window::{Axis, LayoutNode, PaneId};

/// ペインの最小の大きさ: 本文 (ガターを含む) が 20 列 × 3 行と、見出しの 1 行 (PANE-4。定数で、設定にしない。
/// 04 §3.5 の編集面の最小幅 20 列に揃える)。
pub(crate) const PANE_MIN: CellSize = CellSize { cols: 20, rows: 4 };

/// 1 つの葉 (ペイン) の配置。座標は編集面の左上が原点 (PANE-10 (a))。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LeafRect {
    /// ペイン。
    pub pane: PaneId,
    /// 本文 (ガター + テキスト) の矩形。見出しの行を含まない。
    pub body: CellRect,
    /// 見出しの行。葉が 2 つ以上の木だけ `Some` (葉が 1 つの木は見出しを出さない)。
    pub bar: Option<CellRect>,
}

/// 木の全体の配置。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PaneLayout {
    /// 葉を木の葉の順 (深さ優先。`Row` は左から、`Column` は上から) に並べたもの。
    pub leaves: Vec<LeafRect>,
    /// `Row` の子の間の区切り (1 列。高さは親の分割の矩形の全体で、見出しの行を含む)。
    pub dividers: Vec<CellRect>,
}

/// 木の最小の大きさ (PANE-4): 葉が (20, 4)、`Row` は列が子の和 + 区切りの数・行が子の最大、`Column` は列が子の最大・
/// 行が子の和。
pub(crate) fn min_size(node: &LayoutNode) -> CellSize {
    match node {
        LayoutNode::Leaf(_) => PANE_MIN,
        LayoutNode::Split { axis, children } => {
            let mins = children.iter().map(|(c, _)| min_size(c));
            let gaps = u16::try_from(children.len().saturating_sub(1)).unwrap_or(u16::MAX);
            match axis {
                Axis::Row => mins.fold(
                    CellSize {
                        cols: gaps,
                        rows: 0,
                    },
                    |acc, m| CellSize {
                        cols: acc.cols.saturating_add(m.cols),
                        rows: acc.rows.max(m.rows),
                    },
                ),
                Axis::Column => mins.fold(CellSize::default(), |acc, m| CellSize {
                    cols: acc.cols.max(m.cols),
                    rows: acc.rows.saturating_add(m.rows),
                }),
            }
        }
    }
}

/// 木を編集面 `size` に配置する (PANE-3)。葉が 2 つ以上なら、各葉の最下行を見出しにする。
///
/// 編集面が木の最小 ([`min_size`]) より小さいときも矩形は作る (子を最小にしてから、はみ出す分を後ろの子から削る)。
/// そのときの表示 (縮退表示、PANE-4 (b)) は呼び側が決める。
pub(crate) fn layout(node: &LayoutNode, size: CellSize) -> PaneLayout {
    let mut out = PaneLayout::default();
    let bars = !matches!(node, LayoutNode::Leaf(_));
    let area = CellRect {
        col: 0,
        row: 0,
        cols: size.cols,
        rows: size.rows,
    };
    place(node, area, bars, &mut out);
    out
}

fn place(node: &LayoutNode, area: CellRect, bars: bool, out: &mut PaneLayout) {
    match node {
        LayoutNode::Leaf(pane) => {
            let (body, bar) = if bars && area.rows > 0 {
                let body_rows = area.rows - 1;
                (
                    CellRect {
                        rows: body_rows,
                        ..area
                    },
                    Some(CellRect {
                        row: area.row.saturating_add(body_rows),
                        rows: 1,
                        ..area
                    }),
                )
            } else {
                (area, bars.then_some(CellRect { rows: 0, ..area }))
            };
            out.leaves.push(LeafRect {
                pane: *pane,
                body,
                bar,
            });
        }
        LayoutNode::Split { axis, children } => {
            let n = children.len();
            let ratios: Vec<u16> = children.iter().map(|(_, r)| *r).collect();
            let mins: Vec<u16> = children
                .iter()
                .map(|(c, _)| {
                    let m = min_size(c);
                    match axis {
                        Axis::Row => m.cols,
                        Axis::Column => m.rows,
                    }
                })
                .collect();
            let gaps = match axis {
                Axis::Row => u16::try_from(n.saturating_sub(1)).unwrap_or(u16::MAX),
                Axis::Column => 0,
            };
            let length = match axis {
                Axis::Row => area.cols,
                Axis::Column => area.rows,
            };
            let sizes = distribute(length.saturating_sub(gaps), &ratios, &mins);
            let mut pos = match axis {
                Axis::Row => area.col,
                Axis::Column => area.row,
            };
            let end = pos.saturating_add(length);
            for (i, ((child, _), len)) in children.iter().zip(sizes).enumerate() {
                let len = len.min(end.saturating_sub(pos));
                let rect = match axis {
                    Axis::Row => CellRect {
                        col: pos,
                        cols: len,
                        ..area
                    },
                    Axis::Column => CellRect {
                        row: pos,
                        rows: len,
                        ..area
                    },
                };
                place(child, rect, bars, out);
                pos = pos.saturating_add(len);
                if *axis == Axis::Row && i + 1 < n && pos < end {
                    out.dividers.push(CellRect {
                        col: pos,
                        cols: 1,
                        ..area
                    });
                    pos = pos.saturating_add(1);
                }
            }
        }
    }
}

/// `total` を比率 `ratios` で配る (PANE-3): 各子に `floor(total × 比率 / 比率の和)` を配り、余りを小数部の大きい子から
/// 1 つずつ (同じなら前の子) 配る。最小 `mins` を割る子があれば、その子を最小にし、残りをほかの子で比率により配り直す
/// (子の数の回数で止まる)。最小の和が `total` を超えるときは、和が `total` を超えたまま返す (呼び側が端で切る)。
pub(crate) fn distribute(total: u16, ratios: &[u16], mins: &[u16]) -> Vec<u16> {
    let n = ratios.len();
    let mut fixed: Vec<Option<u16>> = vec![None; n];
    let mut sizes = vec![0u16; n];
    for _ in 0..=n {
        let free: Vec<usize> = (0..n).filter(|&i| fixed[i].is_none()).collect();
        let used: u32 = fixed.iter().flatten().map(|&v| u32::from(v)).sum();
        let remaining = u32::from(total).saturating_sub(used);
        let alloc = by_ratio(
            remaining,
            &free.iter().map(|&i| ratios[i]).collect::<Vec<_>>(),
        );
        let mut changed = false;
        for (&i, &a) in free.iter().zip(&alloc) {
            sizes[i] = a;
            let min = mins.get(i).copied().unwrap_or(0);
            if a < min {
                fixed[i] = Some(min);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    for (s, f) in sizes.iter_mut().zip(&fixed) {
        if let Some(v) = f {
            *s = *v;
        }
    }
    sizes
}

/// `total` を比率で配る (最大剰余法。同じ剰余は前の子を先にする)。比率の和が 0 なら等分する。
fn by_ratio(total: u32, ratios: &[u16]) -> Vec<u16> {
    let n = ratios.len();
    if n == 0 {
        return Vec::new();
    }
    let ratio_sum: u64 = ratios.iter().map(|&r| u64::from(r)).sum();
    let (weights, sum): (Vec<u64>, u64) = if ratio_sum == 0 {
        (vec![1; n], u64::try_from(n).unwrap_or(u64::MAX))
    } else {
        (ratios.iter().map(|&r| u64::from(r)).collect(), ratio_sum)
    };
    let total = u64::from(total);
    let mut base: Vec<u64> = weights.iter().map(|&w| total * w / sum).collect();
    let mut rems: Vec<(u64, usize)> = weights
        .iter()
        .enumerate()
        .map(|(i, &w)| (total * w % sum, i))
        .collect();
    let given: u64 = base.iter().sum();
    let mut left = total.saturating_sub(given);
    // 剰余の大きい順 (同じなら前の子)
    rems.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    for (_, i) in rems {
        if left == 0 {
            break;
        }
        base[i] += 1;
        left -= 1;
    }
    base.into_iter()
        .map(|b| u16::try_from(b).unwrap_or(u16::MAX))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(n: u64) -> LayoutNode {
        LayoutNode::Leaf(PaneId(n))
    }

    fn split(axis: Axis, children: Vec<(LayoutNode, u16)>) -> LayoutNode {
        LayoutNode::Split { axis, children }
    }

    fn rect(col: u16, row: u16, cols: u16, rows: u16) -> CellRect {
        CellRect {
            col,
            row,
            cols,
            rows,
        }
    }

    fn size(cols: u16, rows: u16) -> CellSize {
        CellSize { cols, rows }
    }

    #[test]
    fn 葉が_1_つなら編集面の全体で見出しも区切りも無い() {
        let l = layout(&leaf(1), size(80, 23));
        assert_eq!(
            l,
            PaneLayout {
                leaves: vec![LeafRect {
                    pane: PaneId(1),
                    body: rect(0, 0, 80, 23),
                    bar: None,
                }],
                dividers: vec![],
            }
        );
        assert_eq!(min_size(&leaf(1)), size(20, 4));
    }

    #[test]
    fn 左右の_2_つは区切りを_1_列置いて残りを比率で配る() {
        // 81 列 − 区切り 1 = 80 を 5,000 / 5,000 で配る
        let t = split(Axis::Row, vec![(leaf(1), 5_000), (leaf(2), 5_000)]);
        let l = layout(&t, size(81, 24));
        assert_eq!(
            l.leaves,
            vec![
                LeafRect {
                    pane: PaneId(1),
                    body: rect(0, 0, 40, 23),
                    bar: Some(rect(0, 23, 40, 1)),
                },
                LeafRect {
                    pane: PaneId(2),
                    body: rect(41, 0, 40, 23),
                    bar: Some(rect(41, 23, 40, 1)),
                },
            ]
        );
        assert_eq!(l.dividers, vec![rect(40, 0, 1, 24)]);
        assert_eq!(min_size(&t), size(41, 4));
    }

    #[test]
    fn 余りは小数部の大きい子から_同じなら前の子へ() {
        // 80 列 − 区切り 1 = 79 を半分ずつ: 39.5 / 39.5 → 前の子が 40
        let t = split(Axis::Row, vec![(leaf(1), 5_000), (leaf(2), 5_000)]);
        let l = layout(&t, size(80, 10));
        assert_eq!(l.leaves[0].body.cols, 40);
        assert_eq!(l.leaves[1].body, rect(41, 0, 39, 9));
        // 3 つを等分に近く (3,334 / 3,333 / 3,333) で 100 行: 33.34 / 33.33 / 33.33 → 34 / 33 / 33
        assert_eq!(
            distribute(100, &[3_334, 3_333, 3_333], &[0, 0, 0]),
            vec![34, 33, 33]
        );
        // 小数部の大きいほうが余りを取る: 10 を 2,500 / 7,500 → 2.5 / 7.5 → 同じ小数部なので前の子
        assert_eq!(distribute(10, &[2_500, 7_500], &[0, 0]), vec![3, 7]);
        assert_eq!(distribute(10, &[1_600, 8_400], &[0, 0]), vec![2, 8]);
    }

    #[test]
    fn 上下は区切りを置かず_上の子の見出しが区切りになる() {
        let t = split(Axis::Column, vec![(leaf(1), 5_000), (leaf(2), 5_000)]);
        let l = layout(&t, size(80, 24));
        assert_eq!(
            l.leaves,
            vec![
                LeafRect {
                    pane: PaneId(1),
                    body: rect(0, 0, 80, 11),
                    bar: Some(rect(0, 11, 80, 1)),
                },
                LeafRect {
                    pane: PaneId(2),
                    body: rect(0, 12, 80, 11),
                    bar: Some(rect(0, 23, 80, 1)),
                },
            ]
        );
        assert_eq!(l.dividers, Vec::<CellRect>::new());
        assert_eq!(min_size(&t), size(20, 8));
    }

    #[test]
    fn 入れ子は葉の順に並び_区切りは親の分割の高さ() {
        // 左に 1、右を上下に 2 / 3 で分ける
        let t = split(
            Axis::Row,
            vec![
                (leaf(1), 5_000),
                (
                    split(Axis::Column, vec![(leaf(2), 5_000), (leaf(3), 5_000)]),
                    5_000,
                ),
            ],
        );
        let l = layout(&t, size(61, 20));
        let order: Vec<PaneId> = l.leaves.iter().map(|x| x.pane).collect();
        assert_eq!(order, vec![PaneId(1), PaneId(2), PaneId(3)]);
        assert_eq!(l.leaves[0].body, rect(0, 0, 30, 19));
        assert_eq!(l.leaves[1].body, rect(31, 0, 30, 9));
        assert_eq!(l.leaves[1].bar, Some(rect(31, 9, 30, 1)));
        assert_eq!(l.leaves[2].body, rect(31, 10, 30, 9));
        assert_eq!(l.leaves[2].bar, Some(rect(31, 19, 30, 1)));
        assert_eq!(l.dividers, vec![rect(30, 0, 1, 20)]);
        // 最小: 列は 20 + 1 + 20、行は max(4, 4 + 4)
        assert_eq!(min_size(&t), size(41, 8));
    }

    #[test]
    fn 最小を割る子は最小にして_ほかの子から比率で引き直す() {
        // 50 列 − 区切り 1 = 49 を 1,000 / 9,000 → 4.9 / 44.1。左が最小 20 を割るので 20 にし、残り 29 を右へ
        let t = split(Axis::Row, vec![(leaf(1), 1_000), (leaf(2), 9_000)]);
        let l = layout(&t, size(50, 10));
        assert_eq!(l.leaves[0].body, rect(0, 0, 20, 9));
        assert_eq!(l.leaves[1].body, rect(21, 0, 29, 9));
        // 3 つで、2 つが最小を割る: 100 を 500 / 500 / 9,000 → 5 / 5 / 90 → 20 / 20 / 60
        assert_eq!(
            distribute(100, &[500, 500, 9_000], &[20, 20, 20]),
            vec![20, 20, 60]
        );
        // 引き直して別の子が最小を割る: 60 を 1,000 / 4,500 / 4,500 → 6 → 20、残り 40 を 4,500 / 4,500 → 20 / 20
        assert_eq!(
            distribute(60, &[1_000, 4_500, 4_500], &[20, 20, 20]),
            vec![20, 20, 20]
        );
    }

    #[test]
    fn 編集面が最小より小さくても矩形は面の中に収める() {
        let t = split(Axis::Row, vec![(leaf(1), 5_000), (leaf(2), 5_000)]);
        let l = layout(&t, size(30, 3));
        for x in &l.leaves {
            assert!(x.body.col + x.body.cols <= 30, "{x:?}");
            assert!(
                x.body.row + x.body.rows < 3,
                "見出しの行が面の中に入る: {x:?}"
            );
        }
        // 0 × 0 でもパニックしない
        let l = layout(&t, size(0, 0));
        assert_eq!(l.leaves.len(), 2);
    }

    #[test]
    fn 比率の和が_0_なら等分する() {
        assert_eq!(distribute(9, &[0, 0, 0], &[0, 0, 0]), vec![3, 3, 3]);
        assert_eq!(distribute(10, &[0, 0, 0], &[0, 0, 0]), vec![4, 3, 3]);
    }

    #[test]
    fn 上下の分割が最小より低い面でも矩形は面の中に収める() {
        let t = split(Axis::Column, vec![(leaf(1), 5_000), (leaf(2), 5_000)]);
        for rows in [0, 1, 3, 7] {
            let l = layout(&t, size(80, rows));
            assert_eq!(l.leaves.len(), 2);
            for x in &l.leaves {
                let bar = x.bar.expect("見出しの矩形");
                assert!(x.body.row + x.body.rows <= rows, "{rows}: {x:?}");
                assert!(bar.row + bar.rows <= rows.max(bar.row), "{rows}: {x:?}");
            }
        }
    }
}
