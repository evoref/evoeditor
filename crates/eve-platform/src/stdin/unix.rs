//! 標準入力の読取の Unix の部分 (docs/05 §9.2 の「標準入力の読取」)。OS を呼ぶのはこのファイルだけ (R5)。
//!
//! - 私有の fd: `fcntl(F_DUPFD_CLOEXEC)` で fd 0 を高い番号へ移し、`/dev/null` を fd 0 に `dup2` する。
//! - 読み方: 私有の fd と停止の自己パイプを `poll` で待ってから `read(2)` する (`EINTR`・`EAGAIN` は待ち直す。0 バイトが
//!   EOF)。私有の fd の `O_NONBLOCK` は外さない (open file description を共有する他のプロセスにも効くため)。`poll` が
//!   `POLLNVAL` を返す fd (macOS のデバイス) は、ブロックする `read` に切り替える (デバイスはすぐ返る)。
//! - 止め方: 停止の自己パイプに書いて書き端を閉じる。`eve-stdin` が `poll` で気づき、私有の fd を自分で閉じて終わる。

#![allow(unsafe_code)]

use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use super::{CHUNK, Msg};

/// 私有の fd を移す先の下限 (標準入出力と、起動の直後に開く fd を避ける。05 §9.2)。
const PRIVATE_FD_MIN: libc::c_int = 10;

/// 私有の fd。
pub(super) struct Private(OwnedFd);

impl Private {
    /// 任意の fd (パイプの読み端など) を私有の fd として使う (試験の口 `PlatformStdin::from_pipe`)。
    pub(super) fn from_fd(fd: OwnedFd) -> Private {
        Private(fd)
    }
}

/// 停止の自己パイプの書き端。
pub(super) type Waker = UnixStream;
/// 停止の自己パイプの読み端 (`eve-stdin` が持つ)。
pub(super) type Stopped = UnixStream;

/// 標準入力 (fd 0) を私有の fd へ移し、fd 0 に `/dev/null` を置く。
pub(super) fn take_process_stdin() -> io::Result<Private> {
    // SAFETY: fd 0 を複製するだけ (fd 0 が閉じていれば EBADF が返る)。
    let fd = unsafe { libc::fcntl(0, libc::F_DUPFD_CLOEXEC, PRIVATE_FD_MIN) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fcntl` が返した、ほかに持ち主の無い fd。
    let private = unsafe { OwnedFd::from_raw_fd(fd) };
    // SAFETY: NUL 終端の固定のパス。
    let null = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    if null < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `open` が返した、ほかに持ち主の無い fd。
    let null = unsafe { OwnedFd::from_raw_fd(null) };
    // fd 0 を空けたままにしない (後で開くファイルが fd 0 を得て、標準入力と取り違えられない)。dup2 は FD_CLOEXEC を
    // 写さないので、fd 0 は子プロセスに引き継がれる (読んで空)
    // SAFETY: 開いている fd を fd 0 に複製する。
    if unsafe { libc::dup2(null.as_raw_fd(), 0) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Private(private))
}

/// 停止の自己パイプ (書き端, 読み端)。
pub(super) fn stop_pair() -> io::Result<(Waker, Stopped)> {
    UnixStream::pair()
}

/// `eve-stdin` を止める: 停止の自己パイプの書き端を閉じる (読み端が `POLLHUP` で起きる。書かない: `eve-stdin` が
/// 先に終わって読み端を閉じていると、書込が `EPIPE` になる)。待たない。
pub(super) fn stop(waker: Waker, _thread: &JoinHandle<()>, _finished: &AtomicBool, _wait: bool) {
    drop(waker);
}

/// `poll` の後にすること。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Next {
    /// 止める (停止の自己パイプが読める・閉じた)。
    Stop,
    /// 読める (または閉じた・誤り。`read` で確かめる)。
    Read,
    /// `POLLNVAL`: `poll` が使えない fd。以後はブロックする `read` で読む。
    Blocking,
    /// まだ何も無い (待ち直す)。
    Wait,
}

/// `poll` の結果 (標準入力の fd と停止の自己パイプの `revents`) から、次にすることを決める。止める指示を先に見る。
pub(super) fn after_poll(fd_revents: libc::c_short, stop_revents: libc::c_short) -> Next {
    if stop_revents != 0 {
        Next::Stop
    } else if fd_revents & libc::POLLNVAL != 0 {
        Next::Blocking
    } else if fd_revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
        Next::Read
    } else {
        Next::Wait
    }
}

fn pollin(fd: BorrowedFd<'_>) -> libc::pollfd {
    libc::pollfd {
        fd: fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    }
}

/// `eve-stdin` の本体。終わる前に私有の fd を閉じる (この関数が持って drop する)。`blocking` は最初から `poll` を
/// 使わない (試験)。
pub(super) fn read_loop(
    private: Private,
    stopped: Stopped,
    stop: &AtomicBool,
    blocking: bool,
    sink: &mut dyn FnMut(Msg) -> bool,
) {
    let fd = private.0;
    let mut blocking = blocking;
    let mut buf = vec![0u8; CHUNK];
    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }
        if !blocking {
            let mut fds = [pollin(fd.as_fd()), pollin(stopped.as_fd())];
            // SAFETY: 有効な pollfd の配列 (どの fd もこの関数の間は開いている)。
            let r = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) };
            if r < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                sink(Msg::Err(e));
                break;
            }
            match after_poll(fds[0].revents, fds[1].revents) {
                Next::Stop => break,
                Next::Wait => continue,
                Next::Blocking => blocking = true,
                Next::Read => {}
            }
        }
        // SAFETY: 開いている fd (この関数が持つ)。`buf` は有効な書込先。
        let n = unsafe { libc::read(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
        if n > 0 {
            if !sink(Msg::Data(buf[..n.unsigned_abs()].to_vec())) {
                break;
            }
            continue;
        }
        if n == 0 {
            sink(Msg::Eof);
            break;
        }
        let e = io::Error::last_os_error();
        match e.raw_os_error() {
            Some(libc::EINTR) => {}
            // 書き手のシェルが O_NONBLOCK にしたパイプ: `poll` で待ち直す。`poll` が使えない fd では短く譲る (デバイスは
            // すぐ返るので、ふつうは起きない)
            Some(code) if code == libc::EAGAIN || code == libc::EWOULDBLOCK => {
                if blocking {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
            _ => {
                sink(Msg::Err(e));
                break;
            }
        }
    }
    // 私有の fd はこのスレッドで閉じる (別のスレッドから閉じない、05 §9.2)
    drop(fd);
}
