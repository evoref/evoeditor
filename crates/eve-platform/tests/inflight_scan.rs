//! 起動時の走査 (docs/05 §4.6、03 §10.3 手順 1b): `list_inflight` と `list_orphan_backups`。実ファイルで、
//! 動いていないプロセスが残したフラグ・事前バックアップだけを返し、動いているプロセスのものは除く。

#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};

use eve_core::{FileSystem, SaveMethod};
use eve_platform::fs::WinFs;

/// 動いていない pid (Windows の pid は 4 の倍数で、この値は割り当てられない)。
const DEAD_PID: u32 = 0xFFFF_FFF0;

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
        let root = std::env::temp_dir().join(format!("eve-scan-{tag}-{}-{n}", std::process::id()));
        let state = root.join("state");
        fs::create_dir_all(&state).unwrap();
        Env {
            fs: WinFs::new(&state),
            root,
            state,
        }
    }

    /// `recovery/inflight/<key>/<name>` にフラグを置く。
    fn flag(&self, key: &str, name: &str, body: &str) -> PathBuf {
        let dir = self.state.join("recovery").join("inflight").join(key);
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        fs::write(&p, body).unwrap();
        p
    }

    fn backup(&self, name: &str, body: &[u8]) -> PathBuf {
        let dir = self.state.join("backup");
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        fs::write(&p, body).unwrap();
        p
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn flag_body(target: &Path, pid: u32, method: &str) -> String {
    format!(
        "format_version = 1\ntarget = \"{}\"\npid = {pid}\nmethod = \"{method}\"\nbackup_recorded = false\nbackup_complete = false\n",
        target.display().to_string().replace('\\', "\\\\")
    )
}

#[test]
fn 動いていないプロセスのフラグを全部返す() {
    let e = Env::new("flags");
    let a = e.root.join("a.txt");
    let b = e.root.join("b.txt");
    e.flag(
        "aaaa",
        &format!("{DEAD_PID}-1.toml"),
        &flag_body(&a, DEAD_PID, "atomic"),
    );
    e.flag(
        "aaaa",
        &format!("{DEAD_PID}-2.toml"),
        &flag_body(&a, DEAD_PID, "in_place"),
    );
    e.flag(
        "bbbb",
        &format!("{DEAD_PID}-1.toml"),
        &flag_body(&b, DEAD_PID, "new_file"),
    );
    let recs = e.fs.list_inflight().unwrap();
    assert_eq!(recs.len(), 3, "{recs:?}");
    assert!(recs.iter().all(|r| !r.malformed && r.pid == DEAD_PID));
    let methods: Vec<_> = recs.iter().map(|r| r.method).collect();
    assert!(methods.contains(&SaveMethod::InPlace));
    assert!(methods.contains(&SaveMethod::NewFile));
    assert!(recs.iter().any(|r| r.target.as_path() == b));
}

#[test]
fn 動いているプロセスのフラグは除く() {
    let e = Env::new("alive");
    let a = e.root.join("a.txt");
    let me = std::process::id();
    e.flag(
        "aaaa",
        &format!("{me}-1.toml"),
        &flag_body(&a, me, "atomic"),
    );
    e.flag(
        "aaaa",
        &format!("{DEAD_PID}-1.toml"),
        &flag_body(&a, DEAD_PID, "atomic"),
    );
    let recs = e.fs.list_inflight().unwrap();
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0].pid, DEAD_PID);
}

#[test]
fn 壊れたフラグは_malformed_で返す() {
    let e = Env::new("bad");
    e.flag("aaaa", &format!("{DEAD_PID}-1.toml"), "garbage");
    e.flag("aaaa", "readme.txt", "toml ではない");
    let recs = e.fs.list_inflight().unwrap();
    assert_eq!(recs.len(), 1, "{recs:?}");
    assert!(recs[0].malformed);
    assert_eq!(recs[0].pid, DEAD_PID, "pid はファイル名から");
}

#[test]
fn 状態ディレクトリが空ならどちらも空() {
    let e = Env::new("empty");
    assert!(e.fs.list_inflight().unwrap().is_empty());
    assert!(e.fs.list_orphan_backups().unwrap().is_empty());
}

#[test]
fn 孤児の事前バックアップを列挙する() {
    let e = Env::new("bak");
    let me = std::process::id();
    let full = e.backup(
        &format!("0123456789abcdef-20261030T010203Z-{DEAD_PID}.bak"),
        b"hello",
    );
    let part = e.backup(
        &format!("0123456789abcdef-20261030T010204Z-{DEAD_PID}.bak.tmp"),
        b"he",
    );
    // 動いているプロセスの分、形に合わない名前、サブディレクトリは含めない
    e.backup(
        &format!("0123456789abcdef-20261030T010205Z-{me}.bak"),
        b"live",
    );
    e.backup("notes.txt", b"x");
    e.backup("no-pid.bak", b"x");
    fs::create_dir_all(e.state.join("backup").join("fr119")).unwrap();
    let got = e.fs.list_orphan_backups().unwrap();
    assert_eq!(got.len(), 2, "{got:?}");
    let f = got.iter().find(|o| o.path.as_path() == full).unwrap();
    assert!(!f.partial);
    assert_eq!((f.pid, f.len), (DEAD_PID, 5));
    let p = got.iter().find(|o| o.path.as_path() == part).unwrap();
    assert!(p.partial);
    assert_eq!(p.len, 2);
}
