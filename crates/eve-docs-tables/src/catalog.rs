//! 07 §11.6 の CommandId カタログ (docs/07-config-extensibility.md) を読む。
//!
//! カタログの読み手は、既定キーマップの突合 (eve-core)・権限と種類と `when` の突合 (eve-core のテスト)・
//! 文言の検査 (`cargo xtask i18n`)・生成リファレンスとの突合 (`cargo xtask gen-reference --check`) の 4 つ。列の解釈 (複数 ID の行の分け方・版の注記) をここ 1 か所にまとめる
//! (docs/11 §8.5、docs/review/v0.3-findings.md の STR-E1)。

use crate::{Error, identifiers, read_tables, strip_parentheticals};

/// カタログの見出し (07 §11.6)。
pub const HEADING: &str = "### 11.6 v0.1 CommandId カタログ (付録)";

/// カタログの列 (07 §11.6)。どの表にもすべてそろっていること。
pub const COLUMNS: [&str; 9] = [
    "ID",
    "title key",
    "kind",
    "args",
    "when",
    "perm",
    "コンテキスト",
    "既定キー",
    "ヒント",
];

/// カタログの 1 行。`` `a` / `b` `` のように複数の ID をまとめた行もある。
///
/// `title_keys` / `kinds` / `perms` は `ids` と同じ数・同じ順に分けてある。`args` から先の列は、行のまま
/// (読み手ごとに解釈する。分け方は各列の注記の書き方に従う)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// docs のファイルの中の行番号 (1 始まり)。
    pub line: usize,
    pub ids: Vec<String>,
    /// 後の版の注記 (`(v0.2)` など) が ID 列に付いた行。実装した版から登録してよい (必須ではない)。
    pub later: bool,
    pub title_keys: Vec<String>,
    /// `Edit` / `Motion` / `Ui` / `File`。
    pub kinds: Vec<String>,
    /// `-` / `read` / `propose` / `ui`。
    pub perms: Vec<String>,
    pub args: String,
    pub when: String,
    pub contexts: String,
    pub keys: String,
    pub hints: String,
}

impl Row {
    /// 注記の「`<語>` のみ」の語 (ID の最後の `.` / `_` の後ろ) に当たる ID の位置。
    pub fn id_index_by_last_word(&self, word: &str) -> Option<usize> {
        let word = word.trim();
        if word.is_empty() {
            return None;
        }
        self.ids
            .iter()
            .position(|id| id.rsplit(['.', '_']).next() == Some(word))
    }
}

/// 07 の本文からカタログを読む。列の欠け・ID と title key の個数の違い・分けられない kind / perm は失敗にする。
pub fn read(doc07: &str) -> Result<Vec<Row>, Error> {
    let tables = read_tables(doc07, HEADING, &COLUMNS)?;
    let mut out = Vec::new();
    for t in &tables {
        for r in &t.rows {
            let cell = |name: &str| t.cell(r, name).unwrap_or_default().to_owned();
            let id_cell = cell("ID");
            let ids = identifiers(&strip_parentheticals(&id_cell));
            let title_keys = identifiers(&strip_parentheticals(&cell("title key")));
            let bad = |reason: String| Error::BadRow {
                line: r.line,
                reason,
            };
            if ids.is_empty() || ids.len() != title_keys.len() {
                return Err(bad(format!(
                    "`ID` 列 ({}) と `title key` 列 ({}) の個数が合いません",
                    ids.len(),
                    title_keys.len()
                )));
            }
            let mut row = Row {
                line: r.line,
                later: id_cell.contains("(v"),
                title_keys,
                kinds: Vec::new(),
                perms: Vec::new(),
                args: cell("args"),
                when: cell("when"),
                contexts: cell("コンテキスト"),
                keys: cell("既定キー"),
                hints: cell("ヒント"),
                ids,
            };
            row.kinds = split_kinds(&row, &cell("kind")).map_err(bad)?;
            row.perms = split_perms(&row, &cell("perm")).map_err(bad)?;
            out.push(row);
        }
    }
    Ok(out)
}

