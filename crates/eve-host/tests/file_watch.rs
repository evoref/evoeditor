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

/// 後片付けが済んだか (Windows: 監視していたディレクトリの祖先を改名できる。Linux: notify のスレッドが無い)。
fn wait_released(root: &std::path::Path) {
    let end = Instant::now() + WAIT;
    #[cfg(windows)]
    {
        let moved = root.with_extension("moved");
        loop {
            match fs::rename(root, &moved) {
                Ok(()) => {
                    let _ = fs::rename(&moved, root);
                    return;
                }
                Err(e) => assert!(Instant::now() < end, "手放した後も祖先を改名できない: {e}"),
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    #[cfg(target_os = "linux")]
    {
        let _ = root;
        loop {
            let n = fs::read_dir("/proc/self/task")
                .expect("/proc/self/task")
                .filter_map(Result::ok)
                .filter(|e| {
                    fs::read_to_string(e.path().join("comm"))
                        .is_ok_and(|c| c.starts_with("notify-rs"))
                })
                .count();
            if n == 0 {
                return;
            }
            assert!(
                Instant::now() < end,
                "手放した後も notify のスレッドが {n} 本残る"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

#[test]
fn io_のレーンで登録し_知らせは受信口に届き_手放すと後片付けする() {
    let base = std::env::temp_dir().join(format!("eve-host-watch-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(base.join("sub")).expect("一時ディレクトリ");
    let tmp = Tmp(fs::canonicalize(&base).expect("canonicalize"));
    let root = {
        let s = tmp.0.to_string_lossy().into_owned();
        PathBuf::from(s.strip_prefix(r"\\?\").map_or(s.clone(), str::to_owned))
    };
    let dir = root.join("sub");

    let (spawner, tx, rx) = spawner();
    let tasks: Rc<dyn HostTasks> = spawner.clone();
    let w = file_watcher(&tasks, tx);
    drop(tasks);
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
    wait_released(&root);
}

#[test]
fn spawner_が無くなった後の要求は捨てる() {
    let (spawner, tx, rx) = spawner();
    let tasks: Rc<dyn HostTasks> = spawner.clone();
    let w = file_watcher(&tasks, tx);
    drop(tasks);
    drop(spawner);
    // 監視は spawner を弱い参照で持つので、spawner は手放した所で終わり、後の要求は走らない (パニックもしない)
    let _ = w.watch_dir(&FsPath::new(std::env::temp_dir()));
    drop(w);
    assert!(
        rx.recv_timeout(Duration::from_millis(300)).is_err(),
        "結果は届かない"
    );
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

/// core の監視 (03 §9.5.1) を、本番の監視 (notify)・`io` のレーン・実際の FS で通す (M33 の段 8)。受信口の知らせを Host の
/// `step` と同じ順 (`tick` → 受け口) で `Editor` へ渡す。
#[cfg(any(windows, target_os = "linux"))]
mod end_to_end {
    use super::*;
    use eve_core::{
        Args, Clock as _, CommandInvocation, DocumentId, Editor, ExternalPending, InvocationSource,
        LoadState, Ports,
    };

    #[cfg(windows)]
    fn native(
        state: &std::path::Path,
    ) -> (Arc<dyn eve_core::FileSystem>, Arc<dyn eve_core::Paths>) {
        (
            Arc::new(eve_platform::fs::WinFs::new(state)),
            Arc::new(eve_platform::dirs::WinPaths::from_process_env().expect("パス")),
        )
    }
    #[cfg(target_os = "linux")]
    fn native(
        state: &std::path::Path,
    ) -> (Arc<dyn eve_core::FileSystem>, Arc<dyn eve_core::Paths>) {
        (
            Arc::new(eve_platform::unix_fs::UnixFs::new(state)),
            Arc::new(eve_platform::dirs::UnixPaths::from_process_env().expect("パス")),
        )
    }

    struct World {
        editor: Editor,
        rx: Receiver<HostMsg>,
        clock: Arc<SystemClock>,
        _spawner: Rc<ThreadJobSpawner>,
    }

    impl World {
        fn deliver(&mut self, wait: Duration) {
            match self.rx.recv_timeout(wait) {
                Ok(HostMsg::Job(out)) => {
                    self.editor.tick(self.clock.now());
                    self.editor.on_job_result(*out);
                }
                Ok(HostMsg::Fs(ev)) => {
                    self.editor.tick(self.clock.now());
                    self.editor.on_fs_event(ev);
                }
                Ok(_) | Err(_) => {}
            }
            self.editor.tick(self.clock.now());
        }

        /// 受信口の知らせを渡しながら、`done` が立つまで待つ。
        fn pump_until(&mut self, what: &str, done: impl Fn(&Editor) -> bool) {
            let end = Instant::now() + WAIT;
            loop {
                self.editor.tick(self.clock.now());
                if done(&self.editor) {
                    return;
                }
                assert!(Instant::now() < end, "{what} にならない");
                self.deliver(Duration::from_millis(10));
            }
        }

        /// 決まった時間だけ知らせを渡し続ける (起きないことを確かめる)。
        fn pump_for(&mut self, d: Duration) {
            let end = Instant::now() + d;
            while Instant::now() < end {
                self.deliver(Duration::from_millis(10));
            }
        }

        fn run(&mut self, id: &str, args: Args) {
            self.editor
                .execute(CommandInvocation::new(id, InvocationSource::Keymap).with_args(args));
        }
    }

    fn ready(e: &Editor, d: DocumentId) -> bool {
        e.state()
            .document(d)
            .is_some_and(|x| x.load_state == LoadState::Ready && x.disk_stamp.is_some())
    }

    fn pending(e: &Editor, d: DocumentId) -> Option<ExternalPending> {
        e.state().document(d).and_then(|x| x.external_pending)
    }

    fn text(e: &Editor, d: DocumentId) -> String {
        e.state()
            .document(d)
            .map(eve_core::Document::to_string_lossy)
            .unwrap_or_default()
    }

    // verifies: AC-FR-117-7, AC-FR-117-8
    #[test]
    fn 実際の監視で外部の書込を知らせ_表示中の文書は読み直し_自分の保存は知らせない() {
        let base = std::env::temp_dir().join(format!("eve-host-watch-e2e-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("w")).expect("一時ディレクトリ");
        let tmp = Tmp(fs::canonicalize(&base).expect("canonicalize"));
        let root = {
            let s = tmp.0.to_string_lossy().into_owned();
            PathBuf::from(s.strip_prefix(r"\\?\").map_or(s.clone(), str::to_owned))
        };
        let a = root.join("w").join("a.txt");
        let b = root.join("w").join("b.txt");
        fs::write(&a, b"a\n").expect("a");
        fs::write(&b, b"b\n").expect("b");

        let clock = Arc::new(SystemClock::new());
        let (fsys, paths) = native(&root.join("state"));
        let (tx, rx) = channel();
        let spawner = Rc::new(
            ThreadJobSpawner::new(
                JobPorts {
                    stdin: None,
                    fs: fsys.clone(),
                    clock: clock.clone(),
                    paths: paths.clone(),
                },
                tx.clone(),
            )
            .expect("io のワーカー"),
        );
        let tasks: Rc<dyn HostTasks> = spawner.clone();
        let watcher = file_watcher(&tasks, tx);
        let editor = Editor::new(
            Ports {
                watcher: Some(watcher),
                fs: fsys,
                clock: clock.clone(),
                jobs: spawner.clone(),
                paths,
            },
            eve_view::Capabilities::default(),
        );
        let mut w = World {
            editor,
            rx,
            clock,
            _spawner: spawner,
        };
        w.editor
            .open_initial(Some(a.to_str().expect("UTF-8")), false);
        let da = w.editor.state().active_document().expect("a");
        w.pump_until("a を開く", |e| ready(e, da));
        w.run(
            "file.open",
            Args::new().with_str("path", b.to_str().expect("UTF-8")),
        );
        w.pump_until("b を開く", |e| {
            e.state()
                .active_document()
                .is_some_and(|d| d != da && ready(e, d))
        });
        let db = w.editor.state().active_document().expect("b");
        // 監視の登録 (io の作業) が済むまで待つ
        w.pump_for(Duration::from_millis(300));

        // 表示していない a を外部で書く: 印が付く (読み直さない)
        fs::write(&a, b"changed outside\n").expect("a を書く");
        w.pump_until("a の印", |e| {
            matches!(pending(e, da), Some(ExternalPending::Modified(_)))
        });
        assert_eq!(text(&w.editor, da), "a\n");

        // 自分の保存 (原子的保存の一時ファイルと置換) では印を付けない
        w.run("edit.insert_text", Args::new().with_str("text", "x"));
        w.run("file.save", Args::new());
        w.pump_until("b の保存", |e| {
            e.state().document(db).is_some_and(|x| !x.is_dirty())
        });
        w.pump_for(Duration::from_millis(600));
        assert_eq!(pending(&w.editor, db), None, "自分の保存");
        assert_eq!(fs::read(&b).expect("b"), b"xb\n");

        // 表示中の未編集の b を外部で書く: 確認なしで読み直す
        fs::write(&b, b"from outside\n").expect("b を書く");
        w.pump_until("b の読み直し", |e| text(e, db) == "from outside\n");
        assert_eq!(pending(&w.editor, db), None);
        assert_eq!(w.editor.state().overlay_kind(), "");
    }
}
