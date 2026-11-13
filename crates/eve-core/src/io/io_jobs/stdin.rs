//! 標準入力の読込ジョブ (`JobKind::OpenStdin`、docs/03 §9.2.2。v0.4 の M25)。
//!
//! ファイルの読込 (`open_inner`) と同じ関数で復号・文字コードの判定・改行の統計・保持方式の記録を行い、次の点を変える。
//!
//! - **確認を出さない**: バイナリ・UTF-32 の BOM・不正バイトの比率の確認は、「読み取り専用で開く」を選んだのと同じに扱う
//!   (理由 `Binary` / `InvalidBytes` / `StatefulEncodingWithInvalidBytes` を付ける)。確認の後に先頭から読み直せないため。
//! - **判定は 64 KiB が揃ってから**: パイプは小さく区切って届くので、先頭の 64 KiB が揃うか終わる (EOF・取消し・失敗) まで
//!   貯めてから判定する。届き方 (1 バイトずつでも) で結果が変わらない。判定のやり直し (03 §8.2.1) はしない。
//! - **文字の境界で切る**: 読み終えなかったとき (上限・取消し・読込の失敗・確保の失敗) は、最後の文字の境界
//!   (`Decoder::at_char_boundary`) までを開き、切れた多バイト列 (持ち越し) は開かない。読み終えた (EOF) ときだけ、
//!   ファイルの末尾と同じく持ち越しを不正バイトとして出す。そのために、復号した出力と元のバイト列は文字の境界に来るまで
//!   本文と保持方式の検査器に渡さず持つ (`Uncommitted`)。まとめて復号した後の末尾の数バイトは 1 バイトずつ復号して
//!   境界を探す。
//! - **上限**: 読んだバイト数か復号した本文のバイト数が `file.huge_file_warn_bytes` を超えたら読むのをやめる
//!   (`StdinEnd::Limit`)。上限の手前は 1 バイトずつ復号する。
//! - **確保の失敗**: 復号の作業領域の `try_reserve` と、本文が倍になるごとの確保の見込みの確かめ (`probe_reserve`) で
//!   見つけ、構築済みの分を返す (`StdinEnd::OutOfMemory`)。
//! - 1 バイト以上受け取って読み終えなかったら `LoadIncomplete` を付ける。開いた量は `StdinDone.bytes`。
//! - 進捗 `Progress` は受け取ったバイト数を `total = 0` で送る (大きさは分からない)。

use eve_encoding::{DetectOptions, DetectedKind, Detection, RoundtripChecker, detect};
use eve_text::{TextBuilder, TextRead};
use eve_view::JobId;

use super::{
    DecodedFacts, HUGE_FILE_WARN, OpenOptions, SNIFF_LEN, content_reasons, debug_assert_inv_d7,
    line_stats_result, loaded_format, preserved_from_records, probe_reserve,
};
use crate::document::{LineStatsScanner, ReadOnlyReason, ReadOnlyReasons};
use crate::io::fs::{ChunkError, FsError, FsErrorKind};
use crate::jobs::cancel::CancelToken;
use crate::jobs::{JobError, JobOutput, JobPorts, JobResult, StdinEnd};
use eve_encoding::{Decoder, Encoding};

/// 上限の手前で 1 バイトずつ読む区間 (読んだバイト数)。
const TAIL_RAW: u64 = 64;
/// 同じく、復号した本文のバイト数の区間。1 バイトの入力が最大 4 バイト (私用文字のエスケープ) になるので、その分の余裕。
const TAIL_DECODED: u64 = TAIL_RAW * 4;
/// 1 バイトの入力が増やす本文の最大 (バイト)。復号器が持ち越していた不正バイト (最大 3) とその 1 バイトを、それぞれ
/// 4 バイトの私用文字にする。
const MAX_GROWTH_PER_BYTE: usize = 16;
/// まとめて復号した後に、1 バイトずつ復号して文字の境界を探す末尾のバイト数 (持ち越しは最大 3 バイト)。
const STEP_BYTES: usize = 4;
/// 文字の境界の無いまま持つ元のバイト列の上限。これを超えるのは、境界に戻らない不正なバイト列 (ISO-2022-JP の壊れた
/// エスケープ列の繰り返しなど) だけで、そのときは境界を待たずに渡し、読み終えなかったときも持ち越しを不正バイトとして
/// 出す (内容を黙って捨てない)。
const PENDING_CAP: usize = 64;
/// 確保の見込みを確かめ始める本文の大きさ (以後、倍になるごと)。
const PROBE_FIRST: u64 = 1024 * 1024;

