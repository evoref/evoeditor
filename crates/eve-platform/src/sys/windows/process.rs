//! プロセスの API (作成時刻・他のプロセスの生存・自プロセスの強制終了)。

use eve_core::FileTime;
use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, GetProcessTimes, OpenProcess,
    PROCESS_QUERY_LIMITED_INFORMATION,
};

use super::file::last_error;

/// 100 ns 単位・1601-01-01 起点と UNIX 紀元の差 (秒)。
const EPOCH_DIFF_SECS: i64 = 11_644_473_600;

const ZERO: FILETIME = FILETIME {
    dwLowDateTime: 0,
    dwHighDateTime: 0,
};

fn ticks(t: &FILETIME) -> u64 {
    (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)
}

/// 自プロセスの作成時刻 (UNIX 紀元からの経過)。取れなければ `None`。
pub(crate) fn process_start() -> Option<FileTime> {
    let (mut c, mut e, mut k, mut u) = (ZERO, ZERO, ZERO, ZERO);
    // SAFETY: 自プロセスのハンドルと、書込先の有効な `FILETIME` 4 つ。
    let ok = unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) };
    if ok == 0 {
        return None;
    }
    let ticks = ticks(&c);
    // 100 ns 単位、1601-01-01 起点 → UNIX 紀元
    let secs = (ticks / 10_000_000) as i64 - EPOCH_DIFF_SECS;
    let nanos = ((ticks % 10_000_000) * 100) as u32;
    Some(FileTime { secs, nanos })
}

/// 他のプロセスを問い合わせた結果 (`process_state`)。
pub(crate) enum ProcessState {
    /// 開けなかった。`denied` はアクセス拒否 (いるが開けない)。それ以外 (無効な pid) はいない。
    NotOpened { denied: bool },
    /// 開けた。`running` は終了コードが `STILL_ACTIVE`。`started` は作成時刻 (秒だけ。問い合わせたときだけ、取れたときだけ)。
    Opened {
        running: bool,
        started: Option<FileTime>,
    },
}

/// pid のプロセスを `OpenProcess` で開き、動いているかと (`want_start` で動いていれば) 作成時刻を問い合わせる。
pub(crate) fn process_state(pid: u32, want_start: bool) -> ProcessState {
    const STILL_ACTIVE: u32 = 259;
    const ERROR_ACCESS_DENIED: u32 = 5;
    // SAFETY: 有効な引数。返ったハンドルは下で閉じる。
    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if h.is_null() {
        // 直前の呼出しのエラーを読む
        return ProcessState::NotOpened {
            denied: last_error() == ERROR_ACCESS_DENIED,
        };
    }
    let mut code = 0u32;
    // SAFETY: 有効なハンドルと書込先。
    let running = unsafe { GetExitCodeProcess(h, &mut code) } != 0 && code == STILL_ACTIVE;
    let mut started = None;
    if running && want_start {
        let (mut c, mut e, mut k, mut u) = (ZERO, ZERO, ZERO, ZERO);
        // SAFETY: 有効なハンドルと書込先の `FILETIME` 4 つ。
        if unsafe { GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u) } != 0 {
            let secs = (ticks(&c) / 10_000_000) as i64 - EPOCH_DIFF_SECS;
            started = Some(FileTime { secs, nanos: 0 });
        }
    }
    // SAFETY: `OpenProcess` が返したハンドル。
    unsafe { CloseHandle(h) };
    ProcessState::Opened { running, started }
}

/// プロセスを今すぐ終了する (`TerminateProcess`。Drop も `atexit` も走らせない。テストのフックだけ)。
#[cfg(feature = "test-hooks")]
pub(crate) fn kill_self() -> ! {
    use windows_sys::Win32::System::Threading::TerminateProcess;
    // SAFETY: 自プロセスのハンドルを終了させる。
    unsafe {
        TerminateProcess(GetCurrentProcess(), 137);
    }
    // 戻らないはずだが、型のために
    std::process::abort()
}
