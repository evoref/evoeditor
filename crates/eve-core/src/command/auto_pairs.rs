//! 括弧・引用符の自動閉じ (docs/03 §15.7 の APR-1〜APR-8、01 FR-210)。
//!
//! - 打鍵の経路の文字の入力 (`edit.insert_text` を `InvocationSource::Typing` で実行したとき) を、書記素ごとに
//!   順に当てる (APR-2 (a))。組の文字を含まない連続した書記素は 1 回にまとめて入れる (1 つずつ入れても同じ結果)。
//!   書記素ごとの入力は、それぞれ 1 つの Transaction (`Origin::User`) で、連続入力の結合 (03 §7.3) で 1 つずつ打った
//!   ときと同じに数える (APR-7。`Transaction::coalesce_as`)。
//! - 自動で足した閉じ字は View ごとに覚え ([`AutoClosed`])、編集の写像で追う。カーソルが対の間を離れたら忘れる
//!   ([`prune_auto_closed`]。入力のたびに確かめる)。Undo / Redo・選択を置き直す操作 (`set_selection`)・本文の差し替えでも
//!   忘れる。写像は覚えの位置を並べて 1 回の走査で行う (`ChangeSet::map_sorted`。費用は覚えの数 + 演算の数)。
//! - `edit.delete_backward` の対の間の削除 (APR-4) は [`delete_backward_pairs`]。
//!
//! spec: 03 §15.7, 03 §7.3, 03 §4.5

use std::sync::{Arc, LazyLock};

use eve_text::{
    Bias, ByteOff, ByteRange, ChangeSet, Edit, Range, ScriptClass, Selection, TextRead,
    classify_grapheme, grapheme_indices, next_grapheme_boundary, prev_grapheme_boundary,
};
use eve_view::{DocumentId, ViewId};

use crate::command::{CommandCtx, CommandResult, RejectReason};
use crate::document::{Document, Origin, Transaction};
use crate::editor::state::EditorState;

/// 既定の組 (APR-1。開き字, 閉じ字)。`'` と全角の括弧は含めない (言語定義の `auto_pairs` で足す)。
pub(crate) const DEFAULT_PAIRS: [(char, char); 5] =
    [('(', ')'), ('[', ']'), ('{', '}'), ('"', '"'), ('`', '`')];

/// 組の並び (開き字, 閉じ字)。言語定義の組と既定の組を、複製せずに共有する。
pub(crate) type Pairs = eve_config::AutoPairs;

/// 既定の組の共有の写し。
static DEFAULT_SHARED: LazyLock<Pairs> = LazyLock::new(|| Arc::from(DEFAULT_PAIRS.as_slice()));

/// 直後にあると対にする全角の閉じ括弧 (03 §5.7 の BRK-1 の閉じ括弧のうち全角のもの。APR-2 (c))。
const FULLWIDTH_CLOSERS: [char; 7] = ['）', '］', '｝', '」', '』', '】', '〕'];

/// 直後にあると対にする句読点 (APR-2 (c))。
const STOP_PUNCT: [char; 6] = [',', ';', ':', '.', '、', '。'];

/// 自動で足した閉じ字 1 つの覚え (APR-3。開き字と閉じ字の位置と文字)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AutoClosed {
    open: ByteOff,
    open_char: char,
    close: ByteOff,
    close_char: char,
}

impl AutoClosed {
    /// 開き字の直後 (対の中身の始まり)。
    fn open_end(&self) -> ByteOff {
        ByteOff(self.open.0 + self.open_char.len_utf8())
    }

    /// 閉じ字の直後。
    fn close_end(&self) -> ByteOff {
        ByteOff(self.close.0 + self.close_char.len_utf8())
    }
}

