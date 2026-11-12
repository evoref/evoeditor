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

use super::limits::{
    SEARCH_DISCARD_BUDGET_FACTOR, SEARCH_DISCARD_BUDGET_MIN, SEARCH_RESUME_STEP_BYTES,
};
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

impl ScanEnd {
    /// 探さなかった・打ち切った理由 (`TooLarge` / `TooHeavy`)。それ以外は `None`。
    pub fn limit(self) -> Option<ScanLimit> {
        match self {
            ScanEnd::TooLarge => Some(ScanLimit::TooLarge),
            ScanEnd::TooHeavy => Some(ScanLimit::TooHeavy),
            ScanEnd::Done | ScanEnd::Stopped | ScanEnd::Cancelled => None,
        }
    }
}

/// 検索を探さなかった・打ち切った理由 (03 §12.3.1 の「探さない場合」と「最悪の入力の上限」)。検索バーの「探さない理由」と、
/// 検索のジョブの結果 (`SearchDone::limit`・`SearchLimited`) が運ぶ。どちらでも、それまでに届いた一致 (強調・保持・件数) は捨てる。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanLimit {
    /// 行の数に上限の無い検索語で文書が全文の連結の上限 (16 MiB) を超える、または行の窓がその上限を超える。
    TooLarge,
    /// 重すぎる (上限の無い重い検索語・処理の速さの下限・捨てた候補の予算)。
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

/// `scan_range` と同じ入口で、利用者の正規表現なら一致ごとのキャプチャも渡す (置換、03 §12.5)。
pub(super) fn scan_with_captures(
    text: &dyn TextRead,
    m: &Matcher,
    range: ByteRange,
    ctx: &mut ScanCtx<'_>,
    f: &mut OnMatch<'_>,
) -> ScanEnd {
    scan(text, m, range, ctx, true, f)
}

/// 走査の入口の本体 (`scan_bytes`・`scan_range`・`scan_with_captures`)。
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

/// 走査の入口で `range` を走査する。最後まで走査したか、`f` が止めたら `Ok`。取り消し・探さない・打ち切りは `Err` (その終わり方)。
fn scan_ok(
    text: &dyn TextRead,
    m: &Matcher,
    range: ByteRange,
    clock: Option<&dyn Clock>,
    cancel: &CancelToken,
    f: &mut dyn FnMut(ByteRange) -> bool,
) -> Result<(), ScanEnd> {
    let mut ctx = ScanCtx {
        cancel,
        clock,
        resume: None,
    };
    match scan_range(text, m, range, &mut ctx, f) {
        ScanEnd::Done | ScanEnd::Stopped => Ok(()),
        e => Err(e),
    }
}

/// 単発の検索 (`JobKind::FindNext`) の条件。
pub struct FindJob<'a> {
    /// 検索する本文。
    pub snapshot: &'a dyn TextRead,
    /// 検索の条件。
    pub query: &'a SearchQuery,
    /// 起点 (前へは選択の始端、次へは選択の終端)。
    pub from: ByteOff,
    /// `true` なら後ろへ、`false` なら前へ探す。
    pub forward: bool,
    /// 正本の走査を始められる位置 (再開の位置と、今の一致の終端。行をまたぐ検索の規則だけが使う。文書の先頭は無くても使う)。
    pub resume: &'a [ByteOff],
}

