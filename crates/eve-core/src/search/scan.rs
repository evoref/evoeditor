//! 文書の走査 (一致を集める・次の一致を探す・検索のジョブ、docs/03 §12.1・§12.2)。
//!
//! 走査の入口は `scan_bytes` (範囲を受ける) の 1 つ (03 §12.3 の「走査の境界」)。中の経路は 2 つで、どちらも一致は
//! 行をまたがない: チャンクごとの memmem (改行を含まないリテラル) と、行ごとに切り出して照合する `LineScan`
//! (`scan_lines`)。複数行の正規表現 (v0.4、OQ-60) は、この入口の中に 3 つ目の経路 (`StreamScan`) として足し、
//! 呼び出し側は変えない。

use std::ops::Range;

use eve_text::{ByteOff, ByteRange, LineIdx, TextRead, is_grapheme_boundary};
use eve_view::JobId;

use crate::jobs::cancel::CancelToken;
use crate::jobs::{JobError, JobResult};
use crate::time::{Clock, Millis};

use super::matcher::Matcher;
use super::matcher::SearchQuery;
/// 全件走査の結果を送る間隔 (件数または時間、§12.2)。
const BATCH_MATCHES: usize = 256;

const BATCH_MILLIS: Millis = Millis(20);

/// `LineScan`: 行 `lines` (半開区間) を順に走査し、一致を `f` へ渡す。`f` が `false` を返すか取り消されたら止めて `false`。
/// 外からは `scan_bytes` を通して使う (走査の境界、03 §12.3)。
pub(super) fn scan_lines(
    text: &dyn TextRead,
    m: &Matcher,
    lines: Range<usize>,
    cancel: &CancelToken,
    f: &mut dyn FnMut(ByteRange) -> bool,
) -> bool {
    for l in lines {
        if cancel.is_cancelled() {
            return false;
        }
        let ls = text.line_to_byte(LineIdx(l)).0;
        let le = text.line_end_byte(LineIdx(l)).0;
        if le - ls < m.len {
            continue;
        }
        let line = text.slice_to_string(ByteRange::new(ByteOff(ls), ByteOff(le)));
        let ok = m.find_in_line(text, ls, &line, &mut |r| !m.in_scope(r) || f(r));
        if !ok {
            return false;
        }
    }
    true
}

/// 走査の入口。`range` (行頭と行末に揃った範囲) を走査し、一致を `f` へ渡す。チャンクごとに走査できる検索語は、行に切り出さずに
/// memmem で探す (1 GB/s 級、NFR-108 / 109)。`f` が `false` を返すか取り消されたら止めて `false`。
pub fn scan_bytes(
    text: &dyn TextRead,
    m: &Matcher,
    range: ByteRange,
    cancel: &CancelToken,
    f: &mut dyn FnMut(ByteRange) -> bool,
) -> bool {
    if !m.chunked {
        let first = text.byte_to_line(range.start).0;
        let last = if range.end.0 >= text.len_bytes() {
            text.len_lines()
        } else {
            text.byte_to_line(range.end).0
        };
        return scan_lines(text, m, first..last, cancel, f);
    }
    // 範囲外の一致は渡さない (走査を止めずに読み飛ばす)
    let f = &mut |r: ByteRange| !m.in_scope(r) || f(r);
    let n = m.len;
    // 前のチャンクの末尾のうち、まだ一致に使っていない最大 n-1 バイト (畳み込み済み)
    let mut carry: Vec<u8> = Vec::new();
    let mut pos = range.start.0; // 次のチャンクの文書内の開始位置
    let mut used = range.start.0; // これより前は一致に使った
    for chunk in text.chunks(range) {
        if cancel.is_cancelled() {
            return false;
        }
        let lowered;
        let body: &[u8] = if m.fold {
            lowered = chunk.to_ascii_lowercase();
            lowered.as_bytes()
        } else {
            chunk.as_bytes()
        };
        let carry_off = pos - carry.len();
        // 1. チャンクをまたぐ一致 (carry の中で始まるもの)
        if !carry.is_empty() {
            let take = (n - 1).min(body.len());
            let mut st = carry.clone();
            st.extend_from_slice(&body[..take]);
            let mut at = 0;
            while let Some(i) = m.finder.find(&st[at..]) {
                let s0 = at + i;
                if s0 >= carry.len() {
                    break;
                }
                let (so, eo) = (carry_off + s0, carry_off + s0 + n);
                if is_grapheme_boundary(text, ByteOff(so))
                    && is_grapheme_boundary(text, ByteOff(eo))
                {
                    if !f(ByteRange::new(ByteOff(so), ByteOff(eo))) {
                        return false;
                    }
                    used = eo;
                    at = s0 + n;
                } else {
                    at = s0 + 1;
                }
            }
        }
        // 2. チャンクの中の一致
        let mut at = used.saturating_sub(pos).min(body.len());
        while at < body.len() {
            let Some(i) = m.finder.find(&body[at..]) else {
                break;
            };
            let s0 = at + i;
            let (so, eo) = (pos + s0, pos + s0 + n);
            if is_grapheme_boundary(text, ByteOff(so)) && is_grapheme_boundary(text, ByteOff(eo)) {
                if !f(ByteRange::new(ByteOff(so), ByteOff(eo))) {
                    return false;
                }
                used = eo;
                at = s0 + n;
            } else {
                at = s0 + 1;
            }
        }
        // 3. 次のチャンクへ持ち越す末尾 (使った分は除く)
        let end = pos + body.len();
        let keep_from = end.saturating_sub(n - 1).max(used);
        let mut next: Vec<u8> = Vec::new();
        if keep_from < end {
            if keep_from >= pos {
                next.extend_from_slice(&body[keep_from - pos..]);
            } else {
                // 短いチャンク: carry の末尾と合わせる
                next.extend_from_slice(&carry[keep_from - carry_off..]);
                next.extend_from_slice(body);
            }
        }
        carry = next;
        pos = end;
    }
    true
}

