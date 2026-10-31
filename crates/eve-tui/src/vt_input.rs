//! VT (xterm 形式) の入力の解析 (docs/04 §6.1〜§6.9、Unix の入力経路。v0.2 の M7)。端末から読んだバイト列を `InputEvent` と
//! マウスの記録 (`MouseRec`、Windows と同じ形にして `MouseTracker` に渡す) にする。OS を呼ばない純粋な状態機械で、
//! 1 回の読取 (待ちなしで読める分) ごとに `feed` を呼ぶ。
//!
//! - 印字可能文字 (UTF-8) は `Text` にまとめる (最大 64 KiB、§6.1 の合体)。制御バイトは §6.2 の表のキーにする。
//! - Alt は `ESC` 前置。**`ESC` + 文字が同じ読取に来たときだけ** Alt とみなし、読取の末尾の単独の `ESC` は `Esc` にする (§6.5)。
//! - CSI / SS3 のキー (矢印・Home/End・PageUp/Down・Insert/Delete・F1〜F12、修飾の引数 `;m`)、CSI u (kitty の
//!   DISAMBIGUATE)、SGR マウス (`CSI < b ; x ; y M/m`)、フォーカス (`CSI I` / `CSI O`)、ブラケットペースト (`CSI 200~` 〜 `CSI 201~`)。
//! - ブラケットペーストの本文は読取をまたいで溜め、終わりで `Paste` にする。終了の直後の同じ読取の入力は §6.6 の
//!   「バースト規則」で本文として扱う (制御文字・Ctrl 付きキーをコマンドにしない。矢印などの文字にならないキーは捨てる)。
//! - 読取の末尾で途切れた CSI / SS3 / UTF-8 は次の読取へ持ち越す。持ち越したまま入力が途絶えたら `flush` で捨てる。
//! - 未知のシーケンスは捨てる。

use eve_view::{InputEvent, KeyCode, KeyEvent, Modifiers};

use crate::input::MAX_TEXT_BYTES;
use crate::mouse::{MouseRec, button_bits};

/// 解析の結果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VtOut {
    Event(InputEvent),
    /// SGR マウスの記録 (座標は端末の左上を原点とする 0 始まりのセル)。
    Mouse(MouseRec),
}

const ESC: u8 = 0x1B;
const PASTE_END: &[u8] = b"\x1b[201~";

/// VT の入力の解析器。
#[derive(Debug, Default)]
pub struct VtParser {
    /// 前の読取から持ち越した、途切れたシーケンス・UTF-8。
    pending: Vec<u8>,
    /// ブラケットペーストの本文 (受信中なら `Some`)。
    paste: Option<Vec<u8>>,
    /// SGR マウスで押されているボタン (`MouseRec.buttons` のビット)。
    buttons: u32,
}

/// 1 つの単位を解析した結果。
enum Unit {
    /// `n` バイトを消費して何も出さない (未知・捨てる)。
    Skip(usize),
    Key(usize, KeyCode, Modifiers),
    /// 印字可能文字 (UTF-8 の 1 文字)。
    Char(usize, char),
    Mouse(usize, MouseRec),
    Focus(usize, bool),
    PasteStart(usize),
    /// 途切れている (次の読取を待つ)。
    Incomplete,
}

fn key(code: KeyCode, mods: Modifiers) -> InputEvent {
    InputEvent::Key(KeyEvent::press(code, mods))
}

/// CSI の修飾の引数 (`1 + ビット`。1 = Shift、2 = Alt、4 = Ctrl、8 = Super)。
fn mods_param(p: Option<u32>) -> Modifiers {
    let bits = p.unwrap_or(1).saturating_sub(1);
    let mut m = Modifiers::NONE;
    for (b, f) in [
        (1, Modifiers::SHIFT),
        (2, Modifiers::ALT),
        (4, Modifiers::CTRL),
        (8, Modifiers::SUPER),
    ] {
        if bits & b != 0 {
            m = m | f;
        }
    }
    m
}

