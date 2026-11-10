# リリースのチェックリスト — v0.3.0

docs/09 §8.2 の「リリースのチェックリスト」と、08 §2.4 の v0.3 の追加ゲート (a)〜(f)、09 §9.2 手順 3。**タグの前に済む行だけ** を載せる。タグの後の結果 (配布物・タグの検証・配布物の受入) は Release のノートに書く。

次の行は `release.yml` が読む (先頭の 1 か所だけ)。

判定コミット: d46f528b76487e7b4ed7e1f656d3cae0a7c8b4f0

- 版: 0.3.0 (正式版。プレリリースは出さない (PO の判断 2026-11-08)。`develop` → `main` で昇格する、docs/09 §9.2 手順 4)
- 判定した人 / 日付: 下書き: Claude / 2026-11-08。**最終の判定は PO** (この PR のマージをもって確認とする)
- 判定コミットの確認 (`git merge-base --is-ancestor develop <SHA>` と `git merge-base --is-ancestor main <SHA>` が真): [x] (2026-11-08。判定コミットは PR #481 (版上げ) のマージコミットで、`main` (v0.2.0) はその祖先)
- この後に入れてよい変更は `docs/compat/0.3.0/`・`docs/perf/0.3.0/`・`tests/ac-checklist.toml` だけ。ほかを入れたらゲートをやり直す。

判定は 合格 / 不合格 / 対象外 (理由) / 保留 (結果待ち) のいずれか。基準機の型番など固有の情報は書かない (docs/11 §9.7)。

## v0.3 の追加ゲート (08 §2.4)

| ゲート | 条件の正本 | 判定 | 根拠 | 確認者 | 日付 |
|---|---|---|---|---|---|
| (a) 試験とゴールデン | 全試験・e2e・シナリオが緑で、画面のスナップショットと外から見える出力のゴールデン (M13) に意図しない差分がない | 合格 | 判定コミットの CI (全ジョブ成功): <https://github.com/evoref/evoeditor/actions/runs/42392746150>。v0.3 で更新した既存のゴールデンは、どれも PR で PO が認めたもの (#450・#469・#470・#479 ほか) | Claude (下書き) | 2026-11-08 |
| (b) 性能 (劣化しない) | 基準機の BM 全行が v0.2.0 の記録から劣化しない (効率化した項目は改善を記録する) | 合格 | [`../../perf/0.3.0/summary.md`](../../perf/0.3.0/summary.md) の所見。表の BM-104・BM-106 は v0.2.0 より遅く出たが、v0.2.0 の判定コミットと同じ時に交互に測ると差が無い (計測中の機械の一時的な状態)。改善: BM-103b p50 1.57 → 0.93 ms、BM-103b-search 2.44 → 1.16 ms (v0.2.0 の例外は解消)、BM-102b 49.10 → 26.72 ms ほか | Claude (下書き) | 2026-11-08 |
| (c) clippy と missing_docs | 採用した pedantic のリントを含む `cargo clippy -D warnings` と、公開 crate の `missing_docs` が通る | 合格 | 09 §8.1 の 9 個のリント (#464〜#466・#475) と、公開 crate 5 つの `#![deny(missing_docs)]` (#471〜#473)。判定コミットの CI の `clippy (windows)` / `clippy (linux)` (上と同じ実行) | Claude (下書き) | 2026-11-08 |
| (d) 仕様の検査 | `xtask check-docs` と `trace --check` が通る | 合格 | 判定コミットの CI の `check` ジョブ (`cargo xtask check`。check-docs・spec-index・trace・gen-reference を含む。上と同じ実行) | Claude (下書き) | 2026-11-08 |
| (e) 夜間 fuzz 5 日 | 夜間 fuzz 5 日の観測を免除しない (09 §8.2 の fuzz の行) | 合格 (PO の判断 2026-11-10: 観測は 3 日で可とする) | 2026-11-08 (UTC 2026-11-07 18:00) の定時実行から 5 日分を観測する。1 日目: 成功 (<https://github.com/evoref/evoeditor/actions/runs/42415895534>)。2 日目: 成功 (<https://github.com/evoref/evoeditor/actions/runs/42526351085>)。3 日目: 成功 (<https://github.com/evoref/evoeditor/actions/runs/42733849953>)。4 日目・5 日目を待たずに判定する (PO の判断 2026-11-10。下の所見) | Claude (下書き)、PO の判断 | 2026-11-10 |
| (f) 契約の版 | `CONTRACT_VERSION` は 0 のまま (C3 まで 0 に固定する、02 §8.4) | 合格 | `crates/eve-view/src/lib.rs` の `pub const CONTRACT_VERSION: u32 = 0;`。v0.3 の `eve-view` の変更は、`contract-change` のラベルの PR で PO が認めたもの: #444 (`DocumentInfo.read_only` の型の変更。破壊的で、PO の合意を得た)・#446 (キーの表記の関数の追加)・#465 (試験の 1 行)・#471 (ドキュメント)・#478 (`StatusKind::ExternalChange` の追加)。版は C3 まで 0 に固定する決まりなので変えない | Claude (下書き) | 2026-11-08 |

## 09 §8.2 のリリースのチェックリスト

| ゲート | 条件の正本 | 判定 | 根拠 | 確認者 | 日付 |
|---|---|---|---|---|---|
| 要件充足 | 09 §8.2: 01 §5.5 の Must の AC が `cargo xtask trace --check` で揃い、`docs/traceability.md` に未対応が無い。v0.1 の Must は `tests/ac-checklist.toml` | 合格 | 判定コミットの CI の `check` ジョブの `trace --check` (警告 0 件)。v0.3 は要件を足さない版 (01 §4.3、ADR-0045) | Claude (下書き) | 2026-11-08 |
| 性能 | 09 §8.2。BM-1xx/2xx の Must が基準機で目標内 (p50/p99)。CI の `perf (instr)` / `perf (time)` に劣化の警告が無い。Linux の BM は報告のみ | 合格 | [`../../perf/0.3.0/summary.md`](../../perf/0.3.0/summary.md): 判定の付く行はすべて合格 (不合格 0)。上の (b) | Claude (下書き) | 2026-11-08 |
| 互換 (Windows) | 09 §8.2。Windows Terminal と conhost で手動シナリオ (開く・日本語入力 (Microsoft IME・Google 日本語入力)・検索・保存・終了) | 合格 | 判定コミットのリリースビルド (`evoeditor 0.3.0`) で、Windows Terminal と conhost の両方で 11 項目すべて OK (PO の報告 2026-11-09)。項目: 引数なしの起動 (`無題-1`)・日本語入力 (Microsoft IME・Google 日本語入力。変換・確定・Esc の取り消し・候補窓の位置)・検索 (Ctrl+F・Enter・Shift+Enter)・日本語のファイルを開いて保存 (Ctrl+S)・終了 (Ctrl+Q)・未保存の終了の確認と取り消し・タイトルの `●`・Ctrl+O の入力欄 (Ctrl+A・Ctrl+←/→)・パレットの「保存」の先頭が「ファイル: 保存」・Ctrl+N (`無題-2`)・「失敗として終了」の終了コード 4・「設定の診断を表示」 | PO | 2026-11-09 |
| 互換 (Linux) | 09 §8.2。リリース候補の tar.gz で WSL2 の Windows Terminal と tmux の受入 | 合格 | [`linux-wsl.md`](linux-wsl.md): `cargo xtask accept-linux` の 26 項目すべて ○ (判定コミットで作った tar.gz)。手の項目は v0.2.0 の記録を流用 (PO の判断 2026-11-08) | Claude (下書き) | 2026-11-08 |
| 互換 (英語 UI) | `cargo xtask i18n` が通り、`--locale en` で日本語が残らない | 合格 | 判定コミットの CI の `i18n` ジョブ (上と同じ実行) | Claude (下書き) | 2026-11-08 |
| 受入 | 09 §8.2。L7 の Must 項目と、NFR-601 の 3 名の簡易テスト | 合格 (v0.2.0 の記録を流用) | v0.3 は操作の流れを変えていないので、[`../0.2.0-rc.1/acceptance.md`](../0.2.0-rc.1/acceptance.md) を流用する (PO の判断 2026-11-08) | Claude (下書き) | 2026-11-08 |
| データ保全 | 09 §8.2。kill テスト・往復コーパスなど | 合格 | 判定コミットの CI (上と同じ実行) の `test (windows)`・`test (linux)`・`e2e`・`e2e (linux)`。v0.3 で緊急退避を 2 段に改め (#461)、強制終了の後も本文が残る kill テストを足した | Claude (下書き) | 2026-11-08 |
| データ保全 (Unix) | Unix の kill テスト (Linux CI) と、判定コミットでの LazyFS のクラッシュ一貫性 (`only=crash`) が ext4 / xfs / btrfs で合格 | 合格 | Unix の kill テストは判定コミットの CI の `test (linux)` (上と同じ実行)。LazyFS のクラッシュ一貫性は判定コミットの手動実行 (`only=audit,crash,soak`) で ext4・xfs・btrfs とも 13 件すべて合格: <https://github.com/evoref/evoeditor/actions/runs/42416861655> | Claude (下書き) | 2026-11-08 |
| セキュリティ (依存) | `cargo audit` / `deny advisories` がクリーン | 合格 | 判定コミットの CI の `deps` ジョブ (上と同じ実行)。夜間の `cargo audit` と deny advisories: 本体 (170 crate) と `fuzz/` (91 crate) の両方でクリーン (RustSec の 1,422 件で照合): <https://github.com/evoref/evoeditor/actions/runs/42416861655> | Claude (下書き) | 2026-11-08 |
| セキュリティ (リリース前の確認) | 09 §8.2 の (a) DLL の検索順、(b) UNC パス | 合格 | (a) 判定コミットの CI の `build-release-smoke` (`check-dist` が `DependentLoadFlags` を検査)。(b) v0.2.0 から UNC の扱いの変更なし | Claude (下書き) | 2026-11-08 |
| fuzz | 09 §8.2 (v0.2 の表): 判定コミットで `fuzz_decode` と `fuzz_editor` をそれぞれ 1 時間回してクラッシュ 0 | 合格 (PO の判断 2026-11-09: 走り切ったことをクラッシュ 0 の根拠とする) | 判定コミットの手動実行 (`fuzz_seconds=3600`): <https://github.com/evoref/evoeditor/actions/runs/42397140980>。`fuzz_decode` (17:39〜18:39 UTC)・`fuzz_editor` (18:39〜19:39)・`fuzz_encode` (19:39〜20:39) は 3600 秒を走り切った (libFuzzer は最初のクラッシュで止まるので、走り切ったことがクラッシュ 0 を示す)。暗号化したクラッシュ入力 (`*.age`) も 0 件。4 つ目の `fuzz_vt_input` の途中でジョブの上限 (240 分) に当たり、実行は「キャンセル」で要約は残らなかった (下の所見) | Claude (下書き)、PO の判断 | 2026-11-09 |
| 長時間 (観察) | BM-306 の 1 時間の soak の推移を所見に書く (ゲートにしない) | 観察済み | 判定コミットで 60 分: RSS 36.4 → 33.4 MiB (増加 -0.1 MiB)、入力 → Frame の p99 0.195 → 0.197 ms、「平坦」(<https://github.com/evoref/evoeditor/actions/runs/42416861655>) | Claude | 2026-11-08 |
| MSRV | CI の `msrv` ジョブが判定コミットで通る | 合格 | 判定コミットの CI の `msrv` ジョブ (上と同じ実行) | Claude (下書き) | 2026-11-08 |
| 契約 | `eve-view` の型が `#[non_exhaustive]` で `CONTRACT_VERSION` を持つ | 合格 | 上の (f) | Claude (下書き) | 2026-11-08 |
| 依存 | C を含む依存 0 (`cargo xtask check-deps`) | 合格 | 判定コミットの CI の `deps` ジョブ (上と同じ実行) | Claude (下書き) | 2026-11-08 |
| ドキュメント | README・CHANGELOG・生成リファレンスが最新。英語版 (`dist/README.en.md`・`CHANGELOG.en.md`) | 合格 | CHANGELOG の `[0.3.0]` (#481)。同梱の README の「できること」「既知の制約」は v0.2.0 に合わせ直した (#478)。生成リファレンス `docs/reference/` は `gen-reference --check` で最新 (#468) | Claude (下書き) | 2026-11-08 |
| ライセンス・名称 | 12 §3.8 の LR-1 と LR-4。結果は 10 の OQ-01 | 合格 | v0.1.0-rc.1 の確認から変更なし。v0.3 で足した依存は無い (`deps` ジョブ) | Claude (下書き) | 2026-11-08 |
| 未決事項の解消 | 当該版の Must に影響する OQ がすべて回答済み (08 §5) | 合格 | v0.3 は要件を足さない版で、範囲の判断は ADR-0045 と M11〜M19 の各 PR で PO が決めた | Claude (下書き) | 2026-11-08 |

## 所見・例外

- **性能 (BM-104・BM-106)**: 上の (b) と `summary.md` の所見のとおり。劣化ではない。
- **PRF-20 (ratatui の依存の計測)**: `summary.md` の所見に BM-B01 (ビルド時間) と依存の数を書いた。exe の中の ratatui の大きさ (`cargo bloat`) は今回は測らない (PO の判断 2026-11-08)。外すかの判断は v0.6 の前。
- **互換・受入の記録の流用**: Linux の手の項目と NFR-601 の簡易テストは v0.2.0 の記録を流用した (PO の判断 2026-11-08)。v0.3 は振る舞いを変えない整理の版で、自動の受入がすべて ○ であることを根拠にする。
- **夜間 fuzz の観測 (ゲート (e))**: 5 日のうち、判定コミットを対象にした定時の実行 3 日分 (すべて成功、クラッシュ 0) で判定した。残りの 2 日は待たない (PO の判断 2026-11-10)。判定コミットでは別に `fuzz_decode`・`fuzz_editor`・`fuzz_encode` を各 1 時間回している (上の fuzz の行)。
- **夜間ワークフローの時間の上限 (リリースの後に直す)**: `fuzz_seconds=3600` の手動実行は、fuzz の 4 ターゲット × 1 時間がジョブの `timeout-minutes: 240` に収まらない。ターゲットを選ぶ入力が無いので、fuzz と crash・audit・soak を分けて実行した。
- **見つけた小さな不具合 (リリースの後に直す)**: `cargo xtask dist` が `CARGO_TARGET_DIR` を指定すると実行ファイルを見つけられない ([`linux-wsl.md`](linux-wsl.md) の所見)。
- **このファイルの「Claude (下書き)」の判定**: 最終の判定は PO。この PR のマージをもって PO の確認とする。
