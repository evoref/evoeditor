//! 保存の途中で強制終了 (Windows は `TerminateProcess`、Unix は `SIGKILL`) したときのディスクの状態と、次に開いたときの報告 (docs/09 §3.6 (6)、05 §4.4 手順 10、
//! §4.6、02 §4.5「中断の痕跡の写像」、01 FR-105 AC-1 / NFR-301)。補助プロセス `eve-test-helpers save` を `EVE_TEST_HOOK=<切断点>` で
//! 起動し、切断点で後始末なしに終了させる。

#![cfg(any(windows, target_os = "linux"))]
#![allow(clippy::result_large_err)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use eve_core::{
    CancelToken, Clock, FsPath, InterruptedSave, JobKind, JobPorts, JobRequest, JobResult,
    Leftover, LeftoverKind, LeftoverOp, OpenOptions, SaveMethod, Tick, run_job,
};
use eve_platform::dirs::Dirs;
#[cfg(unix)]
use eve_platform::dirs::UnixPaths as NativePaths;
#[cfg(windows)]
use eve_platform::dirs::WinPaths as NativePaths;
#[cfg(windows)]
use eve_platform::fs::WinFs as NativeFs;
use eve_platform::path::PathCtx;
#[cfg(unix)]
use eve_platform::unix_fs::UnixFs as NativeFs;
use eve_view::{JobId, Revision};

const EXE: &str = env!("CARGO_BIN_EXE_eve-test-helpers");

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> Tick {
        Tick(0)
    }
}

struct Env {
    root: PathBuf,
    state: PathBuf,
}

/// 保存の結果 (終了コード)。
struct Run {
    code: Option<i32>,
}

impl Env {
    fn new(tag: &str) -> Env {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let root = std::env::temp_dir().join(format!("eve-kill-{tag}-{}-{n}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        Env {
            state: root.join("state"),
            root,
        }
    }

    fn file(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// 補助プロセスで保存する。`hook` を指すと、その切断点で強制終了する。
    fn save(&self, name: &str, mode: &str, content: &[u8], hook: Option<&str>) -> Run {
        let content_file = self.root.join("content.bin");
        fs::write(&content_file, content).unwrap();
        let mut cmd = Command::new(EXE);
        cmd.args(["save"])
            .arg(&self.state)
            .arg(self.file(name))
            .arg(mode)
            .arg(&content_file);
        match hook {
            Some(h) => cmd.env("EVE_TEST_HOOK", h),
            None => cmd.env_remove("EVE_TEST_HOOK"),
        };
        let st = cmd.status().expect("補助プロセスを起動できない");
        // Unix の SIGKILL は終了コードが無いので、Windows の TerminateProcess(137) と同じく 137 (128 + 9) に写す
        #[cfg(unix)]
        let code = {
            use std::os::unix::process::ExitStatusExt;
            st.code().or_else(|| st.signal().map(|s| 128 + s))
        };
        #[cfg(windows)]
        let code = st.code();
        Run { code }
    }

    fn fs(&self) -> NativeFs {
        NativeFs::new(&self.state)
    }

    /// 環境変数を読まない (開発者の設定に依らない) `Paths`。
    fn paths(&self) -> NativePaths {
        let ctx = PathCtx {
            cwd: self.root.to_string_lossy().into_owned(),
            home: None,
        };
        #[cfg(windows)]
        return NativePaths::new(Dirs::resolve(&|_| None), ctx);
        #[cfg(unix)]
        return NativePaths::new(Dirs::resolve_unix(&|_| None, None), ctx, None);
    }

    /// 開くジョブの結果から、中断の痕跡の報告を得る。
    fn open_report(&self, name: &str) -> (Option<InterruptedSave>, bool) {
        let ports = JobPorts {
            fs: Arc::new(self.fs()),
            clock: Arc::new(FixedClock),
            paths: Arc::new(self.paths()),
        };
        let req = JobRequest {
            id: JobId(1),
            doc: None,
            base: Revision(0),
            kind: JobKind::Open {
                path: FsPath::new(self.file(name)),
                opts: OpenOptions::default(),
            },
        };
        let mut out = Vec::new();
        run_job(&req, &ports, &CancelToken::new(), &mut |r| out.push(r));
        match out.pop().expect("結果がある") {
            JobResult::Complete {
                interrupted_save, ..
            } => (interrupted_save, true),
            JobResult::NewFile {
                interrupted_save, ..
            } => (interrupted_save, false),
            other => panic!("{other:?}"),
        }
    }

    fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(&self.root)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "state" && n != "content.bin")
            .collect();
        v.sort();
        v
    }