/// 覚えを編集 `cs` で写像する。開き字か閉じ字が消えた (置き換えられた) ものは忘れる。各文字の始まりを `After`、終わりを
/// `Before` で写し、写した長さが元の長さと同じものだけを残す。位置 (覚えごとに 4 つ) を並べ替えて (入れ子では開き字の順が
/// 閉じ字の順と逆になる) 1 回の走査で写す (費用は覚えの数 × log + 演算の数。覚えごとに `map_pos` を呼ぶと、覚えの数 × 演算の
/// 数になり、1 万カーソルで 2 乗になる)。
pub(crate) fn map_auto_closed(list: &mut Vec<AutoClosed>, cs: &ChangeSet) {
    if list.is_empty() {
        return;
    }
    // (位置, 向き, 覚えの番号 × 4 + 種類 (0 開き字の始まり・1 終わり・2 閉じ字の始まり・3 終わり))
    let mut ps: Vec<(ByteOff, Bias, usize)> = Vec::with_capacity(list.len() * 4);
    for (i, a) in list.iter().enumerate() {
        ps.push((a.open, Bias::After, i * 4));
        ps.push((a.open_end(), Bias::Before, i * 4 + 1));
        ps.push((a.close, Bias::After, i * 4 + 2));
        ps.push((a.close_end(), Bias::Before, i * 4 + 3));
    }
    ps.sort_unstable_by_key(|(p, _, _)| *p);
    let sorted: Vec<(ByteOff, Bias)> = ps.iter().map(|(p, b, _)| (*p, *b)).collect();
    let mapped = cs.map_sorted(&sorted);
    let mut at = vec![ByteOff(0); list.len() * 4];
    for ((_, _, k), m) in ps.iter().zip(mapped) {
        at[*k] = m;
    }
    let mut i = 0;
    list.retain_mut(|a| {
        let q = &at[i * 4..i * 4 + 4];
        i += 1;
        let kept = |s: ByteOff, e: ByteOff, len: usize| e.0.checked_sub(s.0) == Some(len);
        if !kept(q[0], q[1], a.open_char.len_utf8()) || !kept(q[2], q[3], a.close_char.len_utf8()) {
            return false;
        }
        a.open = q[0];
        a.close = q[2];
        true
    });
    normalize(list);
}

/// 本文の `at` に文字 `ch` があるか (覚えの守り: 本文を差し替える経路が覚えを消し忘れても、違う文字を飛び越え・消さない)。
fn char_at(text: &dyn TextRead, at: ByteOff, ch: char) -> bool {
    let end = at.0 + ch.len_utf8();
    end <= text.len_bytes()
        && text.is_char_boundary(at)
        && text.is_char_boundary(ByteOff(end))
        && text
            .slice_to_string(ByteRange::new(at, ByteOff(end)))
            .as_str()
            == &*ch.encode_utf8(&mut [0; 4])
}

/// 閉じ字の位置の順に並べ、同じ位置の覚えを 1 つにする。
fn normalize(list: &mut Vec<AutoClosed>) {
    list.sort_by_key(|a| a.close);
    list.dedup_by_key(|a| a.close);
}

/// カーソル (各 Range の head) が対の間 (開き字の直後から閉じ字の直前まで、同じ行) に無い覚えを忘れる (APR-3)。
/// 費用は覚えの数 × log (Range の数)。
pub(crate) fn prune_auto_closed(text: &dyn TextRead, sel: &Selection, list: &mut Vec<AutoClosed>) {
    if list.is_empty() {
        return;
    }
    let mut heads: Vec<ByteOff> = sel.iter().map(|r| r.head).collect();
    heads.sort_unstable();
    let len = text.len_bytes();
    list.retain(|a| {
        if a.close.0 >= len || a.open_end() > a.close {
            return false;
        }
        let i = heads.partition_point(|h| *h < a.open_end());
        let inside = heads.get(i).is_some_and(|h| *h <= a.close);
        inside && text.byte_to_line(a.open) == text.byte_to_line(a.close)
    });
}

impl EditorState {
    /// すべての View の、自動で足した閉じ字の覚えのうち、カーソルが対の間を離れたものを忘れる (APR-3。入力とコマンドの
    /// 実行の後に呼ぶ)。
    pub(crate) fn prune_auto_pairs(&mut self) {
        let docs = &self.docs.documents;
        for v in self.docs.views.values_mut() {
            if v.auto_closed.is_empty() {
                continue;
            }
            match docs.get(&v.doc) {
                Some(d) => prune_auto_closed(d.text(), &v.selection, &mut v.auto_closed),
                None => v.auto_closed.clear(),
            }
        }
    }
}

