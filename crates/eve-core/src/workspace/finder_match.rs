//! ファインダの照合 (docs/03 §12.7.4 の「照らす対象」「採点」「件数」「照合のジョブ」)。eve-core の純粋関数で、`bg` の
//! ジョブ `JobKind::FinderMatch` が呼ぶ (照合はメインスレッドで行わない、01 FR-125 AC-2)。
//!
//! - **照らす対象**: 候補のルートからの相対パス (区切りは `/`)。検索語は前後の空白を除き、空白で区切った語の **すべて** が
//!   部分列として現れる候補だけを残す (AND)。大文字小文字は ASCII の英字だけを畳む。Windows では検索語の `\` を `/` にする。
//! - **採点** (S-26 の試作の fzf v1 の形): 語ごとに、前から当てて終端を決め、後ろから当て直して最短の窓にし、窓の中を前から
//!   採点する。値は [`MATCH_SCORE`] などの定数。
//! - **並び**: ファイル名 (最後の `/` の後ろ) の中に収まった語の数の多い順 → 点の高い順 → 相対パスの短い順 → バイト列の順。
//!   ファイル名の中の語の数を点より先に比べるのは、性質 (a) (「ファイル名の中だけで一致する候補は、同じ語でディレクトリに
//!   またがって一致する候補より上」) を、間の減点の大きさに依らずに保つため (03 §12.7.4 の段 6 の細目)。
//! - **件数**: 上位 `top` 件 (既定 [`FINDER_TOP`]) だけを持ち、一致した件数はすべて数える。検索語が空なら相対パスの短い順。
//!
//! 文字の単位: 検索語の 1 文字 (UTF-8 の 1 文字) は、候補の中の 1 文字にだけ当たる (ASCII の外の文字はバイト列の全体で比べ、
//! 先頭のバイトは文字の境目にしか現れないので、別の文字の途中には当たらない)。一致の位置は、相対パスの書記素の番号の半開区間で
//! 返す (`FinderHit::ranges`)。
//!
//! spec: 03 §12.7.4, 03 §12.7.7, 01 FR-125

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::ops::Range;
use std::sync::Arc;

use super::{Candidate, char_bit};
use crate::jobs::cancel::CancelToken;

/// 一覧に出す上位の件数 (03 §12.7.4 の `FINDER_TOP`)。表示の窓は 12 行 (04 §8.11)。
pub const FINDER_TOP: usize = 100;
/// 取消しを見る間隔 (候補の件数)。
pub const FINDER_CANCEL_EVERY: usize = 4096;

/// 一致した文字 1 つの点。
pub const MATCH_SCORE: i32 = 16;
/// 区切り (`/`・`_`・`-`・`.`・空白) の直後と、相対パスの先頭の加点。
pub const BONUS_BOUNDARY: i32 = 8;
/// camelCase の境目・英字と数字の境目の加点。
pub const BONUS_CAMEL: i32 = 7;
/// 直前の文字と連続した一致の加点。
pub const BONUS_CONSECUTIVE: i32 = 4;
/// 語の最初の文字の境目の加点の倍率。
pub const FIRST_CHAR_MULTIPLIER: i32 = 2;
/// 間の最初の 1 文字の減点。
pub const PENALTY_GAP_START: i32 = 3;
/// 間の 2 文字目からの 1 文字ごとの減点。
pub const PENALTY_GAP_EXTENSION: i32 = 1;
/// 語の一致がファイル名の中に収まったときの、語の 1 文字あたりの加点。
pub const BONUS_FILENAME_PER_CHAR: i32 = 8;

/// 照合で一致した候補 1 つ (`JobResult::FinderMatched` の `top` の要素、02 §9.3)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinderHit {
    /// 照合に渡した塊の列の中の番号。
    pub chunk: u32,
    /// 塊の中の番号。
    pub index: u32,
    /// 点 (語の点の合計)。
    pub score: i32,
    /// 一致した文字の位置 (相対パスの書記素の番号の半開区間。昇順で重ならない)。
    pub ranges: Vec<Range<u32>>,
}

/// 照合の結果 (一致した件数と上位)。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinderMatches {
    /// 一致した件数 (上位に入らなかったものを含む)。
    pub matched: usize,
    /// 上位 (並びの順)。
    pub top: Vec<FinderHit>,
}

