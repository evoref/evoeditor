//! evoeditor の実行ファイル (docs/02 §4.1、§9.5)。引数の解析と、本番の部品での Host の組み立て・起動だけを行う。
//! 編集のロジックは core、端末は eve-tui、OS は eve-platform にある。
//!
//! 現在の範囲 (M2 の E4): 起動・描画・入力・終了。保存・文字コード (`--encoding`)・設定の読込は後の版 (M3 / M4)。

#![forbid(unsafe_code)]
// 実行ファイルは、端末を初期化する前後に標準出力・標準エラーへ書く (ヘルプ・版・使い方の誤り・終了時の行)。
#![allow(clippy::print_stdout, clippy::print_stderr)]

mod cli;
mod texts;

use std::io::Write as _;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::process::ExitCode as ProcessExit;
use std::rc::Rc;
use std::sync::Arc;

use eve_config::Catalog;
use eve_core::RescueReason;
use eve_core::{Encoding, JobPorts, Paths as _, Ports};
use eve_host::{Host, HostMsg, SignalKind, SystemClock, ThreadJobSpawner, channel};
use eve_platform::dirs::WinPaths;
use eve_platform::fs::WinFs;
use eve_platform::rescue::WinRescue;
use eve_tui::frontend::TuiFrontend;

use cli::{Cli, UsageError};

/// パニックの内容 (フックが書き、復元の後に出す)。メッセージ・場所・バックトレース。文書の内容は含まない。
static PANIC_INFO: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// 実行時エラーでの終了 (07 §18.4)。
const EXIT_ERROR: u8 = 1;
/// 使い方の誤り。
const EXIT_USAGE: u8 = 2;
/// `--check-config` でエラーの診断があった。
const EXIT_CONFIG_ERROR: u8 = 3;
/// 端末の初期化に失敗 (`Host::run` が返す。ここでは使わない)。
#[allow(dead_code)]
const EXIT_TERMINAL: u8 = 5;
/// パニック。
const EXIT_PANIC: u8 = 70;

/// 端末の復元の後に出す行 (`Effect::Quit` の `stderr` と起動の失敗)。定数表 (英語の行、日本語の行の順) にあるキーはそれで、
/// 無ければ選択中の言語のカタログで解決し、それも無ければキー名を出す。
fn print_exit_lines(host: &mut Host, locale: eve_config::Locale) {
    let lines = host.take_exit_stderr();
    if lines.is_empty() {
        return;
    }
    let cat = Catalog::builtin(locale);
    let mut err = std::io::stderr().lock();
    for l in lines {
        let args: Vec<(&str, &str)> = l
            .args
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        match texts::stderr_line(&l.key, &l.args) {
            Some(bi) => bi.write_to(&mut err),
            None => {
                let _ = writeln!(err, "{}", cat.render(&l.key, &args, &|_| None));
            }
        }
    }
}

use eve_core::{CONFIG_FILE, KEYMAP_FILE};