/// 判定の結果。
struct Plan {
    enc: Encoding,
    binary_confirmed: bool,
}

/// 最後の文字の境界より後の、本文と検査器にまだ渡していない分。
#[derive(Default)]
struct Uncommitted {
    /// 元のバイト列。`feed` の呼び出しの間は、渡されたスライスの `base..` も未確定 (呼び出しの終わりにここへ写す)。
    raw: Vec<u8>,
    /// 復号の出力。
    text: String,
}

/// 読込の途中の状態。
struct Load {
    opts: OpenOptions,
    limit: u64,
    plan: Option<Plan>,
    dec: Option<Decoder>,
    chk: Option<RoundtripChecker>,
    /// 保持方式の検査器に渡さない先頭の BOM の残り (バイト)。
    bom_skip: usize,
    builder: TextBuilder,
    line_stats: LineStatsScanner,
    out: String,
    /// 復号器に渡した元のバイト数 (未確定を含む)。
    read_total: u64,
    /// 本文に足した (確定した) バイト数。
    decoded_total: u64,
    /// 検査器に渡した (開いた) 元のバイト数。
    committed_raw: u64,
    pend: Uncommitted,
    /// 境界を待たずに渡した (`PENDING_CAP` を超えた)。次の境界で下ろす。
    lost: bool,
    /// 上限・確保の失敗で読むのをやめた。
    end: Option<StdinEnd>,
    /// 次に確保の見込みを確かめる本文の大きさ。
    next_probe: u64,
}

impl Load {
    fn new(opts: OpenOptions, long_line_threshold: usize) -> Load {
        Load {
            opts,
            limit: opts.huge_file_warn_bytes.unwrap_or(HUGE_FILE_WARN),
            plan: None,
            dec: None,
            chk: None,
            bom_skip: 0,
            builder: TextBuilder::new(),
            line_stats: LineStatsScanner::new(long_line_threshold),
            out: String::new(),
            read_total: 0,
            decoded_total: 0,
            committed_raw: 0,
            pend: Uncommitted::default(),
            lost: false,
            end: None,
            next_probe: PROBE_FIRST,
        }
    }

    /// 先頭 (`head`、最大 64 KiB) で文字コードを判定する (03 §8.2)。`complete` は入力の全部が `head` (EOF まで読んだ)。
    /// 確認は出さず、バイナリ・未対応の BOM は UTF-8 で読む (「読み取り専用で開く」と同じ)。
    fn start(&mut self, head: &[u8], complete: bool) {
        let head = &head[..head.len().min(SNIFF_LEN)];
        let det_opts = DetectOptions::default().with_forced(self.opts.forced);
        let Detection { kind, .. } = detect(head, complete, &det_opts);
        let binary_confirmed =
            matches!(kind, DetectedKind::Binary | DetectedKind::UnsupportedBom(_));
        let (enc, bom_len) = match kind {
            DetectedKind::Utf8 { bom_len } => (Encoding::Utf8, bom_len),
            DetectedKind::Encoded { encoding, bom_len } => (encoding, bom_len),
            _ => (Encoding::Utf8, 0),
        };
        self.dec = Some(Decoder::new(enc, bom_len > 0));
        // 保持方式の記録も集める (03 §8.7.1。ファイルと同じ検査器)。BOM は検査器に渡さない
        self.chk = Some(RoundtripChecker::with_records(enc));
        self.bom_skip = bom_len;
        self.plan = Some(Plan {
            enc,
            binary_confirmed,
        });
    }

    /// 保持方式の検査器に元のバイト列を渡す (先頭の BOM を除く)。
    fn check(&mut self, bytes: &[u8]) {
        self.committed_raw += bytes.len() as u64;
        let skip = self.bom_skip.min(bytes.len());
        self.bom_skip -= skip;
        if let Some(chk) = self.chk.as_mut() {
            chk.feed(&bytes[skip..], false);
        }
    }

    /// 未確定の分 (`pend` と、スライスの `s[base..pos]`) を確定する: 本文と検査器に渡す。
    fn commit(&mut self, s: &[u8], base: &mut usize, pos: usize) {
        let raw = std::mem::take(&mut self.pend.raw);
        self.check(&raw);
        self.check(&s[*base..pos]);
        *base = pos;
        let text = std::mem::take(&mut self.pend.text);
        self.decoded_total += text.len() as u64;
        self.builder.append(&text);
        self.line_stats.feed(&text);
        // 作業領域は使い回す
        self.pend.raw = raw;
        self.pend.raw.clear();
        self.pend.text = text;
        self.pend.text.clear();
        // 本文が倍になるごとに、その大きさを確保できるかを確かめる (rope の構築は確保の失敗を返せない。03 §9.2.2)
        if self.decoded_total >= self.next_probe {
            if let Err(JobError::OutOfMemory { requested }) = probe_reserve(self.decoded_total) {
                self.end = Some(StdinEnd::OutOfMemory { requested });
            }
            self.next_probe = self.decoded_total.saturating_mul(2);
        }
    }

