//! テキスト変換の `ChangeSet` を作る (docs/03 §15.8 の TXT-1〜TXT-4・TXT-6、01 FR-214)。メインスレッドの同期の経路と
//! `bg` のジョブ (`JobKind::TextTransform`) が同じ関数 ([`build_text_transform`]) を使う (どちらの経路でも結果が同じ)。
//!
//! 末尾空白の除去の関数 ([`trailing_whitespace`]) は、保存時の変換 (03 §9.3.1 の SVT-2) でも使う。
//!
//! spec: 03 §15.8, 03 §13.4, 03 §9.3.1, 03 §2.4

// document/ の中に置くが、文書テキストを直接変える経路 (`eve_text::Text::apply`) は使わない
// (document/mod.rs の許可を受け継がない。変更は Transaction を通す、02 §5.2)
#![deny(clippy::disallowed_methods)]

use std::sync::Arc;

use eve_text::{ByteOff, ByteRange, ChangeSet, InsertText, LineIdx, Op, TextRead};

use crate::jobs::cancel::CancelToken;

/// テキスト変換をメインスレッドで作る対象の大きさの上限 (バイト。03 §15.8 の TXT-6)。対象の合計 (Range の長さの和・
/// ブロックの長さの和) がこれを超えれば `bg` のジョブで作る。改行コードの変換 (03 §8.4)・すべて置換 (§12.5) の同期の
/// 上限と同じ値。保存時の変換 (§9.3.1 の SVT-6) は、文書がこれを超えると変換しない。
pub const TRANSFORM_SYNC_BYTES: usize = 1 << 20;

/// 取消しを確かめる行の間隔 (行の変換の途中)。
const CANCEL_EVERY_LINES: usize = 4096;

/// テキスト変換の種類 (03 §15.8、02 §9.3 の `TransformOp`)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransformOp {
    /// 大文字にする (`edit.transform_upper`、TXT-2)。
    Upper,
    /// 小文字にする (`edit.transform_lower`、TXT-2)。
    Lower,
    /// 行をコードポイントの昇順に並べ替える (`edit.sort_lines_ascending`、TXT-3)。
    SortAscending,
    /// 行をコードポイントの降順に並べ替える (`edit.sort_lines_descending`、TXT-3)。
    SortDescending,
    /// 行末の空白とタブを消す (`edit.trim_trailing_whitespace`、TXT-4)。
    TrimTrailing,
}

impl TransformOp {
    /// 行を単位にする変換 (並べ替えと末尾空白の除去。対象は行のブロック、TXT-1 (b))。
    pub fn is_line_op(self) -> bool {
        !matches!(self, TransformOp::Upper | TransformOp::Lower)
    }
}

/// テキスト変換の `ChangeSet` を作れなかった理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransformError {
    /// 取り消された (同じ文書に新しい変換を頼んだ・文書を閉じた)。
    Cancelled,
    /// 一時の領域を確保できない (02 §11.2.2。`requested` は確保しようとした量のバイト)。
    OutOfMemory {
        /// 確保しようとした量 (バイト)。
        requested: u64,
    },
}

fn oom(bytes: usize) -> TransformError {
    TransformError::OutOfMemory {
        requested: u64::try_from(bytes).unwrap_or(u64::MAX),
    }
}

fn ls(t: &dyn TextRead, l: usize) -> usize {
    t.line_to_byte(LineIdx(l)).0
}

fn le(t: &dyn TextRead, l: usize) -> usize {
    t.line_end_byte(LineIdx(l)).0
}

/// 位置 `p` のバイト (文書の外なら `None`)。
fn byte_at(t: &dyn TextRead, p: usize) -> Option<u8> {
    if p >= t.len_bytes() {
        return None;
    }
    t.chunks(ByteRange::new(ByteOff(p), ByteOff(p + 1)))
        .next()
        .and_then(|c| c.as_bytes().first().copied())
}

/// 行の区切りが単独の CR か (CRLF の CR ではない)。
fn ends_with_lone_cr(t: &dyn TextRead, l: usize) -> bool {
    let end = le(t, l);
    l + 1 < t.len_lines() && ls(t, l + 1) == end + 1 && byte_at(t, end) == Some(b'\r')
}

