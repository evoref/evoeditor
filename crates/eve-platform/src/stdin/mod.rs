//! 標準入力の読取 (`StdinSource` の実装。docs/02 §4.4 / §9.1、03 §9.2.2、05 §9.1 / §9.2。v0.4 の M25)。
//!
//! - **私有のハンドル / fd**: 作るとき (`PlatformStdin::from_process_stdin`) に標準入力を私有のハンドル / fd へ移し、
//!   標準入力の位置には `NUL` / `/dev/null` を置く (子プロセスがパイプの読み手を持ち続けない。閉じたハンドルの値の
//!   再利用で別のものを読まない)。
//! - **読取のスレッド `eve-stdin`**: 作るときに始め、私有のハンドル / fd から読んだチャンク (最大 1 MiB) を深さ 4 の
//!   チャネルで受け手 (`eve-stdin-job` のジョブ) に渡す。受け手はデータが無い間チャネルだけで待つ (時間で区切らない)。
//!   `wake` はチャネルへ「起きよ」の印を送る (1 つに畳む)。
//! - **止め方**: 受け手が読み手 (`ReadChunks`) を手放すと、`eve-stdin` を OS ごとの方法で起こして止める (Unix は停止の
//!   自己パイプ、Windows は停止の印と `CancelSynchronousIo` の繰り返し)。`eve-stdin` は自分で私有のハンドル / fd を
//!   閉じて終わる (別のスレッドから閉じない)。止めるのを待つのは、取った読み手を手放したときの Windows の繰り返しの間
//!   (最大約 0.5 秒。`eve-stdin-job` の上で起きる) だけで、それを過ぎたら切り離す。取られないまま `PlatformStdin` を
//!   手放したときは、印を立てて切り離すだけで待たない。**終了のときは待たない** (02 §9.6。プロセスの終わりでスレッド
//!   ごと終わる)。

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

use eve_core::{ChunkError, FsError, FsErrorKind, ReadChunks, StdinSource};

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as os;
#[cfg(windows)]
mod win;
#[cfg(windows)]
use win as os;

/// 1 回の読取の大きさ (チャンクの最大)。
const CHUNK: usize = 1024 * 1024;
/// 受け渡しのチャネルの深さ (チャンクの数)。
const DEPTH: usize = 4;

/// `eve-stdin` から受け手へ渡すもの。
enum Msg {
    Data(Vec<u8>),
    Eof,
    Err(io::Error),
    /// `wake` の印。
    Wake,
}

/// 受け手と `wake` が共有するもの。
struct Shared {
    /// `wake` の印がチャネルに入っている (印を 1 つに畳む)。
    wake_pending: AtomicBool,
    /// `wake` が印を送る口。
    tx: SyncSender<Msg>,
}

/// `eve-stdin` を止めるもの (`ReadChunks` の drop で使う)。
struct Stopper {
    /// 立てたら `eve-stdin` は次の読取の前に終わる。
    stop: Arc<AtomicBool>,
    /// `eve-stdin` が私有のハンドル / fd を閉じて終わった。
    finished: Arc<AtomicBool>,
    /// OS ごとの起こし方 (Unix は停止の自己パイプの書き端)。
    waker: os::Waker,
    /// Windows は `CancelSynchronousIo` に渡すスレッドのハンドルとして使う。join はしない (drop で切り離す)。
    thread: JoinHandle<()>,
}

/// 本番の `StdinSource` (05 §9.1 / §9.2)。
pub struct PlatformStdin {
    shared: Arc<Shared>,
    reader: Mutex<Option<StdinChunks>>,
}

impl PlatformStdin {
    /// 標準入力を私有のハンドル / fd へ移し (標準入力の位置には `NUL` / `/dev/null` を置く)、読取のスレッド `eve-stdin` を
    /// 始める。端末かどうかの判定は呼ぶ側 (Host) が先に行う (03 §9.2.2 の「標準入力が端末」)。標準入力のハンドルが
    /// 無い・無効なとき (Windows) は作れるが、最初の読取が失敗する (`msg.stdin.read_failed` の経路)。
    ///
    /// # Errors
    ///
    /// 停止の自己パイプの作成 (移し替えの前に行う。失敗したら標準入力はそのまま)、私有のハンドル / fd への移し替え、
    /// またはスレッドの作成に失敗したとき。移し替えの後のスレッドの作成の失敗では標準入力を取り戻せないので、Host は
    /// 最初から読めないとき (`msg.stdin.read_failed`、05 §9.1) と同じに扱う。
    pub fn from_process_stdin() -> io::Result<PlatformStdin> {
        // 端末かどうかの判定は呼ぶ側の責務 (端末を読むと、キー入力を取り合う)
        debug_assert!(
            !std::io::IsTerminal::is_terminal(&std::io::stdin()),
            "標準入力が端末のときは StdinSource を作らない (03 §9.2.2)"
        );
        let pair = os::stop_pair()?;
        PlatformStdin::start_with(os::take_process_stdin()?, pair, false)
    }

    /// 試験の口: パイプの読み端を私有のハンドル / fd として読む (標準入力を移さない)。Host と組み合わせた試験だけが使う。
    ///
    /// # Errors
    ///
    /// 停止の自己パイプ、またはスレッドの作成に失敗したとき。
    #[doc(hidden)]
    pub fn from_pipe(r: std::io::PipeReader) -> io::Result<PlatformStdin> {
        #[cfg(unix)]
        let private = os::Private::from_fd(r.into());
        #[cfg(windows)]
        let private = os::Private::from_handle(r.into());
        PlatformStdin::start(private)
    }

