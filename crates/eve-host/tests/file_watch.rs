//! ファイル監視の組み立て (`file_watcher`) と Host の作業の口 (`HostTasks::spawn_io_task`) を、本番の `ThreadJobSpawner`
//! (実スレッドの `io` のレーン) と実際の FS で確かめる (docs/07 §12.6.2 の段 7、02 §4.4、05 §12.2 の 9)。

use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use eve_core::{
    FsEvent, FsEventKind, FsPath, JobError, JobKind, JobPorts, JobResult, JobSpawner, MetaPurpose,
};
use eve_host::{HostMsg, HostTasks, SystemClock, ThreadJobSpawner, channel, file_watcher};
use eve_testkit::{FixedPaths, MemFs};
use eve_view::Revision;

const WAIT: Duration = Duration::from_secs(10);

fn spawner() -> (
    Rc<ThreadJobSpawner>,
    mpsc::Sender<HostMsg>,
    Receiver<HostMsg>,
) {
    let clock = Arc::new(SystemClock::new());
    let fs = Arc::new(MemFs::new(clock.clone()));
    fs.add_dir("/w");
    let jp = JobPorts {
        stdin: None,
        fs,
        clock,
        paths: Arc::new(FixedPaths::default()),
    };
    let (tx, rx) = channel();
    let s = ThreadJobSpawner::new(jp, tx.clone()).expect("io のワーカー");
    (Rc::new(s), tx, rx)
}

fn next_fs(rx: &Receiver<HostMsg>, what: &str, pred: impl Fn(&FsEvent) -> bool) {
    let end = Instant::now() + WAIT;
    let mut seen = Vec::new();
    loop {
        match rx.recv_timeout(end.saturating_duration_since(Instant::now())) {
            Ok(HostMsg::Fs(ev)) => {
                let hit = pred(&ev);
                seen.push(ev);
                if hit {
                    return;
                }
            }
            Ok(_) => {}
            Err(_) => panic!("{what} が届かない。届いたもの: {seen:?}"),
        }
    }
}

struct Tmp(PathBuf);

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn io_のレーンで登録し_知らせは受信口に届く() {
    let base = std::env::temp_dir().join(format!("eve-host-watch-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).expect("一時ディレクトリ");
    let tmp = Tmp(fs::canonicalize(&base).expect("canonicalize"));
    let dir = {
        let s = tmp.0.to_string_lossy().into_owned();
        PathBuf::from(s.strip_prefix(r"\\?\").map_or(s.clone(), str::to_owned))
    };

    let (spawner, tx, rx) = spawner();
    let mut w = file_watcher(Rc::clone(&spawner) as Rc<dyn HostTasks>, tx);
    let id = w.watch_dir(&FsPath::new(&dir));
    next_fs(&rx, "Started", |e| {
        e.watch == id && e.kind == FsEventKind::Started
    });
    fs::write(dir.join("a.txt"), b"x").expect("書く");
    next_fs(&rx, "Changed", |e| {
        e.watch == id
            && e.kind
                == FsEventKind::Changed {
                    name: OsString::from("a.txt"),
                }
    });
    // 手放すと後片付けの作業を io に積む。spawner を手放すと io のワーカーが作業を走らせ切ってから終わる
    drop(w);
    drop(spawner);
}

#[test]
fn host_の作業は投入順に走り_パニックしてもレーンは続く() {
    let (spawner, _tx, rx) = spawner();
    let ran = Arc::new(AtomicUsize::new(0));
    let r1 = Arc::clone(&ran);
    spawner.spawn_io_task(Box::new(move || {
        assert_eq!(r1.fetch_add(1, Ordering::SeqCst), 0);
    }));
    spawner.spawn_io_task(Box::new(|| panic!("作業の中のパニック (試験)")));
    // パニックの後のジョブも走る (ジョブと作業は同じ io のキューで投入順)
    let job = spawner.spawn(
        None,
        Revision(0),
        JobKind::Metadata {
            path: FsPath::from("/w"),
            purpose: MetaPurpose::ExternalCheck,
            known_ids: Vec::new(),
        },
    );
    let r2 = Arc::clone(&ran);
    spawner.spawn_io_task(Box::new(move || {
        assert_eq!(r2.fetch_add(1, Ordering::SeqCst), 1);
    }));
    let end = Instant::now() + WAIT;
    loop {
        match rx.recv_timeout(end.saturating_duration_since(Instant::now())) {
            Ok(HostMsg::Job(out)) if out.job == job => {
                // ワーカーが生きていて run_job が走った (死んでいれば `Failed { Panic }`)
                assert!(
                    !matches!(
                        out.body,
                        JobResult::Failed {
                            err: JobError::Panic
                        }
                    ),
                    "{:?}",
                    out.body
                );
                break;
            }
            Ok(_) => {}
            Err(_) => panic!("ジョブの結果が届かない"),
        }
    }
    let end = Instant::now() + WAIT;
    while ran.load(Ordering::SeqCst) < 2 {
        assert!(Instant::now() < end, "後の作業が走らない");
        std::thread::sleep(Duration::from_millis(5));
    }
}
