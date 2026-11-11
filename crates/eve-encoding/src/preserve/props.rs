//! 保持方式の記録の性質テスト (03 §8.7.1、09 §3.3 L1)。
//!
//! - (a) どの記録も不変条件を満たす (元のバイト列を初期状態から復号すると、本文の範囲の文字列になり、初期状態で終わる)。
//! - (b) 位置 (`line`・`col`・`len`) が、同じバイト列を `Decoder` で通しで復号した本文の座標と合う。
//! - (c) 記録は昇順で重ならない。
//! - (d) チャンクの分け方 (と、塊・小分けの大きさ) に依らない。
//! - 再構成: 記録の範囲は元のバイト列、それ以外は `Encoder` で符号化すると、元のバイト列に戻る (ISO-2022-JP は行ごと。
//!   保持できなかった箇所が無い入力)。
//!
//! (a)〜(c) は `fuzzing::check_preserved` で確かめる (`fuzz_decode` も同じ関数を使う)。

use proptest::prelude::*;

use crate::fuzzing::{body_lines, check_preserved};
use crate::{
    Decoder, EncodeReport, Encoder, Encoding, EscapePolicy, PreserveState, PreservedRecords,
    RoundtripChecker, RoundtripSummary,
};

/// `Shift_JIS` の部品: ASCII、ふつうの漢字・かな・ギリシャ文字、重複符号化 (IBM 拡張・NEC 選定 IBM 拡張・NEC 特殊文字)、
/// CP932 互換文字・外字 (記録にならない)、半角カナ、0x80、不正バイト (単独の先頭バイト・0xA0・0xFD〜0xFF、対応の無い組)。
const SJIS: &[&[u8]] = &[
    b"a",
    b"Z\\~ ",
    b"0",
    b"\x93\xFA",
    b"\x82\xA0",
    b"\x83\x9F",
    b"\xFA\x54",
    b"\xFA\x55",
    b"\xFA\x40",
    b"\xED\x40",
    b"\xEE\xEF",
    b"\xEE\xFC",
    b"\x87\x40",
    b"\x87\x9B",
    b"\x87\x90",
    b"\x81\x60",
    b"\x81\xCA",
    b"\xF0\x40",
    b"\xF9\xFC",
    b"\xB1",
    b"\x80",
    b"\x81",
    b"\xA0",
    b"\xFD",
    b"\xFF",
    b"\x85\x40",
    b"\x85\x80",
];

/// EUC-JP の部品: ASCII、漢字・半角カナ、補助漢字 (JIS X 0212。復号専用)、別のコードに写るもの (0x8FA2B7)、
/// NEC 特殊文字 (0xAD)・IBM 拡張 (0xFC) の重複、互換文字、不正バイト。
const EUC: &[&[u8]] = &[
    b"a",
    b"Z\\~ ",
    b"\xC6\xFC",
    b"\xA4\xA2",
    b"\x8E\xB1",
    b"\x8F\xB0\xA1",
    b"\x8F\xA2\xB7",
    b"\x8F\xA2\xC3",
    b"\x8F\xF3\xF3",
    b"\xAD\xA1",
    b"\xAD\xE2",
    b"\xFC\xFC",
    b"\xFC\xEE",
    b"\xF9\xA1",
    b"\xA1\xC1",
    b"\x8F",
    b"\x8E",
    b"\xFF",
    b"\xA1",
    b"\x8F\xA1",
];

/// ISO-2022-JP の行の途中の部品 (出力する文字で始める。エスケープシーケンスが続くと不正バイトになるため): ASCII、正規形の
/// JIS X 0208、旧 JIS (`ESC $ @`)、ローマ字 (`ESC ( J`。開いたままにすると、状態が改行をまたぐ)、冗長なエスケープ、
/// 半角カナ (`ESC ( I`)、ローマ字状態で終わる JIS X 0208。
const ISO_MID: &[&[u8]] = &[
    b"ab",
    b"\\~",
    b"x\x1B$B$\"\x1B(B",
    b"x\x1B$@$\"\x1B(B",
    b"x\x1B(J\\a",
    b"x\x1B(Ba",
    b"x\x1B(I12\x1B(B",
    b"x\x1B$B$\"\x1B(J\\",
    b"x\x1B$BF|K\\\x1B(B",
];

/// ISO-2022-JP の行頭の部品 (エスケープシーケンスで始まるもの。空も選ぶ。冗長な `ESC ( B` だけの行は、内容が空の記録になる)。
const ISO_START: &[&[u8]] = &[
    b"",
    b"",
    b"\x1B$@$\"\x1B(B",
    b"\x1B(Ba",
    b"\x1B(B",
    b"\x1B(J\\",
    b"\x1B$B$\"\x1B(B",
    b"\x1B(I1\x1B(B",
];

/// ISO-2022-JP の行末の部品 (空、または ASCII 状態に戻す)。
const ISO_END: &[&[u8]] = &[b"", b"", b"z\x1B(B"];

/// ISO-2022-JP の行末の、不正バイトになる部品 (開いたままの JIS X 0208 と半角カナで改行する、SO、8 bit)。少しだけ選ぶ。
const ISO_BAD: &[&[u8]] = &[b"x\x1B$BF|", b"x\x1B(I1", b"\x0E", b"\x80"];

