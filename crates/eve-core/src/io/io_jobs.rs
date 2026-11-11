//! 読込・保存ジョブの本体 (docs/03 §9.2 / §9.3)。`run_job` から呼ばれ、ワーカー (またはテストの `ManualJobSpawner`) で動く。
//!
//! 読込: 先頭 64 KiB で文字コードを判定し (03 §8.2)、チャンクごとに復号して `TextBuilder` に積む。不正バイトは PUA にエスケープして
//! 保持し (§8.3)、読込時に往復を検査し (§8.7)、信頼度が低いとき後半で別の文字コードが現れたら判定し直して 1 回だけやり直す (§8.2.1)。
//! 保存は `FileSystem::write_atomic` (原子的保存) だけで、
//! 非 UTF-8 では出力の前に乾式走査で表現不能の文字を探し、あれば何も書かずに `JobResult::Unmappable` を返す (§8.6 E1)。

use eve_encoding::{
    Confidence, Decoder, DetectOptions, DetectedKind, Detection, EncodeReport, Encoder, Encoding,
    EscapePolicy, NoPreserveReason, PreserveState, PreserveStatus, PreservedRecords,
    RoundtripChecker, detect, find_iso_switch, redetect_window,
};
use eve_text::{ByteRange, LineEnding, Snapshot, TextBuilder, TextRead};
use eve_view::JobId;

use crate::document::{
    LineStatsScanner, PreservedMap, ReadOnlyReason, ReadOnlyReasons, TextFormat,
};
use crate::io::fs::{
    ChunkError, Expect, FileKind, FileStamp, FsError, FsErrorDetail, FsErrorKind, ReadChunks,
    SaveOptions,
};
use crate::io::fspath::FsPath;
use crate::io::interrupted::map_interrupted;
use crate::io::save_plan::{Decision, Planner, Unused};
use crate::jobs::cancel::CancelToken;
use crate::jobs::{ConfirmAsk, JobError, JobOutput, JobPorts, JobResult, OpenNotes};

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

