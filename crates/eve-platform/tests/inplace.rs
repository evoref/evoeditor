//! `WinFs::write_in_place` の実ファイルでの検査 (docs/05 §4.4 の手順 1〜9、09 §3.6 の in-place の項)。

#![cfg(windows)]
#![allow(clippy::result_large_err, clippy::print_stderr)]

use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};

use eve_core::{
    CancelToken, ChunkError, Expect, FileStamp, FileSystem, FsErrorDetail, FsErrorKind, FsPath,
    InPlaceBackup, ReadChunks, SaveMethod, SaveOptions,
};
use eve_platform::fs::WinFs;

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
        let root =
            std::env::temp_dir().join(format!("eve-inplace-{tag}-{}-{n}", std::process::id()));
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

    fn stamp(&self, name: &str) -> FileStamp {
        FileStamp::from_meta(&self.fs.metadata(&self.path(name)).unwrap(), None)
    }

    fn count(&self, dir: &Path) -> usize {
        fn walk(d: &Path) -> usize {
            fs::read_dir(d)
                .map(|rd| {
                    rd.flatten()
                        .map(|e| {
                            if e.path().is_dir() {
                                walk(&e.path())
                            } else {
                                1
                            }
                        })
                        .sum()
                })
                .unwrap_or(0)
        }
        walk(dir)
    }

    fn save(&self, name: &str, data: &[u8]) -> Result<eve_core::SaveReport, eve_core::FsError> {
        let o = SaveOptions::user_file(Expect::Stamp(self.stamp(name)));
        self.save_with(name, data, &o)
    }

    fn save_with(
        &self,
        name: &str,
        data: &[u8],
        o: &SaveOptions,
    ) -> Result<eve_core::SaveReport, eve_core::FsError> {
        self.fs.write_in_place(
            &self.path(name),
            &mut Data::new(data),
            data.len() as u64,
            o,
            &CancelToken::new(),
        )
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct Data {
    d: Vec<u8>,
    pos: usize,
}

impl Data {
    fn new(d: &[u8]) -> Data {
        Data {
            d: d.to_vec(),
            pos: 0,
        }
    }
}

impl ReadChunks for Data {
    fn next_chunk(&mut self) -> Result<Option<&[u8]>, ChunkError> {
        if self.pos >= self.d.len() {
            return Ok(None);
        }
        let end = (self.pos + 4096).min(self.d.len());
        let c = &self.d[self.pos..end];
        self.pos = end;
        Ok(Some(c))
    }
}

#[test]
fn 長くする_ファイルの_id_と作成物は変わらず_後片付けされる() {
    let e = Env::new("longer");
    fs::write(e.file("a.txt"), b"short").unwrap();
    let before = e.stamp("a.txt");
    let r = e.save("a.txt", "もっと長い内容にする".as_bytes()).unwrap();
    assert_eq!(r.method, SaveMethod::InPlace);
    assert_eq!(
        fs::read(e.file("a.txt")).unwrap(),
        "もっと長い内容にする".as_bytes()
    );
    assert_eq!(
        r.new_stamp.file_id, before.file_id,
        "同じファイルに書くので FileId は変わらない"
    );
    assert_eq!(r.new_stamp, e.stamp("a.txt"));
    assert_eq!(e.count(&e.state), 0, "フラグもバックアップも消える");
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

#[test]
fn 短くすると書き終えた後に縮める() {
    let e = Env::new("shorter");
    fs::write(e.file("a.txt"), b"0123456789").unwrap();
    e.save("a.txt", b"abc").unwrap();
    assert_eq!(fs::read(e.file("a.txt")).unwrap(), b"abc");
    // 空にもできる
    e.save("a.txt", b"").unwrap();
    assert_eq!(fs::read(e.file("a.txt")).unwrap(), b"");
}

#[test]
fn 大きな内容を書く() {
    let e = Env::new("big");
    let orig: Vec<u8> = (0..2 * 1024 * 1024).map(|i| (i % 253) as u8).collect();
    fs::write(e.file("a.bin"), &orig).unwrap();
    let new: Vec<u8> = (0..3 * 1024 * 1024).map(|i| (i % 241) as u8).collect();
    e.save("a.bin", &new).unwrap();
    assert_eq!(fs::read(e.file("a.bin")).unwrap(), new);
    assert_eq!(e.count(&e.state), 0);
}

#[test]
fn 書込の途中でも事前バックアップは完全で_フラグは_backup_complete() {
    let e = Env::new("during");
    let orig = b"original content".to_vec();
    fs::write(e.file("a.txt"), &orig).unwrap();
    let state = e.state.clone();
    // 本書込の最初のチャンクを渡す時点で、バックアップとフラグを調べる
    struct Peek<'a> {
        state: PathBuf,
        orig: &'a [u8],
        seen: Cell<bool>,
        done: bool,
    }
    impl ReadChunks for Peek<'_> {
        fn next_chunk(&mut self) -> Result<Option<&[u8]>, ChunkError> {
            if self.done {
                return Ok(None);
            }
            self.done = true;
            let baks: Vec<_> = fs::read_dir(self.state.join("backup"))
                .unwrap()
                .flatten()
                .map(|e| e.path())
                .collect();
            assert_eq!(baks.len(), 1, "{baks:?}");
            assert!(
                baks[0].extension().unwrap() == "bak",
                "一時名ではなく最終名"
            );
            assert_eq!(
                fs::read(&baks[0]).unwrap(),
                self.orig,
                "元のバイト列そのもの"
            );
            let flag = fs::read_dir(self.state.join("recovery").join("inflight"))
                .unwrap()
                .flatten()
                .flat_map(|d| fs::read_dir(d.path()).unwrap().flatten())
                .next()
                .unwrap()
                .path();
            let t = fs::read_to_string(flag).unwrap();
            assert!(
                t.contains("method = \"in_place\"") && t.contains("backup_complete = true"),
                "{t}"
            );
            assert!(t.contains("backup_recorded = true"));
            self.seen.set(true);
            Ok(Some(b"new"))
        }
    }
    let mut src = Peek {
        state,
        orig: &orig,
        seen: Cell::new(false),
        done: false,
    };
    let o = SaveOptions::user_file(Expect::Stamp(e.stamp("a.txt")));
    e.fs.write_in_place(&e.path("a.txt"), &mut src, 3, &o, &CancelToken::new())
        .unwrap();
    assert!(src.seen.get());
    assert_eq!(fs::read(e.file("a.txt")).unwrap(), b"new");
}

