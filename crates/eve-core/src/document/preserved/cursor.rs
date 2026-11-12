//! 本文を前から順に読む位置 `TextCursor` (保持方式の記録を本文と突き合わせる処理の下回り。03 §8.7.1)。
//!
//! 読込の完了時の照合 (`PreservedMap::from_records`・`check`) と保存の計画 (`io::save_plan::Planner`)・保存の符号化
//! (`io::io_jobs::SaveStream`) は、記録を範囲の昇順に 1 件ずつ見て、範囲の文字列・行の位置・char 境界を引く。記録ごとに
//! rope を根から降りる (`line_to_byte`・`is_char_boundary`・`chunks`。1 回ごとに木を降りる) と、区分 B の箇所が密な
//! ファイル (数十万件) で読込・保存の時間が倍になる。この位置は本文のチャンクを前から順に持ち、範囲が今のチャンクに
//! 収まれば文字列を借りて返す (rope を降りるのはチャンクごとに 1 回)。後ろへ戻るとき・遠くへ飛ぶときだけ引き直す。
//!
//! 結果は `TextRead` の対応する操作と同じ (rope のチャンクは char 境界で区切られているので、チャンクの中の char 境界は
//! 本文の char 境界と同じ)。

use std::borrow::Cow;

use eve_text::{ByteOff, ByteRange, Chunks, LineIdx, TextRead};

/// 行を前へ順にたどる代わりに、行の位置を引き直す (rope を降りる) 行の差。記録がまばらな文書で、間の行を全部たどらない。
const NEAR_LINES: usize = 16;

/// 本文を前から順に読む位置。
pub(crate) struct TextCursor<'a> {
    text: &'a dyn TextRead,
    len: usize,
    /// 今のチャンクの次からのチャンク。
    rest: Chunks<'a>,
    /// 今のチャンク (空の本文では空)。
    cur: &'a str,
    /// 今のチャンクの先頭の位置。
    base: usize,
    /// 行をたどった位置: 行の番号と行頭 (`line_bounds` が使う)。
    line: usize,
    line_start: usize,
}

impl<'a> TextCursor<'a> {
    pub(crate) fn new(text: &'a dyn TextRead) -> TextCursor<'a> {
        let len = text.len_bytes();
        let mut rest = text.chunks(ByteRange::all(len));
        let cur = rest.next().unwrap_or("");
        TextCursor {
            text,
            len,
            rest,
            cur,
            base: 0,
            line: 0,
            line_start: 0,
        }
    }

    /// 本文の長さ。
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// `pos` を含むチャンク (`pos` が本文の末尾以上なら最後のチャンク) へ移る。前へは順に進み、後ろへ戻るときは引き直す。
    fn seek(&mut self, pos: usize) {
        if pos < self.base {
            let (c, b) = self.text.chunk_at(ByteOff(pos));
            self.cur = c;
            self.base = b.0;
            self.rest = self
                .text
                .chunks(ByteRange::new(ByteOff(b.0 + c.len()), ByteOff(self.len)));
        }
        while pos >= self.base + self.cur.len() {
            let Some(next) = self.rest.next() else {
                return;
            };
            self.base += self.cur.len();
            self.cur = next;
        }
    }

    /// `pos` のバイト (本文の外なら `None`)。今のチャンクより前の位置は、位置を動かさずに引く (直前のバイトを見るため)。
    fn byte(&mut self, pos: usize) -> Option<u8> {
        if pos >= self.len {
            return None;
        }
        if pos < self.base {
            let (c, b) = self.text.chunk_at(ByteOff(pos));
            return c.as_bytes().get(pos.checked_sub(b.0)?).copied();
        }
        self.seek(pos);
        self.cur.as_bytes().get(pos - self.base).copied()
    }

    /// `[s, e)` (`s < e`) のうち、`s` を含むチャンクに収まる前の部分 (空でない)。区間を前から順に切り出すのに使う
    /// (`s` から先は、返した長さだけ進めて呼び直す)。範囲が本文の外・逆順・空、または端が char 境界でなければ `None`。
    pub(crate) fn piece(&mut self, s: usize, e: usize) -> Option<&'a str> {
        if s >= e || e > self.len {
            return None;
        }
        self.seek(s);
        let to = e.min(self.base + self.cur.len());
        self.cur
            .get(s.checked_sub(self.base)?..to - self.base)
            .filter(|p| !p.is_empty())
    }