/// 利用者のデータ量で決まる確保の見込みを `try_reserve` で確かめる (INV-IO4、02 §11.2.2、03 §9.2)。
/// 確保できたらすぐ解放する。rope の構築 (ropey) は確保の失敗を返せないので、読み始める前の検査で
/// OOM で abort する経路を減らす。失敗は `JobError::OutOfMemory` (プロセスは続ける)。
fn probe_reserve(len: u64) -> Result<(), JobError> {
    let oom = JobError::OutOfMemory { requested: len };
    let Ok(n) = usize::try_from(len) else {
        return Err(oom);
    };
    let mut v: Vec<u8> = Vec::new();
    if v.try_reserve_exact(n).is_err() {
        return Err(oom);
    }
    // 使わない確保を最適化で消させない (消えると失敗を検出できない)
    std::hint::black_box(v.as_ptr());
    Ok(())
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
    emit: &mut dyn FnMut(JobOutput),
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
    emit: &mut dyn FnMut(JobOutput),
    redetected: Option<Encoding>,
) -> JobResult {
    let fail = |err| JobResult::Failed { err };
    let cancelled = || JobResult::Failed {
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
            ask: ConfirmAsk::HugeFile { len: meta.len },
        };
    }
    let stamp_before = FileStamp::from_meta(&meta, None);
    // 本文の大きさ分を確保できるかを先に確かめる (rope の構築は確保の失敗を返せない。02 §11.2.2、03 §9.2)
    if let Err(e) = probe_reserve(meta.len) {
        return fail(e);
    }

    // 3. 読み始める
    let mut reader = match ports.fs.open_read(path) {
        Ok(r) => r,
        Err(e) => return fail(JobError::Fs(e)),
    };
    let limit = meta.len.saturating_add(read_slack(meta.len));
    let mut read_total: u64 = 0;
    let mut builder = TextBuilder::new();
    // 改行の統計と長大行の数は、復号した文字列を木に足すときに一緒に数える (読込の後に全文を走査し直さない。PRF-17)
    let long_line_threshold = opts
        .long_line_threshold
        .unwrap_or(crate::document::LONG_LINE_THRESHOLD);
    let mut line_stats = LineStatsScanner::new(long_line_threshold);
    let det_opts = DetectOptions::default().with_forced(opts.forced);
    let mut plan: Option<Plan> = None;
    let mut decoder: Option<Decoder> = None;
    let mut checker: Option<RoundtripChecker> = None;
    let mut redetect_checked = false;
    let mut out = String::new();
    let mut truncated = false;
    // 進捗: 読み始めてすぐ (開く操作から 50 ms 以内) に 1 件、以後は 100 ms 以上あけて (10 Hz 以下、03 §9.2)
    emit(JobOutput {
        job: id,
        body: JobResult::Progress {
            bytes: 0,
            total: meta.len,
        },
    });
    let mut last_progress = ports.clock.now();
    // 内容ハッシュ (16 MiB 以下。読み込みながら計算する、05 §5.5)
    let mut hasher = (meta.len <= crate::io::content_hash::HASH_MAX_LEN)
        .then(crate::io::content_hash::ContentHasher::new);
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
            let file_complete = usize::try_from(meta.len).is_ok_and(|n| n <= head.len());
            let Detection {
                kind,
                confidence,
                unsupported_guess,
            } = detect(head, file_complete, &det_opts);
            let mut binary_confirmed = false;
            match kind {
                DetectedKind::Binary if !opts.read_only_requested => {
                    return JobResult::NeedsConfirm {
                        ask: ConfirmAsk::Binary,
                    };
                }
                DetectedKind::UnsupportedBom(kind) if !opts.read_only_requested => {
                    return JobResult::NeedsConfirm {
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
                // 不正バイトは head の中の数なので 1000 以下
                let permille = u32::try_from(
                    u64::from(probe.invalid_bytes()) * 1000 / head.len().max(1) as u64,
                )
                .unwrap_or(u32::MAX);
                if permille > MANY_INVALID_PERMILLE {
                    return JobResult::NeedsConfirm {
                        ask: ConfirmAsk::ManyInvalidBytes {
                            encoding: enc,
                            permille,
                        },
                    };
                }
            }
            decoder = Some(Decoder::new(enc, bom_len > 0));
            // 保持方式の記録も集める (03 §8.7.1。区分 A の文字コードでは集めない)。BOM は検査器に渡さない (下の `body`)
            checker = Some(RoundtripChecker::with_records(enc));
            plan = Some(Plan {
                enc,
                bom_len,
                confidence,
                binary_confirmed,
                guess_unsupported: unsupported_guess,
            });
        }
        // 読む量の上限
        // usize に収まらない残りは、上限が無いのと同じ
        let room = usize::try_from(limit.saturating_sub(read_total)).unwrap_or(usize::MAX);
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
        // 復号のバッファ: 1 バイトが最大 3 バイト (不正バイトの PUA・2 バイトの文字コード) になる
        let need = chunk.len().saturating_mul(3).saturating_add(16);
        if out.try_reserve(need).is_err() {
            return fail(JobError::OutOfMemory {
                requested: need as u64,
            });
        }
        dec.decode(chunk, false, &mut out);
        builder.append(&out);
        line_stats.feed(&out);
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
            emit(JobOutput {
                job: id,
                body: JobResult::Progress {
                    bytes: read_total,
                    total: meta.len,
                },
            });
        }
        if truncated {
            break;
        }
    }
    // 空のファイルは sniff なしで終わる
    let mut dec =
        decoder.unwrap_or_else(|| Decoder::new(opts.forced.unwrap_or(Encoding::Utf8), true));
    let mut chk = checker
        .unwrap_or_else(|| RoundtripChecker::with_records(opts.forced.unwrap_or(Encoding::Utf8)));
    out.clear();
    dec.decode(&[], true, &mut out);
    builder.append(&out);
    line_stats.feed(&out);
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
    // 閾値 (既定 1 MiB) を超える行は char 近似モードで扱う (編集できる。NFR-106、03 §11.5.1)
    let (eol_stats, long_lines) = line_stats.finish();
    // 保持方式の記録を文書の表にし、全部の記録を本文と照合する (03 §8.7.1 の「読込の完了時の照合」。記録の総数に
    // 比例するので、メインスレッドではなくここで行う)
    let (mut roundtrip, records) = chk.finish_with_records();
    let (preserved, status) = preserved_from_records(&records, enc, dec.invalid_bytes(), &text);
    drop(records);
    roundtrip.preserve = status;
    let format = TextFormat {
        encoding: enc,
        has_bom: dec.has_bom(),
        default_eol: eol_stats.majority(opts.default_eol.unwrap_or(LineEnding::Lf)),
        eol_stats,
        invalid_bytes: dec.invalid_bytes(),
        pua_escape: dec.pua_mode(),
        roundtrip,
        long_line_threshold,
        long_lines,
    };
    let mut detected = opts.pending;
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
        text,
        format,
        preserved,
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

