#![no_main]

use libfuzzer_sys::fuzz_target;

// 任意のバイト列を任意の位置で区切った読取の VT 入力の解析 (eve_tui::vt_input::fuzz_pipeline)
fuzz_target!(|data: &[u8]| {
    eve_tui::vt_input::fuzz_pipeline(data);
});
