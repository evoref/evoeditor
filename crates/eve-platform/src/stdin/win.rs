//! 標準入力の読取の Windows の部分 (docs/05 §9.1 の「標準入出力のリダイレクト」)。OS を呼ぶのはこのファイルだけ (R5)。
//!
//! - 私有のハンドル: 継承しない複製を `DuplicateHandle` で作り、`NUL` を開いて `SetStdHandle(STD_INPUT_HANDLE, …)` で
//!   置いてから元を閉じる。ハンドルが無い・無効なら、最初の読取が失敗する。
//! - 読み方: `ReadFile` でブロックして読む。パイプの書き手が閉じた `ERROR_BROKEN_PIPE` は EOF。`eve-stdin` は `ReadFile`
//!   以外の同期の I/O をしない (`CancelSynchronousIo` はそのスレッドの同期の I/O をどれでも中止するため)。
//! - 止め方: 停止の印を立て、`CancelSynchronousIo(eve-stdin)` を `eve-stdin` が終わるまで短い間隔で繰り返す (`ReadFile` に
//!   入る前に呼ぶと `ERROR_NOT_FOUND` で空振りするので、`eve-stdin` は印を見てから読む)。最大の回数を超えたら切り離す。
//!   `eve-stdin` は印を見るか `ERROR_OPERATION_ABORTED` で戻ったら、自分で私有のハンドルを閉じて終わる (同期の読取で
//!   止まっているパイプのハンドルを別のスレッドから閉じると、閉じる側が長く止まるため)。

#![allow(unsafe_code)]

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, ERROR_BROKEN_PIPE, ERROR_HANDLE_EOF,
    ERROR_INVALID_HANDLE, ERROR_MORE_DATA, ERROR_OPERATION_ABORTED, GENERIC_READ, GetLastError,
    HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, ReadFile,
};
use windows_sys::Win32::System::Console::{
    GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
};
use windows_sys::Win32::System::IO::CancelSynchronousIo;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

use super::{CHUNK, Msg};

/// `CancelSynchronousIo` を繰り返す間隔。
const CANCEL_INTERVAL: Duration = Duration::from_millis(10);
/// `CancelSynchronousIo` を繰り返す最大の回数 (超えたら切り離す)。
const CANCEL_TRIES: u32 = 50;

/// 私有のハンドル (`None` は標準入力のハンドルが無い・無効)。
pub(super) struct Private(Option<OwnedHandle>);

impl Private {
    /// 任意のハンドル (パイプの読み端など) を私有のハンドルとして使う (試験の口 `PlatformStdin::from_pipe`)。
    pub(super) fn from_handle(h: OwnedHandle) -> Private {
        Private(Some(h))
    }

    /// 試験: 標準入力のハンドルが無い・無効なとき。
    #[cfg(test)]
    pub(super) fn none() -> Private {
        Private(None)
    }
}

/// Windows は起こすための部品を持たない (スレッドのハンドルに `CancelSynchronousIo` を送る)。
pub(super) struct Waker;
/// 同上。
pub(super) struct Stopped;

