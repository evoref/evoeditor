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
> **版の表記**: **v0.1 (最小核) / v0.2 (日常の不足を埋める) / v0.3 (整理・効率化・品質) / v0.4 (日本語と巨大ファイルの完成) / v0.5〜v1.X** を使う (版と要件の対応の正本は 01 / 08)。後の版に回した機能も **型・Port・ID などの拡張点だけは v0.1 で残す** ([§1.4](#14-版ごとの範囲と先行投資))。core は frontend を知らない境界 (02 §8.4) の内側に置き、v0.1 の検証はヘッドレス / テスト用 Frontend で行う。core に frontend 固有の分岐を持ち込まず、core の機能はすべて TUI から使える。

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
| **性能** | NFR-103 (入力 2 ms) / NFR-105 (100 MB で 1 ms) / NFR-106 (10 MB 行) を満たすか | rope、可視範囲限定のレイアウト、長大行の読み取り専用での縮退表示 (v0.1、§11.5)・char 近似モード (v0.2) と書記素の単位の列の索引 (v0.4、§14.4)、全走査はジョブ |
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
| 不正バイトを含む文書 (§8.3) | 読み取り専用で開き、可視化 | 編集と保存は v0.4 | PUA エスケープの型・チャンク境界の回収 (表示に必要) |
| 復号専用文字の保持方式 (§8.6 E2 / §8.7 区分 B) | 検出して保存時に確認ダイアログ | 保持方式 (バイト一致) は v0.4 | `TextFormat.roundtrip` (v0.1〜v0.3 は `RoundtripSummary`: 行の数と先頭 100 行の位置だけを記録)。v0.4 は記録の表 `PreservedMap` を `Document` に持ち、`TextFormat.roundtrip` は要約 (件数と状態) になる (§8.7.1) |
| ISO-2022-JP (§8) | **読み書き** (OQ-03 で確定 (ユーザー回答)。02 §13.4)。保存は行ごとにエンコーダを閉じて各行を ASCII 状態で終え、1 行の内部ではチャンクの境界をまたいで状態を引き継ぐ (OQ-162 で確定 (ユーザー回答)、§8.6 E6)。エスケープ選択の差は区分 D として検出・確認 (§8.7) | 元のエスケープ配置を行単位で保持する保持方式は v0.4 (§8.7.1)。WHATWG デコーダが受け付けない入力 (JIS X 0212 `ESC $ ( D`、冗長なエスケープ等) の寛容な読込は OQ-138 | `Encoding` の列挙と `Encoding::is_stateful()`、`PreserveReason::StatefulEscape` (v0.4、§8.7.1) |
| Partial (先頭部分の先行表示、§9.2 / §11.3) | なし (進捗表示とキャンセルのみ) | v0.4 の M25 (最小の形: 読込が 200 ms を過ぎても終わらないときだけ、固定の先頭 1 MiB を読み取り専用で出す。§9.2.1) | `load_state` の値の名前 `Partial` だけを決めてある (v0.3 までのコードの列挙には無い。v0.4 で足す。eve-view の `LoadStateView` は `#[non_exhaustive]` なので、追加は互換) |
| 中断された保存の検出 (§9.2 / §10.3) | 開くたびにそのパスの保存中フラグだけを `inflight_for` で調べて場所を案内する (ディレクトリの孤児ファイルは列挙しない)。置換途中の失敗はその場で退避パス付きで報告 | 起動時の一斉走査 (`list_inflight`) と復旧の操作は v0.2 | `list_inflight` の Port 操作 (02 §4.5) |
| フォーカス復帰時の外部変更確認 (§9.5) | なし (保存ジョブ内の照合と、明示の再読込 `file.reload` のみ) | v0.2 (明示コマンド `file.check_external_changes` も)。監視は v0.5 | `JobKind::Metadata` の目的 `MetaPurpose::ExternalCheck` (02 §9.3) |
| スワップ (§10.2) | なし (リスク受容、OQ-42) | v0.2 最優先 (swap-lite) | 緊急退避の manifest を版付きで確定し、スワップと共用 |
| 置換 (§12.5) | リテラルのすべて置換 (検索バーの置換欄、`search.replace_open` / `search.replace_all`) | 1 件ずつの置換・正規表現の置換 (`$1` の展開)・選択範囲内の置換は v0.2 | `search.replace` の CommandId、`SearchQuery.scope` |
| 空白の可視化 (§13.2) | 全角スペースとタブだけ (定数で常に表示) | 切替の設定キーは v0.2、半角の空白・行末・改行は v0.5 (FR-508) | `view.show_whitespace` の名前と `DecorationKind::Whitespace` |
| 正規表現の検索 (§12.3) / 単語単位検索 (§12.4) / 複数行の正規表現 | なし (リテラルと大文字小文字の区別のみ) | 行単位の正規表現と単語単位は v0.2、複数行は v0.4 | `SearchQuery.kind` の `Regex`・`SearchQuery.whole_word`・`SearchBarModel.regex`、`search.toggle_regex` の CommandId、正規表現の実行経路を `TextStorage` の公開 API から隔離 |
| ソフトラップ (§14.3) | なし (水平スクロールのみ) | v0.2 | `View.wrap`、`RowSource` の折り返し番号、`view.wrap` 設定キー |
| 長大行 (§11.5) | 閾値超の行を含む文書は **読み取り専用** で開き、その行は先頭 64 KiB まで表示 | 編集 (char 近似モード) は v0.2 (M9d で実装、§11.5.1)、書記素の単位の列の索引 (遅延と差分。§14.4) は v0.4 の M25。長大行の折り返しは M25 の計測で送り、v0.5 の計画で v0.6 以降に送った (§14.4、08 §7) | 読み取り専用の理由 `LongLine`、閾値の名前 `file.long_line_threshold` (v0.1 は定数)、列の索引の差し替え点 (v0.2 から `layout::long` の `ColIndex`、§11.5.1。v0.4 はこれを書記素の単位に広げる。v0.3 までの §14.4 の `LineLayoutCache` の案は採らなかった) |
| 保存の OS 別手順 (§9.3) | Windows (`ReplaceFileW`) | Unix (rename + ディレクトリ fsync、モード・所有者) は macOS / Linux を出荷する版 | Port の trait (02 §4.5) と OS 別の規則 (05) は Unix の分も仕様として残す |
| `pending_changes` (§17.1) | なし (Revision 不一致の結果は捨てる) | LSP / AI の着手時 (v1.L) | `Document.pending_changes` のフィールドと 02 §5.3 の規則 |
| `DecorationProvider` trait (§14.5) | なし (選択・検索一致は Frame 生成で直接合成) | tree-sitter (v0.6) と同時 | `Decoration` / `DecorationKind` の型 |
| Proposal の演算 (§18) | `ChangeSet::map_pos` / `map_range` / `from_sorted_edits` のみ (選択・検索結果の写像に使う) | `ChangeSet::rebase` と `RebasePolicy` (収束性のプロパティテストを含む) は v1.L (最初の利用者 = Proposal の写像) | 02 §10.2.1 の型 (eve-view) と本章の演算の名前 |
| `Origin::Ai` / `Origin::External` (§6.3) | 列挙に予約 | v1.L / 駆動 API (v0.2 以降、dev feature) | 列挙と `Transaction` の拡張メタデータ欄 |
| `EditorEvent::DocumentChanged` の `EditSummary` (§6.5 手順 10) | **型の予約のみ** (v0.1 の `DocumentChanged` は `edits` を持たない。後から `Option` で追加するのは C2 の「追加」に当たる) | 生成は最初の利用者 (LSP 増分同期、ローカル LLM のキャッシュ境界・補完破棄、v1.L) と同時 | `EditSummary::{Exact, Coarse, Replaced}` と `EditDelta` の名前 (eve-view) |
| `SyntaxProvider` trait (§16.3) | 名前の予約のみ (v0.1 のハイライトは無し) | `detect_language` は v0.5 (言語別の設定と同時)、完全な署名は v0.6 (tree-sitter と同時) | trait 名と `LanguageId` |
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
| バージョン | OQ-06 の規則に従う。**要確認**: 2026-10 時点の最新安定版は 1.6.1 で、2.x は `2.0.0-beta.1` (2025-08) のまま安定版ではない。OQ-06 の規則どおりなら **P0 時点では 1.x** になる見込み。1.x の場合 `byte_to_char` / `char_to_byte` の往復コストを `TextStorage` 内に閉じ込め、2.x (バイト中心) なら変換を恒等にする。公開 API は変えない。P0 スパイク S-02 で 1.6 / 2.0-beta / crop の API (行種別の指定方法、バイト中心 API) を比較し、`TextStorage` 抽象がどれにも合うことを確認する。S-02 の確認項目には「`RopeSlice` は元の rope を借用し、部分木だけを所有する独立の値にできない」(§6.5 の `InsertText::Slice` と実体化ジョブの前提) も含める。2.x / crop で部分木を安価に所有できるなら、実体化ジョブを省ける。確認の結果 (2026-11、SAF-33): ropey 1.6.1 の `Rope::from(RopeSlice)` は部分木を Arc で共有し、端の経路だけを複製する。2.0.0-beta.1 は全文を複写する。版に依らない形は `clone()` と両端の `remove` (COW)。**2.x に移る条件** に、部分木を安く共有できること (範囲の rope を全文の複写なしに作れること。§6.5 の v0.4 の方針の前提) を加える |
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
    fn pos_to_line_grapheme(&self, p: Pos) -> LineGrapheme;       // O(log n + 行頭からの距離)。長大行は §14.4 の列の索引の記録点で O(log n + 4 KiB)
    fn line_grapheme_to_pos(&self, lg: LineGrapheme) -> Pos;      // 同上。col が行の書記素数を超えたら行末
    fn pos_to_display(&self, p: Pos, cfg: &WidthConfig) -> LineDisplay;   // タブ幅・曖昧幅設定に依存
    fn display_to_pos(&self, ld: LineDisplay, cfg: &WidthConfig, bias: Bias) -> Pos; // セルの途中 (全角の右半分) は bias で前後に寄せる
    fn pos_to_utf16(&self, p: Pos) -> LineUtf16;                  // LSP (v1.L)
}
```

| 規則 | 内容 |
|---|---|
| C1 | 行頭からの距離に比例する計算 (書記素・表示列) は **行内の走査** を伴う。通常行 (数百バイト) では無視できない長さにならない。長大行 (NFR-106) では、v0.1 は閾値 (1 MiB) 超の行を含む文書を読み取り専用で開き、その行は先頭 64 KiB までしか走査しない ([§11.5](#115-長大行-nfr-106))。v0.2 で閾値超の行を **char 近似モード** で扱い、ASCII の連続には高速経路を入れて走査コストを下げる (10 MB 行で各操作 p99 ≤ 100 ms)。v0.4 (M25) で [§14.4](#144-長大行の列の索引) の書記素の単位の列の索引 (4 KiB ごとの記録点。要る位置まで遅延して作り、編集では変更の手前を残す) により位置からの走査を 4 KiB に抑え、書記素単位の正確さを取り戻す |
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
pub struct Selection { ranges: SmallVec<[Range; 1]>, primary: usize }   // 非空。v0.1〜v0.4 は len() == 1 に制限。v0.5 の M32 (FR-211) から 1..=MAX_CURSORS (10,000。§4.5.1 の MCUR-6。`eve_text::MAX_CURSORS`。上限を超える Range の並びからは作れない (`Selection::new` が None))
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
| N4 | 空の Range の位置が非空 Range の `[start, end]` (**両端を含む**) にある場合、空の方を捨てる (非空を優先)。選択の端に重なるカーソルは見た目に冗長なため。ただし空の Range が primary のときは、非空の Range に併合し (その Range が primary になる。primary は捨てない、N5)、位置が端 (`start` か `end`) ならカーソルの側を head にする (`end` なら前向き、`start` なら後ろ向き。内側なら向きを変えない)。主の空のカーソルを選択の端に残すと、続けての入力・改行が選択の置き換えとカーソルへの挿入の 2 つの編集になり (`abcdef` の 2..5 の選択と 5 の主のカーソルで改行すると `ab⏎⏎f`)、見た目 (1 つの選択の端のカーソル) と結果が食い違うため (M32 の段 3 で、端なら残すとしていた規則を改めた) |
| N5 | ranges が空になることはない (最後の 1 つは削除できない。削除要求は primary を残す) |
| N6 | すべての `Pos <= len_bytes()` |

### 4.5 複数カーソルでの編集セマンティクス (FR-211)

v0.1〜v0.4 では `len() == 1` に制限するが、**編集コマンドはすべて複数 Range を前提に書く** (単一の場合は長さ 1 の特殊例)。**v0.5 の M32 で制限を外す** (上限は §4.5.1 の MCUR-6)。下の 1〜7 は v0.1 からの原則で、M32 の仕様の PR (2026-11-22) で決めた細目 (足す・減らす操作、コマンドごとの扱い、クリップボード (OQ-56 の細目)、ほかの機能との関係、表示) は §4.5.1〜§4.5.5、試験・実装の順・決めた点は §4.7。受入基準は 01 §5.5 の FR-211、コマンドと既定キーは 07 §11.6、描画は 04 §9。

1. **編集は 1 Transaction**: 1 回のコマンド実行で、全 Range に対する変更を 1 つの `ChangeSet` に合成し、1 つの `Transaction` として適用する。Undo は全カーソル分がまとめて戻る。
2. **各 Range は適用前の文書座標で変更を記述**する。コマンドハンドラは `Selection` を走査して `(Range, 局所 ChangeSet)` のリストを作り、`ChangeSet::from_sorted_edits` で位置が重ならないことを検証しつつ合成する。重なる場合 (例: 2 つの Range が同じ単語を削除) は **先行 Range を優先し、後続は重複部分を切り詰める**。切り詰めは、後続の削除の始まりを、それまでの (先行の) 削除の末尾まで後ろへずらす (後続の挿入は、ずらした始まりに入る。削除も挿入も空になった編集 (と空の片の貼り付け) は文書を変えないが、挿入の直後にカーソルを置くコマンドでは、その Range のカーソルを先行の編集の後ろ (文書の順のまま) に置くために残す。どの Range も文書を変えなければ何もしない。M32 の段 2 で決めた)。切り詰めは **呼び出し側 (コマンドハンドラ) が `from_sorted_edits` に渡す前に行う**。`from_sorted_edits` 自身は切り詰めず、重なりを受け取ったら `Err(Overlap)` を返す (ハンドラの誤りなので debug ビルドでは assert、リリースではコマンドを何もせずに終えてログする)。
3. **自分の挿入は自分の後ろへ**: Range が挿入した文字列に対して、その Range の head/anchor は挿入の **後** (`Bias::After`) に写像する。他の Range が同じ位置にある場合は **前** (`Bias::Before`) に留まる。これにより 2 つのカーソルが同一位置で入力した場合に文字が交互に並ばない (N3 により同位置の空カーソルは事前に 1 つに併合されているため、実際には発生しない)。
4. **コマンドの結果 Selection** は「各 Range が自分の変更をどう追うか」をハンドラが明示する (`selection_after`)。既定は規則 3 の写像。
5. **行単位コマンド** (行削除・行複製・インデント・コメントトグル) は「各 Range が触れる行の集合」を取り、同じ行を複数 Range が触れても **1 回だけ** 処理する。
6. **クリップボード**: 複数 Range のコピーは Range ごとの文字列を改行で連結し、同数のカーソルへの貼り付けは 1 対 1 に分配する (エディタ慣習)。連結の改行・分配の 3 段 (OQ-56 の細目) は §4.5.3。
7. **矩形選択** (FR-212, Could) は「各行に 1 つの Range を持つ Selection + 矩形モードフラグ (表示列で揃える)」として表現し、新しい選択型は作らない (§4.5.4。v0.5 の M34 の規則は §4.8)。

#### 4.5.1 カーソルを足す・減らす操作 (v0.5 の M32)

| ID | 規則 |
|---|---|
| MCUR-1 | **主のカーソル** (`primary`): ステータスバーの位置・カーソルが見えるようにするスクロール・上下の移動の目標の列 (§5.2)・ハードウェアカーソルと IME の候補窓 (`Frame.primary_cursor`)・検索の起点・対応括弧の強調 (§5.7) は、主のカーソルについて決める。**足す操作 (MCUR-2〜MCUR-4) で足した Range を主のカーソルにする** (画面と状態の表示が、いま足した位置を追う)。`normalized()` の併合 (§4.4 N3 / N4) で主の Range が併合されたときは、併合した後の Range が主になる (今の規則のまま) |
| MCUR-2 | **`select.add_next_match`** (次の一致を選択に足す): (a) **全 Range が空** なら、各カーソルの位置の単語 (§13.3 の境界で区切った同じ文字種の並び。カーソルの直後の文字の単語を優先し、無ければ直前の文字の単語。空白・改行の上は単語が無い) をそれぞれ選択にする (Range は足さない)。主のカーソルの位置に単語が無ければ、何もせずに `info.select.no_word` を出す。このとき View に「単語の一致」の印を立てる。(b) **主の Range が空でない** なら、主の Range の文字列 (検索語) の次の一致を探し、新しい Range として足して主にする (MCUR-1)。探す起点は主の Range の `end` で、文書の末尾まで行ったら先頭から続け、主の Range の `start` で止める。既にある Range と重なる一致は飛ばす。照合は **リテラル・大小を区別** し、「単語の一致」の印があれば **単語単位** (§12.4 の単語単位と同じ境界)。一致が無ければ (足せる一致が無い・全部選んである) 何も足さずに `info.select.no_more_matches` を出す。(c) **主の Range が空で、ほかに空でない Range がある** ときは、主のカーソルの位置の単語を選ぶ (a の主だけ)。(d) 探し方は `search.next` と同じ (§12.2): 同期検索 (最大 256 KiB か 2 ms) で見つからなければ単発のジョブ (`FindNext`) に回し、結果が届いた時点で Revision と選択が投入時のままなら足す (違えば捨てる)。単発のジョブの枠 (02 §5.1 の `find_job`) は `search.next` / `search.prev` と共有し、新しい要求が古い要求を取り消す (どちらの要求かは `Pending` で分ける。§12.2 の単発と同じく、検索バーの全件の走査は取り消して後ろに投げ直す)。日本語の単語は §13.3 の文字種の境界で決まるので、漢字の連なりの `設定` から足すと、`設定ファイル` の `設定` には当たるが `再設定` の `設定` には当たらない (ヘルプに例として書く)。(e) 「単語の一致」の印は、選択を変えるほかの操作 (移動・クリック・編集・Undo など) で消える。足す操作 (MCUR-2〜MCUR-4) と `ui.cancel` では残す。**細目 (M32 の段 3)**: (f) 「既にある Range と重なる一致」は、**空でない** Range と重なるか **接する** 一致 (足すと N3 で併合され、主の Range の文字列 (検索語) の範囲が変わる。`aaa` の 1 文字目の `a` から足すと、2 文字目は飛ばして 3 文字目を足す)。空のカーソルを含む一致は足し、そのカーソルは N4 で捨てる。(g) 照合は CP932 互換文字の同一視 (`search.cp932_compat_fold`、§12.4.1) もかけない (リテラル)。(h) 単語は §13.3 の境界で求め、カーソルの前後 256 バイトから読み始めて、単語が読んだ窓の端に届いたら窓を倍にし、64 KiB で止める (長大行で行の全体を読まない。それより長い単語は窓の中の部分を選ぶ。単語移動の窓 (§11.5.1) と同じ大きさ)。窓が行の途中で始まるときは、窓の最初の書記素を信用しない (書記素の途中で切れていることがあり、直前の文字種に従う `ー`・`々` などを直前なしで分類している) ので、単語がその書記素の後ろまで届いていれば窓を広げる。(a) で同じ単語の中にある続くカーソルは、前のカーソルの単語を使う。(i) 同期検索の予算 (`search.next` と同じ 256 KiB。行ごとに切り出す経路は 1/16) は、飛ばす一致の後ろから続けて探す全部の同期検索で分け合い (読んだ量を引いていく)、尽きたら単発のジョブに回す (時間の 2 ms は `search.next` と同じく見ない)。ジョブの結果が飛ばす一致なら、その後ろから同期検索とジョブを続ける (文書の末尾から先頭へ回るのは 1 回だけで、主の Range の `start` に戻ったら止める)。(j) 「単語の一致」の印は、印を立てたときの文書の Revision と選択の写しで持つ。選択を置き直す操作 (移動・クリック・検索の移動など) は印を消し、編集・写像・Undo の後は Revision か選択が写しと違うので印は無いものとする (足す操作と `ui.cancel` は、操作の後に写しを取り直す。検索バーの逐次検索のスクロール (選択を変えない) は、印と目標の列・帰属を戻す)。(k) 主の Range の文字列 (検索語) の長さは 64 KiB (`MATCH_QUERY_MAX_BYTES`、単語の窓と同じ) まで。超えたら何もせずに `info.select.query_too_long` を出す (大きな選択を、メインスレッドで文字列に写して探さない)。主の Range が文書の全体なら、何もしない (ほかに一致は無い。`select.all` の後の `Ctrl+D`) |
| MCUR-3 | **`select.all_matches`** (一致をすべて選ぶ): 検索語の決め方は MCUR-2 と同じ (主の Range が空なら主のカーソルの位置の単語を選び、単語単位にする。単語が無ければ `info.select.no_word`)。文書の全体から一致を選び (§12.1 の一致の選び方: 重ならないものを左から順に、両端が書記素境界のもの)、**今の Range を全部置き換える**。主は、置き換える前の主の Range と重なる一致 (無ければその後ろの最初の一致、それも無ければ最後の一致)。一致の数が上限 (MCUR-6) を超えるときは、選択を変えずに `info.select.too_many_matches` (`{limit}`) を出す (一部だけを選ばない)。文書が 1 MiB 以下ならメインスレッドで、超えれば `bg` の全件の検索のジョブ (02 §9.3 の `JobKind::Search` を、`from = 0`・`max_matches` と `count_limit` を上限 + 1 にして、検索バーとは別の `Pending` で投げる。新しいジョブの種類は足さない。検索バーの全件の走査は取り消さず、前の `select.all_matches` の走査だけを取り消す) で探し、終わった時点で Revision と選択が投入時のままなら置き換える (違えば捨てる。上限 + 1 件に達したら、そこで上限超えとして扱う)。**細目 (M32 の段 3)**: 一致どうしが接する (`aaa` の `a`) と、置き換えた後の N3 で 1 つの Range に併合される (上限と比べるのは一致の数)。主のカーソルの単語から始めたときは「単語の一致」の印を立て、空でない主から始めたときは今の印の有無を保つ (印があれば単語単位で探す)。検索語の長さの上限と文書の全体の扱いは MCUR-2 (k) と同じ。1 MiB は定数 (`ALL_MATCHES_SYNC_MAX_BYTES`)。メインスレッドでは上限 + 1 件の一致で走査を止め、ジョブでは届いた束の合計が上限を超えた時点でジョブを取り消して知らせる。一致が無い (検索語は文書から取るので、ふつうは起きない) ときは `info.select.no_more_matches` |
| MCUR-4 | **`select.add_cursor_above` / `select.add_cursor_below`** (上 / 下の行にカーソルを足す): 一番上 (above) / 一番下 (below) の Range の `head` から、上下の移動 (§5.2。折り返し ON は表示行の単位) を 1 回した位置に空のカーソルを足して主にする。目標の列は View の `goal_dcol` (主の Range のもの。無ければ起点の表示列を `goal_dcol` に入れてから使う) で、足した後も `goal_dcol` を保つ (短い行をはさんでも列が戻る)。起点が主でないときも View の `goal_dcol` を使う。行が短ければ行末。文書の先頭の行 (above) / 最後の行 (below) (折り返し ON では先頭 / 最後の表示行) からは何もしない (知らせない)。足す位置に既にカーソルがあれば、`normalized()` で 1 つになる (数は増えない)。**細目 (M32 の段 3)**: 起点の折り返し位置の帰属 (§14.3.1) は、起点が主の Range なら View の値、主でなければ下の表示行の先頭 (`After`) として移動し、移動の後の帰属を View に入れる (足したカーソルが主になるため)。足す位置が既にある空でない Range の中か端 (例: 前向きの複数行の選択の head から上) なら、N3 / N4 でその Range に併合され、その Range が主になる (数は増えない)。上限は、併合の後の数で調べる (`Selection::with_new_primary`) |
| MCUR-5 | **減らす操作**: `ui.cancel` (`Esc`) は、Range が 2 つ以上なら **主の Range だけを残す** (主の選択は残す。もう一度 `Esc` で選択を解く。04 §7.5 の順)。§4.5.2 の「1 つにする」コマンドも、実行の後は Range が 1 つになる。**細目 (M32 の段 3)**: Range が 1 つで選択があれば、`Esc` は選択を解いて `head` の空のカーソルにする (04 §7.5 の「選択あり → 選択解除」。v0.4 までは実装していなかった)。どちらも、オーバーレイ・検索バー・読込・保存の照合などの前の段 (04 §7.5) が何もしなかったときだけで、「単語の一致」の印は残す。段 3 で、前の段のうち抜けていた「未編集の上書き保存の照合 (`CheckingUnchanged`) の中止」と「表現不能の置換の一覧の作成 (`BuildingReplacement`) の中止 (E3 のキャンセルと同じ)」を入れた (取り消す順は §12.7.6 の細目 (9) に挟み、表示中の文書の読込 → 未編集の上書き保存の照合 → 名前を付けて保存の照合 → 表示していない文書の読込 → 表現不能の置換)。「後続を持つ保存 (`Saving` で `cont` が Some) の取消し」は未実装のまま (後の fix で入れる。それまでは保存の間の `Esc` が選択を減らす)。**v0.5 の M34**: Range が 1 つで選択も無く、View が矩形選択のモードなら、`Esc` はモードを切る (§4.8 の BOX-1。04 §7.5 の層の「選択の解除」の次、「何もしない」の前。前の段の取消しの順 (§12.7.6 の細目 (9)) の後ろなので、読込・保存の照合の取消しとは重ならない) |
| MCUR-6 | **上限**: Range の数は `MAX_CURSORS` = **10,000** (定数。設定のキーは足さない)。Range の数を増やす操作 (MCUR-2〜MCUR-4、矩形選択 (FR-212)、後の版の足す操作) は、増やした後の数が上限を超えるなら **選択を変えずに** `info.cursor.limit` (`{limit}`) を出す。編集・移動・写像は Range の数を増やさない (併合で減るだけ) ので上限に当たらない。Undo / Redo で戻す選択は記録した時点で上限以下 |
| MCUR-7 | **Undo**: 足す・減らす操作は Motion (02 §6.3) で、文書を変えず Undo の履歴に残らない。実行すると Undo の結合を封印する (§7.3 の R3 の「カーソル移動」)。編集の Undo / Redo は、記録した複数の Range をそのまま戻す (`selection_before` / `selection_after`、§7.6)。連続入力の結合 (§7.3 の R7 / R8) は、直前のノードと Range の数が同じときだけ Range を文書の順に対応させて調べ、数が違えば新しいノードにする。R7 の 4 KiB は **Range ごと** の挿入の長さで見る (合計で見ると、カーソルが多いと数文字ごとに区切れる)。**細目 (M32 の段 2)**: Range の数は、新しい Transaction の `selection_before` と直前のノードの `selection_after` で比べる。記録する `selection_after` は併合 (§4.4 の N2〜N6) の後の Selection なので、編集で 2 つのカーソルが 1 つにまとまった後の続けての入力・削除は、1 つの Range どうしとして結合を調べる。Range の数に加えて、変更のまとまり (ChangeSet の変更の区間) の数も同じときだけ、まとまりを文書の順に対応させて調べる (隣り合う 2 つの Range の編集が 1 つのまとまりにつながる (例: 空のカーソルの直前の 1 文字を選んだ Range があるときの `edit.delete_backward`) と、次の編集とはまとまりの数が違い、新しいノードになる) |

#### 4.5.2 コマンドごとの扱い (全カーソル・主のカーソルだけ)

既存のコマンドを、複数の Range でどう動かすかの一覧 (01 FR-211 AC-3 の「主のカーソルにだけ効くコマンドの一覧」)。ここに無いコマンドは選択を変えない。

| 扱い | コマンド | 複数カーソルでの動き |
|---|---|---|
| **全カーソル** (各 Range に同じ規則を当て、1 つの Transaction にする。§4.5 の 1〜5) | `edit.insert_text` (IME の確定を含む。前編集は主のカーソルの位置にだけ出す。§8.3.1 の私用文字の知らせ (`info.paste.pua_as_text`) は 1 回で、件数は入れる文字列の中の数 (カーソルの数を掛けない。MCUR-10 の貼り付けと同じ))・`edit.newline` (自動インデントは各カーソルの行から)・`edit.delete_backward` / `edit.delete_forward`・`edit.delete_word_backward` / `edit.delete_word_forward` | 各 Range の位置で同じ編集。選択のある Range は選択を置き換える / 消す |
| | `edit.indent` / `edit.outdent` | **どれか 1 つでも空でない Range があれば**、全 Range が触れる行の和集合を行単位でインデント / 逆インデントする (同じ行は 1 回、§4.5 の 5)。全部空なら、`edit.indent` は各カーソルにインデントを挿入し、`edit.outdent` は各カーソルの行を逆インデントする (`selection.empty` の意味は §4.5.5) |
| | `edit.duplicate_line`・`edit.delete_line`・`edit.move_line_up` / `edit.move_line_down` (§15.4) | 全 Range が触れる行を連続する行のまとまり (ブロック) にして、ブロックごとに行う。行の移動は、どれか 1 つのブロックが文書の端で動けなければ **全体を何もしない** (一部のブロックだけを動かして並びを崩さない)。各 Range は、**始まりの行を含むブロック** に付いていく (終わりが次の行の行頭でも同じブロック。行の複製は複製の中の同じ相対位置へ、行の削除はそのブロックを消した位置の空のカーソルへ、行の移動は動かした先へ。同じ位置になった Range は `normalized()` で 1 つになる。M32 の段 2 で決めた) |
| | `clipboard.copy` / `clipboard.cut` / `edit.paste` | §4.5.3 |
| | `cursor.move_*`・`select.extend_*`・`cursor.home` / `cursor.end`・`cursor.page_up` / `cursor.page_down`・`cursor.doc_start` / `cursor.doc_end`・`cursor.goto_bracket` (§5.7) | 各 Range を独立に動かして `normalized()` で併合する (§5 の冒頭)。上下の移動の目標の列は主の Range だけが `goal_dcol` を持ち、ほかは各自の今の表示列 (§5.2)。`cursor.doc_start` / `cursor.doc_end` は全部が同じ位置になり、結果として 1 つになる |
| | `edit.toggle_comment` (FR-209、M33) | 全 Range が触れる行の和集合を 1 つの対象にし (同じ行は 1 回、§4.5 の 5)、付ける・外すを対象の全体で 1 回決める (§15.6 の CMT-1 / CMT-2。M33 の仕様の PR で決めた) |
| | 選択の拡張 (FR-309、M34。`select.line`・`select.expand`。§5.7 の SEL-5)・テキスト変換 (FR-214、§15.8 の TXT-1: 大文字・小文字は各 Range、並べ替えと末尾空白の除去は全 Range が触れる行のブロックごと)・自動閉じ (FR-210、§15.7 の APR-6) | 各 Range を独立に (行のコマンドは行のブロックごとに) 扱い、1 つの Transaction にする。保存時の末尾空白除去と末尾改行付与 (FR-216、§9.3.1) は選択に依らず文書の全体で、選択は写像して保つ |
| | `select.shrink` (FR-309) | 直前の `select.expand` の前の Selection (複数の Range) をそのまま戻す (§5.7 の SEL-3) |
| **1 つにする** (主のカーソルについて実行し、実行の後は Range が 1 つ) | `cursor.goto_line`・`select.all`・左クリックとドラッグ (§5.8.1。`Shift` 付きのクリックは主の Range だけを伸ばし、ほかを捨てる。ダブル / トリプルクリック (FR-311、§5.8.2) も同じ)・`cursor.jump_back` / `cursor.jump_forward` (FR-310、§5.9)・`search.next` / `search.prev` (検索バーの `Enter` を含む。一致を選んだ 1 つの Range)・`search.replace` (1 件置換。置換の後ろの 1 つのカーソル)・ファインダと横断検索の結果へのジャンプ (M31)・開き直し / 文字コードの変更 (§8.8。先頭の 1 つのカーソル)・保存の確認のキャンセルによる位置への移動 (表現不能の文字 (§8.6) とつながり (§8.3.1) の確認で、最初の箇所へ移る) | VS Code と同じ |
| **矩形にする** (v0.5 の M34) | 矩形選択のモード (FR-212) の間の `select.extend_*`・左のドラッグ・`Shift` 付きの左のクリック (§4.8 の BOX-2 / BOX-6) | 矩形が無ければ主の Range の `head` から矩形を作り (ほかの Range は捨てる)、矩形の各行の Range にする (BOX-3) |
| **写像して保つ** | `search.replace_all`・ほかのペインからの編集 (§4.6、PANE-2)・外部変更の再読込 (§9.5) | 各 Range を写像し、`normalized()` で併合する。ほかのペインからの編集は `MapPolicy::Other` で写像する。`search.replace_all` の「主カーソルは元の位置の写像」(§12.5 の手順 5) は、複数カーソルでは全 Range の写像になる (発行 View の写像なので、Range が 1 つのときと同じく `MapPolicy::Own`。M32 の段 2 で、Range が 1 つの振る舞いを変えないよう今のままにした)。再読込は全置換の ChangeSet なので、§9.5 の「同じ行:桁 (書記素)」の写像を各 Range の `head` に当てた空のカーソルにし (選択は解く。Range が 1 つのときの今の振る舞いと同じ。M32 の段 2 で決めた)、本文の範囲と書記素の境界に収めてから (PANE-8 (e) の安全網) `normalized()` で併合する |
| **主のカーソルで求め、選択を変えない** | `search.open` (選択からの検索語の取り込みは主の Range)・ステータスバーの位置・`Frame.primary_cursor`・対応括弧の強調 (§5.7) | — |
| **全 Range を使い、選択を変えない** | `search.toggle_selection` (範囲は検索バーを開いたときの空でない Range の全部。§12.5 の表の「選択範囲内」) | — |
| **View ごとに保つ** | `buffer.*` で文書を切り替える・`pane.split_*` (新しい View に全 Range を写す、PANE-1 / PANE-5 (a))・ペインのフォーカスの移動 | 選択は View ごと (PANE-2) なので、切り替えて戻ると複数の Range も戻る |

#### 4.5.3 クリップボード (OQ-56 の細目)

| ID | 規則 |
|---|---|
| MCUR-8 | **コピー** (`clipboard.copy`): 対象は **空でない Range** を文書の順に並べたもの (空のカーソルは数えない)。全部空なら何もしない (§12.6 の「選択が空のとき」。`selection.empty` は全 Range が空のとき真、§4.5.5)。システムのクリップボードへ渡す文字列は、各 Range の文字列を **文書の既定の改行** (`default_eol`、§8.4) で連結したもの (末尾に改行を足さない)。内部レジスタは Snapshot と **片の範囲の並び** (`Vec<ByteRange>`) と連結に使った改行を持ち、`to_text` が連結する (片が 1 つなら今と同じ)。64 KiB の境目 (`Inline` / `Handle`)・`clipboard.system_max_bytes` の判定は片の合計のバイト数 (連結の改行を含む) で行う (§12.6) |
| MCUR-9 | **切り取り** (`clipboard.cut`): コピーと同じものをクリップボードと内部レジスタに入れ、空でない Range を全部消す (1 つの Transaction) |
| MCUR-10 | **貼り付け** (`edit.paste` の応答と、`InputEvent::Paste` (ブラケットペースト・Windows のバーストの合成。04 §6.6)。どちらも同じ `paste_text` を通る) で Range が 2 つ以上のとき、届いた文字列 (読み取れなかったときは内部レジスタの文字列。§12.6) を、次の **3 段** を上から調べて最初に当たった形で入れる (Range が 1 つなら今と同じ)。(1) **内部レジスタの片**: 内部レジスタの片の数が Range の数と同じで、届いた文字列が内部レジスタの連結した文字列と一致する (§12.6 の照合と同じく、両側に同じ改行の正規化をかけて比べる。読み取れずに内部レジスタを使ったときも当たる) なら、片を 1 つずつ文書の順に各 Range へ入れる。(2) **行**: 届いた文字列の行の数が Range の数と同じなら、1 行ずつ文書の順に各 Range へ入れる。行は LF・CRLF・CR のどれでも区切り、区切りの文字は入れない。末尾の区切り 1 つは行を増やさない (`a⏎b⏎` は 2 行。`a⏎⏎b` は 3 行)。(3) **全文**: それ以外は、届いた文字列の全体を各 Range へ入れる。どの段も、各 Range の選択を置き換え、入れた片の後ろにカーソルを置く。入れる片ごとに §8.4 の改行の正規化と §8.3.1 の「本文に入れる規則」(エスケープ文字) を通す ((1) で照合した片は内部レジスタの内容のまま入れる)。**細目 (M32 の段 2)**: (1) の各片には、Range が 1 つのときの「内部レジスタと一致した」場合と同じ規則 (同じ文字コードのバイトの文書にはエスケープ文字のまま、ほかは `to_text` の文字列に経路 (2)) をかける。(2) と (3) の文字列は、届いた文字列に Range が 1 つのときと同じ照合・改行の正規化・本文に入れる規則をかけた後のもので、(2) はそれを行に分ける (改行の正規化で区切りの数は変わらない)。知らせ (`info.paste.*`) は 1 回で、件数は届いた文字列 (内部レジスタ) の中の数 (カーソルの数を掛けない)。全体を 1 つの Transaction (`Origin::Paste`) にする。**入れる量の上限**: 入れる合計のバイト数 (各 Range に入れる片の長さの和。(3) では全文の長さ × Range の数) が 64 MiB (`MULTI_PASTE_MAX_BYTES`、定数) を超えるときは、何もせずに `info.paste.multi_too_large` を出す (10 MB を 1 万個のカーソルに入れて、メインスレッドで 100 GB を作らない)。**既知の制約**: ブラケットペーストの無い端末と、Windows Terminal の短い貼り付け (バーストにならない 16 打鍵未満。04 §6.6) は個別のキーとして届き、改行が `edit.newline` になるので分配されない (全カーソルに同じ文字が入る)。端末に貼り付けを横取りされない `edit.paste` の代替キー `ctrl+k v` (07 §11.6) を案内する (01 §4.6 の既知の制約には M32 の段 2 の PR で書いた。ヘルプの案内は、キー `ctrl+k v` と足す操作を入れる段 3 の PR で書く (段 2 ではキーも複数のカーソルを置く操作も無く、ヘルプのスナップショットも変えない)) |
| MCUR-11 | **入力欄** (プロンプト・検索バー・パレット) への貼り付けは今と同じ (最初の行だけ。04 §6.6)。分配しない |

#### 4.5.4 検索・置換・補完・矩形選択・ペインとの関係

- **検索** (FR-401〜404): 検索バーは主のカーソルで働く。開いたときの検索語の取り込み (選択がある場合) は主の Range から。インクリメンタルの強調は選択を変えないので、`Esc` で閉じれば複数の Range は残る。`Enter` (`search.next`) で一致に移ると 1 つになる (§4.5.2)。選択範囲内 (`search.toggle_selection`) の範囲は、開いたときの空でない Range の全部 (§12.5)。
- **置換** (FR-403): 1 件置換は 1 つにする。すべて置換は全 Range を写像して保つ (§4.5.2)。
- **一致を足す操作の照合** (MCUR-2 / MCUR-3) は検索バーのトグル (大小区別・正規表現・単語単位) に依らない (リテラル・大小を区別・「単語の一致」の印のときだけ単語単位)。検索バーの状態も変えない。
- **補完** (v1.L の AI のインライン補完・LSP の補完): 主のカーソルの位置だけで求め、Range が 2 つ以上の間は出さない (全カーソルに入れる形は v1.L で 06 が決める)。
- **矩形選択** (FR-212、Could。M34): 各行に 1 つの Range を持つ Selection と View の矩形の印で表し、編集は複数カーソルと同じ規則 (§4.5.2) で行う。行の数が上限 (MCUR-6) を超える矩形は作らずに `info.cursor.limit` を出す。仮想の空白・タブと全角の揃え・キーは M34 の仕様の PR で決めた (§4.8)。
- **ペイン** (M30、§9.9): 選択は View ごと (PANE-2) で、ペインごとに別の複数カーソルを持てる。ほかのペインでの編集は全 Range を `MapPolicy::Other` で写像する (§4.6)。Undo の選択の戻し方は §7.6 と PANE-2 のまま (発行 View が表示中なら記録した複数の Range を戻す)。分割は全 Range を写す (PANE-5 (a))。
- **読み取り専用の文書**: 足す・減らす操作は使える (Motion)。編集は今と同じく断る。
- **費用**: Range は `start` の順に並んでいるので、Frame の生成は可視範囲の最初の Range を二分探索で探し、可視の Range だけを見る (O(log n + 可視の数)。02 §8.2 の「可視範囲のみ生成」)。編集は Range の数に比例する (1 つの ChangeSet)。Undo のノードは Range の数ぶんの Selection を持つ (§7.5 のコストに含める)。上限 (10,000) での 1 文字の入力の費用は BM-211 で記録する (§4.7)。

#### 4.5.5 表示・状態・`when` の変数

- **Frame**: `PaneFrame.cursors` は今のまま「可視範囲のすべてのカーソル」(主と主でないもの)。全 Range の選択を `ui.selection` で描く (主と主でないものを色で分けない)。**境界の型は変えない**: frontend は、フォーカスのペインでは `Frame.primary_cursor` (編集面の座標) に当たるセルを主のカーソル (ハードウェアカーソル) とし、`PaneFrame.cursors` のほかのセルを主でないカーソルとして反転のセルで描く (04 §9。フォーカスの無いペインのカーソルと同じ描き方)。`primary_cursor` が `None` (主のカーソルが画面の外) なら、見えているカーソルは全部が主でない。オーバーレイ・検索バーを開いている間 (`focused` が偽) のフォーカスのペインは、今と同じくカーソルを描かない。主でないカーソルのセルは、反転を切り替えたうえで **下線を足す** (前向きの選択の `head` は選択の直後のセルなので、無色の段では反転だけだと選択が 1 セル長く見える。フォーカスの無いペインのカーソルは今のまま反転だけ)。現在行の強調 (`ui.cursor_line`) と現在行の行番号の太字 (`ui.line_number.active`) は **主のカーソルの行だけ** (今の描き方のまま。全カーソルの行に付けると、`select.all_matches` の後に画面じゅうが強調になる)。
- **ステータスバー**: Range が 2 つ以上のとき、右群の選択情報 (`StatusKind::Selection`) の代わりに **カーソルの数** (`status.cursors.long` / `.short`、`{count}`。優先度 4。04 §8.7) を出す。位置 (`StatusKind::Position`) は主のカーソルの位置。
- **`when` の変数** (07 §7.3): `selection.multi` は Range が 2 つ以上のとき真 (M32 で実装する)。`selection.empty` は **全 Range が空** のとき真 (Range が 1 つなら今と同じ「主選択が空」)。`clipboard.copy` / `clipboard.cut` の `when` (`!selection.empty`) は、どれか 1 つでも空でない Range があれば真になる。
- **ヘッドレスの ASCII の Frame** (eve-testkit): フォーカスのペインに主でないカーソルがあるときだけ、末尾 (`cursor: (col,row)` の行があればその後ろ) に `secondary: (col,row) (col,row) …` の行を足す。座標は `cursor:` と同じ **編集面の左上が原点** (`PaneFrame.cursors` に `rect` の位置を足す) で、並びは行・列の順。`primary_cursor` が `None` で `cursor:` の行が無いときも出す (既存のゴールデンは変わらない)。

### 4.6 Selection の View 間写像

同じ Document を複数 View (v0.5) で開いているとき、ある View の編集は他の View の Selection を `Selection::map` で写像する ([02 §5.2](02-architecture.md#52-変更の唯一の経路))。写像ポリシーは `MapPolicy::Other` (挿入位置に一致する点は **前** に留まる、削除範囲内の点は削除開始位置へ潰れる)。スクロール位置 (`ScrollPos.top`、先頭可視行の行頭の `Pos`) も同じ関数で写像し、写像後に行頭へ丸める。

### 4.7 M32 (複数カーソル・対応括弧・タブライン) の試験・実装の順・決めた点 (v0.5)

v0.5 の M32 (08 §2.6) は、FR-211 複数カーソル (§4.5) → FR-308 対応括弧 (§5.7) と、FR-510 タブライン (§9.9 の PANE-12) を入れる。本節は 3 つに共通の試験と計測・PR の分け方と各段の完了の条件・決めた点を置く (M32 の仕様の PR、2026-11-22。点検の材料は docs/review/v0.5-plan.md §3.2 の TAB-01〜05 と §3.3 の MC-01〜09・BR-01 / 02)。

#### 4.7.1 試験と計測 (09)

- **L0 / L1 (複数カーソル)**: proptest で、任意の文書・任意の複数の Range (重なり・接する・空と空でないの混在・CRLF の間を含む) と任意の編集コマンドの列について、(a) 複数の Range への 1 回の編集の結果が、参照モデル「(i) 元の文書の座標で Range ごとの局所の編集を求め、(ii) 重なりは先行の Range を優先して後続を切り詰め (§4.5 の 2)、行のコマンドは触れる行をブロックにまとめて同じ行を 1 回にし (§4.5 の 5、§15.4)、(iii) 末尾から順に適用する」と一致する (§22.1 の「複数カーソル」の行。`edit.newline` の自動インデント・`edit.indent` / `edit.outdent` を含む)。(b) どの操作の後も Selection が INV-S1〜INV-S3 を満たす (`start` の順・重ならない・上限以下・主が範囲内)。(c) 複数カーソルの編集の Undo で本文と複数の Range が編集の前に戻り、Redo で後に戻る。連続入力の結合 (§7.3 の R7 / R8) は全 Range で条件を満たすときだけ。(d) 貼り付けの分配 (MCUR-10。`edit.paste` と `InputEvent::Paste` の両方の経路、Windows Terminal の改行の CR 化の後も 1 段目が当たること) が、入れる量の上限ちょうど / 超えたときを含めて 3 段の参照モデルと一致し、同じ数のカーソルでのコピー → 貼り付けが各 Range の文字列を戻す (片に改行を含む場合も 1 段目で戻る)。(e) ほかの View からの編集の写像の後も (b) を満たす。表の試験: MCUR-2 の単語の選び方 (日本語の文字種の境界、§13.3)・折り返し・終端での打ち切り、MCUR-4 の短い行と `goal_dcol`、上限ちょうど・上限 + 1。
- **L0 / L1 (対応括弧)**: 表の試験で BRK-1〜BRK-6 (10 組・直後と直前・結合文字の付いた括弧・エスケープ文字・ほかの組の交差 `「(」)`・入れ子・上限のちょうど 1 MiB と 1 バイト超・チャンクの境目にかかる全角の括弧)。proptest で、上限の内側では結果が「コードポイントを 1 つずつ読み、同じ組をスタックで数える」素朴な参照と一致し、対応のある括弧の **直前** から `cursor.goto_bracket` を 2 回実行すると元の位置に戻る (括弧の直後から始めたときは、1 回目の後の 2 回で往復する)。
- **L0 (タブライン)**: core の `TabLineModel` (開いた順・アクティブ・PANE-11 の名前・印)。eve-tui の並べ方 (PANE-12 (e)) を「(タブの幅の並び, アクティブ, 幅) → 並び」の純関数にし、全部入る・右だけ省く・左右とも省く・アクティブだけでも入らない・ASCII の縮退の印を単体で確かめる。
- **L3 シナリオ** (`crates/eve-core/tests/multi_cursor.rs`・`bracket_match.rs`・`tab_line.rs` の案。AC の ID は 01 §5.5、`verifies:` を関数の直前に書く): FR-211 AC-1 (足す 4 つの操作・入力・削除・改行・インデント・貼り付け・1 回の Undo・`Esc`)・AC-2 (3 段の分配・コピーの連結)・AC-3 (ステータスの数・上限の知らせ・1 つにするコマンド)。FR-308 AC-1 (強調の有無・移動・3 つの理由)・AC-2 (上限)。FR-510 AC-1 (並び・アクティブ・印・同じ名前・`ui.tab_line` の 3 値)・AC-2 (タブのクリックでフォーカスのペインに表示・オーバーレイの間は捨てる・出し入れでカーソルが見える範囲に保たれる・編集面が 3 行を割るときは出さない)。
- **fuzz**: `fuzz_editor` (`crates/eve-testkit/src/fuzzing.rs`) に、足す 4 つのコマンドと `cursor.goto_bracket`・`ui.tab_line` の切り替え・タブの `ChromeMouse` を足す。既存の種 (`fuzz/corpus/fuzz_editor`) と Windows で回す決定的な試験の読み替えを変えないよう、M30 の `pane.*` と同じく、新しいコマンドは登録したコマンドから選ぶ `cmd` の操作の一覧 (`% commands.len()` で選ぶ) から外し、新しい操作の番号 (今の最後の番号の後ろ) で選ぶ。`KEYS` の表には足さない (剰余が変わる)。`KEYS` に既にある `ctrl+d` は段 3 から `select.add_next_match` になり、既存の種の意味が変わる (受け入れる。パニックしないことの検査なので結果の比較は無い)。**段 2 で足したもの**: 操作 25 (複数カーソル) で、フォーカスのペインの View に 1〜8 個の Range (重なり・同じ位置・逆向き・文書の外を含む。置くときに正規化される) を置き、半分は続けて既存の編集・クリップボード・行のコマンド・Undo / Redo・貼り付けを 1 つ実行する。不変条件に、全 View の Range の数が上限以下で主が範囲内 (INV-S3) を足した。**段 3 で足したもの**: 操作 26 (カーソルを足す操作) で、足す 4 つのコマンドを 1〜8 回 (ときどき検索のジョブを走らせ)、続けて `esc`・文字の入力・ジョブの実行のどれか。**例外** (§4.7.6): 対応の括弧が書記素の途中 (Prepend の後ろなど) にある場合は、移った先から往復しない (性質の試験の往復は、対象の括弧が `head` の直後の書記素の始まりにあるときだけを見る)。**段 4 で足したもの**: 操作 27 (対応括弧。剰余を 27 → 28) で、半分は括弧を含む文字列を入力してから `cursor.goto_bracket` を 1〜4 回実行し、文書が変わらないこと (Motion) を確かめて Frame を作る (強調の経路)。`cursor.goto_bracket` は `cmd` の操作の一覧から外した。
- **ゴールデン (新しく作る)**: ASCII の Frame で、主でないカーソル (`secondary:` の行、§4.5.5)・タブライン (80 桁で全部入る・幅を超える)。
- **L4** (VirtualScreen、`crates/eve-tui/tests/snapshots.rs`): truecolor と mono の段で、主でないカーソルの反転 (選択の上では反転を外す)・ハードウェアカーソルが主のカーソルだけに出ること、対応括弧の太字と下線、タブラインのアクティブの反転と ASCII の縮退の `<` `>`。
- **キー**: 既定キーマップのゴールデン (`default_keymap.tsv`・`commands.tsv`) に 07 §11.6 の M32 の行が増える。FR-606 の頻出操作の一覧には足さない (到達性の検査の対象は変わらない)。
- **計測** (09 §5.2): **BM-211** (参考) — 1 MB の文書に 1,000 個と 10,000 個のカーソルを置き、1 文字の入力 → Frame の core の区間 (BM-103a と同じ測り方) の p50 / p99 を記録する。10,000 個で p99 が 50 ms を超えたら、上限 (MCUR-6) を下げるかを段 3 の PR で決める (仕様を先に直す)。**BM-103b-bracket** (判定) — 10 MB の文書の末尾近くで、対応の無い `)` の直前 (毎回 1 MiB を前へ探す) に 30 ms ごとに 1 文字ずつ打ち、BM-103b の判定区間が NFR-103 (p50 2 ms / p99 8 ms) を満たす (01 FR-308 AC-2)。
- **手での確認 (M)**: Windows Terminal と conhost で、タブライン (FR-510 AC-2)・主でないカーソルの反転と下線・対応括弧の下線 (conhost で下線が出るか。無色の段で、検索の一致の下線の中の括弧を見分けられるか。見えなければ `ui.bracket_match` の無色の段の形を見直す) を目で確かめる (09 §8.2 の互換の行)。

#### 4.7.2 実装の順と各段の完了の条件 (08 §2.6 の M32)

| 段 | PR (ブランチ) | 中身 | 完了の条件 |
|---|---|---|---|
| 1 | `feat/eve-core-tab-line` (`feat`、`contract-change`) | FR-510: PANE-12 (a)〜(h)。eve-view の `Frame.tab_line`・`TabLineModel`・`TabItem`・`ChromeTarget::Tab` (02 §7.3 / §8.2。M30 の仕様の PR で計画した追加だけで、`CONTRACT_VERSION` は 0 のまま)、`ui.tab_line` の公開 (07 §5.3)、ロール `ui.tab_line` / `ui.tab_line.active` を 07 §8.8 から §8.2 と §8.7.1 / §8.7.2 の表へ移す (組込みテーマと eve-tui の `theme_table` を同じ PR で)、TUI の描画 (`ScreenLayout` にタブラインの行を数え、編集面を 1 行下げる。PANE-13)・マウス (幾何の写しにタブを足す)、ヘッドレスの ASCII の Frame、`CHANGELOG.md` の `eve-view:` の行 | 01 FR-510 AC-1 / AC-2 に `verifies:` の試験がある (§4.7.1)。新しいゴールデン。既存のゴールデンの差分は PANE-12 (h) の計画したもの (文書が 2 つ以上の画面の最上行と編集面の 1 行) と、ロールの追加による `crates/eve-core/tests/golden/builtin_themes.tsv` の行の追加と、`ui.tab_line` の公開による既定の設定の JSON (`bins/evoeditor/tests/golden/dump_config_defaults.json`) の 1 行の追加だけで、人間の判断を得て更新する (PO の決定 2026-11-22 で、この段の PR で更新してよい)。`ui.tab_line` の公開で 07 §5.0 の公開キーの件数と `crates/eve-config/src/reserved.rs` の予約の一覧を直す。`check-themes`・`gen-reference --check`・`cargo xtask check` の全体。Windows Terminal と conhost で目で確かめる |
| 2 | `feat/eve-core-multi-cursor-edit` (`feat`) | INV-S3 の制限を外す (`MAX_CURSORS`)。§4.5.2 の既存のコマンドの扱い、§4.5.3 のクリップボード (内部レジスタの片の並び・3 段の分配・連結)、`selection.empty` の意味 (§4.5.5)、ステータスバーのカーソルの数 (言語ファイルの `status.cursors.*`)。新しいコマンドと既定キーは足さない (試験は `Editor` に複数の Range を直接与える) | §4.7.1 の性質 (a)〜(e)。01 FR-211 AC-1 の編集と Undo・AC-2 の試験。Range が 1 つのときの振る舞いは変えない (既存の試験とゴールデンが差分なしで通る) |
| 3 | `feat/eve-core-multi-cursor-commands` (`feat`) | MCUR-1〜MCUR-7 (`select.add_next_match`・`select.all_matches`・`select.add_cursor_above` / `select.add_cursor_below`・`ui.cancel` の段・上限)、`selection.multi`、07 §11.6 の M32 の行と既定キー、言語ファイル (04 §8.14 の文言)、TUI の主でないカーソルの描画 (04 §9)、ASCII の Frame の `secondary:` の行、ヘルプの貼り付けの案内 (MCUR-10 の既知の制約と `ctrl+k v`。文言キーの案は 04 §8.14 の `ui.help.terminal.multi_cursor_paste`)、`editor.multi_cursor` の情報診断 (07 §5.1、`crates/eve-config/src/reserved.rs` の扱いを替える。`msg.config.multi_cursor_always`) | 01 FR-211 AC-1〜AC-3 の試験。既定キーとコマンドの一覧のゴールデンは 07 §11.6 の表どおりの行の追加 (`edit.paste` の `ctrl+k v` を含む) だけ。コマンドとキーの追加による eve-tui のスナップショット (パレットの絞り込みと一覧のあふれ (`palette-filtered`・`palette-overflow`)・ヘルプのキー一覧・which-key) と、ヘルプの案内の追加による help のスナップショットの差分は、人間の判断を得て更新する。L4 の無色の段で主でないカーソルを見分けられる。BM-211 を記録する。段 1・段 2 の後 (08 §2.6 の「Frame・TUI・マウス」の順は FR-510 → FR-211 のカーソルの描画) |
| 4 | `feat/eve-core-bracket-match` (`feat`) | BRK-1〜BRK-6、`cursor.goto_bracket` と既定キー、ロール `ui.bracket_match` (07 §8.8 から §8.2 と §8.7.1 / §8.7.2 へ移し、§8.6 の重ねるロールの一覧と `eve_config::theme` の表に足す)、言語ファイル | 01 FR-308 AC-1 / AC-2 の試験。`check-themes`。`builtin_themes.tsv` の行の追加と、コマンドとキーの追加による eve-tui のスナップショット (パレット・ヘルプ・which-key) の差分は、人間の判断を得て更新する。BM-103b-bracket が NFR-103 を満たす。段 3 の後 (Frame の生成と既定キーのゴールデンを続けて触るため) |

- 段 1 と段 2 は触る場所が違う (段 1 は Frame・TUI・設定、段 2 は `command/`・`command/clipboard.rs`・eve-text の Selection) ので並べて進められる。段 3 は 1・2 の後、段 4 は 3 の後。M33 の FR-704 (`command/edit.rs` のインデント) は段 2 の後 (08 §2.6)。
- 段 2〜4 は境界の型を変えない (§4.5.5 の副カーソルの描き方)。変えることになったら、その PR の前に仕様を直す。

#### 4.7.3 決めた点 (M32 の仕様の PR、2026-11-22。退けた案と理由。下の PO の決定 2026-11-22 で確定)

**PO の決定 (2026-11-22、PR #613。6 点ともユーザー回答で推奨案のまま確定)**: (1) 新しいコマンドの ID (`select.all_matches`・`select.add_cursor_above` / `select.add_cursor_below`・`cursor.goto_bracket`。`select.add_next_match` は v0.1 からの予約) (2) 既定キー (07 §11.6 の M32 の行と `edit.paste` の代替 `ctrl+k v`) (3) 設定のキー (`editor.multi_cursor` は公開しない・`ui.tab_line` を公開する・カーソルの上限と括弧の探す範囲は定数) (4) 境界の型 (複数カーソルでは変えない。§4.5.5) (5) 既存のゴールデンの差分の扱い (段 1 のタブラインとロールの行、段 3・段 4 の既定キーとコマンドの一覧の行の追加・パレットとヘルプと which-key のスナップショット)。(6) `Esc` で残るのは最後に足したカーソル (主。MCUR-1)。(3) の `editor.multi_cursor` は書いたら専用の情報診断 (`msg.config.multi_cursor_always`) を出す。(5) の差分は各段の PR で更新してよい (表に無い既存の行が変わる・消えるときは尋ねる)。

| 点 | 採った案 | 退けた案と理由 |
|---|---|---|
| 主のカーソル | 足した Range を主にする (MCUR-1)。`Esc` は主を残す | 最初のカーソルを主のまま保つ (VS Code): 画面が追う位置 (足した位置) と、ステータスの位置・`Esc` で残る位置が分かれ、「見せるだけ」の別の状態が要る。`Esc` で画面の外へ戻る |
| 主でないカーソルの描き方 | TUI が、`Frame.primary_cursor` に当たるセル以外の `PaneFrame.cursors` を反転と下線で描く (反転はフォーカスの無いペインのカーソルと同じ。下線は、前向きの選択の直後のセルの反転が無色の段で選択の続きに見えないように)。境界の型は変えない | `PaneFrame` に主の添字を足す: `primary_cursor` から求まり、v0.5 の境界の変更を M30 で計画した追加に限りたい。core が主でないカーソルにロールの Span を付ける: 行末のカーソル (文字の無いセル) のために core が空白の Span を足すことになり、フォーカスの無いペインのカーソル (TUI が反転) と描き方が 2 通りになる |
| カーソルの数の上限 | 10,000 の定数 (MCUR-6)。超える操作は何も変えずに知らせる | 上限なし: 1 文字の入力と Undo のノードの大きさが際限なく増える。設定のキー: 公開する名前は戻しにくく、要る理由が分かった版で足せる。1,000: 数千行のログを一括で直す使い方に足りない |
| `editor.multi_cursor` | **公開しない** (07 §5.1。複数カーソルは常に使える) | 公開する (`false` で足す操作を止める): 使わない利用者はキーマップの `"none"` で割当を外せる。全コマンドに分岐と試験の組が増える |
| 一致を足す照合 | リテラル・大小を区別。単語から始めたときだけ単語単位。検索バーのトグルに依らない | 検索バーのトグルに従う: 閉じた検索バーの見えない状態が結果を変える。大小を区別しない: `Foo` と `foo` を同じ名前として一括で直す事故が起きやすい |
| すべて選ぶ (`select.all_matches`) が上限を超えたとき | 何も選ばずに知らせる | 先頭から上限まで選ぶ: 後ろの一致が黙って漏れ、一括の編集で直し残しが出る (01 FR-211 AC-3 の「足さない」) |
| 貼り付けの分配 (OQ-56 の細目) | 内部レジスタの片 → 行 → 全文の 3 段 (MCUR-10) | 行だけ: 改行を含む片をコピーして貼ると戻らない。片だけ: ほかのアプリからの複数行を分配できない |
| コピーの連結の改行 | 文書の既定の改行 (`default_eol`) | 常に LF: CRLF の文書でコピーした片の中の改行 (CRLF) と、片の間の改行 (LF) が混ざる |
| 行の移動で動けないブロックがあるとき | 全体を何もしない | 動けるブロックだけ動かす: ブロックの間の並びが崩れ、結果を予想しにくい |
| マウスで足す (`Alt` + クリック) | v0.5 では入れない | 入れる: 端末が `Alt` / `Ctrl` との組のクリックを矩形選択・リンクに使い (マウスを捕捉していても端末側で使う設定がある)、届き方が揃わない。AC に無い |
| 足したカーソルを 1 つ戻す (VS Code の `Ctrl+U`)・一致を飛ばす (`Ctrl+K Ctrl+D`) | 送る (後の版で要望があれば足す) | 入れる: AC に無い。`Esc` で主の 1 つに戻してやり直せる |
| 括弧の組と数え方 | 10 組。同じ組の括弧だけを数える (BRK-1 / BRK-3) | 全部の組を 1 つのスタックで数える: `「(」)` のように和文の括弧と ASCII が交差する文で対応を誤る。`<>` を含める: 比較の演算子・矢印 (`->`) を括弧と数える |
| 括弧を探す範囲 | 1 MiB の定数 (03 §5.7 の当初の値)。組の 2 つのコードポイントだけを探す | 行の数で区切る: 長大行 (NFR-106) で効かない。上限なし: 100 MB の文書で打鍵ごとに全体を読む |
| 括弧への移動先 | 対応の括弧の直前 (BRK-6) | 閉じ括弧の後ろ: もう一度実行しても元に戻れない |
| 括弧の強調の対象 | 主のカーソルだけ | 全カーソル: 1 万個のカーソルで Frame ごとに 1 万回探す |
| タブの並べ方 | TUI が Frame ごとに決め、前の範囲を覚えない (PANE-12 (e)) | スクロールの位置を覚える (ブラウザのタブ風): TUI に状態が増え、同じ Frame から違う画面になって試験しにくい |
| タブの名前の幅 | 1 タブあたり 32 セルまで | 上限なし: 長い `名前 (親)` の 1 つで、ほかのタブが全部隠れる |
| アクティブなタブの示し方 | 反転の帯に加えて、`[` の後ろの印の欄に `▶` (ASCII は `>`)。印は短い形 (PANE-12 (e)) | 反転だけ: 文字をコピー・読み上げたときに分からない (ペインの見出しの `▶` と揃わない)。ヘッドレスだけ別の括弧 (`{` `}`) にする: TUI とヘッドレスで表示が分かれる |
| 括弧の強調で見る順 | 直後の文字を優先し、括弧でなければ直前 (BRK-2。01 FR-308 AC-1 のとおり) | 「直前が閉じ括弧ならそれ → 直後 → 直前の開き括弧」(打った直後の閉じ括弧を確かめやすい): AC と違い、移動の 2 回の往復 (BRK-6) と見る括弧が分かれる。実装の後に使って困るなら、強調だけ順を変える案を AC の改訂として出す |
| 貼り付けで全文を入れる手段 | v0.5 は持たない (行の数がカーソルの数と同じなら 1 行ずつになる) | `edit.paste` に引数 (例 `distribute = false`) を足す (VS Code の `editor.multiCursorPaste` に当たる): AC に無い。引数は後の版で互換のまま足せる |
| `editor.multi_cursor` を書いたときの診断 | 専用の情報診断 (`msg.config.multi_cursor_always`、04 §8.14。複数カーソルは常に有効で、外すにはキーマップで足す操作の割当を `"none"` にする、と示す) | 公開前のキーと同じ「このバージョンでは未対応」: 後の版で対応すると読める |
| 入れる量の上限 (貼り付け) | 合計 64 MiB の定数。超えたら何もせずに知らせる (MCUR-10) | 上限なし: 10 MB × 1 万個で 100 GB をメインスレッドで作り、メモリが尽きて異常終了する。カーソルの数の上限を下げて代える: 上限 10,000 の用途 (小さな片の一括編集) を狭める |

#### 4.7.4 段 2 (`feat/eve-core-multi-cursor-edit`) で決めた細目 (実装の PR、2026-11-22)

規則の本文は §4.5〜§4.5.3 の各所 (「M32 の段 2 で決めた」と書いた所) に書いた。ここには、決めた点と退けた案を置く。

| 点 | 採った案 | 退けた案と理由 |
|---|---|---|
| 重なる編集の切り詰め (§4.5 の 2) | 後続の削除の始まりを先行の削除の末尾まで後ろへずらし、後続の挿入はそこに入れる | 先行より前から始まる後続の編集を丸ごと捨てる (v0.4 までの実装): 先行の削除の後ろに残る部分 (単語の削除の残り) まで消えず、参照モデルの「重複部分を切り詰める」と合わない。後続を 2 つに割る: 1 つの Range の編集が 2 か所になり、カーソルの置き場所が決まらない |
| 行の操作の後の各 Range (§4.5.2) | 始まりの行を含むブロックに付いていく (Range の数と主を保ち、同じ位置は併合) | 行の削除で、ブロックごとに 1 つのカーソルにする (v0.4 までの実装): Range の数が変わり、主の番号が範囲の外になって編集を断っていた。head の行のブロック: 次の行の行頭で終わる選択の head がブロックの外になる |
| 記録する `selection_after` | 併合 (N2〜N6) の後の Selection | 併合の前のまま記録し、適用のときだけ正規化する: 結合の判定 (MCUR-7 の Range の数) が、見えている選択と違う数で行われ、2 つのカーソルが 1 つになった後の続けての削除が別のノードになる |
| 外部変更の再読込の選択 (§4.5.2) | 各 Range の `head` の行:桁の空のカーソル | anchor と head の両方を写して選択を保つ: Range が 1 つのときの今の振る舞い (選択を解く) が変わる |
| `search.replace_all` の写像 (§4.5.2) | 発行 View の写像 (`MapPolicy::Own`。今のまま) | `MapPolicy::Other` (§4.5.2 の当初の文): Range が 1 つのとき、置換の挿入の位置にあるカーソルの行き先が変わる (段 2 は Range が 1 つの振る舞いを変えない) |
| 貼り付けの知らせの件数 (MCUR-10) | 届いた文字列 (内部レジスタ) の中の数。知らせは 1 回 | カーソルの数を掛ける: 入れた数としては正しいが、1 万個のカーソルで件数が大きくなり、クリップボードの中身の説明として読めない |
| 貼り付けの 2 段目の行の数え方 | 照合・改行の正規化・本文に入れる規則をかけた後の文字列を、LF・CRLF・CR で分ける | 届いた文字列をそのまま分ける: 内部レジスタと一致したときの「エスケープ文字のまま」の規則を、行ごとにもう一度かけることになる。区切りの数は正規化で変わらないので、結果の行の数は同じ |
| `editor.multi_cursor` の情報診断 (07 §5.1) | 段 3 で入れる (07 §5.1 の「M32 の段 3 で `reserved.rs` の扱いを替える」のとおり) | 段 2 で入れる: 段 2 の完了の条件 (§4.7.2) に無く、足す操作の無い段 2 では、案内する「足す操作の割当」がまだ無い |
| ヘルプの貼り付けの案内 (MCUR-10 の既知の制約) | 段 3 で入れる。01 §4.6 の既知の制約は段 2 で書く | 段 2 でヘルプに書く (MCUR-10 の当初の文): 案内する `ctrl+k v` が段 3 まで無く、ヘルプのスナップショットが変わる (段 2 の完了の条件の「既存のゴールデンが差分なしで通る」と食い違う) |
| 同じ行の多数のカーソルの費用 (§4.5.4 の「費用」) | 再読込の行:桁の写しは、旧本文では行ごとに行頭から 1 回、新本文では行ごとに桁の昇順に 1 回だけ数える (行の長さ + カーソルの数)。表示列を求める所 (上下の移動の目標の列・`edit.indent` の挿入の幅・表示行の移動) は、長い行の表示単位と折り返しの結果を直近の 2 行分覚え (元の行と行き先の行)、位置と列は二分探索で引く。`edit.indent` は同じ行の前のカーソルの表示列から数え続ける | カーソルごとに行頭から数える (段 2 の最初の実装): 300 KB の 1 行に 10,000 個のカーソルで再読込が 2 分余りメインスレッドを止めた。BM-211 に再読込を足す: BM-211 は打鍵の応答の参考値で、再読込は打鍵ごとに起きない。数える回数の試験 (`io/reload.rs` の単体試験) で守る |
| ステータスバーの短い形 (04 §8.7) | 04 §8.14 の言語ファイルの案 (`{count}カーソル`) に揃え、§8.7 の表の例も直した | §8.7 の例 (`3 個`) に言語ファイルを合わせる: 2 つの表で違っていた。§8.14 が言語ファイルの文言の案の正本なので、そちらに揃えた |

#### 4.7.5 段 3 (`feat/eve-core-multi-cursor-commands`) で決めた細目 (実装の PR、2026-11-23)

規則の本文は §4.4 の N4 と §4.5.1 の MCUR-2〜MCUR-5 (「M32 の段 3」と書いた所) に書いた。ここには、決めた点と退けた案を置く。

| 点 | 採った案 | 退けた案と理由 |
|---|---|---|
| 主の空のカーソルが選択の端にあるとき (§4.4 の N4) | その選択に併合して選択を主にし、カーソルの側を head にする | 端なら残す (段 2 までの規則): 1 つの選択の端のカーソルに見えるのに、入力・改行が選択の置き換えとカーソルへの挿入の 2 つの編集になる (`abcdef` の 2..5 の選択と 5 の主のカーソルで改行すると `ab⏎⏎f`)。併合して向きを変えない: 画面のカーソル (主のカーソルを置いた端) と head が食い違い、続けての `Shift` + 移動が逆の端から伸びる |
| 次の一致が既にある選択に接するとき (MCUR-2 (f)) | 飛ばす (重なる一致と同じ) | 足して併合する: 主の Range が `aa` に伸び、続けての `Ctrl+D` の検索語が変わる |
| すべて選ぶで一致どうしが接するとき (MCUR-3) | 置き換えた後の N3 で併合されるまま (上限は一致の数で調べる) | 接する一致を 1 つおきに選ぶ・数えない: 一致の一部が黙って漏れ、「一部だけを選ばない」(MCUR-3) と食い違う。接する非空の Range を併合しない: N3 (併合の規則) を変えると、ほかの操作 (移動・写像) の結果まで変わる |
| 照合と CP932 互換文字の同一視 (MCUR-2 (g)) | かけない | 設定 (`search.cp932_compat_fold`) に従う: 「リテラル・大小を区別」と食い違い、`～` と `〜` のような別の文字を同じ名前として選ぶ |
| カーソルの位置の単語の読み方 (MCUR-2 (h)) | 前後 256 バイトから倍にして 64 KiB で止める | 行の全体を読む: 長大行で、カーソルごとに行の全体を読む (カーソルの数 × 行の長さ)。固定の 64 KiB の窓: 短い単語でも毎回 128 KiB を読む |
| 「単語の一致」の印の持ち方 (MCUR-2 (j)) | 立てたときの Revision と選択の写し。`set_selection` で消す | 印だけを持ち、選択を変える経路ごとに消す: 編集・写像・Undo・再読込の全部に消す処理が要り、漏れると単語単位のまま残る。写しだけで判定する: 移動して同じ選択に戻ると印が生き返る |
| 単一の選択の `Esc` (MCUR-5) | 選択を解いて head の空のカーソル (04 §7.5 のとおり) | v0.4 までのまま何もしない: MCUR-5 の「もう一度 `Esc` で選択を解く」と 04 §7.5 に合わない。start / anchor へ潰す: 主のカーソル (ハードウェアカーソル) の位置が跳ぶ |
| `edit.paste` の代替 `ctrl+k v` (07 §11.6) | `editor` のコンテキストだけ | `prompt` にも置く: 入力欄の貼り付けは最初の行だけで分配しない (MCUR-11) ので、要る理由が無い |
| `editor.multi_cursor` の情報診断 (07 §5.1) | 公開前のキーの一覧から外し、常に有効の機能のキーの一覧 (`eve_config::reserved::ALWAYS_ON_KEYS`) と診断の種類 `DiagKind::MultiCursorAlways` を足す | 公開前のキーの一覧に残し、名前で分岐する: 一覧と 07 §5 の表の突合 (`config_reserved`) が「公開前」と「公開しない」を区別できない |
| 主でないカーソルを数える ASCII の Frame の `secondary:` の行 (§4.5.5) | `show_cursor` のときだけ、フォーカスのペイン (`focused` が真) の分 | オーバーレイの間も出す: TUI が描かないものをヘッドレスだけが出し、L3 の画面の比較が TUI と食い違う |
| fuzz (§4.7.1) | `fuzz_editor` の操作 26 (剰余を 26 → 27) で足す 4 つのコマンドを 1〜8 回実行し、`esc`・文字の入力・ジョブの実行を続ける。`cmd` の操作の一覧からは外す | `cmd` の一覧に入れる: 既存の種の番号の読み替えが変わる |
| カーソルの数の上限 (BM-211、§4.7.1 の「計測」) | 10,000 のまま。開発機 (基準機ではない) の BM-211-10k の p99 は 42.7 ms / 44.3 ms (2 回。p50 は 26.9 / 29.0 ms。1k は p99 4.3 / 2.1 ms) で 50 ms を超えない | 上限を下げる: 条件 (p99 が 50 ms を超える) に当たらない。50 ms に近いので、基準機で測り直して超えたら仕様を先に直して見直す |
| 検索語の長さ (MCUR-2 (k)) | 64 KiB まで。超えたら知らせて何もしない。文書の全体なら何もしない | 上限なし: `select.all` の後の `Ctrl+D` で 100 MB をメインスレッドで文字列に写す。検索バーの検索語に揃える: 検索バーには長さの上限が無い |
| 同期検索の予算 (MCUR-2 (i)) | 飛ばす一致の後ろから続ける全部の呼び出しで 1 回分を分け合う | 呼び出しごとに予算を新しくし、進んだ量で打ち切る: 最悪で予算の約 2 倍を読み、行ごとの経路の 1/16 の予算を超えて読む |
| 検索バーの逐次検索と印 | スクロールのために置き直した選択を戻すとき、「単語の一致」の印・目標の列・帰属も戻す | 戻さない (段 3 の最初の実装): 検索バーを開いて閉じるだけで、続けての `Ctrl+D` が単語単位でなくなる |
| `Esc` の段 (04 §7.5) | 抜けていた `CheckingUnchanged` と `BuildingReplacement` の中止を入れた | 入れない: 段 3 から、照合や置換を待つ間の `Esc` が選択を減らし、照合は止まらない |
| 「2 打目の前に `Ctrl` を離す」のヘルプの案内 (07 §11.6 の `ctrl+k shift+up` / `down`) | 後の PR で文言キーを足して入れる (主の決定 2026-11-23)。それまでは既存の `ui.help.terminal.chord` の案内で代える | この PR で入れる: 04 §8.14 に文言キーの案が無い |
| 長い行と多いカーソルの費用 | 上下に足すのは端の Range の 1 つだけを動かす (表示列は段 2 の覚えを使う)。一致を足すのは主の Range からの検索と、二分探索の重なりの判定 (Range の数に比例しない) | — |

#### 4.7.6 段 4 (`feat/eve-core-bracket-match`) で決めた細目 (実装の PR、2026-11-23)

規則の本文は §5.7 の BRK-5 / BRK-6 (「M32 の段 4」と書いた所) に書いた。ここには、決めた点と退けた案を置く。

| 点 | 採った案 | 退けた案と理由 |
|---|---|---|
| 複数カーソルの `cursor.goto_bracket` の費用 (BRK-6) | 主の Range を先に探し、残りは文書の順。全 Range の走査で読む量の合計が 8 MiB (`GOTO_SCAN_TOTAL_MAX_BYTES`、定数) に達したら、残りの Range は探さずに動かさない | 上限なし: 1 万個のカーソルの全部が対応の無い括弧の隣にあると、1 MiB × 1 万 = 約 10 GiB をメインスレッドで読む。主だけを動かす: §4.5.2 の「全カーソル」と BRK-6 の「各 Range について」に合わない。Range ごとの上限を Range の数で割る: 同じ括弧の結果がカーソルの数で変わり、1 つのときの振る舞い (1 MiB) と食い違う |
| 強調を求める所 (BRK-5) | Frame を作る直前 (`Editor::frame`) に、表示中のペインの View だけを求め、View に鍵と結果を覚える。Frame の生成は読むだけ | Frame の生成 (`&EditorState`) の中で毎回探す: 打鍵の無い Frame (時刻・ジョブの結果) でも 1 MiB を読む。内部可変 (`RefCell`) で覚える: 状態の変わり方が型から見えにくい |
| 強調の鍵 (BRK-5) | (Revision, 文書の長さ, 主の head, 探す範囲) | Revision だけ: 本文が Revision を変えずに伸びる経路 (読込の先行表示など) があると、文書の端で決めた「対応なし」が残る。長さは安く比べられるので守りに足す |
| 括弧の探し方 (BRK-4) | ASCII は `memchr2`、全角は先頭の 2 バイトを `memmem` で探して 3 バイト目を見る (10 組の全角の括弧は、開きと閉じで先頭の 2 バイトが同じ)。rope のチャンクは文字の境界で切れているので、括弧がチャンクをまたがない | 3 バイト目だけを `memchr2` で探す: 日本語の本文の継続バイトに多く当たる。チャンクを文字に復号して数える: 1 MiB の走査が打鍵の予算 (NFR-103) に収まりにくい |
| 対応の括弧が書記素の途中にあるとき (Prepend の後ろなど。まれ) | その書記素の始まりへ移り、そこを強調する。**例外**: 移った先の直後の書記素の最初のコードポイントは括弧でないので (BRK-2 は書記素の最初のコードポイントだけを見る)、移った後の強調と、もう一度の実行での往復は途切れる (BRK-2 は広げない) | 書記素の途中へ置く: Selection の正規化 (§4.4 の N1) で書記素の境界へ寄せられ、置いた位置と違う所に出る。そうした括弧を数えない: 素朴な参照 (コードポイントを 1 つずつ数える、§4.7.1) と食い違う。BRK-2 を「書記素の中の括弧」に広げる: 結合文字の付いた括弧・エスケープ文字との区別の規則が増え、まれな場合のために AC の文 (直後の文字) から離れる |
| フォーカスの無いペインの可視範囲 (BRK-5) | 先頭の行の始端から、ペインの行数ぶん下の行の終端まで (折り返し ON でも論理行で数え、見えている行より広い側に寄せる)。長い行でも始端から行数 × 桁数 × 4 バイトまでに切り詰める (PR #622 のレビューで足した)。対象の括弧が範囲の外なら強調しない | 表示行の終わりまで正確に求める: 折り返しの計算を Frame の前にもう 1 回する。行の終端までのまま: 長い行 (100 MB の 1 行) では、可視範囲が数画面でも 1 MiB を読む。水平スクロールの左端から数える: 行頭から左端までの位置を求める費用が要る (行頭からの切り詰めで、上限の 1 MiB より小さく収まる) |
| 属性だけを足す強調が見えないとき (既知の制約。PR #622 のレビュー) | ロールの値は変えず、既知の制約として 07 §8.8 に書き、手での確認 (docs/compat/checklist.md の K1) で確かめる。現在の一致 (`ui.search.current` は `underline` + `bold`) の中の括弧と、高コントラストのテーマの 16 の段 (`ui.bracket_match` は `underline` だけ) のふつうの一致 (`underline`) の中の括弧は、強調が変わって見えない | `ui.bracket_match` に下線の色 (`underline_color`) や前景色を持たせる: テーマの値 (07 §8.2 / §8.7 の表と組込みテーマ) の変更で、段 4 の範囲 (§8.8 の値のまま移す) を越える。後の版で、目で確かめた結果を見て検討する |
| どの Range も動かないときの選択 (BRK-6) | 変えない (目標の列・「単語の一致」の印も残す) | 置き直す: 何も動かないのに `Ctrl+D` の続き (単語単位) が切れる |
| BM-103b-bracket の文書と手順 (§4.7.1) | BM-103 の行 (`make_one_mb` と同じ) を 10 MB まで繰り返し、最後の行に `)` だけを置く。`Ctrl+End` → `←` で `)` の直前に置き、`a` を 30 ms ごとに打つ (`bm_pty` の `--only 103bk`) | 文書の途中に `)` を置く: 末尾の近くでないと、前へ探す 1 MiB が本文の先頭で切れることがある (10 MB では起きないが、手順を短くできる) |


---

### 4.8 矩形選択 (FR-212、v0.5 の M34)

§4.5 の 7 の形 (各行に 1 つの Range を持つ Selection と、View の矩形の印) の規則。受入基準は 01 §5.5 の FR-212。M34 の仕様の PR (2026-11-24) で決めた (試験・実装の段・決めた点は §15.9)。**専用の移動のキーを持たず、モードを入れて既存の選択の操作で矩形にする** (VS Code の「列の選択のモード」と同じ考え。専用のキーの候補は端末とデスクトップに横取りされる、§15.9.3)。

| ID | 規則 |
|---|---|
| BOX-1 | **モード**: `select.toggle_box` (07 §11.6 の M34 の表) で、View の「矩形選択のモード」を入れる・切る (View ごと。分割で作る View は切った状態。文書を切り替えても View のモードは保つ)。入れても選択は変えない。切ると矩形 (BOX-2) を消し、今の Range を複数カーソルとしてそのまま残す。コマンドの種類は `Ui` (`view.toggle_wrap` などの切り替えと揃える。文書を変えず、Undo の履歴に残らない。読み取り専用の文書でも使える)。**抜け方**: もう一度 `select.toggle_box` を実行するか、`Esc` (`ui.cancel`) の層の最後 (04 §7.5: 複数カーソル → 主の 1 つ (§4.5.1 の MCUR-5) → 選択の解除 → **矩形のモードを切る** → 何もしない。1 回の `Esc` は 1 つだけ行う。矩形があるうちは Range が 2 つ以上なので、1 回目の `Esc` で矩形が消える) |
| BOX-2 | **矩形**: モードの間に、選択を伸ばすコマンドとマウスのドラッグ・`Shift` 付きのクリック (BOX-6) は、View の矩形 `{ anchor: (行, 表示列), head: (行, 表示列) }` を作る・動かす。矩形が無いときは、主の Range の `head` の (行, 表示列) を `anchor` と `head` にして作ってから動かす (ほかの Range は捨てる)。行は論理行、表示列は行頭からの表示の列 (タブは文書の `tab_width` で展開し、全角は 2。§13.2) で、折り返し ON でも論理行の行頭から数える。`head` の動かし方: `select.extend_left` / `select.extend_right` は `head` の行の前 / 次の書記素の境界の表示列へ (行の幅より右 (仮想の列) では 1 列ずつ)。**右へは、矩形の行のうち最も長い行の幅 + 1 列で止める** (キーボードで行末より遠くへ広げない)。`select.extend_up` / `select.extend_down` は行を 1 つ上 / 下へ (列は保つ)。`select.extend_word_left` / `select.extend_word_right` は `head` の行の中の単語の移動 (§5.3) の位置の列 (行の外へは出ない)。`select.extend_home` は `head` の行の字下げの終わり (§5.4 の 1 段目)、`select.extend_end` は **矩形の行のうち最も長い行の幅** (全部の行を末尾まで選ぶ。短い行では仮想の列になる)。`select.extend_page_up` / `select.extend_page_down` は行を `viewport_rows - 1` 動かす (§5.5)。`select.extend_to_doc_start` / `select.extend_to_doc_end` は先頭 / 最後の行へ。仮想の列 (行の幅より右の列) になるのは、`select.extend_end`・ドラッグ・`Shift` 付きのクリックと、上の 1 列の余りだけ。モードの間も、移動 (`cursor.*`)・`select.all`・`Shift` の無いクリック・検索の移動・足す操作 (MCUR-2〜MCUR-4) は矩形を消して今までどおりに動く (モードは残る) |
| BOX-3 | **Selection の作り方**: 先に **矩形の行の数** (`anchor` と `head` の行の差 + 1) を見て、`MAX_CURSORS` (10,000、§4.5.1 の MCUR-6) を超えるなら、**行を走査する前に** 断る (直前の矩形と Selection のまま。`info.cursor.limit`)。超えなければ、矩形の行の範囲の各行に、次の Range を 1 つ作る。左の列 `l` = 2 つの列の小さいほう、右の列 `r` = 大きいほう。`l < r` なら、`start` は表示列 `l` の位置 (書記素 (全角・タブ) の途中に落ちたらその書記素の前)、`end` は表示列 `r` の位置 (途中に落ちたらその書記素の後ろ。掛かった文字を含める)。`l == r` (幅 0) なら表示列 `l` の位置の空のカーソル (途中に落ちたらその書記素の前。縦に並んだカーソル)。**行の幅が `l` に届かない行には Range を作らない** (仮想の空白を作らないので、打った文字が行ごとに違う列に入らないように。BOX-5)。ただし `head` の行は必ず 1 つの Range を持つ (届かなければ行末の空のカーソル)。Range の向きは、`head` の列が `anchor` の列以上なら前向き、小さければ後ろ向き。主の Range は `head` の行の Range (MCUR-1。ステータスの位置とスクロールが `head` を追う) |
| BOX-4 | **編集とクリップボード**: 矩形の Selection は複数カーソルと同じ規則で編集する (§4.5.2 の「全カーソル」)。編集・Undo / Redo・ほかの View からの写像・再読込の後は矩形を消し (モードは残る)、Range は複数カーソルとして残す (続けて選択を伸ばすコマンドを使うと、主の Range の `head` から新しい矩形を作る)。**コピー・切り取り** (矩形があるとき): 片は **矩形の行ごとに 1 つ** (行の数 = 矩形の高さ)。Range の無い短い行 (BOX-3) は **空の片** として数える (§4.5.3 の MCUR-8 の「空のカーソルは数えない」の例外)。内部レジスタは矩形の高さの数の片を持ち、システムのクリップボードには片を文書の既定の改行で連結した文字列を渡す (空の片は空の行になる)。切り取りが消すのは Range だけ。**貼り付け** (矩形があるとき): 内部レジスタの片の数が矩形の高さと同じで、届いた文字列が内部レジスタと一致する (MCUR-10 (1) と同じ照合) なら、矩形の i 行目に i 番目の片を入れる (Range の無い行の片は入れない)。そうでなければ MCUR-10 の 3 段を矩形の Range に当てる (行の数が Range の数と同じなら 1 行ずつ)。矩形が無い (複数カーソルとして残った) ときは MCUR-8〜MCUR-10 のまま。1 つのカーソルへの矩形の形の貼り付け (下の行へ揃えて入れる) は v0.5 では入れない |
| BOX-5 | **仮想の空白**: 作らない。矩形の列は行末より右にあってよい (BOX-2) が、Range は行の中だけで、行末より右に文字を入れるときに空白を詰めない。行末より右の矩形の部分は描かない (選択の色は文字のあるセルと、改行を含む選択の行末の 1 セルだけ。04 §9) |
| BOX-6 | **マウス**: モードの間の左のドラッグは、押した位置を `anchor`、ドラッグの位置を `head` にした矩形を作る。`Shift` 付きの左のクリックは、矩形の `head` をその位置へ動かす (矩形が無ければ BOX-2 のとおり主の Range の `head` から作る)。位置は §5.8.1 の「位置の確定」で得た **(論理行, 行頭からの表示列)** で、表示行の末尾より右のセルは、その表示行の末尾の表示列 + はみ出したセルの数にする (仮想の列)。例: 折り返し ON (`view.wrap_indent = "none"`)・編集面の幅 80 列で、表示の幅 100 列の行の継続行 (`wrap_idx = 1`。行頭からの表示列 80〜99 を描く) の 30 セル目 (ガターの後ろから数える) を押すと、表示行の末尾が表示列 100、はみ出しが 10 セルなので、列は 110。ドラッグしない `Shift` の無いクリックは矩形を消して 1 つのカーソルにする。ダブル / トリプルクリック (§5.8.2) はモードの間も単語・行を選ぶ (矩形にしない)。`Alt` 付きのドラッグは今と同じく使わない (端末が自分の矩形選択に使う。§4.7.3) |
| BOX-7 | **表示**: モードの間は、ステータスバーの選択情報 (`StatusKind::Selection`) を、矩形があれば `status.box.long` / `.short` (`{rows}`・`{cols}`。行の数と `r - l` の列の数。`(矩形 5行 12列)` / `矩形5x12`。右 1 のほかの項目 (`(12 文字選択)`) と同じ丸括弧で、曖昧幅の `×` を使わない)、無ければ `status.box_mode.long` / `.short` (`(矩形選択)` / `矩形`) に替える (04 §8.7。カーソルの数 (`status.cursors.*`) より前に見る)。境界の型は変えない。`when` の変数は足さない |
| BOX-8 | **折り返しと長大行**: 折り返し ON でも論理行と行頭からの表示列で決めるので、折り返した行では見た目の矩形にならない (01 §4.6 の既知の制約)。長大行 (NFR-106) では、表示列から位置への変換に列の索引 (§14.4) を使う |
| BOX-9 | **費用**: 行の数を走査の前に上限と比べる (BOX-3) ので、矩形を変えるたびの費用は、行の数 (10,000 まで) × 1 行の表示列から位置への変換。`select.extend_right` / `select.extend_end` の「最も長い行の幅」も同じ行の範囲で求める。Frame の生成は §4.5.4 の「費用」と同じ (可視の Range だけを見る) |

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
- **長大行の縮退 (§11.5、v0.1) との関係** (v0.2 の char 近似モード (§11.5.1) では丸めない。以下は v0.1 の記録): カーソルと選択は長大行の表示範囲 (先頭 64 KiB を書記素境界で切った位置まで) に限るので、文書の最終行が閾値超の行なら `cursor.doc_end` と `select.all` の head は `len_bytes()` ではなく **その行の表示範囲の末尾** に丸める (途中の長大行は選択の内側に入るだけなので丸めない)。この丸めで全選択のコピーが行の残りを黙って落とさないよう、**`select.all` の直後で選択が変わっていない間の `clipboard.copy` は、丸めた選択ではなく文書全体 `[0, len_bytes())` を対象にする** (View に「全選択の丸め中」の印を持ち、選択が変わる操作で消す) (提案、OQ-195。代案は丸めた範囲だけをコピーし「表示範囲外は含まれていません」と通知する)。長大行の文書は読み取り専用なので、切り取りと削除は起きない。

### 5.5 ページ移動 (FR-304)

- 移動量は `viewport_rows - 1` 表示行 (最低 1。01 FR-304 AC-1 の「可視行数 − 1」。前の画面の最終行が次の画面の先頭行として残る)。スクロール位置も同量移動し、カーソルの画面上の相対位置を保つ。目標列は維持する。

### 5.6 指定行へ移動 (FR-306)

- 入力は 1 始まりの行番号 (+ 任意の桁 = 書記素数)。最終行を超える番号は C2 で最終行に丸め、丸めた旨を通知する (`info.goto_line.clamped`)。**1 未満 (0・負数) は丸めずに不正な入力として拒否する** (入力プロンプトは入力欄に `ui.prompt.goto_line.error` を出して開いたまま、コマンドは `InvalidArgs`)。桁は 1 未満を 1、行末を超える値を行末に丸め、通知しない。
- 行番号の代わりに `"$"` (最終行) も受け付ける。引数 `line` の型は 07 §11.2 の `int|"$"` (02 §6.1 の `ArgSchema` では `IntOrLast` (仮称))。`"$"` は文書の最終行を指し、範囲外ではないので丸めの通知 (`info.goto_line.clamped`) は出さない。移動後、カーソルを含む行をビューポート中央に置く (04 でスクロール方針を確定)。

### 5.7 対応括弧・テキストオブジェクト (FR-308 / 309, v0.5)

- 括弧対応は文字ベース (スタックで走査、走査上限 1 MiB で打ち切り) を v0.5 で提供し、tree-sitter の `SyntaxProvider` (v0.6) が対応する言語では構文木ベースを優先する ([§16.5](#165-括弧対応-fr-308-の二段構え))。v0.5 の M32 の仕様の PR (2026-11-22) で決めた細目は下の BRK-1〜BRK-7 (受入基準は 01 §5.5 の FR-308、コマンドと既定キーは 07 §11.6、ロールは 07 §8.8、描画は 04 §9、試験と実装の順は §4.7)。

| ID | 規則 |
|---|---|
| BRK-1 | **対象の組** (定数。設定にしない): ASCII の `()` `[]` `{}` と、全角の `（）` `［］` `｛｝` `「」` `『』` `【】` `〔〕` の **10 組**。`<>` (比較の演算子と区別できない)・`〈〉` `《》` (`〈` は互換の U+2329 と紛らわしく、使う文書が少ない)・引用符は含めない。言語ごとの組 (FR-704 の言語定義) は v0.6 以降 |
| BRK-2 | **対象の括弧**: 主の Range の `head` の **直後の書記素** の最初のコードポイントが組のどちらかなら、その括弧。そうでなければ **直前の書記素** の最初のコードポイントで同じく見る。どちらでもなければ対象なし。選択があっても `head` で見る。不正バイトのエスケープ文字 (§8.3) は括弧ではない |
| BRK-3 | **対応の探し方**: 開き括弧なら後ろへ、閉じ括弧なら前へ、**同じ組の括弧だけ** を数えて入れ子の深さをたどり、深さが 0 に戻った括弧を対応とする (ほかの組の括弧は数えない。`「(」)` の `(` の対応は `)`)。括弧のコードポイントは、後ろに結合文字が続いても 1 つと数える。文字列・コメントの中の括弧も区別しない (構文の解析は v0.6 の §16.5) |
| BRK-4 | **探す範囲の上限**: 対象の括弧から探す向きに **1 MiB** (1,048,576 バイト。`BRACKET_SCAN_MAX_BYTES`、定数) まで。前向きは対象の括弧の終端から、後ろ向きは対象の括弧の始端から数え、対応の括弧の全体 (全角は 3 バイト) がその範囲に入るときだけ対応とする。上限までに深さが 0 に戻らなければ「範囲を超えた」(対応なし) とする。走査はチャンクごとに組の 2 つのコードポイントだけを探す (ASCII は `memchr2`、全角は UTF-8 の 3 バイトの並びの検索) ので、1 MiB でも打鍵の予算 (NFR-103) に収まる (BM-103b-bracket で確かめる、§4.7) |
| BRK-5 | **強調**: 表示中の各ペインの View について、主の Range の対象の括弧と対応の括弧の 2 つのセルに `ui.bracket_match` (重ねるロール。07 §8.8) を重ねる。対応が無ければ (BRK-4 の範囲を超えたときも) 何も強調しない。画面の外の片方は描かない。結果は View ごとに `(Revision, 文書の長さ, 主の head, 探す範囲)` を鍵に 1 つ覚え (文書と主のカーソルと探す範囲が変わらない間は探し直さない)、Frame の生成で使う。主でない Range は強調しない。**フォーカスの無いペイン** は、探す範囲をそのペインの可視範囲 (見えている行の始端から終端まで) に限る (同じ文書を N 個のペインに出していても、1 回の打鍵で 1 MiB を N 回探さない)。**細目 (M32 の段 4)**: 求めるのは Frame を作る直前 (`Editor::frame`) で、表示中のペインの View だけ。可視範囲は、先頭の行の始端 (`line_to_byte(top)`) から、ペインの行数ぶん下の行の終端まで (折り返し ON でも論理行で数える)。ただし長い行でも可視範囲の程度に収めるため、始端から **行数 × 桁数 × 4 バイト** (1 セルに UTF-8 の 1 文字の最大 4 バイト) までに切り詰める (書記素の境界へ前に寄せる)。対象の括弧が可視範囲の外なら強調しない。強調するセルは、括弧のコードポイントを含む書記素の始まり。重ねる順は選択の後 (03 §14.5 の最後。属性だけを足す) |
| BRK-6 | **移動** (`cursor.goto_bracket`、Motion): 各 Range について、対象の括弧に対応があれば、対応の括弧の **直前** へ移って選択を解く (もう一度実行すると元の括弧の直前へ戻る。BRK-2 が直後を優先するため)。対応の無い Range は動かさない。主の Range が動かなかったときは、文書を変えずに理由を出す: 対象の括弧が無い (`info.bracket.none`)・対応が無い (`info.bracket.no_match`)・上限を超えた (`info.bracket.too_far`、`{limit}`)。目標の列 (`goal_dcol`) は消す (§5.2 の水平移動と同じ)。**細目 (M32 の段 4。複数カーソル)**: 主の Range を先に探し、残りは文書の順に探す。全 Range の走査で読む量の合計が **8 MiB** (`GOTO_SCAN_TOTAL_MAX_BYTES`、定数) に達したら、残りの Range は探さずに動かさない (主の Range は必ず探す。1 万個のカーソルの全部が対応の無い括弧の隣にあるときに、1 MiB を 1 万回読まない)。動いた Range があれば選択を置き直し (`normalized()` で同じ位置の Range を併合する)、目標の列・折り返し位置の帰属・「単語の一致」の印 (§4.5.1 の MCUR-2 (j)) を消す。どの Range も動かなければ選択を変えない。対応の括弧が書記素の途中 (前の文字に結合する Prepend の後ろなど) なら、その書記素の始まりへ移る。主の理由は、ほかの Range が動いたときも出す。`{limit}` は `1 MiB` |
| BRK-7 | **後の版**: `SyntaxProvider` のある言語では `bracket_pairs` を優先する (v0.6、§16.5)。巨大ファイル (`file.large_file_threshold`、v0.6 の FR-123) では強調を止める。選択の拡張 (FR-309、M34) の括弧の段はこの探し方を使う (下の SEL-2) |

**選択の拡張 (FR-309、v0.5 の M34)**: コマンドは `select.line` (行を選ぶ)・`select.expand` (選択を広げる)・`select.shrink` (広げる前に戻す) (07 §11.6 の M34 の表)。受入基準は 01 §5.5 の FR-309。M34 の仕様の PR (2026-11-24) で決めた (試験・実装の段・決めた点は §15.9)。構文の木で広げる形は FR-805 (v1.L。tree-sitter の `node_ranges_at`、§16。コマンドは `lang.select_node`) に任せ、v0.5 は単語・括弧・行・文書に絞る (01 FR-309 の「構文ベースは v1.L」に揃える)。

| ID | 規則 |
|---|---|
| SEL-1 | **`select.line`**: 各 Range について、Range が **行の全体** (空でなく、始まりが行頭で、終わりが次の行の行頭か文書の末尾) でなければ、Range が触れる行 (§15.4 の取り方: 前の行から始まって行頭で終わる選択は、その最後の行を含めない) の全体 (最初の行の行頭から、最後の行の区切りの後ろまで。最後の行に区切りが無ければ文書の末尾まで) を前向きに選ぶ。行の全体なら、次の 1 行を足す (終わりを次の行の区切りの後ろへ)。文書の末尾まで選んでいれば変えない (知らせない)。区切りで終わる文書の最後の空の行 (§2.4) では、行の全体が空なので選択は変わらない (知らせない) |
| SEL-2 | **`select.expand`**: 各 Range について、次の候補のうち **Range を真に含む (Range と等しくない) 最小のもの** に広げる (長さの短い順。同じ長さなら番号の小さい順)。(1) **単語**: §13.3 の単位で Range を含むもの (空の Range は §4.5.1 の MCUR-2 (a) の単語。空白のラン・改行は単語にしない) (2) **括弧の中**: Range を含む対応のある括弧の組 (BRK-1 の 10 組) の、括弧の間 (3) **括弧を含む**: 同じ組の括弧を含めた範囲 (4) **行の中身**: Range の最初の行の字下げの後ろから、最後の行の区切りの前まで (5) **行の全体**: SEL-1 の行の全体 (6) **文書の全体**。括弧の組は、Range の始まりから前へ、組ごとに深さを数えて (BRK-3) まだ閉じていない開き括弧を近い順に取り、その対応の閉じ括弧 (BRK-3) が Range の終わり以降にあるもの (内側から外側へ。広げるたびに 1 つ外側の組が候補になる)。前へ探すのは Range の始まりから 1 MiB、対応を探すのは BRK-4 の 1 MiB までで、超えた組は候補にしない。向きは保つ (空の Range からは前向き)。どの候補も Range を含まなければ (文書の全体を選んでいる) 変えない |
| SEL-3 | **`select.shrink`**: View の拡張の履歴 (SEL-4) の最後の Selection に戻し、履歴から取り除く。履歴が無ければ何もしない (知らせない) |
| SEL-4 | **拡張の履歴**: View ごとに、`select.expand` で変える前の Selection を積む (64 個まで。`SELECT_EXPAND_HISTORY_MAX`、定数。超えたら古いものから捨てる)。履歴は、`select.expand` / `select.shrink` の後に取った (Revision, Selection) の写しと、今の文書の Revision と Selection が同じ間だけ有効 (§4.5.1 の MCUR-2 (j) の印と同じ持ち方)。違えば (編集・移動・クリック・ほかの選択の操作・ほかの View からの写像の後) 履歴を捨てる。`select.line` は履歴に積まない |
| SEL-5 | **複数カーソル**: 各 Range を独立に広げ、`normalized()` で併合する (広げて重なった Range は 1 つになる。`select.shrink` は併合の前の Selection に戻す)。主の Range は MCUR-1 のまま。括弧を探して読む量は、全 Range の合計で 8 MiB (BRK-6 の `GOTO_SCAN_TOTAL_MAX_BYTES`) まで (主の Range を先に探す)。使い切った後の Range は括弧の候補を使わない |
| SEL-6 | **候補にしないもの**: 文字列 (引用符の中)・コメント・段落 (空行で区切った塊) は v0.5 では候補にしない (構文を見ないので、`"a" + "b"` の真ん中を文字列と誤る。段落は要望があれば候補に足せる。構文による候補は FR-805、v1.L) |
| SEL-7 | **Motion**: 3 つとも文書を変えず、Undo の履歴に残らない (結合を封印する、§7.3 の R3)。読み取り専用の文書でも使える。目標の列 (`goal_dcol`) と「単語の一致」の印 (MCUR-2 (j)) は消す。矩形選択のモード (§4.8) の間は矩形を消してから広げる |

- テキストオブジェクト (v0.5) は Motion の派生として Selection を返す関数群とし、Vim 風キーマップ (FR-607) からも再利用できる形にする。CommandId (例 `select.word` / `select.inner_brackets`) は 07 §11.6 のカタログに未掲載の将来候補で、名前は着手時に 07 で決める。v0.5 の M34 の選択の拡張 (上の SEL-2) の候補 (単語・括弧の中・括弧を含む・行) を返す関数は、この関数群として作る (Vim 風キーマップの版でコマンドの名前を決める)。

### 5.8 マウス位置 → 文書位置

frontend から `(ViewId, RowIdx, CellCol)` が届く ([02 §7.3](02-architecture.md#73-マウス))。core は View のレイアウト結果から `RowIdx → (論理行, 折り返し番号)` を引き、`display_to_pos` でセル → `Pos` に変換する。行末より右のクリックは行末、空の最終行より下は文書末尾。全角セルの右半分は `Bias::Before` (その文字の前) に寄せる ([§23](#23-未決事項) の OQ-58)。

#### 5.8.1 マウスの扱い (M9b、2026-10-30。案)

M9b の 5 本目の突合。`MouseEvent` / `HitMap` は仕様にあるだけでコードにはまだ無いので、型と挙動をここで決める。

**入力の型 (eve-view)**:

```rust
struct MouseEvent { kind: MouseKind, mods: Modifiers, at: MouseAt }
enum MouseKind { Down(MouseButton), Drag(MouseButton), Up(MouseButton), WheelUp, WheelDown }
enum MouseButton { Left, Middle, Right }
// ヒットの結果。編集面の外 (クローム・オーバーレイ) は TUI が先に判定して core へ渡さない (04 §6.8)。
// v0.5 のペインの見出し・タブは別の入力 InputEvent::ChromeMouse で渡す (02 §7.3、§9.9 の PANE-7)
struct MouseAt { view: ViewId, row: i32, col: i32 }   // 編集面 (PaneFrame.rect) の左上を原点とするセル。ボタンを押している間は面の外 (負・はみ出し) もそのまま渡す
```

`HitMap` は**作らない** (02 §8.2 の `PaneFrame.hit` は型もフィールドも置かず、将来の追加として注記だけを残す)。v0.2 の Window は編集面 1 つで、`PaneFrame.rect` と `gutter_width` があれば TUI は (セル → `MouseAt`) を作れ、折り返し・タブ・全角を考慮した文書位置の確定は core が `ScrollPos` とレイアウトから直接引く (§14.3.1。Frame を作り直さない)。複数ペイン (v0.5) でも `hit` は作らない: TUI は各ペインの `PaneFrame.rect` と `gutter_width` で、押した位置のペインを決めて (セル → `MouseAt`) を作る。ペインの見出しのクリック・フォーカスの無いペインのクリック・ホイールの規則は §9.9 の PANE-7。

**挙動 (core、`Editor::handle_input(Mouse)`)**:

| 操作 | 動作 |
|---|---|
| 左 `Down` | カーソルをその位置へ (Selection を 1 点に)。ドラッグの起点 (anchor) を覚える。`Shift` 付きは anchor を保ったまま head だけ動かす (選択の拡張)。Undo の結合は封印する (カーソル移動と同じ) |
| 左 `Drag` | anchor からその位置までを選択する。面の上下の外では、tick (50 ms ごと、core の deadline) で 1 表示行ずつスクロールしながら選択を伸ばす。マウスが動かなくても進む |
| 左 `Up` | ドラッグを終える (選択は残す) |
| `WheelUp` / `WheelDown` | `ui.mouse_wheel_lines` (既定 3) 表示行ぶんスクロール。カーソルは動かさない。`Shift` 付きは横スクロール (折り返し OFF のときだけ。1 回 = 画面幅の 1/8 を目安に 4 列。案) |
| 中・右ボタン、`Ctrl`/`Alt` 付き | v0.2 は何もしない (中ボタンの貼り付け・右クリックメニュー・複数カーソルは後の版。v0.5 の M32 の複数カーソルもマウスでは足さない (`Alt` 付きのクリックは端末が矩形選択などに使い、届き方が揃わない。§4.7 の「決めた点」))。キーマップの `mouse.*` も v0.2 では既定のみ (07 §7) |
| ダブル・トリプルクリック | v0.5 (FR-311)。v0.2 は単クリックを 2 回として扱う。v0.5 の M34 の規則は §5.8.2 (core が押下の時刻と位置で回数を数える。`MouseEvent` は変えない) |
| オーバーレイ・検索バーが開いている間 | 編集面のマウスは無視する (キー操作と同じく、フォーカスは最前面にある)。v0.5 の複数ペインでは、どのペインの編集面と見出しのマウスも無視する (§9.9 の PANE-6) |

**位置の確定 (§5.8 の具体化)**: 面の行 `row` を表示行として引く — 折り返し OFF は `top` の論理行 + `row`、ON は `ScrollPos { top, wrap_idx }` から `row` 本ぶん進めた (論理行, 表示行)。列は、OFF は `left_col + (col − gutter_width)`、ON は表示行内の列 (継続行のずらしを含む。§14.3.1 の `LineRows::at_col`)。ガターの列は列 0 として扱う。画面の下の `Virtual` 行は文書末尾、表示行の幅を超えた列はその表示行の末尾 (折り返し位置では上の表示行の末尾 = `Before`)。全角セルは左右どちらの半分でも**その文字の前** (OQ-58 の推奨を採る)。長大行の読み取り専用の範囲 (§11.5) を超える位置は表示範囲の末尾に丸める (`set_selection` が行う)。

**実装の分け方**:

1. この節 (突合の記録)。
2. eve-view の `MouseEvent` と core の処理 (クリック・ドラッグ・Shift・ホイール・自動スクロール。ヘッドレスの試験)。`ui.mouse_wheel_lines` の定数 3 を持つ。
3. TUI と設定: 端末のマウス取得の有効化 / 解除 (04 §6.8)、`ui.mouse` の公開、`ui.toggle_mouse`、`Capabilities.mouse`。

#### 5.8.2 ダブル・トリプルクリック (FR-311、v0.5 の M34)

受入基準は 01 §5.5 の FR-311。M34 の仕様の PR (2026-11-24) で決めた (試験・実装の段・決めた点は §15.9)。**境界の型は変えない**: frontend は今と同じく押下のたびに `Mouse { Down(Left) }` を送り、回数は core が数える (ヘッドレスの試験は押下の列と手動の時計で回数を作れる)。

| ID | 規則 |
|---|---|
| CLK-1 | **数え方**: 左の `Down` (修飾なし) を受けたとき、直前の左の `Down` と比べて、(a) 同じ View (b) 同じ表示行 (`MouseAt.row`) で、確定した文書の位置 (§5.8.1 の「位置の確定」) が同じか同じセル (c) 間隔が **500 ms 以内** (`DOUBLE_CLICK_MS`、定数。Windows の既定のダブルクリックの時間と同じ) のすべてを満たせば回数を 1 増やし、そうでなければ 1 にする。時刻は押下を処理するときの Editor の「今」(Host が Clock から渡す時刻。02 §5.4) で比べる。間に、キーの入力・ホイール・ほかのボタン・`Shift` や `Ctrl` / `Alt` 付きの押下・オーバーレイや検索バーの開閉・押下によらないペインのフォーカスの移動 (`pane.focus_*` など) があれば数え直す (1 にする)。押下によるフォーカスの移動 (§9.9 の PANE-7。フォーカスの無いペインの押下) は数え直さない (CLK-5)。ドラッグ (`Drag`) と `Up` は回数を変えない。数える状態は Editor に 1 つ |
| CLK-2 | **2 回 (ダブルクリック)**: 押した位置の §13.3 の単位 (単語・空白のラン・記号 1 つ) を前向きに選ぶ。位置の直後の書記素を含む単位で、位置が行末 (直後が区切りか文書の末尾) なら直前の書記素の単位。空の行では何も選ばない (1 回目のカーソルのまま)。単位を探す窓は §4.5.1 の MCUR-2 (h) と同じ (64 KiB まで。長大行で行の全体を読まない) |
| CLK-3 | **3 回 (トリプルクリック)**: 押した位置の行の全体 (行頭から区切りの後ろまで。最後の行は文書の末尾まで。§5.7 の SEL-1 の行の全体と同じ) を前向きに選ぶ。折り返し ON でも論理行。4 回目より後も 3 回と同じ (行を選び直す) |
| CLK-4 | **ドラッグ**: 2 回・3 回の押下に続くドラッグは、その単位 (単語・行) で選択を伸ばす。起点の単位を常に含め、ドラッグの位置の単位の端まで選ぶ (位置が起点の単位より後ろなら位置の単位の終わりまで前向きに、前なら位置の単位の始まりまで後ろ向きに。`head` は位置の側)。面の外での自動スクロール (§5.8.1) も同じ単位で伸ばす。1 回の押下のドラッグは今のまま (文字の単位) |
| CLK-5 | **ほかとの関係**: どの回数の押下も、今の単クリックと同じく Range を 1 つにする (§4.5.2 の「1 つにする」)。フォーカスの無いペインの押下 (§9.9 の PANE-7) も同じに数える (1 回目でフォーカスが移り、2 回目で単語を選ぶ)。矩形選択のモード (§4.8 の BOX-6) の間も単語・行を選ぶ (矩形を消す)。キーマップの `mouse.left.double` / `.triple` (07 §7.2) の割当は v0.5 でも読まない (既定の振る舞いだけ) |
| CLK-6 | **Motion**: 文書を変えず、Undo の結合を封印する (今のクリックと同じ)。読み取り専用の文書でも使える |
| CLK-7 | **費用**: 単位を求めるのは押下と自動スクロールのときだけで、窓は 64 KiB まで (CLK-2) |

### 5.9 ジャンプ履歴 (FR-310、v0.5 の M34)

コマンドは `cursor.jump_back` (前の位置へ戻る) / `cursor.jump_forward` (進む) (07 §11.6 の M34 の表)。受入基準は 01 §5.5 の FR-310。M34 の仕様の PR (2026-11-24) で決めた (試験・実装の段・決めた点は §15.9)。

| ID | 規則 |
|---|---|
| JMP-1 | **記録する移動** (ジャンプ): 次の操作で主のカーソルの位置か表示の文書が変わったとき、変わる前の (文書, 主の Range の `head`) を記録する。(a) `cursor.goto_line`・`cursor.doc_start` / `cursor.doc_end`・`cursor.goto_bracket` (主の Range が動いたとき) (b) `search.next` / `search.prev` (検索バーの `Enter` を含む)・`search.replace` の後の移動。**検索バーを開いてから閉じるまでに続けた移動は、最初の移動の前の位置の 1 項目だけを記録する** (続く移動では項目を足さない。`cursor.jump_back` で検索を始める前の位置へ戻る) (c) ファインダ・横断検索の結果へのジャンプ (03 §12.7。別の文書を含む) (d) ペインに出す文書の切り替え (`buffer.next` / `buffer.prev`・バッファ一覧の選択・タブのクリック・`file.open` / `file.new` で開いた文書を出す。切り替える前の文書の位置を記録する)。矢印などの移動・ページ移動・マウスのクリック・スクロール・選択を伸ばす操作・`cursor.jump_back` / `cursor.jump_forward` 自身は記録しない |
| JMP-2 | **持ち方**: **ペインごと** (ペインの木の葉ごと。ペインの中で文書を切り替えても同じ履歴が続く) に、項目 (DocumentId, Pos) の並びと今の位置 (添字) を持つ。上限は 100 項目 (`JUMP_HISTORY_MAX`、定数。超えたら古いものから捨てる)。記録するときは、今の位置より先 (進む側) の項目を捨ててから末尾に足す。末尾の項目が同じ文書の同じ行なら、足さずに置き換える (同じ行を重ねない)。分割 (`pane.split_*`) で作るペインは元のペインの履歴を写す。`pane.close` で閉じたペインの履歴は捨てる |
| JMP-3 | **戻る・進む**: `cursor.jump_back` は、今の位置より前の項目へ移る。今の位置が末尾 (まだ戻っていない) なら、先に今の (文書, 主のカーソル) を末尾に記録してから戻る (進むで今の位置へ戻れる)。今と同じ文書の同じ行の項目は飛ばす。移るときは、項目の文書をそのペインに出し (ペインの文書の切り替えと同じ。View は §9.9 の PANE-2 のとおり文書ごと)、主のカーソル 1 つ (選択なし) をその位置に置いて、`cursor.goto_line` と同じく行を見える範囲の中央へ置く。`cursor.jump_forward` は逆向き。端なら何もせず `info.jump.no_back` / `info.jump.no_forward` を出す |
| JMP-4 | **位置の写像**: 項目の `Pos` は、その文書の変更のたびに Selection と同じく写像する (`MapPolicy::Other`。§4.6)。外部変更の再読込は §9.5 の「同じ行:桁」の写像。どれも本文の範囲と書記素の境界に収める |
| JMP-5 | **閉じた文書**: 文書を閉じる (`buffer.close`、置き換え (§9.2) を含む) と、全ペインの履歴からその文書の項目を消す (閉じた文書を開き直さない)。今の位置の添字は、消した項目の分だけ詰める |
| JMP-6 | **保存しない**: 履歴はメモリだけに持ち、ディスクに書かない (セッションの復元 (FR-121、07 §16) を入れる版で考える) |
| JMP-7 | **Motion**: 戻る・進むは文書を変えず、Undo の履歴に残らない。読み取り専用の文書でも使える。複数カーソルは 1 つにする (§4.5.2)。費用は項目の写像が項目の数 (ペインの数 × 100) に比例 |

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
  4. text.apply(&tx.changes); format.eol_stats を更新 (下記「eol_stats の更新」); (v0.4) 保持方式の記録 (preserved) を写像する
     (§8.7.1 の「編集での写像」。外した記録は手順 6 で Undo のノードに持たせる。tx.is_undo なら写像で外さず、ノードの記録を戻す (Undo)・外す (Redo))
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
- **eol_stats の更新** (手順 4。INV-D5): 各編集 (旧座標の削除範囲 `[s, e)` と、同じ位置の挿入文字列 x) について、旧文書の窓 `[s-1, e+1)` と新文書の窓 `[s-1, s+|x|+1)` (どちらも文書の範囲に切り詰め、窓の端が CRLF の `\r` と `\n` の間に当たるときは 1 バイト外へ広げる) を求める。**ChangeSet が複数の編集を持つときは、旧の窓どうしが重なるか接する (間が 1 バイト以下の) 編集の窓を 1 つにまとめる** (旧の窓は最初の編集の `s-1` から最後の編集の `e+1` まで、新の窓はそれに対応する新座標の範囲。編集の間の変わらないテキストは両方の窓に入る)。まとめないと、重なった部分の区切りや、隣り合う 2 つの窓の端の広げで両方に入った同じ CRLF を、二重に引き・足す。まとめた窓ごとに区切りを種類別に数え、旧の窓の分を引いて新の窓の分を足す。窓の端の前後 1 バイトを含めるのは、§2.4 の合体・分離 (端の `\r` と `\n` の組み替え) を数え漏らさないため。区切りの種類は窓の外の隣のバイトも見て決める。コストは「削除量 + 挿入量」に比例し、文書長に依存しない (100 MB の中央への 1 文字挿入は数バイトの走査)。
  - **巨大な削除** (削除量 64 KiB 以上): 削除テキストを区切りの種類ごとに走査する代わりに、旧の窓の区切りの総数 `T` を行数差 (`byte_to_line` の差、O(log n)) で求め、窓の中の `\r` と `\n` のバイト数 `C_r` / `C_n` を SIMD のバイト計数 (数 GB/s 以上) で数えて、`crlf = C_r + C_n − T`、`cr = C_r − crlf`、`lf = C_n − crlf` とする。100 MB の全選択削除でも数十 ms 以内の見込み (09 の BM で計測し、超える場合は本節の下の実体化ジョブが複写の際に数える方式に替える)。編集後の文書が 64 KiB 以下になる削除 (全選択削除など) は、編集後の全文を数え直す方が速いのでそうする。
- **保持方式の記録の更新 (手順 4、v0.4)**: 本文を変える経路 (通常の編集、Undo / Redo、再読込、復旧、改行コードの変換、すべて置換、E3 の置換など) はどれもこの `apply` を通るので、記録の更新もここ 1 か所で行い、経路ごとに別の更新を書かない (規則は §8.7.1)。費用は変更の数と触れた記録の数に比例し、記録の総数に比例しない (NFR-105)。文字コードを指定して開き直す (§8.8) は `apply` を通らず、記録を新しい読込の結果に入れ替える。
- 手順 1 の `StaleBase` は、AI Proposal や LSP 編集 (v1.L) が古い Revision に基づいて届いた場合に起きる。パニックではなく **エラーで返し**、呼び出し側が §18 の写像規則 (v1.L) で処理する。v0.1 ではこの経路の呼び出し側が無く、届いた結果は破棄する (§17.2)。
- **`EditSummary` の生成 (手順 10、v1.L)**: v0.1 は型 (`EditSummary` / `EditDelta`) の名前を予約するだけで生成しない (ADR-0039)。生成は最初の利用者 (LSP 増分同期・ローカル LLM) と同時に入れ、`DocumentChanged` への `Option` フィールドの追加は C2 の「追加」に当たる。生成の規則: ChangeSet の各 `Op` を旧 Revision の座標で走査し、連続する `Delete` + `Insert` は 1 つの `EditDelta` にまとめる。コストは編集箇所の数に比例し、文書長に依存しない。購読者 (LSP の増分同期、ローカル LLM のプロンプトキャッシュ境界の判定と補完要求の破棄 (06 §6.7 / §5.3)、駆動 API、swap) は `first_old` より前が変わっていないことを前提にできる。型 (`EditSummary` / `EditDelta`) は eve-view (境界型)、生成は eve-core。
- **`Query` の決定性 (06 §17 X26、v0.1 で保証)**: `Editor::query(Query::DocumentText { doc, range })` は **同じ Revision・同じ範囲なら同じバイト列** を返す (改行は文書のまま非正規化、不正バイトは §8.3 の PUA エスケープのまま、末尾改行の有無も保持。時刻・パス・ID を混ぜない)。これが 06 §6.7 の「先頭を固定したプロンプト」の前提になる。性質テストは 09 §3.2 (eve-core `Query`)。
- 手順 3 の逆変換生成コストは削除量に比例する。「全選択削除 → 100 MB の削除テキスト」のような場合に `Arc<str>` へコピーするとメインスレッドで 100 MB を複写するので、`Op::Insert` の文字列表現は次の 3 つを許す `InsertText` 列挙体とする。

| 変種 | 中身 | 使う場面 | 会計 (§7.5) |
|---|---|---|---|
| `Str(Arc<str>)` | 文字列そのもの | 通常の挿入、64 KiB 未満の削除の逆変換 (コピーする) | 文字列のバイト数 |
| `Slice { snapshot, range }` | 編集前の Snapshot と範囲 | 64 KiB 以上の削除の逆変換の **記録直後だけ** (実体化ジョブの完了まで) | 削除バイト数 + 8 KiB |
| `Rope(Snapshot)` | 範囲だけを持つ独立の rope (eve-text の自前の型 `Text` で包む。ropey の型は公開しない、§2.2) | 実体化ジョブの完了後。範囲が Snapshot 全体 (全選択削除・Reload / Recovery の全置換) なら記録時から旧 rope をそのまま入れる (コピーなし) | 削除バイト数 + 8 KiB |

- **Slice を長く持たない理由と実体化** (02 §14.1 MR1): ropey 1.x の `RopeSlice` は元の rope を借用し部分木だけを所有できないので (この前提は P0 スパイク S-02 で確認する、§2.3)、`Slice` は **編集前の文書全体** の Snapshot を保持する。その後の編集で現在の rope が経路を作り直すたびに旧 Snapshot だけが参照するノードが増え、最大で当時の文書全体が会計外で残る。そこで手順 6 で `Slice` を記録したら、直ちに `bg` ワーカーへ **実体化ジョブ** (`JobKind::Materialize { snapshot, key: SliceKey, range }` (02 §9.3。文書は `JobRequest.doc`)。`SliceKey` は `Slice` を作るときに振る文書内の通番) を投げ、ジョブは範囲だけを複写した独立の `Text` を作って `JobResult::Materialized { key, rope }` で返す。メインは結果を受け取ったとき、その `SliceKey` の `Slice` がまだ履歴 (undo / redo のいずれかのノードの ChangeSet。結合で別のノードに合成されていてもよい) にあれば `InsertText::Rope` に置き換え (旧 Snapshot を解放)、無ければ捨てる。ノードの `NodeId` は結合で振り直されるので、採否の鍵には使わない。実体化ジョブは検索と同じ `bg` で動くが、同種の後続ジョブにキャンセルされない (各ノード独立)。複写のコストは削除量に比例し (100 MB で数十 ms)、メインスレッドでは行わない。
- 64 KiB 未満の削除を `Str` にコピーするのは、小さな削除のたびに Snapshot (と、その版にしかない経路 4〜8 KiB) を持つと 1 万ステップで数十 MB が会計外に残るため。結果として、会計に現れないメモリは「実体化ジョブの実行中の Snapshot」だけで、時間的に有界になる。性質テスト「ランダムな 1 万回の編集 (64 KiB 以上の削除を一定割合で含む) の後、実体化ジョブがすべて完了した時点で RSS 増分 ≤ Undo 会計の 1.2 倍」を v0.1 の L1 に置く (09 §3.2)。
- **実装の状態 (v0.3、SAF-33)**: 実装は `Str` と、文書全体の置き換えの `Rope` (`Snapshot` を包む) だけ。`Rope` を使うのは再読込の新全文 (読込の rope を共有。§9.5) と、文書全体の削除の逆変換の旧全文 (旧 rope を共有) で、どちらも複写しない。文書全体の削除の適用は rope を空にするだけ、`Rope` の挿入の適用は rope をつなぐだけ (共有している節を複製しない)。それ以外の削除は 64 KiB 以上でも `Str` に複写する (表の `Slice` と実体化ジョブは未実装。**v0.4 の方針 (2026-11-07 に決定。docs/review/v0.3-findings.md の SAF-33)**: 64 KiB 以上の削除の逆変換は、削除の前の rope を `clone()` して両端を `remove` した `Rope` で持つ。`Slice`・実体化ジョブ・`SliceKey` は入れず、この表と下の「Slice を長く持たない理由と実体化」、02 MR1 は v0.4 の作業で改める)。なお ropey 1.6.1 の `Rope::from(RopeSlice)` は範囲の部分木を共有して端の経路 (O(log n) 個の節) だけを複製するので、上の「部分木だけを所有できない」前提は 1.6.1 では成り立たない (4 MiB の範囲で確保は約 12 KB)。

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
    view: Option<ViewId>,            // 発行 View (記録した Transaction の tx.view。結合したら後の Transaction のもの。v0.5 の M30 の段 2a)。Undo / Redo で選択を戻す相手 (§7.6)
    format_change: Option<FormatChange>, // 種類で戻す範囲が決まる (§7.2): Reload { before, after } = 再読込前後の TextFormat 全体と内容由来の読み取り専用の理由 (§9.5) と、v0.4 は保持方式の記録の表 (FormatState.preserved。表の複製は O(1))。DefaultEol { before, after } = 改行コードの変換 (file.set_line_ending) の前後の default_eol だけ (§8.4)。Recovery { before, after } (v0.4) = 復旧 (Origin::Recovery) の前後の保持方式の記録の表と状態と、M23 から escape_encoding・pua_escape・読み取り専用の理由 InvalidBytes の有無 (PreservedState { map, status, escape_encoding, pua_escape, invalid_bytes_reason }。§8.3.1)。スワップの復旧は呼び出し側が Document::recovery_change で作って渡し、渡されなければ Document::apply が値を変えない形で作る (§8.7.1)
    preserved_removed: PreservedUndo,    // v0.4: この Transaction の適用で外した保持方式の記録 (§8.7.1)。Undo で本文と一緒に戻し、Redo で再び外す。M22 の実装 (2026-11-12): None | Removed(Arc<[PreservedSpan]>) (編集の前の座標、昇順。Undo / Redo で複写しない) | Range { after, hull_len, before, removed } (粗い逆変換 §12.5 のノード。Undo は編集の後の範囲 after の記録を、編集の前に affected_hull で広げた範囲にあった記録 before に入れ替え、Redo は removed を外す)。再読込・復旧の表の入れ替えは format_change が持つ。実装のフィールド名は preserved
    origin_class: OriginClass, last_edit_at: Tick, group_sealed: bool,
    bytes: usize,                    // メモリ会計用 (§7.5)
}
// 現在位置 current() = undo.back().map(|n| n.id).unwrap_or(base)。dirty = (saved != Some(current()))
```

- **位置の表し方**: 履歴上の位置は **ノードの通番 (`NodeId`)** で表し、スタック上の深さでは表さない。深さで表すと、予算超過で底のノードを捨てたときに残りの位置が全部ずれ、保存点と現在位置が偶然一致して「未変更」と誤判定する (FR-107 の確認が出ずに未保存の内容を失う)。通番なら底を捨てても他のノードの位置は変わらない。
- **Undo** = `undo` スタック末尾の `undo` ChangeSet を Transaction (元の Origin + `is_undo`) として適用し、ノードを `redo` スタックへ移す。発行 View (`view`) の Selection は `selection_before` を復元し、他 View は写像 (発行 View の選び方は §7.6)。ノードが `format_change` を持てば再読込前の `TextFormat` と読み取り専用の理由も戻す。適用後に封印する (直後の入力を Undo 前のノードに結合しない、§7.3)。
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
| 文字コード・改行コード設定の変更 | **いいえ** (メタデータ)。ただし「改行コード変換」(`file.set_line_ending`、§8.4) はテキスト変更なので戻り、`default_eol` も変換前に戻る (`UndoNode.format_change`)。変換のノードの `format_change` が戻すのは **`default_eol` だけ** で、文字コード・BOM・往復の情報 (`roundtrip`)・不正バイトの数・読み取り専用の理由は戻さない (変換の後に変えたメタデータを Undo で巻き戻さない。`eol_stats` と長大行の数は本文の適用で数え直す)。`TextFormat` 全体を戻すのは下の再読込のノードだけ | メタデータ変更はステータスバーで見え、再設定が容易。変換は本文と新しく挿入する改行を一緒に戻さないと、Undo の後の Enter が変換後の改行になり混在を作る |
| 外部変更の再読込 (Reload。v0.1 は明示の `file.reload`、フォーカス復帰時の確認は v0.2) | **はい**。本文と一緒に、再読込前の `TextFormat` と、再読込で付け直した読み取り専用の理由 (どの理由を付け直すかは §8.8 の「読み取り専用の理由の差し替え」) も戻る (`UndoNode.format_change`)。ただし **再読込の後に読み取り専用の文書では Undo できない** (`edit.undo` の when が `!doc.read_only`、07 §11.6。提案、OQ-216) | 「再読込で自分の編集が消えた」を取り消せる (§9.5)。本文だけを戻すと、不正バイトを含む本文が編集可能になる等の不整合が起きる |
| 文字コードを指定して開き直す (FR-111) | **いいえ** (Undo できない操作。履歴を消す。確定 (ユーザー回答)、§8.8) | 開き直しは dirty でないときだけ可なので未保存の内容を失わない。本文・文字コード・読み取り専用・保存点が食い違う状態を作らない |
| 保持方式の記録 (v0.4、§8.7.1) | **はい**。編集で外した記録は、その Transaction の Undo のノードが持ち、Undo で本文と一緒に戻し、Redo で再び外す。再読込のノードは前の記録を、復旧のノードは前の記録と保持の状態を戻す。復旧のノードは、M23 から `escape_encoding`・`pua_escape`・内容に由来する読み取り専用の理由 `InvalidBytes` の有無も戻す (§8.3.1。読み取り専用の文書では Undo できないのは再読込と同じ)。内容が同じ文字列を打ち直しても記録は戻らない (戻すのは Undo / Redo だけ) | 編集して Undo で戻した範囲は「未編集」と扱う (PO の回答、2026-11-11) |
| 読み取り専用の解除・設定 (`file.toggle_read_only`。v0.4 の M23 の不正バイトの「編集を許可」を含む) | **いいえ** (文書の状態。解除の記録 `invalid_bytes_released` も戻らない)。再読込の Undo / Redo は、再読込で付け直した (または解除を持ち越した) 前後の理由を `format_change` で戻す (§8.3.1 / §8.8。ただし解除の記録がある間は、戻した後がバイトの文書なら `InvalidBytes` を付けない。復旧のノードも同じ。§8.3.1 の「解除の持ち越し」の細目 (b)) | 編集の権限は履歴の外の利用者の判断 |
| 保存 | 該当なし (履歴に境界を作るだけ) | FR-204 |

### 7.3 結合 (グループ化) 規則 (FR-203)

連続する Transaction を 1 つの Undo 単位にまとめる。`Document::apply` 手順 6 で `undo` スタック末尾のノード (以下「直前のノード」) に **結合** するか **新ノード** を作るかを判定する。

**判定の順序**: 新しい Transaction `tx` について、下表を **上から順に** 調べ、最初に当たった行の結果を使う (後の行は見ない)。「封印」はノードの `group_sealed` を立てることで、封印されたノードには以後何も結合しない。FR-203 AC-1 / AC-2 のプロパティテストの参照モデル (§22.1) はこの表をそのまま上から評価する関数として書く。

| 順 | 条件 | 結果 | 理由 |
|---|---|---|---|
| — | (前提) `tx.changes` が恒等 | 判定しない。§6.5 手順 1 で記録も封印もせずに終える (R1〜R9 のどれにも入らない) | 何も変えない操作で Undo の単位や結合の状態を変えない |
| R1 | `tx.undo_group == UndoGroupHint::Merge`、かつ直前のノードがあり、封印されていない | 直前のノードに結合 (R2 以降を見ない)。直前のノードが無い、または封印済みなら R1 には当たらず R2 以降で判定する | 1 つの操作が複数の Transaction に分かれる場合に、それらを 1 ノードにする (ハンドラが指定。§7.6) |
| R2 | `tx.origin` が `User` 以外 (`Paste` / `Command` / `Ai` / `Reload` / `Lsp` / `Plugin` / `External` / `Recovery` / `System`)、または `tx.undo_group == UndoGroupHint::Break` | 新ノード。直前のノードと新ノードの両方を封印 | 1 操作 = 1 Undo (FR-206, FR-403, FR-903) |
| R3 | 直前のノードが無い (`undo` が空)、または封印済み | 新ノード | 封印の契機: カーソル移動 (Motion コマンドの実行時に `UndoHistory::seal()`)、Undo / Redo の適用後 (Undo 直後の入力を、戻した位置の直前のノードに結合しない)、保存の開始 (§9.3 手順 3。Undo で保存時点にちょうど戻れる)、ペインのフォーカスの移動 (移る前のペインの文書。分割・閉じる・クリックで移るときも。v0.5、§9.9 の PANE-6 (a))、R2、R6 |
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
  - **既定の上限での判定 (M10、2026-10-31)**: `crates/eve-core/tests/undo_total_budget_default.rs`。既定の 256 MiB のまま、文書ごとの上限 (64 MiB) に掛からない 5 文書 (62 MiB と 60 MiB × 4、合計 302 MiB) を積むと、合計が 256 MiB 以下に戻り、最も大きい文書の古い 1 回から破棄され、それでも超える分は次に大きい文書の 1 回が破棄される。どの文書も直前の操作は取り消せ、通知 (`history_trimmed`) が出る。数百 MB を使うので debug で約 30 秒かかる (CI の `test` ジョブで走る)。
- ノードのコスト = `redo` と `undo` の `Insert` 文字列バイト数 + 固定オーバーヘッド (約 128 B) + Selection のサイズ + (v0.4) ノードが持つ保持方式の記録 (`preserved_removed`、再読込の前後の記録) の元のバイト列のバイト数 (§8.7.1)。`InsertText::Rope` は挿入のバイト数で計上する。v0.3 の `Rope` は文書全体の置き換え (§6.5 の「実装の状態」) にだけ現れ、範囲の端の経路を複製しないので、経路の分 (8 KiB) を足さない。再読込の新全文は現在の文書と節を共有するので多めに数え、旧全文は rope の葉の詰まり具合によって、実際は最大で約 2 倍になり得る (BM-203 で確かめる)。`Slice` と、64 KiB 以上の削除の範囲の `Rope` の「削除バイト数 + 経路オーバーヘッド 8 KiB」は、それを入れる版で適用する (§6.5)。
- 破棄は古い方 (`undo` スタックの底) からで、**最低 1 ノードは残す**。破棄したら `base` を更新し、`saved` の扱いは §7.1 の規則に従う (位置は通番なので補正は要らない)。破棄時にメッセージで 1 回通知する (「Undo 履歴が上限に達したため古い履歴を破棄しました」)。プロセス全体の上限による破棄は、利用者が編集していない文書の履歴も削るが、その文書の dirty 表示は変わらない (§7.1)。
- 大きな単一操作 (100 MB の全選択削除) はそれ自体が上限を超えるが、**直前の 1 操作は必ず Undo 可能** とする (上限より優先)。超過分は次の操作で解消される。
- **Reload / Recovery の全置換** (§9.5 / §10.3) は旧全文を `InsertText::Rope` (旧 rope そのもの。コピーなし) として保持するため、100 MB 文書では一時的にメモリが約 2 倍になり NFR-202 の目標を超え得る。これは「直前 1 操作は予算より優先」の例外が NFR-202 の例外にもなることを意味する。**Reload / Recovery は常に Undo 可** (「再読込で自分の編集が消えた」を取り消せることを優先。明示の再読込 `file.reload` は v0.1 から、フォーカス復帰時の確認と復旧は v0.2 から) とし、このメモリ増は 01 NFR-202 の例外として扱う。旧 rope は Reload のノードが破棄されるまで残る。予算 (文書あたり 64 MiB) を超える大きさなら、次の編集でそのノードが「直前の 1 操作」でなくなった時点で予算の規則により破棄され、解放される。
- OQ-18 (永続化) は当面行わないため、履歴は Document を閉じると消える。

### 7.6 Undo と複数 View・複数カーソル

- Undo の単位は Document 単位 (View 単位ではない)。View A で編集し View B で Undo すると、A の編集が戻り、B の Selection は `selection_before` (A の View のもの) **ではなく** 写像のみ。発行 View が現在もあれば、その Selection を復元する。**発行 View** はノードの `view` (§7.1。記録した Transaction の View)。v0.5 (複数ペイン、§9.9 の PANE-2) からは「現在もある」を「どれかのペインに表示中」と読み、表示中でなければ (別の文書に切り替えたペインの表に残っているだけ・ペインに属さない) 写像だけにする。ノードに View が無い (View を指さずに適用した Transaction) ときは、Undo / Redo を実行した View を発行 View とみなす (v0.4 までと同じ)。実行した View が発行 View でないときは、その Selection は写像だけで、戻した変更の先頭が見えるように最小限スクロールだけする (Redo も同じ。`selection_after` を発行 View に戻す)。
- 複数カーソルの 1 キーストロークは全 Range の変更を合成した **1 つの Transaction** (§4.5 の 1) なので、それだけで 1 ノードになり、`UndoGroupHint::Merge` は要らない。連続入力の結合 (R5 / R7 / R8) は各 Range について条件を調べ、全 Range で満たすときだけ結合する。`Merge` は、1 つの操作をハンドラが複数の Transaction に分けて適用する場合 (前の Transaction の結果を見て次を作る後処理など) に、2 つ目以降に付ける。

---

## 8. 文字コード・BOM・改行コード

### 8.1 Document のメタデータ

```rust
// フィールド名・型の正本は 02 §5.1。本書が加えるフィールドは invalid_bytes と pua_escape の 2 つ (v0.4 の M23 で escape_encoding を足して 3 つ)
pub struct TextFormat {
    encoding: Encoding,              // encoding_rs の静的 Encoding + Utf16Le/Utf16Be (自前)
    has_bom: bool,                   // BOM の種類は encoding から決まる (UTF-8 / UTF-16LE / UTF-16BE)。保存時に同じものを書く
    default_eol: LineEnding,         // 新規に挿入する改行 (LF / CRLF / CR)。読込時は多数派
    eol_stats: EolStats { lf, crlf, cr },  // 読込時に数え、編集で増減 (更新規則は §6.5「eol_stats の更新」。ステータス表示「混在」の判定)
    final_newline: bool,             // 導出値: 最終行が空か (§2.4)。保存時にそのまま書く (FR-114)
    roundtrip: RoundtripSummary,     // §8.7: 再エンコードで元バイト列に戻らない行の検出の結果 (v0.1〜v0.3)。v0.4 は保持方式の要約 (件数と状態、§8.7.1)。記録そのものは Document.preserved に持つ (TextFormat は編集のたびに複製されるので入れない)
    // 以下は本書が 02 §5.1 に加えるもの
    invalid_bytes: u32,              // §8.3 の保持マーカー数。0 でなければ読み取り専用既定 (NFR-304)。読込で数えた数で、編集では増減しない (v0.4 の M23、§8.3.1)
    pua_escape: PuaEscapeMode,       // Enabled / DisabledCollision / DisabledStateful (§8.3)
    escape_encoding: Option<Encoding>, // v0.4 の M23: 本文のエスケープ文字がどの文字コードのバイトか (None か Some(encoding))。保存で元のバイトに戻すのは、書く文字コードがこれと同じときだけ (§8.3.1)
}
pub struct RoundtripSummary {            // v0.1〜v0.3 の実装の形 (eve-encoding の型)。検出して確認に使うだけ (メモリは定数)。v0.4 は保持の状態を足す (§8.7.1)
    lossy_lines: u32,                    // 再符号化で元のバイト列に戻らない行の数 (どの文字コードでも行の単位、§8.7)
    first_offsets: Vec<u64>,             // そのうち先頭 100 行の、行頭の位置 (元のファイルでのバイトオフセット)
    unmappable_lines: u32,               // 表現不能の文字を含む行の数 (lossy_lines には数えない)
    incomplete: bool,                    // 長すぎる行 (4 MiB 超) があり、検査を打ち切った
    preserve: PreserveStatus,            // v0.4 (M22 の 5 で決定): 保持方式の状態。読込のジョブが記録を表にして照合した結果で決める (検査器は既定のまま返す)
}
pub enum PreserveStatus {                // 既定は NotApplicable
    NotApplicable,                       // 対象外: 区分 A の文字コード・新規文書・別の文字コードで保存した後
    Active { unpreserved: u32 },         // 保持が効いている (記録 0 件を含む)。unpreserved > 0 が「保持できなかった箇所がある」
    Unavailable(NoPreserveReason),       // 保持なし。理由は Overflow (上限の超過とメモリの確保の失敗、§8.7.1 の (a)(e)) / Incomplete (b) / VerifyFailed (c) / Recovered (d) / InvalidBytes (不正バイトを含む ISO-2022-JP) / Discarded (違う文字コードで保存して記録を捨てた後に、Undo で前の書式に戻った)
}
// v0.4 の保持方式の記録 (§8.7.1 が正本)。TextFormat には入れず、Document のフィールド preserved: PreservedMap に持つ
pub struct PreservedSpan { range: ByteRange /* 文書の位置。編集で写像 (§8.7.1) */, original: Arc<[u8]> /* 元のバイト列 */, reason: PreserveReason }
pub enum PreserveReason { DuplicateEncoding, DecodeOnly /* 復号専用の文字。ISO-2022-JP の半角カナを含む行も */, StatefulEscape }
pub struct PreservedMap { /* PreservedSpan の表。データ構造の細部 (塊に分けて共有する形など) は M22 の実装の PR で決める */ }
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
7. chardetng 1.0 に sniff 範囲を与えて推定 → Shift_JIS / EUC-JP / ISO-2022-JP / windows-1252 等 (信頼度 中)。**要確認**: chardetng 1.0 (2026-03) のヒントは TLD 形式であり「候補リストを与える API」は無いと理解している。07 §5.2 の `file.detection_candidates` は **chardetng への入力ではなく推定結果の事後フィルタ** (候補外の推定結果は信頼度を下げ、`file.default_encoding` と比較して採否を決める) として定義する。**事後フィルタの手続き** (v0.1): chardetng の推定結果 E について、(i) E が v0.1 の 7 種 (§8.1) に含まれ、かつ `detection_candidates` に含まれる → E を採る (信頼度 中)。(ii) E が 7 種に含まれるが `detection_candidates` に含まれない → 信頼度を「低」に下げ、`file.default_encoding` (`"system"` は 05 §6 で解決した値。以下 D) で sniff 範囲を復号して不正バイトが 0 なら D を、1 以上なら E を採る (どちらも信頼度 低。§8.2.1 の再判定の対象)。(iii) E が 7 種に含まれない (windows-1252・GBK・Big5・EUC-KR・windows-125x 等。FR-110 で明示指定により扱うのは v0.5 以降 (v0.4 から送った、08 §7)) → E を採らず、手順 8 と同じく D で開き、不正バイトは §8.3 で保持する (読み取り専用)。(iii) では情報メッセージ (`info.open.guess_unsupported`、04 §15.2) で推定結果 E の名前と「文字コードを指定して開き直す」(FR-111) を案内する。ロケール由来のヒント (05 §6 の `system`) は TLD 形式に写像できる場合のみ渡す。1.0 で追加された `Iso2022JpDetection` / `Utf8Detection` 等の API は手順 4' / 5 と整合するよう P0 (S-01 (a)) で確認する。chardetng が ISO-2022-JP を返した場合も手順 4' の復号可能性検査を適用し、復号できない箇所は不正バイト扱い (手順 4' の注記)
8. 推定不能 → 設定の既定 (`file.default_encoding`、07 §5.2。既定 UTF-8。`"system"` は 05 §6 のヒント) で開き、不正バイトは §8.3 で保持
```

#### 8.2.1 先頭だけで判定することの危険と再判定規則

- 先頭 64 KiB が ASCII のみで後半に Shift_JIS が現れるファイルは、手順 6 で UTF-8 と誤判定される。
- 対策: デコード中 (チャンク毎) に **UTF-8 として不正なバイト列、または ISO-2022-JP のエスケープシーケンス (§8.2 手順 4') を初めて検出した時点** で、信頼度が「低」なら **その周辺 64 KiB を再判定 (手順 4' → chardetng)** し (ISO-2022-JP のシーケンスが契機で窓が 7 bit のときは手順 4' だけを行い、不合格なら手順 6 と同じく UTF-8 のまま読み続ける。chardetng と手順 7 の事後フィルタは窓が 8 bit バイトを含むときだけ使う。窓の両端がファイルの途中で切れる場合も手順 4' と同じく、窓の末尾の未完のシーケンスは Malformed とみなさない。ISO-2022-JP のシーケンスが契機の場合、窓の先頭はそのエスケープシーケンスの位置にする。シーケンス自身が状態を明示的に切り替えるので、窓の前の状態に依存しない)、別の文字コードが推定されれば **読込ジョブを先頭からやり直す** (1 回のみ)。信頼度「高」(非 ASCII の妥当な UTF-8 を既に見ている) なら不正バイトとして保持する。
- 再判定は稀で、コストは最大で読込 2 回分。ユーザーには「文字コードを再判定しました (Shift_JIS)」と通知する。
- 誤判定はそれでも起き得るため、FR-111 (文字コードを指定して開き直す、v0.1 Must) を回復手段とする。

### 8.3 不正バイト列の保持 (OQ-16 / NFR-304)

**方式: 不正バイト 1 つを 1 つのプライベート用途文字にエスケープし、保存時に元のバイトに戻す。**

**版ごとの範囲**: v0.1 は不正バイトを含む文書を **読み取り専用で開き、内容を可視化する** まで (エスケープは表示と無編集保存のために使う)。解除の手段は FR-111 (文字コードを指定して開き直す)。**編集と保存 (「編集を許可」、隣接エスケープの確認、PUA 衝突時の処理) は v0.4** (正本は §8.3.1、M23)。チャンク境界での元バイトの回収は表示にも必要なので v0.1 で実装する。不正バイトは 1 バイトが UTF-8 で 4 バイトになるため、**不正バイトの比率が 1% を超えるファイルは開く前に「メモリを約 N 倍使う」と警告** する (§11.2、FR-127。`ConfirmAsk::ManyInvalidBytes`。512 MiB の警告とは別の確認 (`ui.confirm.open.invalid_bytes`) で、選択肢は「読み取り専用で開く / 文字コードを指定して開く / 中止」、§9.2 の「確認からの再投入」)。比率は **先頭の sniff 範囲 (§8.2 の 64 KiB。ファイルが小さければ全体)** を判定した文字コードで復号したときの「不正バイト数 / sniff 範囲のバイト数」で決め、本文の構築を始める前に判定する (§9.2 手順 3)。sniff 範囲より後ろの不正バイトでは警告しない (その文書も不正バイトを含むので読み取り専用 (`InvalidBytes`) で開く)。警告の「N 倍」(`memory_factor`) は §9.2 手順 3 の「読込中のメモリ検査」の見積りと同じ式、すなわち **文字コードの倍率 (Shift_JIS・EUC-JP・UTF-16 は 1.5、UTF-8 は 1) × (1 + 3 × sniff 範囲の不正バイトの比率)** で求め、ファイルのバイト数に対するデコード後の本文の大きさの倍率として示す。NFR-202 の目標は妥当なテキストに適用する。

| 項目 | 内容 |
|---|---|
| エスケープ先 | 補助私用面 B の末尾 256 文字: `U+10FF00 + b` (b = 0x00..0xFF)。Rust の `char` として妥当であり、UTF-8 では 4 バイト。サロゲート (Python の surrogateescape 方式) は Rust `String` に入らないため不可 |
| デコード | encoding_rs の `decode_to_string_without_replacement` 相当で **置換なしデコード** を行い、`Malformed(len, consumed_after)` が返るたびに該当 len バイトを個別にエスケープして続行。UTF-8 の場合は `std::str::from_utf8` の `valid_up_to` + `error_len` で同じことを行う。U+FFFD への置換は **行わない** (情報が消える) |
| チャンク境界 | encoding_rs の `Malformed(len, consumed_after)` が示す不正バイトは **直前の入力バッファに属することがある** (公式ドキュメントに明記)。読込ジョブは直前チャンクの末尾 **最大 16 バイト** を保持し、`len + consumed_after` が現チャンクの消費量を超える場合はそこから元のバイト値を回収する。09 L1 に「チャンク境界をまたぐ不正バイト」のプロパティテストを置く (1 バイト単位のチャンク分割でも結果が変わらないこと) |
| 状態付き符号化 | ISO-2022-JP のような状態付き符号化では、生バイトを差し込むとエンコーダの状態 (現在の ESC シーケンス) と食い違い、保存結果が別の意味になる。したがって **状態付き符号化ではエスケープ文字を元のバイトに戻して書くことを無効化** し (`pua_escape = DisabledStateful`)、不正バイトを含む ISO-2022-JP 文書は **常に読み取り専用** とする (02 §13.4 (f)。ISO-2022-JP の保存は v0.1 で行うが、この規則は同じ)。編集したい場合は FR-111 で別の文字コード (UTF-8 等) として開き直すか、不正バイトを含まない文書に直してから扱う。ISO-2022-JP で「不正バイト」になるのは、8 bit バイト、`SO` / `SI` (0x0E / 0x0F)、未対応のエスケープ (`ESC $ ( D` 補助漢字、ISO-2022-JP-2 系)、WHATWG デコーダが誤りとする冗長なエスケープ (出力を挟まない連続エスケープ)、JIS X 0208 の未定義コード、**JIS X 0208 状態・半角カナ状態 (`ESC ( I`) の C0 制御文字 (CR / LF / TAB を含む)**。WHATWG のデコーダは 2 バイト文字・半角カナの状態で 0x21〜0x7E と ESC 以外のバイトを誤りとして 1 バイトずつ消費し、状態はそのまま続く。行末で ASCII 状態に戻さないファイル (RFC 1468 に従わない書き手の出力) では **改行が不正バイトになり、復号結果から改行が消える** (実測、encoding_rs 0.8.35: `1B 24 42 24 22 0A 1B 28 42` → U+3042 の後に `Malformed` (0x0A の 1 バイト)。CR LF なら 2 バイトとも。`ESC ( I` の後の LF も同じ)。このファイルは 7 bit で §8.2 手順 4' を通らないので、自動判定では ASCII (UTF-8) として開く (手順 4' の注記)。**保持と保存の方式** (確定 (ユーザー回答)、OQ-207): 状態付き符号化でも不正バイトは **他の文字コードと同じ PUA エスケープで保持して表示する** (`<0x0A>` 等。消えた改行の位置で行は分かれない。`invalid_bytes` に数え、読み取り専用の理由は `StatefulEncodingWithInvalidBytes`)。不正バイトを文書から落とす実装や U+FFFD に置き換える実装は採らない (どちらも、名前を付けて保存 (読み取り専用でも実行できる、§9.6) で別の文字コードへ書いたときに内容が黙って失われる)。保存は次の 2 つで扱う。(a) **ISO-2022-JP では保存しない**: 書く format の文字コードが ISO-2022-JP (`is_stateful()`) で、文書がエスケープ文字を含む (`invalid_bytes > 0`) とき、core は **保存の連鎖の開始時** (§9.7 の「連鎖の開始」。`file.save`・`file.save_as`・`AskUnsaved` の「保存」・`file.set_encoding` のいずれも。`AskName` で名前を尋ねる前、§9.3 手順 0 の未編集の判定より前) に、保存ジョブも metadata のジョブも投入せずに拒否し、`msg.save.encoding_unsupported_iso2022` (04 §15.2。FR-111 で別の文字コードとして開き直すよう案内する。別の文字コードを指定した保存は (b) でキャンセルだけになるので案内しない) を示して連鎖を中止する (後続 (終了・置き換え・開き直し) も中止し、文書と `TextFormat` は変えない)。名前を付けて保存・文字コードを指定して保存 (§8.8) で ISO-2022-JP を選んだ場合も同じ (他の文字コードで開いた文書のエスケープ文字を ISO-2022-JP で書こうとした場合を含む)。core の拒否が漏れて保存ジョブに届いた場合も、`DisabledStateful` ではエスケープ文字をバイトに戻さず、エスケープ文字 (U+10FF00〜) は ISO-2022-JP のエンコーダで `Unmappable` になるので E1 で表現不能として止まり、何も書かない (二重の防御)。(b) **他の文字コードへの保存** ではエスケープ文字を E4 のとおり表現不能として扱う (§8.6 の E3 で中止。エスケープ文字を含むので「UTF-8 で保存」と「置換して保存」は出ず、キャンセルだけになる)。したがって、この文書から不正バイトの内容が欠けたファイルを書く経路は無い (内容を残したいときは FR-111 で別の文字コードとして開き直す)。表示には `<ESC>` を含むバイト列の可視化 (FR-507) を使う |
| 隣接エスケープの意味変化 (v0.4) | 編集によってエスケープ文字同士、またはエスケープ文字と通常文字が隣接すると、保存後の再読込で **別の文字として復号される** ことがある (例: 孤立バイト `E3 81` の後ろに `82` が置かれると UTF-8 では「あ」になる)。v0.4 の M23 で決めた扱い (正本は §8.3.1 の「つながりの検出」): 保存ジョブは **書く前の乾式走査** で、エスケープ文字の並びごとに、戻したバイト列とその直後の 1 単位を同じ文字コードで復号して本文に戻るかを調べ (局所の検査)、戻らない並びがあれば (BOM の無い文書の先頭が、書く文字コード自身の BOM の並びになる場合を含む) 何も書かずに `JobResult::EscapeMerge` を返す。core は何が何に変わるかと変わる範囲の大きさを示して **書く前に確認する** (`ui.confirm.save.escape_merge`。既定はキャンセル。承認すればそのまま保存する。利用者が意図して並べた (壊れたファイルの修復) 可能性があるため)。v0.3 までの仕様にあった「`SaveWarning::EscapeMerge` で保存の後に警告し、確認への切り替えは v0.5 の設定」は採らない (01 §5.5 の NFR-304 AC-5「書き込む前に警告」と合わない) |
| 衝突 | 元のファイルが **真に** `U+10FF00..U+10FFFF` を含む場合、保存時に区別できない。読込時にデコード結果にこの範囲の文字が現れたら `TextFormat.invalid_bytes` とは別に `pua_escape = DisabledCollision` とし、(a) 不正バイトが 0 ならエスケープ機構を無効化 (この文書では PUA 文字をそのまま扱う)、(b) 両方あるなら **読み取り専用で固定** し警告する。(b) の文書はどの文字コードでも保存しない (エスケープ文字を `Reject`。名前を付けて保存も E3 のキャンセルだけで、FR-111 で別の文字コードとして開き直す。v0.4 の M23 で決定、§8.3.1。v0.3 までの実装は `AsText` で、不正バイトを私用文字の UTF-8 として書いていた)。v0.4 の M23 でも解除できない。実運用上ほぼ起きない |
| 表示 | FR-507 に従い `<0x9A>` のような記号で描画 (幅 6 セル)。Frame 生成時に Span として展開する (§14.2)。記号にする条件は §8.3.1 の「表記にするかどうかの条件」(`pua_escape != DisabledCollision`。状態付き符号化 (`DisabledStateful`) も、バイトに戻さないのは保存だけで、表示は他の文字コードと同じ。`invalid_bytes` に依らない) |
| 編集 (v0.4) | 既定は読み取り専用。v0.4 でユーザーが「編集を許可」(`file.toggle_read_only` で `InvalidBytes` を解除する。確認を 1 回出す。§8.3.1 / §9.6) すると通常の文字として扱える (前後が必ず書記素の境界の 1 文字 (§13.1)、削除可、コピー可)。コピーした場合、システムのクリップボードには `<0x9A>` の表記文字列を書く (バイトは書けないため)。内部レジスタはエスケープ文字のまま持ち、同じ文字コードの不正バイトを含む文書への貼り付けではバイトのまま運ぶ。外から入る `U+10FF00..U+10FFFF` は `<U+10FFNN>` の文字列にする。規則の正本は §8.3.1 の「本文に入れる規則」。v0.1〜v0.3 は `InvalidBytes` を解除できない (`file.toggle_read_only` (v0.2) の対象外) |
| 保存 | エンコード時に `U+10FF00..FF` を見つけたら **そのバイト値をそのまま出力** する。これにより無編集保存でバイト一致 (NFR-303 の例外ではなく、保持方式で満たす) |
| 検索 | 正規表現 (v0.2) では `\x{10FF9A}` で一致させられる。UI では `\x9A` 風の糖衣を検討 (04/12) |
| 他の文字コードへ変換保存 | エスケープ文字は表現不能として FR-112 の警告対象 (位置を示す) |

> Latin-1 として開き直すことは FR-111 で行える。

#### 8.3.1 不正バイトを含む文書の編集 (v0.4、M23)

本節が v0.4 の不正バイトを含む文書の編集 (NFR-304 の v0.4 の部分。01 §5.5 の NFR-304 AC-4〜AC-6、[OQ-16](10-decisions.md#oq-16-不正バイト列を含むファイルの編集方式) の (c)) の正本である。細目のうち PO が決めた点は [OQ-217](10-decisions.md#29-oq-176-以降-並行修正での追加起票) にある (確定 (ユーザー回答、2026-11-12): 10 点とも推す案のとおり。本節はその内容。仕様との整合・データの損失・利用者の 3 つの視点のレビューを反映)。上の表の「隣接エスケープの意味変化」「編集」「衝突」の行と、§8.6 E2 の注記にあった M23 で決めること ((a) 記録を渡さない保存のつなぎ目、(b) 意味変化と確認 (ii) の関係) を、ここで決める。不正バイトを含む ISO-2022-JP (`StatefulEncodingWithInvalidBytes`) は、今のまま読み取り専用で固定し、ISO-2022-JP では保存しない (上の表の「状態付き符号化」の行、OQ-207)。

**エスケープ文字の由来の文字コード** (`TextFormat.escape_encoding: Option<Encoding>`、§8.1): エスケープ文字そのものには印が無いので、本文のエスケープ文字が **どの文字コードのバイトか** を文書が持つ。値は `None` か `Some(format.encoding)` のどちらかで (不変条件。`Some` なら `pua_escape == Enabled` で、`encoding` は状態付きでない)、次の時点で決める。

| 時点 | `escape_encoding` |
|---|---|
| 読込 (開く・再読込・文字コードを指定して開き直す) | `pua_escape == Enabled` で不正バイトがあれば (`invalid_bytes > 0`) `Some(encoding)`。ほかは `None` (状態付きの文書・衝突の文書も `None`) |
| 違う文字コードでの保存の成功 (`file.set_encoding`・E3 の「UTF-8 で保存」。§8.8) | `None` にする (保持方式の記録を捨てるのと同じ場所、§8.7.1)。エスケープ文字を含む文書は違う文字コードで保存できない (E4) ので、このとき本文にエスケープ文字は無い。この後に Undo でエスケープ文字が本文に戻っても、元のバイトには戻さない (`Reject`。E3 のキャンセルだけになる) |
| 同じ文字コードでの保存の成功 | 変えない |
| 復旧の適用 (§10.3) | 復旧は本文の全体を置き換えるので、エスケープ文字の意味はディスクではなくスワップ (manifest) に従う。manifest の `pua_escaped` が真のとき (衝突の文書は次の行): manifest の `encoding` が文書の文字コードと同じで `pua_escape == Enabled` なら `Some(encoding)` (ディスクのファイルが不正バイトを含まなくなっていても。無題の文書はスワップの形式を引き継ぐので `Some(manifest の encoding)`。どちらも状態付きの文字コードを除く)、違えば `None` (エスケープ文字は `Reject`)。`pua_escaped` が偽なら `None` (スワップの本文のエスケープ文字はバイトではない。前の値が `Some` でも)。**`Some` になった文書には、読込と同じく内容に由来する読み取り専用の理由 `InvalidBytes` を付ける** (読込の「`Some` ⇔ 不正バイトがある ⇒ `InvalidBytes`」と揃える。「本文に入れる規則」と「編集を許可」が入るまで、外から入れた私用文字が元のバイトに戻されないように)。復旧の Undo では、`escape_encoding`・`pua_escape`・`InvalidBytes` の有無が前の値に戻り、Redo で後の値になる (`FormatChange::Recovery` に持たせる) |
| 衝突の文書への `pua_escaped = true` の復旧 | 衝突の文書 (`DisabledCollision`。復旧を適用できるのは編集できる (a)) は、`pua_escape` を `Enabled` にしてから上の行のとおりに決める (スワップの私用文字は不正バイトの印。衝突の文書の扱い (`AsText`) のままだと、不正バイトを私用文字の UTF-8 として書く)。状態付きの文書 (`DisabledStateful`) の `pua_escape` は変えない (`None`)。復旧の Undo で `pua_escape` も前の値に戻る |
| 再読込の Undo / Redo | 書式 (`TextFormat`) ごと戻る (§7.2 の `format_change`) |

- `invalid_bytes` は読込で数えた数のままで、編集では増減しない (エスケープ文字を全部消しても 0 にならない)。ステータスバーには数を出さない (v0.3 と同じ)。使い道は、読込のときの読み取り専用の理由と、衝突の文書の (a) / (b) の区別 (下の表) と、ISO-2022-JP の保存の拒否 (§9.7 の「連鎖の開始時の拒否」) だけで、保存の扱いは `escape_encoding` で決める。manifest の `pua_escaped` (02 §11.2.1) は `escape_encoding == Some(encoding)` (不変条件の下では `escape_encoding.is_some()` と同じ。不変条件が破れていても、別の文字コードのバイトを「バイト」と書かない) を書く。
- 不変条件の補足: `Some(e)` の `e` は状態付きの文字コードでない (読込では状態付きの文書は `DisabledStateful` なので `None`、復旧でも状態付きの文字コードは `Some` にしない)。`save_escape_policy` は、これが破れていても状態付きの文字コードへは `Restore` を返さない (二重の防御)。
- 表の細目 (M23 の実装で決めた): 違う文字コードでの保存の成功では、履歴の復旧のノードが持つ前後の値も `None` にする (Redo で元の文字コードの `Some` が戻らないように。再読込のノードは文字コードと組で書式ごと戻るので変えない)。無題の文書の復旧でスワップの文字コードを写すとき、前の値が `Some(別の文字コード)` なら先に `None` にする (復旧の Undo で文字コードは戻らないので、前の値も今の文字コードに合わせる)。写した形式は、適用が失敗したら元に戻す。本文が同じ復旧 (全置換が恒等) は `escape_encoding` も変えない (本文が今のままなので、エスケープ文字の意味も今の値のまま)。
- 古い manifest の読み方: `pua_escaped` のキーが無い・値が読めないときは偽とする (02 §11.2.1)。v0.3 までの書き手は `invalid_bytes > 0` を書いていたが、その文書は常に読み取り専用で、スワップは読み取り専用の文書を書かないので、`pua_escaped = true` の古いスワップは無い。

**文書の種類と保存の扱い** (`TextFormat::save_escape_policy(out)`。v0.3 の「今の文字コード == 書く文字コード かつ `invalid_bytes > 0`」を改める):

| 文書 | 条件 | エスケープ文字の意味 | 書く文字コード == `escape_encoding` | それ以外の保存 |
|---|---|---|---|---|
| **バイトの文書** | `pua_escape == Enabled`、`escape_encoding == Some(e)` | `e` の 1 バイト | 元のバイトに戻す (`Restore`) | 表現不能 (`Reject`、§8.6 E4) |
| バイトでない文書 | `pua_escape == Enabled`、`escape_encoding == None` (不正バイトの無い文書、違う文字コードで保存した後、文字コードの違う復旧) | 意味を持たない (下の「本文に入れる規則」で、外からは入らない) | — | 表現不能 (`Reject`。v0.3 の暫定 (docs/review/v0.3-findings.md の SAF-02b) を正式の規則にする) |
| 衝突の文書 (a) | `pua_escape == DisabledCollision`、`invalid_bytes == 0` (元のファイルが真にこの範囲の文字を含む。編集できる) | 通常の文字 | — | 通常の文字として符号化 (`AsText`。表せない文字コードでは表現不能) |
| 衝突の文書 (b) | `pua_escape == DisabledCollision`、`invalid_bytes > 0` (不正バイトと真の私用文字の両方。読み取り専用で固定、上の表の「衝突」(b)) | 区別できない | — | **表現不能 (`Reject`)**。どの文字コードでも保存できない (名前を付けて保存も E3 のキャンセルだけ。FR-111 で別の文字コードとして開き直す)。v0.3 の実装は `AsText` で、同じ文字コードの名前を付けて保存で不正バイトを私用文字の UTF-8 として書いていた (直す) |
| 状態付き・不正バイトあり | `pua_escape == DisabledStateful`、`invalid_bytes > 0` (読み取り専用で固定、`StatefulEncodingWithInvalidBytes`) | 1 バイト (表示と退避のため) | ISO-2022-JP では保存しない (上の表の「状態付き符号化」) | 表現不能 |
| 状態付き・不正バイトなし | `pua_escape == DisabledStateful`、`invalid_bytes == 0` (編集できる ISO-2022-JP の文書) | 意味を持たない (外からは入らない) | — | 表現不能 |

- 状態付きの行の読み方: 状態付きの文書の `escape_encoding` は常に `None` なので、列「書く文字コード == `escape_encoding`」に当たる保存は無い。「状態付き・不正バイトあり」の行のその列は、代わりに「書く文字コードが ISO-2022-JP のとき」の扱い (保存の連鎖の開始時に拒否する) を書いたもので、`save_escape_policy` はどちらの行もすべての文字コードで `Reject` を返す。

- **表記にするかどうかの条件** (表示 (FR-507、§14.2)・システムのクリップボードへの文字列化・本文に入れるときの置き換えで共通): `pua_escape != DisabledCollision`。衝突の文書だけは私用文字をそのまま扱い、ほかはすべて `U+10FF00..U+10FFFF` を印として扱う。v0.3 までの表示の実装 (`layout/units.rs` の `esc_enabled`) は `Enabled` だけを見て、ISO-2022-JP の不正バイトを `<0x0A>` にしていなかった。これは実装の不具合で、v0.4 で直した (§8.3 / OQ-207 / NFR-304 AC-3 のとおり)。

**編集を許可** (`file.toggle_read_only`。新しいコマンドは作らない):

- **解除できる文書**: 読込が済み (`load_state == Ready`)、理由がすべて `FileAttribute` / `UserRequested` / `InvalidBytes` のどれかで、`InvalidBytes` を含むなら `pua_escape == Enabled` (= `escape_encoding` が `Some` の文書。実装は両方を見る。文字コードの違う復旧の後など、`InvalidBytes` が付いていて `escape_encoding` が `None` の文書は解除できず、編集の拒否は一般の `msg.edit.read_only` (理由は「不正なバイトを含むファイル」) で切替の案内をしない)。衝突の文書 (b) (理由は `InvalidBytes` だが `DisabledCollision`)・`NoWritePermission`・`StatefulEncodingWithInvalidBytes`・`Binary`・`LargeFileViewer`・`LoadIncomplete` があれば解除できない (§9.6 の表)。不正バイトの比率の確認 (FR-127) で「読み取り専用で開く」を選んだ文書 (理由は `InvalidBytes` だけ) も解除できる (その確認はメモリの警告で、編集の可否を決めたものではない)。when 条件 `doc.read_only_releasable` (07 §7.3) はこの条件を返す。パレットで淡色のときの理由は、解除できない理由 (`ui.read_only_reason.*`) を示す。
- **確認** (`ui.confirm.read_only.invalid_bytes`、04 §15.2 の案): 理由に `InvalidBytes` を含む文書で実行すると、外す前に 1 回出す。本文は短くし (文書の名前・文字コード・不正なバイトの数・最初の不正なバイトの位置 (行・桁)、「それぞれを `<0xNN>` の 1 文字として消したり動かしたりできる」)、要点を `details` に並べる: (1) 保存すると残っている `<0xNN>` は元のバイトに戻る、(2) 消すとそのバイトはファイルから無くなる (保存の前なら元に戻すで戻せる)、(3) 隣の文字とつながると開き直したときに別の文字になる (保存の前に確認する)、(4) 他のアプリに貼ると `<0xNN>` という文字列になる。さらに「文字化けして見えるときは、編集せずに文字コードを指定して開き直してください」と言う (判定が外れただけのファイルで編集を許可すると、打った部分だけが誤った文字コードで書かれる)。選択肢は `[A] 編集を許可` (★)・`[E] 文字コードを指定して開き直す` (`file.reopen_with_encoding` を実行する。`ui.confirm.open.invalid_bytes` の `[E]` と同じ意味)・キャンセル (`Esc`)。比率が 1% を超えた文書でも既定は変えない (OQ-217)。
  - 最初の不正なバイトの位置は、確認を出すときに本文の UTF-8 で `F4 8F BC`〜`F4 8F BF` (`U+10FF00..` の先頭 3 バイト) を先頭から探して求める (`memchr` 相当。最初に見つけた所で止まる。100 MB の文書で 10 ms 程度の見込みで、利用者の 1 回の操作の費用として許す。見つからなければ位置を省く)。
  - 許可すると `InvalidBytes`・`FileAttribute`・`UserRequested` を外し (`FileAttribute` を外した文書の上書き保存の確認は §9.6 の M9d の規則のまま)、解除の記録 `Document.invalid_bytes_released = true` を立て、`info.read_only.released_invalid_bytes` (「編集を許可しました (不正なバイトは <0xNN> の 1 文字として編集できます)」) を出す。キャンセルでは何も変えない。理由に `InvalidBytes` を含まない文書は、v0.2 と同じく確認なしに外す。
- **解除の持ち越し** (`Document.invalid_bytes_released`。`attribute_released` の先例に倣う): 解除は文書の状態で、Undo の対象外 (§7.2)。**同じ文書の再読込** (`file.reload`、外部変更からの再読込。§9.5) では、新しい読込に不正バイトがあっても `InvalidBytes` を付けず、解除を持ち越す (確認の内容はどのバイトかに依らない一般の説明なので、もう一度聞く利益が小さい。付け直すと、再読込の Undo が読み取り専用で実行できず (`info.reload.done_read_only`)、確認が連鎖する)。**文字コードを指定して開き直す** (§8.8)・**新しく開く**・**復旧の読込** では付け直す (記録を消す)。もう一度 `file.toggle_read_only` を実行すると `UserRequested` を付ける (§9.6 のまま。解除の記録は残す)。再読込の Undo / Redo は前後の理由ごと戻る (§7.2 の `format_change`。ただし下の細目 (b))。細目 (M23 の 6 本目で決めた): (a) 再読込で持ち越すのは、新しい読込がバイトの文書 (`pua_escape == Enabled` で `escape_encoding` が `Some`) のときだけ (読み直した内容が私用文字と衝突した文書 (b) になったら、解除できない文書なので `InvalidBytes` を付ける)。再読込の開く前の確認 (不正バイトの比率、FR-127) で「読み取り専用で開く」を選んだとき (`OpenOptions.read_only_requested`) も持ち越さず、記録を消す (利用者が読み取り専用を選んだので、再読込の Undo / Redo でも外さない)。(b) 解除の記録がある文書では、再読込・復旧のノードの Undo / Redo で書式を戻しても、戻した後がバイトの文書 (`pua_escape == Enabled` で `escape_encoding` が `Some`) なら `InvalidBytes` を付けない (違う文字コードで保存した後の復旧の Redo のように `None` なら、ノードの後の理由のまま付ける) (解除は Undo の対象外、§7.2。解除の後に再読込・復旧を Undo して読み取り専用に戻ると、Undo の続きができず確認が連鎖する)。(c) 復旧の適用では、書式を変える前に記録を消す (`escape_encoding` が `Some` になれば `InvalidBytes` が付く。`None` の復旧でも記録は消す)。適用に失敗したら記録も戻す。(d) 確認の `[E]` は、パスの無い文書 (無題の文書への復旧で `InvalidBytes` が付いた文書) では出さない (`file.reopen_with_encoding` が使えないため)。 (e) 確認の本文の不正なバイトの数は、本文のエスケープ文字の数 (最初の位置と同じ 1 回の走査で数える。本文の全体を走査するので、最初に見つけた所では止まらない)。`invalid_bytes` は読込の時点の数で、解除・編集・復旧の後の本文とは合わないため使わない。
- **読み取り専用のときの案内** (`msg.edit.read_only` の hint を理由ごとに分ける、04 §15.2 の案): 不正バイト (解除できる) には「読み取り専用の切替で、確認の後に編集できる」を先頭に言い、衝突の文書 (b)・ISO-2022-JP・バイナリでは「切替で編集できる」と言わない。衝突の文書 (b) の理由の文言は `InvalidBytes` と分ける (`pua_escape` で選ぶ)。

**書記素と単語** (§13.1 / §13.3): エスケープ文字の前後は常に書記素の境界とする。UAX #29 の GB9 では私用文字の直後の結合文字・異体字セレクタ・ZWJ は同じ書記素に入るが、エスケープ文字は 1 バイトを表すだけで基底の文字ではない。境界を置かないと、`<0x9A>` の上の `Backspace` が直後の結合文字まで消し、カーソルを間に置けない (表示は v0.3 から 1 文字ずつ `<0xNN>` にしている)。この規則は eve-text の書記素の判定で `U+10FF00..U+10FFFF` に一律に当てる (文書の `pua_escape` を見ない。衝突の文書の真の私用文字の直後の結合文字も別の書記素になるが、実運用上ほぼ起きない)。単語の境界 (§13.3) では、連続するエスケープ文字の並びを 1 つの単語とする (文字種「不正バイト」。どの文字種ともつながらない)。カーソルと選択の見え方は 04 §5 (カーソルは `<0xNN>` の記号の先頭のセルに置き、記号の中には止まらない。選択は記号の全体を反転する)。

**本文に入れる規則** (コピー・貼り付け・入力・置換。NFR-304 AC-4。黙ってバイトにも別の文字にもしない):

- **挿入の入口**: 次の入口のすべてで、改行の正規化 (§8.4) と同じ段階 (挿入するテキストを Transaction にする前) に下の規則をかける。規則は `command::insert_rule` の 1 か所にあり、各入口がそれを呼ぶ (M23 の 5 本目。改行の正規化は、これまでどおり入口 (1) だけ)。(1) `command::edit::paste_text` (`edit.paste` の応答 `InputEvent::ClipboardContent` と、端末からの貼り付け `InputEvent::Paste`)、(2) `edit.insert_text` (文字の入力 `InputEvent::Text` もこれを通る。マクロ・駆動 API からの実行を含む)、(3) 置換 (`search.replace` と、すべて置換のジョブ `ReplaceAll` に渡す置換の文字列。§12.5。文字どおりの部分だけ。`$1` / `${name}` の参照が運ぶ文字は同じ文書のもの。`$1` / `${name}` を展開する前に文字どおりの部分を表記にする (表記は `$` を含まないので展開は変わらない)。すべて置換の `info.paste.pua_as_text` の件数は、置換 1 つあたりの数 × 置換した件数で、`info.search.replaced` の後に出す)、(4) v1.L の Proposal の適用 (その版の仕様で確かめる)。
- **2 つの経路**: **(1) 内部のコピー**: 同じ文字コードのバイトの文書から内部レジスタで運んだもの。挿入先が同じ文字コードのバイトの文書 (`escape_encoding` が同じ `Some`) なら、エスケープ文字のまま (バイトのまま) 入れ、件数を `info.paste.as_bytes` で知らせる。**(2) 外からの文字列** (上の入口のうち、(1) に当たらないもの): 挿入先が `pua_escape != DisabledCollision` の文書なら、その中の `U+10FF00..U+10FFFF` の各文字を、コードポイントの表記の文字列 `<U+10FFNN>` (ASCII の 10 文字。§13.2 の書き分け「バイトは `<0xHH>`、コードポイントは `<U+XXXX>`」に合わせる。外から来たものはバイトではないので、バイトがあったと誤解させない) に置き換えて入れ、件数を `info.paste.pua_as_text` で知らせる (拒否はしない)。衝突の文書 (a) にはそのまま入れる (通常の文字)。
- **表記の文字列をバイトに戻すことはしない**: 利用者が `<0x9A>` と打った文字列・他のアプリから貼った `<0x9A>` は、ただの 6 文字である。バイトのまま運ぶ条件は、下の「内部レジスタとの照合」だけ。
- **コピー** (`clipboard.copy` / `clipboard.cut`、§12.6): 内部レジスタ (`ClipboardSource`。eve-core の型で、境界型 (eve-view) は変えない) は範囲をエスケープ文字のまま持ち、コピーした時点の状態として 3 つを覚える: `notation: bool` (コピー元が `pua_escape != DisabledCollision`。表記にするか)、`escape_origin: Option<Encoding>` (コピー元の `escape_encoding`)、`indistinct: bool` (コピー元が衝突の文書 (b)。不正バイトと真の私用文字を区別できない)。文書の外の文字列のコピー (メッセージ履歴など) は `notation = false`・`escape_origin = None`・`indistinct = false` とする。システムのクリップボードへ渡す文字列 (`Effect::SetClipboard` の `Inline`、`Handle` を Host が文字列化したもの、OSC 52、駆動 API の `editor/effects` の通知) は、`notation` が真ならエスケープ文字を表記の文字列 `<0xNN>` にしたもの (表示と同じ。衝突の文書はそのまま)。文字列化は `ClipboardSource::to_text` 1 つで行い、Host は Handle の文字列化でそれを呼ぶ (02 §5.4 の `take_clipboard`。Host は変換の規則を持たない)。
- **内部レジスタとの照合** (入口 (1) の貼り付け): 届いた文字列と、内部レジスタを `to_text` で文字列にしたものを、**両側に同じ改行の正規化 (`\r\n`・`\r` を `\n` に) をかけてから** 比べる。先頭から比べて最初の違いで打ち切る (O(短い方))。レジスタが大きいとき (64 KiB 以上) もメインスレッドで比べながら行う (貼り付け自体が O(貼る量) なので、費用の桁は変わらない)。届いた文字列が NUL を含まず、内部レジスタの文字列を (改行を正規化したうえで) 最初の NUL で切ったものと同じときも一致とみなし、内部レジスタの全体を入れる (Windows のクリップボードの取得は最初の NUL で切れる、05 §8.1)。
  - **一致**: 内部のコピーとみなす。`escape_origin` が挿入先の `escape_encoding` と同じ `Some` なら、内部レジスタの内容 (エスケープ文字のまま) に、設定どおりの改行の正規化 (§8.4。`editor.paste_normalize_eol = false` なら正規化しない) をかけて入れる (経路 (1))。そうでなければ届いた文字列 (NUL で切れて一致したときは内部レジスタを `to_text` で文字列にしたもの) を経路 (2) で入れ (表記にする文書からのコピーなら、エスケープ文字は既に `<0xNN>` になっている)、内部レジスタが `notation` でエスケープ文字を含めば、その数を `info.paste.escape_as_text` で知らせる (理由: 貼り付け先ではバイトとして扱えないため。コピー元がバイトでない文書・状態付きで不正バイトのある文書 (`escape_origin` が `None`) でも知らせる)。内部レジスタが `indistinct` なら、挿入先が衝突の文書 (a) でも `U+10FF00..` を `<U+10FFNN>` にして `info.paste.pua_as_text` で知らせる (区別できないので、黙ってどちらかの意味にしない)。`editor.paste_normalize_eol` は 07 §5.13 の冒頭の「版の注記が無ければ v0.2 で公開」に反して、v0.4 でもまだ公開していない (定数 `true`。`false` の場合は純粋な関数の性質テストで確かめる)。
  - **不一致**: 経路 (2)。ただし、内部レジスタが `notation` でエスケープ文字を含み、届いた文字列に `<0x[0-9A-F]{2}>` の形があるときは、それが文字列として入ったことを `info.paste.notation_as_text` で知らせる (クリップボードの内容がエディタ内のコピーと違うため。黙って文字列にしない)。知らせが 2 つになるとき (`notation_as_text` と `pua_as_text` など) は、両方を順に出す (先のものはメッセージ履歴に残る)。
  - **届かなかったとき・書き出しに失敗したとき**: 届いた文字列を読めなかったとき (`None`) は、内部レジスタを使って一致とみなす。システムのクリップボードへの書き出しが失敗した・上限を超えた (02 §6.3 / 05 §8.4) ときは `msg.clipboard.fallback` で知らせ、照合の規則は変えない (書き出しに失敗した直後に、システム側に残っていた古い内容を貼ると、その内容が入る。v0.3 と同じ)。書き出しの失敗の後の貼り付けで内部レジスタを使う方式は後の版の候補 (§23)。
  - 一致の判定は文字列だけで行うので、他のアプリで同じ文字列をコピーし直した場合も内部のコピーとみなす (違いが出るのは表記の部分がバイトになるかどうかだけで、それが起きるのは同じ文字コードのバイトの文書に貼ったときだけ)。OS のクリップボードの変更番号 (Windows の `GetClipboardSequenceNumber` など) で確かめる方式は後の版の候補 (§23)。

| コピー元 \ 貼り付け先 | 同じ文字コードのバイトの文書 | 別の文字コードのバイトの文書・バイトでない文書・状態付きの文書 | 衝突の文書 (a) | 他のアプリ |
|---|---|---|---|---|
| バイトの文書 (エスケープ文字) | **バイトのまま** + `info.paste.as_bytes` (切り取り → 貼り付けで不正バイトが移る) | `<0xNN>` の文字列 + `info.paste.escape_as_text` | `<0xNN>` の文字列 + `info.paste.escape_as_text` | `<0xNN>` の文字列 |
| 表記にするがバイトでない文書のエスケープ文字 (バイトでない文書に残ったもの・状態付きで不正バイトのある文書。`escape_origin` が `None`) | `<0xNN>` の文字列 + `info.paste.escape_as_text` | 同左 | 同左 | `<0xNN>` の文字列 |
| 衝突の文書 (a) (真の私用文字) | `<U+10FFNN>` の文字列 + `info.paste.pua_as_text` | 同左 | そのまま | そのまま (私用文字) |
| 衝突の文書 (b) (不正バイトと真の私用文字。区別できない) | `<U+10FFNN>` の文字列 + `info.paste.pua_as_text` | 同左 | 同左 | そのまま (私用文字) |
| 他のアプリ・端末の貼り付け・入力 (`U+10FF00..` を含む) | `<U+10FFNN>` の文字列 + `info.paste.pua_as_text` | 同左 | そのまま | — |

- **文書の中の操作** (行の複製・削除・上下移動 (§15.4)、インデント、改行コードの変換 (§8.4)、すべて置換の `$1` 等の参照、Undo / Redo、E3 の置換) は本文の文字をそのまま運ぶので、エスケープ文字はバイトのまま。
- **入力欄** (検索欄・置換欄・プロンプト) への貼り付けは、届いた文字列のまま入れる (入力欄は文書ではないので照合しない)。**検索欄** への貼り付けで、届いた文字列が内部レジスタと一致し、内部レジスタが `notation` でエスケープ文字を含むときは、`info.paste.search_notation` で「`<0xNN>` の文字列は不正なバイトに一致しない。正規表現の `\x{10FFNN}` で探せる」と知らせる (文言が検索の説明なので、置換欄・プロンプト・パレットの欄では出さない)。置換欄に入った `U+10FF00..` は、置換の適用のときに経路 (2) で表記にする。

**つながりの検出** (`EscapeMerge`。NFR-304 AC-5):

- **定義**: バイトの文書を `escape_encoding` と同じ文字コードで書く保存 (`Restore`) の出力を、同じ文字コードを指定して開き直す (FR-111。§8.2 手順 2 のとおり、先頭が指定した文字コードの BOM と一致すれば読み飛ばす) と、本文と違う文字になる箇所。エスケープ文字を元のバイトに戻すことでだけ起きる (エスケープ文字でない部分は、E1 の照合と §8.7.1 の不変条件で本文に戻る)。例: UTF-8 で `<0xE3><0x81><0x82>` が並ぶ (「あ」になる)。Shift_JIS で `<0x81>` の直後に `@` (0x40) を置く (`0x8140` = 全角の空白)。EUC-JP で `<0x8F>` の直後にかな・漢字を置く (補助漢字の 3 バイト列の頭になり、後ろのバイトを巻き込む)。UTF-16 で奇数個のエスケープ文字の並びを動かす (後ろが 1 バイトずれる)。
- **復号する範囲**: 確認 (ii) と局所の検査は、**書いた BOM を除いた出力** を復号する (BOM 付きの文書を開き直すと BOM は読み飛ばされるため。M23 の前の `Verify` は `Decoder::new(encoding, false)` で BOM を読み飛ばさなかったので、確認 (ii) を広げると BOM 付きの不正バイトを含む UTF-8 / UTF-16 の文書が毎回 `VerifyFailed` になるところだった。M23 の 3 本目で、書いた BOM のバイト数を読み飛ばしてから復号するようにした)。
- **局所の検査**: 本文の、連続するエスケープ文字の並び (以下「並び」。改行や通常の文字で区切られる) ごとに、並びを戻したバイト列と、その直後の 1 単位 (次の 1 文字を符号化したバイト列。次の文字が保持方式の記録 (§8.7.1) の範囲にあり、その記録の元のバイト列を書くなら、記録の元のバイト列の全体。文書の末尾なら無し) を、その文字コードの初期状態の復号器で復号し (末尾は `last = true`)、並びのエスケープ文字と直後の単位の文字に戻るかを比べる。戻らなければ、その並びを `EscapeMerge` の 1 件とする。直前の文字は調べない (通常の文字の符号化は完全な列で、復号器を初期状態に戻す。記録の元のバイト列も §8.7.1 の不変条件で初期状態で終わる。どちらも後ろのバイトとはつながらない)。この検査は「出力の全体を復号すると本文に一致する」ことと同値である (どの並びもつながらなければ、各並びとその直後の単位の後で復号器は初期状態にあるので、出力の全体が本文に戻る。WHATWG の復号器は先読みをせず、誤りのときは今のバイトを読み直すだけなので、直後の 1 単位までを見れば足りる)。**M23 の仕様のレビューで、総当たり (長さ 3 まで) と乱数 (5 つの文字コードで計 5,000 万件以上) で確かめた (見逃し 0・誤検出 0・区切った確認 (ii) の誤った不一致 0。記録を使う保存と「保持なし」の保存で判定が変わらないことも)。09 の性質テストで固定する**。費用は並びのバイト数と直後の単位の長さの和に比例し、復号器を 1 つ使い回す。
- **文書の先頭の BOM**: 書く文字コード自身の BOM の並び (UTF-8 の `EF BB BF`、UTF-16LE の `FF FE`、UTF-16BE の `FE FF`) が、BOM の無い文書の出力の先頭にエスケープ文字を戻したバイトでできる場合 (エスケープ文字だけの並び、エスケープ文字と通常の文字の組 (UTF-16LE の `<0xFF>` と `þ` など)) は、上の局所の検査が見つける (初期状態の復号器では U+FEFF になって本文と違う。同じ文字コードを指定して開き直すと BOM として読み飛ばされる)。別の文字コードの BOM の並びは検出しない (下の「検出しないもの」)。M23 の仕様のレビューの時点で置いた「出力の先頭が BOM の並びと一致すれば、文書の文字コードに依らずつながりとする」規則は、M23 の 3 本目のレビューで外した (別の文字コードの BOM の並びで始まるファイルを、未編集のまま名前を付けて保存しても止まり、v0.3 からの後退になったため)。
- **示す位置と変わる範囲**: 1 件の位置は、局所の復号で **最初に違いが出たエスケープ文字** (並びの先頭とは限らない。違いが直後の 1 単位で初めて出た (並びのエスケープ文字はどれも元に戻った) ときは、並びの最後のエスケープ文字)。全部の件数を数え (`total`)、先頭 100 件を持つ (E1 の表現不能と同じ上限)。1 件は (最初に違いが出たエスケープ文字の位置、そこから直後の 1 単位の終わりまでの文字数 (強調の範囲)、その範囲を局所に復号した文字列の先頭 16 文字 (一覧の「→」の右)) を持つ。つながった後の復号は直後の 1 単位で終わらず、後ろへずれ続けることがある (Shift_JIS / EUC-JP は次の ASCII のバイト (改行を含む) まで、UTF-16 は文書の末尾か次に奇数個の並びがある所まで。例: UTF-16 の文書の先頭に不正バイトを 1 つ置くと、ファイルの全部の文字が別の文字になり、改行が 0 になる。Shift_JIS で `<0x81>` の後に全角文字が 5,000 字続くと 5,001 字が変わる)。そこで **変わる範囲の大きさ** を乾式走査で求める: つながりを 1 件以上見つけたときだけ、区切らない復号器 (開き直しと同じ 1 本。出力の先頭から始めるときは先頭の BOM を読み飛ばす) を、つながる並びの先頭から符号化の側の単位 (本文の 1 文字、または記録) ごとに進め、範囲の中の最後のつながりの位置より後ろの単位のうち、復号器がその単位で出したものがその単位の文字で終わり、復号器に持ち越しが無くなった最初の単位の前で範囲を閉じる (そこで復号の側の文字の境界と符号化の側の単位の境界が再び一致し、以後の復号は本文と一致する。Shift_JIS の `<0x81>` + 全角 5,000 字 + 改行は 5,001 字で、改行は変わらない)。範囲の中で次のつながりに来たら範囲を延ばす (重なるつながりは 1 つの範囲にまとめる。例: Shift_JIS の `<0x81>亜<0x81>亜` + 改行は 2 件で、範囲は 1 つの 3 文字)。範囲の文字数と行数 (範囲の文字がある行の数) を合計して `changed_chars` / `changed_lines` とする。文書の末尾まで閉じなければ範囲は末尾までとし、末尾で復号器に持ち越しが残るとき (末尾までずれたまま戻らないとき) に `to_eof` を立てる。費用は、つながりがあるときだけ、つながりの位置から範囲を閉じるまでの単位ごとの復号 (乾式走査の中で行う。本番の書き込みでは変わる範囲を求めない)。
- **確認 (ii) との関係** (§8.6 E2 の注記の (a)(b)): 確認 (ii) (出力の全体を同じ文字コードの 1 本の復号器で復号して本文と突き合わせる) を、記録を渡す保存だけでなく、**エスケープ文字を元のバイトに戻して書く保存の全部** (`Restore`。記録の有無・文字コードに依らない。UTF-8 / UTF-16 を含む) に広げる (= (a) 記録を渡さない保存のつなぎ目も確かめる)。ただし突き合わせは、つながる並びごとに区切って行う: つながる並びの先頭の前までを突き合わせ、並びと直後の 1 単位の後で、復号器を初期状態にし直して続ける (つながる範囲の中だけを突き合わせない。つながらない並びはふつうに突き合わせる。並びがつながるかは直後の単位が来るまで決まらないので、確認 (ii) はその並びの所から出力を持って待つ)。区切った突き合わせで合わない箇所 (局所の検査で説明できない不一致) は、これまでどおり実装の誤りとして `JobError::VerifyFailed` で止め、記録を捨てる (§8.7.1 の「保存」) (= (b) 意味変化は確認、それ以外は中止)。分け方が成り立つ理由: つながりは並びの所でだけ始まり (上の局所の検査)、並びの外の不一致は、記録の誤り (確認 (i) と表の不変条件で先に見る) か符号化器の誤りでしか起きない。記録の元のバイト列が並びの直後にあるときは記録の全体を直後の 1 単位として扱うので、並びが記録の先頭のバイトとつながる場合も `EscapeMerge` になり (`VerifyFailed` にしない)、並びの直前にあるときはつながらない。
  - **記録が無い文書の `VerifyFailed`**: `Document::abandon_preserved` (記録を捨てて「保持なし」にする) は、保持の状態が `NotApplicable` の文書 (UTF-8 / UTF-16 の文書など、記録をもともと持たない) では状態を変えない (変えると、以後の保存のたびに E2' の確認が出て、文言が事実と合わない。M23 の前の実装は常に `Unavailable(VerifyFailed)` にしていた。M23 の 3 本目で直した)。捨てる記録が無いので、同じ保存は同じ理由で止まる (実装の誤りでだけ起きる)。この文書を書ける手段は **「退避先に保存」だけ** である (エスケープ文字を含む文書は別の文字コードでは `Reject` で、E3 のキャンセルだけになる)。
- **見る順序**: 乾式走査で、(1) 確認 (i) と表の不変条件 (`VerifyFailed`)、(2) E1 の表現不能 (`Unmappable`。表現不能の文字は出力から抜けて後ろの突き合わせがずれるので、つながりより先)、(3) つながり (`EscapeMerge`。承認と一致すれば止めない、下の「承認」)、(4) 区切った確認 (ii) (`VerifyFailed`) の順に見て、最初に当たったもので止める (§9.3 の手順 4')。
- **いつ見つけるか**: `Restore` の保存は、原子的保存でも **書く前に乾式走査をする** (§9.3 の手順 4'。v0.3 までの実装は、UTF-8 / UTF-16 の原子的保存では、エスケープの方針が `Reject` のときだけ乾式走査をし、`Restore` では書きながら符号化していた)。つながりがあれば一時ファイルも作らずに `JobResult::EscapeMerge` (02 §9.3) を返す (何も書いていない)。本番の書き込みでも同じ検査と区切った確認 (ii) を行い、乾式走査と結果が違えば (不変条件の違反) 書くのを止める (原子的保存は一時ファイルを捨て、直接上書きは `InPlaceFailed`。§9.3 の「乾式走査と出力長」と同じ)。
- **利用者への示し方 (書く前の確認)**: core は `ui.confirm.save.escape_merge` (04 §15.2) を出す。見出しと本文は結果で言う (「開き直すと文字が変わる箇所」。「隣のバイト」のような見えない単位の言葉を避ける): 件数・最初の位置 (行・桁)・変わる範囲 (「合わせて N 文字 (M 行) が別の文字になります」、`to_eof` なら「ファイルの末尾まで別の文字になります」)・「まだ保存していません」。`details` に E3 の置換の一覧と同じ形で、1 件 1 行の「行 {line}, 桁 {col}: <0xE3><0x81><0x82> → あ」(左は強調の範囲の本文を表記にしたもの、右は局所の復号。どちらも §14.6 の無害化を通し、改行・制御文字は記号にする) と、100 件を超えるときの「ほか {count} 箇所」を並べる。確認を出すときに、最初の位置が画面に入るようにスクロールする (カーソルは動かさない)。選択肢は `[S] このまま保存` とキャンセル (`Esc`。★。破壊的な選択肢は既定にしない、04 §15.2)。一覧の左は強調の範囲の本文の先頭 16 文字 (長ければ `…` を付ける。右の局所の復号と同じ長さ) にする (M23 の 4 本目で決めた)。
  - **キャンセル**: 保存しない。カーソルを最初の位置へ移し、全部の位置 (先頭 100 件) を表現不能と同じ強調 (`unmappable_marks` と同じ core の内部の印。範囲は上の「強調の範囲」。Revision が変わると消える。eve-view の Decoration の型は足さない) で示し、`info.save.escape_merge_cancelled` (強調したこと、直し方: 不正なバイトを消す・間に改行を入れる・元の並びに戻す。件数は強調した数 (先頭 100 件まで)) を出す。後続 (終了・置き換え・開き直し) を中止する (E3 のキャンセルと同じ)。キャンセルの時点で文書の Revision が結果の base と違う (保存の待ちの間に編集した) ときは、位置が今の本文と合わないので、強調もカーソルの移動もせずに `info.save.aborted` を出す (M23 の 4 本目で決めた)。
  - **このまま保存 (承認)**: 承認は「何を承認したか」で持つ: (文書, 結果の base Revision, 結果のつながりの件数・先頭 100 件・全件のダイジェスト)。core の `SaveApprovals` に覚え (`lossy_ok` / `readonly_ok` と同じ置き場所)、保存のジョブには `escape_merge_ok: Option<EscapeMergeApproval>` として渡す。ジョブは承認があってもつながりの検査を走らせ、**結果 (件数・先頭 100 件・全件のダイジェスト) が承認したものと同じときだけ** 止めずに書く (違えば `EscapeMerge` を返し、もう一度確認する)。先頭 100 件は位置・強調の範囲・局所の復号を比べ、全件のダイジェスト (位置・強調の範囲・局所の復号を全部の件について順に入れたハッシュ。保存の内容のハッシュと同じ XXH3-64、05 §5.5) で 101 件目より後ろの違いも見る (件数と先頭 100 件が同じでも、後ろで承認していない位置がつながる文書に書かないため。M23 の 3 本目のレビューで足した)。書く文字コード・BOM・保持方式の記録の状態 (`VerifyFailed` で捨てた後など) が変わると出力が変わるので、Revision と文字コードの組をキーにせず、結果で照らし合わせる。「このまま保存」を選んだ時点で文書の Revision が base と違えば (保存の待ちの間に編集した)、承認を覚えずに今の Snapshot で保存し直す (つながりが残っていれば、もう一度確認する。利用者が見ていない位置を承認しない)。承認した保存も、区切った確認 (ii) は行う (安全網を外すのは、つながる範囲の中だけ)。
- **全保存** (§9.8): 確認を出さずに、その文書を保存しないで「確認が必要です」(`ui.confirm.save_all_failed.needs_confirm`) として一覧に載せる (承認が `SaveApprovals` にあり結果が一致すれば保存する)。**終了の前の保存** (`AskUnsaved` の「保存」): 連鎖の中で同じ確認を出し、キャンセルで終了を中止する。**保存を中止して終了** (§9.7 (f) の `QuitAbort`) の後に届いた `EscapeMerge` は、何も書いていないので、そのまま終了へ進む (dirty な文書は退避される)。
- **つながったまま保存した後**: 文書はエスケープ文字のまま (本文を変えない)。保存点はその Revision (dirty = false)。書き出しは決定的なので、同じ内容をもう一度保存すると同じバイト列になり、文書とディスクの対応は保たれる (未編集の上書き保存は §9.3 手順 0 で書かない)。開き直す・再読込すると、つながった文字として読まれる。完了のメッセージは `info.save.done` に代えて `info.save.escape_merged` (要点 (保存したこと・開き直すと N 箇所が別の文字になること) を先頭に、80 桁で見える長さ) にする。直接上書きで書いたときは、`info.save.in_place` の後に出す (直接上書きで書いたことも 1 回示す、§9.3 手順 7)。core は「承認をジョブに渡して投入した保存の成功」でこのメッセージを選ぶ (`SaveReport` はつながりを書いたかを持たない。承認を渡した保存で結果が承認と違えばジョブは書かずに `EscapeMerge` を返すので、書いた保存のつながりは承認したものと同じ。承認を渡したのにつながりが 0 件になった保存 (同じ Revision のまま、BOM・保持方式の記録の状態だけが変わった場合。ふつうは起きない) でもこのメッセージになる)。自動では再読込しない。
- **検出しないもの**: 開き直すときの文字コードの自動判定 (§8.2) の結果が、BOM 以外の理由で変わる場合 (本文のバイトの並びから、文字コードの推定が変わる場合。例: エスケープ文字を戻したバイトが増えて、Shift_JIS の文書が別の文字コードと推定される)。また、出力の先頭が **別の文字コードの BOM の並び** になる場合 (例: BOM の無い UTF-8 の文書の先頭に `<0xFF><0xFE>`・`<0xFE><0xFF>`、Shift_JIS の文書の先頭に `<0xEF><0xBB><0xBF>`・`<0xFF><0xFE>` (`<0xEF><0xBB>` と半角カナ 0xBF の組を含む)、EUC-JP の文書の先頭に `<0xFF><0xFE>`)。自動判定では別の文字コード (UTF-16 など) や UTF-32 の BOM の確認になり得るが、同じ文字コードを指定して開き直せば (FR-111) 元に戻り、そうしたファイルは読込の時点から同じバイト列で始まっている (未編集のまま名前を付けて保存すると、元のファイルとバイト単位で同じものを書く) ので、これらの範囲に限って検出しない。書く文字コード自身の BOM の並びは、上の「文書の先頭の BOM」のとおり局所の検査で見つける。
- 03 §8.3 の表の旧記述にあった `SaveWarning::EscapeMerge` (保存の後の警告) は置かない (02 §4.5)。

**E3 との関係** (§8.6 の「E3 の選択肢」): `JobResult::Unmappable` に、`escapes_restored` (この保存で元のバイトに戻したエスケープ文字の数) と `escapes_unmappable` (表現不能になったエスケープ文字の数。**記録した先頭 100 件ではなく全件で数える**) を足す (02 §9.3)。「UTF-8 で保存」は両方が 0 のときだけ、「置換して保存」は `escapes_unmappable == 0` のときだけ出す (UTF-8 にするとエスケープ文字が表現不能になり、もう一度 E3 になる。置換すると不正バイトが `?` で書かれる)。置換 (`JobKind::ReplaceUnmappable`) はエスケープ文字を対象にしない。`escapes_unmappable` が 1 以上の E3 は、本文を不正なバイトの説明にする (「不正なバイト (<0xNN>) {count} 個は {encoding} では保存できません (最初: 行 {line}, 桁 {col})。消すか正しい文字に打ち直してから保存してください」。04 §15.2 の `ui.confirm.save.unmappable.escape_message`。「最初」は最初のエスケープ文字の位置で、記録した先頭 100 件に無ければ最初の表現不能の文字の位置)。選択肢「不正なバイトを消して保存」は v0.4 では足さない (利用者が自分で消す。OQ-217)。今の実装は「表現不能の文字にエスケープ文字を含まない」を記録した先頭 100 件だけで判定しており、101 件目より後ろにエスケープ文字があると「置換して保存」が出て、エスケープ文字が `?` で書かれる。これは v0.3 までの実装の不具合で、v0.4 で直した (`escapes_unmappable` を全件で数え、置換の対象からエスケープ文字を外す)。

**関連する場面**:

| 場面 | 扱い |
|---|---|
| Undo / Redo | エスケープ文字は通常の文字として戻る (§7)。つながりは保存のときに見るので、Undo で並びが元に戻れば確認は出ない。違う文字コードで保存した後に Undo で戻ったエスケープ文字は `escape_encoding == None` なのでバイトに戻さない (上の表) |
| 検索・置換 | 正規表現の `\x{10FF9A}` で一致する。リテラルの `<0x9A>` は表記の文字列にだけ一致する。置換の文字列の `$1` 等が運ぶエスケープ文字は同じ文書のもの (バイトのまま)。置換の文字列の文字どおりの部分は上の経路 (2)。すべて置換でエスケープ文字を消すこともできる (その不正バイトはファイルから無くなる。解除の確認で言う) |
| 改行コードの変換・行の結合 | 改行コードの変換 (§8.4) は CR / LF だけを変えるので、つながりを作らない (CR (0x0D) / LF (0x0A) は Shift_JIS・EUC-JP・UTF-8 の多バイト列の後続のバイトにならず、UTF-16 は 2 バイトの単位のまま)。行の結合 (改行の削除) でエスケープ文字が次の行の文字と隣り合えば、つながることがあり、保存のときに見つける。エスケープ文字は改行に数えない (`<0x0A>` は行を分けない、§2.4) ので、`eol_stats` と `eol_guard` (§6.2) は変わらない |
| スワップ・緊急退避・復旧 | 解除した文書は読み取り専用でないのでスワップの対象 (§10.2)。本文はエスケープ文字のまま書き、manifest の `pua_escaped` は `escape_encoding.is_some()` (02 §11.2.1。形式は変えない)。**復旧の提案と適用は、読み取り専用の理由が `InvalidBytes` だけ (`pua_escape == Enabled`) の文書にも、スワップの文字コードが文書の文字コードと同じときは行う** (§10.3。違えば提案せず `info.swap.encoding_mismatch` で、その文字コードで開き直すよう案内する。解除して編集した文書は、次に開くと `InvalidBytes` で読み取り専用に戻るので、今の実装 (読み取り専用の文書には提案も適用もしない) では復旧が出ない)。復旧を適用した文書は読み取り専用のまま (読込で付いた `InvalidBytes`。復旧の読込では解除を持ち越さない) で、`info.swap.applied_read_only` で「読み取り専用の切替で編集を許可すると続けられる」と案内する。「退避先に保存」と名前を付けて保存はできる。`escape_encoding` は上の表のとおり |
| 文字コードを指定して開き直す | 新しい読込の結果に置き換わる (§8.8)。解除は引き継がない (上の「解除の持ち越し」) |
| 不正バイトの比率の警告 (FR-127) | 「読み取り専用で開く」を選んだ文書も、上の確認の後に解除できる |
| バイナリ (`Binary`) | 解除できない (NG10)。バイナリの確認から「文字コードを指定して開く」で開き直した文書は `Binary` が外れ、不正バイトがあれば `InvalidBytes` だけになるので解除できる |
| 長大行 | char 近似モード (§11.5) でも、エスケープ文字は 1 文字 = 1 書記素で扱う (上の「書記素と単語」) |
| 別の文字コードへの保存 (AC-6) | §8.6 E4 のまま: エスケープ文字は表現不能で E3。選択肢はキャンセルだけ (「UTF-8 で保存」「置換して保存」は出さない。上の「E3 との関係」) |
| ISO-2022-JP | 変えない (`StatefulEncodingWithInvalidBytes` は解除できず、ISO-2022-JP では保存しない。OQ-207)。不正バイトの無い ISO-2022-JP の文書には、外から来た私用文字を `<U+10FFNN>` にして入れる (上の表の「状態付き・不正バイトなし」) |

**実装の PR の分け方**: 正本は 08 §2.5 の M23 の行 (順番、各 PR の範囲、どれを 2 つの視点のレビューに掛けるか)。本節の決定はその順に入れる。v0.3 までの実装の不具合 (E3 の選択肢の判定が先頭 100 件だけ、ISO-2022-JP の不正バイトの表示) は、M23 とは別の fix の PR で直した。

### 8.4 改行コード (OQ-04)

**OQ-04 で確定 (ユーザー回答) した「内部でも元の改行コードをそのまま保持 (混在も保持)」に従う。** 新規文書の既定は LF、貼り付けは文書の既定改行に揃える (OQ-51)。 本書での影響をすべて列挙する。

| 領域 | 影響と規則 |
|---|---|
| 行の定義 | §2.4。LF / CRLF / CR を区切りとし、CRLF は 1 つ |
| 改行の挿入 (FR-202) | `default_eol` を挿入する (混在ファイルで直前・直後の改行と合体する場合は §6.2 の `eol_guard` が種類を合わせる)。読込時の `default_eol` は多数派 (同数なら LF、全く改行がなければ設定の既定 `file.default_line_ending`、既定 LF。OS 依存にしない: P6) |
| 混在 (Mixed) | `eol_stats` の 2 種以上が非 0 なら Mixed。ステータスバーに `混在` (01 FR-503 / 04 §8.7) と示す。警告も「揃える」提案もしない (保持が正しい挙動。OQ-52) |
| 変換コマンド (FR-113、**v0.1**) | `file.set_line_ending(eol)` (`eol` = LF / CRLF / CR。パレットでリストから選ぶ、07 §11.6)。手順: (1) 文書が編集できない (`read_only` が空でない、または `load_state != Ready`) なら Transaction を作らずに `msg.edit.read_only` で拒否する (§9.6)。(2) 変える改行の数 k を `eol_stats` から求める (`eol` 以外の 2 種の和。走査しない)。k = 0 なら本文を変えず、`default_eol` だけを `eol` にして (メタデータの変更。Undo の対象外で dirty にしない、§7.2) `info.eol.unchanged` を出す。(3) k > 0 なら、`eol` と異なるすべての区切り (混在の少数派を含む) を `eol` に置き換える ChangeSet を作る。文書が 1 MiB (同期で作る上限。定数。実装は `document/eol_cmd.rs` の `SYNC_BYTES`。§12.2 の `SEARCH_SYNC_STEP_BYTES` (256 KiB) とは別の値で、v0.5 の M34 から `TRANSFORM_SYNC_BYTES` (§15.8) と同じ値) 以下ならメインスレッドで作り、超えればジョブ (`JobKind::ConvertEol`、02 §9.3) で Snapshot (Revision R) から作る (全文走査。適用は O(k log n))。すべての区切りを同じ `eol` にするので、置き換えで隣り合った CR と LF が新しい CRLF に結合することはない。(4) 適用の時点で Revision が R でなければ、今の Snapshot で 1 回だけ作り直し、それも古ければ適用せずに `msg.eol.convert_stale` を出す。(5) **1 つの Transaction** (`Origin::Command("file.set_line_ending")`、`UndoGroupHint::Break`) として適用し、同時に `default_eol` を `eol` にする。Undo ノードの `format_change` に前後の `default_eol` だけを持たせ (`FormatChange::DefaultEol`。文字コード・BOM などは戻さない)、1 回の Undo で本文と `default_eol` が一緒に戻る (§7.2)。`eol_stats` は適用で数え直され (§2.4)、ステータスバーの改行コードは `eol` になる (「混在」が消える)。文書は dirty になり、`info.eol.converted` で件数を示す。選択は ChangeSet で写像する。変換は保存時のエンコードと独立で、どの文字コードでもそのまま書く (保存は区切りをそのまま書く、下の「保存」)。Undo の予算 (§7.5) の「直前の 1 操作は保持」が大きな変換にも働く |
| カーソル | CRLF の内側に置けない (書記素) |
| 削除 | 行末で Delete → CRLF 全体 (1 書記素) を削除。Backspace も同様 |
| 検索 | リテラル検索では改行 (`\n`) を入力できない (単一行の入力欄。§12.3)。正規表現の `\n` (複数行にまたがる一致は v0.4。v0.2 は「未対応」と表示、§12.3) は改行 1 つに一致する糖衣 (書き換えの形は §12.3.1) で、`LF` / `CRLF` / `CR` の **どれにも一致** させる (正規表現の `R` フラグ = CRLF モード)。`$` は CR の直前にも一致する ([§12.3](#123-正規表現))。v0.4 の細目 (クラスの中の `\n`、続けて書いた `\r\n`、`\s` や `[^x]` は行の終端に一致しない) は §12.3.1 |
| 貼り付け (FR-113 / FR-206) | **正規化は core の貼り付けの入口 1 か所** で行う: 挿入するテキスト中の `\r\n`・単独の `\r`・単独の `\n` を、それぞれ 1 つの `default_eol` に置き換える (`normalize_eol(text, default_eol)`、1 パス)。対象は **すべての貼り付けの経路**: ブラケットペースト (`InputEvent::Paste`)、バースト検出で合成された `Paste` (04 §6.6)、システムクリップボードの読取結果 (`InputEvent::ClipboardContent`、Windows では CRLF で届く)、内部レジスタ (CRLF・CR の文書からのコピーを含む)。端末からの貼り付けは CR になることが多く、そのままでは意図せず Mixed になるか、`\r\n` の `\n` だけを変換して `\r\r\n` (行が増える) になる。frontend (eve-tui) が `\r\n` / `\r` → `\n` の前処理をしても結果は同じなので、前処理は **任意** とする。不正バイトのエスケープ文字 (`U+10FF00..U+10FFFF`) の表記への置き換えと、内部レジスタとの照合 (v0.4 の M23) も同じ入口で行う (§8.3.1 の「本文に入れる規則」)。正規化を切る設定は v0.2 で公開 (07 §5.13 `editor.paste_normalize_eol`)、v0.1 は常に ON。正規化したあとの挿入文字列の端が、挿入位置の前後の単独の `\r` / `\n` と隣接して合体する場合は、§6.2 の `eol_guard` が端の 1 文字だけを直す (混在ファイルの場合) |
| 表示 | 行末の区切りは描画しない。混在時に少数派の区切りを可視化する設定 (FR-508 の一部、v0.5) |
| 保存 | そのまま書き出す (変換なし)。NFR-303 を混在ファイルでも満たす |
| LSP / tree-sitter | 位置は行 + 桁で渡す。CRLF が 2 バイトである点は `LineByte` 変換で自然に扱える |

### 8.5 末尾改行 (FR-114)

- 読込時に末尾が区切りなら最終行が空 (§2.4)。保存はテキストをそのまま書くので、末尾改行の有無は **自動的に保持** される。専用フラグは不要 (`final_newline` は導出値)。
- FR-216 (保存時に末尾改行を付与) は、保存ジョブに渡す前に Transaction (Origin::Command) として適用する (Undo 可)。保存後のバッファと保存内容を一致させるため、内部のテキストも変更する。未編集の文書の上書き保存には適用しない (§9.3 の手順 2)。規則 (足す改行・空の文書・取り除かないこと) は §9.3.1 の SVT-3 (v0.5 の M34)。

### 8.6 エンコード (保存) と表現不能文字 (FR-112)

**前提 (要確認 S-01)**: encoding_rs (WHATWG 準拠) のエンコーダは、表現不能文字を常に `Unmappable` で返すわけではない。Shift_JIS / EUC-JP エンコーダは U+00A5 (¥) を 0x5C、U+203E (‾) を 0x7E、U+2212 (−) を U+FF0D 相当のバイト列に **黙って写像** し、ISO-2022-JP エンコーダは半角カナを全角カナに黙って変換する。また EUC-JP エンコーダは JIS X 0212 (3 バイト、補助漢字) を、Shift_JIS エンコーダは CP932 の外字 (U+E000〜U+E757、§8.7) を **エンコードしない** (復号専用。外字は `eve-encoding` が自前の逆算で補う、§8.7 の「CP932 の外字」)。JIS 系・Mac 系の写像で作られた文字 (U+301C `〜`・U+2016 `‖`・U+00A2 `¢`・U+00A3 `£`・U+00AC `¬`・U+00A6 `¦`・U+2014 `—`) は Shift_JIS / EUC-JP / ISO-2022-JP のいずれでも `Unmappable` になる (実測、encoding_rs 0.8.35。§8.7 の「CP932 互換文字と検索」)。「`Unmappable` が返ったら中止」だけでは FR-112 (黙って置換しない) を満たせない。

| 規則 | 内容 |
|---|---|
| E1 照合 | 保存ジョブは Snapshot のチャンクを順に `encode_from_utf8_without_replacement` 相当で変換し、**UTF-8 以外の文字コードでは、出力したバイト列を直ちに同じ文字コードで decode して元のチャンク文字列と照合** する。`Unmappable(ch)` が返った文字と、照合で一致しなかった文字 (黙示置換) を **同じ「表現不能」として扱い**、位置 (`Pos`) を記録して続行 (最大 100 件記録、総数は数える)。ただし Shift_JIS で `Unmappable(ch)` の `ch` が U+E000〜U+E757 (CP932 の外字) のときは、表現不能とせず §8.7 の「CP932 の外字」の逆算で 2 バイトを出力して続行する (照合はその出力を encoding_rs で復号して行い、同じ文字に戻るので一致する)。コストは encode とほぼ同じ (decode 1 回追加)、追加メモリはチャンク 1 つ分。ファイルサイズによる閾値は設けない。v0.4 では、E2 で元のバイト列を書く記録の範囲の文字を照合の対象にせず、表現不能に数えない (§8.7.1) |
| E2 記録が駆動する差し替え (v0.4、§8.7.1) | 保存の符号化は、Snapshot と保持方式の記録の版 (保存を始めた時点のもの) から決まる **1 つの計画** に従う。計画は、記録の範囲ごとに「元のバイト列を書く」か「符号化する」かを決めたもので (チャンクの区切りに依らない)、乾式走査・本番の書き込み・`JobKind::ReplaceUnmappable` の対象の判定が同じ計画を使う。元のバイト列を書くのは、(a) 書く文字コードが記録を集めたときの文字コードと同じ、(b) 記録が編集されていない (表に残っている。§8.7.1 の「編集での写像」)、(c) **確認 (i)**: 範囲の今の文字列が、元のバイト列をその文字コードの初期状態から復号したものと一致する、(d) ISO-2022-JP では範囲が 1 つの行の内容の全体である (行頭で始まり、その行の終端の直前か文書の末尾で終わる。符号化器は ASCII 状態。M22 の 6 で決定)、のすべてを満たす記録で、そのときは **E1 の照合の結果に依らず** 元のバイト列をそのまま書く。この範囲の文字は E1 の表現不能に数えず (補助漢字や ISO-2022-JP の半角カナが、編集していない範囲にあるだけなら E3 で止まらない)、「置換して保存」の対象にもしない。(d) を満たさない行の記録 (前後の行の終端を消して行が変わった ISO-2022-JP の行の記録) は使わず、その行を編集した行と同じ (区分 C) に正規形で書く (保存は成功する。確認 (E2') にも数えない。記録は表に残す: 記録が表すのは「範囲の今の文字列の、元のバイト列」でディスクの内容ではないので、Undo で行が戻れば次の保存で使う。M22 の 6 のレビューで決定)。(c) を満たさない記録と、表の不変条件に反する記録 (範囲が本文の外・char 境界でない・前の記録と重なる) は実装の誤りで、下の確認 (ii) の不一致と同じく、どのビルドでも何も書かずに保存を止める (`JobError::VerifyFailed`。乾式走査の段階で見つける)。**確認 (ii)**: 保存の出力の全体を、同じ文字コードの 1 本の復号器で復号すると、Snapshot の本文に一致する (元のバイト列と符号化した部分のつなぎ目を含めて確かめる。不正バイトのエスケープは §8.3 のとおり)。一致しなければ保存を中止する (原子的保存では一時ファイルを捨てる。直接上書きでは乾式走査の段階で分かるので、元のファイルに触れない)。確認 (ii) は記録の版を渡した保存 (保持が効いている文書を元の文字コードで書く保存) と、エスケープ文字を元のバイトに戻して書く保存 (M23 で広げた。下の注記) で、乾式走査と本番の書き込みの両方で行う (どちらでもない保存は v0.3 と同じ出力で、確認 (ii) をしない)。中止の結果は `JobResult::Failed { err: JobError::VerifyFailed }` (02 §9.3) で、core は `msg.save.verify_failed` (04 §15.2) を §9.7 (e) の `VerifyFailed` の行の選択肢 (別名で保存 ★・退避先に保存・中止) で示し、文書の記録を全部捨てて「保持なし」(`NoPreserveReason::VerifyFailed`) にする (同じ表を渡し続けると同じ文字コードで二度と保存できないため。次の保存は E2' の確認を経て、記録を使わずに書く。M22 の 6 で決定)。**不正バイトを含む文書の編集 (v0.4 の M23、§8.3.1 の「つながりの検出」)**: 確認 (ii) は、記録を渡す保存に加えて、エスケープ文字を元のバイトに戻して書く保存 (`EscapePolicy::Restore`) の全部で行う ((a) 記録を渡さない保存のつなぎ目も確かめる)。不一致のうち、エスケープ文字の並びとその直後の 1 単位で説明できるもの (局所の検査で見つけたつながり) は `JobResult::EscapeMerge` として書く前に確認し (承認すれば書く)、それ以外は今のまま `VerifyFailed` で止める ((b))。突き合わせは、つながる並びごとに区切って行う。編集した範囲の区分 B の文字は、代表バイト列で書く (区分 C。確認は出さない。符号化できない文字は、これまでどおり E3 で止める)。v0.3 までの仕様にあった「照合で一致しない文字だけを差し替える」条件は採らない (重複符号化は、再符号化しても復号すれば同じ文字に戻るので、照合では違いが出ない)。**v0.1〜v0.3 は保持方式を持たず E2' で代替** した |
| E2' 検出と確認 (v0.1〜v0.3。v0.4 は条件を改める) | **v0.1〜v0.3**: 読込時の照合 (§8.7) で記録した `RoundtripSummary` の行数 (`lossy_lines`) が 1 以上の文書を **元の文字コードで保存するとき**、「N 箇所が同じ文字の別のバイト表現で保存されます (例: IBM 拡張の ¬ 0xFA54 → 0x81CA)」と確認ダイアログを出し、承認後に E1 の照合結果 (代表バイト列) で書く。復号専用文字 (EUC-JP の JIS X 0212 等、エンコーダが出力できないもの) は E1 で表現不能として報告される。CP932 の外字は自前の逆算で元のバイトに戻るので (§8.7、確定 (ユーザー回答)、OQ-206) E2' の件数にも E1 の表現不能にも入らない。**無編集の上書き保存は書き込まない** (§9.3 手順 0) ので、確認が出るのは編集した文書を元の文字コードで保存する場合だけ。意味 (文字列) は変わらず、黙って変わることはない。**v0.4** (§8.7.1): 保持が効いている文書では出さない。「保持なし」の文書 (読込時の検査で区分 B / D の箇所を見つけた、または数を確かめられなかったもの) と「保持できなかった箇所がある」文書を、元の文字コード (記録を集めたときの文字コード) で書くときだけ出す。出す時点は v0.3 と同じ (既定は「別名で保存」。区分 D の `escape_normalized` は文字が変わらないので「保存」。承認した Revision では繰り返さない。全保存 (§9.8) では、その文書を保存せずに残す)。この確認は **文書が未編集でも出す** (未編集の文書を同じ文字コードで「名前を付けて保存」する・`file.set_encoding` で同じ文字コードを選ぶ場合。v0.3 までは確認なしに代表バイト列で書いていた)。未編集の上書き保存は、これまでどおり書き込まないので出ない。文言は、保持できなかった箇所の数を言えるときは `ui.confirm.save.lossy_roundtrip` (Shift_JIS / EUC-JP) / `ui.confirm.save.escape_normalized` (ISO-2022-JP)、言えないときは `ui.confirm.save.roundtrip_unchecked` (04 §15.2)。**v0.4 の条件の細部** (M22 の 6 で決定): 保持が効いている文書 (`PreserveStatus::Active`) は、読込の時点で保持できなかった行の数 `unpreserved` (ISO-2022-JP だけが数える) が 1 以上なら、その数で出す (`escape_normalized`。その行を後で編集・削除しても減らない。行を結合して行の内容の全体でなくなった行の記録は、編集した行と同じ区分 C なので数えない)。「保持なし」(`Unavailable`) は、理由が `VerifyFailed` なら無条件で、`InvalidBytes` 以外のほかの理由なら `lossy_lines ≥ 1` か `incomplete` のときだけ、数を言わずに出す (`roundtrip_unchecked`)。補助漢字・半角カナなど復号専用の文字の行 (`unmappable_lines`) だけの「保持なし」の文書には出さない (保存は E3 で止まるので、確認の文が事実と合わない。v0.3 と同じく E3 に任せる)。理由が `InvalidBytes` (不正バイトを含む ISO-2022-JP) は、保存の連鎖の開始で拒否する (§8.3) ので出さない。対象外 (`NotApplicable`) には出さない (読込が作る対象外の文書は件数を持たない)。全保存も同じ条件で保存せずに残す |
| E3 中止 | E1 の表現不能が 1 件以上あれば **保存を中止** し (原子的保存では一時ファイルを Port が破棄、直接上書きでは乾式走査で検出するので元ファイルに触れない、§9.3)、`JobResult::Failed { err: JobError::Unmappable { positions, total } }` (02 §9.3) を返す。core は該当箇所を Decoration で示し、下の「E3 の選択肢」を出す (`ui.confirm.save.unmappable`、04 §15.2)。利用者が選ばない限り置換しない (黙って `?` にしない)。本規則 (E1〜E3) は 02 §13.4 / ADR-0026 の具体化で、ADR-0026 の (1) 照合は v0.1、(2) 保持方式は v0.4 と段階化する |
| E4 エスケープ文字 | §8.3 のエスケープ文字は、元の文字コードで保存するときはバイトに戻し、別の文字コードへ変換するときは表現不能として扱う。状態付き符号化ではバイトに戻さず、エスケープ文字を含む文書をその文字コードで保存しない (保存ジョブを投入する前に拒否する。§8.3、確定 (ユーザー回答)、OQ-207)。文書の種類ごとの扱い (`save_escape_policy`。元のバイトに戻すのは書く文字コードが `escape_encoding` と同じときだけ。バイトでない文書・衝突の文書 (b) では表現不能、衝突の文書 (a) では通常の文字) の正本は §8.3.1 の表 (v0.4、M23)。バイトに戻す保存では、戻したバイトが隣のバイトとつながるかを書く前に調べる (§8.3.1 の「つながりの検出」) |
| E5 UTF-16 | UTF-16LE/BE のエンコードは自前 (`char::encode_utf16`)。BOM は `TextFormat.has_bom` に従って書く。照合は不要 (1:1)。細目は下の「UTF-16 の書き込み」 |
| E6 ISO-2022-JP の保存 (**v0.1**、02 §13.4) | 状態付き符号化のため、保存ジョブは encoding_rs の ISO-2022-JP エンコーダを **行ごとに閉じる** (確定 (ユーザー回答)、OQ-162): 1 行の内部ではエンコーダの状態 (ASCII / ローマ字 / JIS X 0208) をチャンクの境界をまたいで引き継ぎ (チャンクごとに作り直さない)、行の内容を `last = true` で符号化して ASCII 状態に戻してから改行バイトを付ける。各チャンクの出力バッファは `max_buffer_length_from_utf8_if_no_unmappables` で取る (状態遷移のエスケープ 3 バイト分を含む)。各行の終わりで `last = true` で呼び、ASCII 状態へ戻す `ESC ( B` を必ず出す (`has_pending_state()` が偽になることを確認。改行を含まない最終行も同じ)。E1 の照合に使うデコーダも同じ単位 (1 行の内部はチャンクをまたいで通し) で使い、出力を復号した文字列を元の文字列と比較する (エスケープは文字を生まないので、チャンク境界をまたいでも比較は成り立つ。最後は `last = true` で復号し、残りが無いことを確認)。**行末の状態**: WHATWG のエンコーダは、JIS X 0208 状態からは ASCII 文字の前で `ESC ( B` を出して ASCII 状態に戻るが、**ローマ字状態 (`¥` U+00A5 / `‾` U+203E の出力後) では U+005C / U+007E 以外の ASCII (改行を含む) を状態を変えずに出力** し、ASCII 状態へ戻すのは入力の終わり (`last = true`) だけである。したがってエンコーダを文書全体で 1 つ通すだけでは、各行が ASCII 状態で終わるとは限らない (例: `"¥\nabc"` → `ESC ( J 5C 0A 61 62 63 ESC ( B`)。各行を ASCII 状態で終える (FR-109 AC-3 の行末条件、RFC 1468 の慣行) ために **行ごとにエンコーダを閉じる** (OQ-162 で確定 (ユーザー回答)。encoding_rs の公開 API だけで実装でき、E1 の照合とも両立する。同じ例は `ESC ( J 5C ESC ( B 0A 61 62 63` になる)。S-01 (a') で `"¥\nabc"` の出力を確認する。**S-01 で照合が成り立たない文字コード** は v0.1 では読込のみ (別の文字コードでの保存は可) とし、Shift_JIS / EUC-JP の照合が成り立たなければ v0.1 のリリースを止める (OQ-162)。**半角カナ** (U+FF61〜FF9F) はエンコーダが黙って全角に変換するので E1 の照合で不一致になり **表現不能** として扱う (選択肢は下の「E3 の選択肢」の表)。v0.1〜v0.3 では ISO-2022-JP の半角カナは **E1 / E3 だけで扱い**、区分 D (§8.7) と E2' の対象にしない (半角カナを含む行は E3 の置換を経ないと保存できないので、E2' の確認に意味が無い)。**v0.4** では、編集していない、ASCII 状態で始まり ASCII 状態で終わる行の半角カナは、行の記録 (`DecodeOnly`、§8.7.1) で元のバイト列のまま書き、E1 の表現不能に数えない。編集した行の半角カナは、これまでどおり E1 / E3 で扱う。`¥` (U+00A5) / `‾` (U+203E) は `ESC ( J` で表現されるので一致する。`SO` / `SI` / `ESC` (U+000E / U+000F / U+001B) は WHATWG エンコーダが誤りとするので表現不能。PUA エスケープ (不正バイト) を含む文書は §8.3 のとおり ISO-2022-JP では保存しない (保存の連鎖の開始時に `msg.save.encoding_unsupported_iso2022` で拒否する。確定 (ユーザー回答)、OQ-207)。**エスケープの選び方の差** (`ESC $ @` → `ESC $ B`、`ESC ( J` で書かれた ASCII → `ESC ( B`、冗長なエスケープの除去) は再エンコードで変わるため、読込時に区分 D として検出し (§8.7)、v0.1〜v0.3 は編集後に ISO-2022-JP で保存するとき E2' の確認を出す (v0.4 は、編集していない、ASCII 状態で始まり終わる行を元のバイト列で書き、E2' は §8.7.1 の「保持なし」「保持できなかった箇所がある」文書だけ)。無編集の上書き保存は書き込まない (§9.3 手順 0) ので無編集ならバイト一致 |

**E3 の選択肢 (本表が正本。02 §13.4 と 04 §15.2 `ui.confirm.save.unmappable` は本表を参照する)**: 表現不能で保存を中止したとき、次の 3 つを出す。どれを選んでも、それまでに元ファイルへは何も書いていない。

| # | 選択肢 | 動作 | `TextFormat` | 出す条件 |
|---|---|---|---|---|
| 1 | UTF-8 で保存 | 同じ保存先へ、`format.encoding = UTF-8`・`has_bom = false` (他の項目は元のまま) の format を渡して保存ジョブを投入し直す | 保存が成功したときだけ UTF-8 に確定する。失敗・キャンセル・`Conflict` で終わったら元の format のまま | 表現不能の文字に §8.3 のエスケープ文字 (UTF-8 でも表現不能、E4) を含まず、かつ、この保存でエスケープ文字を元のバイトに戻していない (v0.4 の M23: `JobResult::Unmappable` の `escapes_unmappable == 0` かつ `escapes_restored == 0`、§8.3.1 の「E3 との関係」) とき。エスケープ文字を含むかは **記録した先頭 100 件ではなく全件で** 判定する (v0.3 までの実装は先頭 100 件だけを見ていた。別の fix の PR (#516) で直した) |
| 2 | 置換して保存 | 表現不能と判定された文字 (Unicode スカラー値) を下の「置換先」の規則で置き換える **1 つの Transaction** (`Origin::System`、§6.3。Undo で戻せる) を作り、**適用の前に対象の一覧を示して確認を取り** (下の「置換の一覧」)、承認されたら適用してから同じ文字コードで保存ジョブを投入し直す。置き換える位置は全件 (記録した先頭 100 件だけではない) なので、ChangeSet は E1 と同じ照合を全文に走らせるジョブ (`JobKind::ReplaceUnmappable`、02 §9.3) で作る (§12.5 の「すべて置換」と同じく、作った Revision と適用時の Revision が違えば作り直す) | 変えない | 文書が編集できるとき (`read_only` が空で `load_state == Ready`)。表現不能の文字にエスケープ文字を含まないとき (全件で判定する。v0.4 は `escapes_unmappable == 0`。選択肢 1 の条件と同じ注記) |
| 3 | キャンセル (`Esc`。既定) | 保存しない。文書は変わらない (dirty のまま)。カーソルを最初の表現不能の位置へ移し、該当箇所の Decoration を次の編集まで残す | 変えない | 常に |

- **置換先** (選択肢 2): (a) 保存先の文字コードが ISO-2022-JP で、対象が **半角カナ** (U+FF61〜FF9F) のとき → **全角カナ** に置き換える。対応は WHATWG Encoding Standard の index ISO-2022-JP katakana (エンコーダが黙って行う変換と同じ) で、1 文字を 1 文字に置き換える。濁点・半濁点 (`ﾞ` U+FF9E / `ﾟ` U+FF9F) は直前の文字と合成せず `゛` (U+309B) / `゜` (U+309C) にする (`ｶﾞ` → `カ゛`。一覧の 1 件 = 表現不能の 1 文字を保つため)。(a') 保存先の文字コードが Shift_JIS / EUC-JP / ISO-2022-JP で、対象が下の **互換文字の表** にあるとき → 表の置換後の文字に置き換える (確定 (ユーザー回答)、OQ-208。Windows の CP932 への変換が行う置き換えと同じ組で、1 文字を 1 文字に置き換える。置換後の文字は 3 つの文字コードのすべてで表現でき、往復する (実測、encoding_rs 0.8.35。ISO-2022-JP では JIS X 0208 状態の 2 バイト。U+FFE4 は ISO-2022-JP でも `ESC $ B 7C 7C` で往復する)。件数は `JobResult::UnmappableReplacement` の `counts.compat` に数える)。表の行のうち `¥` `‾` は保存先が Shift_JIS / EUC-JP のときだけ対象で、ISO-2022-JP では表現できる (`ESC ( J`) ので表現不能にならない。(b) それ以外の表現不能文字 ((a) と (a') の対象以外のすべて。例: JIS X 0208 に無い漢字・絵文字、編集した JIS X 0212 の文字、EUC-JP / ISO-2022-JP で保存する U+E000〜U+E757。§8.3 のエスケープ文字を含むときは選択肢 2 自体を出さない。エスケープ文字は置換の対象にしない (`?` で書くと不正バイトの値が失われる。`JobKind::ReplaceUnmappable` はエスケープ文字を数えず、変えない。v0.4 の M23、§8.3.1)) → ASCII の `?` (U+003F) に置き換える (1 文字 = 1 スカラー値を 1 つの `?` に)。置き換えた後の文字は必ずその文字コードで表現できる (全角カナ・互換文字の置換後は JIS X 0208 または CP932 にある)。**判定の順序**: 1 つの表現不能文字について (a) → (a') → (b) の順に最初に当てはまるものを使う (対象が重ならないので順序で結果は変わらないが、実装はこの順で引く)。

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
| **B 保持方式で保証** | 再エンコードでは元に戻らないが、読込時に元バイト列を保持方式の記録 (`PreservedMap`、§8.7.1。v0.4) に持ち、編集していない範囲は E2 でそのまま書くため一致 (v0.1〜v0.3 は検出して確認するだけ。下の「区分 B の v0.1 での主張」) | Shift_JIS (CP932) の NEC 特殊文字 / IBM 拡張の **重複符号化** (例: `¬` が 0x81CA / 0xEEF9 / 0xFA54)。EUC-JP の JIS X 0212 (3 バイト) と、WHATWG が別コードに写す文字 (例: 0x8FA2B7 → U+FF5E → 0xA1C1)。**復号専用 (エンコーダが出力しない) の文字**: EUC-JP の JIS X 0212 の大半 (v0.4 の記録の理由は `DecodeOnly`。重複符号化と別のコードに写す文字は `DuplicateEncoding`。v0.3 までの検査は JIS X 0212 を含む行を `unmappable_lines` に数え、`lossy_lines` に入れていなかった。§8.7.1)。Shift_JIS (CP932) の **外字** (ユーザー定義領域 0xF040〜0xF9FC → U+E000〜U+E757) は encoding_rs では復号専用だが、`eve-encoding` の自前の逆算で元のバイトに戻るので **区分 A** (下の「CP932 の外字」、確定 (ユーザー回答)、OQ-206)。CP932 互換文字 (0x8160 → U+FF5E 等) は再エンコードで元に戻るので区分 B ではない (下の「CP932 互換文字と検索」)。その他 encoding_rs の 1 対多写像 (GBK/Big5 の一部) |
| **C 編集箇所のみ非保証** | 区分 B の範囲を **編集した** 場合、その範囲は再エンコード結果 (代表バイト列) になる。JIS X 0212 のように encoding_rs がエンコードできない文字は E1 で表現不能として報告される。v0.4 の「編集した」は、§8.7.1 の写像で記録が外れた範囲 (Undo で戻した範囲は含まない) で、代表バイト列で書くときに確認を出さない | 区分 B と同じ対象の編集後 |
| **D 状態付き符号化の区分 B (エスケープ選択の差)** | ISO-2022-JP: エスケープシーケンスの選択 (`ESC $ @` vs `ESC $ B`、ローマ字 `ESC ( J` で書かれた ASCII vs `ESC ( B`、冗長なエスケープ、行末でのリセット位置、末尾の状態) は復号で失われ、再エンコードでは WHATWG エンコーダの正規形 (`ESC $ B` / `ESC ( B`、`¥` `‾` のみ `ESC ( J`、JIS X 0208 状態からは ASCII の直前で ASCII に戻り、末尾で ASCII に戻る。行末の扱いは §8.6 E6 の方式 (OQ-162) に従う) に揃う。**v0.1**: 読込時に **行単位** で元バイト列と再エンコード結果を比較し、差のある行数を `RoundtripSummary.lossy_lines`、行頭の位置 (元のファイルでのバイトオフセット) を `first_offsets` に記録する (行単位にするのは、状態付きでは文字単位の対応付けが一意に取れないから。比較用の再エンコードは保存と同じ方式 (§8.6 E6) で行う。行の終わりの状態が元ファイルと再エンコード結果で違う場合は、その行を差として数え、次の行は再エンコード側の状態を引き継いで比較する)。無編集の上書き保存は書き込まない (§9.3 手順 0) のでバイト一致。編集後・名前を付けて保存は E2' の確認 (「N 行のエスケープシーケンスが正規形に揃えられます」) を経て正規形で書く (意味 = 文字列は一致、NFR-303 の例外文)。**v0.4**: 保持方式では「ASCII 状態で始まり ASCII 状態で終わる、編集していない行」だけ元バイト列を書き (`PreservedSpan` の単位は行の内容、`reason = StatefulEscape`。半角カナを含む行は `DecodeOnly`)、それ以外の行は正規形 (区分 C。ASCII 状態で始まらない・終わらない行は §8.7.1 の「保持できなかった箇所」)。**半角カナ** (`ESC ( I` で書かれたものを含む) は、v0.1〜v0.3 では区分 D の対象にしない: E1 の照合で表現不能として検出され、エンコーダが黙って全角化することはない。全角化は E3 の置換 (§8.6 の「E3 の選択肢」2) でだけ行う。読込時の行単位の比較では、表現不能の文字 (半角カナ等) を含む行を `lossy_lines` に数えず、`unmappable_lines` に分ける (その行は E3 の置換を経ないと保存できないので、E2' の確認の対象にならない)。v0.4 は、編集していない行の半角カナを行の記録で元のバイト列のまま書く (§8.7.1。編集した行は v0.3 と同じく E1 / E3) | ISO-2022-JP |

- **検出は読込時にストリーミングで行う**: UTF-8 以外の文字コードでは、デコードした各チャンクを再エンコードして元バイト列と比較し、不一致を `roundtrip` に記録する。**ファイルサイズによる省略はしない** (読込時間の増分は再エンコード 1 パス分で、S-01 で Shift_JIS 100 MB を実測し NFR-104 の 1.5 倍許容内かを確認する)。ISO-2022-JP では比較単位を **行** にし (区分 D)、再エンコードは保存と同じ方式 (§8.6 E6、OQ-162) で行い、行ごとの出力バイト列を元ファイルの対応バイト範囲 (デコーダが消費したオフセットで区切る) と比較する。v0.4 の検査は、行の区切りを本文 (§2.4) と同じ LF・CR・CRLF にし、保持方式の記録を集める (§8.7.1)。
- **版ごとの記録内容**: v0.1〜v0.3 は `RoundtripSummary { lossy_lines, first_offsets (≤ 100), unmappable_lines, incomplete }` だけ (メモリは定数)。保存時は E2' の確認に使う。**実装は、どの文字コードでも行の単位で数える** (Shift_JIS / EUC-JP も、LF で区切った行ごとに再符号化して元のバイト列と比べる。上の表と文言の「N 箇所」は、この行の数)。v0.4 の保持方式の記録 (`PreservedMap`、§8.7.1) は不一致の範囲の元バイト列を持つ (通常は数バイト〜数 KB) が、不一致が密なファイルでは記録が非常に多くなり得るため、**件数 100 万または元のバイト列の合計 16 MiB を超えたら記録を全部捨てて「保持なし」** にする (安全側。§8.7.1 の「上限」)。
- **v0.1〜v0.3**: `roundtrip` が非空なら読込完了時に情報メッセージ (「この文字コードでは、編集した箇所の一部の文字が保存時に別のバイト列になることがあります」) を出し、該当箇所を Decoration (`DecorationKind::LossyEncoding`) で示す (v0.1 は先頭 100 箇所。**この Decoration は v0.3 までの実装に無い**。v0.4 でも付けない、§8.7.1 の「読込の完了時の通知」)。無編集保存では警告しない (v0.1 は書かない、v0.4 は区分 B で一致するため)。v0.4 の読込完了時の通知は §8.7.1 の「読込の完了時の通知」(保持が効いている文書では出さない)。
- **検査を打ち切った文書** (Issue #495): 比較は行の単位で、1 行を丸ごとバッファに置く。1 行が 4 MiB を超えると、メモリを抑えるために その行から後ろの検査をやめ、`roundtrip` に「打ち切った」ことを記録する (`incomplete`。それまでに数えた行数は残す)。検査していない範囲に区分 B / D の箇所があり得るので、**この文書は件数が 0 でも、区分 B の箇所がある文書と同じに扱う** (安全側): 編集して元の文字コードのまま保存するときに、§8.6 E2' と同じ時点で確認を出し (`ui.confirm.save.roundtrip_unchecked`、04 §15.2。数を言えないので専用の文言。既定は「別名で保存」)、承認した Revision では繰り返さない。全保存では、確認が要る文書として保存しない (E2' の文書と同じ)。無編集の上書き保存は書き込まないので、確認は出ない。**v0.4** (§8.7.1): Shift_JIS / EUC-JP は、1 行が長くても検査を打ち切らない (4 MiB の打ち切りを無くす)。ISO-2022-JP は、1 行が 4 MiB を超えたら v0.3 と同じく検査を打ち切り、その文書は「保持なし」になる (元の文字コードで書くときに同じ確認を出す。未編集でも出す、§8.6 E2')。
- **区分 B の v0.1 での主張**: 無編集の上書き保存は書き込まないため (§9.3 手順 0)、文字コードによらずバイト一致が成り立つ。編集した文書では E2' の確認を経て代表バイト列で書く (区分 C と同じ結果になる)。バイト一致を保持方式で保証するのは v0.4 (01 §5.5 の NFR-303 AC-5〜AC-7: 編集していない範囲 (ISO-2022-JP は ASCII 状態で始まり終わる、編集していない行) がバイト一致する。「保持なし」の文書は除く。§8.7.1)。
- **CP932 互換文字と検索**: encoding_rs (WHATWG) の Shift_JIS / EUC-JP の復号は Windows (CP932) と同じ写像で、0x8160 (EUC-JP 0xA1C1) → U+FF5E (FULLWIDTH TILDE)、0x8161 → U+2225、0x817C → U+FF0D、0x8191 / 0x8192 / 0x81CA → U+FFE0 / U+FFE1 / U+FFE2、0x815C → U+2015 に写し、いずれも再エンコードで元のバイトに戻る (実測、encoding_rs 0.8.35)。互換文字の問題は逆向きに起きる: JIS 系・Mac 系の写像で作られた文字 (U+301C WAVE DASH・U+2016・U+2212・U+00A2・U+00A3・U+00AC・U+00A6・U+2014) は、UTF-8 の文書や他アプリからの貼り付けで文書に入り、(a) Shift_JIS / EUC-JP / ISO-2022-JP で保存すると表現不能になる (U+2212 以外は `Unmappable`、U+2212 は 0x817C に黙って写されて E1 の照合で不一致。扱いは §8.6 の E3 で、「置換して保存」では置換先 (a') の CP932 の文字に置き換える。確定 (ユーザー回答)、OQ-208)。(b) 文書内の「〜」(U+301C) を IME で入力した「～」(U+FF5E) で検索しても一致せず、同じ文書内で見た目が同じ文字のコードポイントが混在し得る。**保存データは変えない** (P1)。検索でこれらを同一視するオプション (`search.cp932_compat_fold`、既定 ON、v0.4) は OQ-41 (確定 (ユーザー回答、2026-11-11)) で、対象は §8.6 の互換文字の表の組 (JIS 系の文字と CP932 の文字) である。この組の一覧を 09 §4 のコーパスに置く。リテラル・正規表現・置換での扱いと検索バーの表示は §12.4.1 (v0.4 の M24。検索の同一視の組は、この表から `¥` / `‾` の 2 行を除いた 8 組で、文書の文字コードに依らない。利用者に見える点は OQ-218、確定 (ユーザー回答、2026-11-12))。
- **CP932 の外字 (ユーザー定義領域)**: encoding_rs の Shift_JIS デコーダは 0xF040〜0xF9FC (1,880 字) を U+E000〜U+E757 (私用領域) に復号するが、エンコーダはこれらを `Unmappable` で返す (復号専用。実測、encoding_rs 0.8.35: 0xF040 → U+E000、0xF9FC → U+E757、U+E000 / U+E757 の符号化は `Unmappable`)。補わなければ外字を含む文書を編集して Shift_JIS で保存すると E1 の表現不能になり、外字のバイト値が失われる。**確定 (ユーザー回答)、OQ-206**: `eve-encoding` が Shift_JIS の符号化で U+E000〜U+E757 を **自前で逆算して書く** (Windows の CP932 の変換と同じ写像。encoding_rs は復号だけに使う)。
  - **式**: encoding_rs の復号は `p = (lead − 0xC1) × 188 + (trail − (trail < 0x7F ? 0x40 : 0x41))`、`cp = 0xE000 + p − 8836` (lead 0xF0〜0xF9、trail 0x40〜0x7E / 0x80〜0xFC)。その逆は、`i = cp − 0xE000` (0〜1879) として `lead = 0xF0 + i / 188`、`t = i % 188`、`trail = t + (t < 0x3F ? 0x40 : 0x41)` (0x7F を飛ばす)。`p = i + 8836` と置けば `lead = p / 188 + 0xC1` と同じ (8836 = 47 × 188)。
  - **例と境界**: U+E000 → 0xF040 (先頭)、U+E03E → 0xF07E、U+E03F → 0xF080 (0x7F を飛ばす)、U+E05E → 0xF09F、U+E0BB → 0xF0FC (lead の行の末尾)、U+E0BC → 0xF140 (次の lead)、U+E3EB → 0xF580、U+E757 → 0xF9FC (末尾)。範囲外 (U+DFFF 以下、U+E758 以上) は逆算しない (encoding_rs の結果どおり。私用領域の残りは表現不能)。0xF040〜0xF9FC の外側 (0xEFFC は encoding_rs で `Malformed`、0xFA40 以降は IBM 拡張) は対象外。
  - **組み込み方**: 保存の符号化 (E1) で encoding_rs のエンコーダが `Unmappable(ch)` を返し、保存先が Shift_JIS で `ch` が U+E000〜U+E757 なら、上の 2 バイトを出力してエンコーダを続ける (エンコーダの状態は無いので作り直さない)。E1 の照合は出力を encoding_rs で復号して行い、同じ文字に戻るので一致する。読込時の区分 B の検出 (再エンコードして元のバイト列と比べる) も同じ符号化器を使うので、外字を不一致に数えない (区分 A)。`JobKind::ReplaceUnmappable` の照合も同じ符号化器を使う (外字は置換の対象にならない)。
  - **検証** (実測、encoding_rs 0.8.35。S-01 (c) で再確認する): lead 0xF0〜0xF9 × trail 188 通り = 1,880 個の全コードで「encoding_rs で復号 → 上の式で符号化 → 元の 2 バイト」が成り立ち、復号結果は U+E000〜U+E757 の 1,880 個 (重複なし) で、encoding_rs のエンコーダはその全部を `Unmappable` で返す (自前の逆算が必要な根拠)。09 L1 に全 1,880 個の往復の単体テストを置く。
  - **副作用**: UTF-8 等で開いた文書中の U+E000〜U+E757 (別の用途の私用文字) も、Shift_JIS で保存すると外字のバイトとして書かれる (Windows の変換と同じ。表現不能として止めない)。
  - **対象外**: EUC-JP の外字領域 (0xF5A1〜0xFEFE) と ISO-2022-JP には WHATWG の写像が無く、復号の時点で不正バイト (§8.3 で保持) になる。EUC-JP / ISO-2022-JP で U+E000〜U+E757 を保存すると表現不能 (E3 の置換先 (b)) になる (encoding_rs は `Unmappable` を返す。実測)。
- **Shift_JIS の単独バイト 0xA0 / 0xFD〜0xFF**: Windows (CP932) は U+F8F0〜U+F8F3 に写すが、WHATWG では誤りで、encoding_rs は `Malformed` を返す (実測)。§8.3 の不正バイトとして保持し、文書は読み取り専用になる (無編集保存はバイト一致)。0x80 は encoding_rs も U+0080 に復号し、往復する。
- 往復コーパス (09 §4) に次を加える。期待は括弧内。(1) CP932 の外字 (0xF040 / 0xF07E / 0xF080 / 0xF09F / 0xF0FC / 0xF140 / 0xF580 / 0xF9FC) を含む Shift_JIS 文書 (区分 A: 無編集はバイト一致、外字の前後を編集して保存しても外字のバイトはそのまま、E2' の確認も E3 も出ない)。(2) 単独バイト 0xA0 / 0xFD〜0xFF を含む Shift_JIS 文書 (不正バイトとして読み取り専用、名前を付けて保存でバイト一致)。(3) CP932 互換文字の復号結果 (0x8160 / 0x8161 / 0x817C / 0x8191 / 0x8192 / 0x81CA / 0x815C / 0xFA55) を含む Shift_JIS 文書と、同じ文字を含む EUC-JP 文書 (0xA1C1 / 0xA1C2 / 0xA1DD / 0xA1F1 / 0xA1F2 / 0xA2CC / 0xA1BD / 0xFCFC) (往復する側: 区分 A でバイト一致)。(4) 互換文字の表の JIS 系の文字 (U+301C・U+2016・U+2212・U+00A2・U+00A3・U+00AC・U+00A6・U+2014・U+00A5・U+203E) と私用文字 U+E000 / U+E757 を含む UTF-8 文書 (Shift_JIS / EUC-JP / ISO-2022-JP への保存で E3 になり、`置換して保存` で互換文字の表どおりに置き換わって `counts.compat` に数えられ、置き換えた文書が保存先で往復すること。`¥` `‾` は ISO-2022-JP では表現不能にならない。U+E000 / U+E757 は Shift_JIS では外字 0xF040 / 0xF9FC で書かれ、EUC-JP / ISO-2022-JP では `?` になる)。(5) JIS X 0208 状態で改行する ISO-2022-JP 文書 (`1B 24 42 24 22 0A 1B 28 42`。FR-111 で ISO-2022-JP を指定して開くと 0x0A が不正バイト `<0x0A>` として見え、読み取り専用で、ISO-2022-JP での保存は拒否、UTF-8 等での保存は E3 でキャンセルだけ)。
- NFR-303 の文言はこの 4 区分と版ごとの範囲 (v0.1 は区分 A + 無編集保存を書かないこと、区分 B の保持方式は v0.4) を前提にする。ISO-2022-JP の例外文は「無編集はバイト一致、編集後はエスケープ選択の差を確認付きで許す (意味の一致)」(02 §13.4 (i))。
- **encoding_rs の ISO-2022-JP 対応範囲と自前実装の要否**: encoding_rs (WHATWG 準拠) のデコーダは `ESC ( B` / `ESC ( J` / `ESC ( I` / `ESC $ @` / `ESC $ B` のみを受け付け、JIS X 0212 (`ESC $ ( D`)・ISO-2022-JP-2 系・`SO` / `SI`・8 bit バイト・出力を挟まない連続エスケープを誤りとする。エンコーダは `ESC $ B` / `ESC ( B` / `ESC ( J` (¥ ‾ 用) だけを出し、半角カナを全角に変換し、JIS X 0212 を出力しない。v0.1 はこの範囲で **自前実装を持たない** (判定 (§8.2 手順 4') の ESC 検出と復号可能性検査、状態の引継ぎ、行単位の区分 B 比較は encoding_rs の API の上に書く薄い層で、`eve-encoding` の依存は encoding_rs / chardetng のまま)。受け付けない入力は不正バイトとして読み取り専用で開く (§8.3)。S-01 (a') の実コーパス (メール・旧 Windows ツール・JIS 規格書由来の文書) で「読み取り専用になるファイルの比率」を記録し、有意なら **デコーダのみ** 自前の寛容実装 (JIS X 0208 / 0212 / 半角カナの表は encoding_rs の EUC-JP デコーダへ `バイト | 0x80` で写して流用し、表を持ち込まない) を v0.2 以降で足す (OQ-138)。エンコーダは正規形を出す encoding_rs のままとする。

#### 8.7.1 保持方式 (v0.4、M22)

本節が v0.4 の保持方式 (ADR-0026 (2)、NFR-303 の区分 B / D) の正本である (2026-11-11 に決定。3 つの視点の検討と、PO の回答による)。読込のときに、**復号して再び符号化すると、元とは別のバイト列になる箇所** の元のバイト列を覚え (以下「記録」)、保存のときに、編集していない記録の範囲は元のバイト列をそのまま書く (§8.6 E2)。v0.1〜v0.3 の「検出して確認する」(§8.6 E2'、上の「版ごとの記録内容」) は、v0.4 では元のバイト列を覚えていない文書 (下の「保持なし」) の確認に縮める。

- **保持する対象** (理由を問わない): (a) **重複符号化** (CP932 の NEC 選定 IBM 拡張・IBM 拡張など。再符号化すると代表バイト列になる。EUC-JP で WHATWG が別のコードに写す文字 (例: 0x8FA2B7 → U+FF5E → 0xA1C1) も同じ): 理由は `DuplicateEncoding`。(b) **復号専用の文字** (EUC-JP の JIS X 0212 など。符号化器が出力できない): `DecodeOnly`。JIS X 0212 は保持の対象である (v0.3 までの検査は、JIS X 0212 を含む行を `unmappable_lines` に数え、`lossy_lines` に入れていなかった)。(c) **ISO-2022-JP のエスケープの選び方の差** (区分 D): `StatefulEscape`。(d) **ISO-2022-JP の半角カナを含む行** (符号化器は半角カナを出力できない): `DecodeOnly`。v0.3 までは (d) を区分 D の対象にせず E1 / E3 だけで扱った (§8.6 E6) が、v0.4 では編集していない行を元のバイト列で書く。**対象外**: CP932 の外字 (自前の逆算で戻る) と CP932 互換文字 (再符号化で戻る) は、これまでどおり区分 A。不正バイトは §8.3 の PUA エスケープで保持する (別の仕組み)。
- **不変条件** (どの記録も満たす): 記録の元のバイト列は、**その文字コードの初期状態から復号すると、記録の範囲の今の文字列になり、初期状態で終わる** (ISO-2022-JP の初期状態は ASCII 状態)。この性質があるので、記録を並べても前後の編集と干渉しない (記録の前後を符号化した出力とつないでも、復号の結果が変わらない)。保存の確認 (§8.6 E2 の (i)) と性質テスト (§22.1) は、この条件を見る。
- **保持の単位**: Shift_JIS / EUC-JP (状態を持たない) は **文字の範囲**。再符号化で違いが出る連続した文字を 1 件にする (行の中で隣り合うものは 1 つにまとめてよい)。ISO-2022-JP (状態を持つ) は **行の内容** (行の終端を含まない) で、**ASCII 状態で始まり、ASCII 状態で終わる行だけ** を対象にする。行の始まりの状態は、行ごとに ASCII と仮定せず、**本文を復号した復号器の実際の状態** (エスケープを追って求める) で判定する。ASCII 状態で始まらない・終わらない行 (ローマ字状態のまま改行する行など) は記録せず、保存のときは正規形になる (その行は下の「保持できなかった箇所」に数える)。
- **検査の行と長い行**: 検査の行の区切りは、本文 (§2.4) と同じ LF・CR・CRLF にする (v0.3 までの検査は LF だけで区切っていた)。Shift_JIS / EUC-JP では、1 行が長くても検査を打ち切らない (上の「検査を打ち切った文書」の 4 MiB の打ち切りを無くす)。ISO-2022-JP は、1 行が 4 MiB を超えたら v0.3 と同じく検査を打ち切り、その文書は「保持なし」になる。
- **集める側と持つ側**: eve-encoding が読込の検査で記録を集め、(行番号, 行の中の位置, 長さ, 元のバイト列, 理由) の形で返す (位置と長さは、復号した本文 (UTF-8) のバイト数)。eve-core の読込のジョブがそれを文書の位置に直し、`JobResult::Complete` の `preserved` で返す (02 §9.3)。文書は記録を **`Document` のフィールド `preserved: PreservedMap`** に持つ (`TextFormat` には入れない。`TextFormat` は編集のたびに複製されるため)。`TextFormat.roundtrip` (`RoundtripSummary`) は要約 (件数と状態) のまま残す。型 (§8.1): 文書が持つ表は `PreservedMap`、1 件は `PreservedSpan`、理由は `PreserveReason { DuplicateEncoding, DecodeOnly, StatefulEscape }`。データ構造の細部 (塊に分けて共有する形など) と、eve-encoding が返す型の名前は M22 の実装の PR で決めるが、次は仕様とする: **編集 1 回の更新の費用は、変更の数と触れた記録の数に比例し、記録の総数に比例しない** (NFR-105)。保存のジョブには、保存を始めた時点の記録の版を、本文の Snapshot と一緒に渡す (渡した後の編集の影響を受けない。§9.3 手順 4)。
- **読込の完了時の照合**: 読込のジョブは `Complete` を返す前に、全部の記録について「範囲の文字列 = 元のバイト列を初期状態から復号したもの」を照合する。合わない記録が 1 件でもあれば、その文書の記録を全部捨てて「保持なし」にする (実装の誤りへの備え。本文を含まないログを残す。eve-core にログの出口が無い間 (v0.4 の M22 の 5 の時点。02 §9.2 の `LogSink` は Host の部品) は、状態 `Unavailable(VerifyFailed)` だけを残し、debug ビルドでは止める。Undo / Redo の適用で表と履歴が食い違ったとき (起きないはず) も同じく、件数を数えて debug ビルドで止める)。
- **上限**: 記録の件数が **100 万**、または元のバイト列の合計が **16 MiB** のどちらかを超えたら、**記録を全部捨てて**「保持なし」にする (一部だけ残すことはしない)。上限に達した時点で、検査も打ち切ってよい。
- **「保持なし」と「保持できなかった箇所がある」**: 次のどれかに当たる文書は **「保持なし」** とし、v0.3 と同じ扱いに落ちる: (a) 記録が上限を超えた、(b) 検査を打ち切った (ISO-2022-JP の 4 MiB を超える行)、(c) 読込の完了時の照合に失敗した、(d) 緊急退避・スワップから復旧した (下の「緊急退避・スワップ・復旧」)、(e) 記録を集めるメモリを確保できなかった。ASCII 状態で始まらない・終わらない ISO-2022-JP の行のように、**一部だけ保持できなかった箇所がある文書** は、記録は残したまま「保持できなかった箇所がある」とし、(a)〜(e) と同じ確認 (§8.6 E2') の対象にする。状態は `TextFormat.roundtrip` の要約に持つ (`RoundtripSummary.preserve: PreserveStatus`、§8.1。M22 の 5 で決定)。読み取り専用にはしない (eve-view の `ReadOnlyReasonView` は変えない)。v0.3 までの仕様で記録の件数の超過に置いていた「別の文字コードを求める」状態は置かず、この「保持なし」に置き換えた。
- **保存の前の確認 (§8.6 E2')**: 保持が効いている文書では出さない。「保持なし」の文書 (読込時の検査で区分 B / D の箇所を見つけた、または数を確かめられなかったもの) と「保持できなかった箇所がある」文書を、元の文字コードで書くときだけ出す。**文書が未編集でも出す** (未編集の文書を同じ文字コードで「名前を付けて保存」する・`file.set_encoding` で同じ文字コードを選ぶ場合。未編集の上書き保存は書き込まないので出ない)。時点・既定・全保存での扱い・文言の選び方は §8.6 の E2' の行。
- **読込の完了時の通知**: 保持が効いている文書では、v0.3 までの情報メッセージ (`info.open.lossy_roundtrip`。「別のバイト列になることがあります」) を出さない。「保持なし」(区分 B / D の箇所を見つけた、または数を確かめられない) と「保持できなかった箇所がある」の文書では、読込の完了時に情報メッセージを 1 回出す (M22 の 6 で決定: 「保持なし」は数を言わない `info.open.preserve_off`、「保持できなかった箇所がある」は数 (`unpreserved`) を言う `info.open.preserve_partial`。04 §15.2。出す条件は、保存の前の確認 (§8.6 E2') の、編集していない文書の条件と同じ (「保持なし」は区分 B / D の箇所を見つけた (`lossy_lines ≥ 1`。復号専用の文字の行だけの文書は除く) か検査を打ち切った文書と、照合に失敗した (`VerifyFailed`) 文書だけ。理由が `InvalidBytes` の文書には出さない)。文字コードを指定して開き直したとき (§8.8) も出す。再読込 (§9.5) では出さない)。Decoration (`DecorationKind::LossyEncoding`) は v0.4 でも付けない (M22 の 6 で決定。v0.3 までの実装にも無い。`first_offsets` は元のファイルのバイト位置で、編集の後の文書の位置に写せないため。後の版の候補)。
- **編集での写像**: 編集 (Transaction の適用、§6.5 手順 4) のたびに、各記録の範囲を `map_range` (§6.4) で写す。変更と交差しない範囲 (`Mapped`。端に接する削除を含む) は、位置を写して残す。**範囲の内側に掛かる編集・範囲をまたぐ編集** (`Inside` / `Straddle`) は、その記録を外す。**範囲の端にちょうど接する挿入** (`Touch`) は、文字の範囲の記録 (Shift_JIS / EUC-JP) では残し (範囲は挿入した文字を含まない)、行の記録 (ISO-2022-JP) では外す (行の内容が変わるため)。**長さ 0 の記録** (ISO-2022-JP の、冗長なエスケープだけの行。本文では空の行) は、その位置に接する変更があれば、削除でも外す (削除の前後の点が同じ位置になり、Undo で元の位置を一意に戻せないため。外した記録は Undo で戻る)。外した記録は、その Transaction の Undo のノードが持つ (§7.1 の `preserved_removed`)。ISO-2022-JP の記録が、前の行の終端を消したなどで保存のときに 1 つの行の内容の全体でなければ、その記録は使わずに、その行を編集した行と同じく正規形で書く (§8.6 E2 の (d)。記録は表に残り、Undo で行が戻れば使う)。**隣り合う区分 B の文字は 1 件の記録にまとまる** (上の「保持の単位」) ので、その並びの内側への挿入は、並びの全体を編集したものと扱う (`Inside` で記録が外れ、並びの全部が代表バイト列になる。並びの端への挿入は `Touch` で記録を残す。M22 の 6 のレビューで決定。記録を分けて残すのは後の版の候補、§23)。
- **Undo / Redo** (PO の回答: 編集して Undo で戻した範囲は「未編集」と扱う): 編集で外した記録は、その Transaction の Undo のノードが持ち、**Undo で本文と一緒に戻し、Redo で再び外す**。Undo / Redo の Transaction (`tx.is_undo`) の適用では、写像で記録を外さずに位置だけを写す (本文が編集の前・後と同じになるので、記録の集合も編集の前・後と同じになる。§22.1 の保持方式の (d))。ノードを結合 (§7.3) したときは、外した記録も合わせて持つ。逆変換が、変更と変更の間の変わっていない範囲も一緒に置き換えるノード (大量のすべて置換の粗い逆変換 (§12.5)。文書全体を置き換える再読込・復旧も同じ) は、位置を写せないので、置き換える範囲にあった編集の前の記録を全部持ち、Undo でその範囲の記録を入れ替える (Redo は、編集の後の記録に入れ替える)。ノードが持つ記録の元のバイト列のバイト数は、Undo の予算 (§7.5) に数える。予算で捨てたノードと、新しい編集で捨てた redo のノードが持つ記録は、ノードと一緒に捨てる。内容が同じ文字列を打ち直しても、記録は戻さない (戻すのは Undo / Redo だけ)。
- **経路を 1 か所にまとめる**: 本文を変える経路 (通常の編集、Undo / Redo、再読込、復旧、改行コードの変換、すべて置換、E3 の置換など) は、どれも `Document::apply` (§6.5) の手順 4 で記録を更新する。経路ごとに別の更新を書かない。
- **再読込・開き直し・文字コードの変更**: 再読込 (§9.5) は、記録を新しい読込の結果に入れ替える。再読込の Undo (§7.2 の `format_change`) は、本文・書式と一緒に前の記録も戻す。文字コードを指定して開き直す (§8.8) と、新しい読込の記録になる。`file.set_encoding` と「名前を付けて保存」は、書く文字コードが記録を集めたときの文字コードと **同じなら記録を使う** (§8.6 E2)。違う文字コードで保存が成功したら、記録を捨てる (Undo のノードが持つ記録と、再読込・復旧のノードが持つ前後の表も捨てる。Undo で別の文字コードの記録が表に戻らないように。表を捨てた前後の状態は「保持なし (捨てた)」(`NoPreserveReason::Discarded`) にするので、その後に Undo で前の文字コードの書式に戻った文書は、元の文字コードで書く前の確認 (§8.6 E2') の対象になる。M22 の 5 で決定)。同じ文字コードで保存が成功しても、記録は **捨てない** (ディスクに元のバイト列を書いたので、記録は有効なまま)。v0.3 までの「保存が成功したら `roundtrip` を空にする」(§8.8) は、違う文字コードへ変えたときだけにする。
- **保存**: §8.6 の E2 (記録が駆動する差し替えと、確認 (i)・(ii)) と E2' (確認の条件)。保存・乾式走査・置換 (`JobKind::ReplaceUnmappable`) の対象の判定は、Snapshot と記録の版から決まる **同じ 1 つの計画** を使い (チャンクの区切りに依らない)、乾式走査と本番の書き込みは同じ手順で出力を作る (§9.3 の「乾式走査と出力長」)。**使わなかった記録** (§8.6 E2 の (d) を満たさない行の記録。前後の行の終端を消して行が変わった ISO-2022-JP の行の記録) は、保存の後も表に残す (M22 の 6 のレビューで決定。記録が表すのは「範囲の今の文字列の、元のバイト列」でディスクの内容ではなく、保存のたびに計画が (d) を見て使わないだけなので害は無い。外すと、行を結合して保存 → Undo → 保存で元のバイト列に戻らず、01 §5.5 の NFR-303 AC-7・上の「Undo / Redo」・§22.1 の (d) と食い違う)。**確かめに合わずに保存が止まった** (`JobError::VerifyFailed`。§8.6 E2 の (c) と表の不変条件の違反、確認 (ii) の不一致) ときは、core が文書の表と履歴の記録を全部捨てて「保持なし」(`NoPreserveReason::VerifyFailed`) にする (同じ表を渡し続けると同じ文字コードで二度と保存できないため。次の保存は E2' の確認 (`roundtrip_unchecked`) を経て、記録を使わずに v0.3 と同じ出力で書く。M22 の 6 のレビューで決定)。
- **緊急退避・スワップ・復旧** (PO の回答: 元のバイト列は残さない): manifest の形式 (02 §11.2.1) は変えず、緊急退避・スワップに元のバイト列を書かない。復旧を適用した文書 (§10.3) は記録を失い、「保持なし」になる (v0.3 と同じ確認に落ちる)。復旧の Undo では、上の「Undo / Redo」のとおり記録も戻り、保持の状態も復旧の前に戻る (前の状態をノードに持たせる。M22 の 5 で決定: `Origin::Recovery` の Transaction について `FormatChange::Recovery { before, after }` (前後の表と状態。表の複製は O(1)) をノードに持たせる。v0.4 の M23 から、スワップの復旧は呼び出し側が `Document::recovery_change` で (manifest から決めた `escape_encoding` などと一緒に、§8.3.1) 作って Transaction に渡し、渡されなければ `Document::apply` が表と状態だけを変える形で作る。本文が同じ復旧 (恒等の Transaction) は何も変えない)。復旧を「前後の共通部分を残して差し替える」形にして記録を生かす案は、v0.4 では採らない (後の版の候補。§23)。
- **計測** (v0.4 の M22 の 7。開発機での参考値で、判定は基準機の `cargo xtask bench --env`。09 §5.2 の BM-104 / BM-105 / BM-202 / BM-107e の「v0.4 (参考)」): 区分 B の箇所が密な 10 MB のファイル (記録 16 万〜29 万件) の読込は、記録を集めない読込 (v0.3 と同じ経路) の約 0.8〜1.1 倍 (Shift_JIS 約 1.0 倍、EUC-JP 約 1.1 倍、ISO-2022-JP 約 0.8 倍) で、読込の後のメモリの増分は記録の分 (Shift_JIS の 29 万件で約 6 MB) 多い。Shift_JIS の 100 MB で記録が約 75 万件の文書の保存のジョブは、同じ文書を記録を使わずに書く保存より 1 割ほど短い (元のバイト列の範囲は符号化と照合をしない)。記録が約 69 万件の 100 MB の文書の中央への 1 文字の挿入と Undo は p50 20〜40 µs (記録の無い文書は 5〜10 µs) で、記録の総数に比例しない。M22 の 6 の時点 (読込は記録を集めない読込の約 2.3 倍、保存は記録を使わない保存の約 2.1 倍) から、記録ごとに rope を根から降りない (本文を前から順に読む)、記録・区間ごとに作業用の文字列と復号器を作らない、確認 (i) を計画で 1 回だけ行って結果を型 (`PreservedBytes`。確認を経ずには作れない) で符号化器の口へ運ぶ、1 文字の再符号化の結果を覚える、の 4 つで縮めた。
- **実装の PR の分け方** (この順に入れる。08 §2.5 の M22): (1) 保存の乾式走査と本番の書き込みの符号化のループを 1 つにまとめる (振る舞いを変えない)。(2) 読込時の検査の行の区切りを LF・CR・CRLF に揃える。(3) eve-encoding で記録を集める (上限・長い行・分類の変更を含む)。(4) `PreservedMap` (写像・共有・性質テスト)。(5) 読込・編集・Undo / Redo・再読込・開き直し・`set_encoding` への組み込み。(6) 保存での差し替えと二重の確認、確認 (E2') の条件の変更。**この PR は、仕様との整合・データの損失の 2 つの視点のレビューに掛ける**。(7) コーパス・端末の試験・計測。

### 8.8 文字コードの変更 (FR-111)

- 「指定して開き直す」(`file.reopen_with_encoding`): dirty なら実行前に保存か破棄を求める (01 FR-111 AC-2。FR-107 と同じ 3 択で、保存を選んだら保存の成功後に開き直す。§9.7)。Document を破棄せず、同じ `DocumentId` で読込ジョブ (手順 2 の明示指定付き) を再投入し、新しい本文は **旧本文とは別に構築** する。読込中は `load_state = Loading` で **旧本文を表示し続け** (編集はできない)、完了時に本文・`TextFormat`・読み取り専用の理由 (`InvalidBytes` 等の内容由来のもの) を一括で差し替え、**履歴を消して新しい根を作り、`saved` = 根** (dirty = false) にする。**開き直しは Undo できない操作** とし、書式 (文字コード・BOM・改行の統計) と読み取り専用の理由は本文と一緒に置き換える (確定 (ユーザー回答))。理由: 本文だけを Undo で戻すと、文字コード・不正バイトの読み取り専用・保存点が本文と食い違い (例: Shift_JIS で開き直した後に Undo すると、不正バイトを含む旧デコード結果が編集可能になる。NFR-304 に反する)。dirty でないときだけ実行するので、失うのは保存済みの編集の Undo 履歴だけで、未保存の内容は失わない。完了時に「開き直したため Undo 履歴を消去しました」と 1 回通知する。保持方式の記録 (v0.4) も新しい読込の結果に入れ替える (§8.7.1)。外部変更の再読込 (§9.5) は Undo 可のまま (自分の編集を取り戻すため) で、書式も一緒に戻す (§7.2)。
  - 開き直しのキャンセル (`ui.cancel`)・失敗では、旧本文・旧 `TextFormat`・旧い読み取り専用の理由・履歴をそのまま保ち、`load_state = Ready` に戻す (エラーはメッセージで示す)。
  - 開く前の確認 (512 MiB 超・不正バイトの比率、`JobResult::NeedsConfirm`) は通常の読込 (§9.2) と同じ規則で出す。「中止」は上のキャンセルと同じ。明示指定で読むので文字コードの判定 (§8.2 手順 1 の BOM の判定 (指定した文字コードの BOM と一致すれば読み飛ばす) と手順 3〜8) とバイナリ・UTF-32 の BOM の確認は行わない (バイナリの確認から開き直した文書の `Binary` の理由は外れる)。
  - **読み取り専用の理由の差し替え** (開き直しと再読込 (§9.5) の両方の正本。§7.2 と §9.5 手順 5 はここを参照する): `FileAttribute` は新しい `FileMeta.readonly` で付け直し、`UserRequested` と `NoWritePermission` は保つ。内容由来の理由のうち `InvalidBytes` / `StatefulEncodingWithInvalidBytes` は新しい読込の結果で付け直す (v0.4 の M23: `file.toggle_read_only` で外した `InvalidBytes` は、**再読込では** 解除の記録 (`Document.invalid_bytes_released`) があれば付けず、解除を持ち越す。**開き直しでは** 新しい読込に不正バイトがあれば付け直し、記録を消す。§8.3.1)。`Binary` と `LargeFileViewer` は、**開き直しでは** 新しい読込の結果で付け直し (バイナリの確認から開き直した文書の `Binary` は外れる)、**再読込では保つ** (明示指定の読込ではバイナリの判定をせず、`LargeFileViewer` の文書は `confirmed_large` と `read_only_requested` を立てて読み直すため、§9.5 手順 3)。`LoadIncomplete` (v0.4 の M25。標準入力の読込を途中でやめた文書、§9.2.2) は、開き直し・再読込のどちらでも **必ず外す** (読み直した内容はファイルの全体。パスを持つのは名前を付けて保存の後だけで、その成功の時点で既に外れている)。再読込で付け直した理由は前後の値を `format_change` に持ち、Undo / Redo で本文と一緒に戻る (§7.2)。開き直しは Undo できない。
  - 保存ジョブの実行中は、その完了を待ってから dirty を判定する。
- 「指定して保存する」(`file.set_encoding`): 引数の `encoding` と `bom` から新しい format (`encoding` / `has_bom` だけを変え、他は元のまま) を作り、**今の format は変えずに** 保存ジョブにその format を渡す。`TextFormat` は **保存が成功したときだけ** 新しい format に確定し、失敗・キャンセル・`Conflict`・表現不能 (E3 の 3) で終わったら元のまま (メタデータの変更なので Undo の対象外、§7.2)。手順 0 (未編集なら書かない、§9.3) は適用せず、常に書く。表現不能のチェックは §8.6。
  - `bom` の扱い (既定値の正本は 07 の `file.set_encoding` の引数): 省略したときは **UTF-16LE/BE は BOM 付き、UTF-8 は今の文書の BOM の有無を保つ** (`file.write_bom` には依らない)。BOM を持たない文字コード (Shift_JIS / EUC-JP / ISO-2022-JP) では `bom` を **無視** する (`has_bom = false`。指定しても誤りにしない)。
  - **v0.1〜v0.3**: 保存が成功して `TextFormat` を新しい format に確定したら、`roundtrip` (`RoundtripSummary`) は旧文字コードでの検出結果なので、そのままにしない: 新しい文字コードが UTF-8 / UTF-16 (区分 A) なら空 (`RoundtripSummary::default()`) にし、それ以外 (区分 B / D の文字コード) なら、書いたバイト列は E1 の照合を通った代表バイト列なので同じく空にする (書いた内容を再エンコードすれば同じバイト列になり、無編集の再保存でバイト一致する)。`invalid_bytes` / `pua_escape` は保存の前提 (§8.3) により変わらない。旧 `roundtrip` を元に出していた `DecorationKind::LossyEncoding` の Decoration も消す。**v0.4** (§8.7.1): 書く文字コードが記録を集めたときの文字コードと **同じなら**、保持方式の記録を使って書き (§8.6 E2)、保存が成功しても記録と要約を **捨てない** (ディスクに元のバイト列を書いたので、記録は有効なまま)。**違う文字コード** で保存が成功したときだけ、記録を捨て、要約を上の v0.3 と同じく空にする。同じ時点で `escape_encoding` を `None` にする (v0.4 の M23。この後に Undo で戻ったエスケープ文字を元の文字コードのバイトとして別の文字コードのファイルに書かない、§8.3.1)。「名前を付けて保存」(書式を変えない) も同じ規則で、同じ文字コードなので記録を使い、捨てない。
  - 保存先: パスを持ち、読み取り専用でない文書は今のパスに上書き保存する (`Expect::Stamp(disk_stamp)`、§9.3)。無題の文書と読み取り専用の文書は、名前を付けて保存の流れ (§9.7 (a)) に新しい format を渡す。

### 8.9 対応しない文字コード

**確定 (ユーザー回答、[OQ-03](10-decisions.md#oq-03-mvp-の文字コード対応範囲))**: 文字コードの対応範囲は、v0.1 は FR-108 / 109 の 7 種、FR-110 (v0.4 から送り、v0.5 の計画で v0.6 以降に送った、08 §7) で encoding_rs (WHATWG Encoding Standard) が扱うもので、これ以上広げない。次の表の文字コードには **対応しない**。需要が出てから自前実装を検討する (その場合は新しい OQ を起こす)。対応しない文字コードのファイルも **黙って壊さない** (P1): 下の「開いたときの挙動」は既存の判定 (§8.2)・不正バイトの保持 (§8.3)・読み取り専用の規則から決まるもので、追加の実装は UTF-32 の BOM の検出 (§8.2 手順 1) だけである。

| 文字コード | 対応しない理由 | 開いたときの挙動 (v0.1) |
|---|---|---|
| **JIS X 0213 系** (Shift_JIS-2004、EUC-JIS-2004、ISO-2022-JP-2004 / -3。第 3・第 4 水準漢字) | encoding_rs / WHATWG に無い。対応には符号表と往復の規則を自前で持つ必要がある | JIS X 0208 の範囲は Shift_JIS / EUC-JP / ISO-2022-JP として読める。第 3・第 4 水準の文字は不正バイト (Shift_JIS・EUC-JP) として §8.3 で保持され、文書は読み取り専用 (NFR-304)。ISO-2022-JP-2004 の `ESC $ ( Q` 等は §8.2 手順 4' の復号可能性検査を通らず、7 bit なので手順 6 で ASCII (UTF-8) として開く (エスケープシーケンスが `<ESC>` 等で見え、ASCII として往復する)。FR-111 で ISO-2022-JP を指定して開き直すと、復号できない箇所を不正バイトとして読み取り専用で開く |
| **UTF-32** (LE / BE、BOM 有無) | encoding_rs / WHATWG に無い。テキストファイルとしての流通が少ない | v0.1 は開く対象に含めない。BOM 付き (`FF FE 00 00` / `00 00 FE FF`) は §8.2 手順 1 で UTF-16LE の BOM より先に検出し、本文を読まずに `ConfirmAsk::UnsupportedBom { kind }` で「UTF-32 には対応していない」と案内して、FR-115 と同じ選択肢 (読み取り専用で開く / 文字コードを指定して開く / 中止) の確認を出す。BOM なしは NUL が多いので §8.2 手順 4 でバイナリと判定され、同じ確認になる。UTF-16LE の BOM の直後に U+0000 が来る文書は UTF-32LE と判定されるが、FR-111 の明示指定で UTF-16LE として開き直せる (明示指定は手順 1 に優先し、`FF FE` を UTF-16LE の BOM として読み飛ばして続く `00 00` を U+0000 として読む。§8.2 手順 2) |
| **UTF-7** | WHATWG が安全上の理由で除外 (XSS に使われた)。現在ほぼ使われない | 7 bit なので §8.2 手順 6 で UTF-8 (ASCII) として開く (`+ZeVnLIqe-` のような表記がそのまま見える)。ASCII として往復するのでバイト一致は保たれる |
| **ISO-2022-JP の補助漢字等** (ISO-2022-JP-1 の JIS X 0212 `ESC $ ( D`、ISO-2022-JP-2 の `ESC $ A` / `ESC $ ( C` / `ESC . A` 等、`SO` / `SI` を使う変種) | encoding_rs の ISO-2022-JP デコーダは受け付けない (§8.7 末尾)。寛容なデコーダの自前実装は OQ-138 で別途判断 | §8.2 手順 4' で ISO-2022-JP と確定せず、7 bit なので手順 6 で ASCII (UTF-8) として開く (手順 7 の chardetng には届かない。ASCII として往復する)。FR-111 で ISO-2022-JP を指定して開き直すと、復号できない箇所を不正バイトとして読み取り専用で開く (§8.3 の状態付き符号化の規則)。8 bit バイトが混在するものは手順 7 に届き、chardetng が ISO-2022-JP と推定すれば同じく読み取り専用で開く |
| その他 WHATWG に無い文字コード (EBCDIC 系、ISO-2022-KR / CN、HZ 等) | 同上 | 判定 (§8.2) の結果に従い、多くは `file.default_encoding` (既定 UTF-8) として不正バイトを含む文書 (読み取り専用) かバイナリになる (chardetng が 7 種外の文字コードを推定しても採らない、§8.2 手順 7 の (iii)) |

- 保存側の制約: これらの文字コードを「文字コードを指定して保存」(FR-111) の選択肢に出さない (encoding_rs のラベル一覧に無いので自然に出ない)。
- EUC-JP の JIS X 0212 (補助漢字) は **対応範囲内** (読める・エンコーダは出力しないので区分 B / C、§8.6 / §8.7) で、本表の対象ではない。CP932 の外字も対応範囲内で、`eve-encoding` の自前の逆算で書き戻すので区分 A (§8.7、確定 (ユーザー回答)、OQ-206)。不正バイトを含む ISO-2022-JP の文書は読めるが ISO-2022-JP では保存しない (§8.3、確定 (ユーザー回答)、OQ-207)。
- WHATWG にあるが v0.1 の 7 種に無い文字コード (windows-1252・GBK・Big5・EUC-KR 等) は FR-110 (明示指定。v0.6 以降、08 §7) の対象で、本表の対象ではない。v0.1 で chardetng がこれらを推定した場合は採らず、`file.default_encoding` で開いて不正バイトを §8.3 で保持する (§8.2 手順 7 の (iii))。

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
| F9 | **保存中フラグの列挙** (置換途中のクラッシュ痕跡。フラグが示すパスの `.<name>.eve-tmp-*` / `.<name>.eve-bak-*`) | `inflight_for(path) -> Vec<InflightRecord>` (v0.1、1 パス分のフラグだけを読む)、`list_inflight()` (v0.2。02 §4.5)。一般の `read_dir()` は v0.5 | §9.2 手順 2 (**開くたびに** metadata の結果に依らずそのパスのフラグを調べる。v0.1、05 §4.6)、§10.3 (起動時の一斉走査は **保存中フラグ** が示すパスについてのみ。ディレクトリの列挙は行わない。v0.2) |
| F10 | 外部変更の比較キー | `FileStamp { mtime, len, file_id, hash }` (02 §4.5。05 §5.5 が計算規則) | §9.3 / §9.5 の `disk_stamp` / `external_modified` |
| F11 | 「退避先に保存」の書込 (退避ディレクトリの作成を含む) と退避ディレクトリの走査 (場所・名前・壁時計・pid は Port の実装が決める) | `write_recovery()` / `list_recovery_dirs()` (v0.1。02 §4.5) | §9.7 (e) の「退避先に保存」(`WriteRecovery`)、§10.1 の「復旧」(`RecoveryScan`) |

要求する性質: (a) すべて同期 API、(b) `Send + Sync` (ジョブから呼ぶ)、(c) エラーは 02 §4.5 の構造体 `FsError { kind: FsErrorKind, detail: Option<FsErrorDetail>, source }` で、core は `kind` (と必要なら `detail`) で分岐する (`NotFound`、`PermissionDenied`、`Busy` = 共有違反・ロック (再試行は 05 §4.2)、`InvalidPath`、`DiskFull`、`ReadOnlyFs`、`Other`、保存の `Conflict { current: Option<Box<FileStamp>> }` = `SaveOptions.expected` 不一致 (`None` = ファイルが無い)、`AtomicUnsupported` = 原子的置換不可 (何も書いていない)、`SourceFailed`、`ReplacedMovedToBackup { backup }`、`BackupFailed`、`InPlaceFailed { backup: Option<FsPath> }` (事前バックアップを省いた (`SkipConfirmed`) ときは `None`))。キャンセルは `FsErrorKind` に含めず、ジョブの結果 `JobError::Cancelled` (02 §9.3) だけで表す。Windows の置換途中失敗 (`ERROR_UNABLE_TO_MOVE_REPLACEMENT_2`、元ファイルが退避名に移動済み) は 02 §4.5 の `ReplacedMovedToBackup { backup }` で退避パスを添えて返る (OS 側の検出は 05 §4.2)。

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
           interrupted_save: Option<InterruptedSave { method: SaveMethod /*NewFile | Atomic | InPlace*/, backup: Option<FsPath>, tmp: Option<FsPath>, complete: bool }>
           を作る (02 §4.5 / §9.3)。記録から InterruptedSave への写像と MessageKey は下の「中断された保存の写像」の表が正本。
             v0.1 は pid の生存確認をしない (他インスタンスが保存中の記録でも報告する。誤報は害が小さい側、05 §4.6)。
             main は interrupted_save が Some なら表の MessageKey (04 §15.2) で場所を示して警告する。
             文書は通常どおり開く (空の新規文書として黙って開かない)。復旧の操作は v0.2 (§10.3)
           ・NotFound で OpenOptions.must_exist (v0.5 の M31。ファインダ・横断検索から開くとき、§12.7.4 / §12.7.5) → JobResult::Failed { err: JobError::Fs(NotFound) }
             (main: msg.finder.not_found を出して Document を閉じる。新規の文書にしない)
           ・NotFound → JobResult::NewFile { interrupted_save, parent_missing } (main: 空の新規 Document、dirty=false として確定)。
             親フォルダが無ければ parent_missing にそのフォルダを入れ、main は `msg.open.parent_missing` で示す (フォルダは作らない。保存は §9.3 の ParentMissing になる。01 FR-101 AC-2)
           ・FileMeta.kind が File でない (Dir / Other) → JobResult::Failed { err: JobError::Fs(FsError) } (main: エラー表示、Document を閉じる)
           ・len > file.huge_file_warn_bytes (§11.2) かつ OpenOptions.confirmed_large でない → JobResult::NeedsConfirm { ask: ConfirmAsk::HugeFile { len } } を送って **ジョブを終了** する
             (len 以下でも、デコード後の見積り・読込中に伸びた分で閾値を超えれば手順 3 の「読込中のメモリ検査」で同じ確認を出す)。
             main は回答 (読み取り専用で開く / 中止。FR-127) を得たら、「読み取り専用で開く」なら OpenOptions { confirmed_large: true, read_only_requested: true, .. } で
             読込ジョブを再投入する (下の「確認からの再投入」)。確認中に io ワーカー (02 §9.1 の 1 本) を占有せず、他の文書の保存を止めない。回答待ちの状態は Document 側に持つ
  2b.[job]  meta.file_id と canonicalize(path) を取得し、非終端の JobResult::Identity { file_id, file_id_unique, canonical, stale } を送る (stale は識別が古いと確かめた文書、§12.7.6 の 0)
           (v0.2.0〜v0.4 の実装は Identity の中間結果を送らず、二重オープンは開く前のパスの比較だけだった、02 §9.3。**v0.5 の M31 で入れた (SAF-28、§12.7.6)**。metadata と canonicalize は手順 2 の HugeFile の確認より前に行い、Identity を確認 (NeedsConfirm) より前に送る。
            NotFound (新規のファイル) では親を canonicalize してファイル名を付けたものを canonical にして、NewFile の前に送る。判定のやり直し (§8.2.1) の読み直しでは送り直さない)
           main: 既に同じファイルの Document (読込中・確認待ちを含む) があれば (信頼できる FileId とボリュームの一致、または canonical のパスの一致、§12.7.6 の 0)、新 Document を閉じて既存を開いた時点のペインに出し、ジョブをキャンセル。
                 開いたパスと既存のパスが違えば info.open.same_file (「{path} は {other} として開いています」) で知らせる
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
           ・UTF-8 以外: 再エンコード照合で roundtrip を記録 (§8.7、ストリーミング。v0.1 は件数 + 先頭 100 箇所)。
             v0.4 は保持方式の記録も集め、文書の位置に直し、読込の完了時の照合を済ませて Complete の preserved で返す (§8.7.1)
           ・16 MiB 以下: 内容ハッシュをストリーミング計算 (05 §5.5 の比較キー)
           ・開く操作から 50 ms 以内に最初の JobResult::Progress { bytes, total } を送り、以後は **最大 10 Hz** (受信口に積まない。02 §9.2)
           ・(v0.4 の M25) 本文の読込を始めてから 200 ms を過ぎても読み終えていなければ、その後の最初のチャンクの区切りで 1 回だけ
             JobResult::Partial { head, format } (固定の先頭 1 MiB。非終端) を送る (§9.2.1。OpenOptions.partial が立っているときだけ。core が新しく開く読込で立てる)
           ・CancelToken (02 §9.3) をチャンク毎に確認 (FR-122)
           ・**読込中のメモリ検査** (OpenOptions.confirmed_large でないとき。手順 2 の判定は metadata の len だけなので、読込中に伸びたファイル・
             デコードで膨らむファイルを見逃さないため): (i) sniff の直後に「デコード後の見積り」= len × 文字コードの倍率 (Shift_JIS・EUC-JP は 1.5、
             UTF-16 は 1.5、UTF-8 は 1) × (1 + 3 × sniff 範囲の不正バイトの比率) (不正バイト 1 つは PUA のエスケープで UTF-8 の 4 バイトになる、§8.3 / §11.7) を求め、
             huge_file_warn_bytes を超えれば本文の構築の前に止める。(ii) 各チャンクの後に「デコード後のバイト数 (構築中の本文の UTF-8 のバイト数)」と
             「読んだバイト数」の大きい方を huge_file_warn_bytes と照合し、超えたらその時点で止める。どちらも
             JobResult::NeedsConfirm { ask: ConfirmAsk::HugeFile { len } } を送ってジョブを終了し (構築中の本文は捨てる)、`len` には判定に使った値を入れる。
             回答と再投入は手順 2 の HugeFile と同じ (下の「確認からの再投入」)。
             **未実装**: v0.3 までのコード (`io_jobs.rs` の `open_inner`) にあるのは手順 2 の metadata の大きさの確認と `probe_reserve` だけで、
             (i)・(ii) は無い。v0.4 の M25 でも作らない (先行表示 (§9.2.1) の「確認」の行は、(ii) が実装されたときの扱い)
           ・**読む量の上限**: 読込は metadata の len + 余裕 (max(1 MiB, len / 16)) で打ち切る (読込中に伸び続けるログ等を際限なく読まない)。
             打ち切ったら changed_during_load = true とし、disk_stamp には読込前の stamp を入れる (ディスクの今の内容と一致しないので、
             そのまま上書き保存すると手順 5a の照合で Conflict になり、読んだ分だけでファイルを黙って切り詰めることはない。05 §4.5)
           ・§8.2.1 の再判定が起きたら先頭からやり直し (1 回限り)
           ・読込前後の metadata を比較し、変化していれば Complete に `changed_during_load = true` を付ける (05 §4.5)
  4. [job]  JobResult::Complete { text: Text, format: TextFormat, meta: FileMeta, stamp: FileStamp, changed_during_load, detected: ReadOnlyReasons, interrupted_save,
                                  preserved: PreservedMap /* v0.4、§8.7.1 */ }
  5. [main] Document.text = text; (v0.4) Document.preserved = preserved; load_state = Ready (v0.4: Partial からの切り替えではカーソル・選択・スクロールを引き継ぐ、§9.2.1); 読み取り専用の理由 (ReadOnlyReasons、§9.6) を下の「付く読み取り専用の理由の決め方」の表で付ける;
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
| `ManyInvalidBytes { ratio, encoding, memory_factor }` | `ui.confirm.open.invalid_bytes` (`HugeFile` とは **別のキー**) | 読み取り専用で開く (既定) → `confirmed_invalid_bytes = true`, `read_only_requested = true` (理由は `pending_reasons` に入れず、読込の結果 (`Complete.detected`) が `InvalidBytes` / `StatefulEncodingWithInvalidBytes` を付ける。§8.2.1 の再判定で文字コードが変わり不正バイトが 0 になったときに、理由を残さないため。`UserRequested` は付けない (v0.4 で直した)) / 文字コードを指定して開く → `forced_encoding = Some(enc)` (`confirmed_invalid_bytes` は立てない。指定した文字コードでも 1% を超えればもう一度確認する) / 中止 | `InvalidBytes` (ISO-2022-JP なら `StatefulEncodingWithInvalidBytes`。`LargeFileViewer` は付けない) |

  - `read_only_requested` は確認で「読み取り専用で開く」を選んだことを表し、開くジョブは以降の **バイナリ・UTF-32 の BOM・不正バイトの比率 (`ManyInvalidBytes`) の確認を出さずに続ける** (読み取り専用で開くことが決まっているので二重に確認しない。本文は判定した文字コード (バイナリ・UTF-32 の BOM では UTF-8) で復号し、不正バイトは §8.3 で保持する。NUL は `<0x00>` で表示、§9.6)。確認を省いた判定は、ジョブが理由として `Complete.detected` に入れる (下の表)。巨大ファイルの確認は `confirmed_large` で決まる。`--readonly` (`OpenRequest.readonly`) ではこの値を立てず、`open_initial` が最初の投入で `pending_reasons` に `UserRequested` を入れるだけで、確認は通常どおり出す。
  - `forced_encoding` は §8.2 手順 2 の明示指定で、文字コードの判定 (手順 1 の BOM の判定 (指定した文字コードの BOM と一致すれば読み飛ばし、一致しなければ本文のバイトとして扱う) と手順 3〜8) とバイナリ・UTF-32 の BOM の確認を行わない。巨大ファイルと不正バイトの比率の確認はフラグが立っていなければ出す (§8.8)。
  - `pending_reasons: ReadOnlyReasons` は確認の回答と `--readonly` で決まった理由を main が入れて運ぶもので、ジョブは読まずに引き継ぐ (`read_only_requested` の値だけでは、どの確認で立てたかが分からないため。理由の保持場所はここだけにする、02 §9.3)。

**付く読み取り専用の理由の決め方** (手順 5。02 §9.3 の「確認の回答と読み取り専用の理由」と同じ規則): 文書の `read_only` は `OpenOptions.pending_reasons` ∪ `Complete.detected` ∪ (`meta` から決まる理由) とする。

| 由来 | 理由 | 決め方 |
|---|---|---|
| 確認の回答・開く要求 (`pending_reasons`、main が入れる) | `LargeFileViewer` / `Binary` / `InvalidBytes` / `StatefulEncodingWithInvalidBytes` / `UserRequested` | 上の表で「読み取り専用で開く」を選んだときの理由。`--readonly` (`OpenRequest.readonly`、07 §18.3) は `UserRequested` |
| 内容 (`Complete.detected`、ジョブが入れる) | `InvalidBytes` / `StatefulEncodingWithInvalidBytes` / `Binary` | `format.invalid_bytes > 0` で状態付きでなければ `InvalidBytes`、状態付き符号化 (ISO-2022-JP) なら `StatefulEncodingWithInvalidBytes` (§8.3。確認を経ずに開いた場合 (比率が 1% 以下・文字コードを指定して開いた) も付く、NFR-304)。閾値超の行は理由にしない (v0.1 にあった `LongLine` は v0.2 で外した。§9.6、§11.5.1)。`read_only_requested` でバイナリ・UTF-32 の BOM の確認を省いたときは、その判定が当たれば `Binary` |
| ファイル (`meta`、main が決める) | `FileAttribute` | `meta.readonly` (05 §5.2) |

- **メインスレッドの I/O はゼロ**: `stat` / サイズ確認 / canonical 比較はすべてジョブで行う。「開く」の体感は Loading 表示が即出る分だけ良くなり、小さいローカルファイルでは手順 2〜5 が数 ms で完了するため違いは見えない。読込ジョブは `io` スレッドで直列に実行する (02 §9.1)。
- **Partial (v0.4 の M25)**: 読込が一定の時間を過ぎても終わらないときだけ、本文の固定の先頭を読み取り専用で先に表示する (最小の形)。規則の正本は下の §9.2.1。**v0.1〜v0.3 では作らない** (`load_state` の値の名前 `Partial` だけを決めてある。v0.3 までのコードの列挙は `Loading` / `Ready` / `Failed` の 3 つで、v0.4 で足す)。v0.1 の 100 MB は「開く操作から 50 ms 以内に進捗を表示し、入力に応答し、キャンセルできる。全読込は 30 回計測の p50 1.0 s で判定し、最大値は参考値 (目安 1.5 s) として記録する」(01 NFR-104) で判定する (§11.3)。
- **閉じる手順**: 文書を閉じるとき (v0.1 は次の「置き換えの手順」で旧文書を閉じるとき、v0.2 からは `buffer.close` も) は (a) 実行中の **保存ジョブは完了を待つ** (閉じた後でも結果を受け取る。閉じた文書の保存の結果は 02 §9.3 の採否表のとおり採用し、成功なら何もしない)。保存が失敗したら、その保存ジョブに渡した Snapshot (`DocSnapshot`) から「退避先に保存」できるようにする: core は閉じた文書の `DocSnapshot` を **その保存ジョブの完了まで保持** し (緊急退避のスロットも保存の完了まで空にしない。§10.1)、失敗なら **確認を出さずに退避先へ書く** (`JobKind::WriteRecovery` に保持した Snapshot を渡す。閉じた文書なので別名で保存・再試行・破棄の選択肢は出さず、内容を失わない側に倒す。書いた場所はメッセージ行で示す)。退避の完了で Snapshot とスロットを解放する (提案、OQ-199。2026-11-04 に実装 (v0.2) に合わせて「退避先に保存 / 破棄」の 2 択の確認から改めた)。(b) 読込・検索・構文・スワップのジョブは **キャンセル** し、結果が届いても `DocumentId` が無効なので破棄する、(c) `Loading` 中の文書を閉じるときは読込ジョブをキャンセルして即閉じる (確認不要: 未保存変更は存在しない)、(d) dirty なら FR-107 の確認。ただし置き換え (下の「置き換えの手順」) の `AskUnsaved` で既に「破棄」を選んだ文書は **確認済みの印** (`discard_confirmed`。その確認の後に編集が無いこと (Revision が同じ) が条件) を付けて閉じ、FR-107 の確認を二重に出さない。
- **置き換えの手順 (v0.1 の単一文書。v0.2 は §9.8 の `file.open` / `file.new` の行のとおり置き換えず足す。01 FR-107 AC-3)**: v0.1 は表示も操作も 1 文書だが、文書の集合 (`EditorState.documents`) は集合のまま持つ。`file.open` / `file.new` は (1) 今の文書が dirty なら FR-107 と同じ 保存 / 破棄 / キャンセル を確認する (キャンセルなら何もしない。保存を選んだら保存が成功したときだけ (2) へ進み、保存が成功しなければ置き換えを中止する。§9.7)、(2) 新しい Document を集合に加えて上の手順 1〜5 で読み込み (`file.new` は無題の Document を作るだけ)、その間も旧文書を表示し続ける。**読込の間、旧文書は編集を受け付けない** (読み取り専用として扱う。`ReadOnlyReasons` には入れず「置き換えの読込中」であることから導出し、編集のコマンドは `msg.edit.read_only` で拒否する。(1) で保存・破棄を確認した後の編集が (3) で黙って捨てられるのを防ぐ。表示の文言・when 句の追随は 02 §5.1 / 04 §15.2)。(1) で「破棄」を選んだ場合は旧文書に確認済みの印を付け、(3) で閉じるときに FR-107 の確認を二重に出さない (上の「閉じる手順」(d))。(4) で置き換えを中止したら印を外し、編集の受付を戻す。(3) 新しい文書が `Ready` (または手順 2 の NewFile) になった時点で View の対象を切り替え、旧文書を「閉じる手順」で閉じる、(4) 読込が失敗・キャンセル (読込中の `ui.cancel` (`Esc`)。FR-122 AC-1 のキャンセル手段) されたら新しい Document を閉じ、旧文書をそのまま残す。手順 0 / 2b で開こうとしたファイルが今の文書と同じなら、何もせず「既に開いています」と示す。緊急退避 (§10.1) は集合のどの文書も対象にするので、置き換えの途中に異常終了しても旧文書の内容を失わない。
- `+行番号` / `path:行:桁` (FR-102) は v0.2 で入れ、Complete を待ってジャンプする (07 §18.2)。
- **読込中のメモリ**: チャンク + Decoder バッファ + 構築中 rope。全文 `Vec<u8>` を作らない。読込用の事前確保など **利用者のデータ量で決まる確保には `try_reserve`** を使い、失敗したら読込ジョブは `JobError::OutOfMemory { requested }` を返し、core は「メモリが足りないため中止した」(`msg.open.out_of_memory`、04 §15.2) と知らせてプロセスは続行する (OOM で abort する経路を減らす。安定版 Rust では確保失敗をフックできない)。v0.3 で `try_reserve` にする確保は 2 つ: (a) 読み始める前に、ファイルの大きさ (`meta.len`) 分の確保を試してすぐ解放する (rope の構築 (ropey) は確保の失敗を返せないため、その見込みの検査として行う。最適化で消えないようにする)、(b) チャンクごとの復号のバッファ (チャンクの 3 倍)。rope の節の確保そのものは失敗を返せない (見込みの検査を通った後の不足は従来どおり abort)。見込みは `meta.len` の分だけで、復号で増える分 (UTF-16 / Shift_JIS では約 1.5 倍)・rope の葉の余り・再読込中に残る旧本文は含まない。Linux の overcommit の下では、検査を通っても実際の不足を検出できないことがある。再読込 (§9.5) も同じジョブなので同じ。
- **stdin** (FR-120、v0.4 の M25): 復号・文字コードの判定・改行の統計は同じ関数で行うが、開くジョブの種類 (`JobKind::OpenStdin`)・走らせるスレッド・確認・上限・取消し・変更ありの扱いが違う。規則の正本は下の §9.2.2。
- **失敗** (v0.1): 読込ジョブが `JobResult::Failed` (途中の I/O エラーを含む) で終わったら、読めた部分は捨て、エラーをメッセージで示す。(i) 置き換えの途中 (上の「置き換えの手順」(4)) なら新しい Document を閉じて旧文書をそのまま残す。(ii) 起動時のパス指定 (`Editor::open_initial`、02 §5.4) の失敗・キャンセルなら、その Document を閉じて無題の文書で開始する。再試行の専用コマンドは持たず、利用者がもう一度 `file.open` を実行する。読めた部分だけを読み取り専用で開く選択肢 (理由 `LoadIncomplete`、§9.6) は v0.1 では出さない (ファイルでは v0.4 でも出さない。v0.4 で `LoadIncomplete` を付けるのは標準入力の読込を途中でやめたときだけ、§9.2.2)。

#### 9.2.1 先行表示 (Partial。v0.4 の M25、FR-122 AC-2〜AC-4)

**最小の形** (docs/review/v0.4-plan.md §1.2 の決定): 読込が一定の時間を過ぎても終わらないときだけ、本文の **固定の先頭** を読み取り専用で先に見せる。先頭は読むにつれて伸ばさない。基準機の 100 MB は UTF-8 が約 180 ms で読み終わり (`docs/perf/0.3.0/`)、多くは先行表示を出さずに全文を出す。効くのは Shift_JIS (約 420 ms)・遅いディスク・ネットワーク上のファイル。利用者に見える点は [OQ-219](10-decisions.md#29-oq-176-以降-並行修正での追加起票) の 1〜3 (確定 (ユーザー回答、2026-11-13)。すべて推す案のとおりで、本節はその内容)。実装の PR の分け方は 08 §2.5 の M25 の行が正本。

| 項目 | 規則 |
|---|---|
| 対象 | core が **新しく開く** 読込 (`file.open` と起動時のパス) だけ。core は投入のときに `OpenOptions.partial` (02 §9.3) を立て、ジョブは `partial` が立っているときだけ `Partial` を送る (確認からの再投入 (§9.2) は元の指定を引き継ぐ)。再読込 (`file.reload`、§9.5) と文字コードを指定して開き直す (§8.8) は立てない (読込の間は旧本文を表示し続けるので要らない)。標準入力 (§9.2.2) も対象にしない (`OpenStdin` は `Partial` を送らない) |
| 出す条件 | 開くジョブが本文の読込 (手順 3 の `open_read`) を始めてから **200 ms** (定数 `PARTIAL_AFTER`。設定キーにしない) を過ぎても読み終えていないとき、その後の最初のチャンクの区切りで **1 回だけ** `JobResult::Partial` を送る (下の「判定のやり直し」を除く)。時刻はジョブの `JobPorts.clock` で測る。200 ms は、BM-104v の p50 300 ms (01 FR-122 AC-2) に、送ってから描くまで (先頭の木の構築 約 1 ms と Frame) と揺れの分の余裕を残し、UTF-8 の 100 MB (約 180 ms) では多くの場合に出さない値。sniff と開く前の確認 (手順 3 のバイナリ・UTF-32 の BOM・不正バイトの比率) は最初のチャンクで済むので、先行表示はそれらの後にしか出ない |
| 先頭 | その時点までに復号した本文 (UTF-8) の先頭 **1 MiB** (定数 `PARTIAL_HEAD`) まで。**行の区切りで切る**: 1 MiB の中の最後の行の終端 (LF、CRLF、後ろが LF でないと分かっている CR) の直後。復号した文字列の末尾の CR は、次のチャンクの先頭が LF かもしれないので行の終端に数えない (CRLF を先頭と残りに分けない)。1 MiB の中に行の終端が無ければ (長大行の途中)、1 MiB の位置から **書記素の境界** (§13.1。境界の判定には 1 MiB の位置の次の char まで見る) まで戻して切り、そこで末尾のバイトが CR なら、その CR の前で切る (CRLF を分けない)。1 MiB の位置が復号済みの末尾に当たるときは、最後の書記素を含めない (後ろに結合する文字が続くかもしれないため)。ジョブは復号した文字列の先頭 (1 MiB と 1 チャンク分まで) を別に複写して持ち、そこから先頭の `Text` を作る (構築中の木 `TextBuilder` に触れない)。**先頭は全文の接頭辞** (同じ復号の結果の先頭のバイト列) である |
| 結果の形 | `JobResult::Partial { head: Text, format: TextFormat }` (02 §9.3。非終端で、この後も `Progress` を送る)。`format` は先頭の時点の形式で、表示に使うだけ (文字コード・BOM・不正バイトのエスケープの扱いは判定の値、改行の統計・不正バイトの数・長大行の数は先頭の分)。保持方式の記録・往復の要約・内容ハッシュ・`changed_during_load`・読み取り専用の理由は送らない (完了で決まる) |
| 状態 | main は `Partial` を受けたら、文書の `load_state` を `Partial` にし、本文を先頭にする (`Loading` → `Partial` → `Ready`、または閉じる)。`Partial` は読込中の一つで、`Loading` と同じく編集できず (`read_only` の理由には入れない、§9.6)、dirty でなく、保存・スワップ・緊急退避の対象にならない (§10.1 / §10.2 の「`Loading` の文書」に含める)。`Query::DocumentText` / `Selection` は `NotLoaded` を返す (`LoadStateView::Partial`、02 §10.1 の `Query`)。ジョブの採否は `Open` の行のとおり (02 §9.3) |
| Revision | 先頭・2 回目の先頭・全文の文書は、どれも差し替える前の文書 (`Loading` の空の文書・前の先頭) の **続きの Revision** を振る (今のコードの `put_document` (`editor/state.rs`) が読込の完了・開き直しで行っている規則と同じ。docs/review/v0.3-findings.md の SAF-06、§17.1 の文書ごとの単調増加)。そのため (文書, Revision) を鍵にした覚え (行の表示単位・可視範囲の強調) と、差し替えの前に投入したジョブの結果は、自動で古くなる (同じ鍵で別の本文を引かない)。§14.4 の長大行の索引の覚えは行頭の位置を鍵にして Revision をまたいで引き継ぐので、差し替え (2 回目の先頭・全文) のときに明示して捨てる |
| 表示の切り替え | 先行表示の時点で、読込の完了と同じ条件 (§9.8 の `file.open` の行: 開いたときに表示していた文書 (切り替え元) をまだ表示していれば) で、表示を新しい文書へ切り替える。切り替えるのは **切り替え元がある読込 (`file.open`) だけ**: 起動時の最初のパスは最初から読込中の文書を表示しているので、そのまま先頭を出す。起動時の 2 つ目以降のパス (§9.8 の「読込中のバッファ」。表示しないバッファとして足す) は、先行表示でも表示を変えない。**オーバーレイ (入力のプロンプト・確認・パレット・バッファ一覧・ヘルプ等。04 §7.5) か検索バーが開いている間は、自動の表示の移動 (先行表示と完了の切り替え、下の「失敗・取消し・閉じる」の表示の戻し、完了で無題のバッファを閉じること) を保留** し、閉じた後に (その時点でも条件を満たしていれば) 行う (入力の途中で下の文書が替わらない・一覧の行がずれない・パレットで選んだコマンドが開いたときと別の文書に効かないため。§9.8 の「検索バー・オーバーレイを開いたままの切替」)。自動の切り替えで表示を移したら、ほかの待っている読込の切り替え元を移した先に書き換える (§9.8 の `file.open` の行の「切り替え元の書き換え」)。オーバーレイで実行する操作 (名前を付けて保存など) の対象は、オーバーレイを開いた時点の文書である (§9.7・§9.8 の「入力・確認の対象」。#534 で入れた)。先行表示で切り替えた後の完了では切り替え直さない (先行表示の間に利用者が元の文書へ戻したら、完了でもそのまま)。**何も入力していない無題のバッファ** (§9.8 の `file.open` の行の「例外」) から切り替えたときは、先行表示の時点ではその無題のバッファを閉じず (表示から外すだけ)、読込の完了の時点でまだ何も入力していなければ閉じる (置き換えの完了。一覧の番号を増やさない。利用者がその無題のバッファへ戻して表示しているときは、切り替え直さないので閉じない)。読込が失敗・取消し・確認での中止で終わったら、その無題のバッファを表示に戻す (同じ番号のまま。新しい無題の文書を作らない)。判定のやり直し (2 回目の先頭) と確認 (下の行) の後も、表示の切り替えはそのまま保つ (切り替えた後なら先頭を差し替えた文書を表示し続け、切り替える前なら条件はそのまま。確認の時点で保留していた切り替えは落とし、確認の後に本文の無い `Loading` の文書へ切り替えない。回答で投入し直した読込が先頭を送った時点で、改めて切り替えを見る) |
| できる操作 | スクロール・カーソルの移動・選択・選択範囲のコピー (先頭の中)・バッファの切替・閉じる・読込の取消し (`Esc`、`ui.cancel`。FR-122 AC-1。取り消す範囲は §9.8 の「読込中のバッファ」)・パレット・ヘルプ・終了。先頭の末尾より先へ動こうとしたときは末尾に止め、`info.open.partial_end` を出す (同じ文書の先行表示の間は 1 回だけ)。「先へ動こうとした」は次の前進の移動 (選択を伸ばす版を含む) で決める: `cursor.doc_end` (常に)、末尾の行での `cursor.move_down` / `select.extend_down`、末尾の行に着いた `cursor.page_down` / `select.extend_page_down`、先頭の末尾での `cursor.move_right` / `cursor.move_word_right` (と選択を伸ばす版)、末尾の行が行の途中で切れている (長大行の途中で切った先頭) ときにそこへ着いた `cursor.end` / `select.extend_end`。行の区切りで切った先頭の末尾の行 (空) での `End` と、ホイール・スクロールのコマンドでは出さない |
| できない操作 | 編集 (Undo / Redo を含む) は `Loading` と同じく `msg.edit.read_only_loading` (読込を待つか `Esc` で取り消すかの案内。04 §15.2) で拒否する。保存・名前を付けて保存・再読込・文字コードを指定して開き直す・文字コードの変更・改行コードの変換は `Loading` と同じく実行しない (黙って拒否せず、`info.reload.busy` で読込中であることを示す。名前を付けて保存は名前を尋ねる前に拒否する)。**検索・置換・行へ移動 (`cursor.goto_line`)・すべて選択 (`select.all`)・文書の末尾まで選択を伸ばす操作 (`select.extend_to_doc_end`)** は、先頭の中だけの結果が文書全体の結果に見えるので、`msg.open.partial_unavailable` で拒否する (検索バーを開かない。開いていた検索バーは §9.8 の切替の規則で閉じている)。同じ理由の拒否が続くときは 1 回にまとめる (表示中のメッセージが同じなら出し直さない)。拒否するのは **新しく開く読込の文書** (先行表示の `Partial` と、本文の無い `Loading`。M25 の前は空の本文を探して黙って 0 件になっていた) だけで、再読込・開き直しの間の `Loading` (旧本文を表示し続ける。§9.5 手順 3、`io/reload.rs`) は今のまま検索等を許す (旧本文の全体が対象なので、結果は全体の結果) |
| 全文への切り替え (`Complete`) | 先頭の文書を全文の文書で置き換え、`load_state = Ready` にする (手順 5。Revision は上の「Revision」)。カーソル・選択・スクロールの位置 (`ScrollPos`)・上下の移動の目標の列は **引き継ぐ** (先頭は全文の接頭辞なので、バイト位置がそのまま通る)。ただし全 View の選択とカーソルに **書記素の境界への寄せ** (`snap_to_grapheme` 相当。§4.1 の正規化) を通す (先頭の末尾の書記素に、全文では結合する文字が続くことがあるため)。起動時の `+行` / `path:行:桁` (§9.8) は完了の時点で移る (先行表示の間に動かしたカーソルより優先)。完了の知らせ (`info.open.*`・`msg.open.changed_during_load` 等) は今と同じ |
| 判定のやり直し (§8.2.1) | 先頭を送った後に判定をやり直す (先頭から読み直す) ことになったら、やり直した読込は時間の条件を待たずに、先頭が揃った時点 (1 MiB の位置の次の char まで復号した、最初のチャンクの区切り) で (揃う前に読み終えたら終端の結果の直前に。そのときは、読み終えた全文が 1 MiB 以下なら全文を、超えれば上の「先頭」の切り方 (ファイルの末尾の CR は行の終端に数える) で切った先頭を送る) `Partial` を送り直す。先頭を送る前に判定をやり直したときは、やり直した読込も上の「出す条件」のとおりに送る (200 ms の起点は最初の読込の `open_read` のまま)。main は 2 回目の `Partial` で先頭を差し替え (Revision は続き。§14.4 の覚えも捨てる)、全 View のカーソル・選択・スクロールを文書の先頭に戻す (表示の切り替えはそのまま。上の「表示の切り替え」)。これで `Complete` の時点の先頭は、常に最後の復号の接頭辞になる |
| 確認 (`NeedsConfirm`) | **読込中のメモリ検査 (§9.2 手順 3 の (ii)) が実装されたときの扱い** (v0.3 までのコードの `open_inner` にあるのは手順 2 の metadata の大きさの確認と `probe_reserve` だけで、(i)・(ii) は無い。M25 でも作らないので、それまで先行表示の後に確認は来ない): main は先頭を捨てて `Loading` に戻し (本文は空。全 View の選択とスクロールを 0 に戻す。表示の切り替えは上の「表示の切り替え」のとおり保ち、保留していた切り替えは落とす)、確認を出す。回答で再投入した読込は、改めて上の条件で `Partial` を送る |
| 失敗・取消し・閉じる | 読込の失敗 (`Failed`)・取消し (`Esc`)・確認での中止・文書を閉じる・終了は、先頭も捨てる (上の「失敗」の「読めた部分は捨てる」と同じ。先頭だけを読み取り専用で残す選択肢は出さない)。表示していた文書を閉じるときの表示の移し方は §9.8 の `file.open` の行と同じ (開く前の文書 (上の「表示の切り替え」の無題のバッファを含む)、ほかの文書、新しい無題の文書の順)。読込の失敗・取消し・確認での中止で表示を移すのは、オーバーレイ・検索バーが閉じるまで保留する (先頭はその時点で捨て、本文の無い `Failed` の文書を表示したまま待つ。閉じた後に文書を閉じて表示を移す。保留の間に利用者が別の文書へ移していたら閉じるだけ)。表示を戻したときは、ほかの読込の切り替え元を書き換えない (戻した先を基準に、続けて開いた読込が表示を奪わない側に倒す) |
| 外部の変更 | 先行表示の間にファイルが変わっても先頭は差し替えない。読み終えた全文で `changed_during_load` を今と同じく判定する (05 §4.5) |
| ステータスバー | 読込中の項目 (`StatusKind::Loading`) を、先行表示の間は `status.partial.long` / `.short` (例 `[先頭のみ 45%]` / `45%`) で出す (04 §8.7。境界の型は変えない)。表示中の文書が `Loading` / `Partial` のときは、その文書の進捗を出す (`ui_model/chrome.rs`。`status.partial.*` にするのは表示中の文書が先行表示のときだけで、表示していない文書の読込の進捗は先行表示でも `status.loading.*`)。行番号の欄の幅は、先頭の行数をファイルの大きさ (`Progress.total`) に比例させた推定の行数から決め (§11.4)、完了で確定する (推定が外れると欄の幅が変わる) |
| 計測 | BM-104v (09 §5.2): 「開く」コマンドの受付 → 先頭を含む最初の Frame の `write` (先に読み終えたら全文の最初の Frame)。`bench-trace` の区間 `open.content` (キー入力の読取完了 → 本文の無い Frame の後で本文が出た最初の Frame の `write`) で測り、その Frame が先行表示の先頭 (読込の進捗も出ている) なら印 `open.partial` も記録する (09 §3.6 の「計測の記録」) |

**採らなかった案**: (1) 読むにつれて先頭を伸ばす: 状態の遷移 (先頭の差し替えのたびのカーソル・スクロールの扱い) が増える。速い機械では 300 ms の前に読み終わるので得るものが小さい (v0.4-plan §1.2)。(2) 時間の条件を置かず、読込の始めから先頭を出す: UTF-8 の多くのファイルで、先頭 → 全文の 2 回の切り替えが数十 ms の間に起きてちらつく。(3) 先頭の中だけで検索・行へ移動を許す、または完了まで保留して完了で実行する (v0.3 までの §11.3 の「保留」): 前者は件数と移動先が文書全体の結果に見え、後者は利用者が待っている間に別の操作をしたときの順序の規則が要る。拒否して、完了を待つか取り消すかを利用者に任せる (OQ-219 の 1)。(4) 先頭と全文を同じ (初期の) Revision にして、覚えを差し替えのたびに捨てる: 差し替えの前に投入したジョブの結果を Revision で古いと判定できず、今のコードの SAF-06 の規則とも食い違う。

#### 9.2.2 標準入力 (FR-120。v0.4 の M25)

`cmd | evoeditor -` で、標準入力を EOF まで読んだ内容を無題の文書として開く。パイプは閉じないことがあり (`tail -f`)、内容は読み直せない (ファイルと違い、取り消すと取り戻せない) ので、ファイルの読込 (§9.2) と次の点を変える。利用者に見える点は [OQ-219](10-decisions.md#29-oq-176-以降-並行修正での追加起票) の 4〜10 (確定 (ユーザー回答、2026-11-13)。すべて推す案のとおりで、本節はその内容。★ は決めた案の印)。実装の PR の分け方は 08 §2.5 の M25 の行が正本。

| 項目 | 規則 |
|---|---|
| 書き方 | `evoeditor -` (07 §18.2)。`-` は 1 つまで (2 つ以上は使い方の誤り。TUI を始めずに `stderr.usage.generic` を出して終了コード 2)。ファイルと一緒に書ける (`evoeditor a.txt - b.txt`。書いた順にバッファを足し、最初のものを表示する。§9.8 の起動時の複数パス)。`--` の後ろの `-` も標準入力 (v0.3 までの CLI は `--` の後ろをすべてファイルとして扱っていた。M25 の (9) で変えた)。`-` という名前のファイルは `./-` (Windows は `.\-` も) と書く。`--readonly` と `--encoding` は標準入力の文書にも掛かり、`+行` は直後の `-` に掛かる (後置 (`- +10`) なら直前の `-`。07 §18.2 の FILE と同じ)。`--headless --rpc stdio` と `-` の併用は使い方の誤り (標準入力は RPC が使う。07 §18) |
| 起動の経路 | CLI の `-` は、`open_initial_files` の `InitialFile` の標準入力の種類として core に渡す (02 §5.4)。標準入力の種類は **Host が判定** し (下の「標準入力が端末」と「書く手段の無い文書」)、`InitialTarget::Stdin(StdinKind)` で core に伝える (`Terminal` (端末)・`File` (ふつうのファイル。`evoeditor - < file`)・`Stream` (それ以外。パイプ・デバイス)。v0.5 の M29 で `Stdin { terminal: bool }` から替えた)。端末なら `StdinSource` を作らずに「標準入力は端末」と core に伝える (`StdinKind::Terminal`)。core は空の無題の文書を開いて `info.stdin.terminal` を出す。端末でなければ Host が `StdinSource` (02 §4.4) を作り、core は無題の文書を `Loading` で作って `JobKind::OpenStdin` (02 §9.3) を投入する。Host が `StdinSource` を作れなかった (移し替えの後に `eve-stdin` を作れなかったときを含む、05 §9.1 / §9.2) ときは `JobPorts.stdin` に入れずに同じく投入し、読込は「最初から読めない」(下の「読込の失敗」) で終わる (M25 の (9) で実装) |
| 標準入力が端末 | 読まずに空の無題の文書を開き、`info.stdin.terminal` を出す (07 §18.5)。判定は Unix が `isatty(0)`、Windows が標準入力のハンドルに `GetConsoleMode` が成功すること (コンソール) と、MSYS / Cygwin の擬似端末 (Rust の `std::io::IsTerminal` と同じ判定: パイプの名前が `msys-` / `cygwin-` で始まり `-pty` を含む) (05 §9.1 / §9.2)。`NUL` / `/dev/null` は端末ではなく、読んで空 (EOF) になる (理由の無い空の文書。メッセージは出さない) |
| 端末の初期化の時期 | ★ (OQ-219 の 9): 標準入力が端末でないとき、Host は端末の初期化 (raw モード・代替スクリーン。02 §9.5 の手順 4) を、標準入力の最初のデータか EOF (または上限・読取の失敗) が `eve-stdin` から届くまで遅らせる。待つ間は端末を変えず、stderr に `stderr.stdin.waiting` 「標準入力を待っています (Ctrl+C で中止)」(04 §15.2。言語ファイルの前に出すので bin の定数表に置き、ほかの stderr の行と同じく英語の行と日本語の行の 1 組) を出す。出すのは最初のデータか終わりが 100 ms の間に届かないときだけ (`cat file | evoeditor -` のようにすぐ届く入力で、終了後の画面に待ちの行を残さない。M25 の (9) で決めた)。待ちは `PlatformStdin::wait_ready` (`eve-stdin` が最初の知らせをチャネルへ入れるまで。データは読み捨てない) で、Host (bin) は終了の要求の受信 (コンソールハンドラ・シグナルの登録) をこの待ちの後に入れる。**標準入力を移す前に端末を確かめる** (M25 の (9)): Host (bin) は標準入力を私有のハンドル / fd へ移して読み始める前に、端末を使えるかだけを確かめる (`TERM=dumb` でないこと、Unix は `/dev/tty` を開けること、Windows は `CONIN$` / `CONOUT$` を開けて `GetConsoleMode` が成功すること。モードは変えない。05 §9.1 / §9.2)。使えなければ標準入力を読まずに、端末の初期化の失敗と同じ経路 (`stderr.not_a_terminal`、終了コード 5) で終わる (`ssh host 'cmd | evoeditor -'` を `-t` なしで実行したとき、読んだ分を黙って失わない)。Host の組み立ての失敗し得る段 (作業ディレクトリの取得・ワーカーの起動) も、標準入力を移す前に済ませる。**既知の制約**: 確かめた後の端末の初期化 (VT への切替・raw モードの設定) が失敗したとき (VT に対応しないコンソールなど) は、移した標準入力の読んだ分を失う。待つ間の Ctrl+C は OS の既定 (Unix は SIGINT の既定の動作、Windows はコンソールの既定の処理) で終わる (まだ文書も端末の変更も無く、失うものが無い)。理由: パイプの書き手が同じ端末でパスワードを尋ねる (`sudo`・`ssh`・`gpg` の pinentry) とき、書き手の入力が先に終わる。同時に始めると、同じ端末の termios を両方が保存・復元して競り、終わった後もエコーが切れたままになる (WSL2 で再現)。**どちらの案でも**、読み終えた (EOF・上限・取消し・失敗) ら、端末のモードを入れ直して全面を描き直す (05 §11.1 の SIGCONT の再初期化と同じ経路。書き手が端末のモードを変えたり stderr に書いたりして画面が崩れるため)。Host は `StdinDone` を core に渡した後に `Frontend::reinit_terminal` (02 §9.2) を呼ぶ (Unix は SIGCONT と同じ再初期化の印、Windows はコンソールの入出力のモードと代替スクリーンを入れ直す。パニック (`Failed`) では呼ばない) |
| 文書 | 無題の文書 (`path = None`。番号は §9.8 の無題の採番。保存は名前を付けて保存になる)。作った時点は `Loading` で、ステータスバーは読んだバイト数を出す (読んだ量はジョブの `Progress` で知る。**最初のデータの進捗は、受け取った直後 (文字コードの判定と復号の前) に間引かずに送る**: 閉じないパイプで最初のチャンクの後に書き手が黙ると以後の進捗が来ず、core が「1 バイト以上読んだ」を知らないまま、閉じる・終了の確認が出ずに捨てるため。その後の進捗は 10 Hz 以下に間引くので、黙ったパイプでは表示の量が実際より少ないことがある) (`status.loading_bytes.long` / `.short`、例 `[読込中 12.3 MiB]`)。割合は出さない。大きさが分からないかどうかは **文書の種類 (標準入力の読込)** で判定する (`Progress { total: 0 }` は空のファイルでも出るので、`total` では判定しない)。先行表示 (§9.2.1) はしない |
| 読込の持ち方 (core) | 標準入力の読込は、ジョブ表 (`Pending`) で **ファイルの読込 (`Pending::Open`) とは別の種類** (`Pending::OpenStdin`) に持ち、`open_in_progress` (置き換えの読込中の判定)・`cancel_own_loading` / `cancel_hidden_loading` (`Esc`)・`loading_percent` (ステータスバー) の対象から外す (ファイルの読込のための判定に、閉じないパイプの読込を混ぜない)。文書は「標準入力の読込」の欄 (02 §5.1 の `Document`: 読んだバイト数・読み込んだだけの印) を持つ |
| 読む仕組み | Host が作る Port `StdinSource` (02 §4.4) から 1 回だけ取れる `ReadChunks` を、開くジョブ `JobKind::OpenStdin` が読む。走らせるスレッドは ★ (OQ-219 の 8 (a)): `io` でも `bg` でもない **専用のスレッド** `eve-stdin-job` (02 §9.1。パイプが閉じない間も、`io` の他の文書の読込・保存と `bg` の検索を止めない。ADR-0006 の「固定 2 本」の例外)。`ReadChunks` は、さらに別のスレッド `eve-stdin` (02 §9.1) が読んだチャンク (最大 1 MiB) を深さ 4 のチャネルで受け取る形で、**データが無い間、受け手 (ジョブ) はチャネルだけで待つ** (時間で区切って起きない。02 §14 の NFR-204 の「ポーリング禁止」)。取消し (`CancelToken`) のときは、Host が同じチャネルへ起こすための印を送り、ジョブはそれを受けて取消しを見る (02 §4.4 の `StdinSource::wake`)。終了のときはどちらのスレッドも待たない (02 §9.6) |
| 読取の止め方 | 読むのをやめる (取消し・上限・終了) ときは、`eve-stdin` を OS ごとの方法で起こして止める (Unix は停止の自己パイプ、Windows は `CancelSynchronousIo`。05 §9.1 / §9.2)。`eve-stdin` は自分で私有のハンドル / fd (下の行) を閉じて終わる (別のスレッドからは閉じない)。閉じた後、書き手は次の書込で失敗する。そこで書き手が止まるかは書き手による: Unix は SIGPIPE で既定では終わる。Windows は `ERROR_NO_DATA` で、書込の誤りを無視するプログラム (`ping -t` など) は動き続け、シェルが戻らない (evoeditor の終了後に Ctrl+C で止める。07 §18.5) |
| 私有のハンドル | Host は `StdinSource` を作るとき、標準入力を私有のハンドル / fd に移し、標準入力の位置には `NUL` / `/dev/null` を置く (Windows は継承しない複製を `DuplicateHandle` で作り、`NUL` を開いて `SetStdHandle(STD_INPUT_HANDLE, …)` で置いて元を閉じる。Unix は `fcntl(F_DUPFD_CLOEXEC)` で高い番号の fd へ移し、`/dev/null` を fd 0 に `dup2` する。05 §9.1 / §9.2)。子プロセス (v0.5 の `ProcessRunner`・v1.L の LSP など) がパイプの読み手を持ち続けて書き手が止まらないこと、閉じたハンドルの値の再利用で別のものを読むことを防ぐ |
| 文字コードの判定 | ファイルと同じ (§8.2)。ただし sniff は、先頭の 64 KiB が揃うか EOF になるまで読んでから行う (パイプは小さく区切って届くので、最初の 1 回の読取だけで判定しない)。`--encoding` を付けたら判定しない (§8.2 手順 2)。**判定のやり直し (§8.2.1) はしない** (読んだバイト列を持たないので、先頭から読み直せない)。パスが無いので、文字コードを指定して開き直す (§8.8)・再読込 (§9.5) も使えない。誤判定は `--encoding` を付けて起動し直す (OQ-219 の 7) |
| 開く前の確認 | バイナリ・UTF-32 の BOM・不正バイトの比率の確認 (§9.2 手順 3) は出さず、「読み取り専用で開く」を選んだのと同じに扱う (`read_only_requested` を立てて読み、理由 `Binary` / `InvalidBytes` / `StatefulEncodingWithInvalidBytes` を付ける。`InvalidBytes` は §8.3.1 の「編集を許可」で解除できる)。確認の後に先頭から読み直す (§9.2 の「確認からの再投入」) ことができないため |
| 書く手段の無い文書 | ファイルでは開き直し (§8.8) で回復するとしている文書 (`StatefulEncodingWithInvalidBytes` (§8.3。どの文字コードでも書けない)・私用文字と衝突した文書 (b) (§8.3 の「衝突」)) は、標準入力では開き直せないので、開いた時点で知らせる。**起動し直す案内は、流し直せる入力のときだけ出す** (v0.5 の M29。v0.5-plan §1.3 の (4) (c)): 標準入力がふつうのファイル (`StdinKind::File`。シェルがヒアドキュメント (`<<EOF`) に使う一時ファイルも含む (使うかはシェルと大きさで変わる)。Host が標準入力を移す前に判定する。Unix は fd 0 の `fstat` が `S_ISREG`、Windows は `GetFileType` が `FILE_TYPE_DISK`。05 §9.1 / §9.2) なら、起動し直せば同じ内容を読めるので `msg.stdin.unsaveable` (「この内容は保存できません / --encoding を付けて起動し直すか、表示している内容を選択してコピーしてください」。v0.4 からのキーで、起動し直す案内の意味は変えない)。それ以外 (`StdinKind::Stream`。パイプ・デバイス・判定できないとき) は `msg.stdin.unsaveable_copy` (「この内容は保存できません / 表示している内容は、選択してコピーできます」。v0.5 の M29 で足した) で、起動し直す案内を出さない (04 §15.2)。パイプの書き手が同じ出力を出し直せるか (`cat file \|` は出し直せるが、閉じないパイプ・一度きりの出力 (`sudo`・`curl` など) は分からない) は evoeditor には判断できないので、案内しない側に倒す (v0.4 は入力の種類に依らず `--encoding` を案内していた)。読み取り専用の文書でも選択とコピーはできる (§9.6) |
| 大きさの上限 | 読んだバイト数、または復号した本文のバイト数が `file.huge_file_warn_bytes` (既定 512 MiB、07 §5.2) を超えたら、そこで読むのをやめて (`StdinEnd::Limit`)、読んだところまで (上限を超えた分の手前の文字の境界まで。下の「読込の失敗」の「切る位置」) を開く (上の「読取の止め方」)。文書には読み取り専用の理由 `LoadIncomplete` (§9.6) を付け、`msg.stdin.truncated` を出す。上限の前の確認は出さない (読み終えるまで待つ確認を出すと、その間パイプの書き手が止まり、答えの後に続きを読む状態が要る) |
| 取消し (`Esc`) | ★ (OQ-219 の 10 (i)): `Esc` (`ui.cancel`) で読むのをやめられるのは、標準入力の文書を **表示しているときだけ** (ほかの文書を表示しているときの `Esc` は標準入力の読込に触れない。§9.8 の「読込中のバッファ」)。やめる前に確認 `ui.confirm.stdin_stop` (「読むのをやめると書き手が止まり、続きは読めません」。共通のキャンセルは並べない。選択肢: 読み続ける (既定。`Esc` も同じ)・ここまでで開く、04 §15.2) を出す。「ここまでで開く」で読むのをやめて (`StdinEnd::Stopped`)、読んだところまで (最後の文字の境界まで。切れた多バイト列は開かない。上の「大きさの上限」と同じ) を開き (止めるときは、`eve-stdin` が既にパイプから読んで受け渡し待ちにした分 (チャネルに積まれた分。最大 4 MiB) を `ReadChunks::next_ready` でブロックせずに取り込んでから終える。書き手の側では渡し済みで、捨てると取り戻せないため。上限 (`Limit`) と読取の失敗 (`Error`) では取り込まない。チャネルが一杯で `eve-stdin` が積もうとして待っている 1 チャンク (最大 1 MiB) は、取り込みの間に積まれなければ失う (既知の制約))、`LoadIncomplete` を付けて `info.stdin.stopped` を出す。まだ 1 バイトも読んでいなければ、理由の無い空の文書になる |
| 閉じる・終了 | ★ (OQ-219 の 10 (ii)): 1 バイト以上読んだ読込中の標準入力の文書を閉じる (`buffer.close`)・終了 (`app.quit`) するときは、閉じるときの確認と終了の連鎖 (§9.8) の対象にする (`Loading` の文書は dirty でなく本来は連鎖の対象でないが、この文書だけ例外)。確認 `ui.confirm.stdin_loading` (選択肢: ここまでで開く (既定)・破棄・キャンセル (共通の `ui.confirm.cancel`)、04 §15.2)。「ここまでで開く」は上の `Esc` の「ここまでで開く」と同じで、開いた文書は変更あり (下の「変更あり」) なので、閉じる・終了はそのまま FR-107 の確認 (連鎖の次の段) に進む。「破棄」は読むのをやめて内容を捨てる (終了の連鎖では、ほかの文書と同じく破棄の印を付けるだけで、終了が確定するまで読み続ける。連鎖をキャンセルすれば読込はそのまま。§9.8)。1 バイトも読んでいなければ確認を出さずに捨てる。確認の間に読み終えたら、「ここまでで開く」はそのまま次 (FR-107 の確認) に進む。「ここまでで開く」の後に読み終えたときの続き (FR-107 の確認・終了の連鎖の次) は、オーバーレイ・検索バーが開いている間は進めずに待ち、閉じた後に進める (§9.2.1 の「表示の切り替え」と同じ保留。開いているほかの確認・入力を置き換えない)。読込がパニック (`Failed`) で終わったときも続きを進める (文書は閉じたので、終了なら連鎖の次へ、閉じるなら済み)。終了の連鎖で「破棄」の印を付けた読込中の文書が、連鎖の途中で読み終わると、Revision が進んで印が外れ、FR-107 でもう一度尋ねる (安全側。§9.8 の「破棄の印」と同じ規則)。`--readonly` の文書は読み終えても変更なしで FR-107 の確認が出ないので、「ここまでで開く」の後は閉じる・終了を進めずに文書を残す (終了なら連鎖を止める。「ここまでで開く」が「破棄」と同じにならないため。M25 の (9) で決めた)。保存中の終了の「保存を中止して終了」(§9.7 (f) 3) では、読込中の標準入力の文書 (連鎖で「破棄」の印が付いたものを除く。印は Revision に依らず見る) の読込を、受け取った量によらず止め、読み終えた文書を退避してから終了する |
| 読込の失敗 | **切る位置**: 読み終えなかったとき (上限・取消し・途中の I/O エラー・確保の失敗) は、どれも最後の文字の境界 (持ち越しのバイトが無い位置。状態付き符号化 (ISO-2022-JP) の漢字の区間の状態は問わず、組の直後なら境界) までを開き、切れた多バイト列 (最大 3 バイト) は開かない (文書には `LoadIncomplete` が付くので、途中までであることは分かる)。EOF (読み終えた) だけは、ファイルの末尾と同じく持ち越しを不正バイトとして出す。境界に戻らない不正なバイト列 (壊れたエスケープ列の繰り返しなど) が 64 バイトを超えて続いたときは、境界を待たずに不正バイトとして開く (内容を黙って捨てない)。途中の I/O エラーは、読んだところまで (文字の境界まで) を開く (`StdinEnd::Error`。`LoadIncomplete` を付け、`msg.stdin.read_failed` を出す)。最初から読めない (無効なハンドルなど。受け取ったバイト数が 0 のまま `StdinEnd::Error` で終わった) ときは、理由の無い空の文書と `msg.stdin.unreadable` (`msg.stdin.read_failed` の「{size} を読み取り専用で開きました」は文書の状態と合わないので分ける。M25 の (9))。受け取りが 0 バイトのまま確保に失敗した (`StdinEnd::OutOfMemory`) ときも同じく `msg.stdin.unreadable` (理由は `out of memory`)。受け取りが 1 バイト以上なら、開いた量が 0 でも `msg.stdin.read_failed` / `msg.stdin.out_of_memory`。**確保の失敗** (`try_reserve`、§9.2 の「読込中のメモリ」): 大きさが分からないので、読み始める前の確保の見込みの確かめ (§9.2 の `probe_reserve`) の代わりに、判定の前に貯める先頭 (64 KiB) を先に確保し、復号の作業領域を `try_reserve` し、本文が 1 MiB を超えて倍になるごとにその大きさを確保できるかを確かめる (rope の構築は確保の失敗を返せないので、abort する経路を減らす)。どれかが失敗したら、構築済みの本文があれば、上限と同じく読んだところまでを `LoadIncomplete` で開く (`StdinEnd::OutOfMemory`。`msg.stdin.out_of_memory`、04 §15.2)。パニックは、ファイルと同じく `Failed { Panic }` で終わり、読んだ内容は失う (`msg.stdin.failed` を出し、標準入力の文書は閉じ、表示していたら §9.8 の順で別の文書を表示する) |
| 結果 | `JobResult::StdinDone { text, format, detected, preserved, bytes, end: StdinEnd }` (02 §9.3。終端。`bytes` は開いた量)。`meta`・`stamp`・内容ハッシュ・中断された保存の痕跡は持たない。**保持方式の記録は、ファイルと同じ関数で集める** (§8.7.1。上限 (件数 100 万・元のバイト列の合計 16 MiB) を超えたら `Overflow` の「保持なし」)。これで、文字コードの判定が外れても、入力が有効なバイト列なら同じ文字コードで書けば元のバイトに一致する (OQ-219 の 7)。改行の統計・長大行の数・不正バイトの数・`escape_encoding` はファイルと同じく読込で決める。`LoadIncomplete` は `end` が `Limit` / `Stopped` / `Error` / `OutOfMemory` (で 1 バイト以上読んだ) のときに `detected` に入れる。「読んだ」は受け取った量で判定する (文字の境界で切った後に開いた量が 0 でも、途中までであることを示す) |
| 変更あり | 読み終えた (`end` のどれでも、1 バイト以上読んだ) 文書は **変更あり** (dirty) にする (保存点の無い履歴にする。Undo で読込の直後に戻っても dirty のまま)。終了・閉じるでは FR-107 の確認が出る (内容はほかに残っていないため)。ただし `--readonly` で開いた標準入力の文書 (理由に `UserRequested` を含む) は変更なしにし (見るだけと明示したので)、読み取り専用を解除した (`file.toggle_read_only`、§9.6) 時点で変更ありにする (まだ保存していない (パスの無い) 文書だけ。名前を付けて保存した後は保存したファイルと一致するので変えない)。OQ-219 の 4。**1 バイトも読まなかった文書** (すぐに EOF・何も読まずに取り消した・最初から読めない) は変更なしの理由の無い空の文書で、何も入力していない無題の文書 (§9.8 の `file.open` の行の「例外」) と同じく、`file.open` で開いたファイルに置き換わる (失うものが無い。M25 の (9) で決めた) |
| `LoadIncomplete` の外し方 | `LoadIncomplete` (§9.6) は、**名前を付けて保存が成功したら外す** (その文書は保存したファイルの全体と一致し、読まなかった後半を失う誤解が無いため)。完了のメッセージは `info.save.partial_saved` (「途中までの内容を {path} に保存しました ({size})」、04 §15.2)。名前を付けて保存の入力の案内と、FR-107 の確認 (閉じる・終了の連鎖) の文書名の後ろに「途中まで ({size})」を出す (04 §15.2 の `ui.prompt.save_as.partial`・`ui.confirm.unsaved.partial_suffix`)。`{size}` は開いた量 (止めたときは受け渡し待ちの分を取り込んだ後の量。パイプに残っていた分と、`eve-stdin` が読んでいる途中の分は含まない)。再読込・開き直しでは必ず外す (§8.8 の「読み取り専用の理由の差し替え」。名前を付けて保存の後にだけ起きる) |
| 最初の編集 | 「標準入力から読み込んだだけ」の印 (02 §5.1 の `Document`) は、**読込以外の本文の変更** (本文を変える Transaction。`Origin` を問わず、利用者の編集・貼り付け・コマンドのほか、駆動 API の `Ai` / `External` を含む。読込そのものは Transaction を作らない) で外す。Undo で取り消しても戻さない |
| スワップと緊急退避 | ★ (OQ-219 の 5): **緊急退避 (§10.1) は読み込んだ時点から対象にする** (変更ありなので。`--readonly` で開いた変更なしの文書も、まだ名前を付けて保存していなければ対象。下の末尾)。標準入力の文書の manifest には常に `sensitive = true` (02 §11.2.1 の欄。名前を付けて保存した後も立てたまま。スワップの manifest も同じ) を付け、復旧の案内で場所を示して削除を促す (§10.1 の「秘密ファイル」と同じ扱い)。**スワップ (§10.2) は最初の編集から** (64 MiB 以下。ほかの無題の文書と同じ)。理由: 標準入力の内容は読み直せない (P1)。異常終了だけでなく SSH の切断 (SIGHUP) でも失う (`journalctl \| evoeditor -`)。§10.1 の「秘密のファイルも退避する (P1 > 機密性)」と同じ扱い。読込中 (`Loading`) の内容は緊急退避の対象外 (下の「既知の制約」)。**`--readonly` で開いた文書** は変更なし (dirty でない) だが、内容はほかに残っておらず読み直せないので、**dirty でなくても緊急退避の対象にする** (v0.5 の M29。v0.5-plan §1.3 の (4) (b)。v0.4 はほかの文書と同じく dirty でないので対象外だった)。core は「まだ名前を付けて保存していない (パスの無い) 標準入力の文書で、1 バイト以上読んだもの」(`Document::stdin_unsaved`) を、dirty でなくても退避用の公開物に出す (§10.1 の core の契約)。名前を付けて保存した後 (保存したファイルに残る) と、1 バイトも読まなかった文書 (失うものが無い) は、dirty でなければ対象外。スワップは変えない (読み取り専用の文書は書かない、§10.2)。通常の終了 (dirty な文書が無ければ確認なしで終わる) では、ほかの文書と同じく退避しない (退避は異常終了と「保存を中止して終了」だけの経路)。「保存を中止して終了」(§9.7 (f)) の退避の対象は緊急退避と同じなので、`--readonly` の文書も退避する (選択肢の文言 `ui.confirm.quit_saving.abort` の「保存していない内容は退避します」のとおり。04 §15.2)。退避の書く順による制約は §10.1 の「対象」 |
| 端末 | キー入力は端末 (Windows は `CONIN$`、Unix は `/dev/tty`) から読む (04 §5.2a、05 §9.1 / §9.2)。端末の初期化の後は、標準入力を読んでいる間も操作できる (ほかの文書の編集・`Esc`・終了) |
| 既知の制約 | 読込中 (`Loading`) の内容は緊急退避の対象外 (§10.1。内容はジョブが持ち、core の文書には無い): 閉じないパイプ (`journalctl -f` など) を読んでいる間に端末や SSH が切れる (SIGHUP)・異常終了すると、読込中の内容は退避されずに失われる。残したい内容は `Esc` の「ここまでで開く」で開いてから扱う (開いた文書は変更ありで退避の対象)。書き手が端末から読むコマンド (`cat` 単独・対話のスクリプト) では、書き手と evoeditor がキー入力を取り合う。書き手の stderr は画面に出る (`ui.redraw` で描き直す)。Windows PowerShell 5.1 のパイプは非 ASCII を `?` に置き換えるので、`--encoding` でも直らない。07 §18.5 と help の注意 (04 §8.6) に書く |
| 計測 | BM-120-100m (09 §5.2。参考で、判定には使わない): 100 MB をパイプで渡して `evoeditor -` を起動し、起動 → 読み終えた文書の最初の Frame の `write`。`bench-trace` の区間 `startup.first_loaded` (`main` 入口 → 本文が見え、読込の進捗が出ていない最初の Frame の `write`。09 §3.6 の「計測の記録」) で測る。読込中の文書は本文が無く進捗が出ているので、読み終える前の Frame は数えない |

**採らなかった案**: (1) 端末を初期化する前に標準入力を EOF まで読む (Vim に近い): 仕組みは小さいが、読み終えるまで画面が出ず、進捗も取消し (読んだところまでを開く) も無い。閉じないパイプでは起動しない (★ の「端末の初期化の時期」は最初のデータまでで、EOF までは待たない)。(2) `io` のワーカーで読む: パイプが閉じない間、ほかの文書の保存が待たされる (OQ-179 の監視で切り離されても、`io` の監視の警告が出続ける)。(3) 読んだバイト列を持ち (16 MiB まで等)、判定のやり直しと「文字コードを指定して開き直す」を使えるようにする: メモリが 2 倍になり、上限を超えたときの規則が増える (OQ-219 の 7 の (b))。(4) データが無いときに 100 ms ごとに空のチャンクを返して、ジョブが `CancelToken` を見る: 02 §14 の「ポーリング禁止」(NFR-204) に当たる。取消しでチャネルを起こす形にした。

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
  2. 保存前フック (v0.5、FR-216: 末尾空白除去等の設定) を Transaction として適用。**dirty な文書にだけ適用** し、dirty でない文書
     (手順 0 の照合が一致せずに書くことになった文書・disk_stamp が None の新規文書) には適用しない
     (開いただけのファイルに、頼んでいない差分を出さない。手順 0 と揃える。PO の決定 2026-11-20、v0.5-plan §1.3 の (7))。
     かける時点 (連鎖ごとに 1 回、すべての確認の後で手順 3 の直前に同期で)・確認の後の投入し直しで二重にかけないこと・1 MiB を超える文書では
     かけないこと・書けずに終わったときも本文に残して知らせることは §9.3.1 (v0.5 の M34)
  3. history.seal(); snapshot = text.snapshot(); node = 履歴上の現在位置 (§7.1 の NodeId)
  4. ジョブ投入: JobRequest { id, doc, base: revision, kind: JobKind::Save { path, snapshot, format, preserved, node, opts, method } } (02 §9.3。id は JobSpawner が採番する。
     Port はジョブの種別に持たせず、run_job が JobPorts から取る)
       format: 文書の format (file.set_encoding・E3 の「UTF-8 で保存」では新しい format。保存が成功するまで文書の format は変えない、§8.8)
       preserved: (v0.4) 保持方式の記録の、この時点の版 (渡した後の編集の影響を受けない。書く文字コードが記録を集めたときの文字コードと違えば使わない。§8.6 E2、§8.7.1)
       escape / escape_merge_ok: (v0.4) エスケープ文字の扱い (`TextFormat::save_escape_policy`、§8.3.1 の表) と、つながり (`EscapeMerge`) の承認
                 (`Option<EscapeMergeApproval>`。`SaveApprovals` にこの文書のこの Revision の承認があればそれ。ジョブは結果 (件数・先頭 100 件・全件のダイジェスト) が同じときだけ止めない。§8.3.1、§9.7)
       opts: SaveOptions { expected, in_place_backup: Required, fsync, private: false, preserve, hardlink: HardlinkPolicy (既定 Ask、07 §5.2),
                           backup: None (FR-119 (v0.6 以降) の予約。v0.1 / v0.2 は常に None) }
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
     ※ v0.1〜v0.3 で元の文字コードのまま書き、roundtrip.may_be_lossy() (lossy_lines ≥ 1 または incomplete) が真なら、投入前に §8.6 E2' の確認を出す (承認後に投入)。
        v0.4 は §8.6 E2' の v0.4 の条件 (§8.7.1 の「保持なし」「保持できなかった箇所がある」文書だけ。未編集でも出す) で出す
  [job]
  4'. (v0.4 の M23) method = Atomic でも、escape = Restore (エスケープ文字を元のバイトに戻して書く。書く文字コードが escape_encoding と同じ、§8.3.1) なら、write_atomic を呼ぶ前に乾式走査 (手順 5'a と同じ) をし、
       (1) 確認 (i) と表の不変条件 (`VerifyFailed`) → (2) 表現不能 (§8.6 E3) → (3) つながり (`JobResult::EscapeMerge`。escape_merge_ok の承認と結果 (件数・先頭 100 件・全件のダイジェスト) が同じなら止めない)
       → (4) 区切った確認 (ii) (`VerifyFailed`) の順に見て、当たれば Port を呼ばずに返す (一時ファイルも作らない。§8.3.1 の「見る順序」)。復号するのは書いた BOM を除いた出力。
       v0.3 までの実装は、UTF-8 / UTF-16 の原子的保存では escape = Reject のときだけ乾式走査をし、Restore では書きながら符号化していた。
       記録を渡す保存と、UTF-8 / UTF-16 以外の文字コードの保存は、今の実装どおり書く前に乾式走査をする
  5. method = Atomic のとき write_atomic(path, エンコード済みチャンクのストリーム, opts):
       a. Port が metadata を取り opts.expected (`Expect`) と照合 (05 §4.1 手順 0b)。不一致 → FsError { kind: Conflict { current: Option<Box<FileStamp>> } } を返して終了 (何も書かない。`current` は今の stamp で、None = ファイルが無い)
       b. チャンクを順に encode + 照合 (§8.6 E1、v0.4 は E2 の差し替えと確認 (i)・(ii)、つながりの検査も)。表現不能を 1 件でも検出したら **その時点で書込を止める** (ストリームが `ChunkError::Unmappable` を返し、
          Port は一時ファイルを破棄して `SourceFailed` を返す)。ジョブは書込なしの走査 (encode + 照合) だけを終端まで続けて位置を集め、JobError::Unmappable { positions, total } を返す
          (無駄な I/O を避け、空き容量不足 (DiskFull) で表現不能が隠れないようにする)。状態付き符号化は最後に last = true で呼び、終端のエスケープ (ESC ( B) まで書く (E6)
       c. 書き終えたら Port が fsync → 属性複製 → **置換の直前に metadata を取り直して opts.expected と再照合** (書いている数秒の間に他のプログラムが保存した場合を検出。
          不一致なら一時ファイルを消して Conflict。残る競合の窓は置換の 1 回分だけ。05 §4.1) → 原子的置換 → (Unix) ディレクトリ fsync (ADR-0016、05 §4)
       d. Port が FsError { kind: AtomicUnsupported, detail: Some(InPlacePattern) } を返した (一時ディレクトリ規則、下の「一時ディレクトリ直下の一時ファイルの保存」) →
          **確認を出さずに同じジョブで** 手順 5' (乾式走査 → write_in_place) へ進む (02 §4.5、OQ-34)。他の AtomicUnsupported はそのまま main へ返す (手順 7)
  5'. method = InPlace (承認済み、または save_strategy = "in_place")、または 5d のとき:
       a. **乾式走査 (必須)**: Snapshot 全体を、保存と同じエンコーダで encode + 照合だけ行い、書き出さない (`SaveEncoder::dry_scan`、下の「乾式走査と出力長」)。
          表現不能が 1 件でもあれば **Port を呼ばずに** JobResult::Unmappable { total, recorded, encoding, escapes_restored, escapes_unmappable } を返す (元ファイルに一切触れない。§8.6 E3 へ)。
          (v0.4 の M23) escape = Restore では続けてつながり (承認と一致しなければ JobResult::EscapeMerge) と区切った確認 (ii) を見る (手順 4'、§8.3.1)
       b. 乾式走査で確定した出力バイト長 len を渡して write_in_place(path, 同じエンコーダのストリーム (2 回目), len, opts, cancel) を呼ぶ (cancel はジョブの CancelToken) (02 §4.5、05 §4.4 手順 0〜9)。
          05 §4.4 の手順 6 (元ファイルへの書き始め) 以降は CancelToken を見ず、本書込のエンコーダは `Aborted` を返さない (5d から来た場合も同じ。手順 7 の `Cancelled` の箇条)
  6. JobResult::Saved { node, report: SaveReport { method, new_stamp, warnings }, canonical, meta } / JobResult::Failed { err: JobError }
  [main]  (採否は 02 §9.3 の採否表が正本 (§17.2)。Revision が進んでいても採用する)
  7. 成功: history.saved = node; disk_stamp = report.new_stamp (置換の後の T が書いた一時ファイルのものと確かめられなければ (FileId か len が違う。置換をまたいで識別が保たれない FS
     (FAT 系・ネットワーク等、05 §5.4) では len か mtime が違う。直接上書きは開いたハンドルの FileId と len を必ず比べ、保たれない FS では mtime も比べる)、Port は書いた内容の stamp を入れて警告 (ChangedAfterSave) を積む。
     ディスクの T とは一致しないので、次の保存は照合で、中身が同じでなければ Conflict になり、外部変更として扱われる。05 §4.1 手順 7); path 確定 (新規ファイル・名前を付けて保存の場合);
     canonical = Saved.canonical; file_id = Saved.meta.file_id; 名前を付けて保存では読み取り専用の理由 (FileAttribute / NoWritePermission、§9.6) を Saved.meta から付け直す; dirty 再計算; DocumentSaved 通知; スワップを削除 (§10.2)
     report.method == InPlace なら `info.save.in_place` (04 §15.2) のメッセージで直接上書きで保存したことを 1 回示す (ステータスの項目にはしない。05 §4.4)。report.warnings (権限複製失敗等。v0.3 までの仕様にあった EscapeMerge の警告は、v0.4 の M23 で書く前の確認 (`JobResult::EscapeMerge`、§8.3.1) に替えたので無い) をメッセージで提示 (保存の完了のメッセージの後に、最も重い 1 つを警告として出す。重さの順は TempNotRemoved → FsyncFailed → ChangedAfterSave → BackupNotRemoved → PermissionNarrowed → AttrsNotPreserved → InflightFlagNotWritten。文言は 04 §15.2)
     失敗: Document は dirty のまま。JobError と FsError.kind (と detail) で分類し、**§9.7 (e) の「保存失敗の後の選択肢」の表** (MessageKey・選択肢・次の状態・後続処理) に従って提示する。
       分類の要点 (選択肢は §9.7 (e) の表が正本):
       - JobResult::Unmappable → §8.6 E3 の選択肢
       - JobResult::EscapeMerge (v0.4 の M23。エスケープ文字を戻したバイトが隣とつながる。何も書いていない) → `ui.confirm.save.escape_merge` (§9.7 (e)、§8.3.1)。
         承認した保存の成功は、完了のメッセージを `info.save.escape_merged` にする
       - JobError::VerifyFailed (v0.4。保持方式の記録を使う保存の確かめ (§8.6 E2 の確認 (i)・(ii)) に合わない。何も書いていない) → 文書の記録を全部捨てて「保持なし」(`NoPreserveReason::VerifyFailed`) にし、`msg.save.verify_failed` + `ui.confirm.save_failed` (§9.7 (e))
       - Conflict (外部変更) → §9.5
       - AtomicUnsupported + detail Hardlink { nlink } (HardlinkPolicy::Ask) → 選んだ HardlinkPolicy を SaveOptions.hardlink に入れて再投入する (WriteInPlace は method = InPlace)
       - AtomicUnsupported (上記以外。条件は 05 §4.4 の表が正本: 他ユーザー所有、root で所有者を保てない (Unix)、ディレクトリに書込権限なし、FS が非対応、
         既存の T の rename の EBUSY (Unix。1 ファイルの bind mount ほか)、Windows で ReplaceFileW が元ファイルを変えずに失敗した) →
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
- **乾式走査と出力長** (02 §4.5 の `write_in_place(path, data, len, opts, cancel)` の `len`): 保存のエンコーダ (`SaveEncoder`、eve-encoding / eve-core。Snapshot と format から作り、`ReadChunks` を実装する) は次の 2 つを持つ。(a) `SaveEncoder::dry_scan(&snapshot, &format, cancel) -> Result<DryScan { out_len: u64 }, Unmappable { positions, total }>`: 書き出さずに全文を encode + 照合し (BOM と ISO-2022-JP の終端エスケープを含む)、出力の総バイト数を数える。(b) `SaveEncoder::new(snapshot, format)`: 同じ規則で 2 回目のエンコードをチャンクで供給する `ReadChunks`。ジョブは (a) の `out_len` を `len` として Port に渡し、(b) を `data` に渡す。v0.4 の M23 からは、(a) は `Unmappable` のほかに、つながり (`EscapeMerge`、§8.3.1) と区切った確認 (ii) の結果も返し、`escape = Restore` の保存では原子的保存でも (a) を先に行う (手順 4')。同じ Snapshot と format からの 2 回のエンコードは同じバイト列になる (エンコーダは決定的。チャンクの区切り方・キャンセル確認の回数によらない)。これを 09 L1 のプロパティテストとして置く: 任意の Snapshot (チャンクの分割を任意に変えたものを含む) と format について、`dry_scan` の `out_len` と表現不能の判定が、`SaveEncoder::new` の出力を連結したバイト列の長さ・判定と一致し、2 回の `SaveEncoder::new` の出力が同じバイト列になる。本書込 (2 回目) が `out_len` と違う長さ・表現不能を返したら (不変条件違反) ストリームが `ChunkError` を返し、Port は書込を止めて `InPlaceFailed { backup }` を返す (02 §4.5、05 §4.4 手順 8。書き始めた後なので元ファイルは壊れている可能性があり、バックアップのパスを示す)。`ReadChunks` trait (02) に長さのメソッドは足さない。乾式走査のコストは encode 1 回分 (照合を含めて 2 回分) で、メモリはチャンク 1 つ分。キャンセルはチャンク毎に確認する。**v0.4** (§8.7.1): `dry_scan` と `SaveEncoder::new` は、Snapshot と format に加えて保持方式の記録の版 (手順 4 の `preserved`) を受け取り、同じ計画 (§8.6 E2) から同じ手順で出力を作る (出力の長さと内容が一致する。記録を入力に含めた性質テストを §22.1 に置く)。E2 の確認 (ii) は乾式走査でも行い、一致しなければ Port を呼ばずに中止する。
- **シンボリックリンク** (FR-106): Port がジョブ内で `canonicalize` した **リンク先** に保存する (リンクをファイルで置き換えない)。一時ファイルはリンク先のディレクトリに作る。リンク先が存在しない (ダングリング) 場合の確認と選択肢は [05 §4.1](05-platform.md#41-共通手順-adr-0016-の具体化) の規則に従う (「リンク先に作成 (リンクを維持)」/「キャンセル」。リンク自体を通常ファイルで置き換えない)。core は `FileMeta.link` の dangling を見て確認を出し、承認後に保存ジョブを投入する。**v0.1 はダングリングのリンクを確認なしのエラー** とする: Port は書かずに `FsErrorKind::NotFound` (詳細 `DanglingLink`。05 §4.1) を返し、core はリンク先の絶対パスを示して「名前を付けて保存」を案内する。確認してリンク先に新規作成する UI は v0.2 で入れる。循環 (ELOOP) は `InvalidPath` として報告する。
- **ハードリンク (`nlink > 1`)、他ユーザー所有、ディレクトリに書込権限なし**: 原子的置換が意味を変える/不可能なケース。Port は何も書かずに `AtomicUnsupported` を返し、core は利用者に選ばせる (手順 7)。ハードリンクは `SaveOptions.hardlink: HardlinkPolicy` の 3 値 (`Ask` (既定。確認を出す) / `BreakLink` (原子的置換で書き、このパスだけが新しい内容になる) / `WriteInPlace` (直接上書きで全リンクを新しい内容にする。乾式走査が前提)) に従う (OQ-73、確定 (ユーザー回答)。設定キー `file.hardlink_policy` は 07 §5.2)。他ユーザー所有・ディレクトリに書込権限なし等の場合 (どの条件で `AtomicUnsupported` になるかは 05 §4.4 の表が正本) は、確認を出すかどうかを設定 `file.atomic_save_fallback` (07 §5.2。v0.1 は既定 `ask` の定数、公開は v0.2) で決める: `ask` = 手順 7 の確認 (`ui.confirm.save.atomic_unsupported`)、`fail` = 確認を出さずに保存の失敗として報告する。どちらの値でも、承認なしに直接上書きへ進むことはない (ADR-0016)。例外は一時ディレクトリ規則 (`InPlacePattern`、下記) で、Port が `AtomicUnsupported { InPlacePattern }` を返し、core が確認なしで乾式走査から直接上書きへ進む (手順 5d)。この設定の対象外である。
- **大きいファイルの保存時間** (NFR-107): エンコードはストリーミングで、Snapshot のチャンクをそのまま書くため追加メモリはバッファのみ。fsync コストが支配的。
- **読み取り専用属性を持つファイルを保存** (FR-116、v0.2): ユーザーが明示的に「読み取り専用を解除して保存」を選んだ場合のみ、Port に属性変更 (置換の直前に外し、置換の後に付け直す、05 §4.2) を含めて依頼する。v0.1 は読み取り専用属性のファイルを書かない (§9.6)。開いた後に属性が付いた・権限が変わった場合は手順 5 の `PermissionDenied` になり、文書に `NoWritePermission` を付ける。
- **新規ファイル** (FR-101): `SaveOptions.expected = Expect::Absent` (「存在しないこと」を期待する)。保存時点で同名ファイルが存在していたら (起動後に他者が作成) `Conflict` として扱う。既存のファイルの変更とは別の文言 (`ui.confirm.save.target_exists`、04 §15.2) で示す (§9.5)。
- **一時ディレクトリ直下の一時ファイルの保存**: 呼び出し元が同一 inode を期待することがあるため、**一時ディレクトリ (`$TMPDIR` 等) の直下 (サブディレクトリを含まない) でファイル名が 05 §4.4 のパターン (`*.tmp` 等、`file.in_place_patterns`) に一致し、そのファイルが既に存在する** 場合は (存在しなければ同一 inode の期待も無いので、新規ファイルとして原子的保存の経路 (`Expect::Absent`) で書く)、Port は書かずに `AtomicUnsupported` (詳細 `InPlacePattern`) を返し、保存ジョブは **確認を出さずに同じジョブで** 乾式走査 → `write_in_place` に進む (手順 5d / 5'。OQ-34、確定 (ユーザー回答))。直接上書きの手順 (乾式走査・事前バックアップを含む) は承認後の in-place と同じで、`SaveReport.method = InPlace` で結果を知らせる。
- **名前を付けて保存**: 保存先の決め方と確認は §9.7 (a)。新パスに保存成功後、Document の path / file_id / canonical / disk_stamp / 言語判定を更新し (file_id と canonical は `JobResult::Saved` の `meta` / `canonical` から取る。main は I/O しない)、読み取り専用の理由を判定し直し (§9.6)、旧パスのスワップを削除。
- **予約**: `SaveOptions.backup` (FR-119 の「保存前の版を 1 世代残す」) は v0.6 以降 (v0.4 から送り、v0.5 の計画で再び送った、08 §7) の予約で、v0.1 / v0.2 は常に `None` を渡す (§10.4)。直接上書きの事前バックアップ (`in_place_backup`) とは別物。
- **全保存** (FR-104、v0.2。複数文書と同時): dirty な Document を順にジョブ投入。結果は個別に報告。

#### 9.3.1 保存時の変換 (FR-216、v0.5 の M34)

手順 2 の保存前フックの規則。受入基準は 01 §5.5 の FR-216。M34 の仕様の PR (2026-11-24。仕様との整合・安全性・利用者の 3 つの視点のレビューで、ジョブで変換する案をやめて同期だけにし、安全性・データの損失の再レビューで、書けなかったときに自動で戻す案をやめて「残して知らせる」にした) で決めた (試験・実装の段・決めた点は §15.9。実装の段の PR は、仕様との整合と、データの損失・安全性の 2 つの視点のレビューに掛ける、08 §2.6)。**変換は保存を止めない**: 変換できないとき・かけないときは変換せずに書く。**変換は書出しのジョブを投入する直前にメインスレッドで同期に当て、ジョブを持たない** (確認と変換の間に編集が割り込む窓を作らない)。**変換は普通の編集** (1 つの Undo のノード) で、書けたかどうかに依らず本文に残る。書くバイト列の作り方 (§8.6・§8.7・05 §4.1 の原子的保存の手順) は変えない。

| ID | 規則 |
|---|---|
| SVT-1 | **設定と値**: `editor.trim_trailing_whitespace_on_save` (末尾空白の除去) と `editor.insert_final_newline_on_save` (末尾の改行の付与) (07 §5.1。既定はどちらも `false`)。どちらも **文書ごとのキー** (07 §3.6) で、言語定義の同じ名前の項目 (07 §9.6 の LANG-3。組込みの `markdown` と `diff` は末尾空白を除かない、07 §9.6.1) と `.editorconfig` の `trim_trailing_whitespace` / `insert_final_newline` (07 §10.1) で替わる。かける時点 (SVT-4 (a)) の文書の値で決める。どちらも偽なら何もしない (v0.4 までの保存と同じ)。**値が決まりきっていないときは変換しない** (頼んでいない差分を出さない側に倒す。知らせない。01 §4.6 の既知の制約): (a) この保存で文書のパスが変わる (別のファイルへの名前を付けて保存・無題の文書の最初の保存・**標準入力の文書の最初の保存** (§9.2.2))。新しいパスの `.editorconfig` とワークスペース層の値は保存の後に読む (07 §10.3 の EC-1・§12.5) ので、決まっていない (同じファイルへの名前を付けて保存 (§9.7 (a) の 1) はパスが変わらないのでかける) (b) **その文書の初回の値がまだ無い**: 次のどちらか。(b1) その文書の `.editorconfig` の読込 (EC-1) を積んでいて (実装は `editorconfig_latest` に文書の番号がある)、**かつ** 文書が前に当てた `.editorconfig` の値を持っていない (`Document.editorconfig` が `None`。開いた直後・名前を付けて保存でパスが変わった直後) (b2) 起動の後のワークスペースのルートの検出 (03 §12.7.1) の結果がまだ届いていない、または届いたルートのワークスペース層の最初の読込 (07 §12.5 の WSC-3) を待っている (文書がルートの下か、まだ決まっていない)。**設定の読み直し (`config.reload`・自動の再読込) の間は、前に当てた値で変換する** (EC-1 の「`config.reload` では、届くまで前の値を使う」と同じ。前の値がある間は (b) に当たらない) |
| SVT-2 | **除く空白**: 各行の末尾 (区切りの直前。最後の行は文書の末尾) から前へ続く **ASCII の空白 (U+0020) とタブ (U+0009) だけ** を消す。全角の空白 (U+3000)・ノーブレークスペース (U+00A0)・ほかの Unicode の空白・制御文字・不正バイトのエスケープ文字 (§8.3) は消さない (全角の空白は日本語の文書で意図して置くことがあり、`view.show_whitespace` の既定でも見える)。CR は区切りの一部 (§2.4) で、空白に数えない。空白だけの行は空の行になる (行は消さない)。**CR と LF をつながない**: 消した結果、直前の単独の CR (前の行の区切り) と直後の単独の LF (この行の区切り) が隣り合う行 (改行の混在した文書の `a⏎(CR)␠␠⏎(LF)b` の真ん中の行) は、その行の空白を消さない (§2.4 では CR と LF が隣り合うと 1 つの CRLF になり、行の数が変わるため。§6.2 の `eol_guard` と同じ考え)。テキスト変換の `edit.trim_trailing_whitespace` (§15.8 の TXT-4) と同じ関数 |
| SVT-3 | **末尾の改行**: 文書が空でなく、最後の文字が区切り (LF・CR・CRLF) でなければ、末尾に文書の既定の改行 (`default_eol`、§8.4) を 1 つ足す (続けて打つ `Enter` が入れるのと同じ改行で、混在を作らない)。区切りが 1 つも無い文書でも `default_eol` (読込のときに `file.default_line_ending` の利用者の範囲の値で決まったもの、§8.4) を足す。`.editorconfig` の `end_of_line` は既存のファイルには効かない (07 §3.6 の LAYER-4 のとおり。01 §4.6 の既知の制約)。空の文書には足さない。**最後の文字が不正バイトのエスケープの文字 (§8.3) なら足さない** (UTF-16 の奇数の長さの末尾などで、足した改行がつながりの確認 (`EscapeMerge`、§8.3.1) を自分で起こさないように)。**取り除かない**: 設定が偽でも、`.editorconfig` の `insert_final_newline = false` でも、末尾の改行はそのまま (FR-216 は付与だけ。07 §10.1)。2 つの設定が両方とも真なら、除去の後に足すかを決める (`a⏎··` は除去で `a⏎` になり、区切りで終わるので足さない) |
| SVT-4 | **かける時点** (入口): (a) 保存の連鎖 (§9.7) で、**すべての確認が終わった後、書出しの保存ジョブを投入する直前** (手順 3 の Snapshot を取る直前) に、メインスレッドで同期に当てる。確認 (`AskName`・`CheckTarget`・`ConfirmOverwrite`・`PreConfirm` の §8.6 E2' と区分 D の正規化・`ui.confirm.save.read_only` (読み取り専用を解除して保存、手順 1)) はどれも変換の前に済む。連鎖の入口は `file.save`・`file.save_as`・`file.save_all` の各文書・`AskUnsaved` の「保存」(終了・置き換え・開き直し・**閉じる** (`Continuation::Close`、`buffer.close` の前) の前。`app.save_and_quit` を含む)・`file.set_encoding` のすべて。(b) その時点で文書が **dirty でなければかけない** (v0.5-plan §1.3 の (7)。手順 0 の照合が一致せずに書く文書 (`CheckingUnchanged` の不一致)・未編集の新しい文書・未編集の文書の `file.set_encoding` と名前を付けて保存を含む)。(c) **連鎖ごとに 1 回**: 連鎖 (`SaveChain`) は「変換を判定した」印を持つ。確認の後の投入し直し (`AfterFailure` の各選択 (再試行・`Conflict` の上書き・ハードリンク・直接上書き・バックアップなし・E3 の「UTF-8 で保存」)・`ConfirmReplacement` の置換の後・`EscapeMerge` の承認) も同じ「投入の直前」を通るが、印があるので二度は当てない (変換をかけなかった判定 (SVT-1 (a)(b)・SVT-6) も 1 回と数える)。**変換の後に「別名で保存」で `AskName` へ移ったとき** は、前のパスの値で当てた変換を本文に残したまま (バッファどおり) 新しいパスへ書く (印があるので、新しいパスの値で判定し直さない)。手順 4 の深さ 1 の待ち行列で再実行する保存は、新しい要求として、始めるときに (a)〜(c) で決める (前の保存の間の編集に効く)。**完了の後に、保存の間の編集で dirty なので出し直す `AskUnsaved` の「保存」(§9.7 (f) の 2 など) も新しい連鎖** で、印を新しく判定する。(d) 保存でない書出しにはかけない: 緊急退避 (§10.1)・スワップ (§10.2)・「退避先に保存」(`WriteRecovery`、§9.7 (e))。(e) 読み取り専用の文書 (§9.6。本文を変えられない) にはかけない (名前を付けて保存で別のファイルへ書くときも、そのまま書く。読み取り専用を解除して保存する流れでは、解除の後なのでかける)。(f) 全保存で保存しない文書 (dirty でない・名前の入力を取り消した) にはかけない (保存ジョブを投入する文書だけ) |
| SVT-5 | **Transaction・承認・知らせ**: ChangeSet は **消す空白の範囲と足す改行だけ** (行の全体を置き換えない。最小の変更にし、保持方式の記録 (SVT-7) とほかの View の選択を崩さない) にし、1 つの Transaction (`Origin::Command`。id は保存を始めたコマンド。`UndoGroupHint::Break` で直前の入力と結合しない) として適用する。本文が変わらなければ Transaction を作らない。適用した後の Snapshot を保存する (手順 3。§8.5 のとおり、保存の後のバッファとディスクの内容が一致し、保存点はこのノードなので dirty でなくなる)。1 回の Undo で変換が戻る (戻すと dirty になる)。**選択**: 空白の除去は写像する (`MapPolicy::Own`。消した空白の中のカーソルは行末へ寄る)。末尾の改行を足す編集は、**すべてのカーソルを足した改行の前に留める** (`Bias::Before`。文書の末尾のカーソルで保存しても、続けて打つ文字は同じ行に入る)。ほかの View も同じ (`MapPolicy::Other`、§4.6)。**カーソルのある行も除く** (v0.4 までの §15.3 の「カーソル行は除外する設定を既定にする」は採らない。反対の論点は §15.9.3 の PO に確かめる点)。**承認の引き継ぎ**: この連鎖の確認で得た承認のうち、(文書, Revision) で照らすもの (区分 B / D の確認の `lossy_ok`・読み取り専用を解除して保存の `readonly_ok`) は、変換で進んだ Revision に引き継ぐ (変換の Transaction を当てるときに Revision を書き換える。引き継がないと、`Busy` の再試行などの投入し直しで同じ確認が再び出る)。つながりの承認 (`escape_merge_ok`) は変換の後の結果で得るので、引き継ぐものが無い。**知らせ**: 変換で本文が変わったときは、保存の完了のメッセージに付記する (`info.save.suffix.transformed`。`{detail}` は「行末の空白を N 行で除きました」・「末尾に改行を足しました」の組。付ける先と、ほかの完了のメッセージとの順は 04 §8.16 の「完了のメッセージと付記」) (行末の空白は既定の表示では見えないので、黙って変えない) |
| SVT-6 | **大きい文書**: 文書が `TRANSFORM_SYNC_BYTES` (1 MiB、定数。§15.8 の TXT-6 と同じ) を超えるときは **変換せずに保存し**、保存の完了のメッセージに `info.save.suffix.transform_skipped_large` を付記する (04 §8.16。どちらかの設定が真のときだけ) (メインスレッドで 1 MiB を超える全文をなめない。ジョブにすると、確認と書出しの間に編集が割り込む窓と、`Esc` での取消しの状態が増える。§15.9.3) |
| SVT-7 | **確認・区分 B / D・つながり**: 変換は `PreConfirm` の判定の後に当たるが、**新しい確認の条件を生まない** ので、例外を置かない: (1) `PreConfirm` (§8.6 E2'・§8.7.1) の条件は、読込のときに決まる文書の性質 (「保持なし」「保持できなかった箇所がある」・v0.3 までの `lossy_lines`) で、編集では変わらない。(2) ISO-2022-JP (区分 D、§8.7) では、末尾の空白を消した行と末尾の改行を足した最後の行は **編集した行** になり、正規形のエスケープで書き直る (元のエスケープの書き方のバイト一致から外れる。01 NFR-303 AC-6 の「編集していない行」から外れる)。これは編集の結果で、確認の条件 (保持できなかった行) ではない。(3) 区分 B (CP932 の重複符号化など) の記録は、消した空白の範囲と重ならないので保つ (末尾の空白は ASCII で、記録の対象の文字ではない。ChangeSet が最小なので記録の範囲は写像で保たれる)。(4) つながり (`EscapeMerge`、§8.3.1): 変換で不正バイトのエスケープの文字の隣に来るのは区切り (対応するどの文字コードでも、不正バイトの続きのバイトにならない) だけで、末尾の改行はエスケープの文字の後ろには足さない (SVT-3) ので、新しいつながりを生まない。(5) 表現不能 (§8.6) は増えない (消すか ASCII の改行を足すだけ)。文字コード・BOM は変えない |
| SVT-8 | **書けずに終わったとき**: 変換を当てた連鎖が書けずに終わった (保存の失敗の後の中止・`Conflict` で中止・後続の取消し・`WritingRecovery` (退避先に保存) で終わった・全保存でその文書の保存が失敗した、など §9.7 の「中止」の全部) ときも、**変換は本文に残す** (普通の Undo のノードとして。1 回の Undo で戻る。自動では戻さない)。そのかわり **知らせる**: その保存の失敗・中止のメッセージ (`msg.save.*`・`info.save.aborted`・`info.save.to_recovery`) に `info.save.suffix.transform_kept` (「保存時の変換は本文に残っています。{key:edit.undo} で戻せます」) を付記し、全保存の失敗の一覧 (`ui.confirm.save_all_failed`) の各行の `{reason}` の後ろにも付ける。文書は dirty のまま。保存し直すと、新しい連鎖として SVT-4 で決める (変換の後の本文には除く空白が無いので、ふつうは何も変えない)。終了へ進む中止 (§9.7 (f) の「保存を中止して終了」) では知らせない (終了する)。自動で戻す案を採らない理由は §15.9.3 |
| SVT-9 | **費用**: 末尾の空白は本文を 1 回なめて求める (行の数と文書の大きさに比例)。メインスレッドで扱うのは 1 MiB まで (SVT-6)。Undo のノードは消した空白と足した改行だけを持つ |

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

- **検知手段**: (v0.1) 保存ジョブ内の比較 (§9.3 手順 0 / 5a)。外部の変更を取り込む手段は、利用者の明示の再読込 `file.reload` (下の「明示の再読込」) だけで、検知はしない。(v0.2) ユーザーの明示的なコマンド (`file.check_external_changes`) と、フォーカス復帰 (`InputEvent::Focus(true)`) 時に **ジョブで** `metadata` を取り `disk_stamp` と比較 (メインスレッドでは行わない。ADR-0020。`JobKind::Metadata` の目的 `MetaPurpose::ExternalCheck` (02 §9.3) を使う)。(v0.5) `FileWatcher` Port のイベント (core が待ち合わせてから照合する。細目は §9.5.1)。自分の保存によるイベントは `disk_stamp` と一致するので無視できる。比較キーは 02 §4.5 の `FileStamp` (mtime + len + FileId、16 MiB 以下は内容ハッシュ併用。計算規則は 05 §5.5)。
- **保存時の競合 (v0.1)**: 保存ジョブが `Conflict` を返したら (§9.3 手順 5a / 5c)、書き込まずに次の確認を出す (01 FR-117 AC-1)。選択肢は v0.1 では「上書き / 別名で保存 (既定) / 中止 (Esc)」の 3 つで、**このダイアログの再読込の選択肢は v0.2** (フォーカス復帰時の確認と同時。v0.1 は中止してから `file.reload` で読み直せる)。上書きを選んだら `expected` を `Conflict { current: Option<Box<FileStamp>> }` の `current` から決めて再投入する (`Some(stamp)` → `Expect::Stamp(stamp)`、`None` (ファイルが無い。開いた後に削除された) → `Expect::Absent`)。
  - 既存のファイルが開いた後に変更・削除された: `ui.confirm.external_change` (削除されていた場合の「上書き」はファイルを作り直す)
  - 新規ファイル (`Expect::Absent`) の保存先に同名のファイルが作られていた: `ui.confirm.save.target_exists`
- **明示の再読込 (`file.reload`、v0.1。FR-117 の Should の部分、確定 (ユーザー回答))**: 読込の経路は「文字コードを指定して開き直す」(§8.8) と同じで、適用だけが違う (開き直しは履歴を消すが、再読込は Undo できる下の「再読込の実現」)。
  1. when は `doc.has_path` (07 §11.6)。パスを持たない文書 (無題) ではパレットに淡色と理由で出て実行できない。読込中 (`load_state == Loading`) の文書では何もせず `info.reload.busy` を出す。保存ジョブの実行中は、その完了を待ってから 2. の判定をする (§8.8 と同じ)。読み取り専用の文書でも実行できる (再読込は利用者の編集ではなく、`Origin::Reload` は §6.5 の拒否の例外。エディタの外で属性を外したファイルを読み直す手段になる)。
  2. dirty なら、読み込む前に `ui.confirm.reload_discard` (04 §15.2) で確認する。選択肢は「`[L]` 再読込 (変更を破棄)」と `[Esc]` キャンセル で、既定はキャンセル (破壊的な選択肢は既定にしない)。キャンセルでは何も変わらない。dirty でなければ確認しない。
  3. 読込ジョブ (`JobKind::Open`) を同じ `DocumentId` で投入する。`OpenOptions.forced` に今の文書の文字コードを入れ (§8.2 手順 2。判定し直さず、バイナリ・UTF-32 の BOM の確認もしない。BOM は手順 2 の規則で読み飛ばして `has_bom` を決める)、今の文書が `LargeFileViewer` なら `confirmed_large` と `read_only_requested` を立てて巨大ファイルの確認を繰り返さない。それ以外の開く前の確認 (512 MiB 超・不正バイトの比率) は通常の読込 (§9.2) と同じ規則で出し、「中止」はキャンセルと同じ。二重オープンの判定 (`Identity`) は同じ文書なので行わない。読込中は `load_state = Loading` で **旧本文を表示し続け** (編集はできない。ステータスバーは `[読込中 n%]`)、`ui.cancel` で取り消せる。
  4. 取消し・失敗 (ファイルが無い・読めない等) では、旧本文・`TextFormat`・読み取り専用の理由・履歴をそのまま保って `load_state = Ready` に戻し、失敗は理由を示す (ファイルが無いときは `msg.reload.missing`、それ以外は読込と同じ `msg.open.*`。取消しは `info.open.cancelled`)。
  5. `Complete` を受けたら、下の「再読込の実現」で `Origin::Reload` の Transaction として適用し、`info.reload.done` を出す。読み取り専用の理由は §8.8 の「読み取り専用の理由の差し替え」(再読込の規則。`Binary` と `LargeFileViewer` は保つ) で付け直す。再読込の後に読み取り専用の文書では `info.reload.done` の代わりに `info.reload.done_read_only` を出す (Undo で戻せないため。下の「再読込の実現」、提案、OQ-216)。ステータスバーの文字コード・改行コード・読み取り専用の表示は新しい内容に合わせて変わる。
- **状態** (v0.2): 確認で「無視」を選んだときのファイルの Stamp を `Document.external_seen: Option<FileStamp>` に持つ (下の「同じ変更で何度も聞かない」)。`Some` の間は、文書の内容がディスクと違うと分かっているので、ステータスバーに **外部変更** (`StatusKind::ExternalChange`、04 §8.7) を出す。保存の成功・再読込・開き直しで `None` に戻り、表示も消える。確認なしで読み直したとき (未編集) と、削除の「新規として保持」(新規ファイル扱い) では出さない。確認を出している間と、確認を `Esc` で閉じたとき (次のフォーカス復帰でまた尋ねる) も出さない (v0.3 の M18 で実装)。
- **検知したときの選択肢** (v0.2。dirty かどうかで既定を変える):

| Document の状態 | 既定の挙動 |
|---|---|
| dirty でない | 自動で再読込 (設定で確認に変更可)。Selection は写像で維持 |
| dirty | 確認: 「再読込 (自分の変更を破棄) / 比較 (v1.L: 差分ビュー FR-517) / 無視 (上書き保存時に再確認)」 |
| 削除された | 「新規として保持 (保存で再作成) / 閉じる」 |

- **M9d の実装 (v0.2、フォーカス復帰と明示のコマンド)**:
  - 契機は 2 つ。(1) `InputEvent::Focus(true)`: **表示中の文書 (アクティブ) だけ** を確かめる (他の文書は保存時の照合 (§9.3 手順 0 / 5a) で守られ、切り替えた後に (2) で確かめられる)。`file.reload_on_external_change = "never"` なら確かめない。確認ダイアログ・プロンプト等のオーバーレイが開いている間は確かめない (次のフォーカス復帰で確かめる)。その文書のジョブ (`Pending::busy_doc`: 読込・保存・照合 (フォーカス復帰・`file.check_external_changes`・監視の照合) と、結果を文書に当てる非同期の編集。§9.5.1 の WATCH-5) がある間は、黙って飛ばさずに延ばし、ジョブが終わってから (まだその文書が前に出ていて、フォーカスを失っていなければ) 確かめる (v0.5 の M33 の段 8。§9.5.1 の WATCH-8 の (b) と段 8 の細目 (11))。`Focus(false)` は延ばしたフォーカス復帰の確かめを捨てる (§9.5.1 の段 8 の細目 (19))。ほかは何もしない。(2) `file.check_external_changes` (パレット。アクティブ文書。設定に関わらず確かめ、変わっていなければ `info.external.unchanged`)。
  - ジョブは `JobKind::Metadata { purpose: ExternalCheck }` で、16 MiB 以下は内容ハッシュも取る (05 §5.5)。比べ方は未編集の上書き保存の照合 (§9.3 手順 0) と同じ。フォーカス復帰の結果が届いたとき、その文書がもうアクティブでない、またはオーバーレイが開いていれば捨てる。
  - **変更あり**: dirty でなく、設定が既定の `auto_if_unmodified` なら確認なしで再読込する (`file.reload` と同じ経路。Undo で戻せる)。dirty か `ask` なら `ui.confirm.external_modified` (`[L]` 再読込 (自分の変更を破棄) / `[I]` 無視 ★)。
  - **削除**: `ui.confirm.external_deleted` (`[K]` 新規として保持 ★ / `[C]` 閉じる。閉じるは `buffer.close` と同じ手順で、dirty なら未保存の確認が出る)。保持は文書を新規ファイル扱いにする (`disk_stamp` を捨てる): 次の保存は `Expect::Absent` で確認なしにファイルを作り直し (その間に同名のファイルが作られていれば `ui.confirm.save.conflict_created`)、フォーカス復帰でも再び尋ねない (ファイルが無いのは新規ファイルの状態と同じ)。
  - **同じ変更で何度も聞かない**: 「無視」を選んだら、そのときのファイルの Stamp を `Document.external_seen` に覚え、フォーカス復帰で同じ Stamp を見つけても尋ねない (明示のコマンドでは尋ねる。削除の「新規として保持」は上のとおり新規ファイル扱いにするので覚える必要が無い)。保存の成功・再読込で忘れる。保存時の照合の確認 (`ui.confirm.save.conflict_*`) はこれと関係なく出る。
  - 照合の失敗 (読めない等) は、フォーカス復帰では何も出さず、コマンドでは `msg.external.check_failed` を出す。
  - 残し: 保存時の競合の確認の「再読込」の選択肢、アクティブでない文書の確認。ステータスバーの表示 (上の「状態」) は v0.3 の M18 で入れた。
- **再読込の実現**: 新内容をジョブで読み込み (§9.2 と同じ経路。文字コードは文書の現在の `encoding` を §8.2 手順 2 の明示指定として渡し、判定し直さない。BOM は手順 2 の規則で、その文字コードの BOM と一致すれば読み飛ばして `has_bom` を決める)、完了時に **旧テキスト → 新テキストの ChangeSet** を作って `Transaction(Origin::Reload)` として適用する。**再読込 (v0.1 の `file.reload`、v0.2 のフォーカス復帰時の確認とも) は全置換 ChangeSet** (`Delete(全体) + Insert(新全文)`。新全文は読込の rope を、旧全文は逆変換が旧 rope を、どちらも `InsertText::Rope` で共有するためコピーなし (§6.5 の「実装の状態」))。新しい内容に合わせて `TextFormat` (`has_bom`・`eol_stats`・`invalid_bytes`・`roundtrip`・`pua_escape` 等) と内容由来の読み取り専用の理由 (v0.4 は保持方式の記録も、§8.7.1) を置き換え、前後の値を Undo ノードの `format_change` に持たせる (Undo / Redo で本文と一緒に戻る、§7.2)。Selection は「同じ行:桁 (書記素)」を目安に写像し (各 Range の head を写した空のカーソル。選択は解く。§4.5.2)、Decoration は無効化する。行単位 diff による **最小差分** (imara-diff / similar) は、差分ビュー FR-517 (v1.L) と同時に導入する。**Undo で元に戻せる** (P1 原則)。ただし再読込の後に読み取り専用の文書 (読み取り専用の文書を読み直した、または読み直した内容に不正バイトがある等) では、`edit.undo` の when (`!doc.read_only`、07 §11.6) により Undo できない (再読込そのものは §6.5 の拒否の例外だが、`edit.undo` / `edit.redo` はコマンドの when の段階で実行できない。提案、OQ-216)。旧全文の保持によるメモリ増は 01 NFR-202 の例外 (§7.5)。再読込後 `history.saved` を再読込ノードの位置 (§7.4) にし、`disk_stamp` を更新。Undo で再読込前に戻ると、内容がディスクと違うので dirty になる。
- **自分の保存と外部変更の競合**: §9.3 手順 5a のジョブ内チェックで、競合時はファイルを壊さずに中止する。ユーザーが「上書き」を選んだ場合でも、元ファイルは原子的置換まで無傷 (Windows の置換途中失敗は退避名で残る、§9.4)。

#### 9.5.1 ファイル監視 (v0.5 の M33。FR-117 の監視の部分)

M33 の段 8 (07 §12.6.2) で入れる。受入基準は 01 §5.5 の FR-117 AC-7〜AC-10。OS ごとの監視の方式・監視できる FS・登録の呼び出しは 05 §12、Port の形は 02 §4.4。規則は M33 の仕様の PR (2026-11-22) で決めた (S-24 の「M33 で決めること」1〜9)。

| ID | 規則 |
|---|---|
| WATCH-1 | **監視する文書とディレクトリ**: パスのある文書 (無題・標準入力の文書は除く) の、canonical のパスの親ディレクトリを非再帰で監視する (シンボリックリンクで開いた文書は、リンク先のファイルのあるディレクトリ。開くジョブが返す canonical のパス、§12.7.6)。同じディレクトリの文書は 1 つの監視を共有し (数え上げ)、最後の文書を閉じたら外す。開いた (読込の完了)・名前を付けて保存でパスが変わった・再読込した・保存したときに、監視があることを確かめ (無い・失効した (WATCH-3 の `Lost`) なら張り直す。消して作り直したディレクトリにも効く)、パスが変わったら古いディレクトリの数を減らす。`file.watch.enabled = false` か `file.reload_on_external_change = "never"` のときは監視しない (`config.reload` で値が変わったら張る・外す。止めたら監視で付けた印を外し、監視からの読み直しを取り消す。入れ直したら監視を頼んだ文書を 1 回ずつ照合する。下の段 8 の細目 (12)・(15)・(16)) |
| WATCH-2 | **監視できないとき** (縮退): 登録の結果が失敗 (05 §12.2 の 4 / 5。監視しない FS (ネットワークの共有・WSL2 の DrvFs・許可リストに無い Linux の FS)・inotify の上限・権限・ディレクトリが無い) なら、そのディレクトリの文書は監視せず、v0.2 の確かめ方 (保存時・フォーカス復帰・`file.check_external_changes`) のままにする。知らせ (`info.watch.unavailable`。`{reason}` は理由の語、`{path}` はそのディレクトリの最初の文書のパス (文言は「{path} のフォルダ」。M33 の段 8 で決めた)) は **理由の種類ごとにプロセスで 1 回** (01 FR-117 AC-9)。ポーリングはしない (NFR-204。OQ-78 の明示の設定のポーリングは v0.6 以降) |
| WATCH-3 | **イベントの受け方**: `HostMsg::Fs` の `FsEvent` (02 §4.4) の `Changed { name }` は、そのディレクトリの文書のうちファイル名が `name` と同じもの (Windows は大文字小文字を区別しない序数の比較、Unix はバイト列) にだけ効かせる (自分の保存の一時ファイル・退避名 (05 §4.1) と、ほかのファイルの知らせはここで捨てる)。`Rescan` (イベントのあふれ・監視の誤り) はそのディレクトリの全文書に効かせる。`Lost` (監視中のディレクトリそのものの削除・改名) も全文書に効かせ、そのディレクトリの監視を「失効」にする (WATCH-1 の確かめで張り直す)。**Windows の短い名前 (8.3)**: どの文書のファイル名にも合わない名前で、8.3 の形 (基の名前は 8 文字以下で、6 文字以下の前の部分・`~`・数字の並び。拡張子は 3 文字以下) のもの (`LONGNA~1.TXT`。`~` を含むだけの `~$x.docx` (Office の一時ファイル)・`a.txt~` は当たらない。M33 の段 8 のレビューで絞った。短い名前でファイルを開いた外のプログラムの変更は、短い名前で届くことがある) は、そのディレクトリの `Rescan` として扱う (名前を長い名前に直す I/O をしない)。読むだけの開閉 (Linux の `IN_OPEN`) は eve-platform が捨てて送らない (05 §12.2 の 10) |
| WATCH-4 | **待ち合わせ**: 文書ごとに、最後のイベントから 100 ms (`WATCH_SETTLE`。05 §12.2 の 2) 待ち、最初のイベントから最長 1 s (`WATCH_SETTLE_MAX`) で打ち切って照合へ進む (書き続けるログのファイルでも知らせが出る)。**打ち切りの照合は、待ち合わせの期限 (最後に届いた知らせ + `WATCH_SETTLE`) を消さない。その期限が打ち切りの照合より後なら、期限でもう 1 回照合する** (打ち切りの照合の後に届いた知らせは、新しい待ち合わせとして同じ規則で扱う)。これで「届けた知らせ e ごとに、e の 100 ms (`WATCH_SETTLE`) 後以降に照合が必ず 1 回ある」が成り立つ。eve-platform は続けて届いた同じ名前の知らせを 50 ms の内でまとめて捨てる (05 §12.2 の 2) が、捨てた知らせの元の書込は、届けた知らせから 50 ms の内に済んでいるので、その知らせの 100 ms 後以降の照合が取り込む (打ち切りの照合の直前に届いた知らせの後の書込を捨てても、期限の照合が取り込む)。M33 の段 7 のレビューで足した。段 8 で core が持ち、手動の時計の試験で「打ち切りの照合の直前 (10 ms 前) に知らせが届き、その後の書込の知らせは捨てられた (届かない)」順を確かめる (07 §12.6.2 の段 8 の行)。期限は core の `next_deadline` に入れ、Host の待機がそれを待つ (一定の間隔で起きるスレッドを持たない、NFR-204。debouncer の crate を使わない)。どちらの時間も定数で、設定のキーは足さない |
| WATCH-5 | **照合と自分の保存** (OneDrive などのオンライン専用の属性 `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS` が立ったファイルは、内容ハッシュを使わず (mtime・長さ・FileId だけで比べ)、照合でファイルを呼び戻さない): 期限が来たら、その文書に保存・読込・再読込・照合のジョブ (M33 の段 8 のレビューで、結果を文書に当てる非同期の編集 (すべて置換・1 件置換・改行コードの変換・一致をすべて選ぶ・表現不能の文字の置換) も足した。判定は `Pending::busy_doc` の 1 か所で、フォーカス復帰の確かめも同じものを使う) があれば照合を延ばし (そのジョブの結果が届いてから、待たずに照合する)、無ければ `Metadata { purpose: WatchCheck }` (02 §9.3。`io`) を積む。結果を文書の `disk_stamp` と、05 §5.5 の規則 (16 MiB 以下は内容ハッシュを併用) で比べる: 同じなら何もしない (自分の原子的保存・直接上書きのイベントは、保存のジョブの結果で `disk_stamp` が新しくなった後なので、ここで同じになる。内容の変わらない属性だけの変更・同期ソフトの mtime の更新も同じ)。違えば **変更**、無ければ **削除** とする。照合の失敗 (読めない。書き手が排他で開いている間の共有違反など) は、0.5 s と 2 s の後にやり直し、それでも読めなければ何もしない (ログだけ。次のイベント・切替・フォーカス復帰・保存で確かめる)。自分の一時ファイルの名前で抑えることはしない (自分の保存の直後のほかのプログラムの書込を隠すため、S-24 の own-then-ext) |
| WATCH-6 | **知らせ方** (監視のイベントでモーダルの確認を出さない、05 §12.2 の 7): 変更・削除が分かったら、文書に外部の状態 (`Document.external_pending: Option<ExternalPending { Modified(FileStamp) \| Deleted }>`) を付け、ステータスバー (`StatusKind::ExternalChange`。削除は `status.external_deleted.*`、04 §8.7) とバッファ一覧の項目 (04 §8.5) に印を出し、メッセージを出す (`info.external.changed` / `info.external.deleted`。照合の回 (同じ `tick` で期限が来て積んだ照合) の結果がすべて返ったとき、ほかの文書の待ち合わせの期限が `WATCH_SETTLE` の内にあれば、知らせを保留して次の回の印と合わせる (保留は最長 `WATCH_SETTLE`。ただし照合の回が残っていれば、その回の結果まで (回が終わったときに知らせる)。照合は早めない)。こうして 2 つ以上の文書に付いたら `info.external.changed_many` (`{count}`) の 1 件にまとめる (`git checkout` の知らせは数 ms ずれて届く)。M33 の段 8 で「同じ `step`」から改めた: core は Host の `step` の区切りを知らず、`step` の予算 (8 ms) で区切りが変わる。段 8 のレビューで、同じ回だけでは 1 ms ずれた知らせがまとまらないので、保留を足した)。ただし **表示している (どれかのペインに出ている) 未編集の文書** で、変更 (削除ではない) で、`file.reload_on_external_change = "auto_if_unmodified"` で、次の上限の内なら、印を付けずに確認なしで再読込する (フォーカス復帰の「変更あり」と同じ経路。Undo で戻せる。読み直した後は `info.external.reloaded`): (a) ファイルが 16 MiB 以下 (05 §5.5 の内容ハッシュの上限と同じ。巨大ファイルの閲覧 (`LargeFileViewer`) の文書は読み直さない) (b) 同じ文書を監視で読み直してから 5 s (`WATCH_RELOAD_MIN_INTERVAL`。定数) 経っている (書き続けるログのファイルで、読み直すたびに旧全文を Undo に積まない。間の変更は印を付け、最後に読み直した 5 s 後に照合し直して読み直す。下の段 8 の細目 (2))。(c) Redo の履歴が無い (Undo で保存の時点に戻した文書は dirty でなくても読み直さない。読み直すと Redo が消える)。上限を超えたら印だけを付ける。読込中 (`Loading`) に利用者が打鍵したら (待っている貼り付けの応答が届いたときも)、再読込を取り消して印にする (打った文字・貼り付けを読込で捨てない)。監視からの再読込では、開く前の確認 (巨大ファイル・不正バイトの比率、§9.2) が要る内容になっていたら読み直さずに印だけを付ける (監視のイベントでモーダルの確認を出さない)。表示していない未編集の文書は読み直さず、印だけを付ける (切り替えたときに WATCH-8 で読み直す) |
| WATCH-7 | **同じ変更で何度も知らせない**: 照合の結果の stamp が、文書に付けた `external_pending` の stamp と同じ、または「無視」を選んだ stamp (`external_seen`、§9.5 の「同じ変更で何度も聞かない」) と同じなら、印もメッセージも出し直さない (大きいファイルの書込でイベントが 2 回に分かれても、見た目は 1 回。S-24 の所見 2)。**印が既に変更 (`Modified`) で stamp だけが変わった** (書き続けるファイル・表示中の文書の読み直しの最短の間隔の内の変更) ときは、印の stamp を黙って新しくし、メッセージを出さない。メッセージを出すのは、印の無い文書に付いたとき・変更と削除が入れ替わったとき・切替の照合 (WATCH-8 の (a)) のときだけ (M33 の段 8 のレビュー)。削除の後に同じ名前で作り直されたら、次の照合で変更として付け直す (変更と削除が入れ替わったので知らせる)。監視の知らせ (情報) は、表示中の未読の警告・エラーを置き換えず、メッセージの履歴にだけ積む |
| WATCH-8 | **確認の流れ** (今の流れに、外部の状態を足す): (a) 文書の切替 (`buffer.*`・タブのクリック・ファインダや横断検索で開いている文書を選ぶ・ペインのフォーカスの移動で別の文書が前に出る) で、前に出た文書に `external_pending` があれば、照合のジョブ (`WatchCheck`) を積み、結果が今も変更なら、未編集で `auto_if_unmodified` のときは WATCH-6 の上限の内で読み直し、それ以外 (dirty・`ask`・削除・上限の外) は **確認のダイアログを出さず**、メッセージ (`info.external.changed` / `info.external.deleted`) で知らせて印を残す (切り替えた直後に打った文字が、非同期に出たダイアログのアクセラレータに当たって編集を失わない。07 §12.6.3)。確かめて取り込むのは、利用者の明示の `file.reload`・`file.check_external_changes` と、フォーカス復帰の今の確認と、保存時の照合の確認。(b) フォーカス復帰 (`Focus(true)`) と (c) `file.check_external_changes` と (d) 保存時の照合は今のまま。ただし (b) で、その文書のジョブ (監視の照合など、下の WATCH-5 の「ジョブ」) の間だったら、黙って飛ばさずに、ジョブが終わってから (まだその文書が前に出ていて、フォーカスを失っていなければ) 確かめ直す (M33 の段 8 のレビュー)。`external_pending` は保存の成功・再読込・開き直し・「無視」(→ `external_seen`)・削除の「新規として保持」で外す |
| WATCH-9 | **印の表示**: ステータスバーの外部変更の項目は、v0.3 からの `external_seen` に加えて `external_pending` のときにも出す (長い形 `[外部で変更]` / `[外部で削除]`)。バッファ一覧は項目の印の欄に出す (04 §8.5)。タブライン (FR-510) には出さない (境界の型 `TabItem` を変えない。07 §12.6.3) |
| WATCH-10 | **ジョブの置き場所と遅れ**: 照合 (`WatchCheck`) と登録 (05 §12.2 の 9) は `io` に置き、`bg` を使わない。ワークスペースの走査・横断検索 (`bg`、§12.7.7) が長く続いても、監視の知らせは遅れない (M31 から送った「`bg` の詰まり」の件。07 §12.6.3)。`io` の長いジョブ (大きな文書の読込・保存) の後ろでは照合が待つが、待ち合わせが 1 回の照合にまとめるので積み上がらない |
| WATCH-11 | **多数のイベント**: `git checkout` などで 1 つのディレクトリに多数のイベントが来ても、主スレッドの 1 件の処理はファイル名の表引き (O(1)) だけで、照合は文書ごとに 1 回にまとまる。監視していない名前のイベントは捨てる |
| WATCH-12 | **設定ファイルの監視** (FR-706): 07 §6 の RLD-1〜RLD-5。待ち合わせ (WATCH-4) と縮退 (WATCH-2) は同じで、自分の保存を抑えない (WATCH-5 の照合をせず、待ち合わせの後に読み直す)。**M33 の段 9 で実装した** (`io/watch_config.rs`): 設定のディレクトリは文書の監視の表に設定の印を載せ、同じディレクトリの文書と 1 つの要求を共有する (ディレクトリは、文書が無く設定の印も無くなったときに外す。`file.reload_on_external_change = "never"` で止めるのは文書の登録だけ)。縮退の知らせ (理由の種類ごとに 1 回) も共有し、設定だけのディレクトリの `NotFound` は知らせない。細目は 07 §6 の「段 9 の実装」 |

- **実装の状態 (M33 の段 8、07 §12.6.2)**: eve-core の `io/watch_doc.rs` (監視の表・待ち合わせ・照合・知らせ方) と、`Document.external_pending` (`ExternalPending::{Modified(FileStamp), Deleted}`)・`MetaPurpose::WatchCheck`・`OpenKind::WatchReload` (監視からの再読込。開く前の確認が要る内容・失敗・取消しでは確認を出さずに印にする)・`FileMeta.offline` (WATCH-5 の呼び戻しの属性、02 §4.5) を入れた。定数は `WATCH_SETTLE` (100 ms)・`WATCH_SETTLE_MAX` (1 s)・`WATCH_RELOAD_MIN_INTERVAL` (5 s)・`WATCH_RELOAD_MAX_BYTES` (16 MiB)。段 8 で決めた細目 (表の行を補う。退けた案と理由は 07 §12.6.3): (1) 照合のやり直しは、1 回目の失敗の 0.5 s 後と 2 回目の失敗の 2 s 後 (WATCH-5)。(2) 最短の間隔 (WATCH-6 の (b)) で読み直さなかった文書は、印を付けたうえで、最後に読み直した 5 s 後に照合を 1 回積む (次の知らせを待たない。一回きりの期限で、一定の間隔では起きない)。(3) 開くジョブ・再読込で `FsKind::Network` と分かった文書のディレクトリは `FileWatcher` に頼まない (05 §12.2 の 4 の段 7 の既知の制約)。名前を付けて保存でパスが変わったら、この覚えを捨てて Port に判定させる。(4) 登録の失敗のうち、ディレクトリが無い (`NotFound`) ものだけを、失効 (`Lost`) と同じく次の確かめ (保存・再読込) で頼み直す。ほか (監視しない FS・上限・権限) は、そのディレクトリの文書をすべて閉じるまで頼み直さない (同じ失敗を繰り返して `io` を使わない)。存在しないフォルダで開いた新規ファイルは、保存してフォルダができてから監視する。(5) 監視からの再読込の読込中の打鍵 (キー・文字・貼り付け) は、表示中の文書 (アクティブ) のものだけを見る。読込を取り消して旧本文のまま `Ready` に戻し、印とメッセージ (`info.external.changed`) を出してから、その打鍵を通常どおり処理する。(6) 切替 (WATCH-8 の (a)) は、`tick` ごとにアクティブな文書が変わったかで見つける (切替の経路ごとに入口を足さない)。切替の照合の結果は、印が前と同じでもメッセージを出す (切り替えた利用者に知らせる)。(7) 照合の結果が「同じ」(ディスクの内容と一致した) なら、付けていた印を外す (外部で元の内容に戻された)。フォーカス復帰・`file.check_external_changes` の照合で同じと分かったときも外す。**段 8 のレビューで足した細目**: (8) 照合を積んだ後にその文書の知らせが届き、待ち合わせの照合がまだ来ていないなら、その結果は当てずに次の照合に任せる。ただし打ち切りの照合の結果は捨てない (WATCH-4 の最長 1 s を守る)。続けても捨てない。(9) 監視からの再読込は、読んだ内容が 16 MiB を超えていたら当てずに印にする (照合の後に大きくなった)。読んでいる間に変わった (`changed_during_load`) なら、読み直した後に印を残し、待ち合わせの後に照合し直す。(10) 待っている貼り付けの応答 (`InputEvent::ClipboardContent`) も打鍵と同じく監視の読み直しを取り消す (貼り付けを捨てない)。旧本文に戻すどの経路 (取消し・開く前の確認の中止・監視を止める) でも、横断検索の保留のジャンプを済ませる (`reopen_restore` が §12.7.5 の段 7 の細目 (11) の移動を行う)。(11) ジョブの結果無しに `Ready` に戻った文書 (再読込の開く前の確認を閉じたなど) の延ばした照合は、次の `tick` で再開する。(12) `file.watch.enabled = false`・`never` にしたら、監視で付けた印 (`external_pending`) を全部外し、監視からの再読込の覚え・照合の回・保留した知らせを捨て、その後に届いた照合の結果と切替では何もしない (「無視」の `external_seen` は確認の流れのものなので残す)。(13) 「無視」は、尋ねた変更の stamp の印だけを外す (確認の間に届いた新しい変更の印は残す)。(14) 監視の止め・入れ直しでも、フォルダの無い所に開いた新規ファイルは保存するまで頼まない。**段 8 の再レビューで足した細目**: (15) 監視を止めるとき、監視からの読み直しのジョブは取り消して旧本文のまま `Ready` に戻す (読込中の打鍵を捨てない。止めた後に届いた結果も当てない)。(16) 監視を入れ直したら、監視を頼んだ文書を 1 回ずつ照合する回を積む (止めていた間の変更に気付く。知らせのまとめ方は WATCH-6。監視できないと分かっているフォルダの文書は、監視していない文書と同じく照合しない)。止める前に積んで、入れ直した後に届いた照合の結果 (その回はもう無い) の印は、保留に足して今の回と合わせて知らせる (知らせを落とさない)。(17) 保留した知らせは、出すときに今の印 (`external_pending`) で数え直す (保留の間に印が外れた文書・閉じた文書は数えない。切替の照合で知らせた文書は保留から外す)。「近い期限」は保留している文書のほかの文書の待ち合わせだけを見る。(18) 「無視」の stamp (`external_seen`) と印の stamp は `same_on_disk` で比べる (オンライン専用のファイルの監視の照合は内容ハッシュを持たない)。(19) 延ばしたフォーカス復帰の確かめは、フォーカスを失ったら捨てる。(20) 監視の読み直しの知らせ (`info.external.reloaded`・`info.reload.done_read_only`) も、表示中の警告・エラーを置き換えない。読んでいる間に変わった (細目 (9)) ときは、印だけを残し、知らせは `reloaded` の 1 件にする

### 9.6 読み取り専用・バイナリ (FR-115 / 116)

- 読み取り専用の理由は **理由の集合** `ReadOnlyReasons` (02 §5.1。変種は `ReadOnlyReason`、宣言順 = 下表の行の順 = 表示の優先順) で持ち、ステータスバーに `[RO]` と理由 (`ReadOnlyReasons::primary()`) を表示する。読込中は理由ではなく `load_state == Loading` から導出する (ステータスは読込の進捗を出す)。編集の可否は「`read_only` が空 かつ `load_state == Ready`」で、when の `doc.read_only` (07 §7.3) はその否定 (理由が 1 つ以上ある、または `load_state != Ready`) を返す。**読み取り専用の文書への編集操作** (文書を変える全コマンド。文字の入力・削除・貼り付け・Undo / Redo を含む) は、Transaction を作る前に拒否し、理由付きのメッセージ (`msg.edit.read_only`、引数 `{reason}` は理由ごとの文言、04 §15.2) を出す。文書は変わらない。例外は再読込 (v0.1 の `file.reload`、§9.5) とそのノードの Undo / Redo で、内容由来の理由を本文と一緒に戻すので拒否しない (§7.2)。when 条件 `!doc.read_only` (07 §7.3) を持つコマンドはパレットに淡色で出る。
- **理由と解除の可否** (本表が正本。01 FR-116、07 `doc.read_only_releasable` はこの表を引く。v0.1 には解除の手段が無く、`doc.read_only_releasable` は常に false。v0.2 は `FileAttribute` / `UserRequested`、v0.4 は加えて `InvalidBytes` (§8.3.1)):

| 理由 | 付く時点 | 解除 |
|---|---|---|
| `FileAttribute` | 開くとき、ファイルに読み取り専用属性 (Windows の READONLY) がある (05 §5.2) | v0.2: `file.toggle_read_only` で解除できる。解除した文書の上書き保存は「読み取り専用を解除して保存」(属性を外して置換し、置換の後に付け直す。05 §4.2、FR-106) の確認を経る |
| `UserRequested` | `--readonly` で開いた (07 §18.3) | v0.2: `file.toggle_read_only` で解除できる |
| `NoWritePermission` | 保存が `PermissionDenied` で失敗した (v0.1 の Windows は開く時点では書込権限 (ACL) を判定しない楽観方式、05 §5.2) | 解除できない (書けないことが分かっているため)。名前を付けて保存で回避する |
| `InvalidBytes` | 読込で不正バイトを検出した (§8.3)。私用文字との衝突で固定した文書 (§8.3 の「衝突」(b)) もこの理由 | v0.4 (M23): `file.toggle_read_only` で、確認 (`ui.confirm.read_only.invalid_bytes`) の後に解除できる (「編集を許可」。条件・確認・付け直しの正本は §8.3.1)。ただし `pua_escape == DisabledCollision` (衝突で固定した文書) は解除できない。v0.1〜v0.3 は解除できず、「文字コードを指定して開き直す」(FR-111) で回復する |
| `StatefulEncodingWithInvalidBytes` | 状態付き符号化 (ISO-2022-JP) で不正バイトを検出した (§8.3) | 解除できない (FR-111 で別の文字コードとして開き直す) |
| `Binary` | バイナリの警告で「読み取り専用で開く」を選んだ (FR-115) | 解除できない (NG10) |
| `LargeFileViewer` | 巨大ファイル (512 MiB 超) の警告で「読み取り専用で開く」を選んだ (FR-127、§11.2)。不正バイトの多いファイルの警告 (FR-127) で「読み取り専用で開く」を選んだ文書には付けず、理由は `InvalidBytes` だけになる (v0.4 では `InvalidBytes` の解除の対象。§8.3.1) | 解除できない (開き直さずに編集できると、FR-127 の警告の意味が無くなるため) |
| `LoadIncomplete` | v0.1〜v0.3 では付かない (読込の失敗では読めた部分を開かない、§9.2 の「失敗」)。v0.4 (M25): 標準入力の読込を、上限 (`file.huge_file_warn_bytes`)・取消し (`Esc` の確認の「ここまでで開く」)・途中の I/O エラー・確保の失敗でやめて、読んだところまでを開いたとき (§9.2.2)。ファイルの読込では v0.4 でも付かない | 解除できない (編集して保存すると、読まなかった残りが無いことに気づかないまま後半を失うため)。名前を付けて保存は可で、**成功したら外す** (その文書は保存したファイルの全体と一致するため。完了のメッセージは `info.save.partial_saved`、§9.2.2 の「`LoadIncomplete` の外し方」)。再読込・開き直しでは必ず外す (§8.8) |

- v0.1 にあった理由 `LongLine` (閾値を超える行を含む、§11.5) は、v0.2 の M9d で長大行を char 近似モードで編集できるようにしたので外した (§11.5.1。変種も無い)。
- 理由が複数あるときは全部を保持し、表示はこの表の上にあるものを先に出す。**名前を付けて保存** は読み取り専用の文書でも実行できる (v0.1 で読み取り専用の文書を保存する唯一の手段)。例外: `StatefulEncodingWithInvalidBytes` の文書 (不正バイトを含む ISO-2022-JP) は、ISO-2022-JP のままの名前を付けて保存・ISO-2022-JP を指定した保存を連鎖の開始時に `msg.save.encoding_unsupported_iso2022` で拒否する (保存先を尋ねない。§8.3、確定 (ユーザー回答)、OQ-207)。別の文字コードを指定した保存はエスケープ文字が表現不能 (E4) なので E3 のキャンセルだけになり、この文書を書く手段は無い (FR-111 で別の文字コードとして開き直す)。成功したら保存先について `FileAttribute` / `NoWritePermission` を判定し直し (外れれば編集できる)、`LoadIncomplete` (v0.4) を外し、それ以外の理由は残す。 名前を付けて保存の対象は **名前の入力を開いた時点の文書** に固定し、入力の間に表示が別の文書へ切り替わっても、確定ではその文書を保存する (§9.8 の「入力・確認の対象」)。
- 解除の取り消し (`file.toggle_read_only` をもう一度) は、どの文書でも `UserRequested` を付ける (v0.2)。
- **M23 の規則 (v0.4)**: `doc.read_only_releasable` は「読込が済み、理由があり、すべてが `FileAttribute` / `UserRequested` / `InvalidBytes` で、`InvalidBytes` を含むなら `pua_escape == Enabled`」。`InvalidBytes` を含む文書の解除は、外す前に確認 (`ui.confirm.read_only.invalid_bytes`) を 1 回出す。解除の記録 (`Document.invalid_bytes_released`) を持ち、同じ文書の再読込では解除を持ち越し、文字コードを指定して開き直す・新しく開く・復旧の読込では付け直す (§8.8 の「読み取り専用の理由の差し替え」)。読み取り専用の理由が `InvalidBytes` だけ (`pua_escape == Enabled`) の文書にも復旧を提案・適用する (§10.3)。細目は §8.3.1 の「編集を許可」。
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
    strategy: SaveStrategy,              // Atomic { hardlink } | InPlace { backup } (§9.3 手順 4、02 §9.3)
    hardlink: HardlinkPolicy,            // Ask (既定、07 §5.2 file.hardlink_policy) | BreakLink | WriteInPlace
    in_place_backup: InPlaceBackup,      // Required (既定) | SkipConfirmed
    escape_merge_ok: Option<EscapeMergeApproval>, // v0.4 の M23: つながり (§8.3.1) の承認 (Revision と、承認した結果 (件数・先頭 100 件・全件のダイジェスト))。投入のたびに SaveApprovals から
                                         // この文書の今の Revision のものを引いて入れる (無ければ None)。ジョブは結果が承認と同じときだけ止めない
    state: SaveChainState,
}
```

**各項目を更新する遷移** (表に無い遷移では項目を変えない。保存の重なり (§9.3 手順 4 の深さ 1 のキュー) で再実行するときの `expect` は §9.3 のとおり直前の `report.new_stamp`):

| 遷移 | `target` | `expect` | `format` | `method` | `hardlink` | `in_place_backup` | `escape_merge_ok` |
|---|---|---|---|---|---|---|---|
| 連鎖の開始 (`file.save`、`AskUnsaved` の「保存」、`file.set_encoding`) | 文書のパス (無題・読み取り専用なら None のまま `AskName` へ) | `disk_stamp` が Some なら `Stamp(disk_stamp)`、None なら `Absent` | 文書の format (`file.set_encoding` は新しい format、§8.8) | `Atomic`。`file.save_strategy = "in_place"` で T が存在する (`disk_stamp` が Some) なら `InPlace` (§9.3 手順 4) | 設定値 (既定 `Ask`) | `Required` | `SaveApprovals` にこの文書の今の Revision の承認があればそれ、無ければ `None` |
| `CheckTarget` で保存先が決まる ((a) の 1: 今の文書と同じファイル) | 文書のパス | `Stamp(disk_stamp)` (§9.3 手順 0 から) | 変えない | 開始時と同じ規則 | 設定値 | `Required` | 投入のたびに `SaveApprovals` から引き直す (Revision が同じなら同じ承認) |
| 同 ((a) の 2: 存在しない) | 入力したパス | `Absent` | 変えない | `Atomic` | 設定値 | `Required` | 投入のたびに `SaveApprovals` から引き直す (Revision が同じなら同じ承認) |
| `ConfirmOverwrite` で「上書き」((a) の 6) | 入力したパス | `Stamp(取得した stamp)` | 変えない | `Atomic`。`save_strategy = "in_place"` なら `InPlace` | 設定値 | `Required` | 投入のたびに `SaveApprovals` から引き直す (Revision が同じなら同じ承認) |
| `AfterFailure` の `Conflict` で「上書き」 | 変えない | `Conflict { current }` の `current` が `Some(stamp)` なら `Stamp(stamp)`、`None` (不在) なら `Absent` | 変えない | 変えない | 変えない | 変えない | 投入のたびに `SaveApprovals` から引き直す (Revision が同じなら同じ承認) |
| `AfterFailure` のハードリンクで「リンクを保って直接上書き」/「切り離して保存」 | 変えない | 変えない | 変えない | `InPlace` / `Atomic` | `WriteInPlace` / `BreakLink` | 変えない | 投入のたびに `SaveApprovals` から引き直す (Revision が同じなら同じ承認) |
| `AfterFailure` の `AtomicUnsupported` で「直接上書き」 | 変えない | 変えない | 変えない | `InPlace` | 変えない | 変えない | 投入のたびに `SaveApprovals` から引き直す (Revision が同じなら同じ承認) |
| `AfterFailure` の `BackupFailed` で「バックアップなしで上書き」 | 変えない | 変えない | 変えない | `InPlace` (のまま) | 変えない | `SkipConfirmed` | 投入のたびに `SaveApprovals` から引き直す (Revision が同じなら同じ承認) |
| `AfterFailure` の `Busy` で「再試行」 | 変えない | 変えない (何も書いていないので照合は通る) | 変えない | 変えない | 変えない | 変えない | 投入のたびに `SaveApprovals` から引き直す (Revision が同じなら同じ承認) |
| `AfterFailure` の E3 で「UTF-8 で保存」 | 変えない | 変えない | `encoding = UTF-8`、`has_bom = false` (他は元のまま、§8.6) | 変えない | 変えない | 変えない | 投入のたびに `SaveApprovals` から引き直す (Revision が同じなら同じ承認) |
| `ConfirmReplacement` で「置換して保存」 | 変えない | 変えない | 変えない (置換の Transaction は文書に適用する) | 変えない | 変えない | 変えない | 投入のたびに `SaveApprovals` から引き直す (Revision が同じなら同じ承認) |
| `AfterFailure` の `EscapeMerge` で「このまま保存」(v0.4 の M23) | 変えない | 変えない | 変えない | 変えない | 変えない | 変えない | 結果の base Revision が今の Revision と同じなら、結果 (件数・先頭 100 件・全件のダイジェスト) を承認として `SaveApprovals` に覚えて入れる。違えば覚えずに `None` のまま今の Snapshot で投入し直す (§8.3.1) |
| 「別名で保存」で `AskName` へ戻る | `CheckTarget` で上の行のとおり決め直す | 同左 | 変えない | 同左 | 同左 | 同左 | 投入のたびに `SaveApprovals` から引き直す (Revision が同じなら同じ承認) |

**連鎖の開始時の拒否**: 上の表の「連鎖の開始」の契機 (`file.save`・`file.save_as`・`AskUnsaved` の「保存」・`file.set_encoding`) で `format` を決めた直後、`AskName` / `CheckingUnchanged` / `PreConfirm` のどれにも進む前に、`format.encoding` が ISO-2022-JP で文書が §8.3 のエスケープ文字を含む (`invalid_bytes > 0`) なら、ジョブを投入せずに `msg.save.encoding_unsupported_iso2022` を示して連鎖を中止する (`cont` の後続も実行しない。文書・`TextFormat` は変えない。§8.3 の (a)、確定 (ユーザー回答)、OQ-207)。全保存 (§9.8) の文書ごとの保存も同じ判断で拒否し、メッセージは出さずに、その文書を保存せず理由 (このメッセージの text) を添えて失敗の一覧に載せる。

**状態** (`SaveChainState`):

| 状態 | 入る契機 | 利用者の操作・ジョブの結果 → 次の状態 |
|---|---|---|
| `AskUnsaved` | dirty な文書で後続を伴う操作 (終了・置き換え・開き直し) を実行した | 保存 → 文書がパスを持ち、かつ読み取り専用の理由が無い (`read_only` が空) なら `PreConfirm` (確認が要らなければ素通りして `Saving`)。パスが無い (無題)、または **読み取り専用 (dirty でも)** なら `AskName` (読み取り専用の文書の上書き保存は無効、§9.3 手順 1) / 破棄 → 後続をすぐ実行 / キャンセル (`Esc`) → 中止 |
| `AskName` | 無題の文書の保存、`file.save_as`、読み取り専用の文書の `file.set_encoding`、`AskUnsaved` の保存 (無題・読み取り専用) | パスを入力 → `CheckTarget` / キャンセル (`Esc`) → 中止 |
| `CheckTarget { job }` | 名前を付けて保存の保存先が決まった (`JobKind::Metadata { purpose: MetaPurpose::SaveAsTarget }`) | 下の (a) の規則 → `ConfirmOverwrite` / `PreConfirm` / エラーを示して `AskName` に戻る ((a) の 1 で未編集なら、§9.3 手順 0 のとおり `CheckingUnchanged` へ) |
| `CheckingUnchanged { job }` | 未編集の文書の上書き保存 (§9.3 手順 0。`disk_stamp` が Some で dirty でない)、または (a) の 1 で未編集。`JobKind::Metadata { purpose: MetaPurpose::UnchangedSave }` を投入した | 下の「全状態の遷移表」 |
| `ConfirmOverwrite` | 保存先に既存のファイルがある ((a) の 6) | 上書き → `PreConfirm` / 別の名前を入力 (**既定**。`Esc` も同じ) → `AskName` に戻る (入力した名前は残す) |
| `PreConfirm` | 投入の前の確認が要る (§8.6 E2' の区分 B、ISO-2022-JP のエスケープの正規化。v0.4 は §8.7.1 の「保持なし」「保持できなかった箇所がある」文書だけ) | 保存 → `Saving` / 別名で保存 → `AskName` / キャンセル → 中止。確認が要らなければ素通りして `Saving` |
| `Saving { job, node, cancel_origin }` | 保存ジョブを投入した (§9.3 手順 4)。`cancel_origin` は投入時に `None` | 下の「全状態の遷移表」。`cancel_origin` は取消しの由来で、`None` (取り消していない) / `User` (`ui.cancel` で後続を取り消した。`cont` が Some のときだけ立つ) / `QuitAbort` ((f) の `[Q] 保存を中止して終了`) の 3 値。結果の扱いは `cancel_origin` ごとに遷移表の行を分ける |
| `BuildingReplacement { job }` | `AfterFailure` の E3 で「置換して保存」を選んだ。`JobKind::ReplaceUnmappable { snapshot, format }` を `bg` に投入した (§8.6) | `JobResult::UnmappableReplacement` → 結果の base Revision が今の Revision と同じなら `ConfirmReplacement`、違えば (待つ間に編集が入った) 結果を捨て、今の Snapshot でジョブを投入し直して `BuildingReplacement` のまま / `Failed` → エラーを示して中止 |
| `ConfirmReplacement { changes, base }` | 置換の ChangeSet ができた | `ui.confirm.save.unmappable_replace.message` (§8.6 の「置換の一覧」。`targets` は先頭 100 件、件数は `counts { kana, compat, question }` で、総数はその合計) を出す。置換して保存 → 今の Revision が `base` と同じなら `Transaction(Origin::System)` として適用し、同じ文字コードで `PreConfirm` へ (違えば `BuildingReplacement` に戻って作り直す) / キャンセル (既定。`Esc`) → E3 の選択肢 3 と同じ扱いで中止 |
| `AfterFailure { err }` | 保存が失敗し、利用者に選ばせる (§9.3 手順 7 の分類) | 下の (e) の表 |
| `WritingRecovery { job }` | (e) で「退避先に保存」を選んだ。`JobKind::WriteRecovery` を投入した (下の「退避先に保存」) | `JobResult::RecoveryWritten { path }` → `info.save.to_recovery` を示して中止 (後続は行わない) / `Failed` → `msg.save.recovery_failed` を示して中止 |
| 完了 | `JobResult::Saved` (`CheckingUnchanged` では照合の一致) | §9.3 手順 7 の成功処理と、format の確定 (§8.8) (`CheckingUnchanged` からは書いていないので `info.save.unchanged` を示すだけ)。`cancel_origin = User` なら `cont` を捨てる (取り消した後続は実行しない)。`cont` があれば、文書が dirty でない (保存した時点の後に編集が無い) ときだけ後続を実行する。保存中の編集で dirty なら後続を実行せず `AskUnsaved` をもう一度出す |
| 中止 | 上のキャンセル・中止 | `cont` を捨てる (終了・置き換え・開き直しはしない)。文書は元の編集状態のまま (本文・dirty・format・選択は変えない。E3 の置換を適用した後の中止では、置換の Transaction は残り Undo で戻せる。v0.5 の M34 の保存時の変換 (§9.3.1) を当てた後の中止でも、変換の Transaction は残り Undo で戻せ、中止のメッセージに `info.save.suffix.transform_kept` を付記する (SVT-8))。保存しなかったことをメッセージ (`info.save.aborted`) で示す (退避先に保存の後の中止では `info.save.to_recovery` / `msg.save.recovery_failed` だけを、`CheckingUnchanged` の照合の失敗による中止では `msg.save.check_failed` だけを示す) |

**全状態の遷移表** (状態 × 事象。`file.save` は `file.save_as` / `file.set_encoding` を含む同じ文書の保存要求。「無視」は状態を変えず、保存中であること (`info.save.in_progress`、ステータスは `status.saving.*`、04 §8.7) を示す。確認・入力の状態では、オーバーレイが入力を受けるので `ui.cancel` はオーバーレイの `Esc` として届く。ジョブを待つ状態 (`CheckTarget`・`CheckingUnchanged`・`BuildingReplacement`・`Saving`・`WritingRecovery`) はオーバーレイを出さず、`Esc` は 04 §7.5 の `Esc` (`ui.cancel`) の意味の層として本表の列のとおりに届く):

| 状態 | `ui.cancel` (`Esc`) | `app.quit` | `file.save` | `file.open` / `file.new` (置き換え、§9.2) | ジョブの結果 |
|---|---|---|---|---|---|
| `AskUnsaved` | 中止 (キャンセルと同じ) | 無視 (この確認に答えてから実行し直す) | 無視 | 無視 (この確認に答えてから実行し直す) | — (ジョブなし) |
| `AskName` | 中止 | 無視 | 無視 | 無視 | — |
| `CheckTarget` | 照合ジョブの結果を捨て、`AskName` に戻る (入力した名前は残す。今の文書に照合中のものが無ければ、最後に投入した照合を取り消す。表示していない文書の読込の取消しより先に判定する、§12.7.6 の細目 (9)) | 無視 (`info.save.in_progress`。終了の連鎖を始めない) | 無視 | 無視 (`info.save.in_progress`) | `Meta` / `TargetAbsent` / `Failed` → (a) の表。ほかのオーバーレイか検索バーが出ていれば、閉じるまで保留する (§12.7.6 の細目 (7)) |
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
- **保存時の変換** (v0.5 の M34、§9.3.1): 連鎖は「変換を判定した」印を持ち、変換は最初に保存ジョブを投入する直前に同期で 1 回だけ当てる (状態は足さない)。変換は普通の Undo のノードで、書けずに終わった連鎖でも本文に残し、失敗・中止のメッセージに付記して知らせる (§9.3.1 の SVT-8)。
- **連鎖の中の確認** (外部変更・ハードリンク・原子的置換不可・表現不能・区分 B 等) は連鎖の外の保存と同じ規則・同じ選択肢で、選んだ結果が「保存を続ける」なら `cont` を持ったまま上の「各項目を更新する遷移」で項目を変えて投入し直し、それ以外は中止する。
- 外部からの終了 (コンソールクローズ等) は連鎖を待たずに緊急退避する (FR-107 AC-2、§10.1)。

**(a) 名前を付けて保存の保存先** (`CheckTarget`。I/O はジョブ。保存先の照合は `JobKind::Metadata`)

| 順 | 保存先の状態 | 扱い |
|---|---|---|
| 1 | 入力したパスが今の文書のパスと構文的に同じ (`Paths::normalize` の後、05 §3.2)、または metadata の `file_id` が今の文書の `file_id` と同じ (別名・リンク・ハードリンクで同じファイルを指す。比べ方は §12.7.6 の 0。v0.4 までの実装はパスの文字列だけで比べていた。`file_id` と canonical の比較は v0.5 の M31 で入れた、SAF-28。構文的に同じパスは照合のジョブを投げずに I/O なしで決める) | **通常の上書き保存** として扱う (§9.3 手順 0 から。未編集なら書かない)。ただし `file.set_encoding` の連鎖 (連鎖の `format` が文書の format と違う) は手順 0 を飛ばして書く (§9.3 の「手順 0 の適用対象」)。ただし文書が読み取り専用なら上書き保存は無効 (§9.3 手順 1) なので、「読み取り専用の文書は同じファイルには保存できない」(`msg.save_as.same_file_read_only`) と示して `AskName` に戻る |
| 1b | (v0.5 の M31、SAF-28) 保存先が **ほかの開いている文書** と同じファイル (1 と同じ比べ方。§12.7.6 の 0) | 保存せず、`msg.save_as.open_in_other` (「{path} は別の文書 {other} として開いています」。`{other}` は相手の文書の表示用のパスで、リンクでは名前が違う) を示して `AskName` に戻る (入力した名前は残す)。2 つの文書が 1 つのファイルを指すと、二重オープンの不変条件 (§9.2 手順 2b) と外部変更の検知 (§9.5) が崩れるため。v0.2〜v0.4 の実装は 1 をパスの文字列で比べるだけで、1b は無く、照合のジョブも投げずに `Expect::Absent` で保存していた (既存のファイルは `Conflict` からの上書きの確認 `ui.confirm.save.conflict_created` になり、読み取り専用属性・フォルダも書く段で失敗していた)。v0.5 の M31 で表のとおりにした |
| 2 | 存在しない (`NotFound`、詳細なし) | `Expect::Absent` で保存する (書くまでに他者が作れば `Conflict`、`ui.confirm.save.target_exists`、§9.5) |
| 3 | リンク先が存在しない (`NotFound` + `DanglingLink`) | v0.1 はエラー (リンク先の絶対パスを示す、§9.3 の「シンボリックリンク」)。`AskName` に戻る。v0.2 もこの行は同じ (名前の入力で選んだリンクには作らない。リンク先を作る確認は、開いた文書の保存がダングリングで失敗したときだけ。下の (e) の表) |
| 4 | ファイルでない (`FileMeta.kind` が `Dir` / `Other`)、パスが不正 (`InvalidPath`。予約名を含む) | エラー (`msg.save_as.not_a_file` / `msg.save.invalid_path`) を示して `AskName` に戻る |
| 5 | 読み取り専用属性のファイル (`FileMeta.readonly`) | 上書きの確認を出す前に拒否する (`msg.save_as.target_read_only` を示し、`AskName` に戻る。今の文書の理由は変えない)。v0.1 は読み取り専用属性のファイルに書かない (FR-116) |
| 6 | 既存のファイル (リンクならリンク先) | `ui.confirm.save.overwrite` (「{path} は既にあります。上書きしますか」。リンクのときはリンク先のパスも示す。既定は「別の名前を入力」で `AskName` に戻る (04 §15.2)) を出し、上書きなら metadata で取得した stamp で `Expect::Stamp(stamp)` として保存する (確認の間に変わっていれば `Conflict`)。保存は §9.3 と同じ手順で、ハードリンク等は手順 7 の確認になる |
| 7 | metadata が `PermissionDenied` / `Busy` / `Other` で失敗 | エラー (`msg.save.permission_denied` / `msg.save.busy` / `msg.save.failed`) を示して `AskName` に戻る |

- 名前を付けて保存の保存が `PermissionDenied` で失敗しても、今の文書に `NoWritePermission` は付けない (失敗したのは別のパス)。`ui.confirm.save_failed` で別名での保存を案内する。
- 成功したら §9.3 の「名前を付けて保存」の更新を行う。

**(b) 文字コードを指定して保存** (`file.set_encoding`) と **(c) 文字コードを指定して開き直す** (`file.reopen_with_encoding`) の規則は §8.8。(b) は連鎖の `format` に新しい format を入れて投入し、`TextFormat` を確定するのは完了のときだけである。(c) で dirty なら `AskUnsaved` から入り、`cont = Reopen` とする。

**(d) FR-107 の 3 択の「保存」**: 上の `AskUnsaved` から入る連鎖そのもの。保存が成功した場合だけ後続 (終了・置き換え・開き直し) を実行し、保存の失敗・キャンセル・`Conflict` や表現不能の確認での中止では後続を中止して元の編集状態に戻る (保存時の変換 (v0.5 の M34) を当てた後なら、変換は本文に残り Undo で戻せる。§9.3.1 の SVT-8)。

**(e) 保存失敗の後の選択肢** (`AfterFailure`。本表が正本で、04 §15.2 の MessageKey の選択肢はこれに合わせる)。本表が扱うのは **保存ジョブ (書込系) の失敗だけ** で、`CheckingUnchanged` の照合 (`JobKind::Metadata`) の失敗は本表に流さず、全状態の遷移表の `CheckingUnchanged` の行 (`msg.save.check_failed` を示して中止) で扱う。`CheckTarget` の metadata の失敗は (a) の 7。連鎖の外の保存 (`file.save` / `file.save_as` を単独で実行した) も同じ表を使い、「後続」の列だけが意味を持たない。`msg.save.*` と `ui.confirm.save_failed` の組は、メッセージの文言を確認の本文に入れて 1 つの確認として出す (04 §15.2)。★ は既定、`Esc` はどの確認でも「中止」。`ui.confirm.save_failed` を使う行 (`Busy`・`PermissionDenied`・`DiskFull` / `ReadOnlyFs`・`InPlaceFailed`・`ReplacedMovedToBackup`・`ParentMissing`・`DanglingLink`・その他) の選択肢は本表のとおりで、`Busy` の行だけが「再試行」を加える (04 §15.2 の目録の `ui.confirm.save_failed` はこの表に合わせる)。

| 失敗 (§9.3 手順 7) | MessageKey | 選択肢 → 次の状態 | 後続 (`cont`) |
|---|---|---|---|
| `JobResult::Unmappable` | `ui.confirm.save.unmappable` | UTF-8 で保存 → `Saving` (連鎖の `format` を UTF-8 に) / 置換して保存 → `BuildingReplacement` / キャンセル ★ → 中止 (最初の該当位置へ移動)。出す条件は §8.6 の「E3 の選択肢」 | 保存を続ける選択では保つ。キャンセルで捨てる |
| `JobResult::EscapeMerge` (v0.4 の M23。エスケープ文字を戻したバイトが隣とつながる、§8.3.1。何も書いていない) | `ui.confirm.save.escape_merge` | このまま保存 → `Saving` (結果の base Revision が今の Revision と同じなら、結果 (件数・先頭 100 件・全件のダイジェスト) を承認として `SaveApprovals` に覚え、`escape_merge_ok` に入れる。違えば覚えずに今の Snapshot で投入し直す。他の項目は変えない) / キャンセル ★ → 中止 (最初の位置へ移動し、全部の位置を強調して次の編集まで残し、`info.save.escape_merge_cancelled` を出す) | このまま保存では保つ。キャンセルで捨てる |
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
| `JobError::VerifyFailed` (v0.4。保持方式の記録を使う保存の確かめに合わない、§8.6 E2。何も書いていない) | `msg.save.verify_failed` + `ui.confirm.save_failed` | 同上 (別名で保存 ★ / 退避先に保存 / 中止)。core は先に文書の記録を全部捨てて「保持なし」(`VerifyFailed`) にする (§8.7.1) ので、`file.save` で保存し直すと E2' の確認を経て記録を使わずに書く | 同上 |
| その他 (`InvalidPath`、`Other`、`JobError::Panic` 等) | `msg.save.invalid_path` / `msg.save.failed` + `ui.confirm.save_failed` | 同上 | 同上 |
| `JobError::Cancelled` | `info.save.aborted` | 選択肢なし → 中止 | 捨てる |

- **退避先に保存**: 文書の Snapshot を緊急退避と同じ形式 (§10.1: UTF-8・改行非変換・PUA エスケープのまま、manifest 付き。manifest の `reason = "user_save"`) で `state_dir/recovery/rescue/<UTC timestamp>-<pid>/` に書く (緊急退避のディレクトリとは別。05 §7.5)。書き手は `bg` のジョブ `JobKind::WriteRecovery { doc }` (02 §9.3。`DocSnapshot` を渡す。出力先のディレクトリと本文・manifest は `FileSystem::write_recovery` (02 §4.5) が、名前・`created_at`・代替の退避先を決めて作り、書く。I/O を伴うが `io` ではなく `bg` ワーカーで `FileSystem` Port を使う。シリアライズの関数は緊急退避・スワップと共用) で、結果は `JobResult::RecoveryWritten { path }`。`state_dir` を使えない (作れない・書けない) ときは緊急退避と同じ代替の場所の `rescue\` (Windows は `%TEMP%\eve-recovery-<SID>\rescue\<UTC timestamp>-<pid>\`、所有者のみの DACL。05 §7.5 / §11、02 §11.2.1 (OQ-163)) に書く。結果は文書の Revision が進んでいても、取消しの後でも、文書が閉じられていても採用する (02 §9.3 の採否表)。書けたら `info.save.to_recovery` で退避先を示し、書けなければ `msg.save.recovery_failed` (04 §15.2) で理由を示す (どちらでも文書は dirty のまま)。元のファイルは変えず、`saved` は動かさない。**後続の処理は中止する** (退避は保存の代わりではないので、終了・置き換え・開き直しをしない)。

**(f) 保存中の終了操作** (04 §15.2 `ui.confirm.quit_saving`・05 §11.5 と同じ方式): 保存ジョブの実行中 (`Saving`、または連鎖の外で `Document.saving` が Some) に `app.quit` を実行したら、次のとおりにする。

1. その文書の連鎖の `cont` を `Quit` にして `Saving` のまま待つ (連鎖の外の保存なら、`cont = Quit` の連鎖をその場で作る。置き換え・開き直しの `cont` は終了で置き換える。`cancel_origin = User` なら `None` に戻す、遷移表)。すぐには確認を出さない。`CheckingUnchanged` での `app.quit` は待たない (遷移表)。
2. 保存が **5 秒以内** に終われば (`app.quit` を受けた時点の Editor の「今」に `Millis(5000)` を足した期限を `next_deadline` に載せ、`tick` で判定する。02 §4.4 / §5.4) 確認を出さずに完了の規則に従う: 成功して dirty でなければ終了、保存中の編集で dirty なら `AskUnsaved`。失敗したら `AfterFailure` に進み、(e) の表で `cont = Quit` を保つ選択 (再試行・別名で保存等) が成功すれば終了する。退避先に保存・中止を選べば終了しない。
3. 5 秒を超えても終わらなければ `ui.confirm.quit_saving` (「{path} の保存が終わっていません — `[W] 待つ` ★ / `[Q] 保存を中止して終了`」) を出す。
   - **待つ** (既定。`Esc` も同じ): 確認を閉じ、`Saving` のまま 2 の規則で待ち続ける (`cancel_origin` は変えない。`Esc` は確認を閉じるだけで、保存の取消し (`cancel_origin = User`) にはならない。04 §7.5)。確認は自動では出し直さない。もう一度 `app.quit` を実行すると、この確認をすぐに出す。
   - **保存を中止して終了**: `cancel_origin = QuitAbort` にし、保存ジョブに取消しを **常に** 送る (ジョブは `ReplaceFileW` の開始以降・直接上書きの 05 §4.4 手順 6 以降なら取消しを無視して完了まで進む。main はその完了を待つ、05 §11.5)。ジョブが終わったら結果に依らず終了へ進む (遷移表の `cancel_origin = QuitAbort` の行): `Saved` なら採用し (§9.3 手順 7)、`Cancelled`・失敗 (`ReplacedMovedToBackup`・`InPlaceFailed` を含む) なら `AfterFailure` に進まない。その時点で緊急退避の対象の文書 (dirty な文書と、まだ名前を付けて保存していない標準入力の文書。§10.1 の「対象」) は緊急退避と同じ書き手・形式 (§10.1、02 §11.2.1) で退避してから終了する (core は `Effect::Quit { code, rescue: QuitRescue::DirtyDocuments, stderr }` (02 §6.3) を出し、退避は Host が `EmergencySnapshot` から行う。退避の結果は Host が終了時の stderr に `stderr.recovery.*` で出す)。元のファイルは保存前の内容 (または保存を終えた新しい内容) のまま。保存が失敗していたら、失敗のパスを端末の復元後の stderr に出す (04 §15.2): `ReplacedMovedToBackup { backup }` → `stderr.quit_abort.moved_to_backup` (`{path}` と退避パス `{backup}`)、`InPlaceFailed { backup: Some }` → `stderr.quit_abort.in_place_failed` (`{path}` とバックアップのパス `{backup}`)、`InPlaceFailed { backup: None }` → `stderr.quit_abort.in_place_failed_no_backup`、その他の失敗 → `stderr.quit_abort.save_failed` (`{path}`、`{reason}`)。`Cancelled` (何も書いていない) と `Saved` では出さない。**読込中の標準入力の文書** (v0.4 の M25、§9.2.2): 読込中の標準入力の文書があり、終了の連鎖で「破棄」の印が付いていなければ (印は Revision に依らず見る)、黙って捨てない。受け取った量によらず (選んだ時点の進捗は遅れることがあり、受け渡し待ちの分もあるため) その読込も止め (`StdinEnd::Stopped` と同じ扱い。切る位置も、受け渡し待ちの分を取り込むことも同じ)、保存ジョブの完了と一緒に `StdinDone` を待つ。読み終えた文書は、1 バイト以上読んでいれば変更あり (`--readonly` で変更なしの文書も、まだ名前を付けて保存していないので緊急退避の対象。v0.5 の M29、§10.1 の「対象」) なので、ほかの dirty な文書と同じく退避してから終了する (manifest は `sensitive = true`)。0 バイトなら空の文書で終わるだけ。「破棄」の印が付いた文書は待たずに捨てる。読込がパニック (`Failed`) で終わったら、読んだ内容は失ったまま終了へ進む。待つ間に標準入力の文書が閉じられた (待ちの対象から外れた) ときも、終われるかを見直す。これらの行は core が `StderrLine { key, args }` として `Effect::Quit.stderr` に入れ (文言には解決しない)、Host が eve-host の定数表から文言を解決して端末の復元の後に出す (02 §6.3 の「stderr の文言の解決」、07 §17.1)。
   - **M9a の実装**: 保存中の文書が 2 件以上でも、確認の `{path}` は先頭の 1 件だけを示す (「ほか N 件」は M9b で実装した)。「中止して終了」で保存が失敗した場合の stderr の行は、失敗した保存ごとに 1 行ずつ出す。
   - **確認の表示中に保存ジョブの結果が届いた** (`cancel_origin = None` のまま): 確認を閉じてから結果を遷移表の `Saving` (`cont = Quit`) の行のとおりに扱う。`Saved` → 完了の規則で終了へ (`cont = Quit`。保存中の編集で dirty なら `AskUnsaved`)。`Failed { Cancelled }` → 中止 (終了しない)。その他の `Failed` と、`Unmappable`・`EscapeMerge` (v0.4) の結果 → `quit_saving` の確認を `AfterFailure` の確認 ((e) の表) に **置き換える** (確認を積まない。`cont = Quit` を保つ)。

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
| 対象と順序 | dirty な全文書 (`Loading` 中を除く。例外: v0.4 の M25 で、1 バイト以上読んだ読込中の標準入力の文書は対象にし、確認は `ui.confirm.stdin_loading` (ここまでで開く / 破棄 / キャンセル)。「ここまでで開く」を選んだら、その文書の FR-107 の確認に続ける。§9.2.2 の「閉じる・終了」)。アクティブな文書を先頭に、残りは開いた順。連鎖の状態 (`QuitAll { remaining: Vec<DocumentId>, discarded: Vec<(DocumentId, Revision)> }`) は `SaveChain` の外に 1 つだけ持つ |
| 各文書の確認 | その文書をアクティブにして、FR-107 の 3 択 (保存 / 破棄 / キャンセル) を出す。文言は何件目か・残り何件かを含む (`ui.confirm.unsaved` の `{index}` / `{total}`、04 §15.2 に足す)。標準入力を途中まで読んだ文書 (`LoadIncomplete`、v0.4) は、文書名の後ろに「途中まで ({size})」を出す (§9.2.2) |
| 保存 | 文書ごとの `SaveChain` (§9.7)。成功したら次の文書へ。失敗・キャンセルしたら連鎖全体を中止し、終了しない |
| 破棄 | 不変条件 2 のとおり印だけ付けて次へ。印は (文書, その時点の Revision) の組で、後で Revision が進んだ文書の印は無効 (もう一度尋ねる)。v0.4 の M25: 読込中の標準入力の文書に付けた印も同じで、連鎖の途中で読み終わると Revision が進むので、読み終えた文書を FR-107 でもう一度尋ねる (安全側)。ただし「保存を中止して終了」(§9.7 (f) 3) で読込を待つかの判定では、印を Revision に依らず見る (印の付いた読込は待たずに捨てる) |
| キャンセル | 連鎖全体を中止し、印をすべて無効にする。保存済みの文書はそのまま保存済み |
| 全部済んだら | `Effect::Quit { rescue: None }`。破棄の印が付いた文書だけが未保存のまま残るが、利用者が捨てると答えたものである |
| 保存中の `app.quit` | `ui.confirm.quit_saving` (§9.7 (f)) は、保存中の文書が 2 件以上でも 1 つのダイアログにまとめる (`{path}` は先頭の 1 件に「ほか N 件」を足す) |
| 外部終了 (コンソールクローズ等) | 連鎖を待たず、dirty な全文書を緊急退避する (§10.1) |
| `app.quit_with_error` / `app.save_and_quit` (v0.3) | 同じ連鎖を使う (07 §18.4 / §18.6)。違いは終了コードだけで、`app.quit_with_error` は全部済んだら (と、保存中の「保存を中止して終了」でも) `Effect::Quit { code: 4 }`。`app.save_and_quit` は表示中の文書の `SaveChain` (`cont = Quit`) から始め、保存に成功したらこの連鎖に入る (表示中の文書が dirty でなければ、そのまま連鎖に入る)。終了コードは要求のたびに決め直し、キャンセルした要求のものを次の終了に持ち越さない |

**バッファ操作** (確定。FR-103、07 §11.6 の `buffer.*`):

| 項目 | 規則 |
|---|---|
| 同じパスをもう一度開く | 新しく開かず、既存のバッファに切り替える (`FsPath` と `file_id` で同一判定、§9.2 の二重オープン判定)。読込中なら読込の完了を待つ。v0.4 までの実装はパスの文字列だけで比べていた。v0.5 の M31 で、開くジョブの `Identity` の後に `file_id` (と canonical のパス) でも比べるようにした (SAF-28、§12.7.6。別の書き方のパスの文書は、識別が届いた時点で閉じて既存の文書を出す) |
| `buffer.next` / `buffer.prev` | 開いた順で循環する (一覧の表示順とは別。一覧は最近使った順、04 §8.5) |
| `file.open` / `file.new` (実装済み、M9a の残し) | **今の文書を置き換えず、バッファを足してアクティブにする**。何も捨てないので dirty の確認 (`unsaved_replace`) は出さない。`file.open` は、新しい `Loading` のバッファを足し、読込の間は今の文書を表示し続けて編集もできる (v0.1 の「読込の間は旧文書が読み取り専用」は置き換えがなくなるので不要)。読込が終わったとき、**その間に利用者が別の文書へ切り替えていなければ** 新しいバッファへ切り替える (切り替えていたら、一覧に載るだけで表示は変えない)。読込の失敗・取り消し・確認での中止は、足したバッファを閉じ、今の文書は変えない。読込中にその足したバッファへ切り替えて表示していたときは、開く前の文書 (閉じられていればほかの文書、それも無ければ新しい無題の文書) を表示してから閉じる (表示を移さずに閉じると Window が消えた View を指したままになる。M10 の `fuzz_editor` で見つけて直した)。読込中に重ねて `file.open` してもよい (読込ジョブは直列、上の「読込中のバッファ」)。`file.new` は無題のバッファを足して、すぐ切り替える。**例外 (何も失わない置き換え)**: 表示中のバッファが、何も入力していない無題のバッファ (パスなし・未編集・空・読込済み) で、読込の完了までそのままなら、`file.open` は開いたファイルでそれを置き換える (起動直後の `Ctrl+O` で空の無題が残らないため。一覧の番号も増えない)。同じパスは上の「同じパスをもう一度開く」のとおり既存のバッファへ切り替える。`Continuation::Replace` / `unsaved_replace` は v0.2 で使わなくなる (開き直し `Reopen` は残る)。**v0.4 (M25) の先行表示** (§9.2.1 の「表示の切り替え」): 先行表示 (`Partial`) を受けた時点で、この行の「読込が終わったとき」と同じ条件で切り替える。オーバーレイが開いている間は保留し、閉じた後に切り替える (完了で切り替えるときも同じ)。何も入力していない無題のバッファからの切り替えでは、無題のバッファを先行表示の時点では閉じず、完了の時点で閉じる (失敗・取消しでは表示に戻す。番号は増えない)。先行表示で切り替えた後の完了では、切り替え直さない。オーバーレイ・検索バーの間の保留は先行表示と完了の切り替え・失敗の戻し・無題を閉じることのすべてに掛かる (§9.2.1)。**切り替え元の書き換え** (v0.4 の M25): 自動の切り替え (先行表示・完了) で表示を X から Y へ移したら、ほかの待っている読込のうち切り替え元が X のものを、切り替え元 Y に書き換える (利用者は Y を見ているので、続けて開いた読込は Y を表示している限り切り替える。X に戻した後に、見ていない Y を基準にした読込が表示を奪わない)。読込の失敗・取消しで開く前の文書へ表示を戻したときは書き換えない (戻した先を表示していても、ほかの読込は表示を奪わない側に倒す)。「何も入力していない無題のバッファ」には、その文書を待つジョブ (保存・スワップ・復旧の読込など) が無いことも含める |
| 閉じた後に表示する文書 | 最近使った順で次の文書。最後の 1 件を閉じたら、無題の文書を新しく作る (終了はしない)。v0.5 の複数ペインでは、閉じた文書を表示していたペインごとに、そのペインの最近使った順で選ぶ (§9.9 の PANE-9) |
| 無題の採番 | 起動ごとの通し番号 (`無題-1`、`無題-2` …)。無題の文書を作るたびに振る (起動時の無題・`file.new`・最後の 1 件を閉じた後の無題を含む)。閉じても番号を再利用しない。名前を付けて保存した文書の番号は使わなくなるが、戻さない (次の無題は続きの番号)。緊急退避の manifest とスワップの `untitled_name`、バッファ一覧・ステータスバー・タイトルもこの番号を使う。スワップから復旧した無題 (§10.3 の 5。空の無題に適用する) は、適用先の文書の番号のまま (前のセッションの `untitled_name` は復旧の提案の表示にだけ使う) |
| 読込中のバッファ | 一覧に載せる (読込中と分かる印: 一覧の右の欄の文字コードの代わりに `ui.load_state.loading` (「読込中」) を出す、04 §8.5。先行表示 (`Partial`) の文書も同じ。本文の無い `Loading` の文書のうち、まだ表示していない `file.open` の読込は一覧に出さない)。切替はできるが、編集はできない (§9.2)。読込ジョブは直列に実行する (複数パスで起動したときは先頭から順に)。標準入力 (v0.4、§9.2.2) の読込は専用のスレッドで、ファイルの読込と並んで進む。**`Esc` (`ui.cancel`) で取り消す読込** (v0.4 の M25 で明記。04 §7.5): ステータスバーが読込中として示している読込だけ。表示中の文書が読込中 (`Loading` / `Partial`。開く・再読込・開き直しのどれでも) ならその文書の読込だけを取り消し、表示中の文書が読込中でなければ、`file.open` で足した表示していない文書の読込 (ステータスバーが進捗を示すもの。v0.2 からの振る舞い) を取り消す。ほかの文書の再読込・開き直しは取り消さない。標準入力の読込は、その文書を表示しているときだけ確認の後にやめる (§9.2.2 の「取消し」。ファイルの読込の取消しの対象に含めない)。コードは `io/file.rs` の `cancel_own_loading` (表示中の文書) と `cancel_hidden_loading` (表示していない文書) (M25 の (2) で揃えた。それまでは進行中の読込をすべて取り消していた。v0.5 の M31 で 2 つに分けた)。**取り消す順** (v0.5 の M31、§12.7.6 の細目 (9)): 表示中の文書の読込 (読込中の標準入力の文書の確認を含む) → 名前を付けて保存の照合 (§9.7 の `CheckTarget`) → 表示していない文書の読込。取り消し済みで結果を待つだけの読込は数えず、次の `Esc` を次の段へ通す |
| 検索バー・オーバーレイを開いたままの切替 | 切替の前に検索バーと進行中の検索を閉じ、検索状態は文書ごとには持たない (単一の検索状態を切替で破棄する) |
| 入力・確認の対象 | 文書を書く・変える入力と確認 (名前を付けて保存・文字コードを指定して保存・開き直すの入力、未保存・再読込・保存の失敗などの確認) は、**開いた時点の文書** を対象に持ち、確定ではその文書に効かせる (読込の完了などで表示が別の文書へ切り替わっても、表示中の文書を書かない・変えない)。確定の時点で対象の文書が閉じられていたら、何もせずに入力を閉じて `info.prompt.target_closed` で知らせる。行へ移動は内容を変えないので、確定の時点の表示中の文書に効く |
| 表示されていない文書のジョブの失敗・完了 | 保存の失敗はファイル名つきのメッセージで知らせる。読込の完了は一覧の印だけ変える |
| 文書の数 | 数の上限は置かず、メモリの予算 (NFR-203) で制御する |
| ステータスバー・タイトル | 文書が 2 件以上のとき、ファイル名の後ろに `(2/3)` を出す |
| 起動時の複数パス (FR-101) | `--readonly` と `--encoding` は全パスに掛かる。`+行` は直後のパスに掛かり、`path:行:桁` はそのパスに掛かる。最初のパスをアクティブにする |

**全保存** (`file.save_all`、FR-104 AC-2。確定 (ユーザー回答)、2026-10-29):

| 項目 | 規則 |
|---|---|
| 対象 | dirty で、パスがあり、読み取り専用でなく、保存ジョブが実行中でない文書。**名前の無い (無題の) dirty な文書と読み取り専用の dirty な文書は飛ばし**、名前の入力は求めない。飛ばした件数を完了のメッセージで知らせる (利用者が個別に保存する) |
| 実行 | 開いた順に、文書ごとの保存 (§9.3) を `cont` なしで始める。保存は文書ごとのジョブで、1 件の失敗で他の文書を止めない。表示中の文書は変えない |
| 対話しない | 全保存の保存は **確認を出さない**。保存の前後に確認が要るもの (§9.7 (e) の `AfterFailure` の各確認、区分 B・表現不能の確認、不正バイトのつながりの確認 (v0.4 の `EscapeMerge`、§8.3.1。理由は `needs_confirm`)、外部変更の `Conflict` の確認) は、その文書を **保存せず失敗として記録し**、理由を添えて一覧に載せる (個別に `file.save` で実行し直すと、通常の確認が出る)。複数の確認が同時に出るのを避けるため |
| 完了 | 全部のジョブが終わったら 1 回だけ知らせる。失敗が無ければ `info.save_all.done` (保存した件数と飛ばした件数)。失敗が 1 件以上あれば、失敗した文書を **一覧にした確認** (`ui.confirm.save_all_failed`。本文は失敗の件数、`details` に 1 文書 1 行で `{path}: {reason}`、選択肢は閉じるだけ。`Esc` も同じ) を出す。保存できた件数は `info.save_all.done` で別に知らせる |
| 保存中の編集 | 保存の間に編集された文書は、保存の完了の後も dirty のまま残る (§9.7 の完了の規則と同じ。全保存の再実行で保存できる) |
| 保存中の `app.quit` | 全保存の途中の終了は、通常の保存中の終了 ((f)) と同じ扱いを、保存中の文書ごとに行う |

**メモリ** (NFR-203): プロセス全体の Undo 履歴の合計が上限 (`editor.undo.total_max_bytes`、既定 256 MiB) を超えたら、履歴が最も大きい文書の古い順に破棄して通知する (01 FR-204 AC-4: 破棄しても dirty は保つ)。文書ごとの予算 (`UndoBudget`) の合計を出す API と、`after_change` で最大の文書を剪定する処理を足す。

### 9.9 複数ペインと分割 (v0.5 の M30、FR-509 / FR-510)

v0.2 の複数文書 (§9.8) は、1 つの Window に View を 1 つだけ出す前提 (「1 Window = 1 View」) で作ってある。本節は、Window の編集面を **ペイン** に分け、文書・ペイン・View の関係と、ペインを作る・移る・閉じる規則を定める (M30 の仕様の PR、2026-11-21)。境界の型は 02 §8.2、描画・マウス・ロールは 04 §3 / §6.8 / §9 / §11 と 07 §8.8、コマンドと既定キーは 07 §11.6、試験は 09 §3.4、受入基準は 01 §5.5 の FR-509 / FR-510。点検の材料は docs/review/v0.5-plan.md §3.1・§3.2 (FND-01〜04、SPL-01〜08、TAB-01〜05)。

**用語**

| 用語 | 意味 |
|---|---|
| Window | frontend の画面 1 つ (v0.x は 1 つ)。レイアウトの木と、フォーカスのあるペインを持つ |
| ペイン | レイアウトの木の葉。編集面の矩形を 1 つ持ち、そこに View を 1 つ表示する。ID (`PaneId`) は core の内部の型で、境界の型には出さない (境界では、表示中の `ViewId` でペインを指す。PANE-7) |
| View | 文書の表示の状態 (選択・スクロール・上下の移動の目標の列・折り返しの設定)。**どの View も高々 1 つのペインに属する** (文書を作ったときの View は、表示するまでどのペインにも属さない。PANE-1) |
| 分割 | 子を並べる向き (`Row` = 左右に並べる、`Column` = 上下に並べる) と、子ごとの比率を持つ木の節 |
| 見出し | ペインが 2 つ以上のときに各ペインの最下行に出す 1 行 (文書名と印。フォーカスのペインは強調する)。v0.4 までの画面には無い |

| ID | 規則 |
|---|---|
| PANE-1 | **文書・ペイン・View の関係**: 各ペインは「文書 → View」の表 (`views`) を持ち、ある文書を初めて表示するときにそのペインの View を作る。同じ文書を 2 つのペインで表示すると、View は 2 つ (ペインごとに 1 つ) になる。ペインで別の文書に切り替えて戻ると、表の View をそのまま使うので、そのペインでの位置 (選択・スクロール) が保たれる (FR-103 AC-2 をペインごとに)。**ペインに属さない View**: 今と同じく、文書を作ると View を 1 つ作る (`add_document`。裏で読む `file.open`・起動時の複数パス・標準入力・読込のジョブの `spawn_open` もこの View を使う)。この View は、最初にその文書を表示したペインの表に **そのまま移す** (ViewId は変わらない。1 ペインでは今と同じ ID になる)。ペインがある文書を初めて表示するとき、その文書にペインに属さない View があればそれを移し、無ければ、その文書の View のうち **最後にフォーカスのあったもの** (どのペインのものでも、ペインに属さないものでもよい) の選択・スクロール・上下の移動の目標の列・折り返しの設定を写して新しい View を作る (写して作った View と、ペインに属さない View を移したときは、表示するペインの本文の矩形でカーソルが見えるように最小限スクロールする (写し元と表示するペインで本文の桁数 (折り返し幅) が違えば、折り返しの表示行の番号を 0 に戻してから)。写し元のペイン・それまで大きさを求めたペインと大きさが違うことがあるため。ペインの表から出した View はスクロールを変えずにそのまま使う。表の View の大きさは、木を変えた後と端末の大きさが変わった後に合わせてある (PANE-5 (f) の後、PANE-13))。**どの View も高々 1 つのペインの表にあり、開いている文書にはどれも View が 1 つ以上ある**。文書を閉じると、その文書の View を全部 (全ペインの表とペインに属さないもの) 消す。ペインを閉じると、そのペインの表の View を消すが、その文書のほかの View が無ければ、その View をペインに属さない View として残す (そのペインでだけ見ていた文書の位置を失わない。v0.4 の「文書ごとに位置が残る」を保つ)。ペインに属さない View も `after_change` の写像を受ける |
| PANE-2 | **共有するもの・ペインごとのもの**: 本文・Revision・Undo / Redo の履歴・保存点 (dirty)・文字コードと改行・読み取り専用の理由・読込の状態は **文書に 1 つ** で、全ペインで共有する。選択 (カーソル)・スクロール・上下の移動の目標の列・折り返しの位置の帰属・折り返しの ON / OFF (`view.toggle_wrap`) は **View ごと** (ペインごと)。片方のペインでの編集は文書に 1 回適用され、ほかのペインの View の選択とスクロールは ChangeSet で写像される (§6.5 の手順 7、02 §5.2)。Undo は文書単位で、どのペインから戻しても同じ本文になる。選択の扱いは §7.6 を次のように読む: 発行 View が **どれかのペインに表示中** ならその選択を戻す。表示中でなければ (別の文書に切り替えたペインの表に残っているだけ・ペインに属さない) 戻さず写像だけにする。Undo を実行したペインの View が発行 View でないときは、選択は写像だけで、戻した変更の先頭が見えるようにスクロールだけする。検索バーと検索の状態は Window に 1 つ (PANE-6) |
| PANE-3 | **レイアウトの木**: 下の型。分割の子は 2 つ以上 (1 つになった分割は親に畳む)。同じ向きの分割を親子にしない (子が同じ向きの分割なら、孫を親に並べ直す)。比率は分割ごとに合計 10,000 の正の整数。**矩形の計算は core が行う** (02 §8.3): 編集面 (`Frame.size`) を根に与え、`Row` は子の間に区切りを 1 列ずつ置いて残りの列を、`Column` は行を、比率で配る (`Column` は区切りを置かない。上の子の見出しが区切りになる)。配り方: 各子に `floor(残り × 比率 / 10,000)` を配り、余りを小数部の大きい子から 1 つずつ (同じなら前の子) 配る。最小 (PANE-4) を割る子があれば、その子を最小にし、足りない分をほかの子から比率で引き直す (子の数の回数で止まる) |
| PANE-4 | **最小の大きさと、端末が縮んだとき**: ペインの最小は、本文 (ガターを含む) が **20 列 × 3 行** と見出しの 1 行 (定数。設定にしない。04 §3.5 の編集面の最小幅 20 列に揃える)。木の最小は、葉が (20, 4)、`Row` が列は子の和 + 区切りの数・行は子の最大、`Column` が列は子の最大・行は子の和 (葉が 1 つの木は見出しを出さないので、下の (b) の縮退表示は無い。端末の最小は (c))。(a) **分割の可否**: 分割した後の木の最小が今の編集面に入らないときは分割せず、`info.pane.no_room` (「ペインを分ける広さがありません」。対処「端末を広げるか、ほかのペインを閉じてください」) を出す。(b) **縮退表示**: 編集面が木の最小より小さくなったら、木はそのまま保ち、**フォーカスのペインだけ** を編集面の全体に、見出しと区切りなしで出す (1 ペインと同じ見た目)。縮退表示に入ったときに 1 回 `info.pane.collapsed` (「端末が狭いので、フォーカスのペインだけを表示しています」) を出し、縮退表示の間はステータスバーに機能縮退の項目 (`StatusKind::Degraded`、優先度 5) を `status.degraded.panes.long` / `.short` (`{index}` / `{total}`。「[ペイン 2/3 だけ表示]」/「2/3」) で出し続ける (ほかのペインが隠れていることに後から気付けるように)。縮退表示の間も、ペインの操作 (移動・閉じる・大きさ) は木に対して働き、フォーカスを移すと表示するペインが替わる。方向の移動と大きさ (PANE-5 (c) / (e)) は、木の最小の大きさで計算した配置で決める (どのペインも最小なので、大きさは変わらない。揃えるは比率に働く)。分割は、分割した後の木の最小が入らないので (a) で断る。編集面が木の最小以上に戻ったら元の配置に戻し、各ペインのカーソルが見えるようにスクロールする。(c) 編集面が frontend の最小 (`Capabilities.min_surface`) より小さいときの表示 (`ChromeTexts.too_small`) は今と同じ |
| PANE-5 | **ペインの操作** (ID と既定キーは 07 §11.6): (a) **分割** (`pane.split_right` / `pane.split_down`): フォーカスのペイン P の右 / 下に新しいペイン Q を作る。P の親が同じ向き (`Row` / `Column`) の分割なら Q を P の直後の兄弟にし、P の比率を P と Q で半分ずつにする (P が切り上げ。丸めで 0 になる子があれば 1 にし、合計の過不足を最も大きい子で吸収する。比率の計算し直し (e) と畳むとき (d) も同じ)。そうでなければ P を、P と Q を子に持つ新しい分割 (比率 5,000 / 5,000) で置き換える。Q は P と同じ文書を、P の View を写した新しい View で出す (PANE-1)。Q の最近使った順 (`recent`) は P のものを写す。フォーカスは Q に移る。PANE-4 (a) に当たれば何もしない。(b) **次 / 前のペイン** (`pane.focus_next` / `pane.focus_prev`): 木の葉の順 (深さ優先。`Row` は左から、`Column` は上から) で次 / 前へ。端から反対の端へ巡る。(c) **方向の移動** (`pane.focus_left` / `pane.focus_right` / `pane.focus_up` / `pane.focus_down`): その向きで隣り合う (区切りか見出しを挟んで接し、接する辺の範囲がフォーカスのペインの辺と重なる。範囲は本文と見出しを合わせた矩形で見る) ペインのうち、フォーカスのペインの主カーソルの画面の行 (左右のとき)・列 (上下のとき) を含むもの (ホイールなどで主カーソルが画面の外にあるときは、その行・列をペインの本文の範囲に寄せた値を使う)。含むものが無ければその行・列に最も近いもの (同じ距離なら上・左のもの)。隣り合うペインが無ければ何もしない (巡らない)。(d) **ペインを閉じる** (`pane.close`): フォーカスのペインを閉じ、その View を全部消す。**文書は閉じない** (dirty でも確認を出さない。どのペインにも表示されていない文書は、バッファ一覧・`buffer.next` から表示できる)。空いた場所は前の兄弟 (先頭なら次の兄弟) が比率を足して受け取り、子が 1 つになった分割は親に畳む (畳んで同じ向きの分割が親子になったら、孫を親に並べ直し、孫の比率を「子の比率 × 孫の比率 / 10,000」(切り捨て) にして余りを最後の孫に足す)。フォーカスは、残ったペインのうち最後にフォーカスのあったもの (`pane_recent`) に移る。**最後のペインは閉じない** (`when` が `pane.split`。パレットでは淡色で `ui.palette.reason.single_pane` 「ペインが 1 つだけです」を出す。キーで実行して `pane.split` のために断ったときも、「割り当てられていません」ではなく同じ文言を `info.pane.single_pane` で出す (移動・大きさのコマンドも同じ))。(e) **大きさ** (`pane.grow` / `pane.shrink`): フォーカスのペインを、親の分割の向き (`Row` なら列、`Column` なら行) に `cells` (既定: 親の矩形のその向きの長さの 1/10、少なくとも 1) だけ広げる / 狭める。差は次の兄弟 (末尾なら前の兄弟) から取る / に返し、どちらのペインも最小を割らない所で止める。比率はセルの大きさから計算し直す (各子を `floor(セル × 10,000 / 全体のセル)` にし、余りを最後の子に足して合計を 10,000 に保つ)。(f) **揃える** (`pane.equalize`): 全部の分割で子の比率を等しくする (10,000 を子の数で割り、余りを前の子から 1 ずつ)。(a) / (d) / (e) / (f) で木を変えた後は、本文の大きさが変わったペイン (と新しく出たペイン) の表の View (表示していない View を含む。次にそのペインで表示するときの大きさが変わる) を、端末の大きさが変わったとき (`apply_viewport`) と同じ規則で合わせる (本文の桁数 (折り返し幅) が変わった View だけ折り返しの表示行の番号を 0 に戻し、カーソルが見えるように最小限スクロールする) |
| PANE-6 | **フォーカスとオーバーレイ・検索バー** (M31 のファインダ・横断検索の一覧を含む一般の規則): (a) フォーカスのペインが変わると、検索バーと進行中の検索を閉じ (§9.8 の文書の切替と同じ。検索の状態はペインごとに持たない)、前のペインの文書の Undo の結合を区切り (§7.3 の封印。§7.3 の封印の契機には FR-509 の PR でこの契機を足す)、`pane_recent` と「最後にフォーカスのあった View」(PANE-1) を記録する。(b) オーバーレイ (パレット・プロンプト・確認・一覧・ヘルプ・ファインダ・横断検索の一覧) と検索バーは **Window に 1 つ** で、画面の全体に対して出す (ペインの中には出さない。位置は frontend が決める、04 §3.6)。働く対象は **開いた時点のフォーカスのペイン** (その View と文書)。(c) オーバーレイ・検索バーが開いている間は、キーはそちらに届き、キーとマウスではペインのフォーカスは変わらない (例外は (e) のパレット経由)。編集面のマウス (どのペインでも)・見出しのクリック・タブのクリックは無視する (§5.8.1 の「オーバーレイ・検索バーが開いている間」と同じ)。(d) オーバーレイから実行したコマンド (パレットから `pane.*` を選ぶなど) は、オーバーレイを閉じてからフォーカスのペインに対して実行する。オーバーレイから文書を開く・表示する操作 (バッファ一覧・ファインダ・横断検索の結果へのジャンプ) は、フォーカスのペインに表示する (PANE-8)。(e) 検索バーのコンテキスト (`search`) に `pane.*` の割当は無い。パレット経由などで検索バーを開いたままフォーカスが替わったときは (a) のとおり閉じる。(f) パネル (02 §8.1。v0.5 には利用者が無い) は編集面の外に置き、レイアウトの木はパネルを除いた編集面に置く (04 §3.4 (h)) |
| PANE-7 | **マウス** (§5.8.1 への追加。TUI のヒット判定は 04 §6.8): (a) フォーカスの無いペインの編集面で左 `Down` → そのペインにフォーカスを移し (PANE-6 (a))、続けて今と同じクリックの処理 (カーソルの移動・`Shift` の拡張・ドラッグの起点) をそのペインの View で行う。(b) ドラッグと `Up` は、押したペインの View のまま (面の外の座標も、そのペインの `rect` の左上を原点に渡す。今と同じ)。(c) ホイールは **ポインタの下のペイン** をスクロールし、フォーカスは移さない。(d) 見出しの左 `Down` (`ChromeTarget::PaneBar`) → そのペインにフォーカスを移す (カーソルは動かさない。修飾キーを問わず左ボタンの押下で反応する。M30 の段 2b)。(e) タブの左 `Down` (`ChromeTarget::Tab`、FR-510) → その文書をフォーカスのペインに表示する。(f) 区切りの上の操作・中と右のボタン・見出しとタブのホイールは、v0.5 では何もしない (区切りのドラッグでの大きさの変更は後の版。そのときに `ChromeTarget` に変種を足す)。(g) **境界でのペインの指し方**: マウスの入力 (`MouseAt.view`・`ChromeTarget::PaneBar { view }`) と Frame (`PaneFrame.view`・`FocusTarget::Pane(view)`) は、ペインを **表示中の `ViewId`** で指す。View は高々 1 つのペインに属するので、core は View からペインを引ける (どのペインにも属さない View を指す入力は捨てる)。Frame を作った後にペインが別の文書へ切り替わったとき: 見出しの入力 (`PaneBar`) は、古い View がそのペインの表に残っていれば同じペインを指す (フォーカスを移すだけなので)。本文の入力 (`Mouse`) は、指す View がそのペインの **今の表示中の View でなければ捨てる** (座標を見えていない文書の位置として解かない)。消えた View を指す入力は、どちらも捨てる |
| PANE-8 | **文書の表示とペイン** (言葉: **アクティブな文書** = フォーカスのペインの文書、**表示中の文書** = どれかのペインに出ている文書。03 のほかの節の「アクティブな文書」「表示しているとき」(§9.2.2 の標準入力の取消し・§9.5 のフォーカスの復帰での確認・§9.8 の終了の順など) と 04 §7.5 はこの意味で読む): (a) 文書を表示する操作 (`buffer.next` / `buffer.prev`・バッファ一覧の選択・`file.new`・`file.open`・ファインダ (M31)・横断検索の結果へのジャンプ (M31)・終了の連鎖で「その文書をアクティブにする」(§9.8)) は **フォーカスのペイン** に表示し、ほかのペインの表示は変えない。表示を替える前のペインの文書を、そのペインの `recent` と Window 全体の `recent` (今の `DocSet.recent`) に記録する。起動時の複数パスは 1 ペインのまま (§9.8 の「起動時の複数パス」)。(b) **非同期の読込の完了・先行表示での切り替え** (§9.8 の `file.open`、§9.2.1): 「その間に切り替えていなければ切り替える」の判定を、**開いた時点のフォーカスのペイン** について行う。そのペインが残っていて、開いた時点と同じ文書を表示していれば、そのペインを新しい文書に切り替える。ペインが閉じられた・別の文書に切り替えた後なら切り替えない (一覧に載るだけ)。切り替えたペインがフォーカスの無いペインでも、フォーカスは移さない。読込の失敗・取消しで開く前の文書に戻すのも、そのペインだけ (開く前の文書が閉じられていれば、開いた順で最初の文書。ほかに読込中の文書を表示しているペインは、閉じる文書と同じ PANE-9 の順で次の文書を出す)。読込のジョブ (`Pending::Open` と切り替えの保留) は、切り替え元を「開いた時点のフォーカスのペインと、そこに出ていた文書」の組で持つ (M30 の段 2a)。§9.8 の「切り替え元の書き換え」は同じペインを待つ読込の間で行う。(c) **何も入力していない無題のバッファの置き換え** (§9.8 の例外): その無題の文書を **フォーカスのペインだけ** が表示しているときに限る。ほかのペインにも表示されていれば、置き換えずにバッファを足す。(d) 入力と確認の対象は、今と同じく **開いた時点の文書** で、ペインに結び付けない (§9.8 の「入力・確認の対象」)。(e) 外部変更・再読込・開き直し・文字コードの変更は文書に対する操作なので、その文書の全 View (全ペイン) に同じ規則で効く (開き直し (§8.8) は、実行したペインの View だけでなく、文書の全 View (表に残っている View・ペインに属さない View を含む) の選択とスクロールを先頭に戻す。本文を差し替える経路 (読込の完了・開き直し・先行表示の差し替え・標準入力の完了) はどれも、差し替えた後に文書の全 View の選択とスクロールを新しい本文の範囲と書記素の境界に収める (位置を決め直す前の安全網。M30 の段 2a)) |
| PANE-9 | **`buffer.close` と `pane.close`**: `buffer.close` (バッファ一覧の中の `buffer.close` も同じ) は **文書を閉じる**。閉じる前の確認 (FR-107、§9.2 の閉じる手順) は今と同じで文書ごとに 1 回。閉じたら、その文書を **表示していた全ペイン** が、それぞれ次の文書を表示する: (1) そのペインの `recent` を新しい順にたどって、開いている文書があればそれ、(2) 無ければ Window 全体の `recent` で同じく、(3) それも無ければ開いた順で最初の文書、(4) 文書が 1 つも残っていなければ、新しい無題の文書を 1 つ作り (§9.8 の「閉じた後に表示する文書」)、それを表示していたすべてのペインに表示する (各ペインに View を作る)。全ペインの表と `recent` からその文書を消す。**ペインは閉じない**。文書を閉じるどの経路 (読込の失敗・取消し、無題のバッファの置き換え、標準入力の取消しなど) も、閉じる前に表示している全ペインをこの順で替える (ペインが消えた View を指したままにしない)。`pane.close` は **ペインを閉じ、文書を閉じない** (PANE-5 (d))。違いは各コマンドの説明 (`cmd.buffer.close` / `cmd.pane.close` の description) とヘルプに書き、`cmd.buffer.close` の title を「閉じる」から「文書を閉じる」(en: Close Document) に改める (パレットで「ペインを閉じる」と並ぶため。FR-509 の PR の段 2b。既存の TUI のスナップショット (ヘルプ・パレット) が変わるので、ゴールデンを更新する段 2b に回した) |
| PANE-10 | **Frame の組み立て** (型は 02 §8.2。言葉: **編集面** = Window の本文の領域の全体 (`Frame.size`。タブライン・検索バー・クロームを除く)、**ペインの本文** = 1 つのペインのガター + テキスト (`PaneFrame.rect`)。`MouseAt` の原点はペインの本文の左上、`PaneFrame.rect`・見出し・区切り・`primary_cursor` の原点は編集面の左上): (a) `Frame.panes` はペインを木の葉の順に並べる (縮退表示ではフォーカスのペインだけ)。`PaneFrame.rect` は見出しを除いた本文 (ガター + テキスト) の矩形で、編集面の左上が原点。`PaneFrame.cursors` は `rect` の左上が原点。`Frame.primary_cursor` は **編集面の左上が原点** で、フォーカスのペインの `rect` の位置を足す (v0.4 までは `rect` が (0, 0) で同じ値だったので、足し忘れに注意する。v0.5-plan FND-04)。(b) `PaneFrame.bar` は、ペインが 2 つ以上で縮退表示でないときだけ `Some`。`PaneBarModel { rect, items }` の `items` はステータスバーと同じ `StatusItem` で、`Path` (PANE-11 の名前。`(2/3)` の番号は付けない)・`Dirty`・`ReadOnly` を、ステータスバーの左群と同じ文言・優先度で入れる (04 §8.7)。frontend はステータスバーと同じ規則で省略し、**Window のフォーカスのペイン** (`PaneFrame.active`、v0.5 で足す。オーバーレイ・検索バーを開いていても真のまま) の見出しを強調する。今の `PaneFrame.focused` は「入力の行き先がこの面」(オーバーレイ・検索バーの間は偽) の意味のまま変えない (C2 で意味は変えられない、02 §8.4)。(c) `Frame.dividers` は `Row` の区切り (1 列。高さは親の分割の矩形の全体で、見出しの行を含む)。(d) フォーカスの無いペイン (`active` が偽) は、現在行の強調 (`ui.cursor_line`) と現在行の行番号の太字 (`ui.line_number.active`) を付けない (フォーカスの手がかりを見出しのほかにも持つ)。カーソルは `PaneFrame.cursors` に入れ、frontend が反転のセルで描く (04 §9)。(e) 生成の費用はペインごとの可視範囲の和で、文書の大きさに依らない (02 §8.2)。(f) ステータスバー・端末のタイトル・`Query::Status` は、フォーカスのペインの文書を示す (今と同じ) |
| PANE-11 | **同じ名前の区別** (見出しとタブラインの名前。数える文書の集合は PANE-12 (j) の (1)): 開いている文書のうち、ファイル名 (パスの最後の要素) が同じものが 2 つ以上あるとき、その組の各文書の名前を `名前 (親)` にする。`親` は親のディレクトリのパスの末尾から k 個の要素 (区切りは OS の区切り)。k は、その組の全部が区別できる最小の数 (要素が足りない文書は、親のパスの全体)。無題の文書は通し番号で区別できるので付けない。ステータスバー・タイトル・バッファ一覧の名前は今のまま |
| PANE-12 | **タブライン** (FR-510。実装は M32。型の置き場所と形を本節で決める): (a) 位置は画面の **最上行**、画面の全幅 (パネルの上。04 §3.4 (h))。出す・出さないは `ui.tab_line` (07 §5。`auto` は文書が 2 つ以上のとき)。(b) 行は **core が** 検索バーと同じく編集面の行から引く (`InputEvent::Resize` の行数はタブラインの行を含み、`Frame.size` は引いた後)。出し入れで frontend からの `Resize` は要らない。出し入れで編集面の行数が変わったら、各ペインの選択は変えず、カーソルが隠れるときだけ最小限スクロールする (折り返しの表示行 `wrap_idx` は、その View を表示するペインの本文の桁数 (折り返し幅) が変わったときだけ 0 に戻す (§14.3.1 の「リサイズ・設定変更」)。タブラインと検索バーの出し入れでは、ふつうは桁数が変わらないので戻さず、先頭の表示行を保つ。出し入れで縮退表示 (PANE-4 (b)) に出入りしてペインの幅が変わったときは戻す。FR-510 AC-2)。タブラインを出すと編集面が 3 行を割るときは出さない (frontend の最小 `min_surface` に近い端末)。(c) 内容 (`TabLineModel`) は開いた順 (`buffer.next` と同じ順) の全文書で、フォーカスのペインの文書をアクティブにする。各タブの項目は見出しと同じ `StatusItem` の並び (`Path` = PANE-11 の名前・`Dirty`・`ReadOnly`) で、frontend はステータスバーと同じ規則で縮める。どの範囲を出すか (アクティブを含む範囲) と省いた側の `‹` `›` は frontend が決める。(d) タブのクリックは PANE-7 (e)。**M32 の仕様の PR (2026-11-22) で足した細目**: (e) **タブの並べ方** (frontend。04 §3.4 (f) / §3.5): 各タブは `[`・印の欄 1 セル・項目 (間は 1 セル)・空白・`]` で、タブの間は空白 1 つ。印の欄は、アクティブなタブ (フォーカスのペインの文書) では `▶` (ASCII の縮退では `>`)、ほかは空白 (色が無くても、文字をコピー・読み上げても分かる。ペインの見出しの `▶` と同じ考え。アクティブが替わってもタブの幅は変わらない)。名前 (`Path`) は 1 タブあたり 32 セルまで (超えたら `CellText.cut` の中間で `…`)。印 (`Dirty`・`ReadOnly`) は落とさず、短い形があれば短い形を使う (`ReadOnly` は短い形の `RO`。`Dirty` の `[+]` は短い形が無いのでそのまま `[▶main.rs [+] ]` になる)。アクティブなタブは `[` から `]` までを `ui.tab_line.active`、ほかと行の残りを `ui.tab_line` で描く。並べ方は次の手順で決める (タブ i の幅を `w[i]`、アクティブを `a`、行の幅を `W`): (1) `Σw + (タブの数 − 1) ≤ W` なら、全部を 0 列目から並べる。(2) そうでなければ両端に 1 セルずつ印の場所を取り、使える幅を `W − 2` とする。範囲を `[a, a]` から始め (`w[a]` が使える幅を超えるなら、アクティブなタブの名前をさらに中間で縮めて収め、範囲はそれだけ)、右・左の順に交互に、次のタブを (間の 1 セルを含めて) 入れられれば範囲に足す。片側が入らないか端に達したら、その側はそこで止め、もう片側だけを続ける。両側が止まったら終える。(3) 1 列目から範囲のタブを並べ、左に省いたタブがあれば 0 列目に `‹`、右に省いたタブがあれば最後の列に `›` (`ui.scroll_marker`。ASCII の縮退では `<` / `>`) を描く。印の無い側の端と行の残りは空白。並べ方は Frame ごとに決め、前の Frame の範囲を覚えない (同じ Frame からは同じ並び。「(タブの幅の並び, アクティブ, 幅) → 並び」の純関数)。(f) **設定**: `ui.tab_line` は FR-510 の PR で公開する (07 §5.3。不正な値は警告して `auto`。`config.reload` で即時に効き、出し入れは (b) の規則)。(g) **マウス**: TUI は (e) で並べた各タブの矩形と `doc` を、入力スレッドの幾何の写し (PANE-13) に足し、タブの左 `Down` を `ChromeMouseEvent { target: ChromeTarget::Tab { doc }, .. }` で渡す。`‹` `›` の印・タブの間・行の残りのクリック、タブラインのホイール・中と右のボタンは何もしない (PANE-7 (f))。core は、`doc` が開いていなければ・オーバーレイか検索バーが開いていれば (PANE-6 (c)) 捨て、そうでなければ `buffer.next` と同じ経路 (PANE-8 (a)。Undo の封印・検索バーを閉じる・`recent` の記録を含む) でフォーカスのペインに表示する。(h) **ヘッドレスの ASCII の Frame** (eve-testkit): タブラインが `Some` のときは最上行に (e) の並べ方で、ASCII の縮退の記号 (アクティブの印は `>`、省いた側は `<` / `>`) で出す。文書が 1 つなら `auto` では出さないので、既存の 1 文書のゴールデンは変わらない。(i) **ほかの規則との関係**: タブラインを出すかは (b) の 3 行の条件だけで決め、ペインの木の最小 (PANE-4) はタブラインの行を引いた後の編集面で判定する (タブラインを出すと縮退表示に入ることがある)。アクティブなタブのクリックは何もしない (Undo の封印・`recent` の記録もしない)。タブの上で押した後の `Drag` / `Up` は捨てる。文書を 2 つ以上開く既存のゴールデン (ASCII の Frame・L4) は、最上行のタブラインと編集面の 1 行の減少の分だけ変わる (計画した差分。§4.7 の段 1)。(j) **実装で決めた細目** (M32 の段 1、FR-510 の PR): (1) **数える文書**: タブに出す文書と `auto` で数える文書は、ステータスバーの `(n/total)` (§9.8) と同じ集合 (置き換えで読込中の文書は、まだ表示していない間は除く。先行表示 (§9.2.1) と切り替えの保留の間は、読込中の文書と置き換え元の無題のバッファの両方がタブに出る (ステータスバーの `(2/2)` と同じ)。完了して置き換え元を閉じると、文書が 1 つになってタブラインは消える)。PANE-11 の同じ名前の区別もこの集合で行う (見出しとタブで同じ名前になる)。(2) **行の確保の判定の時期**: core は、文書の数・設定・置き換えの読込の状態が変わり得る所 (入力・コマンドの実行・ジョブの結果・時刻の処理の後と、文書を閉じたとき・設定を差し替えたとき・Frame を作る前) で、出すかを決め直す。3 行の条件は、端末の編集面から検索バーの行を引いた後の行数で見る (検索バーの行を先に引く)。(3) **タブの名前を縮めても入らないとき**: アクティブなタブだけで使える幅 (`W − 2`) を超え、名前を空にしても枠と印が入らない幅では、タブを行の幅 (右を省いたときは `›` の手前) で切る (印の列にタブを掛けない。マウスでも印の上は何もしない)。(4) **ASCII の縮退の名前の省略記号** は `...` (3 セル。04 §3.7 の対応表)。32 セルはこの記号を含めた幅。(5) **オーバーレイとタブライン**: オーバーレイ (パレット・一覧・確認など) と which-key は、今と同じく端末の編集面の全体 (タブラインの行を含む) に置く (PANE-6 (b) の「画面の全体に対して出す」。タブラインの下に置き直さない)。検索バーは Frame の編集面の下のまま。(6) **タブのクリックの経路**: core は、アクティブでない開いているタブなら、フォーカスのペインの文書の Undo の結合を封印してから、`buffer.next` と同じく 文書を表示する (表示を替えるときに検索バーを閉じ、ペインの `recent` を記録する)。修飾キーは問わない。(7) **マウスの幾何**: TUI は、描画と同じ記号の表 (ASCII の縮退を含む) で並べたタブの矩形 (`[` から `]` まで) を幾何の写しに入れ、ペインの本文と見出しの矩形を タブラインの 1 行だけ下げて写す (`MouseAt` と見出しの `ChromeMouse` の原点は今と同じく本文・見出しの左上)。本文で押したドラッグがタブラインの行へ出たときは、今と同じく押したペインの原点で (負の行で) 渡す。押している間にタブラインが出入りして幾何が替わったら、押したペインの矩形を、新しい幾何の同じ View のペインから引き直す (原点がずれて、残りのドラッグが 1 行ずれないように。同じ View が無ければ (ペインが別の文書に切り替わった) 押したときの矩形のまま)。(8) **出し入れのスクロール** は、端末の大きさが変わったときと同じ `apply_viewport` の規則で行う (カーソルがスクロールの余白 (`view.scroll_margin`、04 §10) を含めて見えるように最小限動かす。(b) の「カーソルが隠れるとき」はこの余白に入るときを含む) |

```rust
// eve-core (内部。境界の型ではない)
struct Window { id: WindowId, panes: BTreeMap<PaneId, Pane>, root: LayoutNode, focus: PaneId, pane_recent: Vec<PaneId> /*フォーカスした順。末尾が直近*/ }
enum LayoutNode { Leaf(PaneId), Split { axis: Axis, children: Vec<(LayoutNode, u16 /*比率。分割ごとに合計 10,000*/)> } }
// 矩形の計算 (PANE-3 / PANE-4) は状態から切り離した純関数にする: fn layout(&LayoutNode, CellSize) -> 葉の矩形・見出し・区切り、fn min_size(&LayoutNode) -> CellSize
// View からペインを引くときは、各ペインの表をたどる (View にペインの ID を持たせない。表と二重に持たない)
enum Axis { Row /*左右に並べる。子の間に区切り*/, Column /*上下に並べる*/ }
struct Pane {
    id: PaneId,
    view: ViewId,                              // 表示中の View
    views: BTreeMap<DocumentId, ViewId>,       // このペインで表示したことのある文書の View (PANE-1)
    recent: Vec<DocumentId>,                   // このペインで表示した順。末尾が直近 (PANE-9)
}
// 文書ごとに「最後にフォーカスのあった View」を覚える (PANE-1。Document ではなく DocSet に持つ)
// 最近使った順の記録 (Window.pane_recent・Pane.recent・DocSet.recent・文書ごとの最後の View) の更新は、
// フォーカスを移す関数とペインに文書を出す関数の 2 か所に集める。閉じた文書・ペインの ID はどの記録にも残さない
```

**決めた点と退けた案** (M30 の仕様の PR。コマンドの ID・既定キー・フォーカスの示し方 (ペインごとの見出し) は **確定 (ユーザー回答、2026-11-21)。推奨案のまま**)

| 論点 | 採った案 | 退けた案と理由 |
|---|---|---|
| フォーカスのあるペインの示し方 | ペインごとの **見出し** (最下行。フォーカスは反転、ほかは `─` で埋めた淡色) と、フォーカスの無いペインの現在行の強調を消すこと | 区切りの線の種類 (`║` / `│`、04 の v0.4 までのモック): 左右 2 つのペインの区切りは両方に接するので、どちらにフォーカスがあるか分からない。ASCII の縮退では両方 `\|` になり、無色の段で区別できない (FR-509 AC-1) |
| 「水平」「垂直」の言葉 | ID と文言は向きで書く (`pane.split_right` = 左右に並べる、`pane.split_down` = 上下に並べる) | `split_vertical` / `split_horizontal`: 「垂直に分割」が区切りの向きか並ぶ向きかで、Vim と VS Code・tmux で逆に読める |
| 木の形 | 子を 2 つ以上持てる木 (同じ向きを入れ子にしない) と、合計 10,000 の整数の比率 | 2 分木: 同じ向きに 3 つ並べると 50 / 25 / 25 になり、揃える (`pane.equalize`) の結果が直感と合わない。セルの数で持つ: 端末の大きさを変えるたびに丸めの誤差が溜まる |
| 端末が縮んだとき | 木を保ったまま、フォーカスのペインだけを出す (縮退表示) | ペインを最小より小さく描く: 本文が 1 列・0 行になり、カーソルを置けない。木からペインを消す: 端末を戻しても元の配置に戻らない |
| 境界でのペインの指し方 | 表示中の `ViewId` (`PaneId` を境界の型に出さない)。後の版でペインが文書以外 (差分の表示など) を出すようになったら、`FocusTarget`・`ChromeTarget` に変種を足す (追加で済む) | `PaneId` を eve-view に足す: 既存の `FocusTarget::Pane(ViewId)`・`MouseAt.view` と二重になる |
| 文書を作ったときの View | 今と同じく作り、最初に表示したペインに移す。ペインを閉じても、その文書の唯一の View なら残す | ペインで表示するときだけ作る: 裏で読む文書・読込のジョブ・`Editor::open_document` (試験の 11 か所) が View を前提にしていて、作り替えで ViewId と Query の応答が変わる |
| 区切りの確保 | core が `Frame.dividers` で渡す (v0.5-plan §1.2) | TUI が矩形の隙間から区切りを推して描く: ヘッドレスとの食い違いとヒット判定の重複 |
| 区切りのドラッグ | v0.5 は入れない (キーで大きさを変える) | 入れる: 判定の条件に要らず、マウスの捕捉の規則 (ドラッグ中の面の外) を区切りにも足す必要がある |

**PANE-13 実装の入口** (「1 Window = 1 View」の前提がある所。develop の `866f287` の時点)

| 場所 | 今の前提 | 直し方 |
|---|---|---|
| `crates/eve-core/src/editor/state.rs` の `Window { id, root: ViewId }`・`DocSet.windows`・`add_window`・`window_view`・`first_window`・`focused_window`・`focused_view` | Window が View を 1 つ持つ。入力と表示の対象は先頭の Window の root (STR-B2 でここに集めてある) | `Window` を上の型にする。`focused_view` はフォーカスのペインの表示中の View |
| 同ファイルの `viewport` / `set_viewport` / `apply_viewport` と、`term.viewport` を読むスクロールの調整 (`ensure_cursor_visible` ほか) | 編集面の大きさが全体で 1 つで、どの View もその大きさ | 編集面の大きさは Window の全体のまま持ち、View の可視の大きさは、その View を表示するペインの本文の矩形から求める (表示していない View は、表に持つペインの本文の矩形 (次に表示するペイン)、どのペインにも属さない View はフォーカスのペインの本文の矩形で求める。端末の大きさが変わったときは、今と同じく全 View のスクロールを調整する。作り替えの PR で決めた) |
| `crates/eve-core/src/editor/buffer.rs` の `activate_document`・`show_view`・`buffer_close` | 先頭の Window の root を書き換える。文書の View を全 View から探す | フォーカスのペインの表を使う (PANE-1 / PANE-8 / PANE-9) |
| `crates/eve-core/src/io/file.rs` の `active_document`・`close_document`・`new_untitled`、`crates/eve-core/src/io/open.rs` の読込の完了の切り替え | 同上。閉じた後の表示は Window 全体の `recent` | PANE-8 / PANE-9 |
| `crates/eve-core/src/layout/frame.rs` の `build_frame`、`layout/rows.rs`・`layout/wrap_scroll.rs` | ペインを 1 つ、`rect` を (0, 0)、`primary_cursor` をペインの中の座標で作る。折り返しの幅を編集面の幅から求める | ペインごとに作る (PANE-10)。折り返しの幅はペインの本文の幅から求める |
| `crates/eve-core/src/view/mouse.rs`・`view/motion.rs`・`view/view_cmds.rs` | ページ移動・自動スクロールの量を編集面の大きさ (`viewport()`) で決める | ペインの本文の矩形の大きさ |
| `crates/eve-core/src/editor/query.rs` (`Query::Windows`) | `views: vec![w.root]`、フォーカスは root | 葉の順に全ペインの表示中の View。フォーカスはフォーカスのペインの View |
| `crates/eve-core/src/editor/query.rs` の `window_root` (`Query::Status` / `Query::Overlay`) | Window の root の View | フォーカスのペインの表示中の View |
| `crates/eve-tui/src/buffer.rs`・`chrome.rs` の `ScreenLayout` / `bar_rows` | 編集面の上にはクロームが無い。検索バーの行だけを数える | M32 でタブラインの行を同じく数え、編集面を 1 行下げて置く (02 §8.2 の `primary_cursor` の注記)。**M32 の段 1 で直した** (`ScreenLayout.tab_rows`・`surface_top`。マウスの幾何も 1 行下げる、PANE-12 (j) の (7)) |
| `crates/eve-core/src/ui_model/search_bar.rs`・`ui_model/overlay/mod.rs` | 検索とオーバーレイがアクティブな文書を見る | 開いた時点のフォーカスのペイン (PANE-6) |
| `focused_view`・`activate_document`・`viewport()` を呼ぶほかの所 (`crates/eve-core/src/editor/mod.rs`・`io/partial_view.rs`・`io/stdin_doc.rs`・`io/startup.rs`・`io/reload.rs`・`io/escape_merge.rs`・`io/unmappable.rs`・`io/leftover_offer.rs`・`command/clipboard.rs`・`command/edit.rs`・`command/line_ops.rs`・`search/search_state.rs`・`ui_model/config_report.rs`・`ui_model/overlay/quit_chain.rs`) | `focused_view` は「先頭の Window の View」、`viewport()` は「その View の可視の大きさ」として使っている | `focused_view` はフォーカスのペインの View として今の意味のまま使える。表示を切り替える所 (先行表示・標準入力・起動・終了の連鎖) は PANE-8 の「どのペインに出すか」に、大きさを使う所 (ページの量・スクロール) はペインの本文の矩形に合わせる。呼び出しごとに確かめる (`focused_window` / `focused_view` / `window_view` / `viewport()` / `activate_document` / `show_view` を grep する) |
| `crates/eve-core/src/editor/state.rs` の `add_document` / `add_view`、`editor/mod.rs` の `Editor::open_document` (ViewId を返す。試験で使う)、`io/file.rs` の `spawn_open` | 文書を作ると View を 1 つ作り、読込のジョブがその View を持つ | 今のまま (ペインに属さない View。PANE-1)。最初に表示したペインに移す |
| `crates/eve-tui/src/frontend.rs` (`frame.panes.first()` で幾何を覚える)・`crates/eve-tui/src/mouse.rs` (入力スレッドの `SharedGeom`。今は (view, rows) を原子変数で持ち、列の上限を見ない) | マウスの変換が先頭のペインだけを見る | 描画側が Frame ごとに作る幾何の写し (全ペインの `view` と `rect`。見出しは FR-509 の PR、タブとタブラインの行は M32 で足す。`MouseAt` の原点はガターを含むペインの本文の左上で、ガターは core が引くので `gutter_width` は写さない) を入力スレッドへ差し替えて渡す (原子変数の組から `Mutex<Arc<..>>` の差し替えに。並行処理の変更なので作り替えの PR の範囲)。ボタンを押している間は、押したペインの原点で Drag / Up を渡す (PANE-7 (b))。変換を「(幾何, マウスの記録) → 入力」の純関数にし、複数ペインの単体試験を置く (`HitMap` は作らない、§5.8.1)。見出しは `InputEvent::ChromeMouse` にする |
| `crates/eve-testkit/src/ascii.rs`・`harness.rs`・`helpers.rs`・`scenario.rs`・`fuzzing.rs`、`crates/eve-host/src/host.rs` | 先頭の Window・先頭のペインだけを見る (ASCII の Frame の描画を含む) | 全ペインを矩形で描く (1 ペインの出力は変えない。見出しは、フォーカスを `=`、ほかを `-` で埋めて ASCII でも区別できるようにする。M30 の段 2b: フォーカスは `> `・項目・空白の後ろを `=`、ほかは空白・項目・空白の後ろを `-` で埋め、項目は長い形を空白 1 つで並べる。区切りは `|`)。Window の ID の取り方は今のまま |

**PANE-14 実装の順と完了の条件** (08 §2.6 の M30 / M32。同じ場所を触る作業の順は 08 §2.6 の「順序」)

| 段 | PR | 範囲 | 完了の条件 |
|---|---|---|---|
| 1. 作り替え (v0.5-plan の FND) | `refactor` (振る舞いを変えない。境界の型・コマンド・設定・文言を足さない) | 上の型 (葉 1 つ)、PANE-1 のペインの表と `recent`、PANE-8 / PANE-9 (1 ペインでは今と同じ結果。PANE-8 (b) の、読込のジョブ (`Pending::Open`・切り替えの保留) に開いた時点のペインを持たせる所は 2a で、作り替えではフォーカスのペインで判定する)、PANE-10 (a) の座標、PANE-13 の各所 (TUI のマウスと ASCII の Frame を全ペインに回す) | 既存の試験とゴールデンが **差分なしで** 通る (1 ペインの画面が変わらない、FR-509 AC-1 の後半)。`Query` の応答 (`Windows` の `views` と `focus`、`Status`・`Selection`) も 1 ペインでは今と同じ。`cargo xtask check` の全体。木と矩形の計算 (PANE-3) の単体試験 (葉 1 つ・2 つ・入れ子。試験の中で木を組む) |
| 2. FR-509 (M30)。2a (core の木の操作と PANE-4〜PANE-9、PANE-2 の Undo / Redo の選択 (§7.6 の発行 View。`UndoNode.view`)。パレットと `execute` からだけ実行。言語ファイルは 2a で使う `cmd.pane.*`・`info.pane.*`・`status.degraded.panes.*`・`ui.palette.reason.single_pane`・`ui.palette.category.pane` を足す。縮退表示の Frame (フォーカスのペインだけ) は 2a で出す) と 2b (境界の型・TUI・ヘッドレス・既定キー・ロール・ゴールデンの更新。境界の型は TUI とヘッドレスと同じ PR、02 §8.4) に分けてよい | `feat` (`contract-change` のラベル) | 07 §11.6 の `pane.*` と既定キー、`when` の変数 `pane.split` (07 §7.3)、PANE-4〜PANE-11、02 §8.2 の `Frame.dividers`・`Divider`・`PaneFrame.bar`・`PaneBarModel`・`InputEvent::ChromeMouse` (`ChromeTarget::PaneBar` だけ)、07 §8.8 のペインのロール (07 §8.2・§8.7 の表へ移す)、言語ファイル (`cmd.pane.*`・`cmd.buffer.close` の title の変更・`info.pane.no_room`・`info.pane.collapsed`・`info.pane.single_pane`・`status.degraded.panes.*`・`ui.palette.reason.single_pane` と 04 §15.1 の理由の表・`ui.palette.category.pane`。`crates/eve-core/src/ui_model/palette.rs` の `when` の変数 → 理由の対応に `pane.split` → `single_pane` を足す)、TUI の描画とマウス (04)、ASCII の Frame、`CHANGELOG.md` の `eve-view:` の行。07 §11.6 の頻出操作 (FR-606) に分割・次のペイン・ペインを閉じるを足す | 01 §5.5 の FR-509 AC-1〜AC-4 に `verifies:` の試験がある (09 §3.4 の計画)。既存のゴールデンは、次の計画した変更のほかは変わらない (1 ペインの画面は変わらない): 既定キーとコマンドの一覧 (`crates/eve-core/tests/golden/default_keymap.tsv`・`commands.tsv`) の行の追加 (仕様の表どおりの行の追加だけなら更新してよい。PO の決定 2026-11-21。既存の行が変わる・消えるときは人間の判断を得る)、ロールの追加 (07 §8.2 の `ui.pane.*`) による `builtin_themes.tsv` の行の追加、`cmd.buffer.close` の改名 (PANE-9) とペインの既定キーの追加による eve-tui のスナップショット (パレット・ヘルプ・which-key) の差分。M30 の段 2b では、後の 2 つも PO の判断 (2026-11-22) を得て更新した。`check-themes`・`gen-reference --check`・到達性の検査 (`key_reachability`) が通る。Windows Terminal と conhost で目で確かめる (09 §8.2 の互換の行) |
| 3. FR-510 (M32) | `feat` (`contract-change`) | PANE-12、`Frame.tab_line`・`TabLineModel`・`TabItem`・`ChromeTarget::Tab`、`ui.tab_line`、タブラインのロール (07 §8.8) | 01 §5.5 の FR-510 AC-1 / AC-2。ブランチ名・中身・完了の条件の細目は §4.7 の段 1 (M32 の仕様の PR で決めた) |

## 10. クラッシュ復旧・スワップ・バックアップ

3 つの仕組みを区別する。

| 仕組み | 目的 | タイミング | 版 | 要件 |
|---|---|---|---|---|
| 緊急退避 (emergency save) | パニック・シグナル時に未保存内容を失わない | 異常終了の瞬間に同期的に 1 回 | v0.1 | NFR-302 |
| スワップ (swap) | プロセス kill・電源断・OOM でも直前の内容を復旧する | 最後の入力から 2 秒後、以後変更があるたび (最短 10 秒間隔) | **v0.2 の最優先** (swap-lite、§10.2) | FR-118, NFR-307, OQ-17, OQ-42 |
| バックアップ (backup) | 保存前の版を 1 世代残す | 保存成功時 | v0.6 以降 (Could。v0.4 から送り、v0.5 の計画で再び送った、08 §7) | FR-119 |

**データ損失防止の段階化**: v0.1 で必須なのは、原子的保存 (§9.3)、保存直前の外部変更照合 (§9.3 手順 5a)、表現不能・黙示置換の検出と保存の中止 (§8.6 E1 / E3)、無編集の上書き保存を書かないこと (§9.3 手順 0)、区分 B の検出と確認 (§8.6 E2')、終了時の未保存確認 (FR-107)、保存後も Undo を保持すること (FR-204)、巨大ファイルを開く前の警告 (§11.2)、緊急退避 (§10.1) である。スワップは **v0.2 の最優先** とし、v0.1 の既知の制約として「SIGKILL・OOM・電源断では未保存の内容を失う」ことをリリースノートに明記する (リスク受容、OQ-42)。退避からの復旧 UI は v0.1 では最小限 (起動時に「前回の退避があります: パス」と表示し、利用者が手で開く)、v0.2 でスワップと同じ UI (適用 / 別バッファで開く / 破棄、§10.3) にする。

### 10.1 緊急退避 (v0.1, NFR-302)

**方式 (`EmergencySnapshot` から別スレッドで書く) の正本は [02 §11.2.1](02-architecture.md#1121-緊急退避の方式-正本adr-0023)** (ADR-0023)。書き手は eve-host で、OS 別のイベント受信と書き手の起動・配置は [05 §11.4](05-platform.md#114-緊急スナップショット-nfr-302-の実装前提)。本節は **core 側の契約** (`EmergencySnapshot` に公開する内容と退避内容の形式) を定める。退避の書き手は **メインの `catch_unwind` (メインのパニック)・`HostMsg::Fatal` を受けたメイン (他スレッドのパニック)・受信スレッド (Windows のコンソールイベント、(macOS / Linux を出荷する版の) シグナル)** で、どれも `EditorState` に触れずに `EmergencySnapshot` から書く (受信スレッドはメインスレッド以外で書くため)。パニックフックは退避を書かず、02 §11.2.1 の「1 回だけ」の CAS に参加しない (02 §9.1、03 §10.1a)。core は書き出しに関与せず Snapshot を公開するだけにする。

| 項目 | 内容 |
|---|---|
| core の契約 | Host は Editor の状態を変える呼出し (`open_initial` / `handle_input` / `on_job_result` / `tick` / `execute`) の **直後に毎回** `Editor::take_snapshot_updates()` を呼び、返った文書 (本文・`TextFormat`・dirty・パスのいずれかが変わった文書、閉じた文書) のスロットだけを `Editor::emergency_snapshot(doc)` で差し替える (文書ごとの `ArcSwapOption<DocSnapshot>`。打鍵ごとに全文書の Vec を作り直さない。02 §5.4 / 05 §11.4)。`Document::apply` の成功だけでなく、保存の完了 (dirty が変わる)・読込の完了・名前を付けて保存 (パスが変わる) もその呼出しの後に反映される。dirty でない文書 (履歴の現在位置が保存点にある。保存の完了・Undo で保存点に戻った等。保存中に編集が入った文書は保存の完了の後も dirty なので空にしない)・読込中の文書 (`Loading`。v0.4 の先行表示 `Partial` も同じ)・閉じた文書では `emergency_snapshot` が `None` を返し、Host はスロットを空にする (解放する)。空にする条件はこの「dirty でない」「読込中 (`Loading` / `Partial`)」「閉じた」の 3 つだけで判定する (02 §5.4 の `emergency_snapshot`・02 §11.2.1 の「公開」と同じ書き方。標準入力から読んだ文書 (v0.4、§9.2.2) は読み終えた時点で dirty なので、読み込んだ時点から対象になる)。**「dirty でない」の例外** (v0.5 の M29、§9.2.2 の「スワップと緊急退避」): まだ名前を付けて保存していない (パスの無い) 標準入力の文書で 1 バイト以上読んだもの (`Document::stdin_unsaved`。`--readonly` で開いた変更なしの文書) は、dirty でなくても空にしない (内容はほかに残っておらず、パイプは読み直せない)。名前を付けて保存が成功すると (パスが変わる) 空にする。「保存を中止して終了」(§9.7 (f)) の退避もこのスロットから行うので、対象は同じ (終了の種類で例外を外さない)。例外として、**保存ジョブの実行中に閉じた文書** は、その保存が成功するまで (失敗したら §9.2 の「閉じる手順」(a) で退避先に保存・破棄を選ぶまで) 保存に渡した Snapshot を返し続け、スロットを空にしない。Snapshot の clone は O(1) なのでコストは無視できる。core は Snapshot と `TextFormat` を提供するだけで、書き出し自体には関与しない |
| トリガと役割 | パニック (メインスレッドは `catch_unwind` の後にメインが書く。他スレッドはパニックフックが `HostMsg::Fatal` を送るだけで、受けたメインが書く。フックは書かない)、Windows のコンソールイベント (CTRL_CLOSE、外部からの CTRL_C / CTRL_BREAK)、入力スレッドの死亡 (対話なし。以上が v0.1)。Windows のセッション終了通知 (ログオフ・シャットダウン。ベストエフォート、02 §11.2.1) は v0.2、Unix の SIGTERM / SIGINT / SIGHUP は macOS / Linux を出荷する版。退避は **プロセスで 1 回だけ** 実行し、他の終了経路は退避の完了 (または時間予算の満了) を待ってから終了する (02 §11.2.1)。**シグナル・コンソールハンドラのスレッドがメインを待たずに直ちに退避を書く** (メインが端末への書込でブロックしていても退避される)。その後 `HostMsg::Signal` を送り、最大 2 秒メインの終了を待ち、終わらなければ端末復元をベストエフォートで 1 回試みて終了する (終了コードは 07 §18.4)。ただし `CTRL_CLOSE_EVENT` (と v0.2 のセッション終了) では `Signal` を送るだけでメインの終了を待たず、端末復元もしない (コンソールごと閉じられるため。OS の猶予の中で置換中フラグを待ってハンドラから戻る、02 §11.2.1)。シグナル時は確認を出さずに退避する (OQ-28、確定 (ユーザー回答))。退避の経路では **ブロックするロックを取らない**。詳細は 05 §11 |
| 対象 | dirty なすべての Document と、まだ名前を付けて保存していない標準入力の文書 (`--readonly` で開いて dirty でないものを含む。v0.5 の M29。上の core の契約の「例外」) (`Loading` 中・先行表示 (`Partial`、v0.4) の間は除く。標準入力から読んだ文書 (v0.4、§9.2.2) は読み込んだ時点から対象で、manifest に常に `sensitive = true` を付ける (下の「秘密ファイル」と同じ扱い。OQ-219 の 5)。v0.1 は単一文書だが、置き換えの途中 (§9.2) は旧文書も対象になるので集合として扱う)。**小さい文書から順に** 書き、時間予算 (Windows の 5 秒等) 内に OS に渡した分 (`stored`) と確定した (fsync を終えた) 分 (`written`) を manifest に記録する (全文書を OS に渡してから予算の残りで確定する 2 段の書く順は 02 §11.2.1)。**既知の制約** (v0.5 の M29、01 §4.6): 書く順は大きさだけで決め、編集したかどうかを見ない。`--readonly` で開いた標準入力の文書 (変更なし) も対象なので、大きなその文書が、それより大きい編集した文書より先に時間予算や退避先の空き容量を使い、編集した文書を書き終えられないことがある (書く順は変えない)。**時間切れだけが中断で、1 文書の書込の失敗 (権限・ロック) はその文書を `stored = false` (段 1。本文を OS に渡せなかった) / `written = false` (段 2。fsync できなかった) のまま次の文書へ進む** (複数文書で、1 件の失敗が他の文書を落とさないため。§9.8) |
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
| 除外 | 秘密ファイルパターン (正本は 07 §5.12.2。設定 `file.swap.exclude_secret_files`) に該当する文書は **既定でスワップしない** (平文で状態ディレクトリに写さない)。`file.swap.exclude_secret_files = false` で除外をまとめて止められる (ファイルやパターンごとに有効化する手段は無い。照合は 07 §5.12.2 の一覧だけで行う)。読み取り専用・Loading・先行表示 (`Partial`、v0.4)・バイナリの Document も対象外。標準入力から読んだだけで、まだ編集していない文書 (v0.4、§9.2.2。「読み込んだだけ」の印が立っている) も対象外で、最初の編集 (読込以外の本文の変更) から対象にする (緊急退避 (§10.1) は読み込んだ時点から対象。OQ-219 の 5) |
| 書き出しタイミング | 最後の Transaction から `file.swap.interval_ms` (07 §5.2、既定 2000、Clock の deadline) 後。以後は変更があるたび、ただし同じ文書は最短 `file.swap.min_interval_ms` (既定 10000) 間隔。止まらずに打ち続けて `interval_ms` の空きが来なくても、書けていない変更が `min_interval_ms` 続けば書く (前の書出しからは `min_interval_ms` 空ける)。書込中に来た変更は、気づいた時刻を記録しておき、完了後に期限どおり最新の Snapshot で書く。`bg` スレッドのジョブで実行し UI を止めない (`DocSnapshot` を渡す)。同じ文書のスワップのキューは **深さ 1**: 書込中のスワップはキャンセルせず (書きかけのスワップを残さない)、その間に来た要求は文書ごとに 1 つに畳み、完了後に **最新の Snapshot で 1 回だけ** 再実行する (02 §9.1 の `bg` の例外) |
| 削除 | 保存成功 (dirty = false) で削除、または正常終了時に削除。dirty で終了 (破棄を選択) した場合も削除 (ホットエグジットは OQ-17 により既定無効) |
| 復旧 (§10.3) | スワップは全文なので元ファイルの状態に依存せず常に復元できる。元ファイルが外部で変更されていれば、その旨を示して「別バッファで開く」を既定にする。**v0.2 の M6 では複数文書 (FR-103、M9a) が無いので「別バッファで開く」は出さず、「適用 / 破棄」の 2 択にして、元ファイルが外部で変更されている旨を示した上で適用を確認する (確定 (ユーザー回答)、2026-10-28)。「別バッファで開く」は M9a で足す** |

### 10.3 復旧の提案と適用 (v0.2。v0.1 は退避先の表示のみ)

1. 起動時 (または対象ファイルを開くとき)、`swap/` と `recovery/` から該当エントリを **ジョブで** 探す。別インスタンスが同じファイルを開いている可能性 (スワップのロック情報: PID + 起動時刻) を確認し、生きていれば「他のインスタンスで開いています」と警告する。
1b. **置換途中の痕跡 (Windows)** (起動時の一斉走査と復旧の提案は M6e で実装した (2026-10-30)。M6 が実装したのは 1 と 2〜4 のスワップの分): 保存ジョブは開始時 (一時ファイルを作る前) に **保存中フラグ** (`recovery/inflight/<hash(path)>/<pid>-<seq>.toml`、05 §7.5) に対象パスと tmp / bak 名を書き、成功時に消す (退避名・一時ファイルを消せなかったときは残す、05 §4.1 手順 7 / 8) (書けなければ、原子的保存では警告して保存は続け、直接上書きでは書かずに `BackupFailed` を返す。§9.3)。**開く時の検出は v0.1** で、開くたびに (T の有無に依らず) そのパスのフラグを調べ、T が無いとき、または T があり `method = InPlace` の記録があるときに警告する (§9.2 手順 2、05 §4.6)。**起動時の一斉走査は v0.2** で、`list_inflight` で残ったフラグの一覧を得て、各フラグに **記録された** 対象パス・tmp 名・bak 名についてだけ `metadata` を取る (O(フラグ数)。利用者のディレクトリを列挙して `.<name>.eve-tmp-*` / `.<name>.eve-bak-*` を探すことはしない)。直接上書きの事前バックアップはフラグを消した後に削除するので (05 §4.4 手順 9)、フラグの無いバックアップが残り得る。そのため evoeditor 自身の `state_dir/backup/` の直下だけは列挙し、どのフラグにも記録されていないバックアップを孤児として示す (05 §4.6)。`eve-bak` は置換直前の **元ファイル**、`eve-tmp` は書き込み途中または置換に失敗した **新内容** である。以下の提案はすべて v0.2 (v0.1 は §9.2 手順 2 の場所の案内だけで、「bak を元の名前に戻す」操作は提供しない、§9.3 手順 7): (a) 元ファイルが存在せず bak だけがある → 「bak を元の名前に戻す」を既定で提案、(b) 元ファイルも bak もある → bak は置換成功後の削除漏れの可能性が高いので「削除」を既定、(c) tmp がある → 内容が完全か分からないため「別バッファで開いて確認 / 削除」を提案。判定の詳細と OS 固有の手順は 05 §4.6。Unix の rename は原子的なので、この手順が意味を持つのは Windows だけ。
2. 復旧候補があれば **Document を通常どおり開いた上で** 「復旧内容を適用 / 復旧内容を別バッファで開いて比較 (v1.L: 差分ビュー) / 破棄」を提案する。M6 の時点では「別バッファで開く」を出さない (上の表の復旧の行)。
3. 「適用」は `Transaction(Origin::Recovery)` として全置換 (または最小差分) を適用し、**dirty = true、Undo 可** とする。ユーザーが保存するまでディスク上のファイルは変わらない。(v0.4) 緊急退避・スワップは元のバイト列を持たないので、復旧を適用した文書は保持方式の記録を失い「保持なし」になる (§8.7.1)。復旧の Undo では、記録と保持の状態も戻る。(v0.4 の M23) 読み取り専用の文書には提案も適用もしないが、**読み取り専用の理由が `InvalidBytes` だけ (`pua_escape == Enabled`) の文書には、スワップの文字コード (manifest の `encoding`、`SwapEntry.encoding`) が文書の文字コードと同じときだけ提案し、適用する** (不正バイトを含む文書を解除して編集した後のスワップは、次に開くと `InvalidBytes` で読み取り専用に戻るので、そうしないと復旧が出ない。文字コードが違う候補 (例: 自動判定で UTF-8 + 不正バイトと判定された Shift_JIS のファイルを、Shift_JIS で開き直して編集した後のスワップ) は提案せず、候補を残して `info.swap.encoding_mismatch` (「文字コードを指定して開き直すで {encoding} を選ぶと、復旧を提案します」) を 1 回出す。適用すると本文は正しく見えるが、文書は別の文字コードのままなので、解除して保存すると元のファイルを確認なしに別の文字コードで書くため。適用の時点で本文の文字コードが違っても適用しない。読み取り専用でない文書への文字コードの違う復旧は M23 の前からの扱いのまま (開いた形式のまま本文だけ置き換える)。`crates/eve-core/src/io/swap_recovery.rs` の提案と適用の条件を変える)。適用した文書は読み取り専用のまま (`Origin::Recovery` は読み取り専用の拒否の例外) で、`info.swap.applied_read_only` (「読み取り専用の切替で編集を許可すると続けられます」) を出す。`escape_encoding` は manifest の `pua_escaped`・`encoding` と文書の文字コードで決め (§8.3.1)、`Some` になった文書 (元は読み取り専用でなかった文書を含む) には内容に由来する読み取り専用の理由 `InvalidBytes` を付ける (M23 の 2 本目から。衝突の文書への `pua_escaped = true` の復旧は `pua_escape` を `Enabled` にする)。復旧の Undo では、`escape_encoding`・`pua_escape`・`InvalidBytes` の有無も復旧の前に戻る (読み取り専用の間は Undo できない。解除の後に戻せる)。
4. 復旧候補は適用・破棄のいずれかを選ぶまで削除しない。破棄も確認を求める。
5. **複数文書** (v0.2 の M9a、§9.8): 提案の対象は、**表示している文書** に当たる候補 (同じパス、または空の無題)。複数パスで起動したときは、文書をアクティブにした時点で、その文書の候補を提案する (確認ダイアログの表示中は保留し、閉じたら出す)。空の無題に当たる候補が複数あるときは、新しい順に 1 件ずつ提案する (適用か破棄を選んだら次を出す)。提案の後に文書が閉じられたら、候補は一覧に戻す (ディスクに残っているので、次回起動でも見つかる)。
   - **M6 の実装**: 提案は「適用 / 破棄」の確認ダイアログ 1 つで、「破棄」の選択が破棄の確認を兼ねる (Esc で閉じれば何もせず、スワップは残る)。提案は起動時の `RecoveryScan` の結果と、tick での見直し (後から `file.open` で開いた文書)。出すのは読込済みで dirty でない文書だけ (編集中の内容を上書きしない)。該当は、同じパス (Windows は大文字小文字を区別しない)、または無題で空の文書。元のファイルがスワップの後に変わっていれば、その旨を詳細に添える (`disk_stamp` の更新時刻とスワップの manifest の更新時刻の比較)。適用した古いスワップは、新しいスワップが書けるか、文書が保存・閉じられるまで消さない。書いたプロセスが動いているスワップ (他のインスタンス) は提案せず、同じファイルなら警告だけ出す。

### 10.4 バックアップ (v0.6 以降, FR-119 Could)

**v0.4 から v0.5 以降に送り** (08 §7。PO の決定 2026-11-14。`write_atomic` の共通部分の抽出 STR-D4 と一緒に入れる)、**v0.5 の計画で v0.6 以降に送った** (保存の経路は v0.5 の FR-216 も触るので、2 つの視点のレビューを同じ版で重ねない。v0.5-plan §4.3)。下の規則は入れる版の仕様の PR で見直す。

- 保存成功時に、置換前の元ファイルを **`state_dir/backup/` (既定)** または同ディレクトリの `name~` (設定で選択) へ **1 世代** 残す。既定を状態ディレクトリにする理由: `name~` を同ディレクトリに置くと Web 公開ディレクトリでの漏洩 (`config.php~`) や git への混入の古典的な原因になる。原子的置換と組み合わせるため、Port の `write_atomic` に `SaveOptions.backup` (02 §4.5。v0.4 で使う予約で、v0.1 / v0.2 は常に `None`) として渡し、置換直前に元ファイルをコピー (別ディレクトリ) または rename (同ディレクトリ) してもらう。
- 既定は無効 (07 §5.2 `file.backup.enabled`、`file.backup.dir` の既定は空 = 状態ディレクトリの `backup/fr119/`。05 §7.5、OQ-108)。v0.1 ではバックアップの除外パターンを指定する設定は無い (後の版で検討する)。

---

## 11. 巨大ファイル戦略

### 11.1 基本方針

- **全文メモリ常駐 (rope)** を維持し、mmap や部分ロードは v0.1 で採用しない ([ADR-0004](10-decisions.md#adr-0004-テキストバッファのデータ構造), [OQ-05](10-decisions.md#oq-05-巨大ファイルの上限と扱い))。
- 「固まらない」は **UI スレッドで全文走査をしない** ことで実現する。行数・行位置は rope から O(log n)、書記素・幅は可視範囲のみ、検索・解析・保存・スワップはジョブ。
- 巨大さには 2 種類あり、別々に対策する: (a) **バイト数が大きい** (100 MB〜1 GB、行は普通) → 読込時間とメモリ、(b) **1 行が長い** (10 MB の 1 行、JSON/ログ) → 行内走査。

### 11.2 サイズ閾値と挙動

| 閾値 (既定、設定可。キー名は 07 §5.2 が正本) | 挙動 | 要件 |
|---|---|---|
| `file.large_file_threshold` = 20 MiB (**v0.6**。v0.1 には無効化する対象の機能が無いため、キーと枠組みはハイライトと同時に入れる) | FR-123: シンタックスハイライト・インデント自動検出・括弧対応を無効化し、ステータスに「大きいファイル: 一部機能無効」を表示。往復照合 (§8.7) は **無効化しない** (ストリーミングで安価) | FR-123, ADR-0015 |
| `file.huge_file_warn_bytes` = **512 MiB (固定閾値、v0.1。OQ-05、確定 (ユーザー回答))**。v0.1 は定数で、設定キーの公開は v0.2 (M9d で公開した) | 開く前にジョブ内 `metadata` のサイズで判定し警告: 「大きなファイルです (N MB)。メモリ不足になる可能性があります。読み取り専用で開く / 中止」 (§9.2 手順 2)。サイズが閾値以下でも、sniff の後の **デコード後の見積り** (文字コードの倍率と不正バイトの PUA 展開 (約 4 倍) を掛けたもの) と、読込中の **デコード後のバイト数・読んだバイト数** が閾値を超えたら、本文を捨てて同じ確認を出す (§9.2 手順 3 の「読込中のメモリ検査」。読込中に伸びたファイル・デコードで膨らむファイルへの対策) | FR-127, OQ-05, NFR-104 |
| `file.huge_file_refuse_bytes` = なし (既定) | 固定上限は設けない。設定で指定可 | OQ-05 |
| 不正バイトの比率 > 1% (定数。先頭の sniff 範囲 64 KiB で判定) | 開く前 (本文の構築の前) に「メモリを約 N 倍使う」と警告 (§8.3 / §9.2 手順 3。`ConfirmAsk::ManyInvalidBytes`。512 MiB の警告とは別のキー `ui.confirm.open.invalid_bytes` で、選択肢は「読み取り専用で開く / 文字コードを指定して開く / 中止」)。「読み取り専用で開く」の理由は `InvalidBytes` (§9.6)。不正バイト 1 つは UTF-8 で 4 バイトになるため、バイナリに近いファイルではテキストが元の約 4 倍になる | FR-127、02 §14.1 MR4 |
| `file.long_line_threshold` = **1 MiB** (1 行。v0.1 は定数、v0.2 の M9d で公開した。文書は開いたときの閾値を持つ) | v0.1: その行を含む文書を **読み取り専用** (`LongLine`) で開き、その行は先頭 64 KiB まで表示する (§11.5)。v0.2: その行を **char 近似モード** (表示と列は書記素 = char、幅 = char ごとの表引き、その行を含む文書は折り返さない。§11.5.1) で扱い編集できる。検索ハイライトは可視範囲のみ、ハイライトは行単位で打ち切り | NFR-106 |

- 警告の閾値は **固定値 (512 MiB)** とし、v0.1 ではメモリ量に連動させない (OQ-05 で確定)。メモリ量への連動は v0.5 以降で、`SystemInfo { available_memory(), logical_cores() }` を 05 §2 に追加し (OS 別の「利用可能メモリ」の定義: Linux `MemAvailable`、Windows `GlobalMemoryStatusEx`、macOS `host_statistics64`、cgroup 制限)、`file.huge_file_warn_ratio` (07 §5.2) で連動させる。1 GB は v0.1 では参考値の記録だけにし、512 MiB の警告は Must のまま。
- 読み取り専用で開いた巨大ファイルは Undo 履歴・スワップを持たないため、メモリ倍率が小さい。
- 長時間の稼働で増え得る構造には v0.1 から定数の上限を持つ: メッセージ履歴 1,000 件、レイアウトキャッシュは可視行の 2 倍、検索履歴 100 件、検索結果 (§12.2)、Undo (§7.5)。
- 長い行 (4 KiB 以上で長大行の閾値以下) の表示単位の覚えは直近の 2 行分で、その 2 行の折り返しの結果は表示単位を共有する (M32 の段 2。複数カーソルの上下の移動で、元の行と行き先の行を交互に引く。§4.7.4)。最悪の量は 1 MiB 弱の ASCII の行 2 行分の表示単位 (1 つ 64 バイト以下で、約 2 × 64 MiB) と、その折り返しの結果。本文を変えたら (Revision が変わったら) 放す。

### 11.3 段階的ロードと早期表示 (NFR-104)

- **v0.1**: 100 MB は「開く操作から 50 ms 以内に進捗を表示し、入力に応答し、キャンセルできる。全読込は 30 回計測の p50 1.0 s で判定し、最大値は参考値 (目安 1.5 s)」で判定する (§9.2、01 NFR-104)。読込中は `Loading` (読み取り専用・本文なし) で、進捗だけを Frame に出す。
- **v0.4 (M25)**: §9.2.1 の `Partial` により、100 MB で 300 ms の「先頭部分の先行表示」を満たす (1 GB の値は NFR-104 の参考値)。読込が 200 ms を過ぎても終わらないときだけ、固定の先頭 1 MiB (行の区切りで切る) を出す。Partial の間は読み取り専用で、スクロールとカーソルは **先頭の範囲内** に制限する。検索・置換・行へ移動・すべて選択・文書の末尾まで選択を伸ばす操作は Complete まで拒否する (`msg.open.partial_unavailable`。保留はしない。新しく開く読込だけで、再読込・開き直しの間は旧本文で検索できる)。起動時の `+行` は Complete で移る。
- 読込・保存は `io` スレッドで直列に実行する ([02 §9.1](02-architecture.md#91-スレッド一覧-adr-0006))。複数ファイルを同時に開いた場合、アクティブな Document の読込を先に投入する。

### 11.4 行インデックス

- 専用の行インデックスは **作らない**。rope が行区切り数をノードに持つため、`len_lines` / `line_to_byte` / `byte_to_line` は O(log n) で得られる (T3)。
- 行区切りの種類 (LF/CRLF/CR) ごとの件数 (`EolStats`) のみ読込時に数え (読込ジョブが、復号した文字列を木に足すときに、長大行の数と一緒に数える。読込の後に全文を走査し直さない。v0.4 の M21、docs/review/v0.3-findings.md の PRF-17)、編集時は §6.5 の「eol_stats の更新」の規則で増減する (編集の端の前後 1 バイトの窓だけを数え、削除が大きいときは行数の差で補う。文書長に依存しない)。
- 先行表示 (v0.4 の Partial、§9.2.1) の間の行数は「先頭の行数 × ファイルの大きさ (`Progress.total`) / 先頭のバイト数 (UTF-8)」で推定し、行番号の欄の幅だけに使う (行の総数は画面に出さない。文字コードによって外れるが、欄の幅にしか使わない)。Complete で確定する。

### 11.5 長大行 (NFR-106)

- 問題: 10 MB 行で「カーソルの表示列」「表示列 → Pos」「折り返し位置」を求めるには行頭からの走査が必要で、書記素分割 + 幅計算のスループットを数十〜百数十 MB/s とみると **1 回 50〜300 ms** かかり得る。毎キー入力で行うと NFR-103 を破る。
- **対策 (v0.1、Must): 編集しない縮退 (読み取り専用での表示)** (v0.2 の M9d で char 近似モードに置き換えた。以下は v0.1 の記録) (文書末が長大行のときの `cursor.doc_end` / `select.all` の丸めと、全選択のコピーの扱いは §5.4)。読込ジョブ (§9.2 手順 3) が行の長さを数え、`file.long_line_threshold` (1 MiB、v0.1 は定数) を超える行が 1 つでもあれば、その文書を読み取り専用 (理由 `LongLine`、§9.6) で開く。閾値超の行は **先頭 64 KiB (書記素の境界で切る) までだけをレイアウトの対象** とし、その先は行末に「… (以下 N バイト省略)」の印を出して描かない。カーソルと選択はその範囲に限り、End はその範囲の末尾へ動く。行の残りはメモリに保持し (省略は表示だけ)、文書は編集しないので保存で失われることもない (無編集の上書き保存は書き込まない、§9.3 手順 0)。検索の一致がその行の表示範囲より後ろにあるときは件数に数え、移動先としては「長大行の表示範囲外」と示して移動しない。64 KiB の範囲は通常の書記素分割と幅計算で走査するので、char 近似は要らない。**判定は「10 MB の行を含む文書での各操作 (表示・End・水平と垂直のスクロール) が p99 ≤ 100 ms」** (v0.1 Must、01 NFR-106 AC-3、09 BM-106)。ハングを防ぐことが目的で、閾値以下の行と他の行は通常どおり表示する。
- **対策 (v0.2、Must): char 近似モード (編集)**。`file.long_line_threshold` (1 MiB) を超える行は、書記素 = char、幅 = char ごとの表引き (その char だけを 1 書記素とした通常の行の幅。結合文字は `◌` 付き、ZWJ・VS は不可視文字の記号になり、幅 0 にはならない。§11.5.1)、折り返し OFF で扱う。char ごとの幅の表引きは数百 MB/s 出る見込みで 10 MB 行で 30 ms 前後、ASCII の連続部分に高速経路 (memchr 系の SIMD 走査で非 ASCII の初出を探し、その手前は表引きも省く) を入れればさらに速い。**判定は「10 MB の行での各操作 (表示・End・中央挿入) が p99 ≤ 100 ms」** (v0.2 Must。OQ-31、確定 (ユーザー回答)。01 NFR-106 AC-1)。ジョブ化もキャンセルも不要で、「固まらない」を測れる上限に言い換えたもの。近似により結合文字がカーソル位置では分離して見えることがあるが、閾値超の行に限った既知の制約とする。
- 通常行 (閾値以下) でも、ASCII / 結合しない BMP 文字の連続は「書記素 = char、幅は表引き」とみなす高速経路を `eve-text` の走査に入れる。ログ・JSON のような行の大部分はこれで走査コストが 1/10〜1/50 になる見込み (v0.1 から入れる。閾値以下の長い行の NFR-103 を守るため)。なお、閾値 (1 MiB) 以下の長い行は読み取り専用にならず編集できる。§12.2 の可視範囲のハイライトが 1 行あたりメインスレッドで検索する範囲の上限 (16 KiB、`SEARCH_SYNC_LINE_MAX`) は閾値とは別の値で、閾値以下でも広げた範囲が 16 KiB を超える行の検索はジョブに回る (届くまでその行にはハイライトを出さない)。
- **閾値以下の長い行 (例: 300 KB の日本語の段落)**: 縮退も char 近似モードも適用されず列の索引 (v0.4 の §14.4 も閾値を超える行だけ) も無いため、毎フレーム行頭から書記素分割と幅計算をすると NFR-103 を破り得る。v0.1 は **行ごとに直前に計算した `(Pos, 表示列, 書記素数)` を 1 つキャッシュ** し (View ごと、Revision と行で無効化)、カーソル付近の変換はそこからの差分走査で求める。S-03 で不足が分かった場合は、このキャッシュを強化 (複数点化) する。
- **対策 (v0.4、M25)**: [§14.4](#144-長大行の列の索引) の **書記素の単位の列の索引** (v0.2 の `ColIndex` を広げる。行内 4 KiB ごとに書記素の境界に `(バイト位置, 表示列, 書記素の数)` を記録し、要る位置まで遅延して作り、編集では変更の位置より前の記録点を残す) で、閾値超の行でも書記素単位の正確さを保ちつつ各操作 50 ms を目標にする (01 NFR-106 AC-4・AC-5)。v0.3 までの案 (`LineLayoutCache`) は採らなかった (§14.4 の「採らなかった案」)。ジョブにはしない (02 §14、OQ-31)。
- 書記素境界の判定は任意位置から **後方 256 バイト** のコンテキストで行い、それより長い結合列は正確さを諦める (実用上 ZWJ 絵文字列は 50 バイト未満)。**既知の制約**: 地域指示記号 (国旗) の対の偶奇判定は原理的に無制限の後方参照を要し、128 個を超える連続する国旗列では境界を誤り得る。OQ-53 の `GraphemeCursor` + `PreContext` 方式でも同じ制約があり、受容する。v0.4 の長大行の列の索引 (§14.4) は行頭から順に分けるので国旗の制約は受けないが、4 KiB を超える 1 書記素はそこで切る (§14.4 の「1 書記素の上限」。その後ろの桁・表示列が通常の行と食い違う)。
- 折り返し ON での長大行は表示行数が数十万になる。表示行 → 論理行の写像は §14.4 の列の索引の記録点から近似し、ビューポート近傍だけ正確に計算する (案。v0.2 から長大行を含む文書は折り返さない。M25 の計測で送り、v0.5 の計画で v0.6 以降に送った。§14.4 の「長大行の折り返し」)。
- 検索の一致ハイライトは可視範囲の一致のみ求める (§12.2)。

#### 11.5.1 char 近似モードを使う側の突合と実装の分け方 (M9d、2026-10-31。案)

v0.1 の縮退 (読み取り専用で先頭 64 KiB だけ) を、v0.2 の char 近似モード (編集できる) に置き換える。行のレイアウトを使っている箇所を洗い出し、それぞれの扱いを先に決める。**v0.4 (M25)** は、下の表の利用者を同じに保ったまま、索引を書記素の単位に広げて遅延と差分で作る (§14.4。v0.4 での違いは §14.4 の表の「使う側」)。

**今の作りの問題**: 行の表示・カーソルの列・列からの位置は、すべて `line_content` (行全体を `String` にする) と `units_for` (行全体を表示の単位 `Unit` に展開する。1 単位ごとに文字列を持つ) を通る。単位の展開は (Revision, 行) で 1 行分だけキャッシュする。10 MB の行ではこの展開が数百 MB のメモリと数百 ms を使うので、v0.1 は先頭 64 KiB に切って避けている。char 近似モードでは **行全体を展開しない別の経路** が要る。

**対象の判定**: 行ごとに「その行のバイト長 > 文書の `format.long_line_threshold`」(開いたときの閾値、07 §5.2)。行の長さは行インデックスから O(1) で分かるので、表示のたびに判定してよい。編集で行が閾値を超えた・下回ったら、その時点から経路が替わる。読込ジョブは長大行を読み取り専用の理由にしない (v0.1 にあった `LongLine` は v0.2 で外した。§9.6)。v0.1 の先頭 64 KiB の切り取り (`LONG_LINE_VISIBLE`・`line_visible_end`・`clamp_to_visible`) と、行末の「以下 N バイト省略」の印はやめる。

**核となる仕組み**: 長大行の **列の索引** を 1 つ作り、下のすべての利用者がこれだけを根拠にする。

- 行頭から char を 1 回走査し、64 KiB ごとに `(バイト位置, 表示列, char 数)` を記録する (10 MB の行で約 160 点)。**v0.4 の M25 の (5) で §14.4 (書記素の単位・4 KiB ごと) に替えた**。幅は char ごとの表引きで、その char だけを 1 書記素とした通常の行の幅と同じ (結合文字は `◌` + 結合の幅 1、ZWJ・VS などは不可視文字の記号の幅。幅 0 にはならないのでカーソルを置ける)。タブは次のタブストップ、制御文字・不正バイトの記号 (`<0xHH>` 等) も通常の行と同じ幅。印字できる ASCII の連続は表引きを省く高速経路で数える。
- 索引は (文書, Revision, 行) をキーに、文書ごとに長大行 4 行分 (`layout/units.rs` の `LONG_CACHE_LINES = 4`) をキャッシュする (M9d の案は直近の 1 行分だったが、実装で 4 行にした)。編集で Revision が変われば作り直す (その行の 1 回の走査。10 MB で 30 ms 前後の見込み。NFR-106 の 100 ms に収める)。v0.4 (M25) は §14.4 の「覚え」と「編集 (差分)」のとおり、行頭の位置を鍵にして変更の手前を引き継ぐ (**M25 の (4) で char 近似のまま実装した**: 鍵は (行頭の位置, 表示の設定) で、索引は要る位置まで作り、編集では変更の位置より前の記録点を残す。**M25 の (5) で書記素の単位と記録点の間隔 4 KiB にした** (§14.4 の「走査」))。
- バイト位置 → 表示列 / char 数、表示列 → バイト位置は、手前の記録点からの差分走査 (64 KiB 以内) で求める。
- 表示と列の計算は書記素 = char とみなす (**v0.4 の M25 の (5) で §14.4 の書記素の単位に替えた**)。近似によって、結合文字が分かれて見えることがある (§11.5 の既知の制約)。カーソルの位置 (移動・削除・選択の正規化) は通常どおり書記素の境界とする: 境界は位置の前後の窓 (後方 256 バイトの文脈) で求めるので、行の長さに依らない。char の境界に止めると、選択の正規化 (`Selection::normalized`) が書記素の境界へ戻してしまい、結合文字の手前で右へ進めなくなるため (M9d の実装で決めた)。

| 利用者 | 今 (通常の行) | 長大行 (char 近似モード) での扱い |
|---|---|---|
| Frame の行の生成 (`build_row`) | 行全体の `Unit` から `left_col` 以降を切り出す | 索引で `left_col` のバイト位置を求め、そこから画面の幅の分だけを char 単位で単位にする (行全体を `String` にしない)。選択・検索の強調・カーソルの印はバイト範囲で突き合わせるので変更なし |
| カーソルのセル位置 (`cursor_cell` / `cursor_col`) | 行全体の `Unit` を探す | 索引で表示列を求める。セルの幅は、その char の幅 (0 なら 1) |
| 列 → 位置 (`byte_at_col`。垂直移動の目標列・マウス) | 行全体の `Unit` を探す | 索引で求める。セルの途中は前に寄せる (今と同じ) |
| 左右の移動・Backspace / Delete の単位 | 書記素の境界 (`prev_` / `next_grapheme_boundary`。後方 256 バイトの文脈) | 変更なし (窓で求めるので行の長さに依らない。上の「表示と列の計算」の項)。`editor.backspace_unit` も通常どおり |
| 単語の移動 (`next_word_stop` / `prev_word_stop`) | 行全体を文字列にして単位の境界を求める | 行全体を読まない: 位置の前後の窓 (64 KiB) だけを見る。窓の中に境界が無ければ窓の端に止まる (近似) |
| `Home` / `End`・インデント終端 (`indent_end`) | 行全体を文字列にする | `End` は行末 (表示列は索引から)。インデント終端は行頭から空白の連続だけを読む (行全体を読まない) |
| Tab キーのインデント (`display_col`) | 行頭からカーソルまでを文字列にして幅を数える | 索引でカーソルの表示列を求める |
| 折り返し (`wrap_line`) | `Unit` を表示行に切る | **長大行を含む文書を表示する View は、折り返しなしとして扱う** (`view.wrap` の値は変えない。長大行が無くなれば戻る)。表示行数が数十万になる折り返しの写像は、v0.4 には入れない (M25 の計測で送り、v0.5 の計画で v0.6 以降に送った。§14.4 の「長大行の折り返し」)。判定は文書の「長大行の数」で行い、読込で数え、編集のたびに変わった行だけを見て増減する (変わった行は、§6.5「eol_stats の更新」と同じ窓 (端の前後 1 バイトを含み、重なる窓はまとめる) が触れる行。端で CR と LF が合体・分離すると隣の行の境目が動くので、変更の範囲だけでは足りない) |
| 横スクロール (`left_col` の追従) | カーソルの表示列から決める | 同じ (表示列を索引から求める) |
| 検索の強調 | 可視行を行ごとに検索し、16 KiB を超える行はジョブに回す (§12.2) | 変更なし (長大行は可視の範囲に一致があっても、ジョブの結果が届くまで強調しない)。一致への移動は通常どおり (表示範囲外の扱いはやめる) |
| ステータスバーの桁 (`ui.column_unit`) | 行頭からの書記素数・表示列 | 索引の char 数・表示列。縮退の表示 (`status.degraded.long_line`) は「長い行は近似で表示している」意味に変える (01 NFR-106 AC-2)。v0.4 (M25) は索引の書記素の数で、縮退の表示は「長い行を含む文書は折り返さない」意味に改める (§14.4) |
| `cursor.doc_end` / `select.all` / コピー (§5.4 の長大行の丸め) | 先頭 64 KiB に丸める | 丸めない (通常の行と同じ) |
| 保存・スワップ・Undo | 行の形に依らない | 変更なし |

**閾値以下の長い行** (例: 300 KB の段落) は今のまま (行全体の展開と 1 行分のキャッシュ、§11.5)。

**実装の分け方 (案、PR 3 本)**:

1. **列の索引** (済み。core の `layout::long` の `ColIndex` / `CharWidth` と、eve-text の `next_word_stop_near` / `prev_word_stop_near`): 索引の構築・バイト位置 ↔ 表示列 / char 数・char 単位の窓での単語境界。ASCII / 日本語 / 絵文字混在 / タブ / 制御文字で、通常の行の `Unit` の結果と幅が一致すること (書記素が char と一致する範囲で) を試験で確かめる。
2. **core の切り替え** (済み): 上の表の利用者を長大行で索引に切り替え、`LongLine` の理由と先頭 64 KiB の切り取りをやめる。折り返しの扱い (長大行の数)。v0.1 の長大行の試験 (`long_line.rs`) を v0.2 の扱いに書き直す。
3. **表示と計測** (済み): ステータスバーの縮退の表示と文言 (2 で済ませた。`LongLine` をやめると v0.1 の文言が誤りになるため)、BM-106 (09 §5.2。10 MB の 1 行の ASCII / 日本語 / 絵文字混在で、表示・`End`・中央挿入が各 p99 ≤ 100 ms) の計測と記録。
   - **開発機の実測 (2026-10-31。目安で、判定は基準機の `cargo xtask bench --env ENV-W1`)**: リリースビルド、ヘッドレス、120×40。p99 は ASCII / 日本語 / 絵文字混在で、開く (読込 + 最初の Frame) 78 / 29 / 31 ms、`End` 0.4 / 0.1 / 0.2 ms、横スクロール・垂直スクロール 0.3 ms 以下、中央挿入 (1 文字 → Frame。列の索引の作り直しを含む) 23 / 13 / 13 ms。すべて 100 ms の内側。
   - 計測で見つけて直したこと: 絵文字などの一般の経路の char は 1 文字ごとに書記素の表示を組み立てていたので、絵文字混在の 10 MB の行で索引の作成が約 200 ms かかった (開く・中央挿入の p99 が 210〜235 ms)。幅は列に依らないので、`CharWidth` に char の値で引く直接写像の覚え (1024 項目) を入れて 13 ms に縮めた。

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
    compat_fold: bool,                     // CP932 互換文字の同一視 (OQ-41、v0.4 の M24。§12.4.1)
}
pub struct Match { range: ByteRange, captures: Option<SmallVec<[Option<ByteRange>; 4]>> }
```

- **`SearchQuery` の構築** ([OQ-41](10-decisions.md#oq-41-cp932-互換文字の検索同一視) の準備、確定): `SearchQuery` は `#[non_exhaustive]` とし、フィールドを直接並べた構築 (構造体リテラル) を crate の外で許さない。構築は `SearchQuery::literal(pattern)` 等のビルダー (`with_case(..)` などの設定メソッドを含む) 経由に限る。これで CP932 互換文字の同一視 (OQ-41、v0.4) などのフィールドを後から足しても呼び出し側を壊さない。**v0.1 のフィールドは上の 5 つ** (`pattern` / `kind` / `case` / `whole_word` / `scope`) で、これ以外は足さない。v0.1 のビルダーが作るのは `kind = Literal`・`whole_word = false`・`scope = Document` の値だけで、これらを変える設定メソッド (`with_regex` / `with_whole_word` / `with_scope` 等) は v0.2 で加える (v0.1 で変えられるのは `case` だけ)。検索バーの状態 `SearchBarModel` は eve-view の型で、全型 `#[non_exhaustive]` ([02 §8.4](02-architecture.md#84-境界型の契約と凍結段階)) なので追加の作業は要らない。
- **リテラル検索は `memchr::memmem` でチャンクを跨いで実行** する (境界の重なり分 (パターン長 − 1 バイト) だけ連結して走査)。1 GB/s を超えるため NFR-108 / 109 の目標値はリテラル検索に課す。
- **一致の選び方** (リテラル・正規表現に共通): (1) 一致は **重ならないものを左から順に** 取る。ある一致を採ったら、次の候補はその終端以降から探す (`aa` を `aaa` で探すと 1 件)。チャンク境界の重なり部分で見つかった候補も同じ規則で、直前に採った一致の終端より前から始まるものは捨てる。これにより **結果はチャンクの分割に依存しない** (§22.1 のプロパティテスト)。(2) **両端が書記素境界でない候補は捨て**、件数にも数えない (`e` で `é` (e + U+0301) の `e` には一致しない。`👨` で `👨‍👩‍👧` の先頭には一致しない)。捨てた候補の次は、その候補の先頭の次の char 境界から探し直す (捨てた候補と重なる位置の一致を取りこぼさない)。境界の判定は §13.1 と同じ 256 バイトの文脈で行い、コストは候補 1 件あたり定数。一致は常に書記素境界上にあるので、選択 (§4.1 の N1) と Decoration (§14.5) で丸めが起きない。
- **正規表現 (v0.2) は行単位で実行** する (§12.3。v0.4 は文書全体を 1 本の文字列とした意味で一致し、行をまたぎうるパターン (k ≥ 1) と、`\A`・`\z`・`(?-m)` / `(?-R)` の `^` / `$` を含むパターンは行の窓で実行する。§12.3.1 の「実行」)。NFR-108 / 109 は正規表現には課さず、「入力を止めない (NFR-103 を維持する)」ことだけを Must とし、所要時間は参考値として記録する。
- 既定は `Smart`。設定キー `search.case` (07 §5.5) で `"smart"` / `"sensitive"` / `"insensitive"` に変えられる (検索バーを開いたときの値で効く。v0.4 で公開)。
- **検索バーでの切替** (FR-402、コマンドとキーの正本は 07 §11.6): `search.toggle_case` は大小区別を **Smart → Sensitive → Insensitive → Smart** の順に巡回する。初期値は検索バーを開くたびに設定 `search.case` の値 (未設定は Smart)。巡回した値はそのセッション中だけ保持し、設定ファイルには書かない。`search.toggle_regex` (v0.2) は `kind` を Literal ⇔ Regex で切り替える。初期値は検索バーを開くたびに設定 `search.regex_default` の値 (未設定は false。v0.4 で公開)。どちらも when は `search.active` で、切り替えたら直ちに前の検索ジョブをキャンセルして再検索する (§12.2 のインクリメンタルと同じ経路)。状態は `SearchBarModel.case` / `regex` (02 §8.2) に入れ、表示 (`[Aa~]` 等) は 04 §8.4。

### 12.2 実行モデル (FR-404 / NFR-108 / 109)

| 要素 | 方針 |
|---|---|
| 全件走査 | **常にジョブ** (`bg` スレッド。新しい検索要求が来たら古い要求をキャンセル、02 §9.1)。Snapshot と Revision を渡し、結果を `(Revision, Vec<Match>)` のバッチで返す。**バッチは 256 件または 20 ms ごと** (受信口は上限なしなので、送る側がまとめる。02 §9.2)。cancel はチャンク毎に確認 |
| 最初の一致 (NFR-108) | 同じジョブが **カーソル位置から前方** を先に走査し、最初の一致を即時送る。続いて文書先頭から残りを走査。これにより「次へ」の応答と全件カウントを 1 ジョブで賄う |
| インクリメンタル (FR-401) | 検索語が変わるごとに前ジョブをキャンセルし新ジョブを投入。入力の合体 ([02 §9.2](02-architecture.md#92-メインループ-擬似コード)) により連打中の無駄な起動を抑える |
| 件数上限 (FR-404) | 一致範囲の **保持** は `search.max_matches` (07 §5.5、既定 10,000、範囲 100..=1,000,000) まで。件数の **カウント** は `search.count_limit` (07 §5.5、既定 **1,000,000**、範囲 1,000..=1,000,000,000) まで続け、超えたら「1,000,000+」(既定のとき) と表示して打ち切る。どちらも検索バーを開いたときの値で効く (設定の変更は次に検索バーを開いたときから。バーの外からの検索は今の設定) |
| 保持の窓 | 保持する一致は **現在の一致 (無ければカーソル位置) の前後それぞれ 5,000 件** (`max_matches` の半分ずつ) の窓とし、ジョブから届いた一致のうち窓の外のものは捨てる。現在の一致が窓の端から 1,000 件以内に近づいたら、窓を現在の一致を中心に取り直す (取り直しはジョブの単発の再走査、下の `search.next` / `search.prev` と同じ経路)。ジョブは保持とは別に **1 MiB ブロックごとの件数** を返し、main はそれを持つ (100 MB で 100 項目)。現在の一致の番号 (`current`) は「現在の一致を含むブロックより前のブロックの件数の和 + そのブロック内で前にある一致の数」で求める |
| `search.next` / `search.prev` (FR-401) | 移動先が保持の窓の中にあれば、それへ移る (O(log 件数))。**窓の外、全件ジョブの実行中 (`Running`) でその方向の一致がまだ届いていない、または編集で保持が無効になった (`Approx`)** ときは、カーソル (選択の `end`、prev は `start`) から **同期検索** する。同期検索は最大 256 KiB (`SEARCH_SYNC_STEP_BYTES`、定数。1 MiB だと全件検索中のキー入力が p50 2.4 ms になり NFR-103 を超えた、BM-103b-search。行ごとに切り出す経路 (正規表現と §12.4 の 3) は memmem より 1〜2 桁遅い (v0.1 の行ごとに畳み込む実装で約 50 MB/s) ので 1/16 に絞る) か時間予算 2 ms で打ち切り、そこまでに見つからなければ同じ位置から **単発のジョブ** (`Search` の `find_next_from(pos, dir)`。全件ジョブとは別に 1 つだけ、新しい要求は古い単発をキャンセル) を投げ、結果が届いた時点で Revision と選択が投入時のままなら移動する (違えば捨てる)。文書末尾 (先頭) を越えたら先頭 (末尾) から続ける (折り返した旨をメッセージで示す、04)。`current` は、`Exact` / `AtLeast` ではブロック別件数から求めて表示し、`Approx` (編集後) では **求めず表示もしない** (02 の `Approx { total }` のまま。再検索が終わって `Exact` に戻ると表示される)。上限で打ち切った (`AtLeast`) 後ろの一致へ移った場合は `current` を None (`-`) にする |
| 可視範囲のハイライト | 結果が上限で打ち切られていても、可視範囲については **メインスレッドで小さな検索** を行って Decoration を作る。対象は可視行 (ビューポート ± 1 画面) の各行について、**可視列範囲 (水平スクロールの `left_col` から画面幅まで) のバイト範囲を前後にパターンのバイト長だけ広げた範囲** に限る (行全体を走査しない)。広げた範囲が 1 行あたり 16 KiB (`SEARCH_SYNC_LINE_MAX`、定数) を超える行と、範囲を決めるのに行頭からの長い走査が要る行 (§11.5 の閾値以下の長い行で、§11.5 の列キャッシュから求められないもの) はメインスレッドで検索せず、その行の範囲を **ジョブ** に回して結果が届いたら描く (届くまでその行にはハイライトを出さない)。これで同期検索の量は O(可視行 × 16 KiB) 以下に収まり、NFR-103 内に収まる。正規表現 (v0.2) は範囲を広げる量が決まらないので行全体を対象にし、16 KiB を超える行はジョブに回す。時間予算 1 ms を超えたら残りの行をジョブに回す |
| 編集後の結果 | **v0.1**: 保持中の一致は編集で無効にし (`pending_changes` を持たないため写像しない、§17.1)、可視範囲のハイライトは同期検索で即時に描き直し、件数は「約」表示に落として、入力が止まって 300 ms 後に可視範囲から再ジョブ (01 FR-404 AC-4 はこの挙動を受入条件にしている)。**v1.L 以降** (`pending_changes` 導入後): 保持中の一致を `map_range` で写像し、変更範囲と交差するものだけ無効化して周辺を再検索 |

**v0.1 の実装の範囲** (M4 の途中経過。差は次の版までに埋めるか、この節を直す):

- 保持の窓 (現在の一致の前後 5,000 件、1 MiB ブロックごとの件数、窓の取り直し) は作らない。全件ジョブは **走査した順の最初の `max_matches` 件** を保持し、数えるのは `count_limit` 件まで。保持に無い (走査中・上限超・編集後) ときの `search.next` / `search.prev` は同期検索 (最大 256 KiB) で動き、そこで見つからなければ **単発のジョブ (`JobKind::FindNext`、`bg` ワーカー。進行中は 1 つだけで新しい要求が古いものを取り消す)** で探し、結果が届いた時点で Revision と選択が投入時のままなら移る (違えば捨てる)。保持の窓は、この単発のジョブが答えの正しさを担うので、移動の速さを上げたくなったときの最適化として後に回す。
- ジョブは `bg` ワーカー (eve-host の 2 本目のスレッド。最初の検索の投入で作る) で動き、保存や読込を待たせない。最初の一致は待たずに送り、以後は 256 件または 20 ms ごとのバッチ。
- 走査は、検索語がチャンクごとに探せる場合 (大小を区別するか、大小の区別がない文字だけか、ASCII だけで `k` `K` `s` `S` を含まない検索語で、単語単位でないもの) は行に切り出さず `memmem` で行う (開発機で約 1.4〜1.9 GB/s、100 MB。`crates/eve-core/tests/search_speed.rs`)。それ以外は行ごとに切り出して探す: 大小を区別しない設定で非 ASCII の区別のある文字か `k` `K` `s` `S` を含むもの (と、大小を区別しない単語単位) は、検索語をエスケープした `(?i)` の正規表現で探す (§12.4 の 3)。大小を区別する単語単位は行ごとの memmem で探し、両端を行の文脈で調べる。
- 可視範囲のハイライトは、保持した一致ではなく **可視行をその都度同期検索** して作る (1 行が 16 KiB を超える行は強調しない)。正規表現の経路 (利用者の正規表現と §12.4 の 3) は、照合器を `bg` のジョブが作る (§12.3.1 の「メインスレッドでのコンパイル」) ので、最初のジョブの結果が届くまで強調しない。強調は Decoration ではなく、Frame を作るときに行のスタイルへ直接重ねる (一致は下線、入力で見つけた現在の一致は反転 + 太字 + 下線)。Decoration の仕組みは E3 の該当箇所の強調など他の用途が入るときに足す。編集後 300 ms の再検索は未実装で、次の `search.next` / `prev` か入力の変更でやり直す。

**複数行の正規表現 (v0.4、M24)**: 行をまたぎうる正規表現 (§12.3.1 の k が 1 以上か上限なし) の全件走査・`search.next` / `search.prev`・入力中の現在の一致・可視範囲のハイライトは、上の表と違う規則に従う (一致の集合を文書の先頭から選ぶため。全件走査は先頭から、次 / 前は同期検索をせず単発のジョブで先頭から、可視範囲は k 行前から)。規則の正本は §12.3.1 の「呼び出し側」。

### 12.3 正規表現

**版: v0.2** (行単位)。v0.1 は利用者が正規表現を書く経路を持たない。ただし行単位の実行経路そのものは v0.1 から使う: 大文字小文字を区別しないリテラル検索で、大小の区別がある文字を含み ASCII の経路に乗らないパターンは、パターンをエスケープしてこの経路で照合する (§12.4)。

- エンジンは `regex` (線形時間保証、[02 §13.5](02-architecture.md#135-検索))。後方参照・先読みは提供しない (ReDoS 回避、P3 UI を止めない)。
- **行単位の実行 (v0.2〜v0.3。v0.4 も行をまたがない検索語はこの経路)**: 1 行ずつ `&str` に切り出して (チャンクを跨ぐ行は連結して) `regex` を実行する。長大行 (§11.5) はワーカー内でコピーするので、メインスレッドで大きな文字列を作ることはない。**複数行にまたがるパターン (`\n` を含む) は v0.2〜v0.3 では「未対応」と表示** する (`newline_sugar` の糖衣は行内の `\r` の扱いにのみ使う。v0.3 までの実装は、`\n`・`\r` のエスケープを含むパターンをすべて「未対応」にし、糖衣を入れていない。v0.4 の規則は §12.3.1)。
- **複数行の正規表現 (v0.4)** の実行方式は **行の窓** (OQ-60、S-04 の実測で 2026-11-11 に確定。`spikes/S-04/REPORT.md`): 検索語の構文木から、1 つの一致が含みうる行の終端のバイト (`\r` / `\n`) の数の上限 k を求める。行の境界に揃えた約 1 MiB の核と、その後ろの k 行を 1 本に複写した窓の上で `regex` を実行し、核の中で始まる一致だけを採る (k 行より先へは伸びないので、窓に収まる)。窓の前後には 1 文字の文脈を付け、`\b`・`^`・`$` の判定を全文と揃える。k に上限の無いパターンは窓で探せないので、小さい文書は全文を連結して探し、大きい文書は未対応と示す (大きさの境は 16 MiB、どのパターンが当たるかは §12.3.1)。`regex-cursor` などの crate は足さない (crate の数は増やさない。検索語の書き換えに使う `regex-syntax` は `regex` の依存として既に入っている)。`TextStorage` の公開 API は変えない。意味の細目 (行の終端に一致する書き方・一致の集合・次 / 前・範囲の端・単語単位・置換のキャプチャ) は §12.3.1 (M24。利用者に見える点は OQ-218、確定 (ユーザー回答、2026-11-12))。
- **走査の境界 (v0.3、docs/review/v0.3-findings.md の STR-B6)**: 走査の入口は `search::scan_bytes` (行頭と行末に揃った範囲を受ける) の 1 つにする。中の経路は、チャンクごとの memmem (改行を含まないリテラル) と、行ごとに切り出して照合する `LineScan` の 2 つで、どちらも一致は行をまたがない。`LineScan` の行の切り出しは、範囲のチャンクを 1 回だけ順に読んで行の終端 (LF・CR・CRLF) で区切り、1 つのチャンクに収まる行は複写しない (v0.4 の M21。行ごとに行の位置を引き直して複写していたのをやめた。docs/review/v0.3-findings.md の PRF-05 / 12)。検索バー・全件走査・次の一致・置換は、どれもこの入口を通す (置換の `scan_replacements` は、行を直接回していたのを v0.4 の M24 の (1) で入口 (キャプチャも渡す `scan_bytes_with_captures`。本体は `scan_bytes` と同じ) に寄せた、§12.3.1)。複数行の正規表現 (v0.4) は、OQ-60 で決めた方式を `StreamScan` (行の窓と全文の連結) としてこの入口の中に足した (M24 の (2)。(2)〜(3) の間は eve-core の内部の設定で開いたときだけ通り、(4) で既定で通るようにした。核の大きさと連結の上限を試験から小さくする口 `SearchTuning` は feature `test-hooks` のときだけあり、既定の値より大きくはできない)。探さない理由と打ち切りの理由は、同じ本体の `scan_range` が `ScanEnd` で返す (`scan_bytes` は真偽だけ)。入口の約束は「`range.start` から選び始めた一致を返す」で、正本 (文書の先頭から選んだ一致) と同じになるのは `range.start` が文書の先頭・正本の一致の終端・再開の位置のときだけ (1 行で済むパターンでは行頭も)。呼び出し側は正本が要る所でこれに合わせる (§12.3.1 の「走査の入口の約束」)。v0.3 では `regex-cursor` などの crate は足さない。
- **フラグ**: 複数行モード `m` (`^`/`$` が行頭/行末)、**CRLF モード `R`** (`\r\n` と `\r` を行終端として扱う。§8.4 の改行の「保持」方式で `$` が CR の前に一致するために必須)、Unicode モード既定 ON (`\w`, `\b`, `(?i)` が Unicode 対応)。`.` は改行に一致しない (`s` フラグはユーザーが明示)。v0.3 までの実装は `m`・`R` を付けていない (1 行ずつ探すので、`\A` / `\z` が各行の先頭 / 末尾に一致すること以外は結果が変わらない)。v0.4 は常に付け、文書全体の意味で一致させる (§12.3.1)。
- **検索語の `\n`**: リテラル検索では `\n` の入力は不可 (単一行入力欄)。正規表現モードで `\n` を書いた場合は、改行 1 つ (LF・CRLF・単独の CR) に一致させる糖衣を入れる (v0.4 の書き換えは `(?:\r\n|\n|\r(?mR:^))`。CRLF を CR と LF に分けて一致させない。設定で無効化可。複数行一致は v0.4 から)。クラスの中の `\n`・続けて書いた `\r\n`・単独の `\r` の扱いは §12.3.1 の表。
- **構文エラー**: 入力中は赤色表示のみ、ジョブは投入しない。エラーメッセージは regex のものを整形して表示。
- **コンパイル上限**: `size_limit` を設定 (既定 10 MiB) し、巨大パターンは拒否。

#### 12.3.1 複数行の正規表現 (v0.4、M24)

FR-402 の v0.4 の部分 (01 §5.5 の FR-402 AC-5〜AC-9)。実行の方式は OQ-60 (確定) の **行の窓**。本節の意味の細目は M24 の仕様の PR で決めた (2026-11-12 に案。仕様との整合・利用者・性能と依存の 3 つの視点でレビューした)。利用者に見える点は [OQ-218](10-decisions.md#29-oq-176-以降-並行修正での追加起票) で PO が決めた (確定 (ユーザー回答、2026-11-12): すべて推す案 (★) のとおり)。本節はその内容 (★ は決めた案の印)。S-04 の REPORT の「実装の前に決めること」の 1〜9 は、下の各項で決める (8 の依存は「書き換え」、9 の核の大きさと取り消しは「実行」と「最悪の入力の上限」)。

**行の終端に一致する書き方** (OQ-218 の 1、★): 行の終端の文字は LF (U+000A) と CR (U+000D) の 2 つ (§2.4。U+0085・U+2028・U+2029 と、不正バイトのエスケープ文字 `<0x0A>` (U+10FF0A) は行の終端でない)。v0.3 までは 1 行ずつ探したので、どの書き方も行の終端に一致しなかった。v0.4 は、**行の終端を明示した書き方だけが行の終端に一致し、それ以外の書き方は v0.3 と同じく 1 行の中で一致する**。`\n` を 1 つ書き足しても、検索語の他の部分の意味は変わらない。

| 書き方 | 行の終端に一致するか | 書き換え (`(?mR:^)` は「CR の後ろが LF でない」の意味。下の「CRLF は 1 つの行の終端」) |
|---|---|---|
| クラスの外の `\n` (`\x0A`・`\x{A}` など LF を符号位置で書いたものを含む) | 改行 1 つ (LF・CRLF・単独の CR のどれか) | `search.newline_sugar = true` (既定): `(?:\r\n\|\n\|\r(?mR:^))`。`false`: `\n` のまま (LF だけの改行に一致し、CRLF の改行には一致しない) |
| クラスの外で続けて書いた `\r\n` | CRLF の改行だけ | `\r\n` のまま (糖衣を掛けない)。間にグループや量指定子を挟んだものは続けて書いたものに数えない。`\n` に量指定子が付いたもの (`\r\n+`・`\r\n?`) と、`\r` と `\n` の間にフラグがあるもの (`\r(?i)\n`) も続けて書いたものに数えない (`\r` は単独の CR として扱い、CRLF の文書では一致しない。CRLF の続きには `(?:\r\n)+` か `\n+` を使う) |
| クラスの外の `\r` (`\x0D`・`\x{D}` を含む) | 単独の CR の改行だけ | `\r(?mR:^)`。明示した `\r?\n` は、CRLF の改行と、単独の CR の改行に続く改行 (2 つの改行) の両方に一致する (改行 1 つには `\n` を書く) |
| 要素として `\n` を書いたクラス (`[\n]`・`[ \t\n]`・`[\s\n]`・クラスの中の `\x0A`) | 改行 1 つか、クラスの 1 文字 | `true`: `[ \n]` は `(?:\r\n\|\r(?mR:^)\|[ \n])`。`[\n ]+` は CRLF の文書でも行をまたぐ。`false`: `\n` (LF) を要素に残すだけ |
| 要素として `\r` を書いたクラス (`[\r]`・`[\r ]`・クラスの中の `\x0D`) | 単独の CR の改行か、クラスの 1 文字 | `[\r ]` は `(?:\r(?mR:^)\|[ ])` |
| `(?s)` の `.` と、集合がすべての文字になるクラス (`[\s\S]`・`[\w\W]`・`[\d\D]`) | 改行 1 つ (CRLF も 1 文字に数える) か、ほかの 1 文字 | `(?:\r\n\|\r(?mR:^)\|[^\r])`。すべての文字のクラスは、明示した「どの文字でも」とみなす (ripgrep・JavaScript の書き方をそのまま貼っても動く)。繰り返すと上限なし (下の「実行」) |
| `.` (`s` なし) | しない | CRLF モード `R` で CR と LF の両方を除く (`regex` のまま) |
| 否定のクラス (`[^"]`・`[^\n]`)、略記のクラス (`\s`・`\S`・`\W`・`\D`・`\w`・`\d`)、Unicode のクラス (`\p{..}`・`\P{..}`・`[[:space:]]`)、範囲 (`[\x00-\x1F]`) | しない | 行の終端を集合から除く。`[^\n]` は CR にも一致しない (S-04 の「`[^\n]` は CR に一致する」問題を避ける)。`"[^"]*"` と `\s+` は v0.3 と同じく 1 行の中で止まる。集合がすべての文字になるものは上の行 |

- **CRLF は 1 つの行の終端** (正しさの規則。OQ の論点ではない): 検索語のどの部分も、CRLF の CR と LF に分けて一致しない (`\n`・`\r`・クラスの糖衣・`(?s)` の `.` のどれも)。たとえば `a\r\nb` で `\n\n`・`\n{2,}`・`\n\s*\n`・`[\s\n]{2}` は 0 件、`a\r\n\r\nb` の `\n\n` は 1 件 (`[1, 5)`)。CRLF の文書で空行をまとめる置換 (`\n\n+` を `\n\n` に) が、すべての行の後ろに空行を足すことはない。**実現の方式**: CR に一致しうる書き方を上の表の形に書き換え、CR は「後ろの LF と一緒 (`\r\n`)」か「後ろが LF でないとき (`\r(?mR:^)`。CRLF モードの `^` は CR と LF の間には一致しないので、CR の直後で `^` が成り立つのは後ろが LF でないときだけ)」にだけ一致させる。LF に一致する書き方は書き換えない (CRLF の LF だけに当たるのは一致の先頭がその LF にあるときだけで、それは書記素境界の規則 (§12.1) で捨てる)。挿む `^` は `(?mR:…)` で包み、利用者の `(?-m)` などのフラグに左右されない。この方式を選んだ理由: 文書のバイトを変えない (窓の複写と全文の連結はそのまま。一致の位置とキャプチャを写し戻さなくてよい)、`regex` の機能 (CRLF モードの `^`) だけで書ける、行の窓と全文の連結の両方に同じに効く。退けた案: 窓に写した文字列で CRLF の CR を、どの書き方にも当たらない 1 バイトに置き換えて `regex::bytes` で探す (CRLF モードの `$` が CR の前に一致しなくなり、`$` の書き換えが要る。CR の代わりに LF を置き換えると `^` が同じことになる)。
- 行をまたぐ空白は `[\s\n]*` のように、行の終端をクラスの要素として書き足して表す。入れ子のクラスと集合の演算 (`&&`・`--`・`~~`) の細かな書き換えは、この表の意味を保つように実装の PR で決め、表の例 (上の CRLF の例を含む) を L0 のゴールデンテストに置く。
- **書き換えの細目** (M24 の (1) で決めた。表の書き方ごとの結果は `crates/eve-core/tests/golden/search_rewrite.tsv`): 「要素として書いた」に数えるのは、否定でないクラスの一番外の要素に書いた `\n` / `\r` の 1 文字だけで、範囲 (`[\t-\r]`)・入れ子のクラス (`[a[\n]]`)・集合の演算 (`[\n&&\s]`) の中のものは数えない (集合から行の終端を除く)。要素を書いたクラスの残りの 1 文字の側では、ほかの要素が持ち込む CR を除き、LF は要素に書いたときだけ残す (`[\s\n]` は `(?:\r\n|\r(?mR:^)|[[\s\n]--\r])`、`[\r\s]` は `(?:\r(?mR:^)|[[\s]--[\n\r]])`)。集合から行の終端を除くクラスは、集合が含む行の終端だけを除く形にする (`[X--\r]` / `[X--\n]` / `[X--[\n\r]]`。`[^\n]` は `[[^\n]--\r]`)。`(?-R)` の `.` は CR に一致するので `[^\n\r]` にする。`\p{Any}` もすべての文字のクラス。すべての文字のクラスのうち要素に `\n` / `\r` を書いたもの (`[\n\s\S]`・`[\r\n\s\S]`) は、書き換えは `[\s\S]` と同じで、行の終端を明示した書き方に数える (行をまたぐ検索を開くまで (08 §2.5 の M24 の (4)) は、`\n` と同じく「未対応」)。続けて書いた `\r\n` は `(?x)` の空白を挟んでも続けて書いたものに数える (空白は構文の上で消える)。量指定子の対象を並びに置き換えるときは `(?:…)` で包む (`\r+` は `(?:\r(?mR:^))+`)。
- **書き換え**: 上の表と CP932 互換文字の同一視 (§12.4.1) は、検索語の構文木の **AST の段** で書き換え、書き換えた AST をそのまま印字して `regex` でコンパイルする (`\w`・`\p{..}` を範囲に展開しないので、コンパイルの費用が元の検索語に近い)。一致が含みうる行の終端の数の上限 k と最大の長さは、書き換えた AST から作った HIR で求める。使う crate は `regex-syntax` (`regex` の依存として既に入っている。crate の数は増やさない。eve-core の直接の依存にする、02 §4.1 / §13.5)。`regex-automata` の `build_from_hir` で HIR から直接作る案は、置換の `Captures::expand` などを `regex` の外で持つことになるので採らない。**書き換えは捕獲グループを足さない** (足すグループはすべて `(?:…)`。グループの番号と名前を保ち、置換の `$1` / `${name}` の意味を変えない)。書き換えそのものの確かめ (差分 fuzz は参照にも同じ書き換えを掛けるので、書き換えの誤りを見つけない): CR と LF を含まない文書で同一視を切ったとき、書き換えた検索語と元の検索語 (`m`・`R` を付けたもの) の一致とキャプチャが同じであること (09 §3.2)、と CRLF の例のゴールデン、と 09 §3.3 の 2 つ目の参照。

**文書全体の意味**: 正規表現は、文書の全文を 1 本の文字列とした意味で一致する (行の窓・行単位の経路は実行の方式で、結果を変えない)。フラグ `m` (複数行モード) と `R` (CRLF モード) を常に付ける (§12.3)。`^` / `$` は行頭 / 行末 (CRLF の間には一致しない)。**`\A` / `\z` は文書の先頭 / 末尾** に一致する (OQ-218 の 4、★。v0.3 までは 1 行ずつ探したので、各行の先頭 / 末尾に一致していた)。`\b` は全文の前後の文字で決まる (行頭の `\b` は前の行の終端を単語でない文字として見るので、v0.3 と同じ結果)。

**一致の集合 (正本)**: 一致は、文書 (選択範囲内の検索でも文書の全体) を **先頭から** §12.1 の「一致の選び方」(重ならないものを左から、空の一致は採らない、両端が書記素境界でない候補は捨てる) で選んだものとする。1 行の中で済むパターンでは、行頭から選び直しても同じ結果になる (行の境界が区切りになる)。行をまたぐパターンでは、選び始める位置で結果が変わりうる (例: `a\na` を `a\na\na` で探すと、先頭からは `[0, 3)` の 1 件、2 行目の行頭から探すと `[2, 5)`)。**すべて置換・件数・次 / 前の移動** は正本に従う (1 件置換の対象の確かめ方は下の「呼び出し側」の置換、画面の強調は同じく可視範囲の項)。

**走査の入口の約束** (`scan_bytes`、§12.3 の「走査の境界」): `scan_bytes(range)` は、`range.start` から §12.1 の選び方で選び始めた一致を返す。これが正本と一致するのは、`range.start` が文書の先頭か、正本の一致の終端か、下の「再開の位置」のときだけ (k が 0 のパターンでは行頭も。今と同じ)。行の窓・全文の連結の経路では `range.start` は char 境界ならどこでもよく、`range.end` より前で始まる一致を返す (一致の終端は `range.end` を越えうる)。行単位の経路 (`LineScan`・memmem) は範囲を行頭と行末に揃えて受け、行の途中の始端は、その行の行頭へ丸めて探す (始端より前で始まる一致も返すので、呼び出し側が除く。今と同じ)。v0.3 の「呼び出し側は変えない」は、この約束に置き換える。呼び出し側は、正本が要る所では始端をこのどれかにする (下の「呼び出し側」)。

**実行** (検索語ごとに、書き換えた検索語の HIR から「1 つの一致が含みうる行の終端のバイトの数の上限 k」を求める。S-04 の `max_newlines`。CRLF は 2 と数える):

| k | 経路 |
|---|---|
| 0 (行をまたがない) | 行単位 (`LineScan`) か行の窓。結果は同じなので実装が選ぶ (S-04 では窓が行単位の 0.20〜0.87 倍)。ただし `\A`・`\z`・`(?-m)` の `^` / `$` と、`(?-R)` の `^` / `$` (CRLF・CR を行の終端と見ない行頭 / 行末) を含む検索語は、行単位では文書全体の意味にならない (行の終端を文字列の端と見てしまう) ので窓で走らせる (§12.1 の「正規表現は行単位で実行」の例外。リテラル検索の経路は変えない。延ばす分は無く、窓の大きさの上限 (16 MiB) も掛けない。核の切り方は下の「1 以上の有限」の行と同じで、最大の長さが無い検索語と単語単位の検索語では、長い行は行単位と同じく行の全体を複写する)。下の「重い検索語」も、取り消しの遅れと処理の速さの下限を掛けるため窓で走らせる |
| 1 以上の有限 | **行の窓** (`StreamScan`、OQ-60)。窓を延ばす量は「核の最後のバイトを含む行の後ろの k 行 (CRLF も 1 行と数える)」と「書き換えた検索語の最大の長さ (HIR の `maximum_len` が有限なら、そのバイト数)」の小さいほう。核は、約 1 MiB (`SEARCH_WINDOW_CORE_BYTES`、定数。重い検索語は小さく始める、下の「取り消しの遅れ」) と延ばす量の大きいほうを、**最大の長さが有限で単語単位でない検索語では char 境界で (行の途中で) 切り**、それ以外 (最大の長さが無い検索語・単語単位) は行の境界に揃えたもの (行の途中で切ると、延ばす分の行を核ごとに読み直して O(n²) になるため。延ばす分の複写を核より大きくしないので、複写の合計は文書の約 2 倍まで。大きくすると長い行に揃って窓が 16 MiB を超えるときは大きくしない)。窓 = 核 + 延ばす分 + 前後 1 文字の文脈 (大きさは先に確保する) の上で `regex` を実行し、核の中で始まる一致だけを採る。核の中で始まる一致は k 個より多くの行の終端を越えず、最大の長さも越えないので、窓の終わりより手前で終わる。最大の長さが有限で単語単位でない検索語の窓は「核 + 最大の長さ」以下で、文書の行の長さに依らない (16 MiB を超える行があっても探せる)。**窓が 16 MiB を超えるとき** は、核を 1 行 (核の始端を含む行の残り) に縮めて窓を作り直し、それでも超えるとき (最大の長さが無く k が大きい検索語と長い行の組 (`(?:[^\n]*\n){100}` など)、単語単位と長い行の組、最大の長さそのものが 16 MiB に近い検索語) は、上限なしと同じに扱う。超えるかを核の切り方 (範囲の始端・縮めた核の大きさ) に依らず核の始端を含む行で決めるためで、正本の走査が探せた文書を、行の途中の正本の位置 (再開の位置・一致の終端) から走査し直す次 / 前が探さないとはしない (単語単位の窓は核の始端を含む行の行頭から作るので、行の途中の始端に核の大きさを足して行の境界へ揃えると次の行まで含み、正本の走査のどの窓より大きくなりうる) |
| 上限なし | 窓で探せない (`(?s).*`・`\n+`・`[\s\n]*`・`[\s\S]*` のように、行の終端に一致しうる書き方を上限の無い繰り返しに入れたもの。上の表の書き方では、`\s+`・`[^"]*` は k が 0 で、これに当たらない)。文書が **16 MiB** (`SEARCH_CONCAT_MAX_BYTES`、定数。文書の UTF-8 のバイト数で数える) 以下なら、全文を 1 本に連結して `regex` で探す (連結は 10 MB で約 3 ms、S-04)。超える文書では探さない (OQ-218 の 2、★。下の「探さない場合」)。重い検索語 (下の「重い検索語」) は連結に入れず、始める前に打ち切る (下の「取り消しの遅れ」) |

- **窓から窓への引き継ぎ**: 各核で探し始める位置は max(核の始端, 直前に採った一致の終端)。この位置が核の外 (直前の一致が核の終わりを越えた) なら、その核は飛ばす (実装は、次の核をその位置 (行の途中) から始める。結果は同じ)。書記素の境界・単語単位で捨てた候補の次の探し直し (候補の始端の次の char 境界から) も、窓をまたいで続ける (探し直す位置が核の外なら次の核へ)。これで窓の切り方に依らず、全文で選んだ結果と同じになる (`fuzz_regex_search` で核を 1〜40 バイトにして確かめる)。各核で探し始める位置は正本の走査の状態そのものなので、再開の位置として呼び出し側へ知らせる (`ScanCtx::resume`。全文の連結は核の大きさの間隔ごと)。窓の前の 1 文字は、始端より前を `regex` の `find_at` が `^`・`\b` の判定に使う。後ろの 1 文字は、核の中で始まる一致が窓の終わりに届かないので判定には使われない (安全のための余り)。
- **探さない場合** (上限なしで 16 MiB を超える文書・窓が 16 MiB を超える検索語): 構文エラーと同じ扱いにする。上限なしの検索語と、最大の長さが有限で単語単位でない検索語は、始める前に文書の大きさと最大の長さで決まる (`Matcher::too_large_for`)。k が有限で最大の長さが無い検索語と、単語単位の検索語は、窓が上限を超えることが走査の途中で初めて分かることがある (`ScanEnd::TooLarge`)。そのときは、それまでに届いた一致 (強調・保持・件数) を捨てて、探さない場合と同じに示す (呼び出し側の処理は M24 の (3) で入れた)。検索バーの件数の位置に `msg.search.multiline_too_large` (04 §15.2) の text だけを出し (検索バーは hint を描かない)、ジョブを投げない。`search.next` / `search.prev`・すべて置換・1 件置換では、メッセージ行に text と hint を出して何もしない。text は結論を先に短く、引数は `{mib}` (16)。hint は回数を限る書き方の例 `(?s).{0,1000}` (M24 の (4) で、書き換えた後に下の「重い検索語」に当たらず (繰り返しの上限は 1000 で、1000 を超えない)、最大の長さが有限で窓が 16 MiB に収まることを試験で確かめた)。判定は検索を始めるときの文書の大きさで行い、編集で境を越えた・下回ったときは次の再検索 (編集が止まって 300 ms 後、§12.2) で切り替わる (始める前に探さないと決めた場合も、走査の途中で分かった場合も同じ。探さない理由を持つ検索の状態は編集で古くなり、再検索で判定し直す)。理由をメッセージ行と検索バーに出す文言は、M24 の (3) では仮に `msg.search.invalid_regex` の理由に入れていたのを、(4) で専用のキー (`msg.search.multiline_too_large` / `msg.search.pattern_too_heavy`) にした。
- v0.3 の「改行を含むパターンは未対応」(`QueryError::Multiline`、`msg.search.newline_unsupported` / `msg.search.replace_newline_unsupported`) は M24 の (4) で無くなった。

**最悪の入力の上限** (2026-11-12 のレビューでの実測に基づく。具体の値は実装の PR で基準機で測って決め、定数で書く。M24 の (2) で初めの値を `crates/eve-core/src/search/limits.rs` に定数で書いた。基準機での確かめと直しは (6)。(6) の基準機の記録 (`docs/perf/0.4.0-m24/`): BM-103b-search-multiline-1m / BM-103b-search-multiline は NFR-103 を満たし、16 MiB の連結は 24〜27 ms、k の大きい検索語も打ち切りにならなかったので、値は変えていない。ただし、重くない検索語で繰り返しの上限が判定の値に近く行をまたぐもの (`[^\n]{0,1000}\nzq`) は、行の長い文書 (行 8〜32 KiB) の 64 KiB 以下の窓の同期検索 (下の「可視範囲の強調」) に p99 6〜10 ms かかった (`{0,100}` は 0.2 ms)。同期検索の条件 (`SEARCH_SYNC_MULTILINE_MAX` と重い検索語の判定の値) を見直すかは未決 (案: 窓の大きさと繰り返しの上限の積で決める)。(2)〜(3) の間は内部の設定で開いたときだけ掛け、(4) で常に掛かるようにした (行単位の経路の正規表現にも捨てた候補の予算を掛ける)。`dfa_size_limit` と重い検索語の判定は、照合器を作るときの設定・解析の結果なので (2) から常に掛かる):

- **取り消しの遅れ**: 検索のジョブ (全件・単発・すべて置換) は、取り消しの遅れを約 50 ms (`SEARCH_CANCEL_LATENCY_MS`、案) 以内にする。取り消しは `regex` の呼び出し (`find_at` / `captures_at`。捨てた候補の分を含む) ごとに確かめる。1 回の呼び出しは途中で止められず、費用は検索語の大きさ × 窓の大きさにほぼ比例する (`regex` の線形時間は検索語の大きさを掛けたもの) ので、核の大きさを直前の窓の処理の速さから縮める (最小は、核を char 境界で切る検索語では最大の長さの 1/4 (窓は「核 + 延ばす分」なので、それより小さくしても 1 回の呼び出しはほとんど短くならず、延ばす分の読み直しで処理の速さだけが落ちる)、行の境界に揃える検索語では 1 行。M24 の (2) の実装: 1 つの窓の処理が目安を超えたら、かかった時間に比べて少なくとも半分に縮め、目安の 1/4 より速ければ元の大きさまで倍にして戻す。時間は `Clock` の Port で測り、時計を渡さない走査は縮めない)。重い検索語 (下) は、核を小さく (16 KiB、`SEARCH_HEAVY_CORE_BYTES`) 始め、目安の 1/4 より速ければ倍にして 1 MiB まで戻す。核を最小まで縮めても 1 つの窓の処理が目安を超える検索語 (重い検索語で、延ばす分 (最大の長さ) だけで遅いもの。例: `[^\n]{0,3000}zq` は 16 KiB の行で 1 つの窓に 80〜250 ms) は、目安を守れず、遅れは窓 1 つ分になる (処理の速さの下限で打ち切る)。上限の無い重い検索語は、全文の連結の 1 回の呼び出しが文書の全体に掛かって取り消せないので、連結に入れず、始める前に打ち切る (`msg.search.pattern_too_heavy`)。**例外**: 重くない上限なしの検索語は、全文の連結の 1 回の呼び出しの間は取り消せない (文書は 16 MiB 以下。呼び出しが終わったところで取り消しを確かめ、取り消しとして返す)。メインスレッドの「時間予算 1 ms を超えたらジョブに回す」も、1 回の呼び出しを止められないので、呼び出す前の条件 (窓の大きさと、検索語が重くないこと) で決める。
- **重い検索語**: 書き換えた検索語の HIR を辿り、上限のある繰り返しの上限 (入れ子は掛け合わせる。上限の無い繰り返しは掛けない) が **1000** (`SEARCH_HEAVY_REPEAT`) を超えるものを「重い」とする。メインスレッドの解析 (構文の解析と書き換え) の段で決まり (`QueryPlan::heavy`)、コンパイルは 10 MiB の上限で 1 回だけ行う。例: `[^\n]{0,3000}zq` (16 KiB の文で 0.4〜0.7 s)、`(?s).{0,10000}zq` (64 KiB の文で 180 s を超えても終わらない)。重くないもの: `\w{10}`・`\d{100}`・`\b\w{5,20}\b`・メールの形の検索語 (M24 の (2) の初めは「小さいコンパイルの上限 (256 KiB) を超えるもの」としたが、Unicode のクラスが大きいので普通の検索語まで重くなり、本当に重い `[^\n]{0,3000}zq` は `\w{100}` よりコンパイルが小さく、大きさの境では分けられなかった)。構文の判定は外れうる (重くないとした検索語が遅い) ので、取り消しの遅れ・処理の速さの下限・捨てた候補の予算を安全網として残す。重い検索語は、メインスレッドで同期検索をしない (可視範囲の強調は保持した一致だけ、次 / 前は単発のジョブ)。1 MiB 以下の文書のすべて置換と、1 件置換の対象の確かめもジョブで行う (上限の無い検索語の 1 件置換の確かめも同じ。下の「呼び出し側」の置換)。
- **処理の速さの下限**: ジョブの処理の速さが下限 (1 MB/s、`SEARCH_MIN_THROUGHPUT`) を下回ったら (核の大きさに依らず、1 秒 (`SEARCH_SLOW_JUDGE_MS`) ごとに、その間に進んだ核の量 (延ばす分の読み直しは数えない) と時間で見る。全文の連結は、始めからの量と時間で見る)、走査を打ち切り、検索バーの件数の位置に `msg.search.pattern_too_heavy` (04 §15.2) を出す (件数は出さない。次 / 前・置換も行わない)。打ち切りは走査の途中で分かるので、それまでに届いた一致 (強調・保持・件数) は捨てる (呼び出し側の処理は M24 の (3) で入れた)。
- **捨てた候補の予算**: 書記素の境界・単語単位で候補を捨てて 1 文字先から探し直すと、O(n²) になりうる (NFD の文書の `(?s)de.*e`、単語単位の `foo.*bar`。256 KiB で約 70 s)。捨てた候補のために `regex` が読んだ量 (探し始めた位置から候補の終端まで) の合計に、走査する範囲の 8 倍 (少なくとも 1 MiB。小さな文書を打ち切らないための下限。`SEARCH_DISCARD_BUDGET_FACTOR` / `_MIN`) の予算を設け、超えたら打ち切って `msg.search.pattern_too_heavy` を出す。メインスレッドの同期検索 (可視範囲の強調・次 / 前の同期) にも同じ予算を掛け、超えたらその場は強調せず、次 / 前は単発のジョブに回す。
- **遅延 DFA の大きさ**: `size_limit` (10 MiB、§12.3) に加え、`dfa_size_limit` (遅延 DFA のキャッシュの大きさ) を明示して設定する (既定の値に頼らない。**8 MiB**、`SEARCH_DFA_SIZE_LIMIT`。`regex` の既定の 2 MiB では、有界の繰り返しを含む普通の検索語でキャッシュがあふれて PikeVM に落ち、100 倍以上遅くなる (M24 の (2) のレビューでの実測: `[^\n\r]{0,1500}zq` は 4 KiB の行の 1 MiB で 2 MiB なら 10.4 s、8 MiB なら 21 ms)。行の経路を含め、常に掛かる)。
- **メインスレッドでのコンパイル**: 打鍵ごとに正規表現をコンパイルすると、メインスレッドで有界の繰り返しの検索語に 4〜13 ms、上限超過の判定まで約 50 ms かかる (実測)。メインスレッドで行うのは、`regex-syntax` での構文の解析と誤りの判定・書き換え・k の計算 (0.03〜0.17 ms) まで。`regex` のコンパイルは `bg` のジョブ (全件ジョブ・単発のジョブの最初の段) で行い (重さの判定は構文の判定なので、解析の段で決まる)、作った照合器は問い合わせの覚え (`Compiled`。複製と共有する `OnceLock`、PRF-09) に入る。メインスレッドは覚えが埋まるまで同期検索をしない (覚えを自分で埋めない)。リテラル検索の memmem の経路は今どおりメインスレッドで作る。大小を区別しないリテラルのうち §12.4 の 3 の経路 (エスケープした `(?i)` の正規表現) も `regex` のコンパイルなので `bg` で作る。覚えが埋まる前は、入力中の現在の一致と可視範囲の強調を出さず (最初のジョブの結果が届いたときに求める)、次 / 前は単発のジョブに回し、すべて置換は文書が小さくてもジョブで作り、1 件置換は何もしない (`Enter` を数えない)。コンパイルで分かる誤り (大きさの上限の超過) は、届いた時点で構文の誤りと同じく検索バーに示す (全件・単発・すべて置換のどのジョブの結果で分かっても示す。照合器を待つ間に `Enter` で単発のジョブへ移った後でも、0 件とは出さない)。
- **`bg` レーンの順序と取り消し**: M24 の (3) で、単発の次 / 前 (`FindNext`) とすべて置換 (`ReplaceAll`) を `io` から `bg` へ移し、1 件置換の確かめ (`ReplaceOne`) も `bg` に置いた (それまでの実装は `Search` だけが `bg`、02 §9.3。k が 1 以上では単発のジョブが文書の先頭から走ることがあり、`io` に置くと保存を待たせる)。`bg` は 1 本の先着順 (OQ-198 の優先は提案のまま入れていない) で、走っている全件ジョブは取り消すまで次のジョブを待たせる (100 MB の正規表現の全件の走査は秒の単位)。そこで **単発の次 / 前・すべて置換・1 件置換の確かめを投げるときは、走っている全件ジョブを取り消し、そのジョブを投げた後に同じ条件の全件ジョブを投げ直す** (k が 0 の既定の経路を含む)。待ちは全件ジョブの取り消しの遅れ (取り消しは行・チャンク・`regex` の呼び出しごとに確かめるので、普通は数 ms。行の窓は約 50 ms 以内) だけになり、全件の走査はやり直しになる (件数は走査中の表示に戻る)。すべて置換の後は、適用の後の再検索で同じ結果になる。可視範囲の強調のジョブは作らない (下の「呼び出し側」)。取り消しの相手: 新しい全件ジョブは、古い全件と古い単発を取り消す。新しい単発は、同じ (Revision, 起点, 向き) の単発が走っていれば取り消さない (下の「次 / 前」の連打)。すべて置換と 1 件置換の確かめは、検索バーを閉じる・検索語・トグル・置換の文字列を変えると取り消す。

**呼び出し側** (§12.2 / §12.4 / §12.5 の規則のうち、行をまたぎうるパターン (k ≥ 1 と上限なし) で変わるもの。k が 0 のパターンは v0.3 の規則のまま。ただし **k が 0 の重い検索語** (下の「重い検索語」) は、走査の順は v0.3 のまま (カーソルの行から末尾まで、続けて先頭から) で、メインスレッドの同期検索だけをしない: 入力中の現在の一致は全件ジョブの `first_after` (k が 0 の順でも、起点以降の最初の一致を送る) で決め、次 / 前は保持した一致 (全件を持つとき) か単発のジョブ、可視範囲は届いた保持した一致だけ、1 件置換の確かめはジョブ (下の「置換」と同じ形。選び始める位置は対象の始端を含む行の行頭)):

- **全件走査**: カーソルの行からではなく、**文書の先頭から** 順に走査する (正本のため)。カーソル以降の最初の一致は、見つけた時点で `first_after` (案、02 §9.3) として、保持の上限 (`max_matches`) に依らず送る。届いたら v0.3 のインクリメンタルと同じく、そこを現在の一致にしてスクロールする (投入した後にカーソル・選択を動かしていたら、強調だけ)。NFR-108 / 109 は正規表現に課さない (§12.1) ので、この遅れは目標に掛からない。保持は走査した順 (= 文書の順) の最初の `max_matches` 件 (§12.2 の v0.1 の実装の範囲と同じ)。ジョブは 1 MiB ごとに **再開の位置** (正本の走査がそこから続く位置: その時点の探し始める位置 = 直前に採った一致の終端か核の始端) を記録して `SearchDone` で返し (100 MB で約 100 項目)、main は持つ。編集したときは、編集した行の (k + 1) 行より前の記録を有効のまま残し、後ろを捨てる (そこまでの正本の走査は、編集した範囲を読まない)。残すのは、記録が編集の前の文書 (編集の前の Revision) のものであるときだけ (文書の差し替え・読み直しの後の 1 回目の編集で、前の文書の位置を有効に戻さない)。入力中の現在の一致は編集で消す (編集の前の文書の一致は正本と限らない。次の再検索で求め直す)。
- **次 / 前** (`search.next` / `search.prev`。OQ-218 の 3、★ 厳密): (1) **今の一致 (選択中の一致・入力中の現在の一致) が正本の一致なら、`search.next` はその終端から探す (Must)**。その終端は正本の再開の位置。(2) 前向きは、保持した一致 (編集で無効になっていない `Exact` / `AtLeast` のもの) に移動先があればそれへ移る。後ろ向きと折り返し (端を越えて反対側から続ける) は、保持が文書の全部のとき (`truncated` でなく、件数の上限で打ち切っていない) だけ保持を使う。(3) それ以外は、同期検索 (256 KiB) をせずに単発のジョブ (`FindNext`) で探す。前向きは今の一致の終端 (無ければ起点より前の最も近い再開の位置、それも無ければ文書の先頭) から走査して起点以降に始まる最初の一致を、後ろ向きは起点より前の最も近い再開の位置から起点まで走査して起点より前で終わる最後の一致を採り、無ければ 1 つ前の再開の位置から探し直す。(4) 全件ジョブが走っている間は単発のジョブを投げず、全件ジョブの結果 (`first_after` と保持) を待って移る。(5) 単発のジョブ (か (4) の待ち) が走っている間に、同じ (Revision, 起点, 向き) の `search.next` / `search.prev` を押したら、取り消さずに押した回数を数え、結果が届いて移った後に、残りの回数だけ移った一致の終端 (後ろ向きは始端) から続ける (連打をまとめる)。起点が違う (待つ間にカーソル・選択を動かした) か向きが違うときは、待っているものを取り消して今の起点から投げ直す。(6) 待ちが約 100 ms を過ぎたら、件数の位置を `…` にする (`ui.search.count.running` と同じ形。新しいキーは足さない)。`Esc`・検索語の入力・トグル・編集 (Revision が変わる) で取り消し、届いた結果は Revision と選択が投入の時のままのときだけ使う (§12.2 と同じ)。S-04 の「窓で近似」(`from` の w 行前から探す。乱数のパターンで約 2.5% が厳密と食い違った) は採らない。
- **入力中の現在の一致** (インクリメンタル、§12.2): 同期検索をせず、全件ジョブの `first_after` で決める。起点以降に一致が無ければ、数え終えた時点で文書の最初の一致 (折り返し) にする。編集の前の文書を走査したジョブの `first_after` は採らない。編集の後 (再検索の前) は、入力中の現在の一致を使わない (選ばない・置き換えない・正本の一致として記録しない)。
- **可視範囲の強調**: 全件ジョブが終わり (`Exact` / `AtLeast`)、保持した一致が可視行を覆うときは、それを使う。それ以外は、検索語が重くなく、窓 (可視行の先頭の k 行前の行頭から可視行の末尾までを核にした窓の全体) が 64 KiB (`SEARCH_SYNC_MULTILINE_MAX`、定数。BM-103b-search-multiline-1m を見て実装の PR で直してよい) 以下のときだけ、メインスレッドで窓を同期検索し、可視行と交わる一致を強調する。条件に合わないとき・上限なしのパターンでは、同期検索をせず、届いている保持した一致だけを強調する (強調のためのジョブは作らない)。k 行前から始める同期検索は、正本と食い違うことがある (上の `a\na` の例)。食い違うのは強調だけで、件数・移動・すべて置換は正本に従う。
- **単語単位** (`whole_word`、§12.4): 一致の始端はそれを含む行の、終端はそれを含む行の単語境界 (§13.3) で調べる (終端が行頭なら境界)。v0.3 の「行の文脈で調べる」と同じ規則を、両端の行のそれぞれに当てる。行の文脈は窓 (上限なしは連結した全文) から取る (単語単位では、窓を核の始端を含む行の行頭から作り、延ばす量に最大の長さを使わない。両端の行の全体が窓に入る)。
- **選択範囲内** (`scope`、§12.5): 一致は文書全体の正本から選び、範囲のどれか 1 つに収まるものだけを採る (範囲の境界をまたぐ一致は採らない。v0.3 と同じ)。窓は範囲の外 (後ろの k 行と前後の文脈) も読む。範囲の端は行の境界でなくてよい (S-04 の 6)。
- **置換** (§12.5): すべて置換は正本の一致を置き換える (Snapshot を文書の先頭から走査する。今と同じ)。キャプチャ (`$1` / `${name}`) は窓 (上限なしは連結した全文) の上で取り、キャプチャに入った行の終端は文書の種類のまま入れる (改行の正規化はしない。置換の文字列の文字どおりの `\n` を `default_eol` にするのは今と同じ)。**1 件置換の対象の確かめ方**: k が 0 のパターンは今の規則のまま (選択範囲の行を正本どおりに探す)。k が 1 以上・上限なしのパターン (と k が 0 の重い検索語) は正本で確かめる。**正本と分かっている一致** は、保持した一致 (編集で無効になっていないもの)・入力中の現在の一致 (編集の後でない新しいもの)・次 / 前で移った一致 (編集でその位置が無効になっていないもの) の 3 つ。(1) 選択範囲がそのどれかと同じなら、それを置き換える。(2) そうでなく、新しい入力中の現在の一致があれば、それを置き換える (v0.3 と同じ)。(3) どちらも無ければ置き換えず、次の一致を探す (`search.next` と同じ。届いたら選び、もう一度 `Enter` で置き換える)。置き換える前に、対象の始端から走査の入口で選んだ最初の一致が対象そのものであることを確かめ、そのキャプチャを使う (正本の一致は始まる位置で決まるので、その位置から探すと必ず最初に出る。窓は走査と同じ作り方で、`$`・`\z`・`\b` が窓の端で偽って成り立たない)。この確かめをメインスレッドで行うのは、重くなく行の数に上限のある検索語で、取り直しの窓 (対象の始端から延ばす分の終わりまで) が 64 KiB (`SEARCH_SYNC_MULTILINE_MAX`) 以下のときだけ。**重い検索語・上限の無い検索語・それより大きな窓は、ジョブ (`ReplaceOne`、02 §9.3) で確かめる** (一致 1 件が 16 MiB 近くになりうる。重い検索語の照合はメインスレッドを止めうる)。**待っている間の `Enter`**: 次の一致 (単発のジョブ・(4) の全件ジョブの待ち)・入力中の現在の一致・確かめのジョブのどれかを待っている間に置換欄で `Enter` を押したときは、置き換えず何もしない (照合器を待つ間と同じ。押した回数を数えない。1 回の `Enter` で 2 件を置き換えない。飛ばしたつもりの、まだ選ばれている一致を置き換えない。件数の位置の `…` が待ちを示す、04 §8.4)。すべて置換と 1 件置換の確かめのジョブは、検索バーを閉じる・検索語・トグル・置換の文字列を変えると取り消す (古い条件の結果を当てない)。**すべて置換の知らせの行数** (M24 の (4)。(3) のデータの損失の視点のレビューの推す案): 件数 (`info.search.replaced` の `{count}`) に加えて、**置換の前後の文書の行数 (`len_lines`) の差** を言う。減ったときは `info.search.replaced_lines_removed`、増えたときは `info.search.replaced_lines_added` (`{count}`・`{lines}`。04 §15.2)、変わらないときは今までの `info.search.replaced`。差を選んだ理由: 「文書が N 行減った」は置換の後に画面で確かめられる量で、上限の無い検索語 (`(?s)1.*` など) が文書の残りを 1 件として消したことが、そのまま行数で分かる。退けた案「置換の範囲が含んでいた行の終端の数」は、行をまたがない置換で 0 になり (常に出すと「0 行」が並ぶ)、改行を改行に置き換える置換 (LF を CRLF に、など) では行数が変わらないのに大きな数を言い、利用者が数の意味を取りにくい。差は増減を打ち消し合う (消した行と足した行が同じなら言わない) が、言うのは置き換えた結果の文書の形なので、元に戻す (Undo) かを決める手がかりとしてはこれで足りる。CP932 互換文字の同一視の件数を分ける知らせ (`info.search.replaced_folded`、§12.4.1) と組み合わせる形は M24 の (5) で決めた: 行数が変わり同一視でも当たったときは、1 つの知らせ (`info.search.replaced_folded_lines_removed` / `_lines_added`) に「件数 → 行数 → 同一視の件数」の順でまとめる (理由は §12.4.1 の「置換」)。置換も検索と同じく走査の入口 (`scan_bytes`) を通す (`scan_replacements` は M24 の (1) で入口に寄せた。§12.3 の「走査の境界」)。一致した文字列と置換の文字列 (展開した後) が同じ一致は、すべて置換でも 1 件置換でも置き換えない (v0.4 の M24 の (5)、§12.5)。
- **置換で CR と LF がつながる場合** (既知の制約): 置換で単独の CR と単独の LF が隣り合うと 1 つの CRLF になり、行の数が変わる (例: `a\rX\nb` の `X` を消す)。これを直す `eol_guard` (§6.2、OQ-194 は提案中) は今の実装に無い。M24 では直さず、既知の制約とする (行をまたぐ置換で起きやすくなる。OQ-194 に書き足した)。

**型と状態の持ち主** (02 §9.3 に合わせる):

- `SearchQuery` に `newline_sugar: bool` (糖衣) と `compat_fold: bool` (§12.4.1) を足す。どちらも検索バーを開いたときの設定 (`search.newline_sugar` / `search.cp932_compat_fold`) の値で作り、設定の変更は次に検索バーを開いたときから効く。
- 照合器 (`Matcher`) は、文書に依らない結果だけを持つ: k (`Bounded(k)` / `Unbounded`)、窓を延ばす量の元 (最大の長さ)、重いか。コンパイルの覚え (`Compiled`) に文書の大きさを混ぜない。
- 16 MiB の判定は、検索を始めるときに eve-core の検索バーの状態 (`SearchBarState`) が文書の大きさと照らして行い、「探さない理由」として持つ (構文の誤り `QueryError` とは別の項目。重すぎる打ち切りも同じ項目)。検索バーの `SearchBarModel.error` (02 §8.2、`CellText`) には、構文の誤り・探さない理由・重すぎる打ち切りのどれかの text を入れる (境界の型は変えない)。
- ジョブの結果: `SearchBatch` に `first_after: Option<ByteRange>` (カーソル以降の最初の一致。k ≥ 1 で送る)、`SearchDone` に `resume: Vec<ByteOff>` (1 MiB ごとの再開の位置) と打ち切りの理由 (`limit: Option<ScanLimit>`。探さない (`TooLarge`)・重すぎる (`TooHeavy`)) を足す。`FindNext` に走査を始める位置 (`resume: Vec<ByteOff>`。再開の位置と今の一致の終端。後ろ向きは起点より前の位置を近い順に試すので、1 つではなく並びで渡す) を足す。1 件置換の確かめのジョブ `ReplaceOne { from, range, replacement }` (結果は `ReplaceOneDone { insert: Option<String> }`) を足す。`FindNext`・`ReplaceAll`・`ReplaceOne` が探さなかった・打ち切ったときは `SearchLimited { limit }` を返す。連打の回数は main が数え、ジョブには持たせない。可視範囲の強調の `JobKind` は足さない (M24 の (3) で入れた。02 §9.3)。

**v0.3 からの振る舞いの変化** (CHANGELOG とリリースノートの「振る舞いの変化」に書く。08 §2.5 の M24 の (4) / (5)):

- `\n` を含むパターン: 「未対応」→ 行をまたいで一致する。`\x0A` などで書いた行の終端: 黙って 0 件 → 一致する。`[^\n]`: 「未対応」→ 1 行の中で一致する。
- `(?s)` を含むパターン: 1 行の中 → 行をまたぐ。`(?s)a.*b` は上限なしになり、16 MiB を超える文書では探さない (OQ-218 の 1・2)。
- `\A` / `\z`、`(?-m)` の `^` / `$`: 各行の先頭 / 末尾 → 文書の先頭 / 末尾 (OQ-218 の 4)。`(?-R)` の `^` / `$` (利用者が CRLF モードを外したもの): 各行の先頭 / 末尾 → LF の後ろ / 前と文書の先頭 / 末尾 (CR・CRLF の行の終端の前後では一致しない。`regex` の `(?-R)` の意味どおり)。
- `search.newline_sugar` を公開する (07 §5.5)。`[search]` は公開キーを持つセクションになるので、`[search]` に書いた公開前のキー (`case` など) の「このバージョンでは未対応」の情報診断は、セクション単位の 1 件からキーごとになる (M24 の (4))。
- CP932 互換文字の同一視が既定 ON になり、リテラル検索と正規表現の結果が変わる (`〜` で `～` にも当たる。§12.4.1)。
- 正規表現のコンパイルを `bg` へ移すので、正規表現と、大小を区別しないリテラルのうち §12.4 の 3 の経路 (`k` `K` `s` `S` か ASCII 以外の区別のある文字を含む語と、大小を区別しない **単語単位** の語。例: Smart の `foo` で単語単位) の強調と現在の一致は、最初のジョブの結果まで (数 ms) 出ない。大きすぎる正規表現の誤りも、そのとき (`Enter` で移った後でも) 検索バーに出る。
- 照合器を待つ間 (最初のジョブの結果まで) の置換欄の `Enter` (1 件置換) は何もしない。すべて置換は、文書が小さくてもジョブで作る (結果は同じ)。
- 書き換えで照合器が大きくなるので、コンパイルの大きさの上限 (10 MiB、§12.3) に掛かる境が下がる (非常に大きな繰り返しの検索語で「大きすぎる」と出る境が少し下がる。M24 の (1) での実測: `(?s).{0,N}` は N = 10443 → 8428、`(?s).{N}` は 10877 → 8708、`[^a]{N}` は 10485 → 9781)。

**検索語の入力**: 検索欄は 1 行のまま (§12.3。貼り付けは最初の行だけ、04 §6.6)。複数行のパターンは `\n` で書く。リテラル検索は行をまたがない (改行を入力できない)。

**性能と NFR-103**: 正規表現の所要時間は参考値 (NFR-108 / 109 を課さない、§12.1)。BM-109m (09 §5.2) で記録する。入力を止めない (NFR-103) ことを Must とし、M24 の実装の後に BM-103b-search-multiline-1m / BM-103b-search-multiline (09 §5.2) で測る (S-04 は代わりの目安だけを測った)。メインスレッドで動くのは、構文の解析と書き換えと、可視範囲の強調の同期検索 (上の条件の内) だけ。

**実装の PR の分け方**: 正本は 08 §2.5 の M24 の行。

### 12.4 単語単位・大文字小文字

- `whole_word` (v0.2): 一致の両端が [§13.3](#133-単語境界-日本語補助規則) の単語境界であることを後段で検査する (regex の `\b` は ASCII/Unicode の `\w` 基準で日本語に不適)。行をまたぐ一致 (v0.4) は、両端をそれぞれの行の文脈で調べる (§12.3.1)。
- `Insensitive`: 意味は **Unicode の単純ケースフォールディング** (CaseFolding.txt の C と S。regex の `(?i)` と同じ) で一致する、と定める。リテラル検索の経路は次の順に選ぶ (v0.1 から。利用者に正規表現は見せない)。どの経路でも結果はこの意味と同じになる。
  1. **大小の区別が無い文字だけの検索語** (どの文字も単純ケースフォールディングの対応表に写像元としても写像先としても現れない。ひらがな・カタカナ・漢字・記号・数字だけの日本語の検索語はこれに当たる) は、`Insensitive` / `Smart` でも **`Sensitive` と同じ** で、`memmem` で検索する (NFR-108 / 109 の目標の対象)。全角英字 (`Ａ` / `ａ`) は区別のある文字なのでこれに当たらない。
  2. **ASCII だけの検索語で `k` `K` `s` `S` を含まない** ものは、ASCII の大小を畳み込んだ `memmem` (ASCII 大小無視の走査) で検索する (NFR-108 / 109 の目標の対象)。
  3. それ以外 (ASCII 以外の区別のある文字を含む、または `k` `K` `s` `S` を含む) は、パターンをエスケープして行単位の正規表現経路 (§12.3) で `(?i)` として検索する。`k` / `s` を 2 の経路から外すのは、単純ケースフォールディングでは `K` (U+212A KELVIN SIGN) が `k` に、`ſ` (U+017F LATIN SMALL LETTER LONG S) が `s` に畳み込まれ、ASCII の畳み込みではこれらに一致しないため (ASCII 以外から ASCII へ畳み込まれる文字はこの 2 つだけ)。この経路の所要時間は正規表現と同じく参考値とし、入力を止めないこと (FR-404) だけを Must とする (提案、OQ-196。代案は「ASCII だけの検索語では ASCII の大小だけを同一視し、K と ſ には一致しない」と意味を定めて 2 の経路を常に使う。速いが、ASCII 以外を含む検索語との間で意味が揃わない)。
- **v0.1 の実装** は `regex` を使わず、大小を区別しない照合を「1 文字ずつ小文字へ畳み込んだ文字列どうし」で行う (小文字が 1 文字になる文字だけ畳み込む)。上の 1・2 の経路は仕様どおり。3 の経路は、単純ケースフォールディングとの差が `ſ` (U+017F) と `s` を同一視しない点だけ (K (U+212A) は `k` に一致する)。`regex` を入れる v0.2 で仕様の意味に揃える (OQ-196 の判断と合わせる)。v0.4 で (M24 とは別の修正として) 揃えた: 3 の経路はエスケープした検索語を `regex` の `(?i)` で探し (単語単位の大小無視も同じ)、2 の経路から `s` `S` も外したので、`ſ` も `s` `S` に一致する。
- 全角/半角、ひらがな/カタカナの同一視は **行わない**。NFC/NFD 正規化差も同一視しない ([§13.5](#135-正規化と-bidi))。CP932 互換文字の同一視 (OQ-41) は v0.4 で、規則は §12.4.1。

#### 12.4.1 CP932 互換文字の同一視 (v0.4、M24、OQ-41)

OQ-41 (確定) の細目。M24 の仕様の PR で決めた (2026-11-12 に案)。利用者に見える点は [OQ-218](10-decisions.md#29-oq-176-以降-並行修正での追加起票) の 5〜8 で PO が決めた (確定 (ユーザー回答、2026-11-12): すべて推す案 (★) のとおり)。本節はその内容 (★ は決めた案の印)。**M24 の (5) で実装した** (`crates/eve-core/src/search/rewrite.rs` の書き換えと `matcher.rs` のリテラルの経路。試験は `crates/eve-core/tests/search_compat_fold.rs` (09 §4 のコーパスを含む)・`fuzz_regex_search`・ヘッドレスのシナリオ `tests/scenarios/search/compat_fold_*.toml`)。

- **対象**: **8 組** (U+301C `〜` / U+FF5E `～`、U+2016 `‖` / U+2225 `∥`、U+2212 `−` / U+FF0D `－`、U+00A2 `¢` / U+FFE0 `￠`、U+00A3 `£` / U+FFE1 `￡`、U+00AC `¬` / U+FFE2 `￢`、U+00A6 `¦` / U+FFE4 `￤`、U+2014 `—` / U+2015 `―`)。組は対称で、片方で探すともう片方にも一致する。**`¥` (U+00A5) / `￥` (U+FFE5) と `‾` (U+203E) / `￣` (U+FFE3) は含めない** (OQ-218 の 8、★): 文書の `¥` / `‾` の多くは JIS X 0201 のローマ字の 0x5C / 0x7E から来る半角の字で (JIS X 0208 の「￥」(0x216F) は CP932 でも JIS 系の写像でも U+FFE5 で、写像の違いは起きない)、この 2 組は「同じ字の写像の違い」ではなく「半角と全角」に近く、§12.4 の「全角と半角を同一視しない」と食い違う。保存で表現できない `¥` / `‾` は、E3 が位置を示して置き換える (§8.6)。U+005C `\` と U+007E `~` も含めない。ほかの全角 / 半角・NFC / NFD は同一視しない (§12.4)。
- **§8.6 の互換文字の表との関係**: 保存の E3 の互換文字の表 (OQ-208。`¥` / `‾` を含む 10 行) は変えない。検索の同一視の組 (上の 8 組) は、その表から `¥` / `‾` の 2 行を除いたもので、2 つは別の表になる。OQ-41 の決定 (「対象は互換文字の表と同じ組」) の文言は変えず、`¥` / `‾` を除くことは OQ-218 の 8 の決定として扱う。
- **設定**: `search.cp932_compat_fold` (bool、既定 `true`、07 §5.5)。設定の変更は、次に検索バーを開いたときから効く。文書の文字コードに依らず (UTF-8 の文書にも) 効く。文書の本文と保存するバイト列は変えない (P1)。
- **検索バーの表示** (OQ-218 の 7、★): 同一視が効いているとき (設定が ON で、検索語 (リテラル検索では検索語、正規表現ではリテラルの文字とクラス) が組の文字を含むとき) だけ、件数の後ろに印を付ける (04 §8.4。core が作る件数の文字列 `count_text` に足す。境界の型 `SearchBarModel` は変えない)。トグルと切り替えるコマンドは作らない。**「組の文字を含む」の細目** (M24 の (5)): リテラル検索では、検索語に組の文字が 1 つでもあれば付ける。正規表現では、書き換えで一致する文字が変わるときだけ付ける: リテラルの互換文字 (符号位置で書いたものを含む) と、組の片方だけを含むクラス (`[〜]`・`[^〜]`・`\p{Sm}`・`[\x{2010}-\x{2014}]` など)。組の両方を含む (か、どちらも含まない) クラス (`\S`・`\W`・`.`・`[^a]`・`[〜～]`) は同一視しても一致が変わらないので付けない (`\S+` のようなよくある検索語のたびに印が出ると、印が「互換文字に一致しうる」の意味になり、効いているかが読めなくなるため)。件数がどの状態でも (走査中・`0 件`・`約 N` を含む) 付ける (0 件でも、同一視して探したことが分かる)。判定は解析の段 (`QueryPlan::compat_folded`) で決まるので、照合器を待たずに出せる。注: 書き換えは要素ごとなので、選択 `〜|～` には付き (リテラルの互換文字)、クラス `[〜～]` には付かない (一致する文字は同じ)。どちらも同一視で一致は変わらないので、害は無いとして直さない。**改訂 (ユーザー回答、2026-11-13。OQ-218 の 7 の改訂)**: **今の一致が同一視で当たったものなら、印を「互換一致」にする** (`ui.search.count.compat_match_suffix`。例 `2/5 互換一致`。en "compat match")。今の一致は件数の番号を出す一致 (入力中の現在の一致か、保持した一致と同じ選択)。「同一視で当たった」の判定は下の「置換」の定義と同じ (同一視を切った検索語が、文書の前後の文脈のまま、その範囲にちょうど一致できない)。次へで 1 件ずつ見て、片方の文字だけを 1 件置換で直せる。判定はメインスレッドで、同一視を切った照合器が全件のジョブ (`bg`) で作られた後、重くない検索語の 64 KiB (`SEARCH_SYNC_MULTILINE_MAX`) 以下の一致だけで行い (照合器を作らない。覚えて、同じ一致では走らせ直さない)、それ以外は「互換」のまま出す。
- **リテラル検索**: 検索語の互換文字は、組の両方に一致する。一致の範囲は文書の元のバイト範囲 (UTF-8 の長さが違う組がある。`¢` は 2 バイト、`￠` は 3 バイト)。検索語が互換文字を含まないときは、今の経路 (memmem など、§12.4) のまま。含むときは、検索語をエスケープし、互換文字を 2 文字のクラスにした正規表現を、行単位の経路ではなく **行の窓の経路** (§12.3.1。k は 0) で探す (大小を区別しない語は `(?i)` を付ける。§12.4 の 3 の経路と同じ意味)。照合器は `regex` のコンパイルなので、§12.4 の 3 の経路と同じく `bg` のジョブが作る (§12.3.1 の「メインスレッドでのコンパイル」。強調と入力中の現在の一致は最初のジョブの結果から。数 ms)。走査の順と呼び出し側の規則は k が 0 の検索語のまま (カーソルの行から。§12.3.1 の「呼び出し側」は掛からない)。この経路でも 100 MB で 2〜28 ms (memmem と同じ程度。2026-11-12 のレビューでの実測) なので、**NFR-108 / 109 の目標を外さない** (BM-109-compat をこの目標で記録する)。可視範囲の強調は、検索語の 1 文字が文書で最大 3 バイトなので、今と同じく範囲を広げる量が決まり、同期で強調できる (§12.2 の「可視範囲のハイライト」の「パターンのバイト長」を、互換文字 1 つにつき 3 バイトで数える)。
- **正規表現** (OQ-218 の 5、★): `(?i)` が大小に掛かるのと同じ形で、リテラルの文字とクラスの両方に掛ける。リテラルの互換文字は 2 文字のクラスにする (`〜` は `[〜～]`)。クラスは、組の片方を含めばもう片方も含める (クラスが組の文字を含むかは HIR で調べ、相手の文字を要素として足す)。否定のクラスは、否定する前の集合に掛けてから否定する (`[^〜]` は `〜` にも `～` にも一致しない)。符号位置で書いた文字 (`\x{301C}`) と Unicode のクラス (`\p{..}`) にも掛ける (区別して探したいときは、設定を `false` にする)。書き換えは §12.3.1 と同じく AST の段で行う。行の終端は対象の組に無いので、§12.3.1 の k を変えない。**書き換えの細目** (M24 の (5)): 掛け方は `regex-syntax` が `(?i)` を掛けるのと同じく **クラスの要素ごと** で、要素 (1 文字・範囲・`\p{..}`) の集合が組の片方だけを含めば相手を要素に足し、否定の要素 (`\P{..}`) は相手を含まない側を差し引き (`[\P{Sm}--[〜－‖]]`)、入れ子のクラスの否定と集合の演算 (`&&`・`--`・`~~`) はその後に掛かる。略記のクラス (`\d`・`\s`・`\w` とその否定) は組の文字をどちらも含まない (否定はどちらも含む) ので変えない。書き換えても一致する文字が変わらないクラス (`[〜～]`) は元のまま残す。`(?-u)` の範囲 (バイトの意味で探す) には掛けない。表の書き方ごとの結果は `crates/eve-core/tests/golden/search_rewrite_compat.tsv` (同一視を切ったときの k を並べ、同一視が k を変えないことを確かめる)。
- **大小区別との関係**: 互換文字はどれも大小の区別が無いので、`Smart` の判定 (§12.1) に影響しない。`Sensitive` / `Insensitive` のどちらでも、同一視は同じに掛かる。
- **単語単位**: 同一視は単語境界 (§13.3) の判定を変えない (一致の両端は、文書の元の文字で調べる)。
- **置換** (OQ-218 の 6、★): 置換の文字列は書いたまま入れる (同一視しない)。一致は検索と同じ (同一視する)。`$0` / `$1` などは文書の元の文字を運ぶ。反対の論点 (PO に示す): **片方の文字だけを置き換えるつもりの操作で、もう片方も書き換わる** (`—` を `--` にすると `―` も、`−` を `-` にすると `－` も変わる)。`〜` を `～` に揃える すべて置換は、もとから `～` だった箇所にも一致するが、一致した文字列と置換の文字列が同じなので置き換えず、件数にも数えない (改訂 (ユーザー回答、2026-11-13)。§12.5)。片方の文字だけを直したいときは、件数の後ろの「互換一致」の印 (上の「検索バーの表示」) で今の一致を見分け、次へと 1 件置換で直す。これを知らせるため、すべて置換の結果の知らせは、同一視で当たった件数を分けて言う (`info.search.replaced_folded`、04 §15.2。同一視で当たった件数が 0 なら今の `info.search.replaced` とその行数の版)。「同一視で当たった」は、**同一視を切った検索語が、文書の前後の文脈のまま、その範囲にちょうど一致できない一致** (すべて置換のジョブで一致ごとに確かめる。改訂 (ユーザー回答、2026-11-13): 前の「同一視を切った検索語の最初の一致と比べる」は、選択の優先 (`〜|～cc|[〜～]` の `～` は 3 つ目の選択肢で一致できる) と、窓の端での `$`・`\b`・`\z` の偽の成立 (`—(?:\d+\b)?` と `—2nd`) で食い違ったので、この定義に揃えた)。**確かめ方**: 同一視を切って書き換えた検索語 (同じ大小の扱い・糖衣) から、`regex-automata` の PikeVM を「すべての一致」(`MatchKind::All`) で作り、一致の始端に固定して (`Anchored::Yes`)、探す範囲をその一致の範囲に限って探す。範囲の終端で終わる一致があれば、それが最も長い一致として返るので、返った一致の終端が範囲の終端でなければ「同一視で当たった」に数える。探す文字列は一致の前後 1 文字を付けたもので、範囲の外の文字は `^`・`$`・`\b` の判定にだけ使われ (`regex-automata` の範囲つきの検索の約束)、文書の端はそのまま端なので、文書の上で探すのと同じになる。`regex` の API には、範囲の終端の文脈を保ったまま始端に固定して探す口が無いので、`regex-automata` (`regex` の依存として既に入っている crate。02 §13.5) を直接使う。同一視を切った照合器は、全件の検索のジョブとすべて置換で初めて作る (メインスレッドでは作らない)。同一視が効いていない検索語 (上の「検索バーの表示」の印が付かないもの) では確かめず 0 とする。**一致した文字列と置換の文字列が同じ一致は置き換えない** (改訂 (ユーザー回答、2026-11-13)。§12.5): `〜` を `～` に揃える すべて置換で、もとから `～` だった箇所は編集を作らず、件数にも数えない (同一視で当たった件数にも数えない)。全部がそうなら文書は変わらず (未保存にならない)、`info.search.replace_no_match` を出す。**行数の知らせとの組み合わせ** (§12.3.1 の「置換」の行数。M24 の (5) で決めた): 行数が変わり、同一視でも当たったときは、1 つの知らせにまとめる (`info.search.replaced_folded_lines_removed` / `_lines_added`。`{count}`・`{lines}`・`{folded}`。同一視の件数は「(互換文字の同一視で一致したのは {folded} 件。元に戻すで戻せます)」と文の最後に言う。「うち」は行数を指すように読めるので使わない)。文の順は「件数 → 行数 → 同一視の件数 → 元に戻す案内」(行数の変わらない `info.search.replaced_folded` は「件数 → 同一視の件数 → 元に戻す案内」)。理由: メッセージ行は最新の 1 件だけを出すので、2 つに分けると先の知らせ (行数。データの損失に関わる) が後の知らせに隠れる。メッセージ行は幅が足りないと末尾から切る (04 §8.9。全文はメッセージ履歴に残る) ので、大事なものから並べれば、80 桁で切れても件数と行数が残る (`info.*` は hint を持たない (07 §17.2) ので、元に戻す案内は文の最後に置く)。退けた案: 同一視の件数の知らせを行数の知らせの後に続けて出す (すべて置換の私用文字の知らせ `info.paste.pua_as_text` と同じ出し方。行数が隠れる)、同一視の件数だけを言う (行数が分からない)。1 件置換は、件数の後ろの印 (上の「検索バーの表示」。今の一致が同一視で当たったものなら「互換一致」) と、置き換える前の一致の強調で足りるとし、別の知らせを出さない。
- **エスケープ文字** (`U+10FF00..`、§8.3.1) は対象の組に無いので、同一視の影響を受けない。
- **型**: `SearchQuery` に `compat_fold: bool` を足す (`with_compat_fold`。既定は `true`。§12.1 の OQ-41 の準備のとおり `#[non_exhaustive]` なので、呼び出し側を壊さない)。検索バーは、開いたときの設定の値でこれを作る。解析の結果 `QueryPlan::compat_folded` が「効いているか」(件数の印)。すべて置換の結果 (`build_replace_all_in` の `ReplaceAllBuilt`、ジョブの `ReplaceAllDone`) に同一視で当たった件数 `folded` を足す (02 §9.3)。同一視を切った照合器は `Matcher` が持ち (`Matcher::fold_check`。`bg` で作る。メインスレッドは作られた後だけ `fold_check_ready` で使う)。
- **検証**: 09 §4 のコーパスに、8 組の両側を並べた UTF-8 の文書と、CP932 の側を並べた Shift_JIS の文書を置き、リテラル・正規表現・大小区別の 3 値・単語単位・選択範囲内・置換の組で確かめる (09 §3.2 の検索の行、`fuzz_regex_search` にも同一視を入れる)。`¥` / `￥`・`‾` / `￣`・`\` / `¥` を並べた行 (UTF-8 の文書) で、同一視しないことも確かめる。

### 12.5 置換 (FR-403、v0.2)

**v0.1 はリテラルの「すべて置換」だけを実装する** (確定 (ユーザー回答)。FR-403 の v0.1 の範囲)。`search.replace_open` (置換欄を開く) と `search.replace_all` を実装し、1 件置換 `search.replace`・正規表現の置換文字列の展開・選択範囲内 (`SearchQuery.scope`) は v0.2 で、CommandId と型を予約する。手順は下表で固定し、表の「1 件置換」「選択範囲内」と正規表現の行は v0.2 の規則である。

**v0.2 の追加** (M9a。下の v0.1 の手順に重ねる):

- **一致した文字列と置換の文字列が同じ一致は置き換えない** (v0.4 の M24 の (5)。改訂 (ユーザー回答、2026-11-13)、OQ-218 の 6 の改訂): すべて置換では、置換の文字列 (正規表現では `$1` などを展開し、`\n` を改行にした後) が一致した文字列とバイトで同じ一致は、編集を作らず、件数にも数えない。全部がそうなら文書は変わらず (未保存にならない。Undo の単位も作らない)、`info.search.replace_no_match` を出す。1 件置換で同じなら、置き換えずに (文書を変えずに) カーソルをその一致の後ろへ置き、次の一致を強調する (置き換えたときと同じ進み方)。同一視 (§12.4.1) に限らず、普通の置換にも効く (`a` を `a` に置き換える、`(\w+)` を `$1` に置き換えるなど)。

- **すべて置換の問い合わせ**: 検索バーの現在の `SearchQuery` をそのまま使う (大小区別・正規表現・単語単位。下の手順 2 の「`kind = Literal`・`whole_word = false`」は v0.1 の定数で、v0.2 は検索バーのトグルに従う)。選択範囲内 (`scope`) は未実装。
- **置換文字列**: リテラルでは従来どおり展開しない。**正規表現では** `$1` / `${name}` を一致ごとに展開し (`regex` crate の構文。`$$` は `$`)、`\n` は文書の既定の改行 (`default_eol`) に、`\\` は `\` 1 つに直す (ほかの `\` はそのまま)。存在しないグループは空文字列になる。
- **大量のすべて置換の Undo の記録 (v0.3 の M15、REF-05)**: 一致が `REPLACE_COARSE_UNDO_MIN` (10,000) 件以上で、最初の一致の始端から最後の一致の終端までの旧本文のバイト数が、一致ごとの逆変換の見積り (削除する本文のバイト数 + 1 件あたり 128 バイト) 以下のときは、Undo の記録 (逆変換) を「その範囲を旧本文で置き換える」1 件の編集にする (Redo と適用は一致ごとの ChangeSet のまま。メモリは一致ごとの逆変換より増えない)。この Undo では、ほかの View のカーソル・選択のうちその範囲の中にあったものは範囲の始端に寄る (発行 View は `selection_before` を戻すので変わらない)。
- **1 件置換 (`search.replace`、置換欄の `Enter`)**: when は `search.replace_active && !doc.read_only`。対象は、**選択範囲がちょうど 1 つの一致ならその一致** (`search.next` で確定した一致)、そうでなければ **入力で強調している現在の一致**。その範囲を置換文字列 (正規表現では展開後) に置き換える 1 つの Transaction (`Origin::Command("search.replace")`、`UndoGroupHint::Break` で **置換ごとに別の Undo 単位**) として適用し、カーソルを置換の後ろに置いて、そこから先の最初の一致を強調する (終端で折り返さない。起点は置換の後ろ)。対象が無ければ `info.search.replace_no_match`、検索語が空なら `info.search.replace_empty_query`。検索欄の `Enter` (`search.next`) は変わらない。

**v0.1 のすべて置換の手順**:

1. **置換欄**: `search.replace_open` (when `!doc.read_only`) は検索バーを置換欄つきで開く (開いていれば置換欄を足す)。フォーカスは検索語が空なら検索欄、空でなければ置換欄に置く。置換欄は検索欄と同じ 1 行の入力欄 (`SearchBarModel.replace`、02 §8.2。改行は入力できず、貼り付けは最初の行だけ、04 §6.6)。検索語の入力・大小区別の切替・件数の表示は置換欄があっても検索と同じ (§12.1 / §12.2)。読み取り専用の文書では置換欄を開かず `msg.edit.read_only` を出す。開いた後に文書が読み取り専用になったときは、`search.replace_all` の when が偽になり実行できない (パレットでは淡色と理由)。
2. **実行**: `search.replace_all` (when `search.replace_active && !doc.read_only`。キーは `Alt+A`、または `Tab` で `[A] すべて置換` のボタンへ移って `Enter`、04 §8.4) は、そのときの検索欄の検索語・大小区別 (`SearchBarModel.case`) で作った `SearchQuery` (`kind = Literal`・`whole_word = false`・`scope = Document`) と置換欄の文字列を使う。検索語が空なら何もせず `info.search.replace_empty_query` を出す。置換文字列は展開せずにそのまま挿入する (`$1` や `\n` も文字どおり。置換文字列が空なら一致を削除する)。改行の正規化はしない (置換欄に改行は入らない)。確認ダイアログは出さない (1 回の Undo で戻せるため)。
3. **一致の収集と ChangeSet**: 一致の選び方は検索と同じ (§12.1 の「一致の選び方」: 重ならないものを左から順に、両端が書記素境界のものだけ)。文書が 1 MiB (同期で作る上限。定数。実装は `ui_model/search_bar.rs` の `REPLACE_SYNC_BYTES`。§12.2 の `SEARCH_SYNC_STEP_BYTES` (256 KiB) とは別) 以下ならメインスレッドで、超えればジョブ (`JobKind::ReplaceAll`、02 §9.3) で Snapshot (Revision R) を走査し、一致を保持せずに `ChangeSet::from_sorted_edits` で 1 つの ChangeSet を直接作る (`search.max_matches` / `search.count_limit` の上限の影響を受けない)。ジョブの間も入力は止めず (NFR-103)、検索バーを閉じる (`Esc`) と取り消す。
4. **適用**: 適用の時点で Revision が R でなければ (準備中に編集した)、今の Snapshot で 1 回だけ作り直し、それも古ければ適用せずに `msg.search.replace_stale` を出す。一致が 0 件なら文書を変えずに `info.search.replace_no_match` を出す。1 件以上なら **1 つの Transaction** (`Origin::Command("search.replace_all")`、`UndoGroupHint::Break`) として適用し (1 Undo 単位、FR-403)、`info.search.replaced` で件数を示す。文書は dirty になる。Undo の予算 (§7.5) の「直前の 1 操作は保持」が大きな置換にも働く。
**v0.1 の実装の範囲**: 手順 4 の「適用の時点で Revision が R でなければ今の Snapshot で 1 回だけ作り直す」は未実装で、古ければ作り直さず `msg.search.replace_stale` を出して中止する (ジョブの結果が届く前に文書が変わった場合。ジョブにするのは 1 MiB 超の文書だけで、それ以下はメインスレッドで作るので古くならない)。

5. **適用の後**: 主カーソルは元の位置の写像に置き (置換箇所の全選択は `search.replace_all_select_result`、v0.2 以降、07 §5.13)、検索バーと置換欄は開いたまま、保持していた一致は編集で無効になり (§12.2 の「編集後の結果」)、入力が止まった後の再検索で件数が更新される (置換文字列が検索語を含まなければ 0 件になる)。

| 操作 | 手順 |
|---|---|
| 1 件置換 | 現在の一致を `Edit { range, insert }` の Transaction (Origin::Command("search.replace")) で置換し、次の一致へ移動。各置換は個別の Undo 単位 |
| すべて置換 | ジョブで Snapshot (Revision R) 上の全一致を収集 → `ChangeSet::from_sorted_edits` で 1 つの ChangeSet → メインスレッドで `Document::apply`。**適用時に Revision が R でなければ `StaleBase`** となるので、再ジョブ (または小さい文書なら同期で再計算)。1 Undo 単位 |
| 置換文字列 | 正規表現モードでは `$1` / `${name}` の展開 (regex crate の構文)、`\n` は `default_eol` に展開。リテラルモードでは展開なし。行をまたぐ一致 (v0.4) のキャプチャは窓の上で取り、行の終端を文書の種類のまま運ぶ (§12.3.1)。CP932 互換文字の同一視 (v0.4) は置換文字列に掛けない (§12.4.1)。一致した文字列と同じになる置換は置き換えない (v0.4、上の「v0.2 の追加」の先頭) |
| 選択範囲内 | `scope: Selection` で各 Range 内の一致のみ対象。複数 Range にも対応。**M9b の実装 (確定 (ユーザー回答)、2026-10-30)**: 検索バーのトグル `search.toggle_selection` (`Alt+L` / `Ctrl+K L`)。範囲は**検索バーを開いたときの選択** (空でない Range) で、編集に合わせて写像する (範囲の端での挿入は範囲に含める)。ON の間は検索・件数・次へ / 前へ・可視範囲の強調・1 件置換・すべて置換のすべてを範囲内に限る (`SearchQuery::with_scope` → `Matcher` の走査の入口で、範囲に収まらない一致を捨てる)。一致は文書全体と同じ規則 (左から重ならずに) で選び、範囲の境界をまたぐ一致は採らない。開いたときに選択が無ければ切り替えずに `info.search.no_selection` を出す |
| 結果 Selection | すべて置換後は primary カーソルを元位置の写像に置く (置換箇所を全選択する Helix 風は設定で選択可) |
| 件数上限超 | `max_matches` を超える「すべて置換」はジョブ内で **ChangeSet を直接構築** する (一致を保持しない) ため上限の影響を受けない。ただし ChangeSet 自体のサイズが大きくなるので、Undo 予算 (§7.5) の例外規則 (直前 1 操作は保持) が働く |

### 12.6 クリップボードへのコピーと大きな文字列化

- 大きな選択範囲 (100 MB の全選択など) をコピーするとき、メインスレッドで `String` を作ってはならない (数十 ms と 100 MB のメモリ)。大きい範囲 (64 KiB 以上) では、core は Snapshot と範囲 (`ClipboardSource`) を Editor のハンドル表に登録し、`Effect::SetClipboard` は **`ClipboardText::Handle(ClipboardHandleId)`** を運ぶ。Host は `Editor::take_clipboard(id)` で `ClipboardSource` を取り出し、**文字列化を Host 側のワーカーで行う** (02 §5.4 / §6.3。取り出した後のハンドルは無効)。小さい範囲 (64 KiB 未満) は core が文字列化して `ClipboardText::Inline(String)` で運ぶ。`ClipboardText` は eve-view の serde 可能な型で、eve-text の型 (Snapshot) を含まない。
- 内部レジスタは、64 KiB 未満なら `Arc<str>` にコピーし、64 KiB 以上なら一旦 Snapshot 参照で持ってから §6.5 と同じ **実体化ジョブ** で範囲だけの独立の rope に置き換える (結果が届いたときにレジスタの内容が同じ `SliceKey` のままなら置き換え、別の内容に替わっていれば捨てる) (Snapshot 参照のままだと、その後の編集で文書全体が会計外で残り得る。§6.5)。レジスタは最新の 1 件だけを持つので、その大きさがそのまま追加のメモリになる (Undo の会計には含めない)。内部レジスタからの貼り付けも §8.4 の改行の正規化を通す。システムクリップボードへの受け渡しには上限 (`clipboard.system_max_bytes`、既定 64 MiB、07 §5.6) を設け、**文字列化の前に** 選択範囲の総バイト数 (全 Range の合計) で判定する。超えたら `Effect::SetClipboard` を出さず (文字列化もしない)、内部レジスタだけに入れて通知する (01 FR-205 AC-4 (a))。上限未満の大きな範囲は上のハンドルの経路で、文字列化とシステムクリップボードへの書込の間も入力を止めない (同 (b))。OSC 52 の上限 (100,000 バイト、05 §8.4) は別。
- **選択が空のときの `clipboard.copy` / `clipboard.cut`** は何もしない (行全体のコピー・切り取りは行わない。通知も出さない。文書・クリップボード・内部レジスタは変わらない)。
- **不正バイトのエスケープ文字** (v0.4 の M23。正本は §8.3.1 の「本文に入れる規則」): 内部レジスタはエスケープ文字のまま持ち、コピーした時点の `notation` (コピー元が `pua_escape != DisabledCollision`) と `escape_origin` (コピー元の `escape_encoding`) を覚える。システムのクリップボードへ渡す文字列は、`notation` が真ならエスケープ文字を `<0xNN>` にしたもの (`ClipboardSource::to_text`。Handle を Host が文字列化するときも同じ関数)。貼り付けでは、届いた文字列と内部レジスタの文字列を、両側に同じ改行の正規化をかけてから比べ、一致して挿入先が同じ `escape_encoding` の文書なら内部レジスタの内容 (エスケープ文字のまま) を入れる。それ以外で `U+10FF00..U+10FFFF` が入るときは `<U+10FFNN>` の文字列にして知らせる。届いた文字列を読めなかったときの貼り付けは、内部レジスタを使う (書き出しに失敗した後も、照合の規則は変えない)。
- **システムクリップボードからの貼り付け** (`edit.paste`): core は `Effect::RequestClipboard { token }` で読取を要求し、応答 `InputEvent::ClipboardContent { token, text }` が届いた時点の発行 View の **到着時の Selection** に貼り付ける (要求時の Selection を保存しない。待つ間に動かしたカーソルの位置に入る)。token が古くなって応答を捨てるのは、**次の `RequestClipboard` を出したときだけ** で、待つ間の編集・移動・検索では古くならない。応答が届いたときに文書が読み取り専用になっている、または View が閉じている場合は貼り付けずに捨てる。

### 12.7 ワークスペース: 走査・除外・ファインダ・横断検索 (v0.5 の M31。FR-125 / FR-405)

**方式** (v0.5 の計画で決めた。v0.5-plan §1.2 と WS-01 / WS-02。06 X10 の方式に揃えた。それまで本節は `ignore` + `grep-searcher` と書いていた): ripgrep の部品 (`ignore` / `grep-searcher`) は使わない。`std::fs` を直接呼ぶので FileSystem Port と `MemFs` の試験を通らず、`ignore::WalkParallel` は自前でスレッドを作る (ADR-0006 の固定 2 本・02 §4.3 R11 に反する)。`grep-searcher` は UTF-8 と BOM 付き UTF-16 だけを扱うので、Shift_JIS / EUC-JP のファイルで当たらない (速さより正しさを採る)。ワーカーは固定 2 本のままで、走査は `bg` でディレクトリ 16 個ごとに積み直して譲る (OQ-103、確定 (ユーザー回答、2026-11-21)。S-26。基準機で S-26 の (3) を測り直し、M31 の BM で走査が遅いと分かったら、走査の専用のレーン 2 本に替える条件付き。`JobSpawner` の形は変えないので後から差し替えられる)。

本節は **M31 の仕様の PR** (2026-11-21) で細目を決めた。材料は S-26 の REPORT (「M31 の仕様の PR で決める点」1〜6) と v0.5-plan §2.1〜§2.3。ルートの検出の規則と CLI の正本は 07 §12.1 / §18、`FileSystem::read_dir` の形は 02 §4.5、OS ごとの実装は 05 §2.1、表示とキーは 04 §8.11 / §8.12 と 07 §11.6、受入基準は 01 §5.5 の FR-125 / FR-405、試験と計測は 09 (§3.3 の `fuzz_gitignore`、§4 のワークスペースの木、§5.2 の BM-125 / BM-125s / BM-103b-finder / BM-103b-search-ws / BM-405)。

#### 12.7.1 ルートの検出 (規則の正本は 07 §12.1)

- **ジョブ**: `JobKind::WorkspaceRoot` (`io`、02 §9.3)。起動のたびに 1 回、起動時のパスを開くジョブ (03 §9.8) を投入した **後に** 投入する (`io` は直列なので、上方の探索が最初の文書の読込を遅らせない。最初の Frame も待たない。01 FR-125 AC-3)。入力は `--workspace` の値・位置引数 (正規化したパス。`-` は除く)・`PlatformEnv.cwd`・`workspace.root_markers` (07 §5.9)。
- **手順** (07 §12.1 の規則をジョブで行う): (1) `--workspace` があれば、その `metadata` がディレクトリならそれ。ディレクトリでない・読めなければ `msg.workspace.root_invalid` (04 §8.13。`{reason}` は結果の `explicit_error` から作る) を出し、`--workspace` が無かったものとして (2) 以降の規則で決める。(2) `--workspace` が無い (または (1) で使えなかった) とき、位置引数にディレクトリがあれば、最初のディレクトリ。(3) 起点 (FILE の親ディレクトリの共通の祖先。FILE が無ければ cwd) から上へ、各ディレクトリで目印を `metadata` で探し、最初に見つかったディレクトリ。(4) 見つからなければ cwd。どの規則で決まったか (`RootSource { Explicit, Dir, Marker, Cwd }`) を結果に入れる。
- **結果**: `JobResult::WorkspaceRoot { root: FsPath /*canonicalize して 05 §3.3 の表示用の形に揃えたもの。canonicalize できなければ正規化しただけのパス*/, source: RootSource, dir_args: Vec<FsPath> /*ディレクトリだった位置引数 (ルートにしたものを含む)*/, explicit_error: Option<FsError> /*`--workspace` を使えなかった理由*/ }`。core は `Editor` にワークスペース (`root`・`source`) を 1 つ持つ (1 プロセスに 1 つ、07 §12.1)。結果が届くまでの間に `workspace.open_file` / `search.workspace` を実行したら、オーバーレイを開いて `ui.finder.footer_waiting` (「ワークスペースを調べています…」) を出し、届いた時点で走査・検索を始める。ジョブが失敗して (パニックなど) ルートが決まらなかったときは、待ち続けずに件数の欄を `ui.finder.footer_no_root` (「ワークスペースを決められませんでした」) にする (ファインダ・横断検索とも。M31 の段 7 の PR で足した。それまでの実装は「調べています」のまま止まっていた)。
- **位置引数のディレクトリ**: 起動時のパスを開くジョブがディレクトリで失敗したとき (03 §9.2 の `FileMeta.kind` が `Dir`)、仕様 (07 §18.2) は v0.1 から `msg.cli.directory` を出すとしていたが、v0.4 までの実装は開くジョブの失敗として `msg.open.not_a_file` を出す (`msg.cli.directory` は言語ファイルにあるだけで使われていない)。v0.5 (M31 の段 1) は、起動時のパスについてはこの失敗を `msg.open.not_a_file` にせず、ルートの検出の結果が届くまで知らせを保留し、届いたら、ルートにしたディレクトリには何も出さず、それ以外のディレクトリには `msg.cli.directory_ignored` (「ルートにできるフォルダは 1 つだけ」) を出す。ルートにした `DIR` が位置引数にあったときは、結果が届いた時点でフォーカスのあるペインに対してファインダを開く (07 §18.2。起動時に開くファインダは、そのまま閉じれば何も変わらない)。`DIR` だけを渡したときに開く文書は無題の文書 1 つ (v0.1 と同じ)。
- **所有者の検査はしない**: ルートの検出は目印の有無を見るだけで、ワークスペース設定を読まない (設定を読むのは M33 の FR-705。そのときに 07 §12.1 の所有者の検査を通す)。他のユーザーが作れる `/tmp/.git` を目印として拾っても、M31 で起きるのはファインダ・横断検索の範囲が `/tmp` になることだけで、何も実行せず設定も読まない。
- **段 1 の実装で決めた細目** (M31 の段 1 の PR、2026-11-21): (1) **目印**: 設定のキー `workspace.root_markers` (07 §5.9) は予約のまま、段 1 は既定値 (`.eve`・`.git`・`.hg`・`.svn`) を定数で渡す (公開すると `--dump-config` の出力と生成リファレンスが変わるので、同じ `[workspace]` の `workspace.exclude` を公開する段でまとめて公開する)。(2) **ジョブの順**: `io` には、起動時のパスを開くジョブ → `WorkspaceRoot` → 前回の版の記録 (`LastVersion`) → 退避の走査 (`RecoveryScan`) の順に積む (退避の案内を最後のメッセージにする、02 §9.3)。`--workspace` の値は bin → Host (`HostInit.workspace`) → core (`StartupConfig.workspace`) と渡し、起動 (`Editor::open_initial*`) がジョブを投入するときに `Paths::normalize` で正規化する (正規化できなければジョブには渡さず、結果が届いた時点で `msg.workspace.root_invalid` を出す)。カレントディレクトリは `Paths::normalize(".")` (起動時に解決した作業ディレクトリ)。(3) **ディレクトリの見分け**: 位置引数がディレクトリかはジョブの `metadata` (リンクを辿る) で決め、`dir_args` に入れる。無い・読めない位置引数は FILE として起点に数える。核は、起動時のパスを開いた文書の開くジョブが `NotAFile` で失敗したときだけ知らせを保留し、結果が届いたら、ルートにした `DIR` には何も出さず、`dir_args` にあるほかのパスには `msg.cli.directory_ignored`、`dir_args` に無いパス (FIFO・デバイスなど) には v0.4 までと同じ `msg.open.not_a_file` を出す。結果が届いた後に開くジョブの失敗が届いても同じ判定をその場で行う。ジョブが失敗した (パニックなど) ときはルートを決めず、保留した知らせは `msg.open.not_a_file` で出す。起動の後の `file.open` でフォルダを指定したときは今までどおり `msg.open.not_a_file`。(4) **知らせの順**: 位置引数のディレクトリの知らせを引数の順に出し、`msg.workspace.root_invalid` を最後に出す (利用者が明示した指定なので、現在のメッセージにする)。`{reason}` は、ディレクトリでなかったときは `not a directory`、ほかはほかの読込の知らせと同じく `FsErrorKind` の名前 (`notfound`・`permissiondenied` など)。水準はどちらも警告。(5) **ファインダ**: `DIR` で決まったとき (`RootSource::Dir`) にファインダを開くのは段 6 (段 1 はルートと決まった規則を持つだけ。段 6 で入れた。§12.7.4 の段 6 の細目 (7))。

#### 12.7.2 走査 (`WorkspaceWalk`)

**形**: 走査の状態 `WorkspaceWalk` は eve-core の型で、I/O は `FileSystem::read_dir` (02 §4.5)、除外の照合に要るファイルの読込 (`open_read`)、リポジトリの検出 (§12.7.3) と `follow_symlinks = true` のリンクの確かめに使う `metadata` だけを使う。除外の設定 `ExcludeConfig` は、`workspace.exclude` の行・`search.workspace.respect_gitignore`・`search.workspace.follow_symlinks` を、走査を始める時点の値で持つ (走査の途中で設定が変わっても、次の走査から効く)。**区切って譲る仕組み**: 1 区切りを終えたジョブは、終端の結果を送らずに続き (`JobStep::Continue`、02 §9.3) を返し、ワーカーのレーンが同じ `JobId`・同じ `CancelToken` のまま自分のキューの末尾に積む (v0.4 までのレーンにこの経路は無く、M31 の段 4 で eve-host と `ManualJobSpawner` に入れる)。状態は、未処理のディレクトリのスタック (各ディレクトリの除外の連鎖 (§12.7.3) を `Arc` で持つ)、見つけた件数、打ち切りの印、読めなかったディレクトリの数。同じ型をファインダの走査 (`JobKind::WorkspaceScan`) と横断検索 (`JobKind::WorkspaceSearch`) が使う。

1. **区切り** (S-26 の決める点 3): 1 回のジョブの実行 (1 区切り) で **ディレクトリを 16 個** (ただし、区切りの始めから `Clock` で 8 ms を過ぎたら、16 個に満たなくてもそのディレクトリの後で) 処理し、続きを `bg` のキューの末尾へ積み直す (02 §9.1 の検索と同じ。間に入ったジョブが先に走る)。1 つのディレクトリの項目が **4,096 個** を超えるときは、そのディレクトリの照合と候補の用意も 4,096 個ごとに区切って積み直す (`read_dir` の呼出し 1 回は区切れない。下の 3)。区切りの数は定数 (`WALK_DIRS_PER_STEP = 16`・`WALK_ENTRIES_PER_STEP = 4096`・`WALK_STEP_TIME = 8 ms`) で、設定に出さない。
2. **順序**: 深さ優先。各ディレクトリの項目は名前のバイト列の順に並べてから処理する (`read_dir` の並びは OS と FS で違うので、並べて結果を決定的にする。試験の `MemFs` でも同じ結果になる)。候補の束は見つけた順に送る。
3. **`read_dir` の上限**: `read_dir(dir, max_entries)` の `max_entries` は常に `WALK_ENTRIES_PER_DIR_MAX = 200_000` (残りの件数を渡さない。除外される項目も数えるので、残りを渡すと OS の並びでどの項目が残るかが決まり、結果が決定的でなくなる)。`truncated` が返ったディレクトリは、そこまでの項目で続け、走査全体の打ち切りの印を立てる (表示は下の 6 と同じ)。OS の列挙 1 回 (Windows の `FindFirstFileExW` / `FindNextFileW`、Unix の `getdents`) は区切れないので、1 つのディレクトリに数万の項目があると、その 1 回が区切り 1 回の長さを決める (S-26 で 15,000 項目のディレクトリが 11〜31 ms。10 万を超えると 100 ms に近づき得る)。保存・読込は `io` にあるので待たない (S-26 の決める点 5。走査中も `io` の開始の遅れは 0.3 ms 未満)。`bg` のほかのジョブ (エディタ内の検索・ファインダの照合) は、その 1 回の後ろに待つ。
4. **項目の種類** (`DirEntry.kind`、02 §4.5。名前が `.git` かの比較と、下の 5 の名前の比較は、Windows・macOS では ASCII の大文字小文字を畳む (`.GIT` も git のディレクトリになる)): `File` は候補にする。`Dir` は除外に当たらなければ降りる。`Symlink` (Windows のジャンクションと名前の代理の再解析点を含む、05 §3.1) は、`search.workspace.follow_symlinks = false` (既定) では **降りない**。ファインダには候補として出し (開くとリンク先を開く、FR-106。リンク先がディレクトリなら開くときに `msg.open.not_a_file`)、横断検索では読まない (リンク先のワークスペースの外を読まない。07 §12.3 の危険の行の理由)。`true` のときは `metadata` (リンクを辿る) で種類を確かめ、ディレクトリなら降り、ファイルなら候補にする。辿ったディレクトリは `FileId` を覚え、同じ `FileId` に 2 度降りない (循環の防止。訪れた集合には、走査を始めるときにルート自身の `FileId` を入れる (ルートを指すリンクで木を 2 回走査しない)。`FileId` が取れない FS では canonicalize したパスで代える。Windows でディレクトリの `FileId` を取る方法は 05 §2.1)。`Other` (FIFO・ソケット・デバイス) は候補にしない (開くと止まり得る)。
5. **常に飛ばすもの**: 名前が `.git` の項目 (ディレクトリでもファイルでも) は候補にせず降りない (入れ子のリポジトリの境目の判定にだけ使う、§12.7.3)。evoeditor 自身の保存の一時ファイルと退避名 (`.<name>.eve-tmp-*` / `.<name>.eve-bak-*`、05 §4.6) も候補にせず読まない (退避名を開いて保存すると、唯一の復旧の写しを壊し得る。復旧は起動時の案内の経路で行う)。Linux でルートがファイルシステムのルート `/` のときは、`/proc`・`/sys`・`/dev`・`/run` に降りない (疑似 FS。定数の表)。`.` で始まるほかの名前 (`.github`・`.eve`・`.env` など) は除外に当たらなければ出す (01 FR-125 AC-1)。Windows の隠し属性・システム属性も同じく出す。
6. **項目の数の上限** (S-26 の決める点 6): 候補 (除外に当たらないファイル) が **200,000 件** (`WORKSPACE_MAX_FILES`。定数。設定のキーは足さない) に達したら、走査をやめて打ち切りの印を立てる。除外される項目とディレクトリばかりの木 (ルートが広すぎるとき) に備えて、列挙した項目の総数 **2,000,000** (`WORKSPACE_MAX_ENTRIES`) と深さ **256** (`WORKSPACE_MAX_DEPTH`。それより深いディレクトリには降りない) にも上限を置き、どちらかに当たったら同じく打ち切りの印を立てる。表示は §12.7.4 / §12.7.5 (`ui.finder.footer_truncated`・`ui.workspace_search.footer_truncated`)。S-26 の木 (残るファイル 10.6 万) で候補の用意 23.8 ms・照合の p99 10 ms なので、上限の 20 万でも 01 FR-125 AC-2 の目標の内に収まる見込み (基準機の BM-125 で確かめる)。
7. **読めないディレクトリ**: `read_dir` が失敗したディレクトリ (`PermissionDenied`・`NotFound` (走査の間に消えた)・`Other`) は飛ばし、数だけを数える (`unreadable_dirs`)。走査は止めない (01 FR-125 AC-3)。`read_dir` が `incomplete` (列挙が途中で失敗した。02 §4.5) を返したディレクトリは、返った項目で続け、`unreadable_dirs` に同じく数え、6 の打ち切りの印も立てる (一覧が欠けていると分かるように。表示は 6 と同じ)。ルートそのものが読めないときだけ、走査を失敗として終え `msg.workspace.scan_failed` を出す。
8. **取消し**: 走査のジョブは区切りごと (と、1 つのディレクトリの 4,096 項目ごと) に `CancelToken` を見る。OS の列挙の中では見ない (`read_dir` の 1 回が戻るまで取消しは効かない)。新しい走査を始めるときは前の走査を取り消し、結果は世代 (`ScanGeneration`) で照らして古いものを捨てる。
9. **OS の呼出しで止まったとき**: 走査はルートの `FsKind` に依らず行う。`read_dir`・`open_read` が OS の呼出しの中で長く止まる (応答の無いネットワーク共有・exFAT の外付けのドライブの 0.3〜2.5 s の止まり (S-25)) と、`bg` のほかのジョブがすべて待つ (`bg` には `io` の監視時間 (OQ-179) が無い。取消しも呼出しから戻るまで効かない)。待つのは、今の実装ではエディタ内の検索とファインダの照合だけだが、仕様が `bg` に置く `WriteRecovery` (退避先に保存) とスワップの書出し (02 §9.1。今の実装は `io`) を `bg` へ移す版では、それらも待つ (データの保全に関わる)。v0.5 は既知の制約にし (01 §4.6 は M35 で書き換える)、`WriteRecovery` とスワップを `bg` へ移す PR は、その前にワークスペースのジョブを別のレーンへ分けるか (OQ-103 の (B)、S-26) を決める。
10. **段 4 の実装で決めた細目** (M31 の段 4 の PR、2026-11-21。`eve_core::WorkspaceWalk`): (a) 区切りの判定は、ディレクトリを 1 つ読んだ (列挙して並べた) 直後に「その区切りで読んだ数が 16 個・区切りの始めから 8 ms」を見て、項目を 1 つ処理する前に「その区切りで処理した項目が 4,096 個」を見る (項目の数は 1 つのディレクトリに限らず区切り全体で数える。16 個の大きなディレクトリで 1 区切りが 6 万項目にならないように)。取消しは区切りの始め (`run_job` の入口) で見る (4,096 項目ごとも区切りの始めになる)。(b) 候補の塊は 4,096 件ごとに送り、満たなくても前に送ってから 100 ms (`SCAN_FLUSH_INTERVAL`。ファインダの照合のやり直しの間隔と同じ) を過ぎていれば区切りの終わりで送る (候補の少ない遅い FS でも走査の途中の候補が届くように)。走査の終わりに、送る候補が無くても読めなかったディレクトリの数が前に送った値から増えていれば、空の塊の `ScanBatch` で数を届ける (`ScanDone` の形は変えない)。(c) 候補の数の上限は「20 万件を **超える** 候補を見つけたら」打ち切る (ちょうど 20 万件の木は打ち切りの印を立てない)。(d) `respect_gitignore = false` でもリポジトリは探し、`core.ignorecase` を読む (`workspace.exclude` を項目を含むリポジトリの畳み方で照らすため、§12.7.3 の細目 (5))。読まないのは `.gitignore` と `info/exclude` だけ。(e) ルート自身とその祖先は、祖先の `.gitignore` に当たっても除外しない (利用者が選んだフォルダ。中の項目には祖先の規則が効く)。(f) `follow_symlinks = true` で、`metadata` がその他 (FIFO など) を返したリンクは候補にせず、辿れない (ダングリングの) リンクは候補にする (`false` のときと同じく、開くときに失敗を示す)。辿ったディレクトリの `FileId` の集合に入れるのはリンクで辿ったディレクトリとルートだけで (上の 4)、リンクとふつうのディレクトリの両方から同じディレクトリに届くと 2 回走査する (循環はしない)。(g) `.git` (入れ子のリポジトリの境目) は、ディレクトリを列挙した項目の中で見つける (ルートとその祖先だけ `metadata` で探す)。`.git` がシンボリックリンク・その他なら、境目にするが源を読まない (`core.ignorecase` は OS の既定)。(h) 除外した項目・降りたディレクトリ・入れ子のリポジトリを知らせる口 `WalkObserver` を持つ (git との差分試験 (09 §3.4) が使う。本番は何もしない)。**既知の制約** (段 4 の PR のレビューで記録した。直していない): `follow_symlinks = true` のとき、リンクの項目ごとの `metadata` (と `FileId` の取得の canonicalize) は項目の処理の中で行い、その間は時間 (8 ms) を見ないので、リンクの多いディレクトリでは 1 区切りが 4,096 項目分の `metadata` の長さになり得る。`bg` のレーンのスレッドを起動できなかったとき (`ThreadJobSpawner` が投入に失敗して `Failed { Panic }` を返す経路) は、`ReleaseCandidates` の塊が主スレッドで drop される。

**候補の持ち方と解放** (S-26 の決める点 4。「**主スレッドで大きな一覧を解放しない**」): 候補 `Candidate` は、ルートからの相対パス (区切りは全 OS で `/`。照合と表示に使う UTF-8 の文字列。相対パスのどれか 1 つの要素でも UTF-8 で表せないときは、置換文字にした表示用の文字列と、開くための相対パス全体の元の `OsString` の両方を持つ)、ASCII だけを小文字にしたバイト列、ファイル名の開始位置、含む文字の集合 (64 ビット。速い棄却に使う) を持つ。候補は走査の側 (`bg`) で作り、4,096 件前後の **不変の塊** (`Arc<[Candidate]>`) にして束で主スレッドへ送る。主スレッドは塊の `Arc` を列に足すだけで、候補を 1 件ずつ作らない・複製しない。ファインダの照合のジョブには塊の `Arc` の列を渡す (複製は塊の数だけ)。

- 一覧を作り直す (新しい走査が終わった・ワークスペースを替えた・ファインダを閉じて一定の時間が経った (下)) ときの古い一覧は、主スレッドで drop しない: `JobKind::ReleaseCandidates { chunks }` (`bg`。キューの順は検索と同じ段) に渡して `bg` で解放する。照合のジョブが同じ塊を持っていれば、最後の参照はジョブの側に残り、どちらにしても `bg` で解放される。S-26 で、10 万件の一覧を主スレッドで解放すると 1 回 13〜15 ms 止まり、打鍵の遅れが p99 8.5 ms まで悪化した (NFR-103 の p99 8 ms を超える)。
- **一覧の寿命**: ファインダを最初に開いたときに走査を始め、一覧は閉じた後も持つ (次に開いたときにすぐ絞り込める)。開くたびに、前の一覧を見せたまま新しい走査を始め、新しい走査が **終わった時点で** 一覧を差し替える (走査の途中の新しい候補を前の一覧に混ぜない。差し替えの前後で選択中の候補のパスが残っていれば選択を保つ)。最初の走査の間 (前の一覧が無い) だけは、届いた束をそのまま候補に足していく (01 FR-125 AC-3 の「走査が終わる前も、それまでに見つけたファイルで絞り込める」)。閉じてから 5 分 (`FINDER_LIST_KEEP`。定数) 開かなければ一覧を手放す (`ReleaseCandidates`。10 万件で数十 MB を持ち続けない)。ファイル監視 (FR-117、M33) が入っても、v0.5 は監視のイベントで一覧を直さない (開いたときの走査で直る)。

#### 12.7.3 除外の照合 (gitignore。eve-core の純粋関数、06 X10)

**読むもの** (ripgrep の既定に揃える。v0.5-plan §1.2):

| 源 | 読む条件 | 置き場所への固定 (アンカー) |
|---|---|---|
| `workspace.exclude` (07 §5.9) | 常に | ワークスペースのルート |
| 各ディレクトリの `.gitignore` (ルートより上の祖先を含む。下の「リポジトリ」) | `search.workspace.respect_gitignore = true` (既定) で、そのディレクトリがリポジトリの中にあるとき | その `.gitignore` のあるディレクトリ |
| リポジトリの `info/exclude` | 同上 | リポジトリのルート (作業ツリーの最上位) |

- **読まないもの**: グローバルの excludesFile (`core.excludesFile` は、どの階層の git の設定に書かれていても読まない)、システム・利用者の git の設定、`.ignore` / `.rgignore` (ripgrep 独自。git の結果と比べられないので読まない)、`.gitattributes`。git の設定から読むのは `core.ignorecase` だけ (下)。
- **リポジトリ**: ルートから上へ (ルートを含む) 名前 `.git` の項目を探し、最初に見つかったディレクトリをリポジトリのルートとする (探すのは走査のジョブの最初の区切りで、`metadata` を使う。上方の探索はファイルシステムのルートで止める)。見つからなければ gitignore の源 (`.gitignore`・`info/exclude`) は読まず、`workspace.exclude` だけを効かせる (「`.git` があるときだけ効かせる」)。ワークスペースのルートがリポジトリのルートより下 (`--workspace` でリポジトリの中のフォルダを指したとき) なら、リポジトリのルートからワークスペースのルートまでの祖先の `.gitignore` も読む (結果が、そのフォルダで `git ls-files` を走らせたときと同じになるように)。`.git` がディレクトリならその中の `info/exclude` と `config`、**ファイル** (`git worktree`・サブモジュール) なら中の `gitdir: <path>` の行が指すディレクトリと、そこに `commondir` があればそれが指すディレクトリ (共通の git ディレクトリ) の `info/exclude` と `config` を読む (相対パスはそのファイルのあるディレクトリから解決する)。`gitdir:` / `commondir` の行き先は、ローカルのパス (相対パスか、ドライブ文字・`/` で始まる絶対パス) だけを辿り、UNC のパス (verbatim の UNC の形を含む)・デバイスの名前空間 (05 §3.2 の手順 5 で拒むもの) は辿らない (ファインダを開くだけで SMB に接続し、資格情報を相手に渡すことを防ぐ)。読めない・辿らないときはその源を飛ばしてログに残す (画面には出さない)。
- **入れ子のリポジトリ**: 走査で、ルートより下のディレクトリの中に `.git` があれば、そこから下は別のリポジトリとして扱う (git がサブモジュール・入れ子のリポジトリの中に外側の規則を効かせないのと同じ。ripgrep の `ignore` も同じ)。外側の `.gitignore` と `info/exclude` はその境目で効かなくなり、内側の `.gitignore` と内側のリポジトリの `info/exclude`・`core.ignorecase` が効く。境目のディレクトリ自身を降りるかどうかは外側の規則で決める。`workspace.exclude` は境目に依らず全体に効く。
- **優先**: 1 つの項目について、`workspace.exclude` → 内側の `.gitignore` から外側へ → `info/exclude` の順に見て、最初に当たった源が決める。各源の中は **後ろの行から** 当て、最初に当たった行が決める (否定 `!` の行なら「除外しない」)。どの行にも当たらなければ除外しない。除外したディレクトリには降りないので、その中のファイルを否定の行で戻すことはできない (git と同じ)。
- **`workspace.exclude` の書き方**: 各要素を、ワークスペースのルートに置いた `.gitignore` の 1 行と同じ書式で読む (`**/node_modules`、`/build/`、`*.log`、`!keep.log`)。gitignore より優先するので、`!` の要素で gitignore が除外したものを戻せる (例 `!.env.example`)。`workspace.exclude+ = [...]` で既定に足す (07 §3.3)。既定 `["**/.git", "**/node_modules", "**/target"]` の `**/.git` は、§12.7.2 の 5 で常に飛ばすので書かなくても同じ。`respect_gitignore = false` でも効く。
- **行の解釈** (gitignore(5) の記述に揃える): UTF-8 の BOM を飛ばす。行末の CR を 1 つ落とす (CRLF のファイル)。空行と `#` で始まる行は飛ばす (`\#` は `#` の文字)。行頭の `!` は否定 (`\!` は `!` の文字)。エスケープされていない末尾の空白を落とす (`\ ` は残す)。末尾の `/` はディレクトリだけに当てる印 (照合の前に外す)。先頭か途中に `/` があれば置き場所からの相対パスに当て、無ければ名前 (パスの最後の要素) に当てる。`**` は、`**/` (先頭。どの深さのディレクトリにも当たる)・`/**` (末尾。中のすべてに当たる)・`/**/` (0 個以上のディレクトリ) のときだけ特別な意味を持ち、ほかの位置の `**` は `*` 2 つとして読む。
- **照合** (git の wildmatch に揃える。S-26 の決める点 2): `*` は `/` 以外の 0 バイト以上、`?` は `/` 以外の 1 バイト、`[...]` は `/` 以外の 1 バイトの集合 (照らすのはバイト列で、UTF-8 の 2 バイト以上の文字は `?` 1 つに当たらない。git と同じ。範囲 `a-z`、否定 `[!x]` / `[^x]`、POSIX の文字の集合 `[[:alpha:]]` `[[:digit:]]` `[[:alnum:]]` `[[:space:]]` `[[:upper:]]` `[[:lower:]]` `[[:punct:]]` `[[:xdigit:]]` `[[:print:]]` `[[:graph:]]` `[[:cntrl:]]` `[[:blank:]]`。集合は ASCII で判定する)、`\` は次の 1 文字をそのまま当てる。閉じていない `[` を含む行は、その行だけ当たらないものとして扱う (git と同じ)。照らすのはルート (またはその `.gitignore` の置き場所) からの相対パスのバイト列で、区切りは `/` (Windows も)。名前が UTF-8 で表せないファイルは、Unix は元のバイト列、Windows は WTF-8 のバイト列で照らす。
- **大文字小文字** (S-26 の決める点 1): リポジトリの git の設定の `core.ignorecase` を読み、`true` なら ASCII の英字だけを畳んで照らす (git の照合と同じく、ASCII の外の文字は畳まない)。キーが無ければ `false` (git の既定)。設定のファイルを読めなければ OS の既定 (Windows・macOS は `true`、Linux は `false`)。読むのは `[core]` の節の `ignorecase` の行だけで (節とキーの名前は大文字小文字を区別しない。値は `true` / `false` / `yes` / `no` / `on` / `off` / `1` / `0`)、`[include]` / `[includeIf]` は辿らない。リポジトリが無いとき (`workspace.exclude` だけのとき) は区別する。Windows の `git init` は `core.ignorecase = true` を書くので、`*.log` で `FOO.LOG` も除外される (S-26 の木で 1,607 件の差。区別すると、除外したつもりのファイルがファインダと横断検索に出る)。
- **段 3 の実装で git と照らして決めた細目** (M31 の段 3 の PR、2026-11-21。どれも git の実行結果との差分試験 (09 §3.4 の細目の規則とランダムな規則) で確かめた): (1) 集合の中の大文字小文字: 畳むときは照らす文字を小文字にしてから比べ、1 文字の要素 (`[A]`) は畳まない (`core.ignorecase = true` でも `[A]` は `A` にも `a` にも当たらない)。範囲 (`[A-Z]`) は、小文字にした文字かその大文字のどちらかが範囲に入れば当たる。`[[:upper:]]` は畳むときは小文字にも当たり、`[[:lower:]]` は大文字にも当たる。(2) `**` が特別な意味を持つのは、直前が `/` か行の先頭で、直後が `/`・行の終わり・`\/` のときだけ (`a**/z` の `**` は `*` と同じ)。逃がした `/` の前の `**` (`**\/z`) は 0 個のディレクトリに当たらない。(3) 末尾の空白は空白 (U+0020) だけを落とす (タブは落とさない)。行の中の NUL から後ろは読まない。逃がしていない `\` で終わる行・`[a-\` は当たらない。否定・固定の印を外すと空になる行 (`!`・`/`・`!/`) は何にも当たらない。(4) `[[:space:]]` は空白・タブ・LF・CR (垂直タブ・改ページを含めない)、`[[:cntrl:]]` は 0x00〜0x1F と 0x7F (Unix の差分試験で確かめる。Windows はこれらの文字を名前に使えない)。(5) `workspace.exclude` は、項目を含むリポジトリの `core.ignorecase` で照らす (リポジトリの外は区別する。ルートの `.gitignore` の 1 行と同じ扱い)。(6) 実装は `eve_core::{IgnoreRules, ExcludeChain, RepoIgnore}` (03 §12.7.2 の走査が、ディレクトリごとに `ExcludeChain` を持ち、`.gitignore` を足す・入れ子のリポジトリで `enter_repo` する)。パスは走査の最上位 (リポジトリのルートがワークスペースのルートより上ならリポジトリのルート) からの相対パスで照らす。
- **読み方**: ignore のファイル・`info/exclude`・git の `config`・`.git` のファイルは、`read_dir` の種類が `File` のときだけ読む (シンボリックリンクの `.gitignore` は辿らない。git 2.32 以降と同じ)。読込は上限 + 1 バイトで打ち切り (`/dev/zero` などへの取り違えに備える)、`open_read` は開いた後のハンドルで通常のファイルと確かめる (FIFO・デバイスで止まらない。05 §2.1)。
- **所有者の検査**: 除外の源 (祖先・入れ子のリポジトリの `.gitignore`・`info/exclude`・`config`) には、M31 では所有者の検査 (07 §12.1) を掛けない。読んだ内容で変わるのは、どのファイルを一覧・検索の対象にするかと `core.ignorecase` だけで、何も実行しない。他のユーザーが置いた `.git` (Windows のドライブの直下・Unix の `/tmp`) で一覧の範囲や除外が変わり得ることは既知の制約にし、所有者の検査を入れる M33 (FR-705) で、除外の源にも同じ検査を掛けるかを決める。**M33 の仕様の PR (2026-11-22) で「掛けない」と決めた** (07 §12.1 の OWN-8、理由は 07 §12.6.3。`workspace.exclude` はワークスペース層の設定のキーなので、所有者の検査を通った `.eve/config.toml` からだけ読む)。
- **資源の上限**: 1 つの ignore のファイルは 1 MiB まで (超えたらそのファイルを読まずにログに残す。S-26 の木の最大は 1 KiB 未満)。1 行は 4,096 バイトまで (超えた行は飛ばす)。パターンの数は 1 ファイル 10,000 行・1 つの項目に効く連鎖の合計 50,000 行まで (超えた分の行は読まずにログに残す)。照合の時間は、1 回の照合でパターンの長さ × パスの長さに比例する上限に収まるように作る (`*` の後戻りを無制限にしない。fuzz の性質、09 §3.3)。
- **ライセンス**: 実装は gitignore(5) の記述と許容的なライセンスの資料だけから書く (クリーンルーム。段 3 の PR の説明に書く)。git のソース (`dir.c`・`wildmatch.c`。GPL-2.0) と git の試験のデータ (`t/`) は写さない (許可リスト 05 §17.1 の外のライセンスで、リポジトリの Apache-2.0 と両立しない)。振る舞いは gitignore(5) の記述と、git との差分試験 (09 §3.4) で揃える。S-26 の試作 (git の照合を写したもの) はリポジトリの外の使い捨てで、コードを持ち込まない。
- **共通部品**: 同じ照合の関数を AI の除外 (06 §6.5。v1.L) が使う (06 X10)。ただし AI の除外では、ワークスペース層が除外を **緩める** 指定は効かせない (06 §6.5 の「ワークスペース設定は除外を追加することだけができる」): ワークスペース層の `workspace.exclude` の否定 (`!`) の行と、ワークスペース層の `search.workspace.respect_gitignore = false` は、AI の範囲の照合では使わない。ファインダ・横断検索では使う (範囲を広げても、何も外へ送らない)。

#### 12.7.4 ファインダ (FR-125)

- **開く**: `workspace.open_file` (既定キー `mod+o`、07 §11.6) で、フォーカスのあるペインに対して一覧のオーバーレイ (`OverlayModel::List`、`overlay.active == "finder"`) を開く。オーバーレイは画面全体に 1 つで、開いたときにフォーカスのあったペインに対して働く (ペインの細目は M30 の仕様)。引数 `query?` があれば入力欄に入れて開く。走査は §12.7.2 の「一覧の寿命」のとおり始める。
- **照らす対象**: 候補のルートからの相対パス (区切りは `/`)。検索語は前後の空白を除き、空白で区切った語の **すべて** が一致する候補だけを残す (AND)。Windows では検索語の `\` を `/` として照らす。大文字小文字は ASCII の英字だけを常に畳む (検索語に大文字があっても区別しない。全角英字・ASCII の外の文字は畳まない)。各語は、語の文字が相対パスの中に順に現れること (部分列) で一致とする。
- **採点** (S-26 の試作の fzf v1 の形。値は案で、下の性質を試験で固定し、性質を保つ範囲で実装の PR が直してよい): 語ごとに、前から当てて一致の終端を決め、後ろから当て直して最短の窓にし、窓の中を前から採点する。一致した文字 1 つにつき 16、区切り (`/`・`_`・`-`・`.`・空白) の直後と先頭は +8、camelCase の境目・英字と数字の境目は +7、直前の文字と連続した一致は +4 (語の最初の文字の加点は 2 倍)、間の最初の 1 文字は −3、続く間は 1 文字ごとに −1。各語は、まずファイル名 (最後の `/` の後ろ) の中だけで当て、当たればその窓で採点して語の文字数 × 8 を足す (ファイル名の中に収まった語として数える)。当たらなければ相対パス全体で当てる。候補の点は語の点の合計。性質 (試験で固定する): (a) ファイル名の中だけで一致する候補は、同じ語でディレクトリにまたがって一致する候補より上 (01 FR-125 AC-1 の「ファイル名に一致する候補を上位に」)。(b) 連続した一致は、同じ文字の飛び飛びの一致より上。(c) 語頭での一致は、語の途中での一致より上。並びは、ファイル名の中に収まった語の数の多い順 → 点の高い順 → 相対パスの短い順 (バイト数) → バイト列の順 (下の段 6 の細目 (1))。
- **件数**: 上位 **100 件** (`FINDER_TOP = 100`) だけを持って一覧に出す (表示の窓は 12 行、04 §8.11)。一致した件数はすべて数える。検索語が空なら、相対パスの短い順 (同じ長さはバイト列の順) の 100 件を出す。
- **照合のジョブ**: `JobKind::FinderMatch { chunks: Vec<Arc<[Candidate]>>, query: Arc<str>, top: usize, generation }` (`bg`。キューの順は検索と同じ段)。照合はメインスレッドで行わない (01 FR-125 AC-2)。候補 4,096 件ごとに `CancelToken` を見る。結果は `JobResult::FinderMatched { generation, matched: usize, top: Vec<FinderHit { chunk: u32, index: u32, score: i32, ranges: Vec<Range<u32>> /*相対パスの書記素番号の半開区間*/ }> }` (`top` は 100 件まで)。主スレッドは表示の窓に入る行だけを `ListItem` に組む。
- **取消しと投げ直し**: 入力欄が変わるたびに、走っている照合のジョブを取り消して新しい照合を投げる (01 FR-125 AC-2)。結果は世代で照らし、古い世代を捨てる。最初の走査の間は、新しい候補が届いていて、前の照合を始めてから 100 ms 以上経っていれば同じ検索語で照合し直す (束が届くたびに投げ直すと、`bg` に照合が積み上がる。最後の束の後は必ず 1 回照合する)。同じ検索語での照合し直し (最初の走査の間の照合し直しと、走査し直した一覧への差し替えの後の照合) では、選択中の候補のパスが新しい上位 100 件に残っていれば選択を保ち、残っていなければ先頭を選ぶ。入力欄が変わったときは、次に届いた結果で先頭を選ぶ (パレットと同じ。`Enter` で開くのは今の検索語の最上位)。
- **表示** (04 §8.11): 見出し `ui.finder.title` (ルートを `{root}` で示す)、入力欄の案内 `ui.finder.placeholder`、行は相対パス (`ListItem.label`。双方向の制御文字・C0 などは編集面と同じ可視の記法 (FR-507、§14.2) にしてから入れ、`match_ranges` は記法に直した後の書記素の番号で数える。一致した文字を強調。`detail` は使わない)、件数 `ui.palette.footer` (`{matched}/{total}`)。走査の間は `ui.finder.footer_scanning`、上限で打ち切ったときは `ui.finder.footer_truncated`、読めないフォルダがあれば `ui.finder.unreadable_suffix` を足す。0 件は `ui.finder.no_match`。境界の型 (eve-view) は変えない。
- **開く** (`ui.confirm`、`Enter`): オーバーレイを閉じ、選んだ候補の絶対パス (ルート + 相対パス。名前が UTF-8 で表せないときは元の `OsString`) を、フォーカスのあるペインで開く (`file.open` と同じ経路。03 §9.2。同じファイルの判定は §12.7.6)。開くジョブには `OpenOptions.must_exist = true` を渡し、走査の後に消えたファイル (`NotFound`) は新規の文書にせず `msg.finder.not_found` を出す (02 §9.3)。リンク先の無いシンボリックリンク (`NotFound` + `DanglingLink`) は `msg.finder.dangling_link` (リンク先を示す) を出す。キャンセル (`Esc`) では何も変わらない (01 FR-125 AC-1)。
- **キー** (07 §7.1 の `finder` コンテキスト。参照順は `finder` → `list` → `prompt` → `global`): 候補の移動は `list` の `up` / `down`・`ctrl+p` / `ctrl+n`・`pageup` / `pagedown`、入力欄の編集は `prompt`。v0.5 で `finder` コンテキストに置く既定の割当は横断検索のトグルだけ (§12.7.5)。
- **段 6 の実装で決めた細目** (M31 の段 6 の PR、2026-11-22。`eve_core::{finder_match, FinderQuery, FinderHit}` と core の `workspace/finder.rs`): (1) **並び** (推奨案を採用 (PO、2026-11-22)): ファイル名の中に収まった語の数の多い順 → 点の高い順 → 相対パスの短い順 (バイト数) → バイト列の順。語の文字数 × 8 の加点だけでは、ファイル名の中の一致の間の減点が大きいときに性質 (a) が崩れる (`ab` で `a/b/x.rs` が 53 点、`dir/xaxb.rs` が 45 点) ので、ファイル名の中の語の数を点より先に比べる。点 (`FinderHit.score`) は上の案の値のまま (性質 (b)・(c) は同じ段の中の比べ方)。**語はまずファイル名の中だけで当てる** (前から当てた窓だけで決めると、ディレクトリ名にも同じ語がある候補 (`src/main/java/Main.java` の `main`、`cmd/server/server.go` の `server`) がファイル名の中に収まらないと数えられ、`lib/domain.rs`・`pkg/observer.go` より下になる)。ファイル名の中で当たれば、強調もその窓に付ける。退けた案: ファイル名の加点を大きくする (例 +1,000): 語の数と長さで `i32` の範囲と点の意味が崩れる。(2) **文字の単位**: 検索語の 1 文字は候補の 1 文字にだけ当たる (ASCII の外の文字は UTF-8 のバイト列の全体で比べる。先頭のバイトは文字の境目にしか現れないので、別の文字の途中には当たらない)。語は Unicode の空白 (全角の空白を含む) で区切る。境目の加点では、ASCII の外の文字を英字と同じに扱う (数字との境目に数える)。間の減点は文字の数で数える。連続の加点と境目の加点は足す。(3) **強調の位置**: 照合のジョブは上位に入った候補だけを当て直して、一致した文字を含む書記素の番号の区間を返す。主スレッドは、行を `cell_text` と同じ規則で記法に直すときの「書記素 → `Span`」の写しで、`match_ranges` を記法の後の番号に直す (幅 0 で落とした書記素は強調しない)。(4) **照合を待つ間の `Enter`**: 今の入力欄の照合の結果が届くまで待ち、届いたらその選択 (先頭) を開く (前の検索語の行を開かない)。入力を変えると待ちを解く。今の世代の照合が失敗した (パニック) ときも待ちを解く。出している行が今の世代の結果でなく、照合も走っていないとき (照合の失敗の後) の `Enter` は何もしない (前の検索語の行を開かない。入力を変えると照合し直す)。今の世代の照合が失敗した (取消しでない) 時点で、メッセージ行に `msg.finder.match_failed` (「ファイルの絞り込みに失敗しました: {reason} / 入力を変えると絞り込み直します」) を出す (M31 の段 7 の PR で足した。`Enter` が黙って何もしない状態にしない。横断検索の `msg.workspace.search_failed` と揃えた)。(5) **行と塊の解放**: 一覧に出す行は塊の `Arc` と番号を持つ (一覧を差し替えても、次の結果が届くまで行は有効)。主スレッドで塊の参照を外すときは、一覧 (と走査の途中の塊) が持っている塊はそのまま外し、それ以外 (差し替えた古い一覧・古い世代の束・取り消した照合の写し・閉じたときの行) は `ReleaseCandidates` で `bg` に渡す。(6) **開き直し**: 走査が走っている間に開き直しても、新しい走査は始めない (走っている走査が終わると新しい一覧になる)。退けた案: 取り消して始め直す: 最初の走査の進みを捨て、開閉のたびに I/O をやり直す。走査を始めるとき、一覧が空 (最初・手放した後・前の走査が候補を 1 つも出さずに失敗した) なら最初の走査として束をそのまま足す (空の一覧を見せたまま、終わるまで候補を出さないことにしない)。(7) **起動の `DIR`**: ルートの検出の結果が `RootSource::Dir` のとき、ほかのオーバーレイが出ていなければファインダを開く (出ていれば開かない。確認・入力を置き換えない)。(8) **除外の設定**: 設定のキー (`workspace.exclude`・`search.workspace.respect_gitignore`・`search.workspace.follow_symlinks`) は予約のままにし、07 §5.9 の既定 (`**/.git`・`**/node_modules`・`**/target`、`.gitignore` を読む、リンクを辿らない) を定数で渡す (公開は段 1 の細目 (1) のとおり、`[workspace]` のキーをまとめて公開する段)。(9) **見出し** (推奨案を採用 (PO、2026-11-22)): `{root}` は `FsPath::display` の形。長いときは `CutPolicy::Middle` で中間を詰める (境界の型に `Start` が無いので、前を詰める案は採らない。eve-tui の枠の見出しは `cut` に従って切る)。ルートの検出を待つ間の `{root}` は `…` (ASCII に縮退した端末では `...`)。(10) **消えたファイル**: `msg.finder.not_found` は警告。リンク先の無いシンボリックリンク (走査は候補にする、§12.7.2 の段 4 の細目 (f)) は、開くジョブの `metadata` が `NotFound` + `DanglingLink` を返すので、`msg.finder.dangling_link` (警告。リンク先を示す) で知らせる (「もうありません」は実態と合わない)。(11) **一覧の寿命の時計**: 閉じた時刻から `FINDER_LIST_KEEP` (5 分) を `Editor::tick` で数え (`next_deadline` に入れる)、最初の走査の間の照合し直し (100 ms、`FINDER_RESCORE_INTERVAL`) も同じ時計で待つ。(12) **走査の失敗**: ルートを読めない (`Failed { Fs }`) ときは `msg.workspace.scan_failed`、走査のジョブがパニックした (`Failed { Panic }`) ときも同じ文言 (`{reason}` は `panic`) を出す。どちらも、その走査で作った一覧は欠けているので、件数の欄を `ui.finder.footer_incomplete` (「一覧が途中までです」) にする (完了の `{matched}/{total}` に見せない)。次の走査が終われば戻る。**既知の制約** (直していない): **名前の Unicode の正規化を揃えない**: 照合はバイト列で行うので、macOS の FS が返す NFD の名前 (`データ` の `デ` が `テ` + 結合文字) に、IME が入れる NFC の検索語が一致しない (v0.5 の配布物は Windows と Linux。macOS へ移すとき (OQ を起票する) に、候補の照合用の文字列と検索語を NFC に揃えるかを決める)。ファインダの中の `Ctrl+W` は、パレットと同じく `list` の `buffer.close` に届き、今の文書を閉じる (バッファ一覧の外では選択中の候補を閉じる意味が無い。v0.4 までのパレットと同じ振る舞い)。

#### 12.7.5 横断検索 (FR-405)

- **開く**: `search.workspace` (既定キー `mod+shift+f` [E][横] と `ctrl+k f` (alt)、07 §11.6) で、一覧のオーバーレイ (`OverlayModel::List`、`overlay.active == "workspace_search"`) を開く。入力欄の初期値は、検索バーから開いたときはその検索語、編集面から開いて選択が 1 行の中の 256 バイト以下ならその文字列、それ以外は前回の横断検索の検索語 (ワークスペースが同じとき)。引数 `query?` があればそれを入れる。
- **検索語の指定**: 大小の区別 (`smart` / `sensitive` / `insensitive`)・正規表現・単語単位は、エディタ内の検索 (FR-401 / FR-402) と同じ `SearchQuery` で表し、同じコマンド (`search.toggle_case` / `search.toggle_regex` / `search.toggle_word`) で切り替える。3 つのコマンドの `when` は `search.active || overlay.active == "workspace_search"` に広げ、横断検索のオーバーレイが開いているときはオーバーレイの検索語に効く (検索バーが同時に開いていても、フォーカスのあるオーバーレイが先。検索バーのトグルは変えない)。キーは `finder` コンテキストに置く (`alt+c` / `alt+r` / `alt+w` と `ctrl+k c` / `ctrl+k x` / `ctrl+k b`。07 §11.6 の「v0.5 の M31 で足す割当」)。初期値は、検索バーから開いたときはその値、それ以外は `search.case` / `search.regex_default` と単語単位 OFF。選択範囲内 (`search.toggle_selection`) は無い。正規表現の構文の誤りは、件数の位置に検索バーと同じ文言で出し、検索しない。
- **検索を始める時機**: 入力欄・トグルが変わるたびに、走っている検索を取り消して新しい検索を始める (待ち時間を置かない。エディタ内の検索と同じ。01 FR-405 AC-3)。検索語が空なら検索せず、結果を空にする。
- **ジョブ**: `JobKind::WorkspaceSearch { root, generation, query: SearchQuery, exclude: Arc<ExcludeConfig>, open_docs: Vec<OpenDocRef /*パス・canonical・FileId・file_id_unique・中身 (Snapshot か、飛ばす印)。§12.7.6 の 3*/>, max_file_size: u64, max_lines: usize, state: Option<Box<WorkspaceSearchState>> }` (`bg`。キューの順は検索と同じ段)。`WorkspaceWalk` (§12.7.2) でファイルを見つけ、見つけた順に 1 つずつ探す。1 区切りは「ディレクトリ 16 個・探したファイル 64 個・探したファイルの中身の合計 1 MiB・区切りの始めから `Clock` で 8 ms」のどれか早いほう (1 つのファイルは途中で区切らない。小さいファイルが多いディレクトリで 1 区切りに何千回も開かないように、ファイルの数と時間でも区切る。中身を読む費用は S-26 で測っていないので、値は BM-405 で確かめて直す) で、続きを自分のキューの末尾へ積み直す。`CancelToken` はファイルごとと、復号・照合の 1 MiB ごとに見る。
- **1 つのファイルの扱い** (01 FR-405 AC-2):
  1. **開いている文書**: 候補が開いている文書と同じファイルなら、ディスクではなく検索を始めた時点の Snapshot を探す (保存していない編集を含む)。突き合わせは §12.7.6 の 3。ジョブに渡す `open_docs` は、canonical のパスがルートの下にある文書だけにする (ルートの外の文書の Snapshot を検索の間持ち続けない)。Snapshot の UTF-8 のバイト数が `max_file_size` を超えるとき、または文書がバイナリとして読み取り専用で開かれている (§8.2 / §9.6 の `Binary`) ときは、Snapshot を渡さずに飛ばす印 (`OpenDocBody::TooLarge` / `Binary`) だけを渡し、ジョブはその文書のファイルを飛ばして数える (ディスクも探さない)。読込中 (`Loading` / `Partial`) の文書はディスクを探す。
  2. **大きさとオフライン**: `DirEntry.offline` (Windows の `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS` / `RECALL_ON_OPEN` / `OFFLINE`。OneDrive のオンライン専用のファイルなど、05 §2.1) のファイルは開かずに飛ばす (読むとクラウドから呼び戻し、ネットワークを待ってディスクを埋める。ファインダには候補として出す)。`DirEntry.len` (列挙で分かるときだけ。Windows) が `max_file_size` を超えれば開かずに飛ばす。分からなければ開いて読み、`max_file_size` + 1 バイトを読んだ時点でやめて飛ばす (`metadata` を 1 件ずつ呼ばない。Windows の NTFS で 10 万回の `stat` は数秒かかる)。
  3. **判定と復号**: ファイルは `open_read` で開き (開いた後のハンドルで通常のファイルと確かめる、05 §2.1。Windows はルートの canonical な形に verbatim の接頭辞を付けたパスで開き、末尾のドット・空白・予約名の名前を別のファイルやデバイスに読み替えない)、読込と同じ判定 (§8.2。BOM・判定の候補・判定し直し (§8.2.1)。読込と同じ関数の `DetectOptions` の既定を使う。読込が `file.default_encoding`・`file.encoding_detection`・`file.detection_candidates` を使うようになったら、同じ値をジョブに渡す) で文字コードを決め、不正バイトのエスケープ (§8.3) を含めて復号する。1 つのファイルの読込は `max_file_size` に依らず 64 MiB (`WS_MAX_DECODE_BYTES`。定数) までにする (`max_file_size` を大きくしても、復号で膨らんだ 1 ファイルで `bg` が数 GB を持たない)。バイナリの判定 (§8.2 手順 4) と UTF-32 の BOM (手順 1) のファイルは飛ばす。不正バイトの比率が高い (読込なら確認を出す `ManyInvalidBytes`) ファイルは、確認を出さずに復号したまま探す。
  4. **照合**: エディタ内の検索の照合器 (`Matcher`。正規表現・大文字小文字・単語単位・CP932 互換文字の同一視 (§12.4.1)・`\n` の扱い (§12.3 / §12.3.1)) で、復号した本文を探す。最悪の入力の上限 (§12.3.1) もファイルごとに同じに効かせ、打ち切ったファイルは飛ばした数に数える。
  5. **読めないファイル**: 開けない・読込の途中で失敗したファイルは飛ばした数に数える (理由は区別しない。v0.5 ではログに残さない。core にジョブからログを書く出口が無いため。M31 の段 7 の PR で、それまでの「ログには残す」を実装に合わせて直した)。
- **結果**: 一致を **行ごと** に 1 件にする (同じ行の一致は 1 件にまとめ、強調の範囲を複数持つ)。`WsHit { path: FsPath /*絶対*/, rel: Arc<str> /*ルートからの相対パス、区切りは `/`*/, line: u32 /*0 始まり*/, ranges: Vec<Range<u32>> /*その行の中のバイト位置 (復号した UTF-8 の行。行をまたぐ一致は行末までを強調し、位置は始まりの行に置く)。抜粋に掛かるものを 16 個まで持ち、それより多い一致と抜粋の外の一致は持たない (表示しない)。最初の要素はその行の最初の一致 (ジャンプの位置)*/, excerpt: Box<str> /*行の内容。最初の一致の前後を合わせて 256 バイトまで (文字の境界で切り、先頭の空白は詰める)*/, excerpt_start: u32 /*抜粋の始まりの、行の中のバイト位置*/, from_open_doc: bool }`。`ranges` は行の中の位置で持ち、一覧の行 (`ui.workspace_search.item`) を組むときに `excerpt_start` を引いて抜粋の中の位置に直し、抜粋の外に出る部分は切り落としてから、行全体の書記素番号 (`ListItem.match_ranges`) に写す。`JobResult::WorkspaceHits { generation, hits: Vec<WsHit>, progress: WsProgress { files: u64 /*探したファイル*/, hit_files: u64 /*一致のあったファイル*/, skipped_large: u64, skipped_binary: u64 /*対応していない BOM を含む*/, skipped_other: u64 /*読めない・オフライン・最悪の入力の上限で打ち切った*/, unreadable_dirs: u64 } }` を区切りごとに送り、終端は `JobResult::WorkspaceSearchDone { generation, progress, truncated: bool }`。
- **上限**: 結果は **10,000 行** (`WS_MAX_RESULT_LINES`。定数。`search.max_matches` の既定と同じ桁) で打ち切り、走査もやめて打ち切りの印を立てる (01 FR-405 AC-3)。探すファイルの数は §12.7.2 の 20 万件の上限に従う。
- **表示** (04 §8.12): 見出し `ui.workspace_search.title` (トグルの状態を検索バーと同じ記号 `[Aa~] [.*] [ab]` で示す)、行は `ui.workspace_search.item` (`{path}:{line}: {text}`。行番号は 1 始まり。`{text}` は抜粋で、制御文字・不正バイトのエスケープは編集面と同じ可視の記法、§14.2)。一致の箇所を `match_ranges` で強調する。件数は検索の間 `ui.workspace_search.footer_running`、終わったら `ui.workspace_search.footer_done`、上限で打ち切ったら `ui.workspace_search.footer_truncated`、飛ばしたファイルがあれば `ui.workspace_search.skipped_suffix` を足す (01 FR-405 AC-2 の「飛ばした件数が示される」)。0 件は `ui.workspace_search.no_match`。境界の型 (eve-view) は変えない。
- **ジャンプ** (`ui.confirm`、`Enter`): オーバーレイを閉じ、結果のファイルをフォーカスのあるペインで開く (開いていればその文書へ切り替える。§12.7.6。`OpenOptions.must_exist = true`。走査の後に消えたファイルは新規の文書にせず `msg.finder.not_found`、ファインダと同じ)。検索バーが開いていれば閉じる (開いたままだと読込の完了の表示の切り替えが保留される、§9.2.1)。今の文書の中の一致へは、開き直さずに (「既に開いています」を出さずに) すぐ移る。読込が終わった時点で、その文書がまだそのペインに出ていれば一致の位置へ移る (読込の途中で別の文書に切り替えたら移らない。読込の間にほかのオーバーレイを開いて表示の切り替えが保留されたときは、切り替えた時点で移る)。位置は `(line, ranges[0])` を今の本文に当てる: 行が無ければ最後の行、行の中のバイト位置が行より長ければ行末、書記素の境界でなければ前の境界に丸める。丸めずに当てられた (範囲の終わりも行の中の書記素の境界にある) ときは一致の範囲を主の選択にし (検索の `search.next` と同じ。行をまたぐ一致は `ranges[0]` が始まりの行の行末までなので、そこまでを選ぶ)、丸めたときはカーソルだけを置く。文書を中央に寄せてスクロールする。
- **結果を保つ**: オーバーレイを閉じても (ジャンプ・`Esc`)、最後の検索語・トグル・結果・選択中の位置を持ち、次に開いたときにそのまま出す (ワークスペースが同じとき。結果は開いたときに探し直さない。入力を変えると探し直す)。結果を持っている間に新しい横断検索を始めたら、古い結果は `ReleaseCandidates` (`hits` に入れる) で `bg` に渡して解放する (10,000 行の抜粋を主スレッドで解放しない)。
- **段 7 の実装で決めた細目** (M31 の段 7 の PR、2026-11-22。`eve_core::{WorkspaceSearchState, WsHit, WsProgress, OpenDocRef}` と core の `workspace/ws_search.rs` (ジョブ)・`ws_search_ui.rs` (主スレッド)): (1) **設定のキー**: `search.workspace.max_file_size`・`respect_gitignore`・`follow_symlinks` と `workspace.exclude` は予約のままにし (PO の決定 2026-11-21。上限は定数)、07 §5.5 / §5.9 の既定 (2 MiB、`.gitignore` を読む、リンクを辿らない、`**/.git`・`**/node_modules`・`**/target`) を定数 (`WS_MAX_FILE_SIZE` など) で渡す (段 6 の細目 (8) と同じ)。(2) **区切り**: 走査 (§12.7.2 の 1 区切り) は 1 回の区切りに 1 回まで呼び、見つけた候補を列に溜めて、その区切りの残りでファイルを探す。区切りの最初の 1 回 (走査かファイル 1 つ) のほかは、走査を呼ぶ前もファイルを探す前も、上限 (ファイル 64 個・中身 1 MiB・区切りの始めから 8 ms) を見て、どれかに達していれば譲る。走査は、区切りの始めからの残りの時間を時間の区切りにして呼ぶ (`WorkspaceWalk::step_within`。区切り全体で 8 ms。ディレクトリの列挙 1 回は区切れない)。ファイルの数には飛ばしたファイル (開いて読み始めたもの・開かずに飛ばしたもの) も 1 つと数え、中身の合計には飛ばすまでに読んだバイト数 (大きさの上限 + 1 バイトまで読んで飛ばしたファイル・読んでからバイナリと分かったファイル) も足す。(3) **結果の束**: 区切りごとに、結果があれば `WorkspaceHits` を送る。結果の無い区切りが続くときは、進捗が変わっていれば 100 ms (`WS_PROGRESS_INTERVAL`) ごとに空の `hits` で進捗だけを送る (探したファイルの数が出る、01 FR-405 AC-3)。(4) **抜粋**: 最初の一致の前は 64 バイト (`WS_EXCERPT_BEFORE`) まで、残りを一致とその後ろに使う。行の終わりに近い一致は、前を広げて 256 バイトを使う。始まりは後ろへ、終わりは前へ書記素の境界に揃える (結合文字・ZWJ の並びの途中で切ると、一覧の行で前の文言の文字と 1 つの書記素になる)。一覧の行で抜粋の位置を探すときは、抜粋を空にして組んだ文言との前後の共通部分から位置を決め、候補が 2 つ以上あれば後ろのものを採る。先頭の空白は、最初の一致の手前までだけ詰める (空白に一致した語の強調を失わない)。(5) **上限**: 結果の行の上限は、上限を **超える** 行を見つけた時点で打ち切る (ちょうど上限の行数で終わる検索は打ち切りの印を立てない。§12.7.2 の段 4 の細目 (c) と同じ)。打ち切りの印は、走査の上限 (20 万件など)・列挙の途中の失敗 (§12.7.2 の 6 / 7) でも立て、件数の欄は同じ `ui.workspace_search.footer_truncated` にする。(6) **飛ばしたファイルの分け方**: 大きさの上限 (列挙で分かった大きさ・読んだ量・開いている文書の本文) は `skipped_large`、バイナリと対応していない BOM (UTF-32) は `skipped_binary`、読めない・オフライン・最悪の入力の上限で打ち切ったファイルは `skipped_other`。`follow_symlinks = false` のシンボリックリンクは読まず、どれにも数えない (ファイルとして探していない)。(7) **入力が変わったとき**: 走っている検索を取り消し、出している結果を `bg` で手放してから (`ReleaseCandidates` の `hits`)、空の結果で新しい検索を始める (前の検索語の結果を見せたまま `Enter` で開かない。ファインダと違い、照合を待つ `Enter` の待ちを作らない)。検索語が空 (件数の欄も空にする。ルートの検出を待つ間も `footer_waiting` を出さない)・正規表現の誤り (件数の位置に検索バーと同じ `msg.search.invalid_regex.text`)・どのファイルでも探さない検索語 (上限の無い重い検索語、§12.3.1。件数の位置に検索バーと同じ `msg.search.pattern_too_heavy.text`) では探さない。コンパイルで分かる誤り (大きさの上限) はジョブが照合器を作れずに何も探さずに終わり、主スレッドが `known_error` で件数の位置に出す。(8) **開く・結果を保つ**: 入力欄の初期値の優先は、引数 `query` → 開いている検索バーの検索語 (空でないとき。トグルも引き継ぐ) → 編集面の主の選択 (空でなく、1 行の中の 256 バイト以下) → 前回の検索語。前回と同じ検索語 (検索バーから開いたときはトグルも同じ) なら、前回の状態 (結果・選択・トグル) をそのまま出し、探し直さない。違えば、前回の結果を `bg` で手放し、トグルを検索バー (から開いたとき) か設定 (`search.case`・`search.regex_default`、単語単位 OFF) の値にして探す。閉じたとき (ジャンプ・`Esc`) に走っていた検索とルートを待っていた検索は取り消し、次に開いたときに同じ検索語で探し直す (途中までの結果を、終わった結果に見せない)。正規表現の `\n` の糖衣と CP932 互換文字の同一視は、検索を始めるたびに設定 (`search.newline_sugar`・`search.cp932_compat_fold`) の値を使う。(9) **トグル**: `search.toggle_*` は、横断検索のオーバーレイが開いていればオーバーレイの検索語に効き、検索バーのトグルは変えない。オーバーレイの入力欄の横取り (モーダル) からこの 3 つを外す。(10) **開いている文書との突き合わせ** (§12.7.6 の 3): (i) は、候補の絶対パス (ルート + 相対パス) と文書の canonical のパスを、Windows の verbatim の接頭辞を外した形 (`canonical_key`) にして要素ごとに比べる。(ii) は、信頼できる識別を持ち、ファイル名が同じ文書があるときだけ (識別の無い文書は (i) の比べ方だけ)、その候補に `metadata` を 1 回と canonicalize を呼んで §12.7.6 の 0 の比べ方で確かめる (NTFS など識別を信頼できる FS では、開いている文書と同じ名前の候補 (`mod.rs` のようなよくある名前を開いていれば、その名前の候補のすべて) ごとに呼ぶ)。識別が候補と等しい文書は、文書のパスを `metadata` し直して今も同じ識別かを確かめ、違えば (作り直された・番号を使い回した) その文書とは識別で比べない (ジョブの中だけで下ろし、主スレッドの文書の識別は変えない)。ルートの下かは、canonical のパスを要素ごとに (Windows・macOS は ASCII の大文字小文字を畳んで) 比べる。(11) **ジャンプ**: `Enter` で、開いた時点のペインを覚え、検索バーを閉じる。開いている文書 (パスの文字列か、canonical のパスの比べる形 (`canonical_key`) が結果のパスと同じ。別の書き方のパスで開いた文書を含む) なら、開くジョブを走らせずに (「既に開いています」「…として開いています」を出さずに) その文書へ切り替え、読み終えていればすぐに移り、読込中なら読込の完了 (`complete_open`) で、そのペインにまだ出ていれば移る (再読込・文字コードを指定して開き直す間の文書は、その完了 (`finish_reopen`。失敗して旧本文に戻ったときも) で移る。確認を待つ間は持ち越す。M31 の段 7 の PR のレビューで直した)。開いていなければ開き、読込の完了で同じく移る (表示の切り替えが保留されていれば、`try_open_switch` で切り替えた時点で移る。利用者がそのペインを別の文書に切り替えたら捨てる)。開くジョブの識別で既存の文書と同じファイルと分かった (§12.7.6 の 1) ときは、既存の文書に移す (読み終えていて、開いた時点のペインに出ていれば、出した直後に移る。ほかのペインのカーソルは動かさない)。ヒントバーは、横断検索の一覧でもほかの一覧と同じく、トグルのキーを示さない (`overlay` 文脈のヒント、07 §11.6 は検索バーのもの)。(12) **失敗**: ルートを読めない (`Failed { Fs }`) ときは `msg.workspace.scan_failed`、ジョブのパニックなどは `msg.workspace.search_failed` (「ワークスペースの検索に失敗しました: {reason} / 検索語を変えると探し直します」) を出し、件数の欄を `ui.workspace_search.footer_failed` (「{lines} 件 (検索に失敗しました)」) にする (届いていた結果は残し、`Enter` で開ける。検索中の表示のまま止めない)。(13) **Windows の読込のパス**: verbatim の形のルートはジョブの状態に 1 回だけ作る。接頭辞を付けるのは、ルートがドライブ文字 (`C:\`) か UNC (`\\server\share`) の形のときだけ (`\\?\` / `\\.\` で始まるルートはそのまま)。結果の `path` (開くパス) は接頭辞の無い形のまま (ファインダと同じ)。**既知の制約** (直していない): 1 つのファイルは途中で区切らないので、上限 (2 MiB) に近いファイルの復号と照合で 1 区切りが 8 ms を超え得る (`bg` のほかのジョブがその間待つ。BM-405 で見る)。ファイルは 1 つずつ `bg` の 1 本のレーンで開いて読むので、10 万ファイルの木を最後まで探すのに数十秒かかる (開発機の BM-405。レーンの構成は OQ-103 として RC の基準機の値で判断する)。開いている文書との突き合わせ (§12.7.6 の 3) は、ルートの外のパスで開いた同じファイルを見分けない (`follow_symlinks = true` を開けるときに見直す)。名前の Unicode の正規化 (§12.7.4 の既知の制約) は横断検索の相対パスの表示にも同じく効かない。
- **v0.5 に含めないもの**: 横断の置換 (01 FR-405 AC-1)、次 / 前の結果へ移るキー (オーバーレイを開き直して選ぶ)、結果の下部パネル (04 §11、後の版)、ファイルの種類・パスでの絞り込み (`--glob` 相当)。

#### 12.7.6 同じファイルの判定 (SAF-28)

v0.2〜v0.4 の実装は、二重オープンも名前を付けて保存の保存先も、正規化したパスの文字列を比べるだけだった (02 §9.3 の `JobResult` の注記、03 §9.7 (a))。ファインダ・横断検索は別の書き方のパス (大文字小文字・シンボリックリンク・ハードリンク・`..`) で同じファイルを開き得るので、**ファインダの PR の前に** 次を入れる (08 §2.6 の順序)。

0. **比べ方** (2 つのファイルが同じか): (a) どちらも **信頼できる `FileId`** を持ち、それが等しく、かつ両方の canonical のパスの **ボリュームの部分** が等しい、**または** (b) どちらも canonical のパス (05 §3.3 の表示用の形に揃えたもの) を持ち、その文字列が等しい、のどちらかなら同じとする (大文字小文字を自前で畳まない、05 §3.1)。**ボリュームの部分** は、Windows ではドライブ文字 (`C:`)・UNC のサーバとシェア (`\\server\share`)・`\\?\Volume{…}` の名前・`\\?\GLOBALROOT\Device\…` の装置名で、ASCII の大文字小文字だけを畳んで比べる。Unix では条件を置かない (`FileId` に `st_dev` が入っている)。どちらかのボリュームの部分が分からなければ (a) は使わない。(a) にボリュームの条件を置くのは、ディスクの複製・マウントしたバックアップの VHDX・シャドウコピーで、ボリュームのシリアル番号と `FileId` がそのまま重なるため (C: の文書を複製先の E: へ名前を付けて保存すると、(a) の表の 1 に当たって E: に何も書かない)。ハードリンクは同じボリュームの中にしか作れないので、ハードリンクの検出は失われない。同じボリュームの複製を別のフォルダにマウントしたもの (`C:\mnt\clone`) はこの条件で見分けられない (05 §5.4)。信頼できる `FileId` は、`FileMeta.file_id_unique` (02 §4.5。eve-platform が、Windows は NTFS / ReFS のローカルのボリュームで 0 でない `FileIdInfo` が取れたとき、Unix は `FsKind::Local` のときに立てる) が立ったものだけ。FAT・exFAT・`Network`・`Unknown` の FS の `FileId` は、ディレクトリの項目の位置から作られて使い回される・複製したボリュームでシリアル番号が重なる・サーバが 0 や重なる値を返すことがあるので使わない (別のファイルを同じと判定すると、名前を付けて保存で何も書かずに済ませてしまう)。文書の `FileId` は別の値として持たず、`disk_stamp.file_id` (保存・再読込・外部変更の受け入れのたびに直る。原子的保存で `FileId` は変わる、05 §5.4) から取る。canonical のパスは文書の `Document.canonical` (境界の型ではない) に持ち、開いたとき・名前を付けて保存したときに直す。canonical を取れなかった文書 (canonicalize の失敗) は、正規化したパス (05 §3.2) の文字列を代わりに使う。**代わりのパスでは (a) を使わない**: ジョブは、ファイルが在るのに `path` 自身を canonicalize できなかったとき (親を canonicalize してファイル名を付けた形で代えたときも、canonical が取れなかったときも) 結果の `file_id_unique` を `false` にし、主スレッドも canonical が無い結果は `false` として扱う。シャドウコピーやドライブ文字の無いボリュームの先 (`mklink /d` のリンクの先) は DOS の名前が無いので canonicalize に失敗し、打ったパスのボリューム (`C:`) で (a) を使うと、複製の識別が元と一致して同じと誤判定する (名前を付けて保存では元のファイルを上書き保存してしまう) ため。新規のファイルとして開いた文書 (`NewFile`) は、親ディレクトリを canonicalize してファイル名を付けたものを canonical にする (開くジョブが `NewFile` の前に `Identity` を送る)。比べる文字列は、Windows では verbatim の接頭辞を外した形に揃える (05 §3.3 の表示用の形は 260 文字を超えるパスで接頭辞を残すので、そのままでは「ルート + 名前」で作ったパスと揃わない)。canonical のパスは OS の文字列のまま比べ、Unicode の文字列に変えない (Windows の対になっていないサロゲートを置換文字にすると、別の名前が同じ文字列になる。05 §3.3)。**古い識別の確かめ**: 文書の `FileId` は `disk_stamp` の値なので、ファイルが外で作り直されると古いまま残り、その番号を別のファイルが使うことがある (Linux の ext4 などは削除した inode の番号をすぐ使い回す)。そこで開くジョブと名前を付けて保存の照合のジョブに、投入した時点の文書のうち信頼できる識別を持つもののパスと識別の一覧 (`known_ids`、02 §9.3) を渡す。ジョブは、調べたファイルの識別が一覧のどれかと等しければ、その文書のパスを `metadata` し直し、今は同じ識別でないもの (作り直された・消えた・`metadata` に失敗した) を結果の `stale` (02 §9.3) で返す。主スレッドは比べる前に、`stale` の文書 (パスと識別が一覧のときのまま) の `file_id_unique` を `false` に下ろす (その文書は以後 (b) だけで比べる。再読込・開き直しの `Identity` で信頼を戻す)。調べたファイルの識別は使えるまま (ほかの文書とのハードリンクの検出は失わない)。一覧に無い文書 (投入の後に開いた文書。識別は開いた直後の新しい値) との (a) の一致は、確かめずに使う。
1. **開く**: 開くジョブは `metadata` と `canonicalize` の後、読込と確認 (§9.2 手順 2 の `HugeFile`・手順 3 の `Binary` などの `NeedsConfirm`) の **前に**、非終端の `JobResult::Identity { file_id: Option<FileId>, file_id_unique: bool, canonical: Option<FsPath>, stale: Vec<KnownFileId> }` を送る (§9.2 手順 2b。確認を待つ文書も identity を持つので、別の書き方のパスで開いた同じ巨大なファイルが 2 つの文書にならない)。主スレッドは、ほかの文書 (読込中・確認待ちを含む) と 0 の比べ方で比べる。同じなら新しい文書を閉じ、ジョブを取り消し、既存の文書をそのペインに出す。開いたパスと既存の文書のパスが (正規化した文字列で) 違えば `info.open.same_file` (「{path} は {other} として開いています」) で知らせ、同じなら、既存が今の文書だったときだけ `info.open.already_open` を出す。違えば、文書に canonical を覚える。手順 0 のパスの文字列の早い一致 (I/O なし) はそのまま残す。
2. **名前を付けて保存**: 保存先の照合 `JobKind::Metadata { purpose: MetaPurpose::SaveAsTarget }` は、`metadata` に加えて canonical のパス (保存先が無ければ、親ディレクトリを canonicalize してファイル名を付けたもの) を返す。§9.7 (a) の表の 1 (今の文書と同じファイル) と 1b (ほかの開いている文書と同じファイル。`msg.save_as.open_in_other` (`{other}` は相手の文書の表示用のパス) を出して `AskName` に戻る) を、0 の比べ方で判定する。保存に成功したら、文書の canonical を照合で得たものに替え (`FileId` は `disk_stamp` で直る)、その時点でもう一度ほかの文書と canonical のパスだけで比べる (保存した直後の識別は、ほかの文書の古い識別を確かめていないので使わない。照合から保存の完了までの間に、別の文書の `Identity` が届いて同じファイルになっていたら、保存した文書はそのまま残し、`msg.save_as.now_open_in_other` で知らせる (保存したことは `info.save.done` で示した後。拒んだときの `msg.save_as.open_in_other` と読み違えないよう、別の文言にする)。どちらの文書も閉じない (保存していない編集を捨てない))。
3. **横断検索**: 走査は `stat` をしないので候補は `FileId` を持たない。開いている文書との突き合わせ (§12.7.5 の 1) は、(i) 候補の絶対パス (ルート + 相対パス) と文書の canonical のパスを、Windows の verbatim の接頭辞を外した形にして要素ごとに比べ、(ii) 一致しない文書のうち、信頼できる識別を持ち、ファイル名が候補と同じ (Windows・macOS は ASCII の大文字小文字を畳んで比べる) ものがあるときだけ、その候補に `metadata` を呼んで 0 の比べ方で確かめる (M31 の段 7 の実装で (ii) が効くのは、ルートの下の別の場所にある同じ名前のハードリンクで開いた文書だけ。ジョブに渡すのは canonical のパスがルートの下にある文書だけで、260 文字を超えるパス・リンク・短い名前 (8.3 形式) で開いた文書は canonical のパス (長い名前・リンク先) で (i) に当たる。ルートの外の文書は渡さないので、ルートの外のパスで開いた同じファイル (リンク・ハードリンク) とは突き合わせず、ディスクを探す。`follow_symlinks = true` を開けて、リンクを通ったパスの候補を探すようにするときに、この突き合わせの設計を見直す (既知の制約。§12.7.5 の段 7 の細目の既知の制約))。canonical の無い文書 (無題・`Identity` が届く前) はディスクを探す。(ii) の識別の比べ方も 0 のとおりで、古い識別は 0 の「古い識別の確かめ」と同じく確かめる: ジョブに渡す `OpenDocRef` が文書のパスと識別を持ち (別の `known_ids` は渡さない)、識別が候補と等しい文書は文書のパスを `metadata` し直し、今は違えばその文書とは識別で比べない (M31 の段 7 で入れた。細目は §12.7.5 の段 7 の細目 (10))。

- **段 5 の実装で決めた細目** (M31 の段 5 の PR、2026-11-22): (1) 識別を信頼する FS の見分け方: Windows は `GetVolumeInformationByHandleW` の FS の名前が NTFS / ReFS で、FS の旗に `FILE_SUPPORTS_OPEN_BY_FILE_ID` があり (WinFsp・Dokan などで作る FS が NTFS と名乗るため)、ハンドルが `FileRemoteProtocolInfo` に答えない (リモートでない) こと、かつパスの判定 (`fs_kind`) が `Local` のとき。識別は `GetFileInformationByHandleEx(FileIdInfo)` の 128 bit の値 (取れない FS は従来の 64 bit の値) を `metadata` と直接上書きのハンドルの両方で使う (ReFS の識別は 64 bit に収まらない)。Linux は `statfs` の `f_type` で NFS・SMB/CIFS/SMB2・9p (WSL の DrvFs を含む)・FUSE・AFS (`AFS_SUPER_MAGIC`・kAFS の `AFS_FS_MAGIC`)・Coda・Ceph・NCP・OCFS2・GFS2 (どれもカーネルの公開されたヘッダ `linux/magic.h`・`linux/gfs2_ondisk.h` で値を確かめたもの) を `Network` にし、それ以外を `Local` にする。`Local` でも FAT (msdos・vfat)・exFAT の識別は信頼しない (Windows と同じく、項目の位置から作られ使い回される)。Linux 以外の Unix は `Unknown` (信頼しない。macOS へ移すときに `f_fstypename` で見分ける)。ディレクトリの `file_id_unique` は立てない。(2) 文書の識別は `disk_stamp.file_id` を正とし、読込の完了の前 (`disk_stamp` が無い間) だけ `Identity` で受けた識別を使う。`file_id_unique` は `Identity` と名前を付けて保存の照合の値を文書に覚える (新しく作ったファイルへの名前を付けて保存では照合で分からないので `false`。canonical のパスで比べる)。(3) 二重オープンで閉じた文書に起動時の `path:行:桁` があれば、既存の文書がまだ読込中のときだけ引き継ぐ。既存の文書を出すペインは、開いた時点のペイン (切り替え元が無ければフォーカスのペイン)。オーバーレイ・検索バーの間は表示を動かさない (§9.2.1 の「表示の切り替え」)。(4) 名前を付けて保存の照合 (`SaveAsTarget`) で保存先が無いときは、`Failed` ではなく `JobResult::TargetAbsent { err, canonical }` を返す (無い保存先の canonical を運ぶため。02 §9.3)。(5) 照合のジョブを待つ間の `Esc` は結果を捨てて名前の入力に戻り、`file.save` は保存中と同じく無視する (§9.7 の全状態の遷移表の `CheckTarget` の行)。(6) 表の 1 で未編集なら、単独の名前を付けて保存 (後続・新しい書式なし) に限って §9.3 手順 0 の照合 (書かない) にする。(7) 照合の結果は、ほかのオーバーレイ (入力・確認・パレット・一覧・ヘルプ) か検索バーが出ている間は保留し、閉じた後に (a) の表で扱う (§9.2.1 の「表示の切り替え」と同じ。名前の入力・上書きの確認で、利用者が開いたほかの入力を黙って置き換えない)。保留の間も照合中として扱う。(8) 照合の間 (保留を含む) の `app.quit`・`file.open`・`file.new` は `info.save.in_progress` を示して無視する (§9.7 の全状態の遷移表の `CheckTarget` の行)。`file.open` / `file.new` はコマンドの入口で無視し、起動時のパスなど内部の開く経路は止めない。(9) 照合の間の `Esc` は、照合中の文書が今の文書ならその照合を、今の文書のものが無ければ最後に投入した照合を取り消し、その文書の名前の入力に戻る (入力の対象は照合を始めた文書、§9.8 の「入力・確認の対象」)。取り消す順は「今の文書の読込 (読込中の標準入力の文書の確認 (§9.2.2) を含む) → 照合 → 表示していない文書の読込 (§9.8)」。照合は在る文書のものだけを数える (文書を閉じると、その照合のジョブを取り消し、保留した結果も捨てる)。取り消し済みで結果を待つだけの読込は数えない (2 回目の `Esc` が照合に届く)。(10) 名前を付けて保存の識別の覚えは、そのパスへの保存が終わったとき (成功・失敗とも) と文書を閉じたときに消す。失敗の後の再試行 (外部変更の確認の「上書き」など) が成功したときは、正規化したパスを canonical にし、識別は信頼しない側にする。

#### 12.7.7 ジョブとワーカーのまとめ

| ジョブ (02 §9.3) | ワーカー | 1 区切り | 取消し | 結果 |
|---|---|---|---|---|
| `WorkspaceRoot` | `io` (起動時のパスを開くジョブの後ろ) | 区切らない (目印の `metadata` だけ) | 見ない (短い) | `WorkspaceRoot` |
| `WorkspaceScan` | `bg` (検索と同じ段) | ディレクトリ 16 個 / 1 つのディレクトリの 4,096 項目 / 8 ms | 区切りごと | `ScanBatch { generation, chunk: Arc<[Candidate]>, unreadable_dirs }` を束ごと、終端 `ScanDone { generation, total, truncated }` |
| `FinderMatch` | `bg` (同) | 区切らない (照合 1 回。S-26 で 10 万件の p99 10 ms) | 4,096 件ごと | `FinderMatched` |
| `WorkspaceSearch` | `bg` (同) | ディレクトリ 16 個 / ファイル 64 個 / 中身 1 MiB / 8 ms | ファイルごと・1 MiB ごと (復号はチャンクごと、照合は行・チャンクごと) | `WorkspaceHits` を区切りごと (結果の無い区切りは 100 ms ごとの進捗だけ)、終端 `WorkspaceSearchDone` |
| `ReleaseCandidates` | `bg` (同) | 区切らない | 見ない | 送らない (終端の結果 `Released` だけ) |

- `bg` のキューの順 (02 §9.1、OQ-198) では、上の `bg` のジョブはすべて検索 (`Search` / `SearchFrom` / `SearchLines`) と同じ最も低い段で、同じ段の間は投入順。走査と横断検索は区切りごとに末尾へ積み直すので、入力のたびに投げる `FinderMatch` は前の区切り 1 回の後に走る (S-26 の (4) で、打鍵から結果が届くまで p99 23 ms)。
- 保存・読込・`metadata` は `io` にあり、走査と横断検索の影響を受けない (S-26 の決める点 5)。ファインダ・横断検索から開くジョブも `io`。
- 主スレッドで行うのは、ジョブの投入と結果の採否、表示の窓に入る行の組立て、塊の `Arc` の受け渡しだけ。1 回のメッセージの処理で 1 ms を超える作業を主スレッドに置かない (NFR-103)。

#### 12.7.8 試験と計測 (09)

- **L0 / L1**: gitignore の行の解釈と照合の表の試験 (git の試験から写さず、gitignore(5) の記述から自分で作る)、`core.ignorecase` の読み方、`WorkspaceWalk` の区切り・順序・上限・読めないディレクトリ・入れ子のリポジトリ・`.git` のファイル (worktree) を `MemFs` で。ファインダの採点の性質 (a)〜(c) と並びの規則。横断検索の 1 ファイルの扱い (開いている文書・大きさ・バイナリ・復号・最悪の入力の上限)。proptest: 任意の検索語と候補で、照合の結果が「語の部分列の AND」と一致し、点の並びが決定的であること。
- **git との差分試験** (09 §3.4。01 FR-125 AC-1): `xtask gen-corpus --workspace` の木 (09 §4) で、自作の走査の結果を `git ls-files -z --others --exclude-standard` (残るファイル)・`--ignored` (除外されるファイル)・`git check-ignore --stdin -z` (全パス) と比べ、差 0 を確かめる (git はシステム・利用者の設定とグローバルの excludesFile を読まない設定で走らせる: `GIT_CONFIG_NOSYSTEM=1`・`GIT_CONFIG_GLOBAL` を空のファイル・`-c core.excludesFile=<空のファイル>`。木に 260 文字を超えるパスがあるので Windows では `-c core.longpaths=true` も付ける。自作の側は `workspace.exclude = []` で走らせる (既定の `**/node_modules` などに当たる git の規則は無い))。`core.ignorecase` は `true` と `false` の両方で比べる。入れ子のリポジトリは、各リポジトリのルートで git を走らせた結果と比べる。毎 PR は小さい木 (約 2,000 ファイル)、夜間は 10 万ファイルの木。
- **契約試験** (09 §3.4 の FileSystem の契約試験、STR-D3): `read_dir` を `MemFs` と実 FS に同じ試験で掛ける (空のディレクトリ・種類 (ファイル・ディレクトリ・シンボリックリンク・その他)・`.` と `..` を返さない・隠しファイル・日本語と空白の名前・UTF-8 で表せない名前 (Unix) と対にならないサロゲート (Windows)・260 文字を超えるパス (Windows)・末尾のドット・空白と予約名 (`con.txt`) の名前 (Windows。列挙で名前が変わらないこと)・オフラインの属性 (Windows。`offline`)・`NotFound`・ファイルを渡したとき・`PermissionDenied`・`max_entries` での打ち切り)。`open_read` の通常のファイルの確かめ (FIFO・デバイスで止まらずに `InvalidPath`) も同じ契約試験に足す。
- **fuzz**: `fuzz_gitignore` (09 §3.3。v0.5 の M31 で足す。08 §2.6 の (d) の「足すか」の決定)。
- **L3**: ファインダで開く (同じファイルを別の書き方のパスで開いても 1 つの文書になる、SAF-28)・消えたファイル・キャンセル。横断検索で Shift_JIS / EUC-JP / UTF-16 のファイルが当たる・開いている文書の未保存の内容が当たる・ジャンプの丸め。起動の `DIR` と `--workspace`。
- **計測**: BM-125 / BM-103b-finder / BM-125s / BM-103b-search-ws / BM-405 (09 §5.2)。BM-125s は基準機で S-26 の (3) を測り直す場を兼ねる (OQ-103 の条件。遅ければ走査の専用のレーン 2 本に替えるかを判断する)。

#### 12.7.9 実装の順と各段の完了の条件 (08 §2.6 の M31)

| 段 | PR (種別) | 中身 | 完了の条件 |
|---|---|---|---|
| 1 | `feat/eve-core-workspace-root` | ルートの検出 (`WorkspaceRoot`)、CLI の `DIR` と `--workspace` / `-w` (07 §18)、起動の引数のディレクトリの知らせの保留 | L3: `DIR`・`-w`・目印・cwd の 4 つの規則と、ディレクトリの引数が 2 つのとき。最初の Frame を遅らせない (BM-101a / 102a が劣化しない) |
| 2 | `feat/eve-core-read-dir` | `FileSystem::read_dir` (02 §4.5) の eve-platform (Windows / Unix) と `MemFs` の実装、契約試験。既存の `list_dir_names` (v0.4 の M26) は残す (`read_dir` の上に作り直すのは別の `refactor` の PR でよい) | 契約試験が Windows と Linux (WSL) で通る |
| 3 | `feat/eve-core-gitignore` | 除外の照合 (§12.7.3)、git との差分試験、`fuzz_gitignore` | 差分試験の差 0 (小さい木を毎 PR、10 万ファイルの木を手元と夜間で)。fuzz を夜間に足す |
| 4 | `feat/eve-core-workspace-scan` | `WorkspaceWalk` と `WorkspaceScan`・`ReleaseCandidates`、候補の塊。区切って譲る経路 (`JobStep::Continue`、02 §9.3) を eve-host のレーンと `ManualJobSpawner` に入れる | `MemFs` の試験 (§12.7.8)。BM-125s を記録し、走査の間の `io` / `bg` の開始の遅れが S-26 と同じ桁 |
| 5 | `fix/eve-core-same-file` (SAF-28) | §12.7.6 の 0〜2 (`Identity` と確認の前への移動、`FileMeta.file_id_unique`、`Document.canonical`、名前を付けて保存の `CheckTarget` の照合のジョブ (v0.4 までの実装は投げずに `Expect::Absent` で保存する。03 §9.7 の状態の表のとおりに入れる)、`msg.save_as.open_in_other`) | L3: 大文字小文字・シンボリックリンク・ハードリンクで同じファイルを開くと 1 つの文書になる。名前を付けて保存の表 (§9.7 (a)) の 1 / 1b。M30 のペインの作り替え (FND) と同時に進めない (08 §2.6) |
| 6 | `feat/eve-core-finder` | FR-125 (§12.7.4)、`workspace.open_file` と `Ctrl+O` の移し替え (07 §11.6 の `file.open` の行を `—` に、`workspace.open_file` の行に `mod+o`)、`finder` コンテキスト、文言 | 01 FR-125 AC-1〜AC-3 の試験。BM-125 と BM-103b-finder を基準機で測り、目標を満たす |
| 7 | `feat/eve-core-workspace-search` | FR-405 (§12.7.5)、`search.workspace` のキー、トグルの割当、文言 | 01 FR-405 AC-1〜AC-3 の試験。BM-103b-search-ws を基準機で満たし、BM-405 を記録する |

- 段 1〜4 は互いに独立で、並べて進められる (1 は「開く」の経路と起動、2 は Port、3 は純粋関数、4 は 2・3 の後)。5 は 6 の前。6 は 1・4・5 の後、7 は 6 の後 (オーバーレイと照合のジョブの形を 6 で作る)。
- 段 6・7 の PR は、追加だけの `eve-view` の変更を含まない (`OverlayModel::List` をそのまま使う)。含むことになったら、その PR の前に仕様を直す。

#### 12.7.10 決めた点 (M31 の仕様の PR、2026-11-21。退けた案と理由)

**PO の確認 (確定 (ユーザー回答、2026-11-21)。どれも推奨案のまま)**: (1) ルートの決め方と CLI (`-w` → 最初の `DIR` → 目印 → カレントディレクトリ。2 つ目以降の `DIR` は知らせて無視、`-w` はファインダを開かない) (2) 横断検索の既定キー (`Ctrl+Shift+F` [E] と `Ctrl+K F`) と、ファインダと横断検索の一覧で `finder` コンテキストを 1 つ使うこと (3) 上限と区切りを定数にし設定のキーを足さないこと・`workspace.exclude` の書式・境界の型 (eve-view) を変えないこと (4) 下の表の「`bg` で OS の呼出しが止まったとき」と「除外の源の所有者の検査」を先送りすること。

| 点 | 採った案 | 退けた案と理由 |
|---|---|---|
| 大文字小文字 (S-26 の 1) | リポジトリの `core.ignorecase` を読む。無ければ `false`、読めなければ OS の既定 | 常に区別 (ripgrep の既定): Windows の利用者が `*.log` で除外したつもりの `FOO.LOG` が出る (git の結果とも違う)。常に OS の既定: Linux の大文字小文字を区別しないディレクトリ・設定を変えたリポジトリで git と違う |
| POSIX の文字の集合 (S-26 の 2) | git の wildmatch に揃える | globset の解釈: git と 4 件違った (S-26) |
| 区切り (S-26 の 3) | ディレクトリ 16 個。大きなディレクトリの中は 4,096 項目ごと。`read_dir` に `max_entries` を渡す | `read_dir` を反復子にして OS の列挙も区切る: Port の形が複雑になり、`MemFs` との契約試験も増える。OS の列挙 1 回は 15,000 項目で 31 ms まで (S-26)。上限で打ち切るだけ: 打ち切りまでの照合が区切れない |
| 候補の持ち方 (S-26 の 4) | 不変の塊 `Arc<[Candidate]>` の列。古い一覧は `bg` で解放する | 主スレッドの `Vec<Candidate>`: 解放で 13〜15 ms 止まる (S-26) |
| 保存の置き場所 (S-26 の 5) | ADR-0006 どおり `io` | `bg`: 走査の区切りと照合の後ろに最大 47 ms 待つ |
| 項目の数の上限 (S-26 の 6) | 20 万件の定数。設定のキーは足さない | 設定のキー: 公開の名前は戻しにくく、必要が分かった版で足せる |
| ファインダの一覧の寿命 | 開くたびに走査し直し、終わってから差し替える (前の一覧を見せたまま)。閉じて 5 分で手放す | 起動時に走査する: ファインダを使わない起動でも I/O と数十 MB を使う。開くたびに空から: 毎回 0.4〜0.8 s は一覧が欠ける。監視で直す: 監視は M33 で、v0.5 は監視しない FS もある |
| 横断検索の走査 | 検索ごとに走査する (ファインダの一覧を使わない) | ファインダの一覧を使う: 一覧の寿命と横断検索の結果の新しさが絡み、状態が増える。走査の費用は中身を読む費用より小さい |
| 横断検索の結果の単位 | 行ごとに 1 件 (同じ行の一致はまとめて強調) | 一致ごとに 1 件: 同じ行が並び、10,000 件の上限が早く埋まる |
| 横断検索を始める時機 | 入力ごとに取り消して始める (待ち時間なし) | 入力が止まってから (例 200 ms): エディタ内の検索と振る舞いが違い、BM-103b-search-ws の形とも合わない |
| `workspace.exclude` の書式 | ルートの `.gitignore` と同じ書式。gitignore より優先し、`!` で戻せる | 別の glob の方言: 2 つの書式を覚えることになり、照合の実装も 2 つになる |
| シンボリックリンク | 既定では降りない。ファインダには出し、横断検索では読まない | どちらにも出さない (ripgrep): git はリンクを一覧に出すので、差分試験と合わない。どちらでも読む: リンク先のワークスペースの外を読む (07 §12.3) |
| `.ignore` / `.rgignore` | 読まない | 読む (ripgrep の既定): git の結果と比べられない |
| 消えたファイルを選んだとき | `must_exist` で開き、新規の文書にしない | ふつうの `file.open`: 空の新規の文書ができ、保存するとファイルを作り直す |
| 保存先がほかの開いている文書と同じ | 拒んで名前の入力に戻る (`msg.save_as.open_in_other`) | 上書きの確認を出す: 2 つの文書が 1 つのファイルを指し、二重オープンの不変条件と外部変更の検知が崩れる |
| 同じファイルの比べ方 (SAF-28) | 信頼できる `FileId` の一致 **または** canonical のパスの一致。`FileId` は NTFS / ReFS とローカルの Unix の FS だけで信頼し、`disk_stamp` から取る | `FileId` だけ: 保存で `FileId` が変わると同じファイルを別と判定する。FAT・exFAT・ネットワークの `FileId` も信じる: 別のファイルを同じと判定し、名前を付けて保存で何も書かない |
| 除外の源の所有者の検査 | M31 では掛けない (`gitdir` は UNC・デバイスを辿らない、種類が `File` のものだけを非ブロックで読む)。M33 で決める (**M33 の仕様の PR で、v0.5 も掛けないと決めた**。07 §12.1 の OWN-8) | M31 から掛ける: 所有者の検査の実装 (S-25、M33) を前倒しすることになり、読んで変わるのは一覧の範囲と `core.ignorecase` だけ |
| 区切って譲る仕組み | ジョブが続き (`JobStep::Continue`) を返し、レーンが同じ `JobId`・`CancelToken` で末尾に積む | 続きを結果として主スレッドに返し、core が投げ直す: 区切りごとに主スレッドを通り、取消しと世代の扱いが増える |
| 一覧の行の切り詰め | 既存の `CutPolicy` (ファインダは `Middle`、横断検索は `End`) で、境界の型を変えない | `CutPolicy::Start` と行の 2 つ目の欄を足す: M30 が境界の型を変える時期に重ねず、v0.5 の判定 (`CONTRACT_VERSION` は 0) の範囲を広げない |
| オンライン専用のファイル (OneDrive など) | 横断検索は読まずに飛ばす。ファインダには出す | 読む: 全ファイルを呼び戻し、ネットワークを待ってディスクを埋める |
| `bg` で OS の呼出しが止まったとき | v0.5 は既知の制約。`WriteRecovery` とスワップを `bg` へ移す前に、ワークスペースのジョブを別のレーンへ分けるかを決める (M33 の仕様の PR で、M33 のジョブ (監視の照合・設定の読込) は `io` に置いて `bg` の詰まりの影響を受けないと決めた。§9.5.1 の WATCH-10、07 §12.6.3) | 監視と切り離しを `bg` にも掛ける: OQ-179 (`io` の監視) も未実装で、M31 の範囲を超える |

### 12.8 検索履歴 (FR-407、v0.5 の M34)

コマンドは `ui.history_prev` / `ui.history_next` (入力欄に 1 つ古い / 新しい履歴を入れる。07 §11.6 の M34 の表)。受入基準は 01 §5.5 の FR-407。M34 の仕様の PR (2026-11-24) で決めた (試験・実装の段・決めた点は §15.9)。

| ID | 規則 |
|---|---|
| SHI-1 | **履歴の種類**: **検索語の履歴** (検索バーの検索欄と、横断検索 (FR-405、§12.7.5) の検索語で共有する) と、**置換の文字列の履歴** (検索バーの置換欄) の 2 つ。どちらもプロセスに 1 つずつ (文書・ペインに依らない)。ほかの入力欄 (行へ移動・名前を付けて保存・パレット・ファインダ・引数の入力) は v0.5 では履歴を持たない |
| SHI-2 | **記録する時期**: 検索語は、検索バーで `search.next` / `search.prev` (`Enter` を含む) を実行したとき・`search.replace` / `search.replace_all` を実行したとき・横断検索を実行したとき (検索語で走査を始めたとき) に、その検索語を記録する。置換の文字列は `search.replace` / `search.replace_all` を実行したときに記録する。空の文字列と、4 KiB (`SEARCH_HISTORY_ENTRY_MAX_BYTES`、定数) を超える文字列は記録しない。同じ文字列 (大小を区別して比べる) が既にあれば、それを消して最新に置く。トグル (大小・正規表現・単語・選択範囲内) は記録しない (文字列だけ) |
| SHI-3 | **上限**: どちらも 100 件 (02 §14.1 の MR8 の「検索履歴 100 件」。定数)。超えたら古いものから捨てる |
| SHI-4 | **呼び出し**: `ui.history_prev` は、欄の文字列を 1 つ古い項目に置き換え (欄の全体を置き換え、カーソルを末尾へ、選択を解く)、`ui.history_next` は新しい側へ戻す。検索欄と横断検索の検索語の欄では検索語の履歴を、置換欄では置換の履歴を使う。最初の `ui.history_prev` で欄の今の文字列を **下書き** として覚え、最新の項目より新しい側へ戻ると下書きに戻す。欄を開き直すと位置は最新に戻る (下書きも捨てる)。置き換えた文字列は、打ったときと同じく逐次検索 (§12.2) と横断検索の更新に効く。端なら何もしない (知らせない)。入力欄の Undo (欄の中の編集) には残さない |
| SHI-5 | **保存しない**: 履歴はメモリだけに持ち、ディスク (状態ディレクトリ・セッション・緊急退避・スワップ・ログ・クラッシュログ) に書かない。終了で消える (検索語に秘密を含み得る。NFR-504)。保存 (07 §16.1 の `history.toml` と `history.enabled`) は、セッションの復元 (FR-121) と一緒に入れる版で決める |
| SHI-6 | **効く欄**: `when` は `(overlay.active == "" && search.active) \|\| overlay.active == "workspace_search"` (検索バーの検索欄・置換欄と、横断検索のオーバーレイの入力欄にフォーカスがあるとき。07 §11.6)。ほかの入力欄では偽で、キーは下の層へ進む (割当が無ければ捨てられる) |

---

## 13. Unicode 処理

### 13.1 書記素クラスタ (FR-201 / 301)

- 単位は **拡張書記素クラスタ** (UAX #29、`unicode-segmentation`)。カーソル移動・Backspace/Delete・選択の最小単位。
- **チャンク跨ぎ**: 境界判定の入力は `TextSlice` から前後最大 256 バイトを連結した小バッファとし、その中央付近の境界のみを信用する。`unicode-segmentation` の `GraphemeCursor` (チャンク境界で `PreContext` を要求できる API) を使えば、より正確に扱える。実装はどちらかで、挙動は同じ。
- **Backspace の特則**: 既定は書記素単位 (FR-201)。ただし一部の利用者はハングルや結合文字で「1 コードポイントずつ」の削除を期待する。v0.2 で設定 `editor.backspace_unit = "grapheme" | "codepoint"` (07 §5.13) として公開し、v0.1 は `grapheme` 固定 (FR-201、OQ-45)。
- **絵文字**: ZWJ シーケンス・肌色修飾・国旗 (地域指示記号ペア)・キーキャップはすべて 1 書記素。幅は §13.2。
- **異体字セレクタ (VS15/VS16)**: 基底文字と結合して 1 書記素。VS16 は幅を 2 にする (§13.2)。
- **制御文字**: 単独で 1 書記素。表示は可視記号 (§14.2)。
- **不正バイトのエスケープ文字** (`U+10FF00..U+10FFFF`、§8.3。v0.4 の M23): 前後を常に書記素の境界とする (UAX #29 の GB9 では直後の結合文字・異体字セレクタ・ZWJ と 1 書記素になるが、エスケープ文字は 1 バイトを表すだけで基底の文字ではない。理由と範囲は §8.3.1 の「書記素」)。表示は v0.3 から 1 文字ずつ `<0xNN>` にしている (§14.2)。境界の集合は「UAX #29 の境界 ∪ エスケープ文字の前後」で、eve-text の境界の入口 (文書の位置の `is_grapheme_boundary` / `next_grapheme_boundary` / `prev_grapheme_boundary` と、文字列を分ける `grapheme_indices` / `first_grapheme`。表示の単位・幅・単語の境界もこれを通る) のすべてで同じ規則を使う。境界が増えるだけなので、エスケープ文字の無い本文の境界と、結合しない文字の近道 (§11.5) は変わらない。

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
| 不正バイトのエスケープ (§8.3。`pua_escape` が `Enabled` か `DisabledStateful` の文書の `U+10FF00..FF`) | `<0x9A>` = 6 | **バイトは `<0xHH>`、コードポイントは `<U+XXXX>`** と書き分ける (不正バイト 0x85 は `<0x85>`、文字 U+0085 は `<U+0085>`)。`DisabledCollision` の文書 (§8.3) が真に含む `U+10FF00..FF` は通常の私用文字として扱い、エスケープとして表示しない |
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
- 半角の空白 (U+0020)・行末の空白・改行・他の空白類 (U+00A0 等の Zs) は v0.1 では可視化しない (v0.5、FR-508)。全角スペースが不可視文字 (Cf 等) と同じ書記素に入ることは無い (U+3000 は単独で 1 書記素)。
- v0.1 は `view.show_whitespace` を定数 `"minimal"` (全角スペースとタブ) として実装し、切り替えない (07 §5.3。キーの公開は v0.2 で、`"off"` を選べる)。**v0.2 の M9c で公開した**: `off` ではタブを空白のセル (幅は次のタブストップまで)、全角スペースを ASCII の空白 2 セルで描き、記号のスタイル (`ui.whitespace`) を付けない。幅・カーソルの列・折り返しの位置は `minimal` と同じ。設定の差し替え (`set_config`) で開いている文書にも写す。

- **Unicode バージョン**: `unicode-width` / `unicode-segmentation` のバージョンで決まる。端末との不一致 (新しい絵文字を端末が 1 セルで描く等) による描画崩れは **04 の再描画戦略** で扱い、core は一貫した幅を使う ([02 §13.3](02-architecture.md#133-unicode))。

### 13.3 単語境界 (日本語補助規則)

- **不正バイトのエスケープ文字** (`U+10FF00..U+10FFFF`、v0.4 の M23、§8.3.1): 連続するエスケープ文字の並びを 1 つの単語とする (文字種「不正バイト」(`ScriptClass::InvalidByte`)。下の表のどの文字種ともつながらない。書記素の境界は §13.1)。並びの直後の長音・踊り字・濁点は、記号の直後と同じくランの先頭として扱う (並びに続けない)。書記素の境界と同じく、文書の `pua_escape` は見ない (衝突の文書の真の私用文字の並びも 1 語)。

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
- 上の表に当たらないものの落ち先: **Cf** (書式文字。ZWSP・bidi 制御・単独の ZWJ / ZWNJ 等)、**Cc** (TAB 以外の制御文字。TAB は White_Space なので `Space`)、**Co** (私用領域。不正バイトのエスケープ `U+10FF00..FF` は除く (上の先頭の項))、**Cn** (未割当) は `Punct` (1 書記素で 1 単位。可視化される文字 (§13.2) の前後で単語移動が止まる)。**ハングル** (Script=Hangul)・**タイ文字**・キリル・ギリシャ・アラビア等の CJK 以外の文字体系の文字 (General Category L* / N* / 書記素の先頭の Mn / Mc) は `Alnum` (空白で区切られる言語は空白で止まる。タイ語のように空白で区切らない言語は句読点か空白まで 1 語になる。辞書による分割は icu4x のオプション (下記) の範囲)。
- 例外の踊り字・長音 (`々` `ー` 等) が「直前の文字種に従う」の「直前」は、直前の書記素の分類を指す。

**境界決定の規則 (全文字種に共通の「文字種の遷移」規則)**

1. 文字列を `ScriptClass` のランに分ける。**`ScriptClass` が変わる位置をすべて境界とし、ランの内側は境界にしない**。日本語も英語も同じ規則で決まる (VS Code の単語区切りや Vim の `w` に近く、予測しやすい)。
2. `Han → Hiragana` の遷移も **境界にする** (01 FR-302 / OQ-117。「食べる」は `食|べる`、「勉強する」は `勉強|する`)。送り仮名・助詞を漢字に続けて 1 語にする動作は v0.5 の設定 `editor.word_attach_hiragana` (既定 `false`、07 §5.13) で選べるようにする。
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
| 2 | 食べる | `食\|べる` | → 1, 3 / ← 1, 0 | Han→Hiragana は境界。1 語にするのは `word_attach_hiragana` (v0.5) |
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

- 単語移動 (§5.3)、ダブルクリック選択 (FR-311)、`whole_word` 検索 (§12.4、v0.2)、テキストオブジェクト (§5.7、v0.5) で共用する。
- 実装は Unicode の Script / General Category 表引きだけで、`unicode-segmentation` の単語境界は使わない。表の全件一致は L0 のゴールデンテスト (09 §3.2 / §4) で検査する (P0 スパイクは不要)。
- 将来 `icu4x` の辞書分割 (「お茶」を 1 語にするなど) をオプション提供できるよう、`WordSegmenter` trait として抽象化する ([02 §13.3](02-architecture.md#133-unicode))。拡張点はこの trait だけで、v0.1 は遷移規則の 1 実装を持つ。

### 13.4 大文字・小文字変換 (FR-214)

- Rust 標準の `to_uppercase` / `to_lowercase` (完全 Unicode 対応、`ß → SS` のように長さが変わり得る)。トルコ語 `i` などロケール依存変換は行わない。コマンド・対象・選択の扱いは §15.8 (v0.5 の M34)。

### 13.5 正規化と BiDi

| 項目 | 方針 |
|---|---|
| Unicode 正規化 (NFC/NFD/NFKC) | **行わない**。読み込んだコードポイント列をそのまま保持する (v0.1、NFR-303)。正規化コマンドを v0.5 で提供可 (Transaction として明示的に) |
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
  for grapheme g in line (長大行は §14.4 の列の索引で from_dcol に最も近い記録点から):
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
| 長大行 | (案。v0.4 に入れるかは §14.4 の「長大行の折り返し」の手順で決める) §14.4 の記録点が折り返し位置も記録する。折り返し ON では、記録点は **表示行の先頭にだけ** 置く (4 KiB を超えた後の最初の表示行の先頭。そこから走査を再開すれば、単語境界・禁則の判定が前の表示行に依存しない) |
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
| リサイズ・設定変更 | — | `top` (論理行) は保ち、その View を表示するペインの本文の桁数 (折り返し幅) が変わったときだけ `wrap_idx` を 0 に戻す (§14.3 の「ScrollPos.top を保持」と同じ)。行数だけが変わるとき (端末の高さ・検索バーとタブラインの出し入れ、M32 の段 1) は `wrap_idx` を保つ。ただし、行数の変化で縮退表示 (§9.9 の PANE-4 (b)) に出入りすると本文の桁数が変わるので戻す。守りとして、スクロールの計算は `wrap_idx` を先頭の論理行の表示行の数 − 1 に丸めて読む |
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
- **性能 (NFR-103、M9b の 4 本目の実測。リリースビルド、ヘッドレスで入力 → Frame 生成)**: 1 MB の日本語混在文書 (120×40、中央、`view.wrap = word`) で p50 0.6 ms / p99 1.5 ms (折り返し OFF は p50 0.3 ms / p99 0.9 ms)。目標 (p50 2 ms / p99 8 ms) の内側。測るには `cargo bench -p eve-testkit --bench bm_103a -- --wrap`。**長大行**は 1 回の操作で `wrap_line` を行の表示範囲 (64 KiB) の分だけ数回走らせるので、64 KiB の 1 行 (ASCII) で上下移動 + Frame が約 8 ms (日本語は約 0.8 ms、20 KB の行は約 0.3 ms)。この最悪ケースは v0.2 で許容した。v0.4 の §14.4 の列の索引は閾値 (1 MiB) を超える行だけが対象なので、閾値以下のこの行には効かない (v0.3 までは「§14.4 の列チェックポイント (v0.4) で表示行の先頭を引けるようにして解消する」としていた)。表示行の先頭を記録点で引く形と折り返しの結果の再利用は、長大行の折り返し (§14.4。M25 の計測で v0.5 以降に送った) を入れるときに合わせて検討する。
- 編集で `top` の論理行の行頭が動かなければ `wrap_idx` を新しい表示行数に収め、動いたら 0 に戻す。リサイズ・縮退表示の出入り・ペインの大きさの変更で、その View の本文の桁数が変わったときも 0 に戻す (行数だけの変化では保つ)。

**実装の分け方 (README §3.5 の 3・4 を細かくする)**:

1. この節 (突合の記録)。
2. `wrap_line` と、Frame の行・カーソル・選択・検索の強調への反映 (`View.wrap` は内部の状態として持ち、設定とコマンドはまだ公開しない。試験は内部 API で切り替える)。
3. 表示行単位の上下移動・`Home` / `End` と affinity・ページ移動・スクロール・`max_top`。
4. TUI の描画 (`↪`)、設定キー (`view.wrap` / `view.wrap_indent` / `view.wrap_kinsoku`) と `view.toggle_wrap`、長大行・巨大ファイルでの NFR-103 の確認。

### 14.4 長大行の列の索引

**版: v0.4 の M25 (書記素の単位。01 NFR-106 AC-4・AC-5、計測は 09 の BM-106)**。v0.2 の char 近似モード (§11.5.1) の列の索引 `ColIndex` (`layout::long`。64 KiB ごとに `(バイト位置, 表示列, char 数)`) を **書記素の単位** に広げ、作り方を **遅延** (要る位置まで。「開くときの索引の遅延」) と **差分** (編集では変更の位置より前を残す。PRF-16) にする。v0.3 までのこの節の案 (`LineLayoutCache`: 4 KiB ごとに折り返しの位置まで記録し、Document × WidthConfig ごとに LRU 1,000 行で共有し、行末までの計算をジョブにして完了まで近似値を出す) は採らない (下の「採らなかった案」)。v0.1 の長大行対策は §11.5 の読み取り専用での縮退、v0.2 は char 近似モードである。

```rust
struct ColIndex {                     // layout::long。長大行 1 行分。位置はすべて行頭からのバイト
    len: usize,                       // 行の内容のバイト長 (行の終端を含まない。編集の引き継ぎで新しい長さにする)
    points: Vec<ColPoint>,            // 記録点 (先頭は行頭)。書記素の境界にだけ置き、byte の昇順
    built_to: ColPoint,               // 行頭からここまで走査した (遅延)
    end: Option<ColPoint>,            // 行末 (行末まで走査したときだけ Some)
    rule: CharWidth,                  // 幅の決め方 (文書の表示の設定から作る。v0.2 と同じ)
}
struct ColPoint { byte: usize, col: usize /*表示列*/, graphemes: usize /*行頭からの書記素の数*/ }
```

| 項目 | 規則 |
|---|---|
| 単位 | 書記素 (UAX #29 の拡張書記素クラスタ)。境界の規則と幅は通常の行の `units` と同じ (書記素ごとの `unit_slow`。結合文字の列・ZWJ の列・国旗は 1 つの書記素で、幅も通常の行と同じ)。v0.2 の char ごとの幅と、結合文字を `◌` 付きで分けて出す表示はやめる (長大行の表示が通常の行と同じになる) |
| 記録点の間隔 | 4 KiB (定数 `LONG_STEP`) ごとに、そこから先の最初の書記素の境界に置く。位置からの変換 (バイト ↔ 表示列 ↔ 書記素の数) は手前の記録点からの走査で、走査は 4 KiB と 1 書記素まで。10 MB の行で約 2,600 点 (約 60 KiB)。間隔は実装の PR の計測で 4〜64 KiB の間で決め直してよい (決めた値をこの行に書く)。**M25 の (5) で 4 KiB に決めた** (BM-106 の 5 つの操作が 3 種のコーパスで p99 ≤ 50 ms。`docs/perf/0.4.0-m25/summary.md`) |
| 走査 | 行頭から順に書記素に分ける (後方の文脈の窓を使わないので、国旗の列の長さに依らず正しい)。速くするため、(a) 印字できる ASCII の長い連続 (16 バイト以上) は 1 バイト = 1 書記素 = 幅 1 としてまとめて数え、(b) **結合しうる文字の表** に無い char が続く間は char ごとに境界とし (各 char を 1 回だけ復号して表を引く)、幅は v0.2 の `CharWidth` の表引き (覚えを含む) で求める。**(a)・(b) のどちらも、次の char が表にあるときは、最後の char から書記素の分割に渡す** (例: 「か」+ U+3099 (結合する濁点)、ハングル音節 + 字母の V / T、デーヴァナーガリーの `क` + `्` (InCB の Linker) + `ष`)。表にある char の箇所だけ書記素の分割を行い、表に無い char まで進んだら (a)・(b) に戻る。**書記素の分割** は eve-text の `first_grapheme_len` (表引きの経路): `first_grapheme` (`unicode-segmentation` の UAX #29 にエスケープ文字の前後の境界を足したもの、§13.1) と同じ境界を、char の分類の表 (Grapheme_Cluster_Break と、Extended_Pictographic・InCB の Consonant / Linker / Extend の印) を引いて前から求める (**M25 の (5) のレビューで足した**: `unicode-segmentation` の 1 回の呼び出しが 1 書記素あたり 30〜100 ns かかり、国旗・絵文字の ZWJ の列・異体字セレクタ・インド系の文字が続く 10 MB の行で `End` が 100 ms を超えたため。分類の表も `unicode-segmentation` の振る舞いから全コードポイントについて作り、作り直した表との一致と、乱数の文字列・全コードポイント × 文脈で境界の一致を試験で確かめる)。分割に渡すのは書記素の上限と 1 文字まで (境界は「左の文脈と右の 1 文字」で決まるので答えは変わらない。写した範囲の残りの全体を渡さない)。複数の char の書記素の幅は、表示を組み立てずに `grapheme_width` で求め (0 のときとエスケープ文字を含みうるときだけ `unit_slow`)、32 バイトまでの書記素は幅を覚え (256 項目の直接写像)、それより長い書記素は最後の 1 つだけ覚える。**表** (eve-text の `may_join_grapheme`。境界の規則に関わる char): Grapheme_Cluster_Break が Extend (結合文字・異体字セレクタ・肌色修飾)・ZWJ・Prepend・SpacingMark・Regional_Indicator (地域指示記号)・L / V / T (ハングルの字母) のどれかの char。表に無い 2 つの char の間は、行の内容の中では必ず境界になる: 境界を消す規則は、GB3 (CR × LF) を除いて、どれも境界の左右のどちらかに表の char を要る (GB6 は左の L、GB7・GB8 は右の V・T、GB9・GB9a は右の Extend・ZWJ・SpacingMark、GB9b は左の Prepend、GB9c は左の Linker・Extend、GB11 は左の ZWJ、GB12・GB13 は左右の Regional_Indicator)。GB3 は、CR も LF も表に無いが、CR・LF が行の終端で行の内容に現れないことに頼る。Indic_Conjunct_Break (InCB) の Linker・Extend は Extend・ZWJ に含まれる。InCB の Consonant・Extended_Pictographic・ハングルの音節 (Hangul_Syllable_Type が LV / LVT) は、前の char と結合するのが左に Linker・Extend (GB9c)・ZWJ (GB11)・Prepend (GB9b)・字母の L (GB6) があるときか、後ろの char と結合するのが右に Extend・ZWJ・SpacingMark・字母の V / T (GB7・GB8) があるときだけで、その相手の char が表にあるので表に入れない (**M25 の (5) で改めた**: 当初の案は前の 2 つも表に入れていたが、絵文字混在のコーパスでは絵文字とその手前の char が毎回書記素の分割に回り、BM-106 の `end` が p99 141 ms (`--quick` の 20 回の目安) で v0.2 の 100 ms を割った。ハングルの音節はレビューで外した (韓国語の文の `End` が 135 ms だった))。Control は常に境界なので表に入れない (幅は表引き)。表は書記素の分割に使う `unicode-segmentation` の振る舞いから全コードポイントについて作り (短い文字列に置いて、その char が表に無い隣と結合する場所があるかを調べる)、同じ方法で作り直した表との一致と、表の版と `unicode_segmentation::UNICODE_VERSION` の一致を試験で確かめる (2026-11 の `Cargo.lock` の 1.13.3 は Unicode 17.0 の表を持ち、`src/grapheme.rs` で GB9c (InCB の Consonant + Linker + Consonant) を実装している。版を上げて規則が増えたら、試験が落ちるので、`join.rs` と `cluster.rs` の説明の手順で表を作り直す)。**1 書記素の上限**: 4 KiB (定数 `MAX_CLUSTER`) を超える結合列は、そこ (char の境界) で切って次の書記素にする (病的な入力で、1 回の位置からの変換が行の長さに比例しないため。§11.5 の後方 256 バイトの窓と同じく、正確さを諦める既知の制約で、その場合だけ通常の行の単位と食い違う。例: `a` × 70,000 + `e` + U+0301 × 3,000 + `z` の行の `z` の前は、長大行では桁 70,003、通常の行では 70,002)。**既知の制約 (速さ)**: 書記素の分割に回る書記素が密に続く行は、表に無い char だけの行より遅い (10 MB の行の開いた直後の `End` は、国旗・異体字の IVS・異体字セレクタ付きの絵文字の繰り返しで 40 ms 前後、BM-106 の参考のケースの p99 で最大 75 ms。病的な入力では、Prepend (U+0600 など) と ASCII の 1 文字の組の繰り返しが 80 ms 前後 (3 バイトごとに 2 char の書記素になり、どれも表引きの分割と幅の覚えを通る)。どれも `docs/perf/0.4.0-m25/summary.md`)。速い状態の値で、この機械の遅い状態では約 2.3 倍になりうる。固まりはしない |
| 遅延 (開くときの索引の遅延) | 索引は要る位置までしか作らない (`ensure_byte(b)` / `ensure_col(c)` が `built_to` から先へ走査して記録点を足す)。開いた直後の最初の Frame は、表示する列の範囲 (左端から画面の幅) までしか走査しない。行末の表示列・書記素の数が要るとき (`End`・行末への横スクロール・行末にあるカーソルの桁) に行末まで作る。閾値を超える行が画面に出なければ作らない |
| 編集 (差分。PRF-16) | ChangeSet (編集・Undo / Redo・置換・改行コードの変換) を適用したら、覚えている長大行の索引ごとに次のとおりにする。(1) 変更がすべてその行の外 (行の直前の改行より前、または行の終端より後ろ) にあれば、索引を **すべて残す** (行頭の位置を `map_pos` で写す)。(2) 行頭ちょうどへの挿入、または行の直前の改行 (CR・LF・CRLF のどれか) への変更 (削除・置き換え・その前後への挿入で CR と LF が合体・分離するもの) は **「行頭が変わる」** として索引を捨てる。直前の改行の手前 (直前の行の内容の末尾) への挿入は、挿入の末尾が CR でなければ (直前の LF と合体しないので) (1) とする。行頭から始まる削除・置き換え (s = 0) も捨てる (残るのは行頭の記録点だけなので、作り直しと同じ)。(3) それ以外で、変更が行の中にあるとき、行頭からのバイト数で数えた最初の変更の位置を s とする (s は行頭からのバイト数で、変更の前後で同じ)。**byte < s の記録点だけを残し** (行頭の記録点は常に残る)、`built_to` を s より前の最後の記録点に戻し (既に s より前なら動かさない)、`end` を捨てる (残りは要るときに作り直す)。書記素の境界の規則は「左の文脈と右の 1 文字」で決まるので、s より前の境界と、そこまでの表示列・書記素の数は変わらない (s の直前の書記素は右の文字が変わり得るので、s より前の最後の記録点から作り直す)。(4) 行が閾値以下になる、行が分かれる・つながる (行の終端への変更) ときは捨てる (挿入した文字列の改行で行が分かれる場合も含めて、引き継ぐ前に適用後の本文で、行頭が行頭のままで行の長さが見込みどおりかを確かめ、違えば捨てる)。索引は行の番号ではなく行頭の位置で引き継ぐ (前の行の増減で番号が変わっても使える)。後ろの記録点をずらして使い回すことは v0.4 では行わない (下の「採らなかった案」の (3))。proptest (下の「正しさの確かめ」の (b)) の編集の生成には、行頭ちょうどへの挿入と、直前の改行の変更 (CR の後ろへの LF の挿入・CRLF の片方の削除を含む) を必ず含める |
| 覚え | 文書ごとに長大行 4 行分 (今の `LONG_CACHE_LINES`)。鍵は (行頭の位置, 表示の設定 (タブ幅・曖昧幅・エスケープ文字の表示))。Revision が進んだら上の「編集」で引き継ぐか捨てる。表示の設定が変わったら捨てる (§3 の C3)。先行表示から全文の文書に替えたときも捨てる (§9.2.1) |
| ジョブにしない | 索引はメインスレッドで同期に作る (ジョブ・キャンセル・完了までの近似の表示を持たない。02 §14、OQ-31 の決定のまま。BIG-07 の食い違いはこちらに揃えた)。最も重いのは、開いた直後・編集の直後の `End` (行頭、または変更の位置から行末までの 1 回の走査) で、その時間を BM-106 の `end` と参考の `end_after_insert` (09 §5.2) で見る |
| 使う側 | §11.5.1 の表の利用者は、v0.4 では書記素の単位の索引を使う。v0.2 との違い: (a) Frame の行は、索引で左端の列の書記素の境界を求め、そこから画面の幅の分の書記素を通常の行と同じ単位にする (左端の列が幅 2 の書記素の右半分に当たるときは通常の行の横スクロールと同じ扱い)、(b) ステータスバーの桁 (`ui.column_unit = "grapheme"`) は書記素の数 + 1 (v0.2 は char の数 + 1)、(c) カーソルの左右の移動・Backspace / Delete の単位は今と同じ (後方 256 バイトの文脈の窓で求める書記素の境界。§11.5 の既知の制約の場合 (256 バイトを超える結合列・128 個を超える国旗の列) だけ、索引の境界と食い違い得る。通常の行と同じ扱い)、(d) 単語の移動は今と同じ窓 (64 KiB) の近似、(e) 折り返しは下の「長大行の折り返し」のとおり |
| 正しさの確かめ | 閾値を下げて同じ行を、通常の行の経路 (`units`) と長大行の経路 (索引) の両方で作ったとき、表示の単位・カーソルのセル・列からの位置・桁がすべて一致する。差分で引き継いだ索引が、作り直した索引と一致する。どちらも proptest (09 §3.2) |
| 速さ | 01 NFR-106 AC-4: BM-106 の 5 つの操作 (開く・`End`・水平と垂直のスクロール・中央挿入) が 3 種のコーパスで各 p99 ≤ 50 ms (Should)。**v0.2 の判定 (各 p99 ≤ 100 ms。Must) を割らないことを、書記素の単位に切り替える PR を入れる条件にする** (割るなら、その PR は入れずに char 近似のまま残し、書記素単位の正確さ (AC-5) を 08 §7 に繰り越す)。50 ms に届かないときは 08 §7 に繰り越しを記録する (Should) |
| 文言 | 書記素の単位に切り替えると「近似」の表示が誤りになるので、同じ PR で `info.open.long_line` と `status.degraded.long_line.*` の文言を「長い行を含む文書は折り返さない」意味に改める (04 §8.7 / §15.2 の案)。下の折り返しを v0.4 に入れたら、どちらも出さない |

**長大行の折り返し** (v0.4 の判定の条件にしない。01 §5.5 の NFR-106 AC-5、08 §2.5): v0.2 から、長大行を含む文書を表示する View は折り返しなしとして扱っている (§11.5.1 の表の「折り返し」)。v0.4 に入れるか v0.5 以降に送るかを、次の手順と条件で決める。

1. 時期: 書記素の単位の索引の PR (08 §2.5 の M25 の (5)) を入れた後。
2. 試作: `spike/` のブランチで、記録点を表示行の先頭に置く形 (§14.3 の表の「長大行」の行。4 KiB を超えた後の最初の表示行の先頭) で、折り返し ON の長大行の表示行 ↔ バイト位置の写像と、可視範囲の近くだけの正確な計算を作る。
3. 計測 (BM-106w、09 §5.2。参考): 1 行 10 MB の 3 種、`view.wrap = "word"`、120×40 で、開く・`End`・表示行の単位の垂直スクロール (`PageDown`)・中央挿入の p99。
4. 入れる条件 (すべて): (a) 3 の各操作が p99 ≤ 100 ms (v0.2 の長大行の判定と同じ値)、(b) 折り返しなしの BM-106 が劣化しない、(c) 製品の品質に仕上げる工数の見立てが 2 週 (フルタイム換算) 以内で、M26・M27 を遅らせない。
5. 条件を満たせば、実装の PR を M25 の (7) として出す (仕様との整合・性能と依存の 2 つの視点のレビューに掛ける)。満たさなければ v0.5 以降に送り、08 §7 に繰り越しを記録し、01 §4.6 の既知の制約に「1 MiB を超える行を含む文書は折り返さない」を書く。どちらでも、結果 (計測の値と決めたこと) をこの節と 08 §2.5 の M25 の行に書く。

**判定の結果 (M25 の (6)、2026-11-14): v0.5 以降に送った** (v0.5 の計画 (2026-11-20) で v0.6 以降に送った。分割 (FR-509) が View とレイアウトを変えるので、その後に作り直す。v0.5-plan §4.3、08 §7)。手順 2 の試作 (`spike/eve-core-long-line-wrap`。通常の行の経路との一致と、編集で引き継いだ索引の一致を試験で確かめた) で、手順 3 の BM-106w を測った (値と条件は `docs/perf/0.4.0-m25/summary.md` の「BM-106w」)。`End` は折り返し ON では 1 回目が表示行の末尾へ動くので、行末まで走査する 2 回目 (論理行の末尾へ) を判定に使った。(a) 3 種のコーパスの各操作は p99 ≤ 40 ms (速い状態) で満たすが、遅い状態 (約 2.3 倍) の見込みでは emoji の行末への `End` が約 91 ms で余裕が 1 割に満たず、参考の korean は約 146 ms で超える (非 ASCII の表示の単位あたりの折り返しの判定の費用で、折り返しなしの走査の 2.5〜2.7 倍)。(b) 折り返しなしの BM-106 は劣化しない。(c) 製品の品質に仕上げる工数の見立てが約 11.5 日で 2 週を超え、M26・M27 を遅らせる。v0.5 以降で入れるときは、この試作を土台にし、速さ (非 ASCII の単位の上乗せ) を詰めてから、上の手順 3・4 を測り直す。

**採らなかった案**: (1) v0.3 までのこの節の `LineLayoutCache` の形 (4 KiB ごとに折り返しの位置も記録し、Document × WidthConfig ごとに LRU 1,000 行で共有): 折り返しの位置は上の判断の後でよく、長大行を 1,000 行覚える場面が無い (画面に出る長大行は数行)。(2) 行末までの索引をジョブで作り、完了まで近似値を出す: 状態と表示の遷移が増え、02 §14 と OQ-31 の「ジョブ化しない」と食い違う。遅延と差分で最悪の走査を「行頭 (または変更の位置) から行末まで 1 回」に抑え、その時間を BM-106 で確かめる。(3) 編集の後ろの記録点を、バイト・表示列・書記素の数の差だけずらして使い回す: タブの幅 (列の差がタブ幅の倍数でないと後ろのタブの幅が変わる) と、編集の端の書記素の境界の確かめが要る。速くなるのは中央挿入の後の `End` だけで、判定の 5 つの操作には効かない。

### 14.5 Decoration の合成

```rust
pub struct Decoration { range: ByteRange, style: StyleDelta, priority: u16, kind: DecorationKind /*Selection|SearchMatch|Syntax|Diagnostic|AiDiff|BracketMatch|Whitespace|LossyEncoding*/ }
pub trait DecorationProvider { fn decorations(&self, doc: DocumentId, visible: ByteRange, out: &mut Vec<Decoration>); }
```

- Provider は **可視範囲を渡されて** その範囲の Decoration だけ返す ([02 §10.1](02-architecture.md#101-mvp-時点で用意する拡張点-内部-api))。全文の Decoration を保持する義務はない。
- **v0.1 では `DecorationProvider` trait を定義しない**。選択と検索一致 (と制御文字・全角スペース・タブの可視化) のスタイルは Frame 生成で直接合成する。trait は最初の外部利用者 (tree-sitter、v0.6) と同時に導入する (内部構造であり、先に trait 化しても公開面には影響しない)。`Decoration` / `DecorationKind` / 優先度の表は v0.1 から使う。
- 合成は優先度順 (低 → 高) に `StyleDelta` を上書き: Syntax (100) < Whitespace (150) < LossyEncoding (表現不能の文字、§8.6。反転 + 下線) (200) < SearchMatch (400。現在の一致はふつうの一致の代わり) < Diagnostic 下線 (500、下線のみ) < AiDiff (600) < Selection (900、背景のみ) < BracketMatch (950、属性だけを足す重ねるロール。07 §8.6) < カーソル (frontend)。数値は 07 のテーマで調整可能。v0.5 の実装 (`layout/rows.rs` の `build_row`) の順は、本文・記号 → 表現不能の文字 → 検索の一致 / 現在の一致 → 選択 → 対応括弧 (M32 の段 4。Syntax・Diagnostic・AiDiff は後の版)。
- 書記素の内側で Decoration が切れる場合 (バイト精度の診断など。検索の一致は §12.1 で書記素境界のものだけを採るので当たらない)、その書記素全体を含める方向に拡張する。

### 14.6 意味的 UI モデルの生成 (core 側の要件。型の正本は 02 §8.2)

テキスト領域以外の **意味的 UI モデル** を core がどう生成するかを本節に置く。型の骨格 (`Frame` / `OverlayModel` / `RenderRow` のフィールド) は 02 §8.2 が正本であり、本節は「core が何を埋めるか」と不変条件だけを定める。TUI とヘッドレス参照 Frontend が同時にこのモデルを消費するため、`OverlayModel` 等への変種追加は 02 の契約変更手続きに従い、frontend は未知の変種を既定描画に退化させる (02 §8.2)。

| モデル | core が埋める内容 | 根拠 |
|---|---|---|
| `Frame.messages` | `current: Option<Message { level, text, hint }>`、`unread: u16`。メッセージ行 (FR-505) の素材。`hint` は「どうすればよいか」(NFR-602) で、frontend は切り詰め時に `text` から縮める | 04 §8.9 |
| `OverlayModel::List` | `input: InputFieldModel` (02 §8.2: 1 書記素 = 1 Span の幅付きセル列、カーソルと選択は欄内の書記素番号。幅は core が `WidthConfig` (曖昧幅の設定を含む) で計算する)、`items[i] { label, match_ranges (label の **書記素番号** の半開区間の列。バイト位置ではない), keys: Vec<KeyLabel> (解決済みキーマップからの逆引き), disabled_reason: Option<CellText> (解決済みの文言、02 §8.2) }`、`selected`、`total`、`truncated`。パレット・バッファ一覧・ファインダ共通。照合と並び順は core (v0.1 は前方一致 → 部分一致 → アルファベット順。ファジー順位付けと最近実行順は v0.2、01 FR-602 / 07 §11.4)。**パレットの項目** は `hidden` でない全コマンド (`hidden` = `internal.*` と、オーバーレイ内専用の `ui.confirm` / `ui.list_next` / `ui.list_prev` / `ui.list_page_down` / `ui.list_page_up` / `ui.dialog_choose`。一覧の正本は 07 §11.6)。**引数の尋ね方** (v0.1): 選んだコマンドに既定値の無い必須引数があれば、同じオーバーレイの中で順に尋ねる。列挙値の引数は `List` (例: `file.reopen_with_encoding` → 対応する 7 つの文字コード)、それ以外は `Prompt`。`Esc` で 1 段戻る。省略可能な引数と既定値付きの引数は尋ねない (各コマンドの既定の動作) | 04 §8.1 / FR-605 (IME 追従には入力欄カーソルが必要) / FR-111 / FR-601 |
| `OverlayModel::Prompt` | `InputFieldModel` + 説明 + 検証エラー + 候補リスト (v0.5) | 04 §8.2 |
| `OverlayModel::Confirm` | `message`、`choices [{ id, label, key (アクセラレータ), destructive }]`、`default`、`focused` (初期値は `default`。`tab` / `right` で次、`shift+tab` / `left` で前のボタンへ移り、`enter` で `focused` のボタンを実行、`esc` は常にデータを変えない側。アクセラレータは組込みの dialog 層が単一書記素の `Text` を `key` と照合して `ui.dialog_choose` を実行する、02 §7.2 K1。破壊的な選択肢は `default` にしない。IME が ON で文字が前編集に入る場合は Tab / 矢印 + Enter で選べることを示す)、`details: Option<Vec<CellText>>` (1 行 = 1 つの `CellText`。core が解決・幅計算・無害化した文言、02 §8.2。§8.6 の置換の一覧に使い、送信プレビュー・権限要求の詳細表示に拡張可、06 X7)、`reason_untrusted: bool` (要求理由が非信頼であることの表示指示。v1.L、02 §8.2) | 04 §8.3 / 06 §13.1 |
| `OverlayModel::Help` (**v0.2**。v0.1 は変種の予約のみで生成しない。v0.1 のヘルプはコマンドパレットで代用する、01 FR-514) | `sections` (キー一覧はキーマップの逆引き) と `scroll`。タブなしの 1 本のスクロールで、`help.keys` / `help.commands` は該当の節へスクロールした状態で開く。ヘルプ内の検索は v0.5 | FR-514 |
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

### 14.7 行番号 (FR-502、v0.5 の M34 で相対を足す)

ガターの行番号は core が `RenderRow.gutter` の Span として作る (02 §8.2)。v0.5 の M34 で `view.line_numbers` (07 §5.3) の `relative` と `off` を入れる。受入基準は 01 §5.3 の FR-502 AC-1 (絶対) と §5.5 の FR-502 AC-2 (相対)。M34 の仕様の PR (2026-11-24) で決めた (試験・実装の段・決めた点は §15.9)。

| ID | 規則 |
|---|---|
| LNO-1 | **値**: `absolute` (既定。v0.4 までのまま)・`relative`・`off` (行番号を出さない。ガターの幅は 0 で、折り返しの継続行の `↪` (04 §10) も出さない。継続行の印が消えることは 01 §4.6 の既知の制約)。`config.reload` で即時に効く (表示中の全ペイン) |
| LNO-2 | **相対**: 主のカーソルの行には **絶対の行番号** を (今と同じく `ui.line_number.active` の太字)、ほかの行には主のカーソルの行からの **論理行の差の絶対値** (`1`、`2`、…) を出す (Vim の `number` と `relativenumber` を両方立てた形)。**主のカーソルの行の絶対の番号は左寄せ、ほかの行の差は右寄せ** にする (色・太字に頼らずに現在の行が分かる。フォーカスの無いペインと無色の段でも分かる。Vim の形と同じ)。折り返し ON でも論理行で数え、継続行は今と同じく `↪`。ロール (`ui.line_number`) は絶対と同じ |
| LNO-3 | **どのカーソルから**: View の主の Range の `head` の行から数える。ペインごと (フォーカスの無いペインも、そのペインの View の主のカーソルから数える。現在の行の太字はフォーカスの無いペインでは付けない、§9.9 の PANE-10 のまま)。主のカーソルが画面の外にあっても、その行からの差を出す |
| LNO-4 | **ガターの幅**: `absolute` と同じく、文書の行数の桁数で決める (相対の値の桁数に依らない。カーソルを動かしても幅が変わらない) |
| LNO-5 | **境界の型は変えない**: Span の文字列が変わるだけ。主のカーソルの行が変わると、見えている全行のガターが変わる (TUI のセルの差分で出す。費用は可視行の数に比例し、NFR-103 の予算の中) |
| LNO-6 | **ヘッドレスの ASCII の Frame**: ガターの文字列をそのまま出す (相対の値と、現在の行の左寄せが見える。新しいゴールデン、§15.9.1) |

---

## 15. タブとインデント

### 15.1 設定と検出

```rust
pub struct IndentConfig { style: Spaces | Tabs, width: u8 /*インデント 1 段のセル幅*/, tab_width: u8 /*タブ文字の表示幅*/ }
```

- 既定: `Spaces`, `width = 4`, `tab_width = 4`。言語別 (FR-704) / `.editorconfig` (FR-705) / ワークスペース設定で上書き (07)。
- **自動検出** (案。版は未定): 読込時に先頭 1,000 行の行頭空白を集計し、タブ主体か空白主体か・空白なら最頻の増分 (2/4/8) を推定。巨大ファイルでは無効 (FR-123)。検出結果は Document の `indent_detected: Option<IndentConfig>` に入り、設定より優先 (設定で検出を無効化可)。**v0.5 では入れない** (v0.4 までの本節は「v0.5」と書いていたが、01 に要件と AC が無い。入れるときは、言語定義・`.editorconfig` との優先の順 (07 §3.6) と一緒に要件を足す。M33 の仕様の PR で決めた、07 §12.6.3)。
- **v0.5 の M33**: `IndentConfig` (と `tab_width`) は文書ごとに決まる (07 §3.6 の LAYER-1。言語定義・`.editorconfig`・ワークスペース層・`--set` を重ねた値)。インデントの操作 (下の §15.3) と表示 (§14.2 のタブの幅) は、全体の設定ではなく文書の値を読む。

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
| 保存時の末尾空白除去 (FR-216) | 保存前に Transaction として適用 (§8.5 と同じ理由)。未編集の文書の上書き保存には適用しない (§9.3 の手順 2)。規則は §9.3.1 (v0.5 の M34。v0.4 までの本行の「カーソル行は除外する設定を既定にする」は、保存した内容がカーソルの位置で変わるので採らない。§15.9.3) |

### 15.4 行の複製・削除・上下移動 (FR-207、v0.2 の M9b)

対象の行は、各 Range が触れる行の和集合を連続する行のまとまり (ブロック) にしたもの。選択が行頭 (列 0) で終わり、その行より前から始まっていれば、最後の行は含めない (選んでいない行を巻き込まない。インデント / アンインデントも同じ)。複数 Range のブロックは重ならないように併合する。どの操作も 1 Transaction (Undo 1 回、`Origin::Command`)。

| 操作 | 規則 |
|---|---|
| `edit.duplicate_line` | ブロックを改行ごと、ブロックの直後にもう一度挿入する。最後の行に改行が無いときは、複製の前に `default_eol` を足す。選択 (カーソル) は、ブロック内の同じ相対位置のまま複製の側へ移る |
| `edit.delete_line` | ブロックを改行ごと削除する。最後の行が改行で終わっていなければ、手前の改行も取り除く (空の行を残さない)。文書が 1 行だけなら空になる。カーソルは、詰めた行の先頭 (最後の行を消したときは新しい末尾) |
| `edit.move_line_up` / `edit.move_line_down` | ブロックと、隣の 1 行の **内容を入れ替える。改行は動かさない** (改行の種類と、最後の行に改行が無いことはそのまま)。先頭のブロックの上移動と、末尾のブロックの下移動は何もしない。選択はブロックについていく |

- インデント操作後の Selection は、各 Range の始まりを前寄り (`Bias::Before`)・終わりを後ろ寄り (`Bias::After`) に写す (向きは保つ)。行全体を選んでいた場合は行全体の選択を維持する (行頭が後ろにずれない)。**空の Range (ほかの Range と一緒に選択の経路で扱うカーソル) は、行コメントの切替 (§15.6 の CMT-5 の細目 (b)) と同じく、行頭 (列 0) なら前寄り (列 0 のまま)、ほかは `MapPolicy::Own` (`Bias::After`)** に写す (空のカーソルが、足した字下げを選んだ選択に変わらない。Issue #630。アンインデントでも同じ写し方を使う。消した範囲の中のカーソルは、どちらの寄せ方でも消した範囲の始まりへ寄る)。
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

### 15.5 混在ファイル

- タブと空白が混在する行は表示幅で判断し (`tab_width`)、「1 段削除」は表示幅ベースで行う。混在の自動修正は行わない (v0.1、明示コマンド `edit.convert_indentation` で変換、v0.5)。

### 15.6 行コメントの切替 (FR-209、v0.5 の M33)

`edit.toggle_comment` (ID と既定キーは 07 §11.6 の M33 の表)。受入基準は 01 §5.5 の FR-209。規則は M33 の仕様の PR (2026-11-22) で決めた (試験・実装の順は 07 §12.6)。

| ID | 規則 |
|---|---|
| CMT-1 | **対象の行**: 各 Range が触れる行の和集合 (§15.4 と同じく、前の行から始まって行頭 (列 0) で終わる選択は、その最後の行を含めない。空の Range はカーソルの行)。全 Range の行を 1 つの対象にまとめ、同じ行は 1 回だけ扱う (§4.5 の 5)。対象のうち、空行と空白 (ASCII の空白 U+0020 とタブ) だけの行は飛ばす (判定にも編集にも使わない)。残りが 0 行なら文書を変えずに `info.comment.no_lines` を出す |
| CMT-2 | **付けるか外すか**: 記号 `t` は文書の言語の `comment.line` (07 §9.6 の LANG-3)。対象のすべての行で、行頭の空白の直後が `t` で始まっていれば外し、そうでなければ付ける (対象の全体で 1 回だけ決める。一部の行だけがコメントのときは全部に付ける。付けた後にもう一度実行すると、付けた分だけが外れて元に戻る) |
| CMT-3 | **付ける位置**: 対象の各行の行頭の空白の表示の幅 (タブは文書の `tab_width` で展開した列、§15.5) の最小値を `m` とし、各行で、行頭の空白の先頭から数えた幅が初めて `m` 以上になる位置 (幅が `m` ちょうどの行は行頭の空白の末尾) に、`t` と空白 1 つ (U+0020) を入れる。幅 `m` がタブの途中に落ちる行は、そのタブの後ろに入れる (タブを割らない)。文書の `indent_unit` には依らない |
| CMT-4 | **外し方**: 各行の行頭の空白の直後の `t` を消し、そのすぐ後が空白 (U+0020) 1 つならそれも消す (タブと 2 つ目の空白は消さない)。`t` の後ろが続く文字 (`t` = `//` の行 `///`) でも、`t` の分だけを消す |
| CMT-5 | **Transaction と選択**: 全部の行の編集を 1 つの ChangeSet にし、1 つの Transaction (`Origin::Command`) にする (1 回の Undo で本文と選択が戻る)。選択は §15.4 の下の箇条のインデント操作と同じく、各 Range の始まりを前寄り (`Bias::Before`)・終わりを後ろ寄り (`Bias::After`) に写す (行全体を選んでいれば行全体の選択を保つ。空のカーソルは、行頭 (列 0) なら列 0 のまま、ほかは `MapPolicy::Own`。下の細目 (b))。改行コード・文字コード・BOM は変えない |
| CMT-6 | **断る場合**: 言語に `comment.line` が無い (`plain` を含む) ときは、文書を変えずに `info.comment.unsupported` (`{language}` は言語の `display_name`) を出す。読み取り専用の文書では `when` (`!doc.read_only`) が偽で実行できない (パレットで淡色と理由、キーでは今の編集のコマンドと同じ読み取り専用の知らせ。巨大ファイルの閲覧 (`LargeFileViewer`) も読み取り専用なので同じ) |
| CMT-7 | **費用**: インデントの操作 (`edit.indent`) と同じく行の数の上限を置かず、行の数に比例する (選んだ行ごとに行頭の空白だけを見る。長大行 (NFR-106) でも行の全体を走査しない) |

**細目 (M33 の段 3 で決めた)**: (a) 断る順は CMT-6 (記号の無い言語) が先で、CMT-1 の「残りが 0 行」は記号のある言語でだけ調べる (`plain` の文書の空行で実行しても `info.comment.unsupported` を出す。言語を替えないと何をしても付けられないことを先に知らせる)。言語が決まっていない文書 (読込中は読み取り専用なので、ふつうは起きない) は `plain` として扱い、`{language}` は `plain`。(b) CMT-5 の写し方は、空でない Range は始まりを前寄り (`Bias::Before`)・終わりを後ろ寄り (`Bias::After`) に写す (§15.4 の下のインデントの操作の写し方と同じ。行全体の選択は行全体のまま、向きは保つ)。空のカーソルは、行頭 (列 0) なら前寄り (列 0 のまま)、ほかは `MapPolicy::Own` (`Bias::After`。付ける位置にあるカーソルは記号と空白の後ろへ移る)。外すときは、消した範囲の中の端は消した範囲の始まりへ寄る (選択の終わりが外した記号の中にあると、行頭へ寄り、次の切替ではその行は対象から外れる)。(c) 外した結果、CR (単独) で終わる行の次の行が空になり、その行が LF で終わるときは、CR と LF が隣り合って 1 つの CRLF の改行になる (行の数が 1 つ減る。手で記号を消したときと同じで、1 回の Undo で戻る。改行のバイトは変えない)。

### 15.7 括弧・引用符の自動閉じ (FR-210、v0.5 の M34)

`editor.auto_pairs` (07 §5.1。既定 `false`) が真のとき、括弧・引用符の開き字を打つと閉じ字を足す。受入基準は 01 §5.5 の FR-210。規則は M34 の仕様の PR (2026-11-24) で決めた (試験・実装の段・決めた点は §15.9)。

| ID | 規則 |
|---|---|
| APR-1 | **組**: 文書の言語の `auto_pairs` (07 §9.6 の LANG-3。開き字 → 閉じ字の表) があればその組、無ければ **既定の組** (定数): `(` `)`・`[` `]`・`{` `}`・`"` `"`・`` ` `` `` ` `` の 5 組。`'` は既定に含めない (英文の `don't`・Rust の生存期間 `'a` で邪魔になる。使う言語は言語定義で足す)。全角の括弧 (`「」` など) も既定に含めない (言語定義で足せる)。言語の表は既定の組を **置き換える** (足すのではない。`auto_pairs = {}` でその言語だけ止められる)。組込みの言語定義は `auto_pairs` を持たない (どの言語も既定の組。07 §9.6.1) |
| APR-2 | **対にする条件**: 開き字 `o` と閉じ字 `c` の組 (`o == c` の組は引用符) について、次のすべてを満たすときだけ、`o` を入れた直後に `c` を足し、カーソルを 2 つの間に置く。(a) 入力が `Paste` でない `Text` (打鍵・IME の確定・合体して届いた打鍵・Windows Terminal の短い貼り付け。**core はこれらを区別しない**) の中の書記素で、それが組の開き字。**2 書記素以上の `Text` は、書記素ごとに順に APR-2〜APR-4 を当てる** (`(a)` を 1 つの `Text` で受けても、1 文字ずつ受けても同じ結果になる。打つ速さで結果が変わらない)。貼り付け (`edit.paste`・`InputEvent::Paste`・バーストの合成、04 §6.6) では対にしない。キーマップの割当で引数を付けて呼んだ `edit.insert_text` (`{ command = "edit.insert_text", args = { text = "(" } }`) とパレットからの実行も対にしない (**打鍵の経路だけ**)。(b) Range が空 (選択のある Range は APR-5)。(c) カーソルの直後が、行末 (区切りの直前か文書の末尾)・ASCII の空白かタブ・全角の空白 (U+3000)・どれかの組の閉じ字 (`o == c` の組の文字を除く)・BRK-1 の全角の閉じ括弧 (`）` `］` `｝` `」` `』` `】` `〕`)・ASCII の `,` `;` `:` `.`・`、` `。` のどれか (語の直前で `(` を打つと対にしない: `|foo` で `(` → `(|foo`)。(d) 引用符 (`o == c`) では、カーソルの直前の書記素が英数字 (§13.3 の `Alnum` / `FullwidthAlnum`) でも同じ引用符でもない (`don't`・`""` の後ろで対にしない) |
| APR-3 | **閉じ字の飛び越え**: 書記素 (APR-2 (a) の順) が組の閉じ字 `c` で、カーソルの直後が **自動で足した** 同じ `c` (下の「覚え方」) なら、文字を入れずにカーソルを `c` の後ろへ進める (`(|)` で `)` を打つと `()|`)。自動で足していない閉じ字 (ファイルにあったもの・貼り付けたもの) は飛び越えない (`foo|)` で `)` → `foo)|)`)。引用符は、直後が自動で足した同じ引用符なら飛び越え、そうでなければ APR-2 で対にするか、そのまま入れる。**覚え方**: View が、自動で足した閉じ字の位置を Range ごとに覚え (編集の写像で追う)、カーソルがその開き字と閉じ字の間 (同じ行) を離れたとき (改行を入れたときを含む)・Undo / Redo・ほかの位置への移動 (クリック・検索・行へ移動など) で忘れる |
| APR-4 | **対の間の Backspace**: `edit.delete_backward` で、Range が空で、直前が組の開き字 `o`、直後がそれと組の **自動で足した** 閉じ字 `c` (APR-3) なら、`o` と `c` の両方を消す (`(|)` → `|`)。自動で足していない閉じ字の前では、今の 1 書記素の削除のまま。`edit.delete_forward`・単語の削除 (§5.3) は変えない |
| APR-5 | **選択があるとき**: 選択のある Range に開き字 (打鍵の経路の書記素、APR-2 (a)) が届くと、選択を **囲む** (選択の前に `o`、後ろに `c` を入れ、選択は囲んだ中身のまま保つ。向きも保つ)。選択の置き換えにはしない。複数行の選択も同じ (行ごとには囲まない)。閉じ字やほかの文字は今のまま (選択を置き換える)。2 書記素以上の `Text` も書記素ごとに当てる (囲んだ後は中身が選ばれたままなので、続く書記素は中身を置き換える。`(a` を 1 つの `Text` で受けても 1 文字ずつ受けても `(a|)` になる) |
| APR-6 | **複数カーソル**: 各 Range に APR-2〜APR-5 を独立に当て、全体を 1 つの Transaction にする (§4.5 の 1)。1 回の打鍵で、ある Range は対にし、ある Range は飛び越え、ある Range はそのまま入れる、が起こり得る (飛び越えだけの Range は文書を変えない。どの Range も文書を変えなければ Transaction を作らない)。矩形選択 (§4.8) の Range も同じ |
| APR-7 | **Undo**: 対にした入力・飛び越え・対の間の Backspace は、連続入力の結合 (§7.3 の R7 / R8) では **自動閉じを切っていたときの入力と同じに数える**: 対にした挿入の終わりはカーソルの位置 (開き字の後ろ) とみなし、飛び越えは閉じ字の入力とみなす (飛び越えは文書を変えないので Transaction を作らないが、結合を封印しない)。これで、同じ打鍵の列の Undo のノードの数は `editor.auto_pairs` の真偽で変わらない (`(a)b` を打つと、どちらも 1 つのノードで、1 回の Undo で全部が消える)。選択を囲む入力 (APR-5) は 1 つの Transaction で、新しい Undo のノードにする (選択の置き換えと同じ) |
| APR-8 | **効かないところと既知の制約**: 入力欄 (プロンプト・検索バー・パレット) では対にしない。読み取り専用の文書は今と同じく編集を断る。対の間の `Enter` で中の行を字下げする (VS Code の `{` `}` の間の改行) のは v0.6 以降 (構文の字下げ FR-803 と一緒)。文字列・コメントの中かを区別しない (構文の解析は v0.6 の §16)。**core は 2 書記素以上の `Text` を、IME の確定・高速の打鍵・端末の短い貼り付けと区別しない** ので、**既知の制約** (01 §4.6) は次の 3 つ: (1) ブラケットペーストの無い端末と Windows Terminal の短い貼り付け (16 打鍵未満、04 §6.6) は打鍵として届き、対にする。釣り合った括弧の貼り付け (`f(x)`) は APR-3 の飛び越えで結果が同じになるが、片方だけ (`(`) の貼り付けでは閉じ字が増える。(2) 行をまたぐ貼り付けが打鍵として届くと、改行で飛び越えの覚えが消える (APR-3) ので、閉じ字が二重になる (`{⏎␣␣x⏎}` が `{⏎␣␣x⏎}}` になる)。(3) IME で ASCII の開き字を確定すると対にする。どれも、`ctrl+k v` (端末に横取りされない `edit.paste`) で貼れば対にしない |

### 15.8 テキスト変換 (FR-214、v0.5 の M34)

コマンドは `edit.transform_upper` / `edit.transform_lower` (大文字・小文字)・`edit.sort_lines_ascending` / `edit.sort_lines_descending` (行の並べ替え)・`edit.trim_trailing_whitespace` (末尾空白の除去) の 5 つ (07 §11.6 の M34 の表。既定キーは付けず、パレットから使う)。受入基準は 01 §5.5 の FR-214。

| ID | 規則 |
|---|---|
| TXT-1 | **対象**: (a) 大文字・小文字は、空でない Range のそれぞれ (空の Range は飛ばす)。全 Range が空なら、各カーソルの位置の単語 (§4.5.1 の MCUR-2 (a) と同じ選び方。単語の無いカーソルは飛ばす) で、どのカーソルにも単語が無ければ文書を変えずに `info.select.no_word` を出す。(b) 並べ替えと末尾空白の除去は、どれか 1 つでも空でない Range があれば、空でない Range が触れる行 (§15.4 の取り方: 前の行から始まって行頭で終わる選択は、その最後の行を含めない) を連続する行のまとまり (ブロック) にしたもので、ブロックごとに行う (空の Range は数えない)。**全 Range が空なら文書の全体** を 1 つのブロックにする。どちらも、文書が区切りで終わるときの最後の空の行 (最後の区切りの後ろ、§2.4) はブロックに入れない (並べ替えで空の行が先頭へ移り、末尾の改行が消えるのを防ぐ) |
| TXT-2 | **大文字・小文字**: Rust の標準の `to_uppercase` / `to_lowercase` (§13.4。完全な Unicode・ロケールに依らない。`ß` → `SS` のように長さが変わり得る)。全角の英字も変わる (`ａ` → `Ａ`)。不正バイトのエスケープ文字 (§8.3) と、大文字・小文字の無い文字 (漢字・かななど) は変わらない。Range ごとに、変換した文字列が元と違えば Range の全体を置き換える。変換の結果が文書の文字コードで表せない (`ß` → `SS` は表せても、大文字・小文字の片方だけがその文字コードにある文字など) ときも変換は止めず、保存のときの表せない文字の確認 (§8.6 の E3) に任せる (M34 の段 2 のレビューで決めた) |
| TXT-3 | **並べ替え**: ブロックの各行の中身 (区切りを除いた文字列) を、**Unicode のコードポイントの順** (UTF-8 のバイトの順と同じ。文書の文字コード (エンコーディング) のバイトの順ではない) で **安定に** 並べ替える。`edit.sort_lines_descending` は逆の順で、等しい行は元の順を保つ (安定)。大文字小文字を区別し、数を数として比べない (`10` は `9` より前)。不正バイトのエスケープ文字 (`U+10FF00..`) もコードポイントとして比べる (後ろに並ぶ)。**行の区切りは動かさない** (§15.4 の行の移動と同じく中身だけを入れ替え、区切りの種類と最後の行に区切りが無いことはそのまま)。1 行のブロックは何もしない。**CR と LF をつながない**: 並べ替えると中身が空になる行で、前の行の単独の CR とその行の単独の LF が隣り合う (§2.4 で 1 つの CRLF になり、行の数と区切りの並びが変わる) ブロックは並べ替えない (TXT-4 と同じ考え)。飛ばしたブロックがあれば `info.transform.sort_skipped_cr_lf` を出す (ほかのブロックを並べ替えたときも、Transaction の後に出す。どのブロックも変わらなければ `info.transform.unchanged` の代わりに出す) (M34 の段 2 のレビューで決めた) |
| TXT-4 | **末尾空白の除去**: ブロックの各行の末尾 (区切りの直前) から前へ続く ASCII の空白 (U+0020) とタブ (U+0009) を消す。空白の定義と関数は保存時の変換 (§9.3.1 の SVT-2) と同じ (全角の空白 U+3000 などは消さない)。**CR と LF をつながない**: 消した結果、直前の単独の CR と直後の単独の LF が隣り合う行は消さない (§2.4 で 1 つの CRLF になり、行の数が変わるため。SVT-2 と同じ) |
| TXT-5 | **Transaction と選択**: 1 回のコマンドで 1 つの Transaction (`Origin::Command`、`UndoGroupHint::Break`。1 回の Undo で本文と選択が戻る)。本文が変わらなければ Transaction を作らず `info.transform.unchanged` を出す。選択: 大文字・小文字では、空でない Range は変換した文字列の全体を選ぶ (向きを保つ)。単語を選んだ空のカーソルは、単語の始まりからの書記素の数を保った位置に置く (長さが変わって収まらなければ単語の末尾)。並べ替えでは、各ブロックの Range をブロックの全体 (最初の行の行頭から最後の行の区切りの直前まで、前向き) にする (文書の全体を並べ替えたときは、主のカーソルを同じ行・桁 (書記素) に置いた 1 つのカーソルにする。§9.5 の再読込の写像と同じ)。末尾空白の除去は写像 (`MapPolicy::Own`。消した空白の中のカーソルは行末へ寄る) |
| TXT-6 | **大きいとき**: 対象の合計 (Range の長さの和・ブロックの長さの和) が `TRANSFORM_SYNC_BYTES` (**1 MiB**、定数。§8.4 の改行コードの変換・§12.5 のすべて置換の同期の上限と同じ値。§12.2 の `SEARCH_SYNC_STEP_BYTES` (256 KiB) とは別) 以下ならメインスレッドで ChangeSet を作り、超えれば `bg` のジョブ (`JobKind::TextTransform { snapshot, op, ranges }`、02 §9.3) で投入した時点の Snapshot から作る。**投入するときの対象の範囲 (`ranges`) を覚え**、結果の Revision が投入のときと違えば (待つ間の編集)、覚えた範囲を **その後の ChangeSet で写像した範囲** (今の選択は読まない。文書の全体を対象にしたときは今の文書の全体) で、今の Snapshot から 1 回だけ作り直す。写像した範囲が消えた (Range の長さが 0 になった・ブロックが 1 行も残らない。ブロックに区切りだけが残ったときも消えたとみなす) ときは諦めて `msg.transform.target_gone` を出す。**範囲のどれか 1 つでも消えたら全体を諦める** (残った範囲だけを変換しない。頼んだ範囲の一部だけを変えない)。作り直しの結果も古ければ適用せずに `msg.transform.stale` を出す (§17.2)。ジョブは文書の `Pending::busy_doc` に入れる (監視の読み直し・再読込 (§9.5.1) と重ねない)。**再読込との重なり**: 待つ間に、その文書の再読込 (`Origin::Reload` の置き換え: `file.reload`・`file.reopen_with_encoding`・監視の読み直しなど。§9.5) を始めたら、その時点で待っている変換を取り消して `msg.transform.target_gone` を出す (読込がどちらの順で終わっても、読み直した本文を変換しない。文書の全体を対象にした変換も同じ)。**編集できないとき**: 作り直し・適用の時点で文書が読み取り専用・読込中になっていたら、作らず・当てずに、編集と同じ読み取り専用の知らせ (`msg.edit.read_only` など。読込中は `msg.edit.read_only_loading`) を出す (文書が前に出ていなくても。`msg.transform.failed` はジョブの本当の失敗だけに使う)。適用の後の選択は、Range の数と `ChangeSet` の演算の数に比例する費用で決め、文書の大きさに比例する複写と走査をしない (単語の位置と行・桁は、適用の後の本文で、その単語・その行の書記素だけを数える) (M34 の段 2 のレビューで決めた)。待つ間も入力を受け付け、ステータスバーの左 6 の位置に `status.transforming.long` / `.short` を出す (種類は `StatusKind::Saving` を使い、境界の型を変えない。04 §8.7)。同じ文書に新しい変換を頼んだら前のジョブを取り消す。ジョブの失敗 (確保の失敗 `JobError::OutOfMemory`・パニック) は文書を変えずに `msg.transform.failed` を出す。並べ替えは行の数に比例する一時の領域を `try_reserve` で取る (02 §11.2.2) |
| TXT-7 | **断る場合**: 読み取り専用の文書では `when` (`!doc.read_only`) が偽で実行できない (パレットで淡色と理由)。全角・半角の変換・Unicode の正規化 (§13.5)・タイトルケース・数の順の並べ替え・重複の削除は v0.5 では入れない |

### 15.9 M34 (Could の枠) の試験・実装の段・決めた点 (v0.5)

v0.5 の M34 (08 §2.6。**判定の条件にしない**) は、FR-210 自動閉じ (§15.7)・FR-214 テキスト変換 (§15.8)・FR-216 保存時の変換 (§9.3.1)・FR-309 選択の拡張 (§5.7 の SEL-1〜SEL-7)・FR-311 ダブル / トリプルクリック (§5.8.2)・FR-212 矩形選択 (§4.8)・FR-310 ジャンプ履歴 (§5.9)・FR-407 検索履歴 (§12.8)・FR-502 の相対行番号 (§14.7) を入れる。本節は 9 つに共通の試験と計測・PR の分け方と各段の完了の条件・決めた点を置く (M34 の仕様の PR、2026-11-24。点検の材料は docs/review/v0.5-plan.md §3.5・§4.1・§4.3 と §1.3 の (7)。下書きに、仕様との整合・安全性・利用者と端末の 3 つの視点のレビューを掛けて直した)。既知の制約は 01 §4.6 の (v0.5 の M34) の行。**FR-126 ファイルツリーは本節の範囲の外** で、M34 の最後に別の仕様の PR で扱う (入らなければ 08 §7 に送る)。

**境界の型 (eve-view)**: 9 つとも変えない。FR-311 は押下の回数を core が数え (`MouseEvent` に回数を足さない)、FR-502 の相対行番号は `RenderRow.gutter` の Span の文字列を core が作り、FR-212 の矩形のモードはステータスバーの選択情報 (`StatusKind::Selection`) の文言で、FR-214 のジョブの待ちは `StatusKind::Saving` の位置の文言で示す (新しい `StatusKind` を足さない)。変えることになったら、その段の PR の前に仕様を直す (`contract-change` の扱いはその時に書く)。

#### 15.9.1 試験と計測 (09)

- **L0 / L1 (自動閉じ)**: 表の試験で APR-2 の (c)(d) の各場合 (行末・空白・全角の空白・閉じ字・全角の閉じ括弧・`,`・`、`・語の直前・英数字の後ろの引用符・`""` の後ろ)・APR-3 の飛び越え (自動で足したものだけ・忘れる契機)・APR-4・APR-5 (前向き・後ろ向き・複数行)・言語の `auto_pairs` (置き換え・`{}`)・キーマップの `edit.insert_text { text = "(" }` と貼り付けで対にしないこと。proptest で、任意の文書と任意の複数の Range と、任意の打鍵の列 (組の文字と英数字と空白を混ぜる。**改行は含めない**: 改行は `Enter` のキー (`edit.newline`) で届き `Text` に入らない。行をまたぐ場合は APR-8 (2) の既知の制約) について、(a) **同じ打鍵の列を任意に区切って `Text` に合体させても**、結果の本文と選択が 1 文字ずつ送ったときと同じ (b) 釣り合った括弧の文字列 (`(a[b]"c")` など) を打った結果の本文が `auto_pairs` を切って打った結果と同じで、**Undo のノードの数も同じ** (APR-7)。
- **L0 / L1 (テキスト変換・保存時の変換)**: proptest で、任意の文書 (CRLF・CR・混在・最後の行の区切りの有無・区切りの無い文書・タブと空白と全角の空白の末尾・不正バイトのエスケープ文字 (末尾を含む) を含む) について、(a) 並べ替えの結果の行の中身の多重集合が元と同じで、区切りの並び (種類と位置の順) が元と同じ。昇順の結果は隣り合う行が Unicode のコードポイントの順、等しい行は元の順 (安定)。降順も同じ (b) 末尾空白の除去 (TXT-4・SVT-2) の後は、**行の数が変わらず**、CR と LF がつながる行を除いてどの行の末尾にも U+0020 / U+0009 が無く、それ以外の文字の並びと区切りは元と同じ。2 回目は何も変えない (冪等) (c) 保存時の変換 (SVT-2 / SVT-3) を当てた本文が参照モデル「行ごとに末尾の空白を落とし (CR と LF がつながる行は落とさない)、空でなく区切りで終わらず最後の文字がエスケープでなければ `default_eol` を 1 つ足す」と一致し、ChangeSet が **消す空白の範囲と足す改行だけ** で、1 回の Undo で元に戻る (d) **往復のコーパス** (09 §4。区分 B の CP932 の重複符号化と EUC-JP の補助漢字、区分 D の ISO-2022-JP) の行の末尾に空白を足した文書を、変換を真にして元の文字コードで保存すると、変換した行 (と ISO-2022-JP の書き直した行) 以外のバイトが元と一致し、区分 B の保持方式の記録 (§8.7.1) が保たれる (e) 1 MiB を超える対象のテキスト変換のジョブの ChangeSet が、メインスレッドで作ったものと同じ (`TRANSFORM_SYNC_BYTES` を試験の値で小さくして両方の経路を通す)。表の試験: 大文字・小文字 (`ß`・全角・エスケープ文字・長さの変わる文字と選択)、**文書の末尾のカーソルで保存した後、打った文字が同じ行に入る** (SVT-5 の `Bias::Before`)、`a⏎(CR)␠⏎(LF)b` (`a\r␠\nb`) の除去で行の数が変わらない (SVT-2 / TXT-4)、区切りの無い文書に `default_eol` が足される (SVT-3)、TXT-6 の作り直しの範囲の写像と範囲が消えた場合。
- **L0 / L1 (選択の拡張・ダブル / トリプルクリック・矩形選択)**: 表の試験で SEL-2 の候補の順 (`foo(bar baz)` の各位置、複数行の括弧、括弧の無い行、範囲を超えた括弧)・SEL-1 の最後の空の行・SEL-3 / SEL-4 の履歴 (戻る・編集で捨てる)・CLK-1 の数え方 (間隔のちょうど 500 ms と 501 ms、位置の違い、ほかの入力をはさむ、押下によるフォーカスの移動では数え直さない)・CLK-4 の単位のドラッグ・BOX-3 の Selection の作り方 (タブ・全角の途中に落ちる列、短い行を飛ばす、幅 0、上限 + 1 行で行を走査しない)・BOX-2 の右の止まり方と `select.extend_end`・BOX-4 の矩形のコピー (短い行の空の片) と同じ高さの矩形への貼り付け・BOX-6 の継続行の押下の列。proptest で、(a) 任意の Selection に `select.expand` を k 回、`select.shrink` を k 回行うと元の Selection に戻る (b) 矩形の各 Range が、矩形の列の範囲 `[l, r]` に掛かる書記素だけを含む (表示列の参照モデルと一致する) (c) 矩形の Selection が INV-S1〜INV-S3 を満たす。
- **L3 シナリオ** (`crates/eve-core/tests/auto_pairs.rs`・`text_transform.rs`・`save_transform.rs`・`select_expand.rs`・`multi_click.rs`・`box_selection.rs`・`jump_history.rs`・`search_history.rs`・`line_numbers.rs` の案。AC の ID は 01 §5.5、`verifies:` を関数の直前に書く): FR-210 AC-1 / AC-2、FR-214 AC-1 / AC-2、FR-216 AC-1〜AC-3 (保存・同じファイルへの名前を付けて保存・全保存・終了の連鎖・`file.set_encoding` のそれぞれで効くこと、未編集の保存・未編集の `CheckingUnchanged` の不一致・確認の後の投入し直し (二重にかけない)・パスの変わる保存・`.editorconfig` の届く前・1 MiB 超で効かないこと、`.editorconfig` の 2 つのプロパティ、言語ごとの切り替え (組込みの `markdown` / `diff`)、**保存の失敗・中止 (`Conflict` で中止・`Busy` で中止・後続の取消し)・全保存の失敗の後も変換が本文に残り、1 回の Undo で戻ること、そのとき失敗・中止のメッセージと全保存の失敗の一覧に変換が残った旨が出ること** (SVT-8)、**承認の引き継ぎ** (区分 B の確認を承認 → 変換 → `Busy` の再試行で、確認が再び出ないこと。SVT-5)、`Continuation::Close` (閉じる前の保存) で効くこと、別名で保存へ移ったときに前のパスの変換のまま書くこと、保存の完了の知らせ (付記)、ISO-2022-JP の編集した行)、FR-309 AC-1、FR-311 AC-1、FR-212 AC-1 / AC-2、FR-310 AC-1 (検索の続けた移動が 1 項目)、FR-407 AC-1、FR-502 AC-2 (現在の行の左寄せ)。
- **fuzz**: `fuzz_editor` (`crates/eve-testkit/src/fuzzing.rs`) に、M34 のコマンド (07 §11.6 の M34 の表) と `editor.auto_pairs` を真にした文字の入力 (複数の書記素の `Text` を含む)・ダブル / トリプルクリック (`Mouse` の 2 回 / 3 回の押下)・矩形のモードでのドラッグを、M32 と同じく新しい操作の番号で足す (既存の種の意味を変えない。`cmd` の操作の一覧と `KEYS` の表には足さない)。保存時の変換は `fuzz_editor` の保存の操作で、設定を真にした場合を足す。
- **ゴールデン** (新しく作る): ASCII の Frame で、相対行番号 (`relative` の 80 桁の画面: 現在の行の絶対の番号が左寄せ、ほかの行の差が右寄せ。主のカーソルが画面の外のとき)・`off`・矩形のモードのステータスバー。
- **L4** (VirtualScreen): 相対行番号の現在の行の左寄せと太字 (truecolor と mono の段)。
- **キー**: 既定キーマップのゴールデン (`default_keymap.tsv`・`commands.tsv`) に 07 §11.6 の M34 の行が増える。FR-606 の頻出操作の一覧には足さない。
- **計測** (09 §5.2): **BM-216** (参考。判定しない) — 1 MiB の文書 (BM-103 のコーパスの全行の末尾に空白を 2 つ足したもの) の保存時の変換 (SVT-1〜SVT-5) のメインスレッドの時間を記録する (`TRANSFORM_SYNC_BYTES` の上限で、打鍵の予算 (NFR-103) と比べられるように)。テキスト変換の並べ替えは 10 MB の文書の全体の並べ替えの時間 (ジョブ) を同じ BM の参考の行に記録する。
- **手での確認 (M)**: 09 §8.2 の v0.5 の互換の行に書いた (この PR で足した)。Windows Terminal と conhost に加えて、tmux (`mouse on`・`extended-keys`)・SSH 越し・GNOME Terminal (IBus)・macOS Terminal.app / iTerm2 で、自動閉じ (IME が ON のとき、全角の括弧を確定したとき、短い貼り付け)・ダブル / トリプルクリック (tmux で押下が二重に届かないか。間隔)・矩形のモードのドラッグ・`Ctrl+L` / `Alt+U` / `Alt+I` / `Alt+J` / `Alt+K` / `Ctrl+K` の 2 打目 / `Alt+↑` の届き方 (Windows Terminal は `Alt+↑` を横取りするので `Ctrl+K ↑` で確かめる)・相対行番号の再描画のちらつき。判定の条件にはしない (Could)。

#### 15.9.2 実装の段と各段の完了の条件 (08 §2.6 の M34)

ゴールデンの扱いは全段で同じ: **新しいゴールデンは作ってよい。既存のゴールデン (eve-tui のスナップショット・`default_keymap.tsv` / `commands.tsv`・`dump_config_defaults.json` を含む) の差分の更新は、人間 (PO) が判断する** (§15.9.3 の (8))。

| 段 | PR (ブランチ) | 中身 | 完了の条件 |
|---|---|---|---|
| 1 | `feat/eve-core-relative-line-numbers` (`feat`) | FR-502 の相対: §14.7 の LNO-1〜LNO-6、`view.line_numbers` の公開 (07 §5.3。`absolute` / `relative` / `off`)、01 §4.6 の既知の制約 (`off` で継続行の印が消える) | 01 FR-502 AC-2 の試験と新しいゴールデン。`absolute` の画面は既存のゴールデンが差分なしで通る。公開による `dump_config_defaults.json` の行の追加・07 §5.0 の件数・`crates/eve-config/src/reserved.rs`・`config.example.toml` を直す。**落ちる既存の試験** (予約の例のキーを、別の予約のキー (`view.rulers` など) に替える): `crates/eve-config/src/config/tests.rs` の 222〜245 行 (AC-FR-701-3。`view.line_numbers` を公開前のキーの例にしている) |
| 2 | `feat/eve-core-text-transform` (`feat`) | FR-214: §15.8 の TXT-1〜TXT-7、`JobKind::TextTransform` (02 §9.3)、`TRANSFORM_SYNC_BYTES`、末尾空白の関数 (段 3 と共用)、07 §11.6 の 5 つの行、言語ファイル (04 §8.16) | 01 FR-214 AC-1 / AC-2 の試験 (§15.9.1 の proptest を含む) |
| 3 | `feat/eve-core-save-transform` (`feat`) | FR-216: §9.3.1 の SVT-1〜SVT-9 (連鎖の「変換を判定した」印・承認の引き継ぎ・書けずに終わったときの知らせ)、`editor.trim_trailing_whitespace_on_save` / `insert_final_newline_on_save` の公開と文書ごとのキー (07 §3.6)、言語定義の同じ名前の項目と組込みの `markdown` / `diff` (07 §9.6.1)、`.editorconfig` の `trim_trailing_whitespace` / `insert_final_newline` の写像 (07 §10.1。情報診断を外す)、言語ファイル、01 §4.6 の既知の制約 | 01 FR-216 AC-1〜AC-3 の試験 (§15.9.1 の往復のコーパスの性質を含む) と FR-705 AC-4 の書き換え。**仕様との整合と、データの損失・安全性の 2 つの視点のレビューに掛ける** (08 §2.6 の「保存の経路 (FR-216)」。保存の連鎖の全状態 (§9.7)・書けずに終わったときに本文に残る変換と知らせ (SVT-8)・承認の引き継ぎ (SVT-5)・05 §4.1 の手順・往復のコーパス (09 §4) の区分 B / D を見る)。BM-216 を記録する。どの層でも値が偽なら v0.4 と同じ保存 (既存の保存・往復コーパス・連鎖の試験が差分なしで通る)。公開による `dump_config_defaults.json` の行の追加・07 §5.0 の件数・`reserved.rs`・`config.example.toml` を直す。**落ちる既存の試験**: `crates/eve-config/src/editorconfig/tests.rs` の 76〜79 行と 206〜225 行 (`insert_final_newline` / `trim_trailing_whitespace` を写さないプロパティの例にしている。`max_line_length` などに替える) |
| 4 | `feat/eve-core-auto-pairs` (`feat`) | FR-210: §15.7 の APR-1〜APR-8、`editor.auto_pairs` の公開、言語定義の `auto_pairs` を読む (07 §9.6 の LANG-3 / LANG-4。情報診断を外す)、01 §4.6 の既知の制約 (APR-8)、ヘルプの案内 (04 §8.16 の `ui.help.terminal.auto_pairs_paste`) | 01 FR-210 AC-1 / AC-2 の試験。設定が偽のときは v0.4 と同じ入力 (既存の試験が差分なしで通る)。公開による `dump_config_defaults.json` の行の追加・07 §5.0 の件数・`reserved.rs`・`config.example.toml` を直す。**落ちる既存の試験** (`auto_pairs` を未対応の言語の項目の例にしている。`word_separators` などに替える): `crates/eve-core/tests/language.rs` の 389〜417 行、`bins/evoeditor/tests/cli_config.rs` の 65〜71 行 |
| 5 | `feat/eve-core-select-expand` (`feat`) | FR-309: §5.7 の SEL-1〜SEL-7 (`select.line`・`select.expand`・`select.shrink`)、07 §11.6 の行と既定キー、言語ファイル、ヘルプの案内 (`ui.help.terminal.ctrl_shift_l`・`ui.help.terminal.redraw`) | 01 FR-309 AC-1 の試験 |
| 6 | `feat/eve-core-multi-click` (`feat`) | FR-311: §5.8.2 の CLK-1〜CLK-7 (段 5 の単語・行の単位を使う) | 01 FR-311 AC-1 の試験 (手動の時計で間隔)。**`Harness::click` (`crates/eve-testkit/src/harness.rs` の 432 行) は時計を進めないので、同じ位置の 2 回の押下 (`crates/eve-core/tests/mouse.rs` の 58〜67 行) が 2 回押しに数えられる**: `Harness::click` は押す前に時計を多重クリックの間隔 (`DOUBLE_CLICK_MS`) より進めるようにし、`Harness::double_click` / `triple_click` を足す (既存の試験を直さずに済む)。単クリックとドラッグは v0.4 と同じ (既存のマウスの試験が差分なしで通る) |
| 7 | `feat/eve-core-box-selection` (`feat`) | FR-212: §4.8 の BOX-1〜BOX-9 (`select.toggle_box`、`select.extend_*` と `Shift` 付きのクリックの矩形の扱い、モードのドラッグ、矩形のクリップボード、`Esc` の層 (04 §7.5)、ステータスの文言)、01 §4.6 の既知の制約 (折り返し) | 01 FR-212 AC-1 / AC-2 の試験。モードを入れないときは v0.4 と同じ (既存の試験が差分なしで通る) |
| 8 | `feat/eve-core-jump-history` (`feat`) | FR-310: §5.9 の JMP-1〜JMP-7、07 §11.6 の行と既定キー、言語ファイル | 01 FR-310 AC-1 の試験 |
| 9 | `feat/eve-core-search-history` (`feat`) | FR-407: §12.8 の SHI-1〜SHI-6、07 §11.6 の `ui.history_prev` / `ui.history_next` と既定キー、言語ファイル | 01 FR-407 AC-1 の試験 |

- **段の依存** (入らなかった段が後の段を止めるか):
  - **機能の前提**: 段 2 → 段 3 (末尾空白の関数。段 2 が入らなければ、段 3 が関数を作り、段 2 はそれを使う)。段 5 → 段 6 (単語・行の単位の関数。同じく、段 5 が入らなければ段 6 が作る)。どちらも後の段を止めない。
  - **衝突を避ける順** (同じファイルを触るので続けて入れる。前の段が入らなくても後の段は入れられる): 段 3 → 段 4 (eve-config の言語定義の項目)。段 6 → 段 7 (マウスのドラッグの処理)。段 1・段 3・段 4 は、どれも `reserved.rs`・`config.example.toml`・07 §5.0 の公開キーの件数・`dump_config_defaults.json` を触る (同時に開かず、入った順に件数を数え直す)。段 8・段 9 は、どちらも検索の移動の後処理 (`search.next` / `search.prev`。記録の契機) を触る。
  - 段 1・段 8・段 9 はほかと並べて進められる。保存の経路を触るのは段 3 だけ (08 §2.6)。
- 各段は Could なので、入らなかった段は 08 §7 に繰り越しを記録する (判定の条件にしない)。FR-126 は別の仕様の PR (上の冒頭)。
- 文言 (04 §8.16) は、使う段の PR で言語ファイルと §15 の目録に足す (先に足さない。09 §6.2 の `i18n`)。

#### 15.9.3 決めた点 (M34 の仕様の PR、2026-11-24。退けた案と理由)

**PO に確かめる点** (戻しにくい決定。下の表の推奨案で進め、PO の決定を待つ): (1) 新しいコマンドの ID (`select.line`・`select.expand` / `select.shrink`・`select.toggle_box`・`cursor.jump_back` / `cursor.jump_forward`・`ui.history_prev` / `ui.history_next`・`edit.transform_upper` / `edit.transform_lower`・`edit.sort_lines_ascending` / `edit.sort_lines_descending`・`edit.trim_trailing_whitespace`) (2) 既定キー (07 §11.6 の M34 の表: `mod+l`・`alt+u` / `alt+i` と `ctrl+k u` / `ctrl+k i`・`editor` の `ctrl+k b`・`alt+j` / `alt+k` と `ctrl+k j` / `ctrl+k k`・`prompt` の `alt+up` / `alt+down` と `ctrl+k up` / `ctrl+k down`。テキスト変換は既定キーなし。2 打目に `Shift` を使わない。下の点検の表) (3) 設定のキー (新しいキーは足さない。`editor.auto_pairs`・`editor.trim_trailing_whitespace_on_save`・`editor.insert_final_newline_on_save`・`view.line_numbers` を公開し、既定は今の表のまま (`false`・`false`・`false`・`"absolute"`)。`editor.trim_trailing_whitespace_skip_cursor_line` は v0.5 では使わず、予約 (情報診断) のまま残す。上限・間隔は定数) (4) ディスク上の形式 (検索履歴とジャンプ履歴は v0.5 では保存しない。形式を決めない) (5) 境界の型 (9 つとも変えない) (6) ワークスペース層の分類 (公開する 4 つのキーを安全にする。07 §12.3 の注の「FR-216 のキーは準安全とする案」と違う) (7) 言語定義の項目 (`auto_pairs` を読み、表ごと置き換える。`trim_trailing_whitespace_on_save` / `insert_final_newline_on_save` を足し、組込みの `markdown` と `diff` は末尾空白を除かない) (8) 既存のゴールデンの差分の扱い (新しいゴールデンは各段の PR で作ってよい。既存のゴールデンの差分は、表どおりの行の追加でも、各段の PR で人間 (PO) が判断して更新する。M32・M33 の「表どおりの行の追加は更新してよい」の一括の許可はこの版では求めない) (9) **保存時の変換がカーソルの行も除くこと** (SVT-5。反対の論点: `foo␠` と打って `Ctrl+S` を押し、続けて `bar` と打つと `foobar` になる (打鍵の途中の保存で空白が消える)。推奨は除く (保存した内容がカーソルの位置で変わらない) で、困るなら自動保存を入れる版の `editor.trim_trailing_whitespace_skip_cursor_line` で選べるようにする) (10) **書けずに終わった保存でも、保存時の変換を本文に残すこと** (SVT-8。自動では戻さず、失敗・中止のメッセージに付記して知らせる。退けた案は自動で戻す: 保存の経路に足す例外が多く、Could には重い)。

| 点 | 採った案 | 退けた案と理由 |
|---|---|---|
| 自動閉じの既定 | `editor.auto_pairs = false` (07 §5.1 の表のまま) | `true` (VS Code の既定): 打った文字と違う文字が入る振る舞いを、v0.4 までの利用者の既定で変えない (日本語の文書で `「` を打つ使い方とも合わない)。入れたい利用者は 1 行で入れられる |
| 既定の組 | `()` `[]` `{}` `""` `` ` ` `` の 5 組。`'` と全角の括弧は含めない (APR-1) | `'` を含める: `don't`・Rust の `'a` で閉じ字が余る。言語ごとに組込みの組を持つ: 07 §9.6.1 の表に列が増え、言語ごとの規則を利用者が予想しにくい (言語定義で足せる) |
| 言語の `auto_pairs` の重ね方 | 表ごと置き換える (`{}` で止める) | キーごとに重ねる (§3.3 の表の既定): 組を外す書き方が無くなる |
| 対にする条件 | 直後が行末・空白・閉じ字・`,;:.` のときだけ、英数字の後の引用符は対にしない (APR-2) | 常に対にする: 語の前で `(` を打つたびに余分な `)` を消すことになる |
| 飛び越えと Backspace | 自動で足した閉じ字だけ (APR-3 / APR-4) | ファイルにある閉じ字も飛び越える・消す: 釣り合っていない既存のコードで、打った `)` が入らない・消すつもりの無い `)` が消える |
| 選択があるときの開き字 | 囲む (APR-5) | 置き換える: 自動閉じを入れた利用者には囲むほうが慣習 (VS Code・Sublime Text)。置き換えたいときは先に `Delete` で消せる |
| テキスト変換の対象 (選択が無いとき) | 大文字・小文字は単語、並べ替えと末尾空白は文書の全体 (TXT-1) | 何もしない: パレットから選んで何も起きないと分かりにくい。並べ替えの全体は大きな変更だが 1 回の Undo で戻る |
| 並べ替えの順 | コードポイントの順、安定、区切りは動かさない (TXT-3) | ロケールの照合順・自然な数の順: 依存 (ICU) が要り、OS で結果が変わる。行を区切りごと動かす: 混在の改行と最後の行の改行の有無が崩れる |
| 保存時の変換をかける時点 | すべての確認の後、書出しのジョブを投入する直前に、メインスレッドで同期に、連鎖ごとに 1 回 (§9.3.1 の SVT-4) | 保存先が決まった後・`PreConfirm` の前 (下書きの案): `PreConfirm` の確認の間に編集が割り込み、確認した本文と書く本文が分かれる。連鎖の開始 (保存先を尋ねる前): 名前の入力を取り消しても本文が変わる。投入のたび (印なし): 確認の後の投入し直しで 2 回かかり、Undo のノードが分かれる。保存のジョブの中で変換して書く: 保存したバッファとディスクの内容が食い違い、dirty の判定が壊れる (§8.5) |
| 未編集の保存 | 効かせない (§1.3 の (7)、SVT-4 (b)) | 効かせる: 開いただけのファイルに頼んでいない差分を出す (PO の決定 2026-11-20) |
| カーソルの行 | 除く (§15.3 の v0.4 までの「カーソル行は除外する設定を既定にする」は採らない。PO に確かめる点の (9)) | カーソルの行を除く: 保存した内容がカーソルの位置で変わり、`.editorconfig` の検査に通らない。v0.5 に自動保存は無いので、打鍵の途中で空白が消える場面は利用者が保存を押したときだけ (VS Code も手での保存ではカーソルの行を除かない)。自動保存を入れる版で、自動保存のときだけ除くかを決める |
| `editor.trim_trailing_whitespace_skip_cursor_line` の扱い | v0.5 では使わず、**予約 (「このバージョンでは未対応」の情報診断) のまま残す** (07 §5.13) | 段 3 で予約から外す: 書いていた利用者の診断が「未対応 (情報)」から「未知のキー (警告)」に変わり、自動保存の版で同じ名前を使うときにまた変わる。予約のままなら診断も変わらず、使う版で名前を決め直せる |
| 大きい文書の保存時の変換 | 1 MiB (`TRANSFORM_SYNC_BYTES`) を超えたら変換せずに保存し、完了のメッセージで知らせる (SVT-6)。ジョブを持たない | ジョブで作る (下書きの案。`Transforming` の状態): 確認と書出しの間に編集が割り込む窓・古い結果の積み直し・`Esc` での取消しの状態が増え、保存の連鎖 (データを守る経路) が複雑になる。保存を止める: 変換は見た目の整えで、書くことより後回しでよい |
| 値が決まりきっていないとき | 変換しない (パスの変わる保存・`.editorconfig` とワークスペース層の届く前。SVT-1。01 §4.6 の既知の制約) | 届いた値で後から変換する・届くまで保存を待つ: 保存が遅れ、待つ状態が増える。ユーザー層の値だけで変換する: プロジェクトが `false` にしていても消してしまう (頼んでいない差分) |
| 書けずに終わったとき | 変換を普通の Undo のノードとして本文に残し、失敗・中止のメッセージと全保存の失敗の一覧に「保存時の変換は本文に残っています ({key:edit.undo} で戻せます)」を付記する (SVT-8) | 自動で戻す (前の下書きの案): 正しく書くには、Undo としての戻し (ISO-2022-JP の行の保持の記録・`selection_before`)・「履歴の先頭か」の判定 (保存の待ちの間の Undo / Redo)・保存の失敗で読み取り専用になった文書での例外・散らばった連鎖の出口のすべてでの戻す時点・E3 の置換とつながりの確認の位置の写像・退避先に書いた内容との食い違い・捨てる Redo の扱いがすべて要り、Could の機能には重すぎる (保存の経路に誤りを持ち込む危険のほうが大きい)。黙って残す: 行末の空白は見えないので、保存しなかったのに本文だけ変わったことに気付かない |
| 変換の知らせ | 本文が変わったら、保存の完了のメッセージに付記する (SVT-5) | 知らせない (下書きの案): 行末の空白は既定の表示で見えないので、黙って本文を変える |
| 末尾改行の `false` | 足さないだけで、取り除かない (SVT-3。`.editorconfig` の `insert_final_newline = false` も同じ) | EditorConfig の仕様の文言どおり「終わらないようにする」(取り除く): 頼んでいない差分を出し、`false` を「何もしない」と読む書き手の意図と合わない |
| 末尾空白の定義 | ASCII の空白とタブだけ (SVT-2) | Unicode の空白 (U+3000 を含む): 日本語の文書で意図して置いた全角の空白を消す |
| `markdown` と `diff` | 組込みの定義で末尾空白を除かない (07 §9.6.1) | 言語に依らず同じ: Markdown の行末の 2 つの空白 (改行の意味) とパッチの文脈の行 (空白 1 つの行) を壊す |
| ワークスペース層の分類 (`editor.trim_trailing_whitespace_on_save` / `insert_final_newline_on_save`) | 安全 (07 §12.3) | 準安全 (07 §12.3 の注の案): 同じプロジェクトが `.editorconfig` (安全) で同じことを書けるので、`.eve/config.toml` だけ保留しても守れる範囲が変わらない。変換は利用者の保存に伴い、保存の完了のメッセージで知らせ (行末の空白は既定の表示では見えないので)、Undo で戻り、書けなければ戻す (B3) |
| 選択の拡張の順 | Range を真に含む候補 (単語・括弧の中・括弧を含む・行の中身・行の全体・文書) のうち最小のもの (SEL-2)。構文による拡張は FR-805 (v1.L) | 固定の順 (単語 → 行 → 括弧): 括弧が行の中にあるとき (`f(a, b)`) に行が先になり、括弧の中を選べない。文字列 (引用符の中) を候補にする: 構文を見ないので、`"a" + "b"` の真ん中を文字列と誤る |
| 選択の拡張のキー | 1 打の `alt+u` (広げる) / `alt+i` (縮める) と、代替 `ctrl+k u` / `ctrl+k i` (繰り返して押す操作なので 1 打の別名を持つ。2 打目に `Shift` を使わない) | VS Code の `alt+shift+right` / `left`: 既存の単語の選択の代替 (`select.extend_word_*`) で使っていて、Windows Terminal も横取りする。Helix の `alt+o` / `alt+i`: 従来の VT 端末では `Alt+O` が `ESC O` (SS3。アプリケーションモードのカーソルキーの前置き) と同じバイトで、続くキーと取り違える。`mod+w` (JetBrains): 文書を閉じる。2 打目の `shift+u` で縮める (下書きの案): CapsLock で向きが入れ替わり、`Ctrl+Shift+U` は IBus・kitty の Unicode 入力に取られる |
| 行の選択のキー | `mod+l` (v0.1 から予約) | 付けない: 予約していたキーを使わない理由が無い。従来の端末で `Ctrl+Shift+L` (すべて選ぶ) が `Ctrl+L` として届き行の選択になる点は、代替 `ctrl+k e` を案内する (07 §11.6) |
| ダブルクリックの回数の数え方 | core が押下の時刻と位置で数え、間隔は 500 ms の定数 (CLK-1) | frontend が回数を数えて `MouseEvent` に入れる: 境界の型が変わり、ヘッドレスの試験で回数を偽れなくなる。設定のキー: OS の設定を読めない端末で、定数より良い値を選ぶ根拠が無い (要望があれば足せる) |
| 矩形選択の作り方 | モード (`select.toggle_box`。種類は `Ui`) を入れて、既存の `select.extend_*`・ドラッグ・`Shift` 付きのクリックで矩形にする。抜け方は同じコマンドか `Esc` の層の最後 (BOX-1 / BOX-2) | 専用のキー (VS Code の `alt+shift+矢印`・`ctrl+alt+shift+矢印`): 前者は単語の選択の代替と Windows Terminal の横取り、後者は Linux の作業領域の移動に当たり、4 方向 × 2 のキーが要る。`Alt` + ドラッグ: 端末が自分の矩形選択に使う (§4.7.3 のマウスで足す操作と同じ理由) |
| 仮想の空白 | 持たない (BOX-5)。左の列に届かない行には Range を作らない (BOX-3)。キーボードでは最も長い行の幅 + 1 列で止める (BOX-2) | 空白を詰める: 頼んでいない空白を入れ、タブの文書で幅が合わない。短い行の行末にカーソルを置く: 打った文字が行ごとに違う列に入る。キーボードで 4,096 列まで右へ広げる (下書きの案): 何も選ばない列を押し続けられ、見えない矩形になる |
| 矩形のコピー | 矩形の行ごとに 1 つの片 (短い行は空の片) を持ち、同じ高さの矩形へ 1 行ずつ貼る (BOX-4) | Range の片だけ (MCUR-8 のまま): 短い行のある矩形では片の数が高さと合わず、同じ形の矩形へ戻らない |
| ジャンプ履歴の単位 | ペインごと、上限 100、閉じた文書の項目は消す (JMP-2 / JMP-5) | Window に 1 つ (VS Code): 分割したときに別のペインの移動で戻り先が混ざる。閉じた文書を開き直す: 戻るだけで I/O と開く確認が起きる |
| ジャンプにする移動 | 行へ移動・文書の端・対応括弧・検索の移動 (検索バーを開いている間の続けた移動は最初の 1 項目だけ)・ファインダと横断検索・文書の切り替え (JMP-1) | 一定の行数を超える移動を全部 (VS Code): `PageDown` を続けるだけで履歴が埋まる |
| ジャンプ履歴のキー | 1 打の `alt+j` (戻る) / `alt+k` (進む) と、代替 `ctrl+k j` / `ctrl+k k` (2 打目に `Shift` を使わない) | `alt+left` / `alt+right` (VS Code の Windows): 単語の移動の代替で使っている (v0.1 の既定を変えない)。`ctrl+o` / `ctrl+i` (Vim): ファインダと `tab`。`alt+-` / `alt+=`: 日本語の配列で `=` が `Shift+-` になり、`Alt+Shift+-` は Windows Terminal がペインの分割に使う。`ctrl+k shift+j` (下書きの案): 上の選択の拡張と同じ理由 |
| 検索履歴の保存 | v0.5 は保存しない (SHI-5) | `history.toml` に保存する (07 §16.1): 状態ディレクトリの形式・`history.enabled`・秘密の検索語の扱いを、セッションの復元 (FR-121) と別に決めることになる |
| 検索履歴の共有 | 検索バーと横断検索で検索語を共有し、置換は別 (SHI-1) | 別々: 横断検索は検索バーの検索語を引き継ぐ (03 §12.7.5) ので、分けると引き継いだ語が片方の履歴にしか残らない |
| 検索履歴のキー | `prompt` の `alt+up` / `alt+down` (v0.1 から予約) と代替 `ctrl+k up` / `ctrl+k down` | `up` / `down` を第一にする (VS Code の検索欄と同じ): 検索バーの `up` / `down` は v0.1 から一致の移動 (`search.next` / `search.prev`) に使っていて、横断検索の結果では `list` の候補の移動に使っている (どちらも空いていない) |
| 相対行番号の形 | 主のカーソルの行は絶対の番号を左寄せ、ほかは差を右寄せ (LNO-2)。論理行で数える | 主の行も `0`: 今いる行の番号が分からなくなる。表示行で数える: 折り返しの見た目と移動の単位が合うが、Vim の `relativenumber` と違い、論理行の操作 (行へ移動) の数と合わない |

**既定キーの点検** (M34 の仕様の PR で、07 §11.6 の M34 の表の既定キーを、既定キーマップのゴールデン `crates/eve-core/tests/golden/default_keymap.tsv` (2026-11-24 の `develop`)・07 §11.6 の予約・04 §7.3a の横取り表に照らした。macOS Terminal.app / iTerm2 は `Alt` + 文字を送らない想定 (04 §7.3a の注記) なので、`Alt` の 1 打はそこでは代替のチョードで使う。tmux・SSH・GNOME Terminal・macOS の端末での届き方は 09 §8.2 の手での確認):

| キー | コンテキスト | 既存の割当・予約 (同じフォーカスで届くもの) | 端末の横取り (04 §7.3a) | 採否 |
|---|---|---|---|---|
| `mod+l` | `editor` | 無し (v0.1 からの予約 `mod+l` = `select.line` を使う)。従来の端末では `mod+shift+l` (`select.all_matches`) と同じ符号で届く | 無し (Windows Terminal・conhost) | 採る。全端末で届く |
| `alt+u` / `alt+i` | `editor` | 無し (`editor` の `alt` + 文字は `z` `b` `f` だけ。`search` / `finder` の `alt+c` `r` `w` `l` `a` は別のフォーカス) | 無し | 採る (macOS の端末では `ctrl+k u` / `ctrl+k i`) |
| `ctrl+k u` / `ctrl+k i` | `editor` | 無し (`editor` と `global` の `ctrl+k` の 2 打目に `u` / `i` は無い) | 無し | 採る (代替) |
| `ctrl+k b` | `editor` | 無し (`search` / `finder` の `ctrl+k b` は別のフォーカス) | 無し | 採る |
| `alt+j` / `alt+k` | `editor` | 無し | 無し | 採る (macOS の端末では `ctrl+k j` / `ctrl+k k`) |
| `ctrl+k j` / `ctrl+k k` | `editor` | 無し | 無し | 採る (代替) |
| `alt+up` / `alt+down` | `prompt` | 無し (`editor` の `alt+up` / `alt+down` (行の移動) は別のフォーカス。`search` → `prompt` → `global` と `finder` → `list` → `prompt` → `global` の参照順のどの層にも無い) | Windows Terminal (ペインのフォーカスの移動) | 採る (Windows Terminal では `ctrl+k up` / `ctrl+k down`) |
| `ctrl+k up` / `ctrl+k down` | `prompt` | 無し (`search` の `ctrl+k` の 2 打目は `c` `x` `b` `l` `f`、`finder` は `c` `x` `b`、`global` は `r` `h`) | 無し | 採る (代替) |
| `alt+o` | `editor` | 無し | 無し | 退ける: 従来の VT 端末で `ESC O` (SS3) と取り違える |
| `ctrl+k shift+u` / `ctrl+k shift+j` | `editor` | 無し | 無し (`Ctrl` を離さないと `Ctrl+Shift+U` = IBus・kitty の Unicode 入力) | 退ける: CapsLock で向きが入れ替わる |
| `up` / `down` (検索の欄) | `search` / `list` | `search.next` / `search.prev`・`ui.list_next` / `ui.list_prev` | — | 退ける: 使用済み |

`view.line_numbers` は 07 §5.3 に「公開 v0.2」とあるが、実装では公開していなかった (`reserved.rs` の予約のまま)。M34 の段 1 で `absolute` / `relative` / `off` の 3 つの値で公開する (07 §5.3 を直した)。

---

## 16. シンタックスハイライト基盤

### 16.1 版と本書の範囲

- 実装 (tree-sitter、`eve-syntax`) は **v0.6** ([ADR-0015](10-decisions.md#adr-0015-シンタックスハイライト方式))。v0.1 で用意するのは `Document.language` (`LanguageId`)、Decoration の型 (§14.5)、`DocumentChanged` 通知の ChangeSet 形式と、`SyntaxProvider` / `detect_language` の **名前の予約** だけである。§16.2〜§16.5 の署名・規則は v0.6 (tree-sitter と同時。`detect_language` は v0.5) に確定する案である (02 §10.1)。
- 「v0.1 で tree-sitter を一時的に組み込んで試す」ことは **しない** (バイナリサイズ NFR-205、C コンパイラ依存を v0.1 で抱えない)。

### 16.2 言語判定

```rust
pub struct LanguageId(Arc<str>);   // "rust", "markdown" … 07 の言語定義に対応
fn detect_language(path: Option<&Path>, first_line: &str, cfg: &LanguageRegistry) -> Option<LanguageId>;   // v0.5
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

- 括弧対応 (FR-308) は v0.5。Provider なし (プレーンテキスト・未対応言語): 文字ベース走査 (§5.7 の BRK-1〜BRK-6。v0.5 の M32)。
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
| ワークスペース検索 (v0.5) | Document 単位で (a) | — |
| 改行コード変換・すべて置換の ChangeSet 構築 (`JobKind::ConvertEol` / `ReplaceAll`、v0.1) | **(b) 破棄して再要求**: 結果の base Revision が今の Revision と違えば捨て、今の Snapshot で 1 回だけ作り直す (1 MiB 以下の文書は同期で再計算)。それも古ければ適用せずに `msg.eol.convert_stale` / `msg.search.replace_stale` を出す。文書が閉じられた・検索バーが閉じられたら破棄 | §8.4 / §12.5 |
| テキスト変換の ChangeSet 構築 (`JobKind::TextTransform`、v0.5 の M34) | **(b) 破棄して再要求**: 結果の base Revision が今の Revision と違えば捨て、投入したときの対象の範囲をその後の ChangeSet で写像した範囲で、今の Snapshot から 1 回だけ作り直す (範囲が消えていれば `msg.transform.target_gone`)。それも古ければ適用せずに `msg.transform.stale`。文書が閉じられた・同じ文書に新しい変換を頼んだら破棄。その文書の再読込を始めたら取り消して `msg.transform.target_gone`。保存時の変換 (FR-216) はジョブを持たない (§9.3.1) | §15.8 の TXT-6 |
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
pub enum InsertText { Str(Arc<str>), Slice { key: SliceKey, snapshot: Snapshot, range: ByteRange } /* 削除 64 KiB 以上の記録直後のみ */, Rope(Snapshot) /* 実体化後・全置換 (§6.5)。ropey の型は公開しない */ }
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
    format: TextFormat, preserved: PreservedMap /* v0.4、§8.7.1 */, revision: Revision, history: UndoHistory,
    pending_changes: ChangeLog /* 02 §5.3。v0.1 は常に空。v1.L で §17.1 の予算規則 */,
    language: Option<LanguageId>, decorations: DecorationSet /* 02 §5.1 */, read_only: ReadOnlyReasons /* 理由の集合 (02 §5.1、§9.6)。読込中は理由に入れない */,
    load_state: Loading { progress } | Partial /* v0.4 の M25: 先行表示の間。text は先頭、§9.2.1 */ | Ready | Failed(err),
    // 以下は本書が 02 §5.1 に加えるもの
    canonical: Option<FsPath>, indent: IndentConfig, external_modified: Option<FileStamp>,
    saving: Option<JobId>, swap: SwapState /* v0.2 */, syntax: SyntaxState /* v0.6 */, layout_cache: LayoutCacheTable,
}
pub struct View { id: ViewId, doc: DocumentId, selection: Selection, goal_dcol: Option<u32>, scroll: ScrollPos { top: Pos /*先頭可視行の行頭*/, wrap_idx: u32 /*v0.2*/, left_col: u32 /*水平スクロール量 (表示列)、v0.1*/ }, wrap: bool /* v0.1 は常に false */, viewport_size: CellSize }
pub struct Transaction { changes: ChangeSet, selection_before: Option<Selection>, selection_after: Option<Selection>, origin: Origin, is_undo: bool, view: Option<ViewId>, timestamp: Tick /* Clock Port、02 */, undo_group: UndoGroupHint, meta: TransactionMeta /* 拡張メタデータ欄 (06) */ }
pub enum Origin { User, Command(CommandId), Paste, Reload, Ai { request: AiRequestId, proposal: ProposalId }, Lsp, Plugin(PluginId), External(ClientId), Recovery, System }
pub enum UndoGroupHint { Auto, Merge, Break }
pub struct NodeId(u64);   // 履歴の位置の通番。単調増加・再利用しない (§7.1、02 §5.1)
pub struct UndoHistory { undo: VecDeque<UndoNode>, redo: Vec<UndoNode>, base: NodeId, next_id: u64, saved: Option<NodeId>, budget: UndoBudget }   // 線形 (§7.1)。木構造は FR-213 で内部を入れ替える
pub struct TextFormat { encoding: Encoding, has_bom: bool, default_eol: LineEnding, eol_stats: EolStats, final_newline: bool, roundtrip: RoundtripSummary /* v0.1〜v0.3 は検出の結果、v0.4 は保持方式の要約。記録は Document.preserved (§8.7.1) */, invalid_bytes: u32, pua_escape: PuaEscapeMode }   // §8.1。後ろの 2 つが本書の追加
#[non_exhaustive] pub struct SearchQuery { pattern, kind, case, whole_word /* v0.1 は false 固定。変更は v0.2 */, scope /* v0.1 は Document 固定。変更は v0.2 */ }   // 構築は SearchQuery::literal(pattern) 等のビルダー経由 (OQ-41 の準備、§12.1)
// ClipboardText (eve-view、02 §6.3): Inline(String) | Handle(ClipboardHandleId)。Effect::SetClipboard が運ぶ (eve-view は eve-text の型を参照しない、02 §4.3 R4)。
// Handle の中身 ClipboardSource (Snapshot + 範囲、Send) は eve-core の Editor のハンドル表にあり、Host が Editor::take_clipboard(id) で取り出す (§12.6、02 §5.4)
// Proposal / Hunk の型は 02 §10.2.1 が正本 (eve-view に置く提案。eve-ai が生成)。本書は演算のみ (§18)
pub trait SyntaxProvider { /* §16.3。v0.1 は名前の予約のみ、署名は v0.6 */ }
pub trait DecorationProvider { fn decorations(&self, doc: DocumentId, visible: ByteRange, out: &mut Vec<Decoration>); }   // v0.6 で導入 (§14.5)
// FileSystem trait / FileStamp / SaveOptions / FsErrorKind は 02 §4.5 が正本 (§9.1 は要求一覧のみ)
// JobRequest { id /*JobSpawner が採番*/, doc: Option<DocumentId> /*文書に属さない RecoveryScan は None*/, base, kind } / JobKind / JobResult / JobError は 02 §9.3 が正本。v0.1 の JobKind は Open (OpenOptions、§9.2) / Save / Metadata (MetaPurpose: UnchangedSave・SaveAsTarget、v0.2 で ExternalCheck) /
// ReplaceUnmappable (§8.6 E3 の置換、bg) / Search / Materialize (§6.5) / WriteRecovery (§9.7 の「退避先に保存」、bg。結果は RecoveryWritten { path }) /
// RecoveryScan (起動時の退避の走査、io。結果は RecoveryFound { dirs }、§10.1)、
// JobKind の変種は Port を持たない (run_job が JobPorts から取る)。JobRequest.base は Revision (Option ではない)。
// v0.2 で Swap。後の版の変種 (Parse v0.6、EolConvert / ReplaceAll v0.2、WorkspaceSearch v0.5、v0.2 の起動時の保存中フラグの走査 (list_inflight) 等) は最初の利用者と同時に 02 §9.3 に足す
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
| INV-S3 | Selection | v0.1〜v0.4 では `ranges.len() == 1`。v0.5 の M32 から `1 <= ranges.len() <= MAX_CURSORS` (10,000。§4.5.1 の MCUR-6。設定で解除・変更しない) |
| INV-C1 | ChangeSet | `Σ Retain + Σ Delete == len_before`、`Σ Retain + Σ Insert.len == len_after`、正規形 (CS6) |
| INV-C2 | ChangeSet | `Insert` の文字列は妥当な UTF-8。全 Op の境界 (各 `Retain` / `Delete` の両端と各 `Insert` の位置) が適用対象で char 境界 (`from_sorted_edits` と `apply` が検査し、違反は `NotCharBoundary`。INV-T1) |
| INV-D1 | Document | `text` の内容と履歴が対応 (読込直後の内容に `undo` スタックの `redo` を順に合成すると現在の内容になる) |
| INV-D2 | Document | `revision` は `apply` ごとに厳密に +1。(v1.L〜) `pending_changes` は Revision 順で、畳み込み後も合成すると「最古の未決要求の base → 現在」の写像を与える (§17.1)。v0.1 では常に空 |
| INV-D3 | Document | `dirty == (history.saved != Some(現在の履歴位置))` (位置は §7.1 の通番 `NodeId`。saved が None なら true)。予算による履歴の破棄の前後で dirty は変わらない |
| INV-D4 | Document | `Loading` 中は `apply` が Rejected (Reload/Recovery を除く) かつ編集不可 (`load_state` から導出し、`ReadOnlyReasons` には理由を入れない。§9.6) |
| INV-D5 | Document | `format.eol_stats` の合計 == `len_lines() - 1`、かつ種類別の件数が内容と一致 (編集ごとに §6.5 の「eol_stats の更新」で保ち、編集の端での CR と LF の合体・分離 (§2.4) と巨大な削除でも近似しない) |
| INV-D6 | Document | `format.final_newline == (最終行が空)` |
| INV-D7 | Document | 読込の直後 (編集の前) は、`invalid_bytes > 0` ⇒ テキスト中の `U+10FF00..=U+10FFFF` の個数 == `invalid_bytes` (PUA 衝突フラグが立っていない場合)。v0.4 の M23 から、編集の後は成り立たない (`invalid_bytes` は読込の数のままで、エスケープ文字は消す・複製する・貼り付けで増減する。§8.3.1)。どの時点でも `escape_encoding` は `None` か `Some(encoding)` で、`Some` なら `pua_escape == Enabled` で `encoding` は状態付きでない |
| INV-H1 | UndoHistory | 各ノードの `redo` と `undo` は互いに逆 (CS2/CS3)。`undo` スタックの `redo` を順に合成したものと `undo` を逆順に合成したものも互いに逆 |
| INV-H2 | UndoHistory | 文書ごと `Σ nodes.bytes <= budget.max_bytes`、または `undo.len() <= 1` (直前 1 操作の例外)。プロセス全体 `Σ <= 256 MiB` は **v0.2** (複数文書と同時、§7.5) |
| INV-H3 | UndoHistory | 新しい編集 (`record`) の後は `redo` スタックが空。ノードの `id` は文書内で一意かつ `undo` スタック上で底から末尾へ増加し、`base` より大きい。`saved` が Some なら、その値は `base`・`undo` のいずれかのノードの `id`・`redo` のいずれかのノードの `id` のどれかである (戻れない位置を指す保存点は None にする、§7.1。保存の完了時にも検査して None にする、§7.4) |
| INV-H4 | UndoHistory | `InsertText::Slice` / `InsertText::Rope` は削除 64 KiB 以上のノードと、文書全体の置き換え (再読込の新全文・全体の削除の旧全文。大きさを問わない、§6.5 の「実装の状態」) にのみ現れ、`Slice` は実体化ジョブの完了前にしか存在しない (§6.5)。ランダムな 1 万回の編集の後、実体化ジョブがすべて完了した時点の RSS 増分 ≤ Undo 会計の 1.2 倍 (09 §3.2) |
| INV-L1 | LayoutCache | `revision` / `width_cfg_hash` が Document / 設定と一致しないエントリは参照されない |
| INV-J1 | Job | ジョブに渡した Snapshot の Revision ≤ 結果処理時の Document.revision |
| INV-F1 | Frame | Frame に制御文字 (TAB/LF/CR を含む)、§13.2 の可視化対象の不可視文字 (可視化が有効なとき)、および **PUA エスケープとして扱われている** `U+10FF00..FF` (`pua_escape` が `Enabled` か `DisabledStateful` の文書のもの。§8.3) が **含まれない** (無害化済み、NFR-503)。`DisabledCollision` の文書が真に含む `U+10FF00..FF` は通常の私用文字として扱う |
| INV-F2 | Frame | 各 RenderRow の `Σ Span.width <= viewport_cols` |
| INV-F3 | Frame | `RenderRow.width_risky == 行内のいずれかの書記素で is_width_risky() が真` (§13.2) |
| INV-IO1 | 保存 | **Unix** (macOS / Linux を出荷する版): 原子的置換 (`rename`) が完了するまで元ファイルのバイト列は変わらない。**Windows** (v0.1): `ReplaceFileW` の途中失敗時、元ファイルは無傷のまま残るか、または退避名 (`.<name>.eve-bak-*`) で完全な状態で残り、`ReplacedMovedToBackup` で位置が報告される (原子性ではなく回復可能性を主張する)。いずれも失敗時に一時ファイルが残らない (削除不能時はログに位置を残す) |
| INV-IO2 | 保存 | 保存成功時のディスク内容 == Snapshot をエンコードしたバイト列 (ジョブ内で書いたものと同一) |
| INV-IO3 | 保存 | 無編集の上書き保存は書き込まない (§9.3 手順 0) ので文字コードによらずバイト一致。編集後の保存は §8.7 区分 A でバイト一致、区分 B は v0.4 から保持方式で編集していない範囲が一致 (v0.1〜v0.3 は E2' の確認後に代表バイト列。「保持なし」の文書は除く、§8.7.1)。区分 D は意味一致 (v0.4 は、ASCII 状態で始まり終わる、編集していない行がバイト一致) |
| INV-IO4 | 保存・読込 | 利用者のデータ量に比例する確保は `try_reserve` で行い、失敗はエラーとして返す (abort しない。v0.3 の読込は、rope の節の確保が失敗を返せないので、§9.2 の見込みの検査 (ファイルの大きさ分の try_reserve) で代える。見込みを通った後の不足は abort。貼り付け等は SAF-27 の残り)。ユーザーデータの大きさに比例する **再帰を core に書かない** (ChangeSet の合成・履歴の走査・正規表現の構文木は反復で書く。スタックオーバーフローは退避できない) |

---

## 21. 性能目標との対応

数値目標の正本は 01 §3 (OQ-113 で確定)。ここでは 01 §3 の値 (判定区間・統計量を含む) と本書の手段を対応づける。

| NFR | v0.1 のゲート (01 §3) | 本書の手段 | 残るリスク (計測は 09) |
|---|---|---|---|
| NFR-101 起動 | **(a) プロセス内** (`main` 入口 → 最初の `write`+`flush`): p50 8 ms / p99 15 ms (全 OS)。(b) spawn から: Windows p50 40 / p99 80 ms (基準機 ENV-W1 の条件 = Defender 無効・サードパーティ製のウイルス対策ソフト有効で判定、ウォーム。Defender 有効の値は参考値として別記、01 §3.0)。Linux/macOS p50 20 / p99 40 ms (出荷する版)。(a) を CI の回帰検知と判定に使う | core 側: Document なしの `EditorState` 構築は定数時間。正規表現・Unicode テーブルの初期化は遅延 | `unicode-width` 等の静的テーブルはゼロコスト。regex のコンパイルは検索時 |
| NFR-102 1 MB 表示 | (a) プロセス内 p50 25 / p99 50 ms。(b) は NFR-101 (b) + 20 ms | 1 MB は 1 チャンク以内で Complete。rope 構築 ~数 ms、レイアウトは可視行のみ | encoding_rs の UTF-8 検証 ~1 ms |
| NFR-103 入力 | p50 2 / p99 8 ms で判定 (1,000 回計測、ウォームアップ 100 回を除く)。p99.9 (目安 16 ms) は **参考値** として記録し、合否に使わない (01 NFR-101〜105 AC-1)。条件: 120×40、1 MB の日本語混在 UTF-8、カーソルは中央、検索一致のハイライト有効、打鍵間隔 30 ms | `apply` は O(log n) + 逆変換 (削除量)、Selection 写像 O(k)、レイアウトは可視行のみ、検索ハイライトは可視範囲のみ | 可視範囲の同期検索 (§12.2) が複雑な正規表現や長い行で遅い場合 → 1 行あたり 16 KiB と時間予算 1 ms を超える分はジョブに回す |
| NFR-104 100 MB | 進捗表示 ≤ 50 ms / 全読込 **p50 1.0 s** で判定 (30 回計測。最大値は参考値として記録し、目安 1.5 s。01 §3.0 の「統計」) (Partial による先頭表示 300 ms は v0.4 の M25、§9.2.1。BM-104v で判定)。1 GB は参考値の記録のみ、512 MiB の警告は Must | 全読込は I/O + UTF-8 検証 + rope 構築 (ropey の `from_reader` 相当で 1 GB/s 級)。`metadata` / `file_id` もジョブ内 | 非 UTF-8 (Shift_JIS 100 MB) は再エンコード照合 (§8.7、ストリーミング) 込みで 1.5 倍許容内か (S-01 で実測) |
| NFR-105 100 MB 中央挿入 | p50 1 / p99 5 ms (コマンド実行時間: 編集コマンドの開始から Transaction の適用完了まで。Frame 生成を含まず、01 §3.0.1 の (a) core 区間とは別) | rope O(log n)、逆変換は挿入なので空、`eol_stats` 更新は挿入文字列と編集の端の前後 1 バイトの走査のみ (§6.5) | Undo ノードの結合判定は O(1) |
| NFR-106 長大行 | **v0.1 Must: 閾値 (1 MiB) 超の行を含む文書は読み取り専用で開き、その行は先頭 64 KiB まで表示。表示・End・スクロールが p99 ≤ 100 ms**。v0.2: char 近似モードで表示・End・中央挿入が p99 ≤ 100 ms。50 ms と書記素単位の正確さは v0.4 の M25 (書記素の単位の列の索引、遅延と差分。§14.4) | §11.5 の縮退 (v0.1)、char 近似モード (v0.2、§11.5.1)、書記素の単位の索引 (v0.4、§14.4) | char ごとの表引きが数百 MB/s 出るか (09 BM-106 で実測)。v0.4: 書記素の分割の高速経路で、開いた直後の `End` (行末までの 1 回の走査) が 50 ms に収まるか (絵文字の多い行が最も重い) |
| NFR-107 保存 | 参考値 (fsync は OS とディスクで決まる)。Must は「**保存中も NFR-103 を維持する**」 | ストリーミングエンコード (チャンク直書き)、追加メモリなし。保存はジョブなので UI は止まらない | macOS の `F_FULLFSYNC` は遅い |
| NFR-108 / 109 検索 100 MB | **リテラル検索** に 50 ms / 300 ms (Should)。正規表現は「入力を止めない」のみ Must、所要時間は参考値 | カーソル位置から前方優先走査、`memmem` で 1 GB/s 級。結果はバッチ送信 (256 件 / 20 ms) | 行単位の正規表現は 100 MB の全件走査が 1〜2 s になり得る → 件数上限と打ち切り表示 |
| NFR-110 ハイライト 50 ms (v0.6) | — | 旧 Decoration の写像で即時表示、解析はジョブ、`budget` 20 ms で Partial | 注入言語の多い Markdown で解析が重い |
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
| 改行 | 任意の LF/CRLF/CR 混在列で `len_lines`、`line_terminator`、`eol_stats` が参照実装と一致、正規表現 (v0.4 の複数行) の `\n` が全区切りに一致 | 手書き走査 |
| 改行の合体・分離 (§2.4 / §6.5) | `\r` と `\n` を多く含む任意の文書に、端が `\r` / `\n` に接する挿入・削除・置換 (CRLF の間への挿入、`\n` で始まる挿入、`\r` で終わる挿入、`\r` と `\n` の間の削除、64 KiB 以上の削除を含む) を任意に適用した後、増分で更新した `eol_stats` が全文を数え直した値と一致し INV-D5 を満たす。Undo / Redo の後も一致する。編集コマンド経由 (`eol_guard`、§6.2) の挿入・削除では、改行を 1 つ挿入すると `len_lines` がちょうど 1 増え、行の中身だけを消しても `len_lines` が減らない | 全文の数え直し |
| 検索 (§12.1 / §12.4) | 任意のテキストとパターンで、(a) 一致の列がチャンク分割 (1 バイト葉を含む) に依存しない、(b) 一致は互いに重ならず左から順、(c) 両端が書記素境界、(d) `Insensitive` の 3 経路 (Sensitive 扱い・ASCII・正規表現) の結果が、単純ケースフォールディングによる素朴な照合と一致する (K U+212A・ſ U+017F・全角英字を含むテキストで検査)、(e) **v0.4 (M24)**: 正規表現 (行をまたぐもの・k が 0 のもの・上限なし) の一致の列と前の一致が、全文を 1 本に連結して同じ書き換えの検索語で `regex` を走らせ (a)〜(c) の選び方で選んだ列と一致する (窓の核を 1〜40 バイトに縮めて窓の境目を多く作る。§12.3.1)、(f) **v0.4 (M24)**: CP932 互換文字の同一視 (§12.4.1) の結果が、組の片側にそろえた文字列での素朴な照合と一致し、範囲は元のバイト範囲、(g) **v0.4 (M24)**: CR と LF を含まない文書で同一視を切ると、書き換えた検索語と元の検索語 (`m`・`R` 付き) の一致とキャプチャが同じ (書き換えそのものの確かめ)。CRLF を含む文書では、`\r` も行の終端に 1 文字ずつ一致しうる書き方も含まない検索語について、CRLF と CR を LF にそろえた文書で `\n` を LF だけとして探して位置を元へ戻した結果と一致する (CRLF を分けて一致させない、§12.3.1) | `String` 全体での素朴な走査 |
| 文字コード (バイト → テキスト → バイト) | 任意のバイト列 (有効/無効混在) について、**`roundtrip` の範囲外は decode → encode が元のバイト列と一致** し (UTF-8 / UTF-16 / Shift_JIS / EUC-JP / ISO-2022-JP)、範囲内は v0.1〜v0.3 では `RoundtripSummary.lossy_lines` に数えられ、v0.4 では保持方式 (E2) で一致する。ISO-2022-JP は行単位 (§8.7 区分 D): 差のある行のうち表現不能の文字を含まない行の数 == `lossy_lines`、正規形の行は一致。不正バイトのエスケープ数 == `invalid_bytes`。**チャンク分割 (1 バイト単位を含む。ISO-2022-JP ではエスケープシーケンスの途中で切る分割を含む) によって結果が変わらない** | 全体を 1 チャンクで decode した結果 |
| 保存エンコーダの決定性 (§9.3 の「乾式走査と出力長」、v0.1、L1) | 任意の Snapshot (チャンクの分割を任意に変えたものを含む) と format (UTF-8 / UTF-8+BOM / UTF-16LE/BE / Shift_JIS / EUC-JP / ISO-2022-JP) について、(a) `SaveEncoder::dry_scan` の `out_len` == `SaveEncoder::new` の出力の連結の長さ、(b) 表現不能の判定 (有無・位置・総数) が両者で一致、(c) `SaveEncoder::new` を 2 回作って得た出力が同じバイト列、(d) キャンセル確認の回数・チャンクの区切り方で結果が変わらない | 1 チャンクで encode した結果 |
| 保持方式 (v0.4、§8.7.1。M22) | (a) 任意の編集・Undo / Redo・再読込の列の後に元の文字コードで保存して開き直すと、本文が保存の時点の文書と一致し、編集していない記録の範囲の出力は元のバイト列に一致し、編集した範囲に元のバイト列を書かない。(b) 乾式走査と本番の書き込みの出力が、記録を入力に含めても一致する (チャンクの分け方に依らない)。(c) どの時点でも、各記録が §8.7.1 の不変条件 (元のバイト列を初期状態から復号すると範囲の今の文字列になり、初期状態で終わる) を満たす。(d) 編集してから全部 Undo すると、記録が編集の前と同じになる | — |
| 不正バイトのつながり (v0.4、§8.3.1。M23) | 不正バイトを含む任意のバイト列 (UTF-8 / UTF-8+BOM / UTF-16LE/BE (BOM の有無) / Shift_JIS / EUC-JP) を読んだ文書に、エスケープ文字の並びを動かす・複製する・消す・間に文字を入れる・文書の先頭に置く編集を任意に加えて、(a) 局所の検査が 0 件 ⇔ 書いた BOM を除いた出力の全体を、同じ文字コードを指定して (初期状態の復号器で) 復号すると本文に一致する (参照モデル: 出力の全体を 1 本の復号器で復号して比べる。書く文字コード自身の BOM の並びが先頭にできる場合も局所の検査が見つける。M23 の仕様のレビューで総当たりと乱数で確かめた性質を固定する)、(b) 参照モデルの最初の不一致が、見つけた最初の箇所の強調の範囲の中にあり、示す位置は局所の復号で最初に違いが出たエスケープ文字である、(c) つながりの無い文書を保存して開き直すと、本文とエスケープ文字のバイト値・順が保存の時点の文書と一致する (NFR-304 AC-4)、(d) 区切った確認 (ii) は、保持方式の記録 (§8.7.1) が正しい限り `VerifyFailed` を出さない (記録の隣にエスケープ文字を置いた場合、記録を使う保存と「保持なし」の保存の両方を含む)、(e) 乾式走査でつながりの件数・位置・全件のダイジェスト・変わる範囲 (`changed_chars` / `changed_lines` / `to_eof`) がチャンクの分け方に依らず、本番の書き込み (件数・先頭 100 件・全件のダイジェストだけを求め、変わる範囲は求めない) がそれと一致する、(f) 変わる範囲が参照モデル (§8.3.1 の「示す位置と変わる範囲」の閉じ方の定義どおりに、区切らない復号器を単位ごとに進め、その単位で出たものがその単位の文字で終わり、持ち越しが無くなった単位の前で閉じる。重なる範囲はまとめる) と一致する、(g) `escape_encoding` が書く文字コードと違うとき (違う文字コードで保存した後の Undo、文字コードの違う復旧) はエスケープ文字を元のバイトに戻さない | 出力の全体の復号 |
| 書記素とエスケープ文字 (v0.4、§13.1。M23) | 任意の Unicode 列にエスケープ文字を混ぜても、エスケープ文字の前後が必ず書記素の境界になり、それ以外の境界は `unicode-segmentation` と一致する | `unicode-segmentation` + エスケープ文字の前後で切る |
| 貼り付けのエスケープ文字 (v0.4、§8.3.1。M23) | 任意の貼り付け文字列・内部レジスタ (`notation` / `escape_origin` / `indistinct`)・挿入先の文書 (6 種、§8.3.1 の表)・`editor.paste_normalize_eol` (純粋な関数で両方の値)・届かない場合と NUL で切れた場合の組で、挿入の後の本文に入った `U+10FF00..U+10FFFF` は、(1) 内部レジスタと一致し (両側の改行を正規化して比べる。NUL で切れた一致を含む)、`escape_origin` と挿入先の `escape_encoding` が同じ `Some` のときだけで、その数は内部レジスタの中の数と同じ、(2) 衝突の文書 (a) への挿入 (衝突の文書 (b) からのコピーを除く) で、貼り付け文字列の中の数と同じ、のどちらか。それ以外では 0 個で、外から来たものは `<U+10FFNN>`、内部レジスタ由来は `<0xNN>` の文字列になる。知らせ (`as_bytes` / `escape_as_text` / `notation_as_text` / `pua_as_text`) の種類と件数が規則と一致する。挿入の入口 (貼り付け・`edit.insert_text`・置換) のどれでも同じ結果になる | 規則の表 (§8.3.1) の素朴な実装 |
| ISO-2022-JP の正規形と状態引継ぎ (§8.6 E6) | (a) 冪等性: `encode(decode(encode(t))) == encode(t)` (正規形は再エンコードで変わらない)、(b) チャンク分割不変: 1 行の内部で状態を引き継いで任意に分割して encode した連結 == 分割せずに (行ごとに閉じて) encode した結果 (1 文字単位の分割を含む)、(c) 終端: 出力は常に ASCII 状態で終わる (`has_pending_state() == false`。非 ASCII を含めば末尾は `ESC ( B`)、(d) 行末の状態: 各行末 (LF / CR の直前) が ASCII 状態 (`¥` / `‾` で終わる行を含む。OQ-162 で行ごとにエンコーダを閉じる方式に確定 (ユーザー回答))、(e) 行単位の区分 D 件数 == 「全体を比較したときに異なる行のうち、表現不能の文字 (半角カナ等) を含まない行」の数、(f) 半角カナ・`SO` / `SI` / `ESC` を 1 文字でも含む文字列は E1 の照合で 100% 検出される | 全体を 1 回で encode した結果 / 行ごとの比較 |
| 単語境界 (§13.3) | 境界は単調・書記素境界上・パニックなし。期待表の全件一致は L0 のゴールデン | 期待表 |
| 長大行の列の索引 (v0.4、§14.4。M25) | (a) 結合文字・ZWJ の列・国旗・ハングルの音節と字母・InCB の子音と Linker (GB9c)・タブ・全角・制御文字・エスケープ文字を重み付けした任意の行で、閾値を下げて長大行の経路 (索引) で作った表示の単位・カーソルのセル・列からの位置・桁 (書記素の数) が、通常の行の経路 (`units`) と一致する (記録点の間隔を小さくして、記録点をまたぐ場合を多く作る)。(b) 任意の編集・Undo / Redo の列の後、差分で引き継いだ索引 (記録点・`built_to`) が、同じ本文から作り直した索引の同じ範囲と一致する (行頭が動く・行が分かれる・つながる編集と、**行頭ちょうどへの挿入・直前の改行の変更 (CR の後ろへの LF の挿入・CRLF の片方の削除) を必ず生成に含める**)。(c) 遅延で作る順 (先に行末・先に中央・左から少しずつ) に依らず同じ記録点になる | 通常の行の経路、作り直した索引 |
| 文字コード (テキスト → バイト → テキスト) | 当該文字コードで **表現可能な文字集合のみ** から生成した文字列について encode → decode が一致する。生成器は §8.6 の照合で表現不能と判定される文字 (¥ ‾ − 半角カナ等の黙示置換対象、JIS X 0212、互換文字の表の JIS 系の文字) を **除外** する。Shift_JIS の生成器は CP932 の外字 U+E000〜U+E757 を **含める** (自前の逆算で往復する、§8.7)。除外対象を含む文字列については「E1 の照合が 100% 検出する」ことを別の性質として検査する | — |
| 複数カーソル | N 個の Range への同時挿入/削除の結果が「各 Range を末尾から順に個別適用」した結果と一致 | 逐次適用 |
| キー入力列 (v0.1。fuzz `fuzz_editor` は v0.2) | 任意の `InputEvent` / `CommandInvocation` 列でヘッドレス `Editor` がパニックせず、各ステップの Frame が INV-F1 (制御文字の無害化) を満たす | — |
| 設定 (v0.1) | 任意の TOML 文字列 (構文エラー・型エラー・未知キーを含む) で読込がパニックせず、該当項目が既定値になり診断が出る (FR-703) | — |

### 22.2 fuzz (cargo-fuzz, NFR-305)

- v0.1 は **デコード** の 1 ターゲット (01 NFR-305、09 §3.3)。**キー入力列** と **設定** は v0.1 ではプロパティテスト (§22.1) で同じ性質 (パニックしない・Frame の不変条件) を検査し、fuzz のターゲットにするのはキー入力列が v0.2 (設定はプロパティテストのまま)。各ターゲット: **デコード** (`fuzz_decode`。任意バイト列 + 任意の文字コードラベル (ISO-2022-JP を含む) + 任意の sniff 長 + 任意のチャンク分割。判定 §8.2 の手順 4' を通し、ISO-2022-JP では状態を引き継いだ再エンコードと行単位比較がパニックせず、復号できない入力が不正バイトとして数えられる)、**キー入力列** (`fuzz_editor`、v0.2。任意の `InputEvent` / `CommandInvocation` 列でヘッドレス `Editor` がパニックしない。各ステップで Frame 生成まで行い、制御文字の無害化 (NFR-503) も検査する)、**設定** (07。プロパティテストで代える)。v0.2 以降の追加: ChangeSet の `apply` (任意の ops 列 → エラーかパニックなしの成功)、正規表現パターン (任意文字列 → コンパイル成功またはエラー、パニックなし)、`recovery/manifest.toml` とスワップ (破損ファイル)、レイアウト (任意テキスト × 任意 WidthConfig × 任意幅 → INV-F1/F2/F3)、単語境界 (任意 Unicode 列 → 境界が単調・書記素境界上・パニックなし)。v0.4 の M23 の追加: `fuzz_encode` に、エスケープの方針 `Restore` でつながりの検査と区切った確認 (ii) を入れる (パニックなし。乾式走査と本番の書き込みで結果が同じ。§22.1 の「不正バイトのつながり」の (a) を fuzz の入力でも確かめる)。`fuzz_editor` に、読み取り専用の解除とその確認の選択、`U+10FF00..` を含む貼り付け・入力の操作を足す (09 §3.3)。
- **正規表現の検索** (`fuzz_regex_search`、v0.4 の M24。09 §3.3): 任意の文書・検索語・フラグ・窓の核の大きさ・チャンクの分け方で、行の窓と全文の連結の経路がパニックせず、全文を 1 本に連結して同じ書き換えの検索語で `regex` を走らせた結果 (§12.1 の選び方) と、一致・前 / 次の一致・置換の展開が同じ。2 つ目の参照として、CRLF と CR を LF にそろえた文書での結果とも比べる (§12.3.1)。

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
- 先行表示 (v0.4 の M25、§9.2.1。01 §5.5 の FR-122 AC-2〜AC-4): `MemFs` の読み手にチャンクごとの遅延 (`ManualClock` をチャンクごとに進める) を入れて、(a) 200 ms を過ぎてから最初のチャンクの区切りで `Partial` が 1 回だけ届き、先頭が 1 MiB 以下で行の区切り (CRLF を分けない) で切れ、全文の接頭辞になっている。改行の無い行では書記素の境界で切れ、末尾の CR の前で切れる。200 ms の前に読み終えると `Partial` が届かない。再読込・開き直しでは届かない (`OpenOptions.partial` が立たない)。(b) 先行表示の間、編集・保存は拒否され、検索・置換・行へ移動・すべて選択・文書の末尾まで選択を伸ばす操作は `msg.open.partial_unavailable` で拒否され (本文の無い `Loading` でも同じ。再読込の間の `Loading` では検索できる)、先頭の末尾より先へは動かず `info.open.partial_end` が 1 回出る。`Query::DocumentText` は `NotLoaded`。(c) 完了でカーソル・選択・スクロールが引き継がれ (書記素の境界へ寄る)、Revision が先頭の続きで、起動時の `+行` は完了で移る。(d) 先行表示の後の判定のやり直しで `Partial` が送り直され、カーソルが先頭に戻る。(e) 取消し・失敗・閉じるで先頭が捨てられ、開く前の文書 (無ければほかの文書、無題) が表示される。`Esc` は表示中の文書の読込だけを取り消す。(f) 先行表示の時点で表示が切り替わり、その後に元の文書へ戻すと完了でも切り替わらない。オーバーレイ (名前の入力等) が開いている間は切り替わらず、閉じた後に切り替わる。起動時の 2 つ目以降のパスでは切り替わらない。何も入力していない無題のバッファは先行表示の時点では閉じられず、完了で閉じられ、失敗では同じ番号のまま表示に戻る。(g) 先行表示の文書はスワップ・緊急退避の対象にならない。
- 標準入力 (v0.4 の M25、§9.2.2。01 §5.5 の FR-120 AC-1〜AC-5): 偽の `StdinSource` (チャンクの列・遅延・途中のエラー・閉じないパイプ・端末を注入できる) で、(a) EOF まで読んだ内容が無題の文書になり、dirty で、終了で FR-107 の確認が出る (`--readonly` では dirty でなく、読み取り専用の解除で dirty になる)。(b) 先頭の 64 KiB が揃うまで判定しない (1 バイトずつ届く Shift_JIS でも Shift_JIS と判定する)。`--encoding` が効く。判定のやり直しはしない。区分 B を含む Shift_JIS の入力を同じ文字コードで名前を付けて保存すると、入力とバイトが一致する (保持方式の記録)。(c) バイナリ・不正バイトの多い内容で確認を出さずに読み取り専用 (`Binary` / `InvalidBytes`) になる。不正バイトを含む ISO-2022-JP で、標準入力がパイプなら `msg.stdin.unsaveable_copy` (起動し直す案内なし)、ふつうのファイル (`StdinKind::File`) なら `msg.stdin.unsaveable` (`--encoding` を付けて起動し直す案内) が出る (v0.5 の M29)。Host の判定 (`stdin_is_file`) は、子プロセスにファイル・パイプ・`NUL` / `/dev/null` を渡して確かめる (05 §9.1 / §9.2)。(d) 上限 (試験では `file.huge_file_warn_bytes` を小さくする) を超えると、そこまでの文字の境界までが `LoadIncomplete` で開き、`msg.stdin.truncated` が出る。名前を付けて保存の成功で `LoadIncomplete` が外れ、`info.save.partial_saved` が出る。(e) 閉じないパイプで、標準入力の文書を表示して `Esc` を押すと確認が出て、「ここまでで開く」でそこまでが `LoadIncomplete` で開き、`info.stdin.stopped` が出る。ほかの文書を表示しているときの `Esc` は標準入力に触れない。読込中に閉じる・終了すると `ui.confirm.stdin_loading` が出る (0 バイトなら出ない)。データが無い間、読込のジョブが起きない (時計を進めても受け手が回らない)。(f) 読込の間もほかの文書を編集・保存でき (`io` のジョブが待たされない)、終了できる。(g) 読み終えた文書は緊急退避の対象で manifest が `sensitive = true`、スワップは最初の編集 (駆動 API の `External` を含む) の後から。`--readonly` で開いた文書 (変更なし) も、名前を付けて保存するまで緊急退避の対象で (1 バイトも読まなければ対象外。標準入力でない文書の対象は変わらない)、シグナル・「保存を中止して終了」の退避の文書の数に入り、保存中に閉じて保存に失敗したら退避先に書く (v0.5 の M29)。(h) 標準入力が端末なら空の文書と `info.stdin.terminal`。
- 文字コード往復: フィクスチャ (BOM 各種、CRLF/CR/LF 混在、末尾改行なし、不正バイト、CP932 重複符号 (0xEEF9 / 0xFA54)、CP932 互換文字 (0x8160 / 0x8161 / 0x817C / 0x8191 / 0x8192 / 0x81CA / 0x815C)、CP932 の外字 (0xF040 / 0xF07E / 0xF080 / 0xF0FC / 0xF140 / 0xF9FC)、JIS X 0212 を含む EUC-JP、ISO-2022-JP の正規形、先頭 64 KiB が ASCII で後半のみ ISO-2022-JP) を開いて 1 文字編集して元の文字コードで保存 → v0.1 は区分 B の文書 (重複符号・JIS X 0212・ISO-2022-JP のエスケープ選択) で確認ダイアログ (E2') が出て件数が一致し、承認後は区分 C の該当箇所のみ変化。互換文字と外字は区分 A なので E2' の件数に入らず、編集箇所以外のバイトが一致する (外字の文書で外字の隣を編集しても外字のバイトは変わらない)。v0.4 は無編集保存でバイト一致 (区分 A / B) と編集後の区分 C を検査し、区分 B の文書で区分 B でない箇所を 1 文字編集して保存すると確認 (E2') が出ずに編集していない範囲がバイト一致し、区分 B の箇所を編集すると、その範囲だけが代表バイト列になることを確かめる (§8.7.1)。
- ISO-2022-JP (v0.1、09 §4 の日本語ゴールデン): (a) 正規形のファイル (`ESC $ B` / `ESC ( B`、各行末で ASCII、末尾 `ESC ( B`) → 区分 B 件数 0、1 文字編集して保存 → 再読込でテキスト一致、元と編集箇所以外のバイトが一致。(b) 旧 JIS `ESC $ @`、`ESC ( J` で書かれた ASCII、冗長なエスケープ、行末で ASCII に戻らない、末尾が ASCII 状態でない、各ファイル → 読込時に区分 B の行数が期待値と一致し情報メッセージが出る。無編集の上書き保存は書き込まない。編集後の保存は E2' の確認後に正規形で書かれ、再読込でテキスト一致 (v0.1〜v0.3。v0.4 は、ASCII 状態で始まり終わる、編集していない行が元のバイト列のまま書かれ、確認は §8.7.1 の「保持なし」「保持できなかった箇所がある」文書だけ)。(c) `ESC ( I` の半角カナを含むファイル → 読込は半角カナとして復号され、半角カナを含む行は区分 D (`lossy_lines`) に数えられない (§8.7)。編集して保存 → 表現不能 (半角カナ) として中止され、§8.6 の「E3 の選択肢」が出る (v0.4 は、半角カナを含む行を編集したときだけ。半角カナを含む行を編集せずに他の行を編集して保存すると、その行は元のバイト列のまま書かれる、§8.7.1)。「置換して保存」を選ぶと置換の一覧 (半角カナ → 全角カナ (`ｶﾞ` → `カ゛`)、互換文字 → 互換文字の表の文字 (`〜` U+301C → `～` U+FF5E)、他の表現不能文字 → `?`。件数は全角カナ・互換文字・`?` の順) が示され、承認すると Transaction (`Origin::System`、Undo 可) の後に保存され、一覧でキャンセルすると保存されずカーソルが最初の該当位置へ移る。「UTF-8 で保存」を選ぶと保存の成功後に文字コードが UTF-8 になる。(d) `ESC $ ( D` (JIS X 0212)、8 bit バイト、`SO` / `SI`、ISO-2022-JP-2 のエスケープを含むファイル → 不正バイトとして可視化され読み取り専用、`file.toggle_read_only` (v0.2) で解除できない、FR-111 で別の文字コードとして開き直せる。(e) チャンク境界 (64 KiB / 1 MiB) の直前でエスケープシーケンスが切れるファイル → 1 チャンクで読んだ結果と一致。(f) ANSI エスケープ (`ESC [`) を含むログ → ISO-2022-JP と判定されない。(g) `¥` / `‾` を含む UTF-8 文書を ISO-2022-JP で保存 → `ESC ( J` で書かれ、再読込で一致 (表現不能にならない)。(h) `−` (U+2212) → E1 の照合で表現不能 (U+FF0D への黙示写像)。(i) `"¥\nabc"` を ISO-2022-JP で保存 → 出力バイト列が `ESC ( J 5C ESC ( B 0A 61 62 63` (改行の前に `ESC ( B`。OQ-162 で確定した行ごとにエンコーダを閉じる方式の期待値) と一致する。(j) JIS X 0208 状態のまま改行するファイル (`1B 24 42 24 22 0A 1B 28 42`) を ISO-2022-JP を指定して開く → 0x0A が `<0x0A>` として見え (行は分かれない)、`StatefulEncodingWithInvalidBytes` で読み取り専用。`file.save_as` (ISO-2022-JP のまま)・`file.set_encoding` (ISO-2022-JP) → 名前を尋ねずに `msg.save.encoding_unsupported_iso2022` が出て、ジョブが投入されない (`MemFs` の書込回数 0、metadata の問い合わせ 0)。`file.set_encoding` で UTF-8 / Shift_JIS を選ぶ → E3 で「UTF-8 で保存」「置換して保存」が出ずキャンセルだけ。
- FR-112: Shift_JIS で保存できない文字 (絵文字) を含む文書の保存 → 中止、位置が報告される、ファイル無傷。**互換文字の置換**: 互換文字の表の文字 (U+301C・U+2016・U+2212・U+00A2・U+00A3・U+00AC・U+00A6・U+2014・U+00A5・U+203E) と絵文字を含む UTF-8 文書を Shift_JIS で保存 → E3、「置換して保存」の一覧で `counts` が `kana = 0`・`compat = 10`・`question = 1` (絵文字 1 個のとき)、承認後の保存結果を Shift_JIS で再読込すると互換文字の表の置換後の文字と `?` になる。同じ文書を ISO-2022-JP で保存すると `¥` `‾` は対象外 (`compat = 8`)。**外字**: U+E000 / U+E757 を含む文書を Shift_JIS で保存 → 表現不能にならず 0xF040 / 0xF9FC で書かれる。EUC-JP で保存 → 表現不能で、置換先は `?`。**黙示置換の検出**: `¥` (U+00A5)、`‾` (U+203E)、`−` (U+2212) を含む文書を Shift_JIS で保存 → 表現不能として報告される (0x5C 等に化けない)。半角カナを含む文書を ISO-2022-JP で保存 → 表現不能として報告され、E3 の 3 つの選択肢が出る (E6)。直接上書き (承認後・`InPlacePattern`) の保存で表現不能があると、乾式走査で検出されて `MemFs` の元ファイルへの書込回数が 0。
- 不正バイト: 先頭 64 KiB の比率が 1% 超のファイル → 本文の構築の前に `ui.confirm.open.invalid_bytes` (巨大ファイルとは別のキー) が出る (中止なら何も開かない。「文字コードを指定して開く」で別の文字コードを選ぶと `forced_encoding` 付きで先頭から読み直す)。不正バイトの確認で「読み取り専用で開く」で開いた文書の理由は `InvalidBytes` だけで、`file.toggle_read_only` で解除できない (v0.2〜v0.3。v0.4 は確認の後に解除できる、§8.3.1)。先頭 64 KiB は妥当で後半に不正バイトがあるファイルは確認なしで `InvalidBytes` の読み取り専用で開く。UTF-32 の BOM のファイル → `ConfirmAsk::UnsupportedBom` の確認が出て、「読み取り専用で開く」で開いた文書の理由は `Binary` (§9.2 の表。バイナリの確認と同じ) で、その再投入では不正バイトの比率の確認を出さない (`read_only_requested`)。「文字コードを指定して開く」で UTF-16LE を選ぶと `FF FE` を BOM として読み飛ばして開く (§8.2 手順 2)。
- 不正バイトを含む文書の編集 (v0.4 の M23、§8.3.1。01 §5.5 の NFR-304 AC-4〜AC-6): (a) **解除**: 不正バイトを含む UTF-8 / Shift_JIS / EUC-JP / UTF-16LE (奇数長・孤立サロゲート) の文書で `file.toggle_read_only` → `ui.confirm.read_only.invalid_bytes` が出て (最初の不正なバイトの位置を含む)、キャンセルでは読み取り専用のまま、`[E]` で文字コードの入力に進み、`[A]` で編集できる。ISO-2022-JP・バイナリ・巨大ファイルの読み取り専用・衝突の文書 (b) ではパレットで淡色になり、理由が示され、編集の拒否のメッセージは理由ごとの hint になる (「切替で編集できる」と言わない)。解除した文書を `file.reload` → 解除を持ち越して編集でき、再読込の Undo ができる。文字コードを指定して開き直すと読み取り専用に戻る。 (b) **保存**: 不正バイト以外を編集して上書き保存・同じ文字コードの名前を付けて保存 → 不正バイトが元のバイト値のまま同じ順で残り、確認は出ない。BOM 付きの UTF-8 / UTF-16LE の文書でも `VerifyFailed` にならない。不正バイトを含む行を `edit.move_line_up` で動かして保存 → バイトのまま移る。 (c) **つながり**: UTF-8 で `<0xE3><0x81>` と `<0x82>` を隣り合わせる、Shift_JIS で `<0x81>` の直後に `@` を入れる、Shift_JIS で `<0x81>` の後に全角文字が続く、UTF-16LE で奇数個の並びを動かす (文書の先頭)、BOM の無い UTF-8 の文書の先頭に `<0xEF><0xBB><0xBF>`・UTF-16LE の文書の先頭に `<0xFF><0xFE>` を置く (書く文字コード自身の BOM の並び) → 保存で `ui.confirm.save.escape_merge` が出て、原子的保存でも直接上書きでも書込が 0 回 (一時ファイルもバックアップも作らない。`file.save_strategy` は v0.4 でも予約のキーで core に実装が無く、製品の経路で最初のジョブが直接上書きになることは無いので、保存のジョブを直接上書きで直に投入する試験で確かめる。M23 の 3 本目)、件数・最初の位置・`details` の一覧 (「→」の右の文字)・変わる範囲 (文字数と行数、UTF-16 の先頭では「ファイルの末尾まで」) が期待どおり、確認の表示で最初の位置が画面に入り、キャンセルでカーソルが最初の位置へ移って全部の位置に強調が付き `info.save.escape_merge_cancelled` が出る、「このまま保存」で書かれたファイルを同じ文字コードで開き直すと、つながった文字として読まれ、完了のメッセージが `info.save.escape_merged`。承認の後に保存が `Busy` で失敗 → 再試行・別名で保存では確認が出ない。確認の表示中 (保存の待ちの間) に編集してから「このまま保存」→ 承認を使わず、今の内容でもう一度確認が出る。件数と先頭 100 件が同じで 101 件目より後ろのつながりの位置だけが違う文書に前の承認を渡す → 書かずにもう一度確認が出る (全件のダイジェスト)。別の文字コードの BOM の並びの不正バイトで始まるファイル (BOM の無い UTF-8 の `FF FE` など) を未編集のまま名前を付けて保存する → 確認は出ず、元のファイルとバイト単位で同じものを書く。表現不能の文字とつながりが両方ある → 先に E3 が出る。全保存では保存されず一覧に「確認が必要です」。`app.quit` → 保存 → 確認でキャンセル → 終了しない。「保存を中止して終了」の後に `EscapeMerge` が届く → 何も書かずに終了し、文書が退避される。保持方式の記録の直前にエスケープ文字を置いてつなげた Shift_JIS の文書 → `EscapeMerge` (`VerifyFailed` にならない)。記録の元のバイト列を障害注入で壊す → つながりがあっても `VerifyFailed`。記録の無い UTF-8 の文書の `VerifyFailed` (障害注入) → 保持の状態が `NotApplicable` のまま。 (d) **コピーと貼り付け** (`FakeClipboard`): コピーで `SetClipboard` に `<0x9A>` の表記が入る (Inline と Handle の両方)。同じ文書で切り取り → 貼り付けでバイトのまま (`info.paste.as_bytes`、保存でバイト一致)。別の文字コードの文書・不正バイトの無い文書・不正バイトの無い ISO-2022-JP の文書に貼ると `<0xNN>` の文字列で `info.paste.escape_as_text`。OS のクリップボードを `<0x9A>` を含む別の文字列に替えてから貼ると内部レジスタを使わず `info.paste.notation_as_text`。システムのクリップボードへの書込を失敗させた後でも、届いた文字列が内部レジスタと違えば届いた文字列が入る (端末からの貼り付けを含む)。読めなかった貼り付けは内部レジスタを使う。NUL を含む範囲を切り取って貼ると、NUL で切れた文字列が届いても内部レジスタの全体が入る。状態付きで不正バイトのある文書からのコピーを貼ると `info.paste.escape_as_text`。衝突の文書 (b) からのコピーは、衝突の文書 (a) へも `<U+10FFNN>` の文字列で `info.paste.pua_as_text`。CRLF の文書のコピーが LF で届いても照合が一致する (`editor.paste_normalize_eol = false` の場合は、キーが公開されていないので純粋な関数の性質テストで確かめる)。外から `U+10FF80` を貼る・打つ・置換の文字列に入れる → `<U+10FF80>` の 10 文字と `info.paste.pua_as_text`。衝突の文書 (a) ではそのまま入る。検索欄に内部レジスタ由来の `<0x9A>` を貼る → 文字列のまま入り `info.paste.search_notation`。 (e) **E3**: 不正バイトを含む Shift_JIS の文書に絵文字を入れて保存 → E3 に「UTF-8 で保存」が出ない。UTF-8 への `file.set_encoding` → E3 でキャンセルだけで、本文が不正なバイトの説明の文言になる。表現不能が 101 個以上あり、101 個目より後ろにだけエスケープ文字がある → 「UTF-8 で保存」も「置換して保存」も出ない (今の版の不具合の回帰の試験)。 (f) **由来の文字コード**: Shift_JIS の不正バイトを全部消して UTF-8 で保存 → Undo でエスケープ文字を戻して保存 → E3 のキャンセルだけ (元の Shift_JIS のバイトを UTF-8 のファイルに書かない)。manifest の `encoding` が Shift_JIS のスワップを、UTF-8 として開いた文書に適用 → 保存で E3 のキャンセルだけ。 (g) **衝突の文書 (b)**: 同じ文字コードの名前を付けて保存 → E3 のキャンセルだけ (私用文字の UTF-8 を書かない)。 (h) **復旧**: 解除して編集 → スワップ → 「クラッシュ」→ 開くと `InvalidBytes` で読み取り専用だが復旧が提案され、適用すると内容が一致し、読み取り専用のまま `info.swap.applied_read_only` が出る。
- 保存の連鎖 (§9.7): dirty な文書で `app.quit` → 保存 → 表現不能で E3 の「キャンセル」→ 終了しない・文書は dirty のまま。`Conflict` で「中止」→ 終了しない。保存の成功で終了する。無題の文書で保存 → 名前の入力をキャンセル → 終了しない。保存中に入力 → 保存の完了後に終了せず FR-107 の確認がもう一度出る。名前を付けて保存で既存のファイルを選ぶ → 上書きの確認 (既定は「別の名前を入力」)、`Esc` で名前の入力に戻り書かない。読み取り専用の文書で FR-107 の「保存」→ 名前の入力に進む。E3 の「置換して保存」→ 置換の ChangeSet の構築中に編集を挟む → 結果が捨てられて作り直される。構築中の `Esc` → E3 のキャンセルと同じ。`AtomicUnsupported` の確認の既定が「別名で保存」。保存中に `app.quit` (偽時計) → 5 秒以内に保存が終われば確認なしで終了し、5 秒を超えると `ui.confirm.quit_saving` が出る → 「待つ」(`Esc` も同じ) → 保存の失敗で `AfterFailure` に進み、再試行の成功で終了する。「保存を中止して終了」→ 取消しを送った後に `Saved` が届いても採用され、dirty なら退避してから終了する。後続を伴う `Saving` で `Esc` → 取消しが送られ、`Cancelled` で中止、`Saved` なら完了。単独の保存の `Saving` で `Esc` → 取り消されない。連鎖の途中の `file.save` → 無視され保存中が示される。「退避先に保存」を選ぶ → `WriteRecovery` で `recovery/rescue/` に書かれ、後続 (終了) は行われない。`state_dir` に書けない (障害注入) → 代替の場所に書かれる。読み取り専用属性のファイルを選ぶ → 上書きの確認の前に拒否。`file.set_encoding` の保存が失敗 → `TextFormat` が元のまま。開き直しのキャンセル・失敗 → 旧本文・旧 format が残る。
- 保存の重なり: 保存 A の実行中に保存 B を要求 → B は A の完了後に A の `new_stamp` を `Expect::Stamp` にして実行され、`Conflict` にならない。
- 置換途中の痕跡 (開く時の検出は v0.1): 保存中フラグが残った状態でそのパスを開く → §9.2 の「中断された保存の写像」の表の各行 (記録 × T の有無 × バックアップの実在) について、`interrupted_save` の値と MessageKey が表のとおりになり、報告しない行ではフラグのファイルだけが消える (一時ファイル・バックアップは残る)。文書は通常どおり開く。(v0.2) 起動時の一斉走査 → `.<name>.eve-bak-*` だけがある場合は復旧提案、`.eve-tmp-*` が残る場合は確認提案。
- 緊急退避の形式: CRLF / 混在改行・PUA エスケープを含む dirty 文書を退避 → `recovery/` の本文が改行を変換せず保持し、manifest の `default_eol` / `eol_mixed` / `encoding` が元と一致 (manifest のキーは 02 §11.2.1)。
- Undo 結合 (§7.3 の R1〜R9): 「abc」入力 → 1 Undo で全部消える。「ab」→ 1.5 s 待ち (偽時計) →「c」→ Undo で「ab」が残る。字下げの無い行で「ab」→ Enter →「c」→ Undo で「c」だけが消え、もう一度の Undo で「ab」と改行が消える。字下げのある行での Enter (改行 + 空白) は単独の 1 単位。改行後の入力は別単位。保存で封印。Undo 後に編集 → Redo 不可、保存点が redo 側にあれば dirty。
- 複数 View (v0.5): View A で削除 → View B の Selection / scroll が写像される。
- 検索: 編集中に古い検索結果が届く → v0.1 は破棄され、300 ms 後に再検索される (偽時計)。件数上限超で「N+」表示。長大行の正規表現検索 (v0.2) でメインスレッドが `String` を作らない。長大行を含む文書が読み取り専用で開き、表示範囲外の一致へは移動しない (v0.1)。
- Proposal (v1.L): base が古い Proposal の写像、衝突ハンクの失効、部分承認が 1 Undo 単位。
- 復旧 (v0.2): 編集 → スワップ書き出し (偽時計で 2 s 進める) → プロセス「クラッシュ」(状態破棄) → 再起動で復旧提案 → 適用で内容一致、dirty = true。
- 大きな選択範囲のコピー (01 FR-205 AC-4): 60 MB を全選択してコピー → `Effect::SetClipboard` が `ClipboardText::Handle` を運び、Host が `Editor::take_clipboard` で取り出した `ClipboardSource` をワーカーで文字列化する (メインスレッドで文字列化が起きない。取り出した後のハンドルは無効)。100 MB を全選択してコピー → 上限 (64 MiB) を文字列化の前に総バイト数で判定し、`Effect::SetClipboard` を出さずに内部レジスタだけに入れて通知する (文字列化が起きない)。

### 22.4 ベンチマーク (criterion、09 で CI 化)

- NFR-103〜111 を §21 の手段ごとに分解したマイクロベンチ: `apply` (100 MB 中央)、`layout_line` (10 MB 行の先頭 64 KiB (v0.1) / char 近似モード (v0.2) / 書記素の単位の索引の行末までの作成と差分の作り直し (v0.4、§14.4) / 通常)、`snap_to_grapheme` (チャンク跨ぎ)、decode (UTF-8 / Shift_JIS 100 MB)、encode + 書き出し、検索 全件 (100 MB、リテラル `memmem` / 行単位の正規表現 `(?i)`)、ChangeSet compose (10,000 edits)、Undo 10,000 回。
- メモリ: 1 MB / 100 MB / 1 GB 読込後の RSS (NFR-202、デコード後サイズ基準、OS 別指標)、Undo 10,000 ステップ後の履歴サイズと RSS (NFR-203、会計との比)。

### 22.5 スナップショットテスト

- 代表的なテキスト (全角・絵文字・タブ・制御文字・不正バイト・長大行の先頭・折り返し) の `Frame` をシリアライズして比較 (`eve_testkit::golden::check`、09 §3.4)。WidthConfig の `ambiguous_wide` 両方を網羅。

---

## 23. 未決事項

本書に関わる OQ / ADR。**番号は [10 §2](10-decisions.md#2-未決事項-open-questions) の統一 OQ 番号**。状態・区分 (PO 決定 / 裁量 / スパイク) と詳細は 10 を正とし、本表は本書での結論 (確定していないものは「未決」と記し、推奨案) を示す。

| OQ | 件名 | 結論 | 区分 / 影響 |
|---|---|---|---|
| OQ-32 | 「桁」の単位 (FR-102 / FR-503) | **書記素数 (1 始まり) で確定 (ユーザー回答)**。設定で表示列に切替 (キーの公開は v0.2、07 §5) | 確定 / 03 §3.2 C4, 04, 07 |
| OQ-45 | Backspace の単位 | **v0.1 は書記素単位に固定** (FR-201、§13.1)。コードポイント単位への切替は v0.2 で設定 `editor.backspace_unit` (07 §5.13、既定は書記素) として公開する | 確定 / 01 FR-201, 03 §13.1, 07 §5.13 |
| OQ-46 | 単語移動の停止位置 | 既定は **次の単語の先頭** (Windows 慣習)。現在の単語の末尾 (macOS/Unix 慣習) は v0.2 で設定として公開 (`editor.word_motion`、07 §5.13) | 確定 (ユーザー回答) / 03 §5.3, 07 §5.13 |
| OQ-47 | 読込時の文字コード往復照合 (§8.7) | **UTF-8 以外で常に行う (ストリーミング、サイズ制限なし)**。v0.1 は件数 + 先頭 100 箇所の記録で保存時の確認 (E2') に使い、v0.4 で保持方式の素材にする | 確定 (ユーザー回答) / 読込時間 (08 §3 S-01 で実測) |
| OQ-48 | スワップファイルの形式 | 確定 (ユーザー回答、2026-10-28): 緊急退避と同じ (UTF-8 本文 + 版付き manifest。swap-lite、§10.2)。v0.1 で確定する manifest 形式 (02 §11.2.1) を共用し、書き手も再利用 | 裁量 / 03 §10.2, 05 §7.5 (配置・命名は 05) |
| OQ-49 | Undo 結合の時間閾値 (`editor.undo.group_timeout_ms`) | **1000 ms** (07 §5.1 の既定値) | 確定 (ユーザー回答) / 03 §7.3, 07 §5.1 |
| OQ-50 | Undo 結合の単語境界での封印 | 既定 **OFF** (連続入力をまとめる)。設定公開は v0.2 (`editor.undo.break_on_word`、07 §5.13) | 確定 (ユーザー回答) / 03 §7.3 |
| OQ-51 | 貼り付け時の改行正規化 (§8.4) | **`default_eol` に正規化、で確定 (ユーザー回答)**。v0.1 は常に ON、無効化の設定キー `editor.paste_normalize_eol` の公開は v0.2 (07 §5.13) | 確定 (ユーザー回答) / 03 §8.4, 04 §6.6, 05 §8.2 |
| OQ-52 | 混在改行の Mixed 表示方法と「揃える」提案の有無 | **表示のみ** (揃える提案はしない) | 確定 (ユーザー回答) / 04 |
| OQ-05 | 巨大ファイルの警告閾値と利用可能メモリの取得 | **512 MiB 超で開く前に警告する固定閾値、で確定 (ユーザー回答)** (`file.huge_file_warn_bytes`、§11.2)。利用可能メモリへの連動は v0.5 以降 (02 §4.4 に `SystemInfo` を予約) | 確定 / 02 §4.4, 05, 07 |
| OQ-53 | 書記素境界判定のチャンク跨ぎ方式 | 未決 (08 §3 S-02 で判断)。推奨: `GraphemeCursor` + PreContext、不可なら 256 バイト窓バッファ。国旗列の偶奇はどちらの方式でも既知の制約 | スパイク / 03 §13.1 |
| OQ-54 | 正規化 (NFC/NFD) 非依存の検索 | 未決。推奨: **v0.x では非対応**、要望があれば Could | 裁量 / 03 §13.5 |
| OQ-39 | BiDi を 00 の非目標に追加するか | **追加 (NG11)** | 確定 (ユーザー回答) / 00 §3 |
| OQ-55 | 検索結果上限値 | **保持 10,000、カウント 1,000,000** (07 `search.count_limit` の既定値) | 確定 (ユーザー回答) / 03 §12.2, 07 §5.5 |
| OQ-56 | 複数カーソルでの貼り付け分配規則 | **確定 (推奨案、2026-11-20。v0.5-plan §1.1)**: **行数一致なら分配**。細目は M32 の仕様の PR (2026-11-22) で §4.5.3 に決めた (内部レジスタの片 → 行 → 全文の 3 段、コピーは空でない Range を文書の既定の改行で連結) | 回答済み / 03 §4.5.3 (v0.5) |
| OQ-57 | 折り返しの禁則処理の既定 | 未決。推奨: **ON** (最小集合) | 裁量 / 03 §14.3 |
| OQ-58 | マウスクリックで全角セルの右半分を押したときの位置 | 未決。推奨: **文字の前** (`Bias::Before`、§5.8) | 裁量 / 04 |
| OQ-59 | `Origin::Undo` を独立した Origin にするか、元 Origin + フラグか | **元 Origin + `is_undo` フラグ** (監査で元の主体が分かる。02 §5.2 で確定) | 確定 (ユーザー回答) / 03 §7.1, 02 §5.2 |
| OQ-60 | 複数行の正規表現を rope 上でどう実行するか (v0.4) | **v0.2 は行単位の実行で複数行は未対応** (§12.3。v0.1 は正規表現を利用者に出さない)。**v0.4 の実行方式は「行の窓」に確定 (S-04 の実測、2026-11-11。§12.3、10 の OQ-60)**。regex-cursor は採らない。合否の基準の正本は 08 §3 の S-04 の行 (全文連結の `regex` との差分 fuzz で不一致 0、行単位の方式 (BM-109r)・全文連結の方式のそれぞれの 2 倍以内、依存の条件) で、行の窓は 3 項目とも満たした。意味の細目 (複数行の実行に入る条件など) は §12.3.1 (M24 の仕様の PR で案。利用者に見える点は OQ-218、確定 (ユーザー回答、2026-11-12)) | スパイク (S-04、v0.4) / 03 §12.3 / 確定 (2026-11-11) |
| OQ-41 | CP932 互換文字 (〜/～、−/－、‖/∥、¢/￠ 等) を検索で同一視するか | **確定 (ユーザー回答、2026-11-11)**: 同一視オプション `search.cp932_compat_fold`、既定 ON (**v0.4**)。保存データは変えない。対象の組は §8.6 の互換文字の表 (OQ-208) と同じ。細部は §12.4.1 (M24 の仕様の PR で案。利用者に見える点は OQ-218、確定 (ユーザー回答、2026-11-12)) | PO / 03 §8.7 / §12.4, 07 §5.5, 09 §4 |
| OQ-218 | 検索の拡張の細目 (v0.4 の M24。OQ-60 / 41 の細目) | **確定 (ユーザー回答、2026-11-12)**: 8 点とも推す案のとおり。8 点: 行の終端に一致する書き方、上限の無いパターン (16 MiB)、次 / 前 (厳密)、`\A` / `\z`、同一視の正規表現・置換・表示・`¥` / `‾`。本書はその内容 (§12.3.1 / §12.4.1)。CRLF を分けて一致させない規則は正しさの規則で、論点に含めない | PO / §12.3.1 / §12.4.1, 10 OQ-218 |
| OQ-42 | v0.1 のデータ保全の穴 (SIGKILL / OOM / 電源断 / CTRL_CLOSE 不達) の扱い | **リスク受容を明記し、FR-118 (swap-lite) を v0.2 の最優先にする、で確定 (ユーザー回答)**。固定閾値警告と不正バイト比率の警告 (§11.2)、`try_reserve` を Must の前提とし、リリースノートに既知の制約として明記する | 確定 / 01 FR-118, 10, 03 §10 |
| **OQ-139** | 状態付き符号化 (ISO-2022-JP) で不正バイトを含む文書を常に読み取り専用にするか | **読み取り専用で固定、で確定 (ユーザー回答)** (OQ-207 に統合。FR-111 で UTF-8 として開き直せば編集可。02 §13.4 (f) と同じ)。v0.4 の M23 (§8.3.1) でも `StatefulEncodingWithInvalidBytes` は解除できない | 確定 / 03 §8.3, 02 §13.4, OQ-16 / 207 |
| **OQ-217** | 不正バイトを含む文書の編集の細目 (v0.4 の M23) | **確定 (ユーザー回答、2026-11-12)**: §8.3.1 のとおり (10 の OQ-217 の一覧が論点の正本)。解除は `file.toggle_read_only` と確認 (既定は編集を許可)、同じ文書の再読込では解除を持ち越す。システムのクリップボードには `<0xNN>`、内部レジスタとの照合で同じ `escape_encoding` の文書にだけバイトのまま運び、外からの私用文字は `<U+10FFNN>` にして知らせる。つながりは書く前の確認 (既定はキャンセル、承認は結果で照らし合わせる)。エスケープ文字の前後を書記素の境界にする | PO / 03 §8.3.1 / §9.6 / §9.7 / §10.3 / §12.6 / §13.1, 02 §9.3, 04 §15.2, 07 §7.3 |
| — | クリップボードの書き出しの失敗の向き (取得 / 書き出し) と世代の番号を Host が報告に添え、書き出しの失敗の後の貼り付けで内部レジスタを使うか (§8.3.1) | 後の版の候補。v0.4 は書き出しの失敗を知らせるだけで、照合の規則は変えない (今の `HostReport::ClipboardFallback` の理由は取得か書き出しかを持たず、非同期の書き出しの失敗がどのコピーのものかも分からないため。M23 の 5 本目のレビューで、印を付ける案は取りやめた) | 候補 / 03 §8.3.1, 02 §5.4, 05 §8.1 |
| — | 内部レジスタとシステムのクリップボードの照合を、OS のクリップボードの変更番号 (Windows の `GetClipboardSequenceNumber`、macOS の `changeCount` 等) で確かめるか (§8.3.1) | 後の版の候補。v0.4 は文字列の照合だけで判定する (他のアプリで同じ文字列をコピーし直したときも内部のコピーとみなす)。変更番号は OS ごとに取れるものが違い、Port (05 §8) の型を足すことになる | 候補 / 03 §8.3.1, 05 §8 |
| OQ-31 | NFR-106 (長大行) の優先度 | **Must のまま、判定を「閾値超は char 近似モード、10 MB 行で各操作 p99 ≤ 100 ms」とする、で確定 (ユーザー回答)** (§11.5)。判定値は char 近似モードを入れる v0.2 から適用し、v0.1 は閾値超の行を含む文書を読み取り専用で表示する縮退で固まらないことだけを満たす (確定 (ユーザー回答))。50 ms と書記素単位の正確さは v0.4 の書記素の単位の列の索引 (§14.4) で | 確定 / 01 NFR-106, 03 §11.5 / §14.4 |
| **OQ-116** | 無編集の上書き保存を書き込まないか (§9.3 手順 0) | **書き込まない、で確定 (ユーザー回答)** (dirty でなくディスクも `disk_stamp` と一致するとき。「変更はありません」と表示)。無編集保存でレガシー文字コードのバイト列が変わる事故を 1 つの条件で防げる。ISO-2022-JP のエスケープ選択 (§8.7 区分 D) も同じ条件で守られる | 確定 / 03 §9.3, 01 FR-104 / NFR-303 |
| **OQ-114** | Undo の内部構造 (§7.1) | **線形 2 スタック、で確定 (ユーザー回答)**。履歴は永続化せず置換は crate 内で閉じる。木は FR-213 で | 確定 / 03 §7.1 |
| **OQ-03** | v0.1 の日本語レガシー文字コードの範囲 | **Shift_JIS / EUC-JP / ISO-2022-JP をすべて読み書き、で確定 (ユーザー回答)**。ISO-2022-JP の保存は §8.6 E6 (行ごとにエンコーダを閉じて正規形で書く。OQ-162 で確定)、エスケープ選択の差は区分 D として行単位で検出し確認 (§8.7 区分 D。v0.4 は保持方式、§8.7.1) | 確定 / 03 §8, 01 FR-109 / NFR-303, 02 §13.4, 08 S-01 (a') |
| **OQ-138** | ISO-2022-JP で WHATWG (encoding_rs) のデコーダが受け付けない入力 (JIS X 0212 `ESC $ ( D`、ISO-2022-JP-2 系、`SO` / `SI`、出力を挟まない連続エスケープ、8 bit バイト) の扱い (§8.2 手順 4' / §8.3 / §8.7) | **v0.1 は不正バイトとして読み取り専用で開く** (FR-111 で UTF-8 等として開き直す。encoding_rs だけで閉じ、`eve-encoding` の依存と自前の表を増やさない)。v0.2 以降は未決: S-01 (a') で実コーパス中の「読み取り専用になるファイルの比率」を記録し、有意 (目安 5% 超) ならデコーダのみ自前の寛容実装 (JIS X 0208 / 0212 / 半角カナの表は encoding_rs の EUC-JP デコーダへ `バイト \| 0x80` で写して流用) を追加し、受け付けた箇所を区分 B に数える | 裁量 (S-01 の結果で判断) / v0.2 着手前 / 03 §8.2 / §8.3 / §8.7, 08 S-01 |
| **OQ-162** | ISO-2022-JP の保存で各行を ASCII 状態で終える方式と、S-01 不合格時の扱い (§8.6 E6) | **行ごとにエンコーダを `last = true` で閉じてから改行を付ける、で確定 (ユーザー回答)** (encoding_rs の公開 API だけで実装でき、E1 の照合と両立する)。S-01 不合格時: 照合が成り立たない文字コードは v0.1 では読込のみ (別の文字コードでの保存は可、リリースは止めない)、Shift_JIS / EUC-JP が不成立ならリリースを止める | 確定 / 03 §8.6 / §8.7, 01 FR-109, 02 §13.4, 08 S-01 (a') |
| **ADR-0031** (安定性の簡略化) | 設計者の裁量の項目 (ADR-0031 に収録): `InsertText::Slice` の閾値 64 KiB (§6.5)、プロセス全体の Undo 上限 256 MiB (§7.5)、固定 2 スレッド io / bg (§17.3)、正規表現の行単位実行 (§12.3)、単語境界の文字種遷移規則 (§13.3)、保持方式の記録の上限 (件数 100 万・元のバイト列の合計 16 MiB、§8.7.1)、クリップボードの Snapshot 受け渡し (§12.6) | **本書の記述どおり** (ADR-0031 は確定 (ユーザー回答)。細目は本書が正本) | 確定 / 03, 05, 07 |
| (後の版の候補) | 復旧で保持方式の記録を生かす (§8.7.1 / §10.3) | v0.4 では採らない (2026-11-11、PO の回答: 緊急退避・スワップに元のバイト列を残さない)。復旧を「前後の共通部分を残して差し替える」形にして、変わらなかった範囲の記録を残す案を、後の版の候補とする | 候補 / 03 §8.7.1, §10.3 |
| (後の版の候補) | 隣り合う区分 B の文字の並びの記録 (1 件にまとまる) を、並びの内側への挿入で分けて残す (§8.7.1 の「編集での写像」) | 未決。v0.4 は内側への挿入を並びの全体の編集と扱う (並びの全部が代表バイト列になる。M22 の 6 のレビューで決定) | 候補 / 03 §8.7.1 |
| **OQ-110** | ソフトラップの時期 (§14.3) | **v0.2、で確定 (ユーザー回答)**。レイアウト・上下移動・禁則処理がまとめて入るため v0.2 とする。型と設定キーは v0.1 で予約 | 確定 / 01 FR-506, 03 §14.3, 04 §10 |
