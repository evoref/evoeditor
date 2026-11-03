# リリースのチェックリスト — v0.2.0-rc.1

docs/09 §8.2 の「リリースのチェックリスト」(v0.2 の判定の表を含む)、§9.2 手順 3。**タグの前に済む行だけ** を載せる。タグの後の結果 (配布物・タグの検証・配布経路の検証・公開後の minisign の検証) は GitHub Release のノートの「検証」の節に書いた。

**この rc の順序の注記**: v0.2.0-rc.1 は、版上げ (#318) の直後に PO がタグを付けて公開し、ゲートの判定はその後に同じコミット (`ced9f04`) で行った。判定コミットとタグのコミットは同じで、判定のための製品コードの変更は無い。

次の行は `release.yml` が読む (先頭の 1 か所だけ)。

判定コミット: ced9f047bab47747b68b0505d51e5c24e2f59ce3

- 版: 0.2.0-rc.1 (プレリリース。昇格しない、docs/09 §9.2 手順 1a)
- 判定した人 / 日付: 下書き: Claude / 2026-11-03。**最終の判定は PO** (この PR のマージをもって確認とする)
- 判定コミットの確認 (`git merge-base --is-ancestor develop <SHA>` と `git merge-base --is-ancestor main <SHA>` が真): [x] (2026-11-03、判定コミットは `develop` の先頭で、`main` の先頭はその祖先)
- この後に入れてよい変更は `docs/compat/0.2.0-rc.1/`・`docs/perf/0.2.0-rc.1/`・`tests/ac-checklist.toml` だけ。ほかを入れたらゲートをやり直す。

判定は 合格 / 不合格 / 対象外 (理由) のいずれか。基準機の型番など固有の情報は書かない (docs/11 §9.7)。

| ゲート | 条件の正本 | 判定 | 根拠 | 確認者 | 日付 |
|---|---|---|---|---|---|
| 要件充足 | 09 §8.2 (v0.2): 01 §5.5 の v0.2 の Must の AC が `cargo xtask trace --check` で揃い、`docs/traceability.md` に未対応が無い。v0.1 の Must は `tests/ac-checklist.toml` | 合格 | `cargo xtask trace --check`: OK (警告 0 件、判定コミット)。`docs/traceability.md` の v0.2 の Must の AC 26 件すべてに試験がある (「なし」は Should の FR-307・FR-403 だけ)。`tests/ac-checklist.toml` (v0.1 の Must): done 169 件、todo 0 件 | Claude (下書き) | 2026-11-03 |
| 性能 | 09 §8.2。BM-1xx/2xx の Must が基準機で目標内 (p50/p99)。v0.2: CI の `perf (instr)` / `perf (time)` に劣化の警告が無い。Linux の BM は報告のみ | **不合格 (例外 1 件)** | [`docs/perf/0.2.0-rc.1/summary.md`](../../perf/0.2.0-rc.1/summary.md) (基準機 ENV-W1、判定コミット)。**BM-101b** (spawn → 最初の Frame) の p99 218.57 ms が目標 80 ms を超えた (p50 23.69 ms は目標内)。原因はログオフ検知の非表示のトップレベルウィンドウで、前の回の終了の直後に起動したときだけ約 0.2 秒遅れる (切り分けは summary.md の所見)。**PO 判断 (2026-11-03): rc.1 は例外として続け、v0.2.0 の前に直して測り直す**。BM-103b-clip100 は測り直しで合格 (所見)。ほかの判定の行はすべて合格。CI の perf: 版上げの PR #318 で `perf (instr)` / `perf (time)` 成功 (<https://github.com/evoref/evoeditor/actions/runs/41567479053>)。BM-104p はベンチの手順だけを直して測った (所見) | Claude (下書き)、PO の判断 | 2026-11-03 |
| 互換 (Windows) | 09 §8.2。Windows Terminal と conhost で手動シナリオ (開く・日本語入力 (Microsoft IME・Google 日本語入力)・検索・保存・終了) | 合格 | [`windows-terminal.md`](windows-terminal.md)・[`windows-conhost.md`](windows-conhost.md): 公開した zip (`4e1975fa461b`) で PO が確認し、手動の項目はすべて OK | Claude (下書き)、PO の報告 | 2026-11-03 |
| 互換 (Linux、v0.2 から) | 09 §8.2 の v0.2 の表。リリース候補の tar.gz で WSL2 の Windows Terminal と tmux の受入 | 合格 | [`linux-wsl.md`](linux-wsl.md): 公開した tar.gz (`0b9961fe586b`) で `accept-linux --archive` が 26 項目すべて ○、手の項目は PO の確認ですべて OK。C2 の △ は既知の制約 (README と 05 §9.4) | Claude (下書き)、PO の報告 | 2026-11-03 |
| 互換 (英語 UI、v0.2 から) | `cargo xtask i18n` が通り、`--locale en` で日本語が残らない | 合格 | 判定コミットの CI の `i18n` ジョブ (<https://github.com/evoref/evoeditor/actions/runs/41569572044>)。`--locale en` の一巡は [`acceptance.md`](acceptance.md) の「英語 UI」 | Claude | 2026-11-03 |
| 受入 | 09 §8.2。L7 の Must 項目と、v0.2 の NFR-601 の 3 名の簡易テスト | 合格 | [`acceptance.md`](acceptance.md): H1〜H3 (公開した zip、PO の確認) と I1〜I3 は ○、NFR-601 は 3 / 3 成功 (`docs/review/usability-v0.2.md` §3)、英語 UI は合格 | Claude (下書き)、PO の報告 | 2026-11-03 |
| データ保全 | 09 §8.2。kill テスト・往復コーパスなど | 合格 | 判定コミットの CI (全ジョブ成功): <https://github.com/evoref/evoeditor/actions/runs/41569572044> — `test (windows)`・`e2e` (Windows の kill テスト・往復コーパス・緊急退避) | Claude (下書き) | 2026-11-03 |
| データ保全 (Unix、v0.2 から) | Unix の kill テスト (Linux CI) と、判定コミットでの LazyFS のクラッシュ一貫性 (`only=crash`) が ext4 / xfs / btrfs で合格。スワップと復旧のシナリオと kill テストが通る | 合格 | Unix の kill テスト・スワップと復旧: 上の CI の `test (linux)`・`e2e (linux)`。LazyFS: 判定コミットの `nightly.yml` の手動実行で ext4・xfs・btrfs の各 13 件すべて合格 (<https://github.com/evoref/evoeditor/actions/runs/41582766378>) | Claude (下書き) | 2026-11-03 |
| セキュリティ (依存) | `cargo audit` / `deny advisories` がクリーン (例外は `deny.toml` に期限と理由つき) | 合格 | 判定コミットの CI の `deps` ジョブ。夜間の `cargo audit` と deny advisories: 判定コミットの手動実行で成功 (<https://github.com/evoref/evoeditor/actions/runs/41582766378>)、定期実行 2026-11-03 成功 (<https://github.com/evoref/evoeditor/actions/runs/41553311229>) | Claude (下書き) | 2026-11-03 |
| セキュリティ (リリース前の確認) | 09 §8.2 の (a) DLL の検索順、(b) UNC パス | 合格 | (a) 判定コミットの CI の `build-release-smoke` (`check-dist` が `DependentLoadFlags` = 0x800 を検査) と `e2e` (偽の DLL を同じフォルダとカレントに置いた起動で読み込まれない)。起動時の `SetDefaultDllDirectories` は `bins/evoeditor/src/main.rs` の `main` の先頭 (v0.1.0-rc.1 から変更なし)。(b) 到達できない UNC で UI が止まらない e2e (同じ CI) と、README の既知の制約。v0.1.0-rc.1 の [`security-checks.md`](../0.1.0-rc.1/security-checks.md) の手順と同じ | Claude (下書き) | 2026-11-03 |
| fuzz | 09 §8.2 (v0.2): 判定コミットで `fuzz_decode` と `fuzz_editor` をそれぞれ 1 時間回してクラッシュ 0。夜間 fuzz の観測 (rc は 2 日) | 合格 | 判定コミットで各 3600 秒、**クラッシュ 0 件** (seed 20760): <https://github.com/evoref/evoeditor/actions/runs/41582763034>。夜間 fuzz (定期実行) は 2026-11-01 (<https://github.com/evoref/evoeditor/actions/runs/41353345758>。fuzz はクラッシュ 0。ジョブの失敗は crash の手順で、#309 で直した) と 2026-11-03 (<https://github.com/evoref/evoeditor/actions/runs/41553311229>) の 2 日でクラッシュ 0 | Claude (下書き) | 2026-11-03 |
| 長時間 (観察、v0.2 から) | BM-306 の 1 時間の soak の推移を所見に書く (ゲートにしない) | 観察済み | 判定コミットで 60 分: RSS 24.1 → 24.5 MiB (増加 0.4 MiB)、入力 → Frame の p99 0.519 → 0.527 ms、「平坦」(<https://github.com/evoref/evoeditor/actions/runs/41582766378>) | Claude | 2026-11-03 |
| MSRV (v0.2 から) | CI の `msrv` ジョブが判定コミットで通る | 合格 | 判定コミットの CI の `msrv` ジョブ (<https://github.com/evoref/evoeditor/actions/runs/41569572044>) | Claude | 2026-11-03 |
| 契約 | `eve-view` の型が `#[non_exhaustive]` で `CONTRACT_VERSION` を持つ | 合格 | `crates/eve-view/src/lib.rs` の `pub const CONTRACT_VERSION: u32 = 0;`、公開の型は `#[non_exhaustive]` (v0.1.0-rc.1 の確認と同じ形) | Claude (下書き) | 2026-11-03 |
| 依存 | C を含む依存 0 (`cargo xtask check-deps`) | 合格 | 判定コミットの CI の `deps` ジョブ (`cargo xtask check-deps --docs`) | Claude (下書き) | 2026-11-03 |
| ドキュメント | README・CHANGELOG・生成リファレンスが最新。v0.2: 英語版 (`dist/README.en.md`・`CHANGELOG.en.md`) と README の Linux の導入・更新・既知の制約 | 合格 (生成リファレンスは対象外) | `CHANGELOG.md` / `CHANGELOG.en.md` の `[0.2.0-rc.1]` (#318)、`dist/README.md` / `README.en.md` の Linux の導入・更新・既知の制約 (WSL の Windows Terminal の `Ctrl+Shift`)。**生成リファレンス (`docs/reference/`、`cargo xtask gen-reference`) は未実装** (v0.1.0-rc.1 の記録で「v0.2」とした項目)。キーとコマンドの一覧は画面のヘルプ (`Ctrl+K H`) にある。**PO 判断 (2026-11-03): 対象外として後の版へ送る** (実装の版は判定の後に仕様書を直して決める) | Claude (下書き) | 2026-11-03 |
| ライセンス・名称 | 12 §3.8 の LR-1 と LR-4。結果は 10 の OQ-01 | 合格 | v0.1.0-rc.1 の確認から変更なし (LR-1: `docs/review/lr-1-memo.md`、LR-4: 2026-10-24 実施) | Claude (下書き) | 2026-11-03 |
| 未決事項の解消 | 当該版の Must に影響する OQ がすべて回答済み (08 §5) | 合格 (所見あり) | 期限が「v0.2 着手前」などの提案中の OQ のうち、裁量の区分 (OQ-45・48・57・58・77・109・124・138・148・175 ほか) は 10 §0.5 により推奨で確定扱い。**PO の区分の OQ-112 (Linux の配布物) が提案中**: 推奨 (musl 静的版だけを配布する) のとおりに出荷しているが、判断の材料の「musl 版と glibc 版の BM-104 / BM-202 の比較」の記録が無い。**PO 判断 (2026-11-03): 計測してから確定する**。BM-104 / BM-202 のベンチ (`crates/eve-platform/benches/bm_104_202.rs`) は Windows 専用なので、判定の後に Linux へ移して musl 版と glibc 版を測り、10 の OQ-112 を直す。rc.1 は推奨のとおり musl 静的版だけを配布した | Claude (下書き) | 2026-11-03 |

## 所見・例外

- **性能 (BM-101b)**: 上の行と [`summary.md`](../../perf/0.2.0-rc.1/summary.md) の所見のとおり。v0.2.0 の前に `fix/` の PR で直し、基準機で測り直す。
- **ベンチの手順の修正**: BM-104p は v0.2 の複数の文書に合わせてベンチだけを直して測った。ベンチの修正は判定の後に `develop` へ入れる。
- **凍結中に止めている PR**: Dependabot の #319・#320・#321。#321 (`unicode-width` 0.2.3) は半角カナの濁点の幅が二重に数えられる (試験 `半角カナの濁点は基底に足して数える` が失敗) ので、判定の後に `eve-text` の幅の計算と一緒に直す。
- **OQ-112**: 上の「未決事項の解消」の行のとおり。計測の結果で閾値を超えたら、v0.2.0 に glibc 版を足す (05 §17.3 の判断規則)。
- **生成リファレンス**: 対象外として後の版へ送る (PO 判断)。08 などの記述を判定の後に直す。
- **記録の粒度**: 互換・受入の手の項目は PO の報告 (「すべて OK」) による。行ごとの細目は聞き取っていない。各記録に明記してある。
- **このファイルの「Claude (下書き)」の判定**: 最終の判定は PO。この PR のマージをもって PO の確認とする。
