//! 文書の形式情報 (docs/02 §5.1、docs/03 §8.1 / §9.6): 改行の統計、`TextFormat`、読み取り専用の理由、読込状態。

use eve_encoding::{Encoding, PuaEscapeMode, RoundtripSummary};
use eve_text::{ByteOff, LineEnding, TextRead};

/// 改行の種類別の数。`CRLF` は 1 つと数える (03 §2.4)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EolStats {
    pub lf: usize,
    pub crlf: usize,
    pub cr: usize,
}

/// 長大行の閾値の既定 (`file.long_line_threshold`)。これを超える行を含む文書は読み取り専用で開く (NFR-106)。
pub const LONG_LINE_THRESHOLD: usize = 1 << 20;
/// 閾値を超える行のうち、表示とカーソルの対象にする先頭の大きさ (書記素の境界で切る)。
pub const LONG_LINE_VISIBLE: usize = 64 * 1024;

/// 閾値 `threshold` (バイト) を超える行があるか。チャンクを 1 回走査する (読込ジョブで使う)。
pub fn has_long_line(text: &dyn TextRead, threshold: usize) -> bool {
    let mut run = 0usize;
    for chunk in text.chunks(eve_text::ByteRange::all(text.len_bytes())) {
        let b = chunk.as_bytes();
        let mut at = 0;
        while let Some(i) = memchr::memchr2(b'\n', b'\r', &b[at..]) {
            run += i;
            if run > threshold {
                return true;
            }
            run = 0;
            at += i + 1;
        }
        run += b.len() - at;
        if run > threshold {
            return true;
        }
    }
    false
}

impl EolStats {
    /// 文書全体を数える (読込時)。
    pub fn count(text: &dyn TextRead) -> EolStats {
        count_range(text, 0, text.len_bytes())
    }

    pub fn total(&self) -> usize {
        self.lf + self.crlf + self.cr
    }

    /// 2 種類以上があるか (ステータスバーの「混在」)。
    pub fn is_mixed(&self) -> bool {
        [self.lf, self.crlf, self.cr]
            .iter()
            .filter(|&&n| n > 0)
            .count()
            >= 2
    }

    /// 多数派の改行。同数なら LF。改行が全くなければ `fallback` (設定の既定、03 §8.4)。
    pub fn majority(&self, fallback: LineEnding) -> LineEnding {
        if self.total() == 0 {
            return fallback;
        }
        if self.crlf > self.lf && self.crlf >= self.cr {
            LineEnding::Crlf
        } else if self.cr > self.lf && self.cr > self.crlf {
            LineEnding::Cr
        } else {
            LineEnding::Lf
        }
    }

    /// `self - removed + added` (各成分を 0 で止める)。
    pub(crate) fn replaced(self, removed: EolStats, added: EolStats) -> EolStats {
        let f = |cur: usize, rem: usize, add: usize| {
            let v = cur as i128 - rem as i128 + add as i128;
            debug_assert!(v >= 0, "eol_stats が負になった");
            v.max(0) as usize
        };
        EolStats {
            lf: f(self.lf, removed.lf, added.lf),
            crlf: f(self.crlf, removed.crlf, added.crlf),
            cr: f(self.cr, removed.cr, added.cr),
        }
    }
}

fn byte_at(text: &dyn TextRead, i: usize) -> Option<u8> {
    if i >= text.len_bytes() {
        return None;
    }
    let (chunk, start) = text.chunk_at(ByteOff(i));
    chunk.as_bytes().get(i - start.0).copied()
}

