//! 読込時の往復の検査 (docs/03-core-design.md §8.7 の「検出は読込時にストリーミングで行う」)。
//!
//! UTF-8 / UTF-16 は区分 A (1:1) `なので検査しない。Shift_JIS` / EUC-JP / ISO-2022-JP では、元のバイト列を **行ごと**
//! (区切りは本文の行 (03 §2.4) と同じ LF・CR・CRLF。Shift_JIS・EUC-JP の CR / LF は後続バイトにならず、ISO-2022-JP は
//! 行ごとに ASCII 状態へ戻す E6 の単位。v0.3 までは LF だけで区切っていた。03 §8.7.1 の「検査の行と長い行」) に復号し、
//! 保存と同じ方式で再符号化して元のバイト列と比べる。一致しない行を「別のバイト表現で保存される」行として数える
//! (`RoundtripSummary`、v0.1)。表現不能の文字を含む行は E3 の置換 (または E1 の報告) を経ないと保存できないので、
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
    /// 長すぎる行があり、検査を打ち切った (その行から後ろは検査していない)。
    pub incomplete: bool,
}

impl RoundtripSummary {
    /// 元の文字コードのまま書くと、別のバイト表現になる箇所があり得るか (03 §8.6 E2' の確認が要るか)。
    ///
    /// 差のある行を数えたときと、検査を打ち切ったとき (検査していない範囲に差があり得る。安全側、Issue #495)。
    #[must_use]
    pub fn may_be_lossy(&self) -> bool {
        self.lossy_lines > 0 || self.incomplete
    }
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
    /// 直前のチャンクが CR で終わった (行はまだ閉じていない。次のチャンクの先頭が LF なら、その CRLF で閉じる)。
    pending_cr: bool,
}

/// `bytes` の中の最初の行の終端 (LF・CR・CRLF) を探し、(終端の先頭の位置, 終端の長さ) を返す。
/// `bytes` が CR で終わるときは、その後ろが LF かどうかがまだ分からないので、長さを `None` にする。
fn find_terminator(bytes: &[u8]) -> Option<(usize, Option<usize>)> {
    let i = bytes.iter().position(|&b| b == b'\n' || b == b'\r')?;
    if bytes[i] == b'\n' {
        return Some((i, Some(1)));
    }
    match bytes.get(i + 1) {
        Some(b'\n') => Some((i, Some(2))),
        Some(_) => Some((i, Some(1))),
        None => Some((i, None)),
    }
}