/// 行の区切りが単独の LF か。
fn ends_with_lone_lf(t: &dyn TextRead, l: usize) -> bool {
    let end = le(t, l);
    l + 1 < t.len_lines() && ls(t, l + 1) == end + 1 && byte_at(t, end) == Some(b'\n')
}

/// 行 `l` の中身が空になると、前の行の単独の CR とこの行の単独の LF が隣り合い、1 つの CRLF になる (03 §2.4。行の数が
/// 変わる)。
fn would_join_cr_lf(t: &dyn TextRead, l: usize) -> bool {
    l > 0 && ends_with_lone_cr(t, l - 1) && ends_with_lone_lf(t, l)
}

/// 範囲の本文を読む (一時の領域は `try_reserve` で取る)。
fn read(t: &dyn TextRead, r: ByteRange) -> Result<String, TransformError> {
    let mut s = String::new();
    s.try_reserve_exact(r.len()).map_err(|_| oom(r.len()))?;
    for c in t.chunks(r) {
        s.push_str(c);
    }
    Ok(s)
}

/// 範囲 (`ranges`) が触れる行の、連続する行のまとまり (ブロック) `(最初の行, 最後の行)` (両端を含む。03 §15.8 の TXT-1 (b))。
///
/// 行の取り方は §15.4 と同じ: 前の行から始まって行頭で終わる範囲は、その最後の行を含めない。文書が区切りで終わるときの
/// 最後の空の行 (最後の区切りの後ろ、§2.4) は含めない。重なる・隣り合うブロックは 1 つにする。`ranges` に文書の全体
/// `[0, len)` を渡すと、文書の全体の 1 つのブロックになる (空の文書は行 0 だけ)。
pub(crate) fn line_blocks(t: &dyn TextRead, ranges: &[ByteRange]) -> Vec<(usize, usize)> {
    let lines = t.len_lines();
    let final_empty = (lines > 1 && ls(t, lines - 1) == t.len_bytes()).then_some(lines - 1);
    let mut blocks: Vec<(usize, usize)> = ranges
        .iter()
        .filter_map(|r| {
            let first = t.byte_to_line(r.start).0;
            let mut last = t.byte_to_line(r.end).0;
            if r.end > r.start && last > first && ls(t, last) == r.end.0 {
                last -= 1;
            }
            if Some(last) == final_empty {
                if last == first {
                    return None;
                }
                last -= 1;
            }
            Some((first, last))
        })
        .collect();
    blocks.sort_unstable();
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (a, b) in blocks {
        match out.last_mut() {
            Some(prev) if a <= prev.1 + 1 => prev.1 = prev.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// ブロックの範囲 (最初の行の行頭から、最後の行の区切りの後ろまで。最後の行が文書の最後の行なら文書の末尾まで)。
/// [`line_blocks`] に渡すと同じブロックに戻る (写像の対象にするときの形)。
pub(crate) fn block_range(t: &dyn TextRead, (first, last): (usize, usize)) -> ByteRange {
    let end = if last + 1 < t.len_lines() {
        ls(t, last + 1)
    } else {
        t.len_bytes()
    };
    ByteRange::new(ByteOff(ls(t, first)), ByteOff(end))
}

/// ブロックの中身の範囲 (最初の行の行頭から、最後の行の区切りの直前まで。TXT-5 の並べ替えの後の選択、TXT-6 の大きさ)。
pub(crate) fn block_content(t: &dyn TextRead, (first, last): (usize, usize)) -> ByteRange {
    ByteRange::new(ByteOff(ls(t, first)), ByteOff(le(t, last)))
}

/// 行 `first..=last` の末尾の空白 (消す範囲。昇順) (03 §15.8 の TXT-4、§9.3.1 の SVT-2)。
///
/// 各行の末尾 (区切りの直前。最後の行は文書の末尾) から前へ続く ASCII の空白 (U+0020) とタブ (U+0009) だけを消す。
/// 全角の空白・ほかの Unicode の空白・不正バイトのエスケープ文字は消さない (どれも ASCII のバイトを含まないので、バイトで
/// 走査してよい)。空白だけの行は空の行になるが、**CR と LF をつながない**: 消すと前の行の単独の CR とこの行の単独の
/// LF が隣り合う (§2.4 で 1 つの CRLF になり、行の数が変わる) 行は消さない。保存時の変換 (段 3) と共用する。
/// 取り消されたら `None`。
pub(crate) fn trailing_whitespace(
    t: &dyn TextRead,
    first: usize,
    last: usize,
    cancel: &CancelToken,
) -> Option<Vec<ByteRange>> {
    let mut out = Vec::new();
    for l in first..=last.min(t.len_lines().saturating_sub(1)) {
        if (l - first).is_multiple_of(CANCEL_EVERY_LINES) && cancel.is_cancelled() {
            return None;
        }
        let (start, end) = (ls(t, l), le(t, l));
        if start == end {
            continue;
        }
        // 最後の空白でないバイトの直後 (行の全体を 1 回なめる。SVT-9)
        let mut keep = start;
        let mut pos = start;
        for chunk in t.chunks(ByteRange::new(ByteOff(start), ByteOff(end))) {
            for (i, b) in chunk.bytes().enumerate() {
                if b != b' ' && b != b'\t' {
                    keep = pos + i + 1;
                }
            }
            pos += chunk.len();
        }
        if keep == end || (keep == start && would_join_cr_lf(t, l)) {
            continue;
        }
        out.push(ByteRange::new(ByteOff(keep), ByteOff(end)));
    }
    Some(out)
}

/// 置き換えの列 (昇順で重ならない。範囲と入れる文字列)。
type Edits = Vec<(ByteRange, Arc<str>)>;

/// 置き換えの列 (昇順で重ならない) から `ChangeSet` を作る。
fn changeset(len: usize, edits: Vec<(ByteRange, Arc<str>)>) -> ChangeSet {
    let mut ops: Vec<Op> = Vec::with_capacity(edits.len() * 3 + 1);
    let mut pos = 0usize;
    for (r, s) in edits {
        ops.push(Op::Retain(r.start.0 - pos));
        ops.push(Op::Delete(r.len()));
        ops.push(Op::Insert(InsertText::Str(s)));
        pos = r.end.0;
    }
    ops.push(Op::Retain(len - pos));
    ChangeSet::from_ops(ops)
}

/// 大文字・小文字の変換で、変換した文字列が元の何倍のバイトまで伸び得るか (`to_uppercase` の最大は `ΐ` の 2 バイトが
/// 3 文字 6 バイトになる 3 倍)。この見積もりで先に一時の領域を取る (02 §11.2.2)。
const CASE_GROWTH: usize = 3;

/// 大文字・小文字の変換 (TXT-2) を `out` に足す。Rust の標準の `to_uppercase` / `to_lowercase` と同じ結果 (完全な
/// Unicode・ロケールに依らない)。不正バイトのエスケープ文字 (私用面の U+10FFxx) と、大文字・小文字の無い文字は変わらない。
///
/// 大文字は 1 文字ずつ変換して `out` に入れる (標準の `str::to_uppercase` も 1 文字ずつ)。小文字は語末のシグマ (`Σ` →
/// `ς`) が前後の文字で決まるので、ASCII の空白 (大文字・小文字が無く、無視できる文字でもない) で区切った断片ごとに標準の
/// `str::to_lowercase` を使う (結果は全体に当てたものと同じ)。
fn convert_case_into(s: &str, op: TransformOp, out: &mut String) {
    match op {
        TransformOp::Lower => {
            for piece in s.split_inclusive(|c: char| c.is_ascii_whitespace()) {
                out.push_str(&piece.to_lowercase());
            }
        }
        _ => {
            for c in s.chars() {
                out.extend(c.to_uppercase());
            }
        }
    }
}

/// 大文字・小文字の編集: Range ごとに、変換した文字列が元と違えば Range の全体を置き換える。
fn case_edits(
    t: &dyn TextRead,
    op: TransformOp,
    ranges: &[ByteRange],
    cancel: &CancelToken,
) -> Result<Vec<(ByteRange, Arc<str>)>, TransformError> {
    let len = t.len_bytes();
    let mut sorted: Vec<ByteRange> = ranges
        .iter()
        .map(|r| ByteRange::new(r.start, ByteOff(r.end.0.min(len))))
        .filter(|r| !r.is_empty())
        .collect();
    sorted.sort_unstable_by_key(|r| (r.start, r.end));
    let mut out = Vec::new();
    let mut prev_end = 0usize;
    for r in sorted {
        if cancel.is_cancelled() {
            return Err(TransformError::Cancelled);
        }
        // 重なる Range は飛ばす (Selection の Range は重ならない。写像した後も順は保たれる)
        if r.start.0 < prev_end {
            continue;
        }
        prev_end = r.end.0;
        let s = read(t, r)?;
        let want = s.len().saturating_mul(CASE_GROWTH);
        let mut converted = String::new();
        converted.try_reserve_exact(want).map_err(|_| oom(want))?;
        convert_case_into(&s, op, &mut converted);
        if converted != s {
            // `Arc<str>` への写しは避けられない (安定版の Rust では String の確保をそのまま使えない)
            out.push((r, Arc::from(converted)));
        }
    }
    Ok(out)
}

/// 行の並べ替えの編集 (TXT-3)。ブロックの各行の中身 (区切りを除く) を、コードポイントの順 (UTF-8 のバイトの順) で
/// 安定に並べ替える。区切りは動かさず、中身の変わる行だけを置き換える。並べ替えると、中身が空になる行で前の行の単独の
/// CR とその行の単独の LF がつながる (§2.4 で 1 つの CRLF になり、区切りの並びが変わる) ブロックは並べ替えない。
fn sort_edits(
    t: &dyn TextRead,
    blocks: &[(usize, usize)],
    descending: bool,
    cancel: &CancelToken,
) -> Result<(Edits, bool), TransformError> {
    let mut out = Vec::new();
    let mut skipped = false;
    for &(first, last) in blocks {
        if cancel.is_cancelled() {
            return Err(TransformError::Cancelled);
        }
        let n = last - first + 1;
        if n < 2 {
            continue;
        }
        let base = ls(t, first);
        let buf = read(t, block_content(t, (first, last)))?;
        // 行ごとの中身の範囲 (`buf` の中の位置) と並び。どちらも行の数に比例する一時の領域 (02 §11.2.2)
        let mut spans: Vec<(usize, usize)> = Vec::new();
        spans
            .try_reserve_exact(n)
            .map_err(|_| oom(n * std::mem::size_of::<(usize, usize)>()))?;
        for l in first..=last {
            if (l - first).is_multiple_of(CANCEL_EVERY_LINES) && cancel.is_cancelled() {
                return Err(TransformError::Cancelled);
            }
            spans.push((ls(t, l) - base, le(t, l) - base));
        }
        let mut order: Vec<usize> = Vec::new();
        order
            .try_reserve_exact(n)
            .map_err(|_| oom(n * std::mem::size_of::<usize>()))?;
        order.extend(0..n);
        let line = |i: usize| &buf[spans[i].0..spans[i].1];
        // `sort_by` は安定 (等しい行は元の順を保つ)。降順も、比べ方を逆にして安定に並べる
        if descending {
            order.sort_by(|&a, &b| line(b).cmp(line(a)));
        } else {
            order.sort_by(|&a, &b| line(a).cmp(line(b)));
        }
        if cancel.is_cancelled() {
            return Err(TransformError::Cancelled);
        }
        // 中身が空になる行で CR と LF がつながるなら、このブロックは並べ替えない (区切りの並びを保つ)
        let joins = order
            .iter()
            .enumerate()
            .any(|(i, &src)| line(src).is_empty() && would_join_cr_lf(t, first + i));
        if joins {
            skipped = true;
            continue;
        }
        // 中身の変わる行の続き (連続する行) ごとに 1 つの置き換えにする。間の区切りは元のまま入れ直す (大きな文書で行ごとに
        // 確保しない)。中身の変わらない行は置き換えない
        let changed = |i: usize| order[i] != i && line(order[i]) != line(i);
        let mut i = 0;
        while i < n {
            if !changed(i) {
                i += 1;
                continue;
            }
            let run_start = i;
            while i + 1 < n && changed(i + 1) {
                i += 1;
            }
            let (from, to) = (spans[run_start].0, spans[i].1);
            let mut s = String::new();
            s.try_reserve_exact(to - from).map_err(|_| oom(to - from))?;
            for k in run_start..=i {
                s.push_str(line(order[k]));
                if k < i {
                    // 行 k と行 k + 1 の間の区切り
                    s.push_str(&buf[spans[k].1..spans[k + 1].0]);
                }
            }
            let r = ByteRange::new(ByteOff(base + from), ByteOff(base + to));
            out.push((r, Arc::from(s)));
            i += 1;
        }
    }
    Ok((out, skipped))
}

/// テキスト変換の `ChangeSet` を作る (03 §15.8)。メインスレッドの経路とジョブ (`JobKind::TextTransform`) が同じ関数を使う。
///
/// `ranges` は対象: 大文字・小文字では変換する Range (と単語) の範囲、行の変換ではブロックの範囲 ([`block_range`]。文書の
/// 全体なら `[0, len)` の 1 つ)。行の変換は [`line_blocks`] でブロックに直してから行う。変わる箇所が無ければ空の
/// (`is_identity` の) `ChangeSet` を返す。並べ替えで、CR と LF がつながるので並べ替えなかったブロックがあれば
/// `sort_skipped_cr_lf` が真 (TXT-3)。
///
/// # Errors
///
/// 取り消されたら `Cancelled`、一時の領域を確保できなければ `OutOfMemory`。
pub fn build_text_transform(
    t: &dyn TextRead,
    op: TransformOp,
    ranges: &[ByteRange],
    cancel: &CancelToken,
) -> Result<TransformBuilt, TransformError> {
    let mut sort_skipped_cr_lf = false;
    let edits = match op {
        TransformOp::Upper | TransformOp::Lower => case_edits(t, op, ranges, cancel)?,
        TransformOp::SortAscending | TransformOp::SortDescending => {
            let (edits, skipped) = sort_edits(
                t,
                &line_blocks(t, ranges),
                op == TransformOp::SortDescending,
                cancel,
            )?;
            sort_skipped_cr_lf = skipped;
            edits
        }
        TransformOp::TrimTrailing => {
            let mut out = Vec::new();
            let empty: Arc<str> = Arc::from("");
            for (first, last) in line_blocks(t, ranges) {
                let found =
                    trailing_whitespace(t, first, last, cancel).ok_or(TransformError::Cancelled)?;
                out.extend(found.into_iter().map(|r| (r, Arc::clone(&empty))));
            }
            out
        }
    };
    Ok(TransformBuilt {
        changes: changeset(t.len_bytes(), edits),
        sort_skipped_cr_lf,
    })
}

/// [`build_text_transform`] の結果。
#[derive(Debug)]
pub struct TransformBuilt {
    /// 変換の `ChangeSet` (変わる箇所が無ければ `is_identity`)。
    pub changes: ChangeSet,
    /// 並べ替えで、CR と LF がつながるので並べ替えなかったブロックがある (TXT-3)。
    pub sort_skipped_cr_lf: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use eve_text::Snapshot;

    fn all(t: &Snapshot) -> Vec<ByteRange> {
        vec![ByteRange::all(t.len_bytes())]
    }

    fn trimmed(s: &str) -> Vec<(usize, usize)> {
        let t = Snapshot::from(s);
        let last = t.len_lines() - 1;
        trailing_whitespace(&t, 0, last, &CancelToken::new())
            .expect("取り消していない")
            .iter()
            .map(|r| (r.start.0, r.end.0))
            .collect()
    }

    #[test]
    fn 末尾空白は_ascii_の空白とタブだけを消し_全角の空白は残す() {
        assert_eq!(trimmed("a \t\nb\u{3000}\nc  "), vec![(1, 3), (10, 12)]);
    }

    #[test]
    fn 末尾空白で_cr_と_lf_がつながる行は消さない() {
        // a⏎(CR)␠⏎(LF)b: 真ん中の行を消すと \r\n になり行が減る
        assert_eq!(trimmed("a\r \nb"), Vec::<(usize, usize)>::new());
        // CRLF の後の空白だけの行は消してよい (つながらない)
        assert_eq!(trimmed("a\r\n \nb"), vec![(3, 4)]);
        // 空白の前に文字があればつながらない
        assert_eq!(trimmed("a\rx \nb"), vec![(3, 4)]);
    }

    #[test]
    fn ブロックは行頭で終わる範囲の最後の行と最後の空の行を含めない() {
        let t = Snapshot::from("a\nb\nc\n");
        assert_eq!(line_blocks(&t, &all(&t)), vec![(0, 2)]);
        assert_eq!(
            line_blocks(&t, &[ByteRange::new(ByteOff(0), ByteOff(2))]),
            vec![(0, 0)]
        );
        // 隣り合うブロックは 1 つにする
        assert_eq!(
            line_blocks(
                &t,
                &[
                    ByteRange::new(ByteOff(0), ByteOff(1)),
                    ByteRange::new(ByteOff(2), ByteOff(3))
                ]
            ),
            vec![(0, 1)]
        );
        let empty = Snapshot::from("");
        assert_eq!(line_blocks(&empty, &all(&empty)), vec![(0, 0)]);
        // block_range を渡すと同じブロックに戻る
        let r = block_range(&t, (1, 2));
        assert_eq!(line_blocks(&t, &[r]), vec![(1, 2)]);
    }

    #[test]
    fn 並べ替えで_cr_と_lf_がつながるブロックは並べ替えない() {
        // 行: "" (CR), "x" (LF), "" (LF)。昇順だと 2 行目が空になり \r\n になる
        let t = Snapshot::from("\rx\n\n");
        let built = build_text_transform(
            &t,
            TransformOp::SortAscending,
            &all(&t),
            &CancelToken::new(),
        )
        .expect("作れる");
        assert!(built.changes.is_identity());
        assert!(built.sort_skipped_cr_lf);
    }

    #[test]
    fn 小文字は語末のシグマも標準と同じで_大文字は長さの変わる文字も同じ() {
        for s in ["ΣΑΣ ΣΑΣ.\tΌΣ", "straße ﬃ ΐ", "A\u{10FF41}Σ\u{301}"] {
            for (op, want) in [
                (TransformOp::Lower, s.to_lowercase()),
                (TransformOp::Upper, s.to_uppercase()),
            ] {
                let mut out = String::new();
                convert_case_into(s, op, &mut out);
                assert_eq!(out, want, "{s:?} {op:?}");
                assert!(out.len() <= s.len() * CASE_GROWTH);
            }
        }
    }

    #[test]
    fn 並べ替えは中身の変わる行の続きを_1_つの置き換えにし_変わらない行は残す() {
        // 行 0〜1 が入れ替わり、行 2 は同じ、行 3〜4 が入れ替わる
        let t = Snapshot::from("b\r\na\nc\ne\rd\n");
        let cs = build_text_transform(
            &t,
            TransformOp::SortAscending,
            &all(&t),
            &CancelToken::new(),
        )
        .expect("作れる");
        assert!(!cs.sort_skipped_cr_lf);
        let changed: Vec<(usize, usize)> = cs
            .changes
            .changed_ranges()
            .map(|(b, _)| (b.start.0, b.end.0))
            .collect();
        assert_eq!(changed, vec![(0, 4), (7, 10)]);
    }

    #[test]
    fn 取り消すと作らない() {
        let t = Snapshot::from("b\na\n");
        let cancel = CancelToken::new();
        cancel.cancel();
        assert_eq!(
            build_text_transform(&t, TransformOp::SortAscending, &all(&t), &cancel).err(),
            Some(TransformError::Cancelled)
        );
    }
}
