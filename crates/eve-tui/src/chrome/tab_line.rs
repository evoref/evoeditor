//! タブライン (FR-510、04 §3.4 (f) / §3.5、03 §9.9 の PANE-12 (e))。core が渡す `TabLineModel` (開いた順のタブ・アクティブ)
//! を、画面の最上行に並べる。TUI が決めるのは、タブの形・どの範囲を出すか・省いた側の印だけ。
//!
//! - タブの形: `[`・印の欄 1 セル (アクティブは `▶`、ASCII の縮退では `>`。ほかは空白)・項目 (間は 1 セル)・空白・`]`。
//!   タブの間は空白 1 つ。名前 (`CutPolicy::Middle` の項目) は 1 タブあたり `TAB_NAME_MAX` セルまで (中間を省略)。印
//!   (変更・読み取り専用) は落とさず、短い形があれば短い形を使う。
//! - 並べ方 ([`arrange`]): 「(タブの幅の並び, アクティブ, 幅) → 並び」の純関数。前の Frame の範囲を覚えない。
//! - スタイル: アクティブなタブは `[` から `]` までを `ui.tab_line.active` (無色の段は反転)、ほかと行の残りを
//!   `ui.tab_line`、省いた側の `‹` `›` (ASCII の縮退では `<` `>`) を `ui.scroll_marker` で描く。

use std::ops::Range;

use eve_view::{CellRect, CutPolicy, DocumentId, Span, Style, TabItem, TabLineModel};

use super::parts::{ascii_span, glyph_span, put, text_width, truncate_middle};
use crate::buffer::Writer;
use crate::glyphs::Glyphs;
use crate::theme::{Theme, overlay_style};

/// 1 タブの名前の幅の上限 (セル。03 §9.9 の PANE-12 (e))。超えたら中間を省略する。
pub const TAB_NAME_MAX: u16 = 32;

/// タブラインの並び ([`arrange`] の結果)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabArrangement {
    /// 出すタブの添字の範囲。
    pub range: Range<usize>,
    /// 範囲の最初のタブを置く列 (全部入るときは 0、そうでなければ 1。0 列目は `‹` の場所)。
    pub start: u16,
    /// 左に省いたタブがある (0 列目に `‹`)。
    pub more_left: bool,
    /// 右に省いたタブがある (最後の列に `›`)。
    pub more_right: bool,
    /// アクティブなタブだけでも入らないとき、そのタブに使える幅 (名前をさらに縮める)。入るときは `None`。
    pub squeeze: Option<u16>,
}

/// タブの並べ方 (03 §9.9 の PANE-12 (e))。`widths` はタブの幅 (枠を含む)、`active` はアクティブなタブの添字 (範囲外は
/// 最後のタブ)、`cols` は行の幅。
///
/// 1. タブの幅とタブの間の空白の和が `cols` 以下なら、全部を 0 列目から並べる。
/// 2. そうでなければ両端に 1 セルずつ印の場所を取り、使える幅を `cols − 2` とする。範囲をアクティブなタブだけから始め
///    (それだけでも入らなければ、そのタブを縮める `squeeze`)、右・左の順に交互に、次のタブを (間の 1 セルを含めて) 入れ
///    られれば範囲に足す。片側が入らないか端に達したら、その側はそこで止め、もう片側だけを続ける。
/// 3. 1 列目から並べ、省いた側の端に印を出す。
pub fn arrange(widths: &[u16], active: usize, cols: u16) -> TabArrangement {
    let n = widths.len();
    if n == 0 {
        return TabArrangement {
            range: 0..0,
            start: 0,
            more_left: false,
            more_right: false,
            squeeze: None,
        };
    }
    let a = active.min(n - 1);
    let gaps = u32::from(crate::cells(n - 1));
    let total: u32 = widths.iter().map(|&w| u32::from(w)).sum::<u32>() + gaps;
    if total <= u32::from(cols) {
        return TabArrangement {
            range: 0..n,
            start: 0,
            more_left: false,
            more_right: false,
            squeeze: None,
        };
    }
    let avail = u32::from(cols.saturating_sub(2));
    if u32::from(widths[a]) > avail {
        return TabArrangement {
            range: a..a + 1,
            start: 1,
            more_left: a > 0,
            more_right: a + 1 < n,
            squeeze: Some(cols.saturating_sub(2)),
        };
    }
    let mut used = u32::from(widths[a]);
    let (mut lo, mut hi) = (a, a + 1);
    let (mut right, mut left) = (true, true);
    while right || left {
        if right {
            match widths.get(hi) {
                Some(&w) if used + 1 + u32::from(w) <= avail => {
                    used += 1 + u32::from(w);
                    hi += 1;
                }
                _ => right = false,
            }
        }
        if left {
            match lo.checked_sub(1).map(|i| widths[i]) {
                Some(w) if used + 1 + u32::from(w) <= avail => {
                    used += 1 + u32::from(w);
                    lo -= 1;
                }
                _ => left = false,
            }
        }
    }
    TabArrangement {
        range: lo..hi,
        start: 1,
        more_left: lo > 0,
        more_right: hi < n,
        squeeze: None,
    }
}

