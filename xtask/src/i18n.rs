//! `cargo xtask i18n`: UI 文言の検査 (docs/09-testing-quality.md §6.2 の `i18n`、docs/07 §17、docs/11 §8.5、NFR-602 / 603)。
//!
//! 入力は docs の 2 つの表 (`eve-docs-tables` で、11 §8.5 の規則どおりに読む。読めなければ失敗):
//! - 04 §15.2 の MessageKey 目録 (`MessageKey` / `ja` 列)
//! - 07 §11.6 の v0.1 CommandId カタログ (`ID` / `title key` / `ヒント` 列)
//!
//! 検査する内容:
//! 1. 目録の整合: MessageKey の重複がない、`ja` が空でない、`{…}` の引数の括弧が対応している、
//!    `msg.*` は `.text` と `.hint` の対 (NFR-602)、`info.*` は `.hint` を持たない、`hint.*` は表示幅 8 セル以内。
//! 2. カタログの整合: v0.1 の各 CommandId の `title key` が `cmd.<id>` で、左から順に 1 対 1 に対応する。
//! 3. `locales/ja.toml` があれば、期待するキー集合 (目録から `stderr.*` / `panic.*` / `cli.*` を除いたもの、
//!    `cmd.<id>.title` / `cmd.<id>.description`、ヒントの文脈 (`normal` / `overlay`) を持つ ID の `hint.<id>`) と
//!    比べる。欠落も余剰もエラー。無ければ、その旨を表示してこの項目だけ飛ばす (言語ファイルは MessageKey を
//!    実装するマイルストーンで作る)。
//!
//! コードの MessageKey の列挙との突合は、その列挙ができてから足す。

use std::collections::{BTreeMap, BTreeSet};

use eve_docs_tables::{identifiers, read_tables, strip_parentheticals};

use crate::util::{read_text, workspace_root};

const MESSAGE_HEADING: &str = "### 15.2 保存・読込・退避・パニックの文言目録";
const SCREEN_HEADING: &str = "### 15.1 v0.1 の画面文言 (ja)";
const CATALOG_HEADING: &str = "### 11.6 v0.1 CommandId カタログ (付録)";
/// 言語ファイルに入れない (eve-host の定数表に置く) キーの接頭辞。
const NOT_IN_LOCALE: [&str; 3] = ["stderr.", "panic.", "cli."];
/// ヒントの幅の上限 (表示幅 8 セル = 全角 4 文字 / 半角 8 文字、NFR-603)。
const HINT_MAX_CELLS: usize = 8;

/// 目録の 1 行。
struct Entry {
    key: String,
    ja: String,
    line: usize,
}

/// 文字の表示幅の近似 (全角・半角カナ以外の幅広の文字を 2、それ以外を 1)。
/// eve-text の幅の計算ができるまでの、この検査だけの近似。
fn cells(s: &str) -> usize {
    s.chars()
        .map(|c| match c as u32 {
            0x1100..=0x115F
            | 0x2E80..=0x303E
            | 0x3041..=0x33FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xA000..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE30..=0xFE6F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6 => 2,
            _ => 1,
        })
        .sum()
}

fn braces_balanced(s: &str) -> bool {
    let mut depth = 0i32;
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            // `{{` は `{` のエスケープ (07 §17.2)
            '{' if it.peek() == Some(&'{') => {
                it.next();
            }
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
        if !(0..=1).contains(&depth) {
            return false;
        }
    }
    depth == 0
}

fn read_messages(doc04: &str) -> Result<Vec<Entry>, String> {
    let tables = read_tables(doc04, MESSAGE_HEADING, &["MessageKey", "ja"])
        .map_err(|e| format!("04 §15.2 の目録を読めません: {e}"))?;
    let mut out = Vec::new();
    for t in &tables {
        for row in &t.rows {
            let keys = identifiers(t.cell(row, "MessageKey").unwrap_or_default());
            if keys.len() != 1 {
                return Err(format!(
                    "04 §15.2 行 {}: `MessageKey` 列には MessageKey を 1 つだけバッククォートで書きます (略記・複数は不可): {:?}",
                    row.line,
                    t.cell(row, "MessageKey")
                ));
            }
            out.push(Entry {
                key: keys.into_iter().next().unwrap_or_default(),
                ja: t.cell(row, "ja").unwrap_or_default().to_owned(),
                line: row.line,
            });
        }
    }
    Ok(out)
}