    /// `[s, e)` が、本文の 1 つの行の内容の全体 (行頭で始まり、その行の終端の直前か本文の末尾で終わる) か (ISO-2022-JP の
    /// 行の記録を保存で使う条件、03 §8.6 E2 の (d))。長さ 0 の範囲は空の行。範囲が本文の外・逆順なら偽。結果は
    /// `byte_to_line` で行を引いて行頭と行の内容の末尾に一致するかを見るのと同じ (試験で比べる)。
    pub(crate) fn is_whole_line(&mut self, s: usize, e: usize) -> bool {
        if s > e || e > self.len {
            return false;
        }
        // 行頭: 本文の先頭、LF の直後、または CR の直後で LF が続かない位置 (CRLF の間は行頭ではない)
        let line_start = s == 0
            || match self.byte(s - 1) {
                Some(b'\n') => true,
                Some(b'\r') => self.byte(s) != Some(b'\n'),
                _ => false,
            };
        // 行の内容の末尾: `s` から後ろで最初の行の終端 (無ければ本文の末尾)
        line_start && self.find_terminator(s) == e
    }

    /// `pos` が char 境界か (`TextRead::is_char_boundary` と同じ。本文の末尾は境界、外は境界でない)。
    pub(crate) fn is_char_boundary(&mut self, pos: usize) -> bool {
        if pos >= self.len {
            return pos == self.len;
        }
        self.seek(pos);
        self.cur.is_char_boundary(pos - self.base)
    }

    /// `[s, e)` の文字列。1 つのチャンクに収まれば借りて返し、またぐときは写す。範囲が本文の外・逆順、または端が
    /// char 境界でなければ `None` (丸めない。`TextRead::chunks` は外側の境界へ丸めるが、ここは丸めずに断る)。
    pub(crate) fn slice(&mut self, s: usize, e: usize) -> Option<Cow<'a, str>> {
        if s > e || e > self.len {
            return None;
        }
        self.seek(s);
        let chunk_end = self.base + self.cur.len();
        if e <= chunk_end {
            return self
                .cur
                .get(s - self.base..e - self.base)
                .map(Cow::Borrowed);
        }
        if !self.is_char_boundary(s) || !self.is_char_boundary(e) {
            return None;
        }
        Some(Cow::Owned(
            self.text
                .slice_to_string(ByteRange::new(ByteOff(s), ByteOff(e))),
        ))
    }

    /// `pos` から後ろで最初の行の終端 (CR か LF) の位置。無ければ本文の末尾。
    fn find_terminator(&mut self, pos: usize) -> usize {
        let mut at = pos;
        while at < self.len {
            self.seek(at);
            let off = at - self.base;
            match self.cur.as_bytes()[off..]
                .iter()
                .position(|&b| b == b'\r' || b == b'\n')
            {
                Some(i) => return at + i,
                None => at = self.base + self.cur.len(),
            }
        }
        self.len
    }

    /// 行 `l` (行数より小さいこと) の行頭と、行の内容の末尾 (`TextRead::line_to_byte` / `line_end_byte` と同じ)。
    /// 前の呼び出しの行から近い後ろの行は、行の終端を前から順にたどって求め、遠い行・前の行は引き直す。
    pub(crate) fn line_bounds(&mut self, l: usize) -> (usize, usize) {
        if l != self.line {
            if l > self.line && l - self.line <= NEAR_LINES {
                while self.line < l {
                    let t = self.find_terminator(self.line_start);
                    let next = match self.byte(t) {
                        Some(b'\r') if self.byte(t + 1) == Some(b'\n') => t + 2,
                        Some(_) => t + 1,
                        // 本文の末尾 (行数より大きい行を求めた。呼び出し側の誤り): 引き直しに任せる
                        None => {
                            self.line_start = self.text.line_to_byte(LineIdx(l)).0;
                            self.line = l;
                            break;
                        }
                    };
                    self.line += 1;
                    self.line_start = next;
                }
            } else {
                self.line_start = self.text.line_to_byte(LineIdx(l)).0;
                self.line = l;
            }
        }
        let end = self.find_terminator(self.line_start);
        (self.line_start, end)
    }
}

#[cfg(test)]
mod tests {
    use eve_text::Text;

    use super::*;

