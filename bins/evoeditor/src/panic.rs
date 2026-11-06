//! パニックの記録 (04 §14.2、09 §10)。フックは内容を覚えるだけで端末に触れず、表示とクラッシュログは端末の復元の後にする
//! (代替スクリーン内に出すと消えるため)。

/// パニックの内容 (フックが書き、復元の後に出す)。メッセージ・場所・バックトレース。文書の内容は含まない。
static PANIC_INFO: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// パニックのフックを入れる。内容とバックトレースを `PANIC_INFO` に覚えるだけ。
pub(crate) fn install_hook() {
    std::panic::set_hook(Box::new(|info| {
        if let Ok(mut m) = PANIC_INFO.lock() {
            let bt = std::backtrace::Backtrace::force_capture();
            *m = Some(format!("{info}\n\n{bt}"));
        }
    }));
}

/// クラッシュログを `state_dir/log/crash-<pid>-<秒>.txt` に書く (09 §10)。パニックのメッセージ・場所・バックトレースだけで、
/// 文書の内容は含まない。書けなければ `None`。
pub(crate) fn write_crash_log(state_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let info = PANIC_INFO.lock().ok()?.clone()?;
    let body = format!(
        "evoeditor {} のクラッシュログ (文書の内容は含みません)\n\n{info}\n",
        env!("CARGO_PKG_VERSION")
    );
    // 所有者のみで作り、最新 20 件を残す (05 §7.3。docs/review/v0.3-findings.md の SAF-11)
    eve_platform::log::write_crash_log(state_dir, &body).ok()
}