/// 文字のキーの正準形: 英字は小文字 + SHIFT (§6.2)。
fn char_key(c: char, mods: Modifiers) -> (KeyCode, Modifiers) {
    if c.is_ascii_uppercase() {
        (
            KeyCode::Char(c.to_ascii_lowercase()),
            mods | Modifiers::SHIFT,
        )
    } else {
        (KeyCode::Char(c), mods)
    }
}

/// 制御バイト 1 つのキー (§6.2 の表)。
fn control_key(b: u8) -> (KeyCode, Modifiers) {
    match b {
        0x0D => (KeyCode::Enter, Modifiers::NONE),
        0x09 => (KeyCode::Tab, Modifiers::NONE),
        0x7F => (KeyCode::Backspace, Modifiers::NONE),
        0x08 => (KeyCode::Backspace, Modifiers::CTRL),
        0x1B => (KeyCode::Esc, Modifiers::NONE),
        0x00 => (KeyCode::Char(' '), Modifiers::CTRL),
        0x1C => (KeyCode::Char('\\'), Modifiers::CTRL),
        0x1D => (KeyCode::Char(']'), Modifiers::CTRL),
        0x1E => (KeyCode::Char('^'), Modifiers::CTRL),
        0x1F => (KeyCode::Char('/'), Modifiers::CTRL),
        // 0x01〜0x1A: Ctrl + 英字 (0x0A は Ctrl+J。raw モードの Enter は 0x0D)
        b => (KeyCode::Char(char::from(b'a' + b - 1)), Modifiers::CTRL),
    }
}

/// `~` で終わる CSI の番号のキー。
fn tilde_key(n: u32) -> Option<KeyCode> {
    Some(match n {
        1 | 7 => KeyCode::Home,
        2 => KeyCode::Insert,
        3 => KeyCode::Delete,
        4 | 8 => KeyCode::End,
        5 => KeyCode::PageUp,
        6 => KeyCode::PageDown,
        11..=15 => KeyCode::F((n - 10) as u8),
        17..=21 => KeyCode::F((n - 11) as u8),
        23 | 24 => KeyCode::F((n - 12) as u8),
        _ => return None,
    })
}

/// 文字で終わる CSI / SS3 のキー。
fn final_key(f: u8) -> Option<KeyCode> {
    Some(match f {
        b'A' => KeyCode::Up,
        b'B' => KeyCode::Down,
        b'C' => KeyCode::Right,
        b'D' => KeyCode::Left,
        b'H' => KeyCode::Home,
        b'F' => KeyCode::End,
        b'P' => KeyCode::F(1),
        b'Q' => KeyCode::F(2),
        b'R' => KeyCode::F(3),
        b'S' => KeyCode::F(4),
        _ => return None,
    })
}

/// UTF-8 の 1 文字の長さ (先頭バイトから)。不正なら `None`。
fn utf8_len(b: u8) -> Option<usize> {
    match b {
        0xC2..=0xDF => Some(2),
        0xE0..=0xEF => Some(3),
        0xF0..=0xF4 => Some(4),
        _ => None,
    }
}

/// `;` 区切りの数の引数 (空は `None`)。`:` の副引数は先頭だけを使う。
fn params(p: &[u8]) -> Vec<Option<u32>> {
    p.split(|&b| b == b';')
        .map(|s| {
            let s = s.split(|&b| b == b':').next().unwrap_or(&[]);
            std::str::from_utf8(s).ok().and_then(|t| t.parse().ok())
        })
        .collect()
}

impl VtParser {
    pub fn new() -> VtParser {
        VtParser::default()
    }

    /// ブラケットペーストの受信中か。
    pub fn in_paste(&self) -> bool {
        self.paste.is_some()
    }

