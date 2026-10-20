# 貢献について

evoeditor への関心をありがとうございます。

## 現在の状況

**外部からの Pull Request は、まだ受け付けていません。** 受付は、v0.1 のリリース後、受付の条件が整ってから始める予定です (開始の条件は [docs/08-roadmap.md](docs/08-roadmap.md) と [docs/12-licensing-distribution.md](docs/12-licensing-distribution.md) の §3.5 を参照)。受付前に届いた Pull Request は、内容を確認せずに閉じることがあります。

## 今できること

- **バグの報告・機能の要望・意見** は、Issues で受け付けています。再現の手順、OS と端末、evoeditor の版を書いてください。
- **脆弱性** は、公開の Issue に書かず、[SECURITY.md](SECURITY.md) の窓口に報告してください。
- 仕様の誤りや読みにくい箇所があれば、[docs/](docs/README.md) を指して Issue に書いてください。

## 寄稿のライセンスと DCO

このプロジェクトは [Apache License 2.0](LICENSE) で提供されています。寄稿は、同じ Apache-2.0 の条件で提供されたものとして受け付けます (inbound = outbound)。

寄稿の受付を始めたあとは、**DCO (Developer Certificate of Origin)** を使います。すべてのコミットに `Signed-off-by:` の行を付けてください。

```
git commit -s -m "feat(eve-core): ..."
```

これは、「この変更は自分が書いた、または提出する権利を持ち、プロジェクトのライセンスで提供する」という宣言です。署名の書類や事前の登録は要りません。メンテナ (エージェントを含む) のコミットも同じ手順です。

## `NOTICE` への追記

`NOTICE` は、Apache-2.0 の帰属表示のためのファイルです。寄稿による `NOTICE` への追記は、受け付けていません。

## 開発の進め方

開発は仕様書駆動です。ブランチ、コミット、リリースの運用は [docs/11-dev-workflow.md](docs/11-dev-workflow.md) の §9 にあります。完全な貢献ガイドは、外部寄稿の受付を始める前に整備します。
