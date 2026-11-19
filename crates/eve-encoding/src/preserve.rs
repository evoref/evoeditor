//! 保持方式の記録 (docs/03-core-design.md §8.7.1、v0.4 の M22)。
//!
//! 読込時の往復の検査 (`RoundtripChecker`) が、**復号して再び符号化すると元とは別のバイト列になる箇所** の元のバイト列を
//! 集める。ここには、記録の型と、記録をためる入れ物 (上限とメモリの確保の失敗の扱い)、`Shift_JIS` / EUC-JP の文字ごとの
//! 突き合わせ、長い行を小分けにする位置の求め方、ISO-2022-JP の状態の追い方を置く。集めた記録を文書に持たせるのと、
//! 保存で使うのは eve-core (M22 の 5・6。保存の符号化器の口は `Encoder::write_preserved`。確認 (i) を済ませた記録は
//! `PreservedBytes` にして `Encoder::write_checked` へ)。
//!
//! - **不変条件** (どの記録も満たす): 記録の元のバイト列を、その文字コードの初期状態 (ISO-2022-JP は ASCII 状態) から
//!   復号すると、本文の記録の範囲の文字列になり、初期状態で終わる。
//! - **位置**: 行番号 (`line`) と、行頭からの位置 (`col`) と長さ (`len`) は、同じバイト列を `Decoder` で通しで復号した
//!   本文 (UTF-8) の座標。行の区切りは本文と同じ LF・CR・CRLF。不正バイトの PUA エスケープ (1 バイト → 1 文字、UTF-8 で
//!   4 バイト) の分も数える。

use encoding_rs::EncoderResult;

use crate::encode::cp932;
use crate::encoding::Encoding;
use crate::utf8::unescape_char;

/// 記録の件数の上限 (これを超えたら全部捨てる、03 §8.7.1 の「上限」)。
pub const MAX_PRESERVED_RECORDS: usize = 1_000_000;
/// 記録の元のバイト列の合計の上限 (バイト)。
pub const MAX_PRESERVED_BYTES: usize = 16 * 1024 * 1024;

/// 元のバイト列を保持する理由 (03 §8.7.1)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PreserveReason {
    /// 再符号化すると、同じ文字の別のバイト列 (代表バイト列) になる (CP932 の IBM 拡張の重複、EUC-JP の 0x8FA2B7 など)。
    DuplicateEncoding,
    /// 符号化器が出力できない (EUC-JP の JIS X 0212、ISO-2022-JP の半角カナを含む行)。
    DecodeOnly,
    /// ISO-2022-JP のエスケープシーケンスの選び方の差 (区分 D)。
    StatefulEscape,
}

/// 読込時に集めた 1 件の記録。位置は、復号した本文の座標で持つ。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreservedRecord {
    /// 本文の行番号 (0 始まり。行の区切りは LF・CR・CRLF)。
    pub line: u64,
    /// 行頭からの位置 (復号した本文の UTF-8 のバイト数)。ISO-2022-JP の記録は常に 0 (行の内容の全体)。
    pub col: u64,
    /// 範囲の長さ (復号した本文の UTF-8 のバイト数)。行の終端を含まない。`Shift_JIS` / EUC-JP では 1 以上。ISO-2022-JP では
    /// 0 のことがある (冗長なエスケープシーケンスだけの行。元のバイト列は空でない)。
    pub len: u32,
    /// `PreservedRecords::arena` の中の、元のバイト列。
    pub bytes: std::ops::Range<u32>,
    pub reason: PreserveReason,
}

/// 記録の状態。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PreserveState {
    /// 集め終えた (0 件のこともある)。
    Collected,
    /// 上限を超えた、または記録を集めるメモリを確保できなかったので、全部捨てた。
    Overflow,
    /// 検査を打ち切った (ISO-2022-JP の 4 MiB を超える行) ので、全部捨てた。検査器の内部の食い違い (起きないはず) も
    /// 安全側に倒してここに入れる。
    Incomplete,
    /// 集めない: 区分 A の文字コード (UTF-8 / UTF-16)、不正バイトを含む ISO-2022-JP の文書、記録を求めずに作った検査器。
    #[default]
    Disabled,
}

