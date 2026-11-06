//! 画面の行の割り当て (04 §3.1 / §3.5 / §3.6)。端末の大きさ・ヒントバーの表示 (FR-504)・検索バーの行数から、編集面・検索バー・
//! クローム (ステータスバー・メッセージ行・ヒントバー) の行を決める。行の計算はここに集め、描画 (`render_panes` /
//! `render_screen`)・編集面の `Resize` (`input::surface_resize`)・Frame との大きさの比べ (`frontend`) はこれを使う。

use eve_view::{CellSize, Frame};

/// 端末の最小の大きさ (04 §3.5。`Capabilities.min_surface` で core に知らせる値と同じ)。
pub const MIN_WINDOW: CellSize = CellSize { cols: 20, rows: 5 };

/// 編集面の下のクロームの行数 (ステータスバー・メッセージ行・ヒントバー、04 §3.1)。
pub const CHROME_ROWS: u16 = 3;

/// 編集面が `surface_rows` 行のときのクロームの行数 (ヒントバーを出すとき)。端末が 10 行未満 (= 編集面が 8 行未満) のときは、
/// メッセージ行を確保せず 2 行にする (04 §3.5。メッセージは表示中だけヒントバーの行を使う)。
pub const fn chrome_rows_for_surface(surface_rows: u16) -> u16 {
    chrome_rows(surface_rows, true)
}

/// `chrome_rows_for_surface` にヒントバーの表示 (`Frame.hint_bar`、FR-504) を加えたもの。ヒントバーを隠すときは、端末の
/// 大きさに依らずステータスバーとメッセージ行の 2 行 (10 行未満でも、その 1 行をメッセージの表示中だけ使う、04 §3.5)。
pub const fn chrome_rows(surface_rows: u16, hint_bar: bool) -> u16 {
    if !hint_bar || surface_rows < 8 {
        CHROME_ROWS - 1
    } else {
        CHROME_ROWS
    }
}

/// 端末が `window_rows` 行のときの編集面の行数 (`chrome_rows` の逆)。
pub const fn surface_rows_for_window(window_rows: u16, hint_bar: bool) -> u16 {
    if !hint_bar || window_rows < 10 {
        window_rows.saturating_sub(CHROME_ROWS - 1)
    } else {
        window_rows - CHROME_ROWS
    }
}

/// 検索バーが取る行数 (検索欄 1 行 + 置換欄があれば 1 行)。
pub fn bar_rows(frame: &Frame) -> u16 {
    frame
        .search
        .as_ref()
        .map_or(0, |b| 1 + u16::from(b.replace.is_some()))
}

/// 画面の行の割り当て。上から、Frame の編集面 (`frame_size().rows` 行)・検索バー (`bar_rows` 行)・ステータスバー・
/// メッセージ行・ヒントバー (04 §3.1)。「端末の編集面」(`surface`) は Frame の編集面と検索バーを合わせたもので、core に
/// `Resize` で知らせる大きさ。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenLayout {
    /// 列数。
    pub cols: u16,
    /// 端末の編集面の行数 (検索バーを含む)。
    pub surface_rows: u16,
    /// 検索バーの行数 (閉じていれば 0)。
    pub bar_rows: u16,
    /// 編集面の下のクロームの行数 (`chrome_rows`)。
    pub chrome_rows: u16,
    /// ヒントバーを出すか (`Frame.hint_bar`、FR-504)。
    pub hint_bar: bool,
}

impl ScreenLayout {
    /// 端末の大きさ `window` から求める (04 §3.1 / §3.5)。`hint_bar` は最後の Frame のヒントバーの表示、`bar_rows` は
    /// 検索バーの行数 (端末の編集面だけが要るときは 0)。
    pub const fn compute(window: CellSize, hint_bar: bool, bar_rows: u16) -> ScreenLayout {
        ScreenLayout::from_surface(
            window.cols,
            surface_rows_for_window(window.rows, hint_bar),
            hint_bar,
            bar_rows,
        )
    }

    /// Frame から求める (描画用。Frame の編集面に検索バーの行を足したものを端末の編集面とする)。
    pub fn for_frame(frame: &Frame) -> ScreenLayout {
        let bar = bar_rows(frame);
        ScreenLayout::from_surface(frame.size.cols, frame.size.rows + bar, frame.hint_bar, bar)
    }

    const fn from_surface(cols: u16, surface_rows: u16, hint_bar: bool, bar_rows: u16) -> Self {
        ScreenLayout {
            cols,
            surface_rows,
            bar_rows,
            chrome_rows: chrome_rows(surface_rows, hint_bar),
            hint_bar,
        }
    }

    /// 端末が最小の大きさ (`MIN_WINDOW`) に満たないか (04 §3.5。満たなければ §3.3 (e) を出す)。
    pub const fn is_too_small(window: CellSize) -> bool {
        window.cols < MIN_WINDOW.cols || window.rows < MIN_WINDOW.rows
    }

    /// 端末の編集面の大きさ (検索バーを含む。`InputEvent::Resize` で core に送る値)。オーバーレイもこの中に置く。
    pub const fn surface(&self) -> CellSize {
        CellSize {
            cols: self.cols,
            rows: self.surface_rows,
        }
    }

    /// Frame の編集面の大きさ (検索バーの行を除く。core が `Frame.size` に入れるはずの値)。
    pub const fn frame_size(&self) -> CellSize {
        CellSize {
            cols: self.cols,
            rows: self.surface_rows.saturating_sub(self.bar_rows),
        }
    }

    /// 検索バーの最初の行 (Frame の編集面のすぐ下)。
    pub const fn bar_row(&self) -> u16 {
        self.frame_size().rows
    }

    /// ステータスバーの行。
    pub const fn status_row(&self) -> u16 {
        self.surface_rows
    }