/// `from` から先 (`forward`) か前の最初の一致を探す。端を越えたら反対側から続ける (`wrapped`)。
/// 一致が無ければ `Some(None)`、取り消されたら `None`。単発の検索ジョブ (`JobKind::FindNext`) の本体。
pub fn find_next(
    text: &dyn TextRead,
    query: &SearchQuery,
    from: ByteOff,
    forward: bool,
    cancel: &CancelToken,
) -> Option<Option<(ByteRange, bool)>> {
    let Some(m) = query.matcher() else {
        return Some(None);
    };
    let len = text.len_bytes();
    let line_start = text.line_to_byte(text.byte_to_line(from)).0.min(len);
    let mut hit: Option<ByteRange> = None;
    if forward {
        // カーソルの行から末尾まで (カーソルより前に始まる一致は除く)
        let ok = scan_bytes(
            text,
            &m,
            ByteRange::new(ByteOff(line_start), ByteOff(len)),
            cancel,
            &mut |r| {
                if r.start >= from {
                    hit = Some(r);
                    return false;
                }
                true
            },
        );
        if let Some(r) = hit {
            return Some(Some((r, false)));
        }
        if !ok && cancel.is_cancelled() {
            return None;
        }
        // 先頭から (最初の一致)
        let ok = scan_bytes(
            text,
            &m,
            ByteRange::new(ByteOff(0), ByteOff(len)),
            cancel,
            &mut |r| {
                hit = Some(r);
                false
            },
        );
        if !ok && hit.is_none() {
            return None;
        }
        return Some(hit.map(|r| (r, true)));
    }
    // 前へ: 全体を 1 回走査して、`from` より前で終わる最後の一致と、全体の最後の一致を取る
    let (mut before, mut last): (Option<ByteRange>, Option<ByteRange>) = (None, None);
    let ok = scan_bytes(
        text,
        &m,
        ByteRange::new(ByteOff(0), ByteOff(len)),
        cancel,
        &mut |r| {
            if r.end <= from {
                before = Some(r);
            }
            last = Some(r);
            true
        },
    );
    if !ok {
        return None;
    }
    Some(before.map(|r| (r, false)).or(last.map(|r| (r, true))))
}

/// 全件走査のジョブ本体 (§12.2)。`from` のある行から文書の末尾まで、続けて先頭からその行の手前までを走査し、
/// 256 件または 20 ms ごとに `SearchBatch` を送る。保持する一致は `max_matches` 件まで、数えるのは `count_limit` 件まで。
pub struct SearchJob<'a> {
    /// 結果に付けるジョブ。
    pub job: JobId,
    /// 検索する本文。
    pub snapshot: &'a dyn TextRead,
    /// 検索の条件。
    pub query: &'a SearchQuery,
    /// 走査を始める位置 (バイト。この位置のある行から走査する)。
    pub from: ByteOff,
    /// 保持する一致の上限 (件)。
    pub max_matches: usize,
    /// 数える一致の上限 (件)。
    pub count_limit: u64,
}

/// 全件走査を実行し、`SearchBatch` を順に、最後に `SearchDone` を `emit` に渡す (03 §12.2)。
/// キャンセルされたら `Failed { Cancelled }` で終わる。検索語が空・照合器を作れなければ 0 件の `SearchDone`。
pub fn run_search(
    p: SearchJob<'_>,
    clock: &dyn Clock,
    cancel: &CancelToken,
    emit: &mut dyn FnMut(JobResult),
) {
    let Some(m) = p.query.matcher() else {
        emit(JobResult::SearchDone {
            job: p.job,
            total: 0,
            count_limited: false,
            truncated: false,
        });
        return;
    };
    let len = p.snapshot.len_bytes();
    let first = p
        .snapshot
        .line_to_byte(p.snapshot.byte_to_line(p.from))
        .0
        .min(len);
    let mut batch: Vec<ByteRange> = Vec::new();
    let (mut total, mut kept) = (0u64, 0usize);
    let (mut count_limited, mut truncated) = (false, false);
    let mut last = clock.now();
    let mut cut = false;
    let mut sent_any = false;
    let mut on_match = |r: ByteRange| {
        total += 1;
        if kept < p.max_matches {
            kept += 1;
            batch.push(r);
            let now = clock.now();
            // 最初の一致は待たずに送る (NFR-108)
            if !sent_any || batch.len() >= BATCH_MATCHES || now.saturating_sub(last) >= BATCH_MILLIS
            {
                sent_any = true;
                emit(JobResult::SearchBatch {
                    job: p.job,
                    matches: std::mem::take(&mut batch),
                });
                last = now;
            }
        } else {
            truncated = true;
        }
        if total >= p.count_limit {
            count_limited = true;
            cut = true;
            return false;
        }
        true
    };
    let done = scan_bytes(
        p.snapshot,
        &m,
        ByteRange::new(ByteOff(first), ByteOff(len)),
        cancel,
        &mut on_match,
    ) && scan_bytes(
        p.snapshot,
        &m,
        ByteRange::new(ByteOff(0), ByteOff(first)),
        cancel,
        &mut on_match,
    );
    if !done && !cut {
        emit(JobResult::Failed {
            job: p.job,
            err: JobError::Cancelled,
        });
        return;
    }
    if !batch.is_empty() {
        emit(JobResult::SearchBatch {
            job: p.job,
            matches: batch,
        });
    }
    emit(JobResult::SearchDone {
        job: p.job,
        total,
        count_limited,
        truncated,
    });
}