/// 集めた記録の全部と、状態。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PreservedRecords {
    /// 行番号・行内の位置の昇順。重ならない。
    pub records: Vec<PreservedRecord>,
    /// 元のバイト列をつないだもの。
    pub arena: Vec<u8>,
    pub state: PreserveState,
    /// 保持できなかった箇所の数 (ISO-2022-JP で、ASCII 状態で始まらない・終わらない、差のある行)。`Collected` のときだけ数える。
    pub unpreserved: u32,
}

impl PreservedRecords {
    /// 記録の元のバイト列。
    #[must_use]
    pub fn bytes_of(&self, r: &PreservedRecord) -> &[u8] {
        &self.arena[r.bytes.start as usize..r.bytes.end as usize]
    }
}

/// 記録の上限 (試験では小さい値に差し替える)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreserveLimits {
    /// 件数の上限。
    pub max_records: usize,
    /// 元のバイト列の合計の上限 (バイト)。
    pub max_bytes: usize,
}

impl Default for PreserveLimits {
    fn default() -> PreserveLimits {
        PreserveLimits {
            max_records: MAX_PRESERVED_RECORDS,
            max_bytes: MAX_PRESERVED_BYTES,
        }
    }
}

/// 記録をためる入れ物。上限を超えたら、またはメモリの確保に失敗したら、全部捨てて以後はためない。
pub(crate) struct Collector {
    records: Vec<PreservedRecord>,
    arena: Vec<u8>,
    state: PreserveState,
    unpreserved: u32,
    limits: PreserveLimits,
}

impl Collector {
    pub(crate) fn new(limits: PreserveLimits) -> Collector {
        Collector {
            records: Vec::new(),
            arena: Vec::new(),
            state: PreserveState::Collected,
            unpreserved: 0,
            limits,
        }
    }

    /// まだためているか (上限を超えた・打ち切った後は偽)。
    pub(crate) fn active(&self) -> bool {
        self.state == PreserveState::Collected
    }

    /// 全部捨てて、以後はためない。
    pub(crate) fn abandon(&mut self, state: PreserveState) {
        if self.state == PreserveState::Collected {
            self.records = Vec::new();
            self.arena = Vec::new();
            self.unpreserved = 0;
            self.state = state;
        }
    }

    /// 保持できなかった箇所を 1 つ数える。
    pub(crate) fn count_unpreserved(&mut self) {
        if self.active() {
            self.unpreserved = self.unpreserved.saturating_add(1);
        }
    }

    /// 1 件を足す。直前の記録と同じ行で、本文の範囲が隣り合うなら 1 件にまとめる (元のバイト列も隣り合う。
    /// 呼び出し側は、元のバイト列が直前の記録に続くときだけ隣り合う範囲を渡す)。
    pub(crate) fn push(
        &mut self,
        line: u64,
        col: u64,
        len: usize,
        bytes: &[u8],
        reason: PreserveReason,
    ) {
        if !self.active() {
            return;
        }
        let merge = self.records.last().is_some_and(|last| {
            last.line == line
                && last.col + u64::from(last.len) == col
                && last.bytes.end as usize == self.arena.len()
                && reason != PreserveReason::StatefulEscape
                && last.reason != PreserveReason::StatefulEscape
        });
        let new_count = self.records.len() + usize::from(!merge);
        let new_bytes = self.arena.len().saturating_add(bytes.len());
        if new_count > self.limits.max_records || new_bytes > self.limits.max_bytes {
            self.abandon(PreserveState::Overflow);
            return;
        }
        // 範囲は u32 に収める (上限の 16 MiB より大きい値は、上の検査で弾かれる。上限を差し替えた場合も守る)
        let (Ok(end), Ok(len32)) = (u32::try_from(new_bytes), u32::try_from(len)) else {
            self.abandon(PreserveState::Overflow);
            return;
        };
        if self.arena.try_reserve(bytes.len()).is_err() {
            self.abandon(PreserveState::Overflow);
            return;
        }
        if merge {
            self.arena.extend_from_slice(bytes);
            if let Some(last) = self.records.last_mut() {
                last.len = last.len.saturating_add(len32);
                last.bytes.end = end;
                if reason == PreserveReason::DecodeOnly {
                    last.reason = PreserveReason::DecodeOnly;
                }
            }
            return;
        }
        if self.records.try_reserve(1).is_err() {
            self.abandon(PreserveState::Overflow);
            return;
        }
        let start = u32::try_from(self.arena.len()).unwrap_or(u32::MAX);
        self.arena.extend_from_slice(bytes);
        self.records.push(PreservedRecord {
            line,
            col,
            len: len32,
            bytes: start..end,
            reason,
        });
    }