/// 文書 `doc` で使う組 (APR-1)。自動閉じを切っている (`editor.auto_pairs = false`) なら `None`。言語定義の `auto_pairs` が
/// あればその組 (`{}` なら空)、無ければ既定の組。
pub(crate) fn pairs_for(state: &EditorState, doc: &Document) -> Option<Pairs> {
    if !state.config().auto_pairs {
        return None;
    }
    Some(
        match state.language_of(doc).and_then(|l| l.auto_pairs.as_ref()) {
            Some((p, _)) => Arc::clone(p),
            None => Arc::clone(&DEFAULT_SHARED),
        },
    )
}

/// 開き字 `o` の閉じ字。
fn closer_of(pairs: &[(char, char)], o: char) -> Option<char> {
    pairs.iter().find(|(a, _)| *a == o).map(|(_, c)| *c)
}

/// 組の文字 (開き字か閉じ字) か。
fn is_pair_char(pairs: &[(char, char)], ch: char) -> bool {
    pairs.iter().any(|(o, c)| *o == ch || *c == ch)
}

/// 1 つのコードポイントだけの書記素なら、その文字。
fn single_char(g: &str) -> Option<char> {
    let mut it = g.chars();
    match (it.next(), it.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    }
}

/// APR-2 (c): カーソル `p` の直後が、対にしてよい文字か (行末・空白・閉じ字・全角の閉じ括弧・句読点)。
fn next_allows(text: &dyn TextRead, p: ByteOff, pairs: &[(char, char)]) -> bool {
    if p.0 >= text.len_bytes() {
        return true;
    }
    let end = next_grapheme_boundary(text, p);
    let Some(ch) = text.slice_to_string(ByteRange::new(p, end)).chars().next() else {
        return true;
    };
    matches!(ch, '\n' | '\r' | ' ' | '\t' | '\u{3000}')
        || pairs.iter().any(|(o, c)| *c == ch && o != c)
        || FULLWIDTH_CLOSERS.contains(&ch)
        || STOP_PUNCT.contains(&ch)
}

/// APR-2 (d): 引用符 `q` を対にしてよいか (直前の書記素が英数字でも同じ引用符でもない)。
fn prev_allows_quote(text: &dyn TextRead, p: ByteOff, q: char) -> bool {
    if p.0 == 0 {
        return true;
    }
    let start = prev_grapheme_boundary(text, p);
    let g = text.slice_to_string(ByteRange::new(start, p));
    if single_char(&g) == Some(q) {
        return false;
    }
    !matches!(
        classify_grapheme(&g, None),
        ScriptClass::Alnum | ScriptClass::FullwidthAlnum
    )
}

/// 1 つの Range への操作。
enum Act {
    /// 範囲を `ins` で置き換える (空の範囲は挿入)。カーソルは入れた文字列の後ろ。
    Put { del: ByteRange, ins: Arc<str> },
    /// 開き字と閉じ字を入れ、カーソルを間に置く (APR-2)。
    Pair {
        at: ByteOff,
        open: char,
        close: char,
    },
    /// 自動で足した閉じ字を飛び越える (APR-3)。
    Skip { at: ByteOff, close: char },
    /// 選択を囲む (APR-5。向きを保つ)。
    Wrap {
        range: Range,
        open: char,
        close: char,
    },
    /// 対の間の Backspace で、開き字と閉じ字を消す (APR-4)。
    PairDelete {
        open_at: ByteOff,
        open: char,
        close: char,
    },
    /// 範囲を消す (今の `edit.delete_backward`。空なら何もしない)。
    Delete { del: ByteRange },
}

/// 1 回の入力 (書記素 1 つか、組の文字を含まない書記素の並び) の計画。
struct Step {
    edits: Vec<Edit>,
    /// 各 Range の適用後の Range (`None` は写像で追う)。
    ranges: Vec<Option<Range>>,
    /// 連続入力の結合で見る変更のまとまり (APR-7)。
    coalesce: Vec<(ByteRange, ByteRange)>,
    /// 新しく覚える、自動で足した閉じ字 (適用後の位置)。
    closed: Vec<AutoClosed>,
    /// 飛び越えた Range がある。
    skipped: bool,
}