    /// 復号した後: 文字の境界なら確定する。境界の無いまま `PENDING_CAP` を超えたら、境界を待たずに確定する。
    fn settle(&mut self, s: &[u8], base: &mut usize, pos: usize) {
        let boundary = self.dec.as_ref().is_some_and(Decoder::at_char_boundary);
        if boundary {
            self.lost = false;
        } else if self.pend.raw.len() + (pos - *base) > PENDING_CAP {
            self.lost = true;
        }
        if boundary || self.lost {
            self.commit(s, base, pos);
        }
    }

    /// 判定の後のバイト列を読む。上限・確保の失敗で `self.end` を立てたら、残りは捨てる。
    fn feed(&mut self, s: &[u8]) {
        let mut base = 0;
        let mut pos = 0;
        while pos < s.len() && self.end.is_none() {
            // まとめて復号するのは、文字の境界 (未確定が無い) からだけ。上限の手前と末尾の数バイトは 1 バイトずつ
            let at_boundary = self.pend.raw.is_empty() && base == pos;
            if at_boundary || self.lost {
                let pending_text = self.pend.text.len() as u64;
                let k_raw = self
                    .limit
                    .saturating_sub(TAIL_RAW)
                    .saturating_sub(self.read_total);
                let k_dec = self
                    .limit
                    .saturating_sub(TAIL_DECODED)
                    .saturating_sub(self.decoded_total + pending_text)
                    / 4;
                let k = usize::try_from(k_raw.min(k_dec))
                    .unwrap_or(usize::MAX)
                    .min((s.len() - pos).saturating_sub(STEP_BYTES));
                if k > 0 {
                    if self.bulk(s, &mut base, pos, k) {
                        pos += k;
                    }
                    continue;
                }
            }
            if self.step(s, &mut base, pos) {
                pos += 1;
            }
        }
        // 未確定のスライスの分は持ち越す (次の呼び出し・終わりで扱う)
        self.pend.raw.extend_from_slice(&s[base..pos]);
    }

    /// `s[pos..pos + k]` をまとめて復号する (上限まで十分に余裕がある区間)。確保に失敗したら復号せずに `false`。
    fn bulk(&mut self, s: &[u8], base: &mut usize, pos: usize, k: usize) -> bool {
        self.out.clear();
        // 復号のバッファ: 1 バイトが最大 4 バイト (不正バイトの私用文字) になる
        let need = k.saturating_mul(4).saturating_add(16);
        if self.out.try_reserve(need).is_err() || self.pend.text.try_reserve(need).is_err() {
            self.end = Some(StdinEnd::OutOfMemory {
                requested: need as u64,
            });
            return false;
        }
        let Some(dec) = self.dec.as_mut() else {
            return false;
        };
        dec.decode(&s[pos..pos + k], false, &mut self.out);
        self.read_total += k as u64;
        if self.pend.text.is_empty() {
            std::mem::swap(&mut self.pend.text, &mut self.out);
        } else {
            self.pend.text.push_str(&self.out);
        }
        self.settle(s, base, pos + k);
        true
    }

    /// `s[pos]` を 1 バイトだけ復号する。上限に届いたら復号せずに `false`。
    fn step(&mut self, s: &[u8], base: &mut usize, pos: usize) -> bool {
        // 読んだバイト数が上限を超える: このバイトは読まない (最後の文字の境界で切る)
        if self.read_total + 1 > self.limit {
            self.end = Some(StdinEnd::Limit);
            return false;
        }
        if self.pend.text.try_reserve(MAX_GROWTH_PER_BYTE).is_err() {
            self.end = Some(StdinEnd::OutOfMemory {
                requested: MAX_GROWTH_PER_BYTE as u64,
            });
            return false;
        }
        let Some(dec) = self.dec.as_mut() else {
            return false;
        };
        dec.decode(&s[pos..=pos], false, &mut self.pend.text);
        self.read_total += 1;
        // 復号した本文のバイト数が上限を超えた: 最後の文字の境界 (このバイトの前) で切る
        if self.decoded_total + self.pend.text.len() as u64 > self.limit {
            self.end = Some(StdinEnd::Limit);
            return true;
        }
        self.settle(s, base, pos + 1);
        true
    }

