//! 対応括弧 (docs/03 §5.7 の BRK-1〜BRK-6、01 FR-308)。文字ベースで、同じ組の括弧だけを数えて対応を探す。
//!
//! - `find_pair`: 主の Range の `head` の隣の括弧 (BRK-2) と、その対応 (BRK-3。探す範囲は 1 MiB まで、BRK-4)。
//! - `cursor.goto_bracket` (BRK-6): 各 Range を対応の括弧の直前へ移す (Motion)。
//! - 強調 (BRK-5): Frame を作る前に、表示中の各ペインの View について主の Range の括弧と対応を求め、View に
//!   `(Revision, 長さ, head, 探す範囲)` を鍵に 1 つ覚える (`EditorState::sync_bracket_marks`)。Frame の生成はそれを読むだけ。
//!
//! 走査はチャンクごとに組の 2 つのコードポイントだけを探す (ASCII は `memchr2`、全角は UTF-8 の先頭 2 バイトの並びを
//! `memmem` で探して 3 バイト目を見る。10 組の全角の括弧は、開きと閉じで先頭の 2 バイトが同じ)。
//!
//! spec: 03 §5.7, 03 §4.5.2, 03 §4.7, 01 FR-308

use eve_text::{
    Bias, ByteOff, ByteRange, Range, Selection, TextRead, prev_grapheme_boundary, snap_to_grapheme,
};
use eve_view::{CellSize, MessageLevel, Revision, ViewId, WindowId};
use memchr::memmem;

use crate::command::{
    CommandCtx, CommandKind, CommandRegistry, CommandResult, Permission, RejectReason, ids,
};
use crate::editor::state::EditorState;
use crate::ui_model::keys;
use crate::view::motion;

/// 対応を探す範囲の上限 (バイト、03 §5.7 の BRK-4)。対象の括弧から探す向きに数える。
pub(crate) const BRACKET_SCAN_MAX_BYTES: usize = 1 << 20;

/// `cursor.goto_bracket` で、全 Range の走査で読む量の合計の上限 (バイト、03 §5.7 の BRK-6 の細目)。主の Range を先に
/// 探し、読んだ量の合計がこれに達したら、残りの Range は探さずに動かさない (1 万個のカーソルの全部が対応の無い括弧の隣に
/// あるときに、1 MiB を 1 万回読まない)。
pub(crate) const GOTO_SCAN_TOTAL_MAX_BYTES: usize = 8 << 20;

/// 対象の組 (03 §5.7 の BRK-1。定数。開き, 閉じ)。
const PAIRS: [(char, char); 10] = [
    ('(', ')'),
    ('[', ']'),
    ('{', '}'),
    ('（', '）'),
    ('［', '］'),
    ('｛', '｝'),
    ('「', '」'),
    ('『', '』'),
    ('【', '】'),
    ('〔', '〕'),
];

/// 括弧の組の番号と、開き括弧か。組の括弧でなければ `None`。
fn classify(c: char) -> Option<(usize, bool)> {
    PAIRS.iter().enumerate().find_map(|(i, &(o, cl))| {
        if c == o {
            Some((i, true))
        } else if c == cl {
            Some((i, false))
        } else {
            None
        }
    })
}

/// 探した結果 (03 §5.7 の BRK-2〜BRK-4)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lookup {
    /// `head` の隣に括弧が無い (`info.bracket.none`)。
    NoBracket,
    /// 文書の端まで探して対応が無い (`info.bracket.no_match`)。
    NoMatch,
    /// 探す範囲 (1 MiB) の端までに対応が無い (`info.bracket.too_far`)。
    TooFar,
    /// 対象の括弧 (`target`) と対応の括弧 (`pair`)。どちらもコードポイント 1 つの範囲。
    Found { target: ByteRange, pair: ByteRange },
}

/// `p` から始まる文字 (末尾なら `None`)。チャンクの中で読む (確保しない)。
fn char_at(text: &dyn TextRead, p: ByteOff) -> Option<char> {
    if p.0 >= text.len_bytes() {
        return None;
    }
    let (chunk, start) = text.chunk_at(p);
    chunk.get(p.0.checked_sub(start.0)?..)?.chars().next()
}

/// 対象の括弧 (03 §5.7 の BRK-2): `head` の直後の書記素の最初のコードポイントが組の括弧ならそれ、そうでなければ直前の
/// 書記素の最初のコードポイントで同じく見る。(範囲, 組の番号, 開きか)。
fn target_at(text: &dyn TextRead, head: ByteOff) -> Option<(ByteRange, usize, bool)> {
    let at = |p: ByteOff| {
        let c = char_at(text, p)?;
        let (pair, open) = classify(c)?;
        Some((ByteRange::new(p, ByteOff(p.0 + c.len_utf8())), pair, open))
    };
    at(head).or_else(|| {
        (head.0 > 0)
            .then(|| at(prev_grapheme_boundary(text, head)))
            .flatten()
    })
}

/// 組の括弧の UTF-8 の並び (開きと閉じで、最後のバイトだけが違う)。
struct Pattern {
    /// 共通の先頭の置き場 (ASCII は 0 バイト、全角は 2 バイトを使う。10 組の括弧は 3 バイト以下)。
    head: [u8; 2],
    /// `head` のうち使うバイト数。
    head_len: usize,
    open: u8,
    close: u8,
}

impl Pattern {
    fn new(pair: usize) -> Pattern {
        let (o, c) = PAIRS[pair];
        let mut ob = [0u8; 4];
        let mut cb = [0u8; 4];
        let (o, c) = (
            o.encode_utf8(&mut ob).as_bytes(),
            c.encode_utf8(&mut cb).as_bytes(),
        );
        let n = o.len() - 1;
        assert!(
            o.len() == c.len() && n <= 2 && o[..n] == c[..n],
            "組の括弧は 3 バイト以下で、最後のバイトだけが違う: {o:?} {c:?}"
        );
        let mut head = [0u8; 2];
        head[..n].copy_from_slice(&o[..n]);
        Pattern {
            head,
            head_len: n,
            open: o[n],
            close: c[n],
        }
    }

    /// 共通の先頭 (ASCII は空)。
    fn prefix(&self) -> &[u8] {
        &self.head[..self.head_len]
    }

