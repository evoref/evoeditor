//! テキスト変換のコマンドの実行 (docs/03 §15.8 の TXT-1・TXT-5〜TXT-7、01 FR-214)。
//!
//! 対象 (TXT-1) を決め、対象の合計が `TRANSFORM_SYNC_BYTES` (1 MiB) 以下ならメインスレッドで、超えれば `bg` のジョブ
//! (`JobKind::TextTransform`) で `ChangeSet` を作る (TXT-6)。ジョブを待つ間は、投入したときの対象の範囲を文書の変更の
//! たびに写像しておき、結果の Revision が古ければ、写像した範囲で今の本文から 1 回だけ作り直す (今の選択は読まない)。
//! 再読込 (`Origin::Reload` の置き換え) を始めたら、待っている変換は取り消す。適用は 1 つの Transaction
//! (`Origin::Command`・`UndoGroupHint::Break`) で、選択は TXT-5 のとおりに置く。
//!
//! 適用の後の選択は、Range の数と `ChangeSet` の演算の数に比例する費用で決める (位置の写像は昇順に 1 回だけ進める。文書の
//! 大きさに比例する複写と走査をしない)。単語の位置と、文書の全体の並べ替えの主のカーソルは、適用の後の本文で、その単語・
//! その行の書記素だけを数えて決める。
//!
//! spec: 03 §15.8, 03 §17.2, 02 §9.3, 03 §4.5.2

// document/ の中に置くが、文書テキストを直接変える経路 (`eve_text::Text::apply`) は使わない
// (document/mod.rs の許可を受け継がない。変更は Transaction を通す、02 §5.2)
#![deny(clippy::disallowed_methods)]

use eve_text::{
    Bias, ByteOff, ByteRange, ChangeSet, Op, Range, Selection, TextRead, next_grapheme_boundary,
    word_at,
};
use eve_view::{CommandId, DocumentId, MessageLevel, Revision, ViewId};

use crate::command::{CommandResult, RejectReason, ids};
use crate::document::transform::{
    TransformBuilt, TransformOp, block_content, block_range, build_text_transform, line_blocks,
};
use crate::document::{Document, Origin, Transaction, UndoGroupHint};
use crate::editor::state::EditorState;
use crate::io::pending::Pending;
use crate::jobs::cancel::CancelToken;
use crate::jobs::{JobError, JobKind, JobResult};
use crate::ui_model::keys;
use crate::view::multi_cursor::WORD_WINDOW;

/// 変換の対象 (TXT-1)。
#[derive(Clone, Debug)]
pub(crate) enum Target {
    /// 文書の全体 (並べ替え・末尾空白の除去で、全 Range が空のとき)。作り直しでは今の文書の全体。
    Whole,
    /// 対象の範囲 (昇順)。大文字・小文字では空でない Range か、カーソルの位置の単語 (`words`)。行の変換では行のブロックの
    /// 範囲 (`block_range`)。ジョブを待つ間は文書の変更で写像する (始まりは後ろ寄り、終わりは前寄り。消えると長さ 0 になる)。
    Ranges {
        ranges: Vec<ByteRange>,
        /// 全 Range が空で、カーソルの位置の単語を選んだ (TXT-1 (a)。選択の置き方が変わる、TXT-5)。
        words: bool,
    },
}

/// テキスト変換の依頼 (ジョブの結果を受けるときまで持つ。`Pending::TextTransform`)。
#[derive(Clone, Debug)]
pub(crate) struct TransformAsk {
    pub doc: DocumentId,
    /// コマンドを実行した View (選択を置く)。
    pub view: ViewId,
    pub op: TransformOp,
    pub target: Target,
    /// `ChangeSet` を作った Revision。
    pub revision: Revision,
    /// 作り直した (一度古くなった)。
    pub retried: bool,
}

/// 範囲の本文が区切り (`\r`・`\n`) だけか (空を含む)。最初の区切りでないバイトで止める。
fn only_line_breaks(t: &dyn TextRead, r: ByteRange) -> bool {
    t.chunks(r)
        .all(|c| c.bytes().all(|b| b == b'\r' || b == b'\n'))
}

