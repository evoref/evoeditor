# CLAUDE.md — evoeditor

Rust 製テキストエディタ。**仕様書駆動** ([ADR-0001](docs/10-decisions.md)) で開発する。仕様は `docs/` が正本 (入口は [docs/README.md](docs/README.md))。コードを書く前に該当する仕様書の節を読み、仕様と食い違うときは **コードではなく仕様書を先に直す提案** をする。ADR・OQ の状態は [docs/10-decisions.md](docs/10-decisions.md) を正とする。

ブランチ運用の正本は [docs/11-dev-workflow.md §9.1](docs/11-dev-workflow.md)。要点は次のとおり。

## ブランチ

- 長命ブランチは `main` と `develop` の 2 本だけ。
  - `main`: リリース専用。リリースした版だけが入る。**直接 commit / push しない**。
  - `develop`: 開発の本線。**直接 commit / push しない** (最初の push (M0e) は 2026-10-20 に完了済みで、例外はない)。変更は作業ブランチ → PR で入れる。
- 作業ブランチは **`develop` から作る**。種別は `feat/` `fix/` `spec/` (docs) `chore/` `spike/`。名前は `<種別>/<crate>-<短い説明>` (例: `feat/eve-core-linear-undo`、`spec/04-color-depth`)。エージェントの作業は `agent/<名前>/<Issue 番号>-<短い説明>`。
- **1 タスク = 1 作業ブランチ = 1 PR**。PR は `develop` 宛て、squash で入れる (develop 上で 1 作業 = 1 コミット)。マージ後に作業ブランチを削除する。rebase は作業ブランチ内でだけ自由。
- `dev/<版>`、`release/*`、`hotfix/*` ブランチは使わない。
- merge は人間が行う。`gh pr merge` は実行前に確認が出る。

## リリースと版

- 機能リリースは `0.y.0` 単位 (v0.1.0、v0.2.0 …)。`develop` → `main` の PR (merge commit) を人間が入れ、`main` のマージコミットに **人間が署名付きタグ** を付けて GitHub Releases に公開する。**タグ (`v*`) は私が作らない・push しない**。
- bugfix リリースはパッチ版 (`0.y.z` の `z`。10 以上も可)。経路は `fix/…` → `develop` → `main` (タグ) だけ。**出せる条件**: 前回のタグ以降の `develop` に `feat` が 1 件も入っていない。`feat` が入っているときは、修正を次の機能版に含める。機能版のリリース直後からパッチを出し終えるまでは、`feat` の PR を後回しにして `fix` を先に入れる。
- 版の比較は数値順 (`v0.1.10` は `v0.1.9` より新しい)。タグの並べ替えは `git tag --sort=version:refname`。
- v0.1.0 のリリースまでの `0.0.x` は内部カウンタ (マイルストーンを develop に取り込む PR で +1、タグなし・Release なし)。パッチ版 (`0.y.z` の `z`) とは別物。
- プレリリース (`-rc.N` など) のタグは `develop` の先頭に付ける (人間)。凍結 (ゲート判定) から昇格・タグまでの間は、`develop` への merge を保留する。

## コミット

- `git commit -s` (DCO。`Signed-off-by:` を付ける)。公開してよい著者名とメールアドレスだけを使う (このリポジトリの `user.email` は `info@evoref.com`)。
- コミットメッセージの形式は [docs/11 §9.2](docs/11-dev-workflow.md) に従う。
- 秘密情報・個人情報・基準機の型番などを docs やコードに書かない ([docs/12 §5.6](docs/12-licensing-distribution.md) の PC-1〜PC-7)。

## 機械的な強制

- `.claude/settings.json` の PreToolUse hook (`.claude/hooks/guard-git.py`) が、次を拒否または確認にする。
  - `main` / `develop` への push、force push、リモートのブランチ削除
  - `main` / `develop` 上の commit / merge / rebase など
  - `v*` タグの作成と push
  - `git stash` (作業ツリーを動かし、改行コードや未追跡の変更を壊しやすいため)
  - `git reset --hard` / `git clean -f` / `git restore` / `git checkout -- <path>` (作業ツリーの変更を捨てる。確認が出る)
  - `gh pr merge` (確認)
- GitHub のブランチ保護 (`main` と `develop`: PR 必須、直接 push・force push・削除の禁止) も併用する。設定の手順は [docs/11 §9.1](docs/11-dev-workflow.md)。
- hook に拒否されたら、回避せずに理由を人間に伝えて指示を待つ。

## サブエージェントに作業を任せるとき

- 依頼文に、**作業ツリーを動かす git コマンド (`stash` / `checkout` / `reset` / `restore` / `clean`) を使わない** ことを明記する (hook でも拒否される)。
- 編集するファイルの範囲 (例: `docs/` のみ) と、コミットしないことを明記する。
- 各ファイルの改行コード (CRLF/LF) を維持させる。Python では `newline=''` で読み書きする。
- 報告は信用しきらず、完了後に自分で `git status` / `git diff --stat` / リンク検査で確かめる。
