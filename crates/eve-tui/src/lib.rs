//! `Frontend` の TUI 実装 (docs/02 §4.1、docs/04)。
//!
//! 持つもの: 端末能力の判定 (`caps`)、装飾記号の ASCII 縮退 (`glyphs`)、出力の無害化 (`sanitize`)、
//! `Frame` → ratatui `Buffer` の変換 (`buffer`)、入力 (`input`: 正規化・VT の解析・OS ごとの端末と入力スレッド (`input::win` / `input::unix`))。
//! 差分出力 (`ansi`)、`Frontend` の実装 (`frontend`)、クロームとオーバーレイ (`chrome`)、ロールのスタイル (`theme`)。

// unsafe は OS の API を呼ぶ `input::win` (Windows のコンソール) と `input::unix` (端末) だけに許す (R5)。他のモジュールは unsafe を書けない。
#![deny(unsafe_code)]

pub mod ansi;
pub mod buffer;
pub mod caps;
pub mod chrome;
pub mod color;
pub mod frontend;
pub mod glyphs;
pub mod input;
pub mod intercept;
pub mod mouse;
pub mod sanitize;
pub mod theme;
pub mod trace;

/// セルの数・座標 (usize) を `u16` にする。収まらない値は `u16::MAX` で止める
/// (描画の座標は u16。黙って下位のビットだけにしない。描画の経路なのでパニックにもしない)。
pub(crate) fn cells(n: usize) -> u16 {
    u16::try_from(n).unwrap_or(u16::MAX)
}

// 以前の場所 (crate の直下) の名前。bin (`unix::is_background_job`)・fuzz (`vt_input::fuzz_pipeline`)・診断の例が使う。
#[cfg(unix)]
pub use input::unix;
pub use input::vt as vt_input;
#[cfg(windows)]
pub use input::win;
