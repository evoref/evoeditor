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

use eve_docs_tables::strip_parentheticals;

use crate::util::{read_text, workspace_root};

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

/// かな・漢字 (半角カナを含む)。〔 〕・矢印・★・… などの記号は含めない。
fn is_cjk_text(c: char) -> bool {
    matches!(c as u32, 0x3041..=0x30FF | 0x3400..=0x9FFF | 0xFF66..=0xFF9F)
}

/// 文言の `{name}` と `{key:id}` の名前の集合 (`{{` は除く)。読み方は言語ファイルの解決と同じ (eve-config)。
fn placeholders(s: &str) -> BTreeSet<String> {
    eve_config::placeholders(s)
        .into_iter()
        .map(str::to_owned)
        .collect()
}

/// 文言の `[X]` (大文字 1 字の角括弧) の並び。確認ダイアログのアクセラレータ。
fn accelerators(s: &str) -> Vec<char> {
    let cs: Vec<char> = s.chars().collect();
    cs.windows(3)
        .filter(|w| w[0] == '[' && w[2] == ']' && w[1].is_ascii_uppercase())
        .map(|w| w[1])
        .collect()
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
    let list = eve_docs_tables::messages::read_catalog(doc04)
        .map_err(|e| format!("04 §15.2 の目録を読めません: {e}"))?;
    Ok(list
        .into_iter()
        .map(|m| Entry {
            key: m.key,
            ja: m.ja,
            line: m.line,
        })
        .collect())
}

/// 04 §15.1 の画面文言の MessageKey (`a.b.long` / `.short` の略記は展開したもの)。
fn read_screen_keys(doc04: &str) -> Result<BTreeSet<String>, String> {
    let list = eve_docs_tables::messages::read_screen(doc04)
        .map_err(|e| format!("04 §15.1 の画面文言を読めません: {e}"))?;
    Ok(list.into_iter().map(|m| m.key).collect())
}

/// カタログの行: (CommandId, title key, ヒントの文脈の集合)。
struct Command {
    id: String,
    title_key: String,
    hint_contexts: BTreeSet<String>,
    /// 後の版の注記 (`(v0.2)` 等) が付いた行。言語ファイルにあってもなくてもよい (実装した版で足す)。
    later: bool,
}