/// 読込の検査が集めた記録 (`records`) を文書の表にし、保持方式の状態を決める (03 §8.7.1 の「集める側と持つ側」
/// 「読込の完了時の照合」「『保持なし』と『保持できなかった箇所がある』」)。`text` は記録を集めたのと同じバイト列を
/// 復号した本文。照合に合わない記録が 1 件でもあれば、記録を全部捨てて「保持なし」にする。
///
/// 照合の失敗は実装の誤りなので debug ビルドでは止める。eve-core にはまだログの出口が無い (02 §9.2 の `LogSink` は
/// Host の部品) ので、製品では状態 (`NoPreserveReason::VerifyFailed`) だけを残す (本文を含めない)。
fn preserved_from_records(
    records: &PreservedRecords,
    enc: Encoding,
    invalid_bytes: u32,
    text: &dyn TextRead,
) -> (PreservedMap, PreserveStatus) {
    let none = |reason| (PreservedMap::new(), PreserveStatus::Unavailable(reason));
    match records.state {
        PreserveState::Collected => match PreservedMap::from_records(records, enc, text) {
            Ok(map) => (
                map,
                PreserveStatus::Active {
                    unpreserved: records.unpreserved,
                },
            ),
            Err(e) => {
                debug_assert!(false, "読込の完了時の照合に失敗した: {e}");
                none(NoPreserveReason::VerifyFailed)
            }
        },
        PreserveState::Overflow => none(NoPreserveReason::Overflow),
        PreserveState::Incomplete => none(NoPreserveReason::Incomplete),
        // 不正バイトを含む ISO-2022-JP の文書 (検査器が記録を捨てる。ISO-2022-JP では保存しない、03 §8.3)
        PreserveState::Disabled if enc.is_stateful() && invalid_bytes > 0 => {
            none(NoPreserveReason::InvalidBytes)
        }
        // 区分 A の文字コード (UTF-8 / UTF-16。バイナリの確認から UTF-8 で読んだものを含む)
        PreserveState::Disabled => (PreservedMap::new(), PreserveStatus::NotApplicable),
    }
}

/// 保存の計画があるときの、1 段の出力の目安の大きさ (バイト)。
const STEP_TARGET: usize = 64 * 1024;

/// 保存の出力の確認 (ii) (03 §8.6 E2): 出力の全体を同じ文字コードの 1 本の復号器で通しで復号し、Snapshot の本文と
/// 前から順に突き合わせる (元のバイト列と符号化した部分のつなぎ目を含む。不正バイトは §8.3 のとおり PUA に戻る)。
/// 作業のメモリは、1 段の出力を復号した分だけ。
struct Verify {
    dec: Decoder,
    buf: String,
    /// 突き合わせ終えた本文の位置。
    at: usize,
    ok: bool,
}

impl Verify {
    fn new(encoding: Encoding) -> Verify {
        Verify {
            dec: Decoder::new(encoding, false),
            buf: String::new(),
            at: 0,
            ok: true,
        }
    }

    /// 出力 `bytes` を復号して、本文の続きと比べる。`last` は出力の終わり (復号器の持ち越しを出し切る)。
    fn feed(&mut self, text: &Snapshot, bytes: &[u8], last: bool) {
        if !self.ok {
            return;
        }
        self.buf.clear();
        self.dec.decode(bytes, last, &mut self.buf);
        if self.buf.is_empty() {
            return;
        }
        let end = self.at + self.buf.len();
        if end > text.len_bytes() || !text.is_char_boundary(eve_text::ByteOff(end)) {
            self.ok = false;
            return;
        }
        let mut off = 0usize;
        for c in text.chunks(ByteRange::new(
            eve_text::ByteOff(self.at),
            eve_text::ByteOff(end),
        )) {
            let got = self.buf.as_bytes().get(off..off + c.len());
            if got != Some(c.as_bytes()) {
                self.ok = false;
                return;
            }
            off += c.len();
        }
        self.ok = off == self.buf.len();
        self.at = end;
    }

