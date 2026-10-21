#![no_main]

use libfuzzer_sys::fuzz_target;

// 判定 → 復号 → 往復の検査。入力は「文字コードのラベル + sniff 長 + チャンク分割 + 本文」(eve_encoding::fuzzing)
fuzz_target!(|data: &[u8]| {
    eve_encoding::fuzzing::decode_pipeline(data);
});
