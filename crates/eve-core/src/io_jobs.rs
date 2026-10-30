//! 読込・保存ジョブの本体 (docs/03 §9.2 / §9.3)。`run_job` から呼ばれ、ワーカー (またはテストの `ManualJobSpawner`) で動く。
//!
//! 読込: 先頭 64 KiB で文字コードを判定し (03 §8.2)、チャンクごとに復号して `TextBuilder` に積む。不正バイトは PUA にエスケープして
//! 保持し (§8.3)、読込時に往復を検査し (§8.7)、信頼度が低いとき後半で別の文字コードが現れたら判定し直して 1 回だけやり直す (§8.2.1)。
//! 巨大ファイルの確認、進捗、保存中フラグ、内容ハッシュは、それぞれを使う版で足す。保存は `FileSystem::write_atomic` (原子的保存) だけで、
//! 非 UTF-8 では出力の前に乾式走査で表現不能の文字を探し、あれば何も書かずに `JobResult::Unmappable` を返す (§8.6 E1)。

use eve_encoding::{
    Confidence, Decoder, DetectOptions, DetectedKind, Detection, EncodeReport, Encoder, Encoding,
    EscapePolicy, PuaEscapeMode, RoundtripChecker, detect, find_iso_switch, redetect_window,
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
use crate::interrupted::map_interrupted;
use crate::jobs::{ConfirmAsk, JobError, JobPorts, JobResult, OpenNotes};

/// 先頭の判定に使うバイト数 (03 §8.2 の `sniff_len`)。
const SNIFF_LEN: usize = 64 * 1024;
/// 再判定の窓の大きさ (03 §8.2.1)。
const REDETECT_WINDOW: usize = 64 * 1024;
/// 不正バイトの比率の警告の閾値 (千分率。sniff 範囲で 1% 超、03 §8.3)。
const MANY_INVALID_PERMILLE: u32 = 10;
/// 巨大ファイルの警告の閾値 (`file.huge_file_warn_bytes` の既定、FR-127)。
const HUGE_FILE_WARN: u64 = 512 * 1024 * 1024;

/// 読込の上限の余裕 (`max(1 MiB, len / 16)`、03 §9.2)。読込中に伸び続けるファイルを際限なく読まない。
fn read_slack(len: u64) -> u64 {
    (1024 * 1024).max(len / 16)
}

/// 開くときの指定。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OpenOptions {
    /// 確認 (バイナリ・UTF-32 の BOM・不正バイトが多い) への回答「読み取り専用で開く」の再投入。確認を出さず、読み取り専用で開く。
    pub read_only_requested: bool,
    /// `--readonly`。読み取り専用の理由 `UserRequested` を付けるだけで、確認は通常どおり出す。
    pub user_read_only: bool,
    /// 文字コードを指定して開く (FR-111、`--encoding`)。判定せず、BOM は指定した文字コードのものだけ読み飛ばす (03 §8.2 手順 2)。
    pub forced: Option<Encoding>,
    /// 確認「巨大ファイル」への回答「読み取り専用で開く」の再投入。巨大ファイルの確認を出さない。
    pub confirmed_large: bool,
    /// 改行がない・LF と CR と CRLF が同数のときの `default_eol` (`file.default_line_ending`、03 §8.4)。`None` は LF。
    pub default_eol: Option<LineEnding>,
    /// 確認の回答で決まった読み取り専用の理由 (ジョブは読まずに引き継ぐ、03 §9.2)。
    pub pending: ReadOnlyReasons,
    /// 開く前に警告する大きさ (`file.huge_file_warn_bytes`、FR-127)。`None` は既定 (512 MiB)。
    pub huge_file_warn_bytes: Option<u64>,
    /// 長大行の閾値 (`file.long_line_threshold`、NFR-106)。`None` は既定 (1 MiB)。
    pub long_line_threshold: Option<usize>,
}

pub(crate) fn run_open(
    id: JobId,
    path: &FsPath,
    opts: OpenOptions,
    ports: &JobPorts,
    cancel: &CancelToken,
    emit: &mut dyn FnMut(JobResult),
) -> JobResult {
    open_inner(id, path, opts, ports, cancel, emit, None)
}

/// 先頭の判定の結果 (1 チャンク目で決まる)。
struct Plan {
    enc: Encoding,
    bom_len: usize,
    confidence: Confidence,
    binary_confirmed: bool,
    guess_unsupported: Option<&'static str>,
}

