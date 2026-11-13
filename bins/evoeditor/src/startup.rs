//! 本番の部品での Host の組み立て (02 §9.5): ディレクトリ・設定・ログ・Ports・クリップボード・緊急退避の書き手を
//! `HostInit` にまとめて `Host` を作り、非対話の終了の手順 (`NonInteractiveShutdown`) を用意する。端末はまだ始めない。

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use eve_core::{Encoding, JobPorts, KEYMAP_FILE, Paths as _, Ports};
use eve_host::{
    Host, HostInit, NonInteractiveShutdown, ShutdownParts, SystemClock, ThreadJobSpawner, channel,
};
use eve_platform::rescue::StateRescue;
use eve_platform::stdin::PlatformStdin;

use crate::exit::EXIT_ERROR;
use crate::settings::{
    choose_locale, config_sources, describe, load_config, load_keymap, load_user_theme, log_level,
};
use crate::{NativeFs, NativePaths, cli, texts};

/// 組み立てた Host と、起動の後に要るもの。
pub(crate) struct Started {
    pub host: Host,
    pub shutdown: NonInteractiveShutdown,
    /// 置換中の計数 (保存の置換・直接上書きの間だけ上がる)。終了の経路が下りるのを待つ (02 §9.6 手順 3)。
    pub replacing: eve_core::ReplacingCount,
    pub state_dir: PathBuf,
    pub locale: eve_config::Locale,
    pub color_mode: eve_config::ColorMode,
    /// 標準入力 (`evoeditor -` で端末でないとき)。bin は最初のデータを待った後に手放す (`Arc` を最後に手放すのを
    /// 読込のスレッド `eve-stdin-job` にする)。
    pub stdin: Option<Arc<PlatformStdin>>,
}

/// Host を組み立てる。作業ディレクトリ・ワーカーを用意できなければ、標準エラーに理由を出して終了コードを返す。
/// `take_stdin` は標準入力 (`evoeditor -`、端末でないとき) を移して返す。失敗し得る段の後に 1 回だけ呼び、ジョブの
/// ワーカーに渡す。
pub(crate) fn start(
    encoding: Option<Encoding>,
    cli_locale: Option<&str>,
    config_args: &cli::ConfigArgs,
    cli_log_level: Option<eve_platform::log::Level>,
    take_stdin: impl FnOnce() -> Option<Arc<PlatformStdin>>,
) -> Result<Started, u8> {
    // 作業ディレクトリ・ディレクトリ・Ports
    let paths = match NativePaths::from_process_env() {
        Ok(p) => Arc::new(p),
        Err(e) => {
            texts::Bi::new(
                format!("Cannot get the working directory: {e}"),
                format!("作業ディレクトリを取得できません: {e}"),
            )
            .write_to(&mut std::io::stderr().lock());
            return Err(EXIT_ERROR);
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
    let fs = Arc::new(NativeFs::new(state_dir.as_path()).with_replacing(replacing.clone()));
    let (tx, rx) = channel();
    let job_ports = JobPorts {
        stdin: None,
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
            return Err(EXIT_ERROR);
        }
    };
    // 標準入力 (`evoeditor -` で端末でないときだけ。`OpenStdin` のジョブだけが使う、03 §9.2.2)。この関数の失敗し得る段
    // (作業ディレクトリ・ワーカーの起動) の後に移す (失敗で読んだ分を捨てない)
    let stdin = take_stdin();
    spawner.set_stdin(stdin.clone().map(|s| s as Arc<dyn eve_core::StdinSource>));
    let ports = Ports {
        fs,
        clock,
        jobs: Rc::new(spawner),
        paths,
    };
    let config = loaded.config;
    let mut config_reports = loaded.reports;
    // テーマファイル (組込みでない `theme.name`)。読めなければ default で動き、診断を出す
    let user_theme = load_user_theme(&sources, &config).map(|(name, theme, reports)| {
        for (file, diags) in &reports {
            for d in diags {
                logger.log(log_level(d.severity), &describe(file, d));
            }
        }
        config_reports.extend(reports);
        (name, theme)
    });
    for d in &keymap.diagnostics {
        logger.log(log_level(d.severity), &describe(KEYMAP_FILE, d));
    }
    // 要約の対処 (hint) は省略されないので、長い絶対パスではなく末尾の `log\evoeditor.log` (Unix は `log/evoeditor.log`) だけを出す
    let log_path = logger.path().and_then(|p| {
        let name = p.file_name()?.to_string_lossy().into_owned();
        let dir = p.parent()?.file_name()?.to_string_lossy().into_owned();
        Some(format!("{dir}{}{name}", std::path::MAIN_SEPARATOR))
    });
    // システムクリップボード (所有者ウィンドウのスレッドは最初の読み書きで遅延して起こす、05 §8.1)
    #[cfg(windows)]
    let clipboard: Arc<dyn eve_host::ClipboardProvider> =
        Arc::new(eve_platform::clipboard::WinClipboard::new());
    // Unix は外部コマンド (wl-copy / xclip / xsel / WSL の clip.exe)。検出も最初の読み書きで行う (05 §8.2 / §8.3)
    #[cfg(unix)]
    let clipboard: Arc<dyn eve_host::ClipboardProvider> =
        Arc::new(eve_platform::clipboard::UnixClipboard::new());
    let color_mode = config.color_mode;
    let host_init = HostInit {
        initial_encoding: encoding,
        config: Some(config),
        config_reports,
        user_theme,
        config_sources: Some(sources),
        keymap: Some((KEYMAP_FILE.to_owned(), keymap)),
        locale: Some(locale),
        log_path,
        system_encoding: eve_platform::locale::system_encoding().and_then(Encoding::from_label),
        clipboard: Some(clipboard),
        // 緊急退避の書き手 (公開済みの DocSnapshot だけを読み、Port を通さずに書く、02 §11.2.1)
        rescue_writer: Some(Arc::new(StateRescue::new(Some(
            state_dir.as_path().to_path_buf(),
        )))),
        ssh_session: None,
    };
    let host = Host::with_channel(ports, (tx.clone(), rx), host_init);
    // 非対話の終了の手順 (退避 → メインへの通知と終了待ち → 置換中の計数の待ち → 端末の緊急復元 → 退避の結果の行、
    // 02 §9.6 / §11.2.1)。手順と予算は eve-host が持ち、OS のコールバックへの登録は `os_events` が行う
    let shutdown = NonInteractiveShutdown::new(ShutdownParts {
        emergency: host.emergency(),
        replacing: replacing.clone(),
        restore_info: host.restore_info(),
        run_finished: host.run_finished(),
        tx,
    })
    .with_report(|o| {
        let mut err = std::io::stderr().lock();
        for l in texts::recovery_lines(o) {
            l.write_to(&mut err);
        }
    });
    Ok(Started {
        host,
        shutdown,
        replacing,
        state_dir: state_dir.as_path().to_path_buf(),
        locale,
        color_mode,
        stdin,
    })
}
