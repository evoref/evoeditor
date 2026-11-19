//! 書記素の境界の規則に関わる char の表 (03 §14.4 の「走査」の **表**)。長大行の列の索引の走査の高速経路が使う。
//!
//! 表に無い 2 つの char の間は、行の内容の中では必ず書記素の境界になる。UAX #29 で境界を消す規則は、GB3 を除いて、
//! どれも境界の左右のどちらかに表の char を要る: GB6 (L × L・V・LV・LVT。左の L が表)、GB7 (LV・V × V・T。右の V・T
//! が表)、GB8 (LVT・T × T。右の T が表)、GB9・`GB9a` (右が Extend・ZWJ・`SpacingMark`)、`GB9b` (左が Prepend)、`GB9c`
//! (`InCB` の Consonant の左は Linker か Extend で、どちらも `Grapheme_Cluster_Break` の Extend・ZWJ に含まれる)、GB11
//! (`Extended_Pictographic` の左は ZWJ)、GB12・GB13 (左右とも `Regional_Indicator`)。GB3 (CR × LF) は CR も LF も表に
//! 無いが、CR・LF は行の終端で、行の内容に現れないことに頼る。そのため `Extended_Pictographic`・`InCB` の Consonant・
//! ハングルの音節 (LV・LVT) は表に入れない (前の char と結合するのは、左の char が表にあるときか、右の char が表にある
//! ときだけ)。走査は、表にある char の箇所だけ書記素の分割 (`unicode-segmentation`) に渡し、それ以外は 1 char =
//! 1 書記素として数える。
//!
//! 表は、書記素の分割に使う `unicode-segmentation` の版 (Unicode の版) の振る舞いから作った (全コードポイントを短い
//! 文字列に置いて、その char が表に無い隣と結合する場所があるかを調べる)。表にあるのは、`Grapheme_Cluster_Break` が
//! Extend・ZWJ・`SpacingMark`・Prepend・`Regional_Indicator`・L・V・T の char。
//!
//! **表の作り直しの手順** (`unicode-segmentation` の版を上げて、試験 `表の_unicode_の版は…` か `表は全コードポイントで…`
//! が落ちたとき): `cargo test -p eve-text --lib join::tests::表の区間を書き出す -- --ignored --nocapture` が、同じ方法で
//! 作り直した `RANGES` の本体を出すので、下の `RANGES` に貼り替え、`GRAPHEME_JOIN_UNICODE_VERSION` を新しい版にする。
//! 規則が増えて、表に無い 2 つの char の間で境界が消えるようになったら、試験 `表に無い_char_の後ろで…` が落ちる。その
//! ときは 03 §14.4 の表の定義と `derive` の調べ方を先に見直す。

/// 表を作った `unicode-segmentation` の Unicode の版 (`unicode_segmentation::UNICODE_VERSION` と一致することを試験で確かめる)。
pub const GRAPHEME_JOIN_UNICODE_VERSION: (u64, u64, u64) = (17, 0, 0);

