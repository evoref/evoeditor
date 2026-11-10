# evoeditor

[English](README.md) | 日本語

Rust 製の、端末で使うテキストエディタです。日本語を快適に編集でき、ファイルを元の文字コードと改行コードのまま安全に保存できることを、最初の目標にしています。

> **最新の版は v0.2.0 (2026-11-04) です。** 配布物は [GitHub Releases](https://github.com/evoref/evoeditor/releases) にあります。Windows 11 (x86_64) 用の zip と、Linux (x86_64 / aarch64) 用の静的リンクの `tar.gz` です。インストール・更新・署名の検証・既知の制約は、同梱の README ([dist/README.md](dist/README.md)、英語版は [dist/README.en.md](dist/README.en.md)) にあります。

## 状況

- **v0.1 (最小核)** は、2026-10-27 に v0.1.0 としてリリースしました。
  - Windows Terminal とコンソールホストで動きます。
  - テキストファイルを開き、検索・置換し、元の文字コード・BOM・改行コードのまま安全に保存できます。
  - UTF-8・UTF-16 のほか、Shift_JIS / EUC-JP / ISO-2022-JP を扱います。
- **v0.2** は、2026-11-04 に v0.2.0 としてリリースしました。
  - 次のものを足しました: Linux への対応、英語の画面、未保存の内容の自動保存 (スワップ) と復旧、複数の文書、正規表現の検索と置換、折り返しとテーマ、ヘルプ画面。
  - macOS には、まだ対応していません。
- **v0.3** を開発しています。振る舞いを変えずに内部を整理・効率化し、データの保全と正しさの不具合を直す版です ([docs/08-roadmap.md](docs/08-roadmap.md) §2.4)。

変更点は [CHANGELOG.md](CHANGELOG.md) (英語版は [CHANGELOG.en.md](CHANGELOG.en.md)) にあります。

## 配布物の検証

リリースの `SHA256SUMS` には、minisign の署名 (`SHA256SUMS.minisig`) が付いています。公開鍵は下のとおりです。このリポジトリの [minisign.pub](minisign.pub) と同じものなので、両方を見比べてから使ってください。

```text
untrusted comment: minisign public key 39CA49EFC7BC926D
RWRtkrzH70nKORpBFPEtLy6elo0xP5XsEli9SU/OaxnBmNM8o14GKMhv
```

```sh
minisign -Vm SHA256SUMS -P RWRtkrzH70nKORpBFPEtLy6elo0xP5XsEli9SU/OaxnBmNM8o14GKMhv
```

OS ごとの詳しい手順は、同梱の README にあります。

## 仕様書

仕様書は [docs/](docs/README.md) にあります。

**作業中の版として公開しています。** 内容は変わることがあり、「提案中」と書かれた事項は未確定です。

リリースした版の仕様は、その版のタグ (または `main`) の `docs/` です。

- 入口: [docs/README.md](docs/README.md)
- 決定記録 (ADR) と未決事項 (OQ): [docs/10-decisions.md](docs/10-decisions.md)
- ロードマップ: [docs/08-roadmap.md](docs/08-roadmap.md)

## ブランチ

- `main`: リリース専用です。リリースした版だけが入ります。
- `develop`: 開発の本線です。作業ブランチから PR で入れます。

詳細は [docs/11-dev-workflow.md](docs/11-dev-workflow.md) の §9.1 を参照してください。

## 貢献について

**外部からの Pull Request は、まだ受け付けていません。** バグ報告や意見は Issues で歓迎します。

脆弱性は公開の Issue に書かず、[SECURITY.md](SECURITY.md) の窓口に報告してください。

貢献の方針は [CONTRIBUTING.md](CONTRIBUTING.md) にあります。

## ライセンス

[Apache License 2.0](LICENSE) です。帰属表示は [NOTICE](NOTICE) を参照してください。

---

この README は日本語版が正です。英語版 ([README.md](README.md)) と食い違うときは、日本語版に従います。
