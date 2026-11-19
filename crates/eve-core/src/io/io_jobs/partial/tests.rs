//! 先頭の切り方 (03 §9.2.1 の「先頭」) と、先頭の複写の送る時機の試験。読込ジョブを通した試験 (全文の接頭辞・時間の
//! 条件・判定のやり直し・文字コード) は `tests/open_partial.rs`。

use eve_text::{Bias, Text, TextRead, prev_grapheme_boundary, snap_to_grapheme};
use proptest::prelude::*;

use super::*;

const H: usize = PARTIAL_HEAD;

/// `n` バイトの `x` (行の終端の無い長大行)。
fn xs(n: usize) -> String {
    "x".repeat(n)
}

#[test]
fn 行の区切り_lf_crlf_cr() {
    assert_eq!(head_cut("a\nb\nccc", false), 4, "LF の直後");
    assert_eq!(head_cut("a\r\nbb", false), 3, "CRLF の直後");
    assert_eq!(head_cut("a\rbb", false), 2, "後ろが LF でない CR の直後");
    assert_eq!(head_cut("a\nb\rc\nd", false), 6, "混在でも最後の終端");
}

#[test]
fn 末尾の_cr_は行の終端に数えない() {
    // 次のチャンクの先頭が LF かもしれない
    assert_eq!(head_cut("a\nb\r", false), 2);
    assert_eq!(head_cut("a\r\nb\r", false), 3);
    // 行の終端が無ければ書記素の境界まで戻し、最後の書記素 (CR) を含めない
    assert_eq!(head_cut("ab\r", false), 2);
    assert_eq!(head_cut("\r", false), 0);
    // ファイルの末尾まで読み終えていれば、末尾の CR も行の終端で、全部が先頭
    assert_eq!(head_cut("a\nb\r", true), 4);
    assert_eq!(head_cut("abc", true), 3);
    assert_eq!(head_cut("", false), 0);
    assert_eq!(head_cut("", true), 0);
}

#[test]
fn 一_mib_の中の最後の行の終端で切る() {
    let s = format!("{}\n{}", xs(H - 10), xs(100));
    assert_eq!(head_cut(&s, false), H - 9);
    // 1 MiB の位置のちょうど手前の LF
    let s = format!("{}\n{}", xs(H - 1), xs(10));
    assert_eq!(head_cut(&s, false), H);
    // 1 MiB の位置の直後の LF は 1 MiB の外
    let s = format!("a\n{}\n", xs(H - 2));
    assert_eq!(head_cut(&s, false), 2);
}

#[test]
fn 一_mib_をまたぐ_crlf_は分けない() {
    // CR が 1 MiB の中の最後のバイト、LF が外: CRLF は 1 MiB の中で終わらないので、その前の終端で切る
    let s = format!("a\n{}\r\nb", xs(H - 3));
    assert_eq!(head_cut(&s, false), 2);
    // 前に終端が無ければ書記素の境界まで戻し (CR と LF の間は境界でない)、CR の前で切る
    let s = format!("{}\r\nb", xs(H - 1));
    assert_eq!(head_cut(&s, false), H - 1);
    // 1 MiB の中の最後のバイトの CR で、次が LF でないと分かっている: 行の終端
    let s = format!("{}\rb", xs(H - 1));
    assert_eq!(head_cut(&s, false), H);
}

#[test]
fn 長大行は書記素の境界まで戻す_結合文字() {
    // 1 MiB の位置が基底 e と結合文字の間
    let s = format!("{}e\u{301}zz", xs(H - 1));
    assert_eq!(head_cut(&s, false), H - 1);
    // 1 MiB の位置が境界なら、そこで切る
    let s = format!("{}zz", xs(H));
    assert_eq!(head_cut(&s, false), H);
    // 1 MiB の位置が char の途中 (あ は 3 バイト): char の境界へ戻す
    let s = format!("{}あzz", xs(H - 1));
    assert_eq!(head_cut(&s, false), H - 1);
}

