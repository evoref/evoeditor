//! リテラル検索の基盤 (docs/03 §12、01 FR-401・FR-402・FR-404)。
//!
//! 検索語は 1 行の入力欄から来るので改行を含まない。一致は行をまたがないため、**行ごと**に走査する
//! (チャンクの境界を気にしなくてよい)。一致は重ならないものを左から順に取り、両端が書記素境界でない候補は捨てる (§12.1)。
//!
//! 大小を区別しない照合は、1 文字ずつ小文字へ畳み込んだ文字列どうしで行う。畳み込めるのは小文字が 1 文字になる
//! 文字だけで、それ以外 (`İ` など) はそのままにする。これは Unicode の単純ケースフォールディングとほぼ同じだが、
//! `ſ` (U+017F) と `s` は同一視しない (§12.4 の実装メモ)。

use std::borrow::Cow;
use std::ops::Range;

use eve_text::{ByteOff, ByteRange, LineIdx, TextRead, is_grapheme_boundary};
use eve_view::JobId;
use memchr::memmem;

use crate::cancel::CancelToken;
use crate::jobs::{JobError, JobResult};
use crate::time::{Clock, Millis};

/// 全件走査の結果を送る間隔 (件数または時間、§12.2)。
const BATCH_MATCHES: usize = 256;
const BATCH_MILLIS: Millis = Millis(20);

/// 大文字小文字の扱い (FR-402)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Case {
    /// 検索語に大文字を含めば区別し、含まなければ区別しない。
    Smart,
    Sensitive,
    Insensitive,
}

impl Case {
    /// `search.toggle_case` の巡回: Smart → Sensitive → Insensitive → Smart。
    pub fn next(self) -> Case {
        match self {
            Case::Smart => Case::Sensitive,
            Case::Sensitive => Case::Insensitive,
            Case::Insensitive => Case::Smart,
        }
    }
}

/// 検索の問い合わせ。構築は `SearchQuery::literal` 経由 (v0.2 以降に項目を足しても呼び出し側を壊さない、03 §12.1)。
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SearchQuery {
    pattern: String,
    case: Case,
}

impl SearchQuery {
    /// リテラル検索 (大小は `Smart`)。
    pub fn literal(pattern: &str) -> SearchQuery {
        SearchQuery {
            pattern: pattern.to_owned(),
            case: Case::Smart,
        }
    }

    pub fn with_case(mut self, case: Case) -> SearchQuery {
        self.case = case;
        self
    }

    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    pub fn case(&self) -> Case {
        self.case
    }

    /// 大小を実際に区別して探すか (検索バーのトグルを反転して示す)。
    pub fn is_case_sensitive(&self) -> bool {
        match self.case {
            Case::Sensitive => true,
            Case::Insensitive => false,
            Case::Smart => self.pattern.chars().any(|c| fold_char(c) != c),
        }
    }

    /// 照合器。検索語が空なら `None` (何にも一致しない)。
    pub fn matcher(&self) -> Option<Matcher> {
        // 検索語は 1 行の入力欄から来る。改行を含むものは行をまたぐので一致させない (v0.2 の複数行は別途)
        if self.pattern.is_empty() || self.pattern.contains(['\n', '\r']) {
            return None;
        }
        // 大小の区別がある文字 (小文字・大文字) を 1 つも含まない検索語 (数字・記号・かな・漢字だけ等) は、
        // 区別しない設定でも区別する設定と同じなので、そのまま memmem で探す (§12.4 の 1)
        let cased = self
            .pattern
            .chars()
            .any(|c| c.is_lowercase() || c.is_uppercase());
        let fold = cased
            && match self.case {
                Case::Sensitive => false,
                Case::Insensitive => true,
                Case::Smart => !self.pattern.chars().any(|c| fold_char(c) != c),
            };
        let needle = if fold {
            self.pattern.chars().map(fold_char).collect::<String>()
        } else {
            self.pattern.clone()
        };
        // ASCII だけで `k` を含まない検索語は、チャンクごとに ASCII を畳み込んで memmem できる (§12.4 の 2)。
        // `k` は K (U+212A KELVIN SIGN) が畳み込まれて一致するので、行ごとの経路にする
        let chunked = !fold || (needle.is_ascii() && !needle.contains('k'));
        Some(Matcher {
            finder: memmem::Finder::new(needle.as_bytes()).into_owned(),
            len: needle.len(),
            fold,
            chunked,
        })
    }
}

