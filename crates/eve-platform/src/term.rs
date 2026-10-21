//! 緊急の端末復元 (docs/05 §11.2「端末の緊急復元」、02 §9.6 手順 4)。メインが止まったとき、コンソールのハンドラの
//! スレッド (受信スレッド) が呼ぶ。frontend に触れず、起動時に渡された値だけで復元する。
//!
//! 書込は **使い捨てのスレッド** で行う (コンソールの選択モード中などは `WriteConsoleW` が止まるため)。呼び側は最大
//! `WAIT` だけ待って先へ進む。復元は frontend の `restore` と「復元済み」のフラグを共有し、`false → true` に立てた側だけが行う。

#![allow(unsafe_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Console::{SetConsoleMode, WriteConsoleW};

/// 復元の書込を待つ上限 (02 §11.2.1 の表の 4)。
const WAIT: Duration = Duration::from_millis(200);

/// 端末を緊急に復元する。`handle` は描画先 (コンソールの出力) の生ハンドル、`bytes` は CAN + 復元の固定の列 (ASCII)、
/// `saved_mode` は出力 (u32 LE) + 入力 (u32 LE) のモード、`restored` は frontend と共有する「復元済み」のフラグ。
/// 復元の担当になった (フラグを立てた) ときは `true`。他の経路が復元済みなら何もせず `false`。
pub fn emergency_restore(
    handle: usize,
    bytes: &'static [u8],
    saved_mode: &[u8],
    restored: &AtomicBool,
) -> bool {
    if restored.swap(true, Ordering::AcqRel) {
        return false;
    }
    let modes = parse_modes(saved_mode);
    let (tx, rx) = mpsc::channel::<()>();
    // 使い捨てのスレッド (02 §9.1 の表: 緊急の端末復元)。止まっても、呼び側は WAIT で先へ進む
    #[allow(clippy::disallowed_methods)]
    let spawned = std::thread::Builder::new()
        .name("eve-term-restore".to_owned())
        .spawn(move || {
            restore_now(handle as HANDLE, bytes, modes);
            let _ = tx.send(());
        });
    if spawned.is_ok() {
        let _ = rx.recv_timeout(WAIT);
    }
    true
}

/// `saved_mode` (出力 4 バイト + 入力 4 バイト、LE) を読む。短ければ該当のモードは戻さない。
fn parse_modes(b: &[u8]) -> (Option<u32>, Option<u32>) {
    let word = |i: usize| {
        b.get(i..i + 4)
            .and_then(|s| <[u8; 4]>::try_from(s).ok())
            .map(u32::from_le_bytes)
    };
    (word(0), word(4))
}

fn restore_now(out: HANDLE, bytes: &[u8], (out_mode, in_mode): (Option<u32>, Option<u32>)) {
    let units: Vec<u16> = bytes.iter().map(|&b| u16::from(b)).collect();
    let mut written = 0u32;
    // SAFETY: `out` は起動時に得たコンソール出力のハンドル、`units` は有効な u16 の領域、`written` は書込先の有効な u32。
    unsafe {
        WriteConsoleW(
            out,
            units.as_ptr().cast(),
            units.len() as u32,
            &mut written,
            std::ptr::null(),
        );
    }
    if let Some(m) = out_mode {
        // SAFETY: `out` は有効なコンソール出力のハンドル。
        unsafe { SetConsoleMode(out, m) };
    }
    if let Some(m) = in_mode {
        let wide: Vec<u16> = "CONIN$".encode_utf16().chain(Some(0)).collect();
        // SAFETY: `wide` は NUL 終端の UTF-16。残りの引数は `CreateFileW` の仕様どおり。
        let h = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        if h != INVALID_HANDLE_VALUE {
            // SAFETY: `h` は今開いた有効なコンソール入力のハンドル。
            unsafe {
                SetConsoleMode(h, m);
                windows_sys::Win32::Foundation::CloseHandle(h);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn モードの読み() {
        let mut b = Vec::new();
        b.extend_from_slice(&7u32.to_le_bytes());
        b.extend_from_slice(&9u32.to_le_bytes());
        assert_eq!(parse_modes(&b), (Some(7), Some(9)));
        assert_eq!(parse_modes(&b[..4]), (Some(7), None));
        assert_eq!(parse_modes(&[]), (None, None));
    }

    #[test]
    fn 復元済みなら何もしない() {
        let f = AtomicBool::new(true);
        assert!(!emergency_restore(0, b"", &[], &f));
    }
}
