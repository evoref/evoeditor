//! `FakeStdin`: 偽の標準入力 (`StdinSource`、docs/02 §4.4、09 §3.4 の「遅い読込と偽の標準入力」。v0.4 の M25)。
//!
//! 段 (`StdinStep`) の列を順に返す: チャンク・遅延 (`ManualClock` を進める)・途中の読取の失敗・閉じないパイプ (`Hang`)。
//! 段を使い切ったら EOF。`Hang` はデータの無いまま待つ段で、`wake` を受けたら空のチャンクを返して戻り、次の呼び出しで
//! また待つ (書き手が何も書かない閉じないパイプ)。単一スレッドの試験では、待つ前に `on_hang` の関数を 1 回呼ぶ (そこで
//! 取消しを立てて `wake` を呼べば、ジョブが待ちから戻って取消しを見る。本番で Host が取消しのときに `wake` を呼ぶのと同じ)。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use eve_core::{ChunkError, FsError, FsErrorKind, Millis, ReadChunks, StdinSource};

use crate::ManualClock;

/// `Hang` のまま起こされないときに試験を止めるまでの時間 (単一スレッドの試験の書き誤りで止まり続けないため)。
const HANG_LIMIT: Duration = Duration::from_secs(10);

/// 偽の標準入力の 1 段。
#[derive(Clone, Debug)]
pub enum StdinStep {
    /// このチャンクを返す (空にはしない。空のチャンクは `wake` の印の意味になるため)。
    Data(Vec<u8>),
    /// `with_clock` の時計をこの時間だけ進める (チャンクは返さず、次の段へ進む)。
    Delay(u64),
    /// 読取の失敗を返す。以後は読まない (EOF)。
    Error(FsErrorKind),
    /// 閉じないパイプ: データが無いまま待つ。以後の段へは進まない。
    Hang,
}

type HangHook = Box<dyn FnMut() + Send>;

struct Shared {
    steps: Mutex<VecDeque<StdinStep>>,
    clock: Option<ManualClock>,
    /// `wake` の印 (1 つに畳む)。
    woken: Mutex<bool>,
    cond: Condvar,
    taken: AtomicBool,
    dropped: AtomicBool,
    wakes: AtomicU32,
    on_hang: Mutex<Option<HangHook>>,
}

/// 偽の `StdinSource` (`Arc` で `JobPorts.stdin` に渡し、試験の側でも持って調べる)。
#[derive(Clone)]
pub struct FakeStdin(Arc<Shared>);

impl FakeStdin {
    /// 段の列から作る。
    pub fn new(steps: impl IntoIterator<Item = StdinStep>) -> FakeStdin {
        FakeStdin::build(steps, None)
    }

    /// `bytes` を `chunk` バイトずつ返して EOF になる偽物。
    pub fn chunks(bytes: &[u8], chunk: usize) -> FakeStdin {
        FakeStdin::new(
            bytes
                .chunks(chunk.max(1))
                .map(|c| StdinStep::Data(c.to_vec())),
        )
    }

    /// `Delay` の段で `clock` を進める偽物。
    pub fn with_clock(steps: impl IntoIterator<Item = StdinStep>, clock: ManualClock) -> FakeStdin {
        FakeStdin::build(steps, Some(clock))
    }

    fn build(steps: impl IntoIterator<Item = StdinStep>, clock: Option<ManualClock>) -> FakeStdin {
        FakeStdin(Arc::new(Shared {
            steps: Mutex::new(steps.into_iter().collect()),
            clock,
            woken: Mutex::new(false),
            cond: Condvar::new(),
            taken: AtomicBool::new(false),
            dropped: AtomicBool::new(false),
            wakes: AtomicU32::new(0),
            on_hang: Mutex::new(None),
        }))
    }

    /// `Hang` で待つ前に呼ぶ関数 (待つたびに 1 回)。単一スレッドの試験で、取消しを立てて `wake` を呼ぶのに使う。
    pub fn on_hang(&self, f: impl FnMut() + Send + 'static) {
        *self
            .0
            .on_hang
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Box::new(f));
    }

    /// 読み手を取った (`take` が `Some` を返した)。
    pub fn taken(&self) -> bool {
        self.0.taken.load(Ordering::Acquire)
    }

    /// 読み手が手放された (ジョブが読むのをやめた。本番では `eve-stdin` を止める契機)。
    pub fn reader_dropped(&self) -> bool {
        self.0.dropped.load(Ordering::Acquire)
    }

    /// `wake` が呼ばれた回数。
    pub fn wakes(&self) -> u32 {
        self.0.wakes.load(Ordering::Acquire)
    }
}

impl StdinSource for FakeStdin {
    fn take(&self) -> Option<Box<dyn ReadChunks + Send>> {
        if self.0.taken.swap(true, Ordering::AcqRel) {
            return None;
        }
        Some(Box::new(FakeReader {
            shared: Arc::clone(&self.0),
            cur: Vec::new(),
            done: false,
        }))
    }

    fn wake(&self) {
        self.0.wakes.fetch_add(1, Ordering::AcqRel);
        *self.0.woken.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.0.cond.notify_all();
    }
}

struct FakeReader {
    shared: Arc<Shared>,
    cur: Vec<u8>,
    done: bool,
}

impl FakeReader {
    /// `wake` の印を待つ (印は 1 つに畳み、受けたら下ろす)。
    fn wait_woken(&self) {
        let hook = self
            .shared
            .on_hang
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(mut f) = hook {
            f();
            *self
                .shared
                .on_hang
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(f);
        }
        let mut woken = self
            .shared
            .woken
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        while !*woken {
            let (w, t) = self
                .shared
                .cond
                .wait_timeout(woken, HANG_LIMIT)
                .unwrap_or_else(PoisonError::into_inner);
            woken = w;
            assert!(
                !t.timed_out() || *woken,
                "偽の標準入力: Hang のまま起こされない (取消しの後に wake を呼んでいない)"
            );
        }
        *woken = false;
    }
}

impl ReadChunks for FakeReader {
    fn next_chunk(&mut self) -> Result<Option<&[u8]>, ChunkError> {
        if self.done {
            return Ok(None);
        }
        // `wake` の印はデータの後ろに並ぶ (本番のチャネルと同じ)。データの段では印を残し、`Hang` で受ける
        loop {
            let step = self
                .shared
                .steps
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .front()
                .cloned();
            match step {
                None => {
                    self.done = true;
                    return Ok(None);
                }
                Some(StdinStep::Hang) => {
                    self.wait_woken();
                    return Ok(Some(&[]));
                }
                Some(s) => {
                    self.shared
                        .steps
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .pop_front();
                    match s {
                        StdinStep::Data(d) => {
                            self.cur = d;
                            return Ok(Some(&self.cur));
                        }
                        StdinStep::Delay(ms) => {
                            if let Some(c) = &self.shared.clock {
                                c.advance(Millis(ms));
                            }
                        }
                        StdinStep::Error(kind) => {
                            self.done = true;
                            return Err(ChunkError::Io(FsError::new(kind)));
                        }
                        StdinStep::Hang => unreachable!("上で扱った"),
                    }
                }
            }
        }
    }
}

impl Drop for FakeReader {
    fn drop(&mut self) {
        self.shared.dropped.store(true, Ordering::Release);
    }
}
