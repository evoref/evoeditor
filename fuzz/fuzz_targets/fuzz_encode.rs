#![no_main]

use libfuzzer_sys::fuzz_target;

// 任意の文字列 x 6 文字コード x 3 方針の符号化 (eve_encoding::fuzzing::encode_pipeline の性質の検査)
fuzz_target!(|data: &[u8]| {
    eve_encoding::fuzzing::encode_pipeline(data);
});
