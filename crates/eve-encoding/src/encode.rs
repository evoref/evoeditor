//! すべての文字コードのチャンク単位のエンコード (docs/03-core-design.md §8.6)。
//!
//! - **E1 照合**: UTF-8 以外 (UTF-16 を除く) では、出力したバイト列を同じ文字コードで復号して元の文字列と照合し、
//!   `Unmappable` と、黙示の置換 (`¥` → 0x5C、`−` → 0x817C、半角カナ → 全角カナなど) を同じ「表現不能」として
//!   位置つきで報告する (黙って置換しない、FR-112)。最大 100 件を記録し、総数は数える。
//! - **CP932 の外字** (U+E000〜U+E757): Shift_JIS では自前の逆算で 2 バイトを出す (encoding_rs は復号専用、§8.7)。
//! - **UTF-16**: 自前の書き手 (encoding_rs は UTF-16 のエンコードを提供しない。作ると黙って UTF-8 になる)。
//! - **ISO-2022-JP (E6)**: 行ごとにエンコーダを閉じ、各行を ASCII 状態で終える。行は次の改行までためてから符号化する
//!   (改行を含まない巨大な 1 行は、その行の長さだけメモリを使う)。
//! - **不正バイトのエスケープ** (`U+10FF00 + b`): `EscapePolicy::Restore` なら元のバイトに戻して書く (§8.3)。

use encoding_rs::EncoderResult;

use crate::decode::Decoder;
use crate::encoding::Encoding;
use crate::utf8::{PuaEscapeMode, Utf8Encoder, unescape_char};

/// 記録する表現不能の最大件数 (総数は数える)。
pub const MAX_RECORDED: usize = 100;

/// 不正バイトのエスケープ文字の扱い (E4)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EscapePolicy {
    /// 元のバイトに戻して書く (元の文字コードのまま保存するとき、無編集保存でバイト一致)。
    Restore,
    /// 通常の文字として書く (`DisabledCollision`: 元のテキストが真にこの範囲の文字を含む)。表せない文字コードでは表現不能。
    AsText,
    /// 表現不能として報告する (別の文字コードで保存するとき・状態付き符号化、E4)。
    Reject,
}

impl EscapePolicy {
    /// `PuaEscapeMode` から (同じ文字コードで保存するとき)。
    pub fn from_mode(mode: PuaEscapeMode) -> EscapePolicy {
        match mode {
            PuaEscapeMode::Enabled => EscapePolicy::Restore,
            PuaEscapeMode::DisabledCollision => EscapePolicy::AsText,
            _ => EscapePolicy::Reject,
        }
    }
}

/// 表現不能の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnmappableKind {
    /// エンコーダが `Unmappable` を返した (またはエスケープ文字を書けない)。
    Unmappable,
    /// 別の文字として黙って写された (照合で不一致)。
    Substituted,
}

/// 表現不能の 1 件。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unmappable {
    /// これまでに `encode` へ渡した文字の通し番号 (0 始まり、Unicode スカラー値の数)。
    pub char_index: u64,
    pub ch: char,
    pub kind: UnmappableKind,
}

/// 保存中に見つけた表現不能の報告。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EncodeReport {
    /// 先頭 `MAX_RECORDED` 件。
    pub recorded: Vec<Unmappable>,
    /// 総数。
    pub total: u64,
}

impl EncodeReport {
    pub fn is_empty(&self) -> bool {
        self.total == 0
    }

    fn push(&mut self, u: Unmappable) {
        self.total += 1;
        if self.recorded.len() < MAX_RECORDED {
            self.recorded.push(u);
        }
    }
}

/// CP932 の外字 (ユーザー定義領域 0xF040〜0xF9FC) ↔ U+E000〜U+E757 (§8.7、OQ-206)。
pub mod cp932 {
    pub const GAIJI_FIRST: u32 = 0xE000;
    pub const GAIJI_LAST: u32 = 0xE757;

    /// 外字の符号位置 → Shift_JIS の 2 バイト。
    pub fn gaiji_to_sjis(ch: char) -> Option<[u8; 2]> {
        let v = ch as u32;
        if !(GAIJI_FIRST..=GAIJI_LAST).contains(&v) {
            return None;
        }
        let n = v - GAIJI_FIRST;
        let lead = 0xF0 + (n / 188) as u8;
        let t = (n % 188) as u8;
        let trail = if t < 0x3F { 0x40 + t } else { 0x41 + t };
        Some([lead, trail])
    }
}

