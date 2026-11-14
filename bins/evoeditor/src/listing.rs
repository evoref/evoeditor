//! 隠しオプション `--list-commands [--json]` と `--dump-keymap [--json]` (07 §11.5 / §7.7 / §18.3、09 §11.2)。
//! 端末を始めずに、組込みのコマンドの一覧と解決後のキーマップを標準出力へ出して終える。生成リファレンス
//! (`cargo xtask gen-reference` → `docs/reference/*`) と、07 §11.6 のカタログとの突合の入力になる。
//!
//! JSON の形 (`format` = 1。項目を足すときは同じ版のまま後ろに足し、意味を変えるときは版を上げる):
//! - `--list-commands --json`: `{ "format", "locale", "commands": [ { "id", "title", "description", "kind", "args":
//!   [ { "name", "type", "required" } ], "when", "permission", "hidden", "keys": [ { "context", "key", "label",
//!   "enhanced" } ] } ] }`。コマンドは `CommandId` の順、`keys` は組込みの既定キーマップの割当 (07 §7.1 のコンテキストの順、その中は層の順・登録の順)。
//! - `--dump-keymap --json`: `{ "format", "locale", "bindings": [ { "context", "key", "label", "command", "title",
//!   "when", "layer", "enhanced" } ] }`。コンテキストは 07 §7.1 の順、同じコンテキストの中は層の優先順・登録の順。
//!
//! `key` は設定の表記 (`ctrl+k s`。`keymap.toml` にそのまま書ける)、`label` は画面の表記 (`eve_view::key_notation`、04 §7.8)。
//! `permission` は 07 §11.6 の perm 列の表記 (`-` / `read` / `propose` / `ui`)。`when` の空は常に真。

use std::fmt::Write as _;

use eve_config::Catalog;
use eve_core::{
    Binding, CommandKind, KEYMAP_CONTEXTS, Keymap, Permission, builtin_registry, command_args,
    command_hidden, resolve_keymap,
};
use eve_view::{KeyCode, KeyStroke, Modifiers};

use crate::exit::EXIT_ERROR;
use crate::settings::{LocaleRead, config_sources, load_config, load_keymap, setup_locale};
use crate::{NativePaths, cli};

/// 出力の形の版。
const FORMAT: u32 = 1;

/// JSON の文字列の表記 (`"` と `\` と制御文字をエスケープする)。
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// キー 1 打の設定の表記 (07 §7.2: 小文字、修飾は `ctrl` `alt` `shift` `super` の順、`+` は `plus`、空白は `space`)。
fn config_stroke(k: KeyStroke) -> String {
    let mut s = String::new();
    for (m, name) in [
        (Modifiers::CTRL, "ctrl"),
        (Modifiers::ALT, "alt"),
        (Modifiers::SHIFT, "shift"),
        (Modifiers::SUPER, "super"),
    ] {
        if k.mods.contains(m) {
            s.push_str(name);
            s.push('+');
        }
    }
    match k.key {
        KeyCode::Char(' ') => s.push_str("space"),
        KeyCode::Char('+') => s.push_str("plus"),
        KeyCode::Char(c) => s.extend(c.to_lowercase()),
        KeyCode::F(n) => {
            let _ = write!(s, "f{n}");
        }
        KeyCode::Enter => s.push_str("enter"),
        KeyCode::Tab => s.push_str("tab"),
        KeyCode::Esc => s.push_str("esc"),
        KeyCode::Backspace => s.push_str("backspace"),
        KeyCode::Delete => s.push_str("delete"),
        KeyCode::Insert => s.push_str("insert"),
        KeyCode::Home => s.push_str("home"),
        KeyCode::End => s.push_str("end"),
        KeyCode::PageUp => s.push_str("pageup"),
        KeyCode::PageDown => s.push_str("pagedown"),
        KeyCode::Up => s.push_str("up"),
        KeyCode::Down => s.push_str("down"),
        KeyCode::Left => s.push_str("left"),
        KeyCode::Right => s.push_str("right"),
        KeyCode::Menu => s.push_str("menu"),
        KeyCode::CapsLock => s.push_str("capslock"),
        KeyCode::PrintScreen => s.push_str("printscreen"),
        // 後から足したキー (`KeyCode` は `non_exhaustive`)。表記が決まるまでは `?`
        _ => s.push('?'),
    }
    s
}

