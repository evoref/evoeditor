//! 文書の走査 (一致を集める・次の一致を探す・検索のジョブ、docs/03 §12.1・§12.2)。
//!
//! 走査の入口は `scan_bytes` (範囲を受ける) の 1 つ (03 §12.3 の「走査の境界」)。置換の走査 (キャプチャが要る) も同じ
//! 本体を通す (`scan_bytes_with_captures`)。中の経路は 3 つ: チャンクごとの memmem (改行を含まないリテラル)、行ごとに
//! 切り出して照合する `LineScan` (`scan_lines`)、行の窓と全文の連結 (`StreamScan`、`stream`。行をまたぎうる正規表現と、
//! `\A` などで文書の先頭 / 末尾の意味が要る正規表現。03 §12.3.1。M24 の (4) までは内部の設定で開いたときだけ)。
//!
//! 入口の約束 (03 §12.3.1 の「走査の入口の約束」): `range.start` から §12.1 の選び方で選び始めた一致を返す。始端が文書の先頭・
//! 正本の一致の終端・再開の位置 (`ScanCtx::resume` が知らせる) のとき、正本 (文書の先頭から選んだ一致) と同じになる。

use std::ops::Range;

use eve_text::{ByteOff, ByteRange, TextRead, is_grapheme_boundary};
use eve_view::JobId;

use crate::jobs::cancel::CancelToken;
use crate::jobs::{JobError, JobOutput, JobResult};
use crate::time::{Clock, Millis};

use super::limits::{SEARCH_DISCARD_BUDGET_FACTOR, SEARCH_DISCARD_BUDGET_MIN};
use super::lines::for_each_line;
use super::matcher::Matcher;
use super::matcher::SearchQuery;
use super::stream::scan_stream;
/// 全件走査の結果を送る間隔 (件数または時間、§12.2)。
const BATCH_MATCHES: usize = 256;

const BATCH_MILLIS: Millis = Millis(20);

/// 一致 (と、求めたときはキャプチャ) を受け取る関数。`false` を返すと走査を止める。
pub(super) type OnMatch<'f> = dyn FnMut(ByteRange, Option<&regex::Captures<'_>>) -> bool + 'f;

/// 走査の終わり方 (`scan_range`)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanEnd {
    /// 範囲の終わりまで走査した。
    Done,
    /// 一致を受け取る関数が `false` を返したので止めた。
    Stopped,
    /// 取り消された。
    Cancelled,
    /// 探さなかった (k に上限の無い検索語で文書が全文の連結の上限 (16 MiB) を超える、または行の窓がその上限を超える。
    /// 03 §12.3.1 の「探さない場合」)。窓が上限を超えるのは走査して初めて分かるので、それまでに渡した一致は捨てる。
    TooLarge,
    /// 重すぎるので打ち切った (処理の速さの下限、捨てた候補の予算。03 §12.3.1 の「最悪の入力の上限」)。それまでに渡した
    /// 一致は捨てる。
    TooHeavy,
}

/// 走査の条件 (`scan_range`)。
pub struct ScanCtx<'a> {
    /// 取り消し。`regex` の呼び出しごと (行の経路は行ごと、memmem はチャンクごと) に確かめる。
    pub cancel: &'a CancelToken,
    /// 時計。あれば、行の窓の 1 つの処理の時間から核を縮め (取り消しの遅れ)、処理の速さの下限で打ち切る (03 §12.3.1)。
    pub clock: Option<&'a dyn Clock>,
    /// 再開の位置 (正本の走査がそこから続く位置: その時点の探し始める位置) を知らせる。行の窓と全文の連結の経路だけが、
    /// 核ごと (連結は核の大きさの間隔ごと) に呼ぶ (03 §12.3.1 の「全件走査」)。
    pub resume: Option<&'a mut dyn FnMut(ByteOff)>,
}

impl<'a> ScanCtx<'a> {
    /// 取り消しだけの条件 (時計なし・再開の位置を知らせない)。
    pub fn new(cancel: &'a CancelToken) -> ScanCtx<'a> {
        ScanCtx {
            cancel,
            clock: None,
            resume: None,
        }
    }
}

/// 書記素の境界・単語単位で捨てた候補のために `regex` が読んだ量の予算 (03 §12.3.1 の「捨てた候補の予算」)。
pub(crate) struct Discard {
    used: usize,
    limit: usize,
    on: bool,
    /// 予算を超えた。
    pub exceeded: bool,
}

impl Discard {
    /// 走査する範囲の長さ `range_len` に対する予算。`on` が偽なら数えない (行をまたぐ検索を開くまでは v0.3 のまま)。
    pub(crate) fn new(range_len: usize, on: bool) -> Discard {
        Discard {
            used: 0,
            limit: range_len
                .saturating_mul(SEARCH_DISCARD_BUDGET_FACTOR)
                .max(SEARCH_DISCARD_BUDGET_MIN),
            on,
            exceeded: false,
        }
    }

