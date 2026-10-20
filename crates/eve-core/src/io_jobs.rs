//! 読込・保存ジョブの本体 (docs/03 §9.2 / §9.3)。`run_job` から呼ばれ、ワーカー (またはテストの `ManualJobSpawner`) で動く。
//!
//! M1 の範囲: UTF-8 だけを開く。非 UTF-8 の判定 (UTF-16・Shift_JIS 等)、巨大ファイルの確認、進捗、保存中フラグ、
//! 内容ハッシュは、それぞれを使う版で足す。保存は `FileSystem::write_atomic` (原子的保存) だけ。

use eve_encoding::{
    DetectOptions, DetectedKind, Detection, PuaEscapeMode, Utf8Decoder, Utf8Encoder, detect,
};
use eve_text::{ByteRange, LineEnding, Snapshot, TextBuilder, TextRead};
use eve_view::JobId;

use crate::cancel::CancelToken;
use crate::document::{EolStats, ReadOnlyReason, ReadOnlyReasons, TextFormat};
use crate::fs::{
    ChunkError, Expect, FileKind, FileStamp, FsError, FsErrorDetail, FsErrorKind, ReadChunks,
    SaveOptions,
};
use crate::fspath::FsPath;
use crate::jobs::{ConfirmAsk, JobError, JobPorts, JobResult};

/// 先頭の判定に使うバイト数 (03 §8.2 の `sniff_len`)。
const SNIFF_LEN: usize = 64 * 1024;

/// 読込の上限の余裕 (`max(1 MiB, len / 16)`、03 §9.2)。読込中に伸び続けるファイルを際限なく読まない。
fn read_slack(len: u64) -> u64 {
    (1024 * 1024).max(len / 16)
}

/// 開くときの指定。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OpenOptions {
    /// 確認 (バイナリ・UTF-32 の BOM) への回答「読み取り専用で開く」の再投入。確認を出さず、読み取り専用で開く。
    pub read_only_requested: bool,
    /// `--readonly`。読み取り専用の理由 `UserRequested` を付けるだけで、確認は通常どおり出す。
    pub user_read_only: bool,
    /// 文字コードを UTF-8 と指定して開く (FR-111)。BOM の判定や確認を出さず本文として読む。
    pub forced_utf8: bool,
}

