//! すべての文字コードのチャンク単位のデコード (docs/03-core-design.md §8.3)。
//!
//! 不正なバイトは **1 バイトにつき 1 つの私用文字 `U+10FF00 + b`** にエスケープして続行する (U+FFFD への置換はしない)。
//! UTF-8 は `Utf8Decoder`、それ以外は encoding_rs の `decode_to_string_without_replacement` を使う。
//! encoding_rs が返す `Malformed(len, consumed_after)` の不正バイトは直前の入力バッファに属することがあるので、
//! 消費した入力の末尾 `HISTORY` バイトを保持して元のバイト値を回収する。結果はチャンクの分け方に依存しない。

use encoding_rs::DecoderResult;

use crate::encoding::Encoding;
use crate::utf8::{PuaEscapeMode, Utf8Decoder, escape_byte, unescape_char};

/// 不正バイトの回収のために覚えておく、消費済みの入力の長さ (`len + consumed_after` の最大は 6、余裕を見て 16)。
const HISTORY: usize = 16;

/// 文字コードごとのデコーダ。`decode` をチャンクごとに呼び、最後のチャンクで `last = true` にする。
pub struct Decoder {
    enc: Encoding,
    inner: Inner,
}

enum Inner {
    Utf8(Utf8Decoder),
    Rs(Box<RsDecoder>),
}

struct RsDecoder {
    enc: Encoding,
    dec: encoding_rs::Decoder,
    /// 消費済みの入力の末尾。
    history: Vec<u8>,
    /// BOM を読み飛ばす (UTF-16)。
    skip_bom: bool,
    at_start: bool,
    start_carry: Vec<u8>,
    has_bom: bool,
    invalid_bytes: u32,
    collision: bool,
}

impl Decoder {
    /// `skip_bom`: 先頭の BOM (その文字コードの BOM と一致するもの) を読み飛ばして `has_bom` に記録する。
    /// Shift_JIS・EUC-JP・ISO-2022-JP には BOM がないので無視される。
    pub fn new(enc: Encoding, skip_bom: bool) -> Decoder {
        let inner = match enc {
            Encoding::Utf8 => Inner::Utf8(Utf8Decoder::new(skip_bom)),
            _ => Inner::Rs(Box::new(RsDecoder {
                enc,
                dec: enc.rs().new_decoder_without_bom_handling(),
                history: Vec::with_capacity(HISTORY * 2),
                skip_bom: skip_bom && enc.is_utf16(),
                at_start: true,
                start_carry: Vec::new(),
                has_bom: false,
                invalid_bytes: 0,
                collision: false,
            })),
        };
        Decoder { enc, inner }
    }

    pub fn encoding(&self) -> Encoding {
        self.enc
    }

    /// 先頭に BOM があり、読み飛ばしたか。
    pub fn has_bom(&self) -> bool {
        match &self.inner {
            Inner::Utf8(d) => d.has_bom(),
            Inner::Rs(d) => d.has_bom,
        }
    }

    /// エスケープした不正バイトの数。
    pub fn invalid_bytes(&self) -> u32 {
        match &self.inner {
            Inner::Utf8(d) => d.invalid_bytes(),
            Inner::Rs(d) => d.invalid_bytes,
        }
    }

    /// 元のテキストが真に `U+10FF00..=U+10FFFF` を含んだか。
    pub fn pua_collision(&self) -> bool {
        match &self.inner {
            Inner::Utf8(d) => d.pua_collision(),
            Inner::Rs(d) => d.collision,
        }
    }

    /// 衝突と不正バイトの両方がある (読み取り専用に固定して警告する、03 §8.3 の「衝突」(b))。
    pub fn collision_with_invalid(&self) -> bool {
        self.pua_collision() && self.invalid_bytes() > 0
    }

    /// 読み終えた後のエスケープの扱い。状態付き符号化は常に `DisabledStateful` (§8.3)。
    pub fn pua_mode(&self) -> PuaEscapeMode {
        if self.enc.is_stateful() {
            PuaEscapeMode::DisabledStateful
        } else if self.pua_collision() {
            PuaEscapeMode::DisabledCollision
        } else {
            PuaEscapeMode::Enabled
        }
    }

    /// `chunk` を復号して `out` に追記する。`last` が真なら持ち越しを残さず、切れた末尾も不正バイトとして出す。
    pub fn decode(&mut self, chunk: &[u8], last: bool, out: &mut String) {
        match &mut self.inner {
            Inner::Utf8(d) => d.decode(chunk, last, out),
            Inner::Rs(d) => d.decode(chunk, last, out),
        }
    }
}

impl RsDecoder {
    fn decode(&mut self, chunk: &[u8], last: bool, out: &mut String) {
        if self.at_start {
            // UTF-16 の BOM の判定には先頭 2 バイトが要る
            self.start_carry.extend_from_slice(chunk);
            if self.skip_bom && self.start_carry.len() < 2 && !last {
                return;
            }
            self.at_start = false;
            let mut data = std::mem::take(&mut self.start_carry);
            if self.skip_bom && data.starts_with(self.enc.bom().unwrap_or(b"\0\0\0")) {
                self.has_bom = true;
                data.drain(..2);
            }
            self.decode_slice(&data, last, out);
        } else {
            self.decode_slice(chunk, last, out);
        }
    }