    /// チャンクが小さくなるよう、長い本文を作る (rope のチャンクは 1 KiB 前後)。
    fn sample() -> String {
        let mut s = String::new();
        let parts = ["abc", "日本語", "\r\n", "\n", "\r", "😀", "x\r", "\n\n", ""];
        let mut x = 0x2545_F491_4F6C_DD1Du64;
        while s.len() < 20_000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            s.push_str(parts[(x % 9) as usize]);
        }
        s
    }

    #[test]
    fn 行の位置は_textread_と同じ() {
        for body in [
            sample(),
            String::new(),
            "abc".to_owned(),
            "a\r\n".to_owned(),
        ] {
            let text = Text::from(body.as_str());
            let lines = text.len_lines();
            let want = |l: usize| {
                (
                    text.line_to_byte(LineIdx(l)).0,
                    text.line_end_byte(LineIdx(l)).0,
                )
            };
            // 順に、飛び飛びに、後ろへ戻って
            let mut c = TextCursor::new(&text);
            for l in 0..lines {
                assert_eq!(c.line_bounds(l), want(l), "行 {l}");
            }
            let mut c = TextCursor::new(&text);
            for l in (0..lines).step_by(7).chain((0..lines).step_by(40).rev()) {
                assert_eq!(c.line_bounds(l), want(l), "行 {l}");
            }
        }
    }

    #[test]
    fn 範囲の文字列と境界は_textread_と同じ() {
        let body = sample();
        let text = Text::from(body.as_str());
        let mut c = TextCursor::new(&text);
        let n = body.len();
        // 前へ進みながら、ときどき後ろへ戻る
        let mut s = 0usize;
        let mut step = 1usize;
        while s <= n {
            for len in [0usize, 1, 3, 7, 1500, 3000] {
                let e = s + len;
                let got = c.slice(s, e);
                let ok = e <= n && body.is_char_boundary(s) && body.is_char_boundary(e);
                if ok {
                    assert_eq!(got.as_deref(), Some(&body[s..e]), "{s}..{e}");
                } else {
                    assert_eq!(got, None, "{s}..{e}");
                }
                assert_eq!(c.is_char_boundary(e), text.is_char_boundary(ByteOff(e)));
            }
            if s.is_multiple_of(5) && s > 400 {
                let back = s - 400;
                let want = body.get(back..back + 9);
                assert_eq!(c.slice(back, back + 9).as_deref(), want, "戻る {back}");
            }
            step = step % 97 + 13;
            s += step;
        }
        assert_eq!(c.slice(3, 2), None);
    }

    /// 行の全体の判定を `TextRead` の行の操作で求めたもの (M22 の 6 の保存の計画が使っていた形。比べる相手)。
    fn is_whole_line_by_lines(text: &dyn TextRead, s: usize, e: usize) -> bool {
        if e > text.len_bytes() || s > e {
            return false;
        }
        // CRLF の `\n` の位置は、その CRLF で終わる行に属する (行頭ではない)
        let line = text.byte_to_line(ByteOff(s));
        text.line_to_byte(line).0 == s && text.line_end_byte(line).0 == e
    }

    #[test]
    fn 行の全体の判定は_textread_で求めたものと同じ() {
        for body in [
            sample(),
            "a\r\n\r\rb\n".to_owned(),
            "\r".to_owned(),
            String::new(),
        ] {
            let text = Text::from(body.as_str());
            let n = body.len();
            let mut c = TextCursor::new(&text);
            let mut s = 0usize;
            while s <= n + 1 {
                for e in [s, s + 1, s + 2, s + 5, s + 30] {
                    let want = is_whole_line_by_lines(&text, s, e);
                    assert_eq!(c.is_whole_line(s, e), want, "{s}..{e}");
                }
                // 行頭と行末の組は必ず通す
                if s < text.len_lines() {
                    let (a, b) = (
                        text.line_to_byte(LineIdx(s)).0,
                        text.line_end_byte(LineIdx(s)).0,
                    );
                    assert!(c.is_whole_line(a, b), "行 {s}");
                }
                s += 1 + s % 3;
            }
        }
    }

    #[test]
    fn 区間を前から切り出すとつないだものが範囲の文字列になる() {
        let body = sample();
        let text = Text::from(body.as_str());
        let mut c = TextCursor::new(&text);
        let mut s = 0usize;
        while s < body.len() {
            let e = (s + 2500).min(body.len());
            let a = (s..=e).find(|&i| body.is_char_boundary(i)).unwrap_or(e);
            let b = (a..=e)
                .rev()
                .find(|&i| body.is_char_boundary(i))
                .unwrap_or(a);
            let mut got = String::new();
            let mut at = a;
            while at < b {
                let p = c.piece(at, b).expect("境界の間は切り出せる");
                got.push_str(p);
                at += p.len();
            }
            assert_eq!(got, &body[a..b]);
            s = e + 1;
        }
        assert_eq!(c.piece(5, 5), None);
        assert_eq!(c.piece(0, body.len() + 1), None);
    }
}
