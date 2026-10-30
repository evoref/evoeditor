//! Windows の入力の正規化 (docs/04 §6.1 / §6.2 / §6.6)。`ReadConsoleInputW` が返す記録 (`Raw`) を `InputEvent` にする。
//! OS を呼ばない純粋な状態機械で、読取のバッチ (`feed`) とバーストの終了 (`close_burst`) を呼び側 (`win`) が駆動する。
//!
//! 実測 (S-07、2026-10-21) を反映している:
//! - `Ctrl+文字` は `UnicodeChar` が制御文字で届くので、**仮想キーコードを優先** してキーを作る (バースト中だけ例外)。
//! - IME の確定文字は `vk = 0` の KeyDown / KeyUp の対 (`UnicodeChar` = 文字) で届く → `Text`。
//! - 半角/全角・変換・無変換などは KeyUp だけが届く (KeyDown は IME が消費) → Release は破棄するので何も起きない。
//! - 貼り付けは 1 つの大きなバッチで届く。N = 16・T = 16 ms (`BURST_N` / `BURST_T`)。

use std::collections::BTreeSet;

use eve_view::{InputEvent, KeyCode, KeyEvent, KeyKind, Modifiers};

/// バーストの開始に必要な、文字を生成する KeyDown の件数 (04 §6.6。S-07 で確定)。
pub const BURST_N: usize = 16;
/// バースト中の無入力の待ち (ms)。
pub const BURST_T_MS: u32 = 16;
/// 連続する `Text` を 1 メッセージにまとめる上限 (バイト)。
pub const MAX_TEXT_BYTES: usize = 64 * 1024;

const LEFT_ALT: u32 = 0x0002;
const RIGHT_ALT: u32 = 0x0001;
const LEFT_CTRL: u32 = 0x0008;
const RIGHT_CTRL: u32 = 0x0004;
const SHIFT: u32 = 0x0010;

/// `KEY_EVENT_RECORD` の写し。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyRec {
    pub down: bool,
    pub repeat: u16,
    pub vk: u16,
    /// `UnicodeChar` (UTF-16 の 1 単位。0 は文字なし)。
    pub ch: u16,
    pub state: u32,
}

/// 読んだ記録。マウスとフォーカスは `Ignored` (v0.1 は捨てる、04 §6.2)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Raw {
    Key(KeyRec),
    /// `WINDOW_BUFFER_SIZE_EVENT`。大きさは呼び側が `GetConsoleScreenBufferInfo` から求める。
    Resize,
    /// `FOCUS_EVENT` (真 = フォーカスを得た)。v0.2 から core に渡す (04 §6.2、03 §9.5)。
    Focus(bool),
    Ignored,
}

/// 正規化の出力。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Out {
    Event(InputEvent),
    /// 端末の大きさが変わった (連続は 1 つに合体する)。
    Resize,
}

impl KeyRec {
    fn ctrl(&self) -> bool {
        self.state & (LEFT_CTRL | RIGHT_CTRL) != 0
    }
    fn shift(&self) -> bool {
        self.state & SHIFT != 0
    }
    /// AltGr: 右 Alt、または 左 Alt + Ctrl (欧州配列の AltGr は Ctrl+Alt として届く)。
    fn altgr_like(&self) -> bool {
        self.state & RIGHT_ALT != 0 || (self.state & LEFT_ALT != 0 && self.ctrl())
    }
    fn alt(&self) -> bool {
        self.state & (LEFT_ALT | RIGHT_ALT) != 0
    }
    /// 文字を生成する KeyDown か (バースト検出の数え方、04 §6.6)。
    fn makes_char(&self) -> bool {
        self.down && self.ch != 0
    }
}

/// 印字可能な文字か (C0・DEL・C1 を除く)。
fn printable(u: u16) -> bool {
    u >= 0x20 && u != 0x7F && !(0x80..=0x9F).contains(&u)
}

/// 修飾キー・ロックキー単独。
fn modifier_vk(vk: u16) -> bool {
    matches!(
        vk,
        0x10 | 0x11 | 0x12 | 0x14 | 0x5B | 0x5C | 0x90 | 0x91 | 0xA0..=0xA5
    )
}

