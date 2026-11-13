//! **spike (M25 の 6、`spike/eve-core-long-line-wrap`)**: 長大行の折り返し (03 §14.4 の「長大行の折り返し」の手順 2)。
//!
//! 折り返し ON の長大行では、列の索引の記録点を **表示行の先頭にだけ** 置く (03 §14.3 の表の「長大行」。手前の記録点から
//! `step` 以上進んだ後の最初の表示行の先頭)。表示行の先頭から走査を再開すれば、単語境界・禁則の判定は前の表示行に依存しない
//! (`wrap_line` は表示行ごとに、その行の先頭から決める) ので、記録点から先の表示行は通常の行の `wrap_line` と同じになる。
//! 記録点ごとに表示行の番号と、その表示行の先頭を決めるのに読んだ範囲の末尾 (`horizon`) を持ち、編集では `horizon` が
//! 変更の位置以下の記録点だけを残す (03 §14.4 の「編集」の (3) の折り返し版)。
//!
//! 折り返しの規則は `wrap::wrap_line` を、表示の単位を 1 つずつ受け取る形に書き直したもの (後ろからの走査の代わりに、
//! 折れる位置の候補を前から覚える。結果は同じ)。試験で `wrap_line` と突き合わせる。

use std::collections::VecDeque;

use eve_encoding::unescape_char;
use eve_text::TextRead;

use super::super::wrap::{is_space, no_line_end, no_line_start};
use super::{ColIndex, ColPoint, Flow, Piece, Want, scan};

/// 折り返しの幾何 (覚えの鍵に含める)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WrapGeom {
    /// 折り返し幅 (本文の幅 − 右余白 1)。
    pub cols: usize,
    /// `view.wrap = "word"` (偽なら `soft`)。
    pub word: bool,
    pub indent_same: bool,
    pub kinsoku: bool,
    /// 空白の可視化 (タブ・全角空白が記号か。幅 2 の単位が記号かに効く)。
    pub ws: bool,
}

/// 記録点ごとの (表示行の番号, その表示行の先頭を決めるのに読んだ範囲の末尾)。
#[derive(Clone, Copy, Debug)]
pub(super) struct RowMeta {
    pub row: usize,
    pub horizon: usize,
}

/// 折り返しの索引の状態 (`Built` の中)。
#[derive(Clone, Debug, Default)]
pub(in crate::layout) struct WrapState {
    /// 継続行のずらしと、それを決めるのに読んだ範囲の末尾。
    indent: Option<(usize, usize)>,
    /// `points` と同じ長さ。
    rows: Vec<RowMeta>,
    /// `built_to` (表示行の先頭) の表示行の番号。
    built_row: usize,
    /// 最後の表示行の番号 (行末まで作ったときだけ)。
    end_row: Option<usize>,
}

/// 折り返しの索引をどこまで作るか。
#[derive(Clone, Copy)]
pub(super) enum WWant {
    /// 位置 `b` より後ろの表示行の先頭まで (`b` を含む表示行が決まる)。
    Byte(usize),
    /// 表示列が `c` を越える表示行の先頭まで。
    Col(usize),
    /// 表示行 `r` の次の表示行の先頭まで (`r` の終わりが決まる)。
    Row(usize),
    End,
}

impl WWant {
    pub(super) fn from_want(w: Want) -> WWant {
        match w {
            Want::Byte(b) => WWant::Byte(b),
            Want::Col(c) => WWant::Col(c),
            Want::End => WWant::End,
        }
    }

    fn done(self, at: ColPoint, row: usize) -> bool {
        match self {
            WWant::Byte(b) => at.byte > b,
            WWant::Col(c) => at.col > c,
            WWant::Row(r) => row > r,
            WWant::End => false,
        }
    }
}

/// 表示の単位 1 つ (折り返しの判定に要るものだけ。書記素の数は表示行の先頭からの順番で決まるので持たない)。
#[derive(Clone, Copy)]
struct U {
    byte: usize,
    col: usize,
    len: u16,
    w: u16,
    /// 書記素の最初の char。
    c: char,
    /// 最初の Span が記号か (幅 2 のときだけ意味がある。`Unit::starts_with_symbol`)。
    sym: bool,
}

impl U {
    #[inline]
    fn new(at: ColPoint, len: usize, w: usize, c: char, sym: bool) -> U {
        U {
            byte: at.byte,
            col: at.col,
            len: u16::try_from(len).unwrap_or(u16::MAX),
            w: u16::try_from(w).unwrap_or(u16::MAX),
            c,
            sym,
        }
    }

    #[inline]
    fn end_byte(&self) -> usize {
        self.byte + usize::from(self.len)
    }
}

