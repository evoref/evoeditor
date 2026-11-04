#![no_main]

use libfuzzer_sys::fuzz_target;

// 初期文書 + ビューポート + 操作の列をヘッドレスの Editor に流し、各操作の後の Frame と状態の不変条件を検査する
// (eve_testkit::fuzzing。docs/09 §3.3 の fuzz_editor)
fuzz_target!(|data: &[u8]| {
    eve_testkit::fuzzing::editor_sequence(data);
});
