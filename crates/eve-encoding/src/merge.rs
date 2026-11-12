//! 不正バイトのエスケープ文字のつながりの検出 (docs/03-core-design.md §8.3.1 の「つながりの検出」、v0.4 の M23)。
//!
//! エスケープ文字を元のバイトに戻して書く保存 (`EscapePolicy::Restore`) の出力を、符号化の単位 (本文の 1 文字、または
//! 保持方式の記録の元のバイト列の全体) ごとに受け取り、次を求める。
//!
//! - **局所の検査**: 連続するエスケープ文字の並びごとに、戻したバイト列と直後の 1 単位を、その文字コードの初期状態の
//!   復号器で復号し (末尾は `last = true`)、並びのエスケープ文字と直後の単位の文字に戻るかを比べる。戻らなければ 1 件。
//!   示す位置は、局所の復号で最初に違いが出たエスケープ文字 (違いが直後の単位で初めて出たときは並びの最後のエスケープ
//!   文字)。この検査は「出力の全体を復号すると本文に一致する」ことと同値である (03 §8.3.1)。書く文字コード自身の BOM の
//!   並びが出力の先頭にできる場合も、初期状態の復号器では U+FEFF になって本文と違うので、この検査で見つかる。
//! - **区切った確認 (ii) の区切り** (`MergeRegion`): つながる並びと直後の 1 単位の範囲 (出力のバイトと本文のバイト)。
//!   出力の全体の突き合わせ (確認 (ii)) はこの範囲だけを飛ばし、範囲の後で復号器を初期状態にし直して続ける。並びの
//!   終わりが決まるまでは終わりの無い区切りを置き、つながらないと分かったら外す (確認 (ii) は、その間の出力を持って待つ)。
//! - **全件のダイジェスト**: 全部のつながり (位置・強調の範囲・局所の復号) を順に、呼び出し側が渡すハッシュに入れる
//!   (承認の照らし合わせで、先頭 100 件より後ろの違いも見る)。
//! - **変わる範囲** (`track_range`): つながりの位置から、区切らない復号器 (開き直しと同じ 1 本) を単位ごとに進め、
//!   つながりの位置より後ろのある単位で、復号器の出したものがその単位の文字で終わり、復号器に持ち越しが無くなったら、
//!   その単位の前で範囲を閉じる。重なる範囲は 1 つにまとめ、本文の文字数と行数を合計する。文書の末尾まで閉じず、末尾で
//!   持ち越しが残れば `to_eof`。
//!
//! 呼び出し側 (eve-core の保存の符号化の流れ) は、`is_idle` が真の間はエスケープ文字の無い区間をまとめて符号化して `bulk` で
//! 知らせ、偽の間は単位ごとに符号化して `unit` で知らせる (費用は並びとつながりの周りだけに掛かる)。

use std::collections::VecDeque;
use std::hash::Hasher;

use crate::decode::Decoder;
use crate::encoding::Encoding;
use crate::utf8::unescape_char;

/// 記録するつながりの最大件数 (表現不能と同じ上限。総数は数える)。
pub const MAX_MERGES: usize = 100;
/// 1 件の局所の復号の文字列の最大の文字数。
pub const MERGE_DECODED_CHARS: usize = 16;

/// つながりの 1 件 (02 §9.3 の `EscapeMergeAt`)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EscapeMergeAt {
    /// 局所の復号で最初に違いが出たエスケープ文字の、本文の通し番号 (Unicode スカラー値。表現不能と同じ数え方)。
    /// 違いが直後の 1 単位で初めて出た (並びのエスケープ文字はどれも元に戻った) ときは、並びの最後のエスケープ文字。
    pub char_index: u64,
    /// そこから直後の 1 単位の終わりまでの文字数 (強調の範囲)。
    pub len_chars: u32,
    /// その範囲を局所に復号した文字列の先頭 16 文字 (無害化の前)。
    pub decoded: String,
}

