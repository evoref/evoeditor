//! evoeditor の実行ファイル (docs/02 §4.1、§9.5)。引数の解析と、本番の部品での Host の組み立て・起動だけを行う。
//! 編集のロジックは core、端末は eve-tui、OS は eve-platform にある。
//!
//! 起動・描画・入力・終了のほか、保存・文字コード (`--encoding`)・設定の読込もここで Host に渡す。責務ごとのモジュール:
//! `cli` (引数の解析)、`settings` (設定の読込)、`modes` (端末を始めない起動の形)、`listing` (隠しオプションのコマンドとキーマップの一覧)、`startup` (Host の組み立て)、
//! `os_events` (OS からの終了の要求)、`panic` (パニックの記録)、`exit` (終了コードと終了時の行)、`texts` (端末の外の文言)。

#![forbid(unsafe_code)]
// 実行ファイルは、端末を初期化する前後に標準出力・標準エラーへ書く (ヘルプ・版・使い方の誤り・終了時の行)。
#![allow(clippy::print_stdout, clippy::print_stderr)]

mod cli;
mod exit;
mod listing;
mod modes;
mod os_events;
mod panic;
mod settings;
mod startup;
mod texts;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::process::ExitCode as ProcessExit;
use std::sync::Arc;
use std::time::Duration;

use eve_core::Encoding;
#[cfg(unix)]
use eve_platform::dirs::UnixPaths as NativePaths;
#[cfg(windows)]
use eve_platform::dirs::WinPaths as NativePaths;
#[cfg(windows)]
use eve_platform::fs::WinFs as NativeFs;
use eve_platform::stdin::PlatformStdin;
#[cfg(unix)]
use eve_platform::unix_fs::UnixFs as NativeFs;
use eve_tui::frontend::TuiFrontend;

use cli::{Cli, UsageError};
use exit::EXIT_USAGE;

/// 標準入力の最初のデータを黙って待つ時間。過ぎても届かなければ、待っていることを stderr に出す (`cat file | evoeditor -`
/// のようにすぐ届く入力で、終了後の画面に待ちの行を残さない)。
const STDIN_NOTICE_AFTER: Duration = Duration::from_millis(100);

/// 標準入力 (`-`) を私有のハンドル / fd へ移して、読取のスレッド `eve-stdin` を始める (03 §9.2.2 の「起動の経路」、
/// 05 §9.1 / §9.2)。作れなかったとき (移し替えの後にスレッドを作れなかったときを含む) は `None` で、読込は「最初から
/// 読めない」(`msg.stdin.unreadable`) で終わる。Host の組み立ての失敗し得る段の後に呼ぶ (`startup::start`)。
fn take_stdin() -> Option<Arc<PlatformStdin>> {
    PlatformStdin::from_process_stdin().ok().map(Arc::new)
}

/// 端末を使えるか (`TERM=dumb` でない、入力・描画の端末を開ける) を、標準入力を移す前に確かめる。使えなければ端末の
/// 初期化の失敗と同じ行を出して終了コード 5 を返す (標準入力は読まない。03 §9.2.2 の「端末の初期化の時期」)。
fn probe_terminal() -> Result<(), u8> {
    let Err(e) = eve_tui::frontend::probe_terminal() else {
        return Ok(());
    };
    let mut args = vec![(
        "reason_key".to_owned(),
        format!("frontend.{}", e.kind.key()),
    )];
    args.extend(e.detail.map(|d| ("detail".to_owned(), d)));
    if let Some(l) = texts::stderr_line("stderr.not_a_terminal", &args) {
        l.write_to(&mut std::io::stderr().lock());
    }
    Err(exit::EXIT_TERMINAL)
}

