//! 実ファイルシステム (`WinFs`) で core の読込ジョブ (`run_job`) が動くこと。M1 の MemFs と同じ経路を、実ファイルで通す。

use std::sync::Arc;

use eve_core::{
    CancelToken, Clock, FsPath, JobKind, JobPorts, JobRequest, JobResult, OpenOptions, Tick,
    run_job,
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

fn ports(cwd: &str) -> JobPorts {
    JobPorts {
        fs: Arc::new(WinFs),
        clock: Arc::new(FixedClock),
        paths: Arc::new(WinPaths::new(
            Dirs::resolve(&|_| None),
            PathCtx {
                cwd: cwd.to_owned(),
                home: None,
            },
        )),
    }
}

fn open(path: &FsPath, ports: &JobPorts) -> JobResult {
    let req = JobRequest {
        id: JobId(1),
        doc: None,
        base: Revision(0),
        kind: JobKind::Open {
            path: path.clone(),
            opts: OpenOptions::default(),
        },
    };
    let mut out = Vec::new();
    run_job(&req, ports, &CancelToken::new(), &mut |r| out.push(r));
    assert_eq!(out.len(), 1, "結果はちょうど 1 件");
    out.remove(0)
}

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let p = std::env::temp_dir().join(format!("eve-platform-it-{tag}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn utf8_のファイルを実ファイルシステムで読み込む() {
    let d = tmp_dir("open");
    let file = d.join("a.txt");
    std::fs::write(&file, "こんにちは\r\nworld\r\n").unwrap();
    let p = ports(&d.to_string_lossy());
    match open(&FsPath::new(file.clone()), &p) {
        JobResult::Complete {
            text, format, meta, ..
        } => {
            use eve_text::TextRead;
            assert_eq!(text.len_bytes(), "こんにちは\r\nworld\r\n".len());
            assert_eq!(format.default_eol, eve_text::LineEnding::Crlf);
            assert_eq!(meta.len, "こんにちは\r\nworld\r\n".len() as u64);
        }
        other => panic!("{other:?}"),
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn 存在しないファイルは新規ファイル_親が無ければ親を知らせる() {
    let d = tmp_dir("new");
    let p = ports(&d.to_string_lossy());
    match open(&FsPath::new(d.join("new.txt")), &p) {
        JobResult::NewFile { parent_missing, .. } => assert!(parent_missing.is_none()),
        other => panic!("{other:?}"),
    }
    match open(&FsPath::new(d.join("nodir").join("new.txt")), &p) {
        JobResult::NewFile { parent_missing, .. } => assert!(parent_missing.is_some()),
        other => panic!("{other:?}"),
    }
    let _ = std::fs::remove_dir_all(&d);
}
