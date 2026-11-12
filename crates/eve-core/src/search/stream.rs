//! 行の窓 (`StreamScan`) と全文の連結 (docs/03 §12.3.1 の「実行」・「窓から窓への引き継ぎ」・「最悪の入力の上限」)。
//!
//! 走査の入口 (`scan_bytes`、03 §12.3 の「走査の境界」) の中の 3 つ目の経路。行をまたぎうる正規表現 (k ≥ 1) と、`\A`・`\z`・
//! `(?-m)` / `(?-R)` の `^` / `$` を含む正規表現 (k = 0 でも文書全体の意味が要る) と、k = 0 の重い正規表現を行の窓で探し、
//! k に上限の無い正規表現は、文書が 16 MiB 以下なら全文を 1 本に連結して探す (重いものは連結せずに打ち切る)。
//!
//! **窓の作り方**: 核 `[core_start, core_end)` は約 1 MiB (`SEARCH_WINDOW_CORE_BYTES`。重い検索語は 16 KiB から始める) を、
//! 最大の長さが有限で単語単位でない検索語では char 境界に、それ以外は行の境界に揃えたもの (範囲の始端と、直前の一致が核の
//! 終わりを越えたときは、行の途中から始まる)。延ばす量は「核の最後のバイトを含む行の後ろの k 行」と「最大の長さ」の小さいほう
//! で、窓 = 前の 1 文字 + 核 + 延ばす分 + 後ろの 1 文字を 1 本に複写する。核の中で始まる一致の文字列は、k 個より多くの行の
//! 終端を含まず最大の長さも越えないので、窓の終わりより手前で終わり、窓の外へ伸びる候補は (全文でも窓でも) 無い。`regex` の
//! `find_at` は始端より前の文字を `^`・`\b` の判定に使うので、核の中で始まる一致は全文で探したものと同じになる。後ろの 1 文字は
//! 判定には使われない (安全のための余り)。核の外で始まる一致は捨てる。
//!
//! **引き継ぎ**: 各核で探し始める位置は max(核の始端, 直前に採った一致の終端) で、次の核はそこから始める (直前の一致が核の
//! 終わりを越えたら、越えた位置から)。書記素の境界・単語単位で捨てた候補の次の探し直し (候補の始端の次の char 境界から) も、
//! 核をまたいで続ける。探し始める位置は正本の走査の状態そのものなので、核の切り方に依らず全文で選んだ結果と同じになり、
//! その位置を再開の位置として知らせられる (`ScanCtx::resume`)。
//!
//! **単語単位**: 一致の始端はそれを含む行の、終端はそれを含む行の単語境界 (03 §13.3) で調べる (終端が行頭なら境界)。行の内容は
//! 窓から取るので、単語単位では窓を核の始端を含む行の行頭から作り、延ばす量に最大の長さを使わない (終端を含む行の全体が窓に入る)。

use eve_text::{ByteOff, ByteRange, LineIdx, TextRead, is_grapheme_boundary, word_boundaries};
use memchr::{memchr2, memrchr2};

use crate::jobs::cancel::CancelToken;

use super::limits::{
    SEARCH_CANCEL_LATENCY_MS, SEARCH_HEAVY_CORE_BYTES, SEARCH_MIN_THROUGHPUT, SEARCH_SLOW_JUDGE_MS,
};
use super::matcher::{Matcher, WINDOW_SLACK};
use super::rewrite::LineSpan;
use super::scan::{Discard, OnMatch, ScanCtx, ScanEnd};

/// 行の窓と全文の連結の経路の入口 (`scan` から呼ぶ)。`range` の中で始まる一致を `f` へ渡す。
pub(super) fn scan_stream(
    text: &dyn TextRead,
    m: &Matcher,
    re: &regex::Regex,
    range: ByteRange,
    ctx: &mut ScanCtx<'_>,
    want_caps: bool,
    f: &mut OnMatch<'_>,
) -> ScanEnd {
    let len = text.len_bytes();
    let end = range.end.0.min(len);
    let start = range.start.0.min(end);
    // 始める前に分かる理由: 判定は文書の大きさで行う (範囲が空でも、上限なしの検索語は大きな文書で探さない)
    if m.too_large_for(len) {
        return ScanEnd::TooLarge;
    }
    if m.too_heavy() {
        return ScanEnd::TooHeavy;
    }
    if start >= end {
        return ScanEnd::Done;
    }
    let mut st = Stream {
        text,
        m,
        re,
        want_caps: want_caps && m.expands(),
        buf: String::new(),
        base: 0,
        words: Words::default(),
        discard: Discard::new(end - start, true),
    };
    match m.lines {
        LineSpan::Unbounded => st.whole(start, end, ctx, f),
        LineSpan::Bounded(k) => st.windows(k, start, end, ctx, f),
    }
}