    pub(crate) fn finish(self) -> PreservedRecords {
        PreservedRecords {
            records: self.records,
            arena: self.arena,
            state: self.state,
            unpreserved: self.unpreserved,
        }
    }
}

/// `Shift_JIS` / EUC-JP: 文字の元のバイト数 (`b0` は文字の先頭のバイト。不正バイトのエスケープ文字には使わない)。
/// 呼び出し側は、エスケープ文字でない文字だけを渡す (その文字は妥当なバイト列から復号されている)。
fn char_len(enc: Encoding, b0: u8) -> usize {
    match enc {
        // ASCII・0x80 (U+0080)・半角カナ 0xA1〜0xDF は 1 バイト、ほかは 2 バイト
        Encoding::ShiftJis => {
            if b0 <= 0x80 || (0xA1..=0xDF).contains(&b0) {
                1
            } else {
                2
            }
        }
        // ASCII は 1 バイト、補助漢字 (0x8F) は 3 バイト、ほか (半角カナ 0x8E・JIS X 0208) は 2 バイト
        _ => match b0 {
            0x00..=0x7F => 1,
            0x8F => 3,
            _ => 2,
        },
    }
}

/// 再符号化で別のバイト列になり得る文字の先頭バイトか (`Shift_JIS` / EUC-JP。偽なら、その文字は必ず元のバイト列に戻る)。
/// 読込時の検査の `may_differ` と同じ判定で、取りこぼしが無いことは全ての 2 バイト (と EUC-JP の 0x8F の 3 バイト) の組で
/// 確かめている (`roundtrip` の試験)。
pub(crate) fn may_differ_lead(enc: Encoding, b0: u8) -> bool {
    match enc {
        // NEC 特殊文字 0x87、NEC 選定 IBM 拡張 0xED / 0xEE、IBM 拡張 0xFA
        Encoding::ShiftJis => matches!(b0, 0x87 | 0xED | 0xEE | 0xFA),
        // 補助漢字 0x8F、NEC 特殊文字 0xAD、IBM 拡張 0xFC
        Encoding::EucJp => matches!(b0, 0x8F | 0xAD | 0xFC),
        _ => true,
    }
}

/// 1 文字を再符号化した結果 (元のバイト列に依らない部分)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reencoded {
    /// 符号化器の出力 (`out[..len]`) と、それを復号すると同じ文字に戻るか。
    Bytes { out: [u8; 16], len: u8, back: bool },
    /// 出力できない (表現不能)。
    Unmappable,
}

/// 再符号化の結果の覚えの大きさ (直接写像の項目の数。2 のべき)。
const MEMO_SLOTS: usize = 256;

/// 1 文字の再符号化の結果の覚え (文字の値で引く直接写像。`Shift_JIS` / EUC-JP の符号化器は状態を持たないので、結果は
/// 文字だけで決まる)。区分 B の箇所が密な文書では同じ文字が何度も現れるので、1 文字ごとの再符号化と復号を繰り返さない
/// (読込の検査の記録を集める費用の大半だった)。
pub(crate) struct ReencodeMemo {
    /// 覚えている結果の文字コード。
    enc: Option<Encoding>,
    slots: Vec<Option<(char, Reencoded)>>,
}

impl ReencodeMemo {
    /// 空の覚え (項目は最初に使うときに確保する)。
    pub(crate) fn new() -> ReencodeMemo {
        ReencodeMemo {
            enc: None,
            slots: Vec::new(),
        }
    }

