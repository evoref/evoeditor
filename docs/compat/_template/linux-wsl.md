# 互換性の記録 — Linux (WSL2 の Windows Terminal と tmux) (v<版>)

`docs/compat/<版>/linux-wsl.md` に写して埋める。手順は [`../checklist.md`](../checklist.md) の A〜G と L。基準機の固有情報 (機種名・型番・OS のビルド番号・ウイルス対策ソフトの製品名・ユーザー名を含むパス) は書かない。

- 確認した人 / 日付: <名前> / <YYYY-MM-DD>
- 判定したコミットと、使った tar.gz の SHA-256 (先頭 12 桁): <コミット> / <SHA-256>
- Windows / WSL: <例: Windows 11 24H2 級 / WSL 2.x 系>
- ディストリビューション: <例: Ubuntu 26.04 LTS>
- 端末: <例: Windows Terminal 1.24 系>
- tmux: <例: 3.6>
- 日本語入力: <Microsoft IME の版 / Google 日本語入力の版>

○ = 期待どおり、△ = 動くが気になる点あり、× = 期待どおりでない。△ と × は Issue 番号を備考に。自動の試験 (e2e (linux)・CI) で確かめた項目も、手で確かめたら書く。

| 項目 | Microsoft IME | Google 日本語入力 | 備考 |
|---|---|---|---|
| A1〜A5 起動と終了 | | | |
| B1〜B5 日本語入力 (L6) | | | |
| C1 リサイズ | | | |
| C2 Ctrl+Shift の区別 | | | |
| C3 端末が横取りするキー | | | |
| C4・C5 色の区別と段階 | | | |
| D1〜D6 貼り付けとクリップボード | | | |
| E1〜E3 検索と置換 | | | |
| F1〜F6 ファイルの読み書き | | | |
| G1〜G3 巨大ファイル | | | |
| L1 README の Linux の導入手順 | | | |
| L2 タブを閉じたときの退避 (SIGHUP) | | | |
| L3 サスペンドと再開 | | | |
| L4 Windows のクリップボードとの往復 | | | |
| L5 端末の貼り付け (ブラケットペースト) | | | |
| L7 tmux の中 (推奨設定あり) と kill-session | | | |
| L8 tmux の推奨設定なし | | | |

## 判定

- 合格 / 不合格: <>
- 未確認のまま出荷するもの (05 §9.4): 実機のデスクトップ環境 (GNOME / KDE の Wayland・X11) のクリップボードと IME の候補窓、`TERM=linux` のコンソール
