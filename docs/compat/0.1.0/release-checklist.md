# リリースのチェックリスト — v0.1.0

雛形: [`../_template/release-checklist.md`](../_template/release-checklist.md) (docs/09 §8.2 の「リリースのチェックリスト」、§9.2 手順 3)。
**タグの前に済む行だけ** を載せる。タグの後の結果 (配布物・タグの検証・MSRV・配布経路の検証・更新手順の文書の確認・公開後の minisign の検証) は
GitHub Release のノートの「検証」の節に書き、このファイルには戻さない。

次の行は `release.yml` が読む (先頭の 1 か所だけ)。

判定コミット: a9b52481734af31d1427ebd6239faa39461d8bb4

- 版: 0.1.0 (正式版。`develop` → `main` で昇格する、docs/09 §9.2 手順 4)
- 判定した人 / 日付: 下書き: Claude / 2026-10-27。**最終の判定は PO** (この PR のマージをもって確認とする)
- 判定コミットの確認 (`git merge-base --is-ancestor develop <SHA>` と `git merge-base --is-ancestor main <SHA>` が真): [x] (2026-10-27。判定コミットは PR #166 (版上げ) のマージコミットで、その時点の `develop` の先頭。`main` は初期コミットのまま `develop` の祖先)
- この後に入れてよい変更は `docs/compat/0.1.0/`・`docs/perf/0.1.0/`・`tests/ac-checklist.toml` だけ。ほかを入れたらゲートをやり直す。

判定は 合格 / 不合格 / 対象外 (理由) のいずれか。基準機の型番など固有の情報は書かない (docs/11 §9.7)。

| ゲート | 条件の正本 | 判定 | 根拠 | 確認者 | 日付 |
|---|---|---|---|---|---|
| 要件充足 | 09 §8.2。Must FR/NFR がすべて T/B/M/R で合格。`tests/ac-checklist.toml` に todo が無い (または理由つきの例外) | 合格 | `tests/ac-checklist.toml`: 168 件すべて done、todo 0 件。`AC-NFR-305-1` (夜間 fuzz の観測) は、`develop` の夜間 fuzz が 2026-10-22〜2026-10-26 の 5 日とも実行されクラッシュ 0 であることを確かめて done にした (下の「fuzz」の行) | Claude (下書き) | 2026-10-27 |
| 性能 | 09 §8.2。BM-1xx/2xx の Must が基準機で目標内 (p50/p99)。記録は `docs/perf/0.1.0/` | 合格 (rc.1 の記録を流用、PO 確認済み) | 製品のコードは `v0.1.0-rc.1` から変わっていない (`git diff v0.1.0-rc.1 -- crates bins xtask` は版の更新だけ)。基準機の記録は [`docs/perf/0.1.0-rc.1/summary.md`](../../perf/0.1.0-rc.1/summary.md) (全行が合格)。取り直す場合は `cargo xtask bench --env ENV-W1 --out docs/perf/0.1.0` | Claude (下書き) | 2026-10-27 |
| 互換 | 09 §8.2。Windows Terminal と conhost で手動シナリオ (開く・日本語入力 (Microsoft IME・Google 日本語入力)・検索・保存・終了)。記録は `docs/compat/0.1.0/` | 合格 (rc.1 の記録を流用、PO 確認済み) | 製品のコードは rc.1 から変わっていない。記録は [`../0.1.0-rc.1/windows-terminal.md`](../0.1.0-rc.1/windows-terminal.md)・[`../0.1.0-rc.1/windows-conhost.md`](../0.1.0-rc.1/windows-conhost.md)・[`../0.1.0-rc.1/automated-coverage.md`](../0.1.0-rc.1/automated-coverage.md)・[`../0.1.0-rc.1/color-modes.md`](../0.1.0-rc.1/color-modes.md)。v0.1.0 の zip での再確認をするかは PO が決める | Claude (下書き) | 2026-10-27 |
| 受入 | 09 §8.2。L7 の Must 項目。記録は `docs/compat/0.1.0/acceptance.md` | 合格 (rc.1 の記録を流用、PO 確認済み) | [`../0.1.0-rc.1/acceptance.md`](../0.1.0-rc.1/acceptance.md) (H1〜H3、I1〜I3) | Claude (下書き) | 2026-10-27 |
| データ保全 | 09 §8.2。kill テスト・往復コーパスなど | 合格 | 判定コミット `a9b5248` の CI (全ジョブ成功): <https://github.com/evoref/evoeditor/actions/runs/40498687585> — `test (windows)` (kill テスト・往復コーパス・表現不能 / 黙示置換の検出・緊急退避のパニック注入 / CTRL_CLOSE / CTRL_BREAK / メインのブロック)、`e2e` | Claude (下書き) | 2026-10-27 |
| セキュリティ (依存) | `cargo audit` / `deny advisories` がクリーン (例外は `deny.toml` に期限と理由つき) | 合格 | 判定コミットの CI の `deps` ジョブ (`cargo deny check`、上と同じ実行)。`deny.toml` の `[advisories] ignore` に例外なし。夜間の `cargo audit` と deny advisories は 2026-10-22〜2026-10-26 の 5 日とも成功 (下の「fuzz」の行に実行の URL) | Claude (下書き) | 2026-10-27 |
| セキュリティ (リリース前の確認) | 09 §8.2 の (a)〜。結果をここに書く | 合格 (rc.1 の記録を流用、PO 確認済み) | [`../0.1.0-rc.1/security-checks.md`](../0.1.0-rc.1/security-checks.md): (a) DLL の検索順、(b) UNC、NFR-501。製品のコードは rc.1 から変わっていない。配布物の `check-dist` は `release.yml` の `build` でも検査される | Claude (下書き) | 2026-10-27 |
| fuzz | 判定コミットで `fuzz_decode` を 1 時間 (`nightly.yml` の手動実行) してクラッシュ 0。夜間 fuzz の観測 (正式版は 5 日、v0.5.0 以降は 7 日。rc は 2 日) | 合格 | 夜間 fuzz (`develop` 対象、定期実行) は 2026-10-22・23・24・25・26 の 5 日とも実行されクラッシュ 0: <https://github.com/evoref/evoeditor/actions/runs/39855273359> (10-22)、<https://github.com/evoref/evoeditor/actions/runs/40023382460> (10-23)、<https://github.com/evoref/evoeditor/actions/runs/40128748606> (10-24)、<https://github.com/evoref/evoeditor/actions/runs/40230458824> (10-25)、<https://github.com/evoref/evoeditor/actions/runs/40411308331> (10-26)。判定コミットでの 1 時間の fuzz: 判定コミット `a9b5248` で `fuzz_decode` 3600 秒、**クラッシュ 0 件** (seed 20753): <https://github.com/evoref/evoeditor/actions/runs/40498769196> | Claude (下書き) | 2026-10-27 |
| 契約 | `eve-view` の型が `#[non_exhaustive]` で `CONTRACT_VERSION` を持つ | 合格 | `crates/eve-view/` は rc.1 から変わっていない。`#[non_exhaustive]` と `pub const CONTRACT_VERSION: u32 = 0;` (`lib.rs`) | Claude (下書き) | 2026-10-27 |
| 依存 | C を含む依存 0 (`cargo xtask check-deps`) | 合格 | 判定コミットの CI の `deps` ジョブ (`cargo xtask check-deps --docs`)。上と同じ実行: <https://github.com/evoref/evoeditor/actions/runs/40498687585> | Claude (下書き) | 2026-10-27 |
| ドキュメント | README (`dist/README.md`)・CHANGELOG・生成リファレンスが最新。端末の設定・IME とチョードの注意・既知の制約 (01 §4.6) が README にある | 合格 (生成リファレンスは対象外) | `dist/README.md` は rc.1 から変更なし。`CHANGELOG.md` に `[0.1.0]` (日付は `未定`。タグを付けるときに入れる)。生成リファレンスは v0.1 に無い (v0.2) | Claude (下書き) | 2026-10-27 |
| ライセンス・名称 | 12 §3.8 の LR-1 と LR-4。結果は 10 の OQ-01 | 合格 | rc.1 の判定のとおり (LR-1: 自己確認済み、`docs/review/lr-1-memo.md`。LR-4: 実施済み)。[`../0.1.0-rc.1/release-checklist.md`](../0.1.0-rc.1/release-checklist.md) | Claude (下書き) | 2026-10-27 |
| 未決事項の解消 | 当該版の Must に影響する OQ がすべて回答済み (08 §5) | 合格 (所見あり) | `OQ-44` は v0.1 の Must に影響せず、PO 判断 (2026-10-24) で v0.1 のリリース後に確定する | Claude (下書き) | 2026-10-27 |

## 所見・例外

- **rc.1 の記録の流用**: 製品のコードが `v0.1.0-rc.1` から変わっていない (`crates`・`bins`・`xtask` の差分は版の文字列だけ) ので、性能・互換・受入・リリース前のセキュリティ確認は rc.1 の記録を流用する。PO が流用でよいと確認した (2026-10-27)。
- **`OQ-44`**: v0.1 の Must に影響しないので v0.1.0 は止めない。v0.1 のリリース後 (遅くとも v0.2 着手前) に確定する。
- **このファイルの「Claude (下書き)」の判定**: 最終の判定は PO。この PR のマージをもって PO の確認とし、続けて `develop` → `main` の PR を作る (docs/09 §9.2 手順 4)。