/// kind 列: `Edit` (行のすべて)、`Edit / Ui` (ID の順)、`Edit (cut) / Ui (copy)` (注記の語で ID を選ぶ)。
fn split_kinds(row: &Row, cell: &str) -> Result<Vec<String>, String> {
    let parts: Vec<&str> = cell.split(" / ").map(str::trim).collect();
    let n = row.ids.len();
    if parts.len() == 1 && !parts[0].contains('(') {
        return Ok(vec![parts[0].to_owned(); n]);
    }
    let mut out = vec![String::new(); n];
    for (i, p) in parts.iter().enumerate() {
        let (kind, idx) = match p.split_once(" (") {
            Some((k, w)) => {
                let w = w.strip_suffix(')').unwrap_or(w);
                let idx = row
                    .id_index_by_last_word(w)
                    .ok_or_else(|| format!("kind 列の `({w})` に当たる ID がありません"))?;
                (k.trim(), idx)
            }
            None if parts.len() == n => (*p, i),
            None => return Err(format!("kind 列 `{cell}` を ID {n} 個に分けられません")),
        };
        out[idx] = kind.to_owned();
    }
    if out.iter().any(String::is_empty) {
        return Err(format!("kind 列 `{cell}` に kind の無い ID があります"));
    }
    Ok(out)
}

/// perm 列: `` `-` `` (行のすべて) か、`` `-` / `read` `` (ID の順)。括弧の注記は読まない。
fn split_perms(row: &Row, cell: &str) -> Result<Vec<String>, String> {
    let stripped = strip_parentheticals(cell);
    let parts: Vec<String> = stripped
        .split('/')
        .map(|p| p.trim().trim_matches('`').to_owned())
        .collect();
    let n = row.ids.len();
    match parts.len() {
        1 => Ok(vec![parts[0].clone(); n]),
        m if m == n => Ok(parts),
        m => Err(format!("ID ({n}) と perm ({m}) の個数が合いません")),
    }
}

/// 既定キー列から読んだ割当 1 件 (07 §11.6 の「既定キー」列の記法)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogBinding {
    /// 割当の対象の CommandId。
    pub id: String,
    /// 既定キー列に `` `ctx`: `` の見出しがあればそのコンテキスト。無ければ `None` (行の「コンテキスト」列に従う)。
    pub context: Option<String>,
    /// 設定の表記のままのキー (`mod+s`、`ctrl+k s`)。
    pub keys: String,
    /// 拡張キーが要る (`[E]`)。
    pub enhanced: bool,
}

/// 既定キー列の 1 つの組 (`` `a` / `b` `` の並び) と、直後の括弧書きの注記。
struct Group<'a> {
    context: Option<&'a str>,
    /// キー (`—` は `None`) と、拡張キー (`[E]`) が要るか。
    elems: Vec<(Option<&'a str>, bool)>,
    note: String,
}

