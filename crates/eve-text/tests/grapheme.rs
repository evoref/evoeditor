//! 書記素境界のプロパティテスト: 文脈の上限 (256 バイト) より短い文書では、文書全体を `unicode-segmentation` で
//! 分割した結果と一致する (docs/03-core-design.md §13.1)。

use eve_text::{
    Bias, ByteOff, Text, TextRead, is_grapheme_boundary, next_grapheme_boundary,
    prev_grapheme_boundary, snap_to_grapheme,
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
