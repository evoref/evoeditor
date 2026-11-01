//! Unix の端末の入出力 (docs/04 §6.1 / §14、05 §9 / §11.1。v0.2 の M7)。`win` と同じ形の部品 (`ConsoleInput`・`ConsoleOut`・
//! `spawn_input_thread`) で、`frontend` は OS に依らずこれらを呼ぶ。OS を呼ぶのはこのファイルだけ (R5)。
//!
//! - 描画先と入力は **`/dev/tty`** (標準入出力がリダイレクトされていても端末に描く。05 §9.1)。出力と入力で別々に開く
//!   (入力の側だけを非ブロックにするため)。
//! - raw モード: termios を `cfmakeraw` 相当にする (ISIG / IXON / ICRNL などを落とす。Ctrl+C / Z / S / Q がキーとして届く、
//!   FR-609)。起動時の termios を保存し、`restore` と緊急の端末復元 (`eve_platform::term`) で戻す。
//! - 入力スレッド: 端末と、SIGWINCH / SIGCONT の自己パイプを `poll` で待つ。端末が読めたら **待ちなしで読める分を読み切って**
//!   (1 回の読取) `VtParser::feed` に渡す。途切れたシーケンスを持ち越したまま入力が途絶えたら `ESC_WAIT` で `flush` する。
//!   SIGWINCH は `Resize`。SIGCONT (外から止められて `fg` で再開した) は「端末の再初期化が要る」印を立てて `Resize` を送る
//!   (次の描画で raw モード・代替スクリーン・各モードを入れ直して全面を描く)。
//! - サスペンド (`app.suspend`、05 §11.3): frontend が端末を復元してから `stop_self` で止まる。再開は上の SIGCONT と同じ経路。

#![allow(unsafe_code)]

use std::io;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};

use eve_view::{CellSize, InputEvent};

use crate::buffer::surface_rows_for_window;
use crate::mouse::{MouseTracker, SharedGeom};
use crate::vt_input::{VtOut, VtParser};

/// 1 回の `read` の大きさ。
const READ_CHUNK: usize = 64 * 1024;
/// 途切れたシーケンスを持ち越したまま入力が途絶えたときの待ち (ms)。
const ESC_WAIT_MS: i32 = 50;

/// マウスの報告 (押下・解放 + ボタン押下中の移動 + SGR 座標。1003 は使わない、04 §6.8)。
const MOUSE_ON: &str = "\x1b[?1000h\x1b[?1002h\x1b[?1006h";
const MOUSE_OFF: &str = "\x1b[?1006l\x1b[?1002l\x1b[?1000l";
/// 初期化で入れるモード (ENTER の後): ブラケットペースト (04 §6.6)・フォーカス報告 (§6.9)・kitty キーボードの
/// DISAMBIGUATE の push (§6.3)。
pub const ENTER_MODES: &str = "\x1b[?2004h\x1b[?1004h\x1b[>1u";
/// 復元で外すモード (LEAVE の前): kitty の pop・フォーカス・ブラケットペースト・マウス。
pub const LEAVE_MODES: &str = "\x1b[<u\x1b[?1004l\x1b[?2004l\x1b[?1006l\x1b[?1002l\x1b[?1000l";

/// 入力の送り先 (本番は `eve_host::InputSink`)。
pub trait InputOut: Send + 'static {
    /// `InputEvent` を送る。受け手が閉じていれば `false`。
    fn event(&self, ev: InputEvent) -> bool;
    /// 入力スレッドが続けられなくなった。
    fn died(&self);
}