    /// `c` の再符号化の結果 (覚えていなければ求めて覚える)。違う文字コードで引いたら、覚えを全部捨てる。
    fn get(&mut self, enc: Encoding, e: &mut encoding_rs::Encoder, c: char) -> Reencoded {
        if self.enc != Some(enc) || self.slots.is_empty() {
            self.enc = Some(enc);
            self.slots = vec![None; MEMO_SLOTS];
        }
        let i = c as usize & (MEMO_SLOTS - 1);
        if let Some((k, r)) = self.slots[i]
            && k == c
        {
            return r;
        }
        let r = reencode(enc, e, c);
        self.slots[i] = Some((c, r));
        r
    }
}

/// 1 文字を再符号化する (`Shift_JIS` / EUC-JP)。
fn reencode(enc: Encoding, e: &mut encoding_rs::Encoder, c: char) -> Reencoded {
    let mut tmp = [0u8; 4];
    let s = c.encode_utf8(&mut tmp);
    let mut out = [0u8; 16];
    // 状態を持たない符号化器なので、`last = false` のまま 1 文字ずつ使い回せる (持ち越しが残らない)
    let (res, _, written) = e.encode_from_utf8_without_replacement(s, &mut out, false);
    match res {
        EncoderResult::InputEmpty => Reencoded::Bytes {
            out,
            // 作業領域 (16 バイト) に収まった長さ
            len: u8::try_from(written).unwrap_or(16),
            back: decodes_to(enc, &out[..written], s),
        },
        EncoderResult::Unmappable(_) | EncoderResult::OutputFull => Reencoded::Unmappable,
    }
}

/// 1 文字を再符号化して元のバイト列と比べ、違えば理由を返す。
fn char_diff(
    enc: Encoding,
    e: &mut encoding_rs::Encoder,
    memo: &mut ReencodeMemo,
    c: char,
    orig: &[u8],
) -> Option<PreserveReason> {
    // CP932 の外字は自前の逆算で書く (03 §8.7 の「CP932 の外字」。符号化器 `Encoder` と同じ)
    if enc == Encoding::ShiftJis
        && let Some(g) = cp932::gaiji_to_sjis(c)
    {
        return (g[..] != *orig).then_some(PreserveReason::DuplicateEncoding);
    }
    match memo.get(enc, e, c) {
        Reencoded::Bytes { out, len, back } => {
            if out[..usize::from(len)] == *orig {
                None
            } else if back {
                Some(PreserveReason::DuplicateEncoding)
            } else {
                // 別の文字に黙って写される (照合で一致しない): 出力できないのと同じ
                Some(PreserveReason::DecodeOnly)
            }
        }
        Reencoded::Unmappable => Some(PreserveReason::DecodeOnly),
    }
}

/// `bytes` を初期状態から復号すると、ちょうど `want` になるか (不正バイトがあれば偽)。
///
/// 復号の結果を文字列に作らず、手元の小さな作業領域に少しずつ復号して `want` の先頭から突き合わせる (記録ごと・区間ごとに
/// 呼ぶので、1 回ごとにヒープを確保しない。区分 B の箇所が密なファイルの読込・保存で効く)。結果は
/// `decode_without_bom_handling_and_without_replacement(bytes) == Some(want)` と同じ。
pub(crate) fn decodes_to(enc: Encoding, bytes: &[u8], want: &str) -> bool {
    let mut d = enc.rs().new_decoder_without_bom_handling();
    // 1 回の呼び出しで出力できる文字 (UTF-8 で 4 バイトまで) より十分に大きい
    let mut buf = [0u8; 512];
    let (mut src, mut rest) = (bytes, want.as_bytes());
    loop {
        let (res, read, written) = d.decode_to_utf8_without_replacement(src, &mut buf, true);
        let Some(tail) = rest.strip_prefix(&buf[..written]) else {
            return false;
        };
        rest = tail;
        src = &src[read..];
        match res {
            encoding_rs::DecoderResult::InputEmpty => return rest.is_empty(),
            // 進まないことは無い (作業領域は 1 文字より大きい) が、念のため止める
            encoding_rs::DecoderResult::OutputFull if read > 0 || written > 0 => {}
            encoding_rs::DecoderResult::OutputFull | encoding_rs::DecoderResult::Malformed(..) => {
                return false;
            }
        }
    }
}

