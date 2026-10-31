# 03. コア設計 (Core Design)

| 項目 | 内容 |
|---|---|
| 文書ID | 03-core-design |
| 状態 | ドラフト |
| 主対象 crate | `eve-text`, `eve-core` ([02 §4.1](02-architecture.md#41-crate-一覧)) |
| 前提 | [00-overview](00-overview.md) (用語・原則) / [01-requirements](01-requirements.md) / [02-architecture](02-architecture.md) |
| 決定記録 | [10-decisions](10-decisions.md) (ADR-0004 / 0009 / 0010 / 0016 / 0020 / 0023 / 0026 / 0031 / 0039 / 0042 / 0043、OQ-03 / 04 / 05 / 06 / 16 / 17 / 18 / 31 / 32 / 42 / 110 / 113 / 114 / 116 / 138 / 139 / 162) |
| v0.1 で読む節 | §1.4、§2〜§9、§10.1 / §10.1a、§11 (§11.6 を除く)、§12.1〜§12.4 / §12.6 (§12.3 は v0.1 では大文字小文字を区別しないリテラル検索の実行経路として読む)、§13.1〜§13.3、§14.1〜§14.2 / §14.5 / §14.6、§15 (§15.3 は改行と、選択なしの Tab / Shift+Tab のみ)、§17、§19〜§22 (一覧の正本は [README §3.1](README.md#31-v01-の実装で読む節)) |

> 本書は [02 §12.1](02-architecture.md#121-文書と-crate-の対応) で 03 に割り当てられた事項 — `TextStorage` の API、位置変換、書記素/単語境界、ChangeSet/Transaction、Undo の結合規則と上限、Selection の正規化と写像、文字コード判定と往復保証、改行コード、ファイル読込/保存の **論理手順**、文書内検索、Revision とジョブ結果の採否、表示幅・折り返しのアルゴリズム — を定める。
> 原子的置換の OS 実装は 05、描画は 04、設定項目名は 07 の責務であり、本書では「core が platform/frontend に何を要求するか」までを書く。
>
> コード片はすべて **Rust 風の擬似コード (型シグネチャ程度)** であり、実装ではない ([ADR-0001](10-decisions.md#adr-0001-仕様書駆動での開発))。「未決」の事項は 10-decisions の状態を正とする。
>
> **版の表記**: **v0.1 (最小核) / v0.2 (日常の不足を埋める) / v0.3 (日本語と巨大ファイルの完成) / v0.4〜v1.X** を使う (版と要件の対応の正本は 01 / 08)。後の版に回した機能も **型・Port・ID などの拡張点だけは v0.1 で残す** ([§1.4](#14-版ごとの範囲と先行投資))。core は frontend を知らない境界 (02 §8.4) の内側に置き、v0.1 の検証はヘッドレス / テスト用 Frontend で行う。core に frontend 固有の分岐を持ち込まず、core の機能はすべて TUI から使える。

---

## 目次

1. [目的・スコープ・設計上の視点](#1-目的スコープ設計上の視点)
2. [テキストストレージ (TextStorage)](#2-テキストストレージ-textstorage)
3. [座標系と位置変換](#3-座標系と位置変換)
4. [位置・範囲・選択 (Pos / Range / Selection)](#4-位置範囲選択-pos--range--selection)
5. [カーソル移動のセマンティクス](#5-カーソル移動のセマンティクス)
6. [ChangeSet と Transaction](#6-changeset-と-transaction)
7. [Undo / Redo](#7-undo--redo)
8. [文字コード・BOM・改行コード](#8-文字コードbom改行コード)
9. [ファイル I/O の論理手順](#9-ファイル-io-の論理手順)
10. [クラッシュ復旧・スワップ・バックアップ](#10-クラッシュ復旧スワップバックアップ)
11. [巨大ファイル戦略](#11-巨大ファイル戦略)
12. [検索・置換の基盤](#12-検索置換の基盤)
13. [Unicode 処理](#13-unicode-処理)
14. [表示幅・折り返し・レイアウト](#14-表示幅折り返しレイアウト)
15. [タブとインデント](#15-タブとインデント)
16. [シンタックスハイライト基盤](#16-シンタックスハイライト基盤)
17. [Revision とジョブ結果の採否規則](#17-revision-とジョブ結果の採否規則)
18. [AI 差分適用 (Proposal) との接合](#18-ai-差分適用-proposal-との接合)
19. [型スケッチのまとめ](#19-型スケッチのまとめ)
20. [不変条件一覧](#20-不変条件一覧)
21. [性能目標との対応](#21-性能目標との対応)
22. [テスト観点](#22-テスト観点)

<!-- -->

23. [未決事項](#23-未決事項)

---

## 1. 目的・スコープ・設計上の視点

### 1.1 本書が満たす要件

| 領域 | 要件 ID |
|---|---|
| バッファ・位置 | ADR-0004, ADR-0010, NFR-105, NFR-106, NFR-202 |
| 編集・Undo | FR-201〜204, FR-206, FR-208, FR-211〜213, NFR-203, NFR-306 |
| 選択・移動 | FR-301〜306, FR-308〜309 |
| 文字コード・改行 | FR-108〜114, NFR-303, NFR-304, OQ-03, OQ-04, OQ-16 |
| ファイル I/O (論理) | FR-101〜102, FR-104〜107, FR-115〜119, FR-122〜123, FR-127, NFR-104, NFR-107, NFR-301, NFR-302, NFR-307, NFR-505 |
| 検索 | FR-401〜404, NFR-108, NFR-109 |
| Unicode / 表示幅・折り返し | FR-501, FR-506, FR-507, NFR-605, G8 |
| 構文 | FR-801, NFR-110, ADR-0015 (基盤のみ) |
| AI 接合 | FR-903, ADR-0013 (ChangeSet の rebase 規則) |
| 品質 | NFR-305, NFR-701, NFR-702 |

### 1.2 設計の 5 つの視点

本書の各判断は、次の視点で衝突がないことを条件とする。衝突した場合は [00 §4](00-overview.md#4-設計原則-優先順位付き) の優先順位 (P1 > P2 > …) に従う。

| 視点 | 問い | 本書での主な現れ方 |
|---|---|---|
| **性能** | NFR-103 (入力 2 ms) / NFR-105 (100 MB で 1 ms) / NFR-106 (10 MB 行) を満たすか | rope、可視範囲限定のレイアウト、長大行の読み取り専用での縮退表示 (v0.1、§11.5)・char 近似モード (v0.2) と列チェックポイント (v0.3、§14.4)、全走査はジョブ |
| **安定性・メモリ** | 想定外の入力・長時間の運用で OOM・ハング・データ損失に至らないか | Undo 会計に現れないメモリを作らない (§6.5 / §7.5)、プロセス全体の Undo 上限、受信口へ送る側の合体規則 (§12.2 / §17.3)、メインスレッドで大きな文字列を作らない (§12.6)、`try_reserve` |
| **正しさ** | Unicode・改行・複数カーソル・Undo が規定どおり一貫するか | 書記素境界への正規化、ChangeSet の代数的性質、プロパティテスト |
| **データ損失防止 (P1 原則)** | どの経路でも元のバイト列・未保存内容を失わないか | 不正バイト保持、改行の非正規化、保存前の表現不能チェック、スワップ・緊急退避、Reload も Undo 可能 |
| **frontend 境界の維持** | core が端末概念を持ち込んでいないか。TUI とヘッドレス参照 Frontend が同じ core を使えるか (テスト用 Frontend で検証できるか)。frontend 固有の概念が core に入っていないか | レイアウトは「セル」単位、`LayoutEngine` 差し替え点の予約、frontend 差は `Capabilities` のみ |
| **AI・外部駆動との相性** | 外部生成の ChangeSet を安全に写像・部分適用できるか。駆動 API (v0.2 以降) から同じ経路で操作できるか | base Revision 付き ChangeSet、`map_range` と衝突判定 (`rebase` は v1.L)、ハンク分割、`Origin::Ai` / `Origin::External` |

### 1.3 crate への配置 (本書の提案)

| 要素 | crate | 理由 |
|---|---|---|
| `TextStorage`, `Snapshot`, 位置変換、書記素/単語境界、表示幅、`ChangeSet`, `Pos`, `Range`, `Selection` (純データ + ChangeSet による写像) | `eve-text` | I/O なし・core 非依存で fuzz / ベンチしたい。Selection の写像は ChangeSet と不可分 |
| `Document`, `Transaction`, `Origin`, `UndoHistory`, ファイル I/O 手順 (Port 呼び出し)、検索ジョブ、`SyntaxProvider` trait、レイアウト (Frame 生成) | `eve-core` | 状態・Port・ジョブに関わる。`Revision` の型は eve-view が定義し (境界型の ID 型、02 §5.3 / §12.2)、Document (eve-core) が `Snapshot` と対にして持つ |
| 文字コード判定・デコード・encode→decode 照合・PUA エスケープ・保持方式 (§8) | 独立 crate **`eve-encoding`** (02 §4.1、ADR-0003) | `&[u8]` チャンク → `&str` + メタの純粋関数で、I/O も `Document` も要らない。単独で fuzz・ベンチでき、`eve-core` の最大の衝突領域を外に出せる。他ツールからの再利用価値も高い |

### 1.4 版ごとの範囲と先行投資

後の版に回す機能は **公開面 (型・ID・Port・ファイル形式) だけを v0.1 で決めて実装し、内部構造は最小にする**。内部構造 (Undo の木構造、ジョブの優先度、写像用の変更ログ等) は後から crate 内で入れ替えられるため、v0.1 では作らない。本書の各節に版を注記し、ここに一覧する (版の割当の正本は 01 / 08)。

| 機能 | v0.1 | 後の版 | v0.1 で残す拡張点 |
|---|---|---|---|
| 複数文書 (FR-103) | **単一文書**。別のファイルを開く・新規バッファは今の文書を置き換える (§9.2 の「置き換えの手順」。dirty なら FR-107 の確認) | v0.2 (バッファ一覧・`buffer.*`・全保存・プロセス全体の Undo 上限の判定) | 文書の集合 (`EditorState.documents` の `DocumentId` の表) と文書ごとの処理 (ジョブ・退避のスロット) は集合のまま実装し、UI と CLI だけを 1 つに制限する |
| Undo (§7) | 線形 2 スタック | 木構造は FR-213 (v1.L) | `UndoHistory` の API (`undo` / `redo` / `seal` / `saved`) を crate 内に閉じる |
| 不正バイトを含む文書 (§8.3) | 読み取り専用で開き、可視化 | 編集と保存は v0.3 | PUA エスケープの型・チャンク境界の回収 (表示に必要) |
| 復号専用文字の保持方式 (§8.6 E2 / §8.7 区分 B) | 検出して保存時に確認ダイアログ | 保持方式 (バイト一致) は v0.3 | `TextFormat.roundtrip` の型と `LossySpan`。v0.1 は件数と先頭 100 箇所だけを記録 |
| ISO-2022-JP (§8) | **読み書き** (OQ-03 で確定 (ユーザー回答)。02 §13.4)。保存は行ごとにエンコーダを閉じて各行を ASCII 状態で終え、1 行の内部ではチャンクの境界をまたいで状態を引き継ぐ (OQ-162 で確定 (ユーザー回答)、§8.6 E6)。エスケープ選択の差は区分 B として検出・確認 (§8.7) | 元のエスケープ配置を行単位で保持する保持方式は v0.3 (§8.7)。WHATWG デコーダが受け付けない入力 (JIS X 0212 `ESC $ ( D`、冗長なエスケープ等) の寛容な読込は OQ-138 | `Encoding` の列挙と `Encoding::is_stateful()`、`LossyReason::StatefulEscape` |
| Partial (先頭部分の先行表示、§9.2 / §11.3) | なし (進捗表示とキャンセルのみ) | v0.3 | `load_state` の列挙に `Partial` の値だけ予約 |
| 中断された保存の検出 (§9.2 / §10.3) | 開くたびにそのパスの保存中フラグだけを `inflight_for` で調べて場所を案内する (ディレクトリの孤児ファイルは列挙しない)。置換途中の失敗はその場で退避パス付きで報告 | 起動時の一斉走査 (`list_inflight`) と復旧の操作は v0.2 | `list_inflight` の Port 操作 (02 §4.5) |
| フォーカス復帰時の外部変更確認 (§9.5) | なし (保存ジョブ内の照合と、明示の再読込 `file.reload` のみ) | v0.2 (明示コマンド `file.check_external_changes` も)。監視は v0.4 | `JobKind::Metadata` の目的 `MetaPurpose::ExternalCheck` (02 §9.3) |
| スワップ (§10.2) | なし (リスク受容、OQ-42) | v0.2 最優先 (swap-lite) | 緊急退避の manifest を版付きで確定し、スワップと共用 |
| 置換 (§12.5) | リテラルのすべて置換 (検索バーの置換欄、`search.replace_open` / `search.replace_all`) | 1 件ずつの置換・正規表現の置換 (`$1` の展開)・選択範囲内の置換は v0.2 | `search.replace` の CommandId、`SearchQuery.scope` |
| 空白の可視化 (§13.2) | 全角スペースとタブだけ (定数で常に表示) | 切替の設定キーは v0.2、半角の空白・行末・改行は v0.4 (FR-508) | `view.show_whitespace` の名前と `DecorationKind::Whitespace` |
| 正規表現の検索 (§12.3) / 単語単位検索 (§12.4) / 複数行の正規表現 | なし (リテラルと大文字小文字の区別のみ) | 行単位の正規表現と単語単位は v0.2、複数行は v0.3 | `SearchQuery.kind` の `Regex`・`SearchQuery.whole_word`・`SearchBarModel.regex`、`search.toggle_regex` の CommandId、正規表現の実行経路を `TextStorage` の公開 API から隔離 |
| ソフトラップ (§14.3) | なし (水平スクロールのみ) | v0.2 | `View.wrap`、`RowSource` の折り返し番号、`view.wrap` 設定キー |
| 長大行 (§11.5) | 閾値超の行を含む文書は **読み取り専用** で開き、その行は先頭 64 KiB まで表示 | 編集 (char 近似モード) は v0.2、列チェックポイント (§14.4) は v0.3 | 読み取り専用の理由 `LongLine`、閾値の名前 `file.long_line_threshold` (v0.1 は定数)、`LineLayoutCache` の差し替え点 |
| 保存の OS 別手順 (§9.3) | Windows (`ReplaceFileW`) | Unix (rename + ディレクトリ fsync、モード・所有者) は macOS / Linux を出荷する版 | Port の trait (02 §4.5) と OS 別の規則 (05) は Unix の分も仕様として残す |
| `pending_changes` (§17.1) | なし (Revision 不一致の結果は捨てる) | LSP / AI の着手時 (v1.L) | `Document.pending_changes` のフィールドと 02 §5.3 の規則 |
| `DecorationProvider` trait (§14.5) | なし (選択・検索一致は Frame 生成で直接合成) | tree-sitter (v0.4) と同時 | `Decoration` / `DecorationKind` の型 |
| Proposal の演算 (§18) | `ChangeSet::map_pos` / `map_range` / `from_sorted_edits` のみ (選択・検索結果の写像に使う) | `ChangeSet::rebase` と `RebasePolicy` (収束性のプロパティテストを含む) は v1.L (最初の利用者 = Proposal の写像) | 02 §10.2.1 の型 (eve-view) と本章の演算の名前 |
| `Origin::Ai` / `Origin::External` (§6.3) | 列挙に予約 | v1.L / 駆動 API (v0.2 以降、dev feature) | 列挙と `Transaction` の拡張メタデータ欄 |
| `EditorEvent::DocumentChanged` の `EditSummary` (§6.5 手順 10) | **型の予約のみ** (v0.1 の `DocumentChanged` は `edits` を持たない。後から `Option` で追加するのは C2 の「追加」に当たる) | 生成は最初の利用者 (LSP 増分同期、ローカル LLM のキャッシュ境界・補完破棄、v1.L) と同時 | `EditSummary::{Exact, Coarse, Replaced}` と `EditDelta` の名前 (eve-view) |
| `SyntaxProvider` trait (§16.3) | 名前の予約のみ (v0.1 のハイライトは無し) | 完全な署名と `detect_language` は v0.4 (tree-sitter と同時) | trait 名と `LanguageId` |
| `Query` の範囲取り出しの決定性 (§6.5、06 §17 X26) | **保証** (同じ Revision・同じ範囲 → 同じバイト列) | `ContextItem.stability` の型は v1.L | `Query::DocumentText` の契約 |

---

## 2. テキストストレージ (TextStorage)

### 2.1 要求

| # | 要求 | 由来 |
|---|---|---|
| T1 | 任意位置の挿入・削除が O(log n) | NFR-105 |
| T2 | O(1) の不変スナップショット (`clone`)。スナップショットは `Send + Sync` | 02 §2.2 (ワーカーへの受け渡し) |
| T3 | 行数・行頭バイト位置を O(log n) で得る | 行番号表示、`Go to line` |
| T4 | バイト ⇔ char ⇔ UTF-16 ⇔ 行 の変換を O(log n) で得る | ADR-0010、LSP (v1.L) |
| T5 | 内容は常に妥当な UTF-8 | ADR-0009 |
| T6 | チャンク (連続スライス) 単位の走査ができ、全文連結を要求しない | 検索 (リテラルのチャンク跨ぎ走査、§12.3)、tree-sitter の `TextProvider`、保存時のストリーミングエンコード |
| T7 | メモリは元サイズの 1.5 倍 + 定数以内 (許容 2.5 倍) | NFR-202 |
| T8 | 1 行 10 MB でも T1〜T4 が劣化しない | NFR-106 |
| T9 | 実装 (ropey / crop / 自前) を差し替え可能 | ADR-0004, OQ-06 |

### 2.2 API スケッチ

```rust
// eve-text (擬似コード)。ropey の型は一切公開しない。
pub struct Text;                       // 可変ハンドル。Document のみが保持する
pub struct Snapshot;                   // 不変。Clone は O(1)。Send + Sync

impl Text {
    pub fn snapshot(&self) -> Snapshot;                   // O(1)
    // crate をまたぐ可視性の制限は Rust ではできないため pub とする。到達の制限は下記の注で行う
    pub fn apply(&mut self, cs: &ChangeSet) -> Result<(), ChangeSetError>;
}

pub trait TextRead {                   // Text と Snapshot の両方が実装
    fn len_bytes(&self) -> usize;
    fn len_chars(&self) -> usize;
    fn len_lines(&self) -> usize;                        // §2.4 の行定義。末尾が改行なら +1 (空の最終行)
    fn byte_to_char(&self, b: ByteOff) -> CharIdx;        // 事前条件: b は char 境界
    fn char_to_byte(&self, c: CharIdx) -> ByteOff;
    fn byte_to_line(&self, b: ByteOff) -> LineIdx;
    fn line_to_byte(&self, l: LineIdx) -> ByteOff;        // 行頭のバイト位置
    fn line_end_byte(&self, l: LineIdx) -> ByteOff;       // 改行文字の直前 (改行を含まない行末)
    fn line_terminator(&self, l: LineIdx) -> Option<LineEnding>;  // その行を終える改行。最終行は None
    fn byte_to_utf16(&self, b: ByteOff) -> Utf16Off;     // LSP 用 (v1.L)
    fn utf16_to_byte(&self, u: Utf16Off) -> ByteOff;     // サロゲートペアの間を指す u は、その文字の先頭 (直前の char 境界) へ丸める
    fn is_char_boundary(&self, b: ByteOff) -> bool;
    fn chunks(&self, range: ByteRange) -> Chunks<'_>;                  // T6。具体型のイテレータ (Item = &str)
    fn chunk_at(&self, b: ByteOff) -> (&str, ByteOff /*chunk 先頭*/);
    fn slice_to_string(&self, range: ByteRange) -> String;             // 小範囲のみに使う
    fn byte_slice(&self, range: ByteRange) -> TextSlice<'_>;           // 遅延。書記素走査・検索の入力
}
pub struct Chunks<'a>;                 // impl Iterator<Item = &'a str>。内部の rope のチャンク走査を包む (ropey の型は公開しない)

pub struct TextBuilder;                // 読込時に decode 済みチャンクを追記して構築 (RopeBuilder 相当)
```

- **`TextRead` は dyn で使える (オブジェクト安全な) trait とする**: メソッドは型引数を持たず、`impl Trait` を返さない。チャンクの走査は具体型 `Chunks<'_>` を返す。これにより `&dyn TextRead` (Selection の正規化、§4.3。書記素・単語境界の関数、§19) で `Text` と `Snapshot` を同じように扱える。位置変換などの派生の操作 (§3.2) は `TextReadExt` (`TextRead` を実装する全型と `dyn TextRead` への blanket 実装) に置く。

- **可変ハンドル `Text` を Document の外に出さない**ことで、「文書変更は Transaction 経由のみ」([02 §5.2](02-architecture.md#52-変更の唯一の経路)) を守る。`Text` を保持するのは eve-core の `Document` の非公開フィールドだけで、`Document` は `&mut Text` を返す API を持たない。`Text::apply` は `pub` だが、`Text` に到達できるのは `Document::apply` だけになる (eve-text を直接使ってよい crate は 02 §4.3 の依存規則で決まる)。
- `Snapshot` は `TextRead` のみを実装し、ジョブ・サービス・AI へ渡す。`Snapshot` は内容だけを持ち、`Revision` を持たない (eve-text は他の `eve-*` に依存せず、`Revision` の型は eve-view が定義する。02 §5.3)。Revision と結びつけるのは Document (eve-core) で、ジョブ・退避・購読者へは常に `(Snapshot, Revision)` の対として渡す (§17)。
- `TextSlice` は書記素・単語境界・表示幅の計算入力であり、`&str` を返さない (チャンク跨ぎのため)。`unicode-segmentation` はチャンク跨ぎを直接扱えないため、`eve-text` 側で「境界候補の前後に十分なコンテキスト (最大 256 バイト) を結合した小さなバッファ」を作って判定する (§13.1)。

### 2.3 ropey の使い方 (ADR-0004 / OQ-06)

| 論点 | 方針 |
|---|---|
| バージョン | OQ-06 の規則に従う。**要確認**: 2026-10 時点の最新安定版は 1.6.1 で、2.x は `2.0.0-beta.1` (2025-08) のまま安定版ではない。OQ-06 の規則どおりなら **P0 時点では 1.x** になる見込み。1.x の場合 `byte_to_char` / `char_to_byte` の往復コストを `TextStorage` 内に閉じ込め、2.x (バイト中心) なら変換を恒等にする。公開 API は変えない。P0 スパイク S-02 で 1.6 / 2.0-beta / crop の API (行種別の指定方法、バイト中心 API) を比較し、`TextStorage` 抽象がどれにも合うことを確認する。S-02 の確認項目には「`RopeSlice` は元の rope を借用し、部分木だけを所有する独立の値にできない」(§6.5 の `InsertText::Slice` と実体化ジョブの前提) も含める。2.x / crop で部分木を安価に所有できるなら、実体化ジョブを省ける |
| 行区切り | **LF / CRLF / CR のみ**を行区切りとする ([§8.4](#84-改行コード-oq-04))。ropey の `unicode_lines` 相当 (U+0085 NEL / U+2028 LS / U+2029 PS / VT / FF を行区切りにする機能) は **無効化** する。これらは制御文字・不可視文字として可視化 (FR-507。表記は §13.2: NEL は `<U+0085>`、LS / PS は `<LS>` / `<PS>`、VT / FF は `^K` / `^L`) し、行には影響させない。ropey の feature で切れない場合は行計算を自前で持つ (`line_index` 補助構造) |
| チャンク | ropey の葉ノード (既定 1 KiB 前後) を `chunks()` でそのまま露出する。チャンク境界は char 境界であることが保証される (ropey の不変条件)。**CRLF が葉を跨がないことは保証されないものとして** `line_terminator` はチャンク跨ぎを考慮して実装する。ropey 1.x は CRLF をチャンク境界で分割しない設計と理解しているが **要確認**: S-02 で確認し、保証があれば実装を簡略化できる。2.x / crop へ移行した場合の保証も同時に確認する |
| 行数 | `len_lines` は ropey の O(1)/O(log n) 行カウントを使う。巨大ファイルでも読込完了時点で行数が確定する (全行走査を別途しない) |
| `Send + Sync` | ropey の `Rope` は `Send + Sync` で、clone は `Arc` の参照カウント増加のみ。`Snapshot` の T2 を満たす |
| 読込 | `TextBuilder` は decode 済み `&str` チャンクを追記する (`RopeBuilder::append`)。全文 `String` を一度も作らない |

### 2.4 行の定義

- **行区切り**: `LF`, `CRLF`, `CR` の 3 種のみ。`CRLF` は 1 つの区切りとして数える (2 行にならない)。
- **行 (LineIdx)**: 0 始まり。「文書先頭から n 個目の区切りの直後から、n+1 個目の区切りの直前 (または文書末尾) まで」。
- **最終行**: 文書が区切りで終わる場合、その直後に **空の最終行** が存在する (`len_lines` = 区切りの数 + 1)。これにより `末尾改行あり/なし` (FR-114) は `最終行が空か否か` と一致し、カーソルを文書末尾の空行に置ける。
- 行の **内容範囲** `[line_to_byte(l), line_end_byte(l))` は区切りを含まない。カーソルが置ける行内位置の最大は `line_end_byte(l)` (区切りの直前)。カーソルは CRLF の `CR` と `LF` の間には置けない ([§4.1](#41-pos-の正規化))。
- **行の構造は内容だけで決まる**: `\r` の直後に `\n` があれば、それがどの編集で並んだかに関係なく 1 つの CRLF である。したがって編集の端で単独の `CR` と単独の `LF` が隣接すると 2 つの区切りが 1 つに **合体** し (行が 1 つ減る)、CRLF の間に文字を挿入すると `CR` と `LF` の 2 つに **分離** する (行が 1 つ増える)。`eve-text` はこれを禁止せず、`len_lines`・`line_terminator`・`eol_stats` はいつも内容どおりに数える (更新規則は §6.5 の「eol_stats の更新」)。利用者の編集コマンドが意図せず合体・分離を起こさないようにする規則は §6.2 の「編集の端の改行の保護」で定める。

### 2.5 TextStorage の不変条件

| ID | 不変条件 |
|---|---|
| INV-T1 | 内容は常に妥当な UTF-8。挿入文字列は `&str` であることで保証する。位置は **全 Op の境界** (各 `Retain` / `Delete` の両端と各 `Insert` の位置。いずれも適用前の文書のバイト位置) を検査し、1 つでも char 境界でなければ `apply` は `ChangeSetError::NotCharBoundary { pos }` を返して **何も変更しない** (検査は書き換えの前にまとめて行う。`len_before` が文書のバイト数と違うときは `ChangeSetError::LenMismatch` で、これも何も変更しない)。`ChangeSet::from_sorted_edits` も同じ検査を行い、違反を `NotCharBoundary` で返す (§6.2) |
| INV-T2 | `len_lines() == (区切りの数) + 1` |
| INV-T3 | `Snapshot` は生成後に内容が変化しない。`Text::apply` 後も既存 Snapshot は旧内容を保つ |
| INV-T4 | `byte_to_char(char_to_byte(c)) == c`、`char_to_byte(byte_to_char(b)) == b` (b が char 境界のとき) |
| INV-T5 | 任意の `l < len_lines()` で `line_to_byte(l) <= line_end_byte(l) <= line_to_byte(l+1)` |

---

## 3. 座標系と位置変換

### 3.1 座標系の一覧

| 座標系 | 型 (案) | 単位 | 用途 | 正準か |
|---|---|---|---|---|
| バイトオフセット | `ByteOff(usize)` = `Pos` | UTF-8 バイト | **正準** ([ADR-0010](10-decisions.md#adr-0010-位置の正準表現))。ChangeSet、Selection、検索結果、tree-sitter、ファイル I/O | **はい** |
| char インデックス | `CharIdx(usize)` | Unicode スカラ値 | ropey 1.x 内部のみ。`eve-text` の外に出さない | いいえ |
| UTF-16 オフセット | `Utf16Off(usize)` | UTF-16 コード単位 | LSP (v1.L) の `Position.character` (既定 negotiation) | いいえ |
| 行・バイト桁 | `LineByte { line, byte_col }` | 行 (0 始まり) + 行頭からのバイト | tree-sitter `Point`、内部の行単位処理 | いいえ |
| 行・書記素桁 | `LineGrapheme { line, col }` | 行 + 書記素数 | ステータスバーの「行:桁」(FR-503)、`path:行:桁` (FR-102)、Go to line | いいえ |
| 行・表示列 | `LineDisplay { line, dcol }` | 行 + 端末セル (タブ展開後) | 上下移動の目標列 (FR-301)、レイアウト、マウスヒット | いいえ |
| 可視行・セル | `(RowIdx, CellCol)` | 折り返し後の表示行とセル | `Frame` 内の座標。View とビューポートに依存 | いいえ |

> UI の表示は 1 始まり (「行 12, 桁 5」)、内部はすべて 0 始まり。変換は Frame 生成時のみ行う。
> UTF-16 変換 (LSP、v1.L) では §8.3 のエスケープ文字 `U+10FF00..` は補助面の文字としてサロゲートペア 2 単位に数える (特別扱いしない)。LSP に送る文書本文でのエスケープ文字の扱い (そのまま送るか、マーカーに置換するか) は eve-lsp 文書 (v1.L) で決める。

### 3.2 変換関数と計算量

```rust
// eve-text。impl<T: TextRead + ?Sized> TextReadExt for T (dyn TextRead にも使える。§2.2)
pub trait TextReadExt: TextRead {
    fn pos_to_line_byte(&self, p: Pos) -> LineByte;               // O(log n)
    fn line_byte_to_pos(&self, lb: LineByte) -> Pos;              // O(log n)。byte_col が行長を超えたら行末に丸め、char 境界でなければ直前の char 境界へ丸める (書記素境界への丸めは呼び出し側が snap_to_grapheme で行う)
    fn pos_to_line_grapheme(&self, p: Pos) -> LineGrapheme;       // O(log n + 行頭からの距離)。長大行は §14.4 のチェックポイントで O(log n + 4 KiB)
    fn line_grapheme_to_pos(&self, lg: LineGrapheme) -> Pos;      // 同上。col が行の書記素数を超えたら行末
    fn pos_to_display(&self, p: Pos, cfg: &WidthConfig) -> LineDisplay;   // タブ幅・曖昧幅設定に依存
    fn display_to_pos(&self, ld: LineDisplay, cfg: &WidthConfig, bias: Bias) -> Pos; // セルの途中 (全角の右半分) は bias で前後に寄せる
    fn pos_to_utf16(&self, p: Pos) -> LineUtf16;                  // LSP (v1.L)
}
```

| 規則 | 内容 |
|---|---|
| C1 | 行頭からの距離に比例する計算 (書記素・表示列) は **行内の走査** を伴う。通常行 (数百バイト) では無視できない長さにならない。長大行 (NFR-106) では、v0.1 は閾値 (1 MiB) 超の行を含む文書を読み取り専用で開き、その行は先頭 64 KiB までしか走査しない ([§11.5](#115-長大行-nfr-106))。v0.2 で閾値超の行を **char 近似モード** で扱い、ASCII の連続には高速経路を入れて走査コストを下げる (10 MB 行で各操作 p99 ≤ 100 ms)。v0.3 で [§14.4](#144-長大行の列チェックポイント) のチェックポイントにより上限を 4 KiB の走査に抑え、書記素単位の正確さを取り戻す |
| C2 | 変換の入力が範囲外 (行数超、行長超) の場合はパニックせず **末尾に丸める** (NFR-305)。丸めたことは戻り値のフラグで知らせ、呼び出し側 (Go to line 等) が通知に使う |
| C3 | 表示列への変換は `WidthConfig` (タブ幅、曖昧幅 1/2、制御文字の表示方式) に依存するため、設定変更時はレイアウトキャッシュを無効化する |
| C4 | `LineGrapheme.col` の「桁」は **書記素数 (UI 上は 1 始まり)** とする (FR-102 / FR-503。確定 (ユーザー回答)、[OQ-32](10-decisions.md#oq-32-桁の単位))。設定で表示列 (`LineDisplay.dcol + 1`) に切り替えられる。キー名と公開版は 07 §5 (`ui.column_unit = "grapheme" \| "display"`、公開は v0.2。v0.1 は書記素数で固定)。理由: 全角を含む行でユーザーが数える単位に一致し、OS 間で一致する (P6) |

---

## 4. 位置・範囲・選択 (Pos / Range / Selection)

### 4.1 Pos の正規化

- `Pos` は任意の char 境界を取り得る (ChangeSet の中間値はバイト精度。検索の一致は両端が書記素境界のものだけを採る、§12.1)。
- **カーソル (head) と anchor は書記素境界に正規化** する ([ADR-0010](10-decisions.md#adr-0010-位置の正準表現))。正規化関数 `snap_to_grapheme(pos, Bias)` を `eve-text` が提供し、Selection を View に格納する前に必ず通す。
- CRLF は 1 書記素 (UAX #29 GB3) なので、`CR` と `LF` の間は自動的に不正な位置となる。
- 書記素境界のうち「カーソルを置けない」特別扱いは **設けない** (例: 結合文字の内側は書記素境界ではないので既に除外される)。

### 4.2 Range

```rust
pub struct Range { pub anchor: Pos, pub head: Pos }   // anchor == head なら選択なしのカーソル
impl Range {
    fn start(&self) -> Pos; fn end(&self) -> Pos;      // min / max
    fn is_empty(&self) -> bool; fn is_forward(&self) -> bool;   // head >= anchor
    fn with_head(self, head: Pos) -> Range;            // 選択拡張 (Shift + 移動)
    fn collapse_to_head(self) -> Range;                // 移動 (Shift なし)
}
```

- **半開区間** `[start, end)` を選択内容とする。`end` にある文字は含まない。
- 向き (`anchor < head` か否か) は保持する。Shift+← で縮む/伸びる方向を決めるため。
- Range は **書記素境界に正規化された状態** でのみ View に格納される (INV-S1)。

### 4.3 Selection (複数 Range)

```rust
pub struct Selection { ranges: SmallVec<[Range; 1]>, primary: usize }   // 非空。v0.1〜v0.3 は len() == 1 に制限 (FR-211 は v0.4、01 §4.4)
impl Selection {
    fn primary(&self) -> &Range;
    fn iter(&self) -> impl Iterator<Item = &Range>;    // start 昇順
    fn normalized(self, text: &dyn TextRead) -> Selection;   // §4.4
    fn map(&self, cs: &ChangeSet, policy: MapPolicy) -> Selection;   // §6.4
    fn transform(&self, text: &dyn TextRead, f: impl Fn(Range) -> Range) -> Selection; // 各 Range に移動を適用 → normalized
}
```

### 4.4 Selection の正規化規則

`normalized()` は次を保証する。全操作 (移動・編集・写像・外部からの設定) の後に必ず呼ぶ。

| ID | 規則 |
|---|---|
| N1 | 各 Range の anchor / head を書記素境界へスナップ (head は移動方向の `Bias`、anchor は元位置に近い側) |
| N2 | `start` 昇順にソート。`primary` はソート後も同じ Range を指し続ける |
| N3 | **重なる、または接する** Range は 1 つに併合する。接する (`a.end == b.start`) ものを併合するのは、両方とも非空のときのみ。空カーソル同士が同一位置にあれば 1 つにする。併合後の向きは primary (含まれていれば) の向き、なければ先頭の向き |
| N4 | 空の Range の位置が非空 Range の `[start, end]` (**両端を含む**) にある場合、空の方を捨てる (非空を優先)。選択の端に重なるカーソルは見た目に冗長なため。ただし空の Range が primary のときは、`start < 位置 < end` なら非空の Range に併合し (その Range が primary になる)、端なら残す (primary は捨てない、N5) |
| N5 | ranges が空になることはない (最後の 1 つは削除できない。削除要求は primary を残す) |
| N6 | すべての `Pos <= len_bytes()` |

### 4.5 複数カーソルでの編集セマンティクス (FR-211)

v0.1〜v0.3 では `len() == 1` に制限するが、**編集コマンドはすべて複数 Range を前提に書く** (単一の場合は長さ 1 の特殊例)。

1. **編集は 1 Transaction**: 1 回のコマンド実行で、全 Range に対する変更を 1 つの `ChangeSet` に合成し、1 つの `Transaction` として適用する。Undo は全カーソル分がまとめて戻る。
2. **各 Range は適用前の文書座標で変更を記述**する。コマンドハンドラは `Selection` を走査して `(Range, 局所 ChangeSet)` のリストを作り、`ChangeSet::from_sorted_edits` で位置が重ならないことを検証しつつ合成する。重なる場合 (例: 2 つの Range が同じ単語を削除) は **先行 Range を優先し、後続は重複部分を切り詰める**。切り詰めは **呼び出し側 (コマンドハンドラ) が `from_sorted_edits` に渡す前に行う**。`from_sorted_edits` 自身は切り詰めず、重なりを受け取ったら `Err(Overlap)` を返す (ハンドラの誤りなので debug ビルドでは assert、リリースではコマンドを何もせずに終えてログする)。
3. **自分の挿入は自分の後ろへ**: Range が挿入した文字列に対して、その Range の head/anchor は挿入の **後** (`Bias::After`) に写像する。他の Range が同じ位置にある場合は **前** (`Bias::Before`) に留まる。これにより 2 つのカーソルが同一位置で入力した場合に文字が交互に並ばない (N3 により同位置の空カーソルは事前に 1 つに併合されているため、実際には発生しない)。
4. **コマンドの結果 Selection** は「各 Range が自分の変更をどう追うか」をハンドラが明示する (`selection_after`)。既定は規則 3 の写像。
5. **行単位コマンド** (行削除・行複製・インデント・コメントトグル) は「各 Range が触れる行の集合」を取り、同じ行を複数 Range が触れても **1 回だけ** 処理する。
6. **クリップボード**: 複数 Range のコピーは Range ごとの文字列を改行で連結し、同数のカーソルへの貼り付けは 1 対 1 に分配する (エディタ慣習。04 で UI を定める)。
7. **矩形選択** (FR-212, Could) は「各行に 1 つの Range を持つ Selection + 矩形モードフラグ (表示列で揃える)」として表現し、新しい選択型は作らない。

### 4.6 Selection の View 間写像

同じ Document を複数 View (v0.4) で開いているとき、ある View の編集は他の View の Selection を `Selection::map` で写像する ([02 §5.2](02-architecture.md#52-変更の唯一の経路))。写像ポリシーは `MapPolicy::Other` (挿入位置に一致する点は **前** に留まる、削除範囲内の点は削除開始位置へ潰れる)。スクロール位置 (`ScrollPos.top`、先頭可視行の行頭の `Pos`) も同じ関数で写像し、写像後に行頭へ丸める。

---

## 5. カーソル移動のセマンティクス

すべて `Motion` 種のコマンド ([02 §6.3](02-architecture.md#63-コマンドtransactioneffect-の関係)) であり、文書を変更しない。移動の起点はいつも `head` で、Shift 付きは移動先を `with_head` で head にする (anchor は動かさない)。Shift なしは移動先へ潰す (`anchor = head = 移動先`)。**選択がある状態の Shift なしの移動** は、水平移動 (§5.1) だけが例外で `start` / `end` へ潰して移動しない。上下・Home / End・単語移動・ページ移動・文書端は、選択があっても `head` を起点に移動してから潰す。複数 Range には各 Range 独立に適用し、`normalized()` で併合する。

### 5.1 水平移動 (FR-301)

| コマンド (案) | 動作 |
|---|---|
| `cursor.move_left` / `cursor.move_right` | 書記素 1 つ分。行頭で左 → 前行末 (区切りをまたぐ、CRLF は 1 ステップ)、行末で右 → 次行頭 |
| 選択ありで Shift なし | `cursor.move_left` は `start` へ潰す、`cursor.move_right` は `end` へ潰す (移動しない)。CUA 慣習 |
| 曖昧幅・結合文字 | 書記素単位なので幅の影響を受けない |

> コマンド ID の正本は 07 §11.6 の v0.1 CommandId カタログ。本書では 02 §6.1 の例 (`cursor.move_left`) に合わせた表記を使う。

### 5.2 垂直移動と目標列 (FR-301)

- View は `goal_dcol: Option<DisplayCol>` を持つ。上下移動は `goal_dcol` (未設定なら現在の表示列) を目標とし、移動先行で `display_to_pos(goal, Bias::Before)` を取る。全角の右半分に当たる場合は **前** の書記素境界 (左) に寄せる。
- 水平移動・編集・クリックは `goal_dcol` をクリアし、垂直移動は維持する。 複数 Range のとき `goal_dcol` は primary の Range のもの (View に 1 つ)。他の Range は各自の現在の表示列を目標にする。
- **折り返し ON の View** では、垂直移動は **表示行 (折り返し後の行)** 単位とし、目標列は表示行内のセル列とする。折り返し OFF では論理行単位。これは View のレイアウト結果 (§14) を参照する唯一の Motion であり、core 内にレイアウトがある理由の一つ ([ADR-0008](10-decisions.md#adr-0008-viewmodel-による描画分離))。
- 先頭行で上 → 文書先頭、最終行で下 → 文書末尾 (CUA 慣習。「移動しない」への切替は v0.2 の設定 `editor.vertical_motion_at_edge`、07 §5.13「後続版で公開する設定項目」)。

### 5.3 単語移動 (FR-302)

- 単語境界は [§13.3](#133-単語境界-日本語補助規則) の `word_boundaries` を使う。
- `cursor.move_word_right`: 現在位置から **次の単語の先頭** (CUA/Windows 慣習) か **現在の単語の末尾** (macOS / Unix 慣習) かは OS 間で揺れがある。P6 (同じ体験) により **1 つに固定** し、既定は「空白をスキップして次の単語の先頭。行末に達したら行末で止まり、次の呼び出しで次行頭へ」。macOS 風は v0.2 で設定 `editor.word_motion` として公開 (07 §5.13「後続版で公開する設定項目」)。v0.1 は既定値を定数として実装する。
- `cursor.move_word_left`: 空白をスキップして現在/前の単語の先頭へ。
- 日本語を含む全文字種で **UAX #29 の単語境界は使わず**、文字種の遷移だけで境界を決める ([§13.3](#133-単語境界-日本語補助規則)、OQ-117)。期待停止位置の表 (FR-302 の受入条件) は §13.3。
- **単語単位の削除** (`edit.delete_word_backward` (Ctrl+Backspace) / `edit.delete_word_forward` (Ctrl+Delete)、v0.1。編集コマンドだがここで停止位置を共有する):

| 状況 | `edit.delete_word_backward` | `edit.delete_word_forward` |
|---|---|---|
| 選択なし | `[q, p)` を削除。q は p から `cursor.move_word_left` を 1 回行った停止位置 (§13.3) | `[p, q)` を削除。q は p から `cursor.move_word_right` を 1 回行った停止位置 |
| 空白の連続 | 停止位置の規則どおり、空白を読み飛ばした先の単語の先頭まで消す (`foo   \|` → `\|`、`foo bar\|` → `foo \|`) | 次の単語の先頭まで消す (`\|foo bar` → `\|bar`、`foo\| bar` → `foo\|bar`)。行内に単語が無ければ行末まで消す (`foo\|   ⏎` → `foo\|⏎`) |
| 行頭 / 行末 | 行頭では前の行の区切り (CRLF は 1 つ) を消して前の行と連結する。文書先頭では何もしない | 行末では区切りを消して次の行と連結する。文書末尾では何もしない |
| 選択あり | 選択 `[start, end)` を削除する (Backspace と同じ。単語単位には広げない) | 同左 |
| 何も消えないとき | Transaction を作らない (§6.5 の恒等の扱いと同じく、Revision も Undo 履歴も変わらない) | 同左 |

  Origin は `User` (§7.3) で、連打は R8 により直前の削除と結合する (backward は「新しい削除の末尾 == 直前の削除の先頭」、forward は「新しい削除の先頭 == 直前の削除の先頭」)。Backspace / Delete の 1 書記素削除とも R8 の条件を満たせば結合する。入力欄 (`prompt`・検索バー) でも同じ規則を欄の 1 行に適用する。

### 5.4 行頭・行末・文書端 (FR-303)

- `cursor.home` (スマートホーム): 現在位置が **インデント終端以外** (インデント終端より後、インデントの途中、行頭) ならインデント終端へ、インデント終端にいれば行頭 (`line_to_byte`) へ (トグル)。インデント終端 = 行頭から続く空白 (スペース・タブ) の直後。インデントが無い行 (インデント終端 == 行頭) では常に行頭へ。空白だけの行ではインデント終端 == 行末。折り返し ON では **まず表示行の先頭**、次に上記トグル (3 段階にならないよう、表示行の先頭 = 論理行頭の場合は 2 段階)。
- `cursor.end`: 行末 (区切りの直前)。折り返し ON ではまず表示行末、既に表示行末なら論理行末。表示行末と次の表示行の先頭は同じ `Pos` なので、どちらに描くかは v0.2 で head に持たせる affinity で区別する (§14.3)。
- `cursor.doc_start` / `cursor.doc_end`: `Pos(0)` / `Pos(len_bytes())`。
- **長大行の縮退 (§11.5、v0.1) との関係**: カーソルと選択は長大行の表示範囲 (先頭 64 KiB を書記素境界で切った位置まで) に限るので、文書の最終行が閾値超の行なら `cursor.doc_end` と `select.all` の head は `len_bytes()` ではなく **その行の表示範囲の末尾** に丸める (途中の長大行は選択の内側に入るだけなので丸めない)。この丸めで全選択のコピーが行の残りを黙って落とさないよう、**`select.all` の直後で選択が変わっていない間の `clipboard.copy` は、丸めた選択ではなく文書全体 `[0, len_bytes())` を対象にする** (View に「全選択の丸め中」の印を持ち、選択が変わる操作で消す) (提案、OQ-195。代案は丸めた範囲だけをコピーし「表示範囲外は含まれていません」と通知する)。長大行の文書は読み取り専用なので、切り取りと削除は起きない。

### 5.5 ページ移動 (FR-304)

- 移動量は `viewport_rows - 1` 表示行 (最低 1。01 FR-304 AC-1 の「可視行数 − 1」。前の画面の最終行が次の画面の先頭行として残る)。スクロール位置も同量移動し、カーソルの画面上の相対位置を保つ。目標列は維持する。

### 5.6 指定行へ移動 (FR-306)

- 入力は 1 始まりの行番号 (+ 任意の桁 = 書記素数)。最終行を超える番号は C2 で最終行に丸め、丸めた旨を通知する (`info.goto_line.clamped`)。**1 未満 (0・負数) は丸めずに不正な入力として拒否する** (入力プロンプトは入力欄に `ui.prompt.goto_line.error` を出して開いたまま、コマンドは `InvalidArgs`)。桁は 1 未満を 1、行末を超える値を行末に丸め、通知しない。
- 行番号の代わりに `"$"` (最終行) も受け付ける。引数 `line` の型は 07 §11.2 の `int|"$"` (02 §6.1 の `ArgSchema` では `IntOrLast` (仮称))。`"$"` は文書の最終行を指し、範囲外ではないので丸めの通知 (`info.goto_line.clamped`) は出さない。移動後、カーソルを含む行をビューポート中央に置く (04 でスクロール方針を確定)。

### 5.7 対応括弧・テキストオブジェクト (FR-308 / 309, v0.4)

- 括弧対応は文字ベース (スタックで走査、走査上限 1 MiB で打ち切り) を v0.4 で提供し、tree-sitter の `SyntaxProvider` (v0.4) が対応する言語では構文木ベースを優先する ([§16.5](#165-括弧対応-fr-308-の二段構え))。
- テキストオブジェクト (v0.4) は Motion の派生として Selection を返す関数群とし、Vim 風キーマップ (FR-607) からも再利用できる形にする。CommandId (例 `select.word` / `select.inner_brackets`) は 07 §11.6 のカタログに未掲載の将来候補で、名前は着手時に 07 で決める。

### 5.8 マウス位置 → 文書位置

frontend から `(ViewId, RowIdx, CellCol)` が届く ([02 §7.3](02-architecture.md#73-マウス))。core は View のレイアウト結果から `RowIdx → (論理行, 折り返し番号)` を引き、`display_to_pos` でセル → `Pos` に変換する。行末より右のクリックは行末、空の最終行より下は文書末尾。全角セルの右半分は `Bias::Before` (その文字の前) に寄せる ([§23](#23-未決事項) の OQ-58)。

#### 5.8.1 マウスの扱い (M9b、2026-10-30。案)

M9b の 5 本目の突合。`MouseEvent` / `HitMap` は仕様にあるだけでコードにはまだ無いので、型と挙動をここで決める。

**入力の型 (eve-view)**:

```rust
struct MouseEvent { kind: MouseKind, mods: Modifiers, at: MouseAt }
enum MouseKind { Down(MouseButton), Drag(MouseButton), Up(MouseButton), WheelUp, WheelDown }
enum MouseButton { Left, Middle, Right }
// ヒットの結果。編集面の外 (クローム・オーバーレイ) は TUI が先に判定して core へ渡さない (04 §6.8)
struct MouseAt { view: ViewId, row: i32, col: i32 }   // 編集面 (PaneFrame.rect) の左上を原点とするセル。ボタンを押している間は面の外 (負・はみ出し) もそのまま渡す
```

`HitMap` は**作らない** (02 §8.2 の `PaneFrame.hit` は予約のままにして中身を足さない)。v0.2 の Window は編集面 1 つで、`PaneFrame.rect` と `gutter_width` があれば TUI は (セル → `MouseAt`) を作れ、折り返し・タブ・全角を考慮した文書位置の確定は core が `ScrollPos` とレイアウトから直接引く (§14.3.1。Frame を作り直さない)。複数ペイン (v0.4) で `hit` を実体にする。

**挙動 (core、`Editor::handle_input(Mouse)`)**:

| 操作 | 動作 |
|---|---|
| 左 `Down` | カーソルをその位置へ (Selection を 1 点に)。ドラッグの起点 (anchor) を覚える。`Shift` 付きは anchor を保ったまま head だけ動かす (選択の拡張)。Undo の結合は封印する (カーソル移動と同じ) |
| 左 `Drag` | anchor からその位置までを選択する。面の上下の外では、tick (50 ms ごと、core の deadline) で 1 表示行ずつスクロールしながら選択を伸ばす。マウスが動かなくても進む |
| 左 `Up` | ドラッグを終える (選択は残す) |
| `WheelUp` / `WheelDown` | `ui.mouse_wheel_lines` (既定 3) 表示行ぶんスクロール。カーソルは動かさない。`Shift` 付きは横スクロール (折り返し OFF のときだけ。1 回 = 画面幅の 1/8 を目安に 4 列。案) |
| 中・右ボタン、`Ctrl`/`Alt` 付き | v0.2 は何もしない (中ボタンの貼り付け・右クリックメニュー・複数カーソルは後の版)。キーマップの `mouse.*` も v0.2 では既定のみ (07 §7) |
| ダブル・トリプルクリック | v0.4 (FR-311)。v0.2 は単クリックを 2 回として扱う |
| オーバーレイ・検索バーが開いている間 | 編集面のマウスは無視する (キー操作と同じく、フォーカスは最前面にある) |

**位置の確定 (§5.8 の具体化)**: 面の行 `row` を表示行として引く — 折り返し OFF は `top` の論理行 + `row`、ON は `ScrollPos { top, wrap_idx }` から `row` 本ぶん進めた (論理行, 表示行)。列は、OFF は `left_col + (col − gutter_width)`、ON は表示行内の列 (継続行のずらしを含む。§14.3.1 の `LineRows::at_col`)。ガターの列は列 0 として扱う。画面の下の `Virtual` 行は文書末尾、表示行の幅を超えた列はその表示行の末尾 (折り返し位置では上の表示行の末尾 = `Before`)。全角セルは左右どちらの半分でも**その文字の前** (OQ-58 の推奨を採る)。長大行の読み取り専用の範囲 (§11.5) を超える位置は表示範囲の末尾に丸める (`set_selection` が行う)。

**実装の分け方**:

1. この節 (突合の記録)。
2. eve-view の `MouseEvent` と core の処理 (クリック・ドラッグ・Shift・ホイール・自動スクロール。ヘッドレスの試験)。`ui.mouse_wheel_lines` の定数 3 を持つ。
3. TUI と設定: 端末のマウス取得の有効化 / 解除 (04 §6.8)、`ui.mouse` の公開、`ui.toggle_mouse`、`Capabilities.mouse`。

---

## 6. ChangeSet と Transaction

### 6.1 ChangeSet の形式

```rust
// eve-text
pub enum Op { Retain(usize /*bytes*/), Delete(usize /*bytes*/), Insert(InsertText) }   // InsertText は §6.5 (Str / Slice / Rope)
pub struct ChangeSet {
    ops: Vec<Op>,          // 文書先頭から末尾まで連続。Retain の合計 + Delete の合計 == len_before
    len_before: usize,
    len_after: usize,
}
```

- **文書全体を覆う操作列** (Retain / Delete / Insert) で表す。xi-editor / Helix / CodeMirror 6 と同じ族の表現で、合成・逆変換・位置写像が代数的に定義できる。
- 「位置 + 文字列」の編集リストより扱いにくいが、(1) 複数カーソル編集の合成、(2) AI Proposal の写像、(3) 他 View の Selection 写像、が **同じ 3 つの演算** で済むため採用する。
- `Insert` の文字列は `InsertText` とし、Undo 履歴と Transaction で共有する (コピーしない)。通常の編集は `InsertText::Str(Arc<str>)`。大きな削除の逆変換だけが Snapshot 参照・独立の rope を使う (§6.5)。
- 削除した内容は `ChangeSet` 自身には含まない。逆変換 (`invert`) には適用前のテキストが必要なので、`Document::apply` が **適用前の Snapshot から削除テキストを取り出して逆 ChangeSet を作る** (§7.2)。

### 6.2 演算

```rust
impl ChangeSet {
    pub fn identity(len: usize) -> ChangeSet;
    pub fn from_sorted_edits(base: &dyn TextRead, edits: impl Iterator<Item = Edit>) -> Result<ChangeSet, EditError>;  // EditError = Overlap | NotCharBoundary { pos } | OutOfRange。len_before = base.len_bytes()。重なりを切り詰めない (§4.5)
    pub fn compose(&self, next: &ChangeSet) -> Result<ChangeSet, ComposeError>;  // ComposeError = LenMismatch | SplitInsideChar (next の境界が、self が挿入した文字の途中)。self を適用後に next を適用 = 1 つの ChangeSet
    pub fn invert(&self, before: &Snapshot) -> ChangeSet;                       // 適用を取り消す ChangeSet。InsertText::Slice を作るため所有した Snapshot を受け取る (§6.5)
    pub fn map_pos(&self, p: Pos, bias: Bias) -> Pos;                           // 位置写像 (§6.4)
    pub fn map_range(&self, r: ByteRange) -> RangeMap;                          // Mapped(ByteRange) | Overlapped { mapped: Option<ByteRange>, kind } (§6.4)
    pub fn rebase(&self, other: &ChangeSet, policy: RebasePolicy) -> Result<ChangeSet, Conflict>;  // §18。v1.L (v0.1 の API には入れない)
    pub fn is_identity(&self) -> bool;
    pub fn changed_ranges(&self) -> impl Iterator<Item = (ByteRange /*before*/, ByteRange /*after*/)>;  // 購読者 (syntax 等) 向け
}
pub struct Edit { pub range: ByteRange /*削除*/, pub insert: Arc<str> }       // 置換。range.is_empty() なら純挿入
```

代数的性質 (プロパティテストの対象、§22。`rebase` の収束性は v1.L で追加):

| ID | 性質 |
|---|---|
| CS1 | `apply(apply(t, a), b) == apply(t, a.compose(b))` |
| CS2 | `apply(apply(t, a), a.invert(t)) == t` |
| CS3 | `a.invert(t).invert(apply(t, a)) == a` (正規形で比較) |
| CS4 | `map_pos` は単調非減少: `p <= q ⇒ map_pos(p) <= map_pos(q)` |
| CS5 | `identity.compose(a) == a == a.compose(identity)` |
| CS6 | 正規形: 隣接する同種 Op は結合され、`Retain(0)` / `Delete(0)` / 空 Insert は含まない。`Delete` と `Insert` が隣接する場合は `Delete` を先に置く |

**編集の端の改行の保護 (`eol_guard`)** (提案、OQ-194): §2.4 のとおり、編集の端で単独の `\r` と単独の `\n` が新たに隣接すると 2 つの区切りが 1 つの CRLF に合体する。混在ファイル (§8.4) では次のように利用者の操作だけで起き、「Enter を押したのに行が増えない」「行の中身を消したら空行ごと消えた」になる。そこで、eve-core の編集コマンド (Origin が `User` / `Paste` / `Command` のもの) は `from_sorted_edits` に渡す前に各 `Edit` を次の規則で直す。直した結果も同じ 1 Transaction に入る (Undo は直した後の内容を元に戻す)。

| 場面 (編集後の文書で見る) | 例 | 規則 |
|---|---|---|
| 挿入文字列が `\n` で始まり、挿入位置の直前 (削除で残った側) が単独の `\r` | CR の文書で `default_eol = LF` のとき、行頭 (前の行の CR の直後) で Enter | 挿入文字列の先頭の改行を **直前の改行と同じ種類** (`\r`) に置き換える (`"\nabc"` → `"\rabc"`)。結果は CR の区切りが 2 つで、行が 1 つ増える |
| 挿入文字列が `\r` で終わり、挿入位置の直後 (削除で残った側) が単独の `\n` | LF の文書で `default_eol = CR` のとき、行末で Enter | 挿入文字列の末尾の改行を **直後の改行と同じ種類** (`\n`) に置き換える |
| 削除 (または置換で挿入文字列が空) の結果、残った `\r` と `\n` が隣接する | `"x\rabc\ny"` で `abc` を削除 | 削除範囲を直後の `\n` まで広げ、その位置に `\r` を挿入する (直後の改行を **直前の改行と同じ種類** に置き換える。`"x\r\ry"`、空行が残る) |
| 挿入位置が CRLF の `\r` と `\n` の間 | カーソルはそこに置けない (§4.1) ので編集コマンドでは起きない | `from_sorted_edits` は受け付ける (char 境界)。編集コマンドは位置を書記素境界へ丸めてから `Edit` を作るので現れない |

- 規則は挿入・削除の **端の 1 文字** だけを見る。挿入文字列の内部の改行は §8.4 の貼り付けの正規化 (`normalize_eol`) の対象で、本規則は触れない。
- `Reload` / `Recovery` / `Ai` / `Lsp` / `Plugin` / `External` / `System` と Undo / Redo の Transaction には適用しない (内容をそのまま再現するため)。これらで合体・分離が起きても、§2.4 のとおり行の構造と `eol_stats` は内容どおりに数える。
- 代案: 規則を設けず合体を受け入れる (行の構造は内容どおり)。その場合も `eol_stats` の更新規則 (§6.5) は同じでよい。

### 6.3 Transaction

```rust
// eve-core
pub struct Transaction {
    pub changes: ChangeSet,
    pub selection_before: Option<Selection>,   // 発行 View の Selection (Undo 時の復元用)。他 View 由来や Reload では None
    pub selection_after: Option<Selection>,    // 適用後に発行 View へ設定する Selection。None なら changes で写像
    pub origin: Origin,
    pub is_undo: bool,                         // Undo / Redo による適用 (Origin は元のまま。下記)
    pub view: Option<ViewId>,                  // 発行 View
    pub timestamp: Tick,                       // Editor の「今」= Host が step で各メッセージの処理の前に Editor::tick で渡した Tick (02 §4.4 の「時刻の源」。コマンドの実行中に Clock を読み直さない)。Undo 結合の判定に使う
    pub undo_group: UndoGroupHint,             // Merge / Break / Auto (§7.3)
    pub meta: TransactionMeta,                 // 拡張メタデータ欄 (06 が provider / model / ハンク一覧等に使う。v0.1 は空)
}
pub enum Origin {
    User,                 // キー入力による直接編集 (insert_text / delete)
    Command(CommandId),   // その他の編集コマンド (インデント、行操作、置換、変換 …)
    Paste,                // ブラケットペースト / クリップボード貼り付け (FR-206)
    Reload,               // 外部変更の再読込 (§9.5)
    Ai { request: AiRequestId, proposal: ProposalId },   // 承認済み Proposal の適用 (§18)。02 §10.2.1 の不透明 ID
    Lsp,                  // LSP の workspace edit (v1.L)
    Plugin(PluginId),     // v1.X
    External(ClientId),   // 駆動 API (v0.2 以降)・MCP sidecar など外部クライアント (07 §13.1 Stage D)。v0.1 は予約のみ。監査・Undo 結合は Plugin と同規則
    Recovery,             // スワップ・緊急退避からの復旧適用 (§10)
    System,               // 利用者の選択を受けて core が組み立てる編集 (§8.6 E3 の選択肢 2「置換して保存」)。Undo 可、前後と結合しない (§7.3 R2)
}
```

- **Transaction は文書変更の唯一の単位** ([00 §6](00-overview.md#6-用語集-glossary))。Command ハンドラは Transaction を返し、自分で `Text` に触れない。
- `Origin` の用途: Undo の結合可否 (§7.3)、監査ログ、AI 提案の失効判定、変更通知の購読者が「自分の起こした変更か」を見分ける (スワップ・LSP の didChange)。**本列挙が `Origin` の正本** (02 / 00 は例示)。`Ai` / `Lsp` / `Plugin` / `External` は v0.1 では生成されないが列挙に予約し、`Transaction` には拡張メタデータ欄 (provider / model / ハンク一覧など、06 が利用) を持たせる。
- Undo / Redo で適用される Transaction は独立した Origin を持たず「元の Origin + `is_undo` フラグ」で表す (監査で元の主体が分かる)。

### 6.4 位置写像 (map) の規則

| 位置 p の状況 | `Bias::Before` | `Bias::After` |
|---|---|---|
| 挿入位置 == p | p のまま (挿入の前) | p + 挿入長 (挿入の後) |
| 削除範囲 `[s, e)` の内側 (`s < p < e`) | s | s |
| 削除範囲の端 (`p == s` または `p == e`) | s | s |
| 変更より前 | p | p |
| 変更より後 | p + (累積差分) | p + (累積差分) |
| 置換 (`Delete(k)` の直後に同じ位置で `Insert(x)`) の削除範囲の内側・端 | s | s + x の長さ (挿入の後) |

置換は「削除範囲の内側・端の点を `s` に潰し、そのうえで同じ位置 `s` の挿入に対して Bias に従う」と定める (行 1 と行 2〜3 の組合せ)。例: 選択 `[2, 5)` の上で `X` を入力すると、head = 5 は `Bias::After` で 3 (入力した文字の後) に写る。プロパティテスト (§22.1) は置換を含む ChangeSet で CS4 と本規則を検査する。

`MapPolicy` は Selection 全体に対する既定方針を決める: `Own` (自分の挿入は After)、`Other` (他者の挿入は Before)。

**`map_range` の戻り値** (06 X3 の要求): 範囲 `[s, e)` に対して ChangeSet の変更が (a) 範囲と交差しない → `Mapped(新範囲)`、(b) 範囲の内側で挿入・削除が起きた、範囲の境界をまたぐ、または **範囲の端 (`s` / `e`) に接する挿入** がある → `Overlapped { mapped: Option<新範囲>, kind: Inside | Straddle | Touch }`。Decoration / 検索結果は `Overlapped` のうち `Touch` を「写像のみ (位置をずらす)」、`Inside` / `Straddle` を無効化と解釈し、AI Proposal (§18 / 06 §7.4) は **`Touch` も含めて失効** と解釈する (保守的: 誤適用より失効の方が安全、v0.1)。呼び出し側が意味を決められるよう、`map_range` 自身は分類だけを返す。空の範囲 (位置) が削除・置換の内側にある場合は、位置が消えるので `Straddle` とする (端に接する削除は交差とみなさない)。`compose` で合成した ChangeSet の `map_pos` は、元の ChangeSet を順に写像した結果と一致するとは限らない (合成で「残す部分」の構造が失われるため)ので、Selection の写像には合成したものを使わず、元の ChangeSet を順に使う。`OverlapKind::{Inside, Straddle, Touch}` の 3 変種は残し (確定、[OQ-36](10-decisions.md#oq-36-ai-の安全方針-3-点))、`OverlapKind` は `#[non_exhaustive]` とする (分類を後から足せるように)。AI ハンクの失効の方針は未決 (OQ-36 (c)。推奨は `Touch` も失効)。

### 6.5 Document::apply の手順

[02 §5.2](02-architecture.md#52-変更の唯一の経路) を具体化する。

```
Document::apply(tx) -> Result<AppliedTx, ApplyError>
  1. 前提検査: tx.changes.len_before == text.len_bytes()  (違えば ApplyError::StaleBase → 呼び出し側で写像 (v1.L) or 破棄)
     read_only (ReadOnlyReasons) が空でない、または load_state != Ready なら Rejected (Origin::Recovery / Reload は例外)
     tx.changes.is_identity() なら手順 2 以降を行わない: 適用も履歴への記録も封印もせず、revision も進めず、通知も出さない。
     発行 View の selection だけを tx.selection_after (あれば。normalized() を通す) に更新し、goal_dcol は手順 7 と同じ規則で扱う。戻り値は AppliedTx::Identity
  2. before = text.snapshot()
  3. inverse = tx.changes.invert(&before)          ← 削除テキストをここで確保
  4. text.apply(&tx.changes); format.eol_stats を更新 (下記「eol_stats の更新」)
  5. revision += 1
  6. tx.is_undo なら history は記録せず、Undo / Redo のスタック間の移動だけを行い封印する (§7.1 / §7.3)。
     それ以外は history.record(tx, inverse) (結合規則は §7.3。inverse に 64 KiB 以上の Slice があれば実体化ジョブを投入、§6.5 下)
  7. 全 View: selection = (発行 View なら tx.selection_after または Own 写像) / (他 View は Other 写像); scroll 写像;
     goal_dcol は発行 View ではクリア、他 View では維持 (§5.2)
  8. Decoration / 検索結果 / 診断 / 括弧対応キャッシュ: map_range で写像、変更範囲と交差するものは無効化
  9. レイアウトキャッシュ: 変更行以降を無効化 (折り返し ON) / 変更行のみ無効化 (OFF)
 10. 購読者へ DocumentChanged { doc, old_revision, revision, changes: Arc<ChangeSet>, origin } を内部通知 (syntax / lsp / swap / ai / Proposal 写像 §18) し、
     外向きの EditorEvent::DocumentChanged { doc, revision, origin_kind } (02 §10.1、06 §17 X25) を drain_events() のキューへ積む
     (origin_kind は §6.3 の Origin の種別。Undo / Redo は Origin の変種ではなく tx.is_undo のフラグで表し、origin_kind は元の Origin の種別のまま)。
     v1.L 以降は edits: Option<EditSummary> を追加して tx.changes から作る: 編集箇所 ≤ 64 なら Exact(Vec<EditDelta { old: 旧 Revision 上の ByteRange, new_len }>) (昇順・重なりなし)、
     65 以上なら Coarse { first_old, last_old_end, len_delta }、Origin::Reload / Recovery (全文置換) は Replaced。本文は運ばない (必要なら Query)
 11. dirty 判定を更新 (§7.4)
 12. 退避用の公開物が変わった文書として記録する。Host は Editor の状態を変える呼出し (handle_input / on_job_result / tick / execute 等) の直後に
     take_snapshot_updates() で変わった文書を受け取り、EmergencySnapshot のスロットを差し替える (dirty でなくなった文書はスロットを空にする。§10.1 / 02 §11.2.1)。core は Snapshot を提供するだけ
```

- **恒等の Transaction** (手順 1): 文書端での Backspace のように変更の無い Transaction は、文書・Revision・Undo 履歴 (結合の状態を含む) ・dirty・購読者のどれにも影響しない。コマンドハンドラは変更が無いと分かれば Transaction を作らないのが基本だが、作っても上の規則で無害になる。
- **eol_stats の更新** (手順 4。INV-D5): 各編集 (旧座標の削除範囲 `[s, e)` と、同じ位置の挿入文字列 x) について、旧文書の窓 `[s-1, e+1)` と新文書の窓 `[s-1, s+|x|+1)` (どちらも文書の範囲に切り詰め、窓の端が CRLF の `\r` と `\n` の間に当たるときは 1 バイト外へ広げる) にある区切りを種類別に数え、旧の窓の分を引いて新の窓の分を足す。窓の端の前後 1 バイトを含めるのは、§2.4 の合体・分離 (端の `\r` と `\n` の組み替え) を数え漏らさないため。区切りの種類は窓の外の隣のバイトも見て決める。コストは「削除量 + 挿入量」に比例し、文書長に依存しない (100 MB の中央への 1 文字挿入は数バイトの走査)。
  - **巨大な削除** (削除量 64 KiB 以上): 削除テキストを区切りの種類ごとに走査する代わりに、旧の窓の区切りの総数 `T` を行数差 (`byte_to_line` の差、O(log n)) で求め、窓の中の `\r` と `\n` のバイト数 `C_r` / `C_n` を SIMD のバイト計数 (数 GB/s 以上) で数えて、`crlf = C_r + C_n − T`、`cr = C_r − crlf`、`lf = C_n − crlf` とする。100 MB の全選択削除でも数十 ms 以内の見込み (09 の BM で計測し、超える場合は本節の下の実体化ジョブが複写の際に数える方式に替える)。編集後の文書が 64 KiB 以下になる削除 (全選択削除など) は、編集後の全文を数え直す方が速いのでそうする。
- 手順 1 の `StaleBase` は、AI Proposal や LSP 編集 (v1.L) が古い Revision に基づいて届いた場合に起きる。パニックではなく **エラーで返し**、呼び出し側が §18 の写像規則 (v1.L) で処理する。v0.1 ではこの経路の呼び出し側が無く、届いた結果は破棄する (§17.2)。
- **`EditSummary` の生成 (手順 10、v1.L)**: v0.1 は型 (`EditSummary` / `EditDelta`) の名前を予約するだけで生成しない (ADR-0039)。生成は最初の利用者 (LSP 増分同期・ローカル LLM) と同時に入れ、`DocumentChanged` への `Option` フィールドの追加は C2 の「追加」に当たる。生成の規則: ChangeSet の各 `Op` を旧 Revision の座標で走査し、連続する `Delete` + `Insert` は 1 つの `EditDelta` にまとめる。コストは編集箇所の数に比例し、文書長に依存しない。購読者 (LSP の増分同期、ローカル LLM のプロンプトキャッシュ境界の判定と補完要求の破棄 (06 §6.7 / §5.3)、駆動 API、swap) は `first_old` より前が変わっていないことを前提にできる。型 (`EditSummary` / `EditDelta`) は eve-view (境界型)、生成は eve-core。
- **`Query` の決定性 (06 §17 X26、v0.1 で保証)**: `Editor::query(Query::DocumentText { doc, range })` は **同じ Revision・同じ範囲なら同じバイト列** を返す (改行は文書のまま非正規化、不正バイトは §8.3 の PUA エスケープのまま、末尾改行の有無も保持。時刻・パス・ID を混ぜない)。これが 06 §6.7 の「先頭を固定したプロンプト」の前提になる。性質テストは 09 §3.2 (eve-core `Query`)。
- 手順 3 の逆変換生成コストは削除量に比例する。「全選択削除 → 100 MB の削除テキスト」のような場合に `Arc<str>` へコピーするとメインスレッドで 100 MB を複写するので、`Op::Insert` の文字列表現は次の 3 つを許す `InsertText` 列挙体とする。

| 変種 | 中身 | 使う場面 | 会計 (§7.5) |
|---|---|---|---|
| `Str(Arc<str>)` | 文字列そのもの | 通常の挿入、64 KiB 未満の削除の逆変換 (コピーする) | 文字列のバイト数 |
| `Slice { snapshot, range }` | 編集前の Snapshot と範囲 | 64 KiB 以上の削除の逆変換の **記録直後だけ** (実体化ジョブの完了まで) | 削除バイト数 + 8 KiB |
| `Rope(Text)` | 範囲だけを持つ独立の rope (eve-text の自前の型 `Text` で包む。ropey の型は公開しない、§2.2) | 実体化ジョブの完了後。範囲が Snapshot 全体 (全選択削除・Reload / Recovery の全置換) なら記録時から旧 rope をそのまま入れる (コピーなし) | 削除バイト数 + 8 KiB |

- **Slice を長く持たない理由と実体化** (02 §14.1 MR1): ropey 1.x の `RopeSlice` は元の rope を借用し部分木だけを所有できないので (この前提は P0 スパイク S-02 で確認する、§2.3)、`Slice` は **編集前の文書全体** の Snapshot を保持する。その後の編集で現在の rope が経路を作り直すたびに旧 Snapshot だけが参照するノードが増え、最大で当時の文書全体が会計外で残る。そこで手順 6 で `Slice` を記録したら、直ちに `bg` ワーカーへ **実体化ジョブ** (`JobKind::Materialize { snapshot, key: SliceKey, range }` (02 §9.3。文書は `JobRequest.doc`)。`SliceKey` は `Slice` を作るときに振る文書内の通番) を投げ、ジョブは範囲だけを複写した独立の `Text` を作って `JobResult::Materialized { job, key, rope }` で返す。メインは結果を受け取ったとき、その `SliceKey` の `Slice` がまだ履歴 (undo / redo のいずれかのノードの ChangeSet。結合で別のノードに合成されていてもよい) にあれば `InsertText::Rope` に置き換え (旧 Snapshot を解放)、無ければ捨てる。ノードの `NodeId` は結合で振り直されるので、採否の鍵には使わない。実体化ジョブは検索と同じ `bg` で動くが、同種の後続ジョブにキャンセルされない (各ノード独立)。複写のコストは削除量に比例し (100 MB で数十 ms)、メインスレッドでは行わない。
- 64 KiB 未満の削除を `Str` にコピーするのは、小さな削除のたびに Snapshot (と、その版にしかない経路 4〜8 KiB) を持つと 1 万ステップで数十 MB が会計外に残るため。結果として、会計に現れないメモリは「実体化ジョブの実行中の Snapshot」だけで、時間的に有界になる。性質テスト「ランダムな 1 万回の編集 (64 KiB 以上の削除を一定割合で含む) の後、実体化ジョブがすべて完了した時点で RSS 増分 ≤ Undo 会計の 1.2 倍」を v0.1 の L1 に置く (09 §3.2)。

---

## 7. Undo / Redo

### 7.1 線形か木か

Undo 履歴は **線形スタック (undo / redo の 2 本)** とする (v0.1〜。OQ-114 で確定 (ユーザー回答))。Undo 後に編集すると Redo 側は破棄する (他エディタの既定と同じ)。履歴は永続化しない (OQ-18) ため、内部表現は `eve-core` の内部で閉じる。木 (全分岐を保持、Vim の undotree。FR-213、Could、v1.L) は同じ API のまま内部を入れ替えて実現する。

```rust
// eve-core。API (undo / redo / seal / saved / is_dirty / record) を crate 内に閉じ、内部表現 (線形 / 木) は差し替え可能にする
pub struct NodeId(u64);              // 履歴の位置の識別子。文書ごとに単調増加し、破棄しても再利用しない (02 §5.1)
pub struct UndoHistory {
    undo: VecDeque<UndoNode>,        // 適用済み。末尾が直前の操作。予算超過では先頭 (底) から破棄
    redo: Vec<UndoNode>,             // Undo で戻した分。新しい編集で破棄
    base: NodeId,                    // undo が空のときの現在位置: 根 (読込・開き直しの時点) または最後に破棄した底のノード
    next_id: u64,                    // 次に振る通番
    saved: Option<NodeId>,           // 最後に保存 (または読込) した時点の位置 (§7.4)。None = 保存時点に戻れない
    budget: UndoBudget,              // §7.5 (文書ごと)。プロセス全体の上限は EditorState 側で管理
}
pub struct UndoNode {
    id: NodeId,                      // このノードを適用した直後の位置。結合で内容が変わるたびに新しい通番に振り直す
    redo: ChangeSet,                 // 適用
    undo: ChangeSet,                 // 取り消し (削除テキストを含む)
    selection_before: Option<Selection>, selection_after: Option<Selection>,
    format_change: Option<FormatChange>, // Origin::Reload: 再読込前後の TextFormat と内容由来の読み取り専用の理由 (§9.5)。改行コードの変換 (file.set_line_ending): 前後の default_eol だけ (§8.4)
    origin_class: OriginClass, last_edit_at: Tick, group_sealed: bool,
    bytes: usize,                    // メモリ会計用 (§7.5)
}
// 現在位置 current() = undo.back().map(|n| n.id).unwrap_or(base)。dirty = (saved != Some(current()))
```

- **位置の表し方**: 履歴上の位置は **ノードの通番 (`NodeId`)** で表し、スタック上の深さでは表さない。深さで表すと、予算超過で底のノードを捨てたときに残りの位置が全部ずれ、保存点と現在位置が偶然一致して「未変更」と誤判定する (FR-107 の確認が出ずに未保存の内容を失う)。通番なら底を捨てても他のノードの位置は変わらない。
- **Undo** = `undo` スタック末尾の `undo` ChangeSet を Transaction (元の Origin + `is_undo`) として適用し、ノードを `redo` スタックへ移す。Selection は `selection_before` を復元 (他 View は写像)。ノードが `format_change` を持てば再読込前の `TextFormat` と読み取り専用の理由も戻す。適用後に封印する (直後の入力を Undo 前のノードに結合しない、§7.3)。
- **Redo** = `redo` スタック末尾の `redo` を適用し、ノードを `undo` スタックへ戻す (`format_change` があれば再読込後の状態にする)。適用後に封印する。
- Undo / Redo 自身は履歴に新ノードを作らない (履歴上の移動。§6.5 手順 6 は `is_undo` のとき `record` を呼ばない)。`revision` は進む (Snapshot の識別のため)。
- **新しい編集** (`record`) は `redo` スタックを破棄し、新ノードに新しい通番を振る (結合した場合は、結合先のノードに新しい通番を振り直す。内容が変わったので以前の位置とは別の位置になる)。破棄した redo のノードを `saved` が指していれば `saved = None` (その位置には二度と戻れない)。
- **底の破棄** (§7.5): ノード n を捨てたら `base = n.id`。`saved` は補正しない。`saved` が `base` より古い通番を指していれば `None` にする (その位置には戻れない)。`saved == base` はそのまま有効で、すべて Undo すれば保存時点に戻る。
- 木構造 (FR-213、v1.L) に移行しても `NodeId` の意味 (位置の通番) と `saved: Option<NodeId>` は変わらない (02 §5.1)。

### 7.2 Undo で元に戻るもの・戻らないもの

| 対象 | Undo で戻る | 根拠 |
|---|---|---|
| 文書テキスト | はい | — |
| 発行 View の Selection | はい (`selection_before`) | 入力の文脈を取り戻す |
| 他 View の Selection / スクロール | 写像のみ | 他 View のユーザー操作を巻き戻さない |
| 文字コード・改行コード設定の変更 | **いいえ** (メタデータ)。ただし「改行コード変換」(`file.set_line_ending`、§8.4) はテキスト変更なので戻り、`default_eol` も変換前に戻る (`UndoNode.format_change`) | メタデータ変更はステータスバーで見え、再設定が容易。変換は本文と新しく挿入する改行を一緒に戻さないと、Undo の後の Enter が変換後の改行になり混在を作る |
| 外部変更の再読込 (Reload。v0.1 は明示の `file.reload`、フォーカス復帰時の確認は v0.2) | **はい**。本文と一緒に、再読込前の `TextFormat` と、再読込で付け直した読み取り専用の理由 (どの理由を付け直すかは §8.8 の「読み取り専用の理由の差し替え」) も戻る (`UndoNode.format_change`)。ただし **再読込の後に読み取り専用の文書では Undo できない** (`edit.undo` の when が `!doc.read_only`、07 §11.6。提案、OQ-216) | 「再読込で自分の編集が消えた」を取り消せる (§9.5)。本文だけを戻すと、不正バイトを含む本文が編集可能になる等の不整合が起きる |
| 文字コードを指定して開き直す (FR-111) | **いいえ** (Undo できない操作。履歴を消す。確定 (ユーザー回答)、§8.8) | 開き直しは dirty でないときだけ可なので未保存の内容を失わない。本文・文字コード・読み取り専用・保存点が食い違う状態を作らない |
| 保存 | 該当なし (履歴に境界を作るだけ) | FR-204 |

### 7.3 結合 (グループ化) 規則 (FR-203)

連続する Transaction を 1 つの Undo 単位にまとめる。`Document::apply` 手順 6 で `undo` スタック末尾のノード (以下「直前のノード」) に **結合** するか **新ノード** を作るかを判定する。

**判定の順序**: 新しい Transaction `tx` について、下表を **上から順に** 調べ、最初に当たった行の結果を使う (後の行は見ない)。「封印」はノードの `group_sealed` を立てることで、封印されたノードには以後何も結合しない。FR-203 AC-1 / AC-2 のプロパティテストの参照モデル (§22.1) はこの表をそのまま上から評価する関数として書く。

| 順 | 条件 | 結果 | 理由 |
|---|---|---|---|
| — | (前提) `tx.changes` が恒等 | 判定しない。§6.5 手順 1 で記録も封印もせずに終える (R1〜R9 のどれにも入らない) | 何も変えない操作で Undo の単位や結合の状態を変えない |
| R1 | `tx.undo_group == UndoGroupHint::Merge`、かつ直前のノードがあり、封印されていない | 直前のノードに結合 (R2 以降を見ない)。直前のノードが無い、または封印済みなら R1 には当たらず R2 以降で判定する | 1 つの操作が複数の Transaction に分かれる場合に、それらを 1 ノードにする (ハンドラが指定。§7.6) |
| R2 | `tx.origin` が `User` 以外 (`Paste` / `Command` / `Ai` / `Reload` / `Lsp` / `Plugin` / `External` / `Recovery` / `System`)、または `tx.undo_group == UndoGroupHint::Break` | 新ノード。直前のノードと新ノードの両方を封印 | 1 操作 = 1 Undo (FR-206, FR-403, FR-903) |
| R3 | 直前のノードが無い (`undo` が空)、または封印済み | 新ノード | 封印の契機: カーソル移動 (Motion コマンドの実行時に `UndoHistory::seal()`)、Undo / Redo の適用後 (Undo 直後の入力を、戻した位置の直前のノードに結合しない)、保存の開始 (§9.3 手順 3。Undo で保存時点にちょうど戻れる)、R2、R6 |
| R4 | `tx.timestamp` と直前のノードの `last_edit_at` の差 (`Tick` 同士の差は `Millis`、02 §4.4) が `editor.undo.group_timeout_ms` (07 §5.1。既定 **1000 ms**) を **超える** (ちょうど 1000 ms は R5 以降へ進む。01 FR-203 AC-1 の「1 s 以内」) | 新ノード | 時間による区切り。設定キー名と既定値の正本は 07 |
| R5 | 編集の種類が直前のノードと異なる (挿入 → 削除、削除 → 挿入。種類は各 Range の変更が純挿入か純削除かで決め、置換 (選択範囲への入力) は挿入として扱う) | 新ノード | 慣習 (VS Code / Sublime) |
| R6 | 挿入で、各 Range の挿入文字列が **改行を含む** 場合: (a) 挿入文字列が **改行 1 つだけ** (`default_eol` の LF / CRLF / CR のどれか 1 つで、他の文字を含まない) なら、R7 の位置の条件を満たせば直前のノードに結合し、**結合した後で封印** する (位置の条件を満たさなければ新ノードにして封印)。(b) 改行と他の文字を含む (自動インデントの空白付きの改行 (§15.3)、Tab の展開を伴う改行など) なら、**結合せず単独の新ノード** にして封印する | (a) 結合して封印 / (b) 新ノードにして封印 | 行単位で戻る感覚 (FR-202)。改行の直後の入力は次の行の新ノードになる |
| R7 | 挿入で改行を含まず、各 Range について「新しい挿入位置 == 直前のノードの挿入の末尾」、かつ結合後の挿入長が 1 ノードあたり 4 KiB 以下 | 直前のノードに結合 | 連続タイピング。4 KiB の上限は長い自動入力で巨大ノードになるのを防ぐ |
| R8 | 削除で、各 Range について直前のノードの削除と隣接する (Backspace の連打: 新しい削除の末尾 == 直前の削除の先頭、Delete の連打: 新しい削除の先頭 == 直前の削除の先頭) | 直前のノードに結合 | 連続削除 |
| R9 | 上のどれにも当たらない | 新ノード | — |

- **改行の後の封印は行によらない**: 改行を含む挿入を記録したノードは、R1〜R6 のどの行で結果が決まっても記録の後で封印する (例: 削除の直後の改行は R5 で新ノードになり、そのノードを封印する)。
- **空白の入力** は R7 で結合する (単語境界で封印しない、既定)。封印する動作は v0.2 で設定として公開する (07 §5.13 `editor.undo.break_on_word`)。v0.1 は既定値のみ。
- **例** (参照モデルの確認用): `a` `b` `c` の入力 (各 1 s 以内) → 1 ノード。`a` `b` → 1.5 s 待つ → `c` → 2 ノード (`ab` と `c`)。`a` `b` `Enter` (直前の行に字下げが無く、改行 1 つだけの挿入) `c` → `ab⏎` の 1 ノード (封印) と `c` の 1 ノード。字下げのある行で `Enter` (改行 + 空白の挿入) → 直前の入力とは別の 1 ノード (封印) で、続く入力はさらに別のノード。

結合時は `redo = redo.compose(new)`, `undo = new_inverse.compose(undo)`、`selection_after` は更新、`selection_before` は維持し、ノードの `id` を新しい通番に振り直す (§7.1)。文字入力・Backspace / Delete・単語単位の削除 (§5.3)・改行 (FR-202)・Tab の Transaction の Origin は `User` とする (R2 の `Command` は、これら以外の編集コマンドに当たる)。

### 7.4 保存時点と「未変更」判定 (FR-204)

- `saved` は最後に保存 (または読込) した **履歴上の位置** (§7.1 の `NodeId`。02 §5.1 の `history.saved`)。`dirty = (saved != Some(現在位置))`。
- `Revision` の比較では判定できない (Undo で保存時点の内容に戻っても Revision は進む) ため、**履歴上の位置の同一性** で判定する。位置は通番なので、予算による底の破棄・redo の破棄・結合のどれが起きても、保存時点と別の内容の位置が保存点と一致することはない。
- 保存はジョブで行い、保存に使った Snapshot の位置 (`NodeId`) を `saved` にする。保存の開始時に封印する (§9.3 手順 3) ので、保存中に入力しても新しいノードになり、完了時に `saved` が過去の位置を指して dirty のまま正しく表示される。**完了時に、保存に使った位置の `NodeId` が `base`・`undo`・`redo` のどこにも無ければ** (保存中に Undo してから新しい編集をして redo ごと捨てた、予算の破棄で底より古くなった) **`saved = None`** とする (INV-H3。戻れない位置を保存点にしない)。保存したディスクの内容は Snapshot のとおりなので、`canonical` などの保存の結果の反映 (§9.3) はこれと独立に行う。
- 保存点に戻れなくなった場合 (redo の破棄で保存時点のノードが消えた、予算の破棄で `base` より古くなった) は `saved = None` とし、dirty = true として扱う (安全側)。
- 読込完了・新規文書・開き直し (§8.8) では根を作り `saved = Some(根)`。外部変更の再読込 (§9.5) の完了時は `saved = Some(再読込ノードの位置)`。
- 外部変更を検知した場合 (§9.5) は dirty と独立に `external_modified` フラグを立てる。

### 7.5 メモリ上限 (NFR-203)

- 文書ごとに `UndoBudget { max_bytes: 64 MiB, max_nodes: 10_000 }` (`editor.undo.max_bytes` / `editor.undo.max_steps`。v0.1 は定数、公開は v0.2、07 §5.1)。**どちらか一方を超えたら** 古い側から破棄する。
- **プロセス全体の上限 256 MiB** (`editor.undo.total_max_bytes`、07 §5.1) を加える (**v0.2**、複数文書と同時。v0.1 は単一文書なので文書あたりの上限が実質の上限になる。会計は文書ごとに持ち、合計を出せる形で実装しておく)。文書あたりの上限だけでは、20 文書を開くと最悪 1.28 GB になる。超えたら **履歴の最も大きい文書から古い順に** 破棄する。「直前の 1 操作は必ず保持する」規則 (下記) は全体上限より優先する。
- ノードのコスト = `redo` と `undo` の `Insert` 文字列バイト数 + 固定オーバーヘッド (約 128 B) + Selection のサイズ。`InsertText::Slice` / `InsertText::Rope` (削除 64 KiB 以上のみ、§6.5) は「削除バイト数 + 経路オーバーヘッド 8 KiB」で計上する。小さな削除は `Arc<str>` にコピーし、大きな削除は実体化ジョブで範囲だけの rope に置き換える (§6.5) ので、Snapshot の保持で会計に現れないメモリが増え続けることはない。
- 破棄は古い方 (`undo` スタックの底) からで、**最低 1 ノードは残す**。破棄したら `base` を更新し、`saved` の扱いは §7.1 の規則に従う (位置は通番なので補正は要らない)。破棄時にメッセージで 1 回通知する (「Undo 履歴が上限に達したため古い履歴を破棄しました」)。プロセス全体の上限による破棄は、利用者が編集していない文書の履歴も削るが、その文書の dirty 表示は変わらない (§7.1)。
- 大きな単一操作 (100 MB の全選択削除) はそれ自体が上限を超えるが、**直前の 1 操作は必ず Undo 可能** とする (上限より優先)。超過分は次の操作で解消される。
- **Reload / Recovery の全置換** (§9.5 / §10.3) は旧全文を `InsertText::Rope` (旧 rope そのもの。コピーなし) として保持するため、100 MB 文書では一時的にメモリが約 2 倍になり NFR-202 の目標を超え得る。これは「直前 1 操作は予算より優先」の例外が NFR-202 の例外にもなることを意味する。**Reload / Recovery は常に Undo 可** (「再読込で自分の編集が消えた」を取り消せることを優先。明示の再読込 `file.reload` は v0.1 から、フォーカス復帰時の確認と復旧は v0.2 から) とし、このメモリ増は 01 NFR-202 の例外として扱う。旧 rope は Reload のノードが破棄されるまで残る。予算 (文書あたり 64 MiB) を超える大きさなら、次の編集でそのノードが「直前の 1 操作」でなくなった時点で予算の規則により破棄され、解放される。
- OQ-18 (永続化) は当面行わないため、履歴は Document を閉じると消える。

### 7.6 Undo と複数 View・複数カーソル

- Undo の単位は Document 単位 (View 単位ではない)。View A で編集し View B で Undo すると、A の編集が戻り、B の Selection は `selection_before` (A の View のもの) **ではなく** 写像のみ。発行 View が現在もあれば、その Selection を復元する。
- 複数カーソルの 1 キーストロークは全 Range の変更を合成した **1 つの Transaction** (§4.5 の 1) なので、それだけで 1 ノードになり、`UndoGroupHint::Merge` は要らない。連続入力の結合 (R5 / R7 / R8) は各 Range について条件を調べ、全 Range で満たすときだけ結合する。`Merge` は、1 つの操作をハンドラが複数の Transaction に分けて適用する場合 (前の Transaction の結果を見て次を作る後処理など) に、2 つ目以降に付ける。

---

## 8. 文字コード・BOM・改行コード

### 8.1 Document のメタデータ

```rust
// フィールド名・型の正本は 02 §5.1。本書が加えるフィールドは invalid_bytes と pua_escape の 2 つ
pub struct TextFormat {
    encoding: Encoding,              // encoding_rs の静的 Encoding + Utf16Le/Utf16Be (自前)
    has_bom: bool,                   // BOM の種類は encoding から決まる (UTF-8 / UTF-16LE / UTF-16BE)。保存時に同じものを書く
    default_eol: LineEnding,         // 新規に挿入する改行 (LF / CRLF / CR)。読込時は多数派
    eol_stats: EolStats { lf, crlf, cr },  // 読込時に数え、編集で増減 (更新規則は §6.5「eol_stats の更新」。ステータス表示「混在」の判定)
    final_newline: bool,             // 導出値: 最終行が空か (§2.4)。保存時にそのまま書く (FR-114)
    roundtrip: RoundtripInfo,        // §8.7: 再エンコードで元バイト列に戻らない箇所。v0.1 は Summary、v0.3 で Spans (保持方式)
    // 以下は本書が 02 §5.1 に加えるもの
    invalid_bytes: u32,              // §8.3 の保持マーカー数。0 でなければ読み取り専用既定 (NFR-304)
    pua_escape: PuaEscapeMode,       // Enabled / DisabledCollision / DisabledStateful (§8.3)
}
pub enum RoundtripInfo {
    Summary { count: u32, first_positions: Vec<Pos> /* 最大 100 */ },   // v0.1: 検出して確認に使うだけ (メモリは定数)
    Spans(Vec<LossySpan>),                                            // v0.3: 保持方式 (隣接するものは結合。上限は §8.7)
}
pub struct LossySpan { range: ByteRange /* テキスト座標。編集で写像 */, original_bytes: Arc<[u8]>, reason: LossyReason /* DuplicateEncoding | DecodeOnly | SilentMapping | StatefulEscape */ }
```

> `default_eol` は「文書全体の改行コード」ではなく「新規挿入用の既定」である (02 §5.1)。
>
> `Encoding` は encoding_rs の静的 `Encoding` への参照 + 自前の `Utf16Le` / `Utf16Be` を包む列挙で、`is_stateful()` (ISO-2022-JP のみ `true`) を持つ。状態付き符号化では §8.3 の PUA エスケープを元のバイトに戻して書くことを無効にし (`DisabledStateful`。不正バイトの保持と表示の方式、その文書を ISO-2022-JP で保存しないことは §8.3 の「状態付き符号化」の行、確定 (ユーザー回答)、OQ-207)、保存はエンコーダの状態をチャンクの境界をまたいで引き継ぐ (行末の扱いは §8.6 E6)。v0.1 で読み書きする文字コードは UTF-8 (BOM 有無)、UTF-16LE/BE (BOM 付き)、Shift_JIS (CP932)、EUC-JP、**ISO-2022-JP** (OQ-03、ユーザー回答で確定)。BOM の無い UTF-16 は §8.2 手順 3 の推定で開けるが、FR-108 の保証 (§8.7 の区分 A) には含めない (判定の誤りは FR-111 で回復する)。実装はすべて `eve-encoding` (02 §4.1)。

### 8.2 判定アルゴリズム (FR-108 / 109 / 115)

読込ジョブが **先頭 `sniff_len` バイト (既定 64 KiB、ファイルが小さければ全体)** を使って判定する。順序は [ADR-0009](10-decisions.md#adr-0009-内部文字コード-utf-8-と入出力時変換) に従い、**UTF-16 BOM 判定を NUL 判定より先に行う** (UTF-16 は NUL バイトを大量に含むため。FR-115)。

```
1. BOM: 長い BOM から先に調べる。FF FE 00 00 → UTF-32LE の BOM / 00 00 FE FF → UTF-32BE の BOM (UTF-16LE の BOM `FF FE` より先に判定する)。
   UTF-32 は v0.1 の対象外 (§8.9) なので、本文を読まずに JobResult::NeedsConfirm { ask: ConfirmAsk::UnsupportedBom { kind: Utf32Le | Utf32Be } } で確認を求める
   (選択肢は手順 4 のバイナリと同じ「読み取り専用で開く / 文字コードを指定して開く / 中止」。文言で「UTF-32 の BOM があり、この文字コードには対応していない」と案内する。`ui.confirm.open.utf32`、04 §15.2)
   EF BB BF → UTF-8+BOM / FF FE → UTF-16LE+BOM / FE FF → UTF-16BE+BOM  → 確定
2. 明示指定 (`OpenOptions.forced_encoding`: FR-111 の「指定して開く / 開き直す」、`--encoding`、外部変更の再読込 (§9.5)) があれば → その文字コードで確定。
   明示指定は **手順 1 にも優先する** (手順 1 より先に評価する): 先頭のバイト列が指定した文字コードの BOM と一致すれば BOM として読み飛ばし (`has_bom = true`)、
   一致しなければ (別の文字コードの BOM・UTF-32 の BOM を含む) BOM とみなさず本文のバイトとして復号する (`has_bom = false`。UTF-32 の BOM の確認 (手順 1) も出さない)
3. UTF-16 無 BOM 判定: 偶数長で、偶数または奇数バイト位置の NUL 比率が 30% 超かつ他方が 1% 未満 → UTF-16LE/BE (信頼度 中)。
   ※ 日本語 UTF-16 は NUL が少ないため、ASCII 主体のときのみ効く。chardetng は UTF-16 を判定しないため自前
4. バイナリ判定 (FR-115): 上記以外で NUL が 1 つ以上、または C0 制御文字 (TAB/LF/CR/FF/ESC 除く) が 5% 超 → Binary → UI へ「読み取り専用で開く / 文字コードを指定して開く (FR-111) / 中止」。
   ※ BOM なしの日本語 UTF-16 は下位バイトに C0 が約 1 割現れ、手順 3 を通らずここでバイナリ扱いになり得る。そのためバイナリ判定のダイアログには **「文字コードを指定して開く」を常に表示** する
4'. **ISO-2022-JP 判定** (02 §13.4 で確定): sniff 範囲が 7 bit (0x80 以上を含まない) で、かつ **文字集合を切り替える** ESC シーケンス `ESC $ @` / `ESC $ B` / `ESC ( I` のいずれかを含み、かつ sniff 範囲全体が encoding_rs の ISO-2022-JP デコーダで **Malformed なしに復号できる** 場合に限り → ISO-2022-JP (信頼度 中) として確定 (chardetng に投入せず自前で判定する)。**sniff 範囲がファイルの末尾に達していない** (ファイルが 64 KiB より大きい) ときは、復号を `last = false` で行い、範囲の末尾で未完のシーケンス (2 バイト文字の 1 バイト目、ESC 列の途中) が残っても Malformed とみなさない (手順 5 の「末尾の切れた多バイト列は許容」と同じ扱い。切れ目の位置で判定結果が変わらないようにする)。ファイル全体が sniff 範囲に収まるときは `last = true` で復号し、末尾の未完のシーケンスは Malformed とする。ISO-2022-JP は 7 bit なので手順 6 の「ASCII のみ」に吸収され、§8.2.1 の再判定 (不正 UTF-8 バイト契機) も発火しないため、**この段がないと判定できない**。`ESC ( J` (JIS X 0201 ローマ字) だけを含むファイルは ASCII と区別する価値がない (`ESC ( J` 区間の `\` が `¥` になるだけ) ので条件に含めない。ESC を含むが上記シーケンスを含まないファイル (ANSI エスケープ `ESC [` 入りのログ等) と、上記シーケンスを含むが復号できないファイル (JIS X 0212 の `ESC $ ( D`、ISO-2022-JP-2 の `ESC $ A` / `ESC . A`、WHATWG デコーダが誤りとする冗長なエスケープ、8 bit バイトの混在) は確定せず手順 5 以降へ。**7 bit のファイル (8 bit バイトの混在以外) は手順 5 も通らず手順 6 で UTF-8 (ASCII、信頼度 低) に確定し、手順 7 に届かない**: ESC を含む ASCII の文書として開き (ESC は FR-507 の `<ESC>` で見える)、ASCII として往復するのでバイト一致は保たれる。ISO-2022-JP として読みたいときは FR-111 で指定して開き直す (復号できない箇所は不正バイト、§8.3 の状態付き符号化の規則で読み取り専用)。手順 7 に届くのは 8 bit バイトが混在するものだけで、chardetng が ISO-2022-JP と推定した場合は ISO-2022-JP として開いて復号できない箇所を不正バイト (§8.3) とし、状態付き符号化のため **読み取り専用** で開く (FR-111 で別の文字コードとして開き直せる。寛容な自前デコーダの要否は OQ-138)
5. UTF-8 妥当性: sniff 範囲が妥当な UTF-8 (末尾の切れた多バイト列は許容) で ASCII 以外を含む → UTF-8 (信頼度 高)
6. ASCII のみ → UTF-8 と仮定 (信頼度 低、§8.2.1 の再判定対象)。後半にのみ ISO-2022-JP が現れるファイルのため、§8.2.1 の再判定契機に **「手順 4' のシーケンス (`ESC $ @` / `ESC $ B` / `ESC ( I`) の初出」** も加える
7. chardetng 1.0 に sniff 範囲を与えて推定 → Shift_JIS / EUC-JP / ISO-2022-JP / windows-1252 等 (信頼度 中)。**要確認**: chardetng 1.0 (2026-03) のヒントは TLD 形式であり「候補リストを与える API」は無いと理解している。07 §5.2 の `file.detection_candidates` は **chardetng への入力ではなく推定結果の事後フィルタ** (候補外の推定結果は信頼度を下げ、`file.default_encoding` と比較して採否を決める) として定義する。**事後フィルタの手続き** (v0.1): chardetng の推定結果 E について、(i) E が v0.1 の 7 種 (§8.1) に含まれ、かつ `detection_candidates` に含まれる → E を採る (信頼度 中)。(ii) E が 7 種に含まれるが `detection_candidates` に含まれない → 信頼度を「低」に下げ、`file.default_encoding` (`"system"` は 05 §6 で解決した値。以下 D) で sniff 範囲を復号して不正バイトが 0 なら D を、1 以上なら E を採る (どちらも信頼度 低。§8.2.1 の再判定の対象)。(iii) E が 7 種に含まれない (windows-1252・GBK・Big5・EUC-KR・windows-125x 等。FR-110 で明示指定により扱うのは v0.3) → E を採らず、手順 8 と同じく D で開き、不正バイトは §8.3 で保持する (読み取り専用)。(iii) では情報メッセージ (`info.open.guess_unsupported`、04 §15.2) で推定結果 E の名前と「文字コードを指定して開き直す」(FR-111) を案内する。ロケール由来のヒント (05 §6 の `system`) は TLD 形式に写像できる場合のみ渡す。1.0 で追加された `Iso2022JpDetection` / `Utf8Detection` 等の API は手順 4' / 5 と整合するよう P0 (S-01 (a)) で確認する。chardetng が ISO-2022-JP を返した場合も手順 4' の復号可能性検査を適用し、復号できない箇所は不正バイト扱い (手順 4' の注記)
8. 推定不能 → 設定の既定 (`file.default_encoding`、07 §5.2。既定 UTF-8。`"system"` は 05 §6 のヒント) で開き、不正バイトは §8.3 で保持
```

#### 8.2.1 先頭だけで判定することの危険と再判定規則

- 先頭 64 KiB が ASCII のみで後半に Shift_JIS が現れるファイルは、手順 6 で UTF-8 と誤判定される。
- 対策: デコード中 (チャンク毎) に **UTF-8 として不正なバイト列、または ISO-2022-JP のエスケープシーケンス (§8.2 手順 4') を初めて検出した時点** で、信頼度が「低」なら **その周辺 64 KiB を再判定 (手順 4' → chardetng)** し (ISO-2022-JP のシーケンスが契機で窓が 7 bit のときは手順 4' だけを行い、不合格なら手順 6 と同じく UTF-8 のまま読み続ける。chardetng と手順 7 の事後フィルタは窓が 8 bit バイトを含むときだけ使う。窓の両端がファイルの途中で切れる場合も手順 4' と同じく、窓の末尾の未完のシーケンスは Malformed とみなさない。ISO-2022-JP のシーケンスが契機の場合、窓の先頭はそのエスケープシーケンスの位置にする。シーケンス自身が状態を明示的に切り替えるので、窓の前の状態に依存しない)、別の文字コードが推定されれば **読込ジョブを先頭からやり直す** (1 回のみ)。信頼度「高」(非 ASCII の妥当な UTF-8 を既に見ている) なら不正バイトとして保持する。
- 再判定は稀で、コストは最大で読込 2 回分。ユーザーには「文字コードを再判定しました (Shift_JIS)」と通知する。
- 誤判定はそれでも起き得るため、FR-111 (文字コードを指定して開き直す、v0.1 Must) を回復手段とする。

### 8.3 不正バイト列の保持 (OQ-16 / NFR-304)

**方式: 不正バイト 1 つを 1 つのプライベート用途文字にエスケープし、保存時に元のバイトに戻す。**

**版ごとの範囲**: v0.1 は不正バイトを含む文書を **読み取り専用で開き、内容を可視化する** まで (エスケープは表示と無編集保存のために使う)。解除の手段は FR-111 (文字コードを指定して開き直す)。**編集と保存 (「編集を許可」、隣接エスケープの警告、PUA 衝突時の処理) は v0.3**。チャンク境界での元バイトの回収は表示にも必要なので v0.1 で実装する。不正バイトは 1 バイトが UTF-8 で 4 バイトになるため、**不正バイトの比率が 1% を超えるファイルは開く前に「メモリを約 N 倍使う」と警告** する (§11.2、FR-127。`ConfirmAsk::ManyInvalidBytes`。512 MiB の警告とは別の確認 (`ui.confirm.open.invalid_bytes`) で、選択肢は「読み取り専用で開く / 文字コードを指定して開く / 中止」、§9.2 の「確認からの再投入」)。比率は **先頭の sniff 範囲 (§8.2 の 64 KiB。ファイルが小さければ全体)** を判定した文字コードで復号したときの「不正バイト数 / sniff 範囲のバイト数」で決め、本文の構築を始める前に判定する (§9.2 手順 3)。sniff 範囲より後ろの不正バイトでは警告しない (その文書も不正バイトを含むので読み取り専用 (`InvalidBytes`) で開く)。警告の「N 倍」(`memory_factor`) は §9.2 手順 3 の「読込中のメモリ検査」の見積りと同じ式、すなわち **文字コードの倍率 (Shift_JIS・EUC-JP・UTF-16 は 1.5、UTF-8 は 1) × (1 + 3 × sniff 範囲の不正バイトの比率)** で求め、ファイルのバイト数に対するデコード後の本文の大きさの倍率として示す。NFR-202 の目標は妥当なテキストに適用する。

| 項目 | 内容 |
|---|---|
| エスケープ先 | 補助私用面 B の末尾 256 文字: `U+10FF00 + b` (b = 0x00..0xFF)。Rust の `char` として妥当であり、UTF-8 では 4 バイト。サロゲート (Python の surrogateescape 方式) は Rust `String` に入らないため不可 |
| デコード | encoding_rs の `decode_to_string_without_replacement` 相当で **置換なしデコード** を行い、`Malformed(len, consumed_after)` が返るたびに該当 len バイトを個別にエスケープして続行。UTF-8 の場合は `std::str::from_utf8` の `valid_up_to` + `error_len` で同じことを行う。U+FFFD への置換は **行わない** (情報が消える) |
| チャンク境界 | encoding_rs の `Malformed(len, consumed_after)` が示す不正バイトは **直前の入力バッファに属することがある** (公式ドキュメントに明記)。読込ジョブは直前チャンクの末尾 **最大 16 バイト** を保持し、`len + consumed_after` が現チャンクの消費量を超える場合はそこから元のバイト値を回収する。09 L1 に「チャンク境界をまたぐ不正バイト」のプロパティテストを置く (1 バイト単位のチャンク分割でも結果が変わらないこと) |
| 状態付き符号化 | ISO-2022-JP のような状態付き符号化では、生バイトを差し込むとエンコーダの状態 (現在の ESC シーケンス) と食い違い、保存結果が別の意味になる。したがって **状態付き符号化ではエスケープ文字を元のバイトに戻して書くことを無効化** し (`pua_escape = DisabledStateful`)、不正バイトを含む ISO-2022-JP 文書は **常に読み取り専用** とする (02 §13.4 (f)。ISO-2022-JP の保存は v0.1 で行うが、この規則は同じ)。編集したい場合は FR-111 で別の文字コード (UTF-8 等) として開き直すか、不正バイトを含まない文書に直してから扱う。ISO-2022-JP で「不正バイト」になるのは、8 bit バイト、`SO` / `SI` (0x0E / 0x0F)、未対応のエスケープ (`ESC $ ( D` 補助漢字、ISO-2022-JP-2 系)、WHATWG デコーダが誤りとする冗長なエスケープ (出力を挟まない連続エスケープ)、JIS X 0208 の未定義コード、**JIS X 0208 状態・半角カナ状態 (`ESC ( I`) の C0 制御文字 (CR / LF / TAB を含む)**。WHATWG のデコーダは 2 バイト文字・半角カナの状態で 0x21〜0x7E と ESC 以外のバイトを誤りとして 1 バイトずつ消費し、状態はそのまま続く。行末で ASCII 状態に戻さないファイル (RFC 1468 に従わない書き手の出力) では **改行が不正バイトになり、復号結果から改行が消える** (実測、encoding_rs 0.8.35: `1B 24 42 24 22 0A 1B 28 42` → U+3042 の後に `Malformed` (0x0A の 1 バイト)。CR LF なら 2 バイトとも。`ESC ( I` の後の LF も同じ)。このファイルは 7 bit で §8.2 手順 4' を通らないので、自動判定では ASCII (UTF-8) として開く (手順 4' の注記)。**保持と保存の方式** (確定 (ユーザー回答)、OQ-207): 状態付き符号化でも不正バイトは **他の文字コードと同じ PUA エスケープで保持して表示する** (`<0x0A>` 等。消えた改行の位置で行は分かれない。`invalid_bytes` に数え、読み取り専用の理由は `StatefulEncodingWithInvalidBytes`)。不正バイトを文書から落とす実装や U+FFFD に置き換える実装は採らない (どちらも、名前を付けて保存 (読み取り専用でも実行できる、§9.6) で別の文字コードへ書いたときに内容が黙って失われる)。保存は次の 2 つで扱う。(a) **ISO-2022-JP では保存しない**: 書く format の文字コードが ISO-2022-JP (`is_stateful()`) で、文書がエスケープ文字を含む (`invalid_bytes > 0`) とき、core は **保存の連鎖の開始時** (§9.7 の「連鎖の開始」。`file.save`・`file.save_as`・`AskUnsaved` の「保存」・`file.set_encoding` のいずれも。`AskName` で名前を尋ねる前、§9.3 手順 0 の未編集の判定より前) に、保存ジョブも metadata のジョブも投入せずに拒否し、`msg.save.encoding_unsupported_iso2022` (04 §15.2。FR-111 で別の文字コードとして開き直すよう案内する。別の文字コードを指定した保存は (b) でキャンセルだけになるので案内しない) を示して連鎖を中止する (後続 (終了・置き換え・開き直し) も中止し、文書と `TextFormat` は変えない)。名前を付けて保存・文字コードを指定して保存 (§8.8) で ISO-2022-JP を選んだ場合も同じ (他の文字コードで開いた文書のエスケープ文字を ISO-2022-JP で書こうとした場合を含む)。core の拒否が漏れて保存ジョブに届いた場合も、`DisabledStateful` ではエスケープ文字をバイトに戻さず、エスケープ文字 (U+10FF00〜) は ISO-2022-JP のエンコーダで `Unmappable` になるので E1 で表現不能として止まり、何も書かない (二重の防御)。(b) **他の文字コードへの保存** ではエスケープ文字を E4 のとおり表現不能として扱う (§8.6 の E3 で中止。エスケープ文字を含むので「UTF-8 で保存」と「置換して保存」は出ず、キャンセルだけになる)。したがって、この文書から不正バイトの内容が欠けたファイルを書く経路は無い (内容を残したいときは FR-111 で別の文字コードとして開き直す)。表示には `<ESC>` を含むバイト列の可視化 (FR-507) を使う |
| 隣接エスケープの意味変化 (v0.3) | 編集によってエスケープ文字同士、またはエスケープ文字と通常文字が隣接すると、保存後の再読込で **別の文字として復号される** ことがある (例: 孤立バイト `E3 81` の後ろに `82` が置かれると UTF-8 では「あ」になる)。保存ジョブは出力前に「連続するエスケープ列 (および直前・直後の通常文字との組) が、その文字コードで妥当な多バイト列を形成する箇所」を検出し、該当位置を `SaveWarning::EscapeMerge { positions }` として返す。core はその位置を Decoration で示して **警告する** (保存は中止しない。ユーザーが意図して並べた可能性があるため)。設定で「警告時は保存前に確認」に変更可 (v0.4) |
| 衝突 | 元のファイルが **真に** `U+10FF00..U+10FFFF` を含む場合、保存時に区別できない。読込時にデコード結果にこの範囲の文字が現れたら `TextFormat.invalid_bytes` とは別に `pua_escape = DisabledCollision` とし、(a) 不正バイトが 0 ならエスケープ機構を無効化 (この文書では PUA 文字をそのまま扱う)、(b) 両方あるなら **読み取り専用で固定** し警告する。実運用上ほぼ起きない |
| 表示 | FR-507 に従い `<0x9A>` のような記号で描画 (幅 6 セル)。Frame 生成時に Span として展開する (§14.2) |
| 編集 (v0.3) | 既定は読み取り専用。v0.3 でユーザーが「編集を許可」(`InvalidBytes` の解除、§9.6) すると通常の文字として扱える (1 書記素、削除可、コピー可)。コピーした場合クリップボードには `<0x9A>` の表記文字列を書く (バイトは書けないため)。v0.1 / v0.2 は `InvalidBytes` を解除できない (`file.toggle_read_only` (v0.2) の対象外) |
| 保存 | エンコード時に `U+10FF00..FF` を見つけたら **そのバイト値をそのまま出力** する。これにより無編集保存でバイト一致 (NFR-303 の例外ではなく、保持方式で満たす) |
| 検索 | 正規表現 (v0.2) では `\x{10FF9A}` で一致させられる。UI では `\x9A` 風の糖衣を検討 (04/12) |
| 他の文字コードへ変換保存 | エスケープ文字は表現不能として FR-112 の警告対象 (位置を示す) |

> Latin-1 として開き直すことは FR-111 で行える。

### 8.4 改行コード (OQ-04)

**OQ-04 で確定 (ユーザー回答) した「内部でも元の改行コードをそのまま保持 (混在も保持)」に従う。** 新規文書の既定は LF、貼り付けは文書の既定改行に揃える (OQ-51)。 本書での影響をすべて列挙する。

| 領域 | 影響と規則 |
|---|---|
| 行の定義 | §2.4。LF / CRLF / CR を区切りとし、CRLF は 1 つ |
| 改行の挿入 (FR-202) | `default_eol` を挿入する (混在ファイルで直前・直後の改行と合体する場合は §6.2 の `eol_guard` が種類を合わせる)。読込時の `default_eol` は多数派 (同数なら LF、全く改行がなければ設定の既定 `file.default_line_ending`、既定 LF。OS 依存にしない: P6) |
| 混在 (Mixed) | `eol_stats` の 2 種以上が非 0 なら Mixed。ステータスバーに `混在` (01 FR-503 / 04 §8.7) と示す。警告も「揃える」提案もしない (保持が正しい挙動。OQ-52) |
| 変換コマンド (FR-113、**v0.1**) | `file.set_line_ending(eol)` (`eol` = LF / CRLF / CR。パレットでリストから選ぶ、07 §11.6)。手順: (1) 文書が編集できない (`read_only` が空でない、または `load_state != Ready`) なら Transaction を作らずに `msg.edit.read_only` で拒否する (§9.6)。(2) 変える改行の数 k を `eol_stats` から求める (`eol` 以外の 2 種の和。走査しない)。k = 0 なら本文を変えず、`default_eol` だけを `eol` にして (メタデータの変更。Undo の対象外で dirty にしない、§7.2) `info.eol.unchanged` を出す。(3) k > 0 なら、`eol` と異なるすべての区切り (混在の少数派を含む) を `eol` に置き換える ChangeSet を作る。文書が 1 MiB (`SEARCH_SYNC_STEP_BYTES`、§12.2) 以下ならメインスレッドで作り、超えればジョブ (`JobKind::ConvertEol`、02 §9.3) で Snapshot (Revision R) から作る (全文走査。適用は O(k log n))。すべての区切りを同じ `eol` にするので、置き換えで隣り合った CR と LF が新しい CRLF に結合することはない。(4) 適用の時点で Revision が R でなければ、今の Snapshot で 1 回だけ作り直し、それも古ければ適用せずに `msg.eol.convert_stale` を出す。(5) **1 つの Transaction** (`Origin::Command("file.set_line_ending")`、`UndoGroupHint::Break`) として適用し、同時に `default_eol` を `eol` にする。Undo ノードの `format_change` に前後の `default_eol` を持たせ、1 回の Undo で本文と `default_eol` が一緒に戻る (§7.2)。`eol_stats` は適用で数え直され (§2.4)、ステータスバーの改行コードは `eol` になる (「混在」が消える)。文書は dirty になり、`info.eol.converted` で件数を示す。選択は ChangeSet で写像する。変換は保存時のエンコードと独立で、どの文字コードでもそのまま書く (保存は区切りをそのまま書く、下の「保存」)。Undo の予算 (§7.5) の「直前の 1 操作は保持」が大きな変換にも働く |
| カーソル | CRLF の内側に置けない (書記素) |
| 削除 | 行末で Delete → CRLF 全体 (1 書記素) を削除。Backspace も同様 |
| 検索 | リテラル検索では改行 (`\n`) を入力できない (単一行の入力欄。§12.3)。正規表現の `\n` (複数行にまたがる一致は v0.3。v0.2 は「未対応」と表示、§12.3) は `\r?\n\|\r` と等価の糖衣で、`LF` / `CRLF` / `CR` の **どれにも一致** させる (正規表現の `R` フラグ = CRLF モード)。`$` は CR の直前にも一致する ([§12.3](#123-正規表現)) |
| 貼り付け (FR-113 / FR-206) | **正規化は core の貼り付けの入口 1 か所** で行う: 挿入するテキスト中の `\r\n`・単独の `\r`・単独の `\n` を、それぞれ 1 つの `default_eol` に置き換える (`normalize_eol(text, default_eol)`、1 パス)。対象は **すべての貼り付けの経路**: ブラケットペースト (`InputEvent::Paste`)、バースト検出で合成された `Paste` (04 §6.6)、システムクリップボードの読取結果 (`InputEvent::ClipboardContent`、Windows では CRLF で届く)、内部レジスタ (CRLF・CR の文書からのコピーを含む)。端末からの貼り付けは CR になることが多く、そのままでは意図せず Mixed になるか、`\r\n` の `\n` だけを変換して `\r\r\n` (行が増える) になる。frontend (eve-tui) が `\r\n` / `\r` → `\n` の前処理をしても結果は同じなので、前処理は **任意** とする。正規化を切る設定は v0.2 で公開 (07 §5.13 `editor.paste_normalize_eol`)、v0.1 は常に ON。正規化したあとの挿入文字列の端が、挿入位置の前後の単独の `\r` / `\n` と隣接して合体する場合は、§6.2 の `eol_guard` が端の 1 文字だけを直す (混在ファイルの場合) |
| 表示 | 行末の区切りは描画しない。混在時に少数派の区切りを可視化する設定 (FR-508 の一部、v0.4) |
| 保存 | そのまま書き出す (変換なし)。NFR-303 を混在ファイルでも満たす |
| LSP / tree-sitter | 位置は行 + 桁で渡す。CRLF が 2 バイトである点は `LineByte` 変換で自然に扱える |

### 8.5 末尾改行 (FR-114)

- 読込時に末尾が区切りなら最終行が空 (§2.4)。保存はテキストをそのまま書くので、末尾改行の有無は **自動的に保持** される。専用フラグは不要 (`final_newline` は導出値)。
- FR-216 (保存時に末尾改行を付与) は、保存ジョブに渡す前に Transaction (Origin::Command) として適用する (Undo 可)。保存後のバッファと保存内容を一致させるため、内部のテキストも変更する。

### 8.6 エンコード (保存) と表現不能文字 (FR-112)

**前提 (要確認 S-01)**: encoding_rs (WHATWG 準拠) のエンコーダは、表現不能文字を常に `Unmappable` で返すわけではない。Shift_JIS / EUC-JP エンコーダは U+00A5 (¥) を 0x5C、U+203E (‾) を 0x7E、U+2212 (−) を U+FF0D 相当のバイト列に **黙って写像** し、ISO-2022-JP エンコーダは半角カナを全角カナに黙って変換する。また EUC-JP エンコーダは JIS X 0212 (3 バイト、補助漢字) を、Shift_JIS エンコーダは CP932 の外字 (U+E000〜U+E757、§8.7) を **エンコードしない** (復号専用。外字は `eve-encoding` が自前の逆算で補う、§8.7 の「CP932 の外字」)。JIS 系・Mac 系の写像で作られた文字 (U+301C `〜`・U+2016 `‖`・U+00A2 `¢`・U+00A3 `£`・U+00AC `¬`・U+00A6 `¦`・U+2014 `—`) は Shift_JIS / EUC-JP / ISO-2022-JP のいずれでも `Unmappable` になる (実測、encoding_rs 0.8.35。§8.7 の「CP932 互換文字と検索」)。「`Unmappable` が返ったら中止」だけでは FR-112 (黙って置換しない) を満たせない。

| 規則 | 内容 |
|---|---|
| E1 照合 | 保存ジョブは Snapshot のチャンクを順に `encode_from_utf8_without_replacement` 相当で変換し、**UTF-8 以外の文字コードでは、出力したバイト列を直ちに同じ文字コードで decode して元のチャンク文字列と照合** する。`Unmappable(ch)` が返った文字と、照合で一致しなかった文字 (黙示置換) を **同じ「表現不能」として扱い**、位置 (`Pos`) を記録して続行 (最大 100 件記録、総数は数える)。ただし Shift_JIS で `Unmappable(ch)` の `ch` が U+E000〜U+E757 (CP932 の外字) のときは、表現不能とせず §8.7 の「CP932 の外字」の逆算で 2 バイトを出力して続行する (照合はその出力を encoding_rs で復号して行い、同じ文字に戻るので一致する)。コストは encode とほぼ同じ (decode 1 回追加)、追加メモリはチャンク 1 つ分。ファイルサイズによる閾値は設けない |
| E2 保持方式 (v0.3) | 照合不一致の文字が **`roundtrip` の範囲内にあり、その範囲が読込以来編集されていない** (ChangeSet 写像で `Mapped` のまま) 場合は、表現不能とせず **`LossySpan.original_bytes` をそのまま出力** する。これにより JIS X 0212 を含む EUC-JP ファイルや CP932 の重複符号化文字を含むファイルを **無編集で元のバイト列どおり保存できる** (NFR-303)。編集された範囲内の該当文字は E1 の表現不能として扱う。**v0.1 は保持方式を持たず E2' で代替** する |
| E2' 検出と確認 (v0.1) | 読込時の照合 (§8.7) で記録した `RoundtripInfo::Summary` の件数が 1 以上の文書を **元の文字コードで保存するとき**、「N 箇所が同じ文字の別のバイト表現で保存されます (例: IBM 拡張の ¬ 0xFA54 → 0x81CA)」と確認ダイアログを出し、承認後に E1 の照合結果 (代表バイト列) で書く。復号専用文字 (EUC-JP の JIS X 0212 等、エンコーダが出力できないもの) は E1 で表現不能として報告される。CP932 の外字は自前の逆算で元のバイトに戻るので (§8.7、確定 (ユーザー回答)、OQ-206) E2' の件数にも E1 の表現不能にも入らない。**無編集の上書き保存は書き込まない** (§9.3 手順 0) ので、確認が出るのは編集した文書を元の文字コードで保存する場合だけ。意味 (文字列) は変わらず、黙って変わることはない |
| E3 中止 | E1 の表現不能が 1 件以上あれば **保存を中止** し (原子的保存では一時ファイルを Port が破棄、直接上書きでは乾式走査で検出するので元ファイルに触れない、§9.3)、`JobResult::Failed { err: JobError::Unmappable { positions, total } }` (02 §9.3) を返す。core は該当箇所を Decoration で示し、下の「E3 の選択肢」を出す (`ui.confirm.save.unmappable`、04 §15.2)。利用者が選ばない限り置換しない (黙って `?` にしない)。本規則 (E1〜E3) は 02 §13.4 / ADR-0026 (条件付き確定 (S-01)) の具体化で、ADR-0026 の (1) 照合は v0.1、(2) 保持方式は v0.3 と段階化する |
| E4 エスケープ文字 | §8.3 のエスケープ文字は、元の文字コードで保存するときはバイトに戻し、別の文字コードへ変換するときは表現不能として扱う。状態付き符号化ではバイトに戻さず、エスケープ文字を含む文書をその文字コードで保存しない (保存ジョブを投入する前に拒否する。§8.3、確定 (ユーザー回答)、OQ-207) |
| E5 UTF-16 | UTF-16LE/BE のエンコードは自前 (`char::encode_utf16`)。BOM は `TextFormat.has_bom` に従って書く。照合は不要 (1:1)。細目は下の「UTF-16 の書き込み」 |
| E6 ISO-2022-JP の保存 (**v0.1**、02 §13.4) | 状態付き符号化のため、保存ジョブは encoding_rs の ISO-2022-JP エンコーダを **行ごとに閉じる** (確定 (ユーザー回答)、OQ-162): 1 行の内部ではエンコーダの状態 (ASCII / ローマ字 / JIS X 0208) をチャンクの境界をまたいで引き継ぎ (チャンクごとに作り直さない)、行の内容を `last = true` で符号化して ASCII 状態に戻してから改行バイトを付ける。各チャンクの出力バッファは `max_buffer_length_from_utf8_if_no_unmappables` で取る (状態遷移のエスケープ 3 バイト分を含む)。各行の終わりで `last = true` で呼び、ASCII 状態へ戻す `ESC ( B` を必ず出す (`has_pending_state()` が偽になることを確認。改行を含まない最終行も同じ)。E1 の照合に使うデコーダも同じ単位 (1 行の内部はチャンクをまたいで通し) で使い、出力を復号した文字列を元の文字列と比較する (エスケープは文字を生まないので、チャンク境界をまたいでも比較は成り立つ。最後は `last = true` で復号し、残りが無いことを確認)。**行末の状態**: WHATWG のエンコーダは、JIS X 0208 状態からは ASCII 文字の前で `ESC ( B` を出して ASCII 状態に戻るが、**ローマ字状態 (`¥` U+00A5 / `‾` U+203E の出力後) では U+005C / U+007E 以外の ASCII (改行を含む) を状態を変えずに出力** し、ASCII 状態へ戻すのは入力の終わり (`last = true`) だけである。したがってエンコーダを文書全体で 1 つ通すだけでは、各行が ASCII 状態で終わるとは限らない (例: `"¥\nabc"` → `ESC ( J 5C 0A 61 62 63 ESC ( B`)。各行を ASCII 状態で終える (FR-109 AC-3 の行末条件、RFC 1468 の慣行) ために **行ごとにエンコーダを閉じる** (OQ-162 で確定 (ユーザー回答)。encoding_rs の公開 API だけで実装でき、E1 の照合とも両立する。同じ例は `ESC ( J 5C ESC ( B 0A 61 62 63` になる)。S-01 (a') で `"¥\nabc"` の出力を確認する。**S-01 で照合が成り立たない文字コード** は v0.1 では読込のみ (別の文字コードでの保存は可) とし、Shift_JIS / EUC-JP の照合が成り立たなければ v0.1 のリリースを止める (OQ-162)。**半角カナ** (U+FF61〜FF9F) はエンコーダが黙って全角に変換するので E1 の照合で不一致になり **表現不能** として扱う (選択肢は下の「E3 の選択肢」の表)。ISO-2022-JP の半角カナは **E1 / E3 だけで扱い**、区分 D (§8.7) と E2' の対象にしない (半角カナを含む行は E3 の置換を経ないと保存できないので、E2' の確認に意味が無い)。`¥` (U+00A5) / `‾` (U+203E) は `ESC ( J` で表現されるので一致する。`SO` / `SI` / `ESC` (U+000E / U+000F / U+001B) は WHATWG エンコーダが誤りとするので表現不能。PUA エスケープ (不正バイト) を含む文書は §8.3 のとおり ISO-2022-JP では保存しない (保存の連鎖の開始時に `msg.save.encoding_unsupported_iso2022` で拒否する。確定 (ユーザー回答)、OQ-207)。**エスケープの選び方の差** (`ESC $ @` → `ESC $ B`、`ESC ( J` で書かれた ASCII → `ESC ( B`、冗長なエスケープの除去) は再エンコードで変わるため、読込時に区分 B として検出し (§8.7)、編集後に ISO-2022-JP で保存するとき E2' の確認を出す。無編集の上書き保存は書き込まない (§9.3 手順 0) ので無編集ならバイト一致 |

**E3 の選択肢 (本表が正本。02 §13.4 と 04 §15.2 `ui.confirm.save.unmappable` は本表を参照する)**: 表現不能で保存を中止したとき、次の 3 つを出す。どれを選んでも、それまでに元ファイルへは何も書いていない。

| # | 選択肢 | 動作 | `TextFormat` | 出す条件 |
|---|---|---|---|---|
| 1 | UTF-8 で保存 | 同じ保存先へ、`format.encoding = UTF-8`・`has_bom = false` (他の項目は元のまま) の format を渡して保存ジョブを投入し直す | 保存が成功したときだけ UTF-8 に確定する。失敗・キャンセル・`Conflict` で終わったら元の format のまま | 表現不能の文字に §8.3 のエスケープ文字 (UTF-8 でも表現不能、E4) を含まないとき |
| 2 | 置換して保存 | 表現不能と判定された文字 (Unicode スカラー値) を下の「置換先」の規則で置き換える **1 つの Transaction** (`Origin::System`、§6.3。Undo で戻せる) を作り、**適用の前に対象の一覧を示して確認を取り** (下の「置換の一覧」)、承認されたら適用してから同じ文字コードで保存ジョブを投入し直す。置き換える位置は全件 (記録した先頭 100 件だけではない) なので、ChangeSet は E1 と同じ照合を全文に走らせるジョブ (`JobKind::ReplaceUnmappable`、02 §9.3) で作る (§12.5 の「すべて置換」と同じく、作った Revision と適用時の Revision が違えば作り直す) | 変えない | 文書が編集できるとき (`read_only` が空で `load_state == Ready`)。表現不能の文字にエスケープ文字を含まないとき |
| 3 | キャンセル (`Esc`。既定) | 保存しない。文書は変わらない (dirty のまま)。カーソルを最初の表現不能の位置へ移し、該当箇所の Decoration を次の編集まで残す | 変えない | 常に |

- **置換先** (選択肢 2): (a) 保存先の文字コードが ISO-2022-JP で、対象が **半角カナ** (U+FF61〜FF9F) のとき → **全角カナ** に置き換える。対応は WHATWG Encoding Standard の index ISO-2022-JP katakana (エンコーダが黙って行う変換と同じ) で、1 文字を 1 文字に置き換える。濁点・半濁点 (`ﾞ` U+FF9E / `ﾟ` U+FF9F) は直前の文字と合成せず `゛` (U+309B) / `゜` (U+309C) にする (`ｶﾞ` → `カ゛`。一覧の 1 件 = 表現不能の 1 文字を保つため)。(a') 保存先の文字コードが Shift_JIS / EUC-JP / ISO-2022-JP で、対象が下の **互換文字の表** にあるとき → 表の置換後の文字に置き換える (確定 (ユーザー回答)、OQ-208。Windows の CP932 への変換が行う置き換えと同じ組で、1 文字を 1 文字に置き換える。置換後の文字は 3 つの文字コードのすべてで表現でき、往復する (実測、encoding_rs 0.8.35。ISO-2022-JP では JIS X 0208 状態の 2 バイト。U+FFE4 は ISO-2022-JP でも `ESC $ B 7C 7C` で往復する)。件数は `JobResult::UnmappableReplacement` の `counts.compat` に数える)。表の行のうち `¥` `‾` は保存先が Shift_JIS / EUC-JP のときだけ対象で、ISO-2022-JP では表現できる (`ESC ( J`) ので表現不能にならない。(b) それ以外の表現不能文字 ((a) と (a') の対象以外のすべて。例: JIS X 0208 に無い漢字・絵文字、編集した JIS X 0212 の文字、EUC-JP / ISO-2022-JP で保存する U+E000〜U+E757。§8.3 のエスケープ文字を含むときは選択肢 2 自体を出さない) → ASCII の `?` (U+003F) に置き換える (1 文字 = 1 スカラー値を 1 つの `?` に)。置き換えた後の文字は必ずその文字コードで表現できる (全角カナ・互換文字の置換後は JIS X 0208 または CP932 にある)。**判定の順序**: 1 つの表現不能文字について (a) → (a') → (b) の順に最初に当てはまるものを使う (対象が重ならないので順序で結果は変わらないが、実装はこの順で引く)。

  | 対象 (互換文字の表、(a')) | 置換後 | 備考 |
  |---|---|---|
  | U+301C `〜` WAVE DASH | U+FF5E `～` | Shift_JIS 0x8160 / EUC-JP 0xA1C1 |
  | U+2016 `‖` | U+2225 `∥` | 0x8161 |
  | U+2212 `−` | U+FF0D `－` | `Unmappable` ではなく 0x817C に黙って写され、E1 の照合で不一致になる |
  | U+00A2 `¢` / U+00A3 `£` / U+00AC `¬` | U+FFE0 `￠` / U+FFE1 `￡` / U+FFE2 `￢` | 0x8191 / 0x8192 / 0x81CA |
  | U+00A6 `¦` | U+FFE4 `￤` | Shift_JIS 0xFA55 (IBM 拡張) / EUC-JP 0xFCFC |
  | U+2014 `—` EM DASH | U+2015 `―` | 0x815C |
  | U+00A5 `¥` / U+203E `‾` (Shift_JIS / EUC-JP のときだけ。ISO-2022-JP は `ESC ( J` で表現できる) | U+FFE5 `￥` / U+FFE3 `￣` | encoding_rs が 0x5C / 0x7E に黙って写し、E1 の照合で不一致になるので **E3 の対象として利用者に知らせる** (黙って 0x5C / 0x7E で書かない)。置換先は U+FFE5 / U+FFE3 (Shift_JIS 0x818F / 0x8150、EUC-JP 0xA1EF / 0xA1B1) に固定し、0x5C (U+005C) / 0x7E には置き換えない (確定 (ユーザー回答)、OQ-208。U+005C にすると再読込で `\` になり、文字が変わるため) |
- **置換の一覧** (選択肢 2 を選んだ後、適用の前。§9.7 の `BuildingReplacement` → `ConfirmReplacement`): ChangeSet を作るジョブの結果 `JobResult::UnmappableReplacement { changes, targets, counts: { kana, compat, question } }` (02 §9.3。`targets` は先頭 100 件で、位置はジョブの base Revision の座標。対象の総数は `counts` の合計 `kana + compat + question` で導き、別のフィールドに持たない) から、対象の件数 (置換先ごと: 「全角カナへ {kana} 件」「互換文字へ {compat} 件」「`?` へ {question} 件」。この順で並べ、0 件の項目も省かない) と、先頭 100 件の一覧 (1 件 1 行: `行 {line}, 桁 {col}: {元の文字} (U+XXXX) → {置換後}`。行・桁は base Revision の座標から求める。base が今の Revision と違う結果は使わずに作り直すので、座標はそのまま今の文書の座標である。元の文字は §14.6 の無害化を通す) を、確認 (`ui.confirm.save.unmappable_replace`、04 §15.2) の `details` に入れて示す。100 件を超えるときは「ほか N 件」を末尾に付ける (画面に収まらない行の省略は 04 §8.3)。選択肢は「置換して保存」/ キャンセル (`Esc`。既定) で、キャンセルは選択肢 3 と同じ扱い (保存しない、最初の該当位置へ移動、後続の中止)。
- 選択肢 1 / 2 で投入し直した保存も、E1〜E3 と §9.3 の手順をそのまま通る (2 回目の表現不能・`Conflict` 等はその時点の規則で扱う)。後続の処理 (終了・置き換え等) を伴う保存の連鎖では、3 を選ぶと後続を中止する (§9.7)。

**UTF-16 の書き込み (E5 の細目、v0.1 の確定)**: encoding_rs は UTF-16LE/BE への **エンコードを提供しない** (WHATWG Encoding Standard で UTF-16 はデコード専用。encoding_rs の `Encoding::output_encoding()` は UTF-16 に対して UTF-8 を返す。02 §13.4 の encoding_rs の行)。したがって UTF-16 の保存は `eve-encoding` の自前の書き手で行い、encoding_rs の `Encoder` を UTF-16 に対して作らない (作ると黙って UTF-8 で書かれる)。規則は次のとおり。

| 項目 | 規則 |
|---|---|
| 符号化 | 文字ごとに `char::encode_utf16` で 1〜2 単位にし、`to_le_bytes` / `to_be_bytes` で書く。チャンクの境界で文字は割れない (入力は `&str` のチャンク) ので状態を持たない |
| BOM | `TextFormat.has_bom = true` なら先頭に `FF FE` (LE) / `FE FF` (BE) を 1 回書く。無 BOM で判定した文書 (§8.2 手順 3) は無 BOM のまま書く (無編集保存はそもそも書かない、§9.3 手順 0。BOM なしの UTF-16 はバイト一致の保証の対象外、§8.7)。「文字コードを指定して保存」(FR-111) で UTF-16 を選んだときの既定は **BOM 付き** (`file.write_bom` は UTF-8 の BOM の設定なので関係しない。BOM なし UTF-16 は判定が当てにならないため)。文書中の U+FEFF (先頭以外、または BOM とは別に先頭にある 2 個目) は通常の文字として符号化する (§8.7 区分 A) |
| サロゲートの不整合 | Rust の `char` はサロゲート (U+D800..DFFF) を持てないので、**編集・入力したテキストから不整合なペアは生まれない**。読込時の対になっていないサロゲート (孤立した上位・下位、逆順) は encoding_rs のデコーダが `Malformed` として返すので、§8.3 のとおり **元のバイトを 1 バイトずつ PUA (`U+10FF00 + b`) にエスケープ** して保持し (不正バイトを含む文書として読み取り専用、v0.1)、保存時は E4 のとおり元のバイト値をそのまま出す。したがって孤立サロゲートを含むファイルも無編集・同じ文字コードの保存ならバイト一致になる。奇数長のファイルの末尾 1 バイトも同じ扱い |
| E1 の照合との関係 | UTF-16 は全 Unicode スカラー値を 1:1 で表せるので、E1 の encode→decode 照合を **行わない** (E1 の「UTF-8 以外の文字コードでは照合」の例外は UTF-16LE/BE だけ)。表現不能になり得るのは §8.3 のエスケープ文字だけで、これは E4 の規則 (元の文字コードならバイトに戻し、別の文字コード (例: Shift_JIS 由来のエスケープを UTF-16 で保存) なら表現不能) で扱う。読込時の区分 B の比較 (§8.7) も UTF-16 では行わない (区分 A) |
| 検証 | 往復コーパス (09 §4) に UTF-16LE/BE の BOM 付き・補助面の文字 (サロゲートペア)・孤立サロゲート・奇数長を置き、無編集の上書き保存 (書かない) と「名前を付けて保存」でバイト一致を確かめる (FR-108 AC-1、NFR-303 AC-2)。BOM なしの UTF-16 のファイルは判定の結果 (UTF-16 と判定されるか、バイナリの確認になるか) を記録するだけで、バイト一致を合否にしない (FR-108)。encoding_rs に UTF-16 のエンコーダを要求するコードが無いことを単体テストで固定する |

### 8.7 往復保証の範囲 (NFR-303)

「読込 → 無編集 → 保存」でのバイト一致を、**4 区分** で明文化する。

| 区分 | 意味 | 該当 |
|---|---|---|
| **A 保証** | 変換が 1:1 で、追加の仕組みなしに一致 | UTF-8 / UTF-8+BOM / UTF-16LE/BE (**BOM 付きのみ**。BOM なしの UTF-16 は判定が推定 (§8.2 手順 3) なので区分 A に含めず、バイト一致を保証しない。FR-108)。混在改行、末尾改行なし、文書中の U+FEFF (非正規化、§8.4 / §8.5)。不正バイト列 (§8.3 の保持方式。NFR-304 の「別方式」はこれで満たす。Shift_JIS の単独バイト 0xA0 / 0xFD〜0xFF もここ)。Shift_JIS (CP932) の外字 (自前の逆算、下の「CP932 の外字」。編集した後もバイト一致) と CP932 互換文字の復号結果 (U+FF5E 等。encoding_rs で往復) |
| **B 保持方式で保証** | 再エンコードでは元に戻らないが、読込時に元バイト列を `roundtrip` に保持し、無編集なら E2 でそのまま書くため一致 | Shift_JIS (CP932) の NEC 特殊文字 / IBM 拡張の **重複符号化** (例: `¬` が 0x81CA / 0xEEF9 / 0xFA54)。EUC-JP の JIS X 0212 (3 バイト) と、WHATWG が別コードに写す文字 (例: 0x8FA2B7 → U+FF5E → 0xA1C1)。**復号専用 (エンコーダが出力しない) の文字**: EUC-JP の JIS X 0212 の大半。Shift_JIS (CP932) の **外字** (ユーザー定義領域 0xF040〜0xF9FC → U+E000〜U+E757) は encoding_rs では復号専用だが、`eve-encoding` の自前の逆算で元のバイトに戻るので **区分 A** (下の「CP932 の外字」、確定 (ユーザー回答)、OQ-206)。CP932 互換文字 (0x8160 → U+FF5E 等) は再エンコードで元に戻るので区分 B ではない (下の「CP932 互換文字と検索」)。その他 encoding_rs の 1 対多写像 (GBK/Big5 の一部) |
| **C 編集箇所のみ非保証** | 区分 B の範囲を **編集した** 場合、その範囲は再エンコード結果 (代表バイト列) になる。JIS X 0212 のように encoding_rs がエンコードできない文字は E1 で表現不能として報告される | 区分 B と同じ対象の編集後 |
| **D 状態付き符号化の区分 B (エスケープ選択の差)** | ISO-2022-JP: エスケープシーケンスの選択 (`ESC $ @` vs `ESC $ B`、ローマ字 `ESC ( J` で書かれた ASCII vs `ESC ( B`、冗長なエスケープ、行末でのリセット位置、末尾の状態) は復号で失われ、再エンコードでは WHATWG エンコーダの正規形 (`ESC $ B` / `ESC ( B`、`¥` `‾` のみ `ESC ( J`、JIS X 0208 状態からは ASCII の直前で ASCII に戻り、末尾で ASCII に戻る。行末の扱いは §8.6 E6 の方式 (OQ-162) に従う) に揃う。**v0.1**: 読込時に **行単位** で元バイト列と再エンコード結果を比較し、差のある行数を `RoundtripInfo::Summary.count`、行頭位置を `first_positions` に記録する (行単位にするのは、状態付きでは文字単位の対応付けが一意に取れないから。比較用の再エンコードは保存と同じ方式 (§8.6 E6) で行う。行の終わりの状態が元ファイルと再エンコード結果で違う場合は、その行を差として数え、次の行は再エンコード側の状態を引き継いで比較する)。無編集の上書き保存は書き込まない (§9.3 手順 0) のでバイト一致。編集後・名前を付けて保存は E2' の確認 (「N 行のエスケープシーケンスが正規形に揃えられます」) を経て正規形で書く (意味 = 文字列は一致、NFR-303 の例外文)。**v0.3**: 保持方式では「ASCII 状態で始まり ASCII 状態で終わる無編集の行」だけ元バイト列を書き (`LossySpan` の単位は行、`reason = StatefulEscape`)、それ以外の行は正規形 (区分 C)。**半角カナ** (`ESC ( I` で書かれたものを含む) は区分 D の対象にしない: E1 の照合で表現不能として検出され、エンコーダが黙って全角化することはない。全角化は E3 の置換 (§8.6 の「E3 の選択肢」2) でだけ行う。読込時の行単位の比較では、表現不能の文字 (半角カナ等) を含む行を `Summary.count` に数えない (その行は E3 の置換を経ないと保存できないので、E2' の確認の対象にならない) | ISO-2022-JP |

- **検出は読込時にストリーミングで行う**: UTF-8 以外の文字コードでは、デコードした各チャンクを再エンコードして元バイト列と比較し、不一致を `roundtrip` に記録する。**ファイルサイズによる省略はしない** (読込時間の増分は再エンコード 1 パス分で、S-01 で Shift_JIS 100 MB を実測し NFR-104 の 1.5 倍許容内かを確認する)。ISO-2022-JP では比較単位を **行** にし (区分 D)、再エンコードは保存と同じ方式 (§8.6 E6、OQ-162) で行い、行ごとの出力バイト列を元ファイルの対応バイト範囲 (デコーダが消費したオフセットで区切る) と比較する。
- **版ごとの記録内容**: v0.1 は `Summary { count, first_positions (≤ 100) }` だけ (メモリは定数)。保存時は E2' の確認に使う。v0.3 の `Spans` は不一致範囲の元バイト列を持つ (通常は数バイト〜数 KB) が、不一致が密なファイルでは 1 箇所約 80 バイトの `LossySpan` が数千万個になり得るため、**隣接するものを結合し、件数が 100 万を超えたら記録をやめて「この文書は元の文字コードで保存できない (別の文字コードを求める)」状態** にする (安全側)。
- `roundtrip` が非空なら読込完了時に情報メッセージ (「この文字コードでは、編集した箇所の一部の文字が保存時に別のバイト列になることがあります」) を出し、該当箇所を Decoration (`DecorationKind::LossyEncoding`) で示す (v0.1 は先頭 100 箇所)。無編集保存では警告しない (v0.1 は書かない、v0.3 は区分 B で一致するため)。
- **区分 B の v0.1 での主張**: 無編集の上書き保存は書き込まないため (§9.3 手順 0)、文字コードによらずバイト一致が成り立つ。編集した文書では E2' の確認を経て代表バイト列で書く (区分 C と同じ結果になる)。バイト一致を保持方式で保証するのは v0.3。
- **CP932 互換文字と検索**: encoding_rs (WHATWG) の Shift_JIS / EUC-JP の復号は Windows (CP932) と同じ写像で、0x8160 (EUC-JP 0xA1C1) → U+FF5E (FULLWIDTH TILDE)、0x8161 → U+2225、0x817C → U+FF0D、0x8191 / 0x8192 / 0x81CA → U+FFE0 / U+FFE1 / U+FFE2、0x815C → U+2015 に写し、いずれも再エンコードで元のバイトに戻る (実測、encoding_rs 0.8.35)。互換文字の問題は逆向きに起きる: JIS 系・Mac 系の写像で作られた文字 (U+301C WAVE DASH・U+2016・U+2212・U+00A2・U+00A3・U+00AC・U+00A6・U+2014) は、UTF-8 の文書や他アプリからの貼り付けで文書に入り、(a) Shift_JIS / EUC-JP / ISO-2022-JP で保存すると表現不能になる (U+2212 以外は `Unmappable`、U+2212 は 0x817C に黙って写されて E1 の照合で不一致。扱いは §8.6 の E3 で、「置換して保存」では置換先 (a') の CP932 の文字に置き換える。確定 (ユーザー回答)、OQ-208)。(b) 文書内の「〜」(U+301C) を IME で入力した「～」(U+FF5E) で検索しても一致せず、同じ文書内で見た目が同じ文字のコードポイントが混在し得る。**保存データは変えない** (P1)。検索でこれらを同一視するオプション (`search.cp932_compat_fold`、既定 ON を提案、v0.3) は OQ-41 で、対象は §8.6 の互換文字の表の組 (JIS 系の文字と CP932 の文字) である。この組の一覧を 09 §4 のコーパスに置く。
- **CP932 の外字 (ユーザー定義領域)**: encoding_rs の Shift_JIS デコーダは 0xF040〜0xF9FC (1,880 字) を U+E000〜U+E757 (私用領域) に復号するが、エンコーダはこれらを `Unmappable` で返す (復号専用。実測、encoding_rs 0.8.35: 0xF040 → U+E000、0xF9FC → U+E757、U+E000 / U+E757 の符号化は `Unmappable`)。補わなければ外字を含む文書を編集して Shift_JIS で保存すると E1 の表現不能になり、外字のバイト値が失われる。**確定 (ユーザー回答)、OQ-206**: `eve-encoding` が Shift_JIS の符号化で U+E000〜U+E757 を **自前で逆算して書く** (Windows の CP932 の変換と同じ写像。encoding_rs は復号だけに使う)。
  - **式**: encoding_rs の復号は `p = (lead − 0xC1) × 188 + (trail − (trail < 0x7F ? 0x40 : 0x41))`、`cp = 0xE000 + p − 8836` (lead 0xF0〜0xF9、trail 0x40〜0x7E / 0x80〜0xFC)。その逆は、`i = cp − 0xE000` (0〜1879) として `lead = 0xF0 + i / 188`、`t = i % 188`、`trail = t + (t < 0x3F ? 0x40 : 0x41)` (0x7F を飛ばす)。`p = i + 8836` と置けば `lead = p / 188 + 0xC1` と同じ (8836 = 47 × 188)。
  - **例と境界**: U+E000 → 0xF040 (先頭)、U+E03E → 0xF07E、U+E03F → 0xF080 (0x7F を飛ばす)、U+E05E → 0xF09F、U+E0BB → 0xF0FC (lead の行の末尾)、U+E0BC → 0xF140 (次の lead)、U+E3EB → 0xF580、U+E757 → 0xF9FC (末尾)。範囲外 (U+DFFF 以下、U+E758 以上) は逆算しない (encoding_rs の結果どおり。私用領域の残りは表現不能)。0xF040〜0xF9FC の外側 (0xEFFC は encoding_rs で `Malformed`、0xFA40 以降は IBM 拡張) は対象外。
  - **組み込み方**: 保存の符号化 (E1) で encoding_rs のエンコーダが `Unmappable(ch)` を返し、保存先が Shift_JIS で `ch` が U+E000〜U+E757 なら、上の 2 バイトを出力してエンコーダを続ける (エンコーダの状態は無いので作り直さない)。E1 の照合は出力を encoding_rs で復号して行い、同じ文字に戻るので一致する。読込時の区分 B の検出 (再エンコードして元のバイト列と比べる) も同じ符号化器を使うので、外字を不一致に数えない (区分 A)。`JobKind::ReplaceUnmappable` の照合も同じ符号化器を使う (外字は置換の対象にならない)。
  - **検証** (実測、encoding_rs 0.8.35。S-01 (c) で再確認する): lead 0xF0〜0xF9 × trail 188 通り = 1,880 個の全コードで「encoding_rs で復号 → 上の式で符号化 → 元の 2 バイト」が成り立ち、復号結果は U+E000〜U+E757 の 1,880 個 (重複なし) で、encoding_rs のエンコーダはその全部を `Unmappable` で返す (自前の逆算が必要な根拠)。09 L1 に全 1,880 個の往復の単体テストを置く。
  - **副作用**: UTF-8 等で開いた文書中の U+E000〜U+E757 (別の用途の私用文字) も、Shift_JIS で保存すると外字のバイトとして書かれる (Windows の変換と同じ。表現不能として止めない)。
  - **対象外**: EUC-JP の外字領域 (0xF5A1〜0xFEFE) と ISO-2022-JP には WHATWG の写像が無く、復号の時点で不正バイト (§8.3 で保持) になる。EUC-JP / ISO-2022-JP で U+E000〜U+E757 を保存すると表現不能 (E3 の置換先 (b)) になる (encoding_rs は `Unmappable` を返す。実測)。
- **Shift_JIS の単独バイト 0xA0 / 0xFD〜0xFF**: Windows (CP932) は U+F8F0〜U+F8F3 に写すが、WHATWG では誤りで、encoding_rs は `Malformed` を返す (実測)。§8.3 の不正バイトとして保持し、文書は読み取り専用になる (無編集保存はバイト一致)。0x80 は encoding_rs も U+0080 に復号し、往復する。
- 往復コーパス (09 §4) に次を加える。期待は括弧内。(1) CP932 の外字 (0xF040 / 0xF07E / 0xF080 / 0xF09F / 0xF0FC / 0xF140 / 0xF580 / 0xF9FC) を含む Shift_JIS 文書 (区分 A: 無編集はバイト一致、外字の前後を編集して保存しても外字のバイトはそのまま、E2' の確認も E3 も出ない)。(2) 単独バイト 0xA0 / 0xFD〜0xFF を含む Shift_JIS 文書 (不正バイトとして読み取り専用、名前を付けて保存でバイト一致)。(3) CP932 互換文字の復号結果 (0x8160 / 0x8161 / 0x817C / 0x8191 / 0x8192 / 0x81CA / 0x815C / 0xFA55) を含む Shift_JIS 文書と、同じ文字を含む EUC-JP 文書 (0xA1C1 / 0xA1C2 / 0xA1DD / 0xA1F1 / 0xA1F2 / 0xA2CC / 0xA1BD / 0xFCFC) (往復する側: 区分 A でバイト一致)。(4) 互換文字の表の JIS 系の文字 (U+301C・U+2016・U+2212・U+00A2・U+00A3・U+00AC・U+00A6・U+2014・U+00A5・U+203E) と私用文字 U+E000 / U+E757 を含む UTF-8 文書 (Shift_JIS / EUC-JP / ISO-2022-JP への保存で E3 になり、`置換して保存` で互換文字の表どおりに置き換わって `counts.compat` に数えられ、置き換えた文書が保存先で往復すること。`¥` `‾` は ISO-2022-JP では表現不能にならない。U+E000 / U+E757 は Shift_JIS では外字 0xF040 / 0xF9FC で書かれ、EUC-JP / ISO-2022-JP では `?` になる)。(5) JIS X 0208 状態で改行する ISO-2022-JP 文書 (`1B 24 42 24 22 0A 1B 28 42`。FR-111 で ISO-2022-JP を指定して開くと 0x0A が不正バイト `<0x0A>` として見え、読み取り専用で、ISO-2022-JP での保存は拒否、UTF-8 等での保存は E3 でキャンセルだけ)。
- NFR-303 の文言はこの 4 区分と版ごとの範囲 (v0.1 は区分 A + 無編集保存を書かないこと、区分 B の保持方式は v0.3) を前提にする。ISO-2022-JP の例外文は「無編集はバイト一致、編集後はエスケープ選択の差を確認付きで許す (意味の一致)」(02 §13.4 (i))。
- **encoding_rs の ISO-2022-JP 対応範囲と自前実装の要否**: encoding_rs (WHATWG 準拠) のデコーダは `ESC ( B` / `ESC ( J` / `ESC ( I` / `ESC $ @` / `ESC $ B` のみを受け付け、JIS X 0212 (`ESC $ ( D`)・ISO-2022-JP-2 系・`SO` / `SI`・8 bit バイト・出力を挟まない連続エスケープを誤りとする。エンコーダは `ESC $ B` / `ESC ( B` / `ESC ( J` (¥ ‾ 用) だけを出し、半角カナを全角に変換し、JIS X 0212 を出力しない。v0.1 はこの範囲で **自前実装を持たない** (判定 (§8.2 手順 4') の ESC 検出と復号可能性検査、状態の引継ぎ、行単位の区分 B 比較は encoding_rs の API の上に書く薄い層で、`eve-encoding` の依存は encoding_rs / chardetng のまま)。受け付けない入力は不正バイトとして読み取り専用で開く (§8.3)。S-01 (a') の実コーパス (メール・旧 Windows ツール・JIS 規格書由来の文書) で「読み取り専用になるファイルの比率」を記録し、有意なら **デコーダのみ** 自前の寛容実装 (JIS X 0208 / 0212 / 半角カナの表は encoding_rs の EUC-JP デコーダへ `バイト | 0x80` で写して流用し、表を持ち込まない) を v0.2 以降で足す (OQ-138)。エンコーダは正規形を出す encoding_rs のままとする。

### 8.8 文字コードの変更 (FR-111)

- 「指定して開き直す」(`file.reopen_with_encoding`): dirty なら実行前に保存か破棄を求める (01 FR-111 AC-2。FR-107 と同じ 3 択で、保存を選んだら保存の成功後に開き直す。§9.7)。Document を破棄せず、同じ `DocumentId` で読込ジョブ (手順 2 の明示指定付き) を再投入し、新しい本文は **旧本文とは別に構築** する。読込中は `load_state = Loading` で **旧本文を表示し続け** (編集はできない)、完了時に本文・`TextFormat`・読み取り専用の理由 (`InvalidBytes` 等の内容由来のもの) を一括で差し替え、**履歴を消して新しい根を作り、`saved` = 根** (dirty = false) にする。**開き直しは Undo できない操作** とし、書式 (文字コード・BOM・改行の統計) と読み取り専用の理由は本文と一緒に置き換える (確定 (ユーザー回答))。理由: 本文だけを Undo で戻すと、文字コード・不正バイトの読み取り専用・保存点が本文と食い違い (例: Shift_JIS で開き直した後に Undo すると、不正バイトを含む旧デコード結果が編集可能になる。NFR-304 に反する)。dirty でないときだけ実行するので、失うのは保存済みの編集の Undo 履歴だけで、未保存の内容は失わない。完了時に「開き直したため Undo 履歴を消去しました」と 1 回通知する。外部変更の再読込 (§9.5) は Undo 可のまま (自分の編集を取り戻すため) で、書式も一緒に戻す (§7.2)。
  - 開き直しのキャンセル (`ui.cancel`)・失敗では、旧本文・旧 `TextFormat`・旧い読み取り専用の理由・履歴をそのまま保ち、`load_state = Ready` に戻す (エラーはメッセージで示す)。
  - 開く前の確認 (512 MiB 超・不正バイトの比率、`JobResult::NeedsConfirm`) は通常の読込 (§9.2) と同じ規則で出す。「中止」は上のキャンセルと同じ。明示指定で読むので文字コードの判定 (§8.2 手順 1 の BOM の判定 (指定した文字コードの BOM と一致すれば読み飛ばす) と手順 3〜8) とバイナリ・UTF-32 の BOM の確認は行わない (バイナリの確認から開き直した文書の `Binary` の理由は外れる)。
  - **読み取り専用の理由の差し替え** (開き直しと再読込 (§9.5) の両方の正本。§7.2 と §9.5 手順 5 はここを参照する): `FileAttribute` は新しい `FileMeta.readonly` で付け直し、`UserRequested` と `NoWritePermission` は保つ。内容由来の理由のうち `InvalidBytes` / `StatefulEncodingWithInvalidBytes` / `LongLine` は新しい読込の結果で付け直す。`Binary` と `LargeFileViewer` は、**開き直しでは** 新しい読込の結果で付け直し (バイナリの確認から開き直した文書の `Binary` は外れる)、**再読込では保つ** (明示指定の読込ではバイナリの判定をせず、`LargeFileViewer` の文書は `confirmed_large` と `read_only_requested` を立てて読み直すため、§9.5 手順 3)。再読込で付け直した理由は前後の値を `format_change` に持ち、Undo / Redo で本文と一緒に戻る (§7.2)。開き直しは Undo できない。
  - 保存ジョブの実行中は、その完了を待ってから dirty を判定する。
- 「指定して保存する」(`file.set_encoding`): 引数の `encoding` と `bom` から新しい format (`encoding` / `has_bom` だけを変え、他は元のまま) を作り、**今の format は変えずに** 保存ジョブにその format を渡す。`TextFormat` は **保存が成功したときだけ** 新しい format に確定し、失敗・キャンセル・`Conflict`・表現不能 (E3 の 3) で終わったら元のまま (メタデータの変更なので Undo の対象外、§7.2)。手順 0 (未編集なら書かない、§9.3) は適用せず、常に書く。表現不能のチェックは §8.6。
  - `bom` の扱い (既定値の正本は 07 の `file.set_encoding` の引数): 省略したときは **UTF-16LE/BE は BOM 付き、UTF-8 は今の文書の BOM の有無を保つ** (`file.write_bom` には依らない)。BOM を持たない文字コード (Shift_JIS / EUC-JP / ISO-2022-JP) では `bom` を **無視** する (`has_bom = false`。指定しても誤りにしない)。
  - 保存が成功して `TextFormat` を新しい format に確定したら、`roundtrip` (`RoundtripInfo`) は旧文字コードでの検出結果なので、そのままにしない: 新しい文字コードが UTF-8 / UTF-16 (区分 A) なら空 (`Summary { count: 0, .. }`) にし、それ以外 (区分 B / D の文字コード) なら、書いたバイト列は E1 の照合を通った代表バイト列なので同じく空にする (書いた内容を再エンコードすれば同じバイト列になり、無編集の再保存でバイト一致する)。`invalid_bytes` / `pua_escape` は保存の前提 (§8.3) により変わらない。旧 `roundtrip` を元に出していた `DecorationKind::LossyEncoding` の Decoration も消す。
  - 保存先: パスを持ち、読み取り専用でない文書は今のパスに上書き保存する (`Expect::Stamp(disk_stamp)`、§9.3)。無題の文書と読み取り専用の文書は、名前を付けて保存の流れ (§9.7 (a)) に新しい format を渡す。

### 8.9 対応しない文字コード

**確定 (ユーザー回答、[OQ-03](10-decisions.md#oq-03-mvp-の文字コード対応範囲))**: 文字コードの対応範囲は、v0.1 は FR-108 / 109 の 7 種、v0.3 は FR-110 で encoding_rs (WHATWG Encoding Standard) が扱うもので、これ以上広げない。次の表の文字コードには **対応しない**。需要が出てから自前実装を検討する (その場合は新しい OQ を起こす)。対応しない文字コードのファイルも **黙って壊さない** (P1): 下の「開いたときの挙動」は既存の判定 (§8.2)・不正バイトの保持 (§8.3)・読み取り専用の規則から決まるもので、追加の実装は UTF-32 の BOM の検出 (§8.2 手順 1) だけである。

| 文字コード | 対応しない理由 | 開いたときの挙動 (v0.1) |
|---|---|---|
| **JIS X 0213 系** (Shift_JIS-2004、EUC-JIS-2004、ISO-2022-JP-2004 / -3。第 3・第 4 水準漢字) | encoding_rs / WHATWG に無い。対応には符号表と往復の規則を自前で持つ必要がある | JIS X 0208 の範囲は Shift_JIS / EUC-JP / ISO-2022-JP として読める。第 3・第 4 水準の文字は不正バイト (Shift_JIS・EUC-JP) として §8.3 で保持され、文書は読み取り専用 (NFR-304)。ISO-2022-JP-2004 の `ESC $ ( Q` 等は §8.2 手順 4' の復号可能性検査を通らず、7 bit なので手順 6 で ASCII (UTF-8) として開く (エスケープシーケンスが `<ESC>` 等で見え、ASCII として往復する)。FR-111 で ISO-2022-JP を指定して開き直すと、復号できない箇所を不正バイトとして読み取り専用で開く |
| **UTF-32** (LE / BE、BOM 有無) | encoding_rs / WHATWG に無い。テキストファイルとしての流通が少ない | v0.1 は開く対象に含めない。BOM 付き (`FF FE 00 00` / `00 00 FE FF`) は §8.2 手順 1 で UTF-16LE の BOM より先に検出し、本文を読まずに `ConfirmAsk::UnsupportedBom { kind }` で「UTF-32 には対応していない」と案内して、FR-115 と同じ選択肢 (読み取り専用で開く / 文字コードを指定して開く / 中止) の確認を出す。BOM なしは NUL が多いので §8.2 手順 4 でバイナリと判定され、同じ確認になる。UTF-16LE の BOM の直後に U+0000 が来る文書は UTF-32LE と判定されるが、FR-111 の明示指定で UTF-16LE として開き直せる (明示指定は手順 1 に優先し、`FF FE` を UTF-16LE の BOM として読み飛ばして続く `00 00` を U+0000 として読む。§8.2 手順 2) |
| **UTF-7** | WHATWG が安全上の理由で除外 (XSS に使われた)。現在ほぼ使われない | 7 bit なので §8.2 手順 6 で UTF-8 (ASCII) として開く (`+ZeVnLIqe-` のような表記がそのまま見える)。ASCII として往復するのでバイト一致は保たれる |
| **ISO-2022-JP の補助漢字等** (ISO-2022-JP-1 の JIS X 0212 `ESC $ ( D`、ISO-2022-JP-2 の `ESC $ A` / `ESC $ ( C` / `ESC . A` 等、`SO` / `SI` を使う変種) | encoding_rs の ISO-2022-JP デコーダは受け付けない (§8.7 末尾)。寛容なデコーダの自前実装は OQ-138 で別途判断 | §8.2 手順 4' で ISO-2022-JP と確定せず、7 bit なので手順 6 で ASCII (UTF-8) として開く (手順 7 の chardetng には届かない。ASCII として往復する)。FR-111 で ISO-2022-JP を指定して開き直すと、復号できない箇所を不正バイトとして読み取り専用で開く (§8.3 の状態付き符号化の規則)。8 bit バイトが混在するものは手順 7 に届き、chardetng が ISO-2022-JP と推定すれば同じく読み取り専用で開く |
| その他 WHATWG に無い文字コード (EBCDIC 系、ISO-2022-KR / CN、HZ 等) | 同上 | 判定 (§8.2) の結果に従い、多くは `file.default_encoding` (既定 UTF-8) として不正バイトを含む文書 (読み取り専用) かバイナリになる (chardetng が 7 種外の文字コードを推定しても採らない、§8.2 手順 7 の (iii)) |

- 保存側の制約: これらの文字コードを「文字コードを指定して保存」(FR-111) の選択肢に出さない (encoding_rs のラベル一覧に無いので自然に出ない)。
- EUC-JP の JIS X 0212 (補助漢字) は **対応範囲内** (読める・エンコーダは出力しないので区分 B / C、§8.6 / §8.7) で、本表の対象ではない。CP932 の外字も対応範囲内で、`eve-encoding` の自前の逆算で書き戻すので区分 A (§8.7、確定 (ユーザー回答)、OQ-206)。不正バイトを含む ISO-2022-JP の文書は読めるが ISO-2022-JP では保存しない (§8.3、確定 (ユーザー回答)、OQ-207)。
- WHATWG にあるが v0.1 の 7 種に無い文字コード (windows-1252・GBK・Big5・EUC-KR 等) は v0.3 の FR-110 (明示指定) の対象で、本表の対象ではない。v0.1 で chardetng がこれらを推定した場合は採らず、`file.default_encoding` で開いて不正バイトを §8.3 で保持する (§8.2 手順 7 の (iii))。

---

## 9. ファイル I/O の論理手順

core は `FileSystem` Port ([02 §4.4](02-architecture.md#44-ports-core-が定義し外側が実装する-trait)) 越しにのみファイルに触れる。本章は **何を・どの順で・どの失敗をどう扱うか** を定め、OS 別手順 (`ReplaceFileW`、`rename`、ACL、`\\?\` パス) は 05 に委ねる。

### 9.1 Port に要求する操作 (05 への入力)

**`FileSystem` trait のシグネチャ・型名・エラー分類の正本は [02 §4.5](02-architecture.md#45-filesystem-port-正本) である** (05 は OS 別の実装規則、本書は「core がいつ・何を呼ぶか」を書き、いずれも trait を再定義しない)。本節は core が Port に **要求する操作と呼び出し時点** のみを列挙する。02 §4.5 の名称との対応を括弧で示す。

| # | core が要求する操作 | 02 §4.5 の対応 | 呼び出し時点 (本書) |
|---|---|---|---|
| F1 | メタデータ取得 (種別、サイズ、mtime、読み取り専用、`file_id`、`nlink`、リンク情報 (dangling 含む)、`fs_kind`) | `metadata() -> FileMeta` | §9.2 手順 2 (ジョブ)、§9.3 手順 0 / 5a (ジョブ)、§9.5 フォーカス復帰 (ジョブ、v0.2)、§9.6 |
| F2 | ストリーム読込 (チャンク、キャンセル確認可) | `open_read() -> ReadChunks` | §9.2 手順 3 |
| F3 | シンボリックリンク解決 (I/O を伴うため **ジョブ内でのみ呼ぶ**) | `canonicalize()` | §9.2 手順 2b (表示用パス)、§9.3 手順 5 (保存先、Port 内部) |
| F4 | ファイル同一性の識別子 | `FileMeta.file_id` (05 §5.4。取得できない FS のみ canonical path で代替) | 二重オープン判定 (§9.2 手順 2b)、保存直前の置換検知 (`FileStamp.file_id`) |
| F5 | 原子的保存 (バイト列ストリームを受け取り、fsync → 属性複製 → 置換 → ディレクトリ fsync を行う)。原子的置換ができない・意味が変わる場合は何も書かずに `AtomicUnsupported` を返す (一時ディレクトリ規則 (05 §4.4) に当たる場合は詳細 `InPlacePattern`、ハードリンクの確認は詳細 `Hardlink`) | `write_atomic(path, ReadChunks, SaveOptions) -> SaveReport` | §9.3 手順 5 |
| F6 | 直接上書き。core が乾式走査で表現不能が無いことと出力長 `len` を確定してから呼ぶ (§9.3)。経路は (1) `AtomicUnsupported` の確認で利用者が承認した後、(2) `InPlacePattern` のとき (確認なし)、(3) `file.save_strategy = "in_place"` (07 §5.2) なら core が最初から `method = InPlace` を選ぶ (05 §4.4)、の 3 つ。(2) と (3) は保存先 T が存在するときだけで、T が無ければ新規ファイルとして原子的保存の経路 (`Expect::Absent`) で書く | `write_in_place(path, ReadChunks, len, SaveOptions, CancelToken)` (`cancel` はジョブの `CancelToken`。見るのは 05 §4.4 手順 6 より前の段だけ) | §9.3 手順 5d (`InPlacePattern`、同じジョブ内) / 手順 4 (`save_strategy = "in_place"`) / 手順 7 (`AtomicUnsupported` の確認の後に再投入) |
| F7 | **所有者のみ権限での私的書込** (スワップ・履歴・状態ファイル。NFR-505) | `SaveOptions.private = true` + `create_dir_all_private()` | §10.2 (v0.2)。緊急退避 (§10.1) は eve-platform の Host 部品 (`std::fs` を直接使う) を eve-host が呼んで書くため Port を使わない (02 §11.2.1) |
| F8 | 削除 | `remove_file()` | スワップ・一時ファイルの後片付け |
| F9 | **保存中フラグの列挙** (置換途中のクラッシュ痕跡。フラグが示すパスの `.<name>.eve-tmp-*` / `.<name>.eve-bak-*`) | `inflight_for(path) -> Vec<InflightRecord>` (v0.1、1 パス分のフラグだけを読む)、`list_inflight()` (v0.2。02 §4.5)。一般の `read_dir()` は v0.4 | §9.2 手順 2 (**開くたびに** metadata の結果に依らずそのパスのフラグを調べる。v0.1、05 §4.6)、§10.3 (起動時の一斉走査は **保存中フラグ** が示すパスについてのみ。ディレクトリの列挙は行わない。v0.2) |
| F10 | 外部変更の比較キー | `FileStamp { mtime, len, file_id, hash }` (02 §4.5。05 §5.5 が計算規則) | §9.3 / §9.5 の `disk_stamp` / `external_modified` |
| F11 | 退避ディレクトリの作成と走査 (場所・名前・壁時計・pid は Port の実装が決める) | `create_recovery_dir()` / `list_recovery_dirs()` (v0.1。02 §4.5) | §9.7 (e) の「退避先に保存」(`WriteRecovery`)、§10.1 の「復旧」(`RecoveryScan`) |

要求する性質: (a) すべて同期 API、(b) `Send + Sync` (ジョブから呼ぶ)、(c) エラーは 02 §4.5 の構造体 `FsError { kind: FsErrorKind, detail: Option<FsErrorDetail>, source }` で、core は `kind` (と必要なら `detail`) で分岐する (`NotFound`、`PermissionDenied`、`Busy` = 共有違反・ロック (再試行は 05 §4.2)、`InvalidPath`、`DiskFull`、`ReadOnlyFs`、`Other`、保存の `Conflict { current: Option<FileStamp> }` = `SaveOptions.expected` 不一致 (`None` = ファイルが無い)、`AtomicUnsupported` = 原子的置換不可 (何も書いていない)、`SourceFailed`、`ReplacedMovedToBackup { backup }`、`BackupFailed`、`InPlaceFailed { backup: Option<FsPath> }` (事前バックアップを省いた (`SkipConfirmed`) ときは `None`))。キャンセルは `FsErrorKind` に含めず、ジョブの結果 `JobError::Cancelled` (02 §9.3) だけで表す。Windows の置換途中失敗 (`ERROR_UNABLE_TO_MOVE_REPLACEMENT_2`、元ファイルが退避名に移動済み) は 02 §4.5 の `ReplacedMovedToBackup { backup }` で退避パスを添えて返る (OS 側の検出は 05 §4.2)。

**原則 (ADR-0020 確定、02 §4.4 規則 2)**: core は `metadata` / `canonicalize` を含む **いかなる I/O もメインスレッドで行わない**。ネットワーク FS では `stat` 1 回が秒単位でブロックし得る (P3 違反)。§9.2 / §9.3 / §9.5 の手順はこの原則に従う。

### 9.2 読込 (FR-101 / 122 / NFR-104)

```
file.open(path)
  0. [main] パスを構文的に正規化 (`Paths::normalize`、02 §4.4。規則は 05 §3.2 の手順 1〜5。I/O なし)。既に同じ「指定パス」の Document があれば
           それをアクティブにして終了 (I/O なしで判定できる早期一致。FileId による確定判定は手順 3)
  1. [main] Document { load_state: Loading, read_only: ReadOnlyReasons::empty(), path } を作り、Frame を描ける状態にして **直ちにジョブ投入**
           (JobKind::Open、02 §9.3。編集できないことは load_state == Loading から導出し、読み取り専用の理由には入れない (02 §5.1))
           (ネットワーク FS で stat が数秒ブロックしても UI は止まらない: ADR-0020)
  2. [job]  metadata(path)。**結果に依らず** (T = path が存在してもしなくても) inflight_for(path) で保存中フラグのディレクトリ
           `recovery/inflight/<hash(path)>/` だけを 1 回調べる (O(1)。ディレクトリ全体は列挙しない。05 §4.6 / §7.5。v0.1)。返った記録 (Vec<InflightRecord>) から
           interrupted_save: Option<InterruptedSave { method: InterruptedMethod /*NewFile | Atomic | InPlace*/, backup: Option<FsPath>, tmp: Option<FsPath>, complete: bool }>
           を作る (02 §4.5 / §9.3)。記録から InterruptedSave への写像と MessageKey は下の「中断された保存の写像」の表が正本。
             v0.1 は pid の生存確認をしない (他インスタンスが保存中の記録でも報告する。誤報は害が小さい側、05 §4.6)。
             main は interrupted_save が Some なら表の MessageKey (04 §15.2) で場所を示して警告する。
             文書は通常どおり開く (空の新規文書として黙って開かない)。復旧の操作は v0.2 (§10.3)
           ・NotFound → JobResult::NewFile { job, interrupted_save, parent_missing } (main: 空の新規 Document、dirty=false として確定)。
             親フォルダが無ければ parent_missing にそのフォルダを入れ、main は `msg.open.parent_missing` で示す (フォルダは作らない。保存は §9.3 の ParentMissing になる。01 FR-101 AC-2)
           ・FileMeta.kind が File でない (Dir / Other) → JobResult::Failed { job, err: JobError::Fs(FsError) } (main: エラー表示、Document を閉じる)
           ・len > file.huge_file_warn_bytes (§11.2) かつ OpenOptions.confirmed_large でない → JobResult::NeedsConfirm { job, ask: ConfirmAsk::HugeFile { len } } を送って **ジョブを終了** する
             (len 以下でも、デコード後の見積り・読込中に伸びた分で閾値を超えれば手順 3 の「読込中のメモリ検査」で同じ確認を出す)。
             main は回答 (読み取り専用で開く / 中止。FR-127) を得たら、「読み取り専用で開く」なら OpenOptions { confirmed_large: true, read_only_requested: true, .. } で
             読込ジョブを再投入する (下の「確認からの再投入」)。確認中に io ワーカー (02 §9.1 の 1 本) を占有せず、他の文書の保存を止めない。回答待ちの状態は Document 側に持つ
  2b.[job]  meta.file_id と canonicalize(path) を取得 → JobResult::Identity { job, file_id, canonical }
           main: 既に同じ FileId の Document があれば、新 Document を閉じて既存をアクティブにし、ジョブをキャンセル
                 (二重に開かない。判定は FileId が正: ハードリンク・大文字小文字非区別に強い。05 §5.4。v0.1 の単一文書では、
                  既存 = 今の文書なので、新 Document を閉じて今の文書を残す (下の「置き換えの手順」))
           ※ 開くたびの検出は手順 2 のそのパスのフラグ (inflight_for) だけで行い、同じディレクトリの孤児ファイル (.eve-tmp-* / .eve-bak-*) を
             列挙することはしない。10 万ファイルのディレクトリ (ログ・node_modules) では開くたびに数百 ms かかる。置換途中の失敗は
             その場で退避パス付きで報告する (§9.3 手順 7)。起動時の一斉走査 (list_inflight) は v0.2 (§10.3)
  3. [job]  open_read → チャンク読み (既定 1 MiB) → 先頭 sniff (§8.2) → Decoder 作成
           ・OpenOptions.read_only_requested でも forced_encoding でもなく、sniff の結果が UTF-32 の BOM (§8.2 手順 1) →
             JobResult::NeedsConfirm { ask: ConfirmAsk::UnsupportedBom { kind } }、バイナリ (§8.2 手順 4) → JobResult::NeedsConfirm { ask: ConfirmAsk::Binary }
             を送ってジョブを終了 (FR-115。選択肢はどちらも 読み取り専用で開く / 文字コードを指定して開く / 中止)
           ・sniff 範囲 (64 KiB) を判定した文字コードで復号したときの不正バイトの比率が 1% を超え、OpenOptions.confirmed_invalid_bytes でも
             read_only_requested でもない → 本文の構築を始める前に
             JobResult::NeedsConfirm { ask: ConfirmAsk::ManyInvalidBytes { ratio, encoding, memory_factor } } を送ってジョブを終了
             (§8.3、FR-127 の「開く前に」。選択肢は 読み取り専用で開く / 文字コードを指定して開く / 中止)
           各チャンク: decode (不正バイトは §8.3 でエスケープ) → TextBuilder.append → EolStats 更新
             (EolStats は復号後のテキストで数える。チャンクが CR で終わるときはその CR を数えずに持ち越し、次のチャンクの先頭が LF なら crlf、
              それ以外なら cr として数える。ファイルの末尾で持ち越しが残っていれば cr。§8.2.1 のやり直しでは持ち越しも捨てる)
           ・UTF-8 以外: 再エンコード照合で roundtrip を記録 (§8.7、ストリーミング。v0.1 は件数 + 先頭 100 箇所)
           ・16 MiB 以下: 内容ハッシュをストリーミング計算 (05 §5.5 の比較キー)
           ・開く操作から 50 ms 以内に最初の JobResult::Progress { job, bytes, total } を送り、以後は **最大 10 Hz** (受信口に積まない。02 §9.2)
           ・CancelToken (02 §9.3) をチャンク毎に確認 (FR-122)
           ・**読込中のメモリ検査** (OpenOptions.confirmed_large でないとき。手順 2 の判定は metadata の len だけなので、読込中に伸びたファイル・
             デコードで膨らむファイルを見逃さないため): (i) sniff の直後に「デコード後の見積り」= len × 文字コードの倍率 (Shift_JIS・EUC-JP は 1.5、
             UTF-16 は 1.5、UTF-8 は 1) × (1 + 3 × sniff 範囲の不正バイトの比率) (不正バイト 1 つは PUA のエスケープで UTF-8 の 4 バイトになる、§8.3 / §11.7) を求め、
             huge_file_warn_bytes を超えれば本文の構築の前に止める。(ii) 各チャンクの後に「デコード後のバイト数 (構築中の本文の UTF-8 のバイト数)」と
             「読んだバイト数」の大きい方を huge_file_warn_bytes と照合し、超えたらその時点で止める。どちらも
             JobResult::NeedsConfirm { ask: ConfirmAsk::HugeFile { len } } を送ってジョブを終了し (構築中の本文は捨てる)、`len` には判定に使った値を入れる。
             回答と再投入は手順 2 の HugeFile と同じ (下の「確認からの再投入」)
           ・**読む量の上限**: 読込は metadata の len + 余裕 (max(1 MiB, len / 16)) で打ち切る (読込中に伸び続けるログ等を際限なく読まない)。
             打ち切ったら changed_during_load = true とし、disk_stamp には読込前の stamp を入れる (ディスクの今の内容と一致しないので、
             そのまま上書き保存すると手順 5a の照合で Conflict になり、読んだ分だけでファイルを黙って切り詰めることはない。05 §4.5)
           ・§8.2.1 の再判定が起きたら先頭からやり直し (1 回限り)
           ・読込前後の metadata を比較し、変化していれば Complete に `changed_during_load = true` を付ける (05 §4.5)
  4. [job]  JobResult::Complete { job, text: Text, format: TextFormat, meta: FileMeta, stamp: FileStamp, changed_during_load, detected: ReadOnlyReasons, interrupted_save }
  5. [main] Document.text = text; load_state = Ready; 読み取り専用の理由 (ReadOnlyReasons、§9.6) を下の「付く読み取り専用の理由の決め方」の表で付ける;
           disk_stamp = stamp; canonical = canonical (表示用は 05 §3.3 の簡略化パス); history 根を作る;
           interrupted_save が Some なら手順 2 の警告を出す; 購読者へ DocumentOpened; FR-123 の機能縮退判定 (§11.3)
```

**中断された保存の写像** (手順 2。記録 × T の有無 × バックアップの実在から `InterruptedSave` を作る規則の正本は 02 §4.5 の「中断の痕跡の写像」で、本表はそれを写し、main が出す MessageKey (04 §15.2) を定める): `backup` / `tmp` は実在するものだけを入れ (実在の確認は同じジョブの `metadata`)、1 つのパスに報告する記録が複数あれば表の上の行に当たるものを 1 つ選ぶ。`backup_recorded = false` は `in_place_backup = SkipConfirmed` (バックアップなしで上書きを承認した) の直接上書きの記録で、`backup` を持たない (§9.3 の「直接上書きの規則」(2)、05 §4.4)。

| 記録 (`method` / `backup_recorded` / `backup_complete`) | T (保存先) | バックアップの実在 | `interrupted_save` | MessageKey (04 §15.2) |
|---|---|---|---|---|
| `InPlace` / true / true | なし・あり | 最終名 (`state_dir/backup/` のバックアップ) が在る | `Some { method: InPlace, backup: Some(最終名), tmp, complete: true }` | `msg.open.interrupted_save` (T があれば上書きの途中で中断され壊れている可能性がある) |
| `InPlace` / true / false | なし・あり | 一時名 `<最終名>.tmp` が在る (バックアップを書き終える前に中断。上書きは始まっていない) | `Some { method: InPlace, backup: Some(<最終名>.tmp), tmp, complete: false }` | `msg.open.interrupted_save_incomplete` |
| `InPlace` / true / false | なし・あり | 一時名も無い | None (報告しない。記録を掃除する) | — |
| `InPlace` / false / false | なし・あり | — (作らない) | `Some { method: InPlace, backup: None, tmp: None, complete: false }` | `msg.open.interrupted_save_no_backup` |
| 上書きの可能性あり: `InPlace` / true / true で最終名が無い、上の行に無い不正な組合せ (`InPlace` / false / true 等)、解析できないフラグ | なし・あり | 最終名が無い (または記録から読めない) | `Some { method: InPlace, backup: None, tmp, complete: false }` (`tmp` は記録から読めて実在するものだけ。読めなければ None) | `msg.open.interrupted_save_no_backup` (直接上書きが中断して壊れている可能性がある側に倒す)。**記録を掃除しない** (フラグを残す) |
| `Atomic` | なし (NotFound) | 退避名 `.eve-bak-*` が在る (Windows の置換途中のクラッシュで T が退避名に移ったまま) | `Some { method: Atomic, backup: Some(退避名), tmp, complete: true }` | `msg.open.interrupted_save` (`{backup}` は退避名。元の内容は完全に残る) |
| `Atomic` | なし (NotFound) | 退避名が無い | None (報告しない。T は外部で消えたもので、保存の痕跡ではない。記録を掃除する) | — |
| `Atomic` / `NewFile` | あり | — | None (報告しない。T は置換・作成の前の版か後の版のどちらかで完全。記録を掃除する) | — |
| `NewFile` | なし (NotFound) | — | `Some { method: NewFile, backup: None, tmp, complete: false }` | `msg.open.interrupted_new` (新規作成の保存が完了しておらず、ファイルは作られていない) |

- 「記録を掃除する」は、開くジョブがその記録のフラグのファイル (`InflightRecord.flag`) を消すこと (一時ファイル・バックアップは消さない。02 §4.5)。ただし記録された一時ファイル・退避名 (`.eve-tmp-*` / `.eve-bak-*`) のどれかが実在する記録は、報告せずにフラグを残す (置換の成功後に退避名を消せなかった・一時ファイルを消せなかった等。v0.2 の起動時の走査の手掛かり。05 §4.6 の「フラグを消す条件」)。
- 報告した記録 (表の `Some` の行) は開くジョブが消さず、**同じ保存先への保存が成功したとき** に保存の後片付けで消える (02 §4.5 の「報告した記録を消す時機」)。それまでは開くたびに同じ警告が出る。
- 結果は T がなければ `JobResult::NewFile`、あれば `JobResult::Complete` の `interrupted_save` に入れる。

- **確認からの再投入** (`OpenOptions { forced_encoding, read_only_requested, confirmed_large, confirmed_invalid_bytes, pending_reasons }`、02 §9.3): 開くジョブは確認が要るたびに `NeedsConfirm` で終了し、main は回答に応じて次の `OpenOptions` で **先頭から** 読込ジョブを投入し直す。前の回答で立てたフラグと `pending_reasons` は引き継ぐ (2 つ目の確認が出ても 1 つ目の確認を繰り返さない)。「中止」はどの確認でも読込のキャンセルと同じ扱い (§9.2 の「置き換えの手順」(4)、起動時は無題の文書で開始)。本表の選択肢が正本で、04 §15.2 の文言はこれに合わせる。

| 確認 (`ConfirmAsk`) | MessageKey (04 §15.2) | 選択肢 → 再投入の `OpenOptions` | 付く読み取り専用の理由 (§9.6) |
|---|---|---|---|
| `HugeFile { len }` | `ui.confirm.open.huge` | 読み取り専用で開く (既定) → `confirmed_large = true`, `read_only_requested = true`, `pending_reasons += LargeFileViewer` / 中止 | `LargeFileViewer` |
| `UnsupportedBom { kind }` | `ui.confirm.open.utf32` | 読み取り専用で開く (既定) → `read_only_requested = true`, `pending_reasons += Binary` / 文字コードを指定して開く → 文字コードを尋ね (§14.6 の引数の一覧)、`forced_encoding = Some(enc)` / 中止 | 読み取り専用で開いたとき `Binary` |
| `Binary` | `ui.confirm.open.binary` | 同上 | 同上 |
| `ManyInvalidBytes { ratio, encoding, memory_factor }` | `ui.confirm.open.invalid_bytes` (`HugeFile` とは **別のキー**) | 読み取り専用で開く (既定) → `confirmed_invalid_bytes = true`, `read_only_requested = true`, `pending_reasons += InvalidBytes` (判定した文字コードが ISO-2022-JP なら `StatefulEncodingWithInvalidBytes`) / 文字コードを指定して開く → `forced_encoding = Some(enc)` (`confirmed_invalid_bytes` は立てない。指定した文字コードでも 1% を超えればもう一度確認する) / 中止 | `InvalidBytes` (ISO-2022-JP なら `StatefulEncodingWithInvalidBytes`。`LargeFileViewer` は付けない) |

  - `read_only_requested` は確認で「読み取り専用で開く」を選んだことを表し、開くジョブは以降の **バイナリ・UTF-32 の BOM・不正バイトの比率 (`ManyInvalidBytes`) の確認を出さずに続ける** (読み取り専用で開くことが決まっているので二重に確認しない。本文は判定した文字コード (バイナリ・UTF-32 の BOM では UTF-8) で復号し、不正バイトは §8.3 で保持する。NUL は `<0x00>` で表示、§9.6)。確認を省いた判定は、ジョブが理由として `Complete.detected` に入れる (下の表)。巨大ファイルの確認は `confirmed_large` で決まる。`--readonly` (`OpenRequest.readonly`) ではこの値を立てず、`open_initial` が最初の投入で `pending_reasons` に `UserRequested` を入れるだけで、確認は通常どおり出す。
  - `forced_encoding` は §8.2 手順 2 の明示指定で、文字コードの判定 (手順 1 の BOM の判定 (指定した文字コードの BOM と一致すれば読み飛ばし、一致しなければ本文のバイトとして扱う) と手順 3〜8) とバイナリ・UTF-32 の BOM の確認を行わない。巨大ファイルと不正バイトの比率の確認はフラグが立っていなければ出す (§8.8)。
  - `pending_reasons: ReadOnlyReasons` は確認の回答と `--readonly` で決まった理由を main が入れて運ぶもので、ジョブは読まずに引き継ぐ (`read_only_requested` の値だけでは、どの確認で立てたかが分からないため。理由の保持場所はここだけにする、02 §9.3)。

**付く読み取り専用の理由の決め方** (手順 5。02 §9.3 の「確認の回答と読み取り専用の理由」と同じ規則): 文書の `read_only` は `OpenOptions.pending_reasons` ∪ `Complete.detected` ∪ (`meta` から決まる理由) とする。

| 由来 | 理由 | 決め方 |
|---|---|---|
| 確認の回答・開く要求 (`pending_reasons`、main が入れる) | `LargeFileViewer` / `Binary` / `InvalidBytes` / `StatefulEncodingWithInvalidBytes` / `UserRequested` | 上の表で「読み取り専用で開く」を選んだときの理由。`--readonly` (`OpenRequest.readonly`、07 §18.3) は `UserRequested` |
| 内容 (`Complete.detected`、ジョブが入れる) | `InvalidBytes` / `StatefulEncodingWithInvalidBytes` / `LongLine` / `Binary` | `format.invalid_bytes > 0` で状態付きでなければ `InvalidBytes`、状態付き符号化 (ISO-2022-JP) なら `StatefulEncodingWithInvalidBytes` (§8.3。確認を経ずに開いた場合 (比率が 1% 以下・文字コードを指定して開いた) も付く、NFR-304)。閾値超の行があれば `LongLine` (§11.5)。`read_only_requested` でバイナリ・UTF-32 の BOM の確認を省いたときは、その判定が当たれば `Binary` |
| ファイル (`meta`、main が決める) | `FileAttribute` | `meta.readonly` (05 §5.2) |

- **メインスレッドの I/O はゼロ**: `stat` / サイズ確認 / canonical 比較はすべてジョブで行う。「開く」の体感は Loading 表示が即出る分だけ良くなり、小さいローカルファイルでは手順 2〜5 が数 ms で完了するため違いは見えない。読込ジョブは `io` スレッドで直列に実行する (02 §9.1)。
- **Partial (v0.3)**: 読込中に先頭部分だけを読み取り専用の Snapshot として表示する仕組み。構築中の rope とは別に先頭の Snapshot を作り、スクロールを読込済み範囲に制限し、`+行番号` や検索を保留し、`Partial` 状態の Document を扱う必要がある。**v0.1 では作らない** (`load_state` に `Partial` の値だけ予約)。v0.1 の 100 MB は「開く操作から 50 ms 以内に進捗を表示し、入力に応答し、キャンセルできる。全読込は 30 回計測の p50 1.0 s で判定し、最大値は参考値 (目安 1.5 s) として記録する」(01 NFR-104) で判定する (§11.3)。
- **閉じる手順**: 文書を閉じるとき (v0.1 は次の「置き換えの手順」で旧文書を閉じるとき、v0.2 からは `buffer.close` も) は (a) 実行中の **保存ジョブは完了を待つ** (閉じた後でも結果を受け取る。閉じた文書の保存の結果は 02 §9.3 の採否表のとおり採用し、成功なら何もしない)。保存が失敗したら、その保存ジョブに渡した Snapshot (`DocSnapshot`) から「退避先に保存」できるようにする: core は閉じた文書の `DocSnapshot` を **その保存ジョブの完了まで保持** し (緊急退避のスロットも保存の完了まで空にしない。§10.1)、失敗なら `msg.save.failed` 系の失敗の確認 (§9.7 (e) の表の MessageKey) を「退避先に保存 ★ / 破棄」の 2 択で出す (閉じた文書なので別名で保存・再試行は出さない。退避先に保存は `JobKind::WriteRecovery` に保持した Snapshot を渡す)。破棄・退避の完了で Snapshot とスロットを解放する (提案、OQ-199)。(b) 読込・検索・構文・スワップのジョブは **キャンセル** し、結果が届いても `DocumentId` が無効なので破棄する、(c) `Loading` 中の文書を閉じるときは読込ジョブをキャンセルして即閉じる (確認不要: 未保存変更は存在しない)、(d) dirty なら FR-107 の確認。ただし置き換え (下の「置き換えの手順」) の `AskUnsaved` で既に「破棄」を選んだ文書は **確認済みの印** (`discard_confirmed`。その確認の後に編集が無いこと (Revision が同じ) が条件) を付けて閉じ、FR-107 の確認を二重に出さない。
- **置き換えの手順 (v0.1 の単一文書。v0.2 は §9.8 の `file.open` / `file.new` の行のとおり置き換えず足す。01 FR-107 AC-3)**: v0.1 は表示も操作も 1 文書だが、文書の集合 (`EditorState.documents`) は集合のまま持つ。`file.open` / `file.new` は (1) 今の文書が dirty なら FR-107 と同じ 保存 / 破棄 / キャンセル を確認する (キャンセルなら何もしない。保存を選んだら保存が成功したときだけ (2) へ進み、保存が成功しなければ置き換えを中止する。§9.7)、(2) 新しい Document を集合に加えて上の手順 1〜5 で読み込み (`file.new` は無題の Document を作るだけ)、その間も旧文書を表示し続ける。**読込の間、旧文書は編集を受け付けない** (読み取り専用として扱う。`ReadOnlyReasons` には入れず「置き換えの読込中」であることから導出し、編集のコマンドは `msg.edit.read_only` で拒否する。(1) で保存・破棄を確認した後の編集が (3) で黙って捨てられるのを防ぐ。表示の文言・when 句の追随は 02 §5.1 / 04 §15.2)。(1) で「破棄」を選んだ場合は旧文書に確認済みの印を付け、(3) で閉じるときに FR-107 の確認を二重に出さない (上の「閉じる手順」(d))。(4) で置き換えを中止したら印を外し、編集の受付を戻す。(3) 新しい文書が `Ready` (または手順 2 の NewFile) になった時点で View の対象を切り替え、旧文書を「閉じる手順」で閉じる、(4) 読込が失敗・キャンセル (読込中の `ui.cancel` (`Esc`)。FR-122 AC-1 のキャンセル手段) されたら新しい Document を閉じ、旧文書をそのまま残す。手順 0 / 2b で開こうとしたファイルが今の文書と同じなら、何もせず「既に開いています」と示す。緊急退避 (§10.1) は集合のどの文書も対象にするので、置き換えの途中に異常終了しても旧文書の内容を失わない。
- `+行番号` / `path:行:桁` (FR-102) は v0.2 で入れ、Complete を待ってジャンプする (07 §18.2)。
- **読込中のメモリ**: チャンク + Decoder バッファ + 構築中 rope。全文 `Vec<u8>` を作らない。読込用の事前確保など **利用者のデータ量で決まる確保には `try_reserve`** を使い、失敗したら読込ジョブは `JobError::OutOfMemory { requested }` を返し、core は「メモリが足りないため中止した」(`msg.open.out_of_memory`、04 §15.2) と知らせてプロセスは続行する (OOM で abort する経路を減らす。安定版 Rust では確保失敗をフックできない)。
- **stdin** (FR-120, v0.3): `open_read` の代わりに stdin ストリームを渡す同じ経路。サイズ不明なので進捗は読んだバイト数のみ。
- **失敗** (v0.1): 読込ジョブが `JobResult::Failed` (途中の I/O エラーを含む) で終わったら、読めた部分は捨て、エラーをメッセージで示す。(i) 置き換えの途中 (上の「置き換えの手順」(4)) なら新しい Document を閉じて旧文書をそのまま残す。(ii) 起動時のパス指定 (`Editor::open_initial`、02 §5.4) の失敗・キャンセルなら、その Document を閉じて無題の文書で開始する。再試行の専用コマンドは持たず、利用者がもう一度 `file.open` を実行する。読めた部分だけを読み取り専用で開く選択肢 (理由 `LoadIncomplete`、§9.6) は v0.1 では出さない。

### 9.3 保存 (FR-104 / 105 / 106 / NFR-301 / NFR-107)

**版**: 本節の論理手順は OS 共通である。v0.1 で実装する Port の OS 別手順は Windows (`ReplaceFileW` 系、読み取り専用属性、シンボリックリンク・ジャンクション・NTFS のハードリンク。05 §4) だけで、Unix の手順 (rename + ディレクトリ fsync、モード・所有者の複製、他ユーザー所有の判定) は **v0.2 (macOS / Linux を出荷する版)** で実装する。下の手順と規則の Unix の記述は、その版の仕様として残す。

```
file.save(doc)  [main]
  0. dirty でなく、disk_stamp が Some (ディスク上のファイルから読んだ文書) なら、ジョブ (JobKind::Metadata { purpose: MetaPurpose::UnchangedSave }) で metadata を取り
     disk_stamp と照合し、一致していれば **書き込まず** 「変更はありません」と表示して終わる (結果を待つ間は §9.7 の `CheckingUnchanged`。
     不一致・失敗・待つ間の編集と取消しの扱いは §9.7 の全状態の遷移表)
     (disk_stamp が None の新規文書 (存在しないパスを開いた・無題) は手順 0 を飛ばし、未編集でも Expect::Absent で書く。FR-101)
     (無編集の保存でレガシー文字コードの重複符号化バイト列が変わる事故を防ぐ。NFR-303 の主ケースを文字コードによらず満たす。
      どの保存操作に手順 0 を適用するかは下の「手順 0 の適用対象」の表。OQ-116 で確定 (ユーザー回答)、§23)
  1. 読み取り専用の文書 (§9.6。`doc.read_only` が真) では file.save は **無効** (when `!doc.read_only`、07 §11.6)。キーでも実行されず、
     パレットでは淡色で、選べない理由として「名前を付けて保存 (file.save_as)」を案内する。v0.1 で読み取り専用の文書を保存する手段は名前を付けて保存だけ (§9.7 (a))。
     FR-116 AC-3 の「名前を付けて保存」の案内は、上書き保存が PermissionDenied で失敗した時点 (手順 7) の確認 `ui.confirm.save_failed` で出す
     (その時点で文書に NoWritePermission が付き、以後 file.save は無効になる)。
     v0.2: 理由が FileAttribute で解除済みなら「読み取り専用を解除して保存」(`ui.confirm.save.read_only`) の確認を経て、SaveOptions に属性の一時解除を含めて投入する (FR-116)
  2. 保存前フック (v0.4: 末尾空白除去等の設定) を Transaction として適用
  3. history.seal(); snapshot = text.snapshot(); node = 履歴上の現在位置 (§7.1 の NodeId)
  4. ジョブ投入: JobRequest { id, doc, base: revision, kind: JobKind::Save { path, snapshot, format, node, opts, method } } (02 §9.3。id は JobSpawner が採番する。
     Port はジョブの種別に持たせず、run_job が JobPorts から取る)
       format: 文書の format (file.set_encoding・E3 の「UTF-8 で保存」では新しい format。保存が成功するまで文書の format は変えない、§8.8)
       opts: SaveOptions { expected, in_place_backup: Required, fsync, private: false, preserve, hardlink: HardlinkPolicy (既定 Ask、07 §5.2),
                           backup: None (v0.3 の FR-119 の予約。v0.1 / v0.2 は常に None) }
       expected: 既存のファイル → Expect::Stamp(disk_stamp)。新規ファイル → Expect::Absent。上書きを確認済み (§9.5 の「上書き」) → Conflict の `current` から決める (Some(stamp) → Stamp(stamp)、None (不在) → Absent)。
                 Expect::Any (照合しない) は v0.1 では使わない (02 §4.5 の予約)
       method: Atomic (既定) | InPlace (AtomicUnsupported の確認で「直接上書き」が承認された再投入、ハードリンクの WriteInPlace、
               または file.save_strategy = "in_place" (07 §5.2) のとき core が最初から選ぶ (05 §4.4。確認なし、乾式走査は行う))。
               save_strategy = "in_place" と一時ディレクトリ規則 (InPlacePattern) は **保存先 T が存在するときだけ** 直接上書きにする。
               T が無い (disk_stamp が None の新規ファイル、Expect::Absent) ときは method = Atomic の新規ファイルの経路で書く
     ※ format.encoding.is_stateful() (ISO-2022-JP) ならジョブは行ごとにエンコーダを閉じ、1 行の内部ではエンコーダと照合用デコーダの状態をチャンクをまたいで引き継ぐ (§8.6 E6、OQ-162 で確定)
     Document.saving = Some(job_id)。保存中も編集可 (02 §9.3)。同じ文書の保存要求が重なったら前の完了を待ってから最新 snapshot で再実行する (キューは深さ 1)。
     再実行の Expect::Stamp には **直前の保存結果の `report.new_stamp`** (= 手順 7 で更新した disk_stamp) を使う
     (前の保存の要求時点の disk_stamp を使うと、自分の保存を外部変更と誤って Conflict にする)。
     **前の保存が失敗したら、積んだ保存要求は破棄する** (再実行しない)。前の保存が `AfterFailure` (§9.7 (e)) に進んだ場合は、その確認の選択 (再試行・別名で保存・
     退避先に保存・中止) だけで続きを決める (失敗の原因 (Conflict・表現不能・共有違反等) を利用者が見る前に、同じ条件の保存を黙って投入し直さない。
     確認で保存を続ける選択をしたときは、その時点の最新 Snapshot で投入するので、積んだ要求の内容も含まれる)。`Failed { Cancelled }` で中止したときも破棄する
     (文書は dirty のままなので、利用者は file.save で保存し直せる)。遷移は §9.7 の全状態の遷移表
     ※ 外部変更の事前チェックはメインスレッドでは行わない (I/O 禁止)。チェックはジョブ内 (手順 5a) で行い、
        不一致なら書き込み前に Conflict として返す。ユーザーが §9.5 で「上書き」を選んだら expected を現在値に更新して再投入する
     ※ v0.1 で元の文字コードが UTF-8 以外かつ roundtrip.count ≥ 1 なら、投入前に §8.6 E2' の確認を出す (承認後に投入)
  [job]
  5. method = Atomic のとき write_atomic(path, エンコード済みチャンクのストリーム, opts):
       a. Port が metadata を取り opts.expected (`Expect`) と照合 (05 §4.1 手順 0b)。不一致 → FsError { kind: Conflict { current: Option<FileStamp> } } を返して終了 (何も書かない。`current` は今の stamp で、None = ファイルが無い)
       b. チャンクを順に encode + 照合 (§8.6 E1、v0.3 は E2 も)。表現不能を 1 件でも検出したら **その時点で書込を止める** (ストリームが `ChunkError::Unmappable` を返し、
          Port は一時ファイルを破棄して `SourceFailed` を返す)。ジョブは書込なしの走査 (encode + 照合) だけを終端まで続けて位置を集め、JobError::Unmappable { positions, total } を返す
          (無駄な I/O を避け、空き容量不足 (DiskFull) で表現不能が隠れないようにする)。状態付き符号化は最後に last = true で呼び、終端のエスケープ (ESC ( B) まで書く (E6)
       c. 書き終えたら Port が fsync → 属性複製 → **置換の直前に metadata を取り直して opts.expected と再照合** (書いている数秒の間に他のプログラムが保存した場合を検出。
          不一致なら一時ファイルを消して Conflict。残る競合の窓は置換の 1 回分だけ。05 §4.1) → 原子的置換 → (Unix) ディレクトリ fsync (ADR-0016、05 §4)
       d. Port が FsError { kind: AtomicUnsupported, detail: Some(InPlacePattern) } を返した (一時ディレクトリ規則、下の「一時ディレクトリ直下の一時ファイルの保存」) →
          **確認を出さずに同じジョブで** 手順 5' (乾式走査 → write_in_place) へ進む (02 §4.5、OQ-34)。他の AtomicUnsupported はそのまま main へ返す (手順 7)
  5'. method = InPlace (承認済み、または save_strategy = "in_place")、または 5d のとき:
       a. **乾式走査 (必須)**: Snapshot 全体を、保存と同じエンコーダで encode + 照合だけ行い、書き出さない (`SaveEncoder::dry_scan`、下の「乾式走査と出力長」)。
          表現不能が 1 件でもあれば **Port を呼ばずに** JobError::Unmappable { positions, total } を返す (元ファイルに一切触れない。§8.6 E3 へ)
       b. 乾式走査で確定した出力バイト長 len を渡して write_in_place(path, 同じエンコーダのストリーム (2 回目), len, opts, cancel) を呼ぶ (cancel はジョブの CancelToken) (02 §4.5、05 §4.4 手順 0〜9)。
          05 §4.4 の手順 6 (元ファイルへの書き始め) 以降は CancelToken を見ず、本書込のエンコーダは `Aborted` を返さない (5d から来た場合も同じ。手順 7 の `Cancelled` の箇条)
  6. JobResult::Saved { job, node, report: SaveReport { method, new_stamp, warnings }, canonical, meta } / JobResult::Failed { job, err: JobError }
  [main]  (採否は 02 §9.3 の採否表が正本 (§17.2)。Revision が進んでいても採用する)
  7. 成功: history.saved = node; disk_stamp = report.new_stamp (置換の後の T の FileId が書いた一時ファイルのものと違えば、Port は書いた内容の stamp を入れて警告を積む。
     ディスクの T とは一致しないので次の保存は照合で Conflict になり、外部変更として扱われる。05 §4.1 手順 7); path 確定 (新規ファイル・名前を付けて保存の場合);
     canonical = Saved.canonical; file_id = Saved.meta.file_id; 名前を付けて保存では読み取り専用の理由 (FileAttribute / NoWritePermission、§9.6) を Saved.meta から付け直す; dirty 再計算; DocumentSaved 通知; スワップを削除 (§10.2)
     report.method == InPlace なら `info.save.in_place` (04 §15.2) のメッセージで直接上書きで保存したことを 1 回示す (ステータスの項目にはしない。05 §4.4)。report.warnings (権限複製失敗、EscapeMerge §8.3 等) をメッセージで提示
     失敗: Document は dirty のまま。JobError と FsError.kind (と detail) で分類し、**§9.7 (e) の「保存失敗の後の選択肢」の表** (MessageKey・選択肢・次の状態・後続処理) に従って提示する。
       分類の要点 (選択肢は §9.7 (e) の表が正本):
       - JobError::Unmappable → §8.6 E3 の選択肢
       - Conflict (外部変更) → §9.5
       - AtomicUnsupported + detail Hardlink { nlink } (HardlinkPolicy::Ask) → 選んだ HardlinkPolicy を SaveOptions.hardlink に入れて再投入する (WriteInPlace は method = InPlace)
       - AtomicUnsupported (上記以外: 他ユーザー所有、ディレクトリに書込権限なし、FS が非対応、Windows で ReplaceFileW が元ファイルを変えずに失敗した) →
         `file.atomic_save_fallback` (下の「ハードリンク…」) が ask なら確認を出す (既定の選択肢は「別名で保存」)。ADR-0016 のフォールバックは利用者の承認を要し、
         承認後に method = InPlace で再投入する (手順 5'。乾式走査を必ず行う)。ReplaceFileW の失敗で MoveFileExW などの別の置換手段へ切り替えることはしない (02 §4.5)
       - BackupFailed (in-place の事前バックアップに失敗、または in-place の保存中フラグを書けない (detail = InflightFlag)。**何も書いていない**) →
         承認されたら SaveOptions.in_place_backup = SkipConfirmed で method = InPlace を再投入 (フラグを書けなくても SaveWarning::InflightFlagNotWritten で続ける)
       - InPlaceFailed { backup } (in-place の書込途中の失敗。元ファイルは壊れている可能性があり、ディスク上の内容は disk_stamp と一致しない) → バックアップのパスを必ず示す
         (backup が None = バックアップなしで上書きを承認した場合は、バックアップが無いことを示す)。
         同じパスへの再試行は Expect の照合で Conflict になるので出さず、名前を付けて保存へ誘導する
       - JobError::OutOfMemory { requested } (利用者のデータ量で決まる確保の try_reserve の失敗。02 §11.2.2。プロセスは続ける) →
         保存では、エンコーダのバッファの確保に失敗したストリームが ChunkError::OutOfMemory { requested } を返し、run_job が JobError::OutOfMemory に写す (02 §4.5)。
         原子的保存 (手順 5) では Port が一時ファイルを消して SourceFailed を返し、元ファイルは無傷 (分類できない失敗として `msg.save.failed` で示す)。
         乾式走査 (手順 5'a) で起きたら Port を呼ばずに返す (元ファイルに触れない)。直接上書きの本書込 (手順 5'b) で返ったときは Port が書込を止めて
         InPlaceFailed { backup } を返し、上の InPlaceFailed の箇条で扱う (JobError::OutOfMemory にはしない)。
         読込 (§9.2) の確保の失敗は JobError::OutOfMemory で `msg.open.out_of_memory` (04 §15.2) を示し、文書は開かない
       - Busy (Windows 共有違反) → Port 側の再試行 (方針・回数は 05 §4.2) 後も失敗。「他のプログラムが使用中」と表示する
         (原子的置換 (`ReplaceFileW`) の共有違反は、再試行が尽きて元ファイルが無傷なら Busy ではなく AtomicUnsupported で返り、上の確認から直接上書きへ進める。
          Busy になるのは直接上書きで元ファイルを開けない場合などに限る。05 §4.2)
       - ReplacedMovedToBackup { backup } (Windows の置換途中失敗、02 §4.5 / 05 §4.2) → **元ファイルは退避名に移動済み**。退避パスを必ず表示し、名前を付けて保存へ誘導する。
         退避ファイルを元の名前に戻す操作は v0.1 では提供しない (確定 (ユーザー回答)。利用者が手で戻す。戻す操作の提案は v0.2、§10.3)。
         クラッシュで報告できなかった場合、開く時の検出は v0.1 (§9.2 手順 2、05 §4.6)、起動時の一斉走査は v0.2 (§10.3)
       - PermissionDenied → 文書に NoWritePermission を付け (§9.6)、名前を付けて保存を案内する (FR-116 AC-3)
       - DiskFull / ReadOnlyFs → 名前を付けて保存、または緊急退避ディレクトリへの保存 (「退避先に保存」) を提示 (データを失わせない)
       - NotFound + detail ParentMissing (保存先の親フォルダが無い。開いた後に消された、存在しないフォルダのパスを開いた (`msg.open.parent_missing`) 等) →
         **フォルダは作らない**。フォルダが無いことを示して名前を付けて保存を案内する
       - NotFound + detail DanglingLink { target } → リンク先の絶対パスを示して名前を付けて保存を案内する (v0.1、下の「シンボリックリンク」)
       - JobError::Cancelled (保存の待ちを利用者が取り消した、§9.7) → 書かなかった旨をメッセージ (`info.save.aborted`) で示す。main は取り消すとき常に取消し (JobSpawner::cancel) を送り、
         ジョブの段を問わない。CancelToken を見るのはジョブ側で、原子的保存の `ReplaceFileW` の開始以降と直接上書きの 05 §4.4 の手順 6 (元ファイルへの書き始め) 以降は
         取消しを見ずに完了まで進み、`Saved` (または失敗) を返す (05 §11.5 と同じ規則)。`Cancelled` を返すのはその前の段 (照合・一時ファイルへの書込・乾式走査・事前バックアップ) だけ。
         手順 5d で原子的保存のジョブが一時ディレクトリ規則により直接上書きへ切り替わった場合も同じで、本書込 (手順 5'b) のエンコーダは `Aborted` を返さない (`Aborted` を返すのは置換・本書込より前の段だけ)
```

- **手順 0 の適用対象** (未編集なら書かない規則。本表が正本で、02 §9.3 の「未編集の上書き保存」はこれを要約する): 手順 0 は「同じファイルに、同じ format で書く」保存にだけ適用する。

| 操作 | 保存先 | 書く format | 手順 0 |
|---|---|---|---|
| `file.save` | 今のパス | 文書の format | **適用** (未編集で `disk_stamp` と一致すれば書かない) |
| `file.save_as` | 今の文書と同じファイル (§9.7 (a) の 1) | 文書の format | **適用** (上書き保存と同じ。読み取り専用の文書は (a) の 1 のとおり拒否) |
| `file.save_as` | 別のファイル (§9.7 (a) の 2〜6) | 文書の format | 適用しない (未編集でも書く) |
| `file.set_encoding` | 今のパス、または名前を付けて保存の流れ (無題・読み取り専用、§8.8) | 新しい format | 適用しない (未編集でも書く。同じファイルを指しても format が違うので書く) |
| `AskUnsaved` の「保存」(FR-107) | 今のパス (無題・読み取り専用なら `AskName`) | 文書の format | 適用 (dirty なので実際には常に書く) |
| (v0.2 以降の書式を変えて保存するコマンド) | — | 新しい format | 適用しない (`file.set_encoding` と同じ) |

- **手順 0 と dirty の関係**: dirty は履歴ノードの位置だけで決まる (§7.4、02 §5.1)。format の変更 (`file.set_encoding`) は保存の成功で確定し、それまで文書の format は変えないので、「未保存の書式の変更」という状態は無く、dirty に format を含めなくてよい。未編集の文書を `file.set_encoding` で書いた後も `saved` = 今のノードで dirty = false のまま。
- **原子的保存の論理要件** (OS 実装は 05): (a) 同じディレクトリに一時ファイルを作る (別ボリュームへの rename は非原子的)、(b) 一時ファイルは **所有者のみ** 権限で作成し (Windows は所有者のみの DACL、05 §4.1)、置換直前に元の権限・属性を複製する (FR-106)、(c) 置換は既存ファイルを上書きする操作で、**Windows (`ReplaceFileW`) は「途中失敗時に元ファイルが退避名で残り、回復可能」** (原子性は保証されない)、**Unix (`rename`、macOS / Linux を出荷する版) は原子的**、(d) 失敗時に一時ファイルを残さない。成功後の不変条件は INV-IO1 (OS 別の主張範囲を §20 に明記)。
- **直接上書き (in-place) の規則** (FR-105 AC-2。OS 別の手順は 05 §4.4、型は 02 §4.5): 承認後の in-place も、一時ディレクトリ規則 (`InPlacePattern`) による自動の in-place (下記) も、`file.save_strategy = "in_place"` による in-place も同じ手順で書く (自動の 2 つは保存先 T が存在するときだけ。手順 4)。順序は 05 §4.4 の手順のとおりで、元ファイルに書き始めるのは事前バックアップと領域の確保の後 (下の (6)) である。(0) **乾式走査は必須** (core のジョブが Port を呼ぶ前に行う。手順 5'a): Snapshot 全体を encode + 照合して、表現不能の有無と出力バイト長を確定する。表現不能があれば元ファイルに一切触れずに §8.6 E3 の選択肢へ進む。(1) **書き始める前の照合**: Port が metadata を `SaveOptions.expected` と照合し、不一致なら何も書かずに `Conflict`。(2) **保存中フラグ** (05 §7.5) を書く (`method = InPlace` とバックアップのパスを記録する。ただし `in_place_backup = SkipConfirmed` (バックアップなしで上書きを承認した) ではバックアップを作らないので、フラグに `backup` を記録しない (`InflightRecord.backup_recorded = false`。正本は 05 §4.4。開くときは §9.2 の写像で `msg.open.interrupted_save_no_backup` になる)。in-place の書込中もフラグは立っており、成功で消し、`InPlaceFailed` では残す。書けなければ何も書かずに `BackupFailed` (detail = `InflightFlag`) を返し、(4) の失敗と同じ確認に進む)。(3) 元ファイルを切り詰めずに、**他のプロセスの書込を禁じる共有モードで** 開き (Windows は `FILE_SHARE_READ` のみ。05 §4.4)、開いたハンドルで再照合する (不一致なら `Conflict`)。(4) **事前バックアップは必須**: (3) で開いたハンドルから元の内容を読んで state_dir の `backup/` に書き、fsync する (照合した内容とバックアップが同じものであることを、別に開き直さずに保証する)。失敗したら書かずに `BackupFailed` を返す (利用者が「バックアップなしで上書き」を承認したときだけ省略し、そのときの `InPlaceFailed` の `backup` は `None`)。(5) 出力長 `len` が元より長ければ、書く前に元の末尾より後ろの領域を確保する。確保できなければ何も変えずに `DiskFull` を返す。(6) 先頭から上書きし、(7) 長さを `len` に合わせて fsync する。(8) 書込途中の失敗は `InPlaceFailed { backup }` で返し、core はバックアップのパスを示す。本書込でエンコーダが乾式走査と違う出力長・表現不能を返した場合も `InPlaceFailed` である (下の「乾式走査と出力長」)。表現不能と容量不足が書く前に分かるので、in-place でも「書いている途中で表現不能に気付いて元ファイルが半端に壊れる」経路と「容量不足で元ファイルが壊れる」経路は残らない (コピーオンライトの FS で確保後に上書きが失敗する場合は (4) のバックアップが安全網)。
- **乾式走査と出力長** (02 §4.5 の `write_in_place(path, data, len, opts, cancel)` の `len`): 保存のエンコーダ (`SaveEncoder`、eve-encoding / eve-core。Snapshot と format から作り、`ReadChunks` を実装する) は次の 2 つを持つ。(a) `SaveEncoder::dry_scan(&snapshot, &format, cancel) -> Result<DryScan { out_len: u64 }, Unmappable { positions, total }>`: 書き出さずに全文を encode + 照合し (BOM と ISO-2022-JP の終端エスケープを含む)、出力の総バイト数を数える。(b) `SaveEncoder::new(snapshot, format)`: 同じ規則で 2 回目のエンコードをチャンクで供給する `ReadChunks`。ジョブは (a) の `out_len` を `len` として Port に渡し、(b) を `data` に渡す。同じ Snapshot と format からの 2 回のエンコードは同じバイト列になる (エンコーダは決定的。チャンクの区切り方・キャンセル確認の回数によらない)。これを 09 L1 のプロパティテストとして置く: 任意の Snapshot (チャンクの分割を任意に変えたものを含む) と format について、`dry_scan` の `out_len` と表現不能の判定が、`SaveEncoder::new` の出力を連結したバイト列の長さ・判定と一致し、2 回の `SaveEncoder::new` の出力が同じバイト列になる。本書込 (2 回目) が `out_len` と違う長さ・表現不能を返したら (不変条件違反) ストリームが `ChunkError` を返し、Port は書込を止めて `InPlaceFailed { backup }` を返す (02 §4.5、05 §4.4 手順 8。書き始めた後なので元ファイルは壊れている可能性があり、バックアップのパスを示す)。`ReadChunks` trait (02) に長さのメソッドは足さない。乾式走査のコストは encode 1 回分 (照合を含めて 2 回分) で、メモリはチャンク 1 つ分。キャンセルはチャンク毎に確認する。
- **シンボリックリンク** (FR-106): Port がジョブ内で `canonicalize` した **リンク先** に保存する (リンクをファイルで置き換えない)。一時ファイルはリンク先のディレクトリに作る。リンク先が存在しない (ダングリング) 場合の確認と選択肢は [05 §4.1](05-platform.md#41-共通手順-adr-0016-の具体化) の規則に従う (「リンク先に作成 (リンクを維持)」/「キャンセル」。リンク自体を通常ファイルで置き換えない)。core は `FileMeta.link` の dangling を見て確認を出し、承認後に保存ジョブを投入する。**v0.1 はダングリングのリンクを確認なしのエラー** とする: Port は書かずに `FsErrorKind::NotFound` (詳細 `DanglingLink`。05 §4.1) を返し、core はリンク先の絶対パスを示して「名前を付けて保存」を案内する。確認してリンク先に新規作成する UI は v0.2 で入れる。循環 (ELOOP) は `InvalidPath` として報告する。
- **ハードリンク (`nlink > 1`)、他ユーザー所有、ディレクトリに書込権限なし**: 原子的置換が意味を変える/不可能なケース。Port は何も書かずに `AtomicUnsupported` を返し、core は利用者に選ばせる (手順 7)。ハードリンクは `SaveOptions.hardlink: HardlinkPolicy` の 3 値 (`Ask` (既定。確認を出す) / `BreakLink` (原子的置換で書き、このパスだけが新しい内容になる) / `WriteInPlace` (直接上書きで全リンクを新しい内容にする。乾式走査が前提)) に従う (OQ-73、確定 (ユーザー回答)。設定キー `file.hardlink_policy` は 07 §5.2)。他ユーザー所有・ディレクトリに書込権限なし等の場合 (どの条件で `AtomicUnsupported` になるかは 05 §4.4 の表が正本) は、確認を出すかどうかを設定 `file.atomic_save_fallback` (07 §5.2。v0.1 は既定 `ask` の定数、公開は v0.2) で決める: `ask` = 手順 7 の確認 (`ui.confirm.save.atomic_unsupported`)、`fail` = 確認を出さずに保存の失敗として報告する。どちらの値でも、承認なしに直接上書きへ進むことはない (ADR-0016)。例外は一時ディレクトリ規則 (`InPlacePattern`、下記) で、Port が `AtomicUnsupported { InPlacePattern }` を返し、core が確認なしで乾式走査から直接上書きへ進む (手順 5d)。この設定の対象外である。
- **大きいファイルの保存時間** (NFR-107): エンコードはストリーミングで、Snapshot のチャンクをそのまま書くため追加メモリはバッファのみ。fsync コストが支配的。
- **読み取り専用属性を持つファイルを保存** (FR-116、v0.2): ユーザーが明示的に「読み取り専用を解除して保存」を選んだ場合のみ、Port に属性変更 (置換の直前に外し、置換の後に付け直す、05 §4.2) を含めて依頼する。v0.1 は読み取り専用属性のファイルを書かない (§9.6)。開いた後に属性が付いた・権限が変わった場合は手順 5 の `PermissionDenied` になり、文書に `NoWritePermission` を付ける。
- **新規ファイル** (FR-101): `SaveOptions.expected = Expect::Absent` (「存在しないこと」を期待する)。保存時点で同名ファイルが存在していたら (起動後に他者が作成) `Conflict` として扱う。既存のファイルの変更とは別の文言 (`ui.confirm.save.target_exists`、04 §15.2) で示す (§9.5)。
- **一時ディレクトリ直下の一時ファイルの保存**: 呼び出し元が同一 inode を期待することがあるため、**一時ディレクトリ (`$TMPDIR` 等) の直下 (サブディレクトリを含まない) でファイル名が 05 §4.4 のパターン (`*.tmp` 等、`file.in_place_patterns`) に一致し、そのファイルが既に存在する** 場合は (存在しなければ同一 inode の期待も無いので、新規ファイルとして原子的保存の経路 (`Expect::Absent`) で書く)、Port は書かずに `AtomicUnsupported` (詳細 `InPlacePattern`) を返し、保存ジョブは **確認を出さずに同じジョブで** 乾式走査 → `write_in_place` に進む (手順 5d / 5'。OQ-34、確定 (ユーザー回答))。直接上書きの手順 (乾式走査・事前バックアップを含む) は承認後の in-place と同じで、`SaveReport.method = InPlace` で結果を知らせる。
- **名前を付けて保存**: 保存先の決め方と確認は §9.7 (a)。新パスに保存成功後、Document の path / file_id / canonical / disk_stamp / 言語判定を更新し (file_id と canonical は `JobResult::Saved` の `meta` / `canonical` から取る。main は I/O しない)、読み取り専用の理由を判定し直し (§9.6)、旧パスのスワップを削除。
- **予約**: `SaveOptions.backup` (FR-119 の「保存前の版を 1 世代残す」) は v0.3 の予約で、v0.1 / v0.2 は常に `None` を渡す (§10.4)。直接上書きの事前バックアップ (`in_place_backup`) とは別物。
- **全保存** (FR-104、v0.2。複数文書と同時): dirty な Document を順にジョブ投入。結果は個別に報告。

### 9.4 Windows 固有の論理上の扱い (実装は 05)

| 事象 | core の扱い |
|---|---|
| 共有違反 (他プロセスがファイルを開いている) | `FsError::Busy`。保存は Port の再試行 (待ちの合計 2.55 s、05 §4.2) 後にエラー提示。ただし `ReplaceFileW` の共有違反 (相手が `FILE_SHARE_DELETE` なしで開いたまま) は再試行しても解消しないことが多いので、再試行が尽きて元ファイルが無傷なら `AtomicUnsupported` として §9.3 手順 7 の確認 (直接上書き / 別名で保存 / キャンセル) に進める (05 §4.2)。読込は読み取り共有で開けることが多いので通常成功 |
| 長いパス (`MAX_PATH` 超) | Port が `\\?\` 前置で扱う (NFR-404)。core はパスを `FsPath` (`PathBuf` の newtype、02 §4.4) として不透明に扱い、表示用の短縮だけ行う |
| 読み取り専用属性 | `FileMeta.readonly` として受け取り FR-116 に使う。保存時は FR-106 で維持 |
| ファイル名の大文字小文字非区別 | 「同じファイルを二重に開かない」判定 (§9.2 手順 2b) は **`FileId`** で行う (05 §5.4。canonical path 比較は表示名の決定にのみ使う) |
| 置換途中の失敗 (`ReplaceFileW`) | `FsErrorKind::ReplacedMovedToBackup { backup }` (02 §4.5): 元ファイルが `.<name>.eve-bak-<pid>-<rand8>` (命名は 05 §4.2) に移動済みで、新内容の配置に失敗した状態。core は退避パスを提示し、名前を付けて保存へ誘導する (v0.1。退避ファイルを元の名前に戻す操作は v0.1 では提供しない、確定 (ユーザー回答)。§9.7 (e))。クラッシュで報告できなかった場合は保存中フラグ (05 §7.5) から検出する: そのパスを開くときの検出は v0.1 (§9.2 手順 2、05 §4.6)、起動時の一斉走査は v0.2 (§10.3) |
| 代替データストリーム / 圧縮属性 | `ReplaceFileW` が維持する (05 §4.2)。直接上書き (in-place) は元ファイルをそのまま使うので維持される |
| 置換の失敗 (元ファイルは無傷) | `ReplaceFileW` が元ファイルを変えずに失敗したら、Port は別の置換手段へ切り替えずに `AtomicUnsupported` を返す。core は §9.3 手順 7 の確認 (直接上書き / 別名で保存 / キャンセル) を経て直接上書きへ進む (02 §4.5。経路は「原子的置換 → 確認 → 直接上書き」の 1 本) |
| ウイルス対策による一時的ロック | 保存の再試行で吸収。失敗時のメッセージに「他のプログラムが使用中の可能性」を含める (NFR-602) |

### 9.5 外部変更の検知と再読込 (FR-117)

- **検知手段**: (v0.1) 保存ジョブ内の比較 (§9.3 手順 0 / 5a)。外部の変更を取り込む手段は、利用者の明示の再読込 `file.reload` (下の「明示の再読込」) だけで、検知はしない。(v0.2) ユーザーの明示的なコマンド (`file.check_external_changes`) と、フォーカス復帰 (`InputEvent::Focus(true)`) 時に **ジョブで** `metadata` を取り `disk_stamp` と比較 (メインスレッドでは行わない。ADR-0020。`JobKind::Metadata` の目的 `MetaPurpose::ExternalCheck` (02 §9.3) を使う)。(v0.4) `FileWatcher` Port のイベント (デバウンス済み)。自分の保存によるイベントは `disk_stamp` と一致するので無視できる。比較キーは 02 §4.5 の `FileStamp` (mtime + len + FileId、16 MiB 以下は内容ハッシュ併用。計算規則は 05 §5.5)。
- **保存時の競合 (v0.1)**: 保存ジョブが `Conflict` を返したら (§9.3 手順 5a / 5c)、書き込まずに次の確認を出す (01 FR-117 AC-1)。選択肢は v0.1 では「上書き / 別名で保存 (既定) / 中止 (Esc)」の 3 つで、**このダイアログの再読込の選択肢は v0.2** (フォーカス復帰時の確認と同時。v0.1 は中止してから `file.reload` で読み直せる)。上書きを選んだら `expected` を `Conflict { current: Option<FileStamp> }` の `current` から決めて再投入する (`Some(stamp)` → `Expect::Stamp(stamp)`、`None` (ファイルが無い。開いた後に削除された) → `Expect::Absent`)。
  - 既存のファイルが開いた後に変更・削除された: `ui.confirm.external_change` (削除されていた場合の「上書き」はファイルを作り直す)
  - 新規ファイル (`Expect::Absent`) の保存先に同名のファイルが作られていた: `ui.confirm.save.target_exists`
- **明示の再読込 (`file.reload`、v0.1。FR-117 の Should の部分、確定 (ユーザー回答))**: 読込の経路は「文字コードを指定して開き直す」(§8.8) と同じで、適用だけが違う (開き直しは履歴を消すが、再読込は Undo できる下の「再読込の実現」)。
  1. when は `doc.has_path` (07 §11.6)。パスを持たない文書 (無題) ではパレットに淡色と理由で出て実行できない。読込中 (`load_state == Loading`) の文書では何もせず `info.reload.busy` を出す。保存ジョブの実行中は、その完了を待ってから 2. の判定をする (§8.8 と同じ)。読み取り専用の文書でも実行できる (再読込は利用者の編集ではなく、`Origin::Reload` は §6.5 の拒否の例外。エディタの外で属性を外したファイルを読み直す手段になる)。
  2. dirty なら、読み込む前に `ui.confirm.reload_discard` (04 §15.2) で確認する。選択肢は「`[L]` 再読込 (変更を破棄)」と `[Esc]` キャンセル で、既定はキャンセル (破壊的な選択肢は既定にしない)。キャンセルでは何も変わらない。dirty でなければ確認しない。
  3. 読込ジョブ (`JobKind::Open`) を同じ `DocumentId` で投入する。`OpenOptions.forced_encoding` に今の文書の文字コードを入れ (§8.2 手順 2。判定し直さず、バイナリ・UTF-32 の BOM の確認もしない。BOM は手順 2 の規則で読み飛ばして `has_bom` を決める)、今の文書が `LargeFileViewer` なら `confirmed_large` と `read_only_requested` を立てて巨大ファイルの確認を繰り返さない。それ以外の開く前の確認 (512 MiB 超・不正バイトの比率) は通常の読込 (§9.2) と同じ規則で出し、「中止」はキャンセルと同じ。二重オープンの判定 (`Identity`) は同じ文書なので行わない。読込中は `load_state = Loading` で **旧本文を表示し続け** (編集はできない。ステータスバーは `[読込中 n%]`)、`ui.cancel` で取り消せる。
  4. 取消し・失敗 (ファイルが無い・読めない等) では、旧本文・`TextFormat`・読み取り専用の理由・履歴をそのまま保って `load_state = Ready` に戻し、失敗は理由を示す (ファイルが無いときは `msg.reload.missing`、それ以外は読込と同じ `msg.open.*`。取消しは `info.open.cancelled`)。
  5. `Complete` を受けたら、下の「再読込の実現」で `Origin::Reload` の Transaction として適用し、`info.reload.done` を出す。読み取り専用の理由は §8.8 の「読み取り専用の理由の差し替え」(再読込の規則。`Binary` と `LargeFileViewer` は保つ) で付け直す。再読込の後に読み取り専用の文書では `info.reload.done` の代わりに `info.reload.done_read_only` を出す (Undo で戻せないため。下の「再読込の実現」、提案、OQ-216)。ステータスバーの文字コード・改行コード・読み取り専用の表示は新しい内容に合わせて変わる。
- **状態** (v0.2): `external_modified: Option<FileStamp>` を Document に持ち、ステータスバーに表示。
- **検知したときの選択肢** (v0.2。dirty かどうかで既定を変える):

| Document の状態 | 既定の挙動 |
|---|---|
| dirty でない | 自動で再読込 (設定で確認に変更可)。Selection は写像で維持 |
| dirty | 確認: 「再読込 (自分の変更を破棄) / 比較 (v1.L: 差分ビュー FR-517) / 無視 (上書き保存時に再確認)」 |
| 削除された | 「新規として保持 (保存で再作成) / 閉じる」 |

- **M9d の実装 (v0.2、フォーカス復帰と明示のコマンド)**:
  - 契機は 2 つ。(1) `InputEvent::Focus(true)`: **表示中の文書 (アクティブ) だけ** を確かめる (他の文書は保存時の照合 (§9.3 手順 0 / 5a) で守られ、切り替えた後に (2) で確かめられる)。`file.reload_on_external_change = "never"` なら確かめない。確認ダイアログ・プロンプト等のオーバーレイが開いている間、その文書の読込・保存・照合のジョブがある間は確かめない (次のフォーカス復帰で確かめる)。`Focus(false)` は何もしない。(2) `file.check_external_changes` (パレット。アクティブ文書。設定に関わらず確かめ、変わっていなければ `info.external.unchanged`)。
  - ジョブは `JobKind::Metadata { purpose: ExternalCheck }` で、16 MiB 以下は内容ハッシュも取る (05 §5.5)。比べ方は未編集の上書き保存の照合 (§9.3 手順 0) と同じ。フォーカス復帰の結果が届いたとき、その文書がもうアクティブでない、またはオーバーレイが開いていれば捨てる。
  - **変更あり**: dirty でなく、設定が既定の `auto_if_unmodified` なら確認なしで再読込する (`file.reload` と同じ経路。Undo で戻せる)。dirty か `ask` なら `ui.confirm.external_modified` (`[L]` 再読込 (自分の変更を破棄) / `[I]` 無視 ★)。
  - **削除**: `ui.confirm.external_deleted` (`[K]` 新規として保持 ★ / `[C]` 閉じる。閉じるは `buffer.close` と同じ手順で、dirty なら未保存の確認が出る)。保持は文書を新規ファイル扱いにする (`disk_stamp` を捨てる): 次の保存は `Expect::Absent` で確認なしにファイルを作り直し (その間に同名のファイルが作られていれば `ui.confirm.save.conflict_created`)、フォーカス復帰でも再び尋ねない (ファイルが無いのは新規ファイルの状態と同じ)。
  - **同じ変更で何度も聞かない**: 「無視」を選んだら、そのときのファイルの Stamp を `Document.external_seen` に覚え、フォーカス復帰で同じ Stamp を見つけても尋ねない (明示のコマンドでは尋ねる。削除の「新規として保持」は上のとおり新規ファイル扱いにするので覚える必要が無い)。保存の成功・再読込で忘れる。保存時の照合の確認 (`ui.confirm.save.conflict_*`) はこれと関係なく出る。
  - 照合の失敗 (読めない等) は、フォーカス復帰では何も出さず、コマンドでは `msg.external.check_failed` を出す。
  - 残し: ステータスバーの表示 (上の「状態」)、保存時の競合の確認の「再読込」の選択肢、アクティブでない文書の確認。
- **再読込の実現**: 新内容をジョブで読み込み (§9.2 と同じ経路。文字コードは文書の現在の `encoding` を §8.2 手順 2 の明示指定として渡し、判定し直さない。BOM は手順 2 の規則で、その文字コードの BOM と一致すれば読み飛ばして `has_bom` を決める)、完了時に **旧テキスト → 新テキストの ChangeSet** を作って `Transaction(Origin::Reload)` として適用する。**再読込 (v0.1 の `file.reload`、v0.2 のフォーカス復帰時の確認とも) は全置換 ChangeSet** (`Delete(全体) + Insert(新全文)`。旧全文は `InsertText::Rope` (旧 rope そのもの) で保持するためコピーなし)。新しい内容に合わせて `TextFormat` (`has_bom`・`eol_stats`・`invalid_bytes`・`roundtrip`・`pua_escape` 等) と内容由来の読み取り専用の理由も置き換え、前後の値を Undo ノードの `format_change` に持たせる (Undo / Redo で本文と一緒に戻る、§7.2)。Selection は「同じ行:桁 (書記素)」を目安に写像し、Decoration は無効化する。行単位 diff による **最小差分** (imara-diff / similar) は、差分ビュー FR-517 (v1.L) と同時に導入する。**Undo で元に戻せる** (P1 原則)。ただし再読込の後に読み取り専用の文書 (読み取り専用の文書を読み直した、または読み直した内容に不正バイトがある等) では、`edit.undo` の when (`!doc.read_only`、07 §11.6) により Undo できない (再読込そのものは §6.5 の拒否の例外だが、`edit.undo` / `edit.redo` はコマンドの when の段階で実行できない。提案、OQ-216)。旧全文の保持によるメモリ増は 01 NFR-202 の例外 (§7.5)。再読込後 `history.saved` を再読込ノードの位置 (§7.4) にし、`disk_stamp` を更新。Undo で再読込前に戻ると、内容がディスクと違うので dirty になる。
- **自分の保存と外部変更の競合**: §9.3 手順 5a のジョブ内チェックで、競合時はファイルを壊さずに中止する。ユーザーが「上書き」を選んだ場合でも、元ファイルは原子的置換まで無傷 (Windows の置換途中失敗は退避名で残る、§9.4)。

### 9.6 読み取り専用・バイナリ (FR-115 / 116)

- 読み取り専用の理由は **理由の集合** `ReadOnlyReasons` (02 §5.1。変種は `ReadOnlyReason`、宣言順 = 下表の行の順 = 表示の優先順) で持ち、ステータスバーに `[RO]` と理由 (`ReadOnlyReasons::primary()`) を表示する。読込中は理由ではなく `load_state == Loading` から導出する (ステータスは読込の進捗を出す)。編集の可否は「`read_only` が空 かつ `load_state == Ready`」で、when の `doc.read_only` (07 §7.3) はその否定 (理由が 1 つ以上ある、または `load_state != Ready`) を返す。**読み取り専用の文書への編集操作** (文書を変える全コマンド。文字の入力・削除・貼り付け・Undo / Redo を含む) は、Transaction を作る前に拒否し、理由付きのメッセージ (`msg.edit.read_only`、引数 `{reason}` は理由ごとの文言、04 §15.2) を出す。文書は変わらない。例外は再読込 (v0.1 の `file.reload`、§9.5) とそのノードの Undo / Redo で、内容由来の理由を本文と一緒に戻すので拒否しない (§7.2)。when 条件 `!doc.read_only` (07 §7.3) を持つコマンドはパレットに淡色で出る。
- **理由と解除の可否** (本表が正本。01 FR-116、07 `doc.read_only_releasable` はこの表を引く。v0.1 には解除の手段が無く、`doc.read_only_releasable` は常に false):

| 理由 | 付く時点 | 解除 |
|---|---|---|
| `FileAttribute` | 開くとき、ファイルに読み取り専用属性 (Windows の READONLY) がある (05 §5.2) | v0.2: `file.toggle_read_only` で解除できる。解除した文書の上書き保存は「読み取り専用を解除して保存」(属性を外して置換し、置換の後に付け直す。05 §4.2、FR-106) の確認を経る |
| `UserRequested` | `--readonly` で開いた (07 §18.3) | v0.2: `file.toggle_read_only` で解除できる |
| `NoWritePermission` | 保存が `PermissionDenied` で失敗した (v0.1 の Windows は開く時点では書込権限 (ACL) を判定しない楽観方式、05 §5.2) | 解除できない (書けないことが分かっているため)。名前を付けて保存で回避する |
| `InvalidBytes` | 読込で不正バイトを検出した (§8.3) | v0.3: 「編集を許可」で解除できる (§8.3)。v0.1 / v0.2 は解除できず、「文字コードを指定して開き直す」(FR-111) で回復する |
| `StatefulEncodingWithInvalidBytes` | 状態付き符号化 (ISO-2022-JP) で不正バイトを検出した (§8.3) | 解除できない (FR-111 で別の文字コードとして開き直す) |
| `Binary` | バイナリの警告で「読み取り専用で開く」を選んだ (FR-115) | 解除できない (NG10) |
| `LargeFileViewer` | 巨大ファイル (512 MiB 超) の警告で「読み取り専用で開く」を選んだ (FR-127、§11.2)。不正バイトの多いファイルの警告 (FR-127) で「読み取り専用で開く」を選んだ文書には付けず、理由は `InvalidBytes` だけになる | 解除できない (開き直さずに編集できると、FR-127 の警告の意味が無くなるため) |
| `LoadIncomplete` | v0.1 では付かない (読込の失敗では読めた部分を開かない、§9.2 の「失敗」)。読めた部分だけを開く選択肢を入れる版のための予約 | 解除できない (保存すると読めなかった後半を失うため)。名前を付けて保存は可 |
| `LongLine` | 閾値を超える行を含む (§11.5、v0.1) | 解除できない。v0.2 からは char 近似モードで編集できるので、この理由は付かない |

- 理由が複数あるときは全部を保持し、表示はこの表の上にあるものを先に出す。**名前を付けて保存** は読み取り専用の文書でも実行できる (v0.1 で読み取り専用の文書を保存する唯一の手段)。例外: `StatefulEncodingWithInvalidBytes` の文書 (不正バイトを含む ISO-2022-JP) は、ISO-2022-JP のままの名前を付けて保存・ISO-2022-JP を指定した保存を連鎖の開始時に `msg.save.encoding_unsupported_iso2022` で拒否する (保存先を尋ねない。§8.3、確定 (ユーザー回答)、OQ-207)。別の文字コードを指定した保存はエスケープ文字が表現不能 (E4) なので E3 のキャンセルだけになり、この文書を書く手段は無い (FR-111 で別の文字コードとして開き直す)。成功したら保存先について `FileAttribute` / `NoWritePermission` を判定し直し (外れれば編集できる)、それ以外の理由は残す。
- 解除の取り消し (`file.toggle_read_only` をもう一度) は、どの文書でも `UserRequested` を付ける (v0.2)。
- **M9d の実装 (v0.2)**: `doc.read_only_releasable` は「読込が済み、理由があり、すべてが `FileAttribute` か `UserRequested`」。`FileAttribute` を解除した文書は解除の記録 (`Document.attribute_released`) を持ち、元のパスへの上書き保存の前に `ui.confirm.save.read_only` (既定は別名で保存) を出す。承認は (文書, Revision) ごとで、同じ Revision の保存では繰り返さない。全保存では保存せず、確認が要る文書として一覧に出す。承認した保存は `SaveOptions.clear_readonly` で投入し、Port が置換の直前に属性を外して置換の後に付け直す (05 §4.2)。解除の記録は、名前を付けて保存で別のパスに書いたときと、再読込 (属性をディスクから判定し直す) で消える。直接上書き (§9.3 手順 7 の確認の後、05 §4.4) では属性を外さないので、`PermissionDenied` (理由 `NoWritePermission`) になる。
- バイナリ判定されたファイルを「読み取り専用で開く」場合も §8.3 のエスケープで内容を表示する (NUL は `<0x00>`)。NG10 によりバイナリ編集は提供しない。

### 9.7 保存の連鎖と後続処理 (Continuation)

保存は確認 (上書き・区分 B・表現不能・外部変更・原子的置換不可 等) をはさんで何度かジョブを投入し直すことがあり、終了・置き換え・開き直しの前の「保存」(FR-107) はその保存が成功したときだけ後続の処理を行う。本節はその連鎖を 1 つの状態機械として定める。状態は文書ごとに core (`io/` モジュール) が持ち、ジョブは確認を待ってブロックしない (02 §9.3)。

```rust
// eve-core の内部状態 (境界型ではない)
enum Continuation {              // 保存が成功したときだけ実行する後続の処理
    Quit,                        // app.quit (FR-107 AC-1)
    Replace(ReplaceTarget),      // file.open / file.new による置き換え (§9.2、FR-107 AC-3)。ReplaceTarget = Open(OpenRequest) | New
    Reopen { encoding: EncodingName }, // file.reopen_with_encoding (§8.8、FR-111 AC-2)
}
struct SaveChain {
    doc: DocumentId,
    cont: Option<Continuation>,          // 後続。None は単独の保存 (file.save / file.save_as を直接実行した)
    target: Option<FsPath>,              // 書く先。None は AskName で決まる前 (無題の文書)。決まった後は常に Some
    expect: Expect,                      // 投入時の SaveOptions.expected (02 §4.5)
    format: TextFormat,                  // このチェーンで書く format (§8.8)。確定するのは完了のときだけ
    opts: SaveOptions,                   // 投入する SaveOptions。expected / hardlink / in_place_backup は投入のたびに expect / hardlink / in_place_backup の値で上書きし、
                                         // それ以外 (fsync・private・preserve・backup) は連鎖の開始時に決めて変えない
    method: SaveMethodRequest,           // Atomic | InPlace (§9.3 手順 4)
    hardlink: HardlinkPolicy,            // Ask (既定、07 §5.2 file.hardlink_policy) | BreakLink | WriteInPlace
    in_place_backup: InPlaceBackup,      // Required (既定) | SkipConfirmed
    state: SaveChainState,
}
```

**各項目を更新する遷移** (表に無い遷移では項目を変えない。保存の重なり (§9.3 手順 4 の深さ 1 のキュー) で再実行するときの `expect` は §9.3 のとおり直前の `report.new_stamp`):

| 遷移 | `target` | `expect` | `format` | `method` | `hardlink` | `in_place_backup` |
|---|---|---|---|---|---|---|
| 連鎖の開始 (`file.save`、`AskUnsaved` の「保存」、`file.set_encoding`) | 文書のパス (無題・読み取り専用なら None のまま `AskName` へ) | `disk_stamp` が Some なら `Stamp(disk_stamp)`、None なら `Absent` | 文書の format (`file.set_encoding` は新しい format、§8.8) | `Atomic`。`file.save_strategy = "in_place"` で T が存在する (`disk_stamp` が Some) なら `InPlace` (§9.3 手順 4) | 設定値 (既定 `Ask`) | `Required` |
| `CheckTarget` で保存先が決まる ((a) の 1: 今の文書と同じファイル) | 文書のパス | `Stamp(disk_stamp)` (§9.3 手順 0 から) | 変えない | 開始時と同じ規則 | 設定値 | `Required` |
| 同 ((a) の 2: 存在しない) | 入力したパス | `Absent` | 変えない | `Atomic` | 設定値 | `Required` |
| `ConfirmOverwrite` で「上書き」((a) の 6) | 入力したパス | `Stamp(取得した stamp)` | 変えない | `Atomic`。`save_strategy = "in_place"` なら `InPlace` | 設定値 | `Required` |
| `AfterFailure` の `Conflict` で「上書き」 | 変えない | `Conflict { current }` の `current` が `Some(stamp)` なら `Stamp(stamp)`、`None` (不在) なら `Absent` | 変えない | 変えない | 変えない | 変えない |
| `AfterFailure` のハードリンクで「リンクを保って直接上書き」/「切り離して保存」 | 変えない | 変えない | 変えない | `InPlace` / `Atomic` | `WriteInPlace` / `BreakLink` | 変えない |
| `AfterFailure` の `AtomicUnsupported` で「直接上書き」 | 変えない | 変えない | 変えない | `InPlace` | 変えない | 変えない |
| `AfterFailure` の `BackupFailed` で「バックアップなしで上書き」 | 変えない | 変えない | 変えない | `InPlace` (のまま) | 変えない | `SkipConfirmed` |
| `AfterFailure` の `Busy` で「再試行」 | 変えない | 変えない (何も書いていないので照合は通る) | 変えない | 変えない | 変えない | 変えない |
| `AfterFailure` の E3 で「UTF-8 で保存」 | 変えない | 変えない | `encoding = UTF-8`、`has_bom = false` (他は元のまま、§8.6) | 変えない | 変えない | 変えない |
| `ConfirmReplacement` で「置換して保存」 | 変えない | 変えない | 変えない (置換の Transaction は文書に適用する) | 変えない | 変えない | 変えない |
| 「別名で保存」で `AskName` へ戻る | `CheckTarget` で上の行のとおり決め直す | 同左 | 変えない | 同左 | 同左 | 同左 |

**連鎖の開始時の拒否**: 上の表の「連鎖の開始」の契機 (`file.save`・`file.save_as`・`AskUnsaved` の「保存」・`file.set_encoding`) で `format` を決めた直後、`AskName` / `CheckingUnchanged` / `PreConfirm` のどれにも進む前に、`format.encoding` が ISO-2022-JP で文書が §8.3 のエスケープ文字を含む (`invalid_bytes > 0`) なら、ジョブを投入せずに `msg.save.encoding_unsupported_iso2022` を示して連鎖を中止する (`cont` の後続も実行しない。文書・`TextFormat` は変えない。§8.3 の (a)、確定 (ユーザー回答)、OQ-207)。

**状態** (`SaveChainState`):

| 状態 | 入る契機 | 利用者の操作・ジョブの結果 → 次の状態 |
|---|---|---|
| `AskUnsaved` | dirty な文書で後続を伴う操作 (終了・置き換え・開き直し) を実行した | 保存 → 文書がパスを持ち、かつ読み取り専用の理由が無い (`read_only` が空) なら `PreConfirm` (確認が要らなければ素通りして `Saving`)。パスが無い (無題)、または **読み取り専用 (dirty でも)** なら `AskName` (読み取り専用の文書の上書き保存は無効、§9.3 手順 1) / 破棄 → 後続をすぐ実行 / キャンセル (`Esc`) → 中止 |
| `AskName` | 無題の文書の保存、`file.save_as`、読み取り専用の文書の `file.set_encoding`、`AskUnsaved` の保存 (無題・読み取り専用) | パスを入力 → `CheckTarget` / キャンセル (`Esc`) → 中止 |
| `CheckTarget { job }` | 名前を付けて保存の保存先が決まった (`JobKind::Metadata { purpose: MetaPurpose::SaveAsTarget }`) | 下の (a) の規則 → `ConfirmOverwrite` / `PreConfirm` / エラーを示して `AskName` に戻る ((a) の 1 で未編集なら、§9.3 手順 0 のとおり `CheckingUnchanged` へ) |
| `CheckingUnchanged { job }` | 未編集の文書の上書き保存 (§9.3 手順 0。`disk_stamp` が Some で dirty でない)、または (a) の 1 で未編集。`JobKind::Metadata { purpose: MetaPurpose::UnchangedSave }` を投入した | 下の「全状態の遷移表」 |
| `ConfirmOverwrite` | 保存先に既存のファイルがある ((a) の 6) | 上書き → `PreConfirm` / 別の名前を入力 (**既定**。`Esc` も同じ) → `AskName` に戻る (入力した名前は残す) |
| `PreConfirm` | 投入の前の確認が要る (§8.6 E2' の区分 B、ISO-2022-JP のエスケープの正規化) | 保存 → `Saving` / 別名で保存 → `AskName` / キャンセル → 中止。確認が要らなければ素通りして `Saving` |
| `Saving { job, node, cancel_origin }` | 保存ジョブを投入した (§9.3 手順 4)。`cancel_origin` は投入時に `None` | 下の「全状態の遷移表」。`cancel_origin` は取消しの由来で、`None` (取り消していない) / `User` (`ui.cancel` で後続を取り消した。`cont` が Some のときだけ立つ) / `QuitAbort` ((f) の `[Q] 保存を中止して終了`) の 3 値。結果の扱いは `cancel_origin` ごとに遷移表の行を分ける |
| `BuildingReplacement { job }` | `AfterFailure` の E3 で「置換して保存」を選んだ。`JobKind::ReplaceUnmappable { snapshot, format }` を `bg` に投入した (§8.6) | `JobResult::UnmappableReplacement` → 結果の base Revision が今の Revision と同じなら `ConfirmReplacement`、違えば (待つ間に編集が入った) 結果を捨て、今の Snapshot でジョブを投入し直して `BuildingReplacement` のまま / `Failed` → エラーを示して中止 |
| `ConfirmReplacement { changes, base }` | 置換の ChangeSet ができた | `ui.confirm.save.unmappable_replace.message` (§8.6 の「置換の一覧」。`targets` は先頭 100 件、件数は `counts { kana, compat, question }` で、総数はその合計) を出す。置換して保存 → 今の Revision が `base` と同じなら `Transaction(Origin::System)` として適用し、同じ文字コードで `PreConfirm` へ (違えば `BuildingReplacement` に戻って作り直す) / キャンセル (既定。`Esc`) → E3 の選択肢 3 と同じ扱いで中止 |
| `AfterFailure { err }` | 保存が失敗し、利用者に選ばせる (§9.3 手順 7 の分類) | 下の (e) の表 |
| `WritingRecovery { job }` | (e) で「退避先に保存」を選んだ。`JobKind::WriteRecovery` を投入した (下の「退避先に保存」) | `JobResult::RecoveryWritten { path }` → `info.save.to_recovery` を示して中止 (後続は行わない) / `Failed` → `msg.save.recovery_failed` を示して中止 |
| 完了 | `JobResult::Saved` (`CheckingUnchanged` では照合の一致) | §9.3 手順 7 の成功処理と、format の確定 (§8.8) (`CheckingUnchanged` からは書いていないので `info.save.unchanged` を示すだけ)。`cancel_origin = User` なら `cont` を捨てる (取り消した後続は実行しない)。`cont` があれば、文書が dirty でない (保存した時点の後に編集が無い) ときだけ後続を実行する。保存中の編集で dirty なら後続を実行せず `AskUnsaved` をもう一度出す |
| 中止 | 上のキャンセル・中止 | `cont` を捨てる (終了・置き換え・開き直しはしない)。文書は元の編集状態のまま (本文・dirty・format・選択は変えない。E3 の置換を適用した後の中止では、置換の Transaction は残り Undo で戻せる)。保存しなかったことをメッセージ (`info.save.aborted`) で示す (退避先に保存の後の中止では `info.save.to_recovery` / `msg.save.recovery_failed` だけを、`CheckingUnchanged` の照合の失敗による中止では `msg.save.check_failed` だけを示す) |

**全状態の遷移表** (状態 × 事象。`file.save` は `file.save_as` / `file.set_encoding` を含む同じ文書の保存要求。「無視」は状態を変えず、保存中であること (`info.save.in_progress`、ステータスは `status.saving.*`、04 §8.7) を示す。確認・入力の状態では、オーバーレイが入力を受けるので `ui.cancel` はオーバーレイの `Esc` として届く。ジョブを待つ状態 (`CheckTarget`・`CheckingUnchanged`・`BuildingReplacement`・`Saving`・`WritingRecovery`) はオーバーレイを出さず、`Esc` は 04 §7.5 の `Esc` (`ui.cancel`) の意味の層として本表の列のとおりに届く):

| 状態 | `ui.cancel` (`Esc`) | `app.quit` | `file.save` | `file.open` / `file.new` (置き換え、§9.2) | ジョブの結果 |
|---|---|---|---|---|---|
| `AskUnsaved` | 中止 (キャンセルと同じ) | 無視 (この確認に答えてから実行し直す) | 無視 | 無視 (この確認に答えてから実行し直す) | — (ジョブなし) |
| `AskName` | 中止 | 無視 | 無視 | 無視 | — |
| `CheckTarget` | 照合ジョブの結果を捨て、`AskName` に戻る (入力した名前は残す) | 無視 | 無視 | 無視 | `Meta` / `Failed` → (a) の表 |
| `CheckingUnchanged` | 中止 (照合ジョブの結果を捨て、`cont` を捨てる。`cont` が None (単独の保存) でも同じ。何も書いていない) | 結果を待たずに照合を捨てて終了へ進む (何も書いていないので取消しは要らない。`QuitAbort` と同じく待たない): 文書が dirty でなければ終了、待つ間の編集で dirty なら `AskUnsaved` (`cont = Quit`) を出す | 無視 | 無視 (照合の結果を待ってから実行し直す) | 先に Revision を見る: 結果の base Revision が今の Revision と違う (待つ間に編集が入った) → 結果を捨て、同じ `cont` で §9.3 手順 0 からやり直す (dirty なので手順 1〜4 へ) / 照合が一致 (変更なし) → 完了 (`info.save.unchanged`。書かない。`cont` を実行) / 不一致 (`Failed` の `NotFound` (開いた後に削除された) を含む) → §9.3 手順 1〜4 で投入して `Saving` (`expect = Stamp(disk_stamp)` のままなので、ジョブの照合で `Conflict { current }` になり (削除されていれば `current = None`)、`AfterFailure` の `ui.confirm.external_change` で選ばせる。「上書き」なら `current` から `Expect` を決める (`None` → `Absent`)) / その他の `Failed` (`Busy`・`PermissionDenied`・その他) → `msg.save.check_failed` (04 §15.2、`{path}`・`{reason}`) を示して中止 (`cont` があれば捨てる)。照合の失敗は (e) の表に流さない: 再試行の選択肢は出さず (利用者が `file.save` を実行し直す)、`NoWritePermission` は付けない (読めなかったことは書けないことを意味しない) |
| `ConfirmOverwrite` | `AskName` に戻る (既定の選択と同じ) | 無視 | 無視 | 無視 | — |
| `PreConfirm` | 中止 | 無視 | 無視 | 無視 | — |
| `BuildingReplacement` | ジョブに取消しを送り、E3 の選択肢 3 (キャンセル) と同じ扱い (保存しない、最初の該当位置へ移動) で中止。後から届く結果は捨てる | 無視 (`Esc` で連鎖を止めてから実行し直す) | 無視 | 無視 | `UnmappableReplacement` → 上の状態表 / `Failed { Cancelled }` → 捨てる / その他の `Failed` → エラーを示して中止 |
| `ConfirmReplacement` | 中止 (E3 の選択肢 3 と同じ) | 無視 | 無視 | 無視 | — |
| `Saving` (`cancel_origin = None`、`cont` が Some) | 保存ジョブに取消し (`JobSpawner::cancel`) を **常に** 送り (main はジョブの段を見ない)、`cancel_origin = User` にする。状態は `Saving` のまま結果を待つ。ジョブは `ReplaceFileW` の開始以降・直接上書きの 05 §4.4 手順 6 以降なら取消しを無視して完了まで進み `Saved` (または失敗) を返す (§9.3 手順 7 の `Cancelled`、05 §11.5)。その `Saved` は `cancel_origin = User` の行で扱う (採用し、後続は捨てる) | 下の (f) | 無視 | 無視 (後続が既にある。`Esc` で連鎖を止めてから実行し直す) | `Saved` → 完了 (`cont` を実行) / `Failed { Cancelled }` → 中止 / その他の `Failed` (`ReplacedMovedToBackup`・`InPlaceFailed` を含む) → `AfterFailure` (`cont` を保つ) |
| `Saving` (`cancel_origin = None`、`cont` が None = 単独の保存) | **取り消さない** (この段を素通りし、次の層の `Esc` の意味へ進む。04 §7.5 の `Esc` (`ui.cancel`) の意味の一覧) | 下の (f) (`cont = Quit` にして上の行の扱いになる) | §9.3 手順 4 の深さ 1 のキューに積み、`Saved` で完了した後に最新の Snapshot で再実行する。前の保存が失敗 (`Cancelled` を含む) したら積んだ要求は破棄し、`AfterFailure` の選択に任せる (§9.3 手順 4) | **保存の連鎖に合流する** ((f) 1 の `app.quit` と同じ方式): `cont = Replace(target)` にして上の行 (`cont` が Some) の扱いになる。保存の完了まで新しい文書の読込は始めない。完了の時点で dirty でなければ §9.2 の「置き換えの手順」(2) へ進み (手順 (1) の確認は済んだものとする)、保存中の編集で dirty なら `AskUnsaved` (`cont = Replace`) を出す。失敗なら `AfterFailure` (`cont` を保つ)。保存の完了を待たずに旧文書を閉じることはしない | 上の行と同じ (`cont` が無いので後続は無い) |
| `Saving` (`cancel_origin = User`) | 何もしない (取消しは送信済み。この段を素通りし、次の層の `Esc` の意味へ進む) | `cont = Quit`、`cancel_origin = None` に戻し、下の (f) に従う (取消しが間に合っていれば結果は `Failed { Cancelled }` で中止になり、終了しない。終了するには `app.quit` をもう一度実行する) | 無視 | 無視 (取消しの結果を待ってから実行し直す) | `Saved` → **採用して完了。ただし `cont` を捨てる** (取り消した終了・置き換え・開き直しは実行しない。取消しが間に合わずに保存できたなら保存を正とする、02 §9.3 の採否表。保存できたことは `info.save.done` で示す) / `Failed { Cancelled }` → 中止 (`info.save.aborted`) / その他の `Failed` → `AfterFailure` (`cont` を捨て、単独の保存として (e) の表で選ばせる) |
| `Saving` (`cancel_origin = QuitAbort`) | 無視 (終了は取り消せない。確認は閉じており、ステータスは保存中の表示のまま) | 無視 | 無視 | 無視 | **どの結果でも終了へ進む** ((f) 3 の「保存を中止して終了」): `Saved` → §9.3 手順 7 で採用 / `Failed { Cancelled }` → 何も書いていない / `ReplacedMovedToBackup { backup }`・`InPlaceFailed { backup }`・その他の `Failed` → `AfterFailure` に進まない。そのあと dirty な文書を退避して終了し、失敗のパスを終了時の stderr に出す ((f) 3。core は `Effect::Quit { rescue: DirtyDocuments, stderr }` を出し、退避と出力は Host が行う、02 §6.3) |
| `AfterFailure` | 中止 (どの確認でも `Esc` は中止) | 無視 (確認に答えてから実行し直す) | 無視 (キューにも積まない。続きは確認の選択で決める、§9.3 手順 4) | 無視 | — |
| `WritingRecovery` | 取り消さない (退避の書込は待つ) | 無視 | 無視 | 無視 | 上の状態表 |

- **待ち時間に上限を置かない**: `CheckingUnchanged` と `Saving` と `BuildingReplacement` はジョブの完了を待ち続ける (タイムアウトしない)。ステータスに保存中 (`status.saving.*`、04 §8.7) を示す。後続が `Quit` の場合、完了の後の終了処理 (`Effect::Quit`) で Host が行う「置換中は exit しない」待ちは 05 §11.5 の規則 (保存中フラグ) に従う。
- **無題・読み取り専用の文書**: `AskUnsaved` で保存を選ぶと `AskName` → `CheckTarget` → `PreConfirm` → `Saving` と進む。`AskName` のキャンセルは後続を中止する。
- **連鎖の中の確認** (外部変更・ハードリンク・原子的置換不可・表現不能・区分 B 等) は連鎖の外の保存と同じ規則・同じ選択肢で、選んだ結果が「保存を続ける」なら `cont` を持ったまま上の「各項目を更新する遷移」で項目を変えて投入し直し、それ以外は中止する。
- 外部からの終了 (コンソールクローズ等) は連鎖を待たずに緊急退避する (FR-107 AC-2、§10.1)。

**(a) 名前を付けて保存の保存先** (`CheckTarget`。I/O はジョブ。保存先の照合は `JobKind::Metadata`)

| 順 | 保存先の状態 | 扱い |
|---|---|---|
| 1 | 入力したパスが今の文書のパスと構文的に同じ (`Paths::normalize` の後、05 §3.2)、または metadata の `file_id` が今の文書の `file_id` と同じ (別名・リンク・ハードリンクで同じファイルを指す) | **通常の上書き保存** として扱う (§9.3 手順 0 から。未編集なら書かない)。ただし `file.set_encoding` の連鎖 (連鎖の `format` が文書の format と違う) は手順 0 を飛ばして書く (§9.3 の「手順 0 の適用対象」)。ただし文書が読み取り専用なら上書き保存は無効 (§9.3 手順 1) なので、「読み取り専用の文書は同じファイルには保存できない」(`msg.save_as.same_file_read_only`) と示して `AskName` に戻る |
| 2 | 存在しない (`NotFound`、詳細なし) | `Expect::Absent` で保存する (書くまでに他者が作れば `Conflict`、`ui.confirm.save.target_exists`、§9.5) |
| 3 | リンク先が存在しない (`NotFound` + `DanglingLink`) | v0.1 はエラー (リンク先の絶対パスを示す、§9.3 の「シンボリックリンク」)。`AskName` に戻る。v0.2 もこの行は同じ (名前の入力で選んだリンクには作らない。リンク先を作る確認は、開いた文書の保存がダングリングで失敗したときだけ。下の (e) の表) |
| 4 | ファイルでない (`FileMeta.kind` が `Dir` / `Other`)、パスが不正 (`InvalidPath`。予約名を含む) | エラー (`msg.save_as.not_a_file` / `msg.save.invalid_path`) を示して `AskName` に戻る |
| 5 | 読み取り専用属性のファイル (`FileMeta.readonly`) | 上書きの確認を出す前に拒否する (`msg.save_as.target_read_only` を示し、`AskName` に戻る。今の文書の理由は変えない)。v0.1 は読み取り専用属性のファイルに書かない (FR-116) |
| 6 | 既存のファイル (リンクならリンク先) | `ui.confirm.save.overwrite` (「{path} は既にあります。上書きしますか」。リンクのときはリンク先のパスも示す。既定は「別の名前を入力」で `AskName` に戻る (04 §15.2)) を出し、上書きなら metadata で取得した stamp で `Expect::Stamp(stamp)` として保存する (確認の間に変わっていれば `Conflict`)。保存は §9.3 と同じ手順で、ハードリンク等は手順 7 の確認になる |
| 7 | metadata が `PermissionDenied` / `Busy` / `Other` で失敗 | エラー (`msg.save.permission_denied` / `msg.save.busy` / `msg.save.failed`) を示して `AskName` に戻る |

- 名前を付けて保存の保存が `PermissionDenied` で失敗しても、今の文書に `NoWritePermission` は付けない (失敗したのは別のパス)。`ui.confirm.save_failed` で別名での保存を案内する。
- 成功したら §9.3 の「名前を付けて保存」の更新を行う。

**(b) 文字コードを指定して保存** (`file.set_encoding`) と **(c) 文字コードを指定して開き直す** (`file.reopen_with_encoding`) の規則は §8.8。(b) は連鎖の `format` に新しい format を入れて投入し、`TextFormat` を確定するのは完了のときだけである。(c) で dirty なら `AskUnsaved` から入り、`cont = Reopen` とする。

**(d) FR-107 の 3 択の「保存」**: 上の `AskUnsaved` から入る連鎖そのもの。保存が成功した場合だけ後続 (終了・置き換え・開き直し) を実行し、保存の失敗・キャンセル・`Conflict` や表現不能の確認での中止では後続を中止して元の編集状態に戻る。

**(e) 保存失敗の後の選択肢** (`AfterFailure`。本表が正本で、04 §15.2 の MessageKey の選択肢はこれに合わせる)。本表が扱うのは **保存ジョブ (書込系) の失敗だけ** で、`CheckingUnchanged` の照合 (`JobKind::Metadata`) の失敗は本表に流さず、全状態の遷移表の `CheckingUnchanged` の行 (`msg.save.check_failed` を示して中止) で扱う。`CheckTarget` の metadata の失敗は (a) の 7。連鎖の外の保存 (`file.save` / `file.save_as` を単独で実行した) も同じ表を使い、「後続」の列だけが意味を持たない。`msg.save.*` と `ui.confirm.save_failed` の組は、メッセージの文言を確認の本文に入れて 1 つの確認として出す (04 §15.2)。★ は既定、`Esc` はどの確認でも「中止」。`ui.confirm.save_failed` を使う行 (`Busy`・`PermissionDenied`・`DiskFull` / `ReadOnlyFs`・`InPlaceFailed`・`ReplacedMovedToBackup`・`ParentMissing`・`DanglingLink`・その他) の選択肢は本表のとおりで、`Busy` の行だけが「再試行」を加える (04 §15.2 の目録の `ui.confirm.save_failed` はこの表に合わせる)。

| 失敗 (§9.3 手順 7) | MessageKey | 選択肢 → 次の状態 | 後続 (`cont`) |
|---|---|---|---|
| `JobError::Unmappable` | `ui.confirm.save.unmappable` | UTF-8 で保存 → `Saving` (連鎖の `format` を UTF-8 に) / 置換して保存 → `BuildingReplacement` / キャンセル ★ → 中止 (最初の該当位置へ移動)。出す条件は §8.6 の「E3 の選択肢」 | 保存を続ける選択では保つ。キャンセルで捨てる |
| `Conflict` (開いた後に変更・削除された) | `ui.confirm.external_change` | 上書き → `Saving` (`Expect` は `Conflict { current }` から決める: `Some(stamp)` → `Stamp(stamp)`、`None` (不在) → `Absent`) / 別名で保存 ★ → `AskName` / 中止 | 上書き・別名で保存では保つ |
| `Conflict` (新規の保存先に同名のファイルができた) | `ui.confirm.save.target_exists` | 同上 | 同上 |
| `AtomicUnsupported` + `Hardlink` (`HardlinkPolicy::Ask`) | `ui.confirm.save.hardlink` | リンクを保って直接上書き → `Saving` (`WriteInPlace`、method = InPlace) / このファイルだけ切り離して保存 → `Saving` (`BreakLink`) / キャンセル ★ → 中止 | 保存を続ける選択では保つ |
| `AtomicUnsupported` (上記以外。`file.atomic_save_fallback = ask`) | `ui.confirm.save.atomic_unsupported` | 直接上書き (バックアップ付き) → `Saving` (method = InPlace) / 別名で保存 ★ → `AskName` / 中止。`fail` (v0.2 で設定できる) のときは確認を出さず、下の「その他」と同じ扱い | 直接上書き・別名で保存では保つ |
| `BackupFailed` (事前バックアップの失敗、または直接上書きで保存中フラグを書けない (detail = `InflightFlag`)。どちらも何も書いていない) | `ui.confirm.save.overwrite_without_backup` | バックアップなしで上書き → `Saving` (`in_place_backup = SkipConfirmed`、method = InPlace) / 別名で保存 → `AskName` / キャンセル ★ → 中止 | 保存を続ける選択では保つ |
| `Busy` | `msg.save.busy` + `ui.confirm.save_failed` | 再試行 (`[T]`) → `Saving` (同じ `Expect`。何も書いていないので照合は通る) / 別名で保存 ★ → `AskName` / 退避先に保存 → `WritingRecovery` (下の「退避先に保存」。退避の後に中止) / 中止 | 再試行・別名で保存では保つ。退避先に保存では捨てる |
| `PermissionDenied` | `msg.save.permission_denied` + `ui.confirm.save_failed` | 別名で保存 ★ → `AskName` / 退避先に保存 → `WritingRecovery` (退避の後に中止) / 中止。上書き保存の失敗なら文書に `NoWritePermission` を付ける (§9.6。名前を付けて保存の失敗では付けない、(a) の後の箇条) | 別名で保存では保つ。退避先に保存では捨てる |
| `DiskFull` / `ReadOnlyFs` | `msg.save.disk_full` / `msg.save.read_only_fs` + `ui.confirm.save_failed` | 別名で保存 ★ → `AskName` / 退避先に保存 → `WritingRecovery` (退避の後に中止) / 中止 (再試行は出さない。空きを作った後は `file.save` で保存し直す。`NoWritePermission` は付けない)。以下の行の「同上」はこの行の選択肢を指す | 同上 |
| `InPlaceFailed { backup }` | `msg.save.in_place_failed` (`backup` が None なら `msg.save.in_place_failed_no_backup`) + `ui.confirm.save_failed` | 別名で保存 ★ → `AskName` / 退避先に保存 → `WritingRecovery` (退避の後に中止) / 中止。バックアップのパスを必ず示す (`backup` が None (バックアップなしで上書きを承認した) ならバックアップが無いことを示す)。**再試行は出さない** (途中まで書き換わったファイルは `disk_stamp` と一致しないので、同じパスへの保存は `Expect` の照合で `Conflict` になる。名前を付けて保存へ誘導する) | 同上 |
| `ReplacedMovedToBackup { backup }` | `msg.save.moved_to_backup` + `ui.confirm.save_failed` | 同上。退避パスを必ず示す。**「元の名前に戻す」は v0.1 では出さない** (確定 (ユーザー回答)。利用者が手で戻す。05 §4.6)。再試行も出さない (元の名前のファイルが無いので `Conflict` になる) | 同上 |
| `NotFound` + `ParentMissing` | `msg.save.parent_missing` + `ui.confirm.save_failed` | 同上。フォルダは作らない (01 FR-101 AC-2) | 同上 |
| `NotFound` + `DanglingLink` | v0.1: `msg.save.dangling_link` + `ui.confirm.save_failed`。v0.2: `ui.confirm.save.dangling_link` | v0.1 は同上。v0.2 は「リンク先に作成」★ / キャンセル (`Esc`)。作成は同じ保存を `SaveOptions.create_link_target` 付きで 1 回だけ投入し直す (05 §4.1) | 同上 |
| その他 (`InvalidPath`、`Other`、`JobError::Panic` 等) | `msg.save.invalid_path` / `msg.save.failed` + `ui.confirm.save_failed` | 同上 | 同上 |
| `JobError::Cancelled` | `info.save.aborted` | 選択肢なし → 中止 | 捨てる |

- **退避先に保存**: 文書の Snapshot を緊急退避と同じ形式 (§10.1: UTF-8・改行非変換・PUA エスケープのまま、manifest 付き。manifest の `reason = "user_save"`) で `state_dir/recovery/rescue/<UTC timestamp>-<pid>/` に書く (緊急退避のディレクトリとは別。05 §7.5)。書き手は `bg` のジョブ `JobKind::WriteRecovery { doc }` (02 §9.3。`DocSnapshot` を渡す。出力先のディレクトリと本文・manifest は `FileSystem::write_recovery` (02 §4.5) が、名前・`created_at`・代替の退避先を決めて作り、書く。I/O を伴うが `io` ではなく `bg` ワーカーで `FileSystem` Port を使う。シリアライズの関数は緊急退避・スワップと共用) で、結果は `JobResult::RecoveryWritten { job, path }`。`state_dir` を使えない (作れない・書けない) ときは緊急退避と同じ代替の場所の `rescue\` (Windows は `%TEMP%\eve-recovery-<SID>\rescue\<UTC timestamp>-<pid>\`、所有者のみの DACL。05 §7.5 / §11、02 §11.2.1 (OQ-163)) に書く。結果は文書の Revision が進んでいても、取消しの後でも、文書が閉じられていても採用する (02 §9.3 の採否表)。書けたら `info.save.to_recovery` で退避先を示し、書けなければ `msg.save.recovery_failed` (04 §15.2) で理由を示す (どちらでも文書は dirty のまま)。元のファイルは変えず、`saved` は動かさない。**後続の処理は中止する** (退避は保存の代わりではないので、終了・置き換え・開き直しをしない)。

**(f) 保存中の終了操作** (04 §15.2 `ui.confirm.quit_saving`・05 §11.5 と同じ方式): 保存ジョブの実行中 (`Saving`、または連鎖の外で `Document.saving` が Some) に `app.quit` を実行したら、次のとおりにする。

1. その文書の連鎖の `cont` を `Quit` にして `Saving` のまま待つ (連鎖の外の保存なら、`cont = Quit` の連鎖をその場で作る。置き換え・開き直しの `cont` は終了で置き換える。`cancel_origin = User` なら `None` に戻す、遷移表)。すぐには確認を出さない。`CheckingUnchanged` での `app.quit` は待たない (遷移表)。
2. 保存が **5 秒以内** に終われば (`app.quit` を受けた時点の Editor の「今」に `Millis(5000)` を足した期限を `next_deadline` に載せ、`tick` で判定する。02 §4.4 / §5.4) 確認を出さずに完了の規則に従う: 成功して dirty でなければ終了、保存中の編集で dirty なら `AskUnsaved`。失敗したら `AfterFailure` に進み、(e) の表で `cont = Quit` を保つ選択 (再試行・別名で保存等) が成功すれば終了する。退避先に保存・中止を選べば終了しない。
3. 5 秒を超えても終わらなければ `ui.confirm.quit_saving` (「{path} の保存が終わっていません — `[W] 待つ` ★ / `[Q] 保存を中止して終了`」) を出す。
   - **待つ** (既定。`Esc` も同じ): 確認を閉じ、`Saving` のまま 2 の規則で待ち続ける (`cancel_origin` は変えない。`Esc` は確認を閉じるだけで、保存の取消し (`cancel_origin = User`) にはならない。04 §7.5)。確認は自動では出し直さない。もう一度 `app.quit` を実行すると、この確認をすぐに出す。
   - **保存を中止して終了**: `cancel_origin = QuitAbort` にし、保存ジョブに取消しを **常に** 送る (ジョブは `ReplaceFileW` の開始以降・直接上書きの 05 §4.4 手順 6 以降なら取消しを無視して完了まで進む。main はその完了を待つ、05 §11.5)。ジョブが終わったら結果に依らず終了へ進む (遷移表の `cancel_origin = QuitAbort` の行): `Saved` なら採用し (§9.3 手順 7)、`Cancelled`・失敗 (`ReplacedMovedToBackup`・`InPlaceFailed` を含む) なら `AfterFailure` に進まない。その時点で dirty な文書は緊急退避と同じ書き手・形式 (§10.1、02 §11.2.1) で退避してから終了する (core は `Effect::Quit { code, rescue: QuitRescue::DirtyDocuments, stderr }` (02 §6.3) を出し、退避は Host が `EmergencySnapshot` から行う。退避の結果は Host が終了時の stderr に `stderr.recovery.*` で出す)。元のファイルは保存前の内容 (または保存を終えた新しい内容) のまま。保存が失敗していたら、失敗のパスを端末の復元後の stderr に出す (04 §15.2): `ReplacedMovedToBackup { backup }` → `stderr.quit_abort.moved_to_backup` (`{path}` と退避パス `{backup}`)、`InPlaceFailed { backup: Some }` → `stderr.quit_abort.in_place_failed` (`{path}` とバックアップのパス `{backup}`)、`InPlaceFailed { backup: None }` → `stderr.quit_abort.in_place_failed_no_backup`、その他の失敗 → `stderr.quit_abort.save_failed` (`{path}`、`{reason}`)。`Cancelled` (何も書いていない) と `Saved` では出さない。これらの行は core が `StderrLine { key, args }` として `Effect::Quit.stderr` に入れ (文言には解決しない)、Host が eve-host の定数表から文言を解決して端末の復元の後に出す (02 §6.3 の「stderr の文言の解決」、07 §17.1)。
   - **M9a の実装**: 保存中の文書が 2 件以上でも、確認の `{path}` は先頭の 1 件だけを示す (「ほか N 件」は M9b で実装した)。「中止して終了」で保存が失敗した場合の stderr の行は、失敗した保存ごとに 1 行ずつ出す。
   - **確認の表示中に保存ジョブの結果が届いた** (`cancel_origin = None` のまま): 確認を閉じてから結果を遷移表の `Saving` (`cont = Quit`) の行のとおりに扱う。`Saved` → 完了の規則で終了へ (`cont = Quit`。保存中の編集で dirty なら `AskUnsaved`)。`Failed { Cancelled }` → 中止 (終了しない)。その他の `Failed` → `quit_saving` の確認を `AfterFailure` の確認 ((e) の表) に **置き換える** (確認を積まない。`cont = Quit` を保つ)。

---

### 9.8 複数文書の操作 (v0.2 の M9a、FR-103 / FR-104 / FR-107)

v0.1 は文書の集合を持ちながら表示と操作を 1 文書に制限してきた (§1 の表、§9.2 の置き換えの手順)。M9a でその制限を外す。本節は、複数の文書があるときに **データを失わない** ための規則を定める。利用者に見える振る舞いの既定は PO の確認を得て確定した (確定 (ユーザー回答)、2026-10-29)。

**安全の不変条件** (どの操作でも守る。M9a の試験の根拠):

1. **確認なしに dirty な文書を捨てない**。終了・閉じる・置き換えの前に、対象の dirty な文書すべてについて FR-107 の 3 択を通す。「アクティブな文書だけ」を見て終了してはならない (v0.1 の実装は単一文書のため、`confirm_if_dirty` がアクティブ文書だけを見る。複数文書では **全 dirty 文書を対象にする** 形に直す)。
2. **確認の途中で内容を捨てない**。連鎖の途中で「破棄」を選んでも、終了が確定するまでは内容を残す (§9.2 の `discard_confirmed` に Revision を付けた印だけを付ける)。後の文書でキャンセルしたら、印は無効になり、すべての文書が元のまま残る。
3. **保存の連鎖は、保存した文書に属する**。`Continuation` はアクティブ文書ではなく、`SaveChain.doc` (保存を始めた文書) に対して続きを行う。保存中に別の文書へ切り替えても、連鎖は元の文書のまま進む。
4. **閉じた文書の内容は、保存の完了まで緊急退避のスロットに残す**。閉じた文書の保存ジョブが実行中の間は、`DocSnapshot` と緊急退避のスロットを取り下げない (§9.2 の閉じる手順 (a)、§10.1)。保存が失敗したときに「退避先に保存」できるようにするため。
5. **緊急退避は 1 文書の失敗で他の文書を諦めない** (§10.1)。

**`Continuation` の拡張**:

```rust
enum Continuation {
    Quit,                         // app.quit。複数 dirty では下の「終了の連鎖」に入る
    Close(DocumentId),            // buffer.close。§9.2 の閉じる手順と共通
    Replace(ReplaceTarget),       // 既存
    Reopen { encoding: EncodingName }, // 既存
}
```

**終了の連鎖** (`app.quit`、dirty な文書が 1 件以上):

| 手順 | 内容 |
|---|---|
| 対象と順序 | dirty な全文書 (`Loading` 中を除く)。アクティブな文書を先頭に、残りは開いた順。連鎖の状態 (`QuitAll { remaining: Vec<DocumentId>, discarded: Vec<(DocumentId, Revision)> }`) は `SaveChain` の外に 1 つだけ持つ |
| 各文書の確認 | その文書をアクティブにして、FR-107 の 3 択 (保存 / 破棄 / キャンセル) を出す。文言は何件目か・残り何件かを含む (`ui.confirm.unsaved` の `{index}` / `{total}`、04 §15.2 に足す) |
| 保存 | 文書ごとの `SaveChain` (§9.7)。成功したら次の文書へ。失敗・キャンセルしたら連鎖全体を中止し、終了しない |
| 破棄 | 不変条件 2 のとおり印だけ付けて次へ |
| キャンセル | 連鎖全体を中止し、印をすべて無効にする。保存済みの文書はそのまま保存済み |
| 全部済んだら | `Effect::Quit { rescue: None }`。破棄の印が付いた文書だけが未保存のまま残るが、利用者が捨てると答えたものである |
| 保存中の `app.quit` | `ui.confirm.quit_saving` (§9.7 (f)) は、保存中の文書が 2 件以上でも 1 つのダイアログにまとめる (`{path}` は先頭の 1 件に「ほか N 件」を足す) |
| 外部終了 (コンソールクローズ等) | 連鎖を待たず、dirty な全文書を緊急退避する (§10.1) |

**バッファ操作** (確定。FR-103、07 §11.6 の `buffer.*`):

| 項目 | 規則 |
|---|---|
| 同じパスをもう一度開く | 新しく開かず、既存のバッファに切り替える (`FsPath` と `file_id` で同一判定、§9.2 の二重オープン判定)。読込中なら読込の完了を待つ |
| `buffer.next` / `buffer.prev` | 開いた順で循環する (一覧の表示順とは別。一覧は最近使った順、04 §8.5) |
| `file.open` / `file.new` (実装済み、M9a の残し) | **今の文書を置き換えず、バッファを足してアクティブにする**。何も捨てないので dirty の確認 (`unsaved_replace`) は出さない。`file.open` は、新しい `Loading` のバッファを足し、読込の間は今の文書を表示し続けて編集もできる (v0.1 の「読込の間は旧文書が読み取り専用」は置き換えがなくなるので不要)。読込が終わったとき、**その間に利用者が別の文書へ切り替えていなければ** 新しいバッファへ切り替える (切り替えていたら、一覧に載るだけで表示は変えない)。読込の失敗・取り消し・確認での中止は、足したバッファを閉じ、今の文書は変えない。読込中に重ねて `file.open` してもよい (読込ジョブは直列、上の「読込中のバッファ」)。`file.new` は無題のバッファを足して、すぐ切り替える。**例外 (何も失わない置き換え)**: 表示中のバッファが、何も入力していない無題のバッファ (パスなし・未編集・空・読込済み) で、読込の完了までそのままなら、`file.open` は開いたファイルでそれを置き換える (起動直後の `Ctrl+O` で空の無題が残らないため。一覧の番号も増えない)。同じパスは上の「同じパスをもう一度開く」のとおり既存のバッファへ切り替える。`Continuation::Replace` / `unsaved_replace` は v0.2 で使わなくなる (開き直し `Reopen` は残る) |
| 閉じた後に表示する文書 | 最近使った順で次の文書。最後の 1 件を閉じたら、無題の文書を新しく作る (終了はしない) |
| 無題の採番 | 起動ごとの通し番号 (`無題-1`、`無題-2` …)。閉じても番号を再利用しない。緊急退避の manifest とスワップの `untitled_name` もこの番号を使う |
| 読込中のバッファ | 一覧に載せる (読込中と分かる印)。切替はできるが、編集はできない (§9.2)。読込ジョブは直列に実行する (複数パスで起動したときは先頭から順に) |
| 検索バー・オーバーレイを開いたままの切替 | 切替の前に検索バーと進行中の検索を閉じ、検索状態は文書ごとには持たない (単一の検索状態を切替で破棄する) |
| 表示されていない文書のジョブの失敗・完了 | 保存の失敗はファイル名つきのメッセージで知らせる。読込の完了は一覧の印だけ変える |
| 文書の数 | 数の上限は置かず、メモリの予算 (NFR-203) で制御する |
| ステータスバー・タイトル | 文書が 2 件以上のとき、ファイル名の後ろに `(2/3)` を出す |
| 起動時の複数パス (FR-101) | `--readonly` と `--encoding` は全パスに掛かる。`+行` は直後のパスに掛かり、`path:行:桁` はそのパスに掛かる。最初のパスをアクティブにする |

**全保存** (`file.save_all`、FR-104 AC-2。確定 (ユーザー回答)、2026-10-29):

| 項目 | 規則 |
|---|---|
| 対象 | dirty で、パスがあり、読み取り専用でなく、保存ジョブが実行中でない文書。**名前の無い (無題の) dirty な文書と読み取り専用の dirty な文書は飛ばし**、名前の入力は求めない。飛ばした件数を完了のメッセージで知らせる (利用者が個別に保存する) |
| 実行 | 開いた順に、文書ごとの保存 (§9.3) を `cont` なしで始める。保存は文書ごとのジョブで、1 件の失敗で他の文書を止めない。表示中の文書は変えない |
| 対話しない | 全保存の保存は **確認を出さない**。保存の前後に確認が要るもの (§9.7 (e) の `AfterFailure` の各確認、区分 B・表現不能の確認、外部変更の `Conflict` の確認) は、その文書を **保存せず失敗として記録し**、理由を添えて一覧に載せる (個別に `file.save` で実行し直すと、通常の確認が出る)。複数の確認が同時に出るのを避けるため |
| 完了 | 全部のジョブが終わったら 1 回だけ知らせる。失敗が無ければ `info.save_all.done` (保存した件数と飛ばした件数)。失敗が 1 件以上あれば、失敗した文書を **一覧にした確認** (`ui.confirm.save_all_failed`。本文は失敗の件数、`details` に 1 文書 1 行で `{path}: {reason}`、選択肢は閉じるだけ。`Esc` も同じ) を出す。保存できた件数は `info.save_all.done` で別に知らせる |
| 保存中の編集 | 保存の間に編集された文書は、保存の完了の後も dirty のまま残る (§9.7 の完了の規則と同じ。全保存の再実行で保存できる) |
| 保存中の `app.quit` | 全保存の途中の終了は、通常の保存中の終了 ((f)) と同じ扱いを、保存中の文書ごとに行う |

**メモリ** (NFR-203): プロセス全体の Undo 履歴の合計が上限 (`editor.undo.total_max_bytes`、既定 256 MiB) を超えたら、履歴が最も大きい文書の古い順に破棄して通知する (01 FR-204 AC-4: 破棄しても dirty は保つ)。文書ごとの予算 (`UndoBudget`) の合計を出す API と、`after_change` で最大の文書を剪定する処理を足す。

## 10. クラッシュ復旧・スワップ・バックアップ

3 つの仕組みを区別する。

| 仕組み | 目的 | タイミング | 版 | 要件 |
|---|---|---|---|---|
| 緊急退避 (emergency save) | パニック・シグナル時に未保存内容を失わない | 異常終了の瞬間に同期的に 1 回 | v0.1 | NFR-302 |
| スワップ (swap) | プロセス kill・電源断・OOM でも直前の内容を復旧する | 最後の入力から 2 秒後、以後変更があるたび (最短 10 秒間隔) | **v0.2 の最優先** (swap-lite、§10.2) | FR-118, NFR-307, OQ-17, OQ-42 |
| バックアップ (backup) | 保存前の版を 1 世代残す | 保存成功時 | v0.3 (Could) | FR-119 |

**データ損失防止の段階化**: v0.1 で必須なのは、原子的保存 (§9.3)、保存直前の外部変更照合 (§9.3 手順 5a)、表現不能・黙示置換の検出と保存の中止 (§8.6 E1 / E3)、無編集の上書き保存を書かないこと (§9.3 手順 0)、区分 B の検出と確認 (§8.6 E2')、終了時の未保存確認 (FR-107)、保存後も Undo を保持すること (FR-204)、巨大ファイルを開く前の警告 (§11.2)、緊急退避 (§10.1) である。スワップは **v0.2 の最優先** とし、v0.1 の既知の制約として「SIGKILL・OOM・電源断では未保存の内容を失う」ことをリリースノートに明記する (リスク受容、OQ-42)。退避からの復旧 UI は v0.1 では最小限 (起動時に「前回の退避があります: パス」と表示し、利用者が手で開く)、v0.2 でスワップと同じ UI (適用 / 別バッファで開く / 破棄、§10.3) にする。

### 10.1 緊急退避 (v0.1, NFR-302)

**方式 (`EmergencySnapshot` から別スレッドで書く) の正本は [02 §11.2.1](02-architecture.md#1121-緊急退避の方式-正本adr-0023)** (ADR-0023、条件付き確定 (S-11))。書き手は eve-host で、OS 別のイベント受信と書き手の起動・配置は [05 §11.4](05-platform.md#114-緊急スナップショット-nfr-302-の実装前提)。本節は **core 側の契約** (`EmergencySnapshot` に公開する内容と退避内容の形式) を定める。退避の書き手は **メインの `catch_unwind` (メインのパニック)・`HostMsg::Fatal` を受けたメイン (他スレッドのパニック)・受信スレッド (Windows のコンソールイベント、(macOS / Linux を出荷する版の) シグナル)** で、どれも `EditorState` に触れずに `EmergencySnapshot` から書く (受信スレッドはメインスレッド以外で書くため)。パニックフックは退避を書かず、02 §11.2.1 の「1 回だけ」の CAS に参加しない (02 §9.1、03 §10.1a)。core は書き出しに関与せず Snapshot を公開するだけにする。

| 項目 | 内容 |
|---|---|
| core の契約 | Host は Editor の状態を変える呼出し (`open_initial` / `handle_input` / `on_job_result` / `tick` / `execute`) の **直後に毎回** `Editor::take_snapshot_updates()` を呼び、返った文書 (本文・`TextFormat`・dirty・パスのいずれかが変わった文書、閉じた文書) のスロットだけを `Editor::emergency_snapshot(doc)` で差し替える (文書ごとの `ArcSwapOption<DocSnapshot>`。打鍵ごとに全文書の Vec を作り直さない。02 §5.4 / 05 §11.4)。`Document::apply` の成功だけでなく、保存の完了 (dirty が変わる)・読込の完了・名前を付けて保存 (パスが変わる) もその呼出しの後に反映される。dirty でない文書 (履歴の現在位置が保存点にある。保存の完了・Undo で保存点に戻った等。保存中に編集が入った文書は保存の完了の後も dirty なので空にしない)・`Loading` の文書・閉じた文書では `emergency_snapshot` が `None` を返し、Host はスロットを空にする (解放する)。空にする条件はこの「dirty でない」だけで判定する (02 §11.2.1 の「公開」と同じ)。例外として、**保存ジョブの実行中に閉じた文書** は、その保存が成功するまで (失敗したら §9.2 の「閉じる手順」(a) で退避先に保存・破棄を選ぶまで) 保存に渡した Snapshot を返し続け、スロットを空にしない。Snapshot の clone は O(1) なのでコストは無視できる。core は Snapshot と `TextFormat` を提供するだけで、書き出し自体には関与しない |
| トリガと役割 | パニック (メインスレッドは `catch_unwind` の後にメインが書く。他スレッドはパニックフックが `HostMsg::Fatal` を送るだけで、受けたメインが書く。フックは書かない)、Windows のコンソールイベント (CTRL_CLOSE、外部からの CTRL_C / CTRL_BREAK)、入力スレッドの死亡 (対話なし。以上が v0.1)。Windows のセッション終了通知 (ログオフ・シャットダウン。ベストエフォート、02 §11.2.1) は v0.2、Unix の SIGTERM / SIGINT / SIGHUP は macOS / Linux を出荷する版。退避は **プロセスで 1 回だけ** 実行し、他の終了経路は退避の完了 (または時間予算の満了) を待ってから終了する (02 §11.2.1)。**シグナル・コンソールハンドラのスレッドがメインを待たずに直ちに退避を書く** (メインが端末への書込でブロックしていても退避される)。その後 `HostMsg::Signal` を送り、最大 2 秒メインの終了を待ち、終わらなければ端末復元をベストエフォートで 1 回試みて終了する (終了コードは 07 §18.4)。ただし `CTRL_CLOSE_EVENT` (と v0.2 のセッション終了) では `Signal` を送るだけでメインの終了を待たず、端末復元もしない (コンソールごと閉じられるため。OS の猶予の中で置換中フラグを待ってハンドラから戻る、02 §11.2.1)。シグナル時は確認を出さずに退避する (OQ-28、確定 (ユーザー回答))。退避の経路では **ブロックするロックを取らない**。詳細は 05 §11 |
| 対象 | dirty なすべての Document (`Loading` 中は除く。v0.1 は単一文書だが、置き換えの途中 (§9.2) は旧文書も対象になるので集合として扱う)。**小さい文書から順に** 書き、時間予算 (Windows の 5 秒等) 内に書けた分を manifest に記録する。**時間切れだけが中断で、1 文書の書込の失敗 (権限・ロック) はその文書を `written = false` のまま次の文書へ進む** (複数文書で、1 件の失敗が他の文書を落とさないため。§9.8) |
| 書き出し先 | `state_dir/recovery/<UTC timestamp>-<pid>/<n>-<basename>.txt` + `manifest.toml` (ディレクトリ構成・命名は 05 §7.5)。権限は所有者のみ (NFR-505)。`state_dir` を作れないときの退避先は 02 §11.2.1 (OQ-163) |
| 内容 | **UTF-8 のまま、改行は変換せず (CR / CRLF / 混在をそのまま)、§8.3 のエスケープ文字もそのまま** 書く (LF への正規化は改行情報を失う)。元の文字コードへの再エンコードはしない (表現不能で失敗し得る)。core は manifest に記録する値 (元パス、`TextFormat` の各値、Revision、保存点の有無) を Snapshot と一緒に公開する。**manifest のキー・型・必須・`format_version` と未知の版の扱いは 02 §11.2.1 が正本** で、版付きの公開面として v0.1 で確定し、スワップ (§10.2、v0.2) と共用する |
| 秘密ファイル | 秘密ファイルパターン (`.env` 等。パターンの正本は 07 §5.12.2) に該当する文書も **退避する** (P1 > 機密性。失うより残す) が、manifest の文書に `sensitive = true` を付け (core は `DocSnapshot.sensitive` で公開する。キーの定義は 02 §11.2.1)、復旧案内で退避先パスを明示して削除を促す。スワップ (§10.2) は既定で除外する |
| 書き出し | 同期・ベストエフォート (失敗しても端末復元へ進む)。eve-platform の Host 部品 (`std::fs` を直接使う) を eve-host が呼んで書く (Port 経由では注入状態に左右されるため。02 §11.2.1 / 05 §11.4) |
| 復旧 | 次回起動時に `recovery/` を走査し、v0.1 は退避の場所 (`msg.recovery.found`、「退避先に保存」の退避だけなら `msg.recovery.found_rescue`。退避の中身は読まない、02 §9.3) を表示する (利用者が手で開く)。走査は `Editor::open_initial` が投入する `io` のジョブ `JobKind::RecoveryScan` (02 §9.3。文書に属さないので `JobRequest.doc` は `None`、結果は特定の文書に紐づけず Editor が受ける) が `FileSystem::list_recovery_dirs` (02 §4.5) で行い、main は結果 `RecoveryFound` を示すだけ (メインスレッドは I/O をしない)。走査の対象は `recovery/` 直下の、05 §7.5 の「退避ディレクトリの名前の形」(`<UTC timestamp>-<pid>[-<n>]`) に合う緊急退避のディレクトリと、`rescue/` (§9.7 (e) の「退避先に保存」) の中の同じ形のディレクトリだけで、**`inflight/` (保存中フラグ) は走査も削除もしない** (フラグは `inflight_for` / `list_inflight` だけが扱う)。それ以外の名前のエントリは無視する (05 §7.5 の「`recovery/` の走査と削除の規則」)。v0.2 で該当ファイルを開いたとき (または起動直後に一覧で) 適用 / 別バッファで開く / 破棄を提案する (§10.3)。30 日を超えた退避は起動時に一覧で知らせ、利用者の確認を得て削除する (v0.2。自動では消さない) |

> **v0.1 で守れない経路**: SIGKILL 相当 (Windows の `TerminateProcess`。タスクマネージャの強制終了等) / OOM (Rust では abort になり `catch_unwind` できない) / 電源断 / CTRL_CLOSE が届かないタブ閉じ / Windows のログオフ・シャットダウン (v0.2 で隠しウィンドウの方式) では緊急退避は動かず未保存内容を失う。巨大ファイルを開いて OOM になると **他の dirty な文書もまとめて失う**。v0.1 ではこれを **リスク受容** とし (OQ-42)、リリースノートとヘルプに明記する。緩和策: (a) 巨大ファイルを開く前の固定閾値警告と不正バイト比率の警告、読込中のデコード後のサイズの検査 (§11.2、§9.2 手順 3) を Must の前提条件とする、(b) 利用者が決めるサイズの確保に `try_reserve` を使う (§9.2)、(c) v0.2 の swap-lite (§10.2) を最優先で入れる。

### 10.1a 他スレッドのパニック

- ワーカーはジョブ単位で `catch_unwind` し、`JobResult::Failed(JobError::Panic)` に変換してメインへ返す (エディタは継続。該当 Document の状態は変えない)。
- 入力スレッドのパニック・死亡は `HostMsg::Fatal` としてメインへ届け、メインは緊急退避 → 端末復元 → 終了する (操作不能のまま残さない)。
- パニックフックはどのスレッドでも **端末に触れない** (復元済み端末への描画を防ぐ)。`run_job` (と Host の作業) を実行中のワーカー (スレッドローカルの印 `IN_JOB` が立っている) のフックは **ログだけを書き、`HostMsg::Fatal` を送らない** (ジョブは上の `catch_unwind` で `Failed { err: JobError::Panic }` で終端し、エディタは続く)。それ以外のメイン以外のスレッド (入力スレッド等) のフックは `HostMsg::Fatal` を送るだけにする。正本は 02 §9.1、詳細は 05 §11.4。

### 10.2 スワップ (v0.2, FR-118 / NFR-307)

**方式: swap-lite = 「定期的に呼ばれる緊急退避」。** 64 MiB 以下の dirty な文書の **全文** を、緊急退避と同じ形式 (UTF-8 + 版付き manifest、§10.1) で `swap/` に原子的に書く。**書き手は `bg` ワーカーのジョブ** (`JobKind::Swap { dir, doc: DocSnapshot }`、02 §9.3。Port は run_job が JobPorts から取る) で、`FileSystem` Port を `SaveOptions { private: true, state_file: true, expected: Expect::Any, .. }` で使い原子的保存で置き換える (02 §11.2.1 の「スワップの書き手」)。緊急退避の書き手 (eve-platform の Host 部品 (`std::fs` を直接使う) を eve-host が呼ぶ) とは別だが、本文と manifest を作るシリアライズの関数は共用するので小さく作れる。

| 項目 | 内容 |
|---|---|
| ファイル | 配置・命名は **05 §7.5 が正本** (`state_dir/swap/` 配下。名前は正規化パスのハッシュと pid から作り、無題は DocumentId と pid から作る)。復旧時はハッシュ前方一致で同じ文書のスワップを探し、pid の生存確認 (05 §5.3) で「他インスタンスが開いている」と「残骸」を区別する。所有者のみ権限 (`SaveOptions.private`、F7) |
| 形式 | 緊急退避と同一 (本文は UTF-8・改行非変換・PUA エスケープ保持。manifest は 02 §11.2.1) |
| 対象 | dirty な文書のうち **`file.swap.max_bytes` (既定 64 MiB) 以下** のもの。超える文書は対象外であることをステータスに表示する |
| 除外 | 秘密ファイルパターン (正本は 07 §5.12.2。設定 `file.swap.exclude_secret_files`) に該当する文書は **既定でスワップしない** (平文で状態ディレクトリに写さない)。`file.swap.exclude_secret_files = false` で除外をまとめて止められる (ファイルやパターンごとに有効化する手段は無い。照合は 07 §5.12.2 の一覧だけで行う)。読み取り専用・Loading・バイナリの Document も対象外 |
| 書き出しタイミング | 最後の Transaction から `file.swap.interval_ms` (07 §5.2、既定 2000、Clock の deadline) 後。以後は変更があるたび、ただし同じ文書は最短 `file.swap.min_interval_ms` (既定 10000) 間隔。止まらずに打ち続けて `interval_ms` の空きが来なくても、書けていない変更が `min_interval_ms` 続けば書く (前の書出しからは `min_interval_ms` 空ける)。書込中に来た変更は、気づいた時刻を記録しておき、完了後に期限どおり最新の Snapshot で書く。`bg` スレッドのジョブで実行し UI を止めない (`DocSnapshot` を渡す)。同じ文書のスワップのキューは **深さ 1**: 書込中のスワップはキャンセルせず (書きかけのスワップを残さない)、その間に来た要求は文書ごとに 1 つに畳み、完了後に **最新の Snapshot で 1 回だけ** 再実行する (02 §9.1 の `bg` の例外) |
| 削除 | 保存成功 (dirty = false) で削除、または正常終了時に削除。dirty で終了 (破棄を選択) した場合も削除 (ホットエグジットは OQ-17 により既定無効) |
| 復旧 (§10.3) | スワップは全文なので元ファイルの状態に依存せず常に復元できる。元ファイルが外部で変更されていれば、その旨を示して「別バッファで開く」を既定にする。**v0.2 の M6 では複数文書 (FR-103、M9a) が無いので「別バッファで開く」は出さず、「適用 / 破棄」の 2 択にして、元ファイルが外部で変更されている旨を示した上で適用を確認する (確定 (ユーザー回答)、2026-10-28)。「別バッファで開く」は M9a で足す** |

### 10.3 復旧の提案と適用 (v0.2。v0.1 は退避先の表示のみ)

1. 起動時 (または対象ファイルを開くとき)、`swap/` と `recovery/` から該当エントリを **ジョブで** 探す。別インスタンスが同じファイルを開いている可能性 (スワップのロック情報: PID + 起動時刻) を確認し、生きていれば「他のインスタンスで開いています」と警告する。
1b. **置換途中の痕跡 (Windows)** (起動時の一斉走査と復旧の提案は M6e で実装した (2026-10-30)。M6 が実装したのは 1 と 2〜4 のスワップの分): 保存ジョブは開始時 (一時ファイルを作る前) に **保存中フラグ** (`recovery/inflight/<hash(path)>/<pid>-<seq>.toml`、05 §7.5) に対象パスと tmp / bak 名を書き、成功時に消す (退避名・一時ファイルを消せなかったときは残す、05 §4.1 手順 7 / 8) (書けなければ、原子的保存では警告して保存は続け、直接上書きでは書かずに `BackupFailed` を返す。§9.3)。**開く時の検出は v0.1** で、開くたびに (T の有無に依らず) そのパスのフラグを調べ、T が無いとき、または T があり `method = InPlace` の記録があるときに警告する (§9.2 手順 2、05 §4.6)。**起動時の一斉走査は v0.2** で、`list_inflight` で残ったフラグの一覧を得て、各フラグに **記録された** 対象パス・tmp 名・bak 名についてだけ `metadata` を取る (O(フラグ数)。利用者のディレクトリを列挙して `.<name>.eve-tmp-*` / `.<name>.eve-bak-*` を探すことはしない)。直接上書きの事前バックアップはフラグを消した後に削除するので (05 §4.4 手順 9)、フラグの無いバックアップが残り得る。そのため evoeditor 自身の `state_dir/backup/` の直下だけは列挙し、どのフラグにも記録されていないバックアップを孤児として示す (05 §4.6)。`eve-bak` は置換直前の **元ファイル**、`eve-tmp` は書き込み途中または置換に失敗した **新内容** である。以下の提案はすべて v0.2 (v0.1 は §9.2 手順 2 の場所の案内だけで、「bak を元の名前に戻す」操作は提供しない、§9.3 手順 7): (a) 元ファイルが存在せず bak だけがある → 「bak を元の名前に戻す」を既定で提案、(b) 元ファイルも bak もある → bak は置換成功後の削除漏れの可能性が高いので「削除」を既定、(c) tmp がある → 内容が完全か分からないため「別バッファで開いて確認 / 削除」を提案。判定の詳細と OS 固有の手順は 05 §4.6。Unix の rename は原子的なので、この手順が意味を持つのは Windows だけ。
2. 復旧候補があれば **Document を通常どおり開いた上で** 「復旧内容を適用 / 復旧内容を別バッファで開いて比較 (v1.L: 差分ビュー) / 破棄」を提案する。M6 の時点では「別バッファで開く」を出さない (上の表の復旧の行)。
3. 「適用」は `Transaction(Origin::Recovery)` として全置換 (または最小差分) を適用し、**dirty = true、Undo 可** とする。ユーザーが保存するまでディスク上のファイルは変わらない。
4. 復旧候補は適用・破棄のいずれかを選ぶまで削除しない。破棄も確認を求める。
5. **複数文書** (v0.2 の M9a、§9.8): 提案の対象は、**表示している文書** に当たる候補 (同じパス、または空の無題)。複数パスで起動したときは、文書をアクティブにした時点で、その文書の候補を提案する (確認ダイアログの表示中は保留し、閉じたら出す)。空の無題に当たる候補が複数あるときは、新しい順に 1 件ずつ提案する (適用か破棄を選んだら次を出す)。提案の後に文書が閉じられたら、候補は一覧に戻す (ディスクに残っているので、次回起動でも見つかる)。
   - **M6 の実装**: 提案は「適用 / 破棄」の確認ダイアログ 1 つで、「破棄」の選択が破棄の確認を兼ねる (Esc で閉じれば何もせず、スワップは残る)。提案は起動時の `RecoveryScan` の結果と、tick での見直し (後から `file.open` で開いた文書)。出すのは読込済みで dirty でない文書だけ (編集中の内容を上書きしない)。該当は、同じパス (Windows は大文字小文字を区別しない)、または無題で空の文書。元のファイルがスワップの後に変わっていれば、その旨を詳細に添える (`disk_stamp` の更新時刻とスワップの manifest の更新時刻の比較)。適用した古いスワップは、新しいスワップが書けるか、文書が保存・閉じられるまで消さない。書いたプロセスが動いているスワップ (他のインスタンス) は提案せず、同じファイルなら警告だけ出す。

### 10.4 バックアップ (v0.3, FR-119 Could)

- 保存成功時に、置換前の元ファイルを **`state_dir/backup/` (既定)** または同ディレクトリの `name~` (設定で選択) へ **1 世代** 残す。既定を状態ディレクトリにする理由: `name~` を同ディレクトリに置くと Web 公開ディレクトリでの漏洩 (`config.php~`) や git への混入の古典的な原因になる。原子的置換と組み合わせるため、Port の `write_atomic` に `SaveOptions.backup` (02 §4.5。v0.3 で使う予約で、v0.1 / v0.2 は常に `None`) として渡し、置換直前に元ファイルをコピー (別ディレクトリ) または rename (同ディレクトリ) してもらう。
- 既定は無効 (07 §5.2 `file.backup.enabled`、`file.backup.dir` の既定は空 = 状態ディレクトリ)。v0.1 ではバックアップの除外パターンを指定する設定は無い (後の版で検討する)。

---

## 11. 巨大ファイル戦略

### 11.1 基本方針

- **全文メモリ常駐 (rope)** を維持し、mmap や部分ロードは v0.1 で採用しない ([ADR-0004](10-decisions.md#adr-0004-テキストバッファのデータ構造), [OQ-05](10-decisions.md#oq-05-巨大ファイルの上限と扱い))。
- 「固まらない」は **UI スレッドで全文走査をしない** ことで実現する。行数・行位置は rope から O(log n)、書記素・幅は可視範囲のみ、検索・解析・保存・スワップはジョブ。
- 巨大さには 2 種類あり、別々に対策する: (a) **バイト数が大きい** (100 MB〜1 GB、行は普通) → 読込時間とメモリ、(b) **1 行が長い** (10 MB の 1 行、JSON/ログ) → 行内走査。

### 11.2 サイズ閾値と挙動

| 閾値 (既定、設定可。キー名は 07 §5.2 が正本) | 挙動 | 要件 |
|---|---|---|
| `file.large_file_threshold` = 20 MiB (**v0.4**。v0.1 には無効化する対象の機能が無いため、キーと枠組みはハイライトと同時に入れる) | FR-123: シンタックスハイライト・インデント自動検出・括弧対応を無効化し、ステータスに「大きいファイル: 一部機能無効」を表示。往復照合 (§8.7) は **無効化しない** (ストリーミングで安価) | FR-123, ADR-0015 |
| `file.huge_file_warn_bytes` = **512 MiB (固定閾値、v0.1。OQ-05、確定 (ユーザー回答))**。v0.1 は定数で、設定キーの公開は v0.2 (M9d で公開した) | 開く前にジョブ内 `metadata` のサイズで判定し警告: 「大きなファイルです (N MB)。メモリ不足になる可能性があります。読み取り専用で開く / 中止」 (§9.2 手順 2)。サイズが閾値以下でも、sniff の後の **デコード後の見積り** (文字コードの倍率と不正バイトの PUA 展開 (約 4 倍) を掛けたもの) と、読込中の **デコード後のバイト数・読んだバイト数** が閾値を超えたら、本文を捨てて同じ確認を出す (§9.2 手順 3 の「読込中のメモリ検査」。読込中に伸びたファイル・デコードで膨らむファイルへの対策) | FR-127, OQ-05, NFR-104 |
| `file.huge_file_refuse_bytes` = なし (既定) | 固定上限は設けない。設定で指定可 | OQ-05 |
| 不正バイトの比率 > 1% (定数。先頭の sniff 範囲 64 KiB で判定) | 開く前 (本文の構築の前) に「メモリを約 N 倍使う」と警告 (§8.3 / §9.2 手順 3。`ConfirmAsk::ManyInvalidBytes`。512 MiB の警告とは別のキー `ui.confirm.open.invalid_bytes` で、選択肢は「読み取り専用で開く / 文字コードを指定して開く / 中止」)。「読み取り専用で開く」の理由は `InvalidBytes` (§9.6)。不正バイト 1 つは UTF-8 で 4 バイトになるため、バイナリに近いファイルではテキストが元の約 4 倍になる | FR-127、02 §14.1 MR4 |
| `file.long_line_threshold` = **1 MiB** (1 行。v0.1 は定数、v0.2 の M9d で公開した。文書は開いたときの閾値を持つ) | v0.1: その行を含む文書を **読み取り専用** (`LongLine`) で開き、その行は先頭 64 KiB まで表示する (§11.5)。v0.2: その行を **char 近似モード** (書記素 = char、幅 = char ごとの表引き、折り返し OFF) で扱い編集できる。検索ハイライトは可視範囲のみ、ハイライトは行単位で打ち切り | NFR-106 |

- 警告の閾値は **固定値 (512 MiB)** とし、v0.1 ではメモリ量に連動させない (OQ-05 で確定)。メモリ量への連動は v0.4 以降で、`SystemInfo { available_memory(), logical_cores() }` を 05 §2 に追加し (OS 別の「利用可能メモリ」の定義: Linux `MemAvailable`、Windows `GlobalMemoryStatusEx`、macOS `host_statistics64`、cgroup 制限)、`file.huge_file_warn_ratio` (07 §5.2) で連動させる。1 GB は v0.1 では参考値の記録だけにし、512 MiB の警告は Must のまま。
- 読み取り専用で開いた巨大ファイルは Undo 履歴・スワップを持たないため、メモリ倍率が小さい。
- 長時間の稼働で増え得る構造には v0.1 から定数の上限を持つ: メッセージ履歴 1,000 件、レイアウトキャッシュは可視行の 2 倍、検索履歴 100 件、検索結果 (§12.2)、Undo (§7.5)。

### 11.3 段階的ロードと早期表示 (NFR-104)

- **v0.1**: 100 MB は「開く操作から 50 ms 以内に進捗を表示し、入力に応答し、キャンセルできる。全読込は 30 回計測の p50 1.0 s で判定し、最大値は参考値 (目安 1.5 s)」で判定する (§9.2、01 NFR-104)。読込中は `Loading` (読み取り専用・本文なし) で、進捗だけを Frame に出す。
- **v0.3**: §9.2 の `Partial` により、100 MB で 300 ms の「先頭部分の先行表示」を満たす (1 GB の値は NFR-104 の参考値)。先頭 1 MiB のデコードと rope 構築が最初の Partial の支配コスト。Partial の間は読み取り専用で、スクロールは **読み込み済み範囲内** に制限する。`+行番号` や検索は Complete まで保留 (進捗を表示)。
- 読込・保存は `io` スレッドで直列に実行する ([02 §9.1](02-architecture.md#91-スレッド一覧-adr-0006))。複数ファイルを同時に開いた場合、アクティブな Document の読込を先に投入する。

### 11.4 行インデックス

- 専用の行インデックスは **作らない**。rope が行区切り数をノードに持つため、`len_lines` / `line_to_byte` / `byte_to_line` は O(log n) で得られる (T3)。
- 行区切りの種類 (LF/CRLF/CR) ごとの件数 (`EolStats`) のみ読込時に数え、編集時は §6.5 の「eol_stats の更新」の規則で増減する (編集の端の前後 1 バイトの窓だけを数え、削除が大きいときは行数の差で補う。文書長に依存しない)。
- 読込中 (v0.3 の Partial) の行数は「読み込み済み部分の行数 + 推定 (バイト数比例)」を表示し、Complete で確定する。

### 11.5 長大行 (NFR-106)

- 問題: 10 MB 行で「カーソルの表示列」「表示列 → Pos」「折り返し位置」を求めるには行頭からの走査が必要で、書記素分割 + 幅計算のスループットを数十〜百数十 MB/s とみると **1 回 50〜300 ms** かかり得る。毎キー入力で行うと NFR-103 を破る。
- **対策 (v0.1、Must): 編集しない縮退 (読み取り専用での表示)** (文書末が長大行のときの `cursor.doc_end` / `select.all` の丸めと、全選択のコピーの扱いは §5.4)。読込ジョブ (§9.2 手順 3) が行の長さを数え、`file.long_line_threshold` (1 MiB、v0.1 は定数) を超える行が 1 つでもあれば、その文書を読み取り専用 (理由 `LongLine`、§9.6) で開く。閾値超の行は **先頭 64 KiB (書記素の境界で切る) までだけをレイアウトの対象** とし、その先は行末に「… (以下 N バイト省略)」の印を出して描かない。カーソルと選択はその範囲に限り、End はその範囲の末尾へ動く。行の残りはメモリに保持し (省略は表示だけ)、文書は編集しないので保存で失われることもない (無編集の上書き保存は書き込まない、§9.3 手順 0)。検索の一致がその行の表示範囲より後ろにあるときは件数に数え、移動先としては「長大行の表示範囲外」と示して移動しない。64 KiB の範囲は通常の書記素分割と幅計算で走査するので、char 近似は要らない。**判定は「10 MB の行を含む文書での各操作 (表示・End・水平と垂直のスクロール) が p99 ≤ 100 ms」** (v0.1 Must、01 NFR-106 AC-3、09 BM-106)。ハングを防ぐことが目的で、閾値以下の行と他の行は通常どおり表示する。
- **対策 (v0.2、Must): char 近似モード (編集)**。`file.long_line_threshold` (1 MiB) を超える行は、書記素 = char、幅 = char ごとの表引き (その char だけを 1 書記素とした通常の行の幅。結合文字は `◌` 付き、ZWJ・VS は不可視文字の記号になり、幅 0 にはならない。§11.5.1)、折り返し OFF で扱う。char ごとの幅の表引きは数百 MB/s 出る見込みで 10 MB 行で 30 ms 前後、ASCII の連続部分に高速経路 (memchr 系の SIMD 走査で非 ASCII の初出を探し、その手前は表引きも省く) を入れればさらに速い。**判定は「10 MB の行での各操作 (表示・End・中央挿入) が p99 ≤ 100 ms」** (v0.2 Must。OQ-31、確定 (ユーザー回答)。01 NFR-106 AC-1)。ジョブ化もキャンセルも不要で、「固まらない」を測れる上限に言い換えたもの。近似により結合文字がカーソル位置では分離して見えることがあるが、閾値超の行に限った既知の制約とする。
- 通常行 (閾値以下) でも、ASCII / 結合しない BMP 文字の連続は「書記素 = char、幅は表引き」とみなす高速経路を `eve-text` の走査に入れる。ログ・JSON のような行の大部分はこれで走査コストが 1/10〜1/50 になる見込み (v0.1 から入れる。閾値以下の長い行の NFR-103 を守るため)。なお、閾値 (1 MiB) 以下の長い行は読み取り専用にならず編集できる。§12.2 の可視範囲のハイライトが 1 行あたりメインスレッドで検索する範囲の上限 (16 KiB、`SEARCH_SYNC_LINE_MAX`) は閾値とは別の値で、閾値以下でも広げた範囲が 16 KiB を超える行の検索はジョブに回る (届くまでその行にはハイライトを出さない)。
- **閾値以下の長い行 (例: 300 KB の日本語の段落)**: 縮退も char 近似モードも適用されず列チェックポイント (v0.3) も無いため、毎フレーム行頭から書記素分割と幅計算をすると NFR-103 を破り得る。v0.1 は **行ごとに直前に計算した `(Pos, 表示列, 書記素数)` を 1 つキャッシュ** し (View ごと、Revision と行で無効化)、カーソル付近の変換はそこからの差分走査で求める。S-03 で不足が分かった場合は、このキャッシュを強化 (複数点化) する。
- **対策 (v0.3)**: [§14.4](#144-長大行の列チェックポイント) の **列チェックポイント** (行内 4 KiB ごとに `(byte, display_col, grapheme_count)` を記録) で、閾値超の行でも書記素単位の正確さを保ちつつ各操作 50 ms を目標にする。v0.1 では構造体の差し替え点 (`LineLayoutCache`) のみ予約し、実装しない。
- 書記素境界の判定は任意位置から **後方 256 バイト** のコンテキストで行い、それより長い結合列は正確さを諦める (実用上 ZWJ 絵文字列は 50 バイト未満)。**既知の制約**: 地域指示記号 (国旗) の対の偶奇判定は原理的に無制限の後方参照を要し、128 個を超える連続する国旗列では境界を誤り得る。OQ-53 の `GraphemeCursor` + `PreContext` 方式でも同じ制約があり、受容する。
- 折り返し ON での長大行は表示行数が数十万になる。表示行 → 論理行の写像はチェックポイントから近似し、ビューポート近傍だけ正確に計算する。
- 検索の一致ハイライトは可視範囲の一致のみ求める (§12.2)。

#### 11.5.1 char 近似モードを使う側の突合と実装の分け方 (M9d、2026-10-31。案)

v0.1 の縮退 (読み取り専用で先頭 64 KiB だけ) を、v0.2 の char 近似モード (編集できる) に置き換える。行のレイアウトを使っている箇所を洗い出し、それぞれの扱いを先に決める。

**今の作りの問題**: 行の表示・カーソルの列・列からの位置は、すべて `line_content` (行全体を `String` にする) と `units_for` (行全体を表示の単位 `Unit` に展開する。1 単位ごとに文字列を持つ) を通る。単位の展開は (Revision, 行) で 1 行分だけキャッシュする。10 MB の行ではこの展開が数百 MB のメモリと数百 ms を使うので、v0.1 は先頭 64 KiB に切って避けている。char 近似モードでは **行全体を展開しない別の経路** が要る。

**対象の判定**: 行ごとに「その行のバイト長 > 文書の `format.long_line_threshold`」(開いたときの閾値、07 §5.2)。行の長さは行インデックスから O(1) で分かるので、表示のたびに判定してよい。編集で行が閾値を超えた・下回ったら、その時点から経路が替わる。読込ジョブは `LongLine` の理由を付けない (§9.6 の表のとおり)。v0.1 の先頭 64 KiB の切り取り (`LONG_LINE_VISIBLE`・`line_visible_end`・`clamp_to_visible`) と、行末の「以下 N バイト省略」の印はやめる。

**核となる仕組み**: 長大行の **列の索引** を 1 つ作り、下のすべての利用者がこれだけを根拠にする。

- 行頭から char を 1 回走査し、64 KiB ごとに `(バイト位置, 表示列, char 数)` を記録する (10 MB の行で約 160 点)。幅は char ごとの表引きで、その char だけを 1 書記素とした通常の行の幅と同じ (結合文字は `◌` + 結合の幅 1、ZWJ・VS などは不可視文字の記号の幅。幅 0 にはならないのでカーソルを置ける)。タブは次のタブストップ、制御文字・不正バイトの記号 (`<0xHH>` 等) も通常の行と同じ幅。印字できる ASCII の連続は表引きを省く高速経路で数える。
- 索引は (文書, Revision, 行) をキーに、通常の行の `units_for` と同じく直近の 1 行分をキャッシュする。編集で Revision が変われば作り直す (その行の 1 回の走査。10 MB で 30 ms 前後の見込み。NFR-106 の 100 ms に収める)。
- バイト位置 → 表示列 / char 数、表示列 → バイト位置は、手前の記録点からの差分走査 (64 KiB 以内) で求める。
- 書記素 = char とみなす (カーソルは char の境界に止まる)。近似によって、結合文字がカーソル位置で分かれて見えることがある (§11.5 の既知の制約)。

| 利用者 | 今 (通常の行) | 長大行 (char 近似モード) での扱い |
|---|---|---|
| Frame の行の生成 (`build_row`) | 行全体の `Unit` から `left_col` 以降を切り出す | 索引で `left_col` のバイト位置を求め、そこから画面の幅の分だけを char 単位で単位にする (行全体を `String` にしない)。選択・検索の強調・カーソルの印はバイト範囲で突き合わせるので変更なし |
| カーソルのセル位置 (`cursor_cell` / `cursor_col`) | 行全体の `Unit` を探す | 索引で表示列を求める。セルの幅は、その char の幅 (0 なら 1) |
| 列 → 位置 (`byte_at_col`。垂直移動の目標列・マウス) | 行全体の `Unit` を探す | 索引で求める。セルの途中は前に寄せる (今と同じ) |
| 左右の移動・Backspace / Delete の単位 | 書記素の境界 (`prev_` / `next_grapheme_boundary`。後方 256 バイトの文脈) | char の境界。`editor.backspace_unit` の値に関わらず char 単位 |
| 単語の移動 (`next_word_stop` / `prev_word_stop`) | 行全体を文字列にして単位の境界を求める | 行全体を読まない: 位置の前後の窓 (64 KiB) だけを見る。窓の中に境界が無ければ窓の端に止まる (近似) |
| `Home` / `End`・インデント終端 (`indent_end`) | 行全体を文字列にする | `End` は行末 (表示列は索引から)。インデント終端は行頭から空白の連続だけを読む (行全体を読まない) |
| Tab キーのインデント (`display_col`) | 行頭からカーソルまでを文字列にして幅を数える | 索引でカーソルの表示列を求める |
| 折り返し (`wrap_line`) | `Unit` を表示行に切る | **長大行を含む文書を表示する View は、折り返しなしとして扱う** (`view.wrap` の値は変えない。長大行が無くなれば戻る)。表示行数が数十万になる折り返しの写像は v0.3 の列チェックポイント (§14.4) と一緒に入れる。判定は文書の「長大行の数」で行い、読込で数え、編集のたびに変わった行だけを見て増減する |
| 横スクロール (`left_col` の追従) | カーソルの表示列から決める | 同じ (表示列を索引から求める) |
| 検索の強調 | 可視行を行ごとに検索し、16 KiB を超える行はジョブに回す (§12.2) | 変更なし (長大行は可視の範囲に一致があっても、ジョブの結果が届くまで強調しない)。一致への移動は通常どおり (表示範囲外の扱いはやめる) |
| ステータスバーの桁 (`ui.column_unit`) | 行頭からの書記素数・表示列 | 索引の char 数・表示列。縮退の表示 (`status.degraded.long_line`) は「長い行は近似で表示している」意味に変える (01 NFR-106 AC-2) |
| `cursor.doc_end` / `select.all` / コピー (§5.4 の長大行の丸め) | 先頭 64 KiB に丸める | 丸めない (通常の行と同じ) |
| 保存・スワップ・Undo | 行の形に依らない | 変更なし |

**閾値以下の長い行** (例: 300 KB の段落) は今のまま (行全体の展開と 1 行分のキャッシュ、§11.5)。

**実装の分け方 (案、PR 3 本)**:

1. **列の索引** (済み。core の `layout::long` の `ColIndex` / `CharWidth` と、eve-text の `next_word_stop_near` / `prev_word_stop_near`): 索引の構築・バイト位置 ↔ 表示列 / char 数・char 単位の窓での単語境界。ASCII / 日本語 / 絵文字混在 / タブ / 制御文字で、通常の行の `Unit` の結果と幅が一致すること (書記素が char と一致する範囲で) を試験で確かめる。
2. **core の切り替え**: 上の表の利用者を長大行で索引に切り替え、`LongLine` の理由と先頭 64 KiB の切り取りをやめる。折り返しの扱い (長大行の数)。v0.1 の長大行の試験 (`long_line.rs`) を v0.2 の扱いに書き直す。
3. **表示と計測**: ステータスバーの縮退の表示と文言、BM-106 (09 §5.2。10 MB の 1 行の ASCII / 日本語 / 絵文字混在で、表示・`End`・中央挿入が各 p99 ≤ 100 ms) の計測と記録。

### 11.6 mmap の可否

mmap は **v1.X まで使わない**。mmap 中にファイルが切り詰められると SIGBUS (Unix) / 例外 (Windows) になり NFR-305 (パニックしない) と相容れず、Windows では mmap 中のファイルを他プロセスが削除・置換できず自分自身の原子的保存もできないため。v1.X の「閲覧専用モード (編集不可、検索のみ)」で、別の `TextStorage` 実装として扱うかを決める (OQ-05)。

### 11.7 メモリ見積り (NFR-202)

| 項目 | 見積り |
|---|---|
| rope 本体 | 元 UTF-8 サイズの約 1.1〜1.4 倍 (葉の充填率 ~70%、ノードメタデータ) |
| 読込中の一時 | チャンク 1 MiB + Decoder バッファ ≒ 数 MiB |
| 非 UTF-8 ファイル | UTF-8 化で Shift_JIS は約 1.5 倍、UTF-16 は 0.5〜1.5 倍のサイズ変化。NFR-202 の基準は **デコード後の UTF-8 のサイズ** (元のファイルサイズではない) |
| 不正バイトの多いファイル | 1 バイト → 4 バイト。比率 1% 超は警告 (§11.2)。NFR-202 の対象外 |
| Undo 履歴 | 文書あたり 64 MiB、プロセス全体 256 MiB (§7.5)。小さな削除は `Arc<str>` にコピーするため Snapshot 保持による未計上の増加はない (§6.5) |
| レイアウトキャッシュ | 可視行の 2 倍まで、O(ビューポート) |
| 検索結果 | 上限件数で打ち切り (§12.2) |
| Frame と frontend の 2 枚のバッファ | < 2 MB |

→ 無編集時 (デコード後サイズ) × 1.5 + 15 MB の目標は満たせる見込み (100 MB の UTF-8 を 1 つ開いて少し編集した状態で rope 110〜140 MB + Undo 0〜64 MB + その他 < 10 MB)。計測は読込完了から 1 秒アイドルの後に行う (ワーカーで確保したメモリがアロケータのスレッド別アリーナに残ることがある)。P0 スパイクで ropey 1.x/2.x の実測が必要 (ADR-0004 検証項目)。

---

## 12. 検索・置換の基盤

### 12.1 検索の種類とモード

```rust
#[non_exhaustive]                          // OQ-41 の準備 (確定)。構築は SearchQuery::literal(pattern) 等のビルダー経由に限る
pub struct SearchQuery {
    pattern: String,
    kind: Literal | Regex,                 // FR-402。v0.1 は Literal のみ (Regex は v0.2)
    case: Sensitive | Insensitive | Smart, // Smart: 大文字を含めば区別 (設定で既定選択)
    whole_word: bool,                      // FR-402。両端を単語境界 (§13.3) で検査 (v0.2。v0.1 は常に false)
    scope: Document | Selection,           // 選択範囲内検索 (置換で有用。v0.2。v0.1 は常に Document)
}
pub struct Match { range: ByteRange, captures: Option<SmallVec<[Option<ByteRange>; 4]>> }
```

- **`SearchQuery` の構築** ([OQ-41](10-decisions.md#oq-41-cp932-互換文字の検索同一視) の準備、確定): `SearchQuery` は `#[non_exhaustive]` とし、フィールドを直接並べた構築 (構造体リテラル) を crate の外で許さない。構築は `SearchQuery::literal(pattern)` 等のビルダー (`with_case(..)` などの設定メソッドを含む) 経由に限る。これで CP932 互換文字の同一視 (OQ-41、v0.3) などのフィールドを後から足しても呼び出し側を壊さない。**v0.1 のフィールドは上の 5 つ** (`pattern` / `kind` / `case` / `whole_word` / `scope`) で、これ以外は足さない。v0.1 のビルダーが作るのは `kind = Literal`・`whole_word = false`・`scope = Document` の値だけで、これらを変える設定メソッド (`with_regex` / `with_whole_word` / `with_scope` 等) は v0.2 で加える (v0.1 で変えられるのは `case` だけ)。検索バーの状態 `SearchBarModel` は eve-view の型で、全型 `#[non_exhaustive]` ([02 §8.4](02-architecture.md#84-境界型の契約と凍結段階)) なので追加の作業は要らない。
- **リテラル検索は `memchr::memmem` でチャンクを跨いで実行** する (境界の重なり分 (パターン長 − 1 バイト) だけ連結して走査)。1 GB/s を超えるため NFR-108 / 109 の目標値はリテラル検索に課す。
- **一致の選び方** (リテラル・正規表現に共通): (1) 一致は **重ならないものを左から順に** 取る。ある一致を採ったら、次の候補はその終端以降から探す (`aa` を `aaa` で探すと 1 件)。チャンク境界の重なり部分で見つかった候補も同じ規則で、直前に採った一致の終端より前から始まるものは捨てる。これにより **結果はチャンクの分割に依存しない** (§22.1 のプロパティテスト)。(2) **両端が書記素境界でない候補は捨て**、件数にも数えない (`e` で `é` (e + U+0301) の `e` には一致しない。`👨` で `👨‍👩‍👧` の先頭には一致しない)。捨てた候補の次は、その候補の先頭の次の char 境界から探し直す (捨てた候補と重なる位置の一致を取りこぼさない)。境界の判定は §13.1 と同じ 256 バイトの文脈で行い、コストは候補 1 件あたり定数。一致は常に書記素境界上にあるので、選択 (§4.1 の N1) と Decoration (§14.5) で丸めが起きない。
- **正規表現 (v0.2) は行単位で実行** する (§12.3)。NFR-108 / 109 は正規表現には課さず、「入力を止めない (NFR-103 を維持する)」ことだけを Must とし、所要時間は参考値として記録する。
- 既定は `Smart` (v0.1 は定数。設定キー `search.case` (07 §5.5) の公開は v0.2)。
- **検索バーでの切替** (FR-402、コマンドとキーの正本は 07 §11.6): `search.toggle_case` は大小区別を **Smart → Sensitive → Insensitive → Smart** の順に巡回する。初期値は検索バーを開くたびに既定値 (v0.2 からは設定 `search.case`)。巡回した値はそのセッション中だけ保持し、設定ファイルには書かない。`search.toggle_regex` (v0.2) は `kind` を Literal ⇔ Regex で切り替える (初期値は `search.regex_default`)。v0.1 の検索バーは `regex` を常に false とする。どちらも when は `search.active` で、切り替えたら直ちに前の検索ジョブをキャンセルして再検索する (§12.2 のインクリメンタルと同じ経路)。状態は `SearchBarModel.case` / `regex` (02 §8.2) に入れ、表示 (`[Aa~]` 等) は 04 §8.4。

### 12.2 実行モデル (FR-404 / NFR-108 / 109)

| 要素 | 方針 |
|---|---|
| 全件走査 | **常にジョブ** (`bg` スレッド。新しい検索要求が来たら古い要求をキャンセル、02 §9.1)。Snapshot と Revision を渡し、結果を `(Revision, Vec<Match>)` のバッチで返す。**バッチは 256 件または 20 ms ごと** (受信口は上限なしなので、送る側がまとめる。02 §9.2)。cancel はチャンク毎に確認 |
| 最初の一致 (NFR-108) | 同じジョブが **カーソル位置から前方** を先に走査し、最初の一致を即時送る。続いて文書先頭から残りを走査。これにより「次へ」の応答と全件カウントを 1 ジョブで賄う |
| インクリメンタル (FR-401) | 検索語が変わるごとに前ジョブをキャンセルし新ジョブを投入。入力の合体 ([02 §9.2](02-architecture.md#92-メインループ-擬似コード)) により連打中の無駄な起動を抑える |
| 件数上限 (FR-404) | 一致範囲の **保持** は `search.max_matches` (07 §5.5、既定 10,000) まで。件数の **カウント** は `search.count_limit` (07 §5.5、既定 **1,000,000**) まで続け、超えたら「1,000,000+」と表示して打ち切る |
| 保持の窓 | 保持する一致は **現在の一致 (無ければカーソル位置) の前後それぞれ 5,000 件** (`max_matches` の半分ずつ) の窓とし、ジョブから届いた一致のうち窓の外のものは捨てる。現在の一致が窓の端から 1,000 件以内に近づいたら、窓を現在の一致を中心に取り直す (取り直しはジョブの単発の再走査、下の `search.next` / `search.prev` と同じ経路)。ジョブは保持とは別に **1 MiB ブロックごとの件数** を返し、main はそれを持つ (100 MB で 100 項目)。現在の一致の番号 (`current`) は「現在の一致を含むブロックより前のブロックの件数の和 + そのブロック内で前にある一致の数」で求める |
| `search.next` / `search.prev` (FR-401) | 移動先が保持の窓の中にあれば、それへ移る (O(log 件数))。**窓の外、全件ジョブの実行中 (`Running`) でその方向の一致がまだ届いていない、または編集で保持が無効になった (`Approx`)** ときは、カーソル (選択の `end`、prev は `start`) から **同期検索** する。同期検索は最大 256 KiB (`SEARCH_SYNC_STEP_BYTES`、定数。1 MiB だと全件検索中のキー入力が p50 2.4 ms になり NFR-103 を超えた、BM-103b-search。行ごとに畳み込む経路 (§12.4) は約 50 MB/s なので 1/16 に絞る) か時間予算 2 ms で打ち切り、そこまでに見つからなければ同じ位置から **単発のジョブ** (`Search` の `find_next_from(pos, dir)`。全件ジョブとは別に 1 つだけ、新しい要求は古い単発をキャンセル) を投げ、結果が届いた時点で Revision と選択が投入時のままなら移動する (違えば捨てる)。文書末尾 (先頭) を越えたら先頭 (末尾) から続ける (折り返した旨をメッセージで示す、04)。`current` は、`Exact` / `AtLeast` ではブロック別件数から求めて表示し、`Approx` (編集後) では **求めず表示もしない** (02 の `Approx { total }` のまま。再検索が終わって `Exact` に戻ると表示される)。上限で打ち切った (`AtLeast`) 後ろの一致へ移った場合は `current` を None (`-`) にする |
| 可視範囲のハイライト | 結果が上限で打ち切られていても、可視範囲については **メインスレッドで小さな検索** を行って Decoration を作る。対象は可視行 (ビューポート ± 1 画面) の各行について、**可視列範囲 (水平スクロールの `left_col` から画面幅まで) のバイト範囲を前後にパターンのバイト長だけ広げた範囲** に限る (行全体を走査しない)。広げた範囲が 1 行あたり 16 KiB (`SEARCH_SYNC_LINE_MAX`、定数) を超える行と、範囲を決めるのに行頭からの長い走査が要る行 (§11.5 の閾値以下の長い行で、§11.5 の列キャッシュから求められないもの) はメインスレッドで検索せず、その行の範囲を **ジョブ** に回して結果が届いたら描く (届くまでその行にはハイライトを出さない)。これで同期検索の量は O(可視行 × 16 KiB) 以下に収まり、NFR-103 内に収まる。正規表現 (v0.2) は範囲を広げる量が決まらないので行全体を対象にし、16 KiB を超える行はジョブに回す。時間予算 1 ms を超えたら残りの行をジョブに回す |
| 編集後の結果 | **v0.1**: 保持中の一致は編集で無効にし (`pending_changes` を持たないため写像しない、§17.1)、可視範囲のハイライトは同期検索で即時に描き直し、件数は「約」表示に落として、入力が止まって 300 ms 後に可視範囲から再ジョブ (01 FR-404 AC-4 はこの挙動を受入条件にしている)。**v1.L 以降** (`pending_changes` 導入後): 保持中の一致を `map_range` で写像し、変更範囲と交差するものだけ無効化して周辺を再検索 |

**v0.1 の実装の範囲** (M4 の途中経過。差は次の版までに埋めるか、この節を直す):

- 保持の窓 (現在の一致の前後 5,000 件、1 MiB ブロックごとの件数、窓の取り直し) は作らない。全件ジョブは **走査した順の最初の `max_matches` 件** を保持し、数えるのは `count_limit` 件まで。保持に無い (走査中・上限超・編集後) ときの `search.next` / `search.prev` は同期検索 (最大 256 KiB) で動き、そこで見つからなければ **単発のジョブ (`JobKind::FindNext`、`bg` ワーカー。進行中は 1 つだけで新しい要求が古いものを取り消す)** で探し、結果が届いた時点で Revision と選択が投入時のままなら移る (違えば捨てる)。保持の窓は、この単発のジョブが答えの正しさを担うので、移動の速さを上げたくなったときの最適化として後に回す。
- ジョブは `bg` ワーカー (eve-host の 2 本目のスレッド。最初の検索の投入で作る) で動き、保存や読込を待たせない。最初の一致は待たずに送り、以後は 256 件または 20 ms ごとのバッチ。
- 走査は、検索語がチャンクごとに探せる場合 (大小の区別がない文字だけ、または ASCII だけで `k` を含まない) は行に切り出さず `memmem` で行う (開発機で約 1.4〜1.9 GB/s、100 MB。`crates/eve-core/tests/search_speed.rs`)。それ以外 (大小を区別しない設定で非 ASCII の区別のある文字か `k` を含む) は行ごとに畳み込んで探す。
- 可視範囲のハイライトは、保持した一致ではなく **可視行をその都度同期検索** して作る (1 行が 16 KiB を超える行は強調しない)。強調は Decoration ではなく、Frame を作るときに行のスタイルへ直接重ねる (一致は下線、入力で見つけた現在の一致は反転 + 太字 + 下線)。Decoration の仕組みは E3 の該当箇所の強調など他の用途が入るときに足す。編集後 300 ms の再検索は未実装で、次の `search.next` / `prev` か入力の変更でやり直す。

### 12.3 正規表現

**版: v0.2** (行単位)。v0.1 は利用者が正規表現を書く経路を持たない。ただし行単位の実行経路そのものは v0.1 から使う: 大文字小文字を区別しないリテラル検索で、大小の区別がある文字を含み ASCII の経路に乗らないパターンは、パターンをエスケープしてこの経路で照合する (§12.4)。

- エンジンは `regex` (線形時間保証、[02 §13.5](02-architecture.md#135-検索))。後方参照・先読みは提供しない (ReDoS 回避、P3 UI を止めない)。
- **行単位の実行 (v0.2 まで)**: 1 行ずつ `&str` に切り出して (チャンクを跨ぐ行は連結して) `regex` を実行する。長大行 (§11.5) はワーカー内でコピーするので、メインスレッドで大きな文字列を作ることはない。**複数行にまたがるパターン (`\n` を含む) は v0.2 では「未対応」と表示** する (`newline_sugar` の糖衣は行内の `\r` の扱いにのみ使う)。
- **複数行の正規表現 (v0.3)** の実行方式は未決 (OQ-60、S-04 で判断)。どの方式でも `TextStorage` の公開 API は変えない。
- **フラグ**: 複数行モード `m` (`^`/`$` が行頭/行末)、**CRLF モード `R`** (`\r\n` と `\r` を行終端として扱う。§8.4 の改行の「保持」方式で `$` が CR の前に一致するために必須)、Unicode モード既定 ON (`\w`, `\b`, `(?i)` が Unicode 対応)。`.` は改行に一致しない (`s` フラグはユーザーが明示)。
- **検索語の `\n`**: リテラル検索では `\n` の入力は不可 (単一行入力欄)。正規表現モードで `\n` を書いた場合は **`\r?\n|\r`** と等価に扱う糖衣を入れる (設定で無効化可。複数行一致は v0.3 から)。
- **構文エラー**: 入力中は赤色表示のみ、ジョブは投入しない。エラーメッセージは regex のものを整形して表示。
- **コンパイル上限**: `size_limit` を設定 (既定 10 MiB) し、巨大パターンは拒否。

### 12.4 単語単位・大文字小文字

- `whole_word` (v0.2): 一致の両端が [§13.3](#133-単語境界-日本語補助規則) の単語境界であることを後段で検査する (regex の `\b` は ASCII/Unicode の `\w` 基準で日本語に不適)。
- `Insensitive`: 意味は **Unicode の単純ケースフォールディング** (CaseFolding.txt の C と S。regex の `(?i)` と同じ) で一致する、と定める。リテラル検索の経路は次の順に選ぶ (v0.1 から。利用者に正規表現は見せない)。どの経路でも結果はこの意味と同じになる。
  1. **大小の区別が無い文字だけの検索語** (どの文字も単純ケースフォールディングの対応表に写像元としても写像先としても現れない。ひらがな・カタカナ・漢字・記号・数字だけの日本語の検索語はこれに当たる) は、`Insensitive` / `Smart` でも **`Sensitive` と同じ** で、`memmem` で検索する (NFR-108 / 109 の目標の対象)。全角英字 (`Ａ` / `ａ`) は区別のある文字なのでこれに当たらない。
  2. **ASCII だけの検索語で `k` `K` `s` `S` を含まない** ものは、ASCII の大小を畳み込んだ `memmem` (ASCII 大小無視の走査) で検索する (NFR-108 / 109 の目標の対象)。
  3. それ以外 (ASCII 以外の区別のある文字を含む、または `k` `K` `s` `S` を含む) は、パターンをエスケープして行単位の正規表現経路 (§12.3) で `(?i)` として検索する。`k` / `s` を 2 の経路から外すのは、単純ケースフォールディングでは `K` (U+212A KELVIN SIGN) が `k` に、`ſ` (U+017F LATIN SMALL LETTER LONG S) が `s` に畳み込まれ、ASCII の畳み込みではこれらに一致しないため (ASCII 以外から ASCII へ畳み込まれる文字はこの 2 つだけ)。この経路の所要時間は正規表現と同じく参考値とし、入力を止めないこと (FR-404) だけを Must とする (提案、OQ-196。代案は「ASCII だけの検索語では ASCII の大小だけを同一視し、K と ſ には一致しない」と意味を定めて 2 の経路を常に使う。速いが、ASCII 以外を含む検索語との間で意味が揃わない)。
- **v0.1 の実装** は `regex` を使わず、大小を区別しない照合を「1 文字ずつ小文字へ畳み込んだ文字列どうし」で行う (小文字が 1 文字になる文字だけ畳み込む)。上の 1・2 の経路は仕様どおり。3 の経路は、単純ケースフォールディングとの差が `ſ` (U+017F) と `s` を同一視しない点だけ (K (U+212A) は `k` に一致する)。`regex` を入れる v0.2 で仕様の意味に揃える (OQ-196 の判断と合わせる)。
- 全角/半角、ひらがな/カタカナの同一視は **行わない**。NFC/NFD 正規化差も同一視しない ([§13.5](#135-正規化と-bidi))。CP932 互換文字の同一視 (OQ-41) は v0.3。

### 12.5 置換 (FR-403、v0.2)

**v0.1 はリテラルの「すべて置換」だけを実装する** (確定 (ユーザー回答)。FR-403 の v0.1 の範囲)。`search.replace_open` (置換欄を開く) と `search.replace_all` を実装し、1 件置換 `search.replace`・正規表現の置換文字列の展開・選択範囲内 (`SearchQuery.scope`) は v0.2 で、CommandId と型を予約する。手順は下表で固定し、表の「1 件置換」「選択範囲内」と正規表現の行は v0.2 の規則である。

**v0.2 の追加** (M9a。下の v0.1 の手順に重ねる):

- **すべて置換の問い合わせ**: 検索バーの現在の `SearchQuery` をそのまま使う (大小区別・正規表現・単語単位。下の手順 2 の「`kind = Literal`・`whole_word = false`」は v0.1 の定数で、v0.2 は検索バーのトグルに従う)。選択範囲内 (`scope`) は未実装。
- **置換文字列**: リテラルでは従来どおり展開しない。**正規表現では** `$1` / `${name}` を一致ごとに展開し (`regex` crate の構文。`$$` は `$`)、`\n` は文書の既定の改行 (`default_eol`) に、`\\` は `\` 1 つに直す (ほかの `\` はそのまま)。存在しないグループは空文字列になる。
- **1 件置換 (`search.replace`、置換欄の `Enter`)**: when は `search.replace_active && !doc.read_only`。対象は、**選択範囲がちょうど 1 つの一致ならその一致** (`search.next` で確定した一致)、そうでなければ **入力で強調している現在の一致**。その範囲を置換文字列 (正規表現では展開後) に置き換える 1 つの Transaction (`Origin::Command("search.replace")`、`UndoGroupHint::Break` で **置換ごとに別の Undo 単位**) として適用し、カーソルを置換の後ろに置いて、そこから先の最初の一致を強調する (終端で折り返さない。起点は置換の後ろ)。対象が無ければ `info.search.replace_no_match`、検索語が空なら `info.search.replace_empty_query`。検索欄の `Enter` (`search.next`) は変わらない。

**v0.1 のすべて置換の手順**:

1. **置換欄**: `search.replace_open` (when `!doc.read_only`) は検索バーを置換欄つきで開く (開いていれば置換欄を足す)。フォーカスは検索語が空なら検索欄、空でなければ置換欄に置く。置換欄は検索欄と同じ 1 行の入力欄 (`SearchBarModel.replace`、02 §8.2。改行は入力できず、貼り付けは最初の行だけ、04 §6.6)。検索語の入力・大小区別の切替・件数の表示は置換欄があっても検索と同じ (§12.1 / §12.2)。読み取り専用の文書では置換欄を開かず `msg.edit.read_only` を出す。開いた後に文書が読み取り専用になったときは、`search.replace_all` の when が偽になり実行できない (パレットでは淡色と理由)。
2. **実行**: `search.replace_all` (when `search.replace_active && !doc.read_only`。キーは `Alt+A`、または `Tab` で `[A] すべて置換` のボタンへ移って `Enter`、04 §8.4) は、そのときの検索欄の検索語・大小区別 (`SearchBarModel.case`) で作った `SearchQuery` (`kind = Literal`・`whole_word = false`・`scope = Document`) と置換欄の文字列を使う。検索語が空なら何もせず `info.search.replace_empty_query` を出す。置換文字列は展開せずにそのまま挿入する (`$1` や `\n` も文字どおり。置換文字列が空なら一致を削除する)。改行の正規化はしない (置換欄に改行は入らない)。確認ダイアログは出さない (1 回の Undo で戻せるため)。
3. **一致の収集と ChangeSet**: 一致の選び方は検索と同じ (§12.1 の「一致の選び方」: 重ならないものを左から順に、両端が書記素境界のものだけ)。文書が 1 MiB (`SEARCH_SYNC_STEP_BYTES`、§12.2) 以下ならメインスレッドで、超えればジョブ (`JobKind::ReplaceAll`、02 §9.3) で Snapshot (Revision R) を走査し、一致を保持せずに `ChangeSet::from_sorted_edits` で 1 つの ChangeSet を直接作る (`search.max_matches` / `search.count_limit` の上限の影響を受けない)。ジョブの間も入力は止めず (NFR-103)、検索バーを閉じる (`Esc`) と取り消す。
4. **適用**: 適用の時点で Revision が R でなければ (準備中に編集した)、今の Snapshot で 1 回だけ作り直し、それも古ければ適用せずに `msg.search.replace_stale` を出す。一致が 0 件なら文書を変えずに `info.search.replace_no_match` を出す。1 件以上なら **1 つの Transaction** (`Origin::Command("search.replace_all")`、`UndoGroupHint::Break`) として適用し (1 Undo 単位、FR-403)、`info.search.replaced` で件数を示す。文書は dirty になる。Undo の予算 (§7.5) の「直前の 1 操作は保持」が大きな置換にも働く。
**v0.1 の実装の範囲**: 手順 4 の「適用の時点で Revision が R でなければ今の Snapshot で 1 回だけ作り直す」は未実装で、古ければ作り直さず `msg.search.replace_stale` を出して中止する (ジョブの結果が届く前に文書が変わった場合。ジョブにするのは 1 MiB 超の文書だけで、それ以下はメインスレッドで作るので古くならない)。

5. **適用の後**: 主カーソルは元の位置の写像に置き (置換箇所の全選択は `search.replace_all_select_result`、v0.2 以降、07 §5.13)、検索バーと置換欄は開いたまま、保持していた一致は編集で無効になり (§12.2 の「編集後の結果」)、入力が止まった後の再検索で件数が更新される (置換文字列が検索語を含まなければ 0 件になる)。

| 操作 | 手順 |
|---|---|
| 1 件置換 | 現在の一致を `Edit { range, insert }` の Transaction (Origin::Command("search.replace")) で置換し、次の一致へ移動。各置換は個別の Undo 単位 |
| すべて置換 | ジョブで Snapshot (Revision R) 上の全一致を収集 → `ChangeSet::from_sorted_edits` で 1 つの ChangeSet → メインスレッドで `Document::apply`。**適用時に Revision が R でなければ `StaleBase`** となるので、再ジョブ (または小さい文書なら同期で再計算)。1 Undo 単位 |
| 置換文字列 | 正規表現モードでは `$1` / `${name}` の展開 (regex crate の構文)、`\n` は `default_eol` に展開。リテラルモードでは展開なし |
| 選択範囲内 | `scope: Selection` で各 Range 内の一致のみ対象。複数 Range にも対応。**M9b の実装 (確定 (ユーザー回答)、2026-10-30)**: 検索バーのトグル `search.toggle_selection` (`Alt+L` / `Ctrl+K L`)。範囲は**検索バーを開いたときの選択** (空でない Range) で、編集に合わせて写像する (範囲の端での挿入は範囲に含める)。ON の間は検索・件数・次へ / 前へ・可視範囲の強調・1 件置換・すべて置換のすべてを範囲内に限る (`SearchQuery::with_scope` → `Matcher` の走査の入口で、範囲に収まらない一致を捨てる)。一致は文書全体と同じ規則 (左から重ならずに) で選び、範囲の境界をまたぐ一致は採らない。開いたときに選択が無ければ切り替えずに `info.search.no_selection` を出す |
| 結果 Selection | すべて置換後は primary カーソルを元位置の写像に置く (置換箇所を全選択する Helix 風は設定で選択可) |
| 件数上限超 | `max_matches` を超える「すべて置換」はジョブ内で **ChangeSet を直接構築** する (一致を保持しない) ため上限の影響を受けない。ただし ChangeSet 自体のサイズが大きくなるので、Undo 予算 (§7.5) の例外規則 (直前 1 操作は保持) が働く |

### 12.6 クリップボードへのコピーと大きな文字列化

- 大きな選択範囲 (100 MB の全選択など) をコピーするとき、メインスレッドで `String` を作ってはならない (数十 ms と 100 MB のメモリ)。大きい範囲 (64 KiB 以上) では、core は Snapshot と範囲 (`ClipboardSource`) を Editor のハンドル表に登録し、`Effect::SetClipboard` は **`ClipboardText::Handle(ClipboardHandleId)`** を運ぶ。Host は `Editor::take_clipboard(id)` で `ClipboardSource` を取り出し、**文字列化を Host 側のワーカーで行う** (02 §5.4 / §6.3。取り出した後のハンドルは無効)。小さい範囲 (64 KiB 未満) は core が文字列化して `ClipboardText::Inline(String)` で運ぶ。`ClipboardText` は eve-view の serde 可能な型で、eve-text の型 (Snapshot) を含まない。
- 内部レジスタは、64 KiB 未満なら `Arc<str>` にコピーし、64 KiB 以上なら一旦 Snapshot 参照で持ってから §6.5 と同じ **実体化ジョブ** で範囲だけの独立の rope に置き換える (結果が届いたときにレジスタの内容が同じ `SliceKey` のままなら置き換え、別の内容に替わっていれば捨てる) (Snapshot 参照のままだと、その後の編集で文書全体が会計外で残り得る。§6.5)。レジスタは最新の 1 件だけを持つので、その大きさがそのまま追加のメモリになる (Undo の会計には含めない)。内部レジスタからの貼り付けも §8.4 の改行の正規化を通す。システムクリップボードへの受け渡しには上限 (`clipboard.system_max_bytes`、既定 64 MiB、07 §5.6) を設け、**文字列化の前に** 選択範囲の総バイト数 (全 Range の合計) で判定する。超えたら `Effect::SetClipboard` を出さず (文字列化もしない)、内部レジスタだけに入れて通知する (01 FR-205 AC-4 (a))。上限未満の大きな範囲は上のハンドルの経路で、文字列化とシステムクリップボードへの書込の間も入力を止めない (同 (b))。OSC 52 の上限 (100,000 バイト、05 §8.4) は別。
- **選択が空のときの `clipboard.copy` / `clipboard.cut`** は何もしない (行全体のコピー・切り取りは行わない。通知も出さない。文書・クリップボード・内部レジスタは変わらない)。
- **システムクリップボードからの貼り付け** (`edit.paste`): core は `Effect::RequestClipboard { token }` で読取を要求し、応答 `InputEvent::ClipboardContent { token, text }` が届いた時点の発行 View の **到着時の Selection** に貼り付ける (要求時の Selection を保存しない。待つ間に動かしたカーソルの位置に入る)。token が古くなって応答を捨てるのは、**次の `RequestClipboard` を出したときだけ** で、待つ間の編集・移動・検索では古くならない。応答が届いたときに文書が読み取り専用になっている、または View が閉じている場合は貼り付けずに捨てる。

### 12.7 ワークスペース検索 (v0.4, FR-405) への接続

- ワークスペース横断検索は `ignore` + `grep-searcher` ([02 §13.5](02-architecture.md#135-検索)) をジョブで走らせ、結果を `(path, line, ByteRange)` で返す。開いている Document についてはディスクではなく Snapshot を検索する (未保存の内容を反映)。
- ジャンプ時に Document を開き (§9.2)、`LineByte → Pos` で位置を決める。本章の検索基盤 (`SearchQuery`、正規表現フラグ) を共有する。

---

## 13. Unicode 処理

### 13.1 書記素クラスタ (FR-201 / 301)

- 単位は **拡張書記素クラスタ** (UAX #29、`unicode-segmentation`)。カーソル移動・Backspace/Delete・選択の最小単位。
- **チャンク跨ぎ**: 境界判定の入力は `TextSlice` から前後最大 256 バイトを連結した小バッファとし、その中央付近の境界のみを信用する。`unicode-segmentation` の `GraphemeCursor` (チャンク境界で `PreContext` を要求できる API) を使えば、より正確に扱える。実装はどちらかで、挙動は同じ。
- **Backspace の特則**: 既定は書記素単位 (FR-201)。ただし一部の利用者はハングルや結合文字で「1 コードポイントずつ」の削除を期待する。v0.2 で設定 `editor.backspace_unit = "grapheme" | "codepoint"` (07 §5.13) として公開し、v0.1 は `grapheme` 固定 (FR-201、OQ-45)。
- **絵文字**: ZWJ シーケンス・肌色修飾・国旗 (地域指示記号ペア)・キーキャップはすべて 1 書記素。幅は §13.2。
- **異体字セレクタ (VS15/VS16)**: 基底文字と結合して 1 書記素。VS16 は幅を 2 にする (§13.2)。
- **制御文字**: 単独で 1 書記素。表示は可視記号 (§14.2)。

### 13.2 表示幅 (FR-501 / NFR-605)

```rust
pub struct WidthConfig { tab_width: u8, ambiguous_wide: bool, control_style: ControlStyle /* Caret(^X) | Hex(<U+001B>)。C0 と DEL にだけ効く */, emoji_width: EmojiWidth, show_invisible: bool /* 不可視文字の可視化。既定 true */ }
pub enum EmojiWidth { Auto, One, Two }   // Auto = 04 の端末補正 (幅プローブ・端末種別の表) が決めた値。決まらなければ Two
pub fn grapheme_width(g: &str, cfg: &WidthConfig) -> u8;   // 0, 1, 2 または可視記号の長さ (下表。最大 48)。タブは呼び出し側が列から計算
pub fn is_width_risky(g: &str) -> bool;                      // 端末との幅不一致が起きやすい書記素か (下表)。RenderRow.width_risky の素材
```

設定キーの正本は 07 §5.3 (`view.ambiguous_width = 1 | 2` → `ambiguous_wide`、`view.emoji_width = "auto" | 1 | 2` → `emoji_width` (`EmojiWidth::Auto` / `One` / `Two`)、`view.control_char_style`)。`view.emoji_width` と `view.control_char_style` は v0.1 では既定値の定数で、キーの公開は v0.2 (07 §5.13)。

**幅危険書記素の分類は `eve-text` (本節) の責務** とする (04 §4.7 (b) は端末実測の補正を持つ)。幅関数と同じ場所に置く理由: 分類は Unicode プロパティで決まり端末に依存しないため。04 は端末実測に基づく **補正表** (端末種別ごとに「この文字を 1 セルで描く」等) と、危険行の **再描画戦略** を持つ。`is_width_risky` が真となる分類: ZWJ シーケンス、異体字セレクタ (VS15/VS16) 付き、地域指示記号 (国旗)、肌色修飾、半角カナの濁点・半濁点 (U+FF9E / U+FF9F)、Emoji_Presentation、East Asian Ambiguous、私用領域 (PUA、Nerd Font アイコン)、`unicode-width` のバージョンより新しい可能性のある未割当・新規コードポイント。core は `RenderRow.width_risky = 行内に 1 つ以上含む` を Frame に載せる (02 §8.2)。

| 分類 | 幅 | 根拠 / 補足 |
|---|---|---|
| 東アジアの W / F | 2 | UAX #11 (`unicode-width`) |
| 東アジアの Ambiguous (A) | 設定 (`ambiguous_wide`、既定 false = 1) | NFR-605。端末の挙動と合わせる。04 の幅プローブ (明示コマンド) で既定値を提案 |
| 結合文字 (Mn/Me/Mc の一部)、絵文字 ZWJ 列の中の ZWJ、異体字の列の中の VS | 0 (基底に吸収) | 書記素単位で幅を求めるため個別には扱わない。可視化の対象 (下の「不可視文字」) に当たるものは除く |
| 半角カナの濁点・半濁点 (U+FF9E / U+FF9F) | **各 1** (基底の幅に足す。例 `ｶﾞ` = 2、`ﾊﾟ` = 2) | UAX #29 では Extend として基底に結合し `unicode-width` は 0 で数えるが、**Windows Terminal と conhost のどちらも、これらを基底とは別に 1 セル進めて描く** (S-06 の実測、08 §3)。幅 0 のままだと行の後ろの列がずれる。`is_width_risky` は真とする |
| 絵文字表示 (Emoji_Presentation) | 2 | 端末の多くが 2 セル。`emoji_width = One` で 1 に、`Auto` は 04 の端末補正に従う |
| テキスト表示の絵文字 + VS16 | 2 | — |
| 地域指示記号ペア (国旗) | 2 | 単独の指示記号も 2 とする (端末差大。04 の補正表) |
| ハングル音節 / 字母結合列 | 2 | — |
| 不可視文字 (下の「可視化の対象」) | **可視化して幅 > 0** (FR-507。`<ZWSP>` 等の記号の長さ) または 0 (`show_invisible = false`。設定キーは 07、v0.1 は既定 true の定数) | 不可視文字によるセキュリティ (bidi 制御による表示の偽装など)・編集上の混乱を避ける。既定は可視化 |
| U+2028 LS / U+2029 PS | `<LS>` / `<PS>` = 4 (可視化の対象) | 行区切りにしない (§2.3)。行は分けずに 1 行の中に記号で示す |
| C0 制御文字 (TAB, LF, CR 除く) と DEL (U+007F) | `^X` = 2 (`Caret`) または `<U+001B>` = 8 (`Hex`) | FR-507。VT / FF も行区切りにせず `^K` / `^L` |
| C1 制御文字 (U+0080..U+009F。NEL U+0085 を含む) | 常に `<U+0085>` = 8 (`control_style` によらず 16 進。キャレット表記にしない) | キャレット表記は C1 では慣習が割れ (`M-^E` 等)、不正バイトの表記とも紛れるため |
| 不正バイトのエスケープ (§8.3。`pua_escape = Enabled` の文書の `U+10FF00..FF`) | `<0x9A>` = 6 | **バイトは `<0xHH>`、コードポイントは `<U+XXXX>`** と書き分ける (不正バイト 0x85 は `<0x85>`、文字 U+0085 は `<U+0085>`)。`DisabledCollision` の文書 (§8.3) が真に含む `U+10FF00..FF` は通常の私用文字として扱い、エスケープとして表示しない |
| タブ | 次のタブストップまで (`tab_width`、既定 4) | 列に依存するため表 外 |
| 書記素の先頭が結合文字 (Mn / Me) の書記素 (孤立結合文字) | 1 (可視記号 ◌ + 結合) | カーソルを置けるようにする。**Mn / Me に限る** (VS は Mn だが下の「単独の VS」として可視化する) |
| 上のどれにも当たらず幅 0 になる書記素 | `<U+XXXX>` (先頭のコードポイント) の長さ | 幅 0 のセルを作らない (カーソルを置けるようにする) |

**不可視文字の可視化の対象** (FR-507。判定は **書記素の中の各コードポイントを走査** して行う。書記素の先頭だけを見ない):

| 対象 (コードポイント集合) | 表記 | 例外 (可視化しない) |
|---|---|---|
| General Category **Cf** (書式文字) の全体。ZWSP U+200B、WJ U+2060、BOM U+FEFF (文書の途中のもの。先頭の BOM は読込で取り除かれる、§8.1)、SHY U+00AD、LRM / RLM / ALM、bidi 制御 U+202A–202E (LRE / RLE / PDF / LRO / RLO) と U+2066–2069 (LRI / RLI / FSI / PDI)、タグ文字 U+E0001 と U+E0020..U+E007F、U+FFF9..FFFB 等を含む | 略号のあるもの (`<ZWSP>` `<WJ>` `<BOM>` `<SHY>` `<LRM>` `<RLM>` `<ALM>` `<LRE>` `<RLE>` `<PDF>` `<LRO>` `<RLO>` `<LRI>` `<RLI>` `<FSI>` `<PDI>` `<ZWNJ>` `<ZWJ>`) はその略号、それ以外は `<U+XXXX>` (4〜6 桁の大文字 16 進) | **ZWJ (U+200D)**: 絵文字 ZWJ 列の中 (前後がともに Extended_Pictographic、またはその VS / 肌色修飾付き) のものは可視化しない。**タグ文字**: 絵文字タグ列 (U+1F3F4 の後に続き U+E007F で終わる、地域の旗) の中のものは可視化しない |
| ZWNJ U+200C (Cf) | `<ZWNJ>` | 無し (ペルシャ語などの正当な用途でも示す。表示を切る設定は `show_invisible`) |
| General Category **Zl / Zp** (U+2028 / U+2029) | `<LS>` / `<PS>` | 無し |
| **単独の VS** (U+FE00..FE0F、U+E0100..E01EF のうち、書記素の先頭にあるもの、または直前のコードポイントも VS であるもの) | `<VS1>`..`<VS256>` | 基底文字の直後の 1 つ (異体字の列・絵文字表示の指定) は可視化しない |

- 1 つの書記素に可視化の対象が複数あれば、基底の文字 (可視化の対象を除いた残り) の後に記号を **コードポイントの順に** 並べる (例: `a` + ZWNJ (U+200C は UAX #29 で前の文字に付く) は 1 書記素で `a<ZWNJ>`。ZWSP や bidi 制御は単独で 1 書記素になり、記号だけを描く)。並べる記号は 1 書記素あたり 4 個までとし、残りは `<+N>` にまとめる (幅の上限 48 セル。結合列を伸ばして Frame を壊す入力を防ぐ)。カーソルの単位は書記素のままで、記号の内部には置けない。
- 判定は `eve-text` の 1 つの関数 (`invisible_marks(g: &str) -> SmallVec<[Mark; 2]>`) にまとめ、レイアウト (§14.2)・`sanitize_for_display` (§14.6)・幅計算が共有する。Default_Ignorable_Code_Point のうち上の対象に入らないもの (CGJ U+034F、ハングルのフィラー U+115F / U+1160 / U+3164 / U+FFA0 等) は v0.1 では可視化しない (提案、OQ-197)。

**空白の可視化** (FR-508 の v0.1 の範囲。確定 (ユーザー回答)): 全角スペース (U+3000) とタブ文字を、上の不可視文字と同じ Frame 生成の経路 (§14.2 の `layout_line`) で記号に置き換えて示す。不可視文字との違いは **幅を変えない** こと (記号は元の書記素のセルにそのまま収める) で、`grapheme_width`・タブの幅 (§14.2)・`pos_to_display` 等の座標変換・カーソルと IME の位置は可視化の有無で変わらない。

| 対象 | 記号 (セル) | 幅 |
|---|---|---|
| 全角スペース U+3000 (単独の書記素) | `[]` (2 セル) | 2 (W) のまま |
| タブ | 先頭のセルに `>`、残りのセルは空白 | 次のタブストップまで (`tab_width`) のまま |

- 記号は ASCII だけにする (曖昧幅の設定 (`view.ambiguous_width`) や ASCII モード (04 §3.7) で幅・表示が変わらないように)。スタイルは `DecorationKind::Whitespace` (優先度 150、§14.5) として記号のセルにだけ付け、ロール `ui.whitespace` (前景だけ。背景は選択・検索一致・現在行のものをそのまま使う、07 §8.2) で描く。セルの種類は全角スペースが `CellKind::Text`、タブが `CellKind::Tab` のままで、可視化は Span の文字列とスタイルだけを変える。
- 対象は編集面の本文だけで、入力欄 (検索バー・プロンプト)・メッセージ・ステータスには適用しない。文書の内容・検索・コピー (システムクリップボードと内部レジスタ)・保存には影響しない (コピーした文字列は U+3000 とタブのまま)。
- 半角の空白 (U+0020)・行末の空白・改行・他の空白類 (U+00A0 等の Zs) は v0.1 では可視化しない (v0.4、FR-508)。全角スペースが不可視文字 (Cf 等) と同じ書記素に入ることは無い (U+3000 は単独で 1 書記素)。
- v0.1 は `view.show_whitespace` を定数 `"minimal"` (全角スペースとタブ) として実装し、切り替えない (07 §5.3。キーの公開は v0.2 で、`"off"` を選べる)。**v0.2 の M9c で公開した**: `off` ではタブを空白のセル (幅は次のタブストップまで)、全角スペースを ASCII の空白 2 セルで描き、記号のスタイル (`ui.whitespace`) を付けない。幅・カーソルの列・折り返しの位置は `minimal` と同じ。設定の差し替え (`set_config`) で開いている文書にも写す。

- **Unicode バージョン**: `unicode-width` / `unicode-segmentation` のバージョンで決まる。端末との不一致 (新しい絵文字を端末が 1 セルで描く等) による描画崩れは **04 の再描画戦略** で扱い、core は一貫した幅を使う ([02 §13.3](02-architecture.md#133-unicode))。

### 13.3 単語境界 (日本語補助規則)

**前提**: UAX #29 (`unicode-segmentation::split_word_bound_indices`) では、ひらがな・漢字には WB13 (カタカナ同士の連結) のような連結規則が **無く**、WB999 により **1 文字ごとに境界** になる。連結されるのはカタカナ (WB13) と英数のみ。そこで単語境界は次の文字種の遷移規則だけで決める (OQ-117)。

**文字種 (ScriptClass)**

| ScriptClass | 判定 (Unicode Script / General Category による) |
|---|---|
| `Han` | 漢字 (Script=Han)。`〆` (U+3006)、`ヶ`/`ヵ` (U+30F6/U+30F5。Script=Katakana だが慣用上漢字に付く) を含む。踊り字 `々` (U+3005) は 01 FR-302 のとおり **直前の文字種に従い**、ランの先頭に来たときだけ `Han` |
| `Hiragana` | ひらがな (Script=Hiragana)。長音 `ー` (U+30FC) と `ゝゞ` は **直前の文字種に従う** (Hiragana のランの中では Hiragana) |
| `Katakana` | カタカナ・半角カタカナ (Script=Katakana、U+FF66..U+FF9F)。`ー`・`ヽヾ` は直前が Katakana なら Katakana。ランの先頭に来た `ー` は Katakana |
| `Alnum` | Latin / 数字 / `_` / 結合文字を含む非 CJK の文字 (`\w` 相当。Script=Common の数字を含む)。全角英数は含まない (次行)。設定可能な追加区切り文字 (`editor.word_separators`、FR-302。キーの公開は v0.2、v0.1 は既定値の定数) に含まれる文字は `Punct` に移る |
| `FullwidthAlnum` | 全角英数: 全角数字 U+FF10..U+FF19、全角ラテン大文字 U+FF21..U+FF3A、全角ラテン小文字 U+FF41..U+FF5A。01 FR-302 (OQ-117) の列挙どおり `Alnum` とは別の分類にする (`ＡＢＣabc` は `ＡＢＣ\|abc`)。全角の記号 (`！` `＿` 等) は `Punct` |
| `Punct` | 記号・句読点 (General Category P* / S*。全角句読点 `、。「」` を含む)。**1 書記素ごと** (絵文字の ZWJ 列・国旗・キーキャップも 1 書記素で 1 単位) |
| `Space` | 空白 (White_Space。U+3000、U+2028 / U+2029 を含む) |

- **分類の単位は書記素** (§13.1) で、**書記素の先頭のスカラ値** の分類をその書記素の分類とする (結合文字・VS・ZWJ・肌色修飾など書記素の 2 つ目以降のスカラ値は分類に影響しない)。表の「文字」はすべて「書記素」と読む。
- 上の表に当たらないものの落ち先: **Cf** (書式文字。ZWSP・bidi 制御・単独の ZWJ / ZWNJ 等)、**Cc** (TAB 以外の制御文字。TAB は White_Space なので `Space`)、**Co** (私用領域。不正バイトのエスケープ `U+10FF00..FF` を含む)、**Cn** (未割当) は `Punct` (1 書記素で 1 単位。可視化される文字 (§13.2) の前後で単語移動が止まる)。**ハングル** (Script=Hangul)・**タイ文字**・キリル・ギリシャ・アラビア等の CJK 以外の文字体系の文字 (General Category L* / N* / 書記素の先頭の Mn / Mc) は `Alnum` (空白で区切られる言語は空白で止まる。タイ語のように空白で区切らない言語は句読点か空白まで 1 語になる。辞書による分割は icu4x のオプション (下記) の範囲)。
- 例外の踊り字・長音 (`々` `ー` 等) が「直前の文字種に従う」の「直前」は、直前の書記素の分類を指す。

**境界決定の規則 (全文字種に共通の「文字種の遷移」規則)**

1. 文字列を `ScriptClass` のランに分ける。**`ScriptClass` が変わる位置をすべて境界とし、ランの内側は境界にしない**。日本語も英語も同じ規則で決まる (VS Code の単語区切りや Vim の `w` に近く、予測しやすい)。
2. `Han → Hiragana` の遷移も **境界にする** (01 FR-302 / OQ-117。「食べる」は `食|べる`、「勉強する」は `勉強|する`)。送り仮名・助詞を漢字に続けて 1 語にする動作は v0.4 の設定 `editor.word_attach_hiragana` (既定 `false`、07 §5.13) で選べるようにする。
3. 例外: 長音符 `ー`・踊り字 `々 ゝ ゞ ヽ ヾ`・濁点 (半角 U+FF9E/FF9F、結合文字) は直前の文字の分類に続ける (表の定義どおり)。
4. `Punct` は 1 書記素ごとに境界 (`editor.word_separators` の文字は `Punct` に移る)。`Space` のランは 1 つの単位 (単語移動では読み飛ばす)。
5. **UAX #29 の単語境界 (`unicode-segmentation` の `split_word_bound_indices`) は使わない**。UAX #29 はひらがな・漢字を 1 文字ごとに区切り (WB999)、非 CJK 区間でも `3.14` や `don't` の扱いが用途 (コード編集) に合わないことがあるため。書記素境界 (§13.1) には UAX #29 を使い続ける (混同しない)。

**単語移動の停止位置 (FR-302、§5.3)**: 上の規則で分けた単位 (ラン。`Punct` は 1 文字で 1 単位、`Space` のランは 1 単位) を使い、1 行の中で次のように止まる。

- `cursor.move_word_right` (Ctrl+→): 現在位置 p より後ろで始まる **`Space` 以外の単位の先頭** のうち最も近い位置に止まる。行の中に無ければ行末 (区切りの直前) に止まり、行末にいれば次の行の行頭へ進む。`Space` のランの先頭には止まらない (空白は読み飛ばす)。
- `cursor.move_word_left` (Ctrl+←): 現在位置 p より前で始まる **`Space` 以外の単位の先頭** のうち最も近い位置に止まる。行の中に無ければ行頭に止まり、行頭にいれば前の行の行末へ進む。
- `Punct` は連続していても 1 文字ずつ止まる (`()` は `(` と `)` の 2 か所)。`Punct` のランをまとめて 1 語にはしない。

**期待結果の表 (FR-302 の受入条件。`|` が境界。09 §4 のコーパスとして 1 箇所に置き、01 の AC から参照する)**。「停止位置」は行頭 (0) から Ctrl+→ を繰り返したときと、行末から Ctrl+← を繰り返したときに止まる位置を、行頭からの **書記素数** で示す (各行を 1 行だけの文書として扱い、最後の値が行末・行頭)。

| # | 入力 | 境界 | 停止位置 (→ / ←) | 補足 |
|---|---|---|---|---|
| 1 | 日本語を勉強する | `日本語\|を\|勉強\|する` | → 3, 4, 6, 8 / ← 6, 4, 3, 0 | 漢字列・ひらがな列ごとに停止 (Han→Hiragana も Hiragana→Han も境界) |
| 2 | 食べる | `食\|べる` | → 1, 3 / ← 1, 0 | Han→Hiragana は境界。1 語にするのは `word_attach_hiragana` (v0.4) |
| 3 | 東京の天気 | `東京\|の\|天気` | → 2, 3, 5 / ← 3, 2, 0 | — |
| 4 | コーヒーを飲む | `コーヒー\|を\|飲\|む` | → 4, 5, 6, 7 / ← 6, 5, 4, 0 | 長音はカタカナに従う。`飲\|む` は Han→Hiragana の境界 |
| 5 | ＡＢＣ１２３とabc | `ＡＢＣ１２３\|と\|abc` | → 6, 7, 10 / ← 7, 6, 0 | 全角英数 (`FullwidthAlnum`) は 1 ラン。全角英数→ひらがな、ひらがな→半角英数は境界 |
| 6 | ＡＢＣ１２３と abc | `ＡＢＣ１２３\|と\| \|abc` | → 6, 8, 11 / ← 8, 6, 0 | 空白のランの先頭 (7) には止まらない |
| 7 | ＡＢＣabc | `ＡＢＣ\|abc` | → 3, 6 / ← 3, 0 | `FullwidthAlnum` → `Alnum` は境界 |
| 8 | 佃々木ヶ原 | 佃々木ヶ原 | → 5 / ← 0 | 々・ヶ は漢字扱い |
| 9 | データベース設計 | `データベース\|設計` | → 6, 8 / ← 6, 0 | Katakana→Han は境界 |
| 10 | ｱｲｳｴｵ | ｱｲｳｴｵ (1 語) | → 5 / ← 0 | 半角カナは Katakana |
| 11 | 「こんにちは」 | `「\|こんにちは\|」` | → 1, 6, 7 / ← 6, 1, 0 | 全角括弧は Punct |
| 12 | 行きます。次に | `行\|きます\|。\|次\|に` | → 1, 4, 5, 6, 7 / ← 6, 5, 4, 1, 0 | 句点は単独。`行\|きます`・`次\|に` は Han→Hiragana の境界 |
| 13 | hello world | `hello\| \|world` | → 6, 11 / ← 6, 0 | Alnum → Space → Alnum。空白は読み飛ばす |
| 14 | foo_bar baz | `foo_bar\| \|baz` | → 8, 11 / ← 8, 0 | `_` は Alnum |
| 15 | foo-bar | `foo\|-\|bar` | → 3, 4, 7 / ← 4, 3, 0 | `-` は Punct (既定の word_separators にも含まれる) |
| 16 | foo_bar.baz() | `foo_bar\|.\|baz\|(\|)` | → 7, 8, 11, 12, 13 / ← 12, 11, 8, 7, 0 | 連続する Punct (`()`) も 1 文字ずつ止まる |
| 17 | 123,456 | `123\|,\|456` | → 3, 4, 7 / ← 4, 3, 0 | `,` は Punct (UAX #29 の MidNum 規則は採らない。コード編集では分かれた方が扱いやすい) |
| 18 | version2.0 | `version2\|.\|0` | → 8, 9, 10 / ← 9, 8, 0 | `.` は Punct |
| 19 | don't | `don\|'\|t` | → 3, 4, 5 / ← 4, 3, 0 | `'` は Punct (UAX #29 の MidLetter 規則は採らない) |
| 20 | 日本語English | `日本語\|English` | → 3, 10 / ← 3, 0 | — |
| 21 | 第3回 | `第\|3\|回` | → 1, 2, 3 / ← 2, 1, 0 | 半角数字は `Alnum`。全角 `第３回` なら `第\|３\|回` (`FullwidthAlnum` も漢字と別ラン) |
| 22 | ひらがなカタカナ | `ひらがな\|カタカナ` | → 4, 8 / ← 4, 0 | — |
| 23 | ﾌﾞﾛｸﾞ記事 | `ﾌﾞﾛｸﾞ\|記事` | → 3, 5 / ← 3, 0 | 半角カナの濁点 (U+FF9E) は Katakana で、直前の文字と 1 書記素 (`ﾌﾞ` `ﾛ` `ｸﾞ` の 3 書記素) |
| 24 | お茶 | `お\|茶` | → 1, 2 / ← 1, 0 | Hiragana→Han は境界。「お茶」を 1 語にしたい場合は `word_attach_hiragana` では扱えない (辞書が必要。icu4x オプションの動機) |
| 25 | 😀😀abc | `😀\|😀\|abc` | → 1, 2, 5 / ← 2, 1, 0 | 絵文字は Punct (So) 相当で 1 文字ずつ |
| 26 | 　全角スペース　 | `　\|全角\|スペース\|　` | → 1, 3, 8 / ← 3, 1, 0 | U+3000 は Space (読み飛ばす)。`全角\|スペース` は Han→Katakana の境界 |
| 27 | 👨‍👩‍👧abc | `👨‍👩‍👧\|abc` | → 1, 4 / ← 1, 0 | ZWJ 列 (U+1F468 U+200D U+1F469 U+200D U+1F467) は 1 書記素で、先頭のスカラ値 (So) により `Punct` の 1 単位 |
| 28 | 한국어 텍스트 | `한국어\| \|텍스트` | → 4, 7 / ← 4, 0 | ハングルは `Alnum`。空白で止まる |
| 29 | 한국어abc | `한국어abc` (1 語) | → 6 / ← 0 | ハングルとラテン文字はどちらも `Alnum` で 1 ラン |
| 30 | foo⁠bar (`foo` U+2060 `bar`) | `foo\|<WJ>\|bar` | → 3, 4, 7 / ← 4, 3, 0 | Cf は `Punct` の 1 単位 (可視化して `<WJ>` と描く、§13.2) |

- 単語移動 (§5.3)、ダブルクリック選択 (FR-311)、`whole_word` 検索 (§12.4、v0.2)、テキストオブジェクト (§5.7、v0.4) で共用する。
- 実装は Unicode の Script / General Category 表引きだけで、`unicode-segmentation` の単語境界は使わない。表の全件一致は L0 のゴールデンテスト (09 §3.2 / §4) で検査する (P0 スパイクは不要)。
- 将来 `icu4x` の辞書分割 (「お茶」を 1 語にするなど) をオプション提供できるよう、`WordSegmenter` trait として抽象化する ([02 §13.3](02-architecture.md#133-unicode))。拡張点はこの trait だけで、v0.1 は遷移規則の 1 実装を持つ。

### 13.4 大文字・小文字変換 (FR-214)

- Rust 標準の `to_uppercase` / `to_lowercase` (完全 Unicode 対応、`ß → SS` のように長さが変わり得る)。トルコ語 `i` などロケール依存変換は行わない。

### 13.5 正規化と BiDi

| 項目 | 方針 |
|---|---|
| Unicode 正規化 (NFC/NFD/NFKC) | **行わない**。読み込んだコードポイント列をそのまま保持する (v0.1、NFR-303)。正規化コマンドを v0.4 で提供可 (Transaction として明示的に) |
| 正規化非依存検索 | v0.x では非対応。要望があれば Could (OQ-54) |
| BiDi (双方向テキスト、UAX #9) | **非目標** とする (00 §3 の NG11、確定 (ユーザー回答)、OQ-39)。表示は論理順のまま左から右に並べ、RTL 文字は端末任せ。端末自身が BiDi を再配置する場合、カーソル位置がずれ得るが、core は介入しない |
| 合字 (リガチャ) | TUI では端末任せ。core は幅 1 ずつとして扱う |
| 全角/半角の同一視 | 行わない (検索・比較ともに) |

---

## 14. 表示幅・折り返し・レイアウト

core は編集テキスト領域をセルグリッドにレイアウトして `Frame` に入れる ([ADR-0008](10-decisions.md#adr-0008-viewmodel-による描画分離))。本章はそのアルゴリズムを定める。描画 (色の縮退、端末出力) は 04。

### 14.1 入力と出力

```rust
pub struct LayoutInput<'a> { text: &'a Snapshot, view: &'a View /*scroll, wrap, viewport*/, width_cfg: &'a WidthConfig, decorations: DecorationSet, gutter: GutterSpec }
pub struct LayoutRow { line: LineIdx, wrap_idx: u32, byte_range: ByteRange, cells: Vec<Cell> /* or spans */, cursors: SmallVec<[CellCol; 1]> }
pub struct Cell { pos: Pos /*この書記素の開始*/, width: u8, kind: CellKind /*Text|Tab|Control|Escaped|Wide2nd|VirtualText*/ }
```

- **出力は可視行のみ** (`viewport_rows` 本)。文書サイズに依存しない (O(可視行 × 可視列))。
- 各行は `RenderRow` ([02 §8.2](02-architecture.md#82-frame-の構造-擬似コード)) に変換され、スタイルは Decoration を優先度順に合成して解決する。

### 14.2 1 行のレイアウト

```
layout_line(line, from_dcol, max_cols):
  for grapheme g in line (from チェックポイント §14.4 で from_dcol に最も近い位置):
    w = match g:
      "\t"            → tab_width - (dcol % tab_width)   (可視化 (§13.2 の「空白の可視化」) では先頭のセルを ">"、残りを空白にする。幅は同じ)
      "\u{3000}"      → 2   (可視化では "[]"。幅は同じ)
      制御/不可視/エスケープ (§13.2 の表と「可視化の対象」。書記素内の各コードポイントを走査) → 可視記号 (例 "^[", "<U+0085>", "<0x9A>", "<ZWSP>") を文字列として展開し、その幅 (無害化 NFR-503: 制御文字は Frame に決して入らない)
      その他          → grapheme_width(g, cfg)
    if 折り返し ON and dcol + w > max_cols → 行を切って次の LayoutRow へ (§14.3)
    cells.push(Cell{pos, width: w, ...}); dcol += w
```

- **全角が右端で入らない場合** (折り返し OFF の水平スクロール時): 右端 1 セルを空白のパディングで埋める。折り返し ON ではその書記素から次の表示行へ送る。
- **全角が左端にかかる場合** (折り返し OFF の水平スクロール時): `left_col` が幅 2 以上の書記素 (全角・可視記号) の途中 (全角の右半分など) に当たるときは、その書記素を描かず、左端の欠けたセルを **空白 1 セルずつで埋める** (半分だけの文字を出さない)。行が左右で切れていることを示す `‹` / `›` (`clipped_left` / `clipped_right`、04 §10) を置く場合、そのセルは **幅 1 として先に確保** し、残りのセルで上の規則 (左端の空白埋め・右端のパディング) を適用する。`‹` のセルに全角の左半分がかかるときも同様に、その全角を描かずに空白で埋める。カーソルが空白で埋めた書記素にあるときは、その書記素が見える位置まで水平スクロールする (04)。
- **選択範囲の末尾が改行を含む** 場合、行末に 1 セル分の選択表示を出す (改行が選ばれていることを示す。04 の表現)。
- **仮想テキスト** (v1.L: インレイヒント、AI ゴーストテキスト) は `Cell.pos` を持たない (`VirtualText`) セルとして挿入し、クリック時は直前の実セルの Pos に写す。

### 14.3 折り返し (FR-506、ソフトラップは v0.2)

**v0.1 は折り返し OFF (水平スクロール) のみ**。日本語の文章は改行の少ない長い行になりがちでソフトラップの価値は高いが、レイアウト・上下移動・禁則処理・折り返し継続行の表示がまとめて入るため v0.2 とする (OQ-110 で確定 (ユーザー回答))。v0.1 では `View.wrap`、`RowSource` の折り返し番号、`View.scroll: ScrollPos` (§19。v0.1 は `top` と水平スクロール量 `left_col` を使い、`wrap_idx` は常に 0)、設定キー `view.wrap` の名前 (公開は v0.2、07 §5.3。v0.1 の公開キー 7 個には含めない) を予約し、`layout_line` は `max_cols` を無限大として `left_col` から可視列を切り出す。以下は v0.2 の規則。

| 項目 | 方針 |
|---|---|
| 単位 | 既定は **単語境界で折り返し** (UAX #14 の簡略版: 空白の後、句読点の後、CJK 文字はどこでも可)。単語が 1 行に収まらない場合は書記素単位で強制折り返し |
| 日本語 | CJK 文字間は任意で折り返し可。行頭禁則 (、。」など) と行末禁則 (「 など) の **最小集合** を適用 (設定で無効化可)。**禁則を満たす折り返し位置が表示行の中に無い** (禁則文字が幅いっぱいに続く等) ときは、禁則を破って書記素単位で強制分割する (表示行を幅より長くしない) |
| タブ | タブの幅は **論理行の先頭からの表示列** (折り返しが無いとしたときの列) でタブ停止位置を決める。表示行の先頭や折り返し後のインデント (`wrap_indent`) を基準にしない (折り返し位置が変わってもタブの幅が変わらない)。タブが表示行の残りに収まらなければ次の表示行へ送り、幅は送った先でも論理行の列で決める |
| 折り返し後の行頭 | インデント維持 (`view.wrap_indent`、07 §5.3。enum `none \| same` (既定 `same` = 論理行のインデント幅で揃える)。`plus_one` は後の版で値を追加、07 §5.3 / §5.13)。インデントが幅の 2/3 を超えるときは 0 に落とす |
| 幅 | `viewport_cols - gutter_width - 右余白 1`。全角 1 セル超過は次行へ |
| 可視行 → 論理行 | View は `scroll: ScrollPos { top, wrap_idx, left_col }` を保持 (折り返し ON では `left_col = 0`)。上下スクロールは表示行単位。折り返し数は **可視範囲の論理行についてのみ計算** し、スクロールバー比率は論理行数 × 推定折り返し数で近似 |
| 長大行 | §14.4 のチェックポイントが折り返し位置も記録する。折り返し ON では、チェックポイントは **表示行の先頭にだけ** 置く (4 KiB を超えた後の最初の表示行の先頭。そこから走査を再開すれば、単語境界・禁則の判定が前の表示行に依存しない) |
| 折り返し位置のカーソル (affinity) | 折り返し位置の `Pos` は「上の表示行の末尾」と「下の表示行の先頭」の 2 か所に描ける。v0.2 で `affinity: Affinity { Before, After }` を持たせる (Before = 上の表示行の末尾、After = 下の表示行の先頭)。**置き場所は `Range.head` ではなく `View` (主カーソルだけ、`goal_dcol` と同じ寿命。案、§14.3.1)**。End (§5.4) は Before、Home・上下移動で表示行の先頭に来たとき・文字入力の後は After とする。`Range` には持たせないので、Selection の比較・併合 (§4.4) は影響を受けない。**v0.2 の予約**で、v0.1 は持たせない (v0.1 は折り返し OFF のみ) |
| 設定変更・リサイズ | レイアウトキャッシュ全無効化。スクロール位置は `ScrollPos.top` (先頭可視行の行頭の `Pos`) で保持しているので再レイアウト後も同じ論理位置を先頭に置く |

#### 14.3.1 折り返しを使う側の突合と実装の分け方 (M9b、2026-10-30。案)

折り返し ON にすると「1 論理行 = 1 表示行」が崩れる。レイアウトの結果を使っている箇所を洗い出し、それぞれの扱いを先に決める。

**核となる関数**: `wrap_line(units, text_cols, mode, indent, kinsoku) -> Vec<WrapRow>` を 1 つ作り、下のすべての利用者がこれだけを根拠にする (純関数。`units` は既存の `units_for` の結果 = 書記素ごとの (論理行の表示列, 幅, バイト位置)。`WrapRow { first_unit, end_unit, byte_start, byte_end, indent_cols }`)。折り返し OFF は「全体で 1 行」の `WrapRow` を返すものとして同じ経路に載せ、利用者側の分岐を増やさない。

| 利用者 | 今 (折り返し OFF) | 折り返し ON での扱い |
|---|---|---|
| Frame の行の生成 (`build_frame` / `build_row`) | 可視行 = 論理行 `top_line + r`。行内を `left_col` から切り出す | `ScrollPos { top, wrap_idx }` から表示行を順に取り、`WrapRow` の範囲を `build_row` に渡す (`left = 0`)。継続行は `RowSource::Line { line, wrap_idx > 0 }`、ガターは幅だけ空白で、`↪` は TUI が `wrap_idx > 0` を見て描く (記号の縮退は TUI の責務、04 §10) |
| カーソルのセル位置 | `cursor_cell(line, byte)` = 論理行の列 | 該当の `WrapRow` 内の列 (`u.col − 行の先頭の col + indent_cols`)。折り返し位置は affinity で上下どちらの表示行かを決める (下) |
| 選択・検索の強調・未表現文字の印 | バイト範囲で突き合わせ (`Marks` / `sel`) | **変更なし** (バイト位置で判定するので、表示行ごとに範囲を切り出すだけで足りる)。行末の改行の選択 1 セルは論理行の最後の表示行にだけ出す |
| 垂直移動 (`Up` / `Down`) | 論理行を 1 つ動き、`goal_dcol` の列へ | **表示行**を 1 つ動く。`goal_dcol` は表示行内の列 (`indent_cols` を含む画面上の列)。先頭/最終の表示行での挙動 (文書の端へ) は今と同じ |
| `Home` / `End` | 論理行の頭 / 終わり | 03 §5.4 のとおり。`Home` はまず表示行の先頭、次にインデント終端のトグル。`End` はまず表示行末 (affinity = Before)、すでに表示行末なら論理行末 |
| ページ移動・ホイール | `viewport_rows − 1` 論理行 | **表示行** (`scroll_lines` を表示行単位にする)。ページ移動は画面上の相対位置を保つ |
| スクロールの追従 (`ensure_vertical`) | カーソルの論理行が余白を除く可視範囲に入るよう `top` を動かす | カーソルの**表示行**で同じ判定。カーソルが `top` から 1 画面分の論理行より遠いときは、間の折り返し数を数えずに直接 `top` を置く (指定行へ移動・検索の一致への移動でも文書の大きさに比例する計算をしない) |
| 水平スクロール (`ensure_horizontal`) | カーソルの列が見えるよう `left_col` を動かす | 何もしない。`left_col = 0` に保つ。折り返しを ON にした時点で 0 に戻す |
| 文書の最大スクロール (`max_top_line`) | `len_lines − rows` | 末尾の論理行の最後の表示行が画面の最下行に来る位置まで (末尾の数行だけ数える。文書全体の折り返しは数えない) |
| 編集による `top` の追従 | `top` を変更の写像で動かし行頭に丸める | 同じ。丸めた先の論理行が変わったら `wrap_idx = 0`、同じなら新しい折り返し数に収まるよう `wrap_idx` を丸める |
| リサイズ・設定変更 | — | `top` (論理行) は保ち `wrap_idx` を 0 に戻す (§14.3 の「ScrollPos.top を保持」と同じ) |
| ガターの幅 | 行数の桁数 + 1 | **変更なし**。本文の折り返し幅は `viewport_cols − gutter − 1` (右余白 1) |
| 状態バーの行・桁 | 論理行・書記素の桁 | 変更なし (論理位置) |
| マウスの位置変換 (M9b の 5 本目) | — | Frame の `RowSource` (論理行 + `wrap_idx`) と、同じ `wrap_line` から、(表示行, 列) → バイト位置を引く関数を足す。全角の右半分は §14.2 と同じ |

**affinity の置き場所 (§14.3 の表を改める案)**: 表では `Range.head` に持たせるとしたが、`Range` を変えると選択の比較・併合・保存の全部に波及する。折り返し位置の曖昧さを持つのは**主カーソルだけ**で十分なので、`View` に `cursor_affinity: Affinity { Before, After }` を持たせ、`goal_dcol` と同じ寿命で扱う (水平移動・編集・クリックで `After` に戻す。`End` と表示行末への移動で `Before`)。副カーソルは常に `After`。`Range` は変えない。

**折り返しの規則 (OQ-57 の最小集合)**:

- 設定 `view.wrap`: `off` (既定) / `soft` (幅いっぱいの書記素単位。禁則だけ適用) / `word` (単語境界を優先。禁則も適用)。`view.wrap_indent`: `none` / `same`。`view.wrap_kinsoku`: 既定 `true`。
- 折り返せる位置 (`word`): 空白の連なりの直後、CJK 文字 (幅 2 の書記素) の前後。そのどれも使えないとき (長い英単語・URL など) は書記素単位で強制的に折る。`soft` はすべての書記素の間。
- 空白は行末にぶら下げる: 行末の空白は 1 セルまで右余白に食い込んでよい (空白が次の行の行頭に来ない)。2 つ目以降の空白は通常どおり幅に数える。
- 禁則 (`kinsoku = true` のとき、`word` と `soft` の両方): **行頭に来てはならない** 書記素 = `、。，．・：；！？）］｝〕〉》」』】゛゜ー々ゝゞヽヾ` と小書きの仮名 (`ぁぃぅぇぉっゃゅょゎァィゥェォッャュョヮヵヶ`)、**行末に来てはならない** 書記素 = `（［｛〔〈《「『【` と ASCII の `([{`。折る位置の直前・直後がこれに当たるときは、その位置を使わず、1 つ前の折れる位置を探す。**そのような位置が表示行の中に無ければ禁則を破って**強制的に折る (表示行を幅より長くしない、§14.3)。
- タブは論理行の列で幅を決め (§14.3)、残りに収まらなければ次の表示行へ送る。1 つのタブが折り返し幅より広いときは単独の表示行にする。
- `wrap_indent = same`: 継続行を論理行の先頭の空白の表示幅だけ右にずらす。その幅が折り返し幅の 2/3 を超えるときは 0。
- 幅が 2 以上の書記素が表示行の残りに入らないときは次の表示行へ送る (右端に空白は足さない)。

**表示行単位の移動・スクロールの細部 (3 本目で決めた案)**:

- 上下移動の `goal_dcol` は、表示行内の画面上の列 (継続行のずらしを含む)。折り返しの ON/OFF を切り替えたら解除する。行の幅を超える目標列は、その表示行の末尾に寄せる (折り返し位置では `Before`、論理行の最後の表示行では論理行末)。
- 先頭の表示行で `Up`・最後の表示行で `Down` は、今までと同じく文書の先頭・末尾へ行く。ページ移動は画面の行数 − 1 の**表示行**を動き、文書の端に達したらそこで止まる。
- `Home`: 先頭の表示行では今までのスマートホーム。継続行では、まずその表示行の先頭へ行き (`After`)、すでにそこにいれば論理行のスマートホーム (インデント終端 → 行頭)。`End`: 最後の表示行でなく、かつ表示行末にいなければ表示行末 (`Before`)、そうでなければ論理行末。
- `cursor_affinity` は、`End` と上下移動で表示行末にとどまったときだけ `Before`。ほかの移動・編集・Selection の設定はすべて `After` に戻す。
- スクロールは (論理行, 行内の表示行) の組で持つ。カーソルが `top` から 1 画面分の論理行より遠いときは間の折り返しを数えずに直接置き、近いときは `top` からカーソルまでの表示行を数える (数える範囲は画面の高さまで)。
- **性能 (NFR-103、M9b の 4 本目の実測。リリースビルド、ヘッドレスで入力 → Frame 生成)**: 1 MB の日本語混在文書 (120×40、中央、`view.wrap = word`) で p50 0.6 ms / p99 1.5 ms (折り返し OFF は p50 0.3 ms / p99 0.9 ms)。目標 (p50 2 ms / p99 8 ms) の内側。測るには `cargo bench -p eve-testkit --bench bm_103a -- --wrap`。**長大行**は 1 回の操作で `wrap_line` を行の表示範囲 (64 KiB) の分だけ数回走らせるので、64 KiB の 1 行 (ASCII) で上下移動 + Frame が約 8 ms (日本語は約 0.8 ms、20 KB の行は約 0.3 ms)。この最悪ケースは §14.4 の列チェックポイント (v0.3) で、表示行の先頭を引けるようにして解消する (v0.2 は許容。折り返しの結果の再利用は v0.3 で合わせて検討)。
- 編集で `top` の論理行の行頭が動かなければ `wrap_idx` を新しい表示行数に収め、動いたら 0 に戻す。リサイズでも 0 に戻す。

**実装の分け方 (README §3.5 の 3・4 を細かくする)**:

1. この節 (突合の記録)。
2. `wrap_line` と、Frame の行・カーソル・選択・検索の強調への反映 (`View.wrap` は内部の状態として持ち、設定とコマンドはまだ公開しない。試験は内部 API で切り替える)。
3. 表示行単位の上下移動・`Home` / `End` と affinity・ページ移動・スクロール・`max_top`。
4. TUI の描画 (`↪`)、設定キー (`view.wrap` / `view.wrap_indent` / `view.wrap_kinsoku`) と `view.toggle_wrap`、長大行・巨大ファイルでの NFR-103 の確認。

### 14.4 長大行の列チェックポイント

**版: v0.3 (v0.1 は差し替え点のみ予約)**。チェックポイント構造は **v0.1 では型 (`LineLayoutCache` の差し替え点) だけを予約し、実装は v0.3** とする。v0.1 の長大行対策は §11.5 の読み取り専用での縮退 (閾値超の行は先頭 64 KiB だけを走査)、v0.2 は char 近似モードであり、`layout_line` は「行頭から対象位置まで走査する」素朴な実装でよい (閾値超の行は v0.1 では走査範囲が限られ、v0.2 では char 近似で走査が速い)。

```rust
struct LineLayoutCache {
    line: LineIdx, revision: Revision, width_cfg_hash: u64, wrap_cols: Option<u16>,
    checkpoints: Vec<Checkpoint>,   // 行頭 (byte 0) から 4 KiB ごと。短い行は 1 つだけ。v0.1 は常に 1 つ (行頭) のみ
}
struct Checkpoint { byte: ByteOff /*行頭基準*/, dcol: u32 /*折り返し OFF 時の累積表示列*/, grapheme: u32, wrap_row: u32, wrap_col: u16 /*折り返し ON では表示行の先頭にだけ置くので常に 0 (§14.3)*/ }
```

- 走査は常に「対象位置の直前のチェックポイント」から始め、最大 4 KiB + 最後の書記素で止まる → `pos_to_display` / `display_to_pos` / `pos_to_line_grapheme` が長大行でも **O(log n + 4 KiB)**。
- 編集で変更された位置以降のチェックポイントを捨て (`truncate`)、必要時に再計算。行末までの全チェックポイント再計算は **行末の表示列が要るとき** (End キー、水平スクロールバー) のみ発生する。所要時間は v0.1 の計測 (09 BM-106) で決める (10 MB 行で 50〜300 ms の可能性。高速経路の効果次第)。閾値を超える行は **ジョブで計算し、完了まで近似値を表示** する。
- キャッシュは View ごとではなく **Document ごと × WidthConfig** で共有し、可視範囲 ± 50 行と「最近触った長大行」のみ保持 (LRU、上限 1,000 行)。

### 14.5 Decoration の合成

```rust
pub struct Decoration { range: ByteRange, style: StyleDelta, priority: u16, kind: DecorationKind /*Selection|SearchMatch|Syntax|Diagnostic|AiDiff|BracketMatch|Whitespace|LossyEncoding*/ }
pub trait DecorationProvider { fn decorations(&self, doc: DocumentId, visible: ByteRange, out: &mut Vec<Decoration>); }
```

- Provider は **可視範囲を渡されて** その範囲の Decoration だけ返す ([02 §10.1](02-architecture.md#101-mvp-時点で用意する拡張点-内部-api))。全文の Decoration を保持する義務はない。
- **v0.1 では `DecorationProvider` trait を定義しない**。選択と検索一致 (と制御文字・全角スペース・タブの可視化) のスタイルは Frame 生成で直接合成する。trait は最初の外部利用者 (tree-sitter、v0.4) と同時に導入する (内部構造であり、先に trait 化しても公開面には影響しない)。`Decoration` / `DecorationKind` / 優先度の表は v0.1 から使う。
- 合成は優先度順 (低 → 高) に `StyleDelta` を上書き: Syntax (100) < Whitespace (150) < BracketMatch (300) < SearchMatch (400) < Diagnostic 下線と LossyEncoding (§8.7) (500、下線のみ) < AiDiff (600) < Selection (900、背景のみ) < カーソル (frontend)。数値は 07 のテーマで調整可能。
- 書記素の内側で Decoration が切れる場合 (バイト精度の診断など。検索の一致は §12.1 で書記素境界のものだけを採るので当たらない)、その書記素全体を含める方向に拡張する。

### 14.6 意味的 UI モデルの生成 (core 側の要件。型の正本は 02 §8.2)

テキスト領域以外の **意味的 UI モデル** を core がどう生成するかを本節に置く。型の骨格 (`Frame` / `OverlayModel` / `RenderRow` のフィールド) は 02 §8.2 が正本であり、本節は「core が何を埋めるか」と不変条件だけを定める。TUI とヘッドレス参照 Frontend が同時にこのモデルを消費するため、`OverlayModel` 等への変種追加は 02 の契約変更手続きに従い、frontend は未知の変種を既定描画に退化させる (02 §8.2)。

| モデル | core が埋める内容 | 根拠 |
|---|---|---|
| `Frame.messages` | `current: Option<Message { level, text, hint }>`、`unread: u16`。メッセージ行 (FR-505) の素材。`hint` は「どうすればよいか」(NFR-602) で、frontend は切り詰め時に `text` から縮める | 04 §8.9 |
| `OverlayModel::List` | `input: InputFieldModel` (02 §8.2: 1 書記素 = 1 Span の幅付きセル列、カーソルと選択は欄内の書記素番号。幅は core が `WidthConfig` (曖昧幅の設定を含む) で計算する)、`items[i] { label, match_ranges (label の **書記素番号** の半開区間の列。バイト位置ではない), keys: Vec<KeyLabel> (解決済みキーマップからの逆引き), disabled_reason: Option<CellText> (解決済みの文言、02 §8.2) }`、`selected`、`total`、`truncated`。パレット・バッファ一覧・ファインダ共通。照合と並び順は core (v0.1 は前方一致 → 部分一致 → アルファベット順。ファジー順位付けと最近実行順は v0.2、01 FR-602 / 07 §11.4)。**パレットの項目** は `hidden` でない全コマンド (`hidden` = `internal.*` と、オーバーレイ内専用の `ui.confirm` / `ui.list_next` / `ui.list_prev` / `ui.list_page_down` / `ui.list_page_up` / `ui.dialog_choose`。一覧の正本は 07 §11.6)。**引数の尋ね方** (v0.1): 選んだコマンドに既定値の無い必須引数があれば、同じオーバーレイの中で順に尋ねる。列挙値の引数は `List` (例: `file.reopen_with_encoding` → 対応する 7 つの文字コード)、それ以外は `Prompt`。`Esc` で 1 段戻る。省略可能な引数と既定値付きの引数は尋ねない (各コマンドの既定の動作) | 04 §8.1 / FR-605 (IME 追従には入力欄カーソルが必要) / FR-111 / FR-601 |
| `OverlayModel::Prompt` | `InputFieldModel` + 説明 + 検証エラー + 候補リスト (v0.4) | 04 §8.2 |
| `OverlayModel::Confirm` | `message`、`choices [{ id, label, key (アクセラレータ), destructive }]`、`default`、`focused` (初期値は `default`。`tab` / `right` で次、`shift+tab` / `left` で前のボタンへ移り、`enter` で `focused` のボタンを実行、`esc` は常にデータを変えない側。アクセラレータは組込みの dialog 層が単一書記素の `Text` を `key` と照合して `ui.dialog_choose` を実行する、02 §7.2 K1。破壊的な選択肢は `default` にしない。IME が ON で文字が前編集に入る場合は Tab / 矢印 + Enter で選べることを示す)、`details: Option<Vec<CellText>>` (1 行 = 1 つの `CellText`。core が解決・幅計算・無害化した文言、02 §8.2。§8.6 の置換の一覧に使い、送信プレビュー・権限要求の詳細表示に拡張可、06 X7)、`reason_untrusted: bool` (要求理由が非信頼であることの表示指示。v1.L、02 §8.2) | 04 §8.3 / 06 §13.1 |
| `OverlayModel::Help` (**v0.2**。v0.1 は変種の予約のみで生成しない。v0.1 のヘルプはコマンドパレットで代用する、01 FR-514) | `sections` (キー一覧はキーマップの逆引き) と `scroll`。タブなしの 1 本のスクロールで、`help.keys` / `help.commands` は該当の節へスクロールした状態で開く。ヘルプ内の検索は v0.4 | FR-514 |
| `StatusModel` / `HintModel` | 02 §8.2 の `StatusItem` / `HintItem`。各項目の **長い形と短い形の文言を core が言語ファイルから解決して両方入れ** (例: `[読込中 45%]` と `45%`、`行 6, 桁 22` と `6:22`)、優先度 (`priority`) を付ける。キーは構造化した `KeyLabel` で渡す。ヒントの文脈は v0.1 では **Normal (編集面にフォーカスがあり、チョードの保留が無いとき) と Overlay (オーバーレイ (パレット・プロンプト・確認等)・検索バーにフォーカスがあるとき、またはチョードの保留中)** の 2 つで、文脈ごとの内容と優先度は 07 §11.6 のカタログ「ヒント」列から生成する (ヒント語は短い形を持たない。文脈を細かく分けるのは後の版)。ステータス項目の優先度と短い形の MessageKey は 04 §8.7 が正本。frontend は優先度の低い項目から落とすか短い形に替えるだけで、文言を作らない | FR-503 / FR-504 |
| `Frame.search` (`SearchBarModel`) | 検索バーの入力欄 (`InputFieldModel`)、大小区別の 3 値 (`Smart` / `Sensitive` / `Insensitive`)、正規表現の ON/OFF (v0.2。v0.1 は常に OFF)、件数の状態 (`SearchCount`: 実行中 / 確定 / 上限で打ち切り / 編集後の「約」/ 一致なし。§12.2)、正規表現の構文エラーの文言 (§12.3、v0.2)。検索バーはクローム (04) として描かれるが、モデルは core が出す | FR-401 / FR-402 / FR-404 / FR-605 |
| `RenderRow` | `clipped_left` / `clipped_right` (水平スクロール、FR-506)、`width_risky` (§13.2)、`source` (論理行 + 折り返し番号) | 04 §10 / §4.7 |
| `Frame.focus` / `Frame.primary_cursor` | `focus` は入力の行き先 (編集面 / オーバーレイ / 検索バー)。`primary_cursor` は **編集面にフォーカスがあるときだけ** ハードウェアカーソルの位置 (セル矩形) と形状を入れる。入力欄 (オーバーレイ・検索バー) にフォーカスがあるときは None で、frontend が自分で配置した欄の位置と `InputFieldModel` の幅付きセル列からカーソル位置を求める (core はクロームとオーバーレイの配置を知らないため。02 §8.2) | FR-605 |
| クローム・オーバーレイの文字列の幅 | すべて core が `WidthConfig` で計算して `Span.width` / `CellText.width` に入れる。切り詰めてよい文字列 (ファイル名等) は 1 書記素 = 1 Span で渡す。frontend は幅関数を持たない (02 §8.2) | FR-503 / NFR-605 |

**オーバーレイと検索バーのフォーカス遷移** (`OverlayStack`。規則の利用者向けの説明は 04 §7.5 / §8.4 で、本表はそれを core の状態遷移として書いたもの。両者は同じ内容である): core は前面順のオーバーレイのスタック (`Frame.overlays` の元) と、検索バーの有無 (`Frame.search`) を持ち、`Frame.focus` は「スタックが空でなければ最前面のオーバーレイ、空で検索バーがあれば検索バー、どちらも無ければ編集面」から導く (フォーカスを別に持たない)。規則は 2 つ: **`Esc` (`ui.cancel`) は最前面の 1 段だけを閉じる**、**検索バーで確定した一致は選択範囲として残す**。パレットから実行できるかどうかはコマンドの `when` で決まり (01 FR-601 AC-1)、フォーカスの位置では決まらない: `search.next` / `search.prev` / `search.toggle_case` (when `search.active` = 検索バーが開いている、07 §11.6) は、検索バーの上にパレットを積んだ場面では実行でき (実行するとパレットが閉じ、フォーカスは検索バーに戻る)、検索バーが無い場面ではパレットに淡色と理由で出て実行できない。

| 今の状態 (前面 → 背面) | 事象 | 次の状態とフォーカス |
|---|---|---|
| 編集面のみ | `search.open` | 検索バーを開く → フォーカスは検索バー |
| 検索バー | 入力 (インクリメンタル検索) | 現在の一致を強調するだけ (選択は変えない) |
| 検索バー | `search.next` / `search.prev` (`Enter` / `↓` / `F3` / `↑`) | 移った一致を主選択にする (確定)。検索バーは開いたまま |
| 検索バー | `Esc` | 検索バーを閉じる → フォーカスは編集面。確定していない現在の一致は選択にしない。確定した選択はそのまま残す |
| 検索バー | `search.open` (`Ctrl+F`) | 欄を全選択する (状態は変えない) |
| 検索バー | 編集面へフォーカスを移す | v0.1 はこの事象を持たない (検索バーを開いたまま編集面にフォーカスを戻す手段は無い。編集するときは `Esc` で閉じる) |
| 検索バー | `ui.palette_open` (`global`) | パレットを積む → フォーカスはパレット。検索バーは閉じない |
| パレット / 検索バー | `Esc`、またはコマンドの実行でパレットが閉じる | パレットを除く → フォーカスは元の検索バー。実行したコマンドが新しいオーバーレイ (プロンプト・確認) を開いたときはそれを積み、それが閉じたときに検索バーへ戻る |
| パレット → 引数の一覧 → 入力プロンプト (同じオーバーレイの段) | `Esc` | 1 段ずつ戻る (プロンプト → 引数の一覧 → パレット → 閉じる) |
| 確認 (`ui.confirm.save.overwrite` 等) / 名前を付けて保存のプロンプト | `Esc` (または `[R] 別の名前を入力`) | 確認だけを閉じる → フォーカスはプロンプト。入力した名前は残る (§9.7 の `ConfirmOverwrite` → `AskName`) |
| 任意のオーバーレイが 2 段以上 | `Esc` | 最前面の 1 段だけを閉じ、その下の段にフォーカスを戻す。1 回の `Esc` で 2 段以上は閉じない |

不変条件: INV-F1 (Frame に制御文字・PUA エスケープ文字が入らない) は本節のすべての文字列 (メッセージ・候補ラベル・ファイル名・外部由来の文言) に適用する。無害化関数は `eve-text::sanitize_for_display(&str) -> String` として公開し、06 X12 の要求どおりファイル内容以外にも再利用する。

---

## 15. タブとインデント

### 15.1 設定と検出

```rust
pub struct IndentConfig { style: Spaces | Tabs, width: u8 /*インデント 1 段のセル幅*/, tab_width: u8 /*タブ文字の表示幅*/ }
```

- 既定: `Spaces`, `width = 4`, `tab_width = 4`。言語別 (FR-704) / `.editorconfig` (FR-705) / ワークスペース設定で上書き (07)。
- **自動検出** (v0.4): 読込時に先頭 1,000 行の行頭空白を集計し、タブ主体か空白主体か・空白なら最頻の増分 (2/4/8) を推定。巨大ファイルでは無効 (FR-123)。検出結果は Document の `indent_detected: Option<IndentConfig>` に入り、設定より優先 (設定で検出を無効化可)。

### 15.2 タブ文字の扱い

- タブは 1 書記素・可変幅 (§14.2)。カーソルはタブの前後にのみ置ける (内部には入らない)。
- 選択がタブの一部を覆うことはない (書記素境界)。
- 表示: v0.1 から既定で可視化する (FR-508 の最小の範囲): 先頭のセルに `>`、残りのセルは空白で詰める (§13.2 の「空白の可視化」。曖昧幅の `→` は幅が設定で変わるので使わない)。

### 15.3 インデント操作 (FR-202 / 208)

| 操作 | 規則 |
|---|---|
| 改行 (`edit.newline`) | 挿入位置 p (選択があれば `start`) の行を参照行とし、**参照行の行頭空白 (タブ/空白の生の文字列) のうち p より前にある部分** を複製して、`default_eol` の後に挿入する (p が行頭空白の後ろなら行頭空白の全体、行頭空白の途中ならその手前まで、行頭なら何も複製しない)。p より後ろの文字 (残りの行頭空白を含む) はそのまま新しい行へ移る。構文ベースの追加インデント (FR-803) は `SyntaxProvider::indent_delta` があれば加算。空白のみの行で改行したら、その行の空白を残す (設定で削除可)。改行後のカーソルは複製した空白の直後。期待例は表の下 |
| Tab キー (選択なし) | `style = Spaces` なら次のインデント停止位置までの空白数、`Tabs` なら `\t` を挿入。行頭空白内かどうかに依らない (ソフトタブ) |
| Tab キー (選択あり) / `edit.indent` (選択範囲へのインデントは **v0.2**) | **v0.1: 何もしない** (文書・選択・Undo 履歴・Revision を変えず、メッセージも出さない。選択を置き換えてタブを入れることもしない) (提案、OQ-193)。v0.2: 選択が触れる各行の行頭に 1 段 (`width` 分の空白または 1 タブ) を追加。空行はスキップ。1 Transaction |
| Shift+Tab / `edit.outdent` (選択なしは v0.1、選択範囲は **v0.2**) | 対象の各行 (選択なしはカーソルの行) の行頭空白 (スペースとタブの並び) の先頭から、**表示幅 (`tab_width` で展開した列) が `width` 以内に完全に収まる部分** だけを削除する。列 `width` をまたぐタブ (途中で切れるタブ) は消さない (例 `width = 4`, `tab_width = 4`: `"  \tx"` → `"x"`、`" \t\tx"` → `"\tx"`、`"      x"` → `"  x"`)。ただし先頭の 1 文字が幅 `width` を超えるタブ (`tab_width > width`) のときは、そのタブ 1 つを削除する (何も起きない状態を避ける)。行頭空白が無ければ何もしない (恒等、§6.5)。**v0.1 で選択があるとき**: 何もしない (Tab と同じ。提案、OQ-193) |
| Backspace (行頭空白内、`Spaces`) | 既定は 1 書記素。v0.2 の設定 `editor.backspace_unindent = true` (07 §5.13) で前のインデント停止位置まで削除 (Could) |
| 貼り付け (FR-206) | 自動インデントを適用しない。貼り付けテキストの行頭空白はそのまま |
| 保存時の末尾空白除去 (FR-216) | 保存前に Transaction として適用 (§8.5 と同じ理由)。カーソル行は除外する設定を既定にする (入力中の空白が消える不快感を避ける) |

### 15.4 行の複製・削除・上下移動 (FR-207、v0.2 の M9b)

対象の行は、各 Range が触れる行の和集合を連続する行のまとまり (ブロック) にしたもの。選択が行頭 (列 0) で終わり、その行より前から始まっていれば、最後の行は含めない (選んでいない行を巻き込まない。インデント / アンインデントも同じ)。複数 Range のブロックは重ならないように併合する。どの操作も 1 Transaction (Undo 1 回、`Origin::Command`)。

| 操作 | 規則 |
|---|---|
| `edit.duplicate_line` | ブロックを改行ごと、ブロックの直後にもう一度挿入する。最後の行に改行が無いときは、複製の前に `default_eol` を足す。選択 (カーソル) は、ブロック内の同じ相対位置のまま複製の側へ移る |
| `edit.delete_line` | ブロックを改行ごと削除する。最後の行が改行で終わっていなければ、手前の改行も取り除く (空の行を残さない)。文書が 1 行だけなら空になる。カーソルは、詰めた行の先頭 (最後の行を消したときは新しい末尾) |
| `edit.move_line_up` / `edit.move_line_down` | ブロックと、隣の 1 行の **内容を入れ替える。改行は動かさない** (改行の種類と、最後の行に改行が無いことはそのまま)。先頭のブロックの上移動と、末尾のブロックの下移動は何もしない。選択はブロックについていく |

- インデント操作後の Selection は、各 Range の両端を写像 (`MapPolicy::Own`)。行全体を選んでいた場合は行全体の選択を維持する (行頭が後ろにずれない)。
- **改行の自動インデントの期待例** (FR-202 の受入条件。`|` がカーソル、`·` が空白 1 つ、`→` がタブ):

| 改行前の行 | 改行後 | 補足 |
|---|---|---|
| `····foo\|` | `····foo` / `····\|` | 行末: 行頭空白の全体を複製 |
| `→foo\|` | `→foo` / `→\|` | タブはタブのまま複製 |
| `····foo\|bar` | `····foo` / `····\|bar` | 行の途中 |
| `\|····foo` | (空行) / `\|····foo` | 行頭: 何も複製しない (インデントが二重にならない) |
| `··\|··foo` | `··` / `··\|··foo` | インデントの途中: カーソルより前の `··` だけを複製し、新しい行のインデントは元と同じ 4 |
| `····\|` (空白だけの行) | `····` / `····\|` | 空白を残す (設定で削除可) |

- **選択がある状態の編集** (v0.1。すべて 1 Transaction で、Undo 1 回で選択ごと元に戻る (`selection_before`)。複数 Range では各 Range に同じ規則を適用し §4.5 で合成する):

| 操作 | 規則 | Origin |
|---|---|---|
| 文字入力 (`edit.insert_text`) | `[start, end)` を入力文字列で置換。カーソルは挿入の直後 | `User` (R5 では挿入として扱う) |
| Enter (`edit.newline`) | `[start, end)` を「改行 + 自動インデント」で置換。参照行は **`start` の行**、複製するのは `start` より前の行頭空白 (上の規則の p = `start`) | `User` |
| Backspace / Delete (`edit.delete_backward` / `edit.delete_forward`) | `[start, end)` を削除 (1 書記素は追加で消さない) | `User` |
| 単語単位の削除 (§5.3) | `[start, end)` を削除 (単語単位には広げない) | `User` |
| 貼り付け (`edit.paste`) | `[start, end)` を、改行を正規化 (§8.4) した貼り付けテキストで置換。自動インデントは適用しない。カーソルは挿入の直後 | `Paste` |
| Tab / Shift+Tab | v0.1 は何もしない (上表。提案、OQ-193) | — |
| 切り取り (`clipboard.cut`) | 選択をクリップボードへ入れてから `[start, end)` を削除 | `Command` |

  テスト (v0.1): 選択がある状態で Tab / Shift+Tab を押すと、文書・選択・Undo 履歴・Revision・dirty が変わらず、メッセージも出ない。選択がある状態の文字入力・Enter・Backspace・Delete・貼り付けは、それぞれ 1 回の Undo で選択した文字列と選択が元に戻る。

### 15.4 混在ファイル

- タブと空白が混在する行は表示幅で判断し (`tab_width`)、「1 段削除」は表示幅ベースで行う。混在の自動修正は行わない (v0.1、明示コマンド `edit.convert_indentation` で変換、v0.4)。

---

## 16. シンタックスハイライト基盤

### 16.1 版と本書の範囲

- 実装 (tree-sitter、`eve-syntax`) は **v0.4** ([ADR-0015](10-decisions.md#adr-0015-シンタックスハイライト方式))。v0.1 で用意するのは `Document.language` (`LanguageId`)、Decoration の型 (§14.5)、`DocumentChanged` 通知の ChangeSet 形式と、`SyntaxProvider` / `detect_language` の **名前の予約** だけである。§16.2〜§16.5 の署名・規則は v0.4 (tree-sitter と同時) に確定する案である (02 §10.1)。
- 「v0.1 で tree-sitter を一時的に組み込んで試す」ことは **しない** (バイナリサイズ NFR-205、C コンパイラ依存を v0.1 で抱えない)。

### 16.2 言語判定

```rust
pub struct LanguageId(Arc<str>);   // "rust", "markdown" … 07 の言語定義に対応
fn detect_language(path: Option<&Path>, first_line: &str, cfg: &LanguageRegistry) -> Option<LanguageId>;   // v0.4
```

- 判定順: 設定の明示 (`lang.set`) > ファイル名完全一致 (`Makefile`, `Cargo.toml`) > 拡張子 > 先頭行の shebang (`#!/usr/bin/env python`)。**モードライン (`vim: ft=...`) は読まない** (NFR-502)。
- 判定は読込完了時と「名前を付けて保存」時に行う。未判定は `None` (プレーンテキスト)。

### 16.3 SyntaxProvider trait

```rust
pub trait SyntaxProvider: Send {
    fn supports(&self, lang: &LanguageId) -> bool;
    fn parse(&mut self, req: ParseRequest) -> ParseResult;     // ジョブ内で実行
    fn highlights(&self, tree: &SyntaxTree, range: ByteRange) -> Vec<(ByteRange, ScopeName)>;
    fn indent_delta(&self, tree: &SyntaxTree, text: &Snapshot, line: LineIdx) -> Option<i8>;   // FR-803
    fn node_ranges_at(&self, tree: &SyntaxTree, pos: Pos) -> Vec<ByteRange>;                  // FR-805 構文選択
    fn bracket_pairs(&self, tree: &SyntaxTree, pos: Pos) -> Option<(ByteRange, ByteRange)>;    // FR-308
}
pub struct ParseRequest { lang: LanguageId, text: Snapshot, revision: Revision /*Snapshot と対で渡す (§2.2)*/, prev: Option<(SyntaxTree, Vec<InputEdit>)>, cancel: CancelToken, budget: Duration }
pub struct InputEdit { start: ByteOff, old_end: ByteOff, new_end: ByteOff, start_pt: LineByte, old_end_pt: LineByte, new_end_pt: LineByte }
pub enum ParseResult { Complete(SyntaxTree), Partial(SyntaxTree) /*budget 超過*/, Cancelled, Error(String) }
```

- `SyntaxTree` は不透明型 (tree-sitter の `Tree` を内包。`Tree` は `Send` でクローンが安い)。core は中身を見ない。
- **InputEdit は core が ChangeSet から生成** する。`changed_ranges()` の各要素を旧/新の `LineByte` に変換 (`pos_to_line_byte`、旧座標は適用前の Snapshot で計算) して渡す。tree-sitter の「古い木に編集を適用してから再解析」に必要。
- **TextProvider**: tree-sitter には rope のチャンクをそのまま与える (`chunks()`、T6)。全文コピーなし。
- 実行は **ジョブ** (`bg` スレッド。可視範囲を含む解析を先に)。結果は `(DocumentId, Revision, SyntaxTree)` で返り、§17 の方針 (b) (古ければ破棄して再要求) を取る。ただし **再要求は古い木 + その後の InputEdit 列** を渡すので、インクリメンタル性は失われない (core は「最後に Provider へ渡した Revision 以降の ChangeSet」を保持する)。
- **タイムアウト**: `budget` (既定 20 ms) を超えたら `Partial` を返し、core は古いハイライトを **写像して** 表示し続ける (NFR-110: 色付けの反映 50 ms)。連続して超過する文書は「構文解析が遅い」として自動無効化 (閾値: 5 回連続)。
- **巨大ファイル**: `file.large_file_threshold` (20 MiB) 超は無効 (FR-123)。長大行 (`file.long_line_threshold`) ではその行の解析結果を使わない。
- **注入言語** (Markdown 内のコードブロック等) は Provider の内部責務 (`highlights` が合成済みを返す)。

### 16.4 ハイライトの Decoration 化

- `highlights(tree, visible ± 1 画面)` の結果 `(ByteRange, ScopeName)` をテーマ (07) で `StyleDelta` に解決し、`DecorationKind::Syntax` として §14.5 に投入する。
- 編集直後、新しい木が届くまでは **旧 Decoration を ChangeSet で写像** して表示 (変更範囲はプレーン表示)。これにより入力遅延に解析が乗らない (NFR-110)。

### 16.5 括弧対応 (FR-308) の二段構え

- 括弧対応 (FR-308) は v0.4。Provider なし (プレーンテキスト・未対応言語): 文字ベース走査 (§5.7)。
- Provider あり: `bracket_pairs` を優先。文字列・コメント内の括弧を誤認しない。

---

## 17. Revision とジョブ結果の採否規則

[02 §5.3](02-architecture.md#53-revision-とスナップショット) を具体化する。

### 17.1 Revision

- `Revision(u64)`: Document ごとに単調増加。`Document::apply` (Undo/Redo を含む) で +1。Reload / Recovery でも +1。
- `Revision` の型は eve-view が定義し (02 §5.3)、`Snapshot` は Revision を持たない。Document が `(Snapshot, Revision)` の対を作ってジョブに渡し (`JobRequest { doc, base: revision, .. }`、02 §9.3。開くジョブも Loading の文書の Revision を入れる)、core はジョブ表から `(DocumentId, base Revision)` を引いて結果の採否を決める。
- **`pending_changes` は v0.1 では作らない**。v0.1 のジョブは読込 (再読込を含む)・保存・metadata の照合・検索・Undo の実体化・E3 の置換・すべて置換・改行コードの変換の ChangeSet 構築・退避先に保存 (と v0.2 のスワップ) だけで、保存は Snapshot と履歴上の位置で済み、検索結果と置換・変換の ChangeSet は編集後に捨てて作り直せば足りる (§12.2 / §17.2)。写像が必要な利用者 (AI 提案・LSP 診断) は v1.L なので、`Document.pending_changes` はフィールドだけ予約 (常に空) し、最初の利用者の着手時に次の規則で実装する。
- (v1.L〜) Document は `pending_changes: ChangeLog` (02 §5.1 / §5.3。中身は `(Revision, Arc<ChangeSet>)` の列) として、**「最古の未決ジョブ / Proposal / 診断の base Revision」まで** の ChangeSet を保持し、古い Revision からの写像を可能にする (件数では打ち切らない)。
- **メモリを一定に保つ規則** (02 §5.3 の 1〜3 と同じ): (a) 保持範囲は「その文書に対する最古の未決要求の base Revision」以降で、それより古い ChangeSet は破棄する (未決要求が無ければ空)、(b) 保持範囲のうち **どの未決要求の base にも当たらない区間** の ChangeSet は `compose` で 1 つに畳み込む (要求 k 個に対し ChangeSet は最大 k+1 個。どの要求から見ても写像の正確さは変わらない)、(c) 保持中の合計バイト数 (ChangeSet の挿入テキスト + 固定オーバーヘッド) が予算 (既定 **16 MiB**、設定キー `editor.pending_changes_max_bytes` は 07 §5.1 (公開は v1.L)) を超えたら、**最古の要求** を「写像不能 = 失効」として通知し、保持範囲を次に古い要求の base まで縮める (予算内に収まるまで繰り返す。失効させるのは最古の側からで、全体を一度に失効させない)。
- 未決の要求は `JobTable` / `AiSession` / 診断保持が `base: Revision` を登録・解除することで追跡する (06 X9 の汎用要求 ID)。

### 17.2 ジョブ種別ごとの採否

**v0.1〜v0.2 のジョブ種別 (`Open` / `Save` / `Metadata` / `Search` / `Materialize` / `ReplaceUnmappable` / `ReplaceAll` / `ConvertEol` / `WriteRecovery` / (v0.2) `Swap`) の採否は [02 §9.3](02-architecture.md#93-ジョブとキャンセル) の採否表が正本** で、下表のその行は要約である (食い違えば 02 に従う)。後の版の種別 (構文解析・横断検索・AI・LSP) の規則は本表が書く。

| ジョブ | 結果が古い (Revision < 現在) ときの扱い | 根拠 |
|---|---|---|
| ファイル読込 (Complete) | Loading 中は編集不可なので古くならない。途中で閉じられたら破棄 | §9.2、02 §9.3 |
| 保存 | Snapshot のノードを `saved` にする。Revision が進んでいても、取消しを送った後に届いた `Saved` でも **採用** (保存した版の記録が目的。`canonical` / `meta` も反映、§9.3 手順 7) | §7.4 / §9.7、02 §9.3 |
| 退避先に保存 (`WriteRecovery`) | 採用 (書いた場所を示すだけで、文書の状態は変えない) | §9.7、02 §9.3 |
| 検索 (全件) | **v0.1: (b) 破棄して再要求** (Revision が一致しない結果は捨て、入力が止まって 300 ms 後に可視範囲から再検索)。**v1.L〜 (`pending_changes` 導入後): (a) 写像して採用**。ChangeSet を順に `map_range` し、変更範囲と交差する一致は無効化してその周辺を再検索。写像不能なら再要求 | §12.2 |
| 構文解析 | **(b) 破棄して再要求** (ただし古い木と InputEdit 列を渡す)。見た目は旧 Decoration の写像でつなぐ | §16.3 |
| スワップ書き出し (v0.2) | `JobResult::SwapWritten { revision }` で書いた Revision を記録するだけ (古くても採用)。待っている要求があれば最新の Snapshot で 1 回だけ再実行 | §10.2 |
| 文字コード往復検証 (§8.7) | 読込時のみ。古くならない | — |
| Undo の実体化 (`JobKind::Materialize`) | Revision ではなく **`SliceKey`** で採否を決める: その `Slice` がまだ履歴 (undo / redo) にあれば `Rope` に置き換えて採用、無ければ破棄 | §6.5 |
| AI Proposal (v1.L) | **(a) 写像** (ハンクごとに `map_range`)。接触・交差したハンクは失効 (§18) | ADR-0013 |
| LSP 診断 (v1.L) | (a) 写像。変更行の診断は「古い可能性」マーク | 02 §5.3 |
| LSP 補完 (v1.L) | (b) 破棄 | — |
| ワークスペース検索 (v0.4) | Document 単位で (a) | — |
| 改行コード変換・すべて置換の ChangeSet 構築 (`JobKind::ConvertEol` / `ReplaceAll`、v0.1) | **(b) 破棄して再要求**: 結果の base Revision が今の Revision と違えば捨て、今の Snapshot で 1 回だけ作り直す (1 MiB 以下の文書は同期で再計算)。それも古ければ適用せずに `msg.eol.convert_stale` / `msg.search.replace_stale` を出す。文書が閉じられた・検索バーが閉じられたら破棄 | §8.4 / §12.5 |
| E3 の置換の ChangeSet 構築 (`JobKind::ReplaceUnmappable`、v0.1) | **(b) 破棄して再要求**: 結果の base Revision が今の Revision と違えば結果を捨て、今の Snapshot で作り直す (§9.7 の `BuildingReplacement`)。確認 (`ConfirmReplacement`) の後、適用の時点でも Revision を照合し、違えば作り直す。文書が閉じられたら破棄 | §8.6 / §9.7 |

### 17.3 ジョブの投入規律

- 同一 `(DocumentId, JobKind)` の実行中ジョブは **1 つ** (検索・構文・スワップ)。新しい要求は古いものをキャンセルするか (検索・構文)、完了後に最新で再実行する (保存・スワップ: キュー深さ 1。実行中のものはキャンセルせず、待っている要求は文書ごとに 1 つに畳み、完了後に最新の Snapshot で 1 回だけ再実行する。02 §9.1)。例外は `Materialize` で、ノードごとに独立しており、後続の要求にキャンセルされず順に実行する (文書を閉じたときはキャンセルする)。
- ジョブは Snapshot 以外に **Document の可変状態を参照しない** (Send の制約で型的に保証)。ワーカーとメインスレッドの間で `Mutex` を共有しない (メッセージのみ。ワーカーのパニックでロックが poison されるのを防ぐ)。
- ジョブ結果の処理はメインスレッドで、1 件あたり O(結果サイズ) に抑える (大きな結果は分割バッチで届く)。
- **受信口に送る側の規則** (受信口は上限なしの 1 本なので、送る側がまとめる。02 §9.2): 進捗は 10 Hz 以下、検索結果は 256 件または 20 ms ごと、入力スレッドは連続する `Text` を 1 メッセージ (最大 64 KiB) にまとめる (04 §6.1)。debug ビルドでは受信口の滞留数を計測し、1 万件を超えたら警告する。
- スレッド構成は **固定 2 本** (`io`: 読込・保存・metadata を直列、`bg`: 検索・スワップ (v0.2)・Undo の実体化・E3 の置換・すべて置換・改行コードの変換の ChangeSet 構築・退避先に保存 (`WriteRecovery`)。新しい要求が来たら同種の古い要求をキャンセルする。例外は実体化 (キャンセルしない) とスワップ (完了後に最新で 1 回再実行)。02 §9.1)。`JobSpawner` Port の形は変えずに、後から内部の構成を差し替えられる。
- **`bg` のキューの優先順** (提案、OQ-198): `bg` の待ち行列は到着順ではなく次の優先順で取り出す (同じ優先度の中は到着順)。(1) `WriteRecovery` (利用者が終了や退避を待っている)、(2) `Materialize` (待つ間は旧 Snapshot が会計外のメモリとして残る、§6.5)、(3) `ReplaceUnmappable`・`ReplaceAll`・`ConvertEol` (利用者が結果を待っている) と (v0.2) `Swap`、(4) 検索 (全件・単発・可視範囲の行、§12.2)。実行中のジョブは中断しないが、**検索はチャンクごとの取消しの確認と同じ箇所で、(1)〜(3) の要求が待っていれば走査位置を保存して自分をキューの (4) に積み直し** (同じ検索の続きとして、結果の Revision と件数は引き継ぐ)、先に高い方を実行させる。これで (1)〜(3) の待ち時間は検索の 1 チャンク分 (1 MiB の走査、1 ms 前後) 以下になり、100 MB の全件検索の後ろで実体化や退避が数百 ms 待つことはない。検索どうしの優先は「単発の `search.next` / `search.prev` (利用者が待っている) > 可視範囲の行 > 全件」とする。

---

## 18. AI 差分適用 (Proposal) との接合

**`Proposal` / `Hunk` の型と失効規則の正本は [02 §10.2.1](02-architecture.md#1021-proposal-モデル-正本)** であり、生成・提示・承認のフロー・UI モデル・状態遷移の詳細は [06 §7](06-ai-integration.md#7-編集適用モデル-proposal) が書く。本章は 02 の形 (`Proposal { id, request, summary, items: Vec<ProposalItem::{Edit{doc, base, hunks}, Create, Delete, Rename}> }`、`Hunk { id, range, replacement, note, state, flags }`) と保守的な接触規則 (端に接する挿入も失効) を前提に、02 / 06 が依拠する **core が提供する演算** のみを定め、型を再定義しない。型の定義 crate は **eve-view (データ)** とし、演算は eve-core に置く案とする (駆動 API / MCP sidecar が Proposal をプロトコルで運ぶため。所有 crate の決定は 02 §12.2)。実装は v1.L で、v0.1 は演算のうち `ChangeSet::map_pos` / `map_range` / `from_sorted_edits` (選択・検索結果の写像に使う) だけを持つ。

### 18.1 core が提供する演算 (06 X3 / X5 / X9 への回答)

| 演算 | 内容 | 06 での用途 |
|---|---|---|
| `ChangeSet::map_range(range) -> RangeMap` (§6.4) | `Mapped(新範囲)` または `Overlapped { kind: Inside \| Straddle \| Touch }` (`OverlapKind` は `#[non_exhaustive]`。3 変種を残すことは確定、OQ-36)。**AI ハンクは `Touch` (範囲の端に接する挿入) も失効** とする (06 §7.4 の保守的規則) | `DocumentChanged` ごとのハンク写像、`Stale` 判定 |
| `ChangeSet::compose` | `pending_changes` (§17.1) の畳み込み | base が古い Proposal の一括写像 |
| `ChangeSet::from_sorted_edits(base, edits)` | 承認済みハンク (`range` + `replacement`) を **1 つの ChangeSet** に合成。ハンクは位置で分割されており重ならない | 06 §7.5 の適用 |
| `ChangeSet::rebase(other, policy)` (**v1.L**) | OT の transform。`RebasePolicy::{OtherFirst, SelfFirst}` の **相補的な組** を提供する (収束性は `a.rebase(b, OtherFirst)` と `b.rebase(a, SelfFirst)` の組で成立する。同じ tie ポリシー同士では同位置同時挿入で成り立たない)。06 のハンク写像は `map_range` による逐次写像 + 失効で足りるため、v0.1 の API には入れず、v1.L の AI 提案の着手時に要否を決める (収束性のプロパティテストも同時) | — (予約) |
| `Snapshot` + `Revision` (§2.2 / §17) | 範囲の切り出しは O(範囲長 + log n) | ContextBundle の素材 |
| `Transaction { origin: Origin::Ai { request, proposal }, undo_group: Break }` (§6.3 / §7.3) | `Origin::Ai` の中身は 02 §10.2.1 の `{ request, proposal }` を **不透明な ID 群** として持つ (06 が監査ログ用に provider / model 等を付加する場合は `Transaction` の拡張メタデータ欄 (06 X1) に置く)。前後と結合しない | 06 §7.5 |
| `Document::apply` の `StaleBase` (§6.5) | `changes.len_before != len_bytes()` ならエラーで返す。06 は承認時に現在 Revision へ写像済みのハンクを渡すので通常は起きない。起きたら (承認操作と並行して編集が入った稀なケース) 06 §7.4 の写像を再実行し、UI を更新してから再度承認を求める (黙って再試行しない) | 06 §7.5 |
| 汎用要求 ID (06 X9) | `JobTable` のキャンセルトークンと `pending_changes` の base 登録を、ジョブ以外の要求 (AI セッション、LSP 診断) にも使える `RequestId` に一般化する | 06 §4.5 / §17 (X9) |

- **diff の計算場所**: AI 出力 (置換後テキスト) → ハンク (`range` + `replacement`) の差分計算は **eve-ai 側** (06 §4.3 `parse`)。core は置換後テキストを直接受け取らない。差分ライブラリ (imara-diff / similar) は eve-ai の依存であり、v0.1 の core には入れない。
- **適用後のカーソル**: 06 §7.5 に合わせ「最初の適用ハンクの末尾」。
- **外部クライアント由来の Proposal**: 駆動 API (v0.2 以降) や MCP sidecar (07 §13.1 Stage D、v1.L-b 候補) 経由で提出された Proposal は、承認後の Transaction の Origin を `Origin::External(client)` とする (`Ai` ではない。06 §8.1 の主体に対応)。演算は同じ。
- 選択範囲への指示編集 (FR-903) では、AI には選択範囲のテキストだけを送っても、ハンクの `range` は **文書全体座標** (06 `EditBase::Open { doc, base }` 基準)。

### 18.2 差分表示の素材

- `DecorationKind::AiDiff` で、削除部分 (打消し表示) と挿入部分 (仮想テキストとして行内/行間に表示) を Frame に載せる。文書自体は変更しない (承認前)。
- 削除・挿入の両方を見せる「インライン差分」と、2 ペインの差分ビュー (FR-517) の両方に、同じ `Proposal.items[].hunks` を素材として使う。
- Proposal の表示中も文書は編集可能で、Decoration は ChangeSet で写像される (`Stale` ハンクは失効表示)。

### 18.3 相性の確認 (設計視点)

| 観点 | 評価 |
|---|---|
| 位置表現 | バイトオフセットの範囲は LLM の出力 (テキスト) と直接は対応しないが、diff で変換する層が eve-ai に 1 箇所あれば済む |
| 同時編集 | ハンクの逐次写像によりユーザーが待たずに編集を続けられる (P3 UI を止めない)。接触したハンクは失効し再要求 (3-way マージはしない、06 §7.4) |
| 安全性 | 文書変更は `Document::apply` のみ、Origin::Ai で監査可能、文書あたり Undo 1 回で戻る (ADR-0013 / FR-903) |
| 複数カーソル | Proposal は Selection に依存しない (文書座標)。複数 Range に同じ指示を出す場合は Range ごとに Proposal を作るか 1 つに合成するかは 06 で決める |
| 巨大ファイル | Proposal は差分のみなので文書サイズに依存しない。diff 計算は eve-ai 側でジョブ化 |

---

## 19. 型スケッチのまとめ

> 実装の API ではなく、責務と所有関係を示す擬似コード。

```rust
// ── eve-text ───────────────────────────────────────────────
pub struct ByteOff(pub usize);  pub type Pos = ByteOff;
pub struct ByteRange { start: ByteOff, end: ByteOff }   // eve-text の範囲。eve-view にも同名の境界型 ({ start: u64, end: u64 }) があり、別の型 (変換規則は 02 §12.2)
pub struct CharIdx(usize);  pub struct Utf16Off(usize);  pub struct LineIdx(pub usize);
pub struct LineByte { line: LineIdx, byte_col: usize }
pub struct LineGrapheme { line: LineIdx, col: usize }
pub struct LineDisplay { line: LineIdx, dcol: u32 }
pub enum Bias { Before, After }
pub enum LineEnding { Lf, CrLf, Cr }

pub struct Text;  pub struct Snapshot /* Clone O(1), Send + Sync。Revision は持たない (Document が対にして持つ、§2.2) */;  pub struct TextBuilder;
pub trait TextRead { /* §2.2。dyn で使える (chunks は具体型 Chunks<'_> を返す) */ }
pub trait TextReadExt: TextRead { /* §3.2 の位置変換。TextRead を実装する全型と dyn TextRead に blanket 実装 */ }
pub struct Chunks<'a>;  pub struct TextSlice<'a>;

pub struct Range { anchor: Pos, head: Pos }
pub struct Selection { ranges: SmallVec<[Range; 1]>, primary: usize }
pub enum MapPolicy { Own, Other }

pub enum Op { Retain(usize), Delete(usize), Insert(InsertText) }
pub enum InsertText { Str(Arc<str>), Slice { key: SliceKey, snapshot: Snapshot, range: ByteRange } /* 削除 64 KiB 以上の記録直後のみ */, Rope(Text) /* 実体化後・全置換 (§6.5)。ropey の型は公開しない */ }
pub struct ChangeSet { ops: Vec<Op>, len_before: usize, len_after: usize }
pub struct Edit { range: ByteRange, insert: Arc<str> }
pub enum RebasePolicy { OtherFirst, SelfFirst }   // v1.L (§18)
pub struct Conflict { hunks: Vec<usize> }         // v1.L
pub enum RangeMap { Mapped(ByteRange), Overlapped { mapped: Option<ByteRange>, kind: OverlapKind } }
#[non_exhaustive] pub enum OverlapKind { Inside, Straddle, Touch }   // 3 変種を残す (OQ-36、確定)
pub fn is_width_risky(g: &str) -> bool;
pub fn sanitize_for_display(s: &str) -> String;   // INV-F1 の無害化 (§14.6)

pub fn snap_to_grapheme(text: &dyn TextRead, p: Pos, bias: Bias) -> Pos;
pub fn next_grapheme(text: &dyn TextRead, p: Pos) -> Option<Pos>;  pub fn prev_grapheme(text: &dyn TextRead, p: Pos) -> Option<Pos>;
pub trait WordSegmenter { fn next_boundary(&self, text: &dyn TextRead, p: Pos) -> Option<Pos>; fn prev_boundary(&self, text: &dyn TextRead, p: Pos) -> Option<Pos>; }
pub struct WidthConfig { tab_width: u8, ambiguous_wide: bool, control_style: ControlStyle, emoji_width: EmojiWidth /* Auto | One | Two */, show_invisible: bool }
pub fn grapheme_width(g: &str, cfg: &WidthConfig) -> u8;

// ── eve-view (境界型。本書は使うだけ) ─────────────────────────
pub struct Revision(u64);   // 型の所有は eve-view (02 §5.3 / §12.2)。Document ごとの単調増加

// ── eve-core ───────────────────────────────────────────────
pub struct Document {   // フィールド名・型の正本は 02 §5.1。本書が加えるものは後半に列挙
    id: DocumentId, text: Text, path: Option<FsPath /* 02 §4.4 */>, file_id: Option<FileId>, disk_stamp: Option<FileStamp /* 02 §4.5 */>,
    format: TextFormat, revision: Revision, history: UndoHistory,
    pending_changes: ChangeLog /* 02 §5.3。v0.1 は常に空。v1.L で §17.1 の予算規則 */,
    language: Option<LanguageId>, decorations: DecorationSet /* 02 §5.1 */, read_only: ReadOnlyReasons /* 理由の集合 (02 §5.1、§9.6)。読込中は理由に入れない */,
    load_state: Loading { progress } | Ready | Failed(err) /* Partial は値のみ予約 (v0.3) */,
    // 以下は本書が 02 §5.1 に加えるもの
    canonical: Option<FsPath>, indent: IndentConfig, external_modified: Option<FileStamp>,
    saving: Option<JobId>, swap: SwapState /* v0.2 */, syntax: SyntaxState /* v0.4 */, layout_cache: LayoutCacheTable,
}
pub struct View { id: ViewId, doc: DocumentId, selection: Selection, goal_dcol: Option<u32>, scroll: ScrollPos { top: Pos /*先頭可視行の行頭*/, wrap_idx: u32 /*v0.2*/, left_col: u32 /*水平スクロール量 (表示列)、v0.1*/ }, wrap: bool /* v0.1 は常に false */, viewport_size: CellSize }
pub struct Transaction { changes: ChangeSet, selection_before: Option<Selection>, selection_after: Option<Selection>, origin: Origin, is_undo: bool, view: Option<ViewId>, timestamp: Tick /* Clock Port、02 */, undo_group: UndoGroupHint, meta: TransactionMeta /* 拡張メタデータ欄 (06) */ }
pub enum Origin { User, Command(CommandId), Paste, Reload, Ai { request: AiRequestId, proposal: ProposalId }, Lsp, Plugin(PluginId), External(ClientId), Recovery, System }
pub enum UndoGroupHint { Auto, Merge, Break }
pub struct NodeId(u64);   // 履歴の位置の通番。単調増加・再利用しない (§7.1、02 §5.1)
pub struct UndoHistory { undo: VecDeque<UndoNode>, redo: Vec<UndoNode>, base: NodeId, next_id: u64, saved: Option<NodeId>, budget: UndoBudget }   // 線形 (§7.1)。木構造は FR-213 で内部を入れ替える
pub struct TextFormat { encoding: Encoding, has_bom: bool, default_eol: LineEnding, eol_stats: EolStats, final_newline: bool, roundtrip: RoundtripInfo /* v0.1 Summary / v0.3 Spans */, invalid_bytes: u32, pua_escape: PuaEscapeMode }   // §8.1。後ろの 2 つが本書の追加
#[non_exhaustive] pub struct SearchQuery { pattern, kind, case, whole_word /* v0.1 は false 固定。変更は v0.2 */, scope /* v0.1 は Document 固定。変更は v0.2 */ }   // 構築は SearchQuery::literal(pattern) 等のビルダー経由 (OQ-41 の準備、§12.1)
// ClipboardText (eve-view、02 §6.3): Inline(String) | Handle(ClipboardHandleId)。Effect::SetClipboard が運ぶ (eve-view は eve-text の型を参照しない、02 §4.3 R4)。
// Handle の中身 ClipboardSource (Snapshot + 範囲、Send) は eve-core の Editor のハンドル表にあり、Host が Editor::take_clipboard(id) で取り出す (§12.6、02 §5.4)
// Proposal / Hunk の型は 02 §10.2.1 が正本 (eve-view に置く提案。eve-ai が生成)。本書は演算のみ (§18)
pub trait SyntaxProvider { /* §16.3。v0.1 は名前の予約のみ、署名は v0.4 */ }
pub trait DecorationProvider { fn decorations(&self, doc: DocumentId, visible: ByteRange, out: &mut Vec<Decoration>); }   // v0.4 で導入 (§14.5)
// FileSystem trait / FileStamp / SaveOptions / FsErrorKind は 02 §4.5 が正本 (§9.1 は要求一覧のみ)
// JobRequest { id /*JobSpawner が採番*/, doc: Option<DocumentId> /*文書に属さない RecoveryScan は None*/, base, kind } / JobKind / JobResult / JobError は 02 §9.3 が正本。v0.1 の JobKind は Open (OpenOptions、§9.2) / Save / Metadata (MetaPurpose: UnchangedSave・SaveAsTarget、v0.2 で ExternalCheck) /
// ReplaceUnmappable (§8.6 E3 の置換、bg) / Search / Materialize (§6.5) / WriteRecovery (§9.7 の「退避先に保存」、bg。結果は RecoveryWritten { path }) /
// RecoveryScan (起動時の退避の走査、io。結果は RecoveryFound { dirs }、§10.1)、
// JobKind の変種は Port を持たない (run_job が JobPorts から取る)。JobRequest.base は Revision (Option ではない)。
// v0.2 で Swap。後の版の変種 (Parse v0.4、EolConvert / ReplaceAll v0.2、WorkspaceSearch v0.4、v0.2 の起動時の保存中フラグの走査 (list_inflight) 等) は最初の利用者と同時に 02 §9.3 に足す
enum Continuation { Quit, Replace(ReplaceTarget), Reopen { encoding: EncodingName } }   // 保存の連鎖の後続 (§9.7)。内部型
```

---

## 20. 不変条件一覧

実装では `debug_assert!` で検査し、リリースでは安全側に補正してログする ([02 §11.1](02-architecture.md#111-エラー処理方針-adr-0012))。プロパティテスト (§22) の性質としても使う。

| ID | 対象 | 不変条件 |
|---|---|---|
| INV-T1〜T5 | TextStorage | §2.5 |
| INV-S1 | Selection | View に格納された全 Range の anchor/head は書記素境界かつ `<= len_bytes()` |
| INV-S2 | Selection | ranges は非空、start 昇順、互いに重ならない (非空同士は接しない)、`primary < len` |
| INV-S3 | Selection | v0.1〜v0.3 では `ranges.len() == 1` (設定フラグで解除、v0.4) |
| INV-C1 | ChangeSet | `Σ Retain + Σ Delete == len_before`、`Σ Retain + Σ Insert.len == len_after`、正規形 (CS6) |
| INV-C2 | ChangeSet | `Insert` の文字列は妥当な UTF-8。全 Op の境界 (各 `Retain` / `Delete` の両端と各 `Insert` の位置) が適用対象で char 境界 (`from_sorted_edits` と `apply` が検査し、違反は `NotCharBoundary`。INV-T1) |
| INV-D1 | Document | `text` の内容と履歴が対応 (読込直後の内容に `undo` スタックの `redo` を順に合成すると現在の内容になる) |
| INV-D2 | Document | `revision` は `apply` ごとに厳密に +1。(v1.L〜) `pending_changes` は Revision 順で、畳み込み後も合成すると「最古の未決要求の base → 現在」の写像を与える (§17.1)。v0.1 では常に空 |
| INV-D3 | Document | `dirty == (history.saved != Some(現在の履歴位置))` (位置は §7.1 の通番 `NodeId`。saved が None なら true)。予算による履歴の破棄の前後で dirty は変わらない |
| INV-D4 | Document | `Loading` 中は `apply` が Rejected (Reload/Recovery を除く) かつ編集不可 (`load_state` から導出し、`ReadOnlyReasons` には理由を入れない。§9.6) |
| INV-D5 | Document | `format.eol_stats` の合計 == `len_lines() - 1`、かつ種類別の件数が内容と一致 (編集ごとに §6.5 の「eol_stats の更新」で保ち、編集の端での CR と LF の合体・分離 (§2.4) と巨大な削除でも近似しない) |
| INV-D6 | Document | `format.final_newline == (最終行が空)` |
| INV-D7 | Document | `invalid_bytes > 0` ⇒ テキスト中の `U+10FF00..=U+10FFFF` の個数 == `invalid_bytes` (PUA 衝突フラグが立っていない場合) |
| INV-H1 | UndoHistory | 各ノードの `redo` と `undo` は互いに逆 (CS2/CS3)。`undo` スタックの `redo` を順に合成したものと `undo` を逆順に合成したものも互いに逆 |
| INV-H2 | UndoHistory | 文書ごと `Σ nodes.bytes <= budget.max_bytes`、または `undo.len() <= 1` (直前 1 操作の例外)。プロセス全体 `Σ <= 256 MiB` は **v0.2** (複数文書と同時、§7.5) |
| INV-H3 | UndoHistory | 新しい編集 (`record`) の後は `redo` スタックが空。ノードの `id` は文書内で一意かつ `undo` スタック上で底から末尾へ増加し、`base` より大きい。`saved` が Some なら、その値は `base`・`undo` のいずれかのノードの `id`・`redo` のいずれかのノードの `id` のどれかである (戻れない位置を指す保存点は None にする、§7.1。保存の完了時にも検査して None にする、§7.4) |
| INV-H4 | UndoHistory | `InsertText::Slice` / `InsertText::Rope` は削除 64 KiB 以上のノードにのみ現れ、`Slice` は実体化ジョブの完了前にしか存在しない (§6.5)。ランダムな 1 万回の編集の後、実体化ジョブがすべて完了した時点の RSS 増分 ≤ Undo 会計の 1.2 倍 (09 §3.2) |
| INV-L1 | LayoutCache | `revision` / `width_cfg_hash` が Document / 設定と一致しないエントリは参照されない |
| INV-J1 | Job | ジョブに渡した Snapshot の Revision ≤ 結果処理時の Document.revision |
| INV-F1 | Frame | Frame に制御文字 (TAB/LF/CR を含む)、§13.2 の可視化対象の不可視文字 (可視化が有効なとき)、および **PUA エスケープとして扱われている** `U+10FF00..FF` (`pua_escape = Enabled` の文書のもの。§8.3) が **含まれない** (無害化済み、NFR-503)。`DisabledCollision` の文書が真に含む `U+10FF00..FF` は通常の私用文字として扱う |
| INV-F2 | Frame | 各 RenderRow の `Σ Span.width <= viewport_cols` |
| INV-F3 | Frame | `RenderRow.width_risky == 行内のいずれかの書記素で is_width_risky() が真` (§13.2) |
| INV-IO1 | 保存 | **Unix** (macOS / Linux を出荷する版): 原子的置換 (`rename`) が完了するまで元ファイルのバイト列は変わらない。**Windows** (v0.1): `ReplaceFileW` の途中失敗時、元ファイルは無傷のまま残るか、または退避名 (`.<name>.eve-bak-*`) で完全な状態で残り、`ReplacedMovedToBackup` で位置が報告される (原子性ではなく回復可能性を主張する)。いずれも失敗時に一時ファイルが残らない (削除不能時はログに位置を残す) |
| INV-IO2 | 保存 | 保存成功時のディスク内容 == Snapshot をエンコードしたバイト列 (ジョブ内で書いたものと同一) |
| INV-IO3 | 保存 | 無編集の上書き保存は書き込まない (§9.3 手順 0) ので文字コードによらずバイト一致。編集後の保存は §8.7 区分 A でバイト一致、区分 B は v0.3 から保持方式で一致 (v0.1 は E2' の確認後に代表バイト列)。区分 D は意味一致 |
| INV-IO4 | 保存・読込 | 利用者のデータ量に比例する確保は `try_reserve` で行い、失敗はエラーとして返す (abort しない)。ユーザーデータの大きさに比例する **再帰を core に書かない** (ChangeSet の合成・履歴の走査・正規表現の構文木は反復で書く。スタックオーバーフローは退避できない) |

---

## 21. 性能目標との対応

数値目標の正本は 01 §3 (OQ-113 で確定)。ここでは 01 §3 の値 (判定区間・統計量を含む) と本書の手段を対応づける。

| NFR | v0.1 のゲート (01 §3) | 本書の手段 | 残るリスク (計測は 09) |
|---|---|---|---|
| NFR-101 起動 | **(a) プロセス内** (`main` 入口 → 最初の `write`+`flush`): p50 8 ms / p99 15 ms (全 OS)。(b) spawn から: Windows p50 40 / p99 80 ms (基準機 ENV-W1 の条件 = Defender 無効・サードパーティ製のウイルス対策ソフト有効で判定、ウォーム。Defender 有効の値は参考値として別記、01 §3.0)。Linux/macOS p50 20 / p99 40 ms (出荷する版)。(a) を CI の回帰検知と判定に使う | core 側: Document なしの `EditorState` 構築は定数時間。正規表現・Unicode テーブルの初期化は遅延 | `unicode-width` 等の静的テーブルはゼロコスト。regex のコンパイルは検索時 |
| NFR-102 1 MB 表示 | (a) プロセス内 p50 25 / p99 50 ms。(b) は NFR-101 (b) + 20 ms | 1 MB は 1 チャンク以内で Complete。rope 構築 ~数 ms、レイアウトは可視行のみ | encoding_rs の UTF-8 検証 ~1 ms |
| NFR-103 入力 | p50 2 / p99 8 ms で判定 (1,000 回計測、ウォームアップ 100 回を除く)。p99.9 (目安 16 ms) は **参考値** として記録し、合否に使わない (01 NFR-101〜105 AC-1)。条件: 120×40、1 MB の日本語混在 UTF-8、カーソルは中央、検索一致のハイライト有効、打鍵間隔 30 ms | `apply` は O(log n) + 逆変換 (削除量)、Selection 写像 O(k)、レイアウトは可視行のみ、検索ハイライトは可視範囲のみ | 可視範囲の同期検索 (§12.2) が複雑な正規表現や長い行で遅い場合 → 1 行あたり 16 KiB と時間予算 1 ms を超える分はジョブに回す |
| NFR-104 100 MB | 進捗表示 ≤ 50 ms / 全読込 **p50 1.0 s** で判定 (30 回計測。最大値は参考値として記録し、目安 1.5 s。01 §3.0 の「統計」) (Partial による先頭表示 300 ms は v0.3)。1 GB は参考値の記録のみ、512 MiB の警告は Must | 全読込は I/O + UTF-8 検証 + rope 構築 (ropey の `from_reader` 相当で 1 GB/s 級)。`metadata` / `file_id` もジョブ内 | 非 UTF-8 (Shift_JIS 100 MB) は再エンコード照合 (§8.7、ストリーミング) 込みで 1.5 倍許容内か (S-01 で実測) |
| NFR-105 100 MB 中央挿入 | p50 1 / p99 5 ms (コマンド実行時間: 編集コマンドの開始から Transaction の適用完了まで。Frame 生成を含まず、01 §3.0.1 の (a) core 区間とは別) | rope O(log n)、逆変換は挿入なので空、`eol_stats` 更新は挿入文字列と編集の端の前後 1 バイトの走査のみ (§6.5) | Undo ノードの結合判定は O(1) |
| NFR-106 長大行 | **v0.1 Must: 閾値 (1 MiB) 超の行を含む文書は読み取り専用で開き、その行は先頭 64 KiB まで表示。表示・End・スクロールが p99 ≤ 100 ms**。v0.2: char 近似モードで表示・End・中央挿入が p99 ≤ 100 ms。50 ms と書記素単位の正確さは v0.3 (列チェックポイント) | §11.5 の縮退 (v0.1)、char 近似モードと ASCII 高速経路 (v0.2) | char ごとの表引きが数百 MB/s 出るか (09 BM-106 で実測) |
| NFR-107 保存 | 参考値 (fsync は OS とディスクで決まる)。Must は「**保存中も NFR-103 を維持する**」 | ストリーミングエンコード (チャンク直書き)、追加メモリなし。保存はジョブなので UI は止まらない | macOS の `F_FULLFSYNC` は遅い |
| NFR-108 / 109 検索 100 MB | **リテラル検索** に 50 ms / 300 ms (Should)。正規表現は「入力を止めない」のみ Must、所要時間は参考値 | カーソル位置から前方優先走査、`memmem` で 1 GB/s 級。結果はバッチ送信 (256 件 / 20 ms) | 行単位の正規表現は 100 MB の全件走査が 1〜2 s になり得る → 件数上限と打ち切り表示 |
| NFR-110 ハイライト 50 ms (v0.4) | — | 旧 Decoration の写像で即時表示、解析はジョブ、`budget` 20 ms で Partial | 注入言語の多い Markdown で解析が重い |
| NFR-111 10 MB ペースト | Windows の自前の入力ループ (`ReadConsoleInputW`) + バースト検出 (04 §6.6、S-07 で測る) で 1 s (Should。Unix のブラケットペーストは出荷する版)。入力スレッドで `Text` をまとめる処理 (04 §6.1) は Must | 1 Transaction、`Insert(Arc<str>)` で 1 回のコピー、`eol_stats` 走査 10 MB ~10 ms、改行正規化 (§8.4) は 1 パス | 貼り付け後のレイアウトはカーソル行のみ。Windows は自前ループ + バースト検出の性能 (S-07 で測る) 次第 |
| NFR-201 アイドル RSS | 10 MB (許容 20 MB)。指標を OS ごとに固定: Windows `PrivateUsage` (v0.1)、Linux `VmRSS`、macOS `phys_footprint` (出荷する版)。最初の Frame を描いてから 10 秒アイドル | §11.7 | — |
| NFR-202 開いた時のメモリ | **デコード後の UTF-8 サイズ** × 1.5 + 15 MB (読込完了から 1 秒アイドル後に計測)。妥当なテキストに適用し、不正バイトの多い文書は警告の対象 | §11.7 | ropey 1.x のノード倍率 (P0 実測)。musl 版はアロケータの差があるので別に記録 |
| NFR-203 Undo 上限 | 文書あたり 64 MiB / 10,000 ステップ (v0.1)。**プロセス全体 256 MiB は v0.2** (複数文書と同時。01 NFR-203、§7.5) | §7.5 の会計。`InsertText::Slice` は 64 KiB 以上のみで、実体化ジョブで範囲だけの rope に置き換える。小さな削除はコピー (会計に現れないメモリを作らない、§6.5) | 実体化ジョブが `bg` の検索の後ろで待つ間は旧 Snapshot が残る (時間的に有界) |
| NFR-204 アイドル CPU | CPU 使用率 0.1% 未満 (Windows)。起床 0 (Linux、出荷する版) | Undo 結合タイムアウト・スワップは deadline 方式。ジョブ完了後にタイマを残さない | — |
| NFR-306 長時間 | v0.1 は 1 時間の soak を夜間に回して傾向を見る (ゲートにしない)。8 時間の判定は v1.0 | 履歴上限 (全体 256 MiB)、メッセージ履歴 1,000 件・検索履歴 100 件・レイアウトキャッシュの上限 (§11.2)、検索結果上限、(v1.L〜) `pending_changes` のバイト予算と畳み込み | Snapshot 参照の循環がないこと (Arc のみ、Weak 不要) |

---

## 22. テスト観点

詳細な階層・ツール・CI は 09-testing-quality。本章は **本書の設計が要求する検査項目** を列挙する (NFR-701 / 702: text / バッファ / Undo は 90%)。

### 22.1 プロパティテスト (proptest)

| 対象 | 性質 | 参照モデル |
|---|---|---|
| TextStorage | 任意の編集列で `Text` と `String` (参照実装) の内容・`len_lines`・行境界・byte⇔char⇔utf16 変換が一致 | `String` + 素朴な走査 |
| ChangeSet | CS1〜CS6 (§6.2)。`compose` の結合律、`invert` の対合性、`map_pos` の単調性、`rebase` の収束 (v1.L): **`apply(apply(t, b), a.rebase(b, OtherFirst)) == apply(apply(t, a), b.rebase(a, SelfFirst))`** (非衝突時。同位置への同時挿入があるため、tie ポリシーは **相補的な組** で定義する。同じポリシー同士では成り立たない)。`map_range` の分類: 交差しない変更では `Mapped`、範囲内の変更・端に接する挿入では `Overlapped` で `kind` が正しい | — |
| Selection | `normalized` の冪等性、N1〜N6、`map` 後も INV-S1/S2 を満たす | — |
| 書記素 | 任意の Unicode 列で `snap_to_grapheme` が境界を返し、`next/prev_grapheme` が往復する。チャンク跨ぎ (1 バイト葉を強制する小さい rope) でも `&str` 全体の `unicode-segmentation` と同じ境界 | `unicode-segmentation` on `String` |
| 表示幅 | `grapheme_width` が 0..=48 (§13.2 の可視記号の上限)、可視化の対象 (§13.2) を含む書記素の幅が 0 にならない、結合文字で変わらない、`display_to_pos(pos_to_display(p)) == p` (書記素境界 p) | — |
| Undo | 任意の編集/Undo/Redo/保存/予算による破棄 (文書あたり。プロセス全体は v0.2) の列で (a) `Undo; Redo` が内容を戻す、(b) 破棄が無ければ全部 Undo すると初期内容、(c) INV-H1〜H4、(d) 予算超過時も直前 1 操作は Undo 可、(e) 保存点の判定: **dirty が参照モデルと常に一致** (参照モデル = 「現在の位置が保存した位置と同じ」を位置の列で追跡したもの)。特に、Undo で保存位置に戻れば dirty = false、保存位置が redo 側にある状態で新しい編集をすると以後 dirty = true、保存直後の入力は dirty = true (前のノードに結合しない)、Undo 後に保存 → 新しい編集 → Undo で dirty = false、底の破棄を挟んでも dirty が変わらない、(f) Undo / Redo の直後の入力は新しいノードになる、(g) Reload の Undo / Redo で `TextFormat` と読み取り専用の理由が本文と一緒に戻る | 編集列を直接再生した `String` と位置の列 |
| Undo のメモリ会計 (v0.1、L1。01 FR-203 AC-3) | ランダムな編集 (小さな削除を多く含み、64 KiB 以上の削除を一定割合で含む。1 万回を目安とし、Undo の上限 (64 MiB / 10,000 ステップ) による履歴の破棄が起きない範囲に収める) の後、メモリの増分 ≤ Undo 会計の合計 × 1.2。指標は Private Bytes (`PrivateUsage`。NFR-201 の Windows の指標) で、増分は「読込完了から 1 秒後」の値から「編集の後、実体化ジョブがすべて完了した時点」の値までの差 (測り方は 09 §3.2)。`InsertText::Slice` が 64 KiB 未満の削除に現れず、実体化の完了後に残らない | 会計値と実測の Private Bytes |
| 貼り付けの改行 (§8.4) | 任意の LF / CRLF / CR 混在の貼り付けテキストと任意の `default_eol` で、経路 (Paste / クリップボード / 内部レジスタ、frontend の前処理の有無) によらず挿入結果が同じで、挿入部分の改行がすべて `default_eol` になる | `normalize_eol` の素朴な実装 |
| 改行 | 任意の LF/CRLF/CR 混在列で `len_lines`、`line_terminator`、`eol_stats` が参照実装と一致、正規表現 (v0.3 の複数行) の `\n` が全区切りに一致 | 手書き走査 |
| 改行の合体・分離 (§2.4 / §6.5) | `\r` と `\n` を多く含む任意の文書に、端が `\r` / `\n` に接する挿入・削除・置換 (CRLF の間への挿入、`\n` で始まる挿入、`\r` で終わる挿入、`\r` と `\n` の間の削除、64 KiB 以上の削除を含む) を任意に適用した後、増分で更新した `eol_stats` が全文を数え直した値と一致し INV-D5 を満たす。Undo / Redo の後も一致する。編集コマンド経由 (`eol_guard`、§6.2) の挿入・削除では、改行を 1 つ挿入すると `len_lines` がちょうど 1 増え、行の中身だけを消しても `len_lines` が減らない | 全文の数え直し |
| 検索 (§12.1 / §12.4) | 任意のテキストとパターンで、(a) 一致の列がチャンク分割 (1 バイト葉を含む) に依存しない、(b) 一致は互いに重ならず左から順、(c) 両端が書記素境界、(d) `Insensitive` の 3 経路 (Sensitive 扱い・ASCII・正規表現) の結果が、単純ケースフォールディングによる素朴な照合と一致する (K U+212A・ſ U+017F・全角英字を含むテキストで検査) | `String` 全体での素朴な走査 |
| 文字コード (バイト → テキスト → バイト) | 任意のバイト列 (有効/無効混在) について、**`roundtrip` の範囲外は decode → encode が元のバイト列と一致** し (UTF-8 / UTF-16 / Shift_JIS / EUC-JP / ISO-2022-JP)、範囲内は v0.1 では `Summary.count` に数えられ、v0.3 では保持方式 (E2) で一致する。ISO-2022-JP は行単位 (§8.7 区分 D): 差のある行のうち表現不能の文字を含まない行の数 == `Summary.count`、正規形の行は一致。不正バイトのエスケープ数 == `invalid_bytes`。**チャンク分割 (1 バイト単位を含む。ISO-2022-JP ではエスケープシーケンスの途中で切る分割を含む) によって結果が変わらない** | 全体を 1 チャンクで decode した結果 |
| 保存エンコーダの決定性 (§9.3 の「乾式走査と出力長」、v0.1、L1) | 任意の Snapshot (チャンクの分割を任意に変えたものを含む) と format (UTF-8 / UTF-8+BOM / UTF-16LE/BE / Shift_JIS / EUC-JP / ISO-2022-JP) について、(a) `SaveEncoder::dry_scan` の `out_len` == `SaveEncoder::new` の出力の連結の長さ、(b) 表現不能の判定 (有無・位置・総数) が両者で一致、(c) `SaveEncoder::new` を 2 回作って得た出力が同じバイト列、(d) キャンセル確認の回数・チャンクの区切り方で結果が変わらない | 1 チャンクで encode した結果 |
| ISO-2022-JP の正規形と状態引継ぎ (§8.6 E6) | (a) 冪等性: `encode(decode(encode(t))) == encode(t)` (正規形は再エンコードで変わらない)、(b) チャンク分割不変: 1 行の内部で状態を引き継いで任意に分割して encode した連結 == 分割せずに (行ごとに閉じて) encode した結果 (1 文字単位の分割を含む)、(c) 終端: 出力は常に ASCII 状態で終わる (`has_pending_state() == false`。非 ASCII を含めば末尾は `ESC ( B`)、(d) 行末の状態: 各行末 (LF / CR の直前) が ASCII 状態 (`¥` / `‾` で終わる行を含む。OQ-162 で行ごとにエンコーダを閉じる方式に確定 (ユーザー回答))、(e) 行単位の区分 D 件数 == 「全体を比較したときに異なる行のうち、表現不能の文字 (半角カナ等) を含まない行」の数、(f) 半角カナ・`SO` / `SI` / `ESC` を 1 文字でも含む文字列は E1 の照合で 100% 検出される | 全体を 1 回で encode した結果 / 行ごとの比較 |
| 単語境界 (§13.3) | 境界は単調・書記素境界上・パニックなし。期待表の全件一致は L0 のゴールデン | 期待表 |
| 文字コード (テキスト → バイト → テキスト) | 当該文字コードで **表現可能な文字集合のみ** から生成した文字列について encode → decode が一致する。生成器は §8.6 の照合で表現不能と判定される文字 (¥ ‾ − 半角カナ等の黙示置換対象、JIS X 0212、互換文字の表の JIS 系の文字) を **除外** する。Shift_JIS の生成器は CP932 の外字 U+E000〜U+E757 を **含める** (自前の逆算で往復する、§8.7)。除外対象を含む文字列については「E1 の照合が 100% 検出する」ことを別の性質として検査する | — |
| 複数カーソル | N 個の Range への同時挿入/削除の結果が「各 Range を末尾から順に個別適用」した結果と一致 | 逐次適用 |
| キー入力列 (v0.1。fuzz `fuzz_editor` は v0.2) | 任意の `InputEvent` / `CommandInvocation` 列でヘッドレス `Editor` がパニックせず、各ステップの Frame が INV-F1 (制御文字の無害化) を満たす | — |
| 設定 (v0.1) | 任意の TOML 文字列 (構文エラー・型エラー・未知キーを含む) で読込がパニックせず、該当項目が既定値になり診断が出る (FR-703) | — |

### 22.2 fuzz (cargo-fuzz, NFR-305)

- v0.1 は **デコード** の 1 ターゲット (01 NFR-305、09 §3.3)。**キー入力列** と **設定** は v0.1 ではプロパティテスト (§22.1) で同じ性質 (パニックしない・Frame の不変条件) を検査し、fuzz のターゲットにするのはキー入力列が v0.2 (設定はプロパティテストのまま)。各ターゲット: **デコード** (`fuzz_decode`。任意バイト列 + 任意の文字コードラベル (ISO-2022-JP を含む) + 任意の sniff 長 + 任意のチャンク分割。判定 §8.2 の手順 4' を通し、ISO-2022-JP では状態を引き継いだ再エンコードと行単位比較がパニックせず、復号できない入力が不正バイトとして数えられる)、**キー入力列** (`fuzz_editor`、v0.2。任意の `InputEvent` / `CommandInvocation` 列でヘッドレス `Editor` がパニックしない。各ステップで Frame 生成まで行い、制御文字の無害化 (NFR-503) も検査する)、**設定** (07。プロパティテストで代える)。v0.2 以降の追加: ChangeSet の `apply` (任意の ops 列 → エラーかパニックなしの成功)、正規表現パターン (任意文字列 → コンパイル成功またはエラー、パニックなし)、`recovery/manifest.toml` とスワップ (破損ファイル)、レイアウト (任意テキスト × 任意 WidthConfig × 任意幅 → INV-F1/F2/F3)、単語境界 (任意 Unicode 列 → 境界が単調・書記素境界上・パニックなし)。

### 22.3 ヘッドレス統合テスト (eve-testkit)

- ジョブの実行: テストは `ManualJobSpawner` (02) を注入し、`run_jobs()` を呼んだ時点でだけジョブが実行される。確認・取消し・Revision の不一致 (結果が届く前に編集を挟む) の順序をテストが決める。
- メモリ FS に障害注入: 保存中の `rename` 失敗、`fsync` 失敗、ディスク満杯、`Busy` (Windows 想定) → 元ファイル無傷 (INV-IO1)、Document は dirty、エラーメッセージがキー付きで出る。置換途中の失敗 (`ReplacedMovedToBackup`) → 退避パスが示され、選択肢に「元の名前に戻す」が無い (v0.1、確定 (ユーザー回答))。親フォルダが無いパスへの保存 → `NotFound { ParentMissing }`、フォルダが作られない。in-place の書込途中の失敗 (`InPlaceFailed`) → バックアップのパスが示され、選択肢に再試行が無い。本書込のエンコーダが乾式走査と違う長さを返す (障害注入) → `InPlaceFailed`。
- 外部変更シナリオ: 保存直前に mtime 変更 → `Conflict` (v0.1)。`file.reload` (v0.1): 未編集なら確認なしに読み直し、dirty なら `ui.confirm.reload_discard` が出てキャンセルで何も変わらず、再読込は 1 回の Undo で本文・`TextFormat`・読み取り専用の理由ごと戻る。ファイルが消えていれば文書は変わらず `msg.reload.missing`。dirty でない Document の自動再読込で Selection が写像される (v0.2)。
- 改行コードの変換 (v0.1): LF / CRLF / CR の混在の文書を各 `eol` に変換すると全区切りがそれになり、`eol_stats` が 1 種だけになり、1 回の Undo で本文と `default_eol` が戻る。変換済みの文書では Undo の履歴が増えない。1 MiB 超の文書のジョブの結果を `run_jobs_stale()` で編集の後に届けると、作り直した結果が編集後の文書に適用される。
- すべて置換 (v0.1): 大小区別の 3 値で置換の対象が検索の一致と同じになり、1 回の Undo で全件が戻り、置換文字列の `$1` / `\n` は文字どおり挿入される。重なる候補 (`aa` を `aaa` で) と書記素境界でない候補 (`e` と `é`) は置換しない。読み取り専用の文書では置換欄が開かない。
- 空白の可視化 (v0.1): 全角スペースとタブを含む行の Frame で、記号 (`[]` / `>`) とロールが付き、セル位置・カーソル位置が可視化しない場合と一致する。
- 置き換え (v0.1、§9.2): dirty な文書で `file.open` → 保存 / 破棄 / キャンセルの確認。キャンセルで何も変わらない。読込の失敗・キャンセルで旧文書が残る。置き換えの途中のパニック注入で旧文書が退避される。
- 無編集の上書き保存: dirty でなくディスクも一致 → 書き込みが発生しない (`MemFs` の書込回数 0)。「名前を付けて保存」は書く。
- 巨大ファイルの読込キャンセル: 進捗表示中にキャンセル → 読込中の Document が閉じ (置き換えの途中なら旧文書が残る)、ジョブが停止する。進捗メッセージが 10 Hz を超えない。
- 文字コード往復: フィクスチャ (BOM 各種、CRLF/CR/LF 混在、末尾改行なし、不正バイト、CP932 重複符号 (0xEEF9 / 0xFA54)、CP932 互換文字 (0x8160 / 0x8161 / 0x817C / 0x8191 / 0x8192 / 0x81CA / 0x815C)、CP932 の外字 (0xF040 / 0xF07E / 0xF080 / 0xF0FC / 0xF140 / 0xF9FC)、JIS X 0212 を含む EUC-JP、ISO-2022-JP の正規形、先頭 64 KiB が ASCII で後半のみ ISO-2022-JP) を開いて 1 文字編集して元の文字コードで保存 → v0.1 は区分 B の文書 (重複符号・JIS X 0212・ISO-2022-JP のエスケープ選択) で確認ダイアログ (E2') が出て件数が一致し、承認後は区分 C の該当箇所のみ変化。互換文字と外字は区分 A なので E2' の件数に入らず、編集箇所以外のバイトが一致する (外字の文書で外字の隣を編集しても外字のバイトは変わらない)。v0.3 は無編集保存でバイト一致 (区分 A / B) と編集後の区分 C を検査。
- ISO-2022-JP (v0.1、09 §4 の日本語ゴールデン): (a) 正規形のファイル (`ESC $ B` / `ESC ( B`、各行末で ASCII、末尾 `ESC ( B`) → 区分 B 件数 0、1 文字編集して保存 → 再読込でテキスト一致、元と編集箇所以外のバイトが一致。(b) 旧 JIS `ESC $ @`、`ESC ( J` で書かれた ASCII、冗長なエスケープ、行末で ASCII に戻らない、末尾が ASCII 状態でない、各ファイル → 読込時に区分 B の行数が期待値と一致し情報メッセージが出る。無編集の上書き保存は書き込まない。編集後の保存は E2' の確認後に正規形で書かれ、再読込でテキスト一致。(c) `ESC ( I` の半角カナを含むファイル → 読込は半角カナとして復号され、半角カナを含む行は区分 D (`Summary.count`) に数えられない (§8.7)。編集して保存 → 表現不能 (半角カナ) として中止され、§8.6 の「E3 の選択肢」が出る。「置換して保存」を選ぶと置換の一覧 (半角カナ → 全角カナ (`ｶﾞ` → `カ゛`)、互換文字 → 互換文字の表の文字 (`〜` U+301C → `～` U+FF5E)、他の表現不能文字 → `?`。件数は全角カナ・互換文字・`?` の順) が示され、承認すると Transaction (`Origin::System`、Undo 可) の後に保存され、一覧でキャンセルすると保存されずカーソルが最初の該当位置へ移る。「UTF-8 で保存」を選ぶと保存の成功後に文字コードが UTF-8 になる。(d) `ESC $ ( D` (JIS X 0212)、8 bit バイト、`SO` / `SI`、ISO-2022-JP-2 のエスケープを含むファイル → 不正バイトとして可視化され読み取り専用、`file.toggle_read_only` (v0.2) で解除できない、FR-111 で別の文字コードとして開き直せる。(e) チャンク境界 (64 KiB / 1 MiB) の直前でエスケープシーケンスが切れるファイル → 1 チャンクで読んだ結果と一致。(f) ANSI エスケープ (`ESC [`) を含むログ → ISO-2022-JP と判定されない。(g) `¥` / `‾` を含む UTF-8 文書を ISO-2022-JP で保存 → `ESC ( J` で書かれ、再読込で一致 (表現不能にならない)。(h) `−` (U+2212) → E1 の照合で表現不能 (U+FF0D への黙示写像)。(i) `"¥\nabc"` を ISO-2022-JP で保存 → 出力バイト列が `ESC ( J 5C ESC ( B 0A 61 62 63` (改行の前に `ESC ( B`。OQ-162 で確定した行ごとにエンコーダを閉じる方式の期待値) と一致する。(j) JIS X 0208 状態のまま改行するファイル (`1B 24 42 24 22 0A 1B 28 42`) を ISO-2022-JP を指定して開く → 0x0A が `<0x0A>` として見え (行は分かれない)、`StatefulEncodingWithInvalidBytes` で読み取り専用。`file.save_as` (ISO-2022-JP のまま)・`file.set_encoding` (ISO-2022-JP) → 名前を尋ねずに `msg.save.encoding_unsupported_iso2022` が出て、ジョブが投入されない (`MemFs` の書込回数 0、metadata の問い合わせ 0)。`file.set_encoding` で UTF-8 / Shift_JIS を選ぶ → E3 で「UTF-8 で保存」「置換して保存」が出ずキャンセルだけ。
- FR-112: Shift_JIS で保存できない文字 (絵文字) を含む文書の保存 → 中止、位置が報告される、ファイル無傷。**互換文字の置換**: 互換文字の表の文字 (U+301C・U+2016・U+2212・U+00A2・U+00A3・U+00AC・U+00A6・U+2014・U+00A5・U+203E) と絵文字を含む UTF-8 文書を Shift_JIS で保存 → E3、「置換して保存」の一覧で `counts` が `kana = 0`・`compat = 10`・`question = 1` (絵文字 1 個のとき)、承認後の保存結果を Shift_JIS で再読込すると互換文字の表の置換後の文字と `?` になる。同じ文書を ISO-2022-JP で保存すると `¥` `‾` は対象外 (`compat = 8`)。**外字**: U+E000 / U+E757 を含む文書を Shift_JIS で保存 → 表現不能にならず 0xF040 / 0xF9FC で書かれる。EUC-JP で保存 → 表現不能で、置換先は `?`。**黙示置換の検出**: `¥` (U+00A5)、`‾` (U+203E)、`−` (U+2212) を含む文書を Shift_JIS で保存 → 表現不能として報告される (0x5C 等に化けない)。半角カナを含む文書を ISO-2022-JP で保存 → 表現不能として報告され、E3 の 3 つの選択肢が出る (E6)。直接上書き (承認後・`InPlacePattern`) の保存で表現不能があると、乾式走査で検出されて `MemFs` の元ファイルへの書込回数が 0。
- 不正バイト: 先頭 64 KiB の比率が 1% 超のファイル → 本文の構築の前に `ui.confirm.open.invalid_bytes` (巨大ファイルとは別のキー) が出る (中止なら何も開かない。「文字コードを指定して開く」で別の文字コードを選ぶと `forced_encoding` 付きで先頭から読み直す)。不正バイトの確認で「読み取り専用で開く」で開いた文書の理由は `InvalidBytes` だけで、`file.toggle_read_only` (v0.2) で解除できない (§9.6)。先頭 64 KiB は妥当で後半に不正バイトがあるファイルは確認なしで `InvalidBytes` の読み取り専用で開く。UTF-32 の BOM のファイル → `ConfirmAsk::UnsupportedBom` の確認が出て、「読み取り専用で開く」で開いた文書の理由は `Binary` (§9.2 の表。バイナリの確認と同じ) で、その再投入では不正バイトの比率の確認を出さない (`read_only_requested`)。「文字コードを指定して開く」で UTF-16LE を選ぶと `FF FE` を BOM として読み飛ばして開く (§8.2 手順 2)。
- 保存の連鎖 (§9.7): dirty な文書で `app.quit` → 保存 → 表現不能で E3 の「キャンセル」→ 終了しない・文書は dirty のまま。`Conflict` で「中止」→ 終了しない。保存の成功で終了する。無題の文書で保存 → 名前の入力をキャンセル → 終了しない。保存中に入力 → 保存の完了後に終了せず FR-107 の確認がもう一度出る。名前を付けて保存で既存のファイルを選ぶ → 上書きの確認 (既定は「別の名前を入力」)、`Esc` で名前の入力に戻り書かない。読み取り専用の文書で FR-107 の「保存」→ 名前の入力に進む。E3 の「置換して保存」→ 置換の ChangeSet の構築中に編集を挟む → 結果が捨てられて作り直される。構築中の `Esc` → E3 のキャンセルと同じ。`AtomicUnsupported` の確認の既定が「別名で保存」。保存中に `app.quit` (偽時計) → 5 秒以内に保存が終われば確認なしで終了し、5 秒を超えると `ui.confirm.quit_saving` が出る → 「待つ」(`Esc` も同じ) → 保存の失敗で `AfterFailure` に進み、再試行の成功で終了する。「保存を中止して終了」→ 取消しを送った後に `Saved` が届いても採用され、dirty なら退避してから終了する。後続を伴う `Saving` で `Esc` → 取消しが送られ、`Cancelled` で中止、`Saved` なら完了。単独の保存の `Saving` で `Esc` → 取り消されない。連鎖の途中の `file.save` → 無視され保存中が示される。「退避先に保存」を選ぶ → `WriteRecovery` で `recovery/rescue/` に書かれ、後続 (終了) は行われない。`state_dir` に書けない (障害注入) → 代替の場所に書かれる。読み取り専用属性のファイルを選ぶ → 上書きの確認の前に拒否。`file.set_encoding` の保存が失敗 → `TextFormat` が元のまま。開き直しのキャンセル・失敗 → 旧本文・旧 format が残る。
- 保存の重なり: 保存 A の実行中に保存 B を要求 → B は A の完了後に A の `new_stamp` を `Expect::Stamp` にして実行され、`Conflict` にならない。
- 置換途中の痕跡 (開く時の検出は v0.1): 保存中フラグが残った状態でそのパスを開く → §9.2 の「中断された保存の写像」の表の各行 (記録 × T の有無 × バックアップの実在) について、`interrupted_save` の値と MessageKey が表のとおりになり、報告しない行ではフラグのファイルだけが消える (一時ファイル・バックアップは残る)。文書は通常どおり開く。(v0.2) 起動時の一斉走査 → `.<name>.eve-bak-*` だけがある場合は復旧提案、`.eve-tmp-*` が残る場合は確認提案。
- 緊急退避の形式: CRLF / 混在改行・PUA エスケープを含む dirty 文書を退避 → `recovery/` の本文が改行を変換せず保持し、manifest の `default_eol` / `eol_mixed` / `encoding` が元と一致 (manifest のキーは 02 §11.2.1)。
- Undo 結合 (§7.3 の R1〜R9): 「abc」入力 → 1 Undo で全部消える。「ab」→ 1.5 s 待ち (偽時計) →「c」→ Undo で「ab」が残る。字下げの無い行で「ab」→ Enter →「c」→ Undo で「c」だけが消え、もう一度の Undo で「ab」と改行が消える。字下げのある行での Enter (改行 + 空白) は単独の 1 単位。改行後の入力は別単位。保存で封印。Undo 後に編集 → Redo 不可、保存点が redo 側にあれば dirty。
- 複数 View (v0.4): View A で削除 → View B の Selection / scroll が写像される。
- 検索: 編集中に古い検索結果が届く → v0.1 は破棄され、300 ms 後に再検索される (偽時計)。件数上限超で「N+」表示。長大行の正規表現検索 (v0.2) でメインスレッドが `String` を作らない。長大行を含む文書が読み取り専用で開き、表示範囲外の一致へは移動しない (v0.1)。
- Proposal (v1.L): base が古い Proposal の写像、衝突ハンクの失効、部分承認が 1 Undo 単位。
- 復旧 (v0.2): 編集 → スワップ書き出し (偽時計で 2 s 進める) → プロセス「クラッシュ」(状態破棄) → 再起動で復旧提案 → 適用で内容一致、dirty = true。
- 大きな選択範囲のコピー (01 FR-205 AC-4): 60 MB を全選択してコピー → `Effect::SetClipboard` が `ClipboardText::Handle` を運び、Host が `Editor::take_clipboard` で取り出した `ClipboardSource` をワーカーで文字列化する (メインスレッドで文字列化が起きない。取り出した後のハンドルは無効)。100 MB を全選択してコピー → 上限 (64 MiB) を文字列化の前に総バイト数で判定し、`Effect::SetClipboard` を出さずに内部レジスタだけに入れて通知する (文字列化が起きない)。

### 22.4 ベンチマーク (criterion、09 で CI 化)

- NFR-103〜111 を §21 の手段ごとに分解したマイクロベンチ: `apply` (100 MB 中央)、`layout_line` (10 MB 行の先頭 64 KiB (v0.1) / char 近似モード (v0.2) / 通常)、`snap_to_grapheme` (チャンク跨ぎ)、decode (UTF-8 / Shift_JIS 100 MB)、encode + 書き出し、検索 全件 (100 MB、リテラル `memmem` / 行単位の正規表現 `(?i)`)、ChangeSet compose (10,000 edits)、Undo 10,000 回。
- メモリ: 1 MB / 100 MB / 1 GB 読込後の RSS (NFR-202、デコード後サイズ基準、OS 別指標)、Undo 10,000 ステップ後の履歴サイズと RSS (NFR-203、会計との比)。

### 22.5 スナップショットテスト (insta)

- 代表的なテキスト (全角・絵文字・タブ・制御文字・不正バイト・長大行の先頭・折り返し) の `Frame` をシリアライズして比較。WidthConfig の `ambiguous_wide` 両方を網羅。

---

## 23. 未決事項

本書に関わる OQ / ADR。**番号は [10 §2](10-decisions.md#2-未決事項-open-questions) の統一 OQ 番号**。状態・区分 (PO 決定 / 裁量 / スパイク) と詳細は 10 を正とし、本表は本書での結論 (確定していないものは「未決」と記し、推奨案) を示す。

| OQ | 件名 | 結論 | 区分 / 影響 |
|---|---|---|---|
| OQ-32 | 「桁」の単位 (FR-102 / FR-503) | **書記素数 (1 始まり) で確定 (ユーザー回答)**。設定で表示列に切替 (キーの公開は v0.2、07 §5) | 確定 / 03 §3.2 C4, 04, 07 |
| OQ-45 | Backspace の単位 | **v0.1 は書記素単位に固定** (FR-201、§13.1)。コードポイント単位への切替は v0.2 で設定 `editor.backspace_unit` (07 §5.13、既定は書記素) として公開する | 確定 / 01 FR-201, 03 §13.1, 07 §5.13 |
| OQ-46 | 単語移動の停止位置 | 既定は **次の単語の先頭** (Windows 慣習)。現在の単語の末尾 (macOS/Unix 慣習) は v0.2 で設定として公開 (`editor.word_motion`、07 §5.13) | 確定 (ユーザー回答) / 03 §5.3, 07 §5.13 |
| OQ-47 | 読込時の文字コード往復照合 (§8.7) | **UTF-8 以外で常に行う (ストリーミング、サイズ制限なし)**。v0.1 は件数 + 先頭 100 箇所の記録で保存時の確認 (E2') に使い、v0.3 で保持方式の素材にする | 確定 (ユーザー回答) / 読込時間 (08 §3 S-01 で実測) |
| OQ-48 | スワップファイルの形式 | 確定 (ユーザー回答、2026-10-28): 緊急退避と同じ (UTF-8 本文 + 版付き manifest。swap-lite、§10.2)。v0.1 で確定する manifest 形式 (02 §11.2.1) を共用し、書き手も再利用 | 裁量 / 03 §10.2, 05 §7.5 (配置・命名は 05) |
| OQ-49 | Undo 結合の時間閾値 (`editor.undo.group_timeout_ms`) | **1000 ms** (07 §5.1 の既定値) | 確定 (ユーザー回答) / 03 §7.3, 07 §5.1 |
| OQ-50 | Undo 結合の単語境界での封印 | 既定 **OFF** (連続入力をまとめる)。設定公開は v0.2 (`editor.undo.break_on_word`、07 §5.13) | 確定 (ユーザー回答) / 03 §7.3 |
| OQ-51 | 貼り付け時の改行正規化 (§8.4) | **`default_eol` に正規化、で確定 (ユーザー回答)**。v0.1 は常に ON、無効化の設定キー `editor.paste_normalize_eol` の公開は v0.2 (07 §5.13) | 確定 (ユーザー回答) / 03 §8.4, 04 §6.6, 05 §8.2 |
| OQ-52 | 混在改行の Mixed 表示方法と「揃える」提案の有無 | **表示のみ** (揃える提案はしない) | 確定 (ユーザー回答) / 04 |
| OQ-05 | 巨大ファイルの警告閾値と利用可能メモリの取得 | **512 MiB 超で開く前に警告する固定閾値、で確定 (ユーザー回答)** (`file.huge_file_warn_bytes`、§11.2)。利用可能メモリへの連動は v0.4 以降 (02 §4.4 に `SystemInfo` を予約) | 確定 / 02 §4.4, 05, 07 |
| OQ-53 | 書記素境界判定のチャンク跨ぎ方式 | 未決 (08 §3 S-02 で判断)。推奨: `GraphemeCursor` + PreContext、不可なら 256 バイト窓バッファ。国旗列の偶奇はどちらの方式でも既知の制約 | スパイク / 03 §13.1 |
| OQ-54 | 正規化 (NFC/NFD) 非依存の検索 | 未決。推奨: **v0.x では非対応**、要望があれば Could | 裁量 / 03 §13.5 |
| OQ-39 | BiDi を 00 の非目標に追加するか | **追加 (NG11)** | 確定 (ユーザー回答) / 00 §3 |
| OQ-55 | 検索結果上限値 | **保持 10,000、カウント 1,000,000** (07 `search.count_limit` の既定値) | 確定 (ユーザー回答) / 03 §12.2, 07 §5.5 |
| OQ-56 | 複数カーソルでの貼り付け分配規則 | 未決。推奨: **行数一致なら分配** | 裁量 / 04 (v0.4) |
| OQ-57 | 折り返しの禁則処理の既定 | 未決。推奨: **ON** (最小集合) | 裁量 / 03 §14.3 |
| OQ-58 | マウスクリックで全角セルの右半分を押したときの位置 | 未決。推奨: **文字の前** (`Bias::Before`、§5.8) | 裁量 / 04 |
| OQ-59 | `Origin::Undo` を独立した Origin にするか、元 Origin + フラグか | **元 Origin + `is_undo` フラグ** (監査で元の主体が分かる。02 §5.2 で確定) | 確定 (ユーザー回答) / 03 §7.1, 02 §5.2 |
| OQ-60 | 複数行の正規表現を rope 上でどう実行するか (v0.3) | **v0.2 は行単位の実行で複数行は未対応** (§12.3。v0.1 は正規表現を利用者に出さない)。v0.3 の実行方式は未決 (判定基準: キャプチャ・CRLF モード・最新 regex との互換・性能比 2 倍以内)。S-04 は v0.1 の着手ゲートに含めない | スパイク (S-04、v0.3) / 03 §12.3 / v0.3 着手前 |
| OQ-41 | CP932 互換文字 (〜/～、−/－、‖/∥、¢/￠ 等) を検索で同一視するか | 未決。推奨: 同一視オプション `search.cp932_compat_fold`、既定 ON (**v0.3**)。保存データは変えない。対象表は S-01 (c) の実測後に確定 | PO / 03 §8.7 / §12.4, 07 §5.5, 09 §4 |
| OQ-42 | v0.1 のデータ保全の穴 (SIGKILL / OOM / 電源断 / CTRL_CLOSE 不達) の扱い | **リスク受容を明記し、FR-118 (swap-lite) を v0.2 の最優先にする、で確定 (ユーザー回答)**。固定閾値警告と不正バイト比率の警告 (§11.2)、`try_reserve` を Must の前提とし、リリースノートに既知の制約として明記する | 確定 / 01 FR-118, 10, 03 §10 |
| **OQ-139** | 状態付き符号化 (ISO-2022-JP) で不正バイトを含む文書を常に読み取り専用にするか | 未決。推奨: **読み取り専用で固定** (FR-111 で UTF-8 として開き直せば編集可。02 §13.4 (f) と同じ)。v0.1 は不正バイトを含む文書を文字コードによらず読み取り専用にするため、v0.3 (NFR-304 の編集) まで差は出ない | 裁量 / 03 §8.3, 02 §13.4, OQ-16 |
| OQ-31 | NFR-106 (長大行) の優先度 | **Must のまま、判定を「閾値超は char 近似モード、10 MB 行で各操作 p99 ≤ 100 ms」とする、で確定 (ユーザー回答)** (§11.5)。判定値は char 近似モードを入れる v0.2 から適用し、v0.1 は閾値超の行を含む文書を読み取り専用で表示する縮退で固まらないことだけを満たす (確定 (ユーザー回答))。50 ms と書記素単位の正確さは v0.3 の列チェックポイントで | 確定 / 01 NFR-106, 03 §11.5 / §14.4 |
| **OQ-116** | 無編集の上書き保存を書き込まないか (§9.3 手順 0) | **書き込まない、で確定 (ユーザー回答)** (dirty でなくディスクも `disk_stamp` と一致するとき。「変更はありません」と表示)。無編集保存でレガシー文字コードのバイト列が変わる事故を 1 つの条件で防げる。ISO-2022-JP のエスケープ選択 (§8.7 区分 D) も同じ条件で守られる | 確定 / 03 §9.3, 01 FR-104 / NFR-303 |
| **OQ-114** | Undo の内部構造 (§7.1) | **線形 2 スタック、で確定 (ユーザー回答)**。履歴は永続化せず置換は crate 内で閉じる。木は FR-213 で | 確定 / 03 §7.1 |
| **OQ-03** | v0.1 の日本語レガシー文字コードの範囲 | **Shift_JIS / EUC-JP / ISO-2022-JP をすべて読み書き、で確定 (ユーザー回答)**。ISO-2022-JP の保存は §8.6 E6 (行ごとにエンコーダを閉じて正規形で書く。OQ-162 で確定)、エスケープ選択の差は区分 B として行単位で検出し確認 (§8.7 区分 D) | 確定 / 03 §8, 01 FR-109 / NFR-303, 02 §13.4, 08 S-01 (a') |
| **OQ-138** | ISO-2022-JP で WHATWG (encoding_rs) のデコーダが受け付けない入力 (JIS X 0212 `ESC $ ( D`、ISO-2022-JP-2 系、`SO` / `SI`、出力を挟まない連続エスケープ、8 bit バイト) の扱い (§8.2 手順 4' / §8.3 / §8.7) | **v0.1 は不正バイトとして読み取り専用で開く** (FR-111 で UTF-8 等として開き直す。encoding_rs だけで閉じ、`eve-encoding` の依存と自前の表を増やさない)。v0.2 以降は未決: S-01 (a') で実コーパス中の「読み取り専用になるファイルの比率」を記録し、有意 (目安 5% 超) ならデコーダのみ自前の寛容実装 (JIS X 0208 / 0212 / 半角カナの表は encoding_rs の EUC-JP デコーダへ `バイト \| 0x80` で写して流用) を追加し、受け付けた箇所を区分 B に数える | 裁量 (S-01 の結果で判断) / v0.2 着手前 / 03 §8.2 / §8.3 / §8.7, 08 S-01 |
| **OQ-162** | ISO-2022-JP の保存で各行を ASCII 状態で終える方式と、S-01 不合格時の扱い (§8.6 E6) | **行ごとにエンコーダを `last = true` で閉じてから改行を付ける、で確定 (ユーザー回答)** (encoding_rs の公開 API だけで実装でき、E1 の照合と両立する)。S-01 不合格時: 照合が成り立たない文字コードは v0.1 では読込のみ (別の文字コードでの保存は可、リリースは止めない)、Shift_JIS / EUC-JP が不成立ならリリースを止める | 確定 / 03 §8.6 / §8.7, 01 FR-109, 02 §13.4, 08 S-01 (a') |
| **ADR-0031** (安定性の簡略化) | 設計者の裁量の項目 (ADR-0031 に収録): `InsertText::Slice` の閾値 64 KiB (§6.5)、プロセス全体の Undo 上限 256 MiB (§7.5)、固定 2 スレッド io / bg (§17.3)、正規表現の行単位実行 (§12.3)、単語境界の文字種遷移規則 (§13.3)、`LossySpan` の件数上限 100 万 (§8.7)、クリップボードの Snapshot 受け渡し (§12.6) | **本書の記述どおり** (ADR-0031 は確定 (ユーザー回答)。細目は本書が正本) | 確定 / 03, 05, 07 |
| **OQ-110** | ソフトラップの時期 (§14.3) | **v0.2、で確定 (ユーザー回答)**。レイアウト・上下移動・禁則処理がまとめて入るため v0.2 とする。型と設定キーは v0.1 で予約 | 確定 / 01 FR-506, 03 §14.3, 04 §10 |
