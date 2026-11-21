//! BM-125s: ワークスペースの走査の時間 (docs/09-testing-quality.md §5.2、01 FR-125 AC-3 の参考値) と、走査の間の `io` / `bg` の
//! ジョブの開始の遅れ (03 §12.7.9 の段 4 の完了の条件。S-26 の (4) と同じ測り方)。
//!
//! 本番の `ThreadJobSpawner` (`io` / `bg` の 2 本) と実 FS (Windows は `WinFs`、Unix は `UnixFs`) で、`JobKind::WorkspaceScan`
//! を投入してから `ScanDone` が届くまでを測る (ルートの検出は M31 の段 1 なので、区間はジョブの開始 → 走査の完了)。木は
//! `cargo xtask gen-corpus --workspace large` が作るワークスペースの木 (09 §4。Windows の NTFS を測るときは内蔵の SSD に置く)。
//!
//! 3 つを測る:
//!
//! 1. **走査** (BM-125s): 1 回温めてから `--runs` 回 (既定 5)。中央値と最大値。終わるたびに、集めた塊を `ReleaseCandidates` で
//!    `bg` に渡して解放する (本番と同じ。主スレッドで解放しない)。
//! 2. **区切り 1 回の長さ** (参考): 同じ木を `WorkspaceWalk::step` で直に回し (このスレッドで)、1 区切りの時間の分布。
//!    `bg` のほかのジョブが走査の後ろで待つ時間の上限の目安。
//! 3. **開始の遅れ** (S-26 の (4)): 走査を繰り返している間、20〜80 ms ごとに `io` (`Metadata`) と `bg` (小さな木の
//!    `WorkspaceScan`) の見張りのジョブを交互に投入し、投入からジョブが最初の FS の呼出しをするまでの時間を測る。
//!
//! ```text
//! cargo xtask gen-corpus --workspace large --seed 42 --out <内蔵の SSD のディレクトリ>
//! cargo bench -p eve-host --bench bm_125s -- --tree <出力>/workspace-large [--runs 5] [--probe-secs 20] [--json]
//! cargo bench -p eve-host --bench bm_125s -- --diag --tree <木>   # 診断: 列挙だけ・走査 (既定の除外 / 除外なし) を 3 回ずつ
//! ```
//!
//! **参考値**: 合否を付けない (01 FR-125 AC-3)。基準機の値と OS の再起動の直後の値 (`-ntfs-cold`) は人間が測る。

#![allow(clippy::print_stdout, clippy::print_stderr)]
#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "計測用の数値変換。値は十分に小さい"
)]

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eve_core::{
    Candidate, Clock, DirListing, ExcludeConfig, FileMeta, FileSystem, FsError, FsPath, JobKind,
    JobPorts, JobResult, JobSpawner, MetaPurpose, ReadChunks, SaveOptions, SaveReport, Tick,
    WalkStep, WorkspaceWalk,
};
use eve_host::{HostMsg, SystemClock, ThreadJobSpawner, channel};
use eve_platform::dirs::Dirs;
use eve_platform::path::PathCtx;
use eve_view::Revision;

/// 見張りのジョブのレーン。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lane {
    Io,
    Bg,
}

/// 見張りのジョブの最初の FS の呼出しの時刻を知らせる FS (ほかは実 FS へそのまま渡す)。
struct ProbeFs {
    inner: Arc<dyn FileSystem>,
    /// `io` の見張り (`Metadata`) が調べるパス。
    io_probe: FsPath,
    /// `bg` の見張り (小さな木の走査) の最初の呼出し (ルートから上の `.git` の探索) のパス。
    bg_probe: FsPath,
    started: Mutex<Sender<(Lane, Instant)>>,
}

impl ProbeFs {
    fn mark(&self, p: &FsPath) {
        let lane = if *p == self.io_probe {
            Lane::Io
        } else if *p == self.bg_probe {
            Lane::Bg
        } else {
            return;
        };
        let now = Instant::now();
        let _ = self.started.lock().expect("lock").send((lane, now));
    }
}

