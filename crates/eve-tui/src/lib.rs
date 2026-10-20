//! `Frontend` の TUI 実装 (docs/02 §4.1、docs/04)。
//!
//! 現在の範囲 (M2 の E1): 端末能力の判定 (`caps`)、装飾記号の ASCII 縮退 (`glyphs`)、出力の無害化 (`sanitize`)、
//! `Frame` → ratatui `Buffer` の変換 (`buffer`)。入力 (`ReadConsoleInputW`)・端末の初期化と復元・`Frontend` の実装は後の版で足す。

#![forbid(unsafe_code)]

pub mod buffer;
pub mod caps;
pub mod color;
pub mod glyphs;
pub mod sanitize;