    /// 読み終える。`end` が EOF なら持ち越しを不正バイトとして出す (それで上限を超えるなら、最後の文字の境界で切って
    /// `Limit` にする)。読み終えなかったときは、未確定の分 (切れた多バイト列) を捨てる。捨てた出力の中の不正バイトの数を返す。
    fn finish(&mut self, end: &mut StdinEnd) -> u32 {
        let mut dropped = 0;
        let mut flushed = String::new();
        let flush = matches!(end, StdinEnd::Eof) || self.lost;
        if flush && let Some(dec) = self.dec.as_mut() {
            dec.decode(&[], true, &mut flushed);
        }
        let over = self.decoded_total + (self.pend.text.len() + flushed.len()) as u64 > self.limit;
        if flush && !(over && !self.lost) {
            self.pend.text.push_str(&flushed);
            let mut base = 0;
            self.commit(&[], &mut base, 0);
        } else {
            if flush {
                // EOF の持ち越しで上限を超えた
                *end = StdinEnd::Limit;
                dropped += count_escapes(&flushed);
            }
            // 最後の文字の境界で切る: 未確定の出力と元のバイト列を捨て、復号器は終えない
            dropped += count_escapes(&self.pend.text);
            self.read_total -= self.pend.raw.len() as u64;
            self.pend = Uncommitted::default();
        }
        if let Some(chk) = self.chk.as_mut() {
            chk.feed(&[], true);
        }
        dropped
    }

    fn line_stats_threshold(&self) -> usize {
        self.opts
            .long_line_threshold
            .unwrap_or(crate::document::LONG_LINE_THRESHOLD)
    }
}

/// 捨てた出力の中の、不正バイトのエスケープ文字の数 (衝突のある文書では真の私用文字も数える。その文書は INV-D7 の
/// 対象外)。
fn count_escapes(s: &str) -> u32 {
    u32::try_from(
        s.chars()
            .filter(|&c| eve_encoding::unescape_char(c).is_some())
            .count(),
    )
    .unwrap_or(u32::MAX)
}

/// 標準入力を読み終える (`JobKind::OpenStdin`)。取消しも `StdinDone` で返す (02 §9.3 の採否表)。
pub(crate) fn run_open_stdin(
    id: JobId,
    opts: OpenOptions,
    ports: &JobPorts,
    cancel: &CancelToken,
    emit: &mut dyn FnMut(JobOutput),
) -> JobResult {
    let long_line_threshold = opts
        .long_line_threshold
        .unwrap_or(crate::document::LONG_LINE_THRESHOLD);
    // 進捗: 読み始めてすぐに 1 件 (大きさは分からないので total = 0)、以後は 100 ms 以上あけて (10 Hz 以下)
    let progress = |bytes| JobOutput {
        job: id,
        body: JobResult::Progress { bytes, total: 0 },
    };
    emit(progress(0));
    let mut last_progress = ports.clock.now();
    let mut load = Load::new(opts, long_line_threshold);
    // 最初から読めない (Port が無い・2 回目の取得): 空の文書と読取の失敗
    let Some(mut reader) = ports.stdin.as_ref().and_then(|s| s.take()) else {
        return finish(load, StdinEnd::Error(FsError::new(FsErrorKind::Other)), 0);
    };
    // 判定の前に貯める先頭は、先に確保する (途中で確保に失敗して、受け取ったチャンクを捨てない)
    let mut sniff: Vec<u8> = Vec::new();
    if sniff.try_reserve_exact(SNIFF_LEN).is_err() {
        return finish(
            load,
            StdinEnd::OutOfMemory {
                requested: SNIFF_LEN as u64,
            },
            0,
        );
    }
    let mut received: u64 = 0;
    let end = loop {
        if cancel.is_cancelled() {
            break StdinEnd::Stopped;
        }
        let chunk = match reader.next_chunk() {
            Ok(Some(c)) => c,
            Ok(None) => break StdinEnd::Eof,
            Err(ChunkError::Io(e)) => break StdinEnd::Error(e),
            Err(_) => break StdinEnd::Error(FsError::new(FsErrorKind::Other)),
        };
        // 空のチャンクは `wake` の印 (取消しを見に戻る)
        if chunk.is_empty() {
            continue;
        }
        received += chunk.len() as u64;
        if load.plan.is_none() {
            // 先頭の 64 KiB が揃うまで貯める (03 §9.2.2 の「文字コードの判定」。容量は先に確保した)
            let take = (SNIFF_LEN - sniff.len()).min(chunk.len());
            sniff.extend_from_slice(&chunk[..take]);
            if sniff.len() == SNIFF_LEN {
                load.start(&sniff, false);
                load.feed(&sniff);
                sniff = Vec::new();
                load.feed(&chunk[take..]);
            }
        } else {
            load.feed(chunk);
        }
        if let Some(end) = load.end.take() {
            break end;
        }
        let now = ports.clock.now();
        if now.saturating_sub(last_progress) >= crate::time::Millis(100) {
            last_progress = now;
            emit(progress(received));
        }
    };
    // 読むのをやめる: 読み手を手放して `eve-stdin` を止める (書き手に早く知らせる。02 §4.4)
    drop(reader);
    if load.plan.is_none() && !sniff.is_empty() {
        // 64 KiB が揃う前に終わった: あるだけで判定する (EOF なら入力の全部)
        load.start(&sniff, matches!(end, StdinEnd::Eof));
        load.feed(&sniff);
    }
    // 判定の後の feed が上限・確保の失敗を立てていれば、そちらが終わりの理由
    let end = load.end.take().unwrap_or(end);
    finish(load, end, received)
}

