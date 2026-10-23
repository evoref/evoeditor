# 互換性の記録 — conhost (v0.1.0-rc.1)

手順書: [`../checklist.md`](../checklist.md)。基準機の固有情報は書かない。conhost は機能が Must、描画品質は best effort (05 §9.4)。

- 確認した人 / 日付: PO / 2026-10-24
- 判定したコミットと使った配布物: `3de957f` から `cargo xtask dist` で作った `evoeditor-0.1.0-rc.1-x86_64-pc-windows-msvc.zip` (SHA-256 先頭 12 桁 `8a4ae6daede4`) を展開した exe
- Windows: Windows 11 (基準機 ENV-W1)
- 日本語入力: Microsoft IME と Google 日本語入力の両方
- 端末: conhost (コンソールホスト)

**記録の粒度**: PO から「手順書 (`docs/compat/checklist.md`) の手動の項目は **すべて OK**」と報告を受けた。行ごとの ○ / △ / × は個別には
書き取っていない (△・× は無かったので Issue も無い)。手順書の A・H などは自動テストの結果で代替した ([`automated-coverage.md`](automated-coverage.md))。

## 判定

**合格**。手動の項目がすべて OK。
