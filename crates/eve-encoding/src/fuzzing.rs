//! `fuzz_decode` の本体 (docs/09 §3.3)。cargo-fuzz のターゲット (`fuzz/fuzz_targets/fuzz_decode.rs`) と、Windows の CI で回す
//! proptest (`tests/fuzz_decode.rs`) が同じ関数を使う (libFuzzer は MSVC 非対応のため、Linux の夜間ジョブだけで fuzz する)。
//!
//! 入力: 任意のバイト列 + 文字コードのラベル + sniff 長 + チャンク分割位置。判定 (03 §8.2) を通してから復号し、
//! 往復は「バイト → テキスト → バイト」(区分 A の一致と区分 B の検出) だけを検査する。パニックしないことが主な性質。

use crate::{
    Confidence, Decoder, DetectOptions, DetectedKind, EncodeReport, Encoder, Encoding,
    EscapePolicy, RoundtripChecker, detect, redetect_window, unescape_char,
};

/// 入力の先頭の数バイトを制御に使い、残りを本文にする (cargo-fuzz の生のバイト列をそのまま渡せる)。
///
/// 先頭: [0] 文字コードのラベル (0 = 判定に任せる、1..=6 = `Encoding::ALL` の明示指定)、[1..3] sniff 長 (小さな値に丸める)、
/// [3] チャンクの大きさの種 (0 = 一括)。
pub fn decode_pipeline(data: &[u8]) {
    let (head, body) = if data.len() >= 4 {
        data.split_at(4)
    } else {
        (&[0u8, 0, 0, 0][..], data)
    };
    let forced = match head[0] % 8 {
        n @ 1..=6 => Some(Encoding::ALL[(n - 1) as usize]),
        _ => None,
    };
    let sniff_len = (usize::from(u16::from_le_bytes([head[1], head[2]])) % 2048).max(1);
    let chunk = usize::from(head[3]);

    // 判定 (手順 1〜8)
    let sniff = &body[..body.len().min(sniff_len)];
    let complete = sniff.len() == body.len();
    let opts = DetectOptions::default().with_forced(forced);
    let det = detect(sniff, complete, &opts);
    // バイナリ・未対応の BOM は確認が入るので、本文を UTF-8 として読む (確認後の「読み取り専用で開く」と同じ)
    let (enc, bom_len) = match det.kind {
        DetectedKind::Utf8 { bom_len } => (Encoding::Utf8, bom_len),
        DetectedKind::Encoded { encoding, bom_len } => (encoding, bom_len),
        _ => (Encoding::Utf8, 0),
    };
    // 再判定 (§8.2.1): 信頼度「低」のとき、不正な UTF-8 または ISO-2022-JP の列の周辺で判定し直す
    let enc = if det.confidence == Confidence::Low && forced.is_none() {
        match redetect_window(body, true, &opts).and_then(|d| d.encoding()) {
            Some(e) => e,
            None => enc,
        }
    } else {
        enc
    };
    let bom_len = if enc == det.encoding().unwrap_or(enc) {
        bom_len
    } else {
        0
    };

    // 復号 (チャンク分割は入力から決める)
    let body = &body[bom_len.min(body.len())..];
    let mut dec = Decoder::new(enc, false);
    let mut text = String::new();
    let mut checker = RoundtripChecker::new(enc);
    let step = if chunk == 0 { body.len().max(1) } else { chunk };
    let mut it = body.chunks(step).peekable();
    if it.peek().is_none() {
        dec.decode(&[], true, &mut text);
        checker.feed(&[], true);
    }
    while let Some(c) = it.next() {
        let last = it.peek().is_none();
        dec.decode(c, last, &mut text);
        checker.feed(c, last);
    }
    let summary = checker.finish();

    // 状態付き符号化で不正バイトがあれば読み取り専用 (復号できない入力は不正バイトに数えられる)
    let escapes = text.chars().filter(|c| unescape_char(*c).is_some()).count() as u32;
    assert!(dec.invalid_bytes() <= escapes.max(dec.invalid_bytes()));
    if enc.is_stateful() {
        assert_eq!(dec.pua_mode(), crate::PuaEscapeMode::DisabledStateful);
    }

    // 往復: 区分 A はバイト一致 (真に PUA を含む衝突を除く)。ISO-2022-JP はパニックしないことだけ
    let policy = EscapePolicy::from_mode(dec.pua_mode());
    let mut e = Encoder::new(enc, false, policy);
    let (mut out, mut rep) = (Vec::new(), EncodeReport::default());
    e.encode(&text, &mut out, &mut rep);
    e.finish(&mut out, &mut rep);
    if (enc == Encoding::Utf8 || enc.is_utf16()) && !dec.pua_collision() {
        assert!(rep.is_empty());
        assert_eq!(out, body, "区分 A のバイト一致 ({})", enc.name());
    }
    // 区分 B の検出は復号と独立に数えるだけ (件数が上限を超えない)
    assert!(summary.first_offsets.len() <= crate::MAX_POSITIONS);
}

