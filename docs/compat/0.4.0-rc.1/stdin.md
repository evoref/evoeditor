# 標準入力の確認の記録 (v0.4.0-rc.1)

手順は [`../checklist.md`](../checklist.md) の N (05 §9.4 の「標準入力の手での確認」)。基準機の固有情報 (機種名・型番・OS のビルド番号・ウイルス対策ソフトの製品名・ユーザー名を含むパス) は書かない。

- 確認した人 / 日付: PO / 2026-11-16
- 判定したコミットと、使った配布物の SHA-256 (先頭 12 桁): `90b2c81` / Windows の zip `9582d12a60da`、Linux の tar.gz `c4031c3b893a` (release.yml が作った draft の Release の配布物)
- Windows / WSL のディストリビューション / ssh の接続先の OS: Windows 11 / Ubuntu 26.04 LTS / (ssh は確かめていない)
- 端末とシェル: Windows Terminal、conhost、VS Code の統合端末、mintty (Git Bash)、WSL の Windows Terminal と tmux 3.6

○ = 期待どおり、△ = 動くが気になる点あり、× = 期待どおりでない、— = その環境に当てはまらない、未 = 確かめていない。△ と × は Issue 番号を備考に。

| 環境 | N1 閉じるパイプ | N2 閉じないパイプと `Esc` | N3 書き手のパスワード | N4 端末の判定 | N5 読込中のサスペンド | 備考 (N2 の書き手の止まり方など) |
|---|---|---|---|---|---|---|
| Windows Terminal (PowerShell 7) | ○ | ○ | — | ○ | — | |
| Windows Terminal (cmd) | ○ | ○ | — | ○ | — | |
| conhost | ○ | ○ | — | ○ | — | |
| VS Code の統合端末 | ○ | ○ | — | ○ | — | |
| mintty (Git Bash) | ○ | ○ | — | ○ | — | |
| Windows PowerShell 5.1 のパイプ | 未 | 未 | — | 未 | — | 確かめていない |
| WSL からの interop (`cmd.exe /c`) | 未 | 未 | — | 未 | — | 確かめていない |
| WSL の側のパイプ (Windows Terminal) | ○ | ○ | ○ | ○ | ○ | |
| tmux の中 (WSL) | ○ | ○ | ○ | ○ | ○ | |
| `ssh -t` 越し (Linux のホスト) | 未 | 未 | 未 | 未 | 未 | 確かめていない |

## 判定

合格。確かめた環境 (Windows Terminal (PowerShell 7・cmd)・conhost・VS Code の統合端末・mintty・WSL の Windows Terminal と tmux) では、N1〜N5 がすべて期待どおり (PO の報告 2026-11-16)。× と △ は無い。Windows PowerShell 5.1 のパイプ・WSL からの interop・`ssh -t` 越しは今回は確かめていない (次の版の確認で見る)。
