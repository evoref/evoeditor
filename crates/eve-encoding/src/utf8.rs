//! UTF-8 のチャンク単位のデコード / エンコード (docs/03-core-design.md §8.3 / §8.6)。
//!
//! - デコード: 不正なバイトは **1 バイトにつき 1 つの私用文字 `U+10FF00 + b`** にエスケープして続行する
//!   (U+FFFD への置換はしない)。チャンクの境目で切れた多バイト列は次のチャンクへ持ち越す。
//!   結果はチャンクの分け方に依存しない (1 バイト単位に分けても同じ)。
//! - エンコード: エスケープ文字を元のバイトに戻して書く。これにより無編集の保存でバイト列が一致する。
//! - 元のテキストが真に `U+10FF00..=U+10FFFF` を含むときは衝突として検出する (`PuaEscapeMode`)。

/// エスケープ先の先頭 (`U+10FF00`)。`b` のバイトは `ESCAPE_BASE + b`。
pub const ESCAPE_BASE: u32 = 0x10FF00;

/// 不正バイト `b` のエスケープ文字。
pub fn escape_byte(b: u8) -> char {
    // `ESCAPE_BASE + 0..=255` は補助私用面 B の中で、常に妥当な char
    char::from_u32(ESCAPE_BASE + u32::from(b)).expect("U+10FF00..=U+10FFFF は妥当な char")
}

/// エスケープ文字なら元のバイト。
pub fn unescape_char(c: char) -> Option<u8> {
    let v = c as u32;
    (ESCAPE_BASE..=ESCAPE_BASE + 0xFF)
        .contains(&v)
        .then(|| (v - ESCAPE_BASE) as u8)
}

/// 不正バイトのエスケープの扱い (03 §8.1 / §8.3)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PuaEscapeMode {
    /// エスケープ文字を保存時に元のバイトへ戻す。
    #[default]
    Enabled,
    /// 元のテキストが真にこの範囲の文字を含む。不正バイトが 0 ならエスケープを使わない
    /// (この範囲の文字をそのまま扱う)。不正バイトもあるときは `Decoder::collision_with_invalid` が真で、読み取り専用に固定する。
    DisabledCollision,
}

/// UTF-8 のデコーダ。`decode` をチャンクごとに呼び、最後のチャンクで `last = true` にする。
#[derive(Debug)]
pub struct Utf8Decoder {
    /// 前のチャンクの末尾で切れた多バイト列 (最大 3 バイト)。先頭では BOM の途中も入る。
    carry: [u8; 3],
    carry_len: usize,
    /// まだ最初の 3 バイトを見ていない (BOM の判定待ち)。
    at_start: bool,
    skip_bom: bool,
    has_bom: bool,
    invalid_bytes: u32,
    collision: bool,
}

impl Utf8Decoder {
    /// `skip_bom`: 先頭の BOM (EF BB BF) を読み飛ばして `has_bom` に記録する。
    pub fn new(skip_bom: bool) -> Utf8Decoder {
        Utf8Decoder {
            carry: [0; 3],
            carry_len: 0,
            at_start: true,
            skip_bom,
            has_bom: false,
            invalid_bytes: 0,
            collision: false,
        }
    }

    /// 先頭に BOM があり、読み飛ばしたか。
    pub fn has_bom(&self) -> bool {
        self.has_bom
    }

    /// エスケープした不正バイトの数。
    pub fn invalid_bytes(&self) -> u32 {
        self.invalid_bytes
    }

    /// 元のテキストが真に `U+10FF00..=U+10FFFF` を含んだか。
    pub fn pua_collision(&self) -> bool {
        self.collision
    }

    /// 衝突と不正バイトの両方がある (読み取り専用に固定して警告する、03 §8.3 の「衝突」(b))。
    pub fn collision_with_invalid(&self) -> bool {
        self.collision && self.invalid_bytes > 0
    }

    /// 読み終えた後のエスケープの扱い。
    pub fn pua_mode(&self) -> PuaEscapeMode {
        if self.collision {
            PuaEscapeMode::DisabledCollision
        } else {
            PuaEscapeMode::Enabled
        }
    }

    /// `chunk` を復号して `out` に追記する。`last` が真なら持ち越しを残さず、切れた末尾も不正バイトとして出す。
    pub fn decode(&mut self, chunk: &[u8], last: bool, out: &mut String) {
        if self.carry_len == 0 && !self.at_start {
            self.decode_slice(chunk, last, out);
            return;
        }
        // 持ち越し (最大 3 バイト) と先頭の BOM の判定を、結合した小さな先頭部分で処理する
        let mut joined = Vec::with_capacity(self.carry_len + chunk.len());
        joined.extend_from_slice(&self.carry[..self.carry_len]);
        joined.extend_from_slice(chunk);
        self.carry_len = 0;
        let mut data: &[u8] = &joined;
        if self.at_start {
            if data.len() < 3 && !last && b"\xEF\xBB\xBF".starts_with(data) {
                // BOM かもしれない先頭が切れている: 次を待つ
                self.carry[..data.len()].copy_from_slice(data);
                self.carry_len = data.len();
                return;
            }
            self.at_start = false;
            if self.skip_bom && data.starts_with(b"\xEF\xBB\xBF") {
                self.has_bom = true;
                data = &data[3..];
            }
        }
        self.decode_slice(data, last, out);
    }

