//! `cargo xtask gen-reference`: 生成リファレンス `docs/reference/*` の生成と検査 (docs/09 §11.2、11 §2.2)。
//!
//! bin (`bins/evoeditor`) をビルドして、隠しオプションの JSON を出させ、Markdown に変換する。
//! - `docs/reference/commands.md` ← `evoeditor --list-commands --json`
//! - `docs/reference/keymap.md` ← `evoeditor --dump-keymap --json`
//! - `docs/reference/config.md` ← `evoeditor --dump-config --defaults --json`
//!
//! どれも `--no-config --locale ja` を付けて出させる (利用者の設定と環境の言語に左右されない。生成物は日本語)。
//! `--check` は書き換えずに、生成した結果が今のファイルと一致することと、07 §11.6 のカタログとの突合を確かめる。
//! **カタログ (仕様) が正、生成物は実装出力** (ADR-0025)。突合で差があれば失敗し、どちらを直すかは人が判断する。
//! 突合の項目: (1) 版の注記の無い行の CommandId はすべて登録され、登録済みのコマンドはカタログのどれかの行にある。
//! (2) 既定キーマップの (コマンド, キー) の組が、カタログの「既定キー」列と一致する (注記付きの行の割当は、あってもなくてもよい)。
//! (3) 拡張キー (`[E]`) の印が一致する。(4) 割当を置くコンテキストの集合が一致する (「コンテキスト」列の意味、07 §11.6)。
//! (4) の差のうち、どちらを直すかを判断して報告したもの (実装を直す作業が残るもの) だけは、`CONTEXT_EXCEPTIONS` の表で
//! 警告にとどめる (PO 判断 2026-11-07、09 §11.2)。表にあるのに差が無くなった行は、表から外すよう失敗にする。

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

use serde::Deserialize;

use crate::util::{read_text, run_capture, workspace_root};

const COMMANDS: &str = "docs/reference/commands.md";
const KEYMAP: &str = "docs/reference/keymap.md";
const CONFIG: &str = "docs/reference/config.md";

/// 出力の形の版 (`bins/evoeditor/src/listing.rs` の `FORMAT`)。
const FORMAT: u32 = 1;

#[derive(Debug, Deserialize)]
struct CommandList {
    format: u32,
    commands: Vec<CommandEntry>,
}

#[derive(Debug, Deserialize)]
struct CommandEntry {
    id: String,
    title: String,
    kind: String,
    args: Vec<ArgEntry>,
    when: String,
    permission: String,
    hidden: bool,
    keys: Vec<KeyEntry>,
}

#[derive(Debug, Deserialize)]
struct ArgEntry {
    name: String,
    #[serde(rename = "type")]
    ty: String,
    required: bool,
}

#[derive(Debug, Deserialize)]
struct KeyEntry {
    context: String,
    label: String,
    enhanced: bool,
}

#[derive(Debug, Deserialize)]
struct KeymapDump {
    format: u32,
    bindings: Vec<BindingEntry>,
}

#[derive(Debug, Deserialize)]
struct BindingEntry {
    context: String,
    key: String,
    label: String,
    command: String,
    title: String,
    when: String,
    layer: String,
    enhanced: bool,
}

/// `--dump-config --json` の平らな表 (キー, 値の JSON の表記)。並びは出力の順 (07 §5 の表の順) のまま。
#[derive(Debug)]
struct ConfigDump(Vec<(String, String)>);

impl<'de> Deserialize<'de> for ConfigDump {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = ConfigDump;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("設定のキーと値の表")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut m: A,
            ) -> Result<ConfigDump, A::Error> {
                let mut out = Vec::new();
                while let Some((k, v)) = m.next_entry::<String, serde_json::Value>()? {
                    out.push((k, v.to_string()));
                }
                Ok(ConfigDump(out))
            }
        }
        d.deserialize_map(V)
    }
}