/// 検索語の語 1 つ (ASCII を小文字にした UTF-8 のバイト列と、各文字の開始位置)。
#[derive(Clone, Debug)]
struct Word {
    bytes: Vec<u8>,
    /// 各文字の開始位置 (末尾に `bytes.len()` を足した列)。
    starts: Vec<usize>,
}

impl Word {
    fn new(s: &str) -> Word {
        let bytes: Vec<u8> = s.bytes().map(|b| b.to_ascii_lowercase()).collect();
        let mut starts: Vec<usize> = s.char_indices().map(|(i, _)| i).collect();
        starts.push(bytes.len());
        Word { bytes, starts }
    }

    fn chars(&self) -> usize {
        self.starts.len() - 1
    }

    fn unit(&self, k: usize) -> &[u8] {
        &self.bytes[self.starts[k]..self.starts[k + 1]]
    }
}

/// 解釈した検索語 (空白で区切った語の並び)。
#[derive(Clone, Debug)]
pub struct FinderQuery {
    words: Vec<Word>,
    /// 全部の語の文字の集合 ([`char_bit`] の和)。候補の集合に無いビットがあれば一致しない。
    bits: u64,
}

impl FinderQuery {
    /// 検索語を解釈する: 前後の空白を除き、空白で区切る。Windows では `\` を `/` として照らす。
    #[must_use]
    pub fn parse(query: &str) -> FinderQuery {
        let query = if cfg!(windows) {
            std::borrow::Cow::Owned(query.replace('\\', "/"))
        } else {
            std::borrow::Cow::Borrowed(query)
        };
        let words: Vec<Word> = query.split_whitespace().map(Word::new).collect();
        let bits = words
            .iter()
            .flat_map(|w| w.bytes.iter())
            .fold(0u64, |acc, &b| acc | char_bit(b));
        FinderQuery { words, bits }
    }

    /// 語が無い (空・空白だけの検索語)。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }
}

/// 1 つの候補の採点の結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Score {
    /// ファイル名の中に収まった語の数 (並びで点より先に比べる)。
    in_name: u32,
    /// 点。
    score: i32,
}

/// 文字の種類 (境目の加点に使う)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Separator,
    Lower,
    Upper,
    Digit,
    /// ASCII の外の文字 (英字と同じく、数字との境目に数える)。
    Other,
    /// ASCII の記号 (区切りを除く)。
    Symbol,
}

fn class_of(b: u8) -> Class {
    match b {
        b'/' | b'_' | b'-' | b'.' | b' ' => Class::Separator,
        b'a'..=b'z' => Class::Lower,
        b'A'..=b'Z' => Class::Upper,
        b'0'..=b'9' => Class::Digit,
        0x80..=0xFF => Class::Other,
        _ => Class::Symbol,
    }
}

/// `rel` のバイト位置 `p` (文字の先頭) の文字の、境目の加点。
fn boundary_bonus(rel: &[u8], p: usize) -> i32 {
    if p == 0 {
        return BONUS_BOUNDARY;
    }
    // 直前の文字の先頭 (UTF-8 の続きのバイトを飛ばす)
    let mut q = p - 1;
    while q > 0 && rel[q] & 0xC0 == 0x80 {
        q -= 1;
    }
    let (prev, cur) = (class_of(rel[q]), class_of(rel[p]));
    let letter = |c: Class| matches!(c, Class::Lower | Class::Upper | Class::Other);
    match (prev, cur) {
        (Class::Separator, _) => BONUS_BOUNDARY,
        (Class::Lower, Class::Upper) => BONUS_CAMEL,
        (p, Class::Digit) if letter(p) => BONUS_CAMEL,
        (Class::Digit, c) if letter(c) => BONUS_CAMEL,
        _ => 0,
    }
}

/// `hay` の `at` から `unit` が始まるか。
fn unit_at(hay: &[u8], at: usize, unit: &[u8]) -> bool {
    hay.get(at..at + unit.len()) == Some(unit)
}

/// `hay[from..]` で `unit` が最初に始まる位置。
fn find_unit(hay: &[u8], from: usize, unit: &[u8]) -> Option<usize> {
    let first = unit[0];
    let mut p = from;
    while p < hay.len() {
        let i = p + hay[p..].iter().position(|&b| b == first)?;
        if unit_at(hay, i, unit) {
            return Some(i);
        }
        p = i + 1;
    }
    None
}

