# CI で確かめる受入基準の記録 (v0.1.0-rc.1)

CI のジョブが毎回確かめている受入基準 (AC) の対応と、実行の記録 (2026-10-24 に確認)。ジョブの定義は `.github/workflows/ci.yml` と `nightly.yml`。`develop` と `main` はブランチ保護で、必須のジョブが通らない PR を取り込めない (docs/11 §9.1)。

## NFR-401 AC-1: Windows と Linux のビルドとテスト (PR ごと)

| 内容 | ジョブ | 実行の記録 |
|---|---|---|
| Windows x86_64: 全 crate のビルドとテスト | `clippy (windows)`・`test (windows)` (`cargo nextest run --workspace` + doctest) | `develop` の先頭 `cda298e` で成功: <https://github.com/evoref/evoeditor/actions/runs/40066654939> |
| Linux x86_64: core 系 crate (`eve-text` `eve-encoding` `eve-view` `eve-config` `eve-core`) と `eve-host`・`eve-testkit` のビルドとヘッドレスのテスト | `clippy (linux)`・`test (linux)` (上の crate に `eve-docs-tables`・`xtask` を加えて実行) | 同上 |
| リリースでは Windows x86_64 のアーカイブを作る | `build-release-smoke` (PR ごと)、`release.yml` (タグ時) | 同上 (`build-release-smoke` 成功)。配布の手順は `cargo xtask dist` |

## NFR-507 AC-1: cargo-deny と cargo-audit

| 内容 | ジョブ | 実行の記録 |
|---|---|---|
| cargo-deny (ライセンス・禁止依存・sources) | `deps` ジョブの `cargo deny check` (PR ごと) | 上と同じ実行で `deps` 成功 |
| cargo-audit と deny の advisories (本体と `fuzz/`) | `nightly.yml` の `cargo xtask nightly` (audit の手順。毎晩) | 定期実行 2026-10-23 成功: <https://github.com/evoref/evoeditor/actions/runs/40023382460>、2026-10-22 成功: <https://github.com/evoref/evoeditor/actions/runs/39855273359> |

リリース候補のコミットでは、判定コミットでこれらのジョブが成功していることを `release-checklist.md` に書く (docs/09 §8.2)。