    /// 出力の終わり: 本文の全体まで突き合わせたことを確かめる (足りなければ不一致)。
    fn finish(&mut self, text: &Snapshot) {
        self.ok &= self.at == text.len_bytes();
    }
}

/// 保存の符号化の流れ: Snapshot を `encoding` に符号化した出力を、1 段ずつ取り出す。
///
/// 乾式走査 ([`dry_run`]) と本番の書き込み ([`SnapshotSource`]) が同じものを使うので、2 つの出力は長さも内容も一致する
/// (03 §9.3 の「乾式走査と出力長」。直接上書きは、乾式走査で求めた長さのとおりに書く)。
///
/// 保持方式の記録の版 (`preserved`) を渡すと (v0.4、03 §8.6 E2)、保存の計画 (`save_plan::Planner`) に従って、記録の境界で
/// 本文を区間に切り、区間はチャンクごとに `Encoder::encode`、元のバイト列を書く記録は `Encoder::write_preserved`、使わない
/// 記録は範囲の文字列をまとめて `encode` に渡す。計画は記録と Snapshot だけで決まるので、出力はチャンクの区切りに依らない
/// (記録が rope のチャンクの境をまたいでも、範囲の文字列を 1 つにしてから扱う)。表が空なら v0.3 と同じ出力。
struct SaveStream<'a> {
    snapshot: &'a Snapshot,
    /// 今の区間 (`pos` から次の記録の始まり、記録が尽きたら文書の末尾まで) のチャンク。
    chunks: eve_text::Chunks<'a>,
    enc: Encoder,
    report: EncodeReport,
    started: bool,
    finished: bool,
    /// 保存の計画 (表が空なら `None`)。
    plan: Option<Planner<'a>>,
    /// 今の区間の終わりで扱う記録。
    pending: Option<Decision<'a>>,
    /// 計画を決め終えた位置 (直前の記録の終わり)。
    pos: usize,
    /// 使わなかった行の記録の数 (ISO-2022-JP の、行の内容の全体でなくなった記録。編集した行と同じく正規形で書く)。
    line_unused: usize,
    /// 確認 (i) に合わない・表の不変条件に反する記録の数 (起きないはず)。1 以上なら書く前に止める (03 §8.6 E2)。
    broken: usize,
    /// 確認 (ii)。表を渡したときだけ行う。
    verify: Option<Verify>,
}

