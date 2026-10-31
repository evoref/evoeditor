//! 緊急の端末復元の Unix 実装 (docs/05 §11.1 / §11.2「端末の緊急復元」、02 §9.6 手順 4、04 §14.3)。Windows の `term` と同じ
//! 呼び方 (`emergency_restore(handle, bytes, saved_mode, restored)`) で、`handle` は描画先の端末の fd、`saved_mode` は
//! 起動時の termios (`save_mode` で取った値)。
//!
//! - `emergency_restore`: メインが止まったときにシグナルスレッドが呼ぶ。書込は **使い捨てのスレッド** で行い (端末が止まって
//!   いても戻れるように)、呼び側は最大 `WAIT` だけ待つ。SIGHUP では端末が既に無いので、書込の失敗は無視する。
//! - `install_crash_restore`: SIGSEGV / SIGBUS / SIGABRT / SIGILL / SIGFPE のハンドラで **端末の復元だけ** を行い (`write(2)` と
//!   `tcsetattr`。メモリの確保もロックの取得もしない)、元のハンドラに戻して返る。障害の命令がもう一度起きて元の動作
//!   (Rust のスタックあふれの表示・コアダンプ) になる。緊急退避はしない (ヒープが壊れているかもしれない。05 §11.1)。
//!
//! どちらも frontend の `restore` と「復元済み」のフラグを共有し、`false → true` に立てた側だけが復元する。

#![allow(unsafe_code)]

use std::io;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::Duration;

/// 復元の書込を待つ上限 (02 §11.2.1 の表の 4)。
const WAIT: Duration = Duration::from_millis(200);

/// `fd` の端末の今の termios を、`emergency_restore` / `install_crash_restore` に渡す形 (バイト列) で返す。端末でなければ `None`。
pub fn save_mode(fd: i32) -> Option<Vec<u8>> {
    // SAFETY: ゼロ初期化した termios (整数と配列だけ) に書かせる。
    let mut t: libc::termios = unsafe { std::mem::zeroed() };
    // SAFETY: 有効な書込先。fd が端末でなければ失敗を返すだけ。
    if unsafe { libc::tcgetattr(fd, &mut t) } != 0 {
        return None;
    }
    // SAFETY: termios は詰め物も含めて初期化済み (zeroed の上に書かれた) の平文の構造体。
    let b = unsafe {
        std::slice::from_raw_parts(
            std::ptr::from_ref(&t).cast::<u8>(),
            size_of::<libc::termios>(),
        )
    };
    Some(b.to_vec())
}

/// `save_mode` の値を termios に戻す。大きさが違えば `None` (モードは戻さず、バイト列だけを書く)。
fn parse_mode(b: &[u8]) -> Option<libc::termios> {
    if b.len() != size_of::<libc::termios>() {
        return None;
    }
    // SAFETY: 大きさが一致し、termios はどのビットの並びでも有効な平文の構造体。
    Some(unsafe { std::ptr::read_unaligned(b.as_ptr().cast::<libc::termios>()) })
}

/// `fd` に `bytes` を書き (EINTR は繰り返し、他の失敗は諦める)、termios を戻す。非同期シグナル安全な呼出しだけを使う。
fn restore_now(fd: i32, bytes: &[u8], mode: Option<&libc::termios>) {
    let mut rest = bytes;
    while !rest.is_empty() {
        // SAFETY: `rest` は有効な領域。fd が無効なら失敗を返すだけ。
        let n = unsafe { libc::write(fd, rest.as_ptr().cast(), rest.len()) };
        if n > 0 {
            rest = &rest[n as usize..];
        } else if n < 0 && io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
            continue;
        } else {
            break;
        }
    }
    if let Some(m) = mode {
        // SAFETY: 有効な termios。fd が端末でなければ失敗を返すだけ。
        unsafe { libc::tcsetattr(fd, libc::TCSANOW, m) };
    }
}

