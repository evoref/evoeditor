//! 符号化の照合 (docs/03 §8.6 E1) と保持方式の確認 (i) (§8.6 E2) の、速い経路の回帰の試験 (v0.4 の M22 の 7)。
//! 速い経路を足した所を、遅いが単純な手順 (速い経路を入れる前の手順を公開の API で組み直したもの) の参照と比べる。
//!
//! - E1 の照合 (`Encoder::encode`): BMP の全文字を文脈に置いて符号化し、出力と報告を、文字ごとに `encoding_rs` で符号化して
//!   復号で照合する参照 (変更前の手順と同じ判定) と比べる。
//! - 確認 (i) (`preserved_matches` / `PreservedBytes::check`): 変更前の eve-core の照合 (ISO-2022-JP は `\~` を足して
//!   `Decoder` で復号する) と、`encoding_rs` の一括の復号と比べる。

use eve_encoding::{
    Decoder, EncodeReport, Encoder, Encoding, EscapePolicy, PreservedBytes, UnmappableKind,
    preserved_matches,
};

fn rs(e: Encoding) -> &'static encoding_rs::Encoding {
    match e {
        Encoding::ShiftJis => encoding_rs::SHIFT_JIS,
        Encoding::EucJp => encoding_rs::EUC_JP,
        _ => encoding_rs::ISO_2022_JP,
    }
}

fn encode(enc: Encoding, text: &str) -> (Vec<u8>, Vec<(u64, char, UnmappableKind)>) {
    let mut e = Encoder::new(enc, false, EscapePolicy::Restore);
    let (mut out, mut rep) = (Vec::new(), EncodeReport::default());
    e.encode(text, &mut out, &mut rep);
    e.finish(&mut out, &mut rep);
    (
        out,
        rep.recorded
            .iter()
            .map(|u| (u.char_index, u.ch, u.kind))
            .collect(),
    )
}

/// `encoding_rs` で `s` を符号化する (表現不能の文字は飛ばし、その位置を返す)。
fn rs_encode(enc: Encoding, s: &str) -> (Vec<u8>, Vec<usize>) {
    let mut e = rs(enc).new_encoder();
    let mut out = vec![0u8; s.len() * 8 + 16];
    let (mut pos, mut end, mut bad) = (0usize, 0usize, Vec::new());
    loop {
        let (r, read, written) =
            e.encode_from_utf8_without_replacement(&s[pos..], &mut out[end..], true);
        pos += read;
        end += written;
        match r {
            encoding_rs::EncoderResult::InputEmpty => break,
            encoding_rs::EncoderResult::Unmappable(_) => {
                let c = s[..pos].chars().next_back().unwrap();
                bad.push(pos - c.len_utf8());
            }
            encoding_rs::EncoderResult::OutputFull => panic!("作業領域が足りない"),
        }
    }
    out.truncate(end);
    (out, bad)
}

/// 参照: 変更前の照合の手順 (表現不能を飛ばして符号化し、出力を復号して、表現できた文字と 1 文字ずつ比べる)。
fn reference(enc: Encoding, text: &str) -> (Vec<u8>, Vec<(u64, char, UnmappableKind)>) {
    let (out, bad) = rs_encode(enc, text);
    let got = rs(enc).decode_without_bom_handling(&out).0.into_owned();
    let mut g = got.chars();
    let mut rep = Vec::new();
    for (i, (bi, c)) in text.char_indices().enumerate() {
        if bad.contains(&bi) {
            rep.push((i as u64, c, UnmappableKind::Unmappable));
            continue;
        }
        match g.next() {
            Some(x) if x == c => {}
            _ => rep.push((i as u64, c, UnmappableKind::Substituted)),
        }
    }
    (out, rep)
}

