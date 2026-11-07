//! Windows のシステムクリップボード (docs/05 §8.1)。`ClipboardProvider` の実装 `WinClipboard` は
//! Win32 API (`windows-sys`) を直接呼ぶ。専用ライブラリは使わない (OQ-111)。
//!
//! - **所有者ウィンドウ**: 書込は `OpenClipboard(NULL)` ではなく、メッセージ専用ウィンドウ (`HWND_MESSAGE`) を所有者にして開く。このウィンドウは
//!   最初の書込・読込で専用のスレッドに作り、そのスレッドでメッセージループを回す (他のアプリが `EmptyClipboard` したときに所有者へ
//!   送られる `WM_DESTROYCLIPBOARD` を処理するため。回さないと相手のアプリを待たせる)。
//! - **競合**: `OpenClipboard` は他のプロセスが開いていると失敗するので、10 ms 間隔で最大 5 回やり直す。
//! - **改行**: `CF_UNICODETEXT` のまま入れ・読む。CRLF の正規化は core の貼り付けの入口で行う。

#![allow(unsafe_code)]

use std::sync::OnceLock;
use std::sync::mpsc;
use std::time::Duration;

use windows_sys::Win32::Foundation::{GlobalFree, HANDLE, HWND};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    SetClipboardData,
};
use windows_sys::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DispatchMessageW, GetMessageW, HWND_MESSAGE, MSG, TranslateMessage,
};

use super::{ClipboardFailure, ClipboardProvider, SYSTEM_MAX_BYTES};

/// `CF_UNICODETEXT`。
const CF_UNICODETEXT: u32 = 13;
/// `OpenClipboard` のやり直し (10 ms × 5)。
const OPEN_RETRIES: u32 = 5;
const OPEN_INTERVAL: Duration = Duration::from_millis(10);

/// Win32 のクリップボード API の薄い層 (L0 で差し替えて、失敗と再試行の分岐を検査する)。
pub trait ClipApi: Send + Sync {
    /// 所有者ウィンドウ (最初の呼出しで作る)。作れなければ `None`。
    fn owner(&self) -> Option<usize>;
    fn open(&self, owner: usize) -> bool;
    fn close(&self);
    fn empty(&self) -> bool;
    /// UTF-16 (終端 NUL なし) を `CF_UNICODETEXT` として入れる。
    fn set_unicode(&self, units: &[u16]) -> bool;
    fn has_unicode(&self) -> bool;
    /// `CF_UNICODETEXT` の中身 (終端 NUL なし)。`max_units` を超えるなら `Err(TooLarge)`。
    fn get_unicode(&self, max_units: usize) -> Result<Option<Vec<u16>>, ClipboardFailure>;
}

pub struct Win32Api {
    owner: OnceLock<Option<usize>>,
}

impl Win32Api {
    pub(super) fn new() -> Win32Api {
        Win32Api {
            owner: OnceLock::new(),
        }
    }
}

/// 所有者ウィンドウの専用スレッドを起動し、ウィンドウのハンドルを受け取る。
// 所有者ウィンドウのスレッドは 02 §9.1 の表のスレッド (R11 の例外。deps-policy.toml の `allowed_files`)。
#[allow(clippy::disallowed_methods)]
fn spawn_owner() -> Option<usize> {
    let (tx, rx) = mpsc::channel::<Option<usize>>();
    std::thread::Builder::new()
        .name("eve-clipboard-owner".to_owned())
        .spawn(move || {
            // 既定のクラス "STATIC" のメッセージ専用ウィンドウ (所有者になるだけで、表示も入力もしない)
            let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
            // SAFETY: メッセージ専用ウィンドウの作成。文字列は NUL 終端の UTF-16 で、この関数の間有効。
            let hwnd = unsafe {
                CreateWindowExW(
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
                )
            };
            if hwnd.is_null() {
                let _ = tx.send(None);
                return;
            }
            let _ = tx.send(Some(hwnd as usize));
            // メッセージループ: 他のアプリが `EmptyClipboard` したときの `WM_DESTROYCLIPBOARD` などを処理する
            // SAFETY: 自分のスレッドのメッセージキューを読む。`msg` は有効な書込先。
            unsafe {
                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&raw mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                    TranslateMessage(&raw const msg);
                    DispatchMessageW(&raw const msg);
                }
            }
        })
        .ok()?;
    rx.recv_timeout(Duration::from_secs(2)).ok().flatten()
}