/// `from` から先 (`forward`) か前の最初の一致を探す。端を越えたら反対側から続ける (`wrapped`)。
/// 一致が無ければ `Ok(None)`。単発の検索ジョブ (`JobKind::FindNext`) の本体。ジョブからは時計を渡す (核の縮めと処理の速さの
/// 下限、03 §12.3.1)。
///
/// 行をまたぐ検索の規則 (`Matcher::whole_doc`、03 §12.3.1 の「次 / 前」の (3)) では、一致の集合を文書の先頭から選んだ正本に
/// 合わせる: 前へは、起点以前で最も近い正本の位置 (`resume`。無ければ文書の先頭) から走査して起点以降に始まる最初の一致を、
/// 後ろへは、起点より前の最も近い正本の位置から起点まで走査して起点以前に終わる最後の一致を採り、無ければ 1 つ前の位置から
/// 探し直す。それ以外の検索語は v0.3 のまま (起点の行から探す。k が 0 なら行頭は正本の位置)。
///
/// # Errors
///
/// 取り消された (`Cancelled`)、探さなかった (`TooLarge`)、重すぎて打ち切った (`TooHeavy`)。
pub fn find_next(
    p: &FindJob<'_>,
    clock: Option<&dyn Clock>,
    cancel: &CancelToken,
) -> Result<Option<(ByteRange, bool)>, ScanEnd> {
    let Some(m) = p.query.matcher() else {
        return Ok(None);
    };
    let text = p.snapshot;
    let (from, forward) = (p.from, p.forward);
    let len = text.len_bytes();
    let all = ByteRange::new(ByteOff(0), ByteOff(len));
    if m.whole_doc() {
        return find_canonical(
            text,
            &m,
            from.min(ByteOff(len)),
            forward,
            p.resume,
            clock,
            cancel,
        );
    }
    let line_start = text.line_to_byte(text.byte_to_line(from)).0.min(len);
    let mut hit: Option<ByteRange> = None;
    if forward {
        // カーソルの行から末尾まで (カーソルより前に始まる一致は除く)
        scan_ok(
            text,
            &m,
            ByteRange::new(ByteOff(line_start), ByteOff(len)),
            clock,
            cancel,
            &mut |r| {
                if r.start >= from {
                    hit = Some(r);
                    return false;
                }
                true
            },
        )?;
        if let Some(r) = hit {
            return Ok(Some((r, false)));
        }
        // 先頭から (最初の一致)
        scan_ok(text, &m, all, clock, cancel, &mut |r| {
            hit = Some(r);
            false
        })?;
        return Ok(hit.map(|r| (r, true)));
    }
    // 前へ: 全体を 1 回走査して、`from` より前で終わる最後の一致と、全体の最後の一致を取る
    let (mut before, mut last): (Option<ByteRange>, Option<ByteRange>) = (None, None);
    scan_ok(text, &m, all, clock, cancel, &mut |r| {
        if r.end <= from {
            before = Some(r);
        }
        last = Some(r);
        true
    })?;
    Ok(before.map(|r| (r, false)).or(last.map(|r| (r, true))))
}

/// 行をまたぐ検索の規則の次 / 前 (`find_next` の本体の一つ)。
fn find_canonical(
    text: &dyn TextRead,
    m: &Matcher,
    from: ByteOff,
    forward: bool,
    resume: &[ByteOff],
    clock: Option<&dyn Clock>,
    cancel: &CancelToken,
) -> Result<Option<(ByteRange, bool)>, ScanEnd> {
    let len = ByteOff(text.len_bytes());
    // 正本の位置 (文書の先頭を含む、昇順で重複なし)
    let mut points: Vec<ByteOff> = resume.iter().copied().filter(|p| *p <= len).collect();
    points.push(ByteOff(0));
    points.sort_unstable();
    points.dedup();
    let mut hit: Option<ByteRange> = None;
    if forward {
        let start = points
            .iter()
            .copied()
            .rfind(|p| *p <= from)
            .unwrap_or(ByteOff(0));
        scan_ok(
            text,
            m,
            ByteRange::new(start, len),
            clock,
            cancel,
            &mut |r| {
                if r.start >= from {
                    hit = Some(r);
                    return false;
                }
                true
            },
        )?;
        if let Some(r) = hit {
            return Ok(Some((r, false)));
        }
        // 折り返し: 文書の最初の一致 (起点以降には無いので、起点より前で始まる)
        scan_ok(
            text,
            m,
            ByteRange::new(ByteOff(0), from),
            clock,
            cancel,
            &mut |r| {
                hit = Some(r);
                false
            },
        )?;
        return Ok(hit.map(|r| (r, true)));
    }
    // 前へ: 起点より前の正本の位置から起点まで走査し、起点以前に終わる最後の一致。無ければ 1 つ前の位置から、前に読んだ所の
    // 手前まで ([p', p)) を探し直す (読み直さないので、全体で文書を 1 回読む量に収まる)。区間の終わりより前で始まり、終わりを
    // 越える一致も、その区間の走査が返す
    let mut end = from;
    for &p in points.iter().rev().filter(|p| **p < from) {
        scan_ok(text, m, ByteRange::new(p, end), clock, cancel, &mut |r| {
            if r.end <= from {
                hit = Some(r);
            }
            true
        })?;
        if let Some(r) = hit {
            return Ok(Some((r, false)));
        }
        end = p;
    }
    // 折り返し: 文書の最後の一致 (後ろの位置から探し、無ければ 1 つ前の位置から、前に読んだ所の手前まで)
    let mut end = len;
    for &p in points.iter().rev() {
        scan_ok(text, m, ByteRange::new(p, end), clock, cancel, &mut |r| {
            hit = Some(r);
            true
        })?;
        if let Some(r) = hit {
            return Ok(Some((r, true)));
        }
        end = p;
    }
    Ok(None)
}

