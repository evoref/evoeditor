//! 本番の `ThreadJobSpawner` (実スレッド) と `Host::run` の結合。時計は実時間、ファイルは `MemFs`。

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use eve_core::{
    FileMeta, FileSystem, FsError, FsPath, JobError, JobKind, JobOutput, JobPorts, JobResult,
    JobSpawner, MetaPurpose, Ports, ReadChunks, SaveOptions, SaveReport,
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
        stdin: None,
        fs: fs.clone(),
        clock: clock.clone(),
        paths: paths.clone(),
    };
    let (tx, rx) = channel();
    let spawner = ThreadJobSpawner::new(jp, tx.clone()).unwrap();
    let ports = Ports {
        watcher: None,
        fs: fs.clone(),
        clock,
        jobs: std::rc::Rc::new(spawner),
        paths,
    };
    let mut host = Host::with_channel(ports, (tx, rx), eve_host::HostInit::default());
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
            target: eve_core::InitialTarget::Path("/w/a.txt".into()),
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

fn recv_job(rx: &std::sync::mpsc::Receiver<HostMsg>) -> JobOutput {
    match rx.recv_timeout(Duration::from_secs(10)).unwrap() {
        HostMsg::Job(r) => *r,
        other => panic!("ジョブの結果ではありません: {other:?}"),
    }
}

#[test]
fn ワーカーのパニックは_failed_で返り_ワーカーは生き続ける() {
    let clock = Arc::new(SystemClock::new());
    let jp = JobPorts {
        stdin: None,
        fs: Arc::new(PanicFs),
        clock,
        paths: Arc::new(FixedPaths::default()),
    };
    let (tx, rx) = channel();
    let sp = ThreadJobSpawner::new(jp, tx).unwrap();
    let kind = || JobKind::Metadata {
        path: FsPath::from("/w/a.txt"),
        purpose: MetaPurpose::ExternalCheck,
        known_ids: Vec::new(),
    };
    let a = sp.spawn(None, Revision(0), kind());
    let b = sp.spawn(None, Revision(0), kind());
    assert_ne!(a, b);
    for want in [a, b] {
        let JobOutput { job, body } = recv_job(&rx);
        match body {
            JobResult::Failed {
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
        stdin: None,
        fs,
        clock,
        paths: Arc::new(FixedPaths::default()),
    };
    let (tx, rx) = channel();
    let sp = ThreadJobSpawner::new(jp, tx).unwrap();
    let kind = || JobKind::Metadata {
        path: FsPath::from("/w/a.txt"),
        purpose: MetaPurpose::ExternalCheck,
        known_ids: Vec::new(),
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
    got.sort_by_key(|r| r.job.0);
    assert!(
        matches!(got[0], JobOutput { job, body: JobResult::Meta { .. } } if job == a),
        "{:?}",
        got[0]
    );
    assert!(
        matches!(&got[1], JobOutput { job, body: JobResult::Failed { err: JobError::Cancelled } } if *job == b),
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
        stdin: None,
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
            let JobOutput { job: j, body } = *r;
            match body {
                JobResult::SearchBatch { matches, .. } if j == job => found += matches.len(),
                JobResult::SearchDone { total: t, .. } if j == job => total = Some(t),
                _ => {}
            }
        }
        total.is_some()
    });
    assert_eq!((found, total), (600, Some(600)));
}

#[test]
fn 単発の検索とすべて置換は_bg_で走り_io_の保存や読込を待たない() {
    // io のワーカーを止めておく (GateFs の metadata が release まで戻らない)。単発の検索 (`FindNext`) とすべて置換
    // (`ReplaceAll`) は bg で走るので、その間に結果が届く (02 §9.3、v0.4 の M24 で io から移した)
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
        stdin: None,
        fs,
        clock,
        paths: Arc::new(FixedPaths::default()),
    };
    let (tx, rx) = channel();
    let sp = ThreadJobSpawner::new(jp, tx).unwrap();
    let blocked = sp.spawn(
        None,
        Revision(0),
        JobKind::Metadata {
            path: FsPath::from("/w/a.txt"),
            purpose: MetaPurpose::ExternalCheck,
            known_ids: Vec::new(),
        },
    );
    erx.recv_timeout(Duration::from_secs(10)).unwrap();
    let snapshot = eve_text::Text::from("ab\nab\n").snapshot();
    let find = sp.spawn(
        None,
        Revision(0),
        JobKind::FindNext {
            snapshot: snapshot.clone(),
            query: eve_core::SearchQuery::literal("ab"),
            from: eve_text::ByteOff(1),
            forward: true,
            resume: Vec::new(),
        },
    );
    let replace = sp.spawn(
        None,
        Revision(0),
        JobKind::ReplaceAll {
            snapshot,
            query: eve_core::SearchQuery::literal("ab"),
            replacement: "x".to_owned(),
        },
    );
    let mut got = [recv_job(&rx), recv_job(&rx)];
    got.sort_by_key(|r| r.job.0);
    assert!(
        matches!(&got[0], JobOutput { job, body: JobResult::FindNextDone { found: Some(_) } } if *job == find),
        "{:?}",
        got[0]
    );
    assert!(
        matches!(&got[1], JobOutput { job, body: JobResult::ReplaceAllDone { count: 2, .. } } if *job == replace),
        "{:?}",
        got[1]
    );
    rtx.send(()).unwrap();
    let r = recv_job(&rx);
    assert_eq!(r.job, blocked, "止めていた io のジョブは後で終わる");
}

