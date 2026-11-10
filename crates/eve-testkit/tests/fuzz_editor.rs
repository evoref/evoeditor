//! `fuzz_editor` の本体 (`eve_testkit::fuzzing::editor_sequence`) を、決定的な乱数で作った入力で回す (docs/09 §3.3、NFR-305)。
//! libFuzzer は MSVC 非対応なので、Windows の CI ではこれで同じ不変条件を確かめる。本格の探索は Linux の夜間の `fuzz_editor`。
//! 回数は `EVE_FUZZ_EDITOR_CASES` (既定 300) で変えられる。
#![expect(
    clippy::cast_possible_truncation,
    reason = "テスト用の数値変換。値は小さく収まる"
)]

use eve_testkit::fuzzing::editor_sequence;

/// xorshift64* (依存を増やさない決定的な乱数)。
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| (self.next() >> 24) as u8).collect()
    }
}

// verifies: NFR-305
#[test]
fn 任意の入力列でパニックせず_frame_と状態の不変条件を保つ() {
    let cases: u64 = std::env::var("EVE_FUZZ_EDITOR_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);
    // 日本語・制御文字・CRLF・不正な UTF-8 を混ぜた種の文書
    let seeds: [&[u8]; 4] = [
        b"hello\r\nworld\r\n",
        "日本語の行\n\t字下げ\n😀👨‍👩‍👧\n".as_bytes(),
        b"\x00\x1b[31m\x7f\xc2\x80\xff\xfe end",
        b"",
    ];
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for i in 0..cases {
        let seed = seeds[(i % seeds.len() as u64) as usize];
        let mut data = Vec::new();
        data.extend_from_slice(&(seed.len() as u16).to_le_bytes());
        data.extend_from_slice(seed);
        let n = 2 + (rng.next() % 400) as usize;
        data.extend(rng.bytes(n));
        let r = std::panic::catch_unwind(|| editor_sequence(&data));
        assert!(
            r.is_ok(),
            "ケース {i} で失敗した。入力 (16 進): {}",
            hex(&data)
        );
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// コミットした最小の corpus (`fuzz/corpus/fuzz_editor/`、docs/review/v0.3-findings.md の STR-T8) を毎回流す。夜間の fuzz の
/// 出発点で、引数付きのコマンド・外部の変更・ジョブの交互の進行・長大行の閾値を下げた設定に最初から届かせる。
#[test]
fn コミットした_corpus_の入力でも不変条件を保つ() {
    let dir =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/corpus/fuzz_editor");
    let mut n = 0;
    for e in std::fs::read_dir(&dir).expect("corpus がある") {
        let path = e.unwrap().path();
        editor_sequence(&std::fs::read(&path).unwrap());
        n += 1;
    }
    assert!(n >= 6, "corpus は 6 件以上: {n}");
}