/// `Shift_JIS` / EUC-JP の 1 行 (または長い行の一部。文字の境界で始まり、文字の境界で終わる) を、復号した文字列 `text` と
/// 文字ごとに突き合わせ、再符号化で違いが出る連続した文字を記録にする。`col_base` は `bytes` の先頭の、行頭からの位置
/// (本文の UTF-8 のバイト数)。不正バイトのエスケープ文字は記録に含めない (区切りになる)。
///
/// 突き合わせが食い違ったら (起きないはず) 偽を返す (呼び出し側は記録を全部捨てる)。`memo` は 1 文字の再符号化の結果の
/// 覚え (検査器が文書の全体で使い回す)。
pub(crate) fn collect_chars(
    rec: &mut Collector,
    memo: &mut ReencodeMemo,
    enc: Encoding,
    bytes: &[u8],
    text: &str,
    line: u64,
    col_base: u64,
) -> bool {
    let mut e = enc.rs().new_encoder();
    let mut p = 0usize;
    let mut col = 0usize;
    // 続いている違いの (元のバイト列の先頭, 本文の先頭, 理由)
    let mut run: Option<(usize, usize, PreserveReason)> = None;
    // 違いの続きを閉じて記録にする。不変条件 (元のバイト列を初期状態から復号すると、範囲の文字列になる) を確かめる
    let close = |run: &mut Option<(usize, usize, PreserveReason)>,
                 rec: &mut Collector,
                 p: usize,
                 col: usize| {
        let Some((bp, cp, reason)) = run.take() else {
            return true;
        };
        let (Some(orig), Some(s)) = (bytes.get(bp..p), text.get(cp..col)) else {
            return false;
        };
        if !decodes_to(enc, orig, s) {
            return false;
        }
        rec.push(line, col_base + cp as u64, col - cp, orig, reason);
        true
    };
    let tb = text.as_bytes();
    while col < tb.len() {
        // ASCII の並びは同じバイトに戻る: 元のバイト列とまとめて比べて飛ばす (食い違うなら、突き合わせがずれている)。
        // 違いの続きは、並びの最初の文字で閉じる
        let k = tb[col..]
            .iter()
            .position(|b| !b.is_ascii())
            .unwrap_or(tb.len() - col);
        if k > 0 {
            if bytes.get(p..p + k) != Some(&tb[col..col + k]) {
                return false;
            }
            if !close(&mut run, rec, p, col) {
                return false;
            }
            p += k;
            col += k;
            continue;
        }
        let Some(c) = text[col..].chars().next() else {
            break;
        };
        let (blen, diff) = if unescape_char(c).is_some() {
            // 不正バイトは 1 バイトを 1 文字にエスケープしている (保存で元のバイトに戻すので、記録にしない)
            (1, None)
        } else {
            let Some(&b0) = bytes.get(p) else {
                return false;
            };
            let blen = char_len(enc, b0);
            let Some(orig) = bytes.get(p..p + blen) else {
                return false;
            };
            let diff = if may_differ_lead(enc, b0) {
                char_diff(enc, &mut e, memo, c, orig)
            } else {
                None
            };
            (blen, diff)
        };
        match (diff, &mut run) {
            (Some(reason), Some((_, _, r))) => {
                if reason == PreserveReason::DecodeOnly {
                    *r = PreserveReason::DecodeOnly;
                }
            }
            (Some(reason), None) => run = Some((p, col, reason)),
            (None, _) => {
                if !close(&mut run, rec, p, col) {
                    return false;
                }
            }
        }
        p += blen;
        col += c.len_utf8();
    }
    close(&mut run, rec, p, col) && p == bytes.len()
}