impl<'a> SaveStream<'a> {
    fn new(
        snapshot: &'a Snapshot,
        encoding: Encoding,
        bom: bool,
        escape: EscapePolicy,
        preserved: &'a PreservedMap,
    ) -> SaveStream<'a> {
        let with_plan = !preserved.is_empty();
        let mut s = SaveStream {
            snapshot,
            chunks: snapshot.chunks(ByteRange::all(snapshot.len_bytes())),
            enc: Encoder::new(encoding, bom, escape),
            report: EncodeReport::default(),
            started: false,
            finished: false,
            plan: with_plan.then(|| Planner::new(snapshot, encoding, preserved)),
            pending: None,
            pos: 0,
            line_unused: 0,
            broken: 0,
            verify: with_plan.then(|| Verify::new(encoding)),
        };
        if with_plan {
            s.next_segment();
        }
        s
    }

    /// `pos` から次の記録までの区間を用意する (表の不変条件に反する記録は、区切りにせずに数える)。
    fn next_segment(&mut self) {
        let len = self.snapshot.len_bytes();
        let Some(plan) = self.plan.as_mut() else {
            return;
        };
        loop {
            match plan.next(self.pos) {
                Some(Decision::Skip { .. }) => self.broken += 1,
                Some(d) => {
                    let end = d.start();
                    self.chunks = self.snapshot.chunks(ByteRange::new(
                        eve_text::ByteOff(self.pos),
                        eve_text::ByteOff(end),
                    ));
                    self.pending = Some(d);
                    return;
                }
                None => {
                    self.chunks = self.snapshot.chunks(ByteRange::new(
                        eve_text::ByteOff(self.pos),
                        eve_text::ByteOff(len),
                    ));
                    self.pending = None;
                    return;
                }
            }
        }
    }

    /// 区間の終わりに来た記録を扱う。
    fn apply(&mut self, d: Decision<'a>, out: &mut Vec<u8>) {
        match d {
            Decision::Raw { span, text } => {
                if !self.enc.write_preserved(&text, span.bytes, out) {
                    // 計画は書けると判断した (起きないはず): 記録を使わずに符号化し、書く前に止める
                    self.enc.encode(&text, out, &mut self.report);
                    self.broken += 1;
                }
                self.pos = span.range.end.0;
            }
            Decision::Encode { span, text, why } => {
                self.enc.encode(&text, out, &mut self.report);
                match why {
                    Unused::Line => self.line_unused += 1,
                    Unused::Mismatch => self.broken += 1,
                }
                self.pos = span.range.end.0;
            }
            // `next_segment` が数えて読み飛ばすので、ここへは来ない
            Decision::Skip { .. } => self.broken += 1,
        }
        self.next_segment();
    }

    /// 次の 1 段の出力を `out` に書く (`out` は先に空にする)。段が尽きたら `false`。
    /// 出力が空の段もある (ISO-2022-JP は行末までためる)。表現不能の文字は `report` にたまる。
    fn step(&mut self, out: &mut Vec<u8>) -> bool {
        out.clear();
        let mut last = false;
        let mut any = false;
        loop {
            if let Some(s) = self.chunks.next() {
                self.enc.encode(s, out, &mut self.report);
                self.started = true;
            } else if let Some(d) = self.pending.take() {
                self.apply(d, out);
                self.started = true;
            } else if !self.started {
                // 空の文書: BOM だけを書く (なければ何も書かない)
                self.started = true;
                self.enc.encode("", out, &mut self.report);
            } else if !self.finished {
                self.finished = true;
                self.enc.finish(out, &mut self.report);
                last = true;
            } else {
                break;
            }
            any = true;
            // 記録が細かく並ぶ文書で、1 段の出力が小さくなりすぎないよう、計画があるときは目安の大きさまでまとめる
            // (出力のバイト列は段の分け方に依らない)。計画が無いときは v0.3 と同じくチャンクごと
            if last || self.plan.is_none() || out.len() >= STEP_TARGET {
                break;
            }
        }
        if !any {
            return false;
        }
        if let Some(v) = self.verify.as_mut() {
            v.feed(self.snapshot, out, last);
        }
        true
    }

    /// 出力を終えた (`step` が偽を返した) 後に呼ぶ: 確認 (ii) で、本文の全体まで突き合わせたことを確かめる。
    fn finish_verify(&mut self) {
        if let Some(v) = self.verify.as_mut() {
            v.finish(self.snapshot);
        }
    }

    /// 保持方式の記録を使う保存の確かめに合わない (03 §8.6 E2): 確認 (i) に合わない・表の不変条件に反する記録があった、
    /// または確認 (ii) で不一致を見つけた (表を渡さなかったときは常に偽)。出力の途中でも、それまでに見つけたものを返す。
    /// どれも書く前に止める (`JobError::VerifyFailed`)。
    fn verify_failed(&self) -> bool {
        self.broken > 0 || self.verify.as_ref().is_some_and(|v| !v.ok)
    }
}

/// 保存のためのチャンク供給: [`SaveStream`] の出力を `write_atomic` / `write_in_place` に渡す。
struct SnapshotSource<'a> {
    stream: SaveStream<'a>,
    buf: Vec<u8>,
    cancel: &'a CancelToken,
    /// 書いた内容のハッシュ (16 MiB 以下だけ。超えたら `None` にして計算をやめる、05 §5.5)
    hasher: Option<crate::io::content_hash::ContentHasher>,
    written: u64,
}

impl SnapshotSource<'_> {
    /// 書き終えた内容のハッシュ (上限を超えたら `None`)。
    fn content_hash(&self) -> Option<crate::io::fs::ContentHash> {
        self.hasher
            .as_ref()
            .map(crate::io::content_hash::ContentHasher::finish)
    }

    /// 返すチャンクを数え、ハッシュに入れる。
    fn track(&mut self) {
        self.written += self.buf.len() as u64;
        if self.written > crate::io::content_hash::HASH_MAX_LEN {
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
        while self.stream.step(&mut self.buf) {
            // 表現不能の文字・確認 (ii) の不一致: 何も書かずに止める (乾式走査で見つけているので、ここへは来ないはず。
            // 原子的保存では Port が一時ファイルを捨てる)
            if !self.stream.report.is_empty() || self.stream.verify_failed() {
                return Err(ChunkError::Aborted);
            }
            // 出力が空の段は飛ばす (ISO-2022-JP は行末までためる。空の文書で BOM が無いとき)
            if !self.buf.is_empty() {
                return Ok(Some(&self.buf));
            }
        }
        // 出力の全体が本文に一致したことを確かめてから終わりを返す (確認 (ii)。一致しなければ置換しない)
        self.stream.finish_verify();
        if self.stream.verify_failed() {
            return Err(ChunkError::Aborted);
        }
        Ok(None)
    }
}

