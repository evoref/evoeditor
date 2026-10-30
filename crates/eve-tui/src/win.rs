//! Windows のコンソール入力 (docs/04 §6.1、05 §9.1)。自前の `WaitForSingleObject` + `ReadConsoleInputW` のループ
//! (確定 (ユーザー回答)、OQ-33)。crossterm の `event::read()` は使わない。OS を呼ぶのはこのファイルだけ (R5)。
//!
//! 正規化とバースト検出は `input::Normalizer` (OS を呼ばない)。ここは読む・待つ・送るだけを行う (入力スレッドの責務の上限)。
//! 実測 (S-07) に基づき、**1 回の `ReadConsoleInputW` は `READ_CHUNK` 件まで** にする (10 MB の貼り付けで 1 回の
//! `GetNumberOfConsoleInputEvents` が約 1,400 万件を返した。全体を 1 本の配列に読まない)。

#![allow(unsafe_code)]

use std::io;
use std::thread::{self, JoinHandle};

use eve_view::{CellSize, InputEvent};
use windows_sys::Win32::Foundation::{
    GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Console::{
    CONSOLE_SCREEN_BUFFER_INFO, DISABLE_NEWLINE_AUTO_RETURN, ENABLE_ECHO_INPUT,
    ENABLE_EXTENDED_FLAGS, ENABLE_LINE_INPUT, ENABLE_MOUSE_INPUT, ENABLE_PROCESSED_INPUT,
    ENABLE_QUICK_EDIT_MODE, ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
    ENABLE_WINDOW_INPUT, FOCUS_EVENT, GetConsoleMode, GetConsoleScreenBufferInfo, GetConsoleTitleW,
    GetNumberOfConsoleInputEvents, INPUT_RECORD, KEY_EVENT, MOUSE_EVENT, ReadConsoleInputW,
    SetConsoleMode, SetConsoleTitleW, WINDOW_BUFFER_SIZE_EVENT, WriteConsoleW,
};
use windows_sys::Win32::System::Threading::{INFINITE, WaitForSingleObject};

use crate::buffer::surface_rows_for_window;
use crate::input::{BURST_T_MS, KeyRec, Normalizer, Out, Raw};
use crate::mouse::{MouseRec, MouseTracker, SharedGeom};

/// 1 回の `ReadConsoleInputW` で読む記録の上限。
pub const READ_CHUNK: usize = 4096;

/// 入力の送り先 (本番は `eve_host::InputSink`、テストや診断は別の実装)。
pub trait InputOut: Send + 'static {
    /// `InputEvent` を送る。受け手が閉じていれば `false`。
    fn event(&self, ev: InputEvent) -> bool;
    /// 入力スレッドが続けられなくなった (入力ハンドルの失敗)。
    fn died(&self);
}

/// コンソールのハンドル (`Send` にするための包み。ハンドルは OS が管理するスレッド間で使える値)。
struct Handle(HANDLE);

// SAFETY: コンソールのハンドルは単なるカーネルオブジェクトの識別子で、複数スレッドから使ってよい。
unsafe impl Send for Handle {}

/// コンソールの入力。`open` でモードを入力向けに設定し、`restore` (または drop) で元に戻す。
pub struct ConsoleInput {
    input: Handle,
    output: Handle,
    saved_mode: u32,
    restored: bool,
}

fn open_console(name: &str) -> io::Result<HANDLE> {
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    // SAFETY: `wide` は NUL 終端の UTF-16。残りの引数は `CreateFileW` の仕様どおり (セキュリティ属性・テンプレートなし)。
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
    if h == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    Ok(h)
}

