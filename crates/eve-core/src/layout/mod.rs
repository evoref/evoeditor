//! 編集面のレイアウトと Frame の生成 (docs/03 §14、docs/02 §8)。可視範囲だけを生成する (コストは可視行数 × 可視列数)。
//!
//! 範囲: 折り返し (`wrap`)、水平スクロール、行番号のガター、タブの展開、全角・結合文字・絵文字の幅、不可視文字・制御文字・
//! 不正バイトのエスケープの可視化、全角スペースとタブの可視化 (FR-508 の最小)、選択、カーソル。
//! スタイルはロール (`RoleStyles`) で引く。
//!
//! spec: 02 §8, 03 §14, 07 §8 (テーマ段の適用)

mod cache;
mod frame;
mod rows;
mod units;

pub(crate) use cache::LayoutCache;
pub(crate) use frame::{build_frame, primary_cursor_cell};
pub(crate) use rows::{cell_text, gutter_width, line_rows, wrap_ctx, wrap_width};
pub(crate) use units::{
    byte_at_col, cursor_cell, cursor_col, doc_width_config, is_long_line, long_line_chars,
};
// 子モジュール (long・wrap) が `super::` で使う
use units::{Unit, is_plain_wide};
#[cfg(test)]
use units::{unit_slow, units, units_with, width_config};

mod long;
pub(crate) mod theme;
mod wrap;
mod wrap_scroll;
use long::{CharWidth, ColIndex};
pub use wrap::{Affinity, WrapMode, WrapSettings};
use wrap::{cursor_in_rows, wrap_line};

/// 画面のセルの数・座標 (usize) を `u16` にする。収まらない値は `u16::MAX` で止める
/// (描画の座標は u16。黙って下位のビットだけにしない。描画の経路なのでパニックにもしない)。
pub(crate) fn cells_u16(n: usize) -> u16 {
    u16::try_from(n).unwrap_or(u16::MAX)
}

/// 行番号・折り返しの番号 (usize) を `u32` にする。収まらない値は `u32::MAX` で止める。
pub(crate) fn index_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}
