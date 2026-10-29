# 05. プラットフォーム (Platform)

| 項目 | 内容 |
|---|---|
| 文書ID | 05-platform |
| 状態 | ドラフト |
| 主対象 crate | `eve-platform`, `evoeditor` (bin)。一部 `eve-tui` (OS 固有の端末挙動) |
| 前提 | [00-overview](00-overview.md) / [01-requirements](01-requirements.md) / [02-architecture](02-architecture.md) |
| 決定記録 | [10-decisions](10-decisions.md) (ADR-0016, ADR-0018, ADR-0019, [ADR-0042](10-decisions.md#adr-0042-v01-の最小核-案-lwindows-のみの出荷日本語のみの-ui) (v0.1 の最小核・Windows のみの出荷・日本語のみの UI)、ADR-0040 / 0041 (更新。§18.10〜§18.14)、OQ-14, OQ-15, OQ-20, OQ-166)。ライセンス・寄稿・配布の方針の正本は [12-licensing-distribution](12-licensing-distribution.md) |
| 責務 ([02 §12.1](02-architecture.md#121-文書と-crate-の対応)) | Ports の OS 実装規則とエラー写像 (trait の正本は 02 §4.4 / §4.5)、原子的保存の OS 別手順、権限/ACL/シンボリックリンク、パス、設定/状態/ログのディレクトリ、クリップボード、ファイル監視、シグナル/コンソールイベント、端末互換性マトリクス、IME の OS 別注意、ビルド・配布 |
| 決めないこと | Port の trait シグネチャ・型名・エラー分類 (02 §4.4 / §4.5)、緊急退避の方式と manifest の形式 (02 §11.2.1)、文字コード変換アルゴリズム (03)、描画・端末能力判定の実装 (04)、設定項目の書式・既定値 (07 §5)、終了コード (07 §18.4)、テスト手順 (09) |
| v0.1 で読む節 | §1〜§10 (§2.5〜§2.7 を除く) と §11 の **Windows の部分** (§4.3・§9.2・§9.3・§11.1・§11.3 と、§11.2 の「セッション終了の受け方」を除く)、§16・§17、§18.1〜§18.3、§18.6、§18.8〜§18.14 (利用者向け文書の内容。Windows の手順) (一覧の正本は [README §3.1](README.md#31-v01-の実装で読む節)) |

> **v0.1 の実装範囲は Windows のみ** (確定 (ユーザー回答)、ADR-0042、OQ-15): 対象は Windows 11 の Windows Terminal と conhost (Windows 10 は公式には対応しない、§1.1) で、macOS / Linux は v0.1 では出荷せず、**v0.2 で出荷する** (確定 (ユーザー回答)。出荷には実機確認の手段 (基準機の追加、10 OQ-23 (d)) の確保が前提で、確保できない OS は次の版に延期する (確定 (ユーザー回答)、10 の残件 #3))。本書の Unix / macOS / Linux の記述 (rename + fsync の保存手順、モード・所有者、シグナル、XDG のパス、`pbcopy` / `wl-copy` / `xclip` / `xsel`・OSC 52 のクリップボード、WSL・SSH、musl 静的ビルド、macOS の署名・公証、Linux パッケージ等) は **v0.2 (macOS / Linux 出荷時) の仕様** として、各節の冒頭または行に明記する。core 系 crate (OS に触れない crate) は OS 非依存を保ち、CI の Linux x86_64 ジョブ (Linux CI の対象 crate (02 §4.7) のコンパイルとヘッドレスのテスト) で移植性を守る (§1.2)。
>
> 本書のコード片は仕様説明のための擬似コード (Rust 風) であり、そのまま実装する API ではない。
> 設計原則の優先順位 ([00 §4](00-overview.md#4-設計原則-優先順位付き)) に従い、本書では **P1 (データを失わない) > P2 (同意なく外へ出さない) > P6 (どの OS でも同じ体験)** を判断軸にする。OS 固有の便利機能は platform 層の「任意機能」として扱い、core は Port 以外の手段で OS に触れない (NFR-406, R3)。

---

## 目次

1. [対象プラットフォーム](#1-対象プラットフォーム)
2. [Ports の OS 実装規則とエラー写像](#2-ports-の-os-実装規則とエラー写像)
3. [パス](#3-パス)
4. [ファイル I/O と原子的保存](#4-ファイル-io-と原子的保存)
5. [権限・属性・ロック・外部変更検知](#5-権限属性ロック外部変更検知)
6. [改行コード・文字コードの OS 既定](#6-改行コード文字コードの-os-既定)
7. [ディレクトリ (設定 / 状態 / データ / キャッシュ / ログ)](#7-ディレクトリ-設定--状態--データ--キャッシュ--ログ)
8. [クリップボード](#8-クリップボード)
9. [端末の OS 固有事項と互換性マトリクス](#9-端末の-os-固有事項と互換性マトリクス)
10. [IME の OS 別注意](#10-ime-の-os-別注意)
11. [シグナル・コンソールイベント・終了](#11-シグナルコンソールイベント終了)
12. [ファイル監視](#12-ファイル監視)
13. [シェル・外部プロセス起動](#13-シェル外部プロセス起動)
14. [資格情報ストア (v1.L)](#14-資格情報ストア-v1l)
15. [WSL / SSH / コンテナ環境](#15-wsl--ssh--コンテナ環境)
16. [ビルド・ツールチェイン・MSRV](#16-ビルドツールチェインmsrv)
17. [依存のライセンス・ビルド時間・サイズ](#17-依存のライセンスビルド時間サイズ)
18. [配布 (パッケージ・署名・公証)](#18-配布-パッケージ署名公証)
19. [未決事項](#19-未決事項)

---

## 1. 対象プラットフォーム

### 1.1 OS・アーキテクチャ・ターゲット (NFR-401 / OQ-15)

| OS | バージョン | アーキ | Rust ターゲット triple | Rust ティア (執筆時点) | 配布形態 | 版 | 備考 |
|---|---|---|---|---|---|---|---|
| Windows | 11 | x86_64 | `x86_64-pc-windows-msvc` | Tier 1 | zip (単一 exe) | **v0.1** (唯一の出荷対象) | 確定 (ユーザー回答)、OQ-15。基準機 (01 §3.0) と同じ Windows 11 で確かめる。MSVC ツールチェインを採用 (GNU 版は配布しない) |
| Windows | 10 (1809 以降、サポート中の版) | x86_64 | `x86_64-pc-windows-msvc` | Tier 1 | zip (v0.1 と同じ) | **公式には対応しない** (確定 (ユーザー回答)、2026-10-25。PO が指示するまで対象に加えない) | v0.1 の対象にしない (確定 (ユーザー回答)、OQ-15。検証環境が無い)。下の「Windows 10 を対象に加えるときの確認事項」 |
| Windows | 10 / 11 on ARM | aarch64 | `aarch64-pc-windows-msvc` | Tier 1 (Rust 1.91 で昇格) | zip | v0.2 以降 (Tier B のビルドは v0.2) | **Tier B (ビルドのみ)**。Rust のティアとは別の、本プロジェクトの配布上の区分。実機テストは [09 §6](09-testing-quality.md) のランナー可用性に依存 |
| macOS | 直近 3 メジャー (執筆 2026-10 時点: 26 / 15 / 14) | aarch64 | `aarch64-apple-darwin` | Tier 1 | tar.gz | v0.2 (macOS / Linux 出荷時) | Apple Silicon。ad-hoc 署名必須 (§18.4) |
| macOS | 同上 | x86_64 | `x86_64-apple-darwin` | Tier 2 (host tools。Rust 1.90 で降格) | tar.gz | v0.2 (macOS / Linux 出荷時) | Intel Mac。出荷時は **Tier B (ビルドのみ)** (CI の x86_64 macOS ランナーの減少・有料化) |
| Linux | 任意のディストリビューション (静的リンク。カーネル 3.2 以降) | x86_64 | `x86_64-unknown-linux-musl` | Tier 2 | tar.gz | v0.2 (macOS / Linux 出荷時) | **Linux を出荷するときの配布物はこれを基本にする** (OQ-112、NFR-206)。完全静的で glibc の版に依存せず、Alpine・古い環境・コンテナでそのまま動く。アロケータは §17.3。v0.1 の CI の Linux ジョブ (ビルドとヘッドレスのテストのみ、§1.2) はこのターゲットではなくランナーのホスト (`x86_64-unknown-linux-gnu`) で行い、配布物は作らない |
| Linux | 任意のディストリビューション (静的リンク。カーネル 4.1 以降) | aarch64 | `aarch64-unknown-linux-musl` | Tier 2 | tar.gz | v0.2 (macOS / Linux 出荷時) | Raspberry Pi 4+ (64bit OS)、AWS Graviton、Ampere |
| Linux | glibc 2.31 以降 (Ubuntu 20.04 / Debian 11 / RHEL 9 相当) | x86_64 / aarch64 | `*-unknown-linux-gnu` | Tier 1 | tar.gz (v0.4: deb, rpm) | v0.2 以降 (条件付き) | **条件付き追加** (OQ-112): Linux を出荷するとき、musl 版が BM-104 で gnu 版比 1.5 倍以上遅い、または BM-202 が 1.3 倍以上なら同時に追加する。そうでなければ deb / rpm を作る v0.4 で追加。ビルドは §16.2 (zigbuild で glibc 2.31 に固定) |

- **非対応 (ビルドできれば歓迎、CI 非保証)**: FreeBSD / OpenBSD / NetBSD、illumos、Android (Termux)、32bit OS 全般、Windows 7/8.1 (Rust 1.78 以降が非対応)、Windows 10 1809 未満 (ConPTY / VT が無い)、`x86_64-pc-windows-gnu`。v0.1 の Windows 10 (1809 以降) は「非対応」ではなく「対象外 (未確認)」で、起動を拒まない (§9.1 の VT の確認に通れば動く) が、動作を保証しない。
- **Windows 10 を対象に加えるときの確認事項 (公式には対応しないので、PO が指示したときだけ使う)**: (1) 1809〜2004 の ConPTY は代替スクリーンのスクロールバックとリサイズに不具合がある (§9.1 の ConPTY の癖)。対象にする最小の版を決め、それより前の版の扱い (既知の制約にするか、対象外にするか) を決める。(2) その時点の Windows Terminal が動く最小の Windows 10 の版を確かめる (**要確認**)。(3) S-06 / S-07 / S-08 / S-11 / S-20 と互換性マトリクス (§9.4) の Windows Terminal・conhost の行を Windows 10 の環境 (実機または VM) で確かめる。Microsoft による Windows 10 の通常のサポートは 2025-10 に終了している (LTSC と拡張セキュリティ更新を除く) ので、加えるかどうか自体をその版の着手前に決める (検証環境の確保は OQ-23 (d) と同じ扱い)。
- **ユニバーサルバイナリ (macOS、macOS 出荷時)**: `lipo` で結合した 1 ファイルを Homebrew 向けに提供するか否かは §19。
- **判定方法**: 実行時に OS バージョンを調べて機能を切り替えることは原則しない (P6)。Windows のみ、ConPTY 由来の挙動差を補うため `RtlGetVersion` 相当でビルド番号を取得し、**ログに記録するだけ** に使う (分岐には使わない)。

### 1.2 「対応」の定義

| レベル | 意味 | 保証 |
|---|---|---|
| Tier A (Must) | **v0.1: Windows x86_64 のみ**。macOS aarch64・Linux x86_64 / aarch64 (musl) は v0.2 で Tier A に加える | CI でビルド + 自動テスト、リリース前に手動互換マトリクス (§9.4。v0.1 は Windows Terminal と conhost) を実施 |
| Tier B (Should、ビルドのみ) | macOS x86_64 (Intel)、Windows aarch64、Linux gnu 版 (追加する場合)。**Tier B のビルドは v0.2 から** | CI でビルド。自動テストは利用できるランナーがある場合のみ。手動確認はベストエフォート |
| 移植性の検査 (出荷しない) | Linux x86_64 (v0.1) | PR ごとに CI で Linux CI の対象 crate (core 系 crate + `eve-host` + `eve-testkit`、02 §4.7) のコンパイルとヘッドレスのテストを行う (09 §6)。配布物は作らず、「対応」とは呼ばない。core 系 crate が OS 非依存であることを守るための検査 |
| Tier C | 非対応リスト | 報告は受けるが修正を約束しない |

**v0.1 で「対応」と呼ぶのは Windows x86_64 (Windows 11) の Windows Terminal と conhost だけ** である (確定 (ユーザー回答)、OQ-15)。開発環境は Windows のみで、macOS / Linux の実機確認の手段が無いので、**v0.1 では macOS / Linux を出荷しない** (動作未確認のまま配布することもしない)。正本は [01 §4.6](01-requirements.md#46-v01-の既知の制約-リリースノートとヘルプに明記する) で、リリースノートと README にも載せる。

| OS | v0.1 の状況 | 内容 |
|---|---|---|
| Windows | **対応・動作確認済み** (開発機) | 開発機 (01 §3.0 の基準機) で実機の動作確認と性能の判定を行う。CI でもビルドと自動テストを行う |
| macOS | **v0.1 では出荷しない** | **v0.2 で出荷する** (確定 (ユーザー回答)。実機確認の手段の確保が前提)。CI のジョブも v0.2 から |
| Linux | **v0.1 では出荷しない** | **v0.2 で出荷する** (同上)。v0.1 の CI は Linux CI の対象 crate (02 §4.7) のビルドとヘッドレスのテストだけを行う (移植性の検査) |

---

## 2. Ports の OS 実装規則とエラー写像

Port の trait シグネチャ・型名・エラー分類の正本は [02 §4.4](02-architecture.md#44-ports-core-が定義し外側が実装する-trait) / [§4.5](02-architecture.md#45-filesystem-port-正本) である (02 §4.4 規則 5)。**`FileSystem` は 02 §4.5 が trait の正本** で、本書 §2.1 は OS 実装規則のみを書く。その他の Port (`Paths` / `Clock` / `JobSpawner` / `FileWatcher` / `ProcessRunner` / `SecretStore`) の trait は **各版の着手時に 02 §4.4 で確定する** (規則 6)。本節は OS 実装規則と、02 への **案** (下の擬似コードはすべて案) を書く。`ProcessRunner::spawn_long_lived` (`LongLivedSpec`・`ProcessHandle`) の trait も 02 §4.4 が正本で、本書 (§2.6 / §13.4 / §13.7) は各 OS での実装だけを書く。trait は **`eve-core` が定義** し、`eve-platform` が本番実装、`eve-testkit` が偽実装を提供する。**v0.1 の本番実装は Windows のみ** で、本節と §3〜§5 の Unix の実装規則 (errno の写像、rename、モード・所有者等) は v0.2 (macOS / Linux 出荷時) に実装する。trait と core 系 crate は OS 非依存に保ち、CI の Linux ジョブで偽実装を使うヘッドレスのテストを回す (§1.2)。共通規約:

1. **同期 API のみ** (core は async を扱わない、ADR-0006)。時間のかかる操作はジョブとしてワーカーで実行されるため、Port 自体はブロッキングでよい。
2. **パスは `PathBuf`/`&Path`** (Port のシグネチャでは 02 §4.4 の `FsPath` (`PathBuf` の newtype)。非 UTF-8 パス保持のため `String` を使わない、NFR-404)。表示用文字列化は §3.5。
3. **エラー型は Port ごとの列挙型** (thiserror、ADR-0012)。各バリアントは「回復可能性」と「ユーザー向けヒント (`MessageKey`)」に写像できる。`FileSystem` のエラーは `FsError { kind: FsErrorKind, detail, source }` で、`kind` が分類、`detail` が細分 (`ReservedName` / `DanglingLink` / `ParentMissing` / `Hardlink` / `InPlacePattern` / `NotAFile` / `Network` / `CrossDevice`)、`source` が元の `std::io::Error` (OS エラーコードを失わない。ログ用) である (型の正本は 02 §4.5)。
4. **`Send` は必要なもののみ** (形の正本は 02 §4.4): `FileSystem`・`Clock`・`Paths` はジョブ (02 §4.4 の `JobPorts`) から使うので `Send + Sync`。`JobSpawner` はメインスレッドからだけ呼び、ジョブには渡さないので `Send` を要求しない。
5. **core はメインスレッドで Port の I/O を呼ばない** (ADR-0020、確定): `metadata` / `canonicalize` / `file_id` を含むすべての `FileSystem` 呼び出しはジョブ内で行う。ネットワーク FS では `stat` 1 回が秒単位でブロックし得る。03 §9 の手順はこの前提で書かれている。

### 2.1 `FileSystem`

**trait のシグネチャ・型名 (`FileMeta` / `FileStamp` / `SaveOptions` / `SaveReport`)・エラー分類 (`FsErrorKind`) の正本は [02 §4.5](02-architecture.md#45-filesystem-port-正本)** である (ADR-0020。03 §9.1 と本書は trait を **再定義しない**)。本書が定めるのは、02 §4.5 の各操作を **各 OS でどう実装するか** (手順・エラー写像・再試行・属性保持・`FsKind` 判定・孤児ファイルの命名) である。

| 02 §4.5 の操作 / 型 | 本書での実装規則 |
|---|---|
| `metadata()` | §5.2 (読み取り専用判定)、§5.4 (`FileMeta.file_id`)、§5.5 (`FsKind`)、§3.3 (シンボリックリンクは辿り、`FileMeta.link` にリンク自体の情報と dangling を入れる)。ジョブ内のみ (02 §4.4 規則 2) |
| `canonicalize()` | §3.3 (Windows の verbatim 接頭辞の扱い、macOS `/private`、ジョブ内のみ) |
| `open_read()` | §4.5 (チャンクサイズ、`EINTR`、共有モード、読込中の外部変更検知) |
| `write_atomic()` | §4.1 (共通手順)、§4.2 (Windows。v0.1)、§4.3 (Unix。v0.2)。T が存在し、§4.4 の一時ファイル規則に該当すれば **何も書かずに** `AtomicUnsupported` (detail = `InPlacePattern`) を返す (本層は in-place を自分で選ばない。core が乾式走査をしてから `write_in_place` を呼ぶ。02 §4.5 / 03 §9.3)。T が存在しなければ規則に該当しても §4.1 の「新規ファイル」の経路 (`SaveMethod::NewFile`) で書く。`SaveOptions.private = true` のときは一時ファイル・最終ファイルを所有者のみ権限で作る (§5.6) |
| `write_in_place()` | §4.4 (フォールバック条件と手順)。呼ぶのは core だけで、経路は (1) `AtomicUnsupported` の確認の後、(2) `InPlacePattern` (確認なし)、(3) `file.save_strategy = "in_place"` のとき core が最初から直接上書きを選ぶ場合、の 3 つ。いずれも T が存在し、乾式走査の後に呼ばれる (T が無ければ (3) でも `write_atomic` の新規ファイルの経路) |
| `inflight_for()` (v0.1) | §4.6 / §7.5 (開くパス 1 つ分の保存中フラグ `recovery/inflight/<hash(path)>/` だけを読み、残っているフラグを **すべて** `Vec<InflightRecord>` で返す (複数のインスタンス・複数回の中断で 1 つのキーに複数の `<pid>.toml` があり得る。無ければ空)。シンボリックリンクならリンクとリンク先の 2 キー。ディレクトリ全体は列挙しない。開くたびに呼ぶ (T の有無に依らない、§4.6)。解析できないフラグ (未知の版・必須キーの欠落・値の不正) も捨てずに `malformed = true` の記録として返す (§4.6 の「上書きの可能性あり」の行。フラグは消さない)) |
| `create_dir_all_private()` | §5.6 (Windows は `recovery/`・`backup/`・`swap/` を「所有者のみ」の保護付き DACL で作る。それ以外の `state_dir` 配下は継承。Unix 0700 は v0.2) |
| `remove_file()` | スワップ・一時ファイルの後片付け。Windows の共有違反は `Busy` 再試行 (§4.2) |
| `list_inflight()` (v0.2) | §4.6 / §7.5 (保存中フラグ `recovery/inflight/*.toml` の列挙と pid 判定)。**開くたびのディレクトリ列挙は行わない** |
| `write_recovery(doc)` (v0.1) | §7.5 (「退避先に保存」の `rescue/<UTC timestamp>-<pid>/` を §5.6 の「所有者のみ」で新しく作る。UTC の時刻・pid は本層が std で取り、名前は §7.5 の「退避ディレクトリの名前の形」(同名があれば `-<n>` を足す) に従う。`state_dir` を使えなければ §7.5 の代替の退避先の `rescue\` に作る (`alternate = true`)。manifest の `created_at` に使う RFC 3339 の文字列は本層が作る。本文と manifest も本層が緊急退避と同じ関数で書き (`reason = "user_save"`)、書いたディレクトリを返す) |
| `list_recovery_dirs()` (v0.1) | §7.5 の「`recovery/` の走査と削除の規則」の対象 (`recovery/` 直下と `rescue/` 直下の、§7.5 の「退避ディレクトリの名前の形」(`<UTC timestamp>-<pid>[-<n>]`) に合うディレクトリ、代替の退避先の同じ位置) だけを返す。`inflight/` と形に合わない名前は見ない。中のファイルは読まない (起動時の `RecoveryScan` ジョブが呼ぶ。02 §9.3) |
| `read_dir()` (v0.4) | ファインダ・パス補完。v0.1 では使わない |
| `FileStamp` | §5.5 (計算規則。`hash` は 16 MiB 以下を読込ジョブ内でストリーミング計算。`hash` を比べるのは書き始める前の照合 (§4.1 手順 0b) だけで、置換の直前 (手順 4b) は `(mtime, len, FileId)` だけを比べる) |
| `SaveReport.method` | `SaveMethod::{Atomic, InPlace, NewFile}`。`Atomic` = §4.2 / §4.3 の置換、`InPlace` = §4.4 の直接上書き、`NewFile` = §4.1 の「新規ファイル」 |
| `SaveOptions.fsync` | §4.1 手順 3 (`Full` / `FileOnly` / `None`、OS 別の fsync API) |
| `SaveOptions.preserve` | §5.1 (OS 別に保持できる属性の一覧) |
| `SaveOptions.hardlink` | §4.4 (`HardlinkPolicy::{Ask, BreakLink, WriteInPlace}`、OQ-73) |
| `SaveOptions.private` | 新しく作るファイル (一時ファイル・最終ファイル) を「所有者のみ」(§5.6) で作る。意味はこれだけで、保存中フラグや照合の有無には関わらない |
| `SaveOptions.state_file` | `true` のとき保存中フラグ (§4.1 手順 0d・7・8) を書かない。evoeditor 自身の状態ファイル (スワップ・manifest 等。§4.1 の「状態ファイルの書込」) の書込だけが立てる。照合を省くかは `expected` だけで決まる (`Expect::Any` のときだけ省く) |

**`FsErrorKind` → OS エラーの写像** (02 §4.5 の列挙に対し、本書は OS 原因と文言を与える。細分は `FsError.detail` に添え、元の OS エラーは `FsError.source` に保持する。v0.1 で実装するのは Windows のエラーコード (`ERROR_*`) の写像で、Unix の errno (`E*`) の写像は v0.2):

| `FsErrorKind` (02) | 主な OS 原因 | 回復可能 | ユーザー向けヒント (MessageKey 案) |
|---|---|---|---|
| `NotFound` | ENOENT / ERROR_FILE_NOT_FOUND。保存先の親フォルダが無い (ERROR_PATH_NOT_FOUND。detail = `ParentMissing`。フォルダは作らない) | 可 | 開くときは新規ファイルとして扱う (FR-101)。保存で `ParentMissing` なら `msg.save.parent_missing` (04 §15.2。01 FR-101 AC-2) |
| `PermissionDenied` | EACCES, EPERM / ERROR_ACCESS_DENIED。不変フラグ (`FS_IMMUTABLE_FL` / `UF_IMMUTABLE`) もここ | 可 | 「読み取り専用で開きます」「別名で保存してください」 |
| `InvalidPath` | EISDIR / FILE_ATTRIBUTE_DIRECTORY (ディレクトリ)、デバイス・FIFO (`FileKind::Other`、detail = `NotAFile`)、ENAMETOOLONG、Windows の予約名 (detail = `ReservedName`)・禁止文字 (一覧は §3.1 の「禁止文字・予約名」の行)、リンクの循環 (ELOOP) | 可 | 「ディレクトリは開けません」「このファイル名は使えません: 理由」 |
| `ReadOnlyFs` | EROFS / ERROR_WRITE_PROTECT | 可 | 「別の場所に保存してください」 |
| `Busy` | ETXTBSY / **ERROR_SHARING_VIOLATION**, ERROR_LOCK_VIOLATION (§4.2 の間隔で再試行した後。ただし `ReplaceFileW` の共有違反・`ERROR_UNABLE_TO_REMOVE_REPLACED` は、再試行が尽きて T が無傷なら `Busy` ではなく `AtomicUnsupported`、§4.2) | 可 (再試行) | 「他のプログラムが使用中です。閉じてから再試行」 (§4.2 の再試行後に表示) |
| `DiskFull` | ENOSPC, EDQUOT / ERROR_DISK_FULL | 可 | 「ディスク容量不足。元ファイルは無傷です」 |
| `AtomicUnsupported` | 同一ディレクトリに作成不可、他ユーザー所有、nlink>1、`ReplaceFileW` の失敗かつ T 無傷 (`ERROR_UNABLE_TO_MOVE_REPLACEMENT`、`ERROR_ACCESS_DENIED`、ACL を引き継げない等。§4.2 / §4.4)、一時ファイル規則 (detail = `InPlacePattern`。確認なし) | 可 | 「安全な置換ができません。直接上書きしますか」 (ADR-0016。`file.atomic_save_fallback`、§4.4) |
| `Conflict { current: Option<FileStamp> }` | `SaveOptions.expected` と実ファイルの不一致 (外部変更・別ファイルへの置換)。`Expect::Stamp` を期待していて T が無い (削除された) ときは `NotFound` ではなく `current = None`、`Expect::Absent` を期待していて T が在るときは `current = Some(T の Stamp)` (§4.1 手順 0b) | 可 | FR-117 の選択 UI (03 §9.5)。何も書かれていない |
| `ReplacedMovedToBackup { backup }` | Windows `ERROR_UNABLE_TO_MOVE_REPLACEMENT_2` (元ファイルが退避名に移動済み) | 要注意 | 「元ファイルはバックアップ名に退避されています: <path>」 (§4.2 / §4.6)。復旧手順を必ず提示 |
| `SourceFailed` | OS 原因ではない (`ReadChunks` の供給側、= core のエンコーダの失敗。`ChunkError`)。一時ファイルを削除してから返す | 可 | 原因は core が `ChunkError` から示す (`Unmappable` は 03 §8.6 E3 の選択)。元ファイルは無傷 |
| `BackupFailed` | `write_in_place` の事前バックアップ (§4.4 手順 4) の失敗 (ERROR_DISK_FULL / ENOSPC、`state_dir` を使えない、権限等。元の OS エラーは `source`)、または in-place の保存中フラグ (§4.4 手順 2) を書けない | 可 | 「バックアップを作れませんでした。バックアップ無しで上書きしますか」(既定はキャンセル。03 §9.3)。何も書いていない |
| `InPlaceFailed { backup: Option<FsPath> }` | `write_in_place` の上書き途中の I/O エラー (§4.4 手順 6 以降)。`backup` は完全な事前バックアップのパスで、利用者が「バックアップなしで上書き」を承認した場合だけ `None` | 要注意 | 「上書きの途中で失敗しました。元の内容のバックアップ: <path>」。バックアップの場所を必ず示す (`None` のときは「バックアップはありません」と示す) |
| `Other` (detail = `Network`) | ENETDOWN, ESTALE / ERROR_NETNAME_DELETED, ERROR_BAD_NETPATH | 可 | 「ネットワーク上のファイルに到達できません」 |
| `Other` (detail = `CrossDevice`) | EXDEV | 可 (内部で回避) | — (一時ファイルは常に同一ディレクトリに作るため通常発生しない) |
| `Other` | その他 (`io::Error` を保持) | 不明 | OS メッセージをそのまま + ログ参照 |

**`ReplaceFileW` の失敗の写像** (§4.2 の「失敗モード」と同じ): `ERROR_UNABLE_TO_REMOVE_REPLACED` (T 無傷) と共有違反は §4.2 の間隔で再試行し、尽きて T が無傷なら `AtomicUnsupported` (確認付きの直接上書きへ。§4.2)。`ERROR_UNABLE_TO_MOVE_REPLACEMENT` (T 無傷) は `AtomicUnsupported`。`ERROR_UNABLE_TO_MOVE_REPLACEMENT_2` は `ReplacedMovedToBackup`。テスト用の置換失敗の注入 (09 が定める `replace_fail` フック) は、既定のエラーを `ERROR_UNABLE_TO_MOVE_REPLACEMENT` として本層のこの写像を通す前提とする (既定の注入は `AtomicUnsupported` の経路になる)。

キャンセルは `FsErrorKind` に持たない (02 §4.5)。ユーザーのキャンセルは `CancelToken` (02 §4.4) と `JobError::Cancelled` で表し、`EINTR` は本層の中で再試行して外に出さない。

### 2.2 `Paths`

trait の形は [02 §4.4](02-architecture.md#44-ports-core-が定義し外側が実装する-trait) の `trait Paths` (`config_dir` / `state_dir` / `data_dir` / `runtime_dir` と、§3.2 の手順 1〜5 の正規化 `normalize`。起動時に解決済みの値を使うだけで I/O しない) が正本で、本書は再定義しない。本書が定めるのは各値の OS 別の解決規則 (§7.1 / §7.2) と、`state_dir` の下の配置 (§7.5: `log/`・`swap/`・`recovery/`・`recovery/inflight/`・`recovery/rescue/`・`backup/`・`sessions/`) である。配下のパスは `state_dir` から導くので Port の項目にしない。

次の値は Port の項目ではなく eve-platform の内部 (`eve-platform::path` と保存の実装) が起動時に 1 回解決して持つ (本書ではまとめて `PlatformEnv` と呼び、`PlatformEnv.home_dir` 等と書く。02 §4.4 の `trait Paths` のメソッドではない): ホームディレクトリ (`home_dir`。チルダ展開、§3.6)、正準化済みの一時ディレクトリ集合 (`temp_dirs`。§4.4 の一時ファイル規則)、起動時の作業ディレクトリ (`cwd`。以後 `chdir` しない)、実行ファイルのディレクトリとポータブルモードの判定 (§7.4)、`cache_dir` (v0.4+)、代替の退避先 (§7.5。Windows は SID を含む)。core がこれらを要る処理は Port のメソッドとして公開する: §3.2 の正規化は `Paths::normalize` (I/O をしない純粋な計算なのでメインスレッドから呼べる)、退避ディレクトリの作成と走査は `FileSystem::create_recovery_dir` / `list_recovery_dirs` (§2.1)。

### 2.3 `Clock`

[02 §4.4](02-architecture.md#44-ports-core-が定義し外側が実装する-trait) どおり。単調時計の `now() -> Tick` (ミリ秒) の 1 本だけで、壁時計は持たない (壁時計の扱いは 02 §4.4 の「時刻の源」)。本番実装は eve-host が起動時の `std::time::Instant` からの経過ミリ秒で与える (core は `Instant::now()` を直接呼ばない。決定性のため)。Windows の `Instant` は `QueryPerformanceCounter` で高分解能だが、**スリープ/タイマの分解能は既定 15.6 ms** であり、02 §9.2 の `recv_timeout` deadline 精度に影響する (キーシーケンスのタイムアウト等は 100 ms 単位で設計すれば問題ない)。15.6 ms より短い待ちは区別できないので、04 §6.6 のバースト検出の間隔 T は **16 ms** とし (分解能の 1 刻み (15.6 ms) より長くとり、タイマの丸めでバーストを途中で閉じないため。`WaitForSingleObject(hIn, T)` の実効の待ちは 16〜31 ms になる)、それより短い値を定めない (`timeBeginPeriod` で分解能を上げることはしない。プロセス全体・電力に影響するため)。

### 2.4 `JobSpawner`

trait の形は [02 §4.4](02-architecture.md#44-ports-core-が定義し外側が実装する-trait) (`spawn(doc, base, kind) -> JobId` / `cancel(id)`。`JobId` は `JobSpawner` が採番し、メインスレッドからだけ呼ぶ) で、本書は再定義しない。キャンセルの旗は eve-core の型 `CancelToken` (02 §4.4。`JobSpawner` がジョブごとに作る) で、ジョブがチャンクの区切りで見る。[02 §9.1 / §9.3](02-architecture.md#93-ジョブとキャンセル) のとおり **ワーカーは固定 2 本** (ADR-0006): `io` (読込・保存・metadata・外部変更チェック・起動時の退避の走査 (`RecoveryScan`) を直列に実行。起動時に生成) と `bg` (検索、Undo の実体化、E3 の置換の ChangeSet の構築、クリップボードの文字列化、スワップ書出し (v0.2)。最初の bg の作業で遅延生成し、新しい要求が来たら同種の古い要求をキャンセルする。例外 (Undo の実体化はキャンセルしない、書込中のスワップはキャンセルせず完了後に最新で 1 回だけ再実行) を含め、正本は 02 §9.1 の表)。ワーカー数は固定で、優先度・予約ワーカー・スレッドプールは持たない (保存が検索に待たされないことは `io` / `bg` の分離で満たす。ワーカー構成は OQ-103 として v0.4 で再評価)。本書が追加する OS 固有の規定は無い (スレッドは `std::thread` で、スタックサイズは既定)。入力スレッド・ログスレッド (§7.3) と合わせ、v0.1 のプロセスはメイン + 入力 + `io` の 3 スレッドで起動し、`bg`・ログ・クリップボードの所有者ウィンドウ (§8.2。Win32 のメッセージループ)・クリップボード取得用の使い捨てスレッド (§8.1) は必要時に増える (一覧の正本は 02 §9.1)。

### 2.5 `FileWatcher` (v0.4)

```rust
// 案。v0.4 着手時に 02 §4.4 で確定
trait FileWatcher {
    fn watch(&mut self, path: &Path, kind: WatchKind /* File | DirShallow */) -> Result<WatchId, WatchError>;
    fn unwatch(&mut self, id: WatchId);
    fn capabilities(&self) -> WatchCaps;   // native_events: bool, polling_interval: Option<Duration>
}
// イベントは HostMsg::Fs(FsEvent { watch: WatchId, path: PathBuf, kind: Created|Modified|Removed|Renamed{from,to}|Rescan })
```

詳細は §12。

### 2.6 `ProcessRunner` (v0.4)

```rust
// 案。v0.4 着手時に 02 §4.4 で確定
struct ProcessSpec {
    argv: Vec<OsString>,                  // argv[0] = 実行ファイル。シェル経由は呼び出し側が明示的に組む (§13.2)
    cwd: Option<PathBuf>,
    env: Vec<(OsString, Option<OsString>)>, // None = 削除
    stdin: StdinSource,                   // None / Bytes(Snapshot 範囲)
    timeout: Option<Duration>,
    kill_tree_on_cancel: bool,            // 既定 true (§13.4)
}
trait ProcessRunner {
    /// 常にユーザー承認後にのみ呼ばれる (02 §4.4)。結果は HostMsg::Job で返る。
    fn spawn(&self, spec: ProcessSpec, cancel: CancelToken) -> Result<JobId, ProcessError>;
}   // CancelToken は eve-core の型 (02 §4.4)。本書は再定義しない
// 出力は raw bytes + 推定文字コードのヒント (§13.5)。デコードは 03 の責務。
```

**長寿命プロセス (`spawn_long_lived`、v1.L)**: trait と `LongLivedSpec` / `ProcessHandle` の正本は [02 §4.4](02-architecture.md#44-ports-core-が定義し外側が実装する-trait) の「長寿命プロセスの API」で、本書は再定義しない。LSP サーバ、ローカル LLM サーバの管理モード (§13.7)、外部エージェント (v1.X) が使う。OS 別の実装は次のとおり。

| 02 §4.4 の項目 | OS 別の実装 |
|---|---|
| `LongLivedSpec.priority` | `BelowNormal` は Unix が nice 10、Windows が `BELOW_NORMAL_PRIORITY_CLASS` |
| `LongLivedSpec.kill_tree_on_exit` | 起動時に §13.4 の機構 (Windows は Job Object の `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`、Unix はプロセスグループ) を適用し、エディタの終了・異常終了で子孫を残さない |
| `LongLivedSpec.log_ring_bytes` | `LogOnly` のとき、標準出力・標準エラーを上限付きのリングに読むだけで、ファイルには書かない (MR8。管理モードの値は §13.7) |
| `ProcessHandle::terminate(grace)` | Unix は SIGTERM → 猶予 → SIGKILL (プロセスグループごと)、Windows は終了要求 → 猶予 → Job Object の終了 (§13.4) |
| 終了・クラッシュの通知 | 監視スレッドが `HostMsg::Service(ProcessExited { id, status })` を送る |

### 2.7 `SecretStore` (v1.L)

```rust
// 案。v1.L 着手時に 02 §4.4 で確定
trait SecretStore {
    fn get(&self, service: &str, account: &str) -> Result<Option<SecretString>, SecretError>;
    fn set(&self, service: &str, account: &str, value: &SecretString) -> Result<(), SecretError>;
    fn delete(&self, service: &str, account: &str) -> Result<(), SecretError>;
    fn backend(&self) -> SecretBackend; // WindowsCredentialManager / MacKeychain / SecretService / EnvOnly / None
}
```

詳細は §14。

### 2.8 クリップボードは Port ではなく Effect 経由 (ADR-0021)

[02 §4.4](02-architecture.md#44-ports-core-が定義し外側が実装する-trait) / [02 §6.3](02-architecture.md#63-コマンドtransactioneffect-の関係) のとおり、クリップボードは core の Port ではなく `Effect::SetClipboard` / `RequestClipboard` + `InputEvent::ClipboardContent` の **Effect 経路** で扱う (ADR-0021)。`eve-platform` の `ClipboardProvider` は **Host (eve-host) が Effect を解釈するときに使う実装部品** であり core には注入しない。理由: (1) OSC 52 は端末出力ストリームに書く必要があり eve-tui 側でしか実行できない、(2) X11/Wayland の取得は数十〜数百 ms ブロックし得るため同期 Port は UI を止める (P3 原則違反)。`Effect::SetClipboard` が運ぶのは `ClipboardText` (02 §6.3) で、小さい選択は `Inline(String)`、大きい選択は `Handle(ClipboardHandleId)` である。`Handle` のとき Host は `Editor::take_clipboard(id)` で `ClipboardSource` (Snapshot と範囲。`Send`) を取り出し、**文字列化は Host のワーカーで行う** (02 §5.4 / §6.3 / §14.1 MR5。メインスレッドで大きな `String` を作らない)。詳細は §8。

---

## 3. パス

### 3.1 OS 差異の一覧

v0.1 で実装するのは **Windows の列** (区切り文字の両受理、ドライブ相対・ルート相対の解決、長いパス (`\\?\`・`longPathAware`)、UNC、予約名・禁止文字の検査、`%USERPROFILE%`) である。macOS / Linux の列は v0.2 (macOS / Linux 出荷時) の仕様。

| 項目 | Windows | macOS | Linux | evoeditor の方針 |
|---|---|---|---|---|
| 区切り文字 | `\` と `/` の両方 | `/` | `/` | 入力は両方受理 (Windows)。内部は `PathBuf` のまま。表示は OS ネイティブ区切り。設定ファイル内は `/` 推奨 (`\` はエスケープと衝突しやすい。書式は 07) |
| 大文字小文字 | 既定 **非区別** (NTFS/ReFS)。ディレクトリ単位で区別化可 (WSL 由来) | APFS 既定 **非区別** (区別ボリュームも作成可) | **区別** (ext4 等。casefold ディレクトリあり) | **自前で大文字小文字を畳み込まない**。「同じファイルか」は §5.4 の `FileId` で判定し、表示名は canonicalize 後の実際の綴りを使う (例外は保存中フラグのキー `hash(path)` の入力で、Windows では畳み込む。§7.5) |
| 絶対パスの形 | `C:\…`, `\\server\share\…` (UNC), `\\?\C:\…` (verbatim), `\\.\device`, ドライブ相対 `C:foo` | `/…` | `/…` | ドライブ相対 (`C:foo`) は `cwd` ではなくそのドライブのカレントで解決される罠があるため、**起動時に絶対化** し以後は絶対パスのみ扱う。`\\.\device`・`\\?\GLOBALROOT\…`・代替データストリーム (`a.txt:s`) は文書のパスとして受け付けない (§3.2 手順 5) |
| 長さ上限 | 260 (MAX_PATH)。`\\?\` 接頭辞または manifest `longPathAware` + レジストリで 32767 | 1024 (PATH_MAX) | 4096 (PATH_MAX)、要素 255 | Rust std は長いパスを自動で `\\?\` 化する (`maybe_verbatim`) ため **読み書きは動く**。問題は「表示」「外部プロセスへの引き渡し」「設定への書き出し」→ §3.3 で非 verbatim 化する。exe manifest に `longPathAware=true` も埋め込む (§16.4) |
| 禁止文字・予約名 | `<>:"\|?*`、制御文字、末尾のドット/空白、`CON` `PRN` `AUX` `NUL` `COM0-9` `LPT0-9`、上付き数字の `COM¹` `COM²` `COM³` `LPT¹` `LPT²` `LPT³`、`CONIN$` `CONOUT$` (大文字小文字を区別せず、拡張子付きでも) | `:` は Finder では `/` と表示 | NUL のみ | Windows では **保存前に検証** し `InvalidPath` (予約名は detail = `ReservedName`) を返す (`\\?\` で作成可能でも Explorer/他ツールが扱えないため)。Unix では検証しない |
| 文字コード | UTF-16 (不正なサロゲート対を含み得る) | UTF-8 (正規化は APFS では保持、HFS+ は NFD) | 任意バイト列 | `OsString`/`PathBuf` を貫通させる。Windows の不正 UTF-16 は std が WTF-8 で保持する。ファイル (セッション・スワップヘッダ) への永続化は §3.5 |
| シンボリックリンク | 作成に特権または開発者モードが必要。ジャンクション・再解析ポイントあり | 可 | 可 | 開く・保存はリンク先を対象 (FR-106)。ジャンクションは「ディレクトリへのシンボリックリンク」として扱う |
| カレントディレクトリ | プロセス単位 + ドライブ別の隠しカレント | プロセス単位 | プロセス単位 | 起動後に **`chdir` しない**。workspace ルートは `PlatformEnv.cwd` (§2.2) から導出 |
| ホーム | `%USERPROFILE%` (`HOME` は通常未設定) | `$HOME` | `$HOME` | §3.6 |

### 3.2 正規化の規則 (`eve-platform::path`)

入力 (CLI 引数・設定値・ダイアログ入力・LSP/AI からの URI) を文書パスにする際、次を順に適用する。手順 1〜5 は I/O をしない計算で、core には `Paths::normalize` (02 §4.4) として公開し、core はメインスレッドで呼ぶ (03 §9.2 手順 0)。手順 6 はジョブの中で行う:

1. **チルダ展開** (`~` または `~/`、`~\` のみ。`~user` 形式は非対応) → `PlatformEnv.home_dir` (§2.2)。ホームが不明なら展開せずエラー。
2. **環境変数展開はしない** (CLI ではシェルが行う。設定ファイル内の展開書式は 07 が決める。実装は本層の `expand_env` を提供)。
3. **相対 → 絶対**: `PlatformEnv.cwd` と結合。Windows のドライブ相対・ルート相対 (`\foo`) はここで解決。
4. **構文的正規化**: `.` 除去、`..` の畳み込みは **行わない** (シンボリックリンクを跨ぐ `..` は意味が変わる)。`..` は `canonicalize` で解決する。
5. **Windows 固有**: `/` → `\`、連続区切りの圧縮 (UNC 先頭の `\\` は保持)、末尾区切り除去 (ルートは除く)、予約名検査 (一覧は §3.1)。**デバイス名前空間とストリームの拒否**: `\\.\` で始まるパス (`\\.\pipe\…` の名前付きパイプ、`\\.\PhysicalDrive0` 等のデバイス)、`\\?\GLOBALROOT\…`、`\\?\` の後がドライブ文字 (`\\?\C:\`) と `UNC\` 以外のもの (`\\?\Volume{…}` を除く)、ファイル名の要素に `:` を含むもの (代替データストリーム `a.txt:s`、`a.txt::$DATA`。ドライブ指定の `C:` は除く) は、`InvalidPath` (detail = `NotAFile`) として **I/O の前に** 拒否する (`metadata` / `open` を呼ぶだけで名前付きパイプに接続する・デバイスを開く副作用を避ける。Rust std の `metadata` / `File::open` が名前付きパイプのパスで実際に接続を試みるかは未確認で、S-09 で確かめる。確かめるまでは接続し得るものとして扱う)。`\\?\` の長いパス自体は拒否しない (§3.1)。
6. **canonicalize は「開くとき」のみ** (存在しないパス = 新規ファイルは親ディレクトリまで canonicalize し、ファイル名を付け直す)。

### 3.3 canonicalize と表示

- `std::fs::canonicalize` は Windows で `\\?\C:\...` 形式を返す。これをそのまま表示・保存・外部プロセスに渡さない。**`dunce::simplified` 相当** (verbatim 接頭辞を外せる場合だけ外す。260 文字超、予約名、`..` 要素を含む等の場合は残す) を適用した「表示用パス」を `Document` に併せて保持する。候補クレート: `dunce` (MIT/CC0。約 200 行。自前実装でも可)。
- (v0.2、macOS 出荷時) macOS で `/tmp` → `/private/tmp`、`/var` → `/private/var` に解決される。表示はユーザーが指定した形 (解決前) を優先し、同一性判定だけ canonical を使う。
- (v0.2、Linux 出荷時) Linux の `/proc/self/fd/N`、bind mount、overlayfs (コンテナ) でも canonicalize は機能する。NFS の stale handle では `ESTALE` → `Network` エラー。
- `canonicalize` は **I/O を伴う** (Windows はファイルを開く)。UI スレッドではなく読込ジョブ内で行う。

### 3.4 ファイル URI (v1.L LSP 用)

`file:///C:/path/to` (Windows はドライブの前に `/`、`\` は `/`、パーセントエンコード UTF-8) ⇔ `PathBuf` の変換を本層で提供する。UNC は `file://server/share/path`。非 UTF-8 パスは URI 化できないため LSP には渡せない (警告ログ)。変換規則の詳細は 06/LSP 側ではなく本層が所有する (OS 依存だから)。

### 3.5 非 UTF-8 パスの永続化と表示

| 用途 | 方式 |
|---|---|
| 画面表示・ログ | `Path::display()` (Unix: 不正バイトは U+FFFD、Windows: 不正サロゲートは U+FFFD)。ログには原則パス全体を出さない (NFR-504。ファイル名のみ or ハッシュ)。パニックの記録 (パニックフックのログ・クラッシュログ・stderr の `panic.*`) に書くのは **場所・スレッド名・payload の型のみ** で、payload の文字列は型が `&'static str` のときだけ書く (`String` の payload は書かない。02 §11.2) |
| スワップ (v0.2)・セッションファイル (v0.4) | Unix: バイト列をそのまま (長さ前置)。Windows: WTF-8 (`OsStrExt::encode_wide` の u16 列を永続化)。読み戻し時は `OsString::from_vec` / `from_wide`。**OS 間で持ち運ばない** 前提 (スワップはホスト固有) |
| 設定ファイル (TOML は UTF-8 必須) | 非 UTF-8 パスは記述不可。該当する場合は `FR-703` の規則で警告し無視 |
| AI への送信 (v1.L) | 表示用文字列。パス自体を除外パターン判定の前に正規化 (06) |

### 3.6 ホームディレクトリの決定

| OS | 順序 |
|---|---|
| Unix (v0.2) | `$HOME` → `getpwuid_r(geteuid())` (sudo で `HOME` が root になる問題は仕様どおり: `sudo evoeditor` は root の設定を読む。`sudo -E`/`--config` で回避可) |
| Windows (v0.1) | `%USERPROFILE%` → `SHGetKnownFolderPath(FOLDERID_Profile)`。`%HOME%` は **見ない** (Git Bash/MSYS2 が設定していることがあり、設定の所在が環境で変わるのを避ける。ただし `XDG_CONFIG_HOME` が明示されていれば §7 の規則で尊重する) |

---

## 4. ファイル I/O と原子的保存

保存手順の所在: 共通手順は §4.1、OS 別の手順は §4.2 (Windows) / §4.3 (Unix)、直接上書き (in-place) の条件と手順は §4.4。

### 4.1 共通手順 (ADR-0016 の具体化)

保存ジョブ (ワーカー、Snapshot 入力) は次の順で行う。各ステップの失敗は **元ファイルを無傷に保ったまま** `FsError` を返す。

v0.1 で実装するのは Windows の手順 (`ReplaceFileW` 系。§4.2) で、下の手順中の Unix / macOS / Linux の項目 (mode 0600、`F_FULLFSYNC`、`fchown` / `fchmod`、ディレクトリ fsync、`renameat2` / `renamex_np`) は v0.2 (macOS / Linux 出荷時) に実装する。

**NFR-301 の主張範囲 (OS 別)**: Unix (v0.2) の `rename` は原子的で、どの時点で電源断が起きても T は旧内容か新内容の完全な状態で残る。Windows の `ReplaceFileW` は **原子性を保証しない** (内部で「T をバックアップ名へ移動 → tmp を T へ移動」を順に行い、その間にクラッシュ・電源断が起きると **T が存在せず `.<name>.eve-bak-*` だけが残る窓** がある)。したがって Windows での主張は「途中失敗時に元ファイルが無傷で残るか、または退避名で完全な状態で残り、そのパスを開くときに報告する (v0.1)。起動時の復旧の提案は v0.2 (回復可能性)」である (§4.6、01 NFR-301)。

```
0. 事前検査 (すべてジョブ内)
   a. path を解決 (canonicalize。シンボリックリンクなら最終リンク先 T を対象。FR-106)。T が存在し、§4.4 の一時ファイル規則に該当すれば
      (T が存在しなければ規則を見ずに下記「新規ファイル」の経路へ)
      何も書かずに AtomicUnsupported (detail = InPlacePattern) を返す (core が確認なしで乾式走査 → write_in_place に進む。02 §4.5 / 03 §9.3)。
      リンク先が存在しない (ダングリング。FileMeta.link が dangling) 場合は下記「ダングリングシンボリックリンク」の規則に従う
   b. 書き始める前の照合: meta = metadata(T) (file_id を含む)。SaveOptions.expected (FileStamp) と §5.5 の規則で比較する
      (mtime と len が等しいときは内容ハッシュ (16 MiB 以下) も T を読んで比べる。ハッシュを比べるのはこの 0b だけ)。
      不一致なら Conflict { current: Some(T の Stamp) } を返し core が FR-117 の確認 UI を出す。何も書かない。
      Expect::Stamp を期待していて T が無い (開いた後に削除された) ときは NotFound ではなく Conflict { current: None } を返す
      (core は確認の後に Expect::Absent で再投入できる)。Expect::Absent を期待していて T が在るときは Conflict { current: Some(T の Stamp) }。
      T も親フォルダも無いときは Expect に依らず NotFound (detail = ParentMissing) を返す (下記「新規ファイル」)
   c. 原子的置換の可否判定 (§4.4)。不可なら AtomicUnsupported を返す (core が file.atomic_save_fallback に従い、承認後に乾式走査 → write_in_place を呼ぶ)
   d. 保存中フラグ (§4.6 / §7.5) を書く (一時ファイルを作る前)。書けなければ SaveWarning に積んで保存は続ける
      (原子的置換では T が無傷のまま残るため。in-place では続けない、§4.4 手順 2)。
      SaveOptions.state_file = true の書込ではこの手順 (と手順 7・8 のフラグの削除) を行わない (下記「状態ファイルの書込」)。
      手順 0b・4b の照合を省くのは SaveOptions.expected = Expect::Any のときだけ (state_file・private とは独立)
1. 一時ファイル作成: T と同じディレクトリに ".<name>.eve-tmp-<pid>-<rand8>" を O_EXCL 相当で作成 (名前の長さの規則は下記)
   - Unix (v0.2): mode 0600 で作成 (本文が一瞬でも他ユーザーに読める時間を作らない。NFR-505 の精神)
   - Windows: CreateFileW の lpSecurityAttributes に **「所有者のみ」(§5.6 の定義) の DACL** を渡して作成する。
     ディレクトリの ACL を継承させると、共有フォルダで T だけを厳しい ACL にしている場合に書込中〜置換までの間に他者が読める。
     置換後は ReplaceFileW が T の ACL を適用するので最終状態は変わらない (フラグ 0 で呼ぶので、ACL を引き継げなければ置換自体が失敗する。§4.2)。置換に ReplaceFileW 以外の手段 (MoveFileExW 等) は使わない
     (所有者のみの DACL のまま T になって ACL を黙って狭める経路を作らない。§4.2)。
     新規ファイル (T が存在しない) の tmp は、SaveOptions.private = false なら親ディレクトリの継承 ACL で作る
     (守るべき T の ACL が無く、移動後のファイルが通常の新規ファイルと同じ ACL を持つようにするため)。private = true なら所有者のみ
2. 本文をチャンク書込 (エンコードは 03。ここはバイト列を受け取るだけ)。取消しはチャンク毎に core のエンコーダが CancelToken を見て
   ChunkError::Aborted を返す形で届き、本層は tmp を削除して SourceFailed を返す (手順 8)。取消しを見るのはここまで (置換の前) の段だけ (下記「取消し」)
3. fsync(一時ファイル)  — FsyncLevel::Full / FileOnly のとき
   - macOS (v0.2): eve-platform が自分で fcntl(F_FULLFSYNC) を呼び、失敗 (ENOTSUP / EINVAL 等。SMB・外部ドライブ) したら fsync() にフォールバックして警告する。
     std の File::sync_all は Apple ターゲットで F_FULLFSYNC を呼ぶだけでフォールバックしないので、sync_all は使わない
   - Windows: FlushFileBuffers (std の sync_all)
   - ネットワーク FS: 失敗 (ENOTSUP/EINVAL) は警告として SaveReport に積み、保存は続行
3b. **一時ファイルのハンドルを閉じる** (Windows では開いたまま ReplaceFileW を呼ぶと共有違反になる。Unix でも fd を閉じてから rename する)
4. 権限・属性の複製 (§5.1)  — 失敗は警告 (所有者複製失敗は 0c で事前に検出済み)。
   v0.1 (Windows) は READONLY・HIDDEN・SYSTEM 属性のみ (所有者・ACL・ADS は ReplaceFileW が保つ分)。Unix の mode / 所有者は v0.2 (macOS / Linux 出荷時)。
   Unix の xattr・ACL と macOS の UF_HIDDEN の明示的な複製は v0.2 (FR-106)
   - Unix (v0.2): fchown → fchmod の順 (所有グループを合わせる前に mode を広げない)。fchown が失敗した (所属していないグループ等) 場合は、
     元の mode を **別のグループに適用しない** (意図しない相手に読まれる)。group / other ビットを落として所有者ビットのみ適用し、
     SaveWarning::PermissionNarrowed で知らせる (または §4.4 の確認フローへ)
   - Windows: HIDDEN / SYSTEM は置換の前に tmp へ付ける。READONLY は付けない (置換の失敗時に tmp を消せなくなるため。手順 5b)
4b. 置換の直前の照合: metadata(T) を取り直し、手順 0b の値と **(mtime, len, FileId) だけ** で比べる (内容ハッシュは計算しない。
    ここで T を読み直すと置換までの窓が広がり、大きなファイルでは保存が遅くなるため)。不一致なら tmp を削除して Conflict を返す
    (current は手順 0b と同じ規則: T が在ればその Stamp、Expect::Stamp を期待していて T が消えていれば None)
    (書込中の数秒間に他のプログラムが保存した場合を黙って上書きしない。残る競合の窓は置換の 1 回分だけ)
5. 原子的置換 (OS 別 §4.2 / §4.3)。置換の直前から直後まで「置換中」の計数を上げる (下記「置換中の計数」)。
   ReplaceFileW (新規ファイルは MoveFileExW、Unix は rename) の呼出しを始めた後は CancelToken を見ず、手順 7 まで進めて結果を返す (取り消されていても Saved になる)
   (§11.5 の通常の終了はこれが 0 に戻るまで exit しない。§11.4 の非対話の終了はその経路の時間予算の範囲で待つ。write_in_place では書込の全体を覆う、§4.4 手順 6)
5b. Windows: 元の T が READONLY だった場合は、置換が成功した後で T に READONLY を付け直す
6. ディレクトリ fsync (Unix のみ、v0.2。Windows は不可・不要)
7. 事後: metadata(T) を取り直し SaveReport.new_stamp (FileStamp。hash は書いた内容から計算済み) に入れる。FileId が変わること (Unix では必ず変わる) を core に伝え、監視 (§12) の自己イベント抑制に使う。
   **置換の後の T が自分の書いたものかを FileId で確かめる**: 手順 3b でハンドルを閉じる前に tmp の FileId (と len) を控えておき、事後の metadata(T) の FileId が
   それと一致しなければ (置換から事後の metadata までの間に他のプロセスが T を置き換えた。ReplaceFileW では置換後の T は tmp の FileId を持つ (§5.4、S-09 で確認)。
   新規ファイルの MoveFileExW も同じ)、事後の値を new_stamp に入れない。new_stamp には自分の書いた内容の値 (tmp の FileId・len・hash、mtime は控えた値) を入れ、
   SaveWarning (保存の直後に他のプログラムが変更した可能性) を積む。core の disk_stamp がディスクの T と一致しないので、次の保存は照合 (手順 0b) で Conflict になり、
   外部変更として利用者に確認される (黙って上書きしない)。直接上書き (§4.4) では FileId は変わらないので、手順 1 の FileId と一致することを同じ規則で確かめる
   Windows で backup 引数に渡した .<name>.eve-bak-<pid>-<rand8> を削除する。削除できたら保存中フラグを削除する。
   **退避名を削除できなかったら (他のプロセスが開いている等) フラグを消さずに残し** (T は新内容で完全なので、開くときの写像では報告しない行になる。§4.6)、
   SaveWarning (退避名を消せなかったこととそのパス) を積む。フラグを残すのは、v0.2 の起動時の走査が残った退避名を見つける手掛かりにするため (§4.6)。
   同じキーのディレクトリに残る自分の今回の記録以外の記録 (他の pid と同じ pid の前の保存の記録。前の中断の痕跡) も消す (一時ファイル・退避名・バックアップは消さない。
   ただし記録された一時ファイル・退避名が実在する記録は消さずに残す (§4.6 の「フラグを消す条件」)。02 §4.5 の「報告した記録を消す時機」。
   in-place の成功後の後片付け (§4.4) も同じ)。v0.1 は pid の生存確認をしないので、同じ T を同時に保存中の他インスタンスの記録も消え、
   その保存が中断したときの痕跡を失うことがある (同時保存に限られるので許容する。v0.2 の pid の生存確認 (§5.3) で除く)
8. 失敗時の後片付け: 一時ファイルを削除。削除も失敗したら名前をログに記録 (本文は含まれるので「どこにあるか」だけ)。
   置換の前の失敗で一時ファイルを消せた場合は保存中フラグも削除する。一時ファイルを消せなかった場合と置換途中の失敗 (ReplacedMovedToBackup) ではフラグを残す
   (§4.6 で検出される。T が在る間は開くときに報告しないが、フラグは開くジョブにも消されない。§4.6 の「フラグを消す条件」)
```

- **置換中の計数 (`ReplacingGuard`)**: 「置換中フラグ」(本書・02 §9.6 / §11.2.1 の呼び名) の実体は単一の真偽値ではなく **計数** (eve-core の型 `ReplacingCount`。中身は `Arc<AtomicUsize>`。Host が起動時に 1 つ作り、本層の `FileSystem` の実装の構築時に渡す。待つ側 (Host のメインと受信スレッド) は同じものを持つ。02 §4.4) で、RAII のガード `ReplacingGuard` を作ると 1 増やし、ガードを落とすと (エラー・パニックの巻き戻しを含む) 1 減らす。「フラグが立っている」は計数が 0 より大きいこと、「下りる」は 0 に戻ることを指す (2 つの書込が並行しても、先に終わった側が他方の区間を隠さない)。ガードを作るのは **利用者のファイルを壊し得る区間だけ** で、(1) 利用者のファイルの保存 (`state_file = false`) の手順 5 の置換 (`ReplaceFileW` / 新規ファイルの `MoveFileExW`。v0.2 の rename を含む) の直前から直後まで、(2) `write_in_place` の手順 6〜7 の全体 (§4.4 手順 6) である。状態ファイルの書込 (`state_file = true`: スワップ・manifest・`trust.toml` 等) と保存中フラグ自身の書込はガードを作らない (途中で終わっても利用者のファイルは無傷で、終了の待ちを延ばさない)。
- **取消し (正本は 02 §4.5 の「取消し」)**: main は保存を取り消すとき常に `CancelToken` を立て、ジョブが置換の前か最中かを観測しない。取消しを無視するのはジョブと本層の側である。`write_atomic` で取消しが効くのは手順 0 の `metadata` (ジョブが呼ぶ前に見る) と手順 2 の一時ファイルへの書込 (エンコーダの `ChunkError::Aborted`) だけで、手順 5 の置換の呼出しを始めた後は見ない。`write_in_place` は §4.4 の手順のとおり。
- **一時ファイル・退避名の長さ**: `.<name>.eve-tmp-<pid>-<rand8>` / `.<name>.eve-bak-<pid>-<rand8>` の全体がファイル名の上限 (Unix は 255 バイト、Windows は 255 UTF-16 単位) を超えるときは、`<name>` を文字境界で切り詰め、末尾に `~<hash8>` (元の名前のハッシュの先頭 8 桁) を付けて上限に収める (日本語の長い名前で ENAMETOOLONG になって保存できない事故を防ぐ)。実際の名前は保存中フラグに記録するので、復旧に困らない。緊急退避の `<n>-<basename>.txt` (§7.5) とバックアップ名も同じ規則で切り詰める。

- **保存中の編集は許可** ([02 §9.3](02-architecture.md#93-ジョブとキャンセル))。Snapshot を書くため競合しない。同じ Document に対する保存ジョブは直列化する (後続は待つ)。
- **状態ファイルの書込** (スワップ・manifest 等): 呼び出し側が `SaveOptions { state_file: true, expected: Expect::Any, private: true, .. }` を渡す。`state_file = true` で保存中フラグ (手順 0d・7・8) を書かず、`Expect::Any` で照合 (手順 0b・4b) を省く (書き手は evoeditor 自身だけで、外部変更を守る対象ではない)。3 つの欄はそれぞれ独立に効く: 照合を省くかは `expected` だけ、フラグを省くかは `state_file` だけ、`private` は「新しく作るファイルを所有者のみで作る」だけを意味する (利用者のファイルを `private = true` で保存しても、フラグと照合は省かない)。保存中フラグ自身は `write_atomic` を通さず、本層の下位関数 (一時ファイル + 置換の同じ手順を、フラグと照合なしで行う内部関数) で直接書く。これで「フラグを書くためにフラグを書く」循環を断つ。手順 1 の所有者のみの作成 (§5.6) と fsync は省かない。
- **新規ファイル** (T が存在しない): 親ディレクトリが無ければ **作らずに** `NotFound` (detail = `ParentMissing`) を返す (何も書かない。01 FR-101 AC-2、04 の `msg.save.parent_missing`)。一時ファイルに書く手順は同じ (部分書込ファイルを残さないため)。`SaveMethod::NewFile`。§4.4 の一時ファイル規則と `file.save_strategy = "in_place"` は T が存在するときだけ効き、この経路には効かない (上書きする元ファイルが無いので in-place の意味が無い)。置換は **既存のファイルを上書きしない操作** で行う: Windows (v0.1) は `MoveFileExW` (`MOVEFILE_REPLACE_EXISTING` を付けない。ReplaceFileW は置換される側が無いと使えない)。v0.2 (macOS / Linux 出荷時) は Linux が `renameat2(RENAME_NOREPLACE)` (使えない FS では `link` + `unlink`)、macOS が `renamex_np(RENAME_EXCL)`。照合の後に同名のファイルが作られていたら `Conflict` を返す (02 §4.5 `Expect::Absent`)。
- **ダングリングシンボリックリンク** (開いたパスがシンボリックリンクで、リンク先が存在しない。01 FR-106): **v0.1 はリンク先を作らず、保存をエラーにする** (`FsErrorKind::NotFound`、`FsError.detail` = `DanglingLink`)。文言にリンク先の絶対パスを示し、「名前を付けて保存」を案内する。何も書かず、リンクも変えない。リンク先への新規作成は v0.2 で、仕様は次のとおり。v0.2: **確認のうえリンク先に新規作成** する。確認文言にリンク先の絶対パスを示し、選択肢は「リンク先に作成 (リンクを維持)」/「キャンセル」。リンク自体を通常ファイルで置き換えることはしない (リンクを壊さない)。承認後は、リンク先パスを T として新規ファイルの手順で書く (一時ファイルはリンク先のディレクトリに作る)。リンク先の親ディレクトリが無い場合は作らずに `NotFound` を返す (リンクの誤りの可能性が高い)。リンク先がさらにダングリングなリンクの連鎖である場合や、解決が循環する場合 (ELOOP) は `InvalidPath` を返す。確認を出すのは core (03 §9.3) で、本層は `FileMeta.link` で dangling を報告し、承認後の保存ジョブでリンク先を解決して書く。
- **`FsyncLevel::None`** は設定で明示した場合のみ (遅いネットワークドライブ用)。既定は Full (P1 原則。NFR-107 の保存時間目標は fsync 込みで定義されている)。

### 4.2 Windows の手順

**v0.1 で実装する保存手順** (ReplaceFileW 系)。

| 段階 | API | 注意 |
|---|---|---|
| 一時ファイル作成 | `CreateFileW(CREATE_NEW, FILE_ATTRIBUTE_TEMPORARY なし)` | `FILE_ATTRIBUTE_TEMPORARY` は書込遅延を招くので付けない。`FILE_SHARE_READ` のみ許可 |
| 置換 | **`ReplaceFileW(T, tmp, backup=".<name>.eve-bak-<pid>-<rand8>", dwReplaceFlags = 0)`**。`backup` 引数は **必須** で、常に T と同じディレクトリの退避名 (書式は §4.1 の 1 つだけ) を渡す。**フラグは 0 にし、`REPLACEFILE_IGNORE_MERGE_ERRORS` も `REPLACEFILE_IGNORE_ACL_ERRORS` も付けない** (Microsoft の文書では、`IGNORE_MERGE_ERRORS` は T から tmp への情報 (属性・ACL 等) のマージのエラーを無視するフラグで、`WRITE_DAC` が無いときも置換は成功するが ACL は保たれない。付けると tmp の「所有者のみ」の DACL のまま T になって ACL を黙って狭める。`IGNORE_ACL_ERRORS` も同じ)。マージ (ACL の引継ぎを含む) に失敗して `ReplaceFileW` が失敗し T が無傷なら、下の「置換に失敗したとき」と同じく `AtomicUnsupported` を返し、確認付きの in-place (§4.4) へ進む (T が無傷かの判定は下の「その他の環境」の行)。**S-09 の確認項目**: (1) フラグ 0 でマージに失敗したときのエラーコードと、T・tmp・退避名が置換前のまま (T が無傷) であること、(2) フラグ 0 で成功したときに T の DACL が置換前の T の DACL と一致し、tmp の「所有者のみ」の DACL になっていないこと (共有フォルダで T だけを厳しい・緩い ACL にした場合を含む)。(2) が成り立たない環境が見つかった場合は、置換の成功後に T の DACL を読み、tmp に付けた所有者のみの DACL のままなら `SaveWarning::AclNarrowed` で知らせる事後検査を加える (S-09 の結果で採否を決める) | Microsoft の文書が保持するとしているのは T の **作成日時・短い名前・オブジェクト ID・DACL・セキュリティリソース属性・暗号化・圧縮・名前付きストリーム (ADS)**。`std::fs::rename` ではこれらが失われる (特に ADS の `Zone.Identifier`)。**READONLY / HIDDEN / SYSTEM 属性と所有者は文書に記載が無い** ので、属性は eve-platform が §4.1 手順 4 / 5b で明示的に複製し、所有者が保たれるかは S-09 で確認する |
| 置換に失敗したとき | **`MoveFileExW(MOVEFILE_REPLACE_EXISTING)` による置換は使わない** | `ReplaceFileW` が失敗し T が無傷 (`ERROR_UNABLE_TO_MOVE_REPLACEMENT`、`ERROR_ACCESS_DENIED`、フラグ 0 での属性・ACL のマージの失敗、下の共有違反の再試行が尽きた場合 等。判定は下の「その他の環境」の行) なら、tmp を削除して `AtomicUnsupported` を返す。core が利用者に確認し、承認されたときだけ `write_in_place` (§4.4) を呼ぶ。この一本だけにする (`MoveFileExW` で置き換えると T の DACL・ADS が tmp の「所有者のみ」の DACL に黙って入れ替わり、共有フォルダで他の利用者が読めなくなる等、ACL を黙って狭める経路になるため) |
| 置換 (未決、S-09) | `SetFileInformationByHandle(FileRenameInfoEx, FILE_RENAME_FLAG_POSIX_SEMANTICS \| FILE_RENAME_FLAG_REPLACE_IF_EXISTS)` (Windows 10 1607+ / NTFS) | 採否は **S-09 (08 §3) で決める** (未決。それまでは ReplaceFileW + §4.6 の復旧)。採用する場合は ACL・ADS・属性の保持を自前で行い、§4.1 の主張範囲を更新する |
| 失敗モード | `ReplaceFileW` のエラー | `ERROR_UNABLE_TO_REMOVE_REPLACED`: T 無傷 → 共有違反として再試行 (下の共有違反と同じ間隔。尽きたら下の共有違反の行と同じく、T が無傷なら `AtomicUnsupported` を返す。テストの置換失敗の注入の既定 (`ERROR_UNABLE_TO_MOVE_REPLACEMENT`、§2.1) とは別の経路)。`ERROR_UNABLE_TO_MOVE_REPLACEMENT`: T 無傷 → `AtomicUnsupported` (上の行)。`ERROR_UNABLE_TO_MOVE_REPLACEMENT_2`: T は既に退避名に移動済みで tmp の移動に失敗 → `FsErrorKind::ReplacedMovedToBackup { backup }` (02 §4.5) を返し、復旧パスを必ずユーザーに提示する。この事故で T を失わないため、`backup` 引数には **常に** `.<name>.eve-bak-<pid>-<rand8>` を渡し (削除ではなく退避)、成功後に削除する。**クラッシュ・電源断による中間状態** (T 不在 + bak のみ、または T + tmp + bak が併存) は §4.6 の走査で検出・復旧する |
| 共有違反 (`ERROR_SHARING_VIOLATION`) | ウイルス対策・検索インデクサ・他エディタが T を開いている | **指数バックオフで再試行** (待ち時間 50, 100, 200, 400, 800, 1000 ms の 6 回、待ちの合計 2.55 s。この数値は本書が正本、03 は参照のみ)。保存ジョブ内なので UI は止まらない (待つ間は保存中の表示のまま。§4.1 手順 5 の置換の段に入っているので、待ちの間も取消しは見ない。最長でも約 2.6 s で終わる)。他のプロセスが T を `FILE_SHARE_DELETE` なしで開いたままにしている (ログの閲覧・一部のエディタ・Office 等) と `ReplaceFileW` は何度試しても失敗するので、再試行が尽きたら、T が無傷 (下の「その他の環境」の判定) なら tmp を消して `AtomicUnsupported` (detail なし) を返し、確認付きの直接上書き (§4.4。`ui.confirm.save.atomic_unsupported`、既定は「別名で保存」) に進める (03 §9.3 手順 7)。直接上書きは T を `FILE_SHARE_READ` の共有モードで開くので、相手が書込アクセスなしで開いているだけなら開ける。相手が書込アクセスで開いていれば §4.4 手順 3 の同じ再試行の後に `Busy` になる。T が無傷でなければ `ReplacedMovedToBackup` の経路 |
| 読み取り専用属性 | 文書は「置換される側に書込アクセスが必要」としているので、T が READONLY だと `ERROR_ACCESS_DENIED` になる見込み (S-09 で確認)。**v0.2**: ユーザーが「読み取り専用を解除して保存」を選んだ場合 (FR-116、`ui.confirm.save.read_only`) に限り、置換の直前に T の READONLY を外し、置換の成功後に付け直す (手順 5b)。置換に失敗したときも元に戻す。**v0.1** は読み取り専用属性のファイルを編集できず (03 §9.6)、T を書き換えない (保存は名前を付けて保存だけ) ので、この手順を使わない | |
| その他の環境 | OneDrive / Dropbox の同期フォルダ (プレースホルダファイル)、「コントロールされたフォルダー アクセス」、SMB 上の NAS (Samba) では `ERROR_ACCESS_DENIED` 以外のエラー (`ERROR_INVALID_FUNCTION`、`ERROR_NOT_SUPPORTED` 等) が返り得る | フォールバック条件を「エラー分類」ではなく **「置換に失敗し、かつ T が無傷 (metadata/file_id が手順 0b と一致)」** で判定し、該当すれば `AtomicUnsupported` として §4.4 の確認フローへ。T が無傷でなければ `ReplacedMovedToBackup` 経路。S-09 で実測 |
| HIDDEN / SYSTEM | `write_in_place` 時に `CREATE_ALWAYS` で開くと **`ERROR_ACCESS_DENIED`** (属性不一致) になる古典的な罠 → `OPEN_EXISTING` で開く。開いた直後に長さを 0 にすること (`SetEndOfFile(0)` / `TRUNCATE_EXISTING`) もしない (§4.4 手順 3) | |
| `std::fs::rename` の挙動 | 近年の std は Windows 10 1607+ で `FILE_RENAME_FLAG_POSIX_SEMANTICS` を試みる版がある。採用 MSRV での挙動を S-09 で確認し、いずれにせよ本層は `ReplaceFileW` を直接呼ぶ (`windows-sys` クレート、`unsafe` は R5 の範囲) | |
| FlushFileBuffers | リムーバブル/ネットワークで失敗し得る → 警告 | |
| ディレクトリ fsync | 不可 (ディレクトリハンドルの FlushFileBuffers は意味を持たない)。NTFS はメタデータジャーナルで整合性を保つため省略 | |
| 8.3 短縮名 | 保持は ReplaceFileW に任せる。自前では扱わない | |
| 大文字小文字 | T の既存の綴りが保持される (ReplaceFileW は T の名前を使う)。新規作成時 (`MoveFileExW` を `MOVEFILE_REPLACE_EXISTING` なしで使う、§4.1) は入力どおり | |

### 4.3 Unix (macOS / Linux) の手順

**v0.2 (macOS / Linux 出荷時) の仕様**。v0.1 では実装しない (v0.1 の CI の Linux ジョブは Linux CI の対象 crate (02 §4.7。`eve-platform` は Unix 向けの空の実装だけ) を扱い、本手順は通らない)。

| 段階 | API | 注意 |
|---|---|---|
| 一時ファイル作成 | `open(O_WRONLY\|O_CREAT\|O_EXCL\|O_CLOEXEC, 0600)` | `O_TMPFILE` + `linkat` は Linux 限定かつ上書き不可なので使わない。macOS は `O_CLOEXEC` 可 |
| 権限複製 | `fchown(tmp, uid, gid)` (必要時のみ。失敗 EPERM は §4.4 で事前判定) → `fchmod(tmp, mode & 07777)` の順 (逆順だと、0640 のファイルが所有グループを合わせる前に利用者の既定グループから一瞬読める) | setuid/setgid ビットは **保持しない** (安全側。警告) |
| xattr / ACL の検出 (複製より先に出荷する場合の最低限) | `flistxattr(T)` で T に xattr・POSIX ACL (`system.posix_acl_access`)・macOS の ACL / Finder タグが付いているかだけを調べる | 付いていれば「拡張属性 / ACL は保存で保たれません」を SaveWarning で知らせる (黙って失わない)。親ディレクトリに既定 ACL がある場合 (`system.posix_acl_default`)、tmp がそれを継承して元のファイルと違う相手に読める状態になり得るので、同じ警告を出す。どちらも下の複製で解消する |
| xattr / ACL の複製 (FR-106) | Linux: `flistxattr`/`fgetxattr`/`fsetxattr` で **`user.*` と `trusted.*` (権限があれば) を複製し、`security.*` と `system.*` は複製しない** (§5.1 と一致)。POSIX ACL (`system.posix_acl_access`) は xattr としてではなく `acl_get_fd` / `acl_set_fd` 相当で別処理する (`system.*` を一律除外しても ACL が落ちないようにする)。macOS: `copyfile(T, tmp, NULL, COPYFILE_XATTR \| COPYFILE_ACL)` (`com.apple.quarantine` はコピーしない: 新しい内容に検疫を付け直す理由がない) | 失敗は警告。`security.selinux` は新規ファイルのラベルを OS に任せる (restorecon 相当) |
| 不変フラグ | Linux `FS_IMMUTABLE_FL` / macOS `UF_IMMUTABLE` (chflags uchg) が立っていると rename に失敗 → `PermissionDenied` で「ファイルがロック (不変) されています」 | |
| fsync (一時ファイル) | Linux `fsync`、macOS `fcntl(F_FULLFSYNC)` (遅い: 数〜数十 ms。SSD では NFR-107 内)。F_FULLFSYNC が失敗したら `fsync` にフォールバックして警告 (§4.1 手順 3。std はフォールバックしない) | ext4 `data=ordered` でも rename 前の fsync は必須 (省略すると 0 バイトファイル事故) |
| 置換 | `rename(tmp, T)` (POSIX で原子的) | — |
| ディレクトリ fsync | `open(dir, O_RDONLY\|O_DIRECTORY)` → `fsync` | 失敗 (EINVAL on some FS) は警告 |
| 作成日時 (macOS birthtime) | 保持できない (rename で新 inode)。Finder の「作成日」が変わるのは **仕様とする** | |
| Finder タグ・カラー | xattr (`com.apple.metadata:_kMDItemUserTags`) として複製される (複製を実装するまでは上の検出で警告するだけ) | |
| mtime | 新しい mtime になる (当然)。`SaveReport.new_stamp` で返す | |

### 4.4 原子的置換を諦めて `write_in_place` にフォールバックする条件

v0.1 で実装するのは Windows の行と、OS に依らない行 (ハードリンク・一時ファイル) の Windows での判定である。Unix の検出 (`access`・`st_uid`・`EXDEV` 等) と root での所有者の維持 (FR-106) は v0.2 (macOS / Linux 出荷時)。本節に出る設定キー (`file.hardlink_policy` / `file.save_strategy` / `file.in_place_patterns`) の公開は v0.2 で、v0.1 は既定値の定数で動く (07 §5)。

| 条件 | 検出 | 既定の挙動 |
|---|---|---|
| 親ディレクトリに書込権限がない (ファイル自身は書ける) | `access(dir, W_OK)` 失敗 / Windows: 一時ファイル作成が `ERROR_ACCESS_DENIED` | 確認ダイアログ → 承認で in-place |
| ハードリンク (`nlink > 1`) | `FileMeta.nlink` | **確認** (`HardlinkPolicy::Ask`。確定 (ユーザー回答)、OQ-73)。選択肢: リンクを切って置換 (`BreakLink`。通常の原子的置換) / 直接上書き (`WriteInPlace`。リンク維持) / キャンセル。設定 `file.hardlink_policy` (07 §5.2) で `BreakLink` / `WriteInPlace` に固定可 |
| 他ユーザー所有で root でない (Unix、v0.2) | `FileMeta.owner_is_me == Some(false)` (Unix: `st_uid != geteuid()` かつ `geteuid() != 0`。判定できない OS・FS は `None` で、置換を試みる) | 確認 → in-place (所有者を保てないため)。sudo 経由なら通常の置換 |
| Windows で ReplaceFileW が ERROR_ACCESS_DENIED (T 無傷) | ACL でディレクトリの削除権限がない等 | 確認 → in-place |
| FUSE / 特殊 FS で rename が `EXDEV`/`ENOTSUP` (Unix、v0.2)、Windows で置換失敗かつ T 無傷 (§4.2) | 置換失敗 | 確認 → in-place |
| **一時ファイル (`visudo` / `crontab -e` / `kubectl edit` 等、呼び出し元が同一 inode を期待)** | 下記の判定規則 (T が存在するときだけ。存在しない T は §4.1 の「新規ファイル」の経路) | **確認なしで in-place** (確定 (ユーザー回答)、OQ-34。`write_atomic` は書かずに `AtomicUnsupported` (detail = `InPlacePattern`) を返し、core が同じジョブで乾式走査 → `write_in_place` に進む。結果は `SaveMethod::InPlace`。事前バックアップは下の手順のとおり省略しない。07 §18.6) |
| ユーザー設定 `file.save_strategy = "in_place"` (07 §5.2) | 設定 | T が存在するときは常に in-place (警告は起動時 1 回)。`write_atomic` を経由せず、core が最初から method = InPlace を選び、乾式走査の後に `write_in_place` を呼ぶ。T が存在しなければ設定に依らず §4.1 の「新規ファイル」の経路 (`write_atomic`、`SaveMethod::NewFile`) |

確認を出す行 (「確認 → in-place」) の挙動は設定 `file.atomic_save_fallback` (07 §5.2。`ask` \| `fail`、既定 `ask`。v0.1 は定数で公開は v0.2) に従う。`ask` は「直接上書き (バックアップ付き) / 別名で保存 / キャンセル」の確認 (04 の `ui.confirm.save.atomic_unsupported`。既定の選択は「別名で保存」) を出し、`fail` は確認を出さずに保存の失敗として報告する。確認なしで直接上書きする値は持たない (ADR-0016)。ハードリンク (`file.hardlink_policy`) と一時ファイル規則 (`file.in_place_patterns`) の行はこの設定の対象外。

**一時ファイルの判定規則 (ヒューリスティックではなく明示的なパターン表)**: 手順 0a で `canonicalize` した T の親ディレクトリを、次の **正準化済み一時ディレクトリ集合** と比較する (macOS の `/tmp` → `/private/tmp`、シンボリックリンク経由の `$TMPDIR` を誤判定しないため、両方を `canonicalize` してから比較する)。

| 判定 | 内容 |
|---|---|
| 一時ディレクトリ集合 | `$TMPDIR` / `$TMP` / `$TEMP` (設定されていれば)、Windows: `%TEMP%` / `%TMP%`、Unix (v0.2): `/tmp`, `/var/tmp`、macOS (v0.2): `$TMPDIR` (`/var/folders/.../T`)。いずれも起動時に 1 回 `canonicalize` し `Paths.temp_dirs` に保持 |
| 対象 | T の親ディレクトリが集合のいずれかと **一致** すること (**一時ディレクトリの直下のみ。サブディレクトリの中は含まない**)。**かつ** ファイル名が既知パターン (`crontab.*`、`*.sudoers.tmp` / `sudoers.tmp.*`、`kubectl-edit-*`、`*.tmp`) に一致するか、設定 `file.in_place_patterns` (07 §5.2、glob 配列、既定は上記) に一致すること。パターンはファイル名だけに照合し、`/` を含むパターンは無効 (診断) |
| 対象外の例 | `$TMPDIR/work/notes.tmp` (サブディレクトリ内)、`$TMPDIR/report.txt` (パターン不一致)、`git-rebase-todo` / `COMMIT_EDITMSG` (`.git/` 配下)。これらは通常どおり原子的置換 (他の条件に該当すれば確認) |
| 通知 | 保存の完了をメッセージ `info.save.in_place` (04 §15.2。直接上書きで保存したこと。事前バックアップは成功後に消すので場所は示さない、下の手順 9) で知らせる。ステータスバーの項目にはしない。FR-119 バックアップ設定が有効なら通常どおり作る |

`write_in_place` の手順 (in-place でも 01 FR-105 AC-2 の「容量不足で元ファイルは変化しない」を守る。元ファイルに書き始めるのは手順 6 からで、それより前のどの失敗でも元ファイルは変わらない)。**取消し** (正本は 02 §4.5 の「取消し」): main は常に `CancelToken` を立て、無視するかどうかはジョブと本層が決める。取消しを見るのは手順 0 の乾式走査 (core) と、手順 1〜5 (照合・開く・事前バックアップの書込のチャンクごと・確保) の本層で、本層は引数の `cancel` が立っていれば手順 9 の「手順 6 より前の失敗」と同じ後片付けをして `SourceFailed` を返す (元ファイルは無傷。core が `Cancelled` に写す)。手順 6 以降は見ない (手順 6):

0. **乾式走査 (呼び出しの前)**: core のエンコーダ (`SaveEncoder::dry_scan`、03 §9.3 の「乾式走査と出力長」) が、全文を **エンコード + 照合だけ** 行って書き出さない走査を 1 回行い、表現不能文字 (黙示置換を含む) の有無と出力バイト長を確定させる (エンコードと照合は 03 の責務。03 §9.3)。表現不能が 1 件でもあれば **元ファイルに一切触れずに中止** し、03 §8.6 E3 の選択肢に進む (`write_in_place` を呼ばない)。原子的置換では表現不能の検出時に一時ファイルを捨てれば済むが、in-place では書き始めた後に見つかると元ファイルが壊れるため、この走査を省略しない。確定した出力バイト長 (`DryScan.out_len`。本節では `new_len` と書く) は `write_in_place` の引数 `len` として本層に渡し、手順 5 の確保と手順 7 の長さ合わせに使う (受け渡しの型は 02 §4.5)。乾式走査は `CancelToken` を見て、立っていれば `Cancelled` で終わる (元ファイルに触れていない)。
1. **書き始める前の照合**: `metadata(T)` を `SaveOptions.expected` と §5.5 の規則で比べる (内容ハッシュはここでだけ比べる)。不一致なら `Conflict { current: Some(T の Stamp) }` を返す。`Expect::Stamp` を期待していて T が無い (削除された) ときは `NotFound` ではなく `Conflict { current: None }` を返す (§4.1 手順 0b と同じ規則)。何も書かない。
2. **保存中フラグ**: §7.5 の保存中フラグを書く (`method = InPlace`、`backup_complete = false` を記録する)。手順 4 のバックアップの最終名と `backup_recorded = true` を記録するのは **`SaveOptions.in_place_backup = Required` のときだけ** で、`SkipConfirmed` (バックアップなしの承認) では `backup` を記録せず `backup_recorded = false` とする (中断したときは `msg.open.interrupted_save_no_backup` で示す、手順 10)。**書けなければ何も書かずに中止し、`BackupFailed` を返す** (in-place ではフラグが無いと、中断したときに次に開く利用者へ「壊れている可能性」とバックアップの場所を知らせる手段が無いため。§4.1 手順 0d の「警告して続ける」は原子的置換だけの規則)。core は手順 4 の失敗と同じ確認 (「バックアップ無しで上書きしますか」) を出す。承認 (`SaveOptions.in_place_backup = SkipConfirmed`) の後はフラグを書けなくても `SaveWarning::InflightFlagNotWritten` に積んで続ける。
3. **開く**: T を `OPEN_EXISTING` (Unix は `O_RDWR`。`O_CREAT` / `O_TRUNC` なし) で、読み書き両方のアクセスで開く。Windows の共有モードは **`FILE_SHARE_READ` のみ** (バックアップを取ってから上書きを終えるまで、他のプロセスが書く・名前を変える・削除することを許さない。バックアップを取った内容と上書きする内容の間に他者の書込が入らない)。**`CREATE_ALWAYS` / `TRUNCATE_EXISTING` / `O_TRUNC` も、開いた直後の `SetEndOfFile(0)` / `ftruncate(0)` も使わない** (この時点で元の内容を消さない)。共有違反 (`ERROR_SHARING_VIOLATION` / `ERROR_LOCK_VIOLATION`) は §4.2 と同じ指数バックオフ (50〜1000 ms の 6 回、待ちの合計 2.55 s) で再試行し、それでも開けなければフラグを消して `Busy` を返す (何も書いていない)。開いたハンドルで `(mtime, len, FileId)` を取り、手順 1 の値と比べる (内容ハッシュは比べない)。不一致なら閉じてフラグを消し、`Conflict` を返す (`current` は手順 1 と同じ規則)。
4. **事前バックアップ (手順 3 のハンドルから取る)**: 手順 3 で開いたハンドルから元のバイト列を先頭から読み、`state_dir/backup/` (§7.5) の **一時名** (`<最終名>.tmp`) に書いて fsync し、閉じてから **最終名に名前を変える** (途中で中断したバックアップが完全なものに見えないようにする)。名前を変えたら、フラグを `backup_complete = true` に書き換える (フラグ自身の書き方は §7.5。この手順を行うのは `in_place_backup = Required` のときだけなので、書き換えも `Required` のときだけ)。元ディレクトリを汚さない。FR-119 のバックアップ (`state_dir/backup/fr119/`、v0.3) とは別の「安全網」で、設定に依らず、一時ファイル規則による in-place でも省略しない。**バックアップ (フラグの書き換えを含む) に失敗したら (容量不足を含む) 何も書かずに中止** し、一時名のバックアップとフラグを消して `BackupFailed` を返す。core は「バックアップ無しで上書きしますか」を確認し (既定はキャンセル。確定 (ユーザー回答))、承認されたとき (`SaveOptions.in_place_backup = SkipConfirmed`) だけ手順 4 を飛ばして行う (03 §9.3)。`state_dir` を使えないときも同じ扱い。確認の文言には「バックアップなしで上書きすると、上書きの途中で失敗・中断したときに元の内容を取り戻せません」を含める (04 §15.2 の `ui.confirm.save.overwrite_without_backup`)。
5. **領域の事前確保**: `new_len` が元の長さより大きければ、書き始める前に領域だけを確保する (Windows は `SetFileInformationByHandle(FileAllocationInfo)` **だけ** を使い、EOF は延ばさない (`SetEndOfFile` を呼ばない)。v0.2 で Linux `fallocate(FALLOC_FL_KEEP_SIZE)`、macOS `fcntl(F_PREALLOCATE)` (いずれもファイルの長さを変えない)。確保の API を使えない FS (`ERROR_INVALID_FUNCTION` / `ERROR_NOT_SUPPORTED` 等) では、代わりに空き容量 (`GetDiskFreeSpaceExW` の `lpFreeBytesAvailableToCaller`。クォータを反映した呼び出し元の空き容量で、ボリューム全体の空き `lpTotalNumberOfFreeBytes` は使わない) が増える分 (`new_len` − 元の長さ) を上回ることだけを確かめて進む。この場合は上書き中の容量不足を完全には防げないので、手順 4 のバックアップが安全網になる (手順 8))。確保できない、または空き容量が足りなければ `DiskFull` を返す (長さも内容も変えていないので元ファイルは変わらない。手順 9 の「手順 6 より前の失敗」の後片付けをする)。
6. **上書き**: 先頭から新しい内容を書く (元の長さを超える部分は追記になる)。手順 6 の開始から手順 7 の fsync の完了まで、§4.1 手順 5 と同じ「置換中」の計数を `ReplacingGuard` で上げる (§4.1 の「置換中の計数」。in-place では書込の全体が元ファイルを壊し得る区間なので、置換の 1 回分ではなく書込の全体を覆う)。§11.5 の通常の終了は、計数が 0 に戻るまで exit しない。§11.4 の非対話の終了は時間予算の範囲で待ち、超えたら打ち切る (手順 10 で回復する)。**手順 6 の開始以降は `CancelToken` を見ない**: 取り消されていても手順 7 まで進めて成功なら `Saved` になる結果を返す。本書込のエンコーダ (`data`) も `ChunkError::Aborted` を返さない (03 §9.3 手順 5d で原子的保存から切り替わった場合も同じ。02 §4.5 の「取消し」)。
7. **長さ合わせと fsync**: 書き終えてから、`new_len` が元の長さより小さければ末尾を縮める (Windows `SetEndOfFile`、Unix `ftruncate(new_len)`)。縮小は書込の完了後にだけ行う。fsync する。
8. **途中の失敗**: 手順 6 / 7 で I/O エラー (容量不足を含む) になったら、元ファイルは途中まで書き換わっている。`InPlaceFailed { backup: Some(バックアップの最終名) }` を返し、バックアップの場所を必ず示す (`SaveReport` と 04 の保存エラーの文言)。利用者が「バックアップなしで上書き」を承認していた (手順 4 を飛ばした) 場合は `InPlaceFailed { backup: None }` で、元の内容は取り戻せない (確認の時点で示した内容)。コピーオンライトの FS (btrfs・ZFS・APFS) や確保を使えない FS では手順 5 が上書きの領域を保証しないので、手順 4 のバックアップが安全網になる。本書込でエンコーダが失敗した場合 (乾式走査と結果が食い違う。実装の誤り) も同じ扱いにする。
9. **後片付け**: 成功 (手順 7 の fsync まで) したら、**先に保存中フラグを削除し (同じキーに残る他の pid の記録も消す。§4.1 手順 7)、その後で** 手順 4 のバックアップを削除する (この順なら、間で中断してもフラグが指すバックアップが無くなることはない。フラグが無いバックアップ (間で中断した、またはバックアップの削除に失敗した) は孤児として残る。`list_inflight()` はフラグしか返さないので、v0.2 の起動時の走査は `state_dir/backup/` の直下も列挙して、どのフラグにも記録されていない事前バックアップを見つけて片付けを提案する (§4.6)。削除に失敗したときは SaveWarning でそのパスを知らせる)。利用者向けのバックアップは FR-119 の設定で別に作る。手順 6 より前の失敗でも同じ順 (フラグ → バックアップ (一時名を含む)) で削除する。`InPlaceFailed` ではどちらも残す。
10. **中断 (プロセスの終了・電源断)**: 手順 6 / 7 の途中でプロセスが終わった場合 (`TerminateProcess`、CTRL_CLOSE の予算超過 (§11.2)、電源断)、T は半端に書き換わっているが、フラグ (`method = InPlace`、`backup_complete = true`) と完全なバックアップが残る。次に T を開いたときに `inflight_for` がそれを見つけ、`msg.open.interrupted_save` でバックアップの場所を示す (§4.6)。`backup_recorded = true` かつ `backup_complete = false` のフラグ (バックアップの途中で中断。T はまだ変わっていない) は「バックアップは不完全」として別の文言 (`msg.open.interrupted_save_incomplete`、04 §15.2) で示す (案内するのは実在する `<最終名>.tmp` で、それも無ければ T は変わっていないので報告しない。§4.6)。バックアップなしの承認の後の中断 (`backup_recorded = false`) は取り戻せない (フラグがあれば「中断された」ことだけを `msg.open.interrupted_save_no_backup` で示す)。**切断点** (09 §3.6 (6) (iii) の test-hooks。この手順の順に): `kill_after_inflight_flag` (手順 2 の直後、バックアップの前) / `kill_mid_backup` (手順 4 でバックアップの一時名 `<最終名>.tmp` を書いている途中。開くと `msg.open.interrupted_save_incomplete`、T は無傷) / `kill_after_backup` (手順 4 の後、`backup_complete = true` の記録の後) / `kill_after_inplace_alloc` (手順 5 の直後) / `kill_mid_inplace_write=<バイト数>` (手順 6 で指定バイト数を上書きした直後)。

### 4.5 読込側の注意

- **ストリーム読込・チャンク**: 64 KiB〜1 MiB のチャンク (ベンチで決定、09)。`read` の `EINTR` 再試行。Windows の `ReadFile` はネットワーク越しで部分読込を返す。
- **ファイルハンドルは読込完了後すぐ閉じる**: Windows では開いたままにすると他プロセスの rename/削除が `SHARING_VIOLATION` になり、他エディタの原子的保存を妨げる。読込時は `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE` を指定する (std の `OpenOptions` は既定でこの 3 つを指定する)。
- **読込中の外部変更**: 読込前後の `FileStamp` (mtime, len, FileId) を比較し、変化していれば `changed_during_load` として core に伝え、再読込を提案 (03 §9.2 手順 3。サイズが増え続けるログは FR-117 の UI に乗せる)。読む量は読込前の `metadata` の len + 余裕 (max(1 MiB, len / 16)) で打ち切り (伸び続けるファイルを際限なく読まない)、打ち切ったときも `changed_during_load` とする。読んだバイト数とデコード後のバイト数は core が閾値と照合する (03 §9.2 手順 3 の「読込中のメモリ検査」)。16 MiB 以下は読込中に内容ハッシュをストリーミング計算して `FileStamp.hash` に入れる (§5.5)。
- **mmap は使わない** (外部からの truncate で SIGBUS になるため。ADR-0004)。
- **デバイス・FIFO・ソケット**: `FileKind::Other` → 開かない (`InvalidPath`、detail `NotAFile`)。Windows のデバイス名前空間 (`\\.\…`・`\\?\GLOBALROOT\…`) と代替データストリーム (`a.txt:s`) は、`metadata` を呼ぶ前に §3.2 手順 5 の正規化で拒否する (同じ `InvalidPath` / `NotAFile`)。`/dev/stdin` は FR-120 の経路で扱う。
- **Windows 予約名** (一覧は §3.1 の「禁止文字・予約名」の行) を開こうとした場合 (`evoeditor con`): `InvalidPath` (detail `ReservedName`)。
- **ディレクトリを指定** (`evoeditor .`): v0.1 は無題の文書を開き、最初の Frame の後に `msg.cli.directory` で知らせる (07 §18.2。エディタ内のファイルを開く操作でディレクトリを指定したときは `msg.open.not_a_file`)。v0.4 でファインダを開く (FR-125)。

### 4.6 孤児ファイルの検出と復旧

保存の中断 (クラッシュ・電源断・`TerminateProcess`・`ReplaceFileW` の中間状態) で同じディレクトリに残る `.<name>.eve-tmp-<pid>-<rand8>` / `.<name>.eve-bak-<pid>-<rand8>` は、**保存中フラグ** (`state_dir/recovery/inflight/<hash(path)>/<pid>-<seq>.toml`、§7.5) を手掛かりに検出する。**ファイルを開くたびのディレクトリ列挙は行わない** (列挙は 02 §4.5 の `list_inflight()` に一本化する。ネットワーク FS での `read_dir` が開く操作を遅くし、v0.1 の範囲も増やすため)。

| 契機 | 版 | 内容 |
|---|---|---|
| 保存ジョブ自身 | v0.1 | 置換途中で失敗したら **その場で** 退避パス付きで報告する (`ReplacedMovedToBackup { backup }`、§2.1、03 §9.3)。フラグは保存ジョブ開始時 (一時ファイルを作る前、§4.1 手順 0d) に書き、成功時に消す |
| 開くとき (T の有無に依らない) | v0.1 | 読込ジョブ内で、開くたびに `inflight_for(path)` により `inflight/<hash(path)>/` だけを 1 回調べ、残っている記録をすべて (`Vec<InflightRecord>`) 受け取る (ディレクトリ全体は列挙しない。O(1)。シンボリックリンクならリンクとリンク先の 2 つのキー、§7.5)。記録と T の有無から 02 §4.5 の `InterruptedSave { method: InterruptedMethod, backup, tmp, complete }` (`JobResult` の `interrupted_save`) を作る規則は下の「開くときの写像」の表 (正本は 02 §4.5 の「中断の痕跡の写像」) による。T が無いときに黙って空の新規文書として開かず (03 §9.2)、T があるときも文書は通常どおり開いて場所を示すだけにする。`inflight_for` が複数の記録を返したら、表の上の行に当たるもの (in-place の記録が先) を 1 つ示す。v0.1 は pid の生存確認 (§5.3) をしないので、他インスタンスが保存中の記録でも警告する (誤報は害が小さい側に倒す)。復旧の操作 (下の表) は v0.2 で、v0.1 は場所の案内だけ |
| 起動時の inflight 走査 (03 §10.3) | v0.2 | `list_inflight()` で残っているフラグを列挙し、記録された T のパスについてだけ `.<basename>.eve-tmp-*` / `.eve-bak-*` の有無を調べる (ジョブ内)。開いていないファイルの痕跡も見つかる。pid の生存確認 (§5.3) で他インスタンスが保存中のものを除く。加えて **`state_dir/backup/` の直下** (in-place の事前バックアップ専用の場所。evoeditor 自身の小さなディレクトリなので列挙してよい。`fr119/` は対象外) を列挙し、どのフラグにも記録されていない `.bak` / `.bak.tmp` (in-place の成功後の削除の前に中断した、または削除に失敗したもの。§4.4 手順 9) を、名前の pid が生存していなければ孤児として片付けの提案に加える |

**開くときの写像** (v0.1。`inflight_for` の記録 × T の有無 × バックアップの実在 → `InterruptedSave` と 04 §15.2 の文言。**正本は 02 §4.5 の「中断の痕跡の写像」** で、本表はその写し)。`backup` / `tmp` には **開くジョブが実在を確かめたパスだけ** を入れる (存在しないパスを案内しない)。`tmp` はどの行も記録された一時ファイルのうち実在するもの (無ければ `None`)。報告する記録が複数あれば表の上の行に当たるものを 1 つ返す (不正な組合せの `Atomic` / `NewFile` の記録は、それより上の「上書きの可能性あり」の行で扱う)。

| 記録 (`method` / `backup_recorded` / `backup_complete`) | T | バックアップの実在 | `InterruptedSave` | 文言 |
|---|---|---|---|---|
| `InPlace` / true / true | 無し・有り | 最終名が在る | `method = InPlace`、`backup = Some(最終名)`、`complete = true` | `msg.open.interrupted_save` |
| `InPlace` / true / false (バックアップの途中で中断。T はまだ変わっていない) | 無し・有り | 一時名 `<最終名>.tmp` が在る (complete = false のときバックアップの実体は `<最終名>.tmp` か、無いかのどちらかで、最終名は実在しない) | `method = InPlace`、`backup = Some(<最終名>.tmp)`、`complete = false` | `msg.open.interrupted_save_incomplete` |
| `InPlace` / true / false | 無し・有り | 一時名も無い | 作らない (`None`。上書きは始まっておらず、示す痕跡も無い) | なし |
| `InPlace` / false / false (バックアップなしの承認、§4.4 手順 2) | 無し・有り | — | `method = InPlace`、`backup = None`、`complete = false` | `msg.open.interrupted_save_no_backup` |
| 上書きの可能性あり: `InPlace` / true / true で最終名が無い (外部で消された)、上の行に無い不正な組合せ (`InPlace` / false / true、`Atomic` / `NewFile` で `backup_recorded` か `backup_complete` が true)、解析できないフラグ (`malformed = true`) | 無し・有り | 最終名が無い (または記録から読めない) | `method = InPlace`、`backup = None`、`complete = false` (`tmp` は記録から読めて実在するものだけ) | `msg.open.interrupted_save_no_backup` (安全側に倒す)。**フラグを残す** (掃除しない) |
| `Atomic` | 無し | 退避名 `.eve-bak-*` が在る (`ReplaceFileW` の途中) | `method = Atomic`、`backup = Some(退避名)`、`complete = true` (元ファイルの移動なので完全) | `msg.open.interrupted_save` |
| `Atomic` | 無し | 退避名が無い | 作らない (`None`。T は外部で消えたもので、保存の痕跡ではない) | なし |
| `Atomic` / `NewFile` | 有り | — | 作らない (`None`。T は置換・作成の前の版か後の版のどちらかで完全) | なし |
| `NewFile` | 無し | — | `method = NewFile`、`backup = None`、`complete = false` | `msg.open.interrupted_new` |

**フラグを消す条件**: 「作らない」記録は開くジョブが黙って掃除する: **フラグだけ** を消し、一時ファイル・バックアップは消さない。ただし、記録された一時ファイル・退避名 (T と同じディレクトリの `.eve-tmp-*` / `.eve-bak-*`) のどれかが実在する記録 (置換の成功後に退避名を消せなかった、置換の前の失敗で一時ファイルを消せなかった、置換の前に中断した、等) は **報告せずにフラグを残す** (消すと、残った一時ファイル・退避名を v0.2 の起動時の走査で見つける手掛かりが無くなる。T が在る間は報告しない行なので、開くたびの警告は出ない)。実在の確認は写像のための `metadata` と同じジョブで行う (v0.1 は pid の生存確認をしないので、他インスタンスが保存中の記録を消すことがあるが、その保存の一時ファイル・退避名には触れないので保存自体は失敗しない)。報告した記録 (文言のある行) は開くジョブは消さず、同じ T への保存が成功したときに保存の後片付け (§4.1 手順 7、§4.4 の成功後) が他の pid の分も含めて消す (02 §4.5 の「報告した記録を消す時機」)。それまでは開くたびに同じ案内が出る。この後片付けも v0.1 は pid の生存確認をしないので、同じ T を同時に保存中の他インスタンスの記録を消し、その保存が中断したときの痕跡を失うことがある (v0.2 の pid の生存確認 (§5.3) で除く)。

| 状態 | 判定 | 提案 (既定を太字) |
|---|---|---|
| T なし、bak あり | `ReplaceFileW` の中間状態で T が退避名に移った後に新内容の配置に失敗 | **bak を T にリネームして復旧** / bak を別名で開いて確認 / 削除 |
| T あり、bak あり | 置換成功後の bak 削除漏れ (手順 7)、または T が別プロセスにより再作成された | **bak を削除** (内容が T と同一なら無確認で削除可) / bak を別名で開いて比較 |
| tmp あり (T の有無に関わらず) | 書込途中、または fsync 後に置換前で中断 (tmp は完全な新内容の可能性もある) | **別名で開いて確認** (完全かどうかは判定できない。末尾が切れている可能性を明示) / 削除 |
| tmp と bak の両方 | 置換の途中 | bak を T に戻し、tmp を別名で開いて確認 |

- 復旧の適用は 03 §10.3 の「Transaction(Origin::Recovery) で全置換、dirty = true」経路または「bak → T へのリネーム」(ディスク操作、確認必須) のどちらか。削除は確認を求める。
**起動時の提案の規則** (v0.2 の M6e。**(案)**: PO の確認前。上の表の曖昧な点を、実装の前に決める):

1. **提案の単位は T のパス 1 つ**。同じ T に当たるフラグ・`.eve-tmp-*`・`.eve-bak-*` はまとめて 1 件の提案にする。上の表は次の順で当てはめ、最初に当たる行を提案にする: (i) T なし・bak あり・tmp あり → **「tmp と bak の両方」**、(ii) T なし・bak あり、(iii) T あり・bak あり (tmp が同時にあれば、(iv) の提案を続けて出す)、(iv) tmp あり。「両方」は **T が無いときだけ** で、T があるときは bak と tmp を別々に扱う。
2. **操作の定義**:
   - 「bak を T に戻す」= `bak` を T の名前に rename する (ディスク操作。確認が要る。T が既にあれば失敗させて何もしない)。戻した後は、その文書を通常どおり開ける。
   - 「bak を削除」= bak を消す。T と bak の内容が同一 (サイズが同じで、ジョブ内の全文比較が一致。**両方が 64 MiB 以下のときだけ比べる**) なら確認なしで消し、それ以外は確認を求める。
   - 「tmp を別バッファで開く」= tmp を **読み取り専用の文書として追加で開く** (M9a の `buffer.*` に加える。T の文書の内容は変えない)。末尾が切れている可能性を、開いたときのメッセージで示す。保存はできない。tmp の削除は別の操作で、確認を求める。
   - 「tmp と bak の両方」= 「bak を T に戻す」を先に行い、成功したら tmp を別バッファで開く。bak を戻せなかったら tmp は開かない。
3. **既定**: (i) は「両方」、(ii) は「bak を T に戻す」、(iii) は「bak を削除」、(iv) は「別バッファで開く」。Esc は何もせず閉じ、痕跡は残す (次回の起動でまた提案する)。
4. **出す順と時機**: 起動時の走査 (ジョブ) の結果を、03 §10.3 手順 5 のスワップの提案と同じ待ち行列に入れる。**スワップの提案を先に**、その後に置換途中の提案を新しい順 (フラグの記録時刻) に 1 件ずつ出す。確認ダイアログの表示中は保留する。
5. **フラグの扱い**: 提案を片付けた (戻した・消した・開いて「削除」した) とき、その T のフラグを消す。Esc で何も選ばなかったときは残す。pid が生存しているフラグは対象にしない (§5.3)。
6. **孤児の `state_dir/backup/`**: pid が生存していない `.bak` / `.bak.tmp` は、提案の対象でなく **無確認で片付ける** (どのフラグにも記録されていない in-place の事前バックアップの削除漏れ。ただし `.bak` は、同名の T が今も在って内容が同一のときだけ消し、それ以外は残して提案に加える)。

- `.eve-tmp-*` / `.eve-bak-*` とフラグのファイル名には **pid** を含め (起動時刻はフラグの中に書く)。pid をファイル名に含めるのは一時ファイル・退避名・フラグの名前の衝突を避けるためで、フラグの名前にはさらにプロセス内の保存の通し番号 `<seq>` を付ける (`<pid>-<seq>.toml`、§7.5。同じプロセスが同じ T を続けて保存したとき、前の保存が残したフラグ (一時ファイル・退避名を消せなかった記録) を次の保存のフラグで上書きして痕跡を失わないため)。フラグは同じ T への保存の成功で自分の今回の記録以外の分も消える (痕跡ファイルが実在する記録は残す。§4.1 手順 7、02 §4.5 の「報告した記録を消す時機」)。pid と起動時刻が一致する生存プロセスがあるもの (他インスタンスが保存中) は孤児とみなさない (§5.3 の生存確認。v0.2、起動時の走査と同時)。フラグの列挙は 02 §4.5 の `list_inflight()` (v0.2)。
- テスト: 09 §3.4 の `MemFs` で各中間状態を生成し、09 §3.6 で Windows の `TerminateProcess` による実 FS 試験を行う。01 FR-105 AC-1 の対象。

---

## 5. 権限・属性・ロック・外部変更検知

### 5.1 保存で保持する属性 (FR-106) の一覧

v0.1 は Windows の列だけを実装する。macOS / Linux の列は v0.2 (macOS / Linux 出荷時) の仕様。

| 属性 | Windows | macOS | Linux | 保持可否 | 版 |
|---|---|---|---|---|---|
| 実行ビット・パーミッション | — | mode | mode | 保持 (setuid/setgid は落とす) | v0.2 (Unix のみ) |
| 所有者 | ACL 所有者 (ReplaceFileW で保持) | uid/gid (fchown、root 時のみ変更可) | 同左 | 保持。不可なら §4.4 | Windows は v0.1 (ReplaceFileW に任せる)、Unix は v0.2 |
| 読み取り専用 | READONLY 属性 | `UF_IMMUTABLE` は保存自体を拒否 | `FS_IMMUTABLE_FL` 同左 | 保持 | v0.1 |
| 隠し・システム | HIDDEN / SYSTEM 属性 | `UF_HIDDEN` (chflags hidden) は xattr ではなくフラグ → `fchflags` で複製 | ドットファイル (名前) | 保持 | Windows の属性は v0.1、`UF_HIDDEN` の複製は v0.2 |
| ACL | ReplaceFileW で保持 | `copyfile(COPYFILE_ACL)` | POSIX ACL xattr | ベストエフォート | v0.2 (ReplaceFileW が保持する分は v0.1 から結果として残る) |
| 拡張属性 | ADS (ReplaceFileW で保持。`Zone.Identifier` 含む) | xattr (`com.apple.quarantine` を除く) | xattr (`security.*` を除く) | ベストエフォート | v0.2 (同上) |
| タイムスタンプ | 作成日時は保持 (ReplaceFileW)、更新日時は新しく | birthtime は失われる | — | mtime は常に更新 | v0.1 |
| 圧縮・暗号化 (NTFS) | ReplaceFileW が保持 | — | — | 保持 | v0.1 |

v0.1 (Windows) で本層が自ら複製するのは READONLY・HIDDEN・SYSTEM 属性だけである (FR-106)。ACL・ADS は `ReplaceFileW` が保つ分だけで、置換に `ReplaceFileW` 以外を使わない (§4.2) ので、本層が Windows の ACL・ADS を自前で複製する経路は無い (S-09 で `FileRenameInfoEx` を採る場合はこの段落を改める)。Unix の mode / 所有者と、xattr・ACL・`UF_HIDDEN` の明示的な複製 (§4.3) は v0.2。直接上書き (§4.4) は同じファイルに書くので属性・ACL・ADS はすべてそのまま残る。

### 5.2 読み取り専用の判定 (FR-116)

- Unix (v0.2): `faccessat(AT_EACCESS, W_OK)` で実効ユーザーの書込可否。ディレクトリ不可書込 (保存できない) とは区別し、後者は保存時に §4.4。
- Windows: READONLY 属性 **または** `CreateFileW(GENERIC_WRITE)` の失敗 (ACL) で判定。属性だけ見ると ACL 拒否を見逃す。判定のためにだけ書込ハンドルを開くのは避け、属性 + `GetEffectiveRightsFromAcl` 相当は複雑なので **「属性 READONLY なら read-only (`FileAttribute`)、それ以外は保存時に `PermissionDenied` を受けたら read-only (`NoWritePermission`) に切り替える」** という楽観方式を採る (v0.1)。理由の一覧と解除の可否の正本は 03 §9.6。
- 読み取り専用 Document で v0.1 にできる保存は「名前を付けて保存」だけ (保存先について読み取り専用の判定をやり直す、03 §9.6)。`FileAttribute` を外して元のファイルに書く「読み取り専用を解除して保存」は v0.2 (§4.2 の手順 5b)。`NoWritePermission` は解除できない (名前を付けて保存で回避する)。`sudo` 昇格して書く機能 (Vim の `:w !sudo tee`) は **やらない** (P2 原則: 任意コマンド実行になる)。

### 5.3 ファイルロック

- **ロックは取らない** (advisory lock / `LockFileEx`)。他プロセスとの同時編集は FR-117 (外部変更検知) で対処する。理由: Windows の強制ロックは他ツールを壊す、NFS の `flock` は信頼できない、ロック解放漏れ (クラッシュ) が厄介。
- 例外: **スワップ/セッションファイルの排他** (同一ファイルを 2 つの evoeditor で開いた検出。スワップ v0.2、セッション v0.4。pid の生存確認自体も v0.2 から) は、ロックではなく「スワップヘッダに pid + hostname + 起動時刻を書き、生存確認 (Unix `kill(pid, 0)` / Windows `OpenProcess` + 起動時刻一致)」で行う。NFS 上のホームでは hostname 不一致を「別ホストで開いている可能性」として表示する。

### 5.4 ファイル同一性 (`FileId`)

| OS | 取得 | 安定性 |
|---|---|---|
| Unix (v0.2) | `(st_dev, st_ino)` | rename 置換で変わる (それが FR-117 の検知にも役立つ)。btrfs のサブボリュームで `st_dev` が揃わない例外あり |
| Windows | `GetFileInformationByHandleEx(FileIdInfo)` → `(VolumeSerialNumber, FileId 128bit)` (ReFS 対応)。Rust std の `MetadataExt::file_index` は Windows の unstable API のため自前で呼ぶ | NTFS では rename 後も同じ ID (ReplaceFileW でも tmp 側の ID になる) |

用途: (1) 同じファイルを二重に開く検出 (`evoeditor A.txt a.txt` on Windows)、(2) 保存直前の「別ファイルに置き換わっていないか」確認、(3) 監視イベントの自己抑制。

### 5.5 外部変更検知 (FR-117) の比較キー

02 §4.5 の `FileStamp { mtime, len, file_id, hash }` を比較キーとする (03 §9 の `known_stamp` の型)。本節はその計算規則を定める。`(mtime, len, FileId)` を基本とし、**mtime が等しく len も等しい** 場合に限って内容ハッシュ (16 MiB 以下) を比較する。加えて、**len と FileId が等しく mtime だけが違う** 場合も、16 MiB 以下なら内容ハッシュで照合し、一致すれば変更なしとする (OneDrive などの同期ソフト・ウイルス対策が内容を変えずに mtime だけを更新することがあり、そのたびに外部変更の確認を出さないため。16 MiB を超えるファイルは mtime の違いを変更として扱う)。一致したときは照合した側の stamp (新しい mtime) を以後の比較に使う。保存ジョブの中でハッシュを比べるのは **書き始める前の照合** (§4.1 手順 0b、§4.4 手順 1) だけで、置換の直前 (§4.1 手順 4b) と in-place の開いた直後 (§4.4 手順 3) は `(mtime, len, FileId)` だけで照合する (T を読み直す時間で競合の窓を広げないため)。理由: mtime 分解能は FAT32 2 秒、HFS+ 1 秒、ext4 は `CLOCK_REALTIME_COARSE` (数 ms) で同一 tick の連続更新が同じ mtime になる。**ハッシュは読込ジョブ内でストリーミング計算** する (保存ジョブで計算すると「開いてから最初の保存まで」ハッシュが無く、その間の外部変更を見逃すため)。保存ジョブでも書いた内容から計算して `SaveReport.new_stamp` に入れる。ハッシュ関数は **XXH3-64 (twox-hash クレート、MIT。純 Rust で §17.2 の「C 依存 0」に合う)** とし、値は 02 §4.5 の `ContentHash([u8; 8])` に入れる (v0.1 の既定)。S-09 のベンチでは速度だけを確かめる (16 MiB のハッシュが読込時間に対して無視できること)。`FsKind::Network` では mtime がクライアントキャッシュで遅延する (NFS `actimeo`) ことを UI の文言で伝える。

**`FsKind` の判定** (`FileMeta.fs_kind`。`metadata()` の中で T の親ディレクトリについて判定する):

| 条件 (上から順に評価) | `FsKind` |
|---|---|
| パスが UNC (`\\server\share\…`。`\\wsl$\…` / `\\wsl.localhost\…` を含む) または `\\?\UNC\…` | `Network` |
| ドライブ文字のパスで、`GetDriveTypeW(<ルート>)` が `DRIVE_REMOTE` (ネットワークドライブの割当て) | `Network` |
| (v0.2、Unix) `statfs` の `f_type` / `f_fstypename` が NFS・CIFS/SMB・9p・FUSE 系 | `Network` |
| 判定の API が失敗した | `Unknown` |
| それ以外 (`DRIVE_FIXED` / `DRIVE_REMOVABLE` / `DRIVE_RAMDISK` 等) | `Local` |

`Unknown` は `Local` と同じ保存手順を使う。`Network` は §12.2 の監視の縮退と上の文言にだけ使い、保存手順は変えない (失敗は §4.2 の「その他の環境」の判定に従う)。

### 5.6 一時ファイル・スワップ・ログの権限 (NFR-505)

**「所有者のみ」の定義** (本書の他の節はこの定義を参照する): Windows では DACL が **現在のユーザーの SID と `SYSTEM` (`SY`) の 2 つだけにフルアクセスを許し、継承を遮断 (保護) したもの** をいう。`Administrators` (`BA`) は含めない。Unix では所有者が自分で、ディレクトリは `0700`・ファイルは `0600` のものをいう。

| OS | ディレクトリ | ファイル |
|---|---|---|
| Unix (v0.2) | `recovery/`・`backup/`・`swap/` を含む `state_dir` 配下を `0700` で作成 (既存ならそのまま。`0700` でなければ警告ログ) | `0600` で作成 (`O_CREAT` の mode 引数。umask に依存しない) |
| Windows: 本文を含む場所 (`recovery/`・`backup/`・`swap/` (v0.2)) | 所有者のみの **明示的な DACL** で作る。SDDL の例: `O:<UserSID>D:P(A;OICI;FA;;;<UserSID>)(A;OICI;FA;;;SY)` (`O:` = 所有者を現在のユーザーに明示、`P` = 親からの継承を遮断、`OICI` = 中のファイルとディレクトリに継承させる)。所有者を明示するのは、管理者として起動した (昇格したトークンの) 場合に既定の所有者が `Administrators` になり、通常の権限で起動したときの検査に通らなくなるため。`state_dir` の場所 (`%LOCALAPPDATA%` 既定、`EVE_STATE_DIR` で指した任意の場所、§7.5 の代替の退避先 `%TEMP%\eve-recovery-<SID>\`) に依らず同じにする。既に存在する場合は所有者が **現在のユーザー、またはトークンの既定の所有者 (`TokenOwner`。昇格時は `Administrators`)** であることを確かめ、DACL が上の定義と違えば付け直す。所有者がどちらでもなければ使わない (§7.5 の「`state_dir` を使えないとき」に進む)。昇格時の作成・検査は S-09 / S-11 の確認項目 | ディレクトリの DACL を継承させる (結果として所有者のみ)。`write_atomic` の `private = true` は手順 1 の tmp を同じ DACL で作る |
| Windows: それ以外の `state_dir` 配下 (`log/`、`history`、`last_version.toml` 等) | 親 (`%LOCALAPPDATA%` は既定で当該ユーザー・SYSTEM・Administrators の ACL) を継承する。`EVE_STATE_DIR` で任意の場所を指した場合もその継承 ACL に従う (ユーザーの意図とみなす。本文を含まない) | 継承 |
| 文書と同じディレクトリに作る保存用一時ファイル | — | Windows は §4.1 手順 1 (所有者のみ。新規ファイルは継承)。Unix (v0.2) は 0600 → fchmod で元の mode に |

**作成は eve-platform の関数で行う**: `std::fs::create_dir` / `create_dir_all` で作ったディレクトリは親の ACL を継承し (`%LOCALAPPDATA%` では Administrators が読める、`EVE_STATE_DIR` では共有フォルダの ACL になり得る)、上の定義を満たさない。そのため `recovery/`・`backup/`・`swap/` と代替の退避先は、`create_dir_all_private` (02 §4.5) または緊急退避の書き手が呼ぶ同じ実装 (`CreateDirectoryW` に上の SDDL から作った `SECURITY_ATTRIBUTES` を渡す) でだけ作る。中のファイルは `std::fs` で作っても、ディレクトリの保護付き DACL を継承するので所有者のみになる。DACL は暗号化ではないので、EFS で暗号化されたファイルの内容も `state_dir` には平文で書かれる (既知の制約。§7.5 の「退避・バックアップの容量と保持」(4))。

---

## 6. 改行コード・文字コードの OS 既定

本節は **「新規文書」と「判定不能時」の既定値** のみを定める。判定・変換のアルゴリズムは 03、設定項目名は 07。

| 項目 | 既定 | 理由 | 設定での選択肢 (07 へ提案) |
|---|---|---|---|
| 新規文書の改行コード | **LF (全 OS)** (07 §5.2 `file.default_line_ending`。OQ-04) | P6 (同一体験)、NFR-405 (設定の OS 間共有)。Windows のメモ帳も 1809 以降 LF を扱える。Git の `autocrlf` 環境でも LF が無難 | `lf` / `crlf` / `cr` / `native` (Windows=CRLF、他=LF。選択肢として残す) |
| 新規文書の文字コード | **UTF-8、BOM なし (全 OS)** | Windows の一部ツール (古い版の Excel の CSV) は BOM を要求するが、BOM は多くのツールを壊す。必要なら保存時に指定 (FR-111) | `utf-8` / `utf-8-bom` / 任意ラベル |
| 判定不能時のフォールバック (03 の最終段) | **UTF-8 として開き、不正バイトがあれば NFR-304 の読み取り専用経路** | 黙って別の文字コードと解釈しない (P1) | `file.default_encoding` を 07 §5.2 の許容値のどれか (`"system"` を含む) にする |
| `"system"` の意味 (本層が提供するヒント) | Windows: `GetACP()` のコードページ (日本語環境 932 → `shift_jis`)。Unix (v0.2): `nl_langinfo(CODESET)` / `LC_ALL` → `LC_CTYPE` → `LANG` (`ja_JP.eucJP` → `euc-jp`、`C`/`POSIX`/未設定 → なし) | 03 §8.2 の推定の事後フィルタ (`file.detection_candidates` の既定を決める元) として利用。chardetng 1.0 に候補リストを渡す API は無いため (要確認)、TLD 形式ヒントへの写像 (`ja` → `jp`) だけを渡す。SSH 先で `LANG` 未設定のときは「なし」 | — |
| 端末への出力文字コード | 常に UTF-8 (TUI の表示)。Windows ではコンソールのコードページに依存させず、**UTF-16 に変換して `WriteConsoleW` で書く**。std の `Stdout` / `Stderr` はコンソールのときこの変換を内部で行うが、`CONOUT$` を `std::fs::File` で開いたハンドル (§9.1 のリダイレクト時の描画先) には行わない (UTF-8 のバイト列がそのままコードページで解釈されて化ける)。そのため描画先が `CONOUT$` のときは eve-tui が自前で UTF-8 → UTF-16 変換して `WriteConsoleW` を呼ぶ (S-07 で確認する)。端末の緊急復元 (§11.2) も同じく生ハンドルに `WriteConsoleW` で書く。`SetConsoleOutputCP(65001)` で済ませることはしない (§9.1) | | — |
| 外部プロセス出力のヒント (§13.5) | Windows: `GetOEMCP()` (cmd.exe 内部コマンドの既定)。Unix: 端末と同じ | | — |

### 6.1 UI の言語の OS ロケール検出 (英語 UI を同梱する版)

**v0.1 では使わない**。v0.1 の UI 文言は日本語 (`ja`) のみを同梱し、言語は `ja` に固定する ([OQ-21](10-decisions.md#oq-21-ui-文言の国際化)、確定 (ユーザー回答)、NFR-603)。MessageKey による文言の外部化は残す (07 §17)。英語 UI は v0.2 (確定 (ユーザー回答)) で、本節の OS ロケール検出は v0.2 で使う (OS 別の行のうち macOS / Linux は、その OS を出荷している場合に限る)。ユーザー追加の言語ファイルは v0.2 以降の候補 (提案中)。

**v0.2 の M8 で実装した** (`eve_platform::locale::ui_language_tag`。Windows は UI 言語、それ以外は下表の環境変数。環境変数の解釈は OS を呼ばない純粋な関数 `tag_from_posix_env` で試験している)。その版では、本層は 07 §17.1 の選択順の「OS ロケール」段に渡す **言語タグの候補** (例 `ja-JP`) を 1 つ返す。照合 (地域を落として同梱の言語に一致させる) と、`ui.locale` / `EVE_LANG` との優先順位は core (eve-config) が行う。検出は起動時に 1 回だけ、端末の初期化前に同期で行う (`--help` も同じ結果を使う)。

| OS | 検出の手順 | 注意 |
|---|---|---|
| Unix (Linux 等。出荷時) | `LC_ALL` → `LC_MESSAGES` → `LANG` の順で、最初の空でない値を使う (POSIX の優先順位)。`ja_JP.UTF-8` → `ja-JP` のように `_` を `-` に替え、`.` 以降 (符号化) と `@` 以降 (修飾子) を落とす。`C` / `POSIX` は「値なし」とする | `LANGUAGE` (GNU の優先言語リスト) は見ない (提案中。対応言語が 2 つのうちは効果が小さい)。SSH 先で `LANG` が未設定なら値なし (→ 次の段。どれからも決まらなければ既定の `ja`、07 §17.1) |
| macOS (出荷時) | Unix と同じ環境変数をまず見る (Terminal.app / iTerm2 は既定で `LANG` を設定する)。値なしのときは CoreFoundation の `CFLocaleCopyPreferredLanguages` の先頭 (システム設定の「優先する言語」) を使う | CoreFoundation の呼出しは eve-platform の FFI (NFR-704 の `unsafe` の範囲)。依存を足すなら NFR-706 の評価を通す (提案中)。起動時間への影響は BM-101a で確かめる |
| Windows | `GetUserDefaultUILanguage` の LANGID を `LCIDToLocaleName` で言語タグ (`ja-JP`) に変換する。**`LANG` / `LC_*` は見ない** | Git Bash・MSYS2 などは UI 言語と無関係に `LANG` を設定することがあるため。一時的に切り替えるときは `EVE_LANG` を使う (07 §17.1) |
| 共通 | 取得に失敗したら値なし (→ 既定の `ja`。v0.1 からの利用者の表示を変えない、07 §17.1)。エラーは情報ログだけで利用者には出さない | 言語の検出は端末との I/O の文字コード (§9.2 の「ロケール」、常に UTF-8) とは独立 |

---

## 7. ディレクトリ (設定 / 状態 / データ / キャッシュ / ログ)

### 7.1 決定: XDG 準拠 (macOS 含む) + Windows は Known Folders

v0.1 で実装するのは **Windows の列** (Known Folders に当たる `%APPDATA%` / `%LOCALAPPDATA%` と、`XDG_*` が明示されているときの尊重) である。Linux / macOS の列 (XDG の既定パス) は v0.2 (macOS / Linux 出荷時) の仕様。

| 用途 | Linux / macOS | Windows | 内容 |
|---|---|---|---|
| 設定 (`config_dir`) | `$XDG_CONFIG_HOME/evoeditor` → `~/.config/evoeditor` | `%APPDATA%\evoeditor` (Roaming) | `config.toml`, `keymap.toml`, `themes/`, `languages/` (07) |
| 状態 (`state_dir`) | `$XDG_STATE_HOME/evoeditor` → `~/.local/state/evoeditor` | `%LOCALAPPDATA%\evoeditor\state` | `log/`, `swap/`, `recovery/`, `sessions/`, `backup/`, `history` (検索履歴), `last_version.toml` (前回起動した版、v0.2、07 §18.8) |
| データ (`data_dir`) | `$XDG_DATA_HOME/evoeditor` → `~/.local/share/evoeditor` | `%LOCALAPPDATA%\evoeditor\data` | 追加文法・辞書など機械生成でない永続データ (v0.4+) |
| キャッシュ (`cache_dir`) | `$XDG_CACHE_HOME/evoeditor` → `~/.cache/evoeditor` | `%LOCALAPPDATA%\evoeditor\cache` | 再生成可能なもの (tree-sitter 解析キャッシュ等。v0.4+) |
| 実行時 (`runtime_dir`) | `$XDG_RUNTIME_DIR/evoeditor` (0700) → なければ `None` (`/tmp/eve-<uid>` は **作らない**: シンボリックリンク攻撃面) | 名前付きパイプ `\\.\pipe\eve-<sid>-...` (ディレクトリ不要) → `None` | sidecar / MCP サーバのソケット (v1.L-b 候補)、管理モードの pid ファイル (v1.L-b) |

- **Windows でも `XDG_*` 環境変数が明示設定されていれば優先する** (dotfile 管理ツール利用者向け。設定していなければ上表)。
- **macOS で `~/Library/Application Support` を使わない** 理由: U1 (dotfiles を全環境で共有) を優先。Helix / Neovim / Zellij 等の TUI ツールと同じ慣習。
- **`XDG_STATE_HOME`** は XDG Base Directory 0.8 (2021) で追加された比較的新しい項目。上表のすべての列は本層が自前で解決する (§7.2)。
- **環境変数による個別上書き**: `EVE_CONFIG_DIR`, `EVE_STATE_DIR`, `EVE_DATA_DIR`, `EVE_CACHE_DIR` (接頭辞 `EVE_` は維持。OQ-01 で確定)。CLI `--config <file>` (v0.2) は設定ファイルのみ差し替え。
- **作成タイミング**: `config_dir` は **ユーザーが設定を書くまで作らない**。`state_dir` とその配下 (`log/` を含む) は **起動経路では作らず、最初の書込 (ログ・退避・swap-lite・事前バックアップ・`last_version.toml`) の直前に、その書込に要るディレクトリだけを作る** (§7.3。`last_version.toml` (FR-518) は最初の描画の後に `io` ワーカーで書く、07 §18.8)。`swap/` / `recovery/` / `backup/` は必要時に §5.6 の DACL で作る。`--version` / `--help` 等の非対話の起動 (v0.2 以降は `--check-config` も) では `last_version.toml` を読み書きしない。
- **全ディレクトリは絶対パスに解決され `Paths` に入る**。`HOME` も `XDG_*` も解決不能 (環境変数ゼロのコンテナ等) なら: 設定なし・状態なしで起動し、ログは無効、スワップ無効 (警告をメッセージ行に 1 回表示)。起動を妨げない (FR-702)。この場合と `state_dir` を作れない場合 (読み取り専用のホーム等) の緊急退避先は §7.5 の「`state_dir` を使えないとき」。

### 7.2 ライブラリ比較

ディレクトリの解決は **本層が自前で行い、ライブラリ (etcetera・dirs 等) には依存しない**。etcetera の Windows の方式は `state_dir` を持たず (状態を置く場所を返さない)、§7.1 の `%LOCALAPPDATA%\evoeditor\state` / `data` / `cache` の分け方とも合わないため。§7.1 の規則は環境変数 (`EVE_*_DIR` → `XDG_*` → OS 既定) と既定パスの組合せだけで決まり、Windows の既定は `SHGetKnownFolderPath(FOLDERID_RoamingAppData / FOLDERID_LocalAppData)` (失敗時は `%APPDATA%` / `%LOCALAPPDATA%`)、Unix (v0.2) の既定は `$HOME` からの固定パスなので、自前の実装は小さい。`config_dir` / `state_dir` / `data_dir` / `cache_dir` のすべてを同じ関数で解決し、`Paths` (§2.2) に入れる。

### 7.3 ログ (02 §11.3 の確定事項)

- **実装**: core と bin は `tracing` の **マクロだけ** を使う。購読者は eve-host の **自前の最小実装** (ADR-0012): **ログのレベル (既定 `warn`。v0.1 は環境変数 `EVE_LOG` だけで変える。設定 `log.level` (07 §5.11) は v0.2) 以上** のイベントをログスレッドへ渡す。ログスレッドは最初に渡すイベントが来たときに遅延生成し、ファイルも最初のイベントで開く。`tracing-subscriber` / `tracing-appender` / `tracing-chrome` は `bench-trace` feature (09 §5) でのみ依存する (v0.1 の依存予算 §17.2)。
- **v0.1 の実装の範囲**: `tracing` の購読者とログスレッドはまだ無く、`eve-platform` の `Logger` が **呼び出しのスレッドで同期に** 書く (最初のイベントでディレクトリとファイルを作る・`EVE_LOG` のレベル・5 MiB × 3 世代のローテーションは仕様どおり)。使うのは今のところ bin の設定の診断だけ (全件を warn、「このバージョンでは未対応」の情報診断を info で書く)。core がイベントを出すようになる版で、`tracing` のマクロとログスレッドに替える。
- 場所: `state_dir/log/evoeditor.log` (+ `evoeditor.log.1`, `.2`)。ローテーションは 5 MiB × 3 世代の **定数** (設定キーにしない)。
- **起動経路で `state_dir` を作らない** (NFR-101): ディレクトリは最初の書込 (ログ・退避・swap-lite・事前バックアップ・`last_version.toml`) の直前に `create_dir_all_private` する (`state_dir/log` も最初のログの書込の直前)。作成の範囲は §7.1 (`config_dir` はユーザーが設定を書くまで作らない)。
- 権限: §5.6。
- 複数インスタンス同時起動: 同一ファイルに append (`O_APPEND` は行単位なら実用上安全。Windows は `FILE_APPEND_DATA`)。ローテーション競合は「サイズ超過を検出したプロセスが rename を試み、失敗したら無視」。他のインスタンスの改名・削除を妨げないよう、Windows ではログファイルを `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE` で開く (`FILE_SHARE_DELETE` が無いと改名が共有違反で失敗する)。**書き込む前に**、開いているハンドルのファイルの同一性 (ファイル ID。Unix は dev + inode) とログのパスが指すファイルの同一性を比べ、違えば (他のインスタンスがローテーションした) 開き直してから書く (改名後の古いファイルに書き続けない。書くのは warn 以上だけなので頻度は低い。02 §11.3)。
- クラッシュログ: `state_dir/log/crash-<UTC timestamp>-<pid>.log` (NFR-302、内容は 09 §10)。**最新 20 件** を保ち、超過分は次の起動時に古い順に削除する (02 §14.1 MR11)。
- レベルの変更は v0.1 では `EVE_LOG` 環境変数だけ。CLI `--log-level` と設定 `log.level` (07) は v0.2 (優先順は 07)。`EVE_LOG_STDERR=1` で TUI 外 (ヘッドレス/テスト) に stderr 出力。

### 7.4 ポータブルモード (Could, v0.4)

実行ファイルと同じディレクトリに `evoeditor.portable` (空ファイル) があるとき、全ディレクトリを `<exe_dir>/data/{config,state,cache}` に向ける。USB メモリ運用・Scoop の慣習に合う。`exe_dir` が書込不可なら無視して通常モード。macOS の .app バンドル内では無効。

### 7.5 スワップ・緊急退避ファイルの配置と命名

| 種別 | 場所 | 名前 | 中身の形式 |
|---|---|---|---|
| スワップ (FR-118、**swap-lite v0.2**) | `state_dir/swap/` | `<hash(canonical path)[..16]>-<pid>.swp` (ハッシュ関数は §5.5 と同じ XXH3-64。入力の正規形は下の「`hash(path)` の入力」)。無題は `untitled-<DocumentId>-<pid>.swp`。**`.swp` は 1 文書のディレクトリ**で、中に本文 `1-<basename>.r<Revision>.txt` と `manifest.toml` (`reason = "swap"`、`pid`、`started_at_*`) を置く。書込は、新しい Revision の名前の本文を一時ファイル + fsync + 名前の変更で作り、manifest を一時ファイル + fsync + rename で置き換えてから古い本文を消す (止まっても manifest が指す本文は常に完全)。ディレクトリは所有者のみの DACL (§5.6)。正常終了のときは Host が消し、異常終了・緊急退避を伴う終了では残す。**復旧の探索** (`FileSystem::list_swaps` / `read_swap` / `remove_swap`): `swap/*.swp` の manifest を読んで要約し (元のパス・Revision・本文の長さ・`sensitive`・pid)、書いたプロセスが動いているか (`OpenProcess` + 起動時刻の一致、§5.3。pid が再利用されて起動時刻が違えば別のプロセス) で `Alive` (他のインスタンスが開いている) と `Stale` (残骸) を分ける。自分のプロセスのものは返さない。未知の `format_version`・壊れた manifest・`written = false` は `readable = false` で返し、適用は提案せず場所だけを示す (移動も削除もしない)。`remove_swap` は `swap/` の直下の `.swp` だけを消す。復旧時はハッシュ前方一致で探し、pid の生存確認 (§5.3) で残骸を判定 | **緊急退避と同じ形式・同じシリアライズ関数** (書き手は `bg` ワーカーのジョブ (`JobKind::Swap`) が `FileSystem` Port を `SaveOptions { private: true, state_file: true, expected: Expect::Any, .. }` で使うもので、Port を通さない緊急退避の書き手とは別。02 §11.2.1): 本文は UTF-8 のまま全文 (改行非変換・PUA エスケープ保持)、形式情報は同じ `manifest.toml` (形式は 02 §11.2.1)。差分ジャーナル (03 §10.2) は必要が出たら v0.4 以降。対象は **64 MiB 以下** の dirty 文書 (`file.swap.max_bytes`)、最後の入力から 2 秒後、以後最短 10 秒間隔 (`file.swap.min_interval_ms`) に `bg` ワーカーで原子的に書く。64 MiB 超の文書は「スワップ対象外」をステータスに表示。秘密ファイルパターン (06 §6.5) の文書は既定で対象外 |
| 緊急退避 (NFR-302) | `state_dir/recovery/<UTC timestamp>-<pid>/` | `<n>-<basename>.txt` + `manifest.toml` (長い basename は §4.1 の「一時ファイル・退避名の長さ」の規則で切り詰める。manifest の更新は一時ファイル + 置換で行う、02 §11.2.1) | 本文の形式 (UTF-8 のまま・改行非変換・PUA エスケープ保持) と `manifest.toml` の形式 (キー・型・必須・形式の版・未知の版の扱い) は **[02 §11.2.1](02-architecture.md#1121-緊急退避の方式-正本adr-0023) が正本**。本書は配置・命名・権限だけを定める。元パスの非 UTF-8 部分の永続化は §3.5 の規則による。元の文字コードへの再エンコードはパニック中には行わない。復旧は 03 §10.3 (v0.1 は起動時の案内 + 手動、復旧 UI は v0.2) か、ユーザーが手で `.txt` を使う。swap-lite と復旧 UI (いずれも v0.2) は同じ形式・同じシリアライズ関数を使う (スワップの書き手は別。上の行)。**30 日を超えた退避** は起動時に一覧で知らせ、確認を得て削除する (v0.2。自動削除しない。02 §14.1 MR11) |
| 退避先に保存 (03 §9.7 (e) の「退避先に保存」) | `state_dir/recovery/rescue/<UTC timestamp>-<pid>/` | 緊急退避と同じ (`<n>-<basename>.txt` + `manifest.toml`) | 緊急退避と同じ形式・同じシリアライズ関数 (書き手は `bg` ワーカーのジョブ `JobKind::WriteRecovery`、02 §9.3 / 03 §9.7 (e))。ディレクトリは本層の `create_recovery_dir()` (§2.1) が作り、名前の UTC timestamp・pid と manifest の `created_at` は本層が決める (core は壁時計・pid を持たない、02 §4.4)。`state_dir` を使えないときは下の「`state_dir` を使えないとき」の代替の退避先に書く。緊急退避のディレクトリとは別にし、利用者の操作で書いたものと異常終了で書いたものを区別する。起動時の案内 (緊急退避が無いときは文言を分けた `msg.recovery.found_rescue`、02 §9.3)・30 日の規則 (v0.2) は緊急退避と同じ扱い |
| 保存中フラグ | `state_dir/recovery/inflight/<hash(path)>/` (`hash(path)` の入力は下記) | `<pid>-<seq>.toml` (`<seq>` はプロセス内で保存ジョブごとに 1 ずつ増やす 10 進の通し番号。同じプロセスの前の保存が残したフラグを上書きしない。§4.6)。保存ジョブ開始時 (一時ファイルを作る前) に T のパスと保存方式 (`Atomic` / `InPlace` / `NewFile`)、tmp / bak の実際の名前・pid・起動時刻 (`InflightRecord.started_at`。OS のプロセス作成時刻) を書き、in-place では `backup_recorded` と `backup_complete` も書く (`in_place_backup = Required` のときだけ事前バックアップの最終名を bak として記録して `backup_recorded = true`、`SkipConfirmed` では bak を記録せず `backup_recorded = false`。`backup_complete` は事前バックアップを最終名に変えたら `true` に書き換える (`Required` のときだけ)、§4.4 手順 2 / 4)。成功時に削除 (退避名・一時ファイルを消せなかったときは残す。§4.1 手順 7 / 8、§4.6 の「フラグを消す条件」。開くときの記録 × T の有無から `InterruptedSave` を作る規則は §4.6 の「開くときの写像」。pid が生きている記録 (他インスタンスが保存中) も v0.1 は生存確認をせずに同じ写像で扱い、掃除してもフラグしか消さない。生存確認 (§5.3) で除くのは v0.2 の起動時の走査。開くときの確認 `inflight_for()` は v0.1、起動時の走査 `list_inflight()` は v0.2)。原子的置換では書けないときは警告して保存を続け、in-place では書けなければ `BackupFailed` と同じ確認を経る (§4.4 手順 2)。フラグ自身は `write_atomic` を通さず、本層の下位関数で直接書く (フラグの書込でフラグを書かない。§4.1 の「状態ファイルの書込」) | TOML、版付き |
| 事前バックアップ (§4.4 の in-place) | `state_dir/backup/` (直下。この場所は in-place の事前バックアップ専用) | `<hash(path)[..16]>-<UTC timestamp>-<pid>.bak`。書込中は `<同じ名前>.tmp` で、fsync の後に名前を変える (§4.4 手順 4) | 元のバイト列をそのまま。in-place の成功後に、フラグを消してから削除 (§4.4 手順 9)。フラグの無いバックアップ (削除の前の中断・削除の失敗) は v0.2 の起動時の走査がこのディレクトリを列挙して見つける (§4.6) |
| FR-119 のバックアップ (v0.3) | `state_dir/backup/fr119/` | v0.3 で定める (03 §10.4、02 §4.5 の `BackupPolicy::KeepPrevious`) | 事前バックアップとは別のサブディレクトリにし、片付けの規則が互いを消さないようにする |

**退避ディレクトリの名前の形** (本書のこの段落が唯一の定義。上表・下表と 02 §4.5 / 03 §10.1 の `<UTC timestamp>-<pid>/` はこの形を指す): 緊急退避・「退避先に保存」のディレクトリ名は `<UTC timestamp>-<pid>[-<n>]` とする。`<UTC timestamp>` は作成時刻の UTC を `YYYYMMDDTHHMMSSZ` (ISO 8601 の基本形式、秒まで) で書いたもの、`<pid>` は書いたプロセスの pid (10 進)、`-<n>` は同じ名前のディレクトリが既に在ったときだけ付ける 2 以上の連番 (10 進。空いている最小の値) である。作る側 (eve-platform の Host 部品の緊急退避の書き手と `create_recovery_dir()`、§2.1) はこの形で作り、走査する側 (`list_recovery_dirs()`) はこの形に合う名前だけを退避として扱う。同じ時刻の退避の新旧は `-<n>` の大きい方を新しいとする。

**`recovery/` の走査と削除の規則**: `state_dir/recovery/` の直下にあるのは、緊急退避のディレクトリ (上の「退避ディレクトリの名前の形」)、`rescue/` (退避先に保存)、`inflight/` (保存中フラグ) の 3 種類だけである。起動時の退避の案内 (v0.1)・復旧 UI (v0.2)・30 日を超えた退避の一覧と削除 (v0.2) は、その形の緊急退避のディレクトリと `rescue/` の中の同じ形のディレクトリだけを対象にし、**`inflight/` は走査も削除もしない** (保存中フラグは §4.6 の規則 (`inflight_for` / `list_inflight`) だけが扱う。他インスタンスが保存中のフラグを消すと中断の痕跡を失う)。それ以外の名前のエントリは無視する (03 §10.1 の「復旧」)。v0.1 の起動時の案内の走査はこの規則の範囲を本層の `list_recovery_dirs()` (§2.1) が返し、`io` のジョブ (`RecoveryScan`、02 §9.3) が呼ぶ (メインスレッドでは列挙しない)。

**`hash(path)` の入力 (正規形)**: 保存中フラグ・スワップ・事前バックアップの名前に使う `hash(path)` は、同じファイルが別の綴りで指されても同じ値になるよう、次の正規形にしてから XXH3-64 (§5.5) を計算する。(1) 親ディレクトリを `canonicalize` し (存在しないファイル名は付け直す。§3.2 手順 6)、§3.3 の表示用パス (verbatim 接頭辞を外せる場合は外す) にする。(2) Windows では全体を大文字小文字を区別しない形に畳む (Unicode の単純大文字化。NTFS の既定に合わせる。§3.1 の「畳み込まない」の例外で、このキーにだけ使う)。Unix (v0.2) は畳まない。(3) 区切り文字は OS ネイティブ (`\` / `/`) に揃え、末尾の区切りを除く。(4) 入力のバイト列は §3.5 の永続化と同じ形 (Windows は UTF-16 単位列、Unix はバイト列) にする。**シンボリックリンク** の場合は、リンク自体のパスとリンク先 (最終リンク先 T) のパスの両方を正規形にし、フラグは T のキーで書き、照会 (`inflight_for` / 開くときの確認) は **両方のキー** で行う (リンク経由で開いた場合も、T を直接開いた場合も、中断した保存の痕跡を見つけるため)。**ダングリングリンク** (リンク先が存在しない。T が無い状態で開く場合を含む) では、リンク全体の `canonicalize` は失敗するので、リンクの読み取り (`readlink` 相当。Windows は `GetFinalPathNameByHandle` ではなくリパースポイントの読み取り) でリンク先のパスを得て (相対ならリンクの親ディレクトリから解決する)、その **親ディレクトリ** を `canonicalize` し、ファイル名を付け直して (1)〜(4) の正規形にする (リンク先が不在でも親ディレクトリが在れば失敗しない)。親ディレクトリも無い・リンクの連鎖が循環する場合はリンク先のキーを作らない。照会はこのリンク先のキーと **リンクのパス自体のキー** の両方で行う。
**状態ファイル共通の規約**: (a) スワップ・セッション・`trust.toml`・`history`・`last_version.toml` (v0.2)・監査ログ (v1.L)・manifest のすべてに **マジック (またはキー) とフォーマット版** を持たせ、未知の版は読まずに `<name>.unsupported-<ts>` へ退避して警告する (OQ-48)。**ダウングレード** (新しい版の evoeditor が書いた状態ファイルを古い版が読む) も未知の版として同じ扱いにする。ただし緊急退避とスワップは利用者のデータなので移動・削除せず、パスと「この退避は新しい版の evoeditor で書かれています。新しい版で開くか、`.txt` を直接開いてください」を案内するだけにする (この案内は manifest を読む v0.2 の復旧 UI のもの。v0.1 の起動時の案内は manifest を読まず、版に依らず場所だけを示す。manifest 側の規定は 02 §11.2.1。利用者向けの説明は §18.11)。(b) 複数インスタンスの同時書込に備え、状態ファイルは **一時ファイル + 原子的置換 (`write_atomic` に `SaveOptions { private: true, state_file: true, expected: Expect::Any, .. }`) で書き、最終書込勝ち** とする (ログは追記、§7.3)。保存中フラグは `state_file = true` で、照合は `Expect::Any` で省く (保存中フラグ自身は下位関数で直接書く。§4.1 の「状態ファイルの書込」)。(c) 権限は §5.6 (`create_dir_all_private`)。

**`state_dir` を使えないとき** (解決できない、作成できない、書込めない。退避先を変えること自体と終了時表示の要否は 02 §11.2.1、OQ-163 提案中。in-place の事前バックアップ (§4.4) は代替の場所に書かず、`BackupFailed` として確認に回す): 緊急退避と「退避先に保存」だけは代替の場所に書く (ログ・スワップは無効のまま、§7.1)。OS 別の実体は次のとおり。

| OS | 代替の退避先 | 作成・検査 |
|---|---|---|
| Unix (v0.2) | `$TMPDIR/eve-recovery-<uid>/` (`TMPDIR` 未設定なら `/tmp`) の下に、通常と同じ `<UTC timestamp>-<pid>/` | `mkdir(0700)`。既にあれば `lstat` で **シンボリックリンクでない・所有者が自分・mode が 0700** を確かめ、満たさなければ使わない (他ユーザーが先に作った場所に書かない) |
| Windows (v0.1) | `%TEMP%\eve-recovery-<SID>\` の下に同上 (「退避先に保存」は `rescue\<UTC timestamp>-<pid>\`) | §5.6 の「所有者のみ」の保護付き DACL (所有者 `O:<UserSID>` を明示) で作成 (eve-platform の関数で作る。§5.6)。既にあれば所有者が現在のユーザーまたはトークンの既定の所有者であることを確かめ (§5.6)、どちらでもなければ使わない。DACL が定義と違えば付け直す |

代替の場所も使えなければ退避は行えない。いずれの場合も、終了時の表示 (stderr) に退避先の絶対パス、または「退避できませんでした (理由)」を必ず出す。代替の場所は OS の一時ファイル掃除 (再起動時の `/tmp` 消去等) で消え得るので、表示に「早めに別の場所へ移してください」を添える。

**退避・バックアップの容量と保持 (提案、OQ-200)**: 退避 (緊急退避・退避先に保存) と事前バックアップは利用者の操作なしに `state_dir` に溜まり得るので、v0.1 から次を行う (上限の値は PO 判断点)。(1) **件数と容量の上限**: `recovery/` の退避ディレクトリ (`rescue/` を含む) と `backup/` 直下の事前バックアップについて、それぞれ件数 (提案: 50 件) と合計容量 (提案: 2 GiB) の上限を定数で持ち、起動時の走査 (`RecoveryScan`。`backup/` は v0.1 では件数と合計だけを数え、中身は読まない) で超えていれば場所と量を 1 回知らせる (今の `list_recovery_dirs` と `RecoveryFound` は件数と大きさを返さないので、採る場合は Port の `list_backup_stats` と `RecoveryFound.stats` を加える。02 §4.5 の提案)。**自動では消さない** (30 日の規則 (v0.2) と同じく確認を得て消す。削除の UI は v0.2)。(2) **書く前の空き容量の確認**: 「退避先に保存」(`WriteRecovery`) と事前バックアップ (§4.4 手順 4) は、書く前に `GetDiskFreeSpaceExW` の `lpFreeBytesAvailableToCaller` が書く量を上回ることを確かめ、足りなければ書かずに失敗として返す (事前バックアップは `BackupFailed`、§4.4 手順 4 の確認へ)。緊急退避は時間予算を使わないよう確認を省き、書けなければ次の文書へ進む (小さい文書から順に書くので、小さいものは残りやすい)。(3) **事前バックアップの `sensitive`**: 秘密ファイルパターン (07 §5.12.2) に当たる文書の直接上書きでは、保存中フラグに `sensitive = true` を記録し、`msg.save.in_place_failed`・`msg.open.interrupted_save` の案内でバックアップの削除を促す (緊急退避の manifest の `sensitive` と同じ扱い、02 §11.2.1)。成功時は手順 9 のとおり消すので残らない。(4) **EFS (NTFS の暗号化) の制約**: 元のファイルが EFS で暗号化されていても (`FILE_ATTRIBUTE_ENCRYPTED`)、緊急退避・退避先に保存・事前バックアップ・スワップ (v0.2) は `state_dir` に **平文で** 書かれる (`state_dir` 自体が暗号化されていなければ)。所有者のみの DACL (§5.6) で他の利用者からは守られるが、ディスクを取り出した読み取りには守られない。v0.1 の既知の制約としてリリースノートとヘルプに書く (原子的保存の `ReplaceFileW` は T の暗号化を保つので、利用者のファイル自体は暗号化されたまま。§5.1)。

---

## 8. クリップボード

**方式 (確定 (ユーザー回答)、OQ-111)**: クリップボードは OS 付属の手段で実装し、専用ライブラリは使わない。S-14 でこの方式が成立しなかった場合に限り、arboard を退路として採る。経路を Effect にすること (ADR-0021) も確定 (ユーザー回答)。

**v0.1 は Windows の Win32 API と内部レジスタだけ** (macOS / Linux を出荷しないため。確定 (ユーザー回答))。OSC 52 は v0.2 (`Effect` の変種と設定名 `clipboard.osc52` / `clipboard.osc52_max_bytes` は予約したまま)。macOS の `pbcopy` / `pbpaste`、Linux の root しか書けない場所にある `wl-copy` / `xclip` / `xsel`、WSL の `clip.exe` / `powershell.exe` (`command` provider) は v0.2 (macOS / Linux 出荷時) の仕様。v0.1 はマウスを捕捉しないので、端末の範囲選択でもコピーできる。

`[clipboard]` の設定キー (07 §5.6) の公開は v0.2。v0.1 は `provider = "auto"` 相当 (Win32 → 内部レジスタ) と上限 `system_max_bytes` の定数 (§8.4) で動く。

### 8.1 経路と優先順位

Host (eve-host + eve-tui) が `Effect::SetClipboard{content: ClipboardText, target}` を受け取ったときの処理 (02 §6.3):

```
0. content が ClipboardText::Handle(id) なら、Editor::take_clipboard(id) で ClipboardSource (Snapshot と範囲) を取り出し、
   **文字列化の前に範囲の総バイト数で** clipboard.system_max_bytes (既定 64 MiB、§8.4) と比べ、超えたら文字列化せずに
   内部レジスタのみとし、通知して終わる。超えなければワーカーで文字列化する (メインスレッドで String を作らない。02 §6.3 / §14.1 MR5)。
   ClipboardText::Inline(String) (64 KiB 未満) はそのまま次へ進む。
1. 内部レジスタへ必ず書く (core 側で既に済んでいる。ここは念のための順序の明示)
2. provider = 設定 clipboard.provider (auto | system | osc52 | command | none。既定 auto。07 §5.6)
3. auto の場合:
   a. システムクリップボード (§8.2 の OS 別実装。v0.1 は Win32 のみ) が利用可 → 書く。
      初期化 (外部コマンドの存在確認など) は **起動経路では行わず、最初の SetClipboard / RequestClipboard で
      ワーカー上で遅延実行** する (NFR-101)。Windows の user32 は crossterm が起動時から読み込んでいるので、遅延の対象ではない
   b. (v0.2) OSC 52 を書くのは Capabilities.osc52_write が真で、かつ次のいずれかのとき (07 §5.6):
      clipboard.osc52 = "always"、または clipboard.osc52 = "auto" で SSH 接続中 (SSH_TTY / SSH_CONNECTION) かシステムクリップボードが利用不可
      (SSH 越しでは a が「リモート側」に書いてしまい意味がないことがある → §8.2)
   c. どちらも不可 → 内部レジスタのみ。初回だけメッセージ「システムクリップボードは利用できません (理由)。エディタ内でのみコピー/貼り付けできます」
4. 失敗は非致命 (コピー自体は内部レジスタで成功)。
```

`Effect::RequestClipboard{token}`: システムクリップボード (§8.2) → `command` (v0.2) → なし、の順 (v0.1 は Win32 → なし)。**OSC 52 の読み取り (`?`) は使わない** (多くの端末が既定で拒否、または確認ダイアログを出し、応答待ちが UI を止める)。端末からの貼り付けはブラケットペースト (`InputEvent::Paste`) で届くため、ユーザーの Ctrl+V は「システムクリップボードが読めない環境では端末の貼り付けキー (Ctrl+Shift+V / Cmd+V) を使ってください」とヒントする。応答は `InputEvent::ClipboardContent{token, text}` で返す (取得は数百 ms かかることがあるため、02 §9.1 の **クリップボード取得用の使い捨てスレッド** (`RequestClipboard` ごとに作る。`io` / `bg` ワーカーではない) で実行し、結果を HostMsg で戻す。取得結果が 64 MiB を超えたら切り詰めず拒否して通知)。

**取得のタイムアウト (2 s)**: Windows の `GetClipboardData` は、クリップボードの所有者が遅延描画 (`WM_RENDERFORMAT`) に応答しないと戻らないことがある。取得は上のクリップボード取得用の使い捨てスレッドで行い (§8.2 の所有者ウィンドウのスレッドとは別)、Host は要求から **2 s** で応答が無ければ、その要求を内部レジスタからの貼り付けに切り替えて `ClipboardContent` を返し、「システムクリップボードの取得がタイムアウトしました。エディタ内のコピーを貼り付けました」を通知する。取得中のスレッドは **放棄する** (終了を待たない・中断もしない。後から結果が届いても token が古いので捨てる)。放棄したスレッドが戻るまでの間の次の要求は、新しいスレッドを立てず、直ちに内部レジスタで応じる (スレッドを積み増さない)。

### 8.2 OS 別の実装詳細

v0.1 で実装するのは下の表の **Windows の 2 行** で、ほかの行は v0.2 (macOS / Linux 出荷時、OSC 52) の仕様。

**arboard を使わない** (確定 (ユーザー回答)、OQ-111)。実装は OS ごとの最小限とし (NFR-207 の依存数の予算)、arboard (または同等) は **S-14 で OS 付属の方式が成立しなかった場合の退路** とする。

| 環境 (版) | 実装 | 既知の問題 | 対処 |
|---|---|---|---|
| Windows (v0.1) | `windows-sys` で Win32 API を直接呼ぶ (`OpenClipboard` / `GetClipboardData` / `SetClipboardData` の `CF_UNICODETEXT`)。`OpenClipboard` には **クリップボード用のメッセージ専用ウィンドウ** (`HWND_MESSAGE`) のハンドルを渡し、所有者にする (`OpenClipboard(NULL)` の後の `EmptyClipboard` では所有者が無くなり、文書上は `SetClipboardData` が失敗し得る)。このウィンドウは最初の `SetClipboard` / `RequestClipboard` で専用スレッドに作り、そのスレッドでメッセージループを回す (他のアプリが `EmptyClipboard` したときに所有者へ送られる `WM_DESTROYCLIPBOARD` を処理するため。回さないと相手のアプリを待たせる)。所有者にはブロードキャストを受ける必要が無いのでメッセージ専用で足りる。§11.2 のセッション終了用の隠しウィンドウ (v0.2) とは別のもので、v0.2 で両者を 1 つにまとめるかは実装時に決める | `OpenClipboard` 競合 (リモートデスクトップ、クリップボード履歴) | 短い再試行 (10 ms × 5)。失敗はメッセージ |
| Windows: 改行 (v0.1) | テキストは CF_UNICODETEXT。**貼り付け時に CRLF が来る**。コピー時は文書の改行コードのまま入れる | 文書が LF でも他アプリから CRLF が来る | 正規化は core の貼り付けの入口 1 か所で行う (`\r\n`・単独 `\r`・単独 `\n` を `default_eol` に。03 §8.4)。Host 側の前処理は不要 |
| macOS (v0.2) | `/usr/bin/pbcopy` / `/usr/bin/pbpaste` を **絶対パス** で起動 (§8.3 の `command` provider と同じ経路。承認不要) | Terminal.app は OSC 52 非対応だが pbcopy で足りる。pbcopy / pbpaste はロケールで文字コードを決めるので、`LANG` が無い環境 (SSH・launchd から起動) では日本語が化ける (S-14 で再現を確認) | 子プロセスの環境に `LANG=en_US.UTF-8` と `__CF_USER_TEXT_ENCODING=0x<uid>:0x8000100:0x0` (UTF-8) を明示して渡す。タイムアウト 2 s |
| Linux (X11 / Wayland) (v0.2) | **OSC 52 + 内部レジスタ** を基本とし、加えて **root しか書けないディレクトリ (`/usr/bin`、`/bin`) に限って** `wl-copy` / `wl-paste` (Wayland)、`xclip` / `xsel` (X11) を検出して `command` provider として使う (§8.3)。PATH は見ない | `wl-copy` は GNOME (Mutter) でもフォーカスがあれば書ける。X11 の所有権モデル (コピー元プロセスが終了すると内容が消える) は `xclip` / `xsel` がデーモン化して保持する | コマンドが無ければ OSC 52 のみ。初回にヒント「`wl-clipboard` または `xclip` を入れるとシステムクリップボードが使えます」 |
| Linux ヘッドレス (`DISPLAY`/`WAYLAND_DISPLAY` 未設定) (v0.2) | コマンド検出をしない | — | OSC 52 / 内部レジスタ |
| WSL (v0.2) | WSLg があれば上記 Linux と同じ。無ければヘッドレス | WSL1 / WSLg 無効 | `command` provider の既定を WSL 検出時に、copy = `/mnt/c/Windows/System32/clip.exe` (本文は **UTF-16LE (BOM 付き)** に変換して標準入力へ渡す。UTF-8 のまま渡すと日本語が化ける)、paste = `/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe -NoProfile -Command "[Console]::OutputEncoding=[Text.Encoding]::UTF8; Get-Clipboard"` (**絶対パス**。出力の文字コードを UTF-8 に固定する) にする (§15.1。キーと既定値の正本は 07 §5.6。文字化けの有無は S-14 で確認)。または OSC 52 (Windows Terminal は対応) |
| SSH 越し (v0.2) | リモートに X11 があっても外部コマンドは「リモートの」クリップボードに書く (無意味) | `SSH_CONNECTION`/`SSH_TTY` あり | `SSH_TTY` が設定されていれば **既定で OSC 52 のみ** とし、外部コマンドは `clipboard.provider = system` の明示時のみ |
| tmux / screen (v0.2) | OSC 52 は tmux の `set-clipboard on` + 外側端末の対応が必要。screen は OSC 52 を透過しない | 伝わらないことが多い | `TMUX` 環境変数検出時は tmux の DCS 透過ラッパ (`\ePtmux;\e...\e\\`) で送る (04 の出力側で実装。本層は方針のみ)。失敗検知はできないので「送った」ことにする |

**退路 (arboard)**: S-14 で OS 付属の方式が成立しなかった場合、arboard (x11rb / wayland-data-control feature を必要最小限で) を `clipboard.provider = system` の実装として採る (OQ-111)。採る場合は依存予算 (§17.2) を再評価し (Windows のクリップボードの所有者は上の表のメッセージ専用ウィンドウの考え方のまま)、X11 の終了時所有権待機 (`clipboard.x11_exit_wait_ms`、OQ-79) もこのときだけ意味を持つ。

### 8.3 `command` provider (設定で明示、既知コマンドの検出、または WSL 既定)

**v0.2 (macOS / Linux 出荷時) の仕様**。v0.1 (Windows のみ) には `command` provider が無い。

`clipboard.command.copy = ["/mnt/c/Windows/System32/clip.exe"]` / `clipboard.command.paste = ["/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe", "-NoProfile", "-Command", "[Console]::OutputEncoding=[Text.Encoding]::UTF8; Get-Clipboard"]` の形 (argv 配列。シェル解釈なし。キー名と既定値の正本は 07 §5.6 で、**設定キーの公開は v0.2**。公開前は WSL 既定と既知コマンドの検出だけを定数で行う)。`ProcessRunner` (v0.4) が無い間は eve-host が `std::process::Command` で直接起動する (stdin に本文、タイムアウト 2 s、ワーカー上)。**クリップボード用コマンドは「設定で明示したもの」または「root しか書けないディレクトリで検出した既知のもの」なので毎回の承認は不要** とする (P2 原則の例外。設定ファイルは信頼入力、[02 §15](02-architecture.md#15-セキュリティ境界))。**PATH 上の自動検出はしない** (PATH 汚染による任意コード実行面)。検出対象は `/usr/bin` と `/bin` の `wl-copy` / `wl-paste` / `xclip` / `xsel`、macOS の `/usr/bin/pbcopy` / `pbpaste` に限り、ユーザーが `clipboard.command.*` を書いたらそれを優先する。

- **copy 側の標準出力・標準エラー**: `xclip` / `xsel` (と一部の `wl-copy`) は内容を保持するためにデーモンとして残り、親から受け継いだ stdout / stderr を握ったままにする。パイプにしていると読み手が EOF を待ち続け、毎回 2 s のタイムアウトになる。copy 側は stdout / stderr を `Stdio::null()` にし、待つのは起動した子プロセスの終了 (`wait`) だけにする。paste 側は stdout をパイプで読む (デーモン化しない)。
- **文字コード**: 子プロセスへの入出力は UTF-8 とし、例外は WSL の `clip.exe` (UTF-16LE) と macOS の `LANG` の明示 (§8.2)。

### 8.4 サイズ上限

上限値のキー名と既定値の正本は 07 §5.6 (`clipboard.osc52_max_bytes` / `clipboard.system_max_bytes`。v0.1 は `system_max_bytes` だけを定数で使い、**公開は v0.2**)。

- **OSC 52 (v0.2)**: base64 で 100 KiB を超えると端末が黙って捨てる例がある (tmux 等)。`clipboard.osc52_max_bytes` 既定 **100,000 バイト** (07 §5.6。安全側)。超えたら OSC 52 は送らず、システムクリップボードのみ。送れなかった旨をメッセージ。
- **システムクリップボード**: `clipboard.system_max_bytes` 既定 **64 MiB** (02 §14.1 MR5、07 §5.6)。文字列化の前に (`ClipboardText::Handle` から取り出した `ClipboardSource` の) 範囲の総バイト数で判定し、超えたら内部レジスタのみ + 通知 (100 MB の全選択コピーでメモリと時間を倍使わないため)。内部レジスタには上限を設けない (Snapshot 参照で保持するため追加コストが無い)。

---

## 9. 端末の OS 固有事項と互換性マトリクス

> 端末能力の **判定方法と縮退の実装** は 04-tui-design の責務。本節は「OS/端末ごとに何が起きるか」の事実と、OS 固有の初期化・復元手順のみ。

### 9.1 Windows コンソールの初期化と復元 (eve-tui)

**v0.1 で実装する** (Windows Terminal と conhost)。

| 項目 | 内容 |
|---|---|
| VT モード (出力) | `SetConsoleMode(hOut, ENABLE_VIRTUAL_TERMINAL_PROCESSING \| DISABLE_NEWLINE_AUTO_RETURN)`。VT を使うのは出力だけで、入力は VT にしない (下の「入力モード」)。crossterm がこれを行うが、**元のモード値を保存し終了・パニック・サスペンド不可の各経路で復元** する責任は eve-tui |
| 入力モード | `ENABLE_PROCESSED_INPUT` を **落とす** (Ctrl+C をキーとして受ける、FR-609)、`ENABLE_MOUSE_INPUT` は **v0.1 では立てない** (マウスを取得しない。`MOUSE_EVENT` が届いても、`FOCUS_EVENT` と同じく捨てる。04 §6.2)、`ENABLE_QUICK_EDIT_MODE` はマウスを取得するとき (v0.2) だけ落とす (v0.1 はマウスを取得しないので変更しない)、`ENABLE_WINDOW_INPUT` を立てる (リサイズ)、`ENABLE_LINE_INPUT` / `ENABLE_ECHO_INPUT` を落とす、**`ENABLE_VIRTUAL_TERMINAL_INPUT` は立てない**。**入力経路は自前のループ** (確定 (ユーザー回答)、OQ-33、04 §4.8 / §6.6): 入力スレッドが `WaitForSingleObject(hIn, …)` で待ち、`GetNumberOfConsoleInputEvents` の件数ぶん `ReadConsoleInputW` で読む (crossterm の `event::read()` は使わない。VT 入力 + 自前の VT パーサ・`win32-input-mode` は採らない)。修飾キー情報は `dwControlKeyState` から得る。ブラケットペーストは届かないので、貼り付けはバースト検出 (1 回の読取の件数と、`WaitForSingleObject(hIn, T)` による無入力の判定) で `Paste` に合成する |
| 失敗時 | `SetConsoleMode` が VT を拒否 (VT に対応しない古い OS のコンソール、または `ENABLE_VIRTUAL_TERMINAL_PROCESSING` 非対応のリダイレクト先) → 起動エラー「この端末は非対応です。Windows Terminal をご利用ください」(`stderr.unsupported_console`、04 §15.2。OQ-15)。終了コード **5** (07 §18.4) |
| コードページ | 変更しない (`SetConsoleOutputCP(65001)` は他プロセスへ影響し、復元漏れが事故になる)。§6 のとおり std が UTF-16 変換する |
| 端末タイトル | 起動時に `GetConsoleTitleW` で元のタイトルを退避し、終了・パニック・`Fatal` の `restore()` で `SetConsoleTitleW` により戻す (ConPTY 経由で Windows Terminal のタブ名にも伝わる)。XTWINOPS の `CSI 22;0 t` / `CSI 23;0 t` は Windows のコンソール VT 一覧に無く Windows Terminal も実装していない認識なので使わない (S-07 で確認)。**緊急復元 (固定のバイト列、§11.2) ではタイトルを戻せない** (既知の制約。04 §14.3) |
| 標準入出力のリダイレクト | stdout がコンソールでない (`GetFileType != FILE_TYPE_CHAR`) → 07 §18.5 の方式: **描画先は常にコンソール (`CONOUT$` を開く)** とし、stdout は `--stdout` (OQ-99、v0.4 Could) のためだけに使う。`CONOUT$` への描画は eve-tui が UTF-16 に変換して `WriteConsoleW` を自前で呼ぶ (std の `File` は変換しない。§6、S-07 で確認)。コンソールが得られない (`CONOUT$` を開けない、サービス・cron 相当) 場合は「出力先が端末ではありません」(`stderr.not_a_terminal`、04 §15.2) で終了コード 5。stdin がパイプ (`evoeditor -`、FR-120、v0.3) → 読み終えたら `CONIN$` を開き直して入力に使う |
| 端末種別の推定 (ログ用・ヒント用。分岐は 04 の能力判定で。conhost と推定したときは装飾記号・枠線を ASCII に縮退する、04 §3.7) | `WT_SESSION` (Windows Terminal。**子プロセスに引き継がれるので、Windows Terminal から起動した conhost や VS Code の統合端末でも立ち得る**。誤判定の扱いと補助判定は 04 §5.2a)、`ConEmuANSI`、`TERM_PROGRAM` (VS Code 統合端末、mintty)、`MSYSTEM` (Git Bash / MSYS2 の mintty は ConPTY ではなく winpty/pty 経由)、`TERM` 未設定 = conhost の可能性 |
| ConPTY の癖 | (1) 出力をいったん自前バッファで解釈し再描画するため **バイト列はそのまま外へ出ない** (同期出力 DEC 2026 は Windows Terminal 1.24 の安定版 (2026-03) とその後のバックポートから対応、conhost 単体は不明。P0 検証)、(2) Windows 10 の 1809〜2004 の ConPTY は代替スクリーンのスクロールバック・リサイズに不具合がある (v0.1 の対象の Windows 11 には当たらない。Windows 10 を対象に加えるときの確認事項、§1.1)、(3) `WINDOW_BUFFER_SIZE_EVENT` が **バッファサイズ** を返し **ウィンドウサイズ** と異なる (conhost でバッファが大きい設定)。eve-tui は `GetConsoleScreenBufferInfo().srWindow` からウィンドウサイズを再計算する |
| 入力の癖 (P0 S-07 で確定) | Windows Terminal・conhost とも `ReadConsoleInputW` の `KEY_EVENT_RECORD` で届き、Press / Release (`bKeyDown`) と修飾 (`dwControlKeyState`) を区別できる見込み (要確認。`Ctrl+H` / `Ctrl+I` / `Ctrl+M` / `Ctrl+[` も `Backspace` / `Tab` / `Enter` / `Esc` と区別されて届く見込み。04 §6.2 の「入力経路ごとの写像」)。AltGr は `RIGHT_ALT_PRESSED` + `LEFT_CTRL_PRESSED` と印字可能な `UnicodeChar` で届く。**半角/全角キー・変換キー** は IME が消費しキーイベントが来ない (正常)。IME 確定の Enter が漏れないかも S-07 / S-08 で確認 |
| ブラケットペースト | Windows Terminal・conhost とも **使わない** (VT 入力を使わないため。貼り付けは高速なキー連打として届く) → 04 §6.6 のバースト検出で `Paste` に合体し、**バースト中の制御文字はコマンドとして解釈しない**。バーストの閾値 N・T は S-07 で確定。CRLF の貼り付けは `\r` のみ届くことがある |
| マウス (v0.2) | VT マウスシーケンスではなく `MOUSE_EVENT` 構造体で届く (eve-tui の自前ループが正規化)。conhost でもホイール・クリック可。v0.1 はマウスを取得しない |
| フォーカスイベント・kitty キーボードプロトコル (v0.2) | フォーカスイベント (`CSI ? 1004`) の有効化と kitty キーボードプロトコルの push / pop は v0.2 (最初の利用者 = フォーカス復帰時の外部変更確認、および拡張キー)。v0.1 はどちらも送らない (Unix も同じ) |
| 復元 | 代替スクリーン解除 → カーソル表示 → ブラケットペースト/マウス/拡張キー解除 → `SetConsoleMode` を保存値へ。**順序はこの逆順ではなく「どれかが失敗しても次を試す」ベストエフォート** |

### 9.2 Unix 端末の初期化と復元

**v0.2 (macOS / Linux 出荷時) の仕様**。v0.1 では実装しない。

| 項目 | 内容 |
|---|---|
| raw モード | `tcgetattr` で保存 → `cfmakeraw` 相当 (ISIG/IXON/ICANON/ECHO/IEXTEN/OPOST を落とす)。ISIG を落とすため Ctrl+C/Ctrl+Z/Ctrl+\ はシグナルを生成せずキーとして届く (FR-609)。IXON を落とすため Ctrl+S/Ctrl+Q のフロー制御が無効 (ただし **tmux 等の外側がフロー制御を奪う** ことは防げない → 代替キーは 04) |
| 復元 | `tcsetattr(TCSAFLUSH)` ではなく `TCSANOW` (未読入力を捨てない方が安全) → 代替スクリーン解除 → カーソル表示。`/dev/tty` が閉じていても (SIGHUP 後) 失敗を無視して続行 |
| `/dev/tty` | stdin がパイプのとき (`evoeditor -`、FR-120、v0.3) は `/dev/tty` を開いて入力に使う。`/dev/tty` が無い (nohup, cron) → 起動エラー |
| `TERM` | 未設定・`dumb` → 起動エラー (またはカラー無しの最小モード。04 で判断)。`TERM=xterm-256color` を SSH 先で `xterm-kitty` 等の未知 terminfo に書き換えられている問題は、terminfo を **使わない** 設計 (crossterm は terminfo 非依存) なので影響が小さい |
| ロケール | `LANG`/`LC_CTYPE` が UTF-8 でなくても **端末との I/O は UTF-8 固定**。非 UTF-8 ロケールの端末は非対応 (起動時に警告 1 回) |
| macOS 固有 | Terminal.app: True Color は macOS 26 から対応 (それより前は 256 色)、OSC 52 非対応、Option を Meta にするには設定が必要 (日本語ユーザーは Option を記号入力に使うため、**Alt 系キーを既定キーマップの Must 機能に割り当てない** ことを 04 に要請)、ブラケットペースト対応。iTerm2: OSC 52 は "Allow clipboard access" を有効化した場合のみ |
| Linux 固有 | VTE 系 (GNOME Terminal, Tilix, …): kitty キーボードプロトコル非対応、**OSC 52 非対応** (VTE は意図的に実装していない。GNOME の課題 vte#2495。黙って無視される)。そのため GNOME Terminal では、SSH 越しにシステムのクリップボードへコピーする手段が無い (tmux の `set-clipboard` 経由で外側の端末に渡すか、他の端末を使う。Linux 出荷時の既知の制約)。同期出力 (DEC 2026) の対応版は要確認。True Color 可。kitty: 全部対応。Alacritty: OSC 52 対応、IME は版依存 (§10)。Linux コンソール (tty、X なし): 256 色なし・日本語フォントなし → 非対象 (動けばよい) |

### 9.3 tmux / screen / mosh

**v0.2 (macOS / Linux 出荷時) の仕様**。

| 多重化 | 影響 |
|---|---|
| tmux | `TERM=tmux-256color`/`screen-256color`。True Color は `terminal-overrides`/`terminal-features` 設定次第 (`COLORTERM` は伝わらない) → 04 の判定は「tmux 内では設定 `theme.color_mode = "auto"` (07 §5.4) のとき 256 色に縮退、ユーザーが `"truecolor"` で上書き可」。OSC 52 は `set-clipboard on`。拡張キーは tmux 3.3+ の `extended-keys` (CSI u / modifyOtherKeys の形式) で、kitty キーボードプロトコルの push (`CSI > u`) は透過しない (要確認。v0.2 の拡張キーの着手時に確かめる)。同期出力 2026 は 3.4+ で透過。フォーカスイベント (v0.2) は `focus-events on` |
| screen | OSC 52・True Color・拡張キー・同期出力のいずれも透過しない or 不安定。スモーク (起動・入力・保存・終了) のみ |
| mosh | 代替スクリーンの一部シーケンス非対応、OSC 52 非対応、ブラケットペーストは透過。遅延 (予測表示) で描画順が乱れて見える。非対象だが動くはず |

### 9.4 互換性マトリクス (リリース前に手動確認。記入は 09 §3.8)

> **v0.1 の対象端末は Windows Terminal と conhost の 2 つ** (Windows 11。確定 (ユーザー回答)、OQ-15、01 NFR-402)。**v0.1 で記入するのはこの 2 行だけ** で、ほかの行 (macOS / Linux の端末・tmux・screen) は v0.2 (macOS / Linux 出荷時) の記入対象として残す。VS Code 統合端末・mintty (Windows) も v0.1 では記入しない。**conhost は機能が Must、描画品質 (絵文字・幅の危険な書記素・同期出力なしのちらつき) は best effort**。下の表の「Must (v0.2)」は macOS / Linux を出荷する v0.2 の Must 端末 (Terminal.app・iTerm2・GNOME Terminal・tmux。確定 (ユーザー回答))、「スモーク (出荷時)」はその版での扱いの案 (提案中) で、スモークは起動・入力・保存・終了ができることだけを見る。マトリクスの全項目の記入は v0.4 (Must 端末を増やすことではない)。性能計測の基準機 (OQ-23。Windows の開発 PC 1 台で確定、01 §3.0) は本表とは別に扱う。**P0 前の推測を Must 要件の根拠にしない**: 執筆時点で実機確認していないセルは `?` とし、S-07 / S-08 / S-11 で確定するまで FR-605 / 606 / 609 の受入条件は「暫定」とする。「既定キーの横取り」列は要約で、**正本は 04 §7.3a の端末の横取り表** (機械可読。eve-tui の定数表と FR-606 の到達性検査の入力。FR-608 (v0.4) の警告データの源泉) である。macOS / Linux の端末の横取りは想定 (未確認) で、出荷する版の到達性検査で横取りとして扱う (v0.1 の到達性検査は Windows Terminal と conhost の 2 端末だけ)。

| 端末 \ 項目 | OS | True Color | 拡張キー (Ctrl+Shift 区別) | ブラケットペースト | OSC 52 書込 | 同期出力 2026 | マウス (v0.2) | IME 候補位置追従 | CTRL_CLOSE / SIGHUP 到達 | 既定キーの横取り (正本は 04 §7.3a) | 備考 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Windows Terminal | Win | ○ | ? (`ReadConsoleInputW` の `dwControlKeyState` で区別できる見込み。S-07) | × (使わない。バースト検出で補う、04 §6.6) | ○ | △ (1.24 以降。版に依存。S-07) | ○ | ○ (ハードウェアカーソル追従) | ? (タブ閉じで CTRL_CLOSE が届くか。S-11。ログオフ・シャットダウンは v0.2) | 要約 (正本は 04 §7.3a): `Ctrl+Shift+P/T/W/N/D/F/A/M/Space`、`Ctrl+Shift+1〜9`、`Ctrl+Alt+1〜9`、`Ctrl+,`、`Ctrl+Shift+↑↓/Home/End/PgUp/PgDn`、`Ctrl+Tab`、`Ctrl+Shift+Tab`、`Ctrl+=/-/0`、`Alt+Enter`、`F11`、`Alt+←→↑↓` (ペインのフォーカス移動)、`Alt+Shift+矢印` (ペインの大きさ)、`Alt+Shift+D/-/+` (ペインの分割)。`Ctrl+V`・`Ctrl+Shift+V`・`Shift+Insert` は端末の貼り付けになる (貼り付けた内容の個別キーとして届き、バースト検出で `Paste` に合成する。S-07)。`Ctrl+C`・`Ctrl+Shift+C`・`Ctrl+Insert` は端末に選択があるとき端末のコピーになる | **Must (v0.1)** |
| conhost (VT) | Win 11 (Win 10 は v0.2 以降の候補) | ○ | ? (`ReadConsoleInputW` の `dwControlKeyState` で区別できる見込み。S-07) | × (バースト検出で補う) | × | ? | ○ | ○ (版依存。S-08) | ? (S-11) | 少ない | **v0.1: 機能 Must・描画 best effort** (OQ-15) |
| VS Code 統合端末 | Win/mac/Linux | ○ | △ | ○ | ○ | ? | ○ | △ | ? | VS Code 自身のキーバインド多数 (`Ctrl+P`、`Ctrl+Shift+P` 等。`terminal.integrated.commandsToSkipShell` の案内) | Should (v0.1 は記入しない) |
| mintty (Git Bash) | Win | ○ | △ | ○ | ○ | ? | ○ | △ | ? | 少ない | Could (v0.1 は記入しない)。winpty 経由では非対応 |
| Terminal.app | mac | △ (macOS 26 から 24 ビット。それより前は 256。S-20、未確認) | × | ○ | × | × | ○ | ○ | ○ (SIGHUP) | `Cmd+*`、`Ctrl+←/→` (Mission Control)、`Home/End/PgUp/PgDn` (スクロールバック。`Shift+Home/End/PgUp/PgDn` は修飾なしのキーとして届く)、Option は既定で Meta でない | Must (v0.2) |
| iTerm2 | mac | ○ | △ (CSI u オプション) | ○ | ○ (設定 "Allow clipboard access") | ○ | ○ | ○ | ○ | Option の扱いはプロファイル次第 | Must (v0.2) |
| kitty | mac/Linux | ○ | ○ (kitty protocol) | ○ | ○ (確認あり) | ○ | ○ | ○ (Linux は `GLFW_IM_MODULE=ibus` 要) | ○ | `Ctrl+Shift+*` 系は端末ショートカット | スモーク (出荷時)。全項目の記入は v0.4 |
| WezTerm | 全 | ○ | ○ (kitty protocol、設定要の版あり) | ○ | ○ | ○ | ○ | ○ | ○ | 少ない | スモーク (出荷時)。全項目の記入は v0.4 |
| Alacritty | 全 | ○ | △ (0.13+) | ○ | ○ | ○ (0.13+) | ○ | △ (版依存) | ○ | 少ない | スモーク (出荷時)。全項目の記入は v0.4 |
| GNOME Terminal (VTE) | Linux | ○ | × | ○ | × (VTE は非対応) | ? (対応版は要確認) | ○ | ○ | ○ | `Ctrl+Shift+C/V/T/W/N/Q`、`Ctrl+PgUp/PgDn`、`F1`、`F10`、`F11`、`Shift+PgUp/PgDn` (スクロールバックに使われる。到達性検査では横取りとして扱う) | Must (v0.2) |
| tmux (内側) | 全 | △ (`terminal-features` 設定) | △ (3.3+ `extended-keys`。CSI u 形式で、kitty プロトコルは透過しない) | ○ | △ (`set-clipboard on`) | △ (3.4+) | ○ | 外側次第 | ○ (tmux kill-session で SIGHUP) | プレフィックス `Ctrl+B`。推奨設定: `escape-time 10`、`extended-keys on`、`set-clipboard on`、`focus-events on` | Must (v0.2) |
| screen (内側) | Unix | × | × | △ | × | × | △ | 外側次第 | ○ | プレフィックス `Ctrl+A` | スモーク (出荷時。動作のみ) |
| Linux console (tty) | Linux | × | × | × | × | × | × (gpm) | × | ○ | — | 非対象 |

(○ = 対応、△ = 条件付き/版依存、× = 非対応、? = 08 §3 のスパイクで検証)

---

## 10. IME の OS 別注意

TUI では IME は端末が処理し、evoeditor は **ハードウェアカーソルを主カーソル位置に置く** (`Frame.primary_cursor`、FR-605) ことだけが責務 ([02 §7.1](02-architecture.md#71-inputevent-eve-view))。v0.1 の対象は Windows Terminal と conhost (MS-IME と Google 日本語入力。S-08) で、macOS / Linux・SSH 越し・WSL の行は v0.2 (macOS / Linux 出荷時) の仕様。OS/端末ごとの注意:

| 環境 | 挙動 | evoeditor 側の配慮 |
|---|---|---|
| Windows Terminal (v0.1) | IME の未確定文字列は WT が描く。候補ウィンドウはハードウェアカーソル位置。**カーソルを非表示にしていると候補が左上に出る** | オーバーレイ (パレット等) 入力中もカーソルを必ず入力欄に置き、非表示にしない。描画中の一時的なカーソル非表示は同期出力でまとめ、フレーム末尾で必ず表示に戻す |
| conhost (v0.1) | 同上。IME ON 中に一部 Ctrl キーが IME に吸われる (正常) | — |
| macOS Terminal.app / iTerm2 (v0.2) | インライン前編集 (下線) をカーソル位置に端末が描く。全角幅のずれは端末の幅計算 (曖昧幅設定) と evoeditor の幅計算 (NFR-605) が一致していないと起きる | 設定 `view.ambiguous_width` (07 §5.3) を端末の設定に合わせるようヘルプで案内 |
| Linux: VTE 系 + ibus/fcitx5 (v0.2) | インライン前編集対応 | — |
| Linux: kitty (v0.2) | `GLFW_IM_MODULE=ibus` 環境変数が必要 (fcitx5 も ibus 互換モードで) | ヘルプに記載 |
| Linux: Alacritty (v0.2) | 0.13 以降で改善。古い版は候補ウィンドウが画面左上に固定される (端末側の制約) | ヘルプに記載 |
| SSH 越し (v0.2) | IME はローカル端末で動作し、確定文字列が UTF-8 で届く。リモートの `LANG` が非 UTF-8 でも evoeditor は UTF-8 として扱うので問題ない | — |
| WSL (v0.2) | Windows 側の IME (Windows Terminal) | — |
| 共通 | **Space/Enter/Esc が IME に消費される** 間はキーイベントが来ない。「Enter で確定」と「Enter で改行」の二重判定は不要 (確定後の Enter だけ届く) | 変換中に `InputEvent::Text` が複数回に分割されて届く端末がある → `Text` の連結は Undo 結合規則 (03) で吸収 |
---

## 11. シグナル・コンソールイベント・終了

### 11.1 Unix シグナル

**v0.2 (macOS / Linux 出荷時) の仕様**。v0.1 (Windows) では実装しない (Windows の経路は §11.2)。

| シグナル | 発生源 | 扱い | HostMsg |
|---|---|---|---|
| SIGINT (Ctrl+C) | raw モードでは発生しない (ISIG off)。`kill -INT` のみ | **終了要求** として扱う (未保存あり → 緊急退避 → 終了。対話確認はしない: 送信者は停止を期待している) | `Signal(Interrupt)` |
| SIGTERM | `kill`、systemd 停止、コンテナ停止 | 同上 (猶予は通常 10 s。退避は数百 ms) | `Signal(Terminate)` |
| SIGHUP | 端末が閉じた、SSH 切断 | 端末は既に無い → **対話不可**。緊急退避 → 端末復元は失敗を無視 → 終了 | `Signal(Hangup)` |
| SIGTSTP (Ctrl+Z) | raw モードでは発生しない。`Effect::Suspend` (コマンド `app.suspend`、v0.2) で自ら発生させる | §11.3 (v0.2) | — |
| SIGCONT | `fg` | 端末再初期化 + 全面再描画 + サイズ再取得 | `Signal(Continue)` |
| SIGWINCH | リサイズ | `InputEvent::Resize` (crossterm/signal-hook が処理) | 入力経路 |
| SIGPIPE | stdout 切断 | Rust 既定で無視 (EPIPE エラーとして出力側に現れる) → 書込失敗は終了要求に変換 | — |
| SIGQUIT (Ctrl+\) | raw モードで発生しない | 既定 (コアダンプ) のまま。デバッグ用に残す | — |
| SIGUSR1/2 | — | 未使用 (将来: 設定再読込?) | — |
| SIGSEGV/SIGBUS/SIGABRT | バグ、mmap なし設計のため SIGBUS は想定外 | ハンドラ内で **端末復元のみ** (`write(2)` で復元シーケンスを書く。メモリ確保・ロック取得をしない) → 既定動作へ再送 (コアダンプ)。緊急退避は行わない (ヒープが壊れている可能性。パニックとは違う) | — |

実装: `signal-hook` (crossterm が Unix で既に依存) でシグナルをフラグ/パイプに変換し、**専用のシグナルスレッド** が受ける。シグナルハンドラ内で `EditorState` に触れない。終了系シグナル (INT / TERM / HUP) の役割分担は §11.4 の「役割分担」のとおりで、シグナルスレッドが **先に退避を書き、その後** `HostMsg::Signal` を送る。

### 11.2 Windows コンソールイベント

**v0.1 で実装する** (CTRL_C / CTRL_BREAK / CTRL_CLOSE。セッション終了の隠しウィンドウは v0.2)。

`SetConsoleCtrlHandler` のハンドラは **別スレッド** で呼ばれる。

| イベント | 発生源 | 扱い | 制約 |
|---|---|---|---|
| CTRL_C_EVENT | `ENABLE_PROCESSED_INPUT` を落としているため通常来ない。`GenerateConsoleCtrlEvent` で来る | 終了要求 (`Signal(Interrupt)`)。終了コード 130 (下の「CTRL_C / CTRL_BREAK の終了手順」) | — |
| CTRL_BREAK_EVENT | Ctrl+Break (`ENABLE_PROCESSED_INPUT` と無関係に来る。入力の自前ループ (§9.1) に `VK_CANCEL` のキーイベントが届いても破棄する。ループが詰まらずにこのイベントが届くことは S-07 で確認) | 同上。**キーボードの Ctrl+Break も、外部から送られた CTRL_BREAK と同じく確認なしで退避して終了する** (ハンドラは送り手を区別できない。下の「キーボードの Ctrl+Break」) | — |
| CTRL_CLOSE_EVENT | コンソールウィンドウ/タブを閉じた、ConPTY 切断 | ハンドラが戻らないまま **約 5 秒** たつと OS がプロセスを強制終了する。この 5 秒は、`HandlerRoutine` の文書の記述から CTRL_CLOSE_EVENT に対するシステムの固定値と読める (`SPI_GETHUNGAPPTIMEOUT` などの利用者設定には依らないと見込む。要確認 (S-11))。メインループへの通知を待たず、ハンドラスレッドで **緊急スナップショットから直接退避** し (§11.4)、下の時系列表の上限内で `TRUE` を返す。大きな文書は書き切れないため **小さい文書から順に** 書き、書けた分を manifest に記録する。ConPTY (Windows Terminal) でタブを閉じた時に本イベントが届くか、メインがブロックした状態でも退避できるかは **S-11 で検証** (v0.1) | ハンドラ内で UI を出せない。§11.4 の「メインスレッドの終了を最大 2 秒待つ」手順と端末の復元は行わない (コンソールは閉じられる) |
| CTRL_LOGOFF_EVENT / CTRL_SHUTDOWN_EVENT | ログオフ・シャットダウン | **対話的なアプリには届かない** (Microsoft の文書: これらはサービスだけが受ける)。evoeditor はこのイベントに頼らない | v0.1 は退避されないことがある (既知の制約)。v0.2 で下の「セッション終了の受け方」で扱う |

**CTRL_CLOSE_EVENT の時間予算** (受信から `TRUE` を返すまで、退避と置換中フラグの待ちを合わせて **≤ 4.5 s**。総額 4.5 s の正本は 02 §11.2.1、内訳は本表):

| 時刻 (受信から) | 処理 (すべてハンドラスレッド) | 上限 |
|---|---|---|
| t = 0 | 受信。退避の状態の CAS (未開始 → 実行中。§11.4) | — |
| t = 0 〜 | CAS に勝てば緊急退避を書く (小さい文書から順に)。負けたら書き手の完了を待つ | **t = 3.5 s** で打ち切り、書けた分を manifest に記録して完了にする |
| 退避の完了 〜 | `HostMsg::Signal` を送る (メインの終了は待たない)。保存ジョブの置換中フラグ (§4.1 の「置換中の計数」。in-place では書込の全体、§4.4 手順 6) が立っていれば (計数 > 0)、0 に戻るのを待つ (0 なら待たない) | **t = 4.3 s** で打ち切る (下の「予算内に終わらない保存」) |
| 〜 | ログの flush を 1 回試みる (待たない)。`TRUE` を返す (以後 OS が終了させる) | **t = 4.5 s** (OS の約 5 秒との差 0.5 s は余裕) |

**予算内に終わらない保存**: 置換中フラグの待ちを t = 4.3 s で打ち切ると、その後 OS がプロセスを終わらせるので、保存は途中で中断される。原子的置換の置換 (通常数 ms) ではまず起きないが、大きなファイルの直接上書き (§4.4 手順 6 / 7。書込の全体がフラグの区間) は 4.3 s に収まらないことがある。この場合の T は半端に書き換わったままになり、§4.4 手順 10 のとおり **事前バックアップ (完全) と保存中フラグ (`method = InPlace`、`backup_complete = true`) で回復する** (次に T を開いたときに `msg.open.interrupted_save` でバックアップの場所を示す)。dirty な内容は (1) の緊急退避に入っている。この経路 (in-place の書込中に CTRL_CLOSE を受け、予算を超えて強制終了される) は 09 の kill テスト (実 FS で `TerminateProcess`、09 §3.6) に加える。

**CTRL_C / CTRL_BREAK の終了手順** (終了コード 130。§11.5。順序と各段の上限の正本は 02 §9.6 / §11.2.1 の「終了の順序と時間予算」の表で、本書は Windows での実装だけを書く): (1) ハンドラスレッドが §11.4 の規則で退避を書く (最大 2.5 s。CAS に負けたら `Done` を待つ)。(2) ハンドラスレッドが `HostMsg::Signal(Interrupt)` をメインへ送る。メインスレッドは、退避の `Done` を待ち → 端末を復元し (`Frontend::restore`) → 置換中フラグが下りるのを待ち (§11.4 の「保存中の終了」) → ログを flush して `std::process::exit(130)` を呼ぶ (通常終了と同じ「restore → フラグ待ち」の順)。(3) ハンドラスレッドは `HostMsg` を送った後 **戻らずに** 最大 2 秒待つ (CTRL_C / CTRL_BREAK ではハンドラが `TRUE` を返すとプロセスが続行するため)。その間にプロセスが終わらなければ (メインがブロックしている)、ハンドラスレッドが置換中フラグが下りるのを、合計の上限 (受信から 5 s) から (4) の 200 ms を残した範囲で待つ。(4) ハンドラスレッドが下の「端末の緊急復元」を 1 回試み (使い捨てのスレッドを最大 200 ms 待つ)、`std::process::exit(130)` を呼ぶ。

**キーボードの Ctrl+Break (提案、OQ-201)**: 利用者が Ctrl+Break を押すと、`app.quit` (FR-107 の未保存の確認) を経ずに、dirty な文書を退避して終了コード 130 で終わる。v0.1 はこれを **既知の挙動** として採り、リリースノートとヘルプに「Ctrl+Break は確認なしで終了します (未保存の内容は退避先に残ります。起動時に案内します)」と書く。理由: (1) コンソールのハンドラはキーボード由来と `GenerateConsoleCtrlEvent` 由来を区別できず、外部から送られた CTRL_BREAK (プロセス管理ツールの停止要求) では送り手が停止を期待している (OQ-28 と同じ)。(2) 確認の経路に回すと、メインがブロックしているときや送り手が続けて強制終了したときに退避されずに内容を失う。(3) 退避するので内容は失わない (P1)。既定のキーマップは Ctrl+Break に何も割り当てない (割り当ててもアプリには届かない)。代替案 (CTRL_BREAK を受けたら退避を書いてから `app.quit` の確認に回す) は、退避の「1 回だけ」の規則 (02 §11.2.1) と正常終了時の退避の後片付けが要るため v0.1 では採らない。

**端末の緊急復元 (シグナル・コンソール経路)**: 実装は eve-platform の Host 部品 `emergency_restore(handle, bytes, saved_mode, restored)` で、引数 (描画先の生ハンドル・復元シーケンスのバイト列・保存したコンソールモード・「復元済み」の原子フラグ `TermRestoredFlag`) は起動時に frontend が `Frontend::emergency_restore_info()` で Host に渡したものを使う (型 `EmergencyRestoreInfo` (`handle: RawTermHandle` 等) の正本は 02 §9.2、復元の手順の正本は 02 §9.6 の手順 4)。Windows のコンソールには非ブロッキングの書込が無い。ハンドラスレッドからの復元は、std の `stdout()` のロック (メインが描画中に握ったまま止まっている可能性がある) を通さず、起動時に保存した **描画先の生ハンドル** (コンソールの出力ハンドル、または §9.1 の `CONOUT$`) に、コンソールなら `WriteConsoleW`、そうでなければ `WriteFile` で直接書く。書く内容は、先頭に **CAN (0x18)** を置いて書きかけの VT シーケンスを打ち切り、続けて §9.1 の「復元」の各シーケンスとし、その後 `SetConsoleMode` を保存値に戻す。メインの `restore()` と **「復元済み」の原子フラグを共有** し (frontend が作って `EmergencyRestoreInfo.restored` で渡したもの。型は eve-view、02 §9.2)、CAS で立てた側だけが復元する (二重の復元で画面を乱さない)。コンソールの選択モード中などは書込が止まるので、書込は使い捨てのスレッドで行い、ハンドラスレッドは最大 200 ms だけ待って先へ進む。

**v0.1 の退避の到達経路** (S-11 で確かめる。08 §3): (1) CTRL_CLOSE_EVENT (コンソールウィンドウ・Windows Terminal のタブを閉じた、ConPTY 切断)、(2) パニック (§11.4)、(3) **メインスレッドがブロックした状態での CTRL_CLOSE_EVENT** (ハンドラスレッドが緊急スナップショットから直接書くので、メインを待たずに退避できることを確かめる)。外部から送られた CTRL_C / CTRL_BREAK (上の表) も同じハンドラで受けて退避する。**Windows のログオフ・シャットダウン時は未保存の内容が退避されないことがある** (v0.1 の既知の制約。確定 (ユーザー回答)。01 §4.6・NFR-302 の備考・リリースノートに書く)。下の隠しウィンドウ方式は v0.2 で実装する。

**セッション終了 (ログオフ・シャットダウン) の受け方 (v0.2。ADR-0023。ベストエフォート)**:

- **前提の事実**: (1) crossterm 0.29 は Windows で user32 の関数 (`ToUnicodeEx` / `GetKeyboardLayout` / `GetForegroundWindow`) を静的にインポートしているので、user32 は **起動時から** 読み込まれている (入力に crossterm の `event::read()` を使わなくても、出力のために crossterm をリンクする限り同じ。インポートが入力の機能 (feature) の有無で変わるかは要確認) (読込の時期を遅らせて CTRL_LOGOFF / CTRL_SHUTDOWN を受ける方法は成り立たない)。(2) CTRL_LOGOFF_EVENT / CTRL_SHUTDOWN_EVENT は対話的なアプリに届かない。(3) メッセージ専用ウィンドウ (`HWND_MESSAGE`) はブロードキャストを受けないので、`WM_QUERYENDSESSION` を受けられない (§8.2 のクリップボードの所有者がメッセージ専用ウィンドウで足りるのは、ブロードキャストを受ける必要が無いため)。
- **方式**: 起動時にセッション終了通知の専用スレッド (02 §9.1) を立て、そこで **非表示のトップレベルウィンドウ** (`WS_VISIBLE` を付けない通常のウィンドウ。`HWND_MESSAGE` ではない) を作ってメッセージループを回す。作成は起動経路をブロックしない (スレッドを立てるだけで先へ進む。コストは BM-101a で計測)。`WM_QUERYENDSESSION` を受けたら §11.4 の退避を開始し (`ShutdownBlockReasonCreate` で「未保存の内容を退避しています」を示す)、`TRUE` を返す。`WM_ENDSESSION` (`wParam = TRUE`) を受けたら、退避の完了 (または時間予算の満了、§11.4) を待ってから戻る。§8.2 のクリップボードの所有者 (v0.1 はメッセージ専用ウィンドウ) をこのウィンドウにまとめるかは実装時に決める。猶予の長さは OS の設定に依存する (既定でおよそ 5 秒。v0.2 の S-11 で実測)。
- **Windows Terminal / ConPTY**: ログオフ時に、端末側がコンソールに CTRL_CLOSE_EVENT を送るか (送れば上の表の経路で退避される) を v0.2 の S-11 で確かめる。
- **確認できなかった場合**: 隠しウィンドウで到達を確認できなければ、方式を増やさず、v0.1 の既知の制約をそのまま残す。確認できた経路もリリースノートに書く。

Windows には SIGTSTP 相当がないため `Capabilities.can_suspend = false` とし、`app.suspend` コマンド (v0.2) は when 句で無効になる (パレットに出さない)。

### 11.3 サスペンド (Unix、`Effect::Suspend`、v0.2)

```
1. 端末復元 (§9.2) — 入力スレッドは read をブロック中なので、復元後に stdin の読み取りをいったん止める
   (crossterm の poll にタイムアウトを入れる、または入力スレッドにフラグを立ててから SIGCONT 後に再開)
2. kill(getpid(), SIGTSTP)  — 自分のプロセスグループではなく自分だけ。tmux/シェルが前面ジョブとして停止させる
3. (停止中) …
4. SIGCONT 受信 → 端末初期化 → サイズ再取得 → needs_render 全面
```

evoeditor が **非対話で起動された (バックグラウンドジョブ)** 場合、SIGTTIN/SIGTTOU で停止するのが Unix の通常挙動。起動時に `tcgetpgrp(STDIN) != getpgrp()` なら「前面で起動してください」と終了する。

### 11.4 緊急スナップショット (NFR-302 の実装前提)

Windows コンソールハンドラ・シグナルスレッド (v0.2)、およびパニック後のメイン (`catch_unwind` の後。`EditorState` の不変条件が壊れている可能性がある) は **`EditorState` に触れずに** 未保存内容を書き出す必要がある。そのため eve-host は次を持つ:

```rust
// 擬似コード。DocSnapshot の型 (フィールド) は 02 §5.4 が正本で、本書は再定義しない (manifest の形式は 02 §11.2.1)
// Host は core から Editor::emergency_snapshot(doc) -> Option<DocSnapshot> で受け取る (None = 退避不要 (dirty でない))
static EMERGENCY: EmergencySlots;   // 文書ごとのスロット: Vec<ArcSwapOption<DocSnapshot>> 相当 (lock-free)。
                                    // Mutex を使う場合は try_lock のみ。退避経路でブロックするロックを取らない (02 §14.1 MR9)
```

- **緊急退避の方式の正本は [02 §11.2.1](02-architecture.md#1121-緊急退避の方式-正本adr-0023)** (ADR-0023、条件付き確定 (S-11)。書き手は eve-host)。本節は OS 別のイベント受信 (シグナル・コンソールハンドラ・隠しウィンドウ)、書き手の起動、退避ファイルの配置 (§7.5) を書く。`DocSnapshot` の内容は 02 §5.4、manifest に書くキー (`sensitive` を含む) は 02 §11.2.1 の正本スキーマに従い (`saved_node_exists` は `DocSnapshot` の欄で、manifest のスキーマには無い。書き手は manifest に書かない)、core 側の契約は 03 §10.1。Host は `Editor::emergency_snapshot(doc)` が返した `DocSnapshot` をそのままスロットに入れ、書き手はスロットの値だけから書く (`EditorState` に触れない)。
- **シグナル・コンソールイベントでは確認を出さずに退避する** (確定 (ユーザー回答)、OQ-28)。送信者は停止を期待しており、端末が既に無いこともあるため。
- 更新の時機の正本は 02 §11.2.1 の「公開」: Host は Editor の状態を変える呼出し (`open_initial` / `handle_input` / `on_job_result` / `tick` / `execute`) の **直後に毎回**、`Editor::take_snapshot_updates()` が返した文書 (Transaction の適用・保存の完了・読込の完了・閉じた文書) のスロットだけを `Editor::emergency_snapshot(doc)` の値で差し替える (02 §9.2 の `sync_emergency`。rope の clone は O(1) なのでコスト無視可。文書ごとのスロットなので他文書の差し替えと競合しない)。スロットを空にするのは **文書が dirty でない (履歴の現在位置が保存点にある) とき** だけである (02 §11.2.1 / 03 §10.1。保存中にも編集できるが、保存中に入力した文書は保存の完了の後も dirty なのでスロットから消えない)。保存ジョブの実行中に閉じた文書のスロットは、その保存の完了まで空にしない (03 §9.2 の「閉じる手順」)。スロットの更新で確保するのは `ArcSwapOption::store` に渡す小さな `Arc` の 1 回だけである。
- **退避は一度だけ (書き手の競合の規則。正本は 02 §11.2.1)**: 退避の状態は原子変数の 3 値 (未開始 / 実行中 / 完了) で、未開始 → 実行中の CAS に勝った経路だけが書く。**負けた経路 (パニックを `catch_unwind` で捕まえたメイン・`HostMsg::Fatal` を受けたメイン・シグナルスレッド (v0.2)・コンソールハンドラ・セッション終了の隠しウィンドウ (v0.2)) は、完了 (または時間予算の満了) を待ってから** exit / `_exit` / ハンドラの return を行う (待ちは時間付き。例: SSH 切断で SIGHUP と入力の EOF が同時に来ても、書込中の退避を途中で殺さない)。**CAS に負けた側は勝った側の完了を最大 5 s 待つ** が、その経路の退避の段の上限 (CTRL_CLOSE とセッション終了 (v0.2) は受信から 3.5 s、シグナル・外部からの CTRL_C / CTRL_BREAK は 2.5 s、Fatal・パニックは 5 s) が先に来ればそこで打ち切って次の段へ進む (合計の上限は CTRL_CLOSE とセッション終了が 4.5 s、シグナル・Fatal・パニックが 5 s。数値の正本は 02 §11.2.1 の「終了の順序と時間予算」の表)。書き手も予算を超えたら、書けた分を manifest に記録して完了にする。
- **役割分担 (ADR-0023。順序と時間の正本は 02 §9.6 の手順 1〜4 と 02 §11.2.1 の「終了の順序と時間予算」の表)**: (1) シグナル・コンソールハンドラのスレッドが **直ちに** 上の規則で退避を書く (メインを待たない。メインがフリーズしていても退避できる)。(2) その後 `HostMsg::Signal` を送り、**最大 2 秒** メインの終了を待つ。メインは退避の `Done` を待つ → `Frontend::restore` → 置換中フラグを待つ → 終了 (通常終了と同じ「restore → フラグ待ち」の順)。(3) メインが終わらなければ、受信スレッドが置換中フラグを、合計の上限の残りから (4) の 200 ms を残した範囲で待つ。(4) 受信スレッドが **復元シーケンスを 1 回だけ** 試し (Unix は非ブロッキング書込。Windows は §11.2 の「端末の緊急復元」。使い捨てのスレッドを最大 200 ms 待つ)、§11.5 の方法で終了する。**Windows の CTRL_CLOSE とセッション終了では (2) は送るだけで待たず、(4) を行わない**: (1)・(2)・(3) を §11.2 の時系列表の上限内で終えて `TRUE` を返す (受信から ≤ 4.5 s)。
- **保存中の終了**: 非対話の終了 (シグナル・コンソールイベント・セッション終了) は、終了する前に、保存ジョブの「置換中」の計数 (`ReplacingGuard`、§4.1 の「置換中の計数」) が 0 に戻るのを時間予算の範囲で待つ (置換の途中で殺すと、Windows では T が無く `.eve-bak-*` だけが残る窓に入る)。計数を上げるのは利用者のファイルの置換と in-place の書込だけで、状態ファイル・manifest・スワップの書込は上げない (待ちの対象にならない)。計数なので、並行する 2 つの保存の一方が終わっても他方の区間は見えたまま残る。**直接上書き (`write_in_place`) では、ガードが上書きの開始から fsync の完了までの全体を覆う** (§4.4 手順 6) ので、書込中も 0 に戻るまで待つ (途中で殺すと T が半端に書き換わる)。ただし待つのはその経路の時間予算の範囲だけで (CTRL_CLOSE は受信から t = 4.3 s まで、§11.2。シグナルは合計 5 s から緊急復元の 200 ms を残すまで。パニック・`HostMsg::Fatal` のメインは端末の復元の後、合計 5 s の残りの範囲で退避の完了から最大 1 s、残りが無ければ待たない。02 §11.2.1 の表)、予算を超えたら待ちを打ち切って終了へ進む。打ち切った in-place は事前バックアップと保存中フラグで回復する (§4.4 手順 10、§11.2 の「予算内に終わらない保存」)。原子的置換の書込中 (一時ファイルへの書込。置換の前) の保存ジョブは待たない (一時ファイルが残るだけで T は無傷。§4.6)。通常の終了 (利用者の操作) の扱いは §11.5。
- 緊急時はスロットを読み、`state_dir/recovery/<ts>-<pid>/` (使えなければ §7.5 の代替の退避先) に 02 §11.2.1 の形式で書く (配置・権限は §7.5)。**小さい文書から順に** 書き、時間予算 (Windows CTRL_CLOSE は退避に t = 3.5 s まで (§11.2)、シグナル・外部からの CTRL_C / CTRL_BREAK は 2.5 s、Fatal・パニックは 5 s。正本は 02 §11.2.1) 内に書けた分を manifest に記録する。書込は Port (`FileSystem`) を経由せず (依存注入の状態に左右されない) eve-platform の関数を直接呼ぶ。ディレクトリは §5.6 の「所有者のみ」の DACL で作る関数で作り (`std::fs::create_dir_all` は使わない)、中のファイルは `std::fs` で書いてよい (ディレクトリの DACL を継承する)。確保は `try_reserve` で行い失敗したらその文書を飛ばす。
- **NFR-302 AC-5 (100 MB を 5 s 以内に退避) の測定経路**: 退避の段が 5 s を使えるのはパニック・`HostMsg::Fatal` の経路だけである。シグナル・外部からの CTRL_C / CTRL_BREAK の経路の退避の段は 2.5 s (CTRL_CLOSE は t = 3.5 s) で打ち切られ、書き切れなければ manifest は `complete = false` のまま残る (§11.2、02 §11.2.1)。したがって AC-5 の 100 MB・5 s はパニック / Fatal の経路で測り、シグナル・コンソールの経路はそれぞれの予算で測る (09)。
- **他スレッドのパニック**: ワーカーはジョブ単位で `catch_unwind` し `JobResult::Failed(Panic)` に変換する (エディタは継続)。パニックフックは **メインスレッド以外では端末に触れない**。`run_job` (と Host の作業) を実行中のワーカー (スレッドローカルの印 `IN_JOB` が立っている) では **ログだけを書き、`HostMsg::Fatal` を送らない** (ジョブ単位の `catch_unwind` が `Failed(Panic)` に変えて続けるため)。それ以外のメイン以外のスレッドでは `HostMsg::Fatal` をメインへ送るだけにする (02 §11.2)。入力スレッドの死亡 (パニック・`read` の恒久エラー) も `Fatal` とし、メインは緊急退避 → 端末復元 → 終了する (操作不能のまま残さない)。パニックフックはどのスレッドでも端末に触れない (02 §9.1)。メインスレッド自身のパニックは、メインループの各メッセージ処理を包む `catch_unwind` で捕まえ、Host が緊急退避 → 端末復元 → クラッシュログ → 終了を行う (02 §11.2。経路はこの 1 本)。
- 書き手 (eve-platform の Host 部品の呼出し) は経路ごとに独立した `catch_unwind` で包み、書き手自身がパニックしても退避の状態を `Done` にして端末の復元へ進む (02 §11.2.1 の「1 回だけ実行する規則」)。
- 書込後、端末復元、クラッシュログ (09 §10)、復旧案内 (退避先の絶対パス、または退避できなかった理由) を stderr に出力して終了。終了コードは **07 §18.4 が正本** (パニック = 70 (EX_SOFTWARE)、シグナル由来は 128+n)。
- **swap-lite (v0.2) があっても緊急退避は行う** (swap-lite は 2 秒遅延があり直近の入力が落ちる)。
- **v0.1 で守れない経路** (`TerminateProcess` (タスクマネージャの強制終了) / OOM abort / 電源断 / CTRL_CLOSE 不達 / Windows のログオフ・シャットダウン (隠しウィンドウは v0.2。既知の制約、確定 (ユーザー回答))) は 03 §10.1 のとおりリスク受容 (OQ-42)。Unix の SIGKILL も同じ扱い (v0.2)。

### 11.5 終了シーケンスと終了コード

```
quit 要求 → 未保存確認 (対話可能な場合のみ) → 実行中の保存ジョブがあれば完了を待つ (最長 5 s。この間は確認を出さない)
  → 5 s を超えたら利用者に確認する (04 §15.2 の ui.confirm.quit_saving。遷移の正本は 03 §9.7 (f)):
       [W] 待つ ★ → 完了まで待ち続ける (成功したら終了へ進む。失敗したら 03 §9.7 (e) の選択へ)
       [Q] 保存を中止して終了 → 保存ジョブを中止し (下記)、dirty な文書を緊急退避と同じ書き手・形式で退避してから終了へ進む
       Esc → [W] と同じ (終了の要求を取り下げずに待つ)
→ swap-lite ファイル削除 (正常終了時、v0.2) → セッション保存 (v0.4) → 端末復元 (Frontend::restore)
→ 置換中フラグが上がっていたら、待つ前に stderr へ 1 行 `stderr.quit.waiting_save` (「保存の完了を待っています: {path}」。04 §15.2。待たずに済むときは出さない。保存先が複数なら 1 行ずつ。02 §9.6 / §11.2.1)
→ 置換中フラグが下りるまで待つ (タイムアウトなし。下記) → stderr の行 (Effect::Quit.stderr と、退避したときの stderr.recovery.*。02 §11.2.1 の表の 5) → ログ flush → exit
(X11 クリップボード所有権の待機は外部コマンドが保持するため不要。arboard 採用時のみ ≤200 ms、OQ-79)
```

- **順序の正本は 02 §9.6 / §11.2.1 の表の「通常終了」の列** (端末の復元の後に置換中フラグを待つ)。
- **置換中は exit しない**: 保存ジョブの置換中フラグ (§4.1 の「置換中の計数」。in-place では書込の全体、§4.4 手順 6) が立っている (計数 > 0) 間は、通常の終了でも exit しない (利用者が「保存を中止して終了」を選んだ場合も、フラグが下りるまで待ってから exit する。置換は通常ミリ秒で終わる)。「保存を中止して終了」は、実行中の保存ジョブに **常に** cancel を送り (main はジョブが置換の前か最中かを観測しない)、ジョブの終端の結果を待つ。取消しを無視するのはジョブ側である: 置換の前の段 (乾式走査・照合・一時ファイル / 事前バックアップの書込) のジョブは一時ファイルを削除して (§4.1 手順 8、§4.4 手順 9) `Cancelled` で終わり、`ReplaceFileW` の呼出しを始めたジョブと in-place の上書き (§4.4 手順 6) を始めたジョブは取消しを見ずに完了まで進む (途中で止めると元ファイルが半端になる。02 §4.5 の「取消し」)。

**終了コードの正本は [07 §18.4](07-config-extensibility.md#184-終了コード)** (CLI 仕様の責務。確定 (ユーザー回答)、OQ-28: `0` 正常 / `1` 実行時エラー / `2` 使い方 / `3` `--check-config` エラー / `4` `app.quit_with_error` / `5` 端末初期化失敗 / `70` パニック / `128+n` シグナル。`3` と `4` は v0.2 で発生し、v0.1 では番号を予約するだけ)。本書は OS 差のみ:

| 項目 | Unix (v0.2) | Windows (v0.1) |
|---|---|---|
| シグナル / コンソールイベントによる終了 | `128 + signo` (SIGHUP = 129、SIGINT = 130、SIGTERM = 143)。緊急退避後 (§11.4 の時間予算の後を含む) に、そのシグナルのハンドラを既定 (`SIG_DFL`) に戻して **自分に再送** し、OS に終了させる (シェルが「シグナル死」と認識できる)。再送しても終わらなかった場合だけ `_exit(128 + signo)` | コンソールイベントには signo が無いため、CTRL_C / CTRL_BREAK = 130 (手順は §11.2 の「CTRL_C / CTRL_BREAK の終了手順」)、CTRL_CLOSE はハンドラから `TRUE` を返した後 OS が終了させる (コードは OS 既定)。セッション終了 (§11.2、v0.2) は `WM_ENDSESSION` から戻った後 OS が終了させる。`app.quit` 相当の経路なら 0 |
| パニック | 70 | 70 (`std::process::exit(70)`。`abort` にしない) |
| 端末初期化失敗 | 5 (`isatty` 偽、`tcsetattr` 失敗、`TERM=dumb`) | 5 (`SetConsoleMode` 拒否、`CONOUT$` 不可) |

---

## 12. ファイル監視

### 12.1 OS 別バックエンド (notify クレート経由)

| OS | 機構 | 粒度 | 既知の制約 |
|---|---|---|---|
| Linux | inotify | ファイル or ディレクトリ (非再帰) | `fs.inotify.max_user_watches` (既定 8192〜、超過で ENOSPC)、`max_user_instances` (128)。**/mnt/* (WSL2 DrvFs)、NFS、CIFS、FUSE の多く、overlayfs の一部** はイベントが来ない。inode 単位なので原子的置換 (rename) 後は **古い inode を監視し続ける** → 親ディレクトリを監視する |
| macOS | FSEvents (既定) / kqueue (`macos_kqueue` feature) | ディレクトリ (再帰、遅延 latency 既定 ~0.1〜1 s) | FSEvents はイベントを合体し「何か変わった」しか分からないことがある → `Rescan` として扱い mtime/len/hash を取り直す。kqueue はファイル記述子を消費 (多数ファイルで枯渇) |
| Windows | `ReadDirectoryChangesW` | ディレクトリ | 1 回の保存で Modified が **複数回** 届く。ネットワーク共有 (SMB) では届かないことがある。監視中のディレクトリは **削除・リネームできなくなる** (ハンドル保持) → 空きがない時は監視を解除 |

### 12.2 方針

1. **監視対象は「開いている Document の親ディレクトリ (非再帰)」**。ファイル単体監視は inode 置換に弱い。ワークスペース全体の再帰監視 (ファイルツリー用、v0.4 Could) は別の WatchKind で、上限 (既定 10,000 エントリ) を設ける。
2. **デバウンス 100 ms** (`notify-debouncer-mini` 相当、または自前)。同じパスのイベントは最後のものだけ core へ。
3. **自己イベント抑制**: 保存ジョブが完了したら `(path, new_stamp)` を監視スレッドに通知し、`new_stamp` と一致するイベントを捨てる。それでも抜けたものは core 側で `(mtime, len, hash)` 比較により「変化なし」と判定され無視される (二重防御)。
4. **イベントが来ない FS** (`WatchCaps.native_events == false`、または `FsKind::Network`、WSL DrvFs): **ポーリングはしない** (NFR-204)。代わりに (a) 保存時 (FR-117 v0.1)、(b) フォーカス取得時 (`InputEvent::Focus(true)`。v0.2)、(c) ユーザーが明示的に `file.check_external_changes` を実行した時、に mtime/len を確認する。設定 `file.watch.poll_interval_ms` (07 §5.2) を **ユーザーが明示的に設定した場合のみ** 低頻度ポーリング (最小 2000 ms) を許可する (OQ-78)。
5. **初期化失敗** (inotify 上限、権限) は警告 1 回で監視なし運用へ縮退。起動を妨げない。
6. 監視スレッドは `HostMsg::Fs` を送るだけで、`EditorState` に触れない。

---

## 13. シェル・外部プロセス起動

本節の機能は v0.4 以降 (`ProcessRunner`) と v1.L (§13.7)。各表の Unix の行は、その版で macOS / Linux を出荷していることが前提。

### 13.1 原則

- `ProcessRunner` は **ユーザー承認後にのみ** 呼ばれる ([02 §4.4](02-architecture.md#44-ports-core-が定義し外側が実装する-trait)、P2 原則)。承認 UI は core (04 の確認ダイアログ)。
- 子プロセスの **stdin/stdout/stderr は常にパイプ** (端末を継承させない。継承すると raw モードの画面を破壊する)。対話的コマンドは非対応。
- 自動検出した実行ファイル (PATH 上の `xclip` 等) を勝手に実行しない (§8.3)。

### 13.2 シェルの既定

| OS | `shell = true` の展開 | 理由 |
|---|---|---|
| Unix | `["/bin/sh", "-c", <cmd>]` | POSIX sh は必ずある。`$SHELL` (fish 等) は構文が違い設定が移植できない |
| Windows | `["cmd.exe", "/D", "/C", <cmd>]` (`%ComSpec%` を優先) | 常に存在、起動が速い (PowerShell は 200〜500 ms)。`/D` でレジストリの AutoRun を無視 |

既定以外のシェル (`$SHELL -c`、`pwsh` / `powershell` 等) は設定 `process.shell` で選べる。

`shell = false` (既定) では argv 配列をそのまま `Command::new(argv[0]).args(&argv[1..])` に渡す。

### 13.3 Windows の引数とバッチファイル

- Windows には argv がなく、コマンドライン文字列を **子プロセスが各自でパースする**。Rust std は MSVC CRT 規約で引用符付けするため、通常の exe には正しく渡る。
- `.bat`/`.cmd` を直接起動する場合、std は `cmd.exe` 経由にし、**BatBadBut (CVE-2024-24576) 対策で引用できない引数を `InvalidInput` エラーにする** (Rust 1.77.2+)。evoeditor はこのエラーをユーザーに「この引数はバッチファイルに安全に渡せません」と表示する。
- npm 系 LSP サーバ (`typescript-language-server.cmd`) は v1.L で同じ経路を通る。`PATHEXT` 解決は std が `.exe` しか行わないため、本層の `resolve_executable` が `PATHEXT` 順に探索して拡張子付きの絶対パスを返す。
- **実行ファイル探索の規則 (全 OS 共通、`resolve_executable` の仕様)**: (1) `command` が絶対パスならそのまま (存在確認のみ)。(2) パス区切りを含む相対パス (`./tool`、`bin/tool`) は **ワークスペース層由来の定義では拒否** し、ユーザー層由来ではワークスペースルートではなく **設定ファイルのディレクトリ基準** で解決する (`${workspace_root}` を明示した場合のみワークスペース基準)。(3) 名前のみの場合は **`PATH` の絶対パス要素だけ** を順に探索する。**カレントディレクトリ (= ワークスペース) は探索しない。`PATH` 中の相対パス要素・空要素 (`::`、末尾の `;`) も無視する** (Rust std は 1.58 以降 Windows でカレントディレクトリを探索しないが、自前で `PATHEXT` 解決するとこの防御が外れる。未信頼リポジトリに置かれた `rust-analyzer.exe` / `rustfmt.cmd` が、ユーザー層の (信頼不要な) 定義から実行される事故を防ぐ)。(4) Windows の `.bat` / `.cmd` も同じ規則に従い、`PATHEXT` の候補拡張子は `.COM;.EXE;.BAT;.CMD` のうち設定で許可したもの (既定: `.exe;.cmd;.bat`) に限る。(5) 解決結果 (絶対パス) を信頼ダイアログ (07 §12) と監査ログに表示する。回帰テスト: 09 §3.4 で cwd に偽の実行ファイルを置いて実行されないことを検査する。
- **ツール自身がリポジトリ内のコードを実行する**: rust-analyzer (build.rs / proc-macro)、prettier (リポジトリの plugin)、ESLint (設定ファイルの `require`) 等は、実行ファイルが信頼できてもリポジトリの内容を実行する。07 §12 の信頼ダイアログと FR-709 の確認文言に「このツールはプロジェクト内のコードを実行する可能性があります」を含める (07 §9.4 / §15)。
- `CREATE_NO_WINDOW` は **付けない** (コンソールアプリから起動する子は親コンソールを共有し新しいウィンドウは出ない)。

### 13.4 プロセス木の終了 (キャンセル・タイムアウト)

| OS | 手段 |
|---|---|
| Unix | `setsid()` または `setpgid(0,0)` で新しいプロセスグループにして起動 (`CommandExt::process_group(0)`) → キャンセル時 `killpg(SIGTERM)` → 2 s 後 `SIGKILL`。孫プロセスも対象 |
| Windows | **Job Object** (`CreateJobObjectW` + `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`、`AssignProcessToJobObject`) → キャンセル時 `TerminateJobObject`。`TerminateProcess` 単体は子孫を殺せない。実装手段は未決 (v0.4 の依存選定で決める)。案は `windows-sys` 直叩き (R5 範囲、約 100 行。依存を増やさず、必要な API は 3 つ) |

パイプの読み取りは専用スレッドで行い (デッドロック防止: stdout と stderr を同時に排出)、出力上限 (既定 64 MiB) を超えたら打ち切る。

### 13.5 環境変数と出力の文字コード

- 子に渡す環境: 親の環境 + `TERM=dumb`、`NO_COLOR=1`、`EVE_FILE=<path>`、`EVE_LINE=<n>` (フィルタの利便)、`LANG`/`LC_*` は継承。`EVE_LOG` 等の自前変数は除去。
- 出力バイト列は raw で返し、**ヒント** として Windows では `GetOEMCP()`、Unix ではロケールのコードセット (`nl_langinfo`) を添える。デコード (UTF-8 優先 → ヒント → chardetng) は 03。

### 13.6 `Effect::OpenUrl`

`xdg-open` / `open` / `rundll32 url.dll,FileProtocolHandler` (Windows は `ShellExecuteW(open)` を推奨) で URL を開く。**http(s)/mailto のみ許可**。`file:` や `javascript:` 等は拒否。これも P2 原則上「外部プロセス」だが、ユーザーが明示的にリンクを開く操作をした場合に限るため都度承認は不要 (ヘルプ画面からのリンク等)。

### 13.7 ローカル LLM サーバ (`llama-server` 等) の OS 別事項 (v1.L)

AI 連携 (06) は **ローカル LLM を一級市民** とし (01 §2.9、ADR-0034)、v1.L-a で外部に起動済みのサーバへ接続する **外部接続** (`connection = "external"`)、v1.L-b で evoeditor が子プロセスとして起動・監視・停止する **管理モード** (`connection = "managed"`、FR-918、Could) を提供する。接続方式・プロトコル・能力検出は 06、設定キーは 07 §5.12 が正本で、本節は **OS 層の規則** だけを書く (S-23 で 3 OS 実機確認)。

| 項目 | 規則 |
|---|---|
| 起動 | `ProcessRunner::spawn_long_lived` (§2.6)。`stdio = LogOnly` (サーバの標準出力・標準エラーはプロトコルに使わず、`log_ring_bytes` (**末尾 256 KiB**、06 §3.3.3) のリングに保持して診断表示 (`ai.runtime_log`) にだけ使う。ファイルには書かない (本文を含み得る)。llama-server のログは冗長で、全保持は MR8 違反)。`priority = BelowNormal` 既定 (`runtime.low_priority = true`。推論が全コアを使ってもエディタの入力処理を奪わない。NFR-115)。`kill_tree_on_exit = true`。起動時期は `runtime.start` (既定 `on_demand`) で、エディタ自身の起動経路には入れない (NFR-101) |
| 実行ファイル | **ユーザー層の設定にある絶対パスのみ** (`ai.providers.<id>.runtime.command`、FR-918 / 06 §3.3.3。`PATH` 探索はしない。`${config_dir}` / `${home}` の展開は可)。初回起動時 (およびコマンド行が変わるごと) に完全なコマンド行を示して承認を得る (`exec` 承認)。承認したコマンド行のハッシュは状態ディレクトリ (`trust.toml` と同じ扱い) に記憶する。**ワークスペース層の起動コマンドは常に無視** (07 §12.3。信頼しても解除できない: 送信先の改変と任意コード実行の両方に当たる)。`.bat` / `.cmd` 経由は不可 (§13.3 の BatBadBut 対策の対象になるため、exe を直接指す)。ユーザーの追加引数 (`runtime.args`) に `--host` / `--port` / `--api-key` / `--cors-origins` と、サーバのツール実行・エージェント・RPC 系 (`--tools` / `--agent` / `--mcp-*` / `--rpc` 等) が含まれていたら **起動を拒否** (06 §3.3.3 / §11.5) |
| 待受アドレスとポート | **ループバック固定** (`--host 127.0.0.1`。NFR-509)。管理モードで `0.0.0.0` や LAN アドレスを渡す設定は拒否 (診断エラー)。ポートは evoeditor が **空きポートを選ぶ**: `127.0.0.1:0` を `bind` して得た番号を閉じてから `--port` で渡す (競合の窓は小さく、失敗したら別ポートで 1 回再試行)。`--port 0` をサーバ自身が受けて実ポートを報告できるなら優先 (S-23 で **要確認**)。ユーザーがポートを固定した場合はそのまま渡し、既に使用中なら **接続せず** エラー (他プロセスの待受に乗らない。06 §11.3 / §11.5 の偽装対策)。`--cors-origins` はブラウザから届かない値に固定 (06 §11.5) |
| 認証 | 起動ごとにランダムな API キーを生成し、**環境変数** (`LLAMA_API_KEY` 相当。サーバが環境変数を受けない版では所有者のみ読めるファイル + `--api-key-file` 相当) で渡す。**コマンド行には載せない** (`ps` / タスクマネージャで見えるため。06 §3.3.3)。以後の要求に Bearer として付ける。キーは `SecretString` (§14) でメモリに持ち、ログ・監査・クラッシュログに出さない (NFR-504) |
| Unix ドメインソケット / 名前付きパイプ (`runtime.transport = "unix"`) | サーバが対応すれば TCP より望ましい (ポート偽装・ブラウザからの到達の余地がなく、権限をファイルモードで閉じられる。06 §3.3.2 (d))。対応状況は S-22 (h) で **要確認** (Windows の名前付きパイプも)。対応時は `runtime_dir` (§7.1、0700) に置き、接続前にソケットファイルの **所有者が自分であること** を確認する (§14.1 の規約と同じ) |
| 起動完了の判定 | `/health` を 250 ms → 最大 2 s の指数バックオフでポーリング (eve-ai のワーカー側。メインスレッドは触らない)。モデル読込中 (503) は `AiStatus.phase = loading_model{elapsed}` を 10 Hz 以下で通知 (04 §8.10)。`runtime.ready_timeout_s` (06 §3.3.3。案 120 s。大きいモデルは数分かかるため設定可) を超えたら子プロセスを止めて報告 |
| 停止 | エディタ終了時 (通常・シグナル・パニックの全経路。§11.5 の終了シーケンスに「管理プロセスの terminate」を加える。退避より **後**、端末復元より前)、無使用時間 `runtime.idle_shutdown_s` 経過 (案 900 s。0 = 停止しない)、設定の変更時、明示コマンド (`ai.runtime_stop`)。手順: 終了要求 → 猶予 (案 3 s、06 §3.3.3) → kill。Unix は `SIGTERM` → `SIGKILL` (プロセスグループ単位、§13.4)。Windows は **Job Object** (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`) に入れて起動し、`TerminateJobObject`。evoeditor 自身が殺された場合も Job の閉鎖で子が残らない (Windows の最大の利点)。Unix で evoeditor が SIGKILL された場合は孤児になり得る → 起動時に前回の `pid` ファイル (`runtime_dir/ai-runtime.pid`、所有者のみ) を見て、同じ argv のプロセスが生きていれば「前回のサーバが残っています: 停止 / 無視」を出す (再接続はしない: 起動ごとのキーが失われているため) |
| GPU・メモリの競合 | evoeditor は GPU を直接触らない。起動前にモデルファイルのサイズ (取得するのはサイズだけ。GGUF の中身は読まない) と空きメモリ (`SystemInfo` Port。最初の利用者が v1.L ならそこで定義、02 §4.4) を比べ、足りなければ警告して確認 (06 §3.3.3)。推論サーバが落ちた場合は `ProcessExited` を受けて「サーバが終了しました (終了コード n)。ログ: …」と表示し、**自動再起動はしない** (クラッシュループ防止。`ai.runtime_restart` で手動)。CPU 推論のスレッド数の既定は論理コア数 − 2 (最低 1。`runtime.threads = 0`)。同じ GPU を複数のサーバで共有する構成はユーザー責任 (evoeditor は 1 プロバイダにつき 1 サーバしか起動しない) |
| 環境変数 | 親の環境を継承 (`CUDA_VISIBLE_DEVICES` / `HIP_VISIBLE_DEVICES` / `GGML_*` 等をユーザーが設定している前提)。`EVE_*` は除去、API キーの環境変数を追加。設定から任意の環境変数を足す項目は **設けない** (危険項目を増やさない。必要ならラッパースクリプトを `command` に指定する)。`TERM` / `NO_COLOR` は §13.5 のとおり |
| Windows の注意 | (1) `127.0.0.1` への bind は **Windows Defender ファイアウォールの許可ダイアログを出さない** (ダイアログが出るのは外向きアドレスに bind したとき。管理モードはループバック固定なので出ない)。ユーザーが自分で `0.0.0.0` で起動した外部サーバについてはダイアログが出るが evoeditor の責務ではない。(2) `CREATE_NO_WINDOW` は不要 (§13.3)。(3) Defender のリアルタイム保護は GGUF (数 GB) の初回読込を遅くする → `ready_timeout_s` の案を 120 s にした理由の 1 つ。(4) Job Object は `CREATE_BREAKAWAY_FROM_JOB` を要する環境 (evoeditor 自身が別の Job 内で起動されている場合) で失敗し得る → 失敗時はプロセス木の終了を `TerminateProcess` + 子の列挙にフォールバックし警告。(5) 優先度は `BELOW_NORMAL_PRIORITY_CLASS` |
| macOS の注意 | 署名のない `llama-server` バイナリは初回に Gatekeeper の隔離属性で起動を拒否されることがある → エラー文言に `xattr -d com.apple.quarantine <path>` を案内 (09 §11.3 のトラブルシューティングにも)。Metal の初期化で最初の要求が遅い (S-22 (c) の TTFT 計測で分離) |
| Linux の注意 | `nice` の変更は自分のプロセスの優先度を下げる方向だけ可 (特権不要)。cgroup / systemd-run による隔離は行わない (Could)。Flatpak / Snap 内の evoeditor からは外部バイナリを起動できないことがある → 外部接続に誘導 |
| WSL | WSL 内の evoeditor から Windows 側で動く `llama-server` (CUDA) に接続するのが一般的 → `localhost` は WSL2 の既定でホストに転送される (`networkingMode=mirrored` または localhostForwarding)。転送が無効な環境では Windows 側のアドレスは **LAN 扱い** (NFR-509) になるため `ai.network.allow_lan_hosts` (平文なら `allow_insecure_hosts` も) への明示が要る旨をエラーに含める |
| 監査 | 起動・停止・再起動・終了コード・解決済み絶対パス・argv (API キーは伏字) を監査ログ (06 §15) に記録 |

---

## 14. 資格情報ストア (v1.L)

| OS | バックエンド | 制約 |
|---|---|---|
| Windows | 資格情報マネージャー (`CredWriteW`) | 汎用資格情報として保存。2560 バイト上限。ユーザープロファイル単位 |
| macOS | Keychain (Security.framework) | 初回アクセス時に「evoeditor がキーチェーンにアクセスしようとしています」ダイアログ。**ad-hoc 署名のバイナリは更新ごとに再確認される** (署名 ID が変わる) → Developer ID 署名の動機 (§18.4) |
| Linux | Secret Service (D-Bus: GNOME Keyring / KWallet / KeePassXC) | ヘッドレス・SSH 先では **D-Bus セッションがない** → `EnvOnly` に縮退。`keyring` クレートの `linux-native` (keyutils) は再起動で消える |

- 採用クレート: `keyring` (MIT/Apache-2.0)。features で不要バックエンドを外す。
- 優先順位: 環境変数 (設定 `api_key_env` で指定した名前、06 §10.1) → SecretStore。**設定ファイルの平文 `api_key` は提供しない** (06 §10.1 / 07 §5.12、FR-907)。
- `SecretString` はドロップ時にゼロ化 (`zeroize`)。ログ・クラッシュログに出さない (NFR-504)。

### 14.1 v1.L 着手時に本書で確定する項目 (見出しのみ予約)

| 項目 | 内容 (v1.L で記述) |
|---|---|
| `SecretStore` のサービス名 / アカウント名規約 | サービス名は `evoeditor` (OQ-01)、アカウント名 = `ai.providers.<name>` のキー |
| 長寿命プロセスの起動と終了 (LSP サーバ v1.L-a、**ローカル LLM サーバの管理モード v1.L-b (06 §3.3.3)**、外部エージェント ACP / MCP v1.X) | `ProcessRunner::spawn_long_lived` (trait は 02 §4.4、OS 別の実装は §2.6) と §13.4 のプロセス木終了。終了要求 → 猶予 (LSP 2 s / LLM サーバ 3 s) → kill。ローカル LLM サーバの OS 別規則 (ジョブオブジェクト / プロセスグループ、優先度、空きポート、環境変数で渡す API キー、上限付きログ、Unix ドメインソケットの所有者確認、`SystemInfo` の空きメモリ) は **§13.7** |
| ローカル待受 (sidecar / MCP サーバ、v1.L-b 候補) のソケット / 名前付きパイプの権限 | Unix: `runtime_dir` (0700) 配下のソケット、`SO_PEERCRED` で同一 uid を確認。Windows: 名前付きパイプの DACL を現在ユーザーのみに。接続トークンは起動ごとにランダム生成し所有者のみ読めるファイルで受け渡し |
| rustls の OS 証明書ストア連携 | 未決 (`rustls-platform-verifier` か `rustls-native-certs` を v1.L で選ぶ)。社内 CA 対応。証明書検証の無効化オプションは提供しない (06 §11.3) |
| 監査ログ (06 §15) の保存場所と権限 | `state_dir/ai-audit/` (0600、ローテーション 5 MB × 3)。書込は eve-ai が直接行うか core 経由かを 02 R7 と合わせて決める |

---

## 15. WSL / SSH / コンテナ環境

**v0.1 では対象外** (Linux バイナリを出荷しないため。WSL 内・SSH 先の Linux・コンテナで動かす用途は成り立たない)。本節は **v0.2 (Linux 出荷時) の仕様** として残す。v0.1 で Windows の evoeditor を WSL のファイル (`\\wsl$\…` / `\\wsl.localhost\…`) に対して使う場合は、ネットワーク上のファイル (`FsKind::Network`) として扱う (保存は §4.2 のフォールバック条件に従う)。

### 15.1 WSL (1 / 2)

| 項目 | 事実 | 方針 |
|---|---|---|
| 検出 | `WSL_DISTRO_NAME` / `WSL_INTEROP` 環境変数、`/proc/sys/fs/binfmt_misc/WSLInterop` | ログと既定値 (クリップボード) にのみ使う |
| `/mnt/c` (DrvFs, 9p) | inotify が **機能しない** (WSL2)、`chmod` は metadata オプションがないと無視、`rename` は原子的だが遅い、大文字小文字非区別、ファイル ID は NTFS 由来 | `FsKind::Network` 相当として扱い、監視はフォーカス時確認に縮退 (§12.2)。保存の fsync 失敗は警告 |
| 改行 | Windows 側で作られた CRLF ファイルが多い | FR-113 のとおり保持。既定は変えない |
| クリップボード | WSLg あり: `wl-copy` / `xclip` (§8.2) が Windows クリップボードと同期。なし: 絶対パスの `clip.exe` (UTF-16LE で渡す) / `powershell.exe` (`/mnt/c/Windows/System32/WindowsPowerShell/v1.0/`、出力を UTF-8 に固定) | §8.2 / §8.3 |
| ローカル LLM (v1.L) | GPU は Windows 側にあることが多く、`llama-server` を Windows で動かし WSL 内の evoeditor から `localhost` に接続する構成が一般的。`localhost` のホスト転送が無効な環境では Windows 側のアドレスが LAN 扱いになる | §13.7 (WSL 行)、06 §11.3 |
| 端末 | Windows Terminal (ConPTY) 経由。Linux バイナリから見ると普通の pty | — |
| パス変換 | `wslpath` で `C:\…` ⇔ `/mnt/c/…`。Windows 側から `wsl evoeditor file` で渡された Windows パスは Linux では無効 | CLI 引数が `X:\` 形式なら **WSL 検出時のみ** `wslpath -u` 相当の変換を試みる (Could) |

### 15.2 SSH 越し (NG5 の「リモート先で起動」の運用)

- `SSH_TTY`/`SSH_CONNECTION` の存在で検出 → クリップボードは OSC 52 を既定優先 (§8.2)。
- `COLORTERM` は転送されない (sshd の `AcceptEnv` 既定は `LANG LC_*`) → True Color 判定が保守的になる。ヘルプで `theme.color_mode = "truecolor"` (07 §5.4) を案内 (04)。
- 遅延の大きい回線では「入力の合体」([02 §9.2](02-architecture.md#92-メインループ-擬似コード)) と同期出力が体感を左右する。evoeditor 側の処理時間目標 (NFR-103) は変わらない。
- 切断 → SIGHUP → 緊急退避 (§11.1)。`tmux`/`screen` 内なら生存する。
- ホームが NFS: スワップの pid 判定に hostname を併用 (§5.3)。

### 15.3 コンテナ (Docker / devcontainer / Kubernetes exec)

- `HOME` が未設定・`/` のことがある → §7.1 の「解決不能」経路。
- `TERM` 未設定で exec されることが多い → 起動エラーの文言で `docker exec -it -e TERM=xterm-256color` を案内。
- musl 静的バイナリで `docker cp` して動かす用途 (NFR-206)。
- PID 1 で動かすとシグナル既定が変わる (SIGTERM 無視) → evoeditor はハンドラを明示登録するので問題ない。
- 読み取り専用ルート FS → `state_dir` 作成失敗 → ログ/スワップ無効で起動 (§7.1)。緊急退避は §7.5 の代替の退避先に書く。

---

## 16. ビルド・ツールチェイン・MSRV

### 16.1 MSRV と edition (OQ-14 の具体化)

| 項目 | 方針 |
|---|---|
| edition | 2024 (Rust 1.85 以上が必須) |
| MSRV | 「リリース時点の最新安定版 − 2 マイナー」を **下限の目安** とし、`[workspace.package] rust-version` に明記。依存が要求すれば引き上げる (マイナーリリースでのみ。パッチリリースでは上げない) |
| 検証 | CI に MSRV ジョブ (`cargo +<MSRV> check --workspace --all-features`。補助ジョブとして v0.2 から。v0.1 は `rust-version` の明記と `Cargo.lock` のコミットまで)。`Cargo.lock` をコミット (bin があるため) し、`resolver.incompatible-rust-versions = "fallback"` (Cargo 1.84+) で MSRV 互換の依存版を選ばせる |
| ツールチェイン固定 | `rust-toolchain.toml` は **置かない** (貢献者の環境を縛らない)。CI は `stable` + MSRV + (nightly は fuzz/udeps のみ) |
| ディストリビューションのパッケージ化 | Debian/Fedora の Rust は古い → MSRV を極端に新しくしない動機。ただし 2 マイナー遅れが限界 (ratatui/crossterm の MSRV も同程度) |

### 16.2 ターゲット別のビルド要件

v0.1 の配布ビルドは `x86_64-pc-windows-msvc` だけ。CI の Linux ジョブ (v0.1) はランナーのホスト (`x86_64-unknown-linux-gnu`) で Linux CI の対象 crate (02 §4.7) のビルドとヘッドレスのテストだけを行い、配布物は作らない (§1.2)。下の表の Windows aarch64・Apple・Linux の配布ビルドは v0.2 (Tier B のビルドと macOS / Linux の出荷)。

| ターゲット | リンカ / 要件 | 備考 |
|---|---|---|
| `*-pc-windows-msvc` (v0.1 は x86_64。aarch64 は v0.2 以降) | Visual Studio Build Tools (MSVC + Windows SDK)。`windows-sys` は純 Rust なのでそれ以上の SDK 依存なし | tree-sitter (v0.4) の C ソースは `cc` クレートが MSVC `cl.exe` でビルド。ARM64 はクロスコンパイル可 (`cl.exe` の arm64 ツールセット) |
| `*-apple-darwin` (macOS 出荷時) | Xcode Command Line Tools。最小デプロイ `MACOSX_DEPLOYMENT_TARGET` は Rust の既定 (aarch64: 11.0、x86_64: 10.12) で十分 (NFR-401 の 3 メジャーより古い) | ユニバーサル化は `lipo -create` |
| `*-unknown-linux-gnu` (Linux 出荷時、条件付き、OQ-112) | glibc 2.31 を保証するため **`cargo-zigbuild --target x86_64-unknown-linux-gnu.2.31`** でビルド (ホストの glibc に依存しない) | 新しいランナー (Ubuntu 24.04, glibc 2.39) で素直にビルドすると 2.31 環境で動かない。追加する場合は 09 の CI に zigbuild ジョブを足す |
| `*-unknown-linux-musl` (**Linux 出荷時の配布物**、v0.2) | `musl-tools` または zigbuild。**完全静的** (`-C target-feature=+crt-static` は musl で既定) | `getaddrinfo` 等の DNS は v1.L (reqwest) で musl 固有の制約 (nsswitch なし)。アロケータ §17.3 |

### 16.3 ビルドプロファイル

```toml
# release は実装済み (ルートの Cargo.toml)。dev は案
[profile.release]
opt-level = 3
lto = "fat"          # バイナリサイズ・起動時間 (NFR-101/205)。ビルド時間増は release のみ
codegen-units = 1
panic = "unwind"     # ADR-0012 (abort にしない)
strip = "symbols"    # 配布物は strip。クラッシュログのバックトレース用に split-debuginfo を別配布 (§18.6)
debug = "line-tables-only"  # バックトレースの行番号のため。strip と併用する場合は split-debuginfo=packed で .pdb/.dSYM/.dwp を別ファイルに

[profile.release.package."*"]
opt-level = 3

[profile.dev]
opt-level = 0
[profile.dev.package."*"]
opt-level = 2        # 依存は最適化 (ropey/regex のデバッグビルドは遅すぎてテストが回らない)
```

- **用途の限定**: `lto = "fat"` / `codegen-units = 1` の release プロファイルは **配布ビルドと BM 計測 (09 §5) だけ** に使う。CI の通常ジョブは dev プロファイルと `cargo check` で回し、ビルド時間の予算 (§17.2) を守る。
- **Windows (v0.1)**: `.pdb` は配布 zip に含めず GitHub Release の別アセット (サイズ)。

### 16.4 Windows 実行ファイルのメタデータ

- アプリケーションマニフェストを埋め込む (`embed-resource` または `winresource` クレート、ビルドスクリプト): `longPathAware=true`、`activeCodePage=UTF-8` (**付けない**: `GetACP()` が 65001 になり §6 の `system` ヒントが壊れる。Rust std は W API を使うので非 UTF-8 パスの問題も起きない)。DPI 認識の設定は埋め込まない (描画は端末が行い、evoeditor はウィンドウを持たない)。
- バージョンリソース (`FileVersion`, `ProductName`, `LegalCopyright`) を埋め込む (SmartScreen の評価・ユーザーの確認に有用)。
- サブシステム: `CONSOLE`。
- **DLL の検索順の制限 (01 NFR-502 AC-2 (a))**: 実行ファイルと同じフォルダ・カレントディレクトリに置かれた DLL を読み込まないよう、次の 2 つを両方行う。(1) リンク時のフラグ `/DEPENDENTLOADFLAG:0x800` (`LOAD_LIBRARY_SEARCH_SYSTEM32`。ビルドスクリプトの `cargo:rustc-link-arg-bins` で bin にだけ付ける) で、ローダーが静的インポートの DLL を `System32` からだけ探すようにする。(2) `main` の最初 (他のどの処理よりも前) で `SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32)` を呼び、実行時に読み込まれる DLL (遅延読込・依存クレートの `LoadLibrary`) の検索先も `System32` に限る。失敗しても起動は続け、情報ログに残す。
- **C ランタイムの静的リンク (`+crt-static`)**: `/DEPENDENTLOADFLAG:0x800` で静的インポートの DLL を `System32` からだけ探すと、`System32` に無い Visual C++ 再頒布パッケージの DLL (`VCRUNTIME140.dll` 等) を実行ファイルの隣に置いて読ませることもできなくなる。そのため MSVC の C ランタイムを静的にリンクし、再頒布パッケージに依存しない実行ファイルにする。リポジトリの `.cargo/config.toml` に次を置く (配布ビルド・CI・開発のビルドで同じにする):

  ```toml
  [target.x86_64-pc-windows-msvc]
  rustflags = ["-C", "target-feature=+crt-static"]
  [target.aarch64-pc-windows-msvc]   # v0.2 以降
  rustflags = ["-C", "target-feature=+crt-static"]
  ```

  `+crt-static` は MSVC の `/MT` 相当で、VC ランタイムと UCRT の両方を静的にリンクする (実行ファイルが読む DLL は OS の `System32` のものだけになる)。v0.4 で C のソースを持つ依存 (tree-sitter、`cc` クレート) を入れるときも、`cc` は `+crt-static` を見て `/MT` でビルドする。

  **`cargo install --git` の経路ではこの `.cargo/config.toml` が効かない場合がある**: `cargo install` は設定ファイルの探索をリポジトリ (取得したパッケージ) ではなく `$CARGO_HOME/config.toml` から始めるので (Cargo の文書の `cargo install` の「Configuration Discovery」)、リポジトリの `.cargo/config.toml` の `rustflags` は読まれない見込みである。そのため README のインストール手順は配布 zip を第一とし、`cargo install` の経路では環境変数 `RUSTFLAGS="-C target-feature=+crt-static"` を付けて実行するよう書く (§18.2a の手順 10)。インストール時に `.cargo/config.toml` が有効になる条件があるか (Cargo の版による違いを含む) と、`RUSTFLAGS` を付けたビルドで `dumpbin /dependents` に `VCRUNTIME*.dll` が無いことは、配布経路の検証 (09 §8.2) で確かめる (**要確認 (S-09)**)。
- **確かめ方 (各リリースの前。結果は 09 §8.2 のリリースの記録に残す。08 §2.2 のリリースゲート (8))**: (a) DLL の検索順: `dumpbin /loadconfig evoeditor.exe` で `DependentLoadFlags` が `0x800` であることを確かめる。加えて、Windows のテスト (`#[cfg(windows)]`) で起動後のロード済みモジュールを列挙し (`EnumProcessModules` + `GetModuleFileNameExW`)、OS の DLL がすべて `System32` (または WinSxS) から読み込まれていることを検査する。(b) UNC の資格情報の送出: UNC パス (`\\server\share\…`) を開いたとき、OS (SMB クライアント) が現在の利用者の資格情報 (NTLM 等) を相手のサーバーへ送り得ることは evoeditor 側で止められない。evoeditor が UNC へ自動で接続する経路 (起動時の最近使ったファイルの確認・ファイル監視・ワークスペースの走査) が v0.1 に無いこと (利用者が開くと指定したパスだけに接続すること) を確かめ、01 §4.6 の既知の制約に書くかどうかを判断して記録する。(c) C ランタイム: `dumpbin /dependents evoeditor.exe` に `VCRUNTIME*.dll` / `MSVCP*.dll` が無いことを確かめ、**Visual C++ 再頒布パッケージを入れていないクリーンな Windows 11** (S-09 で用意する検証環境、または Windows Sandbox) で配布 zip の実行ファイルが起動して文書を開き保存できることを確かめる。

---

## 17. 依存のライセンス・ビルド時間・サイズ

### 17.1 ライセンス運用 (ADR-0018 の具体化。検査は 09)

| 項目 | 方針 |
|---|---|
| 許可リスト (`deny.toml`) | **本表が公開リポジトリの許可リストの正本。確定 (ユーザー回答、2026-10-20)** (12 §3.6・ADR-0018・09・11 §9.6 は参照だけで再掲しない)。`MIT`, `Apache-2.0`, `Apache-2.0 WITH LLVM-exception`, `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, `Zlib`, `Unicode-3.0`, `Unicode-DFS-2016` (古い版の unicode-ident 等), `0BSD`, `BSL-1.0` (Boost Software License。Business Source License とは別物。バイナリでの配布に表示義務が無い寛容ライセンス。例: arboard の退路を採った場合の clipboard-win), `CC0-1.0` (データ・小規模クレート限定)、`MPL-2.0` (個別承認)、`OpenSSL`/`ISC` (`ring`/`aws-lc-rs`、v1.L で個別承認)。`OR` で結ばれた表記は、いずれかが許可リストにあれば通る。**fuzz/ に限る追加 (確定)**: `libfuzzer-sys` (cargo-fuzz) のライセンス表記は `(MIT OR Apache-2.0) AND NCSA` の見込み (**要確認**。導入の時点で crate の表記を確かめる) で、`AND` の NCSA が許可リストに無いと通らない。`fuzz/` は本体の workspace に含めない独立の workspace で配布物に入らない (02 §17.1) ので、`fuzz/` 側の `deny.toml` (または検査の例外) に限って `NCSA` を許す (本体の許可リストには足さない。確定 (ユーザー回答、2026-10-20)。表記の確認は上の **要確認** のとおり導入の時点で行う) |
| 既知の注意点 | **notify** のライセンス表記は最新版 (8.2.0 / 9.0.0-rc) では **`CC0-1.0` のみ** (それより前の版は `CC0-1.0 OR Artistic-2.0`) → CC0-1.0 を許可リストに含める (上の行)。**tree-sitter 文法** は MIT が多いが Apache-2.0/独自もあり、`queries/*.scm` の流用元 (nvim-treesitter は Apache-2.0、Helix は MPL-2.0!) に注意 → ハイライトクエリは **自前作成または MIT/Apache 由来のみ**。**unicode-width / unicode-segmentation** は Unicode データライセンスを含む。**ring** (rustls 既定) は複合ライセンス → TLS の暗号バックエンドは v1.L で決める (未決。候補は `aws-lc-rs` / `rustls-rustcrypto`)。**依存の保守状況**: `regex-cursor` は v0.1 では使わない (03 §12 の検索は行単位。採否は v0.3 の S-04)。プロセス木の終了の実装手段は §13.4 (未決、v0.4) |
| CI で使う道具 | 配布物に入らない CI の道具も、導入するときに上の許可リストと照らす (`deny.toml` の検査は Cargo の依存だけが対象なので、手で確かめて導入の PR に記録する)。**age** (fuzz のクラッシュ入力の暗号化、09 §6.3 / 11 §9.7) のライセンスは BSD-3-Clause の想定 (**要確認**。導入の時点で配布元のライセンスを確かめる) で、この確認の対象に含める |
| 第三者告知 | MIT/BSD は著作権表示の複製が義務 → `cargo about` で `THIRD-PARTY-NOTICES.md` を生成し **配布アーカイブに同梱** する (v0.1)。`evoeditor --licenses` (同梱ファイルの表示) は v0.2 (v0.1 はアーカイブに同梱した `THIRD-PARTY-NOTICES.md` で足りる。07 §18.3) |
| 本体 (公開コア) | **Apache-2.0 単独** (確定 (ユーザー回答)、OQ-19 / 152)。既存の `LICENSE` と `NOTICE` を配布物に同梱し、各 crate は `license = "Apache-2.0"`。TUI は公開・OSS。Apache-2.0 の含意 (特許条項・NOTICE) は 12 §3.2 が正本 |
### 17.2 依存数・ビルド時間の予算 (Should。計測は 09)

| 指標 | v0.1 目標 | 根拠 |
|---|---|---|
| 推移的依存クレート数 (bin、既定 feature、normal 依存。v0.1 は `x86_64-pc-windows-msvc` で数える。Linux は出荷時に加える) | **≤ 90** (NFR-207、Should。v0.1 はリリースゲートにせず報告) | ratatui + crossterm + regex + encoding_rs + chardetng + ropey + serde/toml + tracing (マクロのみ) + windows-sys (Unix の libc / signal-hook は v0.2) + ハッシュ (§5.5) で約 70〜85。v0.1 で **含めない** もの: arboard (§8.2)、tracing-subscriber / tracing-appender (§7.3)、regex-cursor (03 §12)、notify / keyring / tokio (後続版)。`cargo xtask check-deps` で数を報告し、C 依存 0 を CI で検査 (09 §6) |
| クリーン release ビルド (基準環境、LTO fat) | ≤ 3 分 | 貢献者体験・CI コスト |
| インクリメンタル dev ビルド (`eve-core` の 1 行変更 → `cargo test -p eve-core` 開始まで) | ≤ 5 秒 | 開発速度 |
| `cargo check` クリーン | ≤ 45 秒 | — |
| 配布バイナリサイズ | NFR-205 (v0.1 ≤ 8 MB、v0.4 ≤ 30 MB) | `cargo bloat` で監視 |
| C コードを含む依存 (v0.1) | **0** (NFR-207。v0.1 のリリースゲート) | Windows ビルドの容易さ、musl 静的リンクの単純さ (Linux 出荷時) (v0.4 で tree-sitter が初登場し、その時点で NFR-207 の値を見直す)。`blake3` の既定 feature は C/asm を含むため、ハッシュは純 Rust の XXH3-64 (twox-hash、§5.5) を使う |

依存追加時のチェックリスト ([02 §13.0](02-architecture.md#130-依存追加の評価基準)) に「`cargo tree -d` で重複版が増えないか」「`cargo build --timings` でクリティカルパスが伸びないか」を加える。

### 17.3 アロケータ

既定はシステムアロケータ (02 §13.7)。v0.1 (Windows のみ) では判断は要らない。**Linux を出荷するとき (v0.2) の判断規則** を次に決めておく (OQ-112 / OQ-80): musl の malloc は遅い (ロック競合・断片化) が、Linux の配布物は musl 版を基本にするので (§1.1)、 BM-104 (大きなファイルを開く) が gnu 版比 **1.5 倍以上** 遅い、または BM-202 (開いた後のメモリ) が **1.3 倍以上** なら、アロケータを替えるのではなく **gnu 版 (zigbuild、glibc 2.31) を配布物に追加** する。C 依存のアロケータは使わない (NFR-207)。純 Rust のアロケータへの変更は未決 (v0.4)。

---

## 18. 配布 (パッケージ・署名・公証)

### 18.1 段階 (OQ-20)

| 版 | チャネル |
|---|---|
| v0.1 | **GitHub Releases (Windows x86_64 のアーカイブ `evoeditor-0.1.0-x86_64-pc-windows-msvc.zip` + `SHA256SUMS` + minisign 署名) と `cargo install --locked --git <repo> --tag v0.1.0 evoeditor` のみ** (OQ-20。crates.io への公開は 1.0 の前 (確定 (ユーザー回答)、10 の残件 #1。名前の空きは v0.1 のリリース前と各リリースの前に照会する)。Homebrew / Scoop / AUR / winget / binstall の各チャネルは bot / formula の保守を伴い v0.1 の工数と品質ゲートを重くするため後送り)。macOS / Linux のアーカイブは **v0.1 では出荷しない** (v0.2 から。下の v0.2 の行)。Windows aarch64 のアーカイブは v0.2 以降 (Tier B)。`cargo install` には常に **`--locked`** を付ける (付けないと Cargo.lock を無視して依存の新しい版を解決し、MSRV の不一致でビルドが壊れたり、配布物と違う依存で動いたりする)。利用者の環境の要件: Windows、rustc ≥ MSRV (§16.1)、MSVC Build Tools (v0.1 の `cargo install` も Windows だけを対象にする。macOS / Linux でのビルドは保証しない) |
| v0.2 | v0.1 のチャネルに、**macOS aarch64 の tar.gz** (ad-hoc 署名、§18.4、OQ-76) と **Linux x86_64 / aarch64 の musl 静的版の tar.gz** (OQ-112、§1.1) を加える (いずれも `SHA256SUMS` + minisign)。macOS x86_64 と Windows aarch64 は Tier B (ビルドのみ)。確定 (ユーザー回答)、OQ-15。出荷には実機確認の手段 (基準機の追加、10 OQ-23 (d)) の確保が前提で、確保できない OS は次の版に延期する (確定 (ユーザー回答)、10 の残件 #3)。`cargo install --locked --git` は v0.2 から macOS / Linux も対象にする |
| v0.4 | (macOS / Linux 向けのチャネルは、その OS を出荷していることが前提) `cargo binstall` 対応メタデータ、Homebrew **tap** (自前)、Scoop **bucket** (自前)、AUR (`-bin` と source)、winget (microsoft/winget-pkgs へマニフェスト PR)、deb/rpm (`cargo-deb` / `cargo-generate-rpm` で生成し GitHub Release に添付)、Nix (nixpkgs)、Windows コード署名 (§18.3)、SBOM (cyclonedx)、man ページ、シェル補完スクリプト。Homebrew core は著名度条件 (30 fork / 30 watcher / 75 star 相当) を満たしてから |
| 後続 | macOS の Developer ID 署名と公証 (§18.4。時期は未決で、取得時期と TUI への適用は OQ-76 で決める)、Debian/Fedora 公式リポジトリ (Rust 依存の個別パッケージ化が必要で重い。コミュニティ任せ)、Flatpak/Snap は **やらない** (サンドボックスがファイルアクセス・端末と相性悪い) |

### 18.2 アーカイブ構成

```
evoeditor-<version>-<target>.{zip|tar.gz}
├── evoeditor[.exe]
├── LICENSE                        ← Apache-2.0
├── NOTICE                         ← Apache-2.0 第 4 条の帰属表示 (12 §3.2)
├── THIRD-PARTY-NOTICES.md        ← v0.1 (MIT/BSD の著作権表示義務)
├── README.md                      ← 利用者向け (v0.1 は日本語、OQ-21)
├── config.example.toml           ← v0.1 (公開キーと既定値の注釈付きの例。07 §5.0 から生成し、CI で既定値と突合、09 §11.2)
├── completions/ (evoeditor.bash, evoeditor.zsh, evoeditor.fish, _evoeditor.ps1)   ← v0.4。lexopt のため手書き (補完対象は少ない)
└── man/evoeditor.1 (Unix)              ← v0.4
```

- Windows (v0.1): `x86_64` の zip のみ (aarch64 は v0.2 の Tier B)。
- Linux (v0.2、Linux 出荷時): `x86_64` / `aarch64` の `-musl` を基本にする (OQ-112)。gnu 版 (glibc 2.31、zigbuild) は OQ-112 の条件が成立したとき、または deb / rpm を作る v0.4 に追加し、deb / rpm は gnu 版から生成する。
- macOS (v0.2、macOS 出荷時): `aarch64` のアーカイブだけを出す (ad-hoc 署名。Developer ID 署名と公証の時期は未決、OQ-76)。`x86_64` は Tier B (ビルドのみ) でアーカイブを出さない (§18.1。ユニバーサルは §19)。
- Windows: zip のみ (MSI/MSIX は不要。winget は portable 型で zip を扱える)。実行ファイルは `evoeditor.exe`。
- パッケージ名 (v0.4 以降のチャネル): Homebrew / Scoop は `evoeditor`、AUR は `evoeditor` と `evoeditor-bin`、deb / rpm は `evoeditor`、winget の識別子は `<発行者>.evoeditor` (OQ-01)。執筆時点 (2026-10) でこれらの名前は各チャネルで未使用 (調査は 10 の OQ-01)。
- インストーラスクリプト (`curl | sh`、`irm | iex`) は **v0.1 では提供しない** (セキュリティ上の評判、検証困難)。v0.4 で `cargo-dist` 等の生成物として検討 (§19)。
- `NOTICE` はリポジトリ直下に M0e (最初の push の前) で作り (08 §4)、リリースのビルドがアーカイブにそのまま入れる。`config.example.toml` は設定ファイルの雛形で、evoeditor は起動時に `config_dir` を作らない (§7.1) ので、使う人が `%APPDATA%\evoeditor\config.toml` へ写して編集する。

### 18.2a v0.1 のインストール手順 (Windows)

利用者向け README の「インストール」節 (09 §11.3。v0.1 は日本語、OQ-21) に書く手順の正本。v0.1 は Windows 11 の x86_64 だけを対象にし (§1.1)、インストーラを持たない (§18.8)。PowerShell の例は Windows 11 に付属する Windows PowerShell 5.1 と PowerShell 7 のどちらでも動く書き方にする。

| # | 手順 | 内容 |
|---|---|---|
| 1 | 取得 | GitHub Releases の v0.1.0 のページから `evoeditor-0.1.0-x86_64-pc-windows-msvc.zip`・`SHA256SUMS`・`SHA256SUMS.minisig` の 3 つを同じフォルダにダウンロードする |
| 2 | 検証 | §18.9 の手順で、(1) minisign で `SHA256SUMS` の署名を検証し (`minisign -Vm SHA256SUMS -P <公開鍵>`。公開鍵は README と `minisign.pub` に載せたもの)、(2) `Get-FileHash -Algorithm SHA256 .\evoeditor-0.1.0-x86_64-pc-windows-msvc.zip` の値が `SHA256SUMS` の該当行と一致することを確かめる。minisign は公式の配布 (作者の GitHub の Releases の Windows 向けビルド) から入手する (パッケージマネージャでの入手方法は README を書くときに確かめる、**要確認**)。**一致しなければ以降の手順に進まない** |
| 3 | ブロックの解除 (Mark of the Web) | ブラウザでダウンロードしたファイルには「インターネットから来た」印 (代替データストリーム `Zone.Identifier`) が付き、エクスプローラーで展開すると中の `evoeditor.exe` にも引き継がれて SmartScreen の確認の対象になる。**手順 2 の検証が済んだ後で、展開する前に** zip に対して解除する: `Unblock-File .\evoeditor-0.1.0-x86_64-pc-windows-msvc.zip`、またはエクスプローラーで zip のプロパティを開き「全般」の「セキュリティ」の **「許可する」** にチェックを入れて OK。検証していないファイルの印は外さない |
| 4 | 展開先 | 利用者ごとのフォルダ **`%LOCALAPPDATA%\Programs\evoeditor\`** に展開する (管理者権限が要らない。PowerShell なら `Expand-Archive .\evoeditor-0.1.0-x86_64-pc-windows-msvc.zip -DestinationPath "$env:LOCALAPPDATA\Programs\evoeditor"`)。ダウンロードのフォルダに置いたまま使わない (誤って消しやすく、更新の手順 §18.10 で置き換える場所が定まらない)。`C:\Program Files` は管理者権限が要り、更新のたびに昇格が要るので勧めない。展開した `LICENSE`・`NOTICE`・`THIRD-PARTY-NOTICES.md` は消さずに置いておく |
| 5 | PATH への追加 | 利用者の環境変数 `Path` に展開先を加える。画面から: スタートメニューで「環境変数」を検索 →「環境変数を編集」(ユーザーの環境変数) → `Path` を選んで「編集」→「新規」で `%LOCALAPPDATA%\Programs\evoeditor` を加えて OK。PowerShell から: `$p = [Environment]::GetEnvironmentVariable('Path', 'User'); [Environment]::SetEnvironmentVariable('Path', "$p;$env:LOCALAPPDATA\Programs\evoeditor", 'User')`。`setx` は使わない (1024 文字で切り詰め、システムの `Path` と混ぜて書き戻すことがある)。追加の後は **端末を開き直す** (開いている端末には反映されない) |
| 6 | 確認 | 新しい端末 (Windows Terminal) で `evoeditor --version` を実行し、版が表示されることを確かめる |
| 7 | SmartScreen | v0.1 の exe はコード署名が無い (§18.3、署名は v0.4、OQ-75)。手順 3 を済ませていれば通常は確認が出ない。エクスプローラーから exe を直接開いたときなどに「Windows によって PC が保護されました」が出た場合は、手順 2 の検証が済んでいることを確かめたうえで「詳細情報」→「実行」を選ぶ |
| 8 | Smart App Control | Windows 11 の Smart App Control が **オン** の環境では、署名も評判も無い exe は起動の方法 (端末からを含む) に関わらず拒否され得る (§18.3)。v0.1 では回避の手段を用意しない: Smart App Control を無効にする手順は案内しない (セキュリティ機能の無効化を手順にしない)。既知の制約 (01 §4.6) としてリリースノートと README に書き、コード署名 (v0.4、OQ-75) で解消する。評価モードの環境での挙動と、`cargo install` でビルドした exe の扱いはリリース前に確かめて README に書く (**要確認**。リリースごとの配布経路の検証 (09 §8.2 の「配布経路の検証」の (3)) で確かめ、結果をリリースのチェックリストに記録して解消する) |
| 9 | ウイルス対策ソフトの誤検知 | Microsoft Defender などが exe を誤って検出した場合は、隔離から戻す前に手順 2 の検証をやり直す。誤検知は Microsoft の提出窓口 (ファイルの提出) と、リポジトリの Issues (12 §5.6 の PC-7) で知らせてもらう |
| 10 | `cargo install` の経路 | **第一の手順は上の配布 zip** で、README でもこの順に書く (`cargo install` は Rust の開発環境がある利用者向けの第二の経路)。Rust を rustup で入れ (MSVC ツールチェイン)、Visual Studio Build Tools の「C++ によるデスクトップ開発」を入れてから、C ランタイムを静的にリンクするため環境変数 `RUSTFLAGS` を付けて `cargo install --locked --git <repo> --tag v0.1.0 evoeditor` を実行する (§18.1。PowerShell なら `$env:RUSTFLAGS = "-C target-feature=+crt-static"` を設定してから実行する。`cargo install` はリポジトリの `.cargo/config.toml` を読まない場合があり、付けないと Visual C++ 再頒布パッケージを要する exe になり得る。§16.4、要確認 (S-09))。実行ファイルは `%USERPROFILE%\.cargo\bin` に入り、rustup がこのフォルダを `Path` に加えている。よくある失敗と案内: `link.exe` が見つからない → Build Tools を入れる。rustc の版が古いというエラー → `rustup update` (MSRV は §16.1、リリースノートに書く)。依存の解決のエラー → `--locked` を付けたかを確かめる |
| 11 | アンインストール | §18.8 |

- 更新は §18.10、直前の版に戻すのは §18.12。どちらも手順 2〜4 を使う (展開先は同じフォルダで、evoeditor をすべて終了してから置き換える)。

### 18.3 Windows コード署名

| 方式 | 費用 / 条件 | 方針 |
|---|---|---|
| 署名なし | 0 | v0.1 の既定。ブラウザでダウンロードした zip からの初回実行時に SmartScreen の警告が出る。**Windows 11 の Smart App Control** が有効な環境では、署名も評判も無い exe は起動方法 (ターミナルからを含む) に関わらずブロックされ得る。**コントロールされたフォルダー アクセス** が有効だと、未署名の exe は保護されたフォルダ (ドキュメント等) に保存できない。Defender の誤検知 (Rust の小さな exe は "Trojan:Win32/Wacatac" 等と誤判定される実績多数) にも無防備。いずれも 01 §4.6 の既知の制約としてリリースノートに書く (利用者向けの手順と案内は §18.2a) |
| SignPath Foundation (OSS 向け無償) | OSS プロジェクトの審査 | **v0.4 で採る案** (未決、OQ-75)。GitHub Actions から署名を依頼できる |

コード署名はリリース CI で行い、証明書・API キーは保護された `environment` の秘密情報にだけ置く (11 §9.7)。**minisign 署名は署名の有無に関わらず常に付ける** (ユーザーが検証できる一次手段。鍵の扱いは §18.9)。

### 18.4 macOS 署名と公証

**v0.2 (macOS 出荷時) の仕様**。v0.1 では macOS を出荷しないので行わない。

| 状況 | Gatekeeper の挙動 | 方針 |
|---|---|---|
| ad-hoc 署名 (`codesign -s -`。Apple Silicon ではリンカが自動付与) | **ブラウザで DL** → `com.apple.quarantine` が付き、macOS 15 以降は「開く」バイパスが難しく、システム設定 → プライバシーとセキュリティ → 「このまま開く」が必要。**`curl`/Homebrew/`cargo install` 経由は quarantine が付かず問題なし** | macOS 出荷時の既定。README に `xattr -d com.apple.quarantine evoeditor` と Homebrew 推奨を明記 |
| Developer ID 署名 + 公証 (`notarytool`) + ステープル | 警告なし | Apple Developer Program 年 99 USD。取得時期と TUI に適用するかは未決 (OQ-76)。TUI 単体バイナリは `.app` でなく実行ファイル単体でも公証できる (zip で提出) が、単体の実行ファイルには公証をステープルできないので、初回の起動時はオンラインでの確認になる |
| Hardened Runtime | 公証に必須。TUI は JIT 等を使わないため問題なし | — |
| Keychain アクセス (§14) | ad-hoc 署名は更新ごとに ID が変わり、毎回ダイアログ | Developer ID 署名の動機 |

### 18.5 Linux パッケージ

**v0.2 (Linux 出荷時) の仕様** (deb / rpm 等のチャネルは §18.1 の v0.4)。

| 形式 | ツール | 備考 |
|---|---|---|
| deb | `cargo-deb` | `Depends` は glibc のみ (`libc6 (>= 2.31)`)。man ページ・補完を含める。`conffiles` なし (設定は `~/.config`) |
| rpm | `cargo-generate-rpm` | `Requires: glibc >= 2.31`。SELinux ラベルは標準 bin で問題なし |
| AUR | PKGBUILD (`evoeditor-bin`, `evoeditor`) | コミュニティ管理可 |
| Nix | nixpkgs `buildRustPackage` | `cargoHash` の更新が必要。コミュニティ管理 |
| Homebrew (Linux) | tap が Linux でも動く | — |

### 18.6 デバッグ情報とクラッシュログの対応付け

配布バイナリは strip するが、各リリースの **分割デバッグ情報** (v0.1 は Windows の `.pdb`。macOS / Linux 出荷時に `.dSYM.zip` / `.debug` (DWARF `objcopy --only-keep-debug`)) を GitHub Release に別アセットとして添付し、クラッシュログ (09 §10) のバックトレースをメンテナが `addr2line` 等で解決できるようにする。ビルド ID (`.note.gnu.build-id` / PDB GUID / Mach-O UUID) をクラッシュログに記録する。`--remap-path-prefix` でビルドパスを消す (再現ビルド・プライバシー)。

### 18.7 再現可能ビルド (Should, v0.4)

`SOURCE_DATE_EPOCH`、`--remap-path-prefix`、固定ツールチェイン、`Cargo.lock` により、同一コミットから同一バイナリ (ハッシュ一致) が得られることを CI で 2 回ビルドして確認する (09)。Windows の PE タイムスタンプは `/Brepro` (MSVC リンカ) で固定。

### 18.8 アンインストール (提案、OQ-164)

evoeditor はインストーラを持たず、**レジストリ・システム設定・サービスを書かない** (Windows も含む)。したがってアンインストールは「実行ファイルを消す」(`cargo install` なら `cargo uninstall evoeditor`、アーカイブなら §18.2a の展開先のフォルダ (`%LOCALAPPDATA%\Programs\evoeditor\`) を削除し、利用者の環境変数 `Path` からそのフォルダを外す) と、必要なら次のディレクトリを手で消すことである。v0.1 (Windows) の既定の場所は `%APPDATA%\evoeditor` (`config_dir`) と `%LOCALAPPDATA%\evoeditor` (`state_dir` / `data_dir` / `cache_dir` の親) で、内訳は: `config_dir` (設定)、`state_dir` (`log/`・`recovery/`・`swap/`・`backup/`・`session/`・検索履歴・`last_version.toml` (前回起動した版の記録、v0.2、07 §18.8))、`data_dir`・`cache_dir` (v0.4 以降)、§7.5 の代替の退避先 (`%TEMP%\eve-recovery-<SID>\`。macOS / Linux 出荷時は `$TMPDIR/eve-recovery-<uid>/`)、ポータブルモードの `<exe_dir>/data/`、v1.L 以降は資格情報ストアに保存した API キー (§14)。macOS / Linux の実パス (v0.2) を含め、各 OS の実パスは §7.1 (環境変数で上書きしていればその場所)。**`recovery/` と `swap/` には未保存の編集内容が残っていることがある** ので、消す前に中を確認するよう README の手順に書く。evoeditor 自身にディレクトリを消すコマンドは設けない (誤操作で退避を失わないため)。

### 18.9 minisign 署名の検証 (提案、OQ-164)

リリースの `SHA256SUMS` には minisign の署名 (`SHA256SUMS.minisig`) を付ける (§18.1)。**公開鍵** は (1) リポジトリの `README.md` と、(2) リポジトリ直下の `minisign.pub` に置き、両者が一致することを利用者が確かめられるようにする (鍵をリリースのアセットだけに置かない: アセットと鍵を同時に差し替えられると検証にならない)。**秘密鍵** は CI に置かず、メンテナの手元 (オフライン、パスフレーズ付き) に置き、控えを別の媒体に保管する。リリースの手順: CI が作ったアーカイブと `SHA256SUMS` をメンテナが取得し、ハッシュを照合してから手元で `SHA256SUMS` に署名し、`SHA256SUMS.minisig` を Release に添付する (minisign は鍵を持たない署名 (OIDC) に対応しないので、CI に置くと、GitHub のアカウントか CI が侵害されたときに成果物と署名を同時に差し替えられる)。公開鍵は README と `minisign.pub` に加えて、リポジトリの外の経路 (メンテナの別のサイト等) でも公開する。リリースのタグには署名し、照合用のコミット SHA をリリースノートに載せる。鍵を更新するときは、旧鍵で新しい公開鍵に署名したものを公開し、README に更新日を書く。利用者の検証手順 (v0.1 は Windows): `minisign -Vm SHA256SUMS -P <公開鍵>` で `SHA256SUMS` を検証し、次に PowerShell の `Get-FileHash -Algorithm SHA256 <zip>` の値と `SHA256SUMS` の該当行を比べる。macOS / Linux 出荷時 (v0.2) は `shasum -a 256 -c SHA256SUMS --ignore-missing` (macOS) / `sha256sum -c SHA256SUMS --ignore-missing` (Linux) を README に加える。minisign の入手方法 (v0.1 は Windows 向けの公式のビルド等。出荷時に各 OS のパッケージマネージャ) も README に書く。この手順は更新 (§18.10) と直前の版に戻すとき (§18.12) にも使う。

### 18.10 更新の方針 (ADR-0040)

evoeditor (TUI) は **エディタ内に更新機能を持たない** ([ADR-0040](10-decisions.md#adr-0040-更新の方針-エディタ内に更新機能を持たない)、確定 (ユーザー回答)。v0.1〜v1.0)。更新の確認・ダウンロード・置換を行わず、既定状態の通信ゼロ (NFR-501) を保つ。利用者は配布経路ごとの手段で更新する。

| 配布経路 | 更新の手順 | 版 |
|---|---|---|
| GitHub Releases (アーカイブ) | (1) 新しい版のアーカイブ (v0.1 は `evoeditor-<version>-x86_64-pc-windows-msvc.zip`) と `SHA256SUMS`・`SHA256SUMS.minisig` をダウンロードする (2) §18.9 の手順で署名とハッシュを検証する (3) evoeditor をすべて終了してから、展開した `evoeditor.exe` で古い実行ファイルを置き換える (Windows は実行中の exe を置き換えられない。macOS / Linux 出荷時は `evoeditor` を同様に置き換える) (4) `evoeditor --version` で版を確かめる。使っていた版のアーカイブは §18.12 のために残しておく | v0.1〜 |
| `cargo install --git` | `cargo install --locked --git <repo> --tag <新しい版> evoeditor` を再実行する (同じ名前の実行ファイルを置き換える。cargo が求めれば `--force` を付ける) | v0.1〜 |
| crates.io | `cargo install --locked evoeditor` を再実行する。**crates.io に公開した時点から** の経路 (時期は OQ-20 で確定 (ユーザー回答): 1.0 の前)。公開するまで README には書かない | 公開後 |
| パッケージマネージャ | (macOS / Linux 向けはその OS の出荷後) Homebrew (`brew upgrade evoeditor`)、Scoop (`scoop update evoeditor`)、winget (`winget upgrade <発行者>.evoeditor`)、AUR (AUR ヘルパーでの更新)、deb / rpm (Release に添付した新しいパッケージを `apt install ./<ファイル>.deb` / `dnf install ./<ファイル>.rpm`)、`cargo binstall evoeditor` の再実行。各チャネルの正確なコマンドは登録したときに README に書く | v0.4〜 |

- 更新の手順は **リリースノート** (各版の Release 本文に「更新の方法」の短い節と、その版で要る移行手順 (設定キーの改名など) を書く) と **利用者向け README の「更新」節** (v0.1 の成果物、01 §4.2) に書く。README の「更新」節には、本節の表・§18.11 (更新とダウングレード)・§18.12 (直前の版に戻す) の利用者向けの要約を置く。手順は配布の方法 (OQ-20) の結果に依らない書き方にし、crates.io に公開した時点で `cargo install --locked evoeditor` の行を加える。
- 新しい版は、GitHub の Watch (Releases) またはパッケージマネージャで知る。脆弱性の通知は §18.14。
- 利用者が明示的に実行したときだけ版情報を取得する `evoeditor --check-update` は [ADR-0041](10-decisions.md#adr-0041-明示実行の更新確認-版情報の取得のみ) (採るかどうかは提案中、v0.4 以降。コマンド名は確定 (ユーザー回答))。採るまでは本節の手順だけを使う。
- 新しい版で初めて起動したときの要点の表示 (FR-518、v0.2) は通信せず、同梱の文言と状態ファイルの記録だけで行う ([07 §18.8](07-config-extensibility.md#188-更新に関わる予約---check-update-と前回起動した版の記録))。

### 18.11 更新とダウングレード

新しい版・古い版の間での設定と状態の扱い。利用者向けに README の「更新」節に要約する (ADR-0040 決定 5 (a))。

| 対象 | 新しい版に更新したとき | 古い版に戻したとき (ダウングレード) |
|---|---|---|
| 設定ファイル (`config.toml` 等) | そのまま読める。0.x の間はマイナー版でキーの改名・廃止が起こり得るので、CHANGELOG とリリースノートに移行手順を書く (09 §9.1) | 古い版が知らないキーは **警告して無視** し、ほかのキーは使う (FR-703)。evoeditor は設定ファイルに書き込まないので、ファイルは変わらない |
| 緊急退避・スワップ (`recovery/`・`swap/`) | 読める (新しい版は古い版の形式を読む) | v0.1 の起動時の案内は manifest も本文も読まず、版に依らず退避の場所だけを示す (`msg.recovery.found` / `found_rescue`、02 §9.3)。manifest を読む v0.2 以降では、`format_version` が古い版の知る値より大きければ、**本文を読まず、退避の場所と「この退避は新しい版の evoeditor で書かれています。新しい版で開くか、`.txt` を直接開いてください」を示すだけ** にする (`msg.recovery.unknown_format`)。どの版でも**移動も削除もしない** (§7.5 (a)、[02 §11.2.1](02-architecture.md#1121-緊急退避の方式-正本adr-0023))。30 日を超えた退避の削除の確認 (v0.2) の対象にもしない |
| その他の状態ファイル (セッション・`history`・`trust.toml`・`last_version.toml` 等) | 読める | 未知の版は読まずに `<name>.unsupported-<ts>` へ退避して警告する (§7.5 (a))。削除しないので、新しい版に戻したあと手で元の名前に戻せば読める |
| ログ・クラッシュログ | 影響なし | 影響なし |

利用者への説明の要点: ダウングレードしても利用者のデータ (退避・スワップ・設定ファイル) は消えない。古い版で読めない退避は、新しい版で開くか本文の `.txt` を直接開く。検査は 09 §3.4 (未知の `format_version` の退避を移動・削除しないこと)。

### 18.12 直前の版に戻す (ロールバック)

自動のロールバックは実装しない (ADR-0040 決定 5 (c))。利用者向けの手順 (README の「更新」節に書く):

1. 更新する前に、使っていた版のアーカイブと `SHA256SUMS`・`SHA256SUMS.minisig` を手元に残す。残していなければ GitHub Releases から同じ版を取り直す (公開済みのバイナリは削除しない、09 §9.4)。
2. 戻す前に §18.9 の手順で署名とハッシュを検証する。
3. evoeditor をすべて終了してから実行ファイルを置き換え、`evoeditor --version` で確かめる。`cargo install --git` なら `--tag <戻す版>` で再実行する。パッケージマネージャは、それぞれが持つ版の固定・戻しの手段があればそれを使う。
4. 戻した後のデータの扱いは §18.11 の「古い版に戻したとき」の列のとおり。

### 18.13 リリースチャネル

v0.x の間は **stable 1 本** (ADR-0040 決定 6)。プレリリース (`-alpha.N` / `-beta.N` / `-rc.N`) を GitHub Releases の pre-release として出すことはあるが (09 §9.1)、継続的な β チャネルとしては扱わず、パッケージマネージャには流さない。β チャネルを設けるかは v1.0 のリリース前に決める ([OQ-166](10-decisions.md#28-oq-161-以降-仕上げ段階の追加起票)、提案中)。

### 18.14 脆弱性の通知 (OQ-164)

更新機能が無いので、エディタからは脆弱性を知らせない (ADR-0040 決定 7)。通知は **`SECURITY.md`** (報告の窓口・開示の方針・**更新方法の案内** (README の「更新」節への導線と、修正版の入手と検証の手順)) と **GitHub Security Advisory** で行い、修正版のリリースノートの冒頭にも書く。利用者には GitHub の Watch (Releases と Security alerts) を案内する。`SECURITY.md` の最小の版 (報告の窓口と開示の方針) と Issues・GitHub の private vulnerability reporting は最初の push の前に用意し (12 §5.6 の PC-7)、更新方法の案内は v0.1 のリリース前に加える (OQ-164、提案中)。

---

## 19. 未決事項

番号は [10 §2](10-decisions.md#2-未決事項-open-questions) の OQ 番号。区分・詳細は 10 を正とし、本表は本書側の結論 (確定) または結論案 (未決) を記す。確定済みの項目は本書の決定の根拠として残す。P0 検証項目 (S-nn) の正本は 08 §3。

| OQ | 件名 | 結論 / 結論案 | 区分 / 決定期限 |
|---|---|---|---|
| OQ-15 | 対応 OS/端末の範囲 (Intel Mac を Must のままにするか Tier B に下げるか、を含む) | **v0.1 の対象は Windows 11 の Windows Terminal と conhost** (確定 (ユーザー回答)。§1.2 / §9.4。Windows 10 は公式には対応しない、§1.1)。macOS / Linux は v0.1 では出荷せず、**v0.2 で出荷する** (確定 (ユーザー回答))。v0.2 では Intel Mac (`x86_64-apple-darwin`) と Windows aarch64 を **Tier B (ビルドのみ)** にする (CI の x86_64 macOS ランナーが減少・有料化傾向。Tier B のビルドは v0.2) | 確定 (ユーザー回答) |
| OQ-70 | macOS ユニバーサルバイナリを追加提供するか | Homebrew 向けのみ提供 (v0.4)、GitHub Release はアーキ別 | 裁量 / v0.2 着手前 (macOS 出荷前) |
| ADR-0021 | クリップボードを Port ではなく Effect 経路で扱う | Effect 経路 (§2.8、02 §4.4 / §6.3)。`ClipboardText::Handle` (`Editor::take_clipboard` で取り出す `ClipboardSource`) の文字列化はワーカー | 確定 (ユーザー回答) |
| OQ-04 | 新規文書の既定改行を `lf` 固定にするか `native` にするか | `lf` (P6 / NFR-405。07 §5.2) | 確定 (ユーザー回答) |
| OQ-71 | 設定ディレクトリの Windows 既定 (`%APPDATA%\evoeditor` vs `%USERPROFILE%\.config\evoeditor`) | `%APPDATA%` + `XDG_CONFIG_HOME` 尊重 | 確定 (ユーザー回答) |
| OQ-72 | ポータブルモードの提供 | v0.4 Could | 裁量 / v0.4 |
| OQ-73 | ハードリンクファイルの既定 (`Ask` / `WriteInPlace`) | `Ask` (07 §5.2 `file.hardlink_policy`、02 §4.5 `SaveOptions.hardlink`) | 確定 (ユーザー回答) |
| OQ-74 | `cargo-dist` 等のリリースツール採用 (保守状況の確認要) vs 手書き GitHub Actions | v0.1 は bot を使わず、版上げ・CHANGELOG・昇格 PR は人間が行い、配布ビルドは手書き workflow。release-plz と cargo-dist は v0.2 以降の候補 | 裁量 / v0.1 リリース前 |
| OQ-75 | Windows コード署名の手段と時期 | v0.4、SignPath Foundation (OSS 向け無償、§18.3) | 裁量 / v0.4 |
| OQ-76 | macOS の署名と公証 (Developer ID の取得時期) | v0.2 の macOS 版は ad-hoc 署名で出荷する (§18.4。README に quarantine の解除手順)。Developer ID の取得時期と公証の TUI への適用は未決 (12 §5.2) | 裁量 / v0.2 着手前 |
| OQ-77 | `FsyncLevel` の既定を全環境 Full にするか、`FsKind::Network` では FileOnly にするか | Full (P1 原則。遅ければ設定 `file.fsync` で緩める) | 裁量 / G1c (v0.1 の M3 中。S-10 は v0.2 なので待たない) |
| OQ-78 | ファイル監視不可 FS でのポーリングを設定で許可するか (NFR-204 との整合) | 明示設定時のみ許可、最小 2 s | 裁量 / v0.4 着手前 |
| OQ-79 | X11 クリップボード所有権の終了時待機 (既定 200 ms) | v0.1 は外部コマンド (`xclip` / `xsel` がデーモン化して保持) に委ねるので **不要**。arboard を退路として採った場合 (OQ-111) のみ 200 ms、設定可 | 裁量 / arboard 採用時 |
| OQ-80 | musl 版でのアロケータ (`mimalloc` は C 依存) | システム既定のまま。遅ければアロケータではなく gnu 版を追加 (§17.3、OQ-112 の判断規則) | 裁量 / v0.2 着手前 (Linux 出荷前) |
| OQ-34 | 一時ファイル (`$TMPDIR` 直下、`crontab` / `kubectl edit` 等) の in-place フォールバック規則 | **確定 (ユーザー回答)**: §4.4 の判定規則 (正準化後の一時ディレクトリの **直下** だけを対象にし、ファイル名をパターン表 `file.in_place_patterns` と照合)。確認なし | 確定 (ユーザー回答) |
| S-11 | Windows の終了イベントでの退避 | v0.1 は CTRL_CLOSE・パニック・メインがブロックした状態の CTRL_CLOSE の 3 経路で退避の到達を確かめる (§11.2)。CTRL_CLOSE の猶予 (約 5 秒) が利用者設定に依らない固定値であることと、§11.2 の時系列表 (受信から ≤ 4.5 s で `TRUE`) で間に合うことも確かめる。ログオフ / シャットダウンは隠しウィンドウ (`WM_QUERYENDSESSION` / `WM_ENDSESSION`) で受ける方式を v0.2 で実装し、v0.1 は「Windows のログオフ・シャットダウン時は未保存の内容が退避されないことがある」を既知の制約 (01 §4.6・リリースノート) にする (確定 (ユーザー回答))。Unix のシグナル分は v0.2 (macOS / Linux 出荷時) | スパイク / G1c (v0.1 M3。08 §3 S-11) |
| S-09 | Windows の置換 API (`SetFileInformationByHandle(FileRenameInfoEx, POSIX_SEMANTICS)` を採るか) | 未決。ACL / ADS / 属性保持と中間状態の有無で判断 (§4.2)。それまでは `ReplaceFileW` (退避名を必須で渡す) + §4.6 の復旧で、`ReplaceFileW` が失敗して T が無傷なら `AtomicUnsupported` → 確認 → in-place (`MoveFileExW` で置き換えない。確定 (ユーザー回答))。kill テストの切断点は 09 §3.6 (6) (原子的保存の 4 点・置換の途中・in-place の `kill_after_inflight_flag` / `kill_mid_backup` / `kill_after_backup` / `kill_after_inplace_alloc` / `kill_mid_inplace_write`、§4.4 手順 10)。同じスパイクで XXH3-64 (§5.5) の速度だけを計測する。`cargo install --git` の経路でリポジトリの `.cargo/config.toml` (`+crt-static`) が効くかも同じ検証環境で確かめる (§16.4) | スパイク / G1c (v0.1 M3。08 §3 S-09) |
| OQ-111 | クリップボードの実装方式 (v0.1) | **確定 (ユーザー回答)**: OS 付属の手段で実装し arboard を使わない。**v0.1 は Windows = `windows-sys` の Win32 API 直接 (+ 内部レジスタ) のみ**。OSC 52 と、macOS = `/usr/bin/pbcopy` / `pbpaste`、Linux = `/usr/bin` 配下の `wl-copy` / `wl-paste` / `xclip` / `xsel` の検出、WSL の `command` provider は v0.2 (macOS / Linux 出荷時。§8.2 / §8.3)。S-14 (v0.1 は Win32 のみ) で不成立なら arboard が退路 | 確定 (ユーザー回答) |
| OQ-112 | Linux の配布物 | v0.1 は Linux を出荷しない。Linux 出荷時 (v0.2) は **musl 静的を基本** (§1.1 / §18)。BM-104 が gnu 版比 1.5 倍以上遅い、または BM-202 が 1.3 倍以上なら gnu 版 (zigbuild 2.31) を追加 (§17.3) | PO / v0.2 着手前 (Linux 出荷前) |
| OQ-140 | 管理モードのローカル LLM サーバ (§13.7) のポート取得と待受 | Unix ドメインソケット / 名前付きパイプが使えるサーバではそれを使い、不可なら evoeditor が `127.0.0.1:0` を bind して空きポートを選びサーバに渡す (競合の窓は 1 回の再試行で吸収)。対応状況は S-22 (h) / S-23 で確認 | スパイク (S-23) / v1.L-b 着手前 |
| OQ-163 | `state_dir` を使えないときの緊急退避先 | v0.1 (Windows) は `%TEMP%\eve-recovery-<SID>\` (所有者のみの DACL)、v0.2 の Unix は `$TMPDIR/eve-recovery-<uid>/` (0700) に退避し、終了時表示でそのパスを示す (02 §11.2.1。OS 別の実体は §7.5) | 提案中 (裁量) / G1c |
| OQ-164 | 公開前の文書のうち本書の分 (インストール手順、アンインストール手順、minisign 鍵の公開場所と検証手順、脆弱性の通知と `SECURITY.md` の更新方法の案内) | §18.2a / §18.8 / §18.9 / §18.14 の案 | 提案中 (裁量) / `SECURITY.md` の最小の版は最初の push の前、インストール・アンインストールの手順と minisign の検証手順と `SECURITY.md` の更新方法の案内は v0.1 リリース前 |
| OQ-166 | β チャネルを設けるか (v1.0 以降) | v0.x は stable 1 本 (確定、ADR-0040)。β は v1.0 のリリース前に決める (§18.13) | 提案中 (裁量) / v1.0 リリース前 |