impl ClipApi for Win32Api {
    fn owner(&self) -> Option<usize> {
        *self.owner.get_or_init(spawn_owner)
    }

    fn open(&self, owner: usize) -> bool {
        // SAFETY: 所有者ウィンドウのハンドル (自分で作ったもの)。
        unsafe { OpenClipboard(owner as HWND) != 0 }
    }

    fn close(&self) {
        // SAFETY: 直前に `open` に成功している。
        unsafe { CloseClipboard() };
    }

    fn empty(&self) -> bool {
        // SAFETY: クリップボードを開いている間だけ呼ぶ。
        unsafe { EmptyClipboard() != 0 }
    }

    fn set_unicode(&self, units: &[u16]) -> bool {
        let bytes = (units.len() + 1) * 2;
        // SAFETY: 確保したグローバルメモリに UTF-16 + NUL を書いて、所有権を OS に渡す (成功時)。失敗時は自分で解放する。
        unsafe {
            let h = GlobalAlloc(GMEM_MOVEABLE, bytes);
            if h.is_null() {
                return false;
            }
            let p = GlobalLock(h).cast::<u16>();
            if p.is_null() {
                GlobalFree(h);
                return false;
            }
            std::ptr::copy_nonoverlapping(units.as_ptr(), p, units.len());
            *p.add(units.len()) = 0;
            GlobalUnlock(h);
            if SetClipboardData(CF_UNICODETEXT, h as HANDLE).is_null() {
                GlobalFree(h);
                return false;
            }
        }
        true
    }

    fn has_unicode(&self) -> bool {
        // SAFETY: 引数は書式の番号だけ。
        unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT) != 0 }
    }

    fn get_unicode(&self, max_units: usize) -> Result<Option<Vec<u16>>, ClipboardFailure> {
        // SAFETY: クリップボードを開いている間に、OS が持つグローバルメモリを読む。ロック中だけ参照し、`GlobalSize` を超えて読まない。
        unsafe {
            let h = GetClipboardData(CF_UNICODETEXT);
            if h.is_null() {
                return Ok(None);
            }
            let size_units = GlobalSize(h.cast()) / 2;
            let p = GlobalLock(h.cast()).cast::<u16>();
            if p.is_null() {
                return Err(ClipboardFailure::Failed);
            }
            let slice = std::slice::from_raw_parts(p, size_units);
            let n = slice.iter().position(|&u| u == 0).unwrap_or(size_units);
            let result = if n > max_units {
                Err(ClipboardFailure::TooLarge)
            } else {
                let mut v: Vec<u16> = Vec::new();
                if v.try_reserve_exact(n).is_err() {
                    Err(ClipboardFailure::Failed)
                } else {
                    v.extend_from_slice(&slice[..n]);
                    Ok(Some(v))
                }
            };
            GlobalUnlock(h.cast());
            result
        }
    }
}

/// Windows のシステムクリップボード。
pub struct WinClipboard<A: ClipApi = Win32Api> {
    api: A,
    retry_sleep: Duration,
}

impl WinClipboard<Win32Api> {
    pub fn new() -> WinClipboard<Win32Api> {
        WinClipboard {
            api: Win32Api::new(),
            retry_sleep: OPEN_INTERVAL,
        }
    }
}

impl Default for WinClipboard<Win32Api> {
    fn default() -> Self {
        WinClipboard::new()
    }
}

impl<A: ClipApi> WinClipboard<A> {
    #[cfg(test)]
    pub(super) fn with_api(api: A) -> WinClipboard<A> {
        WinClipboard {
            api,
            retry_sleep: Duration::ZERO,
        }
    }

    /// 開く (競合なら 10 ms × 5 回やり直す)。
    fn open(&self) -> Result<(), ClipboardFailure> {
        let owner = self.api.owner().ok_or(ClipboardFailure::Unavailable)?;
        for i in 0..OPEN_RETRIES {
            if self.api.open(owner) {
                return Ok(());
            }
            if i + 1 < OPEN_RETRIES {
                std::thread::sleep(self.retry_sleep);
            }
        }
        Err(ClipboardFailure::Unavailable)
    }
}