/// 各 Range の操作から編集の列と適用後の Range を作る。前の Range の編集と重なる操作は行わない (03 §4.5 の 2)。
fn build(acts: Vec<Act>) -> Step {
    let mut step = Step {
        edits: Vec::new(),
        ranges: Vec::with_capacity(acts.len()),
        coalesce: Vec::new(),
        closed: Vec::new(),
        skipped: false,
    };
    // 前の編集で足した量と消した量 (編集は昇順で重ならないので、開始 + 足した量 >= 消した量)
    let (mut added, mut removed) = (0usize, 0usize);
    // それまでの編集の末尾 (適用前の位置)
    let mut covered = 0usize;
    let after = |p: usize, added: usize, removed: usize| (p + added) - removed;
    for act in acts {
        let first = match &act {
            Act::Put { del, .. } | Act::Delete { del } => del.start.0,
            Act::Pair { at, .. } | Act::Skip { at, .. } => at.0,
            Act::Wrap { range, .. } => range.start().0,
            Act::PairDelete { open_at, .. } => open_at.0,
        };
        if first < covered {
            step.ranges.push(None);
            continue;
        }
        match act {
            Act::Put { del, ins } => {
                let s = after(del.start.0, added, removed);
                step.coalesce
                    .push((del, ByteRange::new(ByteOff(s), ByteOff(s + ins.len()))));
                step.ranges.push(Some(Range::point(ByteOff(s + ins.len()))));
                added += ins.len();
                removed += del.len();
                covered = del.end.0;
                step.edits.push(Edit {
                    range: del,
                    insert: ins,
                });
            }
            Act::Pair { at, open, close } => {
                let s = after(at.0, added, removed);
                let (lo, lc) = (open.len_utf8(), close.len_utf8());
                step.coalesce.push((
                    ByteRange::new(at, at),
                    ByteRange::new(ByteOff(s), ByteOff(s + lo)),
                ));
                step.ranges.push(Some(Range::point(ByteOff(s + lo))));
                step.closed.push(AutoClosed {
                    open: ByteOff(s),
                    open_char: open,
                    close: ByteOff(s + lo),
                    close_char: close,
                });
                added += lo + lc;
                covered = at.0;
                step.edits.push(Edit {
                    range: ByteRange::new(at, at),
                    insert: format!("{open}{close}").into(),
                });
            }
            Act::Skip { at, close } => {
                let s = after(at.0, added, removed);
                let lc = close.len_utf8();
                step.coalesce.push((
                    ByteRange::new(at, at),
                    ByteRange::new(ByteOff(s), ByteOff(s + lc)),
                ));
                step.ranges.push(Some(Range::point(ByteOff(s + lc))));
                step.skipped = true;
                covered = at.0;
            }
            Act::Wrap { range, open, close } => {
                let (start, end) = (range.start(), range.end());
                let (lo, lc) = (open.len_utf8(), close.len_utf8());
                let s = after(start.0, added, removed);
                added += lo;
                let e = after(end.0, added, removed);
                added += lc;
                step.coalesce.push((
                    ByteRange::new(start, start),
                    ByteRange::new(ByteOff(s), ByteOff(s + lo)),
                ));
                step.coalesce.push((
                    ByteRange::new(end, end),
                    ByteRange::new(ByteOff(e), ByteOff(e + lc)),
                ));
                // 囲んだ閉じ字も自動で足した閉じ字として覚える (同じ行の囲みだけが残る。複数行は prune で落ちる。APR-5)
                step.closed.push(AutoClosed {
                    open: ByteOff(s),
                    open_char: open,
                    close: ByteOff(e),
                    close_char: close,
                });
                let (inner_start, inner_end) = (ByteOff(s + lo), ByteOff(e));
                step.ranges.push(Some(if range.is_forward() {
                    Range::new(inner_start, inner_end)
                } else {
                    Range::new(inner_end, inner_start)
                }));
                covered = end.0;
                step.edits.push(Edit {
                    range: ByteRange::new(start, start),
                    insert: open.to_string().into(),
                });
                step.edits.push(Edit {
                    range: ByteRange::new(end, end),
                    insert: close.to_string().into(),
                });
            }
            Act::PairDelete {
                open_at,
                open,
                close,
            } => {
                let s = after(open_at.0, added, removed);
                let (lo, lc) = (open.len_utf8(), close.len_utf8());
                let del = ByteRange::new(open_at, ByteOff(open_at.0 + lo + lc));
                // 自動閉じを切っていたときの、開き字だけの削除とみなす (APR-7)
                step.coalesce.push((
                    ByteRange::new(open_at, ByteOff(open_at.0 + lo)),
                    ByteRange::new(ByteOff(s), ByteOff(s)),
                ));
                step.ranges.push(Some(Range::point(ByteOff(s))));
                removed += del.len();
                covered = del.end.0;
                step.edits.push(Edit {
                    range: del,
                    insert: "".into(),
                });
            }
            Act::Delete { del } => {
                if del.is_empty() {
                    step.ranges.push(None);
                    continue;
                }
                let s = after(del.start.0, added, removed);
                step.coalesce
                    .push((del, ByteRange::new(ByteOff(s), ByteOff(s))));
                step.ranges.push(Some(Range::point(ByteOff(s))));
                removed += del.len();
                covered = del.end.0;
                step.edits.push(Edit {
                    range: del,
                    insert: "".into(),
                });
            }
        }
    }
    step
}