/// 全件走査のジョブ本体 (§12.2)。256 件または 20 ms ごとに `SearchBatch` を送る。保持する一致は `max_matches` 件まで、数えるのは
/// `count_limit` 件まで。走査の順は、行をまたぐ検索の規則 (`Matcher::whole_doc`) では文書の先頭から末尾まで (正本、03 §12.3.1 の
/// 「全件走査」)、それ以外は `from` のある行から文書の末尾まで、続けて先頭からその行の手前まで (v0.3 のまま)。
pub struct SearchJob<'a> {
    /// 結果に付けるジョブ。
    pub job: JobId,
    /// 検索する本文。
    pub snapshot: &'a dyn TextRead,
    /// 検索の条件。
    pub query: &'a SearchQuery,
    /// 起点 (バイト)。行をまたぐ検索の規則では、この位置以降に始まる最初の一致を `first_after` で知らせる。それ以外は、
    /// この位置のある行から走査する。
    pub from: ByteOff,
    /// 保持する一致の上限 (件)。
    pub max_matches: usize,
    /// 数える一致の上限 (件)。
    pub count_limit: u64,
}

/// 全件走査を実行し、`SearchBatch` を順に、最後に `SearchDone` を `emit` に渡す (03 §12.2・§12.3.1)。
/// キャンセルされたら `Failed { Cancelled }` で終わる。検索語が空・照合器を作れなければ 0 件の `SearchDone`。探さなかった・
/// 重すぎて打ち切った (`ScanEnd::TooLarge` / `TooHeavy`) ときは、0 件と理由 (`limit`) の `SearchDone` で終わる (それまでに
/// 送った一致は、受け取る側が捨てる)。行をまたぐ検索の規則では、走査の再開の位置を 1 MiB ごとに記録して `SearchDone` で返す。
pub fn run_search(
    p: SearchJob<'_>,
    clock: &dyn Clock,
    cancel: &CancelToken,
    emit: &mut dyn FnMut(JobOutput),
) {
    let job = p.job;
    let empty_done = |emit: &mut dyn FnMut(JobOutput), limit: Option<ScanLimit>| {
        emit(JobOutput {
            job,
            body: JobResult::SearchDone {
                total: 0,
                count_limited: false,
                truncated: false,
                resume: Vec::new(),
                limit,
            },
        });
    };
    let Some(m) = p.query.matcher() else {
        empty_done(emit, None);
        return;
    };
    let len = p.snapshot.len_bytes();
    let whole = m.whole_doc();
    let mut batch: Vec<ByteRange> = Vec::new();
    let (mut total, mut kept) = (0u64, 0usize);
    let (mut count_limited, mut truncated) = (false, false);
    let mut last = clock.now();
    let mut cut = false;
    let mut sent_any = false;
    // 起点以降の最初の一致 (メインスレッドが同期検索をしない検索語だけ。保持の上限に依らず、見つけたら待たずに送る)。
    // k が 0 の重い検索語は v0.3 の順 (起点の行から末尾まで、続けて先頭から) で、起点以降の一致は最初の区間にだけある
    let mut first_after: Option<ByteRange> = None;
    let mut first_found = !m.sends_first_after();
    let mut on_match = |r: ByteRange| {
        total += 1;
        let mut flush = false;
        if !first_found && r.start >= p.from {
            first_found = true;
            first_after = Some(r);
            flush = true;
        }
        if kept < p.max_matches {
            kept += 1;
            batch.push(r);
            let now = clock.now();
            // 最初の一致は待たずに送る (NFR-108)
            if !sent_any || batch.len() >= BATCH_MATCHES || now.saturating_sub(last) >= BATCH_MILLIS
            {
                flush = true;
                last = now;
            }
        } else {
            truncated = true;
        }
        if flush {
            sent_any = true;
            emit(JobOutput {
                job,
                body: JobResult::SearchBatch {
                    matches: std::mem::take(&mut batch),
                    first_after: first_after.take(),
                },
            });
        }
        if total >= p.count_limit {
            count_limited = true;
            cut = true;
            return false;
        }
        true
    };
    // 再開の位置: 行の窓が核ごとに知らせる位置を 1 MiB ごとに間引く (試験で核を小さくしたときは核の大きさごと)
    let step = SEARCH_RESUME_STEP_BYTES.min(m.internal.core_bytes.max(1));
    let mut resume: Vec<ByteOff> = Vec::new();
    let mut on_resume = |r: ByteOff| {
        if resume
            .last()
            .is_none_or(|l| r.0 >= l.0.saturating_add(step))
        {
            resume.push(r);
        }
    };
    // 時計を渡す: 行の窓の経路 (内部の設定で開いたとき) は、窓の処理の時間から核を縮め、処理の速さの下限で打ち切る (03 §12.3.1)
    let end = if whole {
        let mut ctx = ScanCtx {
            cancel,
            clock: Some(clock),
            resume: Some(&mut on_resume),
        };
        scan_range(
            p.snapshot,
            &m,
            ByteRange::new(ByteOff(0), ByteOff(len)),
            &mut ctx,
            &mut on_match,
        )
    } else {
        let first = p
            .snapshot
            .line_to_byte(p.snapshot.byte_to_line(p.from))
            .0
            .min(len);
        let mut ctx = ScanCtx {
            cancel,
            clock: Some(clock),
            resume: None,
        };
        match scan_range(
            p.snapshot,
            &m,
            ByteRange::new(ByteOff(first), ByteOff(len)),
            &mut ctx,
            &mut on_match,
        ) {
            ScanEnd::Done => scan_range(
                p.snapshot,
                &m,
                ByteRange::new(ByteOff(0), ByteOff(first)),
                &mut ctx,
                &mut on_match,
            ),
            e => e,
        }
    };
    match end {
        ScanEnd::Done => {}
        ScanEnd::Stopped if cut => {}
        ScanEnd::TooLarge | ScanEnd::TooHeavy => {
            empty_done(emit, end.limit());
            return;
        }
        ScanEnd::Stopped | ScanEnd::Cancelled => {
            emit(JobOutput {
                job,
                body: JobResult::Failed {
                    err: JobError::Cancelled,
                },
            });
            return;
        }
    }
    if !batch.is_empty() || first_after.is_some() {
        emit(JobOutput {
            job,
            body: JobResult::SearchBatch {
                matches: batch,
                first_after,
            },
        });
    }
    emit(JobOutput {
        job,
        body: JobResult::SearchDone {
            total,
            count_limited,
            truncated,
            resume: if whole { resume } else { Vec::new() },
            limit: None,
        },
    });
}