/// タブの中身 (`[` と `]` の間の、印の欄の後ろ。項目を 1 セルの空白で並べたもの)。名前は `name_max` セルまで (中間を省略)。
fn tab_items(tab: &TabItem, name_max: u16, glyphs: &Glyphs) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    for (i, item) in tab.items.iter().enumerate() {
        // 間の 1 セルは、名前を縮めて空にしたときも残す (タブの幅を名前の幅から決めるため)
        if i > 0 {
            out.push(ascii_span(" ", Style::DEFAULT));
        }
        if item.long.cut == CutPolicy::Middle {
            out.extend(truncate_middle(
                &item.long.spans,
                name_max,
                &glyphs.ellipsis,
            ));
        } else {
            // 印は落とさず、短い形があれば短い形 (`[RO]` ではなく `RO`。角括弧を入れ子にしない)
            let t = item.short.as_ref().unwrap_or(&item.long);
            out.extend(t.spans.iter().cloned());
        }
    }
    out
}

/// タブ 1 つのセルの並び (`[`・印の欄・中身・空白・`]`)。
fn tab_spans(tab: &TabItem, active: bool, name_max: u16, glyphs: &Glyphs) -> Vec<Span> {
    let mut out = vec![ascii_span("[", Style::DEFAULT)];
    out.push(if active {
        glyph_span(&glyphs.focus, Style::DEFAULT)
    } else {
        ascii_span(" ", Style::DEFAULT)
    });
    out.extend(tab_items(tab, name_max, glyphs));
    out.push(ascii_span(" ", Style::DEFAULT));
    out.push(ascii_span("]", Style::DEFAULT));
    out
}

/// 並べた 1 つのタブ。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlacedTab {
    /// タブの文書。
    pub doc: DocumentId,
    /// アクティブなタブか。
    pub active: bool,
    /// タブの矩形 (行 0。行の幅で切った後)。
    pub rect: CellRect,
    /// タブのセルの並び。
    pub spans: Vec<Span>,
}

/// タブラインの配置 (描画とマウスの幾何が同じものを使う)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabLineLayout {
    /// 並べたタブ (左から)。
    pub tabs: Vec<PlacedTab>,
    /// 左に省いたタブがある (0 列目に `‹`)。
    pub more_left: bool,
    /// 右に省いたタブがある (最後の列に `›`)。
    pub more_right: bool,
}

/// `model` を幅 `cols` の行に並べる (03 §9.9 の PANE-12 (e))。
pub fn layout(model: &TabLineModel, cols: u16, glyphs: &Glyphs) -> TabLineLayout {
    let active = model.active.unwrap_or(usize::MAX);
    let spans: Vec<Vec<Span>> = model
        .tabs
        .iter()
        .enumerate()
        .map(|(i, t)| tab_spans(t, i == active, TAB_NAME_MAX, glyphs))
        .collect();
    let widths: Vec<u16> = spans.iter().map(|s| text_width(s)).collect();
    let a = arrange(&widths, active, cols);
    let mut tabs = Vec::new();
    let mut x = a.start;
    // 右を省いたときは最後の列が `›` の場所 (タブの矩形をそこへ掛けない。マウスでは印の上は何もしない)
    let limit = if a.more_right {
        cols.saturating_sub(1)
    } else {
        cols
    };
    for i in a.range.clone() {
        let tab = &model.tabs[i];
        let is_active = i == active;
        let s = match a.squeeze {
            // アクティブなタブだけでも入らない: 名前をさらに縮めて、使える幅に収める
            Some(room) => {
                let fixed = text_width(&tab_spans(tab, is_active, 0, glyphs));
                let name_max = room.saturating_sub(fixed).min(TAB_NAME_MAX);
                tab_spans(tab, is_active, name_max, glyphs)
            }
            None => spans[i].clone(),
        };
        let w = text_width(&s).min(limit.saturating_sub(x));
        tabs.push(PlacedTab {
            doc: tab.doc,
            active: is_active,
            rect: CellRect {
                col: x,
                row: 0,
                cols: w,
                rows: 1,
            },
            spans: s,
        });
        x = x.saturating_add(w).saturating_add(1);
    }
    TabLineLayout {
        tabs,
        more_left: a.more_left,
        more_right: a.more_right,
    }
}