fn read_catalog(doc07: &str) -> Result<Vec<Command>, String> {
    let rows = eve_docs_tables::catalog::read(doc07)
        .map_err(|e| format!("07 §11.6 のカタログを読めません: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        // ヒント列: `文脈:優先度` を `, ` で区切る。`(<ID の最後の語> のみ)` は一部の ID に限る。
        let mut all_contexts = BTreeSet::new();
        let mut per_id: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for part in row.hints.split(", ") {
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
        for (i, (id, title_key)) in row.ids.iter().zip(&row.title_keys).enumerate() {
            let mut hint_contexts = all_contexts.clone();
            for (word, extra) in &per_id {
                if row.id_index_by_last_word(word) == Some(i) {
                    hint_contexts.extend(extra.iter().cloned());
                }
            }
            out.push(Command {
                id: id.clone(),
                title_key: title_key.clone(),
                hint_contexts,
                later: row.later,
            });
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
    let all_commands = read_catalog(&read_text(&root.join("docs/07-config-extensibility.md"))?)?;
    // 必須なのは v0.1 の行。後の版の行は、実装した版で言語ファイルに足してよい (許すが要求しない)
    let commands: Vec<&Command> = all_commands.iter().filter(|c| !c.later).collect();

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

    // 3. 言語ファイル (同梱の ja と en。キー集合は目録・カタログと一致し、en は ja とも一致する)
    let mut haves: BTreeMap<&str, BTreeMap<String, String>> = BTreeMap::new();
    for lang in ["ja", "en"] {
        let locale_path = root.join(format!("locales/{lang}.toml"));
        if locale_path.exists() {
            let value: toml::Value = toml::from_str(&read_text(&locale_path)?)
                .map_err(|e| format!("locales/{lang}.toml を読めません: {e}"))?;
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
                if e.key.starts_with("msg.")
                    && !e.key.ends_with(".text")
                    && !e.key.ends_with(".hint")
                {
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
            let mut optional: BTreeSet<String> = BTreeSet::new();
            for c in &all_commands {
                let set = if c.later { &mut optional } else { &mut want };
                set.insert(format!("cmd.{}.title", c.id));
                set.insert(format!("cmd.{}.description", c.id));
                if c.hint_contexts.contains("normal") || c.hint_contexts.contains("overlay") {
                    set.insert(format!("hint.{}", c.id));
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
                errors.push(format!("locales/{lang}.toml に `{k}` がありません"));
            }
            for k in have_keys
                .difference(&want)
                .filter(|k| !optional.contains(*k))
            {
                errors.push(format!(
                    "locales/{lang}.toml の `{k}` は目録・カタログにありません (余剰)"
                ));
            }
            let known_ids: BTreeSet<&str> = all_commands.iter().map(|c| c.id.as_str()).collect();
            for (k, v) in &have {
                if v == NOT_A_STRING {
                    errors.push(format!(
                        "locales/{lang}.toml: `{k}` は文字列でもテーブルでもありません"
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
                        "locales/{lang}.toml: `{k}` の `{{key:{id}}}` はカタログにない CommandId です"
                    ));
                    }
                    rest = &tail[end..];
                }
                // 言語ごとの文字の規則: en にかな・漢字を入れない (〔 〕・矢印・★ などの記号は可)
                if lang == "en" && v.chars().any(is_cjk_text) {
                    errors.push(format!(
                        "locales/{lang}.toml: `{k}` に日本語の文字があります ({v})"
                    ));
                }
                if k.starts_with("hint.") && cells(v) > HINT_MAX_CELLS {
                    errors.push(format!("locales/{lang}.toml: `{k}` のヒントが表示幅 {HINT_MAX_CELLS} セルを超えています ({v})"));
                }
            }
            println!(
                "  locales/{lang}.toml: {} キーを目録・カタログと突合",
                have.len()
            );
            haves.insert(lang, have);
        } else {
            println!(
                "  locales/{lang}.toml: まだありません (MessageKey を実装するマイルストーンで作る)。言語ファイルとの突合は飛ばしました"
            );
        }
    }

    // 同梱の言語の間: 同じキーは同じプレースホルダ (`{name}`・`{key:id}`) を持つ。位置の違いは言語の自由 (07 §17.2)
    if let (Some(ja), Some(en)) = (haves.get("ja"), haves.get("en")) {
        for (k, jv) in ja {
            if let Some(ev) = en.get(k) {
                let (pj, pe) = (placeholders(jv), placeholders(ev));
                if pj != pe {
                    errors.push(format!(
                        "locales/en.toml: `{k}` のプレースホルダが ja と違います (ja: {pj:?} / en: {pe:?})"
                    ));
                }
                // 〔 〕 (条件付きの文) の有無も揃える
                if jv.contains('〔') != ev.contains('〔') {
                    errors.push(format!(
                        "locales/en.toml: `{k}` の 〔 〕 が ja と揃っていません"
                    ));
                }
                // 確認ダイアログの選択肢の `[X]` (アクセラレータ) の文字は言語で変えない (コードが決めている)
                if accelerators(jv) != accelerators(ev) {
                    errors.push(format!(
                        "locales/en.toml: `{k}` の `[X]` のアクセラレータが ja と違います"
                    ));
                }
            }
        }
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
    #[test]
    fn プレースホルダの集合() {
        let p = placeholders("{path} を {{ 開く }} {key:file.save} {count}");
        let v: Vec<&str> = p.iter().map(String::as_str).collect();
        assert_eq!(v, ["count", "key:file.save", "path"]);
        assert!(placeholders("なし").is_empty());
    }

    #[test]
    fn アクセラレータの並び() {
        assert_eq!(accelerators("[S] 保存して終了"), ['S']);
        assert_eq!(accelerators("`[K] a` ★ / `[C] b`"), ['K', 'C']);
        let actual = accelerators("[Esc] Cancel");
        assert!(actual.is_empty(), "{actual:?}");
        let actual = accelerators("[a] x");
        assert!(actual.is_empty(), "{actual:?}");
    }

    #[test]
    fn かな漢字の判定() {
        assert!(is_cjk_text('あ') && is_cjk_text('漢') && is_cjk_text('ｱ'));
        for c in ['a', '〔', '〕', '→', '★', '…', 'é'] {
            assert!(!is_cjk_text(c), "{c}");
        }
    }

    use super::*;

    // verifies: AC-NFR-603-1
    // verifies: AC-NFR-603-2
    // verifies: AC-NFR-603-5
    #[test]
    fn このリポジトリの文言の目録と言語ファイルが検査を通る() {
        run().expect("cargo xtask i18n と同じ検査");
    }

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
        assert_eq!(c.len(), 3);
        assert!(c[0].hint_contexts.contains("normal"), "undo はヒントを持つ");
        assert!(c[1].hint_contexts.is_empty(), "redo はヒントを持たない");
        assert!(
            !c[0].later && !c[1].later && c[2].later,
            "v0.2 の行は後の版"
        );
    }
}
