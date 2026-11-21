//! レイアウトの木の操作 (docs/03 §9.9 の PANE-3 / PANE-5)。状態から切り離した純関数で、木の形と比率だけを扱う
//! (矩形の計算は `layout::panes`、ペインと View の関係は `editor::pane_cmds`)。
//!
//! どの操作の後も、木は PANE-3 の形を保つ: 分割の子は 2 つ以上、同じ向きの分割を親子にしない、比率は分割ごとに合計
//! 10,000 の正の整数 ([`LayoutNode::check_shape`])。
//!
//! spec: 03 §9.9 (PANE-3, PANE-5)

use crate::editor::window::{Axis, LayoutNode, PaneId};

/// 分割ごとの比率の合計 (PANE-3)。
pub(crate) const RATIO_TOTAL: u16 = 10_000;

/// 分割の子と、その比率。
pub(crate) type Child = (LayoutNode, u16);

impl LayoutNode {
    /// 葉 `target` の右 (`Row`) / 下 (`Column`) に葉 `new` を置いた木 (PANE-5 (a))。`target` の親が同じ向きの分割なら
    /// `new` を `target` の直後の兄弟にし、`target` の比率を半分ずつにする (`target` が切り上げ)。そうでなければ
    /// `target` を、`target` と `new` を子に持つ新しい分割 (5,000 / 5,000) で置き換える。`target` が無ければ `None`。
    pub(crate) fn with_split(&self, target: PaneId, axis: Axis, new: PaneId) -> Option<LayoutNode> {
        let mut out = self.clone();
        out.split_in(target, axis, new).then_some(out)
    }

    fn split_in(&mut self, target: PaneId, axis: Axis, new: PaneId) -> bool {
        match self {
            LayoutNode::Leaf(p) if *p == target => {
                *self = LayoutNode::Split {
                    axis,
                    children: vec![
                        (LayoutNode::Leaf(target), RATIO_TOTAL / 2),
                        (LayoutNode::Leaf(new), RATIO_TOTAL / 2),
                    ],
                };
                true
            }
            LayoutNode::Leaf(_) => false,
            LayoutNode::Split {
                axis: own,
                children,
            } => {
                if *own == axis
                    && let Some(i) = children
                        .iter()
                        .position(|(c, _)| *c == LayoutNode::Leaf(target))
                {
                    let r = children[i].1;
                    children[i].1 = r - r / 2;
                    children.insert(i + 1, (LayoutNode::Leaf(new), r / 2));
                    fix_ratios(children);
                    return true;
                }
                children
                    .iter_mut()
                    .any(|(c, _)| c.split_in(target, axis, new))
            }
        }
    }

    /// 葉 `target` を除いた木 (PANE-5 (d))。空いた場所は前の兄弟 (先頭なら次の兄弟) が比率を足して受け取り、子が 1 つに
    /// なった分割は親に畳み、畳んで同じ向きの分割が親子になったら孫を親に並べ直す。`target` が無い・木の唯一の葉なら `None`。
    pub(crate) fn without_leaf(&self, target: PaneId) -> Option<LayoutNode> {
        if matches!(self, LayoutNode::Leaf(_)) {
            return None;
        }
        let mut out = self.clone();
        if !out.remove_in(target) {
            return None;
        }
        out.normalize();
        Some(out)
    }

    fn remove_in(&mut self, target: PaneId) -> bool {
        let LayoutNode::Split { children, .. } = self else {
            return false;
        };
        if let Some(i) = children
            .iter()
            .position(|(c, _)| *c == LayoutNode::Leaf(target))
        {
            let (_, r) = children.remove(i);
            let heir = i.saturating_sub(1).min(children.len().saturating_sub(1));
            if let Some((_, hr)) = children.get_mut(heir) {
                *hr = hr.saturating_add(r);
            }
            return true;
        }
        children.iter_mut().any(|(c, _)| c.remove_in(target))
    }

