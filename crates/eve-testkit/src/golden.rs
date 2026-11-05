//! ゴールデン (期待する出力のファイル) との比較 (docs/11 §6.2、docs/09 §3.4)。
//!
//! 環境変数 `EVE_UPDATE_GOLDEN` が立っていれば、比較せずにファイルを書く (無ければ作る、あれば書き換える)。
//! 立っていなければ、ファイルが無いときと内容が違うときに panic する。改行は LF に揃えて比べる。
//!
//! 運用 (11 §6.2): 今の振る舞いをそのまま記録する **新しい** ゴールデンはエージェントが作ってよい (PR の本文に一覧を書く)。
//! **既にある** ゴールデンに差分が出たときの書き換えは人間が判断する (差分を PR に貼って止まる)。

use std::path::Path;

/// `path` のゴールデンと `actual` を比べる。`EVE_UPDATE_GOLDEN` が立っていれば `actual` を書く。
pub fn check(path: &Path, actual: &str) {
    let actual = actual.replace("\r\n", "\n");
    if std::env::var_os("EVE_UPDATE_GOLDEN").is_some() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).expect("ゴールデンのディレクトリを作れる");
        }
        std::fs::write(path, &actual).expect("ゴールデンを書ける");
        return;
    }
    let want = match std::fs::read_to_string(path) {
        Ok(s) => s.replace("\r\n", "\n"),
        Err(_) => panic!(
            "ゴールデン {} がありません (今の出力を記録するなら EVE_UPDATE_GOLDEN=1 で作る)",
            path.display()
        ),
    };
    if want != actual {
        let first = want
            .lines()
            .zip(actual.lines())
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| want.lines().count().min(actual.lines().count()));
        panic!(
            "ゴールデン {} と違います ({} 行目から)。意図した変更なら EVE_UPDATE_GOLDEN=1 で書き換え、差分を PR で確かめる\n--- 期待\n{}\n--- 実際\n{}",
            path.display(),
            first + 1,
            want.lines()
                .skip(first)
                .take(5)
                .collect::<Vec<_>>()
                .join("\n"),
            actual
                .lines()
                .skip(first)
                .take(5)
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
}