/// キーの列の設定の表記 (チョードは空白区切り)。
fn config_keys(seq: &[KeyStroke]) -> String {
    seq.iter()
        .map(|k| config_stroke(*k))
        .collect::<Vec<_>>()
        .join(" ")
}

fn kind_name(k: CommandKind) -> &'static str {
    match k {
        CommandKind::Edit => "Edit",
        CommandKind::Motion => "Motion",
        CommandKind::Ui => "Ui",
        CommandKind::File => "File",
    }
}

fn perm_name(p: Permission) -> &'static str {
    match p {
        Permission::None => "-",
        Permission::Read => "read",
        Permission::Propose => "propose",
        Permission::Ui => "ui",
    }
}

/// 割当 1 件。
struct Row<'a> {
    layer: &'static str,
    b: &'a Binding,
}

impl Row<'_> {
    fn key(&self) -> String {
        config_keys(&self.b.sequence)
    }

    fn label(&self) -> String {
        eve_view::key_notation(&self.b.sequence, false)
    }
}

/// 割当を 07 §7.1 のコンテキストの順に並べる (同じコンテキストの中は層の優先順・登録の順のまま)。
fn sorted_rows(km: &Keymap) -> Vec<Row<'_>> {
    let rank = |ctx: &str| {
        KEYMAP_CONTEXTS
            .iter()
            .position(|c| *c == ctx)
            .unwrap_or(KEYMAP_CONTEXTS.len())
    };
    let mut rows: Vec<Row<'_>> = km
        .listed_bindings()
        .into_iter()
        .map(|(layer, b)| Row { layer, b })
        .collect();
    rows.sort_by_key(|r| rank(r.b.context));
    rows
}

