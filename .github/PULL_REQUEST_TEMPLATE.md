<!--
宛先は `develop` です (`main` は develop → main のリリース PR だけ)。
外部からの Pull Request は、まだ受け付けていません (CONTRIBUTING.md)。
-->

## 概要

<!-- 何を、なぜ変えたか。1 タスク = 1 PR -->

## 関連する仕様・Issue

<!-- docs/ の節、FR / NFR / AC / OQ の番号、Issue -->

## 種別

- [ ] feat
- [ ] fix
- [ ] spec (docs)
- [ ] chore
- [ ] spike

## チェックリスト (docs/09-testing-quality.md §8.4、docs/11-dev-workflow.md §4)

- [ ] コミットに `Signed-off-by:` がある (`git commit -s`)
- [ ] 変更した関数に `spec:` (節 ID) がある
- [ ] 変更した Must 要件の AC に `verifies:` (AC ID) を持つテストがある
- [ ] ユーザーから見える挙動の変更で、シナリオテスト (v0.1 は `Harness`) を追加・更新した
- [ ] core に端末や OS の概念を入れていない。OS への依存は Port の中だけにある
- [ ] 文書の変更が Transaction を通っている
- [ ] `unwrap` / `expect` に理由がある。外部入力でパニックしない
- [ ] ログ・パニック・クラッシュログに、文書の本文・パスの全体・API キーを出していない
- [ ] `unsafe` を足した場合は `// SAFETY:` と不変条件がある
- [ ] UI スレッドで I/O や全文走査をしていない
- [ ] `eve-view` を触っていない (触った場合は、理由を下に書いた)
- [ ] 仕様と異なる判断をした場合、docs を直接直さず OQ の起票案を下に書いた

## 依存を追加した場合 (docs/02-architecture.md §13.0、docs/05-platform.md §17.2)

- [ ] 追加していない
- [ ] 追加した: crate 名・版・ライセンス・追加する理由・代替案・推移的依存の増加を下に書いた

## 補足

<!-- レビューで見てほしい点、`eve-view` の変更の理由、OQ の起票案など -->