    fn len(&self) -> usize {
        self.head_len + 1
    }
}

/// `hay` の中の括弧を、前向き (`forward`) なら先頭から、後ろ向きなら末尾から順に `f(位置, 開きか)` へ渡す。`f` が真を
/// 返したら止めて、その位置を返す。括弧の全体が `hay` に入るものだけを数える (`hay` の両端で切れた文字は数えない)。
fn visit(
    pat: &Pattern,
    finders: &(memmem::Finder<'_>, memmem::FinderRev<'_>),
    hay: &[u8],
    forward: bool,
    mut f: impl FnMut(bool) -> bool,
) -> Option<usize> {
    if pat.head_len == 0 {
        let mut hit = |i: usize| f(hay[i] == pat.open).then_some(i);
        return if forward {
            memchr::memchr2_iter(pat.open, pat.close, hay).find_map(&mut hit)
        } else {
            memchr::memchr2_iter(pat.open, pat.close, hay)
                .rev()
                .find_map(&mut hit)
        };
    }
    let last = pat.head_len;
    let mut hit = |i: usize| match hay.get(i + last) {
        Some(&b) if b == pat.open || b == pat.close => f(b == pat.open).then_some(i),
        _ => None,
    };
    if forward {
        finders.0.find_iter(hay).find_map(&mut hit)
    } else {
        finders.1.rfind_iter(hay).find_map(&mut hit)
    }
}

/// 括弧の対応を `window` の中だけで探す (03 §5.7 の BRK-3)。`from` は探し始める位置 (対象の括弧の、前向きなら終端、後ろ向き
/// なら始端) で、そこでの深さを 1 とする。前向き (開き括弧) は `window.end` まで、後ろ向き (閉じ括弧) は `window.start` まで。
/// (対応の範囲, 読んだバイト数)。
fn scan(
    text: &dyn TextRead,
    from: ByteOff,
    pair: usize,
    open: bool,
    window: ByteRange,
) -> (Option<ByteRange>, usize) {
    let pat = Pattern::new(pair);
    let finders = (
        memmem::Finder::new(pat.prefix()),
        memmem::FinderRev::new(pat.prefix()),
    );
    let n = pat.len();
    // 対象と同じ向きの括弧で深くなり、逆の向きで浅くなる。0 に戻った括弧が対応
    let mut depth = 1usize;
    let mut step = |is_open: bool| {
        if is_open == open {
            depth += 1;
        } else {
            depth -= 1;
        }
        depth == 0
    };
    let mut read = 0;
    if open {
        let (mut pos, hi) = (from.0, window.end.0);
        while pos < hi {
            let (chunk, cs) = text.chunk_at(ByteOff(pos));
            let bytes = chunk.as_bytes();
            let end = (cs.0 + bytes.len()).min(hi);
            if end <= pos {
                break;
            }
            let hay = &bytes[pos - cs.0..end - cs.0];
            read += hay.len();
            if let Some(i) = visit(&pat, &finders, hay, true, &mut step) {
                let s = pos + i;
                return (Some(ByteRange::new(ByteOff(s), ByteOff(s + n))), read);
            }
            pos = end;
        }
    } else {
        let (mut pos, lo) = (from.0, window.start.0);
        while pos > lo {
            let (chunk, cs) = text.chunk_at(ByteOff(pos - 1));
            let bytes = chunk.as_bytes();
            let begin = cs.0.max(lo);
            if begin >= pos {
                break;
            }
            let hay = &bytes[begin - cs.0..pos - cs.0];
            read += hay.len();
            if let Some(i) = visit(&pat, &finders, hay, false, &mut step) {
                let s = begin + i;
                return (Some(ByteRange::new(ByteOff(s), ByteOff(s + n))), read);
            }
            pos = begin;
        }
    }
    (None, read)
}

/// `head` の隣の括弧とその対応を探す (03 §5.7 の BRK-2〜BRK-4)。`scope` を与えると、探す範囲をその中に限る (フォーカスの
/// 無いペインの可視範囲、BRK-5)。対象の括弧が `scope` の外なら対象なしとする。(結果, 読んだバイト数)。
pub(crate) fn find_pair(
    text: &dyn TextRead,
    head: ByteOff,
    scope: Option<ByteRange>,
) -> (Lookup, usize) {
    find_pair_within(text, head, scope, BRACKET_SCAN_MAX_BYTES)
}

/// `find_pair` の本体。探す範囲の上限 (`limit` バイト) を引数にする (試験は小さな上限で窓の端を確かめる)。
fn find_pair_within(
    text: &dyn TextRead,
    head: ByteOff,
    scope: Option<ByteRange>,
    limit: usize,
) -> (Lookup, usize) {
    let Some((target, pair, open)) = target_at(text, head) else {
        return (Lookup::NoBracket, 0);
    };
    let len = text.len_bytes();
    let mut window = if open {
        ByteRange::new(
            target.end,
            ByteOff(target.end.0.saturating_add(limit).min(len)),
        )
    } else {
        ByteRange::new(ByteOff(target.start.0.saturating_sub(limit)), target.start)
    };
    if let Some(s) = scope {
        if target.start < s.start || target.end > s.end {
            return (Lookup::NoBracket, 0);
        }
        window = ByteRange::new(window.start.max(s.start), window.end.min(s.end));
    }
    let from = if open { target.end } else { target.start };
    let (found, read) = scan(text, from, pair, open, window);
    let lookup = match found {
        Some(p) => Lookup::Found { target, pair: p },
        // 文書の端まで探したなら対応なし、上限で止めたなら範囲を超えた
        None if (open && window.end.0 == len) || (!open && window.start.0 == 0) => Lookup::NoMatch,
        None => Lookup::TooFar,
    };
    (lookup, read)
}

/// 組の括弧になり得るバイト (ASCII の 6 つの括弧と、全角の括弧の UTF-8 の先頭バイト `E3` / `EF`)。
const BRACKET_LEAD: [bool; 256] = {
    let mut t = [false; 256];
    let mut i = 0;
    while i < PAIRS.len() {
        let (o, c) = PAIRS[i];
        let mut ob = [0u8; 4];
        let mut cb = [0u8; 4];
        t[o.encode_utf8(&mut ob).as_bytes()[0] as usize] = true;
        t[c.encode_utf8(&mut cb).as_bytes()[0] as usize] = true;
        i += 1;
    }
    t
};

