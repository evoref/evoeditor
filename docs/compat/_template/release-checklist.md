# リリースのチェックリスト — v<版>

この雛形を `docs/compat/<版>/release-checklist.md` に写して埋める (docs/09 §8.2 の「リリースのチェックリスト」、§9.2 手順 3)。
**タグの前に済む行だけ** を載せる。タグの後の結果 (配布物・タグの検証・MSRV・配布経路の検証・更新手順の文書の確認・公開後の
minisign の検証) は GitHub Release のノートの「検証」の節に書き、このファイルには戻さない。

次の行は `release.yml` が読む (先頭の 1 か所だけ)。40 桁の SHA を書く。雛形のままだとタグの検証が失敗する。

判定コミット: <ゲートを判定した版上げ後の develop の先頭の SHA (40 桁)>

- 版: <X.Y.Z>
- 判定した人 / 日付: <名前> / <YYYY-MM-DD>
- 判定コミットの確認 (`git merge-base --is-ancestor develop <SHA>` と `git merge-base --is-ancestor main <SHA>` が真): [ ]
- この後に入れてよい変更は `docs/compat/<版>/`・`docs/perf/<版>/`・`tests/ac-checklist.toml` だけ。ほかを入れたらゲートをやり直す。

判定は 合格 / 不合格 / 対象外 (理由) のいずれか。「根拠」には CI の実行の URL か記録のファイルを書く。基準機の型番など固有の情報は書かない (docs/11 §9.7)。

| ゲート | 条件の正本 | 判定 | 根拠 | 確認者 | 日付 |
|---|---|---|---|---|---|
| 要件充足 | 09 §8.2。Must FR/NFR がすべて T/B/M/R で合格。`tests/ac-checklist.toml` に todo が無い (または理由つきの例外) | | | | |
| 性能 | 09 §8.2。BM-1xx/2xx の Must が基準機で目標内 (p50/p99)。記録は `docs/perf/<版>/` | | | | |
| 互換 | 09 §8.2。Windows Terminal と conhost で手動シナリオ (開く・日本語入力 (Microsoft IME・Google 日本語入力)・検索・保存・終了)。記録は `docs/compat/<版>/` | | | | |
| 受入 | 09 §8.2。L7 の Must 項目。記録は `docs/compat/<版>/acceptance.md` | | | | |
| データ保全 | 09 §8.2。kill テスト・往復コーパスなど | | | | |
| セキュリティ (依存) | `cargo audit` / `deny advisories` がクリーン (例外は `deny.toml` に期限と理由つき) | | | | |
| セキュリティ (リリース前の確認) | 09 §8.2 の (a)〜。結果をここに書く | | | | |
| fuzz | 判定コミットで `fuzz_decode` を 1 時間 (`nightly.yml` の手動実行) してクラッシュ 0。夜間 fuzz の 7 日の観測 (rc は 2 日) | | | | |
| 契約 | `eve-view` の型が `#[non_exhaustive]` で `CONTRACT_VERSION` を持つ | | | | |
| 依存 | C を含む依存 0 (`cargo xtask check-deps`) | | | | |
| ドキュメント | README (`dist/README.md`)・CHANGELOG・生成リファレンスが最新。端末の設定・IME とチョードの注意・既知の制約 (01 §4.6) が README にある | | | | |
| ライセンス・名称 | 12 §3.8 の LR-1 (Apache-2.0 の特許条項と NOTICE の運用) と LR-4 (名称の商標の簡易照会)。結果は 10 の OQ-01 | | | | |
| 未決事項の解消 | 当該版の Must に影響する OQ がすべて回答済み (08 §5) | | | | |

## 所見・例外

(不合格・対象外の理由と、その扱い (繰り越し先など) を書く。)