impl TransformAsk {
    /// 写像した範囲が消えた (TXT-6)。Range の長さが 0 になった、または行の変換のブロックに区切りだけが残った (行の中身が
    /// 1 つも残らない)。どれか 1 つでも消えたら、残りだけを変換せずに全体を諦める (頼んだ範囲の一部だけを変えない)。
    fn target_gone(&self, t: &dyn TextRead) -> bool {
        match &self.target {
            Target::Whole => false,
            Target::Ranges { ranges, .. } => ranges
                .iter()
                .any(|r| r.is_empty() || (self.op.is_line_op() && only_line_breaks(t, *r))),
        }
    }
}

/// 変換のコマンドの ID (Transaction の `Origin::Command`)。
fn command_id(op: TransformOp) -> &'static str {
    match op {
        TransformOp::Upper => ids::EDIT_TRANSFORM_UPPER,
        TransformOp::Lower => ids::EDIT_TRANSFORM_LOWER,
        TransformOp::SortAscending => ids::EDIT_SORT_LINES_ASCENDING,
        TransformOp::SortDescending => ids::EDIT_SORT_LINES_DESCENDING,
        TransformOp::TrimTrailing => ids::EDIT_TRIM_TRAILING_WHITESPACE,
    }
}

/// 各カーソルの位置の単語 (TXT-1 (a)。03 §4.5.1 の MCUR-2 (a) と同じ選び方。単語の無いカーソルは飛ばす。同じ単語は 1 回)。
fn cursor_words(t: &dyn TextRead, sel: &Selection) -> Vec<ByteRange> {
    let mut heads: Vec<ByteOff> = sel.iter().map(|r| r.head).collect();
    heads.sort_unstable();
    let mut out: Vec<ByteRange> = Vec::new();
    for h in heads {
        // 同じ単語の中の続くカーソルは、前のカーソルの単語を使う (同じ長い単語をカーソルごとに読まない)
        if out.last().is_some_and(|w| w.start <= h && h < w.end) {
            continue;
        }
        if let Some(w) = word_at(t, h, WORD_WINDOW)
            && out.last() != Some(&w)
        {
            out.push(w);
        }
    }
    out
}

/// 今の本文での対象の範囲: 文書の全体は `[0, len)`、行の変換はブロックの範囲に直したもの (写像した範囲が行頭で終わらなく
/// なっても、触れる行のブロックにする)。
fn target_ranges(t: &dyn TextRead, op: TransformOp, target: &Target) -> Vec<ByteRange> {
    let ranges = match target {
        Target::Whole => return vec![ByteRange::all(t.len_bytes())],
        Target::Ranges { ranges, .. } => ranges,
    };
    if op.is_line_op() {
        line_blocks(t, ranges)
            .into_iter()
            .map(|b| block_range(t, b))
            .collect()
    } else {
        ranges.clone()
    }
}

/// 対象の合計の大きさ (TXT-6: Range の長さの和・ブロックの長さの和。ブロックの長さは最後の行の区切りを数えない)。
fn target_size(t: &dyn TextRead, op: TransformOp, ranges: &[ByteRange]) -> usize {
    if op.is_line_op() {
        line_blocks(t, ranges)
            .into_iter()
            .map(|b| block_content(t, b).len())
            .sum()
    } else {
        ranges.iter().map(ByteRange::len).sum()
    }
}

/// `start` から `end` まで書記素を数える (`end` は書記素の境界)。
fn graphemes_between(t: &dyn TextRead, start: ByteOff, end: ByteOff) -> usize {
    let (mut p, mut n) = (start, 0usize);
    while p < end {
        let q = next_grapheme_boundary(t, p);
        if q <= p {
            break;
        }
        p = q;
        n += 1;
    }
    n
}

/// 昇順の位置を `ChangeSet` で写す (`ChangeSet::map_pos` と同じ結果)。通り過ぎた演算は読み直さないので、昇順に問い合わせる
/// 限り、全体の費用は問い合わせの数と演算の数の和に比例する。昇順でない問い合わせは `map_pos` で答える。
struct Mapper<'a> {
    cs: &'a ChangeSet,
    /// 次に読む演算と、その演算の始まりの (変換の前, 変換の後) の位置。
    i: usize,
    old: usize,
    new: usize,
}