struct Stream<'a> {
    text: &'a dyn TextRead,
    m: &'a Matcher,
    re: &'a regex::Regex,
    /// 一致ごとにキャプチャを渡す (置換の展開)。
    want_caps: bool,
    /// 窓の文字列 (核ごとに使い回す)。
    buf: String,
    /// `buf` の先頭の文書内の位置。
    base: usize,
    words: Words,
    discard: Discard,
}

impl Stream<'_> {
    /// 行の窓 (k が有限)。核を順に作り、核の中で始まる一致を探す。
    fn windows(
        &mut self,
        k: usize,
        start: usize,
        end: usize,
        ctx: &mut ScanCtx<'_>,
        f: &mut OnMatch<'_>,
    ) -> ScanEnd {
        let text = self.text;
        let internal = self.m.internal;
        // 最大の長さが有限で単語単位でない検索語は、核を行の途中 (char 境界) で切る。窓は「核 + 最大の長さ」以下になり、行の
        // 長さに依らない。それ以外は核を行の境界に揃える (行の途中で切ると、延ばす分の行を核ごとに読み直して O(n²) になる)
        let mid = self.m.cuts_mid_line();
        let cut = |x: usize| {
            if mid {
                ceil_char(text, x)
            } else {
                line_ceil(text, x)
            }
        };
        let mut max_core = internal.core_bytes.max(1);
        if mid && k > 0 {
            // 窓が上限に収まるように核を抑える (収まらない検索語は、始める前に `too_large_for` で断っている)
            let n = self.m.max_len.unwrap_or(0);
            max_core = max_core.min(
                internal
                    .concat_max
                    .saturating_sub(n.saturating_add(WINDOW_SLACK))
                    .max(1),
            );
        }
        // 縮める下限: 核を行の途中で切る検索語では最大の長さの 1/4 (窓は「核 + 延ばす分 (最大の長さまで)」なので、核をこれより
        // 小さくしても 1 回の呼び出しはほとんど短くならず、延ばす分の読み直しが増えて処理の速さだけが落ちる。1/4 なら、窓は
        // 延ばす分だけの窓の 1.25 倍まで、処理の速さの低下は 1/5 まで)。行の境界に揃える検索語では 1 行 (1 バイトを行の境界へ
        // 切り上げる)
        let min_core = if mid {
            (self.m.max_len.unwrap_or(4) / 4).clamp(1, max_core)
        } else {
            1
        };
        // 核の大きさの目安 (取り消しの遅れから縮め、速ければ戻す)。重い検索語は小さい核から始める
        let mut target = if self.m.is_heavy() {
            max_core.min(SEARCH_HEAVY_CORE_BYTES.max(min_core))
        } else {
            max_core
        };
        // 処理の速さの下限: 判定の間隔の間に進んだ核の量と時間
        let (mut slow_bytes, mut slow_ms) = (0u64, 0u64);
        let mut pos = start;
        while pos < end {
            if ctx.cancel.is_cancelled() {
                return ScanEnd::Cancelled;
            }
            let core_start = pos;
            let mut core_end = cut(core_start.saturating_add(target)).min(end);
            let mut ext_end = self.extend(core_end, k);
            let hs = self.head(core_start);
            // 延ばす分の複写を核より大きくしない (複写の合計は文書の約 2 倍まで)。縮めている間と、大きくすると窓が上限を
            // 超える (長い行に揃う) ときは大きくしない
            if target == max_core && ext_end - core_end > core_end - core_start {
                let grown = cut(core_start.saturating_add(ext_end - core_end)).min(end);
                let grown_ext = self.extend(grown, k);
                if next_char(text, grown_ext) - hs <= internal.concat_max {
                    (core_end, ext_end) = (grown, grown_ext);
                }
            }
            let he = next_char(text, ext_end);
            // 窓が上限を超える検索語は、上限なしと同じに扱う (窓は文書の一部なので、文書は上限を超えていて探さない)。
            // 核を行の境界に揃える検索語 (最大の長さが無い・単語単位) だけが、ここで初めて分かる
            if k > 0 && he - hs > internal.concat_max {
                return ScanEnd::TooLarge;
            }
            if let Some(r) = ctx.resume.as_mut() {
                (*r)(ByteOff(pos));
            }
            let t0 = ctx.clock.map(|c| c.now());
            self.fill(hs, he);
            let next = match self.search_core(pos, core_end, ctx.cancel, f, &mut |_| None) {
                Ok(n) => n,
                Err(e) => return e,
            };
            pos = next.max(core_end);
            let (Some(clock), Some(t0)) = (ctx.clock, t0) else {
                continue;
            };
            // 取り消しの遅れ: 1 つの窓の処理が目安を超えたら、かかった時間に比べて核を縮める (少なくとも半分に。行の境界へ
            // 切り上げても縮むように。最小は `min_core`)。目安の 1/4 より速ければ、元の大きさまで倍にして戻す
            let ms = clock.now().saturating_sub(t0).0;
            let core_len = core_end - core_start;
            if ms > SEARCH_CANCEL_LATENCY_MS {
                let scaled =
                    core_len as u128 * u128::from(SEARCH_CANCEL_LATENCY_MS) / u128::from(ms);
                target = usize::try_from(scaled)
                    .unwrap_or(usize::MAX)
                    .min(core_len / 2)
                    .max(min_core);
            } else if ms.saturating_mul(4) < SEARCH_CANCEL_LATENCY_MS {
                target = target.saturating_mul(2).min(max_core);
            }
            // 処理の速さの下限: 核の大きさに依らず、判定の間隔 (1 秒) ごとに、その間に進んだ核の量 (延ばす分の読み直しは
            // 数えない) と時間で見る
            slow_bytes = slow_bytes.saturating_add(core_len as u64);
            slow_ms = slow_ms.saturating_add(ms);
            if slow_ms >= SEARCH_SLOW_JUDGE_MS {
                if too_slow(slow_bytes, slow_ms) {
                    return ScanEnd::TooHeavy;
                }
                (slow_bytes, slow_ms) = (0, 0);
            }
        }
        // 最後の窓の `regex` の呼び出しの間に取り消されたら、取り消しとして返す
        if ctx.cancel.is_cancelled() {
            return ScanEnd::Cancelled;
        }
        ScanEnd::Done
    }

    /// 全文の連結 (k に上限が無い)。範囲の始端の前の 1 文字から文書の末尾までを 1 本に複写し、範囲の中で始まる一致を探す。
    /// 再開の位置は、核の大きさの間隔ごとに知らせる。
    fn whole(
        &mut self,
        start: usize,
        end: usize,
        ctx: &mut ScanCtx<'_>,
        f: &mut OnMatch<'_>,
    ) -> ScanEnd {
        let len = self.text.len_bytes();
        let step = self.m.internal.core_bytes.max(1);
        let cancel = ctx.cancel;
        let clock = ctx.clock;
        let resume = &mut ctx.resume;
        if let Some(r) = resume.as_mut() {
            (*r)(ByteOff(start));
        }
        let hs = self.head(start);
        let t0 = clock.map(|c| c.now());
        self.fill(hs, len);
        let mut last = start;
        let mut tick = |p: usize| {
            if p < end && p >= last.saturating_add(step) {
                last = p;
                if let Some(r) = resume.as_mut() {
                    (*r)(ByteOff(p));
                }
            }
            // 処理の速さの下限 (連結は核を縮められないので、始めからの量と時間で見る)
            if let (Some(c), Some(t0)) = (clock, t0)
                && too_slow(p.saturating_sub(start) as u64, c.now().saturating_sub(t0).0)
            {
                return Some(ScanEnd::TooHeavy);
            }
            None
        };
        match self.search_core(start, end, cancel, f, &mut tick) {
            // 連結の 1 回の `regex` の呼び出しの間は取り消せない (§12.3.1 の「取り消しの遅れ」の例外)。終わった後に確かめる
            Ok(_) if cancel.is_cancelled() => ScanEnd::Cancelled,
            Ok(_) => ScanEnd::Done,
            Err(e) => e,
        }
    }

    /// 窓 (`buf`) の上で、`[pos, core_end)` の中で始まる一致を選び、`f` へ渡す。返すのは次に探し始める位置 (文書内。核の中で
    /// もう始まらないときは `core_end` 以下のこともある)。`tick` は `regex` の呼び出しごとに探し始める位置を受け取り、
    /// 打ち切るときは理由を返す。
    fn search_core(
        &mut self,
        pos: usize,
        core_end: usize,
        cancel: &CancelToken,
        f: &mut OnMatch<'_>,
        tick: &mut dyn FnMut(usize) -> Option<ScanEnd>,
    ) -> Result<usize, ScanEnd> {
        let core_hi = core_end - self.base;
        let mut p = pos - self.base;
        while p < core_hi {
            // 取り消しは `regex` の呼び出しごとに確かめる (1 回の呼び出しは途中で止められない)
            if cancel.is_cancelled() {
                return Err(ScanEnd::Cancelled);
            }
            let Some((s, e, caps)) = find(self.re, &self.buf, p, self.want_caps) else {
                break;
            };
            if s >= core_hi {
                break;
            }
            if s == e {
                // 空の一致は採らず、1 文字進める
                p = e + char_len_at(&self.buf, e);
            } else {
                let (so, eo) = (self.base + s, self.base + e);
                if is_grapheme_boundary(self.text, ByteOff(so))
                    && is_grapheme_boundary(self.text, ByteOff(eo))
                    && self.words.ok(self.m.whole_word, self.base, &self.buf, s, e)
                {
                    if !f(ByteRange::new(ByteOff(so), ByteOff(eo)), caps.as_ref()) {
                        return Err(ScanEnd::Stopped);
                    }
                    p = e;
                } else {
                    // 捨てた候補のために読んだ量 (探し始めた位置から候補の終端まで) を予算に数える
                    if !self.discard.spend(e - p) {
                        return Err(ScanEnd::TooHeavy);
                    }
                    // 採らなかった候補と重なる位置の一致を取りこぼさないよう、先頭の次の char 境界から探し直す
                    p = s + char_len_at(&self.buf, s);
                }
            }
            if let Some(end) = tick(self.base + p) {
                return Err(end);
            }
        }
        Ok(self.base + p)
    }

    /// 核の終わり `core_end` に対する、延ばす分の終わり (文書内)。「核の最後のバイトを含む行の後ろの k 行」の終わり (行頭) と、
    /// 「核の終わり + 最大の長さ」(単語単位では使わない) の小さいほう。
    fn extend(&self, core_end: usize, k: usize) -> usize {
        let text = self.text;
        let last = text.byte_to_line(ByteOff(core_end.saturating_sub(1))).0;
        let mut e = text
            .line_to_byte(LineIdx(last.saturating_add(k).saturating_add(1)))
            .0;
        if !self.m.whole_word
            && let Some(n) = self.m.max_len
        {
            e = e.min(ceil_char(text, core_end.saturating_add(n)));
        }
        e.max(core_end)
    }

    /// 窓の始端: 核の始端の前の 1 文字 (単語単位は、核の始端を含む行の行頭の前の 1 文字)。
    fn head(&self, core_start: usize) -> usize {
        let at = if self.m.whole_word {
            self.text
                .line_to_byte(self.text.byte_to_line(ByteOff(core_start)))
                .0
        } else {
            core_start
        };
        prev_char(self.text, at)
    }

    /// 窓 `[hs, he)` を `buf` に複写する (大きさは先に確保する)。
    fn fill(&mut self, hs: usize, he: usize) {
        self.buf.clear();
        self.buf.reserve(he - hs);
        for c in self.text.chunks(ByteRange::new(ByteOff(hs), ByteOff(he))) {
            self.buf.push_str(c);
        }
        self.base = hs;
    }
}