/// 計画 `step` を View `view_id` の文書 `doc_id` に当てる。文書を変える Range があれば 1 つの Transaction にし、飛び越えだけ
/// なら Transaction を作らずに選択を動かして結合の判定に数える (APR-6 / APR-7)。
fn apply_step(
    state: &mut EditorState,
    view_id: ViewId,
    doc_id: DocumentId,
    before: Selection,
    step: Step,
) -> CommandResult {
    let Some(doc) = state.document(doc_id) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let cs = match ChangeSet::from_sorted_edits(&doc.snapshot(), step.edits) {
        Ok(cs) => cs,
        Err(e) => {
            debug_assert!(false, "自動閉じの編集が重なる・境界が違う: {e:?}");
            return CommandResult::Rejected(RejectReason::InvalidArgs);
        }
    };
    let ranges: Vec<Range> = before
        .iter()
        .zip(step.ranges)
        .map(|(r, n)| n.unwrap_or_else(|| r.map(&cs, Bias::Before)))
        .collect();
    let Some(after) =
        Selection::new(ranges, before.primary_index()).map(|s| s.merged(cs.len_after()))
    else {
        return CommandResult::Rejected(RejectReason::InvalidArgs);
    };
    let tx = Transaction::new(cs, Origin::User, state.now())
        .with_view(view_id)
        .with_selections(Some(before), Some(after.clone()))
        .with_coalesce_as(step.coalesce);
    if tx.changes.is_identity() {
        if !step.skipped {
            return CommandResult::Done;
        }
        // 飛び越えだけ: 文書を変えず、閉じ字の入力として結合の判定に数える (APR-7)。履歴の会計は記録と同じく、文書の予算と
        // プロセス全体の予算で確かめる (03 §7.5)
        let pruned = state
            .docs
            .documents
            .get_mut(&doc_id)
            .is_some_and(|d| d.absorb_identity_input(&tx));
        state.note_history_recorded(doc_id, pruned);
        state.put_selection(view_id, after, crate::editor::state::PairMemory::Keep);
        state.ensure_cursor_visible(view_id);
    } else if state.apply_transaction(doc_id, tx).is_err() {
        return CommandResult::Rejected(RejectReason::ReadOnly);
    }
    let Some(doc) = state.docs.documents.get(&doc_id) else {
        return CommandResult::Done;
    };
    if let Some(v) = state.docs.views.get_mut(&view_id) {
        v.auto_closed.extend(step.closed);
        normalize(&mut v.auto_closed);
        prune_auto_closed(doc.text(), &v.selection, &mut v.auto_closed);
    }
    CommandResult::Done
}