/// `[start, end)` の改行を数える。範囲の端が CRLF の `\r` と `\n` の間に当たるときは 1 バイト外へ広げる
/// (03 §6.5「eol_stats の更新」)。
pub(crate) fn count_range(text: &dyn TextRead, start: usize, end: usize) -> EolStats {
    let len = text.len_bytes();
    let (mut s, mut e) = (start.min(len), end.min(len));
    if s > 0 && byte_at(text, s) == Some(b'\n') && byte_at(text, s - 1) == Some(b'\r') {
        s -= 1;
    }
    if e > 0 && e < len && byte_at(text, e - 1) == Some(b'\r') && byte_at(text, e) == Some(b'\n') {
        e += 1;
    }
    let mut st = EolStats::default();
    // CR / LF だけを探して飛ぶ (本文の大半は改行でないので、1 バイトずつ見ない。10 MB の長大行で数 ms 縮む)
    let mut prev_cr = false;
    for chunk in text.chunks(eve_text::ByteRange::new(ByteOff(s), ByteOff(e))) {
        let bytes = chunk.as_bytes();
        let mut i = 0;
        if prev_cr {
            // 前のチャンクの末尾の CR: このチャンクの先頭が LF なら CRLF
            prev_cr = false;
            if bytes.first() == Some(&b'\n') {
                st.crlf += 1;
                i = 1;
            } else {
                st.cr += 1;
            }
        }
        while let Some(off) = memchr::memchr2(b'\r', b'\n', &bytes[i..]) {
            let p = i + off;
            if bytes[p] == b'\n' {
                st.lf += 1;
                i = p + 1;
            } else {
                match bytes.get(p + 1) {
                    Some(b'\n') => {
                        st.crlf += 1;
                        i = p + 2;
                    }
                    Some(_) => {
                        st.cr += 1;
                        i = p + 1;
                    }
                    None => {
                        prev_cr = true;
                        i = p + 1;
                    }
                }
            }
        }
    }
    if prev_cr {
        st.cr += 1;
    }
    st
}

/// 書式と読み取り専用の理由の組。再読込 (`Origin::Reload`) が本文と一緒に差し替え、Undo / Redo が本文と一緒に戻す (03 §7.2)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatState {
    pub format: TextFormat,
    pub read_only: ReadOnlyReasons,
}

/// Undo ノードに持つ書式の変更 (前後の値)。Undo は `before`、Redo は `after` へ戻す。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatChange {
    pub before: FormatState,
    pub after: FormatState,
}

/// 文書の形式 (03 §8.1)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextFormat {
    pub encoding: Encoding,
    /// BOM の種類は `encoding` から決まる (UTF-8 / UTF-16LE / UTF-16BE)。保存時に同じものを書く。
    pub has_bom: bool,
    /// 新規に挿入する改行 (文書全体の改行コードではない)。
    pub default_eol: LineEnding,
    pub eol_stats: EolStats,
    /// 不正バイトの保持マーカーの数。0 でなければ読み取り専用 (NFR-304)。
    pub invalid_bytes: u32,
    pub pua_escape: PuaEscapeMode,
    /// 読込時の往復の検査 (03 §8.7。v0.1 は件数と先頭の位置だけ)。
    pub roundtrip: RoundtripSummary,
    /// 開いたときの長大行の閾値 (`file.long_line_threshold`)。設定を変えても開いている文書には遡らない (07 §6)。
    pub long_line_threshold: usize,
}

impl TextFormat {
    /// 新規文書 (無題・存在しないパス)。`default_eol` は全 OS で LF。
    pub fn new_document() -> TextFormat {
        TextFormat {
            encoding: Encoding::Utf8,
            has_bom: false,
            default_eol: LineEnding::Lf,
            eol_stats: EolStats::default(),
            invalid_bytes: 0,
            pua_escape: PuaEscapeMode::Enabled,
            roundtrip: RoundtripSummary::default(),
            long_line_threshold: LONG_LINE_THRESHOLD,
        }
    }
}

/// 読み取り専用の理由 (03 §9.6)。宣言順が表示の優先順。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ReadOnlyReason {
    FileAttribute = 0,
    UserRequested,
    NoWritePermission,
    InvalidBytes,
    StatefulEncodingWithInvalidBytes,
    Binary,
    LargeFileViewer,
    LoadIncomplete,
    LongLine,
}

