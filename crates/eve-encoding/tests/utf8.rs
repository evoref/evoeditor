//! UTF-8 のデコード / エンコードのプロパティテスト (docs/03-core-design.md §8.3、09 L1)。
//! - チャンクの分け方で結果が変わらない (1 バイト単位でも同じ)。
//! - 任意のバイト列が、復号して符号化すると元のバイト列に戻る (無編集の保存でバイト一致)。

use eve_encoding::{PuaEscapeMode, Utf8Decoder, Utf8Encoder};
use proptest::prelude::*;

fn bytes() -> impl Strategy<Value = Vec<u8>> {
    // 妥当な文字と不正な断片が混ざるようにする
    let piece = prop_oneof![
        3 => proptest::collection::vec(proptest::char::range('a', 'z'), 0..4)
            .prop_map(|v| v.into_iter().collect::<String>().into_bytes()),
        2 => Just("あ".as_bytes().to_vec()),
        1 => Just("😀".as_bytes().to_vec()),
        2 => proptest::collection::vec(any::<u8>(), 1..3),
        1 => Just(b"\xE3\x81".to_vec()),
        1 => Just(b"\xEF\xBB\xBF".to_vec()),
    ];
    proptest::collection::vec(piece, 0..12).prop_map(|v| v.concat())
}

fn decode(chunks: &[&[u8]], skip_bom: bool) -> (String, u32, bool) {
    let mut d = Utf8Decoder::new(skip_bom);
    let mut out = String::new();
    for (i, c) in chunks.iter().enumerate() {
        d.decode(c, i + 1 == chunks.len(), &mut out);
    }
    if chunks.is_empty() {
        d.decode(&[], true, &mut out);
    }
    (out, d.invalid_bytes(), d.has_bom())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    #[test]
    fn チャンクの分け方に依存しない(data in bytes(), cuts in proptest::collection::vec(0usize..64, 0..6)) {
        let whole = decode(&[&data], true);
        // 1 バイト単位
        let singles: Vec<&[u8]> = data.chunks(1).collect();
        prop_assert_eq!(&decode(&singles, true), &whole);
        // ランダムな位置
        let mut points: Vec<usize> = cuts.iter().map(|c| c % (data.len() + 1)).collect();
        points.sort_unstable();
        points.dedup();
        let mut parts: Vec<&[u8]> = Vec::new();
        let mut prev = 0;
        for p in points {
            parts.push(&data[prev..p]);
            prev = p;
        }
        parts.push(&data[prev..]);
        prop_assert_eq!(&decode(&parts, true), &whole);
    }

    #[test]
    fn 復号して符号化すると元のバイト列に戻る(data in bytes()) {
        let mut d = Utf8Decoder::new(true);
        let mut s = String::new();
        d.decode(&data, true, &mut s);
        // 真の私用文字を含まない入力なので、エスケープは有効
        prop_assert_eq!(d.pua_mode(), PuaEscapeMode::Enabled);
        let mut e = Utf8Encoder::new(d.has_bom(), d.pua_mode());
        let expected_len = e.encoded_len(&s);
        let mut out = Vec::new();
        e.encode(&s, &mut out);
        prop_assert_eq!(&out, &data);
        prop_assert_eq!(expected_len, out.len(), "乾式走査の長さと一致する");
    }
}
