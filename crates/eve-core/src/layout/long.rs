//! 長大行 (行のバイト長が文書の `format.long_line_threshold` を超える行) の列の索引 (03 §14.4、§11.5.1)。
//!
//! 行全体を文字列にも表示の単位にも展開しない。行頭から書記素を順に走査し、4 KiB ごとに書記素の境界に
//! (行内のバイト位置, 表示列, 書記素の数) を記録する。位置 ↔ 表示列は、手前の記録点からの差分走査 (4 KiB と 1 書記素まで)
//! で求める。書記素の境界と幅は通常の行の `units` と同じ (1 書記素の幅は `unit_slow` と同じ値)。
//!
//! **走査** (03 §14.4 の「走査」の表): (a) 印字できる ASCII の長い連続は 1 バイト = 1 書記素 = 幅 1 としてまとめて
//! 数え、(b) 書記素の境界の規則に関わる char の表 (`eve_text::may_join_grapheme`) に無い char が続く間は char ごとに
//! 境界とし、幅は `CharWidth` の表引きで求める。(a)・(b) のどちらも、次の char が表にあるときは、最後の char から
//! 書記素の分割 (表引きの `eve_text::first_grapheme_len`。境界は `first_grapheme` と同じ) に渡す。表にある char の
//! 箇所だけ書記素の分割を行い、表に無い char まで進んだら (a)・(b) に戻る。1 書記素が `MAX_CLUSTER` を超えるときは
//! そこで切る (病的な結合列で、1 回の問い合わせが行の長さに比例しないため)。
//!
//! 索引は **要る位置までしか作らない** (遅延。03 §14.4 の「遅延」): `built_to` まで走査済みで、`ensure_byte` /
//! `ensure_col` / `ensure_end` がそこから先へ走査して記録点を足す。記録点の位置は作る順に依らない (手前の記録点から
//! `STEP` 以上進んだ最初の書記素の境界)。編集では変更の位置より前の記録点だけを残す (差分。03 §14.4 の「編集」、`carry`)。

use std::cell::RefCell;

use eve_encoding::unescape_char;
use eve_text::{
    ByteOff, ByteRange, ChangeSet, TextRead, WidthConfig, first_grapheme_len, grapheme_width,
    may_join_grapheme,
};

use super::is_plain_wide;

/// 記録点の間隔 (バイト。03 §14.4 の `LONG_STEP`)。
const STEP: usize = 4 * 1024;

/// 1 書記素の長さの上限 (バイト)。これを超える結合列は、ここ (char の境界) で切って次の書記素にする (03 §14.4 の「走査」)。
const MAX_CLUSTER: usize = 4 * 1024;

/// 走査で 1 度に写す本文の大きさ (バイト)。最初は小さく (位置からの変換は 4 KiB と少しで止まる)、続けるときは大きく。
const FIRST_BLOCK: usize = 2 * STEP + MAX_CLUSTER;
const BLOCK: usize = 64 * 1024;

/// まとめて数える印字できる ASCII の連続の最小の長さ (バイト)。これより短い連続は 1 文字ずつ数える (片を出す費用の方が
/// 大きいため。絵文字混在の `a日😀b` のような行)。
const ASCII_RUN: usize = 16;

#[cfg(test)]
thread_local! {
    /// 試験で `ColIndex::new` の記録点の間隔を小さくする (文書を通した試験で記録点をまたぐ場合を多く作る)。
    static TEST_STEP: std::cell::Cell<usize> = const { std::cell::Cell::new(STEP) };
    /// 試験で写す本文の大きさ (最初, 続き) を小さくする (写した範囲の端をまたぐ書記素を多く作る)。
    static TEST_BLOCKS: std::cell::Cell<(usize, usize)> =
        const { std::cell::Cell::new((FIRST_BLOCK, BLOCK)) };
}

/// 写す本文の大きさ (最初, 続き)。
fn blocks() -> (usize, usize) {
    #[cfg(test)]
    return TEST_BLOCKS.with(std::cell::Cell::get);
    #[cfg(not(test))]
    (FIRST_BLOCK, BLOCK)
}

/// 一般の経路で求めた幅の覚えの大きさ (char の値の下位ビットで引く。直接写像)。
const MEMO: usize = 1024;

/// 複数の char の書記素の幅の覚え (書記素のバイト列の散らし値で引く。直接写像) の大きさと、覚える書記素の長さの上限。
const CLUSTER_MEMO: usize = 256;
const CLUSTER_MEMO_BYTES: usize = 32;

/// 複数の char の書記素の幅の覚えの 1 項目 (バイト列, その長さ, 幅)。長さ 0 は空き。
type ClusterSlot = ([u8; CLUSTER_MEMO_BYTES], u8, u8);

/// 1 書記素の表示幅の決め方 (文書の表示の設定から作る)。
#[derive(Clone)]
pub(crate) struct CharWidth {
    pub tab: usize,
    /// 不正バイトのエスケープ文字を `<0xHH>` で示す (`format.pua_escape`)。
    pub esc: bool,
    pub cfg: WidthConfig,
    /// 一般の経路 (絵文字・記号など) の幅の覚え。幅は列に依らないので char だけで引ける。
    /// 絵文字の多い 10 MB の行で、1 文字ごとに書記素の表示を組み立てない (BM-106)。幅 0 は空き
    memo: std::cell::RefCell<Box<[(char, u8)]>>,
    /// 複数の char の短い書記素 (`CLUSTER_MEMO_BYTES` バイトまで) の幅の覚え。幅は列に依らない (列で幅が変わるのは
    /// タブだけで、タブは 1 char の書記素)。国旗・異体字セレクタ付きの文字・結合文字付きの文字などが続く行で、
    /// 書記素ごとに幅を求め直さない
    cmemo: std::cell::RefCell<Box<[ClusterSlot]>>,
    /// 最後に求めた長い書記素 (`CLUSTER_MEMO_BYTES` バイトを超えるもの) の (バイト列, 幅)。1 書記素がとても長い行
    /// (上限 `MAX_CLUSTER` で切った同じ結合列が続く病的な行) で、4 KiB ごとに幅を求め直さない
    long_memo: std::cell::RefCell<(Vec<u8>, usize)>,
}

impl CharWidth {
    pub fn new(esc: bool, cfg: WidthConfig) -> Self {
        Self {
            tab: cfg.tab_width.max(1) as usize,
            esc,
            cfg,
            memo: std::cell::RefCell::new(vec![('\0', 0u8); MEMO].into_boxed_slice()),
            cmemo: std::cell::RefCell::new(
                vec![([0u8; CLUSTER_MEMO_BYTES], 0u8, 0u8); CLUSTER_MEMO].into_boxed_slice(),
            ),
            long_memo: std::cell::RefCell::new((Vec::new(), 0)),
        }
    }

    /// 表示列 `col` にある、char `c` 1 つの書記素の幅 (0 にはならない)。
    pub fn of(&self, c: char, col: usize) -> usize {
        match c {
            ' '..='~' => 1,
            '\t' => self.tab - col % self.tab,
            '\u{3000}' => 2,
            c if is_plain_wide(c) => 2,
            c if self.esc && unescape_char(c).is_some() => 6,
            c => {
                let slot = c as usize % MEMO;
                if let Some(&(k, w)) = self.memo.borrow().get(slot)
                    && k == c
                    && w > 0
                {
                    return usize::from(w);
                }
                let w = self.slow(c);
                if let Some(e) = self.memo.borrow_mut().get_mut(slot) {
                    *e = (c, u8::try_from(w).unwrap_or(u8::MAX));
                }
                w
            }
        }
    }

    /// 表示列 `col` にある書記素 `g` の幅 (通常の行の `unit_slow` と同じ値)。
    pub fn cluster(&self, g: &str, col: usize) -> usize {
        let mut cs = g.chars();
        if let (Some(c), None) = (cs.next(), cs.next()) {
            return self.of(c, col);
        }
        let bytes = g.as_bytes();
        let short = bytes.len() <= CLUSTER_MEMO_BYTES;
        let slot = if short {
            // FNV-1a
            let h = bytes.iter().fold(0xCBF2_9CE4_8422_2325u64, |h, &b| {
                (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01B3)
            });
            usize::try_from(h % CLUSTER_MEMO as u64).unwrap_or(0)
        } else {
            0
        };
        if !short {
            let mut last = self.long_memo.borrow_mut();
            if last.0 == bytes {
                return last.1;
            }
            let w = self.multi(g, col);
            last.0.clear();
            last.0.extend_from_slice(bytes);
            last.1 = w;
            return w;
        }
        let mut memo = self.cmemo.borrow_mut();
        let Some(e) = memo.get_mut(slot) else {
            return self.multi(g, col);
        };
        if usize::from(e.1) == bytes.len() && e.0[..bytes.len()] == *bytes {
            return usize::from(e.2);
        }
        let w = self.multi(g, col);
        if let (Ok(n), Ok(wb)) = (u8::try_from(bytes.len()), u8::try_from(w)) {
            e.0[..bytes.len()].copy_from_slice(bytes);
            e.1 = n;
            e.2 = wb;
        }
        w
    }

    /// 複数の char の書記素の幅。表示を組み立てずに `grapheme_width` で求め、0 のとき (表示の組み立てで記号にするもの)
    /// とエスケープ文字を含みうるとき (`U+10FF00..`。UTF-8 の先頭は 0xF4) だけ `unit_slow` を通す。
    fn multi(&self, g: &str, col: usize) -> usize {
        if !g.as_bytes().contains(&0xF4)
            && let w @ 1.. = grapheme_width(g, &self.cfg)
        {
            return usize::from(w);
        }
        super::unit_slow(g, 0, col, self.tab, self.esc, true, &self.cfg).width
    }