impl<'a> Mapper<'a> {
    fn new(cs: &'a ChangeSet) -> Mapper<'a> {
        Mapper {
            cs,
            i: 0,
            old: 0,
            new: 0,
        }
    }

    fn map(&mut self, p: ByteOff, bias: Bias) -> ByteOff {
        let p = p.0;
        if p < self.old {
            return self.cs.map_pos(ByteOff(p), bias);
        }
        let ops = self.cs.ops();
        while let Some(op) = ops.get(self.i) {
            match op {
                Op::Retain(k) => {
                    if p < self.old + k {
                        return ByteOff(self.new + (p - self.old));
                    }
                    self.old += k;
                    self.new += k;
                }
                Op::Delete(k) => {
                    if p <= self.old + k {
                        return match (bias, ops.get(self.i + 1)) {
                            (Bias::After, Some(Op::Insert(s))) => ByteOff(self.new + s.len()),
                            _ => ByteOff(self.new),
                        };
                    }
                    self.old += k;
                }
                Op::Insert(s) => {
                    if p == self.old {
                        return match bias {
                            Bias::Before => ByteOff(self.new),
                            Bias::After => ByteOff(self.new + s.len()),
                        };
                    }
                    self.new += s.len();
                }
            }
            self.i += 1;
        }
        ByteOff(self.new)
    }
}

/// 変換の後の選択の Range の置き方 (適用の前に決め、適用の後の本文で位置にする)。
enum Place {
    /// 写像で決まる Range。
    Mapped(Range),
    /// 単語を選んだ空のカーソル: 変換の後の単語 `[start, end)` の始まりから書記素を `n` 個進めた位置 (収まらなければ
    /// 単語の末尾。TXT-5)。
    Word {
        start: ByteOff,
        end: ByteOff,
        n: usize,
    },
    /// 行 `line` (0 始まり) の桁 `col` (1 始まりの書記素の数。行が短ければ行末)。
    LineCol { line: usize, col: usize },
}

/// 空でない Range: 始まりを前寄り・終わりを後ろ寄りに写し、向きを保つ (変換した文字列の全体を選ぶ。待つ間に選択を変えて
/// いても同じ規則)。
fn map_spanning(m: &mut Mapper<'_>, r: &Range) -> Range {
    let s = m.map(r.start(), Bias::Before);
    let e = m.map(r.end(), Bias::After);
    if r.is_forward() {
        Range::new(s, e)
    } else {
        Range::new(e, s)
    }
}

/// 自分の編集として写す (`MapPolicy::Own`。消した空白の中のカーソルは行末へ寄る)。
fn map_own(m: &mut Mapper<'_>, r: &Range) -> Range {
    let (s, e) = (m.map(r.start(), Bias::After), m.map(r.end(), Bias::After));
    if r.is_forward() {
        Range::new(s, e)
    } else {
        Range::new(e, s)
    }
}

/// 変換の後の選択の置き方 (TXT-5) と主の Range の番号。`doc` は変換の前の文書、`ranges` は `ChangeSet` を作った対象の範囲
/// ([`target_ranges`]。昇順)。選択と対象は、どちらも昇順に 1 回だけ進めて照らす。
fn plan_selection(
    doc: &Document,
    ask: &TransformAsk,
    ranges: &[ByteRange],
    before: &Selection,
    cs: &ChangeSet,
) -> (Vec<Place>, usize) {
    let t = doc.text();
    if matches!(
        (ask.op, &ask.target),
        (
            TransformOp::SortAscending | TransformOp::SortDescending,
            Target::Whole
        )
    ) {
        // 文書の全体の並べ替え: 主のカーソルを同じ行・桁 (書記素) に置いた 1 つのカーソル (区切りは動かないので行の番号は
        // 変わらない)。桁は今の行の書記素だけを数える (長大行は列の索引、03 §14.4)
        let head = before.primary().head;
        let line = t.byte_to_line(head).0;
        let col = crate::ui_model::chrome::grapheme_column(doc, head);
        return (vec![Place::LineCol { line, col }], 0);
    }
    let rs = before.ranges();
    let mut order: Vec<usize> = (0..rs.len()).collect();
    order.sort_by_key(|&i| (rs[i].start(), rs[i].end()));
    let mut m = Mapper::new(cs);
    let mut out: Vec<Option<Place>> = (0..rs.len()).map(|_| None).collect();
    let mut j = 0usize;
    match ask.op {
        TransformOp::Upper | TransformOp::Lower => {
            let words = matches!(ask.target, Target::Ranges { words: true, .. });
            // 同じ単語の続くカーソルは、前のカーソルの写した単語を使う (写像の問い合わせを昇順に保つ)
            let mut last: Option<(usize, ByteOff, ByteOff)> = None;
            for i in order {
                let r = &rs[i];
                let place = if !r.is_empty() {
                    Place::Mapped(map_spanning(&mut m, r))
                } else if words {
                    while j < ranges.len() && ranges[j].end < r.head {
                        j += 1;
                    }
                    match ranges.get(j).filter(|w| w.start <= r.head) {
                        Some(w) => {
                            let (start, end) = match last {
                                Some((k, s, e)) if k == j => (s, e),
                                _ => (m.map(w.start, Bias::Before), m.map(w.end, Bias::After)),
                            };
                            last = Some((j, start, end));
                            Place::Word {
                                start,
                                end,
                                n: graphemes_between(t, w.start, r.head),
                            }
                        }
                        None => Place::Mapped(map_own(&mut m, r)),
                    }
                } else {
                    Place::Mapped(map_own(&mut m, r))
                };
                out[i] = Some(place);
            }
        }
        TransformOp::SortAscending | TransformOp::SortDescending => {
            // 各ブロックの Range はブロックの全体 (最初の行の行頭から最後の行の区切りの直前まで、前向き)
            let blocks = line_blocks(t, ranges);
            let mut last: Option<(usize, Range)> = None;
            for i in order {
                let r = &rs[i];
                let line = t.byte_to_line(r.start()).0;
                while j < blocks.len() && blocks[j].1 < line {
                    j += 1;
                }
                let place = match blocks.get(j).filter(|b| b.0 <= line) {
                    Some(&b) => {
                        let range = match last {
                            Some((k, r)) if k == j => r,
                            _ => {
                                let c = block_content(t, b);
                                Range::new(m.map(c.start, Bias::Before), m.map(c.end, Bias::After))
                            }
                        };
                        last = Some((j, range));
                        Place::Mapped(range)
                    }
                    None => Place::Mapped(map_own(&mut m, r)),
                };
                out[i] = Some(place);
            }
        }
        TransformOp::TrimTrailing => {
            for i in order {
                out[i] = Some(Place::Mapped(map_own(&mut m, &rs[i])));
            }
        }
    }
    let places = out
        .into_iter()
        .zip(rs)
        .map(|(p, r)| p.unwrap_or(Place::Mapped(*r)))
        .collect();
    (places, before.primary_index())
}

/// 置き方を位置にする (`t` は適用の後の本文)。
fn resolve(t: &eve_text::Text, places: Vec<Place>, primary: usize) -> Selection {
    let ranges: Vec<Range> = places
        .into_iter()
        .map(|p| match p {
            Place::Mapped(r) => r,
            Place::Word { start, end, n } => {
                let mut p = start;
                for _ in 0..n {
                    let q = next_grapheme_boundary(t, p);
                    if q <= p || q > end {
                        p = end;
                        break;
                    }
                    p = q;
                }
                Range::point(p.min(end))
            }
            Place::LineCol { line, col } => {
                let line = line.min(t.len_lines().saturating_sub(1));
                Range::point(crate::view::motion::line_col_byte(t, line, col))
            }
        })
        .collect();
    let len = t.len_bytes();
    Selection::new(ranges, primary).map_or_else(|| Selection::point(ByteOff(0)), |s| s.merged(len))
}

/// 置き方が、適用の後の本文を読まずに決まる (写像だけ) なら、その選択。
fn mapped_only(places: &[Place], primary: usize, len_after: usize) -> Option<Selection> {
    let ranges: Option<Vec<Range>> = places
        .iter()
        .map(|p| match p {
            Place::Mapped(r) => Some(*r),
            _ => None,
        })
        .collect();
    let sel = Selection::new(ranges?, primary)?;
    Some(sel.merged(len_after))
}

impl EditorState {
    /// テキスト変換のコマンド (03 §15.8)。`view` はフォーカスのペインの View、`doc` はその文書。
    pub(crate) fn text_transform(
        &mut self,
        view: ViewId,
        doc: DocumentId,
        op: TransformOp,
    ) -> CommandResult {
        let (Some(d), Some(v)) = (self.document(doc), self.view(view)) else {
            return CommandResult::Rejected(RejectReason::NoTarget);
        };
        // `when` (`!doc.read_only`) で断るが、念のため (読込中を含む。TXT-7)
        if !d.is_editable() {
            self.notify_read_only_of(Some(doc));
            return CommandResult::Done;
        }
        let t = d.text();
        let sel = &v.selection;
        let mut nonempty: Vec<ByteRange> = sel
            .iter()
            .filter(|r| !r.is_empty())
            .map(|r| ByteRange::new(r.start(), r.end()))
            .collect();
        nonempty.sort_unstable_by_key(|r| (r.start, r.end));
        let target = if !nonempty.is_empty() {
            Target::Ranges {
                ranges: nonempty,
                words: false,
            }
        } else if op.is_line_op() {
            Target::Whole
        } else {
            let words = cursor_words(t, sel);
            if words.is_empty() {
                self.show_message(MessageLevel::Info, keys::INFO_SELECT_NO_WORD, &[]);
                return CommandResult::Done;
            }
            Target::Ranges {
                ranges: words,
                words: true,
            }
        };
        let revision = d.revision();
        // 同じ文書に新しい変換を頼んだら、前のジョブを取り消す (TXT-6)
        self.cancel_text_transform(doc);
        self.run_text_transform(TransformAsk {
            doc,
            view,
            op,
            target,
            revision,
            retried: false,
        });
        CommandResult::Done
    }