/// つながりの検出の結果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MergeReport {
    /// つながりの件数。
    pub total: u64,
    /// 先頭 `MAX_MERGES` 件。
    pub recorded: Vec<EscapeMergeAt>,
    /// 全件 (位置・強調の範囲・局所の復号) のダイジェスト。
    pub digest: u64,
    /// 開き直すと変わる範囲の合計 (本文の文字数)。`track_range` のときだけ数える。
    pub changed_chars: u64,
    /// 開き直すと変わる範囲の合計 (本文の行数)。`track_range` のときだけ数える。
    pub changed_lines: u64,
    /// 変わる範囲が文書の末尾まで戻らない (末尾で復号器に持ち越しが残る)。
    pub to_eof: bool,
}

/// 区切った確認 (ii) で突き合わせない範囲 (つながる並びと直後の 1 単位)。位置は出力のバイト数と本文 (UTF-8) の
/// バイト数で、どちらも先頭からの通し。並びの終わりが決まるまでは `out_end` / `text_end` が `None` (つながるかどうかも
/// まだ分からない。つながらないと分かったら外す)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MergeRegion {
    pub out_start: u64,
    pub out_end: Option<u64>,
    pub text_start: usize,
    pub text_end: Option<usize>,
}

/// 局所の検査の途中の並び。
struct Run {
    char_start: u64,
    /// 並びのエスケープ文字の数。
    len: u64,
    last_escape: char,
    /// 局所の復号器に渡したが、まだ出てきていない文字 (期待する順)。
    expect: VecDeque<char>,
    /// 期待どおりに出てきた文字の数。
    matched: u64,
    /// 最初に違いが出た位置 (並びの先頭からの文字数)。
    mismatch: Option<u64>,
    /// 違いが出た位置から後に出てきた文字 (最大 16 文字)。
    captured: String,
    captured_n: usize,
}

/// 変わる範囲を求める、区切らない復号器。
struct Changed {
    dec: Decoder,
    /// 範囲が開いている (偽なら、並びの先頭から構えているだけ)。
    open: bool,
    start: u64,
    /// 範囲の中の、最後のつながりの位置。範囲を閉じるのは、これより後ろの単位の前だけ。
    p_last: u64,
    buf: String,
}

/// 変わる範囲の行数を数える (03 §2.4 の行: LF・CR・CRLF)。最初の範囲が開いてから数え始める。
#[derive(Default)]
struct Lines {
    active: bool,
    line: u64,
    prev_cr: bool,
    last_counted: Option<u64>,
    count: u64,
}

impl Lines {
    fn advance_byte(&mut self, b: u8) {
        match b {
            b'\r' => {
                self.line += 1;
                self.prev_cr = true;
            }
            b'\n' => {
                if !self.prev_cr {
                    self.line += 1;
                }
                self.prev_cr = false;
            }
            _ => self.prev_cr = false,
        }
    }

    /// 範囲の外の本文。
    fn advance(&mut self, s: &str) {
        if !self.active {
            return;
        }
        for b in s.bytes() {
            self.advance_byte(b);
        }
    }

    /// 範囲の中の文字。その文字の行を (まだ数えていなければ) 数える。CRLF の LF は CR の行。
    fn touch(&mut self, c: char) {
        self.active = true;
        let line = if c == '\n' && self.prev_cr {
            self.line.saturating_sub(1)
        } else {
            self.line
        };
        if self.last_counted != Some(line) {
            self.count += 1;
            self.last_counted = Some(line);
        }
        let mut buf = [0u8; 4];
        for &b in c.encode_utf8(&mut buf).as_bytes() {
            self.advance_byte(b);
        }
    }
}

/// つながりの検出器 (モジュールの説明を参照)。
pub struct MergeScanner {
    enc: Encoding,
    bom_written: bool,
    track_range: bool,
    local: Decoder,
    buf: String,
    run: Option<Run>,
    total: u64,
    recorded: Vec<EscapeMergeAt>,
    digest: Box<dyn Hasher>,
    regions: VecDeque<MergeRegion>,
    changed: Option<Changed>,
    changed_chars: u64,
    lines: Lines,
    to_eof: bool,
}