/// 表 (コードポイントの閉区間の昇順)。`unicode-segmentation` 1.13.3 (Unicode 17.0.0) から作った。
#[rustfmt::skip]
const RANGES: &[(u32, u32)] = &[
    (0x0300, 0x036F), (0x0483, 0x0489), (0x0591, 0x05BD), (0x05BF, 0x05BF), (0x05C1, 0x05C2), (0x05C4, 0x05C5),
    (0x05C7, 0x05C7), (0x0600, 0x0605), (0x0610, 0x061A), (0x064B, 0x065F), (0x0670, 0x0670), (0x06D6, 0x06DD),
    (0x06DF, 0x06E4), (0x06E7, 0x06E8), (0x06EA, 0x06ED), (0x070F, 0x070F), (0x0711, 0x0711), (0x0730, 0x074A),
    (0x07A6, 0x07B0), (0x07EB, 0x07F3), (0x07FD, 0x07FD), (0x0816, 0x0819), (0x081B, 0x0823), (0x0825, 0x0827),
    (0x0829, 0x082D), (0x0859, 0x085B), (0x0890, 0x0891), (0x0897, 0x089F), (0x08CA, 0x0903), (0x093A, 0x093C),
    (0x093E, 0x094F), (0x0951, 0x0957), (0x0962, 0x0963), (0x0981, 0x0983), (0x09BC, 0x09BC), (0x09BE, 0x09C4),
    (0x09C7, 0x09C8), (0x09CB, 0x09CD), (0x09D7, 0x09D7), (0x09E2, 0x09E3), (0x09FE, 0x09FE), (0x0A01, 0x0A03),
    (0x0A3C, 0x0A3C), (0x0A3E, 0x0A42), (0x0A47, 0x0A48), (0x0A4B, 0x0A4D), (0x0A51, 0x0A51), (0x0A70, 0x0A71),
    (0x0A75, 0x0A75), (0x0A81, 0x0A83), (0x0ABC, 0x0ABC), (0x0ABE, 0x0AC5), (0x0AC7, 0x0AC9), (0x0ACB, 0x0ACD),
    (0x0AE2, 0x0AE3), (0x0AFA, 0x0AFF), (0x0B01, 0x0B03), (0x0B3C, 0x0B3C), (0x0B3E, 0x0B44), (0x0B47, 0x0B48),
    (0x0B4B, 0x0B4D), (0x0B55, 0x0B57), (0x0B62, 0x0B63), (0x0B82, 0x0B82), (0x0BBE, 0x0BC2), (0x0BC6, 0x0BC8),
    (0x0BCA, 0x0BCD), (0x0BD7, 0x0BD7), (0x0C00, 0x0C04), (0x0C3C, 0x0C3C), (0x0C3E, 0x0C44), (0x0C46, 0x0C48),
    (0x0C4A, 0x0C4D), (0x0C55, 0x0C56), (0x0C62, 0x0C63), (0x0C81, 0x0C83), (0x0CBC, 0x0CBC), (0x0CBE, 0x0CC4),
    (0x0CC6, 0x0CC8), (0x0CCA, 0x0CCD), (0x0CD5, 0x0CD6), (0x0CE2, 0x0CE3), (0x0CF3, 0x0CF3), (0x0D00, 0x0D03),
    (0x0D3B, 0x0D3C), (0x0D3E, 0x0D44), (0x0D46, 0x0D48), (0x0D4A, 0x0D4E), (0x0D57, 0x0D57), (0x0D62, 0x0D63),
    (0x0D81, 0x0D83), (0x0DCA, 0x0DCA), (0x0DCF, 0x0DD4), (0x0DD6, 0x0DD6), (0x0DD8, 0x0DDF), (0x0DF2, 0x0DF3),
    (0x0E31, 0x0E31), (0x0E33, 0x0E3A), (0x0E47, 0x0E4E), (0x0EB1, 0x0EB1), (0x0EB3, 0x0EBC), (0x0EC8, 0x0ECE),
    (0x0F18, 0x0F19), (0x0F35, 0x0F35), (0x0F37, 0x0F37), (0x0F39, 0x0F39), (0x0F3E, 0x0F3F), (0x0F71, 0x0F84),
    (0x0F86, 0x0F87), (0x0F8D, 0x0F97), (0x0F99, 0x0FBC), (0x0FC6, 0x0FC6), (0x102D, 0x1037), (0x1039, 0x103E),
    (0x1056, 0x1059), (0x105E, 0x1060), (0x1071, 0x1074), (0x1082, 0x1082), (0x1084, 0x1086), (0x108D, 0x108D),
    (0x109D, 0x109D), (0x1100, 0x11FF), (0x135D, 0x135F), (0x1712, 0x1715), (0x1732, 0x1734), (0x1752, 0x1753),
    (0x1772, 0x1773), (0x17B4, 0x17D3), (0x17DD, 0x17DD), (0x180B, 0x180D), (0x180F, 0x180F), (0x1885, 0x1886),
    (0x18A9, 0x18A9), (0x1920, 0x192B), (0x1930, 0x193B), (0x1A17, 0x1A1B), (0x1A55, 0x1A5E), (0x1A60, 0x1A60),
    (0x1A62, 0x1A62), (0x1A65, 0x1A7C), (0x1A7F, 0x1A7F), (0x1AB0, 0x1ADD), (0x1AE0, 0x1AEB), (0x1B00, 0x1B04),
    (0x1B34, 0x1B44), (0x1B6B, 0x1B73), (0x1B80, 0x1B82), (0x1BA1, 0x1BAD), (0x1BE6, 0x1BF3), (0x1C24, 0x1C37),
    (0x1CD0, 0x1CD2), (0x1CD4, 0x1CE8), (0x1CED, 0x1CED), (0x1CF4, 0x1CF4), (0x1CF7, 0x1CF9), (0x1DC0, 0x1DFF),
    (0x200C, 0x200D), (0x20D0, 0x20F0), (0x2CEF, 0x2CF1), (0x2D7F, 0x2D7F), (0x2DE0, 0x2DFF), (0x302A, 0x302F),
    (0x3099, 0x309A), (0xA66F, 0xA672), (0xA674, 0xA67D), (0xA69E, 0xA69F), (0xA6F0, 0xA6F1), (0xA802, 0xA802),
    (0xA806, 0xA806), (0xA80B, 0xA80B), (0xA823, 0xA827), (0xA82C, 0xA82C), (0xA880, 0xA881), (0xA8B4, 0xA8C5),
    (0xA8E0, 0xA8F1), (0xA8FF, 0xA8FF), (0xA926, 0xA92D), (0xA947, 0xA953), (0xA960, 0xA97C), (0xA980, 0xA983),
    (0xA9B3, 0xA9C0), (0xA9E5, 0xA9E5), (0xAA29, 0xAA36), (0xAA43, 0xAA43), (0xAA4C, 0xAA4D), (0xAA7C, 0xAA7C),
    (0xAAB0, 0xAAB0), (0xAAB2, 0xAAB4), (0xAAB7, 0xAAB8), (0xAABE, 0xAABF), (0xAAC1, 0xAAC1), (0xAAEB, 0xAAEF),
    (0xAAF5, 0xAAF6), (0xABE3, 0xABEA), (0xABEC, 0xABED), (0xD7B0, 0xD7C6), (0xD7CB, 0xD7FB),
    (0xFB1E, 0xFB1E), (0xFE00, 0xFE0F), (0xFE20, 0xFE2F), (0xFF9E, 0xFF9F), (0x101FD, 0x101FD),
    (0x102E0, 0x102E0), (0x10376, 0x1037A), (0x10A01, 0x10A03), (0x10A05, 0x10A06), (0x10A0C, 0x10A0F),
    (0x10A38, 0x10A3A), (0x10A3F, 0x10A3F), (0x10AE5, 0x10AE6), (0x10D24, 0x10D27), (0x10D69, 0x10D6D),
    (0x10EAB, 0x10EAC), (0x10EFA, 0x10EFF), (0x10F46, 0x10F50), (0x10F82, 0x10F85), (0x11000, 0x11002),
    (0x11038, 0x11046), (0x11070, 0x11070), (0x11073, 0x11074), (0x1107F, 0x11082), (0x110B0, 0x110BA),
    (0x110BD, 0x110BD), (0x110C2, 0x110C2), (0x110CD, 0x110CD), (0x11100, 0x11102), (0x11127, 0x11134),
    (0x11145, 0x11146), (0x11173, 0x11173), (0x11180, 0x11182), (0x111B3, 0x111C0), (0x111C2, 0x111C3),
    (0x111C9, 0x111CC), (0x111CE, 0x111CF), (0x1122C, 0x11237), (0x1123E, 0x1123E), (0x11241, 0x11241),
    (0x112DF, 0x112EA), (0x11300, 0x11303), (0x1133B, 0x1133C), (0x1133E, 0x11344), (0x11347, 0x11348),
    (0x1134B, 0x1134D), (0x11357, 0x11357), (0x11362, 0x11363), (0x11366, 0x1136C), (0x11370, 0x11374),
    (0x113B8, 0x113C0), (0x113C2, 0x113C2), (0x113C5, 0x113C5), (0x113C7, 0x113CA), (0x113CC, 0x113D2),
    (0x113E1, 0x113E2), (0x11435, 0x11446), (0x1145E, 0x1145E), (0x114B0, 0x114C3), (0x115AF, 0x115B5),
    (0x115B8, 0x115C0), (0x115DC, 0x115DD), (0x11630, 0x11640), (0x116AB, 0x116B7), (0x1171D, 0x1171F),
    (0x11722, 0x1172B), (0x1182C, 0x1183A), (0x11930, 0x11935), (0x11937, 0x11938), (0x1193B, 0x11943),
    (0x119D1, 0x119D7), (0x119DA, 0x119E0), (0x119E4, 0x119E4), (0x11A01, 0x11A0A), (0x11A33, 0x11A39),
    (0x11A3B, 0x11A3E), (0x11A47, 0x11A47), (0x11A51, 0x11A5B), (0x11A84, 0x11A99), (0x11B60, 0x11B67),
    (0x11C2F, 0x11C36), (0x11C38, 0x11C3F), (0x11C92, 0x11CA7), (0x11CA9, 0x11CB6), (0x11D31, 0x11D36),
    (0x11D3A, 0x11D3A), (0x11D3C, 0x11D3D), (0x11D3F, 0x11D47), (0x11D8A, 0x11D8E), (0x11D90, 0x11D91),
    (0x11D93, 0x11D97), (0x11EF3, 0x11EF6), (0x11F00, 0x11F03), (0x11F34, 0x11F3A), (0x11F3E, 0x11F42),
    (0x11F5A, 0x11F5A), (0x13440, 0x13440), (0x13447, 0x13455), (0x1611E, 0x1612F), (0x16AF0, 0x16AF4),
    (0x16B30, 0x16B36), (0x16D63, 0x16D63), (0x16D67, 0x16D6A), (0x16F4F, 0x16F4F), (0x16F51, 0x16F87),
    (0x16F8F, 0x16F92), (0x16FE4, 0x16FE4), (0x16FF0, 0x16FF1), (0x1BC9D, 0x1BC9E), (0x1CF00, 0x1CF2D),
    (0x1CF30, 0x1CF46), (0x1D165, 0x1D169), (0x1D16D, 0x1D172), (0x1D17B, 0x1D182), (0x1D185, 0x1D18B),
    (0x1D1AA, 0x1D1AD), (0x1D242, 0x1D244), (0x1DA00, 0x1DA36), (0x1DA3B, 0x1DA6C), (0x1DA75, 0x1DA75),
    (0x1DA84, 0x1DA84), (0x1DA9B, 0x1DA9F), (0x1DAA1, 0x1DAAF), (0x1E000, 0x1E006), (0x1E008, 0x1E018),
    (0x1E01B, 0x1E021), (0x1E023, 0x1E024), (0x1E026, 0x1E02A), (0x1E08F, 0x1E08F), (0x1E130, 0x1E136),
    (0x1E2AE, 0x1E2AE), (0x1E2EC, 0x1E2EF), (0x1E4EC, 0x1E4EF), (0x1E5EE, 0x1E5EF), (0x1E6E3, 0x1E6E3),
    (0x1E6E6, 0x1E6E6), (0x1E6EE, 0x1E6EF), (0x1E6F5, 0x1E6F5), (0x1E8D0, 0x1E8D6), (0x1E944, 0x1E94A),
    (0x1F1E6, 0x1F1FF), (0x1F3FB, 0x1F3FF), (0xE0020, 0xE007F), (0xE0100, 0xE01EF),
];