/// タブラインを `y` 行に描く。
pub(crate) fn render_tab_line(
    w: &mut Writer<'_>,
    y: u16,
    cols: u16,
    lay: &TabLineLayout,
    theme: &Theme,
    glyphs: &Glyphs,
) {
    super::parts::fill(w, 0, y, cols, theme.tab_line);
    for t in &lay.tabs {
        let base = if t.active {
            theme.tab_line_active
        } else {
            theme.tab_line
        };
        let limit = t.rect.col.saturating_add(t.rect.cols);
        put(w, t.rect.col, y, limit, &t.spans, base);
    }
    let marker = overlay_style(theme.tab_line, theme.scroll_marker);
    if lay.more_left && cols > 0 {
        put(w, 0, y, 1, &[glyph_span(&glyphs.clip_left, marker)], marker);
    }
    if lay.more_right && cols > 0 {
        put(
            w,
            cols - 1,
            y,
            cols,
            &[glyph_span(&glyphs.clip_right, marker)],
            marker,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(widths: &[u16], active: usize, cols: u16) -> (Range<usize>, u16, bool, bool, Option<u16>) {
        let r = arrange(widths, active, cols);
        (r.range, r.start, r.more_left, r.more_right, r.squeeze)
    }

    #[test]
    fn 全部入れば_0_列目から全部() {
        // 10 + 1 + 10 + 1 + 10 = 32
        assert_eq!(a(&[10, 10, 10], 1, 32), (0..3, 0, false, false, None));
        assert_eq!(a(&[], 0, 10), (0..0, 0, false, false, None));
    }

    #[test]
    fn ちょうど_1_セル足りなければ両端に印の場所を取る() {
        // 33 > 32 → 使える幅 30: アクティブ (0) から右へ 10 + 1 + 10 = 21、次の 1 + 10 = 32 > 30 で止まる
        assert_eq!(a(&[10, 10, 11], 0, 32), (0..2, 1, false, true, None));
    }

    #[test]
    fn 右だけを省く() {
        assert_eq!(a(&[5, 5, 5, 5, 5], 0, 20), (0..3, 1, false, true, None));
    }

    #[test]
    fn 左右とも省き_右を先に足す() {
        // 使える幅 18、アクティブ 2 (5) → 右 3 (11) → 左 1 (17) → 右 4 は入らず止まる → 左 0 も入らず止まる
        assert_eq!(a(&[5, 5, 5, 5, 5, 5], 2, 20), (1..4, 1, true, true, None));
    }

    #[test]
    fn 片側が止まってももう片側は続ける() {
        // 右の 25 は入らないので右は止め、左を足し続ける (使える幅 28: 5 → 11 → 17 → 23)
        assert_eq!(a(&[5, 5, 5, 5, 25], 3, 30), (0..4, 1, false, true, None));
    }

    #[test]
    fn 端に達した側は止めて反対側を続ける() {
        // アクティブが最後: 右は端なので左だけ
        assert_eq!(a(&[5, 5, 5, 5, 5], 4, 20), (2..5, 1, true, false, None));
    }

    #[test]
    fn アクティブだけでも入らなければ縮める() {
        assert_eq!(a(&[5, 40, 5], 1, 20), (1..2, 1, true, true, Some(18)));
        // 範囲外のアクティブは最後のタブ
        assert_eq!(a(&[5, 5, 40], 9, 20), (2..3, 1, true, false, Some(18)));
    }

    fn item(kind: eve_view::StatusKind, text: &str, cut: CutPolicy) -> eve_view::StatusItem {
        let spans: Vec<Span> = text
            .chars()
            .map(|c| Span::new(c.to_string(), 1, Style::DEFAULT))
            .collect();
        eve_view::StatusItem {
            kind,
            long: eve_view::CellText {
                width: text_width(&spans),
                spans,
                cut,
            },
            short: None,
            priority: 5,
            align: eve_view::StatusAlign::Left,
        }
    }

    fn tab(doc: u64, name: &str, dirty: bool) -> TabItem {
        let mut items = vec![item(eve_view::StatusKind::Path, name, CutPolicy::Middle)];
        if dirty {
            items.push(item(eve_view::StatusKind::Dirty, "[+]", CutPolicy::Never));
        }
        TabItem {
            doc: DocumentId(doc),
            items,
        }
    }

    fn text(spans: &[Span]) -> String {
        spans.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn 名前は_32_セルまでで中間を省略し_印は落とさない() {
        let name = "abcdefghijklmnopqrstuvwxyz0123456789.txt";
        let m = TabLineModel {
            tabs: vec![tab(1, name, true)],
            active: Some(0),
        };
        let lay = layout(&m, 200, &Glyphs::unicode());
        let s = text(&lay.tabs[0].spans);
        assert_eq!(s, "[▶abcdefghijklmno…yz0123456789.txt [+] ]");
        assert_eq!(text_width(&lay.tabs[0].spans), 2 + TAB_NAME_MAX + 4 + 2);
        // ASCII の縮退の省略記号は 3 セル
        let lay = layout(&m, 200, &Glyphs::ascii());
        assert_eq!(
            text(&lay.tabs[0].spans),
            "[>abcdefghijklmn...z0123456789.txt [+] ]"
        );
    }

    #[test]
    fn 縮めても入らないタブの矩形は右の印の列に掛からない() {
        // 幅 6: 使える幅 4 に、枠と印だけで 8 セルのアクティブなタブ。右を省くので最後の列 (5) は `›`
        let m = TabLineModel {
            tabs: vec![tab(1, "a", true), tab(2, "b", false)],
            active: Some(0),
        };
        let lay = layout(&m, 6, &Glyphs::unicode());
        assert!(lay.more_right && !lay.more_left);
        assert_eq!(lay.tabs.len(), 1);
        assert_eq!((lay.tabs[0].rect.col, lay.tabs[0].rect.cols), (1, 4));
    }

    /// 結果の範囲の幅の和が使える幅に収まり、アクティブを含み、省いた側と印が一致する (性質)。
    #[test]
    fn 並べた範囲はいつも幅に収まりアクティブを含む() {
        let mut seed = 7u32;
        let mut next = || {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            u16::try_from((seed >> 16) & 0x7fff).expect("15 ビット")
        };
        for _ in 0..2_000 {
            let n = usize::from(next() % 8) + 1;
            let widths: Vec<u16> = (0..n).map(|_| next() % 30 + 4).collect();
            let active = usize::from(next()) % n;
            let cols = next() % 120 + 3;
            let r = arrange(&widths, active, cols);
            assert!(
                r.range.contains(&active),
                "{widths:?} {active} {cols}: {r:?}"
            );
            assert_eq!(r.more_left, r.range.start > 0);
            assert_eq!(r.more_right, r.range.end < n);
            let used: u32 = widths[r.range.clone()]
                .iter()
                .map(|&w| u32::from(w))
                .sum::<u32>()
                + u32::from(crate::cells(r.range.len() - 1));
            if r.squeeze.is_none() {
                let room = if r.start == 0 { cols } else { cols - 2 };
                assert!(used <= u32::from(room), "{widths:?} {active} {cols}: {r:?}");
                // 範囲の隣のタブ (省いたもの) は、どちらも足すと入らない (貪欲に足し切っている)
                for i in [r.range.start.checked_sub(1), Some(r.range.end)]
                    .into_iter()
                    .flatten()
                {
                    if let Some(&w) = widths.get(i) {
                        assert!(
                            used + 1 + u32::from(w) > u32::from(room),
                            "{widths:?} {active} {cols}: {r:?}"
                        );
                    }
                }
            } else {
                assert_eq!(r.range.len(), 1);
            }
        }
    }
}
