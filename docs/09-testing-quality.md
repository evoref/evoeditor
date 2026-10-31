# 09. テスト戦略と品質 (Testing & Quality)

| 項目 | 内容 |
|---|---|
| 文書ID | 09-testing-quality |
| 状態 | ドラフト |
| 主対象 | 全 crate + `eve-testkit` |
| 前提 | [00-overview](00-overview.md) / [01-requirements](01-requirements.md) / [02-architecture](02-architecture.md) / [05-platform](05-platform.md) / [11-dev-workflow](11-dev-workflow.md) (規範行 ID・`spec:`・シナリオ DSL 書式・ヘッドレス検証ループの正本) |
| 決定記録 | [10-decisions](10-decisions.md) (ADR-0012, ADR-0018, ADR-0019, ADR-0039, [ADR-0042](10-decisions.md#adr-0042-v01-の最小核-案-lwindows-のみの出荷日本語のみの-ui) (v0.1 の範囲), OQ-14, OQ-15, OQ-23) |
| 責務 ([02 §12.1](02-architecture.md#121-文書と-crate-の対応)) | テスト階層 (単体/プロパティ/fuzz/ヘッドレス統合/端末 E2E)、ベンチマーク (NFR-1xx の測定方法)、CI マトリクス (OS × アーキ)、依存規則 ([02 §4.3](02-architecture.md#43-依存規則-ci-で機械的に検査する)) の検査、カバレッジ・lint ゲート、リリース判定 |
| 決めないこと | 性能目標値そのもの (01)、要件の追加 (01)、版の計画 (08)、開発運用 (規範行 ID の付け方・タスク単位・CLAUDE.md・ブランチ規約。11) |
| v0.1 で読む節 | §1〜§8 のうち v0.1 の行 (§7.1 を含む)、§9、§10.2、§11.2〜§11.4 (一覧の正本は [README §3.1](README.md#31-v01-の実装で読む節)) |

> 本書は「仕様が守られていることを **機械的に** 確認する方法」を定める。人手の確認 (M) と レビュー (R) は手順と記録様式を定め、属人化を避ける。
> コード片は擬似コード。テストの命名・配置は実装着手時に本書へ合わせる。
> 検証スパイク (S-01〜S-23。S-16〜S-19 は欠番) の **正本は 08 §3**。本書は各スパイクが使うテスト道具と合否判定の自動化だけを書く。

---

## 目次

1. [方針](#1-方針)
2. [検証区分 (T / B / M / R) の定義](#2-検証区分-t--b--m--r-の定義)
3. [テスト階層](#3-テスト階層)
4. [テストデータ (コーパス)](#4-テストデータ-コーパス)
5. [性能ベンチマーク](#5-性能ベンチマーク)
6. [CI (GitHub Actions)](#6-ci-github-actions)
7. [依存規則 R1〜R11 の機械検査](#7-依存規則-r1r11-の機械検査)
8. [品質ゲート](#8-品質ゲート)
9. [リリース手順](#9-リリース手順)
10. [クラッシュ報告・テレメトリ](#10-クラッシュ報告テレメトリ)
11. [ドキュメント運用](#11-ドキュメント運用)
12. [未決事項](#12-未決事項)

---

## 1. 方針

| # | 方針 | 根拠 |
|---|---|---|
| Q1 | **core はヘッドレスでテストする**。TUI を起動しないテストが全テストの 9 割以上を占める | NFR-701、ADR-0002。端末 E2E は遅く不安定なので薄く |
| Q2 | **要件 ID とテストを双方向に辿れる** (トレーサビリティ)。Must 要件に対応テストがない状態で版を閉じない。v0.1 は実装の `spec:` に **節 ID** (`03 §9.3`)、テストの `verifies:` に **AC ID** (`AC-FR-105-1`) を書き、AC のチェックリスト `tests/ac-checklist.toml` (§8.5 の表から起こす。形式は 11 §3.3) で突合する (NFR-708)。規範行 ID (11 §3) と `xtask trace` は v0.2 | 01 §1.4、00 §7.1、ADR-0033 |
| Q3 | **決定性**: テストは偽 Port (`eve-testkit`) と手動進行 `Clock` で時間・I/O・乱数を制御する。`sleep` を含むテストを書かない | ADR-0006、flaky 撲滅 |
| Q4 | **データ保全は障害注入で証明する**: 原子的保存は「途中で殺す」テストを持つ | P1 原則、NFR-301 |
| Q5 | **毎 PR で Windows x86_64 の全テストと、Linux x86_64 の Linux CI の対象 crate (core 系 + `eve-host` + `eve-testkit`、§6.4) のビルドとヘッドレスのテストを走らせる** (§6.4)。v0.1 は Windows だけを出荷する (確定 (ユーザー回答))。Linux は core の移植性を保つためだけに走らせ、macOS の CI は macOS を出荷する版から。OS 固有の分岐 (`eve-platform`) は実 OS でしか意味がない | NFR-401、G3 |
| Q6 | **性能は回帰として監視し、絶対値はリリース前に基準環境で測る** | OQ-23 (基準機は Windows の開発 PC 1 台)、NFR-703。CI ランナーの絶対値はノイズが大きい |
| Q7 | **外部へ何も送らない**: テレメトリ・クラッシュ自動送信は実装しない。CI の成果物も GitHub のリポジトリ内に留める (外部のサービスへ送らない) | ADR-0019、NG8 |
| Q8 | **シナリオを一級のテスト成果物にする**: v0.1 はキー列・コマンド・期待状態を **Rust の `Harness`** (§3.4) で書き、ASCII の Frame を出力して確かめる (NFR-707)。Harness の API (`h.keys` / `h.cmd` / `h.clock` / `h.run_jobs`) は v0.2 で入れる TOML のシナリオ DSL (書式の正本は 11 §6) と同じ語彙にしておき、DSL と `xtask scenario run --print-frame` は v0.2、駆動 API (`evoeditor --headless --rpc stdio --fake-ports`、dev feature) はそれ以降 (01 NFR-707 AC-3 は駆動 API を実装した版から適用)。AI エージェントが最初に書くテストはこれ | NFR-707、ADR-0032 / ADR-0033 |
| Q9 | **安定性の規則 (02 §14.1 MR1〜MR12) を検査で裏打ちする**: Undo の会計と RSS、受信口の生産側集約、メインスレッドの大きな文字列、ブロックするロック、退避のスロット表が文書数に上限を持たないこと (MR12)。長時間稼働はソークで | ADR-0031 |

---

## 2. 検証区分 (T / B / M / R) の定義

[01 §1.4](01-requirements.md#14-受入基準の書き方) の記号を具体化する。

| 区分 | 定義 | 実行者 | 実行頻度 | 記録 |
|---|---|---|---|---|
| **T** 自動テスト | §3 の L0〜L5 のいずれか。CI で合否が自動判定される | CI | 毎 PR | CI ログ + トレーサビリティ行列 (§8.3) |
| **B** ベンチマーク | §5 のベンチ ID を持ち、(a) CI では値の記録 (v0.1。自動の相対回帰の判定は v0.2 から)、(b) リリース前に基準環境で絶対値を測る | CI + リリース担当 | (a) `bench.yml` の手動実行 (v0.2 から `develop` への push でも、§6.1) (b) リリース毎 | `docs/perf/<version>/` |
| **M** 手動確認 | §3.7〜3.8 のチェックリスト。手順・期待結果・環境を記した項目をリリース前に実施 | リリース担当 (+ 協力者) | リリース毎 (端末マトリクスは v0.1 の対象端末 = Windows Terminal と conhost) | `docs/compat/<version>/` |
| **R** レビュー | §8.4 のチェックリストをレビュー時に適用。設計文書との整合は ADR 参照で確認 | レビュア | 毎 PR | PR のチェックボックス + 承認 |

複数区分が付く要件 (例 FR-205 `T/M`) は **全区分を満たす**。

---

## 3. テスト階層

```
                        ┌──────────────┐  L7 手動受入 (M)        : ユーザビリティ、IME、端末マトリクス
                        │  L6 互換      │  L6 互換マトリクス (M)   : 端末 × OS
                       ┌┴──────────────┴┐ L5 端末 E2E (T)         : 疑似端末 (pty / ConPTY) でバイナリを駆動
                      ┌┴────────────────┴┐ L4 TUI 描画 (T)        : Frame → ratatui Buffer のスナップショット
                     ┌┴──────────────────┴┐ L3 ヘッドレス統合 (T)  : Editor + 偽 Port + シナリオ
                    ┌┴────────────────────┴┐ L2 fuzz (T, 夜間)     : 外部入力でパニックしない
                   ┌┴──────────────────────┴┐ L1 プロパティ (T)     : 代数的性質 (proptest)
                  └┴────────────────────────┴┘ L0 単体 / doctest (T)
```

### 3.1 L0: 単体テスト・doctest

- 各 crate の `#[cfg(test)]` と `tests/`。公開 API は doctest を持つ (NFR-705 と同時に満たす)。
- 命名: `fn <対象>_<条件>_<期待>()`。要件対応は属性コメント `// verifies: FR-105, NFR-301` を関数直前に置く (§8.3 の収集対象)。
- `eve-text` は **モデル検査スタイル** を基本にする: 同じ操作を `TextStorage` と素朴な `String`/`Vec<u8>` の参照実装に適用し結果を比較。
- テストランナーは `cargo nextest` (並列・プロセス分離・リトライ制御・JUnit 出力)。`cargo test` でも動くことを維持 (doctest は nextest 非対応のため `cargo test --doc` を別実行)。

### 3.2 L1: プロパティテスト (proptest)

| 対象 crate | 性質 (抜粋) | 生成器 |
|---|---|---|
| eve-text `TextStorage` | 任意の挿入/削除列の結果が参照実装と一致。byte⇔char⇔行/桁の変換が往復する (UTF-16 との変換は LSP と同時の v1.L)。書記素境界への正規化は冪等 | 任意 Unicode 文字列 (結合文字・ZWJ・サロゲート境界・CR/LF/CRLF 混在を重み付け) |
| eve-text `ChangeSet` | `compose(a, b)` の適用 = `a` 適用後 `b` 適用。`invert(a)` 適用で元に戻る。`map_pos` が単調。空 ChangeSet は単位元。**rebase の収束** (v1.L。最初の利用者 = Proposal の写像と同時に実装する。v0.1 の ChangeSet は `map_pos` / `map_range` まで): 非衝突の `a`, `b` について `apply(apply(t, b), a.rebase(b, OtherFirst)) == apply(apply(t, a), b.rebase(a, SelfFirst))` (同位置への同時挿入があるため tie ポリシーは **相補的な組** で定義する。同じポリシー同士では成り立たない)。`map_range` の分類 (03 §6.4): 交差しない変更は `Mapped`、範囲内・境界またぎ・端に接する挿入は `Overlapped` で `kind` が正しい | 文書に整合する ChangeSet |
| eve-core Undo (**線形**、03 §7.1) | 任意の Transaction 列 → 全 Undo で初期状態と **バイト一致**、全 Redo で最終状態と一致。Undo 後の新しい編集で redo スタックが空になる。結合規則 (03 §7.3) 下でも成り立つ。予算超過 (文書 64 MiB / 10,000 ステップ / プロセス全体 256 MiB) 時も直前 1 操作は Undo 可。**保存点と dirty** (03 §7.1 / §7.4、INV-D3 / INV-H3): 予算による破棄 (文書あたり・プロセス全体) を編集列の任意の位置に挟んでも、`dirty` が参照モデル (保存時と同じ履歴位置 = 同じ `NodeId` にいるか) と一致する。保存 → 入力で `dirty = true` になる (保存直後の入力が保存前のノードに結合しない)。Undo してから保存 → 新しい編集 → Undo で保存位置に戻ると `dirty = false`。**会計**: `InsertText::Slice` は削除 64 KiB 以上のノードにのみ現れ (INV-H4)、ランダムな 1 万回の編集 (小さな削除を多く含む) の後のメモリ指標 (Windows は Private Bytes。BM-203 と同じ) の増分が Undo 会計の合計 × 1.2 以下 (01 FR-203 AC-3。数値の判定は BM-203、ここでは会計値と実測の単調性)。64 KiB 以上の削除は `Slice` で記録された後に `bg` ワーカーの実体化ジョブで独立の rope (`InsertText::Rope`) に置き換わる (03 §6.5 / §7.5) ので、メモリ指標は **実体化ジョブがすべて完了した後** に測る。会計は `Slice` / `Rope` とも「削除バイト数 + 8 KiB」で、実体化の前後で変わらない | Transaction 列 + Clock 進行 |
| eve-core Selection | 変更後の Selection 写像が範囲内・正規化済み (anchor ≤ head の規約は 03)。複数 Range の非交差 | 文書 + Selection + ChangeSet |
| eve-core `EditSummary` (03 §6.5、06 §17 X25。**v1.L**: 生成は最初の利用者 (LSP 増分同期 / ローカル LLM) と同時。v0.1 は型の予約のみで、`DocumentChanged` は `edits` を持たない) | 任意の Transaction について、`Exact` の各 `EditDelta.old` が昇順・重なりなしで ChangeSet の変更箇所と一致し、`new_len` の総和 − `old` 長の総和 == 文書長の差。65 箇所以上は `Coarse` で `first_old` ≤ 最初の変更位置、`last_old_end` ≥ 最後の変更終端。`first_old` より前のバイト列は適用前後で一致 (購読者の前提)。Reload / Recovery は `Replaced` | ランダムな ChangeSet (1〜200 箇所) |
| eve-core `Query` の決定性 (03 §6.5、06 §17 X26。v0.1 の `Query` はシナリオの assert に要る変種 (文書一覧・テキスト範囲・選択・ステータス・オーバーレイ) のみ) | 同じ Revision・同じ範囲の `Query::DocumentText` を繰り返し呼んでも同じバイト列。改行 (LF / CRLF / CR 混在) と不正バイト (PUA エスケープ) の表現が Snapshot と一致。範囲外の編集後も Revision が同じ範囲 (別文書) では変わらない | 文書 + 範囲 + 無関係な編集列 |
| eve-encoding (バイト → テキスト → バイト) | 任意のバイト列 (有効/無効混在) について、**v0.1**: `lossy_roundtrip` の範囲外は `decode → encode` が元バイト列と一致し (UTF-8 / UTF-16 / Shift_JIS / EUC-JP / **ISO-2022-JP**)、範囲内 (区分 B) は `LossyRoundtrip::Summary` の件数・先頭位置が正しく検出される (03 §8.6 E2')。ISO-2022-JP の区分 B は **行単位** (03 §8.7 区分 D): 差のある行数 == `Summary.count`、正規形で書かれた行は一致。**v0.3**: 範囲内も保持方式 (03 §8.6 E2) で一致する (NFR-303 の区分 A / B。ISO-2022-JP は ASCII 状態で始まり終わる無編集の行だけ元バイト列)。**チャンク分割 (1 バイト単位を含む。ISO-2022-JP ではエスケープシーケンスの途中で切る分割を含む) で結果が変わらない**。不正バイトのエスケープ数 == `invalid_bytes` (ISO-2022-JP では WHATWG デコーダが受け付けない入力 (`ESC $ ( D`、8 bit、`SO` / `SI`、冗長エスケープ) が不正バイトになり、文書は読み取り専用)。比率 1% 超で `NeedsConfirm` が 1 回だけ出る (FR-127) | 任意バイト列 × 文字コードラベル × チャンク分割 |
| eve-encoding (テキスト → バイト → テキスト) | 当該文字コードで **表現可能な文字集合のみ** から生成した文字列について `encode → decode` が一致。生成器は 03 §8.6 の照合で表現不能になる文字 (`¥` `‾` `−` 半角カナ等の黙示置換対象、JIS X 0212。ISO-2022-JP では半角カナ・`SO` / `SI` / `ESC`) を **除外** する。別の性質として「除外対象を 1 文字でも含む文字列は E1 の照合で 100% 検出される」(01 FR-112 AC-3) | 表現可能文字集合 / 黙示置換対象の注入 |
| eve-encoding E3 の置換 (03 §8.6 の「E3 の選択肢」) | 任意の文字列と保存先の文字コードについて、置換して作った文字列は E1 の照合で表現不能が 0 件。変わるのは表現不能と判定された文字だけで、**ISO-2022-JP で保存する場合の半角カナは対応する全角カナ**、**それ以外の表現不能文字は `?` (U+003F)** に置き換わる。置換の前に示す一覧の **内訳** (置換先ごとの件数: 全角カナへ N 件・`?` へ M 件) の合計が表現不能の件数と一致し、一覧は **先頭 100 件** だけを 1 件 1 行で持ち (100 件を超えるときは末尾に「ほか N 件」が付き、N = 全件数 − 100)、一覧の置換後の文字が実際の置換と一致する (03 §8.6 の「置換の一覧」)。**濁点・半濁点の規則**: ISO-2022-JP の保存で `ﾞ` (U+FF9E) / `ﾟ` (U+FF9F) は直前の文字と合成せず `゛` (U+309B) / `゜` (U+309C) になる (`ｶﾞ` → `カ゛`。表現不能の 1 文字が置換後の 1 文字に対応する) | 表現可能文字集合 + 表現不能文字 (半角カナ・濁点付き半角カナ・絵文字・JIS X 0212 等) の注入 × 文字コードラベル。件数は 0〜300 (100 件の境界をまたぐ) |
| eve-core `SaveEncoder` の決定性 (03 §9.3 の「乾式走査と出力長」) | 任意の Snapshot と format について、`dry_scan` の `out_len` と、`SaveEncoder::new` の 2 回目のエンコードが供給するバイト列の長さが一致し、同じ Snapshot と format からの 2 回のエンコードが **同じバイト列** になる (チャンク分割によらない。ISO-2022-JP の行ごとの終端・BOM を含む)。表現不能があれば `dry_scan` が `Unmappable` を返し、`total` が E1 の件数と一致する | 任意 Unicode 文字列 × 文字コードラベル × BOM の有無 × チャンク分割 |
| eve-encoding ISO-2022-JP の正規形と状態引継ぎ (03 §8.6 E6) | (a) `encode(decode(encode(t))) == encode(t)` (正規形の冪等性)、(b) 1 つのエンコーダで任意に分割して encode した連結 == 全体を 1 回で encode した結果 (1 文字単位を含む)、(c) 出力は常に ASCII 状態で終わる (`has_pending_state() == false`)、(d) 各行末 (LF / CR の直前) は ASCII 状態 — 保存方式は **行ごとにエンコーダを閉じる** ([OQ-162](10-decisions.md#oq-162-iso-2022-jp-保存時の行末の状態と-s-01-不合格時の退路)、確定 (ユーザー回答))。WHATWG (encoding_rs) のエンコーダはローマ字状態 (`¥` / `‾` の出力後) では改行を含む ASCII を状態を変えずに出力するため、エンコーダの出力のままでは成り立たない (`"¥\nabc"` → `ESC ( J 5C 0A 61 62 63 ESC ( B`)。`"¥\nabc"` を固定入力として、期待バイト列 `ESC ( J 5C ESC ( B 0A 61 62 63` を検査する、(e) 行単位の区分 B 件数 == 「全体を比較して異なる行」の数、(f) 判定 (03 §8.2 手順 4'): 正規形で encode したバイト列 (非 ASCII を含む) は常に ISO-2022-JP と判定され、ANSI エスケープ (`ESC [`) だけを含む 7 bit 列は判定されない | 表現可能文字集合の文字列 × 分割位置 / 7 bit バイト列 |
| eve-text 表示幅 / 折り返し | 折り返し後の各行幅 ≤ 幅。全セルの合計 = 元の幅合計。曖昧幅設定 1/2 で単調。`RenderRow.width_risky == 行内に is_width_risky な書記素がある` (INV-F3) | 文字列 + 幅 (1..=300) |
| eve-text 単語境界 (03 §13.3、文字種遷移規則) | 境界が単調・書記素境界上・パニックなし。**境界は必ず文字種 (英数字 / 漢字 / ひらがな / カタカナ / 全角英数 / 記号 / 空白) の遷移点にあり、長音符「ー」・踊り字「々」は直前の分類に続く**。03 §13.3 の期待表は L0 のゴールデンテストとして **全件一致** (01 FR-302 AC-1、OQ-117)。1 MiB 超の行では char 近似モード (03 §11.5) と厳密モードの結果が ASCII 行で一致 | 任意 Unicode 列 (CJK 混在を重み付け) |
| eve-config | `parse(serialize(cfg)) == cfg`。未知キー・公開前のキーは警告 / 情報診断のみで既定値 (FR-703)。**任意のバイト列を設定ファイルとして与えてもパニックせず、エラーは `ConfigError`** (v0.1 は `fuzz_config` の代わりにここで検査) | 設定値の生成器 / 任意バイト列・壊れた TOML |
| eve-core 入力列 (v0.1 は `fuzz_editor` の代わり) | ヘッドレス `Editor` に任意の `InputEvent` / `CommandInvocation` の列 (Text・Key・Paste・Resize・パレットの操作・確認ダイアログの選択を混ぜる) と任意 Unicode の初期文書を与えても **パニックしない**。各ステップで Frame を生成し、不変条件 (Selection が範囲内、Revision が単調、Frame の全 Span の幅 = 宣言幅) と無害化 (NFR-503: Span のテキストに `\x00..\x1f`・`\x7f`・`\u{80}..\u{9f}` が無い) を検査する。パニックした場合の最後の受け止めは緊急退避 (NFR-302) | `InputEvent` 列の生成器 + 初期文書 + 画面サイズ |
| eve-host 受信口 (02 §9.2 (1)〜(4)、MR3) | 任意の入力バースト・進捗列・検索結果列に対し、受信口に届くメッセージが生産側で集約されている: `Text` は 64 KiB 以下に合体、進捗は 10 Hz 以下、検索結果は 256 件 / 20 ms 単位、`debug` ログは 1 万件で警告に縮退。受信口の滞留長が入力量に比例しない | ランダムな入力列 + 手動時計 |
| eve-core キーマップ (02 §7.2 K1〜K8、07 §7.1 のコンテキストの参照順) | **コンテキスト**: フォーカスごとの参照順 (07 §7.1) で解決し、`search` の `enter` / `up` / `down` が `prompt` の `ui.confirm` に、`list` の `ctrl+p` が `global` の `ui.palette_open` に負けない。確認ダイアログ (`dialog`) では `global` の割当 (`mod+q` / `mod+p` / `f1`) が解決されない。チョードの保留中と `dialog` では単一書記素の `Text` が `Key{Char}` として照合され (アクセラレータ・2 打目)、それ以外の `Text` はキーマップを通らない。任意のキー列に対し解決が停止する。**純プレフィックス (完全一致を持たないシーケンス) の保留はタイムアウトせず、`Esc` で取消される** (時間を進めても状態が変わらない)。**v0.1 の実装範囲は組込みキーマップと `Ctrl+K` プレフィックス** (02 §7.2) で、曖昧 (完全一致かつプレフィックス) の `chord_timeout_ms` 経過で完全一致側が実行される性質はユーザーキーマップのチョードと同時 (v0.4。ユーザーの `[keys.*]` は v0.2 から単一ストロークのみ、v0.1 は `[keys]` を読まない)。不一致で保留が破棄され未割当通知が出る。`Text` は `captures_text` 層が無ければキーマップを通らず挿入される。FR-606 の到達性は L3 | キー列 + Clock 進行 |

- proptest の既定ケース数 256、CI では `PROPTEST_CASES=1024` (夜間は 10,000)。失敗ケースは `proptest-regressions/` にコミットして再発防止。
- 回帰ファイルを **削除してはいけない** (レビューで検査)。

### 3.3 L2: fuzz (cargo-fuzz / libFuzzer)

| ターゲット | 入力 | 性質 (NFR-305) |
|---|---|---|
| `fuzz_decode` (**v0.1**) | 任意バイト列 + 文字コードラベル (ISO-2022-JP を含む) + sniff 長 + チャンク分割位置 | パニックなし。判定 (03 §8.2) を通してから復号する。往復は §3.2 の 2 性質のうち「バイト → テキスト → バイト」(v0.1 は区分 A の一致と区分 B の検出、v0.3 で保持方式) のみを検査 (「テキスト → バイト → テキスト」は fuzz では表現不能文字を除外できないため L1 で)。ISO-2022-JP では状態を引き継いだ再エンコードと行単位比較がパニックせず、復号できない入力が不正バイト数に数えられ `read_only` になる |
| `fuzz_vt_input` (**v0.1 は置かない**。Unix (v0.2) で自前の VT 入力パーサを持つ場合に再導入する) | 任意バイト列 (ESC・CSI・SS3・OSC の途中で切れた列、ブラケットペーストの開始・終了 (`ESC[200~` / `ESC[201~`) の欠落と入れ子、UTF-8 の途中で切れた列、巨大な数値引数、`read` の区切り位置を任意に変えたもの) | Windows (v0.1) の入力は `ReadConsoleInputW` の自前ループで `KEY_EVENT_RECORD` 等の構造体として受け、VT のバイト列を解釈しないので、VT 入力パーサを持たない (OQ-33、04 §4.8)。Windows の入力の正規化とバースト判定 (04 §6.2 / §6.6) は、入力レコードの列と読取の区切り・間隔を任意に与える L1 の proptest で検査する (パニックしない、バースト中の制御文字がコマンドにならない (FR-206 AC-2)、バースト中の 0x09 / 0x08 / 0x1B / 0x0D が仮想キーコードの `Key{ Tab }` / `Key{ Backspace }` / `Key{ Esc }` / `Key{ Enter }` にならず `UnicodeChar` から本文 (0x0D は改行) になる (04 §6.6 の「バースト中の本文の作り方」)、IME の確定文字列が `Text` になる)。再導入するときの検査: VT 入力パーサ (バイト列 → `InputEvent` 列) がパニックせず、未完のシーケンスの保留に上限があり、同じバイト列を任意の位置で分割して与えても同じ `InputEvent` 列になり、ブラケットペーストの本文中の制御文字が `Paste` の本文として扱われる |
| `fuzz_word_boundary` (v0.2) | 任意 Unicode 列 | 03 §13.3 の境界合成がパニックせず、境界が書記素境界上にある |
| `fuzz_config` (v0.2 以降の候補) | 任意バイト列 → TOML | パニックなし、エラーは `ConfigError`。v0.1 は L1 の proptest (§3.2 の eve-config) で代える |
| `fuzz_keymap` (v0.2。ユーザーの `[keys.*]` と同時) | 任意文字列 → キー表記パーサ (07) | パニックなし |
| `fuzz_editor` (**v0.2**。キー入力列。v0.1 は L1 の「eve-core 入力列」で代える) | `arbitrary` で生成した `InputEvent`/`CommandInvocation` 列 + 初期文書 (任意 Unicode) + ビューポートサイズ | ヘッドレス `Editor` がパニックしない。**各ステップで Frame 生成まで行い**、不変条件 (`debug_assert` 群: Selection 範囲内、Revision 単調、Frame 内の全 Span 幅 = 宣言幅) と制御文字の無害化 (NFR-503: Span テキストに `\x00..\x1f`, `\x7f`, `\u{80}..\u{9f}`, 孤立サロゲート相当がない) を検査する |
| `fuzz_changeset` (v0.2) | ChangeSet の列 | compose/invert がパニックしない |
| `fuzz_regex_search` (v0.3) | パターン + 文書 | `regex` は線形時間保証。v0.1 の検索は行単位 (03 §12.3) でチャンク跨ぎが無いため対象外。regex-cursor を採用した場合 (S-04、v0.3) に境界条件を追加 |
| `fuzz_path` (eve-platform、v0.2) | 任意バイト列 → パス正規化 (05 §3.2) | パニックなし。Windows 予約名判定 |
| `fuzz_lsp_json` (v1.L) / `fuzz_ai_stream` (v1.L) | 任意 JSON / SSE 断片 (`data:` 行の途中で切れた断片、keep-alive コメント行、`[DONE]` の欠落、llama-server 固有の `timings` / `tokens_cached` フィールドの型違い、OpenAI 互換の `choices` 欠落) | パニックなし。AI 応答のエスケープ無害化 (02 §15)。不正な断片は `AiError::Protocol` で要求全体が失敗し、部分的に受けた本文は Proposal にならない |

- **実行環境は Linux のみ** (libFuzzer は MSVC 非対応、macOS は可だが高コスト)。`cargo fuzz` は nightly が必要 → 夜間ジョブ (§6.3)。**v0.1 は `fuzz_decode` (デコード) の 1 ターゲット × 10 分 (夜間) / リリース前 1 時間** (01 NFR-305 の v0.1 ゲート。キー入力列 (`InputEvent` 以降)、Windows の入力レコードの正規化とバースト判定、設定は L1 の proptest で代える。Windows の入力は VT のバイト列を解釈しないので `fuzz_vt_input` は置かない、OQ-33)。他のターゲットは v0.2 以降に追加し、全ターゲット 20 分 (夜間) / リリース前 2 時間に広げる。v0.1 は Windows だけを出荷するが、fuzz は Linux の夜間ジョブで回す (`fuzz_decode` が対象とする eve-encoding は OS に依存しない)。
- 最小コーパスは `fuzz/corpus/<target>/` にコミット (最小化後 `cargo fuzz cmin`)。夜間の実行で増えた分は Actions のキャッシュに持ち越し、コミットしない (§6.3)。クラッシュ入力は暗号化した artifact でメンテナが受け取り (§6.3)、最小化して回帰テスト (L0) に転記。
- Windows 固有のパス処理は libFuzzer が使えないため、v0.1 は **proptest で Windows 上で** 実行する (`fuzz_path` は v0.2) (`arbitrary` と `proptest` の二重ハーネスを `eve-testkit` が提供)。
- OSS-Fuzz 登録は Could (v0.4 以降、知名度次第)。

### 3.4 L3: ヘッドレス統合テスト (`eve-testkit`)

core を実機なしで駆動する中核のテスト層。同じシナリオを次の経路で実行できるようにする (Q8、02 §17.2): (1) Rust の `Harness` (下記。**v0.1 はこれだけ**)、(2) シナリオ DSL (TOML) を `xtask scenario run [--print-frame]` で (ASCII の Frame を出力。LLM 向けのスクリーンショット。**v0.2**)、(3) 駆動 API `evoeditor --headless --rpc stdio --fake-ports` (JSON-RPC、02 §10.5。**v0.2 以降**。対話的な探索に使う)。v0.1 の Harness は `h.frame_text()` で同じ ASCII の Frame を出せるので、v0.2 で DSL を入れたときに (1)(2) の一致を確かめ、(3) は駆動 API を実装した版から加える (01 NFR-707 AC-3)。DSL の書式と使い方は 11 §6〜7 が正本で、本書は検査対象と CI での扱いだけを書く。

```rust
// 擬似コード
let mut h = Harness::new()
    .fs(MemFs::from([("/w/a.txt", b"hello\r\nworld")]))
    .clock(ManualClock::at(0))
    .caps(Capabilities::tui_default())
    .size(80, 24);
h.open("/w/a.txt");
h.keys("ctrl+end shift+home");        // 07 §7.2 の設定表記 (小文字・`+`・空白区切り) を使う
h.cmd("edit.delete_backward");
h.clock.advance(ms(3000));            // Undo 結合の境界を越える
h.cmd("file.save");
h.run_jobs();                         // ManualJobSpawner のキューに積まれたジョブを実行する
assert_eq!(h.fs.read("/w/a.txt"), b"hello\r\n");
assert_snapshot!(h.frame_text());     // insta: Frame をテキストグリッド化
```

| 構成要素 | 内容 |
|---|---|
| `MemFs` | メモリ上 FS (02 §4.5 の `FileSystem` を実装)。**障害注入**: in-place 書込 (03 §9.3 の規則、05 §4.4) の経路を **05 の手順の順** (乾式走査 → 外部変更の照合 → 保存中フラグ → 元ファイルを開く → 事前バックアップ → 領域確保 → 上書き。順序の正本は 05 §4.4) に注入する。(a0) **書く前の乾式走査** (全文を保存先の文字コードで符号化して表現不能の有無と出力長を確定する走査。in-place では必須) で表現不能な文字が見つかる (何も書かず、元ファイルのバイト列・長さ・更新時刻が不変であること。表現不能の確認 (03 §8.6 E1) が出ること)、(a1) 照合で `FileStamp` が違う (`Conflict`。保存中フラグもバックアップも作らないこと)、(a2) 保存中フラグの作成の失敗: **原子的保存** では警告 (`SaveWarning`) を積んで保存を続け、保存が成功すること。**in-place** では何も書かずに止まり、`BackupFailed` と同じ確認 (`ui.confirm.save.overwrite_without_backup`。既定はキャンセル) を経ること (中断を後から検出できないため。05 §4.4)、(b) 事前バックアップの失敗 (`BackupFailed`。何も書かないこと。続く確認 `ui.confirm.save.overwrite_without_backup` の既定がキャンセルであること)、(b1) 元ファイルを開くときの `Busy` (共有違反) を k 回返してから成功 (再試行の回数と間隔が 05 の規則どおりで、**再試行の後に開いたハンドルで `(mtime, len, FileId)` を照合し直し** (内容ハッシュは比べない、05 §4.4)、その間に外部で変わっていれば書かずに `Conflict` を返すこと)、(c) 書く前の領域確保で `DiskFull` (元ファイルが不変であること)、(c1) 成功時の後片付けの順序 (**保存中フラグを消してからバックアップを消す**。フラグを消した直後で止めた FS 状態から開いても報告が出ず、バックアップを消した後にフラグだけが残る状態が生じないこと、05 §4.4)、(d) 書込途中の失敗 (`InPlaceFailed { backup: Option<…> }`。`state_dir/backup` のバックアップが完全で、メッセージ `msg.save.in_place_failed` の引数にそのパスがあり、保存中フラグが残ること。バックアップなしの上書きを承認した場合は `backup = None` で、パスの無い文言になること)、(e) **乾式走査と本書込の出力長の食い違い** (2 回目のエンコードが乾式走査の `out_len` と違う長さを供給するエンコーダを注入する。03 §9.3 の「乾式走査と出力長」の不変条件違反): Port が書込を止めて `InPlaceFailed { backup }` を返し、(d) と同じくバックアップのパスが示されること、N バイト書込後に `DiskFull`、`write_atomic` の各ステップ (一時ファイル作成/書込/fsync/置換/dir fsync) で失敗、`Busy` を k 回返してから成功、保存中に `FileStamp` を外部から変更 (→ `Conflict`)、権限 `PermissionDenied`、`AtomicUnsupported`。「置換前に強制終了」は **プロセスを殺さず「ステップ i で中断した FS 状態」を生成** して検査 (NFR-301 のモデル検査)。**Windows の中間状態**: 「T なし + `.eve-bak-*` のみ」「T + bak」「tmp のみ」「tmp + bak」の各状態を生成し、v0.1 は保存ジョブの **その場の報告** (退避パス付き、03 §9.3) と、**次に開いたときの警告** (開くたびに T の有無に依らず `inflight/<hash(path)>/` だけを 1 回調べ、T が NotFound で記録が残っていれば `msg.open.interrupted_save` に `.eve-bak-*` のパスが入る。T があり記録が原子的保存 (`method = Atomic`) なら報告しない。ディレクトリ全体を列挙しないことも検査、05 §4.6)、**in-place の中断状態** を 02 §4.5 の不変条件 (元ファイルが途中まで上書きされるのは、フラグの `backup_complete` が真になった後だけ。バックアップなしの承認 (`SkipConfirmed`) の再投入を除く) に沿って生成し、次に開いたときの報告を 02 §4.5 の「中断の痕跡の写像」の行ごとに検査する: (1) `backup_complete` が真 (`state_dir/backup` に完全なバックアップがあり、元ファイルが途中まで上書きされた状態): `msg.open.interrupted_save` (開くジョブの結果 `Complete.interrupted_save` の `complete = true`) がそのバックアップのパスを報告する。(2) 偽で一時名 `<最終名>.tmp` が在る (バックアップの `.tmp` を書いている途中で中断。元ファイルは不変): バックアップが不完全であることを示す別のキー `msg.open.interrupted_save_incomplete` (`complete = false`、04 §15.2) になり、`{backup}` が実在する `<最終名>.tmp` を示す。(3) 偽で一時名も無い (バックアップの前で中断。元ファイルは不変): 報告せず、開いた後に保存中フラグだけが消え、元ファイルがバイト一致する。(4) バックアップの記録が無い (`backup = None`。バックアップなしで上書きを承認した後の中断。元ファイルが途中まで上書きされた状態): `msg.open.interrupted_save_no_backup`。(5) **「上書きの可能性あり」の行**: `InPlace` / true / true なのに最終名のバックアップが消えている記録、写像表に無い不正な組合せの記録 (例: `backup_recorded` が偽で `backup_complete` が真)、解析できないフラグ (壊れた TOML・必須のキーの欠落) のそれぞれで、写像表のその行の `InterruptedSave` と案内のキー (元ファイルが上書きの途中で中断した可能性があること) になり、報告しない記録として黙って掃除されないこと。新規保存 (`method = NewFile`) の中断で T が無い状態では `msg.open.interrupted_new` (`complete = false`) になり、その引数が 04 §15.2 の目録の文言と一致すること (`InterruptedSave.tmp` は残った一時ファイルのうち実在するもの、02 §4.5)、ダングリングのシンボリックリンク経由で開いたときもリンク先のキーで記録が見つかること、報告しない記録 (T があり `Atomic` / `NewFile`) は、記録された一時ファイル・退避名がどれも無ければ開いた後にフラグだけが消え、どれかが実在すれば報告せずにフラグが残ること (05 §4.6 の「フラグを消す条件」)、**開くたびに** `inflight_for(path)` が複数の記録 (`Vec`) を返し得ること (同じファイルの記録が 2 件ある場合を含む)、v0.2 は保存中フラグの `list_inflight()` → 復旧提案 (03 §10.3 / 05 §4.6) を検査。`ReplaceFileW` の各エラー (`ERROR_UNABLE_TO_MOVE_REPLACEMENT_2` 等) は実 OS では再現しにくいため、**eve-platform の Win32 呼出し層を薄い trait にしてモックで注入** し、05 §4.2 の分岐を単体テストする (`ERROR_UNABLE_TO_MOVE_REPLACEMENT` → `AtomicUnsupported`、`ERROR_UNABLE_TO_REMOVE_REPLACED` と `ERROR_SHARING_VIOLATION` → 05 §4.2 の間隔 (待ちの合計 2.55 s) で再試行し、k 回の後の成功と、尽きたときに T が無傷なら `AtomicUnsupported` (確認付きの直接上書きへ。`Busy` にならない) を別のテストにする、`ERROR_UNABLE_TO_MOVE_REPLACEMENT_2` → `ReplacedMovedToBackup`)。検査項目: `ReplaceFileW` が失敗したとき `MoveFileExW` による置換を **呼ばない** こと (呼出し記録で確かめる)、`ReplaceFileW` の `dwReplaceFlags` が 0 であること (`REPLACEFILE_IGNORE_MERGE_ERRORS` も `REPLACEFILE_IGNORE_ACL_ERRORS` も付けない。呼出し記録のフラグで確かめる。05 §4.2)、属性・ACL のマージの失敗 (T 無傷) を注入すると `AtomicUnsupported` になること、置換の後の `metadata(T)` の FileId が一時ファイルのものと違う注入で、`new_stamp` が書いた内容の値になり警告が積まれ、次の保存が `Conflict` になること (05 §4.1 手順 7)、確認 (`ui.confirm.save.atomic_unsupported`) が出て、**既定の選択肢が「別名で保存」** であること (既定で `Enter` を押すと名前の入力に進み、元ファイルが不変で一時ファイルが片付く。キャンセルでも元ファイルは不変)、「上書きして保存」を承認すると in-place の経路 (乾式走査 → 照合 → 保存中フラグ → open → バックアップ → 領域確保 → 上書き) に進むこと。L3 でも同じ流れを `MemFs` で `AtomicUnsupported` (detail なし。03 §9.3 手順 7 の「上記以外」の分岐で、`ReplaceFileW` が元ファイルを変えずに失敗した場合に当たる、02 §4.5) を注入して、確認 (`ui.confirm.save.atomic_unsupported`) の MessageKey と選択肢 ID で検査する。置換の途中で失敗して元ファイルが退避名に移った場合は `ReplacedMovedToBackup { backup }` を注入し、退避パスが示されることを検査する。実 FS での kill テストは L5。**退避ディレクトリ** (`write_recovery` / `list_recovery_dirs`、02 §4.5): 壁時計と pid・SID はこの行が正本で、名前の `<UTC timestamp>` と `created_at` は Harness の now (シナリオの開始時刻、11 §6.1)、pid と SID は Harness が与える固定値を使う。名前は 05 §7.5 の「退避ディレクトリの名前の形」で作り、同名が在れば `-<n>` を付ける (同じ now で 2 回作ると 2 回目に `-2` が付く)。`state_dir` を使えない障害注入 (`state_dir` の作成・書込の失敗) では代替の退避先 (`%TEMP%\eve-recovery-<SID>\` に当たる `MemFs` 上の場所) に作って `alternate = true` を返す。`list_recovery_dirs` は `state_dir` と代替の退避先を同じ規則で走査し、名前の形に合わないエントリと `inflight/` を返さず、中のファイルを読まない (読込の記録で確かめる) |
| `FakeClipboard` / `RecordingHost` | Effect を記録。`SetClipboard` の内容、`Quit`、`Bell` を assert。`FakeClipboard` は **失敗と遅延を注入** できる: 取得・設定の失敗 (クリップボードを開けない、`GetClipboardData` が失敗) と、取得の遅延 (手動時計で N ms 後に応答)。FR-205 AC-2 (失敗時は内部レジスタへ切り替わり、`msg.clipboard.fallback` が最初の 1 回だけ出る) と AC-3 (取得待ちの間も入力が処理され Frame が更新される) を L3 で検査する。**FR-205 AC-4 (a)**: 100 MB の文書で全選択してコピーすると、選択範囲の **総バイト数** で上限 (64 MiB、05 §8.4) を判定して内部レジスタだけにコピーし、通知 (04 §15.2 の MessageKey) が出て、`FakeClipboard` に `SetClipboard` が届かないこと。判定が **文字列化の前** であることは、`ThreadGuard` の MR5 の検出 (メインスレッドでの 64 KiB 超の `String` 生成) が起きないことと、`SetClipboard` 用の文字列化の呼出しが記録されないことで確かめる |
| Host の組み立て (`HostBuilder`、02 §9.2) | Harness は `Host` を **`HostBuilder`** で組み立て、部品を偽物に差し替える (部品ごとの別の口を作らない): Port (`MemFs` / `ManualClock` / `ManualJobSpawner` / 一時ディレクトリの `Paths`)、`ClipboardProvider` (`FakeClipboard`)、ログの出力先 (`LogSink`。メモリに書き、NFR-504 の検査で読む)、緊急退避の書き手 (`RescueWriter`。同じ `MemFs` に本番と共用のシリアライズ関数で書く偽物)。本番の書き手は `std::fs` で書くので `MemFs` の起動時の走査 (`RecoveryScan`、`FileSystem` Port で読む) からは見えないが、偽物の書き手を使えば **退避の往復** (パニックまたは `QuitRescue::DirtyDocuments` で退避 → 同じ `MemFs` で Harness を作り直して起動 → `msg.recovery.found` の `{dir}` / `{count}`) をヘッドレスで検査できる。本番の書き手の `std::fs` の部分は eve-platform の L0 テスト (`tempfile`) と M3 の kill テストで検査する。クリップボードの書込の失敗が core に届く経路 (`HostReport::ClipboardWriteFailed`、02 §5.4) も、この Harness で `FakeClipboard` に失敗を注入して検査する |
| Win32 クリップボード層 (eve-platform、FR-205) | `OpenClipboard` / `GetClipboardData` / `SetClipboardData` / `CloseClipboard` の呼出しを **薄い trait** にし、L0 でモックを差して、失敗 (開けない・取得の失敗・設定の失敗)・開く再試行の回数と間隔・遅延 (応答の遅いモック) の各分岐を検査する (`ReplaceFileW` の層と同じ方式)。実クリップボードの遅延は、補助プロセス **`clip-delay`** (bin crate **`eve-test-helpers`** の bin。`publish = false`、feature を持たず、test-hooks のビルドのテストからだけ使う。配布物に入らない) で作る: `--hold <ms>` はクリップボードを開いたまま保持して他プロセスの取得を失敗させ、`--delay-render <ms>` は遅延レンダリング (`SetClipboardData(CF_UNICODETEXT, NULL)`) で登録し、`WM_RENDERFORMAT` への応答を遅らせて取得を遅くする。eve-platform の実クリップボードの L0 (Windows、`--hold` で取得の失敗から内部レジスタへの切替、`--delay-render` で取得の待ちがメインスレッドを止めないこと) と、BM-103b の負荷下の判定 (FR-205 AC-3) で使う |
| `ManualClock` | 手動進行 (時刻は 02 の `Clock` の `Tick` 型で、経過は `Millis` で進める。`Host::step` が各メッセージの処理の前に tick を進めること (02) も検査する)。Undo 結合・曖昧チョードの deadline・swap-lite の間隔 (v0.2) を決定的に。純プレフィックス保留が時間経過で変わらないことも検査 |
| `ManualJobSpawner` | テスト用の `JobSpawner` (02)。投入されたジョブは実行せずに **キューに積み**、テストが `run_jobs()` を呼んだときに投入順に実行する (`run_job(req, &JobPorts, &CancelToken, emit)` を呼び、`emit` の結果を受信口に積む)。`JobId` は本番と同じく spawner が採番する。ジョブ結果の **Revision 不一致** を意図的に作る `run_jobs_stale()` (定義は 02。キューのジョブを、文書を 1 回編集してから実行する): 古い結果が破棄/写像されること (02 §5.3) と、置換の ChangeSet (`JobKind::ReplaceUnmappable`) が作り直されること (03 §8.6) を検査。キャンセル (`CancelToken`) を実行前・実行中に立てる注入も持つ (**保存ジョブはキャンセルを立てた後に `Saved` で完了したらその結果を採用する** (ディスクは書き換わっているので、dirty と `FileStamp` を保存後の状態にする。02) ことも検査)。**保存の取消しのシナリオ** (02 §4.5 の「取消し」。main は取り消すとき常に `CancelToken` を立てる): (a) **乾式走査の途中の取消し**: `ManualJobSpawner` の注入で乾式走査の途中に取消しを立てると、ジョブが `JobError::Cancelled` を返し、`MemFs` の T のバイト列・長さ・更新時刻が不変で、保存中フラグ・一時ファイル・バックアップが残らない。(b) **置換の開始の後の取消し**: `ReplaceFileW` (`MemFs` では置換の呼出し) の開始の後に取消しを立てても置換が完了し、`Saved` が返って採用される (`Cancelled` にならない)。(c) **`Atomic` から直接上書きへ切り替わったジョブ** (03 §9.3 手順 5d、`InPlacePattern` による切替): 上書き (05 §4.4 手順 6) の開始の後に取消しを立てても本書込が完了し、エンコーダが `Aborted` を返さず、`Saved` が返って採用される (`InPlaceFailed` にも `Cancelled` にもならない)。事前バックアップの書込の途中の取消しでは書かずに `Cancelled` で終わり、T が不変で一時名のバックアップと保存中フラグが片付くことも検査する。(d) **`Conflict { current: None }`**: `Expect::Stamp` で投入した保存の前に `MemFs` から T を削除すると、ジョブの照合が `Conflict { current: None }` を返し、`ui.confirm.external_change` で「上書き」を選ぶと `Expect::Absent` で再投入されて新しく作られる (03 §9.3)。(e) **`CheckingUnchanged` の照合の失敗**: 無編集保存の照合ジョブの `metadata` に `Busy` / `PermissionDenied` を注入すると `msg.save.check_failed` (`{path}`・`{reason}`) が出て保存が中止され (何も書かない)、保存の連鎖 (`app.quit` の保存) では後続の終了も中止される。`NotFound` を注入した場合は `check_failed` にならず (d) と同じ `Conflict { current: None }` の経路に進む (03 §9.3)。本番の `JobSpawner` のワーカーについては、ジョブの実行中 (`IN_JOB`) のパニックでパニックフックが `Fatal` を送らず、そのジョブの失敗として扱われることを eve-host の L0 で検査する (02) |
| `ThreadGuard` | `ManualJobSpawner` とは **別の機構** の検査モード。印 (「いまメインスレッドの処理中か」を示すスレッドローカル) とその API は **eve-core** に置き (偽 Port と eve-core の両方から見えるように)、`Harness` がその印を立て、偽 Port (`MemFs` 等) の各メソッドが印を見て、**メインスレッドから `FileSystem` が呼ばれたら panic** する (02 §4.4 規則 2 / ADR-0020 の自動検査。ジョブは `run_jobs()` の中で印を下ろして実行する)。同じ印で **メインスレッドでの 64 KiB 超 `String` 生成 (クリップボード文字列化、MR5)** と **ワーカーと共有する `Mutex` の取得 (MR9)** を `debug_assert` で検出 |
| `FakeWatcher` (v0.4) | `FsEvent` を手で注入 |
| `RecordingProcessRunner` (v0.4) | 呼び出しを記録し、**承認前に呼ばれたら即 panic** (P2 原則の自動検査)。**実行ファイル探索の回帰テスト**: `tempdir` のワークスペース (cwd) と PATH の相対要素・空要素に偽の `rust-analyzer` / `rustfmt.cmd` を置き、05 §13.3 の `resolve_executable` がそれらを **選ばない** こと (絶対パスの PATH 要素のみ) を 3 OS で検査。Windows では `.bat` / `.cmd` も同様 |
| `FakePathsOwner` (v0.4。ワークスペース層の設定と同時) | ワークスペース設定の所有者検査 (07 §12.1) 用に、ディレクトリの所有者・world-writable 属性を偽装し、他者所有の `.eve/config.toml` / `.editorconfig` が読まれないことを検査 |
| **`HeadlessFrontend`** (v0.1) | `Frontend` trait (02 §9.2) の **テスト用実装**。`init` (`FrontendConfig` を受け取る) で `Capabilities` をプリセット (`tui_default` / `tui_16color` / `tui_mono`、および端末の横取り表 (04 §7.3a) を入れた `tui_<端末>`) から返し、`render` で Frame を `CellOccupancy` に落として保持、`apply_effect` は記録。TUI 以外の **第 2 の Frontend** として、`Frontend` trait と `Host::step` が TUI 専用に歪むのを防ぐ。`CellOccupancy` から 11 §6.5 の ASCII Frame を生成でき、v0.1 のシナリオのスナップショットに使う |
| **`SteppingHarness`** (v1.0 まで) | `Host::run()` を使わず、外部のイベントループに見立てたテストコードから `Host::step(msg)` を 1 メッセージずつ呼ぶハーネス。外部のイベントループが `Host::step` を駆動する経路を検査する。同じシナリオを `run()` と `step()` で流して Frame 列が一致することを検査。v0.1 は「`run()` を `step()` のループとして実装する」規則 (02 §9.2) をレビューで確認する |
| **`MockLlamaServer`** (v1.L、`eve-testkit` の feature `ai`) | ローカル LLM サーバの **模擬** (06 §16 の MockProvider より外側: 実 HTTP をループバックで話す)。詳細は §3.9。OpenAI 互換 (`/v1/chat/completions` (SSE)・`/v1/completions`・`/v1/models`) と llama-server 固有 (`/completion`・`/infill`・`/tokenize`・`/detokenize`・`/health`・`/props`・`/slots`・`/apply-template`) の **最小実装**、カセットの記録再生、遅延・切断・部分 SSE・エラー (503 loading / 429 busy / 400) の注入、決定的 seed。std の `TcpListener` + 1 スレッド (tokio 不要。テスト側の依存を増やさない) |
| **`ManagedServerSim`** (v1.L-b) | 管理モード (FR-918、05 §13.7) の子プロセスを模擬する **実行ファイル** (`eve-testkit` の bin)。引数で「`/health` が OK になるまでの遅延」「起動直後にクラッシュ」「`--host` を無視して 0.0.0.0 に bind しようとする (evoeditor 側が拒否することの検査)」「SIGTERM を無視する (強制終了経路の検査)」を切り替える。`RecordingProcessRunner` と組み合わせ、承認前に起動されないこと・終了時に残らないことを 3 OS で検査 |
| シナリオ (**一級成果物**、NFR-707) | **v0.1 は Rust の `Harness` のテスト** (各 crate の `tests/` に置く。例 `crates/eve-core/tests/undo.rs`。リポジトリ直下の `tests/scenarios/` は DSL と同時の v0.2、11 §1)。1 つのテストが 1 つのシナリオで、`verifies:` (AC ID) を関数の直前に書き、初期状態 (偽 FS の内容、設定、Capabilities、画面サイズ)・操作 (キー列は 07 §7.2 の設定表記、コマンドは CommandId、時計の進行、ジョブ実行)・検査 (`Query` の結果、ASCII の Frame、`EditorEvent` 列、偽 FS の内容) をこの順に書く。frontend を知らない。**TOML のシナリオ DSL** (`tests/scenarios/**/*.toml`、`[meta]` / `[setup]` / `[[step]]` / `[[assert]]`。書式の正本は 11 §6) と `xtask scenario run` は **v0.2** で、そのとき v0.1 の Rust のシナリオを DSL に移せるよう Harness の語彙を DSL と同じにしておく。手動確認手順 (M) と同じ記法で書けるようにして M→T への昇格を容易にする |
| UI の文言 (NFR-603) | v0.1 の UI は `ja` だけなので、全シナリオを `ja` で実行する (言語の切替は無い)。シナリオの検査は文言ではなく `Query` が返す MessageKey と引数で行う (11 §6。後の版で言語が増えてもシナリオを直さずに済む)。文言そのものを見るのは **スナップショット** だけ: 主要画面の各 1 枚以上 (通常画面、コマンドパレット (各行の割当キーを含む。ヘルプ画面の代用、FR-514)、確認ダイアログ (未保存の終了・置き換え)、検索バー、エラーメッセージ (保存失敗)、狭い端末 (40 列) のステータスバーとヒントバー)。クローム (ステータスバー・ヒントバー・検索バー) の省略と圧縮は TUI の仕事なので、そのスナップショットは L4 の VirtualScreen で撮る (§3.5)。ヘッドレスの ASCII Frame のクロームは省略しない参照表示で、TUI と差があってよい。スナップショットの名前は v0.1 から言語を含む `<名前>.ja.snap` にする (11 §6。v0.1 の言語は `ja` だけ)。英語 UI を同梱する v0.2 (確定 (ユーザー回答)、OQ-21) で、`<名前>.en.snap` と全シナリオの言語別の実行を加える (v0.1 のスナップショットの名前は変えない) |
| Frame スナップショット | `Frame` を幅固定のテキストグリッド (文字 + 別レイヤでスタイル記号) にシリアライズし `insta` で比較 (`h.frame_text()`。v0.2 の `--print-frame` と同じ ASCII 表現)。`eve-view` の型は `#[non_exhaustive]` と `CONTRACT_VERSION` の定数を v0.1 から持つ。insta の `accept` は人間のレビュア (11 §4) |

**保存・読込の競合と痕跡のシナリオ (L3、v0.1)**: `MemFs` の障害注入と `ManualJobSpawner` で次を検査する。(1) **保存キューと失敗** (03 §9.3 手順 4): 単独の保存の実行中に `file.save` をもう一度実行して要求を積み、前の保存に `Conflict` / `Busy` / `JobError::Unmappable` / `Cancelled` を注入すると、積んだ要求が再実行されず (2 つ目の保存ジョブが投入されない)、`AfterFailure` の確認だけが出る (`Cancelled` では確認も出ない)。確認で「上書き」/「再試行」を選ぶと、その時点の最新の Snapshot (積んだ時点の後の編集を含む) で 1 回だけ投入される。`AfterFailure` の表示中の `file.save` が積まれない。前の保存が `Saved` なら積んだ要求が `Expect::Stamp(new_stamp)` で 1 回だけ再実行される。(2) **保存中の置き換え** (03 §9.7 の全状態の遷移表の `file.open` / `file.new` の列): 単独の保存の実行中に `file.open` を実行すると、保存の完了まで新しい文書の読込ジョブが投入されず、`Saved` で dirty でなければ置き換えが進み、保存中に編集して dirty なら `ui.confirm.unsaved_replace` が出る。保存が失敗すると `AfterFailure` が出て、置き換えは確認で保存を続けて成功したときだけ行われる。置き換えの読込中に旧文書を編集しようとすると `msg.edit.read_only` で拒否される。`AskUnsaved` で「破棄」を選んだ置き換えで、旧文書を閉じるときに FR-107 の確認が 2 回目に出ない。保存ジョブの実行中に閉じた文書の保存が失敗すると、保持した Snapshot から「退避先に保存」ができ、その間は緊急退避のスロットが空にならない (`EmergencySnapshot` を読んで確かめる。03 §9.2 の「閉じる手順」、§10.1)。(3) **読込中の伸長・膨張** (03 §9.2 手順 3 の「読込中のメモリ検査」と「読む量の上限」): `MemFs` で読込の途中にファイルを伸ばす注入 (metadata の len は閾値未満、読込中に閾値を超える) で `ConfirmAsk::HugeFile` が出て本文が捨てられること、閾値を小さくした試験設定で、len は閾値未満でも Shift_JIS の倍率・不正バイトの比率から求めた見積りが閾値を超えると本文の構築の前に `HugeFile` が出ること、伸び続けるファイルが len + 余裕で打ち切られて `changed_during_load` が付き、そのまま上書き保存すると `Conflict` になって読んだ分だけでファイルが切り詰められないこと。(4) **一時ファイル・退避名の残存** (05 §4.1 手順 7 / 8、§4.6): 置換の成功の後に退避名 (`.eve-bak-*`) の削除を失敗させると、保存は成功して警告が積まれ、保存中フラグが残り、開いても報告されず (T がある)、フラグは開くジョブにも次の保存の成功にも消されない。置換の前の失敗で一時ファイルの削除を失敗させた場合も同じ。同じプロセスが同じファイルを続けて 2 回保存し、1 回目のフラグが残る状態で 2 回目のフラグが別の名前 (`<pid>-<seq>.toml` の `<seq>` が違う) で書かれ、1 回目の記録を上書きしない。in-place の成功の後、フラグを消してからバックアップを消す前に止めた状態 (フラグの無いバックアップ) が、v0.2 の起動時の走査で `state_dir/backup/` の列挙から孤児として見つかる (v0.2 の検査)。

**文言の照合 (C-01)**: 保存・読込・退避の各分岐 (03 §9.3 手順 7 の Conflict / AtomicUnsupported / BackupFailed / InPlaceFailed / Busy / ReplacedMovedToBackup / PermissionDenied / DiskFull / ReadOnlyFs / InvalidPath、無編集保存の照合の失敗 (`msg.save.check_failed`)、ハードリンクとダングリングリンクの確認、in-place の通知、表現不能・区分 B / D の確認、新規ファイルの保存先に同名のファイルがある場合 (`ui.confirm.save.target_exists`)、開くときの巨大・不正バイト・バイナリ・中断された保存・権限なし・ファイルでない・予約名・共有違反・読込の途中の失敗・読込中の変更・取消・親フォルダが無い新規ファイル、読み取り専用の文書への編集の拒否 (`msg.edit.read_only`)、未保存の文書の開き直しの確認、行番号の丸め、クリップボードの切替、起動時の引数 (フォルダ・予約のオプション) と設定の診断、起動時の前回の退避) は、`MemFs` の障害注入とフィクスチャで起こし、04 §15.2 の目録の MessageKey と引数で照合する (確認ダイアログは `Query` の `Confirm` の選択肢 ID とアクセラレータも)。目録にあって、どのシナリオからも出ないキーは CI で警告する (目録と実装の乖離の検出)。起動前・終了時の stderr とパニックの文言 (`stderr.*` / `panic.*`。v0.1 は日本語の行だけ、v0.2 から英語の行と日本語の行、OQ-67) は L5 (§3.6 (6)(7)) と CLI の統合テスト (下記。`stderr.usage.*`、端末でないときの `stderr.not_a_terminal` と終了コード 5) で出力の行を照合する。`stderr.unsupported_console` は VT を拒否するコンソールを CI で作れないので、eve-tui の初期化の失敗を模擬する単体テストで照合する。

L3 で必ずカバーする要件: FR-1xx (ファイル管理) の全 Must (FR-109 の Shift_JIS / EUC-JP / ISO-2022-JP の判定と編集後の保存、FR-113 の改行の保持と貼り付けの正規化 (AC-3。端末からの `Paste` に加え、システムクリップボード (`FakeClipboard` に `"a\r\nb"`) と内部レジスタ経由でも、CRLF 文書には `a\r\nb`、LF 文書には `a\nb` が入る。03 §8.4)、FR-124 の新規バッファ、**FR-107 AC-3 の置き換え** (単一文書なので `file.open` / `file.new` は今の文書を置き換え、dirty なら `ui.confirm.unsaved_replace` が出て、保存 / 破棄 / キャンセルのそれぞれで期待どおりになる)、**FR-111 AC-4 の開き直し** (Undo できず履歴が消え、`info.reopen.history_cleared` が出る) を含む)、FR-2xx/3xx/4xx の全 Must (FR-304 の PageUp / PageDown (可視行数 − 1 行ずつ) と代替キー `ctrl+k ,` / `ctrl+k .` を含む。FR-402 は **トグルのコマンド経由** で検査する: `search.toggle_case` で smart → sensitive → insensitive → smart と巡回し、バーを開き直すと既定値 (smart) に戻る。正規表現は v0.2 で、v0.1 では `search.toggle_regex` がパレットに出ないこと)、FR-502 (ガター幅)・FR-503 (ステータス項目。**値・長い形・短い形・優先度** は `Query` の `StatusItem` で検査し、80 桁未満の省略の結果は L4 で検査)・FR-504 (ヒントバーの項目と優先度は `HintModel` で検査。v0.1 は常時表示で、文脈は `normal` / `overlay`)・FR-505 (メッセージ種別)・FR-506 (水平スクロールと続きの表示)・FR-511 (オーバーレイの共通操作。パレット・プロンプトの `up` / `down` / `enter` / `esc`、**確認ダイアログ** の `tab` / `shift+tab` / `left` / `right` / `up` / `down` でのフォーカス移動と `enter`、アクセラレータ (`d` / `D` / 全角の `Ｄ`) での直接選択、一致しない文字を捨てること。FR-107 AC-1 の「破棄」をこの 3 つの経路のそれぞれで選べる) の自動化できる部分 (Query と ASCII Frame で検査。見た目の確認は L7)、FR-507 無害化、FR-514 (パレットに `hidden` 以外の全コマンドとその割当キーが出る)、FR-601 (`hidden` 以外の全コマンドをパレットから実行できる。07 §11.6 の `hidden` の一覧と `CommandSpec.hidden` が一致し、版注記付きの ID が登録されていない)/602 (部分一致と前方一致の順、割当キーの併記。**引数の尋ね方**: `file.reopen_with_encoding` をパレットから選ぶと文字コードのリストが出て、選ぶと開き直す (FR-111 にパレットから届く)。`Esc` で 1 段戻る)/603/606、**NFR-707 AC-5 (`drain_events`)**: 文書の変更 (入力・Undo・保存による dirty の変化)、メッセージの表示、オーバーレイの開閉 (パレット・確認ダイアログ・検索バー) を起こす操作の後に `Editor::drain_events()` が対応する `EditorEvent` (Document / Message / Overlay 系) を返し、v0.1 で発火するのがこの 3 系だけであること (`JobProgress` / `CapabilitiesChanged` 等が出ない。06 X17)。**NFR-707 AC-6 (`Query` の決定性)**: 同じ状態で同じ `Query` を 2 回発行して同じ応答になること (L1 の `Query` の決定性の性質と同じ観点をシナリオでも確かめる)。FR-701 (ユーザー層の `config.toml` の 7 キーの反映。公開前のキー・`[keys]`・`[os.*]` は情報診断で値を使わない。設定の分離は `EVE_CONFIG_DIR`)/702/703 (起動時の要約 `msg.config.problem` / `msg.config.problems`)、NFR-106 (閾値超の行を含む文書が読み取り専用で開き、その行が先頭 64 KiB まで表示され、`info.open.long_line` と `status.degraded.long_line.long` が出る)、NFR-303/304 (不正バイトの読み取り専用と可視化)/305、NFR-603 (言語の選択を行わない: `ui.locale` は情報診断、`EVE_LANG` は読まない。欠けたキーはキー名を表示する (欠落させた試験用の言語ファイル))、NFR-701。**CLI** (07 §18): 引数解析は L0 で検査し、FILE を 2 つ以上渡したときに TUI を始めずに `stderr.usage.multiple_files` を出して終了コード 2 になること (01 FR-101 AC-1) と、未知のオプションで `stderr.usage.generic` を出して終了コード 2 になることは、バイナリを子プロセスとして起動する統合テスト (pty 不要) で検査する。**実 FS が要るもの** (`MemFs` では意味がないため eve-platform の L0 テストを `tempfile` 上で Windows で実行): FR-106 (読み取り専用属性のファイルは読み取り専用で開き、編集が `msg.edit.read_only` で拒否され、名前を付けて保存しても元のファイルの内容と属性が変わらない (AC-1。「読み取り専用を解除して保存」で属性が戻ることは v0.2)、シンボリックリンク・ジャンクション経由の保存、NTFS のハードリンク (`nlink>1`) の確認、ダングリングリンクで何も書かず `msg.save.dangling_link`。Unix のモード・所有者 (AC-4) は v0.2)、NFR-404 (§4 のパスのコーパスのうち Windows の項目を開いて保存できる。Windows の項目は `skip!` を失敗として扱う)。**NFR-403**: `Capabilities` の色数・拡張キーをそれぞれ無効にしたプリセットで、FR-606 の到達性検査と v0.1 のシナリオが通る (01 NFR-403 AC-1)。**NFR-502**: モードライン・ワークスペース設定に外部コマンドを含むファイルを開いても、プロセス起動の Effect が記録されない (v0.1 はワークスペース設定自体を読まない)。**NFR-504**: 秘密を模した文字列 (API キー形式・固有の目印文字列) を含む文書を編集してから、ログの書き手を偽 FS に向けた Harness で `internal.debug_panic` に相当するパニック経路 (host の `catch_unwind`) を通し、ログとクラッシュログに文書本文と目印文字列が含まれない (01 NFR-504 AC-1)。**パニックの payload からの漏洩**: 標準ライブラリの文字列の切り出しのパニック (`&s[a..b]` で文字の境界でない位置を指したとき) は、メッセージに **切り出そうとした文字列そのもの** (先頭の一部) を含む。目印文字列を含む文書の本文 (と、それを含む行・選択範囲の文字列) に対して、test-hooks で文字の境界でない位置の切り出しを行わせてパニックを起こし (メインスレッドと `run_job` の中のワーカーの両方)、ログ・クラッシュログ・終了時の stderr に目印文字列が含まれないことを検査する (パニックフックと `catch_unwind` の後の記録は payload の文字列をそのまま書かず、型と発生位置 (ファイル・行) だけを書く、09 §10.2)。**FR-606 の到達性検査 (OQ-30)**: 07 §11.6 末尾の「頻出操作」リスト (CommandId で列挙した 37 個: 保存・終了・パレット・`ui.cancel`、検索 (`search.toggle_case` を含む)、Undo/Redo、コピー/カット/貼り付け、移動 (`cursor.goto_line` を含む)、選択 (`select.all` と選択拡張。`select.extend_to_doc_start` / `_end` は含めない)。版注記付きのコマンドは各版で加える。検査が読むリストは 07 §11.6 の頻出操作の箇条書き (読み方は 11 §8.5 の「頻出操作リスト」) で、本書は列挙を持たない) に対し、**v0.1 の対象端末 (Windows Terminal・conhost) のそれぞれ** について、`Capabilities.enhanced_keys = false` で、かつ **04 §7.3a の横取り表でその端末が横取りするキーを含まない割当** で到達できることを、既定キーマップを解決して自動検査する (チョードの代替割当を含む。`[E]` の割当 (`ctrl+h` の `search.replace_open` の別名等) は、Windows の `ReadConsoleInputW` 経路で区別されて届く見込みでも到達の手段に数えない (04 §6.2 の「影響」)。横取り表の「別のキーとして届くもの」の右辺はその端末で押せるキーとして扱う)。不合格のときは「端末・操作・候補の割当とそれぞれが届かない理由」を出す。横取り表は eve-tui の定数表 (`Capabilities.intercepted_keys` の元) と同じデータから読み、04 §7.3a の表と食い違えば失敗する。同じ検査で、パレットの各行・ヒントバー・`{key:…}` に出るキー表記がその端末で届く割当になっていること (04 §7.8。例 Windows Terminal で `select.extend_to_doc_end` に `Ctrl+Shift+End` を出さない) も確かめる。macOS / Linux の端末の行 (04 §7.3a) は、それらを出荷する版で検査に加える。その他の Must 機能は「パレットが開ける」ことで充足とみなす (パレット経由を含めると検査が自明になるため、リストに限定する)。01 §5 の該当 AC と対応づける。加えて 03 §9.2 の「開く」手順 (Loading → ジョブ → `FileId` の判定。開くたびの保存中フラグの照会 `inflight_for` は v0.1、起動時の `list_inflight` による一斉の走査は v0.2) と §9.3 の保存 (手順 0 の無編集保存の no-op、ジョブ内 `Conflict` (開いた後に変更・削除されたファイルは `ui.confirm.external_change` で「上書き / 別名で保存 / 中止」、新規ファイルの保存先に同名のファイルがあれば `ui.confirm.save.target_exists`)、E2' の確認、in-place のバックアップ失敗で既定がキャンセルになること (FR-105 AC-2)) をシナリオで網羅する。**E3 の置換 (03 §8.6、01 FR-112)**: 表現不能で保存を中止したときの「置換して保存」を次の 3 点で検査する。(a) 半角カナを含む文書を ISO-2022-JP で保存すると、半角カナが全角カナに置き換わって保存され、再読込で全角カナになっている。(b) それ以外の表現不能文字 (例: Shift_JIS での絵文字) は `?` に置き換わる。(c) 置換の前に対象の一覧 (位置・元の文字・置換後の文字) が示され、一覧を示した時点では文書と `MemFs` の元ファイルが変わっていない (キャンセルすれば何も変わらない)。承認後の置換は 1 つの Transaction で、Undo で元に戻る。(d) **件数の表示**: 表現不能が 150 件の文書で、一覧の `details` が内訳 (置換先ごとの件数) と先頭 100 件と「ほか 50 件」を持つ (03 §8.6 の「置換の一覧」、MessageKey と引数で検査)。(e) **選択肢 1 (UTF-8 で保存)**: 同じ保存先に UTF-8 (BOM なし) で書かれ、`TextFormat` は **保存が成功したときだけ** UTF-8 に確定する。保存を `MemFs` の障害注入で失敗させる・`Conflict` にする・確認でキャンセルする、のそれぞれで `TextFormat` が元のまま。エスケープ文字 (E4) を含む文書では選択肢 1 と 2 が出ない。(f) **選択肢 3 (キャンセル、既定)**: 文書が変わらず dirty のまま、カーソルが最初の表現不能の位置へ移り、Decoration が次の編集まで残る。保存の連鎖 (`app.quit` の保存) では後続の終了が中止される (03 §9.7)。(g) **置換の作り直し**: 置換の ChangeSet を作るジョブ (`JobKind::ReplaceUnmappable`) を `ManualJobSpawner::run_jobs_stale()` で文書の編集の後に完了させると、Revision の不一致で結果が適用されずにジョブが作り直され、作り直した結果の一覧と置換が編集後の文書に対して正しい。(h) **`file.set_encoding` の確定時機**: 指定した文字コードでの保存が成功したときだけ `TextFormat` が変わり、失敗・キャンセル・E3 の 3 では元のまま (03 §8.8)。(i) **選択肢 2 (置換して保存) が出ない条件** (03 §8.6 の「E3 の選択肢」): (i-1) **読み取り専用の文書** (置換は文書を編集するため。読み取り専用の文書を「名前を付けて保存」・`file.set_encoding` で表現不能になる文字コードに保存しようとした場合) では、確認の選択肢に「置換して保存」が無く (`Query` の `Confirm` の選択肢 ID で検査)、そのアクセラレータを押しても何も起きない、(i-2) **エスケープ文字 (E4、不正バイトの PUA エスケープ) を含む文書** では選択肢 1 と 2 が出ない ((e) と同じ)。どちらも残る選択肢で文書と `MemFs` の元ファイルが変わらないこと。**ダウングレードの退避 (v0.1、05 §18.11。確定 (ユーザー回答)、ゲートは G1c)**: テスト入力として、manifest の `format_version` を実装が知る最大の版より大きくし、**未知のキー** (将来の版が足すキーを模したもの。トップレベルと `[[document]]` の両方) を含む退避ディレクトリのフィクスチャ (`manifest.toml` + 本文の `.txt`。置き場所は 11 §6.2) を用意し、`MemFs` の `state_dir/recovery/` に置いて起動する。検査項目は次の 3 つ: (a) 本文を読まない (その `.txt` の読込が偽 FS に発生しない)、(b) 退避の場所を示す (v0.1 は manifest を読まないので版を区別せず、`msg.recovery.found` が出て `{dir}` がそのフィクスチャのディレクトリ、`{count}` が 1 であることを MessageKey と引数で検査する。02 §9.3。v0.2 の復旧 UI では `msg.recovery.unknown_format` とその `{dir}` で検査する)、(c) 元のファイル (退避ディレクトリ・manifest・本文) が **変更・移動・削除されていない** (起動・終了の前後で偽 FS の内容 (パスとバイト列) が一致する) (02 §11.2.1、05 §7.5 (a))。G1c (保存・退避の本実装。v0.1 の M3 の kill テストと同時期、08 §5) で書き、M3 を終える条件に含める。v0.2 からはスワップ (`swap/`) と 30 日超の退避の削除の確認でも同じことを検査し、`last_version.toml` (FR-518) の未知の版が起動を妨げないことも加える。

### 3.5 L4: TUI 描画テスト

- **色深度の判定 (FR-513 AC-3、04 §5)**: 環境変数 (`COLORTERM` / `TERM` / `WT_SESSION` / `NO_COLOR` 等) の組 → `color_depth` を返す判定関数を、環境を読む部分と分けた純関数として eve-tui に置き、04 §5 の判定表の各行を 1 ケースにした **表駆動の L0 テスト** で検査する (判定表の行と同じ順・同じ入力。環境変数を実際に設定しない)。`theme.color_mode` による固定が判定より優先されることも同じ表に含める。
- `Frame` → `ratatui::Buffer` 変換を `ratatui::backend::TestBackend` で検査。**色の段は core が選ぶ** (04 §12.1 C1): `Capabilities.color_depth` を truecolor / 256 / 16 / mono に切り替えて同じシナリオを流し、Frame に載る `Style` が、truecolor と mono の段では組込みテーマの明示値 (07 §8.2 のロール表) と一致し、256 / 16 の段では truecolor の値を 04 §12.3 の表で近似した値と一致すること (近似で前景と背景が同じ番号になったロールは反転に置き換わっていること) と、eve-tui がセル変換をしていない (出力 SGR が Frame の色と同じ) ことを検査。eve-tui 側の最終防衛線の近似表 (04 §12.3) は単体テスト。
- **`xtask check-themes`** (CI の `check-themes` ジョブ。FR-513 AC-1 / AC-2、NFR-604): 07 §8.2 のロール表 (表の読み方は 11 §8.5 の機械可読の規則) と組込みテーマの定義を入力に、(a) v0.1 の全ロールに truecolor と mono の値があること (「なし」の明示を含む。256 / 16 の段は v0.1 では書かない)、(b) 各段が 07 §8.1 の 3 形 (組 / 属性だけ / 前景だけ) のどれかで、**背景だけを指定した段が無い** こと、(c) truecolor の段で、組は 4.5 以上、前景だけの段は白・黒の両背景に対して 3.0 以上、(d) 意味のあるロール (エラー・警告・選択・検索一致・現在の一致・ボタンのフォーカス・トグルの ON) の mono 段が空でないこと、(e) 表のロール名と実装のロール名の集合が一致すること (ロール名は公開面)、(f) truecolor の値から近似した 256 / 16 の段で、組の前景と背景が同じ番号にならないこと、を検査する。256 / 16 の段のコントラストの検査は、その段を明示値にする版 (v0.2 以降の候補) で加える。
- **クロームの省略と圧縮** (FR-503 AC-2、FR-504、NFR-603。04 §8.7 / §8.8): 省略と圧縮は TUI の仕事なので、ヘッドレスの ASCII Frame ではなく **L4 の VirtualScreen** で検査する。80 桁・60 桁・40 桁・30 桁 × ヒント文脈 (`normal` と、`overlay` の各場面: パレット・プロンプト・確認ダイアログ・検索バー・チョード保留中) のスナップショットを持ち、性質として「優先度 5 の項目は必ず残る」「優先度の低い項目から短い形 → 削除の順」「`overlay` ではその場面で届かない割当の項目が出ない」を確かめる。80 桁の `normal` 文脈のヒントバーは 04 §3.2 のモックと一致すること (`^Q 終了 ^P コマンド ^S 保存 ^F 検索 ^O 開く ^Z 元に戻す ^G 行へ移動`。`Ctrl+S 保存` の形では 80 桁に入らないので圧縮段階 (2) の `^S` 表記・項目の間 1 セルになり、7 項目すべてが残る、04 §8.8) を検査する。曖昧幅 2 (`FrontendConfig.ambiguous_width = 2`) のときに `…` を 2 セルで数えていることも検査する。
- **ハードウェアカーソルの位置** (FR-605、04 §4.6 / §6.7 I1〜I5): 毎フレームの出力の最後の CUP と DECTCEM (表示) を VirtualScreen で読み、編集面 (全角文字の上では先頭セル)、パレットとプロンプトの入力欄、**検索バーの入力欄** (全角の検索語・欄より長い語の横スクロールを含む)、曖昧幅 2 の設定で、論理入力位置と一致することを検査する。L5 (§3.6) でも `vt100` で同じ位置をスモークとして確かめる。IME の候補窓そのものの位置は L7 (M)。
- 全角・結合文字・絵文字 (ZWJ)・タブの **セル占有** が Frame の宣言幅と一致 (FR-501)。曖昧幅 1/2 (NFR-605)。
- **自前の仮想画面モデル (OQ-68)**: `TestBackend` と L5 の `vt100` はいずれも unicode-width 前提で描くため、曖昧幅を 2 として描く端末や VS16 / ZWJ の扱いが異なる端末を再現できない。`eve-testkit` に **セル幅を設定で切り替えられる仮想画面モデル** (`VirtualScreen { ambiguous: 1|2, emoji: 1|2, vs16: 1|2, pua: 1|2 }`) を置き、**Frame → Buffer → 出力バイト列 → 仮想画面** の往復でカーソル・各書記素のセル位置が Frame の宣言と一致することを検査する (04 §4.7 の幅不一致対策 (a)(c) の自動検証)。曖昧幅 2 設定で 1,000 回のランダム更新後も一致すること (08 §3 S-06 の合否基準をテストとして固定)。
- 出力バイト列のスナップショット (ratatui 差分出力は不安定なので **Buffer の比較を正**、バイト列は参考)。
- 端末インジェクション: Span に制御文字が残っていないことを出力直前に `debug_assert` し、テストでは出力バイト列に `\x1b` が「eve-tui が生成した位置以外」に現れないことを検査 (NFR-503 の出力側)。
- 入力の正規化 → `InputEvent` は、Windows (v0.1) は `ReadConsoleInputW` の入力レコード列 → `InputEvent` (04 §6.2 の「入力経路ごとの写像」の `ReadConsoleInputW` 経路の列。`Ctrl+H` / `Ctrl+I` / `Ctrl+M` / `Ctrl+[`、Press / Release・キーリピート、AltGr) を、入力レコードの列を直接与える単体テストで検査する (04 の表を網羅)。crossterm のイベント → `InputEvent` (Ctrl+H/Backspace、Ctrl+I/Tab、ESC 前置 Alt。kitty キーボードプロトコルの有効化を含む) は Unix を出荷する v0.2 から。

**契約テスト (frontend-parity・Frame JSON ゴールデン・contract-export)** (02 §8.4、ADR-0030 / ADR-0039):

- **frontend-parity**: 全ヘッドレス統合シナリオ (§3.4) を TUI (`VirtualScreen`) とヘッドレス参照レンダラ (`eve-testkit` の `HeadlessFrontend`) の 2 つで実行し、同じ Frame・境界型から編集面のセル占有 (`CellOccupancy`) が一致することを検査する (クロームは差があってよい)。TUI が Frame を忠実に描くことと、Frame がレンダラに依存せず解釈できることを確かめる。**v0.1 は CI ジョブを置かず**、`HeadlessFrontend` の ASCII Frame のスナップショット (§3.4) と、上記の L4 の幅の検査までとする。
- **Frame JSON ゴールデン**: 全シナリオの Frame を JSON (eve-view の serde) で固定し、`CONTRACT_VERSION` の更新を伴わない変化を失敗にする。C3 で契約版 1 を固定するとき (02 §8.4) に frontend-parity のジョブと同時に入れる。
- **contract-export**: 契約テストの成果物 (Frame JSON ゴールデン・シナリオ・`CONTRACT_VERSION`) を公開 crate / リリースに添付し、外部の消費者が取り込んで同じシナリオを回せるようにするジョブ (12 §4.3、11 §9.5)。**v0.1 には置かない** (ADR-0039)。

### 3.6 L5: 端末 E2E (疑似端末)

実バイナリを疑似端末 (v0.1 は Windows の ConPTY) 内で起動し、画面状態を VT パーサで解釈して検査する。**遅く不安定になりやすいので最小限** (スモーク + 端末状態の復元 + 障害時の挙動)。v0.1 は Windows だけを出荷するので、L5 は Windows のランナーだけで走らせる (Unix の pty とシグナルのシナリオは macOS / Linux を出荷する版で加える)。

| 項目 | 内容 |
|---|---|
| pty ライブラリ | `portable-pty` (wezterm 由来、MIT。Windows **ConPTY** と Unix `openpty` を同一 API で。v0.1 は ConPTY だけを使う) |
| 画面解釈 | `vt100` クレート (MIT) で出力を仮想画面に適用し、テキスト・カーソル位置・属性を assert。**生バイト列の比較はしない** (ConPTY は出力を再合成するため一致しない) |
| 入力 | バイト列を書く (キーシーケンス)。ブラケットペースト・リサイズ (`portable-pty` の `resize`) |
| 必須シナリオ (v0.1 は 10) | (1) 起動→文字入力→Ctrl+S→Ctrl+Q、ファイル内容一致 (2) 未保存で Ctrl+Q → 確認ダイアログが見える。`Tab` → `Enter` と `d` のそれぞれで「破棄」を選べる (3) raw モードで端末の制御キーがアプリに届く (FR-609 AC-1。T/M): Ctrl+C がプロセスを殺さない、Ctrl+S / Ctrl+Q がコマンドとして届く、Ctrl+Z が Undo として届く (4) **リサイズの追従 (FR-512 AC-1)**: `resize` で端末サイズを **100 ms 間隔** で連続して変え、**フレーム時刻** を記録する: 計測の記録 (下の「計測の記録」、`EVE_TRACE_FILE`。計測用ビルドの feature `bench-trace` が要る) で、各サイズの `Resize` の受信 → そのサイズの Frame の `write` + `flush` の完了の区間を記録する。**測定条件**: リサイズの間隔 100 ms、CI では 10 回を記録 (16 ms は **記録のみ (ゲートにしない)**)、基準機 ENV-W1 では **100 回以上** を手動実行して p99 を求め、最大値も参考として記録する。**FR-512 AC-1 の合否は基準機の p99 ≤ 16 ms** (下の「時間の判定」)。表示が崩れないことは、最後の Frame の flush の後に pty の出力を読み切ってから `vt100` の画面で検査する (読み切りの待ちは判定の時間に含めない。ConPTY の再描画の遅れはアプリの外の遅れなので、画面の到着時刻では判定しない) (5) 終了後、代替スクリーン解除・カーソル表示・コンソールモードの復元のシーケンスが出力に含まれる (端末復元) (6) **保存中の強制終了**: 下の切断点の各フックで **`TerminateProcess`** (環境変数 `EVE_TEST_HOOK=kill_before_rename` 等、**テストビルド限定** の `#[cfg(feature = "test-hooks")]`) し、切断点ごとに **ディスクの状態** と、続けて新しいプロセスで同じファイルを開いたときの **報告** を検査する (強制終了したプロセスは報告できないので「その場の報告」は検査しない。NFR-301 / 01 FR-105 AC-1、05 §4.6)。(i) **原子的保存の 4 切断点** (`kill_after_tmp_alloc` / `kill_after_tmp_write` / `kill_after_flush` / `kill_before_rename`): T が無傷 (保存前とバイト一致)、一時ファイル (`.eve-tmp-*`) が残る、開いても報告なし (`msg.open.interrupted_save` が出ない。T があり記録が `method = Atomic` のため、05 §4.6)。一時ファイルが残るので、開いた後も保存中フラグは消えずに残り (記録された一時ファイルが実在する記録は報告せずに残す。05 §4.6 の「フラグを消す条件」)、もう一度開いても報告は出ない。続けて同じファイルを保存して成功しても、そのフラグ (一時ファイルが実在する記録) は残る (05 §4.1 手順 7)。(ii) **置換の途中** (`kill_after_replace_begin`: T を退避名に移した後、一時ファイルを T に移す前の状態を test-hooks が作って止める。`ReplaceFileW` の内部の中間状態の模擬): T が無く `.eve-bak-*` だけが保存前の内容で残り、開くと `msg.open.interrupted_save` (`complete = true`) がその `.eve-bak-*` のパスを報告する。(iii) **in-place の切断点** (05 の順: 乾式走査 → 照合 → 保存中フラグ → open → バックアップ → 領域確保 → 上書き): `kill_after_inflight_flag` (バックアップの前。フラグだけが残り、`backup_complete` が偽で、バックアップの一時名も無い) / `kill_mid_backup` (バックアップの一時名 `<最終名>.tmp` を書いている途中) / `kill_after_backup` / `kill_after_inplace_alloc` / `kill_mid_inplace_write=<バイト数>`。期待は切断点ごとに次のとおり (写像の正本は 02 §4.5)。**`kill_after_inflight_flag`**: 開いても報告なし (`msg.open.interrupted_save*` が出ない)、開いた後に保存中フラグだけが消え、T が保存前とバイト一致する。**`kill_mid_backup`**: 開くと `msg.open.interrupted_save_incomplete` (`complete = false`。バックアップが不完全であることを示す、04 §15.2) になり、`{backup}` が実在する `<最終名>.tmp` を示す。T は保存前とバイト一致する (上書きはまだ始まっていない)。**`kill_after_backup` / `kill_after_inplace_alloc` / `kill_mid_inplace_write`**: `state_dir/backup` のバックアップが完全 (フラグの `backup_complete` が真。バックアップは一時名で書いて fsync してから名前を変え、その後でフラグに記録する、05 §4.4) で、開くと `msg.open.interrupted_save` (`complete = true`) がそのパスを報告し、バックアップが保存前の元ファイルとバイト一致する。保存中フラグは書込中も維持される (05 §4.4)。(iii') **上書きの可能性あり**: `kill_mid_inplace_write` の後に `state_dir/backup` の最終名のバックアップを消してから開く (`InPlace` / true / true でバックアップが無い記録) と、02 §4.5 の写像表の「上書きの可能性あり」の行のキーになる (報告されずにフラグが消えることがない)。不正な組合せと解析できないフラグは L3 の `MemFs` で検査する (§3.4)。(iv) **バックアップなしの承認の後の中断** (`inplace_backup_fail` で事前バックアップを失敗させ、確認で「バックアップなしで上書き」(`in_place_backup = SkipConfirmed`) を選んだ後に `kill_mid_inplace_write=<バイト数>`): 保存中フラグにバックアップが記録されず (`backup = None`)、`state_dir/backup` にその保存のバックアップが無い。開くと `msg.open.interrupted_save_no_backup` になる (`msg.open.interrupted_save` / `_incomplete` にならない)。(v) **新規保存の中断** (T が存在しないパスへの保存。`method = NewFile`、05 §4.1 の「新規ファイル」): `kill_after_tmp_write` / `kill_before_rename` で、T が作られておらず、一時ファイル (`.eve-tmp-*`) が残り、保存中フラグに `method = NewFile` と一時ファイルの名前 (`tmp`) が記録されている。同じパスを開くと新規の文書 (`JobResult::NewFile`) として開き、`msg.open.interrupted_new` (`method = NewFile`、`complete = false`) が出る。文言とその引数は 04 §15.2 の目録のとおりで (02 §4.5 の写像表の `NewFile` の行)、残った一時ファイルのパスは開くジョブの結果の `InterruptedSave.tmp` (実在するものだけ) で確かめる。(vi) **ダングリングのリンク経由で開く検出**: (v) の中断の後に、T を指すシンボリックリンク (T が無いのでダングリング) を作ってリンクのパスを開くと、リンク先 T の保存中フラグが見つかり、(v) と同じ `msg.open.interrupted_new` が出る (開くときはリンクとリンク先の 2 つのキーで保存中フラグを調べる、05 §4.6 / §7.5)。リンクの作成に権限が要る環境 (開発者モードでない Windows) では管理者権限のランナーで実行し、作れないことを理由にスキップしない。一時ファイル名・退避名の長さの規則 (05 §4.1: 全体が 255 単位を超えるときは名前を切り詰めてハッシュを付ける) を 250 文字級の日本語のファイル名で確かめる。**in-place の保存 (実 FS)**: (i) `ReplaceFileW` の失敗を test-hooks で注入 (`EVE_TEST_HOOK=replace_fail`) → in-place で保存するかの確認が出る → キャンセルで元ファイルと一時ファイルの後始末を検査、承認で in-place の経路で保存され内容が一致する (`MoveFileExW` での置換は行わない、05 §4.2)。(ii) 保存先の文字コードで表現できない文字を含む文書を in-place の経路で保存する → 乾式走査で止まり、元ファイルのバイト列・長さ・更新時刻が不変 (7) `internal.debug_panic` (test-hooks) でパニック → 端末復元 + `recovery/` に退避 (本文は改行非変換・PUA エスケープ保持、manifest の形式情報が一致) + クラッシュログ (NFR-302)。端末復元の後の stderr が 04 §15.2 の順 (`panic.header` → `stderr.recovery.saved` → `stderr.recovery.next` → `panic.crash_log` → `panic.report`。v0.1 は日本語の行だけ、v0.2 からは各英語の行の後に日本語の行) で、パスに制御文字を含むファイル名でも ESC が素通りしない (NFR-503)。`state_dir` に書けない状態では `stderr.recovery.fallback`、退避先が全く無い状態では `stderr.recovery.failed` になる。退避の時間 (NFR-302 AC-5) は (8a) で測る。時間切れを test-hooks (`recovery_timeout`) で起こし、書けた分が manifest に記録され、manifest が `complete = false` になる (8) **緊急退避の時間とコンソールイベント**: 時間の検査は **02 §11.2.1 の「終了の順序と時間予算」の段ごとの表** に合わせ、test-hooks が記録する各段の時刻 (受信 (パニックでは検出)・退避の完了・メインの終了・ハンドラから戻る / 終了) で段ごとに判定する。(8a) **パニック / Fatal の経路で AC-5 を測る** (02 §11.2.1 の表のパニック / Fatal の列: 退避の段 ≤ 5 s、フラグ待ち最大 1 s、合計 ≤ 5 s。退避の段が 5 s を使えるのはこの経路だけ): 合計 100 MB の dirty 文書を持つ状態で `internal.debug_panic` を注入し、パニックの検出から退避の完了 (manifest が `complete = true`) までが 5 s の内であること (**NFR-302 AC-5** の判定) と、検出からプロセスの終了までの合計も 5 s の内であることを確かめる。`HostMsg::Fatal` (入力スレッドの死亡) の経路はメインが同じ手順と予算 (最大 5 s) で退避するので、パニックの注入で代表させる。(8b) **シグナルの経路 (コンソールイベント)**: CTRL_CLOSE と、外部から送った CTRL_C / CTRL_BREAK で、対話なしで退避して終了する (終了コードは 05 §11.5)。送り方は下の「コンソールイベントの送り方」。退避の段の予算が 5 s より短いので、合計 100 MB の退避の完了は要求しない (AC-5 の判定は (8a))。(i) **CTRL_C / CTRL_BREAK** (退避の段 ≤ 2.5 s、メインの終了待ち ≤ 2 s、合計 ≤ 5 s): 退避が 2.5 s の内に終わる量の dirty 文書で manifest が `complete = true` になり、受信からプロセスの終了までが 5 s の内であること。合計 100 MB の dirty 文書 (または `recovery_timeout` で退避の段の予算を縮めた状態) で CTRL_C を送り、2.5 s (縮めた場合はその値) の内に退避が終わらなければ、書けた分が manifest に記録されて `complete = false` で打ち切られ、受信からプロセスの終了までの全体が 5 s (退避 2.5 s + メインの終了待ち 2 s + 端末の復元 0.2 s) の内であることを合格とする。**メインがブロックした状態の CTRL_C** (pty の読み手を止めてメインスレッドが端末への `write` でブロックした状態) も行い、メインの終了待ち (2 s) が尽きた後に受信スレッドが端末の復元を試みて合計 5 s の内に終了することを確かめる。(ii) **CTRL_CLOSE** (退避の段 ≤ 3.5 s、合計 ≤ 4.5 s。メインの終了は待たない): 退避が 3.5 s の内に終わる量の dirty 文書で、manifest が `complete = true` になり、受信からハンドラから戻るまでが 4.5 s の内であること。合計 100 MB の dirty 文書 (または `recovery_timeout` で縮めた状態) で CTRL_CLOSE を起こし、3.5 s (縮めた場合はその値) の内に退避が終わらなければ、書けた分が manifest に記録されて `complete = false` で打ち切られ、受信からハンドラから戻るまでの全体が 4.5 s の内であることを合格とする。**書込ブロック中の退避 (NFR-302 AC-3)**: pty の読み手を止めてメインスレッドが端末への `write` でブロックした状態で CTRL_CLOSE を起こし、退避が書かれ、受信からハンドラから戻るまでが 4.5 s の内であること (01 NFR-302 AC-3、02 §11.2。S-11) (9) **貼り付けの安全 (01 FR-206 AC-2)**: 本文に 0x13 (Ctrl+S 相当) と `ESC[201~` を含めて、個別キーのバーストとして送る (Windows の入力経路は `ReadConsoleInputW` の自前ループ、OQ-33) → 保存等のコマンドが実行されず、文字として挿入・可視化される。ブラケットペーストでの同じ検査は Unix を出荷する版で加える (10) **一時ファイルの権限 (NFR-505)**: 保存用一時ファイル・緊急退避ファイルが **作成直後** (test-hooks のフックで書込前に停止) から所有者のみの DACL であることを `GetSecurityInfo` で検査 (01 NFR-505 AC-1)。**v0.2 以降** (macOS / Linux を出荷する版を含む): Unix の SIGKILL・SIGTERM / SIGHUP (終了コード 128+n)・SIGTSTP と、macOS の制御キー (DSUSP 等)、Unix のファイル名 (非 UTF-8)・0600 の権限、保存中フラグからの復旧提案、`evoeditor -` (v0.3) |
| Windows (ConPTY) 注意 | `windows-2022`/`2025` ランナーで動作。ConPTY は Resize のあと再描画が遅延するが、L5 の合否は **アプリが記録するフレーム時刻** で判定し、画面の到着を待つ時間の許容は置かない (シナリオ (4))。画面の検査の前の「出力の読み切り」はタイムアウト付きで、タイムアウトは失敗として扱う (時間の許容ではない)。Windows の入力は `ReadConsoleInputW` の自前ループ (OQ-33) なので、貼り付けのシナリオ (9) はバースト検出の経路 (ConPTY に貼り付け本文を一度に書き込み、個別キーのバーストとして届ける) で実行する |
| コンソールイベントの送り方 (シナリオ (8)) | **CTRL_C / CTRL_BREAK**: 補助プロセス **`console-event`** (bin crate `eve-test-helpers` の bin。`clip-delay` と同じ crate、§3.4) を起動し、補助プロセスが `FreeConsole` → `AttachConsole(<evoeditor の pid>)` で対象のコンソールに付いてから `GenerateConsoleCtrlEvent` (対象は全体 (`0`)。ConPTY の子は新しいプロセスグループではないので、CTRL_BREAK もグループ指定は使えない)。**ただし CTRL_C は raw モード (`ENABLE_PROCESSED_INPUT` を落とした状態) では conhost が配らない** (実測。M3 で確認。`CONSOLE_IGNORE_CTRL_C` が立つため)。このため CTRL_C のシナリオは、補助プロセスからの送信ではなく「Ctrl+C がプロセスを殺さず入力として届く」(シナリオ (3)) の確認に置き換え、外部から届くのは CTRL_BREAK だけにする を送る。テストの実行プロセス自身は対象のコンソールに付かない (自分にもイベントが届くため)。**CTRL_CLOSE**: `GenerateConsoleCtrlEvent` では作れないので、pty (疑似コンソール) を閉じて起こす。閉じる呼出し (`ClosePseudoConsole`) は版によって conhost の終了を待ってブロックするため、別スレッドで呼び 10 秒で打ち切る (タイムアウト付きのクローズ)。**届いたことの確認**: evoeditor の test-hooks がハンドラの受信時刻と種別をファイルに記録し、テストはそれで「届いた」ことを確かめてから時間予算を判定する。記録が無いまま終了した (イベントが届かなかった) 場合は **失敗** とする (「スキップ」にしない)。失敗の原因が CI の環境 (ランナーの ConPTY・conhost の版など) にあると判断した場合は、その項目を **基準機で再判定する**: 基準機で同じテストを実行し、あわせて Windows Terminal のタブ (と conhost のウィンドウ) を閉じる手順を L7 で行い (08 §3 S-11)、結果をリリースのチェックリスト (§8.2) に書く。再判定に合格した場合も、CI の失敗とその原因の判断はリリースのチェックリストに残す。到達しなかった回数は CI の要約に出す |
| 再試行 | nextest の `retries = 2` を **L5 のみ** に許可。3 回連続失敗で flaky 課題を起票 |
| 安定させる規則 (v0.2 の M8 で整理) | (a) **同時実行数をそろえる**: `.config/nextest.toml` の test-group `e2e` (`max-threads = 4`) を全 profile (`default` / `ci` / `e2e`) の L5 に掛ける。L5 は 1 テストが ConPTY とプロセスを起動し fsync を打つので、コア数なりに並べるとディスクの fsync が詰まって時間予算 (緊急退避の 2.5〜5 s など) を割る。CI の Windows ランナー (4 コア) と同じ並びを、24 コアの開発機でも再現する。(b) **隔離する**: セッションは `EVE_STATE_DIR` と `EVE_CONFIG_DIR` を必ず試験専用にする (開発者の `%APPDATA%` の設定を読まない)。設定ディレクトリを指定しない試験は**スワップを切った**設定で動かし、スワップを試す試験だけが有効にする (負荷でスワップの書込・復旧の提案の時機が揺れ、無関係の試験を邪魔するため)。`EVE_LANG=ja` も固定する。(c) **製品の時間契約は、観測できる最も保守的な側で測る**: 補助プロセスの起動など、負荷で遅れる手順を時計の始点に含めない (下限は手順の前から、上限は手順の後から測る)。(d) **リトライで隠さない**: 手元 (`cargo xtask check`) は retries なしで通ること。CI の retries は最後の保険で、リトライで通ったテストは flaky として課題にする |
| **無通信 (NFR-501 AC-1)** | pty を使わない検査で行う。**v0.1**: (a) **依存の禁止リスト**: `deny.toml` の `bans` に、ネットワーク系の crate (`reqwest` / `hyper` / `tokio` の net 機能 / `ureq` / `socket2` 等) を禁止として書き、`cargo deny check bans` を Windows の配布ビルドの対象 (`x86_64-pc-windows-msvc`、配布ビルドの feature) で実行する (`deps` ジョブ。`xtask check-deps` の R1 の禁止リストとも同じ集合)。(b) **依存規則 R3 とレビュー**: `eve-core` が `std::net` を使わないこと (clippy の `disallowed_types`、§7 R3) と、`std::net` を使うコードが他の crate にも無いこと・テレメトリのコードが無いことをレビュー (§8.4・§10) で確かめる。**v0.2** (Linux を出荷する版。01 NFR-501 AC-2): Linux の CI で、ヘッドレスのシナリオ 1 本 (eve-host + 実 FS の eve-platform。開く → 編集 → 保存 → 終了) を `strace -f -e trace=network` の下で実行し、ネットワーク系のシステムコールが 0 (または seccomp でネットワーク系のシステムコールを失敗させて正常に終わる) ことを検査する |
| 時間の判定 | L5 の **絶対時間** (FR-512 AC-1 の 16 ms、02 §11.2.1 の段ごとの時間予算: パニック / Fatal の退避 5 s (フラグ待ち最大 1 s、合計 5 s。NFR-302 AC-5)、CTRL_C / CTRL_BREAK の退避 2.5 s・メインの終了待ち 2 s・合計 5 s、CTRL_CLOSE の退避 3.5 s・合計 4.5 s (NFR-302 AC-3)) は、CI では **記録のみ (ゲートにしない)** とし、段ごとの値を CI の要約に出す (共有ランナーの時間はノイズが大きい、Q6)。合否は **リリースゲートで基準機 ENV-W1 の手動実行** で判定する: 同じテストを計測用ビルド (`test-hooks` と `bench-trace`) で基準機で実行し、FR-512 AC-1 は (4) の測定条件 (100 ms 間隔・100 回以上) で p99 ≤ 16 ms (最大値は参考)、NFR-302 AC-5 は (8a) のパニックの経路で (100 MB の退避が 5 s の内に `complete = true` で終わる)、シグナルの経路は (8b) で (CTRL_C / CTRL_BREAK は 2.5 s での `complete = false` の打切りと全体 5 s の内の終了、CTRL_CLOSE は 3.5 s での `complete = false` の打切りと全体 4.5 s の内の終了が合格。AC-3 は (8b) (ii) の書込ブロック中の退避)、各 3 回の実行のすべてで段ごとの時間予算の内。結果はリリースのチェックリスト (§8.2) と `docs/perf/<version>/` に書く。CI で判定するのは時間以外の部分 (退避が書かれた・manifest が正しい (`complete` の値を含む)・画面が崩れない) |
| 実行時間予算 | 全 L5 で 3 分以内 |
| **ローカル LLM の E2E (v1.L、任意)** | 実バイナリ + `MockLlamaServer` を pty 内で動かし、補完要求中のキー入力遅延 (BM-115) と、`Esc` によるキャンセルで接続が閉じることを検査。**実 `llama-server`** を使う契約テストは §3.9 (CI の既定では走らせない) |

**配布 exe のスモーク (L5 スモーク 3 本)**: 配布ビルドの `evoeditor.exe` には test-hooks が無いので、test-hooks を使うシナリオ (上の (4)・(6)〜(8)・(10)) は回さず、次の 3 本だけを回す。対象の exe のパスは環境変数 **`EVE_EXE`** で渡す (未設定なら `target/release/evoeditor.exe`)。`release.yml` の検証 (§9.2 手順 6) は展開したアーカイブの exe を `EVE_EXE` に入れて実行する。(S1) **起動と終了**: ConPTY で起動し、最初の画面が出た後に `Ctrl+Q` を送り、終了コード 0 で終わる。端末の復元のシーケンスが出力に含まれる。(S2) **`--version`**: `evoeditor --version` (pty 不要) の出力の版がタグ (または `Cargo.toml` の版) と一致し、終了コード 0。(S3) **日本語のファイルの開く → 保存 → 終了**: 日本語のファイル名 (例 `日本語.txt`) の UTF-8 の日本語の文書を引数で開き、1 文字入力して `Ctrl+S` → `Ctrl+Q` で終わり、ファイルの内容が期待バイト列と一致する。

**test-hooks の一覧 (`EVE_TEST_HOOK`)**: feature `test-hooks` のビルド (テストビルドと計測用ビルド) だけが読む環境変数。値は `,` 区切りで複数指定でき、引数を取るものは `名前=引数` と書く。未設定なら何もしない。フックが動いたことと時刻は **`EVE_TEST_HOOK_LOG`** (ファイルのパス) に 1 行 1 件で記録する (テストはこの記録で「届いた・動いた」を確かめる)。

| 名前 | 挙動 | 使うテスト |
|---|---|---|
| `kill_after_tmp_alloc` / `kill_after_tmp_write` / `kill_after_flush` / `kill_before_rename` | 原子的保存の各切断点 (一時ファイルの領域確保の直後 / 一時ファイルに書いた直後 / flush 直後 / 置換直前) で自プロセスを `TerminateProcess` する | (6) |
| `kill_after_replace_begin` | 置換の途中 (T を退避名 `.eve-bak-*` に移した後、一時ファイルを T に移す前) の状態を作って `TerminateProcess` する | (6) (ii) |
| `kill_after_inflight_flag` / `kill_mid_backup` / `kill_after_backup` / `kill_after_inplace_alloc` / `kill_mid_inplace_write=<バイト数>` | in-place の各切断点 (05 §4.4 の手順の順。保存中フラグの作成直後 (バックアップの前) / バックアップの一時名 `<最終名>.tmp` を書いている途中 (最終名に変える前) / バックアップの作成直後 (`backup_complete` の記録の後) / 領域確保の直後 / 指定バイト数を上書きした直後) で `TerminateProcess` する | (6) (iii) |
| `inplace_backup_fail` | in-place の事前バックアップ (05 §4.4 手順 4) を、何も書かずに失敗させる (`BackupFailed` → 「バックアップなしで上書き」の確認) | (6) (iv) |
| `replace_fail=<エラーコード>` | `ReplaceFileW` を呼ばずに指定のエラーで失敗させ、eve-platform の写像 (05 §2.1) を通す。既定は `ERROR_UNABLE_TO_MOVE_REPLACEMENT` (T 無傷 → `AtomicUnsupported` → in-place の確認)。`replace_fail=ERROR_UNABLE_TO_REMOVE_REPLACED` は `Busy` の再試行のテストとして別に立てる (再試行の後に保存の失敗 `Busy` が報告され、T が無傷) | (6) の in-place の保存 (i)、`Busy` の再試行 |
| `inplace_len_mismatch` | in-place の 2 回目のエンコードが乾式走査の `out_len` より 1 バイト長い列を供給する (不変条件違反の注入) | eve-platform の L0 (`InPlaceFailed`) |
| `pause_after_create=<印のファイル>` | 保存用一時ファイル・退避ファイルを作った直後 (書く前) に、印のファイルが消えるまで待つ | (10) |
| `recovery_timeout=<ms>` | 緊急退避の **退避の段** (02 §11.2.1 の段 1。パニックは 5 s、CTRL_C / CTRL_BREAK は 2.5 s、CTRL_CLOSE は 3.5 s) の予算を指定の値に縮め、時間切れ (`complete = false` の打切り) を起こす。後の段の予算と合計の上限は変えない | (7) (8) |
| `log_console_events` | コンソールイベントのハンドラの受信時刻と種別を `EVE_TEST_HOOK_LOG` に記録する | (8) |
| `panic_command` | `internal.debug_panic` をコマンドとして登録する (無ければ登録しない) | (7)、NFR-504 |
| `main_stall` | メインが `HostMsg::Signal` を受けたところで止まる (無限に待つ)。受信スレッドの 2 秒待ち → 端末の緊急復元 → `128+n` での終了 (02 §9.6 手順 4) を検査する | (8) メインが止まった状態の CTRL_BREAK |
| `undo_limit_expand=<バイト数>/<ステップ数>` | 文書あたりの Undo の上限を一時的に拡大する (例 `undo_limit_expand=1GiB/100000`) | BM-203 (§5.2) |

**計測の記録 (`bench-trace`、`EVE_TRACE_FILE`)**: 区間の時刻 (BM-101a・BM-103b・フレーム時刻 (4)・FR-122 の「開く→進捗」・退避の時間) は `tracing` の span で記録する。(1) **有効化**: 計測用ビルド (§5.2。配布ビルドと同じ release プロファイル・feature に `test-hooks` と `bench-trace` を足したもの) で、環境変数 `EVE_TRACE_FILE=<パス>` があるときだけ購読者を入れる。未設定なら購読者を入れない (`tracing` のマクロはほぼ無コスト、02 §11.3)。配布ビルドには `bench-trace` が無い (§6.2 の `build-release-smoke`)。(2) **形式**: JSON lines (1 行 1 span)。各行は `{"name": span 名, "start_ns": 単調時計の開始 (プロセス開始からの ns), "end_ns": 終了, "thread": スレッド名, "fields": {…}}`。`fields` には文書の本文・パスを入れない (NFR-504。サイズ・件数・種別だけ)。記録するのは名前の一覧 (`input.read`・`input.handle`・`render.frame`・`render.flush`・`resize.recv`・`open.progress`・`recovery.write` 等。正本はコードの定数) に載る span だけ。ボトルネック解析用の Chrome trace (`tracing-chrome`) は `EVE_TRACE_FORMAT=chrome` で選ぶ。(3) **オーバーヘッド**: 書出しは専用のスレッドで行い (バッファ付き)、終了時に flush する。計測区間に書出しの I/O を含めない。`EVE_TRACE_FILE` の有無で BM-103a の p50 の差が 5% 以内であることを計測用ビルドの導入時と依存の更新時に確かめて `docs/perf/` に記録する (超えたら記録する span を減らす)。**v0.1 の実装** (簡素化): `tracing` クレートは使わず、`eve-tui` の `trace` モジュール (feature `bench-trace`) が同じ形式の JSON lines を、メモリにためて終了の直前 (`--bench-exit-after-first-frame` では最初の Frame の直後) にまとめて書き出す (計測区間にファイルの I/O を含めない)。記録するのは `input.to_flush` (入力スレッドが入力を読み終えた時刻 → 描画の `write` が戻った時刻。`fields.vk` に最初のキーの仮想キーコード)、`startup.first_frame` と `startup.first_content` (`main` 入口 → 最初の Frame / 本文が見える最初の Frame の `write` が戻った時刻。`fields.wall_main_unix_ns` / `wall_end_unix_ns` に壁時計)。配布ビルドにはこのモジュールの本体も `EVE_TRACE_FILE` の文字列も入らない (`xtask check-dist`)。`tracing` への移行は、core がイベントを出す版 (`open.progress` など) に回す。BM-103a の `EVE_TRACE_FILE` 有無の差は、BM-103a が headless でこの記録を通らないので該当しない。

### 3.7 L6: 互換マトリクス (M)

05 §9.4 の表をリリースごとに **実機で記入** する。v0.1 の対象は **Windows 11 の Windows Terminal と conhost** だけ (確定 (ユーザー回答)、01 NFR-402)。手順書 `docs/compat/checklist.md` に端末ごとの確認項目 (起動、日本語入力と候補位置 (Microsoft IME・Google 日本語入力)、全角・絵文字表示、Ctrl+Shift の区別、貼り付け、端末の範囲選択でのコピー、リサイズ、終了後の端末状態) を列挙し、結果を `docs/compat/<version>/windows-<terminal>.md` に記録 (Windows と端末のバージョンも記載)。OSC 52・マウスの項目は v0.2、macOS / Linux の端末はそれらを出荷する版で加える。

**対象 OS** (正本は [01 §4.6](01-requirements.md#46-v01-の既知の制約-リリースノートとヘルプに明記する)、確定 (ユーザー回答)):

| OS | v0.1 | 内容 |
|---|---|---|
| Windows 11 | **出荷する** | 開発機 (01 §3.0 の基準機) で実機の動作確認と性能の判定を行う。CI でも全テストを行う (CI のランナーは Windows Server 系なので、実機の確認は基準機で行う) |
| Windows 10 | **出荷しない** (公式には対応しない) | 実機の確認環境が無いため対象にしない (確定 (ユーザー回答)、2026-10-25)。対象に加える版で、L6 の環境と 1809〜2004 の ConPTY の不具合の確認 (05 §1.1) を加える |
| macOS | **出荷しない** (v0.2 で出荷) | CI も行わない (macOS の CI ジョブは v0.2) |
| Linux | **出荷しない** (v0.2 で出荷) | CI は Linux CI の対象 crate (§6.4) のビルドとヘッドレスのテストだけ |

### 3.8 L7: 手動受入 (M)

| 項目 | 手順 | 判定 |
|---|---|---|
| NFR-601 初見ユーザビリティ | 端末に慣れていない協力者に「このファイルを開いて 1 行追加し保存して終了」を説明書なしで依頼。画面のヒント以外の助言をしない。10 分以内。**v0.2 で 3 名の所見** (ゲートにしない)、**v1.0 ゲートは 5 名** (OQ-106)。v0.1 では行わない | v1.0: 5/5 成功。失敗は原因を記録し 04 (ヒントバー) へフィードバック |
| FR-605 IME | Windows Terminal と conhost で、Microsoft IME と Google 日本語入力のそれぞれで日本語を 3 文 (変換・確定・取消・候補ウィンドウ位置)。編集面・パレット・プロンプト・検索バーの入力欄で | 候補がカーソル付近に出る。確定文字列が正しく挿入される |
| FR-122 巨大ファイル進捗 | 100 MB を開き、進捗表示とキャンセルを確認 | 表示される、キャンセルで即戻る |
| FR-206 AC-3 Windows Terminal での貼り付け | Windows Terminal (と conhost) で、(1) インデント付きの 100 行を他アプリからコピーして端末の貼り付け (`Ctrl+V` と右クリック) で貼る、(2) 本文に 0x13 (Ctrl+S) と `ESC[201~` を含むテキスト (手順書に添付するファイル) を貼る。入力経路は `ReadConsoleInputW` の自前ループとバースト検出 (OQ-33。N・T は S-07 で決めた値) | (1) 元のインデントのまま挿入され、1 回の Undo で消える (AC-1)。(2) 保存等のコマンドが実行されず、文字として挿入・可視化される (AC-2)。結果を `docs/compat/<version>/windows-<terminal>.md` に記録 |
| FR-205 クリップボード (AC-1) | Windows Terminal と conhost で、エディタ → 他アプリ、他アプリ → エディタのコピー / 貼り付け (Win32)。端末の範囲選択でのコピー (v0.1 はマウスを捕捉しない) も確かめる | 内容が一致する |
| FR-503/504/505/511/512/513/514、FR-602/603 | 表示系・パレット・既定キーの Must (自動化できる部分は L3 / §11.2 でも検査) | 手順書どおり |
| NFR-302 AC-2 / AC-3 の CTRL_CLOSE (CI で環境起因の失敗になった項目の再判定、§3.6) | 基準機で evoeditor を dirty な状態にし、Windows Terminal のタブを閉じる・conhost のウィンドウを閉じる (AC-3 は大量の出力で端末への書込を詰まらせた状態で)。退避ディレクトリと manifest を確かめる | 退避が書かれ、元の改行コード・文字コードで復元できる。結果を `docs/compat/<version>/acceptance.md` に記録 |
| FR-307 マウス (v0.2) | クリック・ドラッグ・ホイール。v0.1 はマウスを捕捉しないので「端末の選択・スクロールが素通しで動く」ことを確認 | 期待どおり |
| NFR-602 エラー文言 | 代表的なエラー 10 種 (権限なし、ディスク満杯、ファイル消失、文字コード不能、端末非対応…) を再現 | 「何が」+「どうする」が含まれる |
| FR-910 / NFR-116 ローカル LLM (v1.L-a) | S-22 の基準条件 (FIM 対応の 1.5B〜7B 級コードモデル、`llama-server`、プロンプトキャッシュ有効) で、入力停止からゴーストテキスト表示までの体感と、生成中のタイピングの引っかかりの無さを確認 | NFR-116 の値 (BM-117 で計測)、NFR-115 (BM-115) |

### 3.9 ローカル LLM 統合テスト (v1.L)

AI 連携はローカル LLM (`llama-server` 等) を一級市民にする (01 §2.9、06 §3.3)。CI には GPU も実モデルも無いので、**模擬サーバ (`MockLlamaServer`) に対する決定的なテストを正** とし、実サーバは夜間・手動の任意ジョブに限る。AI 固有の不変条件 (AI-INV) と要求の定義は 06 §16 が正本、本節は道具と CI での扱い。

| 項目 | 内容 |
|---|---|
| `MockLlamaServer` の範囲 | **OpenAI 互換**: `POST /v1/chat/completions` (SSE と非 SSE)、`POST /v1/completions`、`GET /v1/models`。**llama-server 固有**: `POST /completion` (`cache_prompt`、`n_probs`、`grammar` / `json_schema`、`stop`、`stream`)、`POST /infill` (`input_prefix` / `input_suffix` / `input_extra`)、`POST /tokenize` / `/detokenize`、`GET /health` (`ok` / `loading model` 503 / `no slot available` 503)、`GET /props` (モデル名・`n_ctx`・`total_slots`・チャットテンプレートの有無)、`GET /slots` (各スロットの `n_ctx`・`cache_tokens` 相当・処理中か)、`POST /apply-template`。各エンドポイントの **フィールドは S-22 で確定した版の llama-server に合わせ、`eve-testkit/src/ai/llama_api.md` に版と差分を記録** する (llama-server の API は版ごとに増減する: 08 §2.7 のリスク、OQ-128)。未知のフィールドはサーバ・クライアントの双方で無視する |
| トークナイザ | 模擬の決定的トークナイザ (空白・句読点・CJK 1 文字単位。実モデルの語彙は再現しない)。eve-ai の **予算計算が `/tokenize` の結果と一致する** こと (01 FR-917 AC) を検査できればよく、実トークン数は実機契約テストで |
| 生成の決定性 | `seed` (要求の `seed` または模擬サーバの `--seed`) とプロンプトのハッシュから擬似トークン列を決定的に生成する。カセット (記録再生) があればそれを優先。構造化出力 (`json_schema` / `grammar`) は「スキーマに合う固定の応答」と「合わない応答 (不正 JSON、余分なフィールド)」を台本で切り替えられる (01 FR-917 AC: 合わない場合は適用せず失敗として表示) |
| カセット (記録再生) | 実サーバとの HTTP 往復を `testdata/ai/cassettes/<name>.json` に記録 (要求ハッシュ → 応答列 + タイミング)。記録は **人間が明示コマンド (`xtask ai-record`) で** 行い、`Authorization` ヘッダ・API キー形式の文字列・ホームパスを自動除去し、除去漏れ検査をコミット前チェックに入れる (06 §16)。再生時のタイミングは偽 `Clock` で進める |
| 注入 | **遅延**: 接続 / 最初のトークン / トークン間隔 (TTFT・tok/s を模擬。偽 `Clock` または実時間を選べる。BM-115 は実時間)。**切断**: N トークン後に TCP を閉じる、`Content-Length` 不足、途中で 500。**部分 SSE**: `data:` 行を任意のバイト位置で分割して別パケットで送る、keep-alive コメント (`: ping`) の挿入、`[DONE]` の欠落、改行コードの `\r\n`。**エラー**: 503 (モデル読込中 → `/health` が後で ok に変わる)、503 (スロット満杯 → 再試行とバックオフ)、429、400 (コンテキスト超過 → 予算を減らして再構成するか失敗表示)、401 (API キー不一致、管理モード)、501 (未対応エンドポイント → 能力検出の縮退経路)。**記録**: 受けた要求 (プロンプト本文・パラメタ・ヘッダ) をすべて保持し、テストが「送信内容のゴールデン」(06 §16) と「除外ファイルの内容が一度も現れない」(AI-INV-4) を HTTP の層で検査できるようにする |
| 判定の決定性 | 模擬サーバでは seed 固定で完全一致を検査できる。**実サーバ** (任意ジョブ) では seed 固定 + `temperature = 0` でも版やハードウェアで出力が変わり得るため、**出力の完全一致ではなく形の適合** (JSON schema に合う、FIM の出力が `max_lines` 以内、停止列で止まる) で判定する |
| 要求スケジューラ (06 §4.8) の性質テスト | 1 スロット前提で: 補完要求は常に 1 件だけ在席 (新しい入力で古い要求はサーバに届く前に破棄、届いた後はキャンセル)、指示編集の要求は補完より優先、キャンセル済みの要求の応答断片は UI に出ない、同時要求数が `max_concurrency` を超えない。ランダムな入力列 + 偽 `Clock` で |
| 能力検出 | `/props` / `/v1/models` / `/slots` の応答を台本で変え、06 §4.4 の `LlmCaps` (FIM・文法制約・トークン計数・`n_ctx`・スロット数) が切り替わり、無い能力は **使われない** (`/infill` 非対応なら FIM テンプレートか Chat にフォールバック、`/tokenize` 非対応なら推定) ことを L3 で検査 (01 FR-917 AC) |
| プロンプト安定性 (KV キャッシュ再利用率) | 同じ文書で続けて要求したとき、**プロンプトの先頭部分が前回とバイト一致** する (01 FR-917 AC)。模擬サーバは前回プロンプトとの最長共通接頭辞トークン数を `timings` 相当 (`prompt_n` / `cache_n`) で返し、テストは **再利用率 = 共通接頭辞 / プロンプト長** を計算する。L3 の性質テスト: カーソル付近の編集・カーソル移動・選択変更を 20 回繰り返しても、接頭辞 (システム・ファイルヘッダ・カーソル前の遠い部分) が変わらない → 再利用率 ≥ 0.9 (閾値は S-22 (c) の実測で調整)。ベンチは BM-116 |
| キャンセル | 生成中に `ai.cancel` → 模擬サーバが接続の切断を観測するまでの時間 (01 FR-917 AC: 50 ms 以内。実機は S-22 (f) で 100 ms 以内)。切断後にスロットが解放されたことを `/slots` で確認 |
| 応答性への非影響 (NFR-115) | BM-115 (§5.2)。模擬サーバを 50 tok/s で流し続けながら BM-103a/b を測り、p99 が無負荷時の +10% 以内。管理モードの起動中 (子プロセスの spawn + `/health` ポーリング) も同じ。メインスレッドの AI 関連処理が 1 メッセージ 1 ms 以内であることを `tracing` span で検査 |
| 管理モード (FR-918、v1.L-b) | `ManagedServerSim` で: 承認前に起動されない、ループバック以外の bind 指定が拒否される、起動ごとの API キー無しの要求が 401、`/health` が ok になるまで要求が流れない、`ready_timeout_s` 超過で停止、禁止引数 (`--host` / `--tools` 等、07 §5.12.1) を含む設定が起動前に拒否される、API キーがコマンド行に現れない (環境変数で渡る)、エディタ終了 (通常・SIGTERM・パニック) と無使用時間 (`idle_shutdown_s`) で子プロセスが残らない (3 OS。L5 相当)、クラッシュ後に自動再起動せず状態表示だけ出て `ai.runtime_restart` で再起動できる (06 §3.3.3) |
| ネットワーク境界 (NFR-509 / 510) | 模擬サーバを `127.0.0.1` と (テスト用の) 非ループバックアドレスで起動し、後者が既定設定で拒否され、`ai.network.allow_lan_hosts` (平文なら `allow_insecure_hosts` も) に載せると接続できることを検査。全テストはネットワークを切った環境 (CI の既定) で通る |
| 実機契約テスト (任意) | 実 `llama-server` (固定版) + 小さい GGUF (数百 MB 級。ライセンスを 05 §17.1 の許可リストで確認し、CI のキャッシュに置く。モデル名は docs に書かず `ci/ai-real.toml` に) で、模擬サーバと **同じシナリオ** を流して応答の形 (フィールド・SSE の形式・`timings`) が一致することだけを検査する (生成内容は検査しない)。GPU の無いランナーで CPU 推論 (遅い) → **夜間・手動の `ai-local.yml`** (§6.1)。Ollama / LM Studio / vLLM の OpenAI 互換は S-22 の手動確認のみ |

---

## 4. テストデータ (コーパス)

| 分類 | 内容 | 保管 |
|---|---|---|
| Unicode | 結合文字 (が = か+゛)、ZWJ 絵文字家族、肌色修飾、国旗 (RI ペア)、異体字セレクタ、全角/半角、曖昧幅 (※ ○ α)、ゼロ幅 (ZWSP/ZWNJ/BOM)、制御文字、タブ、RTL 文字 (bidi は **非対応と明記**: 表示が崩れないことだけ確認)、`\u{2028}`/`\u{85}` (改行とみなさない)、長い結合列 (Zalgo)、不正 UTF-8 (孤立継続バイト、過長符号化、サロゲート符号化、`\xFF`)、**書記素の途中の一致** (`e` と `é` (e + U+0301) の `e`、`👨` と `👨‍👩‍👧` の先頭は検索で一致に数えない、03 §12.1)、**可視化の対象** (bidi 制御 U+202A–202E / U+2066–2069、タグ文字 U+E0020–E007F、単独の異体字セレクタ、U+2028 / U+2029、書記素の途中に現れたもの。C1 制御文字 (`<U+0085>`) と不正バイトのエスケープ (`<0x9A>`) の表記が区別できること、03 §13.2)、**Kelvin 記号 K (U+212A) と ſ (U+017F)** (`Insensitive` の検索で `k` / `s` に一致する、03 §12.1 の経路 3) | `testdata/unicode/*.txt` (小さい。コミット) |
| 文字コード | Shift_JIS (CP932 の NEC/IBM 拡張の重複符号化 `¬` 0x81CA / 0xEEF9 / 0xFA54 (区分 B)、**CP932 互換文字の復号結果** 0x8160 (→ U+FF5E) / 0x8161 / 0x817C (→ U+FF0D) / 0x8191 / 0x8192 / 0x81CA / 0x815C / 0xFA55 (**往復する側**: 区分 A でバイト一致、区分 B に数えない。EUC-JP の 0xA1C1 / 0xA1C2 / 0xA1DD / 0xA1F1 / 0xA1F2 / 0xA2CC / 0xA1BD / 0xFCFC も同じ)、**CP932 の外字** 0xF040 / 0xF07E / 0xF080 / 0xF09F / 0xF0FC / 0xF140 / 0xF580 / 0xF9FC (区分 A: 自前の逆算で往復、外字以外を編集して保存しても外字のバイトは不変。03 §8.7、確定 (ユーザー回答))、**単独バイト 0xA0 / 0xFD / 0xFE / 0xFF** (不正バイトとして読み取り専用、名前を付けて保存でバイト一致) と 0x80 (U+0080 で往復)、半角カナ)、**JIS 系の文字を含む UTF-8 文書** (U+301C WAVE DASH・U+2016・U+2212・U+00A2・U+00A3・U+00AC・U+00A6・U+2014・`¥` U+00A5・`‾` U+203E と私用文字 U+E000 / U+E757。Shift_JIS / EUC-JP / ISO-2022-JP への保存で表現不能 (E3) になり、「置換して保存」で 03 §8.6 の互換文字の表どおりに置き換わって (`¥` → U+FFE5。0x5C にはしない) `counts.compat` が期待件数と一致し、置換後の文書が保存先で往復すること。`¥` `‾` は ISO-2022-JP では表現不能にならない。U+E000 / U+E757 は Shift_JIS では 0xF040 / 0xF9FC で書かれ、EUC-JP / ISO-2022-JP では `?`。期待保存バイト列を文字コードごとに添える)、EUC-JP (3 バイト JIS X 0212 補助漢字を含む無編集保存、1 文字編集後の保存)、**ISO-2022-JP の日本語ゴールデン** (v0.1 読み書き。03 §22.3: 正規形 (`ESC $ B` / `ESC ( B`、末尾 `ESC ( B`。各行末も ASCII 状態、OQ-162)、**判定の sniff 範囲 (64 KiB) の境界が 2 バイト文字の途中、または ESC 列の途中に来るファイル** (末尾に達していない sniff は `last = false` で復号し、未完の末尾を不正とみなさない。03 §8.2 手順 4'。読込チャンクの境界とは別)、**`¥` (ローマ字状態) の直後に改行が来る `"¥\nabc"`** (行ごとにエンコーダを閉じる方式の期待バイト列、§3.2)、旧 JIS `ESC $ @`、`ESC ( J` で書かれた ASCII、冗長なエスケープ、行末で ASCII に戻らない、末尾が ASCII 状態でない、`ESC ( I` の半角カナ、`ESC $ ( D` 補助漢字 (読み取り専用になる)、**JIS X 0208 状態のまま改行する** (`1B 24 42 24 22 0A 1B 28 42`、CR LF 版も。ISO-2022-JP を指定して開くと改行が不正バイト `<0x0A>` として保持・表示され読み取り専用、ISO-2022-JP での保存 (名前を付けて保存・文字コードを指定して保存) はジョブ投入前に拒否、他の文字コードへの保存は E3 でキャンセルだけ。03 §8.3、確定 (ユーザー回答))、8 bit バイト混在、`SO` / `SI`、ISO-2022-JP-2 のエスケープ、チャンク境界 (64 KiB / 1 MiB) の直前でエスケープが切れる、**先頭 64 KiB が ASCII で後半にのみ ISO-2022-JP がある** ファイル、ANSI エスケープ入りのログ (判定されないこと)。各ファイルに期待 UTF-8・期待区分 B 行数・編集後の期待保存バイト列を添える)、UTF-16LE/BE (BOM あり/なし、サロゲート、**BOM なし日本語 UTF-16LE** (C0 比率でバイナリ判定されないか、されても FR-111 で開ける))、UTF-8 BOM、Latin-1 風バイト列 (誤判定の典型)、チャンク境界をまたぐ不正バイト (1 MiB 境界の直前に不完全な多バイト列) | `testdata/encodings/*.bin` + 期待 UTF-8 (`*.utf8`) + 期待保存バイト列 (`*.saved`) |
| 日本語単語境界 | 03 §13.3 の期待結果表 (全角英数 `FullwidthAlnum` の例 `ＡＢＣabc` → `ＡＢＣ\|abc` を含む) (#27〜#30 の ZWJ 列・ハングル・WJ を含む) を `testdata/word_boundary/ja.toml` (入力・期待境界位置) に置き、L0 ゴールデンと L7 (FR-302 AC-1) で共用 | コミット |
| 改行 | LF / CRLF / CR / 混在 / 末尾改行なし / 空ファイル / 改行のみ / `\r\r\n`。**CR / LF 混在の文書での Enter と削除** (行頭・行末・行の中身の削除で行数が期待どおりに増減し、2 つの区切りが 1 つの CRLF に合体しない、`eol_guard` (03 §6.2)) | `testdata/newlines/` |
| バイナリ | NUL 含有、PNG 先頭、ランダム | `testdata/binary/` |
| 巨大・長大 | 1 MB / 10 MB / 100 MB / 1 GB、1 行 10 MB、100 万行の短い行、平均行長 80 バイト (NFR-104 の条件) | **コミットしない**。`xtask gen-corpus --seed 42` で決定的に生成 (ベンチ・テスト双方で同じ内容) |
| パス | 不正サロゲート名 (Windows)、260 文字超、UNC、予約名、末尾ドット、Unicode 正規化差 (NFD/NFC)、大文字小文字違い。非 UTF-8 名 (Unix) は Unix を出荷する版で。**UNC** は外部のサーバを使わず、`tempfile` の作業ディレクトリを管理共有で指す `\\localhost\<ドライブ>$\…` (例 `C:\…\tmp` → `\\localhost\C$\…\tmp`) の形で作る | テスト実行時に `tempfile` 配下へ生成。**Windows の項目 (01 NFR-404 AC-1 の 260 文字超・UNC・ドライブレター・不正 UTF-16) は `skip!` を失敗として扱う** (環境が無いことを理由に飛ばさない。01 NFR-404 AC-1)。それ以外の OS/FS で作成できない項目だけ `skip!` マクロで理由を記録して飛ばす |
| 設定 | 正常、未知キー、型違い、循環キーマップ、巨大 | `testdata/config/` |
| 退避 | 未知の `format_version` と未知のキーを含む manifest + 本文 (§3.4 のダウングレードの退避)、正常な退避 (改行・文字コード・PUA エスケープの復元確認) | `testdata/recovery/` (11 §6.2) |
| AI (v1.L) | `MockLlamaServer` のカセット (`testdata/ai/cassettes/`。記録は人間が `xtask ai-record` で。秘密の除去漏れ検査付き)、SSE の断片コーパス (`fuzz_ai_stream` の種)、インジェクション回帰コーパス (06 §16)、能力検出の台本 (`/props` / `/slots` の応答例を llama-server の版ごとに) | `testdata/ai/`。**モデルの重みはコミットしない** (実機契約テストは CI のキャッシュ、§3.9) |

**テストデータの規則** (確定 (ユーザー回答)、OQ-168、12 §5.6 の PC-6): リポジトリは public で、コミットしたものは誰でも取得できる。コーパス・テスト用のファイル (上表のすべて、fuzz のコーパス `fuzz/corpus/`、AI のカセットを含む) は、**合成したデータ** (生成スクリプト・手書き) か、**再配布が許されるデータ** (出典とライセンスを `testdata/` の README に記録する) だけを使う。**実際の個人の文書** (利用者・開発者の文書、メール、業務の文書) は、全体でも一部でも使わない。不具合の報告で受け取ったファイルは、再現に要る性質 (文字コード・改行・バイト列の形) だけを合成したデータに置き換えてからコミットする。

**`.gitattributes` 必須**: `testdata/** -text` (Git の改行自動変換を完全に無効化。Windows ランナーで CRLF 化されるとテストが壊れる)。`*.rs text eol=lf`。

---

## 5. 性能ベンチマーク

### 5.1 二層構成

| 層 | 道具 | 目的 | 実行場所 |
|---|---|---|---|
| マイクロ | `criterion` (wall-clock、統計) + **`iai-callgrind`** (命令数・決定的。**Valgrind が必要なため Linux 専用**。0.16.1 (2025-07) 時点。**v0.2 から**) | eve-text / ChangeSet / 幅計算 / デコード / Frame 生成の回帰検出 | ローカル (criterion)。iai の CI は v0.2 (Linux)、Windows の wall-clock の同一ジョブ A/B も v0.2 |
| マクロ (NFR 対応) | 自前ハーネス `eve-bench` (bin。置き場所は `crates/eve-bench/`、`publish = false`。`eve-testkit` 上でヘッドレス `Editor` + **実 Port**) + 外部計測 (`hyperfine`、Windows のプロセスのメモリ・CPU 指標) | NFR-1xx / 2xx の数値を測る | 基準環境 (Windows の開発 PC、リリース前)。CI では値を記録するだけ |

CI ランナーの wall-clock は変動が大きい (共有 VM) ため、絶対値 (ms) は基準環境でのみ判定する。**v0.1 は CI で回帰を自動判定しない**: `bench.yml` を手動で実行して (マイルストーンを `develop` に取り込む前など) BM-101a・BM-103a・BM-205 の値を記録し、劣化は人が推移を見て判断する (トリガは §6.1)。自動の回帰判定 (iai-callgrind の命令数 (閾値 ±5%、主要 5 本程度) と、マクロベンチの **同一ジョブ内 A/B 比較** (main と PR を同じランナーで連続実行し比率を見る。閾値 +10%)) は **v0.2 から** (01 NFR-703)。

### 5.2 NFR とベンチ ID の対応

| NFR | ベンチ ID | 測り方 | 開始点 / 終了点 | 環境 |
|---|---|---|---|---|
| NFR-101 起動 (ファイルなし、設定ファイルあり) | BM-101a (**(a) プロセス内**、判定値・CI の記録) / BM-101b (**(b) spawn から**) | 01 §3.0.1 の 2 区間を別のベンチにする。`evoeditor --bench-exit-after-first-frame` (**計測用ビルド** の隠しフラグ: 最初の Frame を出力した直後に終了。計測用ビルドは配布ビルドと同じ release プロファイル・同じ feature に `test-hooks` と `bench-trace` を足したもので、test-hooks のフックと span の記録は環境変数 (`EVE_TEST_HOOK`・`EVE_TRACE_FILE`、§3.6) で有効にしない限り何もしない。`--bench-*` は配布ビルドに含めない、§10.3) を 1,000 回 (ウォームアップ 100 回を除く、01 §3.0) 起動する。(a) はプロセス内の `tracing` span (`main` 入口 → 最初の Frame の `write` + `flush`) を `EVE_TRACE_FILE` に出力させて集計 (§3.6 の「計測の記録」)、(b) は計測ハーネス (`hyperfine` 等) が spawn した時刻から同じ終点まで。**UI の言語**: v0.1 は `ja` だけで、言語ファイルのデシリアライズを含めて目標を満たすこと (07 §17.1、NFR-603)。設定なしの起動は参考に別記録。**内訳** を `tracing` span で記録: 引数解析 / 設定読込 (予算 2 ms、07 §3.2) / 端末初期化 / スレッド起動 / 最初の Frame。クリップボード初期化・`bg` ワーカー・ログスレッド・`state_dir` の作成は起動経路に乗らないこと (02 §9.5、05 §7.3) を内訳で確認。既定キーマップ・ヒントの定数表・端末の横取り表 (04 §7.3a) は定数表で、起動時に TOML を解析しない。組込みテーマの段の解決 (truecolor・mono と 256 / 16 の近似、07 §8.3) と言語ファイルの読込は「設定読込」の内訳に含めて記録する。判定は基準機 ENV-W1 の条件 (サードパーティ製のウイルス対策ソフト有効・Defender 無効、01 §3.0) の値で行う (§5.3 (c)、OQ-23)。Defender 有効の値は参考値として別の行に記録する | (a) `main` 入口 → 最初の Frame の `write` + `flush` が戻る / (b) spawn → 同じ終点 | 基準環境 (Windows) で判定。(a) は CI で記録 |
| NFR-102 起動 (1 MB) | BM-102a / BM-102b | 同上 + ファイル引数 ((a) / (b) とも) | 同上 | 同上 |
| NFR-103 キー入力遅延 (計測範囲は 01 §3.0.1 が正本) | BM-103a (core 区間) | `eve-bench input-latency`: ヘッドレスで `InputEvent::Text("a")` → `frame()` 完了までを `tracing` span (`input.handle` → `render.frame`) で 10,000 回計測、p50/p99。**診断と CI の記録用** (目標値の合否判定には使わない) | (a) `handle_input` 入口 → Frame 生成完了 | CI の記録 + 基準環境 (診断) |
| | BM-103b (**判定区間**) | 実バイナリを pty で起動し、`tracing` span で区間を分解: 入力スレッドの read 完了 → チャネル → `handle_input` → Frame → ratatui 差分 → `write`+`flush` 完了。**NFR-103 の目標 (p50 2 ms / p99 8 ms) はこの区間に適用** (OQ-113、確定 (ユーザー回答))。p99.9 (目安 16 ms) は参考値として記録し、ゲートにしない (01 §3.0 の「統計」)。**条件**: 01 §3.0.1 の NFR-103 の計測条件 (端末 120×40、1 MB の日本語混在 UTF-8、検索一致のハイライト有効、打鍵間隔 30 ms) で **1,000 回** の文字入力を基準機 (Windows、ConPTY) で行い、**p50 / p99 で判定** して p99.9 を参考値として記録する (ConPTY では `write` がブロックし得るので、その影響を含めて測る)。**負荷下の判定** (同じ条件・同じ目標): 100 MB の保存ジョブ中 (NFR-107 AC-1)、100 MB の全件検索中 (FR-404 AC-1)、保存直前の照合 I/O 中 (FR-117 AC-2)、遅いクリップボード取得の待ち中 (FR-205 AC-3)、上限未満の 60 MB の全選択のコピー中 (文字列化とシステムクリップボードへの書込を含む。FR-205 AC-4 (b))、100 MB の全選択のコピー中 (内部レジスタだけへのコピー。FR-205 AC-4 (a) の時間側) | (b) 入力バイト読取完了 → `flush` 呼出し完了 | 基準環境 (Windows) で合否 |
| | BM-103c (端末込み、参考) | ConPTY で「入力バイト書込 → 出力バイト到着」。08 §3 S-13 (基準機で NFR-103 が未達のときだけ行う) の道具 | (c) pty 入力 → pty 出力 | 基準環境 (S-13 を行う場合だけ) |
| NFR-104 サイズ別オープン | BM-104-{1m,10m,100m} | `eve-bench open <file>`: 01 NFR-104 の 2 区間を `tracing` の span で別に測る: (1) **「開く」コマンド → 最初の進捗の Frame** (進捗表示を含む Frame の生成の完了。進捗を出さない小さいファイルは測らない)、(2) **「開く」コマンド → 読込完了** (`load_state == Ready`。Partial 表示は v0.3 で別計測)。回数は 1 MB / 10 MB は 1,000 回 (ウォームアップ 100 回を除く)、100 MB は **30 回で p50 と最大値** (判定は p50、最大値は参考)。Shift_JIS 100 MB (再エンコード照合込み、03 §8.7) も測り 1.5 倍許容内かを確認 (S-01)。1 GB と、Linux の musl 版と gnu 版の比較 (OQ-112) は Linux を出荷する版で | (1) コマンドの受付 → 最初の進捗の Frame / (2) コマンドの受付 → `Ready` | 基準環境 (Windows) |
| NFR-105 巨大ファイル編集 | BM-105 | 100 MB 中央へ挿入 → Undo。1,000 回 (ウォームアップ 100 回を除く、01 §3.0)。コマンドと core が要るので、M1 の core 完成後 (ヘッドレス・`MemFs`) に測る (S-02 の判定、下の BM-S02) | コマンド実行時間 | CI + 基準 |
| S-02 のテキストバッファ判定 (ADR-0004。NFR-104 / 105 / 202 を M1 の縦切りで先に見る代理) | BM-S02 | `eve-text` のベンチ (`crates/eve-text/benches/bm_s02.rs`。p50 / p99 を出すため、criterion ではなく標準ライブラリだけの自前のハーネス (`harness = false`)。メモリ (Private Bytes) は dev 依存の `memory-stats`)。**入力はメモリ上のバイト列** (`xtask gen-corpus` の決定的データを `Vec<u8>` に読んだもの。ファイル I/O・コマンド・Port・core を経由しない。M1 の縦切りの (1) で eve-text だけで測れる)。100 MB (平均行長 80 B) のバイト列からの `TextStorage` の構築時間とプロセスのメモリ (Private Bytes) の倍率、`cr_lines` 有効・`unicode_lines` 無効での行計算、CRLF がチャンク境界で分割されないこと、byte⇔char 変換、clone、中央への 1 文字挿入と逆の `ChangeSet` の適用 (Undo 相当)、チャンクを跨ぐ書記素判定、1 バイト削除を Snapshot で保持したときの未計上メモリ (02 §14.1 MR1 の閾値の根拠)。判定は暫定で、各値が NFR-104 / 105 / 202 の目標値に収まる見込みであること | 入口 → 完了 (p50 / p99 / 最大値。自前のハーネス) | 基準環境 (Windows)。結果は 10 の ADR-0004 / OQ-06 / 53 に記録 (08 §3) |
| NFR-106 長大行 (OQ-31) | BM-106 | 1 行 10 MB を含む文書: 開く / 行末へ移動 (`End`) / 横スクロール / 垂直スクロール / 中央挿入 (v0.2) / Frame 生成。**v0.1 の判定**: `long_line_threshold` (1 MiB) を超える行を含む文書は読み取り専用で開き、その行は先頭 64 KiB だけを表示する (03 §11.5) 経路で、各操作の **p99 ≤ 100 ms** かつ Frame 生成が止まらない (S-03)。**v0.2 の判定** (01 NFR-106 AC-1): char 近似モード (03 §11.5.1) で、各操作 (中央挿入 (`insert_mid`。1 文字の挿入 → Frame。列の索引の作り直しを含む) を含む) の **p99 ≤ 100 ms**。読み取り専用で開いたことの表示 (`info.open.long_line`・`status.degraded.long_line.long`) は L3 のシナリオで検査 | 各操作 | 基準環境 (Windows) |
| NFR-307 スワップ (v0.2) | BM-103b-swap60 | 60 MB (64 MiB 以下) の dirty な文書を開き、最後の入力から約 2 秒後に全文のスワップが書かれる間、30 ms ごとにカーソルを左右へ動かして BM-103b を測る (書出しは UI を止めない、01 NFR-307)。判定は NFR-103 の目標 (p99) を満たすこと。`cargo bench -p evoeditor --features bench-trace --bench bm_pty -- --only 307` | 入力の読取完了 → 描画の write | 基準環境 (Windows) |
| NFR-107 保存 | BM-107-{10m,100m} | 保存ジョブ (fsync Full) の完了まで。**実 FS (Windows、C: の NVMe)**。**保存時間は参考値として 1 回記録するだけ** (01 NFR-107)。判定は「100 MB の保存中も入力が NFR-103 を満たす」(BM-103b を保存ジョブの実行中に測る、01 NFR-107 AC-1) | ジョブ開始 → SaveReport | 基準環境 (Windows) |
| NFR-108/109 検索 (Should、記録) | BM-108 / BM-109 | 100 MB、**リテラル (memmem、03 §12.1 の経路 1・2)**: 最初の一致 / 全件走査。検索語は (i) ASCII 語 (`k` `K` `s` `S` を含まない。Sensitive と ASCII 大小無視の両方)、(ii) **日本語の語** (ひらがな・カタカナ・漢字だけの語は大小の区別が無いので `Insensitive` / `Smart` でも Sensitive と同じ `memmem` になる。BM-108j / BM-109j として 3 文字・10 文字の 2 通り) の両方で測る。`k` / `s` を含む ASCII 語と ASCII 以外の区別のある文字 (全角英字等) を含む語は行単位の正規表現経路 (03 §12.1 の経路 3) になるので BM-109r と同じ扱い (参考値)。所要時間は **参考値として 1 回記録** (01 NFR-108。判定は入力を止めないこと、FR-404 = BM-103b の負荷下の判定)。結果は 256 件 / 20 ms 単位で届くこと (MR3) は L1 で検査。正規表現 (行単位) の BM-109r は v0.2 | ジョブ開始 → 各イベント | 基準環境 (参考) |
| NFR-110 ハイライト (v0.4) | BM-110 | 10,000 行編集時の再解析 → Decoration 到着。同時に BM-103 を満たすか | — | v0.4 |
| NFR-111 大量ペースト (Should、記録) | BM-111 | 10 MB `Paste` → Transaction 完了。その間の Frame 生成が止まらないこと (ジョブ化していれば)。参考値として 1 回記録 | — | 基準環境 (参考) |
| NFR-105 の補助: 巨大削除の `eol_stats` 更新 | BM-EOL | 100 MB の改行を多く含む文書の全選択削除 (1 Transaction) で、`eol_stats` の更新 (03 §6.5 の「巨大な削除」: 行数差 + バイト計数) が数十 ms の見込みに収まるか。1 MB・10 MB・100 MB で計測し、超える場合は 03 §6.5 の代替 (実体化ジョブでの計数) に切り替える材料にする。参考値として記録 | Transaction の適用 | 基準環境 (参考) |
| NFR-112 終了 (Should、記録) | BM-112 | ConPTY で起動し、最初の Frame が出た後に `Ctrl+Q` を送ってからプロセスが終了するまで (未保存の変更なし)。参考値として記録する。隠しフラグ `--bench-quit-immediately` で `hyperfine` から測る方式は v0.2 | `Ctrl+Q` の送信 → プロセス終了 | 基準環境 (参考) |
| NFR-115 AI 処理中の入力遅延 (v1.L) | BM-115 | `MockLlamaServer` (§3.9) を実時間で 50 tok/s のストリーミング (チャット 1 本 + 補完要求の発行と破棄を 300 ms ごと) に設定し、その間 BM-103a / BM-103b を同条件で測る。管理モードのサーバ起動中 (`ManagedServerSim` の `/health` 待ち) も別に測る。**判定: p50 / p99 が無負荷時の +10% 以内、かつ NFR-103 の値を満たす**。メインスレッドの AI 関連処理 (`ai.*` span) は 1 メッセージ 1 ms 以内 | 入力 → `flush` (BM-103 と同じ) | 基準環境 (3 OS) + CI 相対 (Linux) |
| FR-917 (c) プロンプト安定性 (v1.L-a) | BM-116 | 1 MB の文書でカーソル付近の編集・移動・選択変更を 1,000 回行い、各回の補完 / 指示編集要求のプロンプトについて **KV キャッシュ再利用率** (前回プロンプトとの最長共通接頭辞トークン数 ÷ プロンプト長。`MockLlamaServer` の `/slots` / `timings` 相当で取得) を記録。**判定: 中央値 ≥ 0.9、最小値 ≥ 0.5** (閾値は S-22 (c) の実機の TTFT 短縮と合わせて調整)。接頭辞に含めてはいけないもの (カーソル位置・時刻・乱数・選択範囲の長さ) が混入すると再利用率が落ちるので、回帰検知に使う | 要求ごとの比率 | CI 相対 (Linux) |
| NFR-116 補完の体感遅延 (v1.L-a、Should) | BM-117 | S-22 の基準条件 (実 `llama-server`、FIM 対応の 1.5B〜7B 級コードモデル、GPU または Apple Silicon、プロンプトキャッシュ有効、prefix 1,024 / suffix 256 トークン) で、デバウンス満了 → 最初のゴーストテキスト表示まで。模擬サーバでは測れない (TTFT はモデルとハードウェア依存) ので **基準環境の実機のみ**。模擬サーバでは「evoeditor 側の経路 (要求の構築 → 送信 → 最初の断片の受信 → Frame 反映) が p99 20 ms 以内」を別に測り、TTFT 以外の遅れが無いことを確認 | デバウンス満了 → `flush` | 基準環境 (実機、リリース前) |
| NFR-201 アイドルメモリ | BM-201 | ファイルなしで起動し、最初の Frame を描いてから 10 秒アイドルの後に取得。指標は 01 NFR-201 の Windows の指標 `GetProcessMemoryInfo` の PrivateUsage (Private Bytes) — `eve-bench` 内で自己計測し出力 (Linux の VmRSS・macOS の `phys_footprint` は、それらを出荷する版で) | — | 基準環境 (Windows) |
| NFR-202 オープン時メモリ | BM-202-{1m,10m,100m} | 読込完了から 1 秒アイドルの後の指標 (BM-201 と同じ指標) を **絶対値** で判定: デコード後 UTF-8 サイズ × 1.5 + 15 MB 以下 (許容 × 2.5、01 NFR-202)。100 MB の Shift_JIS も測る。**基準はデコード後の UTF-8 のサイズ** (01 NFR-202。Shift_JIS 100 MB は UTF-8 で約 150 MB)。不正バイト 1% 超のファイルは対象外 (FR-127 の警告を検査) | — | 基準環境 (Windows) |
| NFR-203 Undo 会計 (02 §14.1 MR1 / MR2) | BM-203 | 指標は **Private Bytes** (BM-201 と同じ `GetProcessMemoryInfo` の PrivateUsage)。100 MB を開き、**基準点 = 読込完了から 1 秒アイドルの後** の値を取る。ランダムな 1 万回の編集 (小さな削除 90%、64 KiB 以上の削除 10%) の後、Undo の実体化ジョブ (03 §7.5) がすべて完了してから測り、基準点からの増分 ≤ Undo 会計の合計 × 1.2 (01 FR-203 AC-3)。この判定は **Undo の破棄が起きない範囲** で行う: 計測用ビルドの test-hooks (`undo_limit_expand`、§3.6 の一覧) で文書あたりの上限を一時的に拡大する (例 1 GiB / 100,000 ステップ) か、破棄が起きない編集回数にする。破棄が起きたら計測を無効として記録する。別のケースとして既定の上限 (64 MiB / 10,000 ステップ) のまま同じ編集を行い、破棄の後の増分が残っている Undo の会計 × 1.2 以下に下がる (破棄に Private Bytes が追随する) ことを確かめる (v0.1 は単一文書。プロセス全体の上限の検査は複数文書の v0.2) | 基準点 (読込完了 + 1 秒) → 編集後 (実体化ジョブの完了後) の Private Bytes | 基準環境 (Windows) |
| NFR-204 アイドル CPU | BM-204 | 60 秒入力なし (v0.1 は監視なし): Windows (`Get-Counter '\Process(evoeditor)\% Processor Time'`) で **CPU 使用率** を測る (起床回数の厳密計測は ETW 等が必要。01 NFR-204)。Linux の `perf stat` / `strace -c` による起床回数の厳密判定は Linux を出荷する版で | — | 基準環境 (Windows)、リリース前 |
| NFR-205 バイナリサイズ (Should、記録) | BM-205 | 配布物の `evoeditor.exe` サイズ。CI の `build-release-smoke` (毎 PR) で計測して artifact に置き、推移は `bench.yml` で記録。`cargo bloat --release --crates` を成果物に。同梱の言語ファイル (`ja`、見込み約 50 KB、07 §17.1) の大きさも別に記録する | — | CI (Windows) |
| NFR-306 長時間 (v0.2 から) | BM-306 | `eve-bench soak --hours N`: ランダム編集 + Undo/Redo + 検索 + 保存を繰り返し、RSS と BM-103a の推移が平坦。メッセージ履歴・検索履歴・レイアウトキャッシュの上限 (MR8) が効いていること。**v0.2〜v0.4 は夜間に 1 時間** (傾向の観察。ゲートにしない)、**8 時間の判定は v1.0** (01 NFR-306)。v0.1 では行わない | — | 夜間 (v0.2 から)、8 時間は v1.0 のリリース前 |
| 05 §17.2 ビルド時間 (対応する NFR なし) | BM-B01 | `cargo build --release --timings` クリーン。v0.1 は参考に 1 回記録 | — | 基準環境 (参考) |
| NFR-207 依存数 | BM-B02 | `xtask check-deps --count`: 推移的依存 (`x86_64-pc-windows-msvc`、既定 feature、normal。05 §17.2) の総数と C を含む依存の有無。**C を含む依存 0 は毎 PR の失敗条件**、総数 ≤ 90 は **報告** (超えたら警告。NFR-207 は Should で v0.1 のゲートにしない、ADR-0039)。推移を `bench.yml` で記録 | — | CI |

- **S-02 の最終判定の時期** (提案、OQ-173。08 §2.2 / §3): BM-S02 は M1 の暫定判定。BM-105 は M1 の core 完成後、BM-104 / BM-202 は実 Port で読み込める M3 で測る。ADR-0004 の最終判定は M3 の完了時 (BM-104 / 202 の判定後)で、M5 で全件を再確認する。BM-S02 が不合格なら M1 の中で crop へ切り替える。
- 全ベンチは `xtask gen-corpus` の決定的データを使う (§4)。
- **v0.1 で判定に使うベンチ** は NFR-101〜106・NFR-201〜204 のもの (BM-101a/b〜BM-106、BM-103b の負荷下の判定 (NFR-107 AC-1・FR-404・FR-117・FR-205)、BM-201〜BM-204)。それ以外 (BM-107 の時間、BM-108 / 109、BM-111、BM-112、BM-205、BM-B01) は参考値として 1 回記録するだけ (Should の要件の記録・監視)。BM-S02 は ADR-0004 (S-02) の暫定判定用で、NFR の AC の判定には使わない。判定に使うベンチは 01 §3.0 の「1,000 回 (ウォームアップ 100 回を除く)、p50 / p99 で判定し、p99.9 は参考値として記録 (ゲートにしない)」に従う。例外: BM-103a は判定に使わない診断用なので 10,000 回、1 回が数秒以上かかるもの (BM-104 の 100 MB、BM-107、BM-108 / 109 の 100 MB) は 30 回で p50 / 最大値を記録する (判定に使う BM-104 の 100 MB は p50 で判定し、最大値は参考)。
- **実バイナリ (pty、`bm_pty`) で測る判定用ベンチの実装上の ID** (`cargo xtask bench` が `bench-trace` つきで実行する。仕様上の ID との対応):
  - `BM-104p-10m` / `BM-104p-100m`: NFR-104 (1)・FR-122 AC-1 の「開く → 進捗」。`Ctrl+O` → パス → `Enter` の、`Enter` の読取完了 → 進捗 (読込中) を含む最初の Frame の `write`。2 つのファイルを交互に開く。10 MB は p99 ≤ 50 ms、100 MB は 30 回の p50 ≤ 50 ms で判定する。
  - `BM-512`: FR-512 AC-1 の「リサイズ → 再描画」。端末の大きさを 50 ms ごとに変え、最後のリサイズイベントの読取完了 → 新しい大きさの Frame の `write`。p99 ≤ 16 ms で判定する。
  - `BM-103b-search`: FR-404 AC-1。100 MB を開き、検索バーに 30 ms ごとに 1 文字ずつ打つ (一致しない語。全件の走査が走り続ける)。キー入力の判定区間 (BM-103b と同じ) を p50 2 ms / p99 8 ms で判定する。検索語は `k` `s` を含めない (含むと行ごとに畳み込む遅い経路になる。03 §12.4。その経路の同期走査は量を絞って単発のジョブに回す、03 §12.2)。
  - `BM-103b-reconcile`: FR-117 AC-2。15 MB (照合のために内容を読み直す上限以下) の文書の保存中のカーソル移動を、BM-107 と同じ方法で測り、p50 2 ms / p99 8 ms で判定する。
  - `BM-103b-clip60` / `BM-103b-clip100`: FR-205 AC-4。60 MB (文字列化とシステムのクリップボードへの書込を含む) と 100 MB (64 MiB の上限を超えるので内部レジスタだけ) の文書を全選択してコピーし、その間と後に 30 ms ごとにカーソルを左右へ動かす。キー入力の判定区間 (BM-103b と同じ) を p50 2 ms / p99 8 ms で判定する。上限の判定が文字列化の前に総バイト数で行われることは、eve-host の試験で確かめる。
  - `BM-101c` / `BM-102c` / `BM-103c` / `BM-107c`: 端末込みの参考値 (ConPTY で画面に届くまで。`cargo xtask bench --env` が判定用の値と並べて記録する、11 §1 の `docs/perf/`)。判定には使わない。
  - `BM-201` / `BM-204`: NFR-201 / NFR-204。`eve-bench` の自己計測ではなく、`bm_pty` が PowerShell の `Get-Process` で、最初の Frame の 10 秒後の Private Bytes (5 回の中央値) と、その後 60 秒間の CPU 時間の増分から使用率を求める。BM-201 は 10 MB 以下 (許容 20 MB)、BM-204 は 0.1 % 未満で判定する。
- 測定前に **ウォームキャッシュ** を作る (ファイルを 1 回読む)。コールド値は `--cold` オプションで別途 (Windows はファイルキャッシュを空にする手段が限られるので、再起動直後の値を参考に記録する)。
- `tracing` の span の記録は `bench-trace` の feature と `EVE_TRACE_FILE` で行う (形式・有効化・オーバーヘッドは §3.6 の「計測の記録」)。ボトルネック解析には `EVE_TRACE_FORMAT=chrome` で Chrome trace (`tracing-chrome`) を出す (02 §11.3)。

### 5.3 基準環境 (OQ-23) と記録

- 基準環境は **Windows の開発 PC 1 台** (確定 (ユーザー回答)、OQ-23。構成の正本は 01 §3.0)。**環境 ID** (例 `ENV-W1`) で識別し、`docs/perf/environments.md` には環境 ID と一般化した構成 (01 §3.0 と同じ粒度: OS の系列、CPU のコア数の級、メモリの概数、ストレージの種類とドライブ、端末と PowerShell の系列、ウイルス対策の種別 (製品名は書かない)、Rust 版) だけを登録する。**機種名・CPU や SSD の型番・容量・OS のビルド番号・ウイルス対策ソフトの製品名・電源プランの名前などの詳細は非公開** とし、リポジトリの外のメモ (git 管理外) に環境 ID と対応付けて記録する (11 §9.7)。計測時は電源接続、電源プランは記録した既定のまま、他アプリ終了。v0.1 は Windows だけを出荷するので、判定はすべてこの基準機で行う。
- **計測の運用 (確定 (ユーザー回答)、OQ-23 の (a)〜(d)、10 の残件 #3)**:
  - (a) 低速側の条件 (コア数・メモリを制限した条件) での参考値は v0.1 では取らない (v0.2 で決める)。
  - (b) **I/O 系の計測場所**: BM-104 / 107 / 108 / 109 / 202 と BM-B01 (cargo のビルド時間) は、リポジトリと `xtask gen-corpus` の出力を **内蔵 SSD 上の作業ディレクトリにコピーして** 測る。USB 接続の外付け SSD 上の値は参考として、ストレージの種類と接続方式を条件に記録する (ドライブ文字は書かない)。
  - (c) **起動時間とウイルス対策**: BM-101b / 102b (spawn から) は「サードパーティ製のウイルス対策ソフト有効・Defender のリアルタイム保護は無効」の条件で判定し、条件 (製品名ではなく種別) を結果に書く。Defender 有効の値 (一般の利用者の環境) は判定に使わない **参考値** で、別の環境 (Defender が有効な PC・VM、または利用者の報告) で取れたときに別の行に記録する。GitHub Actions の Windows ランナーで Defender のリアルタイム保護が有効かは未確認。
  - (d) **macOS / Linux**: v0.1 では判定しない。macOS / Linux は v0.2 で出荷する (確定 (ユーザー回答)、OQ-15) ので、v0.2 着手前に macOS (Apple Silicon) と Linux x86_64 の実機を基準機に加える (確定)。実機確認の手段が無い OS は v0.2 で出荷せず次の版に延期する。実機が無い OS の性能は CI の相対の確認に留め、絶対値の判定はしない。
- 結果は `docs/perf/<version>/` に記録する: ベンチ ID ごとの JSON (`docs/perf/<version>/<ベンチ ID>.json`。`xtask bench` の出力。p50/p99・最大値・回数・環境 ID・コミット) と、それを表にした要約 (`summary.md`。合否を含む)。計測の手順 (コマンド) の正本は 11 §1 の `docs/perf/` の行。各結果には **環境 ID だけ** を書き、機種・型番は書かない (bench の出力も同じ形式にする)。**不合格の Must NFR があればリリースしない** (§8.2)。「最低環境」(2 コア 4 GB) の記録は v0.1 では行わない (01 §3.0)。

### 5.4 回帰の運用

| 事象 | 対応 |
|---|---|
| iai 命令数 +5% 超 / マクロ A/B +10% 超 (v0.2 から) (NFR-703)。v0.1 は `bench.yml` の記録の推移を人が見る | PR に警告コメント (ブロックしない)。原因説明または修正を求める。意図的な劣化はベースライン更新を PR に含める |
| `bench.yml` の記録で連続 3 回の劣化 | 課題起票。リリースブロッカー候補 |
| ベンチ自体の flaky | ランナー変更 or 閾値調整を別 PR で |

---

## 6. CI (GitHub Actions)

### 6.1 ワークフロー一覧

| ワークフロー | トリガ | 内容 | 所要目標 |
|---|---|---|---|
| **速い車線** `cargo xtask check` | ローカル / PR の最初のジョブ (`ci.yml` の `check` ジョブ、§6.2) | **v0.1 の中身 (11 §8.1・08 §2.2 と同じ。この 3 か所を同時に直す)**: (1) fmt、(2) clippy、(3) `cargo nextest run -p eve-text -p eve-encoding -p eve-core`、(4) `check-deps`、(5) `tests/ac-checklist.toml` の軽い検査 (11 §3.3)、(6) 変更した crate の `Harness` のシナリオテスト。v0.2 の M10 から (5) の後に `spec-index --check`。ラベルによるシナリオの選択と `spec-index` の最新性は v0.2。**2 分以内** を目標 (AI エージェントの検証ループの単位。11 §8) | 2 分 |
| `ci.yml` (フル車線) | PR (宛先が `develop`・`main`) と、これらへの push | §6.2 のジョブ群 | 15 分 (最長ジョブ) |
| `nightly.yml` | 毎日 (UTC 18:00 = JST 03:00)。`schedule` は既定ブランチのワークフロー定義で動く (v0.1.0 のリリースまでは `develop`、以後は `main`。2026-10-27 に `main` に戻した。OQ-170、11 §9.1)。そこで既定ブランチには **薄いトリガ** (スケジュールと `workflow_dispatch` と、次の checkout と入口の呼出しだけを持つ定義) を置く。**対象のブランチ**: 薄いトリガは `actions/checkout` を `ref: ${{ vars.NIGHTLY_REF \|\| 'develop' }}` (リポジトリの変数 `NIGHTLY_REF`。未設定なら `develop`) で行い、チェックアウトしたコードの **`cargo xtask nightly`** を入口にして実体の手順を実行する (手順の中身は対象のブランチの `xtask` にあるので、薄いトリガを直さずに済む)。手動実行では引数で ref を上書きできる。対象は `develop` のままでよい (長命ブランチは `main` と `develop` だけで、リリース候補の期間 (凍結 (§9.2 手順 1) から publish まで) も `develop` を対象にするので、変数の切替は要らない。§8.2 の fuzz の行)。再利用ワークフロー (`uses: <owner>/<repo>/.github/workflows/nightly-impl.yml@develop` 相当) で実体を呼ぶ方式も同等とするが、`uses` の ref は変数で切り替えられないので、変数で対象を変える上の方式を既定とする | `fuzz_decode`、proptest 拡張、`cargo audit`、カバレッジ報告 (`coverage`) (§6.3)。手動実行 (`workflow_dispatch`) では対象のコミットと fuzz の時間を引数で指定でき、リリース前の 1 時間の fuzz (§8.2) に使う。`cargo outdated` 報告・1 時間の soak・`docs-rs`・`unsafe-audit` (`cargo geiger`)・最新依存での再チェック・`udeps` は v0.2 | — |
| `bench.yml` | 手動 (`workflow_dispatch`)。`nightly.yml` と同じく既定ブランチには薄いトリガだけを置き、`ref: ${{ vars.NIGHTLY_REF \|\| 'develop' }}` (引数で上書き可) でチェックアウトしたコードの `cargo xtask bench` を入口にして実行する。v0.2 から `develop` への push でも (11 §8.1) | v0.1: BM-101a・BM-103a・BM-205 (サイズ) の値の記録 (Windows)。**記録先**: v0.1 は Actions の artifact (`xtask bench` の JSON) で、リポジトリへのコミットも `gh-pages` への push もしない (権限は `contents: read`、§6.5)。判定に使う値は基準機の記録 (`docs/perf/<version>/`、§5.3。手順は 11 §1) で、CI の値は推移を見るだけ。v0.2: iai + マクロ A/B と、`gh-pages` の `bench/` へのグラフ化 (github-action-benchmark 等) | 20 分 |
| `release.yml` | タグ `v*` (正式版は `main` のマージコミットに、プレリリース (`-alpha.N` / `-beta.N` / `-rc.N`) は `develop` の先頭に、人間が付ける、§9.1 / §9.2) / 手動 (dry-run) | **最初のステップでタグを検証する** (11 §9.3。検証の置き場所は本ワークフローに統一する): (1) `git verify-tag` (署名の鍵はリポジトリの `.github/allowed_signers` に載る鍵だけを許す。`gpg.ssh.allowedSignersFile` に指定)、(2) 正式版のタグは `main` のマージコミットを指す (`main` から到達でき (`git merge-base --is-ancestor`)、親が 2 つあること)。プレリリースのタグは `develop` の先頭を指す、(3) タグの版と workspace の `Cargo.toml` の版が一致する (プレリリースのタグは `-rc.N` 等の接尾辞まで一致する。そのため、プレリリースの前に `develop` に `-rc.N` 等の版を付ける PR を入れる、§9.2)、(4) **内容の一致** (正式版): タグのコミットと、リリースのチェックリストに記録したゲートの判定のコミット (版上げ後の `develop` の先頭、§9.2 手順 3) の差分 (`git diff --name-only`) が許可リスト (記録の置き場所 `docs/compat/<version>/`・`docs/perf/<version>/` と、AC のチェックリスト `tests/ac-checklist.toml`) のファイルだけである。どれかが失敗したら以後のジョブを走らせない。続けて §9 のビルド・公開の準備 (署名はメンテナの手元) | 40 分 |
| `docs.yml` | PR (宛先が `develop`・`main`) と、これらへの push。**パスのフィルタを付けない** (コードだけを変える PR でも走らせる。生成リファレンスはコードから生成するため) | markdown lint、リンク検査、生成ドキュメントの最新性 (§11.2。`cargo xtask gen-reference --check` を windows のランナーで実行し、`docs/reference/*` を JSON (`--list-commands --json` 等) から Markdown に再生成した結果との差分があれば失敗)。docs の表を入力にする検査は `ci.yml` の `docs-tables` ジョブで行う (§6.2)。必須のステータスチェックの **2 段目** に加える (§6.2 の「必須のステータスチェックの段階的な追加」) | 3 分 |
| `ai-local.yml` (v1.L、**任意・手動 / 夜間週 1**) | 手動 / スケジュール | 実 `llama-server` (固定版) + 小さい GGUF による契約テスト (§3.9)、BM-116 の実機値の記録。CPU 推論なので遅い。失敗しても PR をブロックしない。モデル・サーバのダウンロードはこのワークフローだけに許す (ネットワーク既定なしの例外として `ci/ai-real.toml` に出自とハッシュを固定) | 30 分 |

### 6.2 `ci.yml` のジョブ

| ジョブ | ランナー | 内容 |
|---|---|---|
| `check` | windows | **速い車線** `cargo xtask check` (§6.1、11 §8.1)。PR の最初のジョブで、これが失敗したら他のジョブを始めない (`needs: check`)。`fmt` / `clippy` と内容が重なるが、ローカルと同じ入口が CI でも通ることを確かめるために置く |
| `fmt` | ubuntu | `cargo fmt --all --check` |
| `clippy` (表示名 `clippy (windows)` / `clippy (linux)`) | windows, ubuntu | `cargo clippy --workspace --all-targets --all-features -- -D warnings` (windows)。ubuntu は Linux CI の対象 crate (§6.4) だけ。macOS は macOS を出荷する版から |
| `deps` | ubuntu | `cargo deny check` (licenses / bans / advisories / sources。対象は `x86_64-pc-windows-msvc` と配布ビルドの feature。別の `Cargo.lock` を持つ `fuzz/` にも同じ検査を行う、§6.3。`bans` のネットワーク系の crate の禁止リストで NFR-501 AC-1 の (a) を検査、§3.6) + `xtask check-deps` (§7。`#![forbid(unsafe_code)]` が R5 対象外の crate にあることの検査を含む)。`cargo hack check --each-feature` (feature 組合せ) は v0.2 |
| `test` (表示名 `test (windows)` / `test (linux)`) | **マトリクス** §6.4 | `cargo nextest run` + `cargo test --doc`。シナリオ (v0.1 は Rust の Harness。各 crate の `tests/` に置く、§3.4) も nextest で実行する。`CONTRACT_VERSION` の定数があることの検査もここ |
| `test-min-features` (v0.2) | ubuntu | `--no-default-features` (AI/LSP/syntax を外した v0.1 相当のビルドが壊れていない) |
| `msrv` (v0.2) | ubuntu | `cargo +<rust-version> check --workspace --all-features` (`Cargo.lock` 使用)。v0.1 は `rust-version` を `Cargo.toml` に書き、リリース前に手で確かめる |
| `e2e` | windows | L5 (`--features test-hooks,bench-trace`、nextest profile `e2e`、retries=2。ConPTY の 10 シナリオ、§3.6。`bench-trace` はフレーム時刻 (4) と退避の各段の時刻 (8) の記録に使う) |
| `build-release-smoke` | windows | `cargo build --release` (LTO、配布ビルドと同じ feature) が壊れていないこと + BM-205 のサイズを artifact に。**配布ビルドにテスト専用の経路が無いことの機械的な確認**: (1) `cargo tree -e features` (配布ビルドの feature) に `test-hooks` / `bench-trace` / `headless` が現れない、(2) 生成した `evoeditor.exe` の文字列に test-hooks と bench-trace の目印 (`EVE_TEST_HOOK`・`internal.debug_panic`・`--bench-`・`EVE_TRACE_FILE`・`EVE_TRACE_FORMAT`) が無い、(3) `evoeditor --bench-exit-after-first-frame` が未知のオプションとして終了コード 2 で終わる (§10.3)、(4) CRT を静的にリンクしている (`+crt-static`、11 §1 の `.cargo/config.toml`): `dumpbin /dependents` の出力に `VCRUNTIME*.dll`・`MSVCP*.dll`・`api-ms-win-crt-*` が無い (05 §16.4 の確かめ方 (c) の自動化) |
| `spec-index` (v0.2) | ubuntu | `xtask spec-index --check` (`docs/spec-index.toml` が docs の規範行 ID と一致。NFR-708)。v0.1 は規範行 ID を付けず、節 ID と AC ID のチェックリスト (§8.3 / §8.5) で突合する |
| `check-themes` | windows | `xtask check-themes` (eve-tui の近似表 (04 §12.3) と bin の定数表を読むので windows のランナーで実行する。これらの表を OS に依存しない crate に移した場合は ubuntu でもよい): 組込みテーマの truecolor と mono の段の欠落、ロールの形、truecolor の段のコントラスト基準 (07 §8.1)、近似した 256 / 16 の段の前景と背景の衝突 (§3.5)。入力の 07 §8.2 のロール表は `docs-tables` ジョブと同じ規則 (11 §8.5。段のセルの属性語彙 `bold` / `underline` / `reverse` / `dim` / `none` と ` + ` の結合は 07 §8.2 の凡例を正とする) で読む |
| `docs-tables` | windows | **docs の表と実装の突合**。対象の表 (07 §8.2 のロール表・04 §7.3a の横取り表・04 §15.2 の MessageKey 目録・07 §11.6 のコマンドカタログ・02 §4.1 の crate 一覧の「禁止 crate」列) は、読む側の各検査が 11 §8.5 の機械可読の規則 (見出し・列名・列の順) で読み、規則どおりに読めなければ失敗する (別の xtask のサブコマンドは置かない)。表の読取りは crate **`eve-docs-tables`** (`publish = false`。docs の Markdown の表を読む lib) に 1 本化し、`xtask` と、eve-tui・eve-core のテスト (dev 依存) がそれを使う。本ジョブが行うのは: (1) 下の (2)〜(4) の実行、(2) 04 §7.3a の横取り表と eve-tui の定数表の一致と FR-606 の到達性検査 (§3.4。eve-tui / eve-core のテストを nextest で選んで実行)、(3) 07 §11.6 のカタログと CommandRegistry の一致: **v0.1 の M1** は eve-core のテスト (`eve-docs-tables` で 07 §11.6 を読み、eve-core の組込みコマンドの登録と突合する) で、**`perm` の列** も突合し、組込みコマンド全件の `CommandSpec.permission` がカタログの `perm` 列の値と一致する (01 FR-601 AC-2 の T)。**M2 以降** は eve-tui の統合テスト (eve-core と `eve-docs-tables` を dev 依存に持つ。既定キーマップとヒントの定数表を含めて突合する) を加え、bin の `--list-commands` / `--dump-keymap` の出力 (§11.2) も同じ表と突合する (bin は eve-tui のテストから呼べないので、bin の出力の突合は bin の統合テストで行う)、(4) 02 §4.1 の「禁止 crate」列と `deps-policy.toml` の一致 (`xtask check-deps --docs`)。04 §15.2 の目録と 07 §11.6 のヒント列は `i18n` ジョブが、07 §8.2 は `check-themes` ジョブが読む。docs と実装の片方だけを変えた PR で食い違いを止めるため、**コードだけを変える PR でも走らせる** (パスのフィルタを付けない) |
| `ai-mock` (v1.L から) | ubuntu, windows, macos | `--features ai` で eve-ai と `MockLlamaServer` のテスト (§3.9: 能力検出・予算一致・プロンプト安定性・キャンセル・注入・管理モードの `ManagedServerSim`)。ネットワークなし (ループバックのみ)。BM-115 / BM-116 は `bench.yml` 側 |
| `area-check` (v0.2) | ubuntu | R9 (§7): 変更ファイル集合と PR ラベル (`area:tui` 等) の整合を検査し、frontend crate の PR が `eve-core` / `eve-view` / `eve-config` を変更していたら失敗。`crates/eve-view/` を変更する PR にはラベルを要求する。v0.1 はレビュー (§8.4) で確かめる |
| `i18n` (v0.1 から) | windows | 同梱の言語ファイルの検査 (07 §17、NFR-603。eve-host / bin の定数表 (`stderr.*` / `panic.*` / `cli.*`) を読むので windows のランナーで実行する。定数表を OS に依存しない crate に移した場合は ubuntu でもよい): (1) **キー集合の差分**: コードの MessageKey 列挙と `locales/ja.toml` のキー集合を比べ、**欠落・余剰ともエラー** (差分ゼロが合格)。(2) `msg.*` のすべてが `text` と `hint` を持つ (NFR-602)。(3) **ヒント幅**: `hint.*` が全角 4 文字 / 半角 8 文字 (表示幅 8 セル) 以内で、07 §11.6 の「ヒント」列に v0.1 の文脈 (`normal` / `overlay`) がある CommandId のすべてに `hint.<command_id>` がある。比較は複数形の接尾辞 (`.one` / `.other`) を除いた基底キーで行い、言語ファイルに入れない `stderr.*` / `panic.*` / `cli.*` は除く (eve-host / bin の定数表にあり、04 §15.2 の目録とキー集合が一致すること、v0.2 からは英日の両方の行があることを別に検査する)。`info.*` は文 1 つ、`msg.*` は `text` と `hint` の両方、`ui.confirm.*` は選択肢ごとのアクセラレータが 1 つのダイアログの中で重ならないこと。(4) プレースホルダ名がコードの引数名と一致し、`{key:<command_id>}` の `command_id` が 07 §11.6 のカタログの v0.1 の ID である。(5) キー表記の直書きの検出 (`Ctrl+` 等の文字列が文言に無い)。(6) `cmd.<command_id>` の `title` / `description` が `hidden` でない v0.1 の全コマンドにある。(7) **キーの接頭辞の衝突**: あるキーが別のキーの接頭辞 (`.` 区切りの単位) になっていない (例: 実在のキー `status.degraded.long_line.long` に対して、その接頭辞 `status.degraded.long_line` を別のキーとして足すことを禁じる。TOML では同じ名前が文字列とテーブルを兼ねられず、言語ファイルを読めなくなるため)。コードの MessageKey 列挙と言語ファイルの両方を検査する。言語ファイルに入れない `stderr.*` / `panic.*` / `cli.*` の定数表のキーも同じ規則で検査する (例: 使い方の誤りの汎用の行は `stderr.usage.generic` で、`stderr.usage.multiple_files` 等と兄弟のキーにし、`stderr.usage` を単独のキーにしない。この組が (7) を通ることをテストのケースに含める)。言語間のキー集合の差分の検査は `en` を同梱する v0.2 で加える (`locales/en.toml` もコードの MessageKey 列挙と差分ゼロ、`ja` と `en` のキー集合も一致。`stderr.*` / `panic.*` / `cli.*` の定数表も両言語の行を持つ)。**`ja` の語彙・表記の品質** (用語の統一・自然さ) は機械では見ず、人手のレビュー (11 §8) |
| `frontend-parity` / `contract-export` (後の版。v0.1 には置かない、ADR-0039) | ubuntu | §3.5 の契約テスト: TUI × ヘッドレス参照レンダラの `CellOccupancy` の比較と Frame JSON ゴールデン (C3 の契約版 1 の固定時)、契約テストの成果物の添付 (12 §4.3)。v0.1 は `test` ジョブの ASCII Frame のスナップショットまで |
| `gitleaks` | ubuntu | 秘密情報の検出 (OQ-169)。PR の差分のコミットを `gitleaks` 相当の道具で検査し、検出があれば失敗 (検出の内容は要約に秘密の値を出さない)。M1 のタスク (0) から置く。道具のライセンスは 05 §17.1 の許可リストで確かめる (CI の道具で配布物には入らない) |
| `pr-source` | ubuntu | **PR の head ブランチの検査** (11 §9.1 のブランチ保護を補う軽いジョブ。`pull_request` のイベントの `base.ref` と `head.ref`・`head.repo` だけを見て、コードをチェックアウトしない)。**`main` が受ける PR の head は `develop` だけ**、**`develop` への PR の head は作業ブランチ (`feat/`・`fix/`・`spec/`・`chore/`・`spike/`・`agent/` で始まるもの、11 §9.1)・Dependabot (`dependabot/**`)・`main` (リリース後の取り込み、§9.2 手順 4) だけ** を許し、それ以外は失敗にする。いずれも同じリポジトリのブランチに限る。fork からの PR は base が `develop` のときだけ許し (head の名前は問わない。外部の寄稿者の PR)、`main` への fork の PR は失敗にする |
| `dco` | ubuntu | PR の全コミットに `Signed-off-by:` があること (DCO、確定 (ユーザー回答)、[OQ-154](10-decisions.md#oq-154-寄稿ポリシー)、11 §9.4)。エージェントの PR も同じ。**対象外** (ジョブの `if` で飛ばす。飛ばしたジョブは必須のステータスチェックとして成功扱い): (a) 昇格と取り込みの PR (`develop` → `main`、`main` → `develop` の merge commit の PR。中のコミットは取り込み元のブランチで検査済み。判定は `base.ref` と `head.ref` の組で行い、`pr-source` が許す組 (上の `pr-source` の行) と同じ範囲に限る)、(b) bot (Dependabot。v0.1 は他の bot を使わない、§9.2) の PR。**どちらも head のリポジトリが base と同じこと** (`github.event.pull_request.head.repo.full_name == github.repository`) を条件に含める (fork から同名のブランチで出した PR を対象外にしない)。bot の PR と作業ブランチの PR は squash で入れ、**squash のコミットメッセージに `Signed-off-by:` を残す** (merge する人間の名義。規則は 11 §9.4) |

**必須のステータスチェックの段階的な追加**: ブランチ保護 (11 §9.1) の必須のチェックには **存在するジョブだけ** を指定する (存在しないチェックを必須にすると、どの PR も merge できなくなる)。**最初の必須セット** (M1 のタスク (0) で CI を置いた時点で存在するジョブ。ブランチ保護に書く **表示名そのもの** で、11 §9.1 の固定名と一致させる): `check` / `fmt` / `clippy (windows)` / `clippy (linux)` / `test (windows)` / `test (linux)` / `deps` / `dco` / `gitleaks` / `pr-source`。**2 段目** として、ジョブを足す PR (例: `e2e`・`build-release-smoke`・`docs-tables`・`check-themes`・`i18n`、`docs.yml` の生成リファレンスの検査 (§6.1)) では、その PR が merge された直後に同じ担当者がブランチ保護の必須のチェックに加える (PR の説明に「必須に加えるジョブの表示名」を書き、チェックリストで確かめる。各ジョブを足すマイルストーンは 08 §2.2 の「CI・夜間 (v0.1)」の行、提案、OQ-172)。ジョブ名を変える PR も同じ手順で必須の名前を差し替える。

**道具の導入**: `cargo nextest` などの CI の道具は `taiki-e/install-action` (コミット SHA 固定、§6.5) で版を固定して入れる (`cargo install` でビルドしない)。**insta**: CI では `INSTA_UPDATE=no` を設定し (スナップショットを書き換えない)、テストの後にワークスペースに `*.snap.new` が 1 つでもあれば失敗にする (受け入れていないスナップショットの変化を見逃さない。`accept` は人間、11 §4)。

キャッシュ: `Swatinem/rust-cache`。`CARGO_INCREMENTAL=0`、`CARGO_PROFILE_DEV_DEBUG=0` (CI の速度)。`rust-toolchain.toml` で stable を固定しない (CI で stable / MSRV を明示する)。nextest の設定 (profile `default` / `ci` / `e2e`、`retries`、`slow-timeout`) はリポジトリ直下の `.config/nextest.toml` に置く。`ci.yml` のジョブにはパスのフィルタを付けない (docs だけ・コードだけの PR でも全ジョブが走る)。必須のステータスチェックにするジョブ名の固定は 11 §9.1。

### 6.3 夜間ジョブ

- `cargo +nightly fuzz run fuzz_decode -- -max_total_time=600` (v0.1 は `fuzz_decode` の 1 ターゲット、§3.3。v0.2 以降は 1200 秒 × 全ターゲット、Linux)。fuzz のターゲットとコーパスはリポジトリ直下の `fuzz/` (cargo-fuzz の標準の配置。本体の workspace から除外した別の crate) に置く。**公開のログと artifact に入力とスタックを出さない**: 公開リポジトリの Actions のログと artifact は第三者が読める。そこで fuzz は `-close_fd_mask=3` (対象の標準出力・標準エラーを閉じる) で走らせ、libFuzzer のクラッシュの報告 (入力の 16 進表示・スタック) がログに出ないようにする。**クラッシュ入力は暗号化した artifact として上げる** (確定 (ユーザー回答)、OQ-168): libFuzzer が書いたクラッシュ入力 (`-artifact_prefix` の下の `crash-*` 等) を、ジョブの中で **メンテナの公開鍵で `age` により暗号化** し、暗号化したファイルだけを Actions の artifact に上げる (平文のクラッシュ入力は artifact にもログにも出さず、暗号化の後にランナー上で消す)。受取人の公開鍵はリポジトリの **`.github/fuzz-recipients.txt`** (`age` の recipients ファイルの形式。1 行 1 鍵、メンテナの数だけ) に置き、秘密鍵はメンテナの手元だけに置く (CI にもリポジトリにも置かない、11 §9.7)。復号できるのはメンテナだけなので、artifact を第三者が取得しても入力は読めない。`age` は CI の道具で配布物に入らない (ライセンスは 05 §17.1 の許可リストで確かめる)。ジョブの要約に出すのは、ターゲット名・コミット・libFuzzer の `-seed` の値・クラッシュの件数・暗号化した artifact の名前だけ (入力の内容・ハッシュ・スタックを出さない)。**コーパスの置き場所**: (1) **リポジトリにコミットした最小コーパス** (`fuzz/corpus/<target>/`。`cargo fuzz cmin` で最小化した種。§3.3) と、(2) 夜間の実行で増えた分を持ち越す **Actions のキャッシュ** (同じリポジトリのワークフローからだけ読める。コミットはしない) の 2 つに分ける。夜間の実行は (1) と (2) を合わせて始め、増えた分を (2) に書き戻す。(1) の更新は人間が最小化してから PR で行う。**再現**: メンテナが artifact を取得して手元で復号し、手元 (Linux または WSL。libFuzzer は Windows で動かない、§3.3) で同じコミット・seed で実行して行う。**クラッシュの起票の規則**: 自動で起票する公開の課題には **ジョブの失敗とクラッシュの件数** (ターゲット名・コミット・seed・件数) だけを書き、入力・スタック・原因の推測を書かない。メンテナが手元で再現し、**Rust の `panic` (境界外の添字・`unwrap` の失敗など) でセキュリティ性が無いと判断したものだけ**、詳細 (最小化した入力のハッシュとサイズ・スタック・原因) を公開の課題に書く。panic 以外の異常終了 (セグメンテーション違反・スタックオーバーフロー・メモリの誤用の検出)、`unsafe` を含む crate の中で起きたもの、判断がつかないものは 11 §9.8 の非公開の手順で扱う。最小化した入力は回帰テスト (L0) に転記する。fuzz の入力は生成データ (利用者の文書を含まない、§4 のテストデータの規則) なので、最小化した入力をコミットすることは妨げない (12 §5.6 の PC-6)。
- `PROPTEST_CASES=10000`。
- `cargo audit` (RustSec)、`cargo deny check advisories`。**`fuzz/` も対象にする** (`fuzz/` は本体の workspace から除外した別の crate で、独自の `Cargo.lock` を持つので、`cargo audit` と `cargo deny check` (licenses / bans / advisories / sources) を `fuzz/` でも別に実行する。毎 PR の `deps` ジョブ (§6.2) でも同じ)。**例外の置き場所は `deny.toml` の `[advisories] ignore` の 1 か所** (勧告 ID ごとに理由と期限をコメントで書く)。`cargo audit` には同じ ID を `--ignore` で渡し (ワークフローの手順で `deny.toml` から読む)、`.cargo/audit.toml` は置かない。
- **依存の更新は Dependabot** (`.github/dependabot.yml`。cargo と GitHub Actions の 2 つのエコシステム。cargo の対象ディレクトリはリポジトリ直下と **`fuzz/`** の 2 つ。**週 1、グループ化** (PR 洪水を避ける)、**`target-branch: develop`** (11 §9.8 と同じ)。`develop` は作業ブランチ・`main`・Dependabot からの PR を受ける (11 §9.1 のブランチ保護、§6.2 の `pr-source`)。リリース候補の期間 (凍結から昇格まで、§9.2 手順 1〜4) は Dependabot の PR を含めて `develop` への merge を保留する。セキュリティの更新の PR の宛先は 11 §9.8)。設定ファイルは最初の push の準備 (08 §4 の M0e) で置く。Dependabot の PR も通常の PR と同じゲート (§8.1) を通し、squash で入れる (`dco` ジョブの扱いは §6.2)。
- (v0.2) `cargo update` 後の `cargo check` (最新依存で MSRV・ビルドが壊れないか早期検知。失敗しても PR はブロックしない)。
- (v0.2) `cargo +nightly udeps` (未使用依存)、`cargo outdated` の報告。
- `coverage`: Linux の夜間ジョブで `cargo llvm-cov nextest --lcov` を **Linux CI の対象 crate** (§6.4: core 系 + `eve-host` + `eve-testkit`。`eve-testkit` は測定の対象から除き、テストの実行にだけ使う) に対して実行し → artifact。`eve-platform` / `eve-tui` / `evoeditor` bin と L5 は v0.1 の測定の範囲に入らない (Windows の `llvm-cov` は遅い、§6.4 の落とし穴 (9))。§8.1 の `eve-platform` / `eve-tui` の目安を測る Windows の測定は、ゲートにする v0.4 で加える。**v0.1 は報告のみ** (OQ-101)、`--fail-under-lines` の crate 別閾値 (§8.1) は v0.4 からゲート (毎 PR で走らせるかは v0.4 で判断)。
- (v0.2) `docs-rs`: `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace` (NFR-705。`-D missing_docs` を加えるのは v0.3 から)。
- (v0.2) `unsafe-audit`: `eve-platform` / `eve-tui` の `cargo geiger` の数を記録 (情報。R5 の `forbid` の存在検査は毎 PR の `deps` ジョブ)。
- **クラッシュ一貫性 (NFR-301 の「電源断」、v0.2)**: プロセス kill や `MemFs` の障害注入では fsync の順序 (一時ファイルの fsync → rename → ディレクトリ fsync) の欠落を検出できない (kill テストでは必ず合格してしまう)。Linux の夜間ジョブで **LazyFS** (FUSE ベース、fsync されていない書込を落とす) または **dm-log-writes** (書込順序を記録・再生) を使い、保存手順の各切断点で「元ファイルまたは新ファイルが完全な状態で残る」ことを検査する (08 §3 S-10 の合否基準を自動化)。ext4 / xfs / btrfs をループバックで用意。道具の選定は OQ-143 (§12、未決)。
- BM-306 soak: **v0.2〜v0.4 は夜間に 1 時間** (傾向の観察)。v0.1 では行わない。8 時間の判定は **v1.0** のリリース前に行う (ホストランナーは最長 6 時間なので、セルフホストランナーか基準機で実行する)。
- 週次 (v1.L から、任意): `ai-local.yml` (実 `llama-server` との契約テスト、§3.9)。失敗は Issue を起票するだけで PR をブロックしない。v1.L からは soak に AI 有効 (模擬サーバ) の 1 時間を加え、要求の繰り返しで RSS・`AiSession` の履歴が増え続けないこと (MR8) を見る。

### 6.4 テストマトリクス (OS × アーキ)

**頻度** (01 NFR-401 AC-1、08 §2.2、確定 (ユーザー回答)): v0.1 は Windows だけを出荷するので、毎 PR は **Windows x86_64 (全 crate の全テスト + E2E)** と **Linux x86_64 (Linux CI の対象 crate のビルドとヘッドレスのテスト)** の 2 つ。Linux は core の移植性 (OS に依存しない crate が Windows の API に寄りかからないこと) を守るためだけに走らせ、配布物は作らない。macOS の CI・Linux aarch64・Tier B (Windows aarch64 等) のビルドは v0.2 (macOS / Linux を出荷する版。確定 (ユーザー回答))。fuzz は夜間のみ (§6.3)。Windows のランナー (`windows-2025`) は Windows Server 系で、v0.1 の対象 OS (Windows 11) の実機の確認は基準機の L6 / L7 で行う (§3.7)。**Linux CI の対象 crate** (定義の正本は [02 §4.7](02-architecture.md#47-v01-の対象-os-と移植性の維持)) = core 系 crate (`eve-text` / `eve-encoding` / `eve-view` / `eve-config` / `eve-core`) + `eve-host` + `eve-testkit` (`eve-platform` は Unix の実装を持つ版から、`eve-tui` と `evoeditor` bin は Linux を出荷する版から加える)。`eve-bench` は対象外 (Windows だけでビルドする)。`xtask` と `eve-docs-tables` は ubuntu のジョブ (`deps` の `xtask check-deps` 等) でも動くので **Windows と Linux の両方でビルドする** (OS に依存するコードを持たない、11 §1)。

| ターゲット | ランナー (執筆時点の名称。変更に備え `strategy.matrix` を 1 箇所に集約) | 実行内容 | 頻度 |
|---|---|---|---|
| `x86_64-unknown-linux-gnu` (v0.1) | `ubuntu-24.04` | Linux CI の対象 crate のビルドとヘッドレスのテスト (L0〜L3)。カバレッジと fuzz は夜間 (§6.3)。無通信の strace 検査 (§3.6) は v0.2 の NFR-501 AC-2 (v0.1 の NFR-501 AC-1 は Windows の依存の禁止リストと R3 のレビュー) | 毎 PR |
| `x86_64-unknown-linux-musl` / `aarch64-unknown-linux-musl` (Linux を出荷する版) | `ubuntu-24.04` / `ubuntu-24.04-arm` + `musl-tools` | 全テスト (静的バイナリで nextest。配布物と同じターゲット) | 毎 PR / main・release |
| `x86_64-unknown-linux-gnu` の配布物 (条件付き、OQ-112。Linux を出荷する版) | `ubuntu-24.04` (zigbuild 2.31) | ビルド + 全テスト + BM-104 / BM-202 の musl 比較 | main / 夜間 (配布物に加えた場合は release も) |
| `aarch64-unknown-linux-gnu` (条件付き、OQ-112。Linux を出荷する版) | `ubuntu-24.04-arm` (zigbuild 2.31) | ビルド + テスト | main / 夜間 |
| glibc 2.31 互換 (gnu 版を配布する場合のみ。Linux を出荷する版) | `ubuntu-24.04` + コンテナ `ubuntu:20.04` で **実行のみ** (zigbuild 生成物を持ち込み `evoeditor --version` + L5 スモーク) | NFR-401 の下限確認 (ビルド環境の glibc 固定、05 §16.2) | main / release |
| `x86_64-pc-windows-msvc` (v0.1) | `windows-2025` | 全 crate の全テスト + E2E (ConPTY、§3.6) + `build-release-smoke` | 毎 PR |
| `aarch64-pc-windows-msvc` (Tier B、v0.2 から) | `windows-11-arm` (公開リポジトリ向け ARM ランナー。不可なら x64 からクロスビルドのみ、OQ-102) | ビルドのみ (ランナーがあれば全テスト) | release |
| `aarch64-apple-darwin` (macOS を出荷する版) | `macos-15` (Apple Silicon) | 全テスト + E2E | 毎 PR |
| `x86_64-apple-darwin` (Tier B、macOS を出荷する版) | `macos-15-intel` (提供終了予定あり。終了後は arm64 ランナー上でクロスビルドのみ。OQ-15) | ビルドのみ | release |

- **Windows の落とし穴** (CI 設計で先回りする): (1) `.gitattributes` 無しで fixture が CRLF 化 (§4) (2) 並列テストの同一パス使用で `SHARING_VIOLATION` → 各テストは `tempfile::tempdir()` を使い、**ファイルハンドルを drop してから** rename/削除 (3) シンボリックリンク作成は開発者モード/特権が必要 → ランナーでは可能。**Windows ではシンボリックリンクのテスト (FR-106 AC-2) の `skip!` を失敗として扱う** (作成の権限が無い開発者の環境は開発者モードを有効にすることを前提にし、CONTRIBUTING.md に書く) (4) タイマ分解能 15.6 ms → 時間依存 assert 禁止 (Q3) (5) 大文字小文字非区別 FS 前提のテストは Linux では別の結果 → `cfg!(windows)` で期待値を分ける (6) パス長: ランナーの `D:\a\<repo>\<repo>` は短いが、ローカルでは長くなり得る → `tempdir` を使う (7) Defender のリアルタイム保護で I/O テストが 2〜3 倍遅い → タイムアウトは余裕を持つ (nextest `slow-timeout = 60s`) (8) libFuzzer 不可 (§3.3) (9) `cargo llvm-cov` は動くが遅い → Linux のみ (10) `windows-sys` の `unsafe` 呼び出しは `eve-platform` のみ (R5)。
- **macOS の落とし穴** (macOS を出荷する版): ランナーが高価 (分単位の係数 10×) → `clippy`/`test`/`e2e` 以外は走らせない。`F_FULLFSYNC` で保存テストが遅い (fsync Level を test 時は `FileOnly` に設定可能に、ただし NFR-301 のテストは Full)。APFS 大文字小文字非区別。xattr の複製テスト (v0.2、FR-106) は macOS 固有。
- **Linux の落とし穴** (v0.1 は Linux CI の対象 crate だけなので、主に Linux を出荷する版): inotify 上限 (v0.4 の監視テストは watch 数を小さく)、ランナーに `DISPLAY` がない → クリップボード系はヘッドレス経路のテストになる (X11 経路はローカル or `xvfb-run` で Could)。

### 6.5 セキュリティ (CI 自体)

- **権限の方針**: リポジトリの設定で `GITHUB_TOKEN` の既定を読み取りにし (12 §5.6 の PC-5)、各ワークフローの先頭に `permissions: contents: read` を書き、書込みが要るジョブだけにジョブ単位で次の権限を足す。表に無い権限は付けない。`id-token: write` は GitHub の artifact attestation を使う場合だけ付ける (minisign の署名は CI で行わない)。

| ワークフロー / ジョブ | 権限 (既定の `contents: read` に足すもの) | 理由と注意 |
|---|---|---|
| `ci.yml` (全ジョブ) | なし | — |
| `ci.yml` の依存追加のコメント (§7 R6 (1)) | `pull-requests: write` | fork からの PR では `GITHUB_TOKEN` が読み取りに限られ、この権限は付与されない。コメントに失敗してもジョブは失敗にせず、同じ内容をジョブの要約 (`$GITHUB_STEP_SUMMARY`) に出す。`pull_request_target` で権限を得ることはしない |
| `nightly.yml` の fuzz の起票 (§6.3) | `issues: write` | 自動で起票する課題に書くのはジョブの失敗とクラッシュの件数だけ (§6.3 の起票の規則)。クラッシュ入力の artifact は `age` で暗号化したものだけを上げる (artifact の upload に追加の権限は要らない) |
| `bench.yml` | v0.1: なし (`contents: read` のまま。記録は Actions の artifact、§6.1)。v0.2: `contents: write` | v0.2 の `gh-pages` へのグラフの push のため。手動 (v0.2 から `develop` への push も) でだけ動き、PR では動かない |
| `release.yml` | `contents: write` | Release の draft の作成と成果物の添付。タグ `v*` と手動 (dry-run) だけで動く |
| `docs.yml` | なし | — |
| `ai-local.yml` (v1.L) | `issues: write` | 失敗時の起票 |

- サードパーティ Action は **コミット SHA 固定**。Dependabot で更新。
- `pull_request_target` を使わない。fork からの PR では secrets を使うジョブ (署名) を走らせない。外部の寄稿者 (fork) からの PR のワークフローは、メンテナが承認するまで実行しない設定にする (12 §5.6 の PC-5)。v0.1 の間は CI にシークレットを置かない (リリースの署名は手元の minisign、コード署名は v0.4 から)。
- 秘密情報の検出 (未決、結論案、[OQ-169](10-decisions.md#28-oq-161-以降-仕上げ段階の追加起票)): `gitleaks` 相当のジョブを `ci.yml` に置き (M1 のタスク (0) から毎 PR)、GitHub の secret scanning と push protection を併用する。
- 依存の取得元は crates.io のみ (`cargo deny sources`)。git 依存禁止 (例外は ADR)。
- **秘密情報の置き場** (詳細は 11 §9.7): minisign の秘密鍵は **CI に置かない**。メンテナの手元 (オフライン) で `SHA256SUMS` に署名し、`SHA256SUMS.minisig` をリリースに添付する (minisign は OIDC に対応しない。05 §18.9)。コード署名 (Authenticode / Developer ID) の資格情報は **CI のシークレット (可能なら OIDC で短命トークンを取得) にのみ** 置き、リポジトリ (`.github/`、`.claude/`、`xtask`) に含めない。シークレットを使うジョブは `environment` の保護規則 (承認者・ブランチ制限) の下で動かす。セルフホストランナー (基準機) は公開リポジトリの PR のワークフローに付けず、`workflow_dispatch` とスケジュールのジョブだけを専用のユーザー (または VM) で動かす (fork の PR のコードを基準機で実行させない)。

---

## 7. 依存規則 R1〜R11 の機械検査

[02 §4.3](02-architecture.md#43-依存規則-ci-で機械的に検査する) の各規則を次で検査する。`cargo deny bans` はワークスペース全体への禁止しかできず **crate 単位の依存禁止** を表現できないため、`xtask check-deps` (ワークスペース内 `xtask` crate、`cargo metadata` を解析) を主手段とする。

| 規則 | 検査手段 | 判定 |
|---|---|---|
| R1 `eve-core`, `eve-text`, `eve-encoding`, `eve-view`, `eve-config` は ratatui / crossterm / tokio / reqwest に (推移的にも) 依存しない。`eve-host` も端末クレートに依存しない | `xtask check-deps`: `cargo metadata --format-version 1` の resolve グラフで各 crate から normal 依存の辺だけをたどって到達可能な全 package 名を列挙し (dev 依存は対象外、02 §4.1 の「禁止 crate 列」の注)、禁止リスト (リポジトリ直下の `deps-policy.toml` に記述: `ratatui`, `crossterm`, `tokio`, `reqwest`, `hyper`, `windows-sys`, `libc`(core 系のみ), …) との交差が空 | 交差があれば失敗 (crate 名とパスを表示) |
| R2 Service (`eve-syntax`, `eve-lsp`, `eve-ai`) 同士は相互依存しない。連携は core の trait/メッセージ経由 | 同上: `eve-syntax`/`eve-lsp`/`eve-ai` 相互の到達不能 | 失敗 |
| R3 `eve-core` は `std::fs` / `std::process` / `std::net` を直接使わない (Port 経由)。`std::thread` も `Instant::now` も使わない (JobSpawner / Clock 経由) | **clippy `disallowed_types` / `disallowed_methods`** を `eve-core/clippy.toml` に設定 (`std::fs::File`, `std::fs::read*`, `std::process::Command`, `std::net::*`, `std::thread::spawn`, `std::time::Instant::now`, `std::time::SystemTime::now`)。補助として `xtask check-deps` が `eve-core/src` を字句走査 (`use std::fs`、`std::fs::` 出現) | clippy `-D warnings` で失敗 |
| R4 `eve-view` の外部依存ゼロ (serde のみ optional) | `xtask check-deps`: `eve-view` の normal 依存集合 ⊆ {`serde`} かつ serde は optional | 失敗 |
| R5 `unsafe` は `eve-platform` / `eve-tui` の FFI 部分のみに限定 (`#![forbid(unsafe_code)]` を他 crate に付与) | 他 crate の `lib.rs` 先頭に `#![forbid(unsafe_code)]` が存在することを `xtask` が検査。`eve-platform`/`eve-tui` では `#![deny(unsafe_op_in_unsafe_fn)]` + `cargo geiger` の数を記録し、増加時はレビューで理由確認 (NFR-704) | 失敗 / 情報 |
| R6 新規外部依存は ADR または依存追加チェックリスト (02 §13.0) を通す。推移的依存の総数と C を含む依存の有無を CI で監視する (NFR-207) | (1) `Cargo.lock` の差分で新規 package を検出 → PR テンプレートの「依存追加チェックリスト」が埋まっていなければ bot コメントで要求 (2) `cargo deny check licenses` (ADR-0018 の許可リスト。05 §17.1 の注意点を反映。**公開リポジトリでも GPL / AGPL / LGPL を禁止** し、公開 crate を外部の消費者が静的リンクできる状態を保つ、§7.1) (3) `cargo deny check sources` (crates.io のみ) (4) `cargo deny check bans` で `multiple-versions = "warn"` と `wildcards = "deny"` | (2)(3)(4) は失敗、(1) はレビュー必須ラベル。依存数は BM-B02 (C を含む依存 0 は失敗、総数は報告) |
| R7 `eve-ai` は `std::fs` (自身の監査ログ書込を除く) / `std::process` を使わない。ファイル内容は core から受け取る (v1.L) | R3 と同じ clippy `disallowed_*` を `eve-ai/clippy.toml` に | v1.L から失敗 |
| R8 公開 API の増加を検知する: `cargo public-api` の差分を CI に出し、`eve-view` / `eve-core` の `pub` の増加はレビュー項目にする (v0.4 から。それまでは `pub(crate)` を既定とする規約のみ) | v0.4 から `cargo public-api` の差分を PR に報告し、`pub` の増加をレビュー (§8.4)。v0.1〜v0.3 は `pub(crate)` 既定の規約をレビューで確認 | 報告 + レビュー |
| R9 frontend crate の PR は `eve-core` / `eve-view` / `eve-config` のファイルを変更しない。必要なら型の追加を PR の説明で先に合意する。CI は変更ファイル集合と PR ラベル (`area:tui` 等) の整合を検査する | §6.2 `area-check` ジョブ (v0.2): frontend のラベルの PR の変更ファイル集合に上の 3 crate があれば失敗。v0.1 はレビュー (§8.4) で確かめる | 失敗 (レビュー必須) |
| R10 (案) `eve-encoding` の normal 依存は `encoding_rs` と `chardetng` のみ | `xtask check-deps`: `eve-encoding` の直接の normal 依存集合 ⊆ {`encoding_rs`, `chardetng`} (dev 依存は対象外) | 失敗 |
| R11 (案) `std::thread::spawn` を使ってよいのは `eve-host` のみ (例外は 02 §9.1 の表のスレッド)。作業スレッドは `JobSpawner` の固定 2 本 | clippy `disallowed_methods` に `std::thread::spawn` / `std::thread::Builder::spawn` を全 crate で設定し、`eve-host` と許可リスト (`deps-policy.toml` に crate とファイルを列挙: `eve-tui` の入力スレッド、`eve-platform` のクリップボードの所有者ウィンドウのスレッド (Windows)・緊急の端末復元 (`emergency_restore`) の使い捨てスレッド・セッション終了通知・ファイル監視。02 §4.3 R11 / §9.1 の表と同じ集合) だけ `#[allow]` を認める。`xtask check-deps` が許可リスト外の `#[allow(clippy::disallowed_methods)]` を字句走査で検出 | 失敗 |
| R12 `eve-core` の `src/` の中の単体テスト (`#[cfg(test)]`) は `eve-testkit` を使わない。使うテストは統合テスト (`crates/eve-core/tests/`) に置く (02 §4.3 R12、§3.4) | **`deps-policy.toml` は使わない** (依存の向きの規則ではなく、型が二重になるビルドエラーを防ぐ置き場所の規則のため)。`xtask check-deps` が `crates/eve-core/src/` の中の `eve_testkit` の参照を字句走査で探し、1 件でもあれば失敗 | 失敗 |

推移的依存の **総数 (≤ 90) と C 依存 0** (NFR-207) も `xtask check-deps` が数える (BM-B02。C 依存 0 は失敗、総数は v0.1 では報告)。`deps-policy.toml` の禁止リストは 02 §4.1 の表の「禁止 crate」列 (完全な crate 名をバッククォートで列挙した列。「禁止事項」列は説明の文で突合しない) と 1:1 に対応させ、表を更新する PR では同時に更新する。対応は `xtask check-deps --docs` が 02 §4.1 の表 (11 §8.5 の規則で読む) と突合して検査する (`docs-tables` ジョブ、§6.2)。`deps-policy.toml` は各 crate の **許可する workspace 内の依存** (02 §4.2 の依存グラフの辺) も持ち、`xtask check-deps` は workspace 内の normal 依存がその部分集合であることを検査する (例: `eve-platform` は `eve-core` と `eve-view`。`eve-view` は緊急の端末復元に渡す平文の型 (`RawTermHandle` / `SavedTermMode` / `TermRestoredFlag`) のため、02 §4.1)。

### 7.1 外部の消費者向けの公開 API 互換とライセンス検査

公開 crate を版指定で消費する **外部の消費者** (駆動 API のクライアント等) のために、公開リポジトリの CI で次を検査する (運用は 11 §9.5)。

| 観点 | 公開リポジトリの CI |
|---|---|
| 公開 API の互換 | `cargo semver-checks` を 12 §4.1 の範囲 (`eve-host` / `eve-view` / `eve-config` / `eve-core` の `Editor` ファサード / `eve-testkit`) に適用 (**v0.1 リリースから報告、v0.4 からゲート**、OQ-157。本書で開始時期を書くのはここだけ。`0.y` の間は `y` 上げで破壊的変更を許すが、破壊的変更は **CHANGELOG の「外部の消費者向け移行手順」** を必須にし、契約版は直近 2 版の並行期間を置く。12 §4.2、OQ-157)。範囲外の公開 API は「安定性の保証なし」と crate 文書に明記 (`eve-core` の内部モジュールは `pub(crate)` か `#[doc(hidden)]`)。`CONTRACT_VERSION` の増分が変更と同じ PR にある |
| 契約の不足 | 外部の消費者から来た **契約変更要求** (Issue) を 02 §8.4 の手続きで処理し、TUI とヘッドレス参照レンダラを同じ PR で更新して公開 crate の新版として出す。1 つの消費者の都合で公開 API に破壊的変更を入れない |
| ライセンス検査 | `cargo deny` (許可リストの正本は 05 §17.1 の表。GPL / AGPL / LGPL 禁止 = 12 §3.6 の「公開リポジトリ」の層) |
| 署名・配布物 | minisign、TUI のアーカイブ (§9) |
| 秘密情報 | 無し (minisign の署名はメンテナの手元。§6.5) |
| 共通の規則 | 11 の運用 (タスク単位、`spec:` / `verifies:`、シナリオ DSL、`insta accept` は人間) をそのまま適用。`xtask` の処理本体は公開 crate から使えるようにする |

v0.1 の公開側で用意するのは `eve-view` の型の規則 (`#[non_exhaustive]`・`CONTRACT_VERSION` の定数) と `HeadlessFrontend` の ASCII Frame のテストで、契約版 1 の固定 (02 §8.4 C3) は v0.1 のゲートにしない (ADR-0039)。`cargo semver-checks` は上の表のとおり v0.1 リリースから報告する。

---

## 8. 品質ゲート

### 8.1 PR マージゲート (全部必須)

| ゲート | 条件 |
|---|---|
| CI 緑 | `ci.yml` の全ジョブ成功 (L5 は retries 込み) |
| レビュー | 1 名以上の承認。**単独開発期間** はブランチ保護の「承認必須」を 0 にし (自分の PR は承認できないため)、代わりにセルフレビューのチェックリスト (§8.4) を PR に貼る。管理者のバイパスは許さず、必須のステータスチェックだけで merge を止める (11 §9.1) |
| lint | clippy 警告 0 (`-D warnings`。`[workspace.lints]` に明示)、rustfmt。pedantic の採用リントと `missing_docs` (公開 crate) は **v0.3 から** |
| カバレッジ | **v0.1 は報告のみ** (夜間の artifact、§6.3)。v0.4 からゲート: crate 別の下限 `eve-text` 90% / `eve-encoding` 90% / `eve-core` 80% (`undo` / `buffer` モジュールは 90%、`llvm-cov` の `--ignore-filename-regex` でモジュール別に測る) (ここまでが 01 NFR-702 の値)。本書で加える目安: `eve-config` 80% / `eve-platform` 60% (OS 分岐のため) / `eve-tui` 60%。PR で前回比 −0.5 ポイント超の低下は要説明 |
| 依存 | §7 (各規則の開始版に従う。C を含む依存 0 は失敗、推移的依存の総数は報告) |
| 寄稿の表明 | 全コミットに `Signed-off-by:` (DCO、OQ-154。§6.2 `dco` ジョブ。昇格の PR と bot の PR は対象外で、squash のメッセージに `Signed-off-by:` を残す、11 §9.4) |
| トレーサビリティ | 新規・変更した Must 要件 (AC) に `verifies:` (AC ID) を持つテストがあり、実装に `spec:` (節 ID) がある (§8.3)。**v0.1 は PR テンプレートのチェックリスト** (11 §4) と §8.5 の表で確認、v0.2 から規範行 ID と `xtask trace` の警告が増えないこと |
| 契約 | `eve-view` を変更する PR は、PR の説明に追加する型・フィールドと代替案を書き、全消費者 (TUI・`HeadlessFrontend`・シナリオ) を同じ PR で更新する。frontend の PR は `eve-core` / `eve-view` / `eve-config` を変更しない (§7 R9。v0.1 はレビューで確認) |
| シナリオ | ユーザー可視の挙動を変える PR はシナリオのテスト (v0.1 は Rust の Harness で各 crate の `tests/` に。v0.2 から TOML の DSL を `tests/scenarios/` に) を追加または更新している (Q8) |
| 変更記録 | `CHANGELOG.md` の Unreleased に 1 行 (ユーザー可視の変更時)。または `changelog: none` ラベル |
| 仕様整合 | 仕様 (docs/) と異なる実装をする場合: **人間の PR** は同じ PR で docs を更新し ADR / OQ を参照する (00 §7.1)。**エージェントの PR** は docs の本編 (`docs/*.md`) を直さず、実装を止めて OQ 起票案を PR に添える (11 §4.5)。生成物 (`docs/spec-index.toml`、`docs/reference/*`) は `xtask` で再生成したものを、人間・エージェントとも PR に含める (11 §4.4) |

### 8.2 リリースゲート (版の完了・各リリース)

| ゲート | 条件 | 記録 |
|---|---|---|
| 要件充足 | 当該版の **Must FR/NFR がすべて T/B/M/R のいずれかで合格**。Should の未達は 08-roadmap に繰り越し記録。**v0.1 ゲートの正本は 01 §4.2 / §5.1〜5.4 と 08 §2.2** (本書はその検査手段を定める) | トレーサビリティ行列 (§8.3) |
| 性能 | BM-1xx/2xx の Must (§5.2 の「v0.1 で判定に使うベンチ」) が基準環境 (Windows の開発 PC、§5.3) で目標内 (p50/p99)。許容上限超えは不可。iai の CI は v0.2 | `docs/perf/<version>/` |
| 互換 | L6 マトリクスの **Windows Terminal と conhost** で手動シナリオ「開く・日本語入力 (Microsoft IME・Google 日本語入力)・検索・保存・終了」が通る (08 §2.2 のリリースゲート (5))。リリースノートと README に対象 OS (Windows 11。Windows 10 は未対応) と既知の制約 (01 §4.6: Windows のみ・単一文書・長大行は読み取り専用・ヘルプ画面なし等) を載せる | `docs/compat/<version>/` |
| 受入 | L7 の Must 項目合格。**NFR-601 (初見ユーザビリティ)** は v0.2 で 3 名の所見、**v1.0 ゲートで 5 名** (OQ-106。1.0 の判定条件は OQ-44)。NFR-306 (8 時間ソーク) も v1.0 (v0.2〜v0.4 は夜間 1 時間の観察のみ) | `docs/compat/<version>/acceptance.md` |
| データ保全 | Windows の kill テスト (§3.6 (6)。in-place の経路の切断点、バックアップなしの承認の後の中断、新規保存の中断、ダングリングのリンク経由で開いたときの検出、`ReplaceFileW` の失敗の注入 → 確認 → in-place、乾式走査で表現不能なら元ファイルが不変、を含む)、往復コーパス (§4)、表現不能・黙示置換の検出 (§3.2)、区分 B の検出、緊急退避 (パニック注入・CTRL_CLOSE・外部から送った CTRL_C / CTRL_BREAK・メインがブロックした状態の CTRL_CLOSE と CTRL_C・パニックの経路の 100 MB の退避 (AC-5、§3.6 (8a))・シグナルの経路 (CTRL_C / CTRL_BREAK と CTRL_CLOSE) の退避の段の打切り (`complete = false`、§3.6 (8b))、§3.6 (7)(8)。CTRL_CLOSE が CI で環境起因の失敗になった項目は基準機での再判定 (§3.6) の合格)、L5 の時間の判定 (NFR-302 AC-3 / AC-5 の時間予算。基準機 ENV-W1 の手動実行、§3.6 の「時間の判定」)、ダウングレードの退避 (未知の `format_version` の退避を読まず・変更・移動・削除しない、01 NFR-302 AC-6、§3.4) が合格 (08 §2.2 のリリースゲート (3)) | CI (+ `docs/compat/<version>/acceptance.md`) |
| セキュリティ (依存) | `cargo audit`/`deny advisories` クリーン (例外は `deny.toml` の `[advisories] ignore` だけに期限と理由を付けて書く、§6.3) | CI |
| セキュリティ (リリース前の確認、08 §2.2 のゲート (8)、01 NFR-502 AC-2) | (a) **DLL の検索順**: 配布する `evoeditor.exe` が `/DEPENDENTLOADFLAG:0x800` (`LOAD_LIBRARY_SEARCH_SYSTEM32`) でリンクされていることを `dumpbin /loadconfig` の `DependentLoadFlags` で確かめ、起動の最初に `SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32)` を呼ぶコードがあることをレビューで確かめる。あわせて、実行ファイルと同じフォルダとカレントディレクトリに、evoeditor が読み込むシステムの DLL と同じ名前の偽の DLL (読み込まれると印のファイルを作るもの) を置いて起動し、印のファイルが作られないことを確かめる。(b) **UNC パス**: `\\server\share\…` (到達できない名前を含む) を開いたときの挙動 (OS が認証情報を送り得ること、応答待ちで UI が止まらないこと) を確かめ、既知の制約 (01 §4.6) に書くかを決める。結果と判断を記録する (判定は「確かめて記録した」こと) | リリースのチェックリスト |
| fuzz | **リリース候補のコミット** (ゲートを判定する版上げ後の `develop` の先頭。タグを付けるコミットと同じソース、§9.2) で `fuzz_decode` を **1 時間** (`nightly.yml` の手動実行、§6.1) 回してクラッシュ 0 (v0.1 は `fuzz_decode` の 1 ターゲット、§3.3。01 NFR-305 の v0.1 ゲート)。前提として、夜間 fuzz (10 分) で見つかったクラッシュの課題 (非公開で扱うものを含む、§6.3) がすべて解決済みで、**`develop` を対象にした** 直近 5 日 (v0.5.0 以降の正式版は 7 日) の夜間 fuzz が毎日実行されクラッシュ 0 であること (夜間の対象は `develop`、§6.1。パッチ版も同じ条件。**プレリリース (`-rc.N`) の判定は直近 2 日**、PO 判断 2026-10-24。正式版の判定は、v0.5.0 の前までは 5 日、v0.5.0 以降は 7 日。PO 判断 2026-10-26)。5 日 (v0.5.0 以降は 7 日。rc は 2 日) の観測は 1 時間の実行の代わりにならない | CI + リリースのチェックリスト |
| 契約 | v0.1 は境界型を C2 (追加のみ) で維持し、`eve-view` の型が `#[non_exhaustive]` で `CONTRACT_VERSION` の定数を持つ (駆動 API を実装した版からは `initialize` の値も)。契約版 1 の固定 (02 §8.4 C3) は v0.1 のゲートにしない (ADR-0039) | CI |
| 依存 | C を含む依存 0 (`xtask check-deps`)。推移的依存の総数 (≤ 90、NFR-207 は Should) は **報告** でゲートにしない (ADR-0039)。v0.4 で値を見直す | CI |
| MSRV | `rust-version` でビルド可 (v0.1 は手で確かめる。CI の `msrv` ジョブは v0.2)。確かめ方は下の「配布経路の検証」の `cargo install --tag` の経路と同じ (MSRV のツールチェーンでクリーンな環境からビルド) なので、**タグの後** に行う | Release のノート (タグの後) |
| 配布物 | **Windows x86_64 のアーカイブ (zip)** と `SHA256SUMS` が生成され、Windows のランナーで **ダウンロード → 展開 → `evoeditor --version` → L5 スモーク** が自動で通る (§9.2)。**zip の内容物の検査** (`release.yml` で自動): `evoeditor.exe`・`LICENSE`・`NOTICE`・`THIRD-PARTY-NOTICES.md`・`README` (利用者向け)・`config.example.toml` がそろい、それ以外のファイル (デバッグ情報・テスト用の補助プロセス等) が入っていない。**テスト専用の経路が無いこと**: 配布ビルドに `test-hooks` と `--bench-*` が含まれないことを `build-release-smoke` (§6.2) と同じ検査で配布物の exe に対して行う。macOS / Linux のアーカイブは、それらを出荷する版から (05 §18)。`release.yml` はタグの後に走るので、結果 (実行の URL を含む) は Release のノートの「検証」の節に書く | Release のノート (タグの後) |
| 配布経路の検証 (手動、v0.1 から) | (1) **`cargo install` の経路**: クリーンな環境 (Rust を新しく入れた Windows。CI のランナーでもよい) で MSRV のツールチェーンを使い `cargo install --locked --git <repo> --tag vX.Y.Z evoeditor` が通り、`evoeditor --version` がタグと一致する。(2) **クリーンな Windows 11 での手動インストール** (05 §18.2a の手順どおり。再頒布パッケージ (Visual C++ ランタイム) を入れていない環境で、`+crt-static` で CRT を静的にリンクした exe が起動することも確かめる): draft の成果物をブラウザでダウンロード → `SHA256SUMS` との照合 → `Unblock-File` → 展開 → `Path` への追加 → 新しい端末で起動。SmartScreen の表示の有無と手順どおりに進めることを確かめる (minisign の検証は署名 (§9.2 手順 7) の後なので、ここでは行わず手順 10 の公開後の検証で行う)。(3) **Smart App Control**: 有効 (または評価モード) の環境での挙動 (zip の exe と `cargo install` でビルドした exe のそれぞれ) を確かめ、結果を README とリリースノートの既知の制約 (01 §4.6) に書く (05 §18.2a の「要確認」を解消する)。どれも draft の成果物とタグを使うので **タグの後** に行う | Release のノート (タグの後) |
| ライセンス・名称 (v0.1 から) | 12 §3.8 の **LR-1** (Apache-2.0 の特許条項と `NOTICE` の運用の確認) と **LR-4** (名称の商標の簡易照会。結果は 10 の OQ-01 に記録) が済んでいる | リリースのチェックリスト |
| タグの検証 (v0.1 から) | `release.yml` の最初のステップ (§6.1) が通った: (1) `git verify-tag` でタグの署名が `.github/allowed_signers` に載る鍵のもの、(2) 正式版のタグが `main` のマージコミットを指す (プレリリースのタグは `develop` の先頭)、(3) タグの版と workspace の `Cargo.toml` の版が一致する (プレリリースは `-rc.N` 等まで一致、§9.2)、(4) タグのコミットの内容が、ゲートを判定した版上げ後の `develop` の先頭と一致する (差分が許可リスト `docs/compat/<version>/`・`docs/perf/<version>/`・`tests/ac-checklist.toml` だけ) (11 §9.3)。結果は Release のノートの「検証」の節に書き、リリースのチェックリストには載せない | Release のノート (タグの後) |
| 再現ビルド (Should, v0.4) | 同一コミットの 2 回ビルドでハッシュ一致 (05 §18.7)。`release.yml` で確かめ、結果は Release のノートの「検証」の節に書く | Release のノート (タグの後) |
| ドキュメント | README / ユーザーガイド / CHANGELOG / 生成リファレンス (§11.2) が最新。v0.1 はヘルプ画面が無いので、端末の設定・IME とチョードの注意・既知の制約を README に書く。v0.1 の利用者文書は日本語が正 (§11.3) | `docs.yml` |
| 更新手順の文書 (v0.1 から。ADR-0040) | (1) リリースノートに「更新の方法」(配布経路ごとの手順・その版の移行手順) がある。(2) 利用者向け README の「更新」節 (05 §18.10〜§18.12) がその版の配布経路と一致し、**更新とダウングレード** (設定の未知キーは警告して無視、読めない退避は移動も削除もしない) と **直前の版に戻す手順** (`SHA256SUMS` / minisign で検証してから戻す) が書かれている。(3) 記載どおりの挙動であることを、未知の `format_version` の退避を移動・削除しない検査 (§3.4 のダウングレードの退避) と設定の未知キーの検査 (01 FR-703 AC-1) の合格で確かめる。(4) `SECURITY.md` に更新方法の案内がある (`SECURITY.md` の最小の版と Issues・private vulnerability reporting は最初の push の前に用意済み、12 §5.6 の PC-7、OQ-164。脆弱性の対応の手順は 11 §9.8)。パニック時の文言が案内する Issue の URL (04 §14.4) がその窓口を指す。(1)〜(4) の文書の内容はタグの前に `develop` で整え、**確認 (draft のリリースノートと照らす) はタグの後** (§9.2 手順 9) に行う | Release のノート (タグの後、§9.2 手順 9) |
| **AI 不変条件 (v1.L 以降の Must ゲート、06 §16)** | 06 §16 の性質テストが合格: 除外パターン該当ファイルの内容バイト列が送信ボディに一度も現れない (AI-INV-4)、`ai.enabled = false` で `AiService` が呼ばれず tokio 未起動・ソケット未作成 (AI-INV-1)、AI 無効ビルドの依存グラフに tokio / reqwest が無い (AI-INV-8、§7 R1 と同じ検査)、インジェクション回帰コーパスで承認なしの副作用ゼロ (AI-INV-5)。**ローカル LLM**: 既定設定で非ループバックへの接続が拒否される (NFR-509)、ネットワークを切った環境で模擬サーバだけで指示編集・補完・チャットのシナリオが通る (NFR-510)、BM-115 (NFR-115) が基準環境 3 OS で合格、feature `ai` のみのビルド (TLS なし) で TLS 系 crate が依存グラフに無い (02 §4.1 `ai-remote`)、管理モード (v1.L-b) で子プロセスが 3 OS で残らない (FR-918 AC) | CI (v1.L から) |
| **未決事項の解消** | 当該版の Must に影響する OQ (v0.1 は 10 の「実装着手前 (G1a / G1b) に必須」と期限が v0.1 リリース前のもの) がすべて回答済み。実装着手ゲートの正本は 08 §5 | 08 §5 |

**リリースのチェックリスト**: 上の表が版のリリースの判定項目の **正本** で、08 §2.2 のリリースゲートの各項目もこの表のどれかの行で判定する。記録は **タグの前と後で置き場所を分ける**。(1) **タグの前** (§9.2 手順 3): `develop` に `docs/compat/<version>/release-checklist.md` をコミットする。**このチェックリストに載せるのは、タグの前に済む行だけ** (「記録」の列が「Release のノート (タグの後)」の行は載せない)。各行を「ゲート・判定・根拠 (CI の実行の URL・記録のファイル)・確認者・日付」の列で埋め、**ゲートを判定したコミット** (版上げ後の `develop` の先頭、§9.2 手順 2) の SHA を先頭に書く (`release.yml` の内容の一致の検査に使う、§6.1)。「記録」の列が「リリースのチェックリスト」の行 (リリース前のセキュリティの確認・fuzz・L5 の時間の判定・ライセンスと名称) は、このファイルの行そのものが記録になる (セキュリティの確認の結果はここに書き、詳細に基準機の固有情報を含めない、11 §9.7)。このファイルは `develop` → `main` の昇格で `main` に入る。(2) **タグの後** (§9.2 手順 6〜10): 「記録」の列が「Release のノート (タグの後)」の行 (配布物・タグの検証・MSRV・配布経路の検証・更新手順の文書の確認、v0.4 からは再現ビルド) と、`release.yml` の検証の結果・実行の URL・公開後の検証 (minisign の検証を含む) の結果は **GitHub Release のノート** の「検証」の節に書き、リポジトリには戻さない (タグを付けた後に `main` へ記録のコミットを足さない)。draft の Release を publish する前に、(1) のファイルがすべて埋まり、(2) のうち publish 前に行う項目がノートに書かれていること (§9.2 手順 9)。

### 8.3 トレーサビリティ行列

- **`spec:` と `verifies:`** (NFR-708、ADR-0033): **v0.1** は実装の関数・モジュール・型に `/// spec: 03 §9.3` のように **節 ID** を、テストに `// verifies: AC-FR-105-1` のように **AC ID** を書く。規範行 ID (仕様の規範的な表の各行・箇条書きの安定 ID。既存の K1 / INV-F1 / MR1 / AI-INV-n 等と、無い箇所に振る `<文書>-<節>-<連番>`。付け方の正本は 11 §3) と、`xtask spec-index` による ID → `file:line` の索引 (`docs/spec-index.toml`) の生成と CI での最新性の検査 (§6.2) は **v0.2** から。
- 収集: `xtask trace` がソース中の `spec:` / `verifies:` 行、シナリオ DSL の `[meta] verifies = [...]`、ベンチの `BM-xxx` ↔ NFR 表、手動チェックリストの `verifies:` を集め、01 の要件表 (ID・優先度・版・検証区分) と spec-index と突合して `docs/traceability.md` を生成する。**v0.1 は PR テンプレートのチェックリストと §8.5 の表から起こした `tests/ac-checklist.toml` (AC ID のチェックリスト。人が更新し、`xtask check` が軽い検査をする、11 §3.3) で代用し (記法は v0.1 から書く)、`xtask trace` は v0.2 から** (NFR-708)。
- **受入基準 (AC) 単位の突合**: 01 §5 の Must 要件には判定可能な受入基準 `AC-<要件ID>-<n>` がある。`verifies:` は要件 ID だけでなく **AC ID** (`verifies: AC-FR-105-1`) または規範行 ID (v0.2) を書けるものとし、`xtask trace` は「テストが存在すること」ではなく **現在の版の Must の全 AC に対応するテスト / ベンチ / 手順が存在すること** と、**現在の版の Must に紐付く規範行 ID で `spec:` も `verifies:` も無いもの** を検査する (後者は警告)。AC を持たない Must 要件は 01 への指摘として警告する。リリースゲート (§8.2) は「該当版の Must の AC がすべて合格」。
- 判定: 現在の版の Must で検証区分 T/B のものに対応する自動テスト/ベンチが **0 件** なら CI で警告 (版の終盤に失敗へ切替: 08 の完了条件に連動)。M/R は手順書の項目存在を確認。v0.4 以降の機能に依存する AC (例: FR-117 の「比較」は差分ビュー v1.L) は 01 側で v0.1 の文言から除いてある。
- 逆方向: 存在しない要件 ID・AC ID・規範行 ID を参照する `verifies:` / `spec:` は失敗 (ID の打ち間違い検出。v0.2 から spec-index で判定。v0.1 はレビュー)。廃止 ID (00 §7.1) は警告。
- **M10 の実装 (2026-10-31、最初の範囲)**: `cargo xtask trace` はソースのコメントの `verifies:` (直後 5 行以内の `fn` を試験名にする) と `spec:` の規範行 ID を集め、(1) `verifies:` の AC ID (v0.1 は §5.1〜§5.4、v0.2 は §5.5)・要件 ID・規範行 ID と、`spec:` の規範行 ID が存在しなければ失敗する。(2) 01 §5.5 の v0.2 の行を「要件」と「番号付きの AC」の単位に分け (`FR-101 AC-1:` のように要件を前に置いた AC はその要件の AC、番号の無い行は要件だけの単位)、試験の `verifies:` が AC ID か要件 ID を指していれば確かめたと数えて `docs/traceability.md` を生成する (生成物。`--check` で最新性も検査)。優先度が Must の単位で試験が無いものは警告にする (失敗にしない)。`xtask check` の (5d) と xtask の試験で走る。**まだ集めないもの**: シナリオ DSL の `verifies` (シナリオを入れる PR で足す)、ベンチ表の BM ↔ NFR、手動チェックリスト。v0.1 の AC は従来どおり `tests/ac-checklist.toml` が受ける。

### 8.4 レビューチェックリスト (R)

| 区分 | 項目 |
|---|---|
| 境界 | core に端末/OS 概念が入っていないか 。Port 以外で OS に触れていないか |
| 変更経路 | 文書変更が Transaction 経由か (`TextStorage` の可変 API を crate 外で呼んでいない) |
| エラー | `unwrap`/`expect` が理由付きか。外部入力でパニックしないか。メッセージが「何が+どうする」か (NFR-602) |
| 秘匿 | ログ・パニックメッセージ・クラッシュログに **文書本文・パス全体・API キーを format! していない** (NFR-504)。`Debug` 導出で本文を含む型 (Document/Snapshot) は `Debug` を手書きして本文を省く |
| unsafe | R5 範囲内か、`// SAFETY:` コメントと不変条件があるか (NFR-704) |
| 依存 | 依存追加チェックリスト ([02 §13.0](02-architecture.md#130-依存追加の評価基準) + 05 §17.2) |
| 性能 | UI スレッドで I/O・全文走査をしていないか (P3 原則)。ジョブはキャンセルを確認しているか |
| 安定性 (02 §14.1) | データ量に比例する再帰を書いていないか (MR7)。利用者がサイズを決める確保は `try_reserve` か (MR6)。メインスレッドで大きな `String` を作っていないか (MR5)。ワーカーと `Mutex` を共有していないか、退避経路でブロックするロックを取っていないか (MR9)。受信口へ送る側で集約しているか (MR3)。長時間稼働で増える構造に上限があるか (MR8)。緊急退避のスロット表が容量固定になっていないか (MR12) |
| 権限 (FR-601 AC-2) | **権限判定フックを通らないコマンドの実行経路が無い** か: コマンドの実行はすべて `execute` (権限判定フック、02 §6) を通り、キーマップ・パレット・確認ダイアログ・保存の連鎖・内部の再投入 (E3 の選択肢など) が `CommandSpec` のハンドラを直接呼んでいないか。新しいコマンドに `CommandSpec.permission` があり、その値が 07 §11.6 のカタログの `perm` の列と一致するか |
| 仕様 ID | 実装に `spec:` (v0.1 は節 ID) があるか、テストに `verifies:` (AC ID) があるか (§8.3)。仕様と異なる判断をした場合、docs を直接直さず OQ 起票案を PR に添えたか (エージェント。11 §4) |
| AI (v1.L、06) | メインスレッドの AI 関連処理が 1 メッセージ 1 ms 以内か (NFR-115)。プロンプトの接頭辞に可変要素 (カーソル位置・時刻・乱数・選択長) を入れていないか (BM-116 の再利用率を落とす)。送信先のホスト・ポートが UI に出るか (NFR-509)。モデル名・エンドポイントをコードや既定値に埋め込んでいないか (06 §4.4)。実サーバを前提にしたテストを CI の既定に入れていないか (§3.9) |
| 契約 | `eve-view` の変更は PR の説明で合意しているか。frontend の PR が `eve-core` / `eve-view` / `eve-config` を変更していないか (R9。v0.1 は CI の `area-check` が無いのでここで確認)。`match` に既定描画 (`_ =>`) があるか。全消費者 (TUI・`HeadlessFrontend`・シナリオ。駆動 API を実装した版からはそれも) の更新を同じ PR に含めたか (C1)。公開 API の破壊的変更に外部の消費者 (駆動 API のクライアント等) 向けの移行手順があるか (§7.1)。core / eve-view / eve-config に特定の frontend 向けの分岐を入れていないか |
| テスト | 新しい分岐に L0/L1/L3 があるか。`sleep` を使っていないか。回帰ファイルを消していないか |
| 仕様 | 仕様と異なる場合に docs を更新したか。CommandId を変えた場合 alias を残したか (02 §6.2) |

### 8.5 v0.1 Must の検証手段の対応 (チェックリストの初版)

v0.1 の Must (01 §4.2 の 87 件。v0.1 の範囲は案 L、確定 (ユーザー回答)) と 01 §5.1〜5.4 の AC に対する本書の検証手段。v0.1 のトレーサビリティのチェックリスト (§8.3、08 M0c。`tests/ac-checklist.toml`) はこの表から起こす。区分は 01 の検証列。

| 要件 | 区分 | 検証手段 (本書の節) |
|---|---|---|
| FR-101, 104, 107, 108, 111, 113〜116, 124, 127 | T | L3 シナリオ (§3.4)、往復コーパス (§4)。FR-101 AC-1 (FILE が 2 つ以上なら TUI を始めずに終了コード 2) は CLI の統合テスト (§3.4)。駆動 API の予約名 (`--headless` / `--rpc` / `--fake-ports` / `--allow-write`) を配布ビルド (feature `headless` なし) に渡すと、TUI を始めずに「このビルドでは未対応です (v0.2 以降の開発版機能)」を stderr に出して終了コード 2 で終わり、ファイル名として開かないこと (`evoeditor -- --headless` はファイル名として開くこと) も同じ CLI の統合テストで確かめる (07 §18.3、OQ-120)。`eve-view` の公開 cargo feature が `serde` の 1 つだけで既定が無効 (`default = []`) であることは `xtask check-deps` で検査する (02 §4.3 R4)。FR-107 は確認ダイアログの 3 経路 (`Tab` + `Enter` / 矢印 + `Enter` / アクセラレータ) と L5 (§3.6 (2))、AC-2 (コンソールクローズ等の外部終了では確認を出さずに緊急退避する) は L5 の CTRL_CLOSE のシナリオ (§3.6 (8b) (ii)。確認ダイアログが出ずに退避が書かれること)、AC-3 (開く・新規での置き換えの確認) は L3。FR-111 はパレットからの引数のリスト (§3.4) と AC-4 (開き直しは Undo できない)。FR-113 AC-3 (貼り付けの改行の正規化) は L3 (ブラケットペースト・システムクリップボード・内部レジスタの 3 経路で、LF / CRLF / CR の文書に LF / CRLF / CR の混じるテキストを貼り、挿入部分がすべて文書の既定改行になること)。FR-113 AC-4 (改行コードの変換) は L3 (パレットの引数のリストから `LF` / `CRLF` / `CR` を選ぶ。混在の文書の変換・ステータスバーの改行コード・Enter で挿入される改行・文書が dirty になること・1 回の Undo での本文と `default_eol` の戻り・変換済みの文書で履歴が増えず `info.eol.unchanged` が出ること・読み取り専用の文書の拒否) と、1 MiB を超える文書のジョブの作り直し (`ManualJobSpawner::run_jobs_stale()` で編集の後に結果を届ける)、往復コーパス (§4) の混在ファイルを変換して保存し再読込した結果が 1 種の改行になること。FR-116 は読み取り専用で開くこと・理由の表示 (`ui.read_only_reason.*`)・編集の理由付きの拒否 (`msg.edit.read_only`) (AC-1)、書込権限の無いファイルの保存の `PermissionDenied` で文書が読み取り専用 (理由 `NoWritePermission`) に切り替わり、名前を付けて保存が案内され内容が失われないこと (AC-3)、読み取り専用属性のファイルを書き込める別のパスに名前を付けて保存すると編集できるようになり、`--readonly` と内容に由来する理由の文書は読み取り専用のままであること (AC-4)。解除のコマンド (AC-2) は v0.2。FR-127 は閾値が定数 (設定は v0.2)。各分岐の文言は 04 §15.2 の MessageKey で照合 |
| FR-105 | T | L3 の `MemFs` 障害注入 (§3.4。AC-2: in-place のバックアップ失敗で何も書かず、確認の既定がキャンセル。in-place の乾式走査で表現不能なら元ファイルが不変)、eve-platform の L0 (`ReplaceFileW` の失敗 → `MoveFileExW` を使わず確認 → in-place、§3.4)、L5 の kill テスト (§3.6 (6)。in-place の経路と、`ReplaceFileW` の失敗の注入を含む。中断の検出は MessageKey で照合し、`msg.open.interrupted_save` (`complete = true`) / `msg.open.interrupted_save_incomplete` (`complete = false`) / `msg.open.interrupted_save_no_backup` (`backup = None`) / `msg.open.interrupted_new` (`method = NewFile`、新規保存の中断) の 4 つと、02 §4.5 の写像表の「上書きの可能性あり」の行のキーを分け、バックアップの前の中断 (`kill_after_inflight_flag`) では報告しないことも確かめる、04 §15.2 / 03 §9.2) |
| FR-106 | T | eve-platform の L0 (実 FS、Windows: 読み取り専用属性のファイルが書き換わらず属性も変わらないこと (AC-1)・シンボリックリンク / ジャンクション・NTFS ハードリンク・ダングリングリンクのエラー、§3.4)。「読み取り専用を解除して保存」と Unix のモード・所有者 (AC-4) は v0.2 |
| FR-109, 112 | T | L1 の文字コード性質 (§3.2)、L3、コーパス (§4)、`fuzz_decode` (§3.3) |
| FR-117 (保存直前の照合) | T | L3 (`MemFs` の `FileStamp` 変更)。AC-2 は BM-103b の負荷下の判定 (§5.2) |
| FR-122 (進捗・キャンセル) | M | L7 (§3.8)、BM-104 (§5.2) |
| FR-201〜204, 206 | T (FR-206 AC-3 は M) | L1 (Undo・ChangeSet・入力列)、L3、L5 (§3.6 (9))、受信口の L1 (FR-206 AC-4)。FR-206 AC-3 (Windows Terminal での AC-1 / AC-2) は L6 / L7 の個別の項目 (§3.7 の「貼り付け」、§3.8 の FR-206 AC-3 の行) |
| FR-205 | T/M | L3 (`FakeClipboard` の失敗と遅延の注入。AC-2 / AC-3)、eve-platform の Win32 クリップボード層の L0 (薄い trait のモックと、補助プロセス `clip-delay` による実クリップボードの失敗・遅延、§3.4)、L7 (AC-1。Windows の 2 端末、Win32)、BM-103b の負荷下の判定 (AC-3 は `clip-delay --delay-render` で取得を遅らせた状態、AC-4 (b) は 60 MB の全選択のコピー中の p50 / p99)、AC-4 (a) は L3 (`FakeClipboard`、§3.4: 100 MB の全選択のコピーが文字列化の前に総バイト数で判定され、内部レジスタだけにコピーされる) と BM-103b (100 MB のコピー中)。OSC 52 は v0.2 |
| FR-301〜306 | T | L1 (単語境界・表示幅)、L3、単語境界のゴールデン (§4) |
| FR-401, 402 (大文字小文字の区別) | T | L3 (トグルのコマンド `search.toggle_case` 経由、§3.4)、到達性 (トグルのキーが対象端末で届く、§3.4)。正規表現は v0.2 |
| FR-404 | B | BM-103b の負荷下の判定 (§5.2)、L3 (AC-2〜AC-4)。BM-108 / 109 は参考値 |
| FR-501 | T/M | L4 (§3.5)、L6 (§3.7) |
| FR-502, 506, 507 | T | L3 / L4、NFR-503 の出力検査 (§3.5) |
| FR-503, 504, 505, 511, 512, 514 | M | L7 (§3.8)。自動化できる部分: 項目・優先度・文言は L3、80 桁未満の省略とヒントの圧縮は L4 (§3.5)、確認ダイアログの操作は L3 (§3.4)、FR-512 AC-1 は L5 のフレーム時刻 (§3.6 (4)。CI では記録のみで、合否は基準機 ENV-W1 で 100 ms 間隔・100 回以上のリサイズを行い p99 ≤ 16 ms (最大値は参考)、§3.6 の「時間の判定」)、FR-504 は常時表示 (非表示は v0.2)、FR-514 はパレットに `hidden` 以外の全コマンドと割当キーが出ることを L3 (ヘルプ画面は v0.2) |
| FR-513 | T/M | `xtask check-themes` (§3.5: truecolor と mono の段・ロールの形・truecolor のコントラスト・ロール名・近似した 256 / 16 の衝突)、AC-3 は eve-tui の L0 の表駆動テスト (環境変数 → `color_depth` の判定関数、04 §5 の判定表の各行、§3.5)、L4 の色深度 (§3.5)、L7 / L6 (AC-4。Windows Terminal と conhost で `theme.color_mode` を各段に固定した場合と `NO_COLOR` で、白背景と黒背景の両方の端末設定で区別を失わない) |
| FR-601, 603 | R/T, M | FR-601 AC-1: 生成ドキュメントの突合 (§11.2。コンテキストの列と CommandId の個数 63 を含む)、L3 (`hidden` 以外の全コマンドを、`when` が真の場面でパレットから実行)。AC-2: `docs-tables` ジョブの 07 §11.6 の突合 (`perm` の列と `CommandSpec.permission`、§6.2。M1 は eve-core のテスト、M2 以降は eve-tui と bin の突合も) (T) と、レビューのチェックリストの「権限」の行 (権限判定フックを通らない実行経路が無い、§8.4) (R)。M1 で判定しても合格が自明なので M4 と M5 で再判定する (08 §2.2) |
| FR-602 | M | L3、L7 |
| FR-605 | M (T で補助) | L7 (候補窓の位置。Windows Terminal・conhost × Microsoft IME・Google 日本語入力)。ハードウェアカーソルの位置 (編集面・パレット・プロンプト・検索バー・全角の上) は L4 / L5 で自動検査 (§3.5) |
| FR-606 | T/M | L3 の到達性検査 (§3.4。Windows Terminal と conhost ごとに 04 §7.3a の横取り表を入力にする)。横取り表の 2 端末の行は S-07 の実機確認 |
| FR-609 | T/M | L5 (§3.6 (3)) |
| FR-701〜703 | T | L3 (7 キーの反映、公開前のキーの情報診断、起動時の要約)、L1 (eve-config。任意バイト列でパニックしない) |
| NFR-101〜106 | B | BM-101a/b〜BM-106 (§5.2。NFR-106 は v0.1 が読み取り専用で開いた長大行の表示・`End`・横スクロール、v0.2 が char 近似モードでの表示・`End`・中央挿入)。NFR-106 の表示は L3 も |
| NFR-107 | B | BM-103b を 100 MB の保存ジョブの実行中に測る (§5.2) |
| NFR-201〜204 | B / T | BM-201〜204 (§5.2、Windows の指標)、NFR-203 は L1 の会計 (§3.2) |
| NFR-301 | T | §3.4 の `MemFs`、L5 (§3.6 (6))。電源断の模擬は v0.2 (§6.3) |
| NFR-302 | T (M で補助) | L5 (§3.6 (7)(8)。パニック注入・CTRL_CLOSE・外部から送った CTRL_C / CTRL_BREAK (補助プロセス `console-event`)・メインがブロックした状態の CTRL_CLOSE と CTRL_C・02 §11.2.1 の段ごとの時間予算。AC-5 (100 MB を 5 秒) はパニック / Fatal の経路 (§3.6 (8a))。シグナルの経路は別項目で、CTRL_C / CTRL_BREAK は退避の段 2.5 s での `complete = false` の打切りと全体 5 s、CTRL_CLOSE は退避の段 3.5 s での `complete = false` の打切りと全体 4.5 s (§3.6 (8b)))。AC-3 / AC-5 の時間は CI では記録のみで、合否は基準機 ENV-W1 の手動実行 (§3.6 の「時間の判定」)。CTRL_CLOSE が CI で届かなかった場合は失敗で、環境起因なら基準機で再判定 (§3.6 の「コンソールイベントの送り方」)。**AC-6 (ダウングレード)**: L3 の未知の `format_version` の退避のフィクスチャ (§3.4 のダウングレードの退避)。AC-4 (Windows のログオフ・シャットダウン) は既知の制約 (01 §4.6)。Unix のシグナルは v0.2 |
| NFR-303, 304 | T | L1 (§3.2)、コーパス (§4)、L3 |
| NFR-305 | T | `fuzz_decode` の 1 ターゲット (§3.3、§6.3)。キー入力列と設定は L1 の proptest (§3.2) |
| NFR-401 | T | テストマトリクス (§6.4。Windows x86_64 の全テスト + Linux x86_64 の Linux CI の対象 crate)。リリースは Windows x86_64 のアーカイブ (§9.2) |
| NFR-402 | M | L6 / L7 (§3.7〜3.8。Windows Terminal と conhost) |
| NFR-403 | M | L3 のプリセット別シナリオ (§3.4)、L6 |
| NFR-404 | T | eve-platform の L0 (実 FS、Windows のパス。UNC は `\\localhost\<ドライブ>$\…` の管理共有で作る。Windows の項目の `skip!` は失敗、§4)、パスの proptest (§3.3) |
| NFR-406 | R | `xtask check-deps` と clippy (§7) |
| NFR-501 | T/R | §3.6 の「無通信」: v0.1 は Windows の配布ビルドの依存の禁止リスト (`cargo deny check bans`、`deps` ジョブ) と依存規則 R3 (clippy の `disallowed_types`) のレビュー、テレメトリのコードが無いことのレビュー (§10)。Linux の CI のヘッドレスのシナリオを strace / seccomp の下で実行する検査は v0.2 (NFR-501 AC-2) |
| NFR-502 | T/R | AC-1: L3 (§3.4)。AC-2 (R、リリース前): §8.2 の「セキュリティ (リリース前の確認)」の行の手順 (DLL の検索順: `/DEPENDENTLOADFLAG:0x800` と `SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32)` の確認と偽の DLL による起動の確認。UNC パスの挙動の確認と既知の制約の判断) を行い、リリースのチェックリストに記録する |
| NFR-503 | T | L4 の出力検査 (§3.5)、L1 の入力列 (§3.2) |
| NFR-504 | R/T | L3 (§3.4 末尾の NFR-504)、レビュー (§8.4 秘匿) |
| NFR-505 | T | L5 (§3.6 (10)。Windows の DACL) |
| NFR-507 | T/R | `deps` ジョブ (§6.2)、夜間の `cargo audit` (§6.3) |
| NFR-602 | R | `i18n` ジョブの (2) (§6.2)、レビュー |
| NFR-603 | T/R | `i18n` ジョブ (§6.2。`ja` とコードの MessageKey の差分ゼロ・ヒント幅)、全シナリオを `ja` で (§3.4)、言語の選択を行わないこと (§3.4)、BM-101a (§5.2) |
| NFR-604 | R/T | L4 の無色段 (§3.5)、`xtask check-themes` の (d) (§3.5) |
| NFR-605 | T | L1 の表示幅、L4 の仮想画面モデル (§3.5) |
| NFR-701, 707 | T | L3 (§3.4。Rust の Harness のシナリオ)。NFR-707 AC-4 は `Query` での検査、AC-5 は `drain_events` のシナリオ (文書の変更・メッセージ・オーバーレイの開閉の後の `EditorEvent`、発火が Document / Message / Overlay 系に限られること、§3.4)、AC-6 は `Query` の決定性 (L1 の性質 §3.2 と L3)、AC-2 は M5 で `tests/ac-checklist.toml` から確かめる |
| NFR-704, 706 | R | R5 / R6 (§7)、レビュー (§8.4)。NFR-704 AC-1 は M1 で `unsafe` を持たない crate の `#![forbid(unsafe_code)]` を判定し、FFI を持つ eve-platform / eve-tui が入る M2 以降に再判定する (08 §2.2) |

要件を持たない検査: BM-B01 (§5.2) は記録のための検査で、対応する FR / NFR は無い。文言の照合 (04 §15.2 の目録、§3.4 の「文言の照合」) は 08 G1c の条件 (目録の文書化) に対応する。§3.4 のダウングレードの退避は要件を持つ検査で、01 NFR-302 AC-6 を検証する (05 §18.11、08 G1c の条件にも当たる)。Should で v0.1 に残す記録・監視 (NFR-108 / 109 / 111 / 112 / 205 / 206 / 207 / 702 / 705) は §5.2 の参考値と CI の報告で行い、ゲートにしない。NFR-708 (Should) は v0.1 はこの表から起こすチェックリスト (§8.3)、`xtask trace` は v0.2。v0.1 で実装する Should / Could の機能 (01 §4.2 の「Should / Could (機能)」の行。AC は 01 §5.4 の備考で、`tests/ac-checklist.toml` の対象にしない) も同じ L3 / L4 で検査し、CI で失敗にする (ゲートの判定には使わないが、実装したものの回帰は止める): FR-117 AC-3〜AC-6 (`file.reload`) は L3 (`MemFs` でファイルを書き換えてから実行。未編集で確認なしに読み直すこと、dirty で `ui.confirm.reload_discard` が出て既定がキャンセルであること、1 回の Undo で本文・`TextFormat`・読み取り専用の理由が戻ること (再読込の後に読み取り専用でない文書で。読み取り専用属性のファイルを読み直したとき、および読み直した内容に不正バイトがあるときは、`info.reload.done_read_only` が出て `edit.undo` が実行できず文書が変わらないこと。提案、OQ-216)、ファイルを消すと `msg.reload.missing` で文書が変わらないこと、無題の文書でパレットに淡色と理由で出ること、読込中の `Esc` で元のままであること) と BM-103b の負荷下の判定 (100 MB の再読込中の入力、記録のみ)。FR-403 AC-2〜AC-4 (リテラルのすべて置換) は L3 (置換欄の開閉と `Tab` の巡回、`Alt+A` と `Tab` → `Enter` の 2 経路、大小区別の 3 値での対象、1 回の Undo、空の置換文字列、一致なし・空の検索語の通知、読み取り専用の文書で置換欄が開かないこと、`run_jobs_stale()` での作り直しと `msg.search.replace_stale`) と BM-103b (100 MB・10 万件の置換の準備中の入力、記録のみ)。FR-508 AC-1 (全角スペースとタブの可視化) は L4 (ASCII Frame のスナップショットで記号 `[]` / `>` とロール `ui.whitespace`、セル位置・カーソル位置・ハードウェアカーソルが可視化しない場合と一致すること、無色の段で `dim` が付くこと、ASCII モードでも同じ記号であること) と L3 (コピーした文字列が U+3000 とタブのまま)。

---

## 9. リリース手順

### 9.1 バージョニングと頻度

- SemVer。`0.y.z` の間は `y` が互換性を壊し得る (設定・CommandId の変更は CHANGELOG に移行手順)。版の名前は 01 §1.3 (v0.1 最小核 → v0.2 / v0.3 → v0.4 実用化 → **v1.0 安定化** → v1.L (ローカル LLM 先行) / v1.X)。`1.0` の判定は OQ-44 (確定 (ユーザー回答)、08 §8)。v0.1 の `release.yml` は TUI と公開 crate のみ。
- リリースの時期はゲート駆動 (版のゲートを満たした時点で出す。08)。定期の周期は置かない。パッチ版は随時 (出せる条件は §9.3)。プレリリース `-alpha.N` / `-beta.N` / `-rc.N` は GitHub Release で pre-release フラグ、パッケージマネージャには流さない。**プレリリースのタグは `main` ではなく `develop` の先頭のコミットに付ける** (その前に `develop` に `-rc.N` 等の版を付ける PR を入れる、§9.2 手順 1a。`main` はリリースした版だけを持つ、11 §9.1。`release.yml` のタグの検証は「正式版のタグは `main` のマージコミットを指す、プレリリースのタグは `develop` の先頭を指す」ことを確かめる、§6.1)。**版番号**: 機能リリースは `0.y.0`、bugfix リリースは `0.y.z` の z を上げたパッチ版 (z は 10 以上も可。版・タグの比較は数値順で、`git tag --sort=version:refname` を使う)。v0.1.0 のリリースまでの内部カウンタ `0.0.x` (マイルストーンごとに +1、タグなし) はパッチ版とは別物。リリース PR で workspace の版と CHANGELOG を `0.y.z` にする (§9.2 手順 2)。`X.Y.0-dev` のような `-dev` 付きの版は使わず、リリース後の `develop` の版は直近のリリース版のままにして、次のリリース PR で更新するv0.x の間のリリースチャネルは stable 1 本で、継続的な β チャネルを設けるかは v1.0 のリリース前に決める (05 §18.13、OQ-166)。
- MSRV の引き上げはマイナーのみ (05 §16.1)。

### 9.2 手順

```
1. 凍結: `develop` 上で機能の追加を止める (`dev/<版>` の線と `release/*` ブランチは使わない。ブランチの運用の正本は 11 §9.1)。夜間ジョブの対象は `develop` のままでよい (§6.1。§8.2 の fuzz の行の 5 日 (v0.5.0 以降は 7 日) の観測はここから数える)。
   **ゲートの判定から昇格・タグ (手順 4) まで `develop` への merge (Dependabot の PR を含む) を保留する** (11 §9.1)。
   例外は `main` → `develop` の取り込み (パッチ版のリリース後の取り込み、§9.3) だけで、版上げ (手順 2) の前に限って merge してよい
1a. プレリリース (任意。`-rc.N` / `-beta.N` 等、§9.1): 昇格しない。`develop` に workspace の版を `X.Y.Z-rc.N` 等にする PR を入れ
   (release.yml の検証 (3) はタグの版と `Cargo.toml` の版の一致を見るので、接尾辞まで合わせる)、その PR を入れた `develop` の先頭に
   署名付きタグを付ける (release.yml のタグの検証は「`develop` の先頭を指す」ことを確かめる、§6.1)。プレリリースは何度出してもよく、
   正式版の版上げ (手順 2) で接尾辞を外す
2. 版上げ: `develop` 上のリリース PR で workspace の version を一括更新し (`0.0.x` (v0.1。内部カウンタ、§9.1) または直近のリリース版、あるいは `X.Y.Z-rc.N` 等 (手順 1a のプレリリースを出した場合) から `X.Y.Z` へ。`-dev` 付きの版は使わない、11 §9.1。v0.1 は bot を使わず人間が手で更新する。release-plz による自動化は v0.2 以降の候補、OQ-74)、CHANGELOG の Unreleased を vX.Y.Z に (Keep a Changelog 形式)。**ゲートの前に行う** (ゲートは出荷する版番号のソースで判定する)。
   ゲートの判定コミット (手順 3) を決めたら、`develop` の先頭と `main` の先頭がどちらもその祖先であること
   (`git merge-base --is-ancestor develop <判定コミット>` と `git merge-base --is-ancestor main <判定コミット>`。前者は判定コミットが `develop` の先頭であることを、後者は `main` に `develop` に無い変更 (取り込み忘れ) が無いことを確かめる) を確かめる
   (祖先でなければ `main` を `develop` に取り込む (PR) など `develop` を整えて手順 2 からやり直す。昇格の merge で判定コミットに無い変更が `main` に入らないようにする)
3. ゲート: **版上げ後の `develop` の先頭** で §8.2 のうちタグの前に済む行を実施し、リリースのチェックリスト (`docs/compat/<version>/release-checklist.md`、§8.2。判定したコミットの SHA を書く) を埋める。perf/compat/acceptance の記録とチェックリストを `develop` にコミットする (タグの前。タグの後の結果は Release のノートに書く、§8.2)。記録の許可リスト (`docs/compat/<version>/`・`docs/perf/<version>/`・`tests/ac-checklist.toml`) 以外の変更をこの後に入れたら、ゲートをやり直す
4. 昇格とタグ: `develop` → `main` の 1 段で、人間が作る PR (merge commit) で入れる。`main` のマージコミットに人間が vX.Y.Z (署名付きタグ) を付ける。release.yml が起動し、最初のステップでタグを検証する (`git verify-tag` と `.github/allowed_signers`・`main` のマージコミットを指すこと・`Cargo.toml` の版との一致・ゲートを判定したコミットとの内容の一致 (差分が許可リスト `docs/compat/<version>/`・`docs/perf/<version>/`・`tests/ac-checklist.toml` だけ)、§6.1)。
   昇格が終わったら `develop` への merge の保留を解く。`main` のマージコミットは `develop` の祖先でないので、リリース後に `main` → `develop` の取り込み PR (merge commit) を人間が作る。機能版のリリース直後からパッチ版を出し終えるまでは feat の merge を後回しにする (§9.3)
   - プレリリースは昇格しない (手順 1a)
   - タグの署名鍵は minisign の鍵とは別の鍵 (Git の SSH 署名鍵。種類・保管場所・公開方法は 11 §9.7)
   - release.yml の失敗: Release を publish する前 (draft のまま) なら、原因を直して同じタグで再実行してよい。
     ワークフローの一時的な失敗は「失敗したジョブの再実行」、成果物の内容に関わる修正が要るときは
     メンテナが draft の Release とタグを削除し (タグ `v*` の削除はメンテナに限る、11 §9.1)、
     修正を `develop` に入れて手順 3 (ゲート) と手順 4 (昇格) をやり直してから、同じ版でタグを付け直す。
     publish した後は同じ版のタグを付け直さない (成果物を差し替えない)。直すときは次のパッチ版を出す (§9.3)
5. ビルド (release.yml):
   - Windows: x86_64 (v0.1 の配布物はこれのみ、確定 (ユーザー回答))。CRT は静的リンク (`+crt-static`、11 §1 の `.cargo/config.toml`)。aarch64 (クロス、Tier B ビルドのみ) は v0.2 から。Authenticode (v0.4+)
   - Linux / macOS (それらを出荷する v0.2 から): Linux musl (zigbuild または musl-tools、完全静的、OQ-112)、Linux gnu (OQ-112 の条件成立時、または deb/rpm を作る v0.4 から)、macOS aarch64 / x86_64 (各ネイティブランナー。v0.2 は ad-hoc 署名。Developer ID 署名 + notarytool による公証の取得時期と適用は未決、OQ-76)
   - 分割デバッグ情報 (.pdb/.dSYM/.debug) を別アセットに (05 §18.6)
   - THIRD-PARTY-NOTICES.md (cargo about) を同梱。man・補完スクリプトは v0.4 (05 §18.2)
6. 検証 (release.yml): Windows のランナーで「アーカイブ展開 → zip の内容物の検査 (§8.2 の「配布物」) →
   テスト専用の経路が無いことの検査 (`test-hooks` / `--bench-*`、§6.2 の `build-release-smoke` と同じ) →
   evoeditor --version がタグと一致 → L5 スモーク 3 本 (§3.6 の S1〜S3。展開した exe を `EVE_EXE` で渡す)」(出荷する OS が増えたら各 OS のランナーで)。
   手動の配布経路の検証 (MSRV のツールチェーンでの `cargo install --tag` の経路・クリーンな Windows 11 での手動インストール・
   Smart App Control、§8.2) は draft の成果物とタグで行い、結果を draft の Release のノートの「検証」の節に書く
   (タグの後の結果はリポジトリに戻さない、§8.2)。minisign の検証は署名 (手順 7) の後なのでここでは行わず、手順 10 で行う
7. 署名: リリースのジョブは成果物と `SHA256SUMS` を作るまで。メンテナが手元 (オフライン鍵) で minisign の署名を作り、`SHA256SUMS.minisig` を添付する (05 §18.9、11 §9.7)。公開鍵は README・リポジトリの `minisign.pub` と、リポジトリ外の経路でも公開する
8. SBOM: cargo cyclonedx → evoeditor-X.Y.Z.cdx.json (v0.4)
9. GitHub Release を draft で作成 → 人が CHANGELOG 本文と §8.2 の「更新手順の文書」(リリースノートの「更新の方法」、README の「更新」節、脆弱性の修正を含む版は SECURITY.md と GitHub Security Advisory、11 §9.8) を確認し、リリースのチェックリスト (タグの前に `develop` にコミットしたもの) がすべて埋まり、ノートの「検証」の節に publish 前の結果があることを確かめる → publish
10. 公開後の検証: publish した Release から (手元のビルド成果物ではなく) zip・`SHA256SUMS`・`SHA256SUMS.minisig` を
    ダウンロードし、README と `minisign.pub` に載せた公開鍵で `minisign -V` が通り、zip のハッシュが `SHA256SUMS` と
    一致することを確かめる。結果を Release のノートの「検証」の節に追記する (リポジトリには戻さない)。不一致なら直ちに Release のノートの先頭に
    「検証に失敗したので使わない」旨を書き、告知 (手順 13) を止めて原因を調べる。手順 4 の「publish した後」と同じく、
    同じ版の成果物は差し替えずに次のパッチ版で出す (§9.4)
11. crates.io (公開の時期は確定 (ユーザー回答、OQ-20 / 10 の残件 #1): 1.0 の前。v0.1 は公開しないので、それまでは手順 11 を飛ばす。名前の空きは v0.1 のリリース前と各リリースの前に照会する): cargo publish を依存順 (eve-view → eve-text → eve-encoding → eve-config → eve-core → eve-platform → eve-host → eve-tui → evoeditor。eve-protocol は dev 用で公開しない。eve-testkit は外部の消費者が版を固定して使う公開 crate として、eve-host の後に公開する (12 §4.1)。eve-docs-tables・eve-test-helpers・eve-bench・xtask は `publish = false`)。release-plz による自動化は v0.2 以降の候補 (OQ-74)。
    公開 crate は 1.0 まで 0.x で公開され、12 §4.1 の範囲外の公開 API は「安定性の保証なし」と crate 文書 (README / lib.rs の doc) に明記する。外部の消費者は `=0.y.z` か git タグで固定する想定 (OQ-157)。crate 名は OQ-01 の確定前に公開しない
12. パッケージ (v0.4 以降。05 §18.1: v0.1 は GitHub Releases + `cargo install --locked --git` のみ): Homebrew tap formula bump (bot)、Scoop bucket (bot)、AUR (手動/bot)、winget PR (wingetcreate)、deb/rpm を Release に添付、cargo binstall メタデータ
13. 告知・08-roadmap の繰り越し記録更新
```

### 9.3 ホットフィックス

bugfix リリースはパッチ版 (`0.y.z` の z、§9.1) で出し、`hotfix/**` ブランチは作らない。経路は 1 本だけ: `fix/…` → `develop` (PR、squash) → `main` (PR、merge commit) で入れ、`main` のマージコミットに人間が `vX.Y.Z+1` の署名付きタグを付ける (版上げ・ゲート・昇格とタグは手順 2〜4 と同じ。ゲートは §8.2 の行のうち変更に関わるもの)。リリース後に `main` → `develop` の取り込み PR (merge commit) を人間が作る (bot は使わない)。
**パッチ版を出せる条件**: 前回のタグ以降の `develop` に feat が 1 件も入っていない (fix・chore・spec だけ)。feat が既に入っているときは、バグ修正を次の機能版に含めて出す (または機能版のゲートを早める)。このため、機能版のリリース直後からパッチ版を出し終えるまでは feat の merge を後回しにする。
残余リスク: 条件を守るために feat の merge を止めるので、緊急の修正が `develop` の状況 (feat の取り込み済み・ゲートの待ち) によっては機能版に束ねられて公開が遅れうる。正本は 11 §9.1 (OQ-167 の細目)。

### 9.4 ロールバック

公開済みバイナリは削除しない (再現性。利用者が直前の版に戻す手順 (05 §18.12) もこれを前提にする)。問題のある版は Release ノートに **「既知の重大な問題」** と、直前の版に戻す手順 (README の「更新」節) への案内を追記し、パッケージマネージャは前版へ差し戻し PR。crates.io は `cargo yank`。エディタに更新機能・自動のロールバックは無い (ADR-0040)。

---

## 10. クラッシュ報告・テレメトリ

### 10.1 方針

| 項目 | 決定 | 根拠 |
|---|---|---|
| 利用統計 (テレメトリ) | **実装しない** (既定オフではなく、コードが存在しない) | ADR-0019、NG8、NFR-501、P2 |
| クラッシュレポートの自動送信 | **実装しない** | 同上 |
| クラッシュログ | ローカルファイルに保存 (05 §7.3)。ユーザーが内容を確認した上で手動で Issue に添付する | NFR-302、NFR-504 |
| メモリダンプ (minidump) | **生成しない** (メモリダンプは本文を含む。バックトレース + ビルド ID で解析する) | NFR-504 |
| 更新確認 (ADR-0041、採否は未決) | 採る場合、NFR-501 の検査 (01 §5 の AC-1 と、v0.2 からは AC-2 のネットワークのシステムコール 0) に「**`--check-update` を実行しない限り通信 0**」を加える (編集セッションと、`--version` 等の他の非対話の起動のどれでも 0)。コマンド名は確定 (ユーザー回答) | ADR-0040 / 0041、NFR-501 |
| 将来の opt-in 送信 | 本書では決めない。必要になったら **新 ADR** で、送信内容・保存先・保持期間・撤回方法を定める | ADR-0019 の「別 ADR」 |

### 10.2 クラッシュログの内容 (NFR-302 / 504)

| 含める | 含めない |
|---|---|
| バージョン、コミットハッシュ、ビルド ID (05 §18.6)、ターゲット triple | 文書本文、選択テキスト、クリップボード内容 |
| OS 名・版 (Windows はビルド番号)、アーキ、端末種別のヒント (`TERM`、`WT_SESSION` の有無、`TMUX` の有無、`SSH_TTY` の有無を **真偽値で**)、ロケール | 環境変数の値そのもの (パスやユーザー名を含むため)、ユーザー名、ホスト名 |
| パニックメッセージ (**本文を含まないことをコードで保証**: `panic!`/`expect` の引数に文書テキスト・パスを埋め込まない。レビュー項目 §8.4。ただし標準ライブラリのパニック (文字列の切り出しが文字の境界でない等) は自前のコードに依らず **対象の文字列の一部をメッセージに含む** ので、パニックフックと `catch_unwind` の後の記録は payload が `&'static str` のときだけその文字列を書き、`String` (書式付きのメッセージ。本文を含み得る) のときは書かずに「書式付きのメッセージ (省略)」とする (提案、OQ-203)。ログ (02 §11.3) と終了時の stderr も同じ。検査は §3.4 の NFR-504) と位置 (file:line)、バックトレース (シンボル解決は配布時 strip のためアドレス + ビルド ID。05 §18.6 で解決) | パス全体 (ファイル名のみ、または `blake3` 先頭 8 文字) |
| 直前 50 件の **CommandId のみ** (引数は除外。引数に挿入テキストが入るため)、InvocationSource、各時刻 (相対 ms) | キー入力列 (パスワード等の入力が含まれ得る)、Transaction 内容 |
| 開いていた Document 数、各サイズ・文字コード・改行コード・read_only・load_state | Document のパス |
| スレッド名、パニックしたスレッド | 他スレッドのスタック (本文を指す可能性が高い) |
| 緊急退避の結果 (退避先ディレクトリ、成功/失敗) | — |
| 設定ファイルの **キー名一覧** とエラー (値は含めない) | 設定値 |

- `evoeditor --crash-report` (v0.2、07 §18.3。サブコマンドではなくオプション、07 §18.3 の命名規則): 最新のクラッシュログのパスと内容を表示し、Issue テンプレートの URL を案内する (**M9c で実装した**。試験は `bins/evoeditor/tests/cli_config.rs`)。ユーザーが貼り付けるのは本人の判断。v0.1 は終了時の案内 (04 §14.4) にクラッシュログのパスを表示し、「このファイルを Issue に添付」を案内する。
- クラッシュログは `state_dir/log/` に **20 件** まで保持 (古いものから削除。02 §14.1 MR11、05 §7.3)。
- 起動時に前回クラッシュを検出 (`recovery/` に未処理ディレクトリ) → FR-118 とは別に、退避の場所を `msg.recovery.found` (04 §15.2) で表示し、利用者が手で開く (v0.1。復旧 UI と swap-lite は v0.2 FR-118 / NFR-307)。

### 10.3 デバッグ支援 (明示 opt-in)

- 設定 `log.include_document_text = true` (07 §5.11。v0.1 は定数 `false` で、キーの公開は v0.2): ログに本文断片を含める。**起動時に警告を 1 回表示**し、ステータスバーに記号を出す (02 §11.3)。既定 false。リリースビルドでも使えるが、クラッシュログには **この設定でも本文を書かない**。
- `--bench-*` 隠しフラグ (§5.2) と `test-hooks` feature (§3.6) は **配布ビルドに含めない**。どちらも feature `test-hooks` を有効にしたビルド (テストビルドと計測用ビルド、§5.2) にだけ入る。配布ビルドに入っていないことは `build-release-smoke` と `release.yml` で機械的に確かめる (§6.2、§9.2 手順 6)。

---

## 11. ドキュメント運用

### 11.1 仕様書 (docs/00〜12)

- 00 §7.1 の運用ルールを継続。実装着手後も **仕様が正**: 実装が仕様と異なる判断をする場合、人間の PR は同一 PR で該当文書と 10-decisions (ADR 置換 / OQ 起票) を更新する。エージェントは docs の本編を直さず、OQ 起票案を添えて人間に渡す (§8.1 の「仕様整合」、11 §4.5)。
- 他文書への依頼は、10 に OQ として起票するか、該当文書を同じ PR で直接直す。各文書に依頼の表を残さない。
- 文書内の ID (FR/NFR/ADR/OQ/BM/規範行 ID) への参照は `docs.yml` の `xtask check-docs` (v0.2) で存在確認 (壊れた ID 参照・壊れた内部リンクで失敗)。**M10 の実装** (2026-10-31): `docs.yml` を置くまでは `xtask check` の (5c) と xtask の試験で走る (CI の `check` ジョブ)。検査するのは (1) 本文の `FR-` / `NFR-` / `ADR-` / `OQ-` / `BM-` の参照が定義されていること (定義は索引と §5.2。`OQ-07` と `OQ-7` は同じ。`FR-1xx` のような分類名・`BM-1xx`・`BM-101` (`BM-101a` などの総称)・「欠番」を述べる行・見出しの範囲 (`OQ-161 以降`) は数えない)、(2) `NN §a.b` の節の参照が索引にあること (文書番号の無い `§a.b` は数えない)、(3) 内部リンクの先のファイルと見出し (GitHub のアンカーの規則。見出しの中のリンクは文字だけにする)、(4) `docs/ownership.toml`。コードフェンスとインライン コードの中は例として数えない。`docs/spec-index.toml` (生成) と `docs/ownership.toml` (手書き、02 §4.6) の整合も同じジョブで (11 §3)。v0.1 はリンク検査のみ (`spec-index` ジョブは v0.2、§6.2)。`lychee --offline` で Markdown リンク検査。`markdownlint` (表の列数、見出し階層)。
- **docs を入力にする CI 検査** (§11.2 のカタログ・キーマップとの突合、§3.4 の横取り表、§3.5 の `check-themes` のロール表、`i18n` の MessageKey 目録とヒント列、02 §4.1 と `deps-policy.toml` の突合。`ci.yml` の `docs-tables` / `check-themes` / `i18n` ジョブ、§6.2) は、読む表の形を 11 §8.5 の機械可読の規則 (見出し・列の固定) に従わせ、検査するコードと同じリポジトリ・同じブランチに docs/ がある前提で動く。docs/ を含むブランチの運用は 11 §9.1 に従う (確定 (ユーザー回答)、OQ-167 / 168。docs/ は最初の push から開発ブランチで公開される)。成果物 (`docs/spec-index.toml`・`docs/compat/`・`docs/perf/`・`docs/reference/`・`docs/user/`) も docs/ の下に置く。
- Mermaid 図は GitHub がレンダリングする。画像は置かない (差分レビューのため)。

### 11.2 生成ドキュメント (実装との乖離防止)

| 生成物 | 生成元 | 検査 |
|---|---|---|
| コマンド一覧 (`docs/reference/commands.md`) | `evoeditor --list-commands --json` (CommandRegistry から ID・タイトル・引数スキーマ・when・権限・既定キー) | `docs.yml` で `cargo xtask gen-reference --check` (windows。bin をビルドして JSON を出させ、Markdown に変換した結果と比べる、§6.1) を実行し、差分があれば失敗 (「`cargo xtask gen-reference` で生成物を更新してコミットしてください」)。コードだけを変える PR でも走る。あわせて **07 §11.6 の v0.1 CommandId カタログ (仕様)** と突合する (`docs-tables`、§6.2) |
| 既定キーマップ表 (`docs/reference/keymap.md`) | `evoeditor --dump-keymap --json` (「コンテキスト × キー → コマンド」) + 07 §11.6 カタログの「コンテキスト」「既定キー」「ヒント」列 (04 §7.2 はその設計入力) との突合。コマンド・コンテキスト・キーの組が一致し、ヒントの文脈と優先度が `HintModel` の定数表と一致すること。v0.1 の CommandId の個数 (07 §11.6 の 63 個) と `--list-commands` の行数も一致し、版注記付きの ID (25 個) が登録されていないこと | 同上。**カタログ (仕様) が正、生成物は実装出力**。差分があれば **どちらを直すか判断** する (実装が仕様を上書きする運用にしない。ADR-0025)。カタログを直す場合は 07 の PR と同時に |
| 設定リファレンス (`docs/reference/config.md`) | `evoeditor --dump-config --defaults --json` (v0.2、07 §18.3。eve-config の既定値) | 同上 (v0.2 から) |
| 注釈付き設定例 (`config.example.toml`、手書き、v0.1 から同梱) | 07 §4.1 / §5 | **v0.1 から** CI で eve-config の既定値 (Rust の定数) と突合し、キー名・既定値の差があれば失敗 (07 §4.1) |
| 文言キー一覧と欠落 (NFR-603) | `eve-config`/core の MessageKey 列挙 ↔ `locales/ja.toml` | **v0.1 から欠落・余剰キーは失敗** (§6.2 `i18n` ジョブ。キー集合の差分ゼロが合格)。ヒント幅とプレースホルダの一致、**あるキーが別のキーの接頭辞になっていないこと** (TOML で文字列とテーブルが衝突するため。§6.2 `i18n` の (7)) も同じジョブ。言語間の差分の検査は 2 つ目の言語を同梱する版から |
| man ページ `evoeditor.1` | 手書き (lexopt のため自動生成なし) + `evoeditor --help` 出力との突合テスト | 差分で失敗 |
| API ドキュメント | `cargo doc` → GitHub Pages (`gh-pages`)。公開 crate は `#![deny(missing_docs)]` (v0.3 から) | 夜間の `docs-rs` (§6.3) |
| CHANGELOG | 手書き (Keep a Changelog)。PR ごとに 1 行 | §8.1 |
| トレーサビリティ行列 | `xtask trace` (v0.2) | §8.3 |
| 規範行 ID 索引 (`docs/spec-index.toml`、v0.2) | `xtask spec-index` | §6.2 `spec-index` ジョブ (v0.2)。エージェントは索引で該当節だけを読む (11 §3) |
| 性能・互換記録 | `docs/perf/`, `docs/compat/` | リリースゲート |

### 11.3 ユーザー向けドキュメント

- `README.md` (インストール (v0.1 は Windows の手順: 検証、ダウンロードした zip のブロックの解除、展開先、`Path` への追加、SmartScreen と Smart App Control の扱い、`cargo install` の経路。内容の正本は 05 §18.2a)、最初の 5 分、**更新** (配布経路ごとの更新手段・更新とダウングレード・直前の版に戻す手順。05 §18.10〜§18.12、ADR-0040)、アンインストール (05 §18.8)、署名の検証 (05 §18.9)、スクリーンショットは `asciinema`/`vhs` で生成した GIF を Release に置き、リポジトリ本体には入れない)。
- `docs/user/` (v0.1 リリース時): 設定 (v0.1 の 7 キー)、キーマップのカスタマイズ (v0.2)、文字コード、トラブルシューティング (v0.1 は Windows Terminal と conhost: 05 §9〜10 の内容をユーザー向けに書き直したもの。`xattr -d com.apple.quarantine`・WSL のクリップボード・tmux 設定は macOS / Linux を出荷する版で加える)。v0.1 はヘルプ画面が無いので、キー一覧・端末の設定・IME とチョードの注意もここと README に置く。
- アプリ内ヘルプ (FR-514。v0.1 はパレットで代用、ヘルプ画面は v0.2) は生成コマンド一覧と同じソースから出す (重複保守をしない)。
- 言語: アプリの UI 文言は **v0.1 は日本語のみ** (NFR-603、OQ-21、確定 (ユーザー回答))。英語 UI は v0.2 (確定 (ユーザー回答))。**v0.1 の利用者文書 (`README.md`・リリースノート・`docs/user/`) は日本語を正** とし、英語版は v0.2 の英語 UI と同時に用意する (確定 (ユーザー回答)、OQ-21)。

### 11.4 貢献者向け

- `CONTRIBUTING.md` (人間向け。最初の push の前に最小の版 (外部からの PR をまだ受け付けていない旨・DCO の手順・寄稿のライセンス、12 §5.6 の PC-4) を置き、外部寄稿の受付開始前に次の内容の完全な版にする): ビルド (v0.1 は Windows。Linux は Linux CI の対象 crate のビルドとテスト、§6.4)、`cargo nextest` の導入、`xtask` 一覧 (11 §1 と同じ。v0.1: `check`, `check-deps`, `check-themes`, `i18n`, `gen-corpus`, `bench`, `gen-reference` (Windows), `nightly` (夜間ジョブの入口、§6.1)。v0.2: `spec-index`, `trace`, `scenario run` / `scenario check`, `screenshot`, `check-docs`, `check-assets` (最初の同梱資産と同時、12 §3.6)。v1.L: `ai-mock` / `ai-record` (`ai-record` は人間のみ)), PR テンプレート、依存追加チェックリスト、`spec:` / `verifies:` の書き方、flaky テストの扱い。**寄稿の条件** (11 §9.4): DCO の手順 (`git commit -s` で `Signed-off-by:` を付ける。無いコミットは `dco` ジョブで失敗)、「寄稿は公開リポジトリのライセンス (Apache-2.0) で提供される」の明記、AI コーディング支援で生成したコードの扱いの方針 (12 §3.5。v0.1 のリリース後 に決める、OQ-164)。
- `CLAUDE.md` / `AGENTS.md` / `.claude/` (AI エージェント向け) の内容と、エージェントのタスク単位・検証ループ・ブランチ規約は **11-dev-workflow が正本** (02 §17)。本書からは重複して書かない。
- `CODEOWNERS`: `Cargo.toml`/`Cargo.lock`/`deny.toml`/`deps-policy.toml` と `docs/10-decisions.md` はメンテナ必須レビュー。
- Issue テンプレート (最初の push の前に置く、12 §5.6 の PC-7): バグ (OS・端末・版・クラッシュログの貼り付け欄 (v0.2 からは `evoeditor --crash-report` の出力)、**本文を含めないよう注意書き**)、機能要望 (関連 FR/OQ 欄)、端末互換報告 (マトリクス形式)。

---

## 12. 未決事項

番号は [10 §2](10-decisions.md#2-未決事項-open-questions) の統一 OQ 番号。区分・詳細は 10 を正とし、本表は本書に関わる OQ の結論 (確定) と、未決のものの結論案を記す。

| OQ | 件名 | 結論 (未決は結論案) | 区分 / 決定期限 |
|---|---|---|---|
| OQ-101 | カバレッジ可視化 | **確定 (ユーザー回答)**: CI artifact のみ (Codecov は使わない)。v0.1 は報告のみ、ゲートは v0.4 | 裁量 / 回答済み |
| OQ-102 | `aarch64-pc-windows-msvc` の実機テスト | **確定 (ユーザー回答)**: Tier B (ビルドのみ + リリース前の手動確認)。`windows-11-arm` ランナーが使えれば全テストに採用 | 裁量 / 回答済み |
| OQ-15 | `x86_64-apple-darwin` の CI (Intel ランナーの提供終了後) | **確定 (ユーザー回答)**: Tier B (ビルドのみ)。全テストはランナーがある間だけ夜間 / release で。macOS を出荷する版から適用 (v0.1 は Windows のみ) | PO / 回答済み |
| OQ-103 | ワーカープール (可変数・優先度) を再評価する判定材料 | **未決**: v0.1 は固定 2 本 (`io` / `bg`、ADR-0006)。v0.4 (並列検索・tree-sitter) で BM-108/109 と BM-101 を見て判断 | 裁量 / v0.4 着手前 |
| OQ-60 | `regex-cursor` の採否 (02 §13.5) の判定基準 | **未決**: v0.1 は行単位検索 (リテラルは memmem) で regex-cursor を使わない。v0.3 の S-04 でキャプチャ・CRLF モード・`(?i)`・Unicode クラス・最新 regex-automata との互換を確認し、BM-109r で行単位方式と比較、2 倍以上遅くなければ採用 (保守の停滞リスクを判定項目に含める) | スパイク (S-04) / v0.3 着手前 |
| OQ-74 | リリース自動化ツール | **未決**: v0.1 は bot を使わず、版上げ・CHANGELOG・昇格 PR・`main` → `develop` の取り込み PR を人間が作る。配布ビルドは手書き workflow。release-plz (版上げ + CHANGELOG + publish) は v0.2 以降の候補 | 裁量 / v0.1 リリース前 |
| OQ-104 | OSS-Fuzz への登録 | **未決**: v0.4 以降、知名度次第 | 裁量 / v0.4 |
| OQ-105 | `-Z minimal-versions` による依存下限の検査 | **未決**: 行わない (コスト大)。MSRV ジョブと `Cargo.lock` で足りる | 裁量 / — |
| OQ-44 | 1.0 の判定条件 | **確定 (ユーザー回答、2026-10-28)**: v1.0 (安定化の版) を 1.0 とする (01 §1.3 / 08)。v0.4 (実用化) の後に NFR-601 / NFR-306 のゲートを満たした時点 | PO / v0.2 着手前 (確定済み) |
| OQ-106 | NFR-601 ユーザーテストの協力者確保方法と判定 | **確定 (ユーザー回答)**: v0.2 で 3 名の所見のみ (ゲートにしない。v0.1 では行わない、§3.8)、v1.0 のゲートで 5 名中 5 名 (失敗時の所見を記録し 04 ヒントバーへフィードバック) | PO / 回答済み |
| OQ-107 | Miri の導入 | **未決**: 導入しない (eve-text は unsafe なし、R5 対象 crate の FFI は Miri で実行不能) | 裁量 / — |
| OQ-143 | クラッシュ一貫性テストの道具 (§6.3) | **未決**: LazyFS (FUSE、セットアップが軽い) を第一候補とし、夜間ジョブの Linux ランナーで実行できなければ dm-log-writes (夜間ジョブは v0.2) | 裁量 / v0.2 着手時 |
| OQ-144 | 自前仮想画面モデル (§3.5) の範囲 | **未決**: 曖昧幅・絵文字・VS16・PUA の 4 軸を設定可能にし、`vt100` は L5 の読取用途に限定 | 裁量 / v0.1 中 |
| OQ-112 | Linux の配布物 (musl のみ) の判定材料 | **未決**: musl 静的のみ。BM-104 / BM-202 を musl 版と gnu 版で測り、1.5 倍 / 1.3 倍を超えたら gnu 版を追加 (§5.2、05 §17.3)。Linux を出荷する版で判断する (v0.1 は Windows のみ) | PO / Linux を出荷する版のリリース前 |
| OQ-113 | 数値目標の統計量 (p50 / p99、p99.9 は参考値) と測定条件 | **確定 (ユーザー回答)**: 01 §3 の値。BM-103b は 1,000 回で p50 / p99 を判定し、p99.9 は参考値として記録する (ゲートにしない。01 §3.0 の「統計」。v0.1 は Windows の基準機)。条件は 01 §3.0.1 | PO / 回答済み |
| ADR-0039 / OQ-44 / OQ-101 / OQ-106 | 品質ゲートの版分割 (本書での読み方) | v0.1 のリリースゲートの範囲は **ADR-0039 (確定 (ユーザー回答))**: fuzz (`fuzz_decode` の 1 ターゲット)・C を含む依存 0・Must AC (§8.2)。契約版 1 の固定と推移的依存の総数は v0.1 のゲートにしない。カバレッジのゲートは v0.4 (OQ-101、確定 (ユーザー回答))、NFR-601 の 5 名は v1.0 (OQ-106、確定 (ユーザー回答))、8 時間ソークと 1.0 の判定条件は OQ-44 (確定 (ユーザー回答)、上の OQ-44 の行)。実装着手ゲートの分割 (G1a / G1b / G1c) は OQ-115 で、本表の対象ではない | ADR-0039 は確定 (ユーザー回答)。OQ-44 は確定 (ユーザー回答、2026-10-28) |
| OQ-157 | 公開 API の互換方針: `cargo semver-checks` をゲートにする時期と、`0.y` の間の破壊的変更の運用 (外部の消費者 = 駆動 API クライアント等) | 確定: v0.1 リリースから **報告**、v0.4 から **ゲート** (§7.1) (破壊的変更は `y` 上げ + 移行手順 + 契約版の並行期間)。範囲は 12 §4.1、固定は `=0.y.z` / git タグ (12 §4.2) | 裁量 / v0.1 リリース前 (公開) / v1.0 リリース前 (並行期間) |
| OQ-145 | 実 `llama-server` の契約テスト (§3.9) の実行環境 | **未決**: GitHub ホストのランナーで CPU 推論 (小さい GGUF、夜間週 1、任意) で開始 (形の一致だけを見るので遅くても足りる)。NFR-116 の実機値 (BM-117) は基準環境で手動 | 裁量 / v1.L-a 着手時 |
| OQ-146 | `MockLlamaServer` が追従する llama-server の API 版と、版差の吸収 | **未決**: S-22 時点の 1 版に固定し、能力検出 (`/props`) で未知フィールドを無視する契約だけを検査する。版を上げるときはカセットを再記録し、`llama_api.md` に差分を書く (OQ-128 の試験側) | 裁量 / v1.L-a 着手前 |