/// 仮想キーコード → `KeyCode` (配列に依存する記号キーは US 配列の刻印。JIS の差は v0.2 で確認する)。
fn map_vk(vk: u16) -> Option<KeyCode> {
    Some(match vk {
        0x41..=0x5A => KeyCode::Char((b'a' + (vk - 0x41) as u8) as char),
        0x30..=0x39 => KeyCode::Char((b'0' + (vk - 0x30) as u8) as char),
        0x20 => KeyCode::Char(' '),
        0x0D => KeyCode::Enter,
        0x09 => KeyCode::Tab,
        0x1B => KeyCode::Esc,
        0x08 => KeyCode::Backspace,
        0x2E => KeyCode::Delete,
        0x2D => KeyCode::Insert,
        0x24 => KeyCode::Home,
        0x23 => KeyCode::End,
        0x21 => KeyCode::PageUp,
        0x22 => KeyCode::PageDown,
        0x25 => KeyCode::Left,
        0x26 => KeyCode::Up,
        0x27 => KeyCode::Right,
        0x28 => KeyCode::Down,
        0x5D => KeyCode::Menu,
        0x70..=0x87 => KeyCode::F((vk - 0x70 + 1) as u8),
        0xBA => KeyCode::Char(';'),
        0xBB => KeyCode::Char('='),
        0xBC => KeyCode::Char(','),
        0xBD => KeyCode::Char('-'),
        0xBE => KeyCode::Char('.'),
        0xBF => KeyCode::Char('/'),
        0xC0 => KeyCode::Char('`'),
        0xDB => KeyCode::Char('['),
        0xDC => KeyCode::Char('\\'),
        0xDD => KeyCode::Char(']'),
        0xDE => KeyCode::Char('\''),
        _ => return None,
    })
}

fn key_mods(r: &KeyRec) -> Modifiers {
    let mut m = Modifiers::NONE;
    if r.ctrl() {
        m = m | Modifiers::CTRL;
    }
    if r.alt() {
        m = m | Modifiers::ALT;
    }
    if r.shift() {
        m = m | Modifiers::SHIFT;
    }
    m
}

#[derive(Default)]
struct Burst {
    /// 本文 (UTF-16)。`UnicodeChar` から作る (制御文字もそのまま。CR は改行にする)。
    body: Vec<u16>,
    /// 改行・制御文字・Ctrl 付きキーを含まない (含むなら `Paste`、含まないなら `Text`)。
    plain: bool,
    /// 最後に文字を生成した KeyDown と、その直前の本文の長さ (末尾の 1 打鍵の例外の判定用)。
    last: Option<(KeyRec, usize)>,
}

/// 入力スレッドの正規化の状態。
#[derive(Default)]
pub struct Normalizer {
    /// 押下中の仮想キーコード (キーリピートの判定)。
    pressed: BTreeSet<u16>,
    /// UTF-16 の上位サロゲート (下位が来るまで持つ)。
    high: Option<u16>,
    burst: Option<Burst>,
}

fn push_text(out: &mut Vec<Out>, s: &str) {
    if s.is_empty() {
        return;
    }
    if let Some(Out::Event(InputEvent::Text(last))) = out.last_mut()
        && last.len() + s.len() <= MAX_TEXT_BYTES
    {
        last.push_str(s);
        return;
    }
    out.push(Out::Event(InputEvent::Text(s.to_owned())));
}

impl Normalizer {
    pub fn new() -> Normalizer {
        Normalizer::default()
    }

    /// バースト中か (呼び側は `BURST_T_MS` の待ちで次の入力を待つ)。
    pub fn in_burst(&self) -> bool {
        self.burst.is_some()
    }

