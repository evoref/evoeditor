//! 保持方式の記録の個別の例 (docs/03 §8.7.1)。性質テストは `src/preserve/props.rs`。

use eve_encoding::{
    Decoder, Encoding, PreserveLimits, PreserveReason, PreserveState, PreservedRecords,
    RoundtripChecker, RoundtripSummary, check_roundtrip_with_records, escape_byte,
};

fn decode(enc: Encoding, bytes: &[u8]) -> String {
    let mut d = Decoder::new(enc, false);
    let mut s = String::new();
    d.decode(bytes, true, &mut s);
    s
}

/// 記録を (行, 位置, 長さ, 元のバイト列, 理由) で並べる。
fn recs(r: &PreservedRecords) -> Vec<(u64, u64, u32, Vec<u8>, PreserveReason)> {
    r.records
        .iter()
        .map(|x| (x.line, x.col, x.len, r.bytes_of(x).to_vec(), x.reason))
        .collect()
}

fn collect(enc: Encoding, bytes: &[u8]) -> (RoundtripSummary, PreservedRecords) {
    let (s, r) = check_roundtrip_with_records(enc, bytes);
    // どの例も、性質 (不変条件・本文の座標・昇順) を満たす
    eve_encoding::fuzzing::check_preserved(enc, &decode(enc, bytes), &r);
    (s, r)
}

use PreserveReason::{DecodeOnly, DuplicateEncoding, StatefulEscape};

#[test]
fn cp932_の重複符号化は_1_文字の範囲になる() {
    // 0xFA54 (IBM 拡張の ¬ U+FFE2) は再符号化で 0x81CA になる
    let b = b"ab\xFA\x54cd";
    assert_eq!(decode(Encoding::ShiftJis, b), "ab\u{FFE2}cd");
    let (s, r) = collect(Encoding::ShiftJis, b);
    assert_eq!(s.lossy_lines, 1);
    assert_eq!(r.state, PreserveState::Collected);
    assert_eq!(
        recs(&r),
        vec![(0, 2, 3, b"\xFA\x54".to_vec(), DuplicateEncoding)]
    );
}

#[test]
fn 隣り合う_2_文字は_1_件にまとめる() {
    // 0xFA54 (¬) と 0xFA5B (∵ U+2235。再符号化で 0x81E6) が続く。間に ASCII を挟むと別の記録
    let b = b"a\xFA\x54\xFA\x5Bb\xFA\x54";
    assert_eq!(decode(Encoding::ShiftJis, b), "a\u{FFE2}\u{2235}b\u{FFE2}");
    let (_, r) = collect(Encoding::ShiftJis, b);
    assert_eq!(
        recs(&r),
        vec![
            (0, 1, 6, b"\xFA\x54\xFA\x5B".to_vec(), DuplicateEncoding),
            (0, 8, 3, b"\xFA\x54".to_vec(), DuplicateEncoding),
        ]
    );
}

#[test]
fn euc_jp_の補助漢字は復号専用_別のコードに写るものは重複符号化() {
    // JIS X 0212 の 0x8FB0A1 (丂 U+4E02) は符号化できない。0x8FA2B7 (U+FF5E) は再符号化で 0xA1C1 になる
    let b = b"a\x8F\xB0\xA1b\x8F\xA2\xB7";
    assert_eq!(decode(Encoding::EucJp, b), "a\u{4E02}b\u{FF5E}");
    let (s, r) = collect(Encoding::EucJp, b);
    assert_eq!(s.unmappable_lines, 1, "数え方は v0.3 のまま");
    assert_eq!(
        recs(&r),
        vec![
            (0, 1, 3, b"\x8F\xB0\xA1".to_vec(), DecodeOnly),
            (0, 5, 3, b"\x8F\xA2\xB7".to_vec(), DuplicateEncoding),
        ]
    );
    // 復号専用と重複符号化が隣り合うと、1 件にまとめて復号専用にする
    let (_, r) = collect(Encoding::EucJp, b"\x8F\xA2\xB7\x8F\xB0\xA1");
    assert_eq!(
        recs(&r),
        vec![(0, 0, 6, b"\x8F\xA2\xB7\x8F\xB0\xA1".to_vec(), DecodeOnly)]
    );
}

