//! つながりの検出 (`MergeScanner`、docs/03 §8.3.1) の不変条件の本体。性質テスト (`tests/merge_property.rs`) と、
//! cargo-fuzz のターゲット (`fuzz/fuzz_targets/fuzz_encode_merge.rs`。`#[path]` で取り込む) が同じ関数を使う。

// 使う側ごとに使わない項目がある
#![allow(dead_code)]

use std::hash::DefaultHasher;

use eve_encoding::{
    Decoder, EncodeReport, Encoder, Encoding, EscapePolicy, MergeReport, MergeScanner, escape_byte,
};

/// 本文を 1 文字ずつ符号化して検出器に渡す (保存の符号化の流れの単位ごとの経路と同じ)。表現不能の文字があれば `None`。
fn scan(
    enc: Encoding,
    bom: bool,
    track_range: bool,
    text: &str,
) -> Option<(Vec<u8>, bool, MergeReport)> {
    let mut e = Encoder::new(enc, bom, EscapePolicy::Restore);
    let (mut out, mut rep) = (Vec::new(), EncodeReport::default());
    e.encode("", &mut out, &mut rep);
    let bom_written = !out.is_empty();
    let mut s = MergeScanner::new(
        enc,
        bom_written,
        track_range,
        Box::new(DefaultHasher::new()),
    );
    let mut pos = 0;
    for c in text.chars() {
        let one = c.to_string();
        let idx = e.chars_done();
        let before = out.len();
        e.encode(&one, &mut out, &mut rep);
        s.unit(&one, &out[before..], idx, pos, before as u64);
        pos += one.len();
    }
    e.finish(&mut out, &mut rep);
    if !rep.is_empty() {
        return None;
    }
    s.finish(e.chars_done(), pos, out.len() as u64);
    Some((out, bom_written, s.report()))
}

/// `Restore` で書いた出力から、書いた BOM を除いて、同じ文字コードを指定した初期状態の復号器で復号した本文。
fn reopen(enc: Encoding, bom_written: bool, out: &[u8]) -> String {
    let skip = match (bom_written, enc) {
        (true, Encoding::Utf8) => 3,
        (true, _) => 2,
        (false, _) => 0,
    };
    let mut d = Decoder::new(enc, false);
    let mut back = String::new();
    d.decode(&out[skip..], true, &mut back);
    back
}

/// 本文 `text` (エスケープ文字を含む) を `Restore` で符号化して検出器に掛け、次を検査する。
/// - パニックしない。
/// - 範囲を求める乾式走査と、求めない本番の書き込みで、件数・先頭の位置・全件のダイジェストが同じ。
/// - 局所の検査が 0 件 ⇔ 書いた BOM を除いた出力の全体を同じ文字コードで復号すると本文に一致する。
///
/// つながりの件数を返す (表現不能の文字があって検査できない本文は `None`)。
pub fn check_text(enc: Encoding, bom: bool, text: &str) -> Option<u64> {
    let (out, bom_written, rep) = scan(enc, bom, true, text)?;
    let (out2, _, rep2) = scan(enc, bom, false, text).expect("同じ入力");
    assert_eq!(out2, out, "同じ出力 ({})", enc.name());
    assert_eq!(
        (rep.total, &rep.recorded, rep.digest),
        (rep2.total, &rep2.recorded, rep2.digest),
        "乾式走査と本番で件数・位置・ダイジェストが同じ ({})",
        enc.name()
    );
    let back = reopen(enc, bom_written, &out);
    assert_eq!(
        rep.total == 0,
        back == text,
        "つながり 0 件 ⇔ 復号すると本文に一致 ({} bom={bom} total={} text={text:?} out={out:02X?} back={back:?})",
        enc.name(),
        rep.total
    );
    Some(rep.total)
}

/// 生のバイト列から本文を組んで `check_text` を呼ぶ (`encode_pipeline` と同じ読み替え。cargo-fuzz にそのまま渡せる)。
/// 先頭: [0] 文字コード、[2] の最下位ビットが BOM。残りが本文で、不正な UTF-8 のバイトはエスケープ文字にする。
pub fn merge_pipeline(data: &[u8]) {
    let (head, body) = if data.len() >= 4 {
        data.split_at(4)
    } else {
        (&[0u8, 0, 0, 0][..], data)
    };
    let enc = Encoding::ALL[usize::from(head[0]) % Encoding::ALL.len()];
    let bom = head[2] & 1 == 1 && (enc == Encoding::Utf8 || enc.is_utf16());
    let mut text = String::new();
    for piece in body.utf8_chunks() {
        text.push_str(piece.valid());
        text.extend(piece.invalid().iter().map(|&b| escape_byte(b)));
    }
    let _ = check_text(enc, bom, &text);
}
