# 11. 開発運用 (Development Workflow)

| 項目 | 内容 |
|---|---|
| 文書ID | 11-dev-workflow |
| 状態 | ドラフト |
| 対象 | リポジトリ全体の開発運用。人間と AI コーディングエージェントが同じ規約で並行作業するための取り決め |
| 前提 | [00-overview](00-overview.md) の用語 / [02 §4.6](02-architecture.md#46-eve-core-のモジュール所有境界) (モジュール所有境界) / [02 §8.4](02-architecture.md#84-境界型の契約と凍結段階) (契約と凍結段階) / [02 §10.5](02-architecture.md#105-駆動-api-stage-d) (駆動 API) / [02 §17](02-architecture.md#17-ai-連携に適した開発構成) (開発構成 = 構造の正本) / [09](09-testing-quality.md) (テスト階層・CI・品質ゲート) |
| 決定記録 | [ADR-0033](10-decisions.md#adr-0033-ai-連携に適した開発構成) (AI 連携に適した開発構成) を運用面で具体化する。関連: [ADR-0030](10-decisions.md#adr-0030-eve-host-と-frontend-trait境界型の契約と凍結段階) / [ADR-0032](10-decisions.md#adr-0032-拡張点の追加と駆動-api-stage-d) / [ADR-0001](10-decisions.md#adr-0001-仕様書駆動での開発) |
| v0.1 で読む節 | §1〜§9 (§7.1 の駆動 API は v0.2 以降。各節の「v0.2」と明記した記述は v0.1 では使わない、§0.1) (一覧の正本は [README §3.1](README.md#31-v01-の実装で読む節)) |

> **02 §17 が「構造」の正本、本書が「運用」の正本**。02 §17 は何を置くか (リポジトリ構成・規約の一覧) を決め、本書はそれをどう使うか (ID の付け方、タスクの切り方、検証の回し方、PR と CI の流れ、ブランチとコミットの規約) を決める。テスト階層・ベンチ・品質ゲートそのものは [09](09-testing-quality.md) が正本で、本書は参照するだけで再定義しない。
> 本書の対象は実装着手後の運用だが、[ADR-0001](10-decisions.md#adr-0001-仕様書駆動での開発) の「仕様確定まで実装しない」を変えるものではない。本書と M0c の成果物 (手書きの最小限: 手書きの `CLAUDE.md` 初版と `.claude/settings.json`、手書きの `docs/spec-index.toml` (節 ID → ファイルのみ)、AC のチェックリスト `tests/ac-checklist.toml`) は実装着手ゲート G1a の条件 ([08 §5](08-roadmap.md#5-実装着手の条件-仕様確定のゲート)) に含まれるため、着手前に用意する。**M0c の内容の正本は [08 §4](08-roadmap.md#4-マイルストーン) の M0c の行** で、本書は各成果物の書き方だけを定める。**v0.1 では規範行 ID を付けない** (確定 (ユーザー回答)、[ADR-0042](10-decisions.md#adr-0042-v01-の最小核-案-lwindows-のみの出荷日本語のみの-ui)、OQ-147)。設計の規範行 ID の付与・`docs/ownership.toml`・`.claude/task-template.md`・`xtask spec-index` (生成)・`xtask check-docs`・`AGENTS.md` とその生成器 (OQ-148)・`.claude/agents/` と `.claude/commands/` は v0.2 までに用意する (ADR-0039 / 0042)。開発用ツール (`xtask`・`spikes/`) の整備状況はゲートの対象外。
> `xtask` のサブコマンド名・ファイル名・DSL のキー名は「提案中」。実装時に変える場合は、人間が本書を同じ PR で更新する (`spec-change`。エージェントは §4.5 の起票案を出す) (ADR-0001)。

---

## 目次

0. [本書の範囲](#0-本書の範囲)
1. [リポジトリ構成](#1-リポジトリ構成)
2. [crate の粒度とモジュール所有境界](#2-crate-の粒度とモジュール所有境界)
3. [規範行 ID と `spec:` / `verifies:`](#3-規範行-id-と-spec--verifies)
4. [タスクの単位と PR の形](#4-タスクの単位と-pr-の形)
5. [`CLAUDE.md` / `AGENTS.md` / `.claude/`](#5-claudemd--agentsmd--claude)
6. [決定的なテストとシナリオ DSL](#6-決定的なテストとシナリオ-dsl)
7. [ヘッドレス検証ループ](#7-ヘッドレス検証ループ)
8. [コードレビューと CI の運用](#8-コードレビューと-ci-の運用)
9. [ブランチ・コミット・リリースの規約](#9-ブランチコミットリリースの規約)
10. [未決事項](#10-未決事項)

---

## 0. 本書の範囲

本書は次を **決める**。

- 規範行 ID の付け方と、`spec:` / `verifies:` の記法、`xtask spec-index` / `xtask trace` の入出力 (§3)
- 要件 → タスク → テストの単位、タスクテンプレート、PR の形 (§4)
- `CLAUDE.md` / `AGENTS.md` / `.claude/` に何を書き、何を書かないか (§5)
- シナリオ DSL の書式 (正本)、ゴールデン / スナップショットの運用、ASCII Frame の表現 (§6)
- ヘッドレス検証ループの 3 経路の使い分けと、ローカル LLM を開発ループに使うときの規則 (§7)
- レビューと CI の車線、ラベル、契約変更要求の手続き (§8)
- ブランチ・コミットメッセージ・CHANGELOG の規約 (§9.1〜9.3)
- 寄稿ポリシー (DCO / CLA)、公開 crate と外部の消費者の運用、ライセンス検査のプロファイル、秘密情報の扱い、脆弱性の受付・対応・開示の運用 (§9.4〜9.8。ライセンス・配布の方針そのものは 12)
- docs の表を CI の入力にするときの機械可読の規則 (§8.5)

次は **決めない** (他文書の責務)。

| 事項 | 担当 |
|---|---|
| リポジトリ構成・crate 一覧・モジュール所有境界の **内容** | [02 §4.1](02-architecture.md#41-crate-一覧) / [02 §4.6](02-architecture.md#46-eve-core-のモジュール所有境界) / [02 §17](02-architecture.md#17-ai-連携に適した開発構成) |
| 境界型の契約規則と凍結段階 (C0〜C3) の **内容** | [02 §8.4](02-architecture.md#84-境界型の契約と凍結段階) (本書は手続きだけ) |
| 駆動 API のメソッド・フレーミング・権限 | [02 §10.5](02-architecture.md#105-駆動-api-stage-d) (本書は使い方だけ) |
| テスト階層 (L0〜L7)、ベンチ ID、CI ジョブ、品質ゲート | [09](09-testing-quality.md) (本書は車線とラベルの運用だけ) |
| 版・マイルストーン・着手ゲート | [08](08-roadmap.md) |
| ライセンス・寄稿・配布の方針 | 12-licensing-distribution (本書は寄稿・外部の消費者・検査・秘密情報の **運用** だけ) |
| 文書の運用ルール (ID の不変性、他文書への指摘) | [00 §7.1](00-overview.md#71-文書運用ルール) |

### 0.1 v0.1 の運用 (最小核) と v0.2 で足すもの

v0.1 は最小核 (案 L) で、開発運用も手書きの最小限にする (確定 (ユーザー回答)、[ADR-0042](10-decisions.md#adr-0042-v01-の最小核-案-lwindows-のみの出荷日本語のみの-ui)、10 の「v0.1 の範囲から外した項目」#38 / #42 / #43 / #44)。本書の各節は v0.2 以降の仕様も残し、その箇所に「v0.2」等と明記する。

| 事項 | v0.1 | v0.2 以降 |
|---|---|---|
| 仕様の参照 (§3) | `spec:` は節 ID (`03 §9.3`)、`verifies:` は AC ID (`AC-FR-105-1`)。**規範行 ID は付けない** | 設計の規範行 ID (`<文書>-<節>-<連番>`、§3.1) を付けて併用 (v0.2) |
| 仕様の索引 (§1・§3.3) | 手書きの `docs/spec-index.toml` (節 ID → ファイルのみ、行番号なし) | `xtask spec-index` の生成物 (行番号付き)、`--show` / `--check` (v0.2。M10 で入れた) |
| トレーサビリティ (§3.3) | AC のチェックリスト `tests/ac-checklist.toml` (人が更新、`xtask check` が軽い検査) + PR テンプレートのチェックリスト | `xtask trace` (v0.2。M10 で入れた。範囲は 09 §8.3 の「M10 の実装」) |
| 担当の割当 (§2.3) | タスク (§4.2) の「触ってよい範囲」で示す | `docs/ownership.toml`・`xtask check-docs` (v0.2。M10 で入れた) |
| シナリオ (§6) | Rust の `Harness` (語彙を DSL と同じにする) | TOML のシナリオ DSL と `xtask scenario run` (v0.2) |
| CI (§8.1) | PR / main = Windows x86_64 (全テスト・ConPTY の E2E 10 シナリオ) + Linux x86_64 (Linux CI の対象 crate = core 系 + `eve-host` + `eve-testkit` のビルドとヘッドレスのテスト、09 §6.4)。夜間の fuzz は `fuzz_decode` のみ | macOS の CI・Tier B・iai・soak・補助ジョブ・area-check (v0.2)。Frame JSON ゴールデン・契約変更要求の起票は TUI 完成後 |
| UI 文言 (§8.2) | `ja` のみ。`ja` の文言の人手レビューとキー集合の検査 (`ja` とコードの MessageKey) | 英語 UI を同梱する v0.2 で訳の運用を加える (確定 (ユーザー回答)、OQ-21) |
| 出荷と開発環境 | Windows のみ出荷 (Windows 11 の Windows Terminal と conhost。Windows 10 は公式には対応しない)。開発環境は Windows | macOS / Linux の出荷 (v0.2、確定 (ユーザー回答)。macOS の CI ジョブと Unix の実装を加える) |

---

## 1. リポジトリ構成

構成の正本は [02 §17.1](02-architecture.md#171-リポジトリ構成-案)。本書は各ディレクトリの **所有者と変更の仕方** を定める。

| パス | 内容 | 所有 | 変更の仕方 |
|---|---|---|---|
| `docs/*.md` (本編) | 仕様 (正本)。`00`〜`12` と README | 人間 (PO + 設計者) | 実装中の変更は §4.5 の「仕様の変更規律」。**エージェントは本編を編集しない** (`.claude/settings.json` で拒否、§5) |
| `docs/spec-index.toml` | 仕様の索引。v0.1 は節 ID → ファイル、v0.2 から規範行 ID・節 ID → ファイルと行 | v0.1: 人間 (手書き)。v0.2: `xtask spec-index` (**生成物**) | **v0.1 は手書きの TOML で、節 ID → ファイルの対応だけ** (行番号なし、M0c で作る、08 §4)。見出しを足す・変える `spec-change` の PR で人間が同時に更新する。**v0.2** から `xtask spec-index` の生成物にし、手で編集しない (CI が最新性を検査、09 §6.2)。生成物になった後は、エージェントも `xtask` を通してなら更新してよい (§4.4) |
| `docs/ownership.toml` (v0.2) | モジュール・crate → 作業者の割当表 (手書き、§2.3) | 人間 | **v0.2** から置き、PR で更新する。形式の検査 (`xtask check-docs`) も v0.2。v0.1 は置かず、担当範囲はタスク (§4.2) の「触ってよい範囲」で示す |
| `docs/compat/<version>/` | 版ごとの記録 (compat / acceptance) と **`release-checklist.md`** (リリースのチェックリスト。09 §8.2 の表のうち **タグの前に済む行だけ** を 1 行ずつ埋め、ゲートを判定したコミットの SHA を書く) | メンテナ | 各リリースで作り、**タグの前に `develop` にコミット** して、draft の Release を publish する前にすべて埋める。タグの後の結果 (`release.yml` の検証・実行の URL・MSRV と配布経路の検証・更新手順の文書の確認・公開後の検証) は GitHub Release のノートに書き、リポジトリには戻さない (09 §8.2 / §9.2)。基準機の固有情報を書かない (§9.7) |
| `docs/perf/<version>/` | 基準機での性能の記録 (09 §5.3)。ベンチ ID ごとの JSON (`<ベンチ ID>.json`) と、それを表にした `summary.md` (p50 / p99・最大値・合否)。計測用ビルドの導入時の `EVE_TRACE_FILE` のオーバーヘッドの記録も置く | メンテナ | **計測の手順** (基準機 ENV-W1、09 §5.3 の条件): (1) 計測用ビルドを作る (`cargo build --release --features test-hooks,bench-trace`。配布ビルドと同じ release プロファイル)、(2) `cargo xtask gen-corpus --seed 42` でデータを作り内蔵 SSD の作業ディレクトリに置く、(3) `cargo xtask bench --env ENV-W1 --out docs/perf/<version>/` で v0.1 の判定に使うベンチ (09 §5.2) を回す (個別には `cargo xtask bench <ベンチ ID>`。各ベンチの回数・統計は 09 §5.2)、(4) 出力の JSON (1 ファイル 1 ベンチ ID。ベンチ ID・コミット・環境 ID・回数・p50 / p99 / p99.9 / 最大値・合否) と `summary.md` を `develop` にコミットする (リリースの前、09 §9.2 手順 3)。L5 の時間の判定 (09 §3.6) の結果も同じ置き場所に書く。環境 ID だけを書き、基準機の固有情報を書かない (§9.7)。CI (`bench.yml`) の値は Actions の artifact に置き、ここにはコミットしない (09 §6.1)。**v0.1 の実装**: `--out` を付けると、`xtask bench` は計測用ビルド (`--features bench-trace`。上の (1) の `test-hooks` は要らない) で `bins/evoeditor/benches/bm_pty.rs` を回し、プロセス内の区間の記録 (`EVE_TRACE_FILE`、09 §3.6) から BM-101a / 101b / 102a / 102b / 103b / 107 を仕様の区間で測る (判定に使う値)。ConPTY 経由で画面に届くまでの端末込みの値 (BM-101c / 102c / 103c / 107c) は参考として併記する。BM-103a (診断用) と端末込みの値は `summary.md` で「参考」とし、そのほかは目標があれば合否を付ける。BM-101a / 102a / 101b / 102b / 103b を測るには計測用ビルドが要る (`--out` を付けた `xtask bench` は自動でそうする) |
| `docs/reference/` | 生成リファレンス (コマンド一覧・キーマップ・設定、09 §11.2) | `cargo xtask gen-reference` (bin の `--list-commands --json` 等の JSON を Markdown に変換する) | `docs.yml` で再生成し (`--check`、windows)、差分があれば失敗。生成物なので、エージェントも生成コマンドを通してなら更新してよい |
| `CLAUDE.md` / `AGENTS.md` | エージェント向けの短い入口 (§5) | 人間 | 仕様の再掲をしない。本書と 02 §17 への参照に留める |
| `CONTRIBUTING.md` | 人間向け (09 §11.4) | 人間 | — |
| `.claude/` | 設定・役割別エージェント・コマンド (§5) | 人間 | — |
| `Cargo.toml` / `Cargo.lock` / `deny.toml` / `deps-policy.toml` (いずれもリポジトリ直下) | workspace、依存規則 (09 §7)。`deny.toml` の `[advisories] ignore` が脆弱性の例外の唯一の置き場所 (09 §6.3) | メンテナ (`CODEOWNERS`) | 依存追加は 02 §13.0 のチェックリスト。`deps-policy.toml` は 02 §4.1 の「禁止 crate」列と同じ PR で更新する (09 §7) |
| `.config/nextest.toml` | nextest の profile (`default` / `ci` / `e2e`)・`retries`・`slow-timeout` (09 §3.6 / §6.2) | メンテナ | L5 以外に `retries` を付けない (09 §3.6) |
| `.cargo/config.toml` | cargo の設定。`x86_64-pc-windows-msvc` で CRT を静的にリンクする `rustflags = ["-C", "target-feature=+crt-static"]` (Visual C++ 再頒布パッケージの無い Windows 11 で起動するため。内容の正本は 05 §16.4。配布ビルド・計測用ビルド・テストのビルドが同じ CRT になるようにリポジトリ直下に置く)。`cargo install --git` はリポジトリの `.cargo/config.toml` を読まない (cargo は設定を実行時のカレントディレクトリから探す) ので、`cargo install` の経路での CRT の扱い (README で `RUSTFLAGS` を案内するか) は 05 §16.4 / §18.2a に従う (**要確認**) | メンテナ | M1 のタスク (0) の成果物 (08 §2.2)。変える PR では配布経路の検証 (09 §8.2) のクリーンな Windows 11 での起動を確かめ直す |
| `.github/` | ワークフロー (09 §6。M1 のタスク (0) から)・Issue と PR のテンプレート (最初の push の前)・`dependabot.yml` (cargo と GitHub Actions、週 1・グループ化。最初の push の準備で置く、09 §6.3・12 §5.6 の PC-5 (8))・`allowed_signers` (リリースのタグの SSH 署名の公開鍵。`release.yml` の `git verify-tag` 用、§9.3 / §9.7)・`fuzz-recipients.txt` (fuzz のクラッシュ入力を暗号化する `age` の受取人の公開鍵。メンテナの数だけ 1 行 1 鍵。秘密鍵は置かない、09 §6.3) | メンテナ | ワークフローの権限は 09 §6.5 の表に従う。fork の PR では変更を読んでから実行を承認する (§9.4) |
| `crates/<name>/` | ライブラリ crate (02 §4.1) | crate ごとの所有者 (§2) | 所有者以外は PR で提案。`eve-view` は core の所有者だけが変更する (§8.4) |
| `bins/evoeditor/` | 引数解析と `Host` の起動だけ | eve-host の所有者 | ビジネスロジックを置かない (02 §4.1) |
| `crates/eve-bench/` | マクロベンチの bin (09 §5.1、`publish = false`)。Windows だけでビルドする | メンテナ | ベンチ ID (09 §5.2) を変えるときは 09 を同じ PR で更新 |
| `crates/eve-docs-tables/` | docs の Markdown の表を §8.5 の規則で読む lib (`publish = false`)。`xtask` と、eve-tui・eve-core のテスト (dev 依存) が使う。Windows と Linux の両方でビルドする | メンテナ | 表の規則 (§8.5) を変える `spec-change` の PR で同時に直す |
| `crates/eve-test-helpers/` | テスト用の補助プロセスの bin crate (`console-event`・`clip-delay`、09 §3.4 / §3.6。`publish = false`、feature なし)。test-hooks のビルドのテストからだけ使い、配布物に入れない | メンテナ | — |
| `tests/ac-checklist.toml` | v0.1 の AC のチェックリスト (§3.3。M0c で作る) | 人間 | 人が更新する。`docs/` の外に置くので、`.claude/settings.json` の docs 本編への書込み拒否と `spec-change` ラベルの対象外 (エージェントは PR 本文に AC ID とテスト名を書き、更新は人間) |
| `fuzz/` | cargo-fuzz のターゲットと最小コーパス (`fuzz/corpus/<target>/`、09 §3.3。夜間に増えた分は Actions のキャッシュでコミットしない)。本体の workspace から除外した別の crate (独自の `Cargo.lock`) | メンテナ | `cargo audit`・`cargo deny`・Dependabot の対象に含める (09 §6.3)。クラッシュ入力は `age` で暗号化した artifact でメンテナが受け取り、手元で復号・再現・最小化してから回帰テストに転記する (09 §6.3) |
| `xtask/` | v0.1: `check` (速い車線。`tests/ac-checklist.toml` の軽い検査を含む、§3.3 / §8.1), `check-deps`, `verify-tag` (リリースのタグの検証。`release.yml` の最初のステップ、09 §6.1), `review-wording` (日本語 UI 文言の人手レビュー用の一覧 `docs/review/ja-wording-v0.1.md` の生成。リリース前の文言の確認、09 §9.2), `dist` (配布 zip・`SHA256SUMS` の作成と検査。`--verify ZIP` で検査だけ。05 §18.2), `gen-notices` (`THIRD-PARTY-NOTICES.md` の生成と `--check`。cargo-about、§9.6), `check-dist` (配布ビルドにテスト専用の経路が無いことと CRT の静的リンクの確認。09 §6.2 の `build-release-smoke`、`--exe` で既存の exe も検査する), `check-themes`, `i18n`, `gen-corpus`, `bench` (基準機の計測。手順は上の `docs/perf/` の行), `gen-reference` (`docs/reference/*` の生成と `--check`。bin をビルドして JSON を出させるので Windows で動かす。bin ができる M2 から), `nightly` (夜間ジョブの入口、09 §6.1) (`i18n` はキー集合とヒント幅、`check-themes` は truecolor と mono の 2 段、09 §6.2。`check-themes`・`i18n`・`check-deps` は docs の表を §8.5 の規則で `eve-docs-tables` を通して読む)。v0.2: `spec-index`, `trace`, `scenario run` / `scenario check`, `screenshot`, `check-docs`, `check-assets` (同梱資産の台帳の検査。最初の同梱資産と同時に入れる。v0.1 は同梱資産が無いので置かない、§9.6・12 §3.6)。v1.L: `ai-mock` / `ai-record` (`ai-record` は人間だけが実行)。**Windows と Linux の両方でビルドする** (ubuntu の CI ジョブでも動かす、09 §6.4) | メンテナ | サブコマンドの追加は本書と 09 §11.4 を同じ PR で更新。開発用ツールなのでゲートの対象外 |
| `tests/scenarios/` (v0.2) | シナリオ DSL (§6.3)。v0.1 のシナリオは各 crate の `tests/` に Rust の `Harness` で書く (§6)。どちらも **エージェントが最初に書くテスト** | 誰でも | PR に含める。`accept` (スナップショット更新) は人間 (§4.4) |
| `testdata/` | コーパス (09 §4)。`xtask gen-corpus` の生成物はコミットしない | 誰でも | 本文を含むファイルは追加前に出自とライセンスを確認 |
| `spikes/<S-nn>/` | 使い捨てスパイクと `REPORT.md` (08 §3) | スパイク担当 | 本体 crate から参照しない。採用コードは書き直して crate へ |
| `.gitignore` | git の除外 | メンテナ | 製品のパス (`bins/evoeditor/`・`crates/`) を除外しない。ルート直下の成果物だけを除外するときは `/evoeditor` のように先頭に `/` を付ける (スラッシュ無しの `evoeditor` は `bins/evoeditor/` まで除外する)。M1 のタスク 0 (08 §2.2) で確かめる |

---

## 2. crate の粒度とモジュール所有境界

### 2.1 crate = 作業集合

1 crate は **1 人の作業者 (人間または AI エージェント) のコンテキストに収まる粒度** にする (02 §17.2)。目安は「crate の `src/` と、その crate が所有する仕様節 (§2.2) を合わせて 1 セッションで読める量」。これを超える crate は分割せず (ADR-0003 (c))、モジュール所有境界 (§2.2) で衝突を防ぐ。

| crate | 所有する仕様 (読む範囲) | 主な作業者の想定 |
|---|---|---|
| `eve-text` | 03 §3〜§5、§13、§14 の幅計算 | 1 人 |
| `eve-encoding` | 03 §8 | 1 人 (`eve-text` と兼任可) |
| `eve-view` | 02 §8 (境界型)。変更は契約変更要求 (§8.4) | core の所有者 |
| `eve-config` | 07 §2〜§9 | 1 人 |
| `eve-core` | 02 §5〜§7、03 の大半 (モジュールごとに分割、§2.2) | **core の所有者は 1 人 (人間)**。モジュール単位でエージェントに委譲 |
| `eve-platform` | 05 (v0.1 は Windows の部分) | 1 人 (macOS / Linux を出荷する版から OS ごとに分担可) |
| `eve-host` | 02 §9、05 §11 | core の所有者または eve-platform の所有者 |
| `eve-tui` | 04 | 1 人 |
| `eve-testkit` の `HeadlessFrontend` と `SteppingHarness` (v1.0 まで、09 §3.4) | 02 §8.4、09 §3.4 | メンテナ。テスト用の第 2 の `Frontend` 実装 (v0.1 から ASCII の Frame を作る) |
| `eve-protocol` / `eve-testkit` | 02 §10.5、09 §3.4、本書 §6〜§7 | メンテナ |

### 2.2 `eve-core` のモジュール所有境界

モジュールと所有する仕様節の表は [02 §4.6](02-architecture.md#46-eve-core-のモジュール所有境界) が正本。運用規則:

1. 各モジュールの `mod.rs` (または `<name>.rs`) の冒頭に、所有する仕様節を `//! spec: 03 §6–7, 02 §5` の形で書く。v0.1 はレビューで手書きの索引と照合し、v0.2 からは `xtask spec-index --check` がこの行の節が存在することを検査する。
2. モジュール間は `pub(crate)` の API だけを見せる。`eve-core` の `pub` は `Editor` と境界型の再公開に限る (02 §4.6)。
3. **1 つのタスクは 1 つのモジュールを主対象にする** (§4.1)。他モジュールへの変更が要るときは、(a) 先に当該モジュールの所有者へ「API 追加」のタスクを切る、(b) または同一作業者が順に行う。同じモジュールを 2 つのエージェントが同時に触らない。
4. モジュールの追加・分割・所有節の変更は 02 §4.6 の表を同じ PR で更新する (人間がレビュー)。

### 2.3 `docs/ownership.toml`

**v0.2 から置く** (ADR-0042、10 #44)。v0.1 は単独開発なので置かず、エージェントの担当範囲はタスク (§4.2) の「触ってよい範囲」「触ってはいけない」で示す。

人間とエージェントへの割当表。CODEOWNERS と違い、GitHub の機能ではなく **エージェントが自分の担当範囲を判定する** ために読む (OQ-126)。

```toml
# docs/ownership.toml (案)
version = 1

[[unit]]
path   = "crates/eve-core/src/document"   # crate、またはモジュールのディレクトリ
spec   = ["02 §5.1-5.2", "03 §6", "03 §7"] # 所有する仕様節 (spec-index の節 ID)
owner  = "human:core"                       # human:<役割> | agent:<名前> | shared
stage  = "C1"                               # 契約段階の影響を受ける場合のみ (eve-view と frontend)
notes  = "線形 Undo (03 §7.1) は所有者以外が変更しない"

[[unit]]
path   = "crates/eve-tui"
spec   = ["04"]
owner  = "agent:tui"
reviewer = "human:core"                     # 承認に必要な人間
```

- `owner = "agent:*"` の unit でも、**承認 (merge) は常に人間** (§8.2)。
- `xtask check-docs` (v0.2) は `path` が存在し `spec` の節が索引に存在することを検査する。割当の変更は PR で行い、理由をコミットメッセージに書く。
- **M10 の初版 (2026-10-31)**: unit は実在するパスで書く (02 §4.6 の表のモジュール名は例で、eve-core の実際の構成は `document/`・`editor/`・`editor/layout`・`jobs.rs`・`io_jobs.rs`・`search` など)。v0.2 は人間 1 名とエージェントで進めるので、`owner` はすべて `shared`、`reviewer = "human:maintainer"`。入れ子の path (`editor` と `editor/layout`) は、深いほうがその範囲を持つ。`spec` は節 ID (`"03 §9"`)・範囲 (`"02 §5.1-5.2"`)・文書全体 (`"04"`) のいずれか。

---

## 3. 規範行 ID と `spec:` / `verifies:`

**v0.1 の記法** (確定 (ユーザー回答)、[ADR-0042](10-decisions.md#adr-0042-v01-の最小核-案-lwindows-のみの出荷日本語のみの-ui)、OQ-147): v0.1 では規範行 ID を付けない。コミット・コード・テストの `spec:` は **節 ID** (`03 §9.3`。既存の ID を持つ規範は節 ID に添えてよい: `02 §7.2 K1`)、`verifies:` は **AC ID** (`AC-FR-105-1`) で書く。§3.1 の規範行 ID の付与・`xtask spec-index` (生成)・`xtask trace` は v0.2 から。

### 3.1 規範行 ID の付け方

**v0.2 から** (v0.1 では付けない)。仕様の **規範的な** 表の各行・箇条書きに安定 ID を付ける (ADR-0033、NFR-708)。

| 規則 | 内容 |
|---|---|
| ID-1 既存 ID を優先 | すでに ID を持つ規範 (K1〜K8、INV-F1、MR1〜MR11、AI-INV-n、**LL-1〜LL-10 (06 §2.3 ローカル LLM の原則)**、CS1、N1、Q1、R1〜R11、T1〜T7、C1〜C4、E-01 のような表の `#` 列) はそのまま規範行 ID とする。文書内で一意でない短い ID (`C1` は 04 §12 と 02 §8.4 の両方にある) は、参照時に節を添えて `04 §12.1 C1` と書く |
| ID-2 無い箇所の形 | `<文書番号>-<節>-<連番>` (例 `04-4.2-3` = 04 §4.2 の 3 番目の規範行)。節は `#### ` の小節まで含めてよい (`03-8.6-2`)。連番は節内の出現順で振り、**削除しても詰めない** |
| ID-3 不変 | 一度振ったら再利用・振り直しをしない (00 §7.1 規則 2)。規範を廃止したら行を「廃止」と明記して残す。文言の修正は ID を変えない。意味が変わるときは新 ID を振って旧 ID を廃止する |
| ID-4 付ける対象 | 実装・テストが対応づけられる規範 (「〜する」「〜してはならない」「既定値は〜」「〜以下」)。説明文・根拠・例・却下した案には付けない |
| ID-5 書き方 | Markdown 上では表の先頭列 (`#` / `ID` 列) か、箇条書きの先頭に `**04-4.2-3**` の形で置く。既存の表に ID 列が無い場合は行末に `⟨04-4.2-3⟩` を付ける (索引は両形式を拾う) |
| ID-6 順序 | **v0.1 では付けない** (確定 (ユーザー回答)、ADR-0042、OQ-147): `spec:` は節 ID、`verifies:` は AC ID (01 の AC の ID をそのまま使う) で書き、ADR の決定は `ADR-0040 決定 4` の形 (番号の無い決定は `ADR-0016 決定`) で参照する。M0c では新しい ID を振らない (08 §4)。**v0.2 から** 設計の規範行 ID (ID-2 の形) を、各タスクの着手前にそのタスクが参照する節へ付ける (エージェントが案をまとめて出し、人間が承認する)。全文書への付与は v0.2 まで (OQ-147)。**全文書へ一括では付けない** (確定 (ユーザー回答)、2026-10-31): 試験・実装から参照する行と、その版で直す節にだけ順に付ける。`xtask trace` が Must の網羅を確かめる対象は 01 の AC ID とし、規範行 ID は付いたものだけを存在の検査に使う。付いていない節の規範は節 ID (`<文書> §<節>`) で参照する |
| ID-7 要件 ID との関係 | FR / NFR / AC は 01 の ID をそのまま使う。規範行 ID は要件を実現する **設計上の規範** に付けるもので、要件 ID の代わりではない。1 つの AC が複数の規範行に展開され、1 つの規範行が複数の AC に効く |

### 3.2 `spec:` と `verifies:` の記法

| 記法 | 置き場所 | 意味 | 例 |
|---|---|---|---|
| `//! spec: <節>[, <節>...]` | モジュールの冒頭 | このモジュールが所有する仕様節 (§2.2) | `//! spec: 03 §6–7, 02 §5` |
| `/// spec: <ID>[, <ID>...]` | 関数・型・impl の doc コメント | この項目が実装している規範 (v0.1 は節 ID、v0.2 から規範行 ID も) | `/// spec: 03 §7.1`、`/// spec: 02 §7.2 K1, 02 §7.2 K2` |
| `// spec: <ID>` | 関数の中の 1 ブロック | 分岐 1 つが規範に対応する場合 | `// spec: 03 §6.5 (Slice は 64 KiB 以上のみ)` (v0.2 からは `03-6.5-4` の形も) |
| `// verifies: <ID>[, <ID>...]` | テスト関数の直前 (v0.1 の `Harness` のシナリオテストも) | 検証している AC (v0.2 から規範行も) | `// verifies: AC-FR-105-1` (v0.2 からは `AC-FR-105-1, 05-4.2-3` の形も) |
| `verifies = [...]` (v0.2) | シナリオ DSL `[meta]` (§6.3) | 同上 | `verifies = ["AC-FR-206-2", "04-6.6-1"]` |
| `verifies:` 列 | 手動チェックリスト (09 §3.7〜3.8)、ベンチ表 (09 §5.2) | 同上 | — |

- ID の表記は索引と同じ文字列にする (`§` と節番号の間に空白 1 つ、範囲は `–`)。v0.2 の `xtask trace` は文字列一致で突合する。
- `spec:` を書く単位は「レビュアがその規範を確認するときに読む範囲」。1 関数に 10 個並べるより、関数を分けて 1〜3 個ずつ書く。

### 3.3 `xtask spec-index` と `xtask trace`

| コマンド | 入力 | 出力 | 検査 | 版 |
|---|---|---|---|---|
| `xtask spec-index` | `docs/**/*.md` | `docs/spec-index.toml`: `[[id]] id = "04-4.2-3", file = "docs/04-tui-design.md", line = 412, section = "4.2", title = "..."` と、節 ID (`04 §4.2`) → 行範囲 | `--check` で生成物との差分、重複 ID、`//! spec:` が指す節の存在 | **v0.2**。v0.1 の索引は M0c で人間が手書きする TOML で、`[[section]] id = "04 §4.2", file = "docs/04-tui-design.md"` の形の節 ID → ファイルだけ (行番号なし) |
| `xtask trace` | ソース中の `spec:` / `verifies:`、シナリオの `verifies`、ベンチ表、手動チェックリスト、01 の要件表、spec-index | `docs/traceability.md` (09 §8.3) | 存在しない ID → 失敗。現在の版の Must に紐付く規範行で `spec:` も `verifies:` も無いもの → 警告 (版の終盤に失敗へ、09 §8.3) | 記法は v0.1 から。**xtask は v0.2** (v0.1 は下の AC ID のチェックリストと PR テンプレートのチェックリスト、§4.4) |
| `xtask spec-index --show <ID>` | ID または節 | 該当節の本文 (前後 N 行) | — | **v0.2** (v0.1 は手書きの索引でファイルを引き、該当見出しを読む) |

**M10 の実装 (2026-10-31)**: `cargo xtask spec-index` が `docs/spec-index.toml` を作り直す (生成物。手で直さない)。
- `[[section]]`: 見出しの節番号から節 ID (`03 §9.3`、README は `README §3.8`)、`line` (見出しの行)・`end` (次の同じ水準以上の見出しの直前)・`title`。番号の無い見出しとコードフェンスの中は拾わない。
- `[[id]]`: 規範行 ID (ID-5 の 3 形式。コード表記の中の例は拾わない)、表の先頭列の既存の ID (ID-1。英大文字で始まり数字を含むもの。同じ文書で 2 回目以降は載せない)、10 の ADR / OQ の見出し。`id`・`file`・`line`・`section` (その行の節 ID)。AC ID は 01 §5 の表から `xtask trace` が読む (索引には載せない)。
- `--check` (`xtask check` の (5b) と、xtask の試験 `リポジトリの索引は最新`): 生成物との差分、規範行 ID・ADR / OQ・節 ID の重複、ソースのコメントの `spec:` が指す `NN §a.b` の存在。**docs の見出しを足す・変える PR は、同じ PR で `cargo xtask spec-index` を実行して索引も更新する**。
- `--show <ID>`: 節 ID なら節の全体、ID ならその行から節の終わりまで (最大 20 行)。
- `cargo xtask trace` (M10): `docs/traceability.md` を作り直す。**`verifies:` を足す・変える PR は、同じ PR で `cargo xtask trace` を実行して行列も更新する** (`xtask check` の `trace --check` が古い行列を検出する)。範囲は 09 §8.3 の「M10 の実装」。

- **AC ID のチェックリスト (v0.1、M0c (d))**: 置き場所は **`tests/ac-checklist.toml`** (M0c で作る)。v0.1 の Must の AC ID (01 §5 の AC をそのまま `AC-FR-105-1` の形で) を 1 件ずつ `[[ac]] id = "AC-FR-105-1"`、`verified_by = ["テスト名・ベンチ ID・手順"]` (09 §8.5)、`status = "todo" | "done"` の形で並べる。人間が M0c で作り、AC を満たす PR を merge するときに人間が更新する (エージェントは PR 本文に対応する AC ID とテスト名を書く)。`docs/` の外に置くので、docs 本編 (`docs/*.md`) の書込み拒否 (§5) と `spec-change` ラベル (§8.3) の対象外。**軽い検査** (v0.1 の `xtask check` に入れる): (1) 01 §5 の v0.1 の Must の AC ID がすべてチェックリストにある (欠落は失敗)、(2) テストの `verifies:` に書かれた AC ID がすべて 01 に存在する (打ち間違いは失敗)、(3) **`status = "done"` の項目は、`verified_by` に実在するテスト (ソース中のテスト関数名で、その関数の直前に同じ AC ID の `verifies:` がある) か、ベンチ ID (09 §5.2 にあるもの) か、手順の記録 (`docs/compat/` の記録ファイルの節への参照) が 1 つ以上ある** (無ければ失敗)。01 の AC ID は `eve-docs-tables` で 01 §5 の表から §8.5 の規則で読む。v0.1 の `xtask trace` の代用で、リリースゲート (09 §8.2) の「Must の AC 全合格」はこのファイルで確かめる。v0.2 で `xtask trace` の生成物 (`docs/traceability.md`) に置き換える。
- **エージェントは索引で該当節だけを読む** (02 §17.2)。全文を読む必要があるのは所有する crate の仕様節 (§2.1) だけ。
- 索引はコミットする (エージェントが `xtask` を動かさずに読めるように)。v0.2 で生成物にした後は、CI の `spec-index` ジョブが古くなった索引を検出する (09 §6.2)。

---

## 4. タスクの単位と PR の形

### 4.0 最初のタスク (M1 の縦切り)

M1 の最初のタスクの順は [08 §2.2](08-roadmap.md#22-v01-最小核) の「最初の縦切り」が正本 (workspace の骨格・`.cargo/config.toml` (`+crt-static`)・`deny.toml` (05 §17.1 の許可リスト。ADR-0018 は確定)・CI の最小構成 (Windows と Linux)・`xtask` (`gen-corpus`・`check-deps`・`i18n`・`check`) → eve-text と S-02 の判定ベンチ → eve-testkit のハーネス (Rust の `Harness`・`MemFs`・手動時計) → eve-core / eve-host の端末に依存しない部分 → ヘッドレスでの起動から最初の Frame の生成までのベンチ。BM-101a は M2)。本書の単位 (§4.1) とテンプレート (§4.2) は、骨格のタスクが終わり `cargo xtask check` が動いた後のタスクに適用する。

### 4.1 タスクの単位

要件 → タスク → テストの対応を機械的に保つため、タスクは次の単位で切る。

| 条件 | 内容 | 理由 |
|---|---|---|
| 1 crate、原則 1 モジュール | 主対象は 1 crate。`eve-core` では 1 モジュール (§2.2) | 衝突の回避、コンテキストの節約 |
| 1 AC 群 | 01 の AC を 1〜5 個 (v0.2 からは規範行 ID の集合も)。タスクの完了 = その AC の `verifies:` テストが緑 | トレーサビリティ |
| 差分 ≤ 300 行 (目安) | 生成物 (スナップショット・索引) を除く。超えるなら分割 | レビュー可能性 |
| ヘッドレスで検証可能 | 完了判定がシナリオ (v0.1 は Rust の `Harness`、v0.2 から DSL も、§6) または L0〜L3 テストで書ける。端末の見た目だけが成果のタスクは、ASCII Frame のスナップショット (§6.5) を完了判定にする | Q1 (09) |
| 契約を越えない | `eve-view` の変更を含むタスクは core の所有者のタスクとして先に切る (§8.4) | 02 §8.4 |

### 4.2 タスクテンプレート

Issue の本文の必須項目 (v0.2 からは `.claude/task-template.md` にも置く)。エージェントへの指示はこのテンプレートで渡す。下は v0.1 の形。

```markdown
## タスク: <動詞 + 対象> (例: 線形 Undo の保存点を実装する)
- crate / モジュール: eve-core / document (owner: human:core, agent 委譲可)
- 仕様: 03 §7.1, 03 §7.4   ← 読む範囲 (手書きの索引でファイルを引く。v0.2 は `xtask spec-index --show`)
- AC: AC-FR-203-1, AC-NFR-203-1
- 完了条件:
  - [ ] `verifies:` を持つテスト (L0/L1/L3) が上の AC を全部カバーし緑
  - [ ] `Harness` のシナリオテスト `crates/eve-core/tests/undo.rs` の `save_point` が通る (v0.2 は `tests/scenarios/undo/save_point.toml` を `xtask scenario run`)
  - [ ] `cargo xtask check` 緑 (2 分車線)
  - [ ] 変更した関数に `spec:` (節 ID) がある
- 触ってよい範囲: crates/eve-core/src/document/**, crates/eve-core/tests/undo.rs
- 触ってはいけない: crates/eve-view (契約), 他モジュールの pub(crate) API (必要なら先に API タスクを起票)
- 仕様と違う判断が要ったら: 実装せず OQ 起票案を PR 本文に書く (§4.5)
```

v0.2 からは「規範行: 03-7.1-1, 03-7.1-2, INV-H2」の行を加える (§3.1)。

### 4.3 バックログの持ち方

- v0.1 のバックログは 01 §5 の AC を **AC 単位で** Issue 化し、AC ID のチェックリスト (§3.3) の「未」の行と一致させる (v0.2 からは `xtask trace` の警告 (対応テストの無い Must) と一致させる)。Issue のラベルは `crate:<name>`、`spec:<節>`、`ac:<ID>`、`agent-ok` (エージェントに委譲してよい)。v0.1 はラベルを手で付ける (自動付与は v0.2)。
- マイルストーン (M1〜M5、08 §2.2) は Issue のマイルストーンと 1:1。
- 「設計者裁量で確定した項目」(ADR-0031 等) は Issue にせず、仕様の節 (v0.2 からは規範行) として `spec:` の対象にする。

### 4.4 PR の形

| 項目 | 規則 |
|---|---|
| 1 PR = 1 タスク | §4.1 の単位。複数タスクをまとめない。生成物の更新 (索引・リファレンス) は同じ PR に含める |
| 本文の必須項目 | 対象の仕様節 (v0.2 からは規範行 ID も)、AC ID、動作確認に使ったシナリオ名、**仕様と違う判断をした箇所** (無ければ「なし」) |
| チェックリスト (PR テンプレート) | 09 §8.4 のレビュー項目 + 「変更した関数に `spec:` (節 ID) がある」「AC に `verifies:` (AC ID) がある」「シナリオテスト (v0.1 は `Harness`、v0.2 から `tests/scenarios/`) を追加・更新した (ユーザー可視の挙動の変更時)」「`eve-view` を触っていない (触った場合は §8.4 の記載)」。**v0.1 の `xtask trace` の代用はこのチェックリストと AC ID のチェックリスト (§3.3)** (09 §8.3) |
| スナップショットの更新 | `cargo insta accept` (ASCII Frame を含む) の更新は **人間だけ** が行う (Frame JSON ゴールデンも同じ)。エージェントは差分を PR 本文に貼り、理由を書いて止まる (意図した変更かの判断を機械に委ねない) |
| 生成物 | `docs/reference/*` (と v0.2 からの `docs/spec-index.toml`) の差分は PR に含め、CI が再生成と一致を検査 |
| サイズ | 差分 300 行超は分割を求める (レビュアの裁量で例外) |
| エージェントの PR | PR 本文の先頭に `Agent: <名前>` と、使ったタスク (§4.2) を貼る。レビューと merge は人間 (§8.2) |

### 4.5 仕様の変更規律

実装中に仕様と異なる判断が必要になったとき:

1. **エージェントは docs を直接直さない**。実装も止める (暫定実装で進めると仕様と実装の二重化になる)。
2. PR 本文 (または Issue) に **OQ の起票案** を書く: 該当節 (v0.2 からは規範行 ID も)、何が実装できない / 望ましくないか、代替案と推奨、影響する AC。
3. 人間が 10 に OQ として取り込み (00 §7.1 規則 3)、回答後に docs を更新してから実装を再開する。軽微な記述修正 (誤字・リンク) は人間が直接直す。
4. 仕様に無い振る舞いを「とりあえず」足さない。足す必要があれば同じ手順で OQ。

---

## 5. `CLAUDE.md` / `AGENTS.md` / `.claude/`

エージェント向けの入口。**仕様の再掲はしない** (正本の二重化を避ける)。短く、参照で済ませる。

| ファイル | 内容 (書くこと) | 書かないこと |
|---|---|---|
| `CLAUDE.md` (Claude Code 向け) | (1) 最初に読むもの: 本書 §0.1・§2〜§7、02 §17、タスクの「触ってよい範囲」(v0.2 からは `docs/ownership.toml` の担当 unit)。(2) 読み方: 手書きの `docs/spec-index.toml` で節 ID からファイルを引き、該当見出しだけを読む (v0.2 からは `xtask spec-index --show <節>`)。仕様の参照の書き方: `spec:` は節 ID、`verifies:` は AC ID (規範行 ID は v0.1 では使わない、§3)。(3) 禁止: docs 本編 (`docs/*.md`) と `docs/spec-index.toml` (v0.1 は手書き) の編集、`eve-view` の変更、`insta accept`、`Cargo.toml` への依存追加 (チェックリスト無し)、`unsafe`、`std::fs` / `std::thread` を eve-core で使うこと。(4) 検証: `cargo xtask check` → `Harness` のシナリオテスト (v0.2 から DSL) → PR テンプレート。(5) 仕様と違うときの手順 (§4.5)。(6) コミットメッセージの形 (§9.2) と末尾行の規則 (`Signed-off-by:` は操作した人間の名義、`Agent: <名前>`、ツールが付ける `Co-Authored-By:` は残してよい)。(7) **版の範囲の歯止め**: v0.1 の作業では [README §3.1](README.md#31-v01-の実装で読む節) の節だけを読む。版の注記が v0.1 でない機能・コマンド・設定キー (v0.2 以降・v1.L) は作らない (型・名前の予約だけのものは予約どおりに置く。10 の「v0.1 の範囲から外した項目」)。(8) **`xtask` が無い時期の代わりの手順** (M0c〜M1 のタスク 0): 節は README §3.1 と手書きの `docs/spec-index.toml` (節 ID → ファイル) から引き、ファイルの該当見出しを読む。検証は `cargo fmt --check`・`cargo clippy --workspace -- -D warnings`・`cargo test --workspace` を直接実行する。(9) 開発環境: Windows (v0.1 は Windows のみ出荷。macOS / Linux 向けの実装は作らないが、Linux CI の対象 crate (core 系 + `eve-host` + `eve-testkit`、09 §6.4) に OS 依存を入れない。CI の Linux でそれらのビルドとヘッドレスのテストを行う)。リポジトリは USB 接続の外付け SSD (E:) にあるので `CARGO_TARGET_DIR` を内蔵の C: に置き、ウイルス対策ソフト (サードパーティ製) のスキャン除外にビルドの出力先を入れる (10 OQ-23 (b) / (c))。(10) 生成物 (`docs/reference/`。v0.2 からは `docs/spec-index.toml` も) は生成コマンドを通して更新し、手で書き換えない。(11) UI 文言は `ja` のみ (`locales/ja.toml`)。文言はコードに直書きせず MessageKey で引く | 仕様の要約、設計判断の説明、コード規約の長文 (clippy と `rustfmt.toml` に機械化する) |
| `AGENTS.md` (他のエージェント / 汎用、v0.2 まで) | `CLAUDE.md` と同じ内容を製品非依存の文言で。両者は **同じ生成元** (`.claude/agent-entry.md`) から `xtask` で出力し、手で二重保守しない (OQ-148。生成器を作るまで `AGENTS.md` は置かない) | 同上 |
| `.claude/settings.json` | 許可するコマンド (`cargo xtask check`、`cargo nextest run -p <crate>`、`cargo fmt` / `cargo clippy` / `cargo test`、`git` の読み取り系。v0.2 で `xtask scenario run`・`xtask spec-index` / `--show` を加える) と拒否するコマンド・書込み (`git push --force`、**`main` / `develop` への直接の `git push`** (作業ブランチだけに push する、§9.1)、`cargo publish`、`rm -rf`、**docs 本編 (`docs/*.md`) への書込み**、v0.1 の手書きの `docs/spec-index.toml` への書込み)。生成物 (`docs/reference/`。v0.2 からは `docs/spec-index.toml` も) は拒否の対象にしない (生成コマンドで再生成して PR に含める、§4.4) | 秘密情報 |
| `.claude/agents/<役割>.md` | 役割別のエージェント定義: `core-module` (eve-core の 1 モジュール担当。所有節を渡す)、`frontend-tui` (契約を越えない)、`platform-<os>`、`reviewer` (09 §8.4 のチェックリストで読む。結論を書かずに指摘だけ)、`spec-reader` (節を読んで要約を返す読み取り専用) | 役割をまたぐ権限 |
| `.claude/commands/` | `/task <Issue 番号>` (テンプレートを読み担当範囲を確認)、`/verify` (`cargo xtask check` + 関連シナリオ)、`/frame <シナリオ>` (`--print-frame` で ASCII Frame を表示)、`/spec <節>` (索引から該当節を表示)、`/oq-draft` (§4.5 の起票案の雛形) | — |
| `.claude/task-template.md` (v0.2) | §4.2 のテンプレート | — |

- `.claude/` のうち M0c で用意するのは `settings.json` だけ (M0c の全体は 08 §4)。`task-template.md` は v0.2 (v0.1 は §4.2 の形を Issue の本文に書く)。`.claude/agents/` と `.claude/commands/` は v0.2 までに必要なものから足す (ADR-0039)。
- `CLAUDE.md` は **200 行以内** を保つ。超えたら本書へ移して参照にする。
- エージェントのコンテキストに入る docs は「担当 unit の所有節 + 索引で引いた節」だけ。全文を読む指示を書かない。

---

## 6. 決定的なテストとシナリオ DSL

**v0.1 のシナリオは Rust の `Harness` (09 §3.4) で書く** (確定 (ユーザー回答)、ADR-0042、10 #38、NFR-707)。TOML のシナリオ DSL (§6.3) と `xtask scenario run` は **v0.2**。v0.1 の `Harness` は DSL と **同じ語彙** にする: 入力は `h.open` / `h.keys` / `h.text` / `h.paste` / `h.cmd` / `h.answer` / `h.advance_ms` / `h.resize` / `h.run_jobs`、検査は `query` / `fs` / `frame` / `frame_contains` / `events` / `effects` / `status` / `overlay` / `message` で、名前と意味は §6.3 の表と同じ (キーの表記は 07 §7.2 と共用のパーサ)。v0.2 では DSL をこの `Harness` の薄い写像として足し、v0.1 のテストを書き直さずに済むようにする。本節の決定性 (§6.1)・ゴールデン (§6.2)・ASCII Frame (§6.5) は v0.1 の `Harness` にもそのまま当てはまる。

### 6.1 決定性の規則

テストの決定性は 09 Q3 が原則。本書は環境の固定だけを追加する。

| 固定するもの | 方法 |
|---|---|
| 時刻 | `ManualClock` (09 §3.4)。壁時計が要る出力 (manifest・退避ディレクトリ名・ログ) はシナリオの開始時刻で固定 (v0.1 は `Harness` の設定、v0.2 の DSL は `[setup] now = "2026-01-01T00:00:00Z"`)。退避ディレクトリの時刻・pid・SID の与え方は 09 §3.4 の `MemFs` の行 |
| I/O | `MemFs` と偽 Port。実 FS を使うのは L5 (09 §3.6) だけ |
| ジョブ | `ManualJobSpawner` (投入をキューに積み、テストが `run_jobs()` で実行する、09 §3.4)。シナリオは `[[step]] run_jobs = true` で明示的に進める |
| 環境変数 | ハーネスが `EVE_CONFIG_DIR` / `EVE_STATE_DIR` を一時ディレクトリに、`TZ=UTC`、`LANG=C.UTF-8`、`NO_COLOR` 未設定、`COLORTERM` / `TERM` はシナリオの `caps` で上書き |
| 乱数 | 一時ファイル名などの乱数は `[setup] seed = 1` で固定 (eve-platform は `Paths` 経由で種を受ける) |
| 出力 | Frame JSON / ASCII から時刻・絶対パスを除く (パスはワークスペース相対に正規化、02 §10.5 の決定性) |

### 6.2 ゴールデン / スナップショットの運用

| 種類 | 置き場所 | 更新者 | 比較 |
|---|---|---|---|
| ASCII Frame (`--print-frame`) | v0.1 は `insta` の `<name>.<lang>.snap` (`Harness` のテストの横)。v0.2 の DSL はシナリオと同じディレクトリの `<name>.frame.<lang>.txt`。UI 文言を含むので言語ごとに持つ (09 §3.4。v0.1 の `lang` は `ja` だけ) | **人間** (`cargo insta accept`。エージェントは差分を PR に貼る) | 文字グリッド + スタイル記号レイヤ (§6.5) |
| Frame JSON (TUI 完成後に作る) | `tests/golden/frame/*.json` | 人間 + 契約変更要求 (02 §8.4) | `CONTRACT_VERSION` を含む構造比較。契約変更要求なしの差分は失敗 |
| 設定・キーマップ・コマンド一覧 | `docs/reference/*` | `xtask` (生成) | 09 §11.2 |
| 文字コード往復 | `testdata/encodings/*.bin` (入力) + 期待 UTF-8 (`*.utf8`) + 期待保存バイト列 (`*.saved`) (09 §4) | 人間 | バイト一致 |
| 退避のフィクスチャ | `testdata/recovery/*` (例: `format_version` を現在より大きくし未知のキーを含む manifest と本文。09 §3.4、G1c) | 人間 | 起動・終了の前後で偽 FS の内容 (パスとバイト列) が一致 |

- スナップショットの **差分レビューは意図の確認** であり、「新しい出力を正とする」運用にしない。差分が意図したものなら PR 本文にその理由 (規範行 ID) を書く。

### 6.3 シナリオ DSL の書式 (正本)

**v0.2** (ADR-0042、10 #38)。v0.1 は同じ語彙の Rust の `Harness` で書く (§6 の冒頭)。キー列・コマンド・期待状態を TOML で書く一級成果物 (NFR-707、02 §10.1 X24)。frontend を知らない (TUI でもヘッドレス参照レンダラでも同じシナリオが流れる)。

```toml
# tests/scenarios/file/save_crlf.toml
[meta]
id       = "file/save_crlf"
title    = "CRLF 文書の 1 行を置き換え、LF の貼り付けを含めて保存すると CRLF が保たれる"
verifies = ["AC-FR-113-1", "AC-FR-113-3", "03-9.3-2"]   # 01 の AC か規範行 ID
since    = "v0.1"                                 # このシナリオが通るべき最初の版
tags     = ["file", "eol"]

[setup]
size = [80, 24]                                   # 画面サイズ (列, 行)
caps = "tui_default"                              # Capabilities のプリセット: tui_default | tui_16color | tui_mono
now  = "2026-01-01T00:00:00Z"
seed = 1
config = """                                       # ユーザー層の config.toml (07 の公開キーのみ)
[file]
default_line_ending = "lf"
"""

[setup.fs]                                        # MemFs の初期内容 (パスはワークスペース相対)
"a.txt" = { text = "hello\r\nworld\r\n" }        # text = UTF-8 文字列 / bytes = base64 / file = testdata の相対パス
"big.bin" = { file = "encodings/sjis_100k.bin" }

[[step]]
open = "a.txt"                                    # 文書を開く (ジョブは次の run_jobs で完了)
[[step]]
run_jobs = true
[[step]]
keys = "ctrl+home down shift+end"                 # 07 §7.2 の設定表記。空白区切りで複数 (2 行目の "world" を選択)
[[step]]
text = "日本語"                                    # InputEvent::Text (IME 確定を含む)。選択範囲を置き換える
[[step]]
paste = "\nx"                                     # ブラケットペースト。LF は文書の改行 (CRLF) に揃う (FR-113 AC-3)
[[step]]
advance_ms = 3000                                 # ManualClock を進める (Undo 結合の境界など)
[[step]]
resize = [120, 40]
[[step]]
cmd = "file.save"                                 # CommandId (07 §11.6)。引数は args = { ... }
[[step]]
run_jobs = true

[[assert]]
at = 4                                            # 4 番目の step (text) の直後に検査 (省略時は全 step の後)
query = { kind = "selection" }
expect = { empty = true }
[[assert]]
fs = { "a.txt" = { text = "hello\r\n日本語\r\nx\r\n" } }  # 偽 FS の内容 (バイト一致。text は UTF-8 で書いた期待値)
[[assert]]
query = { kind = "document", path = "a.txt" }     # Query (02 §10.1 X18) を投げ、Answer の一部を照合
expect = { dirty = false, eol = "crlf", encoding = "utf-8" }
[[assert]]
frame = "save_crlf"                               # ASCII Frame のゴールデン (§6.5)。実行中の言語の save_crlf.frame.<lang>.txt と比較。省略時は比較しない
[[assert]]
frame_contains = ["a.txt", "CRLF"]                # ゴールデン無しの部分一致 (ステータスバーの表示など)
[[assert]]
events = ["DocumentChanged", "DocumentSaved"]     # EditorEvent の種類の列 (順序付き、部分列)
[[assert]]
no_panic = true                                   # 既定 true。明示は可
```

| 要素 | 規則 |
|---|---|
| `[meta]` | `id` はパスと一致、`verifies` は spec-index / 01 に存在する ID (xtask trace が検査)、`since` は版名 (01 §1.3) |
| `[setup]` | UI が `ja` のみの間は言語を指定しない (全シナリオを `ja` で実行、NFR-603)。英語 UI を同梱する v0.2 で `locale` (省略可。`"ja"` / `"en"`) を加え、省略時は同梱する全言語で実行する (09 §3.4)。`caps` は `eve-testkit` のプリセット名。`config` は 07 §5.0 の公開キーだけ (未公開キーは「未対応」診断になりテストの意図が隠れる) |
| `[[step]]` | **1 ステップ 1 種類の入力** (次のキーのうち 1 つだけを書く)。`keys` は空白区切りで複数のキーを書ける (表記は 07 §7.2 と共用のパーサ: 小文字、`+`、`mod` 可。`<` / `>` / `,` / `.` などの記号キーも書ける。例 `ctrl+k <`)。`cmd` は 07 §11.6 の CommandId。`text` / `paste` / `advance_ms` / `resize` / `run_jobs` / `answer` / `open` / `focus` (view id) / `signal = "hangup"` (eve-host 経由の退避テスト)。確認オーバーレイへの応答は `answer = "<選択肢の id>"` (例: 保存時に外部変更を検出した後の `"overwrite"`) |
| `[[assert]]` | `fs` / `query` + `expect` / `frame` / `frame_contains` / `events` / `effects` (`RecordingHost` が記録した Effect の列、例 `[{ kind = "Bell" }]`) / `no_panic` / `status = { ... }` (ステータス項目) / `overlay = "confirm"` (開いているオーバーレイの種類)。`at = <n>` を付けると n 番目の step の直後に検査する (途中の状態の assert。省略時は全 step の後)。`expect` のキーは `Answer` の serde 名。**言語に依存しない書き方**: メッセージは `message = { key = "msg.save.failed", level = "error" }` (MessageKey と引数で照合) で検査し、`frame_contains` には翻訳しない値 (ファイル名・`CRLF`・`UTF-8` 等) だけを書く。`frame` のゴールデンは実行中の言語のファイル (`<name>.frame.<lang>.txt`) と比べ、その言語のファイルが無ければ比較しない (英語 UI を同梱する版では主要画面は同梱する全言語のファイルを持つ、09 §3.4) |
| `query.kind` | v0.1 (`Harness`) から、シナリオの assert に要るものだけ: `documents` (文書一覧)・`document` (1 文書の状態)・`text` (テキスト範囲)・`selection`・`status`・`overlay` (02 §10.1 の `Query`。コマンド一覧とキーマップ逆引きは変種名の予約のみ、ADR-0039) |
| スキーマ | DSL の serde 型は `eve-testkit` に置き、`xtask scenario check` が全シナリオを型で検証する (未知のキー・1 step に 2 種類の入力・存在しない CommandId で失敗)。07 の表記や CommandId を変える PR はこれを通す |
| ファイル配置 | `tests/scenarios/<領域>/<name>.toml`。領域は 07 §11.6 のコマンド名前空間 (`file` / `edit` / `cursor` / `search` / `ui` / `app`) か横断 (`encoding` / `recovery` / `keymap`) |
| 実行 | `xtask scenario run [<glob>] [--print-frame] [--caps <preset>] [--frontend tui\|headless]` (`headless` = ヘッドレス参照レンダラ)。CI は `cargo nextest` から全シナリオをテストケースとして列挙する (`eve-testkit` のテストハーネスが TOML を読む) |

- **M10 の実装 (2026-10-31、最初の範囲)**: ランナーは `eve_testkit::scenario` (serde の型は `deny_unknown_fields`)、全シナリオを流す試験は `eve-testkit` の `tests/scenarios.rs` (CI の `test` ジョブで走る)。`xtask scenario run [<部分一致>] [--print-frame]` と `scenario check` (実行せずに検証だけ) はこの試験を環境変数で絞って呼ぶ。対応したもの: `[setup]` の `size`・`caps = "tui_default"`・`config`・`fs` の `text`、`[[step]]` の全種類 (`open`・`run_jobs`・`keys`・`text`・`paste`・`advance_ms`・`resize`・`cmd` + `args`)、`[[assert]]` の `at`・`fs` (`text`)・`query` + `expect` (`documents` の `count`、`document` / `text` の `dirty`・`text`・`read_only`・`encoding`・`eol`、`selection` の `empty`・`count`・`anchor`・`head`)・`frame_contains`・`events`・`overlay`・`no_panic`。**まだのもの** (書くと「未対応」で失敗する): `frame` (ゴールデン、§6.5)・`effects`・`status`・`fs` の `bytes` / `file`・`locale`・`caps` の他のプリセット・`query.kind` の `status` / `overlay`。`now`・`seed` は受け付けるが、今のハーネスは時刻 0 から始まり乱数を使わないので効果がない。パスは偽 FS の `/w` の下に置く。`xtask trace` はシナリオの `verifies` も集める (試験名は `scenario:<id>`)。最初のシナリオは `file/save_crlf`・`buffer/switch_keeps_state`・`edit/undo_groups_typing`。
- DSL は Rust の `Harness` (09 §3.4) の **薄い写像** で、DSL にできることは Harness にもできる。逆に Harness だけでできること (障害注入の細かい制御、性質テスト) は Rust で書く。
- 手動確認手順 (09 §3.7〜3.8) も v0.2 から同じ記法で書き、人が実行する。自動化できたらそのまま `tests/scenarios/` に移す (M → T の昇格)。

### 6.4 シナリオの書き順 (エージェント向け)

1. タスクの AC を読み、**期待する Frame を先に書く** (ASCII で手書きしてよい。§6.5)。
2. 初期状態と入力を書き (v0.1 は `Harness` のテスト、v0.2 は `[setup]` と `[[step]]`)、現在の ASCII Frame を出力して見る (v0.1 は `Harness` の Frame 出力、v0.2 は `xtask scenario run <path> --print-frame`)。
3. 実装 → 再実行 → Frame が期待と一致したらゴールデンとして保存を **依頼** する (PR 本文に貼る。保存は人間)。
4. 検査 (v0.2 の DSL は `[[assert]]`) に `query` / `fs` / `events` を足して、見た目以外の状態も固定する。

### 6.5 ASCII Frame の表現

`--print-frame`、駆動 API の `editor/frame{format: ascii}` (02 §10.5)、スナップショットの 3 つは **同じ表現** を使う。

```
┌─ a.txt (80x24, caps=tui_default, contract=1) ─────────────────────────────────┐
│  1 hello                                                                      │
│  2 日本語▌                                                                     │
│~                                                                              │
│                                                                               │
├───────────────────────────────────────────────────────────────────────────────┤
│ a.txt  [+]  UTF-8  CRLF  2:4                                           v0.1   │
│ ^S 保存  ^Q 終了  ^F 検索  ^P パレット                                        │
└───────────────────────────────────────────────────────────────────────────────┘
styles:
  (2,5)-(2,8) selection
  (7,1)-(7,80) status_bar
```

| 規則 | 内容 |
|---|---|
| 1 セル 1 文字 | 全角は 2 セル分の幅を占めるが、文字は 1 つだけ書く (後続セルは空にしない。表示幅はビューアに任せる)。幅危険書記素はそのまま |
| カーソル | 挿入位置の直前に `▌` を置く (セルを消費しない。`--cursor-marker none` で消せる) |
| 枠 | 画面の外側に 1 行 / 1 列の枠を付け、1 行目の枠にタイトル (文書名、サイズ、caps、`CONTRACT_VERSION`) を書く。枠はスナップショットの一部 |
| 空行 | 文書末尾より後ろの行は `~` (04 の表示と同じ) |
| スタイル | 本体の下に `styles:` として「(行, 開始列)-(行, 終了列) ロール名」を列挙 (ロール名は 07 §8.2 のスコープ)。色の値は書かない (段の選択は core のテストで別に検査、09 §3.5) |
| オーバーレイ | 画面上に重ねて描く (TUI と同じ配置)。`overlays:` に種類と選択肢 id を列挙 |
| 決定性 | 時刻・絶対パスを含めない (§6.1)。ステータスバーの時刻表示は `[setup] now` 由来 |

`xtask screenshot <シナリオ> [--step N]` (v0.2) は指定ステップ時点の ASCII Frame を出力する (`--print-frame` の途中版)。

---

## 7. ヘッドレス検証ループ

エージェントが「開く → キー → Frame を見る → 直す」を自分で回す経路は、v0.1 は (1) だけ、v0.2 は (2) のシナリオ DSL を加え、駆動 API を実装する版からは (3) を加えた 3 つ (02 §17.2、ADR-0042)。

| 経路 | 使う場面 | 入口 | 出力 | 制約 |
|---|---|---|---|---|
| (1) Rust の `Harness` (09 §3.4) | **v0.1 のタスクの完了判定の既定** (DSL と同じ語彙のシナリオテスト、§6)。性質テスト、障害注入、ジョブ順序の細かい制御 | `cargo nextest run -p <crate> <test>` | テスト結果、`assert_snapshot!` (ASCII Frame) | Rust を書く |
| (2) シナリオ DSL (v0.2) | **v0.2 からのタスクの完了判定の既定**。再現手順の共有 | `xtask scenario run <path> --print-frame` | ASCII Frame、assert の結果 | §6.3 の範囲内 |
| (3) 駆動 API (Stage D、v0.2 以降) | 対話的な探索 (「このキーを押したらどう見えるか」を 1 手ずつ)、シナリオを書く前の観察、MCP 経由の操作 (v1.L-b) | `evoeditor --headless --rpc stdio --fake-ports` (dev feature `headless`。02 §10.5) | JSON-RPC の応答 (`editor/frame{format: ascii}` で ASCII Frame) | 探索の結果は **必ず (2) のシナリオに落とす** (探索ログは成果物にしない) |

### 7.1 駆動 API の使い方 (エージェント向けの最小手順)

駆動 API は v0.2 以降の dev feature (01 NFR-707、ADR-0039)。v0.1 ではメソッド名と Frame JSON の形だけが予約されている。

```
$ cargo run -p evoeditor --features headless -- --headless --rpc stdio --fake-ports
→ {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"contract_version":1,"client":"agent:tui"}}
← {"jsonrpc":"2.0","id":1,"result":{"contract_version":1,"capabilities":{...}}}
→ {"id":2,"method":"host/fs/write","params":{"path":"a.txt","text":"hello\r\n"}}   # --fake-ports のときだけ
→ {"id":3,"method":"editor/execute","params":{"invocation":{"id":"file.open","args":{"path":"a.txt"}}}}
→ {"id":4,"method":"host/runJobs"}
→ {"id":5,"method":"editor/input","params":{"window":0,"event":{"Key":"ctrl+end"}}}
→ {"id":6,"method":"editor/frame","params":{"window":0,"format":"ascii"}}
← {"id":6,"result":{"ascii":"┌─ a.txt ...", "contract_version":1}}
→ {"id":7,"method":"editor/query","params":{"query":{"kind":"document","path":"a.txt"}}}
→ {"id":8,"method":"shutdown"}
```

- `--fake-ports` 無しで起動すると実 FS を触る。エージェントは **常に `--fake-ports`** で使う (`.claude/settings.json` で引数を固定)。保存の確認が要るときもシナリオの `fs` assert で行う。
- `--allow-write` は実 FS への保存を許す明示フラグで、エージェントの許可コマンドに含めない。
- 駆動 API の主体は External (02 §10.5)。`exec` / `net` は常に拒否されるので、外部コマンド系の検証はこの経路では行わない (L5)。
- `editor/events` 通知と `editor/frame` の ASCII は (2) と同じ形式なので、探索中に見た Frame をそのままゴールデン案に貼れる。

### 7.2 ループの回し方 (1 タスク)

1. タスク (Issue、§4.2) を読み、担当範囲と仕様節を確認する。手書きの索引で節 ID からファイルを引き、該当節だけを読む (v0.2 からは `/task <Issue>` と `xtask spec-index --show <節>`)。
2. シナリオの期待 Frame を書く (§6.4)。駆動 API がある版では、必要なら (3) で現状を観察する。
3. 実装。`cargo xtask check` (2 分車線、09 §6.1) を回す。
4. `Harness` のシナリオテストで Frame を見比べる (v0.2 からは `xtask scenario run <path> --print-frame` も)。テストに `verifies:` (AC ID) を付ける。
5. 変更した関数に `spec:` (節 ID) を付ける (v0.2 からは `xtask spec-index --check` も回す)。
6. PR を作る (§4.4)。スナップショットの差分は貼って止まる。仕様と違う判断は OQ 起票案 (§4.5)。
7. レビュー指摘 → 同じ PR で修正 → 人間が merge (§8.2)。

### 7.3 ローカル LLM を開発ループに使う (任意)

製品側がローカル LLM を一級市民にする (01 §2.9、06) のと同じ考え方で、**開発ループ側もローカルモデルで閉じられる** ようにしておく。クラウドのエージェントが使えない環境 (オフライン、秘密を含むリポジトリ) や、夜間の無人検証で安く回す用途。必須ではなく、人の作業手順は変えない。

| 場面 | 使い方 | 規則 |
|---|---|---|
| コーディングエージェントの推論先 | Claude Code 等のエージェントがローカルの OpenAI 互換サーバ (`llama-server` 等) を推論先にできる場合、その設定は **開発者ごとの未コミット設定** に置く (OQ-150)。リポジトリ側の入口 (`CLAUDE.md` / `AGENTS.md`、`xtask`、シナリオ DSL、駆動 API) はプロバイダに依存しないので、どのモデルでも同じ手順 (§7.2) で回る | モデル名・エンドポイントを docs / `.claude/settings.json` に書かない。能力の低いモデルほど **シナリオ DSL と ASCII Frame** (構造化された入出力) の価値が高い: 自由文の指示より「期待 Frame を書いて一致させる」方が収束する |
| eve-ai のタスクの検証 (v1.L) | eve-ai のタスクは **模擬 llama-server (`MockLlamaServer`、09 §3.9)** に対して検証する。実サーバを前提にしない (CI に GPU は無い)。実サーバでの確認が要るタスク (能力検出・テンプレート・FIM トークン) は、人間が `xtask ai-record` でカセットを記録し、エージェントは再生で回す | カセットの記録は **人間だけ** (06 §16。実サーバの起動・モデルの取得はネットワークと GPU を使う)。エージェントの許可コマンドに `ai-record` を含めない |
| ヘッドレスのエージェント + ローカルモデルによる無人検証 | 夜間に、ローカルモデルを推論先にしたエージェントが `evoeditor --headless --rpc stdio --fake-ports` に対して **探索的なシナリオ** (ランダムな操作列 + 不変条件の確認。fuzz_editor と同じ性質を自然言語の仮説で広げる) を回し、見つけた再現手順を **シナリオ DSL の案** として Issue に出す。docs や本体コードは直接直さない (§4.5) | 成果物はシナリオ案と Frame のみ。`--allow-write` を与えない。1 晩の予算 (時間・トークン) を `xtask` の引数で固定し、結果は `agent-pr` と同じ扱いで人間が読む |
| 開発環境の起動 | `xtask ai-mock up [--cassette <file>] [--latency <ms>] [--seed <n>]` で模擬サーバを起動し、`ai.providers.<name>.endpoint` をそれに向けた開発者設定でエディタを動かす。管理モード (FR-918) の開発では、模擬サーバの実行ファイルを `managed.command` に指定して子プロセス管理の経路を実サーバなしで通す | 模擬サーバはループバックにしか bind しない (製品と同じ NFR-509) |

- 製品の方針 (「既定で何も送らない」、ループバック既定、除外・伏字) は開発ループにも適用する: エージェントがリポジトリ外のファイルや秘密を推論先に送らないよう、ワークスペース設定の `ai.exclude` と同じパターン集を `.claude/` の無視設定にも写す (`xtask` で生成、二重保守しない)。
- ローカルモデルは応答が遅く長い: 速い車線 (2 分、§8.1) の時間目標はエージェントの推論時間を含めない (検証時間だけを測る)。

---

## 8. コードレビューと CI の運用

### 8.1 CI の車線

| 車線 | 内容 | 時間目標 | いつ |
|---|---|---|---|
| **速い車線** `cargo xtask check` | **v0.1 の中身 (09 §6.1・08 §2.2 と同じ。この 3 か所を同時に直す)**: (1) fmt、(2) clippy、(3) `nextest -p eve-text -p eve-encoding -p eve-core`、(4) `check-deps`、(5) `tests/ac-checklist.toml` の軽い検査 (§3.3)、(6) 変更した crate の `Harness` のシナリオテスト。v0.2 からは (5) の後に `spec-index --check`・`check-docs`・`trace --check` (M10 で入れた) と、変更 crate のラベルで選ぶ DSL のシナリオ | 2 分 | ローカル (エージェントの各反復。開発環境は Windows) と PR の最初のジョブ (`ci.yml` の `check` ジョブ、Windows、09 §6.2) |
| **フル車線** `ci.yml` | 09 §6.2 のジョブ。v0.1 は PR / main とも **Windows x86_64** (全テスト・ConPTY の E2E 10 シナリオ) + **Linux x86_64** (Linux CI の対象 crate (core 系 + `eve-host` + `eve-testkit`) のビルドとヘッドレスのテスト。OS 非依存を守る)、依存 (`cargo deny`・`check-deps`)・`i18n` (`ja` とコードの MessageKey のキー集合・ヒント幅)・`check-themes`・`docs-tables` (docs の表と実装の突合、§8.5)・`dco`・`gitleaks`・`pr-source` (PR の head ブランチの検査)。`docs.yml` (リンク検査・生成リファレンスの最新性) も同じ宛先で走る。パスのフィルタを付けず、コードだけ・docs だけの PR でも全ジョブを回す。**v0.2**: macOS の CI、Tier B のビルド、補助ジョブ (msrv・min-features・cargo-hack・udeps・outdated・docs-rs・geiger・`area-check`)。Frame JSON ゴールデンは TUI 完成後 | 15 分 | PR / main |
| 夜間 | 09 §6.3。v0.1 は fuzz (`fuzz_decode` のみ。M3 で導入。クラッシュ入力は `age` で暗号化した artifact)、proptest 拡張、`cargo audit` (`fuzz/` を含む)、カバレッジ報告。対象のブランチは変数 `NIGHTLY_REF` (既定 `develop`。凍結中も `develop` のまま、09 §6.1)。1 時間の soak・`docs-rs`・`fuzz_editor`・クラッシュ一貫性は v0.2 | — | スケジュール |
| ベンチ | 09 §5。v0.1 は判定に使うベンチ (09 §5.2 の「v0.1 で判定に使うベンチ」: NFR-101〜106 / 201〜204 と BM-103b の負荷下の判定) をリリース前に基準機 (Windows) で測る。`bench.yml` は値の記録 (BM-101a・BM-103a・BM-205。v0.1 は Actions の artifact に置き、`gh-pages` は v0.2) で、命令数と A/B の CI は v0.2 (M10 で `perf.yml` を入れた。09 §5.4)。基準機での計測の手順と記録の形式は §1 の `docs/perf/` の行 | 20 分 | `bench.yml` は手動 (v0.2 から `develop` への push でも)。トリガの正本は 09 §6.1 |

- 速い車線が赤のままフル車線を回さない (ランナー費用)。エージェントは速い車線が緑になるまで PR を draft にする。
- ラベル `agent-pr` の PR は、フル車線の前に `reviewer` エージェント (§5。v0.2 まで) のチェックリストレビューを自動で走らせてよい (人間のレビューの代わりではない)。

### 8.2 レビューの規則

| 規則 | 内容 |
|---|---|
| 人間が merge | エージェントの PR も、人間の PR も、**merge は人間**。単独開発期間はセルフレビュー + チェックリスト (09 §8.1)。単独開発期間のブランチ保護は **承認必須 0・管理者のバイパスなし・必須のステータスチェックだけで merge を止める** (§9.1) |
| レビュアの読み方 | PR 本文の仕様節 (v0.2 からは規範行 ID) → 該当節 (手書きの索引で引く。v0.2 からは `xtask spec-index --show`) → 実装の `spec:` → テストの `verifies:` の順に読む。仕様に書いていない振る舞いがあれば §4.5 |
| 09 §8.4 のチェックリスト | 境界、変更経路、エラー、秘匿、unsafe、依存、性能、安定性 (MR1〜MR11)、仕様 ID、AI (v1.L)、契約、テスト、仕様 |
| エージェントへの差し戻し | 指摘は「どの規範行に反するか」「どのシナリオで再現するか」で書く (自然文の好みではなく ID で)。差し戻しは同じ PR で |
| 設計判断の持ち帰り | レビューで設計の疑義が出たら、PR で議論せず OQ 起票 (§4.5)。PR は一時停止 |
| 所有者の承認 | v0.1 は単独開発なので core の所有者 (人間) が承認する。v0.2 からは `docs/ownership.toml` の `reviewer` (または `owner` が人間ならその人) の承認を必須にし、CODEOWNERS はこれを機械化したもの |
| UI 文言の追加・変更 (文言の運用、NFR-603) | v0.1 の UI 文言は **`ja` のみ** (確定 (ユーザー回答)、ADR-0042、OQ-21)。文言 (MessageKey) を足す・変える PR は **コードの MessageKey と `locales/ja.toml` を同じ PR で** 更新する (キー集合の欠落・余剰は 09 §6.2 の `i18n` ジョブで失敗する)。**`ja` の文言の人手レビューの観点**: (1) 用語の統一 (04 §15.1 の用語と既存の文言。例「保存」「文字コード」「改行コード」「読み取り専用」)、(2) NFR-602 の 2 要素 (`text` = 何が起きたか、`hint` = どうすればよいか) が揃っている、(3) 長さ (ヒントは 8 セル以内、ステータスバー・メッセージ行で省略されても意味が残る)、(4) キー表記を直書きせず `{key:<command_id>}` を使う、(5) プレースホルダの過不足が無い、(6) 識別子 (CommandId・設定キー・CLI オプション) を日本語にしていない。文言の自然さは機械で検査しないので人が見る (単独開発期間はセルフレビューのチェックリストに入れる)。エージェントが書いた文言も同じ。英語 UI を同梱する v0.2 からは、`en` と `ja` を同じ PR で更新し、訳のレビュー (両言語の用語・2 要素の対応) を加える |

### 8.3 ラベル

| ラベル | 意味 | 効果 |
|---|---|---|
| `crate:<name>` / `spec:<節>` / `ac:<ID>` | 対象 (v0.1 は手で付ける) | 検索と絞り込み。速い車線のシナリオ選択・トレース・ラベルの自動付与は v0.2 |
| `agent-ok` (Issue) | エージェントに委譲してよい | `/task` で選べる |
| `agent-pr` (PR) | エージェントが作った PR | `reviewer` エージェントの事前レビュー、人間の承認必須 |
| `area:<frontend>` (PR、v0.2) | frontend crate の PR (`area:tui` 等) | 09 §6.2 `area-check` (v0.2) が `eve-core` / `eve-view` / `eve-config` の変更を拒否する (02 §4.3 R9)。v0.1 はレビューで確かめる |
| `contract-change` (PR) | `eve-view` を変更する | §8.4 の手続き。v0.1 は記録のため手で付ける (`area-check` による要求は v0.2。M10 で `area.yml` を入れた: このラベルの無い PR が `crates/eve-view/` を変えると失敗する) |
| `spec-change` (PR) | docs の **本編** (`docs/*.md`。`00`〜`12` と README) を変更する (人間のみ)。`docs/compat/`・`docs/perf/`・`docs/reference/` (記録と生成物) と `tests/ac-checklist.toml` だけの変更は対象外 | OQ / ADR の参照必須 (00 §7.1) |
| `changelog: none` | ユーザー可視の変更なし | 09 §8.1 の CHANGELOG ゲートを免除 |
| `needs-oq` | 仕様の疑義あり。PR 停止中 | §4.5 |
| `snapshot-review` | スナップショット差分の確認待ち | 人間が `accept` するまで merge 不可 |
| `severity:critical` (Issue) | 08 §8.1 の SB-1〜SB-4 (データ損失・クラッシュとハング・セキュリティ・保存の失敗) に当たり、再現が確認された不具合 | 人間が付ける (エージェントは付けない)。TUI 完成の判定 (08 §8.1 TC-2) で未解決の件数を数える。セキュリティ (SB-3) は §9.8 の非公開の手順で扱い、公開の後に付ける |

### 8.4 契約変更要求の手続き (02 §8.4 の運用面)

境界型 (`eve-view`) の変更は「契約変更要求」として扱う。規則の内容 (段階 C0〜C3、許す変更) は 02 §8.4 が正本で、ここは手順だけ。

**v0.1 の手続き** (確定 (ユーザー回答)、ADR-0042、10 #43): 外部の消費者は TUI 完成まで存在しないので、Issue テンプレート「契約変更要求」の起票・Frame JSON ゴールデンは **TUI 完成後に作る**。v0.1 では、core の所有者 (人間) だけが `eve-view` を変更し、**同じ PR で全消費者 (eve-tui・`HeadlessFrontend`・テストの ASCII Frame のスナップショット) を更新** する。PR には `contract-change` ラベルを手で付け、本文に変更する型・理由・`CONTRACT_VERSION` の増分を書く。段階ごとに許す変更 (02 §8.4。C2 以降は追加のみ) は v0.1 でも守る。下の表の Issue の起票と Frame JSON ゴールデンの更新は TUI 完成後の手続き。

| 段階 | 手続き |
|---|---|
| C0 (M1 まで) | core の所有者が自由に変更。PR に `contract-change` ラベルだけ付ける (記録のため) |
| C1 (M2。二重消費の相手は TUI + ヘッドレスの Frame 消費者 + テスト用 Frontend (`HeadlessFrontend`)。外部の消費者は無い) | v0.1 は上の手続き。TUI 完成後は Issue テンプレート「契約変更要求」(変更する型、理由、全消費者 (eve-tui・ASCII Frame・Frame JSON ゴールデン・`HeadlessFrontend`。駆動 API を実装した版からはその JSON Frame も) への影響、`CONTRACT_VERSION` の増分) を 1 件起票する。どちらも core の所有者が `eve-view` を変更し、**同じ PR で全消費者を更新** → `CONTRACT_VERSION` +1、`docs/contract-changes.md` に 1 行。型の不足は M2 の設計レビュー (02 §8.4 の C1 候補表 + 02 §16) で先に潰す |
| C2 (M2 の完了時〜) | 追加のみ。削除・意味変更は RFC (Issue に `rfc` ラベル、該当 crate の担当の合意) → ADR または 02 §8.4 の改訂を伴う |
| C3 (契約版 1 の固定は TUI 完成後のレビューで行う、02 §8.4。v0.1 は C2 のまま) | C2 と同じ + 契約版 +1 を立てる場合は 02 §8.4 の並行期間。**外部の消費者 (駆動 API のクライアント、プラグイン作者) で見つかった不足は「追加 (Option / Default 付き、`#[non_exhaustive]` 型への変種)」で入れるのを既定とし、追加で表せないときだけ契約版 +1** (直近 2 版を受理)。手続きは「**公開リポジトリの消費者 (TUI とヘッドレス参照レンダラ) を同一 PR で更新し、外部の消費者は契約版の並行期間内に追従する**」(ADR-0030 決定 3 を外部の消費者に当てはめたもの、ADR-0038 決定 4)。外部の消費者からの要求は公開リポジトリの Issue として起票し (§9.5)、公開 crate の新版 (タグ) として出してから外部の消費者が取り込む |

- frontend 担当 (人間・エージェント) は `eve-view` を自分の PR で変更しない (09 §7 R9。機械検査の `area-check` は v0.2、v0.1 はレビューで確かめる)。必要なら契約変更要求を起票して待つ。待つ間は未知の変種を既定描画 (`_ =>`) で受ける (02 §8.4)。

### 8.5 docs の表を CI の入力にする規則 (機械可読の表)

v0.1 の CI は、一部の docs の表を **実装との突合の入力** として読む (09 §6.2)。表を読む専用の `xtask` のサブコマンドは置かず、v0.1 の `xtask check-themes` / `xtask i18n` / `xtask check-deps` / `xtask check` (AC ID、§3.3) と eve-core・eve-tui のテスト (カタログ・横取り表・頻出操作) が、それぞれ下の表を直接読む。読み手は同じ規則で読む: パーサは crate **`eve-docs-tables`** (`publish = false`、§1) に 1 本化し、その利用者は **`xtask`** と、**eve-tui・eve-core のテスト (dev 依存)** である。規則どおりに読めなければ **失敗** する (黙って読み飛ばさない)。

| 表 | 文書と見出し (固定) | 列 (名前と順を固定) | 読む検査 (ジョブ) |
|---|---|---|---|
| ロール表 | 07 §8.2 (見出し `### 8.2 書式`) | `ロール` / `用途` / `形` / `truecolor (fg / bg)` / `256 (fg / bg、参考)` / `16 (fg / bg、参考)` / `mono` | `xtask check-themes` (`check-themes`) |
| 端末の横取り表 | 04 §7.3a (見出し `### 7.3a 端末の横取り表 (FR-606 の到達性検査の入力)`) | `端末` / `横取りするキー` / `別のキーとして届くもの` / `確認` | eve-tui の到達性検査のテスト (`docs-tables`) |
| MessageKey 目録 | 04 §15.2 (見出し `### 15.2 保存・読込・退避・パニックの文言目録`) の各表 | `MessageKey` / `出す場面` / `ja` の 3 列。「起動前・終了時の stderr」と「パニック」の表だけは `MessageKey` / `出す場面` / `en` / `ja` の 4 列 (`en` は v0.2 から出す英語の行。04 §15.2 の凡例) (列名を固定。読む側は列名で `MessageKey` と `ja` の列を見つけ、`en` 列の有無は表ごとに受け入れる) | `xtask i18n` (`i18n`) |
| コマンドカタログ | 07 §11.6 (見出し `### 11.6 v0.1 CommandId カタログ (付録)`) の各表 | `ID` / `title key` / `kind` / `args` / `when` / `perm` / `コンテキスト` / `既定キー` / `ヒント` | `xtask i18n` (ヒント列・`cmd.*`)、eve-core のテスト (`perm` 列と `CommandSpec.permission`。M1 から)、eve-tui の統合テストと bin の `--list-commands` / `--dump-keymap` との突合 (M2 から) (`docs-tables`) |
| 頻出操作リスト (FR-606) | 07 §11.6 の中の、`**FR-606 の到達性検査**` で始まる段落の直後の箇条書き (見出しではなく、この段落の書き出しを固定する) | 表ではなく箇条書き。各行は `- <分類>: ` に続けて CommandId をバッククォートで囲んで `, ` で区切る (07 の v0.1 では 37 個) | eve-tui の到達性検査のテスト (`docs-tables`、09 §3.4) |
| 受入基準 (AC) 表 | 01 §5.1〜§5.4 (見出し `### 5.1 v0.1: ファイル・文書管理` / `### 5.2 v0.1: 編集・移動・検索` / `### 5.3 v0.1: 表示・入力・設定` / `### 5.4 v0.1: 非機能`) の各表 | `要件` / `受入基準` | `xtask check` (AC のチェックリストの検査、§3.3) |
| crate 一覧 | 02 §4.1 (見出し `### 4.1 crate 一覧`) | `crate` / `種別` / `版` / `責務` / `主な外部依存 (案)` / `禁止事項` / `禁止 crate` (`禁止 crate` 列を `deps-policy.toml` と突合する。`禁止事項` 列は説明の文で、突合しない) | `xtask check-deps --docs` (`docs-tables`) |

- **見出しと列を固定する**: 上の見出しの文言、列の名前と順を変えない。変える必要があるときは、表を読む側のコード (パーサの設定) を **同じ PR** で直す (人間の `spec-change` の PR。エージェントは §4.5 の起票案)。列の追加は表の末尾にだけ行い、読む側は知らない列を無視する。
- **1 行 1 項目を基本にする**: 1 行に 1 つのロール・端末・キー・コマンド・crate を書く。セルの中で改行しない (`<br>` を使わない)。
- **識別子はバッククォートで囲む**: ロール名・MessageKey・CommandId・キー表記・crate 名はセルの中で `` `…` `` で囲み、読む側はバッククォートの中だけを識別子として取り出す。説明の文や括弧書きは識別子として読まない。**1 行 (1 セル) に複数の ID を書く場合は、各 ID を完全な形で 1 つずつバッククォートで囲む** (例 `` `select.extend_to_doc_start` `` / `` `select.extend_to_doc_end` ``)。**略記** (`` `select.extend_to_doc_start` / `_end` `` の `_end` のような後半だけの表記) と **ワイルドカード** (`` `ui.confirm.*` ``) は識別子の列に書かない (読む側は 1 つのバッククォートを 1 つの完全な ID として扱い、展開しない)。
- **版の注記**: v0.1 に無い行は、識別子の列のセルに `(v0.2)` などの版の注記を付ける (読む側はその版の行として扱い、v0.1 の必須の検査から外す。実装した版から登録してよく、登録済みのコマンドの照合 (`eve-core` の `perm_catalog`) は注記付きの行も対象にし、言語ファイルの `cmd.*` / `hint.*` は注記付きの行ではあっても余剰とせず無くてもよい)。注記の書き方は 07 §11.6 の凡例に合わせる。**例外**: 02 §4.1 の crate 一覧は、行の版を識別子の列 (`crate`) の注記ではなく **「版」列** で判定する (`xtask check-deps --docs` は「版」列の先頭の版 (`v0.1` 等。続く括弧の注記は読まない) で v0.1 の行を選ぶ。`crate` 列には版の注記を付けず、付いていても版の判定に使わない)。
- **表の外に規範を書かない**: 読む側が見るのは表だけなので、検査に効く値 (色・キー・既定) を表の前後の文章だけに書かない (例外は上の表の「頻出操作リスト」の箇条書きで、書き出しの段落を固定して読む)。
- **AC の抽出規則 (01 §5 の AC 表)**: (1) `要件` 列の要件 ID (`FR-105`・`NFR-302` 等。括弧の注記 `(保存直前の照合)` 等は読まない) と、`受入基準` 列のうち **`AC-<n>:` で始まる部分だけ** を取り出す。`AC-<n>` と `:` の間には括弧の注記 (`AC-6 (ダウングレード):`) が入ってよく、注記の括弧の中のコロンは区切りとみなさない。次の AC の始まりの直前またはセルの終わりまでが 1 つの AC。`AC-<n>` の後に `:` が無い書き方 (「AC-1 と同じ」「AC-2 (全保存) は §5.5 (v0.2)」等) は AC の始まりとして数えない。AC ID は `AC-<要件 ID>-<n>` (`AC-FR-105-1`) とする。(2) **範囲の行** (`NFR-101〜105` のように `〜` で要件を束ねた行) は、範囲に含まれる要件ごとに展開し、各要件が同じ AC (`AC-NFR-101-1` 〜 `AC-NFR-105-1`) を持つものとする。(3) **枝番** (`(a)` / `(b)` 等、AC の中の小項目) は分けずに、その `AC-<n>` に束ねる (チェックリストの 1 項目)。(4) **除外**: AC の頭部 (`AC-<n>` から `:` まで) の注記に版の注記 (`(v0.2)`・`(参考、v0.2 から)` 等) や後の版からの適用 (「…を実装した版から適用」) を含む AC は、v0.1 の AC から除く。`は §5.5` の形 (後の版の §5.5 に回したことを示す書き方) は (1) で AC の始まりにならないので、それ自体が除外になる。AC の本文の括弧書きの中の言及 (「(… の AC は §5.5、v0.2)」のように別の AC を後の版に回したことの説明) では、その AC を除外しない。(5) 読めない行 (要件 ID が無い・`AC-<n>:` が 1 つも無い Must の行) は失敗にする。
- **07 §11.6 の既定キー列・ヒント列・コンテキスト別の記法** (文法の正本は 07 §11.6 の凡例。本書は読む側の要約で、食い違えば 07 を正とする): 既定キー列は、バッククォートで囲んだキー 1 つの直後に属性 `[E]`・`[横]`・`(alt)` が空白なしで続き (`[E][横]` の順)、割当の間を `, ` で区切る。対 (` / ` でつないだ複数 ID) の最後のキーの直後にだけ属性があるときは対のすべてのキーに及び、途中のキーにも属性があるときは各キーが自分の直後の属性だけを持つ。片方の ID だけの割当には `(<ID の最後の語> のみ)` が付く (例 `(redo のみ)`。対の位置では数えない)。コンテキストごとに割当が違う行は `` `<コンテキスト>`: `` で始まる組を `、` で区切る (例 `` `list`: `down` / `up`, `ctrl+n` / `ctrl+p` ``。`` `list`: `` の後ろの ` / ` は複数 ID の行の左右の対応)。`(alt)` 以外の括弧は注記で、中のバッククォートを識別子として読まない。ヒント列は `<文脈>:<優先度>` を `, ` で区切り、直後の `(<ID の最後の語> のみ)` で複数 ID の一部に限る。`-` と `—` は「無い」。
- **07 §8.2 のロール表の値の記法** (`xtask check-themes` が読む。記法の正本は 07 §8.2 の凡例で、本書は読む側の要約。食い違えば 07 を正とする): `ロール` 列はバッククォートで囲んだロール名 1 つ。`形` 列は **1 ロールに 1 値** (`(i)` / `(ii)` / `(iii)` のどれか 1 つ。07 §8.1) で、truecolor の段の形を表す (`check-themes` は truecolor の列をこの形で検査する。256 / 16 の参考値で形が違うものは 07 §8.2 の表の下の注記に書き、`形` 列には書かない)。色の段の列 (`truecolor (fg / bg)` / `256 (fg / bg、参考)` / `16 (fg / bg、参考)`) の各セルは、(a) 色の組 `` `<前景>` / `<背景>` `` (各色はバッククォートで囲み、`default` は端末既定)、(b) (a) の後ろに ` + ` と属性を続けたもの、(c) 属性だけ (07 §8.2 の凡例 (3)。**形 (ii) のロール** に適用する)、(d) `` `none` `` (その段では強調しない。色も属性も付けない。**色の列でも有効**)、のどれか 1 つ。色の書き方は段で違い、truecolor は `#rrggbb` 等、256 は `ansi.<番号>`、**16 の列は `ansi.` を省いて色名だけ** を書く (`bright_black`・`blue` 等。`ansi.blue` とは書かない)。`mono` 列は属性か `` `none` `` だけ。属性は 07 §8.2 の属性語彙 (`bold` / `underline` / `reverse` / `dim`) を 1 語ずつバッククォートで囲んで ` + ` で結合して書く (例 `` `#1a1a1a` / `#e6c84a` + `underline` ``、`` `reverse` + `underline` ``)。`none` は他の語と結合しない。空のセルは失敗にする。括弧の中 (コントラスト比などの注記) は読まない。語彙に無い属性の語・`+` の前後の欠けた式は失敗にする。
- **02 §4.1 の crate 一覧の `禁止 crate` 列**: `xtask check-deps --docs` が `deps-policy.toml` の各 crate の禁止リストと突合する列。セルには禁止する crate を **完全な crate 名** で 1 つずつバッククォートで囲み、`, ` で区切って列挙する (例 `` `ratatui`, `crossterm`, `tokio` ``)。機能の注記 (「`tokio` の net 機能」など) やワイルドカード・略記は書かず、無いときは `—`。括弧の中は読まない。説明の文は `禁止事項` 列に書き、突合には使わない。
- 規則に合わない表の変更は `docs-tables` / `check-themes` / `i18n` の各ジョブが失敗して知らせる。これらのジョブはコードだけを変える PR でも走る (09 §6.2) ので、実装だけを変えて docs の表と食い違った PR も止まる。

---

## 9. ブランチ・コミット・リリースの規約

### 9.1 ブランチ

**確定 (ユーザー回答、[OQ-167](10-decisions.md#oq-167-ブランチと公開の運用) / [OQ-168](10-decisions.md#oq-168-開発期間中のリポジトリの公開状態))**: 公開リポジトリは public のまま開発し、docs/ も最初の push から公開する。長命ブランチは **`main` (リリース専用) と `develop` (開発の本線) の 2 本だけ**。`main` にはリリースした版だけが入り、`main` の docs/ がリリースした版の仕様になる。作業ブランチは `develop` から作り、PR (squash) で `develop` に入れる。版ごとの開発の線 (`dev/<版>`)・`release/*`・`hotfix/**` は作らない。リリースは 2 種類で、**機能リリース** は `0.y.0` 単位 (v0.1.0、v0.2.0 …)、**bugfix リリース** は **パッチ版** (`0.y.z` の z。z は 10 以上も可) で出す。「パッチ版」は `0.y.z` の z を指す用語で、v0.1.0 のリリースまでの内部カウンタ `0.0.x` (下の「版番号」) とは別物である。下の表と箇条のうち、保護の設定・`0.0.x` の番号の上げ方・既定ブランチ (OQ-170) は **提案中** (OQ-167 の細目)。

| ブランチ | 用途 | 規則 |
|---|---|---|
| `main` | リリース専用。リリースした版だけが入り、各リリースのマージコミットにタグ (`v0.1.0` 等) が付く。v0.1.0 のリリースから既定ブランチ (2026-10-27 に戻した。訪問者が見るのはリリースした版の README と docs)。それまでは初期コミット (`LICENSE`・`README`・`.gitignore`) のまま変えない | 直接 push・force push・削除の禁止。PR は `develop` (リリース) からだけ受け、merge は人間 (merge commit)。タグは人間が署名して付ける。タグ `v*` の作成と削除はメンテナに限る。**プレリリースのタグ** (`-alpha.N` / `-beta.N` / `-rc.N`) は `main` ではなく `develop` の先頭のコミットに付ける (09 §9.1) |
| `develop` | 開発の本線。作業ブランチの取り込み先で、リリース (機能リリースもパッチ版も) の起点になる。マイルストーン (08 §2.2 の M1〜M5) の成果もここに入る。v0.1.0 のリリースまでは既定ブランチだった ([OQ-170](10-decisions.md#28-oq-161-以降-仕上げ段階の追加起票)。2026-10-27 に `main` に戻した)。Dependabot のバージョン更新の宛先 (`target-branch: develop`、09 §6.3)。リリース後の版は直近のリリース版のまま (`-dev` 付きの版は使わない)。凍結 (リリース候補の期間) もこのブランチで行う | 直接 push・force push・削除の禁止。PR は作業ブランチ (squash)・Dependabot (`dependabot/**`、squash)・`main` (リリース後の取り込み、merge commit) からだけ受け、必須の CI を通す |
| `<種別>/<crate>-<短い説明>` | 作業ブランチ。種別は `feat` / `fix` / `spec` (docs) / `chore` / `spike`。`develop` から作り、PR の宛先は `develop` | 例: `feat/eve-core-linear-undo`、`spec/04-color-depth`。エージェントは `agent/<名前>/<Issue 番号>-<短い説明>`。1 タスク = 1 ブランチ = 1 PR = `develop` 上の 1 コミットで、merge 後に消す |
| `spike/S-nn` | スパイク (08 §3) | `spikes/S-nn/` 以下のみ。merge せず `REPORT.md` だけを `spec/` ブランチで取り込む |

**流れ**: 作業ブランチ → PR (squash) → `develop` → リリース時に PR (merge commit) → `main` → 人間がマージコミットに署名付きタグ (`v0.1.0` 等) を付けて GitHub Releases に公開 (09 §9) → `main` を `develop` に取り込む (PR、merge commit)。リリースのたびに `develop` → `main` の 1 経路だけで、パッチ版も同じ経路を通る。

- **版番号** (提案中、OQ-167 の細目): v0.1.0 のリリースまでの workspace の版は内部カウンタ `0.0.x` で、`x` はマイルストーン (M1〜M5) を `develop` に取り込む PR (`chore/…` の PR、squash) で 1 つ上げる (M1 の取り込みで `0.0.1`)。`0.0.x` にはタグを付けず、GitHub Releases も作らない。これは **パッチ版 (`0.y.z` の z) ではない**。リリース PR (`develop` に対する `chore/…` の PR、§9.3) で workspace の版と CHANGELOG の Unreleased を `0.y.z` (v0.1 なら `0.1.0`) に直し、`develop` → `main` の順に取り込んで `main` のマージコミットにタグを付ける。`-dev` 付きの版は使わない: リリースの後も `develop` の版は直近のリリース版のままで、次のリリース PR で更新する。プレリリース (`-alpha.N` / `-beta.N` / `-rc.N`) を出すときは、`develop` に workspace の版を `0.1.0-rc.N` 等にする PR を入れ、その時点の `develop` の先頭のコミットにプレリリースのタグを付ける (09 §9.2 手順 1a。任意)。v0.1.0 のリリースで既定ブランチを `main` に切り替える (OQ-170)。タグと版の比較は **数値順** (`git tag --sort=version:refname`。文字列順にしない。`v0.1.10` は `v0.1.9` より新しい)。
- **パッチ版 (bugfix リリース)** (経路は提案中、OQ-167 の細目): 経路は 1 つだけ: `fix/…` → `develop` → `main` (PR、merge commit、人間) で、`main` のマージコミットに署名付きタグ `v0.1.1` 等を付ける。**パッチ版を出せる条件**: 前回のタグ以降の `develop` に `feat` (機能追加) が 1 件も入っていない (`fix` / `chore` / `spec` だけ)。`feat` が既に入っているときは、バグ修正を次の機能リリースに含めて出す (または機能リリースのゲートを早める)。そのため、**機能リリースの直後からパッチ版を出し終えるまでは `feat` の PR の merge を後回し** にし、`fix` を先に入れてパッチ版を切ってから `feat` を入れる。タグを付けたら `main` を `develop` に取り込む (PR、merge commit)。作業ブランチから `main` に直接修正を入れない。**残余リスク**: 条件を満たすまで (`feat` が入った後の緊急の修正など) 修正の公開が遅れうる。必要になったら `main` のタグから切る緊急用の経路を足す余地はあるが、現時点では採らない。
- **docs の変更**: 仕様の変更 (`spec/` ブランチ) も `develop` に入れる。`develop` の docs/ は最初の push から作業中の版として公開され、リリースで `main` の docs/ (リリースした版の仕様) になる (README に書く、12 §5.6 の PC-3)。リリースの後に見つかった docs の誤り (版の内容を変えない誤記) もパッチ版と同じ経路で直す。
- **CI**: PR (宛先が `develop`・`main`) と、これらへの push でフル車線を回す。GitHub Actions は M1 のタスク (0) で CI を置いたときから有効にする (public のリポジトリでは標準のランナーが無料。12 §5.6 の PC-5)。夜間のジョブ (`schedule`) は既定ブランチのワークフロー定義で動く (v0.1.0 のリリースまでは `develop`、以後は `main`。OQ-170) ので、既定ブランチ (`main` を含む) には `nightly.yml` / `bench.yml` の **薄いトリガ** だけを置き、実体はリポジトリの変数 `NIGHTLY_REF` (既定 `develop`) の ref でチェックアウトしたコードの `cargo xtask nightly` / `cargo xtask bench` で実行する。リリース候補の期間 (凍結から publish まで) も対象は `develop` のままで、`NIGHTLY_REF` を切り替えない (リリースゲートの夜間 fuzz の 5 日 (v0.5.0 以降は 7 日) の観測は `develop` を対象にした実行で数える、09 §8.2)。トリガの正本は 09 §6.1。
- 長命ブランチは `main` と `develop` だけ。`gh-pages` は CI が生成物 (ベンチのグラフ・API 文書、09 §6.1 / §11.2) を置くだけのブランチで、開発の線ではない。それ以外は 1 タスク (§4.1) = 1 ブランチ = 1 PR で、merge 後に消す。リポジトリ直下の `CLAUDE.md` (中身は別途用意) に、この運用の要約 (`develop` から作業ブランチを切る、`main` / `develop` に直接 push しない、PR の宛先は `develop`) を置き、エージェントに伝える。
- rebase は作業ブランチ内で自由。保護したブランチ (`main`・`develop`) への force push は禁止 (GitHub のブランチ保護と `.claude/settings.json` の両方で拒否)。`main` / `develop` への直接の `git push` も `.claude/settings.json` で拒否する (§5)。
- **ブランチ保護の設定** (`main`・`develop`。提案中、OQ-167 の細目): PR 必須、直接 push・force push・削除の禁止、**必須のステータスチェック** (09 §6.2 の `ci.yml` のジョブ。**存在するジョブだけを必須にする**: M1 のタスク (0) で CI を置いた時点で存在するジョブだけを加え、ジョブを足す PR が merge されたら同じ担当者がそのジョブを必須に加える (手順は 09 §6.2 の「必須のステータスチェックの段階的な追加」))、**PR の出どころの検査** (GitHub のブランチ保護は「どのブランチからの PR を受けるか」を表せないので、`ci.yml` の `pr-source` ジョブ (09 §6.2) が head ブランチを検査し、これを必須のチェックにする: `main` は `develop` からだけ、`develop` は作業ブランチ (下の表の `<種別>/…` と `agent/…`)・Dependabot (`dependabot/**`)・`main` (リリース後の取り込み) と、fork からの PR (外部の寄稿者) から。規則の正本は 09 §6.2 の `pr-source`)、**管理者のバイパスなし** (メンテナ自身にも規則を適用する)。**単独開発の期間は「承認必須」を 0 にする** (GitHub では PR の作者が自分の PR を承認できないため。代わりに 09 §8.1 のセルフレビューのチェックリストを PR に貼る)。2 人目のメンテナ (またはレビュア) が加わったら承認必須を 1 にする。
- **必須のステータスチェックの名前の固定**: 必須にするチェックはジョブの表示名で照合されるので、`ci.yml` の各ジョブに明示の `name:` を付け、マトリクスのジョブも `name: test (windows)` / `name: test (linux)` のように **OS の役割だけを含む固定の名前** にする (ランナーのラベル (`windows-2025` 等) や Rust の版を名前に入れない。ランナーを替えても必須のチェックの設定を直さずに済む)。**最初の必須セットの固定名** (09 §6.2 と一致させる): `check` / `fmt` / `clippy (windows)` / `clippy (linux)` / `test (windows)` / `test (linux)` / `deps` / `dco` / `gitleaks` / `pr-source`。2 段目 (`e2e`・`build-release-smoke`・`docs-tables`・`check-themes`・`i18n`・`docs.yml` の検査) はジョブを足す PR の merge の後に加える (09 §6.2)。ジョブ名を変える PR ではブランチ保護の設定も同時に直す (メンテナ)。`dco` のように条件で飛ばすジョブは、飛ばされたときも成功として扱われる (09 §6.2)。
- `dev/<版>`・`release/*`・`hotfix/**` のブランチは作らない (凍結とパッチ版は `develop` と `main` だけで扱う)。
- **凍結中は `develop` への merge を保留する**: 版の凍結 (09 §9.2 手順 1) から昇格 (同 手順 4) まで、`develop` への PR (Dependabot の PR を含む。個人開発なのでこのまま保留する) を merge しない。**例外は `main` → `develop` の取り込みだけ** で、版上げ (同 手順 2) の前に限って merge してよい。版上げの後に `main` が進んだら、下の祖先の確かめで止まるので、`main` を `develop` に取り込んでから手順 2 (版上げ) をやり直す。ゲートの判定コミットを決めたら、`main` の先頭がその祖先であること (`git merge-base --is-ancestor main <判定コミット>`) を確かめる (09 §9.2 手順 2)。昇格の merge (`develop` → `main`) で判定コミットに無い変更が `main` に入らないようにするため。保留は昇格の後に解く。
- **最初の push** (08 §4 の M0e、OQ-168): 最初の push (M0e) は 2026-10-20 に完了した (`develop` に署名付き 6 コミットで docs/ と初期ファイルを入れて push 済み。PC-1〜PC-7 と GitHub の設定、既定ブランチ `develop`、ブランチ・タグの保護も済み)。それ以前の docs/ は未コミットで、バージョン管理とバックアップの対象外だった。以下は実施した手順の記録で、将来の公開前の見直しの参考にもなる。手順: (1) 手元のリポジトリで `main` (初期コミット) から `develop` を作る。(2) `develop` に docs/ と初期ファイル (リポジトリ直下の `README.md`・`NOTICE`・`SECURITY.md`・`CONTRIBUTING.md`・`CLAUDE.md`・`.gitattributes`・`minisign.pub`、`.github/` の Issue と PR のテンプレートと `dependabot.yml` (09 §6.3)・`allowed_signers`・`fuzz-recipients.txt`、その時点でできている M0c の成果物) をコミットする (`git commit -s`。著者名とメールアドレスは公開してよいもの、12 §5.6 の PC-6)。`Cargo.toml` を含む製品コードと CI のワークフローは G1a の後の M1 のタスク (0) で入れる。(3) 12 §5.6 の PC-1〜PC-7 を作業ツリーと手元の全履歴に対して行い、該当があれば push の前に履歴を書き換える。GitHub の設定 (PC-5 の (1)〜(6) と (8)) もこのときに済ませる。(4) `main` と `develop` を push する (`main` は v0.1.0 のリリースまで初期コミットのまま変えない)。(5) push の直後に、既定ブランチ (OQ-170) とブランチ・タグの保護 (`main`・`develop`。PC-5 の (7)。設定の内容は上の「ブランチ保護の設定」) を設定する。以後の変更は上の流れ (PR) で入れる。

### 9.2 コミットメッセージ

Conventional Commits を基本にし、仕様 ID をフッタに置く。

```
feat(eve-core): 線形 Undo の保存点と世代番号を実装する

03 §7.1 のとおり redo スタックを新しい編集で破棄し、SavePoint{depth, generation} で
「保存済み状態に戻った」判定を行う。木構造の Undo は持たない (OQ-114)。

spec: 03 §7.1
verifies: AC-FR-203-1, AC-NFR-203-1
Closes #123
```

| 要素 | 規則 |
|---|---|
| 1 行目 | `<type>(<scope>): <日本語または英語の要約>`。type は `feat` / `fix` / `perf` / `refactor` / `test` / `docs` / `spec` (仕様の変更。人間のみ) / `chore` / `ci` / `spike`。scope は crate 名またはモジュール (`eve-core/document`) |
| 本文 | 何を・なぜ。仕様の節を引く。実装の詳細は書かない (コードを読む) |
| フッタ | `spec:` / `verifies:` (§3.2 と同じ ID。v0.1 は節 ID と AC ID、v0.2 からは規範行 ID も (例 `spec: 03-7.1-1, 03-7.1-2, INV-H2`))、`Closes #n`、契約変更なら `contract: C1 +1`、仕様と違う判断があれば `needs-oq:` (PR では `needs-oq` ラベル) |
| エージェント | 本文末尾に `Agent: <名前>`。DCO の `Signed-off-by:` は **エージェントを操作した人間の名義** (§9.4)。エージェントのツールが既定で付ける `Co-Authored-By:` 行は残してよい (DCO の表明の代わりにはならない)。人間の共著は `Co-authored-by:` |
| 言語 | 日本語を基本 (仕様が日本語)。英語でもよい。混在させない |

### 9.3 CHANGELOG とリリース

- `CHANGELOG.md` は Keep a Changelog 形式、PR ごとに Unreleased へ 1 行 (09 §8.1)。ユーザー可視でない変更は `changelog: none`。
- 版・タグ・公開手順は 09 §9 が正本。本書は「版の名前は 01 §1.3 (v0.1 / v0.2 / v0.3 / v0.4 / v1.0 / v1.L (v1.L-a / v1.L-b) / v1.X)」を使うことだけを定める。
- 版上げと CHANGELOG の確定は、`develop` に対するリリース PR (`chore/…`、squash) で、**リリースゲートの判定の前に** 行う (ゲートは版上げ後の `develop` の先頭で判定する、09 §9.2 手順 2 / 3)。**v0.1 は bot を使わない**: リリース PR・昇格の PR (`develop` → `main`)・リリースの後の `main` → `develop` の取り込み PR は人間が作る (`release-plz` は v0.2 以降の候補、OQ-74。使う場合も対象のブランチは `develop`)。その後 §9.1 の流れで `develop` → `main` に入れ (パッチ版も同じ経路)、`main` のマージコミットに人間がタグを付ける。リリース PR と `main` への PR は人間が merge する。**タグの検証** (置き場所は 09 §6.1 の `release.yml` に統一する。手元の確認や `ci.yml` には置かない): `release.yml` の最初のステップで (1) `git verify-tag` (`.github/allowed_signers` に載る鍵の署名だけを許す)、(2) タグが `main` のマージコミットを指す (プレリリースのタグは `develop` の先頭)、(3) タグの版と `Cargo.toml` の版が一致する (プレリリースは `-rc.N` 等の接尾辞まで一致する。プレリリースの前に `develop` に `-rc.N` 等の版を付ける PR を入れる、09 §9.2 手順 1a)、(4) タグのコミットの内容が、リリースのチェックリストに記録したゲートの判定のコミット (版上げ後の `develop` の先頭) と一致する (差分が許可リスト `docs/compat/<version>/`・`docs/perf/<version>/`・`tests/ac-checklist.toml` だけ)、を確かめ、どれかが失敗したら以後を走らせない。タグの検証の結果は Release のノートの「検証」の節に書く (リリースのチェックリストには載せない。リポジトリには戻さない、09 §8.2)。エージェントは版上げ・タグ・publish を行わない。
- `develop` → `main`・`main` → `develop` の PR は merge commit で取り込み、1 行目は `chore(release): v0.1.0` / `chore(release): v0.1.1` / `chore(release): merge main into develop (v0.1.0)` の形にする (§9.2 の type と scope の規則のまま)。マイルストーンの取り込み (`0.0.x` の x を上げる PR、§9.1) は作業ブランチの PR (squash) で、`chore(release): 0.0.2 (M2)` の形にする。作業ブランチの PR は squash (1 PR = 1 コミットで、`spec:` / `verifies:` フッタと `Signed-off-by:` がそのコミットに残る。§9.4)。
- **リリースの前の文書の確認** (ADR-0040。エディタ内に更新機能を持たないため): 人間がリリースを publish する前に、(1) リリースノートの「更新の方法」(配布経路ごとの手順・その版で要る移行手順)、(2) 利用者向け README の「更新」節 (更新とダウングレード・直前の版に戻す手順。05 §18.10〜§18.12) がその版と合っているか、(3) `SECURITY.md` の更新方法の案内と、脆弱性の修正を含む版では GitHub Security Advisory の下書き (05 §18.14、OQ-164) を確認する。確認項目の正本は 09 §8.2 の「更新手順の文書」。エージェントはこれらの文書の下書きを PR で出してよい (publish は人間)。

### 9.4 寄稿ポリシー (CLA / DCO)

公開リポジトリへの外部からの寄稿 (PR) を受けるときの取り決め。寄稿された変更の権利関係を **法的に明確** にしておく。公開リポジトリのライセンスは **Apache-2.0 単独** に確定した (ユーザー回答、OQ-19 / OQ-152)。寄稿ポリシーは **DCO を既定** とすることが確定した (ユーザー回答、[OQ-154](10-decisions.md#oq-154-寄稿ポリシー)。用語と、DCO の意味と限界の整理は [12 §3.5](12-licensing-distribution.md#35-寄稿ポリシー-oq-154)。法的助言ではなく、弁護士の確認の項目と期限は 12 §3.8)。DCO チェック (09 §6.2 の `dco` ジョブ) は最初の CI から入れる。`CONTRIBUTING.md` は **最初の push の前** に最小の版 (外部からの PR をまだ受け付けていない旨・DCO の手順・寄稿は Apache-2.0 で提供される旨・`NOTICE` への追記を受け付けない方針。12 §5.6 の PC-4) を置き、**外部寄稿の受付開始前** に完全な版にする (記載内容は 09 §11.4)。

- **DCO** (Developer Certificate of Origin。`Signed-off-by:` 行): 寄稿者が「自分に寄稿する権利がある」ことを各コミットで表明する。ライセンスは **inbound = outbound** (公開リポジトリのライセンスと同じ条件で寄稿を受ける)。公開リポジトリは寛容ライセンス (Apache-2.0、ADR-0018 で確定) で、Apache-2.0 §5 により寄稿は同ライセンスで提供される。
- **決定** (確定 (ユーザー回答)、OQ-154): **DCO を既定** とし、`Signed-off-by:` の無いコミットは CI (DCO チェック) で失敗させる。CLA は、将来再ライセンスが必要になった場合にだけ導入を検討する (DCO では寄稿者の著作権が寄稿者に残るので、Apache-2.0 から別のライセンスへ変えるには寄稿者全員の同意が要る)。

- 寄稿を受け付ける範囲は **公開リポジトリ**。
- `CONTRIBUTING.md` (09 §11.4) に DCO の手順 (`git commit -s`) と「寄稿は公開リポジトリのライセンス (Apache-2.0) で提供される」旨を **明記** する (寄稿者が知らずに寄稿する状態を作らない)。AI コーディング支援で生成したコードの扱い (権利・表示) の方針も同じ文書に書く (内容は 12 §3.5。v0.1 のリリース後に決める、OQ-164)。
- エージェントの PR (§4.4) にも `Signed-off-by:` を要求する (エージェントを操作した人間の名義)。
- **DCO の検査の対象と squash の規則**: `dco` ジョブ (09 §6.2) が検査するのは **作業ブランチ → `develop` の PR** (外部の寄稿者・エージェント・メンテナの PR) のコミット。次の PR は対象外にする: (a) **昇格と取り込みの PR** (`develop` → `main` と `main` → `develop`。merge commit で取り込み、中のコミットは取り込み元のブランチに入ったときに検査済み。merge commit は新しい内容を持ち込まない)、(b) **bot の PR** (Dependabot。v0.1 は他の bot を使わない。bot は DCO の表明をできない)。(a) と (b) のどちらも、**head のリポジトリが base と同じこと** を対象外の条件に含める (fork から同じ名前のブランチで出した PR を対象外にしない、09 §6.2)。作業ブランチの PR と bot の PR は squash で入れ、**squash のコミットメッセージに `Signed-off-by:` を残す**: GitHub の squash の既定のメッセージ (「Pull request title and commit details」) は各コミットの本文を連結するので寄稿者の `Signed-off-by:` が残る。merge する人間はそれを消さず、bot の PR では自分の名義の `Signed-off-by:` を末尾に足す (人間がその変更を取り込む表明)。squash のメッセージは §9.2 の形 (1 行目・`spec:` / `verifies:` のフッタ) に整える。
- **外部寄稿の受付開始前に外部から PR が来た場合** (提案中、[OQ-171](10-decisions.md#28-oq-161-以降-仕上げ段階の追加起票)): 取り込まず、受付前であることと Issue での提案を案内する定型文で閉じる。fork の PR のワークフローは承認しない (§9.7)。受付開始 (条件は 08 §4、提案、OQ-174) の後は、`dco` ジョブで全コミットの `Signed-off-by:` を確かめ、ワークフローの実行を承認する前に変更 (特に `.github/`・`xtask`・`build.rs`) を読む。
- 翻訳 (`locales/*.toml`) の寄稿もコードと同じく DCO の対象で、公開リポジトリのライセンス (Apache-2.0) で受ける。v0.1 で同梱するのは `ja` だけ (英語 UI は v0.2、確定 (ユーザー回答)、OQ-21)。他の言語の同梱は、ユーザーの言語ファイル (07 §17.1) を入れた後に、保守する人がいるかを見て決める。

### 9.5 公開 crate と外部の消費者の運用

公開 crate は TUI 以外の **外部の消費者** にも使われ得る: 駆動 API のクライアント、プラグイン作者 ([ADR-0038](10-decisions.md#adr-0038-公開-crate-の外部の消費者と寄稿の方針))。TUI (`eve-tui` / `evoeditor` bin) は公開リポジトリに置き、OSS (Apache-2.0 単独) とすることが確定している (ユーザー回答、OQ-152 / 19)。本節は公開 crate の境界を保つための運用規則。

| # | 規則 | 補足 |
|---|---|---|
| XC-1 | **公開 API** = 外部の消費者が依存してよい範囲 ([12 §4.1](12-licensing-distribution.md#41-外部の消費者が依存してよい範囲): `eve-host` の `Frontend` / `Host` / `HostMsg` 公開部分、`eve-view` の境界型と `CONTRACT_VERSION`、`eve-config` の解決済み設定の型、`eve-core` の `Editor` ファサード) と、契約版、設定キー・CommandId (07)、シナリオ DSL の書式 (§6.3)。これらは **semver** で管理し、破壊的変更は `0.y` の間は `y` 上げ + 契約版の並行期間 (直近 2 版受理)、1.0 以降はメジャー上げ。範囲外の `pub` は「安定性の保証なし」と crate 文書に明記。`cargo semver-checks` で検査 (v0.1 リリースから報告、v0.4 からゲート。09 §7.1、OQ-157) | 駆動 API のクライアント・プラグイン作者も同じ API の消費者 |
| XC-2 | 別リポジトリの外部の消費者 は公開 crate を **正確な版 (`=0.y.z`、crates.io) または git タグで固定** して消費し、公開リポジトリをサブモジュール・path 依存で参照しない。公開 crate の未リリース (タグ無し) の変更に依存する作業はできない (先に公開側をリリースまたはタグ付けする) | 公開側の品質ゲート (09 §8) を迂回させないため。12 §4.2 (OQ-157) |
| XC-3 | 外部の消費者で見つかった公開 API / 契約の不足は、**公開リポジトリの Issue (契約変更要求、§8.4)** として起票する。必要な型・フィールドと理由を書く | 契約の議論を公開側に集める |
| XC-4 | 公開 crate のリリースは公開側の `release.yml` (09 §9) | 秘密情報の扱いは §9.7 |
| XC-5 | 公開側の **契約テスト成果物** (Frame JSON ゴールデン、シナリオ DSL、`eve-testkit` プリセット、`CONTRACT_VERSION`) は `contract-export` ジョブ (提案。TUI 完成後に公開側へ加える、12 §4.3) で公開 crate / リリースに添付し、外部の消費者はそれを **取り込む PR** で検査する。加えて別リポジトリの外部の消費者の **下流 CI** は公開側の main を夜間に取り込んで契約テストを回し、失敗は公開側への契約変更要求として扱う (公開側の PR を止めない。12 §4.3)。外部の消費者がゴールデンを独自に書き換えない | 契約の正本は公開側 |
| XC-6 | 公開側のテスト部品 (`HeadlessFrontend` / `CellOccupancy` / シナリオランナー / `xtask` の関数) は外部のリポジトリからも使える形で公開する (`eve-testkit`)。外部のリポジトリに複製しない | 二重保守の禁止 |
| XC-7 | core に **frontend の種類による分岐・型・設定を入れない**。レビューチェックリスト (09 §8.4 契約) で検査 | core が frontend を知らない境界の機械的な支え |

- 公開 crate の配置は v0.1 のリリース後に決める。XC-1 の semver の管理と `cargo semver-checks` の **報告** は公開側の v0.1 リリースから行い (ゲートは v0.4 から、09 §7.1)、XC-5 の `contract-export` は TUI 完成後に始める。v0.1 は `HeadlessFrontend` (ASCII の Frame) だけを置き、Frame JSON ゴールデンも TUI 完成後に作る (ADR-0039 / 0042)。

### 9.6 ライセンス検査 (`cargo deny` と同梱資産)

ADR-0018 の運用 (05 §17.1、09 §7 R6) をプロファイルとして管理する ([12 §3.6](12-licensing-distribution.md#36-依存同梱物のライセンス方針-公開リポジトリの依存とアセットのライセンスadr-0018) の運用面)。

| プロファイル | 対象 | 禁止 | 個別承認 | 検査 |
|---|---|---|---|---|
| `deny.toml` (公開リポジトリ) | 公開 crate と TUI の依存 | 許可・禁止のリストは 05 §17.1 が正本。本書では再掲しない | 同左 | `cargo deny check licenses` (毎 PR) |

- **`deny.toml` の始め方** (08 §5): M1 のタスク (0) は、確定した (ユーザー回答、2026-10-20) 05 §17.1 の許可リストで `deny.toml` を作る (暫定ではない)。`fuzz/` (別の `Cargo.lock`) にも `deny.toml` を当てる (09 §6.3)。`libfuzzer-sys` の `AND NCSA` のため、`NCSA` は `fuzz/` 側の `deny.toml` (または検査の例外) に限って許す (本体の許可リストには足さない、05 §17.1)。
- `THIRD-PARTY-NOTICES.md` は公開 crate 分 (cargo-about) から **生成** し、手で編集しない。
- 同梱アセット (文法・クエリ・テーマ) を追加・更新する PR は、`assets/NOTICES.toml` (資産台帳。出所 URL・版・ライセンス・帰属表示の要否を 1 件 1 行) の更新を必須とする。台帳に無い資産の同梱は認めない。
- CI で `xtask check-assets` (**v0.2 から**。最初の同梱資産と同時に入れる。v0.1 は同梱資産 (文法・クエリ・テーマのファイル) が無いので置かない) を実行し、台帳に無いファイルの同梱・許可リスト外のライセンス・帰属表示の欠落を検出して失敗させる (台帳の仕様は 12 §3.6)。`THIRD-PARTY-NOTICES.md` は台帳と依存の帰属表示から生成する。
- 公開 crate に GPL 系の依存が入ると、公開 crate を使う外部の消費者の再配布の条件が変わるので、公開側の検査は最初から厳しく運用する。

### 9.7 秘密情報の扱い (署名鍵・資格情報)

| 秘密情報 | 置き場 | 禁止 |
|---|---|---|
| リリース署名鍵 (minisign) | **CI に置かない**。メンテナの手元 (オフライン、パスフレーズ付き) に置き、手元で `SHA256SUMS` に署名する (05 §18.9。minisign は鍵を持たない署名 (OIDC) に対応しない)。公開鍵は README・`minisign.pub` とリポジトリの外の経路で公開 | リポジトリ (`.github/`、`xtask`、`.claude/`) と CI のシークレットに置かない |
| タグの署名鍵 (リリースのタグ `vX.Y.Z` に付ける署名。**minisign の鍵とは別の鍵**) | 種類は **Git の SSH 署名** (`gpg.format = ssh`、ed25519 の鍵)。メンテナの手元に置き (パスフレーズ付き。ハードウェアキーでもよい)、CI とリポジトリに置かない。公開は (1) GitHub のアカウントに **Signing key** として登録する (タグが「Verified」と表示される)、(2) 公開鍵をリポジトリの `.github/allowed_signers` (`git verify-tag` に使う `gpg.ssh.allowedSignersFile` の形式) に置く。鍵を替えたら古い鍵の行を消さずに有効期限を付けて残す (過去のタグの検証のため) | minisign の鍵を兼ねない (用途と失効の手順を分ける)。CI のシークレットに置かない |
| fuzz のクラッシュ入力の復号鍵 (`age` の秘密鍵) | メンテナの手元 (パスフレーズ付き)。公開鍵だけを `.github/fuzz-recipients.txt` に置く (09 §6.3)。メンテナが替わったら recipients の行を更新し、古い鍵の行を消す | CI のシークレット・リポジトリに置かない。復号した入力を公開の場 (Issue・PR・ログ) に貼らない (公開の可否は 09 §6.3 の起票の規則) |
| コード署名 (Windows Authenticode は v0.4 から (OQ-75)、macOS の Developer ID 署名・公証は未決 (OQ-76。v0.2 の macOS 版は ad-hoc 署名)) の証明書・API キー | CI シークレット (保護された `environment`: 承認者と、`main` ブランチ・`v*` タグへの制限) | 開発者のローカルに証明書を配らない (署名はリリース CI だけ)。ログに出さない |
| エージェント (Claude Code 等) の実行環境 | `.claude/settings.json` の許可コマンドに署名・publish・鍵を扱うコマンドを含めない (§5)。エージェントはシークレットを読めない環境で動かす | `cargo publish`、`codesign`、`notarytool`、`signtool` を許可しない |
| 基準機・開発機の固有情報 (機種名、CPU や SSD の型番・容量、OS のビルド番号、ウイルス対策ソフトの製品名、ユーザー名を含むパス) | リポジトリの外の非公開のメモ (git 管理外。基準機の詳細の記録) | docs・コード・コミットメッセージ・計測結果に書かない。計測結果には環境 ID (例 `ENV-W1`、01 §3.0) だけを書く |

- 秘密情報の漏えいが疑われたら: 鍵の失効 (minisign は新鍵で再署名して README を更新、タグの署名鍵は GitHub の登録を外して `.github/allowed_signers` の行に失効日を付ける、コード署名証明書は発行元で revoke)。手順はメンテナの運用メモに置き、利用者への通知は `SECURITY.md` と GitHub Security Advisory で行う (05 §18.14)。
- **秘密情報の検出** (提案中、[OQ-169](10-decisions.md#28-oq-161-以降-仕上げ段階の追加起票)): (1) push の前の手元の確認 (12 §5.6 の PC-2 の検索)、(2) GitHub の secret scanning と push protection (最初の push の前に有効にする、PC-5)、(3) CI の `gitleaks` 相当のジョブ (M1 のタスク (0) から毎 PR。`docs.yml` 相当の軽いジョブ) の 3 層にする。手元の pre-commit フックは任意。
- **最初の push の前** (08 §4 の M0e、OQ-160 / 168): [12 §5.6](12-licensing-distribution.md#56-公開前チェック-最初の-push-の前oq-160--168) の公開前チェック (PC-1〜PC-7: 基準機の情報の一般化・履歴を含む再 grep・README の状態・`NOTICE`・`SECURITY.md`・`CONTRIBUTING.md`・GitHub の設定・著者情報とテストデータと fuzz の成果物 (暗号化した artifact)・報告窓口) を行う。push した内容は全ブランチの履歴とともに直ちに公開され、後から消しても複製に残り得るので、検索は作業ツリーだけでなく push する履歴全体に対して行う。最初の push の後も、各 push の前に PC-2 の対象 (秘密情報・基準機の固有情報・個人の情報) が入っていないことを確かめる。
- **テストデータ** (確定 (ユーザー回答)、09 §4): コーパス・テスト用のファイルは合成データか再配布が許されるデータだけを使い、実際の個人の文書を使わない。fuzz のクラッシュ入力は、メンテナの公開鍵 (`.github/fuzz-recipients.txt`) で `age` により暗号化した artifact としてだけ上げ、メンテナが手元で復号して再現する (09 §6.3)。
- **再 grep の検索手順** (PC-2): 製品名・型番などの語は **単語境界で** 検索する (`rg -w -i <語>`、履歴は `git grep -w -i <語> $(git rev-list --all)` と `git log --all --format='%an %ae %s %b'` の出力)。部分一致で探すと、ロケール用語の `CODESET` のような無関係な語に当たって確認が漏れやすくなる。検索語の一覧はリポジトリの外の非公開のメモに置く (本表の最下行と同じ扱い)。秘密情報の形 (鍵・トークン) の検索は部分一致のまま行う。
- 公開リポジトリの Actions では、外部の寄稿者 (fork) からの PR のワークフローの実行に承認を要する設定にし、`GITHUB_TOKEN` の既定の権限を読み取りにする (12 §5.6 の PC-5)。シークレットを使うジョブを fork の PR で動かさない。v0.1 の間は CI にシークレットを置かない (リリースの署名は手元の minisign だけで、コード署名は v0.4 から)。

### 9.8 脆弱性の受付・対応・開示

報告の窓口を用意する時期と文書の一覧は 12 §5.5 / §5.6 (PC-4 / PC-7)、利用者への通知の方針は 05 §18.14。本節はメンテナの運用の手順を定める。

**`SECURITY.md` の最小の版** (最初の push の前、12 §5.6 の PC-4) に書くこと: (1) 報告の窓口は GitHub の **private vulnerability reporting** (リポジトリの「Security」→「Report a vulnerability」) で、公開の Issue に書かないこと、(2) 対象の版 (最新のリリース。v0.1 のリリースまでは「未リリースで配布物が無い」旨)、(3) 下の目安の日数 (受領の連絡・最初の判断)、(4) 開示の方針 (修正版の公開と同時に勧告を公開する。報告者の名前は希望に従って記す)。更新方法の案内 (README の「更新」節への導線) は v0.1 のリリース前に加える (09 §8.2)。

| 段階 | 手順 | 目安 |
|---|---|---|
| 1. 受付 | private vulnerability reporting で受ける。公開の Issue・PR・fuzz の起票で脆弱性の疑いが見つかったら、メンテナが内容を非公開の勧告の下書き (GitHub Security Advisory) に移し、公開側の記述を最小にする (Issue は「非公開で扱う」と書いて閉じる) | 受領の連絡は 7 日以内 |
| 2. トリアージ | 再現・影響の範囲 (対象の版・前提条件)・重大度を判断する。**08 §8.1 の SB-3 (セキュリティ) に当たるものは重大 (`severity:critical` 相当)** として扱い、それ以外は CVSS の目安で高・中・低に分ける。依存の脆弱性で evoeditor の使い方では到達しないものは、到達しない理由を記録して `deny.toml` の `[advisories] ignore` に期限付きで載せる (SB-3 の除外。09 §6.3) | 最初の判断は 14 日以内 |
| 3. 非公開の修正 | Security Advisory の **一時的な private fork** で修正を作る。private fork では Actions が動かないので、メンテナが手元 (Windows の基準機・開発機) で `cargo xtask check` と関連のテスト (L0〜L3、必要なら L5) を回し、結果を勧告の下書きに書く。修正にはテストを付ける (公開の後に回帰テストとして残る) | 重大は 30 日、その他は 90 日以内の修正版を目標 |
| 4. 公開のリリース | 修正をパッチ版の経路 (09 §9.3、§9.1 のパッチ版。`feat` が入っていれば次の機能リリースに含める) で `main` に入れ、パッチ版を出す (09 §9.2。公開前の作業ブランチは private fork から作り、公開の PR は修正版のリリースの直前に作る)。リリースノートに「セキュリティの修正」と勧告への参照を書く | — |
| 5. 開示 | 修正版の publish と同時に勧告を公開する。**識別子**: GitHub Security Advisory の GHSA ID を付け、CVE は GitHub (CNA) に勧告から要求して発番する (重大と高は必ず、中・低は報告者の希望か利用者への影響で判断)。公開した勧告は RustSec に知らせる (公開 crate を crates.io に出している場合)。SB-3 に当たったものは公開の後に Issue のラベル (`severity:critical`) を付けて記録する (08 §8.1 の判定に使う) | 報告から 90 日を開示の上限の目安とし、延ばすときは報告者と合意する |

- **fuzz のクラッシュ**: 夜間の fuzz の公開のログには入力とスタックを出さず (`-close_fd_mask=3`)、クラッシュ入力はメンテナの公開鍵で `age` により暗号化した artifact としてだけ上げる。自動で起票する課題にはジョブの失敗とクラッシュの件数だけを書く。メンテナが artifact を手元で復号して再現し、Rust の `panic` でセキュリティ性が無いと判断したものだけ詳細を公開の課題に書く。それ以外 (panic 以外の異常終了、`unsafe` を含む crate の中で起きたもの、判断がつかないもの) は上の手順の 1 から扱う (09 §6.3)。
- **v0.1.0 のリリースの前 (`0.0.x` の期間) の脆弱性**: 配布物が無いので、パッチ版・勧告の公開は行わない。修正は上の手順 3 と同じく private fork で作り、`fix/…` → `develop` の PR (人間が作る) で入れる。PR の説明とコミットメッセージには脆弱性の詳細を書かない (公開リポジトリのため)。報告者への連絡と、必要なら v0.1.0 のリリースノートでの言及を行う。
- **Dependabot のセキュリティ更新の PR**: GitHub の仕様では、セキュリティ更新の PR は既定ブランチに宛てて作られる (`target-branch` はバージョン更新だけに効く。**要確認**)。v0.1.0 のリリースまでは既定ブランチが `develop` だったので、そのまま `develop` で受けた (凍結中は §9.1 のとおり `develop` への merge を保留し、リリース候補に要る更新は人間が `fix/…` → `develop` で入れてゲートをやり直す)。v0.1.0 の後 (2026-10-27 から) は既定ブランチが `main` なので、`main` 宛ての PR は merge せず (`pr-source` ジョブが失敗する)、人間が同じ更新を `fix/…` → `develop` で入れ、パッチ版の経路 (§9.1。`feat` が入っていれば次の機能リリースに含める) で `main` に入れてから bot の PR を閉じる。
- 勧告の下書き・private fork・報告者とのやり取りはリポジトリの外に複製しない。基準機の固有情報を勧告に書かない (§9.7)。

---

## 10. 未決事項

番号は [10 §2](10-decisions.md#2-未決事項-open-questions) の統一 OQ 番号。区分・詳細は 10 を正とし、本表は本書側の推奨の記録。

| OQ | 件名 | 推奨 | 区分 / 決定期限 |
|---|---|---|---|
| OQ-126 | `docs/ownership.toml` の形式と、CODEOWNERS との関係 | **確定 (ユーザー回答)**: モジュールは 02 §4.6 の 10 個、`ownership.toml` は §2.3 の形式の手書きの割当表。CODEOWNERS は `ownership.toml` から `xtask` で生成する (二重保守しない)。置くのは v0.2 から (ADR-0042) | 裁量 / 回答済み (初版は v0.2) |
| OQ-127 | 03 (コア設計) の分割 | 03 は長大で `eve-core` の複数モジュールにまたがるが、当面 **分割しない**。§2.2 の所有節と spec-index で該当節だけを読む運用で足りるかを見て、v0.4 の文書改訂で再検討する | 裁量 / v0.4 |
| OQ-123 | 契約の凍結時期 (C2 の時点) | M2 の完了時 (M3 の端末に依存する部分を始める時点、08 §2.2・02 §8.4)。C1 の二重消費は TUI + ヘッドレスの Frame 消費者 + テスト用 Frontend。本書 §8.4 の手続きは段階に依存しない。確定 (ユーザー回答、2026-10-21): M2 の完了をもって C2 | 裁量 (確定) / v0.1 の M2 |
| OQ-154 | 寄稿ポリシー: DCO か CLA か (§9.4) | **確定 (ユーザー回答)**: **DCO を既定** (公開リポジトリのライセンスは 12 §3.2 (Apache-2.0 単独、OQ-19))。CLA は再ライセンスが必要になった場合のみ。本書の運用: DCO チェックを CI に置き、エージェントの PR にも `Signed-off-by:` を要求する | PO / 12 §3.5、11 §9.4 / 回答済み (`CONTRIBUTING.md` は外部寄稿の受付開始前) |
| OQ-120 | ヘッドレス RPC (駆動 API) を配布ビルドに同梱するか | v0.x は dev feature。S-21 の計測後に v0.4 で判断する (配布ビルドで `--headless` 系を予約名として終了コード 2 で終えることと、`eve-view` の公開 feature を `serde` の 1 つにすることは確定 (ユーザー回答))。本書の運用は dev feature で足りる (エージェントはソースからビルドする) | PO / v0.4 着手前 |
| OQ-147 | 規範行 ID の全文書への付与時期 | **確定 (ユーザー回答)**: 段階的に付け、全文書は v0.2 まで。付与は人間が行い、エージェントは提案だけ。**v0.1 では規範行 ID を付けない** (確定 (ユーザー回答)、[ADR-0042](10-decisions.md#adr-0042-v01-の最小核-案-lwindows-のみの出荷日本語のみの-ui)): M0c は手書きの最小限で、`spec:` は節 ID (`03 §9.3`)、`verifies:` は AC ID (`AC-FR-105-1`)。設計の規範行 ID は v0.2 から、各タスクの着手前にそのタスクが参照する節へ付ける (ID-6、08 §4) | 裁量 / 回答済み |
| OQ-148 | `CLAUDE.md` と `AGENTS.md` の生成元 1 本化 | M0c は手書きの `CLAUDE.md` だけを置く (ADR-0039)。`AGENTS.md` が必要になった時点 (遅くとも v0.2) で `.claude/agent-entry.md` から `xtask` で両方を出す (§5)。製品固有の記述 (`.claude/` のパス) は条件付きブロックで | 裁量 / v0.2 |
| OQ-149 | エージェントの PR に対する `reviewer` エージェントの事前レビューを必須にするか | 任意 (`agent-pr` ラベルで自動実行、結果は参考)。人間のレビューを置き換えない | 裁量 / v0.1 の M2 末 |
| OQ-150 | ローカル LLM を開発ループ (§7.3) に使うときの `.claude/` 設定の持ち方 | リポジトリには **プロバイダ非依存の入口** (`CLAUDE.md` / `AGENTS.md`、`xtask` のコマンド) だけを置き、ローカルモデルの指定 (エンドポイント・モデル名) は開発者ごとの未コミット設定 (`.claude/settings.local.json` 等) に置く。モデル名を docs やリポジトリ設定に書かない (06 §4.4 の「モデル名をコードに埋め込まない」と同じ理由) | 裁量 / v1.L-a 着手前 |
| [OQ-167](10-decisions.md#oq-167-ブランチと公開の運用) | ブランチと公開の運用 (§9.1) | **確定 (ユーザー回答)**: `main` はリリース用、開発は `develop` の 1 本 (長命ブランチは `main` と `develop` だけ)、`dev/<版>`・`release/*`・`hotfix/**` は作らない、v0.1.0 までの版は内部カウンタ `0.0.x`。細目 (提案中): ブランチ・タグの保護、`0.0.x` の番号の上げ方、パッチ版は `fix/…` → `develop` → `main` (前回のタグ以降に `feat` が無いとき) | PO / 回答済み (細目は最初の push の前) |
| [OQ-168](10-decisions.md#oq-168-開発期間中のリポジトリの公開状態) | 開発期間中のリポジトリの公開状態 (§9.1、§9.7) | **確定 (ユーザー回答)**: public のまま開発し、docs/ も最初の push から公開する。最初の push の前に 12 §5.6 の公開前チェックを行う (§9.1 の「最初の push」) | PO / 回答済み |
| [OQ-169](10-decisions.md#28-oq-161-以降-仕上げ段階の追加起票) | 秘密情報の検出の層 (§9.7) | push の前の手元の確認・GitHub の secret scanning と push protection・CI の `gitleaks` 相当の 3 層。pre-commit フックは任意 | 裁量 / 最初の push の前 |
| [OQ-170](10-decisions.md#28-oq-161-以降-仕上げ段階の追加起票) | v0.1.0 のリリースまでの既定ブランチ (§9.1) | `develop` (v0.1.0 のリリースで `main` に戻す。2026-10-27 に戻した) | 裁量 / 最初の push の前 |
| [OQ-171](10-decisions.md#28-oq-161-以降-仕上げ段階の追加起票) | 外部寄稿の受付開始前に来た PR の扱い (§9.4) | 取り込まずに定型文で閉じる。PR の作成を制限する GitHub の設定があれば使う (要確認) | 裁量 / 最初の push の前 |