#[test]
fn cp932_の外字と互換文字は記録にならない() {
    // 外字 0xF040 / 0xF9FC は自前の逆算で、互換文字 0x8160 (U+FF5E)・0x81CA (U+FFE2) は encoding_rs で元に戻る (区分 A)
    let b = b"\xF0\x40\xF9\xFC\x81\x60\x81\xCA\n";
    let (s, r) = collect(Encoding::ShiftJis, b);
    assert_eq!(s, RoundtripSummary::default());
    assert_eq!(r.state, PreserveState::Collected);
    assert_eq!(r.records, []);
    // EUC-JP の互換文字 0xA1C1 も同じ
    let (_, r) = collect(Encoding::EucJp, b"\xA1\xC1");
    assert_eq!(r.records, []);
}

#[test]
fn 不正バイトの隣の区分_b_の文字の位置() {
    // 0xFF は不正バイト (PUA の 1 文字、UTF-8 で 4 バイト)。記録の位置は本文の UTF-8 のバイト数で数える
    let b = b"a\xFF\xFA\x54\xA0";
    let body = decode(Encoding::ShiftJis, b);
    assert_eq!(
        body,
        format!("a{}\u{FFE2}{}", escape_byte(0xFF), escape_byte(0xA0))
    );
    let (_, r) = collect(Encoding::ShiftJis, b);
    assert_eq!(
        recs(&r),
        vec![(0, 5, 3, b"\xFA\x54".to_vec(), DuplicateEncoding)]
    );
    assert_eq!(&body[5..8], "\u{FFE2}");
    // 先頭バイトだけの不正バイト (後ろが ASCII で読み直される) の後ろ
    let (_, r) = collect(Encoding::ShiftJis, b"\x81 \xFA\x54");
    assert_eq!(
        recs(&r),
        vec![(0, 5, 3, b"\xFA\x54".to_vec(), DuplicateEncoding)]
    );
}

#[test]
fn iso_2022_jp_の旧_jis_の行と半角カナの行() {
    // 1 行目は旧 JIS (`ESC $ @`)、2 行目は正規形、3 行目は半角カナ (`ESC ( I`)
    let b = b"\x1B$@$\"\x1B(B\n\x1B$B$\"\x1B(B\nx\x1B(I1\x1B(B\n";
    assert_eq!(decode(Encoding::Iso2022Jp, b), "あ\nあ\nxｱ\n");
    let (s, r) = collect(Encoding::Iso2022Jp, b);
    assert_eq!((s.lossy_lines, s.unmappable_lines), (1, 1));
    assert_eq!(r.state, PreserveState::Collected);
    assert_eq!(r.unpreserved, 0);
    assert_eq!(
        recs(&r),
        vec![
            (0, 0, 3, b"\x1B$@$\"\x1B(B".to_vec(), StatefulEscape),
            (2, 0, 4, b"x\x1B(I1\x1B(B".to_vec(), DecodeOnly),
        ]
    );
}

#[test]
fn iso_2022_jp_のローマ字状態が改行をまたぐ行は記録にならない() {
    // 0 行目はローマ字状態で終わる (ASCII 状態で終わらない)。1 行目はローマ字状態で始まる (0x5C は ¥)。
    // 2 行目もローマ字状態で始まる (行頭の `ESC ( B` は正規形では書かれない)。3 行目は ASCII 状態で始まる旧 JIS の行
    let b = b"\x1B(J\\\nabc\\\n\x1B(Bxyz\n\x1B$@$\"\x1B(B\nok\n";
    assert_eq!(decode(Encoding::Iso2022Jp, b), "¥\nabc¥\nxyz\nあ\nok\n");
    let (s, r) = collect(Encoding::Iso2022Jp, b);
    assert_eq!(r.state, PreserveState::Collected);
    assert_eq!(r.unpreserved, 3);
    assert_eq!(
        recs(&r),
        vec![(3, 0, 3, b"\x1B$@$\"\x1B(B".to_vec(), StatefulEscape)]
    );
    // 数え方は v0.3 のまま (行ごとに ASCII 状態から復号する。1 行目は ASCII だけなので数えない)
    assert_eq!(s.lossy_lines, 3);
    // ローマ字状態で始まっても、差の無い行は数えない
    let (_, r) = collect(Encoding::Iso2022Jp, b"\x1B(J\\\nabc\n");
    assert_eq!(r.unpreserved, 1);
}