/// 行の終端で終わる行に分ける (終端は行に含める。最後の行は終端が無いことがある)。
/// 終端は LF・CR・CRLF。末尾の単独の CR は、その行の終端として扱う。
fn split_lines(mut bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    std::iter::from_fn(move || {
        if bytes.is_empty() {
            return None;
        }
        let end = match find_terminator(bytes) {
            Some((i, len)) => i + len.unwrap_or(1),
            None => bytes.len(),
        };
        let (line, rest) = bytes.split_at(end);
        bytes = rest;
        Some(line)
    })
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
            pending_cr: false,
        }
    }

    pub fn feed(&mut self, chunk: &[u8], last: bool) {
        if !self.enabled {
            return;
        }
        let mut rest = chunk;
        // 前のチャンクが CR で終わっていた: 先頭が LF なら CRLF、そうでなければ CR だけで行を閉じる
        if self.pending_cr && !rest.is_empty() {
            self.pending_cr = false;
            if rest[0] == b'\n' {
                self.push(&rest[..1]);
                rest = &rest[1..];
            }
            self.finish_line();
        }
        while let Some((i, len)) = find_terminator(rest) {
            let Some(len) = len else {
                // チャンクの末尾の CR: 次のチャンクを見てから閉じる
                self.push(rest);
                rest = &[];
                self.pending_cr = true;
                break;
            };
            self.push(&rest[..i + len]);
            self.finish_line();
            rest = &rest[i + len..];
        }
        self.push(rest);
        if last {
            self.pending_cr = false;
        }
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
    /// (100 MB の `Shift_JIS` で、全行を再符号化すると数秒かかるため)。
    fn flush_block(&mut self) {
        let block = std::mem::take(&mut self.block);
        let mut start = self.block_offset;
        for line in split_lines(&block) {
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
/// - `Shift_JIS`: 2 バイト文字のうち、再符号化で別の符号になるもの (NEC 特殊文字 `0x87`、NEC 選択 IBM 拡張 `0xED` `0xEE`、
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
        // LF・CRLF・CR を混ぜる (CRLF の間・CR の直後で分かれる場合を含む)
        let b = b"abc\n\xFA\x54\r\n\x93\xFA\r\xFA\x54\r\r\n\xFA\x54\n\r";
        let whole = check(Encoding::ShiftJis, b);
        assert_eq!(whole.lossy_lines, 3);
        for step in 1..=7 {
            let mut c = RoundtripChecker::new(Encoding::ShiftJis);
            let mut it = b.chunks(step).peekable();
            while let Some(ch) = it.next() {
                c.feed(ch, it.peek().is_none());
            }
            assert_eq!(c.finish(), whole, "step {step}");
        }
        // 最後のチャンクが空でも同じ (末尾の CR は行を閉じる)
        let mut c = RoundtripChecker::new(Encoding::ShiftJis);
        c.feed(b, false);
        c.feed(b"", true);
        assert_eq!(c.finish(), whole);
    }

    /// 全ての行を、取りこぼしの判定を使わずに正確に検査した結果 (基準)。
    fn reference(enc: Encoding, bytes: &[u8]) -> RoundtripSummary {
        let mut c = RoundtripChecker::new(enc);
        let mut start = 0u64;
        for line in split_lines(bytes) {
            c.check_line(line, start);
            start += line.len() as u64;
        }
        c.finish()
    }

    #[test]
    fn 行は_lf_cr_crlf_で区切る() {
        let lines = |b: &'static [u8]| split_lines(b).collect::<Vec<_>>();
        assert_eq!(
            lines(b"a\nb\r\nc\rd"),
            vec![&b"a\n"[..], b"b\r\n", b"c\r", b"d"]
        );
        // 空の行、末尾の単独の CR、CR が続く場合
        assert_eq!(
            lines(b"\n\r\r\n\r"),
            vec![&b"\n"[..], b"\r", b"\r\n", b"\r"]
        );
        assert_eq!(lines(b""), Vec::<&[u8]>::new());
    }

    #[test]
    fn cr_だけの改行でも行ごとに数える() {
        // v0.3 までは LF だけで区切っていたので、CR だけの改行のファイルは全体が 1 行になっていた
        let b = b"abc\r\xFA\x54\r\x81\xCA\r\xFA\x54";
        let s = check(Encoding::ShiftJis, b);
        assert_eq!(s.lossy_lines, 2);
        assert_eq!(s.first_offsets, vec![4, 10]);
        // CRLF の行頭の位置
        let b = b"abc\r\n\xFA\x54\r\nxyz\r\n";
        let s = check(Encoding::ShiftJis, b);
        assert_eq!(s.lossy_lines, 1);
        assert_eq!(s.first_offsets, vec![5]);
    }

    #[test]
    fn iso_2022_jp_の_cr_の行も行ごとに比べる() {
        // 1 行目 (旧 JIS の `ESC $ @`) だけが差。CR で区切られた 2 行目 (正規形) は差にならない
        let b = b"\x1B$@$\"\x1B(B\r\x1B$B$\"\x1B(B\r";
        let s = check(Encoding::Iso2022Jp, b);
        assert_eq!(s.lossy_lines, 1);
        assert_eq!(s.first_offsets, vec![0]);
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
                    // 行の終端は LF・CRLF・CR を混ぜる
                    bytes.extend_from_slice([&b"\n"[..], b"\r\n", b"\r"][(next() % 3) as usize]);
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
            for a in 0u8..=u8::MAX {
                for b in 0u8..=u8::MAX {
                    let line = [a, b, b'\n'];
                    assert!(
                        may_differ(enc, &line) || !differs(&line),
                        "{enc:?} {line:02X?}"
                    );
                    // 前に ASCII があっても、2 バイト目を先頭と取り違えない
                    let line = [b'a', a, b, b'\n'];
                    assert!(
                        may_differ(enc, &line) || !differs(&line),
                        "{enc:?} {line:02X?}"
                    );
                }
                if enc == Encoding::EucJp {
                    for b in 0u8..=u8::MAX {
                        let line = [0x8F, a, b, b'\n'];
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
        let summary = c.finish();
        assert!(summary.incomplete);
        assert_eq!(summary.lossy_lines, 0);
        assert!(
            summary.may_be_lossy(),
            "検査していない範囲があるので、確認が要る"
        );
        assert!(!RoundtripSummary::default().may_be_lossy());
    }
}