/// 判定し直した位置の周辺の窓 (03 §8.2.1)。信頼度「低」の UTF-8 で、不正な UTF-8 または ISO-2022-JP のエスケープ列を
/// 初めて見つけたときだけ判定し直す。別の文字コードが推定されればその文字コード。
fn redetect(chunk: &[u8], chunk_is_file_end: bool, opts: &DetectOptions) -> Option<Encoding> {
    // 契機: ISO-2022-JP のエスケープ列 (窓の先頭をその位置にする) か、不正な UTF-8 (その周辺)
    let (start, iso) = match find_iso_switch(chunk) {
        Some(p) => (p, true),
        None => match std::str::from_utf8(chunk) {
            Ok(_) => return None,
            // 末尾の切れた多バイト列だけなら不正ではない
            Err(e) if e.error_len().is_none() && !chunk_is_file_end => return None,
            Err(e) => (e.valid_up_to().saturating_sub(REDETECT_WINDOW / 2), false),
        },
    };
    let end = (start + REDETECT_WINDOW).min(chunk.len());
    let window = &chunk[start..end];
    // ISO-2022-JP の契機で窓が 7 bit なら手順 4' だけ (redetect_window が行う)
    let complete = end == chunk.len() && chunk_is_file_end;
    let _ = iso;
    redetect_window(window, complete, opts).and_then(|d| d.encoding())
}