/// 全角の組の括弧の UTF-8 の先頭 2 バイト (`（）［］｛｝` は `EF BC` / `EF BD`、`「」『』【】〔〕` は `E3 80`)。前へ探す走査は、
/// 先頭バイトが `E3` / `EF` の文字のうち、この 2 バイトで始まるものだけを読み直す (かな・漢字を 1 文字ずつ復号しない)。
const WIDE_PREFIXES: [[u8; 2]; 3] = [[0xE3, 0x80], [0xEF, 0xBC], [0xEF, 0xBD]];

/// `hay[i..]` から始まる組の括弧 (組の番号, 開きか, バイト数)。括弧の全体が `hay` に入るものだけ。
fn bracket_at(hay: &[u8], i: usize) -> Option<(usize, bool, usize)> {
    let b = *hay.get(i)?;
    if b.is_ascii() {
        let (pair, open) = classify(char::from(b))?;
        return Some((pair, open, 1));
    }
    let s = hay.get(i..i + 3)?;
    if !WIDE_PREFIXES.contains(&[s[0], s[1]]) {
        return None;
    }
    let c = std::str::from_utf8(s).ok()?.chars().next()?;
    let (pair, open) = classify(c)?;
    Some((pair, open, 3))
}

/// 範囲 `r` を囲む、対応のある括弧の組 (03 §5.7 の SEL-2 の (2) / (3))。`r.start` にある開き括弧 (始まりが `r.start` 以前。
/// 対応の閉じ括弧の終わりが `r.end` より後ろのときだけ) と、`r.start` から前へ組ごとに深さを数えて (BRK-3) まだ閉じていない
/// 開き括弧を近い順に取り、その対応の閉じ括弧 (BRK-3。開き括弧から 1 MiB まで、BRK-4) の終わりが `r.end` 以降にある最初の
/// 組 (内側から外側へ。開き括弧・閉じ括弧を含む Range も、その組に囲まれる) を返す。前へ探すのは `r.start` から 1 MiB まで。
/// 範囲を超えた組は候補にしない。
///
/// 読んだ量が `budget` に達したら、それより後の対応は探さずに `None` (03 §5.7 の SEL-5)。((開き, 閉じ), 読んだバイト数)。
pub(crate) fn enclosing_pair(
    text: &dyn TextRead,
    r: ByteRange,
    budget: usize,
) -> (Option<(ByteRange, ByteRange)>, usize) {
    enclosing_pair_within(text, r, BRACKET_SCAN_MAX_BYTES, budget)
}

/// `enclosing_pair` の本体。探す範囲の上限 (`limit` バイト。前へ探す量と、対応を探す量の両方) を引数にする (試験は小さな
/// 上限で窓の端を確かめる)。
///
/// 同じ組の外側の開き括弧の対応は、内側の開き括弧の対応の後ろから深さ 1 で探し続ける (間の同じ組の括弧は釣り合っている
/// ので、開き括弧から BRK-3 で探すのと同じ結果になり、同じところを読み直さない)。内側の開き括弧に範囲の中の対応が無ければ、
/// 同じ組の外側の開き括弧にも無い (内側の対応より後ろでしか閉じない) ので、その組はそれ以上探さない。
fn enclosing_pair_within(
    text: &dyn TextRead,
    r: ByteRange,
    limit: usize,
    budget: usize,
) -> (Option<(ByteRange, ByteRange)>, usize) {
    let mut st = Enclosing {
        text,
        r,
        limit,
        budget,
        resume: [None; PAIRS.len()],
        dead: [false; PAIRS.len()],
        read: 0,
    };
    // Range の始まりにある開き括弧 (`(` を選んだ Range・`|(` のカーソル) は、最も内側の候補。その組は Range を真に囲むとき
    // (対応の閉じ括弧の終わりが Range の終わりより後ろ) だけ取る
    if let Some(c) = char_at(text, r.start)
        && let Some((pair, true)) = classify(c)
    {
        let o = ByteRange::new(r.start, ByteOff(r.start.0 + c.len_utf8()));
        st.read += o.end.0 - o.start.0;
        match st.try_open(o, pair, true) {
            Step::Found(o, c) => return (Some((o, c)), st.read),
            Step::Stop => return (None, st.read),
            Step::Next => {}
        }
    }
    let lo = r.start.0.saturating_sub(limit);
    // 組ごとの、後ろ向きに数えた閉じ括弧の深さ
    let mut depth = [0usize; PAIRS.len()];
    let mut pos = r.start.0;
    while pos > lo {
        let (chunk, cs) = text.chunk_at(ByteOff(pos - 1));
        let bytes = chunk.as_bytes();
        let begin = cs.0.max(lo);
        if begin >= pos {
            break;
        }
        let hay = &bytes[begin - cs.0..pos - cs.0];
        st.read += hay.len();
        let mut end = hay.len();
        while let Some(i) = hay[..end]
            .iter()
            .rposition(|&b| BRACKET_LEAD[usize::from(b)])
        {
            end = i;
            let Some((pair, open, n)) = bracket_at(hay, i) else {
                continue;
            };
            if !open {
                depth[pair] += 1;
                continue;
            }
            if depth[pair] > 0 {
                depth[pair] -= 1;
                continue;
            }
            let o = ByteRange::new(ByteOff(begin + i), ByteOff(begin + i + n));
            match st.try_open(o, pair, false) {
                Step::Found(o, c) => return (Some((o, c)), st.read),
                Step::Stop => return (None, st.read),
                Step::Next => {}
            }
        }
        pos = begin;
    }
    (None, st.read)
}

/// `enclosing_pair_within` の、閉じていない開き括弧を 1 つ調べた結果。
enum Step {
    /// Range を囲む組。
    Found(ByteRange, ByteRange),
    /// 囲まない (対応が Range の中・範囲の外)。外側を探し続ける。
    Next,
    /// 読む量の予算を使い切った。
    Stop,
}

/// `enclosing_pair_within` の状態: 組ごとの、対応を探し続ける位置とそれ以上探さない印、読んだ量。
struct Enclosing<'t> {
    text: &'t dyn TextRead,
    r: ByteRange,
    limit: usize,
    budget: usize,
    resume: [Option<ByteOff>; PAIRS.len()],
    dead: [bool; PAIRS.len()],
    read: usize,
}