/// 幅 2 の単位の最初の Span が記号か (`units_with` / `unit_slow` と同じ判定。幅 2 以外は使わない)。
fn sym2(c: char, ws: bool, esc: bool) -> bool {
    match c {
        '\t' | '\u{3000}' => ws,
        c if (c as u32) < 0x20 || (0x7F..=0x9F).contains(&(c as u32)) => true,
        c if esc && unescape_char(c).is_some() => true,
        _ => false,
    }
}

/// 走査の出来事。
enum RowEv {
    /// 表示行 `row` が `at` から始まる (`horizon` はそれを決めるのに読んだ範囲の末尾)。
    Start {
        row: usize,
        at: ColPoint,
        horizon: usize,
    },
    /// 行末 (`last_row` は最後の表示行の番号)。
    End { at: ColPoint, last_row: usize },
}

/// `wrap_line` を、単位を 1 つずつ受け取る形にしたもの。
struct Wrapper {
    g: WrapGeom,
    indent: usize,
    len: usize,
    row: Vec<U>,
    row_no: usize,
    /// 今の表示行の先頭の書記素の数 (行頭から)。
    start_g: usize,
    /// 今の表示行の幅 (継続行のずらしを除く)。
    avail: usize,
    base: usize,
    q: VecDeque<U>,
}

impl Wrapper {
    fn new(g: WrapGeom, indent: usize, len: usize, start: ColPoint, row_no: usize) -> Wrapper {
        let mut w = Wrapper {
            g,
            indent,
            len,
            row: Vec::with_capacity(g.cols + 2),
            row_no,
            start_g: start.graphemes,
            avail: 1,
            base: 0,
            q: VecDeque::new(),
        };
        w.avail = w.calc_avail();
        w
    }

    fn calc_avail(&self) -> usize {
        let ind = if self.row_no == 0 { 0 } else { self.indent };
        self.g.cols.saturating_sub(ind).max(1)
    }

    /// 単位 `u` を足す。表示行の先頭が決まるたびに `f` を呼び、`f` が `false` を返したら止めて `true` を返す。
    #[inline(always)]
    fn feed(&mut self, u: U, f: &mut impl FnMut(RowEv) -> bool) -> bool {
        // よくある場合 (待ちが無く、行の途中で、入る) は、ここだけで済ませる
        let i = self.row.len();
        if i > 0 && self.q.is_empty() && u.col + usize::from(u.w) - self.base <= self.avail {
            self.row.push(u);
            return false;
        }
        self.feed_slow(u, f)
    }

    #[cold]
    #[inline(never)]
    fn feed_slow(&mut self, u: U, f: &mut impl FnMut(RowEv) -> bool) -> bool {
        if self.q.is_empty() {
            match self.place(u) {
                None => return false,
                Some(brk) => {
                    if self.emit(brk, u.end_byte(), f) {
                        return true;
                    }
                }
            }
        } else {
            self.q.push_back(u);
        }
        while let Some(x) = self.q.pop_front() {
            if let Some(brk) = self.place(x)
                && self.emit(brk, x.end_byte(), f)
            {
                return true;
            }
        }
        false
    }

    /// 折った: 次の表示行を始め、`f` に知らせる。`trigger_end` は折る位置を決めた単位の末尾。止めるなら `true`。
    fn emit(
        &mut self,
        start: Option<ColPoint>,
        trigger_end: usize,
        f: &mut impl FnMut(RowEv) -> bool,
    ) -> bool {
        let Some(start) = start else {
            return false;
        };
        self.row.clear();
        self.row_no += 1;
        self.start_g = start.graphemes;
        self.avail = self.calc_avail();
        if !f(RowEv::Start {
            row: self.row_no,
            at: start,
            horizon: trigger_end + 4,
        }) {
            self.q.clear();
            return true;
        }
        false
    }

    /// 位置 b (単位 `prev` と `cur` の間) で折れるか: (禁則を守る, 単語の境界として望ましい)。`wrap_line` の
    /// `kinsoku_ok` と `opportunity` と同じ。
    #[inline]
    fn can_break(&self, prev: &U, cur: &U) -> (bool, bool) {
        if self.g.kinsoku && (no_line_start(cur.c) || no_line_end(prev.c)) {
            return (false, false);
        }
        let pref = !self.g.word
            || is_space(prev.c)
            || (prev.w == 2 && !prev.sym)
            || (cur.w == 2 && !cur.sym);
        (true, pref)
    }