/// Markdown の表のセルの中の `|` をエスケープする。
fn cell(s: &str) -> String {
    s.replace('|', "\\|")
}

/// インライン コード (中にバッククォートがあれば 2 つで囲む)。
fn code(s: &str) -> String {
    if s.contains('`') {
        format!("`` {s} ``")
    } else {
        format!("`{s}`")
    }
}

fn or_dash(s: String) -> String {
    if s.is_empty() { "—".to_owned() } else { s }
}

const NOTE: &str = "生成物 (`cargo xtask gen-reference` で作り直す。手で直さない。docs/09 §11.2)。";
const CATALOG_LINK: &str =
    "[07 §11.6 のカタログ](../07-config-extensibility.md#116-v01-commandid-カタログ-付録)";

/// `docs/reference/commands.md`。
fn render_commands(list: &CommandList) -> String {
    let mut m = String::from("# コマンド一覧\n\n");
    let _ = write!(
        m,
        "{NOTE}`evoeditor --list-commands --json` の出力 (組込みのコマンド {} 個。CommandId の順) から作る。\
         コマンドと既定キーの正本は {CATALOG_LINK} で、`cargo xtask gen-reference --check` が突合する。\n\n",
        list.commands.len()
    );
    m.push_str("- **種類**・**権限** は 07 §11.6 の kind / perm 列の表記 (権限の `-` は外部の主体から呼べない)。**when** の `—` は常に実行できる。\n");
    m.push_str("- **引数** は `名前` (`?` は省略可能): 型 (07 §11.2)。\n");
    m.push_str("- **既定キー** は組込みの既定キーマップの割当 (コンテキストごと。画面の表記、04 §7.8)。`[E]` は拡張キーが無い端末では届かない割当。利用者のキーマップを重ねた結果は `evoeditor --dump-keymap` で見る。\n");
    m.push_str(
        "- **パレット** の「出さない」は `hidden` (オーバーレイ内専用と内部用、07 §11.6)。\n\n",
    );
    m.push_str("| ID | タイトル | 種類 | 引数 | when | 権限 | 既定キー | パレット |\n");
    m.push_str("|---|---|---|---|---|---|---|---|\n");
    for c in &list.commands {
        let args = c
            .args
            .iter()
            .map(|a| {
                format!(
                    "{}: {}",
                    code(&format!("{}{}", a.name, if a.required { "" } else { "?" })),
                    code(&a.ty)
                )
            })
            .collect::<Vec<_>>()
            .join("<br>");
        // 既定キー: コンテキストの初出の順に、同じコンテキストの割当をまとめる
        let mut by_ctx: Vec<(&str, Vec<String>)> = Vec::new();
        for k in &c.keys {
            let label = format!("{}{}", code(&k.label), if k.enhanced { " [E]" } else { "" });
            match by_ctx.iter_mut().find(|(ctx, _)| *ctx == k.context) {
                Some((_, v)) => v.push(label),
                None => by_ctx.push((&k.context, vec![label])),
            }
        }
        let keys = by_ctx
            .iter()
            .map(|(ctx, v)| format!("{}: {}", code(ctx), v.join(", ")))
            .collect::<Vec<_>>()
            .join("<br>");
        let when = if c.when.is_empty() {
            String::new()
        } else {
            code(&c.when)
        };
        let _ = writeln!(
            m,
            "| {} | {} | {} | {} | {} | {} | {} | {} |",
            code(&c.id),
            cell(&or_dash(c.title.clone())),
            c.kind,
            cell(&or_dash(args)),
            cell(&or_dash(when)),
            code(&c.permission),
            cell(&or_dash(keys)),
            if c.hidden { "出さない" } else { "出す" }
        );
    }
    m
}