/// 端末を緊急に復元する。復元の担当になった (フラグを立てた) ときは `true`。他の経路が復元済みなら何もせず `false`。
pub fn emergency_restore(
    handle: usize,
    bytes: &'static [u8],
    saved_mode: &[u8],
    restored: &AtomicBool,
) -> bool {
    if restored.swap(true, Ordering::AcqRel) {
        return false;
    }
    let fd = i32::try_from(handle).unwrap_or(-1);
    let mode = parse_mode(saved_mode);
    let (tx, rx) = mpsc::channel::<()>();
    // 使い捨てのスレッド (02 §9.1 の表: 緊急の端末復元)。止まっても、呼び側は WAIT で先へ進む
    #[allow(clippy::disallowed_methods)]
    let spawned = std::thread::Builder::new()
        .name("eve-term-restore".to_owned())
        .spawn(move || {
            restore_now(fd, bytes, mode.as_ref());
            let _ = tx.send(());
        });
    if spawned.is_ok() {
        let _ = rx.recv_timeout(WAIT);
    }
    true
}

// ---------- 異常終了のシグナル ----------

/// 端末の復元だけを行うシグナル。
const CRASH_SIGNALS: [i32; 5] = [
    libc::SIGSEGV,
    libc::SIGBUS,
    libc::SIGABRT,
    libc::SIGILL,
    libc::SIGFPE,
];

struct Crash {
    bytes: &'static [u8],
    mode: Option<libc::termios>,
    restored: Arc<AtomicBool>,
    /// 付け替える前のハンドラ (`CRASH_SIGNALS` と同じ順)。
    previous: [libc::sigaction; CRASH_SIGNALS.len()],
}

static CRASH: OnceLock<Crash> = OnceLock::new();
static CRASH_FD: AtomicI32 = AtomicI32::new(-1);

extern "C" fn on_crash(signo: i32, _: *mut libc::siginfo_t, _: *mut libc::c_void) {
    // ハンドラの中: 初期化済みの値を読むだけ (確保もロックもしない)
    if let Some(c) = CRASH.get() {
        if !c.restored.swap(true, Ordering::AcqRel) {
            restore_now(CRASH_FD.load(Ordering::Acquire), c.bytes, c.mode.as_ref());
        }
        if let Some(i) = CRASH_SIGNALS.iter().position(|&s| s == signo) {
            // SAFETY: 起動時に保存した元のハンドラに戻す。戻った後、同じ障害がもう一度起きて元の動作になる
            unsafe { libc::sigaction(signo, &c.previous[i], std::ptr::null_mut()) };
            return;
        }
    }
    // SAFETY: 想定外 (未登録): 既定の動作に戻す。
    unsafe { libc::signal(signo, libc::SIG_DFL) };
}