    /// 1 回の読取のバイト列を解析する。
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<VtOut> {
        let mut data = std::mem::take(&mut self.pending);
        data.extend_from_slice(bytes);
        let mut out = Vec::new();
        let mut text = String::new();
        let mut i = 0;
        // ブラケットペーストの終了の後ろ (同じ読取の中) か
        let mut after_paste = false;
        while i < data.len() {
            if let Some(body) = self.paste.as_mut() {
                // 本文: 終了のシーケンスまでをそのまま溜める
                match find(&data[i..], PASTE_END) {
                    Some(at) => {
                        body.extend_from_slice(&data[i..i + at]);
                        i += at + PASTE_END.len();
                        let body = self.paste.take().unwrap_or_default();
                        flush_text(&mut text, &mut out);
                        out.push(VtOut::Event(InputEvent::Paste(
                            String::from_utf8_lossy(&body).into_owned(),
                        )));
                        after_paste = true;
                    }
                    None => {
                        // 終了のシーケンスの途中で途切れたかもしれない分は持ち越す
                        let rest = &data[i..];
                        let keep = partial_suffix(rest, PASTE_END);
                        body.extend_from_slice(&rest[..rest.len() - keep]);
                        self.pending = rest[rest.len() - keep..].to_vec();
                        flush_text(&mut text, &mut out);
                        return out;
                    }
                }
                continue;
            }
            let unit = self.unit(&data[i..]);
            match unit {
                Unit::Incomplete => {
                    // 読取の末尾で途切れた。単独の ESC は (同じ読取に続きが無いので) Esc にする
                    if data[i] == ESC && data.len() - i == 1 {
                        self.push_key(
                            KeyCode::Esc,
                            Modifiers::NONE,
                            0x1B,
                            after_paste,
                            &mut text,
                            &mut out,
                        );
                        i += 1;
                        continue;
                    }
                    self.pending = data[i..].to_vec();
                    break;
                }
                Unit::Skip(n) => i += n,
                Unit::Char(n, c) => {
                    push_char(&mut text, &mut out, c);
                    i += n;
                }
                Unit::Key(n, code, mods) => {
                    // 制御バイト 1 つのキーなら、そのバイト (バースト規則で本文にするとき使う)
                    let raw = if n == 1 { data[i] } else { 0 };
                    self.push_key(code, mods, raw, after_paste, &mut text, &mut out);
                    i += n;
                }
                Unit::Mouse(n, rec) => {
                    if !after_paste {
                        flush_text(&mut text, &mut out);
                        out.push(VtOut::Mouse(rec));
                    }
                    i += n;
                }
                Unit::Focus(n, f) => {
                    flush_text(&mut text, &mut out);
                    out.push(VtOut::Event(InputEvent::Focus(f)));
                    i += n;
                }
                Unit::PasteStart(n) => {
                    flush_text(&mut text, &mut out);
                    self.paste = Some(Vec::new());
                    i += n;
                }
            }
        }
        flush_text(&mut text, &mut out);
        out
    }

    /// 入力が途絶えた: 持ち越していた途切れたシーケンスを捨てる (単独の ESC なら Esc)。貼り付けの受信中は何もしない。
    pub fn flush(&mut self) -> Vec<VtOut> {
        if self.paste.is_some() {
            return Vec::new();
        }
        let p = std::mem::take(&mut self.pending);
        if p == [ESC] {
            return vec![VtOut::Event(key(KeyCode::Esc, Modifiers::NONE))];
        }
        Vec::new()
    }

    /// キーを出す。貼り付けの直後 (バースト規則) は、制御バイトを本文に入れ、文字にならないキーは捨てる。
    fn push_key(
        &self,
        code: KeyCode,
        mods: Modifiers,
        raw: u8,
        after_paste: bool,
        text: &mut String,
        out: &mut Vec<VtOut>,
    ) {
        if after_paste {
            if raw != 0 {
                push_char(text, out, char::from(raw));
            } else if let KeyCode::Char(c) = code {
                // Alt + 文字 (ESC 前置) は ESC と文字の 2 つ
                if mods.contains(Modifiers::ALT) {
                    push_char(text, out, '\u{1b}');
                }
                let c = if mods.contains(Modifiers::SHIFT) {
                    c.to_ascii_uppercase()
                } else {
                    c
                };
                push_char(text, out, c);
            }
            return;
        }
        flush_text(text, out);
        out.push(VtOut::Event(key(code, mods)));
    }