/// `docs/reference/keymap.md`。
fn render_keymap(km: &KeymapDump) -> String {
    let mut m = String::from("# 既定キーマップ\n\n");
    let _ = write!(
        m,
        "{NOTE}`evoeditor --dump-keymap --json` (`--no-config`。組込みの既定キーマップ、割当 {} 件) の出力から作る。\
         既定キーの正本は {CATALOG_LINK} の「既定キー」列で、`cargo xtask gen-reference --check` が突合する。\n\n",
        km.bindings.len()
    );
    m.push_str("- コンテキストは 07 §7.1 の順。同じコンテキストの中は優先の順 (層の順・登録の順)。キーを探すときの参照の順 (`search` → `prompt` → `global` など) は 07 §7.1。\n");
    m.push_str("- **キー** は画面の表記 (04 §7.8)、**設定の表記** は `keymap.toml` に書く形 (07 §7.2)。**拡張キー** の「要」は拡張キーが無い端末では届かない割当 (07 §11.6 の `[E]`)。\n\n");
    let mut i = 0;
    while i < km.bindings.len() {
        let ctx = &km.bindings[i].context;
        let _ = writeln!(m, "## {}\n", code(ctx));
        m.push_str("| キー | 設定の表記 | コマンド | タイトル | when | 拡張キー |\n");
        m.push_str("|---|---|---|---|---|---|\n");
        while i < km.bindings.len() && km.bindings[i].context == *ctx {
            let b = &km.bindings[i];
            let when = if b.when.is_empty() {
                String::new()
            } else {
                code(&b.when)
            };
            let _ = writeln!(
                m,
                "| {} | {} | {} | {} | {} | {} |",
                cell(&code(&b.label)),
                cell(&code(&b.key)),
                code(&b.command),
                cell(&or_dash(b.title.clone())),
                cell(&or_dash(when)),
                if b.enhanced { "要" } else { "—" }
            );
            i += 1;
        }
        m.push('\n');
    }
    m.truncate(m.trim_end().len());
    m.push('\n');
    m
}

/// `docs/reference/config.md`。
fn render_config(c: &ConfigDump) -> String {
    let mut m = String::from("# 設定リファレンス\n\n");
    let _ = writeln!(
        m,
        "{NOTE}`evoeditor --dump-config --defaults --json` の出力 (公開キー {} 個と既定値) から作る。\
         キー・型・許容値・意味の正本は [07 §5](../07-config-extensibility.md#5-設定項目一覧-スキーマと既定値)。\
         既定値は `config.toml` にそのまま書ける表記 (TOML の値)。今の設定を重ねた結果は `evoeditor --dump-config` で見る。",
        c.0.len()
    );
    let mut section = "";
    for (k, v) in &c.0 {
        let (sec, rest) = k.split_once('.').unwrap_or(("", k.as_str()));
        if sec != section {
            let _ = write!(
                m,
                "\n## {}\n\n| キー | 既定値 |\n|---|---|\n",
                code(&format!("[{sec}]"))
            );
            section = sec;
        }
        let _ = writeln!(
            m,
            "| {} | {} |",
            code(&format!("{sec}.{rest}")),
            cell(&code(v))
        );
    }
    m
}

