//! 文書ごとのレイアウトの覚え (docs/02 §4.6 の `layout/` の「レイアウトキャッシュ」、03 §11.5.1 / §14.4、NFR-106)。
//!
//! `Document` が持つが、中身を読み書きするのは `layout/` だけ。表示単位の覚えは Revision を鍵に含むので、文書が変われば
//! 自然に使われなくなる (消すのは次に覚えるとき)。長大行の列の索引の覚えは、どの Revision の本文に対して正しいかを覚え
//! (鍵は行頭の位置と表示の設定)、本文を変える `ChangeSet` の適用のたびに 03 §14.4 の「編集」で引き継ぐか捨てる
//! (`on_change`。02 §5.1)。

use std::rc::Rc;

use eve_text::{ChangeSet, TextRead};

use super::long::{ColIndex, carry};
use super::rows::LineRows;
use super::units::Unit;
use super::wrap::WrapSettings;

/// `units_for` の覚えの鍵 (Revision・行・エスケープ・タブ幅・曖昧幅・空白の可視化)。
pub(super) type UnitsKey = (eve_view::Revision, usize, bool, u8, bool, bool);
/// `line_rows` の覚えの鍵 (表示単位の鍵・折り返しの設定・折り返し幅)。
pub(super) type RowsKey = (UnitsKey, WrapSettings, usize);
/// 長大行の列の索引の表示の設定の鍵 (エスケープ・タブ幅・曖昧幅)。
pub(super) type LongRule = (bool, u8, bool);

/// 覚えている長大行の索引 1 行分。
pub(super) struct LongEntry {
    /// この索引が正しい本文の Revision。
    pub rev: eve_view::Revision,
    /// 行頭の位置 (文書のバイト位置)。行の番号ではなく位置で引き継ぐ (前の行の増減で番号が変わっても使える)。
    pub ls: usize,
    pub rule: LongRule,
    pub ix: Rc<ColIndex>,
}

/// 文書ごとのレイアウトの覚え。
#[derive(Default)]
pub(crate) struct LayoutCache {
    /// 長い行の表示単位の計算結果 (直近の 2 行分。新しいものが後ろ)。1 回の操作で同じ行を何度も展開しないための覚え
    /// (複数カーソルの上下の移動は、カーソルごとに元の行と行き先の行を交互に引く。03 §4.5.4 の「費用」)。
    pub(super) units: Vec<(UnitsKey, Rc<Vec<Unit>>)>,
    /// 長い行の折り返しの結果 (直近の 2 つ)。複数カーソルの表示行の移動で、カーソルごとに同じ行を折り返さない。
    /// `units` に覚えている行の分だけを持ち、表示単位はその `Rc` を共有する (同じ行の表示単位を 2 重に持たない。
    /// 覚えの量の上限は、長い行の表示単位 2 行分と、その折り返しの結果。03 §11.2)。
    pub(super) rows: Vec<(RowsKey, LineRows)>,
    /// 長大行の列の索引 (直近の数行分、03 §14.4 の「覚え」)。
    pub(super) long: Vec<LongEntry>,
}

impl LayoutCache {
    /// 表示単位の覚えに足す (古いものを捨てる)。捨てた行の折り返しの覚えも捨てる (表示単位を共有しない覚えを残さない)。
    pub(super) fn put_units(&mut self, key: UnitsKey, units: Rc<Vec<Unit>>) {
        super::units::cache_put(&mut self.units, key, units);
        let units = &self.units;
        self.rows
            .retain(|(k, _)| units.iter().any(|(uk, _)| *uk == k.0));
    }

    /// 折り返しの覚えを引く。当たったら、その行の表示単位の覚えも新しい側へ移す (2 つの覚えの並びを揃える)。
    pub(super) fn get_rows(&mut self, key: &RowsKey) -> Option<LineRows> {
        let lr = super::units::cache_get(&mut self.rows, key)?;
        if super::units::cache_get(&mut self.units, &key.0).is_none() {
            // 表示単位を捨てた行の折り返しは持たない (put_units が捨てるので、ここには来ない)
            self.rows.retain(|(k, _)| k != key);
            return None;
        }
        Some(lr)
    }

    /// 折り返しの覚えに足す。表示単位を覚えている行だけを覚える。
    pub(super) fn put_rows(&mut self, key: RowsKey, lr: LineRows) {
        if self.units.iter().any(|(uk, _)| *uk == key.0) {
            super::units::cache_put(&mut self.rows, key, lr);
        }
    }

    /// 本文に `ChangeSet` の `cs` を当てた (Revision `old` → `new`): 覚えている長大行の索引を、03 §14.4 の「編集」の
    /// (1)〜(4) のとおりに引き継ぐか捨てる。`before` は適用前、`after` は適用後の本文、`threshold` は長大行の閾値。
    pub(crate) fn on_change(
        &mut self,
        before: &dyn TextRead,
        after: &dyn TextRead,
        cs: &ChangeSet,
        (old, new): (eve_view::Revision, eve_view::Revision),
        threshold: usize,
    ) {
        // 表示単位と折り返しの覚えは Revision を鍵に含むので、古い鍵のものは二度と当たらない。持ち続けないように放す
        self.units.clear();
        self.rows.clear();
        self.long.retain_mut(|e| {
            if e.rev != old {
                return false;
            }
            let c = carry(before, after, cs, e.ls, e.ix.len(), threshold);
            let Some(ls) = c.new_ls() else {
                return false;
            };
            Rc::make_mut(&mut e.ix).apply_carry(c);
            e.ls = ls;
            e.rev = new;
            true
        });
    }
}