/// `fuzz_encode` の本体 (docs/review/v0.3-findings.md の §2.7 の 9)。入力の先頭 4 バイトで文字コード (6 個)・
/// エスケープの方針 (3 個)・BOM・チャンクの大きさを決め、残りから文字列を作る (不正な UTF-8 の列はエスケープ文字にする:
/// 不正バイトを含む文書を保存するのと同じ形)。検査する性質:
///
/// - パニックしない。
/// - 一括の符号化とチャンクに分けた符号化で、出力と報告が同じ (保存はチャンクごとに符号化する、03 §9.3)。
/// - 同じ入力を 2 回符号化すると同じ結果 (乾式走査 (書かずに数える) と本番の書き込みが一致する、03 §8.6 E1)。
/// - UTF-16 では、報告が 0 件なら `utf16_len` (乾式走査の出力長) + BOM が出力の長さと一致する。
/// - 報告が 0 件で、エスケープ文字を含まなければ、復号すると元の文字列に戻る。
pub fn encode_pipeline(data: &[u8]) {
    let (head, body) = if data.len() >= 4 {
        data.split_at(4)
    } else {
        (&[0u8, 0, 0, 0][..], data)
    };
    let enc = Encoding::ALL[usize::from(head[0]) % Encoding::ALL.len()];
    let policy = [
        EscapePolicy::Restore,
        EscapePolicy::AsText,
        EscapePolicy::Reject,
    ][usize::from(head[1]) % 3];
    let bom = head[2] & 1 == 1 && (enc == Encoding::Utf8 || enc.is_utf16());
    let chunk_chars = usize::from(head[3] % 32);

    // 文字列: 正しい UTF-8 の部分はそのまま、不正なバイトはエスケープ文字 (復号と同じ表現)
    let mut text = String::new();
    for piece in body.utf8_chunks() {
        text.push_str(piece.valid());
        text.extend(piece.invalid().iter().map(|&b| crate::escape_byte(b)));
    }

    let run = |pieces: &[&str]| {
        let mut e = Encoder::new(enc, bom, policy);
        let (mut out, mut rep) = (Vec::new(), EncodeReport::default());
        for p in pieces {
            e.encode(p, &mut out, &mut rep);
        }
        e.finish(&mut out, &mut rep);
        (out, rep)
    };
    let (whole, whole_rep) = run(&[text.as_str()]);

    // チャンクに分ける (文字の境界で。0 は分けない)
    let pieces: Vec<&str> = if chunk_chars == 0 {
        vec![text.as_str()]
    } else {
        let mut v = Vec::new();
        let mut start = 0;
        for (n, (i, _)) in text.char_indices().enumerate() {
            if n > 0 && n % chunk_chars == 0 {
                v.push(&text[start..i]);
                start = i;
            }
        }
        v.push(&text[start..]);
        v
    };
    let (chunked, chunked_rep) = run(&pieces);
    assert_eq!(
        chunked,
        whole,
        "チャンクに分けても同じ出力 ({})",
        enc.name()
    );
    assert_eq!(
        chunked_rep,
        whole_rep,
        "チャンクに分けても同じ報告 ({})",
        enc.name()
    );

    let (again, again_rep) = run(&[text.as_str()]);
    assert_eq!(
        (&again, &again_rep),
        (&whole, &whole_rep),
        "乾式走査と本番が一致"
    );

    if !whole_rep.is_empty() {
        assert!(whole_rep.recorded.len() as u64 <= whole_rep.total);
        return;
    }
    if enc.is_utf16() {
        let want = crate::encode::utf16_len(&text, policy) + if bom { 2 } else { 0 };
        assert_eq!(whole.len(), want, "UTF-16 の乾式走査の出力長");
    }
    if !text.chars().any(|c| unescape_char(c).is_some()) {
        let mut dec = Decoder::new(enc, bom);
        let mut back = String::new();
        dec.decode(&whole, true, &mut back);
        assert_eq!(back, text, "報告 0 件なら復号で元に戻る ({})", enc.name());
    }
}
