# 変更履歴

このファイルは、版ごとの変更を記録します。リリースノートの元になります (docs/12-licensing-distribution.md、docs/09-testing-quality.md §9)。
形式は [Keep a Changelog](https://keepachangelog.com/ja/1.1.0/) に倣い、版は [Semantic Versioning](https://semver.org/lang/ja/) に従います。

v0.1.0 のリリースまでの `0.0.x` は、マイルストーンを数える内部カウンタで、タグも Release も作りません (docs/11-dev-workflow.md §9.1)。

## [Unreleased]

### 追加

- 仕様書 (`docs/`)、リポジトリの運用の整備 (Issue / PR のテンプレート、Dependabot、SECURITY.md、CONTRIBUTING.md など)。
- workspace の骨格 (M1 のタスク (0)): `Cargo.toml`、`.cargo/config.toml`、`deny.toml`、`deps-policy.toml`、`clippy.toml`、`rustfmt.toml`、crate の枠。
- 開発用ツール `cargo xtask` の `check` (速い車線)・`check-deps` (依存規則 R1〜R12)・`check-ac-checklist`、docs の表を読む `eve-docs-tables`。
- CI (`.github/workflows/ci.yml`): check・fmt・clippy・test・deps・dco・gitleaks・pr-source、nextest の設定 (`.config/nextest.toml`)。
- `cargo xtask i18n` (UI 文言の検査)・`cargo xtask gen-corpus` (巨大・長大のコーパスの決定的な生成)、CI の `i18n` ジョブ。
- `eve-text`: テキストストレージ (`Text` / `Snapshot` / `TextBuilder` / `TextRead`)、行の定義 (LF・CRLF・CR)、位置変換 (byte ⇔ char ⇔ UTF-16 ⇔ 行)、チャンク走査。
