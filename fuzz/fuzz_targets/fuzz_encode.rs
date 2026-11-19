#![no_main]

use libfuzzer_sys::fuzz_target;

// つながりの検出 (MergeScanner、docs/03 §8.3.1) の不変条件 (docs/09 §3.3 の fuzz_encode の M23 の性質)。
// 本体は eve-encoding の tests と共有する (CI の性質テスト: crates/eve-encoding/tests/merge_property.rs)。
// 夜間の fuzz の対象は数を絞っている (xtask/src/nightly.rs の FUZZ_TARGETS。夜間の合計 80 分がジョブの上限に収まるように) ので、
// 別の対象にせず、符号化の fuzz と同じ入力で回す。
#[path = "../../crates/eve-encoding/tests/common/merge_check.rs"]
mod merge_check;

// 任意の文字列 x 6 文字コード x 3 方針の符号化 (eve_encoding::fuzzing::encode_pipeline の性質の検査) と、
// つながりの検出の不変条件
fuzz_target!(|data: &[u8]| {
    eve_encoding::fuzzing::encode_pipeline(data);
    merge_check::merge_pipeline(data);
});