#[test]
fn バックアップなしの承認ではバックアップを作らずフラグは_recorded_false() {
    let e = Env::new("skip");
    fs::write(e.file("a.txt"), b"old").unwrap();
    let mut o = SaveOptions::user_file(Expect::Stamp(e.stamp("a.txt")));
    o.in_place_backup = InPlaceBackup::SkipConfirmed;
    e.save_with("a.txt", b"new", &o).unwrap();
    assert_eq!(fs::read(e.file("a.txt")).unwrap(), b"new");
    assert!(!e.state.join("backup").exists(), "バックアップを作らない");
    assert_eq!(e.count(&e.state), 0);
}

#[test]
fn 状態ディレクトリが無いと_backup_failed_で何も書かない() {
    let e = Env::new("nostate");
    fs::write(e.file("a.txt"), b"keep").unwrap();
    let no_state = WinFs::default();
    let o = SaveOptions::user_file(Expect::Stamp(e.stamp("a.txt")));
    let err = no_state
        .write_in_place(
            &e.path("a.txt"),
            &mut Data::new(b"new"),
            3,
            &o,
            &CancelToken::new(),
        )
        .unwrap_err();
    assert_eq!(err.kind, FsErrorKind::BackupFailed);
    assert!(matches!(err.detail, Some(FsErrorDetail::InflightFlag)));
    assert_eq!(fs::read(e.file("a.txt")).unwrap(), b"keep");
    // 承認の後 (バックアップなし) は警告して書く
    let mut o2 = o;
    o2.in_place_backup = InPlaceBackup::SkipConfirmed;
    let r = no_state
        .write_in_place(
            &e.path("a.txt"),
            &mut Data::new(b"new"),
            3,
            &o2,
            &CancelToken::new(),
        )
        .unwrap();
    assert!(
        r.warnings
            .contains(&eve_core::SaveWarning::InflightFlagNotWritten)
    );
    assert_eq!(fs::read(e.file("a.txt")).unwrap(), b"new");
}

