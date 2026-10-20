//! Composition Root の共通部: 設定読込、Port の注入、JobSpawner、単一受信口、メインループ (docs/02 §4.1 / §9)。G1a で着手するのは端末に依存しない部分 (Host::step・ジョブ・JobSpawner) だけ。
//!
//! 現在は workspace の骨格 (M1 のタスク (0)) で、実装はない。実装は docs/08-roadmap.md §2.2 の「最初の縦切り」の順に入れる。

#![forbid(unsafe_code)]