impl Enclosing<'_> {
    /// 閉じていない開き括弧 `o` (組 `pair`) の対応を探し、Range を囲むか調べる。`at_start` は Range の始まりにある開き括弧
    /// (囲むのは閉じ括弧の終わりが Range の終わりより後ろのとき。それ以外は、閉じ括弧の終わりが Range の終わり以降のとき)。
    fn try_open(&mut self, o: ByteRange, pair: usize, at_start: bool) -> Step {
        if self.dead[pair] {
            return Step::Next;
        }
        if self.read >= self.budget {
            return Step::Stop;
        }
        let len = self.text.len_bytes();
        let bound = ByteOff(o.end.0.saturating_add(self.limit).min(len));
        let from = self.resume[pair].unwrap_or(o.end);
        let (found, k) = if from < bound {
            scan(self.text, from, pair, true, ByteRange::new(o.end, bound))
        } else {
            (None, 0)
        };
        self.read += k;
        match found {
            Some(c) if c.end > self.r.end || (!at_start && c.end >= self.r.end) => {
                Step::Found(o, c)
            }
            Some(c) => {
                self.resume[pair] = Some(c.end);
                Step::Next
            }
            None => {
                self.dead[pair] = true;
                Step::Next
            }
        }
    }
}

/// 括弧の位置のセル (書記素の始まり)。括弧のコードポイントは、ふつうは書記素の始まりにある。前の文字に結合する
/// (Prepend の後ろなど) ときは、それを含む書記素の始まりへ寄せる。
fn cell_of(text: &dyn TextRead, r: ByteRange) -> ByteOff {
    snap_to_grapheme(text, r.start, Bias::Before)
}

/// 1 セルあたりに数える最大のバイト数 (フォーカスの無いペインの探す範囲の大きさの目安、03 §5.7 の BRK-5)。UTF-8 の 1 文字は
/// 4 バイトまで。
const SCOPE_BYTES_PER_CELL: usize = 4;

/// フォーカスの無いペインの探す範囲 (03 §5.7 の BRK-5 の細目): 先頭の行の始端から、行数ぶん下の行の終端まで。ただし長い行でも
/// 可視範囲の程度に収めるため、始端から `行数 × 桁数 × 4` バイトまで (書記素の境界へ前に寄せる) に切り詰める。
fn visible_scope(text: &dyn TextRead, top: ByteOff, rows: u16, cols: u16) -> ByteRange {
    let top_line = text.byte_to_line(top);
    let last = eve_text::LineIdx(
        (top_line.0 + usize::from(rows).saturating_sub(1)).min(text.len_lines().saturating_sub(1)),
    );
    let start = text.line_to_byte(top_line);
    let cap = usize::from(rows)
        .saturating_mul(usize::from(cols))
        .saturating_mul(SCOPE_BYTES_PER_CELL);
    let end = text.line_end_byte(last);
    let end = if end.0 - start.0 > cap {
        snap_to_grapheme(text, ByteOff(start.0 + cap), Bias::Before)
    } else {
        end
    };
    ByteRange::new(start, end)
}

/// View に覚える対応括弧の強調 (03 §5.7 の BRK-5)。鍵が今と同じ間は探し直さない。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BracketMark {
    revision: Revision,
    len: usize,
    head: ByteOff,
    scope: Option<ByteRange>,
    /// 強調する 2 つのセルの位置 (文書のバイト位置。書記素の始まり)。対応が無ければ `None`。
    cells: Option<[usize; 2]>,
}

impl EditorState {
    /// Window `window` に表示中の各ペインの View について、対応括弧の強調を求め直す (03 §5.7 の BRK-5)。フォーカスの無い
    /// ペインは、探す範囲をそのペインの可視範囲に限る。Frame を作る前に呼ぶ。
    pub(crate) fn sync_bracket_marks(&mut self, window: WindowId) {
        let Some(w) = self.window(window) else {
            return;
        };
        let (cols, rows) = self.viewport();
        let panes: Vec<(ViewId, Option<(u16, u16)>)> = w
            .layout(CellSize { cols, rows })
            .leaves
            .iter()
            .filter_map(|l| {
                let p = w.panes.get(&l.pane)?;
                Some((
                    p.view,
                    (l.pane != w.focus).then_some((l.body.rows, l.body.cols)),
                ))
            })
            .collect();
        for (view, unfocused) in panes {
            self.sync_bracket_mark(view, unfocused);
        }
    }

    /// `unfocused` はフォーカスの無いペインの本文の (行数, 桁数)。フォーカスのペインは `None`。
    fn sync_bracket_mark(&mut self, view: ViewId, unfocused: Option<(u16, u16)>) {
        let Some(v) = self.docs.views.get(&view) else {
            return;
        };
        let Some(d) = self.docs.documents.get(&v.doc) else {
            return;
        };
        let t = d.text();
        let head = v.selection.primary().head;
        let scope = unfocused.map(|(rows, cols)| visible_scope(t, v.scroll.top, rows, cols));
        let (revision, len) = (d.revision(), t.len_bytes());
        let fresh = v.bracket.as_ref().is_some_and(|m| {
            m.revision == revision && m.len == len && m.head == head && m.scope == scope
        });
        if fresh {
            return;
        }
        let cells = match find_pair(t, head, scope).0 {
            Lookup::Found { target, pair } => Some([cell_of(t, target).0, cell_of(t, pair).0]),
            _ => None,
        };
        let mark = BracketMark {
            revision,
            len,
            head,
            scope,
            cells,
        };
        if let Some(v) = self.docs.views.get_mut(&view) {
            v.bracket = Some(mark);
        }
    }

    /// View `view` の対応括弧の強調の 2 つのセル (文書のバイト位置)。覚えた鍵が今の文書と主のカーソルに合わなければ `None`。
    pub(crate) fn bracket_cells(&self, view: ViewId) -> Option<[usize; 2]> {
        let v = self.docs.views.get(&view)?;
        let d = self.docs.documents.get(&v.doc)?;
        v.bracket
            .as_ref()
            .filter(|m| {
                m.revision == d.revision()
                    && m.len == d.text().len_bytes()
                    && m.head == v.selection.primary().head
            })
            .and_then(|m| m.cells)
    }

