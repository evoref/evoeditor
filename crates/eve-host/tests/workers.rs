//! 本番の `ThreadJobSpawner` (実スレッド) と `Host::run` の結合。時計は実時間、ファイルは `MemFs`。

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use eve_core::{
    FileMeta, FileSystem, FsError, FsPath, JobError, JobKind, JobPorts, JobResult, JobSpawner,
    MetaPurpose, Ports, ReadChunks, SaveOptions, SaveReport,
};
use eve_host::{ExitCode, Host, HostMsg, SystemClock, ThreadJobSpawner, channel};
use eve_testkit::{AsciiOptions, FixedPaths, HeadlessFrontend, MemFs};
use eve_view::{InputEvent, KeyCode, KeyEvent, Modifiers, Revision};

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(Instant::now() < end, "待ちきれませんでした: {what}");
        thread::sleep(Duration::from_millis(5));
    }
}

fn ctrl(c: char) -> InputEvent {
    InputEvent::Key(KeyEvent::press(KeyCode::Char(c), Modifiers::CTRL))
}

#[test]
fn 実スレッドで開き_編集し_保存して_終了する() {
    let clock = Arc::new(SystemClock::new());
    let fs = Arc::new(MemFs::new(clock.clone()));
    fs.add_dir("/w");
    fs.add_file("/w/a.txt", b"hello");
    let paths = Arc::new(FixedPaths::default());
    let jp = JobPorts {
        fs: fs.clone(),
        clock: clock.clone(),
        paths: paths.clone(),
    };
    let (tx, rx) = channel();
    let spawner = ThreadJobSpawner::new(jp, tx.clone()).unwrap();
    let ports = Ports {
        fs: fs.clone(),
        clock,
        jobs: std::rc::Rc::new(spawner),
        paths,
    };
    let mut host = Host::with_channel(ports, (tx, rx));
    let mut fe = HeadlessFrontend::new(40, 8);
    let probe = fe.probe();

    // 画面が「hello」になったら編集・保存・終了を送る別スレッド (入力スレッドの代わり)
    let probe2 = probe.clone();
    let fs2 = fs.clone();
    let driver = thread::spawn(move || {
        let sink_ready = Instant::now() + Duration::from_secs(10);
        let sink = loop {
            if let Some(s) = probe2.sink() {
                break s;
            }
            assert!(Instant::now() < sink_ready);
            thread::sleep(Duration::from_millis(2));
        };
        wait_until("読込", || {
            probe2
                .last_ascii(AsciiOptions::default())
                .is_some_and(|a| a.contains("hello"))
        });
        assert!(sink.send(InputEvent::Text("X".into())));
        assert!(sink.send(ctrl('s')));
        wait_until("保存", || {
            fs2.read("/w/a.txt").as_deref() == Some(b"Xhello")
        });
        // 保存の完了が画面と Editor に届くのを待ってから終了する
        thread::sleep(Duration::from_millis(100));
        assert!(sink.send(ctrl('q')));
    });
    // 想定外に終了できないときにテストが固まらないための見張り (終了コードの検査で失敗にする)
    let sender = host.sender();
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(20));
        let _ = sender.send(HostMsg::Signal(eve_host::SignalKind::Interrupt));
    });
    let code = host.run(
        &mut fe,
        &[eve_core::InitialFile {
            path: "/w/a.txt".into(),
            line: None,
            col: None,
        }],
        false,
    );
    driver.join().unwrap();
    assert_eq!(code, ExitCode::OK);
    assert_eq!(probe.restore_count(), 1);
}

/// `metadata` でパニックするファイルシステム。
struct PanicFs;

impl FileSystem for PanicFs {
    fn metadata(&self, _: &FsPath) -> Result<FileMeta, FsError> {
        panic!("テスト用のパニック");
    }
    fn canonicalize(&self, p: &FsPath) -> Result<FsPath, FsError> {
        Ok(p.clone())
    }
    fn open_read(&self, _: &FsPath) -> Result<Box<dyn ReadChunks + Send>, FsError> {
        panic!("使わない");
    }
    fn write_atomic(
        &self,
        _: &FsPath,
        _: &mut dyn ReadChunks,
        _: &SaveOptions,
    ) -> Result<SaveReport, FsError> {
        panic!("使わない");
    }
}

fn recv_job(rx: &std::sync::mpsc::Receiver<HostMsg>) -> JobResult {
    match rx.recv_timeout(Duration::from_secs(10)).unwrap() {
        HostMsg::Job(r) => *r,
        other => panic!("ジョブの結果ではありません: {other:?}"),
    }
}