/// `hay[..end]` の中で `unit` が最後に始まる位置 (`unit` は `end` までに収まる)。
fn rfind_unit(hay: &[u8], end: usize, unit: &[u8]) -> Option<usize> {
    let last_start = end.checked_sub(unit.len())?;
    (0..=last_start).rev().find(|&i| unit_at(hay, i, unit))
}

/// 語 1 つを候補に当てる: 前から当てて終端を決め、後ろから当て直して最短の窓にし、窓の中を前から当てた各文字の位置を
/// `pos` に入れる (先に `pos` を空にする)。当たらなければ `None`、当たれば窓の開始位置。
fn match_word(lower: &[u8], w: &Word, pos: &mut Vec<usize>) -> Option<usize> {
    pos.clear();
    let n = w.chars();
    // 1. 前から: 終端を決める
    let mut at = 0;
    let mut end = 0;
    for k in 0..n {
        let u = w.unit(k);
        let i = find_unit(lower, at, u)?;
        at = i + u.len();
        end = at;
    }
    // 2. 後ろから: 最短の窓の開始を決める
    let mut limit = end;
    for k in (0..n).rev() {
        limit = rfind_unit(lower, limit, w.unit(k))?;
    }
    let start = limit;
    // 3. 窓の中を前から当てる (採点と強調の位置)
    let mut at = start;
    for k in 0..n {
        let u = w.unit(k);
        let i = find_unit(lower, at, u)?;
        pos.push(i);
        at = i + u.len();
    }
    Some(start)
}

/// `rel[a..b]` の文字の数 (UTF-8 の続きのバイトを数えない)。
fn chars_between(rel: &[u8], a: usize, b: usize) -> i32 {
    let n = rel[a..b].iter().filter(|&&x| x & 0xC0 != 0x80).count();
    i32::try_from(n).unwrap_or(i32::MAX)
}

/// 当てた位置 `pos` (語 `w` の各文字) の点。
fn word_score(rel: &[u8], w: &Word, pos: &[usize]) -> i32 {
    let mut score = 0i32;
    let mut prev_end: Option<usize> = None;
    for (k, &p) in pos.iter().enumerate() {
        let mut bonus = boundary_bonus(rel, p);
        if k == 0 {
            bonus *= FIRST_CHAR_MULTIPLIER;
        }
        let mut s = MATCH_SCORE + bonus;
        if let Some(e) = prev_end {
            let gap = chars_between(rel, e, p);
            if gap == 0 {
                s += BONUS_CONSECUTIVE;
            } else {
                s -= PENALTY_GAP_START + (gap - 1) * PENALTY_GAP_EXTENSION;
            }
        }
        score = score.saturating_add(s);
        prev_end = Some(p + w.unit(k).len());
    }
    score
}

/// 候補 1 つを採点する。一致しなければ `None`。`positions` には、語ごとの位置を順に足す (呼び側が強調に使うとき)。
fn score_candidate(
    c: &Candidate,
    q: &FinderQuery,
    scratch: &mut Vec<usize>,
    mut positions: Option<&mut Vec<(usize, usize)>>,
) -> Option<Score> {
    if c.chars() & q.bits != q.bits {
        return None;
    }
    let (lower, rel) = (c.lower(), c.rel().as_bytes());
    let name_start = c.name_start();
    let mut total = Score {
        in_name: 0,
        score: 0,
    };
    for w in &q.words {
        let start = match_word(lower, w, scratch)?;
        total.score = total.score.saturating_add(word_score(rel, w, scratch));
        if start >= name_start {
            total.in_name += 1;
            let chars = i32::try_from(w.chars()).unwrap_or(i32::MAX);
            total.score = total
                .score
                .saturating_add(chars.saturating_mul(BONUS_FILENAME_PER_CHAR));
        }
        if let Some(out) = positions.as_deref_mut() {
            out.extend(
                scratch
                    .iter()
                    .enumerate()
                    .map(|(k, &p)| (p, p + w.unit(k).len())),
            );
        }
    }
    Some(total)
}

