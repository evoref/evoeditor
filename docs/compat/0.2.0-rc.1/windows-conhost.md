# 互換性の記録 — conhost (v0.2.0-rc.1)

手順書: [`../checklist.md`](../checklist.md)。基準機の固有情報は書かない。

- 確認した人 / 日付: PO / 2026-11-03
- 判定したコミットと使った配布物: `ced9f04` (判定コミット) / 公開した Release `v0.2.0-rc.1` の `evoeditor-0.2.0-rc.1-x86_64-pc-windows-msvc.zip` (SHA-256 先頭 12 桁 `4e1975fa461b`。`SHA256SUMS` と一致) を展開した exe
- Windows: Windows 11 (基準機 ENV-W1)
- 日本語入力: Microsoft IME と Google 日本語入力の両方
- 端末: conhost

**記録の粒度**: PO から「公開した zip で、手動シナリオ (開く・日本語入力 (Microsoft IME・Google 日本語入力)・検索・保存・終了) と H1〜H3 (異常終了と退避) は **すべて OK**」と報告を受けた。行ごとの ○ / △ / × は個別には書き取っていない (△・× は無かったので Issue も無い)。端末の中の動作の多くは、判定コミットの CI の `e2e` と `test (windows)` でも確かめている (<https://github.com/evoref/evoeditor/actions/runs/41569572044>)。

## 判定

**合格**。手動の項目がすべて OK。
