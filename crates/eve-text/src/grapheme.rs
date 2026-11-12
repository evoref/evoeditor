//! 書記素境界 (docs/03-core-design.md §4.1 / §13.1)。拡張書記素クラスタ (UAX #29) を `unicode-segmentation` で判定する。
//!
//! 判定は位置の前後 `CONTEXT` バイトを連結した小さなバッファの上で行う (§13.1)。それより長い結合列は正確さを諦める
//! (バッファの端を境界として扱う)。地域指示記号 (国旗) の対の偶奇も同じ制約を受ける (§14 の既知の制約)。
//!
//! 不正バイトのエスケープ文字 (`U+10FF00..=U+10FFFF`、§8.3) の前後は、UAX #29 に依らず常に境界とする (§8.3.1 の
//! 「書記素と単語」)。文書の `pua_escape` は見ない (この層は文書の状態を知らない)。境界の集合は
//! 「UAX #29 の境界 ∪ エスケープ文字の前後」で、文書の位置で求める関数 (`is_grapheme_boundary` など) と、
//! 文字列を分ける `str_graphemes` が同じ規則を使う。

use unicode_segmentation::{GraphemeCursor, GraphemeIndices, UnicodeSegmentation};

use crate::changeset::Bias;
use crate::text::TextRead;
use crate::types::{ByteOff, ByteRange, Pos};

/// 位置の前後に取る文脈のバイト数 (§13.1)。
pub const CONTEXT: usize = 256;

/// `pos` を含む小さなバッファを作り、`f(バッファ, バッファ内の位置, バッファの先頭の文書内の位置)` を呼ぶ。`pos` は文書の長さに収め済みで char 境界。
fn with_window<R>(text: &dyn TextRead, pos: usize, f: impl FnOnce(&str, usize, usize) -> R) -> R {
    let len = text.len_bytes();
    let mut lo = pos.saturating_sub(CONTEXT);
    while lo > 0 && !text.is_char_boundary(ByteOff(lo)) {
        lo -= 1;
    }
    let mut hi = (pos + CONTEXT).min(len);
    while hi < len && !text.is_char_boundary(ByteOff(hi)) {
        hi += 1;
    }
    let buf = text.slice_to_string(ByteRange::new(ByteOff(lo), ByteOff(hi)));
    f(&buf, pos - lo, lo)
}

/// 範囲内に収め、char 境界でなければ `bias` の側の char 境界へ寄せる。
fn to_char_boundary(text: &dyn TextRead, pos: Pos, bias: Bias) -> usize {
    let len = text.len_bytes();
    let mut p = pos.0.min(len);
    match bias {
        Bias::Before => {
            while p > 0 && !text.is_char_boundary(ByteOff(p)) {
                p -= 1;
            }
        }
        Bias::After => {
            while p < len && !text.is_char_boundary(ByteOff(p)) {
                p += 1;
            }
        }
    }
    p
}

/// 前後のどちらに何が来ても結合しない文字 (UAX #29 の `Grapheme_Cluster_Break` が Other で、絵文字の ZWJ 列・地域指示記号・
/// ハングルの字母・インド系の結合にも関わらない) のうち、本文によく出る範囲 (印字できる ASCII・かな・漢字・全角の英数と主な句読点)。
/// 両隣がこの範囲なら、窓を切り出さずに境界と分かる (REF-01)。範囲の全文字を試験で確かめている。
fn never_joins(c: char) -> bool {
    matches!(c as u32,
        0x20..=0x7E            // 印字できる ASCII
        | 0x3001..=0x3002      // 、。
        | 0x300C..=0x300D      // 「」
        | 0x3041..=0x3096      // ひらがな (結合する濁点 U+3099・U+309A は除く)
        | 0x30A1..=0x30FA | 0x30FC // カタカナ
        | 0x4E00..=0x9FFF      // CJK 統合漢字
        | 0xFF01..=0xFF5E) // 全角の ASCII
}

/// 不正バイトのエスケープ文字 (`U+10FF00..=U+10FFFF`、03 §8.3) か。UTF-8 では `F4 8F BC..=BF xx` の 4 バイト。
pub(crate) fn is_escape_char(c: char) -> bool {
    matches!(c, '\u{10FF00}'..='\u{10FFFF}')
}