fn reject(report: &mut EncodeReport, idx: u64, ch: char, kind: UnmappableKind) {
    report.push(Unmappable {
        char_index: idx,
        ch,
        kind,
    });
}

/// 文字コードごとのエンコーダ。`encode` をチャンクごとに呼び、最後に `finish` を呼ぶ。
pub struct Encoder {
    enc: Encoding,
    policy: EscapePolicy,
    pending_bom: bool,
    /// これまでに渡した文字数。
    chars_done: u64,
    utf8: Option<Utf8Encoder>,
    /// ISO-2022-JP: 改行までためている行と、その先頭の文字の通し番号。
    line: String,
    line_start: u64,
}

fn rs_encode(enc: Encoding, text: &str, last: bool, out: &mut Vec<u8>) -> Vec<(usize, char)> {
    // `Unmappable` になった文字の (text 内のバイト位置、文字) を返す。出力はその文字を飛ばして続ける
    let mut e = enc.rs().new_encoder();
    rs_encode_with(&mut e, text, last, out)
}

fn rs_encode_with(
    e: &mut encoding_rs::Encoder,
    text: &str,
    last: bool,
    out: &mut Vec<u8>,
) -> Vec<(usize, char)> {
    let mut bad = Vec::new();
    let mut pos = 0;
    loop {
        let rest = &text[pos..];
        let want = e
            .max_buffer_length_from_utf8_if_no_unmappables(rest.len())
            .unwrap_or(rest.len() * 4 + 16);
        let at = out.len();
        out.resize(at + want, 0);
        let (res, read, written) =
            e.encode_from_utf8_without_replacement(rest, &mut out[at..], last);
        out.truncate(at + written);
        pos += read;
        match res {
            EncoderResult::InputEmpty => return bad,
            EncoderResult::OutputFull => {}
            EncoderResult::Unmappable(ch) => bad.push((pos - ch.len_utf8(), ch)),
        }
    }
}

impl Encoder {
    /// `bom`: 先頭に BOM を書く (UTF-8 / UTF-16)。
    pub fn new(enc: Encoding, bom: bool, policy: EscapePolicy) -> Encoder {
        let utf8 = (enc == Encoding::Utf8).then(|| {
            Utf8Encoder::new(
                bom,
                match policy {
                    EscapePolicy::Restore => PuaEscapeMode::Enabled,
                    _ => PuaEscapeMode::DisabledCollision,
                },
            )
        });
        Encoder {
            enc,
            policy,
            pending_bom: bom && enc.is_utf16(),
            chars_done: 0,
            utf8,
            line: String::new(),
            line_start: 0,
        }
    }

    pub fn encoding(&self) -> Encoding {
        self.enc
    }

    /// `text` を符号化して `out` に追記する。表現不能は `report` に記録する (その文字は出力しない)。
    pub fn encode(&mut self, text: &str, out: &mut Vec<u8>, report: &mut EncodeReport) {
        let n_chars = text.chars().count() as u64;
        match self.enc {
            Encoding::Utf8 => self.encode_utf8(text, out, report),
            Encoding::Utf16Le | Encoding::Utf16Be => self.encode_utf16(text, out, report),
            Encoding::Iso2022Jp => self.encode_iso(text, out, report),
            Encoding::ShiftJis | Encoding::EucJp => self.encode_rs(text, out, report),
        }
        self.chars_done += n_chars;
    }

    /// 最後に呼ぶ (ISO-2022-JP の最終行を ASCII 状態で閉じる)。
    pub fn finish(&mut self, out: &mut Vec<u8>, report: &mut EncodeReport) {
        if self.enc == Encoding::Iso2022Jp && !self.line.is_empty() {
            let line = std::mem::take(&mut self.line);
            let start = self.line_start;
            self.flush_iso_line(&line, start, out, report);
        }
    }