    /// 今の表示行の単位の間 b (`1..hi`、`row[b - 1]` と `row[b]` の間) を後ろから見て、(最も後ろの望ましい位置,
    /// 最も後ろの禁則を守る位置)。望ましい位置が見つかったら止める。
    fn search_row(&self, hi: usize) -> (Option<usize>, Option<usize>) {
        let mut ok = None;
        for b in (1..hi).rev() {
            let (k, p) = self.can_break(&self.row[b - 1], &self.row[b]);
            if k {
                ok.get_or_insert(b);
                if p {
                    return (Some(b), ok);
                }
            }
        }
        (None, ok)
    }

    /// 単位を今の表示行に置く。入らなければ折る位置を決め、残り (折る位置から後ろ) を待ちに戻して、次の表示行の先頭を返す
    /// (`Some(None)` は、行末に達したので次の表示行が無いとき)。折る位置は `wrap_line` と同じく、入らない単位の位置から
    /// 後ろ向きに探す (CJK・絵文字はほぼ 1 回目で見つかる)。
    #[inline]
    fn place(&mut self, u: U) -> Option<Option<ColPoint>> {
        let avail = self.avail;
        let i = self.row.len();
        let w = usize::from(u.w);
        if i == 0 {
            self.base = u.col;
            self.row.push(u);
            if w > avail && !(u.c == ' ' && w <= avail + 1) {
                // 行には最低 1 つ入れる
                let nx = ColPoint {
                    byte: u.end_byte(),
                    col: u.col + w,
                    graphemes: self.start_g + 1,
                };
                return Some((nx.byte < self.len).then_some(nx));
            }
            return None;
        }
        let end = u.col + w - self.base;
        if end > avail && !(u.c == ' ' && end <= avail + 1) {
            let (k, p) = self.can_break(&self.row[i - 1], &u);
            let b = if p {
                i
            } else {
                let (pref, ok) = self.search_row(i);
                pref.or(k.then_some(i)).or(ok).unwrap_or(i)
            };
            // 残り (row[b..] と u) を待ちの先頭に戻す (順を保つ)
            self.q.push_front(u);
            for k in (b..i).rev() {
                self.q.push_front(self.row[k]);
            }
            let start = self.q.front().map(|x| ColPoint {
                byte: x.byte,
                col: x.col,
                graphemes: self.start_g + b,
            });
            self.row.truncate(b);
            return Some(start);
        }
        self.row.push(u);
        None
    }

    /// 印字できる ASCII の連続 `s` (先頭の位置 `at`) を足す。表示行の全体が連続の中にあるときは、単位を 1 つずつ置かずに、
    /// 入る幅と折れる位置を連続の上で直接求める (`place` と同じ結果)。止めるなら `true`。
    #[inline(never)]
    fn feed_ascii(&mut self, s: &[u8], at: ColPoint, f: &mut impl FnMut(RowEv) -> bool) -> bool {
        let n = s.len();
        let mut j = 0;
        let unit = |k: usize| {
            U::new(
                ColPoint {
                    byte: at.byte + k,
                    col: at.col + k,
                    graphemes: 0,
                },
                1,
                1,
                char::from(s[k]),
                false,
            )
        };
        while j < n {
            if !self.q.is_empty() {
                // 待ちがあれば 1 つずつ (起きない。`feed` は待ちを空にして戻る)
                if self.feed(unit(j), f) {
                    return true;
                }
                j += 1;
                continue;
            }
            let m = self.row.len();
            let avail = self.avail;
            let col_j = at.col + j;
            let base = if m == 0 { col_j } else { self.base };
            let used = col_j - base;
            let room = avail.saturating_sub(used);
            let over = if used <= avail && j + room < n && s[j + room] == b' ' {
                room + 1
            } else {
                room
            };
            if j + over >= n {
                // 表示行が連続の外まで続く: 残りを 1 つずつ置く
                for k in j..n {
                    if self.feed(unit(k), f) {
                        return true;
                    }
                }
                return false;
            }
            // 折れる位置の候補 b = m + k を後ろから (k = 0 は直前の単位 row[m - 1] との間。m = 0 なら候補でない)
            let (word, kinsoku) = (self.g.word, self.g.kinsoku);
            let (mut pref, mut ok) = (None, None);
            for k in (1..=over).rev() {
                let pc = s[j + k - 1];
                if !kinsoku || !matches!(pc, b'(' | b'[' | b'{') {
                    ok.get_or_insert(m + k);
                    if !word || pc == b' ' {
                        pref = Some(m + k);
                        break;
                    }
                }
            }
            if pref.is_none() && m > 0 {
                let (k0, p0) = self.can_break(&self.row[m - 1], &unit(j));
                if k0 {
                    ok.get_or_insert(m);
                    if p0 {
                        pref = Some(m);
                    }
                }
                if pref.is_none() {
                    let (sp, so) = self.search_row(m);
                    pref = sp;
                    if ok.is_none() {
                        ok = so;
                    }
                }
            }
            let b = pref.or(ok).unwrap_or(m + over);
            if b < m {
                // 折る位置が連続の前: 1 つずつ置いて同じ判定をさせる
                for k in j..=j + over {
                    if self.feed(unit(k), f) {
                        return true;
                    }
                }
                j += over + 1;
                continue;
            }
            let k = b - m;
            let start = ColPoint {
                byte: at.byte + j + k,
                col: at.col + j + k,
                graphemes: self.start_g + b,
            };
            if self.emit(Some(start), at.byte + j + over + 1, f) {
                return true;
            }
            j += k;
        }
        false
    }
}