/// エスケープ文字の UTF-8 のバイト数。
const ESCAPE_LEN: usize = 4;

/// 文字列がエスケープ文字を含み得るか (先頭バイト 0xF4 の有無だけを見る安い判定)。
fn may_contain_escape(s: &str) -> bool {
    s.len() >= ESCAPE_LEN && s.as_bytes().contains(&0xF4)
}

/// `s` の内側 (両端を除く) で、エスケープ文字の前後に当たる最初の位置。無ければ `None`。
fn first_escape_cut(s: &str) -> Option<usize> {
    let cut = if s.chars().next().is_some_and(is_escape_char) {
        ESCAPE_LEN
    } else {
        s.char_indices().find(|&(_, c)| is_escape_char(c))?.0
    };
    (cut < s.len()).then_some(cut)
}

/// `s` の内側 (両端を除く) で、エスケープ文字の前後に当たる最後の位置。無ければ `None`。
fn last_escape_cut(s: &str) -> Option<usize> {
    let cut = if s.chars().next_back().is_some_and(is_escape_char) {
        s.len() - ESCAPE_LEN
    } else {
        s.char_indices().rfind(|&(_, c)| is_escape_char(c))?.0 + ESCAPE_LEN
    };
    (cut > 0).then_some(cut)
}

/// 文字列の書記素 (先頭からのバイト位置, 書記素) の列 (§13.1)。UAX #29 (`unicode-segmentation`) の境界に、
/// エスケープ文字の前後を足したもの。文書の位置で求める `is_grapheme_boundary` などと同じ境界になる
/// (文書は窓の大きさの制約を受ける)。
pub(crate) fn str_graphemes(s: &str) -> StrGraphemes<'_> {
    StrGraphemes {
        inner: s.grapheme_indices(true),
        rest: None,
    }
}

/// `str_graphemes` の反復子。
pub(crate) struct StrGraphemes<'a> {
    inner: GraphemeIndices<'a>,
    /// UAX #29 の 1 書記素をエスケープ文字の前後で切ったときの、まだ返していない残り。
    rest: Option<(usize, &'a str)>,
}