/// ISO-2022-JP の文書: 行ごとに (行頭の部品, 途中の部品, 行末の部品, 改行 (0 は改行なし))。
fn iso_bytes(lines: &[(u8, Vec<u8>, u8, u8)]) -> Vec<u8> {
    let mut b = Vec::new();
    for (start, mids, end, eol) in lines {
        b.extend_from_slice(ISO_START[usize::from(*start) % ISO_START.len()]);
        for &m in mids {
            b.extend_from_slice(ISO_MID[usize::from(m) % ISO_MID.len()]);
        }
        let end = usize::from(*end);
        b.extend_from_slice(if end < 4 {
            ISO_BAD[end]
        } else {
            ISO_END[end % ISO_END.len()]
        });
        if *eol > 0 {
            b.extend_from_slice(EOLS[usize::from(*eol) % 3]);
        }
    }
    b
}

fn iso_lines() -> impl Strategy<Value = Vec<(u8, Vec<u8>, u8, u8)>> {
    proptest::collection::vec(
        (
            any::<u8>(),
            proptest::collection::vec(any::<u8>(), 0..5),
            any::<u8>(),
            0u8..40,
        ),
        0..16,
    )
}

const EOLS: [&[u8]; 3] = [b"\n", b"\r\n", b"\r"];

/// 部品と改行と任意のバイトを並べたバイト列。
fn bytes_from(atoms: &[&[u8]], picks: &[(u8, u8)], newline_ratio: u8) -> Vec<u8> {
    let mut b = Vec::new();
    for &(k, v) in picks {
        if k < newline_ratio {
            b.extend_from_slice(EOLS[usize::from(v) % 3]);
        } else if k == u8::MAX {
            b.push(v);
        } else {
            b.extend_from_slice(atoms[usize::from(v) % atoms.len()]);
        }
    }
    b
}

fn picks() -> impl Strategy<Value = Vec<(u8, u8)>> {
    proptest::collection::vec((any::<u8>(), any::<u8>()), 0..120)
}

fn decode(enc: Encoding, bytes: &[u8]) -> String {
    let mut d = Decoder::new(enc, false);
    let mut s = String::new();
    d.decode(bytes, true, &mut s);
    s
}

fn collect(enc: Encoding, bytes: &[u8]) -> (RoundtripSummary, PreservedRecords) {
    let mut c = RoundtripChecker::with_records(enc);
    c.feed(bytes, true);
    c.finish_with_records()
}

/// チャンク・塊・小分けの大きさを変えて集める。
fn collect_split(
    enc: Encoding,
    bytes: &[u8],
    cuts: &[usize],
    block: usize,
    piece: usize,
) -> (RoundtripSummary, PreservedRecords) {
    let mut c = RoundtripChecker::with_records(enc);
    c.block_limit = block;
    c.piece_limit = piece;
    let mut at = 0;
    for &n in cuts {
        let end = (at + n).min(bytes.len());
        c.feed(&bytes[at..end], false);
        at = end;
    }
    c.feed(&bytes[at..], true);
    c.finish_with_records()
}

/// 記録の範囲は元のバイト列、それ以外は `Encoder` で符号化して、本文からバイト列を作り直す (`Shift_JIS` / EUC-JP)。
fn rebuild_stateless(enc: Encoding, body: &str, recs: &PreservedRecords) -> Vec<u8> {
    let lines = body_lines(body);
    let mut e = Encoder::new(enc, false, EscapePolicy::Restore);
    let (mut out, mut rep) = (Vec::new(), EncodeReport::default());
    let mut at = 0;
    for r in &recs.records {
        let line = usize::try_from(r.line).expect("行番号");
        let start = lines[line].0 + usize::try_from(r.col).expect("位置");
        e.encode(&body[at..start], &mut out, &mut rep);
        out.extend_from_slice(recs.bytes_of(r));
        at = start + r.len as usize;
    }
    e.encode(&body[at..], &mut out, &mut rep);
    e.finish(&mut out, &mut rep);
    assert!(rep.is_empty(), "記録の外に表現不能の文字が無い: {rep:?}");
    out
}

/// ISO-2022-JP: 記録のある行は元のバイト列 + 行の終端、それ以外の行は `Encoder` の出力 (行ごとに閉じる、E6)。
fn rebuild_iso(body: &str, recs: &PreservedRecords) -> Vec<u8> {
    let lines = body_lines(body);
    let mut out = Vec::new();
    let mut rec = recs.records.iter().peekable();
    for (i, &(start, len)) in lines.iter().enumerate() {
        let end = lines.get(i + 1).map_or(body.len(), |n| n.0);
        let eol = &body[start + len..end];
        if let Some(r) = rec.next_if(|r| r.line == i as u64) {
            out.extend_from_slice(recs.bytes_of(r));
            out.extend_from_slice(eol.as_bytes());
        } else {
            let mut e = Encoder::new(Encoding::Iso2022Jp, false, EscapePolicy::Restore);
            let mut rep = EncodeReport::default();
            e.encode(&body[start..end], &mut out, &mut rep);
            e.finish(&mut out, &mut rep);
            assert!(
                rep.is_empty(),
                "記録の無い行に表現不能の文字が無い: {rep:?}"
            );
        }
    }
    out
}