impl ColIndex {
    /// 折り返し ON の長大行の索引を、まだ何も走査していない形で作る。
    pub fn new_wrapped(len: usize, rule: super::CharWidth, geom: WrapGeom) -> Self {
        let mut ix = Self::new(len, rule);
        ix.geom = Some(geom);
        ix.built.get_mut().wrap = Some(WrapState::default());
        ix
    }

    /// 走査済みの位置 (覚えの選び方に使う)。
    pub fn built_to_byte(&self) -> usize {
        self.built.borrow().built_to.byte
    }

    /// 継続行のずらし (03 §14.3 の `wrap_indent = same`。行頭の空白の表示幅。幅の 2/3 を超えるなら 0)。
    fn indent(&self, text: &dyn TextRead, ls: usize) -> (usize, usize) {
        if let Some(i) = self.built.borrow().wrap.as_ref().and_then(|w| w.indent) {
            return i;
        }
        let g = self.geom.unwrap_or(WrapGeom {
            cols: 1,
            word: false,
            indent_same: false,
            kinsoku: false,
            ws: false,
        });
        let mut v = (0, 0);
        if g.indent_same {
            let (mut width, mut horizon, mut all_ws) = (0, 0, true);
            self.walk(text, ls, ColPoint::default(), |at, s, w| {
                horizon = at.byte + s.len() + 4;
                if s.chars().next().is_some_and(is_space) {
                    width = at.col + w;
                    width * 3 <= g.cols * 2
                } else {
                    all_ws = false;
                    false
                }
            });
            let indent = if all_ws || width * 3 > g.cols * 2 {
                0
            } else {
                width
            };
            v = (indent, horizon);
        }
        if let Some(w) = self.built.borrow_mut().wrap.as_mut() {
            w.indent = Some(v);
        }
        v
    }

    /// 表示行 `row` の継続行のずらし。
    pub fn row_indent(&self, text: &dyn TextRead, ls: usize, row: usize) -> usize {
        if row == 0 { 0 } else { self.indent(text, ls).0 }
    }

    /// 表示行の先頭 `start` (表示行 `row`) から折り返しを走査し、出来事を `f` に渡す (`f` が `false` で止める)。
    fn rewrap(
        &self,
        text: &dyn TextRead,
        ls: usize,
        start: ColPoint,
        row: usize,
        f: impl FnMut(RowEv) -> bool,
    ) {
        let indent = self.indent(text, ls).0;
        self.rewrap_raw(text, ls, indent, start, row, f);
    }

    /// `rewrap` の本体 (ずらしを渡す。`built` を借りない)。
    fn rewrap_raw(
        &self,
        text: &dyn TextRead,
        ls: usize,
        indent: usize,
        start: ColPoint,
        row: usize,
        mut f: impl FnMut(RowEv) -> bool,
    ) {
        let Some(g) = self.geom else {
            return;
        };
        let esc = self.rule.esc;
        let mut w = Wrapper::new(g, indent, self.len, start, row);
        let mut stopped = false;
        let end = scan(text, ls, start, self.len, &self.rule, |at, piece| {
            let stop = match piece {
                Piece::Ascii(s) => w.feed_ascii(s.as_bytes(), at, &mut f),
                Piece::Char(c, wd) => w.feed(
                    U::new(at, c.len_utf8(), wd, c, wd == 2 && sym2(c, g.ws, esc)),
                    &mut f,
                ),
                Piece::One(s, wd) => {
                    let c = s.chars().next().unwrap_or(' ');
                    w.feed(
                        U::new(at, s.len(), wd, c, wd == 2 && sym2(c, g.ws, esc)),
                        &mut f,
                    )
                }
            };
            if stop {
                stopped = true;
                Flow::Stop(0)
            } else {
                Flow::Go
            }
        });
        if !stopped {
            f(RowEv::End {
                at: end,
                last_row: w.row_no,
            });
        }
    }