/// 04 §15.1 の画面文言の MessageKey。`MessageKey` 列は `a.b.long` / `.short` のように、後ろの項目が
/// 直前のキーの最後の語を置き換える略記を使う。
fn read_screen_keys(doc04: &str) -> Result<BTreeSet<String>, String> {
    let tables = read_tables(doc04, SCREEN_HEADING, &["MessageKey", "ja"])
        .map_err(|e| format!("04 §15.1 の画面文言を読めません: {e}"))?;
    let mut out = BTreeSet::new();
    for t in &tables {
        for row in &t.rows {
            let mut prev: Option<String> = None;
            for id in identifiers(t.cell(row, "MessageKey").unwrap_or_default()) {
                let key = match (id.strip_prefix('.'), &prev) {
                    (Some(tail), Some(p)) => {
                        let base = p.rsplit_once('.').map_or(p.as_str(), |(b, _)| b);
                        format!("{base}.{tail}")
                    }
                    _ => id,
                };
                prev = Some(key.clone());
                out.insert(key);
            }
        }
    }
    Ok(out)
}

/// カタログの v0.1 の行: (CommandId, title key, ヒントの文脈の集合)。
struct Command {
    id: String,
    title_key: String,
    hint_contexts: BTreeSet<String>,
}

fn read_catalog(doc07: &str) -> Result<Vec<Command>, String> {
    let cols = [
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
    let tables = read_tables(doc07, CATALOG_HEADING, &cols)
        .map_err(|e| format!("07 §11.6 のカタログを読めません: {e}"))?;
    let mut out = Vec::new();
    for t in &tables {
        for row in &t.rows {
            let id_cell = t.cell(row, "ID").unwrap_or_default();
            // 版の注記は ID 列の括弧に書く。(v0.2) などが付いた行は v0.1 の検査から外す。
            let version_note = id_cell.matches('(').count() > 0 && id_cell.contains("(v");
            if version_note {
                continue;
            }
            let ids = identifiers(&strip_parentheticals(id_cell));
            let titles = identifiers(&strip_parentheticals(
                t.cell(row, "title key").unwrap_or_default(),
            ));
            if ids.is_empty() || ids.len() != titles.len() {
                return Err(format!(
                    "07 §11.6 行 {}: `ID` 列 ({}) と `title key` 列 ({}) の個数が合いません",
                    row.line,
                    ids.len(),
                    titles.len()
                ));
            }
            // ヒント列: `文脈:優先度` を `, ` で区切る。`(<ID の最後の語> のみ)` は一部の ID に限る。
            let hint_cell = t.cell(row, "ヒント").unwrap_or_default();
            let mut all_contexts = BTreeSet::new();
            let mut per_id: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
            for part in hint_cell.split(", ") {
                let (body, only) = match part.split_once(" (") {
                    Some((b, rest)) => (b, rest.strip_suffix(" のみ)").map(str::to_owned)),
                    None => (part, None),
                };
                let ctx = body
                    .trim()
                    .split(':')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                if ctx.is_empty() || ctx == "-" || ctx == "—" {
                    continue;
                }
                match only {
                    Some(word) => {
                        per_id.entry(word).or_default().insert(ctx);
                    }
                    None => {
                        all_contexts.insert(ctx);
                    }
                }
            }
            for (id, title_key) in ids.into_iter().zip(titles) {
                let last = id.rsplit(['.', '_']).next().unwrap_or(&id).to_owned();
                let mut hint_contexts = all_contexts.clone();
                if let Some(extra) = per_id.get(&last) {
                    hint_contexts.extend(extra.iter().cloned());
                }
                out.push(Command {
                    id,
                    title_key,
                    hint_contexts,
                });
            }
        }
    }
    Ok(out)
}

/// `locales/ja.toml` を平らなキーの集合 (リーフが文字列のもの) にする。
fn flatten(prefix: &str, v: &toml::Value, out: &mut BTreeMap<String, String>) {
    match v {
        toml::Value::Table(t) => {
            for (k, v) in t {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(&key, v, out);
            }
        }
        toml::Value::String(s) => {
            out.insert(prefix.to_owned(), s.clone());
        }
        // 文字列でもテーブルでもない値は、実行時の読込 (`Catalog::parse`) が拒否する。印を付けて検査で報告する
        _ => {
            out.insert(prefix.to_owned(), NOT_A_STRING.to_owned());
        }
    }
}

/// `flatten` が文字列でない値に付ける印。
const NOT_A_STRING: &str = "\0not-a-string";

pub fn run() -> Result<(), String> {
    let root = workspace_root();
    println!("▶ i18n");
    let mut errors: Vec<String> = Vec::new();
    let messages = read_messages(&read_text(&root.join("docs/04-tui-design.md"))?)?;
    let screen_keys = read_screen_keys(&read_text(&root.join("docs/04-tui-design.md"))?)?;
    let commands = read_catalog(&read_text(&root.join("docs/07-config-extensibility.md"))?)?;

    // 1. 目録の整合
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for e in &messages {
        if let Some(first) = seen.insert(&e.key, e.line) {
            errors.push(format!(
                "04 §15.2 行 {}: MessageKey `{}` が重複しています (行 {first})",
                e.line, e.key
            ));
        }
        if e.ja.trim().is_empty() {
            errors.push(format!(
                "04 §15.2 行 {}: `{}` の ja が空です",
                e.line, e.key
            ));
        }
        if !braces_balanced(&e.ja) {
            errors.push(format!(
                "04 §15.2 行 {}: `{}` の ja の `{{…}}` が対応していません",
                e.line, e.key
            ));
        }
        if e.key.starts_with("hint.") && cells(&strip_parentheticals(&e.ja)) > HINT_MAX_CELLS {
            errors.push(format!(
                "04 §15.2 行 {}: `{}` のヒントが表示幅 {HINT_MAX_CELLS} セルを超えています",
                e.line, e.key
            ));
        }
    }
    let keys: BTreeSet<&str> = messages.iter().map(|e| e.key.as_str()).collect();
    for k in &keys {
        if let Some(base) = k.strip_prefix("msg.").and_then(|r| r.strip_suffix(".text"))
            && !keys.contains(format!("msg.{base}.hint").as_str())
        {
            errors.push(format!(
                "`msg.{base}.text` に対応する `msg.{base}.hint` がありません (NFR-602)"
            ));
        }
        if let Some(base) = k.strip_prefix("msg.").and_then(|r| r.strip_suffix(".hint"))
            && !keys.contains(format!("msg.{base}.text").as_str())
        {
            errors.push(format!(
                "`msg.{base}.hint` に対応する `msg.{base}.text` がありません (NFR-602)"
            ));
        }
        if k.starts_with("info.") && k.ends_with(".hint") {
            errors.push(format!("`{k}`: info.* は hint を持ちません"));
        }
    }

    // 2. カタログの整合
    let mut cmd_ids: BTreeSet<&str> = BTreeSet::new();
    for c in &commands {
        if c.title_key != format!("cmd.{}", c.id) {
            errors.push(format!(
                "07 §11.6: `{}` の title key は `cmd.{}` のはずですが `{}` です",
                c.id, c.id, c.title_key
            ));
        }
        if !cmd_ids.insert(&c.id) {
            errors.push(format!("07 §11.6: CommandId `{}` が重複しています", c.id));
        }
    }
    println!(
        "  目録 {} キー、v0.1 のコマンド {} 個",
        messages.len(),
        commands.len()
    );

    // 3. 言語ファイル
    let locale_path = root.join("locales/ja.toml");
    if locale_path.exists() {
        let value: toml::Value = toml::from_str(&read_text(&locale_path)?)
            .map_err(|e| format!("locales/ja.toml を読めません: {e}"))?;
        let mut have = BTreeMap::new();
        flatten("", &value, &mut have);
        // `[meta]` は文言ではない
        have.retain(|k, _| !k.starts_with("meta."));
        let mut want: BTreeSet<String> = BTreeSet::new();
        for e in messages
            .iter()
            .filter(|e| !NOT_IN_LOCALE.iter().any(|p| e.key.starts_with(p)))
        {
            // 目録の `msg.<domain>.<name>` は言語ファイルでは `.text` と `.hint` の 2 つ (07 §17.2)
            if e.key.starts_with("msg.") && !e.key.ends_with(".text") && !e.key.ends_with(".hint") {
                want.insert(format!("{}.text", e.key));
                want.insert(format!("{}.hint", e.key));
            } else {
                want.insert(e.key.clone());
            }
        }
        for k in screen_keys
            .iter()
            .filter(|k| !NOT_IN_LOCALE.iter().any(|p| k.starts_with(p)))
        {
            if !k.starts_with("hint.") {
                want.insert(k.clone());
            }
        }
        for c in &commands {
            want.insert(format!("cmd.{}.title", c.id));
            want.insert(format!("cmd.{}.description", c.id));
            if c.hint_contexts.contains("normal") || c.hint_contexts.contains("overlay") {
                want.insert(format!("hint.{}", c.id));
            }
        }
        // 複数形の接尾辞 (.one / .other) を除いた基底キーで比べる
        let base = |k: &str| {
            k.strip_suffix(".one")
                .or_else(|| k.strip_suffix(".other"))
                .unwrap_or(k)
                .to_owned()
        };
        let have_keys: BTreeSet<String> = have.keys().map(|k| base(k)).collect();
        for k in want.difference(&have_keys) {
            errors.push(format!("locales/ja.toml に `{k}` がありません"));
        }
        for k in have_keys.difference(&want) {
            errors.push(format!(
                "locales/ja.toml の `{k}` は目録・カタログにありません (余剰)"
            ));
        }
        let known_ids: BTreeSet<&str> = commands.iter().map(|c| c.id.as_str()).collect();
        for (k, v) in &have {
            if v == NOT_A_STRING {
                errors.push(format!(
                    "locales/ja.toml: `{k}` は文字列でもテーブルでもありません"
                ));
                continue;
            }
            // `{key:<command_id>}` の CommandId は 07 §11.6 のカタログにあるもの (07 §17.3)
            let mut rest = v.as_str();
            while let Some(i) = rest.find("{key:") {
                let tail = &rest[i + 5..];
                let end = tail.find('}').unwrap_or(tail.len());
                let id = &tail[..end];
                if !known_ids.contains(id) {
                    errors.push(format!(
                        "locales/ja.toml: `{k}` の `{{key:{id}}}` はカタログにない CommandId です"
                    ));
                }
                rest = &tail[end..];
            }
            if k.starts_with("hint.") && cells(v) > HINT_MAX_CELLS {
                errors.push(format!("locales/ja.toml: `{k}` のヒントが表示幅 {HINT_MAX_CELLS} セルを超えています ({v})"));
            }
        }
        println!(
            "  locales/ja.toml: {} キーを目録・カタログと突合",
            have.len()
        );
    } else {
        println!(
            "  locales/ja.toml: まだありません (MessageKey を実装するマイルストーンで作る)。言語ファイルとの突合は飛ばしました"
        );
    }

    if errors.is_empty() {
        println!("  OK");
        Ok(())
    } else {
        Err(format!(
            "i18n: {} 件の問題があります\n  - {}",
            errors.len(),
            errors.join("\n  - ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 表示幅の近似() {
        assert_eq!(cells("abc"), 3);
        assert_eq!(cells("保存"), 4);
        assert_eq!(cells("保存a"), 5);
    }

    #[test]
    fn 引数の括弧の対応() {
        assert!(braces_balanced("{path} を保存しました"));
        assert!(!braces_balanced("{path を保存しました"));
        assert!(!braces_balanced("path} を保存しました"));
        assert!(!braces_balanced("{{path}}"));
    }

    #[test]
    fn ヒント列の文脈を読む() {
        let md = "### 11.6 v0.1 CommandId カタログ (付録)\n\n\
| ID | title key | kind | args | when | perm | コンテキスト | 既定キー | ヒント |\n|---|---|---|---|---|---|---|---|---|\n\
| `edit.undo` / `edit.redo` | `cmd.edit.undo` / `cmd.edit.redo` | Edit | — | — | `-` | `editor` | `mod+z` / `mod+y` | normal:2 (undo のみ) |\n\
| `file.save_all` (v0.2) | `cmd.file.save_all` | File | — | — | `-` | `editor` | — | chord:3 |\n";
        let c = read_catalog(md).unwrap();
        assert_eq!(c.len(), 2, "v0.2 の行は含めない");
        assert!(c[0].hint_contexts.contains("normal"), "undo はヒントを持つ");
        assert!(c[1].hint_contexts.is_empty(), "redo はヒントを持たない");
    }
}
