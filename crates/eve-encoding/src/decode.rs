//! すべての文字コードのチャンク単位のデコード (docs/03-core-design.md §8.3)。
//!
//! 不正なバイトは **1 バイトにつき 1 つの私用文字 `U+10FF00 + b`** にエスケープして続行する (U+FFFD への置換はしない)。
//! UTF-8 は `Utf8Decoder`、それ以外は `encoding_rs` の `decode_to_string_without_replacement` を使う。
//! `encoding_rs` が返す `Malformed(len, consumed_after)` の不正バイトは直前の入力バッファに属することがあるので、
//! 消費した入力の末尾 `HISTORY` バイトを保持して元のバイト値を回収する。結果はチャンクの分け方に依存しない。

use encoding_rs::DecoderResult;

use crate::encoding::Encoding;
use crate::utf8::{PuaEscapeMode, Utf8Decoder, escape_byte, unescape_char};

/// 1 回の `decode_to_string_without_replacement` に最低限用意する出力の空き (バイト)。
const MIN_SPARE: usize = 16;
/// 空きを倍にしてよい上限。これを超えても `OutputFull` なら何かがおかしいので、ループを打ち切る。
const MAX_SPARE: usize = 1 << 26;

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
    /// 復号器が消費した入力のバイト数 (BOM を除く)。UTF-16 の持ち越しの判定 (`is_neutral`) に使う。
    consumed: u64,
    /// `last = true` で呼んだ (`encoding_rs` の復号器は、その後に使えない)。
    finished: bool,
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
                consumed: 0,
                finished: false,
            })),
        };
        Decoder { enc, inner }
    }

    /// 初期状態に戻す (BOM は読み飛ばさない。`Decoder::new(enc, false)` と同じ状態)。作業の領域は使い回す
    /// (つながりの局所の検査と区切った確認 (ii) が、並びごとに復号器を作り直さない。03 §8.3.1)。
    pub fn reset(&mut self) {
        match &mut self.inner {
            Inner::Utf8(d) => *d = Utf8Decoder::new(false),
            Inner::Rs(d) => {
                d.dec = d.enc.rs().new_decoder_without_bom_handling();
                d.history.clear();
                d.skip_bom = false;
                d.at_start = true;
                d.start_carry.clear();
                d.has_bom = false;
                d.invalid_bytes = 0;
                d.collision = false;
                d.consumed = 0;
                d.finished = false;
            }
        }
    }

    /// 持ち越し (途中で切れた多バイト列・UTF-16 の奇数のバイトと対になる前の上位サロゲート) が無いか。持ち越しが無い
    /// 状態は初期状態と同じに振る舞う (WHATWG の復号器は先読みをしない)。`last = true` で呼んだ後は真。
    /// つながりの変わる範囲 (03 §8.3.1 の「示す位置と変わる範囲」) が、復号の側と符号化の側の境界が再び一致したかを見るのに使う。
    pub fn is_neutral(&self) -> bool {
        match &self.inner {
            Inner::Utf8(d) => d.is_neutral(),
            Inner::Rs(d) => d.is_neutral(),
        }
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
    fn is_neutral(&self) -> bool {
        if self.finished {
            return true;
        }
        if !self.start_carry.is_empty() {
            return false;
        }
        if self.enc.is_utf16() {
            // encoding_rs は UTF-16 の復号器の状態を見せない。消費したバイトの数が偶数で、最後の 16 ビットの単位が上位
            // サロゲート (D800..DBFF) でなければ持ち越しは無い (上位サロゲートは、どの経路で読んでも次の単位を待つ)
            if !self.consumed.is_multiple_of(2) {
                return false;
            }
            let n = self.history.len();
            if self.consumed == 0 || n < 2 {
                return true;
            }
            let (a, b) = (self.history[n - 2], self.history[n - 1]);
            let unit = if self.enc == Encoding::Utf16Le {
                u16::from_le_bytes([a, b])
            } else {
                u16::from_be_bytes([a, b])
            };
            return !(0xD800..=0xDBFF).contains(&unit);
        }
        // Shift_JIS・EUC-JP・ISO-2022-JP: encoding_rs は、持ち越しの無い状態でだけ `Some` を返す (状態を変えない)
        self.dec.latin1_byte_compatible_up_to(&[]).is_some()
    }

    fn decode(&mut self, chunk: &[u8], last: bool, out: &mut String) {
        self.decode_inner(chunk, last, out);
        // 呼び終えた後に is_neutral が encoding_rs の終えた復号器を問い合わせない (問い合わせるとパニックする)
        self.finished |= last;
    }

    fn decode_inner(&mut self, chunk: &[u8], last: bool, out: &mut String) {
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
        // `OutputFull` が続くとき (入力を消費せず出力も増えない) に空きを倍にして、必ず前へ進める。
        // `max_utf8_buffer_length_without_replacement` は、入力の末尾が UTF-16 の孤立したサロゲートで終わる状態など、
        // 内部に持ち越した状態の分を足りなく返すことがある (入力が空でも `OutputFull` になり、空きが 1 バイトのままだと無限ループになる)
        let mut extra = 0usize;
        loop {
            let rest = &src[pos..];
            let want = self
                .dec
                .max_utf8_buffer_length_without_replacement(rest.len())
                .unwrap_or(rest.len() * 4 + 16)
                .max(MIN_SPARE)
                .max(extra);
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
                DecoderResult::OutputFull => {
                    extra = want.saturating_mul(2);
                    // 上限を超えて足りないことは無い (エンコーディングの仕様上、1 文字は 4 バイトまで)。無限ループにしない
                    if extra > MAX_SPARE {
                        return;
                    }
                }
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
        self.consumed = self.consumed.wrapping_add(consumed.len() as u64);
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

    // fuzz_decode が見つけた timeout (2026-10-22 の夜間、課題 #78)。UTF-16LE で 2 バイトずつ渡し、孤立した上位サロゲートが続くと、
    // 入力が空の呼出しで `OutputFull` を返し続けて無限ループになっていた。
    #[test]
    fn 孤立したサロゲートが続く_utf16_を_2_バイトずつ渡しても終わる() {
        let body: [u8; 38] = [
            0xE5, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x25, 0xFF, 0xD8, 0xD8, 0xD8, 0x00, 0xD8,
            0x00, 0x00, 0x00, 0xD8, 0x21, 0x00, 0x3B, 0x24, 0x1A, 0xD8, 0xD8, 0x1B, 0x00, 0x00,
            0xD8, 0xD8, 0xD8, 0xD8, 0xD8, 0xD8, 0x21, 0x00, 0x3B, 0x24,
        ];
        for chunk in [1usize, 2, 3, 5, 38] {
            for cap in [0usize, 1, 64] {
                let mut d = Decoder::new(Encoding::Utf16Le, false);
                let mut out = String::with_capacity(cap);
                let mut it = body.chunks(chunk).peekable();
                while let Some(c) = it.next() {
                    d.decode(c, it.peek().is_none(), &mut out);
                }
                assert!(
                    d.invalid_bytes() > 0,
                    "孤立したサロゲートは不正バイトに数える"
                );
            }
        }
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