    fn start(private: os::Private) -> io::Result<PlatformStdin> {
        PlatformStdin::start_with(private, os::stop_pair()?, false)
    }

    /// `blocking`: Unix で `poll` を使わずにブロックする `read` だけで読む (`POLLNVAL` の退路の試験)。
    fn start_with(
        private: os::Private,
        (waker, stopped): (os::Waker, os::Stopped),
        blocking: bool,
    ) -> io::Result<PlatformStdin> {
        let (tx, rx) = mpsc::sync_channel::<Msg>(DEPTH);
        let stop = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let (stop2, finished2, tx2) = (stop.clone(), finished.clone(), tx.clone());
        // 標準入力の読取のスレッド (02 §9.1 の表。deps-policy.toml の R11 の許可)
        #[allow(clippy::disallowed_methods)]
        let thread = std::thread::Builder::new()
            .name("eve-stdin".to_owned())
            .spawn(move || {
                let mut done = Done {
                    tx: tx2,
                    sent_end: false,
                    finished: finished2,
                };
                os::read_loop(private, stopped, &stop2, blocking, &mut |m| done.send(m));
            })?;
        let shared = Arc::new(Shared {
            wake_pending: AtomicBool::new(false),
            tx,
        });
        let chunks = StdinChunks {
            rx: Some(rx),
            cur: Vec::new(),
            done: false,
            taken: false,
            shared: shared.clone(),
            stopper: Some(Stopper {
                stop,
                finished,
                waker,
                thread,
            }),
        };
        Ok(PlatformStdin {
            shared,
            reader: Mutex::new(Some(chunks)),
        })
    }
}

impl StdinSource for PlatformStdin {
    fn take(&self) -> Option<Box<dyn ReadChunks + Send>> {
        self.reader
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .map(|mut c| {
                c.taken = true;
                Box::new(c) as Box<dyn ReadChunks + Send>
            })
    }

    fn wake(&self) {
        if self.shared.wake_pending.swap(true, Ordering::AcqRel) {
            return;
        }
        // チャネルが一杯なら受け手は待っていない (次のチャンクの区切りで取消しを見る)。印は畳んだままにしない
        if let Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) =
            self.shared.tx.try_send(Msg::Wake)
        {
            self.shared.wake_pending.store(false, Ordering::Release);
        }
    }
}

/// `eve-stdin` の送り口。終わりの印 (EOF・失敗) を送らずに終わる (パニック・止められた) と受け手が待ち続けるので、
/// drop で失敗を送る (受け手が既に手放していれば何もしない)。終わったことを `finished` で知らせる。
struct Done {
    tx: SyncSender<Msg>,
    sent_end: bool,
    finished: Arc<AtomicBool>,
}

impl Done {
    /// 受け手へ送る。受け手が手放していたら `false` (読むのをやめる)。
    fn send(&mut self, m: Msg) -> bool {
        self.sent_end |= matches!(m, Msg::Eof | Msg::Err(_));
        self.tx.send(m).is_ok()
    }
}

impl Drop for Done {
    fn drop(&mut self) {
        if !self.sent_end {
            let _ = self
                .tx
                .send(Msg::Err(io::Error::other("eve-stdin が終わった")));
        }
        self.finished.store(true, Ordering::Release);
    }
}

/// 受け手の側の読み手 (`ReadChunks`)。
struct StdinChunks {
    rx: Option<Receiver<Msg>>,
    cur: Vec<u8>,
    done: bool,
    /// 受け手が取った (`take`)。取られないまま手放したときは、止めるのを待たない。
    taken: bool,
    shared: Arc<Shared>,
    stopper: Option<Stopper>,
}

impl ReadChunks for StdinChunks {
    fn next_chunk(&mut self) -> Result<Option<&[u8]>, ChunkError> {
        if self.done {
            return Ok(None);
        }
        let Some(rx) = self.rx.as_ref() else {
            return Ok(None);
        };
        match rx.recv() {
            Ok(Msg::Data(v)) => {
                self.cur = v;
                Ok(Some(&self.cur))
            }
            Ok(Msg::Wake) => {
                self.shared.wake_pending.store(false, Ordering::Release);
                Ok(Some(&[]))
            }
            Ok(Msg::Eof) => {
                self.done = true;
                Ok(None)
            }
            Ok(Msg::Err(e)) => {
                self.done = true;
                let mut fe = FsError::new(FsErrorKind::Other);
                fe.source = Some(e);
                Err(ChunkError::Io(fe))
            }
            // `Shared` が送り口を持つので起きない
            Err(_) => {
                self.done = true;
                Err(ChunkError::Io(FsError::new(FsErrorKind::Other)))
            }
        }
    }
}

impl Drop for StdinChunks {
    fn drop(&mut self) {
        // 先に受け口を閉じる (`eve-stdin` がチャネルの空きを待っていれば、すぐ戻る)
        self.rx.take();
        if let Some(s) = self.stopper.take() {
            s.stop.store(true, Ordering::Release);
            os::stop(s.waker, &s.thread, &s.finished, self.taken);
            // `thread` の drop で切り離す (join しない)
        }
    }
}

#[cfg(test)]
mod tests;
