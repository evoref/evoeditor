#![no_main]

use libfuzzer_sys::fuzz_target;

// 正規表現の検索 (docs/09 §3.3 の fuzz_regex_search、03 §12.3.1)。入力は「検索の条件 + 窓の核の大きさ + 連結の上限 + 時計 +
// 検索語の構文木 + 文書 + 選択範囲 + チャンクの分け方」(eve_core::search_fuzzing)。行の窓と全文の連結の経路を、全文を連結した
// 参照と、CRLF と CR を LF にそろえた文書での 2 つ目の参照に突き合わせる。CI では同じ関数を
// crates/eve-core/tests/fuzz_regex_search.rs の proptest で回す。
fuzz_target!(|data: &[u8]| {
    eve_core::search_fuzzing::regex_search(data);
});
