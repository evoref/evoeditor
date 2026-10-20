//! 位置と範囲の型 (docs/03-core-design.md §3.1)。位置の正準表現は UTF-8 のバイトオフセット (ADR-0010)。

use std::fmt;

/// UTF-8 のバイトオフセット。文書の位置の **正準** の表現 (`Pos`)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ByteOff(pub usize);

/// 位置 (`ByteOff` の別名)。
pub type Pos = ByteOff;

/// Unicode スカラ値の番号。ropey の内部の単位で、`eve-text` の外には出さない方針 (03 §3.1)。
/// 現在は変換関数の戻り値として公開しているが、使うのは `eve-text` の内部と変換のテストに限る。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CharIdx(pub usize);

/// 行番号 (0 始まり)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LineIdx(pub usize);

/// UTF-16 のコード単位のオフセット (LSP 用、v1.L)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Utf16Off(pub usize);

/// バイトの範囲 `[start, end)`。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ByteRange {
    pub start: ByteOff,
    pub end: ByteOff,
}

impl ByteRange {
    /// `start <= end` でなければ、`end` を `start` に揃えて空の範囲にする。
    pub fn new(start: ByteOff, end: ByteOff) -> ByteRange {
        ByteRange {
            start,
            end: end.max(start),
        }
    }

    /// 全体 `[0, len)`。
    pub fn all(len: usize) -> ByteRange {
        ByteRange {
            start: ByteOff(0),
            end: ByteOff(len),
        }
    }

    /// 範囲のバイト数。フィールドが公開なので、逆順の範囲でもパニックせず 0 を返す。
    pub fn len(&self) -> usize {
        self.end.0.saturating_sub(self.start.0)
    }

    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }
}

impl From<std::ops::Range<usize>> for ByteRange {
    fn from(r: std::ops::Range<usize>) -> ByteRange {
        ByteRange::new(ByteOff(r.start), ByteOff(r.end))
    }
}

/// 行を終える改行の種類 (03 §2.4)。行区切りは LF・CRLF・CR の 3 種だけ。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LineEnding {
    Lf,
    Crlf,
    Cr,
}

impl LineEnding {
    /// 改行のバイト数 (CRLF は 2)。
    pub fn byte_len(self) -> usize {
        match self {
            LineEnding::Crlf => 2,
            LineEnding::Lf | LineEnding::Cr => 1,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::Crlf => "\r\n",
            LineEnding::Cr => "\r",
        }
    }
}

macro_rules! impl_display {
    ($($t:ty),*) => {$(
        impl fmt::Display for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    )*};
}
impl_display!(ByteOff, CharIdx, LineIdx, Utf16Off);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 範囲は_start_が_end_を超えない() {
        let r = ByteRange::new(ByteOff(5), ByteOff(2));
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
        assert_eq!(ByteRange::from(2..5).len(), 3);
    }

    #[test]
    fn 改行のバイト数() {
        assert_eq!(LineEnding::Crlf.byte_len(), 2);
        assert_eq!(LineEnding::Cr.as_str(), "\r");
    }
}
