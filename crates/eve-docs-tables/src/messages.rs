//! 04 §15.1 (画面文言) と §15.2 (保存・読込・退避・パニックの文言目録) の表 (docs/04-tui-design.md) を読む。
//!
//! 読み手は `cargo xtask i18n` (言語ファイルとの突合) と `cargo xtask review-wording` (文言の点検表)
//! (docs/11 §8.5、docs/review/v0.3-findings.md の STR-E1)。

use crate::{Error, identifiers, read_tables};

/// 画面文言の見出し (04 §15.1)。
pub const SCREEN_HEADING: &str = "### 15.1 v0.1 の画面文言 (ja)";
/// 文言目録の見出し (04 §15.2)。
pub const CATALOG_HEADING: &str = "### 15.2 保存・読込・退避・パニックの文言目録";

/// 出す場面の列の名前 (表によって違う。最初に見つかったものを使う)。
const SCENE_COLUMNS: [&str; 3] = ["出す場面", "出す場面 (07 §4.2)", "内容"];

/// 文言 1 件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub line: usize,
    pub key: String,
    /// `ja` 列 (書かれたまま)。
    pub ja: String,
    /// 出す場面の列 (無い表では空)。
    pub scene: String,
}

fn read_with(
    doc04: &str,
    heading: &str,
    keys_of: &dyn Fn(&str) -> Result<Vec<String>, String>,
) -> Result<Vec<Message>, Error> {
    let mut out = Vec::new();
    for t in read_tables(doc04, heading, &["MessageKey", "ja"])? {
        let scene_col = SCENE_COLUMNS.iter().find(|c| t.column(c).is_some());
        for r in &t.rows {
            let keys = keys_of(t.cell(r, "MessageKey").unwrap_or_default()).map_err(|reason| {
                Error::BadRow {
                    line: r.line,
                    reason,
                }
            })?;
            let ja = t.cell(r, "ja").unwrap_or_default().to_owned();
            let scene = scene_col
                .and_then(|c| t.cell(r, c))
                .unwrap_or_default()
                .trim()
                .to_owned();
            for key in keys {
                out.push(Message {
                    line: r.line,
                    key,
                    ja: ja.clone(),
                    scene: scene.clone(),
                });
            }
        }
    }
    Ok(out)
}

/// 04 §15.2 の文言目録。`MessageKey` 列は 1 行に 1 つ (略記・複数は失敗にする)。
///
/// # Errors
///
/// 見出しや列が見つからないとき、または `MessageKey` 列が 1 行に 1 つでないとき (`Error::BadRow` など)。
pub fn read_catalog(doc04: &str) -> Result<Vec<Message>, Error> {
    read_with(doc04, CATALOG_HEADING, &|cell| {
        let keys = identifiers(cell);
        if keys.len() == 1 {
            Ok(keys)
        } else {
            Err(format!(
                "`MessageKey` 列には MessageKey を 1 つだけバッククォートで書きます (略記・複数は不可): {cell:?}"
            ))
        }
    })
}

/// 04 §15.1 の画面文言。`MessageKey` 列は `a.b.long` / `.short` のように、後ろの項目が直前のキーの最後の語を
/// 置き換える略記を使う (展開して 1 件ずつにする。`ja` と場面は同じ行のもの)。
///
/// # Errors
///
/// 見出しや列が見つからないとき、または略記の前にキーがないなど `MessageKey` 列を展開できないとき (`Error::BadRow` など)。
pub fn read_screen(doc04: &str) -> Result<Vec<Message>, Error> {
    read_with(doc04, SCREEN_HEADING, &|cell| {
        let mut out: Vec<String> = Vec::new();
        for id in identifiers(cell) {
            let key = match (id.strip_prefix('.'), out.last()) {
                (Some(tail), Some(p)) => {
                    let base = p.rsplit_once('.').map_or(p.as_str(), |(b, _)| b);
                    format!("{base}.{tail}")
                }
                (Some(_), None) => return Err(format!("略記 `{id}` の前にキーがありません")),
                _ => id,
            };
            out.push(key);
        }
        Ok(out)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MD: &str = "\
### 15.1 v0.1 の画面文言 (ja)

| MessageKey | ja | 出す場面 |
|---|---|---|
| `ui.a.long` / `.short` | 長い / 短い | 画面 |

### 15.2 保存・読込・退避・パニックの文言目録

| MessageKey | ja | 内容 |
|---|---|---|
| `msg.x` | x です | 保存 |
";

    #[test]
    fn 略記を展開し_場面の列を読む() {
        let s = read_screen(MD).unwrap();
        let keys: Vec<&str> = s.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(keys, ["ui.a.long", "ui.a.short"]);
        assert_eq!(s[1].scene, "画面");
        let c = read_catalog(MD).unwrap();
        assert_eq!((c[0].key.as_str(), c[0].scene.as_str()), ("msg.x", "保存"));
    }

    #[test]
    fn 目録の略記は失敗する() {
        let md = MD.replace("| `msg.x` |", "| `msg.x` / `.y` |");
        assert!(matches!(read_catalog(&md), Err(Error::BadRow { .. })));
    }

    #[test]
    fn 実際の_04_を読める() {
        let doc = include_str!("../../../docs/04-tui-design.md");
        assert!(read_screen(doc).unwrap().len() > 50);
        assert!(read_catalog(doc).unwrap().len() > 100);
    }
}
