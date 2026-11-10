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

// verifies: FR-701
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

// verifies: FR-701
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

// ---------- 外から見える出力のゴールデン (docs/review/v0.3-findings.md の STR-T1、v0.3 の M13) ----------
// 比べ方は eve_testkit::golden と同じ (bin は eve-testkit に依存しない決まり、deps-policy.toml)。
// `EVE_UPDATE_GOLDEN=1` で作る・書き換える。既にあるゴールデンの書き換えは人間が判断する (11 §6.2)。

fn golden(name: &str, actual: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    let actual = actual.replace("\r\n", "\n");
    if std::env::var_os("EVE_UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &actual).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| {
            panic!(
                "ゴールデン {} がありません (EVE_UPDATE_GOLDEN=1 で作る)",
                path.display()
            )
        })
        .replace("\r\n", "\n");
    assert_eq!(actual, want, "ゴールデン {} と違います", path.display());
}

#[test]
fn ゴールデン_既定の設定の_json() {
    let d = dir("golden-dump");
    let (code, out) = run(&d, &["--dump-config", "--defaults", "--json"]);
    assert_eq!(code, 0, "{out}");
    golden("dump_config_defaults.json", &out);
}

#[test]
fn ゴールデン_help() {
    let out = Command::new(EXE)
        .arg("--help")
        .output()
        .expect("起動できる");
    assert_eq!(out.status.code(), Some(0));
    golden("help.txt", &String::from_utf8_lossy(&out.stdout));
}

#[test]
fn ゴールデン_使い方の誤り() {
    let out = Command::new(EXE)
        .arg("--no-such-option")
        .output()
        .expect("起動できる");
    assert_eq!(
        out.status.code(),
        Some(2),
        "使い方の誤りは終了コード 2 (07 §18.4)"
    );
    golden("usage_error.txt", &String::from_utf8_lossy(&out.stderr));
}