    /// 折り返し ON の `extend`: `built_to` (表示行の先頭) から先へ、`want` を満たす表示行の先頭か行末まで走査し、
    /// 記録点 (表示行の先頭) を足す。
    pub(super) fn extend_wrapped(&self, text: &dyn TextRead, ls: usize, want: WWant) {
        let (start, row) = {
            let b = self.built.borrow();
            let Some(ws) = b.wrap.as_ref() else {
                return;
            };
            if b.end.is_some() || want.done(b.built_to, ws.built_row) {
                return;
            }
            (b.built_to, ws.built_row)
        };
        // ずらしを先に決めておく (走査の中で `built` を借りないため)
        let indent = self.indent(text, ls).0;
        let step = self.step;
        let mut b = self.built.borrow_mut();
        let most = self.len / step + 2;
        if b.points.capacity() < most {
            let n = most - b.points.len();
            b.points.reserve_exact(n);
        }
        let super::Built {
            points,
            built_to,
            end,
            wrap,
        } = &mut *b;
        let Some(ws) = wrap.as_mut() else {
            return;
        };
        if ws.rows.is_empty() {
            ws.rows.push(RowMeta { row: 0, horizon: 0 });
        }
        let mut next = points.last().map_or(0, |p| p.byte) + step;
        self.rewrap_raw(text, ls, indent, start, row, |ev| match ev {
            RowEv::Start { row, at, horizon } => {
                if at.byte >= next {
                    points.push(at);
                    ws.rows.push(RowMeta { row, horizon });
                    next = at.byte + step;
                }
                *built_to = at;
                ws.built_row = row;
                !want.done(at, row)
            }
            RowEv::End { at, last_row } => {
                *built_to = at;
                ws.built_row = last_row;
                ws.end_row = Some(last_row);
                *end = Some(at);
                false
            }
        });
    }

    /// 編集の後 (折り返し ON): 変更の位置 `s` に触れない記録点だけを残す (`horizon <= s`)。行頭の空白が変わりうるなら
    /// 行頭だけを残す。
    pub(super) fn keep_before_wrapped(&mut self, s: usize, len: usize) {
        let b = self.built.get_mut();
        if let Some(ws) = b.wrap.as_mut() {
            let reset = ws.indent.is_some_and(|(_, h)| s < h);
            let keep = if reset {
                1
            } else {
                1 + b
                    .points
                    .iter()
                    .zip(&ws.rows)
                    .skip(1)
                    .take_while(|(p, m)| p.byte < s && m.horizon <= s)
                    .count()
            };
            b.points.truncate(keep);
            ws.rows.truncate(keep);
            if reset {
                ws.indent = None;
            }
            b.built_to = b.points.last().copied().unwrap_or_default();
            ws.built_row = ws.rows.last().map_or(0, |m| m.row);
            ws.end_row = None;
        }
        b.end = None;
        self.len = len;
    }

    /// 表示行 `r` 以前で最も近い記録点の (位置, 表示行)。
    fn stored_for_row(&self, r: usize) -> (ColPoint, usize) {
        let b = self.built.borrow();
        let Some(ws) = b.wrap.as_ref() else {
            return (ColPoint::default(), 0);
        };
        if ws.rows.is_empty() {
            return (ColPoint::default(), 0);
        }
        let i = ws.rows.partition_point(|m| m.row <= r);
        let i = i.saturating_sub(1);
        (b.points[i], ws.rows[i].row)
    }

    /// 位置 `byte` より前の記録点の 1 つ手前 (表示行 1 つ前の先頭が要る `locate` のため) の (位置, 表示行)。
    fn stored_for_byte(&self, byte: usize) -> (ColPoint, usize) {
        let b = self.built.borrow();
        let Some(ws) = b.wrap.as_ref() else {
            return (ColPoint::default(), 0);
        };
        if ws.rows.is_empty() {
            return (ColPoint::default(), 0);
        }
        let i = b.points.partition_point(|p| p.byte <= byte);
        let i = i.saturating_sub(2);
        (b.points[i], ws.rows[i].row)
    }

    /// 表示行 `r` があるか (要るところまでだけ作る)。
    pub fn has_row(&self, text: &dyn TextRead, ls: usize, r: usize) -> bool {
        self.extend_wrapped(text, ls, WWant::Row(r));
        let b = self.built.borrow();
        let Some(ws) = b.wrap.as_ref() else {
            return r == 0;
        };
        match ws.end_row {
            Some(e) => r <= e,
            None => r <= ws.built_row,
        }
    }

