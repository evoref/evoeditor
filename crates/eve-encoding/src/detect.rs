//! 文字コードの判定 (docs/03-core-design.md §8.2)。M1 の範囲は手順 1 (BOM)・2 (明示指定)・4 (バイナリ)・5 / 6 (UTF-8) と、
//! 手順 8 (推定できなければ UTF-8 で開き、不正バイトを保持する)。UTF-16 の BOM なし判定 (手順 3)・ISO-2022-JP (手順 4')・
//! chardetng による推定 (手順 7) は、それぞれの文字コードの対応と同時に足す。

/// 判定の入力。
#[derive(Clone, Copy, Debug, Default)]
pub struct DetectOptions {
    /// `OpenOptions.forced_encoding` (FR-111)。手順 1 に優先する。
    pub forced: Option<ForcedEncoding>,
}

/// 明示指定できる文字コード (M1 は UTF-8 だけ)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ForcedEncoding {
    Utf8,
}

/// 判定の確からしさ (§8.2.1 の再判定は「低」が対象)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    Low,
    Medium,
    High,
    /// BOM または明示指定。
    Certain,
}

/// 対応していない BOM の種類 (本文を読まずに確認を求める)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnsupportedBom {
    Utf32Le,
    Utf32Be,
    /// M1 では UTF-16 を復号しない (文字コードを指定して開く扱い)。
    Utf16Le,
    Utf16Be,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DetectedKind {
    /// UTF-8。`bom_len` は先頭で読み飛ばすバイト数 (0 または 3)。
    Utf8 { bom_len: usize },
    /// 対応していない BOM がある。UI は「読み取り専用で開く / 文字コードを指定して開く / 中止」を示す。
    UnsupportedBom(UnsupportedBom),
    /// バイナリの疑い (§8.2 手順 4)。「文字コードを指定して開く」を常に示す。
    Binary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Detection {
    pub kind: DetectedKind,
    pub confidence: Confidence,
}

/// C0 制御文字の割合の閾値 (パーセント)。
const C0_PERCENT_LIMIT: usize = 5;

/// 先頭 `sniff` (既定 64 KiB まで、ファイルが小さければ全体) から判定する。
/// `file_complete`: `sniff` がファイルの末尾まで含むか (末尾で切れた UTF-8 の許容に使う)。
pub fn detect(sniff: &[u8], file_complete: bool, opts: &DetectOptions) -> Detection {
    // 手順 2: 明示指定は手順 1 に優先する。指定した文字コードの BOM と一致するときだけ BOM として読み飛ばす
    if let Some(ForcedEncoding::Utf8) = opts.forced {
        let bom_len = if sniff.starts_with(b"\xEF\xBB\xBF") {
            3
        } else {
            0
        };
        return Detection {
            kind: DetectedKind::Utf8 { bom_len },
            confidence: Confidence::Certain,
        };
    }
    // 手順 1: 長い BOM から
    let bom = |kind| Detection {
        kind: DetectedKind::UnsupportedBom(kind),
        confidence: Confidence::Certain,
    };
    if sniff.starts_with(b"\xFF\xFE\x00\x00") {
        return bom(UnsupportedBom::Utf32Le);
    }
    if sniff.starts_with(b"\x00\x00\xFE\xFF") {
        return bom(UnsupportedBom::Utf32Be);
    }
    if sniff.starts_with(b"\xEF\xBB\xBF") {
        return Detection {
            kind: DetectedKind::Utf8 { bom_len: 3 },
            confidence: Confidence::Certain,
        };
    }
    if sniff.starts_with(b"\xFF\xFE") {
        return bom(UnsupportedBom::Utf16Le);
    }
    if sniff.starts_with(b"\xFE\xFF") {
        return bom(UnsupportedBom::Utf16Be);
    }
    // 手順 4: バイナリ (NUL が 1 つ以上、または C0 制御文字 (TAB / LF / CR / FF / ESC を除く) が 5% 超)
    if is_binary(sniff) {
        return Detection {
            kind: DetectedKind::Binary,
            confidence: Confidence::Medium,
        };
    }
    // 手順 5 / 6: UTF-8 の妥当性 (ファイルの途中で切れた末尾の多バイト列は許容)
    let confidence = match std::str::from_utf8(sniff) {
        Ok(_) if sniff.is_ascii() => Confidence::Low, // ASCII のみ: UTF-8 と仮定 (再判定の対象)
        Ok(_) => Confidence::High,
        // ファイルの途中で切れた末尾の多バイト列だけが不正なら許容する
        Err(e) if e.error_len().is_none() && !file_complete => {
            if sniff[..e.valid_up_to()].is_ascii() {
                Confidence::Low
            } else {
                Confidence::High
            }
        }
        // 手順 8: 推定できない → UTF-8 で開き、不正バイトは保持する
        Err(_) => Confidence::Low,
    };
    Detection {
        kind: DetectedKind::Utf8 { bom_len: 0 },
        confidence,
    }
}

fn is_binary(sniff: &[u8]) -> bool {
    if sniff.is_empty() {
        return false;
    }
    let mut c0 = 0usize;
    for &b in sniff {
        if b == 0 {
            return true;
        }
        if b < 0x20 && !matches!(b, b'\t' | b'\n' | b'\r' | 0x0C | 0x1B) {
            c0 += 1;
        }
    }
    c0 * 100 > sniff.len() * C0_PERCENT_LIMIT
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(b: &[u8]) -> Detection {
        detect(b, true, &DetectOptions::default())
    }

    #[test]
    fn bom_の判定は長い順() {
        assert_eq!(
            d(b"\xEF\xBB\xBFabc").kind,
            DetectedKind::Utf8 { bom_len: 3 }
        );
        assert_eq!(
            d(b"\xFF\xFE\x00\x00a").kind,
            DetectedKind::UnsupportedBom(UnsupportedBom::Utf32Le)
        );
        assert_eq!(
            d(b"\x00\x00\xFE\xFF").kind,
            DetectedKind::UnsupportedBom(UnsupportedBom::Utf32Be)
        );
        assert_eq!(
            d(b"\xFF\xFEa\x00").kind,
            DetectedKind::UnsupportedBom(UnsupportedBom::Utf16Le)
        );
        assert_eq!(
            d(b"\xFE\xFF\x00a").kind,
            DetectedKind::UnsupportedBom(UnsupportedBom::Utf16Be)
        );
        assert_eq!(d(b"\xEF\xBB\xBF").confidence, Confidence::Certain);
    }

    #[test]
    fn 明示指定は_bom_の判定に優先する() {
        let forced = DetectOptions {
            forced: Some(ForcedEncoding::Utf8),
        };
        let r = detect(b"\xFF\xFE\x00\x00a", true, &forced);
        assert_eq!(
            r.kind,
            DetectedKind::Utf8 { bom_len: 0 },
            "別の文字コードの BOM は本文のバイト"
        );
        assert_eq!(
            detect(b"\xEF\xBB\xBFa", true, &forced).kind,
            DetectedKind::Utf8 { bom_len: 3 }
        );
        assert_eq!(
            detect(b"a\x00b", true, &forced).kind,
            DetectedKind::Utf8 { bom_len: 0 },
            "NUL があってもバイナリにしない"
        );
    }

    #[test]
    fn バイナリ() {
        assert_eq!(d(b"abc\x00def").kind, DetectedKind::Binary);
        assert_eq!(d(&[0x01u8; 10]).kind, DetectedKind::Binary);
        // 5% ちょうどはバイナリにしない。TAB / LF / CR / FF / ESC は数えない
        let mut v = vec![b'a'; 95];
        v.extend([0x01u8; 5]);
        assert_ne!(d(&v).kind, DetectedKind::Binary);
        v.push(0x01);
        assert_eq!(d(&v).kind, DetectedKind::Binary);
        assert_ne!(
            d(b"\t\n\r\x0C\x1B\t\n\r\x0C\x1B").kind,
            DetectedKind::Binary
        );
        assert_ne!(d(b"").kind, DetectedKind::Binary);
    }

    #[test]
    fn utf8_の信頼度() {
        assert_eq!(d(b"hello").confidence, Confidence::Low, "ASCII のみ");
        assert_eq!(d("あいう".as_bytes()).confidence, Confidence::High);
        assert_eq!(d(b"").confidence, Confidence::Low);
        // 不正なバイトがある: UTF-8 で開き、信頼度は低い
        let r = d(b"abc\x82\xA0");
        assert_eq!(
            (r.kind, r.confidence),
            (DetectedKind::Utf8 { bom_len: 0 }, Confidence::Low)
        );
    }

    #[test]
    fn 途中で切れた末尾は許容する() {
        let cut = &"あい".as_bytes()[..4]; // あ + い の 1 バイト目
        assert_eq!(
            detect(cut, false, &DetectOptions::default()).confidence,
            Confidence::High,
            "ファイルの途中なら許容"
        );
        assert_eq!(
            detect(cut, true, &DetectOptions::default()).confidence,
            Confidence::Low,
            "ファイルの末尾なら不正"
        );
    }
}