    /// 子が 1 つの分割を畳み、同じ向きの分割の親子を 1 段に並べ直す (PANE-3。孫の比率は「子の比率 × 孫の比率 /
    /// 10,000」(切り捨て) にし、余りを最後の孫に足す)。
    fn normalize(&mut self) {
        let LayoutNode::Split { axis, children } = self else {
            return;
        };
        for (c, _) in children.iter_mut() {
            c.normalize();
        }
        let axis = *axis;
        let mut flat: Vec<(LayoutNode, u16)> = Vec::with_capacity(children.len());
        for (c, r) in std::mem::take(children) {
            match c {
                LayoutNode::Split {
                    axis: inner,
                    children: grand,
                } if inner == axis => {
                    let scaled: Vec<u16> =
                        grand.iter().map(|(_, g)| scale(*g, r)).collect::<Vec<_>>();
                    let used: u32 = scaled.iter().map(|&s| u32::from(s)).sum();
                    let rest = u32::from(r).saturating_sub(used);
                    let n = grand.len();
                    for (k, ((g, _), s)) in grand.into_iter().zip(scaled).enumerate() {
                        let s = if k + 1 == n {
                            s.saturating_add(u16::try_from(rest).unwrap_or(u16::MAX))
                        } else {
                            s
                        };
                        flat.push((g, s));
                    }
                }
                other => flat.push((other, r)),
            }
        }
        if flat.len() == 1 {
            if let Some((only, _)) = flat.pop() {
                *self = only;
            }
            return;
        }
        fix_ratios(&mut flat);
        *children = flat;
    }

    /// 全部の分割で子の比率を等しくする (PANE-5 (f)。10,000 を子の数で割り、余りを前の子から 1 ずつ)。
    pub(crate) fn equalize(&mut self) {
        if let LayoutNode::Split { children, .. } = self {
            let ratios = equal_ratios(children.len());
            for ((c, r), e) in children.iter_mut().zip(ratios) {
                *r = e;
                c.equalize();
            }
        }
    }

    /// 葉 `target` の親の分割 (向き・子の比率) と、その中での `target` の位置。`target` が根・無ければ `None`。
    pub(crate) fn parent_of_mut(
        &mut self,
        target: PaneId,
    ) -> Option<(Axis, &mut Vec<Child>, usize)> {
        let LayoutNode::Split { axis, children } = self else {
            return None;
        };
        if let Some(i) = children
            .iter()
            .position(|(c, _)| *c == LayoutNode::Leaf(target))
        {
            return Some((*axis, children, i));
        }
        children
            .iter_mut()
            .find_map(|(c, _)| c.parent_of_mut(target))
    }

    /// 葉 `target` の親の分割の向きと、その中での `target` の位置 (読むだけの形)。
    pub(crate) fn parent_of(&self, target: PaneId) -> Option<(Axis, &[Child], usize)> {
        let LayoutNode::Split { axis, children } = self else {
            return None;
        };
        if let Some(i) = children
            .iter()
            .position(|(c, _)| *c == LayoutNode::Leaf(target))
        {
            return Some((*axis, children, i));
        }
        children.iter().find_map(|(c, _)| c.parent_of(target))
    }

    /// 木の形の不変条件 (PANE-3): 分割の子は 2 つ以上、比率は正で合計 10,000、同じ向きの分割を親子にしない。
    ///
    /// # Errors
    ///
    /// 破れている条件を文で返す。
    pub(crate) fn check_shape(&self) -> Result<(), String> {
        let LayoutNode::Split { axis, children } = self else {
            return Ok(());
        };
        if children.len() < 2 {
            return Err(format!("{axis:?} の分割の子が {} つ", children.len()));
        }
        let sum: u32 = children.iter().map(|(_, r)| u32::from(*r)).sum();
        if sum != u32::from(RATIO_TOTAL) || children.iter().any(|(_, r)| *r == 0) {
            let rs: Vec<u16> = children.iter().map(|(_, r)| *r).collect();
            return Err(format!("{axis:?} の分割の比率 {rs:?} (合計 {sum})"));
        }
        for (c, _) in children {
            if let LayoutNode::Split { axis: inner, .. } = c
                && inner == axis
            {
                return Err(format!("{axis:?} の分割が同じ向きの分割を子に持つ"));
            }
            c.check_shape()?;
        }
        Ok(())
    }
}

/// `n` 個の子の等しい比率 (10,000 を `n` で割り、余りを前の子から 1 ずつ)。
pub(crate) fn equal_ratios(n: usize) -> Vec<u16> {
    let Ok(k) = u16::try_from(n) else {
        return vec![1; n];
    };
    if k == 0 {
        return Vec::new();
    }
    let (base, rem) = (RATIO_TOTAL / k, RATIO_TOTAL % k);
    (0..k).map(|i| base + u16::from(i < rem)).collect()
}

