//! ゴールデン (期待する出力のファイル) との比較 (docs/11 §6.2、docs/09 §3.4)。
//!
//! 環境変数 `EVE_UPDATE_GOLDEN` が立っていれば、比較せずにファイルを書く (無ければ作る、あれば書き換える)。
//! 立っていなければ、ファイルが無いときと内容が違うときに失敗する。改行は LF に揃えて比べる。
//!
//! 運用 (11 §6.2): 今の振る舞いをそのまま記録する **新しい** ゴールデンはエージェントが作ってよい (PR の本文に一覧を書く)。
//! **既にある** ゴールデンに差分が出たときの書き換えは人間が判断する (差分を PR に貼って止まる)。

use std::path::Path;

/// `path` のゴールデンと `actual` を比べる。`EVE_UPDATE_GOLDEN` が立っていれば `actual` を書く。違えば panic する。
///
/// # Panics
///
/// ゴールデンが無い・内容が違う・書けないとき (`compare` が `Err` を返す場合と同じ)。
pub fn check(path: &Path, actual: &str) {
    if let Err(m) = compare(path, actual) {
        panic!("{m}");
    }
}

/// `check` と同じだが、違いを panic でなく `Err` (説明の文) で返す (シナリオ DSL のように失敗を集めて報告するとき)。
///
/// # Errors
///
/// ゴールデンが無い、内容が違う (最初に違う行と前後を文に含める)、または `EVE_UPDATE_GOLDEN` で書くときにディレクトリを
/// 作れない・ファイルに書けないとき。
pub fn compare(path: &Path, actual: &str) -> Result<(), String> {
    let actual = actual.replace("\r\n", "\n");
    if std::env::var_os("EVE_UPDATE_GOLDEN").is_some() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        return std::fs::write(path, &actual).map_err(|e| format!("{}: {e}", path.display()));
    }
    let want = match std::fs::read_to_string(path) {
        Ok(s) => s.replace("\r\n", "\n"),
        Err(_) => {
            return Err(format!(
                "ゴールデン {} がありません (今の出力を記録するなら EVE_UPDATE_GOLDEN=1 で作る)",
                path.display()
            ));
        }
    };
    if want == actual {
        return Ok(());
    }
    let first = want
        .lines()
        .zip(actual.lines())
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| want.lines().count().min(actual.lines().count()));
    let excerpt = |s: &str| s.lines().skip(first).take(5).collect::<Vec<_>>().join("\n");
    Err(format!(
        "ゴールデン {} と違います ({} 行目から)。意図した変更なら EVE_UPDATE_GOLDEN=1 で書き換え、差分を PR で確かめる\n--- 期待\n{}\n--- 実際\n{}",
        path.display(),
        first + 1,
        excerpt(&want),
        excerpt(&actual),
    ))
}
