//! ファイル監視の本番の実装 (`NotifyWatcher`) を実際の FS で確かめる (docs/07 §12.6.2 の段 7、05 §12.2、02 §4.4)。
//!
//! io の口は、渡された作業をその場で走らせる (本番は Host の `io` のレーン。ここでは試験のスレッドが `io` の代わり)。
//! 知らせの口はチャネルへ送る。どの試験も watcher (notify のスレッド) を作るので、プロセスの中で 1 つずつ走らせる
//! (inotify の fd・スレッドを数える試験が、ほかの試験の watcher を数えないため)。
//!
//! 前提: 一時ディレクトリが監視できる FS (Windows は `%TEMP%` の NTFS、Linux は ext4・tmpfs など) にあること。
//! 監視しない FS の試験は、WSL の `/mnt/c` (9p) があるときだけ走る (Windows のネットワークの共有は `EVE_TEST_UNWATCHABLE_DIR`
//! を置いたときだけ。09 §8.2 の手の確認で確かめる)。

#![allow(clippy::print_stderr)]

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use eve_core::{FileWatcher, FsEvent, FsEventKind, FsPath, WatchError, WatchId};
use eve_platform::watch::{FsSink, IoSpawn, NotifyWatcher};

/// 知らせを待つ上限 (CI の遅い機械でも越えない長さ。届くはずの知らせだけをこの長さまで待つ)。
const WAIT: Duration = Duration::from_secs(10);
/// 「届かない」を確かめる間 (待ち合わせの 100 ms より十分長い)。
const QUIET: Duration = Duration::from_millis(500);

static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 試験ごとの一時ディレクトリ (終わりに消す)。
struct Tmp(PathBuf);

