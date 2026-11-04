# リリースのチェックリスト — v0.2.0

docs/09 §8.2 の「リリースのチェックリスト」(v0.2 の判定の表を含む)、§9.2 手順 3。**タグの前に済む行だけ** を載せる。タグの後の結果 (配布物・タグの検証・配布経路の検証・公開後の minisign の検証) は GitHub Release のノートの「検証」の節に書き、このファイルには戻さない。

次の行は `release.yml` が読む (先頭の 1 か所だけ)。

判定コミット: 6728fdedc27af13812175bc7c6137c5c7ca92025

- 版: 0.2.0 (正式版。`develop` → `main` で昇格する、docs/09 §9.2 手順 4)
- 判定した人 / 日付: 下書き: Claude / 2026-11-04。**最終の判定は PO** (この PR のマージをもって確認とする)
- 判定コミットの確認 (`git merge-base --is-ancestor develop <SHA>` と `git merge-base --is-ancestor main <SHA>` が真): [x] (2026-11-04。判定コミットは PR #333 (CHANGELOG の日付) のマージコミットで、その時点の `develop` の先頭。`main` の先頭はその祖先)
- この後に入れてよい変更は `docs/compat/0.2.0/`・`docs/perf/0.2.0/`・`tests/ac-checklist.toml` だけ。ほかを入れたらゲートをやり直す。
- rc.1 からの製品の変更: `git diff v0.2.0-rc.1 6728fde -- crates bins xtask` (ベンチを除く) は、版の更新・依存の更新 (`signal-hook` 0.4.6・`unicode-width` 0.2.3・`encoding_rs` 0.8.44) と `crates/eve-text/src/width.rs` (半角カナの濁点・半濁点の幅を `unicode-width` の版に依らず 1 にする、#324) だけ。

判定は 合格 / 不合格 / 対象外 (理由) のいずれか。基準機の型番など固有の情報は書かない (docs/11 §9.7)。

| ゲート | 条件の正本 | 判定 | 根拠 | 確認者 | 日付 |
|---|---|---|---|---|---|
| 要件充足 | 09 §8.2 (v0.2): 01 §5.5 の v0.2 の Must の AC が `cargo xtask trace --check` で揃い、`docs/traceability.md` に未対応が無い。v0.1 の Must は `tests/ac-checklist.toml` | 合格 | `cargo xtask trace --check`: OK (警告 0 件、判定コミット)。`tests/ac-checklist.toml` (v0.1 の Must): done 169 件、todo 0 件 | Claude (下書き) | 2026-11-04 |
| 性能 | 09 §8.2。BM-1xx/2xx の Must が基準機で目標内 (p50/p99)。v0.2: CI の `perf (instr)` / `perf (time)` に劣化の警告が無い。Linux の BM は報告のみ | **合格 (例外 1 件)** | [`docs/perf/0.2.0/summary.md`](../../perf/0.2.0/summary.md) (基準機 ENV-W1、判定コミット)。判定の 34 行のうち 33 行が合格。rc.1 の不合格の **BM-101b は p99 37.59 ms で解消**。**BM-103b-search** (100 MB の全件検索中のキー入力) の p50 2.44 ms が目標 2 ms を超えたが、rc.1 と同じ機械で交互に測ると差が無く (どちらも p50 約 1.9〜2.0 ms)、製品の劣化ではない。**PO 判断 (2026-11-04): 例外として v0.2.0 を出し、余裕を作る改善は v0.2.x 以降 (#334)**。1 回目の計測は計測の条件 (電源モード・重いアプリ) を満たしていなかったので記録にせず、条件を直して測り直した (summary.md の所見)。CI の perf: 版上げの PR #332 で `perf (instr)` / `perf (time)` 成功 (<https://github.com/evoref/evoeditor/actions/runs/41779048937>) | Claude (下書き)、PO の判断 | 2026-11-04 |
| 互換 (Windows) | 09 §8.2。Windows Terminal と conhost で手動シナリオ (開く・日本語入力 (Microsoft IME・Google 日本語入力)・検索・保存・終了) | 合格 (rc.1 の記録を流用) | 製品の変更は上の「rc.1 からの製品の変更」だけで、画面と入力の経路は変わっていない。記録は [`../0.2.0-rc.1/windows-terminal.md`](../0.2.0-rc.1/windows-terminal.md)・[`../0.2.0-rc.1/windows-conhost.md`](../0.2.0-rc.1/windows-conhost.md)。判定コミットの CI の `e2e` (ConPTY) は成功 | Claude (下書き) | 2026-11-04 |
| 互換 (Linux、v0.2 から) | 09 §8.2 の v0.2 の表。リリース候補の tar.gz で WSL2 の Windows Terminal と tmux の受入 | 合格 (rc.1 の記録を流用) | 記録は [`../0.2.0-rc.1/linux-wsl.md`](../0.2.0-rc.1/linux-wsl.md)。rc.1 の後の `signal-hook` 0.4.6 への更新 (#328) は、WSL で全試験 (シグナルで退避して同じシグナルで終わる e2e を含む) と tmux 上の停止・再開 (SIGTSTP / `fg`) と SIGHUP の退避を確かめた (#328 の PR)。判定コミットの CI の `test (linux)`・`e2e (linux)` は成功 | Claude (下書き) | 2026-11-04 |
| 互換 (英語 UI、v0.2 から) | `cargo xtask i18n` が通り、`--locale en` で日本語が残らない | 合格 | 判定コミットの CI の `i18n` ジョブ (<https://github.com/evoref/evoeditor/actions/runs/41781045647>)。`--locale en` の一巡は rc.1 の [`../0.2.0-rc.1/acceptance.md`](../0.2.0-rc.1/acceptance.md) の「英語 UI」(文言の変更なし) | Claude | 2026-11-04 |
| 受入 | 09 §8.2。L7 の Must 項目と、v0.2 の NFR-601 の 3 名の簡易テスト | 合格 (rc.1 の記録を流用) | [`../0.2.0-rc.1/acceptance.md`](../0.2.0-rc.1/acceptance.md): H1〜H3 と I1〜I3 は ○、NFR-601 は 3 / 3 成功 (`docs/review/usability-v0.2.md` §3)。rc.1 から操作と文言の変更なし | Claude (下書き) | 2026-11-04 |
| データ保全 | 09 §8.2。kill テスト・往復コーパスなど | 合格 | 判定コミットの CI (全ジョブ成功): <https://github.com/evoref/evoeditor/actions/runs/41781045647> — `test (windows)`・`e2e` (Windows の kill テスト・往復コーパス・緊急退避) | Claude (下書き) | 2026-11-04 |
| データ保全 (Unix、v0.2 から) | Unix の kill テスト (Linux CI) と、判定コミットでの LazyFS のクラッシュ一貫性 (`only=crash`) が ext4 / xfs / btrfs で合格。スワップと復旧のシナリオと kill テストが通る | 合格 | Unix の kill テスト・スワップと復旧: 上の CI の `test (linux)`・`e2e (linux)`。LazyFS: 判定コミットで ext4 / xfs / btrfs とも 13 件すべて合格: <https://github.com/evoref/evoeditor/actions/runs/41781299705> | Claude (下書き) | 2026-11-04 |
| セキュリティ (依存) | `cargo audit` / `deny advisories` がクリーン (例外は `deny.toml` に期限と理由つき) | 合格 | 判定コミットの CI の `deps` ジョブ (上と同じ実行)。夜間の `cargo audit` と deny advisories: 本体と `fuzz/` の両方でクリーン (RustSec の 1,403 件で照合): <https://github.com/evoref/evoeditor/actions/runs/41781299705> | Claude (下書き) | 2026-11-04 |
| セキュリティ (リリース前の確認) | 09 §8.2 の (a) DLL の検索順、(b) UNC パス | 合格 | (a) 判定コミットの CI の `build-release-smoke` (`check-dist` が `DependentLoadFlags` = 0x800 を検査) と `e2e` (偽の DLL を同じフォルダとカレントに置いた起動で読み込まれない)。(b) 到達できない UNC で UI が止まらない e2e (同じ CI) と、README の既知の制約。手順は v0.1.0-rc.1 の [`security-checks.md`](../0.1.0-rc.1/security-checks.md) と同じ | Claude (下書き) | 2026-11-04 |
| fuzz | 09 §8.2 (v0.2): 判定コミットで `fuzz_decode` と `fuzz_editor` をそれぞれ 1 時間回してクラッシュ 0。夜間 fuzz の観測 (正式版は 5 日) | 合格 | 判定コミットで各 3600 秒、**クラッシュ 0 件** (seed 20761): <https://github.com/evoref/evoeditor/actions/runs/41781279516>。**夜間 fuzz の 5 日の観測は、PO 判断 (2026-11-04) により今回に限り免除**。凍結 (rc.1 の版上げ、2026-11-03) の後の定期実行は 2 回で、どちらも成功 (クラッシュ 0): <https://github.com/evoref/evoeditor/actions/runs/41553311229> (2026-11-03 00:30 UTC 開始)、<https://github.com/evoref/evoeditor/actions/runs/41734741075> (2026-11-03 22:54 UTC 開始)。ほかに rc.1 の判定コミットでの各 1 時間 (<https://github.com/evoref/evoeditor/actions/runs/41582763034>、クラッシュ 0) | Claude (下書き)、PO の判断 | 2026-11-04 |
| 長時間 (観察、v0.2 から) | BM-306 の 1 時間の soak の推移を所見に書く (ゲートにしない) | 観察済み | 判定コミットで 60 分: RSS 23.0 → 23.1 MiB (増加 -0.2 MiB)、入力 → Frame の p99 0.486 → 0.487 ms、「平坦」(<https://github.com/evoref/evoeditor/actions/runs/41781299705>) | Claude | 2026-11-04 |
| MSRV (v0.2 から) | CI の `msrv` ジョブが判定コミットで通る | 合格 | 判定コミットの CI の `msrv` ジョブ (<https://github.com/evoref/evoeditor/actions/runs/41781045647>) | Claude | 2026-11-04 |
| 契約 | `eve-view` の型が `#[non_exhaustive]` で `CONTRACT_VERSION` を持つ | 合格 | `crates/eve-view/` は rc.1 から変わっていない。`pub const CONTRACT_VERSION: u32 = 0;`、公開の型は `#[non_exhaustive]` | Claude (下書き) | 2026-11-04 |
| 依存 | C を含む依存 0 (`cargo xtask check-deps`) | 合格 | 判定コミットの CI の `deps` ジョブ (`cargo xtask check-deps --docs`)。上と同じ実行 | Claude (下書き) | 2026-11-04 |
| ドキュメント | README・CHANGELOG・生成リファレンスが最新。v0.2: 英語版 (`dist/README.en.md`・`CHANGELOG.en.md`) と README の Linux の導入・更新・既知の制約 | 合格 (生成リファレンスは対象外) | `CHANGELOG.md` / `CHANGELOG.en.md` の `[0.2.0] - 2026-11-04` (#332・#333)。`dist/README.md` / `README.en.md` は rc.1 から変更なし。生成リファレンスは v0.3 に送った (PO 判断 2026-11-04、08 §7・09 §11.2) | Claude (下書き) | 2026-11-04 |
| ライセンス・名称 | 12 §3.8 の LR-1 と LR-4。結果は 10 の OQ-01 | 合格 | v0.1.0-rc.1 の確認から変更なし (LR-1: `docs/review/lr-1-memo.md`、LR-4: 2026-10-24 実施)。`THIRD-PARTY-NOTICES.md` は判定コミットの CI の `deps` で最新 (`gen-notices --check`) | Claude (下書き) | 2026-11-04 |
| 未決事項の解消 | 当該版の Must に影響する OQ がすべて回答済み (08 §5) | 合格 | rc.1 で提案中だった PO の区分の OQ-112 (Linux の配布物) は確定 (PO 判断 2026-11-04: musl 静的版だけ、gnu 版は v0.4。#331)。材料は `docs/perf/0.2.0-rc.1/oq-112-musl-gnu.md`。裁量の区分の提案中の OQ は 10 §0.5 により推奨で確定扱い (rc.1 と同じ) | Claude (下書き) | 2026-11-04 |

## 所見・例外

- **性能 (BM-103b-search)**: 上の行と [`summary.md`](../../perf/0.2.0/summary.md) の所見のとおり。例外 1 件 (PO 判断)。
- **夜間 fuzz の観測**: 5 日の観測は今回に限り免除 (PO 判断 2026-11-04)。次の版では §8.2 のとおり観測する。
- **互換・受入の記録の流用**: rc.1 の後の製品の変更は依存の更新と文字幅の調整だけなので、手の確認は rc.1 の記録 (公開した zip / tar.gz での PO の確認) を流用した。**PO 判断 (2026-11-04): rc.1 の記録の流用で十分** (v0.2.0 の配布物での手の再確認はしない)。
- **このファイルの「Claude (下書き)」の判定**: 最終の判定は PO。この PR のマージをもって PO の確認とする。
