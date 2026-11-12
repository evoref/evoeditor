//! 書記素境界のプロパティテスト: 文脈の上限 (256 バイト) より短い文書では、文書全体を `unicode-segmentation` で
//! 分割した結果と一致する (docs/03-core-design.md §13.1)。不正バイトのエスケープ文字 (`U+10FF00..=U+10FFFF`) を
//! 混ぜた文書では、その前後を必ず境界とし、それ以外は `unicode-segmentation` と一致する (§13.1 / §8.3.1、§22 の
//! 「書記素とエスケープ文字」)。

use std::collections::BTreeSet;

use eve_text::{
    Bias, ByteOff, ByteRange, Text, TextRead, first_grapheme, grapheme_indices,
    is_grapheme_boundary, next_grapheme_boundary, prev_grapheme_boundary, snap_to_grapheme,
    str_display_width,
};
use proptest::prelude::*;
use unicode_segmentation::UnicodeSegmentation;

fn any_char() -> impl Strategy<Value = char> {
    prop_oneof![
        4 => proptest::char::range('a', 'f'),
        2 => Just('\r'),
        2 => Just('\n'),
        2 => Just('\u{301}'),
        2 => Just('\u{200D}'),
        1 => Just('\u{FE0F}'),
        2 => Just('あ'),
        2 => Just('👨'),
        2 => Just('👩'),
        2 => Just('\u{1F1EF}'),
        2 => Just('\u{1F1F5}'),
        1 => Just('\u{1F3FD}'),
        1 => Just('한'),
        1 => Just('\u{1100}'),
        1 => Just('\u{1161}'),
    ]
}

fn any_string() -> impl Strategy<Value = String> {
    proptest::collection::vec(any_char(), 0..40).prop_map(|v| v.into_iter().collect())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    #[test]
    fn 全文の分割と一致する(s in any_string()) {
        let text = Text::from(s.as_str());
        let expected: Vec<usize> = s.grapheme_indices(true).map(|(i, _)| i).chain([s.len()]).collect();
        let actual: Vec<usize> = (0..=s.len()).filter(|&i| is_grapheme_boundary(&text, ByteOff(i))).collect();
        prop_assert_eq!(&actual, &expected);

        // next / prev は隣の境界へ動く
        for w in expected.windows(2) {
            prop_assert_eq!(next_grapheme_boundary(&text, ByteOff(w[0])), ByteOff(w[1]));
            prop_assert_eq!(prev_grapheme_boundary(&text, ByteOff(w[1])), ByteOff(w[0]));
        }
    }

    #[test]
    fn スナップは最も近い境界の前後を返す(s in any_string(), at in 0usize..200) {
        let text = Text::from(s.as_str());
        let p = ByteOff(at.min(text.len_bytes()));
        let before = snap_to_grapheme(&text, p, Bias::Before);
        let after = snap_to_grapheme(&text, p, Bias::After);
        prop_assert!(before <= p && p <= after, "{before:?} {p:?} {after:?}");
        prop_assert!(is_grapheme_boundary(&text, before));
        prop_assert!(is_grapheme_boundary(&text, after));
        // before と after の間に別の境界がない
        for i in (before.0 + 1)..after.0 {
            prop_assert!(!is_grapheme_boundary(&text, ByteOff(i)) || i == p.0);
        }
    }
}

// ---------- 不正バイトのエスケープ文字 (v0.4 の M23、03 §13.1 / §8.3.1) ----------

fn is_escape(c: char) -> bool {
    ('\u{10FF00}'..='\u{10FFFF}').contains(&c)
}

/// 参照モデル (03 §22 の「書記素とエスケープ文字」): `unicode-segmentation` の境界に、エスケープ文字の前後を足したもの。
fn reference(s: &str) -> Vec<usize> {
    let mut set: BTreeSet<usize> = s
        .grapheme_indices(true)
        .map(|(i, _)| i)
        .chain([s.len()])
        .collect();
    for (i, c) in s.char_indices() {
        if is_escape(c) {
            set.insert(i);
            set.insert(i + c.len_utf8());
        }
    }
    set.into_iter().collect()
}

