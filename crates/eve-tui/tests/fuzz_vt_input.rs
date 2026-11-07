//! `fuzz_vt_input` の本体を proptest で回す (docs/review/v0.3-findings.md の §2.7 の 8。libFuzzer は MSVC 非対応なので、
//! CI ではこの試験で同じ関数を検査し、Linux の夜間ジョブで fuzz する)。
#![expect(
    clippy::cast_possible_truncation,
    reason = "テスト用の数値変換。値は小さく収まる"
)]

use eve_tui::vt_input::fuzz_pipeline;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn 任意のバイト列を任意の位置で区切ってもパニックしない(data in proptest::collection::vec(any::<u8>(), 0..400)) {
        fuzz_pipeline(&data);
    }

    #[test]
    fn エスケープの列に近いバイト列でもパニックしない(
        head in proptest::collection::vec(any::<u8>(), 0..8),
        body in proptest::collection::vec(prop_oneof![
            Just(0x1Bu8), Just(b'['), Just(b'O'), Just(b'<'), Just(b';'), Just(b'~'), Just(b'u'),
            Just(b'M'), Just(b'm'), Just(b'I'), b'0'..=b'9', 0x40u8..0x7F, 0x80u8..0xFF, 0x00u8..0x20,
        ], 0..400),
    ) {
        let mut data = vec![head.len() as u8];
        data.extend(head);
        data.extend(body);
        fuzz_pipeline(&data);
    }
}

#[test]
fn 大きな数の引数と_200_の貼り付けの開始だけでも終わる() {
    let mut data = vec![0u8];
    data.extend(b"\x1b[99999999999999999999;99999999999999999999H\x1b[<99999999999;99999999999;99999999999M\x1b[200~abc");
    fuzz_pipeline(&data);
}
