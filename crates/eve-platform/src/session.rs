//! セッション終了 (ログオフ・シャットダウン) の受信 (docs/05 §11.2 の「セッション終了の受け方」、02 §11.2.1、Windows、v0.2)。
//!
//! `CTRL_LOGOFF_EVENT` / `CTRL_SHUTDOWN_EVENT` は対話的なアプリに届かず、メッセージ専用ウィンドウ (`HWND_MESSAGE`) は
//! ブロードキャストを受けない。そこで専用スレッドに **非表示のトップレベルウィンドウ** を作り、`WM_QUERYENDSESSION` /
//! `WM_ENDSESSION` を受ける。緊急退避の実行と時間予算は呼び出し側 (bin) の責務で、ここはウィンドウとメッセージループと、
//! 受けた通知をコールバックへ渡す部分だけ。ベストエフォート (到達は S-11 で実機で確かめる)。

#![allow(unsafe_code)]

use std::io;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::Shutdown::{ShutdownBlockReasonCreate, ShutdownBlockReasonDestroy};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GWLP_USERDATA, GetMessageW,
    GetWindowLongPtrW, MSG, RegisterClassW, SetWindowLongPtrW, TranslateMessage, WM_ENDSESSION,
    WM_QUERYENDSESSION, WNDCLASSW,
};

/// 受けたセッション終了の通知。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SessionEvent {
    /// `WM_QUERYENDSESSION`: セッションを終えてよいかの問い合わせ。退避を **始める** 契機。コールバックは使い捨てのスレッド
    /// (`eve-session-query`) で呼ぶので、そこで退避を書いてよい (メッセージループは待たずに `TRUE` を返す)。
    QueryEnd,
    /// `WM_ENDSESSION`。`ending` が真ならセッションは終わる (戻るとプロセスが終了されうる)。偽は取り消し。
    End { ending: bool },
}

type Callback = Arc<dyn Fn(SessionEvent) + Send + Sync>;

/// ウィンドウに結びつける状態 (`GWLP_USERDATA` に置く。ウィンドウはプロセスの終わりまで残すので解放しない)。
struct Ctx {
    cb: Callback,
    /// `ShutdownBlockReasonCreate` に渡す理由の文言 (NUL 終端の UTF-16)。
    reason: Vec<u16>,
}

/// 起動した受信の手がかり。ウィンドウは専用スレッドが作るので、作り終えるまで `hwnd` は空。
pub struct SessionWatcher {
    hwnd: Arc<OnceLock<usize>>,
}