/// ビット表で引く範囲: 基本多言語面と追加多言語面 (`..BITS_END`) と、第 14 面の先頭 (`P14..P14_END`。タグ・異体字
/// セレクタ補助)。ほかは区間を二分探索する (今の表では、どれも表に無い)。
const BITS_END: u32 = 0x2_0000;
const P14: u32 = 0xE_0000;
const P14_END: u32 = 0xE_1000;
const BIT_SLOTS: usize = (BITS_END + (P14_END - P14)) as usize;

/// コードポイント `v` のビット表の位置。
#[inline]
const fn bit_slot(v: u32) -> Option<usize> {
    if v < BITS_END {
        Some(v as usize)
    } else if v >= P14 && v < P14_END {
        Some((BITS_END + v - P14) as usize)
    } else {
        None
    }
}

/// `RANGES` のうちビット表で引く範囲を 1 ビットずつにした表 (コンパイル時に作る)。
static BITS: [u64; BIT_SLOTS / 64] = build_bits();

const fn build_bits() -> [u64; BIT_SLOTS / 64] {
    let mut out = [0u64; BIT_SLOTS / 64];
    let mut i = 0;
    while i < RANGES.len() {
        let (lo, hi) = RANGES[i];
        let mut c = lo;
        while c <= hi {
            if let Some(j) = bit_slot(c) {
                out[j / 64] |= 1 << (j % 64);
            }
            c += 1;
        }
        i += 1;
    }
    out
}

