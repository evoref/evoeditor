//! 文書ごとのレイアウトの覚え (docs/02 §4.6 の `layout/` の「レイアウトキャッシュ」、03 §11.5.1 / §14.4、NFR-106)。
//!
//! `Document` が持つが、中身を読み書きするのは `layout/` だけ。表示単位の覚えは Revision を鍵に含むので、文書が変われば
//! 自然に使われなくなる (消すのは次に覚えるとき)。長大行の列の索引の覚えは、どの Revision の本文に対して正しいかを覚え
//! (鍵は行頭の位置と表示の設定)、本文を変える `ChangeSet` の適用のたびに 03 §14.4 の「編集」で引き継ぐか捨てる
//! (`on_change`。02 §5.1)。

use std::rc::Rc;

use eve_text::{ChangeSet, TextRead};

use super::long::{ColIndex, carry};
use super::units::Unit;

/// `units_for` の覚えの鍵 (Revision・行・エスケープ・タブ幅・曖昧幅・空白の可視化)。
pub(super) type UnitsKey = (eve_view::Revision, usize, bool, u8, bool, bool);
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
    /// 長い行の表示単位の計算結果 (最後の 1 行分)。1 回の操作で同じ行を何度も展開しないための覚え。
    pub(super) units: Option<(UnitsKey, Rc<Vec<Unit>>)>,
    /// 長大行の列の索引 (直近の数行分、03 §14.4 の「覚え」)。
    pub(super) long: Vec<LongEntry>,
}

impl LayoutCache {
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