/// 異常終了のシグナルで端末を戻すハンドラを付ける (プロセスで 1 回。2 回目以降は何もしない)。`fd`・`bytes`・`saved_mode`・
/// `restored` は `emergency_restore` と同じ。
pub fn install_crash_restore(
    fd: i32,
    bytes: &'static [u8],
    saved_mode: &[u8],
    restored: Arc<AtomicBool>,
) -> io::Result<()> {
    if CRASH.get().is_some() {
        return Ok(());
    }
    // SAFETY: 元のハンドラの読み出し (書込先はゼロ初期化した sigaction)。
    let mut previous: [libc::sigaction; CRASH_SIGNALS.len()] = unsafe { std::mem::zeroed() };
    for (i, &s) in CRASH_SIGNALS.iter().enumerate() {
        // SAFETY: 有効な書込先。
        if unsafe { libc::sigaction(s, std::ptr::null(), &mut previous[i]) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    CRASH_FD.store(fd, Ordering::Release);
    let _ = CRASH.set(Crash {
        bytes,
        mode: parse_mode(saved_mode),
        restored,
        previous,
    });
    for &s in &CRASH_SIGNALS {
        // SAFETY: ゼロ初期化した sigaction に、ハンドラ・フラグ・空のマスクを入れて付ける。SA_ONSTACK: スタックあふれでも
        // Rust の実行時が用意した代替スタックで動く
        unsafe {
            let mut act: libc::sigaction = std::mem::zeroed();
            act.sa_sigaction =
                on_crash as extern "C" fn(i32, *mut libc::siginfo_t, *mut libc::c_void) as usize;
            act.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
            libc::sigemptyset(&mut act.sa_mask);
            if libc::sigaction(s, &act, std::ptr::null_mut()) != 0 {
                return Err(io::Error::last_os_error());
            }
        }
    }
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    /// 擬似端末 (主, 従) の fd。
    fn pty() -> (i32, i32) {
        let (mut m, mut s) = (-1, -1);
        // SAFETY: 有効な書込先 2 つ。名前・termios・大きさは使わない (null)。
        let r = unsafe {
            libc::openpty(
                &mut m,
                &mut s,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        assert_eq!(r, 0, "openpty");
        (m, s)
    }

    fn read_all(fd: i32) -> Vec<u8> {
        // SAFETY: 主の側を非ブロックにしてから読めるだけ読む。
        unsafe {
            let fl = libc::fcntl(fd, libc::F_GETFL);
            libc::fcntl(fd, libc::F_SETFL, fl | libc::O_NONBLOCK);
        }
        let mut out = Vec::new();
        let mut buf = [0u8; 256];
        for _ in 0..50 {
            // SAFETY: 有効な書込先。
            let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
            if n > 0 {
                out.extend_from_slice(&buf[..n as usize]);
            } else {
                std::thread::sleep(Duration::from_millis(10));
                if !out.is_empty() {
                    break;
                }
            }
        }
        out
    }

    fn echo_on(fd: i32) -> bool {
        let t = parse_mode(&save_mode(fd).unwrap()).unwrap();
        t.c_lflag & libc::ECHO != 0
    }

    fn set_raw(fd: i32) {
        let mut t = parse_mode(&save_mode(fd).unwrap()).unwrap();
        // SAFETY: 有効な termios。
        unsafe {
            libc::cfmakeraw(&mut t);
            libc::tcsetattr(fd, libc::TCSANOW, &t);
        }
    }

    #[test]
    fn 起動時のモードに戻し_復元の列を書く_二度目は何もしない() {
        let (m, s) = pty();
        let saved = save_mode(s).expect("端末");
        assert!(echo_on(s));
        set_raw(s);
        assert!(!echo_on(s));
        let flag = AtomicBool::new(false);
        assert!(emergency_restore(
            s as usize,
            b"\x18\x1b[?1049l",
            &saved,
            &flag
        ));
        assert!(echo_on(s), "termios を戻す");
        assert!(flag.load(Ordering::Acquire));
        let out = read_all(m);
        assert!(out.ends_with(b"\x1b[?1049l"), "{out:?}");
        set_raw(s);
        assert!(
            !emergency_restore(s as usize, b"x", &saved, &flag),
            "復元済み"
        );
        assert!(!echo_on(s));
        assert!(save_mode(-1).is_none());
        // SAFETY: 開いた fd を閉じる。
        unsafe {
            libc::close(m);
            libc::close(s);
        }
    }

    #[test]
    fn 端末が無くても止まらない() {
        let flag = AtomicBool::new(false);
        let t = std::time::Instant::now();
        assert!(emergency_restore(usize::MAX, b"x", &[], &flag));
        assert!(t.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn 異常終了のシグナルで端末を戻してから元の動作で終わる() {
        let (m, s) = pty();
        let saved = save_mode(s).unwrap();
        // SAFETY: 子は端末の設定・ハンドラの付け替え・abort だけを行う。
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0);
        if pid == 0 {
            set_raw(s);
            let _ =
                install_crash_restore(s, b"\x18RESTORED", &saved, Arc::new(AtomicBool::new(false)));
            std::process::abort();
        }
        let mut status = 0;
        // SAFETY: 自分の子を待つ。
        assert_eq!(unsafe { libc::waitpid(pid, &mut status, 0) }, pid);
        assert!(libc::WIFSIGNALED(status), "{status:#x}");
        assert_eq!(
            libc::WTERMSIG(status),
            libc::SIGABRT,
            "元の動作 (既定) で終わる"
        );
        assert!(echo_on(s), "子が termios を戻した");
        let out = read_all(m);
        assert!(out.windows(8).any(|w| w == b"RESTORED"), "{out:?}");
        // SAFETY: 開いた fd を閉じる。
        unsafe {
            libc::close(m);
            libc::close(s);
        }
    }
}
