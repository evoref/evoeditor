//! ジョブの同期実行 `run_job` (eve-host のワーカーとテストが共用する)。

use crate::jobs::cancel::CancelToken;

use crate::io::fs::FileStamp;
use crate::io::fspath::FsPath;
use crate::io::io_jobs::{SaveParams, run_open, run_open_stdin, run_save};

use super::{
    ConfigFile, JobError, JobKind, JobOutput, JobPorts, JobRequest, JobResult, MetaPurpose,
};

/// ジョブを同期的に実行する。eve-host のワーカーとテストが同じ関数を使う (スレッドは呼び出し側が用意する)。
/// 開始前にキャンセルされていれば何もせず `Failed { Cancelled }` を返す (`OpenStdin` を除く。標準入力の取消しは「読むのを
/// やめて開く」意味で、終端は常に `StdinDone`、03 §9.2.2)。結果は `emit` に 1 つ渡す。
pub fn run_job(
    req: &JobRequest,
    ports: &JobPorts,
    cancel: &CancelToken,
    emit: &mut dyn FnMut(JobOutput),
) {
    if cancel.is_cancelled() && !matches!(req.kind, JobKind::OpenStdin { .. }) {
        emit(JobOutput {
            job: req.id,
            body: JobResult::Failed {
                err: JobError::Cancelled,
            },
        });
        return;
    }
    let body = match &req.kind {
        JobKind::LastVersion { current } => JobResult::LastVersionChecked {
            outcome: crate::io::last_version::check_and_record(&*ports.fs, current),
        },
        JobKind::RecoveryScan => JobResult::RecoveryFound {
            dirs: ports.fs.list_recovery_dirs().unwrap_or_default(),
            swaps: ports.fs.list_swaps().unwrap_or_default(),
            leftovers: crate::io::leftover::scan(&*ports.fs),
        },
        JobKind::Open { path, opts } => run_open(req.id, path, *opts, ports, cancel, emit),
        JobKind::OpenStdin { opts } => run_open_stdin(req.id, *opts, ports, cancel, emit),
        JobKind::Save {
            path,
            snapshot,
            encoding,
            bom,
            escape,
            expected,
            strategy,
            clear_readonly,
            create_link_target,
            preserved,
            escape_merge_ok,
        } => run_save(
            SaveParams {
                path,
                snapshot,
                encoding: *encoding,
                bom: *bom,
                escape: *escape,
                expected: *expected,
                strategy: *strategy,
                clear_readonly: *clear_readonly,
                create_link_target: *create_link_target,
                preserved,
                escape_merge_ok: escape_merge_ok.as_ref(),
            },
            ports,
            cancel,
        ),
        JobKind::ReplaceUnmappable {
            snapshot,
            encoding,
            escape,
            preserved,
        } => match crate::io::unmap::build_replacement(
            snapshot, *encoding, *escape, preserved, cancel,
        ) {
            Ok(Some((changes, targets, counts))) => JobResult::UnmappableReplacement {
                changes,
                targets,
                counts,
            },
            // 対象が無い (文書が変わって表現不能が消えた等): 空の置換として返す
            Ok(None) => JobResult::UnmappableReplacement {
                changes: eve_text::ChangeSet::identity(eve_text::TextRead::len_bytes(snapshot)),
                targets: Vec::new(),
                counts: crate::io::unmap::ReplaceCounts::default(),
            },
            Err(()) => JobResult::Failed {
                err: JobError::Cancelled,
            },
        },
        JobKind::FindNext {
            snapshot,
            query,
            from,
            forward,
            resume,
        } => {
            let p = crate::search::FindJob {
                snapshot,
                query,
                from: *from,
                forward: *forward,
                resume,
            };
            // 時計を渡す (核の縮めと処理の速さの下限、03 §12.3.1)
            match crate::search::find_next(&p, Some(&*ports.clock), cancel) {
                Ok(found) => JobResult::FindNextDone { found },
                Err(e) => match e.limit() {
                    Some(limit) => JobResult::SearchLimited { limit },
                    None => JobResult::Failed {
                        err: JobError::Cancelled,
                    },
                },
            }
        }
        JobKind::ConvertEol { snapshot, eol } => {
            match crate::document::eol::build_eol_conversion(snapshot, *eol, cancel) {
                Some(changes) => JobResult::ConvertEolDone { changes },
                None => JobResult::Failed {
                    err: JobError::Cancelled,
                },
            }
        }
        JobKind::ReplaceAll {
            snapshot,
            query,
            replacement,
        } => match crate::search::build_replace_all_in(
            snapshot,
            query,
            replacement,
            Some(&*ports.clock),
            cancel,
        ) {
            Ok(Some(b)) => JobResult::ReplaceAllDone {
                changes: b.changes,
                count: b.count,
                folded: b.folded,
            },
            Ok(None) => JobResult::Failed {
                err: JobError::Cancelled,
            },
            Err(e) => match e.limit() {
                Some(limit) => JobResult::SearchLimited { limit },
                None => JobResult::Failed {
                    err: JobError::Cancelled,
                },
            },
        },
        JobKind::ReplaceOne {
            snapshot,
            query,
            from,
            range,
            replacement,
        } => match crate::search::replace_one_job(
            snapshot,
            query,
            *from,
            *range,
            replacement,
            Some(&*ports.clock),
            cancel,
        ) {
            Ok(insert) => JobResult::ReplaceOneDone { insert },
            Err(e) => match e.limit() {
                Some(limit) => JobResult::SearchLimited { limit },
                None => JobResult::Failed {
                    err: JobError::Cancelled,
                },
            },
        },
        JobKind::Search {
            snapshot,
            query,
            from,
            max_matches,
            count_limit,
        } => {
            crate::search::run_search(
                crate::search::SearchJob {
                    job: req.id,
                    snapshot,
                    query,
                    from: *from,
                    max_matches: *max_matches,
                    count_limit: *count_limit,
                },
                &*ports.clock,
                cancel,
                emit,
            );
            return;
        }
        JobKind::WriteRecovery { doc } => match ports.fs.write_recovery(doc) {
            Ok(path) => JobResult::RecoveryWritten { path },
            Err(err) => JobResult::Failed {
                err: JobError::Fs(err),
            },
        },
        JobKind::Swap { doc } => match ports.fs.write_swap(doc) {
            Ok(()) => JobResult::SwapWritten,
            Err(err) => JobResult::Failed {
                err: JobError::Fs(err),
            },
        },
        JobKind::SwapRead { entry } => match ports.fs.read_swap(entry) {
            Ok(body) => JobResult::SwapRead { body },
            Err(err) => JobResult::Failed {
                err: JobError::Fs(err),
            },
        },
        JobKind::SwapRemove { entry } => match ports.fs.remove_swap(entry) {
            Ok(()) => JobResult::SwapRemoved,
            Err(err) => JobResult::Failed {
                err: JobError::Fs(err),
            },
        },
        JobKind::LeftoverAct { leftover, op } => {
            match crate::io::leftover::act(&*ports.fs, leftover, *op) {
                Ok(()) => JobResult::LeftoverDone,
                Err(err) => JobResult::Failed {
                    err: JobError::Fs(err),
                },
            }
        }
        JobKind::SwapDiscard { doc, path } => match ports.fs.discard_swap(*doc, path.as_ref()) {
            Ok(()) => JobResult::SwapDiscarded,
            Err(err) => JobResult::Failed {
                err: JobError::Fs(err),
            },
        },
        JobKind::ReadTheme { dir, name, origin } => {
            let read = |n: &str| -> Result<Option<String>, String> {
                match read_config_file(&*ports.fs, &dir.join(format!("{n}.toml"))) {
                    Ok(Some(b)) => String::from_utf8(b)
                        .map(Some)
                        .map_err(|_| "UTF-8 として読めません".to_owned()),
                    Ok(None) => Ok(None),
                    Err(e) => Err(e),
                }
            };
            let (theme, reports) =
                eve_config::theme_file::load_theme_at(name, origin.as_ref(), &read);
            JobResult::ThemeRead {
                name: name.clone(),
                theme,
                reports,
            }
        }
        JobKind::ReadConfig { files, keymap } => JobResult::ConfigRead {
            config: files
                .iter()
                .map(|f| read_config_file(&*ports.fs, f))
                .collect(),
            keymap: keymap.as_ref().map(|k| read_config_file(&*ports.fs, k)),
        },
        JobKind::Metadata { path, purpose } => match ports.fs.metadata(path) {
            Ok(meta) => {
                if cancel.is_cancelled() {
                    JobResult::Failed {
                        err: JobError::Cancelled,
                    }
                } else {
                    // 未変更の保存の可否と外部変更の確認は、16 MiB 以下なら内容ハッシュでも照合する (05 §5.5)。読めなければ付けない
                    let hash = if matches!(
                        purpose,
                        MetaPurpose::UnchangedSave | MetaPurpose::ExternalCheck
                    ) {
                        crate::io::content_hash::hash_file(ports.fs.as_ref(), path, meta.len)
                            .ok()
                            .flatten()
                    } else {
                        None
                    };
                    let stamp = FileStamp::from_meta(&meta, hash);
                    JobResult::Meta { meta, stamp }
                }
            }
            Err(err) => JobResult::Failed {
                err: JobError::Fs(err),
            },
        },
    };
    emit(JobOutput { job: req.id, body });
}