fn open_inner(
    id: JobId,
    path: &FsPath,
    opts: OpenOptions,
    ports: &JobPorts,
    cancel: &CancelToken,
    emit: &mut dyn FnMut(JobResult),
    redetected: Option<Encoding>,
) -> JobResult {
    let fail = |err| JobResult::Failed { job: id, err };
    let cancelled = || JobResult::Failed {
        job: id,
        err: JobError::Cancelled,
    };

    // 1. 残っている保存中フラグ (開くたびに 1 回。ディレクトリは列挙しない)。見つけた痕跡は T の有無と合わせて写像する
    let inflight = ports.fs.inflight_for(path).unwrap_or_default();

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
                interrupted_save: map_interrupted(ports.fs.as_ref(), &inflight, false),
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
    // 巨大ファイル (FR-127): 読み始める前に確認する
    if !opts.confirmed_large && meta.len >= opts.huge_file_warn_bytes.unwrap_or(HUGE_FILE_WARN) {
        return JobResult::NeedsConfirm {
            job: id,
            ask: ConfirmAsk::HugeFile { len: meta.len },
        };
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
    let det_opts = DetectOptions::default().with_forced(opts.forced);
    let mut plan: Option<Plan> = None;
    let mut decoder: Option<Decoder> = None;
    let mut checker: Option<RoundtripChecker> = None;
    let mut redetect_checked = false;
    let mut out = String::new();
    let mut truncated = false;
    // 進捗: 読み始めてすぐ (開く操作から 50 ms 以内) に 1 件、以後は 100 ms 以上あけて (10 Hz 以下、03 §9.2)
    emit(JobResult::Progress {
        job: id,
        bytes: 0,
        total: meta.len,
    });
    let mut last_progress = ports.clock.now();
    // 内容ハッシュ (16 MiB 以下。読み込みながら計算する、05 §5.5)
    let mut hasher = (meta.len <= crate::content_hash::HASH_MAX_LEN)
        .then(crate::content_hash::ContentHasher::new);
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
        if let Some(h) = hasher.as_mut() {
            h.update(chunk);
        }
        let first = plan.is_none();
        if first {
            let head = &chunk[..chunk.len().min(SNIFF_LEN)];
            let file_complete = meta.len as usize <= head.len();
            let Detection {
                kind,
                confidence,
                unsupported_guess,
            } = detect(head, file_complete, &det_opts);
            let mut binary_confirmed = false;
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
                DetectedKind::Binary | DetectedKind::UnsupportedBom(_) => binary_confirmed = true,
                _ => {}
            }
            let (enc, bom_len) = match kind {
                DetectedKind::Utf8 { bom_len } => (Encoding::Utf8, bom_len),
                DetectedKind::Encoded { encoding, bom_len } => (encoding, bom_len),
                _ => (Encoding::Utf8, 0),
            };
            // 不正バイトが多いファイルは、本文の構築を始める前に確認する (03 §8.3)
            if !opts.read_only_requested {
                let mut probe = Decoder::new(enc, bom_len > 0);
                let mut sink = String::new();
                probe.decode(head, file_complete, &mut sink);
                let permille =
                    (u64::from(probe.invalid_bytes()) * 1000 / head.len().max(1) as u64) as u32;
                if permille > MANY_INVALID_PERMILLE {
                    return JobResult::NeedsConfirm {
                        job: id,
                        ask: ConfirmAsk::ManyInvalidBytes {
                            encoding: enc,
                            permille,
                        },
                    };
                }
            }
            decoder = Some(Decoder::new(enc, bom_len > 0));
            checker = Some(RoundtripChecker::new(enc));
            plan = Some(Plan {
                enc,
                bom_len,
                confidence,
                binary_confirmed,
                guess_unsupported: unsupported_guess,
            });
        }
        // 読む量の上限
        let room = limit.saturating_sub(read_total) as usize;
        if chunk.len() > room {
            chunk = &chunk[..room];
            truncated = true;
        }
        read_total += chunk.len() as u64;
        let (Some(dec), Some(chk), Some(pl)) = (decoder.as_mut(), checker.as_mut(), plan.as_ref())
        else {
            break;
        };
        // 再判定 (03 §8.2.1): 信頼度「低」の UTF-8 で、後半に別の文字コードが現れたら 1 回だけ先頭からやり直す
        if !redetect_checked
            && opts.forced.is_none()
            && redetected.is_none()
            && pl.enc == Encoding::Utf8
            && pl.confidence == Confidence::Low
        {
            let file_end = read_total >= meta.len;
            if let Some(other) = redetect(chunk, file_end, &det_opts) {
                let again = OpenOptions {
                    forced: Some(other),
                    ..opts
                };
                return open_inner(id, path, again, ports, cancel, emit, Some(other));
            }
            // 不正な UTF-8 も ISO-2022-JP の列も無い間は、次のチャンクでも見る。最初に見つけた契機で一度だけ判定する
            if find_iso_switch(chunk).is_some() || std::str::from_utf8(chunk).is_err() {
                redetect_checked = true;
            }
        }
        out.clear();
        dec.decode(chunk, false, &mut out);
        builder.append(&out);
        // 往復の検査は元のバイト列 (BOM を除く) で行う
        let body = if first {
            &chunk[pl.bom_len.min(chunk.len())..]
        } else {
            chunk
        };
        chk.feed(body, false);
        let now = ports.clock.now();
        if now.saturating_sub(last_progress) >= crate::time::Millis(100) {
            last_progress = now;
            emit(JobResult::Progress {
                job: id,
                bytes: read_total,
                total: meta.len,
            });
        }
        if truncated {
            break;
        }
    }
    // 空のファイルは sniff なしで終わる
    let mut dec =
        decoder.unwrap_or_else(|| Decoder::new(opts.forced.unwrap_or(Encoding::Utf8), true));
    let mut chk = checker.unwrap_or_else(|| RoundtripChecker::new(Encoding::Utf8));
    out.clear();
    dec.decode(&[], true, &mut out);
    builder.append(&out);
    chk.feed(&[], true);
    let (enc, binary_confirmed, guess_unsupported) = plan
        .as_ref()
        .map_or((opts.forced.unwrap_or(Encoding::Utf8), false, None), |p| {
            (p.enc, p.binary_confirmed, p.guess_unsupported)
        });
    let text = builder.finish();

    // 読込前後の metadata を比較する (05 §4.5)
    let changed_during_load = truncated
        || ports.fs.metadata(path).map_or(true, |after| {
            let a = FileStamp::from_meta(&after, None);
            a.mtime != stamp_before.mtime || a.len != stamp_before.len
        });

    // 形式: 文字コード・BOM・不正バイト・改行の多数派 (同数・改行なしは `file.default_line_ending`)
    let eol_stats = EolStats::count(&text);
    let format = TextFormat {
        encoding: enc,
        has_bom: dec.has_bom(),
        default_eol: eol_stats.majority(opts.default_eol.unwrap_or(LineEnding::Lf)),
        eol_stats,
        invalid_bytes: dec.invalid_bytes(),
        pua_escape: dec.pua_mode(),
        roundtrip: chk.finish(),
        long_line_threshold: opts
            .long_line_threshold
            .unwrap_or(crate::document::LONG_LINE_THRESHOLD),
    };
    let mut detected = opts.pending;
    // 閾値 (既定 1 MiB) を超える行があれば読み取り専用 (理由 LongLine) で開く。その行は先頭 64 KiB だけを表示する (NFR-106、03 §11.5)
    if crate::document::has_long_line(&text, format.long_line_threshold) {
        detected.insert(ReadOnlyReason::LongLine);
    }
    if meta.readonly {
        detected.insert(ReadOnlyReason::FileAttribute);
    }
    if opts.read_only_requested || opts.user_read_only {
        detected.insert(ReadOnlyReason::UserRequested);
    }
    if binary_confirmed {
        detected.insert(ReadOnlyReason::Binary);
    }
    // 不正バイトがあるとき、または元から真の私用文字と衝突しているとき (03 §8.3) は読み取り専用。
    // 状態付き符号化 (ISO-2022-JP) は不正バイトを元のバイトに戻して書けないので、理由を分ける
    if enc.is_stateful() && format.invalid_bytes > 0 {
        detected.insert(ReadOnlyReason::StatefulEncodingWithInvalidBytes);
    } else if format.invalid_bytes > 0 || dec.collision_with_invalid() {
        detected.insert(ReadOnlyReason::InvalidBytes);
    }
    // 衝突があっても不正バイトが 0 ならエスケープを使わないので編集できる
    // 読込前の stamp を保存の照合に使う。読込中に変わったファイルは、そのまま上書き保存すると照合で Conflict になる
    // 最後まで読み切れたときだけハッシュを付ける (途中で大きさが変わったら付けない)
    let stamp = FileStamp {
        hash: hasher
            .filter(|_| !truncated && read_total == meta.len)
            .map(|h| h.finish()),
        ..stamp_before
    };
    JobResult::Complete {
        job: id,
        text,
        format,
        meta,
        stamp,
        changed_during_load,
        detected,
        notes: OpenNotes {
            redetected,
            guess_unsupported,
        },
        interrupted_save: map_interrupted(ports.fs.as_ref(), &inflight, true),
    }
}

