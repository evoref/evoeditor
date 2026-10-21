//! テキストストレージ (docs/03-core-design.md §2、ADR-0004)。
//!
//! ropey 1.x の `Rope` を包む。**ropey の型は公開しない** (T9: 実装を差し替え可能にする)。
//! 行区切りは LF・CRLF・CR の 3 種だけ (`cr_lines` のみ有効、`unicode_lines` は無効。03 §2.3 / §2.4)。
//!
//! 範囲外の入力はパニックせず、末尾に丸める (C2、NFR-305)。char 境界でない位置は、範囲を取る関数では
//! 外側の境界へ (開始は前へ、終了は後ろへ) 丸める。

use std::fmt;

use ropey::{Rope, RopeBuilder};

use crate::types::{ByteOff, ByteRange, CharIdx, LineEnding, LineIdx, Utf16Off};

/// 可変のテキスト。文書の変更は `ChangeSet` 経由だけ (02 §5.2) なので、`Text` を持つのは文書だけにする
/// (複製できない。読み取りの共有は `snapshot()` を使う)。
#[derive(Default)]
pub struct Text {
    rope: Rope,
}

/// 不変のスナップショット。`Clone` は O(1) (内部の木を共有する)。`Send + Sync`。
/// 内容だけを持ち、`Revision` は持たない (eve-text は他の eve-* に依存しない。03 §2.2)。
#[derive(Clone, Default)]
pub struct Snapshot {
    rope: Rope,
}

/// 読み込み時に decode 済みのチャンクを追記して `Text` を作る。全文の `String` を一度も作らない。
#[derive(Default)]
pub struct TextBuilder {
    builder: RopeBuilder,
}

impl TextBuilder {
    pub fn new() -> TextBuilder {
        TextBuilder::default()
    }

    /// チャンクを末尾に追記する。チャンクの境目は任意の位置でよい (CRLF の間でも、行の構造は内容だけで決まる)。
    pub fn append(&mut self, chunk: &str) {
        self.builder.append(chunk);
    }

    pub fn finish(self) -> Text {
        Text {
            rope: self.builder.finish(),
        }
    }
}

impl Text {
    pub fn new() -> Text {
        Text::default()
    }

    /// O(1) の不変スナップショット (T2)。以後の `Text` の変更は、このスナップショットに影響しない (INV-T3)。
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            rope: self.rope.clone(),
        }
    }

    /// 内部の rope への可変参照。`ChangeSet` の適用 (同じ crate の中) だけが使う。
    pub(crate) fn rope_mut(&mut self) -> &mut Rope {
        &mut self.rope
    }
}

impl From<&str> for Text {
    fn from(s: &str) -> Text {
        Text {
            rope: Rope::from_str(s),
        }
    }
}

impl From<&str> for Snapshot {
    fn from(s: &str) -> Snapshot {
        Snapshot {
            rope: Rope::from_str(s),
        }
    }
}

// 文書の本文を `Debug` に出さない (本文を log やパニックに出さない。docs/09 §8.4 の「秘匿」)。
impl fmt::Debug for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Text")
            .field("len_bytes", &self.rope.len_bytes())
            .field("len_lines", &self.rope.len_lines())
            .finish()
    }
}

impl fmt::Debug for Snapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Snapshot")
            .field("len_bytes", &self.rope.len_bytes())
            .field("len_lines", &self.rope.len_lines())
            .finish()
    }
}

/// `chunks` が返すイテレータ。連続した `&str` のチャンクを順に返す (空のチャンクは返さない)。
pub struct Chunks<'a> {
    inner: ropey::iter::Chunks<'a>,
}

impl<'a> Iterator for Chunks<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        self.inner.by_ref().find(|c| !c.is_empty())
    }
}

/// `Text` と `Snapshot` の読み取り操作 (03 §2.2)。オブジェクト安全 (`&dyn TextRead` で使える)。
pub trait TextRead {
    fn len_bytes(&self) -> usize;
    fn len_chars(&self) -> usize;
    /// 行数。末尾が改行なら、その後ろに空の最終行がある (行数 = 区切りの数 + 1。INV-T2)。
    fn len_lines(&self) -> usize;
    /// 位置のバイトを含む char の番号。`b` が char 境界でなければ、その char の番号を返す。範囲外は末尾。
    fn byte_to_char(&self, b: ByteOff) -> CharIdx;
    fn char_to_byte(&self, c: CharIdx) -> ByteOff;
    /// 位置を含む行。`b` が CRLF の `\n` にあっても、その CRLF で終わる行に属する。範囲外は最終行。
    fn byte_to_line(&self, b: ByteOff) -> LineIdx;
    /// 行頭のバイト位置。`l` が行数以上なら文書の末尾。
    fn line_to_byte(&self, l: LineIdx) -> ByteOff;
    /// 行の内容の末尾 (改行文字の直前)。カーソルが置ける行内位置の最大。
    fn line_end_byte(&self, l: LineIdx) -> ByteOff;
    /// その行を終える改行。最終行は `None`。
    fn line_terminator(&self, l: LineIdx) -> Option<LineEnding>;
    /// `b` が char 境界でなければ、その char の先頭の UTF-16 位置。範囲外は末尾。
    fn byte_to_utf16(&self, b: ByteOff) -> Utf16Off;
    /// サロゲートペアの間を指す `u` は、その文字の先頭へ丸める。
    fn utf16_to_byte(&self, u: Utf16Off) -> ByteOff;
    fn is_char_boundary(&self, b: ByteOff) -> bool;
    /// 範囲 (外側の char 境界へ丸める) を、連続したチャンクで走査する。全文の連結を要求しない (T6)。
    fn chunks(&self, range: ByteRange) -> Chunks<'_>;
    /// 位置を含むチャンクと、そのチャンクの先頭のバイト位置。`b` が末尾以上のときは最後のチャンク
    /// (空のテキストでは空の文字列と 0)。
    fn chunk_at(&self, b: ByteOff) -> (&str, ByteOff);
    /// 範囲 (外側の char 境界へ丸める) を `String` にする。小さな範囲にだけ使う。
    fn slice_to_string(&self, range: ByteRange) -> String;
}

