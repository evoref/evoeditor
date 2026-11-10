//! 隠しオプション `--list-commands [--json]` / `--dump-keymap [--json]` (docs/07 §18.3 / §11.5 / §7.7、09 §11.2)。
//! 実バイナリを普通の子プロセスとして起動し、JSON の形 (生成リファレンス `cargo xtask gen-reference` の入力) と、
//! ユーザーのキーマップの層が重なることを見る。

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

const EXE: &str = env!("CARGO_BIN_EXE_evoeditor");

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("evo-listing-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 設定ディレクトリを `cfg` にして起動し、(終了コード, 標準出力)。
fn run(cfg: &PathBuf, args: &[&str]) -> (i32, String) {
    let out = Command::new(EXE)
        .args(args)
        .env("EVE_CONFIG_DIR", cfg)
        .env("EVE_LANG", "ja")
        .output()
        .expect("起動できる");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8(out.stdout).expect("UTF-8"),
    )
}

fn json(cfg: &PathBuf, args: &[&str]) -> Value {
    let (code, out) = run(cfg, args);
    assert_eq!(code, 0, "{out}");
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("JSON として読めない: {e}\n{out}"))
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key]
        .as_str()
        .unwrap_or_else(|| panic!("{key} が文字列でない: {v}"))
}

#[test]
fn list_commands_の_json_の形() {
    let d = dir("list");
    let v = json(&d, &["--list-commands", "--json", "--no-config"]);
    assert_eq!(v["format"], 1);
    assert_eq!(v["locale"], "ja");
    let cmds = v["commands"].as_array().expect("commands は配列");
    assert!(cmds.len() >= 63, "v0.1 の 63 個以上: {}", cmds.len());
    let ids: Vec<&str> = cmds.iter().map(|c| str_of(c, "id")).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(ids, sorted, "CommandId の順で重複なし");
    for c in cmds {
        for key in ["id", "title", "description", "kind", "when", "permission"] {
            str_of(c, key);
        }
        assert!(c["hidden"].is_boolean(), "{c}");
        assert!(
            ["Edit", "Motion", "Ui", "File"].contains(&str_of(c, "kind")),
            "{c}"
        );
        assert!(
            ["-", "read", "propose", "ui"].contains(&str_of(c, "permission")),
            "{c}"
        );
        for a in c["args"].as_array().expect("args は配列") {
            str_of(a, "name");
            str_of(a, "type");
            assert!(a["required"].is_boolean(), "{a}");
        }
        for k in c["keys"].as_array().expect("keys は配列") {
            for key in ["context", "key", "label"] {
                str_of(k, key);
            }
            assert!(k["enhanced"].is_boolean(), "{k}");
        }
    }
    let save = cmds.iter().find(|c| c["id"] == "file.save").unwrap();
    assert_eq!(save["title"], "保存");
    assert_eq!(save["when"], "!doc.read_only");
    assert_eq!(save["permission"], "-");
    assert_eq!(save["keys"][0]["key"], "ctrl+s");
    assert_eq!(save["keys"][0]["label"], "Ctrl+S");
    let goto = cmds.iter().find(|c| c["id"] == "cursor.goto_line").unwrap();
    assert_eq!(goto["args"][0]["name"], "line");
    assert_eq!(goto["args"][0]["type"], "int|\"$\"");
    assert_eq!(goto["args"][0]["required"], false);
    let confirm = cmds.iter().find(|c| c["id"] == "ui.confirm").unwrap();
    assert_eq!(confirm["hidden"], true);

    // 英語の UI ではタイトルも英語
    let en = json(
        &d,
        &["--list-commands", "--json", "--no-config", "--locale", "en"],
    );
    assert_eq!(en["locale"], "en");
    let save = en["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "file.save")
        .unwrap();
    assert_eq!(save["title"], "Save");

    // JSON でなければ 1 行 1 コマンド
    let (code, out) = run(&d, &["--list-commands", "--no-config"]);
    assert_eq!(code, 0);
    assert_eq!(out.lines().count(), cmds.len());
    assert!(
        out.lines().any(|l| l.starts_with("file.save\t保存\t")),
        "{out}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn dump_keymap_の_json_の形とユーザーの層() {
    let d = dir("keymap");
    let v = json(&d, &["--dump-keymap", "--json", "--no-config"]);
    assert_eq!(v["format"], 1);
    let bindings = v["bindings"].as_array().expect("bindings は配列");
    assert!(bindings.len() > 100, "{}", bindings.len());
    for b in bindings {
        for key in [
            "context", "key", "label", "command", "title", "when", "layer",
        ] {
            str_of(b, key);
        }
        assert!(b["enhanced"].is_boolean(), "{b}");
        assert_eq!(b["layer"], "default", "--no-config なら既定の層だけ");
    }
    // コンテキストは 07 §7.1 の順にまとまる
    let order = [
        "global", "editor", "search", "prompt", "palette", "list", "dialog", "help",
    ];
    let ranks: Vec<usize> = bindings
        .iter()
        .map(|b| {
            order
                .iter()
                .position(|c| *c == str_of(b, "context"))
                .unwrap()
        })
        .collect();
    assert!(ranks.windows(2).all(|w| w[0] <= w[1]), "{ranks:?}");
    let replace = bindings
        .iter()
        .find(|b| b["command"] == "search.replace")
        .unwrap();
    assert_eq!(replace["when"], "search.replace_focused");

    // ユーザーの keymap.toml を重ねる (付け替えた既定の割当は出さない)
    std::fs::write(
        d.join("keymap.toml"),
        "[keys.editor]\n\"ctrl+s\" = \"file.save_as\"\n\"f5\" = \"file.save\"\n",
    )
    .unwrap();
    let v = json(&d, &["--dump-keymap", "--json"]);
    let bindings = v["bindings"].as_array().unwrap();
    let at = |ctx: &str, key: &str| -> Vec<(String, String)> {
        bindings
            .iter()
            .filter(|b| b["context"] == ctx && b["key"] == key)
            .map(|b| {
                (
                    str_of(b, "command").to_owned(),
                    str_of(b, "layer").to_owned(),
                )
            })
            .collect()
    };
    assert_eq!(
        at("editor", "ctrl+s"),
        [("file.save_as".to_owned(), "user".to_owned())]
    );
    assert_eq!(
        at("editor", "f5"),
        [("file.save".to_owned(), "user".to_owned())]
    );

    // JSON でなければ keymap.toml の書式
    let (code, out) = run(&d, &["--dump-keymap", "--no-config"]);
    assert_eq!(code, 0);
    assert!(
        out.starts_with("[keys.global]\n\"ctrl+q\" = \"app.quit\""),
        "{out}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn json_は出力のオプションの修飾だけ() {
    let d = dir("usage");
    let (code, _) = run(&d, &["--json"]);
    assert_eq!(code, 2, "単独の --json は使い方の誤り (07 §18.4)");
    let (code, _) = run(&d, &["--list-commands", "--defaults"]);
    assert_eq!(code, 2, "--defaults は --dump-config の修飾だけ");
    let _ = std::fs::remove_dir_all(&d);
}