/// 小文字が 1 文字になる文字だけ畳み込む。
fn fold_char(c: char) -> char {
    let mut it = c.to_lowercase();
    match (it.next(), it.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

/// 畳み込んだ文字列と、畳み込み後のバイト位置 → 元のバイト位置の表 (長さが変わらなければ `None`)。
fn fold_line(line: &str) -> (Cow<'_, str>, Option<Vec<usize>>) {
    if line.is_ascii() {
        return if line.bytes().any(|b| b.is_ascii_uppercase()) {
            (Cow::Owned(line.to_ascii_lowercase()), None)
        } else {
            (Cow::Borrowed(line), None)
        };
    }
    let mut out = String::with_capacity(line.len());
    let mut map: Vec<usize> = Vec::with_capacity(line.len() + 1);
    let mut same = true;
    for (i, c) in line.char_indices() {
        let f = fold_char(c);
        same &= f.len_utf8() == c.len_utf8();
        for _ in 0..f.len_utf8() {
            map.push(i);
        }
        out.push(f);
    }
    map.push(line.len());
    (Cow::Owned(out), (!same).then_some(map))
}

/// コンパイル済みの検索語。
pub struct Matcher {
    finder: memmem::Finder<'static>,
    /// 畳み込み後の検索語のバイト長。
    len: usize,
    fold: bool,
    /// チャンクごとに走査できる (行ごとの畳み込みが要らない)。
    chunked: bool,
}

impl Matcher {
    /// チャンクごとに (memmem で) 走査できる。`false` は行ごとに畳み込む遅い経路 (約 50 MB/s) になる。
    pub fn is_chunked(&self) -> bool {
        self.chunked
    }

    /// 1 行の中の一致を左から順に `f` へ渡す。`line_start` は行頭の文書内バイト位置。
    fn find_in_line(
        &self,
        text: &dyn TextRead,
        line_start: usize,
        line: &str,
        f: &mut dyn FnMut(ByteRange) -> bool,
    ) -> bool {
        let (hay, map) = if self.fold {
            fold_line(line)
        } else {
            (Cow::Borrowed(line), None)
        };
        let hay = hay.as_bytes();
        let orig = |i: usize| map.as_ref().map_or(i, |m| m[i]);
        let mut pos = 0;
        while pos <= hay.len() {
            let Some(i) = self.finder.find(&hay[pos..]) else {
                break;
            };
            let (s, e) = (pos + i, pos + i + self.len);
            let (so, eo) = (line_start + orig(s), line_start + orig(e));
            if is_grapheme_boundary(text, ByteOff(so)) && is_grapheme_boundary(text, ByteOff(eo)) {
                if !f(ByteRange::new(ByteOff(so), ByteOff(eo))) {
                    return false;
                }
                pos = e;
            } else {
                // 書記素の途中に当たった候補は捨て、その先頭の次の char 境界から探し直す
                pos = s + char_len(&hay[s..]);
            }
        }
        true
    }
}

fn char_len(b: &[u8]) -> usize {
    match b.first() {
        Some(&x) if x >= 0xF0 => 4,
        Some(&x) if x >= 0xE0 => 3,
        Some(&x) if x >= 0xC0 => 2,
        _ => 1,
    }
}

/// 行 `lines` (半開区間) を順に走査し、一致を `f` へ渡す。`f` が `false` を返すか取り消されたら止めて `false`。
pub fn scan_lines(
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
        if !m.find_in_line(text, ls, &line, f) {
            return false;
        }
    }
    true
}

/// `range` (行頭と行末に揃った範囲) を走査し、一致を `f` へ渡す。チャンクごとに走査できる検索語は、行に切り出さずに
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

/// すべて置換の ChangeSet を作る (一致を保持せず、走査しながら直接組み立てる、03 §12.5)。
/// 一致の選び方は検索と同じ。返すのは (ChangeSet, 置換した件数)。取り消されたら `None`。
pub fn build_replace_all(
    text: &dyn TextRead,
    query: &SearchQuery,
    replacement: &str,
    cancel: &CancelToken,
) -> Option<(eve_text::ChangeSet, u64)> {
    let m = query.matcher()?;
    let insert: std::sync::Arc<str> = std::sync::Arc::from(replacement);
    let mut edits: Vec<eve_text::Edit> = Vec::new();
    let full = ByteRange::new(ByteOff(0), ByteOff(text.len_bytes()));
    if !scan_bytes(text, &m, full, cancel, &mut |r| {
        edits.push(eve_text::Edit {
            range: r,
            insert: insert.clone(),
        });
        true
    }) {
        return None;
    }
    let n = edits.len() as u64;
    let cs = eve_text::ChangeSet::from_sorted_edits(text, edits).ok()?;
    Some((cs, n))
}

/// 全件走査のジョブ本体 (§12.2)。`from` のある行から文書の末尾まで、続けて先頭からその行の手前までを走査し、
/// 256 件または 20 ms ごとに `SearchBatch` を送る。保持する一致は `max_matches` 件まで、数えるのは `count_limit` 件まで。
pub struct SearchJob<'a> {
    pub job: JobId,
    pub snapshot: &'a dyn TextRead,
    pub query: &'a SearchQuery,
    pub from: ByteOff,
    pub max_matches: usize,
    pub count_limit: u64,
}

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

#[cfg(test)]
mod tests;
