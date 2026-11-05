//! 外から見える出力のゴールデン (docs/review/v0.3-findings.md の STR-T1、v0.3 の M13)。
//!
//! v0.3 の整理 (M14〜M16) で振る舞いが変わらないことを確かめるため、登録したコマンドの一覧と既定のキーマップを
//! テキストで固定する。比べ方と書き換え方は `eve_testkit::golden` (`EVE_UPDATE_GOLDEN=1` で作る・書き換える)。

use std::fmt::Write as _;
use std::path::PathBuf;

use eve_core::format_stroke;
use eve_testkit::{Harness, golden};

fn path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name)
}

/// 登録したコマンド (ID・種類・権限・`when` の式)。ID の順。
#[test]
fn コマンドの一覧() {
    let h = Harness::new();
    let mut out = String::from("# id\tkind\tpermission\twhen\n");
    for s in h.editor.registry().iter() {
        let when = if s.when_src.is_empty() {
            "—"
        } else {
            &s.when_src
        };
        let _ = writeln!(out, "{}\t{:?}\t{:?}\t{when}", s.id, s.kind, s.permission);
    }
    golden::check(&path("commands.tsv"), &out);
}

/// 既定のキーマップ (コンテキスト・キーの列・コマンド・引数・`when`・拡張キーボードの印)。層の優先順・登録の順。
#[test]
fn 既定のキーマップ() {
    let h = Harness::new();
    let mut out = String::from("# context\tkeys\tcommand\targs\twhen\tenhanced\n");
    for b in h.editor.keymap().all_bindings() {
        let keys: Vec<String> = b.sequence.iter().map(|k| format_stroke(*k)).collect();
        let _ = writeln!(
            out,
            "{}\t{}\t{}\t{:?}\t{:?}\t{}",
            b.context,
            keys.join(" "),
            b.command,
            b.args,
            b.when,
            b.enhanced
        );
    }
    golden::check(&path("default_keymap.tsv"), &out);
}

/// 組込みテーマを色深度ごとに解決したロールの色と属性 (07 §8.6・04 §6)。テーマ・段・ロールの順。
#[test]
fn 組込みテーマの解決結果() {
    use eve_config::theme::{BUILTIN_NAMES, Depth, builtin, resolve_roles};
    let mut out = String::from("# theme\tdepth\trole\tfg\tbg\tmods\n");
    for name in BUILTIN_NAMES {
        let theme = builtin(name).expect("組込みテーマ");
        for depth in [Depth::TrueColor, Depth::Ansi256, Depth::Ansi16, Depth::Mono] {
            for (role, s) in resolve_roles(&theme, depth) {
                let _ = writeln!(
                    out,
                    "{name}\t{depth:?}\t{role}\t{:?}\t{:?}\t{:#04x}",
                    s.fg, s.bg, s.mods
                );
            }
        }
    }
    golden::check(&path("builtin_themes.tsv"), &out);
}

/// 壊れた設定・キーマップ・テーマの診断と、その文言 (ja / en)。`--check-config` と起動時の通知が使う文言 (07 §3.5)。
#[test]
fn 設定の診断の文言() {
    use eve_config::{Catalog, Diagnostic, load_keymap_str, load_str, theme_file::load_theme};

    let configs: &[(&str, &str)] = &[
        ("構文の誤り", "[editor\ntab_width = 4\n"),
        ("知らない節", "[edtor]\ntab_width = 4\n"),
        ("知らないキー", "[editor]\ntab_widht = 4\n"),
        ("型の違い", "[editor]\ntab_width = \"4\"\n"),
        ("節が表でない", "editor = 1\n"),
        ("範囲の外", "[editor]\ntab_width = 0\n"),
        ("許されない値", "[editor]\nbackspace_unit = \"word\"\n"),
        ("未対応の節", "[ai]\nenabled = true\n"),
        ("正しい設定", "[editor]\ntab_width = 8\n"),
    ];
    let keymaps: &[(&str, &str)] = &[
        ("キーマップの構文の誤り", "[keys.editor\n"),
        (
            "キーマップの各種の誤り",
            concat!(
                "[keys]\npreset = \"none\"\n",
                "[keys.nowhere]\n\"ctrl+s\" = \"file.save\"\n",
                "[keys.editor]\n",
                "\"ctrl+s\" = \"no.such_command\"\n",
                "\"ctrl+nope\" = \"file.save\"\n",
                "\"f5\" = { command = \"file.save\", when = \"doc.modified &&\" }\n",
                "\"a\" = \"file.save\"\n",
                "\"ctrl+k\" = \"file.save\"\n",
                "\"ctrl+q\" = \"none\"\n",
                "\"f6\" = { command = \"file.save\", args = { a = 1 } }\n",
            ),
        ),
    ];
    let themes: &[(&str, &str)] = &[
        ("cycle_a", "[meta]\ninherits = \"cycle_b\"\n"),
        ("cycle_b", "[meta]\ninherits = \"cycle_a\"\n"),
        ("broken", "[ui\n"),
        (
            "values",
            "[ui]\nselection = { fg = \"nope\", bg = \"#123456\" }\nnot_a_role = { fg = \"#ffffff\" }\n[syntax]\ncomment = { fg = \"#888888\" }\n",
        ),
    ];

    let mut cases: Vec<(String, Vec<Diagnostic>)> = Vec::new();
    for (label, text) in configs {
        cases.push((format!("config: {label}"), load_str(text).diagnostics));
    }
    for (label, text) in keymaps {
        let mut h = Harness::new();
        let diags = h.editor.set_user_keymap(&load_keymap_str(text));
        cases.push((format!("keymap: {label}"), diags));
    }
    let read = |n: &str| -> Result<Option<String>, String> {
        Ok(themes
            .iter()
            .find(|(k, _)| *k == n)
            .map(|(_, v)| (*v).to_owned()))
    };
    for name in ["cycle_a", "missing", "broken", "values"] {
        let (theme, reports) = load_theme(name, &read);
        for (file, diags) in reports {
            cases.push((
                format!("theme: {name} ({file}, 使える: {})", theme.is_some()),
                diags,
            ));
        }
    }

    for (lang, cat) in [("ja", Catalog::builtin_ja()), ("en", Catalog::builtin_en())] {
        let mut out = String::new();
        for (label, diags) in &cases {
            let _ = writeln!(out, "## {label}");
            if diags.is_empty() {
                out.push_str("(診断なし)\n");
            }
            for d in diags {
                let (body, action) = eve_core::config_diagnostic_texts(cat, d);
                let _ = writeln!(
                    out,
                    "{}:{}\t{:?}\t{}\t{:?}\n  {body}",
                    d.line, d.column, d.severity, d.key, d.value
                );
                // 対処の無い診断 (未対応の情報) は、`--check-config` と同じく行を出さない
                if !action.is_empty() {
                    let _ = writeln!(out, "  {action}");
                }
            }
        }
        golden::check(&path(&format!("config_diagnostics.{lang}.txt")), &out);
    }
}