/// 保存のためのチャンク供給: Snapshot を `encoding` に符号化して `write_atomic` に渡す。
struct SnapshotSource<'a> {
    chunks: eve_text::Chunks<'a>,
    enc: Encoder,
    buf: Vec<u8>,
    report: EncodeReport,
    cancel: &'a CancelToken,
    started: bool,
    finished: bool,
    /// 書いた内容のハッシュ (16 MiB 以下だけ。超えたら `None` にして計算をやめる、05 §5.5)
    hasher: Option<crate::content_hash::ContentHasher>,
    written: u64,
}

impl SnapshotSource<'_> {
    /// 書き終えた内容のハッシュ (上限を超えたら `None`)。
    fn content_hash(&self) -> Option<crate::fs::ContentHash> {
        self.hasher
            .as_ref()
            .map(crate::content_hash::ContentHasher::finish)
    }

    /// 返すチャンクを数え、ハッシュに入れる。
    fn track(&mut self) {
        self.written += self.buf.len() as u64;
        if self.written > crate::content_hash::HASH_MAX_LEN {
            self.hasher = None;
        } else if let Some(h) = self.hasher.as_mut() {
            h.update(&self.buf);
        }
    }
}

impl ReadChunks for SnapshotSource<'_> {
    fn next_chunk(&mut self) -> Result<Option<&[u8]>, ChunkError> {
        // 内側が返すのは常に `self.buf` なので、借用を終えてから数えて、あらためて返す
        match self.next_chunk_inner().map(|c| c.is_some()) {
            Ok(true) => {
                self.track();
                Ok(Some(&self.buf))
            }
            Ok(false) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

impl SnapshotSource<'_> {
    fn next_chunk_inner(&mut self) -> Result<Option<&[u8]>, ChunkError> {
        if self.cancel.is_cancelled() {
            return Err(ChunkError::Aborted);
        }
        loop {
            self.buf.clear();
            match self.chunks.next() {
                Some(s) => {
                    self.enc.encode(s, &mut self.buf, &mut self.report);
                    self.started = true;
                    if !self.report.is_empty() {
                        // 表現不能の文字: 何も書かずに止める (乾式走査で見つけているので、ここへは来ないはず)
                        return Err(ChunkError::Aborted);
                    }
                    // ISO-2022-JP は行末までためるので、出力が空のチャンクがある
                    if self.buf.is_empty() {
                        continue;
                    }
                    return Ok(Some(&self.buf));
                }
                None if !self.started => {
                    // 空の文書: BOM だけを書く (なければ何も書かない)
                    self.started = true;
                    self.enc.encode("", &mut self.buf, &mut self.report);
                    return Ok((!self.buf.is_empty()).then_some(&self.buf[..]));
                }
                None if !self.finished => {
                    self.finished = true;
                    self.enc.finish(&mut self.buf, &mut self.report);
                    if !self.report.is_empty() {
                        return Err(ChunkError::Aborted);
                    }
                    return Ok((!self.buf.is_empty()).then_some(&self.buf[..]));
                }
                None => return Ok(None),
            }
        }
    }
}

/// 保存ジョブの引数。
pub(crate) struct SaveParams<'a> {
    pub path: &'a FsPath,
    pub snapshot: &'a Snapshot,
    pub encoding: Encoding,
    pub bom: bool,
    pub pua: PuaEscapeMode,
    pub expected: Expect,
    pub strategy: crate::jobs::SaveStrategy,
    pub clear_readonly: bool,
}

