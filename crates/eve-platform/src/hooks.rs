//! テスト専用のフック (docs/09 §3.6 (6)、05 §4.4 手順 10)。cargo feature `test-hooks` のときだけ動き、環境変数
//! `EVE_TEST_HOOK=<切断点>[=<値>]` で指した切断点に到達したら、プロセスを **後始末なしで強制終了** (Windows は `TerminateProcess`、Unix は `SIGKILL`) する。
//! 配布ビルドには feature が無く、`hit` / `value` は何もしない (空の関数)。

#![allow(unsafe_code)]

/// 切断点 `name` に到達した。環境変数が `name` (または `name=<値>`) なら、ここでプロセスを終了する。
#[inline]
pub(crate) fn hit(name: &str) {
    #[cfg(feature = "test-hooks")]
    {
        if active(name).is_some() {
            kill_now();
        }
    }
    #[cfg(not(feature = "test-hooks"))]
    {
        let _ = name;
    }
}

/// 切断点 `name=<値>` の値 (例: `kill_mid_inplace_write=<バイト数>`)。フックが指されていなければ `None`。
#[inline]
pub(crate) fn value(name: &str) -> Option<u64> {
    #[cfg(feature = "test-hooks")]
    {
        active(name).and_then(|v| v.parse().ok())
    }
    #[cfg(not(feature = "test-hooks"))]
    {
        let _ = name;
        None
    }
}

/// フックが `name` を指していれば、その値の文字列 (値なしは空)。
#[cfg(feature = "test-hooks")]
fn active(name: &str) -> Option<String> {
    // 値は `,` 区切りで複数指定できる (`panic_command,recovery_timeout=0`)
    let spec = std::env::var("EVE_TEST_HOOK").ok()?;
    spec.split(',').find_map(|h| {
        let (n, v) = h.split_once('=').unwrap_or((h, ""));
        (n == name).then(|| v.to_owned())
    })
}

/// プロセスを今すぐ終了する (Drop も `atexit` も走らせない。`TerminateProcess` による強制終了の再現)。
#[cfg(all(feature = "test-hooks", unix))]
pub(crate) fn kill_now() -> ! {
    // SAFETY: 自プロセスに SIGKILL を送る (捕まえられない)。
    unsafe {
        libc::kill(libc::getpid(), libc::SIGKILL);
    }
    std::process::abort()
}

/// 同上 (Windows: `TerminateProcess`)。
#[cfg(all(feature = "test-hooks", windows))]
pub(crate) fn kill_now() -> ! {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
    // SAFETY: 自プロセスのハンドルを終了させる。
    unsafe {
        TerminateProcess(GetCurrentProcess(), 137);
    }
    // 戻らないはずだが、型のために
    std::process::abort()
}

#[cfg(all(test, feature = "test-hooks"))]
mod tests {
    use super::*;

    #[test]
    fn 環境変数の解釈() {
        // SAFETY: テストは単一スレッドの環境変数の操作 (他のテストはこの変数を使わない)。
        unsafe { std::env::set_var("EVE_TEST_HOOK", "kill_mid_inplace_write=1234") };
        assert_eq!(value("kill_mid_inplace_write"), Some(1234));
        assert_eq!(value("kill_after_backup"), None);
        // SAFETY: 上と同じ (単一スレッドの環境変数の操作)。
        unsafe { std::env::remove_var("EVE_TEST_HOOK") };
        assert_eq!(value("kill_mid_inplace_write"), None);
    }
}
