#![no_main]

use libfuzzer_sys::fuzz_target;

// `.editorconfig` の読込と glob の照合 (docs/09 §3.3 の fuzz_editorconfig、07 §10.3 の EC-3 / EC-4)。入力は「本文とパスの境目 +
// `.editorconfig` の本文 + 照らすパス」(eve_core::editorconfig_fuzzing。本体は eve-config)。パニックしない・照合の手数が上限に
// 収まる・決定的・選択肢の展開と一致・使わないセクションがほかに影響しないことを確かめる。CI では同じ関数を
// crates/eve-config/tests/fuzz_editorconfig.rs の proptest で回す。
fuzz_target!(|data: &[u8]| {
    eve_core::editorconfig_fuzzing::editorconfig(data);
});