/// 長い行 (`Shift_JIS` / EUC-JP) を小分けにする位置を返す。`bytes` は文字の境界から始まる。返す位置 `n` (1 以上) で
/// 区切ると、`bytes[..n]` と残りを別々の復号器で復号しても、通しで復号したのと同じ結果になる。見つからなければ `None`。
///
/// 2 バイト目になり得ない ASCII のバイト (`Shift_JIS` は 0x00〜0x3F と 0x7F、EUC-JP は 0x00〜0x7F) の直後は、必ず文字の
/// 境界 (直前が先頭バイトでも、その先頭バイトは不正バイトになり、ASCII のバイトは読み直される。WHATWG の復号器の規則)。
/// そのようなバイトが無い行 (空白も改行も無い日本語だけの行など) は、先頭から文字の境界を追う。
pub(crate) fn piece_cut(enc: Encoding, bytes: &[u8]) -> Option<usize> {
    let safe = |b: u8| match enc {
        Encoding::ShiftJis => b < 0x40 || b == 0x7F,
        _ => b < 0x80,
    };
    if let Some(i) = bytes.iter().rposition(|&b| safe(b)) {
        return Some(i + 1);
    }
    let mut i = 0;
    let mut last = None;
    while i < bytes.len() {
        let Some(w) = unit_len(enc, &bytes[i..]) else {
            break;
        };
        i += w;
        last = Some(i);
    }
    last
}

/// 先頭から 1 単位 (1 文字、または不正バイトとして消費される並び) のバイト数。決めるのに後ろのバイトが足りなければ `None`。
/// WHATWG の `Shift_JIS` / EUC-JP の復号器の規則: 先頭バイトの後ろが ASCII のバイトでなければ、対応が無くても組で消費する。
/// ASCII のバイトなら、対応が無ければ読み直す (先頭バイトだけが不正バイトになる)。
fn unit_len(enc: Encoding, b: &[u8]) -> Option<usize> {
    let b0 = b[0];
    match enc {
        Encoding::ShiftJis => {
            if !matches!(b0, 0x81..=0x9F | 0xE0..=0xFC) {
                return Some(1);
            }
            let t = *b.get(1)?;
            if t >= 0x80 {
                Some(2)
            } else if (0x40..=0x7E).contains(&t) {
                // 対応のある組 (外字を含む) だけが 2 バイト
                let ok = encoding_rs::SHIFT_JIS
                    .decode_without_bom_handling_and_without_replacement(&b[..2])
                    .is_some();
                Some(if ok { 2 } else { 1 })
            } else {
                Some(1)
            }
        }
        _ => match b0 {
            0x8E | 0xA1..=0xFE => {
                let t = *b.get(1)?;
                Some(if t < 0x80 { 1 } else { 2 })
            }
            0x8F => {
                let t = *b.get(1)?;
                if t < 0x80 {
                    Some(1)
                } else if (0xA1..=0xFE).contains(&t) {
                    let u = *b.get(2)?;
                    Some(if u < 0x80 { 2 } else { 3 })
                } else {
                    Some(2)
                }
            }
            _ => Some(1),
        },
    }
}

/// ISO-2022-JP の復号器の状態 (エスケープシーケンスで切り替わる)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IsoState {
    Ascii,
    Roman,
    Katakana,
    Jis0208,
}

impl IsoState {
    /// この状態に入るエスケープシーケンス。
    pub(crate) fn escape(self) -> &'static [u8] {
        match self {
            IsoState::Ascii => b"\x1B(B",
            IsoState::Roman => b"\x1B(J",
            IsoState::Katakana => b"\x1B(I",
            IsoState::Jis0208 => b"\x1B$B",
        }
    }

    /// `bytes` を復号し終えた後の状態 (`self` から始める)。受け付けるエスケープシーケンスだけを追う (ほかは不正バイトに
    /// なり、その文書は記録を集めないので、状態の正しさは問わない)。
    pub(crate) fn after(self, bytes: &[u8]) -> IsoState {
        let mut s = self;
        let mut i = 0;
        while let Some(off) = bytes[i..].iter().position(|&b| b == 0x1B) {
            let at = i + off;
            s = match bytes.get(at + 1..at + 3) {
                Some(b"(B") => IsoState::Ascii,
                Some(b"(J") => IsoState::Roman,
                Some(b"(I") => IsoState::Katakana,
                Some(b"$@" | b"$B") => IsoState::Jis0208,
                _ => s,
            };
            i = at + 1;
        }
        s
    }
}

/// 半角カナ (U+FF61〜U+FF9F) を含むか (ISO-2022-JP の符号化器は出力できない)。
pub(crate) fn has_halfwidth_kana(s: &str) -> bool {
    s.chars().any(|c| ('\u{FF61}'..='\u{FF9F}').contains(&c))
}