impl<'a> Iterator for StrGraphemes<'a> {
    type Item = (usize, &'a str);

    fn next(&mut self) -> Option<Self::Item> {
        let (off, g) = match self.rest.take() {
            Some(r) => r,
            None => self.inner.next()?,
        };
        if !may_contain_escape(g) {
            return Some((off, g));
        }
        let Some(n) = first_escape_cut(g) else {
            return Some((off, g));
        };
        self.rest = Some((off + n, &g[n..]));
        Some((off, &g[..n]))
    }
}

/// char 境界 `pos` から始まる文字 (末尾なら `None`)。チャンクの中で読む (確保しない)。
fn char_after(text: &dyn TextRead, pos: usize) -> Option<char> {
    let (chunk, start) = text.chunk_at(ByteOff(pos));
    chunk.get(pos.checked_sub(start.0)?..)?.chars().next()
}

/// char 境界 `pos` で終わる文字 (先頭なら `None`)。チャンクの中で読む (確保しない)。
fn char_before(text: &dyn TextRead, pos: usize) -> Option<char> {
    let (chunk, start) = text.chunk_at(ByteOff(pos.checked_sub(1)?));
    chunk.get(..pos.checked_sub(start.0)?)?.chars().next_back()
}

/// `pos` が書記素境界か。文書の先頭と末尾は境界。char 境界でない位置は `false`。範囲外は `false`。
pub fn is_grapheme_boundary(text: &dyn TextRead, pos: Pos) -> bool {
    let len = text.len_bytes();
    if pos.0 > len || !text.is_char_boundary(pos) {
        return false;
    }
    if pos.0 == 0 || pos.0 == len {
        return true;
    }
    if char_before(text, pos.0).is_some_and(never_joins)
        && char_after(text, pos.0).is_some_and(never_joins)
    {
        return true;
    }
    is_boundary_window(text, pos.0)
}

/// `is_grapheme_boundary` の窓の経路 (`0 < pos < len` の char 境界)。エスケープ文字の前後は常に境界。
fn is_boundary_window(text: &dyn TextRead, pos: usize) -> bool {
    if char_before(text, pos).is_some_and(is_escape_char)
        || char_after(text, pos).is_some_and(is_escape_char)
    {
        return true;
    }
    with_window(text, pos, |buf, rel, _| {
        GraphemeCursor::new(rel, buf.len(), true)
            .is_boundary(buf, 0)
            .unwrap_or(true)
    })
}

/// `pos` より後ろの最初の書記素境界。末尾にいれば末尾。`pos` が char 境界でなければ、含む char の先頭から数える。
pub fn next_grapheme_boundary(text: &dyn TextRead, pos: Pos) -> Pos {
    let len = text.len_bytes();
    let p = to_char_boundary(text, pos, Bias::Before);
    if p >= len {
        return ByteOff(len);
    }
    // 今の文字と次の文字がどちらも結合しないなら、次の境界は今の文字の直後
    if let Some(c) = char_after(text, p).filter(|&c| never_joins(c)) {
        let q = p + c.len_utf8();
        if q == len || char_after(text, q).is_some_and(never_joins) {
            return ByteOff(q);
        }
    }
    next_boundary_window(text, p)
}

/// `next_grapheme_boundary` の窓の経路 (`p < len` の char 境界)。UAX #29 の次の境界までにエスケープ文字があれば、
/// その前 (`p` がエスケープ文字ならその後ろ) で止まる。
fn next_boundary_window(text: &dyn TextRead, p: usize) -> Pos {
    let len = text.len_bytes();
    let next = with_window(text, p, |buf, rel, lo| {
        let mut cur = GraphemeCursor::new(rel, buf.len(), true);
        let uax = cur.next_boundary(buf, 0).ok().flatten();
        let seg = &buf[rel..uax.unwrap_or(buf.len())];
        let cut = may_contain_escape(seg)
            .then(|| first_escape_cut(seg))
            .flatten()
            .map(|n| rel + n);
        cut.or(uax).map(|b| lo + b)
    });
    // 進まないことはないが、保険として必ず 1 バイト以上進める
    ByteOff(next.unwrap_or(len).clamp(p + 1, len))
}

/// `pos` より前の最後の書記素境界。先頭にいれば先頭。`pos` が char 境界でなければ、含む char の末尾から数える。
pub fn prev_grapheme_boundary(text: &dyn TextRead, pos: Pos) -> Pos {
    let p = to_char_boundary(text, pos, Bias::After);
    if p == 0 {
        return ByteOff(0);
    }
    // 前の文字とその前の文字がどちらも結合しないなら、前の境界は前の文字の直前
    if let Some(c) = char_before(text, p).filter(|&c| never_joins(c)) {
        let q = p - c.len_utf8();
        if q == 0 || char_before(text, q).is_some_and(never_joins) {
            return ByteOff(q);
        }
    }
    prev_boundary_window(text, p)
}

/// `prev_grapheme_boundary` の窓の経路 (`0 < p` の char 境界)。UAX #29 の前の境界から `p` までにエスケープ文字があれば、
/// その後ろ (`p` の直前がエスケープ文字ならその前) で止まる。
fn prev_boundary_window(text: &dyn TextRead, p: usize) -> Pos {
    let prev = with_window(text, p, |buf, rel, lo| {
        let mut cur = GraphemeCursor::new(rel, buf.len(), true);
        let uax = cur.prev_boundary(buf, 0).ok().flatten();
        let start = uax.unwrap_or(0);
        let seg = &buf[start..rel];
        let cut = may_contain_escape(seg)
            .then(|| last_escape_cut(seg))
            .flatten()
            .map(|k| start + k);
        cut.or(uax).map(|b| lo + b)
    });
    ByteOff(prev.unwrap_or(0).min(p - 1))
}

/// `pos` を書記素境界に寄せる (03 §4.1)。`Bias::Before` は前の境界、`Bias::After` は後ろの境界。
/// すでに境界ならそのまま。範囲外は文書の末尾に収める。
pub fn snap_to_grapheme(text: &dyn TextRead, pos: Pos, bias: Bias) -> Pos {
    // 先に char 境界へ寄せる (寄せた先が書記素境界なら、それが答え)
    let p = ByteOff(to_char_boundary(text, pos, bias));
    if is_grapheme_boundary(text, p) {
        return p;
    }
    match bias {
        Bias::Before => prev_grapheme_boundary(text, p),
        Bias::After => next_grapheme_boundary(text, p),
    }
}

/// 近い方の書記素境界に寄せる (同じ距離なら前)。向きを持たない正規化 (`Selection::normalized`) に使う。
pub fn snap_to_nearest_grapheme(text: &dyn TextRead, pos: Pos) -> Pos {
    let before = snap_to_grapheme(text, pos, Bias::Before);
    if before.0 == pos.0.min(text.len_bytes()) {
        return before;
    }
    let after = snap_to_grapheme(text, pos, Bias::After);
    if pos.0.min(text.len_bytes()) - before.0 <= after.0 - pos.0.min(text.len_bytes()) {
        before
    } else {
        after
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::Text;

    fn t(s: &str) -> Text {
        Text::from(s)
    }

    /// 書記素境界の位置を、全位置を走査して集める。
    fn boundaries(text: &Text) -> Vec<usize> {
        (0..=text.len_bytes())
            .filter(|&i| is_grapheme_boundary(text, ByteOff(i)))
            .collect()
    }

    #[test]
    fn 基本の書記素() {
        assert_eq!(boundaries(&t("abc")), [0, 1, 2, 3]);
        assert_eq!(boundaries(&t("")), [0]);
        assert_eq!(
            boundaries(&t("あい")),
            [0, 3, 6],
            "char 境界でない位置は境界ではない"
        );
        assert_eq!(
            boundaries(&t("e\u{301}x")),
            [0, 3, 4],
            "結合文字は基底に付く"
        );
    }

    #[test]
    fn crlf_は_1_つ() {
        let x = t("a\r\nb");
        assert_eq!(boundaries(&x), [0, 1, 3, 4]);
        assert!(
            !is_grapheme_boundary(&x, ByteOff(2)),
            "CR と LF の間は置けない"
        );
        assert_eq!(snap_to_grapheme(&x, ByteOff(2), Bias::Before), ByteOff(1));
        assert_eq!(snap_to_grapheme(&x, ByteOff(2), Bias::After), ByteOff(3));
        assert_eq!(next_grapheme_boundary(&x, ByteOff(1)), ByteOff(3));
        assert_eq!(prev_grapheme_boundary(&x, ByteOff(3)), ByteOff(1));
    }

    #[test]
    fn 絵文字と国旗() {
        let family = "👨\u{200D}👩\u{200D}👧"; // ZWJ 列
        assert_eq!(boundaries(&t(family)), [0, family.len()]);
        let flags = "🇯🇵🇺🇸"; // 地域指示記号の対 2 つ
        assert_eq!(boundaries(&t(flags)), [0, 8, 16]);
        assert_eq!(boundaries(&t("👍🏽")), [0, "👍🏽".len()], "肌色修飾");
        assert_eq!(
            boundaries(&t("1\u{FE0F}\u{20E3}")),
            [0, "1\u{FE0F}\u{20E3}".len()],
            "キーキャップ"
        );
    }

    #[test]
    fn 移動は書記素単位() {
        let x = t("a👨\u{200D}👩b");
        let end = x.len_bytes();
        let mut p = ByteOff(0);
        let mut seen = vec![0];
        loop {
            let n = next_grapheme_boundary(&x, p);
            if n == p {
                break;
            }
            seen.push(n.0);
            p = n;
        }
        assert_eq!(seen, [0, 1, end - 1, end]);
        let mut back = vec![end];
        let mut p = ByteOff(end);
        loop {
            let n = prev_grapheme_boundary(&x, p);
            if n == p {
                break;
            }
            back.push(n.0);
            p = n;
        }
        back.reverse();
        assert_eq!(back, seen, "前向きと後ろ向きの境界が一致する");
    }

    #[test]
    fn 範囲外と_char_境界でない位置は丸める() {
        let x = t("あ");
        assert_eq!(snap_to_grapheme(&x, ByteOff(99), Bias::Before), ByteOff(3));
        assert_eq!(snap_to_grapheme(&x, ByteOff(1), Bias::Before), ByteOff(0));
        assert_eq!(snap_to_grapheme(&x, ByteOff(1), Bias::After), ByteOff(3));
        assert_eq!(next_grapheme_boundary(&x, ByteOff(3)), ByteOff(3));
        assert_eq!(prev_grapheme_boundary(&x, ByteOff(0)), ByteOff(0));
        assert!(!is_grapheme_boundary(&x, ByteOff(99)));
    }

    #[test]
    fn 近い方への正規化() {
        let x = t("a\r\nb");
        assert_eq!(
            snap_to_nearest_grapheme(&x, ByteOff(2)),
            ByteOff(1),
            "同じ距離なら前"
        );
        let y = t("e\u{301}\u{301}\u{301}x"); // 基底 + 結合 3 つ = 7 バイトの書記素
        assert_eq!(snap_to_nearest_grapheme(&y, ByteOff(6)), ByteOff(7));
        assert_eq!(snap_to_nearest_grapheme(&y, ByteOff(2)), ByteOff(0));
    }

    #[test]
    fn 長い結合列は文脈の上限で諦める() {
        // 基底 + 結合文字 400 個 (800 バイト)。バッファの端を境界として扱うので、パニックせず前進する
        let s = format!("a{}", "\u{301}".repeat(400));
        let x = t(&s);
        let mut p = ByteOff(0);
        let mut steps = 0;
        while p.0 < x.len_bytes() {
            let n = next_grapheme_boundary(&x, p);
            assert!(n > p, "必ず前に進む");
            p = n;
            steps += 1;
            assert!(steps < 100);
        }
    }

    /// 近道の範囲の全文字が、代表の文字 (ASCII・かな・自分自身) のどちら側に来ても窓の経路で境界になる
    /// (Extend・SpacingMark・Prepend・ZWJ・地域指示記号・ハングルの字母が範囲に混ざっていない)。
    #[test]
    fn 結合しない文字の範囲は全文字で窓の経路でも境界になる() {
        let mut checked = 0;
        for v in 0..=0x10FFFFu32 {
            let Some(c) = char::from_u32(v).filter(|&c| never_joins(c)) else {
                continue;
            };
            for x in ['a', 'あ', c] {
                for s in [format!("{x}{c}"), format!("{c}{x}")] {
                    let text = t(&s);
                    let mid = s.chars().next().unwrap().len_utf8();
                    assert!(is_boundary_window(&text, mid), "{v:#x} {s:?}");
                }
            }
            checked += 1;
        }
        assert!(checked > 20_000, "{checked}");
    }

    fn any_char() -> impl proptest::strategy::Strategy<Value = char> {
        use proptest::prelude::*;
        prop_oneof![
            6 => proptest::char::range('a', 'e'),
            2 => Just(' '),
            1 => Just('\t'),
            2 => Just('\r'),
            2 => Just('\n'),
            4 => Just('あ'),
            2 => Just('ア'),
            2 => Just('漢'),
            1 => Just('、'),
            1 => Just('Ａ'),
            2 => Just('\u{301}'),
            1 => Just('\u{3099}'),
            1 => Just('\u{FF9E}'),
            1 => Just('ｶ'),
            1 => Just('\u{200D}'),
            1 => Just('\u{FE0F}'),
            1 => Just('👨'),
            1 => Just('\u{1F1EF}'),
            1 => Just('\u{1100}'),
            1 => Just('\u{1161}'),
            1 => Just('\u{0915}'),
            1 => Just('\u{094D}'),
            1 => Just('\u{0600}'),
            2 => proptest::char::range('\u{10FF00}', '\u{10FFFF}'),
        ]
    }

    /// 1 件が重い試験 (全位置 × 窓) の件数。既定は 48 件、`PROPTEST_CASES` があればその 1/20 (夜間の 10000 なら 500)。
    /// `proptest!` のマクロは `PROPTEST_CASES` で件数を上書きする (夜間に 1 時間を超えた、Issue #453) ので、
    /// この試験は `TestRunner` を直に使って件数を決める。
    fn heavy_config() -> proptest::test_runner::Config {
        let cases = std::env::var("PROPTEST_CASES")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .map_or(48, |n| (n / 20).max(48));
        proptest::test_runner::Config {
            cases,
            source_file: Some(file!()),
            ..proptest::test_runner::Config::default()
        }
    }

    /// 近道を入れた判定は、全位置で窓の経路 (近道の前の実装) と一致する。チャンクの境目をまたぐ長さで試す。
    #[test]
    fn 近道の判定は全位置で窓の経路と一致する() {
        let mut runner = proptest::test_runner::TestRunner::new(heavy_config());
        let strategy = proptest::collection::vec(any_char(), 0..3000);
        let result = runner.run(&strategy, |v| {
            let s: String = v.into_iter().collect();
            let text = t(&s);
            let len = s.len();
            for i in 0..=len {
                let p = ByteOff(i);
                if !s.is_char_boundary(i) {
                    proptest::prop_assert!(!is_grapheme_boundary(&text, p));
                    continue;
                }
                let want = i == 0 || i == len || is_boundary_window(&text, i);
                proptest::prop_assert_eq!(is_grapheme_boundary(&text, p), want, "{}", i);
                if i < len {
                    proptest::prop_assert_eq!(
                        next_grapheme_boundary(&text, p),
                        next_boundary_window(&text, i),
                        "{}",
                        i
                    );
                }
                if i > 0 {
                    proptest::prop_assert_eq!(
                        prev_grapheme_boundary(&text, p),
                        prev_boundary_window(&text, i),
                        "{}",
                        i
                    );
                }
            }
            Ok(())
        });
        if let Err(e) = result {
            panic!(
                "{e}
{runner}"
            );
        }
    }

    #[test]
    fn エスケープ文字の判定は範囲の両端を含む() {
        assert!(is_escape_char('\u{10FF00}') && is_escape_char('\u{10FFFF}'));
        assert!(!is_escape_char('\u{10FEFF}') && !is_escape_char('\u{F8FF}'));
        // UTF-8 の先頭バイトは 0xF4 (安い判定の前提)
        for c in ['\u{10FF00}', '\u{10FF9A}', '\u{10FFFF}'] {
            assert_eq!(c.len_utf8(), ESCAPE_LEN);
            assert_eq!(c.to_string().as_bytes()[0], 0xF4);
        }
    }

    #[test]
    fn 文字列の分割で内側の切れ目だけを返す() {
        let e = "\u{10FF80}";
        assert_eq!(first_escape_cut(e), None, "エスケープ文字 1 つは切らない");
        assert_eq!(last_escape_cut(e), None);
        assert_eq!(first_escape_cut(&format!("{e}\u{301}")), Some(4));
        assert_eq!(last_escape_cut(&format!("{e}\u{301}")), Some(4));
        assert_eq!(first_escape_cut(&format!("\u{600}{e}")), Some(2));
        assert_eq!(last_escape_cut(&format!("\u{600}{e}")), Some(2));
        assert_eq!(first_escape_cut(&format!("\u{600}{e}\u{301}")), Some(2));
        assert_eq!(last_escape_cut(&format!("\u{600}{e}\u{301}")), Some(6));
        assert_eq!(first_escape_cut("e\u{301}"), None);
        let s = format!("a\u{600}{e}\u{301}\u{301}b");
        let parts: Vec<_> = str_graphemes(&s).collect();
        assert_eq!(
            parts,
            [
                (0, "a"),
                (1, "\u{600}"),
                (3, e),
                (7, "\u{301}\u{301}"),
                (11, "b")
            ]
        );
    }

    #[test]
    fn char_境界でない位置からの移動() {
        let x = t("あい");
        assert_eq!(next_grapheme_boundary(&x, ByteOff(1)), ByteOff(3));
        assert_eq!(prev_grapheme_boundary(&x, ByteOff(4)), ByteOff(3));
        assert_eq!(next_grapheme_boundary(&x, ByteOff(4)), ByteOff(6));
        assert_eq!(prev_grapheme_boundary(&x, ByteOff(2)), ByteOff(0));
    }
}
