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
    /// 共通の先頭 (ASCII は空、全角は 2 バイト)。
    prefix: Vec<u8>,
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
        debug_assert!(
            o.len() == c.len() && o[..o.len() - 1] == c[..c.len() - 1],
            "組の括弧は最後のバイトだけが違う: {o:?} {c:?}"
        );
        Pattern {
            prefix: o[..o.len() - 1].to_vec(),
            open: o[o.len() - 1],
            close: c[c.len() - 1],
        }
    }

    fn len(&self) -> usize {
        self.prefix.len() + 1
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
    if pat.prefix.is_empty() {
        let mut hit = |i: usize| f(hay[i] == pat.open).then_some(i);
        return if forward {
            memchr::memchr2_iter(pat.open, pat.close, hay).find_map(&mut hit)
        } else {
            memchr::memchr2_iter(pat.open, pat.close, hay)
                .rev()
                .find_map(&mut hit)
        };
    }
    let last = pat.prefix.len();
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

/// `target` の括弧の対応を `window` の中だけで探す (03 §5.7 の BRK-3)。前向き (開き括弧) は `window.end` まで、後ろ向き
/// (閉じ括弧) は `window.start` まで。(対応の範囲, 読んだバイト数)。
fn scan(
    text: &dyn TextRead,
    target: ByteRange,
    pair: usize,
    open: bool,
    window: ByteRange,
) -> (Option<ByteRange>, usize) {
    let pat = Pattern::new(pair);
    let finders = (
        memmem::Finder::new(&pat.prefix),
        memmem::FinderRev::new(&pat.prefix),
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
        let (mut pos, hi) = (target.end.0, window.end.0);
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
        let (mut pos, lo) = (target.start.0, window.start.0);
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
    let Some((target, pair, open)) = target_at(text, head) else {
        return (Lookup::NoBracket, 0);
    };
    let len = text.len_bytes();
    let mut window = if open {
        ByteRange::new(
            target.end,
            ByteOff(target.end.0.saturating_add(BRACKET_SCAN_MAX_BYTES).min(len)),
        )
    } else {
        ByteRange::new(
            ByteOff(target.start.0.saturating_sub(BRACKET_SCAN_MAX_BYTES)),
            target.start,
        )
    };
    if let Some(s) = scope {
        if target.start < s.start || target.end > s.end {
            return (Lookup::NoBracket, 0);
        }
        window = ByteRange::new(window.start.max(s.start), window.end.min(s.end));
    }
    let (found, read) = scan(text, target, pair, open, window);
    let lookup = match found {
        Some(p) => Lookup::Found { target, pair: p },
        // 文書の端まで探したなら対応なし、上限で止めたなら範囲を超えた
        None if (open && window.end.0 == len) || (!open && window.start.0 == 0) => Lookup::NoMatch,
        None => Lookup::TooFar,
    };
    (lookup, read)
}

/// 括弧の位置のセル (書記素の始まり)。括弧のコードポイントは、ふつうは書記素の始まりにある。前の文字に結合する
/// (Prepend の後ろなど) ときは、それを含む書記素の始まりへ寄せる。
fn cell_of(text: &dyn TextRead, r: ByteRange) -> ByteOff {
    snap_to_grapheme(text, r.start, Bias::Before)
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
        let panes: Vec<(ViewId, Option<u16>)> = w
            .layout(CellSize { cols, rows })
            .leaves
            .iter()
            .filter_map(|l| {
                let p = w.panes.get(&l.pane)?;
                Some((p.view, (l.pane != w.focus).then_some(l.body.rows)))
            })
            .collect();
        for (view, unfocused_rows) in panes {
            self.sync_bracket_mark(view, unfocused_rows);
        }
    }

    fn sync_bracket_mark(&mut self, view: ViewId, unfocused_rows: Option<u16>) {
        let Some(v) = self.docs.views.get(&view) else {
            return;
        };
        let Some(d) = self.docs.documents.get(&v.doc) else {
            return;
        };
        let t = d.text();
        let head = v.selection.primary().head;
        // フォーカスの無いペインの可視範囲 (先頭の行の始端から、行数ぶん下の行の終端まで)
        let scope = unfocused_rows.map(|rows| {
            let top = t.byte_to_line(v.scroll.top);
            let last = eve_text::LineIdx(
                (top.0 + usize::from(rows).saturating_sub(1)).min(t.len_lines().saturating_sub(1)),
            );
            ByteRange::new(t.line_to_byte(top), t.line_end_byte(last))
        });
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

    /// View `view` の対応括弧の強調のセル (文書のバイト位置)。覚えた鍵が今の文書と主のカーソルに合わなければ空。
    pub(crate) fn bracket_cells(&self, view: ViewId) -> Vec<usize> {
        let Some(v) = self.docs.views.get(&view) else {
            return Vec::new();
        };
        let Some(d) = self.docs.documents.get(&v.doc) else {
            return Vec::new();
        };
        v.bracket
            .as_ref()
            .filter(|m| {
                m.revision == d.revision()
                    && m.len == d.text().len_bytes()
                    && m.head == v.selection.primary().head
            })
            .and_then(|m| m.cells)
            .map(|c| c.to_vec())
            .unwrap_or_default()
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
            assert!(p.prefix.is_empty() || p.prefix.len() == 2, "{pair:?}");
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

    /// 素朴な参照 (03 §4.7.1): コードポイントを 1 つずつ読み、同じ組の括弧をスタックで数える (範囲の上限は小さな文書では
    /// 効かない)。対象の括弧の決め方 (BRK-2) は実装と同じ。
    fn naive(s: &str, head: usize) -> Lookup {
        let t = Text::from(s);
        let Some((target, pair, open)) = target_at(&t, ByteOff(head)) else {
            return Lookup::NoBracket;
        };
        let (o, c) = PAIRS[pair];
        let mut depth = 0i64;
        let hit = |(i, ch): (usize, char)| {
            if ch == o || ch == c {
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
            None => Lookup::NoMatch,
        }
    }

    /// 括弧を重み付けした文字列の片 (10 組・ASCII・日本語・結合文字・チャンクの境目にかかる長い片)。
    fn piece() -> impl Strategy<Value = String> {
        let mut v: Vec<String> = PAIRS
            .iter()
            .flat_map(|&(o, c)| [o.to_string(), c.to_string()])
            .collect();
        v.extend(
            [
                "a",
                "あ",
                "\u{0301}",
                "\n",
                "\r\n",
                "x".repeat(300).as_str(),
                "日本語".repeat(80).as_str(),
            ]
            .map(str::to_owned),
        );
        proptest::sample::select(v)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        // 上限の内側では素朴な参照と一致し、対応のある括弧の直前から 2 回移ると元の位置に戻る
        #[test]
        fn 素朴な参照と一致し_直前からの往復で戻る(
            pieces in proptest::collection::vec(piece(), 0..60),
            at in 0usize..10_000,
        ) {
            let s: String = pieces.concat();
            let t = Text::from(s.as_str());
            // 書記素の境界のカーソル
            let head = snap_to_grapheme(&t, ByteOff(at.min(s.len())), Bias::Before);
            let got = find_pair(&t, head, None).0;
            prop_assert_eq!(got, naive(&s, head.0), "{:?} @ {}", s, head.0);
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
}
