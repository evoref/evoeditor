//! 緊急の端末復元に渡す平文の型 (docs/02 §9.2)。OS の型を含まない。eve-tui が作り、eve-host が運び、eve-platform の
//! `emergency_restore` が解釈する。

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// 描画先の生ハンドル (Windows: コンソールの出力ハンドル)。ポインタ ⇔ 整数の変換は eve-tui と eve-platform だけが行う。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawTermHandle(pub usize);

/// 起動時に保存した端末のモード。中身は frontend が詰め、`emergency_restore` だけが解釈する不透明なバイト列。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedTermMode(pub Box<[u8]>);

/// 「復元済み」の原子フラグ。frontend の `restore` と緊急の復元が共有し、`false → true` に立てた側だけが復元する。
#[derive(Clone, Debug, Default)]
pub struct TermRestoredFlag(pub Arc<AtomicBool>);