/// 保存先 T に手を付けた後の失敗か (元のファイルが退避名に移った・直接上書きの途中で失敗した)。
fn wrote_target(e: &crate::FsError) -> bool {
    matches!(
        e.kind,
        crate::FsErrorKind::ReplacedMovedToBackup { .. } | crate::FsErrorKind::InPlaceFailed { .. }
    )
}

/// 保存ジョブの引数。
pub(crate) struct SaveParams<'a> {
    pub path: &'a FsPath,
    pub snapshot: &'a Snapshot,
    pub encoding: Encoding,
    pub bom: bool,
    pub escape: EscapePolicy,
    pub expected: Expect,
    pub strategy: crate::jobs::SaveStrategy,
    pub clear_readonly: bool,
    pub create_link_target: bool,
    /// 保持方式の記録の版 (保存を始めた時点のもの、03 §8.6 E2)。空なら記録を使わない (v0.3 と同じ出力)。
    pub preserved: &'a PreservedMap,
}

/// 乾式走査の結果。
struct DryScan {
    report: EncodeReport,
    out_len: u64,
    /// 確認 (i)・(ii) と表の不変条件に合った (表を渡さなかったときは常に真)。
    verified: bool,
    /// 確認 (i) に合わない・表の不変条件に反する記録の数。
    broken: usize,
}

/// 乾式走査: 出力は捨てて、表現不能の文字と出力のバイト長だけを集める (03 §8.6 E1・05 §4.4 手順 0。書く前に止めるため)。
/// 保持方式の記録を渡したときは、確認 (i)・(ii) もここで行う (03 §8.6 E2。合わなければ Port を呼ばずに止める)。
fn dry_run(p: &SaveParams<'_>, cancel: &CancelToken) -> Option<DryScan> {
    let mut stream = SaveStream::new(p.snapshot, p.encoding, p.bom, p.escape, p.preserved);
    let mut sink = Vec::new();
    let mut out_len = 0u64;
    loop {
        if cancel.is_cancelled() {
            return None;
        }
        if !stream.step(&mut sink) {
            break;
        }
        out_len += sink.len() as u64;
    }
    stream.finish_verify();
    Some(DryScan {
        verified: !stream.verify_failed(),
        broken: stream.broken,
        report: stream.report,
        out_len,
    })
}