/// セルの大きさから比率を求める (PANE-5 (e)。各子を `floor(セル × 10,000 / 全体のセル)` にし、余りを最後の子に足して
/// 合計を 10,000 に保つ)。全体が 0 なら等分。
pub(crate) fn ratios_from_cells(cells: &[u16]) -> Vec<u16> {
    let total: u32 = cells.iter().map(|&c| u32::from(c)).sum();
    if total == 0 {
        return equal_ratios(cells.len());
    }
    let mut out: Vec<u16> = cells
        .iter()
        .map(|&c| {
            u16::try_from(u32::from(c) * u32::from(RATIO_TOTAL) / total).unwrap_or(RATIO_TOTAL)
        })
        .collect();
    let used: u32 = out.iter().map(|&r| u32::from(r)).sum();
    if let Some(last) = out.last_mut() {
        let rest = u32::from(RATIO_TOTAL).saturating_sub(used);
        *last = last.saturating_add(u16::try_from(rest).unwrap_or(0));
    }
    let mut tagged: Vec<((), u16)> = out.iter().map(|&r| ((), r)).collect();
    fix_ratios(&mut tagged);
    tagged.into_iter().map(|(_, r)| r).collect()
}

/// `r × g / 10,000` (切り捨て)。
fn scale(g: u16, r: u16) -> u16 {
    u16::try_from(u32::from(g) * u32::from(r) / u32::from(RATIO_TOTAL)).unwrap_or(u16::MAX)
}

