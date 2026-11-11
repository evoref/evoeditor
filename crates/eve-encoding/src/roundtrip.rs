//! 読込時の往復の検査 (docs/03-core-design.md §8.7 の「検出は読込時にストリーミングで行う」) と、保持方式の記録を集めること
//! (§8.7.1、v0.4)。
//!
//! UTF-8 / UTF-16 は区分 A (1:1) `なので検査しない。Shift_JIS` / EUC-JP / ISO-2022-JP では、元のバイト列を **行ごと**
//! (区切りは本文の行 (03 §2.4) と同じ LF・CR・CRLF。Shift_JIS・EUC-JP の CR / LF は後続バイトにならず、ISO-2022-JP は
//! 行ごとに ASCII 状態へ戻す E6 の単位。v0.3 までは LF だけで区切っていた。03 §8.7.1 の「検査の行と長い行」) に復号し、
//! 保存と同じ方式で再符号化して元のバイト列と比べる。一致しない行を「別のバイト表現で保存される」行として数える
//! (`RoundtripSummary`、v0.1)。表現不能の文字を含む行は E3 の置換 (または E1 の報告) を経ないと保存できないので、
//! ここでは数えず `unmappable_lines` に分ける (§8.7 の区分 D の注記)。
//!
//! - **長い行**: `Shift_JIS` / EUC-JP は、1 行が長くても検査を打ち切らない。行が `PIECE_LIMIT` を超えたら、文字の境界で
//!   小分けにして調べる (メモリは小分けの大きさで抑える。行の数え方は 1 行のまま)。ISO-2022-JP は、1 行が `MAX_LINE` を
//!   超えると検査を打ち切る (`incomplete`)。
//! - **記録** (`RoundtripChecker::with_records`、`finish_with_records`): `Shift_JIS` / EUC-JP は、差のある行を文字ごとに
//!   突き合わせて、再符号化で違いが出る連続した文字の範囲を記録にする。ISO-2022-JP は、ASCII 状態で始まり ASCII 状態で
//!   終わる、差のある行の内容を記録にする (行の始まりの状態は、文書の先頭からエスケープシーケンスを追って求める)。
//!   記録の型・上限・不変条件は `preserve` の説明。

use crate::decode::Decoder;
use crate::encode::{EncodeReport, Encoder, EscapePolicy};
use crate::encoding::Encoding;
use crate::preserve::{
    self, Collector, IsoState, PreserveLimits, PreserveReason, PreserveState, PreservedRecords,
};

/// 1 行として検査する最大の長さ (バイト)。ISO-2022-JP だけに使う (`Shift_JIS` / EUC-JP は小分けにして調べる)。
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
    /// 長すぎる行があり、検査を打ち切った (その行から後ろは検査していない)。v0.4 では ISO-2022-JP だけ。
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

/// 1 行 (または長い行の一部) を復号して再符号化した結果。
struct LineCheck {
    /// 復号した文字列。
    text: String,
    /// 再符号化の結果が元のバイト列と違う。
    differs: bool,
    /// 表現不能の文字がある。
    unmappable: bool,
}

/// 小分けにして調べている長い行 (`Shift_JIS` / EUC-JP) の途中の状態。
#[derive(Default)]
struct LongLine {
    /// 次の一部の先頭の、行頭からの位置 (本文の UTF-8 のバイト数)。
    col: u64,
    differs: bool,
    unmappable: bool,
}