#[test]
fn ワーカーのパニックは_failed_で返り_ワーカーは生き続ける() {
    let clock = Arc::new(SystemClock::new());
    let jp = JobPorts {
        fs: Arc::new(PanicFs),
        clock,
        paths: Arc::new(FixedPaths::default()),
    };
    let (tx, rx) = channel();
    let sp = ThreadJobSpawner::new(jp, tx).unwrap();
    let kind = || JobKind::Metadata {
        path: FsPath::from("/w/a.txt"),
        purpose: MetaPurpose::ExternalCheck,
    };
    let a = sp.spawn(None, Revision(0), kind());
    let b = sp.spawn(None, Revision(0), kind());
    assert_ne!(a, b);
    for want in [a, b] {
        match recv_job(&rx) {
            JobResult::Failed {
                job,
                err: JobError::Panic,
            } => assert_eq!(job, want, "投入順に直列に実行される"),
            other => panic!("{other:?}"),
        }
    }
    assert!(
        rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "結果は 1 件ずつ"
    );
}

/// 1 件目の `metadata` が `release` されるまで止まるファイルシステム。
struct GateFs {
    inner: Arc<MemFs>,
    entered: std::sync::mpsc::Sender<()>,
    release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

impl FileSystem for GateFs {
    fn metadata(&self, p: &FsPath) -> Result<FileMeta, FsError> {
        let _ = self.entered.send(());
        let _ = self.release.lock().unwrap().recv();
        self.inner.metadata(p)
    }
    fn canonicalize(&self, p: &FsPath) -> Result<FsPath, FsError> {
        self.inner.canonicalize(p)
    }
    fn open_read(&self, p: &FsPath) -> Result<Box<dyn ReadChunks + Send>, FsError> {
        self.inner.open_read(p)
    }
    fn write_atomic(
        &self,
        p: &FsPath,
        d: &mut dyn ReadChunks,
        o: &SaveOptions,
    ) -> Result<SaveReport, FsError> {
        self.inner.write_atomic(p, d, o)
    }
}

#[test]
fn 待機中のジョブの取消しは_cancelled_で返り_結果は1件だけ() {
    let clock = Arc::new(SystemClock::new());
    let mem = Arc::new(MemFs::new(clock.clone()));
    mem.add_dir("/w");
    mem.add_file("/w/a.txt", b"x");
    let (etx, erx) = std::sync::mpsc::channel();
    let (rtx, rrx) = std::sync::mpsc::channel();
    let fs = Arc::new(GateFs {
        inner: mem,
        entered: etx,
        release: std::sync::Mutex::new(rrx),
    });
    let jp = JobPorts {
        fs,
        clock,
        paths: Arc::new(FixedPaths::default()),
    };
    let (tx, rx) = channel();
    let sp = ThreadJobSpawner::new(jp, tx).unwrap();
    let kind = || JobKind::Metadata {
        path: FsPath::from("/w/a.txt"),
        purpose: MetaPurpose::ExternalCheck,
    };
    let a = sp.spawn(None, Revision(0), kind());
    // a が実行中 (ワーカーが止まっている) の間に b を投入して取り消す
    erx.recv_timeout(Duration::from_secs(10)).unwrap();
    let b = sp.spawn(None, Revision(0), kind());
    sp.cancel(b);
    // 存在しない JobId の取消しは何もしない
    sp.cancel(eve_view::JobId(999));
    rtx.send(()).unwrap();
    let mut got = [recv_job(&rx), recv_job(&rx)];
    got.sort_by_key(|r| r.job().0);
    assert!(
        matches!(got[0], JobResult::Meta { job, .. } if job == a),
        "{:?}",
        got[0]
    );
    assert!(
        matches!(&got[1], JobResult::Failed { job, err: JobError::Cancelled } if *job == b),
        "{:?}",
        got[1]
    );
    assert!(
        rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "結果は 1 件ずつ"
    );
}

#[test]
fn 検索は_bg_ワーカーで走り_途中経過と終わりが届く() {
    let clock = Arc::new(SystemClock::new());
    let fs = Arc::new(MemFs::new(clock.clone()));
    let jp = JobPorts {
        fs: fs.clone(),
        clock,
        paths: Arc::new(FixedPaths::default()),
    };
    let (tx, rx) = channel();
    let spawner = ThreadJobSpawner::new(jp, tx).unwrap();
    let snapshot = eve_text::Text::from("a\n".repeat(600).as_str()).snapshot();
    let job = spawner.spawn(
        None,
        Revision(0),
        JobKind::Search {
            snapshot,
            query: eve_core::SearchQuery::literal("a"),
            from: eve_text::ByteOff(0),
            max_matches: 10_000,
            count_limit: 1_000_000,
        },
    );
    let (mut found, mut total) = (0usize, None);
    wait_until("検索の終わり", || {
        while let Ok(HostMsg::Job(r)) = rx.try_recv() {
            match *r {
                JobResult::SearchBatch { job: j, matches } if j == job => found += matches.len(),
                JobResult::SearchDone {
                    job: j, total: t, ..
                } if j == job => total = Some(t),
                _ => {}
            }
        }
        total.is_some()
    });
    assert_eq!((found, total), (600, Some(600)));
}