    /// `cursor.goto_bracket` の本体 (03 §5.7 の BRK-6)。
    fn goto_bracket(&mut self, view_id: ViewId) -> CommandResult {
        let Some(v) = self.docs.views.get(&view_id) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let Some(d) = self.docs.documents.get(&v.doc) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        let t = d.text();
        let primary = v.selection.primary_index();
        let mut out: Vec<Range> = v.selection.ranges().to_vec();
        let mut moved = false;
        let mut primary_lookup = Lookup::NoBracket;
        let mut read_total = 0usize;
        // 主の Range を先に探し、残りは文書の順。読んだ量の合計が上限に達したら、残りは動かさない
        let order = std::iter::once(primary).chain((0..out.len()).filter(|&i| i != primary));
        for i in order {
            if read_total >= GOTO_SCAN_TOTAL_MAX_BYTES {
                break;
            }
            let (lookup, read) = find_pair(t, out[i].head, None);
            read_total += read;
            if let Lookup::Found { pair, .. } = lookup {
                out[i] = Range::point(cell_of(t, pair));
                moved = true;
            }
            if i == primary {
                primary_lookup = lookup;
            }
        }
        if moved && let Some(sel) = Selection::new(out, primary) {
            // 選択を解き、目標の列を消す (03 §5.2 の水平移動と同じ)。同じ位置になった Range は併合する
            self.set_selection(view_id, sel);
            self.ensure_cursor_visible(view_id);
        }
        let key = match primary_lookup {
            Lookup::Found { .. } => return CommandResult::Done,
            Lookup::NoBracket => keys::INFO_BRACKET_NONE,
            Lookup::NoMatch => keys::INFO_BRACKET_NO_MATCH,
            Lookup::TooFar => keys::INFO_BRACKET_TOO_FAR,
        };
        let limit = format!("{} MiB", BRACKET_SCAN_MAX_BYTES >> 20);
        self.show_message(MessageLevel::Info, key, &[("limit", &limit)]);
        CommandResult::Done
    }
}

fn goto_bracket(ctx: &mut CommandCtx<'_>) -> CommandResult {
    match motion::active(ctx) {
        Some((view, _)) => ctx.state.goto_bracket(view),
        None => CommandResult::Rejected(RejectReason::NoTarget),
    }
}