impl<A: ClipApi> ClipboardProvider for WinClipboard<A> {
    fn set_text(&self, text: &str) -> Result<(), ClipboardFailure> {
        if text.len() > SYSTEM_MAX_BYTES {
            return Err(ClipboardFailure::TooLarge);
        }
        let units: Vec<u16> = text.encode_utf16().collect();
        self.open()?;
        let ok = self.api.empty() && self.api.set_unicode(&units);
        self.api.close();
        if ok {
            Ok(())
        } else {
            Err(ClipboardFailure::Failed)
        }
    }

    fn get_text(&self) -> Result<Option<String>, ClipboardFailure> {
        self.open()?;
        let r = if self.api.has_unicode() {
            self.api.get_unicode(SYSTEM_MAX_BYTES / 2)
        } else {
            Ok(None)
        };
        self.api.close();
        Ok(r?.map(|units| String::from_utf16_lossy(&units)))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    /// 呼出しを記録し、`open` の失敗を注入できる偽物。
    struct Mock {
        open_fail: u32,
        opens: AtomicU32,
        closes: AtomicU32,
        stored: Mutex<Option<Vec<u16>>>,
        set_ok: bool,
    }

    impl Mock {
        fn new(open_fail: u32) -> Mock {
            Mock {
                open_fail,
                opens: AtomicU32::new(0),
                closes: AtomicU32::new(0),
                stored: Mutex::new(None),
                set_ok: true,
            }
        }
    }

    impl ClipApi for Mock {
        fn owner(&self) -> Option<usize> {
            Some(1)
        }
        fn open(&self, _: usize) -> bool {
            self.opens.fetch_add(1, Ordering::SeqCst) >= self.open_fail
        }
        fn close(&self) {
            self.closes.fetch_add(1, Ordering::SeqCst);
        }
        fn empty(&self) -> bool {
            true
        }
        fn set_unicode(&self, units: &[u16]) -> bool {
            *self.stored.lock().unwrap() = Some(units.to_vec());
            self.set_ok
        }
        fn has_unicode(&self) -> bool {
            self.stored.lock().unwrap().is_some()
        }
        fn get_unicode(&self, max: usize) -> Result<Option<Vec<u16>>, ClipboardFailure> {
            let s = self.stored.lock().unwrap().clone();
            match s {
                Some(v) if v.len() > max => Err(ClipboardFailure::TooLarge),
                other => Ok(other),
            }
        }
    }

    #[test]
    fn 書いて読める_日本語と改行も保つ() {
        let c = WinClipboard::with_api(Mock::new(0));
        c.set_text("日本語\r\nabc😀").unwrap();
        assert_eq!(c.get_text().unwrap().as_deref(), Some("日本語\r\nabc😀"));
        assert_eq!(c.api.opens.load(Ordering::SeqCst), 2);
        assert_eq!(c.api.closes.load(Ordering::SeqCst), 2, "開いたら必ず閉じる");
    }

    #[test]
    fn 開けなければ最大_5_回やり直す() {
        let c = WinClipboard::with_api(Mock::new(3));
        c.set_text("x").unwrap();
        assert_eq!(c.api.opens.load(Ordering::SeqCst), 4);
        // 5 回とも失敗: Unavailable
        let c = WinClipboard::with_api(Mock::new(99));
        assert_eq!(c.set_text("x"), Err(ClipboardFailure::Unavailable));
        assert_eq!(c.api.opens.load(Ordering::SeqCst), 5);
        assert_eq!(c.get_text(), Err(ClipboardFailure::Unavailable));
    }

    #[test]
    fn 設定の失敗は_failed_で_開いたら閉じる() {
        let mut m = Mock::new(0);
        m.set_ok = false;
        let c = WinClipboard::with_api(m);
        assert_eq!(c.set_text("x"), Err(ClipboardFailure::Failed));
        assert_eq!(c.api.closes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn テキストが無ければ_none() {
        let c = WinClipboard::with_api(Mock::new(0));
        assert_eq!(c.get_text(), Ok(None));
    }
}