fn clamp(b: ByteOff, len: usize) -> usize {
    b.0.min(len)
}

fn boundary(rope: &Rope, b: usize) -> bool {
    if b > rope.len_bytes() {
        return false;
    }
    if b == rope.len_bytes() {
        return true;
    }
    let (chunk, start, _, _) = rope.chunk_at_byte(b);
    chunk.is_char_boundary(b - start)
}

/// `b` 以下で最大の char 境界。
fn floor_boundary(rope: &Rope, b: usize) -> usize {
    let mut b = b.min(rope.len_bytes());
    while !boundary(rope, b) {
        b -= 1;
    }
    b
}

/// `b` 以上で最小の char 境界。
fn ceil_boundary(rope: &Rope, b: usize) -> usize {
    let mut b = b.min(rope.len_bytes());
    while !boundary(rope, b) {
        b += 1;
    }
    b
}

/// 範囲を外側の char 境界へ丸める。**空の範囲 (終点が始点以下。逆順を含む) は丸めても空のまま** で、char の途中の位置でも
/// その char に広げない (点は広がらない。広げると、空の範囲の `chunks` が char 1 つ分の文字列を返してしまう)。
fn rounded_range(rope: &Rope, r: ByteRange) -> (usize, usize) {
    let start = floor_boundary(rope, r.start.0);
    if r.end.0 <= r.start.0 {
        return (start, start);
    }
    let end = ceil_boundary(rope, r.end.0).max(start);
    (start, end)
}

fn terminator_of(rope: &Rope, l: usize) -> Option<LineEnding> {
    // 最終行は改行で終わらない。`l + 1` のあふれを避けるため、行数 - 1 と比べる。
    if l >= rope.len_lines().saturating_sub(1) {
        return None;
    }
    let line = rope.line(l);
    let n = line.len_bytes();
    if n == 0 {
        return None;
    }
    match line.byte(n - 1) {
        b'\n' if n >= 2 && line.byte(n - 2) == b'\r' => Some(LineEnding::Crlf),
        b'\n' => Some(LineEnding::Lf),
        b'\r' => Some(LineEnding::Cr),
        _ => None,
    }
}

macro_rules! impl_text_read {
    ($t:ty) => {
        impl TextRead for $t {
            fn len_bytes(&self) -> usize {
                self.rope.len_bytes()
            }

            fn len_chars(&self) -> usize {
                self.rope.len_chars()
            }

            fn len_lines(&self) -> usize {
                self.rope.len_lines()
            }

            fn byte_to_char(&self, b: ByteOff) -> CharIdx {
                CharIdx(self.rope.byte_to_char(clamp(b, self.rope.len_bytes())))
            }

            fn char_to_byte(&self, c: CharIdx) -> ByteOff {
                ByteOff(self.rope.char_to_byte(c.0.min(self.rope.len_chars())))
            }

            fn byte_to_line(&self, b: ByteOff) -> LineIdx {
                LineIdx(self.rope.byte_to_line(clamp(b, self.rope.len_bytes())))
            }

            fn line_to_byte(&self, l: LineIdx) -> ByteOff {
                if l.0 >= self.rope.len_lines() {
                    ByteOff(self.rope.len_bytes())
                } else {
                    ByteOff(self.rope.line_to_byte(l.0))
                }
            }

            fn line_end_byte(&self, l: LineIdx) -> ByteOff {
                let n = self.rope.len_lines();
                if l.0 >= n {
                    return ByteOff(self.rope.len_bytes());
                }
                match terminator_of(&self.rope, l.0) {
                    Some(t) => ByteOff(self.rope.line_to_byte(l.0 + 1) - t.byte_len()),
                    None => ByteOff(self.rope.len_bytes()),
                }
            }

            fn line_terminator(&self, l: LineIdx) -> Option<LineEnding> {
                terminator_of(&self.rope, l.0)
            }

            fn byte_to_utf16(&self, b: ByteOff) -> Utf16Off {
                let c = self.rope.byte_to_char(clamp(b, self.rope.len_bytes()));
                Utf16Off(self.rope.char_to_utf16_cu(c))
            }

            fn utf16_to_byte(&self, u: Utf16Off) -> ByteOff {
                let c = self
                    .rope
                    .utf16_cu_to_char(u.0.min(self.rope.len_utf16_cu()));
                ByteOff(self.rope.char_to_byte(c))
            }

            fn is_char_boundary(&self, b: ByteOff) -> bool {
                boundary(&self.rope, b.0)
            }

            fn chunks(&self, range: ByteRange) -> Chunks<'_> {
                let (s, e) = rounded_range(&self.rope, range);
                Chunks {
                    inner: self.rope.byte_slice(s..e).chunks(),
                }
            }

            fn chunk_at(&self, b: ByteOff) -> (&str, ByteOff) {
                let (chunk, start, _, _) = self.rope.chunk_at_byte(clamp(b, self.rope.len_bytes()));
                (chunk, ByteOff(start))
            }

            fn slice_to_string(&self, range: ByteRange) -> String {
                let (s, e) = rounded_range(&self.rope, range);
                self.rope.byte_slice(s..e).to_string()
            }
        }
    };
}

impl_text_read!(Text);
impl_text_read!(Snapshot);