#[test]
fn 長大行は書記素の境界まで戻す_絵文字の列と国旗() {
    let family = "👨\u{200D}👩\u{200D}👧";
    let s = format!("{}{family}zz", xs(H - 5));
    assert_eq!(head_cut(&s, false), H - 5, "ZWJ 列の先頭へ戻す");
    // 国旗 (地域指示記号の対) の対の間で 1 MiB を迎える: 対の前へ戻す
    let s = format!("{}🇯🇵🇺🇸zz", xs(H - 4));
    assert_eq!(head_cut(&s, false), H - 4);
    // 対と対の間なら、そこで切る
    let s = format!("{}🇯🇵🇺🇸zz", xs(H - 8));
    assert_eq!(head_cut(&s, false), H);
}

#[test]
fn 長大行は書記素の境界まで戻す_不正バイトのエスケープ文字() {
    // エスケープ文字の前後は常に境界 (03 §8.3.1)。後ろに結合文字が続いても、エスケープ文字の直後で切れる
    let s = format!("{}\u{10FF80}\u{301}zz", xs(H - 4));
    assert_eq!(head_cut(&s, false), H);
    // エスケープ文字の途中の 1 MiB: エスケープ文字の前
    let s = format!("{}\u{10FF80}zz", xs(H - 2));
    assert_eq!(head_cut(&s, false), H - 2);
}

#[test]
fn 復号済みの末尾が_1_mib_より前なら最後の書記素を含めない() {
    assert_eq!(head_cut("abce", false), 3);
    assert_eq!(head_cut("abe\u{301}", false), 2, "結合文字が続いていても");
    // 後ろに結合する文字が続くかもしれないので、境界に見えても含めない
    assert_eq!(head_cut("ab\u{10FF80}", false), 2);
}

#[test]
fn 複写は_1_mib_を超えたら持たない_送ったら手放す() {
    let mut h = HeadCopy::new(Tick(0), false);
    h.feed(&xs(H));
    h.feed("y");
    h.feed("z");
    assert_eq!(h.buf.len(), H + 1, "1 MiB を超えた所 (次の char) まで");
    assert!(!h.due(Tick(199), true), "200 ms の前");
    assert!(!h.due(Tick(500), false), "読み終えていれば送らない");
    assert!(h.due(Tick(200), true));
    let params = HeadParams {
        encoding: Encoding::Utf8,
        has_bom: false,
        pua_escape: PuaEscapeMode::Enabled,
        invalid_so_far: 0,
        long_line_threshold: 1024,
        default_eol: LineEnding::Lf,
    };
    let (text, format) = h.take(false, &params);
    assert_eq!(text.len_bytes(), H);
    assert_eq!(format.long_lines, 1, "先頭の分の長大行");
    assert!(h.sent());
    assert_eq!(h.buf, "");
    assert!(!h.due(Tick(900), true), "1 回だけ");
    h.feed("more");
    assert_eq!(h.buf, "");
}

#[test]
fn やり直しの読込は先頭が揃った時点で送る() {
    let mut h = HeadCopy::new(Tick(0), true);
    assert!(h.resend_pending());
    h.feed(&xs(H));
    assert!(!h.due(Tick(0), true), "1 MiB ちょうどでは次の char が無い");
    h.feed("y");
    assert!(h.due(Tick(0), false), "時間・残りに依らない");
}