/// 設定の表記のキーを比べられる形にする (`mod` は TUI では `ctrl`、修飾は `ctrl` `alt` `shift` `super` の順、小文字。
/// 大文字 1 字は `shift+小文字`。07 §7.2)。
fn normalize_keys(keys: &str) -> String {
    keys.split_whitespace()
        .map(|stroke| {
            let mut parts: Vec<&str> = stroke.split('+').collect();
            let key = parts.pop().unwrap_or_default();
            let mut mods: BTreeSet<usize> = BTreeSet::new();
            for p in parts {
                let i = match p.to_ascii_lowercase().as_str() {
                    "ctrl" | "mod" => 0,
                    "alt" => 1,
                    "shift" => 2,
                    _ => 3,
                };
                mods.insert(i);
            }
            let mut chars = key.chars();
            let key = match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_uppercase() => {
                    mods.insert(2);
                    c.to_lowercase().collect()
                }
                _ => key.to_ascii_lowercase(),
            };
            let names = ["ctrl", "alt", "shift", "super"];
            let mut s: String = mods.iter().map(|&i| format!("{}+", names[i])).collect();
            s.push_str(&key);
            s
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// コンテキストの差を警告にとどめる (コマンド, 設定の表記のキー, 理由)。実装かカタログを直したら行を消す。
/// 今は空 (2026-11-07 の 12 件は実装を直した)。表と警告の仕組みは、次に判断を待つ差が出たときのために残す。
const CONTEXT_EXCEPTIONS: &[(&str, &str, &str)] = &[];

/// 07 §11.6 のカタログとの突合: (誤り, 警告)。`exceptions` はコンテキストの差を警告にとどめる (コマンド, キー, 理由)。
fn compare_catalog(
    doc07: &str,
    list: &CommandList,
    km: &KeymapDump,
    exceptions: &[(&str, &str, &str)],
) -> Result<(Vec<String>, Vec<String>), String> {
    let rows = eve_docs_tables::catalog::read(doc07)
        .map_err(|e| format!("07 §11.6 のカタログを読めません: {e}"))?;
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    // (1) CommandId
    let listed: BTreeSet<&str> = list.commands.iter().map(|c| c.id.as_str()).collect();
    let mut all_ids = BTreeSet::new();
    for r in &rows {
        for id in &r.ids {
            all_ids.insert(id.as_str());
            if !r.later && !listed.contains(id.as_str()) {
                errors.push(format!(
                    "07 行 {}: `{id}` (版の注記なし) が登録されていません (--list-commands に無い)",
                    r.line
                ));
            }
        }
    }
    for id in &listed {
        if !all_ids.contains(id) {
            errors.push(format!(
                "登録済みの `{id}` が 07 §11.6 のカタログにありません"
            ));
        }
    }

    // (2) (コマンド, キー) の組、(3) 拡張キーの印、(4) コンテキスト
    struct Want {
        line: usize,
        later: bool,
        enhanced: bool,
        contexts: BTreeSet<String>,
    }
    let mut want: BTreeMap<(String, String), Want> = BTreeMap::new();
    for r in &rows {
        let row_contexts: BTreeSet<String> =
            eve_docs_tables::identifiers(&eve_docs_tables::strip_parentheticals(&r.contexts))
                .into_iter()
                .collect();
        for b in eve_docs_tables::catalog::bindings(r)? {
            let contexts = match b.context {
                Some(c) => BTreeSet::from([c]),
                None => row_contexts.clone(),
            };
            let e = want
                .entry((b.id.clone(), normalize_keys(&b.keys)))
                .or_insert(Want {
                    line: r.line,
                    later: r.later,
                    enhanced: false,
                    contexts: BTreeSet::new(),
                });
            e.enhanced |= b.enhanced;
            e.contexts.extend(contexts);
        }
    }
    let mut used: BTreeSet<(String, String)> = BTreeSet::new();
    let mut have: BTreeMap<(String, String), (bool, BTreeSet<String>)> = BTreeMap::new();
    for b in km.bindings.iter().filter(|b| b.layer == "default") {
        let e = have
            .entry((b.command.clone(), b.key.clone()))
            .or_insert((false, BTreeSet::new()));
        e.0 |= b.enhanced;
        e.1.insert(b.context.clone());
    }
    for ((id, key), w) in &want {
        match have.get(&(id.clone(), key.clone())) {
            None if !w.later => errors.push(format!(
                "07 行 {}: `{id}` の既定キー `{key}` が既定キーマップにありません",
                w.line
            )),
            None => {}
            Some((enhanced, contexts)) => {
                if *enhanced != w.enhanced {
                    errors.push(format!(
                        "07 行 {}: `{id}` の `{key}` の拡張キーの印: 実装 {} / カタログ {}",
                        w.line,
                        if *enhanced { "[E]" } else { "なし" },
                        if w.enhanced { "[E]" } else { "なし" }
                    ));
                }
                if *contexts != w.contexts {
                    let msg = format!(
                        "07 行 {}: `{id}` の `{key}` のコンテキスト: 実装 {:?} / カタログ {:?}",
                        w.line, contexts, w.contexts
                    );
                    match exceptions.iter().find(|(c, k, _)| c == id && k == key) {
                        Some((_, _, why)) => {
                            used.insert((id.clone(), key.clone()));
                            warnings.push(format!("{msg} (例外: {why})"));
                        }
                        None => errors.push(msg),
                    }
                }
            }
        }
    }
    for (id, key) in have.keys() {
        if !want.contains_key(&(id.clone(), key.clone())) {
            errors.push(format!(
                "既定キーマップの `{id}` の `{key}` が 07 §11.6 のカタログの既定キー列にありません"
            ));
        }
    }
    for (c, k, _) in exceptions {
        if !used.contains(&((*c).to_owned(), (*k).to_owned())) {
            errors.push(format!(
                "コンテキストの例外 `{c}` の `{k}` は差が無くなった (または組が無い): CONTEXT_EXCEPTIONS から外してください"
            ));
        }
    }
    Ok((errors, warnings))
}

/// bin を (必要ならビルドして) 実行し、標準出力を返す。
fn bin_output(args: &[&str]) -> Result<String, String> {
    run_capture(
        Command::new("cargo")
            .args(["run", "--quiet", "-p", "evoeditor", "--"])
            .args(args)
            .args(["--no-config", "--locale", "ja"]),
    )
}

fn parse<T: for<'de> Deserialize<'de>>(what: &str, json: &str) -> Result<T, String> {
    serde_json::from_str(json).map_err(|e| format!("{what} の JSON を読めません: {e}"))
}

/// 生成物 (パス, 内容) と、突合の (誤り, 警告)。
#[allow(clippy::type_complexity)]
fn build(root: &Path) -> Result<(Vec<(&'static str, String)>, Vec<String>, Vec<String>), String> {
    let list: CommandList = parse(
        "--list-commands",
        &bin_output(&["--list-commands", "--json"])?,
    )?;
    let km: KeymapDump = parse("--dump-keymap", &bin_output(&["--dump-keymap", "--json"])?)?;
    let config: ConfigDump = parse(
        "--dump-config",
        &bin_output(&["--dump-config", "--defaults", "--json"])?,
    )?;
    for (what, f) in [
        ("--list-commands", list.format),
        ("--dump-keymap", km.format),
    ] {
        if f != FORMAT {
            return Err(format!(
                "{what} の出力の形の版 {f} は知らない版です (xtask は {FORMAT})"
            ));
        }
    }
    let doc07 = read_text(&root.join("docs/07-config-extensibility.md"))?;
    let (errors, warnings) = compare_catalog(&doc07, &list, &km, CONTEXT_EXCEPTIONS)?;
    let files = vec![
        (COMMANDS, render_commands(&list)),
        (KEYMAP, render_keymap(&km)),
        (CONFIG, render_config(&config)),
    ];
    Ok((files, errors, warnings))
}

pub fn run(args: &[String]) -> Result<(), String> {
    let check = args.iter().any(|a| a == "--check");
    println!("▶ gen-reference{}", if check { " --check" } else { "" });
    let root = workspace_root();
    let (files, mut errors, warnings) = build(&root)?;
    for w in &warnings {
        println!("  警告: {w}");
    }
    for (path, text) in &files {
        let full = root.join(path);
        if check {
            let have = std::fs::read_to_string(&full)
                .unwrap_or_default()
                .replace("\r\n", "\n");
            if have != *text {
                errors.push(format!(
                    "{path} が実装の出力と違います: `cargo xtask gen-reference` で生成物を更新してコミットしてください"
                ));
            }
        } else {
            if let Some(dir) = full.parent() {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            std::fs::write(&full, text).map_err(|e| format!("{path}: {e}"))?;
            println!("  {path} を書きました");
        }
    }
    if errors.is_empty() {
        println!(
            "  OK (07 §11.6 のカタログとの突合の警告 {} 件)",
            warnings.len()
        );
        Ok(())
    } else {
        Err(format!(
            "gen-reference に {} 件の問題があります (カタログが正。どちらを直すかを判断する、docs/09 §11.2)\n  - {}",
            errors.len(),
            errors.join("\n  - ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = r#"{
  "format": 1,
  "locale": "ja",
  "commands": [
    { "id": "file.save", "title": "保存", "description": "d", "kind": "File", "args": [], "when": "!doc.read_only",
      "permission": "-", "hidden": false,
      "keys": [ { "context": "editor", "key": "ctrl+s", "label": "Ctrl+S", "enhanced": false },
                { "context": "search", "key": "ctrl+s", "label": "Ctrl+S", "enhanced": false } ] },
    { "id": "file.open", "title": "開く", "description": "", "kind": "File",
      "args": [ { "name": "path", "type": "path", "required": false } ], "when": "a || b",
      "permission": "-", "hidden": true,
      "keys": [ { "context": "editor", "key": "ctrl+shift+o", "label": "Ctrl+Shift+O", "enhanced": true } ] }
  ]
}"#;

    const KEYMAP_JSON: &str = r#"{
  "format": 1,
  "locale": "ja",
  "bindings": [
    { "context": "global", "key": "ctrl+q", "label": "Ctrl+Q", "command": "app.quit", "title": "終了", "when": "", "layer": "default", "enhanced": false },
    { "context": "editor", "key": "ctrl+k |", "label": "Ctrl+K |", "command": "x.y", "title": "", "when": "search.active", "layer": "default", "enhanced": true }
  ]
}"#;

    #[test]
    fn コマンドの一覧の表() {
        let list: CommandList = parse("t", LIST).unwrap();
        let md = render_commands(&list);
        assert!(md.starts_with("# コマンド一覧\n\n生成物"), "{md}");
        assert!(md.contains("組込みのコマンド 2 個"), "{md}");
        assert!(md.contains(
            "| `file.save` | 保存 | File | — | `!doc.read_only` | `-` | `editor`: `Ctrl+S`<br>`search`: `Ctrl+S` | 出す |\n"
        ), "{md}");
        assert!(md.contains(
            "| `file.open` | 開く | File | `path?`: `path` | `a \\|\\| b` | `-` | `editor`: `Ctrl+Shift+O` [E] | 出さない |\n"
        ), "{md}");
    }

    #[test]
    fn キーマップの表はコンテキストごと() {
        let km: KeymapDump = parse("t", KEYMAP_JSON).unwrap();
        let md = render_keymap(&km);
        assert!(md.contains("## `global`\n\n| キー | 設定の表記 |"), "{md}");
        assert!(
            md.contains("| `Ctrl+Q` | `ctrl+q` | `app.quit` | 終了 | — | — |\n"),
            "{md}"
        );
        assert!(
            md.contains("| `Ctrl+K \\|` | `ctrl+k \\|` | `x.y` | — | `search.active` | 要 |\n"),
            "{md}"
        );
        assert!(md.ends_with("|\n"), "末尾の空行を残さない: {md:?}");
    }

    #[test]
    fn 設定の表は出力の順のまま節ごと() {
        let c: ConfigDump =
            parse("t", "{\n  \"editor.tab_width\": 4,\n  \"editor.a\": \"x\",\n  \"file.b\": [\"p\", \"q\"]\n}\n")
                .unwrap();
        let md = render_config(&c);
        let editor = md.find("## `[editor]`").unwrap();
        let file = md.find("## `[file]`").unwrap();
        assert!(editor < file);
        assert!(
            md.contains("| `editor.tab_width` | `4` |\n| `editor.a` | `\"x\"` |\n"),
            "{md}"
        );
        assert!(md.contains("| `file.b` | `[\"p\",\"q\"]` |\n"), "{md}");
    }

    #[test]
    fn キーの表記をそろえる() {
        assert_eq!(normalize_keys("mod+shift+s"), "ctrl+shift+s");
        assert_eq!(normalize_keys("shift+mod+S"), "ctrl+shift+s");
        assert_eq!(normalize_keys("ctrl+k shift+left"), "ctrl+k shift+left");
        assert_eq!(normalize_keys("alt+ctrl+PageDown"), "ctrl+alt+pagedown");
    }

    fn doc07(rows: &str) -> String {
        use eve_docs_tables::catalog::{COLUMNS, HEADING};
        format!(
            "{HEADING}\n\n| {} |\n|{}\n{rows}\n### 11.7 次\n",
            COLUMNS.join(" | "),
            "---|".repeat(COLUMNS.len())
        )
    }

    #[test]
    fn カタログとの突合() {
        let list: CommandList = parse("t", LIST).unwrap();
        let km: KeymapDump = parse(
            "t",
            r#"{ "format": 1, "locale": "ja", "bindings": [
              { "context": "editor", "key": "ctrl+s", "label": "", "command": "file.save", "title": "", "when": "", "layer": "default", "enhanced": false },
              { "context": "search", "key": "ctrl+s", "label": "", "command": "file.save", "title": "", "when": "", "layer": "default", "enhanced": false },
              { "context": "editor", "key": "ctrl+o", "label": "", "command": "file.open", "title": "", "when": "", "layer": "default", "enhanced": false },
              { "context": "editor", "key": "f9", "label": "", "command": "file.open", "title": "", "when": "", "layer": "user", "enhanced": false }
            ] }"#,
        )
        .unwrap();
        let doc = doc07(concat!(
            "| `file.save` | `cmd.file.save` | File | — | — | `-` | `editor` | `mod+s` | - |\n",
            "| `file.open` | `cmd.file.open` | File | — | — | `-` | `editor` | `mod+o` [E], `mod+n` | - |\n",
            "| `file.new` | `cmd.file.new` | File | — | — | `-` | `editor` | — | - |\n",
            "| `file.later` (v0.9) | `cmd.file.later` | File | — | — | `-` | `editor` | `f5` | - |",
        ));
        let exceptions = [
            ("file.save", "ctrl+s", "試験の例外"),
            ("file.save", "f1", "差の無い例外"),
        ];
        let (errors, warnings) = compare_catalog(&doc, &list, &km, &exceptions).unwrap();
        let joined = errors.join("\n");
        assert!(
            joined.contains("`file.new` (版の注記なし) が登録されていません"),
            "{joined}"
        );
        assert!(
            joined.contains("`file.open` の既定キー `ctrl+n` が既定キーマップにありません"),
            "{joined}"
        );
        assert!(
            !joined.contains("f5"),
            "注記付きの行の割当は無くてよい: {joined}"
        );
        assert!(!joined.contains("f9"), "ユーザーの層は突合しない: {joined}");
        assert!(
            joined.contains("`file.open` の `ctrl+o` の拡張キーの印"),
            "{joined}"
        );
        assert!(
            joined.contains("`file.save` の `f1` は差が無くなった"),
            "{joined}"
        );
        assert_eq!(errors.len(), 4, "{joined}");
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            warnings[0].contains("`file.save` の `ctrl+s` のコンテキスト")
                && warnings[0].contains("試験の例外"),
            "{warnings:?}"
        );

        // 例外の表に無いコンテキストの差は失敗
        let (errors, warnings) = compare_catalog(&doc, &list, &km, &[]).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(
            errors
                .iter()
                .any(|e| e.contains("`file.save` の `ctrl+s` のコンテキスト")),
            "{errors:?}"
        );
    }
}