    /// 文書 `doc` の変換のジョブを取り消す (結果は捨てる)。取り消したジョブがあれば `true`。
    fn cancel_text_transform(&mut self, doc: DocumentId) -> bool {
        let jobs: Vec<_> = self
            .io
            .pending
            .iter()
            .filter(|(_, p)| matches!(p, Pending::TextTransform(a) if a.doc == doc))
            .map(|(id, _)| *id)
            .collect();
        let any = !jobs.is_empty();
        for id in jobs {
            self.io.pending.remove(&id);
            self.jobs.cancel(id);
        }
        any
    }

    /// 文書 `doc` の再読込 (`Origin::Reload` の置き換え: `file.reload`・`file.reopen_with_encoding`・監視の読み直し) を
    /// 始めた: 待っている変換を取り消し、範囲が無くなった旨を出す (TXT-6。読み直した本文を変換しない)。
    pub(crate) fn cancel_text_transform_for_reload(&mut self, doc: DocumentId) {
        if self.cancel_text_transform(doc) {
            self.show_message(MessageLevel::Warn, keys::MSG_TRANSFORM_TARGET_GONE, &[]);
            self.mark_dirty_render();
        }
    }

    /// 今の本文で `ChangeSet` を作る。対象の合計が上限以下ならその場で適用し、超えればジョブを投入する (TXT-6)。
    fn run_text_transform(&mut self, mut ask: TransformAsk) {
        let Some(d) = self.docs.documents.get(&ask.doc) else {
            return;
        };
        // 作り直しの時点で読み取り専用・読込中になっていれば作らない (TXT-6)
        if !d.is_editable() {
            self.notify_read_only_of(Some(ask.doc));
            return;
        }
        let t = d.text();
        ask.revision = d.revision();
        let ranges = target_ranges(t, ask.op, &ask.target);
        if target_size(t, ask.op, &ranges) <= self.io.transform_sync_bytes {
            match build_text_transform(t, ask.op, &ranges, &CancelToken::new()) {
                Ok(built) => self.apply_text_transform(&ask, &ranges, built),
                Err(_) => {
                    self.show_message(MessageLevel::Warn, keys::MSG_TRANSFORM_FAILED, &[]);
                }
            }
            return;
        }
        let snapshot = d.snapshot();
        let job = self.jobs.spawn(
            Some(ask.doc),
            ask.revision,
            JobKind::TextTransform {
                snapshot,
                op: ask.op,
                ranges: ranges.clone(),
            },
        );
        // 写像するのは、今の本文で決め直した範囲 (行の変換はブロックの範囲)
        if let Target::Ranges { ranges: r, .. } = &mut ask.target {
            *r = ranges;
        }
        self.io
            .pending
            .insert(job, Pending::TextTransform(Box::new(ask)));
        self.mark_dirty_render();
    }