impl MergeScanner {
    /// `bom_written`: 出力の先頭に BOM を書く (UTF-8 / UTF-16 の `has_bom`)。変わる範囲の復号器が、開き直しと同じく
    /// 先頭の BOM を読み飛ばすかを決める。`track_range`: 変わる範囲を求める (乾式走査。本番の書き込みでは件数・位置・
    /// ダイジェストだけを見る)。`digest`: 全件のダイジェストを求めるハッシュ (空の状態で渡す)。
    pub fn new(
        enc: Encoding,
        bom_written: bool,
        track_range: bool,
        digest: Box<dyn Hasher>,
    ) -> MergeScanner {
        MergeScanner {
            enc,
            bom_written,
            track_range,
            local: Decoder::new(enc, false),
            buf: String::new(),
            run: None,
            total: 0,
            recorded: Vec::new(),
            digest,
            regions: VecDeque::new(),
            changed: None,
            changed_chars: 0,
            lines: Lines::default(),
            to_eof: false,
        }
    }

    /// 単位ごとに知らせなくてよい (並びの途中でなく、変わる範囲も開いていない)。真の間、呼び出し側はエスケープ文字の
    /// 無い区間をまとめて符号化して `bulk` で知らせてよい。
    pub fn is_idle(&self) -> bool {
        self.run.is_none() && self.changed.is_none()
    }

    /// `is_idle` の間に符号化した、エスケープ文字を含まない本文 (行を数えるだけ)。
    pub fn bulk(&mut self, text: &str) {
        self.lines.advance(text);
    }

    /// 1 単位を符号化した: `text` は本文の 1 文字 (エスケープ文字を含む) か保持方式の記録の範囲の文字列、`bytes` は
    /// その出力。`char_index` は `text` の先頭の文字の通し番号、`text_pos` は本文のバイト位置、`out_pos` は出力の通しの
    /// バイト位置 (BOM を含む)。
    pub fn unit(
        &mut self,
        text: &str,
        bytes: &[u8],
        char_index: u64,
        text_pos: usize,
        out_pos: u64,
    ) {
        if let Some(c) = single_escape(text) {
            if self.run.is_none() {
                self.start_run(c, char_index, text_pos, out_pos);
            }
            self.feed_run(c, bytes);
            let resync = self.feed_changed(text, bytes);
            self.close_if(resync, char_index);
            self.count_lines(text);
            return;
        }
        let resync = self.feed_changed(text, bytes);
        if let Some(run) = self.run.take() {
            let end = (out_pos + bytes.len() as u64, text_pos + text.len());
            self.resolve(run, text, bytes, end);
        }
        self.close_if(resync, char_index);
        self.count_lines(text);
        // 構えているだけの復号器は、並びが終わったら外す
        if self.changed.as_ref().is_some_and(|c| !c.open) {
            self.changed = None;
        }
    }

    /// 出力の終わり。`text_end_char` は本文の文字数、`text_end` は本文のバイト数、`out_end` は出力の通しのバイト数。
    pub fn finish(&mut self, text_end_char: u64, text_end: usize, out_end: u64) {
        if let Some(run) = self.run.take() {
            self.resolve(run, "", &[], (out_end, text_end));
        }
        if let Some(c) = self.changed.take()
            && c.open
        {
            self.changed_chars += text_end_char.saturating_sub(c.start);
            self.to_eof = !c.dec.is_neutral();
        }
    }

    /// 区切った確認 (ii) で突き合わせない範囲 (古い順)。確認 (ii) が読み終えたものを前から取り除く。
    pub fn regions_mut(&mut self) -> &mut VecDeque<MergeRegion> {
        &mut self.regions
    }

    /// これまでの結果。
    pub fn report(&self) -> MergeReport {
        MergeReport {
            total: self.total,
            recorded: self.recorded.clone(),
            digest: self.digest.finish(),
            changed_chars: self.changed_chars,
            changed_lines: self.lines.count,
            to_eof: self.to_eof,
        }
    }

    /// これまでの件数 (本番の書き込みが、乾式走査の件数を超えたら止めるため)。
    pub fn total(&self) -> u64 {
        self.total
    }