/// 処理の速さが下限を下回り続けた (見る時間に届き、速さが下限より遅い)。
fn too_slow(bytes: u64, ms: u64) -> bool {
    ms >= SEARCH_SLOW_JUDGE_MS
        && bytes.saturating_mul(1000) < SEARCH_MIN_THROUGHPUT.saturating_mul(ms)
}

/// 一致 (始端, 終端, キャプチャ)。
type Found<'h> = (usize, usize, Option<regex::Captures<'h>>);

fn find<'h>(re: &regex::Regex, hay: &'h str, at: usize, caps: bool) -> Option<Found<'h>> {
    if caps {
        let c = re.captures_at(hay, at)?;
        let m = c.get(0)?;
        Some((m.start(), m.end(), Some(c)))
    } else {
        let m = re.find_at(hay, at)?;
        Some((m.start(), m.end(), None))
    }
}

/// `s[i..]` の最初の char のバイト長 (末尾では 1)。
fn char_len_at(s: &str, i: usize) -> usize {
    s.get(i..)
        .and_then(|t| t.chars().next())
        .map_or(1, char::len_utf8)
}

/// `x` 以上で最初の行頭 (`x` が行頭ならそのまま。文書の末尾を越えたら末尾)。
fn line_ceil(text: &dyn TextRead, x: usize) -> usize {
    if x >= text.len_bytes() {
        return text.len_bytes();
    }
    let l = text.byte_to_line(ByteOff(x));
    let ls = text.line_to_byte(l).0;
    if ls == x {
        x
    } else {
        text.line_to_byte(LineIdx(l.0 + 1)).0
    }
}