const ALL_REASONS: [ReadOnlyReason; 9] = [
    ReadOnlyReason::FileAttribute,
    ReadOnlyReason::UserRequested,
    ReadOnlyReason::NoWritePermission,
    ReadOnlyReason::InvalidBytes,
    ReadOnlyReason::StatefulEncodingWithInvalidBytes,
    ReadOnlyReason::Binary,
    ReadOnlyReason::LargeFileViewer,
    ReadOnlyReason::LoadIncomplete,
    ReadOnlyReason::LongLine,
];

/// 理由の集合 (ビットセット)。「読込中」は理由にしない (`LoadState` から導出する)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReadOnlyReasons(u16);

impl ReadOnlyReasons {
    pub fn insert(&mut self, r: ReadOnlyReason) {
        self.0 |= 1 << (r as u8);
    }

    pub fn remove(&mut self, r: ReadOnlyReason) {
        self.0 &= !(1 << (r as u8));
    }

    pub fn contains(&self, r: ReadOnlyReason) -> bool {
        self.0 & (1 << (r as u8)) != 0
    }

    pub fn is_empty(&self) -> bool {
        self.0 == 0
    }

    /// 優先順に返す。
    pub fn iter(&self) -> impl Iterator<Item = ReadOnlyReason> + '_ {
        ALL_REASONS.into_iter().filter(|r| self.contains(*r))
    }

    /// 表示に出す 1 つ (優先順で先頭)。
    /// `file.toggle_read_only` で解除できるか (理由があり、すべてが `FileAttribute` か `UserRequested`。03 §9.6 の表)。
    pub fn releasable(&self) -> bool {
        !self.is_empty()
            && self.iter().all(|r| {
                matches!(
                    r,
                    ReadOnlyReason::FileAttribute | ReadOnlyReason::UserRequested
                )
            })
    }

    pub fn primary(&self) -> Option<ReadOnlyReason> {
        self.iter().next()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoadState {
    Loading,
    #[default]
    Ready,
    Failed,
}

#[cfg(test)]
mod tests {
    use eve_text::Text;

    use super::*;

    #[test]
    fn 改行を数える() {
        let t = Text::from("a\nb\r\nc\rd\r\n\n");
        assert_eq!(
            EolStats::count(&t),
            EolStats {
                lf: 2,
                crlf: 2,
                cr: 1
            }
        );
        assert!(EolStats::count(&t).is_mixed());
        assert_eq!(EolStats::count(&Text::from("abc")), EolStats::default());
        assert_eq!(
            EolStats::count(&Text::from("\r\r\n\n\r")),
            EolStats {
                lf: 1,
                crlf: 1,
                cr: 2
            }
        );
    }

    #[test]
    fn 多数派() {
        let s = |lf, crlf, cr| EolStats { lf, crlf, cr };
        assert_eq!(
            s(0, 0, 0).majority(LineEnding::Crlf),
            LineEnding::Crlf,
            "改行がなければ設定"
        );
        assert_eq!(
            s(2, 2, 0).majority(LineEnding::Crlf),
            LineEnding::Lf,
            "同数なら LF"
        );
        assert_eq!(s(1, 3, 0).majority(LineEnding::Lf), LineEnding::Crlf);
        assert_eq!(s(1, 0, 3).majority(LineEnding::Lf), LineEnding::Cr);
        assert_eq!(
            s(0, 2, 2).majority(LineEnding::Lf),
            LineEnding::Crlf,
            "CRLF と CR が同数なら CRLF"
        );
    }

    #[test]
    fn 範囲の端が_crlf_を割るときは広げる() {
        let t = Text::from("a\r\nb");
        // [2, 4) は '\n' から始まる → '\r' を含めて CRLF 1 つ
        assert_eq!(
            count_range(&t, 2, 4),
            EolStats {
                lf: 0,
                crlf: 1,
                cr: 0
            }
        );
        // [0, 2) は '\r' で終わる → '\n' を含めて CRLF 1 つ
        assert_eq!(
            count_range(&t, 0, 2),
            EolStats {
                lf: 0,
                crlf: 1,
                cr: 0
            }
        );
    }

    #[test]
    fn 読み取り専用の理由は優先順() {
        let mut r = ReadOnlyReasons::default();
        assert!(r.is_empty() && r.primary().is_none());
        r.insert(ReadOnlyReason::LongLine);
        r.insert(ReadOnlyReason::InvalidBytes);
        r.insert(ReadOnlyReason::FileAttribute);
        assert_eq!(r.primary(), Some(ReadOnlyReason::FileAttribute));
        assert_eq!(
            r.iter().collect::<Vec<_>>(),
            [
                ReadOnlyReason::FileAttribute,
                ReadOnlyReason::InvalidBytes,
                ReadOnlyReason::LongLine
            ]
        );
        r.remove(ReadOnlyReason::FileAttribute);
        assert_eq!(r.primary(), Some(ReadOnlyReason::InvalidBytes));
        assert!(!r.contains(ReadOnlyReason::FileAttribute));
    }

    /// 基準: 1 バイトずつ数える。
    fn count_naive(bytes: &[u8]) -> EolStats {
        let mut st = EolStats::default();
        let mut prev_cr = false;
        for &b in bytes {
            if prev_cr {
                prev_cr = false;
                if b == b'\n' {
                    st.crlf += 1;
                    continue;
                }
                st.cr += 1;
            }
            match b {
                b'\r' => prev_cr = true,
                b'\n' => st.lf += 1,
                _ => {}
            }
        }
        if prev_cr {
            st.cr += 1;
        }
        st
    }

    #[test]
    fn 改行の数え方は_1_バイトずつ数えた結果と同じ() {
        let mut x = 0x2545_F491_4F6C_DD1Du64;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let alphabet = ["a", "\r", "\n", "日", "\r\n", "b"];
        for case in 0..60 {
            // 長さを変える (ropey のチャンクをまたぐ長さを含む)。CR / LF の多い列と少ない列
            let len = [0usize, 1, 7, 100, 3000, 40_000][case % 6];
            let dense = case % 2 == 0;
            let mut s = String::new();
            for _ in 0..len {
                let k = if dense {
                    next() % 6
                } else {
                    [0, 0, 0, 0, 3, 5][(next() % 6) as usize]
                };
                s.push_str(alphabet[k as usize % 6]);
                if !dense && next() % 50 == 0 {
                    s.push_str(alphabet[1 + (next() % 3) as usize % 3]);
                }
            }
            let t = Text::from(s.as_str());
            assert_eq!(
                EolStats::count(&t),
                count_naive(s.as_bytes()),
                "case {case}"
            );
            // 範囲の途中から (CR と LF の間で切る場合を含む)
            for _ in 0..10 {
                if s.is_empty() {
                    break;
                }
                let a = (next() as usize) % s.len();
                let b = a + (next() as usize) % (s.len() - a + 1);
                if !s.is_char_boundary(a) || !s.is_char_boundary(b) {
                    continue;
                }
                let got = count_range(&t, a, b);
                // 基準: 範囲が CR と LF の間で切れていれば広げる (count_range と同じ約束)
                let (mut ra, mut rb) = (a, b);
                let bytes = s.as_bytes();
                if ra > 0 && bytes.get(ra) == Some(&b'\n') && bytes[ra - 1] == b'\r' {
                    ra -= 1;
                }
                if rb > 0 && rb < bytes.len() && bytes[rb - 1] == b'\r' && bytes[rb] == b'\n' {
                    rb += 1;
                }
                assert_eq!(got, count_naive(&bytes[ra..rb]), "case {case} {a}..{b}");
            }
        }
    }
}