    /// 状態ディレクトリの `backup/` の中身。
    fn backups(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = fs::read_dir(self.state.join("backup"))
            .map(|rd| rd.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        v.sort();
        v
    }

    fn flags(&self) -> usize {
        let base = self.state.join("recovery").join("inflight");
        fs::read_dir(base)
            .map(|rd| {
                rd.flatten()
                    .map(|d| fs::read_dir(d.path()).map(|i| i.count()).unwrap_or(0))
                    .sum()
            })
            .unwrap_or(0)
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

const ORIG: &[u8] = b"original content of the file\n";
const NEW: &[u8] = b"NEW content that is somewhat longer than the original one\n";

fn killed(r: &Run) {
    assert_eq!(
        r.code,
        Some(137),
        "切断点で強制終了した (終了コード {:?})",
        r.code
    );
}

// ---- 原子的保存の切断点 (i) ----

#[test]
fn 補助プロセスは切断点なしなら保存できる() {
    let e = Env::new("plain");
    fs::write(e.file("a.txt"), ORIG).unwrap();
    assert_eq!(e.save("a.txt", "atomic", NEW, None).code, Some(0));
    assert_eq!(fs::read(e.file("a.txt")).unwrap(), NEW);
    assert_eq!(e.save("a.txt", "inplace", ORIG, None).code, Some(0));
    assert_eq!(fs::read(e.file("a.txt")).unwrap(), ORIG);
}

// verifies: AC-FR-105-1, AC-NFR-301-1
#[test]
fn 原子的保存の切断点では_t_は無傷で一時ファイルが残り_開いても報告しない() {
    for hook in [
        "kill_after_tmp_alloc",
        "kill_after_tmp_write",
        "kill_after_flush",
        "kill_before_rename",
    ] {
        let e = Env::new("atomic");
        fs::write(e.file("a.txt"), ORIG).unwrap();
        killed(&e.save("a.txt", "atomic", NEW, Some(hook)));
        assert_eq!(
            fs::read(e.file("a.txt")).unwrap(),
            ORIG,
            "{hook}: T は保存前とバイト一致"
        );
        let names = e.names();
        assert!(
            names.iter().any(|n| n.starts_with(".a.txt.eve-tmp-")),
            "{hook}: 一時ファイルが残る {names:?}"
        );
        assert_eq!(e.flags(), 1, "{hook}");
        let (report, existed) = e.open_report("a.txt");
        assert!(existed);
        assert!(
            report.is_none(),
            "{hook}: T は完全な保存前の版なので報告しない"
        );
        assert_eq!(
            e.flags(),
            1,
            "{hook}: 一時ファイルが実在するのでフラグは残る (v0.2 の走査の手掛かり)"
        );
        // 続けて保存しても、そのフラグ (一時ファイルが実在する記録) は残る。報告は繰り返さない
        assert_eq!(e.save("a.txt", "atomic", NEW, None).code, Some(0));
        assert_eq!(fs::read(e.file("a.txt")).unwrap(), NEW);
        assert!(e.open_report("a.txt").0.is_none());
    }
}

// verifies: AC-FR-105-1, AC-NFR-301-1
// 置換の途中の状態 (`kill_after_replace_begin`) は Windows の置換 (T を退避名に移してから一時ファイルを移す) だけにある。
// Unix は rename 1 回で置き換わるので、この状態が無い (05 §4.3)
#[cfg(windows)]
#[test]
fn 置換の途中の状態では_t_が無く退避名に元が完全に残り_開くと案内する() {
    let e = Env::new("replace-begin");
    fs::write(e.file("a.txt"), ORIG).unwrap();
    killed(&e.save("a.txt", "atomic", NEW, Some("kill_after_replace_begin")));
    assert!(!e.file("a.txt").exists(), "T が無い");
    let bak = e
        .names()
        .into_iter()
        .find(|n| n.starts_with(".a.txt.eve-bak-"))
        .expect("退避名");
    assert_eq!(
        fs::read(e.file(&bak)).unwrap(),
        ORIG,
        "退避名に元の内容が完全に残る"
    );
    let (report, existed) = e.open_report("a.txt");
    assert!(
        !existed,
        "T が無いので空の新規として開く (黙って開かず、場所を示す)"
    );
    let r = report.expect("msg.open.interrupted_save");
    assert_eq!((r.method, r.complete), (SaveMethod::Atomic, true));
    assert_eq!(r.backup.unwrap().as_path(), e.file(&bak));
    assert!(r.tmp.is_some(), "書きかけの一時ファイルの場所も示す");
}

// ---- 新規ファイルの保存の中断 (v) ----

#[test]
fn 新規ファイルの保存の中断では_t_が作られず一時ファイルが残る() {
    for hook in ["kill_after_tmp_write", "kill_before_rename"] {
        let e = Env::new("newfile");
        killed(&e.save("n.txt", "atomic", NEW, Some(hook)));
        assert!(!e.file("n.txt").exists(), "{hook}");
        assert!(
            e.names().iter().any(|n| n.starts_with(".n.txt.eve-tmp-")),
            "{hook}"
        );
        let (report, existed) = e.open_report("n.txt");
        assert!(!existed);
        let r = report.expect("msg.open.interrupted_new");
        assert_eq!(
            (r.method, r.complete),
            (SaveMethod::NewFile, false),
            "{hook}"
        );
        assert!(r.tmp.is_some());
    }
}

// ---- 直接上書きの切断点 (iii) ----

#[test]
fn フラグだけの段では_t_は無傷でバックアップは無く_報告なしでフラグは掃除される() {
    let e = Env::new("ip-flag");
    fs::write(e.file("a.txt"), ORIG).unwrap();
    killed(&e.save("a.txt", "inplace", NEW, Some("kill_after_inflight_flag")));
    assert_eq!(fs::read(e.file("a.txt")).unwrap(), ORIG);
    assert_eq!(e.flags(), 1);
    assert!(e.backups().is_empty());
    let (report, _) = e.open_report("a.txt");
    assert!(report.is_none(), "msg.open.interrupted_save* は出ない");
    assert_eq!(e.flags(), 0, "開いた後にフラグだけが消える");
    assert_eq!(fs::read(e.file("a.txt")).unwrap(), ORIG);
}

#[test]
fn バックアップの書込中の中断は不完全として案内し_t_は無傷() {
    let e = Env::new("ip-midbackup");
    // 1 チャンク (1 MiB) より大きい元ファイルで、バックアップの途中を作る
    let orig: Vec<u8> = (0..2 * 1024 * 1024)
        .map(|i| b'a' + (i % 26) as u8)
        .collect();
    fs::write(e.file("a.txt"), &orig).unwrap();
    killed(&e.save("a.txt", "inplace", NEW, Some("kill_mid_backup")));
    assert_eq!(
        fs::read(e.file("a.txt")).unwrap(),
        orig,
        "上書きは始まっていない"
    );
    let baks = e.backups();
    assert_eq!(baks.len(), 1);
    assert!(baks[0].to_string_lossy().ends_with(".bak.tmp"), "{baks:?}");
    let (report, _) = e.open_report("a.txt");
    let r = report.expect("msg.open.interrupted_save_incomplete");
    assert_eq!((r.method, r.complete), (SaveMethod::InPlace, false));
    assert_eq!(
        r.backup.unwrap().as_path(),
        baks[0],
        "実在する一時名を案内する"
    );
}

#[test]
fn バックアップの後の切断点では完全なバックアップが残り_案内する() {
    for hook in ["kill_after_backup", "kill_after_inplace_alloc"] {
        let e = Env::new("ip-backup");
        fs::write(e.file("a.txt"), ORIG).unwrap();
        killed(&e.save("a.txt", "inplace", NEW, Some(hook)));
        // 上書きはまだ始まっていない
        assert_eq!(fs::read(e.file("a.txt")).unwrap(), ORIG, "{hook}");
        let baks = e.backups();
        assert_eq!(baks.len(), 1, "{hook}");
        assert!(baks[0].extension().unwrap() == "bak");
        assert_eq!(
            fs::read(&baks[0]).unwrap(),
            ORIG,
            "{hook}: 保存前の元のファイルとバイト一致"
        );
        let (report, existed) = e.open_report("a.txt");
        assert!(existed);
        let r = report.expect("msg.open.interrupted_save");
        assert_eq!(
            (r.method, r.complete),
            (SaveMethod::InPlace, true),
            "{hook}"
        );
        assert_eq!(r.backup.unwrap().as_path(), baks[0]);
        assert_eq!(e.flags(), 1, "報告した記録は開くジョブが消さない");
    }
}

// verifies: AC-FR-105-1
#[test]
fn 上書きの途中の中断では_t_は半端に書き換わりバックアップが元の内容を保つ() {
    let e = Env::new("ip-mid");
    let orig: Vec<u8> = (0..200_000).map(|i| b'a' + (i % 26) as u8).collect();
    fs::write(e.file("a.txt"), &orig).unwrap();
    let new: Vec<u8> = vec![b'N'; 200_000];
    killed(&e.save(
        "a.txt",
        "inplace",
        &new,
        Some("kill_mid_inplace_write=100000"),
    ));
    let t = fs::read(e.file("a.txt")).unwrap();
    assert_eq!(t.len(), 200_000);
    assert!(
        t[..100_000].iter().all(|&b| b == b'N'),
        "先頭の 100,000 バイトは新しい内容"
    );
    assert_eq!(
        &t[100_000..],
        &orig[100_000..],
        "残りは元のまま (半端に書き換わっている)"
    );
    let baks = e.backups();
    assert_eq!(baks.len(), 1);
    assert_eq!(
        fs::read(&baks[0]).unwrap(),
        orig,
        "バックアップは保存前の元のファイルとバイト一致"
    );
    let (report, _) = e.open_report("a.txt");
    let r = report.expect("msg.open.interrupted_save");
    assert!(r.complete && r.backup.is_some());
}

#[test]
fn 上書きの可能性あり_バックアップを外部で消されたら安全側で案内しフラグを残す() {
    let e = Env::new("ip-suspicious");
    fs::write(e.file("a.txt"), ORIG).unwrap();
    killed(&e.save("a.txt", "inplace", NEW, Some("kill_mid_inplace_write=10")));
    // 最終名のバックアップを消してから開く (InPlace / true / true でバックアップが無い記録)
    for b in e.backups() {
        fs::remove_file(b).unwrap();
    }
    let (report, _) = e.open_report("a.txt");
    let r = report.expect("msg.open.interrupted_save_no_backup (安全側)");
    assert_eq!((r.backup.is_none(), r.complete), (true, false));
    assert_eq!(e.flags(), 1, "掃除しない");
    // 何度開いても同じ案内 (報告した記録は消えない)
    assert!(e.open_report("a.txt").0.is_some());
}

#[test]
fn バックアップなしの承認の後の中断は取り戻せない案内() {
    let e = Env::new("ip-skip");
    fs::write(e.file("a.txt"), ORIG).unwrap();
    killed(&e.save(
        "a.txt",
        "inplace-skip",
        NEW,
        Some("kill_mid_inplace_write=10"),
    ));
    assert!(e.backups().is_empty(), "バックアップを作らない");
    assert!(fs::read(e.file("a.txt")).unwrap().starts_with(&NEW[..10]));
    let (report, _) = e.open_report("a.txt");
    let r = report.expect("msg.open.interrupted_save_no_backup");
    assert_eq!(
        (r.method, r.backup.is_none(), r.complete),
        (SaveMethod::InPlace, true, false)
    );
}

#[test]
fn 中断の後の保存の成功で前の記録は消える() {
    let e = Env::new("ip-recover");
    fs::write(e.file("a.txt"), ORIG).unwrap();
    killed(&e.save("a.txt", "inplace", NEW, Some("kill_mid_inplace_write=10")));
    assert!(e.open_report("a.txt").0.is_some());
    // 同じ保存先 T への保存が成功すると、前の中断の記録 (報告した記録) が消える (02 §4.5 の「報告した記録を消す時機」)
    assert_eq!(e.save("a.txt", "atomic", ORIG, None).code, Some(0));
    assert_eq!(e.flags(), 0);
    assert!(e.open_report("a.txt").0.is_none(), "もう案内しない");
    let _ = Path::new("");
}

// ---- 起動時の走査と提案の適用 (M6e、05 §4.6「起動時の提案の規則」、03 §10.3 手順 1b) ----

impl Env {
    fn run(&self, kind: JobKind) -> JobResult {
        let ports = JobPorts {
            fs: Arc::new(self.fs()),
            clock: Arc::new(FixedClock),
            paths: Arc::new(self.paths()),
        };
        let req = JobRequest {
            id: JobId(1),
            doc: None,
            base: Revision(0),
            kind,
        };
        let mut out = Vec::new();
        run_job(&req, &ports, &CancelToken::new(), &mut |r| out.push(r));
        out.pop().expect("結果がある")
    }

    /// 起動時の走査 (`RecoveryScan`) が作る置換途中の痕跡の提案。
    fn scan(&self) -> Vec<Leftover> {
        match self.run(JobKind::RecoveryScan) {
            JobResult::RecoveryFound { leftovers, .. } => leftovers,
            other => panic!("{other:?}"),
        }
    }

    fn act(&self, l: &Leftover, op: LeftoverOp) -> Result<(), String> {
        match self.run(JobKind::LeftoverAct {
            leftover: Box::new(l.clone()),
            op,
        }) {
            JobResult::LeftoverDone { .. } => Ok(()),
            JobResult::Failed { err, .. } => Err(format!("{err:?}")),
            other => panic!("{other:?}"),
        }
    }
}

// 置換の途中の状態 (`kill_after_replace_begin`) は Windows の置換 (T を退避名に移してから一時ファイルを移す) だけにある。
// Unix は rename 1 回で置き換わるので、この状態が無い (05 §4.3)
#[cfg(windows)]
#[test]
fn 置換の途中の強制終了は_走査で_bak_と_tmp_の両方の提案になり_戻して_tmp_を片付けられる() {
    let e = Env::new("scan-both");
    fs::write(e.file("a.txt"), ORIG).unwrap();
    killed(&e.save("a.txt", "atomic", NEW, Some("kill_after_replace_begin")));
    assert!(!e.file("a.txt").exists());
    let l = e.scan();
    assert_eq!(l.len(), 1, "{l:?}");
    assert_eq!(l[0].kind, LeftoverKind::BakAndTmp);
    assert_eq!(l[0].target.as_path(), e.file("a.txt"));
    // 戻す: T は元の内容に戻る。tmp が残るのでフラグも残り、次の走査では tmp の提案になる
    e.act(&l[0], LeftoverOp::Restore).expect("戻せる");
    assert_eq!(fs::read(e.file("a.txt")).unwrap(), ORIG);
    assert_eq!(e.flags(), 1);
    let l2 = e.scan();
    assert_eq!(l2.len(), 1, "{l2:?}");
    assert_eq!(l2[0].kind, LeftoverKind::Tmp);
    // tmp を消すと痕跡もフラグも無くなる
    e.act(&l2[0], LeftoverOp::DeleteTmp).expect("消せる");
    assert_eq!(e.flags(), 0);
    assert!(e.names().iter().all(|n| n == "a.txt"), "{:?}", e.names());
    assert!(e.scan().is_empty());
}

// 置換の途中の状態 (`kill_after_replace_begin`) は Windows の置換 (T を退避名に移してから一時ファイルを移す) だけにある。
// Unix は rename 1 回で置き換わるので、この状態が無い (05 §4.3)
#[cfg(windows)]
#[test]
fn 戻す先に_t_が作られていたら置き換えずに失敗し_退避名を残す() {
    let e = Env::new("scan-conflict");
    fs::write(e.file("a.txt"), ORIG).unwrap();
    killed(&e.save("a.txt", "atomic", NEW, Some("kill_after_replace_begin")));
    let l = e.scan();
    fs::write(e.file("a.txt"), b"created meanwhile").unwrap();
    assert!(e.act(&l[0], LeftoverOp::Restore).is_err());
    assert_eq!(fs::read(e.file("a.txt")).unwrap(), b"created meanwhile");
    assert!(
        l[0].bak.as_ref().is_some_and(|b| b.as_path().exists()),
        "退避名は残る"
    );
    assert_eq!(e.flags(), 1);
}

#[test]
fn 置換の前の強制終了は_走査で_tmp_の提案で_t_は無傷() {
    for hook in ["kill_after_tmp_write", "kill_before_rename"] {
        let e = Env::new("scan-tmp");
        fs::write(e.file("a.txt"), ORIG).unwrap();
        killed(&e.save("a.txt", "atomic", NEW, Some(hook)));
        let l = e.scan();
        assert_eq!(l.len(), 1, "{hook}: {l:?}");
        assert_eq!(l[0].kind, LeftoverKind::Tmp, "{hook}");
        assert!(l[0].bak.is_none() && l[0].tmp.is_some());
        assert_eq!(fs::read(e.file("a.txt")).unwrap(), ORIG);
        // tmp を消すと痕跡もフラグも無くなり、T は無傷のまま
        e.act(&l[0], LeftoverOp::DeleteTmp).expect("消せる");
        assert_eq!(e.flags(), 0, "{hook}");
        assert_eq!(e.names(), ["a.txt"], "{hook}");
        assert!(e.scan().is_empty(), "{hook}");
        assert_eq!(fs::read(e.file("a.txt")).unwrap(), ORIG);
    }
}

#[test]
fn 新規ファイルの保存の強制終了も_tmp_の提案になる() {
    let e = Env::new("scan-new");
    killed(&e.save("n.txt", "atomic", NEW, Some("kill_before_rename")));
    let l = e.scan();
    assert_eq!(l.len(), 1, "{l:?}");
    assert_eq!(l[0].kind, LeftoverKind::Tmp);
    assert!(!e.file("n.txt").exists());
}

#[test]
fn 保存が成功していれば走査は何も提案しない() {
    let e = Env::new("scan-clean");
    fs::write(e.file("a.txt"), ORIG).unwrap();
    assert_eq!(e.save("a.txt", "atomic", NEW, None).code, Some(0));
    assert!(e.scan().is_empty());
}
