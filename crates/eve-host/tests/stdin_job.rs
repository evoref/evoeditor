//! 標準入力の読込のスレッド `eve-stdin-job` (docs/02 §9.1、03 §9.2.2。v0.4 の M25 の (8)): `io` / `bg` に載せず
//! (閉じないパイプの間も `io` の読込・保存が進む)、取消しで `wake` を呼んで待ちを起こし、終了のときに待たない。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use eve_core::{
    ChunkError, FsPath, JobKind, JobOutput, JobPorts, JobResult, JobSpawner, MetaPurpose,
    OpenOptions, ReadChunks, StdinEnd, StdinSource,
};
use eve_host::{HostMsg, SystemClock, ThreadJobSpawner, channel};
use eve_testkit::{FakeStdin, FixedPaths, MemFs, StdinStep};
use eve_view::Revision;

fn recv_job(rx: &std::sync::mpsc::Receiver<HostMsg>) -> JobOutput {
    match rx.recv_timeout(Duration::from_secs(10)).unwrap() {
        HostMsg::Job(r) => *r,
        other => panic!("ジョブの結果ではありません: {other:?}"),
    }
}

/// 終端 (`StdinDone`) まで受ける (途中の `Progress` は読み飛ばす)。
fn recv_done(rx: &std::sync::mpsc::Receiver<HostMsg>) -> JobOutput {
    loop {
        let r = recv_job(rx);
        if !matches!(r.body, JobResult::Progress { .. }) {
            return r;
        }
    }
}

fn spawner(stdin: Arc<dyn StdinSource>) -> (ThreadJobSpawner, std::sync::mpsc::Receiver<HostMsg>) {
    let clock = Arc::new(SystemClock::new());
    let fs = Arc::new(MemFs::new(clock.clone()));
    fs.add_dir("/w");
    fs.add_file("/w/a.txt", b"x");
    let jp = JobPorts {
        stdin: Some(stdin),
        fs,
        clock,
        paths: Arc::new(FixedPaths::default()),
    };
    let (tx, rx) = channel();
    (ThreadJobSpawner::new(jp, tx).unwrap(), rx)
}