#[cfg(test)]
mod props;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 上限を超えたら全部捨てる() {
        let mut c = Collector::new(PreserveLimits {
            max_records: 2,
            max_bytes: 100,
        });
        c.push(0, 0, 3, b"\xFA\x54", PreserveReason::DuplicateEncoding);
        c.push(1, 0, 3, b"\xFA\x54", PreserveReason::DuplicateEncoding);
        assert!(c.active());
        c.push(2, 0, 3, b"\xFA\x54", PreserveReason::DuplicateEncoding);
        let r = c.finish();
        assert_eq!(r.state, PreserveState::Overflow);
        assert!(r.records.is_empty() && r.arena.is_empty());

        let mut c = Collector::new(PreserveLimits {
            max_records: 100,
            max_bytes: 4,
        });
        c.push(0, 0, 3, b"\xFA\x54", PreserveReason::DuplicateEncoding);
        c.push(0, 6, 3, b"\xFA\x54", PreserveReason::DuplicateEncoding);
        assert!(c.active(), "ちょうど上限は超えていない");
        c.push(1, 0, 3, b"\xFA", PreserveReason::DuplicateEncoding);
        c.count_unpreserved();
        let r = c.finish();
        assert_eq!(r.state, PreserveState::Overflow);
        assert_eq!(r.unpreserved, 0);
    }

    #[test]
    fn 隣り合う範囲は_1_件にまとめる() {
        let mut c = Collector::new(PreserveLimits::default());
        c.push(0, 3, 3, b"\xFA\x54", PreserveReason::DuplicateEncoding);
        c.push(0, 6, 3, b"\x8F\xB0\xA1", PreserveReason::DecodeOnly);
        c.push(0, 12, 3, b"\xFA\x55", PreserveReason::DuplicateEncoding);
        let r = c.finish();
        assert_eq!(r.records.len(), 2);
        assert_eq!(
            (r.records[0].col, r.records[0].len, r.records[0].reason),
            (3, 6, PreserveReason::DecodeOnly)
        );
        assert_eq!(r.bytes_of(&r.records[0]), b"\xFA\x54\x8F\xB0\xA1");
        assert_eq!(r.bytes_of(&r.records[1]), b"\xFA\x55");
    }

    #[test]
    fn iso_2022_jp_の状態をエスケープで追う() {
        assert_eq!(IsoState::Ascii.after(b"abc"), IsoState::Ascii);
        assert_eq!(IsoState::Roman.after(b"abc"), IsoState::Roman);
        assert_eq!(IsoState::Ascii.after(b"\x1B$@$\"\x1B(J\\"), IsoState::Roman);
        assert_eq!(IsoState::Roman.after(b"\x1B(B"), IsoState::Ascii);
        assert_eq!(IsoState::Ascii.after(b"\x1B(I1"), IsoState::Katakana);
        assert_eq!(IsoState::Ascii.after(b"\x1B$B$\""), IsoState::Jis0208);
        // 途中で切れたエスケープは状態を変えない
        assert_eq!(IsoState::Ascii.after(b"\x1B$"), IsoState::Ascii);
    }

    // 再符号化の結果の覚えは文字だけで引く: 同じ文字でも元のバイト列が違えば判定が変わり、覚えの項目を取り合う文字
    // (値の下位 8 ビットが同じ) が交互に来ても、覚えなしと同じ結果になる
    #[test]
    fn 再符号化の覚えは元のバイト列ごとに判定する() {
        let enc = Encoding::ShiftJis;
        let mut e = enc.rs().new_encoder();
        let mut memo = ReencodeMemo::new();
        // 纊 (U+7E8A): NEC 選定 IBM 拡張 0xED40 は代表 (IBM 拡張 0xFA5C) と違う
        for _ in 0..3 {
            assert_eq!(
                char_diff(enc, &mut e, &mut memo, '纊', b"\xED\x40"),
                Some(PreserveReason::DuplicateEncoding)
            );
            assert_eq!(char_diff(enc, &mut e, &mut memo, '纊', b"\xFA\x5C"), None);
            // U+7F8A (羊。下位 8 ビットが纊と同じ) は JIS 第 1 水準で、元に戻る
            assert_eq!(char_diff(enc, &mut e, &mut memo, '羊', b"\x97\x72"), None);
        }
        // 覚えなし (毎回新しい覚え) と同じ
        for (c, orig) in [
            ('纊', &b"\xED\x40"[..]),
            ('羊', b"\x97\x72"),
            ('∵', b"\x87\x9A"),
        ] {
            let mut fresh = ReencodeMemo::new();
            assert_eq!(
                char_diff(enc, &mut e, &mut memo, c, orig),
                char_diff(enc, &mut e, &mut fresh, c, orig),
                "{c}"
            );
        }
        // EUC-JP の補助漢字は出力できない
        let enc = Encoding::EucJp;
        let mut e = enc.rs().new_encoder();
        assert_eq!(
            char_diff(enc, &mut e, &mut memo, '丂', b"\x8F\xB0\xA1"),
            Some(PreserveReason::DecodeOnly)
        );
    }

    // 文字列を作らない突き合わせは、encoding_rs の一括の復号と比べるのと同じ結果になる (作業領域より長い入力・
    // 不正バイト・途中で切れた入力・ISO-2022-JP の状態を含む)
    #[test]
    fn 作らない突き合わせは一括の復号と同じ() {
        let mut x = 0x2545_F491_4F6C_DD1Du64;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let parts: [&[u8]; 9] = [
            b"a",
            b"\x93\xFA",
            b"\xED\x40",
            b"\x8F\xB0\xA1",
            b"\x1B$B$\"\x1B(B",
            b"\x1B(I1",
            b"\xFD",
            b"\x81",
            b"\xC6\xFC",
        ];
        for enc in [Encoding::ShiftJis, Encoding::EucJp, Encoding::Iso2022Jp] {
            for round in 0..4000 {
                // 作業領域 (512 バイト) をまたぐ長さも作る
                let n = if round % 50 == 0 {
                    400
                } else {
                    1 + (next() % 8) as usize
                };
                let mut b = Vec::new();
                for _ in 0..n {
                    b.extend_from_slice(parts[(next() % 9) as usize]);
                }
                let whole = enc
                    .rs()
                    .decode_without_bom_handling_and_without_replacement(&b)
                    .map(std::borrow::Cow::into_owned);
                match &whole {
                    Some(s) => {
                        assert!(decodes_to(enc, &b, s), "{enc:?} {b:02X?}");
                        let mut longer = s.clone();
                        longer.push('x');
                        assert!(!decodes_to(enc, &b, &longer));
                        if let Some(c) = s.chars().next_back() {
                            assert!(!decodes_to(enc, &b, &s[..s.len() - c.len_utf8()]));
                        }
                    }
                    None => assert!(!decodes_to(enc, &b, ""), "{enc:?} {b:02X?}"),
                }
            }
        }
    }

    // 小分けにする位置は、どの入力でも文字の境界 (前後を別々に復号しても、通しの復号と同じ)
    #[test]
    fn 小分けにする位置は文字の境界() {
        let decode = |enc: Encoding, b: &[u8]| {
            let mut d = crate::Decoder::new(enc, false);
            let mut s = String::new();
            d.decode(b, true, &mut s);
            s
        };
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for enc in [Encoding::ShiftJis, Encoding::EucJp] {
            for round in 0..20_000 {
                let n = 1 + (next() % 12) as usize;
                let b: Vec<u8> = (0..n)
                    .map(|_| {
                        let v = next().to_le_bytes()[0];
                        // 半分は ASCII を含まない (先頭から境界を追う側を通す)
                        if round % 2 == 0 { 0x80 | v } else { v }
                    })
                    .collect();
                let Some(cut) = piece_cut(enc, &b) else {
                    continue;
                };
                assert!(cut >= 1 && cut <= b.len());
                let whole = decode(enc, &b);
                let split = decode(enc, &b[..cut]) + &decode(enc, &b[cut..]);
                assert_eq!(split, whole, "{enc:?} {b:02X?} cut {cut}");
            }
        }
    }
}