/// 比率を正の整数で合計 10,000 に直す (丸めで 0 になった子を 1 にし、過不足を最も大きい子で吸収する)。端末の大きさに
/// 比べて子が極端に小さいときだけ効く。
fn fix_ratios<T>(children: &mut [(T, u16)]) {
    if children.is_empty() {
        return;
    }
    for (_, r) in children.iter_mut() {
        if *r == 0 {
            *r = 1;
        }
    }
    loop {
        let sum: u32 = children.iter().map(|(_, r)| u32::from(*r)).sum();
        let target = u32::from(RATIO_TOTAL);
        if sum == target {
            return;
        }
        let Some(big) = (0..children.len()).max_by_key(|&i| (children[i].1, usize::MAX - i)) else {
            return;
        };
        if sum < target {
            let add = u16::try_from(target - sum).unwrap_or(u16::MAX);
            children[big].1 = children[big].1.saturating_add(add);
        } else {
            let over = u16::try_from(sum - target).unwrap_or(u16::MAX);
            let room = children[big].1 - 1;
            if room == 0 {
                return; // 子の数が 10,000 を超える (起こらない)
            }
            children[big].1 -= over.min(room);
        }
    }
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

    #[test]
    fn 葉を分けると新しい分割になり_同じ向きなら兄弟に足す() {
        let t = leaf(1)
            .with_split(PaneId(1), Axis::Row, PaneId(2))
            .expect("分割");
        assert_eq!(
            t,
            split(Axis::Row, vec![(leaf(1), 5_000), (leaf(2), 5_000)])
        );
        // 同じ向き: 直後の兄弟にし、比率を半分ずつ (切り上げは元のペイン)
        let t = t.with_split(PaneId(1), Axis::Row, PaneId(3)).expect("分割");
        assert_eq!(
            t,
            split(
                Axis::Row,
                vec![(leaf(1), 2_500), (leaf(3), 2_500), (leaf(2), 5_000)]
            )
        );
        let t = t.with_split(PaneId(2), Axis::Row, PaneId(4)).expect("分割");
        assert_eq!(t.leaves(), vec![PaneId(1), PaneId(3), PaneId(2), PaneId(4)]);
        t.check_shape().expect("形");
        // 違う向き: その葉を新しい分割で置き換える
        let t = t
            .with_split(PaneId(3), Axis::Column, PaneId(5))
            .expect("分割");
        t.check_shape().expect("形");
        assert_eq!(
            t.leaves(),
            vec![PaneId(1), PaneId(3), PaneId(5), PaneId(2), PaneId(4)]
        );
        assert!(t.with_split(PaneId(9), Axis::Row, PaneId(6)).is_none());
    }

    #[test]
    fn 奇数の比率を分けると元のペインが切り上げ() {
        let t = split(Axis::Row, vec![(leaf(1), 3_333), (leaf(2), 6_667)]);
        let t = t.with_split(PaneId(1), Axis::Row, PaneId(3)).expect("分割");
        assert_eq!(
            t,
            split(
                Axis::Row,
                vec![(leaf(1), 1_667), (leaf(3), 1_666), (leaf(2), 6_667)]
            )
        );
    }

    #[test]
    fn 閉じると前の兄弟が受け取り_先頭なら次の兄弟() {
        let t = split(
            Axis::Row,
            vec![(leaf(1), 2_000), (leaf(2), 3_000), (leaf(3), 5_000)],
        );
        assert_eq!(
            t.without_leaf(PaneId(2)),
            Some(split(Axis::Row, vec![(leaf(1), 5_000), (leaf(3), 5_000)]))
        );
        assert_eq!(
            t.without_leaf(PaneId(1)),
            Some(split(Axis::Row, vec![(leaf(2), 5_000), (leaf(3), 5_000)]))
        );
        // 2 つのうち 1 つを閉じると、分割を畳んで葉になる
        let two = split(Axis::Row, vec![(leaf(1), 5_000), (leaf(2), 5_000)]);
        assert_eq!(two.without_leaf(PaneId(1)), Some(leaf(2)));
        // 唯一の葉・無い葉は閉じない
        assert_eq!(leaf(1).without_leaf(PaneId(1)), None);
        assert_eq!(two.without_leaf(PaneId(7)), None);
    }

    #[test]
    fn 畳んで同じ向きが親子になったら孫を並べ直し_比率を掛ける() {
        // Row[1 (4,000), Column[2, Row[3 (3,000), 4 (7,000)]] (6,000)] で 2 を閉じると、Column が畳まれて
        // Row[3, 4] が Row の子になる → Row[1 4,000, 3 1,800, 4 4,200]
        let t = split(
            Axis::Row,
            vec![
                (leaf(1), 4_000),
                (
                    split(
                        Axis::Column,
                        vec![
                            (leaf(2), 5_000),
                            (
                                split(Axis::Row, vec![(leaf(3), 3_000), (leaf(4), 7_000)]),
                                5_000,
                            ),
                        ],
                    ),
                    6_000,
                ),
            ],
        );
        let after = t.without_leaf(PaneId(2)).expect("閉じる");
        assert_eq!(
            after,
            split(
                Axis::Row,
                vec![(leaf(1), 4_000), (leaf(3), 1_800), (leaf(4), 4_200)]
            )
        );
        after.check_shape().expect("形");
        // 切り捨ての余りは最後の孫に足す: 3,333 × 3,333 / 10,000 = 1,110、3,333 × 6,667 / 10,000 = 2,222 → 余り 1
        let t = split(
            Axis::Row,
            vec![
                (leaf(1), 6_667),
                (
                    split(
                        Axis::Column,
                        vec![
                            (leaf(2), 5_000),
                            (
                                split(Axis::Row, vec![(leaf(3), 3_333), (leaf(4), 6_667)]),
                                5_000,
                            ),
                        ],
                    ),
                    3_333,
                ),
            ],
        );
        assert_eq!(
            t.without_leaf(PaneId(2)),
            Some(split(
                Axis::Row,
                vec![(leaf(1), 6_667), (leaf(3), 1_110), (leaf(4), 2_223)]
            ))
        );
    }

    #[test]
    fn 揃えると全部の分割で等しい比率() {
        let mut t = split(
            Axis::Row,
            vec![
                (leaf(1), 1_000),
                (leaf(2), 2_000),
                (
                    split(Axis::Column, vec![(leaf(3), 9_000), (leaf(4), 1_000)]),
                    7_000,
                ),
            ],
        );
        t.equalize();
        assert_eq!(
            t,
            split(
                Axis::Row,
                vec![
                    (leaf(1), 3_334),
                    (leaf(2), 3_333),
                    (
                        split(Axis::Column, vec![(leaf(3), 5_000), (leaf(4), 5_000)]),
                        3_333
                    ),
                ],
            )
        );
        t.check_shape().expect("形");
    }

    #[test]
    fn セルから比率を求め_余りは最後の子() {
        assert_eq!(ratios_from_cells(&[30, 70]), vec![3_000, 7_000]);
        assert_eq!(ratios_from_cells(&[1, 1, 1]), vec![3_333, 3_333, 3_334]);
        assert_eq!(
            ratios_from_cells(&[0, 0]),
            vec![5_000, 5_000],
            "全体が 0 なら等分"
        );
        // 丸めで 0 になる子も正にする
        let r = ratios_from_cells(&[1, 60_000]);
        assert!(r.iter().all(|&x| x > 0), "{r:?}");
        assert_eq!(r.iter().map(|&x| u32::from(x)).sum::<u32>(), 10_000);
    }

    #[test]
    fn 形の検査は子の数_比率_同じ向きの入れ子を見る() {
        assert!(
            split(Axis::Row, vec![(leaf(1), 10_000)])
                .check_shape()
                .is_err()
        );
        assert!(
            split(Axis::Row, vec![(leaf(1), 5_000), (leaf(2), 4_999)])
                .check_shape()
                .is_err()
        );
        assert!(
            split(Axis::Row, vec![(leaf(1), 0), (leaf(2), 10_000)])
                .check_shape()
                .is_err()
        );
        let nested = split(
            Axis::Row,
            vec![
                (leaf(1), 5_000),
                (
                    split(Axis::Row, vec![(leaf(2), 5_000), (leaf(3), 5_000)]),
                    5_000,
                ),
            ],
        );
        assert!(nested.check_shape().is_err());
    }
}
