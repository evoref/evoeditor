# リリースのチェックリスト — v0.4.0

docs/09 §8.2 の「リリースのチェックリスト」(上の表・v0.2 の表・v0.4 の表) と、08 §2.5 の v0.4 の追加ゲート (a)〜(e)、09 §9.2 手順 3。**タグの前に済む行だけ** を載せる。タグの後の結果 (配布物・タグの検証・配布物の受入) は Release のノートに書く。

次の行は `release.yml` が読む (先頭の 1 か所だけ)。

判定コミット: e4e6f15db568f5fdbb9d2784eb5bf8a4975e3fa3

- 版: 0.4.0 (正式版。`develop` → `main` で昇格する、docs/09 §9.2 手順 4)
- rc.2 からの変更: 版上げ (#573。workspace の版と CHANGELOG だけ) と、v0.3.0 の `main` の取り込み (#574。ファイルの差分なし)。`git diff 0c34ac7 e4e6f15` は `docs/compat/` を除くと `Cargo.toml`・`Cargo.lock` の版の行と `CHANGELOG.md`・`CHANGELOG.en.md` だけ。rc.2 の判定と記録は [`../0.4.0-rc.2/release-checklist.md`](../0.4.0-rc.2/release-checklist.md)
- **判定の結果: 合格 (PO の判定 2026-11-19)**。すべてのゲートが合格または対象外 (下書き: Claude)
- 判定した人 / 日付: 下書き: Claude / 2026-11-19。**最終の判定は PO** (この PR のマージをもって確認とする)
- 判定コミットの確認 (`git merge-base --is-ancestor develop <SHA>` と `git merge-base --is-ancestor main <SHA>` が真): [x] (2026-11-18。判定コミットは PR #574 (`main` の取り込み) のマージコミットで、`develop` の先頭。`main` (v0.3.0、0db83ed) はその祖先。版上げ #573 の後に v0.3.0 の取り込み漏れが見つかったので、#574 を入れてから判定コミットを決めた)
- 凍結: 判定から昇格・タグまで `develop` への merge を保留する (09 §9.2)。この後に入れてよい変更は `docs/compat/0.4.0/`・`docs/perf/0.4.0/`・`tests/ac-checklist.toml` だけ。ほかを入れたらゲートをやり直す。
- 流用 (PO の判断 2026-11-18、案 A): 判定コミットで CI・手動の夜間ジョブ 3 本・Linux の自動受入をやり直し、夜間 fuzz の観測は rc.2 の判定コミットでの 3 日分を流用する (製品のコードは rc.2 と同じ)。性能 (BM) と手動の確認 (G4・M・N・P・Linux の手の項目・NFR-601) は rc.1 の記録の流用を続ける (PO の判断 2026-11-16)

判定は 合格 / 不合格 / 対象外 (理由) / 保留 (結果待ち) のいずれか。基準機の型番など固有の情報は書かない (docs/11 §9.7)。

## v0.4 の追加ゲート (08 §2.5)

| ゲート | 条件の正本 | 判定 | 根拠 | 確認者 | 日付 |
|---|---|---|---|---|---|
| (a) v0.3 のゲートの続き | 試験とゴールデン、clippy と `missing_docs`、仕様の検査、`CONTRACT_VERSION` は 0 (08 §2.4 の (a)・(c)・(d)・(f)) | 合格 | 判定コミットの CI (全ジョブ成功): <https://github.com/evoref/evoeditor/actions/runs/44260738055>。rc.2 から既存のゴールデンの更新は無い。判定コミットの手動実行の proptest 拡張 (`PROPTEST_CASES=10000`) を含む試験 2,554 件がすべて成功: <https://github.com/evoref/evoeditor/actions/runs/44260852558>。判定コミットを対象にした定時の夜間ジョブ (2026-11-18 (UTC)) も成功: <https://github.com/evoref/evoeditor/actions/runs/44424384657> | Claude (下書き) | 2026-11-19 |
| (b) 往復コーパスのバイト一致 | 01 §5.5 の NFR-303 AC-5〜AC-7 と NFR-304 AC-4〜AC-6 の試験が通る | 合格 | 判定コミットの CI の `test (windows)`・`test (linux)` (上と同じ実行) | Claude (下書き) | 2026-11-19 |
| (c) 性能 | 基準機の BM が v0.3.0 の記録から劣化しない (差があれば理由を所見に書く)。BM-104v p50 ≤ 300 ms (Must)。BM-106 の 5 つの操作 p99 ≤ 50 ms (Should)。BM-103b-search-multiline(-1m) p50 2 / p99 8 ms (Must) | 合格 (rc.1 の記録を流用) | [`../../perf/0.4.0-rc.1/summary.md`](../../perf/0.4.0-rc.1/summary.md)。rc.1 からの製品の変更は #567 (保存の前の検査、`crates/eve-encoding/src/merge.rs`) だけで、BM の経路に入らない。BM-102a の劣化は rc.1 の所見のとおり v0.5 で直す (PO の判断 2026-11-15) | Claude (下書き)、PO の判断 | 2026-11-19 |
| (d) fuzz | 判定コミットで 5 つのターゲットをそれぞれ 1 時間回してクラッシュ 0。夜間 fuzz の観測 (3 日で可、PO の判断 2026-11-15) | 合格 | 判定コミットの手動実行 (`fuzz_seconds=3600`)。1 本目 `only=fuzz:fuzz_decode+fuzz_editor+fuzz_encode`: 3 つともクラッシュ 0 件: <https://github.com/evoref/evoeditor/actions/runs/44260795304>。2 本目 `only=fuzz:fuzz_regex_search+fuzz_vt_input,crash`: 2 つともクラッシュ 0 件: <https://github.com/evoref/evoeditor/actions/runs/44288453072>。観測は rc.2 の判定コミット (製品のコードは同じ) での定時の実行 3 日分 (2026-11-15/16/17 (UTC)、[`../0.4.0-rc.2/release-checklist.md`](../0.4.0-rc.2/release-checklist.md) の (d)) を流用 (PO の判断 2026-11-18)。加えて判定コミットでの定時の実行 (2026-11-18 (UTC)) でも 5 つともクラッシュ 0 件 (上の (a) と同じ実行) | Claude (下書き) | 2026-11-19 |
| (e) Could の 3 項目 | FR-110・FR-119・幅プローブは v0.5 以降に送った (08 §7) | 対象外 (v0.5 以降へ。PO の決定 2026-11-14) | 08 §7 の繰り越しの表 (#555) | Claude (下書き) | 2026-11-19 |

## 09 §8.2 のリリースのチェックリスト

| ゲート | 条件の正本 | 判定 | 根拠 | 確認者 | 日付 |
|---|---|---|---|---|---|
| 要件充足 | 09 §8.2 の v0.4 の表: 01 §5.5 の v0.4 の Must の AC に確かめる試験がある。`cargo xtask trace --version v0.4` の警告で見る | 合格 | 判定コミットの CI の `check` ジョブ (上と同じ実行)。rc.2 から AC と試験の対応は変わらない | Claude (下書き) | 2026-11-19 |
| 性能 | 09 §8.2。BM-1xx/2xx の Must が基準機で目標内 | 合格 (rc.1 の記録を流用) | 上の (c) | Claude (下書き)、PO の判断 | 2026-11-19 |
| 互換 (Windows) | 09 §8.2。Windows Terminal と conhost で手動シナリオ (G4・M・N) | 合格 (rc.1 の記録を流用) | rc.1 の配布物で PO が確かめた記録 ([`../0.4.0-rc.1/stdin.md`](../0.4.0-rc.1/stdin.md)・[`../0.4.0-rc.1/acceptance.md`](../0.4.0-rc.1/acceptance.md))。rc.2 の配布物で `--version` と起動・終了を PO が確かめた (2026-11-16)。0.4.0 の配布物の確認はタグの後に Release のノートに書く | PO | 2026-11-19 |
| 互換 (高コントラスト) | 09 §8.2 の v0.4 の表 | 合格 (rc.1 の記録を流用) | [`../0.4.0-rc.1/high-contrast.md`](../0.4.0-rc.1/high-contrast.md)。自動の検査は判定コミットの CI の `check-themes` (上と同じ実行) | PO、Claude (下書き) | 2026-11-19 |
| 互換 (Linux) | 09 §8.2。リリース候補の tar.gz で WSL2 の Windows Terminal と tmux の受入 | 合格 | [`linux-wsl.md`](linux-wsl.md): `cargo xtask accept-linux` の 26 項目すべて ○ (判定コミットで作った tar.gz `417236b2fd3f`、2026-11-18)。手の項目は rc.1 の記録を流用。G1 の判定がソースの置き場のパスの長さに左右されることを所見に書いた (製品ではなく検査の道具の問題) | Claude (下書き)、PO の判断 | 2026-11-19 |
| 互換 (英語 UI) | `cargo xtask i18n` が通り、`--locale en` で日本語が残らない | 合格 | 判定コミットの CI の `i18n` ジョブ (上と同じ実行) | Claude (下書き) | 2026-11-19 |
| 受入 | 09 §8.2。L7 の Must 項目と、NFR-601 の 3 名の簡易テスト | 合格 (v0.2.0 の記録を流用。PO の判断 2026-11-15) | rc.1 と同じ ([`../0.2.0-rc.1/acceptance.md`](../0.2.0-rc.1/acceptance.md)) | Claude (下書き)、PO の判断 | 2026-11-19 |
| データ保全 | 09 §8.2。kill テスト・往復コーパスなど | 合格 | 判定コミットの CI (上と同じ実行) の `test (windows)`・`test (linux)`・`e2e`・`e2e (linux)` | Claude (下書き) | 2026-11-19 |
| データ保全 (Unix) | Unix の kill テスト (Linux CI) と、判定コミットでの LazyFS のクラッシュ一貫性 (`only=crash`) が ext4 / xfs / btrfs で合格 | 合格 | Unix の kill テストは判定コミットの CI の `test (linux)` (上と同じ実行) で合格。LazyFS のクラッシュ一貫性は判定コミットの手動実行で ext4・xfs・btrfs とも 13 件すべて合格: <https://github.com/evoref/evoeditor/actions/runs/44288453072> (定時の実行 44424384657 でも同じ) | Claude (下書き) | 2026-11-19 |
| セキュリティ (依存) | `cargo audit` / `deny advisories` がクリーン | 合格 | 判定コミットの CI の `deps` ジョブ (上と同じ実行)。夜間の `cargo audit` と deny advisories は、判定コミットの手動実行 (`only=proptest,audit,soak`) で本体 (170 crate) と `fuzz/` (91 crate) の両方で成功 (RustSec の 1,533 件で照合): <https://github.com/evoref/evoeditor/actions/runs/44260852558>。本体の情報の警告は RUSTSEC-2026-0486 (`rustix` の unsound) の 1 件だけで、rc.1 の所見と同じく影響なし | Claude (下書き) | 2026-11-19 |
| セキュリティ (リリース前の確認) | 09 §8.2 の (a) DLL の検索順、(b) UNC パス | 合格 | (a) 判定コミットの CI の `build-release-smoke` (上と同じ実行)。(b) rc.1 から変更なし | Claude (下書き) | 2026-11-19 |
| fuzz | 09 §8.2 の v0.4 の表 | 合格 | 上の (d) | Claude (下書き) | 2026-11-19 |
| 長時間 (観察) | BM-306 の 1 時間の soak の推移を所見に書く (ゲートにしない) | 観察済み | 判定コミットで 60 分: RSS 37.3 → 39.3 MiB (傾きからの増加 0.6 MiB)、入力 → Frame の p99 0.181 → 0.178 ms、「平坦」(上の「セキュリティ (依存)」と同じ実行)。定時の実行 (44424384657) でも 32.9 → 33.7 MiB (増加 0.9 MiB)、p99 0.258 → 0.255 ms、「平坦」 | Claude | 2026-11-19 |
| MSRV | CI の `msrv` ジョブが判定コミットで通る | 合格 | 判定コミットの CI の `msrv` ジョブ (上と同じ実行) | Claude (下書き) | 2026-11-19 |
| 契約 | `CONTRACT_VERSION` は 0 のまま。`eve-view` への追加は `#[non_exhaustive]` の型への追加だけ (C2) | 合格 | rc.1 から `eve-view` の変更なし (版の番号を除く) | Claude (下書き) | 2026-11-19 |
| 依存 | C を含む依存 0 (`cargo xtask check-deps`) | 合格 | 判定コミットの CI の `deps` ジョブ (上と同じ実行) | Claude (下書き) | 2026-11-19 |
| ドキュメント | README・CHANGELOG・生成リファレンスが最新。英語版 | 合格 | CHANGELOG の `[0.4.0] - 2026-11-19` (ja / en、#573)。rc.1 の内容を `[0.4.0]` に移し、rc.1 / rc.2 の節は要約にした (0.2.0 と同じ形)。生成リファレンスは判定コミットの CI の `check` ジョブ (上と同じ実行) | Claude (下書き) | 2026-11-19 |
| ライセンス・名称 | 12 §3.8 の LR-1 と LR-4 | 合格 | rc.1 から変更なし。依存の追加は無い | Claude (下書き) | 2026-11-19 |
| 未決事項の解消 | 当該版の Must に影響する OQ がすべて回答済み (08 §5) | 合格 | rc.1 と同じ | Claude (下書き) | 2026-11-19 |

## 所見・例外

- **rc.1・rc.2 の所見の引き継ぎ**: BM-102a の劣化 (v0.5 で直す)、BM-106 と機械の状態、BM-101a-locale、RUSTSEC-2026-0486 は [`../0.4.0-rc.1/release-checklist.md`](../0.4.0-rc.1/release-checklist.md) の所見のとおりで、0.4.0 で変わらない。BM-102a の繰り越し (08 §7) と `rustix` の更新は、昇格の後に入れる。
- **v0.3.0 の `main` の取り込み漏れ**: v0.3.0 のリリース後の `main` → `develop` の取り込み (09 §9.2 手順 4) が済んでいなかった (v0.1.0 は #171、v0.2.0 は #338 で入れていた)。0.4.0 の版上げ (#573) の後に、判定コミットの条件 (`main` の先頭が祖先) を確かめて見つけた。#574 (人が merge commit で入れた。ファイルの差分なし) で取り込み、そのマージコミットを判定コミットにした。昇格の後は、0.4.0 の `main` を `develop` に取り込む PR を忘れずに入れる。
- **CHANGELOG の日付**: `[0.4.0]` の日付は昇格の見込みの日 (2026-11-19) を版上げで入れた。判定の後は CHANGELOG を変えられないため。
- **このファイルの「Claude (下書き)」の判定**: 最終の判定は PO。この PR のマージをもって PO の確認とする。
