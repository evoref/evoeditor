# 受入の記録 (v0.1.0-rc.1)

手順書: [`../checklist.md`](../checklist.md) の H と I。基準機の固有情報は書かない。

- 確認した人 / 日付: PO / 2026-10-24
- 判定したコミット: `3de957f`

## H. 異常終了と退避

| 項目 | 結果 | 備考 |
|---|---|---|
| H1 タブ / ウィンドウを閉じる | ○ | Windows Terminal のタブを実際に閉じて確認 (PO)。conhost は自動テスト (`CTRL_CLOSE_EVENT` を直接送る) |
| H2 次の起動での案内 | ○ | 自動テスト ([`automated-coverage.md`](automated-coverage.md)) |
| H3 メインが詰まった状態 | ○ | 自動テスト (`main_stall`)。基準機での 100 MB の退避は 0.61 秒 (`EVE_PERF_STRICT=1`、NFR-302 AC-5) |

## I. 受入

| 項目 | 結果 | 備考 |
|---|---|---|
| I1 説明書なしで 1 行追加して保存して終了 | ○ | PO |
| I2 README のインストール手順どおりに起動 | ○ | PO (手動の項目として確認)。タグ付けの後に、GitHub の Release から取得した zip でも 1 回確認する (リリースのノートの「検証」) |
| I3 SmartScreen / Smart App Control の表示 | ○ | PO が手動の項目として確認 (表示の有無の詳細は書き取っていない)。Release の zip での確認はノートの「検証」に書く |

## 判定

**合格**。
