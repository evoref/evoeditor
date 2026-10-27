//! `fuzz_decode` の本体を proptest で回す (docs/09 §3.3。Windows の CI では libFuzzer が使えないので、同じ関数をここで検査する)。

use eve_encoding::fuzzing::decode_pipeline;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn 任意のバイト列でパニックしない(data in proptest::collection::vec(any::<u8>(), 0..600)) {
        decode_pipeline(&data);
    }

    #[test]
    fn 日本語らしい_8_bit_と_esc_を多く含む列でもパニックしない(
        head in proptest::collection::vec(any::<u8>(), 4..5),
        body in proptest::collection::vec(prop_oneof![
            Just(0x1Bu8), Just(b'$'), Just(b'B'), Just(b'('), Just(b'I'), Just(0x0Au8),
            0x81u8..0xA0, 0xA1u8..0xFE, 0x40u8..0x7F, 0x21u8..0x7F,
        ], 0..400),
    ) {
        let mut data = head;
        data.extend(body);
        decode_pipeline(&data);
    }
}

#[test]
fn 固定の入力() {
    for data in [
        &b""[..],
        b"abc",
        b"\x00\x00\x00\x00\xEF\xBB\xBFabc",
        b"\x00\x00\x00\x00\xFF\xFEa\x00\x00\xD8",
        b"\x03\x00\x00\x00\x93\xFA\x96\x7B",
        b"\x00\x00\x00\x00\x1B$B$\"\x1B(B\n",
        b"\x06\x00\x00\x01\x1B$B$\"\n\x1B(B\x80",
    ] {
        decode_pipeline(data);
    }
}

/// 2026-10-22 の夜間の fuzz が見つけた timeout (課題 #78)。UTF-16LE を 2 バイトずつ復号すると、孤立した上位サロゲートの続きで
/// `decode_to_string_without_replacement` が `OutputFull` を返し続け、無限ループになっていた。生成データなので、そのまま回帰に入れる。
#[test]
fn fuzz_が見つけた_utf16_の_timeout_は終わる() {
    let data = include_bytes!(
        "../../../fuzz/corpus/fuzz_decode/regress_utf16_lone_surrogates_output_full"
    );
    // 別スレッドで走らせ、10 秒で終わらなければ失敗にする (ハングでテスト全体を止めない)
    let (tx, rx) = std::sync::mpsc::channel();
    #[allow(clippy::disallowed_methods)]
    std::thread::spawn(move || {
        decode_pipeline(data);
        let _ = tx.send(());
    });
    assert!(
        rx.recv_timeout(std::time::Duration::from_secs(10)).is_ok(),
        "decode_pipeline が 10 秒たっても終わらない"
    );
}