/// 上位の候補を選ぶヒープの要素。`Ord` は「良いほど大きい」。
#[derive(Debug)]
struct Ranked<'a> {
    score: Score,
    rel: &'a [u8],
    chunk: u32,
    index: u32,
}

impl Ranked<'_> {
    fn key(&self) -> (u32, i32, std::cmp::Reverse<usize>, std::cmp::Reverse<&[u8]>) {
        (
            self.score.in_name,
            self.score.score,
            std::cmp::Reverse(self.rel.len()),
            std::cmp::Reverse(self.rel),
        )
    }
}

impl PartialEq for Ranked<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

impl Eq for Ranked<'_> {}

impl PartialOrd for Ranked<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Ranked<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.key().cmp(&other.key())
    }
}

/// 一致した位置 (バイトの半開区間) を、相対パスの書記素の番号の半開区間にまとめる。
fn grapheme_ranges(rel: &str, spans: &mut [(usize, usize)]) -> Vec<Range<u32>> {
    let starts: Vec<usize> = eve_text::grapheme_indices(rel)
        .into_iter()
        .map(|(i, _)| i)
        .collect();
    // バイト位置を含む書記素の番号
    let index_of = |b: usize| starts.partition_point(|&s| s <= b).saturating_sub(1);
    spans.sort_unstable();
    let mut out: Vec<Range<u32>> = Vec::new();
    for &(a, b) in spans.iter() {
        if b <= a {
            continue;
        }
        let g0 = u32::try_from(index_of(a)).unwrap_or(u32::MAX);
        let g1 = u32::try_from(index_of(b - 1) + 1).unwrap_or(u32::MAX);
        match out.last_mut() {
            Some(r) if r.end >= g0 => r.end = r.end.max(g1),
            _ => out.push(g0..g1),
        }
    }
    out
}