/// 標準入力を私有のハンドルへ移し、標準入力の位置に `NUL` を置いて元を閉じる。
pub(super) fn take_process_stdin() -> io::Result<Private> {
    // SAFETY: 引数の無い問い合わせ。
    let h = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
    if h.is_null() || h == INVALID_HANDLE_VALUE {
        return Ok(Private(None));
    }
    let mut dup: HANDLE = std::ptr::null_mut();
    // SAFETY: 自プロセスの開いているハンドルを、継承しない複製にする。`dup` は有効な書込先。
    let ok = unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            h,
            GetCurrentProcess(),
            &raw mut dup,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        )
    };
    if ok == 0 {
        let e = io::Error::last_os_error();
        // 値が NULL でなくても無効なハンドル (閉じたもの): 最初から読めない (05 §9.1)
        if e.raw_os_error() == Some(ERROR_INVALID_HANDLE.cast_signed()) {
            return Ok(Private(None));
        }
        return Err(e);
    }
    // SAFETY: `DuplicateHandle` が返した、ほかに持ち主の無いハンドル。
    let private = unsafe { OwnedHandle::from_raw_handle(dup) };
    let name: Vec<u16> = "NUL\0".encode_utf16().collect();
    // 継承できるハンドルで開く (Unix の fd 0 の `/dev/null` と揃える。子に `STARTF_USESTDHANDLES` で渡したときに無効な
    // ハンドルにならない)
    let sa = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(std::mem::size_of::<SECURITY_ATTRIBUTES>()).unwrap_or(0),
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    // SAFETY: NUL 終端の UTF-16 の固定の名前と、この関数の間は有効なセキュリティ属性。テンプレートは無し。
    let nul = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            &raw const sa,
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if nul == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: 開いているハンドルを標準入力の位置に置く (以後は標準入力のハンドルとしてプロセスが持つ)。
    if unsafe { SetStdHandle(STD_INPUT_HANDLE, nul) } == 0 {
        let e = io::Error::last_os_error();
        // SAFETY: 上で開き、どこにも渡していないハンドル。
        unsafe { CloseHandle(nul) };
        return Err(e);
    }
    // 元のハンドルを閉じる (子プロセスがパイプの読み手を持ち続けない。私有の複製だけが読む)。ただし標準出力・標準エラー
    // と同じ値のハンドル (同じものを渡された起動) は閉じない: 閉じると標準エラーも閉じ、閉じた値はすぐ別のものに
    // 再利用される (std は ERROR_INVALID_HANDLE の書込を黙って成功として扱う)。05 §9.1
    // SAFETY: 引数の無い問い合わせ。
    let (out, err) = unsafe {
        (
            GetStdHandle(STD_OUTPUT_HANDLE),
            GetStdHandle(STD_ERROR_HANDLE),
        )
    };
    if h != out && h != err {
        // SAFETY: 標準入力の位置から外した元のハンドル (ほかの std の位置にも無い)。std の `Stdin` は呼ぶたびに
        // `GetStdHandle` で引き直す。
        unsafe { CloseHandle(h) };
    }
    Ok(Private(Some(private)))
}

/// 起こすための部品 (Windows は無い)。
pub(super) fn stop_pair() -> io::Result<(Waker, Stopped)> {
    Ok((Waker, Stopped))
}

/// `eve-stdin` を止める: 停止の印 (呼ぶ側が立てた) を見るまで `CancelSynchronousIo` を繰り返す。`eve-stdin` が終われば
/// すぐ戻り、最大の回数を超えたら切り離す (呼ぶ側が `JoinHandle` を drop する)。`wait` が偽 (読み手が取られないまま
/// 手放された) なら 1 回だけ送って待たない。
pub(super) fn stop(_waker: Waker, thread: &JoinHandle<()>, finished: &AtomicBool, wait: bool) {
    let h = thread.as_raw_handle();
    let tries = if wait { CANCEL_TRIES } else { 1 };
    for i in 0..tries {
        if finished.load(Ordering::Acquire) {
            return;
        }
        // SAFETY: `JoinHandle` が持つ、開いているスレッドのハンドル (`thread` の借用の間は閉じない)。読取の中で
        // なければ ERROR_NOT_FOUND で何もしない。
        unsafe { CancelSynchronousIo(h) };
        if i + 1 < tries {
            std::thread::sleep(CANCEL_INTERVAL);
        }
    }
}