/// v0.5 の M32 の対応括弧のコマンド (07 §11.6 の「複数カーソルと対応括弧」。Motion・`read`・`when` なし)。
pub(crate) fn register(r: &mut CommandRegistry) {
    r.add(
        ids::CURSOR_GOTO_BRACKET,
        CommandKind::Motion,
        Permission::Read,
        "",
        goto_bracket,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use eve_text::Text;
    use proptest::prelude::*;

    fn br(a: usize, b: usize) -> ByteRange {
        ByteRange::new(ByteOff(a), ByteOff(b))
    }

    fn lookup(s: &str, head: usize) -> Lookup {
        find_pair(&Text::from(s), ByteOff(head), None).0
    }

    #[test]
    fn 組の括弧は最後のバイトだけが違う() {
        for (i, pair) in PAIRS.iter().enumerate() {
            let p = Pattern::new(i);
            assert!(p.prefix().is_empty() || p.prefix().len() == 2, "{pair:?}");
            assert_ne!(p.open, p.close);
        }
    }

    #[test]
    fn 直後を優先し_括弧でなければ直前を見る() {
        // `(a)` の直前 (0) は `(`、`)` の直後 (3) は直前の `)`
        assert_eq!(
            lookup("(a)", 0),
            Lookup::Found {
                target: br(0, 1),
                pair: br(2, 3)
            }
        );
        assert_eq!(
            lookup("(a)", 3),
            Lookup::Found {
                target: br(2, 3),
                pair: br(0, 1)
            }
        );
        // `)(` の間 (1) は直後の `(` を優先する (対応なし)
        assert_eq!(lookup(")(", 1), Lookup::NoMatch);
        assert_eq!(lookup("abc", 1), Lookup::NoBracket);
        assert_eq!(lookup("", 0), Lookup::NoBracket);
    }

    #[test]
    fn 同じ組だけを数え_ほかの組の交差を無視する() {
        // `「(」)` の `(` の対応は `)`
        let s = "「(」)";
        assert_eq!(
            lookup(s, 3),
            Lookup::Found {
                target: br(3, 4),
                pair: br(7, 8)
            }
        );
        // 入れ子
        assert_eq!(
            lookup("((a)(b))", 0),
            Lookup::Found {
                target: br(0, 1),
                pair: br(7, 8)
            }
        );
        // 全角
        let s = "【あ【い】】";
        assert_eq!(
            lookup(s, 0),
            Lookup::Found {
                target: br(0, 3),
                pair: br(15, 18)
            }
        );
    }

    #[test]
    fn 探す範囲は_1_mib_で対応の括弧の全体が入るときだけ() {
        // 前向き: 括弧の終端から 1 MiB ちょうどの所で終わる `)` は対応、1 バイト外は範囲を超えた
        let inside = format!("({})", "a".repeat(BRACKET_SCAN_MAX_BYTES - 1));
        assert!(matches!(lookup(&inside, 0), Lookup::Found { .. }));
        let outside = format!("({})", "a".repeat(BRACKET_SCAN_MAX_BYTES));
        assert_eq!(lookup(&outside, 0), Lookup::TooFar);
        // 全角の閉じ括弧 (3 バイト) は、全体が範囲に入るときだけ
        let fw_in = format!("（{}）", "a".repeat(BRACKET_SCAN_MAX_BYTES - 3));
        assert!(matches!(lookup(&fw_in, 0), Lookup::Found { .. }));
        let fw_out = format!("（{}）", "a".repeat(BRACKET_SCAN_MAX_BYTES - 2));
        assert_eq!(lookup(&fw_out, 0), Lookup::TooFar);
        // 後ろ向き
        let back = format!("({})", "a".repeat(BRACKET_SCAN_MAX_BYTES - 1));
        assert!(matches!(lookup(&back, back.len()), Lookup::Found { .. }));
        let back = format!("x({})", "a".repeat(BRACKET_SCAN_MAX_BYTES));
        assert_eq!(lookup(&back, back.len()), Lookup::TooFar);
        // 文書の端に届けば対応なし
        let short = format!("({}", "a".repeat(10));
        assert_eq!(lookup(&short, 0), Lookup::NoMatch);
    }

    /// 素朴な参照の対象の括弧 (03 §5.7 の BRK-2 を独立に書いたもの): 文字列を書記素に分け、`head` から始まる書記素の
    /// 最初のコードポイントが組の括弧ならそれ、無ければ `head` で終わる書記素の最初のコードポイントで同じく見る。
    fn naive_target(s: &str, head: usize) -> Option<(ByteRange, char, char, bool)> {
        let gs = eve_text::grapheme_indices(s);
        let bracket_of = |g: &(usize, &str)| {
            let c = g.1.chars().next()?;
            let (o, cl) = PAIRS.iter().copied().find(|&(o, cl)| c == o || c == cl)?;
            Some((
                ByteRange::new(ByteOff(g.0), ByteOff(g.0 + c.len_utf8())),
                o,
                cl,
                c == o,
            ))
        };
        let after = gs.iter().find(|g| g.0 == head);
        let before = gs.iter().rev().find(|g| g.0 < head);
        after
            .and_then(bracket_of)
            .or_else(|| before.and_then(bracket_of))
    }

    /// 素朴な参照 (03 §4.7.1): コードポイントを 1 つずつ読み、同じ組の括弧をスタックで数える。探す範囲は対象の括弧から
    /// `limit` バイトの窓で、対応の括弧の全体が窓に入るものだけを数える (BRK-4)。
    fn naive(s: &str, head: usize, limit: usize) -> Lookup {
        let Some((target, o, c, open)) = naive_target(s, head) else {
            return Lookup::NoBracket;
        };
        let (lo, hi) = if open {
            (target.end.0, (target.end.0 + limit).min(s.len()))
        } else {
            (target.start.0.saturating_sub(limit), target.start.0)
        };
        let mut depth = 0i64;
        let hit = |(i, ch): (usize, char)| {
            let inside = lo <= i && i + ch.len_utf8() <= hi;
            if inside && (ch == o || ch == c) {
                depth += if (ch == o) == open { 1 } else { -1 };
            }
            (depth < 0).then(|| ByteRange::new(ByteOff(i), ByteOff(i + ch.len_utf8())))
        };
        let found = if open {
            s[target.end.0..]
                .char_indices()
                .map(|(i, ch)| (i + target.end.0, ch))
                .find_map(hit)
        } else {
            s[..target.start.0].char_indices().rev().find_map(hit)
        };
        match found {
            Some(p) => Lookup::Found { target, pair: p },
            None if (open && hi == s.len()) || (!open && lo == 0) => Lookup::NoMatch,
            None => Lookup::TooFar,
        }
    }

    /// 括弧を重み付けした文字列の片 (10 組・ASCII・日本語・結合文字・CRLF)。`long` なら、チャンクの境目にかかる長い片も
    /// 混ぜる。
    fn piece(long: bool) -> impl Strategy<Value = String> {
        let mut v: Vec<String> = PAIRS
            .iter()
            .flat_map(|&(o, c)| [o.to_string(), c.to_string()])
            .collect();
        v.extend(["a", "あ", "、", "。", "　", "\u{0301}", "\n", "\r\n"].map(str::to_owned));
        if long {
            v.extend(["x".repeat(300), "日本語".repeat(80)]);
        } else {
            v.extend(["xxxxxxx".to_owned(), "日本語".to_owned()]);
        }
        proptest::sample::select(v)
    }

    /// 文字列 `s` の中の書記素の境界のカーソル (`at` で文字列の全体に散らす)。
    fn cursor(t: &Text, s: &str, at: proptest::sample::Index) -> ByteOff {
        snap_to_grapheme(t, ByteOff(at.index(s.len() + 1)), Bias::Before)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        // 上限の内側では素朴な参照と一致し、対応のある括弧の直前から 2 回移ると元の位置に戻る
        #[test]
        fn 素朴な参照と一致し_直前からの往復で戻る(
            pieces in proptest::collection::vec(piece(true), 0..60),
            at in any::<proptest::sample::Index>(),
        ) {
            let s: String = pieces.concat();
            let t = Text::from(s.as_str());
            let head = cursor(&t, &s, at);
            let got = find_pair(&t, head, None).0;
            prop_assert_eq!(got, naive(&s, head.0, BRACKET_SCAN_MAX_BYTES), "{:?} @ {}", s, head.0);
            if let Lookup::Found { target, pair } = got
                && target.start == head
            {
                let back = find_pair(&t, cell_of(&t, pair), None).0;
                prop_assert!(
                    matches!(back, Lookup::Found { pair: p, .. } if cell_of(&t, p) == head),
                    "{:?} @ {} → {:?}", s, head.0, back
                );
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        // 小さな上限 (窓) でも、窓を切った素朴な参照と一致する (全角の括弧が窓の端で切れる場合・後ろ向きを含む)
        #[test]
        fn 小さな上限でも窓を切った参照と一致する(
            pieces in proptest::collection::vec(piece(false), 0..80),
            at in any::<proptest::sample::Index>(),
            limit in 1usize..64,
        ) {
            let s: String = pieces.concat();
            let t = Text::from(s.as_str());
            let head = cursor(&t, &s, at);
            let got = find_pair_within(&t, head, None, limit).0;
            prop_assert_eq!(got, naive(&s, head.0, limit), "{:?} @ {} limit {}", s, head.0, limit);
        }
    }

    #[test]
    fn 窓の端で切れた全角の括弧は数えない() {
        // `（` の終端 (3) から 5 バイトの窓 [3, 8) に、`）` (5..8) の全体は入る。4 バイトの窓 [3, 7) では切れる
        let s = "（ab）";
        let t = Text::from(s);
        assert!(matches!(
            find_pair_within(&t, ByteOff(0), None, 5).0,
            Lookup::Found { .. }
        ));
        assert_eq!(find_pair_within(&t, ByteOff(0), None, 4).0, Lookup::TooFar);
        // 後ろ向き: `）` の始端 (5) から 5 バイト前 (0) までなら `（` (0..3) は入る、4 バイト前 (1) からでは切れる
        assert!(matches!(
            find_pair_within(&t, ByteOff(8), None, 5).0,
            Lookup::Found { .. }
        ));
        assert_eq!(find_pair_within(&t, ByteOff(8), None, 4).0, Lookup::TooFar);
    }

    #[test]
    fn エスケープ文字は括弧ではない() {
        // 不正バイト 0x9A のエスケープ文字 (U+10FF9A、03 §8.3) と、括弧のバイト値に当たる私用文字 (U+10FF28 = `(` の値)
        let esc = "\u{10FF9A}(a)";
        assert_eq!(lookup(esc, 0), Lookup::NoBracket, "直後がエスケープ文字");
        assert!(
            matches!(lookup(esc, 4), Lookup::Found { .. }),
            "エスケープ文字の後ろの ("
        );
        assert_eq!(lookup("\u{10FF28}a\u{10FF29}", 0), Lookup::NoBracket);
    }

    #[test]
    fn フォーカスの無いペインの範囲は長い行でも可視範囲の程度() {
        // 1 行が 10,000 バイトの文書を 2 行 × 10 桁のペインに出すと、範囲は 2 × 10 × 4 = 80 バイトまで
        let s = "a".repeat(10_000);
        let t = Text::from(s.as_str());
        assert_eq!(visible_scope(&t, ByteOff(0), 2, 10), br(0, 80));
        // 書記素の境界へ前に寄せる (3 バイトの `あ` の途中で切らない)
        let s = "あ".repeat(100);
        let t = Text::from(s.as_str());
        assert_eq!(visible_scope(&t, ByteOff(0), 2, 10), br(0, 78));
        // 短い行は行の終端まで
        let t = Text::from("ab\ncd\nef");
        assert_eq!(visible_scope(&t, ByteOff(3), 2, 10), br(3, 8));
        // 長い行の先の対応は探さない
        let s = format!("({})", "a".repeat(200));
        let t = Text::from(s.as_str());
        let scope = visible_scope(&t, ByteOff(0), 2, 10);
        assert_eq!(find_pair(&t, ByteOff(0), Some(scope)).0, Lookup::TooFar);
    }

    #[test]
    fn 範囲を限ると外の括弧は対象にしない() {
        let t = Text::from("(ab)\n(cd)");
        assert_eq!(
            find_pair(&t, ByteOff(0), Some(br(5, 9))).0,
            Lookup::NoBracket
        );
        assert!(matches!(
            find_pair(&t, ByteOff(5), Some(br(5, 9))).0,
            Lookup::Found { .. }
        ));
        // 範囲の端で止まれば、範囲を超えた
        assert_eq!(find_pair(&t, ByteOff(0), Some(br(0, 3))).0, Lookup::TooFar);
    }

    // ---------- 範囲を囲む括弧の組 (03 §5.7 の SEL-2 の (2) / (3)) ----------

    /// 素朴な参照: `r.start` にある開き括弧を先に見て、`r.start` の前の `limit` バイトの窓のコードポイントを後ろから 1 つずつ読み、組ごとの深さで閉じていない
    /// 開き括弧を近い順に取り、その対応を開き括弧から素朴に探して (BRK-3 / BRK-4。窓は `limit`)、閉じ括弧の終わりが `r.end`
    /// 以降にある最初の組。組ごとに対応を探し直す (`enclosing_pair_within` の続きから探す近道を使わない)。
    fn naive_enclosing(s: &str, r: ByteRange, limit: usize) -> Option<(ByteRange, ByteRange)> {
        // Range の始まりにある開き括弧は、閉じ括弧の終わりが Range の終わりより後ろなら囲む
        if let Some(ch) = s[r.start.0..].chars().next()
            && classify(ch).is_some_and(|(_, open)| open)
            && let Lookup::Found { target, pair: c } = naive(s, r.start.0, limit)
            && c.end > r.end
        {
            return Some((target, c));
        }
        let lo = r.start.0.saturating_sub(limit);
        let mut depth = [0usize; PAIRS.len()];
        for (i, ch) in s[..r.start.0].char_indices().rev() {
            if i < lo {
                break;
            }
            let Some((pair, open)) = classify(ch) else {
                continue;
            };
            if !open {
                depth[pair] += 1;
                continue;
            }
            if depth[pair] > 0 {
                depth[pair] -= 1;
                continue;
            }
            let o = ByteRange::new(ByteOff(i), ByteOff(i + ch.len_utf8()));
            if let Lookup::Found { pair: c, .. } = naive(s, i, limit)
                && c.end >= r.end
            {
                return Some((o, c));
            }
        }
        None
    }

    fn enclosing(s: &str, a: usize, b: usize) -> Option<(usize, usize)> {
        enclosing_pair(&Text::from(s), br(a, b), usize::MAX)
            .0
            .map(|(o, c)| (o.start.0, c.start.0))
    }

    // spec: 03 §5.7
    #[test]
    fn 範囲を囲む最も近い組を内側から探す() {
        // `f(a, (b))` の各位置
        let s = "f(a, (b))";
        assert_eq!(enclosing(s, 7, 7), Some((5, 7)), "(b) の中");
        assert_eq!(enclosing(s, 6, 7), Some((5, 7)), "b");
        assert_eq!(enclosing(s, 5, 8), Some((1, 8)), "(b) を含む範囲は外側の組");
        assert_eq!(enclosing(s, 2, 3), Some((1, 8)), "a");
        assert_eq!(enclosing(s, 1, 9), None, "外側の組を含む");
        assert_eq!(enclosing(s, 0, 0), None, "f の前");
        // 範囲が閉じ括弧をまたいで外へ出る組は候補にせず、外側を探す
        assert_eq!(enclosing("[(a)b]", 2, 5), Some((0, 5)));
        // 閉じ括弧で終わる範囲 (閉じ括弧を含む) は、その組に囲まれる
        assert_eq!(enclosing("f(a)", 3, 4), Some((1, 3)), "`)` だけ");
        assert_eq!(enclosing("f(a)", 2, 4), Some((1, 3)), "`a)`");
        assert_eq!(enclosing("（a）", 3, 7), Some((0, 4)), "全角の閉じ括弧");
        // 開き括弧で始まる範囲 (開き括弧を含む・開き括弧の直前のカーソル) も、その組に囲まれる
        assert_eq!(enclosing("foo(bar)", 3, 4), Some((3, 7)), "`(` だけ");
        assert_eq!(enclosing("foo(bar)", 3, 7), Some((3, 7)), "`(bar`");
        assert_eq!(enclosing("foo(bar)", 3, 3), Some((3, 7)), "`|(` のカーソル");
        assert_eq!(enclosing("「a」", 0, 3), Some((0, 4)), "全角の開き括弧");
        // 組の全体を選んでいれば、その組は囲まない (外側を探す)
        assert_eq!(enclosing("[foo(bar)]", 4, 9), Some((0, 9)));
        assert_eq!(enclosing("foo(bar)", 3, 8), None);
        // 入れ子: 外側の `(` なら外側の組、内側の `(` なら内側の組
        assert_eq!(enclosing("((a))", 0, 1), Some((0, 4)));
        assert_eq!(enclosing("((a))", 1, 2), Some((1, 3)));
        assert_eq!(
            enclosing("((a))", 1, 4),
            Some((0, 4)),
            "内側の組の全体は外側の組へ"
        );
        // 始まりの開き括弧の対応が範囲の中なら、同じ組の外側は続きから探す
        assert_eq!(enclosing("((a) b)", 1, 6), Some((0, 6)));
        // 対応しない開き括弧は飛ばす
        assert_eq!(enclosing("[a ( b ]", 5, 6), Some((0, 7)));
        // ほかの組は数えない
        assert_eq!(enclosing("「(」)", 4, 4), Some((3, 7)));
        // 同じ組の内側の組が範囲の中で閉じても、外側の組の対応は正しい
        assert_eq!(enclosing("((a) b)", 1, 5), Some((0, 6)));
        assert_eq!(enclosing("(()(x) y)", 4, 8), Some((0, 8)));
    }

    // spec: 03 §5.7
    #[test]
    fn 前へ探すのは_1_mib_までで_予算を使い切ると探さない() {
        // 前へ: 範囲の始まりから 1 MiB の窓に開き括弧が入るか
        let s = format!("({}x)", "a".repeat(BRACKET_SCAN_MAX_BYTES));
        let x = 1 + BRACKET_SCAN_MAX_BYTES;
        assert_eq!(enclosing(&s, x, x + 1), None, "開き括弧が 1 MiB の外");
        // 対応: 開き括弧の終端から 1 MiB の窓に閉じ括弧の全体が入るか
        let s = format!("({}x)", "a".repeat(BRACKET_SCAN_MAX_BYTES - 2));
        let x = BRACKET_SCAN_MAX_BYTES - 1;
        assert_eq!(enclosing(&s, x, x + 1), Some((0, x + 1)), "ちょうど 1 MiB");
        let s = format!("({}x)", "a".repeat(BRACKET_SCAN_MAX_BYTES - 1));
        let x = BRACKET_SCAN_MAX_BYTES;
        assert_eq!(enclosing(&s, x, x + 1), None, "対応が 1 MiB の外");
        let (found, read) = enclosing_pair(&Text::from("(ab)"), br(2, 2), 0);
        assert_eq!(found, None, "予算が 0 なら対応を探さない");
        assert!(read <= 2, "前へ読んだ量だけ: {read}");
    }

    #[test]
    fn 全角の組の括弧はどれも先頭_2_バイトで絞る表に入る() {
        for &(o, c) in &PAIRS {
            for ch in [o, c] {
                let mut b = [0u8; 4];
                let s = ch.encode_utf8(&mut b).as_bytes();
                assert!(
                    s.len() == 1 || WIDE_PREFIXES.contains(&[s[0], s[1]]),
                    "{ch} {s:?}"
                );
                if s.len() == 3 {
                    assert_eq!(bracket_at(s, 0), classify(ch).map(|(p, op)| (p, op, 3)));
                }
            }
        }
        // かな・漢字・全角の句読点は括弧ではない
        for ch in ['あ', 'ア', '漢', '、', '。', '　', '！'] {
            let mut b = [0u8; 4];
            assert_eq!(
                bracket_at(ch.encode_utf8(&mut b).as_bytes(), 0),
                None,
                "{ch}"
            );
        }
    }

    // spec: 03 §5.7
    #[test]
    fn 対応の無い開き括弧が並んでも読む量は窓の程度() {
        // 1 万個の対応しない `(` の後ろの範囲: 組ごとに対応を探し直さない (内側に対応が無ければ外側にも無い)
        let s = format!("{}abc", "(".repeat(10_000));
        let (found, read) = enclosing_pair(&Text::from(s.as_str()), br(10_001, 10_002), usize::MAX);
        assert_eq!(found, None);
        assert!(read < 3 * s.len(), "読んだ量 {read}");
        // 対応のある内側の組が多数並んでも、外側の対応は続きから探す
        let s = format!("[{}x]", "()".repeat(10_000));
        let x = 1 + 20_000;
        let (found, read) = enclosing_pair(&Text::from(s.as_str()), br(x, x + 1), usize::MAX);
        assert_eq!(found.map(|(o, c)| (o.start.0, c.start.0)), Some((0, x + 1)));
        assert!(read < 3 * s.len(), "読んだ量 {read}");
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        // 範囲を囲む組が、組ごとに探し直す素朴な参照と一致する (小さな窓を含む)
        #[test]
        fn 範囲を囲む組が素朴な参照と一致する(
            pieces in proptest::collection::vec(piece(false), 0..60),
            a in any::<proptest::sample::Index>(),
            b in any::<proptest::sample::Index>(),
            limit in prop_oneof![1usize..64, Just(BRACKET_SCAN_MAX_BYTES)],
        ) {
            let s: String = pieces.concat();
            let t = Text::from(s.as_str());
            let (x, y) = (cursor(&t, &s, a), cursor(&t, &s, b));
            let r = ByteRange::new(x.min(y), x.max(y));
            let got = enclosing_pair_within(&t, r, limit, usize::MAX).0;
            prop_assert_eq!(got, naive_enclosing(&s, r, limit), "{:?} {:?} limit {}", s, r, limit);
        }
    }
}
