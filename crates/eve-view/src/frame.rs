//! `Frame`: core が frontend に渡す描画モデル (docs/02-architecture.md §8)。テキスト領域は **セルグリッドにレイアウト済み**、
//! UI 部品 (クローム・オーバーレイ) は意味的モデルにする (ADR-0008)。
//!
//! 現在あるのは編集面 (`PaneFrame`) とカーソル。クローム (ステータス・メッセージ・ヒント) とオーバーレイは、
//! それを生成する側と一緒に足す。
//!
//! **不変条件 INV-F1**: Frame に入るすべての外部由来文字列は無害化済みで、制御文字と不正バイトのエスケープ文字
//! (`U+10FF00..`) を含まない (可視記号に展開済み)。frontend は `Span` をそのまま出力してよい。

use crate::caps::CellSize;
use crate::ids::{Revision, ViewId, ViewRevision, WindowId};
use crate::query::FocusTarget;

/// 色。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Color {
    /// 端末の既定色。
    #[default]
    Default,
    Rgb(u8, u8, u8),
    Indexed(u8),
}

/// 文字の属性 (ビット集合)。キーの修飾 (`Modifiers`) とは別の型。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct StyleMods(u8);

impl StyleMods {
    pub const NONE: StyleMods = StyleMods(0);
    pub const BOLD: StyleMods = StyleMods(1);
    pub const ITALIC: StyleMods = StyleMods(2);
    pub const UNDERLINE: StyleMods = StyleMods(4);
    pub const REVERSE: StyleMods = StyleMods(8);
    pub const DIM: StyleMods = StyleMods(16);

    pub fn contains(self, other: StyleMods) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl std::ops::BitOr for StyleMods {
    type Output = StyleMods;

    fn bitor(self, rhs: StyleMods) -> StyleMods {
        StyleMods(self.0 | rhs.0)
    }
}

/// 解決済みのスタイル (テーマ・選択・検索一致などを合成した結果)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub mods: StyleMods,
    pub underline_color: Option<Color>,
}

impl Style {
    pub const DEFAULT: Style = Style {
        fg: Color::Default,
        bg: Color::Default,
        mods: StyleMods::NONE,
        underline_color: None,
    };

    pub fn with_mods(mods: StyleMods) -> Style {
        Style {
            mods,
            ..Style::DEFAULT
        }
    }
}

/// 同じスタイルで描く文字列。幅 (`width`) は core が計算済みで、frontend は幅関数を持たない。
///
/// **契約 (編集面)**: 各 `Span` は 1 書記素、または含まれるすべての書記素の幅が同じもの (ASCII の連続など)。
/// frontend は `width` を書記素数で割ってセル位置を求める。幅の異なる書記素を 1 つの `Span` に混ぜない。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub width: u16,
    pub style: Style,
}

impl Span {
    pub fn new(text: impl Into<String>, width: u16, style: Style) -> Span {
        Span {
            text: text.into(),
            width,
            style,
        }
    }
}

/// セルの位置 (左上が (0, 0))。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CellPos {
    pub col: u16,
    pub row: u16,
}

/// セルの矩形。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CellRect {
    pub col: u16,
    pub row: u16,
    pub cols: u16,
    pub rows: u16,
}

/// 行の由来。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RowSource {
    /// 文書の行 (0 始まり)。`wrap_idx` は折り返し ON のときの行内の表示行 (v0.2。v0.1 は常に 0)。
    Line { line: u32, wrap_idx: u32 },
    /// 文書の最後より後ろの行 (表示は空)。
    Virtual,
}

/// 可視の 1 行。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderRow {
    pub source: RowSource,
    pub gutter: Vec<Span>,
    pub spans: Vec<Span>,
    /// 水平スクロールで左右に続きがある。
    pub clipped_left: bool,
    pub clipped_right: bool,
    /// 幅危険書記素を含む (TUI は行単位で再出力する)。
    pub width_risky: bool,
}

/// 編集面 (View 1 つ分)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneFrame {
    pub view: ViewId,
    pub rect: CellRect,
    pub revision: (Revision, ViewRevision),
    pub gutter_width: u16,
    pub rows: Vec<RenderRow>,
    /// 可視範囲にあるすべてのカーソルのセル位置 (`rect` の左上が原点)。
    pub cursors: Vec<CellPos>,
    pub focused: bool,
}

/// カーソルの形。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum CursorShape {
    #[default]
    Bar,
    Block,
    Underline,
}

/// ハードウェアカーソル・IME 候補窓の位置 (Frame 全体のセル座標)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorPlacement {
    pub pos: CellPos,
    pub shape: CursorShape,
}

/// 1 回の描画で frontend に渡すもの。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub window: WindowId,
    /// 編集面全体の大きさ。
    pub size: CellSize,
    pub panes: Vec<PaneFrame>,
    pub focus: FocusTarget,
    /// 編集面にフォーカスがあるときだけ `Some`。
    pub primary_cursor: Option<CursorPlacement>,
    /// 端末タイトル (無害化済み)。
    pub title: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn スタイルの属性は集合() {
        let m = StyleMods::BOLD | StyleMods::REVERSE;
        assert!(m.contains(StyleMods::BOLD) && m.contains(StyleMods::REVERSE));
        assert!(!m.contains(StyleMods::DIM));
        assert!(StyleMods::NONE.is_empty() && !m.is_empty());
        assert_eq!(Style::with_mods(m).fg, Color::Default);
    }

    #[test]
    fn スパンの組み立て() {
        let s = Span::new("あ", 2, Style::DEFAULT);
        assert_eq!((s.text.as_str(), s.width), ("あ", 2));
    }
}
