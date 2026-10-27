# 依存の記録のレビュー (v0.1.0-rc.1)

NFR-706 AC-1「全依存に ADR または依存追加チェックリストの記録がある」(R) の確認。2026-10-24、`cargo metadata` の直接依存 (ワークスペース外) 20 件を、docs/02 §13 (主要クレート選定)・ADR・docs/05・09・11・12 から探した。

**結果**: 20 件すべてに記録がある。確認で見つかった不足は 1 件 (`xxhash-rust`。`twox-hash` と合わせて docs/02 §13.7 のハッシュの行に書いた)。

依存の規則 (R1〜R12、`deny.toml`、禁止ライセンス、C を含む依存 0) は `cargo xtask check-deps` と `cargo deny check` が CI で毎回確かめる。推移的な依存の帰属は `THIRD-PARTY-NOTICES.md` (`gen-notices --check`)。

| crate | 種別 | 使う crate | 最初に記録のある場所 |
|---|---|---|---|
| `arc-swap` | normal | `eve-host` | docs/02、行 168 |
| `chardetng` | normal | `eve-encoding` | docs/02、行 163 |
| `encoding_rs` | dev/normal | `eve-core`, `eve-encoding`, `eve-platform`, `evoeditor` | docs/02、行 163 |
| `memchr` | normal | `eve-core` | docs/02、行 166 |
| `memory-stats` | dev | `eve-core`, `eve-platform`, `eve-text` | docs/09、行 332 |
| `portable-pty` | dev | `evoeditor` | docs/09、行 205 |
| `proptest` | dev | `eve-core`, `eve-encoding`, `eve-text` | docs/02、行 173 |
| `ratatui` | normal | `eve-tui` | docs/02、行 67 |
| `ropey` | normal | `eve-text` | docs/02、行 127 |
| `serde` | normal | `xtask` | docs/02、行 164 |
| `serde_json` | normal | `xtask` | docs/02、行 171 |
| `sha2` | normal | `xtask` | docs/02、行 176 |
| `toml` | normal | `eve-config`, `xtask` | docs/02、行 152 |
| `twox-hash` | normal | `eve-core` | docs/02、行 166 |
| `unicode-segmentation` | normal | `eve-text` | docs/02、行 162 |
| `unicode-width` | dev/normal | `eve-testkit`, `eve-text`, `eve-tui` | docs/02、行 162 |
| `vt100` | dev | `evoeditor` | docs/10、行 1231 |
| `windows-sys` | normal | `eve-platform`, `eve-tui` | docs/02、行 162 |
| `xxhash-rust` | normal | `eve-platform` | docs/02、行 2028 |
| `zip` | normal | `xtask` | docs/02、行 176 |

新しい直接依存を足す PR は、この表と同じ形で docs/02 §13 (または ADR) に記録を足す。
