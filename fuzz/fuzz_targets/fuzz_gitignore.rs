#![no_main]

use libfuzzer_sys::fuzz_target;

// gitignore の照合 (docs/09 §3.3 の fuzz_gitignore、03 §12.7.3)。入力は「種類・畳み方・置き場所の深さ + 本文とパスの境目 +
// ignore のファイルの本文 + 照らすパス」(eve_core::gitignore_fuzzing)。パニックしない・照合の手数が上限に収まる・決定的・
// ワイルドカードの無い行は完全一致・文字の集合の無い本文の畳み方を確かめる。CI では同じ関数を
// crates/eve-core/tests/fuzz_gitignore.rs の proptest で回す。
fuzz_target!(|data: &[u8]| {
    eve_core::gitignore_fuzzing::gitignore(data);
});