    /// 1 回の読取 (待ちなしで読める分をすべて読み切ったもの) を処理する。
    pub fn feed(&mut self, batch: &[Raw]) -> Vec<Out> {
        let mut out = Vec::new();
        let mut resized = false;
        // フォーカスは 1 回の読込の中で最後の状態だけを渡す (切替を連続で受けても確認を何度も始めない)
        let mut focus = None;
        let chars = batch
            .iter()
            .filter(|r| matches!(r, Raw::Key(k) if k.makes_char()))
            .count();
        if self.burst.is_none() && chars >= BURST_N {
            self.burst = Some(Burst {
                plain: true,
                ..Burst::default()
            });
        }
        for raw in batch {
            match raw {
                Raw::Resize => resized = true,
                Raw::Focus(f) => focus = Some(*f),
                Raw::Ignored => {}
                Raw::Key(k) => {
                    if self.burst.is_some() {
                        self.absorb(k);
                    } else {
                        self.normal(k, &mut out);
                    }
                }
            }
        }
        if resized {
            out.push(Out::Resize);
        }
        if let Some(f) = focus {
            out.push(Out::Event(InputEvent::Focus(f)));
        }
        out
    }

    /// バーストを閉じる (T ms 無入力)。本文を `Text` または `Paste` にし、末尾の 1 打鍵がコマンドならキーにする。
    pub fn close_burst(&mut self) -> Vec<Out> {
        let mut out = Vec::new();
        let Some(mut b) = self.burst.take() else {
            return out;
        };
        self.high = None;
        let mut tail = None;
        if let Some((k, len_before)) = b.last
            && (k.ctrl() || k.alt())
            && !(k.altgr_like() && printable(k.ch))
        {
            // 貼り付けの直後に押したショートカット (Ctrl+S 等) は、本文ではなくコマンドとして扱う
            b.body.truncate(len_before);
            tail = Some(k);
            b.plain = !b.body.iter().any(|&u| !printable(u));
        }
        if !b.body.is_empty() {
            let s = String::from_utf16_lossy(&b.body);
            if b.plain {
                // Text は 1 メッセージ 64 KiB まで
                let mut rest = s.as_str();
                while !rest.is_empty() {
                    let mut cut = rest.len().min(MAX_TEXT_BYTES);
                    while !rest.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    out.push(Out::Event(InputEvent::Text(rest[..cut].to_owned())));
                    rest = &rest[cut..];
                }
            } else {
                out.push(Out::Event(InputEvent::Paste(s)));
            }
        }
        if let Some(k) = tail {
            self.pressed.insert(k.vk);
            self.key(&k, &mut out);
        }
        out
    }

    /// バースト中の 1 記録: `UnicodeChar` から本文を作る。文字を生成しない記録 (矢印・修飾キー等) と KeyUp は捨てる。
    fn absorb(&mut self, k: &KeyRec) {
        let Some(b) = self.burst.as_mut() else {
            return;
        };
        if !k.down {
            self.pressed.remove(&k.vk);
            return;
        }
        if k.ch == 0 {
            return;
        }
        let before = b.body.len();
        let n = k.repeat.max(1);
        for _ in 0..n {
            // 0x0D (Enter) は改行として本文に入れる (改行の正規化は core の貼り付けの入口、03 §8.4)
            let u = if k.ch == 0x0D { 0x0A } else { k.ch };
            if !printable(u) || k.ctrl() && !k.altgr_like() {
                b.plain = false;
            }
            b.body.push(u);
        }
        b.last = Some((*k, before));
    }

    /// バースト外の 1 記録。
    fn normal(&mut self, k: &KeyRec, out: &mut Vec<Out>) {
        if !k.down {
            self.pressed.remove(&k.vk);
            return;
        }
        let repeating = k.vk != 0 && !self.pressed.insert(k.vk);
        if modifier_vk(k.vk) {
            return;
        }
        // IME の確定文字 (vk = 0)、AltGr の文字、修飾なし (または Shift のみ) の印字可能文字 → Text
        let text_like = k.ch != 0
            && printable(k.ch)
            && (k.vk == 0 || k.altgr_like() || (!k.ctrl() && !k.alt()));
        if text_like {
            for _ in 0..k.repeat.max(1) {
                self.text_unit(k.ch, out);
            }
            return;
        }
        if k.ch != 0 && k.vk == 0 {
            return; // 印字できない文字だけの記録は捨てる
        }
        let kind = if repeating {
            KeyKind::Repeat
        } else {
            KeyKind::Press
        };
        self.key_kind(k, kind, out);
    }

    fn key(&mut self, k: &KeyRec, out: &mut Vec<Out>) {
        self.key_kind(k, KeyKind::Press, out);
    }

