//! つながりの検出 (`MergeScanner`、docs/03 §8.3.1) の性質 (docs/09 §3.3 の `fuzz_encode` の M23 の性質と、§4 の「不正バイトを
//! 含む文書の編集」(a))。局所の検査が 0 件 ⇔ 書いた BOM を除いた出力の全体を同じ文字コードで復号すると本文に一致する。
//! 総当たり (長さ 3 まで) と乱数で確かめる。cargo-fuzz のターゲット (`fuzz/fuzz_targets/fuzz_encode_merge.rs`) と同じ本体。

use eve_encoding::{Encoding, escape_byte};
use proptest::prelude::*;

#[path = "common/merge_check.rs"]
mod merge_check;

// verifies: AC-NFR-304-5
#[test]
fn 長さ_3_までの総当たりで_局所の検査と全体の復号が一致する() {
    // エスケープ文字 (つながりやすい先頭・継続・BOM のバイト) と、つながりの相手になる文字
    let escapes = [
        0x80u8, 0x81, 0x8E, 0x8F, 0x9F, 0xA1, 0xBB, 0xBF, 0xE3, 0xEF, 0xFE, 0xFF,
    ];
    let mut alphabet: Vec<String> = escapes
        .iter()
        .map(|&b| escape_byte(b).to_string())
        .collect();
    alphabet.extend(["a", "@", "\n", "þ", "！", "あ"].map(str::to_owned));
    let n = alphabet.len();
    let (mut merged, mut clean) = (0u32, 0u32);
    for enc in Encoding::ALL {
        for bom in [false, true] {
            if bom && !(enc == Encoding::Utf8 || enc.is_utf16()) {
                continue;
            }
            for len in 0..=3usize {
                for k in 0..n.pow(u32::try_from(len).unwrap()) {
                    let mut rest = k;
                    let mut text = String::new();
                    for _ in 0..len {
                        text.push_str(&alphabet[rest % n]);
                        rest /= n;
                    }
                    match merge_check::check_text(enc, bom, &text) {
                        Some(0) => clean += 1,
                        Some(_) => merged += 1,
                        None => {}
                    }
                }
            }
        }
    }
    // 検査が空振りしていない (つながる本文も、つながらない本文も、ともに多く通る)
    assert!(merged > 1000 && clean > 1000, "{merged} {clean}");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn 乱数の本文でも_局所の検査と全体の復号が一致する(
        head in proptest::collection::vec(any::<u8>(), 4..5),
        body in proptest::collection::vec(prop_oneof![
            0x20u8..0x7F, Just(b'\n'), 0x80u8..0xFF, Just(0xEFu8), Just(0xBBu8), Just(0xBFu8),
            Just(0x81u8), Just(0x8Fu8), Just(0xFEu8), Just(0xFFu8),
        ], 0..200),
    ) {
        let mut data = head;
        data.extend(body);
        merge_check::merge_pipeline(&data);
    }
}

#[test]
fn 固定の入力() {
    for data in [
        &b""[..],
        b"\x00\x00\x01\x00\xEF\xBB\xBFa",
        b"\x01\x00\x00\x00\xFF\xFEa",
        b"\x02\x00\x01\x00\xFE\xFFa",
        b"\x03\x00\x00\x00\x81@",
        b"\x04\x00\x00\x00\x8Fa\xA1",
    ] {
        merge_check::merge_pipeline(data);
    }
}