    /// `d` の先頭の 1 単位を解析する。
    fn unit(&mut self, d: &[u8]) -> Unit {
        let b = d[0];
        if b == ESC {
            let Some(&n) = d.get(1) else {
                return Unit::Incomplete;
            };
            return match n {
                b'[' => self.csi(d),
                b'O' => match d.get(2) {
                    None => Unit::Incomplete,
                    Some(&f) => match final_key(f) {
                        Some(k) => Unit::Key(3, k, Modifiers::NONE),
                        None => Unit::Skip(3),
                    },
                },
                // ESC ESC: 先の ESC を単独の Esc にする
                ESC => Unit::Key(1, KeyCode::Esc, Modifiers::NONE),
                // ESC + 制御バイト: そのキー + Alt
                0x00..=0x1F | 0x7F => {
                    let (k, m) = control_key(n);
                    Unit::Key(2, k, m | Modifiers::ALT)
                }
                // ESC + 文字: Alt + 文字
                _ => match self.char_at(&d[1..]) {
                    Unit::Char(len, c) => {
                        let (k, m) = char_key(c, Modifiers::ALT);
                        Unit::Key(1 + len, k, m)
                    }
                    Unit::Incomplete => Unit::Incomplete,
                    _ => Unit::Key(1, KeyCode::Esc, Modifiers::NONE),
                },
            };
        }
        if b < 0x20 || b == 0x7F {
            let (k, m) = control_key(b);
            return Unit::Key(1, k, m);
        }
        self.char_at(d)
    }

    /// 印字可能文字 1 つ (UTF-8)。
    fn char_at(&self, d: &[u8]) -> Unit {
        let b = d[0];
        if b < 0x80 {
            return if b >= 0x20 && b != 0x7F {
                Unit::Char(1, char::from(b))
            } else {
                Unit::Skip(1)
            };
        }
        let Some(len) = utf8_len(b) else {
            return Unit::Skip(1);
        };
        if d.len() < len {
            return Unit::Incomplete;
        }
        match std::str::from_utf8(&d[..len])
            .ok()
            .and_then(|s| s.chars().next())
        {
            Some(c) => Unit::Char(len, c),
            None => Unit::Skip(1),
        }
    }

    /// `ESC [` で始まる CSI。
    fn csi(&mut self, d: &[u8]) -> Unit {
        // 引数 (0x30〜0x3F)・中間 (0x20〜0x2F)・終端 (0x40〜0x7E)
        let mut j = 2;
        while j < d.len() && (0x20..=0x3F).contains(&d[j]) {
            j += 1;
        }
        let Some(&fin) = d.get(j) else {
            return Unit::Incomplete;
        };
        let n = j + 1;
        if !(0x40..=0x7E).contains(&fin) {
            return Unit::Skip(n.min(d.len()));
        }
        let body = &d[2..j];
        // SGR マウス: CSI < b ; x ; y M / m
        if let Some(rest) = body.strip_prefix(b"<") {
            return match self.sgr_mouse(rest, fin) {
                Some(r) => Unit::Mouse(n, r),
                None => Unit::Skip(n),
            };
        }
        if body
            .iter()
            .any(|b| !(b.is_ascii_digit() || *b == b';' || *b == b':'))
        {
            return Unit::Skip(n);
        }
        let p = params(body);
        let first = p.first().copied().flatten();
        let mods = mods_param(p.get(1).copied().flatten());
        match fin {
            b'~' => match first {
                Some(200) => Unit::PasteStart(n),
                Some(201) => Unit::Skip(n),
                Some(k) => match tilde_key(k) {
                    Some(code) => Unit::Key(n, code, mods),
                    None => Unit::Skip(n),
                },
                None => Unit::Skip(n),
            },
            b'Z' => Unit::Key(n, KeyCode::Tab, Modifiers::SHIFT | mods),
            b'I' if body.is_empty() => Unit::Focus(n, true),
            b'O' if body.is_empty() => Unit::Focus(n, false),
            // CSI u (kitty keyboard protocol の DISAMBIGUATE_ESCAPE_CODES、§6.3)
            b'u' => match first.and_then(char::from_u32) {
                Some('\r') => Unit::Key(n, KeyCode::Enter, mods),
                Some('\t') => Unit::Key(n, KeyCode::Tab, mods),
                Some('\u{1b}') => Unit::Key(n, KeyCode::Esc, mods),
                Some('\u{7f}') => Unit::Key(n, KeyCode::Backspace, mods),
                Some(c) if !c.is_control() => {
                    let (k, m) = char_key(c, mods);
                    Unit::Key(n, k, m)
                }
                _ => Unit::Skip(n),
            },
            f => match final_key(f) {
                Some(code) => Unit::Key(n, code, mods),
                None => Unit::Skip(n),
            },
        }
    }

