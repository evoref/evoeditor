//! Ports の OS 実装 (docs/02 §4.1、docs/05)。v0.1 の実装は Windows だけ。純粋な計算 (パスの正規化・ディレクトリの解決) は
//! どの OS でもテストできるようにしてあり、OS を呼ぶ部分は `cfg(windows)` のモジュールに閉じる。
//!
//! 現在の範囲 (M2 の E3): `path` (正規化)、`dirs` (設定・状態・データのディレクトリ)、`fs` (`FileSystem` の読み取り側)、
//! `console` (コンソールイベントの受信)。保存 (`write_atomic`) は M3、クリップボード (所有者ウィンドウのスレッドが要る) は後の版。

// unsafe は Windows の API を呼ぶ `fs::winapi`・`console`・`save` だけに許す (R5)。
#![deny(unsafe_code)]
// eve-core の `FsError` は大きい型 (エラーは稀な経路)。値で返す。
#![allow(clippy::result_large_err)]

#[cfg(windows)]
pub mod console;
pub mod dirs;
pub mod fs;
#[cfg(windows)]
mod hooks;
#[cfg(windows)]
mod inflight;
#[cfg(windows)]
mod inplace;
pub mod path;
#[cfg(windows)]
pub mod rescue;
#[cfg(windows)]
mod save;
