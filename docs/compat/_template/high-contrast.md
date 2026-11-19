# 高コントラストテーマの確認の記録 (v<版>)

`docs/compat/<版>/high-contrast.md` に写して埋める。手順は [`../checklist.md`](../checklist.md) の P (08 §2.5 の M26 の (4)、09 §8.2 の互換の行、07 §8.7)。基準機の固有情報 (機種名・型番・OS のビルド番号・ウイルス対策ソフトの製品名・ユーザー名を含むパス) は書かない。

- 確認した人 / 日付: <名前> / <YYYY-MM-DD>
- 判定したコミットと、使った配布物の SHA-256 (先頭 12 桁): <コミット> / <SHA-256>
- Windows: <例: Windows 11 24H2 級>
- 端末の版と、使った配色: <例: Windows Terminal 1.24 系 (Campbell / One Half Light)、conhost (既定)>

○ = 期待どおり、△ = 動くが気になる点あり、× = 期待どおりでない、— = その環境で描けない。△ と × は Issue 番号を備考に。

## 1. 見分け (P1〜P7)

端末 × テーマ × 色の段ごとに 1 行。

| 端末 | テーマ | 段 | P1 通常 | P2 選択 | P3 一致 | P4 警告とステータスバー | P5 ダイアログ | P6 パレット | P7 制御文字 | 備考 |
|---|---|---|---|---|---|---|---|---|---|---|
| Windows Terminal | high-contrast-dark | truecolor | | | | | | | | |
| Windows Terminal | high-contrast-dark | 256 | | | | | | | | |
| Windows Terminal | high-contrast-dark | 16 | | | | | | | | |
| Windows Terminal | high-contrast-light | truecolor | | | | | | | | |
| Windows Terminal | high-contrast-light | 256 | | | | | | | | |
| Windows Terminal | high-contrast-light | 16 | | | | | | | | |
| conhost | high-contrast-dark | truecolor | | | | | | | | |
| conhost | high-contrast-dark | 256 | | | | | | | | |
| conhost | high-contrast-dark | 16 | | | | | | | | |
| conhost | high-contrast-light | truecolor | | | | | | | | |
| conhost | high-contrast-light | 256 | | | | | | | | |
| conhost | high-contrast-light | 16 | | | | | | | | |

## 2. カーソルの見え方 (P8)

| 端末 | 端末の配色 | テーマ | 本文 | 選択 | 現在の一致 | ダイアログのボタン | 備考 (見えにくい面) |
|---|---|---|---|---|---|---|---|
| Windows Terminal | 暗い | high-contrast-light | | | | | |
| Windows Terminal | 明るい | high-contrast-dark | | | | | |
| Windows Terminal | 暗い | high-contrast-dark | | | | | |
| Windows Terminal | 明るい | high-contrast-light | | | | | |
| conhost | 暗い | high-contrast-light | | | | | |
| conhost | 明るい | high-contrast-dark | | | | | |
| conhost | 暗い | high-contrast-dark | | | | | |
| conhost | 明るい | high-contrast-light | | | | | |

## 3. 暗い部屋での明るい塗りの面積 (P9)

| 端末 | テーマ | 全面の塗り | 強調 (選択・一致) | ステータスバー・ヒントバー・ダイアログ | 備考 (気になるロール) |
|---|---|---|---|---|---|
| Windows Terminal | high-contrast-dark | | | | |
| Windows Terminal | high-contrast-light | | | | |
| conhost | high-contrast-dark | | | | |
| conhost | high-contrast-light | | | | |

## 判定

(合格 / 不合格と、理由。× と △ の扱い。値を見直すときは、07 §8.7 の表と実装を同じ PR で直す。)
