//! evoeditor の実行ファイル (docs/02 §4.1、§9.5)。引数の解析と、本番の部品での Host の組み立て・起動だけを行う。
//! 編集のロジックは core、端末は eve-tui、OS は eve-platform にある。
//!
//! 起動・描画・入力・終了のほか、保存・文字コード (`--encoding`)・設定の読込もここで Host に渡す。責務ごとのモジュール:
//! `cli` (引数の解析)、`settings` (設定の読込)、`modes` (端末を始めない起動の形)、`startup` (Host の組み立て)、
//! `os_events` (OS からの終了の要求)、`panic` (パニックの記録)、`exit` (終了コードと終了時の行)、`texts` (端末の外の文言)。

#![forbid(unsafe_code)]
// 実行ファイルは、端末を初期化する前後に標準出力・標準エラーへ書く (ヘルプ・版・使い方の誤り・終了時の行)。
#![allow(clippy::print_stdout, clippy::print_stderr)]

mod cli;
mod exit;
mod modes;
mod os_events;
mod panic;
mod settings;
mod startup;
mod texts;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::process::ExitCode as ProcessExit;

use eve_core::Encoding;
#[cfg(unix)]
use eve_platform::dirs::UnixPaths as NativePaths;
#[cfg(windows)]
use eve_platform::dirs::WinPaths as NativePaths;
#[cfg(windows)]
use eve_platform::fs::WinFs as NativeFs;
#[cfg(unix)]
use eve_platform::unix_fs::UnixFs as NativeFs;
use eve_tui::frontend::TuiFrontend;

use cli::{Cli, UsageError};
use exit::EXIT_USAGE;

/// 端末を始めて編集する起動の形: Host を組み立て、OS のコールバックとパニックのフックを入れて `Host::run` を回し、
/// どの経路でも端末を復元してから終わり方 (終了時の行・退避・クラッシュログ) を出す。
fn run(
    files: Vec<eve_core::InitialFile>,
    read_only: bool,
    encoding: Option<Encoding>,
    cli_locale: Option<&str>,
    ignored: &[String],
    config_args: &cli::ConfigArgs,
    cli_log_level: Option<eve_platform::log::Level>,
) -> u8 {
    let startup::Started {
        mut host,
        shutdown,
        replacing,
        state_dir,
        locale,
        color_mode,
    } = match startup::start(encoding, cli_locale, config_args, cli_log_level) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let emergency = host.emergency();
    os_events::register(&shutdown);
    // パニックの表示は端末の復元の後にする (代替スクリーン内に出すと消えるため)。フックは端末に触れない (04 §14.2)
    panic::install_hook();
    // Unix: バックグラウンドのジョブで起動されたら、端末に触れずに終わる (05 §11.3。終了コードは端末の初期化失敗と同じ 5)
    #[cfg(unix)]
    if eve_tui::unix::is_background_job() {
        if let Some(l) = texts::stderr_line("stderr.background_job", &[]) {
            l.write_to(&mut std::io::stderr().lock());
        }
        return exit::EXIT_TERMINAL;
    }
    let mut frontend = TuiFrontend::with_color_mode(settings::color_mode(color_mode));
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        host.run(&mut frontend, &files, read_only)
    }));
    // どの経路でも端末を復元する (冪等)
    frontend.restore_terminal();
    // 保存の置換・直接上書きの最中なら、終わるまで待つ (途中で終わると元のファイルが壊れる、02 §9.6 手順 3)。端末は復元済みで
    // 画面に何も出ないので、待つ前に stderr で知らせる
    exit::wait_replacing(&shutdown, &replacing);
    // コンソールのハンドラが退避を書いている最中なら、終わるまで待つ (先にメインが終わると書込が打ち切られる)
    emergency.wait_if_writing(eve_host::RESCUE_BUDGET_CLOSE);
    match outcome {
        Ok(code) => exit::finish(&mut host, locale, ignored, &emergency, code),
        Err(_) => exit::finish_panic(&emergency, &shutdown, &state_dir),
    }
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
            print!("{}", modes::THIRD_PARTY_NOTICES);
            0
        }
        Ok(Cli::Run {
            mode: cli::Mode::CrashReport,
            ..
        }) => modes::crash_report(),
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
        }) => modes::check_config(&config, locale.as_deref()),
        Ok(Cli::Run {
            mode: cli::Mode::DumpConfig { defaults, json },
            config,
            ..
        }) => modes::dump_config(&config, defaults, json),
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
    // Unix: シグナルで終わったなら、後始末 (退避・端末の復元・終了時の行) の後に同じシグナルで終わる (05 §11.5)
    os_events::die_if_signaled();
    ProcessExit::from(code)
}
