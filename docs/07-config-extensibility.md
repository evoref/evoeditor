# 07. 設定と拡張性 (Config & Extensibility)

| 項目 | 内容 |
|---|---|
| 文書ID | 07-config-extensibility |
| 状態 | ドラフト |
| 主対象 crate | `eve-config` (+ `eve-core` の `CommandRegistry` 公開面、`evoeditor` bin の引数解析) |
| 前提 | [00-overview](00-overview.md) の用語・原則 / [01-requirements](01-requirements.md) / [02-architecture](02-architecture.md) |
| 決定記録 | [10-decisions](10-decisions.md) ([ADR-0007](10-decisions.md#adr-0007-コマンド駆動設計) / [ADR-0011](10-decisions.md#adr-0011-設定ファイル形式) / [ADR-0014](10-decisions.md#adr-0014-拡張プラグイン機構の方針) / [ADR-0018](10-decisions.md#adr-0018-ライセンスと依存ライセンス方針) / [OQ-12](10-decisions.md#oq-12-プラグイン方式) / [OQ-21](10-decisions.md#oq-21-ui-文言の国際化) / [OQ-22](10-decisions.md#oq-22-tree-sitter-文法の同梱方式) / [OQ-24](10-decisions.md#oq-24-cli-引数パーサ) / [ADR-0042](10-decisions.md#adr-0042-v01-の最小核-案-lwindows-のみの出荷日本語のみの-ui) (v0.1 の範囲) / [ADR-0043](10-decisions.md#adr-0043-v01-の範囲に-filereloadfileset_line_endingリテラルのすべて置換全角スペースタブの最小可視化を含める) (v0.1 に含めた 4 機能) / [ADR-0044](10-decisions.md#adr-0044-装飾記号枠線の-ascii-縮退) (装飾の ASCII 縮退)) |
| v0.1 で読む節 | §2〜§4、§5.0 (v0.1 の公開キー) と §5 の各表のうち v0.1 の行、§5.13、§7、§8、§11 (特に §11.6 カタログ)、§17、§18 (一覧の正本は [README §3.1](README.md#31-v01-の実装で読む節)) |

> 本書のコード片・設定例は **仕様説明のための例** であり、キー名・既定値は「提案中」。ただし M0d (08 §4) で確定する v0.1 の CommandId カタログ (§11.6)・v0.1 の公開キー (§5.0)・CLI (§18.3) と、確定済みの OQ・ADR を根拠に書いた項目 (OQ-27 の既定テーマ、OQ-125 の行強調、OQ-05 / 31 / 32 の巨大ファイル・長大行・桁の単位、OQ-28 の終了コード、OQ-34 / 73 の直接上書き・ハードリンク、OQ-111 のクリップボード方式、ADR-0039 の v0.1 の範囲、OQ-04 / 51 の改行、OQ-26 / 29 / 38 のキーマップ解決・`mod`・パレット表示 (ADR-0017 / 0022)、OQ-37 の FR-701、OQ-46 / 49 / 55 / 71 / 117 の既定値と規則、OQ-36 (b) の平文キーの廃止、OQ-132 / 135 の補完の送信範囲、ADR-0025 のカタログの正本) は除く。確定状態は 10-decisions を正とする。
> 名称は [OQ-01](10-decisions.md#oq-01-正式名称とバイナリ名) で確定 (ユーザー回答): バイナリ名は `evoeditor`。ワークスペース設定ディレクトリ `.eve/`、crate の接頭辞 `eve-`、**環境変数の接頭辞 `EVE_`** (`EVE_LOG` / `EVE_CONFIG_DIR` / `EVE_STATE_DIR` 等) は変えない (短い名前を保つ。バイナリ名に合わせて `EVOEDITOR_` にはしない)。
> **本書は CommandId (§11.6 カタログ) と設定キー・既定値 (§5) の正本** である (02 §12.1 / ADR-0025、確定)。03〜06 はここから引用するだけで、独自の ID・キー名を定義しない。

---

## 目次

1. [目的と責務](#1-目的と責務)
2. [設定ファイル形式](#2-設定ファイル形式)
3. [設定の階層とマージ規則](#3-設定の階層とマージ規則)
4. [スキーマ・検証・エラー表示](#4-スキーマ検証エラー表示)
5. [設定項目一覧 (スキーマと既定値)](#5-設定項目一覧-スキーマと既定値)
6. [再読込 (ホットリロード)](#6-再読込-ホットリロード)
7. [キーマップ](#7-キーマップ)
8. [テーマ](#8-テーマ)
9. [言語設定](#9-言語設定)
10. [EditorConfig 対応](#10-editorconfig-対応)
11. [コマンド体系](#11-コマンド体系)
12. [ワークスペースと信頼モデル](#12-ワークスペースと信頼モデル)
13. [拡張機構の段階的方針](#13-拡張機構の段階的方針)
14. [拡張 API 設計原則](#14-拡張-api-設計原則)
15. [LSP / DAP / フォーマッタ / リンタ統合方針](#15-lsp--dap--フォーマッタ--リンタ統合方針)
16. [セッション復元 (FR-121, v0.4 Could)](#16-セッション復元-fr-121-v04-could)
17. [国際化 (UI 文言)](#17-国際化-ui-文言)
18. [CLI 仕様](#18-cli-仕様)
19. [サードパーティ拡張のライセンスと信頼](#19-サードパーティ拡張のライセンスと信頼)
20. [未決事項](#20-未決事項)

---

## 1. 目的と責務

### 1.1 本書が決めること ([02 §12.1](02-architecture.md#121-文書と-crate-の対応))

| 領域 | 内容 | 関連要件 |
|---|---|---|
| 設定 | ファイル形式、場所、階層とマージ規則、全設定項目のスキーマと既定値、検証とエラー表示、再読込 | FR-701〜706, NFR-405 |
| キーマップ | キー表記、コード (複数ストローク)、`when` 句の文法、層の優先順位 | FR-604, 606, 608 |
| テーマ | ファイル形式、スコープ名、継承、色の表現 | FR-513, 707, NFR-604 |
| 言語 | 言語定義 (拡張子→言語、インデント、コメント、文法、LSP、フォーマッタ)、初期対応言語の確定 | FR-704, 802 |
| EditorConfig | 対応プロパティ、優先順位 | FR-705 |
| コマンド | ID 命名規約、引数スキーマ、パレット登録、ヘルプ自動生成 | FR-601, 602, 514 |
| ワークスペース | 概念、ルート検出、ワークスペース設定、信頼モデル | FR-705, NFR-502 |
| 拡張 | 拡張機構の段階的方針、プラグインプロトコルの骨子、拡張 API の安定性・権限 | FR-708, NFR-508 |
| 統合 | LSP / フォーマッタ / リンタの設定上の扱いと時期 | FR-709, 804 |
| セッション | 復元対象と保存場所の方針 | FR-121 |
| 国際化 | UI 文言のキー化 (MessageKey)、言語ファイルの形式 (v0.1 は `ja` のみ。英語 UI と言語の選択は v0.2) | NFR-603 |
| CLI | 引数、終了コード、標準入力、`$EDITOR` 利用時の挙動 | FR-101, 102, 120 |

### 1.2 本書が決めないこと

| 内容 | 責務先 |
|---|---|
| 既定キーの割当表 (どのキーに何を割り当てるか)、端末で区別できないキーの代替割当 | 04-tui-design |
| 設定・状態・キャッシュディレクトリの OS 別の実パス、ファイル監視の実装 | 05-platform |
| AI 固有の設定項目の **意味** (書式と置き場所は本書 §5.12) | 06-ai-integration |
| LSP クライアントのプロトコル実装、位置変換 | eve-lsp (03 / 後続) |
| プラグイン機構の最終選択 | [OQ-12](10-decisions.md#oq-12-プラグイン方式) (本書は推奨と段階方針を示す) |

### 1.3 `eve-config` の責務境界 (再確認)

[02 §4.1](02-architecture.md#41-crate-一覧) より、`eve-config` は **ファイル I/O を行わない** (設定ファイルの内容文字列と「どの層のどのパスか」を受け取る)。読込の順序決定・ディレクトリ解決・ファイル監視は `evoeditor` bin と `eve-platform` (`Paths` / `FileSystem` / `FileWatcher` Port) が行い、`eve-config` は **純粋関数** として「複数層の文字列 → `ResolvedConfig` + 診断 (警告・エラー) の列」を返す。これにより (1) ヘッドレステスト (NFR-701) で設定を文字列から直接構築でき、(2) 起動経路から I/O 以外の遅延要因を排除できる (NFR-101)。

```
                      ┌── eve-platform (Paths / FileSystem) ── 層ごとの文字列を集める
evoeditor (bin) ───────┤
                      └── eve-config::load(layers: &[(LayerKind, Source, &str)])
                               -> (ResolvedConfig, Vec<ConfigDiagnostic>)
                                        │                    │
                        Arc<ResolvedConfig> を core へ    Message として UI に表示 (FR-703)
```

### 1.4 用語 (本書で導入)

| 用語 | 定義 |
|---|---|
| 設定層 (layer) | 既定 / システム / ユーザー / ワークスペース / コマンドライン / セッション の各段階。§3 |
| `ResolvedConfig` | 全層をマージ・検証した後の型付き設定値。core が参照する唯一の設定表現 ([02 §12.2](02-architecture.md#122-境界をまたぐ型の所有者)) |
| 設定診断 (`ConfigDiagnostic`) | 設定読込時の警告・エラー 1 件 (層・ファイル・行・キー・メッセージ・適用した既定値) |
| 安全な設定項目 | ワークスペースを「信頼」していなくても適用できる項目 (インデント幅等)。§12.3 |
| 信頼 (trust) | ユーザーが特定ワークスペースの設定にプロセス起動を伴う項目の適用を許可した状態 |
| プラグイン | v1.X で導入する外部拡張。本書では「拡張点 (内部 API)」と区別して用いる |
| プラグイン ID | `ext.<plugin_id>.*` の名前空間を与える識別子。§13.2 |
| MessageKey | UI 文言の識別子 ([02 §6.1](02-architecture.md#61-基本方針-adr-0007))。§17 |


---

## 2. 設定ファイル形式

### 2.1 決定 (ADR-0011 の具体化)

- **すべての設定ファイル (設定本体・キーマップ・テーマ・言語定義・ロケール) は TOML 1.0** とする ([ADR-0011](10-decisions.md#adr-0011-設定ファイル形式))。1 形式に統一して学習コストを最小化する。TOML は Cargo と同じ形式で既知、コメントを書ける、型 (日付・整数/浮動) が明確、純 Rust の `toml` / `toml_edit` で読込とコメント保持の書き戻しができ、データのみで実行を伴わない。
- 読込は `serde` + `toml`。**書き戻し** (設定変更コマンド `config.set` や「このキーを割り当てる」UI による保存、v0.4 Could) は `toml_edit` でコメント・並び順を保持する。書き戻しは必ず **ユーザー層のファイルにのみ** 行い、ワークスペース・システム層は書き換えない。05 §7.1 の「`config_dir` はユーザーが設定を書くまで作らない」方針に従い、**書き戻し時に初めて `config_dir` を作成** する (`create_dir_all_private`、ジョブ内)。
- 設定ファイルの文字コードは UTF-8 固定 (BOM は許容して無視)。
- **設定は宣言的データのみ**。設定を読む行為がプロセス起動・ネットワーク・コード実行を引き起こしてはならない (NFR-502 / v0.4)。プロセス起動を伴う項目 (LSP、フォーマッタ、プラグイン) は「宣言」であり、実行は別の承認・信頼 (§12.3) を経る。

### 2.2 TOML の弱点への対処

| 弱点 | 対処 |
|---|---|
| 深いネストが冗長 | 設定階層を最大 3 段 (`[section.sub]` + キー) に抑える。言語・テーマなど繰り返し構造は `[[language]]` 配列テーブル、またはファイル分割 (`languages/<name>.toml`) |
| 条件分岐が書けない (OS 別、端末別) | 固定の条件付きセクション `[os.windows]` 等をマージ規則で定義 (§3.4)。汎用の式言語は導入しない |
| 配列のマージが直感に反する | 配列は「置換」を既定とし、キーマップと言語定義だけは「キーによる併合」(§3.3) |
| 大きなキーマップが見づらい | キーマップはインラインテーブル `"ctrl+s" = "file.save"` の 1 行表記を基本形にする (§7) |

---

## 3. 設定の階層とマージ規則

### 3.1 階層 (優先度の低い順)

| # | 層 | 場所 (OS 別の実パスは 05) | 用途 | 書込主体 | 版 |
|---|---|---|---|---|---|
| L0 | **組込み既定値** | バイナリ内 (Rust の既定値。参照用に `evoeditor --dump-config --defaults` で出力可) | 設定ファイルなしで完全動作 (FR-702) | 開発者 | v0.1 |
| L1 | システム | `/etc/evoeditor/config.toml` (Unix) / `%ProgramData%\evoeditor\config.toml` (Windows) | 管理者による組織共通の **既定値** (例: 新規ファイルの改行コード・文字コードの組織の標準)。L2 (ユーザー層) より優先度が低いので、利用者が上書きできる (値を固定する手段ではない) | 管理者 | v0.4 (Could) |
| L2 | **ユーザー** | `<config_dir>/config.toml` (v0.1) + `keymap.toml` (v0.2) + `themes/` (v0.2) + `languages/` (v0.4) + `locales/` (v0.2 以降の候補) | 個人設定。OS 間で共有可能 (NFR-405) | ユーザー / 設定 UI | v0.1 (FR-701 Must、OQ-37) |
| L3 | **ワークスペース** | `<workspace_root>/.eve/config.toml` (+ `.editorconfig`、§10)。**ディレクトリの所有者が自分でない、または world-writable なら層ごと無視** (§12.1) | プロジェクト固有 (インデント、LSP、除外パターン) | プロジェクト (半信頼) | v0.4 (FR-705) |
| L4 | **コマンドライン** | v0.1: 個別オプション (`--readonly`、`--encoding`、§18.3) だけ。v0.2: `--config <file>` / `--no-config` / `--set <key>=<value>`。v0.4: `--theme` 等の短縮形 | 一時的な上書き、テスト、スクリプト | 呼び出し元 | v0.1 (個別オプション) / v0.2 (`--config`・`--set`) |
| L5 | セッション (実行時) | メモリ上のみ (`config.set --session`) | 「今だけ折り返しを切る」等。保存しない | ユーザー操作 | v0.4 |

**v0.1 の層は L0 + L2 (`config.toml` 1 ファイル) + L4 (個別オプション) だけ** (ADR-0028)。マージ規則 (§3.3)・検証 (§4) は v0.1 から実装するが、対象は §5.0 の公開キー (7 個) に限る。条件付きセクション (§3.4) は v0.2 (書式は v0.1 で予約)。v0.1 は `[keys]` を読まず、`[keys]` / `[keys.*]` (ユーザーのキー割当) は「このバージョンでは未対応」の情報診断にする (ADR-0039、§7.1)。`keys.preset`・`keymap.toml` と `[keys.*]` の読込・`themes/` の読込は v0.2 (書式は v0.1 で確定)、ワークスペース層・言語別は v0.4。テストで設定ファイルを読ませないときは `EVE_CONFIG_DIR` を空のディレクトリに向ける (`--no-config` は v0.2)。

`<config_dir>` は `Paths` Port が解決する。OS 別の実パスの正本は [05 §7.1](05-platform.md#71-決定-xdg-準拠-macos-含む--windows-は-known-folders)。環境変数 `EVE_CONFIG_DIR` が設定されていればそれを最優先する (ポータブル運用・テスト用)。

### 3.2 ファイル分割

ユーザー層は複数ファイルに分けられる。分割は任意で、すべて `config.toml` に書いてもよい。

| ファイル | 内容 | 読込タイミング |
|---|---|---|
| `config.toml` | 本体。`[keys]` / `[theme]` / `[[language]]` を含んでもよい | 起動時 (同期、NFR-101 の予算 2 ms 以内)。1 MiB を超えるファイルは読まずに診断を 1 件出して無視する案と、`io` ワーカーで読んでメインは 200 ms だけ待つ案がある (提案、02 §9.5 手順 2、OQ-190) |
| `keymap.toml` | `[keys.*]` のみ。`config.toml` の `[keys]` より後に併合 (v0.2) | 起動時 |
| `themes/<name>.toml` | テーマ 1 つ/ファイル。`theme.name` で選ばれたものだけ読む | 起動時 (選択中のみ)、切替時 |
| `languages/<name>.toml` | 言語定義 1 つ/ファイル (組込み定義への上書き・追加) | 起動時は索引 (拡張子表) のみ、本体は該当言語の文書を開いたとき (遅延) |
| `locales/<lang>.toml` | UI 文言の上書き・追加言語 (v0.2 以降の候補、提案中。同梱の `ja` は v0.1 からバイナリ内にあり、このディレクトリは要らない) | 起動時 (選択した言語のみ) |

### 3.3 マージ規則

層 L0 → L5 の順に **深いマージ (deep merge)** を適用する。

| 値の種類 | 規則 | 例 |
|---|---|---|
| スカラー (文字列・数値・真偽・日付) | 上位層が置換 | `editor.tab_width = 4` → `2` |
| テーブル | キーごとに再帰的にマージ (下位層のキーは残る) | `[editor]` の一部キーだけ上書き |
| 配列 (一般) | **上位層が丸ごと置換** (結合しない) | `workspace.exclude = [...]` |
| 配列に `+` 接尾辞キー | 下位層の配列に **追加** | `workspace.exclude+ = ["*.log"]` |
| キーマップ (`[keys.<context>]`) | キーごとに併合。同じキーは上位層が置換。`"none"` で解除。ただし **ワークスペース層は追加のみ** で、下位層に割当のあるキーを置換・解除できない (信頼後も。02 §7.2 K5) | §7.4 |
| 言語定義 (`[[language]]`) | `name` をキーとして併合。同名の定義はフィールド単位で深いマージ。`file_types` 等の配列は置換 (`+` で追加) | §9 |
| テーマ | `inherits` チェーン → `[palette]` → スコープの順で解決 (§8.3)。層間のマージは「選択されたテーマ名」の置換のみ | — |
| `null` 相当 (キーの削除) | TOML に null がないため、特別値 `"default"` で既定値に戻す (型が文字列の項目では `"${default}"`) | `theme.name = "default"` |

マージ結果に対して検証 (§4) を行う。検証で不正だった **項目のみ** 既定値 (正確には「その項目を含まない下位層の値」) に戻し、ファイル全体を捨てない (FR-703)。ただし **TOML 構文エラー** の場合はそのファイル全体を無視し、該当層だけが欠けた状態で起動する (他の層は生きる)。

### 3.4 条件付きセクション (NFR-405)

OS 間で 1 つの設定ファイルを共有するため、固定の条件キーを持つセクションを定義する。条件付きセクションは、同じ層内で無条件セクションの **後** にマージされる。**解決は v0.2 から** (v0.1 は Windows だけを出荷するので要らない)。v0.1 は書式と名前を予約し、`[os.*]` / `[env.*]` / `[frontend.*]` / `[host.*]` を書くと「このバージョンでは未対応」の情報診断を出してそのセクションを無視する。

```toml
[editor]
tab_width = 4

[os.windows.editor]           # Windows のみ
line_ending = "crlf"

[os.macos.keys.editor]        # macOS では super (cmd) 系も追加
"super+s" = "file.save"

[env.ssh.clipboard]           # SSH セッション (SSH_TTY あり) では OSC 52 を優先 ([clipboard] の公開は v0.2)
provider = "osc52"
```

| 条件キー | 値 | 判定主体 |
|---|---|---|
| `os.<name>` | `windows` / `macos` / `linux` / `unix` (macos + linux + その他 Unix) | bin (コンパイル時定数) |
| `frontend.<name>` | `tui` | 起動した frontend。値は `tui` のみ |
| `env.<name>` | `ssh` (SSH_TTY / SSH_CONNECTION あり) / `tmux` / `wsl` / `ci` | bin が環境変数等から判定 (詳細は 05)。v0.2 で解決するのは `ssh` / `tmux` / `wsl`。`ci` は名前の予約のみで、解決は最初の利用者の版 |
| `host.<hostname>` | ホスト名完全一致 (小文字) | bin。マシン別の調整用 (Could, v0.4)。それまでは書式・名前の予約のみ |

条件は **入れ子にしない** (`[os.windows.env.ssh]` は不可)。複数条件が必要なら `[env.ssh]` の内側に `[env.ssh.os.windows]` ではなく、それぞれ別に書いて重ねる。評価順は `os` → `frontend` → `env` → `host` (後勝ち)。設定全体に汎用の `when` 式は導入しない (条件は固定キーに限り、`--dump-config` で値を一意に出せるようにする)。

**`[os.*]` 等の条件付きセクションと `when` 句 (§7.3) の使い分け**: 条件付きセクションは **設定値の切替** (起動時に 1 回評価され、`--dump-config` で一意に出せる)。`when` はキーマップ・コマンドの **実行時条件** (フォーカス・選択状態・モード等、刻々と変わるもの)。`when` にも `os` / `frontend` 変数があるが、これは「同じキーを OS で別コマンドにしたい」場合のためで、設定値の切替には条件付きセクションを使う。

### 3.5 層の起源の追跡

`ResolvedConfig` は各項目について **どの層・どのファイル・何行目から来たか** を保持する (`config.show` コマンドで `editor.tab_width = 2  (workspace: .eve/config.toml:3)` のように表示)。初心者が「なぜこの挙動か」を辿るため (P7) と、ワークスペース設定の信頼判定 (§12.3) のため。

---

## 4. スキーマ・検証・エラー表示

### 4.1 スキーマの正の情報源

スキーマは **Rust の型 (`ResolvedConfig` とその部分構造体、serde derive)** を正とし、以下を同じ定義から自動生成する。手書きの二重管理をしない。

| 生成物 | 用途 | 手段 (案) | 版 |
|---|---|---|---|
| 既定値入りの注釈付き設定例 (`evoeditor --dump-config --defaults --annotated`) | 初心者が「コピーして編集」できる雛形。項目ごとに説明コメントと既定値 | **v0.1 は手書きの注釈付き雛形 `config.example.toml` を同梱** し、CI で既定値と突合する (09 §11.2。v0.1 は eve-config の既定値を出すテストで、v0.2 から `--dump-config --defaults` で)。doc コメント収集マクロ (proc-macro 依存またはメタ表の二重管理) は v0.4 で検討 | v0.1 (手書き) / v0.4 (生成) |
| JSON Schema (`evoeditor --dump-schema`) | 他エディタ (VS Code, taplo) での補完・検証 | `schemars` (optional feature、起動経路に含めない) | v0.4 (Could) |
| ヘルプ画面の設定一覧 | FR-514 (設定一覧は追加分) | メタ表 | v0.4 |

### 4.2 検証規則

| 検査 | 結果 | 例 |
|---|---|---|
| TOML 構文エラー | そのファイルを無視。エラー 1 件 (行・列) | `expected '=' at line 12` |
| 未知のキー | **警告** (無視して続行)。綴り違いの候補を提示 (編集距離 ≤ 2) | `editor.tab_witdh` → 「`editor.tab_width` ですか?」 |
| 型不一致 | 警告。該当項目を下位層の値に戻す | `tab_width = "4"` (文字列) |
| 範囲外 | 警告。**項目ごとにメタ表 (§4.1) で指定した扱い** に従う: 既定値へ戻す (指定が無い項目の扱い)、または最も近い許容値に丸める。丸める項目は §5 の説明に「範囲外は丸める」と書く。v0.1 の公開キー (§5.0) に丸める項目は無く、すべて既定値へ戻す | `tab_width = 0` / `tab_width = 99` → 既定 4 |
| 列挙値不正 | 警告。許容値を列挙して提示 | `line_ending = "lf\n"` |
| 未知のコマンド ID (キーマップ) | 警告。該当割当のみ無効 | `"ctrl+k" = "file.svae"` |
| キー表記の構文エラー | 警告。該当割当のみ無効 | `"ctl+s"` |
| キーの衝突 (FR-608) | 情報。同じコンテキスト・同じキーを **別の定義行で重複** 定義した場合は後勝ちで解決した上で通知 (配列値 `[ {…}, {…} ]` は先頭から `when` が真の最初を採用する正規の書き方で、衝突ではない。02 §7.2 K4)。プレフィックス衝突 (`ctrl+k` 単体と `ctrl+k ctrl+c` の両方) は警告し、単体側を無効化 (02 §7.2 K3) | §7.6 |
| 廃止キー (リネーム済み) | 情報。新名称へ自動変換して適用。`--fix-config` で書き換え提案 (Could) | `editor.tabsize` → `editor.tab_width` |
| 存在しないテーマ / 言語 / ロケール名 | 警告。既定にフォールバック | `theme.name = "solarized"` |
| 信頼されていないワークスペースの危険項目 | 情報 (「信頼すると適用されます」)。適用しない | §12.3 |
| 安全性違反 (`${...}` 展開でパス外参照、コマンド文字列に shell メタ文字 等) | エラー。項目を無効化 | §9.5 |

検証は **全項目を走査して診断を列として返し、最初のエラーで停止しない**。

### 4.3 エラー表示

- 起動時 (v0.1): 診断は Message キュー (FR-505) に `warn` レベルで積み、**最初の Frame 描画後** に表示する (起動を妨げない、FR-703)。1 件ならその内容 (`msg.config.problem`)、2 件以上なら「設定に N 件の問題があります。最初の 1 件: …」と要約する (`msg.config.problems`。文言は 04 §15.2)。全件は `warn` でログ (05 §7.3) にも書く。診断は host が `Editor::report(HostReport)` で core に渡し、Message キューへの積みは core が行う (02 §5.4)。「このバージョンでは未対応」の情報診断 (公開前のキー・予約のセクション) は件数に数えず、ログにだけ書く (将来の設定を先に書いた利用者を毎回の起動で煩わせない)。
- `config.show_diagnostics` コマンド (**v0.2**): 一覧パネル (意味的モデル `List`) で表示し、Enter で該当ファイルの該当行を開く。
- 各診断は NFR-602 に従い「何が起きたか」+「どうすればよいか」を含む: `tab_width は 1〜16 の整数です。"4" は文字列なので既定値 4 を使います。引用符を外してください (config.toml:7)`。v0.1 の公開キーは 7 個なので、未知のキーの綴りの候補 (編集距離 ≤ 2) は公開キーと公開前のキー (§5.13) の両方から探す。
- CLI (v0.2): `evoeditor --check-config` は診断を標準出力に出して終了する (終了コード §18.4)。CI やドットファイル管理に使う。
- 文言はすべて MessageKey 経由 (§17)。

### 4.4 スキーマの互換性方針

- 設定キーは **公開 API** とみなす。リネーム・削除時は最低 2 マイナーリリースの間、旧名を alias として受け付けて情報診断を出す。
- 明示的な `schema_version` キーは設けない (ユーザーが書き忘れる・意味が分かりにくい)。互換性は alias 表で維持する。破壊的変更が避けられない場合はメジャーバージョンで行い、`evoeditor --migrate-config` を提供する (Could)。
- 既定値の変更は CHANGELOG に明記し、挙動が大きく変わるもの (例: 既定テーマ) は初回起動時に 1 回通知する。

---

## 5. 設定項目一覧 (スキーマと既定値)

> 「提案中」。型は TOML 型で記す。`enum` は許容値を `|` で列挙。**P 列** は項目が意味を持つ最初の版を版名 (`v0.1` / `v0.2` / `v0.3` / `v0.4` / `v1.L-a` / `v1.L-b` / `v1.X`) で書く。**設定ファイルから読めるキー (公開版)** は P 列とは別で、v0.1 は §5.0 の 7 個だけ。P 列が「v0.1」のキーは §5.0 の公開キー、「v0.1 (定数、公開 v0.2)」は v0.1 では既定値を定数として実装しキーの公開は v0.2 のもの (§5.13 も同じ扱い)。v0.2 以降に意味を持つ項目は、P 列の版でキーも公開する。公開前のキーを書いても「未知のキー」ではなく「このバージョンでは未対応」の情報診断にする (将来の設定を先に書いても壊れない)。

### 5.0 v0.1 の公開キー

v0.1 で設定ファイル (`config.toml`、ユーザー層) から読む **公開キーの全集合** (ADR-0028。**7 個**、確定 (ユーザー回答)。v0.1 の範囲は案 L)。これ以外は定数。選定基準: (a) 環境や利用者によって既定値が合わないことがあり、ファイルの中身に直接効く (新規ファイルの文字コード・改行、タブ幅・インデント)、(b) 端末の設定と合わせないと表示が崩れる (曖昧幅)、(c) 端末の色深度の判定を手で上書きする唯一の手段 (`theme.color_mode`)。`ui.locale` は v0.2 の M8 で公開した (§5.12、§17.1)。クリップボードは OS 付属の手段 (Win32) で実装する方式が確定しており (OQ-111、確定 (ユーザー回答))、v0.1 に設定は要らないため `[clipboard]` の公開は v0.2。

| セクション | v0.1 公開キー | 根拠 |
|---|---|---|
| `[editor]` | `tab_width`, `indent_unit`, `indent_width` | FR-208 |
| `[file]` | `default_encoding`, `default_line_ending` | FR-108 / FR-113 |
| `[view]` | `ambiguous_width` | NFR-605 |
| `[theme]` | `color_mode` | FR-513、04 §5.2a |

件数の内訳: `[editor]` 3、`[file]` 2、`[view]` 1、`[theme]` 1 (計 7)。**v0.2 の M9a で `editor.undo.total_max_bytes` を公開した (計 14)**。v0.2 の M6 で `file.swap.*` の 5 個 (`enabled` / `interval_ms` / `min_interval_ms` / `max_bytes` / `exclude_secret_files`、§5.2) を公開した (計 12)**。範囲は `interval_ms` 100..=600000、`min_interval_ms` 100..=3600000、`max_bytes` 1048576..=1099511627776 (範囲外は警告して既定値)。`[file.swap]` の表でも `[file]` の中の `swap.enabled = ...` でも書ける。v0.1 で公開しないキー (定数で実装し、公開は v0.2): `editor.auto_indent` (既定 true)、`editor.undo.total_max_bytes`、`file.encoding_detection`、`file.write_bom`、`file.huge_file_warn_bytes`、`file.long_line_threshold`、`file.fsync`、`view.line_numbers`、`view.cursor_line_highlight`、`view.show_whitespace` (全角スペースとタブの可視化。v0.1 は `"minimal"` の定数で常に表示、FR-508)、`ui.hint_bar`、`ui.message_timeout_ms`、`ui.synchronized_output`、`theme.name` (v0.1 はテーマが 1 つ)、`search.case`、`keys.preset`、`log.level` (`EVE_LOG` で代える、§5.11)。**v0.2 の M8 で `ui.locale` を公開した (計 13)**: 値は `"auto"` (既定) か言語タグ (`ja` / `en`。`ja-JP`・`en_US` のように地域を付けても、地域を落として当てる。大文字小文字は区別しない)。同梱していない言語名は警告 (`InvalidValue`、許容値 `auto` / `ja` / `en`) して `auto` として扱い、選択順の次の段へ進む (§17.1)。節は `[ui]`。`[keys.*]` (ユーザーのキー割当) は v0.2 (§7.1)。`[os.*]` / `[env.*]` の条件付きセクションは v0.2 (§3.4)。ここに無いキーの公開時期は各表の P 列と §5.13。公開前のキーを書いても「このバージョンでは未対応」の情報診断で、値は使わない (§5 冒頭)。

### 5.1 `[editor]` — 編集動作

| キー | 型 / 値 | 既定 | 説明 | 関連 | P |
|---|---|---|---|---|---|
| `tab_width` | int 1..16 | `4` | タブ文字の表示幅 | FR-208 | v0.1 |
| `indent_unit` | `"tab" \| "space"` | `"space"` | インデント操作で挿入する単位 | FR-208 | v0.1 |
| `indent_width` | int 1..16 | `4` | スペースインデントの幅 | FR-208 | v0.1 |
| `auto_indent` | bool | `true` | 改行時に直前行のインデントを継承 | FR-202 | v0.1 (定数、公開 v0.2) |
| `auto_pairs` | bool | `false` | 括弧・引用符の自動閉じ | FR-210 | v0.4 |
| `word_separators` | string | ASCII の空白・タブ・括弧・句読点・演算子記号 (`( ) [ ] { } < > , ; : . ! ? / = + - * & ^ % $ # @ ~`) と、引用符・バッククォート・バックスラッシュ・縦棒 | 単語境界の追加区切り。v0.1 の単語移動は **文字種の遷移規則** (03 §13.3、OQ-117) だけで、本キーは v0.2 で公開 | FR-302 | v0.1 (定数、公開 v0.2) |
| `undo.max_steps` | int | `10000` | 文書あたり Undo 上限 (いずれか一方に達したら古い側から破棄。03 §7.5) | NFR-203 | v0.1 (定数、公開 v0.2) |
| `undo.max_bytes` | int (bytes) | `67108864` (64 MiB) | 同上 (メモリ) | NFR-203 | v0.1 (定数、公開 v0.2) |
| `undo.total_max_bytes` | int (bytes) | `268435456` (256 MiB) | **プロセス全体** の Undo 履歴上限 (02 §14.1 MR2、03 §7.5)。超えたら履歴の最も大きい文書から古い順に破棄 (直前の 1 操作は残す。破棄しても dirty 表示は変わらない)。範囲 1048576..=1099511627776 (範囲外は警告して既定値)。**v0.2 の M9a で公開・実装した** (`[editor.undo]` の表でも `[editor]` の `undo.total_max_bytes` でも書ける) | NFR-203 | v0.2 |
| `undo.group_timeout_ms` | int | `1000` | 連続入力を 1 Undo にまとめる無入力時間 (03 §7.3、OQ-49) | FR-203 | v0.1 (定数、公開 v0.2) |
| `pending_changes_max_bytes` | int (bytes) | `16777216` (16 MiB) | 未決ジョブ・AI 提案の写像用に保持する ChangeSet の合計上限 (02 §5.3 / 03 §17.1)。**v1.L より前には `pending_changes` 自体が無い** (ジョブは Revision 照合で破棄)。写像は v1.L (Proposal の生成と同時、01 §4.3) | NFR-306 | v1.L |
| `trim_trailing_whitespace_on_save` | bool | `false` | 保存時に末尾空白除去 | FR-216 | v0.4 |
| `insert_final_newline_on_save` | bool | `false` | 保存時に末尾改行付与 | FR-216 | v0.4 |
| `multi_cursor` | bool | `true` | 複数カーソルを有効化 | FR-211 | v0.4 |

### 5.2 `[file]` — ファイル I/O

表のキーは `[file]` セクション内の名前で、フルキーは `file.<キー>` (例: `file.swap.enabled`、`file.backup.dir`)。他文書もフルキーで書く。

| キー | 型 / 値 | 既定 | 説明 | 関連 | P |
|---|---|---|---|---|---|
| `default_encoding` | `"utf-8" \| "utf-8-bom" \| "utf-16le" \| "utf-16be" \| "shift_jis" \| "euc-jp" \| "iso-2022-jp" \| "system"` (WHATWG の別名ラベル (`sjis`、`windows-31j` 等) は上の名前に正規化して受理) | `"utf-8"` | 新規ファイル・判定不能時の文字コード。`"system"` は 05 §6 の OS ロケール由来ヒント。**許容値は v0.1 の対応範囲 (01 の「文字コードの対応範囲」、OQ-03) の 7 種と `"system"` に限る**。これ以外 (例 `"windows-1252"`) は列挙値不正として警告 (§4.2) し、既定値 `"utf-8"` を使う (「ラベルとしては有効だが対応外」でも同じ)。許容値を FR-110 (encoding_rs が扱うもの) へ広げるのは v0.3 | FR-108 | v0.1 |
| `encoding_detection` | `"auto" \| "utf8-only" \| "off"` | `"auto"` | chardetng 推定の使用 | FR-109 | v0.1 (定数、公開 v0.2) |
| `detection_candidates` | array of string | ロケール由来 (05 §6 `system` ヒントが `ja` なら `["utf-8", "shift_jis", "euc-jp", "iso-2022-jp"]`、それ以外は `["utf-8"]`) | **推定結果の事後フィルタ**: chardetng 1.0 には候補リストを与える API が無いため、推定結果が候補外なら信頼度を下げ `default_encoding` と比較して採否を決める (03 §8.2 手順 7)。既定値は 05 §6 の `system` ヒントから決める | FR-109 | v0.1 (定数、公開 v0.2) |
| `default_line_ending` | `"lf" \| "crlf" \| "cr" \| "native"` | **`"lf"`** (全 OS。P6 / NFR-405、05 §6、OQ-04) | 新規ファイルの改行コード。`native` は選択肢として残す | FR-113 | v0.1 |
| `write_bom` | `"preserve" \| "never"` | `"preserve"` | UTF-8 BOM の扱い | FR-108 | v0.1 (定数、公開 v0.2) |
| `large_file_threshold` | int (bytes) | `20971520` (20 MiB) | 重い機能を無効化する閾値 (03 §11.2) | FR-123 | v0.4 |
| `huge_file_warn_bytes` | int (bytes) | `536870912` (512 MiB) | 開く前に警告する固定閾値 (03 §11.2、OQ-05 確定 (ユーザー回答))。閾値を設定で変えられるのは v0.2 (FR-127) | OQ-05 | v0.1 (定数、公開 v0.2) |
| `huge_file_refuse_bytes` | int (bytes) / `0` | `0` (上限なし) | 開かない上限 | OQ-05 | v0.1 (定数、公開 v0.2) |
| `huge_file_warn_ratio` | float 0..1 | `0.5` | 利用可能メモリに対する警告比率 (`SystemInfo` Port 導入後。v0.1 は `huge_file_warn_bytes` のみ) | OQ-05 | v0.4 |
| `long_line_threshold` | int (bytes) | `1048576` (1 MiB) | 長大行の閾値。v0.1 はこれを超える行を含む文書を **読み取り専用** で開き、その行は先頭 64 KiB だけを表示する (03 §11.5、NFR-106。表示・End・スクロールが p99 ≤ 100 ms)。長大行を編集できる縮退 (char 近似モード) は v0.2 | NFR-106 | v0.1 (定数、公開 v0.2) |
| `reload_on_external_change` | `"ask" \| "auto_if_unmodified" \| "never"` | `"auto_if_unmodified"` | 外部変更の扱い (保存時 (v0.1)・フォーカス復帰時 (v0.2)・監視 (v0.4)) | FR-117 | v0.1 (定数、公開 v0.2) |
| `watch.enabled` | bool | `true` | ファイル監視 | FR-117 | v0.4 |
| `watch.poll_interval_ms` | int / `0` | `0` (ポーリングしない) | 監視不可 FS での低頻度ポーリング。明示設定時のみ、最小 2000 (05 §12.2、OQ-78) | NFR-204 | v0.4 |
| `save_strategy` | `"atomic" \| "in_place"` | `"atomic"` | 常に直接上書きにする設定 (05 §4.4)。`in_place` では core が最初から直接上書き (method = InPlace) を選び、乾式走査の後に `write_in_place` を呼ぶ (`write_atomic` を経由しない)。起動時に警告 1 回。ワークスペース層では `in_place` は危険の分類 (§12.3) | ADR-0016 | v0.1 (定数、公開 v0.2) |
| `atomic_save_fallback` | `"ask" \| "fail"` | `"ask"` | 原子的置換不可時 (ハードリンク・`in_place_patterns` 以外。03 §9.3 手順 7)。`ask` = 「直接上書き (バックアップ付き) / 別名で保存 / キャンセル」の確認 (04 の `ui.confirm.save.atomic_unsupported`。既定の選択は「別名で保存」。確定 (ユーザー回答)) を出す、`fail` = 確認を出さずに保存の失敗として報告する。確認なしで直接上書きする値は持たない (直接上書きへのフォールバックには利用者の承認が要る、ADR-0016) | ADR-0016 | v0.1 (定数、公開 v0.2) |
| `hardlink_policy` | `"ask" \| "break_link" \| "write_in_place"` | `"ask"` | `nlink > 1` のファイル (05 §4.4、OQ-73 確定 (ユーザー回答)) | FR-106 | v0.1 (定数、公開 v0.2) |
| `in_place_patterns` | array of glob | `["crontab.*", "*.sudoers.tmp", "sudoers.tmp.*", "kubectl-edit-*", "*.tmp"]` | 一時ディレクトリの **直下** (サブディレクトリを含まない) にあり、ファイル名がこれに一致するものは確認なしで直接上書き (05 §4.4、OQ-34 確定 (ユーザー回答))。パターンは **ファイル名 (最後の要素) だけ** に照合する。`/` (Windows は `\` も) を含むパターンは照合に使わず、設定診断 (警告) を出す。ワークスペース層では危険の分類 (§12.3) | ADR-0016 | v0.1 (定数、公開 v0.2) |
| `fsync` | `"full" \| "file_only" \| "none"` | `"full"` | 保存時の fsync (05 §4.1、OQ-77)。ワークスペース層の `none` は危険の分類 (§12.3) | NFR-301 | v0.1 (定数、公開 v0.2) |
| `swap.enabled` | bool | `true` | swap-lite (03 §10.2 / 05 §7.5): 64 MiB 以下の dirty 文書を全文で原子的に書き出す。緊急退避と同じ manifest 形式 | FR-118 | v0.2 |
| `swap.interval_ms` | int | `2000` | 最終入力からの遅延 | NFR-307 | v0.2 |
| `swap.min_interval_ms` | int | `10000` | 連続入力中の書出し間隔の下限 | NFR-307 | v0.2 |
| `swap.max_bytes` | int (bytes) | `67108864` (64 MiB) | これを超える文書は swap-lite の対象外 (ステータスに表示)。差分ジャーナルは v0.4 以降 | NFR-307 | v0.2 |
| `swap.exclude_secret_files` | bool | `true` | 秘密ファイルパターン (§5.12.2) に一致する文書をスワップしない (OQ-109)。ワークスペース層の `false` は危険の分類 (§12.3) | NFR-505 | v0.2 |
| `backup.enabled` | bool | `false` | 保存時バックアップ | FR-119 | v0.3 |
| `backup.dir` | string (path) | `""` (= `state_dir/backup/`。同一ディレクトリの `name~` にするには `"."`) | バックアップ先 (既定を状態ディレクトリに。OQ-108) | FR-119 | v0.3 |
| `hot_exit` | bool | `false` | 終了時に未保存内容を保持 | OQ-17 | v0.4 |

### 5.3 `[view]` / `[ui]` — 表示

| キー | 型 / 値 | 既定 | 説明 | 関連 | P |
|---|---|---|---|---|---|
| `view.line_numbers` | `"absolute" \| "relative" \| "off"` | `"absolute"` | 行番号。`relative` は v0.4 (公開時 (v0.2) の許容値は `absolute` / `off`。`relative` を書くと列挙値不正の警告ではなく「このバージョンでは未対応」の情報診断にして `absolute` を使う) | FR-502 | v0.1 (定数、公開 v0.2) |
| `view.wrap` | `"off" \| "soft" \| "word"` | `"off"` | 折り返し (ソフトラップは v0.2、OQ-110。v0.1 は横スクロールのみ) | FR-506 | v0.2 |
| `view.wrap_indent` | `"none" \| "same"` (`"plus_one"` は §5.13 のとおり後の版で値を追加) | `"same"` | 折り返し行のインデント継続。`none` = 継続しない、`same` = 論理行のインデント幅で揃える (03 §14.3)。最初から enum にしておき、値の追加は互換変更として扱う (§4.4。型を bool から変えない) (提案、OQ-175) | FR-506 | v0.2 |
| `view.show_whitespace` | `"off" \| "minimal" \| "ideographic" \| "trailing" \| "all"` | `"minimal"` | 空白の可視化。`minimal` = 全角スペース (U+3000) とタブ文字だけを記号と属性で示す (03 §13.2、04 §6.7 I8)。`ideographic` (全角スペースのみ)、`trailing` (行末の空白) と `all` (半角の空白・改行も) は v0.4 (`ideographic` は §5.13 の表。公開時 (v0.2) の許容値は `off` / `minimal`。`ideographic` / `trailing` / `all` を書くと「このバージョンでは未対応」の情報診断にして `minimal` を使う) | FR-508 | v0.1 (定数 `"minimal"`、公開 v0.2) |
| `view.rulers` | array of int | `[]` | 桁ガイド | — | v0.4 |
| `view.scroll_margin` | int | `3` | カーソル周辺の余白行 (04 §10) | — | v0.1 (定数、公開 v0.2) |
| `view.scroll_margin_horizontal` | int | `8` | 横スクロール時の左右余白列 (04 §10) | FR-506 | v0.1 (定数、公開 v0.2) |
| `view.cursor_line_highlight` | bool | **`true`** (OQ-125。TrueColor 既定テーマは行の前景・背景の組を確保できる。`mono` では無効) | カーソル行強調 | — | v0.1 (定数、公開 v0.2) |
| `view.ambiguous_width` | `1 \| 2` | `1` | East Asian Ambiguous の幅。`2` のときは eve-tui が描く罫線・記号が ASCII に縮退する (04 §3.7) | NFR-605 | v0.1 |
| `view.emoji_width` | `"auto" \| 1 \| 2` | `"auto"` | 絵文字幅の補正。core の `EmojiWidth::{Auto, One, Two}` に写す (`"auto"` → `Auto` = 04 の端末補正 (幅プローブ・端末種別の表) が決めた値、決まらなければ 2。`1` → `One`、`2` → `Two`。03 §13.2) | FR-501 | v0.1 (定数、公開 v0.2) |
| `view.control_char_style` | `"caret" \| "hex"` | `"hex"` | 制御文字の可視化表記 (`caret` = `^X`、`hex` = `<U+001B>`。C0 と DEL にだけ効き、C1 は常に `<U+0085>` の形。復号できないバイトの `<0xHH>` とは別物、03 §13.2)。空白の可視化 (`view.show_whitespace` の `"minimal"` = 全角スペースとタブ) とは独立で、この値では変わらない | FR-507 | v0.1 (定数、公開 v0.2) |
| `ui.hint_bar` | bool | `true` | ヒントバー (v0.1 は常時表示) | FR-504 | v0.1 (定数、公開 v0.2) |
| `ui.status_bar` | bool | `true` | ステータスバー | FR-503 | v0.1 (定数、公開 v0.2) |
| `ui.tab_line` | `"auto" \| "always" \| "never"` | `"auto"` (2 バッファ以上で表示) | バッファライン | FR-510 | v0.4 |
| `ui.locale` | string (BCP 47) / `"auto"` | `"auto"` | UI の言語 (§17.1)。`auto` は環境変数 `EVE_LANG`、OS ロケールの順で決める。`--locale` が最優先。v0.2 の M8 で公開した | NFR-603 | v0.2 |
| `ui.message_timeout_ms` | int | `5000` | 情報メッセージの表示時間 (04 §8.9) | FR-505 | v0.1 (定数、公開 v0.2) |
| `ui.mouse` | bool | `true` | マウス入力 (v0.2。v0.1 はマウスを捕捉せず端末の選択・スクロールをそのまま使える、04 §6.8) | FR-307 | v0.2 |
| `ui.bell` | `"visual" \| "audible" \| "off"` | `"visual"` | ベル | — | v0.1 (定数、公開 v0.2) |
| `ui.synchronized_output` | `"auto" \| "always" \| "never"` | `"auto"` | 同期出力 DEC 2026 (04 §4.5。`auto` は既知の問題端末のみ送らない。`never` は表示が乱れる端末での逃げ道) | NFR-103 | v0.1 (定数、公開 v0.2) |
| `ui.alternate_screen` | bool | `true` | 代替スクリーンを使う (04 §14.1) | — | v0.1 (定数、公開 v0.2) |
| `ui.ascii_only` | bool | `false` | `true` で eve-tui が描く罫線・記号を常に ASCII にする (04 §3.7 / §13)。`false` は「強制しない」で、端末側の判定による **自動の ASCII 縮退** (04 §3.7: 曖昧幅が 2 のとき、または conhost と推定されるとき (v0.1 から)、`TERM=linux` 等 (Linux を出荷する版から)) は働く。自動の縮退を止める値は持たない。v0.1 は定数 `false` なので、縮退は自動の判定だけで決まる | NFR-604, NFR-605 | v0.1 (定数、公開 v0.2) |
| `ui.screen_reader_mode` | bool | `false` | 読み上げ配慮 (04 §13) | — (04 §13) | v0.4 |
| `ui.column_unit` | `"grapheme" \| "display"` | `"grapheme"` | ステータスバー・`cursor.goto_line`・CLI の位置指定で使う「桁」の単位。`grapheme` = 書記素数、`display` = 表示列。どちらも 1 始まり (OQ-32 確定 (ユーザー回答)) | FR-503 | v0.1 (定数、公開 v0.2) |
| `ui.palette.hide_unavailable` | bool | `false` | `when` 偽コマンドをパレットから隠す (既定は淡色表示 + 理由、§11.4 / OQ-38) | FR-602 | v0.4 (Could) |

### 5.4 `[theme]`

| キー | 型 / 値 | 既定 | 説明 | P |
|---|---|---|---|---|
| `name` | string | **`"default"`** (**TrueColor 既定テーマ**、ADR-0027 / OQ-27 **確定 (ユーザー回答)**。本文は端末の既定色、強調ロールだけ前景・背景の RGB の組で定義し、v0.1 は truecolor と mono の段を明示する (256 / 16 は自動近似)。§8.1 / 04 §12.1 C2) | テーマ名 (組込み `default` (v0.1)、`ansi` / `default-dark` / `default-light` (v0.2)、`high-contrast-*` (v0.3)、または `themes/<name>.toml` (v0.2))。v0.1 はテーマが 1 つなので選べない | v0.1 (定数、公開 v0.2) |
| `color_mode` | `"auto" \| "truecolor" \| "256" \| "16" \| "none"` | `"auto"` | 色深度判定の上書き。`auto` の判定は 04 §5.2a の判定表が正本 (本書は順序を再掲しない)。段の選択は core (02 §8.2) | v0.1 |
| `background` | `"auto" \| "dark" \| "light"` | `"auto"` | 背景まで塗るテーマ (`default-dark` / `default-light`、v0.2) の明暗選択。`auto` は `COLORFGBG` ヒント (04 §5.2a、OQ-124) → 判らなければ `dark`。`default` テーマは背景非依存なので本キーを見ない | v0.2 |
| `overrides` | テーブル (スコープ → スタイル) | `{}` | テーマの部分上書き (§8.4) | v0.4 |

### 5.5 `[search]`

| キー | 型 / 値 | 既定 | 説明 | P |
|---|---|---|---|---|
| `case` | `"smart" \| "sensitive" \| "insensitive"` | `"smart"` | 大文字小文字 (検索バーを開いたときの初期値。`search.toggle_case` で巡回) | v0.1 (定数、公開 v0.2) |
| `regex_default` | bool | `false` | 既定で正規表現モード (正規表現の検索は v0.2) | v0.2 |
| `max_matches` | int | `10000` | 一致範囲の保持上限 (03 §12.2) | v0.1 (定数、公開 v0.2) |
| `count_limit` | int | `1000000` | 一致件数カウントの打ち切り (FR-404、03 §12、OQ-55) | v0.1 (定数、公開 v0.2) |
| `cp932_compat_fold` | bool | `true` | CP932 互換文字 (〜/～、−/－、‖/∥、¢/￠ 等) を検索で同一視 (03 §8.7、OQ-41。対象表は S-01 後に確定) | v0.3 |
| `newline_sugar` | bool | `true` | 正規表現の `\n` を `\r?\n\|\r` と等価に扱う (03 §12.3) | v0.1 (定数、公開 v0.2) |
| `workspace.respect_gitignore` | bool | `true` | 横断検索・ファインダ | v0.4 |
| `workspace.follow_symlinks` | bool | `false` | 〃。ワークスペース層の `true` は危険の分類 (ワークスペース外を読む、§12.3) | v0.4 |
| `workspace.max_file_size` | int (bytes) | `2097152` | 横断検索対象の上限。ワークスペース層で大きくする値は準安全の分類 (§12.3) | v0.4 |

### 5.6 `[clipboard]`

| キー | 型 / 値 | 既定 | 説明 | P |
|---|---|---|---|---|
| `provider` | `"auto" \| "system" \| "osc52" \| "command" \| "none"` | `"auto"` | システム (OS 付属の手段: Win32 API / pbcopy・pbpaste / wl-copy・xclip・xsel。専用ライブラリは使わない。OQ-111 確定 (ユーザー回答)、05 §8.2) → OSC 52 (v0.2) → 内部レジスタ の順で試す (05 §8.1)。v0.1 は Windows の Win32 API → 内部レジスタ。ワークスペース層の `"command"` は危険の分類 (§12.3) | v0.1 (定数、公開 v0.2) |
| `osc52` | `"auto" \| "always" \| "never"` | `"auto"` | OSC 52 の送出方針 (`auto` は SSH 接続中またはシステム provider 不可時。04 §5.2 / 05 §8.1)。v0.1 は OSC 52 を使わない | v0.2 |
| `osc52_max_bytes` | int | `100000` | OSC 52 送出上限 (端末制限対策。05 §8.4) | v0.2 |
| `command.copy` / `command.paste` | array of string (argv) | `[]` (WSL 検出時は copy = `/mnt/c/Windows/System32/clip.exe` (UTF-16LE の BOM 付きで渡す、05 §8.3) / paste = `/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe -NoProfile -Command "[Console]::OutputEncoding=[Text.Encoding]::UTF8; Get-Clipboard"` (どちらも絶対パス)。Linux / macOS は root しか書けないディレクトリで検出した `wl-copy` / `xclip` / `xsel` / `pbcopy` 等、05 §8.3) | 外部コマンド (シェル解釈なし、PATH 自動検出なし)。明示すれば検出より優先。ワークスペース層の `provider = "command"` と `command.*` は危険の分類 (§12.3)。WSL・Linux・macOS 向けなので、それらを出荷する v0.2 から | v0.2 |
| `system_max_bytes` | int (bytes) | `67108864` (64 MiB) | システムクリップボードへ渡す上限 (02 §14.1 MR5 / 05 §8.4)。超えたら内部レジスタのみ + 通知 | v0.1 (定数、公開 v0.2) |
| `x11_exit_wait_ms` | int | `200` | 終了時の X11 クリップボード所有権の待機 (05 §8.2、OQ-79)。S-14 が不成立で arboard を退路として採用した場合のみ意味を持つ | v0.2 |

### 5.7 `[keys]` — §7 で定義。`[keys]` 直下はオプション、`[keys.<context>]` が割当。

| キー | 型 / 値 | 既定 | 説明 | P |
|---|---|---|---|---|
| `preset` | `"default" \| "vim" \| "none"` | `"default"` | 既定キーマップ層の選択 (`vim` は v0.4+ Could、`none` は上級者が全部自分で書く)。v0.1 は `default` 固定で、`[keys]` を読まない | v0.2 |
| `chord_timeout_ms` | int | `1000` | **曖昧シーケンス (完全一致かつプレフィックス) のときだけ** 使う待ち時間。純プレフィックスは無期限保留 (02 §7.2 K2/K3)。曖昧割当を許すのは層が本キーを宣言した場合だけで、宣言の無い層 (ユーザー層を含む) で曖昧割当が生じたら設定診断で **警告し、チョード側を優先する (完全一致側の割当は無効)** (§7.6)。`0` は「曖昧割当を許さない」 | v0.4 |
| `[keys.layer.<preset>]` の `captures_text` | bool | `false` (`vim` プリセットは `true`) | 修飾なし印字可能文字 (`Text`) を `Key{Char}` として先に照合する層か (02 §7.2 K1)。v0.1 では組込みプリセットのみ宣言でき、ユーザー定義層は v0.4 | v0.4 |

### 5.8 `[[language]]` と `[language-server.<name>]`, `[formatter.<name>]` — §9 で定義。

### 5.9 `[workspace]`

| キー | 型 / 値 | 既定 | 説明 | P |
|---|---|---|---|---|
| `root_markers` | array of string | `[".eve", ".git", ".hg", ".svn"]` | ルート検出の目印 (§12.1) |v0.4 |
| `exclude` | array of glob | `["**/.git", "**/node_modules", "**/target"]` | ファインダ・横断検索の除外 (gitignore に追加) |v0.4 |
| `trust.default` | `"ask" \| "never" \| "always"` | `"ask"` | 信頼の既定挙動 (`always` は非推奨で警告) |v0.4 |
| `trust.remember` | bool | `true` | 信頼判断を状態ディレクトリに記憶 |v0.4 |

### 5.10 `[session]` — §16

| キー | 型 / 値 | 既定 | 説明 | P |
|---|---|---|---|---|
| `restore` | `"off" \| "ask" \| "auto"` | `"off"` | 起動時の復元 |v0.4 |
| `remember_cursor` | bool | `true` | ファイルごとのカーソル位置を記憶 (復元 off でも有効) |v0.4 |
| `max_entries` | int | `200` | 記憶するファイル数 |v0.4 |

### 5.11 `[log]`

| キー | 型 / 値 | 既定 | 説明 | P |
|---|---|---|---|---|
| `level` | `"error" \| "warn" \| "info" \| "debug" \| "trace"` | `"warn"` | ログの購読者は `level` 以上のイベントを渡す (`debug` を指定すれば debug 以上が出る)。優先順は **`--log-level` (v0.2) > 環境変数 `EVE_LOG` > `log.level`** (§18.3)。v0.1 は `EVE_LOG` だけで変える | v0.1 (定数 `"warn"`、公開 v0.2) |
| `include_document_text` | bool | `false` | デバッグ用。`true` は起動時に警告 (NFR-504) | v0.1 (定数、公開 v0.2) |

ログのローテーション (5 MiB × 3 世代) は **定数** で、設定キーにしない (05 §7.3)。

### 5.12 `[ai]`, `[lsp]`, `[plugins]` — 書式のみ本書、意味は各文書

| セクション | 本書が決める書式 | 意味を決める文書 |
|---|---|---|
| `[ai]` | `enabled` (bool, 既定 `false`)、`[ai.providers.<id>]` / `[ai.providers.<id>.caps]` / `[ai.providers.<id>.runtime]`、`[ai.profiles.<name>]`、`[ai.features]`、`[ai.inline]`、`ai.exclude` (glob 配列)、`ai.allow_excluded_paths`、`[ai.approval]`、`[ai.context]`、`[ai.secrets]`、`[ai.limits]`、`[ai.timeouts]`、`[ai.network]`、`[ai.prompts]`、`[ai.audit]`。**キー名・型・既定値・層の扱いの正本は本表と §5.12.1**、意味の正本は 06 §14。用途ごとのモデルは `ai.features.*` のプロファイル割当で選ぶ | 06-ai-integration §14。**`api_key` を平文で書く項目は定義しない** (どの層でもエラー、§12.3。06 §10.1、OQ-36)。`auth = "bearer_env"` + `api_key_env` (環境変数名) か `api_key_ref` (資格情報ストアのキー) のみ。v0.1 は `ai.enabled` を認識して「この版では未対応」の情報診断を出す。**ローカル LLM を一級市民** とする (01 §2.9、ADR-0034): 既定の接続先はループバックだけ (NFR-509)、`endpoint` に既定値を持たない (サーバの版で既定ポートが違う)、モデル名を既定値に埋め込まない (06 §4.4) |
| 秘密ファイルパターン | 一覧と照合規則の正本は §5.12.2。`ai.exclude` の **組込み既定値** はこの一覧 (ユーザー層で追加のみ可)。秘密検出パターン集 (06 §10.3。ファイル単位ではなく文字列単位) は `[ai.secrets.patterns]` (同梱データ + ユーザー追加、OQ-83) | 06 §6.5 / §10.3 |
| `[lsp]` | `enabled` (bool, 既定 `true`)、`[language-server.<name>]` (§9.4) | eve-lsp (後続文書) |
| `[plugins]` | `enabled` (bool, 既定 `false`)、`dir`、`[plugins.<id>]` (`enabled`, `approved` (インストール承認の記録, §14.3), `config` テーブル) | §13〜14 |

#### 5.12.1 `[ai]` のスキーマと既定値 (v1.L。ローカル LLM 向け)

意味の正本は 06 §14 / §3.3 / §5.3 / §6.7。本表は **キー名・型・既定値・層の扱い** の正本 (ADR-0025 と同じ分担)。P 列: v1.L-a = ローカル、v1.L-b = クラウド・管理モード、v1.X。v0.1〜v1.0 では `ai.*` を書いても「この版では未対応」の情報診断 (§5 冒頭) で、`ai.enabled` 以外は検証しない。

**プロバイダ (接続先)** `[ai.providers.<id>]` (`<id>` は `[a-z][a-z0-9_]{0,31}`)

| キー | 型 / 値 | 既定 | 説明 | 層 (§12.3) | P |
|---|---|---|---|---|---|
| `kind` | `"llama_server" \| "openai_compatible" \| "ollama" \| "anthropic"` | 必須 | アダプタ。`llama_server` = llama.cpp のネイティブ API (`/props` `/tokenize` `/infill` `/completion` `/slots` `/health`) を使う第一級 (06 §3.3.4)。`ollama` = OpenAI 互換 + 能力取得と FIM のネイティブ補助 (独立した kind にするかは提案中、OQ-142)。`anthropic` は v1.L-b | 常に無視 | v1.L-a (`anthropic` は v1.L-b) |
| `endpoint` | string (URL) または `"unix:<path>"` | **必須 (既定値なし)** | `http://127.0.0.1:8080` 等。`https://` はリモート (v1.L-b、feature `ai-remote`)。`unix:` は Unix ドメインソケット (サーバ対応時。05 §13.7)。ループバック以外は `ai.network.allow_lan_hosts` に載っていなければ接続しない (NFR-509) | 常に無視 | v1.L-a |
| `auth` | `"none" \| "bearer_env"` | `"none"` | `bearer_env` は `api_key_env` の環境変数の値を Bearer に。**平文 `api_key` は無い** | 常に無視 | v1.L-a |
| `api_key_env` / `api_key_ref` | string | — | 環境変数名 / 資格情報ストアのキー (05 §14) | 常に無視 | v1.L-a / v1.L-b |
| `model` | string | `""` | ルーターモード・Ollama・クラウドでのモデル名。単一モデルのサーバでは空のまま。既定値にモデル名を置かない | 常に無視 | v1.L-a |
| `max_concurrency` | int 1..8 | `1` | 同時要求数 (06 §4.8。ローカルは 1 スロット前提) | 常に無視 | v1.L-a |
| `caps.max_context` | int (tokens) / `0` | `0` (= 検出) | 検出結果の上書き | 常に無視 | v1.L-a |
| `caps.fim` | `"auto" \| "infill" \| "suffix" \| "template:<name>" \| "off"` | `"auto"` | FIM の方式。`infill` = `/infill`、`suffix` = OpenAI 互換の `suffix`、`template:<name>` = 同梱の FIM トークンテンプレート名 (モデル系列ごと。名前は 06 §4.4.2) | 常に無視 | v1.L-a |
| `caps.structured` | `"auto" \| "json_schema" \| "grammar" \| "none"` | `"auto"` | 構造化出力の方式 (FR-917 (d)) | 常に無視 | v1.L-a |
| `caps.tokenize` | `"auto" \| "off"` | `"auto"` | サーバのトークナイザで予算を数えるか (FR-917 (b)。`off` は推定) | 常に無視 | v1.L-a |

**管理モード** `[ai.providers.<id>.runtime]` (06 §3.3.3、05 §13.7。v1.L-b、オプトイン)

| キー | 型 / 値 | 既定 | 説明 | 層 | P |
|---|---|---|---|---|---|
| `managed` | bool | `false` | evoeditor が子プロセスとして起動・監視・停止する | 常に無視 | v1.L-b |
| `command` | string (**絶対パス必須**) | — | サーバの実行ファイル。相対パス・名前のみ・`.bat` / `.cmd` は検証エラー。`${config_dir}` / `${home}` は可。初回起動時とコマンド行が変わるごとに完全なコマンド行を示して承認 (**危険項目**: ユーザー層でも `exec` 承認を経る) | 常に無視 (ユーザー層は承認) | v1.L-b |
| `model_path` | string (path) | — | GGUF 等のモデルファイル。evoeditor はサイズだけ取得し中身を読まない | 常に無視 | v1.L-b |
| `args` | array of string | `[]` | 追加引数。**禁止**: `--host` `--port` `--api-key` `--cors-origins` (evoeditor が決める) と `--tools` `--agent` `--mcp-*` `--rpc` 等のツール実行・エージェント・RPC 系 (06 §11.5)。含まれていれば起動を拒否 (検証エラー) | 常に無視 | v1.L-b |
| `ctx_size` | int | `0` (サーバ既定) | コンテキスト長 | 常に無視 | v1.L-b |
| `parallel` | int 1..8 | `1` | スロット数 | 常に無視 | v1.L-b |
| `threads` | int / `0` | `0` (= 論理コア数 − 2、最低 1) | CPU 推論のスレッド数 | 常に無視 | v1.L-b |
| `low_priority` | bool | `true` | nice / `BELOW_NORMAL_PRIORITY_CLASS` で起動 (05 §13.7) | 常に無視 | v1.L-b |
| `start` | `"on_demand" \| "at_enable"` | `"on_demand"` | 起動時期。エディタの起動経路には入れない | 常に無視 | v1.L-b |
| `idle_shutdown_s` | int / `0` | `900` | 無使用でサーバを止めるまでの秒数 (`0` = 止めない) | 常に無視 | v1.L-b |
| `ready_timeout_s` | int | `120` | `/health` が ok になるまでの上限 | 常に無視 | v1.L-b |
| `transport` | `"tcp" \| "unix"` | `"tcp"` | 待受は常にループバックまたは `runtime_dir` のソケット (05 §13.7)。`0.0.0.0` 等は指定不可 | 常に無視 | v1.L-b |

**プロファイル** `[ai.profiles.<name>]` (用途ごとのモデル・サンプリング。同梱 `edit` / `complete`、`provider` は未設定。`ai.profile_select` で切替)

| キー | 型 / 値 | 既定 (`edit` / `complete`) | 説明 | 層 | P |
|---|---|---|---|---|---|
| `provider` | string (`ai.providers.<id>`) | 未設定 (有効化時に選択を促す) | 使うプロバイダ | 常に無視 | v1.L-a |
| `temperature` / `top_p` / `top_k` / `min_p` / `repeat_penalty` | float / float / int / float / float | `0.2` `0.95` — — — / `0.1` `0.9` `40` — — | サンプリング。サーバが受け付けないものは送らない (能力検出) | 常に無視 | v1.L-a |
| `seed` | int | `-1` (乱数) | テストでは固定 (09 §3.9) | 常に無視 | v1.L-a |
| `max_output_tokens` | int | `2048` / `128` | 出力上限 | 常に無視 | v1.L-a |
| `max_context_tokens` | int / `0` | `0` (= `caps` に従う) | 入力予算 (06 §6.4) | 常に無視 | v1.L-a |
| `time_limit_ms` | int / `0` | `0` / `1000` | 生成時間の上限 (`t_max_predict_ms` 相当) | 常に無視 | v1.L-a |
| `minimal` | bool | `false` | 小さいモデル向けの最小プロンプト (06 §4.4.2) | 常に無視 | v1.L-a |

**用途の割当と補完** `[ai.features]` / `[ai.inline]`

| キー | 型 / 値 | 既定 | 説明 | 層 | P |
|---|---|---|---|---|---|
| `features.edit` / `features.chat` | string (プロファイル名) | `"edit"` | 用途 → プロファイル | 常に無視 | v1.L-a / v1.L-b (chat) |
| `features.inline_completion` | `"off"` または プロファイル名 | `"off"` | インライン補完 (FR-910)。ループバックのプロバイダでのみ有効化できる (リモートは v1.X) | 常に無視 | v1.L-a |
| `inline.debounce_ms` | int | `150` (S-22 で確定) | 入力停止から要求まで (06 §5.3) | 可 | v1.L-a |
| `inline.prefix_lines` / `inline.suffix_lines` | int | `256` / `64` | FIM に渡す前後の行数 | 可 | v1.L-a |
| `inline.max_line_suffix` | int | `8` | カーソル行の後方で補完を出す上限文字数 | 可 | v1.L-a |
| `inline.max_lines` | int | `10` | ゴーストテキストの最大行数 | 可 | v1.L-a |
| `inline.stream` | bool | `false` | 補完をストリーミング表示するか | 可 | v1.L-a |
| `inline.prewarm` | bool | `true` | 入力中にキャッシュを温める先読み (06 §5.3) | 可 | v1.L-a |
| `inline.extra_context` | `"off" \| "open_documents"` | `"off"` | 他の開いている文書の断片を渡す (ループバックのみ、OQ-135 で確定) | **常に無視** | v1.L-a |
| `inline.extra_chunks` / `inline.extra_chunk_lines` | int | `16` / `64` | `extra_context` の量 | 可 | v1.L-a |

**承認・文脈・秘密・上限・監査・外部エージェント** `[ai.approval]` / `[ai.context]` / `[ai.secrets]` / `[ai.limits]` / `[ai.prompts]` / `[ai.audit]` / `[ai.agents]` / `[ai.exec]` (意味は 06 §14 の表)

| キー | 型 / 値 | 既定 | 説明 | 層 (§12.3) | P |
|---|---|---|---|---|---|
| `approval.granularity` | `"always" \| "provider_file_session" \| "session"` | `"provider_file_session"` | 送信承認の粒度 (06 §6.6、OQ-25) | 常に無視 | v1.L-a |
| `approval.skip_preview_for_loopback` | bool | `false` | ループバックのプロバイダで送信プレビューを省略する (秘密ファイル規則の確認は省略しない、06 §6.6) | 常に無視 | v1.L-a |
| `allow_excluded_paths` | array of string (path) | `[]` | 除外パターンの個別解除 (都度確認、06 §6.5) | 常に無視 | v1.L-a |
| `context.surrounding_lines` | int | `50` | 指示編集・診断説明で添える周辺行数 (06 §6.3) | 可 | v1.L-a |
| `context.load_instruction_files` | bool | `false` | ワークスペースの指示ファイルを読むか (06 §9.3。信頼済みワークスペースに限る) | 常に無視 | v1.L-a |
| `secrets.mode` | `"mask" \| "warn" \| "block" \| "off"` | `"mask"` | 送信前の秘密検出の動作 (06 §10.3) | 厳しくする方向のみ (`off` < `warn` < `mask` < `block`) | v1.L-a |
| `limits.*` | int / float (要求あたり最大入力トークン・セッション累計・日次累計 (トークンまたは金額)・単価) | 未設定 (= 無制限。使用量の表示はする) | 使用量の上限と単価 (06 §12.1、FR-916)。個々のキー名は v1.L-b 着手時に本表へ採録する | 小さくする方向のみ | v1.L-b |
| `prompts.*` | string (テンプレート) | 未設定 (同梱のテンプレート) | 用途別プロンプトテンプレートの上書き (06 §4.3) | 常に無視 | v1.L-a |
| `audit.enabled` | bool | `true` | 監査ログ (06 §15、FR-909) | 常に無視 | v1.L-a |
| `agents.*` | テーブル (起動コマンド等) | 未設定 | 外部エージェント。`exec` そのもの (06 §8) | 常に無視 | v1.X |
| `exec.allowlist` | array of string | `[]` | セッション承認を許すコマンドの完全一致リスト (06 §8.2) | 常に無視 | v1.X |

**ネットワーク・タイムアウト** `[ai.network]` / `[ai.timeouts]`

| キー | 型 / 値 | 既定 | 説明 | 層 | P |
|---|---|---|---|---|---|
| `network.allow_lan_hosts` | array of `"host:port"` | `[]` | LAN 上のサーバへの接続の明示許可 (06 §11.3、NFR-509)。既定ではループバック以外に接続しない | 常に無視 | v1.L-a |
| `network.allow_insecure_hosts` | array of `"host:port"` | `[]` | LAN の平文 HTTP を許すホスト | 常に無視 | v1.L-a |
| `timeouts.connect_ms` / `first_token_ms` / `idle_ms` | int | `10000` / `60000` / `30000` (管理モードのモデル読込待ちは `runtime.ready_timeout_s` で別) | 接続 / 最初の応答 / 無応答間隔 | 可 | v1.L-a |

- `ai.*` のキーは **ユーザー層と `--config` のみ** で意味を持つ (ワークスペース層の扱いは §12.3)。層の列の「常に無視」は、信頼済みワークスペースでも無視して警告することを指す。
- 条件付きセクション (§3.4) は使える (`[os.macos.ai.providers.local]` で Metal 用の引数を分ける等)。
- CLI (§18.3) に AI 専用のオプションは追加しない (`--set ai.features.inline_completion=off` 等で足りる。v0.2 の `--set` 以降)。

#### 5.12.2 秘密ファイルパターン (正本)

秘密を含み得るファイルを名前で判定する **組込みの一覧** で、本節が一覧と照合規則の正本である (02 §5.4 / §11.2.1 と 06 §6.5 は本節を参照する)。v0.1 から定数として持ち、設定キーにしない (一覧そのものを利用者が減らす手段は無い)。

| 種類 | パターン |
|---|---|
| 環境変数ファイル | `.env`, `.env.*` |
| 鍵・証明書 | `*.pem`, `*.key`, `*.p12`, `*.pfx`, `*.keystore`, `*.jks`, `id_rsa*`, `id_ed25519*`, `id_ecdsa*` |
| パスワード保管・資格情報 | `*.kdbx`, `.netrc`, `.npmrc`, `.pypirc`, `credentials*` |
| インフラの状態・変数 | `*.tfstate`, `*.tfvars` |
| ディレクトリ (その下のすべてのファイル) | `.aws/**`, `.ssh/**`, `.gnupg/**` |

- **照合**: 対象は正規化してシンボリックリンクを解決した実パス。`/` を含まないパターンはファイル名 (パスの最後の要素) だけに照合し、`<名前>/**` はパスのどこかにその名前のディレクトリがあれば一致とする。`*` は 0 文字以上 (区切り文字を含まない)。大文字小文字は区別しない (安全側)。v0.1 はこの規則だけを実装した固定の照合関数で持ち、glob のライブラリ (globset、v0.4) を使わない。パスの無い文書 (無題) は一致しない。
- **使う場所**: (1) 緊急退避の manifest の `sensitive` (`DocSnapshot.sensitive`、02 §11.2.1。v0.1)、(2) スワップの除外 `file.swap.exclude_secret_files` (§5.2。v0.2、OQ-109)、(3) AI に送らないファイルの組込み既定 (`ai.exclude` の既定値、06 §6.5 の規則 2。v1.L)。
- **追加**: v1.L 以降、`ai.exclude` へのユーザー層・ワークスペース層の追加は AI の除外にだけ効く (退避の `sensitive` とスワップの除外は本節の一覧だけで判定する)。一覧への追加・変更は本節を改訂して行い、CHANGELOG に記す。

### 5.13 後続版で公開する設定項目 (v0.1 は既定値を定数として実装)

03/04 の「設定可能」と書かれた挙動のうち、v0.1 で設定スキーマに **公開しない** もの (§5.0 以外のキーはすべてここか各表の P 列に従う)。v0.1 は下表の既定値を定数で実装し、キーは表に版の注記が無ければ v0.2、注記があればその版で追加する (先に書いても「この版では未対応」の情報診断、§5 冒頭)。

各表で P 列が「v0.1 (定数、公開 v0.2)」の項目も同じ扱いである。04 が参照する端末表示の項目を明示しておく: `ui.ascii_only` / `ui.alternate_screen` / `ui.bell` / `ui.status_bar` / `view.emoji_width` / `view.control_char_style` / `view.scroll_margin` / `view.scroll_margin_horizontal` / `ui.column_unit` は v0.1 では §5.3 の既定値の定数で、キーの公開は v0.2。ASCII への自動縮退は設定ではなく端末側の判定で働く (v0.1 から: 曖昧幅が 2、または conhost と推定されるとき。`TERM=linux` 等は Linux を出荷する版から。04 §3.7)。§5.0 の「v0.1 で公開しないキー」も同じ扱いである。

| キー (公開版) | 既定 (v0.1 の定数) | 由来 |
|---|---|---|
| `editor.backspace_unit` | `"grapheme"` (`"codepoint"` に切替可) | 03 §13.1、OQ-45、FR-201 |
| `editor.backspace_unindent` | `false` | 03 §15.3 |
| `editor.undo.break_on_word` | `false` | 03 §7.3、OQ-50 |
| `editor.word_attach_hiragana` (v0.4) | `false` (v0.1 の規則は全文字種共通の遷移で、漢字→ひらがなも境界。OQ-117) | 03 §13.3 |
| `editor.word_motion` | `"next_start"` (`"current_end"` = macOS 流) | 03 §5.3、OQ-46 |
| `editor.vertical_motion_at_edge` | `"doc_edge"` (`"stay"`) | 03 §5.2 |
| `editor.paste_normalize_eol` | `true` | 03 §8.4、OQ-51 |
| `editor.newline_keep_whitespace_only_line` | `true` | 03 §15.3 |
| `editor.trim_trailing_whitespace_skip_cursor_line` (v0.4) | `true` | 03 §15.3 |
| `view.wrap_indent` の値 `"plus_one"` (論理行のインデント幅 + 1 段) の追加 (公開版は未定。`"none" \| "same"` は v0.2 から公開、§5.3) | `"same"` (v0.1・v0.2 は `none` / `same` のみ) | 03 §14.3 |
| `view.wrap_kinsoku` | `true` (禁則処理) | 03 §14.3、OQ-57 |
| `view.width_risky_display` | `"unicode"` (`"replace"`) | 04 §4.7、OQ-64 |
| `view.show_whitespace` の値 `"ideographic"` (全角スペースのみ) (v0.4) | — | 04 |
| `file.swap.compact_bytes` / `file.swap.large_interval_ms` (v0.4、差分ジャーナル導入時) | 8 MiB / 30000 | 03 §10.2 |
| `search.replace_all_select_result` | `false` (Helix 風の全選択は opt-in) | 03 §12.5 |
| `search.regex_size_limit` | 10 MiB | 03 §12.3 |
| `ui.mouse_wheel_lines` (v0.2) | 3 | 04 §6.8 |
| `ui.which_key_delay_ms` (v0.2) | 500 | 04 §7.4 |
| `ui.visual_bell_ms` | 100 | 04 §6.9 |
| `process.shell` (v0.4、`ProcessRunner` と同時) | Unix: `["/bin/sh", "-c"]`、Windows: `["cmd.exe", "/D", "/C"]` (`%ComSpec%` を優先) | 05 §13.2 |

---

## 6. 再読込 (ホットリロード)

| 段階 | 内容 | 版 |
|---|---|---|
| 手動再読込 | `config.reload` コマンド。全層を再読込し、新しい `Arc<ResolvedConfig>` に差し替える | v0.2 (v0.1 は再起動で反映) |
| 自動再読込 | `FileWatcher` Port でユーザー層・ワークスペース層のファイルを監視し、変更後 300 ms デバウンスで再読込 | v0.4 (FR-706, Could) |

**規則**:

1. 再読込に **エラーがあれば旧設定を維持** し診断のみ表示する (半端な状態にしない)。構文エラーでファイルが無視される場合も同様に旧設定を維持する (起動時とは異なる。起動時は「無いもの」として扱うしかない)。
2. 項目は 3 種に分類し、メタ表で宣言する:
   - **即時**: 表示・キーマップ・テーマ・検索オプション等。差し替えた瞬間から適用。
   - **新規文書から**: `file.default_encoding`、`[[language]]` の `file_types` 等。開いている Document には遡及しない (遡及すると「開いたときの判定」と食い違う)。再適用したいときは `lang.redetect` コマンド。
   - **再起動が必要**: `log.*` の出力先、`frontend` 固有初期化。診断で「再起動後に有効」と伝える。
3. `ResolvedConfig` は不変で `Arc` 共有される ([02 §5.1](02-architecture.md#51-所有構造))。ジョブへ渡したスナップショットは古い設定で完走してよい。
4. 再読込は差分を計算して「変更された項目」を情報メッセージで要約する (`3 項目が変わりました: editor.tab_width, theme.name, …`)。
5. 自動再読込は **ワークスペース層の信頼状態を再評価** する。信頼済みワークスペースで危険項目 (§12.3) が **増えた** 場合は自動適用せず、再確認を求める (git pull で設定が書き換わる攻撃経路への対策)。

---

## 7. キーマップ

キーマップの **解決意味論** (Text/Key の照合、純プレフィックスの無期限保留、曖昧時の `chord_timeout_ms`、同一キー競合、層の優先、`mod` の解決) の正本は [02 §7.2](02-architecture.md#72-キーマップ解決は-core-で行う) (規則 K1〜K8。ADR-0022 / OQ-26) であり、本節はその **書式** のみを定める。既定の割当表 (どのキーに何を割り当てるか) は 04 §7.2 が設計入力、§11.6 のカタログ「既定キー」列が正本。

### 7.1 書式

```toml
[keys.editor]                         # コンテキスト "editor" (編集面にフォーカス)
"ctrl+s"        = "file.save"          # 最短形: キー = コマンド ID
"ctrl+shift+s"  = "file.save_as"
"ctrl+q"        = "app.quit"
"alt+up"        = { command = "edit.move_line_up" }
"ctrl+k ctrl+c" = { command = "edit.toggle_comment" }                 # コード (2 ストローク)
"f5"            = { command = "lang.format", args = { on_save = false } }
"ctrl+d"        = { command = "select.add_next_match", when = "!selection.empty" }
"ctrl+w"        = "none"                                              # 既定の割当を解除

[keys.palette]                        # コマンドパレット表示中
"ctrl+n" = "ui.list_next"

[keys.global]                         # どのコンテキストでも最後に参照される層
"f1" = "ui.palette_open"
```

| 要素 | 文法 |
|---|---|
| コンテキスト | `global` / `editor` / `search` (検索バー) / `prompt` (1 行入力欄の編集。全入力欄で共通) / `palette` / `list` (一覧 UI 共通) / `dialog` (確認ダイアログ) / `help` (ヘルプ画面) / `finder` (v0.4) / `panel.<name>` (v0.4+)。core が公開する固定集合。未知名は警告。参照順は下の表 |
| キー | 1 つ以上の **ストローク** を空白で区切ったもの (コード)。書式は v0.1 から固定する (FR-604)。版ごとの解決範囲: **v0.1** はユーザーのキー割当を読まない (`[keys]`・`[keys.*]`・`keymap.toml` は「このバージョンでは未対応」の情報診断。チョードは組込みの既定キーマップのものだけが動く)、**v0.2** はユーザー割当の単一ストロークを解決し、2 ストローク以上は「このバージョンでは未対応」の情報診断、**v0.4** でユーザー定義のチョードを解決する (ADR-0039) |
| ストローク | `[修飾+]*キー名`。修飾は `ctrl` / `alt` / `shift` / `super` (別名 `cmd`, `win`, `meta` を受理し `super` に正規化) / **`mod`** (主修飾の抽象。OQ-29 / 02 §7.2 K6)。順序は任意、正準化は `ctrl+alt+shift+super+キー` |
| `mod` | **正準化時に frontend / OS で解決** する: TUI (全 OS) = `ctrl`。既定キーマップ (§11.6) は `mod+s` のように書く。表示 (04 §7.8) は解決後の名前 (TUI は全 OS で `Ctrl+S`。`⌘S` / `⌃S` の記法は使わない)。`mod` と `ctrl` / `super` を同じストロークに併記した場合は解決後に重複としてエラー。既定キーマップで `ctrl` を直書きする英字との組 (`ctrl+k` のチョード、`list` の `ctrl+n` / `ctrl+p` 等) は `mod` を使わない例外で、一覧は §11.6 の「既定キー」の規則に書く |
| 値 | 文字列 (コマンド ID) / `"none"` (解除) / テーブル `{ command, args?, when? }` / 配列 `[ {…}, {…} ]` (同じキーに `when` が異なる複数割当。先頭から `when` が真の最初を採用、02 §7.2 K4) |

**コンテキストの参照順** (フォーカスのある場所ごと。前にあるものが優先。各コンテキストの中の層の優先は §7.4):

| フォーカス (02 §8.2 `FocusTarget`) | 参照するコンテキスト |
|---|---|
| 編集面 | `editor` → `global` |
| 検索バー | `search` → `prompt` → `global` |
| 入力プロンプト (開く・行へ移動・名前を付けて保存・引数の入力) | `prompt` → `global` |
| コマンドパレット | `palette` → `list` → `prompt` → `global` |
| バッファ一覧 (v0.2) | `list` → `prompt` → `global` |
| ヘルプ画面 (v0.2) | `help` → `global` |
| 確認ダイアログ | `dialog` のみ (**モーダル**。`global` を参照しない。抜ける手段は `esc` と選択肢) |

- `global` に置く既定の割当は `app.quit`・`ui.palette_open` (`mod+p` と `f1`)・`ui.redraw` だけ (どの入力欄からでも抜けられ、パレットが開ける。v0.2 で `help.open` を加える)。
- `prompt` は入力欄の編集 (`edit.*` / `cursor.*` / `select.all` / `edit.paste` の欄内版) と `ui.confirm` / `ui.cancel` を持つ。`search` は `enter` / `up` / `down` を検索の移動に、`palette` と `list` は `up` / `down` を候補の移動に使うので、`prompt` より先に参照する (同じキーが衝突しない)。
- **チョードの 2 打目と文字キー**: 端末からは修飾なしの文字が `Text` で届く。チョードの保留中と、組込みの `dialog` 層 (アクセラレータの照合のため `captures_text` を宣言する) では、単一書記素の `Text` を `Key{Char(c)}` として照合する (02 §7.2 K1 / K2)。複数書記素の `Text` (IME 確定・高速タイプ) は、チョードの保留中は **先頭の書記素だけ** を照合して残りを捨て、`dialog` では先頭も照合せず **すべて捨てる** (`Paste` も同じ。貼り付けや高速タイプで選択肢が実行されないように。提案、02 §7.2 K1 / K2、OQ-180)。IME が ON のときは文字が前編集に入って届かないので、チョードとアクセラレータは IME を OFF にして使う (ダイアログは `tab` / 矢印 + `enter` でも選べる)。

### 7.2 キー名

| 分類 | 表記 (小文字) | 備考 |
|---|---|---|
| 文字 | `a`〜`z`, `0`〜`9` | 大文字 `A` は `shift+a` に正準化。文字キーは **キーボードレイアウト非依存の論理キー** ではなく、端末/OS から届いた文字 (`KeyCode::Char`) を小文字化したもの。詳細な正規化は 04 |
| 記号 | その文字そのまま: `` ` `` `-` `=` `[` `]` `\` `;` `'` `,` `.` `/` `<` `>` | `+` と空白は区切りと衝突するため `plus` / `space` と書く。`shift+1` と `!` は端末によって届き方が違う → 既定キーマップでは記号への修飾付き割当を避ける (04) |
| 機能 | `enter` (`return`), `tab`, `esc` (`escape`), `backspace`, `delete`, `insert`, `home`, `end`, `pageup`, `pagedown`, `up`, `down`, `left`, `right`, `f1`〜`f24`, `space`, `menu`, `capslock`, `printscreen` | 括弧内は受理する別名 |
| メディア等 | `copy`, `paste`, `cut` (キーボードに専用キーがある場合、kitty プロトコル経由) | Could |
| マウス | `mouse.left`, `mouse.right`, `mouse.middle`, `mouse.wheel_up`, `mouse.wheel_down`、修飾付き可 (`ctrl+mouse.left`)、`mouse.left.double` / `.triple` | FR-307, 311。v0.1 では既定のみ |
| 任意文字 | `"u+3042"` のように Unicode スカラ値 | 特殊キーボード用 |

キー名と修飾の表記 (`ctrl+shift+s`、チョードの区切り等) は **シナリオ DSL ([11 §6.3](11-dev-workflow.md#63-シナリオ-dsl-の書式-正本)) のキー入力と同じ** で、パーサも共用する (テストに書いたキーと設定に書いたキーが同じ意味になるように)。

端末では区別できない組 (`ctrl+i`=`tab`, `ctrl+m`=`enter`, `ctrl+h`=`ctrl+backspace` (レガシー VT。04 §6.2 の 0x08), `ctrl+[`=`esc`、`ctrl+shift+文字`、`alt` と ESC 前置) がある (FR-606)。**書式としては両方を書けるようにし**、解決時に eve-tui が報告する `Capabilities` (拡張キーボードプロトコルの有無) で正規化する。`[E]` の割当 (既定の `ctrl+h` 等) は、正規化で別のキーに畳まれて届かない端末では無効になる (割当として残るが発火しない。04 §6.2 の「影響」)。拡張プロトコルがない端末で `ctrl+shift+s` を書いたユーザーには、設定診断で「この端末では `ctrl+s` と区別できません」と知らせる (v0.4 Could。v0.1 は 04 の代替割当表でカバー)。

### 7.3 `when` 句 ([02 §6.4](02-architecture.md#64-when-コンテキスト))

```
expr   := or
or     := and ( "||" and )*
and    := not ( "&&" not )*
not    := "!" not | atom
atom   := ident | ident op literal | "(" expr ")"
op     := "==" | "!=" | "=~"          ; =~ は正規表現部分一致 (regex crate 構文、v0.4)
ident  := [a-z_][a-z0-9_.]*
literal:= '"' ... '"' | number | true | false
```

コンテキスト変数 (core が公開する固定集合。v0.1 時点の案):

| 変数 | 型 | 意味 |
|---|---|---|
| `editor.focus` | bool | 編集面にフォーカス |
| `overlay.active` | string | 表示中オーバーレイ名 (`""` / `palette` / `prompt` / `dialog` / `buffer_list` (v0.2) / `help` (v0.2) / `finder` (v0.4) / `completion` (v0.4)) |
| `panel.focus` | string | フォーカス中パネル名 (v0.4+) |
| `doc.read_only` | bool | 読み取り専用 |
| `doc.language` | string | 言語名 (`"rust"`, `"plain"`) |
| `doc.modified` | bool | 未保存変更あり |
| `doc.large` | bool | 巨大ファイル縮退中 (FR-123) |
| `selection.empty` | bool | 主選択が空 |
| `selection.multi` | bool | 複数カーソル |
| `selection.multiline` | bool | 選択が複数行 |
| `cursor.at_line_start` / `cursor.at_line_end` | bool | — |
| `keymap.mode` | string | モード名 (`"normal"`, `"insert"` 等。Vim 層用。既定 `""`) |
| `view.wrapped` | bool | 折り返し中 (設定キー `view.wrap` (§5.3、`off` / `soft` / `word`) が `off` 以外のとき真。設定キーとは別の名前空間だが、混同を避けて別名にする) |
| `frontend` | string | `"tui"` |
| `os` | string | `"windows"` / `"macos"` / `"linux"` |
| `caps.<field>` | bool / string | 02 §7.1 `Capabilities` の各フィールド (`caps.can_suspend`、`caps.enhanced_keys`、`caps.mouse` 等)。`app.suspend` の when 等に使う |
| `doc.has_path` / `doc.read_only_releasable` | bool | パス付き文書か / 読み取り専用の理由が解除可能か (03 §9.6。解除のコマンドは v0.2 なので、v0.1 では常に false) |
| `search.active` | bool | 検索バーが開いている |
| `search.replace_active` | bool | 検索バーの置換欄が開いている (`search.replace_open` で開く。04 §8.4) |
| `search.replace_focused` | bool | 検索バーの置換欄にフォーカスがある (v0.2。置換欄の `enter` を `search.replace` にするキーマップの `when` に使う。04 §8.4) |
| `has.lsp` | bool | 言語サーバが接続済み |
| `has.ai` | bool | AI が有効 |
| `ai.busy` / `ai.ghost_visible` / `ai.review_active` / `ai.chat_focus` (v1.L) | bool | AI 要求が進行中 (`Esc` → `ai.cancel`、04 §8.10) / ゴーストテキスト表示中 (`tab` → `ai.complete_accept`) / 差分承認オーバーレイ表示中 / チャットパネルにフォーカス (06 §13.2) |
| `ai.locality` (v1.L) | string | 現在の用途のプロバイダの `"loopback"` / `"lan"` / `"remote"` (06 §11.3)。送信先に応じたキー割当の切替用 |
| `ext.<plugin_id>.<var>` | 任意 | プラグインが公開する変数 (v1.X) |

未知の変数は **設定読込時に警告** し、その割当を無効にする (実行時に評価して例外にはしない)。変数の追加は互換 (追加のみ)、削除は alias 期間を設ける (§4.4)。

### 7.4 層と優先順位 (書式上の宣言。意味論は 02 §7.2 K4 / K5)

解決順 (上が優先、02 §7.2 K5): **オーバーレイ固有の組込み** > `keymap.mode` のモード層 (v0.4+) > **ワークスペース層** `[keys.<コンテキスト>]` > ユーザー層 `[keys.<現在のコンテキスト>]` > ユーザー層 `[keys.global]` > 既定層 (`keys.preset`) の同順。

**ワークスペース層のキーマップは「追加のみ」** (02 §7.2 K5、OQ-35): ユーザー層・既定層のいずれかで既に割当のあるキーをワークスペース層が定義した場合は、**信頼の前後を問わず無視して情報診断** を出す (`Ctrl+S` を別コマンドに付け替える、印字キーを奪う等の誤誘導を防ぐ。ワークスペースを信頼しても既存割当の上書き・`"none"` による解除はできない)。未割当キーへの追加は信頼前でも適用する (安全な項目、§12.3)。ただし `exec` 権限のコマンド (外部コマンド実行) への追加の割当は信頼後のみ。脱出キー (`app.quit` / `ui.palette_open` / `ui.cancel`) へ割り当てたキーは既存割当なので、ワークスペース層からは変えられない。

同じ層・同じコンテキストに同じキーが **別の定義行として** 複数回現れた場合は後勝ちで情報診断 (配列値は先頭から `when` が真の最初。K4)。`"none"` は下位層の割当を **その層で遮蔽** する (さらに上位層で再割当可)。

### 7.5 キーマップのプリセットと切替

- `keys.preset = "default"` (v0.1 はこの値に固定。キーの公開は v0.2。CUA、[ADR-0017](10-decisions.md#adr-0017-既定キーバインドはモードレス)。内容は §11.6 のカタログ「既定キー」列。**格納は Rust の定数表 (ビルド時生成)** で、起動時に TOML を解析しない。TOML は `--dump-keymap` の出力形式。02 §7.2) / `"none"` (既定層なし。`app.quit` と `ui.palette_open` だけは **常に** `[keys.global]` のフォールバックとして残す — 脱出不能を防ぐ P7) / `"vim"` (v0.4+ Could、別文書。この層だけが `captures_text = true` と `chord_timeout_ms` を宣言する、§5.7)。
- 実行時切替 `keys.set_preset` コマンド (セッション層)。
- 「このコマンドにキーを割り当てる」UI (`keys.bind` コマンド、v0.4 Could) はユーザー層 `keymap.toml` に `toml_edit` で追記する。

### 7.6 衝突検出 (FR-608)

| 種類 | 扱い |
|---|---|
| 同一キーの重複 (別の定義行) | 情報、後勝ち (02 §7.2 K4) |
| プレフィックス衝突 = 曖昧なチョード (`ctrl+k` と `ctrl+k x`) | ユーザー設定で生じた場合は **警告し、チョード側を優先して完全一致側 (単体) を無効化** (02 §7.2 K3。既定キーマップにはこの衝突を作らない)。層が `chord_timeout_ms` を宣言している場合 (`vim`) のみ、曖昧割当として受理し deadline 後に完全一致側を実行 |
| 脱出キーの消失 | `app.quit` / `ui.palette_open` / `ui.cancel` (esc) に到達する割当がどのコンテキストにも無い場合は **エラー** とし、`[keys.global]` の既定を強制復活させて通知 |
| 文字入力の遮蔽 | 修飾なしの印字可能キー (`a` 等) を **`captures_text` を宣言していない層** (既定の `editor` コンテキスト等) に割り当てても、`Text` はその層を通らないため **一致せず効かない** (02 §7.2 K1)。設定診断で **警告 + 当該割当を無効** にし、「文字キーを割り当てるには `captures_text` を持つ層 (Vim 層等、v0.4) を使う」と案内する。「文字入力ができなくなる」事故は構造的に起きない |

### 7.7 ヘルプとの連携

`--dump-keymap` (v0.1 は隠しオプション) / `help.keys` (v0.2) は解決後のキーマップを「コンテキスト × キー → コマンド (タイトル・由来の層)」で出力する。コマンドパレットはコマンド ID から現在のキーを逆引きして併記する (FR-602。v0.1 はこれがヘルプ画面の代用、FR-514)。

---

## 8. テーマ

### 8.1 方針

- テーマは TOML ファイル 1 つ。**色はテーマが、構造は core が** 持つ。テーマはスコープ名にスタイルを割り当てるだけで、レイアウトや文字 (罫線・アイコン) は変えられない ([ADR-0008](10-decisions.md#adr-0008-viewmodel-による描画分離))。
- スタイルの解決 (テーマ + Decoration 合成) と **色深度に応じた段の選択** は core が行う (`Capabilities.color_depth`、[02 §8.2](02-architecture.md#82-frame-の構造-擬似コード)、04 §12.1 C1)。Frame には端末が描ける色だけが載り、frontend はセル単位の変換をしない。テーマ作者は TrueColor で書き、mono の段を **明示** する。256 / 16 の段は明示してもよく、省略すると 04 §12.3 の近似で求める (v0.1 の組込みテーマは truecolor と mono だけを書き、256 / 16 は近似)。
- **組込みテーマと v0.1 の範囲 (ADR-0027 / OQ-27 確定 (ユーザー回答)、04 §12.1 C2)**: v0.1 の既定テーマ **`default`** は **最初から TrueColor 対応** (FR-513)。方式: 本文は端末の既定色 (`default` / `default`) のまま、選択・ステータスバー・検索一致・メッセージなどの **強調ロールだけを前景色と背景色の RGB の組** で定義する。組で定義するので端末の背景が明るくても暗くても強調部分のコントラストを保証でき、背景色の問い合わせ (OSC 11) を要しない。**v0.1 は各ロールの truecolor と mono の 2 段を明示** し (確定 (ユーザー回答)、案 L)、256 / 16 色の端末では truecolor の値から近似する (04 §12.3。v0.1 の対象端末 Windows Terminal・conhost は truecolor)。近似で色の区別が弱くなっても、意味のあるロールは mono と同じ属性・記号を併せ持つので判別できる (NFR-604)。256 / 16 の段の明示値は v0.2 以降の候補 (近似の品質に問題の報告があったとき)。縮退品質の確認は P0 スパイク S-20 (truecolor と mono)。**`ansi`** (ANSI 16 色 + 属性のみ。端末配色に完全に馴染む。SSH 先・16 色端末向け) と背景まで塗る **`default-dark` / `default-light`** は v0.2 (テーマ切替と同時)、**`high-contrast-dark` / `high-contrast-light`** (NFR-604) は v0.3 (FR-707)。
- **ロールの形 (組込みテーマ)**: 各ロールの各段は次の 3 形のどれかにする。(i) **前景と背景の組** (強調ロール)、(ii) **色を指定せず属性だけ** (前景・背景とも端末既定。例 太字・反転・下線)、(iii) **前景だけ** (背景は端末既定。淡色の補助表示に限る)。**背景だけを指定して前景を端末既定のままにする形は使わない** (端末の既定前景色が分からないので、明るい背景の端末では暗い背景の上に黒い文字が載って読めなくなる)。
- **コントラスト基準** (01 FR-513 AC-2 / NFR-604 と同じ基準。WCAG 2.x の相対輝度による比): (i) の組は truecolor の段で **4.5:1 以上**。(iii) の前景は truecolor の段で **白 (`#ffffff`) と黒 (`#000000`) の両方の背景に対して 3.0:1 以上**。(ii) は端末の配色そのままなので検査しない。CI の `xtask check-themes` (09 §3.5) が §8.2 の表を入力に、truecolor と mono の段の欠落・形の違反・比の不足を検査し、近似で求めた 256 / 16 の段については「前景と背景が同じ番号にならない」ことだけを検査する。256 / 16 の段を明示する版 (v0.2 以降の候補) では、(i) は 256 の段で 4.5:1 以上・16 の段は xterm 既定パレット (§8.2 の表の下) を仮定して 3.0:1 以上、(iii) は 256 の段で白・黒の両背景に 3.0:1 以上・16 の段で使ってよい前景は `bright_black` だけ、を加える。256 の段は xterm の 256 色の標準値 (16〜231 は 6×6×6 の立方体、232〜255 は灰色) で計算する。値の見た目の調整は S-20。

### 8.2 書式

```toml
# themes/my-theme.toml
[meta]
name = "my-theme"               # ファイル名と一致させる
inherits = "default"            # 省略可。差分だけ書ける
background = "any"              # "dark" | "light" | "any" (背景を塗らないテーマは any)
author = "..."                  # 任意
license = "MIT"                 # 任意 (配布時は推奨、§19)

[palette]                       # 名前付き色 (任意)。以下で名前参照できる
bg      = "#1e1e2e"
fg      = "#cdd6f4"
accent  = "#89b4fa"
red     = "ansi.red"            # 端末の ANSI 色を参照 (縮退に強い)

[ui]
background      = { bg = "bg" }
text            = { fg = "fg" }
cursor          = { fg = "bg", bg = "fg" }
cursor.secondary= { fg = "bg", bg = "accent" }          # 複数カーソルの副
selection       = { fg = "#cdd6f4", bg = "#45475a",                       # 強調ロールは fg/bg の組で書く
                    fallback256 = { fg = "ansi.189", bg = "ansi.238" },
                    fallback16  = { fg = "ansi.bright_white", bg = "ansi.blue" },
                    mono        = { mods = ["reverse"] } }
line_number     = { fg = "#6c7086" }
line_number.active = { fg = "fg", mods = ["bold"] }
cursor_line     = { fg = "fg", bg = "#2a2b3c" }                    # 背景だけでなく前景も組で書く (§8.1 のロールの形)
status_bar      = { fg = "fg", bg = "#313244" }
status_bar.inactive = { fg = "#6c7086", bg = "#313244" }
hint_bar        = { fg = "fg", bg = "#313244" }
hint_bar.key    = { fg = "accent", mods = ["bold"] }
message.error   = { fg = "red", mods = ["bold"] }
message.warning = { fg = "#f9e2af" }
message.info    = { fg = "fg" }
overlay         = { bg = "#181825" }
overlay.selected= { bg = "#45475a", mods = ["bold"] }
overlay.match   = { fg = "accent", mods = ["underline"] }   # パレットの一致箇所
search.match    = { bg = "#f9e2af", fg = "bg" }
search.current  = { bg = "#fab387", fg = "bg" }
whitespace      = { fg = "#45475a" }
control_char    = { fg = "red", mods = ["reverse"] }        # NFR-503 の可視化記号
ruler           = { bg = "#2a2b3c" }
diagnostic.error   = { underline = "curl", underline_color = "red" }
diagnostic.warning = { underline = "curl", underline_color = "#f9e2af" }
diagnostic.hint    = { underline = "dotted" }
virtual_text       = { fg = "#6c7086", mods = ["italic"] } # インレイヒント (v1.L)
virtual_text.ghost = { fg = "#6c7086", mods = ["italic"],   # AI のゴーストテキスト (FR-910、v1.L-a。04 §8.10)。4 段必須:
                       fallback256 = { fg = "ansi.243" }, fallback16 = { mods = ["dim"] }, mono = { mods = ["dim"] } }
diff.add / diff.delete / diff.change = { ... }              # 差分ビュー (v1.L)
pane.border / pane.border.active = { ... }                  # 分割 (v0.4)

[syntax]                         # tree-sitter ハイライトのキャプチャ名に対応 (v0.4)
comment            = { fg = "#6c7086", mods = ["italic"] }
keyword            = { fg = "#cba6f7" }
keyword.control    = { fg = "#cba6f7", mods = ["bold"] }
string             = { fg = "#a6e3a1" }
number             = { fg = "#fab387" }
function           = { fg = "accent" }
type               = { fg = "#f9e2af" }
variable           = { fg = "fg" }
constant           = { fg = "#fab387" }
operator           = { fg = "#89dceb" }
punctuation        = { fg = "#9399b2" }
attribute          = { fg = "#f9e2af" }
markup.heading     = { fg = "accent", mods = ["bold"] }
markup.link        = { fg = "#89dceb", mods = ["underline"] }
markup.raw         = { fg = "#a6e3a1" }
diff.plus / diff.minus = { ... }                            # diff ファイル表示用
```

| 要素 | 文法 |
|---|---|
| 色 | `"#rrggbb"` / `"#rgb"` / `"ansi.<name>"` (`black red green yellow blue magenta cyan white` と `bright_` 接頭辞) / `"ansi.<0-255>"` / `"default"` (端末既定色、透過背景用) / `[palette]` の名前 |
| スタイル | `{ fg?, bg?, mods? = ["bold" "italic" "underline" "reverse" "dim" "strikethrough"], underline? = "line" \| "double" \| "curl" \| "dotted" \| "dashed", underline_color?, fallback256?, fallback16?, mono? }` |
| `fallback256` | `{ fg?, bg? }` を `ansi.<16-255>` で指定。256 色端末での **明示縮退色** (無指定なら 04 §12.3 の 6×6×6 キューブ近似。v0.1 の組込みテーマは書かない) |
| `fallback16` | `{ fg?, bg? }` を `ansi.<name>` で指定。16 色端末での **明示縮退色** (無指定なら 04 §12.3 の近似。近似は端末パレットが不明なため不正確。v0.1 の組込みテーマは書かない) |
| `mono` | `{ mods }`。無色 (`color_mode = "none"` / `NO_COLOR`) 時に色の代わりに使う属性 (例: `selection` は `["reverse"]`、`search.match` は `["underline"]`)。無指定なら 04 §12.4 の既定対応表。組込みテーマでは必須 |
| スコープ名 | ドット区切り。**最長一致で上位スコープへフォールバック** (`keyword.control.return` → `keyword.control` → `keyword`)。未定義なら `ui.text` |

**`default` テーマのロール表 (v0.1 の完全な一覧)**: v0.1 の画面に出るロールは本表がすべてで、**ロール名は公開面** (01 §4.2。追加は互換、削除・改名は §4.4 の手続き)。`xtask check-themes` の入力にする。値は提案で、S-20 の結果で調整する (比は 09 §3.5 の検査で確かめた値)。**v0.1 で実装するのは truecolor と mono の列** で、256 / 16 の列は段を明示値にする版 (v0.2 以降の候補) のための参考値である (v0.1 はこの 2 列を使わず、truecolor の値から 04 §12.3 の近似で求める。属性は近似した段にも付ける)。

凡例 (セルの書き方。`check-themes` はこの文法で読む):

- **属性の語彙**: `bold` (太字) / `underline` (下線) / `reverse` (反転) / `dim` (淡色) の 4 語と、強調しないことを表す `none`。TOML の `mods` の同じ名前に対応する (`italic` / `strikethrough` は本表では使わない)。
- **属性列**: 属性を 1 語ずつバッククォートで囲み、` + ` (前後に空白) でつなぐ (例 `` `underline` + `bold` ``)。並びは意味を持たない。`none` は他の語と結合しない。
- **色の列 (truecolor / 256 / 16)**: 次のどれか 1 つ。(1) 前景と背景をそれぞれバッククォートで囲み ` / ` で区切った組 (例 `` `#808080` / `default` ``。色は上の表の「色」の文法で、16 の段は `ansi.` を省いて名前だけを書く)、(2) (1) の後ろに ` + ` と属性列 (例 `` `#1a1a1a` / `#e6c84a` + `underline` ``。全段に共通の属性もその段のセルに書く)、(3) 属性列だけ (形 (ii) のロール)、(4) `` `none` `` (その段では強調しない。色も属性も付けない)。括弧内の数値はコントラスト比で、文法の外 (読み飛ばす)。
- **mono の列**: 属性列か `` `none` `` だけ (`mono = { mods = [...] }` の値。`none` は `mods = []`)。
- **形の列**: 1 ロールに 1 値 ((i) / (ii) / (iii)、§8.1)。truecolor の段の形を表し、`check-themes` は truecolor の列をこの形で検査する。mono の段は形を問わず常に属性列か `none` である。256 / 16 の参考値で形が違うものは表の下の注記に書く。

| ロール | 用途 | 形 | truecolor (fg / bg) | 256 (fg / bg、参考) | 16 (fg / bg、参考) | mono |
|---|---|---|---|---|---|---|
| `ui.text` | 本文・入力欄の文字・情報メッセージ以外の通常の文字 | (ii) | `default` / `default` | `default` / `default` | `default` / `default` | `none` |
| `ui.cursor_line` | 現在行の強調 (`view.cursor_line_highlight`、既定 on。OQ-125)。行の中で他のロール (選択・検索一致等) が付いたセルはそちらが優先 | (i) | `#e8e8e8` / `#3a3d4a` (8.8) | `ansi.254` / `ansi.236` (10.4) | `none` | `none` |
| `ui.line_number` | ガターの行番号 | (iii) | `#808080` / `default` (白 3.9・黒 5.3) | `ansi.244` / `default` (同左) | `bright_black` / `default` | `none` (通常属性。行番号欄と本文の境界の区切りで区別する、04 §12.4) |
| `ui.line_number.active` | 現在行の行番号 | (ii) | `bold` | `bold` | `bold` | `bold` |
| `ui.eof_marker` | 文書末尾より後の行の `~` | (iii) | `#808080` / `default` | `ansi.244` / `default` | `bright_black` / `default` | `none` (通常属性。04 §12.4) |
| `ui.scroll_marker` | 横スクロールの続きの印 `‹` / `›` | (iii) | `#808080` / `default` + `bold` | `ansi.244` / `default` + `bold` | `bright_black` / `default` + `bold` | `bold` |
| `ui.selection` | 選択範囲 (編集面・入力欄) | (i) | `#eef0f6` / `#3b5bb5` (5.5) | `ansi.255` / `ansi.61` (4.8) | `bright_white` / `blue` (9.4) | `reverse` |
| `ui.search.match` | 検索の一致 | (i) | `#1a1a1a` / `#e6c84a` + `underline` (10.6) | `ansi.233` / `ansi.221` + `underline` (13.5) | `black` / `yellow` + `underline` (12.3) | `underline` |
| `ui.search.current` | 現在の一致 | (i) | `#1a1a1a` / `#f0945a` + `underline` + `bold` (7.5) | `ansi.233` / `ansi.209` + `underline` + `bold` (7.9) | `black` / `bright_yellow` + `underline` + `bold` (19.6) | `reverse` + `underline` |
| `ui.status_bar` | ステータスバー | (i) | `#e8e8e8` / `#2f3340` (10.3) | `ansi.254` / `ansi.237` (9.0) | `reverse` | `reverse` |
| `ui.status_bar.chord` | ステータスバーのチョード保留 (`Ctrl+K …`)。`ui.status_bar` に重ねる | (ii) | `bold` | `bold` | `bold` | `bold` |
| `ui.message.info` | 情報メッセージ (先頭に `ℹ`) | (ii) | `default` / `default` | `default` / `default` | `default` / `default` | `none` |
| `ui.message.warning` | 警告メッセージ (先頭に `⚠`) | (i) | `#1a1a1a` / `#e0a53b` + `bold` (8.0) | `ansi.233` / `ansi.179` + `bold` (9.1) | `black` / `yellow` + `bold` (12.3) | `bold` |
| `ui.message.error` | エラーメッセージ (先頭に `✖`)、正規表現のエラー | (i) | `#ffffff` / `#b3261e` + `bold` (6.5) | `ansi.231` / `ansi.124` + `bold` (7.4) | `bright_white` / `red` + `bold` (5.8) | `bold` + `reverse` |
| `ui.hint_bar` | ヒントバーの説明の語 | (ii) | `default` / `default` | `default` / `default` | `default` / `default` | `none` |
| `ui.hint_bar.key` | ヒントバーのキー表記 | (i) | `#1a1a1a` / `#8fd0ff` (10.5) | `ansi.233` / `ansi.117` (11.8) | `black` / `cyan` (10.6) | `reverse` |
| `ui.overlay` | オーバーレイ (パレット・プロンプト・確認・一覧・ヘルプ) の枠と文字 | (ii) | `default` / `default` | `default` / `default` | `default` / `default` | `none` |
| `ui.overlay.selected` | 一覧の選択中の行 (行頭に `▶`) | (i) | `#eef0f6` / `#3b5bb5` (5.5) | `ansi.255` / `ansi.61` (4.8) | `bright_white` / `blue` (9.4) | `reverse` |
| `ui.overlay.match` | パレットの一致箇所 | (ii) | `underline` + `bold` | `underline` + `bold` | `underline` + `bold` | `underline` + `bold` |
| `ui.overlay.disabled` | パレットの実行できない項目 (理由の文言を併記) | (iii) | `#808080` / `default` | `ansi.244` / `default` | `bright_black` / `default` | `none` (通常属性。行頭の `-` で区別する、04 §12.4) |
| `ui.input.placeholder` | 入力欄が空のときの案内 (`コマンドを入力` 等) | (iii) | `#808080` / `default` | `ansi.244` / `default` | `bright_black` / `default` | `none` (通常属性。全体を丸括弧で囲んで区別する、04 §12.4) |
| `ui.dialog.button` | 確認ダイアログのボタン (`[S] 保存`) | (ii) | `default` / `default` | `default` / `default` | `default` / `default` | `none` |
| `ui.dialog.button.focused` | フォーカス中のボタン (初期は既定ボタン。`▶` 付き) | (ii) | `reverse` + `bold` | `reverse` + `bold` | `reverse` + `bold` | `reverse` + `bold` |
| `ui.search_bar.toggle_on` | 検索バーの ON のトグル (`[Aa✓]` / `[.*✓]`、smart で実際に区別しているときの `[Aa~]`) | (ii) | `reverse` | `reverse` | `reverse` | `reverse` |
| `ui.control_char` | 制御文字・不正バイトの可視記号 (NFR-503) | (i) | `#ffffff` / `#8a3ab9` (6.2) | `ansi.231` / `ansi.91` (7.9) | `bright_white` / `magenta` (4.7) | `reverse` |
| `ui.whitespace` | 全角スペース・タブの可視化の記号 (FR-508 の v0.1 の範囲、03 §13.2)。背景は付けず、選択・検索一致・現在行の背景をそのまま使う | (iii) | `#808080` / `default` (白 3.9・黒 5.3) | `ansi.244` / `default` (同左) | `bright_black` / `default` | `dim` |

- 括弧内の数値はコントラスト比 (組は前景と背景、(iii) は白背景・黒背景に対する比)。16 の段の比は xterm 既定パレット (`black #000000`、`red #cd0000`、`yellow #cdcd00`、`blue #0000ee`、`magenta #cd00cd`、`cyan #00cdcd`、`white #e5e5e5`、`bright_black #7f7f7f`、`bright_yellow #ffff00`、`bright_white #ffffff` 等) で計算した。
- `ui.cursor_line` を組にしたので、明るい背景の端末では現在行が暗い帯に明るい文字で出る (読める)。帯が強すぎると感じる利用者は `view.cursor_line_highlight = false` (v0.2 で公開) で消せる。mono の段は現在行を強調しない。16 の段を明示値にする版では、16 の段も強調しない (背景色の組が端末の配色とぶつかるため)。現在行の行番号の太字 (`ui.line_number.active`) はどの段でも残る。
- `ui.status_bar` の 16 の段を明示値にする版では `reverse` にする (その段だけ形 (ii)。`bright_white` / `black` のような組は黒背景の端末で編集面と区別できないため。FR-513 AC-4)。`ui.cursor_line` の 16 の段の `none` も同じく形の例外 (上の箇条)。v0.1 の近似では、近似した前景と背景が同じ番号になったときに反転に置き換える規則 (04 §12.3) で区別を保つ。
- 意味のあるロール (エラー・警告・情報・選択・検索一致・現在の一致・チョード保留・ボタンのフォーカス) は mono の段でも属性か記号で区別できる (NFR-604 AC-1)。情報メッセージは `ℹ`、警告は `⚠`、エラーは `✖` の記号を常に付ける (04 §8.9)。mono の段では **dim だけで他と区別するロールを作らない** (conhost は dim を描かないため)。行番号・`~`・プレースホルダ・実行できないパレット項目は、通常属性に区切り記号・丸括弧・行頭の `-` を併用する規定に従う (04 §12.4。この規定は提案、04 §18 の OQ-181)。
- v0.4 以降のロール (`ui.ruler`、`diagnostic.*`、`pane.border*`、`[syntax]`) と v1.L の `virtual_text*`・`diff.*` は、その版で本表に加える。上の TOML の例はユーザーテーマの書式の説明で、値は `default` テーマと同じではない。

### 8.3 解決手順

1. `theme.name` を組込み → ユーザー層 `themes/` → ワークスペース層 `.eve/themes/` (安全な項目) の順で探す。
2. `inherits` を再帰的に解決 (循環はエラー、深さ上限 8)。子の定義が親を上書き。
3. `[palette]` 名を色に展開。未知の名前は警告して `default`。
4. `theme.overrides` (§5.4、ユーザー設定側の部分上書き) を最後に適用。
5. 結果を **スコープ名 → 4 段 (truecolor / 256 / 16 / mono) の `Style`** の表として保持し (テーマに書かれていない 256 / 16 の段は 04 §12.3 の近似で埋める。v0.1 の組込みテーマはこの 2 段が常に近似)、core が `Capabilities.color_depth` に応じた段を選んで Frame に載せる (04 §12.1 C1)。この表は `ResolvedConfig` の一部で不変。色深度は起動時に 1 回だけ判定し、実行中は変えない (02 §7.1。`CapabilitiesChanged` では送らない)。

### 8.4 ユーザーによる部分上書き

テーマファイルを複製せずに 1〜2 色変えたい初心者向け:

```toml
[theme]
name = "default"
[theme.overrides]
"ui.selection" = { bg = "#444444" }
"syntax.comment" = { mods = [] }
```

### 8.5 シンタックススコープの命名

tree-sitter のハイライトクエリ (`@keyword.control` 等) のキャプチャ名を **そのままスコープ名** とし、Helix / nvim-treesitter で広く使われている命名に揃える (既存クエリ・テーマ資産の流用が容易。流用時のライセンスは ADR-0018 に従って管理)。正式な一覧は v0.4 着手時に `eve-syntax` の文書で固定し、本書の表は骨子とする。

---

## 9. 言語設定

### 9.1 言語定義の書式

組込み定義 (バイナリ内 TOML、`evoeditor --dump-languages` で出力可) をユーザー層・ワークスペース層が `name` キーで併合する (§3.3)。

```toml
[[language]]
name            = "rust"                         # 小文字、識別子。言語 ID (when 句 doc.language の値)
display_name    = "Rust"
file_types      = ["rs"]                         # 拡張子 (ドットなし)
file_names      = []                             # 完全一致ファイル名 (例 "Makefile", "Dockerfile")
file_globs      = []                             # glob (例 "*.d.ts", ".github/**/*.yml")
first_line      = ""                             # 先頭行の正規表現 (shebang 等。例 "^#!.*\\b(ba|z)?sh\\b")
indent          = { unit = "space", width = 4 }  # editor.* を上書き
comment         = { line = "//", block = ["/*", "*/"] }
auto_pairs      = { "(" = ")", "[" = "]", "{" = "}", '"' = '"' }
word_separators = ""                             # 空なら editor.word_separators
grammar         = "rust"                         # tree-sitter 文法名 (v0.4)。省略時 name
highlight_query = "builtin"                      # "builtin" | ファイルパス (ユーザー層 queries/rust/highlights.scm)
language_servers = ["rust-analyzer"]             # [language-server.*] の名前 (v1.L)。順序 = 優先順
formatter        = "rustfmt"                     # [formatter.*] の名前 (v0.4)。省略時は LSP の format (v1.L)
format_on_save   = false
roots            = ["Cargo.toml"]                # LSP ワークスペースルート検出の目印
max_line_length  = 100                           # ruler に反映 (EditorConfig と連携)
```

| 項目 | 安全性 | 層 | 備考 |
|---|---|---|---|
| `file_types` / `file_names` / `file_globs` / `first_line` / `indent` / `comment` / `auto_pairs` / `word_separators` / `max_line_length` / `format_on_save` | **安全** (データのみ) | 全層 | `first_line` の正規表現は `regex` crate (線形時間) で、長さ上限 256、先頭 1 KB にのみ適用 |
| `grammar` / `highlight_query` | 準安全 | ユーザー層以上 (ワークスペースのクエリファイルは信頼後) | クエリは tree-sitter が解釈するデータだが、複雑なクエリは性能攻撃になり得るため信頼を要する |
| `language_servers` / `formatter` (名前の参照) | 安全 (参照のみ) | 全層 | 参照先の **定義** (コマンド行) が危険項目 |
| `[language-server.<name>]` / `[formatter.<name>]` の `command` / `args` / `env` | **危険** (プロセス起動) | ユーザー層以上。ワークスペース層は **信頼後のみ** (NFR-502) | §12.3 |

### 9.2 言語判定の順序

1. ユーザーが明示指定 (`lang.set` コマンド、セッション層。文書単位で記憶)
2. `file_names` 完全一致 → `file_globs` → `file_types` (拡張子。複数拡張子 `.d.ts` は長い方を優先) — 複数の言語が一致する場合は **ワークスペース層 > ユーザー層 > 組込み** の定義元を優先し、同層なら定義順で後勝ち + 警告
3. `first_line` (shebang 等)。ファイル先頭 1 KB にのみ適用
4. 一致なし → `plain` (プレーンテキスト。ハイライトなし)

**モードライン (`# vim: ft=python`, `-*- mode: python -*-`) は読まない** (v0.1/v0.4)。言語名だけなら無害に見えるが「ファイル内容が設定を変える」経路を 1 つも作らない原則 (NFR-502) を優先する。「ファイルタイプ名のみ、許可リスト内」に限った対応は未決 (§20 OQ-96)。巨大ファイル (FR-123) では言語判定は行うが文法・LSP は起動しない (`doc.large`)。

### 9.3 初期対応言語 (FR-802 の確定)

| 言語 ID | 拡張子 / 名前 (抜粋) | 文法同梱 (v0.4) | LSP 既定 (v1.L、定義のみ同梱。実行は信頼後) |
|---|---|---|---|
| `plain` | 既定 | — | — |
| `rust` | rs | 静的 | rust-analyzer |
| `markdown` | md, markdown | 静的 (+ inline) | — |
| `toml` | toml, Cargo.lock | 静的 | taplo (任意) |
| `json` | json, jsonc, json5 (別 ID `jsonc`) | 静的 | — |
| `yaml` | yml, yaml | 静的 | yaml-language-server (任意) |
| `python` | py, pyi; shebang python | 静的 | pyright / pylsp (任意) |
| `javascript` / `typescript` / `tsx` / `jsx` | js, mjs, cjs / ts, mts / tsx / jsx | 静的 (typescript は大きい → OQ-22) | typescript-language-server |
| `c` / `cpp` | c, h / cc, cpp, cxx, hpp, hh | 静的 | clangd |
| `go` | go | 静的 | gopls |
| `bash` | sh, bash, zsh (別 ID `zsh` は bash 文法で代替); shebang | 静的 | bash-language-server (任意) |
| `html` / `css` | html, htm / css | 静的 (v0.4 Should) | — |
| `diff` / `gitcommit` / `gitrebase` | patch, diff / COMMIT_EDITMSG / git-rebase-todo | 静的 (小) | — |
| `ini` / `dockerfile` / `make` | ini, cfg, .editorconfig / Dockerfile / Makefile | 静的 (小) | — |

`gitcommit` / `gitrebase` / `diff` は `$GIT_EDITOR` 利用 (U1、§18.6) のための追加。合計 20 文法前後で NFR-205 (v0.4 30 MB) に収まるかは [OQ-22](10-decisions.md#oq-22-tree-sitter-文法の同梱方式) と v0.4 の計測で判断し、超える場合は `typescript` / `cpp` を外部ロード候補にする。

### 9.4 言語サーバ定義 (書式。意味・プロトコルは eve-lsp)

```toml
[language-server.rust-analyzer]
command = "rust-analyzer"                 # PATH 検索。絶対パス可。shell 経由では実行しない
args    = []
env     = { RA_LOG = "error" }            # 追加環境変数 (信頼後のみ)
config  = { cargo = { features = "all" } }   # initializationOptions / workspace/configuration
timeout_ms = 10000                        # 起動待ち
enabled = true
```

- `command` / `args` は **配列で渡し、シェルを介さない** (インジェクション対策)。`${workspace_root}` `${file}` `${file_dir}` `${config_dir}` の変数展開のみ許可し、展開結果がワークスペース外を指す `${workspace_root}/../x` のような相対参照は許容 (ユーザーの意図) するが、ワークスペース層由来の定義では **ワークスペース内またはPATH 名のみ** に制限する。
- ワークスペース層で `[language-server.*]` を定義・変更するには信頼が必要。信頼時の確認ダイアログは **実行されるコマンド行をそのまま表示** する (`command` は 05 §13.3 の探索規則で解決した **絶対パス** を併記: カレントディレクトリ・PATH の相対要素は探索しないため、リポジトリ内に置かれた同名バイナリは選ばれない)。あわせて「**このツールはプロジェクト内のコード (build.rs / proc-macro / 設定ファイルのプラグイン等) を実行する可能性があります**」を表示する (rust-analyzer・prettier・ESLint 等は実行ファイルが信頼できてもリポジトリの内容を実行する)。

### 9.5 フォーマッタ定義

```toml
[formatter.rustfmt]
command = "rustfmt"
args    = ["--edition", "2021"]
stdin   = true                   # true: 標準入力に文書、標準出力を結果 (既定)
# stdin = false の場合は ${file} を書き換えさせる方式。元ファイルの原子的保存規則を適用できないため非推奨 (警告)
timeout_ms = 5000
```

実行は `ProcessRunner` Port ([02 §4.4](02-architecture.md#44-ports-core-が定義し外側が実装する-trait)) 経由で、FR-709 どおり **実行前にユーザー確認** (ワークスペースごと・コマンドごとに「今後確認しない」を選べる。信頼済みワークスペースではこの選択を記憶)。確認文言には §9.4 と同じ「プロジェクト内のコードを実行する可能性」の注意と、解決済みの絶対パスを含める。**`format_on_save = true` は未信頼ワークスペースでは無効** (: ワークスペース層の `[[language]]` は安全項目だが、保存のたびにユーザー定義フォーマッタを起動する組合せは信頼後のみ。未信頼時は情報診断)。結果は差分 (ChangeSet) に変換し `Transaction(Origin::Command("lang.format"))` として 1 Undo 単位で適用。終了コード非 0 または標準出力が空の場合は適用しない (フォーマッタ失敗で文書を空にしない、v0.1)。

---

## 10. EditorConfig 対応

### 10.1 対応範囲 (FR-705、v0.4)

| プロパティ | 対応 | 写像先 |
|---|---|---|
| `indent_style` | ○ | `editor.indent_unit` (`tab` → `"tab"`, `space` → `"space"`) |
| `indent_size` | ○ (`tab` は tab_width に従う) | `editor.indent_width` |
| `tab_width` | ○ | `editor.tab_width` |
| `end_of_line` | ○ (**新規ファイルのみ**) | `file.default_line_ending`。既存ファイルの改行コードは判定値を維持 (FR-113、P1 原則) |
| `charset` | ○ (**新規ファイルのみ**) | `file.default_encoding`。`utf-8-bom` は BOM 付き。写像できる値は `file.default_encoding` の許容値 (§5.2) だけで、それ以外 (`latin1` 等) は写像せず情報診断 (v0.3 で許容値が広がればその分だけ増える)。既存ファイルは判定値を維持し、不一致なら情報メッセージ |
| `trim_trailing_whitespace` | ○ | `editor.trim_trailing_whitespace_on_save` |
| `insert_final_newline` | ○ | `editor.insert_final_newline_on_save` |
| `max_line_length` | ○ | `view.rulers` に追加 (`off` で無し) |
| `root` | ○ | 探索停止 |
| その他 (`spelling_language` 等) / ドメイン固有 (`ij_*`, `csharp_*`) | 無視 (診断なし) | — |

### 10.2 優先順位

同じ意味の値について: **組込み/ユーザー層の `[[language]]` と `[editor]`** < **`.editorconfig`** < **ワークスペース層 `.eve/config.toml` の `[[language]]`** < コマンドライン < セッション。理由: `.editorconfig` は「プロジェクトの意図」でありユーザー既定より強いが、同じプロジェクトの evoeditor 専用設定はさらに具体的。

### 10.3 実装方針

- 探索: 文書のディレクトリから上位へ `.editorconfig` を集め (`root = true` または FS ルートで停止)、ファイルパスに対するセクション glob を評価。ファイル収集は bin/platform、解釈は `eve-config` (INI 風パーサ + EditorConfig glob)。
- **安全な設定項目のみ** を含むため、ワークスペースの信頼を要しない (NFR-502 の「安全な項目」の代表例)。
- 依存: `.editorconfig` の glob (`{a,b}`, `[!x]`, `**`, 数値範囲 `{1..5}`) は gitignore 構文と異なる。候補 `editorconfig-rs` (参照実装の Rust 版、依存少) または自前 200 行程度。[02 §13.0](02-architecture.md#130-依存追加の評価基準) に従い v0.4 着手時に評価する。
- 文書を開くたびに評価する (キャッシュはディレクトリ単位、`FileWatcher` で無効化)。祖先探索はジョブで非同期に行う。ホットリロードの分類は「新規文書から」。
- `.editorconfig` の glob 評価は非信頼データのパースなので fuzz の対象にする (09)。

---

## 11. コマンド体系

### 11.1 ID の命名規約 ([02 §6.2](02-architecture.md#62-命名規約-案) の具体化)

```
command_id := namespace "." verb ( "_" object )*
namespace  := [a-z][a-z0-9]* ( "." [a-z][a-z0-9]* )?     ; 2 段まで (ext.<plugin_id> のため)
verb       := [a-z][a-z0-9_]*
```

| 名前空間 | 内容 | 例 |
|---|---|---|
| `app` | プロセス全体 | `app.quit`, `app.suspend`, `app.restart` |
| `app` (続き) | | `app.save_and_quit` (§18.6), `app.quit_with_error` (§18.4) |
| `file` | ファイル I/O | `file.open`, `file.new`, `file.save`, `file.save_as`, `file.save_all`, `file.reload`, `file.check_external_changes`, `file.reopen_with_encoding`, `file.set_encoding`, `file.set_line_ending`, `file.toggle_read_only` |
| `buffer` | Document の一覧・切替・閉じる | `buffer.next`, `buffer.prev`, `buffer.close`, `buffer.list` |
| `edit` | 文書変更 | `edit.undo`, `edit.redo`, `edit.insert_text`, `edit.newline`, `edit.delete_backward`, `edit.delete_forward`, `edit.delete_word_backward`, `edit.indent`, `edit.outdent`, `edit.duplicate_line`, `edit.delete_line`, `edit.move_line_up`, `edit.toggle_comment`, `edit.paste`, `edit.transform_upper`, `edit.convert_indentation` (v0.4、03 §15), `edit.filter_through` (外部フィルタ、v0.4、§15。予約) |
| `cursor` | 移動 (選択を解除) | `cursor.move_left`, `cursor.move_right`, `cursor.move_up`, `cursor.move_down`, `cursor.move_word_left`, `cursor.move_word_right`, `cursor.home` (スマートホーム、03 §5.4), `cursor.end`, `cursor.doc_start`, `cursor.doc_end`, `cursor.page_up`, `cursor.page_down`, `cursor.goto_line`。**移動の動詞は `move_` 接頭を正とする** (`cursor.left` のような接頭なしの表記は採らない) |
| `select` | 選択の拡張・操作 | `select.extend_left` 等 (`cursor` の移動ごとの `extend_` 版。一覧は §11.6), `select.all`, `select.extend_to_doc_start`, `select.extend_to_doc_end`, `select.add_next_match` (v0.4), `select.line` (v0.4) |
| `search` | 検索・置換 | `search.open`, `search.next`, `search.prev`, `search.toggle_case`, `search.toggle_regex`, `search.replace_open`, `search.replace`, `search.replace_all`, `search.workspace` (v0.4) |
| `view` | View の表示属性 | `view.toggle_wrap`, `view.scroll_down`, `view.toggle_line_numbers` |
| `pane` | 分割ペイン (1 つの端末画面の中を分割する。04 §11) の作成・フォーカス移動 (v0.4) | `pane.split_vertical`, `pane.focus_next`, `pane.close` |
| `ui` | オーバーレイ・パネル・汎用 UI | `ui.palette_open`, `ui.cancel` (Esc の意味は 04 §7.5), `ui.confirm`, `ui.list_next`, `ui.dialog_choose`, `ui.message_history`, `ui.toggle_hint_bar`, `ui.toggle_mouse`, `ui.redraw`, `ui.probe_char_width` (Could) |
| `config` / `theme` / `keys` | 設定操作 | `config.reload`, `config.open_user`, `config.show`, `config.show_diagnostics`, `config.set`, `theme.select`, `keys.set_preset`, `keys.bind` |
| `lang` | 言語・構文 | `lang.set`, `lang.redetect`, `lang.format`, `lang.select_node`, `lang.run_command` (外部コマンドの出力をパネルに表示、Could v0.4、§15。予約) |
| `lsp` | 言語サーバ (v1.L) | `lsp.goto_definition`, `lsp.hover`, `lsp.rename`, `lsp.restart` |
| `ai` | AI (v1.L)。ID は v1.L 着手時に本書 §11.6 のカタログへ採録する (06 §13.2 は案) | `ai.propose_edit`, `ai.chat_open`, `ai.review_accept_hunk`, `ai.review_accept_all`, `ai.review_reject_hunk`, `ai.review_apply`, `ai.cancel`。ローカル LLM 向け (予約): `ai.provider_check` (サーバの種類・モデル・能力・送信先の表示)、`ai.profile_select`、`ai.runtime_start` / `ai.runtime_stop` / `ai.runtime_restart` / `ai.runtime_log` (管理モード、v1.L-b)、`ai.complete_toggle` (補完のセッション内切替) |
| `workspace` | ワークスペース | `workspace.open_file` (ファインダ), `workspace.trust`, `workspace.untrust`, `workspace.show_trust` (信頼状態と保留項目の表示、§12.4。v0.4、予約) |
| `session` | セッション | `session.restore`, `session.save` |
| `macro` | マクロ (v1.L) | `macro.record`, `macro.play` |
| `help` | ヘルプ | `help.open`, `help.keys`, `help.commands`, `help.about` |
| `clipboard` | — | `clipboard.copy`, `clipboard.cut` (edit.paste は変更を伴うので edit) |
| `internal` | パレット非表示。テスト・内部遷移 | `internal.job_cancel`, `internal.debug_panic` (`test-hooks` のビルドで `EVE_TEST_HOOK=panic_command` のときだけ、§11.6) |
| `ext.<plugin_id>` | プラグイン (v1.X) | `ext.spellcheck.toggle` |
| `debug` | 開発者向け (リリースビルドでは `--debug-commands` で有効化) | `debug.dump_frame` |

規則: 動詞は命令形・小文字スネークケース。トグルは `toggle_<x>`、開く UI は `<x>_open`、設定変更は `set_<x>`。**ID は公開 API**: 変更時は旧 ID を alias として最低 2 マイナーリリース残し、使用時に情報診断を出す。`CommandRegistry` は alias を解決してから実行する。

### 11.2 引数 (`ArgSchema`)

| 型 | TOML/パレット表記 | 例 |
|---|---|---|
| `bool` | `true`/`false` | `args = { select = true }` |
| `int` / `float` | 数値 | `args = { count = 5 }` |
| `int\|"$"` | 数値、または文字列 `"$"` (最終行・末尾を表す。`cursor.goto_line` の `line` だけが使う) | `args = { line = "$" }` |
| `string` | 文字列 | `args = { text = "\t" }` |
| `enum(<値…>)` | 文字列 (許容値検証) | `args = { direction = "forward" }` |
| `path` | 文字列 (変数展開あり) | `args = { path = "${workspace_root}/README.md" }` |
| `command_id` | 文字列 | (マクロ・ラッパー用) |
| `pos` / `range` | `{ line, col }` (1 始まり) | 内部・プラグイン用 |
| `list<T>` | 配列 | `args = { paths = [...] }` |

各引数は `required` / `default` / `description` (MessageKey) を持つ。キーマップで不足した必須引数は **設定診断でエラー**。**パレットから実行するとき (v0.1 から)**: パレットは `hidden` でない全コマンドを出し (§11.6)、選んだコマンドに **既定値の無い必須引数** があれば、`ArgSchema` に基づき同じオーバーレイ内で順に尋ねる。列挙値 (`enum`) は値のリスト (例 `file.reopen_with_encoding` → 文字コードの一覧)、それ以外はプロンプトで尋ね、`Esc` で 1 段戻る。省略可能な引数 (`path?` 等) と既定値付きの引数は尋ねず、各コマンドの既定の動作 (例 `file.open` はパス入力のプロンプトを開く) に任せる。v0.4 で補完付きの入力 (パスの候補等) を加える。`count` という名前の int 引数は慣例としてプレフィックス回数 (Vim 層) にも使う。

### 11.3 CommandSpec の公開面 (`eve-core`、[02 §6.1](02-architecture.md#61-基本方針-adr-0007))

本書が関与する項目: `title` (MessageKey、§17)、`description` (MessageKey、パレットとヘルプ用。`hidden` でないコマンドは同梱の言語ファイル (v0.1 は `ja`) で必須、§17.1)、`args`、`when` (§7.3 と同じ文法)、`kind`、`permission` (§14.3)、`hidden` (パレット非表示。`internal.*` は暗黙)、`aliases`、`since` (導入バージョン。ヘルプと互換性表示用)。

### 11.4 コマンドパレット (FR-602) の登録と表示

- 登録 = `CommandRegistry::register`。組込みは起動時に定数表から、プラグインは接続時に manifest から (v1.X)。
- 照合の対象: title とローカライズ前の英語 title と ID の 3 つ (日本語 UI でも英語名で探せる)。
- 表示順: **v0.1** は前方一致 → 部分一致 → アルファベット順 (CommandId 順。ファジー一致・最近実行は使わない。入力欄が空のときは全コマンドをアルファベット順。01 FR-602、04 §8.1)。**v0.2 から** ファジー一致スコア → 最近実行 (直近 20 件をセッション内で記憶。入力欄が空のときは最近実行 → アルファベット順) → アルファベット順。
- 各行: `タイトル    (id)    キー`。`when` が偽のコマンドは **表示するが淡色** にして「現在は実行できません: <理由>」を示す (隠すと「存在しない」と誤解する。P7)。
- プレフィックス記法は導入しない (`>` でコマンド、`:` で行番号などの VS Code 風)。行番号移動は `cursor.goto_line` を別オーバーレイで提供。

### 11.5 ヘルプの自動生成 (FR-514)

| 出力 | 元データ | 版 |
|---|---|---|
| コマンドパレットの各行 (タイトル・CommandId・届く割当キー、04 §8.1)。v0.1 のヘルプ画面の代用 (FR-514) | 解決済みキーマップ + CommandRegistry | v0.1 |
| `help.keys`: 現在のコンテキストで有効なキー一覧 (キー → タイトル) | 解決済みキーマップ + CommandSpec.title | v0.2 |
| `help.commands`: 全コマンドの一覧 (名前空間ごと、タイトル・説明・キー・`since`) | CommandRegistry | v0.2 |
| `evoeditor --list-commands [--json]` / `--dump-keymap [--json]` | 同上。ドキュメント生成・シェル補完・外部ツール用 | v0.1 |
| ユーザーガイドのキー一覧表 (リポジトリ docs) | `--list-commands --json` から CI で生成し、§11.6 カタログと差分が出たら CI 失敗 (どちらを直すかは判断、09 §11.2) | v0.1 (09) |
| 設定リファレンス | §4.1 のメタ表 | v0.4 |

### 11.6 v0.1 CommandId カタログ (付録)

**本表が CommandId・既定キー割当・コンテキスト・ヒントの正本** (ADR-0025)。**title key** 列の MessageKey (`cmd.<command_id>`) は、同梱の `ja` の言語ファイルに `title` と `description` を持つ (§17。CommandId 自体は翻訳しない。パレットの照合用の英語の title は定数表に持つ、§11.4)。04 §7.2 は本表の「既定キー」列の設計入力 (端末上の癖・代替割当の根拠)、04 §7.3a は端末の横取り表、03 は ID を引用する。`evoeditor --list-commands` / `--dump-keymap` の生成物と CI で突合する (09 §11.2)。**v0.1 で実装するのは本表の ID のうち「(v0.2)」等の注記が無いもの** だけで、注記付きの ID は名前空間を予約し実装は当該版 (ADR-0028、ADR-0039)。v0.1 で実装する ID は **63 個** (確定 (ユーザー回答)、v0.1 の範囲は案 L で、再読込・改行コードの変換・リテラルのすべて置換 (置換欄を開くコマンドを含む) を含む。下の表の合計。ファイル・アプリ 11、編集 14、移動・選択 26、検索・表示 6、オーバーレイ内 6。テストビルド専用の `internal.debug_panic` は数えない)。このうちパレットに出るのは `hidden` の 6 個を除く 57 個。版注記付きの ID は 25 個。版注記付きの行の「既定キー」「ヒント」列は、その版での割当と文脈を示す (予約。v0.1 ではそのキーを他の機能に使わない)。

凡例:

- **kind** = Edit / Motion / Ui / File / External (02 §6.3)。**perm** = AI・プラグイン・**External (駆動 API / sidecar、X16)** からの呼出し可否 (02 §10.4): `-` 不可 / `propose` (結果が Proposal 経由) / `read` (読み取りのみ) / `ui`。v0.1 の権限判定フックは User 主体だけを扱い、External 主体は変種の予約のみ (駆動 API と同じ版で実装、§14.3)。
- **コンテキスト** = その割当が属するキーマップのコンテキスト (§7.1。参照順も §7.1)。コンテキスト列に `パレット` と書いた行は **キーマップのコンテキストではなく**、既定キーを持たずパレットから実行することを表す (既定キー列は `—`)。
- **既定キー** は §7.2 の設定表記 (`mod` = TUI では `ctrl`)。英字との組は `mod` (例 `mod+s`)、特殊キー (`home` / `end` / `insert` / `pageup` / `pagedown` / 矢印 / `backspace` / `delete` / `tab` 等) との組は `ctrl` 固定 (例 `ctrl+home`、`ctrl+insert`)。**`ctrl` を直書きする英字との組 (上の規則の例外。`mod` に解決せず、どの frontend / OS でも物理 Ctrl のまま)**: (1) `ctrl+k` を先頭ストロークとするチョードの代替割当 (`ctrl+k s` 等。`(alt)`)、(2) `list` コンテキストの `ctrl+n` / `ctrl+p` (候補の移動。Emacs / readline 由来の端末の慣習)、(3) `ctrl+h` (`search.replace_open` の別名。レガシー VT の端末では `ctrl+backspace` と同じ符号 (0x08) で、`[E]` のときだけ区別できる。正規化で畳まれて届かない端末ではこの割当は無効、04 §6.2)。これら以外で英字と `ctrl` を直書きする既定キーは作らない (CI の突合で検出する)。コンテキストごとに割当が違う行は `コンテキスト: キー` の形で書く。`[E]` = 拡張キーボードプロトコル (v0.2 から push) または Windows のコンソール API でのみ区別できる。`[横]` = 04 §7.3a の横取り表で、対象端末のどれかが横取りする (その端末では届かない)。`(alt)` = 代替割当 (全端末で届く。04 §7.3a の横取り表を考慮済み)。既定キー列にはキーだけを書き、キーでない手段 (パレットからの実行、別のコマンドを使った案内) は括弧の注記で書く。**キーの記載順** は、ヒントバーと `{key:<command_id>}` (§17.2) に出す割当を選ぶ順でもある: 端末で届くと分かっている割当のうち最初のものを出す (拡張キーが無ければ `[E]` を除き、端末の横取り表に載るキーを除く。02 §7.1 `Capabilities.intercepted_keys`)。
- **ヒント** = ヒントバーに出す文脈と優先度 (`文脈:優先度`)。**v0.1 の文脈は `normal` (編集面) と `overlay` (パレット・入力プロンプト・確認ダイアログ・検索バー・チョード保留中で共通) の 2 つ** (04 §8.8)。`overlay` では、`when` が偽の項目と、その場面のコンテキスト (§7.1) で届く割当が無い項目を出さないので、場面ごとに出る項目は変わる。版注記付きの行に書いた `palette` / `prompt` / `confirm` / `search` / `chord` / `help` は v0.2 で細分する文脈。優先度は **大きいほど残る** (5 = 常に残す。02 §8.2 `HintItem.priority`)。表示順は優先度の降順、同じ優先度は本表の行順。各文脈で抜け出す手段 (`normal` は終了とパレット、`overlay` は `Esc`) を 5 にする。ヒントの語は `hint.<command_id>` (§17.2、8 セル以内。語は 04 §15.1)。`-` = 出さない。
- **表の記法 (機械可読。11 §8.5 のパーサが読む規則)**: 本表は 11 §8.5 の規則 (識別子はバッククォート、1 つのバッククォートに 1 つの完全な識別子、ワイルドカード・略記なし) に加え、次の記法に従う。
  - **版の注記は ID 列だけに、行全体の版として書く** (`(v0.2)` 等。読む側はその版の行として扱う)。**版の注記付きの行は、実装した版から登録してよい**: 登録済みのコマンドはカタログのどれかの行に載っている (注記付きでもよい)ことを検査し、注記の無い行 (v0.1) は必ず登録されていることを検査する。注記付きの行の言語ファイルの `cmd.*` / `hint.*` は、あっても余剰とせず、無くてもよい (`xtask i18n`)。注記は、その版を出荷するときに外す。行の一部の機能だけが後の版になる場合 (例 `edit.indent` の選択範囲) は ID 列に書かず、既定キー列の括弧の注記 (`(版の注記: …)`) に書く。
  - **複数 ID の行**: ID 列で ` / ` で区切った ID と、title key 列・既定キー列で ` / ` で区切った値は、左から順に 1 対 1 で対応する (`` `mod+z` / `mod+y` `` は `edit.undo` / `edit.redo`)。
  - **既定キー列**: 1 つのキーは設定表記をバッククォートで囲んだもの。キーの直後に付く `[E]`・`[横]`・`(alt)` はそのキーの属性で、複数あるときは `[E][横]` の順に空白なしで続ける。**対 (` / ` でつないだ複数 ID の割当) の属性**: 対の最後のキーの直後にだけ属性があるときは、その属性は **対のすべてのキーに及ぶ** (`` `alt+up` / `alt+down` [横] `` は両方が `[横]`、`` `ctrl+k left` / `ctrl+k right` (alt) `` は両方が `(alt)`)。対の途中のキーにも属性があるときは、各キーが自分の直後の属性だけを持つ (`` `ctrl+insert` [横] / `shift+delete` (alt) `` は前が `[横]`、後ろが `(alt)`)。**片方の ID にだけ付く割当** (対の 2 つ目だけに別名のキーがある等) は、その割当の直後に `(<ID の最後の語> のみ)` を付けて示し、対の位置では数えない (`` `mod+shift+z` [E] (redo のみ) `` は `edit.redo` だけの割当で、`[E]` もそのキーの属性)。同じ ID (または同じ位置の ID の組) の割当の間は `, ` で区切り、記載順は上の「キーの記載順」。コンテキストごとに割当が違う行は、`` `<コンテキスト>`: `` で始まる組を `、` (読点) で区切って並べる (例 `` `list`: `down` / `up`, `ctrl+n` / `ctrl+p`、`dialog`: `tab` / `shift+tab` ``)。`` `<コンテキスト>`: `` の無い行の割当は、コンテキスト列のすべてのコンテキストに属する。
  - **括弧書きは注記**: どの列でも `(…)` の中は説明で、読む側は中のバッククォートも識別子として読まない (例外は `(alt)` だけ。上の属性)。`—` は「無い」。
  - **頻出操作リスト (FR-606)**: 本節末尾の、書き出しが `**FR-606 の到達性検査**` (太字。この文字列を変えない) の段落の **直後の箇条書き** を、11 §8.5 のパーサがこの固定の書き出しで探して読む (見出しではなく段落の書き出しで位置を決める)。表ではなく箇条書きで、各行は `- <分類>: ` に続けて CommandId をバッククォートで囲んで `, ` で区切る。箇条書きの後の段落 (対象に含めないもの・合格の条件) は読まない。
  - **ヒント列**: `<文脈>:<優先度>` (コロンの前後に空白なし、優先度は 1〜5 の整数) を `, ` で区切って並べる。`-` は出さない。複数 ID の行で一部の ID だけに付くときは、直後に `(<ID の最後の語> のみ)` を付ける (例 `normal:2 (undo のみ)`)。
- **パレット非表示** (`hidden`): `internal.*` と、オーバーレイ内専用の `ui.confirm` / `ui.list_next` / `ui.list_prev` / `ui.list_page_down` / `ui.list_page_up` / `ui.dialog_choose`。これ以外はすべてパレットに出る (FR-601 AC-1、§11.2 の引数の尋ね方)。
- v1.L 以降の `ai.*` / `lsp.*` は着手時に 06 §13.2 の案 / eve-lsp 文書から本表へ採録し、採録した時点で本表が正本になる (`ai.*` の名前空間と、ローカル LLM 向けの ID `ai.provider_check` / `ai.profile_select` / `ai.runtime_start` / `ai.runtime_stop` / `ai.runtime_restart` / `ai.runtime_log` / `ai.complete_toggle` / `ai.complete_accept` / `ai.complete_dismiss` / `ai.cancel` は v0.1 で **予約** し、他の名前空間で使わない)。

**ファイル・アプリ** (v0.1 の ID 11 個)

| ID | title key | kind | args | when | perm | コンテキスト | 既定キー | ヒント |
|---|---|---|---|---|---|---|---|---|
| `file.save` | `cmd.file.save` | File | — | `!doc.read_only` | `-` | `editor`, `search` | `mod+s` | normal:4 |
| `file.save_as` | `cmd.file.save_as` | File | `path?` | — | `-` | `editor` | `mod+shift+s` [E], `ctrl+k s` (alt) | - |
| `file.save_all` (v0.2) | `cmd.file.save_all` | File | — | — | `-` | `editor` | `ctrl+k a` (alt) | chord:3 |
| `file.open` | `cmd.file.open` | File | `path?` | — | `-` | `editor` | `mod+o` | normal:2 |
| `file.new` | `cmd.file.new` | File | — | — | `-` | `editor` | `mod+n` | - |
| `file.reload` | `cmd.file.reload` | File | — | `doc.has_path` | `-` | パレット | — | - |
| `file.check_external_changes` (v0.2) | `cmd.file.check_external_changes` | File | — | `doc.has_path` | `-` | パレット | — | - |
| `file.reopen_with_encoding` | `cmd.file.reopen_with_encoding` | File | `encoding` (列挙。パレットではリストで尋ねる) | `doc.has_path` | `-` | パレット | — | - |
| `file.set_encoding` | `cmd.file.set_encoding` | File | `encoding` (列挙。同上), `bom?` (bool。既定: `encoding` が UTF-16LE / UTF-16BE なら `true` (BOM 付き)、UTF-8 なら今の文書の BOM の有無を保つ、それ以外の文字コードは BOM を持たないので無視) | — | `-` | パレット | — | - |
| `file.set_line_ending` | `cmd.file.set_line_ending` | Edit | `eol` (列挙 `lf` / `crlf` / `cr`。パレットではリストで尋ねる) | `!doc.read_only` | `propose` | パレット | — | - |
| `file.toggle_read_only` (v0.2) | `cmd.file.toggle_read_only` | File | — | `doc.read_only_releasable \|\| !doc.read_only` | `-` | パレット | — | - |
| `buffer.close` (v0.2) | `cmd.buffer.close` | Ui | — | `editor`: — / `list`: `overlay.active == "buffer_list"` (選択中の項目を閉じる) | `-` | `editor`, `list` | `editor`: `mod+w`, `ctrl+k w` (alt)、`list`: `mod+w` | chord:3, palette:2 |
| `buffer.list` (v0.2) | `cmd.buffer.list` | Ui | — | — | `ui` | `editor` | `mod+e` | normal:1 |
| `buffer.next` / `buffer.prev` (v0.2) | `cmd.buffer.next` / `cmd.buffer.prev` | Ui | — | — | `ui` | `editor` | `ctrl+pagedown` / `ctrl+pageup`, `ctrl+k n` / `ctrl+k p` (alt) | - |
| `app.quit` | `cmd.app.quit` | Ui | — | — | `-` | `global` | `mod+q` | normal:5 |
| `app.save_and_quit` (v0.2) | `cmd.app.save_and_quit` | File | — | — | `-` | パレット | — | - |
| `app.quit_with_error` (v0.2) | `cmd.app.quit_with_error` | Ui | — | — | `-` | パレット | — | - |
| `app.suspend` (v0.2) | `cmd.app.suspend` | Ui | — | `caps.can_suspend` | `-` | `editor` | `ctrl+k z` (alt) | - |
| `ui.palette_open` | `cmd.ui.palette_open` | Ui | — | — | `-` | `global` | `mod+p`, `f1`, `mod+shift+p` [E][横] (Windows Terminal がコマンドパレットに使う、04 §7.3a) | normal:5 |
| `help.open` (v0.2) | `cmd.help.open` | Ui | — | — | `ui` | `global` | `ctrl+k h` (alt) (`f1` をヘルプに移すかは v0.2 で決める。提案中) | normal:5, palette:3, prompt:3, search:1, chord:3 |
| `help.keys` / `help.commands` (v0.2) | `cmd.help.keys` / `cmd.help.commands` | Ui | — | — | `ui` | パレット (ヘルプの該当の節を開く、FR-514) | — | - |
| `ui.message_history` (v0.2) | `cmd.ui.message_history` | Ui | — | — | `ui` | `editor` | `ctrl+k m` (alt) | - |
| `ui.redraw` | `cmd.ui.redraw` | Ui | — | — | `-` | `global` | `ctrl+k r` (alt) | - |
| `ui.toggle_hint_bar` (v0.2) | `cmd.ui.toggle_hint_bar` | Ui | — | — | `ui` | パレット | — | - |
| `ui.toggle_mouse` (v0.2) | `cmd.ui.toggle_mouse` | Ui | — | `frontend == "tui"` | `-` | パレット | — | - |
| `config.show_diagnostics` (v0.2) | `cmd.config.show_diagnostics` | Ui | — | — | `ui` | パレット | — | - |
| `config.reload` (v0.2) | `cmd.config.reload` | Ui | — | — | `-` | パレット | — | - |

**編集** (v0.1 の ID 14 個)

| ID | title key | kind | args | when | perm | コンテキスト | 既定キー | ヒント |
|---|---|---|---|---|---|---|---|---|
| `edit.insert_text` | `cmd.edit.insert_text` | Edit | `text` | `!doc.read_only` | `propose` | `editor`, `prompt` (欄への入力) | (Text 入力。高速経路) | - |
| `edit.newline` | `cmd.edit.newline` | Edit | — | `!doc.read_only` | `propose` | `editor` | `enter` | - |
| `edit.undo` / `edit.redo` | `cmd.edit.undo` / `cmd.edit.redo` | Edit | — | `!doc.read_only` | `-` | `editor` | `mod+z` / `mod+y`, `mod+shift+z` [E] (redo のみ) | normal:2 (undo のみ) |
| `clipboard.copy` / `clipboard.cut` | `cmd.clipboard.copy` / `cmd.clipboard.cut` | Edit (cut) / Ui (copy) | — | `!selection.empty` (cut: `&& !doc.read_only`)。選択が空のときは何もしない (通知なし。行全体のコピー・切り取りには広げない)。キー操作では `when` が偽なのでこの割当に当たらず、ほかに当たる割当が無ければキーは捨てられる (02 §7.2)。パレットでは実行できない項目として出る | `-` | `editor` | `mod+c` / `mod+x`, `ctrl+insert` [横] / `shift+delete` (alt) | - |
| `edit.paste` | `cmd.edit.paste` | Edit | — | `!doc.read_only` | `-` | `editor`, `prompt` | `mod+v`, `shift+insert` (alt) (Windows Terminal ではどちらも端末の貼り付けになり、`Paste` (または個別キーのバースト) として届く。結果は同じ、04 §7.3a) | - |
| `select.all` | `cmd.select.all` | Motion | — | — | `read` | `editor`, `prompt` (欄内全選択) | `mod+a` | - |
| `edit.delete_backward` / `edit.delete_forward` | `cmd.edit.delete_backward` / `cmd.edit.delete_forward` | Edit | — | `!doc.read_only` | `propose` | `editor`, `prompt` | `backspace` / `delete` | - |
| `edit.delete_word_backward` / `edit.delete_word_forward` | `cmd.edit.delete_word_backward` / `cmd.edit.delete_word_forward` | Edit | — | `!doc.read_only` | `propose` | `editor`, `prompt` | `ctrl+backspace` / `ctrl+delete`, `alt+backspace` (alt) (backward のみ)。単語の範囲・空白・行頭行末・選択ありの扱いは 03 §5.3 | - |
| `edit.indent` / `edit.outdent` | `cmd.edit.indent` / `cmd.edit.outdent` | Edit | — | `!doc.read_only` | `propose` | `editor` | `tab` / `shift+tab` (版の注記: 選択範囲へのインデント / アンインデント (`!selection.empty`) は v0.2、FR-208。v0.1 は選択なしの動作だけで、`tab` はタブ / スペースの挿入。**v0.1 で選択ありの `tab` / `shift+tab` は何もしない** (選択を保ち、文書は変わらない)) | - |
| `edit.duplicate_line` (v0.2) | `cmd.edit.duplicate_line` | Edit | — | `!doc.read_only` | `propose` | `editor` | `ctrl+k d` (alt) | - |
| `edit.delete_line` (v0.2) | `cmd.edit.delete_line` | Edit | — | `!doc.read_only` | `propose` | `editor` | `ctrl+k l` (alt), `mod+shift+k` [E] | - |
| `edit.move_line_up` / `edit.move_line_down` (v0.2) | `cmd.edit.move_line_up` / `cmd.edit.move_line_down` | Edit | — | `!doc.read_only` | `propose` | `editor` | `alt+up` / `alt+down` [横], `ctrl+k up` / `ctrl+k down` (alt) | - |

**移動・選択** (v0.1 の ID 26 個。`select.extend_` で始まる ID は対応する移動のキーに `shift+` を付けたもの。when / perm は移動と同じ。`prompt` コンテキストでは入力欄のカーソルを動かす)

| ID | title key | kind | args | when | perm | コンテキスト | 既定キー | ヒント |
|---|---|---|---|---|---|---|---|---|
| `cursor.move_left` / `cursor.move_right` / `cursor.move_up` / `cursor.move_down` | `cmd.cursor.move_left` / `cmd.cursor.move_right` / `cmd.cursor.move_up` / `cmd.cursor.move_down` | Motion | `count?` | — | `read` | `editor`、`prompt` (`left` / `right` のみ) | `left` / `right` / `up` / `down` | - |
| `select.extend_left` / `select.extend_right` / `select.extend_up` / `select.extend_down` | `cmd.select.extend_left` / `cmd.select.extend_right` / `cmd.select.extend_up` / `cmd.select.extend_down` | Motion | `count?` | — | `read` | `editor` | `shift+left` / `shift+right` / `shift+up` / `shift+down` | - |
| `cursor.move_word_left` / `cursor.move_word_right` | `cmd.cursor.move_word_left` / `cmd.cursor.move_word_right` | Motion | `count?` | — | `read` | `editor`, `prompt` | `ctrl+left` / `ctrl+right`, `alt+left` / `alt+right` [横], `alt+b` / `alt+f` (alt) | - |
| `select.extend_word_left` / `select.extend_word_right` | `cmd.select.extend_word_left` / `cmd.select.extend_word_right` | Motion | `count?` | — | `read` | `editor` | `ctrl+shift+left` / `ctrl+shift+right`, `alt+shift+left` / `alt+shift+right` [横] | - |
| `cursor.home` / `cursor.end` | `cmd.cursor.home` / `cmd.cursor.end` | Motion | — | — | `read` | `editor`, `prompt` | `home` / `end`, `ctrl+k left` / `ctrl+k right` (alt) | - |
| `select.extend_home` / `select.extend_end` | `cmd.select.extend_home` / `cmd.select.extend_end` | Motion | — | — | `read` | `editor` | `shift+home` / `shift+end`, `ctrl+k shift+left` / `ctrl+k shift+right` (alt) | - |
| `cursor.doc_start` / `cursor.doc_end` | `cmd.cursor.doc_start` / `cmd.cursor.doc_end` | Motion | — | — | `read` | `editor` | `ctrl+home` / `ctrl+end` (案内: `cursor.goto_line` (`mod+g`) で `1` / `$` を入力しても同じ位置へ移れる。割当ではない) | - |
| `select.extend_to_doc_start` / `select.extend_to_doc_end` | `cmd.select.extend_to_doc_start` / `cmd.select.extend_to_doc_end` | Motion | — | — | `read` | `editor` | `ctrl+shift+home` / `ctrl+shift+end` [E][横] (代替キーなし。届かない端末ではパレットから実行) | - |
| `cursor.page_up` / `cursor.page_down` | `cmd.cursor.page_up` / `cmd.cursor.page_down` | Motion | — | — | `read` | `editor` | `pageup` / `pagedown`, `ctrl+k ,` / `ctrl+k .` (alt) | - |
| `select.extend_page_up` / `select.extend_page_down` | `cmd.select.extend_page_up` / `cmd.select.extend_page_down` | Motion | — | — | `read` | `editor` | `shift+pageup` / `shift+pagedown`, `ctrl+k <` / `ctrl+k >` (alt) | - |
| `cursor.goto_line` | `cmd.cursor.goto_line` | Motion | `line?` (`int` または文字列 `"$"` = 最終行), `col?` (`int`) | — | `read` | `editor` | `mod+g` | normal:1 |
| `ui.cancel` | `cmd.ui.cancel` | Ui | — | — | `-` | `editor`, `search`, `prompt`, `dialog` (v0.2 から `help`) | `esc` | overlay:5 |

**検索・表示** (v0.1 の ID 6 個)

| ID | title key | kind | args | when | perm | コンテキスト | 既定キー | ヒント |
|---|---|---|---|---|---|---|---|---|
| `search.open` | `cmd.search.open` | Ui | `query?` | — | `ui` | `editor`, `search` (欄を全選択) | `mod+f` | normal:3 |
| `search.next` | `cmd.search.next` | Motion | — | `search.active` | `read` | `search` | `enter`, `down`, `f3` | overlay:4 |
| `search.prev` | `cmd.search.prev` | Motion | — | `search.active` | `read` | `search` | `up`, `shift+enter` [E], `shift+f3` [E] | overlay:3 |
| `search.toggle_case` | `cmd.search.toggle_case` | Ui | — | `search.active` | `ui` | `search` | `alt+c`, `ctrl+k c` (alt) | overlay:2 |
| `search.toggle_regex` (v0.2) | `cmd.search.toggle_regex` | Ui | — | `search.active` | `ui` | `search` | `alt+r`, `ctrl+k x` (alt) | search:2 |
| `search.toggle_word` (v0.2) | `cmd.search.toggle_word` | Ui | — | `search.active` | `ui` | `search` | `alt+w`, `ctrl+k b` (alt) | - |
| `search.replace_open` | `cmd.search.replace_open` | Ui | — | `!doc.read_only` | `ui` | `editor`, `search` | `mod+r`, `ctrl+h` [E] | - |
| `search.replace` (v0.2) | `cmd.search.replace` | Edit | — | `search.replace_active && !doc.read_only` | `propose` | `search` (置換欄) | `enter` (置換欄で。v0.1 の置換欄の `enter` は `search.next`。キーマップの `when` は `search.replace_focused`) | search:3 |
| `search.replace_all` | `cmd.search.replace_all` | Edit | — | `search.replace_active && !doc.read_only` | `propose` | `search` | `alt+a` (`tab` で置換欄から `[A] すべて置換` のボタンへ移り `enter` でも実行できる。04 §8.4) | overlay:1 |
| `view.toggle_wrap` (v0.2、OQ-110) | `cmd.view.toggle_wrap` | Ui | — | — | `ui` | `editor` | `alt+z`, `ctrl+k o` (alt) | - |

`search.prev` の主な手段は `up` (`shift+enter` は従来の端末で `enter` と区別できず、`shift+f3` は従来の端末でカーソル位置報告と区別できない)。`search.toggle_case` は大小区別を **smart → sensitive → insensitive → smart** の順に巡回する。開くたびに `search.case` の値 (v0.1 は定数 smart) から始まり、切り替えた値はセッション中だけ保持する (設定ファイルには書かない。意味は 03 §12、表示は 04 §8.4)。`search.toggle_regex` (正規表現の ON / OFF、初期値は `search.regex_default`) は v0.2。`search.replace_open` は検索バーを置換欄つきで開く (検索バーが開いていれば置換欄を足し、置換欄にフォーカスを移す)。`search.replace_all` は v0.1 ではリテラルの一括置換で、大小区別は検索バーの現在の値に従う (手順は 03 §12.5)。置換欄が開いていない (`search.replace_active` が偽) ときは実行できないので、置換文字列を入れないまま `alt+a` で一致を消すことはない。

**オーバーレイ内** (v0.1 の ID 6 個。すべてパレット非表示)

| ID | title key | kind | args | when | perm | コンテキスト | 既定キー | ヒント |
|---|---|---|---|---|---|---|---|---|
| `ui.confirm` | `cmd.ui.confirm` | Ui | — | `overlay.active != ""` | `-` | `prompt`, `dialog` | `enter` (`dialog` ではフォーカス中のボタンを実行) | overlay:4 |
| `ui.list_next` / `ui.list_prev` | `cmd.ui.list_next` / `cmd.ui.list_prev` | Ui | — | `overlay.active != ""` | `-` | `list`, `dialog` (v0.2 から `help`) | `list`: `down` / `up`, `ctrl+n` / `ctrl+p`、`dialog`: `tab` / `shift+tab`, `right` / `left`, `down` / `up` | overlay:3 (next のみ) |
| `ui.list_page_down` / `ui.list_page_up` | `cmd.ui.list_page_down` / `cmd.ui.list_page_up` | Ui | — | `overlay.active != ""` | `-` | `list` (v0.2 から `help`) | `pagedown` / `pageup` | - |
| `ui.dialog_choose` | `cmd.ui.dialog_choose` | Ui | `choice` | `overlay.active == "dialog"` | `-` | `dialog` | (選択肢のアクセラレータ 1 文字。組込みの dialog 層が `Text` を照合する、§7.1。複数書記素の `Text` と `Paste` は先頭の書記素も照合せずすべて捨てる (提案、02 §7.2 K1、OQ-180)) | - |

- `list` では `ctrl+p` = 前、`ctrl+n` = 次。
- `dialog` (確認ダイアログ) のフォーカスは既定ボタンから始まり、`ui.list_next` / `ui.list_prev` でボタン間を移る。`esc` (`ui.cancel`) は常にデータを変えない側。アクセラレータ (`[S]` / `[D]` 等) は大小を区別せず、全角英字 (`Ｄ`) は半角に畳んで照合し、一致しない文字は捨てる。IME が ON で文字が前編集に入るときは `tab` / 矢印 + `enter` で選ぶ (04 §8.3)。
- 入力履歴 (FR-407、v0.4): `prompt` の `alt+up` / `alt+down`。

**内部 (パレット非表示、配布ビルドに無い)**: `internal.debug_panic` (パニックの注入。cargo feature `test-hooks` を有効にしたビルドで、かつ `EVE_TEST_HOOK` に `panic_command` を含むときだけ登録する (09 §3.6 の test-hooks の一覧。どちらかが欠ければ登録しない)。01 NFR-302 AC-1、02 §4.1)。キー割当なし。

**予約 (v0.1 では割り当てない)**: `alt+z` (v0.2 折り返し)、`mod+d` (v0.4 `select.add_next_match`)、`mod+l` (v0.4 `select.line`)、`mod+/` (v0.4 `edit.toggle_comment`)、`mod+\` (v0.4 `pane.split_vertical`)、`mod+t` (v0.4 ファインダ候補)、`ctrl+b` (tmux)、`ctrl+j` (レガシー Enter と紛らわしい)、`ctrl+i` / `ctrl+m` / `ctrl+[` (区別困難)、`ctrl+数字`、`ctrl+k ctrl+<矢印>` (v0.4 ペイン移動)。

**FR-606 の到達性検査** (09 §3.4、OQ-30) の対象「頻出操作」(v0.1 で確定。OQ-30 の分類「保存・終了・検索・パレット・Undo/Redo・コピー/カット/貼り付け・移動・選択」を ID で列挙したもの。計 37 個):

- 保存・終了・パレット: `file.save`, `app.quit`, `ui.palette_open`, `ui.cancel`
- 検索: `search.open`, `search.next`, `search.prev`, `search.toggle_case`
- 取り消し: `edit.undo`, `edit.redo`
- クリップボード: `clipboard.copy`, `clipboard.cut`, `edit.paste`
- 移動: `cursor.move_left`, `cursor.move_right`, `cursor.move_up`, `cursor.move_down`, `cursor.move_word_left`, `cursor.move_word_right`, `cursor.home`, `cursor.end`, `cursor.doc_start`, `cursor.doc_end`, `cursor.page_up`, `cursor.page_down`, `cursor.goto_line`
- 選択: `select.all`, `select.extend_left`, `select.extend_right`, `select.extend_up`, `select.extend_down`, `select.extend_word_left`, `select.extend_word_right`, `select.extend_home`, `select.extend_end`, `select.extend_page_up`, `select.extend_page_down`

対象に含めないもの: `select.extend_to_doc_start` / `select.extend_to_doc_end` (代替キーを持たず、届かない端末ではパレットから実行する。OQ-30 の (2))、`file.open` / `file.new` / `file.save_as` / `file.reopen_with_encoding` / `file.reload` / `file.set_line_ending` / `search.replace_open` / `search.replace_all` 等のその他の操作 (パレット経由で可。置換は OQ-30 の分類に入らない)、版注記付きの ID (`help.open`・行の移動等。各版で加える)。合格の条件: v0.1 の対象端末 (01 NFR-402。Windows Terminal と conhost) の **それぞれ** について、各操作に「`Capabilities.enhanced_keys = false` で区別でき (`[E]` でない)、かつ 04 §7.3a の横取り表でその端末が横取りするキーを含まない」割当が 1 つ以上ある。macOS / Linux の端末は、それらを出荷する版で同じ条件を加える (横取り表の想定 (動作未確認) を横取りとして扱う)。

---

## 12. ワークスペースと信頼モデル

### 12.1 ワークスペースの概念

- **定義** ([00 §6](00-overview.md#6-用語集-glossary)): 起動時のルートディレクトリ。1 プロセスに **1 つ** (複数ルート (multi-root) は Won't (now)。別ワークスペースは別プロセスで開く)。
- **ルート検出**: (1) CLI でディレクトリが渡されたらそれ (`evoeditor .` / `evoeditor path/to/dir`) (2) 渡されたファイル群の共通祖先から上へ `workspace.root_markers` (既定 `.eve`, `.git`, `.hg`, `.svn`) を探し、最初に見つかったディレクトリ (3) 見つからなければ **カレントディレクトリ** (4) `--workspace <dir>` で明示上書き。`$GIT_EDITOR` 等の一時ファイル編集 (§18.6) では `.git` が見つかるためリポジトリがワークスペースになる (ファインダで他ファイルを開けて便利)。
- ワークスペース外のファイルも開ける (ファインダ・横断検索・AI コンテキストの **範囲** を規定するだけ)。
- ワークスペース設定: `<root>/.eve/config.toml` (+ `.eve/themes/`, `.eve/languages/`, `.eve/queries/`)。名前は OQ-01 で確定 (ユーザー回答)。
- **所有者の検査 (OQ-35)**: ルート検出の上方探索と `.editorconfig` の祖先探索は、`/tmp/x.txt` を開いたときに **他ユーザーが作成できる** `/tmp/.eve/config.toml` や `/tmp/.editorconfig` を拾い得る。安全分類の項目にも `[keys]`・テーマ (fg = bg でコードを不可視にする誤誘導)・`format_on_save` が含まれるため、git の `safe.directory` と同様に **ディレクトリの所有者が現在のユーザーでない、または world-writable (Unix: `o+w` で sticky でも、Windows: `Everyone` / `Users` に書込許可) なディレクトリ以下のワークスペース設定・`.editorconfig` は読み込まない** (層ごと無視し、`config.show_diagnostics` に「所有者が異なるため無視しました: <path>」を出す)。例外: ユーザーが `workspace.trust` で明示的に信頼したルートは所有者に関わらず読む (信頼ダイアログに所有者を表示)。判定の `metadata` (owner) はジョブで取る (02 §4.4 規則 2)。

### 12.2 信頼モデルの原則

1. **ファイルを開くだけで何も実行されない** (NFR-502)。ワークスペース設定は読むが、プロセス起動・環境変数・ワークスペース外パスを伴う項目は信頼まで **保留**。
2. 信頼は **ワークスペースルートの正準パス** 単位で記憶 (状態ディレクトリ `trust.toml`。パスと `.eve/config.toml` の内容ハッシュを保存)。設定ファイルが変わったら危険項目の差分を再確認 (§6 規則 5)。
3. 信頼の UI は「何が有効になるか」を **具体的なコマンド行で** 列挙する。「このワークスペースを信頼しますか?」だけの抽象的な問いにしない。
4. 信頼していなくても、ユーザーが **自分で** `lang.format` 等を実行しようとした時点で、その 1 回分の確認ダイアログを出す (FR-709)。信頼は「毎回の確認を省く」操作。
5. CLI `--trust` は提供するが、スクリプト・CI 用途に限る旨をヘルプに明記する。`workspace.trust.default = "always"` は警告付きで許容 (「全部信頼する」ユーザーの選択は尊重する。P6 の「同じ体験」の範囲内で逃げ道を残す)。

### 12.3 項目の分類

| 分類 | 内容 | 未信頼時 |
|---|---|---|
| **安全** | `editor.*`、`view.*`、`search.*` (`search.workspace.follow_symlinks = true` と `search.workspace.max_file_size` を大きくする方向を除く。下の行)、`file.default_encoding` / `file.encoding_detection` / `file.detection_candidates` / `file.default_line_ending` / `file.write_bom` / `file.reload_on_external_change`、`file.atomic_save_fallback = "fail"`、閾値のキー (下の準安全の行) を **小さくする方向のみ**、`[[language]]` のデータ項目 (§9.1。ただし `format_on_save = true` は未信頼では無効、§9.5)、`[keys]` (**追加のみ**: 未割当キーへの追加だけを適用し、既存割当の上書き・解除は信頼後も適用しない。`exec` 権限コマンドへの割当は信頼後、§7.4 / 02 §7.2 K5)、`workspace.exclude`、`.editorconfig`、`ai.exclude` (**除外を増やす方向のみ**)、`ai.secrets.mode` (**厳しくする方向のみ**: `off` < `warn` < `mask` < `block`)、`ai.limits.*` (**上限を小さくする方向のみ**) | 適用 (所有者検査 §12.1 を通ったディレクトリのみ) |
| **準安全** | `theme.name` / `.eve/themes/` (fg = bg 等でコードを不可視にする誤誘導の余地があるため)、`.eve/queries/*.scm` (tree-sitter クエリ)、`.eve/languages/*.toml` の `grammar` 変更、`session.*`、閾値を **大きくする方向** (`file.huge_file_warn_bytes` / `file.huge_file_refuse_bytes` / `file.large_file_threshold` / `file.long_line_threshold` / `file.swap.max_bytes` / `search.workspace.max_file_size`。巨大ファイルの警告を外して OOM を誘う・重い処理を走らせる)、`file.swap.enabled = false` と `file.swap.*` の間隔 (未保存の内容を守る仕組みを弱める)、`file.watch.*` | 保留 (情報診断)。性能攻撃・誤誘導・保護の弱体化の余地があるため |
| **危険** | `[language-server.*]`、`[formatter.*]`、`env`、`[plugins]`、**パスを値に取るすべてのキー** (`file.backup.dir` (ワークスペース外へ書く) と、後の版で足すパスのキー。値がワークスペースの中を指していても同じ扱い)、`log.*`、**外部コマンド**: `clipboard.provider = "command"` と `clipboard.command.*` (コマンドの起動、05 §8.3)、**保存の安全性を下げる値**: `file.fsync = "none"` (電源断で内容を失う)、`file.save_strategy = "in_place"` (原子的保存を止める)、`file.in_place_patterns` (確認なしの直接上書きの対象を広げる)、`file.hardlink_policy = "write_in_place"` (ハードリンクで繋がったワークスペース外のファイルも書き換える)、`file.swap.exclude_secret_files = false` (秘密ファイルの本文を状態ディレクトリに平文で写す)、`file.hot_exit = true`、**ワークスペース外を読む**: `search.workspace.follow_symlinks = true` (リンク先の `~/.ssh` 等を横断検索・ファインダ・AI のコンテキストの範囲に入れる) | 保留。信頼ダイアログで列挙 (コマンド行・解決済み絶対パス・「プロジェクト内のコードを実行し得る」注意、§9.4) |
| **常に無視** (信頼しても適用しない。06 §11.1 / §14、OQ-36) | `ai.enabled`、`[ai.providers.*]` (`caps` を含む)、**`[ai.providers.*.runtime.*]` (管理モードの起動コマンド・引数・モデルパス: 送信先の改変と任意コード実行の両方に当たる)**、`[ai.profiles.*]`、`[ai.features.*]`、`ai.inline.extra_context`、`[ai.approval]`、`ai.allow_excluded_paths`、`[ai.network]`、`[ai.prompts]`、`[ai.agents]`、`ai.context.load_instruction_files`、`ai.audit.enabled`、`ai.exec.allowlist` (送信先の変更・承認緩和はリポジトリ経由の流出に直結するため、信頼で解除できない。`ai.exclude` の追加と `ai.inline.*` の他のキー・`ai.context.surrounding_lines`・`ai.timeouts.*` は安全項目) | 無視 + 警告 (信頼後も) |
| **ユーザー層でも承認を要する** | `ai.providers.*.runtime.command` / `args` / `model_path` (管理モード、§5.12.1): ユーザー層に書かれていても、初回起動時とコマンド行が変わるごとに完全なコマンド行 (解決済み絶対パス・引数・環境変数名) を示して `exec` 承認を得る (06 §3.3.3)。承認の記録は状態ディレクトリ (`trust.toml` と同じ扱い)。禁止引数 (§5.12.1) を含む定義は承認以前に検証エラー | 承認まで起動しない |
| **禁止** (どの層でも) | `api_key` 平文 (06 §10.1)、シェル文字列としてのコマンド指定、`runtime.command` の相対パス・`PATH` 探索・`.bat` / `.cmd`、`runtime.args` の禁止引数、`runtime.transport` にループバック以外の待受 | エラー |

- **表に無いキーの既定は「保留 (危険扱い)」** とする: 上の表のどの行にも当たらないキー (後の版で足すキーを含む) は、ワークスペース層に書かれていれば危険の行と同じく信頼まで保留し、信頼ダイアログで列挙する。キーを足すときは、本表のどの行に入れるかを §5 の表と同時に決める (決めるまでは保留のまま)。
- 「方向のみ」の行 (安全の「小さくする方向のみ」、準安全の「大きくする方向」等) は、ワークスペース層の値をユーザー層 (無ければ既定値) と比べて判定する。比べられない値 (型の違い等) は保留とする。`file.*` の各キーと閾値のキーの安全・準安全への割り振りは提案 (提案、OQ-202)。
- 同じキーでも値で行が分かれるもの (`clipboard.provider`、`file.fsync`、`file.save_strategy`、`file.hardlink_policy`、`file.atomic_save_fallback`、`file.swap.exclude_secret_files`、`search.workspace.follow_symlinks` 等) は、表に書いた値のときだけその行に当たり、他の値は既定値と同じ安全側の値なら安全、そうでなければ保留とする。

### 12.4 コマンド

`workspace.trust` / `workspace.untrust` / `workspace.show_trust` (状態と保留項目の表示)。ステータスバーに未信頼かつ保留項目ありの場合に記号 (例 `[!]`) を表示する (04)。

---

## 13. 拡張機構の段階的方針

### 13.1 段階的方針

| Stage | 版 | 内容 | 拡張の「作者」 |
|---|---|---|---|
| **0: 設定による拡張** | v0.1〜v0.4 | キーマップ・テーマ・言語定義・フォーマッタ/外部フィルタ (`ProcessRunner`、FR-709)・コマンド alias。**「宣言 + 既存コマンドの組合せ」で届く範囲** | 全ユーザー |
| **D: 駆動 API (ヘッドレス RPC)** | v0.2 以降 (dev feature `headless`。v0.1 はメソッド名と Frame JSON の形の予約のみ。配布同梱は OQ-120) | `evoeditor --headless --rpc stdio [--fake-ports] [--allow-write]` で **エディタ自身を JSON-RPC 2.0 (Content-Length フレーミング) のサーバ** にする。メソッド (`initialize` / `editor/input` / `editor/execute` / `editor/query` / `editor/frame{json\|ascii}` / 通知 `editor/events` / `edit/propose` / 駆動モード限定 `host/*`) の正本は [02 §10.5](02-architecture.md#105-駆動-api-stage-d)。主体は External (X16)、権限は read + propose + ui、`exec` / `net` は常に拒否、保存は `--allow-write` 時のみ。動作中のエディタへ接続する **sidecar モード** (ローカルソケット、所有者のみ) は v1.L-b 候補で、MCP サーバ (06 §3.5) の土台。使い方は 11-dev-workflow | テストハーネス、AI コーディングエージェント、将来のスクリプト |
| **1: 専用プロトコル** | v0.4〜v1.L | tree-sitter 文法/クエリの追加 (OQ-22)、LSP (eve-lsp)、AI プロバイダ (eve-ai)。それぞれ既存の標準プロトコル・形式を使う。汎用プラグイン API は作らない | 言語サーバ作者など外部エコシステム |
| **2: 汎用プラグイン (外部プロセス)** | v1.X | 駆動 API と同じ crate `eve-protocol` に足すプラグインのプロトコル (JSON-RPC 2.0 over stdio、LSP と同じフレーミング `Content-Length`。プラグイン専用の crate は作らない、02 §4.1 / §10.3)。manifest、権限、Command 登録、Proposal、Decoration、UI モデル。Stage D の駆動 API と **同じ JSON-RPC 配管・同じメソッド系統** を使い、プラグインは駆動 API のメソッドを権限付きで呼べるようにする (Stage D のメソッドが Stage 2 の下位集合になる) | サードパーティ |
| **3: 軽量同期拡張 (WASM)** | v1.X 以降、要望と計測で判断 | キー入力に同期する処理 (入力補助・テキスト変換・独自 Decoration) が外部プロセスでは遅い場合に限り、extism/wasmtime を **cargo feature** で追加。本体既定ビルドには含めない | — |
| 組込みスクリプト | 計画なし | 設定にスクリプトは使わない (ADR-0011)。「マクロの保存・簡単な条件分岐」は、マクロ (FR-215) の Command 列保存と `when` 句で満たす | — |

**ローカル LLM と拡張点 (X16〜X26) の関係**: AI プロバイダは Stage 1 の「専用プロトコル」(OpenAI 互換 HTTP + `llama-server` ネイティブ API。06 §3.3) であり、汎用プラグイン機構を経由しない。eve-ai が core に触る経路は拡張点だけ: `Query` API (X18) で `ContextBuilder` が文書範囲・選択・診断を決定的に取り出し (X26、06 §6.8)、`EditorEvent::DocumentChanged` の `EditSummary` (X25。v0.1 は型の予約のみで、生成は v1.L の最初の利用者と同時) でキャッシュ境界の判断と古い補完要求の破棄を行い、`Proposal` / `Hunk` (X19、eve-view) で変更を提出し、権限判定フック (X20) で主体 `Ai` の `propose` / `read` を照合する。ローカルサーバの管理モード (FR-918) は `ProcessRunner::spawn_long_lived` (05 §2.6 / §13.7) を使い、承認は §12.3 の「ユーザー層でも承認を要する」行。cargo feature 境界 (X23): `ai` = tokio + ループバック HTTP (TLS なし)、`ai-remote` = rustls を追加 (02 §4.1)。**ローカル LLM だけを使うビルドは TLS 系の依存を含まない** (NFR-207 の予算を守る)。駆動 API (Stage D) は逆方向 (外部のエージェントがエディタを操作する) で、ローカルモデルを推論先にしたエージェントも同じ入口を使う (11 §7.3)。

**Stage 2 を v1.X まで待つ理由**: (1) v1.L で LSP/AI の非同期サービス連携を実装すると「Snapshot の差分送信・Revision 照合・Proposal 経路」が実証済みになり、プラグインプロトコルはその一般化で済む (2) 先に API を公開すると互換性維持の責任が早期に発生する。**Stage D を v1.X より前 (v0.2 以降) に置く理由**: 外部から見える API は「テスト・開発用で非公開 (dev feature)」と割り切ることで互換責任を負わずに済み、同時に CommandId / `Query` / `EditorEvent` / Frame JSON という **後のプラグイン API の骨格** を実際に使って固められる (ADR-0032)。

### 13.2 プラグインプロトコルの骨子 (v1.X で詳細化)

```toml
# <plugin_dir>/plugin.toml (manifest)
[plugin]
id          = "spellcheck"              # [a-z][a-z0-9_]{1,31}。ext.spellcheck.* の名前空間を得る
version     = "1.2.0"                   # semver
api_version = "1"                       # 対応するプロトコルのメジャー版
license     = "MIT"                     # SPDX 式 (必須、§19)
display_name = "Spell Check"
description  = "..."
homepage     = "https://..."

[run]
command = ["python3", "${plugin_dir}/main.py"]     # 配列。shell なし
# または
# wasm = "${plugin_dir}/plugin.wasm"               # Stage 3

[activation]                            # 遅延起動の条件 (起動経路に乗せない)
on_command  = ["ext.spellcheck.*"]
on_language = ["markdown", "plain"]
on_event    = ["document.saved"]

[permissions]                           # §14.3。host API 経由で強制できる権限 (宣言したものだけ要求できる)
read.visible   = true
read.workspace = false
edit.propose   = true
ui             = true
exec           = false

[declares]                              # §14.3。作者の自己申告 (外部プロセス型では host が強制できない。表示・記録のみ)
net            = []                     # 自ら通信するホスト名 (空 = しない)
fs.read        = ["${plugin_dir}", "${cache_dir}/spellcheck"]
fs.write       = ["${cache_dir}/spellcheck"]

[[contributes.commands]]
id    = "ext.spellcheck.toggle"
title = "Toggle Spell Check"            # 英語。翻訳は locales/ に plugin 側で同梱可
when  = "editor.focus"

[contributes.keys.editor]               # 既定キー提案。ユーザー層で上書き可。衝突時は適用せず通知
"f7" = "ext.spellcheck.toggle"

[contributes]
languages = ["languages/*.toml"]         # 言語定義・テーマ・ロケールの同梱
themes    = ["themes/*.toml"]
```

**プロトコル (JSON-RPC)**: `initialize` (双方の `api_version`・能力を交換) → `activate` → 以降双方向。**命名は Stage D の駆動 API (02 §10.5) と同じ系統** (`document/*`、`edit/propose`、`ui/*`、`editor/query`) にし、Stage 2 着手時に両者を 1 つの名前空間に統合する。主要メソッド (案):

| 方向 | メソッド | 内容 |
|---|---|---|
| host→plugin | `document/didOpen` / `didChange` (ChangeSet + Revision) / `didSave` / `didClose` | 購読した文書イベント (`read.visible` 権限の範囲) |
| host→plugin | `command/execute` (id, args, context) | 登録コマンドの実行要求 |
| plugin→host | `document/snapshot` (doc, range?) → text + revision | 明示的な読み取り (権限範囲) |
| plugin→host | `edit/propose` (ChangeSet@revision, description) | **Proposal として提出**。host が `Transaction(Origin::Plugin(id))` に変換。`edit.apply` 権限がなければ承認 UI を経る。Revision が古ければ rebase または失効 ([02 §5.3](02-architecture.md#53-revision-とスナップショット)) |
| plugin→host | `decoration/set` (doc, revision, items) / `virtualText/set` | Decoration Provider 相当。可視範囲外は host が間引く |
| plugin→host | `ui/message` / `ui/prompt` / `ui/list` / `ui/panel` | 意味的 UI モデル (§14.4)。ANSI・端末制御は不可 |
| plugin→host | `config/get` (自分の `[plugins.<id>.config]` のみ) / `context/set` (`ext.<id>.<var>`) | 設定・`when` 変数 |
| plugin→host | `workspace/readFile` / `workspace/find` / `process/run` / `net/fetch` | 権限が必要。`exec` は都度承認 |
| 双方 | `$/cancel`, `shutdown`, `exit` | — |

### 13.3 プラグインの発見と配置

- `<config_dir>/plugins/<id>/plugin.toml` (ユーザー層) と `<workspace_root>/.eve/plugins/<id>/` (ワークスペース層、**信頼 + 個別承認が必要**。既定では一覧表示のみ)。
- 中央レジストリは v1.X 時点では **作らない**。`evoeditor plugin install <git-url|path>` (サブコマンド化は OQ-24 の clap 移行条件) で取得する。**インストール時の承認は「このプラグインはあなたの権限で任意のコードを実行できます」の 1 点に集約** する (OQ-43 / 02 §10.4)。manifest の `[permissions]` (host API 経由で強制できるもの) と `[declares]` (作者の自己申告) は **情報として表示** し、「許可していないことはできない」という誤解を与える承認 UI にしない。
- 更新・署名・レジストリは §19 と §20。

---

## 14. 拡張 API 設計原則

### 14.1 安定性とバージョニング

| 対象 | 方針 |
|---|---|
| Command ID / 引数 / `when` 変数 / 設定キー / テーマスコープ名 / キーマップ書式 | **公開 API**。semver に従う。追加は互換、変更・削除は alias 期間 (最低 2 マイナー) を置き、情報診断で告知。本体 1.0 以前 (0.x) は「マイナーで破壊可、ただし alias 努力義務」 |
| プラグインプロトコル | `api_version` メジャー単位で互換。host は **直近 2 メジャー** を同時にサポートする (旧版プラグインが急に動かなくならない)。能力交換 (`initialize`) で任意機能を名乗り、未対応メソッドは `MethodNotFound` で安全に失敗 |
| `eve-view` の型 (Frame 等) | プラグインには **公開しない** (frontend 境界であり拡張境界ではない) |
| Rust の内部 trait (Decoration Provider 等) | **非公開** (crate 内 API)。安定性を約束しない。組込み機能の構成手段 |

### 14.2 設計規律

1. **Command 経由の原則**: プラグインが起こせる状態変化は、(a) Command の実行、(b) Proposal の提出、(c) Decoration / 仮想テキスト / UI モデルの提示、(d) `when` 変数の公開、だけ。文書・選択・設定の直接書換 API は作らない。AI ([ADR-0013](10-decisions.md#adr-0013-ai-連携の基本方針)) と同じ経路なので権限・監査・Undo (Origin) が自動的に揃う。
2. **非同期・キャンセル可能**: host → plugin の呼出はすべて非同期 (`$/cancel` 対応)。host はプラグイン応答を待って UI を止めない (P3)。応答期限 (既定 5 s) 超過で「応答なし」通知、連続 3 回でプラグインを停止。
3. **Revision 付きデータ**: 文書内容を伴うメッセージはすべて `(DocumentId, Revision)` を持つ。古い Revision への提案は host が rebase/失効を判断 (プラグイン作者に競合処理を強いない)。
4. **最小データ**: `didChange` は ChangeSet 差分。全文送信は `document/snapshot` の明示要求のみで、巨大ファイル (`doc.large`) では範囲必須。
5. **決定的・テスト可能**: プロトコルの JSON を記録・再生するテストハーネス (`eve-testkit`) を提供し、プラグイン作者もモック host で CI できるようにする。

### 14.3 権限 ([02 §10.4](02-architecture.md#104-権限モデル-aiプラグイン共通骨子) の具体化)

**「権限」という語は host API 経由で強制できるものに限る** (OQ-43 / NFR-508)。外部プロセス型プラグインはユーザーの OS 権限で任意のコードを実行できるため、`net` / `fs.*` / `clipboard` は **作者の自己申告 (`[declares]`)** として表示・記録するだけで承認の対象にしない (承認させると「許可していないことはできない」と利用者が誤解する)。インストール時の承認は「任意コード実行」の 1 点 (§13.3)。NFR-508 の検証区分 R は「外部プロセス型では OS レベルの強制がない」ことの明示を含む。

| 区分 | 権限 / 申告 | 内容 | 既定 (manifest で宣言した場合) | 承認時期 |
|---|---|---|---|---|
| 権限 (強制可) | `read.visible` | 開いている文書のうち **activation 条件に合致する言語** の内容と変更イベント (host が送るものだけ) | 許可 | インストール時に表示 |
| 権限 (強制可) | `read.workspace` | `workspace/readFile` / `workspace/find` 経由のワークスペース内ファイル読取 (`ai.exclude` / `workspace.exclude` 適用、`.gitignore` 適用) | 要承認 | インストール時 |
| 権限 (強制可) | `edit.propose` | Proposal 提出 (承認 UI を経る) | 許可 | — |
| 権限 (強制可) | `edit.apply` | 承認なしで Transaction 適用 (フォーマッタ的用途) | **要承認 + 警告**。ユーザーが明示 opt-in | インストール時 + 初回実行時 |
| 権限 (強制可) | `ui` | メッセージ・プロンプト・一覧・パネル | 許可 (頻度制限あり: メッセージ 10 件/分) | — |
| 権限 (強制可) | `exec` | host 経由の外部プロセス実行 (`process/run`) | **禁止**。都度承認のみ (「今後このコマンドは確認しない」は信頼済みワークスペース内で可) | 実行時 |
| 自己申告 (強制不可) | `declares.net` | プラグインプロセスが自ら通信するホスト名 | 表示・監査ログに記録のみ。既定ビルドの「ネットワーク通信ゼロ」(NFR-501) は **本体について** の主張であり、プラグインには及ばないことを UI に明示 | インストール時 (表示) |
| 自己申告 (強制不可) | `declares.fs.read` / `fs.write` | プラグインプロセスが自ら触るディレクトリ | 表示・記録のみ | インストール時 (表示) |
| 自己申告 (強制不可) | `declares.clipboard` | クリップボード読み書き | 表示・記録のみ | インストール時 (表示) |

Stage 3 (WASM、§13.1) に移行した場合は `declares.*` を能力ベースで **強制可能な権限** に昇格できる。これが Stage 3 を残す理由 (§14.4)。権限の **増加** または自己申告の変化を伴う更新は再承認 (任意コード実行の再確認)。承認状態は `[plugins.<id>.approved]` (ユーザー層、`toml_edit` で書込。承認した版と manifest のハッシュ) に保存。

**External 主体 (駆動 API / sidecar、X16。v0.1 は変種の予約のみ、判定は駆動 API と同じ版から)**: プラグインより先に権限判定フック (X20) を通る主体。既定は `read.visible` + `edit.propose` + `ui` (02 §10.4 / §10.5) で、`read.visible` は **承認なしで許可** する (除外規則は `editor/query` の応答を作る箇所で常に適用する)。**`exec` と `net` はどのモードでも常に拒否** し、`edit.apply` も許さない。駆動モード (`--headless`) は対話的な承認 UI を持たないため、承認を要する権限を常に拒否し、保存は起動時の `--allow-write` でのみ許す。sidecar (v1.L-b) は接続時にユーザーが承認し、`[external.<client>]` (v1.L-b で定義) で `read.workspace` のセッション許可まで広げられる (06 §8.2 / AI-INV-2)。

### 14.4 サンドボックス

- プラグインは **端末・OS ウィンドウに触れない**。UI は意味的モデル (`List` / `Prompt` / `Confirm` / `Panel{Tree|Log|Chat|Diff}`) のみで、描画は frontend が行う。
- 文字列出力は host が **無害化** (NFR-503)。プラグイン由来の文字列は「ファイル内容」と同じ非信頼データ扱い。
- 外部プロセス方式の隔離は「プロセス境界 + プロトコル上の権限」。OS レベルの強制 (seccomp / AppContainer / sandbox-exec) は **Could (v1.X+)**。WASM 方式に移った場合は能力ベースで強制できる。この差が Stage 3 を残す理由。
- 資源制限: メッセージサイズ上限 (既定 16 MB)、未処理要求数上限、応答期限。暴走プラグインは停止でき、エディタ本体は影響を受けない。

---

## 15. LSP / DAP / フォーマッタ / リンタ統合方針

| 統合 | 方針 | 設定上の位置 | 時期 |
|---|---|---|---|
| **LSP** | `eve-lsp` が標準プロトコルで接続 (FR-804)。設定は §9.4。言語ごとに複数サーバ可 (優先順)。サーバ定義の **組込み既定** (rust-analyzer 等) を同梱するが、**実行ファイルは同梱しない** し、自動インストールもしない (v0.4 の原則: 通信・ダウンロードは明示操作)。実行ファイルの探索は 05 §13.3 の規則 (絶対パスの PATH 要素のみ。cwd・相対要素は探索しない)。初回起動の確認 (信頼ダイアログ) には「このツールはプロジェクト内のコードを実行する可能性があります」を含める。未インストール時は「見つかりません。インストール方法: <URL>」を 1 回だけ通知し、`lsp.enabled` や言語単位で黙らせられる。長寿命プロセスの起動は `ProcessRunner::spawn_long_lived` (02 §4.4 / 05 §2.6) | `[lsp]`, `[language-server.*]`, `[[language]].language_servers` | v1.L |
| **フォーマッタ** | (a) 外部コマンド (stdin/stdout) を `ProcessRunner` で、FR-709 の承認付き。(b) v1.L 以降は LSP の `textDocument/formatting` を優先し、`formatter` 明示指定があれば (a)。`format_on_save` は既定 `false` (保存を遅くしない・意図しない変更を避ける v0.1) | `[formatter.*]`, `[[language]].formatter`, `.format_on_save` | (a) v0.4 / (b) v1.L |
| **リンタ** | 専用機構は作らない。診断は LSP (v1.L) 経由を正とする。LSP を持たないリンタ (shellcheck 等) は v1.X のプラグイン (`decoration/set` で診断を出す) で対応。それまでの代替は「外部コマンドの出力をパネルに表示 (`lang.run_command`、Could v0.4)」 | — | v1.L / v1.X |
| **DAP (デバッガ)** | **対応しない** (NG1)。将来やるならプラグイン (Stage 2) の領域で本体には入れない。本書での結論は「設定スキーマに `[debugger]` を予約しない」 | — | Won't |
| **外部フィルタ** (FR-709) | 選択範囲を外部コマンドの stdin に渡し stdout で置換 (`edit.filter_through`)。コマンドはプロンプトで入力し **毎回確認**、履歴から再実行可。シェル経由 (`sh -c` / `cmd /C`) は **ユーザーが対話的に入力した場合のみ** 許容 (設定ファイルからは配列指定のみ) | `[keys]` からの `args = { command = [...] }` も可 (信頼後) | v0.4 |
| **MCP** | [OQ-11](10-decisions.md#oq-11-mcp-対応の方針)。Stage 2 の JSON-RPC 基盤を共用し、MCP サーバ側 (外部エージェントに文書・Proposal 提出を公開) を先行。設定は `[mcp]` を 06 で定義 | `[mcp]` (書式のみ予約) | v1.X |

---

## 16. セッション復元 (FR-121, v0.4 Could)

### 16.1 保存する内容

| 内容 | 保存 | 備考 |
|---|---|---|
| 開いていたファイルのパス (ワークスペース単位) と順序、アクティブなもの | ○ | 絶対パス。存在しなくなったファイルは復元時にスキップして通知 |
| ファイルごとのカーソル位置・スクロール位置・折り返し等の View 設定 | ○ (`session.remember_cursor`、復元 off でも「前回の位置から開く」は有効) | 位置は行:桁 (バイトオフセットはファイル変更で狂う) + 変更検知用の mtime/サイズ。ファイルが変わっていれば行番号を目安に丸める |
| レイアウト (分割ツリー、v0.4) | ○ | — |
| 未保存の内容 | **×** | スワップ/ホットエグジット (FR-118, OQ-17) の責務。セッションファイルに本文を書かない (NFR-504/505 と同じ機密性配慮) |
| 検索履歴・コマンド履歴・最近開いたファイル | ○ (別ファイル `history.toml`、上限件数) | 機密性のため `history.enabled = false` で無効化可。パターンに `.env` 等が含まれても **検索語自体** は保存される点を注意書き |
| Undo 履歴 | × | [OQ-18](10-decisions.md#oq-18-undo-履歴の永続化) |

### 16.2 保存場所と形式

`<state_dir>/sessions/<ワークスペースルートのハッシュ>.toml` (TOML、人が読める)。`<state_dir>` は 05 (XDG `$XDG_STATE_HOME/evoeditor`、Windows `%LOCALAPPDATA%\evoeditor\state` 等)。書込は原子的保存、権限は所有者のみ (NFR-505)。終了時 + 5 分ごと (変化があれば) に保存。

### 16.3 復元の挙動

| 起動の形 | `session.restore` の既定 `"off"` | `"ask"` | `"auto"` |
|---|---|---|---|
| `evoeditor` (引数なし、ワークスペースにセッションあり) | 空バッファ。ヒントバーに「前回のセッションを復元: `session.restore`」を 1 回表示 | 確認ダイアログ | 復元 |
| `evoeditor file...` | 指定ファイルだけ開く (カーソル位置は記憶から) | 同左 | 指定ファイルを開き、**セッションは復元しない** (明示指定を尊重) |
| `evoeditor -` / `$EDITOR` 的な一時ファイル / `--no-session` | 復元も保存もしない | 同左 | 同左 |

既定を `"off"` にする理由: 初心者が「前に開いていた知らないファイルが出てきた」と戸惑うのを避ける (P7)。U1 は `"auto"` に設定すればよい。

---

## 17. 国際化 (UI 文言)

### 17.1 方針 ([OQ-21](10-decisions.md#oq-21-ui-文言の国際化) の具体化)

- 文言はすべて **MessageKey** 経由。コードに英語文字列を直接書かない (lint で検出、09)。
- **v0.1 の UI 文言は日本語 (`ja`) のみ** (OQ-21、確定 (ユーザー回答))。言語ファイル `locales/ja.toml` をソースツリーに置き、`include_str!` で埋め込む (実行時のファイル読込なし)。**MessageKey による外部化は v0.1 から行う** (コードに文言を直接書かない) ので、後の版で言語を足すときにコードを変えずに済む。キー集合の正は「コードの MessageKey 一覧」で、`ja` はそれと過不足なく一致させる (§17.3)。
- **言語ファイルに入れる文言** (その版に存在するもの): コマンドの名前と説明 (`cmd.*`)、エラー・警告・情報のメッセージ (`msg.*` の `text` と `hint`、`info.*`)、ヒントバー (`hint.*`)、ステータスバー (`status.*`)、ダイアログ・ラベル (`ui.*`)。設定項目の説明 (`config.*`) とヘルプ画面 (`help.*` の見出しと説明) は、それを画面に出す版 (v0.2 以降) で加える。**言語ファイルに入れないもの**: 識別子 (CommandId、CLI オプション名、設定キー名、テーマのロール名、MessageKey 自体)、ログ (英語のみ)、パレットの照合用の英語の title (定数表、§11.4)、`--help` / `--version` の出力、パニック時・起動前のメッセージと、端末を復元した後に stderr へ出す文言。最後の 3 つは **v0.1 は日本語の行だけ** を出し、**v0.2 (英語 UI と同時) から英語 (ASCII) の行と日本語の行を併記する** (確定 (ユーザー回答)、04 §14.4 / §15.2、OQ-67、01 §4.6)。日本語の行はコンソールなら `WriteConsoleW` (コードページに依らない)、リダイレクト先なら UTF-8 で書く (リダイレクト先を出力コードページ 932 で解釈する環境では化ける。v0.1 の既知の制約で、v0.2 の英語の行はコードページやリダイレクト先によらず読める)。これらも MessageKey (`stderr.*` / `panic.*` / `cli.*`) で識別し、eve-host / bin の定数表に置く (パニックの経路で言語ファイルを引かないため)。目録は 04 §15.2。core が出す stderr の行 (終了時の `Effect::Quit.stderr`) も core は文言を解決せず、キーと引数 (`StderrLine`) で eve-host に渡し、eve-host が定数表から解決する (02 §6.3)。v0.2 の英日併記も eve-host が 1 つの `StderrLine` から英語の行と日本語の行を引いて作る。
- **v0.2** (確定 (ユーザー回答)、OQ-21。**`locales/en.toml` は M8 で同梱済み**: キー集合・プレースホルダ・`[X]` のアクセラレータ・`〔 〕` の有無が `ja` と一致し、`en` にかな・漢字が無いことを `cargo xtask i18n` が検査する): 英語 UI (`en`。`locales/en.toml` を `ja` と同じく `include_str!` で埋め込む)、言語の選択 (`--locale` > `ui.locale` > 環境変数 `EVE_LANG` > OS ロケール (05 §6.1) > 既定 `ja`。**M8 の 2 本目で実装済み**: 選択は `eve_config::select_locale`。`auto`・空の値は未指定として飛ばし、`--locale` と `EVE_LANG` が同梱の言語に当たらなければログに警告して次の段へ進む。OS ロケールの候補が当たらないときは黙って次の段へ進む。言語は起動時に 1 回だけ決める)、言語の切替、言語間のキー集合の差分の CI 検査 (§17.3)、設定項目の説明 (`config.*`)、起動前・パニック時の英語 (ASCII) の行の併記 (OQ-67)。**v0.2 以降の候補 (提案中。推奨は v0.3)**: ユーザー層 `locales/<lang>.toml` による上書き・追加言語。同梱の言語ファイルとユーザーの言語ファイルは同じ書式 (§17.2) なので、v0.1 で書式を確定する。v0.1 は `ui.locale`・`--locale`・`EVE_LANG` を持たず、`EVE_LANG` を設定しても読まなかった (`ui.locale` を書くと「このバージョンでは未対応」の情報診断)。
- フォールバック: `ja` にキーがなければ **キー名そのもの** を表示する (空白や例外にしない)。同梱の `ja` とコードの MessageKey 一覧の差分ゼロを CI で保証するので (09 §11)、この経路は開発中のビルドのためにある。開発ビルドでは欠落キーを警告ログ。
- **サイズと起動時間**: 同梱分は `ja` で約 50 KB (TOML 原文。v0.1 の文言の規模から見込み) で、NFR-205 (8 MB) への影響は小さい。起動時のデシリアライズが NFR-101 の起動の予算 (プロセス内 8 ms) に入るかを BM-101a で確かめる (09 §5)。目安は 1 ms 以下。超えるならビルド時に TOML を静的な表 (キー → 文字列の配列) に変換して埋め込む方式に切り替える (書式は変えない)。

### 17.2 書式

形式は **TOML のキー → 文字列 + `{name}` プレースホルダ** (設定と同じ形式・同じパーサで、依存を増やさない)。複数形は `.one` / `.other` 接尾キーの **任意** 規約 (`ja` は `other` のみ)。

```toml
# locales/ja.toml
[meta]
language = "ja"

[cmd.file.save]
title = "保存"
description = "現在の文書を上書き保存します"

[msg.save.failed]
text = "{path} を保存できませんでした: {reason}"
hint = "別名で保存 ({key:file.save_as}) するか、権限を確認してください"

[msg.search.count]            # 複数形は任意。ja は other のみ
other = "{count} 件"

[hint.app]                    # ヒントの語は hint.<command_id> = 文字列 (テーブルにしない)
quit = "終了"
[hint.file]
save = "保存"

[status]                      # 短い形を持たない項目は status.<name> = 文字列
untitled = "無題-{n}"
selection = "({count} 文字選択)"

[status.loading]              # 短い形を持つ項目は status.<name>.long と status.<name>.short の 2 つ
long = "[読込中 {percent}%]"
short = "{percent}%"

[status.position]
long = "行 {line}, 桁 {col}"
short = "{line}:{col}"
```

TOML では同じキーを文字列とテーブルの両方にはできない (`status.loading = "…"` と `status.loading.short = "…"` は同時に書けない)。そのため、子のキーを持つキーには値を直接置かず、葉のキーにだけ文字列を置く: 短い形を持つステータス項目は `status.<name>.long` / `status.<name>.short`、確認ダイアログは `ui.confirm.<name>.title` / `.message` / `.<choice>` のように、必ず葉の名前を付ける。CI (09 §11) は、あるキーが文字列とテーブルの両方として使われていないこと (コードの MessageKey 一覧の中で、あるキーが別のキーの接頭辞になっていないこと) を検査する。

| キー空間 | 内容 |
|---|---|
| `cmd.<command_id>.title` / `.description` | コマンドの表示名・説明 (パレット。v0.2 からヘルプ)。パレットの照合用の英語 title は manifest / 定数表に、表示する文言は locales に |
| `msg.<domain>.<name>.text` / `.hint` | Message (NFR-602 の「何が起きたか」=text、「どうすれば」=hint)。例: `msg.save.encoding_unsupported_iso2022` (不正バイトを含む文書の ISO-2022-JP 保存の拒否)、`msg.open.read_failed`、`msg.io.stalled` (`io` ワーカーの停止の通知)、`msg.edit.replacing` (置き換えの読込中の編集の拒否)、`msg.save.changed_after_save` (保存の直後に他のプログラムが変更した可能性の警告)。確認は `ui.confirm.save_failed_closed` (閉じた文書の保存失敗: 退避先に保存 / 破棄)。目録は 04 §15.2 |
| `info.<domain>.<name>` | 情報メッセージ (文 1 つ。`hint` を持たない)。例: `info.open.guess_unsupported` (`{guess}` / `{encoding}`)、`info.eol.converted` / `info.eol.unchanged` (`{eol}`)。目録は 04 §15.2 |
| `msg.config.<種別>.text` / `.hint`、`info.config.<種別>` | 設定の診断 (§4.2 の検査の種類ごと。v0.1 は `syntax` / `unknown_key` / `type_mismatch` / `out_of_range` / `invalid_value`、「このバージョンでは未対応」は `info.config.unsupported` (v0.2 から画面に出す))。起動時の要約は `msg.config.problem` / `msg.config.problems` で、その `{detail}` / `{hint}` に種別の文言を入れる (§4.3)。型の語は `ui.config.type.*`。目録は 04 §15.2 |
| `ui.confirm.<name>.title` / `.message` / `.<choice>` | 確認ダイアログ (選択肢の文言はアクセラレータ `[X]` を含む)。保存・読込・退避の目録は 04 §15.2 |
| `stderr.*` / `panic.*` | 端末を復元した後の stderr とパニックの文言 (v0.1 は **日本語のみ**。v0.2 から英語 (ASCII) の行と日本語の行の併記、OQ-67。言語ファイルに入れず eve-host の定数表に置く、§17.1。core が出す行もキーと引数 (`StderrLine`) で渡し、解決は eve-host、02 §6.3。例: `stderr.quit.waiting_save` (終了時に置換中の保存の完了を待つ前に出す 1 行、02 §9.6)。目録は 04 §15.2) |
| `hint.<command_id>` | ヒントバーの短い語 (値は文字列 1 つ。端末幅が狭いため **全角 4 文字 / 半角 8 文字** (8 セル) を上限とし、超えたら CI で失敗)。§11.6 の「ヒント」列に文脈がある CommandId はすべて持つ。対訳は 04 §15.1 |
| `status.<name>.long` / `status.<name>.short` | ステータスバーの項目のうち短い形を持つものの長い形と短い形 (04 §8.7。core が両方を解決して `StatusItem.long` / `short` に入れる)。短い形を持たない項目は `status.<name>` (文字列 1 つ) |
| `ui.confirm.nav_hint` / `ui.confirm.cancel` | 確認ダイアログの操作の案内 (`Confirm.nav_hint`) と、すべてのダイアログに付く `[Esc] キャンセル` |
| `ui.palette.title` / `.arg_title` / `.footer` / `.no_match` / `.placeholder` / `.disabled` | パレット (`OverlayModel::List` の `title` / `footer` / `no_match`、入力欄の `placeholder`、実行できない理由) |
| `ui.prompt.<name>.title` / `.hint` / `.error.<kind>` | 入力プロンプト (`Prompt.title` / `hint` / `error`)。`<name>` は `open` / `save_as` / `goto_line` / `arg` 等 |
| `ui.search.label` / `ui.search.count.<state>` | 検索バーの見出し (`SearchBarModel.labels`) と件数 (`count_text`。`<state>` は `running` / `exact` / `at_least` / `approx` / `no_match`) |
| `ui.too_small` | 端末が小さすぎるときの文言 (`Frame.texts.too_small`、`ChromeTexts`) |
| `ui.<component>.<name>` | その他のダイアログ・ラベル (`ui.key.via_palette`、`ui.reason.*`、`ui.read_only_reason.*` 等) |
| `config.<key>.desc` | 設定項目の説明 (v0.2。§17.1 のとおり設定項目の説明を画面に出す版で加える。§4.1 の雛形の生成 (v0.4) にも使う。v0.1 の雛形 `config.example.toml` は手書き) |
| `cli.help.*` | `--help` の出力 (`cli.help.usage` / `.description` / `.options_title` / `.option.<name>` / `.encodings` / `.exit_codes`。v0.1 は日本語のみ。v0.2 から英語と日本語の併記、OQ-67。言語ファイルに入れず bin の定数表に置く。目録は 04 §15.2)。`--version` は版・ビルド情報だけ (翻訳する文言を持たない) |
| `ext.<plugin_id>.*` | プラグイン用 |

上の `ui.*` の目録 (v0.1 で出る文言とその Frame の欄) の正本は 04 §15.1 / §15.2 で、本表はキー空間の規則だけを決める。オーバーレイ・クロームの文言はすべて core が解決して `CellText` で Frame に入れ、frontend は MessageKey を解決しない (02 §8.2)。確認ダイアログ (`ui.confirm.<name>`) の `<name>` は 04 §15.2 の目録と同じで、保存の確認は `ui.confirm.save.<name>` (`ui.confirm.save.overwrite` (名前を付けて保存の保存先に既にファイルがある。既定は「別の名前を入力」)、`ui.confirm.save.atomic_unsupported` (既定は「別名で保存」)、`ui.confirm.save.unmappable_replace` (表現不能文字の置換の一覧。一覧の行は `ui.confirm.save.unmappable_replace.item`、省略の行は `ui.confirm.save.unmappable_replace.more`) 等)、読込の確認は `ui.confirm.open.<name>` (`ui.confirm.open.huge` / `ui.confirm.open.invalid_bytes` / `ui.confirm.open.binary` / `ui.confirm.open.utf32`)、終了・置き換えの確認は `ui.confirm.unsaved` / `ui.confirm.unsaved_replace` / `ui.confirm.unsaved_reopen` / `ui.confirm.quit_saving` の形にする。読込の途中の失敗は v0.1 では確認ではなくメッセージ (`msg.open.read_failed`) で、確認ダイアログは v0.3 以降の予約。読込中の状態の文言は読み取り専用の理由 (`ui.read_only_reason.*`) と分けて `ui.load_state.loading` に置く (04 §15.2)。

プレースホルダは `{name}`、エスケープは `{{`。型は文字列・整数・パス (パスの表示用の短縮 `~/…` は core が Frame 等を作るときの変換関数 1 つに一本化する。host や frontend は短縮しない。02 §4.4)。引数の順序が言語で変わるため **位置引数は使わない**。引数の名前の目録は 04 §15.2 (`{path}` / `{count}` / `{kana}` / `{compat}` / `{guess}` / `{eol}` 等)。`{eol}` は `LF` / `CRLF` / `CR` で翻訳しない。

**キーの表記は文言に直書きしない**。`{key:<command_id>}` と書き、表示時に core が **解決済みキーマップ** からそのコマンドの第一割当を引いて表示表記 (04 §7.8) に置き換える。キーマップの変更 (v0.2 の `keymap.toml`) や主修飾 `mod` の解決 (TUI は `Ctrl`) に追従させるため。割当が無い (`keys.preset = "none"` 等) ときは「コマンドパレットから実行」に当たる文言 (`ui.key.via_palette`) に置き換える。`{key:…}` の `command_id` がカタログ (§11.6) に無い場合は CI でエラー (09 §11)。

キー集合の比較 (§17.3、09 §11.2) は複数形の接尾辞 (`.one` / `.other`) を除いた **基底キー** で行う (後の版で `en` だけが `.one` を持っても差分にしない)。

### 17.3 表示上の注意

- 日本語の文言は全角を含むのでセル幅が文字数と一致しない (「文字コード」は 5 文字で 10 セル)。ステータスバー・ヒントバーの項目は意味的モデルで渡し、frontend が表示幅 (全角 = 2 セル、03 §14) で切り詰め・省略する ([02 §8.1](02-architecture.md#81-方針-adr-0008)、04 §3 / §8.7)。v0.1 の画面文言は 04 §15.1 に置く。
- 日付・数値の書式: 最小限 (千区切りなし、ISO 8601)。ロケール依存の数値書式は導入しない。
- 文言の品質ゲート (v0.1 から): 同梱の `ja` のキー集合 (複数形の接尾辞を除いた基底キー) とコードの MessageKey 一覧 (言語ファイルに入れない `stderr.*` / `panic.*` / `cli.*` を除く) の差分を CI で検出し、**欠落・余剰ともエラー**。除いた `stderr.*` / `panic.*` / `cli.*` は、コードの MessageKey 一覧 (core が `StderrLine` で使うキーを含む) と eve-host / bin の定数表の差分を同じ CI で検査する (欠落はエラー。v0.2 からは英語と日本語の両方の文言があることも)。`hint.*` の長さ (全角 4 文字 / 半角 8 文字) と、`{key:…}` の CommandId がカタログ (§11.6) にあること、プレースホルダ名がコードの引数名と一致することも CI で検査する (09 §11)。`ja` の語彙・表記の品質は人手のレビュー (11)。言語間のキー集合の差分の検査は、`en` を同梱する v0.2 で加える (`ja` と `en` のそれぞれがコードの MessageKey 一覧と一致し、言語間の差分もゼロ)。

---

## 18. CLI 仕様

### 18.1 書式

```
evoeditor [OPTIONS] [--] [FILE]                                               # v0.1 (FILE は 1 つ)
evoeditor [OPTIONS] [--] [FILE | DIR | -]...                                  # 複数の FILE は v0.2、DIR は v0.4
evoeditor [OPTIONS] +LINE[:COL] FILE                                          # v0.2
evoeditor --version | --help
evoeditor --check-config | --dump-config | --list-commands | --dump-keymap   # v0.2 (--list-commands / --dump-keymap は v0.1 から隠しオプション)
evoeditor -                                                                   # 標準入力 (FR-120、v0.3)
evoeditor --headless --rpc stdio [--fake-ports] [--allow-write]              # dev feature `headless`、v0.2 以降 (02 §10.5)。配布ビルドでは予約名 (未対応と表示して終了コード 2)
```

パーサは lexopt ([OQ-24](10-decisions.md#oq-24-cli-引数パーサ))。サブコマンド (`evoeditor plugin install`, `evoeditor config edit`) が v1.X で必要になった時点で clap へ移行する (移行条件は OQ-24)。`--help` の文は MessageKey (`cli.*`) で識別し、**v0.1 は日本語のみ** (確定 (ユーザー回答)、OQ-67、§17.1。端末を初期化する前に stdout へ出し、書き方は stderr と同じ: コンソールなら `WriteConsoleW`、リダイレクト先なら UTF-8)。v0.2 から英語 (ASCII) と日本語を併記する。`--version` は版・ビルド情報だけ (翻訳する文言を持たない)。言語ファイルは引かない。使い方の誤りは `stderr.usage.*` の 4 つの葉のキー (04 §15.2) のどれかを出して終了コード 2: 未知のオプション等は `stderr.usage.generic`、FILE が 2 つ以上は `stderr.usage.multiple_files`、対応していない文字コード名は `stderr.usage.unknown_encoding`、駆動 API の予約名は `stderr.usage.dev_only`。

### 18.2 位置引数

| 形 | 意味 | 備考 |
|---|---|---|
| `FILE` | ファイルを開く (存在しなければ新規、FR-101) | **v0.1 は 0 個か 1 個** (単一文書。2 つ以上渡すと TUI を始めずに stderr へ `stderr.usage.multiple_files` (04 §15.2) を出し、終了コード 2 (使い方の誤り) で終わる、01 FR-101 AC-1)。**v0.2 は複数** (FR-103): すべてを開いた順にバッファとして開き、最初のものを表示する。`--readonly` と `--encoding` は全 FILE に掛かる。同じパスは 1 つのバッファだけ開く。2 つ目以降は読込中のバッファとして足し、読込を待たない (03 §9.8) |
| `DIR` | ワークスペースルートとして開き、ファインダを開く (v0.4) / v0.1 では無題の文書を開き、最初の Frame の後に `msg.cli.directory` (「フォルダなので開けません。無題の文書を開きました」、04 §15.2) を出す | 使い方の誤りにはしない (終了コード 0 の通常の起動)。ファイルを開くプロンプト (`file.open`) でフォルダを指定した場合は `msg.open.not_a_file` |
| `-` | 標準入力を無題バッファとして読む (FR-120、v0.3) | §18.5 |
| `FILE:LINE[:COL]` | 開いてその位置へ (FR-102、**v0.2**。v0.1 は `FILE:LINE` の形の引数を常にファイル名として扱う) | **曖昧性の解消**: まず `FILE:LINE:COL` 全体が存在するファイルなら位置指定なしで開く → 末尾の `:数字(:数字)?` を剥がしたものが存在すれば位置指定とみなす → どちらも無ければ **全体を新規ファイル名** とする。Windows の `C:\path` はドライブレター (`^[A-Za-z]:[\\/]`) を先に除外 |
| `+LINE` / `+LINE:COL` / `+` | **直後の** FILE に適用 (`+` 単独は末尾。Vim/nano 互換)。**v0.2** (FR-102) | `+LINE` が FILE より後に来た場合も直前の FILE に適用 (`evoeditor file +10` を許容)。v0.1 は `+` で始まる引数を「このバージョンでは未対応」(`info.cli.unsupported`) として無視し、ファイル名として開かない (v0.2 で意味が変わらないように) |
| `+/PATTERN` | 開いて最初の一致へ (Could) | — |

行・桁は 1 始まり。桁は **書記素単位** (表示列ではない。LSP 的な UTF-16 でもない。OQ-32 確定 (ユーザー回答))。`ui.column_unit = "display"` (v0.2) のときも CLI の桁は書記素単位のまま。範囲外は末尾に丸める。

### 18.3 オプション

v0.1 の公開オプションは **5 個** (ADR-0028: 引数解析と `--help` 文の保守を減らす。確定 (ユーザー回答)、v0.1 の範囲は案 L): `--version` `--help` `--readonly` `--encoding` `--` (+ 受理して無視する `--wait`)。**隠しオプションは 2 個** (`--help` に出さない): CI 照合用の `--list-commands` / `--dump-keymap`。計測用の `--bench-*` (`--bench-exit-after-first-frame` 等) は **配布ビルドに含めない** テスト専用のフラグで、cargo feature `test-hooks` を有効にしたビルドだけが受け付ける (09 §5.2 / §10.3。配布ビルドでは未知のオプションとして終了コード 2)。隠しオプションの数には入れない。`--config` `--no-config` `--log-level` `--set` `--check-config` `--dump-config` `--crash-report` `--licenses` と位置引数 `+LINE` は v0.2 (`--licenses` が無い間は、アーカイブに同梱する `THIRD-PARTY-NOTICES.md` が帰属表示の正、05 §17.1。ログの水準は環境変数 `EVE_LOG` で変える)。駆動 API の `--headless` `--rpc` `--fake-ports` `--allow-write` は dev feature `headless` のみ (v0.2 以降) で実装し、配布ビルドには機能が無い (OQ-120)。**配布ビルド (v0.1 から) はこの 4 個を予約名として受け付け**、どれかが渡されると TUI を始めずに stderr へ「このビルドでは未対応です (v0.2 以降の開発版機能)」(`stderr.usage.dev_only`、04 §15.2) を出し、**終了コード 2** で終わる (確定 (ユーザー回答)、[OQ-120](10-decisions.md#oq-120-ヘッドレス-rpc-を配布ビルドに同梱するか))。これらをファイル名の位置引数とは解釈しない (`--headless` という名前のファイルは `evoeditor -- --headless` で開く)。下の「未対応と知らせて無視する」扱いの例外である。後続版のもの (下表で版を持つもの) は書式を予約し、v0.1 では使い方エラーにせず「このバージョンでは未対応」と知らせて無視する (最初の Frame の後に `info.cli.unsupported`。端末を初期化しない `--version` / `--help` と一緒に渡された場合は無視するだけ)。採否が未定の `--check-update` はこれに含めず、採るまでは未知のオプションとして扱う (§18.8)。

| オプション | 短 | 意味 | P |
|---|---|---|---|
| `--readonly` | `-R` | 読み取り専用で開く | v0.1 |
| `--encoding <label>` | `-e` | 文字コードを指定して開く (FR-111)。受け付けるラベルは `file.default_encoding` の許容値 (§5.2) から `system` を除いたもの。v0.2 からは全ファイルに適用。対応していないラベルは使い方の誤りで、TUI を始めずに `stderr.usage.unknown_encoding` (04 §15.2) を出して終了コード 2 | v0.1 |
| `--line-ending <lf\|crlf>` | — | 新規ファイルの改行コード (`--set file.default_line_ending=` で代替可) | v0.4 |
| `--config <file>` | `-c` | ユーザー層の `config.toml` を差し替える (複数可、順にマージ) | v0.2 |
| `--config-dir <dir>` | — | `EVE_CONFIG_DIR` と同じ (環境変数で代替可) | v0.4 |
| `--no-config` | `-N` | ユーザー層・システム層・ワークスペース層を読まない (既定値のみ。トラブルシュート用。v0.1 は `EVE_CONFIG_DIR` を空のディレクトリに向けて代える) | v0.2 |
| `--no-workspace-config` | — | ワークスペース層のみ読まない | v0.4 |
| `--set <key>=<value>` | `-s` | コマンドライン層で 1 項目上書き (TOML 値として解釈。`--set view.wrap=soft`) | v0.2 |
| `--theme <name>` | — | `--set theme.name=` の短縮 | v0.4 |
| `--keymap <preset>` | — | `--set keys.preset=` の短縮 | v0.4 |
| `--locale <lang>` | — | UI 言語 (§17.1 の選択順の最上位)。値は `ja` / `en` (地域付きも可)。同梱していない言語名はログに警告を書いて次の段へ進む (使い方の誤りにはしない)。値が無いときは使い方の誤り (終了コード 2)。M8 で実装した | v0.2 |
| `--workspace <dir>` | `-w` | ワークスペースルートを明示 | v0.4 |
| `--trust` | — | このワークスペースを信頼して起動 (スクリプト用。警告表示) | v0.4 |
| `--no-session` | — | セッション復元・保存をしない | v0.4 |
| `--wait` | — | **受理して無視** (TUI は常にブロックする)。`$EDITOR="evoeditor --wait"` と書けるようにするための互換オプション | v0.1 (受理のみ) |
| `--stdout` | — | 終了時に (最後にアクティブだった) バッファ内容を標準出力へ書き、ファイルには保存しない。`cmd \| evoeditor - --stdout \| next` のパイプフィルタ用 (Could、OQ-99) | v0.4 |
| `--check-config` | — | 設定を読み診断を出力して終了 (端末初期化なし) | v0.2 |
| `--dump-config [--defaults] [--json]` | — | 解決済み設定 (または既定値) を出力。`--annotated` (注釈付き雛形) は v0.1 は同梱の `config.example.toml`、生成は v0.4 (§4.1) | v0.2 |
| `--dump-schema` | — | JSON Schema (Could) | v0.4 |
| `--list-commands [--json]` / `--dump-keymap [--json]` | — | §11.5 / §11.6 (CI 突合用) | v0.1 (隠し) |
| `--dump-languages` | — | §9.1 | v0.4 |
| `--log-level <level>` | — | `EVE_LOG` より優先 | v0.2 |
| `--log-file <path>` | — | ログ出力先の変更 | v0.4 |
| `--debug-commands` | — | `debug.*` をパレットに出す | v0.4 |
| `--crash-report` | — | 最新のクラッシュログを表示 (09 §10.2) | v0.2 |
| `--licenses` | — | 同梱の `THIRD-PARTY-NOTICES.md` (依存の帰属表示) を表示 (05 §17.1) | v0.2 (隠し) |
| `--bench-exit-after-first-frame` | — | 起動時間の計測用 (09 §5.2 の BM-101a / BM-101b)。最初の Frame を出力した直後に終了する。**配布ビルドに含めない** (`test-hooks` のビルドのみ、09 §10.3)。`--help` に出さない | v0.1 (`test-hooks` のみ) |
| `--bench-quit-immediately` | — | 終了時間の計測用 (BM-112)。起動してすぐ `app.quit` を実行する。**配布ビルドに含めない** (`test-hooks` のビルドのみ、09 §10.3)。`--help` に出さない | v0.2 (`test-hooks` のみ) |
| `--script <file>` | — | ヘッドレスのバッチ編集 (シナリオ DSL のファイルを実行する。02 §10.1 の X24)。**名前だけを予約** し、採るか・版・dev feature `headless` の配下に置くかは未定 (提案。採るまでは未知のオプションとして扱い、終了コード 2、§18.4) | 未定 (予約名のみ) |
| `--headless` / `--rpc <transport>` / `--fake-ports` / `--allow-write` | — | 駆動 API (02 §10.5)。dev feature `headless` のビルドだけが実装する。配布ビルドでは予約名で、「このビルドでは未対応です (v0.2 以降の開発版機能)」を出して終了コード 2 (確定 (ユーザー回答)、OQ-120) | v0.2 以降 (dev) / v0.1 から予約名 (配布) |
| `--version` | `-V` | 版・ビルド情報 (コミット、ターゲット、有効 feature) | v0.1 |
| `--help` | `-h` | ヘルプ | v0.1 |
| `--` | — | 以降は位置引数 (`-` で始まるファイル名用) | v0.1 |

v0.1 の 5 個: `--version` `--help` `--readonly` `--encoding` `--` (+ 受理のみの `--wait`、隠しの 2 個 `--list-commands` / `--dump-keymap`)。`test-hooks` のビルドだけの `--bench-*` は数えない。dev feature の `--headless` 系は v0.2 以降 (配布ビルドでは v0.1 から予約名で、終了コード 2。5 個には数えない)。更新の確認 `--check-update` は §18.8 で名前だけを予約する (名前は確定 (ユーザー回答)、採るかどうかは v0.4 以降に ADR-0041 で判断。5 個には数えない)。

**命名規則**: 実行して結果を出力し終了する操作や値の指定はサブコマンドにせず、`--check-config` / `--dump-config` / `--crash-report` / `--set` と同系統の `--` で始まる小文字・ハイフン区切りのオプションにする (位置引数 `FILE` と衝突させない。`--check-update` もこれに従う)。

環境変数: `EVE_CONFIG_DIR`, `EVE_STATE_DIR`, `EVE_DATA_DIR`, `EVE_CACHE_DIR` (ディレクトリの個別上書き、05 §7.1), `EVE_LOG` (ログの水準。v0.1 で水準を変える唯一の手段), `EVE_LOG_STDERR` (`1` でログを stderr にも出す。ヘッドレス・テスト用、05 §7.3), `NO_COLOR` (色なし = `theme.color_mode = "none"` 相当。各ロールの `mono` 段を使う、04 §12.4), `COLORTERM` / `TERM` (04/05 が解釈), `EVE_LANG` (UI 言語。`ja` / `en`、地域付きも可。v0.2、§17.1。v0.1 は読まない), `EVE_NO_SESSION` (`1` で `--no-session` と同じ: セッションの復元も保存もしない。v0.4、§16)。**子プロセスに渡す変数 (予約名。evoeditor が書く側で、読む側ではない)**: `EVE_FILE` (編集中の文書のパス)、`EVE_LINE` (カーソル行、1 始まり)。`ProcessRunner` で起動するフォーマッタ・外部フィルタ (§15。v0.4) の子プロセスにだけ設定する。無題の文書では `EVE_FILE` を設定しない。`EVE_LOG` 等の自前変数は子に渡さない (05 §13.5)。

テスト・計測用のビルドだけが読む環境変数 (配布ビルドは読まず、実行ファイルに名前も含まない。09 §3.6 / §5、検査は 09 の `build-release-smoke`): `EVE_TEST_HOOK` / `EVE_TEST_HOOK_LOG` (障害注入のフックとその記録。cargo feature `test-hooks` のビルドのみ、09 §3.6)、`EVE_TRACE_FILE` / `EVE_TRACE_FORMAT` (`tracing` の span の記録。cargo feature `bench-trace` を加えた計測用ビルド (release + `test-hooks` + `bench-trace`) のみ、09 §5)。`--help` と利用者向けの文書には出さない。

### 18.4 終了コード

**本表が終了コードの正本** (OQ-28 確定 (ユーザー回答)。OS 差 (シグナル番号、コンソールイベント、既定ハンドラへの再送) は 05 §11.5)。

| コード | 意味 |
|---|---|
| `0` | 正常終了 (保存したか否かに **関係なく**。Vim/nano と同じ。`$EDITOR` 利用側は内容で判断する) |
| `1` | 実行時エラーで終了 (I/O 致命エラー、起動時の設定ディレクトリ不可等。パニックは `70`)。メッセージは stderr (端末復元後) |
| `2` | 使い方の誤り (不正なオプション、`+LINE` に数値以外)。配布ビルドに渡された駆動 API の予約名 (`--headless` / `--rpc` / `--fake-ports` / `--allow-write`、§18.3) もこれ |
| `3` | `--check-config` でエラー診断あり (警告のみなら 0) (v0.2。`--check-config` と同時) |
| `4` | `app.quit_with_error` コマンド (Vim の `:cq` 相当) による **明示的な失敗終了**。`$GIT_EDITOR` でコミットを中止させる用途 (v0.2。v0.1 では発生しないが番号は予約する) |
| `5` | 端末初期化失敗 (TTY でない・raw モード不可・`TERM=dumb`・Windows で VT 非対応や `CONOUT$` 不可)。ヒントとして `evoeditor - < file` でなく `cmd \| evoeditor -` と TTY が必要な旨を出す |
| `70` | パニック (EX_SOFTWARE)。緊急退避・端末復元・クラッシュログ保存の後 (02 §11.2 / 05 §11.4)。`1` と分けるのは、スクリプトがクラッシュと I/O エラーを区別できるようにするため |
| `128 + n` | シグナル n による終了 (Unix 慣習。SIGHUP = 129、SIGINT = 130、SIGTERM = 143)。緊急退避してから既定ハンドラへ再送 (05 §11.5)。Unix を出荷する版から。v0.1 (Windows) のコンソールイベントによる終了は 05 §11.5 |

### 18.5 標準入力・標準出力

- `evoeditor -` (FR-120、v0.3): stdin を **EOF まで** 読んでから無題バッファとして開く (読込中は進捗表示、FR-122)。入力デバイスは stdin ではなく `/dev/tty` (Unix) / `CONIN$` (Windows) を開く (05)。stdin が TTY なのに `-` が指定された場合は空の無題バッファ。
- stdin が TTY でなく `-` もない場合 (例: cron から起動) は、終了コード 5 で終了 (フリーズしない)。
- stdout が TTY でない場合 (`evoeditor > log`): 描画先は常に TTY (`/dev/tty`) とし、stdout は `--stdout` のためだけに使う。
- `evoeditor -` で開いたバッファの保存は常に「名前を付けて保存」になる。`--stdout` 指定時は終了時にバッファを stdout へ流し、`app.quit_with_error` なら何も出力せず終了コード 4。

### 18.6 `$EDITOR` / `$GIT_EDITOR` / `$VISUAL` として使われたとき

想定呼び出し: `git commit` (`.git/COMMIT_EDITMSG`)、`git rebase -i`、`crontab -e`、`visudo`、`kubectl edit`、`less` の `v`、シェルの `edit-and-execute-command`。

| 観点 | 挙動 |
|---|---|
| 起動 | 1 ファイル・高速・即終了が最重要。NFR-101/112 がそのまま効く。設定読込に失敗しても起動する (FR-703) |
| 判定 (v0.4。言語判定と同時) | 専用のフラグは不要。パス末尾 (`COMMIT_EDITMSG`, `git-rebase-todo`, `MERGE_MSG`, `/tmp/crontab.*`, `*.sudoers.tmp`) を `[[language]]` の `file_names`/`file_globs` で言語判定するだけ (gitcommit 等、§9.3)。v0.1 は判定しない (どのファイルも同じ扱い) |
| セッション (v0.4。セッション復元 §16 と同時) | 復元しない・保存しない (位置記憶も `gitcommit` 等では無効。「前回のコミットメッセージの 3 行目」に飛ばれると困る)。`[[language]]` に `remember_cursor = false` を持たせる |
| ワークスペース (v0.4。§12 と同時。v0.1 はワークスペース層を読まない) | `.git` からリポジトリをルートとして検出 (ファインダで他ファイルを参照できる)。ワークスペース層の設定は **安全な項目のみ** 適用 (信頼ダイアログで起動を邪魔しない: 未信頼かつ危険項目があってもダイアログは出さず、ステータス記号のみ) |
| ヒントバー (v0.1) | 常時 Ctrl+S 保存 / Ctrl+Q 終了 を表示 (初見ユーザーの `git commit` 迷子対策、NFR-601) |
| 終了 (v0.1) | 保存して終了 (`file.save` → `app.quit`) は 2 操作。未保存のまま Ctrl+Q は確認ダイアログ (FR-107)。「保存せず終了」= git 側は未変更として扱う。v0.1 でコミットを中止するにはメッセージを空にして保存・終了する (git が空のメッセージで中止する)。1 操作の `app.save_and_quit` と、終了コード 4 で中止させる `app.quit_with_error` は v0.2 |
| `--wait` | §18.3。`$EDITOR="evoeditor --wait"` と書かれても TUI では無害 |
| 保存方式 (v0.1。Windows の一時ディレクトリの判定。Unix は v0.2) | 原子的置換 (ADR-0016) は `visudo`/`crontab` のように **同じ inode を期待する** 呼出元で問題になり得る → 一時ファイルと判断できる場合は **確認なしで直接上書き (in-place) にフォールバック** (事前バックアップは in-place の規則どおり取る。03 §9.3 の「直接上書き (in-place) の規則」)。判定規則の正本は [05 §4.4](05-platform.md#44-原子的置換を諦めて-write_in_place-にフォールバックする条件) (正準化後の一時ディレクトリ集合との比較 + `file.in_place_patterns` (§5.2) のパターン表。シンボリックリンク経由の `$TMPDIR` や macOS `/private/tmp` を誤判定しない。OQ-34) |
| 文字コード (v0.4 の言語定義と同時、Could) | 一時ファイルは短い → chardetng 推定が不安定になり得る。`gitcommit` 等では `default_encoding` を優先 (言語定義に `encoding = "utf-8"` を持たせる) |

### 18.7 シェル補完

lexopt は補完スクリプトを生成しない。`evoeditor --list-commands` 等の機械可読出力を用意するに留め、補完スクリプトは手書き (bash/zsh/fish/PowerShell) を `contrib/` に置く (v0.4 Could)。

### 18.8 更新に関わる予約 (`--check-update` と前回起動した版の記録)

エディタ内に更新機能を持たない ([ADR-0040](10-decisions.md#adr-0040-更新の方針-エディタ内に更新機能を持たない)、確定 (ユーザー回答))。更新の手順は 05 §18.10 が正本。本節は、将来入るかもしれない CLI と設定キーの **名前の予約** と、FR-518 (v0.2) が使う状態ファイルを定める。**v0.1 の CLI オプション (公開 5 個) と設定キー (7 個) は変えない**。

**(1) `evoeditor --check-update` (v0.4 以降。名前は確定 (ユーザー回答)、採るかどうかは [ADR-0041](10-decisions.md#adr-0041-明示実行の更新確認-版情報の取得のみ) で提案中)**:

| 項目 | 予約の内容 |
|---|---|
| 書式 | `evoeditor --check-update`。署名付きの版情報だけを取得し、新しい版の有無を標準出力に出して終了する (端末を初期化しない)。ダウンロード・置換を行うオプションは作らない |
| 名前 | オプション `--check-update` (確定 (ユーザー回答)、ADR-0041)。§18.3 の命名規則に従い、位置引数 `FILE` (§18.2) と衝突しない (`update` という名前のファイルも位置引数で開ける)。ADR-0041 で採るまでは未知のオプションとして扱う (§18.4) |
| 設定キー | `update.check` (`"off"` が既定 / `"manual"`)。ユーザー層とコマンドライン層 (`--set`) の値だけを使い、**ワークスペース層の値は無視して警告する** (§12.3 の危険項目と同じ扱い)。自動・定期の確認の値は作らない。ADR-0041 が確定するまで §5 の表には載せず、`[update]` を書いた設定は「このバージョンでは未対応」の情報診断にする |
| 終了コード | 確定時に §18.4 に加える (案: 新しい版あり / なし / 確認できなかった、を区別する) |
| feature | HTTPS のクライアントを含むので cargo feature で分ける (ADR-0041 決定 5)。feature を外したビルドでは使えない旨を示して終了コード 2 |

**(2) 前回起動した版の記録 (FR-518、v0.2)**:

| 項目 | 内容 |
|---|---|
| 場所 | `state_dir/last_version.toml` (05 §7.1。`EVE_STATE_DIR` で変えられる)。`state_dir` を使えないとき (05 §7.1) は表示も記録もしない |
| 書式 | TOML。`format_version = 1` (必須)、`last_run_version = "0.2.0"` (必須。最後に起動した evoeditor の版、SemVer の文字列)、`updated_at` (任意。UTC、RFC 3339) |
| 権限・書込 | 所有者のみ (NFR-505、05 §5.6)。一時ファイル + 原子的置換で書き、最終書込勝ち (05 §7.5 (b))。未知の `format_version` は 05 §7.5 (a) の規約 (読まずに退避して警告) |
| 失敗 | 読めない・書けない・`state_dir` を作れないときは **表示を省略するだけ** にする (メッセージ行にエラーを出さず、編集を妨げない。次の編集セッションで再び試す) |
| 対象の起動 | **編集セッションを開始したときだけ** 読み・比較・表示・記録を行う。`--version` / `--help` / `--check-config` / `--dump-config` / `--crash-report` / `--licenses` / `--list-commands` / `--dump-keymap` 等の **非対話の起動** (端末を初期化せずに終了するもの) では、読みも記録も表示もしない |
| 時期 | 起動経路では読み書きしない (NFR-101)。最初の Frame を描いた後に `io` ワーカーで読み、比較してから書く (NFR-101 の計測区間の外) |
| ディレクトリの作成 | `config_dir` はユーザーが設定を書くまで作らない。`state_dir` は最初の編集セッションで作る (`state_dir/log` はログのため初回起動で作成。05 §7.1)。アンインストールでは `state_dir` ごと `last_version.toml` も消す (05 §18.8) |
| 比較 | 記録が無い → 表示せず記録を作る。記録の版 < 現在の版 → 表示して記録を更新する。同じ → 何もしない。記録の版 > 現在の版 (ダウングレード) → 表示せず記録を現在の版に書き換える。記録が壊れている・版の文字列が不正 → 表示せず書き直す |
| 表示 | 情報メッセージ 1 行。MessageKey `info.app.updated` (例: 「evoeditor {version} に更新しました: {summary}」)。`{summary}` は同梱の `info.app.release_summary` (その版の要点の 1 文。リリースのたびに同梱する全言語を同時に書き換える。メッセージ行の幅を超える部分は省略する)。詳しくはリリースノートを見るよう促すだけで、URL を開かない・取得しない |
| 設定 | 設定キーを設けない (版ごとに 1 回だけ表示し、メッセージ履歴 (FR-505、v0.2) から読み直せる) |

---

## 19. サードパーティ拡張のライセンスと信頼

### 19.1 配布物の種類と危険度

| 種類 | 形式 | 実行を伴うか | 共有時の扱い |
|---|---|---|---|
| テーマ / キーマップ / ロケール / 言語定義 (データ項目) | TOML | × | **安全**。コピーして置くだけ。`evoeditor --check-config` で検証可 |
| tree-sitter 文法 (外部ロード、OQ-22) / ハイライトクエリ | 共有ライブラリ (.so/.dll/.dylib) または .wasm / .scm | 共有ライブラリは **ネイティブコード実行** (= プラグインと同等の危険)。.wasm/.scm はデータ | 共有ライブラリ形式はユーザー層のみ・明示承認。WASM 文法を優先候補にする理由 |
| 言語サーバ・フォーマッタ定義 | TOML (コマンド行) | ○ (間接) | ワークスペース層は信頼後 (§12) |
| プラグイン (v1.X) | manifest + コード | ○ | 権限承認 (§14.3) |

### 19.2 ライセンス

- 本体: [ADR-0018](10-decisions.md#adr-0018-ライセンスと依存ライセンス方針) / [OQ-19](10-decisions.md#oq-19-ライセンス形態)。
- **同梱物** (組込みテーマ・ハイライトクエリ・文法・ロケール・言語定義) の流用元ライセンスは `THIRD-PARTY-NOTICES.md` に自動生成で列挙 (cargo-about / cargo-deny、05 §17.1、09)。Helix 等のクエリを流用する場合は MPL-2.0 のファイル単位条件に注意 (ADR-0018 の個別判断)。
- **サードパーティ拡張**: manifest の `license` (SPDX) を **必須** とし、未記載はインストール時に警告 (拒否はしない。ユーザーの判断)。テーマ/言語定義ファイルの `[meta].license` は任意だが配布時推奨。本体は拡張のライセンスについて責任を持たない旨をドキュメントに明記。
- プラグイン API (プロトコル定義・manifest スキーマ・テストハーネス) は公開コアと同じ **Apache-2.0** で公開する (確定した本体のライセンス、OQ-19)。Apache-2.0 は商用・プロプライエタリなプラグインを妨げない。プロトコル定義と manifest スキーマだけは、実装者の帰属表示の負担をさらに下げるため CC0-1.0 で出すことも候補とする (提案、エコシステムの広がりを優先)。

### 19.3 信頼の段階

| 段階 | 対象 | 根拠 |
|---|---|---|
| 組込み | 本体に同梱 | リリース署名で担保 (05) |
| ユーザーが明示インストール | `<config_dir>/plugins/`、`themes/` 等 | インストール時の権限承認。更新で権限が増えたら再承認。**作者の署名検証は Could** (minisign 鍵を manifest に記載し、更新時に同一鍵を要求する TOFU 方式。sigstore 等の本格的な PKI は v1.X 以降に検討) |
| ワークスペースが推奨 | `.eve/plugins/`、`.eve/config.toml` の `[plugins.recommend]` | **自動インストールしない**。一覧表示して個別承認。信頼済みワークスペースでも同じ (git clone したら動く、は作らない) |
| レジストリ | — | v1.X 時点では作らない。将来作る場合も「レジストリ掲載 = 審査済み」とは言わない設計 (名前空間の予約と改竄検知のみ) |

---

## 20. 未決事項

番号は [10 §2](10-decisions.md#2-未決事項-open-questions) の OQ 番号。区分・詳細は 10 を正とする。本表は本書に関わる未決事項の結論案だけを記す (確定した事項は本文に反映済み)。

| OQ | 件名 | 結論案 | 区分 / 影響 / 決定期限 |
|---|---|---|---|
| OQ-94 | システム層 (L1) を提供するか | v0.4 Could で提供 (組織共通の既定値を配る要望があれば。L1 は L2 より優先度が低いので値を固定できない)。`lock = true` で上位層による上書きを禁じる管理者機能 (AI の無効の固定等) は v1.X 以降 | 裁量 / 05, 07 / v1.0 リリース前 |
| OQ-95 | `.editorconfig` の実装 | 自前 (約 200 行、glob は `globset` を再利用) を第一候補とし、v0.4 着手時に依存評価 ([02 §13.0](02-architecture.md#130-依存追加の評価基準)) で決める | 裁量 / eve-config の依存、09 fuzz / v0.4 着手前 |
| OQ-35 | ワークスペース設定の安全性 (キーマップ・所有者検査) | キーマップは **追加のみ** (既存割当の上書き・`exec`・脱出キーは信頼後)。他者所有 / world-writable なディレクトリの設定は無視 (§7.4 / §12.1 / §12.3) | PO / 07 §7.4, §12 / v0.4 着手前 |
| OQ-96 | モードラインのファイルタイプ名のみ対応 | 当面非対応。要望が出たら許可リスト方式で Could | 裁量 / 07 §9.2 / — |
| OQ-43 | プラグイン権限の表示と強制 | 承認は「任意コード実行」の 1 点、`net` / `fs` は自己申告 (`[declares]`) として表示のみ (§13.3 / §14.3)。OS サンドボックス (seccomp/AppContainer/sandbox-exec) は Could。強制が要件化したら Stage 3 (WASM) を前倒し | PO / 07 §14, NFR-508 / v1.X 着手前 (OQ-12 と同時) |
| OQ-98 | プラグイン署名方式 | minisign 鍵の TOFU (レジストリなしで運用可能) | 裁量 / 07 §19 / v1.X 着手前 |
| OQ-99 | `--stdout` (パイプフィルタ) の提供 | v0.4 Could。stdin/stdout を両方使うため端末入出力を `/dev/tty` / `CONIN$` 固定にする設計が前提 (05) | 裁量 / 05, 07 §18.5 / v1.0 リリース前 |
| OQ-141 | 管理モード (`ai.providers.*.runtime.*`) の承認の持ち方 (§12.3) | ユーザー層でも初回起動時とコマンド行が変わるごとに `exec` 承認を求める (設定ファイルは同期ツール・dotfiles 共有で他マシンから来ることがあり、「書いた = 承認した」とは限らない)。記録は状態ディレクトリ (`trust.toml` と同じ) で、設定ファイルには書き戻さない | 裁量 / 06 §3.3.3, 07 §12.3 / v1.L-b 着手前 |
| OQ-142 | `ai.providers.*.kind` の `ollama` を独立した kind にするか (§5.12.1) | 独立 kind `ollama` (能力取得と FIM のネイティブ補助を明示し、設定を読む人に接続先の種類が見える。実装は 06 §3.3.4 のとおり `openai_compatible` のアダプタを共用) | 裁量 / 06, 07 / v1.L-a 着手前 |
| OQ-120 | ヘッドレス RPC (駆動 API) を配布ビルドに同梱するか | v0.x は dev feature `headless` のみ。`--headless` 系オプションは §18.3 で予約 (隠し)。配布ビルドで予約名として受け付け終了コード 2 で終えることは確定 (ユーザー回答) | PO / 02 §10.5, 07 §18 / v0.4 着手前 |
| OQ-124 | 背景色ヒント `COLORFGBG` の利用 | `theme.background = "auto"` のヒントとしてのみ使う (背景まで塗るテーマ、v0.2)。`default` は背景非依存 | 裁量 / 04 §5.2a, 07 §5.4 / v0.2 着手前 |
