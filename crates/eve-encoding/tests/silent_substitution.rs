//! 黙示の置換 (別の文字として黙って写される文字) の検出 (docs/03 §8.6 E1、01 FR-112 AC-2・AC-3、FR-109 AC-4)。
//! Shift_JIS / EUC-JP / ISO-2022-JP のそれぞれで、すべての Unicode スカラー値を符号化し、
//! 表現不能として報告されなかった文字が、復号すると元の文字に戻る (黙って別の文字になった文字が 1 つも無い) ことを確かめる。

use eve_encoding::{Decoder, EncodeReport, Encoder, Encoding, EscapePolicy, unescape_char};

fn encode_all(enc: Encoding, text: &str) -> (Vec<u8>, EncodeReport) {
    let mut e = Encoder::new(enc, false, EscapePolicy::Restore);
    let (mut out, mut rep) = (Vec::new(), EncodeReport::default());
    e.encode(text, &mut out, &mut rep);
    e.finish(&mut out, &mut rep);
    (out, rep)
}

fn decode(enc: Encoding, bytes: &[u8]) -> String {
    let mut d = Decoder::new(enc, false);
    let mut s = String::new();
    d.decode(bytes, true, &mut s);
    s
}

// verifies: AC-FR-112-3
#[test]
fn 報告されなかった文字はすべて元の文字に戻る() {
    for enc in [Encoding::ShiftJis, Encoding::EucJp, Encoding::Iso2022Jp] {
        let mut silent: Vec<char> = Vec::new();
        // 不正バイトのエスケープ文字は、元のバイトに戻して書く別の経路 (E4) なので対象に含めない
        // 3 つの符号化はどれも BMP の外に対応を持たない。BMP は全件、その外は間引いて調べる
        let all: Vec<char> = (0u32..=0x10FFFF)
            .filter(|u| *u < 0x10000 || u % 61 == 0)
            .filter_map(char::from_u32)
            .filter(|c| unescape_char(*c).is_none())
            .collect();
        for block in all.chunks(2048) {
            // 区切りを挟んで、状態付き符号化でも文字ごとに独立に検査する
            let text: String = block.iter().flat_map(|c| [*c, 'x']).collect();
            let (bytes, rep) = encode_all(enc, &text);
            let reported: std::collections::BTreeSet<u64> =
                rep.recorded.iter().map(|u| u.char_index).collect();
            if rep.total as usize > rep.recorded.len() {
                // 記録は先頭 MAX_RECORDED 件まで。超えるブロックは 1 文字ずつ調べる
                for c in block {
                    let (b, r) = encode_all(enc, &c.to_string());
                    if r.is_empty() && decode(enc, &b) != c.to_string() {
                        silent.push(*c);
                    }
                }
                continue;
            }
            let expected: String = text
                .chars()
                .enumerate()
                .filter(|(i, _)| !reported.contains(&(*i as u64)))
                .map(|(_, c)| c)
                .collect();
            if decode(enc, &bytes) != expected {
                for c in block {
                    let (b, r) = encode_all(enc, &c.to_string());
                    if r.is_empty() && decode(enc, &b) != c.to_string() {
                        silent.push(*c);
                    }
                }
            }
        }
        assert!(
            silent.is_empty(),
            "{enc:?}: 黙って別の文字になる文字 {} 個 (例 {:?})",
            silent.len(),
            &silent[..silent.len().min(8)]
        );
    }
}

// verifies: AC-FR-112-2, AC-FR-109-4
#[test]
fn jis_系の互換文字はどの文字コードでも表現不能として報告される() {
    // U+301C・U+2016・U+2212・U+00A2・U+00A3・U+00AC・U+00A6・U+2014 (全部)、Shift_JIS / EUC-JP は加えて ¥ U+00A5・‾ U+203E
    let common = [
        '\u{301C}', '\u{2016}', '\u{2212}', '\u{A2}', '\u{A3}', '\u{AC}', '\u{A6}', '\u{2014}',
    ];
    for enc in [Encoding::ShiftJis, Encoding::EucJp, Encoding::Iso2022Jp] {
        for c in common {
            let (_, r) = encode_all(enc, &format!("a{c}b"));
            assert_eq!(r.total, 1, "{enc:?} {c:?}");
        }
    }
    for enc in [Encoding::ShiftJis, Encoding::EucJp] {
        for c in ['\u{A5}', '\u{203E}'] {
            let (_, r) = encode_all(enc, &format!("a{c}b"));
            assert_eq!(r.total, 1, "{enc:?} {c:?}");
        }
    }
    // ISO-2022-JP の半角カナ
    let (_, r) = encode_all(Encoding::Iso2022Jp, "ｱｲｳ");
    assert_eq!(r.total, 3);
}
