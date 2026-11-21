//! ファインダの候補の走査のジョブ `JobKind::WorkspaceScan` の 1 区切り (docs/03 §12.7.2 / §12.7.7)。

use std::sync::Arc;

use eve_view::JobId;

use crate::io::fspath::FsPath;
use crate::jobs::{JobError, JobKind, JobOutput, JobPorts, JobResult, JobStep};

use super::{ExcludeConfig, WalkStep, WorkspaceWalk};

/// 走査を 1 区切り進め、その間に溜まった塊を `ScanBatch` で送る。続きがあれば、状態を持った `JobKind::WorkspaceScan` を
/// `JobStep::Continue` で返す (レーンが同じ `JobId`・`CancelToken` のまま自分のキューの末尾へ積む、02 §9.3)。終われば
/// 残りの塊と `ScanDone`、ルートを読めなければ `Failed { Fs }` を送る。
pub(crate) fn run_scan(
    job: JobId,
    root: FsPath,
    exclude: Arc<ExcludeConfig>,
    generation: u64,
    walk: Option<Box<WorkspaceWalk>>,
    ports: &JobPorts,
    emit: &mut dyn FnMut(JobOutput),
) -> JobStep {
    let mut walk =
        walk.unwrap_or_else(|| Box::new(WorkspaceWalk::new(root.clone(), exclude.clone())));
    let step = walk.step(&*ports.fs, &*ports.clock);
    let finish = !matches!(step, WalkStep::Yield);
    for chunk in walk.take_chunks(ports.clock.now(), finish) {
        emit(JobOutput {
            job,
            body: JobResult::ScanBatch {
                generation,
                chunk,
                unreadable_dirs: walk.unreadable_dirs(),
            },
        });
    }
    let body = match step {
        WalkStep::Yield => {
            return JobStep::Continue(JobKind::WorkspaceScan {
                root,
                exclude,
                generation,
                walk: Some(walk),
            });
        }
        WalkStep::Done => JobResult::ScanDone {
            generation,
            total: walk.found(),
            truncated: walk.truncated(),
        },
        WalkStep::Failed(err) => JobResult::Failed {
            err: JobError::Fs(err),
        },
    };
    emit(JobOutput { job, body });
    JobStep::Done
}