/// 乾式走査: 出力は捨てて、表現不能の文字と出力のバイト長だけを集める (03 §8.6 E1・05 §4.4 手順 0。書く前に止めるため)。
fn dry_run(p: &SaveParams<'_>, cancel: &CancelToken) -> Option<(EncodeReport, u64)> {
    let mut enc = Encoder::new(p.encoding, p.bom, EscapePolicy::from_mode(p.pua));
    let mut report = EncodeReport::default();
    let mut sink = Vec::new();
    let mut out_len = 0u64;
    for s in p.snapshot.chunks(ByteRange::all(p.snapshot.len_bytes())) {
        if cancel.is_cancelled() {
            return None;
        }
        sink.clear();
        enc.encode(s, &mut sink, &mut report);
        out_len += sink.len() as u64;
    }
    sink.clear();
    enc.finish(&mut sink, &mut report);
    out_len += sink.len() as u64;
    Some((report, out_len))
}

pub(crate) fn run_save(
    id: JobId,
    p: SaveParams<'_>,
    ports: &JobPorts,
    cancel: &CancelToken,
) -> JobResult {
    use crate::jobs::SaveStrategy;
    // 直接上書きは出力の長さが要るので常に乾式走査をする。それ以外は、UTF-8 と UTF-16 が全 Unicode スカラー値を表せるので、
    // 表現不能になり得るのはエスケープ文字だけ (E4)。Shift_JIS・EUC-JP・ISO-2022-JP は書く前に走査する
    let in_place = matches!(p.strategy, SaveStrategy::InPlace { .. });
    let mut out_len = 0u64;
    if in_place
        || !(p.encoding == Encoding::Utf8 || p.encoding.is_utf16())
        || EscapePolicy::from_mode(p.pua) == EscapePolicy::Reject
    {
        match dry_run(&p, cancel) {
            None => {
                return JobResult::Failed {
                    job: id,
                    err: JobError::Cancelled,
                };
            }
            Some((report, _)) if !report.is_empty() => {
                return JobResult::Unmappable {
                    job: id,
                    total: report.total,
                    recorded: report.recorded,
                    encoding: p.encoding,
                };
            }
            Some((_, n)) => out_len = n,
        }
    }
    let SaveParams {
        path,
        snapshot,
        encoding,
        bom,
        pua,
        expected,
        strategy,
        clear_readonly,
    } = p;
    let mut src = SnapshotSource {
        chunks: snapshot.chunks(ByteRange::all(snapshot.len_bytes())),
        enc: Encoder::new(encoding, bom, EscapePolicy::from_mode(pua)),
        buf: Vec::new(),
        report: EncodeReport::default(),
        cancel,
        started: false,
        finished: false,
        hasher: Some(crate::content_hash::ContentHasher::new()),
        written: 0,
    };
    // 書き始める前の照合 (内容ハッシュ。05 §5.5): mtime が同じで中身が違えば Conflict、中身が同じで mtime だけが違えば新しい Stamp で続ける
    let expected = match expected {
        Expect::Stamp(want) => {
            match crate::content_hash::reconcile(ports.fs.as_ref(), path, want) {
                Ok(s) => Expect::Stamp(s),
                Err(e) => {
                    return JobResult::Failed {
                        job: id,
                        err: JobError::Fs(e),
                    };
                }
            }
        }
        other => other,
    };
    let mut opts = SaveOptions::user_file(expected);
    opts.clear_readonly = clear_readonly;
    let result = match strategy {
        SaveStrategy::Atomic { hardlink } => {
            opts.hardlink = hardlink;
            ports.fs.write_atomic(path, &mut src, &opts)
        }
        SaveStrategy::InPlace { backup } => {
            opts.in_place_backup = backup;
            ports
                .fs
                .write_in_place(path, &mut src, out_len, &opts, cancel)
        }
    };
    match result {
        Ok(mut report) => {
            // 書いた内容のハッシュを新しい Stamp に入れる (次の照合で、開いてから最初の保存までの外部変更も見逃さない)
            if report.new_stamp.len == src.written {
                report.new_stamp.hash = src.content_hash();
            }
            JobResult::Saved { job: id, report }
        }
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