/// 既定キー列を、割当の組と注記に分ける。属性 `[横]` `(alt)` は読み飛ばし、`[E]` は覚える (対の最後のキーにだけ
/// 付いたものは対のすべてに及ぶ)。`` `ctx`: `` はそれ以降の組のコンテキスト。それ以外の `(…)` は注記。
fn groups(cell: &str) -> Result<Vec<Group<'_>>, String> {
    let mut out: Vec<Group<'_>> = Vec::new();
    let mut cur: Vec<(Option<&str>, bool)> = Vec::new();
    let mut context: Option<&str> = None;
    let mut rest = cell.trim();
    while !rest.is_empty() {
        let key = if let Some(r) = rest.strip_prefix('`') {
            let end = r
                .find('`')
                .ok_or_else(|| format!("バッククォートが閉じていません: {cell}"))?;
            let (k, after) = (&r[..end], &r[end + 1..]);
            if let Some(a) = after.strip_prefix(':') {
                context = Some(k); // `ctx`: の見出し
                rest = a.trim_start();
                continue;
            }
            rest = after;
            Some(k)
        } else if let Some(r) = rest.strip_prefix('—') {
            rest = r;
            None
        } else {
            rest = rest.trim_start_matches([',', '、', ' ']);
            if rest.starts_with('(') {
                // 組の外の括弧書き (直前の組の注記)
                let (note, after) = take_paren(rest);
                if let Some(last) = out.last_mut() {
                    last.note.push_str(&note);
                }
                rest = after;
            } else if !rest.is_empty() && !rest.starts_with('`') && !rest.starts_with('—') {
                rest = &rest[rest.chars().next().map_or(0, char::len_utf8)..];
            }
            continue;
        };
        // 属性: `[E]` `[横]` `(alt)`
        let mut enhanced = false;
        loop {
            let t = rest.trim_start();
            if t.starts_with('[') {
                let end = t
                    .find(']')
                    .ok_or_else(|| format!("] が閉じていません: {cell}"))?;
                enhanced |= &t[1..end] == "E";
                rest = &t[end + 1..];
            } else if let Some(r) = t.strip_prefix("(alt)") {
                rest = r;
            } else {
                break;
            }
        }
        cur.push((key, enhanced));
        if let Some(r) = rest.trim_start().strip_prefix("/ ") {
            rest = r; // 同じ組の続き
        } else {
            // 対の最後にだけ属性があるときは、対のすべてに及ぶ
            if cur.len() > 1 && cur[..cur.len() - 1].iter().all(|e| !e.1) && cur[cur.len() - 1].1 {
                for e in &mut cur {
                    e.1 = true;
                }
            }
            out.push(Group {
                context,
                elems: std::mem::take(&mut cur),
                note: String::new(),
            });
        }
    }
    Ok(out)
}

/// `(` から対応する `)` まで (入れ子を数える)。中身と残りを返す。
fn take_paren(s: &str) -> (String, &str) {
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return (s[1..i].to_owned(), &s[i + 1..]);
                }
            }
            _ => {}
        }
    }
    (s[1..].to_owned(), "")
}

