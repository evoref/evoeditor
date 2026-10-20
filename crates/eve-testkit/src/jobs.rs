//! `ManualJobSpawner`: テストのジョブ実行の唯一の実装 (docs/02 §4.4)。
//!
//! `spawn` はキューに積むだけで実行しない。テストが `run_jobs` を呼ぶと、積まれた順に **同じスレッドで** `run_job` を
//! 実行し、結果を inbox に積む。`cancel` は未実行ならキューから外して `Cancelled` を積む。

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use eve_core::{
    CancelToken, JobError, JobKind, JobPorts, JobRequest, JobResult, JobSpawner, run_job,
};
use eve_view::{DocumentId, JobId, Revision};

struct Queued {
    req: JobRequest,
    cancel: CancelToken,
}

pub struct ManualJobSpawner {
    ports: JobPorts,
    next_id: Cell<u64>,
    queue: RefCell<VecDeque<Queued>>,
    inbox: RefCell<VecDeque<JobResult>>,
}

impl ManualJobSpawner {
    pub fn new(ports: JobPorts) -> Rc<ManualJobSpawner> {
        Rc::new(ManualJobSpawner {
            ports,
            next_id: Cell::new(1),
            queue: RefCell::default(),
            inbox: RefCell::default(),
        })
    }

    /// 実行待ちのジョブの数。
    pub fn pending(&self) -> usize {
        self.queue.borrow().len()
    }

    /// 積まれたジョブを積まれた順にすべて実行する。実行したジョブの数を返す。
    /// 実行中に `spawn` されたジョブも、同じ呼び出しの中で続けて実行する。
    pub fn run_jobs(&self) -> usize {
        let mut n = 0;
        loop {
            // 実行中に spawn / cancel されても借用が衝突しないよう、1 件ずつ取り出してから実行する
            let Some(job) = self.queue.borrow_mut().pop_front() else {
                break;
            };
            let mut out = Vec::new();
            run_job(&job.req, &self.ports, &job.cancel, &mut |r| out.push(r));
            self.inbox.borrow_mut().extend(out);
            n += 1;
        }
        n
    }

    /// 結果の inbox を空にして返す (Host の受信口の代わり)。
    pub fn take_results(&self) -> Vec<JobResult> {
        self.inbox.borrow_mut().drain(..).collect()
    }
}

impl JobSpawner for ManualJobSpawner {
    fn spawn(&self, doc: Option<DocumentId>, base: Revision, kind: JobKind) -> JobId {
        let id = JobId(self.next_id.get());
        self.next_id.set(id.0 + 1);
        let req = JobRequest {
            id,
            doc,
            base,
            kind,
        };
        self.queue.borrow_mut().push_back(Queued {
            req,
            cancel: CancelToken::new(),
        });
        id
    }

    fn cancel(&self, job: JobId) {
        let mut q = self.queue.borrow_mut();
        if let Some(pos) = q.iter().position(|j| j.req.id == job) {
            let j = q.remove(pos).expect("位置は直前に見つけた");
            j.cancel.cancel();
            self.inbox.borrow_mut().push_back(JobResult::Failed {
                job,
                err: JobError::Cancelled,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use eve_core::{FsPath, MetaPurpose};

    use super::*;
    use crate::{FixedPaths, ManualClock, MemFs};

    fn setup() -> (Rc<ManualJobSpawner>, Arc<MemFs>) {
        let clock = ManualClock::new();
        let fs = Arc::new(MemFs::new(Arc::new(clock.clone())));
        fs.add_file("/w/a.txt", b"hello");
        let ports = JobPorts {
            fs: fs.clone(),
            clock: Arc::new(clock),
            paths: Arc::new(FixedPaths::default()),
        };
        (ManualJobSpawner::new(ports), fs)
    }

    fn meta(path: &str) -> JobKind {
        JobKind::Metadata {
            path: FsPath::from(path),
            purpose: MetaPurpose::ExternalCheck,
        }
    }

    #[test]
    fn spawn_は実行せず_run_jobs_で積んだ順に実行する() {
        let (sp, _fs) = setup();
        let a = sp.spawn(None, Revision(0), meta("/w/a.txt"));
        let b = sp.spawn(None, Revision(0), meta("/w/none.txt"));
        assert_eq!((a, b), (JobId(1), JobId(2)), "JobId は単調増加");
        assert_eq!(sp.pending(), 2);
        assert!(sp.take_results().is_empty(), "実行前は結果がない");
        assert_eq!(sp.run_jobs(), 2);
        let r = sp.take_results();
        assert!(matches!(&r[0], JobResult::Meta { job, .. } if *job == a));
        assert!(matches!(&r[1], JobResult::Failed { job, err: JobError::Fs(_) } if *job == b));
        assert_eq!(sp.pending(), 0);
    }

    #[test]
    fn 未実行のジョブの取消しは_cancelled_を積む() {
        let (sp, _fs) = setup();
        let a = sp.spawn(None, Revision(0), meta("/w/a.txt"));
        let b = sp.spawn(None, Revision(0), meta("/w/a.txt"));
        sp.cancel(a);
        sp.cancel(JobId(99)); // 存在しないものは何もしない
        assert_eq!(sp.run_jobs(), 1);
        let r = sp.take_results();
        assert!(matches!(&r[0], JobResult::Failed { job, err: JobError::Cancelled } if *job == a));
        assert!(matches!(&r[1], JobResult::Meta { job, .. } if *job == b));
        sp.cancel(b); // 完了済みは何もしない
        assert!(sp.take_results().is_empty());
    }
}