    fn encode_utf8(&mut self, text: &str, out: &mut Vec<u8>, report: &mut EncodeReport) {
        let Some(u) = self.utf8.as_mut() else { return };
        if self.policy == EscapePolicy::Reject {
            // 別の文字コード由来のエスケープ文字は UTF-8 でも表現不能 (E4)
            let mut start = 0;
            for (i, (bi, c)) in text.char_indices().enumerate() {
                if unescape_char(c).is_some() {
                    u.encode(&text[start..bi], out);
                    reject(
                        report,
                        self.chars_done + i as u64,
                        c,
                        UnmappableKind::Unmappable,
                    );
                    start = bi + c.len_utf8();
                }
            }
            u.encode(&text[start..], out);
        } else {
            u.encode(text, out);
        }
    }

    fn encode_utf16(&mut self, text: &str, out: &mut Vec<u8>, report: &mut EncodeReport) {
        if std::mem::take(&mut self.pending_bom) {
            out.extend_from_slice(self.enc.bom().unwrap_or_default());
        }
        let le = self.enc == Encoding::Utf16Le;
        let mut buf = [0u16; 2];
        for (i, c) in text.chars().enumerate() {
            if let Some(b) = unescape_char(c) {
                match self.policy {
                    EscapePolicy::Restore => {
                        out.push(b);
                        continue;
                    }
                    EscapePolicy::Reject => {
                        reject(
                            report,
                            self.chars_done + i as u64,
                            c,
                            UnmappableKind::Unmappable,
                        );
                        continue;
                    }
                    EscapePolicy::AsText => {}
                }
            }
            for u in c.encode_utf16(&mut buf) {
                out.extend_from_slice(&if le { u.to_le_bytes() } else { u.to_be_bytes() });
            }
        }
    }

    /// Shift_JIS / EUC-JP (状態を持たない)。
    fn encode_rs(&mut self, text: &str, out: &mut Vec<u8>, report: &mut EncodeReport) {
        let base = self.chars_done;
        // 特別な文字 (エスケープ・外字) で区切り、間を encoding_rs で符号化して照合する
        let mut run_start = 0;
        let mut chars_before_run = 0u64;
        for (ci, (bi, c)) in text.char_indices().enumerate() {
            let ci = ci as u64;
            let special = unescape_char(c).is_some()
                || (self.enc == Encoding::ShiftJis && cp932::gaiji_to_sjis(c).is_some());
            if special {
                self.run_rs(&text[run_start..bi], base + chars_before_run, out, report);
                if let Some(b) = unescape_char(c) {
                    match self.policy {
                        EscapePolicy::Restore => out.push(b),
                        _ => reject(report, base + ci, c, UnmappableKind::Unmappable),
                    }
                } else if let Some(b) = cp932::gaiji_to_sjis(c) {
                    out.extend_from_slice(&b);
                }
                run_start = bi + c.len_utf8();
                chars_before_run = ci + 1;
            }
        }
        self.run_rs(&text[run_start..], base + chars_before_run, out, report);
    }

    /// 特別な文字を含まない 1 区間を符号化して照合する。`first_idx` は区間の先頭の文字の通し番号。
    fn run_rs(&self, run: &str, first_idx: u64, out: &mut Vec<u8>, report: &mut EncodeReport) {
        if run.is_empty() {
            return;
        }
        let start = out.len();
        let bad = rs_encode(self.enc, run, true, out);
        for &(bi, ch) in &bad {
            let idx = first_idx + run[..bi].chars().count() as u64;
            reject(report, idx, ch, UnmappableKind::Unmappable);
        }
        // 表現できた文字だけの文字列 (照合の期待値)
        let expected: String = if bad.is_empty() {
            run.to_owned()
        } else {
            let mut s = String::with_capacity(run.len());
            let mut last = 0;
            for &(bi, ch) in &bad {
                s.push_str(&run[last..bi]);
                last = bi + ch.len_utf8();
            }
            s.push_str(&run[last..]);
            s
        };
        self.verify(&out[start..], &expected, run, &bad, first_idx, report);
    }