impl FileSystem for ProbeFs {
    fn metadata(&self, p: &FsPath) -> Result<FileMeta, FsError> {
        self.mark(p);
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
    fn read_dir(&self, dir: &FsPath, max: usize) -> Result<DirListing, FsError> {
        self.inner.read_dir(dir, max)
    }
}

#[cfg(windows)]
fn real_fs() -> Arc<dyn FileSystem> {
    Arc::new(eve_platform::fs::WinFs::default())
}

#[cfg(unix)]
fn real_fs() -> Arc<dyn FileSystem> {
    Arc::new(eve_platform::unix_fs::UnixFs::default())
}

#[cfg(windows)]
fn paths() -> Arc<dyn eve_core::Paths> {
    Arc::new(eve_platform::dirs::WinPaths::new(
        Dirs::resolve(&|_| None),
        PathCtx {
            cwd: "C:\\".to_owned(),
            home: None,
        },
    ))
}

#[cfg(unix)]
fn paths() -> Arc<dyn eve_core::Paths> {
    Arc::new(eve_platform::dirs::UnixPaths::new(
        Dirs::resolve(&|_| None),
        PathCtx {
            cwd: "/".to_owned(),
            home: None,
        },
        None,
    ))
}

/// `workspace.exclude` の既定 (07 §5.9) と、`.gitignore` を読む・リンクに降りない既定。
fn exclude() -> Arc<ExcludeConfig> {
    Arc::new(ExcludeConfig {
        exclude: ["**/.git", "**/node_modules", "**/target"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
        ..ExcludeConfig::new()
    })
}

struct Opts {
    tree: PathBuf,
    runs: usize,
    probe_secs: u64,
    json: bool,
}

fn parse_opts() -> Result<Opts, String> {
    let mut o = Opts {
        tree: PathBuf::new(),
        runs: 5,
        probe_secs: 20,
        json: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = |name: &str| {
            args.next()
                .ok_or_else(|| format!("{name} の値がありません"))
        };
        match a.as_str() {
            "--tree" => o.tree = PathBuf::from(value("--tree")?),
            "--runs" => {
                o.runs = value("--runs")?
                    .parse()
                    .map_err(|e| format!("--runs: {e}"))?
            }
            "--probe-secs" => {
                o.probe_secs = value("--probe-secs")?
                    .parse()
                    .map_err(|e| format!("--probe-secs: {e}"))?;
            }
            "--json" => o.json = true,
            // cargo bench が渡す
            "--bench" => {}
            other => return Err(format!("知らない引数: {other}")),
        }
    }
    if o.tree.as_os_str().is_empty() {
        return Err("--tree <ワークスペースの木> を指定してください (cargo xtask gen-corpus --workspace large)".to_owned());
    }
    Ok(o)
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let i = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

struct Stats {
    n: usize,
    p50: f64,
    p99: f64,
    max: f64,
    samples: Vec<f64>,
}

fn stats(mut v: Vec<f64>) -> Stats {
    let samples = v.clone();
    v.sort_by(f64::total_cmp);
    Stats {
        n: v.len(),
        p50: pct(&v, 0.5),
        p99: pct(&v, 0.99),
        max: v.last().copied().unwrap_or(f64::NAN),
        samples,
    }
}

fn json_line(id: &str, s: &Stats, extra: &str) -> String {
    let samples: Vec<String> = s.samples.iter().map(|x| format!("{x:.3}")).collect();
    format!(
        "{{\"id\":\"{id}\",\"n\":{},\"p50_ms\":{:.3},\"p99_ms\":{:.3},\"max_ms\":{:.3},\"samples_ms\":[{}]{extra}}}",
        s.n,
        s.p50,
        s.p99,
        s.max,
        samples.join(",")
    )
}

/// 走査 1 回の結果。
struct ScanRun {
    time: Duration,
    total: u64,
    truncated: bool,
    unreadable: u64,
    batches: usize,
}

/// 走査を 1 回投入して `ScanDone` まで待つ。集めた塊は `ReleaseCandidates` で `bg` に渡し、`Released` まで待つ。
fn scan_once(
    sp: &ThreadJobSpawner,
    rx: &Receiver<HostMsg>,
    root: &FsPath,
    generation: u64,
) -> ScanRun {
    let t = Instant::now();
    let id = sp.spawn(
        None,
        Revision(0),
        JobKind::WorkspaceScan {
            root: root.clone(),
            exclude: exclude(),
            generation,
            walk: None,
        },
    );
    let mut chunks: Vec<Arc<[Candidate]>> = Vec::new();
    let mut unreadable = 0;
    let (total, truncated) = loop {
        let HostMsg::Job(r) = rx.recv_timeout(Duration::from_secs(600)).expect("結果") else {
            continue;
        };
        if r.job != id {
            continue;
        }
        match r.body {
            JobResult::ScanBatch {
                chunk,
                unreadable_dirs,
                ..
            } => {
                chunks.push(chunk);
                unreadable = unreadable_dirs;
            }
            JobResult::ScanDone {
                total, truncated, ..
            } => break (total, truncated),
            other => panic!("走査に失敗: {other:?}"),
        }
    };
    let time = t.elapsed();
    let batches = chunks.len();
    let rel = sp.spawn(None, Revision(0), JobKind::ReleaseCandidates { chunks });
    loop {
        if let HostMsg::Job(r) = rx.recv_timeout(Duration::from_secs(60)).expect("解放")
            && r.job == rel
        {
            break;
        }
    }
    ScanRun {
        time,
        total,
        truncated,
        unreadable,
        batches,
    }
}

/// 区切り 1 回の長さ (このスレッドで `WorkspaceWalk::step` を直に回す)。
fn step_times(fs: &dyn FileSystem, root: &FsPath) -> (Stats, Duration) {
    struct Wall(Instant);
    impl Clock for Wall {
        fn now(&self) -> Tick {
            Tick(self.0.elapsed().as_millis() as u64)
        }
    }
    let clock = Wall(Instant::now());
    let mut w = WorkspaceWalk::new(root.clone(), exclude());
    let mut v = Vec::new();
    let all = Instant::now();
    loop {
        let t = Instant::now();
        let s = w.step(fs, &clock);
        let _ = w.take_pending();
        v.push(ms(t.elapsed()));
        match s {
            WalkStep::Yield => {}
            WalkStep::Done => break,
            WalkStep::Failed(e) => panic!("走査に失敗: {:?}", e.kind),
        }
    }
    (stats(v), all.elapsed())
}

/// splitmix64 (見張りの間隔を決める。決定的)。
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// 走査を繰り返す間の、見張りのジョブの開始の遅れ。返すのは (io, bg, 走査の回数, 走査 1 回の時間)。
fn probe_delays(
    sp: &ThreadJobSpawner,
    rx: &Receiver<HostMsg>,
    started: &Receiver<(Lane, Instant)>,
    root: &FsPath,
    probe_root: &FsPath,
    io_probe: &FsPath,
    secs: u64,
) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let mut rng = Rng(42);
    let end = Instant::now() + Duration::from_secs(secs);
    let mut generation = 1000;
    let spawn_scan = |generation: u64| {
        sp.spawn(
            None,
            Revision(0),
            JobKind::WorkspaceScan {
                root: root.clone(),
                exclude: exclude(),
                generation,
                walk: None,
            },
        )
    };
    let mut scan = spawn_scan(generation);
    let mut scan_start = Instant::now();
    let mut chunks: Vec<Arc<[Candidate]>> = Vec::new();
    let mut scan_times = Vec::new();
    let mut waiting: [VecDeque<Instant>; 2] = [VecDeque::new(), VecDeque::new()];
    let (mut io, mut bg) = (Vec::new(), Vec::new());
    let mut next_probe = Instant::now() + Duration::from_millis(20 + rng.next() % 61);
    let mut next_lane = Lane::Io;
    let mut scanning = true;
    while scanning || waiting.iter().any(|q| !q.is_empty()) {
        // 見張りの開始を集める
        while let Ok((lane, at)) = started.try_recv() {
            let q = &mut waiting[lane as usize];
            if let Some(spawned) = q.pop_front() {
                let d = ms(at.saturating_duration_since(spawned));
                match lane {
                    Lane::Io => io.push(d),
                    Lane::Bg => bg.push(d),
                }
            }
        }
        let now = Instant::now();
        if scanning && now >= next_probe {
            waiting[next_lane as usize].push_back(Instant::now());
            match next_lane {
                Lane::Io => {
                    sp.spawn(
                        None,
                        Revision(0),
                        JobKind::Metadata {
                            path: io_probe.clone(),
                            purpose: MetaPurpose::ExternalCheck,
                        },
                    );
                    next_lane = Lane::Bg;
                }
                Lane::Bg => {
                    sp.spawn(
                        None,
                        Revision(0),
                        JobKind::WorkspaceScan {
                            root: probe_root.clone(),
                            exclude: exclude(),
                            generation: 0,
                            walk: None,
                        },
                    );
                    next_lane = Lane::Io;
                }
            }
            next_probe = Instant::now() + Duration::from_millis(20 + rng.next() % 61);
        }
        let wait = next_probe
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(5));
        let Ok(HostMsg::Job(r)) = rx.recv_timeout(wait) else {
            continue;
        };
        if r.job != scan {
            continue;
        }
        match r.body {
            JobResult::ScanBatch { chunk, .. } => chunks.push(chunk),
            JobResult::ScanDone { .. } => {
                scan_times.push(ms(scan_start.elapsed()));
                sp.spawn(
                    None,
                    Revision(0),
                    JobKind::ReleaseCandidates {
                        chunks: std::mem::take(&mut chunks),
                    },
                );
                if Instant::now() >= end {
                    scanning = false;
                } else {
                    generation += 1;
                    scan = spawn_scan(generation);
                    scan_start = Instant::now();
                }
            }
            other => panic!("走査に失敗: {other:?}"),
        }
    }
    (io, bg, scan_times)
}

/// 診断 (`--diag`): 列挙だけの深さ優先 (`read_dir` のみ、全ディレクトリ) と、`workspace.exclude` を空にした走査の時間。
fn diag(fs: &dyn FileSystem, root: &FsPath) {
    fn raw(fs: &dyn FileSystem, dir: &FsPath, n: &mut (u64, u64)) {
        let Ok(l) = fs.read_dir(dir, usize::MAX) else {
            return;
        };
        n.0 += 1;
        for e in l.entries {
            n.1 += 1;
            if e.kind == eve_core::EntryKind::Dir {
                raw(fs, &dir.join(&e.name), n);
            }
        }
    }
    for _ in 0..3 {
        let t = Instant::now();
        let mut n = (0, 0);
        raw(fs, root, &mut n);
        eprintln!(
            "診断: 列挙だけ {:.1} ms (ディレクトリ {}、項目 {})",
            ms(t.elapsed()),
            n.0,
            n.1
        );
        for (label, cfg) in [
            ("既定の除外", exclude()),
            ("workspace.exclude なし", Arc::new(ExcludeConfig::new())),
        ] {
            let t = Instant::now();
            let mut w = WorkspaceWalk::new(root.clone(), cfg);
            while matches!(w.step(fs, &NoClock), WalkStep::Yield) {
                let _ = w.take_pending();
            }
            eprintln!(
                "診断: 走査 ({label}) {:.1} ms (候補 {})",
                ms(t.elapsed()),
                w.found()
            );
        }
    }
}

/// 止まった時計 (診断の走査は数だけで区切る)。
struct NoClock;

impl Clock for NoClock {
    fn now(&self) -> Tick {
        Tick(0)
    }
}

fn main() {
    if std::env::args().any(|a| a == "--diag") {
        let tree = std::env::args()
            .skip_while(|a| a != "--tree")
            .nth(1)
            .expect("--tree");
        diag(&*real_fs(), &FsPath::new(tree));
        return;
    }
    let o = match parse_opts() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("bm_125s: {e}");
            std::process::exit(2);
        }
    };
    let root = FsPath::new(&o.tree);
    // 見張りの小さな木 (木の外、同じ FS)
    let probe_dir = o
        .tree
        .parent()
        .unwrap_or(Path::new("."))
        .join(format!("bm125s-probe-{}", std::process::id()));
    std::fs::create_dir_all(probe_dir.join("bg")).expect("見張りの木");
    std::fs::write(probe_dir.join("bg").join("a.txt"), b"x").expect("見張りの木");
    std::fs::write(probe_dir.join("io.txt"), b"x").expect("見張りの木");
    let probe_root = FsPath::new(probe_dir.join("bg"));
    let io_probe = FsPath::new(probe_dir.join("io.txt"));
    let (stx, srx) = mpsc::channel();
    let inner = real_fs();
    let fs = Arc::new(ProbeFs {
        inner: inner.clone(),
        io_probe: io_probe.clone(),
        bg_probe: probe_root.join(".git"),
        started: Mutex::new(stx),
    });
    let ports = JobPorts {
        stdin: None,
        fs,
        clock: Arc::new(SystemClock::new()),
        paths: paths(),
    };
    let (tx, rx) = channel();
    let sp = ThreadJobSpawner::new(ports, tx).expect("ワーカー");

    // 1. 走査
    let warm = scan_once(&sp, &rx, &root, 1);
    eprintln!(
        "温め: {:.1} ms (候補 {}、打ち切り {}、読めない {}、束 {})",
        ms(warm.time),
        warm.total,
        warm.truncated,
        warm.unreadable,
        warm.batches
    );
    let mut times = Vec::new();
    for i in 0..o.runs {
        let r = scan_once(&sp, &rx, &root, 2 + i as u64);
        assert_eq!(r.total, warm.total, "回によって候補の数が違う");
        times.push(ms(r.time));
    }
    let scan = stats(times);

    // 2. 区切り 1 回の長さ
    let (steps, walk_all) = step_times(&*inner, &root);

    // 3. 開始の遅れ
    let (io, bg, during) =
        probe_delays(&sp, &rx, &srx, &root, &probe_root, &io_probe, o.probe_secs);
    let (io, bg, during) = (stats(io), stats(bg), stats(during));
    drop(sp);
    let _ = std::fs::remove_dir_all(&probe_dir);

    let extra = format!(
        ",\"candidates\":{},\"truncated\":{},\"unreadable_dirs\":{},\"batches\":{}",
        warm.total, warm.truncated, warm.unreadable, warm.batches
    );
    if o.json {
        println!("{}", json_line("BM-125s", &scan, &extra));
        println!(
            "{}",
            json_line(
                "BM-125s-step",
                &Stats {
                    samples: Vec::new(),
                    ..steps
                },
                &format!(",\"walk_ms\":{:.3}", ms(walk_all))
            )
        );
        println!("{}", json_line("BM-125s-io-start", &io, ""));
        println!("{}", json_line("BM-125s-bg-start", &bg, ""));
        println!("{}", json_line("BM-125s-during-probes", &during, ""));
        return;
    }
    println!(
        "BM-125s 走査 (ジョブの開始 → ScanDone): {} 回 中央値 {:.1} ms / 最大 {:.1} ms (候補 {}、打ち切り {}、読めない {}、束 {})",
        scan.n, scan.p50, scan.max, warm.total, warm.truncated, warm.unreadable, warm.batches
    );
    println!(
        "区切り 1 回 (このスレッドで直に): {} 回 p50 {:.2} / p99 {:.2} / 最大 {:.2} ms (走査 {:.1} ms)",
        steps.n,
        steps.p50,
        steps.p99,
        steps.max,
        ms(walk_all)
    );
    println!(
        "開始の遅れ (走査の間): io {} 回 p50 {:.3} / p99 {:.3} / 最大 {:.3} ms、bg {} 回 p50 {:.3} / p99 {:.3} / 最大 {:.3} ms",
        io.n, io.p50, io.p99, io.max, bg.n, bg.p50, bg.p99, bg.max
    );
    println!(
        "見張りの間の走査: {} 回 中央値 {:.1} ms / 最大 {:.1} ms",
        during.n, during.p50, during.max
    );
}