/// 本文と形式を組み立てて `StdinDone` にする。`received` は受け取ったバイト数 (`LoadIncomplete` の判定に使う)。
fn finish(mut load: Load, end: StdinEnd, received: u64) -> JobResult {
    let mut end = end;
    let dropped_invalid = load.finish(&mut end);
    // 確定の後の確保の見込みの確かめで止まった分は、終わりの理由にしない (読み終えた後なので)
    load.end = None;
    let opened = load.committed_raw;
    debug_assert_eq!(opened, load.read_total, "開いた量と復号器に渡した量");
    let decoded_total = load.decoded_total;
    let long_line_threshold = load.line_stats_threshold();
    let opts = load.opts;
    // 空の入力は判定なしで終わる
    let forced = opts.forced.unwrap_or(Encoding::Utf8);
    let chk = load.chk.take().unwrap_or_else(|| {
        let mut c = RoundtripChecker::with_records(forced);
        c.feed(&[], true);
        c
    });
    let (enc, binary_confirmed) = load
        .plan
        .as_ref()
        .map_or((forced, false), |p| (p.enc, p.binary_confirmed));
    let text = load.builder.finish();
    let (eol_stats, long_lines) = load.line_stats.finish();
    let (mut roundtrip, records) = chk.finish_with_records();
    let mut decoded = load.dec.as_ref().map_or(
        DecodedFacts {
            has_bom: false,
            invalid_bytes: 0,
            pua_escape: if forced.is_stateful() {
                eve_encoding::PuaEscapeMode::DisabledStateful
            } else {
                eve_encoding::PuaEscapeMode::Enabled
            },
            collision_with_invalid: false,
        },
        DecodedFacts::of,
    );
    // 切った分の不正バイトは本文に無い
    decoded.invalid_bytes = decoded.invalid_bytes.saturating_sub(dropped_invalid);
    decoded.collision_with_invalid &= decoded.invalid_bytes > 0;
    // 保持方式の記録はファイルと同じ関数で文書の表にする (03 §8.7.1。上限を超えたら `Overflow` の「保持なし」)
    let (preserved, status) = preserved_from_records(&records, enc, decoded.invalid_bytes, &text);
    drop(records);
    roundtrip.preserve = status;
    let format = loaded_format(
        enc,
        decoded,
        roundtrip,
        line_stats_result(eol_stats, long_lines, &opts, long_line_threshold),
    );
    debug_assert_inv_d7(&text, &format);
    let mut detected: ReadOnlyReasons = opts.pending;
    content_reasons(
        &mut detected,
        &opts,
        enc,
        &format,
        decoded,
        binary_confirmed,
    );
    // 読み終えなかった (上限・取消し・失敗・確保の失敗) 文書は、1 バイト以上受け取ったときだけ `LoadIncomplete`
    // (03 §9.2.2 の「結果」。切った後に開いた量が 0 でも、途中までであることを示す)
    if end.is_incomplete() && received > 0 {
        detected.insert(ReadOnlyReason::LoadIncomplete);
    }
    debug_assert_eq!(text.len_bytes() as u64, decoded_total);
    JobResult::StdinDone {
        text,
        format,
        detected,
        preserved,
        bytes: opened,
        end,
    }
}