/// 照合する (`JobKind::FinderMatch` の本体)。`chunks` の全候補を `query` で照らし、一致した件数と上位 `top` 件を返す。
/// 候補 [`FINDER_CANCEL_EVERY`] 件ごとに `cancel` を見て、立っていれば `None`。
#[must_use]
pub fn finder_match(
    chunks: &[Arc<[Candidate]>],
    query: &str,
    top: usize,
    cancel: &CancelToken,
) -> Option<FinderMatches> {
    let q = FinderQuery::parse(query);
    let mut heap: BinaryHeap<std::cmp::Reverse<Ranked<'_>>> = BinaryHeap::with_capacity(top + 1);
    let mut matched = 0usize;
    let mut seen = 0usize;
    let mut scratch: Vec<usize> = Vec::new();
    for (ci, chunk) in chunks.iter().enumerate() {
        for (ii, c) in chunk.iter().enumerate() {
            if seen.is_multiple_of(FINDER_CANCEL_EVERY) && cancel.is_cancelled() {
                return None;
            }
            seen += 1;
            let score = if q.is_empty() {
                Score {
                    in_name: 0,
                    score: 0,
                }
            } else {
                match score_candidate(c, &q, &mut scratch, None) {
                    Some(s) => s,
                    None => continue,
                }
            };
            matched += 1;
            if top == 0 {
                continue;
            }
            let r = Ranked {
                score,
                rel: c.rel().as_bytes(),
                chunk: u32::try_from(ci).unwrap_or(u32::MAX),
                index: u32::try_from(ii).unwrap_or(u32::MAX),
            };
            if heap.len() < top {
                heap.push(std::cmp::Reverse(r));
            } else if heap.peek().is_some_and(|w| r > w.0) {
                heap.pop();
                heap.push(std::cmp::Reverse(r));
            }
        }
    }
    if cancel.is_cancelled() {
        return None;
    }
    let mut ranked: Vec<Ranked<'_>> = heap.into_iter().map(|r| r.0).collect();
    ranked.sort_unstable_by(|a, b| b.cmp(a));
    let top = ranked
        .into_iter()
        .map(|r| {
            let c = &chunks[r.chunk as usize][r.index as usize];
            let mut spans = Vec::new();
            if !q.is_empty() {
                // 上位に入ったものだけ、もう一度当てて強調の位置を作る
                let _ = score_candidate(c, &q, &mut scratch, Some(&mut spans));
            }
            FinderHit {
                chunk: r.chunk,
                index: r.index,
                score: r.score.score,
                ranges: grapheme_ranges(c.rel(), &mut spans),
            }
        })
        .collect();
    Some(FinderMatches { matched, top })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score_of(rel: &str, query: &str) -> Option<(u32, i32)> {
        let c = Candidate::new(rel);
        let q = FinderQuery::parse(query);
        score_candidate(&c, &q, &mut Vec::new(), None).map(|s| (s.in_name, s.score))
    }

    #[test]
    fn 境目の加点() {
        let r = b"src/fooBar_x2y.rs";
        assert_eq!(boundary_bonus(r, 0), BONUS_BOUNDARY, "先頭");
        assert_eq!(boundary_bonus(r, 4), BONUS_BOUNDARY, "/ の直後");
        assert_eq!(boundary_bonus(r, 5), 0, "語の途中");
        assert_eq!(boundary_bonus(r, 7), BONUS_CAMEL, "camelCase");
        assert_eq!(boundary_bonus(r, 11), BONUS_BOUNDARY, "_ の直後");
        assert_eq!(boundary_bonus(r, 12), BONUS_CAMEL, "英字 → 数字");
        assert_eq!(boundary_bonus(r, 13), BONUS_CAMEL, "数字 → 英字");
        let j = "資料/a".as_bytes();
        assert_eq!(boundary_bonus(j, 3), 0, "ASCII の外の文字の続き");
    }

    #[test]
    fn 点の計算は案の値のとおり() {
        // `abc` が `xabc.rs` の 1〜3 に連続: 16×3 + 連続 4×2 + ファイル名 8×3
        assert_eq!(score_of("xabc.rs", "abc"), Some((1, 48 + 8 + 24)));
        // `ac` が `xabc.rs` の 1 と 3 (間 1 文字): 16×2 − 3 + ファイル名 8×2
        assert_eq!(score_of("xabc.rs", "ac"), Some((1, 32 - 3 + 16)));
        // 間 3 文字: −3 −1 −1
        assert_eq!(score_of("xabbbc.rs", "ac"), Some((1, 32 - 5 + 16)));
        // 先頭の境目は 2 倍: 16 + 8×2
        assert_eq!(score_of("a", "a"), Some((1, 16 + 16 + 8)));
    }

    #[test]
    fn 部分列でなければ一致しない() {
        assert_eq!(score_of("abc", "cb"), None);
        assert_eq!(score_of("abc", "abcd"), None);
        assert_eq!(score_of("資料.txt", "料資"), None);
        assert!(score_of("資料.txt", "資t").is_some());
    }

    #[test]
    fn ascii_の外の文字は文字の途中に当たらない() {
        // 「あ」(E3 81 82) と「い」(E3 81 84) の 2 バイト目までは同じ。`う` (E3 81 86) は当たらない
        assert_eq!(score_of("あい", "う"), None);
        // 全角英字は畳まない
        assert_eq!(score_of("ＡＢ", "ａ"), None);
        assert!(score_of("ＡＢ", "Ａ").is_some());
    }

    #[test]
    fn 最短の窓で採点する() {
        // 前から当てると a(0) … b(5)。後ろから当て直すと a(4) b(5) の連続
        let c = Candidate::new("axxxab");
        let q = FinderQuery::parse("ab");
        let mut pos = Vec::new();
        assert_eq!(match_word(c.lower(), &q.words[0], &mut pos), Some(4));
        assert_eq!(pos, [4, 5]);
    }

    /// 半開区間 (単一の範囲の配列を書くための補助)。
    fn r(start: u32, end: u32) -> Range<u32> {
        start..end
    }

    #[test]
    fn 書記素の区間にまとめる() {
        let mut spans = vec![(0, 1), (1, 2), (4, 5)];
        assert_eq!(grapheme_ranges("abcde", &mut spans), [0..2, 4..5]);
        // 結合文字の書記素 (e + U+0301) の中の位置は、その書記素全体
        let mut spans = vec![(1, 2)];
        assert_eq!(grapheme_ranges("ae\u{301}b", &mut spans), [r(1, 2)]);
        let mut spans = vec![(4, 5)];
        assert_eq!(grapheme_ranges("ae\u{301}b", &mut spans), [r(2, 3)]);
    }
}