#[test]
fn iso_2022_jp_の冗長なエスケープだけの行は内容が空の記録になる() {
    // stress_hang の全探索が見つけた: `ESC ( B` だけの行は、復号すると空で、再符号化すると何も書かない (差がある)。
    // 行の記録 (長さ 0) にして、元のバイト列のまま書けるようにする
    let b = b"\x1B(B\nab\n\x1B(B";
    assert_eq!(decode(Encoding::Iso2022Jp, b), "\nab\n");
    let (s, r) = collect(Encoding::Iso2022Jp, b);
    assert_eq!(s.lossy_lines, 2);
    assert_eq!(r.unpreserved, 0);
    assert_eq!(
        recs(&r),
        vec![
            (0, 0, 0, b"\x1B(B".to_vec(), StatefulEscape),
            (2, 0, 0, b"\x1B(B".to_vec(), StatefulEscape),
        ]
    );
    // 性質テストが見つけた: CR で終わった行の次の、エスケープだけで LF で終わる行は、本文では CRLF の中に入る (行に
    // ならない)。記録にできないので保持できなかった箇所に数え、後ろの行番号は本文に合わせる
    let b = b"x\r\x1B(B\n\x1B$@$\"\x1B(B\n";
    assert_eq!(decode(Encoding::Iso2022Jp, b), "x\r\nあ\n");
    let (_, r) = collect(Encoding::Iso2022Jp, b);
    assert_eq!(r.unpreserved, 1);
    assert_eq!(
        recs(&r),
        vec![(1, 0, 3, b"\x1B$@$\"\x1B(B".to_vec(), StatefulEscape)]
    );
}

#[test]
fn iso_2022_jp_の不正バイトを含む文書は集めない() {
    // JIS X 0208 状態の改行は不正バイト (03 §8.3)
    let (_, r) = collect(Encoding::Iso2022Jp, b"\x1B$@$\"\x1B(B\n\x1B$B$\"\n\x1B(B");
    assert_eq!(r, PreservedRecords::default());
    assert_eq!(r.state, PreserveState::Disabled);
    let (_, r) = collect(Encoding::Iso2022Jp, b"\x1B$@$\"\x1B(B\x80\n");
    assert_eq!(r.state, PreserveState::Disabled);
}

#[test]
fn cr_と_crlf_の文書の行番号() {
    let b = b"a\r\xFA\x54\r\nb\r\n\r\xFA\x54x";
    let (s, r) = collect(Encoding::ShiftJis, b);
    assert_eq!(s.first_offsets, vec![2, 10]);
    assert_eq!(
        recs(&r),
        vec![
            (1, 0, 3, b"\xFA\x54".to_vec(), DuplicateEncoding),
            (4, 0, 3, b"\xFA\x54".to_vec(), DuplicateEncoding),
        ]
    );
    let b = b"x\r\x1B$@$\"\x1B(B\r\n\r\x1B$@$\"\x1B(B";
    let (_, r) = collect(Encoding::Iso2022Jp, b);
    let lines: Vec<u64> = r.records.iter().map(|x| x.line).collect();
    assert_eq!(lines, vec![1, 3]);
}

#[test]
fn 上限を超えたら全部捨てる() {
    let run = |limits: PreserveLimits, b: &[u8]| {
        let mut c = RoundtripChecker::with_records(Encoding::ShiftJis).with_limits(limits);
        c.feed(b, true);
        c.finish_with_records()
    };
    let b = b"\xFA\x54a\xFA\x54a\xFA\x54";
    // 件数: 3 件は 2 件の上限を超える。数え方は続ける
    let (s, r) = run(
        PreserveLimits {
            max_records: 2,
            max_bytes: 1000,
        },
        b,
    );
    assert_eq!(s.lossy_lines, 1);
    assert_eq!(r.state, PreserveState::Overflow);
    assert!(r.records.is_empty() && r.arena.is_empty());
    let (_, r) = run(
        PreserveLimits {
            max_records: 3,
            max_bytes: 1000,
        },
        b,
    );
    assert_eq!((r.state, r.records.len()), (PreserveState::Collected, 3));
    // バイト数: 6 バイトは 5 バイトの上限を超える (ちょうど 6 バイトなら超えていない)
    let (_, r) = run(
        PreserveLimits {
            max_records: 100,
            max_bytes: 5,
        },
        b,
    );
    assert_eq!(r.state, PreserveState::Overflow);
    let (_, r) = run(
        PreserveLimits {
            max_records: 100,
            max_bytes: 6,
        },
        b,
    );
    assert_eq!(r.state, PreserveState::Collected);
    // 上限を超えた後の行も数える
    let mut c = RoundtripChecker::with_records(Encoding::ShiftJis).with_limits(PreserveLimits {
        max_records: 1,
        max_bytes: 1000,
    });
    c.feed(b"\xFA\x54\xFA\x54\n\xFA\x54a\xFA\x54\n\xFA\x54\n", true);
    let (s, r) = c.finish_with_records();
    assert_eq!(s.lossy_lines, 3);
    assert_eq!(r.state, PreserveState::Overflow);
}