/// 入れる文字列を、1 回ずつ当てる単位に分ける: 組の文字 (1 つのコードポイントの書記素) は 1 つずつ、ほかの連続した書記素は
/// まとめる。
fn chunks<'a>(text: &'a str, pairs: &[(char, char)]) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut plain_from: Option<usize> = None;
    for (i, g) in grapheme_indices(text) {
        if single_char(g).is_some_and(|c| is_pair_char(pairs, c)) {
            if let Some(s) = plain_from.take() {
                out.push(&text[s..i]);
            }
            out.push(g);
        } else if plain_from.is_none() {
            plain_from = Some(i);
        }
    }
    if let Some(s) = plain_from {
        out.push(&text[s..]);
    }
    out
}

/// 打鍵の経路の文字の入力 `text` を、組 `pairs` で自動閉じしながら入れる (APR-2〜APR-6)。`text` は外からの文字列の規則
/// (03 §8.3.1) を当てた後のもの。
pub(crate) fn insert_typed(
    ctx: &mut CommandCtx<'_>,
    view_id: ViewId,
    doc_id: DocumentId,
    text: &str,
    pairs: &[(char, char)],
) -> CommandResult {
    for chunk in chunks(text, pairs) {
        let r = insert_chunk(ctx.state, view_id, doc_id, chunk, pairs);
        if r != CommandResult::Done {
            return r;
        }
    }
    CommandResult::Done
}

/// 1 回の入力 (組の文字 1 つか、組の文字を含まない書記素の並び) を各 Range に当てる。
fn insert_chunk(
    state: &mut EditorState,
    view_id: ViewId,
    doc_id: DocumentId,
    chunk: &str,
    pairs: &[(char, char)],
) -> CommandResult {
    let (Some(doc), Some(view)) = (state.document(doc_id), state.view(view_id)) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let text = doc.text();
    let before = view.selection.clone();
    let memory = &view.auto_closed;
    let pair_char = single_char(chunk).filter(|c| is_pair_char(pairs, *c));
    let ins: Arc<str> = chunk.into();
    let acts: Vec<Act> = before
        .iter()
        .map(|r| {
            let put = || Act::Put {
                del: ByteRange::new(r.start(), r.end()),
                ins: Arc::clone(&ins),
            };
            let Some(ch) = pair_char else {
                return put();
            };
            if !r.is_empty() {
                // 選択があれば、開き字は囲み、ほかの文字は今のまま置き換える (APR-5)
                return match closer_of(pairs, ch) {
                    Some(close) => Act::Wrap {
                        range: *r,
                        open: ch,
                        close,
                    },
                    None => put(),
                };
            }
            let p = r.head;
            // 自動で足した同じ閉じ字の前なら飛び越える (APR-3)
            if let Ok(i) = memory.binary_search_by_key(&p, |a| a.close)
                && memory[i].close_char == ch
                && char_at(text, p, ch)
            {
                return Act::Skip { at: p, close: ch };
            }
            // 開き字で、直後と (引用符なら) 直前が条件に合えば対にする (APR-2)
            if let Some(close) = closer_of(pairs, ch)
                && next_allows(text, p, pairs)
                && (ch != close || prev_allows_quote(text, p, ch))
            {
                return Act::Pair {
                    at: p,
                    open: ch,
                    close,
                };
            }
            put()
        })
        .collect();
    let step = build(acts);
    apply_step(state, view_id, doc_id, before, step)
}