/// エスケープ文字の直後・直前で結合し得る文字 (結合文字・異体字セレクタ・ZWJ・Prepend・SpacingMark・ハングルの字母・
/// 地域指示記号・絵文字・CR / LF) と、エスケープ文字を混ぜる。
fn any_char_with_escape() -> impl Strategy<Value = char> {
    prop_oneof![
        3 => proptest::char::range('a', 'c'),
        2 => Just('\r'),
        2 => Just('\n'),
        3 => Just('\u{301}'),
        2 => Just('\u{FE0F}'),
        3 => Just('\u{200D}'),
        1 => Just('\u{0600}'),
        1 => Just('\u{0903}'),
        1 => Just('\u{0915}'),
        1 => Just('\u{094D}'),
        1 => Just('あ'),
        2 => Just('👨'),
        1 => Just('\u{1F3FD}'),
        2 => Just('\u{1F1EF}'),
        1 => Just('\u{1F1F5}'),
        1 => Just('\u{1100}'),
        1 => Just('\u{1161}'),
        1 => Just('\u{11A8}'),
        1 => Just('\u{E000}'),
        1 => Just('\u{10FEFF}'),
        6 => proptest::char::range('\u{10FF00}', '\u{10FFFF}'),
    ]
}

fn string_with_escape(max: usize) -> impl Strategy<Value = String> {
    proptest::collection::vec(any_char_with_escape(), 0..max).prop_map(|v| v.into_iter().collect())
}

/// 前から `next_grapheme_boundary` で、後ろから `prev_grapheme_boundary` で走査した境界の列 (どちらも昇順)。
fn walks(text: &Text) -> (Vec<usize>, Vec<usize>) {
    let len = text.len_bytes();
    let mut fwd = vec![0];
    let mut p = 0;
    while p < len {
        p = next_grapheme_boundary(text, ByteOff(p)).0;
        fwd.push(p);
    }
    let mut back = vec![len];
    let mut p = len;
    while p > 0 {
        p = prev_grapheme_boundary(text, ByteOff(p)).0;
        back.push(p);
    }
    back.reverse();
    (fwd, back)
}

/// 文書の位置の判定・前後の走査・文字列の分割 (`grapheme_indices`) が、どれも参照モデルと一致することを確かめる。
fn check_against_reference(s: &str) -> Result<(), TestCaseError> {
    let text = Text::from(s);
    let want = reference(s);
    let actual: Vec<usize> = (0..=s.len())
        .filter(|&i| is_grapheme_boundary(&text, ByteOff(i)))
        .collect();
    prop_assert_eq!(&actual, &want, "位置の判定");
    let (fwd, back) = walks(&text);
    prop_assert_eq!(&fwd, &want, "前からの走査");
    prop_assert_eq!(&back, &want, "後ろからの走査");
    let split: Vec<usize> = grapheme_indices(s)
        .into_iter()
        .map(|(i, _)| i)
        .chain([s.len()])
        .collect();
    let split = if s.is_empty() { vec![0] } else { split };
    prop_assert_eq!(&split, &want, "文字列の分割");
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    // spec: 03 §22.1 (書記素とエスケープ文字)
    #[test]
    fn エスケープ文字の前後は必ず境界で_それ以外は_unicode_segmentation_と一致する(s in string_with_escape(48)) {
        check_against_reference(&s)?;
        // スナップは参照モデルの境界に寄せる
        let want = reference(&s);
        let text = Text::from(s.as_str());
        for i in 0..=s.len() {
            let before = snap_to_grapheme(&text, ByteOff(i), Bias::Before).0;
            let after = snap_to_grapheme(&text, ByteOff(i), Bias::After).0;
            let k = want.partition_point(|&b| b <= i);
            prop_assert_eq!(before, want[k - 1], "{}", i);
            let k = want.partition_point(|&b| b < i);
            prop_assert_eq!(after, want[k], "{}", i);
        }
    }
}

/// 長い文書 (rope のチャンクが複数になる長さ) の試験の件数。既定は 32 件、`PROPTEST_CASES` があればその 1/20
/// (夜間の 10000 なら 500)。1 件が全位置 × 窓で重いので、`proptest!` のマクロの件数の上書き (Issue #453) を避ける。
fn long_config() -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .map_or(32, |n| (n / 20).max(32));
    ProptestConfig {
        cases,
        source_file: Some(file!()),
        ..ProptestConfig::default()
    }
}

/// チャンクの境目をまたいでも同じ: 先頭に長さの違う ASCII を置いて、エスケープ文字を混ぜた部分を rope のチャンクの
/// 境目 (約 1 KiB ごと) の前後に動かす。
// spec: 03 §22.1 (書記素とエスケープ文字)
#[test]
fn エスケープ文字の境界はチャンクの境目をまたいでも同じ() {
    let mut runner = proptest::test_runner::TestRunner::new(long_config());
    let strategy = (0usize..2200, string_with_escape(600));
    let multi = std::cell::Cell::new(0u32);
    let result = runner.run(&strategy, |(pad, tail)| {
        let s = format!("{}{tail}", "x".repeat(pad));
        let text = Text::from(s.as_str());
        if text.chunks(ByteRange::all(text.len_bytes())).count() > 1 {
            multi.set(multi.get() + 1);
        }
        check_against_reference(&s)
    });
    if let Err(e) = result {
        panic!("{e}\n{runner}");
    }
    assert!(multi.get() > 0, "チャンクが複数になる文書を試していない");
}