impl SessionWatcher {
    /// ウィンドウを作り終えるまで最大 `timeout` 待つ (試験用。起動経路では待たない)。作れていれば真。
    pub fn wait_ready(&self, timeout: Duration) -> bool {
        let end = Instant::now() + timeout;
        while self.hwnd.get().is_none() {
            if Instant::now() >= end {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        true
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: `GWLP_USERDATA` には `start` が `Box::into_raw` した `Ctx` だけを置く (置く前は 0)
    let ctx = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const Ctx;
    if !ctx.is_null() {
        // SAFETY: 上のとおり。`Ctx` は解放しないので、ウィンドウがある間は有効
        let ctx = unsafe { &*ctx };
        match msg {
            WM_QUERYENDSESSION => {
                // SAFETY: 自分のウィンドウと NUL 終端の文字列
                unsafe { ShutdownBlockReasonCreate(hwnd, ctx.reason.as_ptr()) };
                // 退避は別のスレッドで始めるだけ。終了を妨げない (完了は `WM_ENDSESSION` で待つ)。
                // スレッドを作れなければ、ここで呼ぶ (`WM_ENDSESSION` の側でも退避を書くので、遅れるだけ)
                let cb = Arc::clone(&ctx.cb);
                // 使い捨てのスレッド (02 §9.1 の表: セッション終了通知)
                #[allow(clippy::disallowed_methods)]
                let spawned = std::thread::Builder::new()
                    .name("eve-session-query".into())
                    .spawn(move || cb(SessionEvent::QueryEnd));
                if spawned.is_err() {
                    (ctx.cb)(SessionEvent::QueryEnd);
                }
                return 1;
            }
            WM_ENDSESSION => {
                (ctx.cb)(SessionEvent::End {
                    ending: wparam != 0,
                });
                // SAFETY: 自分のウィンドウ
                unsafe { ShutdownBlockReasonDestroy(hwnd) };
                return 0;
            }
            _ => {}
        }
    }
    // SAFETY: 既定の処理に渡す
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// 専用スレッドを立て、そこで非表示のトップレベルウィンドウを作ってメッセージループを回す。スレッドを立てるだけで戻る
/// (起動経路をブロックしない、02 §9.5)。`reason` は終了の画面に出る理由の文言、`cb` は受けた通知 (`QueryEnd` は
/// 使い捨てのスレッドで、`End` はメッセージループのスレッドで呼ぶ。`End` から戻るとプロセスが終了されうる)。
pub fn start(
    reason: &str,
    cb: impl Fn(SessionEvent) + Send + Sync + 'static,
) -> io::Result<SessionWatcher> {
    let hwnd_slot = Arc::new(OnceLock::new());
    let slot = Arc::clone(&hwnd_slot);
    let ctx = Box::new(Ctx {
        cb: Arc::new(cb),
        reason: reason.encode_utf16().chain([0]).collect(),
    });
    // 専用スレッド (02 §9.1 の表: セッション終了通知)
    #[allow(clippy::disallowed_methods)]
    std::thread::Builder::new()
        .name("eve-session".into())
        .spawn(move || {
            let class: Vec<u16> = "evoeditor.session\0".encode_utf16().collect();
            // SAFETY: クラスの登録とウィンドウの作成。文字列は NUL 終端で、この関数の間は有効
            let hwnd = unsafe {
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
                // 2 回目 (試験で複数作る) は登録済みで失敗するが、同じクラスを使えばよい
                RegisterClassW(&wc);
                // `WS_VISIBLE` を付けない通常のウィンドウ (親なし = トップレベル。`HWND_MESSAGE` ではない)
                CreateWindowExW(
                    0,
                    class.as_ptr(),
                    class.as_ptr(),
                    0,
                    0,
                    0,
                    0,
                    0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                )
            };
            if hwnd.is_null() {
                return;
            }
            // SAFETY: 自分のウィンドウに、解放しない `Ctx` の指す先を置く
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(ctx) as isize) };
            let _ = slot.set(hwnd as usize);
            // SAFETY: このスレッドのメッセージループ。`MSG` はゼロ初期化で足りる
            unsafe {
                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        })?;
    Ok(SessionWatcher { hwnd: hwnd_slot })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use windows_sys::Win32::UI::WindowsAndMessaging::{ENDSESSION_LOGOFF, SendMessageW};

    /// ウィンドウに通知を送ったときに、コールバックへ順に届き、`WM_QUERYENDSESSION` は終了を妨げない (1 を返す)。
    /// OS からの実際のログオフ・シャットダウンでの到達は S-11 (実機) で確かめる。
    // verifies: NFR-302
    #[test]
    fn 隠しウィンドウはセッション終了の通知をコールバックへ渡す() {
        let got = Arc::new(Mutex::new(Vec::new()));
        let g = Arc::clone(&got);
        let w = start("test", move |ev| g.lock().unwrap().push(ev)).unwrap();
        assert!(w.wait_ready(Duration::from_secs(5)));
        let hwnd = *w.hwnd.get().unwrap() as HWND;
        // SAFETY: 作ったウィンドウへの同期の送信 (ウィンドウのスレッドが処理して戻る)
        let r = unsafe { SendMessageW(hwnd, WM_QUERYENDSESSION, 0, ENDSESSION_LOGOFF as LPARAM) };
        assert_eq!(r, 1);
        // SAFETY: 同上
        unsafe { SendMessageW(hwnd, WM_ENDSESSION, 0, 0) };
        // SAFETY: 同上
        unsafe { SendMessageW(hwnd, WM_ENDSESSION, 1, ENDSESSION_LOGOFF as LPARAM) };
        // 問い合わせは別スレッドで届くので、3 つそろうまで待つ
        let end = Instant::now() + Duration::from_secs(5);
        while got.lock().unwrap().len() < 3 && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(5));
        }
        let got = got.lock().unwrap().clone();
        assert!(got.contains(&SessionEvent::QueryEnd), "{got:?}");
        let ends: Vec<_> = got
            .iter()
            .filter(|e| **e != SessionEvent::QueryEnd)
            .copied()
            .collect();
        assert_eq!(
            ends,
            [
                SessionEvent::End { ending: false },
                SessionEvent::End { ending: true },
            ]
        );
    }
}
