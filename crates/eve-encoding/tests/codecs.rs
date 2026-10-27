//! 文字コード全体のプロパティテスト (docs/09 §3.3 L1、docs/03 §8.3 / §8.6 / §8.7)。
//!
//! - チャンクの分け方に依存しない (1 バイト単位でも結果が変わらない。encoding_rs の `Malformed` が直前のバッファに属する場合を含む)
//! - UTF-8 / UTF-16LE/BE は任意のバイト列が、復号 → 元のバイトに戻して符号化で **バイト一致** する (区分 A)
//! - Shift_JIS / EUC-JP / ISO-2022-JP は、表現できる文字列が符号化 → 復号で元に戻る
//! - 任意のバイト列でパニックしない (fuzz_decode の入口と同じ性質)

use eve_encoding::{Decoder, EncodeReport, Encoder, Encoding, EscapePolicy, unescape_char};
use proptest::prelude::*;

fn decode(enc: Encoding, bytes: &[u8], cuts: &[usize]) -> (String, u32) {
    let mut d = Decoder::new(enc, false);
    let mut out = String::new();
    let mut at = 0;
    for &c in cuts {
        let end = (at + c).min(bytes.len());
        d.decode(&bytes[at..end], false, &mut out);
        at = end;
    }
    d.decode(&bytes[at..], true, &mut out);
    (out, d.invalid_bytes())
}

fn encode(enc: Encoding, text: &str, policy: EscapePolicy) -> (Vec<u8>, EncodeReport) {
    let mut e = Encoder::new(enc, false, policy);
    let (mut out, mut rep) = (Vec::new(), EncodeReport::default());
    e.encode(text, &mut out, &mut rep);
    e.finish(&mut out, &mut rep);
    (out, rep)
}

const ENCODINGS: [Encoding; 6] = Encoding::ALL;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn チャンクの分け方に依存しない(
        bytes in proptest::collection::vec(any::<u8>(), 0..400),
        cuts in proptest::collection::vec(1usize..9, 0..60),
    ) {
        for enc in ENCODINGS {
            let whole = decode(enc, &bytes, &[]);
            let split = decode(enc, &bytes, &cuts);
            prop_assert_eq!(&whole, &split, "{}", enc.name());
            let ones = decode(enc, &bytes, &vec![1; bytes.len()]);
            prop_assert_eq!(&whole, &ones, "{} (1 バイト単位)", enc.name());
        }
    }

    #[test]
    fn utf8_と_utf16_は任意のバイト列でバイト一致する(bytes in proptest::collection::vec(any::<u8>(), 0..400)) {
        for enc in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
            let (text, _) = decode(enc, &bytes, &[]);
            let (out, rep) = encode(enc, &text, EscapePolicy::Restore);
            // 真に U+10FF00.. を含むバイト列は衝突 (Restore ではバイトに戻ってしまう) なので除く
            let collides = bytes.windows(1).count() > 0 && {
                let mut d = Decoder::new(enc, false);
                let mut s = String::new();
                d.decode(&bytes, true, &mut s);
                d.pua_collision()
            };
            if !collides {
                prop_assert!(rep.is_empty());
                prop_assert_eq!(&out, &bytes, "{}", enc.name());
            }
        }
    }

    #[test]
    fn 不正バイトを含む_shift_jis_と_euc_jp_も重複符号化を除いて戻る(
        bytes in proptest::collection::vec(any::<u8>(), 0..200),
    ) {
        // 不正バイトの位置は必ずバイトに戻る。妥当な部分は同じ文字列に再び復号される (重複符号化のバイト表現は変わり得る)
        for enc in [Encoding::ShiftJis, Encoding::EucJp] {
            let (text, bad) = decode(enc, &bytes, &[]);
            let (out, rep) = encode(enc, &text, EscapePolicy::Restore);
            let (again, bad2) = decode(enc, &out, &[]);
            if rep.is_empty() {
                prop_assert_eq!(&again, &text, "{}", enc.name());
                prop_assert_eq!(bad, bad2);
            }
        }
    }

    #[test]
    fn 任意のバイト列でも_iso_2022_jp_の復号はパニックしない(bytes in proptest::collection::vec(any::<u8>(), 0..300)) {
        let (text, bad) = decode(Encoding::Iso2022Jp, &bytes, &[]);
        let esc = text.chars().filter(|c| unescape_char(*c).is_some()).count() as u32;
        prop_assert_eq!(esc, bad);
        // 状態付き符号化は不正バイトを元に戻せない: 符号化は報告して止まる
        let (_, rep) = encode(Encoding::Iso2022Jp, &text, EscapePolicy::Reject);
        prop_assert!(rep.total >= u64::from(bad));
    }

    #[test]
    fn 表現できる文字列は符号化して復号すると戻る(
        text in "[a-zA-Z0-9 \n\r\t、。あいうえお漢字日本語カタカナ全角１２３]{0,80}",
    ) {
        for enc in ENCODINGS {
            let (bytes, rep) = encode(enc, &text, EscapePolicy::Restore);
            prop_assert!(rep.is_empty(), "{}: {:?}", enc.name(), rep.recorded);
            let (back, bad) = decode(enc, &bytes, &[]);
            prop_assert_eq!(bad, 0, "{}", enc.name());
            prop_assert_eq!(&back, &text, "{}", enc.name());
        }
    }

    #[test]
    fn 符号化はチャンクの分け方に依存しない(
        text in "[a-z0-9\n日本語カナ¥]{0,60}",
        cut in 0usize..60,
    ) {
        for enc in ENCODINGS {
            let whole = encode(enc, &text, EscapePolicy::Restore);
            let at = text.char_indices().map(|(i, _)| i).nth(cut).unwrap_or(text.len());
            let mut e = Encoder::new(enc, false, EscapePolicy::Restore);
            let (mut out, mut rep) = (Vec::new(), EncodeReport::default());
            e.encode(&text[..at], &mut out, &mut rep);
            e.encode(&text[at..], &mut out, &mut rep);
            e.finish(&mut out, &mut rep);
            prop_assert_eq!(&out, &whole.0, "{}", enc.name());
            prop_assert_eq!(rep, whole.1, "{}", enc.name());
        }
    }
}