/// `x` 以上で最初の char 境界 (末尾を越えたら末尾)。
fn ceil_char(text: &dyn TextRead, x: usize) -> usize {
    let len = text.len_bytes();
    let mut b = x.min(len);
    while b < len && !text.is_char_boundary(ByteOff(b)) {
        b += 1;
    }
    b
}

/// `x` (char 境界) の次の char の終わり (末尾ではそのまま)。
fn next_char(text: &dyn TextRead, x: usize) -> usize {
    let len = text.len_bytes();
    if x >= len {
        return len;
    }
    ceil_char(text, x + 1)
}

/// `x` (char 境界) の前の char の始まり (先頭ではそのまま)。
fn prev_char(text: &dyn TextRead, x: usize) -> usize {
    if x == 0 {
        return 0;
    }
    let mut b = x - 1;
    while b > 0 && !text.is_char_boundary(ByteOff(b)) {
        b -= 1;
    }
    b
}

/// 単語単位の両端の検査 (行の単語境界を最近の 2 行だけ覚える。長い行で候補ごとに境界を求め直さない)。
#[derive(Default)]
struct Words {
    /// (行頭の文書内の位置, その行の単語境界 (行頭からのバイト))。
    cache: [Option<(usize, Vec<usize>)>; 2],
    next: usize,
}