/// コマンドの一覧 (`--list-commands`)。キーは組込みの既定キーマップの割当。
pub(crate) fn list_commands_text(cat: &Catalog, locale: &str, json: bool) -> String {
    let registry = builtin_registry();
    let (km, _) = resolve_keymap(&eve_config::KeymapFile::default());
    let rows: Vec<(&'static str, &Binding)> = sorted_rows(&km)
        .into_iter()
        .map(|r| (r.layer, r.b))
        .collect();
    let title = |id: &str| cat.get(&format!("cmd.{id}.title")).unwrap_or("").to_owned();
    let mut out = String::new();
    if !json {
        for spec in registry.iter() {
            let id = spec.id.as_str();
            let keys: Vec<String> = rows
                .iter()
                .filter(|(_, b)| b.command.as_str() == id)
                .map(|(_, b)| {
                    format!(
                        "{}: {}",
                        b.context,
                        eve_view::key_notation(&b.sequence, false)
                    )
                })
                .collect();
            let _ = writeln!(out, "{id}\t{}\t{}", title(id), keys.join(", "));
        }
        return out;
    }
    let _ = write!(
        out,
        "{{\n  \"format\": {FORMAT},\n  \"locale\": {},\n  \"commands\": [",
        json_str(locale)
    );
    for (i, spec) in registry.iter().enumerate() {
        let id = spec.id.as_str();
        let args: Vec<String> = command_args(id)
            .iter()
            .map(|a| {
                format!(
                    "{{ \"name\": {}, \"type\": {}, \"required\": {} }}",
                    json_str(a.name),
                    json_str(&a.ty.notation()),
                    a.required
                )
            })
            .collect();
        let keys: Vec<String> = rows
            .iter()
            .filter(|(_, b)| b.command.as_str() == id)
            .map(|(_, b)| {
                format!(
                    "{{ \"context\": {}, \"key\": {}, \"label\": {}, \"enhanced\": {} }}",
                    json_str(b.context),
                    json_str(&config_keys(&b.sequence)),
                    json_str(&eve_view::key_notation(&b.sequence, false)),
                    b.enhanced
                )
            })
            .collect();
        let list = |items: &[String]| {
            if items.is_empty() {
                "[]".to_owned()
            } else {
                format!("[\n        {}\n      ]", items.join(",\n        "))
            }
        };
        let _ = write!(
            out,
            "{}\n    {{\n      \"id\": {},\n      \"title\": {},\n      \"description\": {},\n      \"kind\": {},\n      \"args\": {},\n      \"when\": {},\n      \"permission\": {},\n      \"hidden\": {},\n      \"keys\": {}\n    }}",
            if i == 0 { "" } else { "," },
            json_str(id),
            json_str(&title(id)),
            json_str(cat.get(&format!("cmd.{id}.description")).unwrap_or("")),
            json_str(kind_name(spec.kind)),
            list(&args),
            json_str(&spec.when_src),
            json_str(perm_name(spec.permission)),
            command_hidden(id),
            list(&keys),
        );
    }
    out.push_str("\n  ]\n}\n");
    out
}

/// 解決後のキーマップ (`--dump-keymap`)。JSON でなければ `keymap.toml` の書式 (07 §7.1 / §7.5)。
pub(crate) fn dump_keymap_text(cat: &Catalog, locale: &str, km: &Keymap, json: bool) -> String {
    let rows = sorted_rows(km);
    let title = |id: &str| cat.get(&format!("cmd.{id}.title")).unwrap_or("").to_owned();
    let mut out = String::new();
    if json {
        let _ = write!(
            out,
            "{{\n  \"format\": {FORMAT},\n  \"locale\": {},\n  \"bindings\": [",
            json_str(locale)
        );
        for (i, r) in rows.iter().enumerate() {
            let id = r.b.command.as_str();
            let _ = write!(
                out,
                "{}\n    {{ \"context\": {}, \"key\": {}, \"label\": {}, \"command\": {}, \"title\": {}, \"when\": {}, \"layer\": {}, \"enhanced\": {} }}",
                if i == 0 { "" } else { "," },
                json_str(r.b.context),
                json_str(&r.key()),
                json_str(&r.label()),
                json_str(id),
                json_str(&title(id)),
                json_str(&r.b.when_src),
                json_str(r.layer),
                r.b.enhanced,
            );
        }
        out.push_str("\n  ]\n}\n");
        return out;
    }
    // TOML: `[keys.<context>]` ごとに `"キー" = 値`。同じキーに複数の割当 (`when` 違い) があれば配列にする
    let mut i = 0;
    while i < rows.len() {
        let ctx = rows[i].b.context;
        if !out.is_empty() {
            out.push('\n');
        }
        let _ = writeln!(out, "[keys.{ctx}]");
        let mut j = i;
        while j < rows.len() && rows[j].b.context == ctx {
            j += 1;
        }
        let group = &rows[i..j];
        let mut done = vec![false; group.len()];
        for (a, r) in group.iter().enumerate() {
            if done[a] {
                continue;
            }
            let same: Vec<usize> = (a..group.len())
                .filter(|&k| !done[k] && group[k].b.sequence == r.b.sequence)
                .collect();
            let values: Vec<String> = same
                .iter()
                .map(|&k| {
                    done[k] = true;
                    let b = group[k].b;
                    if b.when_src.is_empty() {
                        json_str(b.command.as_str())
                    } else {
                        format!(
                            "{{ command = {}, when = {} }}",
                            json_str(b.command.as_str()),
                            json_str(&b.when_src)
                        )
                    }
                })
                .collect();
            let value = if values.len() == 1 {
                values[0].clone()
            } else {
                format!("[{}]", values.join(", "))
            };
            let notes: Vec<String> = same
                .iter()
                .map(|&k| {
                    let g = &group[k];
                    format!("{} ({})", title(g.b.command.as_str()), g.layer)
                })
                .collect();
            let _ = writeln!(
                out,
                "{} = {value}  # {}",
                json_str(&r.key()),
                notes.join(", ")
            );
        }
        i = j;
    }
    out
}

/// UI の言語の選択 (`--locale` > `ui.locale` > `EVE_LANG` > OS、07 §17.1) と、そのカタログ (画面と同じく、ユーザーの
/// 言語ファイルを重ねた表。`--no-config` では同梱の表だけ。07 §17.4)。言語の名前は JSON の `locale` に出す。診断は出さない
/// (`--check-config` で見る)。
fn locale_of(
    paths: &NativePaths,
    args: &cli::ConfigArgs,
    cli_locale: Option<&str>,
) -> (std::sync::Arc<Catalog>, String) {
    let src = config_sources(paths, args);
    let mut resolved = load_config(&src);
    let setup = setup_locale(&src, cli_locale, &mut resolved, LocaleRead::STARTUP);
    (setup.catalog, setup.choice.locale.code().to_owned())
}

/// `--list-commands [--json]`: 組込みのコマンドの一覧を出し、終える。
pub(crate) fn list_commands(args: &cli::ConfigArgs, cli_locale: Option<&str>, json: bool) -> u8 {
    let Ok(paths) = NativePaths::from_process_env() else {
        return EXIT_ERROR;
    };
    let (cat, code) = locale_of(&paths, args, cli_locale);
    print!("{}", list_commands_text(&cat, &code, json));
    0
}

/// `--dump-keymap [--json]`: 組込みの既定キーマップにユーザーの `keymap.toml` を重ねた解決後のキーマップを出し、終える
/// (`--no-config` なら既定だけ)。キーマップの診断は出さない (`--check-config` で見る)。
pub(crate) fn dump_keymap(args: &cli::ConfigArgs, cli_locale: Option<&str>, json: bool) -> u8 {
    let Ok(paths) = NativePaths::from_process_env() else {
        return EXIT_ERROR;
    };
    let (cat, code) = locale_of(&paths, args, cli_locale);
    let file = load_keymap(&config_sources(&paths, args));
    let (km, _) = resolve_keymap(&file);
    print!("{}", dump_keymap_text(&cat, &code, &km, json));
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_の文字列のエスケープ() {
        assert_eq!(json_str("a\"b\\c\n\u{1}"), "\"a\\\"b\\\\c\\n\\u0001\"");
    }

    #[test]
    fn キーの設定の表記は読み直せる() {
        for keys in [
            "ctrl+k s",
            "ctrl+shift+p",
            "shift+f3",
            "ctrl+k <",
            "ctrl+pagedown",
            "alt+up",
            "f1",
        ] {
            let seq = eve_core::parse_sequence(keys).unwrap();
            assert_eq!(config_keys(&seq), keys);
        }
        let plus = eve_core::parse_sequence("ctrl+plus").unwrap();
        assert_eq!(config_keys(&plus), "ctrl+plus");
        assert_eq!(
            config_keys(&eve_core::parse_sequence("mod+s").unwrap()),
            "ctrl+s"
        );
    }

    #[test]
    fn キーマップの_toml_はキーマップのファイルとして読める() {
        let (km, _) = resolve_keymap(&eve_config::KeymapFile::default());
        let text = dump_keymap_text(Catalog::builtin_ja(), "ja", &km, false);
        let file = eve_config::load_keymap_str(&text);
        assert!(file.diagnostics.is_empty(), "{:?}", file.diagnostics);
        assert_eq!(file.entries.len(), km.listed_bindings().len());
    }
}