    /// 表示行の数 (行末まで作る)。
    pub fn row_count(&self, text: &dyn TextRead, ls: usize) -> usize {
        self.extend_wrapped(text, ls, WWant::End);
        self.built
            .borrow()
            .wrap
            .as_ref()
            .and_then(|w| w.end_row)
            .map_or(1, |e| e + 1)
    }

    /// 表示行 `r0` から `n` 本の先頭と、その後ろの境目 (次の表示行の先頭か行末)。戻り値の最後の要素が境目で、
    /// `bool` は境目が行末か。`r0` が無ければ空。
    pub fn row_starts(
        &self,
        text: &dyn TextRead,
        ls: usize,
        r0: usize,
        n: usize,
    ) -> (Vec<ColPoint>, bool) {
        if !self.has_row(text, ls, r0) {
            return (Vec::new(), false);
        }
        let (start, row) = self.stored_for_row(r0);
        let mut out = Vec::with_capacity(n + 1);
        if row >= r0 {
            out.push(start);
        }
        let mut at_end = false;
        if out.len() <= n {
            self.rewrap(text, ls, start, row, |ev| match ev {
                RowEv::Start { row, at, .. } => {
                    if row >= r0 {
                        out.push(at);
                    }
                    out.len() <= n
                }
                RowEv::End { at, .. } => {
                    out.push(at);
                    at_end = true;
                    false
                }
            });
        }
        (out, at_end)
    }

    /// 書記素の境界 `p` を含む表示行: (表示行, その先頭, 1 つ前の表示行の先頭, 次の表示行の先頭または行末)。
    /// 行末 (`p.byte == len`) は最後の表示行。
    fn row_of(
        &self,
        text: &dyn TextRead,
        ls: usize,
        p: ColPoint,
    ) -> (usize, ColPoint, Option<ColPoint>, ColPoint) {
        self.extend_wrapped(text, ls, WWant::Byte(p.byte));
        let (start, row) = self.stored_for_byte(p.byte);
        let mut cur = (row, start);
        let mut prev: Option<ColPoint> = None;
        let mut next = None;
        self.rewrap(text, ls, start, row, |ev| match ev {
            RowEv::Start { row, at, .. } => {
                if at.byte > p.byte {
                    next = Some(at);
                    false
                } else {
                    prev = Some(cur.1);
                    cur = (row, at);
                    true
                }
            }
            RowEv::End { at, .. } => {
                next = Some(at);
                false
            }
        });
        (cur.0, cur.1, prev, next.unwrap_or(cur.1))
    }

    /// 行内の位置 `rel` にあるカーソルの (表示行, 画面上の列)。折り返し位置は `before` (上の表示行の末尾) か。
    /// `wrap::cursor_in_rows` と同じ。
    pub fn locate(&self, text: &dyn TextRead, ls: usize, rel: usize, before: bool) -> (usize, usize) {
        let (p, _) = self.at_byte(text, ls, rel);
        let (r, start, prev, next) = self.row_of(text, ls, p);
        let indent = |r| self.row_indent(text, ls, r);
        if p.byte < self.len && p.byte == start.byte && r > 0 && before {
            let pv = prev.unwrap_or(start);
            return (r - 1, indent(r - 1) + start.col - pv.col);
        }
        if p.byte < self.len {
            (r, indent(r) + p.col - start.col)
        } else {
            (r, indent(r) + next.col - start.col)
        }
    }