    /// ジョブの結果 (02 §9.3 の採否の表、03 §17.2)。Revision が投入のときのままなら適用し、古ければ写像した範囲で今の本文から
    /// 1 回だけ作り直す。作り直しも古ければ諦める。
    pub(crate) fn finish_text_transform(&mut self, ask: TransformAsk, r: JobResult) {
        match r {
            JobResult::TextTransformDone {
                changes,
                sort_skipped_cr_lf,
            } => {
                let Some(d) = self.docs.documents.get(&ask.doc) else {
                    return;
                };
                if d.revision() == ask.revision {
                    let ranges = target_ranges(d.text(), ask.op, &ask.target);
                    let built = TransformBuilt {
                        changes,
                        sort_skipped_cr_lf,
                    };
                    self.apply_text_transform(&ask, &ranges, built);
                } else if ask.retried {
                    self.show_message(MessageLevel::Warn, keys::MSG_TRANSFORM_STALE, &[]);
                } else if ask.target_gone(d.text()) {
                    self.show_message(MessageLevel::Warn, keys::MSG_TRANSFORM_TARGET_GONE, &[]);
                } else {
                    self.run_text_transform(TransformAsk {
                        retried: true,
                        ..ask
                    });
                }
            }
            JobResult::Failed {
                err: JobError::Cancelled,
            } => {}
            JobResult::Failed { .. } => {
                self.show_message(MessageLevel::Warn, keys::MSG_TRANSFORM_FAILED, &[]);
            }
            _ => {}
        }
        self.mark_dirty_render();
    }

