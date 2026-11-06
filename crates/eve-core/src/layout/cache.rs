//! 文書ごとのレイアウトの覚え (docs/02 §4.6 の `layout/` の「レイアウトキャッシュ」、03 §11.5.1、NFR-106)。
//!
//! `Document` が持つが、中身を読み書きするのは `layout/` だけ。どの覚えも Revision を鍵に含むので、文書が変われば
//! 自然に使われなくなる (消すのは次に覚えるとき)。

use std::rc::Rc;

use super::long::ColIndex;
use super::units::Unit;

/// `units_for` の覚えの鍵 (Revision・行・エスケープ・タブ幅・曖昧幅・空白の可視化)。
pub(super) type UnitsKey = (eve_view::Revision, usize, bool, u8, bool, bool);
/// 長い行の列の索引の鍵 (Revision・行・エスケープ・タブ幅・曖昧幅)。
pub(super) type LongKey = (eve_view::Revision, usize, bool, u8, bool);

/// 文書ごとのレイアウトの覚え。
#[derive(Default)]
pub(crate) struct LayoutCache {
    /// 長い行の表示単位の計算結果 (最後の 1 行分)。1 回の操作で同じ行を何度も展開しないための覚え。
    pub(super) units: Option<(UnitsKey, Rc<Vec<Unit>>)>,
    /// 長大行の列の索引 (直近の数行分、03 §11.5.1)。
    pub(super) long: Vec<(LongKey, Rc<ColIndex>)>,
}
