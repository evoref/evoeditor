//! 終了コード (07 §18.4) と、端末の復元の後に標準エラーへ出す行 (終了時の行・退避の結果・保存の待ち・パニック)。

use std::io::Write as _;

use eve_config::Catalog;
use eve_core::RescueReason;
use eve_host::{EmergencyCtl, Host, NonInteractiveShutdown};

use crate::{panic, texts};

/// 実行時エラーでの終了 (07 §18.4)。
pub(crate) const EXIT_ERROR: u8 = 1;
/// 使い方の誤り。
pub(crate) const EXIT_USAGE: u8 = 2;
/// `--check-config` でエラーの診断があった。
pub(crate) const EXIT_CONFIG_ERROR: u8 = 3;
/// 端末の初期化に失敗 (`Host::run` が返す。ここでは Unix のバックグラウンドのジョブでだけ使う)。
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) const EXIT_TERMINAL: u8 = 5;
/// パニック。
pub(crate) const EXIT_PANIC: u8 = 70;

/// 端末の復元の後に出す行 (`Effect::Quit` の `stderr` と起動の失敗)。定数表 (英語の行、日本語の行の順) にあるキーはそれで、
/// 無ければ選択中の言語の **同梱の** カタログ (ユーザーの言語ファイルを重ねない。終了の経路にユーザーの文言を入れない、
/// 07 §17.4) で解決し、それも無ければキー名を出す。
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

/// 置換中フラグが下りるまで (期限なしで) 待つ。上がっていたら、待ち始める前に stderr へ 1 行ずつ知らせる (02 §9.6)。
pub(crate) fn wait_replacing(
    shutdown: &NonInteractiveShutdown,
    replacing: &eve_core::ReplacingCount,
) {
    if replacing.is_zero() {
        return;
    }
    {
        let mut err = std::io::stderr().lock();
        for p in replacing.paths() {
            texts::waiting_save(&p).write_to(&mut err);
        }
    }
    shutdown.wait_replacing(None);
}

/// `Host::run` が戻ったときの終わり方: 無視した引数 → 終了時の行 → 退避の結果。`Host::run` の終了コードを返す。
pub(crate) fn finish(
    host: &mut Host,
    locale: eve_config::Locale,
    ignored: &[String],
    emergency: &EmergencyCtl,
    code: eve_host::ExitCode,
) -> u8 {
    if !ignored.is_empty() {
        texts::ignored_args(ignored).write_to(&mut std::io::stderr().lock());
    }
    print_exit_lines(host, locale);
    // シグナル・致命的エラーで終わったときの退避の結果 (退避したときだけ。通常終了では書いていない)
    if let Some(o) = emergency.outcome() {
        let mut err = std::io::stderr().lock();
        for l in texts::recovery_lines(&o) {
            l.write_to(&mut err);
        }
    }
    code.0
}

/// パニックのときの終わり方: 退避 (最大 5 s。メインの `catch_unwind` の後) → 端末の復元は済み → panic.header → 退避の結果 →
/// クラッシュログ → 報告先。
pub(crate) fn finish_panic(
    emergency: &EmergencyCtl,
    shutdown: &NonInteractiveShutdown,
    state_dir: &std::path::Path,
) -> u8 {
    let rescued = emergency.run(RescueReason::Panic, eve_host::RESCUE_BUDGET_FATAL);
    // 退避の後、置換中フラグを最大 1 s 待つ (合計 5 s の内、02 §11.2.1)
    shutdown.wait_replacing(Some(eve_host::REPLACING_WAIT_FATAL));
    let mut err = std::io::stderr().lock();
    texts::panic_header().write_to(&mut err);
    if let Some(o) = rescued {
        for l in texts::recovery_lines(&o) {
            l.write_to(&mut err);
        }
    }
    if let Some(path) = panic::write_crash_log(state_dir) {
        texts::panic_crash_log(&path).write_to(&mut err);
    }
    texts::panic_report(&format!("{}/issues", env!("CARGO_PKG_REPOSITORY"))).write_to(&mut err);
    EXIT_PANIC
}