/// ISO-2022-JP で記録を集めるときの、文書の通しの状態。
struct IsoTrack {
    /// 次の行の始まりの状態。
    state: IsoState,
    /// 直前の行が単独の CR で終わった (本文では、次の行の終端の LF と CRLF になり得る)。
    prev_cr: bool,
    /// 文書の全体を通しで復号する (不正バイトが 1 つでもあれば、行の対応がずれ得るので記録を捨てる)。
    dec: Decoder,
    sink: String,
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
    pub(crate) block_limit: usize,
    /// 直前のチャンクが CR で終わった (行はまだ閉じていない。次のチャンクの先頭が LF なら、その CRLF で閉じる)。
    pending_cr: bool,
    /// 次に検査する行の行番号 (本文の行番号、0 始まり)。
    line_no: u64,
    /// 今の行を小分けにして調べている途中 (`Shift_JIS` / EUC-JP)。
    long: Option<LongLine>,
    /// 今の行がこの大きさ (バイト) を超えたら、小分けにして調べる (`Shift_JIS` / EUC-JP)。
    pub(crate) piece_limit: usize,
    /// 記録を集める (集めないときは `None`)。
    rec: Option<Collector>,
    /// ISO-2022-JP で記録を集めるときの通しの状態。
    iso: Option<IsoTrack>,
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

/// 行の内容 (末尾の LF・CR・CRLF を除いたもの)。
fn line_content(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\r\n")
        .or_else(|| line.strip_suffix(b"\n"))
        .or_else(|| line.strip_suffix(b"\r"))
        .unwrap_or(line)
}

/// 復号した行の内容 (末尾の LF・CR・CRLF を除いたもの)。
fn text_content(text: &str) -> &str {
    text.strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .or_else(|| text.strip_suffix('\r'))
        .unwrap_or(text)
}

/// 塊の大きさの既定 (バイト)。
const BLOCK_LIMIT: usize = 1024 * 1024;
/// 長い行を小分けにする大きさの既定 (バイト)。
const PIECE_LIMIT: usize = 1024 * 1024;

impl RoundtripChecker {
    /// 記録を集めない検査器 (`RoundtripSummary` だけを数える)。
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
            line_no: 0,
            long: None,
            piece_limit: PIECE_LIMIT,
            rec: None,
            iso: None,
        }
    }

    /// 保持方式の記録も集める検査器 (03 §8.7.1)。区分 A の文字コードでは集めない (`PreserveState::Disabled`)。
    pub fn with_records(enc: Encoding) -> RoundtripChecker {
        let mut c = RoundtripChecker::new(enc);
        if c.enabled {
            c.rec = Some(Collector::new(PreserveLimits::default()));
            if enc == Encoding::Iso2022Jp {
                c.iso = Some(IsoTrack {
                    state: IsoState::Ascii,
                    prev_cr: false,
                    dec: Decoder::new(enc, false),
                    sink: String::new(),
                });
            }
        }
        c
    }

    /// 記録の上限を差し替える (試験用。`feed` の前に呼ぶ)。記録を集めない検査器では何もしない。
    #[must_use]
    pub fn with_limits(mut self, limits: PreserveLimits) -> RoundtripChecker {
        if self.rec.is_some() {
            self.rec = Some(Collector::new(limits));
        }
        self
    }

    pub fn feed(&mut self, chunk: &[u8], last: bool) {
        if !self.enabled {
            return;
        }
        if let Some(iso) = self.iso.as_mut() {
            iso.dec.decode(chunk, last, &mut iso.sink);
            iso.sink.clear();
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
            // 長い行は、小分けで行の全部を調べ終えていても閉じる
            if !self.line.is_empty() || self.long.is_some() {
                self.finish_line();
            }
            self.flush_block();
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        self.consumed += bytes.len() as u64;
        if self.summary.incomplete {
            return;
        }
        if self.enc.is_stateful() {
            // ISO-2022-JP: 長すぎる行は検査を打ち切る (記録も全部捨てる、03 §8.7.1)
            if self.line.len() + bytes.len() > MAX_LINE {
                self.summary.incomplete = true;
                self.line = Vec::new();
                if let Some(rec) = self.rec.as_mut() {
                    rec.abandon(PreserveState::Incomplete);
                }
                return;
            }
            self.line.extend_from_slice(bytes);
            return;
        }
        self.line.extend_from_slice(bytes);
        // Shift_JIS / EUC-JP: 長い行は文字の境界で小分けにして調べ、ためるのは残りだけにする
        while self.line.len() >= self.piece_limit {
            let Some(cut) = preserve::piece_cut(self.enc, &self.line) else {
                break;
            };
            let mut line = std::mem::take(&mut self.line);
            self.check_piece(&line[..cut]);
            line.drain(..cut);
            self.line = line;
        }
    }

    fn finish_line(&mut self) {
        let line = std::mem::take(&mut self.line);
        let start = self.line_offset;
        self.line_offset = self.consumed;
        if self.summary.incomplete {
            return;
        }
        if self.long.is_some() {
            // 小分けにした長い行の最後の一部
            if !line.is_empty() {
                self.check_piece(&line);
            }
            let long = self.long.take().unwrap_or_default();
            self.count(long.differs, long.unmappable, start);
            self.line_no += 1;
            return;
        }
        if line.is_empty() {
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
            let mut body_line = true;
            if self.enc == Encoding::Iso2022Jp {
                body_line = self.iso_line(line, start);
            } else if may_differ(self.enc, line) {
                let c = self.check_line(line, start);
                if c.differs || c.unmappable {
                    self.collect(line, &c.text, self.line_no, 0);
                }
            }
            start += line.len() as u64;
            if body_line {
                self.line_no += 1;
            }
        }
    }

    /// 初期状態から復号する (不正バイトは PUA にエスケープする。本文の復号と同じ)。
    fn decode(&self, bytes: &[u8]) -> String {
        let mut d = Decoder::new(self.enc, false);
        let mut text = String::new();
        d.decode(bytes, true, &mut text);
        text
    }

    /// 復号して、保存と同じ方式で再符号化する。
    fn roundtrip(&self, bytes: &[u8]) -> LineCheck {
        let text = self.decode(bytes);
        let mut e = Encoder::new(self.enc, false, EscapePolicy::Restore);
        let (mut out, mut rep) = (Vec::with_capacity(bytes.len()), EncodeReport::default());
        e.encode(&text, &mut out, &mut rep);
        e.finish(&mut out, &mut rep);
        LineCheck {
            differs: out != bytes,
            unmappable: !rep.is_empty(),
            text,
        }
    }

    /// 1 行の検査の結果を数える。
    fn count(&mut self, differs: bool, unmappable: bool, start: u64) {
        if unmappable {
            self.summary.unmappable_lines += 1;
        } else if differs {
            self.summary.lossy_lines += 1;
            if self.summary.first_offsets.len() < MAX_POSITIONS {
                self.summary.first_offsets.push(start);
            }
        }
    }

    fn check_line(&mut self, line: &[u8], start: u64) -> LineCheck {
        let c = self.roundtrip(line);
        self.count(c.differs, c.unmappable, start);
        c
    }

    /// 記録を集めているか (上限を超えた・打ち切った後は偽)。
    fn collecting(&self) -> bool {
        self.rec.as_ref().is_some_and(Collector::active)
    }

    /// `Shift_JIS` / EUC-JP: 差のある行 (または長い行の一部) を文字ごとに突き合わせて記録にする。
    fn collect(&mut self, bytes: &[u8], text: &str, line_no: u64, col_base: u64) {
        let enc = self.enc;
        let Some(rec) = self.rec.as_mut().filter(|r| r.active()) else {
            return;
        };
        let ok = preserve::collect_chars(rec, enc, bytes, text, line_no, col_base);
        debug_assert!(ok, "文字ごとの突き合わせが食い違った: {enc:?} {bytes:02X?}");
        if !ok {
            // 起きないはず。安全側に倒して記録を全部捨てる (「保持なし」になり、保存の前に確認が出る)
            rec.abandon(PreserveState::Incomplete);
        }
    }

    /// 長い行 (`Shift_JIS` / EUC-JP) の一部を調べる。`piece` は文字の境界で始まり、文字の境界で終わる。
    fn check_piece(&mut self, piece: &[u8]) {
        if self.long.is_none() {
            // ためた前の行を先に調べる (行番号と記録の順を保つ)
            self.flush_block();
        }
        let mut long = self.long.take().unwrap_or_default();
        let line_no = self.line_no;
        if may_differ(self.enc, piece) {
            let c = self.roundtrip(piece);
            long.differs |= c.differs;
            long.unmappable |= c.unmappable;
            if c.differs || c.unmappable {
                self.collect(piece, &c.text, line_no, long.col);
            }
            long.col += c.text.len() as u64;
        } else if self.collecting() {
            // 差は出ないが、後ろの記録の位置のために、復号した長さを数える
            long.col += self.decode(piece).len() as u64;
        }
        self.long = Some(long);
    }

    /// ISO-2022-JP の 1 行。数え方 (`RoundtripSummary`) は v0.3 のまま (行ごとに ASCII 状態から復号する)。記録は、行の
    /// 始まりの実際の状態 (文書の先頭からエスケープシーケンスを追ったもの) が ASCII で、ASCII 状態で終わる、差のある行
    /// だけにする (03 §8.7.1 の「保持の単位」)。
    ///
    /// 偽を返したら、この行は本文では行にならない (行番号を進めない): 単独の CR で終わった行の次の、エスケープシーケンス
    /// だけ (復号すると空) で LF で終わる行は、本文では前の行の CR とこの行の LF が CRLF になる。この行は記録にできない
    /// (元のバイト列が本文の行の終端の中にある) ので、保持できなかった箇所に数える。
    fn iso_line(&mut self, line: &[u8], start: u64) -> bool {
        let chk = may_differ(self.enc, line).then(|| self.check_line(line, start));
        let content = line_content(line);
        let term = &line[content.len()..];
        let Some(prev_cr) = self.iso.as_ref().map(|i| i.prev_cr) else {
            return true;
        };
        let joins = prev_cr
            && term == b"\n"
            && content.starts_with(b"\x1B")
            && self.decode(content).is_empty();
        let Some(iso) = self.iso.as_mut() else {
            return true;
        };
        iso.prev_cr = term == b"\r";
        let begin = iso.state;
        let end = begin.after(content);
        iso.state = end;
        if !self.collecting() {
            return !joins;
        }
        if joins {
            if let Some(rec) = self.rec.as_mut() {
                rec.count_unpreserved();
            }
            return false;
        }
        let line_no = self.line_no;
        if begin == IsoState::Ascii {
            // ASCII だけでエスケープを含まない行 (`may_differ` が偽) は差が出ない
            let Some(c) = chk.filter(|c| c.differs || c.unmappable) else {
                return true;
            };
            let Some(rec) = self.rec.as_mut() else {
                return true;
            };
            let text = text_content(&c.text);
            if end != IsoState::Ascii {
                rec.count_unpreserved();
            } else if preserve::decodes_to(self.enc, content, text) {
                let reason = if preserve::has_halfwidth_kana(text) {
                    PreserveReason::DecodeOnly
                } else {
                    PreserveReason::StatefulEscape
                };
                rec.push(line_no, 0, text.len(), content, reason);
            } else {
                // 起きないはず (不正バイトを含む文書は、最後に記録を全部捨てる)
                rec.count_unpreserved();
            }
            return true;
        }
        // ASCII 以外 (ローマ字状態など) で始まる行: 記録にしない。実際の状態から復号して、再符号化で差が出れば、保持
        // できなかった箇所に数える (エスケープシーケンスで始まる行は、始まりの状態に依らない)
        let mut src = Vec::with_capacity(line.len() + 3);
        if !content.starts_with(b"\x1B") {
            src.extend_from_slice(begin.escape());
        }
        src.extend_from_slice(line);
        let text = self.decode(&src);
        let mut e = Encoder::new(self.enc, false, EscapePolicy::Restore);
        let (mut out, mut rep) = (Vec::with_capacity(line.len()), EncodeReport::default());
        e.encode(&text, &mut out, &mut rep);
        e.finish(&mut out, &mut rep);
        if (!rep.is_empty() || out != line)
            && let Some(rec) = self.rec.as_mut()
        {
            rec.count_unpreserved();
        }
        true
    }

    /// 検査の結果 (最後のチャンクを渡した後)。
    pub fn finish(self) -> RoundtripSummary {
        self.summary
    }

    /// 検査の結果と、集めた記録 (最後のチャンクを渡した後)。`with_records` で作らなかった検査器と、区分 A の文字コードでは
    /// 記録は `PreserveState::Disabled`。
    pub fn finish_with_records(self) -> (RoundtripSummary, PreservedRecords) {
        let records = match self.rec {
            None => PreservedRecords::default(),
            // 不正バイトを含む ISO-2022-JP の文書は、行の対応がずれ得るうえ、ISO-2022-JP では保存しない (03 §8.3)
            Some(_) if self.iso.as_ref().is_some_and(|i| i.dec.invalid_bytes() > 0) => {
                PreservedRecords::default()
            }
            Some(mut rec) => {
                if self.summary.incomplete {
                    rec.abandon(PreserveState::Incomplete);
                }
                rec.finish()
            }
        };
        (self.summary, records)
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
                    if preserve::may_differ_lead(enc, b) {
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
                    _ if preserve::may_differ_lead(enc, b) => return true,
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

/// バイト列全体を検査し、記録も集める (テスト・小さな入力用)。
pub fn check_with_records(enc: Encoding, bytes: &[u8]) -> (RoundtripSummary, PreservedRecords) {
    let mut c = RoundtripChecker::with_records(enc);
    c.feed(bytes, true);
    c.finish_with_records()
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
        let (_, r) = check_with_records(Encoding::Utf8, b"abc\n\xFF");
        assert_eq!(r, PreservedRecords::default());
        assert_eq!(r.state, PreserveState::Disabled);
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

    // 取りこぼしの判定を使った検査は、全行を正確に検査した結果と同じ (差のある行の数と位置、表現不能の行の数)。
    // 長い行を小分けにして調べても同じ
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
                fast.piece_limit = 1 + (next() % 30) as usize;
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
    fn iso_2022_jp_の長すぎる行は打ち切る() {
        let mut c = RoundtripChecker::with_records(Encoding::Iso2022Jp);
        let chunk = vec![b'a'; 1024 * 1024];
        for _ in 0..5 {
            c.feed(&chunk, false);
        }
        // 打ち切った後ろの、エスケープの選び方が違う行は数えない
        c.feed(b"\n\x1B$@$\"\x1B(B\n", true);
        let (summary, records) = c.finish_with_records();
        assert!(summary.incomplete);
        assert_eq!(summary.lossy_lines, 0);
        assert!(
            summary.may_be_lossy(),
            "検査していない範囲があるので、確認が要る"
        );
        assert!(!RoundtripSummary::default().may_be_lossy());
        assert_eq!(records.state, PreserveState::Incomplete);
        assert!(records.records.is_empty() && records.arena.is_empty());
    }

    #[test]
    fn shift_jis_の長い行は打ち切らずに調べる() {
        let mut c = RoundtripChecker::with_records(Encoding::ShiftJis);
        let chunk = vec![b'a'; 1024 * 1024];
        for _ in 0..5 {
            c.feed(&chunk, false);
        }
        // 長い行の末尾と、次の行に重複符号化 (IBM 拡張の ¬ 0xFA54)
        c.feed(b"\xFA\x54\n\xFA\x54\n", true);
        let (summary, records) = c.finish_with_records();
        assert!(!summary.incomplete);
        assert_eq!(summary.lossy_lines, 2);
        assert_eq!(summary.first_offsets, vec![0, 5 * 1024 * 1024 + 3]);
        assert_eq!(records.state, PreserveState::Collected);
        let got: Vec<(u64, u64, u32)> = records
            .records
            .iter()
            .map(|r| (r.line, r.col, r.len))
            .collect();
        assert_eq!(got, vec![(0, 5 * 1024 * 1024, 3), (1, 0, 3)]);
    }
}
