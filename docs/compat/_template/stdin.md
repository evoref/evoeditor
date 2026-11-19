# 標準入力の確認の記録 (v<版>)

`docs/compat/<版>/stdin.md` に写して埋める。手順は [`../checklist.md`](../checklist.md) の N (05 §9.4 の「標準入力の手での確認」)。基準機の固有情報 (機種名・型番・OS のビルド番号・ウイルス対策ソフトの製品名・ユーザー名を含むパス) は書かない。

- 確認した人 / 日付: <名前> / <YYYY-MM-DD>
- 判定したコミットと、使った配布物の SHA-256 (先頭 12 桁): <コミット> / <SHA-256>
- Windows / WSL のディストリビューション / ssh の接続先の OS: <例: Windows 11 24H2 級 / Ubuntu 24.04 / Linux>
- 端末とシェルの版: <例: Windows Terminal 1.24 系、PowerShell 7.6 系、Git for Windows 2.x の mintty、tmux 3.x>

○ = 期待どおり、△ = 動くが気になる点あり、× = 期待どおりでない、— = その環境に当てはまらない。△ と × は Issue 番号を備考に。

| 環境 | N1 閉じるパイプ | N2 閉じないパイプと `Esc` | N3 書き手のパスワード | N4 端末の判定 | N5 読込中のサスペンド | 備考 (N2 の書き手の止まり方など) |
|---|---|---|---|---|---|---|
| Windows Terminal (PowerShell 7) | | | — | | — | |
| Windows Terminal (cmd) | | | — | | — | |
| conhost | | | — | | — | |
| VS Code の統合端末 | | | — | | — | |
| mintty (Git Bash) | | | — | | — | |
| Windows PowerShell 5.1 のパイプ | | | — | | — | |
| WSL からの interop (`cmd.exe /c`) | | | — | | — | |
| WSL の側のパイプ (Windows Terminal) | | | | | | |
| tmux の中 (WSL) | | | | | | |
| `ssh -t` 越し (Linux のホスト) | | | | | | |

## 判定

(合格 / 不合格と、理由。× と △ の扱い。)