    /// 捨てた候補のために読んだ量 `n` を数える。予算を超えたら `false`。
    pub(crate) fn spend(&mut self, n: usize) -> bool {
        if !self.on {
            return true;
        }
        self.used = self.used.saturating_add(n);
        if self.used > self.limit {
            self.exceeded = true;
            return false;
        }
        true
    }
}

/// `LineScan`: 行 `lines` (半開区間) を順に走査し、一致を `f` へ渡す。`f` が `false` を返すか取り消されたら止めて `false`。
/// `want_caps` が真で利用者の正規表現なら、一致ごとにキャプチャも渡す (置換の文字列の展開用)。
/// 外からは `scan_bytes` を通して使う (走査の境界、03 §12.3)。
#[cfg(test)]
pub(super) fn scan_lines(
    text: &dyn TextRead,
    m: &Matcher,
    lines: Range<usize>,
    cancel: &CancelToken,
    want_caps: bool,
    f: &mut OnMatch<'_>,
) -> bool {
    scan_lines_in(
        text,
        m,
        lines,
        cancel,
        want_caps,
        &mut Discard::new(0, false),
        f,
    )
}

/// `scan_lines` の本体。正規表現で捨てた候補を `discard` に数える。範囲外の一致は渡さない (走査を止めずに読み飛ばす)。
fn scan_lines_in(
    text: &dyn TextRead,
    m: &Matcher,
    lines: Range<usize>,
    cancel: &CancelToken,
    want_caps: bool,
    discard: &mut Discard,
    f: &mut OnMatch<'_>,
) -> bool {
    for_each_line(text, lines, &mut |ls, line| {
        if cancel.is_cancelled() {
            return false;
        }
        if line.len() < m.len {
            return true;
        }
        m.find_in_line(text, ls, line, want_caps, discard, &mut |r, caps| {
            !m.in_scope(r) || f(r, caps)
        })
    })
}

/// 走査の入口。`range` (行頭と行末に揃った範囲) を走査し、一致を `f` へ渡す。チャンクごとに走査できる検索語は、行に切り出さずに
/// memmem で探す (1 GB/s 級、NFR-108 / 109)。`f` が `false` を返すか取り消されたら止めて `false`。探さなかった・打ち切った
/// (行をまたぐ検索、`ScanEnd::TooLarge` / `TooHeavy`) ときも `false`。理由が要るときは `scan_range` を使う。
pub fn scan_bytes(
    text: &dyn TextRead,
    m: &Matcher,
    range: ByteRange,
    cancel: &CancelToken,
    f: &mut dyn FnMut(ByteRange) -> bool,
) -> bool {
    scan(
        text,
        m,
        range,
        &mut ScanCtx::new(cancel),
        false,
        &mut |r, _| f(r),
    ) == ScanEnd::Done
}

/// 走査の入口 (`scan_bytes` と同じ本体) で、終わり方の理由と、時計・再開の位置の知らせを扱う。行をまたぎうる正規表現
/// (`Matcher::is_stream`) では、`range.start` は char 境界ならどこでもよく (行頭でなくてよい)、`range.end` より前で始まる
/// 一致を返す (一致の終端は `range.end` を越えうる。窓は範囲の外 (後ろの k 行と前後の 1 文字) も読む)。それ以外の経路
/// (行をまたがない検索語) は今までどおりで、範囲は行頭と行末に揃える (行の途中の始端は、その行の行頭から探す)。
pub fn scan_range(
    text: &dyn TextRead,
    m: &Matcher,
    range: ByteRange,
    ctx: &mut ScanCtx<'_>,
    f: &mut dyn FnMut(ByteRange) -> bool,
) -> ScanEnd {
    scan(text, m, range, ctx, false, &mut |r, _| f(r))
}

/// `scan_bytes` と同じ入口で、利用者の正規表現なら一致ごとのキャプチャも渡す (置換、03 §12.5)。
pub(super) fn scan_bytes_with_captures(
    text: &dyn TextRead,
    m: &Matcher,
    range: ByteRange,
    cancel: &CancelToken,
    f: &mut OnMatch<'_>,
) -> bool {
    scan(text, m, range, &mut ScanCtx::new(cancel), true, f) == ScanEnd::Done
}

/// 走査の入口の本体 (`scan_bytes`・`scan_range`・`scan_bytes_with_captures`)。
fn scan(
    text: &dyn TextRead,
    m: &Matcher,
    range: ByteRange,
    ctx: &mut ScanCtx<'_>,
    want_caps: bool,
    f: &mut OnMatch<'_>,
) -> ScanEnd {
    // 範囲外の一致は渡さない (走査を止めずに読み飛ばす)。行をまたがない経路 (k = 0) を遅くしないよう、包みはどの経路も
    // 1 段だけにする (止めた理由は、取り消しの有無で分ける)
    if let Some(re) = &m.regex
        && m.is_stream()
    {
        return scan_stream(text, m, re, range, ctx, want_caps, &mut |r, caps| {
            !m.in_scope(r) || f(r, caps)
        });
    }
    let cancel = ctx.cancel;
    let ok = if m.chunked {
        scan_memmem(text, m, range, cancel, &mut |r| {
            !m.in_scope(r) || f(r, None)
        })
    } else {
        let first = text.byte_to_line(range.start).0;
        let last = if range.end.0 >= text.len_bytes() {
            text.len_lines()
        } else {
            text.byte_to_line(range.end).0
        };
        let mut discard = Discard::new(
            range.end.0.saturating_sub(range.start.0),
            m.internal.multiline,
        );
        let ok = scan_lines_in(text, m, first..last, cancel, want_caps, &mut discard, f);
        if discard.exceeded {
            return ScanEnd::TooHeavy;
        }
        ok
    };
    if ok {
        ScanEnd::Done
    } else if cancel.is_cancelled() {
        ScanEnd::Cancelled
    } else {
        ScanEnd::Stopped
    }
}

/// チャンクごとの memmem の経路 (改行を含まないリテラル)。一致は範囲の中のものだけを `f` へ渡す。
fn scan_memmem(
    text: &dyn TextRead,
    m: &Matcher,
    range: ByteRange,
    cancel: &CancelToken,
    f: &mut impl FnMut(ByteRange) -> bool,
) -> bool {
    let n = m.len;
    // 前のチャンクの末尾のうち、まだ一致に使っていない最大 n-1 バイト (畳み込み済み)
    let mut carry: Vec<u8> = Vec::new();
    // チャンクごとに確保しないよう、使い回す (畳み込んだチャンク・境目の照合用・次へ持ち越す分。PRF-11)
    let (mut lowered, mut st, mut next): (Vec<u8>, Vec<u8>, Vec<u8>) =
        (Vec::new(), Vec::new(), Vec::new());
    let mut pos = range.start.0; // 次のチャンクの文書内の開始位置
    let mut used = range.start.0; // これより前は一致に使った
    for chunk in text.chunks(range) {
        if cancel.is_cancelled() {
            return false;
        }
        let body: &[u8] = if m.fold {
            lowered.clear();
            lowered.extend_from_slice(chunk.as_bytes());
            lowered.make_ascii_lowercase();
            &lowered
        } else {
            chunk.as_bytes()
        };
        let carry_off = pos - carry.len();
        // 1. チャンクをまたぐ一致 (carry の中で始まるもの)
        if !carry.is_empty() {
            let take = (n - 1).min(body.len());
            st.clear();
            st.extend_from_slice(&carry);
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
        next.clear();
        if keep_from < end {
            if keep_from >= pos {
                next.extend_from_slice(&body[keep_from - pos..]);
            } else {
                // 短いチャンク: carry の末尾と合わせる
                next.extend_from_slice(&carry[keep_from - carry_off..]);
                next.extend_from_slice(body);
            }
        }
        std::mem::swap(&mut carry, &mut next);
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
    emit: &mut dyn FnMut(JobOutput),
) {
    let Some(m) = p.query.matcher() else {
        emit(JobOutput {
            job: p.job,
            body: JobResult::SearchDone {
                total: 0,
                count_limited: false,
                truncated: false,
            },
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
                emit(JobOutput {
                    job: p.job,
                    body: JobResult::SearchBatch {
                        matches: std::mem::take(&mut batch),
                    },
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
    // 時計を渡す: 行の窓の経路 (内部の設定で開いたとき) は、窓の処理の時間から核を縮める (取り消しの遅れ、03 §12.3.1)
    let mut ctx = ScanCtx {
        cancel,
        clock: Some(clock),
        resume: None,
    };
    let done = scan_range(
        p.snapshot,
        &m,
        ByteRange::new(ByteOff(first), ByteOff(len)),
        &mut ctx,
        &mut on_match,
    ) == ScanEnd::Done
        && scan_range(
            p.snapshot,
            &m,
            ByteRange::new(ByteOff(0), ByteOff(first)),
            &mut ctx,
            &mut on_match,
        ) == ScanEnd::Done;
    if !done && !cut {
        emit(JobOutput {
            job: p.job,
            body: JobResult::Failed {
                err: JobError::Cancelled,
            },
        });
        return;
    }
    if !batch.is_empty() {
        emit(JobOutput {
            job: p.job,
            body: JobResult::SearchBatch { matches: batch },
        });
    }
    emit(JobOutput {
        job: p.job,
        body: JobResult::SearchDone {
            total,
            count_limited,
            truncated,
        },
    });
}
