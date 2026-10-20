//! `Frontend` の TUI 実装 (docs/02 §4.1、docs/04)。
//!
//! 現在の範囲 (M2 の E1〜E2): 端末能力の判定 (`caps`)、装飾記号の ASCII 縮退 (`glyphs`)、出力の無害化 (`sanitize`)、
//! `Frame` → ratatui `Buffer` の変換 (`buffer`)、入力の正規化 (`input`)、`ReadConsoleInputW` の入力スレッド (`win`)。
//! 端末の初期化と復元・`Frontend` の実装は後の版で足す。

// unsafe は Windows のコンソール API を呼ぶ `win` だけに許す (R5)。他のモジュールは unsafe を書けない。
#![deny(unsafe_code)]

pub mod buffer;
pub mod caps;
pub mod color;
pub mod glyphs;
pub mod input;
pub mod sanitize;
#[cfg(windows)]
pub mod win;