#[test]
fn 先頭の形式は先頭の分を数える() {
    let s = "a\r\nb\n\u{10FF80}c\rd";
    let mut h = HeadCopy::new(Tick(0), false);
    h.feed(s);
    let params = HeadParams {
        encoding: Encoding::ShiftJis,
        has_bom: false,
        pua_escape: PuaEscapeMode::Enabled,
        invalid_so_far: 5,
        long_line_threshold: 1024,
        default_eol: LineEnding::Lf,
    };
    let (text, f) = h.take(false, &params);
    assert_eq!(
        text.slice_to_string(eve_text::ByteRange::all(text.len_bytes())),
        "a\r\nb\n\u{10FF80}c\r"
    );
    assert_eq!(
        (f.eol_stats.crlf, f.eol_stats.lf, f.eol_stats.cr),
        (1, 1, 1)
    );
    assert_eq!(f.invalid_bytes, 1, "先頭のエスケープ文字の数");
    assert_eq!(f.escape_encoding, Some(Encoding::ShiftJis));
    assert_eq!(f.encoding, Encoding::ShiftJis);
}

/// 先頭の切り方の素朴な実装 (全文の上で書記素の境界を求める)。`head` は 1 MiB の代わりの大きさ。
fn reference_cut(s: &str, complete: bool, head: usize) -> usize {
    let b = s.as_bytes();
    let mut limit = head.min(s.len());
    while !s.is_char_boundary(limit) {
        limit -= 1;
    }
    if complete && limit == s.len() {
        return limit;
    }
    // 位置 i の直前が、分かっている行の終端か
    let line_end = |i: usize| match b[i - 1] {
        b'\n' => true,
        b'\r' => match b.get(i) {
            Some(&c) => c != b'\n',
            None => complete,
        },
        _ => false,
    };
    if let Some(i) = (1..=limit).rev().find(|&i| line_end(i)) {
        return i;
    }
    let full = Text::from(s);
    let mut cut = if limit == s.len() {
        prev_grapheme_boundary(&full, ByteOff(limit)).0
    } else {
        snap_to_grapheme(&full, ByteOff(limit), Bias::Before).0
    };
    if cut > 0 && b[cut - 1] == b'\r' {
        cut -= 1;
    }
    cut
}

fn any_char() -> impl Strategy<Value = char> {
    prop_oneof![
        8 => proptest::char::range('a', 'c'),
        2 => Just('\r'),
        2 => Just('\n'),
        2 => Just('あ'),
        2 => Just('\u{301}'),
        1 => Just('\u{3099}'),
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

/// 行の終端の無い長い部分 (窓の大きさ 2 × `GRAPHEME_CONTEXT` を超える長さを作るため)。
fn any_run() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        prop_oneof![
            6 => proptest::char::range('a', 'c'),
            1 => Just('\u{301}'),
            1 => Just('\u{1F1EF}'),
            1 => Just('\u{200D}'),
            1 => Just('👨'),
        ],
        0..700,
    )
    .prop_map(|v| v.into_iter().collect())
}

proptest! {
    /// 先頭は行の境界か書記素の境界で切れ、CR と LF の間で切れない。小さな窓での判定は全文での判定と同じ。
    #[test]
    fn 切る位置は行の境界か書記素の境界で_cr_と_lf_の間でない(
        pre in any_run(),
        tail in proptest::collection::vec(any_char(), 0..120),
        complete in any::<bool>(),
        head_seed in any::<usize>(),
    ) {
        let s: String = pre.chars().chain(tail).collect();
        // 本文の長さを少し超える所まで
        let head = head_seed % (s.len() + 16);
        let cut = head_cut_at(&s, complete, head);
        prop_assert_eq!(cut, reference_cut(&s, complete, head));
        prop_assert!(s.is_char_boundary(cut));
        prop_assert!(cut <= head.min(s.len()));
        let b = s.as_bytes();
        prop_assert!(
            !(cut > 0 && cut < s.len() && b[cut - 1] == b'\r' && b[cut] == b'\n'),
            "CR と LF の間で切らない"
        );
        let full = Text::from(s.as_str());
        let at_line = cut == 0
            || b[cut - 1] == b'\n'
            || (b[cut - 1] == b'\r' && (cut < s.len() || complete))
            || (complete && cut == s.len());
        prop_assert!(at_line || eve_text::is_grapheme_boundary(&full, ByteOff(cut)));
    }
}