pub(crate) fn run_save(p: SaveParams<'_>, ports: &JobPorts, cancel: &CancelToken) -> JobResult {
    use crate::jobs::SaveStrategy;
    // 直接上書きは出力の長さが要るので常に乾式走査をする。それ以外は、UTF-8 と UTF-16 が全 Unicode スカラー値を表せるので、
    // 表現不能になり得るのはエスケープ文字だけ (E4)。Shift_JIS・EUC-JP・ISO-2022-JP は書く前に走査する。保持方式の記録を
    // 渡したときも走査する (確認 (ii) を書く前に行う。記録を渡すのは Shift_JIS・EUC-JP・ISO-2022-JP だけ)
    let in_place = matches!(p.strategy, SaveStrategy::InPlace { .. });
    let mut out_len = 0u64;
    if in_place
        || !(p.encoding == Encoding::Utf8 || p.encoding.is_utf16())
        || p.escape == EscapePolicy::Reject
        || !p.preserved.is_empty()
    {
        match dry_run(&p, cancel) {
            None => {
                return JobResult::Failed {
                    err: JobError::Cancelled,
                };
            }
            // 確認 (i) に合わない・表の不変条件に反する記録がある (実装の誤り)。どのビルドでも何も書かずに止める。
            // 表現不能より先に見る (合わない記録をふつうに符号化したことで表現不能になった文字を、置換の対象として
            // 示さない)
            Some(d) if d.broken > 0 => {
                return JobResult::Failed {
                    err: JobError::VerifyFailed,
                };
            }
            Some(d) if !d.report.is_empty() => {
                return JobResult::Unmappable {
                    total: d.report.total,
                    recorded: d.report.recorded,
                    encoding: p.encoding,
                };
            }
            // 確認 (ii): 出力を復号しても本文にならない (実装の誤り)。何も書かずに止める (表現不能の文字は出力から抜けて
            // 本文と合わないので、表現不能の後に見る)
            Some(d) if !d.verified => {
                return JobResult::Failed {
                    err: JobError::VerifyFailed,
                };
            }
            Some(d) => out_len = d.out_len,
        }
    }
    let SaveParams {
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
    } = p;
    let mut src = SnapshotSource {
        stream: SaveStream::new(snapshot, encoding, bom, escape, preserved),
        buf: Vec::new(),
        cancel,
        hasher: Some(crate::io::content_hash::ContentHasher::new()),
        written: 0,
    };
    // 書き始める前の照合 (内容ハッシュ。05 §5.5): mtime が同じで中身が違えば Conflict、中身が同じで mtime だけが違えば新しい Stamp で続ける
    let expected = match expected {
        Expect::Stamp(want) => {
            match crate::io::content_hash::reconcile(ports.fs.as_ref(), path, want) {
                Ok(s) => Expect::Stamp(s),
                Err(e) => {
                    return JobResult::Failed {
                        err: JobError::Fs(e),
                    };
                }
            }
        }
        other => other,
    };
    let mut opts = SaveOptions::user_file(expected);
    opts.clear_readonly = clear_readonly;
    opts.create_link_target = create_link_target;
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
            JobResult::Saved { report }
        }
        // 取消しを優先するのは、何も書いていない失敗だけ。置換を始めた後の失敗 (元のファイルが退避名に移った・
        // 直接上書きの途中で壊れた) は、取消しを送った後でも必ずそのまま返し、退避先を利用者に示す
        // (05 §4.1 手順 5、NFR-301 AC-2。docs/review/v0.3-findings.md の SAF-04)
        Err(e) if cancel.is_cancelled() && !wrote_target(&e) => JobResult::Failed {
            err: JobError::Cancelled,
        },
        // 本番の書き込みで確認 (i)・(ii) に合わなかった (乾式走査で確かめているので起きないはず): 置換の前に止めた
        Err(e) if src.stream.verify_failed() && !wrote_target(&e) => JobResult::Failed {
            err: JobError::VerifyFailed,
        },
        Err(e) => JobResult::Failed {
            err: JobError::Fs(e),
        },
    }
}

/// 試験用: 保存の符号化の結果 (`save_for_test`)。
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct SaveTestOut {
    /// 本番の書き込みの供給と同じ手順で作った出力の全部 (表現不能の文字は飛ばしたもの)。
    pub(crate) bytes: Vec<u8>,
    /// 乾式走査の出力の長さ。
    pub(crate) dry_len: u64,
    /// 表現不能の総数。
    pub(crate) unmappable: u64,
    /// 確認 (i)・(ii) と表の不変条件に合った。
    pub(crate) verified: bool,
    /// 確認 (i) に合わなかった・表の不変条件に反した記録の数 (0 で `verified` が偽なら、確認 (ii) の不一致)。
    pub(crate) broken: usize,
    /// 使わなかった行の記録 (ISO-2022-JP の、行の内容の全体でなくなった記録) の数。
    pub(crate) line_unused: usize,
    /// 本番の書き込みの供給 (`SnapshotSource`) が最後まで返した出力 (表現不能・確認 (ii) の不一致で止まったら `None`)。
    pub(crate) source: Option<Vec<u8>>,
}