    /// `ChangeSet` を 1 つの Transaction として適用する (TXT-5)。変わる箇所が無ければ文書を変えずに知らせる。
    fn apply_text_transform(
        &mut self,
        ask: &TransformAsk,
        ranges: &[ByteRange],
        built: TransformBuilt,
    ) {
        let TransformBuilt {
            changes: cs,
            sort_skipped_cr_lf,
        } = built;
        if cs.is_identity() {
            let key = if sort_skipped_cr_lf {
                keys::INFO_TRANSFORM_SORT_SKIPPED_CR_LF
            } else {
                keys::INFO_TRANSFORM_UNCHANGED
            };
            self.show_message(MessageLevel::Info, key, &[]);
            return;
        }
        let Some(d) = self.docs.documents.get(&ask.doc) else {
            return;
        };
        // ジョブを待つ間に読み取り専用・読込中になった: 編集と同じ読み取り専用の知らせ (前に出ていない文書でも)
        if !d.is_editable() {
            self.notify_read_only_of(Some(ask.doc));
            return;
        }
        let view = Some(ask.view).filter(|v| self.view(*v).is_some_and(|v| v.doc == ask.doc));
        let before = view.and_then(|v| self.view(v)).map(|v| v.selection.clone());
        let plan = before
            .as_ref()
            .map(|b| plan_selection(d, ask, ranges, b, &cs));
        let after = plan
            .as_ref()
            .and_then(|(places, primary)| mapped_only(places, *primary, cs.len_after()));
        let mut tx = Transaction::new(
            cs,
            Origin::Command(CommandId::new(command_id(ask.op))),
            self.now(),
        )
        .with_undo_group(UndoGroupHint::Break)
        .with_selections(before, after.clone());
        if let Some(v) = view {
            tx = tx.with_view(v);
        }
        if self.apply_transaction(ask.doc, tx).is_err() {
            self.show_message(MessageLevel::Warn, keys::MSG_TRANSFORM_FAILED, &[]);
            return;
        }
        // 単語の位置・行と桁は、適用の後の本文で決めて、Undo のノードの「後の選択」(Redo で戻す選択) も差し替える
        if after.is_none()
            && let (Some(v), Some((places, primary))) = (view, plan)
            && let Some(d) = self.docs.documents.get(&ask.doc)
        {
            let sel = resolve(d.text(), places, primary).normalized(d.text());
            let node = d.history().current();
            if let Some(d) = self.docs.documents.get_mut(&ask.doc) {
                d.replace_selection_after(node, sel.clone());
            }
            self.set_selection(v, sel);
        }
        if sort_skipped_cr_lf {
            self.show_message(
                MessageLevel::Info,
                keys::INFO_TRANSFORM_SORT_SKIPPED_CR_LF,
                &[],
            );
        }
    }

