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
    let state_dir = paths.state_dir();
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
                    let _ = writeln!(err, "{l}");
                }
            }
            std::process::exit(130)
        });
    }

    // パニックの表示は端末の復元の後にする (代替スクリーン内に出すと消えるため)。フックは端末に触れない (04 §14.2)
    std::panic::set_hook(Box::new(|info| {
        if let Ok(mut m) = PANIC_INFO.lock() {
            let bt = std::backtrace::Backtrace::force_capture();
            *m = Some(format!("{info}\n\n{bt}"));
        }
    }));
    let mut frontend = TuiFrontend::new();
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        host.run(&mut frontend, file.as_deref(), read_only)
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
                eprintln!(
                    "このバージョンでは未対応の引数を無視しました: {}",
                    ignored.join(" ")
                );
            }
            print_exit_lines(&mut host);
            // シグナル・致命的エラーで終わったときの退避の結果 (退避したときだけ。通常終了では書いていない)
            if let Some(o) = emergency.outcome() {
                let mut err = std::io::stderr().lock();
                for l in texts::recovery_lines(&o) {
                    let _ = writeln!(err, "{l}");
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
            let _ = writeln!(err, "{}", texts::PANIC_HEADER);
            if let Some(o) = rescued {
                for l in texts::recovery_lines(&o) {
                    let _ = writeln!(err, "{l}");
                }
            }
            if let Some(path) = write_crash_log(state_dir.as_path()) {
                let _ = writeln!(err, "{}", texts::panic_crash_log(&path));
            }
            let _ = writeln!(
                err,
                "{}",
                texts::panic_report(&format!("{}/issues", env!("CARGO_PKG_REPOSITORY")))
            );
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
            let _ = writeln!(err, "{}", texts::waiting_save(&p));
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