    fn decode_slice(&mut self, mut data: &[u8], last: bool, out: &mut String) {
        out.reserve(data.len());
        loop {
            match std::str::from_utf8(data) {
                Ok(s) => {
                    self.push_valid(s, out);
                    return;
                }
                Err(e) => {
                    let (valid, rest) = data.split_at(e.valid_up_to());
                    // `valid_up_to` までは妥当な UTF-8
                    self.push_valid(
                        std::str::from_utf8(valid).expect("valid_up_to までは妥当"),
                        out,
                    );
                    match e.error_len() {
                        Some(n) => {
                            for &b in &rest[..n] {
                                self.push_escaped(b, out);
                            }
                            data = &rest[n..];
                        }
                        None if !last => {
                            // 末尾で切れた多バイト列 (最大 3 バイト): 次のチャンクへ
                            self.carry[..rest.len()].copy_from_slice(rest);
                            self.carry_len = rest.len();
                            return;
                        }
                        None => {
                            for &b in rest {
                                self.push_escaped(b, out);
                            }
                            return;
                        }
                    }
                }
            }
        }
    }

    fn push_valid(&mut self, s: &str, out: &mut String) {
        // 0xF4 は U+10FF00 を含む U+100000.. の先頭バイト。無ければ衝突の検査を省く
        if !self.collision && s.as_bytes().contains(&0xF4) {
            self.collision = s.chars().any(|c| unescape_char(c).is_some());
        }
        out.push_str(s);
    }

    fn push_escaped(&mut self, b: u8, out: &mut String) {
        self.invalid_bytes = self.invalid_bytes.saturating_add(1);
        out.push(escape_byte(b));
    }
}

/// UTF-8 のエンコーダ。最初の `encode` で BOM を書く (`bom` が真のとき)。
#[derive(Debug)]
pub struct Utf8Encoder {
    pending_bom: bool,
    mode: PuaEscapeMode,
}

impl Utf8Encoder {
    pub fn new(bom: bool, mode: PuaEscapeMode) -> Utf8Encoder {
        Utf8Encoder {
            pending_bom: bom,
            mode,
        }
    }

    /// `text` を符号化して `out` に追記する。`Enabled` ではエスケープ文字を元のバイトに戻す。
    /// UTF-8 には表現不能な文字はない。
    pub fn encode(&mut self, text: &str, out: &mut Vec<u8>) {
        if std::mem::take(&mut self.pending_bom) {
            out.extend_from_slice(b"\xEF\xBB\xBF");
        }
        if self.mode != PuaEscapeMode::Enabled || !text.as_bytes().contains(&0xF4) {
            out.extend_from_slice(text.as_bytes());
            return;
        }
        let mut start = 0;
        for (i, c) in text.char_indices() {
            if let Some(b) = unescape_char(c) {
                out.extend_from_slice(&text.as_bytes()[start..i]);
                out.push(b);
                start = i + c.len_utf8();
            }
        }
        out.extend_from_slice(&text.as_bytes()[start..]);
    }

