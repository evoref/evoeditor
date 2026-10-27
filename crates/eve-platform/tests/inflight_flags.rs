//! 保存中フラグと中断された保存の報告 (docs/05 §4.6 / §7.5、02 §4.5、01 FR-105 AC-1)。実ファイルで、
//! 保存の開始時にフラグを書き、成功・置換前の失敗で消し、クラッシュ (パニック) で残った痕跡を開くときに報告する。

#![cfg(windows)]
#![allow(clippy::result_large_err)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use eve_core::{
    CancelToken, ChunkError, Clock, Expect, FileStamp, FileSystem, FsPath, JobKind, JobPorts,
    JobRequest, JobResult, OpenOptions, ReadChunks, SaveMethod, SaveOptions, Tick, run_job,
};
use eve_platform::dirs::{Dirs, WinPaths};
use eve_platform::fs::WinFs;
use eve_platform::path::PathCtx;
use eve_view::{JobId, Revision};

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> Tick {
        Tick(0)
    }
}

struct Env {
    root: PathBuf,
    state: PathBuf,
    fs: WinFs,
}

impl Env {
    fn new(tag: &str) -> Env {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let root = std::env::temp_dir().join(format!("eve-flag-{tag}-{}-{n}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let state = root.join("state");
        Env {
            fs: WinFs::new(&state),
            root,
            state,
        }
    }

    fn file(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn path(&self, name: &str) -> FsPath {
        FsPath::new(self.file(name))
    }

    /// `state/recovery/inflight/` の下にあるフラグのファイル。
    fn flags(&self) -> Vec<PathBuf> {
        let base = self.state.join("recovery").join("inflight");
        let mut out = Vec::new();
        if let Ok(rd) = fs::read_dir(&base) {
            for d in rd.flatten() {
                if let Ok(inner) = fs::read_dir(d.path()) {
                    out.extend(inner.flatten().map(|e| e.path()));
                }
            }
        }
        out
    }

    fn ports(&self) -> JobPorts {
        JobPorts {
            fs: Arc::new(self.fs.clone()),
            clock: Arc::new(FixedClock),
            paths: Arc::new(WinPaths::new(
                Dirs::resolve(&|_| None),
                PathCtx {
                    cwd: self.root.to_string_lossy().into_owned(),
                    home: None,
                },
            )),
        }
    }

    fn open(&self, name: &str) -> JobResult {
        let req = JobRequest {
            id: JobId(1),
            doc: None,
            base: Revision(0),
            kind: JobKind::Open {
                path: self.path(name),
                opts: OpenOptions::default(),
            },
        };
        let mut out = Vec::new();
        run_job(&req, &self.ports(), &CancelToken::new(), &mut |r| {
            out.push(r)
        });
        out.pop().expect("結果がある")
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct Data(Vec<Vec<u8>>, usize);

impl ReadChunks for Data {
    fn next_chunk(&mut self) -> Result<Option<&[u8]>, ChunkError> {
        let c = self.0.get(self.1);
        self.1 += 1;
        Ok(c.map(Vec::as_slice))
    }
}

/// 2 つめのチャンクでパニックする (クラッシュの再現: 一時ファイルとフラグが残る)。
struct Crash(usize);

impl ReadChunks for Crash {
    fn next_chunk(&mut self) -> Result<Option<&[u8]>, ChunkError> {
        self.0 += 1;
        assert!(self.0 < 2, "クラッシュの再現");
        Ok(Some(b"partial"))
    }
}

fn save(fs: &WinFs, path: &FsPath, data: &[u8], expect: Expect) -> Result<(), eve_core::FsError> {
    fs.write_atomic(
        path,
        &mut Data(vec![data.to_vec()], 0),
        &SaveOptions::user_file(expect),
    )
    .map(|_| ())
}

fn crash(fs: &WinFs, path: &FsPath, expect: Expect) {
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = fs.write_atomic(path, &mut Crash(0), &SaveOptions::user_file(expect));
    }));
    assert!(r.is_err(), "パニックが外へ出る");
}

fn stamp(fs: &WinFs, p: &FsPath) -> FileStamp {
    FileStamp::from_meta(&fs.metadata(p).unwrap(), None)
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n != "state")
        .collect();
    v.sort();
    v
}

#[test]
fn 保存の成功ではフラグは残らない() {
    let e = Env::new("ok");
    save(&e.fs, &e.path("a.txt"), b"new", Expect::Absent).unwrap();
    assert!(e.flags().is_empty(), "{:?}", e.flags());
    fs::write(e.file("a.txt"), b"x").unwrap();
    let s0 = stamp(&e.fs, &e.path("a.txt"));
    save(&e.fs, &e.path("a.txt"), b"y", Expect::Stamp(s0)).unwrap();
    assert!(e.flags().is_empty());
}

#[test]
fn 保存の途中で書き手が止まるとフラグと一時ファイルが残る() {
    let e = Env::new("crash");
    let p = e.path("a.txt");
    crash(&e.fs, &p, Expect::Absent);
    assert_eq!(e.flags().len(), 1);
    let recs = e.fs.inflight_for(&p).unwrap();
    assert_eq!(recs.len(), 1);
    let r = &recs[0];
    assert_eq!(r.method, SaveMethod::NewFile);
    assert_eq!(r.pid, std::process::id());
    assert!(!r.malformed);
    assert!(
        r.tmp.as_ref().unwrap().as_path().exists(),
        "書きかけの一時ファイル"
    );
    assert!(!e.file("a.txt").exists());
}

#[test]
fn 置換前の失敗では一時ファイルとフラグの両方を消す() {
    let e = Env::new("fail");
    let p = e.path("a.txt");
    fs::write(e.file("a.txt"), b"orig").unwrap();
    let s0 = stamp(&e.fs, &p);
    // 供給側の失敗
    struct Fail;
    impl ReadChunks for Fail {
        fn next_chunk(&mut self) -> Result<Option<&[u8]>, ChunkError> {
            Err(ChunkError::Aborted)
        }
    }
    let err =
        e.fs.write_atomic(&p, &mut Fail, &SaveOptions::user_file(Expect::Stamp(s0)))
            .unwrap_err();
    assert_eq!(err.kind, eve_core::FsErrorKind::SourceFailed);
    assert!(e.flags().is_empty());
    assert_eq!(names(&e.root), ["a.txt"]);
    // 照合の不一致 (フラグを書く前)
    fs::write(e.file("a.txt"), b"changed by someone").unwrap();
    assert!(save(&e.fs, &p, b"z", Expect::Stamp(s0)).is_err());
    assert!(e.flags().is_empty());
}

#[test]
fn 開くと中断された新規作成を報告する() {
    let e = Env::new("open-new");
    crash(&e.fs, &e.path("a.txt"), Expect::Absent);
    match e.open("a.txt") {
        JobResult::NewFile {
            interrupted_save: Some(is),
            ..
        } => {
            assert_eq!(is.method, SaveMethod::NewFile);
            assert!(!is.complete);
            assert!(is.tmp.is_some(), "書きかけの一時ファイルの場所を示す");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(e.flags().len(), 1, "報告した記録は開くジョブが消さない");
}

#[test]
fn 置換の前に止まった保存は_t_が在れば報告せずフラグは痕跡があるので残る() {
    let e = Env::new("open-atomic");
    let p = e.path("a.txt");
    fs::write(e.file("a.txt"), b"orig").unwrap();
    let s0 = stamp(&e.fs, &p);
    crash(&e.fs, &p, Expect::Stamp(s0));
    match e.open("a.txt") {
        JobResult::Complete {
            interrupted_save,
            text,
            ..
        } => {
            assert!(interrupted_save.is_none(), "T は置換前の完全な版");
            assert_eq!(eve_text::TextRead::len_bytes(&text), 4);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        e.flags().len(),
        1,
        "一時ファイルが実在するので手掛かりとして残す"
    );
}

#[test]
fn 次の保存の成功で前の中断の記録は消える_痕跡が実在するものは残る() {
    let e = Env::new("clear");
    let p = e.path("a.txt");
    crash(&e.fs, &p, Expect::Absent); // 新規の途中で中断: 一時ファイルが残る
    assert_eq!(e.flags().len(), 1);
    // 一時ファイルを手で消す (痕跡が無くなった)
    for rec in e.fs.inflight_for(&p).unwrap() {
        fs::remove_file(rec.tmp.unwrap().as_path()).unwrap();
    }
    save(&e.fs, &p, b"fine", Expect::Absent).unwrap();
    assert!(e.flags().is_empty(), "痕跡の無い前の記録も消える");
}

#[test]
fn 痕跡が実在する前の記録は成功した保存の後も残る() {
    let e = Env::new("keep");
    let p = e.path("a.txt");
    crash(&e.fs, &p, Expect::Absent);
    save(&e.fs, &p, b"fine", Expect::Absent).unwrap();
    assert_eq!(e.flags().len(), 1, "一時ファイルが実在する記録は消さない");
}

#[test]
fn 壊れたフラグは上書きの可能性ありとして報告する() {
    let e = Env::new("malformed");
    let p = e.path("a.txt");
    crash(&e.fs, &p, Expect::Absent);
    let flag = e.flags().remove(0);
    fs::write(&flag, "format_version = 99\n").unwrap();
    fs::write(e.file("a.txt"), b"maybe broken").unwrap();
    match e.open("a.txt") {
        JobResult::Complete {
            interrupted_save: Some(is),
            ..
        } => {
            assert_eq!(is.method, SaveMethod::InPlace);
            assert!(
                is.backup.is_none() && !is.complete,
                "安全側: バックアップは確認できない"
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(flag.exists(), "解析できないフラグは消さない");
}

#[test]
fn シンボリックリンク経由でもリンク先のフラグを見つける() {
    let e = Env::new("link");
    fs::write(e.file("real.txt"), b"x").unwrap();
    if std::os::windows::fs::symlink_file(e.file("real.txt"), e.file("link.txt")).is_err() {
        return; // 権限なし
    }
    let real = e.path("real.txt");
    crash(&e.fs, &real, Expect::Stamp(stamp(&e.fs, &real)));
    let via_link = e.fs.inflight_for(&e.path("link.txt")).unwrap();
    let direct = e.fs.inflight_for(&real).unwrap();
    assert_eq!(via_link.len(), 1);
    assert_eq!(via_link[0].flag, direct[0].flag);
}

#[test]
fn 状態ディレクトリを持たない実装はフラグを書かず空を返す() {
    let e = Env::new("nostate");
    let fs_no_state = WinFs::default();
    save(&fs_no_state, &e.path("a.txt"), b"x", Expect::Absent).unwrap();
    assert!(e.flags().is_empty());
    assert!(
        fs_no_state
            .inflight_for(&e.path("a.txt"))
            .unwrap()
            .is_empty()
    );
}
