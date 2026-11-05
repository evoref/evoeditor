//! テスト専用のフック (docs/09 §3.6 (6)、05 §4.4 手順 10)。どれも配布ビルド (feature `test-hooks` も `cfg(test)` も無い) では
//! 何もしない (空の関数)。
//!
//! - `hit` / `value`: feature `test-hooks` のとき、環境変数 `EVE_TEST_HOOK=<切断点>[=<値>]` で指した切断点に到達したら、プロセスを
//!   **後始末なしで強制終了** (Windows は `TerminateProcess`、Unix は `SIGKILL`) する。
//! - `fail` (Unix): システムコールの失敗の差し替え (errno)。単体試験は `inject_errno`、別プロセスの試験は
//!   `EVE_TEST_HOOK=fail_<点>=<errno>` (docs/review/v0.3-findings.md の STR-T10)。

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

/// システムコールの失敗の差し替え (docs/review/v0.3-findings.md の STR-T10)。呼び出し点 `point` で、試験が指した errno の
/// エラーを返す (呼び出し側は本物の呼び出しをせずにそれを返す)。配布ビルドでは常に `None`。
///
/// - 単体試験 (`cfg(test)`): `inject_errno(point, errno)` で、**そのスレッドの次の 1 回だけ** 失敗させる (並行する試験と混ざらない)。
/// - feature `test-hooks`: 環境変数 `EVE_TEST_HOOK=fail_<point>=<errno>` で、その点を毎回失敗させる (別プロセスの試験用)。
///   errno は数で書く (読めなければ panic する: 黙って何も差し替えずに試験が通るのを防ぐ)。
///
/// Unix の保存 (`unix_fs.rs`) だけが使う。呼び出し点 (名前は OS に依らない):
/// `replace` (既存の T の置換の rename)・`rename_noreplace` (新規ファイルの上書きしない移動。Linux は `renameat2`)・
/// `link` / `unlink_after_link` (その退路)・`fsync` (一時ファイル)・`fsync_dir` (ディレクトリ)・`fchown` (所有者の複製)。
/// 注意: 差し替えるのはその点の 1 回の呼び出しだけで、中断の後片付け (`abort` の一時ファイルの削除) は本物のまま走る
/// (`unlink_after_link` を失敗させても、後片付けの削除は成功する。本物の失敗では後片付けも失敗し得る)。
#[cfg(unix)]
#[inline]
pub(crate) fn fail(point: &str) -> Option<std::io::Error> {
    #[cfg(test)]
    if let Some(e) = take_injected(point) {
        return Some(std::io::Error::from_raw_os_error(e));
    }
    #[cfg(feature = "test-hooks")]
    if let Some(v) = active(&format!("fail_{point}")) {
        let errno = v.parse().unwrap_or_else(|_| {
            panic!("EVE_TEST_HOOK の fail_{point}= には errno を数で書きます: {v:?}")
        });
        return Some(std::io::Error::from_raw_os_error(errno));
    }
    let _ = point;
    None
}

#[cfg(all(test, unix))]
thread_local! {
    static INJECTED: std::cell::RefCell<Vec<(String, i32)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// `inject_errno` の結果。落とすときに、差し替えが使われたか (その点を通ったか) を確かめる (使われずに試験が通るのを防ぐ)。
#[cfg(all(test, unix))]
#[must_use = "差し替えが使われたかを、落とすときに確かめる"]
pub(crate) struct Injected {
    point: String,
}

#[cfg(all(test, unix))]
impl Drop for Injected {
    fn drop(&mut self) {
        let pending = INJECTED.with(|v| v.borrow().iter().any(|(p, _)| *p == self.point));
        if pending && !std::thread::panicking() {
            INJECTED.with(|v| v.borrow_mut().retain(|(p, _)| *p != self.point));
            panic!(
                "差し替え {} が使われなかった (その呼び出し点を通っていない)",
                self.point
            );
        }
    }
}

/// このスレッドで、次に `point` を通るときに `errno` で失敗させる (1 回だけ)。
#[cfg(all(test, unix))]
pub(crate) fn inject_errno(point: &str, errno: i32) -> Injected {
    INJECTED.with(|v| v.borrow_mut().push((point.to_owned(), errno)));
    Injected {
        point: point.to_owned(),
    }
}

#[cfg(all(test, unix))]
fn take_injected(point: &str) -> Option<i32> {
    INJECTED.with(|v| {
        let mut v = v.borrow_mut();
        let i = v.iter().position(|(p, _)| p == point)?;
        Some(v.remove(i).1)
    })
}

/// フックが `name` を指していれば、その値の文字列 (値なしは空)。
#[cfg(feature = "test-hooks")]
fn active(name: &str) -> Option<String> {
    lookup(&std::env::var("EVE_TEST_HOOK").ok()?, name)
}

/// `EVE_TEST_HOOK` の値 `spec` の中の `name` の値 (値なしは空)。値は `,` 区切りで複数指定できる
/// (`panic_command,recovery_timeout=0`)。環境変数を読まない純粋な関数 (試験で `set_var` せずに確かめる)。
#[cfg(any(test, feature = "test-hooks"))]
fn lookup(spec: &str, name: &str) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 環境変数は読まずに、値の解釈だけを確かめる (`set_var` は同じプロセスで並行に走る保存の試験に効いてしまう)。
    #[test]
    fn 環境変数の値の解釈() {
        let spec = "kill_mid_inplace_write=1234,panic_command,fail_replace=18";
        assert_eq!(
            lookup(spec, "kill_mid_inplace_write").as_deref(),
            Some("1234")
        );
        assert_eq!(lookup(spec, "panic_command").as_deref(), Some(""));
        assert_eq!(lookup(spec, "fail_replace").as_deref(), Some("18"));
        assert_eq!(lookup(spec, "kill_after_backup"), None);
        assert_eq!(lookup("", "panic_command"), None);
    }

    #[cfg(unix)]
    #[test]
    fn 差し替えは指した点の次の_1_回だけ() {
        let g = inject_errno("fsync", libc::EIO);
        assert!(fail("replace").is_none(), "別の点には効かない");
        assert_eq!(
            fail("fsync").and_then(|e| e.raw_os_error()),
            Some(libc::EIO)
        );
        assert!(fail("fsync").is_none(), "1 回だけ");
        drop(g);
    }

    #[cfg(unix)]
    #[test]
    #[should_panic(expected = "使われなかった")]
    fn 使われなかった差し替えは試験を落とす() {
        let _g = inject_errno("fsync_dir", libc::EIO);
    }
}
