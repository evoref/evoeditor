//! 端末を使わない設定のオプション (docs/07 §18.3 の `--check-config` / `--dump-config` と `--config` / `--no-config` /
//! `--set`)。実バイナリを普通の子プロセスとして起動し、標準出力と終了コード (07 §18.4) を見る。

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::PathBuf;
use std::process::Command;

const EXE: &str = env!("CARGO_BIN_EXE_evoeditor");

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("evo-cli-{}-{name}", std::process::id()));
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
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[test]
fn check_config_は警告なら_0_エラーなら_3() {
    let d = dir("check");
    std::fs::write(d.join("config.toml"), "[editor]\ntab_width = 99\n").unwrap();
    let (code, out) = run(&d, &["--check-config"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("config.toml:2:"), "{out}");
    assert!(out.contains("設定に 1 件の問題があります"), "{out}");

    std::fs::write(d.join("config.toml"), "[editor\n").unwrap();
    let (code, _) = run(&d, &["--check-config"]);
    assert_eq!(code, 3);

    // --no-config なら読まない
    let (code, out) = run(&d, &["--check-config", "--no-config"]);
    assert_eq!(code, 0);
    assert!(out.contains("設定に問題はありません"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn dump_config_は層を重ねた結果を出す() {
    let d = dir("dump");
    std::fs::write(d.join("config.toml"), "[editor]\ntab_width = 8\n").unwrap();
    let other = d.join("other.toml");
    std::fs::write(&other, "[editor]\nindent_width = 2\n").unwrap();
    let (code, out) = run(&d, &["--dump-config"]);
    assert_eq!(code, 0);
    assert!(out.contains("tab_width = 8"), "{out}");
    // --config は config.toml の代わりに読み、--set は最後に重ねる
    let (_, out) = run(
        &d,
        &[
            "--dump-config",
            "--config",
            other.to_str().unwrap(),
            "--set",
            "view.wrap=soft",
        ],
    );
    assert!(out.contains("tab_width = 4"), "{out}");
    assert!(out.contains("indent_width = 2"), "{out}");
    assert!(out.contains("wrap = \"soft\""), "{out}");
    let (_, out) = run(&d, &["--dump-config", "--defaults", "--json"]);
    assert!(out.contains("\"editor.tab_width\": 4"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn set_の形の誤りは使い方の誤り() {
    let d = dir("set");
    let (code, _) = run(&d, &["--set", "novalue", "--check-config"]);
    assert_eq!(code, 2);
    let _ = std::fs::remove_dir_all(&d);
}
