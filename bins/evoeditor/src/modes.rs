//! 端末を始めずに出力して終わる起動の形 (07 §18.3): `--check-config`・`--dump-config`・`--crash-report`・`--licenses`。

use std::io::Write as _;

use eve_config::Catalog;
use eve_core::KEYMAP_FILE;
use eve_core::Paths as _;

use crate::exit::{EXIT_CONFIG_ERROR, EXIT_ERROR};
use crate::settings::{
    LocaleRead, LocaleSetup, config_sources, load_config, load_keymap, load_user_theme,
    setup_locale,
};
use crate::{NativePaths, cli, texts};

/// 同梱の第三者告知 (`cargo about` が生成する。05 §17.1)。`--licenses` で表示する。
pub(crate) const THIRD_PARTY_NOTICES: &str = include_str!("../../../THIRD-PARTY-NOTICES.md");

/// `--check-config` の先頭の 1 行 (`ui.config.locale_summary`、07 §17.4): 使う言語・選んだ段・ユーザーの言語ファイルの
/// 絶対パス (無ければ省く)・補う言語。
fn locale_summary(cat: &Catalog, setup: &LocaleSetup) -> String {
    let source = match setup.choice.source {
        eve_config::LocaleSource::Cli => "cli",
        eve_config::LocaleSource::Config => "config",
        eve_config::LocaleSource::Env => "env",
        eve_config::LocaleSource::Os => "os",
        eve_config::LocaleSource::Default => "default",
    };
    let source = cat.render(&format!("ui.config.locale_source.{source}"), &[], &|_| None);
    let file = setup.file().map_or_else(String::new, |p| {
        let path = std::path::absolute(p)
            .unwrap_or_else(|_| p.clone())
            .display()
            .to_string();
        // ファイル全体の問題で使っていないときは、そう添える
        if setup.unusable {
            let unused = cat.render("ui.config.locale_unused", &[], &|_| None);
            format!("{path} {unused}")
        } else {
            path
        }
    });
    let locale = setup.choice.locale;
    cat.render(
        "ui.config.locale_summary",
        &[
            ("lang", locale.code()),
            ("source", &source),
            ("file", &file),
            ("base", locale.base().code()),
        ],
        &|_| None,
    )
}

/// `--check-config` (07 §18.3): 設定を読んで診断を標準出力へ出し、終える。エラーがあれば終了コード 3、警告だけなら 0。
/// キーマップは書式だけを確かめる (コマンド名などの照合は起動したときの診断で行う)。選ばれる言語のユーザーの言語ファイルも
/// 検査し、先頭に言語の選択の結果を 1 行出す (07 §17.4)。文言は画面と同じく選んだ言語の表 (ユーザーの言語ファイルを
/// 重ねた表) で出し、先頭の行と言語ファイル・言語の選択の診断は、補う言語の **同梱の** 表で出す (壊れた訳で読めなくならない
/// ように)。標準出力へ出す行の全体を無害化する (Frame を通らないため、07 §4.3)。
pub(crate) fn check_config(args: &cli::ConfigArgs, cli_locale: Option<&str>) -> u8 {
    let paths = match NativePaths::from_process_env() {
        Ok(p) => p,
        Err(_) => return EXIT_ERROR,
    };
    let src = config_sources(&paths, args);
    let mut resolved = load_config(&src);
    let keymap = load_keymap(&src);
    let setup = setup_locale(&src, cli_locale, &mut resolved, LocaleRead::CHECK);
    let builtin = Catalog::builtin(setup.choice.locale);
    let mut out = std::io::stdout().lock();
    let line = |out: &mut std::io::StdoutLock<'_>, s: &str| {
        let _ = writeln!(out, "{}", eve_config::escape_untrusted(s));
    };
    line(&mut out, &locale_summary(builtin, &setup));
    let mut reports = resolved.reports.clone();
    if !keymap.diagnostics.is_empty() {
        reports.push((KEYMAP_FILE.to_owned(), keymap.diagnostics.clone()));
    }
    if let Some((_, _, theme_reports)) =
        load_user_theme(&src, &resolved.config, resolved.theme_name_at.as_ref())
    {
        eve_config::merge_reports(&mut reports, theme_reports);
    }
    let mut error = resolved.has_error || setup.has_error();
    let locale_reports = setup.reports.len();
    reports.extend(setup.reports.iter().cloned());
    let first_locale = reports.len() - locale_reports;
    let mut shown = 0;
    for (i, (name, diags)) in reports.iter().enumerate() {
        for d in diags {
            error |= d.severity == eve_config::Severity::Error;
            // 「このバージョンでは未対応」の情報診断も出す (画面とは違い、確かめるための出力なので)。言語ファイル・言語の
            // 選択の診断 (`setup_locale` の診断と、言語ファイルの種類の診断) は補う言語の同梱の文言で出す
            let cat = if i >= first_locale || matches!(d.kind, eve_config::DiagKind::Locale(_)) {
                builtin
            } else {
                &*setup.catalog
            };
            let (text, hint) = eve_core::config_diagnostic_texts_long(cat, d);
            let at = if d.line > 0 {
                format!("{name}:{}:{}", d.line, d.column)
            } else {
                name.clone()
            };
            line(&mut out, &format!("{at}: {text}"));
            if !hint.is_empty() {
                line(&mut out, &format!("  {hint}"));
            }
            // 件数: 「ほか N 件」は N 件と数え、訳の無いキー (情報) は数えない
            shown += match &d.kind {
                eve_config::DiagKind::Locale(
                    eve_config::LocaleProblem::Untranslated { .. }
                    | eve_config::LocaleProblem::UntranslatedKey,
                ) => 0,
                _ => eve_core::config_problem_count(&[d]),
            };
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