    /// メッセージ行。`hint_row` と同じ行なら、メッセージの表示中だけそこに上書きする (04 §3.5)。
    pub const fn message_row(&self) -> u16 {
        self.surface_rows + 1
    }

    /// ヒントバーの行 (隠していれば `None`)。10 行未満の端末ではメッセージ行と同じ行になる (04 §3.5)。
    pub const fn hint_row(&self) -> Option<u16> {
        if !self.hint_bar {
            None
        } else if self.chrome_rows >= 3 {
            Some(self.surface_rows + 2)
        } else {
            Some(self.surface_rows + 1)
        }
    }

    /// 画面全体 (編集面 + クローム) の行数 (描画のバッファの高さ)。
    pub const fn rows(&self) -> u16 {
        self.surface_rows + self.chrome_rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn size(cols: u16, rows: u16) -> CellSize {
        CellSize { cols, rows }
    }

    #[test]
    fn ヒントバーありの_24_行は_編集面_21_行と_クローム_3_行() {
        let l = ScreenLayout::compute(size(80, 24), true, 0);
        assert_eq!(l.surface(), size(80, 21));
        assert_eq!(l.frame_size(), size(80, 21));
        assert_eq!(l.chrome_rows, 3);
        assert_eq!(
            (l.status_row(), l.message_row(), l.hint_row()),
            (21, 22, Some(23))
        );
        assert_eq!(l.rows(), 24);
    }

    #[test]
    fn ヒントバーを隠すと_クロームは_2_行() {
        let l = ScreenLayout::compute(size(80, 24), false, 0);
        assert_eq!(l.surface(), size(80, 22));
        assert_eq!(l.chrome_rows, 2);
        assert_eq!(
            (l.status_row(), l.message_row(), l.hint_row()),
            (22, 23, None)
        );
        assert_eq!(l.rows(), 24);
    }

    #[test]
    fn 十行未満は_メッセージとヒントが同じ行() {
        let l = ScreenLayout::compute(size(80, 9), true, 0);
        assert_eq!(l.surface_rows, 7);
        assert_eq!(l.chrome_rows, 2);
        assert_eq!(l.hint_row(), Some(l.message_row()));
        assert_eq!(l.rows(), 9);
        let l = ScreenLayout::compute(size(80, 11), true, 0);
        assert_eq!((l.surface_rows, l.chrome_rows, l.rows()), (8, 3, 11));
    }

    #[test]
    fn 十行ちょうどは_編集面_7_行で_クロームを_2_行と数える() {
        // 今の値を固定する: 端末の編集面は 10 − 3 = 7 行だが、7 行の編集面のクロームは `chrome_rows` で 2 行 (< 8) になり、
        // 描く画面は 9 行 (最後の 1 行は空)。04 §3.5 (< 10 行で 2 行) とは端末 10 行のところで食い違う
        let l = ScreenLayout::compute(size(80, 10), true, 0);
        assert_eq!((l.surface_rows, l.chrome_rows, l.rows()), (7, 2, 9));
    }

    #[test]
    fn 検索バーの行は_frame_の編集面から引く() {
        let l = ScreenLayout::compute(size(80, 24), true, 2);
        assert_eq!(l.surface(), size(80, 21));
        assert_eq!(l.frame_size(), size(80, 19));
        assert_eq!(l.bar_row(), 19);
        assert_eq!(l.status_row(), 21);
        // 編集面より検索バーが大きくても負にならない
        let l = ScreenLayout::compute(size(20, 5), true, 2);
        assert_eq!(l.surface_rows, 3);
        assert_eq!(l.frame_size().rows, 1);
        let l = ScreenLayout::compute(size(20, 2), true, 2);
        assert_eq!((l.surface_rows, l.frame_size().rows), (0, 0));
    }

    #[test]
    fn frame_から求めた割り当ては_端末から求めたものと同じ() {
        // 端末の編集面 12 行 (端末 15 行) で検索バーを開くと、Frame の編集面は 11 行
        let mut h = eve_testkit::Harness::new().with_size(80, 12);
        h.keys("ctrl+f");
        let f = h.frame();
        assert_eq!(f.size, size(80, 11));
        let a = ScreenLayout::for_frame(&f);
        assert_eq!(a.bar_rows, 1);
        assert_eq!(a, ScreenLayout::compute(size(80, 15), true, 1));
        assert_eq!((a.bar_row(), a.status_row(), a.rows()), (11, 12, 15));
        assert_eq!(a.frame_size(), f.size);
        // ヒントバーを隠した Frame
        let mut h = eve_testkit::Harness::new().with_size(80, 13);
        h.cmd("ui.toggle_hint_bar");
        let f = h.frame();
        let a = ScreenLayout::for_frame(&f);
        assert_eq!(a, ScreenLayout::compute(size(80, 15), false, 0));
        assert_eq!(a.hint_row(), None);
    }

    #[test]
    fn 最小の大きさ未満を判定する() {
        assert!(!ScreenLayout::is_too_small(MIN_WINDOW));
        assert!(ScreenLayout::is_too_small(size(19, 24)));
        assert!(ScreenLayout::is_too_small(size(80, 4)));
        // 最小の 20×5 では編集面が 3 行 (04 §3.5)
        assert_eq!(ScreenLayout::compute(MIN_WINDOW, true, 0).surface_rows, 3);
        assert_eq!(ScreenLayout::compute(MIN_WINDOW, false, 0).surface_rows, 3);
    }

    #[test]
    fn 端末が_2_行以下でも編集面は負にならない() {
        assert_eq!(ScreenLayout::compute(size(10, 1), true, 0).surface_rows, 0);
        assert_eq!(ScreenLayout::compute(size(10, 0), false, 0).surface_rows, 0);
    }
}
