//! テスト専用のクリップボード補助 (docs/09 §3.4、feature `test-hooks`)。補助プロセス `eve-test-helpers clip-delay` / `clip-read` が使う。
//! 配布ビルドには入らない。実クリップボードの失敗と遅延を作る: `hold` はクリップボードを開いたまま保持して他プロセスの取得を失敗させ、
//! `serve_delayed` は遅延レンダリング (`SetClipboardData(CF_UNICODETEXT, NULL)`) で登録して `WM_RENDERFORMAT` への応答を遅らせる。

#![allow(unsafe_code)]

use std::sync::OnceLock;
use std::time::Duration;

use windows_sys::Win32::Foundation::{GlobalFree, HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, HWND_MESSAGE, KillTimer, MSG,
    PostQuitMessage, RegisterClassW, SetTimer, TranslateMessage, WM_TIMER, WNDCLASSW,
};

use crate::clipboard::{ClipboardProvider, WinClipboard};

const CF_UNICODETEXT: u32 = 13;
const WM_RENDERFORMAT: u32 = 0x0305;

/// 遅延レンダリングで返す文字列と、応答の遅れ (ミリ秒)。
static SERVED: OnceLock<(Vec<u16>, u64)> = OnceLock::new();

/// クリップボードのテキストを読む (補助プロセス `clip-read`)。
pub fn read_text() -> Option<String> {
    WinClipboard::new().get_text().ok().flatten()
}

/// クリップボードを開いたまま `ms` ミリ秒保持する (他プロセスの `OpenClipboard` を失敗させる)。開けなければ `false`。
pub fn hold(ms: u64, ready: impl FnOnce()) -> bool {
    // SAFETY: 所有者なし (NULL) で開いて、保持した後に閉じる。
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return false;
        }
        ready();
        std::thread::sleep(Duration::from_millis(ms));
        CloseClipboard();
    }
    true
}

unsafe extern "system" fn wnd_proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match m {
        WM_RENDERFORMAT if w as u32 == CF_UNICODETEXT => {
            if let Some((units, delay)) = SERVED.get() {
                std::thread::sleep(Duration::from_millis(*delay));
                // SAFETY: 確保したグローバルメモリに UTF-16 + NUL を書いて OS に渡す (失敗時は解放する)。
                unsafe {
                    let hmem = GlobalAlloc(GMEM_MOVEABLE, (units.len() + 1) * 2);
                    if !hmem.is_null() {
                        let p = GlobalLock(hmem).cast::<u16>();
                        if p.is_null() {
                            GlobalFree(hmem);
                        } else {
                            std::ptr::copy_nonoverlapping(units.as_ptr(), p, units.len());
                            *p.add(units.len()) = 0;
                            GlobalUnlock(hmem);
                            if SetClipboardData(CF_UNICODETEXT, hmem as HANDLE).is_null() {
                                GlobalFree(hmem);
                            }
                        }
                    }
                }
            }
            0
        }
        WM_TIMER => {
            // SAFETY: 自分のウィンドウのタイマーを止めて、メッセージループを終える。
            unsafe {
                KillTimer(h, 1);
                PostQuitMessage(0);
            }
            0
        }
        // SAFETY: OS から渡された引数をそのまま既定の処理へ渡す。
        _ => unsafe { DefWindowProcW(h, m, w, l) },
    }
}

/// 遅延レンダリングで `text` を登録し、`lifetime_ms` の間 `WM_RENDERFORMAT` に `delay_ms` 遅れて応答する。
/// 登録できたら `ready` を呼び (標準出力で「準備完了」を知らせる用)、期間が過ぎるまでメッセージループを回す。
pub fn serve_delayed(text: &str, delay_ms: u64, lifetime_ms: u64, ready: impl FnOnce()) -> bool {
    let units: Vec<u16> = text.encode_utf16().collect();
    let _ = SERVED.set((units, delay_ms));
    let class: Vec<u16> = "EveClipDelay\0".encode_utf16().collect();
    // SAFETY: ウィンドウクラスの登録とメッセージ専用ウィンドウの作成、クリップボードの所有。文字列は NUL 終端の UTF-16。
    unsafe {
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: std::ptr::null_mut(),
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class.as_ptr(),
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        );
        if hwnd.is_null() || OpenClipboard(hwnd) == 0 {
            return false;
        }
        EmptyClipboard();
        // 遅延レンダリング: データは入れず、所有者として登録だけする
        SetClipboardData(CF_UNICODETEXT, std::ptr::null_mut());
        CloseClipboard();
        SetTimer(
            hwnd,
            1,
            u32::try_from(lifetime_ms).unwrap_or(u32::MAX),
            None,
        );
        ready();
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    true
}
