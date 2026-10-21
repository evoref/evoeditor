//! 読込時の往復の検査 (docs/03-core-design.md §8.7 の「検出は読込時にストリーミングで行う」)。
//!
//! UTF-8 / UTF-16 は区分 A (1:1) なので検査しない。Shift_JIS / EUC-JP / ISO-2022-JP では、元のバイト列を **行ごと**
//! (LF 区切り。Shift_JIS・EUC-JP の LF は後続バイトにならず、ISO-2022-JP は行ごとに ASCII 状態へ戻す E6 の単位) に復号し、
//! 保存と同じ方式で再符号化して元のバイト列と比べる。一致しない行を「別のバイト表現で保存される」行として数える
//! (`RoundtripInfo::Summary`、v0.1)。表現不能の文字を含む行は E3 の置換 (または E1 の報告) を経ないと保存できないので、
//! ここでは数えず `unmappable_lines` に分ける (§8.7 の区分 D の注記)。
//!
//! 単位は **行** (元の仕様の「箇所」より粗い)。改行を含まない巨大な 1 行は `MAX_LINE` を超えると検査を打ち切る (`incomplete`)。

use crate::decode::Decoder;
use crate::encode::{EncodeReport, Encoder, EscapePolicy};
use crate::encoding::Encoding;

/// 1 行として検査する最大の長さ (バイト)。
pub const MAX_LINE: usize = 4 * 1024 * 1024;
/// 記録する行頭位置の最大件数。
pub const MAX_POSITIONS: usize = 100;

/// 検査の結果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoundtripSummary {
    /// 再符号化で元のバイト列に戻らない行の数。
    pub lossy_lines: u32,
    /// そのうち先頭 100 行の、行頭の元のバイトオフセット。
    pub first_offsets: Vec<u64>,
    /// 表現不能の文字を含む行の数 (数えない)。
    pub unmappable_lines: u32,
    /// 長すぎる行があり、検査を打ち切った。
    pub incomplete: bool,
}

/// ストリーミングの検査器。`feed` を元のバイト列のチャンクごとに呼び、最後のチャンクで `last = true` にする。
pub struct RoundtripChecker {
    enc: Encoding,
    enabled: bool,
    line: Vec<u8>,
    line_offset: u64,
    consumed: u64,
    summary: RoundtripSummary,
}

impl RoundtripChecker {
    pub fn new(enc: Encoding) -> RoundtripChecker {
        RoundtripChecker {
            enc,
            // 区分 A (UTF-8 / UTF-16) は検査しない
            enabled: !(enc == Encoding::Utf8 || enc.is_utf16()),
            line: Vec::new(),
            line_offset: 0,
            consumed: 0,
            summary: RoundtripSummary::default(),
        }
    }

    pub fn feed(&mut self, chunk: &[u8], last: bool) {
        if !self.enabled {
            return;
        }
        let mut rest = chunk;
        while let Some(i) = rest.iter().position(|&b| b == b'\n') {
            self.push(&rest[..=i]);
            self.finish_line();
            rest = &rest[i + 1..];
        }
        self.push(rest);
        if last && !self.line.is_empty() {
            self.finish_line();
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        if self.summary.incomplete {
            self.consumed += bytes.len() as u64;
            return;
        }
        if self.line.len() + bytes.len() > MAX_LINE {
            self.summary.incomplete = true;
            self.line.clear();
            self.consumed += bytes.len() as u64;
            return;
        }
        self.line.extend_from_slice(bytes);
        self.consumed += bytes.len() as u64;
    }

    fn finish_line(&mut self) {
        let line = std::mem::take(&mut self.line);
        let start = self.line_offset;
        self.line_offset = self.consumed;
        if self.summary.incomplete || line.is_empty() {
            return;
        }
        let mut d = Decoder::new(self.enc, false);
        let mut text = String::new();
        d.decode(&line, true, &mut text);
        let mut e = Encoder::new(self.enc, false, EscapePolicy::Restore);
        let (mut out, mut rep) = (Vec::with_capacity(line.len()), EncodeReport::default());
        e.encode(&text, &mut out, &mut rep);
        e.finish(&mut out, &mut rep);
        if !rep.is_empty() {
            self.summary.unmappable_lines += 1;
        } else if out != line {
            self.summary.lossy_lines += 1;
            if self.summary.first_offsets.len() < MAX_POSITIONS {
                self.summary.first_offsets.push(start);
            }
        }
    }

    /// 検査の結果 (最後のチャンクを渡した後)。
    pub fn finish(self) -> RoundtripSummary {
        self.summary
    }
}

/// バイト列全体を検査する (テスト・小さな入力用)。
pub fn check(enc: Encoding, bytes: &[u8]) -> RoundtripSummary {
    let mut c = RoundtripChecker::new(enc);
    c.feed(bytes, true);
    c.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 区分_a_は検査しない() {
        assert_eq!(
            check(Encoding::Utf8, b"abc\n\xFF"),
            RoundtripSummary::default()
        );
        assert_eq!(
            check(Encoding::Utf16Le, b"a\x00"),
            RoundtripSummary::default()
        );
    }

    #[test]
    fn 重複符号化の行を数える() {
        // 0xFA54 (IBM 拡張の ¬) は再符号化で 0x81CA になる
        let b = b"abc\n\xFA\x54\n\x81\xCA\n";
        let s = check(Encoding::ShiftJis, b);
        assert_eq!(s.lossy_lines, 1);
        assert_eq!(s.first_offsets, vec![4]);
        assert_eq!(s.unmappable_lines, 0);
    }

    #[test]
    fn 無損失の行は数えない() {
        let b = b"\x93\xFA\x96\x7B\n\x82\xA0\x82\xA2";
        assert_eq!(check(Encoding::ShiftJis, b), RoundtripSummary::default());
    }

    #[test]
    fn euc_jp_の補助漢字は復号専用で表現不能の行に分ける() {
        // JIS X 0212 の 3 バイト文字 (0x8F 0xB0 0xA1) は復号できても再符号化できない
        let b = b"abc\n\x8F\xB0\xA1\n";
        let s = check(Encoding::EucJp, b);
        assert_eq!(s.unmappable_lines, 1);
        assert_eq!(s.lossy_lines, 0);
    }

    #[test]
    fn iso_2022_jp_の冗長なエスケープは行単位で差になる() {
        // `ESC $ @` (旧 JIS) で書かれた行は、正規形 `ESC $ B` に揃うので差
        let b = b"\x1B$@$\"\x1B(B\n\x1B$B$\"\x1B(B\n";
        let s = check(Encoding::Iso2022Jp, b);
        assert_eq!(s.lossy_lines, 1);
        assert_eq!(s.first_offsets, vec![0]);
    }

    #[test]
    fn チャンクの分け方に依存しない() {
        let b = b"abc\n\xFA\x54\n\x93\xFA\n\xFA\x54";
        let whole = check(Encoding::ShiftJis, b);
        for step in 1..=5 {
            let mut c = RoundtripChecker::new(Encoding::ShiftJis);
            let mut it = b.chunks(step).peekable();
            while let Some(ch) = it.next() {
                c.feed(ch, it.peek().is_none());
            }
            assert_eq!(c.finish(), whole, "step {step}");
        }
    }

    #[test]
    fn 長すぎる行は打ち切る() {
        let mut c = RoundtripChecker::new(Encoding::ShiftJis);
        let chunk = vec![b'a'; 1024 * 1024];
        for _ in 0..5 {
            c.feed(&chunk, false);
        }
        c.feed(b"\n", true);
        assert!(c.finish().incomplete);
    }
}