/// 設定ファイル 1 つを読む。無ければ `Ok(None)`、読めない (UTF-8 でない・権限がない等) ときは理由。
/// 起動時は端末を始める前なので、ここで直接読む (`config.reload` は io ジョブで読む)。
fn read_source(path: &std::path::Path) -> Result<Option<String>, String> {
    match std::fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| "UTF-8 として読めません".to_owned()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// 設定の読込元 (07 §18.3): 既定は設定ディレクトリの `config.toml` と `keymap.toml`。`--config` はユーザー層の
/// `config.toml` の代わりに指定のファイルを順に重ね、`--no-config` はユーザー層を読まない。`--set` は最後に重ねる。
fn config_sources(paths: &WinPaths, args: &cli::ConfigArgs) -> eve_core::ConfigSources {
    let dir = paths.config_dir();
    let has_dir = !dir.as_path().as_os_str().is_empty();
    let files = if args.no_config {
        Vec::new()
    } else if !args.files.is_empty() {
        args.files
            .iter()
            .map(|f| {
                let path = paths
                    .normalize(f)
                    .unwrap_or_else(|_| eve_core::FsPath::new(f));
                (f.clone(), path)
            })
            .collect()
    } else if has_dir {
        vec![(CONFIG_FILE.to_owned(), dir.join(CONFIG_FILE))]
    } else {
        Vec::new()
    };
    eve_core::ConfigSources {
        files,
        keymap: (!args.no_config && has_dir).then(|| dir.join(KEYMAP_FILE)),
        sets: args.sets.clone(),
        themes: (!args.no_config && has_dir).then(|| dir.join("themes")),
    }
}

/// `theme.name` が組込みでなければ、テーマファイル (`themes/<name>.toml`、`inherits` の親も) を読む (07 §8.6)。
/// 戻り値は (名前, テーマ (読めなければ `None`)、ファイルごとの診断)。組込みなら `None`。
#[allow(clippy::type_complexity)]
fn load_user_theme(
    src: &eve_core::ConfigSources,
    config: &eve_config::Config,
) -> Option<(
    String,
    Option<eve_config::theme::Theme>,
    Vec<(String, Vec<eve_config::Diagnostic>)>,
)> {
    let name =
        eve_config::theme::with_background(&config.theme_name, config.theme_background).to_owned();
    if eve_config::theme::builtin(&name).is_some() {
        return None;
    }
    let read = |n: &str| match &src.themes {
        Some(dir) => read_source(dir.join(format!("{n}.toml")).as_path()),
        None => Ok(None),
    };
    let (theme, reports) = eve_config::theme_file::load_theme(&name, &read);
    Some((name, theme, reports))
}

/// 読込元から設定を合成する。読めない・構文エラーのファイルは重ねずに診断にする (起動を妨げない、FR-703)。
fn load_config(src: &eve_core::ConfigSources) -> eve_config::Resolved {
    let sources: Vec<eve_config::Source> = src
        .files
        .iter()
        .map(|(name, path)| eve_config::Source {
            name: name.clone(),
            text: read_source(path.as_path()),
        })
        .collect();
    eve_config::resolve(&sources, &src.sets)
}

/// `keymap.toml` を読む。無ければ空。読めないときは診断を 1 件出して空。
fn load_keymap(src: &eve_core::ConfigSources) -> eve_config::KeymapFile {
    use eve_config::{DiagKind, Diagnostic, KeymapFile, Severity};
    let Some(path) = &src.keymap else {
        return KeymapFile::default();
    };
    match read_source(path.as_path()) {
        Ok(Some(text)) => {
            eve_config::load_keymap_str(text.strip_prefix('\u{feff}').unwrap_or(&text))
        }
        Ok(None) => KeymapFile::default(),
        Err(message) => KeymapFile {
            entries: Vec::new(),
            diagnostics: vec![Diagnostic {
                severity: Severity::Error,
                kind: DiagKind::Syntax { message },
                key: String::new(),
                value: String::new(),
                line: 0,
                column: 0,
            }],
        },
    }
}

/// UI の言語: --locale > ui.locale > EVE_LANG > OS の言語 > ja (07 §17.1)。
fn choose_locale(
    cli_locale: Option<&str>,
    config: &eve_config::Config,
) -> eve_config::LocaleChoice {
    let env_lang = std::env::var("EVE_LANG").ok();
    let os_tag = eve_platform::locale::ui_language_tag();
    eve_config::select_locale(
        cli_locale,
        config.ui_locale,
        env_lang.as_deref(),
        os_tag.as_deref(),
    )
}

/// `--check-config` (07 §18.3): 設定を読んで診断を標準出力へ出し、終える。エラーがあれば終了コード 3、警告だけなら 0。
/// キーマップは書式だけを確かめる (コマンド名などの照合は起動したときの診断で行う)。
fn check_config(args: &cli::ConfigArgs, cli_locale: Option<&str>) -> u8 {
    let paths = match WinPaths::from_process_env() {
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

/// 同梱の第三者告知 (`cargo about` が生成する。05 §17.1)。`--licenses` で表示する。
const THIRD_PARTY_NOTICES: &str = include_str!("../../../THIRD-PARTY-NOTICES.md");

/// `--crash-report` (09 §10.2): `state_dir/log/` の最新のクラッシュログの場所と内容を表示し、Issue の場所を案内する。
/// 送信はしない (貼り付けるかどうかは利用者が決める)。
fn crash_report() -> u8 {
    let paths = match WinPaths::from_process_env() {
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
fn dump_config(args: &cli::ConfigArgs, defaults: bool, json: bool) -> u8 {
    let config = if defaults {
        eve_config::Config::default()
    } else {
        let paths = match WinPaths::from_process_env() {
            Ok(p) => p,
            Err(_) => return EXIT_ERROR,
        };
        load_config(&config_sources(&paths, args)).config
    };
    print!("{}", eve_config::dump(&config, json));
    0
}

/// 診断の重さ → ログのレベル。
fn log_level(s: eve_config::Severity) -> eve_platform::log::Level {
    match s {
        eve_config::Severity::Info => eve_platform::log::Level::Info,
        eve_config::Severity::Warning => eve_platform::log::Level::Warn,
        eve_config::Severity::Error => eve_platform::log::Level::Error,
    }
}

/// 診断 1 件のログの行 (ログは英語のみ、05 §7.3)。
fn describe(name: &str, d: &eve_config::Diagnostic) -> String {
    format!(
        "config {name}:{}:{} {} {:?}",
        d.line, d.column, d.key, d.kind
    )
}

/// 設定 `theme.color_mode` を TUI の型へ。
fn color_mode(mode: eve_config::ColorMode) -> eve_tui::caps::ColorMode {
    use eve_config::ColorMode as C;
    use eve_tui::caps::ColorMode as T;
    match mode {
        C::Auto => T::Auto,
        C::TrueColor => T::TrueColor,
        C::Color256 => T::Ansi256,
        C::Color16 => T::Ansi16,
        C::None => T::None,
    }
}

fn run(
    files: Vec<eve_core::InitialFile>,
    read_only: bool,
    encoding: Option<Encoding>,
    cli_locale: Option<&str>,
    ignored: &[String],
    config_args: &cli::ConfigArgs,
    cli_log_level: Option<eve_platform::log::Level>,
) -> u8 {
    // 作業ディレクトリ・ディレクトリ・Ports
    let paths = match WinPaths::from_process_env() {
        Ok(p) => Arc::new(p),
        Err(e) => {
            texts::Bi::new(
                format!("Cannot get the working directory: {e}"),
                format!("作業ディレクトリを取得できません: {e}"),
            )
            .write_to(&mut std::io::stderr().lock());
            return EXIT_ERROR;
        }
    };
    let sources = config_sources(&paths, config_args);
    let loaded = load_config(&sources);
    let keymap = load_keymap(&sources);
    let clock = Arc::new(SystemClock::new());
    let state_dir = paths.state_dir();
    // UI の言語。検出は端末の初期化の前に 1 回だけ
    let choice = choose_locale(cli_locale, &loaded.config);
    let locale = choice.locale;
    // 設定の診断は全件をログへ書く (画面には最初の 1 件か要約、07 §4.3)。ファイルは最初の書込で作る
    // ログの水準: --log-level > EVE_LOG > warn (07 §18.3)
    let logger = match cli_log_level {
        Some(l) => eve_platform::log::Logger::with_level(Some(state_dir.as_path()), l),
        None => eve_platform::log::Logger::new(Some(state_dir.as_path())),
    };
    for (name, diags) in &loaded.reports {
        for d in diags {
            logger.log(log_level(d.severity), &describe(name, d));
        }
    }
    for w in &choice.warnings {
        let from = match w.source {
            eve_config::LocaleSource::Cli => "--locale",
            _ => "EVE_LANG",
        };
        logger.log(
            eve_platform::log::Level::Warn,
            &format!(
                "locale {from} {:?} is not a bundled language; trying the next source",
                w.value
            ),
        );
    }
    // 置換中の計数 (保存の置換・直接上書きの間だけ上がる)。終了の経路が下りるのを待つ (02 §9.6 手順 3)
    let replacing = eve_core::ReplacingCount::default();
    let fs = Arc::new(WinFs::new(state_dir.as_path()).with_replacing(replacing.clone()));
    let (tx, rx) = channel();
    let job_ports = JobPorts {
        fs: fs.clone(),
        clock: clock.clone(),
        paths: paths.clone(),
    };
    let spawner = match ThreadJobSpawner::new(job_ports, tx.clone()) {
        Ok(s) => s,
        Err(e) => {
            texts::Bi::new(
                format!("Cannot start the worker thread: {e}"),
                format!("ワーカーを起動できません: {e}"),
            )
            .write_to(&mut std::io::stderr().lock());
            return EXIT_ERROR;
        }
    };
    let ports = Ports {
        fs,
        clock,
        jobs: Rc::new(spawner),
        paths,
    };
    let mut host = Host::with_channel(ports, (tx.clone(), rx));
    host.set_initial_encoding(encoding);
    let config = loaded.config;
    host.set_config(config.clone(), CONFIG_FILE, Vec::new());
    for (name, diags) in loaded.reports {
        host.add_config_report(&name, diags);
    }
    // テーマファイル (組込みでない `theme.name`)。読めなければ default で動き、診断を出す
    if let Some((name, theme, reports)) = load_user_theme(&sources, &config) {
        for (file, diags) in &reports {
            for d in diags {
                logger.log(log_level(d.severity), &describe(file, d));
            }
        }
        for (file, diags) in reports {
            host.add_config_report(&file, diags);
        }
        host.set_user_theme(&name, theme);
    }
    host.set_config_sources(sources);
    for d in &keymap.diagnostics {
        logger.log(log_level(d.severity), &describe(KEYMAP_FILE, d));
    }
    host.set_keymap(KEYMAP_FILE, &keymap);
    host.set_locale(locale);
    // 要約の対処 (hint) は省略されないので、長い絶対パスではなく末尾の `log\evoeditor.log` だけを出す
    host.set_log_path(logger.path().and_then(|p| {
        let name = p.file_name()?.to_string_lossy().into_owned();
        let dir = p.parent()?.file_name()?.to_string_lossy().into_owned();
        Some(format!("{dir}\\{name}"))
    }));
    host.set_system_encoding(
        eve_platform::locale::system_encoding().and_then(Encoding::from_label),
    );
    // システムクリップボード (所有者ウィンドウのスレッドは最初の読み書きで遅延して起こす、05 §8.1)
    host.set_clipboard(Arc::new(eve_platform::clipboard::WinClipboard::new()));

    // 緊急退避の書き手 (公開済みの DocSnapshot だけを読み、Port を通さずに書く、02 §11.2.1)
    host.set_rescue_writer(Arc::new(WinRescue::new(Some(
        state_dir.as_path().to_path_buf(),
    ))));
    let emergency = host.emergency();
    let restore_info = host.restore_info();
    let run_finished = host.run_finished();

    // コンソールのコントロールイベント (Ctrl+Break・ウィンドウの閉鎖など)。ハンドラのスレッドは、メインを待たずに直ちに
    // 退避を書き (CTRL_CLOSE は最大 3.5 s、それ以外は最大 2.5 s)、その後にメインへ終了を知らせる。退避はプロセスで 1 回だけ
    {
        let tx = tx.clone();
        let emergency = emergency.clone();
        let replacing = replacing.clone();
        let _ = eve_platform::console::install_handler(move |ev| {
            let t0 = std::time::Instant::now();
            use eve_host::{RESCUE_BUDGET_CLOSE, RESCUE_BUDGET_SIGNAL};
            use eve_platform::console::CtrlEvent;
            let (kind, reason, budget) = match ev {
                CtrlEvent::Close => (
                    SignalKind::ConsoleClose,
                    RescueReason::ConsoleClose,
                    RESCUE_BUDGET_CLOSE,
                ),
                CtrlEvent::CtrlC | CtrlEvent::CtrlBreak => (
                    SignalKind::Interrupt,
                    RescueReason::Signal,
                    RESCUE_BUDGET_SIGNAL,
                ),
                _ => (
                    SignalKind::Terminate,
                    RescueReason::Signal,
                    RESCUE_BUDGET_SIGNAL,
                ),
            };
            emergency.run(reason, budget);
            let sent = tx.send(HostMsg::Signal(kind)).is_ok();
            // CTRL_CLOSE は送るだけで戻る (ハンドラから戻るとプロセスが終わる。端末はコンソールごと閉じられる、02 §9.6 手順 2)
            if !sent {
                return false;
            }
            if matches!(ev, CtrlEvent::Close) {
                // 置換中フラグが下りるのを、受信から 4.3 s まで待ってから戻る (残りの 0.2 s は復帰の余裕、02 §11.2.1)
                wait_zero(
                    &replacing,
                    Some(std::time::Duration::from_millis(4300).saturating_sub(t0.elapsed())),
                );
                return true;
            }
            // CTRL_C / CTRL_BREAK: 戻らずに最大 2 秒、メインの終了を待つ (戻るとプロセスが続行する)。
            // 終わらなければメインが止まっている: 受信スレッドが端末を復元して終了する (02 §9.6 手順 4)
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while std::time::Instant::now() < deadline {
                if run_finished.load(std::sync::atomic::Ordering::Acquire) {
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            // 置換中フラグが下りるのを、受信から 4.8 s まで (復元の 200 ms を残して) 待つ
            wait_zero(
                &replacing,
                Some(std::time::Duration::from_millis(4800).saturating_sub(t0.elapsed())),
            );
            if let Some(info) = restore_info.get() {
                eve_platform::term::emergency_restore(
                    info.handle.0,
                    info.bytes,
                    &info.saved_mode.0,
                    &info.restored.0,
                );
            }
            // 退避の結果はベストエフォートで 1 回出す (端末の復元の後)
            if let Some(o) = emergency.outcome() {
                let mut err = std::io::stderr().lock();
                for l in texts::recovery_lines(&o) {
                    l.write_to(&mut err);
                }
            }
            std::process::exit(130)
        });
    }

    // ログオフ・シャットダウン (v0.2、ベストエフォート。05 §11.2 の「セッション終了の受け方」): 専用スレッドの非表示の
    // トップレベルウィンドウで受ける。問い合わせ (`WM_QUERYENDSESSION`) で退避を (使い捨てのスレッドで) 始め、終了の通知
    // (`WM_ENDSESSION`) では退避の完了と置換中フラグを待ってから、メインへ終了を知らせて戻る。取り消しでは何もしない
    {
        let tx = tx.clone();
        let emergency = emergency.clone();
        let replacing = replacing.clone();
        let _ = eve_platform::session::start(&texts::session_block_reason(), move |ev| {
            use eve_host::RESCUE_BUDGET_SESSION;
            use eve_platform::session::SessionEvent;
            match ev {
                // 使い捨てのスレッドで呼ばれるので、ここで書いてよい
                SessionEvent::QueryEnd => {
                    emergency.run(RescueReason::SessionEnd, RESCUE_BUDGET_SESSION);
                }
                SessionEvent::End { ending: true } => {
                    let t0 = std::time::Instant::now();
                    // 問い合わせで始めた退避の完了を待つ (届いていなければここで書く)。退避は 1 回だけ
                    emergency.run(RescueReason::SessionEnd, RESCUE_BUDGET_SESSION);
                    wait_zero(
                        &replacing,
                        Some(std::time::Duration::from_millis(4300).saturating_sub(t0.elapsed())),
                    );
                    let _ = tx.send(HostMsg::Signal(SignalKind::Terminate));
                }
                _ => {}
            }
        });
    }

    // パニックの表示は端末の復元の後にする (代替スクリーン内に出すと消えるため)。フックは端末に触れない (04 §14.2)
    std::panic::set_hook(Box::new(|info| {
        if let Ok(mut m) = PANIC_INFO.lock() {
            let bt = std::backtrace::Backtrace::force_capture();
            *m = Some(format!("{info}\n\n{bt}"));
        }
    }));
    let mut frontend = TuiFrontend::with_color_mode(color_mode(config.color_mode));
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        host.run(&mut frontend, &files, read_only)
    }));
    // どの経路でも端末を復元する (冪等)
    frontend.restore_terminal();
    // 保存の置換・直接上書きの最中なら、終わるまで待つ (途中で終わると元のファイルが壊れる、02 §9.6 手順 3)。端末は復元済みで
    // 画面に何も出ないので、待つ前に stderr で知らせる
    wait_replacing(&replacing, None);
    // コンソールのハンドラが退避を書いている最中なら、終わるまで待つ (先にメインが終わると書込が打ち切られる)
    emergency.wait_if_writing(eve_host::RESCUE_BUDGET_CLOSE);
    match outcome {
        Ok(code) => {
            if !ignored.is_empty() {
                texts::ignored_args(ignored).write_to(&mut std::io::stderr().lock());
            }
            print_exit_lines(&mut host, locale);
            // シグナル・致命的エラーで終わったときの退避の結果 (退避したときだけ。通常終了では書いていない)
            if let Some(o) = emergency.outcome() {
                let mut err = std::io::stderr().lock();
                for l in texts::recovery_lines(&o) {
                    l.write_to(&mut err);
                }
            }
            code.0
        }
        Err(_) => {
            // パニック: 退避 (最大 5 s。メインの catch_unwind の後) → 端末の復元は済み → panic.header → 退避の結果 → クラッシュログ → 報告先
            let rescued = emergency.run(RescueReason::Panic, eve_host::RESCUE_BUDGET_FATAL);
            // 退避の後、置換中フラグを最大 1 s 待つ (合計 5 s の内、02 §11.2.1)
            wait_zero(&replacing, Some(std::time::Duration::from_secs(1)));
            let mut err = std::io::stderr().lock();
            texts::panic_header().write_to(&mut err);
            if let Some(o) = rescued {
                for l in texts::recovery_lines(&o) {
                    l.write_to(&mut err);
                }
            }
            if let Some(path) = write_crash_log(state_dir.as_path()) {
                texts::panic_crash_log(&path).write_to(&mut err);
            }
            texts::panic_report(&format!("{}/issues", env!("CARGO_PKG_REPOSITORY")))
                .write_to(&mut err);
            EXIT_PANIC
        }
    }
}

/// 置換中フラグが下りるまで待つ。`limit` が `None` なら無期限。下りたら `true`。
fn wait_zero(replacing: &eve_core::ReplacingCount, limit: Option<std::time::Duration>) -> bool {
    let start = std::time::Instant::now();
    loop {
        if replacing.is_zero() {
            return true;
        }
        if limit.is_some_and(|l| start.elapsed() >= l) {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// 置換中フラグが下りるまで待つ。上がっていたら、待ち始める前に stderr へ 1 行ずつ知らせる。
fn wait_replacing(replacing: &eve_core::ReplacingCount, limit: Option<std::time::Duration>) {
    if replacing.is_zero() {
        return;
    }
    {
        let mut err = std::io::stderr().lock();
        for p in replacing.paths() {
            texts::waiting_save(&p).write_to(&mut err);
        }
    }
    wait_zero(replacing, limit);
}

/// クラッシュログを `state_dir/log/crash-<pid>-<秒>.txt` に書く (09 §10)。パニックのメッセージ・場所・バックトレースだけで、
/// 文書の内容は含まない。書けなければ `None`。
fn write_crash_log(state_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let info = PANIC_INFO.lock().ok()?.clone()?;
    let dir = state_dir.join("log");
    std::fs::create_dir_all(&dir).ok()?;
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let path = dir.join(format!("crash-{}-{secs}.txt", std::process::id()));
    let body = format!(
        "evoeditor {} のクラッシュログ (文書の内容は含みません)\n\n{info}\n",
        env!("CARGO_PKG_VERSION")
    );
    std::fs::write(&path, body).ok()?;
    Some(path)
}

fn main() -> ProcessExit {
    // 他のどの処理より先に、DLL の検索を System32 だけにする (NFR-502 AC-2 (a))
    #[cfg(windows)]
    let _ = eve_platform::dll::restrict_search_to_system32();
    eve_tui::trace::init();
    // `FILE:LINE` の曖昧性の解消のための存在確認 (07 §18.2)。端末を始める前に 1 回だけ行う
    let code = match cli::parse(std::env::args_os().skip(1), &|p| {
        std::path::Path::new(p).exists()
    }) {
        Ok(Cli::Help) => {
            print!("{}", texts::help());
            0
        }
        Ok(Cli::Version) => {
            println!("evoeditor {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Ok(Cli::Run {
            mode: cli::Mode::Licenses,
            ..
        }) => {
            print!("{THIRD_PARTY_NOTICES}");
            0
        }
        Ok(Cli::Run {
            mode: cli::Mode::CrashReport,
            ..
        }) => crash_report(),
        Ok(Cli::Run {
            log_level: Some(l), ..
        }) if eve_platform::log::Level::parse(&l).is_none() => {
            texts::usage_error(&UsageError::BadLogLevel(l)).write_to(&mut std::io::stderr().lock());
            EXIT_USAGE
        }
        Ok(Cli::Run {
            mode: cli::Mode::CheckConfig,
            config,
            locale,
            ..
        }) => check_config(&config, locale.as_deref()),
        Ok(Cli::Run {
            mode: cli::Mode::DumpConfig { defaults, json },
            config,
            ..
        }) => dump_config(&config, defaults, json),
        Ok(Cli::Run {
            files,
            read_only,
            encoding,
            locale,
            ignored,
            config,
            log_level,
            ..
        }) => match encoding.as_deref().map(Encoding::from_label) {
            // 07 §18.4: 文字コードの名前が分からないのは使い方の誤り
            Some(None) => {
                texts::usage_error(&UsageError::BadEncoding(encoding.unwrap_or_default()))
                    .write_to(&mut std::io::stderr().lock());
                EXIT_USAGE
            }
            forced => run(
                files
                    .into_iter()
                    .map(|f| eve_core::InitialFile {
                        path: f.path,
                        line: f.line,
                        col: f.col,
                    })
                    .collect(),
                read_only,
                forced.flatten(),
                locale.as_deref(),
                &ignored,
                &config,
                log_level
                    .as_deref()
                    .and_then(eve_platform::log::Level::parse),
            ),
        },
        Err(e) => {
            texts::usage_error(&e).write_to(&mut std::io::stderr().lock());
            EXIT_USAGE
        }
    };
    eve_tui::trace::flush();
    ProcessExit::from(code)
}