/// 端末を初期化する前に、標準入力の最初のデータか終わりが届くまで待つ (OQ-219 の 9。書き手が同じ端末でパスワードを
/// 尋ねるときに、書き手の入力を先に終わらせる)。すぐに届かなければ stderr に 1 組の行を出す。待つ間の Ctrl+C は OS の
/// 既定の動作で終わる (呼び側は終了の要求の受信 (`os_events::register`) をこの後に入れる)。
fn wait_stdin(stdin: &PlatformStdin) {
    if !stdin.wait_ready(Some(STDIN_NOTICE_AFTER)) {
        texts::stdin_waiting().write_to(&mut std::io::stderr().lock());
        stdin.wait_ready(None);
    }
}

/// 端末を始めて編集する起動の形: Host を組み立て、OS のコールバックとパニックのフックを入れて `Host::run` を回し、
/// どの経路でも端末を復元してから終わり方 (終了時の行・退避・クラッシュログ) を出す。
fn run(
    files: Vec<cli::FileArg>,
    read_only: bool,
    encoding: Option<Encoding>,
    cli_locale: Option<&str>,
    ignored: &[String],
    config_args: &cli::ConfigArgs,
    cli_log_level: Option<eve_platform::log::Level>,
) -> u8 {
    // Unix: バックグラウンドのジョブで起動されたら、端末に触れずに終わる (05 §11.3。終了コードは端末の初期化失敗と同じ 5)。
    // 標準入力を移して読み始める前に確かめる (読み直せない標準入力を読み捨てない)
    #[cfg(unix)]
    if eve_tui::unix::is_background_job() {
        if let Some(l) = texts::stderr_line("stderr.background_job", &[]) {
            l.write_to(&mut std::io::stderr().lock());
        }
        return exit::EXIT_TERMINAL;
    }
    // 標準入力 (`-`): 端末かどうかを判定する。端末でなければ、移して読み始める前に端末を使えるかを確かめる (使えないまま
    // 読み始めると、端末の初期化の失敗で読んだ分を失う。`ssh host 'cmd | evoeditor -'` を `-t` なしで実行したときなど)
    let wants_stdin = files.iter().any(|f| f.stdin);
    let stdin_terminal = wants_stdin && eve_platform::stdin::stdin_is_terminal();
    let read_stdin = wants_stdin && !stdin_terminal;
    if read_stdin && let Err(code) = probe_terminal() {
        return code;
    }
    let files: Vec<eve_core::InitialFile> = files
        .into_iter()
        .map(|f| eve_core::InitialFile {
            target: if f.stdin {
                eve_core::InitialTarget::Stdin {
                    terminal: stdin_terminal,
                }
            } else {
                eve_core::InitialTarget::Path(f.path)
            },
            line: f.line,
            col: f.col,
        })
        .collect();
    let startup::Started {
        mut host,
        shutdown,
        replacing,
        state_dir,
        locale,
        color_mode,
        stdin,
    } = match startup::start(
        encoding,
        cli_locale,
        config_args,
        cli_log_level,
        // 標準入力は、Host の組み立ての失敗し得る段 (作業ディレクトリ・ワーカー) の後に移す
        || if read_stdin { take_stdin() } else { None },
    ) {
        Ok(s) => s,
        Err(code) => return code,
    };
    // 標準入力の最初のデータを待ってから端末を始める。待った後はメインの参照を手放す (`Arc` を最後に手放すのを
    // 読込のスレッド `eve-stdin-job` にする。02 §9.1)
    if let Some(s) = stdin {
        wait_stdin(&s);
    }
    let emergency = host.emergency();
    os_events::register(&shutdown);
    // パニックの表示は端末の復元の後にする (代替スクリーン内に出すと消えるため)。フックは端末に触れない (04 §14.2)
    panic::install_hook();
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
            mode: cli::Mode::ListCommands { json },
            config,
            locale,
            ..
        }) => listing::list_commands(&config, locale.as_deref(), json),
        Ok(Cli::Run {
            mode: cli::Mode::DumpKeymap { json },
            config,
            locale,
            ..
        }) => listing::dump_keymap(&config, locale.as_deref(), json),
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
                files,
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
