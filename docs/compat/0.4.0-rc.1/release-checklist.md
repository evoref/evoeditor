# リリースのチェックリスト — v0.4.0-rc.1

docs/09 §8.2 の「リリースのチェックリスト」(上の表・v0.2 の表・v0.4 の表) と、08 §2.5 の v0.4 の追加ゲート (a)〜(e)、09 §9.2 手順 3。**タグの前に済む行だけ** を載せる。タグの後の結果 (配布物・タグの検証・配布物の受入) は Release のノートに書く。

次の行は `release.yml` が読む (先頭の 1 か所だけ)。

判定コミット: 90b2c81fbf292e635cc03f1b393c4d4d9a45a7ae

- 版: 0.4.0-rc.1 (プレリリース。タグは `develop` の先頭に人が付ける、09 §9.2)
- **判定の結果: 不合格 → 0.4.0-rc.2 で判定をやり直す (PO の判断 2026-11-16)**。rc.1 は既知の不具合 (下の所見) をノートに書いてプレリリースとして公開する
- 判定した人 / 日付: 下書き: Claude / 2026-11-14。**最終の判定は PO** (この PR のマージをもって確認とする)
- 判定コミットの確認: [x] (2026-11-14。判定コミットは PR #564 (版上げ) のマージコミットで、`develop` の先頭)
- 凍結: 判定から昇格・タグまで `develop` への merge を保留する (09 §9.2)。この後に入れてよい変更は `docs/compat/0.4.0-rc.1/`・`docs/perf/0.4.0-rc.1/`・`tests/ac-checklist.toml` だけ。ほかを入れたらゲートをやり直す。

判定は 合格 / 不合格 / 対象外 (理由) / 保留 (結果待ち) のいずれか。基準機の型番など固有の情報は書かない (docs/11 §9.7)。

## v0.4 の追加ゲート (08 §2.5)

| ゲート | 条件の正本 | 判定 | 根拠 | 確認者 | 日付 |
|---|---|---|---|---|---|
| (a) v0.3 のゲートの続き | 試験とゴールデン、clippy と `missing_docs`、仕様の検査、`CONTRACT_VERSION` は 0 (08 §2.4 の (a)・(c)・(d)・(f)) | 不合格 (夜間の性質テストで反例。rc.2 で直す) | 判定コミットの CI (全ジョブ成功): <https://github.com/evoref/evoeditor/actions/runs/43590596088>。v0.4 で更新した既存のゴールデンは、どれも PR で PO が認めたもの (#553・#556・#558 ほか)。`CONTRACT_VERSION` は下の「契約」の行。ただし判定コミットを対象にした定時の夜間ジョブ (2026-11-14 (UTC)) の proptest 拡張 (`PROPTEST_CASES=10000`) で `eve-core` の `つながりの局所の検査は出力の全体の復号と同値で_乾式走査と本番で同じ` が反例を出した: <https://github.com/evoref/evoeditor/actions/runs/43645525374> (下の所見) | Claude (下書き) | 2026-11-14 |
| (b) 往復コーパスのバイト一致 | 01 §5.5 の NFR-303 AC-5〜AC-7 と NFR-304 AC-4〜AC-6 の試験が通る | 合格 | 判定コミットの CI の `test (windows)`・`test (linux)` (上と同じ実行)。`cargo xtask trace --check --version v0.4` で、これらの AC に `verifies:` の試験がある (下の「要件充足」) | Claude (下書き) | 2026-11-14 |
| (c) 性能 | 基準機の BM が v0.3.0 の記録から劣化しない (差があれば理由を所見に書く)。BM-104v p50 ≤ 300 ms (Must)。BM-106 の 5 つの操作 p99 ≤ 50 ms (Should)。BM-103b-search-multiline(-1m) p50 2 / p99 8 ms (Must) | 合格 (劣化 1 件は理由を所見に書いた) | [`../../perf/0.4.0-rc.1/summary.md`](../../perf/0.4.0-rc.1/summary.md)。Must の不合格 0。`perf-diff --base v0.3.0 --mode time` の警告 0。BM-104v-100m p50 124.3 ms・BM-104v-100m-sjis 204.9 ms。BM-103b-search-multiline-1m p50 1.05 / p99 1.68 ms。BM-106 は p99 最大 15.81 ms (速い状態。下の所見)。BM-102a が v0.3.0 より +1.3 ms (下の所見) | Claude (下書き) | 2026-11-14 |
| (d) fuzz | 判定コミットで 5 つのターゲットをそれぞれ 1 時間回してクラッシュ 0。夜間 fuzz の 5 日の観測 | 1 時間の実行は合格。観測 (3 日で可、PO の判断 2026-11-15) は 1 日目の夜間ジョブが proptest で失敗したので rc.2 で数え直す | 判定コミットの手動実行 (`fuzz_seconds=3600`) で 5 つとも 3600 秒を走り切ってクラッシュ 0 件 (暗号化したクラッシュ入力も 0 件)。1 本目 `only=fuzz:fuzz_decode+fuzz_editor+fuzz_encode`: <https://github.com/evoref/evoeditor/actions/runs/43590678644>。2 本目 `only=fuzz:fuzz_regex_search+fuzz_vt_input,crash`: <https://github.com/evoref/evoeditor/actions/runs/43606134663>。夜間 fuzz の観測は、5 日を待たず、判定コミットを対象にした定時の実行の 3 日分 (2026-11-14 (UTC) から) で判定する (PO の判断 2026-11-15。v0.3.0 と同じ扱い)。1 日目: 失敗 (fuzz の 5 つはクラッシュ 0 件。失敗は proptest 拡張の反例。上の (a) と下の所見): <https://github.com/evoref/evoeditor/actions/runs/43645525374>。観測は rc.2 の判定コミットで数え直す | Claude (下書き) | 2026-11-14 |
| (e) Could の 3 項目 | FR-110・FR-119・幅プローブは v0.5 以降に送った (08 §7) | 対象外 (v0.5 以降へ。PO の決定 2026-11-14) | 08 §7 の繰り越しの表 (#555) | Claude (下書き) | 2026-11-14 |

## 09 §8.2 のリリースのチェックリスト

| ゲート | 条件の正本 | 判定 | 根拠 | 確認者 | 日付 |
|---|---|---|---|---|---|
| 要件充足 | 09 §8.2 の v0.4 の表: 01 §5.5 の v0.4 の Must の AC (NFR-303 AC-5〜AC-7、NFR-304 AC-4〜AC-6、FR-122 AC-2〜AC-4) に確かめる試験がある。`cargo xtask trace --version v0.4` の警告で見る | 合格 | 判定コミットで `cargo xtask trace --check --version v0.4` が警告 0 件 (手元、2026-11-14)。判定コミットの CI の `check` ジョブ (上と同じ実行)。行列は [`../../traceability.md`](../../traceability.md) の v0.4 の節 (#563)。FR-122 AC-2 はベンチ BM-104v と手順書の G4 | Claude (下書き) | 2026-11-14 |
| 性能 | 09 §8.2。BM-1xx/2xx の Must が基準機で目標内。CI の `perf (instr)` / `perf (time)` に劣化の警告が無いか、あれば理由。Linux の BM は報告のみ | 合格 | 上の (c) | Claude (下書き) | 2026-11-14 |
| 互換 (Windows) | 09 §8.2。Windows Terminal と conhost で手動シナリオ。v0.4 で足した操作 (不正バイトを含む文書の編集と保存、Shift_JIS の 100 MB の先行表示 (G4)、複数行の正規表現の検索、`cmd \| evoeditor -` (N))。ADR-0005 / 0016 / 0023 から持ち越した手動の確認 (M) | 合格 | release.yml が作った draft の Release の Windows の zip (SHA-256 の先頭 12 桁 `9582d12a60da`) で、Windows Terminal と conhost (N は VS Code の統合端末と mintty も) で手順書の G4・M・N を確かめ、すべて期待どおり (PO の報告 2026-11-16)。記録は [`stdin.md`](stdin.md) (N) と [`acceptance.md`](acceptance.md) (M1〜M10)。N の Windows PowerShell 5.1 のパイプ・WSL からの interop・`ssh -t` 越しは確かめていない | PO | 2026-11-16 |
| 互換 (高コントラスト) | 09 §8.2 の v0.4 の表: `high-contrast-dark` / `high-contrast-light` を Windows Terminal と conhost で目で確かめる | 合格 | 手順書の P のとおり、Windows Terminal と conhost で両方のテーマを確かめ、P1〜P9 がすべて期待どおり (PO の報告 2026-11-16)。記録は [`high-contrast.md`](high-contrast.md)。自動の検査 (07 §8.7 の (h1)〜(h10)) は判定コミットの CI の `check-themes` (上と同じ実行) で合格 | PO | |
| 互換 (Linux) | 09 §8.2。リリース候補の tar.gz で WSL2 の Windows Terminal と tmux の受入 | 合格 | [`linux-wsl.md`](linux-wsl.md): `cargo xtask accept-linux` の 26 項目すべて ○ (判定コミットで作った tar.gz、2026-11-14)。手の項目 (日本語入力の候補窓・Windows Terminal が送るキー・右クリックの貼り付け・色の見た目) は、release.yml が作った tar.gz (`c4031c3b893a`) で PO が確かめ、すべて期待どおり (2026-11-16) | Claude (下書き)、PO | 2026-11-16 |
| 互換 (英語 UI) | `cargo xtask i18n` が通り、`--locale en` で日本語が残らない | 合格 | 判定コミットの CI の `i18n` ジョブ (上と同じ実行)。v0.4 で足した文言 (標準入力・言語ファイルの診断・高コントラストテーマ) を含む | Claude (下書き) | 2026-11-14 |
| 受入 | 09 §8.2。L7 の Must 項目と、NFR-601 の 3 名の簡易テスト | 合格 (v0.2.0 の記録を流用。PO の判断 2026-11-15) | 操作の基本の流れは変わらないので、[`../0.2.0-rc.1/acceptance.md`](../0.2.0-rc.1/acceptance.md) を流用する。v0.4 で足した機能 (標準入力・先行表示・不正バイトの編集・高コントラストテーマ) は、上の「互換 (Windows)」「互換 (高コントラスト)」の手順書の確認 (G4・N・P) で見る | Claude (下書き)、PO の判断 | 2026-11-15 |
| データ保全 | 09 §8.2。kill テスト・往復コーパスなど | 合格 | 判定コミットの CI (上と同じ実行) の `test (windows)`・`test (linux)`・`e2e`・`e2e (linux)`。v0.4 で足した往復コーパスは上の (b) | Claude (下書き) | 2026-11-14 |
| データ保全 (Unix) | Unix の kill テスト (Linux CI) と、判定コミットでの LazyFS のクラッシュ一貫性 (`only=crash`) が ext4 / xfs / btrfs で合格 | 合格 | Unix の kill テストは判定コミットの CI の `test (linux)` (上と同じ実行) で合格。LazyFS のクラッシュ一貫性は判定コミットの手動実行で ext4・xfs・btrfs とも 13 件すべて合格: <https://github.com/evoref/evoeditor/actions/runs/43606134663> | Claude (下書き) | 2026-11-14 |
| セキュリティ (依存) | `cargo audit` / `deny advisories` がクリーン | 合格 | 判定コミットの CI の `deps` ジョブ (上と同じ実行) は合格。夜間の `cargo audit` と deny advisories は、判定コミットの手動実行 (`only=proptest,audit,soak`) で本体 (170 crate) と `fuzz/` (91 crate) の両方で成功 (RustSec の 1,509 件で照合): <https://github.com/evoref/evoeditor/actions/runs/43590697670>。本体の `cargo audit` は情報の警告 (unsound) を 1 件出した (下の所見。影響なし) | Claude (下書き) | 2026-11-14 |
| セキュリティ (リリース前の確認) | 09 §8.2 の (a) DLL の検索順、(b) UNC パス | 合格 | (a) 判定コミットの CI の `build-release-smoke` (`check-dist` が `DependentLoadFlags` を検査。上と同じ実行)。(b) v0.3.0 から UNC の扱いの変更なし。v0.4 で足したファイルの読み先 (`<config_dir>/locales/`) は既存の設定の置き場所の下で、名前を `^[a-z]{2,3}\.toml$` に限る (07 §17.4) | Claude (下書き) | 2026-11-14 |
| fuzz | 09 §8.2 の v0.4 の表 | 1 時間の実行は合格。観測は rc.2 で数え直す | 上の (d) | Claude (下書き) | 2026-11-14 |
| 長時間 (観察) | BM-306 の 1 時間の soak の推移を所見に書く (ゲートにしない) | 観察済み | 判定コミットで 60 分: RSS 37.0 → 36.2 MiB (増加 -0.7 MiB)、入力 → Frame の p99 0.196 → 0.191 ms、「平坦」(上の「セキュリティ (依存)」と同じ実行。同じ実行の proptest 拡張 (`PROPTEST_CASES=10000`) も成功) | Claude | 2026-11-15 |
| MSRV | CI の `msrv` ジョブが判定コミットで通る | 合格 | 判定コミットの CI の `msrv` ジョブ (上と同じ実行) | Claude (下書き) | 2026-11-14 |
| 契約 | `CONTRACT_VERSION` は 0 のまま。`eve-view` への追加は `#[non_exhaustive]` の型への追加だけ (C2) | 合格 | `crates/eve-view/src/lib.rs` の `pub const CONTRACT_VERSION: u32 = 0;`。v0.3.0 から判定コミットまでの `eve-view` の変更は #542 (先行表示の問い合わせ。`query.rs` に 3 行の追加。`contract-change` のラベル) だけ | Claude (下書き) | 2026-11-14 |
| 依存 | C を含む依存 0 (`cargo xtask check-deps`) | 合格 | 判定コミットの CI の `deps` ジョブ (上と同じ実行) | Claude (下書き) | 2026-11-14 |
| ドキュメント | README・CHANGELOG・生成リファレンスが最新。英語版 (`dist/README.en.md`・`CHANGELOG.en.md`) | 合格 | CHANGELOG の `[0.4.0-rc.1]` (ja / en、#564)。01 §4.6 の既知の制約・同梱の README (ja / en) の「できること」「既知の制約」「更新」・起動時の要点 `info.app.release_summary` を v0.4 に合わせた (#562)。生成リファレンスは判定コミットの CI の `check` ジョブの `gen-reference --check` (上と同じ実行) | Claude (下書き) | 2026-11-14 |
| ライセンス・名称 | 12 §3.8 の LR-1 と LR-4。結果は 10 の OQ-01 | 合格 | v0.1.0-rc.1 の確認から変更なし。C を含む依存の追加は無い (`deps` ジョブ) | Claude (下書き) | 2026-11-14 |
| 未決事項の解消 | 当該版の Must に影響する OQ がすべて回答済み (08 §5) | 合格 | v0.4 の細目の OQ-217 (M23)・OQ-218 (M24)・OQ-219 (M25)・OQ-220 / OQ-221 (M26) はすべて確定 (ユーザー回答、2026-11-12〜14)。OQ-16 / 21 / 41 / 60 も確定 (10 の表) | Claude (下書き) | 2026-11-14 |

## 所見・例外

- **性能 (BM-102a の劣化)**: ファイルを 1 つ開く起動が v0.3.0 より約 1.3 ms (+25%) 遅い (目標 p50 25 ms に対して 6.5 ms)。原因は同梱の言語の表の解析 (約 1.33 ms) が、#558 (M26 の (2)) で起動の設定読込の段に移ったこと (v0.3.0 では最初の Frame の生成で遅れて行い、読込のジョブと重なって隠れていた)。置き場所は仕様どおり (02 §9.5 の手順 2、07 §17.4) だが、07 §17.1 の目安 (同梱の表のデシリアライズ 1 ms 以下。超えるならビルド時に静的な表へ切り替える) は v0.3.0 から超えていた。切り分けの計測と直す案は [`../../perf/0.4.0-rc.1/summary.md`](../../perf/0.4.0-rc.1/summary.md) の所見 1。**v0.5 で直す (PO の判断 2026-11-15)**。v0.4.0 には入れない。繰り越しは、昇格の後に 08 §7 に記録する (凍結の間は `docs/compat/`・`docs/perf/` 以外を入れないため)。
- **性能 (BM-106 と機械の状態)**: 計測の間、この機械は速い状態だけだった (状態の目安を 16 回測って 13.78〜14.20 ms)。09 §5.2 は BM-106 を遅い側の値で判定するとしているが、遅い状態 (約 2.3 倍) の値は取れていない。遅い状態の見込みでは、japanese・emoji の `end` と `open` の p99 (約 25〜35 ms) が Should の 50 ms を超えうる (Must の 100 ms は超えない見込み)。超えたときは 08 §7 に繰り越しを記録する。
- **BM-101a-locale (参考)**: 全訳のユーザーの言語ファイルを置いた起動の増分は 3.13 ms で、07 §17.4 の目安 (3 ms) をわずかに超えた。判定の条件ではない。上の BM-102a の直し方 (静的な表) で重ね合わせの費用も下がる見込み。
- **`cargo audit` の情報の警告 (RUSTSEC-2026-0486)**: `rustix` 1.1.5 の xattr の関数が、長さ 0 のバッファを渡されたときに書いていないバイトを初期化済みとして扱う (unsound)。`rustix` は `tempfile` を通して入るだけで、xattr の関数は使っていない (evoeditor の xattr は `eve-platform` の `sys/unix/fs.rs` が `libc` を直に呼ぶ)。脆弱性の扱いではなく、`cargo audit` も `deny advisories` も成功。`rustix` の更新はリリースの後に入れる。
- **夜間の性質テストの反例 (rc.2 で直す)**: 不正バイトを含む文書を保存するときのつながりの検査 (03 §8.3.1、M23) で、UTF-16 (LE / BE) の文書の確認 `ui.confirm.save.escape_merge` に出す「変わる範囲」(文字数・行数・末尾まで) が、実際より大きく出る (文書の長さを超えることもあり、誤って「ファイルの末尾まで」と出ることもある)。原因は `crates/eve-encoding/src/merge.rs` が、並びの局所の検査が決まる前に範囲を閉じ、後で決まったつながりで範囲を開き直して重ねて数えること。**データは失われない**: 確認が出るか・承認と照らし合わせるかは件数・記録・ダイジェストで決まり (`io_jobs.rs` の `approved`)、範囲の値は確認の文言にしか使わない。区切った確認 (ii) の区切りも正しい。UTF-8・Shift_JIS・EUC-JP では起きない (乱数の探索で各 60〜90 万件、0 件)。修正は `merge.rs` だけで、0.4.0-rc.2 に入れる (PO の判断 2026-11-16)。
- **夜間ワークフローの実行の分け方**: fuzz の 5 ターゲット × 1 時間は 1 回の実行の上限 (240 分) に収まらないので、`only=fuzz:…` で 2 回に分けた (09 §8.2 の v0.4 の表)。同じ concurrency の群では待ちの実行は 1 つだけ残るので、2 本目は 1 本目の後に入れ直した (最初に入れた 2 本目 <https://github.com/evoref/evoeditor/actions/runs/43590688531> は取り消し)。
- **このファイルの「Claude (下書き)」の判定**: 最終の判定は PO。この PR のマージをもって PO の確認とする。
