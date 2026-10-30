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

#[test]
fn crash_report_は最新のクラッシュログを表示する() {
    let state = dir("crash");
    let log = state.join("log");
    std::fs::create_dir_all(&log).unwrap();
    let out = |state: &PathBuf| {
        let o = Command::new(EXE)
            .arg("--crash-report")
            .env("EVE_STATE_DIR", state)
            .output()
            .expect("起動できる");
        (
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stdout).into_owned(),
        )
    };
    let (code, text) = out(&state);
    assert_eq!(code, 0);
    assert!(text.contains("No crash logs."), "{text}");
    std::fs::write(log.join("crash-1-100.txt"), "OLD").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    std::fs::write(log.join("crash-2-200.txt"), "NEWEST PANIC").unwrap();
    std::fs::write(log.join("evoeditor.log"), "not a crash log").unwrap();
    let (code, text) = out(&state);
    assert_eq!(code, 0);
    assert!(
        text.contains("crash-2-200.txt") && text.contains("NEWEST PANIC"),
        "{text}"
    );
    assert!(
        !text.contains("OLD") && !text.contains("not a crash log"),
        "{text}"
    );
    assert!(text.contains("/issues"), "{text}");
    let _ = std::fs::remove_dir_all(&state);
}

#[test]
fn licenses_は同梱の第三者告知を表示し_log_level_の誤りは使い方の誤り() {
    let d = dir("lic");
    let (code, out) = run(&d, &["--licenses"]);
    assert_eq!(code, 0);
    assert!(
        out.contains("THIRD-PARTY NOTICES"),
        "{}",
        &out[..out.len().min(200)]
    );
    let (code, _) = run(&d, &["--log-level", "loud"]);
    assert_eq!(code, 2);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn check_config_はテーマファイルも確かめる() {
    let d = dir("theme");
    std::fs::create_dir_all(d.join("themes")).unwrap();
    std::fs::write(d.join("config.toml"), "[theme]\nname = \"mine\"\n").unwrap();
    std::fs::write(
        d.join("themes").join("mine.toml"),
        "[ui]\nselection = { bg = \"nope\" }\n",
    )
    .unwrap();
    let (code, out) = run(&d, &["--check-config"]);
    assert_eq!(code, 0, "値の誤りは警告: {out}");
    assert!(out.contains("themes/mine.toml"), "{out}");
    // 無いテーマはエラー (default で動く)
    std::fs::write(d.join("config.toml"), "[theme]\nname = \"gone\"\n").unwrap();
    let (code, out) = run(&d, &["--check-config"]);
    assert_eq!(code, 3, "{out}");
    let _ = std::fs::remove_dir_all(&d);
}