fn open_tty(nonblock: bool) -> io::Result<i32> {
    let mut flags = libc::O_RDWR | libc::O_CLOEXEC | libc::O_NOCTTY;
    if nonblock {
        flags |= libc::O_NONBLOCK;
    }
    // SAFETY: NUL 終端の固定のパス。
    let fd = unsafe { libc::open(c"/dev/tty".as_ptr(), flags) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(fd)
}

fn termios_of(fd: i32) -> io::Result<libc::termios> {
    // SAFETY: ゼロ初期化した termios (整数と配列だけ) に書かせる。
    let mut t: libc::termios = unsafe { std::mem::zeroed() };
    // SAFETY: 有効な書込先。
    if unsafe { libc::tcgetattr(fd, &mut t) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(t)
}

fn set_termios(fd: i32, t: &libc::termios) -> io::Result<()> {
    // SAFETY: 有効な termios。
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, t) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn window_size(fd: i32) -> io::Result<CellSize> {
    // SAFETY: ゼロ初期化した winsize に書かせる。
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    // SAFETY: TIOCGWINSZ は winsize へ書く。
    if unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if ws.ws_col == 0 || ws.ws_row == 0 {
        return Err(io::Error::other("端末の大きさが 0"));
    }
    Ok(CellSize {
        cols: ws.ws_col,
        rows: ws.ws_row,
    })
}

fn write_all(fd: i32, mut b: &[u8]) -> io::Result<()> {
    while !b.is_empty() {
        // SAFETY: `b` は有効な領域。
        let n = unsafe { libc::write(fd, b.as_ptr().cast(), b.len()) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        b = &b[n as usize..];
    }
    Ok(())
}

/// 端末の入力。`open` で raw モードにし、`restore` (または drop) で起動時の termios に戻す。
pub struct ConsoleInput {
    fd: i32,
    saved: libc::termios,
    raw: libc::termios,
    restored: bool,
    /// SIGCONT を受けて、端末の再初期化 (`reinit`) が要る。入力スレッドが立て、描画が下ろす。
    reinit: Arc<AtomicBool>,
}

// SAFETY: fd と termios (平文の構造体) だけを持つ。
unsafe impl Send for ConsoleInput {}

impl ConsoleInput {
    pub fn open() -> io::Result<ConsoleInput> {
        let fd = open_tty(true)?;
        let saved = match termios_of(fd) {
            Ok(t) => t,
            Err(e) => {
                // SAFETY: 開いた fd を閉じる。
                unsafe { libc::close(fd) };
                return Err(e);
            }
        };
        let mut raw = saved;
        // SAFETY: 有効な termios を raw に書き換える。
        unsafe { libc::cfmakeraw(&mut raw) };
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;
        if let Err(e) = set_termios(fd, &raw) {
            // SAFETY: 開いた fd を閉じる。
            unsafe { libc::close(fd) };
            return Err(e);
        }
        Ok(ConsoleInput {
            fd,
            saved,
            raw,
            restored: false,
            reinit: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn window_size(&self) -> io::Result<CellSize> {
        window_size(self.fd)
    }

    /// マウスの報告を入れる・外す (04 §6.8)。
    pub fn set_mouse(&self, on: bool) {
        let _ = write_all(self.fd, if on { MOUSE_ON } else { MOUSE_OFF }.as_bytes());
    }

    /// 起動時の termios (緊急の端末復元に渡す。`eve_platform::term::save_mode` と同じ形のバイト列)。
    pub fn saved_mode(&self) -> Vec<u8> {
        // SAFETY: termios は平文の構造体 (tcgetattr が書いた値)。
        unsafe {
            std::slice::from_raw_parts(
                std::ptr::from_ref(&self.saved).cast::<u8>(),
                size_of::<libc::termios>(),
            )
        }
        .to_vec()
    }

    /// SIGCONT の後の再初期化が要るか (下ろさずに見る。`Frontend::wake_after` で描画を起こすため)。
    pub fn reinit_pending(&self) -> bool {
        self.reinit.load(Ordering::Acquire)
    }

    /// 再初期化の印を立てる (サスペンドから戻った直後。SIGCONT の自己パイプより先に描画が来ても入れ直すため)。
    pub fn request_reinit(&self) {
        self.reinit.store(true, Ordering::Release);
    }

    /// SIGCONT の後の再初期化が要るか (立っていたら下ろして `true`)。
    pub fn take_reinit(&self) -> bool {
        self.reinit.swap(false, Ordering::AcqRel)
    }

    /// raw モードを入れ直す (SIGCONT の後。止めたシェルが termios を戻しているため)。
    pub fn reenter_raw(&mut self) {
        let _ = set_termios(self.fd, &self.raw);
        self.restored = false;
    }

    pub fn restore(&mut self) {
        if self.restored {
            return;
        }
        self.restored = true;
        let _ = set_termios(self.fd, &self.saved);
    }
}

impl Drop for ConsoleInput {
    fn drop(&mut self) {
        self.restore();
        // SAFETY: 自分が開いた fd を閉じる。
        unsafe { libc::close(self.fd) };
    }
}

/// 端末の出力 (`/dev/tty`、ブロックする書込)。
pub struct ConsoleOut {
    fd: i32,
}

impl ConsoleOut {
    pub fn open() -> io::Result<ConsoleOut> {
        Ok(ConsoleOut {
            fd: open_tty(false)?,
        })
    }

    pub fn write_str(&mut self, s: &str) -> io::Result<()> {
        write_all(self.fd, s.as_bytes())
    }

    pub fn window_size(&self) -> io::Result<CellSize> {
        window_size(self.fd)
    }

    /// 描画先の fd (緊急の端末復元に渡す)。
    pub fn raw_handle(&self) -> usize {
        self.fd as usize
    }

    /// 出力の側には戻すモードが無い (termios は入力の側が戻す)。
    pub fn restore(&mut self) {}
}

impl Drop for ConsoleOut {
    fn drop(&mut self) {
        // SAFETY: 自分が開いた fd を閉じる。
        unsafe { libc::close(self.fd) };
    }
}

/// 自分だけに SIGTSTP を送って止まる (プロセスグループには送らない。シェル・tmux が前面のジョブとして止める、05 §11.3)。
/// SIGTSTP は受信を登録しないので既定の動作 (停止) になり、再開 (SIGCONT) まで戻らない。親が SIGTSTP を無視させて起動した
/// (ジョブ制御の無いシェル) ときは止まらずにすぐ戻る (呼び側は再開と同じに扱う)。
pub fn stop_self() {
    // SAFETY: 自分のプロセスにシグナルを送るだけ。
    unsafe {
        libc::kill(libc::getpid(), libc::SIGTSTP);
    }
}

/// バックグラウンドのジョブとして起動されたか (端末の前面のプロセスグループが自分のでない、05 §11.3)。端末を raw にする前に
/// 確かめる (バックグラウンドから termios を変えると SIGTTOU で止まるため)。`/dev/tty` を開けない・前面を調べられないときは
/// `false` (端末が無い場合は `Frontend::init` が扱う)。
pub fn is_background_job() -> bool {
    let Ok(fd) = open_tty(true) else {
        return false;
    };
    // SAFETY: 開いた fd を調べて閉じる。
    unsafe {
        let fg = libc::tcgetpgrp(fd);
        libc::close(fd);
        fg >= 0 && fg != libc::getpgrp()
    }
}

/// 端末のウィンドウの大きさから、編集面の `Resize` を作る (クローム行を除く、04 §3.1)。
pub fn surface_resize(window: CellSize, hint_bar: bool) -> InputEvent {
    InputEvent::Resize {
        cols: window.cols,
        rows: surface_rows_for_window(window.rows, hint_bar),
    }
}

/// シグナルの自己パイプ (読み端を非ブロックにする)。
fn signal_pipe(signo: i32) -> io::Result<UnixStream> {
    let (rd, wr) = UnixStream::pair()?;
    rd.set_nonblocking(true)?;
    signal_hook::low_level::pipe::register(signo, wr)?;
    Ok(rd)
}

fn drain(s: &UnixStream) {
    use std::io::Read;
    let mut b = [0u8; 64];
    while matches!((&*s).read(&mut b), Ok(n) if n > 0) {}
}

struct Loop<'a> {
    fd: i32,
    out: &'a dyn InputOut,
    geom: &'a SharedGeom,
    parser: VtParser,
    tracker: MouseTracker,
}

impl Loop<'_> {
    fn send(&mut self, outs: Vec<VtOut>) -> bool {
        for o in outs {
            match o {
                VtOut::Event(e) => {
                    if !self.out.event(e) {
                        return false;
                    }
                }
                VtOut::Mouse(r) => {
                    for m in self.tracker.feed(r, self.geom) {
                        if !self.out.event(InputEvent::Mouse(m)) {
                            return false;
                        }
                    }
                }
            }
        }
        true
    }

    fn resize(&self) -> bool {
        match window_size(self.fd) {
            Ok(s) => self.out.event(surface_resize(s, self.geom.hint_bar())),
            Err(_) => true,
        }
    }

    /// 待ちなしで読める分を読み切る。端末が閉じた (EOF / EIO) なら `None`。
    fn read_batch(&self, buf: &mut Vec<u8>) -> Option<io::Result<()>> {
        buf.clear();
        let mut chunk = vec![0u8; READ_CHUNK];
        loop {
            // SAFETY: `chunk` は有効な書込先。
            let n = unsafe { libc::read(self.fd, chunk.as_mut_ptr().cast(), chunk.len()) };
            if n > 0 {
                buf.extend_from_slice(&chunk[..n as usize]);
                continue;
            }
            if n == 0 {
                return if buf.is_empty() { None } else { Some(Ok(())) };
            }
            let e = io::Error::last_os_error();
            return match e.raw_os_error() {
                Some(libc::EAGAIN) => Some(Ok(())),
                Some(libc::EINTR) => continue,
                Some(libc::EIO) => None,
                _ => Some(Err(e)),
            };
        }
    }
}

fn run(
    fd: i32,
    out: &dyn InputOut,
    geom: &SharedGeom,
    winch: UnixStream,
    cont: UnixStream,
    reinit: &AtomicBool,
) {
    use std::os::unix::io::AsRawFd;
    let mut l = Loop {
        fd,
        out,
        geom,
        parser: VtParser::new(),
        tracker: MouseTracker::new(),
    };
    let mut batch = Vec::new();
    loop {
        let mut fds = [
            libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: winch.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: cont.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let timeout = if l.parser.has_pending() {
            ESC_WAIT_MS
        } else {
            -1
        };
        // SAFETY: 有効な pollfd の配列。
        let r = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout) };
        if r < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            out.died();
            return;
        }
        if r == 0 {
            let flushed = l.parser.flush();
            if !l.send(flushed) {
                return;
            }
            continue;
        }
        if fds[2].revents & libc::POLLIN != 0 {
            drain(&cont);
            reinit.store(true, Ordering::Release);
            if !l.resize() {
                return;
            }
        }
        if fds[1].revents & libc::POLLIN != 0 {
            drain(&winch);
            if !l.resize() {
                return;
            }
        }
        if fds[0].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
            match l.read_batch(&mut batch) {
                // 端末が閉じた (SIGHUP の経路が終了を扱う): 黙って終わる
                None => return,
                Some(Err(_)) => {
                    out.died();
                    return;
                }
                Some(Ok(())) => {
                    let outs = l.parser.feed(&batch);
                    if !l.send(outs) {
                        return;
                    }
                }
            }
        }
    }
}

/// 入力スレッドを起動する。termios は `ConsoleInput` が持ち、復元は呼び出し側 (`ConsoleInput::restore`)。
// 入力スレッドは 02 §9.1 の表のスレッド (R11 の例外。deps-policy.toml の `allowed_files` に載せている)。
#[allow(clippy::disallowed_methods)]
pub fn spawn_input_thread(
    input: &ConsoleInput,
    out: impl InputOut,
    geom: Arc<SharedGeom>,
) -> io::Result<JoinHandle<()>> {
    let winch = signal_pipe(libc::SIGWINCH)?;
    let cont = signal_pipe(libc::SIGCONT)?;
    let fd = input.fd;
    let reinit = input.reinit.clone();
    thread::Builder::new()
        .name("eve-input".to_owned())
        .spawn(move || run(fd, &out, &geom, winch, cont, &reinit))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::mpsc;

    struct Chan(Mutex<mpsc::Sender<InputEvent>>);

    impl InputOut for Chan {
        fn event(&self, ev: InputEvent) -> bool {
            self.0.lock().unwrap().send(ev).is_ok()
        }
        fn died(&self) {}
    }

    /// 擬似端末 (主, 従)。
    fn pty() -> (i32, i32) {
        let (mut m, mut s) = (-1, -1);
        let ws = libc::winsize {
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: 有効な書込先と winsize。
        let r =
            unsafe { libc::openpty(&mut m, &mut s, std::ptr::null_mut(), std::ptr::null(), &ws) };
        assert_eq!(r, 0);
        (m, s)
    }

    #[test]
    fn 擬似端末の入力を読んで_input_event_にし_sigwinch_で_resize() {
        let (m, s) = pty();
        // 従の側を raw にする (`ConsoleInput::open` と同じ。行単位のままだと改行まで読めない)
        let mut raw = termios_of(s).unwrap();
        // SAFETY: 有効な termios。
        unsafe { libc::cfmakeraw(&mut raw) };
        set_termios(s, &raw).unwrap();
        // SAFETY: 従の側を非ブロックにする (`ConsoleInput::open` と同じ)。
        unsafe {
            let fl = libc::fcntl(s, libc::F_GETFL);
            libc::fcntl(s, libc::F_SETFL, fl | libc::O_NONBLOCK);
        }
        let (tx, rx) = mpsc::channel();
        let geom = Arc::new(SharedGeom::default());
        let winch = signal_pipe(libc::SIGWINCH).unwrap();
        let cont = signal_pipe(libc::SIGCONT).unwrap();
        let reinit = Arc::new(AtomicBool::new(false));
        let r2 = reinit.clone();
        let out = Chan(Mutex::new(tx));
        #[allow(clippy::disallowed_methods)]
        std::thread::spawn(move || run(s, &out, &geom, winch, cont, &r2));
        write_all(m, "aあ\x1b[A\x13".as_bytes()).unwrap();
        let mut got = Vec::new();
        while got.len() < 3 {
            got.push(
                rx.recv_timeout(std::time::Duration::from_secs(5))
                    .expect("届く"),
            );
        }
        assert_eq!(got[0], InputEvent::Text("aあ".to_owned()));
        assert!(matches!(got[1], InputEvent::Key(k) if k.code == eve_view::KeyCode::Up));
        // 単独の ESC は同じ読取に続きが無いので Esc
        write_all(m, b"\x1b").unwrap();
        let mut esc = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        if !matches!(esc, InputEvent::Key(k) if k.code == eve_view::KeyCode::Esc) {
            esc = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        }
        assert!(
            matches!(esc, InputEvent::Key(k) if k.code == eve_view::KeyCode::Esc),
            "{esc:?}"
        );
        // SIGWINCH: 端末の大きさを変えてから送る
        let ws = libc::winsize {
            ws_row: 30,
            ws_col: 100,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: 主の側の大きさを変え、自分に SIGWINCH を送る。
        unsafe {
            libc::ioctl(m, libc::TIOCSWINSZ, &ws);
            libc::raise(libc::SIGWINCH);
        }
        let ev = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(matches!(ev, InputEvent::Resize { cols: 100, .. }), "{ev:?}");
        // SIGCONT: 再初期化の印と Resize
        // SAFETY: 自分に SIGCONT を送る (止まっていないので何も起きない)。
        unsafe { libc::raise(libc::SIGCONT) };
        let ev = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(matches!(ev, InputEvent::Resize { .. }), "{ev:?}");
        assert!(reinit.load(Ordering::Acquire));
        // SAFETY: 主の側を閉じる (従は入力スレッドが読み続ける)。
        unsafe { libc::close(m) };
    }
}