/// 試験用: 乾式走査と本番の書き込みの供給の両方で、Snapshot と保持方式の記録の版から保存の出力を作る (03 §22.1 の
/// 保持方式の (a)(b) の性質テストと、故障の注入の試験が使う)。
#[cfg(test)]
pub(crate) fn save_for_test(
    snapshot: &Snapshot,
    encoding: Encoding,
    escape: EscapePolicy,
    preserved: &PreservedMap,
) -> SaveTestOut {
    let path = FsPath::new("/t/a.txt");
    let p = SaveParams {
        path: &path,
        snapshot,
        encoding,
        bom: false,
        escape,
        expected: Expect::Any,
        strategy: crate::jobs::SaveStrategy::default(),
        clear_readonly: false,
        create_link_target: false,
        preserved,
    };
    let cancel = CancelToken::new();
    let d = dry_run(&p, &cancel).unwrap_or_else(|| unreachable!("取り消していない"));
    // 表現不能の文字を飛ばした出力の全部 (段をそのままつなぐ)
    let mut stream = SaveStream::new(snapshot, encoding, false, escape, preserved);
    let mut bytes = Vec::new();
    let mut piece = Vec::new();
    while stream.step(&mut piece) {
        bytes.extend_from_slice(&piece);
    }
    let mut src = SnapshotSource {
        stream: SaveStream::new(snapshot, encoding, false, escape, preserved),
        buf: Vec::new(),
        cancel: &cancel,
        hasher: None,
        written: 0,
    };
    let mut got = Vec::new();
    let source = loop {
        match src.next_chunk() {
            Ok(Some(c)) => got.extend_from_slice(c),
            Ok(None) => break Some(got),
            Err(_) => break None,
        }
    };
    SaveTestOut {
        bytes,
        dry_len: d.out_len,
        unmappable: d.report.total,
        verified: d.verified,
        broken: stream.broken,
        line_unused: stream.line_unused,
        source,
    }
}

#[cfg(test)]
mod preserve_save;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::SaveStrategy;

    /// 本番の書き込みの供給 (`SnapshotSource`) が返す内容の全部。
    fn written(text: &str, encoding: Encoding, bom: bool) -> Vec<u8> {
        let t = eve_text::Text::from(text);
        let snap = t.snapshot();
        let cancel = CancelToken::new();
        let empty = PreservedMap::new();
        let mut src = SnapshotSource {
            stream: SaveStream::new(&snap, encoding, bom, EscapePolicy::Restore, &empty),
            buf: Vec::new(),
            cancel: &cancel,
            hasher: None,
            written: 0,
        };
        let mut out = Vec::new();
        while let Some(c) = src.next_chunk().unwrap() {
            out.extend_from_slice(c);
        }
        out
    }

    // 直接上書きは乾式走査で求めた長さのとおりに書くので、2 つの長さは必ず同じ (空の文書の BOM、チャンクをまたぐ文書、
    // ISO-2022-JP の行末でのまとめ書きを含む)
    #[test]
    fn 乾式走査の長さは本番の出力の長さと同じ() {
        let long = "日本語の行 abc\r\n".repeat(400);
        let docs = [
            "",
            "a",
            "abc\n",
            "日本語\nかな\n",
            "末尾に改行なし",
            long.as_str(),
        ];
        for encoding in [
            Encoding::Utf8,
            Encoding::Utf16Le,
            Encoding::Utf16Be,
            Encoding::ShiftJis,
            Encoding::EucJp,
            Encoding::Iso2022Jp,
        ] {
            for bom in [false, true] {
                for text in docs {
                    let t = eve_text::Text::from(text);
                    let snap = t.snapshot();
                    let path = FsPath::new("/w/a.txt");
                    let p = SaveParams {
                        path: &path,
                        snapshot: &snap,
                        encoding,
                        bom,
                        escape: EscapePolicy::Restore,
                        expected: Expect::Any,
                        strategy: SaveStrategy::InPlace {
                            backup: crate::io::fs::InPlaceBackup::Required,
                        },
                        clear_readonly: false,
                        create_link_target: false,
                        preserved: &PreservedMap::new(),
                    };
                    let d = dry_run(&p, &CancelToken::new()).unwrap();
                    assert!(d.report.is_empty(), "{encoding:?} {bom} {text:?}");
                    let out = written(text, encoding, bom);
                    assert_eq!(
                        d.out_len,
                        out.len() as u64,
                        "{encoding:?} bom={bom} {text:?}"
                    );
                }
            }
        }
        // 空の文書でも BOM は書く (乾式走査も数える)
        assert_eq!(written("", Encoding::Utf8, true), b"\xEF\xBB\xBF");
        assert_eq!(written("", Encoding::Utf16Le, true), b"\xFF\xFE");
        assert_eq!(written("", Encoding::Utf8, false), b"");
    }
}
