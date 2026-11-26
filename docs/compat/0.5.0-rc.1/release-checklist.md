# リリースのチェックリスト — v0.5.0-rc.1

docs/09 §8.2 の「リリースのチェックリスト」(上の表・v0.2 の表・v0.4 の表・v0.5 の表) と、08 §2.6 の v0.5 の追加ゲート (a)〜(f)、09 §9.2 手順 3。**タグの前に済む行だけ** を載せる。タグの後の結果 (配布物・タグの検証・配布物の受入) は Release のノートに書く。

次の行は `release.yml` が読む (先頭の 1 か所だけ)。

判定コミット: 4327f1831b76798739542386d2cf2818f1852ffe

- 版: 0.5.0-rc.1 (プレリリース。タグは `develop` の先頭に人が付ける、09 §9.2)
- **判定の結果: 保留 (結果待ち)**。下の「保留」の行 (fuzz の残り・夜間 fuzz の 7 日・手の確認) を待つ (下書き: Claude)。BM-102a は所見を書いて v0.6 へ繰り越す (PO の判断 2026-11-26)
- 判定した人 / 日付: 下書き: Claude / 2026-11-26。**最終の判定は PO** (この PR のマージをもって確認とする)
- 判定コミットの確認: [x] (2026-11-26。判定コミットは PR #668 (版上げ) のマージコミットで、`develop` の先頭。`git merge-base --is-ancestor main 4327f18` も成り立つ)
- 凍結: 判定から昇格・タグまで `develop` への merge を保留する (09 §9.2)。この後に入れてよい変更は `docs/compat/0.5.0-rc.1/`・`docs/perf/0.5.0-rc.1/`・`tests/ac-checklist.toml` だけ。ほかを入れたらゲートをやり直す。開いている Dependabot の PR (#639・#640・#641) も保留する。

判定は 合格 / 不合格 / 対象外 (理由) / 保留 (結果待ち) のいずれか。基準機の型番など固有の情報は書かない (docs/11 §9.7)。

## v0.5 の追加ゲート (08 §2.6)

| ゲート | 条件の正本 | 判定 | 根拠 | 確認者 | 日付 |
|---|---|---|---|---|---|
| (a) v0.4 のゲートの続き | 試験とゴールデン、clippy と `missing_docs`、仕様の検査、`CONTRACT_VERSION` は 0。v0.5 の Must の AC (FR-125・FR-405・FR-509・FR-604) に試験かベンチがある | 合格 | 判定コミットの CI (push、全ジョブ成功): <https://github.com/evoref/evoeditor/actions/runs/45658460631>。版上げの PR #668 の CI も全 22 ジョブ成功。`CONTRACT_VERSION` は 0。判定コミットの手動実行の proptest 拡張 (`PROPTEST_CASES=10000`) を含む試験 3,944 件がすべて成功: <https://github.com/evoref/evoeditor/actions/runs/45720790579> | Claude (下書き) | 2026-11-26 |
| (b) カバレッジ (NFR-702) | 夜間の coverage の段が、crate 別とモジュール別の下限を満たす。判定コミットで `nightly.yml` を `only=coverage` で手動実行 | 合格 | 判定コミットの手動実行: <https://github.com/evoref/evoeditor/actions/runs/45658673193>。eve-core 94.5% (下限 80%)・eve-text 94.7% (90%)・eve-encoding 98.3% (90%)・Undo の履歴 (`history.rs` + `transaction.rs`) 97.1% (90%)。目安: eve-config 93.8% (80%)・eve-platform 92.3% (60%)・eve-tui 93.8% (60%)。結果「coverage: 成功」 | Claude (下書き) | 2026-11-26 |
| (c) 性能 | 基準機の BM が v0.4.0 の判定の記録 (`docs/perf/0.4.0-rc.1/`) から劣化しない。BM-102a は v0.3.0 のコードと交互に測って p50 の差が +10% 以内。BM-125 p99 ≤ 50 ms、BM-103b-finder・BM-103b-search-ws p50 2 / p99 8 ms、BM-204w 0.1 % 未満 | 合格 (BM-102a は所見を書いて v0.6 へ繰り越す。PO の判断 2026-11-26) | [`../../perf/0.5.0-rc.1/summary.md`](../../perf/0.5.0-rc.1/summary.md)。Must の行はすべて目標の内。v0.5 の BM-125 (p99 17.52 / 6.94 ms)・BM-103b-finder (p99 1.32 / 1.42 ms)・BM-103b-search-ws (p99 1.04 ms)・BM-204w (0.00 %) は合格。**BM-102a は v0.3.0 と交互に測って +17.6% (約 +1.5 ms) で、+10% の条件を満たさない** (目標の p50 25 ms には十分な余裕)。所見の 1 | Claude (下書き)、PO の判断 | 2026-11-26 |
| (d) fuzz | 判定コミットで 7 つのターゲットをそれぞれ 1 時間回してクラッシュ 0。夜間 fuzz の直近 **7 日** の観測 (足したターゲットは足した日から 7 日) | 保留 (夜間 fuzz の 7 日を待つ。判定コミットでの 1 時間は合格) | 判定コミットの手動実行 (`fuzz_seconds=3600`)。1 本目 `only=fuzz:fuzz_decode+fuzz_editor+fuzz_encode`: 3 つとも 3600 秒でクラッシュ 0 件: <https://github.com/evoref/evoeditor/actions/runs/45658708517>。2 本目 `only=fuzz:fuzz_vt_input+fuzz_regex_search+fuzz_gitignore`: 3 つとも 3600 秒でクラッシュ 0 件: <https://github.com/evoref/evoeditor/actions/runs/45693528748>。3 本目 `only=fuzz:fuzz_editorconfig,proptest,audit,crash,soak`: `fuzz_editorconfig` が 3600 秒でクラッシュ 0 件: <https://github.com/evoref/evoeditor/actions/runs/45720790579>。判定コミットで 7 つのターゲットすべてが 1 時間でクラッシュ 0 件。夜間 fuzz の 7 日: 判定コミットを対象にした定時の実行を 1 日目から数える (未) | Claude (下書き) | 2026-11-26 |
| (e) MSRV | CI の `msrv` ジョブ (1.99) が判定コミットで通る | 合格 | 判定コミットの CI の `msrv` ジョブ (上と同じ実行) | Claude (下書き) | 2026-11-26 |
| (f) Could (M34) | 判定の条件にしない。入らなかったものは 08 §7 に繰り越しを記録する | 対象外 (判定の条件にしない) | M34 の 9 つは v0.5 に入った。FR-126 は v0.6 以降へ送った (08 §7、PO の決定 2026-11-25、#658) | Claude (下書き) | 2026-11-26 |

## 09 §8.2 のリリースのチェックリスト

| ゲート | 条件の正本 | 判定 | 根拠 | 確認者 | 日付 |
|---|---|---|---|---|---|
| 要件充足 | 09 §8.2 の v0.5 の表: 01 §5.5 の v0.5 の Must の AC (FR-125 AC-1〜AC-3、FR-405 AC-1〜AC-3、FR-509 AC-1〜AC-4、FR-604 AC-1〜AC-3) に試験かベンチがある。`cargo xtask trace --version v0.5` の警告で見る | 合格 | 判定コミットの CI の `check` ジョブ (上と同じ実行。`trace` の版に v0.5 を足したのは #661)。FR-125 AC-2 は BM-125 と BM-103b-finder、FR-405 AC-3 は BM-103b-search-ws で測った (上の (c)) | Claude (下書き) | 2026-11-26 |
| 性能 | 09 §8.2。BM-1xx/2xx の Must が基準機で目標内。CI の `perf (instr)` / `perf (time)` | 合格 (BM-102a は繰り越し。PO の判断 2026-11-26) | 上の (c)。`perf-diff --base v0.4.0 --mode time` (基準機): `open_1m` +22.2%・`undo_1m` +11.3%・`open_sjis_1m` +11.2% で警告 (所見の 1)、ほかは ±10% の内 | Claude (下書き)、PO の判断 | 2026-11-26 |
| 互換 (Windows) | 09 §8.2。Windows Terminal と conhost で手動シナリオ。v0.5 で足した操作 (分割とペイン・タブライン・ファインダと横断検索・ファイル監視・所有者の検査・行コメントのキー・複数カーソル・対応括弧の下線) | 保留 (手の確認、PO) | 手順は `../checklist.md` | PO | — |
| 互換 (高コントラスト) | 09 §8.2 の v0.4 の表 | 保留 (手の確認、PO) | 自動の検査は判定コミットの CI の `check-themes` (上と同じ実行) で成功 | PO、Claude (下書き) | — |
| 互換 (Linux) | 09 §8.2。リリース候補の tar.gz で WSL2 の Windows Terminal と tmux の受入 | 自動の部分は合格、手の部分は保留 | [`linux-wsl.md`](linux-wsl.md): `cargo xtask accept-linux` の 26 項目すべて ○ (判定コミットで作った tar.gz `fa456a85fdf1`、2026-11-26)。手の項目 (v0.5 で足した WSL2 の項目を含む) は PO | Claude (下書き)、PO | 2026-11-26 |
| 互換 (英語 UI) | `cargo xtask i18n` が通り、`--locale en` で日本語が残らない | 合格 | 判定コミットの CI の `i18n` ジョブ (上と同じ実行) | Claude (下書き) | 2026-11-26 |
| 受入 | 09 §8.2。L7 の Must 項目と、NFR-601 の 3 名の簡易テスト | 合格 (v0.2.0 の記録を流用。PO の判断 2026-11-26) | v0.4 と同じく [`../0.2.0-rc.1/acceptance.md`](../0.2.0-rc.1/acceptance.md)。v0.5 で足した操作は「互換」の行の手の確認で見る | Claude (下書き)、PO の判断 | 2026-11-26 |
| データ保全 | 09 §8.2。kill テスト・往復コーパスなど | 合格 | 判定コミットの CI の `test (windows)`・`test (linux)`・`e2e`・`e2e (linux)` (上と同じ実行) | Claude (下書き) | 2026-11-26 |
| データ保全 (Unix) | Unix の kill テスト (Linux CI) と、判定コミットでの LazyFS のクラッシュ一貫性 (`only=crash`) が ext4 / xfs / btrfs で合格 | 合格 | Unix の kill テストは判定コミットの CI の `test (linux)` (上と同じ実行) で合格。LazyFS のクラッシュ一貫性は判定コミットの手動実行で ext4・xfs・btrfs とも 13 件すべて合格: <https://github.com/evoref/evoeditor/actions/runs/45720790579> | Claude (下書き) | 2026-11-26 |
| 安全 (ワークスペース設定) | 09 §8.2 の v0.5 の表: NFR-502 AC-1 と FR-705 AC-2 / AC-3 の試験が通る。所有者の検査は Windows と Linux の両方の CI で走る | 合格 | 判定コミットの CI の `test (windows)`・`test (linux)` (上と同じ実行) | Claude (下書き) | 2026-11-26 |
| セキュリティ (依存) | `cargo audit` / `deny advisories` がクリーン | 合格 | 判定コミットの CI の `deps` ジョブ (上と同じ実行) は成功。夜間の `cargo audit` と deny advisories は、判定コミットの手動実行で本体 (184 crate) と `fuzz/` (103 crate) の両方で成功 (RustSec の 1,568 件で照合。警告なし): <https://github.com/evoref/evoeditor/actions/runs/45720790579> | Claude (下書き) | 2026-11-26 |
| セキュリティ (リリース前の確認) | 09 §8.2 の (a) DLL の検索順、(b) UNC パス | (a) 合格、(b) 保留 (手の確認) | (a) 判定コミットの CI の `build-release-smoke` (上と同じ実行)。(b) v0.5 で UNC パスの所有者の検査を足したので、手で確かめる (`../checklist.md`) | Claude (下書き)、PO | 2026-11-26 |
| fuzz | 09 §8.2 の v0.5 の表 | 保留 (夜間 fuzz の 7 日を待つ) | 上の (d) | Claude (下書き) | 2026-11-26 |
| 長時間 (観察) | BM-306 の 1 時間の soak の推移を所見に書く (ゲートにしない) | 観察済み | 判定コミットで 60 分: RSS 38.5 → 38.4 MiB (傾きからの増加 2.0 MiB)、入力 → Frame の p99 0.185 → 0.185 ms、「平坦」(上の「セキュリティ (依存)」と同じ実行) | Claude | 2026-11-26 |
| MSRV | CI の `msrv` ジョブが判定コミットで通る | 合格 | 上の (e) | Claude (下書き) | 2026-11-26 |
| 契約 | `CONTRACT_VERSION` は 0 のまま。`eve-view` への追加は `#[non_exhaustive]` の型への変種と、構造体への `Vec` / `Option` のフィールドの追加だけ (C2) | 合格 | `CONTRACT_VERSION` は 0 (`crates/eve-view/src/lib.rs`)。v0.4.0 からの `eve-view` の変更は M30 の仕様の PR で決めた追加 (区切り・ペインの見出し・タブライン・`ChromeMouse`) だけ。CI の `check` ジョブ (上と同じ実行) | Claude (下書き) | 2026-11-26 |
| 依存 | C を含む依存 0 (`cargo xtask check-deps`) | 合格 | 判定コミットの CI の `deps` ジョブ (上と同じ実行)。v0.4.0 から足した crate は 14 個 (`notify` と推移的依存など) | Claude (下書き) | 2026-11-26 |
| ドキュメント | README・CHANGELOG・生成リファレンスが最新。英語版 | 合格 | 01 §4.6・同梱の README・起動時の要点・CHANGELOG の書き換え (#667)。CHANGELOG の `[0.5.0-rc.1]` (ja / en、#668)。生成リファレンスは判定コミットの CI の `check` ジョブ (上と同じ実行) | Claude (下書き) | 2026-11-26 |
| ライセンス・名称 | 12 §3.8 の LR-1 と LR-4 | 合格 | 依存のライセンスは CI の `deps` ジョブ (上と同じ実行)。`THIRD-PARTY-NOTICES.md` は最新 (`accept-linux` の `dist` の検査、2026-11-26) | Claude (下書き) | 2026-11-26 |
| 未決事項の解消 | 当該版の Must に影響する OQ がすべて回答済み (08 §5) | 合格 | v0.5 の計画の OQ は PO の決定 2026-11-20 (v0.5-plan §1.3) で確定。M30〜M35 の仕様の PR で決めた点も記録済み | Claude (下書き) | 2026-11-26 |

## 所見・例外

1. **BM-102a が v0.3.0 の水準に戻らない (+17.6%、約 +1.5 ms)**: 08 §2.6 の (c) の条件 (+10% 以内) を満たさない。目標 (NFR-102 の p50 25 / p99 50 ms) には十分な余裕がある。M29 の直し (#598) の時点では v0.3.0 と同じだった (比 0.96)。その後の M30〜M33 で少しずつ増えた (1 つのコミットの段差ではない)。詳しい経緯と見立ては [`../../perf/0.5.0-rc.1/summary.md`](../../perf/0.5.0-rc.1/summary.md) の所見の 1。**案**: 08 §2.6 の (c) の「戻らなければ理由を所見に書き、08 §7 に繰り越しを記録する」に沿って、v0.6 で区間の記録を足して切り分けて直す (08 §7 の記録は昇格の後の `spec` の PR)。RC の中で直すと判定コミットが変わり、ゲートをやり直す。**PO の判断 (2026-11-26): 推奨案のとおり、この所見で通して v0.6 へ繰り越す** (#671)。
2. **計測の道具の不具合 2 件 (製品のコードは関係しない)**: (1) BM-117w は、印の付いた文書の 2 度目の変更で知らせを出し直さない仕様 (WATCH-7) に合わず、30 回を測りきれない。(2) Rust 1.100 の cargo ではベンチの実行ファイルのパスに `/` が混じり、BM-120 の `cmd /c` が起動できない。BM は Rust 1.99.1 と、手元で直した BM-117w の計測のコードで測った (`summary.md` の「測った条件」)。どちらも昇格の後に `fix` で直す (#669・#670)。
3. **一式の自動の判定の「不合格」2 行** (BM-103b-search-multiline・BM-103b-bracket-ja) は、機械が遅い状態の値で、交互に測り直すと目標の内 (`summary.md` の所見の 2)。
4. **測っていないもの (参考値)**: 横断検索を走らせている間の BM-117w と、BM-204w の参考の起床の回数。計測のコードに場面が無い。昇格の後に足す。
5. **夜間ワークフローの実行の分け方**: v0.4 と同じく、fuzz の 7 ターゲット × 1 時間は 1 回の実行の上限 (240 分) に収まらないので `only=` で 3 本に分けた。同じ concurrency の群では待ちの実行が 1 つしか残らないので、待ちが無いときにだけ次を入れた (定時の実行を押し出さない)。
6. **このファイルの「Claude (下書き)」の判定**: 最終の判定は PO。この PR のマージをもって PO の確認とする。