impl ConsoleInput {
    /// `CONIN$` / `CONOUT$` を開き、入力モードを設定する (05 §9.1 の入力モード)。
    /// `ENABLE_PROCESSED_INPUT` / `ENABLE_LINE_INPUT` / `ENABLE_ECHO_INPUT` / `ENABLE_VIRTUAL_TERMINAL_INPUT` /
    /// `ENABLE_MOUSE_INPUT` を落とし、`ENABLE_WINDOW_INPUT` を立てる。簡易編集モードは変えない (v0.1)。
    pub fn open() -> io::Result<ConsoleInput> {
        let input = open_console("CONIN$")?;
        let output = open_console("CONOUT$")?;
        let mut saved = 0u32;
        // SAFETY: `input` は開いたコンソール入力のハンドル、`saved` は書込先の有効な u32。
        if unsafe { GetConsoleMode(input, &mut saved) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut mode = saved
            & !(ENABLE_PROCESSED_INPUT
                | ENABLE_LINE_INPUT
                | ENABLE_ECHO_INPUT
                | ENABLE_VIRTUAL_TERMINAL_INPUT
                | ENABLE_MOUSE_INPUT);
        mode |= ENABLE_WINDOW_INPUT;
        if mode & ENABLE_QUICK_EDIT_MODE != 0 {
            // 簡易編集モードを保つには拡張フラグも立てる必要がある (SetConsoleMode の仕様)
            mode |= ENABLE_EXTENDED_FLAGS;
        }
        // SAFETY: `input` は有効なコンソール入力のハンドル。
        if unsafe { SetConsoleMode(input, mode) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(ConsoleInput {
            input: Handle(input),
            output: Handle(output),
            saved_mode: saved,
            restored: false,
        })
    }

    /// 端末のウィンドウの大きさ (`srWindow` から。バッファの大きさではない、05 §9.1 の ConPTY の癖)。
    pub fn window_size(&self) -> io::Result<CellSize> {
        window_size(self.output.0)
    }

    /// マウスの入力を有効 / 無効にする (04 §6.8)。有効の間は簡易編集モードを外す (立てたままだとマウスの記録が届かない)。
    /// 無効にしたら、簡易編集モードは起動時の状態に戻す。
    pub fn set_mouse(&self, on: bool) {
        let mut mode = 0u32;
        // SAFETY: `input` は有効なコンソール入力のハンドル、`mode` は書込先の有効な u32。
        if unsafe { GetConsoleMode(self.input.0, &mut mode) } == 0 {
            return;
        }
        if on {
            mode |= ENABLE_MOUSE_INPUT | ENABLE_EXTENDED_FLAGS;
            mode &= !ENABLE_QUICK_EDIT_MODE;
        } else {
            mode &= !ENABLE_MOUSE_INPUT;
            if self.saved_mode & ENABLE_QUICK_EDIT_MODE != 0 {
                mode |= ENABLE_QUICK_EDIT_MODE | ENABLE_EXTENDED_FLAGS;
            }
        }
        // SAFETY: `input` は有効なコンソール入力のハンドル。失敗しても入力は続けられるので無視する。
        unsafe { SetConsoleMode(self.input.0, mode) };
    }

    /// 起動時に保存した入力のモード (緊急の端末復元に渡す)。
    pub fn saved_mode(&self) -> u32 {
        self.saved_mode
    }

    /// 入力モードを元に戻す (どの終了経路でも呼ぶ。2 回目以降は何もしない)。
    pub fn restore(&mut self) {
        if !self.restored {
            self.restored = true;
            // SAFETY: `input` は `open` で得た有効なハンドル。失敗しても次の復元を試すためエラーは無視する。
            unsafe { SetConsoleMode(self.input.0, self.saved_mode) };
        }
    }
}

impl Drop for ConsoleInput {
    fn drop(&mut self) {
        self.restore();
    }
}

fn window_size(output: HANDLE) -> io::Result<CellSize> {
    // SAFETY: 全ビット 0 の `CONSOLE_SCREEN_BUFFER_INFO` は有効な値 (すべて整数)。
    let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
    // SAFETY: `output` は開いたコンソール出力のハンドル、`info` は書込先の有効な構造体。
    if unsafe { GetConsoleScreenBufferInfo(output, &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let w = i32::from(info.srWindow.Right) - i32::from(info.srWindow.Left) + 1;
    let h = i32::from(info.srWindow.Bottom) - i32::from(info.srWindow.Top) + 1;
    Ok(CellSize {
        cols: u16::try_from(w.max(0)).unwrap_or(0),
        rows: u16::try_from(h.max(0)).unwrap_or(0),
    })
}

/// 端末のウィンドウの大きさから、編集面の `Resize` を作る (クローム行を除く、04 §3.1)。
pub fn surface_resize(window: CellSize) -> InputEvent {
    InputEvent::Resize {
        cols: window.cols,
        rows: surface_rows_for_window(window.rows),
    }
}

enum Wait {
    Ready,
    Timeout,
}

fn wait(h: HANDLE, ms: u32) -> io::Result<Wait> {
    // SAFETY: `h` は有効なコンソール入力のハンドル (待つだけ)。
    match unsafe { WaitForSingleObject(h, ms) } {
        WAIT_OBJECT_0 => Ok(Wait::Ready),
        WAIT_TIMEOUT => Ok(Wait::Timeout),
        _ => Err(io::Error::last_os_error()),
    }
}

fn pending(h: HANDLE) -> io::Result<u32> {
    let mut n = 0u32;
    // SAFETY: `h` は有効なコンソール入力のハンドル、`n` は書込先の有効な u32。
    if unsafe { GetNumberOfConsoleInputEvents(h, &mut n) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(n)
}

/// 溜まっている記録を読む (最大 `READ_CHUNK` 件)。件数 0 のときは呼ばない (`ReadConsoleInputW` はブロックする)。
fn read_chunk(
    h: HANDLE,
    buf: &mut [INPUT_RECORD],
    want: u32,
    out: &mut Vec<Raw>,
    mouse: &mut Vec<MouseRec>,
) -> io::Result<()> {
    let n = want.min(buf.len() as u32);
    let mut got = 0u32;
    // SAFETY: `buf` は `n` 件以上の有効な領域、`got` は書込先の有効な u32。
    if unsafe { ReadConsoleInputW(h, buf.as_mut_ptr(), n, &mut got) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // 計測用ビルド: 入力のバイトを読み終えた時刻 (01 §3.0.1 の (b) の始点)。文字を押した記録があるときだけ
    if buf[..got as usize]
        .iter()
        .any(|r| u32::from(r.EventType) == WINDOW_BUFFER_SIZE_EVENT)
    {
        crate::trace::resize_read();
    }
    crate::trace::input_read(buf[..got as usize].iter().find_map(|r| {
        if u32::from(r.EventType) != KEY_EVENT {
            return None;
        }
        // SAFETY: `EventType` が `KEY_EVENT` のとき、共用体は `KeyEvent` が有効。
        let k = unsafe { r.Event.KeyEvent };
        (k.bKeyDown != 0).then_some(k.wVirtualKeyCode)
    }));
    for r in &buf[..got as usize] {
        out.push(match u32::from(r.EventType) {
            KEY_EVENT => {
                // SAFETY: `EventType` が `KEY_EVENT` のとき、共用体は `KeyEvent` が有効。
                let k = unsafe { r.Event.KeyEvent };
                Raw::Key(KeyRec {
                    down: k.bKeyDown != 0,
                    repeat: k.wRepeatCount,
                    vk: k.wVirtualKeyCode,
                    // SAFETY: `uChar` は `UnicodeChar` として読んでよい (ReadConsoleInputW は W 版)。
                    ch: unsafe { k.uChar.UnicodeChar },
                    state: k.dwControlKeyState,
                })
            }
            WINDOW_BUFFER_SIZE_EVENT => Raw::Resize,
            MOUSE_EVENT => {
                // SAFETY: `EventType` が `MOUSE_EVENT` のとき、共用体は `MouseEvent` が有効。
                let m = unsafe { r.Event.MouseEvent };
                mouse.push(MouseRec {
                    // 画面バッファの座標。ウィンドウの左上を原点にするのは送る側 (`send_mouse`)
                    col: i32::from(m.dwMousePosition.X),
                    row: i32::from(m.dwMousePosition.Y),
                    buttons: m.dwButtonState,
                    state: m.dwControlKeyState,
                    flags: m.dwEventFlags,
                });
                Raw::Ignored
            }
            FOCUS_EVENT => {
                // SAFETY: `EventType` が `FOCUS_EVENT` のとき、共用体は `FocusEvent` が有効。
                let f = unsafe { r.Event.FocusEvent };
                Raw::Focus(f.bSetFocus != 0)
            }
            // メニューは捨てる (04 §6.2)
            _ => Raw::Ignored,
        });
    }
    Ok(())
}

fn send(out: &dyn InputOut, outs: Vec<Out>, output: HANDLE) -> bool {
    for o in outs {
        let ev = match o {
            Out::Event(e) => e,
            Out::Resize => match window_size(output) {
                Ok(s) => surface_resize(s),
                Err(_) => continue,
            },
        };
        if !out.event(ev) {
            return false;
        }
    }
    true
}

/// マウスの記録を、ウィンドウの左上を原点とするセルにして送る (スクロールしたバッファでも合うように `srWindow` を引く)。
fn send_mouse(
    out: &dyn InputOut,
    recs: &mut Vec<MouseRec>,
    tracker: &mut MouseTracker,
    geom: &SharedGeom,
    output: HANDLE,
) -> bool {
    if recs.is_empty() {
        return true;
    }
    // SAFETY: 全ビット 0 の `CONSOLE_SCREEN_BUFFER_INFO` は有効な値 (すべて整数)。
    let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
    // SAFETY: `output` は開いたコンソール出力のハンドル、`info` は書込先の有効な構造体。
    let (left, top) = if unsafe { GetConsoleScreenBufferInfo(output, &mut info) } == 0 {
        (0, 0)
    } else {
        (i32::from(info.srWindow.Left), i32::from(info.srWindow.Top))
    };
    for mut r in recs.drain(..) {
        r.col -= left;
        r.row -= top;
        for m in tracker.feed(r, geom) {
            if !out.event(InputEvent::Mouse(m)) {
                return false;
            }
        }
    }
    true
}

/// 入力スレッドの本体。受け手が閉じるか入力ハンドルが失敗したら終わる。
fn run(input: Handle, output: Handle, out: &dyn InputOut, geom: &SharedGeom) {
    let (h, o) = (input.0, output.0);
    let mut norm = Normalizer::new();
    let mut buf: Vec<INPUT_RECORD> = vec![
        // SAFETY: 全ビット 0 の `INPUT_RECORD` は有効な値 (読む前の入れ物)。
        unsafe { std::mem::zeroed() };
        READ_CHUNK
    ];
    let mut raws: Vec<Raw> = Vec::with_capacity(READ_CHUNK);
    let mut mouse_recs: Vec<MouseRec> = Vec::new();
    let mut tracker = MouseTracker::new();
    loop {
        // バースト中は T ms だけ待つ。タイムアウトしたらバーストを閉じて送る
        let ms = if norm.in_burst() {
            BURST_T_MS
        } else {
            INFINITE
        };
        match wait(h, ms) {
            Ok(Wait::Ready) => {}
            Ok(Wait::Timeout) => {
                if !send(out, norm.close_burst(), o) {
                    return;
                }
                continue;
            }
            Err(_) => {
                out.died();
                return;
            }
        }
        // 待ちなしで読める分をすべて読み切る (1 回の読取)。1 回に読む件数には上限を付ける
        loop {
            let n = match pending(h) {
                Ok(n) => n,
                Err(_) => {
                    out.died();
                    return;
                }
            };
            if n == 0 {
                break;
            }
            raws.clear();
            if read_chunk(h, &mut buf, n, &mut raws, &mut mouse_recs).is_err() {
                out.died();
                return;
            }
            if !send(out, norm.feed(&raws), o) {
                return;
            }
            if !send_mouse(out, &mut mouse_recs, &mut tracker, geom, o) {
                return;
            }
        }
    }
}

/// 入力スレッドを起動する。コンソールのモードは `ConsoleInput` が持ち、復元は呼び出し側 (`ConsoleInput::restore`)。
// 入力スレッドは 02 §9.1 の表のスレッド (R11 の例外。deps-policy.toml の `allowed_files` に載せている)。
#[allow(clippy::disallowed_methods)]
pub fn spawn_input_thread(
    input: &ConsoleInput,
    out: impl InputOut,
    geom: std::sync::Arc<SharedGeom>,
) -> io::Result<JoinHandle<()>> {
    let (i, o) = (Handle(input.input.0), Handle(input.output.0));
    thread::Builder::new()
        .name("eve-input".to_owned())
        .spawn(move || run(i, o, &out, &geom))
}

/// コンソールの出力 (`CONOUT$`)。VT 出力を有効にし、UTF-16 に変換して `WriteConsoleW` で書く (05 §9.1。標準出力が
/// リダイレクトされていても描画先は常にコンソール)。
pub struct ConsoleOut {
    handle: Handle,
    saved_mode: u32,
    saved_title: Option<Vec<u16>>,
    restored: bool,
}

impl ConsoleOut {
    /// `CONOUT$` を開き、`ENABLE_VIRTUAL_TERMINAL_PROCESSING | DISABLE_NEWLINE_AUTO_RETURN` を設定する。VT を拒否する
    /// コンソールは `Err` (起動エラー、`stderr.unsupported_console`)。元のモードと端末タイトルを保存する。
    pub fn open() -> io::Result<ConsoleOut> {
        let h = open_console("CONOUT$")?;
        let mut saved = 0u32;
        // SAFETY: `h` は開いたコンソール出力のハンドル、`saved` は書込先の有効な u32。
        if unsafe { GetConsoleMode(h, &mut saved) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let mode = saved | ENABLE_VIRTUAL_TERMINAL_PROCESSING | DISABLE_NEWLINE_AUTO_RETURN;
        // SAFETY: `h` は有効なコンソール出力のハンドル。
        if unsafe { SetConsoleMode(h, mode) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buf = vec![0u16; 4096];
        // SAFETY: `buf` は `buf.len()` 個の u16 の有効な領域。
        let n = unsafe { GetConsoleTitleW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
        let title = (n > 0 && n < buf.len()).then(|| buf[..n].to_vec());
        Ok(ConsoleOut {
            handle: Handle(h),
            saved_mode: saved,
            saved_title: title,
            restored: false,
        })
    }

    /// 文字列を書く (UTF-16 に変換し、サロゲートペアを割らずに分けて書く)。
    pub fn write_str(&mut self, s: &str) -> io::Result<()> {
        const MAX_UNITS: usize = 16 * 1024;
        let units: Vec<u16> = s.encode_utf16().collect();
        let mut rest: &[u16] = &units;
        while !rest.is_empty() {
            let mut n = rest.len().min(MAX_UNITS);
            // 上位サロゲートで切らない
            if n < rest.len() && (0xD800..0xDC00).contains(&rest[n - 1]) {
                n -= 1;
            }
            let mut written = 0u32;
            // SAFETY: `rest[..n]` は有効な u16 の領域、`written` は書込先の有効な u32。
            let ok = unsafe {
                WriteConsoleW(
                    self.handle.0,
                    rest.as_ptr().cast(),
                    n as u32,
                    &mut written,
                    std::ptr::null(),
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            rest = &rest[(written as usize).min(rest.len()).max(1)..];
        }
        Ok(())
    }

    /// 端末のウィンドウの大きさ (`srWindow`)。
    pub fn window_size(&self) -> io::Result<CellSize> {
        window_size(self.handle.0)
    }

    /// 描画先の生ハンドル (緊急の端末復元に渡す。ポインタ ⇔ 整数の変換はここだけ)。
    pub fn raw_handle(&self) -> usize {
        self.handle.0 as usize
    }

    /// 起動時に保存した出力のモード (緊急の端末復元に渡す)。
    pub fn saved_mode(&self) -> u32 {
        self.saved_mode
    }

    /// 退避しておいた端末タイトルとコンソールモードを戻す (冪等。失敗しても続ける)。
    pub fn restore(&mut self) {
        if self.restored {
            return;
        }
        self.restored = true;
        if let Some(t) = &self.saved_title {
            let mut z = t.clone();
            z.push(0);
            // SAFETY: `z` は NUL 終端の UTF-16。
            unsafe { SetConsoleTitleW(z.as_ptr()) };
        }
        // SAFETY: `handle` は `open` で得た有効なハンドル。
        unsafe { SetConsoleMode(self.handle.0, self.saved_mode) };
    }
}

impl Drop for ConsoleOut {
    fn drop(&mut self) {
        self.restore();
    }
}