// BMP の全文字 (外字 U+E000..U+E757 は Shift_JIS では自前の逆算なので除く) × 3 文字コード × 文脈
#[test]
fn e1_の照合は文字ごとの参照と同じ() {
    let mut n = 0u64;
    let mut subst = 0u64;
    for enc in [Encoding::ShiftJis, Encoding::EucJp, Encoding::Iso2022Jp] {
        for v in 0u32..=0xFFFF {
            let Some(c) = char::from_u32(v) else { continue };
            if enc == Encoding::ShiftJis && (0xE000..=0xE757).contains(&v) {
                continue;
            }
            // ISO-2022-JP は行ごとに符号化するので、文脈に改行を入れない
            if enc == Encoding::Iso2022Jp && (c == '\n' || c == '\r') {
                continue;
            }
            for t in [format!("a{c}b"), format!("{c}{c}"), format!("日{c}")] {
                let got = encode(enc, &t);
                let want = reference(enc, &t);
                assert_eq!(got, want, "{enc:?} U+{v:04X} {t:?}");
                subst += got
                    .1
                    .iter()
                    .filter(|r| r.2 == UnmappableKind::Substituted)
                    .count() as u64;
                n += 1;
            }
        }
    }
    // 黙示の置換を実際に多数踏んでいる (速い経路を外れて報告する経路を通している)
    assert!(subst > 100, "黙示の置換 {subst} 件 / {n}");
}

/// 変更前の eve-core の `matches_original` (ISO-2022-JP は後ろに `\~` を足して復号し、その 2 文字がそのまま出るか)。
fn old_matches(enc: Encoding, bytes: &[u8], want: &str) -> bool {
    const PROBE: &str = "\\~";
    let mut out = String::new();
    let mut d = Decoder::new(enc, false);
    if enc.is_stateful() {
        d.decode(bytes, false, &mut out);
        d.decode(PROBE.as_bytes(), true, &mut out);
        d.invalid_bytes() == 0 && out.strip_suffix(PROBE) == Some(want)
    } else {
        d.decode(bytes, true, &mut out);
        d.invalid_bytes() == 0 && out == want
    }
}

fn check_one(enc: Encoding, b: &[u8]) {
    let mut wants = vec![String::new()];
    if let Some(s) = rs(enc).decode_without_bom_handling_and_without_replacement(b) {
        let s = s.into_owned();
        wants.push(format!("{s}x"));
        if let Some(c) = s.chars().next_back() {
            wants.push(s[..s.len() - c.len_utf8()].to_owned());
        }
        wants.push(s);
    }
    for w in &wants {
        let a = preserved_matches(enc, b, w);
        assert_eq!(a, old_matches(enc, b, w), "{enc:?} {b:02X?} {w:?}");
        assert_eq!(a, PreservedBytes::check(enc, b, w).is_some());
    }
}

// 1〜2 バイトの全部、ISO-2022-JP は ESC で始まる 3〜6 バイトを意味のあるバイトの組で
#[test]
fn 確認_i_は変更前の照合と同じ() {
    for enc in [Encoding::ShiftJis, Encoding::EucJp, Encoding::Iso2022Jp] {
        for x in 0..=u8::MAX {
            check_one(enc, &[x]);
            for y in 0..=u8::MAX {
                check_one(enc, &[x, y]);
            }
        }
    }
    let sig: &[u8] = b"($BJI@AD\\~!\"01`_\x1B\x0E\x0Fa\n\r\x7F \x21\x7E\x80\xB1";
    let mut b = Vec::new();
    for &x in sig {
        for &y in sig {
            for &z in sig {
                for &w in sig {
                    b.clear();
                    b.extend_from_slice(&[0x1B, x, y, z, w]);
                    for l in 3..=5 {
                        check_one(Encoding::Iso2022Jp, &b[..l]);
                    }
                    // 状態を変えてから戻す / 戻さない
                    b.extend_from_slice(b"\x1B(B");
                    check_one(Encoding::Iso2022Jp, &b);
                    b.truncate(5);
                    b.extend_from_slice(b"\\~");
                    check_one(Encoding::Iso2022Jp, &b);
                }
            }
        }
    }
}