    /// 表示行 `r` の画面上の列 `col` の位置と、上の表示行の末尾か (`wrap.rs` の `LineRows::at_col` と同じ)。
    pub fn row_at_col(&self, text: &dyn TextRead, ls: usize, r: usize, col: usize) -> (usize, bool) {
        let (v, at_end) = self.row_starts(text, ls, r, 1);
        let (Some(&start), Some(&bound)) = (v.first(), v.get(1)) else {
            return (self.len, false);
        };
        let want = col.saturating_sub(self.row_indent(text, ls, r)) + start.col;
        if want < bound.col {
            return (self.at_col(text, ls, want).byte, false);
        }
        if at_end {
            (self.len, false)
        } else {
            (bound.byte, true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::wrap::{WrapMode, WrapSettings, wrap_line};
    use super::super::super::{units_with, width_config};
    use super::super::CharWidth;
    use super::*;
    use eve_text::Text;

    /// 通常の行の `wrap_line` の表示行の先頭 (バイト位置) の列。
    fn by_wrap_line(s: &str, g: WrapGeom, esc: bool) -> Vec<usize> {
        let cfg = width_config();
        let us = units_with(s, esc, g.ws, &cfg);
        let set = WrapSettings {
            mode: if g.word { WrapMode::Word } else { WrapMode::Soft },
            indent_same: g.indent_same,
            kinsoku: g.kinsoku,
        };
        wrap_line(s, &us, g.cols, set)
            .iter()
            .map(|r| us.get(r.first).map_or(s.len(), |u| u.byte))
            .collect()
    }

    fn by_index(s: &str, g: WrapGeom, esc: bool, step: usize) -> Vec<usize> {
        let t = Text::from(s);
        let mut ix = ColIndex::with_step(s.len(), CharWidth::new(esc, width_config()), step);
        ix.geom = Some(g);
        ix.built.get_mut().wrap = Some(WrapState::default());
        let n = ix.row_count(&t, 0);
        let (v, _) = ix.row_starts(&t, 0, 0, n);
        v[..n].iter().map(|p| p.byte).collect()
    }

    struct Gen(u64);

    impl Gen {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn pick<T: Copy>(&mut self, v: &[T]) -> T {
            v[(self.next() % v.len() as u64) as usize]
        }

        fn line(&mut self, case: usize) -> String {
            const PIECES: [&str; 26] = [
                "a", "Z", " ", "  ", "(", "あ", "漢", "、", "「", "」", "ー", "e\u{301}", "😀",
                "🇯🇵", "한", "\t", "\u{3000}", "\u{7}", "\u{85}", "abcdefghijklmnopqrst", "x y z ",
                "\u{10FF80}", "the quick brown fox jumps over the lazy dog ",
                "(lorem) [ipsum] {dolor} sit amet, consectetur ", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "日本語のテキスト",
            ];
            let mut line = String::new();
            if case % 3 == 0 {
                line.push_str(self.pick(&["  ", "\t", "    "]));
            }
            for _ in 0..(self.next() % 60) {
                line.push_str(self.pick(&PIECES));
            }
            line
        }

        fn geom(&mut self) -> WrapGeom {
            WrapGeom {
                cols: self.pick(&[1, 2, 3, 5, 8, 13, 40, 119]),
                word: self.next() % 2 == 0,
                indent_same: self.next() % 2 == 0,
                kinsoku: self.next() % 4 != 0,
                ws: self.next() % 2 == 0,
            }
        }
    }

    fn wrapped(s: &str, g: WrapGeom, esc: bool, step: usize) -> ColIndex {
        let mut ix = ColIndex::with_step(s.len(), CharWidth::new(esc, width_config()), step);
        ix.geom = Some(g);
        ix.built.get_mut().wrap = Some(WrapState::default());
        ix
    }

    fn settings(g: WrapGeom) -> WrapSettings {
        WrapSettings {
            mode: if g.word { WrapMode::Word } else { WrapMode::Soft },
            indent_same: g.indent_same,
            kinsoku: g.kinsoku,
        }
    }

    #[test]
    fn 長大行の折り返しは通常の行の_wrap_line_と同じ() {
        let mut r = Gen(0x9E37_79B9_7F4A_7C15);
        for case in 0..4000 {
            let line = r.line(case);
            let g = r.geom();
            let esc = r.next() % 2 == 0;
            let step = r.pick(&[1, 7, 64, 4096]);
            assert_eq!(
                by_index(&line, g, esc, step),
                by_wrap_line(&line, g, esc),
                "{line:?} {g:?} esc {esc} step {step}"
            );
        }
    }

    /// カーソルの表示行と列 (`cursor_in_rows`) と、表示行の列からの位置 (`LineRows::at_col`) が通常の行と同じ。
    #[test]
    fn 長大行の折り返しのカーソルと列からの位置は通常の行と同じ() {
        use super::super::super::wrap::{Affinity, cursor_in_rows};
        let mut r = Gen(0x1234_5678_9ABC_DEF1);
        let cfg = width_config();
        for case in 0..300 {
            let line = r.line(case);
            let g = r.geom();
            let esc = r.next() % 2 == 0;
            let step = r.pick(&[1, 7, 64, 4096]);
            let t = Text::from(line.as_str());
            let ix = wrapped(&line, g, esc, step);
            let us = units_with(&line, esc, g.ws, &cfg);
            let rows = wrap_line(&line, &us, g.cols, settings(g));
            let mut bytes: Vec<usize> = us.iter().map(|u| u.byte).collect();
            bytes.push(line.len());
            for &b in &bytes {
                for (aff, before) in [(Affinity::After, false), (Affinity::Before, true)] {
                    assert_eq!(
                        ix.locate(&t, 0, b, before),
                        cursor_in_rows(&us, &rows, b, aff),
                        "{line:?} {g:?} esc {esc} step {step} byte {b} {aff:?}"
                    );
                }
            }
            for (ri, row) in rows.iter().enumerate() {
                let base = us.get(row.first).map_or(0, |u| u.col);
                for col in 0..g.cols + 3 {
                    let want = col.saturating_sub(row.indent) + base;
                    let normal = us[row.first..row.end]
                        .iter()
                        .find(|u| want < u.col + u.width)
                        .map(|u| (u.byte, false))
                        .unwrap_or_else(|| {
                            if ri + 1 == rows.len() {
                                (line.len(), false)
                            } else {
                                (us.get(row.end).map_or(line.len(), |u| u.byte), true)
                            }
                        });
                    assert_eq!(
                        ix.row_at_col(&t, 0, ri, col),
                        normal,
                        "{line:?} {g:?} esc {esc} step {step} row {ri} col {col}"
                    );
                }
            }
        }
    }

    /// 編集の後に引き継いだ索引 (`keep_before`) が、作り直した索引と同じ表示行を返す (03 §14.4 の「編集」の折り返し版)。
    #[test]
    fn 長大行の折り返しの索引は編集の後も作り直しと同じ() {
        let mut r = Gen(0x0F0F_1234_5555_AAAA);
        let inserts = [" ", "x", "、", "「", "\u{301}", "あ", "  ", "\t", "abc def ", "😀"];
        for case in 0..2500 {
            let line = r.line(case);
            if line.is_empty() {
                continue;
            }
            let g = r.geom();
            let esc = r.next() % 2 == 0;
            let step = r.pick(&[1, 7, 64]);
            let t = Text::from(line.as_str());
            let mut ix = wrapped(&line, g, esc, step);
            // 一部だけ、または全部を作ってから編集する
            if r.next() % 2 == 0 {
                let _ = ix.row_count(&t, 0);
            } else {
                let _ = ix.has_row(&t, 0, (r.next() % 8) as usize);
            }
            let mut s = 1 + (r.next() as usize) % line.len();
            while !line.is_char_boundary(s) {
                s += 1;
            }
            let (new, del) = if r.next() % 3 == 0 && s < line.len() {
                let mut e = s + 1;
                while !line.is_char_boundary(e) {
                    e += 1;
                }
                (format!("{}{}", &line[..s], &line[e..]), true)
            } else {
                let ins = r.pick(&inserts);
                (format!("{}{ins}{}", &line[..s], &line[s..]), false)
            };
            ix.keep_before(s, new.len());
            let t2 = Text::from(new.as_str());
            let n = ix.row_count(&t2, 0);
            let (v, _) = ix.row_starts(&t2, 0, 0, n);
            let carried: Vec<usize> = v[..n].iter().map(|p| p.byte).collect();
            assert_eq!(
                carried,
                by_wrap_line(&new, g, esc),
                "{line:?} -> {new:?} {g:?} esc {esc} step {step} s {s} del {del}"
            );
        }
    }

    /// spike の計測の内訳 (release で `--ignored --nocapture`)。
    #[test]
    #[ignore = "計測用"]
    fn 内訳() {
        for unit in ["日本語のテキスト", "a日😀b", "abcdefghij"] {
            let line = unit.repeat(10 * 1024 * 1024 / unit.len() + 1);
            let t = Text::from(line.as_str());
            let g = WrapGeom { cols: 119, word: true, indent_same: true, kinsoku: true, ws: true };
            for _ in 0..3 {
                let rule = CharWidth::new(true, width_config());
                let t0 = std::time::Instant::now();
                let mut n = 0usize;
                scan(&t, 0, ColPoint::default(), line.len(), &rule, |_, _| {
                    n += 1;
                    Flow::Go
                });
                let d_scan = t0.elapsed();
                let ix = wrapped(&line, g, true, 4096);
                let _ = ix.indent(&t, 0);
                let t1 = std::time::Instant::now();
                let mut rows = 0usize;
                ix.rewrap_raw(&t, 0, 0, ColPoint::default(), 0, |_| {
                    rows += 1;
                    true
                });
                let d_rewrap = t1.elapsed();
                let ix2 = wrapped(&line, g, true, 4096);
                let t2 = std::time::Instant::now();
                let c = ix2.row_count(&t, 0);
                let d_ext = t2.elapsed();
                let plain = ColIndex::new(line.len(), CharWidth::new(true, width_config()));
                let t3 = std::time::Instant::now();
                let e = plain.end(&t, 0);
                let d_plain = t3.elapsed();
                eprintln!(
                    "{unit}: scan {d_scan:?} ({n} pieces) rewrap {d_rewrap:?} ({rows}) extend {d_ext:?} ({c}) plain-end {d_plain:?} ({})",
                    e.graphemes
                );
            }
        }
    }
}
