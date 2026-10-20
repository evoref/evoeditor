//! 文書の形式情報 (docs/02 §5.1、docs/03 §8.1 / §9.6): 改行の統計、`TextFormat`、読み取り専用の理由、読込状態。

use eve_encoding::PuaEscapeMode;
use eve_text::{ByteOff, LineEnding, TextRead};

/// 改行の種類別の数。`CRLF` は 1 つと数える (03 §2.4)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EolStats {
    pub lf: usize,
    pub crlf: usize,
    pub cr: usize,
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
    let mut prev_cr = false;
    for chunk in text.chunks(eve_text::ByteRange::new(ByteOff(s), ByteOff(e))) {
        for &b in chunk.as_bytes() {
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
    }
    if prev_cr {
        st.cr += 1;
    }
    st
}

/// 文書の形式 (03 §8.1)。`encoding` は M1 では UTF-8 だけなので持たない (他の文字コードの版で足す)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextFormat {
    pub has_bom: bool,
    /// 新規に挿入する改行 (文書全体の改行コードではない)。
    pub default_eol: LineEnding,
    pub eol_stats: EolStats,
    /// 不正バイトの保持マーカーの数。0 でなければ読み取り専用 (NFR-304)。
    pub invalid_bytes: u32,
    pub pua_escape: PuaEscapeMode,
}

impl TextFormat {
    /// 新規文書 (無題・存在しないパス)。`default_eol` は全 OS で LF。
    pub fn new_document() -> TextFormat {
        TextFormat {
            has_bom: false,
            default_eol: LineEnding::Lf,
            eol_stats: EolStats::default(),
            invalid_bytes: 0,
            pua_escape: PuaEscapeMode::Enabled,
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
}