/// 具体例 (03 §8.3.1 の「書記素と単語」)。期待する境界は参照モデルと同じで、エスケープ文字の前後で切れる。
// spec: 03 §13.1
#[test]
fn エスケープ文字と結合する文字の具体例() {
    let e = "\u{10FF9A}";
    let f = "\u{10FF80}";
    let cases: &[(String, &[usize], &str)] = &[
        (format!("{e}\u{301}"), &[0, 4, 6], "結合文字 (GB9)"),
        (format!("{e}\u{FE0F}"), &[0, 4, 7], "異体字セレクタ (GB9)"),
        (format!("{e}\u{200D}👨"), &[0, 4, 7, 11], "ZWJ + 絵文字"),
        (
            format!("👨\u{200D}{e}\u{200D}👩"),
            &[0, 7, 11, 14, 18],
            "絵文字の ZWJ 列の途中",
        ),
        (format!("👨\u{200D}{e}👩"), &[0, 7, 11, 15], "ZWJ の直後"),
        (
            format!("\r{e}\n"),
            &[0, 1, 5, 6],
            "CR + エスケープ文字 + LF",
        ),
        (
            format!("\u{1100}{e}\u{1161}\u{11A8}"),
            &[0, 3, 7, 13],
            "ハングルの字母の間",
        ),
        (
            format!("\u{1F1EF}{e}\u{1F1EF}\u{1F1F5}"),
            &[0, 4, 8, 16],
            "地域指示記号の間 (後ろの対は 1 つ)",
        ),
        (
            format!("{e}{f}{e}"),
            &[0, 4, 8, 12],
            "連続するエスケープ文字",
        ),
        (format!("\u{600}{e}"), &[0, 2, 6], "Prepend の直後 (GB9b)"),
        (format!("{e}\u{903}"), &[0, 4, 7], "SpacingMark (GB9a)"),
        (
            format!("a{e}\u{301}\u{301}b"),
            &[0, 1, 5, 9, 10],
            "結合文字の並びは 1 つ",
        ),
    ];
    for (s, want, what) in cases {
        let text = Text::from(s.as_str());
        let got: Vec<usize> = (0..=s.len())
            .filter(|&i| is_grapheme_boundary(&text, ByteOff(i)))
            .collect();
        assert_eq!(&got, want, "{what}: {s:?}");
        assert_eq!(&reference(s), want, "{what}: 参照モデル");
        let (fwd, back) = walks(&text);
        assert_eq!(&fwd, want, "{what}: 前から");
        assert_eq!(&back, want, "{what}: 後ろから");
    }
    // 文字列の先頭の書記素も同じ規則
    assert_eq!(first_grapheme(&format!("{e}\u{301}")), e);
    assert_eq!(first_grapheme(&format!("\u{600}{e}")), "\u{600}");
}

/// `<0x9A>` の上の Backspace (前の境界までを消す) は、直後の結合文字を巻き込まない。結合文字の上の Backspace は結合文字だけを消す。
// spec: 03 §8.3.1 (書記素と単語)
#[test]
fn エスケープ文字の直後の結合文字は別々に消せる() {
    let s = "a\u{10FF9A}\u{301}b";
    let text = Text::from(s);
    // 結合文字の後ろ (7) から 1 つ戻る: 結合文字の前 (5) まで
    assert_eq!(prev_grapheme_boundary(&text, ByteOff(7)), ByteOff(5));
    // エスケープ文字の後ろ (5) から 1 つ戻る: エスケープ文字の前 (1) まで
    assert_eq!(prev_grapheme_boundary(&text, ByteOff(5)), ByteOff(1));
    // Delete (次の境界まで) も同じ単位
    assert_eq!(next_grapheme_boundary(&text, ByteOff(1)), ByteOff(5));
    assert_eq!(next_grapheme_boundary(&text, ByteOff(5)), ByteOff(7));
    // 幅の計算も同じ単位で数える (幅関数ではエスケープ文字は私用文字、孤立した結合文字は ◌ + 結合で 1)
    let cfg = eve_text::WidthConfig {
        tab_width: 4,
        ambiguous_wide: false,
        control_style: eve_text::ControlStyle::Caret,
        emoji_width: eve_text::EmojiWidth::Two,
        show_invisible: true,
    };
    assert_eq!(
        str_display_width("\u{10FF9A}\u{301}", &cfg),
        str_display_width("\u{10FF9A}", &cfg) + 1
    );
}
