//! evoeditor の実行ファイル (docs/02 §4.1、§9.5)。引数の解析と、本番の部品での Host の組み立て・起動だけを行う。
//! 編集のロジックは core、端末は eve-tui、OS は eve-platform にある。
//!
//! 現在の範囲 (M2 の E4): 起動・描画・入力・終了。保存・文字コード (`--encoding`)・設定の読込は後の版 (M3 / M4)。

#![forbid(unsafe_code)]
// 実行ファイルは、端末を初期化する前後に標準出力・標準エラーへ書く (ヘルプ・版・使い方の誤り・終了時の行)。
#![allow(clippy::print_stdout, clippy::print_stderr)]

mod cli;

use std::io::Write as _;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::process::ExitCode as ProcessExit;
use std::rc::Rc;
use std::sync::Arc;

use eve_config::Catalog;
use eve_core::{Encoding, JobPorts, Ports};
use eve_host::{Host, HostMsg, SignalKind, SystemClock, ThreadJobSpawner, channel};
use eve_platform::dirs::WinPaths;
use eve_platform::fs::WinFs;
use eve_tui::frontend::TuiFrontend;

use cli::{Cli, UsageError};

/// パニックの内容 (フックが書き、復元の後に出す)。
static PANIC_INFO: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

const HELP: &str = "\
使い方: evoeditor [オプション] [ファイル]

  ファイルを指定しないと、無題の文書で始めます。
  存在しないファイルは、保存すると作成されます。

オプション:
  -R, --readonly       読み取り専用で開く
  -e, --encoding <名前>  文字コードを指定して開く (utf-8 / utf-16le / utf-16be / shift_jis / euc-jp / iso-2022-jp)
  -V, --version        版を表示して終了する
  -h, --help           このヘルプを表示して終了する
  --                   以降を位置引数として扱う
";

/// 実行時エラーでの終了 (07 §18.4)。
const EXIT_ERROR: u8 = 1;
/// 使い方の誤り。
const EXIT_USAGE: u8 = 2;
/// 端末の初期化に失敗 (`Host::run` が返す。ここでは使わない)。
#[allow(dead_code)]
const EXIT_TERMINAL: u8 = 5;
/// パニック。
const EXIT_PANIC: u8 = 70;

fn usage_message(e: &UsageError) -> String {
    match e {
        UsageError::UnknownOption(o) => format!("未知のオプションです: {o}"),
        UsageError::MissingValue(o) => format!("{o} には値が必要です"),
        UsageError::MultipleFiles => {
            "このバージョンでは複数のファイルを同時に開けません。ファイルは 1 つだけ指定してください".to_owned()
        }
        UsageError::BadEncoding(v) if v.is_empty() => "文字コードの名前を指定してください".to_owned(),
        UsageError::BadEncoding(v) => format!(
            "文字コード {v} は扱えません (UTF-8・UTF-16LE・UTF-16BE・Shift_JIS・EUC-JP・ISO-2022-JP から指定してください)"
        ),
    }
}

/// 端末の復元の後に出す行 (`Effect::Quit` の `stderr`)。文言は言語ファイルで解決し、キーが無ければキー名を出す。
fn print_exit_lines(host: &mut Host) {
    let lines = host.take_exit_stderr();
    if lines.is_empty() {
        return;
    }
    let cat = Catalog::builtin_ja();
    let mut err = std::io::stderr().lock();
    for l in lines {
        let args: Vec<(&str, &str)> = l
            .args
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let _ = writeln!(err, "{}", cat.render(&l.key, &args, &|_| None));
    }
}

fn run(
    file: Option<String>,
    read_only: bool,
    encoding: Option<Encoding>,
    ignored: &[String],
) -> u8 {
    // 作業ディレクトリ・ディレクトリ・Ports
    let paths = match WinPaths::from_process_env() {
        Ok(p) => Arc::new(p),
        Err(e) => {
            eprintln!("作業ディレクトリを取得できません: {e}");
            return EXIT_ERROR;
        }
    };
    let clock = Arc::new(SystemClock::new());
    let fs = Arc::new(WinFs);
    let (tx, rx) = channel();
    let job_ports = JobPorts {
        fs: fs.clone(),
        clock: clock.clone(),
        paths: paths.clone(),
    };
    let spawner = match ThreadJobSpawner::new(job_ports, tx.clone()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ワーカーを起動できません: {e}");
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

    // コンソールのコントロールイベント (Ctrl+Break・ウィンドウの閉鎖など)。緊急退避は M3
    {
        let tx = tx.clone();
        let _ = eve_platform::console::install_handler(move |ev| {
            use eve_platform::console::CtrlEvent;
            let kind = match ev {
                CtrlEvent::CtrlC | CtrlEvent::CtrlBreak => SignalKind::Interrupt,
                CtrlEvent::Close => SignalKind::ConsoleClose,
                _ => SignalKind::Terminate,
            };
            tx.send(HostMsg::Signal(kind)).is_ok()
        });
    }

    // パニックの表示は端末の復元の後にする (代替スクリーン内に出すと消えるため)。フックは端末に触れない (04 §14.2)
    std::panic::set_hook(Box::new(|info| {
        if let Ok(mut m) = PANIC_INFO.lock() {
            *m = Some(info.to_string());
        }
    }));
    let mut frontend = TuiFrontend::new();
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        host.run(&mut frontend, file.as_deref(), read_only)
    }));
    // どの経路でも端末を復元する (冪等)
    frontend.restore_terminal();
    match outcome {
        Ok(code) => {
            if !ignored.is_empty() {
                eprintln!(
                    "このバージョンでは未対応の引数を無視しました: {}",
                    ignored.join(" ")
                );
            }
            print_exit_lines(&mut host);
            code.0
        }
        Err(_) => {
            // 端末を復元した後に出す (代替スクリーン内の表示は消えるため)
            eprintln!("予期しないエラーのため evoeditor を終了しました。");
            if let Ok(m) = PANIC_INFO.lock()
                && let Some(info) = m.as_ref()
            {
                eprintln!("{info}");
            }
            EXIT_PANIC
        }
    }
}

fn main() -> ProcessExit {
    let code = match cli::parse(std::env::args_os().skip(1)) {
        Ok(Cli::Help) => {
            print!("{HELP}");
            0
        }
        Ok(Cli::Version) => {
            println!("evoeditor {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Ok(Cli::Run {
            file,
            read_only,
            encoding,
            ignored,
        }) => match encoding.as_deref().map(Encoding::from_label) {
            // 07 §18.4: 文字コードの名前が分からないのは使い方の誤り
            Some(None) => {
                eprintln!(
                    "evoeditor: {}",
                    usage_message(&UsageError::BadEncoding(encoding.unwrap_or_default()))
                );
                eprintln!("詳しくは evoeditor --help を見てください。");
                EXIT_USAGE
            }
            forced => run(file, read_only, forced.flatten(), &ignored),
        },
        Err(e) => {
            eprintln!("evoeditor: {}", usage_message(&e));
            eprintln!("詳しくは evoeditor --help を見てください。");
            EXIT_USAGE
        }
    };
    ProcessExit::from(code)
}