#[test]
fn 既定の上限は_100_万件と_16_mib() {
    assert_eq!(eve_encoding::MAX_PRESERVED_RECORDS, 1_000_000);
    assert_eq!(eve_encoding::MAX_PRESERVED_BYTES, 16 * 1024 * 1024);
    assert_eq!(
        PreserveLimits::default(),
        PreserveLimits {
            max_records: 1_000_000,
            max_bytes: 16 * 1024 * 1024
        }
    );
}

#[test]
fn 記録を求めない検査器と区分_a_は集めない() {
    let mut c = RoundtripChecker::new(Encoding::ShiftJis);
    c.feed(b"\xFA\x54", true);
    let (s, r) = c.finish_with_records();
    assert_eq!(s.lossy_lines, 1);
    assert_eq!(r.state, PreserveState::Disabled);
    for enc in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        let (_, r) = collect(enc, b"\xFA\x54");
        assert_eq!(r.state, PreserveState::Disabled);
    }
}

#[test]
fn iso_2022_jp_の_4_mib_を超える行は打ち切って全部捨てる() {
    let mut b = b"\x1B$@$\"\x1B(B\n".to_vec();
    b.extend(std::iter::repeat_n(b'a', 4 * 1024 * 1024 + 1));
    b.extend_from_slice(b"\n\x1B$@$\"\x1B(B\n");
    let (s, r) = check_roundtrip_with_records(Encoding::Iso2022Jp, &b);
    assert!(s.incomplete);
    assert_eq!(s.lossy_lines, 1, "打ち切る前の行は数えている");
    assert_eq!(r.state, PreserveState::Incomplete);
    assert!(r.records.is_empty() && r.arena.is_empty());
}

#[test]
fn shift_jis_の_4_mib_を超える行の後ろの区分_b_の文字は記録になる() {
    // 長い行 (5 MiB) の中ほどと末尾、次の行に重複符号化 (チャンクは 64 KiB)
    let mut b = vec![b'a'; 2 * 1024 * 1024];
    b.extend_from_slice(b"\xFA\x54");
    b.extend(std::iter::repeat_n(b'b', 3 * 1024 * 1024));
    b.extend_from_slice(b"\xFA\x54\n\xFA\x54x");
    let mut c = RoundtripChecker::with_records(Encoding::ShiftJis);
    let mut it = b.chunks(64 * 1024).peekable();
    while let Some(ch) = it.next() {
        c.feed(ch, it.peek().is_none());
    }
    let (s, r) = c.finish_with_records();
    assert!(!s.incomplete);
    assert_eq!(s.lossy_lines, 2);
    assert_eq!(r.state, PreserveState::Collected);
    let at: Vec<(u64, u64)> = r.records.iter().map(|x| (x.line, x.col)).collect();
    let long: u64 = 5 * 1024 * 1024 + 3;
    assert_eq!(at, vec![(0, 2 * 1024 * 1024), (0, long), (1, 0)]);
    // 区切りやすい ASCII のバイトが無い長い行 (漢字だけ、1.5 MiB) も、小分けにして同じ位置になる
    let n = 768 * 1024;
    let mut b = b"\x93\xFA".repeat(n);
    b.extend_from_slice(b"\xFA\x54");
    let (s, r) = check_roundtrip_with_records(Encoding::ShiftJis, &b);
    assert_eq!((s.incomplete, s.lossy_lines), (false, 1));
    let at: Vec<(u64, u64)> = r.records.iter().map(|x| (x.line, x.col)).collect();
    assert_eq!(at, vec![(0, 3 * n as u64)]);
}