    /// `bytes` を復号して `expected` と照合し、一致しない文字を `Substituted` として報告する。
    fn verify(
        &self,
        bytes: &[u8],
        expected: &str,
        original: &str,
        bad: &[(usize, char)],
        first_idx: u64,
        report: &mut EncodeReport,
    ) {
        let mut d = Decoder::new(self.enc, false);
        let mut got = String::new();
        d.decode(bytes, true, &mut got);
        if got == expected {
            return;
        }
        // 文字ごとに対応づける (黙示の置換は 1 文字を 1 文字に写す)。表現不能で飛ばした文字は原文の側で飛ばす
        let bad_bytes: Vec<usize> = bad.iter().map(|b| b.0).collect();
        let mut got_it = got.chars();
        let mut idx = first_idx;
        for (bi, c) in original.char_indices() {
            if bad_bytes.contains(&bi) {
                idx += 1;
                continue;
            }
            match got_it.next() {
                Some(g) if g == c => {}
                _ => reject(report, idx, c, UnmappableKind::Substituted),
            }
            idx += 1;
        }
    }

    /// ISO-2022-JP: 行末までためて、行ごとに符号化する (E6)。
    fn encode_iso(&mut self, text: &str, out: &mut Vec<u8>, report: &mut EncodeReport) {
        let mut rest = text;
        let mut consumed_chars = 0u64;
        while !rest.is_empty() {
            if self.line.is_empty() {
                self.line_start = self.chars_done + consumed_chars;
            }
            match rest.find(['\n', '\r']) {
                Some(i) => {
                    self.line.push_str(&rest[..i]);
                    let eol = rest.as_bytes()[i];
                    let line = std::mem::take(&mut self.line);
                    let start = self.line_start;
                    self.flush_iso_line(&line, start, out, report);
                    out.push(eol);
                    consumed_chars += rest[..i].chars().count() as u64 + 1;
                    rest = &rest[i + 1..];
                }
                None => {
                    self.line.push_str(rest);
                    return;
                }
            }
        }
    }

    fn flush_iso_line(
        &mut self,
        line: &str,
        start: u64,
        out: &mut Vec<u8>,
        report: &mut EncodeReport,
    ) {
        // エスケープ文字 (不正バイト) は状態付き符号化では元のバイトに戻せない: 表現不能
        let mut clean = String::with_capacity(line.len());
        let mut kept: Vec<(usize, char)> = Vec::new(); // clean の各文字の元の位置
        for (i, c) in line.chars().enumerate() {
            if unescape_char(c).is_some() {
                reject(report, start + i as u64, c, UnmappableKind::Unmappable);
            } else {
                clean.push(c);
                kept.push((i, c));
            }
        }
        let bytes_start = out.len();
        let bad = rs_encode(Encoding::Iso2022Jp, &clean, true, out);
        let mut bad_idx: Vec<usize> = Vec::new();
        for &(bi, ch) in &bad {
            let ci = clean[..bi].chars().count();
            bad_idx.push(ci);
            reject(
                report,
                start + kept[ci].0 as u64,
                ch,
                UnmappableKind::Unmappable,
            );
        }
        // 照合: 表現できた文字だけを復号結果と比べる
        let mut d = Decoder::new(Encoding::Iso2022Jp, false);
        let mut got = String::new();
        d.decode(&out[bytes_start..], true, &mut got);
        let expected: String = clean
            .chars()
            .enumerate()
            .filter(|(i, _)| !bad_idx.contains(i))
            .map(|(_, c)| c)
            .collect();
        if got != expected {
            let mut g = got.chars();
            for (i, c) in clean.chars().enumerate() {
                if bad_idx.contains(&i) {
                    continue;
                }
                match g.next() {
                    Some(x) if x == c => {}
                    _ => reject(
                        report,
                        start + kept[i].0 as u64,
                        c,
                        UnmappableKind::Substituted,
                    ),
                }
            }
        }
    }
}

