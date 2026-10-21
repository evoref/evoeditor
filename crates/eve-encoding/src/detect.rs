//! 文字コードの判定 (docs/03-core-design.md §8.2、FR-108 / 109 / 115)。
//!
//! 順序は ADR-0009 に従い、UTF-16 の BOM 判定を NUL 判定より先に行う。手順: 1 BOM → 2 明示指定 (手順 1 に優先) →
//! 3 UTF-16 無 BOM → 4 バイナリ → 4' ISO-2022-JP → 5 / 6 UTF-8 → 7 chardetng (事後フィルタ) → 8 設定の既定。
//! §8.2.1 の「後半で別の文字コードが現れた」ときの再判定は `redetect_window` で行う (読込ジョブが契機を見つけて呼ぶ)。

use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};

use crate::decode::Decoder;
use crate::encoding::Encoding;

/// 明示指定できる文字コード (`OpenOptions.forced_encoding`)。
pub type ForcedEncoding = Encoding;

/// 判定の入力。
#[derive(Clone, Copy, Debug)]
pub struct DetectOptions {
    /// `OpenOptions.forced_encoding` (FR-111)。手順 1 に優先する。
    pub forced: Option<ForcedEncoding>,
    /// `file.default_encoding` (07 §5.2。`system` は 05 §6 で解決した値)。手順 7 (ii)(iii) と手順 8 で使う。
    pub default_encoding: Encoding,
    /// `file.detection_candidates` (手順 7 の事後フィルタ)。`Encoding::ALL` の各文字コードに対応するビット。
    candidates: u8,
}

impl Default for DetectOptions {
    fn default() -> DetectOptions {
        DetectOptions {
            forced: None,
            default_encoding: Encoding::Utf8,
            candidates: 0b11_1111,
        }
    }
}

fn bit(e: Encoding) -> u8 {
    1 << Encoding::ALL.iter().position(|x| *x == e).unwrap_or(0)
}

impl DetectOptions {
    pub fn with_forced(mut self, e: Option<Encoding>) -> DetectOptions {
        self.forced = e;
        self
    }

    pub fn with_default_encoding(mut self, e: Encoding) -> DetectOptions {
        self.default_encoding = e;
        self
    }

    /// 手順 7 で採ってよい文字コード (既定はすべて)。
    pub fn with_candidates(mut self, c: &[Encoding]) -> DetectOptions {
        self.candidates = c.iter().fold(0, |m, e| m | bit(*e));
        self
    }

