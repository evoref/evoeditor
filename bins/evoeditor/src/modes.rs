//! 端末を始めずに出力して終わる起動の形 (07 §18.3): `--check-config`・`--dump-config`・`--crash-report`・`--licenses`。

use std::io::Write as _;

use eve_config::Catalog;
use eve_core::KEYMAP_FILE;
use eve_core::Paths as _;

use crate::exit::{EXIT_CONFIG_ERROR, EXIT_ERROR};
use crate::settings::{choose_locale, config_sources, load_config, load_keymap, load_user_theme};
use crate::{NativePaths, cli, texts};

/// 同梱の第三者告知 (`cargo about` が生成する。05 §17.1)。`--licenses` で表示する。
pub(crate) const THIRD_PARTY_NOTICES: &str = include_str!("../../../THIRD-PARTY-NOTICES.md");

/// `--check-config` (07 §18.3): 設定を読んで診断を標準出力へ出し、終える。エラーがあれば終了コード 3、警告だけなら 0。
/// キーマップは書式だけを確かめる (コマンド名などの照合は起動したときの診断で行う)。
pub(crate) fn check_config(args: &cli::ConfigArgs, cli_locale: Option<&str>) -> u8 {
    let paths = match NativePaths::from_process_env() {
        Ok(p) => p,
        Err(_) => return EXIT_ERROR,
    };
    let src = config_sources(&paths, args);
    let resolved = load_config(&src);
    let keymap = load_keymap(&src);
    let locale = choose_locale(cli_locale, &resolved.config).locale;
    let cat = Catalog::builtin(locale);
    let mut reports = resolved.reports.clone();
    if !keymap.diagnostics.is_empty() {
        reports.push((KEYMAP_FILE.to_owned(), keymap.diagnostics.clone()));
    }
    if let Some((_, _, theme_reports)) = load_user_theme(&src, &resolved.config) {
        reports.extend(theme_reports);
    }
    let mut out = std::io::stdout().lock();
    let mut shown = 0;
    let mut error = resolved.has_error;
    for (name, diags) in &reports {
        for d in diags {
            error |= d.severity == eve_config::Severity::Error;
            // 「このバージョンでは未対応」の情報診断も出す (画面とは違い、確かめるための出力なので)
            let (text, hint) = eve_core::config_diagnostic_texts(cat, d);
            let at = if d.line > 0 {
                format!("{name}:{}:{}", d.line, d.column)
            } else {
                name.clone()
            };
            let _ = writeln!(out, "{at}: {text}");
            if !hint.is_empty() {
                let _ = writeln!(out, "  {hint}");
            }
            shown += 1;
        }
    }
    texts::check_config_summary(shown).write_to(&mut out);
    if error { EXIT_CONFIG_ERROR } else { 0 }
}

/// `--crash-report` (09 §10.2): `state_dir/log/` の最新のクラッシュログの場所と内容を表示し、Issue の場所を案内する。
/// 送信はしない (貼り付けるかどうかは利用者が決める)。
pub(crate) fn crash_report() -> u8 {
    let paths = match NativePaths::from_process_env() {
        Ok(p) => p,
        Err(_) => return EXIT_ERROR,
    };
    let dir = paths.state_dir().as_path().join("log");
    let latest = std::fs::read_dir(&dir).ok().and_then(|entries| {
        entries
            .filter_map(Result::ok)
            .filter(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                n.starts_with("crash-") && n.ends_with(".txt")
            })
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .max_by_key(|(t, _)| *t)
            .map(|(_, p)| p)
    });
    let mut out = std::io::stdout().lock();
    let Some(path) = latest else {
        texts::no_crash_log().write_to(&mut out);
        return 0;
    };
    texts::crash_log_header(&path).write_to(&mut out);
    let _ = writeln!(out);
    match std::fs::read(&path) {
        Ok(bytes) => {
            let _ = writeln!(out, "{}", String::from_utf8_lossy(&bytes).trim_end());
        }
        Err(_) => return EXIT_ERROR,
    }
    let _ = writeln!(out);
    texts::crash_report_guide(&format!("{}/issues", env!("CARGO_PKG_REPOSITORY")))
        .write_to(&mut out);
    0
}

/// `--dump-config` (07 §18.3): 解決済みの設定 (`--defaults` なら既定値) を TOML (`--json` なら JSON) で出し、終える。
pub(crate) fn dump_config(args: &cli::ConfigArgs, defaults: bool, json: bool) -> u8 {
    let config = if defaults {
        eve_config::Config::default()
    } else {
        let paths = match NativePaths::from_process_env() {
            Ok(p) => p,
            Err(_) => return EXIT_ERROR,
        };
        load_config(&config_sources(&paths, args)).config
    };
    print!("{}", eve_config::dump(&config, json));
    0
}