/// 照合なしで、この文字コードの出力長を数える必要がある呼び側のための補助 (乾式走査の出力長)。UTF-16 と UTF-8 だけ厳密。
pub fn utf16_len(text: &str, policy: EscapePolicy) -> usize {
    text.chars()
        .map(|c| match (unescape_char(c), policy) {
            (Some(_), EscapePolicy::Restore) => 1,
            _ => c.len_utf16() * 2,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utf8::escape_byte;

    fn enc(e: Encoding, bom: bool, p: EscapePolicy, chunks: &[&str]) -> (Vec<u8>, EncodeReport) {
        let mut en = Encoder::new(e, bom, p);
        let (mut out, mut rep) = (Vec::new(), EncodeReport::default());
        for c in chunks {
            en.encode(c, &mut out, &mut rep);
        }
        en.finish(&mut out, &mut rep);
        (out, rep)
    }

    fn all(e: Encoding, text: &str) -> (Vec<u8>, EncodeReport) {
        enc(e, false, EscapePolicy::Restore, &[text])
    }

    #[test]
    fn shift_jis_と_euc_jp_に書く() {
        let (b, r) = all(Encoding::ShiftJis, "日本語abc");
        assert_eq!(b, b"\x93\xFA\x96\x7B\x8C\xEAabc");
        assert!(r.is_empty());
        let (b, r) = all(Encoding::EucJp, "日本");
        assert_eq!(b, b"\xC6\xFC\xCB\xDC");
        assert!(r.is_empty());
    }

    #[test]
    fn 表現不能と黙示の置換を位置つきで報告する() {
        // 絵文字は Unmappable、¥ は 0x5C に黙って写される、− は 0x817C に写される
        let (_, r) = all(Encoding::ShiftJis, "a😀b¥c−d");
        let kinds: Vec<(u64, char, UnmappableKind)> = r
            .recorded
            .iter()
            .map(|u| (u.char_index, u.ch, u.kind))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (1, '😀', UnmappableKind::Unmappable),
                (3, '¥', UnmappableKind::Substituted),
                (5, '−', UnmappableKind::Substituted),
            ]
        );
        assert_eq!(r.total, 3);
    }

    #[test]
    fn 位置はチャンクをまたいで通し番号() {
        let (_, r) = enc(
            Encoding::ShiftJis,
            false,
            EscapePolicy::Restore,
            &["ab", "c😀"],
        );
        assert_eq!(r.recorded[0].char_index, 3);
    }

    #[test]
    fn 互換文字は_unmappable() {
        for c in ['〜', '‖', '¢', '£', '¬', '¦', '—'] {
            let (_, r) = all(Encoding::ShiftJis, &c.to_string());
            assert_eq!(r.total, 1, "{c} は Shift_JIS で表現不能");
        }
    }

    #[test]
    fn cp932_の外字は逆算して書く() {
        // U+E000 → 0xF040、U+E757 → 0xF9FC、U+E03F → 0xF07E、U+E040 → 0xF080
        assert_eq!(cp932::gaiji_to_sjis('\u{E000}'), Some([0xF0, 0x40]));
        assert_eq!(cp932::gaiji_to_sjis('\u{E757}'), Some([0xF9, 0xFC]));
        assert_eq!(cp932::gaiji_to_sjis('\u{E03E}'), Some([0xF0, 0x7E]));
        assert_eq!(cp932::gaiji_to_sjis('\u{E03F}'), Some([0xF0, 0x80]));
        assert_eq!(cp932::gaiji_to_sjis('\u{E758}'), None);
        let (b, r) = all(Encoding::ShiftJis, "a\u{E000}b");
        assert_eq!(b, b"a\xF0\x40b");
        assert!(r.is_empty());
        // 復号して同じ文字に戻る
        let mut d = Decoder::new(Encoding::ShiftJis, false);
        let mut s = String::new();
        d.decode(&b, true, &mut s);
        assert_eq!(s, "a\u{E000}b");
        // EUC-JP では外字は表現不能
        let (_, r) = all(Encoding::EucJp, "\u{E000}");
        assert_eq!(r.total, 1);
    }

    #[test]
    fn 外字は全域で往復する() {
        for v in cp932::GAIJI_FIRST..=cp932::GAIJI_LAST {
            let c = char::from_u32(v).unwrap();
            let [l, t] = cp932::gaiji_to_sjis(c).unwrap();
            let mut d = Decoder::new(Encoding::ShiftJis, false);
            let mut s = String::new();
            d.decode(&[l, t], true, &mut s);
            assert_eq!(s, c.to_string(), "{v:#X}");
        }
    }

    #[test]
    fn 不正バイトのエスケープは元のバイトに戻す() {
        let t = format!("a{}b{}", escape_byte(0xA0), escape_byte(0x81));
        let (b, r) = all(Encoding::ShiftJis, &t);
        assert_eq!(b, b"a\xA0b\x81");
        assert!(r.is_empty());
        let (b, r) = enc(Encoding::ShiftJis, false, EscapePolicy::Reject, &[&t]);
        assert_eq!(b, b"ab");
        assert_eq!(r.total, 2, "別の文字コードで保存するときは表現不能");
    }

    #[test]
    fn utf16_は自前で書き_bom_とサロゲートを扱う() {
        let (b, r) = enc(Encoding::Utf16Le, true, EscapePolicy::Restore, &["aあ😀"]);
        assert_eq!(b, b"\xFF\xFEa\x00\x42\x30\x3D\xD8\x00\xDE");
        assert!(r.is_empty());
        let (b, _) = enc(Encoding::Utf16Be, true, EscapePolicy::Restore, &["a"]);
        assert_eq!(b, b"\xFE\xFF\x00a");
        // 不正バイトは元のバイトに戻る (孤立サロゲート・奇数長の再現)
        let t = format!("{}{}", escape_byte(0x00), escape_byte(0xD8));
        let (b, _) = enc(Encoding::Utf16Le, false, EscapePolicy::Restore, &[&t]);
        assert_eq!(b, b"\x00\xD8");
    }

    #[test]
    fn utf16_のエンコーダは_encoding_rs_を使わない() {
        // encoding_rs の UTF-16 の output_encoding は UTF-8 (黙って UTF-8 で書かれるのを防ぐ、§8.6)
        assert_eq!(encoding_rs::UTF_16LE.output_encoding(), encoding_rs::UTF_8);
        let (b, _) = enc(Encoding::Utf16Le, false, EscapePolicy::Restore, &["a"]);
        assert_eq!(b, b"a\x00");
    }

    #[test]
    fn iso_2022_jp_は行ごとに_ascii_状態で閉じる() {
        let (b, r) = all(Encoding::Iso2022Jp, "日本\nabc\n語");
        assert_eq!(b, b"\x1B$BF|K\\\x1B(B\nabc\n\x1B$B8l\x1B(B".to_vec());
        assert!(r.is_empty());
    }

    #[test]
    fn iso_2022_jp_のローマ字状態は行末で閉じる() {
        // `"¥\nabc"` → 行ごとに閉じるので `ESC ( J 5C ESC ( B 0A abc`
        let (b, r) = all(Encoding::Iso2022Jp, "¥\nabc");
        assert_eq!(b, b"\x1B(J\x5C\x1B(B\nabc".to_vec());
        assert!(r.is_empty());
    }

    #[test]
    fn iso_2022_jp_はチャンクの境目をまたいでも同じ() {
        let whole = all(Encoding::Iso2022Jp, "日本語abc\n日本語\r\nx").0;
        let t = "日本語abc\n日本語\r\nx";
        for cut in t.char_indices().map(|(i, _)| i) {
            let (a, b) = t.split_at(cut);
            let (got, _) = enc(Encoding::Iso2022Jp, false, EscapePolicy::Restore, &[a, b]);
            assert_eq!(got, whole, "cut {cut}");
        }
    }

    #[test]
    fn iso_2022_jp_の半角カナは黙って全角にせず表現不能() {
        let (_, r) = all(Encoding::Iso2022Jp, "ｱb");
        assert_eq!(r.total, 1);
        assert_eq!(r.recorded[0].ch, 'ｱ');
        assert_eq!(r.recorded[0].kind, UnmappableKind::Substituted);
    }

    #[test]
    fn iso_2022_jp_はエスケープ文字を元のバイトに戻さない() {
        let t = format!("a{}", escape_byte(0x0A));
        let (_, r) = all(Encoding::Iso2022Jp, &t);
        assert_eq!(r.total, 1);
    }

    #[test]
    fn utf8_は_reject_でエスケープ文字だけ表現不能() {
        let t = format!("あ{}", escape_byte(0x80));
        let (b, r) = enc(Encoding::Utf8, false, EscapePolicy::Reject, &[&t]);
        assert_eq!(b, "あ".as_bytes());
        assert_eq!(r.total, 1);
        let (b, r) = enc(Encoding::Utf8, false, EscapePolicy::Restore, &[&t]);
        assert_eq!(b, b"\xE3\x81\x82\x80");
        assert!(r.is_empty());
    }

    #[test]
    fn 記録は百件まで_総数は数える() {
        let t = "😀".repeat(150);
        let (_, r) = all(Encoding::ShiftJis, &t);
        assert_eq!((r.recorded.len(), r.total), (100, 150));
    }
}