    fn is_candidate(&self, e: Encoding) -> bool {
        self.candidates & bit(e) != 0
    }
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

/// 対応していない BOM の種類 (本文を読まずに確認を求める)。UTF-16 は M3 から復号するので、ここには UTF-32 だけが来る。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnsupportedBom {
    Utf32Le,
    Utf32Be,
    /// 以前の版 (UTF-16 を復号しなかった) の名残。`detect` は返さない。
    Utf16Le,
    Utf16Be,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DetectedKind {
    /// UTF-8。`bom_len` は先頭で読み飛ばすバイト数 (0 または 3)。
    Utf8 { bom_len: usize },
    /// UTF-8 以外の文字コード (UTF-16LE/BE・Shift_JIS・EUC-JP・ISO-2022-JP)。`bom_len` は UTF-16 の BOM (0 または 2)。
    Encoded { encoding: Encoding, bom_len: usize },
    /// 対応していない BOM がある。UI は「読み取り専用で開く / 文字コードを指定して開く / 中止」を示す。
    UnsupportedBom(UnsupportedBom),
    /// バイナリの疑い (§8.2 手順 4)。「文字コードを指定して開く」を常に示す。
    Binary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Detection {
    pub kind: DetectedKind,
    pub confidence: Confidence,
    /// 手順 7 (iii): chardetng が v0.1 の 7 種にない文字コードを推定した (その名前)。`info.open.guess_unsupported` で案内する。
    pub unsupported_guess: Option<&'static str>,
}

impl Detection {
    fn new(kind: DetectedKind, confidence: Confidence) -> Detection {
        Detection {
            kind,
            confidence,
            unsupported_guess: None,
        }
    }

    /// 判定した文字コード (バイナリ・未対応の BOM は `None`)。
    pub fn encoding(&self) -> Option<Encoding> {
        match self.kind {
            DetectedKind::Utf8 { .. } => Some(Encoding::Utf8),
            DetectedKind::Encoded { encoding, .. } => Some(encoding),
            _ => None,
        }
    }

    /// 先頭で読み飛ばす BOM のバイト数。
    pub fn bom_len(&self) -> usize {
        match self.kind {
            DetectedKind::Utf8 { bom_len } | DetectedKind::Encoded { bom_len, .. } => bom_len,
            _ => 0,
        }
    }
}

/// `enc` で確定した結果 (BOM があれば読み飛ばす長さを付ける)。
fn of_encoding(enc: Encoding, bom_len: usize, confidence: Confidence) -> Detection {
    let kind = match enc {
        Encoding::Utf8 => DetectedKind::Utf8 { bom_len },
        e => DetectedKind::Encoded {
            encoding: e,
            bom_len,
        },
    };
    Detection::new(kind, confidence)
}

/// C0 制御文字の割合の閾値 (パーセント)。
const C0_PERCENT_LIMIT: usize = 5;

/// 手順 3 の閾値 (パーセント): 一方の NUL 比率が 30% 超、他方が 1% 未満。
const UTF16_NUL_HIGH: usize = 30;
const UTF16_NUL_LOW: usize = 1;
/// 手順 3 を試す最小の長さ (バイト)。比率が意味を持つだけの標本が要る。
const UTF16_MIN_BYTES: usize = 16;

/// 先頭 `sniff` (既定 64 KiB まで、ファイルが小さければ全体) から判定する。
/// `file_complete`: `sniff` がファイルの末尾まで含むか (末尾で切れた多バイト列・エスケープ列の許容に使う)。
pub fn detect(sniff: &[u8], file_complete: bool, opts: &DetectOptions) -> Detection {
    // 手順 2: 明示指定は手順 1 に優先する。指定した文字コードの BOM と一致するときだけ BOM として読み飛ばす
    if let Some(forced) = opts.forced {
        let bom_len = forced
            .bom()
            .filter(|b| sniff.starts_with(b))
            .map_or(0, <[u8]>::len);
        return of_encoding(forced, bom_len, Confidence::Certain);
    }
    // 手順 1: 長い BOM から
    let unsupported =
        |kind| Detection::new(DetectedKind::UnsupportedBom(kind), Confidence::Certain);
    if sniff.starts_with(b"\xFF\xFE\x00\x00") {
        return unsupported(UnsupportedBom::Utf32Le);
    }
    if sniff.starts_with(b"\x00\x00\xFE\xFF") {
        return unsupported(UnsupportedBom::Utf32Be);
    }
    for e in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        let bom = e.bom().unwrap_or_default();
        if sniff.starts_with(bom) {
            return of_encoding(e, bom.len(), Confidence::Certain);
        }
    }
    // 手順 3: UTF-16 の BOM なし (ASCII 主体のときだけ効く)
    if let Some(e) = utf16_without_bom(sniff, file_complete) {
        return of_encoding(e, 0, Confidence::Medium);
    }
    // 手順 4: バイナリ (NUL が 1 つ以上、または C0 制御文字 (TAB / LF / CR / FF / ESC を除く) が 5% 超)
    if is_binary(sniff) {
        return Detection::new(DetectedKind::Binary, Confidence::Medium);
    }
    // 手順 4': ISO-2022-JP
    if is_iso_2022_jp(sniff, file_complete) {
        return of_encoding(Encoding::Iso2022Jp, 0, Confidence::Medium);
    }
    // 手順 5 / 6: UTF-8 の妥当性 (ファイルの途中で切れた末尾の多バイト列は許容)
    match std::str::from_utf8(sniff) {
        Ok(_) if sniff.is_ascii() => {
            return of_encoding(Encoding::Utf8, 0, Confidence::Low); // ASCII のみ: UTF-8 と仮定 (再判定の対象)
        }
        Ok(_) => return of_encoding(Encoding::Utf8, 0, Confidence::High),
        Err(e) if e.error_len().is_none() && !file_complete => {
            let c = if sniff[..e.valid_up_to()].is_ascii() {
                Confidence::Low
            } else {
                Confidence::High
            };
            return of_encoding(Encoding::Utf8, 0, c);
        }
        Err(_) => {}
    }
    // 手順 7: chardetng
    guess_with_chardetng(sniff, file_complete, opts)
}

/// 手順 7 (事後フィルタ)。
fn guess_with_chardetng(sniff: &[u8], file_complete: bool, opts: &DetectOptions) -> Detection {
    let mut det = EncodingDetector::new(Iso2022JpDetection::Allow);
    det.feed(sniff, file_complete);
    let guess = det.guess(None, Utf8Detection::Deny);
    let supported = Encoding::ALL.iter().copied().find(|e| e.rs() == guess);
    let d = opts.default_encoding;
    match supported {
        // (i) 7 種に含まれ、候補にも含まれる
        Some(e) if opts.is_candidate(e) => of_encoding(e, 0, Confidence::Medium),
        // (ii) 7 種に含まれるが候補にない: 既定で復号して不正バイトが 0 なら既定、あれば推定
        Some(e) => {
            let mut dec = Decoder::new(d, false);
            let mut s = String::new();
            dec.decode(sniff, file_complete, &mut s);
            let pick = if dec.invalid_bytes() == 0 { d } else { e };
            of_encoding(pick, 0, Confidence::Low)
        }
        // (iii) 7 種にない (windows-1252・GBK・Big5 等): 採らず既定で開き、不正バイトは保持する
        None => Detection {
            unsupported_guess: Some(guess.name()),
            ..of_encoding(d, 0, Confidence::Low)
        },
    }
}

/// 手順 3: 偶数長で、偶数または奇数バイト位置の NUL 比率が 30% 超かつ他方が 1% 未満。
fn utf16_without_bom(sniff: &[u8], file_complete: bool) -> Option<Encoding> {
    // ファイルの途中で切れているときは、末尾の 1 バイトを数えない
    let len = if file_complete {
        if !sniff.len().is_multiple_of(2) {
            return None;
        }
        sniff.len()
    } else {
        sniff.len() & !1
    };
    // 短すぎる入力 (バイナリの断片など) は比率が当てにならない
    if len < UTF16_MIN_BYTES {
        return None;
    }
    let (mut even, mut odd) = (0usize, 0usize);
    for (i, &b) in sniff[..len].iter().enumerate() {
        if b == 0 {
            if i % 2 == 0 {
                even += 1;
            } else {
                odd += 1;
            }
        }
    }
    let half = len / 2;
    let high = |n: usize| n * 100 > half * UTF16_NUL_HIGH;
    let low = |n: usize| n * 100 < half * UTF16_NUL_LOW;
    // BE は上位バイト (偶数位置) が NUL、LE は下位の次 (奇数位置) が NUL
    if high(even) && low(odd) {
        Some(Encoding::Utf16Be)
    } else if high(odd) && low(even) {
        Some(Encoding::Utf16Le)
    } else {
        None
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

/// 手順 4' / §8.2.1 のエスケープ列 (`ESC $ @` / `ESC $ B` / `ESC ( I`) の最初の位置。
pub fn find_iso_switch(bytes: &[u8]) -> Option<usize> {
    bytes.windows(3).position(|w| {
        w[0] == 0x1B && matches!((w[1], w[2]), (b'$', b'@') | (b'$', b'B') | (b'(', b'I'))
    })
}

/// 手順 4': 7 bit で、文字集合を切り替える ESC 列を含み、sniff 範囲全体が Malformed なしに復号できる。
fn is_iso_2022_jp(sniff: &[u8], file_complete: bool) -> bool {
    if !sniff.is_ascii() || find_iso_switch(sniff).is_none() {
        return false;
    }
    let mut dec = Decoder::new(Encoding::Iso2022Jp, false);
    let mut s = String::new();
    // 範囲がファイルの末尾に達していなければ `last = false` (末尾の未完のシーケンスは Malformed にしない)
    dec.decode(sniff, file_complete, &mut s);
    dec.invalid_bytes() == 0
}

/// §8.2.1 の再判定: 信頼度「低」のとき、不正な UTF-8 または ISO-2022-JP のエスケープ列を初めて見つけた位置の周辺 `window`
/// (呼び側が 64 KiB を切り出す。ISO-2022-JP が契機なら窓の先頭をそのエスケープ列の位置にする) を判定し直す。
/// 別の文字コードが推定されれば `Some` (読込ジョブを先頭からやり直す)。窓が 7 bit のときは手順 4' だけを行い、
/// 不合格なら `None` (UTF-8 のまま読み続ける)。`window_complete`: 窓の末尾がファイルの末尾か。
pub fn redetect_window(
    window: &[u8],
    window_complete: bool,
    opts: &DetectOptions,
) -> Option<Detection> {
    if window.is_ascii() {
        return is_iso_2022_jp(window, window_complete)
            .then(|| of_encoding(Encoding::Iso2022Jp, 0, Confidence::Medium));
    }
    if std::str::from_utf8(window).is_ok() {
        return None;
    }
    let d = guess_with_chardetng(window, window_complete, opts);
    (d.encoding() != Some(Encoding::Utf8)).then_some(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(b: &[u8]) -> Detection {
        detect(b, true, &DetectOptions::default())
    }

    fn enc(b: &[u8]) -> Option<Encoding> {
        d(b).encoding()
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
            DetectedKind::Encoded {
                encoding: Encoding::Utf16Le,
                bom_len: 2
            }
        );
        assert_eq!(
            d(b"\xFE\xFF\x00a").kind,
            DetectedKind::Encoded {
                encoding: Encoding::Utf16Be,
                bom_len: 2
            }
        );
        assert_eq!(d(b"\xEF\xBB\xBF").confidence, Confidence::Certain);
    }

    #[test]
    fn 明示指定は_bom_の判定に優先する() {
        let forced = DetectOptions::default().with_forced(Some(Encoding::Utf8));
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
        let sj = DetectOptions::default().with_forced(Some(Encoding::ShiftJis));
        assert_eq!(
            detect(b"\xEF\xBB\xBF", true, &sj).kind,
            DetectedKind::Encoded {
                encoding: Encoding::ShiftJis,
                bom_len: 0
            }
        );
        let u16 = DetectOptions::default().with_forced(Some(Encoding::Utf16Le));
        assert_eq!(detect(b"\xFF\xFEa\x00", true, &u16).bom_len(), 2);
        assert_eq!(detect(b"\xFE\xFFa\x00", true, &u16).bom_len(), 0);
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
            "ファイルの末尾なら不正な UTF-8 (手順 7 へ。推定できなければ既定の UTF-8、信頼度 低)"
        );
    }

    #[test]
    fn utf16_無_bom_は_ascii_主体のときだけ() {
        let le: Vec<u8> = "hello world, this is text\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(enc(&le), Some(Encoding::Utf16Le));
        assert_eq!(d(&le).confidence, Confidence::Medium);
        let be: Vec<u8> = "hello world, this is text\n"
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect();
        assert_eq!(enc(&be), Some(Encoding::Utf16Be));
        // 奇数長 (ファイル全体) は UTF-16 にしない → NUL があるのでバイナリ
        let mut odd = le.clone();
        odd.push(b'x');
        assert_eq!(d(&odd).kind, DetectedKind::Binary);
        // 日本語主体の UTF-16 は NUL が少なく、バイナリ扱いになり得る (FR-111 で回復する)
        let jp: Vec<u8> = "日本語のテキストです。"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_ne!(enc(&jp), Some(Encoding::Utf16Le));
    }

    #[test]
    fn iso_2022_jp_の判定() {
        let jis = b"abc\x1B$B$\"$$\x1B(B def\n";
        assert_eq!(enc(jis), Some(Encoding::Iso2022Jp));
        assert_eq!(d(jis).confidence, Confidence::Medium);
        // 半角カナ (ESC ( I)
        assert_eq!(enc(b"\x1B(I1\x1B(B\n"), Some(Encoding::Iso2022Jp));
        // ANSI エスケープ・ESC ( J だけ・補助漢字は確定しない (ASCII の UTF-8)
        assert_eq!(enc(b"\x1B[31mred\x1B[0m\n"), Some(Encoding::Utf8));
        assert_eq!(enc(b"a\x1B(J\x5C\x1B(B\n"), Some(Encoding::Utf8));
        assert_eq!(enc(b"\x1B$(D!!\x1B(B"), Some(Encoding::Utf8));
        // 8 bit の混在は手順 4' を通らない
        assert_ne!(enc(b"\x1B$B$\"\x1B(B\x80"), Some(Encoding::Iso2022Jp));
    }

    #[test]
    fn iso_2022_jp_はファイルの途中で切れた末尾の列を許容する() {
        let cut = b"abc\x1B$B$\"\x1B"; // ESC 列の途中で切れている
        assert_eq!(
            detect(cut, false, &DetectOptions::default()).encoding(),
            Some(Encoding::Iso2022Jp)
        );
        assert_ne!(
            detect(cut, true, &DetectOptions::default()).encoding(),
            Some(Encoding::Iso2022Jp),
            "ファイル全体なら末尾の未完は Malformed"
        );
    }

    #[test]
    fn 日本語の_8_bit_は_chardetng_で推定する() {
        let sjis = encoding_rs::SHIFT_JIS
            .encode("日本語のテキストエディタです。これは Shift_JIS で書かれた文章です。\n")
            .0
            .into_owned();
        assert_eq!(enc(&sjis), Some(Encoding::ShiftJis));
        let euc = encoding_rs::EUC_JP
            .encode("日本語のテキストエディタです。これは EUC-JP で書かれた文章です。\n")
            .0
            .into_owned();
        assert_eq!(enc(&euc), Some(Encoding::EucJp));
    }

    #[test]
    fn 候補にない推定は既定で復号できれば既定を採る() {
        let sjis = encoding_rs::SHIFT_JIS
            .encode("日本語のテキストエディタです。これは文章です。\n")
            .0
            .into_owned();
        // 候補を EUC-JP だけにすると、Shift_JIS の推定は (ii)。既定の UTF-8 では不正バイトがあるので推定を採る (信頼度 低)
        let o = DetectOptions::default().with_candidates(&[Encoding::EucJp]);
        let r = detect(&sjis, true, &o);
        assert_eq!(r.encoding(), Some(Encoding::ShiftJis));
        assert_eq!(r.confidence, Confidence::Low);
        // 既定を Shift_JIS にすると既定で復号できるので既定
        let o = o.with_default_encoding(Encoding::ShiftJis);
        assert_eq!(detect(&sjis, true, &o).encoding(), Some(Encoding::ShiftJis));
    }

    #[test]
    fn 対応外の推定は既定で開いて案内する() {
        // windows-1252 の文章 (é = 0xE9) は 7 種にない → 既定 (UTF-8) で開き、推定の名前を返す
        let latin =
            b"caf\xE9 au lait, na\xEFve r\xE9sum\xE9 and more text here for the detector.\n";
        let r = d(latin);
        assert_eq!(r.encoding(), Some(Encoding::Utf8));
        assert_eq!(r.confidence, Confidence::Low);
        assert!(r.unsupported_guess.is_some(), "{r:?}");
    }

    #[test]
    fn 再判定の窓() {
        let o = DetectOptions::default();
        // 7 bit の窓は手順 4' だけ
        assert_eq!(
            redetect_window(b"\x1B$B$\"\x1B(B", true, &o).and_then(|d| d.encoding()),
            Some(Encoding::Iso2022Jp)
        );
        assert!(redetect_window(b"plain ascii", true, &o).is_none());
        // 妥当な UTF-8 は再判定しない
        assert!(redetect_window("日本語".as_bytes(), true, &o).is_none());
        // Shift_JIS の窓
        let sjis = encoding_rs::SHIFT_JIS
            .encode("日本語のテキストエディタです。これは文章です。\n")
            .0
            .into_owned();
        assert_eq!(
            redetect_window(&sjis, true, &o).and_then(|d| d.encoding()),
            Some(Encoding::ShiftJis)
        );
        assert_eq!(find_iso_switch(b"abc\x1B$Bxyz"), Some(3));
        assert_eq!(find_iso_switch(b"abc\x1B[0m"), None);
    }
}