impl Tmp {
    fn new(name: &str) -> Tmp {
        let base = std::env::temp_dir().join(format!("eve-watch-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).expect("一時ディレクトリ");
        // 監視は canonical のパスの親を頼む (03 §9.5.1 の WATCH-1)。8.3 の短い名前 (`HIROYU~1`) のままにしない
        let base = fs::canonicalize(&base).expect("canonicalize");
        Tmp(strip_verbatim(base))
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Windows の `canonicalize` の `\\?\C:\…` を `C:\…` にする (core の文書のパスの形に合わせる)。
fn strip_verbatim(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy().into_owned();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => p,
    }
}

/// io の口をその場で走らせる watcher と、知らせの受け口。`gate` を握っている間、知らせの口 (notify のスレッド) は待つ。
fn watcher_with_gate(gate: Arc<Mutex<()>>) -> (NotifyWatcher, Receiver<FsEvent>) {
    let (tx, rx) = mpsc::channel::<FsEvent>();
    let io: IoSpawn = Box::new(|task| task());
    let sink: FsSink = Arc::new(move |ev| {
        let _g = gate.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = tx.send(ev);
    });
    (NotifyWatcher::new(io, sink), rx)
}

fn watcher() -> (NotifyWatcher, Receiver<FsEvent>) {
    watcher_with_gate(Arc::new(Mutex::new(())))
}

/// `pred` に合う知らせが届くまで待ち、それまでに届いたもの (合ったものを含む) を返す。
fn wait_for(rx: &Receiver<FsEvent>, what: &str, pred: impl Fn(&FsEvent) -> bool) -> Vec<FsEvent> {
    let end = Instant::now() + WAIT;
    let mut seen = Vec::new();
    loop {
        let left = end.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(ev) => {
                let hit = pred(&ev);
                seen.push(ev);
                if hit {
                    return seen;
                }
            }
            Err(_) => panic!("{what} が {WAIT:?} の内に届かない。届いたもの: {seen:?}"),
        }
    }
}

/// `QUIET` の間に届いたもの。
fn drain_quiet(rx: &Receiver<FsEvent>) -> Vec<FsEvent> {
    let end = Instant::now() + QUIET;
    let mut seen = Vec::new();
    while let Ok(ev) = rx.recv_timeout(end.saturating_duration_since(Instant::now())) {
        seen.push(ev);
    }
    seen
}

fn started(rx: &Receiver<FsEvent>, id: WatchId) {
    let seen = wait_for(rx, "Started", |e| {
        e.watch == id && e.kind != FsEventKind::Rescan
    });
    let last = seen.last().expect("知らせ");
    assert_eq!(last.kind, FsEventKind::Started, "登録の結果: {seen:?}");
}

fn is_changed(e: &FsEvent, id: WatchId, name: &str) -> bool {
    e.watch == id
        && e.kind
            == FsEventKind::Changed {
                name: OsString::from(name),
            }
}

fn fsp(p: &Path) -> FsPath {
    FsPath::new(p)
}

#[test]
fn 登録して直下の変化を名前で知らせる() {
    let _s = serial();
    let tmp = Tmp::new("changed");
    let (w, rx) = watcher();
    let id = w.watch_dir(&fsp(&tmp.0));
    started(&rx, id);

    fs::write(tmp.0.join("a.txt"), b"one").expect("書く");
    wait_for(&rx, "a.txt の Changed", |e| is_changed(e, id, "a.txt"));

    // 改名は元と先の両方
    drain_quiet(&rx);
    fs::rename(tmp.0.join("a.txt"), tmp.0.join("b.txt")).expect("改名");
    let seen = wait_for(&rx, "b.txt の Changed", |e| is_changed(e, id, "b.txt"));
    let seen: Vec<FsEvent> = seen.into_iter().chain(drain_quiet(&rx)).collect();
    assert!(
        seen.iter().any(|e| is_changed(e, id, "a.txt")),
        "改名の元も届く: {seen:?}"
    );

    // 削除
    fs::remove_file(tmp.0.join("b.txt")).expect("削除");
    wait_for(&rx, "削除の Changed", |e| is_changed(e, id, "b.txt"));
}

#[test]
fn 読むだけの開閉と孫の変化は知らせない() {
    let _s = serial();
    let tmp = Tmp::new("quiet");
    fs::write(tmp.0.join("r.txt"), b"read me").expect("書く");
    fs::create_dir(tmp.0.join("sub")).expect("サブディレクトリ");
    let (w, rx) = watcher();
    let id = w.watch_dir(&fsp(&tmp.0));
    started(&rx, id);
    drain_quiet(&rx);

    for _ in 0..5 {
        let _ = fs::read(tmp.0.join("r.txt")).expect("読む");
    }
    let seen = drain_quiet(&rx);
    assert_eq!(
        seen,
        Vec::<FsEvent>::new(),
        "読むだけでは知らせない: {seen:?}"
    );

    // 非再帰: 孫のファイルの変化は名前で知らせない (Windows はサブディレクトリの更新時刻の変化を `sub` で届け得る)
    fs::write(tmp.0.join("sub").join("deep.txt"), b"x").expect("孫を書く");
    let seen = drain_quiet(&rx);
    assert!(
        !seen.iter().any(|e| is_changed(e, id, "deep.txt")),
        "孫は知らせない: {seen:?}"
    );
}

#[test]
fn 解除の後は知らせない() {
    let _s = serial();
    let tmp = Tmp::new("unwatch");
    let (w, rx) = watcher();
    let id = w.watch_dir(&fsp(&tmp.0));
    started(&rx, id);
    w.unwatch(id);
    fs::write(tmp.0.join("a.txt"), b"x").expect("書く");
    let seen = drain_quiet(&rx);
    assert_eq!(seen, Vec::<FsEvent>::new(), "外した後: {seen:?}");

    // 同じディレクトリを頼み直せば、また知らせる
    let id2 = w.watch_dir(&fsp(&tmp.0));
    assert_ne!(id, id2, "識別は使い回さない");
    started(&rx, id2);
    fs::write(tmp.0.join("a.txt"), b"y").expect("書く");
    wait_for(&rx, "頼み直した後の Changed", |e| {
        is_changed(e, id2, "a.txt")
    });
}

#[test]
fn 同じディレクトリの二つの要求は片方を外しても残る() {
    let _s = serial();
    let tmp = Tmp::new("shared");
    let (w, rx) = watcher();
    let a = w.watch_dir(&fsp(&tmp.0));
    let b = w.watch_dir(&fsp(&tmp.0));
    started(&rx, a);
    started(&rx, b);
    w.unwatch(a);
    fs::write(tmp.0.join("a.txt"), b"x").expect("書く");
    let seen = wait_for(&rx, "残った要求の Changed", |e| {
        is_changed(e, b, "a.txt")
    });
    assert!(
        seen.iter().all(|e| e.watch != a),
        "外した要求には送らない: {seen:?}"
    );
}

#[test]
fn 無いディレクトリは_not_found() {
    let _s = serial();
    let tmp = Tmp::new("missing");
    let (w, rx) = watcher();
    let id = w.watch_dir(&fsp(&tmp.0.join("no-such-dir")));
    let seen = wait_for(&rx, "Failed", |e| e.watch == id);
    assert_eq!(
        seen.last().map(|e| &e.kind),
        Some(&FsEventKind::Failed {
            err: WatchError::NotFound
        })
    );
}

#[test]
fn 監視中のディレクトリの削除は失効() {
    let _s = serial();
    let tmp = Tmp::new("lost");
    let dir = tmp.0.join("watched");
    fs::create_dir(&dir).expect("作る");
    let (w, rx) = watcher();
    let id = w.watch_dir(&fsp(&dir));
    started(&rx, id);
    fs::remove_dir(&dir).expect("監視中のディレクトリを消せる (05 §12.1)");
    wait_for(&rx, "Lost", |e| {
        e.watch == id && e.kind == FsEventKind::Lost
    });

    // 作り直したディレクトリを同じパスで頼み直すと、張り直して知らせる (03 §9.5.1 の WATCH-1)
    fs::create_dir(&dir).expect("作り直す");
    w.unwatch(id);
    let id2 = w.watch_dir(&fsp(&dir));
    started(&rx, id2);
    fs::write(dir.join("a.txt"), b"x").expect("書く");
    wait_for(&rx, "張り直した後の Changed", |e| {
        is_changed(e, id2, "a.txt")
    });
}

/// 監視のスレッドが知らせを送れない間に 1 つのディレクトリで多数のファイルを書くと、取りこぼしの恐れ (`Rescan`) が届く
/// (Windows の 0 バイトの完了・`ERROR_NOTIFY_ENUM_DIR`、Linux の `IN_Q_OVERFLOW`。02 §13.7 の段 7 の試験)。
#[test]
fn イベントのあふれは_rescan() {
    let _s = serial();
    let tmp = Tmp::new("overflow");
    let gate = Arc::new(Mutex::new(()));
    let (w, rx) = watcher_with_gate(Arc::clone(&gate));
    let id = w.watch_dir(&fsp(&tmp.0));
    started(&rx, id);
    // Linux は待ち行列の長さ (`max_queued_events`) を超える数を書く (1 ファイルで少なくとも 2 つ (作成・書いた後の閉じ)
    // 積む)。Windows は notify の読取の受け皿 (64 KiB) と OS の受け皿を超える数 (1 ファイルで 30 バイト以上の記録が 2 つ以上)
    let files = overflow_files();
    if files == 0 {
        eprintln!("待ち行列が長すぎてあふれさせられないので飛ばす");
        return;
    }
    {
        // 知らせの口を止めて (notify のスレッドが読まない)、OS の待ち行列をあふれさせる
        let _held = gate.lock().unwrap_or_else(PoisonError::into_inner);
        for i in 0..files {
            fs::write(tmp.0.join(format!("f{i:05}.txt")), b"x").expect("書く");
        }
    }
    wait_for(&rx, "Rescan", |e| {
        e.watch == id && e.kind == FsEventKind::Rescan
    });
}

/// あふれさせるのに書くファイルの数。あふれさせられないほど待ち行列が長ければ 0。
fn overflow_files() -> usize {
    /// 試験で書くファイルの数の上限 (これを超える長さの待ち行列の機械では飛ばす)。
    const MAX_FILES: usize = 60_000;
    #[cfg(target_os = "linux")]
    {
        let queued: usize = fs::read_to_string("/proc/sys/fs/inotify/max_queued_events")
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(16_384);
        let n = queued / 2 + 1_000;
        if n > MAX_FILES { 0 } else { n }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = MAX_FILES;
        8_000
    }
}

/// Windows の入れ子の監視 (親と子のディレクトリの両方): 子の中の保存で、親の監視が子のディレクトリの更新時刻の変化を
/// 子のパスで届けても、子の監視には `Rescan` も `Lost` も送らない (05 §12.2 の 10)。Linux でも同じ結果になる。
#[test]
fn 入れ子の監視で子の中の保存は子に_rescan_を出さない() {
    let _s = serial();
    let tmp = Tmp::new("nested");
    let sub = tmp.0.join("sub");
    fs::create_dir(&sub).expect("作る");
    let (w, rx) = watcher();
    let parent = w.watch_dir(&fsp(&tmp.0));
    started(&rx, parent);
    let child = w.watch_dir(&fsp(&sub));
    started(&rx, child);
    drain_quiet(&rx);
    // 原子的保存と同じ形 (一時ファイルに書いて改名で置き換える) を何度か
    for i in 0..3 {
        let t = sub.join(format!(".a.txt.tmp{i}"));
        fs::write(&t, format!("v{i}")).expect("書く");
        fs::rename(&t, sub.join("a.txt")).expect("置き換える");
    }
    let seen = wait_for(&rx, "子の Changed", |e| is_changed(e, child, "a.txt"));
    let seen: Vec<FsEvent> = seen.into_iter().chain(drain_quiet(&rx)).collect();
    assert!(
        !seen
            .iter()
            .any(|e| e.watch == child && matches!(e.kind, FsEventKind::Rescan | FsEventKind::Lost)),
        "子の監視に Rescan / Lost を出さない: {seen:?}"
    );
}

/// 読むだけの開閉 (`IN_OPEN`・`IN_CLOSE_NOWRITE`) を購読しない (05 §12.2 の 8)。inotify の fd の `fdinfo` の `mask:` で見る。
#[cfg(target_os = "linux")]
#[test]
fn 読むだけの開閉は購読しない() {
    /// `IN_CLOSE_WRITE`・`IN_CLOSE_NOWRITE`・`IN_OPEN` (`linux/inotify.h`)。
    const IN_CLOSE_WRITE: u32 = 0x8;
    const IN_CLOSE_NOWRITE: u32 = 0x10;
    const IN_OPEN: u32 = 0x20;
    let _s = serial();
    let tmp = Tmp::new("mask");
    let (w, rx) = watcher();
    let id = w.watch_dir(&fsp(&tmp.0));
    started(&rx, id);
    let masks: Vec<u32> = fs::read_dir("/proc/self/fd")
        .expect("/proc/self/fd")
        .filter_map(Result::ok)
        .filter(|e| fs::read_link(e.path()).is_ok_and(|l| l.to_string_lossy().contains("inotify")))
        .filter_map(|e| {
            fs::read_to_string(format!(
                "/proc/self/fdinfo/{}",
                e.file_name().to_string_lossy()
            ))
            .ok()
        })
        .flat_map(|info| {
            info.lines()
                .filter_map(|l| l.split_whitespace().find_map(|f| f.strip_prefix("mask:")))
                .filter_map(|m| u32::from_str_radix(m, 16).ok())
                .collect::<Vec<u32>>()
        })
        .collect();
    assert!(!masks.is_empty(), "inotify の監視の mask が読めない");
    for m in masks {
        assert_eq!(m & (IN_OPEN | IN_CLOSE_NOWRITE), 0, "mask {m:#x}");
        assert_ne!(
            m & IN_CLOSE_WRITE,
            0,
            "書いた後の閉じは購読する: mask {m:#x}"
        );
    }
}

/// 後片付け: watcher を手放すと notify のスレッドが止まり、監視のハンドルが閉じる (Windows は監視中のディレクトリの
/// 祖先を改名できるようになる。05 §12.1)。
#[cfg(windows)]
#[test]
fn 手放すと監視のハンドルを閉じる() {
    let _s = serial();
    let tmp = Tmp::new("release");
    let root = tmp.0.join("root");
    let sub = root.join("sub");
    fs::create_dir_all(&sub).expect("作る");
    let (w, rx) = watcher();
    let id = w.watch_dir(&fsp(&sub));
    started(&rx, id);
    assert!(
        fs::rename(&root, tmp.0.join("moved")).is_err(),
        "監視中は祖先を改名できない (05 §12.1 の既知の制約)"
    );
    drop(w);
    let end = Instant::now() + WAIT;
    loop {
        match fs::rename(&root, tmp.0.join("moved")) {
            Ok(()) => break,
            Err(e) if Instant::now() < end => {
                let _ = e;
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => panic!("手放した後も祖先を改名できない: {e}"),
        }
    }
}

/// 後片付け: watcher を手放すと notify のスレッドが止まり、inotify の fd が閉じる。
#[cfg(target_os = "linux")]
#[test]
fn 手放すと監視のスレッドと_fd_を閉じる() {
    fn inotify_fds() -> usize {
        fs::read_dir("/proc/self/fd")
            .expect("/proc/self/fd")
            .filter_map(Result::ok)
            .filter(|e| {
                fs::read_link(e.path()).is_ok_and(|l| l.to_string_lossy().contains("inotify"))
            })
            .count()
    }
    fn notify_threads() -> usize {
        fs::read_dir("/proc/self/task")
            .expect("/proc/self/task")
            .filter_map(Result::ok)
            .filter(|e| {
                fs::read_to_string(e.path().join("comm")).is_ok_and(|c| c.starts_with("notify-rs"))
            })
            .count()
    }
    fn settle(what: &str, want: usize, f: fn() -> usize) {
        let end = Instant::now() + WAIT;
        while f() != want {
            assert!(Instant::now() < end, "{what}: {} のまま (期待 {want})", f());
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    let _s = serial();
    let tmp = Tmp::new("release");
    // 前の試験の watcher のスレッドが止まり切るのを待つ (止まるのは手放した後で、待たずに戻る)
    settle("前の試験の inotify の fd", 0, inotify_fds);
    settle("前の試験の notify のスレッド", 0, notify_threads);
    let (fds, threads) = (0, 0);
    let (w, rx) = watcher();
    let id = w.watch_dir(&fsp(&tmp.0));
    started(&rx, id);
    assert_eq!(inotify_fds(), fds + 1, "watcher はプロセスに 1 つ");
    assert_eq!(notify_threads(), threads + 1);
    // 2 つ目の登録で watcher を増やさない (05 §12.2 の 5)
    let sub = tmp.0.join("sub");
    fs::create_dir(&sub).expect("作る");
    let id2 = w.watch_dir(&fsp(&sub));
    started(&rx, id2);
    assert_eq!(inotify_fds(), fds + 1);
    drop(w);
    settle("inotify の fd", fds, inotify_fds);
    settle("notify のスレッド", threads, notify_threads);
}

/// 監視しない FS (05 §12.2 の 4): WSL の `/mnt/c` (9p) は登録を `Unsupported` で失敗させる (`watch()` は成功するのに
/// イベントが来ない FS を、監視したつもりにならない)。
#[cfg(target_os = "linux")]
#[test]
fn wsl_の_drvfs_は監視しない() {
    let _s = serial();
    let dir = Path::new("/mnt/c");
    if std::env::var_os("WSL_DISTRO_NAME").is_none() || !dir.is_dir() {
        eprintln!("WSL の /mnt/c が無いので飛ばす");
        return;
    }
    let (w, rx) = watcher();
    let id = w.watch_dir(&fsp(dir));
    let seen = wait_for(&rx, "Failed", |e| e.watch == id);
    assert!(
        matches!(
            &seen.last().expect("知らせ").kind,
            FsEventKind::Failed {
                err: WatchError::Unsupported { .. }
            }
        ),
        "{seen:?}"
    );
}

/// 監視しない FS (05 §12.2 の 4): `EVE_TEST_UNWATCHABLE_DIR` (ネットワークの共有・FAT のドライブなど) を置いたときだけ。
#[test]
fn 置いた監視しない_fs_は_unsupported() {
    let _s = serial();
    let Some(dir) = std::env::var_os("EVE_TEST_UNWATCHABLE_DIR") else {
        return;
    };
    let (w, rx) = watcher();
    let id = w.watch_dir(&FsPath::new(PathBuf::from(dir)));
    let seen = wait_for(&rx, "Failed", |e| e.watch == id);
    assert!(
        matches!(
            &seen.last().expect("知らせ").kind,
            FsEventKind::Failed {
                err: WatchError::Unsupported { .. }
            }
        ),
        "{seen:?}"
    );
}