pub(crate) fn run_open(
    id: JobId,
    path: &FsPath,
    opts: OpenOptions,
    ports: &JobPorts,
    cancel: &CancelToken,
) -> JobResult {
    let fail = |err| JobResult::Failed { job: id, err };
    let cancelled = || JobResult::Failed {
        job: id,
        err: JobError::Cancelled,
    };

    // 2. metadata
    let meta = match ports.fs.metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind == FsErrorKind::NotFound => {
            let parent_missing = match e.detail {
                Some(FsErrorDetail::ParentMissing { dir }) => Some(dir),
                _ => None,
            };
            return JobResult::NewFile {
                job: id,
                parent_missing,
            };
        }
        Err(e) => return fail(JobError::Fs(e)),
    };
    if meta.kind != FileKind::File {
        return fail(JobError::Fs(FsError::with_detail(
            FsErrorKind::InvalidPath,
            FsErrorDetail::NotAFile,
        )));
    }
    if cancel.is_cancelled() {
        return cancelled();
    }
    let stamp_before = FileStamp::from_meta(&meta, None);

    // 3. 読み始める
    let mut reader = match ports.fs.open_read(path) {
        Ok(r) => r,
        Err(e) => return fail(JobError::Fs(e)),
    };
    let limit = meta.len.saturating_add(read_slack(meta.len));
    let mut read_total: u64 = 0;
    let mut builder = TextBuilder::new();
    let mut decoder: Option<Utf8Decoder> = None;
    let mut sniffed = false;
    let mut binary_confirmed = false;
    let mut out = String::new();
    let mut truncated = false;
    loop {
        if cancel.is_cancelled() {
            return cancelled();
        }
        let chunk = match reader.next_chunk() {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(ChunkError::Io(e)) => return fail(JobError::Fs(e)),
            Err(_) => return fail(JobError::Fs(FsError::new(FsErrorKind::Other))),
        };
        let mut chunk: &[u8] = chunk;
        if !sniffed {
            sniffed = true;
            let head = &chunk[..chunk.len().min(SNIFF_LEN)];
            let file_complete = meta.len as usize <= head.len();
            let det_opts = DetectOptions {
                forced: opts
                    .forced_utf8
                    .then_some(eve_encoding::ForcedEncoding::Utf8),
            };
            let Detection { kind, .. } = detect(head, file_complete, &det_opts);
            match kind {
                DetectedKind::Binary if !opts.read_only_requested => {
                    return JobResult::NeedsConfirm {
                        job: id,
                        ask: ConfirmAsk::Binary,
                    };
                }
                DetectedKind::UnsupportedBom(kind) if !opts.read_only_requested => {
                    return JobResult::NeedsConfirm {
                        job: id,
                        ask: ConfirmAsk::UnsupportedBom { kind },
                    };
                }
                // 確認への回答「読み取り専用で開く」の再投入: バイナリ・未対応の BOM も本文のバイトとして UTF-8 で読む
                DetectedKind::Binary => binary_confirmed = true,
                _ => {}
            }
            let skip_bom = matches!(kind, DetectedKind::Utf8 { bom_len: 3 });
            decoder = Some(Utf8Decoder::new(skip_bom));
        }
        // 読む量の上限
        let room = limit.saturating_sub(read_total) as usize;
        if chunk.len() > room {
            chunk = &chunk[..room];
            truncated = true;
        }
        read_total += chunk.len() as u64;
        let Some(dec) = decoder.as_mut() else { break };
        out.clear();
        dec.decode(chunk, false, &mut out);
        builder.append(&out);
        if truncated {
            break;
        }
    }
    // 空のファイルは sniff なしで終わる
    let mut dec = decoder.unwrap_or_else(|| Utf8Decoder::new(true));
    out.clear();
    dec.decode(&[], true, &mut out);
    builder.append(&out);
    let text = builder.finish();

    // 読込前後の metadata を比較する (05 §4.5)
    let changed_during_load = truncated
        || ports.fs.metadata(path).map_or(true, |after| {
            let a = FileStamp::from_meta(&after, None);
            a.mtime != stamp_before.mtime || a.len != stamp_before.len
        });

    // 形式: BOM・不正バイト・改行の多数派 (同数なら LF、改行がなければ LF)
    let eol_stats = EolStats::count(&text);
    let format = TextFormat {
        has_bom: dec.has_bom(),
        default_eol: eol_stats.majority(LineEnding::Lf),
        eol_stats,
        invalid_bytes: dec.invalid_bytes(),
        pua_escape: dec.pua_mode(),
    };
    let mut detected = ReadOnlyReasons::default();
    if meta.readonly {
        detected.insert(ReadOnlyReason::FileAttribute);
    }
    if opts.read_only_requested || opts.user_read_only {
        detected.insert(ReadOnlyReason::UserRequested);
    }
    if binary_confirmed {
        detected.insert(ReadOnlyReason::Binary);
    }
    // 不正バイトがあるとき、または元から真の私用文字と衝突しているとき (03 §8.3) は読み取り専用
    if format.invalid_bytes > 0 || dec.collision_with_invalid() {
        detected.insert(ReadOnlyReason::InvalidBytes);
    }
    // 衝突があっても不正バイトが 0 ならエスケープを使わないので編集できる
    // 読込前の stamp を保存の照合に使う。読込中に変わったファイルは、そのまま上書き保存すると照合で Conflict になる
    let stamp = stamp_before;
    JobResult::Complete {
        job: id,
        text,
        format,
        meta,
        stamp,
        changed_during_load,
        detected,
    }
}

/// 保存のためのチャンク供給: Snapshot を UTF-8 に符号化して `write_atomic` に渡す。
struct SnapshotSource<'a> {
    chunks: eve_text::Chunks<'a>,
    enc: Utf8Encoder,
    buf: Vec<u8>,
    cancel: &'a CancelToken,
    started: bool,
}

impl ReadChunks for SnapshotSource<'_> {
    fn next_chunk(&mut self) -> Result<Option<&[u8]>, ChunkError> {
        if self.cancel.is_cancelled() {
            return Err(ChunkError::Aborted);
        }
        self.buf.clear();
        match self.chunks.next() {
            Some(s) => {
                self.enc.encode(s, &mut self.buf);
                self.started = true;
                Ok(Some(&self.buf))
            }
            None if !self.started => {
                // 空の文書: BOM だけを書く (なければ何も書かない)
                self.started = true;
                self.enc.encode("", &mut self.buf);
                Ok((!self.buf.is_empty()).then_some(&self.buf[..]))
            }
            None => Ok(None),
        }
    }
}

/// 保存ジョブの引数。
pub(crate) struct SaveParams<'a> {
    pub path: &'a FsPath,
    pub snapshot: &'a Snapshot,
    pub bom: bool,
    pub pua: PuaEscapeMode,
    pub expected: Expect,
}

pub(crate) fn run_save(
    id: JobId,
    p: SaveParams<'_>,
    ports: &JobPorts,
    cancel: &CancelToken,
) -> JobResult {
    let SaveParams {
        path,
        snapshot,
        bom,
        pua,
        expected,
    } = p;
    let mut src = SnapshotSource {
        chunks: snapshot.chunks(ByteRange::all(snapshot.len_bytes())),
        enc: Utf8Encoder::new(bom, pua),
        buf: Vec::new(),
        cancel,
        started: false,
    };
    match ports
        .fs
        .write_atomic(path, &mut src, &SaveOptions::user_file(expected))
    {
        Ok(report) => JobResult::Saved { job: id, report },
        Err(_) if cancel.is_cancelled() => JobResult::Failed {
            job: id,
            err: JobError::Cancelled,
        },
        Err(e) => JobResult::Failed {
            job: id,
            err: JobError::Fs(e),
        },
    }
}
