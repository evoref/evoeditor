//! 読込の進捗とキャンセル (docs/03 §9.2、01 FR-122 AC-1): 開く操作から 50 ms 以内に最初の進捗、以後 10 Hz 以下、Esc で中止。

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use eve_core::{
    CancelToken, Clock, FsPath, JobKind, JobPorts, JobRequest, JobResult, OpenOptions, Tick,
    run_job,
};
use eve_testkit::{FixedPaths, Harness, MemFs};
use eve_view::{JobId, Revision};

/// 呼ぶたびに `step` ミリ秒進む時計。
struct Stepping {
    now: AtomicU64,
    step: u64,
}

impl Clock for Stepping {
    fn now(&self) -> Tick {
        Tick(self.now.fetch_add(self.step, Ordering::Relaxed))
    }
}

fn open_results(step_ms: u64, chunk: usize, size: usize) -> Vec<JobResult> {
    let clock = Arc::new(Stepping {
        now: AtomicU64::new(0),
        step: step_ms,
    });
    let fs = Arc::new(MemFs::new(clock.clone()));
    fs.add_dir("/w");
    fs.add_file("/w/a.txt", &vec![b'a'; size]);
    fs.set_chunk_size(chunk);
    let ports = JobPorts {
        fs,
        clock,
        paths: Arc::new(FixedPaths::default()),
    };
    let req = JobRequest {
        id: JobId(1),
        doc: None,
        base: Revision(0),
        kind: JobKind::Open {
            path: FsPath::from("/w/a.txt"),
            opts: OpenOptions::default(),
        },
    };
    let mut out = Vec::new();
    run_job(&req, &ports, &CancelToken::new(), &mut |r| out.push(r));
    out
}

// verifies: AC-FR-122-1
#[test]
fn 最初の進捗は読み始めてすぐ送り_終わりは最後に届く() {
    let out = open_results(0, 1024, 10_000);
    assert!(matches!(
        out[0],
        JobResult::Progress {
            bytes: 0,
            total: 10_000,
            ..
        }
    ));
    assert!(matches!(out.last(), Some(JobResult::Complete { .. })));
    assert_eq!(
        out.iter()
            .filter(|r| matches!(r, JobResult::Progress { .. }))
            .count(),
        1,
        "時計が進まなければ 100 ms あけられず、最初の 1 件だけ"
    );
}

// verifies: AC-FR-122-1
#[test]
fn 進捗は_100_ms_以上あけて送る() {
    // 呼ぶたびに 60 ms 進む時計: 2 回に 1 回 (120 ms ごと) だけ送る
    let out = open_results(60, 1024, 20 * 1024);
    let n = out
        .iter()
        .filter(|r| matches!(r, JobResult::Progress { .. }))
        .count();
    assert!((2..=12).contains(&n), "{n}");
    let bytes: Vec<u64> = out
        .iter()
        .filter_map(|r| match r {
            JobResult::Progress { bytes, .. } => Some(*bytes),
            _ => None,
        })
        .collect();
    assert!(bytes.windows(2).all(|w| w[0] < w[1]), "{bytes:?}");
}

// verifies: AC-FR-122-1
#[test]
fn 進捗はステータスバーに百分率で出る() {
    let mut h = Harness::new().with_file("/w/a.txt", b"hello");
    h.open("/w/a.txt");
    // 読込中の文書に進捗が届く (最初の文書は JobId(1) が退避の走査、2 が読込)
    h.editor.on_job_result(JobResult::Progress {
        job: JobId(2),
        bytes: 50,
        total: 200,
    });
    let f = h.frame();
    let st: String = f
        .status
        .items
        .iter()
        .map(|i| {
            i.long
                .spans
                .iter()
                .map(|s| s.text.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join(" ");
    assert!(st.contains("読込中 25%"), "{st}");
}

// verifies: AC-FR-122-1
#[test]
fn esc_で読込中の文書を取り消すと今の文書が残る() {
    let mut h = Harness::new().with_file("/w/a.txt", b"hello");
    h.text("mine");
    h.open("/w/a.txt");
    // 未保存の確認が出る場合は破棄を選んで読込へ進む
    if h.overlay() == "dialog" {
        h.keys("d");
    }
    h.keys("esc");
    h.run_jobs();
    assert_eq!(h.doc_text(), "mine");
    assert!(h.messages().contains(&"info.open.cancelled".to_owned()));
}