/// `c` が書記素の境界の規則に関わる char (表にある) か。表に無い 2 つの char の間は必ず書記素の境界 (CR + LF を除く。
/// CR・LF は行の内容に現れない。03 §14.4 の「走査」)。
#[inline]
pub fn may_join_grapheme(c: char) -> bool {
    let v = c as u32;
    if let Some(j) = bit_slot(v) {
        return BITS[j / 64] >> (j % 64) & 1 != 0;
    }
    RANGES
        .binary_search_by(|&(lo, hi)| {
            if hi < v {
                std::cmp::Ordering::Less
            } else if lo > v {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_segmentation::GraphemeCursor;

    /// `s` の位置 `at` が書記素の境界でない (UAX #29、`unicode-segmentation`)。
    fn joined(s: &str, at: usize) -> bool {
        !GraphemeCursor::new(at, s.len(), true)
            .is_boundary(s, 0)
            .unwrap_or(true)
    }

    /// 表の作り方: `c` を短い文字列に置いて、表に無い隣と結合する場所があるかを調べる。
    fn derive(c: char) -> bool {
        let s = c.to_string();
        let n = s.len();
        joined(&format!("a{s}"), 1) // Extend・ZWJ・`SpacingMark` (GB9・GB9a)
            || joined(&format!("{s}a"), n) // Prepend (GB9b)
            || joined(&format!("{s}{s}"), n) // `Regional_Indicator` (GB12)
            || joined(&format!("{s}\u{AC00}"), n) // LV の前: L (GB6)
            || joined(&format!("\u{AC00}{s}"), 3) // LV の後ろ: V・T (GB7)
            || joined(&format!("\u{0915}\u{094D}{s}\u{0915}"), 6 + n) // 子音の前: InCB の Linker・Extend (GB9c)
    }

    /// 表の区間 (`RANGES` の本体) を `derive` から作り直して書き出す (版を上げたときの手順。モジュールの説明)。
    #[test]
    #[ignore = "表を作り直すときだけ手で走らせる"]
    #[expect(clippy::print_stdout, reason = "表の本体を書き出して貼り替える")]
    fn 表の区間を書き出す() {
        let mut ranges: Vec<(u32, u32)> = Vec::new();
        for v in 0..=0x10_FFFFu32 {
            if !char::from_u32(v).is_some_and(derive) {
                continue;
            }
            match ranges.last_mut() {
                Some(r) if r.1 + 1 == v => r.1 = v,
                _ => ranges.push((v, v)),
            }
        }
        let mut line = String::new();
        for (lo, hi) in ranges {
            let item = format!("(0x{lo:04X}, 0x{hi:04X}), ");
            if line.len() + item.len() > 110 {
                println!("    {}", line.trim_end());
                line.clear();
            }
            line.push_str(&item);
        }
        println!("    {}", line.trim_end());
        println!(
            "// UNICODE_VERSION = {:?}",
            unicode_segmentation::UNICODE_VERSION
        );
    }

    // verifies: AC-NFR-106-5
    #[test]
    fn 表の_unicode_の版は書記素の分割の版と同じ() {
        assert_eq!(
            GRAPHEME_JOIN_UNICODE_VERSION,
            unicode_segmentation::UNICODE_VERSION,
            "unicode-segmentation の版が変わった: 表 (RANGES) を作り直して版を揃える (03 §14.4 の「走査」)"
        );
    }

    // verifies: AC-NFR-106-5
    #[test]
    fn 表は全コードポイントで書記素の分割の振る舞いから作り直した表と同じ() {
        let mut differ = Vec::new();
        for v in 0..=0x10_FFFFu32 {
            let Some(c) = char::from_u32(v) else { continue };
            if may_join_grapheme(c) != derive(c) {
                differ.push(format!("U+{v:04X}"));
            }
        }
        assert!(
            differ.is_empty(),
            "{} 件: {:?}",
            differ.len(),
            &differ[..differ.len().min(20)]
        );
    }

    /// 表に無い char の代表 (`Grapheme_Cluster_Break` の Other・Control、`Extended_Pictographic`、`InCB` の Consonant、
    /// ハングルの音節 LV・LVT、エスケープ文字など)。
    const REPS: &[char] = &[
        'a',
        '\t',
        '\u{1}',
        '\u{7F}',
        '\u{85}',
        '\u{AD}',
        '\u{200B}',
        '\u{2028}',
        '\u{FEFF}',
        '\u{E0001}',
        'あ',
        '漢',
        '\u{1F44D}',
        '\u{1F600}',
        '\u{A9}',
        '\u{203C}',
        '\u{2764}',
        '\u{1F000}',
        '\u{1FFFD}',
        '\u{1F1E5}',
        '\u{0915}',
        '\u{0995}',
        '\u{0A95}',
        '\u{0D15}',
        '\u{10FF80}',
        '\u{FFFC}',
        '\u{0E01}',
        '\u{AC00}',
        '\u{D55C}',
    ];

    /// 左の文脈 (`GB9c`・GB11・GB12/13・`GB9b`・GB6〜8 の左側になりうる列)。
    const CTX: &[&str] = &[
        "",
        "\u{0915}\u{094D}",
        "\u{0915}\u{094D}\u{093C}",
        "\u{0915}\u{093C}\u{094D}\u{200D}",
        "\u{1F44D}\u{200D}",
        "\u{1F44D}\u{1F3FB}\u{200D}",
        "\u{1F1EF}",
        "\u{1F1EF}\u{1F1EF}\u{1F1EF}",
        "\u{0600}",
        "\u{1100}",
        "\u{AC00}",
        "\u{1100}\u{1161}",
    ];

    /// 表に無い 2 つの char `x`・`y` の間が、左の文脈 `CTX` に依らず境界か (境界でない組を返す)。
    fn not_split(xs: &mut dyn Iterator<Item = char>, ys: &[char]) -> Vec<String> {
        let mut bad = Vec::new();
        for x in xs.filter(|&x| !may_join_grapheme(x)) {
            for &y in ys.iter().filter(|&&y| !may_join_grapheme(y)) {
                for c in CTX {
                    let s = format!("{c}{x}{y}");
                    if joined(&s, c.len() + x.len_utf8()) && bad.len() < 50 {
                        bad.push(format!("{c:?} U+{:04X} U+{:04X}", x as u32, y as u32));
                    }
                }
            }
        }
        bad
    }

    /// 表に無い char は、表に無い代表の char の後ろでも前でも結合しない (`Extended_Pictographic` は ZWJ の後ろ (GB11)、
    /// `InCB` の Consonant は Linker の後ろ (`GB9c`)、ハングルの音節は L の後ろ・V / T の前 (GB6〜8) でだけ隣と結合し、
    /// その ZWJ・Linker・字母は表にある)。
    // verifies: AC-NFR-106-5
    #[test]
    fn 表に無い_char_の後ろでも前でも表に無い_char_は結合しない() {
        let all = || (0..=0x10_FFFFu32).filter_map(char::from_u32);
        for v in all() {
            if may_join_grapheme(v) {
                continue;
            }
            let s = v.to_string();
            for x in ["a", "あ", "\u{1F600}", "\u{0915}", "\u{10FF80}", "\u{AC00}"] {
                assert!(
                    !joined(&format!("{x}{s}"), x.len()),
                    "U+{:04X} after {x:?}",
                    v as u32
                );
                assert!(
                    !joined(&format!("{s}{x}"), s.len()),
                    "U+{:04X} before {x:?}",
                    v as u32
                );
            }
        }
    }

    /// 上の性質を、左の文脈 (`CTX`) を付けて、全コードポイント × 代表で確かめる (重い。release で走らせる)。
    #[test]
    #[ignore = "重い (cargo test --release -p eve-text --lib join -- --ignored)"]
    fn 表に無い_2_つの_char_の間は左の文脈に依らず境界_全コードポイント() {
        let all = || (0..=0x10_FFFFu32).filter_map(char::from_u32);
        let bad1 = not_split(&mut all(), REPS);
        let ys: Vec<char> = all().collect();
        let bad2 = not_split(&mut REPS.iter().copied(), &ys);
        assert!(bad1.is_empty() && bad2.is_empty(), "{bad1:?}\n{bad2:?}");
    }

    /// 同じ性質を、代表どうしで文脈を付けて確かめる (軽い。毎回走らせる)。
    #[test]
    fn 表に無い_2_つの_char_の間は左の文脈に依らず境界_代表() {
        let bad = not_split(&mut REPS.iter().copied(), REPS);
        assert!(bad.is_empty(), "{bad:?}");
    }

    /// 高速経路の前提: 印字できる ASCII と、通常の行の近道の文字 (かな・漢字など)・絵文字・子音・ハングルの音節は表に無い。
    #[test]
    fn 印字できる_ascii_と通常の文字は表に無い() {
        for b in 0x20u8..=0x7E {
            assert!(!may_join_grapheme(char::from(b)), "{b:#x}");
        }
        for c in [
            '\t',
            '\r',
            '\n',
            'あ',
            'ア',
            '漢',
            '、',
            'Ａ',
            '\u{3000}',
            '\u{10FF80}',
            '\u{1F600}',
            '\u{0915}',
            '\u{A9}',
            '\u{AC00}',
            '\u{D55C}',
        ] {
            assert!(!may_join_grapheme(c), "{c:?}");
        }
        for c in [
            '\u{301}',
            '\u{200D}',
            '\u{FE0F}',
            '\u{1F1EF}',
            '\u{1F3FB}',
            '\u{1100}',
            '\u{1161}',
            '\u{11A8}',
            '\u{094D}',
            '\u{0600}',
            '\u{3099}',
            '\u{E0100}',
        ] {
            assert!(may_join_grapheme(c), "{c:?}");
        }
    }
}
