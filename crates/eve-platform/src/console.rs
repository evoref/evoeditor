//! コンソールイベントの受信 (docs/05 §11.2、Windows)。`SetConsoleCtrlHandler` の登録と、受けたイベントを呼び出し側の
//! コールバックへ渡す部分だけ。緊急退避・終了コードの決定・端末の復元 (CLOSE で待つ時間の予算など) は eve-host の責務。
//!
//! ハンドラは **OS が作る別スレッド** で呼ばれる。コールバックは `Send + Sync` で、長くブロックしない (CTRL_CLOSE は
//! ハンドラが戻るとプロセスが終了されるので、呼び側が必要なだけ待つ)。

#![allow(unsafe_code)]

use std::io;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{FALSE, TRUE};
use windows_sys::Win32::System::Console::{
    CTRL_BREAK_EVENT, CTRL_C_EVENT, CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT,
    SetConsoleCtrlHandler,
};

/// コンソールのコントロールイベント。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CtrlEvent {
    /// `ENABLE_PROCESSED_INPUT` を落としているので通常は来ない (`GenerateConsoleCtrlEvent` で来る)。
    CtrlC,
    /// `Ctrl+Break`。`ENABLE_PROCESSED_INPUT` と無関係に来る (S-07 で確認)。
    CtrlBreak,
    /// コンソールの閉鎖 (ウィンドウ・タブを閉じる)。
    Close,
    Logoff,
    Shutdown,
}

impl CtrlEvent {
    fn from_raw(t: u32) -> Option<CtrlEvent> {
        match t {
            CTRL_C_EVENT => Some(CtrlEvent::CtrlC),
            CTRL_BREAK_EVENT => Some(CtrlEvent::CtrlBreak),
            CTRL_CLOSE_EVENT => Some(CtrlEvent::Close),
            CTRL_LOGOFF_EVENT => Some(CtrlEvent::Logoff),
            CTRL_SHUTDOWN_EVENT => Some(CtrlEvent::Shutdown),
            _ => None,
        }
    }
}

type Callback = Box<dyn Fn(CtrlEvent) -> bool + Send + Sync>;

static CALLBACK: OnceLock<Callback> = OnceLock::new();

/// OS から呼ばれるハンドラ。`true` を返すとイベントを処理済みにする (既定のハンドラに渡さない)。
unsafe extern "system" fn handler(kind: u32) -> i32 {
    let Some(ev) = CtrlEvent::from_raw(kind) else {
        return FALSE;
    };
    match CALLBACK.get() {
        Some(cb) if cb(ev) => TRUE,
        _ => FALSE,
    }
}

/// ハンドラを登録する (プロセスで 1 回だけ。2 回目は `AlreadyExists`)。コールバックが `true` を返したイベントは処理済み。
pub fn install_handler(cb: impl Fn(CtrlEvent) -> bool + Send + Sync + 'static) -> io::Result<()> {
    if CALLBACK.set(Box::new(cb)).is_err() {
        return Err(io::Error::from(io::ErrorKind::AlreadyExists));
    }
    // SAFETY: `handler` は `extern "system"` の関数で、プロセスの終わりまで有効。`TRUE` で追加する。
    if unsafe { SetConsoleCtrlHandler(Some(handler), TRUE) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 生の値の対応() {
        assert_eq!(CtrlEvent::from_raw(CTRL_C_EVENT), Some(CtrlEvent::CtrlC));
        assert_eq!(
            CtrlEvent::from_raw(CTRL_BREAK_EVENT),
            Some(CtrlEvent::CtrlBreak)
        );
        assert_eq!(
            CtrlEvent::from_raw(CTRL_CLOSE_EVENT),
            Some(CtrlEvent::Close)
        );
        assert_eq!(
            CtrlEvent::from_raw(CTRL_LOGOFF_EVENT),
            Some(CtrlEvent::Logoff)
        );
        assert_eq!(
            CtrlEvent::from_raw(CTRL_SHUTDOWN_EVENT),
            Some(CtrlEvent::Shutdown)
        );
        assert_eq!(CtrlEvent::from_raw(99), None);
    }
}
