//! 書記素境界 (docs/03-core-design.md §4.1 / §13.1)。拡張書記素クラスタ (UAX #29) を `unicode-segmentation` で判定する。
//!
//! 判定は位置の前後 `CONTEXT` バイトを連結した小さなバッファの上で行う (§13.1)。それより長い結合列は正確さを諦める
//! (バッファの端を境界として扱う)。地域指示記号 (国旗) の対の偶奇も同じ制約を受ける (§14 の既知の制約)。

use unicode_segmentation::GraphemeCursor;

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

/// `pos` が書記素境界か。文書の先頭と末尾は境界。char 境界でない位置は `false`。範囲外は `false`。
pub fn is_grapheme_boundary(text: &dyn TextRead, pos: Pos) -> bool {
    let len = text.len_bytes();
    if pos.0 > len || !text.is_char_boundary(pos) {
        return false;
    }
    if pos.0 == 0 || pos.0 == len {
        return true;
    }
    with_window(text, pos.0, |buf, rel, _| {
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
    let next = with_window(text, p, |buf, rel, lo| {
        let mut cur = GraphemeCursor::new(rel, buf.len(), true);
        cur.next_boundary(buf, 0).ok().flatten().map(|b| lo + b)
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
    let prev = with_window(text, p, |buf, rel, lo| {
        let mut cur = GraphemeCursor::new(rel, buf.len(), true);
        cur.prev_boundary(buf, 0).ok().flatten().map(|b| lo + b)
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

    #[test]
    fn char_境界でない位置からの移動() {
        let x = t("あい");
        assert_eq!(next_grapheme_boundary(&x, ByteOff(1)), ByteOff(3));
        assert_eq!(prev_grapheme_boundary(&x, ByteOff(4)), ByteOff(3));
        assert_eq!(next_grapheme_boundary(&x, ByteOff(4)), ByteOff(6));
        assert_eq!(prev_grapheme_boundary(&x, ByteOff(2)), ByteOff(0));
    }
}
