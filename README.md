# evoeditor

Rust 製のテキストエディタです。Windows の端末 (Windows Terminal / conhost) で、日本語を快適に編集することを最初の目標にしています。

> **開発中です。v0.1 は未リリースで、配布物はまだありません。**

## 状況

- **v0.1 (最小核)** を開発中です。Windows のみ・日本語 UI で、1 つのテキストファイルを開き、検索・置換し、元の文字コードと改行コードのまま安全に保存できることを目指します。UTF-8 のほか、Shift_JIS / EUC-JP / ISO-2022-JP を扱います。
- 実装はまだ始まっていません。現在は仕様書の確定と、技術検証の段階です。
- macOS / Linux と英語 UI は v0.2 で出荷する予定です。

## 仕様書

仕様書は [docs/](docs/README.md) にあります。**作業中の版として公開しています**。内容は変わることがあり、「提案中」と書かれた事項は未確定です。

- 入口: [docs/README.md](docs/README.md)
- 決定記録 (ADR) と未決事項 (OQ): [docs/10-decisions.md](docs/10-decisions.md)
- ロードマップ: [docs/08-roadmap.md](docs/08-roadmap.md)

`main` はリリースした版の仕様だけを持ちます (v0.1.0 のリリース後)。開発中の版は `develop` の `docs/` です。

## ブランチ

- `main`: リリース専用です。リリースした版だけが入ります。
- `develop`: 開発の本線です。作業ブランチから PR で入れます。

詳細は [docs/11-dev-workflow.md](docs/11-dev-workflow.md) の §9.1 を参照してください。

## 貢献について

**外部からの Pull Request は、まだ受け付けていません。** バグ報告や意見は Issues で歓迎します。脆弱性は公開の Issue に書かず、[SECURITY.md](SECURITY.md) の窓口に報告してください。貢献の方針は [CONTRIBUTING.md](CONTRIBUTING.md) にあります。

## ライセンス

[Apache License 2.0](LICENSE) です。帰属表示は [NOTICE](NOTICE) を参照してください。