/// `edit.delete_backward` の対の間の削除 (APR-4)。自動閉じが効いていて、ある空の Range の直前が開き字・直後がそれと組の
/// 自動で足した閉じ字なら、その Range は両方を消し、ほかの Range は `span` (今の 1 書記素・コードポイントの削除) で消す。
/// 当たる Range が無ければ `None` (呼び出し側が今の削除を行う)。
pub(crate) fn delete_backward_pairs(
    ctx: &mut CommandCtx<'_>,
    view_id: ViewId,
    doc_id: DocumentId,
    span: &dyn Fn(&Document, ByteOff) -> ByteRange,
) -> Option<CommandResult> {
    let doc = ctx.state.document(doc_id)?;
    let view = ctx.state.view(view_id)?;
    if view.auto_closed.is_empty() {
        return None;
    }
    let pairs = pairs_for(ctx.state, doc)?;
    let text = doc.text();
    let memory = &view.auto_closed;
    let pair_at = |r: &Range| -> Option<AutoClosed> {
        if !r.is_empty() {
            return None;
        }
        let i = memory.binary_search_by_key(&r.head, |a| a.close).ok()?;
        let a = memory[i];
        (a.open_end() == r.head
            && pairs.contains(&(a.open_char, a.close_char))
            && char_at(text, a.open, a.open_char)
            && char_at(text, a.close, a.close_char))
        .then_some(a)
    };
    let before = view.selection.clone();
    if !before.iter().any(|r| pair_at(r).is_some()) {
        return None;
    }
    let acts: Vec<Act> = before
        .iter()
        .map(|r| match pair_at(r) {
            Some(a) => Act::PairDelete {
                open_at: a.open,
                open: a.open_char,
                close: a.close_char,
            },
            None => Act::Delete {
                del: if r.is_empty() {
                    span(doc, r.head)
                } else {
                    ByteRange::new(r.start(), r.end())
                },
            },
        })
        .collect();
    let step = build(acts);
    Some(apply_step(ctx.state, view_id, doc_id, before, step))
}

#[cfg(test)]
mod tests {
    use super::*;
    use eve_text::Text;

    #[test]
    fn 組の文字は_1_つずつ_ほかはまとめる() {
        let pairs = DEFAULT_PAIRS;
        assert_eq!(chunks("ab(cd)e", &pairs), ["ab", "(", "cd", ")", "e"]);
        assert_eq!(chunks("((", &pairs), ["(", "("]);
        assert_eq!(chunks("日本", &pairs), ["日本"]);
        // 結合文字の付いた括弧は 1 書記素だが組の文字ではない
        assert_eq!(chunks("(\u{301}a", &pairs), ["(\u{301}a"]);
    }

    #[test]
    fn 直後の文字の条件() {
        let pairs = DEFAULT_PAIRS;
        let t = Text::from("a b\u{3000}c)d}」e,f、g\"h\r\ni");
        let at = |s: &str| -> ByteOff {
            ByteOff(
                t.slice_to_string(ByteRange::new(ByteOff(0), ByteOff(t.len_bytes())))
                    .find(s)
                    .unwrap(),
            )
        };
        assert!(next_allows(&t, at(" b"), &pairs), "空白");
        assert!(next_allows(&t, at("\u{3000}"), &pairs), "全角の空白");
        assert!(next_allows(&t, at(")"), &pairs), "閉じ字");
        assert!(next_allows(&t, at("」"), &pairs), "全角の閉じ括弧");
        assert!(next_allows(&t, at(","), &pairs), "句読点");
        assert!(next_allows(&t, at("、"), &pairs), "読点");
        assert!(next_allows(&t, at("\r\n"), &pairs), "行末");
        assert!(
            next_allows(&t, ByteOff(t.len_bytes()), &pairs),
            "文書の末尾"
        );
        assert!(!next_allows(&t, at("b"), &pairs), "語の直前");
        assert!(
            !next_allows(&t, at("\"h"), &pairs),
            "引用符は閉じ字に数えない"
        );
    }

    #[test]
    fn 写像で開き字か閉じ字が消えたら忘れる() {
        let a = AutoClosed {
            open: ByteOff(0),
            open_char: '(',
            close: ByteOff(1),
            close_char: ')',
        };
        let base = Text::from("()");
        // 間に入れる: 閉じ字だけが後ろへ
        let mut l = vec![a];
        let cs = ChangeSet::from_sorted_edits(
            &base,
            [Edit {
                range: ByteRange::new(ByteOff(1), ByteOff(1)),
                insert: "xy".into(),
            }],
        )
        .unwrap();
        map_auto_closed(&mut l, &cs);
        assert_eq!((l[0].open, l[0].close), (ByteOff(0), ByteOff(3)));
        // 閉じ字を消す
        let mut l = vec![a];
        let cs = ChangeSet::from_sorted_edits(
            &base,
            [Edit {
                range: ByteRange::new(ByteOff(1), ByteOff(2)),
                insert: ")".into(),
            }],
        )
        .unwrap();
        map_auto_closed(&mut l, &cs);
        assert!(l.is_empty(), "置き換えた閉じ字は自動で足したものではない");
    }
}