    /// SGR マウスを `MouseRec` にする (押されているボタンを覚えて、Windows の記録と同じ「今の状態」にする)。
    fn sgr_mouse(&mut self, body: &[u8], fin: u8) -> Option<MouseRec> {
        if fin != b'M' && fin != b'm' {
            return None;
        }
        let p = params(body);
        let (b, x, y) = (
            p.first()?.as_ref()?,
            p.get(1)?.as_ref()?,
            p.get(2)?.as_ref()?,
        );
        let (b, x, y) = (*b, *x, *y);
        let mut state = 0;
        if b & 4 != 0 {
            state |= button_bits::SHIFT;
        }
        if b & 8 != 0 {
            state |= button_bits::ALT;
        }
        if b & 16 != 0 {
            state |= button_bits::CTRL;
        }
        let (col, row) = (x as i32 - 1, y as i32 - 1);
        let rec = |buttons, flags| MouseRec {
            col,
            row,
            buttons,
            state,
            flags,
        };
        if b & 64 != 0 {
            // ホイール: 64 = 上、65 = 下、66 / 67 = 横
            return Some(match b & 3 {
                0 => rec(self.buttons | (120 << 16), button_bits::WHEELED),
                1 => rec(
                    self.buttons | ((-120i16 as u16 as u32) << 16),
                    button_bits::WHEELED,
                ),
                _ => rec(self.buttons, button_bits::HWHEELED),
            });
        }
        let bit = match b & 3 {
            0 => button_bits::LEFT,
            1 => button_bits::MIDDLE,
            2 => button_bits::RIGHT,
            _ => 0,
        };
        if b & 32 != 0 {
            return Some(rec(self.buttons, button_bits::MOVED));
        }
        if fin == b'M' {
            self.buttons |= bit;
        } else {
            self.buttons &= !bit;
        }
        Some(rec(self.buttons, 0))
    }
}

fn push_char(text: &mut String, out: &mut Vec<VtOut>, c: char) {
    if text.len() + c.len_utf8() > MAX_TEXT_BYTES {
        flush_text(text, out);
    }
    text.push(c);
}