    /// 一般の経路: その char だけを 1 書記素とした幅 (`unit_slow` と同じ値)。
    fn slow(&self, c: char) -> usize {
        let mut buf = [0u8; 4];
        let g: &str = c.encode_utf8(&mut buf);
        match grapheme_width(g, &self.cfg) {
            0 => {
                let d = eve_text::grapheme_display(g, &self.cfg);
                usize::from(d.base_width + d.marks_width).max(1)
            }
            w => usize::from(w),
        }
    }
}

/// 行内のある書記素の境界の (バイト位置, 表示列, 行頭からの書記素の数)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ColPoint {
    pub byte: usize,
    pub col: usize,
    pub graphemes: usize,
}

impl ColPoint {
    /// 印字できる ASCII を `k` 個 (1 バイト = 1 書記素 = 幅 1) 進めた位置。
    fn ascii(self, k: usize) -> ColPoint {
        ColPoint {
            byte: self.byte + k,
            col: self.col + k,
            graphemes: self.graphemes + k,
        }
    }
}

/// 索引の作った範囲 (遅延で伸びる)。
#[derive(Clone)]
struct Built {
    /// 記録点 (先頭は行頭)。`byte` は書記素の境界で昇順。
    points: Vec<ColPoint>,
    /// 行頭からここまで走査した (書記素の境界。`points` の最後以上)。
    built_to: ColPoint,
    /// 行末 (行の内容のバイト長・表示幅・書記素の数)。行末まで走査したときだけ `Some`。
    end: Option<ColPoint>,
}

/// 1 行の列の索引。位置はすべて行頭からのバイト。
#[derive(Clone)]
pub(crate) struct ColIndex {
    /// 行の内容のバイト長 (行の終端を含まない)。
    len: usize,
    /// 記録点の間隔 (バイト。`STEP`。試験では小さくして記録点を多く作る)。
    step: usize,
    built: RefCell<Built>,
    rule: CharWidth,
}