    /// 符号化の結果の長さ (BOM を含む)。乾式走査 (03 §9.3) の出力長に使う。
    pub fn encoded_len(&self, text: &str) -> usize {
        let bom = if self.pending_bom { 3 } else { 0 };
        if self.mode != PuaEscapeMode::Enabled {
            return bom + text.len();
        }
        bom + text
            .chars()
            .map(|c| {
                if unescape_char(c).is_some() {
                    1
                } else {
                    c.len_utf8()
                }
            })
            .sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_all(chunks: &[&[u8]], skip_bom: bool) -> (String, Utf8Decoder) {
        let mut d = Utf8Decoder::new(skip_bom);
        let mut out = String::new();
        for (i, c) in chunks.iter().enumerate() {
            d.decode(c, i + 1 == chunks.len(), &mut out);
        }
        if chunks.is_empty() {
            d.decode(&[], true, &mut out);
        }
        (out, d)
    }

    fn encode_all(text: &str, bom: bool, mode: PuaEscapeMode) -> Vec<u8> {
        let mut e = Utf8Encoder::new(bom, mode);
        let mut out = Vec::new();
        e.encode(text, &mut out);
        out
    }

    #[test]
    fn 妥当な_utf8_はそのまま() {
        let (s, d) = decode_all(&["あい abc 😀".as_bytes()], true);
        assert_eq!(s, "あい abc 😀");
        assert_eq!(
            (d.invalid_bytes(), d.has_bom(), d.pua_collision()),
            (0, false, false)
        );
    }

    #[test]
    fn bom_は読み飛ばして記録する() {
        let (s, d) = decode_all(&[b"\xEF\xBB\xBFabc"], true);
        assert_eq!((s.as_str(), d.has_bom()), ("abc", true));
        // 読み飛ばさない指定: 本文の U+FEFF として残す
        let (s, d) = decode_all(&[b"\xEF\xBB\xBFabc"], false);
        assert_eq!((s.as_str(), d.has_bom()), ("\u{FEFF}abc", false));
        // 途中の BOM は本文
        let (s, d) = decode_all(&[b"a\xEF\xBB\xBF"], true);
        assert_eq!((s.as_str(), d.has_bom()), ("a\u{FEFF}", false));
    }

    #[test]
    fn 不正バイトは_1_バイトずつエスケープする() {
        let (s, d) = decode_all(&[b"a\x9Ab\xFF"], true);
        assert_eq!(s, format!("a{}b{}", escape_byte(0x9A), escape_byte(0xFF)));
        assert_eq!(d.invalid_bytes(), 2);
        // 途中で切れた多バイト列 (E3 81) の後に別の文字: 2 バイトとも不正
        let (s, d) = decode_all(&[b"\xE3\x81a"], true);
        assert_eq!(s, format!("{}{}a", escape_byte(0xE3), escape_byte(0x81)));
        assert_eq!(d.invalid_bytes(), 2);
        // 末尾で切れた列は last で不正バイトになる
        let (s, d) = decode_all(&[b"a\xE3\x81"], true);
        assert_eq!(s, format!("a{}{}", escape_byte(0xE3), escape_byte(0x81)));
        assert_eq!(d.invalid_bytes(), 2);
    }

    #[test]
    fn チャンクの境目をまたぐ文字と不正バイト() {
        let bytes = "aあ😀b".as_bytes();
        for cut in 0..=bytes.len() {
            let (s, _) = decode_all(&[&bytes[..cut], &bytes[cut..]], true);
            assert_eq!(s, "aあ😀b", "cut = {cut}");
        }
        // 先頭の BOM が切れる
        let bom = b"\xEF\xBB\xBFx";
        for cut in 0..=bom.len() {
            let (s, d) = decode_all(&[&bom[..cut], &bom[cut..]], true);
            assert_eq!((s.as_str(), d.has_bom()), ("x", true), "cut = {cut}");
        }
    }

    #[test]
    fn 空の入力() {
        let (s, d) = decode_all(&[], true);
        assert_eq!((s.as_str(), d.invalid_bytes()), ("", 0));
        let (s, _) = decode_all(&[b"\xEF"], true);
        assert_eq!(
            s,
            escape_byte(0xEF).to_string(),
            "BOM の途中で終わると不正バイト"
        );
    }

    #[test]
    fn 無編集の往復はバイト一致() {
        let src: &[u8] = b"ok \xE3\x81 \x9A\xFF \xF0\x9F\x98 end\xC3";
        let (s, d) = decode_all(&[src], true);
        assert!(d.invalid_bytes() > 0);
        assert_eq!(encode_all(&s, false, d.pua_mode()), src);
    }

    #[test]
    fn 真の私用文字は衝突として検出する() {
        let real = "x\u{10FF9A}y";
        let (s, d) = decode_all(&[real.as_bytes()], true);
        assert_eq!(s, real);
        assert!(d.pua_collision() && !d.collision_with_invalid());
        assert_eq!(d.pua_mode(), PuaEscapeMode::DisabledCollision);
        // 無効化した保存では、そのまま 4 バイトで書く
        assert_eq!(encode_all(&s, false, d.pua_mode()), real.as_bytes());
        // 衝突と不正バイトの両方
        let (_, d) = decode_all(&[real.as_bytes(), b"\x80"], true);
        assert!(d.collision_with_invalid());
    }

    #[test]
    fn エンコードの_bom_と長さ() {
        let mut e = Utf8Encoder::new(true, PuaEscapeMode::Enabled);
        let text = format!("あ{}", escape_byte(0x9A));
        assert_eq!(e.encoded_len(&text), 3 + 3 + 1);
        let mut out = Vec::new();
        e.encode(&text, &mut out);
        e.encode("b", &mut out);
        assert_eq!(out, b"\xEF\xBB\xBF\xE3\x81\x82\x9Ab", "BOM は最初だけ");
        assert_eq!(e.encoded_len("b"), 1, "BOM を書いた後は含めない");
    }
}