fn check_all(
    enc: Encoding,
    bytes: &[u8],
    cuts: &[usize],
    block: usize,
    piece: usize,
) -> Result<(), TestCaseError> {
    let body = decode(enc, bytes);
    let (summary, recs) = collect(enc, bytes);
    // (a)〜(c)
    check_preserved(enc, &body, &recs);
    // 記録を集めても、数え方は記録を集めない検査器と同じ
    let mut plain = RoundtripChecker::new(enc);
    plain.feed(bytes, true);
    prop_assert_eq!(&summary, &plain.finish());
    // (d)
    let split = collect_split(enc, bytes, cuts, block, piece);
    prop_assert_eq!(&split, &(summary.clone(), recs.clone()), "{:02X?}", bytes);
    // 再構成
    match enc {
        Encoding::Iso2022Jp => {
            // 打ち切り (4 MiB を超える行) と突き合わせの食い違いは起きない
            prop_assert_ne!(recs.state, PreserveState::Incomplete);
            if recs.state == PreserveState::Collected && recs.unpreserved == 0 {
                prop_assert_eq!(rebuild_iso(&body, &recs), bytes);
            }
        }
        _ => {
            prop_assert_eq!(recs.state, PreserveState::Collected);
            prop_assert_eq!(recs.unpreserved, 0);
            prop_assert_eq!(rebuild_stateless(enc, &body, &recs), bytes);
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn shift_jis_の記録の性質(
        picks in picks(),
        cuts in proptest::collection::vec(1usize..17, 0..40),
        block in 1usize..64,
        piece in 1usize..24,
    ) {
        let bytes = bytes_from(SJIS, &picks, 30);
        check_all(Encoding::ShiftJis, &bytes, &cuts, block, piece)?;
    }

    #[test]
    fn euc_jp_の記録の性質(
        picks in picks(),
        cuts in proptest::collection::vec(1usize..17, 0..40),
        block in 1usize..64,
        piece in 1usize..24,
    ) {
        let bytes = bytes_from(EUC, &picks, 30);
        check_all(Encoding::EucJp, &bytes, &cuts, block, piece)?;
    }

    #[test]
    fn iso_2022_jp_の記録の性質(
        lines in iso_lines(),
        cuts in proptest::collection::vec(1usize..17, 0..40),
        block in 1usize..64,
    ) {
        let bytes = iso_bytes(&lines);
        check_all(Encoding::Iso2022Jp, &bytes, &cuts, block, usize::MAX)?;
    }

    #[test]
    fn iso_2022_jp_は任意のバイト列でも性質を満たす(
        picks in picks(),
        cuts in proptest::collection::vec(1usize..17, 0..40),
        block in 1usize..64,
    ) {
        let atoms: Vec<&[u8]> = ISO_MID.iter().chain(ISO_START).chain(ISO_END).chain(ISO_BAD).copied().collect();
        let bytes = bytes_from(&atoms, &picks, 50);
        check_all(Encoding::Iso2022Jp, &bytes, &cuts, block, usize::MAX)?;
    }

    // 区切りやすい ASCII のバイトが無い長い行 (日本語だけ) も、文字の境界で小分けにして調べる
    #[test]
    fn ascii_の無い行も小分けにして調べる(
        picks in proptest::collection::vec(any::<u8>(), 0..200),
        cuts in proptest::collection::vec(1usize..17, 0..40),
        piece in 1usize..24,
    ) {
        for enc in [Encoding::ShiftJis, Encoding::EucJp] {
            let atoms: Vec<&[u8]> = match enc {
                Encoding::ShiftJis => SJIS.iter().copied().filter(|a| a.iter().all(|&b| b >= 0x40 && b != 0x7F)).collect(),
                _ => EUC.iter().copied().filter(|a| a.iter().all(|&b| b >= 0x80)).collect(),
            };
            let mut bytes = Vec::new();
            for &p in &picks {
                if p == 0 {
                    bytes.push(b'\n');
                } else {
                    bytes.extend_from_slice(atoms[usize::from(p) % atoms.len()]);
                }
            }
            check_all(enc, &bytes, &cuts, 7, piece)?;
        }
    }
}

// 1 バイトずつのチャンクでも、どの小分けの大きさでも同じ (固定の入力)
#[test]
fn 一バイトずつ渡しても同じ() {
    let bytes =
        b"a\xFA\x54\xFA\x55b\xED\x40\r\n\x81\xFA\x54\xFF\x87\x9B\rx\xF0\x40\x81\x60\xFA\x54";
    let (s, r) = collect(Encoding::ShiftJis, bytes);
    for piece in 1..12 {
        let ones = vec![1; bytes.len()];
        assert_eq!(
            collect_split(Encoding::ShiftJis, bytes, &ones, 3, piece),
            (s.clone(), r.clone()),
            "piece {piece}"
        );
    }
}