    fn key_kind(&mut self, k: &KeyRec, kind: KeyKind, out: &mut Vec<Out>) {
        let Some(code) = map_vk(k.vk) else {
            return; // 未知のキー (VK_CANCEL・メディアキー等) は捨てる
        };
        let ev = KeyEvent {
            code,
            mods: key_mods(k),
            kind,
        };
        for _ in 0..k.repeat.max(1) {
            out.push(Out::Event(InputEvent::Key(ev)));
        }
    }

    /// UTF-16 の 1 単位をテキストにする (サロゲートペアは上位を持ち越す)。
    fn text_unit(&mut self, u: u16, out: &mut Vec<Out>) {
        match u {
            0xD800..=0xDBFF => {
                self.high = Some(u);
            }
            0xDC00..=0xDFFF => {
                if let Some(h) = self.high.take() {
                    let s = String::from_utf16_lossy(&[h, u]);
                    push_text(out, &s);
                } else {
                    push_text(out, "\u{FFFD}");
                }
            }
            _ => {
                if self.high.take().is_some() {
                    push_text(out, "\u{FFFD}");
                }
                let s = String::from_utf16_lossy(&[u]);
                push_text(out, &s);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(down: bool, vk: u16, ch: u16, state: u32) -> Raw {
        Raw::Key(KeyRec {
            down,
            repeat: 1,
            vk,
            ch,
            state,
        })
    }

    /// KeyDown と KeyUp の対 (1 打鍵)。
    fn tap(vk: u16, ch: u16, state: u32) -> Vec<Raw> {
        vec![key(true, vk, ch, state), key(false, vk, ch, state)]
    }

    fn ev(o: &[Out]) -> Vec<InputEvent> {
        o.iter()
            .filter_map(|x| match x {
                Out::Event(e) => Some(e.clone()),
                Out::Resize => None,
            })
            .collect()
    }

    fn k(code: KeyCode, mods: Modifiers) -> InputEvent {
        InputEvent::Key(KeyEvent::press(code, mods))
    }

    #[test]
    fn 印字可能文字は_text_で_release_は捨てる() {
        let mut n = Normalizer::new();
        let o = n.feed(&tap(0x41, 'a' as u16, 0));
        assert_eq!(ev(&o), vec![InputEvent::Text("a".into())]);
        let o = n.feed(&tap(0x41, 'A' as u16, SHIFT));
        assert_eq!(ev(&o), vec![InputEvent::Text("A".into())]);
    }

    #[test]
    fn 連続する_text_は_1_つにまとまる() {
        let mut n = Normalizer::new();
        let mut b = tap(0x41, 'a' as u16, 0);
        b.extend(tap(0x42, 'b' as u16, 0));
        b.extend(tap(0x0D, 0x0D, 0));
        b.extend(tap(0x43, 'c' as u16, 0));
        let o = ev(&n.feed(&b));
        assert_eq!(
            o,
            vec![
                InputEvent::Text("ab".into()),
                k(KeyCode::Enter, Modifiers::NONE),
                InputEvent::Text("c".into()),
            ]
        );
    }

    #[test]
    fn ctrl_文字は仮想キーコードからキーになる() {
        // S-07 の実測: Ctrl+H は vk=0x48・UnicodeChar=U+0008。Backspace 単独は vk=0x08
        let mut n = Normalizer::new();
        let cases = [
            (0x48, 0x08, KeyCode::Char('h'), "Ctrl+H"),
            (0x49, 0x09, KeyCode::Char('i'), "Ctrl+I"),
            (0x4D, 0x0D, KeyCode::Char('m'), "Ctrl+M"),
            (0xDB, 0x1B, KeyCode::Char('['), "Ctrl+["),
            (0x53, 0x13, KeyCode::Char('s'), "Ctrl+S"),
        ];
        for (vk, ch, code, name) in cases {
            let o = ev(&n.feed(&tap(vk, ch, LEFT_CTRL)));
            assert_eq!(o, vec![k(code, Modifiers::CTRL)], "{name}");
        }
        for (vk, ch, code) in [
            (0x08, 0x08, KeyCode::Backspace),
            (0x09, 0x09, KeyCode::Tab),
            (0x0D, 0x0D, KeyCode::Enter),
            (0x1B, 0x1B, KeyCode::Esc),
        ] {
            let o = ev(&n.feed(&tap(vk, ch, 0)));
            assert_eq!(o, vec![k(code, Modifiers::NONE)]);
        }
    }

    #[test]
    fn ctrl_shift_と_alt() {
        let mut n = Normalizer::new();
        let o = ev(&n.feed(&tap(0x53, 0x13, LEFT_CTRL | SHIFT)));
        assert_eq!(
            o,
            vec![k(KeyCode::Char('s'), Modifiers::CTRL | Modifiers::SHIFT)]
        );
        // Alt+A: UnicodeChar は 'a' だが Alt 付きなのでキー
        let o = ev(&n.feed(&tap(0x41, 'a' as u16, LEFT_ALT)));
        assert_eq!(o, vec![k(KeyCode::Char('a'), Modifiers::ALT)]);
        // Shift+Tab
        let o = ev(&n.feed(&tap(0x09, 0x09, SHIFT)));
        assert_eq!(o, vec![k(KeyCode::Tab, Modifiers::SHIFT)]);
    }

    #[test]
    fn altgr_の文字は_text_() {
        // 欧州配列の AltGr: RALT + LCTRL と印字可能な UnicodeChar
        let mut n = Normalizer::new();
        let o = ev(&n.feed(&tap(0x51, '@' as u16, RIGHT_ALT | LEFT_CTRL)));
        assert_eq!(o, vec![InputEvent::Text("@".into())]);
        // 日本語配列の Ctrl+Alt+A は文字が付かない → キー
        let o = ev(&n.feed(&tap(0x41, 0, LEFT_ALT | LEFT_CTRL)));
        assert_eq!(
            o,
            vec![k(KeyCode::Char('a'), Modifiers::CTRL | Modifiers::ALT)]
        );
    }

    #[test]
    fn 矢印と_f_キーと修飾キー単独() {
        let mut n = Normalizer::new();
        assert_eq!(
            ev(&n.feed(&tap(0x25, 0, 0))),
            vec![k(KeyCode::Left, Modifiers::NONE)]
        );
        assert_eq!(
            ev(&n.feed(&tap(0x27, 0, LEFT_CTRL | SHIFT))),
            vec![k(KeyCode::Right, Modifiers::CTRL | Modifiers::SHIFT)]
        );
        assert_eq!(
            ev(&n.feed(&tap(0x70, 0, 0))),
            vec![k(KeyCode::F(1), Modifiers::NONE)]
        );
        assert_eq!(
            ev(&n.feed(&tap(0x87, 0, 0))),
            vec![k(KeyCode::F(24), Modifiers::NONE)]
        );
        // 修飾キー単独・CapsLock は何も出さない
        assert!(n.feed(&tap(0x11, 0, LEFT_CTRL)).is_empty());
        assert!(n.feed(&tap(0x10, 0, SHIFT)).is_empty());
        assert!(n.feed(&tap(0x14, 0, 0)).is_empty());
    }

    #[test]
    fn 日本語の_ime_キーと_ctrl_break_は捨てる() {
        let mut n = Normalizer::new();
        // 半角/全角: 何も届かない。変換・無変換: KeyUp だけ
        assert!(n.feed(&[key(false, 0x1C, 0, 0)]).is_empty());
        assert!(n.feed(&[key(false, 0x1D, 0, 0)]).is_empty());
        // KeyDown が届いても未知のキーとして捨てる
        assert!(n.feed(&[key(true, 0x1C, 0, 0)]).is_empty());
        assert!(n.feed(&[key(true, 0xF4, 0, 0)]).is_empty());
        // Ctrl+Break の KeyUp (VK_CANCEL)
        assert!(n.feed(&[key(false, 0x03, 3, LEFT_CTRL)]).is_empty());
        assert!(n.feed(&[key(true, 0x03, 3, LEFT_CTRL)]).is_empty());
    }

    #[test]
    fn ime_の確定文字は_vk_0_で届いて_text_になる() {
        let mut n = Normalizer::new();
        let mut b = Vec::new();
        for c in "にほんご".chars() {
            b.extend(tap(0, c as u16, 0));
        }
        // 確定の Enter は KeyUp だけが届く
        b.push(key(false, 0x0D, 0x0D, 0));
        let o = ev(&n.feed(&b));
        assert_eq!(o, vec![InputEvent::Text("にほんご".into())]);
    }

    #[test]
    fn サロゲートペアは_1_文字になる() {
        let mut n = Normalizer::new();
        let u: Vec<u16> = "𠮷😀".encode_utf16().collect();
        let mut b = Vec::new();
        for x in &u {
            b.extend(tap(0, *x, 0));
        }
        assert_eq!(ev(&n.feed(&b)), vec![InputEvent::Text("𠮷😀".into())]);
        // 上位サロゲートが次の読取に持ち越されても同じ
        let mut n = Normalizer::new();
        assert!(ev(&n.feed(&tap(0, u[0], 0))).is_empty());
        assert_eq!(
            ev(&n.feed(&tap(0, u[1], 0))),
            vec![InputEvent::Text("𠮷".into())]
        );
        // 下位だけ来たら置換文字
        let mut n = Normalizer::new();
        assert_eq!(
            ev(&n.feed(&tap(0, 0xDC00, 0))),
            vec![InputEvent::Text("\u{FFFD}".into())]
        );
    }

    #[test]
    fn キーリピートは_repeat_で_release_でリセットされる() {
        let mut n = Normalizer::new();
        let d = key(true, 0x25, 0, 0);
        let o = n.feed(&[d, d, d]);
        let kinds: Vec<KeyKind> = o
            .iter()
            .filter_map(|x| match x {
                Out::Event(InputEvent::Key(k)) => Some(k.kind),
                _ => None,
            })
            .collect();
        assert_eq!(
            kinds,
            vec![KeyKind::Press, KeyKind::Repeat, KeyKind::Repeat]
        );
        n.feed(&[key(false, 0x25, 0, 0)]);
        let o = n.feed(&[d]);
        assert!(matches!(&o[..], [Out::Event(InputEvent::Key(k))] if k.kind == KeyKind::Press));
    }

    #[test]
    fn wrepeatcount_は_n_回分() {
        let mut n = Normalizer::new();
        let r = Raw::Key(KeyRec {
            down: true,
            repeat: 3,
            vk: 0x41,
            ch: 'a' as u16,
            state: 0,
        });
        assert_eq!(ev(&n.feed(&[r])), vec![InputEvent::Text("aaa".into())]);
        let r = Raw::Key(KeyRec {
            down: true,
            repeat: 2,
            vk: 0x28,
            ch: 0,
            state: 0,
        });
        assert_eq!(ev(&n.feed(&[r])).len(), 2);
    }

    #[test]
    fn リサイズは合体し_フォーカスは最後の状態だけを渡す() {
        let mut n = Normalizer::new();
        let o = n.feed(&[Raw::Focus(false), Raw::Focus(true), Raw::Focus(true)]);
        assert_eq!(o, [Out::Event(InputEvent::Focus(true))], "最後の状態だけ");
        let o = n.feed(&[Raw::Resize, Raw::Ignored, Raw::Resize]);
        assert_eq!(o, vec![Out::Resize]);
    }

    fn paste_batch(s: &str) -> Vec<Raw> {
        let mut b = Vec::new();
        for u in s.encode_utf16() {
            let vk = if u == 0x0D {
                0x0D
            } else if u == 0x09 {
                0x09
            } else if u == 0x08 {
                0x08
            } else if u == 0x1B {
                0x1B
            } else {
                0x41
            };
            b.extend(tap(vk, u, 0));
        }
        b
    }

    #[test]
    fn n_件未満の貼り付けは通常の入力として扱われる() {
        // 既知の制約 (04 §6.6): 15 文字は通常入力
        let mut n = Normalizer::new();
        let o = n.feed(&paste_batch("abcdefghijklmno"));
        assert!(!n.in_burst());
        assert_eq!(ev(&o), vec![InputEvent::Text("abcdefghijklmno".into())]);
    }

    #[test]
    fn 改行を含むバーストは_paste_で_制御文字は本文に入る() {
        let mut n = Normalizer::new();
        // conhost の貼り付け: タブ・BS・ESC・CR は VK のキーとして届くが、UnicodeChar から本文を作る
        let text = "line1\tx\u{8}y\u{1B}z\rline2 日本語です\r";
        let o = n.feed(&paste_batch(text));
        assert!(n.in_burst(), "N 件以上でバースト開始");
        assert!(o.is_empty(), "バースト中は T 待ち");
        let o = n.close_burst();
        assert!(!n.in_burst());
        assert_eq!(
            ev(&o),
            vec![InputEvent::Paste(
                "line1\tx\u{8}y\u{1B}z\nline2 日本語です\n".into()
            )]
        );
    }

    #[test]
    fn 改行も制御文字も無いバーストは_text() {
        // IME の長い確定 (20 文字以上) は Paste にならない
        let mut n = Normalizer::new();
        let mut b = Vec::new();
        for c in "わたしのなまえはなかのですよろしくおねがいします".chars()
        {
            b.extend(tap(0, c as u16, 0));
        }
        assert!(n.feed(&b).is_empty());
        assert!(n.in_burst());
        let o = n.close_burst();
        assert_eq!(
            ev(&o),
            vec![InputEvent::Text(
                "わたしのなまえはなかのですよろしくおねがいします".into()
            )]
        );
    }

    #[test]
    fn バーストは複数のバッチにまたがる() {
        let mut n = Normalizer::new();
        assert!(n.feed(&paste_batch("0123456789abcdef\r")).is_empty());
        assert!(n.in_burst());
        // 次のバッチは N 未満でもバーストに連結される
        assert!(n.feed(&paste_batch("xyz")).is_empty());
        let o = ev(&n.close_burst());
        assert_eq!(o, vec![InputEvent::Paste("0123456789abcdef\nxyz".into())]);
    }

    #[test]
    fn 貼り付けの直後のショートカットは本文に入れずコマンドにする() {
        let mut n = Normalizer::new();
        let mut b = paste_batch("0123456789abcdef\r");
        // 末尾の Ctrl+S (conhost では UnicodeChar = U+0013 で届く)
        b.extend(tap(0x11, 0, LEFT_CTRL));
        b.extend(tap(0x53, 0x13, LEFT_CTRL));
        n.feed(&b);
        let o = ev(&n.close_burst());
        assert_eq!(
            o,
            vec![
                InputEvent::Paste("0123456789abcdef\n".into()),
                k(KeyCode::Char('s'), Modifiers::CTRL),
            ]
        );
    }

    #[test]
    fn バーストの途中の_ctrl_付きは本文に入る() {
        let mut n = Normalizer::new();
        let mut b = paste_batch("0123456789abcdef");
        b.extend(tap(0x53, 0x13, LEFT_CTRL)); // 途中の Ctrl+S (0x13)
        b.extend(paste_batch("tail"));
        n.feed(&b);
        let o = ev(&n.close_burst());
        assert_eq!(
            o,
            vec![InputEvent::Paste("0123456789abcdef\u{13}tail".into())]
        );
    }

    #[test]
    fn 大きなバーストの_text_は_64_kib_ごとに分ける() {
        let mut n = Normalizer::new();
        let s: String = "a".repeat(MAX_TEXT_BYTES * 2 + 10);
        let mut b = Vec::new();
        for u in s.encode_utf16() {
            b.push(key(true, 0, u, 0));
        }
        n.feed(&b);
        let o = ev(&n.close_burst());
        let lens: Vec<usize> = o
            .iter()
            .map(|e| match e {
                InputEvent::Text(t) => t.len(),
                _ => 0,
            })
            .collect();
        assert_eq!(lens, vec![MAX_TEXT_BYTES, MAX_TEXT_BYTES, 10]);
    }

    #[test]
    fn バーストの本文の矢印は捨てる() {
        let mut n = Normalizer::new();
        let mut b = paste_batch("0123456789abcdef");
        b.extend(tap(0x25, 0, 0)); // Left は文字を生成しないので本文に入れない
        n.feed(&b);
        let o = ev(&n.close_burst());
        assert_eq!(o, vec![InputEvent::Text("0123456789abcdef".into())]);
    }
}
