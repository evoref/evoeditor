# リリースのチェックリスト — v0.4.0-rc.2

docs/09 §8.2 の「リリースのチェックリスト」(上の表・v0.2 の表・v0.4 の表) と、08 §2.5 の v0.4 の追加ゲート (a)〜(e)、09 §9.2 手順 3。**タグの前に済む行だけ** を載せる。タグの後の結果 (配布物・タグの検証・配布物の受入) は Release のノートに書く。

次の行は `release.yml` が読む (先頭の 1 か所だけ)。

判定コミット: 0c34ac70a2d8a4fdd98455d69b0a62c8e0c89974

- 版: 0.4.0-rc.2 (プレリリース。タグは `develop` の先頭に人が付ける、09 §9.2)
- rc.1 からの変更: 夜間の性質テストが見つけた UTF-16 のつながりの範囲の誤りの修正 (#567) と、版上げ (#569) だけ。rc.1 の判定と記録は [`../0.4.0-rc.1/release-checklist.md`](../0.4.0-rc.1/release-checklist.md)
- **判定の結果: 合格 (PO の判定 2026-11-18)**。すべてのゲートが合格または対象外 (下書き: Claude)
- 判定した人 / 日付: 下書き: Claude / 2026-11-16。**最終の判定は PO** (この PR のマージをもって確認とする)
- 判定コミットの確認: [x] (2026-11-16。判定コミットは PR #569 (版上げ) のマージコミットで、`develop` の先頭)
- 凍結: 判定から昇格・タグまで `develop` への merge を保留する (09 §9.2)。この後に入れてよい変更は `docs/compat/0.4.0-rc.2/`・`docs/perf/0.4.0-rc.2/`・`tests/ac-checklist.toml` だけ。ほかを入れたらゲートをやり直す。
- 流用 (PO の判断 2026-11-16): 性能 (BM) と手動の確認 (G4・M・N・P・Linux の手の項目・NFR-601) は rc.1 の記録を流用する。rc.1 からの変更は保存の確認の文言に出す範囲の数え方だけで、性能・端末との入出力・画面の経路は変わらない。rc.2 の配布物では `--version` と起動・終了を PO が確かめた (2026-11-16、期待どおり)

判定は 合格 / 不合格 / 対象外 (理由) / 保留 (結果待ち) のいずれか。基準機の型番など固有の情報は書かない (docs/11 §9.7)。

## v0.4 の追加ゲート (08 §2.5)

| ゲート | 条件の正本 | 判定 | 根拠 | 確認者 | 日付 |
|---|---|---|---|---|---|
| (a) v0.3 のゲートの続き | 試験とゴールデン、clippy と `missing_docs`、仕様の検査、`CONTRACT_VERSION` は 0 (08 §2.4 の (a)・(c)・(d)・(f)) | 合格 | 判定コミットの CI (全ジョブ成功): <https://github.com/evoref/evoeditor/actions/runs/43767889322>。rc.1 から既存のゴールデンの更新は無い。判定コミットの手動実行の proptest 拡張 (`PROPTEST_CASES=10000`) を含む試験 2,554 件がすべて成功: <https://github.com/evoref/evoeditor/actions/runs/43785252824>。定時の夜間ジョブ 3 日分も成功 (下の (d)) | Claude (下書き) | 2026-11-18 |
| (b) 往復コーパスのバイト一致 | 01 §5.5 の NFR-303 AC-5〜AC-7 と NFR-304 AC-4〜AC-6 の試験が通る | 合格 | 判定コミットの CI の `test (windows)`・`test (linux)` (上と同じ実行) | Claude (下書き) | 2026-11-16 |
| (c) 性能 | 基準機の BM が v0.3.0 の記録から劣化しない (差があれば理由を所見に書く)。BM-104v p50 ≤ 300 ms (Must)。BM-106 の 5 つの操作 p99 ≤ 50 ms (Should)。BM-103b-search-multiline(-1m) p50 2 / p99 8 ms (Must) | 合格 (rc.1 の記録を流用) | [`../../perf/0.4.0-rc.1/summary.md`](../../perf/0.4.0-rc.1/summary.md)。rc.1 からの変更 (#567) は保存の前の検査 (`crates/eve-encoding/src/merge.rs`) だけで、BM の経路に入らない。BM-102a の劣化は rc.1 の所見のとおり v0.5 で直す (PO の判断 2026-11-15) | Claude (下書き)、PO の判断 | 2026-11-16 |
| (d) fuzz | 判定コミットで 5 つのターゲットをそれぞれ 1 時間回してクラッシュ 0。夜間 fuzz の観測 (3 日で可、PO の判断 2026-11-15) | 合格 | 判定コミットの手動実行 (`fuzz_seconds=3600`)。1 本目 `only=fuzz:fuzz_decode+fuzz_editor+fuzz_encode`: 3 つとも 3600 秒でクラッシュ 0 件: <https://github.com/evoref/evoeditor/actions/runs/43767943963>。2 本目 `only=fuzz:fuzz_regex_search+fuzz_vt_input,crash`: 2 つとも 3600 秒でクラッシュ 0 件: <https://github.com/evoref/evoeditor/actions/runs/43804078213>。判定コミットを対象にした定時の実行の 3 日分: 1 日目 (2026-11-15 (UTC)) 成功 <https://github.com/evoref/evoeditor/actions/runs/43769461208>。2 日目 (2026-11-16 (UTC)) 成功 <https://github.com/evoref/evoeditor/actions/runs/43996598086>。3 日目 (2026-11-17 (UTC)) 成功 <https://github.com/evoref/evoeditor/actions/runs/44208474811>。3 日とも `develop` の先頭 = 判定コミットを試験し、fuzz の 5 つはクラッシュ 0 件 | Claude (下書き) | 2026-11-18 |
| (e) Could の 3 項目 | FR-110・FR-119・幅プローブは v0.5 以降に送った (08 §7) | 対象外 (v0.5 以降へ。PO の決定 2026-11-14) | 08 §7 の繰り越しの表 (#555) | Claude (下書き) | 2026-11-16 |

## 09 §8.2 のリリースのチェックリスト

| ゲート | 条件の正本 | 判定 | 根拠 | 確認者 | 日付 |
|---|---|---|---|---|---|
| 要件充足 | 09 §8.2 の v0.4 の表: 01 §5.5 の v0.4 の Must の AC に確かめる試験がある。`cargo xtask trace --version v0.4` の警告で見る | 合格 | 判定コミットの CI の `check` ジョブ (上と同じ実行)。rc.1 から AC と試験の対応は変わらない (#567 は試験の追加だけ) | Claude (下書き) | 2026-11-16 |
| 性能 | 09 §8.2。BM-1xx/2xx の Must が基準機で目標内 | 合格 (rc.1 の記録を流用) | 上の (c) | Claude (下書き)、PO の判断 | 2026-11-16 |
| 互換 (Windows) | 09 §8.2。Windows Terminal と conhost で手動シナリオ (G4・M・N) | 合格 (rc.1 の記録を流用) | rc.1 の配布物で PO が確かめた記録 ([`../0.4.0-rc.1/stdin.md`](../0.4.0-rc.1/stdin.md)・[`../0.4.0-rc.1/acceptance.md`](../0.4.0-rc.1/acceptance.md))。rc.2 の draft の Release の Windows の zip で、`--version` (`evoeditor 0.4.0-rc.2`) と起動・終了を PO が確かめた (2026-11-16) | PO | 2026-11-16 |
| 互換 (高コントラスト) | 09 §8.2 の v0.4 の表 | 合格 (rc.1 の記録を流用) | [`../0.4.0-rc.1/high-contrast.md`](../0.4.0-rc.1/high-contrast.md)。自動の検査は判定コミットの CI の `check-themes` (上と同じ実行) | PO、Claude (下書き) | 2026-11-16 |
| 互換 (Linux) | 09 §8.2。リリース候補の tar.gz で WSL2 の Windows Terminal と tmux の受入 | 合格 | [`linux-wsl.md`](linux-wsl.md): `cargo xtask accept-linux` の 26 項目すべて ○ (判定コミットで作った tar.gz `b5fdebf80613`、2026-11-16)。手の項目は rc.1 の記録を流用 | Claude (下書き)、PO の判断 | 2026-11-16 |
| 互換 (英語 UI) | `cargo xtask i18n` が通り、`--locale en` で日本語が残らない | 合格 | 判定コミットの CI の `i18n` ジョブ (上と同じ実行) | Claude (下書き) | 2026-11-16 |
| 受入 | 09 §8.2。L7 の Must 項目と、NFR-601 の 3 名の簡易テスト | 合格 (v0.2.0 の記録を流用。PO の判断 2026-11-15) | rc.1 と同じ ([`../0.2.0-rc.1/acceptance.md`](../0.2.0-rc.1/acceptance.md)) | Claude (下書き)、PO の判断 | 2026-11-16 |
| データ保全 | 09 §8.2。kill テスト・往復コーパスなど | 合格 | 判定コミットの CI (上と同じ実行) の `test (windows)`・`test (linux)`・`e2e`・`e2e (linux)` | Claude (下書き) | 2026-11-16 |
| データ保全 (Unix) | Unix の kill テスト (Linux CI) と、判定コミットでの LazyFS のクラッシュ一貫性 (`only=crash`) が ext4 / xfs / btrfs で合格 | 合格 | Unix の kill テストは判定コミットの CI の `test (linux)` (上と同じ実行) で合格。LazyFS のクラッシュ一貫性は判定コミットの手動実行で ext4・xfs・btrfs とも 13 件すべて合格: <https://github.com/evoref/evoeditor/actions/runs/43804078213> | Claude (下書き) | 2026-11-16 |
| セキュリティ (依存) | `cargo audit` / `deny advisories` がクリーン | 合格 | 判定コミットの CI の `deps` ジョブ (上と同じ実行)。夜間の `cargo audit` と deny advisories は、判定コミットの手動実行 (`only=proptest,audit,soak`) で本体 (170 crate) と `fuzz/` (91 crate) の両方で成功 (RustSec の 1,509 件で照合): <https://github.com/evoref/evoeditor/actions/runs/43785252824>。本体の情報の警告 RUSTSEC-2026-0486 (`rustix` の unsound) は rc.1 の所見と同じで影響なし | Claude (下書き) | 2026-11-16 |
| セキュリティ (リリース前の確認) | 09 §8.2 の (a) DLL の検索順、(b) UNC パス | 合格 | (a) 判定コミットの CI の `build-release-smoke` (上と同じ実行)。(b) rc.1 から変更なし | Claude (下書き) | 2026-11-16 |
| fuzz | 09 §8.2 の v0.4 の表 | 合格 | 上の (d) | Claude (下書き) | 2026-11-18 |
| 長時間 (観察) | BM-306 の 1 時間の soak の推移を所見に書く (ゲートにしない) | 観察済み | 判定コミットで 60 分: RSS 37.6 → 38.7 MiB (傾きからの増加 -0.2 MiB)、入力 → Frame の p99 0.180 → 0.179 ms、「平坦」(上の「セキュリティ (依存)」と同じ実行) | Claude | 2026-11-16 |
| MSRV | CI の `msrv` ジョブが判定コミットで通る | 合格 | 判定コミットの CI の `msrv` ジョブ (上と同じ実行) | Claude (下書き) | 2026-11-16 |
| 契約 | `CONTRACT_VERSION` は 0 のまま。`eve-view` への追加は `#[non_exhaustive]` の型への追加だけ (C2) | 合格 | rc.1 から `eve-view` の変更なし | Claude (下書き) | 2026-11-16 |
| 依存 | C を含む依存 0 (`cargo xtask check-deps`) | 合格 | 判定コミットの CI の `deps` ジョブ (上と同じ実行) | Claude (下書き) | 2026-11-16 |
| ドキュメント | README・CHANGELOG・生成リファレンスが最新。英語版 | 合格 | CHANGELOG の `[0.4.0-rc.2]` (ja / en、#569)。生成リファレンスは判定コミットの CI の `check` ジョブ (上と同じ実行) | Claude (下書き) | 2026-11-16 |
| ライセンス・名称 | 12 §3.8 の LR-1 と LR-4 | 合格 | rc.1 から変更なし。依存の追加は無い | Claude (下書き) | 2026-11-16 |
| 未決事項の解消 | 当該版の Must に影響する OQ がすべて回答済み (08 §5) | 合格 | rc.1 と同じ | Claude (下書き) | 2026-11-16 |

## 所見・例外

- **rc.1 の所見の引き継ぎ**: BM-102a の劣化 (v0.5 で直す)、BM-106 と機械の状態、BM-101a-locale、RUSTSEC-2026-0486 は [`../0.4.0-rc.1/release-checklist.md`](../0.4.0-rc.1/release-checklist.md) の所見のとおりで、rc.2 で変わらない。BM-102a の繰り越しと `rustix` の更新は、昇格の後に入れる。
- **rc.1 の不合格の理由の修正**: UTF-16 の文書の保存の確認に出す「変わる範囲」が実際より大きく出る誤りは #567 で直した。並びの局所の検査が決まるまで範囲を閉じない。範囲の候補は、検査が決まらない間も少数に保つ (03 §9.3 のメモリの目安)。夜間の反例を固定した試験と、20,000 バイトの並びで候補が増えないことの試験を足した。判定コミットの proptest 拡張 (上の (a)) で同じ性質のテストが成功。
- **夜間ワークフローの実行の分け方**: rc.1 と同じく、fuzz の 5 ターゲット × 1 時間は 1 回の実行の上限 (240 分) に収まらないので `only=` で分けた。同じ concurrency の群では待ちの実行が 1 つしか残らないので、待ちが無いときにだけ次を入れた。
- **このファイルの「Claude (下書き)」の判定**: 最終の判定は PO。この PR のマージをもって PO の確認とする。
