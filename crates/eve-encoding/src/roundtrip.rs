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
    /// 完成した行をためておく塊 (まとめて検査する。行ごとに復号器・符号化器を作ると 100 MB で数秒かかるため)。
    block: Vec<u8>,
    /// 塊の先頭の、ファイルの先頭からの位置。
    block_offset: u64,
    /// 塊を検査に回す大きさ (バイト)。
    block_limit: usize,
}

/// 塊の大きさの既定 (バイト)。
const BLOCK_LIMIT: usize = 1024 * 1024;

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
            block: Vec::new(),
            block_offset: 0,
            block_limit: BLOCK_LIMIT,
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
        if last {
            self.flush_block();
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
        if self.block.is_empty() {
            self.block_offset = start;
        }
        self.block.extend_from_slice(&line);
        if self.block.len() >= self.block_limit {
            self.flush_block();
        }
    }

    /// ためた行を検査する。無損失と分かる行 (`may_differ` が偽) は復号・再符号化をせず、残りの行だけを正確に調べる
    /// (100 MB の Shift_JIS で、全行を再符号化すると数秒かかるため)。
    fn flush_block(&mut self) {
        let block = std::mem::take(&mut self.block);
        let mut start = self.block_offset;
        for line in block.split_inclusive(|&b| b == b'\n') {
            if may_differ(self.enc, line) {
                self.check_line(line, start);
            }
            start += line.len() as u64;
        }
    }

    fn check_line(&mut self, line: &[u8], start: u64) {
        let mut d = Decoder::new(self.enc, false);
        let mut text = String::new();
        d.decode(line, true, &mut text);
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

/// 行が、復号して符号化し直すと元と違い得るか (偽なら確実に同じ)。**偽の行は検査を省く** ので、取りこぼしが無いように
/// 保守的に (真の側に倒して) 判定する。取りこぼしが無いことは、全ての 2 バイト (と EUC-JP の `0x8F` の 3 バイト) の組で
/// 実際の復号・符号化と照合するテストで確かめている。
///
/// - Shift_JIS: 2 バイト文字のうち、再符号化で別の符号になるもの (NEC 特殊文字 `0x87`、NEC 選択 IBM 拡張 `0xED` `0xEE`、
///   IBM 拡張 `0xFA` の重複) の先頭バイトを持つ行。ほかの文字・半角カナ・不正バイト (エスケープして元に戻す) は同じに戻る。
/// - EUC-JP: 補助漢字 (`0x8F`、復号専用で符号化できない) と、重複する符号の先頭バイト `0xAD` `0xFC` を持つ行。
/// - ISO-2022-JP: ASCII だけで、エスケープ・シフト (`ESC` `SO` `SI`) を含まない行は同じ。それ以外は検査する。
/// - 状態を持たない符号化の 2 バイト目は 1 バイトずつ飛ばして読む (後ろのバイトを先頭バイトと取り違えない)。
fn may_differ(enc: Encoding, line: &[u8]) -> bool {
    match enc {
        Encoding::ShiftJis => {
            let mut i = 0;
            while i < line.len() {
                let b = line[i];
                if matches!(b, 0x81..=0x9F | 0xE0..=0xFC) {
                    if matches!(b, 0x87 | 0xED | 0xEE | 0xFA) {
                        return true;
                    }
                    if matches!(line.get(i + 1), Some(0x40..=0xFC)) {
                        i += 1;
                    }
                }
                i += 1;
            }
            false
        }
        Encoding::EucJp => {
            let mut i = 0;
            while i < line.len() {
                let b = line[i];
                match b {
                    0x8F | 0xAD | 0xFC => return true,
                    0x8E | 0xA1..=0xFE if matches!(line.get(i + 1), Some(0xA1..=0xFF)) => i += 1,
                    _ => {}
                }
                i += 1;
            }
            false
        }
        _ => !(line.is_ascii() && !line.iter().any(|&b| matches!(b, 0x1B | 0x0E | 0x0F))),
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

    /// 全ての行を、取りこぼしの判定を使わずに正確に検査した結果 (基準)。
    fn reference(enc: Encoding, bytes: &[u8]) -> RoundtripSummary {
        let mut c = RoundtripChecker::new(enc);
        let mut start = 0u64;
        for line in bytes.split_inclusive(|&b| b == b'\n') {
            c.check_line(line, start);
            start += line.len() as u64;
        }
        c.finish()
    }

    // 取りこぼしの判定を使った検査は、全行を正確に検査した結果と同じ (差のある行の数と位置、表現不能の行の数)
    #[test]
    fn 省略した検査は全行の検査と同じ結果になる() {
        // 決定的な擬似乱数で、無損失の行・重複符号化・表現不能・不正バイト・ASCII を混ぜる
        let mut x = 0x2545_F491_4F6C_DD1Du64;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let pieces: [&[u8]; 12] = [
            b"abc",
            b"\x93\xFA\x96\x7B",
            b"\x82\xA0\x82\xA2",
            b"\xFA\x54",
            b"\x81\xCA",
            b"\x8F\xB0\xA1",
            b"\xFF",
            b"\x1B$B$\"\x1B(B",
            b"\x1B$@$\"\x1B(B",
            b"\xED\x40",
            b"\xAD\xA1",
            b"\x81",
        ];
        for enc in [Encoding::ShiftJis, Encoding::EucJp, Encoding::Iso2022Jp] {
            for _ in 0..300 {
                let mut bytes = Vec::new();
                for _ in 0..(next() % 40) {
                    for _ in 0..(next() % 4) {
                        bytes.extend_from_slice(pieces[(next() % 12) as usize]);
                    }
                    bytes.push(b'\n');
                }
                let mut fast = RoundtripChecker::new(enc);
                fast.block_limit = 1 + (next() % 200) as usize;
                fast.feed(&bytes, true);
                assert_eq!(fast.finish(), reference(enc, &bytes), "{enc:?} {bytes:?}");
            }
        }
    }

    // 判定が真でない行は、全ての 2 バイト (と EUC-JP の 0x8F の 3 バイト) の組で、実際に差が出ない
    #[test]
    fn 省略する行は全ての組で差が出ない() {
        for enc in [Encoding::ShiftJis, Encoding::EucJp] {
            let differs = |line: &[u8]| {
                let s = reference(enc, line);
                s.lossy_lines + s.unmappable_lines > 0
            };
            for a in 0u16..=255 {
                for b in 0u16..=255 {
                    let line = [a as u8, b as u8, b'\n'];
                    assert!(
                        may_differ(enc, &line) || !differs(&line),
                        "{enc:?} {line:02X?}"
                    );
                    // 前に ASCII があっても、2 バイト目を先頭と取り違えない
                    let line = [b'a', a as u8, b as u8, b'\n'];
                    assert!(
                        may_differ(enc, &line) || !differs(&line),
                        "{enc:?} {line:02X?}"
                    );
                }
                if enc == Encoding::EucJp {
                    for b in 0u16..=255 {
                        let line = [0x8F, a as u8, b as u8, b'\n'];
                        assert!(
                            may_differ(enc, &line) || !differs(&line),
                            "{enc:?} {line:02X?}"
                        );
                    }
                }
            }
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