fn flush_text(text: &mut String, out: &mut Vec<VtOut>) {
    if !text.is_empty() {
        out.push(VtOut::Event(InputEvent::Text(std::mem::take(text))));
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// `s` の末尾が `pat` の (完全ではない) 先頭部分と一致する長さ。
fn partial_suffix(s: &[u8], pat: &[u8]) -> usize {
    (1..pat.len().min(s.len() + 1))
        .rev()
        .find(|&k| s.ends_with(&pat[..k]))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eve_view::KeyCode as K;

    fn ev(o: Vec<VtOut>) -> Vec<InputEvent> {
        o.into_iter()
            .map(|x| match x {
                VtOut::Event(e) => e,
                VtOut::Mouse(m) => panic!("{m:?}"),
            })
            .collect()
    }

    fn k(code: KeyCode, mods: Modifiers) -> InputEvent {
        key(code, mods)
    }

    fn text(s: &str) -> InputEvent {
        InputEvent::Text(s.to_owned())
    }

    fn parse(b: &[u8]) -> Vec<InputEvent> {
        ev(VtParser::new().feed(b))
    }

    #[test]
    fn 印字可能文字と日本語は_1_つの_text() {
        assert_eq!(parse("aB日本語".as_bytes()), vec![text("aB日本語")]);
    }

    #[test]
    fn 制御バイトは表のキー() {
        use Modifiers as M;
        assert_eq!(
            parse(b"\r\t\x7f\x08\x00\x13\x1c\x1d\x1f\x0a"),
            vec![
                k(K::Enter, M::NONE),
                k(K::Tab, M::NONE),
                k(K::Backspace, M::NONE),
                k(K::Backspace, M::CTRL),
                k(K::Char(' '), M::CTRL),
                k(K::Char('s'), M::CTRL),
                k(K::Char('\\'), M::CTRL),
                k(K::Char(']'), M::CTRL),
                k(K::Char('/'), M::CTRL),
                k(K::Char('j'), M::CTRL),
            ]
        );
    }

    #[test]
    fn 文字の間のキーで_text_が分かれる() {
        assert_eq!(
            parse(b"ab\x13cd"),
            vec![text("ab"), k(K::Char('s'), Modifiers::CTRL), text("cd")]
        );
    }

    #[test]
    fn 同じ読取の_esc_と文字は_alt_で_末尾の単独の_esc_は_esc() {
        use Modifiers as M;
        assert_eq!(parse(b"\x1bx"), vec![k(K::Char('x'), M::ALT)]);
        assert_eq!(parse(b"\x1bX"), vec![k(K::Char('x'), M::ALT | M::SHIFT)]);
        assert_eq!(parse(b"\x1b\x7f"), vec![k(K::Backspace, M::ALT)]);
        assert_eq!(parse(b"\x1b"), vec![k(K::Esc, M::NONE)]);
        assert_eq!(
            parse(b"\x1b\x1b"),
            vec![k(K::Esc, M::NONE), k(K::Esc, M::NONE)]
        );
        // 読取が分かれた (回線の遅延): Esc と Text になる (§6.5 の既知の制約)
        let mut p = VtParser::new();
        assert_eq!(ev(p.feed(b"\x1b")), vec![k(K::Esc, M::NONE)]);
        assert_eq!(ev(p.feed(b"x")), vec![text("x")]);
    }

    #[test]
    fn csi_と_ss3_のキーと修飾() {
        use Modifiers as M;
        assert_eq!(
            parse(b"\x1b[A\x1b[1;5C\x1b[1;2D\x1bOB\x1b[H\x1b[F\x1b[3~\x1b[5;3~\x1b[2~\x1b[Z"),
            vec![
                k(K::Up, M::NONE),
                k(K::Right, M::CTRL),
                k(K::Left, M::SHIFT),
                k(K::Down, M::NONE),
                k(K::Home, M::NONE),
                k(K::End, M::NONE),
                k(K::Delete, M::NONE),
                k(K::PageUp, M::ALT),
                k(K::Insert, M::NONE),
                k(K::Tab, M::SHIFT),
            ]
        );
        assert_eq!(
            parse(b"\x1bOP\x1b[1;2Q\x1b[15~\x1b[17~\x1b[21~\x1b[24~"),
            vec![
                k(K::F(1), M::NONE),
                k(K::F(2), M::SHIFT),
                k(K::F(5), M::NONE),
                k(K::F(6), M::NONE),
                k(K::F(10), M::NONE),
                k(K::F(12), M::NONE),
            ]
        );
    }

    #[test]
    fn csi_u_は区別困難なキーを区別する() {
        use Modifiers as M;
        assert_eq!(
            parse(b"\x1b[105;5u\x1b[13u\x1b[27u\x1b[115;6u"),
            vec![
                k(K::Char('i'), M::CTRL),
                k(K::Enter, M::NONE),
                k(K::Esc, M::NONE),
                k(K::Char('s'), M::CTRL | M::SHIFT),
            ]
        );
    }

    #[test]
    fn 未知のシーケンスは捨て_前後の文字は_1_つの_text() {
        assert_eq!(parse(b"a\x1b[?1;2cb\x1b[99~c"), vec![text("abc")]);
    }

    #[test]
    fn 読取の末尾で途切れた_csi_と_utf8_は持ち越す() {
        let mut p = VtParser::new();
        assert!(p.feed(b"\x1b[1;").is_empty());
        assert_eq!(ev(p.feed(b"5A")), vec![k(K::Up, Modifiers::CTRL)]);
        let bytes = "あ".as_bytes();
        assert!(p.feed(&bytes[..1]).is_empty());
        assert_eq!(ev(p.feed(&bytes[1..])), vec![text("あ")]);
        // 途絶えたら捨てる
        assert!(p.feed(b"\x1b[").is_empty());
        assert!(p.flush().is_empty());
        assert_eq!(ev(p.feed(b"z")), vec![text("z")]);
    }

    #[test]
    fn フォーカス() {
        assert_eq!(
            parse(b"\x1b[I\x1b[O"),
            vec![InputEvent::Focus(true), InputEvent::Focus(false)]
        );
    }

    #[test]
    fn ブラケットペーストは読取をまたいで_1_つの_paste() {
        let mut p = VtParser::new();
        assert!(p.feed(b"\x1b[200~line1\r\nli").is_empty());
        assert!(p.in_paste());
        // 終了のシーケンスの途中で途切れる
        assert!(p.feed(b"ne2\x1b[20").is_empty());
        assert_eq!(
            ev(p.feed(b"1~")),
            vec![InputEvent::Paste("line1\r\nline2".to_owned())]
        );
        assert!(!p.in_paste());
        // 本文の ESC やキーの列はそのまま本文
        assert_eq!(
            parse(b"\x1b[200~a\x1b[Ab\x13\x1b[201~"),
            vec![InputEvent::Paste("a\x1b[Ab\x13".to_owned())]
        );
    }

    #[test]
    fn 貼り付けの直後の同じ読取の入力は本文として扱う() {
        // 本文に終了のシーケンスを埋め込んだ攻撃: 後ろの Ctrl+S・Enter・Alt+x はコマンドにしない。矢印は捨てる
        assert_eq!(
            parse(b"\x1b[200~x\x1b[201~\x13\r\x1bxq\x1b[A"),
            vec![InputEvent::Paste("x".to_owned()), text("\x13\r\x1bxq"),]
        );
        // 次の読取からは通常の解釈
        let mut p = VtParser::new();
        let _ = p.feed(b"\x1b[200~x\x1b[201~");
        assert_eq!(ev(p.feed(b"\x13")), vec![k(K::Char('s'), Modifiers::CTRL)]);
    }

    #[test]
    fn sgr_マウスは押下中のボタンを覚えた記録になる() {
        let mut p = VtParser::new();
        let recs: Vec<MouseRec> = p
            .feed(
                b"\x1b[<0;10;5M\x1b[<32;11;5M\x1b[<0;11;5m\x1b[<64;3;4M\x1b[<65;3;4M\x1b[<20;1;1M",
            )
            .into_iter()
            .map(|o| match o {
                VtOut::Mouse(m) => m,
                VtOut::Event(e) => panic!("{e:?}"),
            })
            .collect();
        assert_eq!(recs.len(), 6);
        assert_eq!((recs[0].col, recs[0].row), (9, 4), "0 始まり");
        assert_eq!(recs[0].buttons & button_bits::LEFT, button_bits::LEFT);
        assert_eq!(recs[1].flags, button_bits::MOVED);
        assert_eq!(
            recs[1].buttons & button_bits::LEFT,
            button_bits::LEFT,
            "ドラッグ中"
        );
        assert_eq!(recs[2].buttons & button_bits::LEFT, 0, "解放");
        assert_eq!(recs[3].flags, button_bits::WHEELED);
        assert!(((recs[3].buttons >> 16) as u16 as i16) > 0, "上");
        assert!(((recs[4].buttons >> 16) as u16 as i16) < 0, "下");
        // 20 = 左 + Shift + Ctrl
        assert_eq!(recs[5].state & button_bits::SHIFT, button_bits::SHIFT);
        assert_ne!(recs[5].state & button_bits::CTRL, 0);
    }

    #[test]
    fn 大きな_text_は_64_kib_ごとに分ける() {
        let s = "a".repeat(MAX_TEXT_BYTES + 10);
        let out = parse(s.as_bytes());
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn 部分一致の長さ() {
        assert_eq!(partial_suffix(b"abc\x1b[20", PASTE_END), 4);
        assert_eq!(partial_suffix(b"abc", PASTE_END), 0);
        assert_eq!(partial_suffix(b"\x1b", PASTE_END), 1);
    }
}