    fn decode_slice(&mut self, src: &[u8], last: bool, out: &mut String) {
        let mut pos = 0;
        loop {
            let rest = &src[pos..];
            let want = self
                .dec
                .max_utf8_buffer_length_without_replacement(rest.len())
                .unwrap_or(rest.len() * 4 + 16);
            out.reserve(want);
            let before = out.len();
            let (res, read) = self
                .dec
                .decode_to_string_without_replacement(rest, out, last);
            self.scan_collision(&out[before..]);
            self.remember(&rest[..read]);
            pos += read;
            match res {
                DecoderResult::InputEmpty => return,
                DecoderResult::OutputFull => {}
                DecoderResult::Malformed(len, after) => {
                    // 不正バイトは、消費した入力の末尾から `after` バイトを除いた直前の `len` バイト
                    let end = self.history.len().saturating_sub(usize::from(after));
                    let start = end.saturating_sub(usize::from(len));
                    let bad: Vec<u8> = self.history[start..end].to_vec();
                    for b in bad {
                        self.invalid_bytes = self.invalid_bytes.saturating_add(1);
                        out.push(escape_byte(b));
                    }
                }
            }
        }
    }

    fn remember(&mut self, consumed: &[u8]) {
        self.history.extend_from_slice(consumed);
        if self.history.len() > HISTORY * 2 {
            let cut = self.history.len() - HISTORY;
            self.history.drain(..cut);
        }
    }

    fn scan_collision(&mut self, s: &str) {
        // 0xF4 は U+10FF00 を含む U+100000.. の先頭バイト。無ければ検査を省く
        if !self.collision && s.as_bytes().contains(&0xF4) {
            self.collision = s.chars().any(|c| unescape_char(c).is_some());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_all(enc: Encoding, bytes: &[u8], chunk: usize) -> (String, u32, bool) {
        let mut d = Decoder::new(enc, true);
        let mut out = String::new();
        let mut it = bytes.chunks(chunk.max(1)).peekable();
        if it.peek().is_none() {
            d.decode(&[], true, &mut out);
        }
        while let Some(c) = it.next() {
            d.decode(c, it.peek().is_none(), &mut out);
        }
        (out, d.invalid_bytes(), d.has_bom())
    }

    #[test]
    fn shift_jis_を復号する() {
        let (s, bad, bom) = decode_all(Encoding::ShiftJis, b"\x93\xFA\x96\x7B\x8C\xEAabc", 100);
        assert_eq!((s.as_str(), bad, bom), ("日本語abc", 0, false));
    }

    #[test]
    fn euc_jp_と_iso_2022_jp_を復号する() {
        let (s, bad, _) = decode_all(Encoding::EucJp, b"\xC6\xFC\xCB\xDC", 100);
        assert_eq!((s.as_str(), bad), ("日本", 0));
        let (s, bad, _) = decode_all(Encoding::Iso2022Jp, b"\x1B$BF|K\\\x1B(Babc\n", 100);
        assert_eq!((s.as_str(), bad), ("日本abc\n", 0));
    }

    #[test]
    fn utf16_の_bom_を読み飛ばす() {
        let (s, bad, bom) = decode_all(Encoding::Utf16Le, b"\xFF\xFEa\x00\x42\x30", 1);
        assert_eq!((s.as_str(), bad, bom), ("aあ", 0, true));
        let (s, _, bom) = decode_all(Encoding::Utf16Be, b"\xFE\xFF\x00a\x30\x42", 3);
        assert_eq!((s.as_str(), bom), ("aあ", true));
        // 別の文字コードの BOM は本文のバイト
        let (s, _, bom) = decode_all(Encoding::Utf16Be, b"\xFF\xFE\x00a", 100);
        assert!(!bom);
        assert!(s.contains('a'));
    }

    #[test]
    fn 不正なバイトは_1_バイトずつエスケープする() {
        // Shift_JIS: 先頭バイトだけ / 0xA0 / 0xFD
        let (s, bad, _) = decode_all(Encoding::ShiftJis, b"a\x81\x20b\xA0\xFD", 100);
        let want = format!(
            "a{}\u{20}b{}{}",
            escape_byte(0x81),
            escape_byte(0xA0),
            escape_byte(0xFD)
        );
        assert_eq!((s, bad), (want, 3));
        // UTF-16LE: 孤立した上位サロゲートと奇数長の末尾 1 バイト
        let (s, bad, _) = decode_all(Encoding::Utf16Le, b"\x00\xD8a\x00b", 100);
        let want = format!(
            "{}{}a{}",
            escape_byte(0x00),
            escape_byte(0xD8),
            escape_byte(b'b')
        );
        assert_eq!((s, bad), (want, 3));
    }

    #[test]
    fn iso_2022_jp_の_jis_状態の改行は不正バイトになる() {
        // 03 §8.3: `ESC $ B あ LF ESC ( B` → あ + 改行が不正バイト
        let (s, bad, _) = decode_all(Encoding::Iso2022Jp, b"\x1B$B$\"\n\x1B(B", 100);
        assert_eq!(s, format!("あ{}", escape_byte(0x0A)));
        assert_eq!(bad, 1);
    }

    #[test]
    fn 状態付きは_pua_mode_が_disabled_stateful() {
        let mut d = Decoder::new(Encoding::Iso2022Jp, true);
        let mut out = String::new();
        d.decode(b"abc", true, &mut out);
        assert_eq!(d.pua_mode(), PuaEscapeMode::DisabledStateful);
        let mut d = Decoder::new(Encoding::ShiftJis, true);
        d.decode(b"abc", true, &mut out);
        assert_eq!(d.pua_mode(), PuaEscapeMode::Enabled);
    }

    #[test]
    fn 真に_pua_を含むと衝突() {
        let mut d = Decoder::new(Encoding::Utf16Le, false);
        let mut out = String::new();
        // U+10FF9A = DBFF DF9A
        d.decode(&[0xFF, 0xDB, 0x9A, 0xDF], true, &mut out);
        assert!(d.pua_collision());
        assert_eq!(d.pua_mode(), PuaEscapeMode::DisabledCollision);
    }
}
