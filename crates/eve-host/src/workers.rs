//! 本番の `JobSpawner`: 固定のワーカースレッド (docs/02 §9.1、ADR-0006)。
//!
//! 現在は `io` ワーカー 1 本だけ (M1 のジョブはすべて読込・保存・metadata)。`bg` ワーカーは、最初の `bg` の種別
//! (検索・Undo の実体化など) が入る版で、最初の投入のときに遅延生成する。結果は `HostMsg::Job` で受信口へ送る。
//! ワーカーは `run_job` を呼ぶだけで、`Editor` にも `Ports` にも触れない。

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use eve_core::{
    CancelToken, JobError, JobKind, JobPorts, JobRequest, JobResult, JobSpawner, run_job,
};
use eve_view::{DocumentId, JobId, Revision};

use crate::msg::HostMsg;

/// `Drop` がワーカーの終了を待つ上限 (02 §11.2.1 の置換中フラグの待ちに合わせる)。
const JOIN_WAIT: Duration = Duration::from_secs(1);

struct Work {
    req: JobRequest,
    cancel: CancelToken,
}

pub struct ThreadJobSpawner {
    next_id: Cell<u64>,
    /// 実行中・待機中のジョブの取消しトークン。ワーカーが手放した (他に持つ者がいない) ものは投入のときに掃除する。
    tokens: RefCell<HashMap<JobId, CancelToken>>,
    queue: RefCell<Option<Sender<Work>>>,
    thread: RefCell<Option<JoinHandle<()>>>,
    /// ワーカーが終わると送信側が閉じる (Drop の待ちに上限を付けるため)。
    done: mpsc::Receiver<()>,
    tx: Sender<HostMsg>,
}

impl ThreadJobSpawner {
    /// `tx` は Host の受信口への送信口 (`Host::sender`、または `channel` で作ったもの)。
    pub fn new(ports: JobPorts, tx: Sender<HostMsg>) -> std::io::Result<ThreadJobSpawner> {
        let (qtx, qrx) = mpsc::channel::<Work>();
        let (done_tx, done) = mpsc::channel::<()>();
        let host_tx = tx.clone();
        let handle = thread::Builder::new()
            .name("eve-io".to_owned())
            .spawn(move || {
                let _done = done_tx;
                // 送信口が閉じる (spawner が drop される) までキューを直列に処理する
                while let Ok(Work { req, cancel }) = qrx.recv() {
                    let job = req.id;
                    let outcome = catch_unwind(AssertUnwindSafe(|| {
                        run_job(&req, &ports, &cancel, &mut |r| {
                            let _ = tx.send(HostMsg::Job(Box::new(r)));
                        });
                    }));
                    if outcome.is_err() {
                        let _ = tx.send(HostMsg::Job(Box::new(JobResult::Failed {
                            job,
                            err: JobError::Panic,
                        })));
                    }
                }
            })?;
        Ok(ThreadJobSpawner {
            next_id: Cell::new(1),
            tokens: RefCell::default(),
            queue: RefCell::new(Some(qtx)),
            thread: RefCell::new(Some(handle)),
            done,
            tx: host_tx,
        })
    }
}

impl JobSpawner for ThreadJobSpawner {
    fn spawn(&self, doc: Option<DocumentId>, base: Revision, kind: JobKind) -> JobId {
        let id = JobId(self.next_id.get());
        self.next_id.set(id.0 + 1);
        let cancel = CancelToken::new();
        {
            let mut tokens = self.tokens.borrow_mut();
            // ワーカーが持っていないトークンは完了済み
            tokens.retain(|_, t| t.is_shared());
            tokens.insert(id, cancel.clone());
        }
        let work = Work {
            req: JobRequest {
                id,
                doc,
                base,
                kind,
            },
            cancel,
        };
        if let Some(q) = self.queue.borrow().as_ref()
            && q.send(work).is_err()
        {
            // ワーカーが死んでいる: ジョブ表の行が残らないよう、その場で終端の結果を返す
            let _ = self.tx.send(HostMsg::Job(Box::new(JobResult::Failed {
                job: id,
                err: JobError::Panic,
            })));
        }
        id
    }

    fn cancel(&self, job: JobId) {
        if let Some(t) = self.tokens.borrow().get(&job) {
            t.cancel();
        }
    }
}

impl Drop for ThreadJobSpawner {
    fn drop(&mut self) {
        // 残りのジョブを取り消し (run_job は置換を始めた保存を止めずに完了させる)、キューを閉じて終わるのを待つ
        for t in self.tokens.borrow().values() {
            t.cancel();
        }
        self.queue.borrow_mut().take();
        // 待つのは短い間だけ。OS の呼出しで止まったワーカーは切り離す (実行中の置換は run_job が完了させる)
        let finished = matches!(
            self.done.recv_timeout(JOIN_WAIT),
            Err(RecvTimeoutError::Disconnected)
        );
        if let Some(h) = self.thread.borrow_mut().take()
            && finished
        {
            let _ = h.join();
        }
    }
}
