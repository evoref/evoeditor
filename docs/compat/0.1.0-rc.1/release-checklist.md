# リリースのチェックリスト — v0.1.0-rc.1

雛形: [`../_template/release-checklist.md`](../_template/release-checklist.md) (docs/09 §8.2 の「リリースのチェックリスト」、§9.2 手順 3)。
**タグの前に済む行だけ** を載せる。タグの後の結果 (配布物・タグの検証・MSRV・配布経路の検証・更新手順の文書の確認・公開後の minisign の検証) は
GitHub Release のノートの「検証」の節に書き、このファイルには戻さない。

次の行は `release.yml` が読む (先頭の 1 か所だけ)。

判定コミット: 12b8988f2798463a73f7d317725abebd452b1d18

- 版: 0.1.0-rc.1 (プレリリース。昇格しない、docs/09 §9.2 手順 1a)
- 判定した人 / 日付: 下書き: Claude / 2026-10-24。**最終の判定は PO** (この PR のマージをもって確認とする)
- 判定コミットの確認 (`git merge-base --is-ancestor develop <SHA>` と `git merge-base --is-ancestor main <SHA>` が真): [x] (2026-10-24、`main` の先頭 `d9378fa` は `develop` の祖先。判定コミットは PR #159 のマージコミットで、その時点の `develop` の先頭)
- この後に入れてよい変更は `docs/compat/0.1.0-rc.1/`・`docs/perf/0.1.0-rc.1/`・`tests/ac-checklist.toml` だけ。ほかを入れたらゲートをやり直す。

判定は 合格 / 不合格 / 対象外 (理由) のいずれか。基準機の型番など固有の情報は書かない (docs/11 §9.7)。

| ゲート | 条件の正本 | 判定 | 根拠 | 確認者 | 日付 |
|---|---|---|---|---|---|
| 要件充足 | 09 §8.2。Must FR/NFR がすべて T/B/M/R で合格。`tests/ac-checklist.toml` に todo が無い (または理由つきの例外) | 合格 (例外 1 件) | `tests/ac-checklist.toml`: 168 件中 done 167 件。todo は `AC-NFR-305-1` の 1 件 (夜間 fuzz の **7 日**の観測。プレリリースの判定は 2 日、PO 判断 2026-10-24。下の「fuzz」の行で判定し、正式版 v0.1.0 の前に 7 日の観測を取る) | Claude (下書き) | 2026-10-24 |
| 性能 | 09 §8.2。BM-1xx/2xx の Must が基準機で目標内 (p50/p99)。記録は `docs/perf/0.1.0-rc.1/` | 合格 | `docs/perf/0.1.0-rc.1/summary.md` (基準機 ENV-W1、記録のコミット `87aa0c9`): 全行が合格、不合格 0。起動 BM-101b p50 36.85 ms、キー入力 BM-103b p50 1.30 / p99 2.83 ms、負荷下 (保存中・全件検索中・保存前の照合中・全選択のコピー中) も p99 ≤ 3 ms、開く → 進捗 p50 1.27 ms (100 MB)、リサイズ p99 1.93 ms、アイドルのメモリ 4.01 MB・CPU 0.00 %。**記録後に変わった製品コード**: 既定キー 6 件の追加、起動時の `SetDefaultDllDirectories` と `/DEPENDENTLOADFLAG` (起動で 1 回の呼出し)。測定の経路への影響は無いと見て取り直していない (取り直すなら `cargo xtask bench`) | Claude (下書き) | 2026-10-24 |
| 互換 | 09 §8.2。Windows Terminal と conhost で手動シナリオ (開く・日本語入力 (Microsoft IME・Google 日本語入力)・検索・保存・終了)。記録は `docs/compat/0.1.0-rc.1/` | 合格 | [`windows-terminal.md`](windows-terminal.md)・[`windows-conhost.md`](windows-conhost.md) (PO の確認。手動の項目はすべて OK)、A・H の自動テストでの代替は [`automated-coverage.md`](automated-coverage.md)、色の段階は [`color-modes.md`](color-modes.md) (C5)、検索・置換の再確認 (E1〜E3、判定コミットの zip) は `windows-terminal.md` の追記。行ごと・端末ごとの内訳は聞き取っていない (記録に明記) | Claude (下書き)、PO の報告 | 2026-10-24 |
| 受入 | 09 §8.2。L7 の Must 項目。記録は `docs/compat/0.1.0-rc.1/acceptance.md` | 合格 | [`acceptance.md`](acceptance.md) (H1〜H3、I1〜I3) | Claude (下書き)、PO の報告 | 2026-10-24 |
| データ保全 | 09 §8.2。kill テスト・往復コーパスなど | 合格 | 判定コミットの CI (全ジョブ成功): <https://github.com/evoref/evoeditor/actions/runs/40095422219> — Windows の `test (windows)` (kill テスト・往復コーパス・表現不能 / 黙示置換の検出・緊急退避のパニック注入 / CTRL_CLOSE / CTRL_BREAK / メインのブロック)、`e2e`。100 MB の退避は 0.61 秒 (`acceptance.md` H3) | Claude (下書き) | 2026-10-24 |
| セキュリティ (依存) | `cargo audit` / `deny advisories` がクリーン (例外は `deny.toml` に期限と理由つき) | 合格 | 判定コミットの CI の `deps` ジョブ (`cargo deny check`、上と同じ実行)。`deny.toml` の `[advisories] ignore = []` (例外なし)。夜間の `cargo audit` と deny advisories の直近の定期実行: 2026-10-23 成功 <https://github.com/evoref/evoeditor/actions/runs/40023382460>、2026-10-22 成功 <https://github.com/evoref/evoeditor/actions/runs/39855273359> ([`ci-gates.md`](ci-gates.md)) | Claude (下書き) | 2026-10-24 |
| セキュリティ (リリース前の確認) | 09 §8.2 の (a)〜。結果をここに書く | 合格 | [`security-checks.md`](security-checks.md): (a) DLL の検索順 (`DependentLoadFlags` = 0x800 を `check-dist` が検査、起動時の `SetDefaultDllDirectories`、偽の DLL を同じフォルダとカレントに置いた起動で読み込まれない e2e)、(b) UNC (既知の制約に追記、到達できないパスで UI が止まらない e2e)。NFR-501 (ネットワーク不使用) の確認も同じ記録 | Claude (下書き) | 2026-10-24 |
| fuzz | 判定コミットで `fuzz_decode` を 1 時間 (`nightly.yml` の手動実行) してクラッシュ 0。夜間 fuzz の 7 日の観測 (rc は 2 日) | 合格 | 判定コミット `12b8988` で `fuzz_decode` 3600 秒、**クラッシュ 0 件** (seed 20750): <https://github.com/evoref/evoeditor/actions/runs/40095500159>。夜間 fuzz (`develop` 対象、定期実行) は 2026-10-22・2026-10-23 の 2 日とも毎日実行されクラッシュ 0 (<https://github.com/evoref/evoeditor/actions/runs/39855273359>、<https://github.com/evoref/evoeditor/actions/runs/40023382460>)。rc の観測は 2 日 (PO 判断)。**正式版 v0.1.0 では 7 日** | Claude (下書き) | 2026-10-24 |
| 契約 | `eve-view` の型が `#[non_exhaustive]` で `CONTRACT_VERSION` を持つ | 合格 | `crates/eve-view/src/` の `Effect`・`InputEvent`・`EditorEvent`・`Frame` 系・`Query` 系などが `#[non_exhaustive]` (`caps` `effect` `events` `frame` `input` `query` `lib`)、`pub const CONTRACT_VERSION: u32 = 0;` (`lib.rs`) | Claude (下書き) | 2026-10-24 |
| 依存 | C を含む依存 0 (`cargo xtask check-deps`) | 合格 | 判定コミットの CI の `deps` ジョブ (`cargo xtask check-deps --docs`: C を含む依存 0 個 (上限 0)、R1〜R12、docs 02 §4.1 の突合)。直接依存の記録のレビュー: `docs/review/deps-records-0.1.0-rc.1.md` | Claude (下書き) | 2026-10-24 |
| ドキュメント | README (`dist/README.md`)・CHANGELOG・生成リファレンスが最新。端末の設定・IME とチョードの注意・既知の制約 (01 §4.6) が README にある | 合格 (生成リファレンスは対象外) | `dist/README.md` の「既知の制約 (v0.1)」(UNC を追記)、IME・キーの注意、`CHANGELOG.md` の `[0.1.0-rc.1]` (今回直した不具合を追記)。生成リファレンス (`--list-commands --json` から作るキー一覧) は v0.1 に無い (隠しオプションも未実装。v0.2)。日付は `未定` で、タグを付けるときに入れる | Claude (下書き) | 2026-10-24 |
| ライセンス・名称 | 12 §3.8 の LR-1 (Apache-2.0 の特許条項と NOTICE の運用) と LR-4 (名称の商標の簡易照会)。結果は 10 の OQ-01 | 合格 | LR-1: 確認済み (自己確認、2026-10-24。権利は個人に帰属し、勤務先から Apache-2.0 での公開と GUI 版の有償販売の許可を得ている)、記録 `docs/review/lr-1-memo.md`。LR-4: 実施済み (2026-10-24。類似名なし。法的な調査ではない)。`NOTICE` は `Copyright 2026 hiroyuki.ogawa`。docs/12 §3.8 | Claude (下書き)、PO の回答 | 2026-10-24 |
| 未決事項の解消 | 当該版の Must に影響する OQ がすべて回答済み (08 §5) | 合格 (所見あり) | 当該版の Must に影響する OQ は回答済み。**`OQ-44` (1.0 の判定条件) は「提案中 (期限: v0.1 リリース前)」のまま**だが、v1.0 の判定条件の話で v0.1 の Must に影響しない。PO 判断 (2026-10-24): v0.1 のリリース後に確定する (期限の記述は残る) | Claude (下書き)、PO の判断 | 2026-10-24 |

## 所見・例外

- **`AC-NFR-305-1` (todo)**: 夜間 fuzz の観測期間。プレリリースは 2 日 (満たす)。正式版 v0.1.0 の前に、`develop` の夜間 fuzz が直近 7 日毎日実行されクラッシュ 0 であることを確かめ、その時点で done にする。
- **`OQ-44`**: 上記のとおり、v0.1 の Must に影響しないので v0.1.0-rc.1 は止めない。v0.1 のリリース後 (遅くとも v0.2 着手前) に確定する。
- **性能の記録のコミット (`87aa0c9`) と判定コミットの差**: 製品コードの変更は既定キー 6 件・起動時の DLL 検索の制限だけ。取り直しはしていない。
- **記録の粒度**: 互換・受入は PO の口頭の報告に基づく (行ごとの ○ / △ / × と端末ごとの内訳は聞き取っていない)。各記録に明記してある。
- **このファイルの「Claude (下書き)」の判定**: 最終の判定は PO。この PR のマージをもって PO の確認とし、続けて PO が署名付きタグ `v0.1.0-rc.1` を `develop` の先頭に付ける (docs/09 §9.2 手順 1a。タグの検証は `release.yml` が行う)。