/// 全件の走査 (100 MB、大小を区別しない `quick`。行ごとの経路) の途中で投げた単発の検索とすべて置換が、結果を返すまでの時間
/// (計測用、03 §12.3.1 の「`bg` レーンの順序と取り消し」)。`bg` は先着順なので、そのまま投げると全件の走査の後ろで待つ
/// (fifo)。core は、走っている全件ジョブを取り消してから投げ、後ろに全件ジョブを投げ直す (cancel)。
/// `cargo test -p eve-host --release --test workers -- --ignored --nocapture 計測`
#[test]
#[ignore = "計測用"]
#[allow(clippy::print_stdout)]
fn 計測_全件の走査の途中の単発の検索とすべて置換の待ち() {
    let line = "0123456789 the quick brown fox jumps over the lazy dog 0123456789\n";
    let n = 100 * 1024 * 1024 / line.len();
    let mut doc = String::with_capacity(n * line.len() + 16);
    for i in 0..n {
        if i == (1 << 20) / line.len() {
            doc.push_str("MARK\n");
        }
        doc.push_str(line);
    }
    let snapshot = eve_text::Text::from(doc.as_str()).snapshot();
    let clock = Arc::new(SystemClock::new());
    let jp = JobPorts {
        stdin: None,
        fs: Arc::new(MemFs::new(clock.clone())),
        clock,
        paths: Arc::new(FixedPaths::default()),
    };
    let (tx, rx) = channel();
    let sp = ThreadJobSpawner::new(jp, tx).unwrap();
    let search = || JobKind::Search {
        snapshot: snapshot.clone(),
        query: eve_core::SearchQuery::literal("QUICK").with_case(eve_core::Case::Insensitive),
        from: eve_text::ByteOff(0),
        max_matches: 10_000,
        count_limit: 1_000_000,
    };
    let find = || JobKind::FindNext {
        snapshot: snapshot.clone(),
        query: eve_core::SearchQuery::literal("MARK").with_regex(true),
        from: eve_text::ByteOff(0),
        forward: true,
        resume: Vec::new(),
    };
    let replace = || JobKind::ReplaceAll {
        snapshot: snapshot.clone(),
        query: eve_core::SearchQuery::literal("MARK"),
        replacement: "X".to_owned(),
    };
    // 全件の走査だけの時間
    let t = Instant::now();
    let s = sp.spawn(None, Revision(0), search());
    loop {
        let r = recv_job(&rx);
        if r.job == s && !matches!(r.body, JobResult::SearchBatch { .. }) {
            break;
        }
    }
    println!("全件の走査だけ: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
    for (name, kind) in [("単発の検索", 0), ("すべて置換", 1)] {
        for cancel in [false, true] {
            let mut times = Vec::new();
            for _ in 0..3 {
                let s = sp.spawn(None, Revision(0), search());
                thread::sleep(Duration::from_millis(20));
                if cancel {
                    sp.cancel(s);
                }
                let t = Instant::now();
                let j = sp.spawn(
                    None,
                    Revision(0),
                    if kind == 0 { find() } else { replace() },
                );
                let again = cancel.then(|| sp.spawn(None, Revision(0), search()));
                let mut waited = None;
                let mut open: Vec<eve_view::JobId> = vec![s];
                open.extend(again);
                while waited.is_none() || !open.is_empty() {
                    let r = recv_job(&rx);
                    if r.job == j {
                        waited = Some(t.elapsed());
                    } else if !matches!(r.body, JobResult::SearchBatch { .. }) {
                        open.retain(|x| *x != r.job);
                    }
                }
                times.push(waited.unwrap());
            }
            times.sort();
            println!(
                "{name} ({}): {:.1} ms (3 回の中央値)",
                if cancel { "cancel" } else { "fifo" },
                times[1].as_secs_f64() * 1e3
            );
        }
    }
}

// ---------- 区切って譲るジョブ (JobStep::Continue、02 §9.3。ワークスペースの走査) ----------

/// `gates` のどれかの `read_dir` が `release` されるまで止まるファイルシステム (走査の 1 区切りの途中で止める)。
struct ReadDirGate {
    inner: Arc<MemFs>,
    gates: Vec<FsPath>,
    entered: std::sync::mpsc::Sender<()>,
    release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

impl FileSystem for ReadDirGate {
    fn metadata(&self, p: &FsPath) -> Result<FileMeta, FsError> {
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
    fn read_dir(&self, dir: &FsPath, max: usize) -> Result<eve_core::DirListing, FsError> {
        if self.gates.contains(dir) {
            let _ = self.entered.send(());
            let _ = self.release.lock().unwrap().recv();
        }
        self.inner.read_dir(dir, max)
    }
}

/// 40 個のディレクトリの木 `/w` (走査は 16 個ずつ 3 区切り) と小さな木 `/v` を置き、`gates` の列挙で止まる spawner。
fn gated_scan_at(
    gates: &[&str],
) -> (
    ThreadJobSpawner,
    std::sync::mpsc::Receiver<HostMsg>,
    std::sync::mpsc::Receiver<()>,
    std::sync::mpsc::Sender<()>,
) {
    let clock = Arc::new(SystemClock::new());
    let mem = Arc::new(MemFs::new(clock.clone()));
    for i in 0..40 {
        mem.add_file(&format!("/w/d{i:02}/f.txt"), b"x");
    }
    mem.add_file("/v/a.txt", b"x");
    let (etx, erx) = std::sync::mpsc::channel();
    let (rtx, rrx) = std::sync::mpsc::channel();
    let fs = Arc::new(ReadDirGate {
        inner: mem,
        gates: gates.iter().map(|g| FsPath::from(*g)).collect(),
        entered: etx,
        release: std::sync::Mutex::new(rrx),
    });
    let jp = JobPorts {
        stdin: None,
        fs,
        clock,
        paths: Arc::new(FixedPaths::default()),
    };
    let (tx, rx) = channel();
    (ThreadJobSpawner::new(jp, tx).unwrap(), rx, erx, rtx)
}

/// `/w/d00` の列挙で止まる spawner。
fn gated_scan() -> (
    ThreadJobSpawner,
    std::sync::mpsc::Receiver<HostMsg>,
    std::sync::mpsc::Receiver<()>,
    std::sync::mpsc::Sender<()>,
) {
    gated_scan_at(&["/w/d00"])
}

fn scan_kind() -> JobKind {
    scan_of("/w")
}

fn scan_of(root: &str) -> JobKind {
    JobKind::WorkspaceScan {
        root: FsPath::from(root),
        exclude: Arc::new(eve_core::ExcludeConfig::new()),
        generation: 1,
        walk: None,
    }
}

/// 終端の結果 (`ScanBatch` 以外) が `n` 件届くまで集める。
fn recv_until_terminals(rx: &std::sync::mpsc::Receiver<HostMsg>, n: usize) -> Vec<JobOutput> {
    let mut out = Vec::new();
    let mut terminals = 0;
    while terminals < n {
        let r = recv_job(rx);
        if !matches!(r.body, JobResult::ScanBatch { .. }) {
            terminals += 1;
        }
        out.push(r);
    }
    out
}

#[test]
fn 走査の続きはレーンの末尾に積まれ_間に投入されたジョブが先に走る() {
    let (sp, rx, entered, release) = gated_scan();
    let scan = sp.spawn(None, Revision(0), scan_kind());
    // 1 区切り目の途中 (`/w/d00` の列挙) で止まっている間に、同じ bg に別のジョブを積む
    entered.recv_timeout(Duration::from_secs(10)).unwrap();
    let other = sp.spawn(
        None,
        Revision(0),
        JobKind::ReleaseCandidates {
            chunks: Vec::new(),
            hits: Vec::new(),
        },
    );
    release.send(()).unwrap();
    let r = recv_until_terminals(&rx, 2);
    let pos = |id, pred: fn(&JobResult) -> bool| {
        r.iter()
            .position(|o| o.job == id && pred(&o.body))
            .unwrap_or_else(|| panic!("{r:?}"))
    };
    let released = pos(other, |b| matches!(b, JobResult::Released));
    let done = pos(scan, |b| {
        matches!(
            b,
            JobResult::ScanDone {
                generation: 1,
                total: 40,
                truncated: false
            }
        )
    });
    assert!(released < done, "続きは間に入ったジョブの後に走る: {r:?}");
    let found: usize = r
        .iter()
        .filter_map(|o| match &o.body {
            JobResult::ScanBatch { chunk, .. } => Some(chunk.len()),
            _ => None,
        })
        .sum();
    assert_eq!(found, 40);
    assert!(
        rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "終端の後に結果は来ない"
    );
}

#[test]
fn 区切りの途中で取り消した走査は続きを積まず_cancelled_を1件だけ返す() {
    let (sp, rx, entered, release) = gated_scan();
    let scan = sp.spawn(None, Revision(0), scan_kind());
    entered.recv_timeout(Duration::from_secs(10)).unwrap();
    sp.cancel(scan);
    release.send(()).unwrap();
    let r = recv_until_terminals(&rx, 1);
    assert!(
        matches!(
            r.last(),
            Some(JobOutput {
                job,
                body: JobResult::Failed {
                    err: JobError::Cancelled
                }
            }) if *job == scan
        ),
        "{r:?}"
    );
    assert!(
        rx.recv_timeout(Duration::from_millis(200)).is_err(),
        "続きは積まれない"
    );
    // レーンは生きている
    let next = sp.spawn(
        None,
        Revision(0),
        JobKind::ReleaseCandidates {
            chunks: Vec::new(),
            hits: Vec::new(),
        },
    );
    let r = recv_job(&rx);
    assert_eq!(r.job, next);
    assert!(matches!(r.body, JobResult::Released));
}

#[test]
fn 手元の列で待っている続きを取り消すと_cancelled_を1件だけ返す() {
    // /w の 1 区切り目と /v の列挙で止める。/w の続きは、間に入った /v の走査の後ろ (手元の列) で待つ
    let (sp, rx, entered, release) = gated_scan_at(&["/w/d00", "/v"]);
    let w = sp.spawn(None, Revision(0), scan_kind());
    entered.recv_timeout(Duration::from_secs(10)).unwrap();
    let v = sp.spawn(None, Revision(0), scan_of("/v"));
    release.send(()).unwrap();
    // /v の列挙で止まっている = /w の続きは手元の列にある
    entered.recv_timeout(Duration::from_secs(10)).unwrap();
    sp.cancel(w);
    release.send(()).unwrap();
    let r = recv_until_terminals(&rx, 2);
    let terminals: Vec<&JobOutput> = r
        .iter()
        .filter(|o| !matches!(o.body, JobResult::ScanBatch { .. }))
        .collect();
    assert!(
        terminals
            .iter()
            .any(|o| o.job == v && matches!(o.body, JobResult::ScanDone { total: 1, .. })),
        "{r:?}"
    );
    let of_w: Vec<_> = r.iter().filter(|o| o.job == w).collect();
    assert!(
        matches!(
            of_w.as_slice(),
            [JobOutput {
                body: JobResult::Failed {
                    err: JobError::Cancelled
                },
                ..
            }]
        ),
        "取り消した続きは Cancelled だけ (束も ScanDone も送らない): {r:?}"
    );
    assert!(
        rx.recv_timeout(Duration::from_millis(200)).is_err(),
        "続きは積まれない"
    );
}

#[test]
fn 走査の最中に_spawner_を手放すと終端は1つで_待ちの上限の内に終わる() {
    // eve-host の `JOIN_WAIT` (1 秒) と同じ値
    const JOIN_WAIT: Duration = Duration::from_secs(1);
    let (sp, rx, entered, release) = gated_scan();
    let id = sp.spawn(None, Revision(0), scan_kind());
    entered.recv_timeout(Duration::from_secs(10)).unwrap();
    // drop がワーカーを待っている間に、止めていた列挙を戻す
    let releaser = thread::spawn(move || {
        thread::sleep(Duration::from_millis(100));
        release.send(()).unwrap();
    });
    let t = Instant::now();
    drop(sp);
    let waited = t.elapsed();
    releaser.join().unwrap();
    assert!(
        waited < JOIN_WAIT,
        "drop は区切りの終わりで終わる: {waited:?}"
    );
    let mut r = Vec::new();
    while let Ok(HostMsg::Job(o)) = rx.recv_timeout(Duration::from_millis(200)) {
        r.push(*o);
    }
    let terminals: Vec<_> = r
        .iter()
        .filter(|o| o.job == id && !matches!(o.body, JobResult::ScanBatch { .. }))
        .collect();
    assert!(
        matches!(
            terminals.as_slice(),
            [JobOutput {
                body: JobResult::Failed {
                    err: JobError::Cancelled
                },
                ..
            }]
        ),
        "drop が取り消した走査の終端は Cancelled の 1 つ: {r:?}"
    );
}