#[test]
fn 照合の不一致は_conflict_で何も書かず後片付けする() {
    let e = Env::new("conflict");
    fs::write(e.file("a.txt"), b"mine").unwrap();
    let s0 = e.stamp("a.txt");
    std::thread::sleep(std::time::Duration::from_millis(30));
    fs::write(e.file("a.txt"), b"theirs!").unwrap();
    let err = e
        .save_with("a.txt", b"x", &SaveOptions::user_file(Expect::Stamp(s0)))
        .unwrap_err();
    assert!(
        matches!(err.kind, FsErrorKind::Conflict { current: Some(_) }),
        "{err:?}"
    );
    assert_eq!(fs::read(e.file("a.txt")).unwrap(), b"theirs!");
    assert_eq!(e.count(&e.state), 0);
    // 消えた
    fs::remove_file(e.file("a.txt")).unwrap();
    let err = e
        .save_with("a.txt", b"x", &SaveOptions::user_file(Expect::Stamp(s0)))
        .unwrap_err();
    assert!(matches!(err.kind, FsErrorKind::Conflict { current: None }));
}

#[test]
fn 取消しは書く前の段で見て_元は無傷でバックアップも消える() {
    let e = Env::new("cancel");
    fs::write(e.file("a.txt"), b"orig").unwrap();
    let cancel = CancelToken::new();
    cancel.cancel();
    let o = SaveOptions::user_file(Expect::Stamp(e.stamp("a.txt")));
    let err =
        e.fs.write_in_place(&e.path("a.txt"), &mut Data::new(b"new"), 3, &o, &cancel)
            .unwrap_err();
    assert_eq!(err.kind, FsErrorKind::SourceFailed);
    assert_eq!(fs::read(e.file("a.txt")).unwrap(), b"orig");
    assert_eq!(e.count(&e.state), 0);
}

#[test]
fn ハードリンクの両方に書いた内容が見える() {
    let e = Env::new("hardlink");
    fs::write(e.file("a.txt"), b"shared").unwrap();
    fs::hard_link(e.file("a.txt"), e.file("b.txt")).unwrap();
    e.save("a.txt", b"updated").unwrap();
    assert_eq!(
        fs::read(e.file("b.txt")).unwrap(),
        b"updated",
        "リンクを維持する (直接上書き)"
    );
}

#[test]
fn 他のプロセスが書込アクセスで開いていると_busy() {
    use std::os::windows::fs::OpenOptionsExt;
    let e = Env::new("busy");
    fs::write(e.file("a.txt"), b"busy").unwrap();
    let s0 = e.stamp("a.txt");
    let _hold = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(1 | 2 | 4)
        .open(e.file("a.txt"))
        .unwrap();
    let t = std::time::Instant::now();
    let err = e
        .save_with("a.txt", b"new", &SaveOptions::user_file(Expect::Stamp(s0)))
        .unwrap_err();
    assert_eq!(err.kind, FsErrorKind::Busy, "{err:?}");
    assert!(
        t.elapsed().as_millis() >= 2000,
        "指数バックオフで再試行する"
    );
    drop(_hold);
    assert_eq!(fs::read(e.file("a.txt")).unwrap(), b"busy");
    assert_eq!(e.count(&e.state), 0, "フラグを消す");
}

#[test]
fn 元のファイルが無いときは_not_found() {
    let e = Env::new("missing");
    let err = e
        .save_with("nope.txt", b"x", &SaveOptions::user_file(Expect::Any))
        .unwrap_err();
    assert_eq!(err.kind, FsErrorKind::NotFound);
}

#[test]
fn ads_と_hidden_属性はそのまま残る() {
    let e = Env::new("ads");
    fs::write(e.file("a.txt"), b"old").unwrap();
    let ads = format!("{}:Zone.Identifier", e.file("a.txt").display());
    fs::write(&ads, b"[ZoneTransfer]\r\nZoneId=3\r\n").unwrap();
    assert!(
        std::process::Command::new("attrib")
            .arg("+H")
            .arg(e.file("a.txt"))
            .status()
            .unwrap()
            .success()
    );
    e.save("a.txt", b"new").unwrap();
    assert_eq!(fs::read(&ads).unwrap(), b"[ZoneTransfer]\r\nZoneId=3\r\n");
    let out = std::process::Command::new("attrib")
        .arg(e.file("a.txt"))
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains('H'));
}