/// 行の既定キー列を、ID ごとの割当にする。複数 ID の行の組 (`` `a` / `b` ``) は左から ID に 1 対 1 で当て、1 つだけの
/// 割当は注記の「`<ID の最後の語>` のみ」で対象を決める (07 §11.6 の記法)。割当の無い組 (`—`) は飛ばす。
pub fn bindings(row: &Row) -> Result<Vec<CatalogBinding>, String> {
    let ids = &row.ids;
    let mut out = Vec::new();
    for g in groups(&row.keys)? {
        if g.elems.iter().all(|e| e.0.is_none()) {
            continue; // 割当が無い (`—` だけ)
        }
        let only: Option<&String> = if g.elems.len() == 1 && ids.len() > 1 {
            let word = g.note.split(" のみ").next().unwrap_or("");
            let i = row.id_index_by_last_word(word).ok_or_else(|| {
                format!("行 {}: 割当 1 個の対象が決まりません: {}", row.line, g.note)
            })?;
            Some(&ids[i])
        } else {
            None
        };
        if only.is_none() && g.elems.len() != ids.len() && ids.len() != 1 {
            return Err(format!(
                "行 {}: ID {} 個に割当 {} 個: {:?}",
                row.line,
                ids.len(),
                g.elems.len(),
                g.elems
            ));
        }
        for (i, (e, enhanced)) in g.elems.iter().enumerate() {
            let Some(k) = e else {
                continue; // `—` (無い側)
            };
            let id = only.unwrap_or(&ids[if ids.len() == 1 { 0 } else { i }]);
            out.push(CatalogBinding {
                id: id.clone(),
                context: g.context.map(str::to_owned),
                keys: (*k).to_owned(),
                enhanced: *enhanced,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(rows: &str) -> String {
        format!(
            "{HEADING}\n\n| {} |\n|{}\n{rows}\n### 11.7 次\n",
            COLUMNS.join(" | "),
            "---|".repeat(COLUMNS.len())
        )
    }

    #[test]
    fn 複数_id_の行を_id_ごとに分ける() {
        let doc = md(concat!(
            "| `file.save` | `cmd.file.save` | File | — | `!doc.read_only` | `-` | `editor` | `mod+s` | normal:4 |\n",
            "| `clipboard.copy` / `clipboard.cut` | `cmd.clipboard.copy` / `cmd.clipboard.cut` | Edit (cut) / Ui (copy) | — | — | `read` / `propose` | `editor` | `mod+c` / `mod+x` | - |\n",
            "| `buffer.next` / `buffer.prev` (v0.2) | `cmd.buffer.next` / `cmd.buffer.prev` | Ui | — | — | `ui` | `editor` | — | - |",
        ));
        let rows = read(&doc).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].ids, ["file.save"]);
        assert_eq!(rows[0].when, "`!doc.read_only`");
        assert!(!rows[0].later);
        assert_eq!(rows[1].kinds, ["Ui", "Edit"]);
        assert_eq!(rows[1].perms, ["read", "propose"]);
        assert_eq!(rows[2].kinds, ["Ui", "Ui"]);
        assert!(rows[2].later);
        assert_eq!(rows[1].id_index_by_last_word("cut"), Some(1));
    }

    #[test]
    fn 個数の合わない行は失敗する() {
        let doc = md("| `a.b` / `a.c` | `cmd.a.b` | Ui | — | — | `-` | `editor` | — | - |");
        assert!(matches!(read(&doc), Err(Error::BadRow { .. })));
        let doc = md(
            "| `a.b` / `a.c` | `cmd.a.b` / `cmd.a.c` | Ui (x) | — | — | `-` | `editor` | — | - |",
        );
        assert!(matches!(read(&doc), Err(Error::BadRow { .. })));
    }

    #[test]
    fn 既定キー列を_id_とコンテキストごとの割当にする() {
        let doc = md(concat!(
            "| `edit.undo` / `edit.redo` | `cmd.edit.undo` / `cmd.edit.redo` | Edit | — | — | `propose` | `editor` | `mod+z` / `mod+y`, `mod+shift+z` [E] (redo のみ) | - |
",
            "| `a.x` / `a.y` | `cmd.a.x` / `cmd.a.y` | Ui | — | — | `-` | `list`, `dialog` | `list`: `down` / `up`、`dialog`: `tab` / `shift+tab` [E] | - |",
        ));
        let rows = read(&doc).unwrap();
        let b = bindings(&rows[0]).unwrap();
        let flat: Vec<_> = b
            .iter()
            .map(|b| (b.id.as_str(), b.keys.as_str(), b.enhanced))
            .collect();
        assert_eq!(
            flat,
            [
                ("edit.undo", "mod+z", false),
                ("edit.redo", "mod+y", false),
                ("edit.redo", "mod+shift+z", true),
            ]
        );
        assert!(b.iter().all(|b| b.context.is_none()));
        let b = bindings(&rows[1]).unwrap();
        let flat: Vec<_> = b
            .iter()
            .map(|b| {
                (
                    b.id.as_str(),
                    b.context.as_deref(),
                    b.keys.as_str(),
                    b.enhanced,
                )
            })
            .collect();
        assert_eq!(
            flat,
            [
                ("a.x", Some("list"), "down", false),
                ("a.y", Some("list"), "up", false),
                ("a.x", Some("dialog"), "tab", true),
                ("a.y", Some("dialog"), "shift+tab", true),
            ]
        );
    }

    #[test]
    fn 実際の_07_を読める() {
        let doc = include_str!("../../../docs/07-config-extensibility.md");
        let rows = read(doc).unwrap();
        let v01 = rows
            .iter()
            .filter(|r| !r.later)
            .flat_map(|r| &r.ids)
            .count();
        assert!(v01 >= 60, "v0.1 の CommandId は 63 個: {v01}");
    }
}