/// 走査の 1 片。
enum Piece<'a> {
    /// 印字できる ASCII の連続 (1 バイト = 1 書記素 = 幅 1。最後の 1 文字の後ろも書記素の境界)。
    Ascii(&'a str),
    /// 1 書記素とその幅。
    One(&'a str, usize),
    /// 1 char の書記素とその幅 (表に無い char の連続の速い経路。文字列を切り出さない)。
    Char(char, usize),
}

/// 走査の片を受け取った側の答え。
enum Flow {
    /// 片の全体を進めて続ける。
    Go,
    /// 片の先頭から `k` 書記素 (`Piece::One`・`Piece::Char` では 0) 進めた位置で止める。
    Stop(usize),
}

/// `text` の行 (行頭 `ls`) の、行頭からの位置 `start` (書記素の境界) から `to` (行の内容の末尾) までを書記素に分け、
/// 片ごとに `visit(片の先頭の位置, 片)` に渡す。止めた位置、または `to` の位置を返す。
fn scan(
    text: &dyn TextRead,
    ls: usize,
    start: ColPoint,
    to: usize,
    rule: &CharWidth,
    mut visit: impl FnMut(ColPoint, Piece<'_>) -> Flow,
) -> ColPoint {
    let (first, next_block) = blocks();
    let mut at = start;
    let mut buf = String::new();
    let mut block = first;
    while at.byte < to {
        let mut hi = (at.byte + block).min(to);
        while hi < to && !text.is_char_boundary(ByteOff(ls + hi)) {
            hi += 1;
        }
        buf.clear();
        for chunk in text.chunks(ByteRange::new(ByteOff(ls + at.byte), ByteOff(ls + hi))) {
            buf.push_str(chunk);
        }
        let (moved, stopped) = scan_block(&buf, hi >= to, &mut at, rule, &mut visit);
        if stopped {
            break;
        }
        // 1 書記素も進めなかった (書記素が写した範囲の端より先まで続くかもしれない) ときは、大きく写し直す
        block = if moved == 0 { block * 2 } else { next_block };
    }
    at
}

/// 印字できる ASCII のバイトか。
#[inline]
fn printable(b: u8) -> bool {
    (0x20..=0x7E).contains(&b)
}

/// 写した本文 `s` (先頭は書記素の境界 `pos`) を走査する。`last` は `s` の終わりが行の内容の末尾か。
/// (進めたバイト数, 止めたか) を返す。`s` の終わりで次の char が分からないときは、手前で止めて写し直させる。
#[inline]
fn scan_block(
    s: &str,
    last: bool,
    pos: &mut ColPoint,
    rule: &CharWidth,
    visit: &mut impl FnMut(ColPoint, Piece<'_>) -> Flow,
) -> (usize, bool) {
    let bytes = s.as_bytes();
    // 位置は手元に持ち、返すときに書き戻す
    let mut at = *pos;
    let mut i = 0;
    while i < s.len() {
        // (a) 印字できる ASCII の長い連続 (`ASCII_RUN` バイト以上) はまとめて数える。最後の 1 文字は、次の char が表に
        // ある (または写した範囲の端で分からない) ときは書記素の分割に渡す (例: `e` + U+0301)。短い連続は (b) で数える
        if long_ascii_at(bytes, i) {
            let run = ASCII_RUN
                + bytes[i + ASCII_RUN..]
                    .iter()
                    .position(|&b| !printable(b))
                    .unwrap_or(s.len() - i - ASCII_RUN);
            let next_joins = s[i + run..].chars().next().map_or(!last, may_join_grapheme);
            let n = run - usize::from(next_joins);
            if let Flow::Stop(k) = visit(at, Piece::Ascii(&s[i..i + n])) {
                *pos = at.ascii(k.min(n));
                return (i + k.min(n), true);
            }
            at = at.ascii(n);
            i += n;
            continue;
        }
        // (b) その char も次の char も表に無ければ 1 char = 1 書記素。表に無い char が続く間は、各 char を 1 回だけ
        // 復号して表を引く (次の char の答えを持ち越す)
        let mut chars = s[i..].chars();
        let mut cur = chars.next().map(|c| (c, may_join_grapheme(c)));
        let mut to_cluster = false;
        while let Some((c, c_joins)) = cur {
            if c.is_ascii() && long_ascii_at(bytes, i) {
                break;
            }
            let cl = c.len_utf8();
            let next = chars.next();
            if next.is_none() && !last {
                *pos = at;
                return (i, false);
            }
            let next_joins = next.is_some_and(may_join_grapheme);
            if c_joins || next_joins {
                to_cluster = true;
                break;
            }
            let w = rule.of(c, at.col);
            if let Flow::Stop(_) = visit(at, Piece::Char(c, w)) {
                *pos = at;
                return (i, true);
            }
            at.byte += cl;
            at.col += w;
            at.graphemes += 1;
            i += cl;
            cur = next.map(|n| (n, next_joins));
        }
        if !to_cluster {
            continue;
        }
        // (c) どちらかが表にあれば書記素の分割 (表引きの経路 `first_grapheme_len`。境界は `first_grapheme` と同じ)。
        // 境界は「左の文脈と右の 1 文字」で決まるので、渡すのは `MAX_CLUSTER` と 1 文字まで (写した範囲の残りの全体を
        // 渡さない。1 書記素がとても長い行で、書記素ごとに残りを読み直さない)
        let rest = &s[i..];
        let mut lim = (MAX_CLUSTER + 1).min(rest.len());
        while !rest.is_char_boundary(lim) {
            lim += 1;
        }
        let g = &rest[..first_grapheme_len(&rest[..lim])];
        if g.len() == rest.len() && !last && g.len() <= MAX_CLUSTER {
            break;
        }
        let g = cap_cluster(g);
        let w = rule.cluster(g, at.col);
        if let Flow::Stop(_) = visit(at, Piece::One(g, w)) {
            *pos = at;
            return (i, true);
        }
        at.byte += g.len();
        at.col += w;
        at.graphemes += 1;
        i += g.len();
    }
    *pos = at;
    (i, false)
}

/// 位置 `i` から印字できる ASCII が `ASCII_RUN` バイト以上続くか。
#[inline]
fn long_ascii_at(bytes: &[u8], i: usize) -> bool {
    bytes
        .get(i..i + ASCII_RUN)
        .is_some_and(|w| w.iter().all(|&b| printable(b)))
}

/// 書記素 `g` が `MAX_CLUSTER` を超えるなら、その手前の char の境界までに切る。
fn cap_cluster(g: &str) -> &str {
    if g.len() <= MAX_CLUSTER {
        return g;
    }
    let mut e = MAX_CLUSTER;
    while !g.is_char_boundary(e) {
        e -= 1;
    }
    &g[..e]
}

/// 索引をどこまで作るか。
#[derive(Clone, Copy)]
enum Want {
    /// 位置 `b` 以上の最初の書記素の境界まで。
    Byte(usize),
    /// 表示列が `c` を越える最初の書記素の境界まで。
    Col(usize),
    /// 行末まで。
    End,
}

impl Want {
    fn done(self, at: ColPoint) -> bool {
        match self {
            Want::Byte(b) => at.byte >= b,
            Want::Col(c) => at.col > c,
            Want::End => false,
        }
    }

    /// 位置 `at` からの ASCII の `n` 書記素の中で、最初に `done` になる位置 (`at` からの書記素の数)。
    fn first_in_ascii(self, at: ColPoint, n: usize) -> Option<usize> {
        let k = match self {
            Want::Byte(b) => b.saturating_sub(at.byte),
            Want::Col(c) => c.saturating_add(1).saturating_sub(at.col),
            Want::End => return None,
        };
        (k < n).then_some(k)
    }
}

impl ColIndex {
    /// 長さ `len` (行の内容のバイト長) の行の索引を、まだ何も走査していない形で作る (遅延。03 §14.4)。
    pub fn new(len: usize, rule: CharWidth) -> Self {
        #[cfg(test)]
        let step = TEST_STEP.with(std::cell::Cell::get);
        #[cfg(not(test))]
        let step = STEP;
        Self::with_step(len, rule, step)
    }

    /// 記録点の間隔を指定して作る (試験で記録点をまたぐ場合を多く作る)。
    pub(super) fn with_step(len: usize, rule: CharWidth, step: usize) -> Self {
        let head = ColPoint::default();
        Self {
            len,
            step: step.max(1),
            built: RefCell::new(Built {
                points: vec![head],
                built_to: head,
                end: (len == 0).then_some(head),
            }),
            rule,
        }
    }

    /// 行 (行頭 `ls`、内容の末尾 `le`) を行末まで走査して作る (作り直した索引。試験の比較の相手)。
    #[cfg(test)]
    pub fn build(text: &dyn TextRead, ls: usize, le: usize, rule: CharWidth) -> Self {
        let ix = Self::new(le - ls, rule);
        ix.ensure_end(text, ls);
        ix
    }

    /// 行の内容のバイト長。
    pub fn len(&self) -> usize {
        self.len
    }

    /// `built_to` から先へ、`want` を満たす位置か行末まで走査し、記録点を足す。
    /// 記録点は「手前の記録点から `step` 以上進んだ最初の書記素の境界」なので、どこで止めて続けても同じ位置になる。
    fn extend(&self, text: &dyn TextRead, ls: usize, want: Want) {
        let mut b = self.built.borrow_mut();
        if b.end.is_some() || want.done(b.built_to) {
            return;
        }
        let Built {
            points,
            built_to,
            end,
        } = &mut *b;
        let step = self.step;
        // 記録点は `step` 以上離れるので、行全体でも `len / step + 1` 点。倍々に伸ばして余らせない
        let most = self.len / step + 2;
        if points.capacity() < most {
            points.reserve_exact(most - points.len());
        }
        let mut next = points.last().map_or(0, |p| p.byte) + step;
        let at = scan(text, ls, *built_to, self.len, &self.rule, |at, piece| {
            match piece {
                Piece::Ascii(s) => {
                    // 片の中のどの位置も書記素の境界。止める位置までの記録点をまとめて置く
                    let stop = want.first_in_ascii(at, s.len());
                    let last = at.byte + stop.unwrap_or(s.len() - 1);
                    while next <= last {
                        let p = at.ascii(next.saturating_sub(at.byte));
                        points.push(p);
                        next = p.byte + step;
                    }
                    stop.map_or(Flow::Go, Flow::Stop)
                }
                Piece::One(..) | Piece::Char(..) => {
                    if at.byte >= next {
                        points.push(at);
                        next = at.byte + step;
                    }
                    if want.done(at) {
                        Flow::Stop(0)
                    } else {
                        Flow::Go
                    }
                }
            }
        });
        *built_to = at;
        if at.byte >= self.len {
            *end = Some(at);
        }
    }

    /// 位置 `byte` まで作る (`built_to.byte >= byte`、または行末まで)。
    pub fn ensure_byte(&self, text: &dyn TextRead, ls: usize, byte: usize) {
        self.extend(text, ls, Want::Byte(byte));
    }

    /// 表示列 `col` を越えるまで作る (`built_to.col > col`、または行末まで)。
    pub fn ensure_col(&self, text: &dyn TextRead, ls: usize, col: usize) {
        self.extend(text, ls, Want::Col(col));
    }

    /// 行末まで作る。
    pub fn ensure_end(&self, text: &dyn TextRead, ls: usize) {
        self.extend(text, ls, Want::End);
    }

    /// 行末の (バイト長, 表示幅, 書記素の数)。まだなら行末まで作る (`End`・行末への横スクロール・行末にあるカーソルの桁)。
    pub fn end(&self, text: &dyn TextRead, ls: usize) -> ColPoint {
        self.ensure_end(text, ls);
        self.built.borrow().end.unwrap_or_default()
    }

    /// 位置 `byte` 以前で最も近い記録点 (作った範囲の中で)。
    fn point_before_byte(&self, byte: usize) -> ColPoint {
        let b = self.built.borrow();
        let i = b.points.partition_point(|p| p.byte <= byte);
        b.points[i.saturating_sub(1)]
    }

    /// 位置 `byte` 以上の最初の書記素の境界の (表示列, 書記素の数) と、そこから始まる書記素の幅 (行末では 1)。
    /// 通常の行の「`byte` 以上の最初の単位」と同じ (書記素の途中の位置は次の書記素へ)。行末以上は行末。
    pub fn at_byte(&self, text: &dyn TextRead, ls: usize, byte: usize) -> (ColPoint, usize) {
        if byte >= self.len {
            return (self.end(text, ls), 1);
        }
        self.ensure_byte(text, ls, byte);
        let mut width = 1;
        let at = scan(
            text,
            ls,
            self.point_before_byte(byte),
            self.len,
            &self.rule,
            |at, piece| match piece {
                Piece::Ascii(s) if byte < at.byte + s.len() => {
                    width = 1;
                    Flow::Stop(byte.saturating_sub(at.byte))
                }
                Piece::One(_, w) | Piece::Char(_, w) if at.byte >= byte => {
                    width = w;
                    Flow::Stop(0)
                }
                _ => Flow::Go,
            },
        );
        (at, width)
    }

    /// 表示列 `dcol` に対応する位置。セルの途中は、その書記素の前に寄せる。行の幅以上は行末。
    pub fn at_col(&self, text: &dyn TextRead, ls: usize, dcol: usize) -> ColPoint {
        self.ensure_col(text, ls, dcol);
        let start = {
            let b = self.built.borrow();
            // 行末まで作ったときだけ、行の幅以上かが分かる (作り終えていなければ `built_to.col > dcol` なので行の途中)
            if let Some(end) = b.end
                && dcol >= end.col
            {
                return end;
            }
            let i = b.points.partition_point(|p| p.col <= dcol);
            b.points[i.saturating_sub(1)]
        };
        scan(
            text,
            ls,
            start,
            self.len,
            &self.rule,
            |at, piece| match piece {
                Piece::Ascii(s) if dcol < at.col + s.len() => {
                    Flow::Stop(dcol.saturating_sub(at.col))
                }
                Piece::One(_, w) | Piece::Char(_, w) if at.col + w > dcol => Flow::Stop(0),
                _ => Flow::Go,
            },
        )
    }

    /// 位置 `from` (書記素の境界) から、`f(位置, 書記素, 幅)` が `false` を返すまで書記素を順に渡す (Frame の行の生成が使う)。
    /// 返す位置が行末 (`byte == len()`) なら、そこまでの全体が行の幅 (索引は伸ばさない)。
    pub fn walk(
        &self,
        text: &dyn TextRead,
        ls: usize,
        from: ColPoint,
        mut f: impl FnMut(ColPoint, &str, usize) -> bool,
    ) -> ColPoint {
        scan(
            text,
            ls,
            from,
            self.len,
            &self.rule,
            |at, piece| match piece {
                Piece::Ascii(s) => {
                    for k in 0..s.len() {
                        if !f(at.ascii(k), &s[k..=k], 1) {
                            return Flow::Stop(k);
                        }
                    }
                    Flow::Go
                }
                Piece::One(g, w) => {
                    if f(at, g, w) {
                        Flow::Go
                    } else {
                        Flow::Stop(0)
                    }
                }
                Piece::Char(c, w) => {
                    if f(at, c.encode_utf8(&mut [0u8; 4]), w) {
                        Flow::Go
                    } else {
                        Flow::Stop(0)
                    }
                }
            },
        )
    }

    /// 編集の後に、行頭からのバイト位置 `s` より前の記録点だけを残し、行の長さを `len` にする (03 §14.4 の「編集」の (3))。
    /// `built_to` は `s` より前ならそのまま、そうでなければ `s` より前の最後の記録点に戻し、行末は捨てる。
    /// 行頭の記録点 (位置 0) は常に残す (`s` は 1 以上。`s == 0` の変更は呼び出し側が索引を捨てる)。
    /// 書記素の境界は「左の文脈と右の 1 文字」で決まるので、`s` より前の境界と、そこまでの表示列・書記素の数は変わらない。
    fn keep_before(&mut self, s: usize, len: usize) {
        let b = self.built.get_mut();
        b.points.retain(|p| p.byte == 0 || p.byte < s);
        if b.built_to.byte >= s {
            b.built_to = b.points.last().copied().unwrap_or_default();
        }
        b.end = None;
        self.len = len;
    }

    /// 走査済みの範囲 (試験用): (記録点, `built_to`, 行末)。
    #[cfg(test)]
    fn built_state(&self) -> (Vec<ColPoint>, ColPoint, Option<ColPoint>) {
        let b = self.built.borrow();
        (b.points.clone(), b.built_to, b.end)
    }
}

/// 編集 (`ChangeSet`) で、長大行の索引をどう引き継ぐか (03 §14.4 の「編集」)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Carry {
    /// 行に変更が無い: 索引をすべて残す (行頭は新しい位置 `ls`)。
    Keep { ls: usize },
    /// 行の中に変更がある: 行頭からのバイト数 `s` より前の記録点だけを残す。行の新しい長さは `len`。
    Truncate { ls: usize, s: usize, len: usize },
    /// 行頭が変わる・行が分かれる / つながる・閾値以下になる: 索引を捨てる。
    Drop,
}

/// `before` の行頭 `ls`・内容の長さ `len` の行について、`cs` を適用した後 (`after`) の索引の扱いを決める。
/// `threshold` は長大行の閾値 (行のバイト長がこれ以下になれば捨てる)。
///
/// 規則 (03 §14.4 の「編集」の (1)〜(4)。変更のまとまりごとに、適用前の範囲 `[a, b)` で分ける。`k` は行の直前の改行の
/// バイト数、`t` は行の終端のバイト数):
/// - `b < ls - k` (直前の改行より前): 行頭の位置をずらすだけ。直前の行の内容の末尾 (`a == b == ls - k`) への挿入も、
///   挿入の末尾が CR でなければ (直前の LF と合体しないので) 同じ。
/// - それ以外で `a <= ls` (直前の改行・行頭に触れる。行頭ちょうどへの挿入を含む): 捨てる ((2))。
/// - `t > 0` で `a >= le + t` (行の終端より後ろ): 何もしない ((1))。
/// - `b > le` (行の終端に触れる): 捨てる ((4))。
/// - ほか (行の内容の中。`ls < a`、`b <= le`): `s = min(a - ls)` より前を残す ((3))。
///
/// 最後に、適用後の本文で、新しい行頭が行頭で、その行の内容の長さが見込みどおりかを確かめ、違えば捨てる (挿入した
/// 文字列の改行で行が分かれた場合など)。
pub(super) fn carry(
    before: &dyn TextRead,
    after: &dyn TextRead,
    cs: &ChangeSet,
    ls: usize,
    len: usize,
    threshold: usize,
) -> Carry {
    let l = before.byte_to_line(ByteOff(ls));
    if before.line_to_byte(l).0 != ls || before.line_end_byte(l).0 != ls + len {
        return Carry::Drop;
    }
    let le = ls + len;
    let k = if l.0 == 0 {
        0
    } else {
        before
            .line_terminator(eve_text::LineIdx(l.0 - 1))
            .map_or(0, eve_text::LineEnding::byte_len)
    };
    let t = before
        .line_terminator(l)
        .map_or(0, eve_text::LineEnding::byte_len);
    // 行頭より前の最後の変更の (適用前の終わり, 適用後の終わり)。行頭はその差だけずれる
    let mut shift = (0usize, 0usize);
    let mut s: Option<usize> = None;
    let (mut grow, mut shrink) = (0usize, 0usize);
    for (old, new) in cs.changed_ranges() {
        let (a, b) = (old.start.0, old.end.0);
        // 直前の行の内容の末尾への挿入 (末尾が CR なら直前の LF と合体するので除く)
        let at_prev_end = || {
            k > 0
                && a == b
                && a == ls - k
                && !(new.end.0 > new.start.0 && last_byte(after, new.end.0) == Some(b'\r'))
        };
        if b < ls - k || at_prev_end() {
            shift = (b, new.end.0);
        } else if a <= ls {
            return Carry::Drop;
        } else if t > 0 && a >= le + t {
            // 行の終端より後ろ。以降の変更もすべて後ろ
            break;
        } else if b > le {
            return Carry::Drop;
        } else {
            s = Some(s.map_or(a - ls, |s| s.min(a - ls)));
            grow += new.len();
            shrink += old.len();
        }
    }
    let new_ls = ls - shift.0 + shift.1;
    let new_len = (len + grow).saturating_sub(shrink);
    // 適用後の本文で、行頭と行の長さが見込みどおりか (改行の挿入で行が分かれた・CR と LF が合体した場合は違う)
    let nl = after.byte_to_line(ByteOff(new_ls));
    if after.line_to_byte(nl).0 != new_ls
        || after.line_end_byte(nl).0 != new_ls + new_len
        || new_len <= threshold
    {
        return Carry::Drop;
    }
    match s {
        None => Carry::Keep { ls: new_ls },
        Some(s) => Carry::Truncate {
            ls: new_ls,
            s,
            len: new_len,
        },
    }
}

/// 位置 `end` の直前のバイト (`end` は char の境界)。
fn last_byte(text: &dyn TextRead, end: usize) -> Option<u8> {
    let (chunk, start) = text.chunk_at(ByteOff(end - 1));
    chunk.as_bytes().get(end - 1 - start.0).copied()
}

impl Carry {
    /// 引き継ぐときの新しい行頭 (捨てるなら `None`)。
    pub(super) fn new_ls(self) -> Option<usize> {
        match self {
            Carry::Keep { ls } | Carry::Truncate { ls, .. } => Some(ls),
            Carry::Drop => None,
        }
    }
}

impl ColIndex {
    /// `carry` の結果を当てる (`Drop` では何もしない。捨てるのは覚えの側)。
    pub(super) fn apply_carry(&mut self, c: Carry) {
        if let Carry::Truncate { s, len, .. } = c {
            self.keep_before(s, len);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{unit_slow, units_with};
    use super::*;
    use eve_text::Text;

    fn rule() -> CharWidth {
        CharWidth::new(true, super::super::width_config())
    }

    /// 通常の行の経路 (`units_with`) の各単位の (バイト位置, 表示列, 幅)。
    fn by_units(s: &str, r: &CharWidth) -> Vec<(usize, usize, usize)> {
        units_with(s, r.esc, true, &r.cfg)
            .iter()
            .map(|u| (u.byte, u.col, u.width))
            .collect()
    }

    /// 索引で、通常の行の単位の位置ごとに引いた (バイト位置, 表示列, 幅)。
    fn by_index(s: &str, r: &CharWidth) -> Vec<(usize, usize, usize)> {
        let t = Text::from(s);
        let ix = ColIndex::build(&t, 0, s.len(), r.clone());
        by_units(s, r)
            .iter()
            .map(|&(b, _, _)| {
                let (p, w) = ix.at_byte(&t, 0, b);
                (p.byte, p.col, w)
            })
            .collect()
    }

    #[test]
    fn char_width_matches_single_char_units() {
        // 1 char を書記素とした通常の経路の幅と一致する (サロゲートを除く BMP と補助面の主な範囲。列 0 と 3 で)
        for esc in [false, true] {
            for ambiguous_wide in [false, true] {
                let cfg = WidthConfig {
                    ambiguous_wide,
                    ..super::super::width_config()
                };
                let r = CharWidth::new(esc, cfg);
                for v in (0x01..0x3_0000u32).filter(|v| !matches!(v, 0x0A | 0x0D)) {
                    let Some(c) = char::from_u32(v) else { continue };
                    let g = c.to_string();
                    for col in [0, 3] {
                        let u = unit_slow(&g, 0, col, r.tab, esc, true, &cfg);
                        assert_eq!(r.of(c, col), u.width, "U+{v:04X} col {col}");
                    }
                }
            }
        }
    }

    /// 書記素の単位 (03 §14.4): 結合文字の列・ZWJ の列・国旗・ハングル・InCB の結合・キーキャップ・エスケープ文字を含む行でも、
    /// 索引の位置・表示列・幅・書記素の数が通常の行の単位と同じ。
    #[test]
    fn index_matches_units_by_grapheme() {
        let r = rule();
        let samples = [
            "abc def",
            "日本語のテキスト、です。",
            "a\tb\t\tc",
            "混在 abc\t漢字 😀 x",
            "\u{1}ctl\u{7F}\u{85}",
            "全角\u{3000}空白",
            "e\u{301}x か\u{3099}き a\u{301}\u{302}\u{303}b",
            "👨\u{200D}👩\u{200D}👧 👍🏽 1\u{FE0F}\u{20E3} ❤\u{FE0F}",
            "🇯🇵🇺🇸\u{1F1EF}x",
            "\u{1100}\u{1161}\u{11A8} 한국어 \u{AC00}\u{11A8}",
            "\u{0915}\u{094D}\u{0937} नमस्ते",
            "\u{0600}a \u{10FF9A}\u{301} a\u{10FF80}\u{200D}b",
            "\u{301}\u{301}x\u{200D}",
        ];
        // 32 バイトを超える書記素 (幅の覚えの外。最後の 1 つだけ覚える) が、違う中身で交互に続く行
        let long_a = format!("a{}", "\u{301}".repeat(20));
        let long_b = format!("\u{1F468}{}", "\u{200D}\u{1F469}".repeat(5));
        let long_c = format!("\u{1}{}", "\u{301}".repeat(20));
        let mixed = format!("{long_a}{long_b}{long_a}{long_a}x{long_c}{long_b}{long_b}");
        for s in samples.into_iter().chain([mixed.as_str()]) {
            assert_eq!(by_index(s, &r), by_units(s, &r), "{s:?}");
            let t = Text::from(s);
            let ix = ColIndex::build(&t, 0, s.len(), r.clone());
            let units = by_units(s, &r);
            let last = units.last().map_or(0, |u| u.1 + u.2);
            assert_eq!(ix.end(&t, 0).col, last, "{s:?}");
            assert_eq!(ix.end(&t, 0).graphemes, units.len(), "{s:?}");
            // 書記素の途中の位置は、次の書記素 (通常の行の「その位置以上の最初の単位」)
            for (b, _) in s.char_indices() {
                let want = units.iter().find(|u| u.0 >= b).map_or(s.len(), |u| u.0);
                assert_eq!(ix.at_byte(&t, 0, b).0.byte, want, "{s:?} {b}");
            }
        }
    }

    #[test]
    fn byte_at_col_snaps_before() {
        let s = "aあb\tc";
        let t = Text::from(s);
        let ix = ColIndex::build(&t, 0, s.len(), rule());
        let byte = |c| ix.at_col(&t, 0, c).byte;
        assert_eq!(byte(0), 0);
        assert_eq!(byte(1), 1); // あ
        assert_eq!(byte(2), 1); // あ の右半分は前に寄せる
        assert_eq!(byte(3), 4); // b
        assert_eq!(byte(4), 5); // タブ (列 4..8)
        assert_eq!(byte(7), 5);
        assert_eq!(byte(8), 6); // c
        assert_eq!(byte(99), s.len());
    }

    /// 記録点をまたぐ長い行 (記録点が書記素の境界にあることと、差分走査の結果が先頭からの走査と一致すること)。
    /// 結合文字の列が写す範囲の端をまたぐ場合も含める。
    #[test]
    fn long_line_points_and_lookups() {
        let unit = "ab\tあ😀ce\u{301}\u{302}👨\u{200D}👩🇯🇵";
        let s = format!("x{}", unit.repeat(BLOCK * 3 / unit.len()));
        let t = Text::from(format!("first\n{s}\nlast").as_str());
        let ls = 6;
        let r = rule();
        let ix = ColIndex::build(&t, ls, ls + s.len(), r.clone());
        let (points, _, _) = ix.built_state();
        let units = units_with(&s, r.esc, true, &r.cfg);
        assert!(points.len() >= BLOCK * 3 / STEP);
        let starts: std::collections::HashSet<usize> = units.iter().map(|u| u.byte).collect();
        assert!(points.iter().all(|p| starts.contains(&p.byte)));
        assert_eq!(ix.end(&t, ls).byte, s.len());
        assert_eq!(ix.end(&t, ls).graphemes, units.len());
        // 先頭から数えた値と照合する (幾つかの位置)
        for (n, u) in units.iter().enumerate() {
            if n % 7919 == 0 || u.byte + 40 > s.len() {
                let (p, w) = ix.at_byte(&t, ls, u.byte);
                assert_eq!((p.col, p.graphemes), (u.col, n), "byte {}", u.byte);
                assert_eq!(w, u.width);
                assert_eq!(ix.at_col(&t, ls, u.col).byte, u.byte, "col {}", u.col);
            }
        }
        assert_eq!(
            ix.end(&t, ls).col,
            units.last().map_or(0, |u| u.col + u.width)
        );
    }

    /// 1 書記素が `MAX_CLUSTER` を超える結合列は、そこで切る (1 回の問い合わせが行の長さに比例しない)。
    #[test]
    fn huge_cluster_is_cut() {
        let s = format!("a{}b", "\u{301}".repeat(MAX_CLUSTER));
        let t = Text::from(s.as_str());
        let ix = ColIndex::build(&t, 0, s.len(), rule());
        let mut seen = Vec::new();
        ix.walk(&t, 0, ColPoint::default(), |at, g, _| {
            seen.push((at.byte, g.len()));
            true
        });
        assert!(seen.iter().all(|&(_, n)| n <= MAX_CLUSTER), "{seen:?}");
        assert_eq!(seen[0].0, 0);
        assert!(seen[0].1 + 4 > MAX_CLUSTER, "{:?}", seen[0]);
        assert_eq!(seen.last().map(|&(b, _)| b), Some(s.len() - 1));
        assert_eq!(ix.end(&t, 0).graphemes, seen.len());
    }

    #[test]
    fn walk_stops_where_told() {
        let s = "abcあいう";
        let t = Text::from(s);
        let ix = ColIndex::build(&t, 0, s.len(), rule());
        let from = ix.at_col(&t, 0, 2);
        let mut got = String::new();
        let stop = ix.walk(&t, 0, from, |at, g, _| {
            if at.col >= 7 {
                return false;
            }
            got.push_str(g);
            true
        });
        assert_eq!(got, "cあい");
        assert_eq!((stop.byte, stop.col), (9, 7));
    }

    #[test]
    fn doc_settings_are_reflected() {
        // タブ幅は文書の設定から (呼び出し側は doc_width_config(doc) を渡す)
        let cfg = WidthConfig {
            tab_width: 8,
            ..super::super::width_config()
        };
        let r = CharWidth::new(false, cfg);
        assert_eq!(r.of('\t', 3), 5);
    }

    // ---------- 遅延 (03 §14.4 の「遅延」、09 §3.2 の (c)) ----------

    /// 試験用の長い行 (記録点の間隔 `step` をまたぐ)。
    fn mixed_line(n: usize) -> String {
        let unit = "ab\tあ😀c\u{3000}\u{1}\u{301}x";
        unit.repeat(n)
    }

    /// 作る順に依らず、行末まで作った記録点が同じになる。途中の答えも作り直した索引と同じ。
    #[test]
    fn lazy_points_do_not_depend_on_build_order() {
        let s = mixed_line(40);
        let t = Text::from(format!("前\n{s}\n後").as_str());
        let ls = "前\n".len();
        let full = ColIndex::with_step(s.len(), rule(), 17);
        full.ensure_end(&t, ls);
        let lazy = ColIndex::with_step(s.len(), rule(), 17);
        // 開いた直後は何も作らない
        assert_eq!(lazy.built_state().1, ColPoint::default());
        assert_eq!(lazy.built_state().2, None);
        // 途中まで (列で・位置で) 作る
        let p = lazy.at_col(&t, ls, 30);
        assert_eq!(p, full.at_col(&t, ls, 30));
        let (_, built_to, end) = lazy.built_state();
        assert!(built_to.col > 30 && built_to.byte < s.len());
        assert_eq!(end, None, "行末までは作らない");
        assert_eq!(lazy.at_byte(&t, ls, 200), full.at_byte(&t, ls, 200));
        assert!(lazy.built_state().1.byte >= 200);
        assert!(lazy.built_state().1.byte < s.len());
        // 行末
        assert_eq!(lazy.end(&t, ls), full.end(&t, ls));
        assert_eq!(lazy.built_state(), full.built_state());
    }

    /// 行の外の位置・列 (行末以上) は行末を返し、行末まで作る。
    #[test]
    fn lookups_past_the_end_build_to_the_end() {
        let s = mixed_line(10);
        let t = Text::from(s.as_str());
        let ix = ColIndex::with_step(s.len(), rule(), 8);
        let (p, w) = ix.at_byte(&t, 0, s.len());
        assert_eq!((p.byte, w), (s.len(), 1));
        assert!(ix.built_state().2.is_some());
        let ix = ColIndex::with_step(s.len(), rule(), 8);
        assert_eq!(ix.at_col(&t, 0, usize::MAX).byte, s.len());
        assert!(ix.built_state().2.is_some());
    }

    // ---------- 差分 (03 §14.4 の「編集」、09 §3.2 の (b)) ----------

    use eve_text::{ByteRange, Edit};

    fn cs_of(base: &Text, edits: &[(usize, usize, &str)]) -> ChangeSet {
        ChangeSet::from_sorted_edits(
            base,
            edits.iter().map(|&(a, b, s)| Edit {
                range: ByteRange::new(ByteOff(a), ByteOff(b)),
                insert: s.into(),
            }),
        )
        .expect("変更")
    }

    /// `before` の行頭 `ls`・長さ `len` の行を、`edits` を当てたときの扱い。
    fn carry_of(before: &str, ls: usize, len: usize, edits: &[(usize, usize, &str)]) -> Carry {
        let b = Text::from(before);
        let cs = cs_of(&b, edits);
        // 適用後の本文 (`Text::apply` は document の外から呼ばない、02 §5.2)
        let mut after = before.to_owned();
        for &(x, y, ins) in edits.iter().rev() {
            after.replace_range(x..y, ins);
        }
        let a = Text::from(after.as_str());
        carry(&b, &a, &cs, ls, len, 4)
    }

    #[test]
    fn carry_rules() {
        // 行 "0123456789" (行頭 3、長さ 10)。直前の改行は LF、行の終端は LF
        let doc = "ab\n0123456789\ncd";
        let (ls, len) = (3, 10);
        // (1) 行の外: 直前の行の中・直前の行の内容の末尾への挿入・行の終端より後ろ
        assert_eq!(
            carry_of(doc, ls, len, &[(0, 1, "xyz")]),
            Carry::Keep { ls: 5 }
        );
        assert_eq!(
            carry_of(doc, ls, len, &[(2, 2, "x")]),
            Carry::Keep { ls: 4 }
        );
        assert_eq!(
            carry_of(doc, ls, len, &[(14, 15, "")]),
            Carry::Keep { ls: 3 }
        );
        assert_eq!(
            carry_of(doc, ls, len, &[(16, 16, "\r\n")]),
            Carry::Keep { ls: 3 }
        );
        // (2) 行頭ちょうどへの挿入・直前の改行への変更
        assert_eq!(carry_of(doc, ls, len, &[(3, 3, "x")]), Carry::Drop);
        assert_eq!(carry_of(doc, ls, len, &[(2, 3, "")]), Carry::Drop);
        assert_eq!(
            carry_of(doc, ls, len, &[(2, 2, "\r")]),
            Carry::Drop,
            "CR + LF の合体"
        );
        assert_eq!(
            carry_of(doc, ls, len, &[(3, 4, "")]),
            Carry::Drop,
            "行頭の削除"
        );
        // (3) 行の中: 最初の変更の位置 (行頭から) より前を残す
        assert_eq!(
            carry_of(doc, ls, len, &[(6, 8, "xyz")]),
            Carry::Truncate {
                ls: 3,
                s: 3,
                len: 11
            }
        );
        assert_eq!(
            carry_of(doc, ls, len, &[(0, 1, ""), (6, 6, "q"), (10, 11, "")]),
            Carry::Truncate {
                ls: 2,
                s: 3,
                len: 10
            }
        );
        assert_eq!(
            carry_of(doc, ls, len, &[(13, 13, "!")]),
            Carry::Truncate {
                ls: 3,
                s: 10,
                len: 11
            },
            "行末 (終端の手前) への挿入"
        );
        // (4) 行が分かれる・つながる・閾値以下になる
        assert_eq!(carry_of(doc, ls, len, &[(6, 6, "\n")]), Carry::Drop);
        assert_eq!(carry_of(doc, ls, len, &[(13, 14, "")]), Carry::Drop);
        assert_eq!(
            carry_of(doc, ls, len, &[(13, 13, "\r")]),
            Carry::Drop,
            "CR + LF"
        );
        assert_eq!(
            carry_of(doc, ls, len, &[(5, 12, "")]),
            Carry::Drop,
            "閾値以下"
        );
    }

    #[test]
    fn carry_rules_crlf_and_cr() {
        // 直前の改行が CRLF: その片方の削除・間への挿入は捨てる。CRLF の前の内容の末尾への挿入は残す
        let doc = "ab\r\n0123456789";
        let (ls, len) = (4, 10);
        assert_eq!(
            carry_of(doc, ls, len, &[(2, 3, "")]),
            Carry::Drop,
            "CR の削除"
        );
        assert_eq!(
            carry_of(doc, ls, len, &[(3, 4, "")]),
            Carry::Drop,
            "LF の削除"
        );
        assert_eq!(
            carry_of(doc, ls, len, &[(3, 3, "x")]),
            Carry::Drop,
            "CR と LF の間"
        );
        assert_eq!(
            carry_of(doc, ls, len, &[(2, 2, "x")]),
            Carry::Keep { ls: 5 }
        );
        // 最終行 (終端なし) の末尾への追加は行の中
        assert_eq!(
            carry_of(doc, ls, len, &[(14, 14, "zz")]),
            Carry::Truncate {
                ls: 4,
                s: 10,
                len: 12
            }
        );
        // 直前の改行が CR: その後ろへの LF の挿入 (CRLF に合体。行頭ちょうどへの挿入) は捨てる
        let doc = "ab\r0123456789\rcd";
        let (ls, len) = (3, 10);
        assert_eq!(carry_of(doc, ls, len, &[(3, 3, "\n")]), Carry::Drop);
        // 行の終端 (CR) の後ろへの LF の挿入は、行の内容を変えない
        assert_eq!(
            carry_of(doc, ls, len, &[(14, 14, "\n")]),
            Carry::Keep { ls: 3 }
        );
        // 文書の先頭の行は、先頭への挿入で捨てる
        assert_eq!(carry_of("0123456789\n", 0, 10, &[(0, 0, "x")]), Carry::Drop);
        assert_eq!(
            carry_of("0123456789\n", 0, 10, &[(1, 1, "x")]),
            Carry::Truncate {
                ls: 0,
                s: 1,
                len: 11
            }
        );
    }

    /// 差分で引き継いだ索引を当てると、`s` より前の記録点だけが残り、残りは作り直しと同じになる。
    #[test]
    fn truncated_index_rebuilds_the_rest() {
        let s = mixed_line(30);
        let t = Text::from(s.as_str());
        let mut ix = ColIndex::with_step(s.len(), rule(), 13);
        ix.ensure_end(&t, 0);
        let mid = s
            .char_indices()
            .nth(s.chars().count() / 2)
            .map_or(0, |(b, _)| b);
        let cs = cs_of(&t, &[(mid, mid, "Z\t全")]);
        let after = Text::from(format!("{}Z	全{}", &s[..mid], &s[mid..]).as_str());
        let c = carry(&t, &after, &cs, 0, s.len(), 4);
        assert_eq!(
            c,
            Carry::Truncate {
                ls: 0,
                s: mid,
                len: s.len() + "Z\t全".len()
            }
        );
        ix.apply_carry(c);
        let (points, built_to, end) = ix.built_state();
        assert!(points.iter().all(|p| p.byte < mid));
        assert_eq!(built_to, *points.last().expect("行頭"));
        assert_eq!(end, None);
        let fresh = ColIndex::with_step(ix.len(), rule(), 13);
        assert_eq!(ix.end(&after, 0), fresh.end(&after, 0));
        assert_eq!(ix.built_state(), fresh.built_state());
    }

    // ---------- 文書を通した性質 (09 §3.2 の長大行の列の索引の (a)・(b)・(c)) ----------

    mod doc_props {
        use super::super::super::{byte_at_col, cursor_cell, long_line_graphemes};
        use super::*;
        use crate::document::{Document, Origin, TextFormat, Transaction};
        use crate::time::Tick;
        use proptest::prelude::*;

        /// 長大行の閾値 (バイト)。短い行は 4 文字まで (16 バイト以下) にする。
        const THRESHOLD: usize = 24;
        /// 記録点の間隔 (バイト)。
        const TEST_STEP_BYTES: usize = 11;

        /// 長い行の char (重み付き。ASCII・タブ・全角・絵文字・結合文字・異体字セレクタ・肌色修飾・ZWJ・国旗の片方・
        /// ハングルの字母と音節・InCB の子音と Linker・Prepend・SpacingMark・制御文字・エスケープ文字。09 §3.2)。
        fn line_char() -> impl Strategy<Value = char> {
            prop_oneof![
                12 => proptest::char::range('a', 'z'),
                2 => Just('\t'),
                2 => Just('あ'),
                1 => Just('\u{3000}'),
                1 => Just('😀'),
                1 => Just('👩'),
                2 => Just('\u{301}'),
                1 => Just('\u{3099}'),
                1 => Just('\u{FE0F}'),
                1 => Just('\u{1F3FB}'),
                2 => Just('\u{200D}'),
                1 => Just('\u{1F1EF}'),
                1 => Just('\u{1F1F5}'),
                1 => Just('\u{1100}'),
                1 => Just('\u{1161}'),
                1 => Just('\u{11A8}'),
                1 => Just('\u{AC00}'),
                1 => Just('한'),
                1 => Just('\u{0915}'),
                1 => Just('\u{094D}'),
                1 => Just('\u{0600}'),
                1 => Just('\u{0903}'),
                1 => Just('\u{1}'),
                1 => Just('\u{85}'),
                1 => Just('\u{10FF41}'),
            ]
        }

        /// 挿入する文字列 (改行を含みうる)。
        fn insert_text() -> impl Strategy<Value = String> {
            proptest::collection::vec(
                prop_oneof![
                    6 => line_char(),
                    1 => Just('\n'),
                    1 => Just('\r'),
                ],
                0..5,
            )
            .prop_map(|v| v.into_iter().collect())
        }

        fn eol() -> impl Strategy<Value = &'static str> {
            prop_oneof![Just("\n"), Just("\r\n"), Just("\r")]
        }

        fn short_line() -> impl Strategy<Value = String> {
            proptest::collection::vec(line_char(), 0..4).prop_map(|v| v.into_iter().collect())
        }

        fn long_line() -> impl Strategy<Value = String> {
            proptest::collection::vec(line_char(), 20..120).prop_map(|v| v.into_iter().collect())
        }

        /// 初期の文書: 短い行 (0〜2 行)・長い行・(終端と短い行、または終端なし)・長い行 (ときどき)。
        fn initial_doc() -> impl Strategy<Value = String> {
            (
                proptest::collection::vec((short_line(), eol()), 0..3),
                long_line(),
                proptest::option::of((eol(), short_line())),
                proptest::option::of((eol(), long_line())),
            )
                .prop_map(|(pre, long, tail, second)| {
                    let mut s = String::new();
                    for (l, e) in pre {
                        s.push_str(&l);
                        s.push_str(e);
                    }
                    s.push_str(&long);
                    if let Some((e, l)) = tail {
                        s.push_str(e);
                        s.push_str(&l);
                    }
                    if let Some((e, l)) = second {
                        s.push_str(e);
                        s.push_str(&l);
                    }
                    s
                })
        }

        /// 操作。位置は適用の時点の文書から選ぶ (`pick` で長大行を、`at` / `n` で行の中の位置を)。
        #[derive(Clone, Debug)]
        enum Op {
            /// 長大行の行頭ちょうどへの挿入 (CR の後ろへの LF の挿入を含む)。
            AtLineStart {
                pick: usize,
                text: String,
            },
            /// 長大行の直前の改行の変更: 0 = 終端の最後の 1 バイトを消す (CRLF の LF)、1 = 最初の 1 バイトを消す
            /// (CRLF の CR)、2 = 終端の直前に CR を挿入、3 = 直前の行の内容の末尾に文字を挿入。
            PrevEol {
                pick: usize,
                kind: u8,
            },
            /// 長大行の中の置き換え (`at` の位置から `n` char を消して `text` を入れる)。
            InLine {
                pick: usize,
                at: usize,
                n: usize,
                text: String,
            },
            /// 長大行の終端の変更: 0 = 終端を消す、1 = 終端の直前に挿入、2 = 終端の直後に挿入。
            OwnEol {
                pick: usize,
                kind: u8,
                text: String,
            },
            /// 文書のどこかの置き換え。
            Anywhere {
                at: usize,
                n: usize,
                text: String,
            },
            Undo,
            Redo,
            /// 索引を途中まで作る (カーソルのセル・列からの位置・桁を引く)。
            Look {
                pick: usize,
                at: usize,
            },
        }

        fn op() -> impl Strategy<Value = Op> {
            let ix = 0..64usize;
            prop_oneof![
                2 => (ix.clone(), insert_text()).prop_map(|(pick, text)| Op::AtLineStart { pick, text }),
                3 => (ix.clone(), 0..4u8).prop_map(|(pick, kind)| Op::PrevEol { pick, kind }),
                5 => (ix.clone(), 0..256usize, 0..4usize, insert_text())
                    .prop_map(|(pick, at, n, text)| Op::InLine { pick, at, n, text }),
                1 => (ix.clone(), 0..3u8, insert_text())
                    .prop_map(|(pick, kind, text)| Op::OwnEol { pick, kind, text }),
                2 => (0..4096usize, 0..6usize, insert_text())
                    .prop_map(|(at, n, text)| Op::Anywhere { at, n, text }),
                1 => Just(Op::Undo),
                1 => Just(Op::Redo),
                4 => (ix, 0..4096usize).prop_map(|(pick, at)| Op::Look { pick, at }),
            ]
        }

        /// 操作の列。行頭ちょうどへの挿入 (LF) と、直前の改行の変更 (CRLF の片方の削除・CR の挿入) を必ず含める。
        fn ops() -> impl Strategy<Value = Vec<Op>> {
            (
                proptest::collection::vec(op(), 1..14),
                proptest::collection::vec((0..16usize, 0..64usize), 3),
            )
                .prop_map(|(mut v, forced)| {
                    for (i, (at, pick)) in forced.into_iter().enumerate() {
                        let o = match i {
                            0 => Op::AtLineStart {
                                pick,
                                text: "\n".into(),
                            },
                            1 => Op::PrevEol { pick, kind: 0 },
                            _ => Op::PrevEol { pick, kind: 1 },
                        };
                        // 索引を作ってから変える (引き継ぐものがあるように)
                        let at = at.min(v.len());
                        v.insert(at, o);
                        v.insert(at, Op::Look { pick, at: 4096 });
                    }
                    v
                })
        }

        fn long_lines(d: &Document) -> Vec<usize> {
            let t = d.text();
            (0..t.len_lines())
                .filter(|&l| {
                    let l = eve_text::LineIdx(l);
                    t.line_end_byte(l).0 - t.line_to_byte(l).0 > THRESHOLD
                })
                .collect()
        }

        /// `b` 以下の char の境界。
        fn floor_char(t: &dyn TextRead, mut b: usize) -> usize {
            b = b.min(t.len_bytes());
            while !t.is_char_boundary(ByteOff(b)) {
                b -= 1;
            }
            b
        }

        /// 行内の `at` 番目 (行の char の数で丸める) の char の位置 (行頭からのバイト)。
        fn char_pos(t: &dyn TextRead, ls: usize, le: usize, at: usize) -> usize {
            let s = t.slice_to_string(ByteRange::new(ByteOff(ls), ByteOff(le)));
            let n = s.chars().count() + 1;
            s.char_indices()
                .map(|(b, _)| b)
                .chain(std::iter::once(s.len()))
                .nth(at % n)
                .unwrap_or(0)
        }

        fn replace(d: &mut Document, a: usize, b: usize, text: &str, tick: u64) {
            let t = d.text();
            let a = floor_char(t, a);
            let b = floor_char(t, b.max(a));
            let cs = ChangeSet::from_sorted_edits(
                t,
                [Edit {
                    range: ByteRange::new(ByteOff(a), ByteOff(b)),
                    insert: text.into(),
                }],
            )
            .expect("変更");
            if cs.is_identity() {
                return;
            }
            d.apply(&Transaction::new(cs, Origin::User, Tick(tick)))
                .expect("適用");
        }

        fn run(d: &mut Document, op: &Op, tick: u64) {
            let longs = long_lines(d);
            let pick_line = |pick: usize| longs.get(pick % longs.len().max(1)).copied();
            let t = d.text();
            let bounds = |l: usize| {
                let l = eve_text::LineIdx(l);
                (t.line_to_byte(l).0, t.line_end_byte(l).0)
            };
            match op {
                Op::AtLineStart { pick, text } => {
                    if let Some(l) = pick_line(*pick) {
                        let ls = bounds(l).0;
                        replace(d, ls, ls, if text.is_empty() { "x" } else { text }, tick);
                    }
                }
                Op::PrevEol { pick, kind } => {
                    let Some(l) = pick_line(*pick).filter(|&l| l > 0) else {
                        return;
                    };
                    let ls = bounds(l).0;
                    let (_, pe) = bounds(l - 1);
                    match kind {
                        0 => replace(d, ls - 1, ls, "", tick),
                        1 => replace(d, pe, pe + 1, "", tick),
                        2 => replace(d, pe, pe, "\r", tick),
                        _ => replace(d, pe, pe, "q", tick),
                    }
                }
                Op::InLine { pick, at, n, text } => {
                    if let Some(l) = pick_line(*pick) {
                        let (ls, le) = bounds(l);
                        let a = ls + char_pos(t, ls, le, *at);
                        let b = (a..=le)
                            .filter(|&b| t.is_char_boundary(ByteOff(b)))
                            .nth(*n)
                            .unwrap_or(le);
                        replace(d, a, b, text, tick);
                    }
                }
                Op::OwnEol { pick, kind, text } => {
                    if let Some(l) = pick_line(*pick) {
                        let (_, le) = bounds(l);
                        let te = t.line_to_byte(eve_text::LineIdx(l + 1)).0;
                        match kind {
                            0 => replace(d, le, te, "", tick),
                            1 => replace(d, le, le, text, tick),
                            _ => replace(d, te, te, text, tick),
                        }
                    }
                }
                Op::Anywhere { at, n, text } => {
                    let a = *at % (t.len_bytes() + 1);
                    replace(d, a, a + n, text, tick);
                }
                Op::Undo => {
                    d.undo().expect("Undo");
                }
                Op::Redo => {
                    d.redo().expect("Redo");
                }
                Op::Look { pick, at } => {
                    if let Some(l) = pick_line(*pick) {
                        let (ls, le) = bounds(l);
                        let b = char_pos(t, ls, le, *at);
                        let _ = cursor_cell(d, l, b);
                        let _ = long_line_graphemes(d, l, b);
                        let _ = byte_at_col(d, l, *at % 300);
                    }
                }
            }
        }

        /// 写す本文の大きさ (最初, 続き。バイト)。行の長さ (数十〜数百バイト) より小さくして、写した範囲の端をまたぐ書記素を
        /// 多く作る。
        const TEST_BLOCKS_BYTES: (usize, usize) = (9, 23);

        /// 試験の間だけ記録点の間隔と写す本文の大きさを変える (落ちても戻す)。
        struct StepGuard;

        impl StepGuard {
            fn set(step: usize) -> StepGuard {
                TEST_STEP.with(|s| s.set(step));
                TEST_BLOCKS.with(|s| s.set(TEST_BLOCKS_BYTES));
                StepGuard
            }
        }

        impl Drop for StepGuard {
            fn drop(&mut self) {
                TEST_STEP.with(|s| s.set(STEP));
                TEST_BLOCKS.with(|s| s.set((FIRST_BLOCK, BLOCK)));
            }
        }

        /// 覚えている索引 (引き継いだもの) が、今の本文から作り直した索引と同じ答えを返し、行末まで作れば同じ記録点になる。
        fn check(d: &Document) -> Result<(), TestCaseError> {
            let t = d.text();
            let cache = d.layout_cache.borrow();
            for e in cache.long.iter().filter(|e| e.rev == d.revision()) {
                let l = t.byte_to_line(ByteOff(e.ls));
                prop_assert_eq!(t.line_to_byte(l).0, e.ls, "行頭");
                let (ls, le) = (e.ls, t.line_end_byte(l).0);
                prop_assert_eq!(e.ix.len(), le - ls, "行の長さ");
                // 引き継いだ索引は複製して比べる (比べるために作ると、次の操作で途中まで作った状態を試せなくなる)
                let carried = (*e.ix).clone();
                let fresh = ColIndex::with_step(le - ls, e.ix.rule.clone(), TEST_STEP_BYTES);
                let s = t.slice_to_string(ByteRange::new(ByteOff(ls), ByteOff(le)));
                for b in s.char_indices().map(|(b, _)| b).chain([s.len()]) {
                    prop_assert_eq!(
                        carried.at_byte(t, ls, b),
                        fresh.at_byte(t, ls, b),
                        "byte {}",
                        b
                    );
                }
                let width = fresh.end(t, ls).col;
                for c in 0..=width + 1 {
                    prop_assert_eq!(
                        carried.at_col(t, ls, c),
                        fresh.at_col(t, ls, c),
                        "col {}",
                        c
                    );
                }
                prop_assert_eq!(carried.end(t, ls), fresh.end(t, ls));
                prop_assert_eq!(carried.built_state(), fresh.built_state(), "記録点");
            }
            Ok(())
        }

        /// 1 件が重い試験 (編集の列の各段で、覚えている索引の全位置・全列を作り直しと比べる) の件数。既定は 96 件、
        /// `PROPTEST_CASES` があればその 1/20 (夜間の 10000 なら 500)。`proptest!` のマクロは `PROPTEST_CASES` で件数を
        /// 上書きするので、`TestRunner` を直に使う (09 §6.3、Issue #453 と同じ扱い)。
        fn heavy_config() -> ProptestConfig {
            let cases = std::env::var("PROPTEST_CASES")
                .ok()
                .and_then(|v| v.parse::<u32>().ok())
                .map_or(96, |n| (n / 20).max(96));
            ProptestConfig {
                cases,
                source_file: Some(file!()),
                ..ProptestConfig::default()
            }
        }

        #[test]
        fn 差分で引き継いだ索引は作り直した索引と同じ() {
            let _step = StepGuard::set(TEST_STEP_BYTES);
            let mut runner = proptest::test_runner::TestRunner::new(heavy_config());
            let result = runner.run(&(initial_doc(), ops()), |(doc, ops)| {
                let mut fmt = TextFormat::new_document();
                fmt.long_line_threshold = THRESHOLD;
                let mut d = Document::from_text(Text::from(doc.as_str()), fmt);
                // 最初に全部の長大行の索引を途中まで作る
                for l in long_lines(&d) {
                    let _ = byte_at_col(&d, l, 40);
                }
                check(&d)?;
                for (i, op) in ops.iter().enumerate() {
                    run(&mut d, op, u64::try_from(i).unwrap_or(0) * 10_000);
                    check(&d)?;
                }
                Ok(())
            });
            if let Err(e) = result {
                panic!("{e}");
            }
        }

        // ---------- 長大行の経路 = 通常の行の経路 (09 §3.2 の長大行の列の索引の (a)、01 NFR-106 AC-5) ----------

        /// 1 件の入力: 行・表示の設定 (タブ幅・エスケープ・空白の可視化・曖昧幅)・Frame の窓 (左端の列, 幅) の列・
        /// 選択・カーソル・検索の一致。
        #[derive(Clone, Debug)]
        struct Same {
            line: String,
            tab: u8,
            esc: bool,
            ws: bool,
            ambiguous_wide: bool,
            views: Vec<(usize, usize)>,
            sel: (usize, usize),
            heads: Vec<usize>,
            found: (usize, usize),
        }

        fn same_input() -> impl Strategy<Value = Same> {
            (
                proptest::collection::vec(line_char(), 0..160)
                    .prop_map(|v| v.into_iter().collect::<String>()),
                prop_oneof![Just(1u8), Just(2), Just(4), Just(8)],
                any::<bool>(),
                any::<bool>(),
                any::<bool>(),
                proptest::collection::vec((0..240usize, 1..50usize), 1..4),
                (0..720usize, 0..720usize),
                proptest::collection::vec(0..720usize, 0..3),
                (0..720usize, 0..40usize),
            )
                .prop_map(
                    |(line, tab, esc, ws, ambiguous_wide, views, (a, b), heads, (f, n))| Same {
                        line,
                        tab,
                        esc,
                        ws,
                        ambiguous_wide,
                        views,
                        sel: (a.min(b), a.max(b)),
                        heads,
                        found: (f, f + n),
                    },
                )
        }

        /// 行 `line` と次の短い行の文書。`threshold` は長大行の閾値 (0 なら空でない行は長大行、`usize::MAX` なら通常の行)。
        fn doc_of(x: &Same, threshold: usize) -> Document {
            let mut fmt = TextFormat::new_document();
            fmt.long_line_threshold = threshold;
            if !x.esc {
                fmt.pua_escape = eve_encoding::PuaEscapeMode::DisabledCollision;
            }
            let mut d = Document::from_text(Text::from(format!("{}\nnext", x.line).as_str()), fmt);
            d.display.tab_width = x.tab;
            d.display.show_whitespace = x.ws;
            d.display.ambiguous_wide = x.ambiguous_wide;
            d
        }

        /// 行の組み立てのスタイル (区別できるように、ロールごとに違う属性にする)。
        fn styles() -> crate::layout::rows::RowStyles {
            use eve_view::{Style, StyleMods};
            crate::layout::rows::RowStyles {
                text: Style::DEFAULT,
                whitespace: Style::with_mods(StyleMods::DIM),
                control_char: Style::with_mods(StyleMods::ITALIC),
                selection: Style::with_mods(StyleMods::REVERSE),
                search_match: Style::with_mods(StyleMods::UNDERLINE),
                search_current: Style::with_mods(StyleMods::BOLD),
                line_number: Style::DEFAULT,
                line_number_active: Style::DEFAULT,
            }
        }

        fn check_same(x: &Same) -> Result<(), TestCaseError> {
            let long = doc_of(x, 0);
            let normal = doc_of(x, usize::MAX);
            let s = x.line.as_str();
            // 次の行 (`next`) も長大行になる
            prop_assert_eq!(long.format.long_lines, 1 + usize::from(!s.is_empty()));
            prop_assert_eq!(normal.format.long_lines, 0);
            // カーソルのセル (書記素の途中の位置を含む、すべての char の境界で)
            for b in s.char_indices().map(|(b, _)| b).chain([s.len()]) {
                prop_assert_eq!(
                    cursor_cell(&long, 0, b),
                    cursor_cell(&normal, 0, b),
                    "cursor_cell {}",
                    b
                );
            }
            // 列からの位置 (行の幅の先まで)
            let width = cursor_cell(&normal, 0, s.len()).0;
            for c in 0..=width + 2 {
                prop_assert_eq!(
                    byte_at_col(&long, 0, c),
                    byte_at_col(&normal, 0, c),
                    "col {}",
                    c
                );
            }
            // 桁 (行頭からの書記素の数。通常の行はステータスバーと同じく行頭からの文字列を書記素に分けて数える)
            let starts = eve_text::grapheme_indices(s);
            for b in starts.iter().map(|&(b, _)| b).chain([s.len()]) {
                if s.is_empty() {
                    break;
                }
                prop_assert_eq!(
                    long_line_graphemes(&long, 0, b),
                    Some(eve_text::grapheme_indices(&s[..b]).len()),
                    "graphemes {}",
                    b
                );
            }
            prop_assert_eq!(long_line_graphemes(&normal, 0, 0), None);
            // Frame の行 (表示の単位・左右の切れ・幅の危険・選択と検索の強調・行末の改行の選択・カーソルの列)
            let floor = |p: usize| {
                let mut p = p.min(s.len());
                while !s.is_char_boundary(p) {
                    p -= 1;
                }
                p
            };
            let heads: Vec<usize> = {
                let mut h: Vec<usize> = x.heads.iter().map(|&p| floor(p)).collect();
                h.sort_unstable();
                h.dedup();
                h
            };
            let sel = [x.sel];
            let st = styles();
            let found = [x.found];
            let marks = crate::layout::rows::Marks {
                found: &found,
                current: None,
                unmappable: &[],
                styles: &st,
            };
            for &(left, cols) in &x.views {
                let a = crate::layout::rows::build_row(
                    &long,
                    0,
                    left,
                    cols,
                    &sel,
                    &heads,
                    &marks,
                    Vec::new(),
                    None,
                );
                let b = crate::layout::rows::build_row(
                    &normal,
                    0,
                    left,
                    cols,
                    &sel,
                    &heads,
                    &marks,
                    Vec::new(),
                    None,
                );
                prop_assert_eq!(&a.row, &b.row, "left {} cols {}", left, cols);
                prop_assert_eq!(
                    &a.cursor_cols,
                    &b.cursor_cols,
                    "left {} cols {}",
                    left,
                    cols
                );
            }
            Ok(())
        }

        // verifies: AC-NFR-106-5
        #[test]
        fn 長大行の経路は通常の行の経路と表示_カーソル_列_桁が同じ() {
            let _step = StepGuard::set(TEST_STEP_BYTES);
            let mut runner = proptest::test_runner::TestRunner::new(heavy_config());
            let result = runner.run(&same_input(), |x| check_same(&x));
            if let Err(e) = result {
                panic!("{e}");
            }
        }

        /// 決まった行 (結合文字・ZWJ の列・国旗・ハングル・InCB・キーキャップ・エスケープ文字) でも同じ。
        #[test]
        fn 長大行の経路は決まった行でも通常の行の経路と同じ() {
            let _step = StepGuard::set(TEST_STEP_BYTES);
            let lines = [
                "e\u{301}x か\u{3099}き a\u{301}\u{302}\u{303}b\tc",
                "👨\u{200D}👩\u{200D}👧 👍🏽 1\u{FE0F}\u{20E3} ❤\u{FE0F}x",
                "🇯🇵🇺🇸\u{1F1EF}x\u{1F1EF}\u{1F1F5}\u{1F1FA}",
                "\u{1100}\u{1161}\u{11A8} 한국어 \u{AC00}\u{11A8}\u{1100}",
                "\u{0915}\u{094D}\u{0937} नमस्ते \u{0600}a\u{0903}",
                "\u{10FF9A}\u{301} a\u{10FF80}\u{200D}b \u{0600}\u{10FF80}",
                "\u{301}\u{301}x\u{200D}\u{1}\u{85}\u{3000}全角",
            ];
            for line in lines {
                for esc in [true, false] {
                    let x = Same {
                        line: line.to_owned(),
                        tab: 4,
                        esc,
                        ws: true,
                        ambiguous_wide: false,
                        views: vec![(0, 120), (1, 7), (5, 3), (13, 20)],
                        sel: (3, 17),
                        heads: vec![0, 4, 11, line.len()],
                        found: (6, 12),
                    };
                    if let Err(e) = check_same(&x) {
                        panic!("{line:?} esc {esc}: {e}");
                    }
                }
            }
        }
    }
}