    /// 件数・先頭の位置・全件のダイジェストが、`total`・`recorded`・`digest` と同じか (本番の書き込みの終わりに、
    /// 乾式走査と比べる)。
    pub fn same_as(&self, total: u64, recorded: &[EscapeMergeAt], digest: u64) -> bool {
        self.total == total && self.recorded == recorded && self.digest.finish() == digest
    }

    fn start_run(&mut self, c: char, char_index: u64, text_pos: usize, out_pos: u64) {
        self.local.reset();
        // つながるかはまだ分からない。分かるまで、確認 (ii) はこの位置から先を待つ
        self.regions.push_back(MergeRegion {
            out_start: out_pos,
            out_end: None,
            text_start: text_pos,
            text_end: None,
        });
        if self.track_range && self.changed.is_none() {
            // 並びの先頭では、区切らない復号器は持ち越しが無い (前の単位は完全な列) ので、新しい復号器と同じ。
            // 出力の先頭なら、開き直しと同じく先頭の BOM を読み飛ばす
            self.changed = Some(Changed::new(self.enc, out_pos == 0 && !self.bom_written));
        }
        self.run = Some(Run {
            char_start: char_index,
            len: 0,
            last_escape: c,
            expect: VecDeque::new(),
            matched: 0,
            mismatch: None,
            captured: String::new(),
            captured_n: 0,
        });
    }

    /// 並びのエスケープ文字を局所の復号器に渡す。違いが出たら、その位置でつながりが決まる (範囲を開く)。
    fn feed_run(&mut self, c: char, bytes: &[u8]) {
        let Some(run) = self.run.as_mut() else {
            return;
        };
        run.len += 1;
        run.last_escape = c;
        if run.mismatch.is_some() {
            if run.captured_n >= MERGE_DECODED_CHARS {
                // 違いの位置と、示す文字列は決まった (残りの並びは復号しなくてよい)
                return;
            }
        } else {
            run.expect.push_back(c);
        }
        self.buf.clear();
        self.local.decode(bytes, false, &mut self.buf);
        let before = run.mismatch;
        consume(run, &self.buf);
        if before.is_none()
            && let Some(k) = run.mismatch
        {
            let p = run.char_start + k;
            self.on_flag(p);
        }
    }

    /// 並びの直後の 1 単位 (`text` / `bytes`。文書の末尾なら空) で局所の検査を終える。
    fn resolve(&mut self, mut run: Run, text: &str, bytes: &[u8], end: (u64, usize)) {
        // 並びの途中で違いが出ていれば、つながりはそのとき決めた (変わる範囲もそのとき開いた)
        let early = run.mismatch.is_some();
        if !early {
            run.expect.extend(text.chars());
        }
        if run.captured_n < MERGE_DECODED_CHARS {
            self.buf.clear();
            self.local.decode(bytes, true, &mut self.buf);
            consume(&mut run, &self.buf);
        }
        if run.mismatch.is_none() && !run.expect.is_empty() {
            // 出てきた文字が足りない
            run.mismatch = Some(run.matched);
        }
        let Some(k) = run.mismatch else {
            // つながらない: 区切りを外す (確認 (ii) は、この並びと直後の単位もふつうに突き合わせる)
            self.regions.pop_back();
            return;
        };
        if let Some(r) = self.regions.back_mut() {
            r.out_end = Some(end.0);
            r.text_end = Some(end.1);
        }
        let p_idx = k.min(run.len.saturating_sub(1));
        let p = run.char_start + p_idx;
        let unit_chars = text.chars().count() as u64;
        // 示す文字列: 局所の復号の、示す位置から後ろ。違いの位置より前は期待どおりに出ている (並びの最後のエスケープ
        // 文字と、直後の単位の先頭の文字)
        let mut decoded = String::new();
        if p_idx < k {
            decoded.push(run.last_escape);
            decoded.extend(
                text.chars()
                    .take(usize::try_from(k - run.len).unwrap_or(usize::MAX)),
            );
        }
        decoded.push_str(&run.captured);
        let decoded: String = decoded.chars().take(MERGE_DECODED_CHARS).collect();
        let at = EscapeMergeAt {
            char_index: p,
            len_chars: u32::try_from(run.len + unit_chars - p_idx).unwrap_or(u32::MAX),
            decoded,
        };
        self.digest.write_u64(at.char_index);
        self.digest.write_u32(at.len_chars);
        self.digest.write_usize(at.decoded.len());
        self.digest.write(at.decoded.as_bytes());
        self.total += 1;
        if self.recorded.len() < MAX_MERGES {
            self.recorded.push(at);
        }
        if !early {
            self.on_flag(p);
        }
    }