impl Words {
    /// 窓 `buf` (先頭の文書内の位置が `base`) の上の一致 `[s, e)` の両端が、それぞれの行の単語境界にあるか。`on` が偽なら真。
    fn ok(&mut self, on: bool, base: usize, buf: &str, s: usize, e: usize) -> bool {
        if !on {
            return true;
        }
        let b = buf.as_bytes();
        let (ls, le) = line_at(b, s);
        if !self.has(base + ls, &buf[ls..le], s - ls) {
            return false;
        }
        // 終端が行頭なら境界
        if at_line_start(b, e) {
            return true;
        }
        let (ls, le) = line_at(b, e);
        self.has(base + ls, &buf[ls..le], e - ls)
    }

    fn has(&mut self, key: usize, line: &str, rel: usize) -> bool {
        let i = match self
            .cache
            .iter()
            .position(|c| c.as_ref().is_some_and(|(k, _)| *k == key))
        {
            Some(i) => i,
            None => {
                let i = self.next;
                self.next = (self.next + 1) % self.cache.len();
                self.cache[i] = Some((key, word_boundaries(line)));
                i
            }
        };
        self.cache[i]
            .as_ref()
            .is_some_and(|(_, w)| w.binary_search(&rel).is_ok())
    }
}

/// `b` の位置 `x` を含む行の内容 `[行頭, 行の内容の末尾)` (窓の中の位置。行の終端の手前まで)。
fn line_at(b: &[u8], x: usize) -> (usize, usize) {
    let ls = memrchr2(b'\n', b'\r', &b[..x]).map_or(0, |i| i + 1);
    let le = memchr2(b'\n', b'\r', &b[x..]).map_or(b.len(), |i| x + i);
    (ls, le)
}

/// 位置 `e` (> 0) が行頭か (直前が LF、または後ろが LF でない CR)。
fn at_line_start(b: &[u8], e: usize) -> bool {
    match b[e - 1] {
        b'\n' => true,
        b'\r' => b.get(e) != Some(&b'\n'),
        _ => false,
    }
}