#[test]
fn 閉じないパイプの間も_io_のジョブは進み_取消しで読んだところまでが届く() {
    let fake = FakeStdin::new([StdinStep::Data(b"tail -f\n".to_vec()), StdinStep::Hang]);
    // データを読み終えて待ちに入ったことを知る
    let (htx, hrx) = std::sync::mpsc::channel();
    fake.on_hang(move || {
        let _ = htx.send(());
    });
    let (sp, rx) = spawner(Arc::new(fake.clone()));
    let stdin = sp.spawn(
        None,
        Revision(0),
        JobKind::OpenStdin {
            opts: OpenOptions::default(),
        },
    );
    // 標準入力の読込が待っている間に、io のジョブが終わる
    let meta = sp.spawn(
        None,
        Revision(0),
        JobKind::Metadata {
            path: FsPath::from("/w/a.txt"),
            purpose: MetaPurpose::ExternalCheck,
        },
    );
    let r = loop {
        let r = recv_job(&rx);
        if r.job == meta {
            break r;
        }
    };
    assert!(matches!(r.body, JobResult::Meta { .. }), "{r:?}");
    // 取消し: Host が wake を呼び、ジョブは読んだところまでを StdinDone で返す
    hrx.recv_timeout(Duration::from_secs(10)).unwrap();
    sp.cancel(stdin);
    let r = recv_done(&rx);
    assert_eq!(r.job, stdin);
    let JobResult::StdinDone { text, end, .. } = r.body else {
        panic!("{r:?}");
    };
    assert!(matches!(end, StdinEnd::Stopped), "{end:?}");
    assert_eq!(eve_text::TextRead::len_bytes(&text), 8);
    assert!(fake.wakes() >= 1);
    let end = Instant::now() + Duration::from_secs(10);
    while !fake.reader_dropped() {
        assert!(Instant::now() < end, "読み手を手放さない");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// `wake` にも応じず読みかけのまま止まる読み手 (読取の中で止まったパイプの最悪の形)。
struct Stuck {
    released: Arc<AtomicBool>,
}

struct StuckReader(Arc<AtomicBool>);

impl ReadChunks for StuckReader {
    fn next_chunk(&mut self) -> Result<Option<&[u8]>, ChunkError> {
        while !self.0.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(None)
    }
}

impl StdinSource for Stuck {
    fn take(&self) -> Option<Box<dyn ReadChunks + Send>> {
        Some(Box::new(StuckReader(self.released.clone())))
    }
    fn wake(&self) {}
}

#[test]
fn 終了のとき標準入力の読込のスレッドを待たない() {
    let released = Arc::new(AtomicBool::new(false));
    let (sp, rx) = spawner(Arc::new(Stuck {
        released: released.clone(),
    }));
    sp.spawn(
        None,
        Revision(0),
        JobKind::OpenStdin {
            opts: OpenOptions::default(),
        },
    );
    // 最初の進捗が届けば、ジョブは読取に入っている
    assert!(matches!(recv_job(&rx).body, JobResult::Progress { .. }));
    let t = Instant::now();
    drop(sp);
    let took = t.elapsed();
    // io のレーンの終わりを待つ分 (空のキューなのですぐ) だけで、標準入力のスレッドは待たない (JOIN_WAIT は 1 秒)
    assert!(took < Duration::from_millis(500), "{took:?}");
    released.store(true, Ordering::Release);
}

#[test]
fn 標準入力の読込のジョブのパニックは_failed_で返る() {
    struct Boom;
    impl StdinSource for Boom {
        fn take(&self) -> Option<Box<dyn ReadChunks + Send>> {
            panic!("試験のパニック");
        }
        fn wake(&self) {}
    }
    let (sp, rx) = spawner(Arc::new(Boom));
    let id = sp.spawn(
        None,
        Revision(0),
        JobKind::OpenStdin {
            opts: OpenOptions::default(),
        },
    );
    let r = recv_done(&rx);
    assert_eq!(r.job, id);
    assert!(
        matches!(
            r.body,
            JobResult::Failed {
                err: eve_core::JobError::Panic
            }
        ),
        "{r:?}"
    );
}

#[test]
fn 本物の_platform_stdin_でも_wake_が通り_止める待ちは_eve_stdin_job_の上で起きる() {
    use std::io::Write;
    // 閉じないパイプ (書き手は開いたまま)。データを読んだ後、待ちに入ったところで取り消す
    let (r, mut w) = std::io::pipe().unwrap();
    let stdin = eve_platform::stdin::PlatformStdin::from_pipe(r).unwrap();
    let (sp, rx) = spawner(Arc::new(stdin));
    let id = sp.spawn(
        None,
        Revision(0),
        JobKind::OpenStdin {
            opts: OpenOptions::default(),
        },
    );
    w.write_all("読んだ\n".as_bytes()).unwrap();
    // ジョブがデータを受け取って、次のデータを待つまで待つ (受け取る前に取り消すと空になる)
    std::thread::sleep(Duration::from_millis(300));
    let t = Instant::now();
    sp.cancel(id);
    let cancel_took = t.elapsed();
    // 取消しは待ちを起こす印を送るだけ (止める待ち (Windows の CancelSynchronousIo の繰り返し) はメインで起きない)
    assert!(cancel_took < Duration::from_millis(50), "{cancel_took:?}");
    let r = recv_done(&rx);
    assert_eq!(r.job, id);
    let JobResult::StdinDone {
        text, end, bytes, ..
    } = r.body
    else {
        panic!("{r:?}");
    };
    assert!(matches!(end, StdinEnd::Stopped), "{end:?}");
    assert_eq!(bytes, "読んだ\n".len() as u64);
    assert_eq!(eve_text::TextRead::len_bytes(&text), "読んだ\n".len());
    // eve-stdin は自分で私有のハンドル / fd を閉じて終わる: 書き手の書込が失敗する
    let t = Instant::now();
    while w.write_all(b"x").is_ok() {
        assert!(t.elapsed() < Duration::from_secs(5), "閉じない");
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(sp);
}