    /// つながりを位置 `p` に見つけた: 変わる範囲を開く (開いていれば広げる)。
    fn on_flag(&mut self, p: u64) {
        let Some(c) = self.changed.as_mut() else {
            return;
        };
        if c.open {
            c.p_last = c.p_last.max(p);
        } else {
            c.open = true;
            c.start = p;
            c.p_last = p;
            // 並びのエスケープ文字は改行ではない: 位置の行を数える
            self.lines.touch('\u{10FF00}');
        }
    }

    /// 区切らない復号器に 1 単位を渡す。単位の出力の後に持ち越しが無く、その単位で出たものが単位の文字で終われば真
    /// (その単位の前で、復号の側と符号化の側の境界が再び一致した)。
    fn feed_changed(&mut self, text: &str, bytes: &[u8]) -> bool {
        let Some(c) = self.changed.as_mut() else {
            return false;
        };
        c.buf.clear();
        c.dec.decode(bytes, false, &mut c.buf);
        c.dec.is_neutral() && !text.is_empty() && c.buf.ends_with(text)
    }

    /// 単位 `char_index` の前で範囲を閉じられれば閉じる。
    fn close_if(&mut self, resync: bool, char_index: u64) {
        if !resync {
            return;
        }
        let Some(c) = self.changed.as_mut() else {
            return;
        };
        if c.open && char_index > c.p_last {
            self.changed_chars += char_index - c.start;
            c.open = false;
        }
    }

    fn count_lines(&mut self, text: &str) {
        if self.changed.as_ref().is_some_and(|c| c.open) {
            for ch in text.chars() {
                self.lines.touch(ch);
            }
        } else {
            self.lines.advance(text);
        }
    }
}

impl Changed {
    fn new(enc: Encoding, skip_bom: bool) -> Changed {
        Changed {
            dec: Decoder::new(enc, skip_bom),
            open: false,
            start: 0,
            p_last: 0,
            buf: String::new(),
        }
    }
}

/// `text` が 1 つのエスケープ文字だけなら、その文字。
fn single_escape(text: &str) -> Option<char> {
    let mut it = text.chars();
    let c = it.next()?;
    (it.next().is_none() && unescape_char(c).is_some()).then_some(c)
}

