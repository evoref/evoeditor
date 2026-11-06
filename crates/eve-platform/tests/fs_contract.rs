//! 実 FS (Windows は `WinFs`、Unix は `UnixFs`) に `FileSystem` の契約試験を掛ける (docs/02 §4.5、05 §2.1 / §4.1、STR-D3)。
//! 同じ検査を eve-testkit の `tests/fs_contract.rs` が MemFs に掛ける。状態ディレクトリを持つものと持たないものの両方。

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use eve_core::FsPath;
#[cfg(windows)]
use eve_platform::fs::WinFs as NativeFs;
#[cfg(unix)]
use eve_platform::unix_fs::UnixFs as NativeFs;
use eve_testkit::fs_contract::{Caps, Subject, suite};

/// 一時ディレクトリ (終わりに消す)。
struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Tmp {
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("eve-contract-{tag}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(p.join("work")).unwrap();
        Tmp(p)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn names(dir: &FsPath) -> Vec<String> {
    fs::read_dir(dir.as_path())
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn caps(state_dir: bool) -> Caps {
    Caps {
        state_dir,
        ..Caps::default()
    }
}

#[test]
fn 状態ディレクトリを持つ実_fs_は_filesystem_の契約を満たす() {
    suite(|| {
        let t = Tmp::new("state");
        let fsys = NativeFs::new(t.0.join("state"));
        Subject::new(Arc::new(fsys), FsPath::new(t.0.join("work")))
            .caps(caps(true))
            .lister(names)
            .keep(t)
    });
}

#[test]
fn 状態ディレクトリを持たない実_fs_は_filesystem_の契約を満たす() {
    suite(|| {
        let t = Tmp::new("nostate");
        Subject::new(Arc::new(NativeFs::default()), FsPath::new(t.0.join("work")))
            .caps(caps(false))
            .lister(names)
            .keep(t)
    });
}