/// `eve-stdin` の本体。終わる前に私有のハンドルを閉じる (この関数が持って drop する)。`_blocking` は Unix の試験用。
pub(super) fn read_loop(
    private: Private,
    _stopped: Stopped,
    stop: &AtomicBool,
    _blocking: bool,
    sink: &mut dyn FnMut(Msg) -> bool,
) {
    let Some(handle) = private.0 else {
        sink(Msg::Err(io::Error::from_raw_os_error(
            ERROR_INVALID_HANDLE.cast_signed(),
        )));
        return;
    };
    let mut buf = vec![0u8; CHUNK];
    let len = u32::try_from(buf.len()).unwrap_or(u32::MAX);
    loop {
        // 印を見てから読む (読取の前の `CancelSynchronousIo` は空振りするため)
        if stop.load(Ordering::Acquire) {
            break;
        }
        let mut n: u32 = 0;
        // SAFETY: 開いているハンドル (この関数が持つ)。`buf` は `len` バイトの有効な書込先、`n` は有効な書込先。
        // 同期の読取 (オーバーラップなし)。
        let ok = unsafe {
            ReadFile(
                handle.as_raw_handle(),
                buf.as_mut_ptr(),
                len,
                &raw mut n,
                std::ptr::null_mut(),
            )
        };
        if ok != 0 {
            if n == 0 {
                sink(Msg::Eof);
                break;
            }
            if !sink(Msg::Data(buf[..n as usize].to_vec())) {
                break;
            }
            continue;
        }
        // SAFETY: 引数の無い問い合わせ (直前の `ReadFile` の失敗の理由)。
        let err = unsafe { GetLastError() };
        // メッセージ型のパイプで、メッセージがバッファより長い: 読めた分はデータとして渡し、残りを続けて読む
        if err == ERROR_MORE_DATA && n > 0 {
            if !sink(Msg::Data(buf[..n as usize].to_vec())) {
                break;
            }
            continue;
        }
        match err {
            ERROR_BROKEN_PIPE | ERROR_HANDLE_EOF => {
                sink(Msg::Eof);
                break;
            }
            // 止める合図 (`CancelSynchronousIo`)。印が立っていなければ (ほかの理由の中止) 読み直す
            ERROR_OPERATION_ABORTED => {}
            _ => {
                sink(Msg::Err(io::Error::from_raw_os_error(err.cast_signed())));
                break;
            }
        }
    }
    // 私有のハンドルはこのスレッドで閉じる (別のスレッドから閉じない、05 §9.1)
    drop(handle);
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use windows_sys::Win32::Foundation::GetHandleInformation;

    use super::*;
    use crate::stdin::tests::{CHILD_ENV, spawn_child};

    const SAME_TEST: &str = "stdin::win::tests::子_標準エラーと同じハンドルの標準入力";

    /// 子: 標準入力のハンドルを標準エラーの位置にも置いてから (同じ値を渡された起動の再現) 移し替え、標準エラーの
    /// ハンドルが開いたままかを書く。
    #[test]
    fn 子_標準エラーと同じハンドルの標準入力() {
        if std::env::var(CHILD_ENV).as_deref() != Ok("same") {
            return;
        }
        // SAFETY: 引数の無い問い合わせ。
        let h = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        // SAFETY: 自プロセスの開いているハンドルを標準エラーの位置にも置く。
        unsafe { SetStdHandle(STD_ERROR_HANDLE, h) };
        let p = take_process_stdin().expect("移し替え");
        let mut flags = 0u32;
        // SAFETY: ハンドルの情報を問い合わせるだけ (閉じていれば失敗する)。`flags` は有効な書込先。
        let alive = unsafe { GetHandleInformation(h, &raw mut flags) } != 0;
        // SAFETY: 引数の無い問い合わせ。
        let err_now = unsafe { GetStdHandle(STD_ERROR_HANDLE) };
        drop(p);
        let mut out = std::io::stdout();
        let _ = writeln!(out, "@@alive={alive} same={}", err_now == h);
        let _ = out.flush();
    }

    #[test]
    fn 標準入力と標準エラーが同じハンドルでも標準エラーを閉じない() {
        if std::env::var_os(CHILD_ENV).is_some() {
            return;
        }
        let mut child = spawn_child(SAME_TEST, "same", std::process::Stdio::piped());
        drop(child.stdin.take());
        let out = child.wait_with_output().expect("子の終わり");
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("@@alive=true same=true"), "子の出力: {text}");
    }
}
