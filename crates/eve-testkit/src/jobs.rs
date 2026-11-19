//! `ManualJobSpawner`: テストのジョブ実行の唯一の実装 (docs/02 §4.4)。
//!
//! `spawn` はキューに積むだけで実行しない。テストが `run_jobs` を呼ぶと、積まれた順に **同じスレッドで** `run_job` を
//! 実行し、結果を inbox に積む。`cancel` は未実行ならキューから外して `Cancelled` を積む。標準入力の読込 (`OpenStdin`)
//! だけは、取消しが「読むのをやめて開く」意味 (終端は `StdinDone`、02 §9.3) なので、キューに残して取消しを立て、
//! `JobPorts.stdin` の `wake` を呼ぶ (本番の `JobSpawner` と同じ)。
//!
//! 先行表示 (docs/03 §9.2.1) の試験のために、`run_jobs_until_partial` は最初に `Partial` を送ったジョブの、`Partial` より
//! 後の結果を取り置く (読込の途中で止まっている状態を作る)。取り置いた結果は `release_held` で inbox に移し、その間の
//! `cancel` は取り置いた結果を捨てて `Cancelled` を積む (実際のジョブが次のチャンクの区切りで取消しに気づくのと同じ)。

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use eve_core::{
    CancelToken, JobError, JobKind, JobOutput, JobPorts, JobRequest, JobResult, JobSpawner, run_job,
};
use eve_view::{DocumentId, JobId, Revision};

struct Queued {
    req: JobRequest,
    cancel: CancelToken,
}

/// 積むだけで実行しない `JobSpawner` (テスト用)。`run_jobs` で積んだ順に同じスレッドで実行する。
/// 単一スレッド専用 (`Rc` で共有する)。
pub struct ManualJobSpawner {
    ports: JobPorts,
    next_id: Cell<u64>,
    queue: RefCell<VecDeque<Queued>>,
    inbox: RefCell<VecDeque<JobOutput>>,
    /// `run_jobs_until_partial` が取り置いた結果 (先行表示の先頭より後の進捗・完了)。
    held: RefCell<Vec<JobOutput>>,
}

impl ManualJobSpawner {
    /// 空のキューで作る。`ports` は `run_job` に渡す。
    pub fn new(ports: JobPorts) -> Rc<ManualJobSpawner> {
        Rc::new(ManualJobSpawner {
            ports,
            next_id: Cell::new(1),
            queue: RefCell::default(),
            inbox: RefCell::default(),
            held: RefCell::default(),
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

    /// 積まれたジョブを積まれた順に実行し、最初に `JobResult::Partial` を送ったジョブで止まる: そのジョブの `Partial`
    /// までの結果は inbox に積み、後の結果は取り置く (`release_held` で積む)。残りのジョブは実行しない (読込ジョブは直列)。
    /// `Partial` を送ったジョブがあれば `true`。
    pub fn run_jobs_until_partial(&self) -> bool {
        loop {
            let Some(job) = self.queue.borrow_mut().pop_front() else {
                return false;
            };
            let mut out = Vec::new();
            run_job(&job.req, &self.ports, &job.cancel, &mut |r| out.push(r));
            if let Some(at) = out
                .iter()
                .position(|o| matches!(o.body, JobResult::Partial { .. }))
            {
                let rest = out.split_off(at + 1);
                self.inbox.borrow_mut().extend(out);
                self.held.borrow_mut().extend(rest);
                return true;
            }
            self.inbox.borrow_mut().extend(out);
        }
    }

    /// 取り置いた結果を inbox に積む。
    pub fn release_held(&self) {
        let held: Vec<JobOutput> = self.held.borrow_mut().drain(..).collect();
        self.inbox.borrow_mut().extend(held);
    }

    /// 取り置いた結果の数。
    pub fn held(&self) -> usize {
        self.held.borrow().len()
    }

    /// 実行待ちのジョブをすべて取り消す (それぞれ `Cancelled` の結果が inbox に積まれる)。
    pub fn cancel_all(&self) {
        let ids: Vec<JobId> = self.queue.borrow().iter().map(|j| j.req.id).collect();
        for id in ids {
            self.cancel(id);
        }
    }

    /// 結果の inbox を空にして返す (Host の受信口の代わり)。
    pub fn take_results(&self) -> Vec<JobOutput> {
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
        let cancelled = JobOutput {
            job,
            body: JobResult::Failed {
                err: JobError::Cancelled,
            },
        };
        let mut q = self.queue.borrow_mut();
        // 標準入力の読込: 取消しを立てて待ちを起こす (キューに残し、`run_jobs` で `StdinDone { Stopped }` を返させる)
        if let Some(j) = q
            .iter()
            .find(|j| j.req.id == job && matches!(j.req.kind, JobKind::OpenStdin { .. }))
        {
            j.cancel.cancel();
            if let Some(s) = &self.ports.stdin {
                s.wake();
            }
            return;
        }
        if let Some(pos) = q.iter().position(|j| j.req.id == job) {
            let j = q.remove(pos).expect("位置は直前に見つけた");
            j.cancel.cancel();
            self.inbox.borrow_mut().push_back(cancelled);
            return;
        }
        // 先行表示の後で取り置いたジョブ: 残りの結果の代わりに `Cancelled` を積む
        let mut held = self.held.borrow_mut();
        if held.iter().any(|o| o.job == job) {
            held.retain(|o| o.job != job);
            self.inbox.borrow_mut().push_back(cancelled);
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
            stdin: None,
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
        assert!(matches!(&r[0], JobOutput { job, body: JobResult::Meta { .. } } if *job == a));
        assert!(
            matches!(&r[1], JobOutput { job, body: JobResult::Failed { err: JobError::Fs(_) } } if *job == b)
        );
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
        assert!(
            matches!(&r[0], JobOutput { job, body: JobResult::Failed { err: JobError::Cancelled } } if *job == a)
        );
        assert!(matches!(&r[1], JobOutput { job, body: JobResult::Meta { .. } } if *job == b));
        sp.cancel(b); // 完了済みは何もしない
        assert!(sp.take_results().is_empty());
    }
}