/// 局所の復号器が出した文字 `got` を、期待する文字と前から突き合わせる。
fn consume(run: &mut Run, got: &str) {
    for g in got.chars() {
        if run.mismatch.is_some() {
            if run.captured_n < MERGE_DECODED_CHARS {
                run.captured.push(g);
                run.captured_n += 1;
            }
            continue;
        }
        match run.expect.pop_front() {
            Some(e) if e == g => run.matched += 1,
            _ => {
                run.mismatch = Some(run.matched);
                run.captured.push(g);
                run.captured_n += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::hash::DefaultHasher;

    use super::*;
    use crate::encode::{EncodeReport, Encoder, EscapePolicy};
    use crate::utf8::escape_byte;

    /// 本文の部品: 不正バイトのエスケープ文字か、ふつうの文字列 (1 文字ずつ符号化する)。
    enum U<'a> {
        E(u8),
        S(&'a str),
    }

    struct Out {
        bytes: Vec<u8>,
        report: MergeReport,
        /// 終わりに残った区切り (つながる並びだけ)。
        regions: Vec<MergeRegion>,
    }

    /// 部品を 1 文字ずつ符号化して検出器に渡す (保存の符号化の流れの単位ごとの経路と同じ)。
    fn scan(enc: Encoding, bom: bool, units: &[U<'_>]) -> Out {
        let text: String = units
            .iter()
            .map(|u| match u {
                U::E(b) => escape_byte(*b).to_string(),
                U::S(s) => (*s).to_owned(),
            })
            .collect();
        scan_str(enc, bom, &text)
    }

    /// 本文を 1 文字ずつ符号化して検出器に渡す。
    fn scan_str(enc: Encoding, bom: bool, text: &str) -> Out {
        let mut e = Encoder::new(enc, bom, EscapePolicy::Restore);
        let (mut out, mut rep) = (Vec::new(), EncodeReport::default());
        e.encode("", &mut out, &mut rep);
        let bom_written = !out.is_empty();
        let mut s = MergeScanner::new(enc, bom_written, true, Box::new(DefaultHasher::new()));
        let mut pos = 0;
        for c in text.chars() {
            let one = c.to_string();
            let idx = e.chars_done();
            let before = out.len();
            e.encode(&one, &mut out, &mut rep);
            s.unit(&one, &out[before..], idx, pos, before as u64);
            pos += one.len();
        }
        e.finish(&mut out, &mut rep);
        assert!(rep.is_empty(), "表現不能が無い部品で組む");
        s.finish(e.chars_done(), pos, out.len() as u64);
        Out {
            bytes: out,
            report: s.report(),
            regions: s.regions_mut().iter().copied().collect(),
        }
    }

    fn at(r: &MergeReport, i: usize) -> (u64, u32, &str) {
        let a = &r.recorded[i];
        (a.char_index, a.len_chars, a.decoded.as_str())
    }

    fn range(r: &MergeReport) -> (u64, u64, bool) {
        (r.changed_chars, r.changed_lines, r.to_eof)
    }

    // 03 §8.3.1 の例: UTF-8 で <0xE3><0x81><0x82> が並ぶと「あ」になる。変わるのは 3 文字 (直後の x は変わらない)
    #[test]
    fn utf8_の3つの並びはあになる() {
        let o = scan(
            Encoding::Utf8,
            false,
            &[U::S("a"), U::E(0xE3), U::E(0x81), U::E(0x82), U::S("x\n")],
        );
        assert_eq!(o.bytes, b"a\xE3\x81\x82x\n");
        assert_eq!(o.report.total, 1);
        assert_eq!(at(&o.report, 0), (1, 4, "あx"));
        assert_eq!(range(&o.report), (3, 1, false));
        // 区切りは、つながる並びと直後の 1 単位
        assert_eq!(
            o.regions,
            [MergeRegion {
                out_start: 1,
                out_end: Some(5),
                text_start: 1,
                text_end: Some(14),
            }]
        );
    }

    // Shift_JIS で <0x81> の直後に @ (0x8140 は全角の空白)
    #[test]
    fn shift_jis_の_81_の直後の_at_は全角の空白になる() {
        let o = scan(Encoding::ShiftJis, false, &[U::E(0x81), U::S("@x")]);
        assert_eq!(o.report.total, 1);
        // 局所の復号は並びと直後の 1 単位 (@) まで
        assert_eq!(at(&o.report, 0), (0, 2, "\u{3000}"));
        assert_eq!(range(&o.report), (2, 1, false));
    }

    // EUC-JP で <0x8F> の直後にかな: 補助漢字の 3 バイト列の頭になり、後ろのバイトを巻き込む
    #[test]
    fn euc_jp_の_8f_の直後のかなは巻き込まれる() {
        let o = scan(Encoding::EucJp, false, &[U::E(0x8F), U::S("あいz")]);
        assert_eq!(o.report.total, 1);
        assert_eq!(o.report.recorded[0].char_index, 0);
        assert!(o.report.changed_chars >= 2, "{:?}", o.report);
        assert!(!o.report.to_eof);
    }

    // UTF-16 で奇数個の並び: 後ろが 1 バイトずれ、文書の末尾まで戻らない (改行も別の文字になる)
    #[test]
    fn utf16_の奇数個の並びは末尾まで変わる() {
        for enc in [Encoding::Utf16Le, Encoding::Utf16Be] {
            let o = scan(enc, true, &[U::S("ab\n"), U::E(0x41), U::S("cd\nef")]);
            assert_eq!(o.report.total, 1, "{enc:?}");
            assert_eq!(o.report.recorded[0].char_index, 3);
            assert_eq!(range(&o.report), (6, 2, true), "{enc:?}");
        }
    }

    // Shift_JIS で <0x81> の後に全角文字が 5,000 字続くと、次の ASCII のバイト (改行) まで 5,001 字が変わる
    #[test]
    fn shift_jis_の_81_の後の全角文字は改行までずれ続ける() {
        let zen = "亜".repeat(5000);
        let o = scan(
            Encoding::ShiftJis,
            false,
            &[U::E(0x81), U::S(&zen), U::S("\nabc")],
        );
        assert_eq!(o.report.total, 1);
        assert_eq!(range(&o.report), (5001, 1, false));
        // 改行が無ければ末尾まで (末尾で持ち越しが残る)
        let o = scan(Encoding::ShiftJis, false, &[U::E(0x81), U::S(&zen)]);
        assert_eq!((o.report.changed_chars, o.report.to_eof), (5001, true));
    }

    // 重なるつながりは 1 つの範囲にまとめる: Shift_JIS の <0x81>亜<0x81>亜\n は、並びごとの局所の検査では 2 件だが、
    // 開き直すと 1 件目のずれ (81 88 / 9F 81) が 2 件目の並びまで続き、2 つ目の亜 (88 9F) で戻る。範囲は 1 つで 3 文字
    #[test]
    fn 重なるつながりは_1_つの範囲にまとめる() {
        let o = scan(
            Encoding::ShiftJis,
            false,
            &[U::E(0x81), U::S("亜"), U::E(0x81), U::S("亜\nx")],
        );
        assert_eq!(o.report.total, 2);
        assert_eq!(
            (
                o.report.recorded[0].char_index,
                o.report.recorded[1].char_index
            ),
            (0, 2)
        );
        assert_eq!(range(&o.report), (3, 1, false));
    }

    // つながらない並び (直後が ASCII・改行): 見つけない。範囲も数えず、区切りも残さない
    #[test]
    fn つながらない並びは見つけない() {
        for (enc, b) in [
            (Encoding::Utf8, 0xE3),
            (Encoding::ShiftJis, 0x81),
            (Encoding::EucJp, 0x8F),
        ] {
            let o = scan(enc, false, &[U::S("x"), U::E(b), U::S("\nabc"), U::E(b)]);
            let empty = MergeScanner::new(enc, false, true, Box::new(DefaultHasher::new()));
            assert_eq!(o.report, empty.report(), "{enc:?}");
            assert!(o.regions.is_empty(), "{enc:?}");
        }
    }

    // 書く文字コード自身の BOM の並びが出力の先頭にできると、初期状態の復号器では U+FEFF になるので、局所の検査が
    // 見つける (エスケープ文字だけの並び、エスケープ文字と通常の文字の組の両方)。BOM を書く保存でも同じ
    #[test]
    fn 書く文字コード自身の_bom_の並びは局所の検査で見つける() {
        let cases: [(Encoding, &[U<'_>]); 5] = [
            (
                Encoding::Utf8,
                &[U::E(0xEF), U::E(0xBB), U::E(0xBF), U::S("a")],
            ),
            (Encoding::Utf16Le, &[U::E(0xFF), U::E(0xFE), U::S("a")]),
            (Encoding::Utf16Be, &[U::E(0xFE), U::E(0xFF), U::S("a")]),
            // エスケープ文字と通常の文字の組: <0xFF> と þ (U+00FE は LE で FE 00)、<0xFE> と ！ (U+FF01 は BE で FF 01)
            (Encoding::Utf16Le, &[U::E(0xFF), U::S("þa")]),
            (Encoding::Utf16Be, &[U::E(0xFE), U::S("！a")]),
        ];
        for (enc, units) in cases {
            for bom in [false, true] {
                let o = scan(enc, bom, units);
                assert_eq!(o.report.total, 1, "{enc:?} bom={bom}");
                assert_eq!(o.report.recorded[0].char_index, 0, "{enc:?} bom={bom}");
            }
        }
        // 別の文字コードの BOM の並びは見つけない (同じ文字コードを指定して開き直せば元に戻る。03 §8.3.1 の「検出しない
        // もの」)
        let others: [(Encoding, &[u8]); 5] = [
            (Encoding::Utf8, b"\xFF\xFE"),
            (Encoding::Utf8, b"\xFE\xFF"),
            (Encoding::ShiftJis, b"\xEF\xBB\xBF"),
            (Encoding::ShiftJis, b"\xFF\xFE"),
            (Encoding::EucJp, b"\xFF\xFE"),
        ];
        for (enc, head) in others {
            // そのバイト列で始まるファイルを、その文字コードを指定して開いた本文 (未編集)
            let file = [head, b"ab\n"].concat();
            let mut d = Decoder::new(enc, true);
            let mut text = String::new();
            d.decode(&file, true, &mut text);
            assert!(d.invalid_bytes() > 0 && !d.has_bom(), "{enc:?} {head:02X?}");
            let o = scan_str(enc, false, &text);
            assert_eq!(o.bytes, file, "{enc:?}");
            assert_eq!(o.report.total, 0, "{enc:?} {head:02X?}");
        }
        // 本文の U+FEFF (エスケープ文字でない) は検出の対象外
        let o = scan(Encoding::Utf8, false, &[U::S("\u{FEFF}a"), U::E(0x80)]);
        assert_eq!(o.report.total, 0);
    }

    // 件数は全部数え、記録は先頭 100 件。ダイジェストは全件を見る (101 件目より後ろの違いでも変わる)
    #[test]
    fn 件数は全部数え記録は先頭百件でダイジェストは全件() {
        let mut units = Vec::new();
        for _ in 0..150 {
            units.extend([U::E(0x81), U::S("@"), U::S("\n")]);
        }
        let o = scan(Encoding::ShiftJis, false, &units);
        assert_eq!(o.report.total, 150);
        assert_eq!(o.report.recorded.len(), MAX_MERGES);
        assert_eq!(o.report.recorded[1].char_index, 3);
        assert_eq!((o.report.changed_chars, o.report.changed_lines), (300, 150));
        // 130 件目のつながる文字だけを変える (0x8140 → 0x8141)
        let mut other = Vec::new();
        for i in 0..150 {
            other.extend([
                U::E(0x81),
                U::S(if i == 130 { "A" } else { "@" }),
                U::S("\n"),
            ]);
        }
        let p = scan(Encoding::ShiftJis, false, &other);
        assert_eq!(
            (p.report.total, &p.report.recorded),
            (150, &o.report.recorded)
        );
        assert_ne!(p.report.digest, o.report.digest);
    }

    // 復号器の持ち越しの判定 (`Decoder::is_neutral`)
    #[test]
    fn 復号器の持ち越しを判定する() {
        let cases: [(Encoding, &[u8], bool); 10] = [
            (Encoding::Utf8, b"a", true),
            (Encoding::Utf8, b"\xE3\x81", false),
            (Encoding::ShiftJis, b"\x81", false),
            (Encoding::ShiftJis, b"\x81\x40", true),
            (Encoding::EucJp, b"\x8F\xB0", false),
            (Encoding::Utf16Le, b"a", false),
            (Encoding::Utf16Le, b"a\x00", true),
            (Encoding::Utf16Le, b"\x00\xD8", false),
            (Encoding::Utf16Be, b"\xD8\x00\xDC\x00", true),
            (Encoding::Utf16Be, b"\xD8\x00\x00\x41", true),
        ];
        for (enc, bytes, want) in cases {
            let mut d = Decoder::new(enc, false);
            d.decode(bytes, false, &mut String::new());
            assert_eq!(d.is_neutral(), want, "{enc:?} {bytes:02X?}");
            d.reset();
            assert!(d.is_neutral());
        }
    }
}