/// 設定ファイルの大きさの上限 (これを超えるものは設定ではないとみなして読まない)。
const CONFIG_FILE_MAX: usize = 4 << 20;

fn read_config_file(fs: &dyn crate::io::fs::FileSystem, path: &FsPath) -> ConfigFile {
    let mut r = match fs.open_read(path) {
        Ok(r) => r,
        Err(e) if e.kind == crate::io::fs::FsErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{:?}", e.kind)),
    };
    let mut out = Vec::new();
    loop {
        match r.next_chunk() {
            Ok(Some(c)) => {
                out.extend_from_slice(c);
                if out.len() > CONFIG_FILE_MAX {
                    return Err("too large".to_owned());
                }
            }
            Ok(None) => return Ok(Some(out)),
            Err(e) => return Err(format!("{e:?}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::fs::*;
    use crate::io::paths::Paths;
    use crate::jobs::SaveStrategy;
    use crate::time::{Clock, Tick};
    use eve_encoding::{Encoding, EscapePolicy};
    use eve_text::Text;
    use eve_view::{JobId, Revision};
    use std::sync::Arc;

    struct StubFs(Result<FileMeta, FsErrorKind>);
    impl FileSystem for StubFs {
        fn metadata(&self, _: &FsPath) -> Result<FileMeta, FsError> {
            self.0.clone().map_err(FsError::new)
        }
        fn canonicalize(&self, p: &FsPath) -> Result<FsPath, FsError> {
            Ok(p.clone())
        }
        fn open_read(&self, _: &FsPath) -> Result<Box<dyn ReadChunks + Send>, FsError> {
            Err(FsError::new(FsErrorKind::Other))
        }
        fn write_atomic(
            &self,
            _: &FsPath,
            _: &mut dyn ReadChunks,
            _: &SaveOptions,
        ) -> Result<SaveReport, FsError> {
            Err(FsError::new(FsErrorKind::Other))
        }
    }
    struct StubClock;
    impl Clock for StubClock {
        fn now(&self) -> Tick {
            Tick(0)
        }
    }
    struct StubPaths;
    impl Paths for StubPaths {
        fn config_dir(&self) -> FsPath {
            FsPath::from("/c")
        }
        fn state_dir(&self) -> FsPath {
            FsPath::from("/s")
        }
        fn data_dir(&self) -> FsPath {
            FsPath::from("/d")
        }
        fn runtime_dir(&self) -> FsPath {
            FsPath::from("/r")
        }
        fn normalize(&self, raw: &str) -> Result<FsPath, FsError> {
            Ok(FsPath::from(raw))
        }
    }

    fn meta() -> FileMeta {
        FileMeta {
            kind: FileKind::File,
            len: 3,
            mtime: FileTime::default(),
            readonly: false,
            file_id: None,
            nlink: 1,
            link: None,
            fs_kind: FsKind::Local,
            owner_is_me: None,
        }
    }

    fn ports(r: Result<FileMeta, FsErrorKind>) -> JobPorts {
        JobPorts {
            stdin: None,
            fs: Arc::new(StubFs(r)),
            clock: Arc::new(StubClock),
            paths: Arc::new(StubPaths),
        }
    }

    fn req() -> JobRequest {
        JobRequest {
            id: JobId(7),
            doc: None,
            base: Revision(0),
            kind: JobKind::Metadata {
                path: FsPath::from("/a"),
                purpose: MetaPurpose::ExternalCheck,
            },
        }
    }

    fn run(p: &JobPorts, c: &CancelToken) -> Vec<JobOutput> {
        let mut out = Vec::new();
        run_job(&req(), p, c, &mut |r| out.push(r));
        out
    }

    #[test]
    fn メタ情報のジョブは_stamp_を返す() {
        let out = run(&ports(Ok(meta())), &CancelToken::new());
        assert_eq!(out.len(), 1);
        let JobOutput { job, body } = &out[0];
        match body {
            JobResult::Meta { stamp, .. } => {
                assert_eq!(*job, JobId(7));
                assert_eq!(stamp.len, 3);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn 失敗は_failed_で返る() {
        let out = run(&ports(Err(FsErrorKind::NotFound)), &CancelToken::new());
        assert!(
            matches!(&out[0].body, JobResult::Failed { err: JobError::Fs(e), .. } if e.kind == FsErrorKind::NotFound)
        );
    }

    #[test]
    fn 開始前のキャンセルは_cancelled() {
        let c = CancelToken::new();
        c.cancel();
        let out = run(&ports(Ok(meta())), &c);
        assert_eq!(out.len(), 1);
        assert!(matches!(
            &out[0].body,
            JobResult::Failed {
                err: JobError::Cancelled,
                ..
            }
        ));
    }

    /// `write_atomic` の途中で取消しが届き (利用者が「保存を中止して終了」を選んだ)、そのあと Port が `kind` で失敗する FS。
    struct CancelledMidSave {
        cancel: CancelToken,
        kind: FsErrorKind,
    }
    impl FileSystem for CancelledMidSave {
        fn metadata(&self, _: &FsPath) -> Result<FileMeta, FsError> {
            Ok(meta())
        }
        fn canonicalize(&self, p: &FsPath) -> Result<FsPath, FsError> {
            Ok(p.clone())
        }
        fn open_read(&self, _: &FsPath) -> Result<Box<dyn ReadChunks + Send>, FsError> {
            Err(FsError::new(FsErrorKind::Other))
        }
        fn write_atomic(
            &self,
            _: &FsPath,
            _: &mut dyn ReadChunks,
            _: &SaveOptions,
        ) -> Result<SaveReport, FsError> {
            self.cancel.cancel();
            Err(FsError::new(self.kind.clone()))
        }
    }

    fn save_cancelled_mid(kind: FsErrorKind) -> JobResult {
        let cancel = CancelToken::new();
        let p = JobPorts {
            stdin: None,
            fs: Arc::new(CancelledMidSave {
                cancel: cancel.clone(),
                kind,
            }),
            clock: Arc::new(StubClock),
            paths: Arc::new(StubPaths),
        };
        let r = JobRequest {
            id: JobId(9),
            doc: None,
            base: Revision(0),
            kind: JobKind::Save {
                path: FsPath::from("/a"),
                snapshot: Text::from("abc").snapshot(),
                encoding: Encoding::Utf8,
                bom: false,
                escape: EscapePolicy::Restore,
                expected: Expect::Any,
                strategy: SaveStrategy::default(),
                clear_readonly: false,
                create_link_target: false,
                preserved: crate::document::PreservedMap::new(),
                escape_merge_ok: None,
            },
        };
        let mut out = Vec::new();
        run_job(&r, &p, &cancel, &mut |x| out.push(x));
        out.pop().expect("終端の結果").body
    }

    // 05 §4.1 手順 5、NFR-301 AC-2 (docs/review/v0.3-findings.md の SAF-04)
    #[test]
    fn 置換を始めた後の失敗は_取消しが届いていても失敗のまま返す() {
        let backup = FsPath::from("/.a.eve-bak-1");
        let r = save_cancelled_mid(FsErrorKind::ReplacedMovedToBackup {
            backup: backup.clone(),
        });
        assert!(
            matches!(&r, JobResult::Failed { err: JobError::Fs(e), .. }
                if e.kind == FsErrorKind::ReplacedMovedToBackup { backup: backup.clone() }),
            "{r:?}"
        );
        let r = save_cancelled_mid(FsErrorKind::InPlaceFailed { backup: None });
        assert!(
            matches!(&r, JobResult::Failed { err: JobError::Fs(e), .. }
                if matches!(e.kind, FsErrorKind::InPlaceFailed { .. })),
            "{r:?}"
        );
    }

    #[test]
    fn 何も書いていない失敗は_取消しが届いていれば取消しとして返す() {
        let r = save_cancelled_mid(FsErrorKind::Conflict { current: None });
        assert!(
            matches!(
                r,
                JobResult::Failed {
                    err: JobError::Cancelled,
                    ..
                }
            ),
            "{r:?}"
        );
    }
}