    /// 文書の変更で、待っている変換の対象の範囲を写像する (TXT-6: 作り直すときは、投入したときの範囲をその後の
    /// `ChangeSet` で写像した範囲を使う)。始まりは後ろ寄り・終わりは前寄りで写し、端に足した文字を対象に入れない。
    pub(crate) fn map_text_transform(&mut self, doc: DocumentId, cs: &ChangeSet) {
        for p in self.io.pending.values_mut() {
            if let Pending::TextTransform(ask) = p
                && ask.doc == doc
                && let Target::Ranges { ranges, .. } = &mut ask.target
            {
                // 範囲は昇順で重ならないので、始まりと終わりの問い合わせも昇順
                let mut m = Mapper::new(cs);
                for r in ranges.iter_mut() {
                    let s = m.map(r.start, Bias::After);
                    let e = m.map(r.end, Bias::Before);
                    *r = ByteRange::new(s, e.max(s));
                }
            }
        }
    }

    /// 文書 `doc` の変換のジョブを待っている (ステータスバーの `[変換中]`、04 §8.7 の左 6)。
    pub(crate) fn is_transforming(&self, doc: DocumentId) -> bool {
        self.io
            .pending
            .values()
            .any(|p| matches!(p, Pending::TextTransform(a) if a.doc == doc))
    }

    /// テキスト変換をメインスレッドで作る上限 (`TRANSFORM_SYNC_BYTES`) を試験から小さくする (feature `test-hooks` の口。
    /// 配布ビルドには無い。既定の値より大きくはしない。03 §15.9.1 の (e))。
    #[cfg(feature = "test-hooks")]
    #[doc(hidden)]
    pub fn set_transform_sync_bytes(&mut self, bytes: usize) {
        self.io.transform_sync_bytes = bytes.min(crate::document::transform::TRANSFORM_SYNC_BYTES);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eve_text::{Edit, Snapshot};

    #[test]
    fn 昇順の写像は_map_pos_と同じ() {
        let t = Snapshot::from("abcdefghij");
        let cs = ChangeSet::from_sorted_edits(
            &t,
            [
                Edit {
                    range: ByteRange::new(ByteOff(1), ByteOff(3)),
                    insert: "XYZ".into(),
                },
                Edit {
                    range: ByteRange::new(ByteOff(5), ByteOff(5)),
                    insert: "Q".into(),
                },
                Edit {
                    range: ByteRange::new(ByteOff(7), ByteOff(9)),
                    insert: "".into(),
                },
            ],
        )
        .expect("編集");
        for bias in [Bias::Before, Bias::After] {
            let mut m = Mapper::new(&cs);
            for p in 0..=11 {
                assert_eq!(
                    m.map(ByteOff(p), bias),
                    cs.map_pos(ByteOff(p), bias),
                    "{p} {bias:?}"
                );
            }
        }
        // 前後の向きを混ぜた昇順の問い合わせ (同じ位置を Before → After)
        let mut m = Mapper::new(&cs);
        for p in 0..=11 {
            for bias in [Bias::Before, Bias::After] {
                assert_eq!(m.map(ByteOff(p), bias), cs.map_pos(ByteOff(p), bias));
            }
        }
        // 昇順でない問い合わせも同じ答え
        assert_eq!(
            m.map(ByteOff(2), Bias::After),
            cs.map_pos(ByteOff(2), Bias::After)
        );
    }
}
