//! `ChangeSet`: 文書全体を覆う操作列 (Retain / Delete / Insert) による変更の表現 (docs/03-core-design.md §6.1〜§6.2・§6.4)。
//!
//! xi-editor / Helix / CodeMirror 6 と同じ族の表現で、合成 (`compose`)・逆変換 (`invert`)・位置写像 (`map_pos`) が
//! 代数的に定義できる。位置はすべて **適用前の文書のバイト位置**。
//!
//! 挿入は `InsertText::Str` (複写した文字列) と `InsertText::Rope` (rope を共有して複写しない)。`Rope` は文書全体の
//! 置き換え (再読込の新全文、全体の削除の旧全文) だけで使う (03 §6.5 / §9.5)。64 KiB 以上の削除の `Slice` は v0.4。

use std::fmt;
use std::sync::Arc;

use ropey::Rope;

use crate::text::{Snapshot, Text, TextRead};
use crate::types::{ByteOff, ByteRange};

/// 挿入する文字列。Undo 履歴と Transaction で共有する (コピーしない)。
/// `Debug` は本文を出さず、バイト数だけを出す (本文を log やパニックに出さない)。
/// 等しさは内容で比べる (`Str` と `Rope` でも、同じ内容なら等しい)。
#[derive(Clone)]
#[non_exhaustive]
pub enum InsertText {
    Str(Arc<str>),
    /// 大きな本文を複写せずに持つ (rope の節を共有する。`Snapshot` の複製は O(1))。
    Rope(Snapshot),
}

impl InsertText {
    /// 連続した文字列 (`Str`) のときだけ返す。`Rope` は `None` (全文を連結しない。`chunks` を使う)。
    pub fn as_str(&self) -> Option<&str> {
        match self {
            InsertText::Str(s) => Some(s),
            InsertText::Rope(_) => None,
        }
    }

    /// 内容を連続したチャンクで返す (空のチャンクは返さない)。
    pub fn chunks(&self) -> impl Iterator<Item = &str> + '_ {
        let (s, r) = match self {
            InsertText::Str(s) => (Some(&**s).filter(|s| !s.is_empty()), None),
            InsertText::Rope(r) => (None, Some(r.chunks(ByteRange::all(r.len_bytes())))),
        };
        s.into_iter().chain(r.into_iter().flatten())
    }

    pub fn len(&self) -> usize {
        match self {
            InsertText::Str(s) => s.len(),
            InsertText::Rope(r) => r.len_bytes(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn is_char_boundary(&self, b: usize) -> bool {
        match self {
            InsertText::Str(s) => s.is_char_boundary(b),
            InsertText::Rope(r) => r.is_char_boundary(ByteOff(b)),
        }
    }

    /// `a..b` の部分 (char 境界であること)。`Str` は複写し、`Rope` は節を共有する。
    fn slice(&self, a: usize, b: usize) -> InsertText {
        match self {
            InsertText::Str(s) => InsertText::from(&s[a..b]),
            InsertText::Rope(r) => {
                InsertText::Rope(Snapshot::from_rope(Rope::from(r.rope().byte_slice(a..b))))
            }
        }
    }

    /// rope にする (`Str` は複写する。`Rope` は共有する)。
    fn to_rope(&self) -> Rope {
        match self {
            InsertText::Str(s) => Rope::from_str(s),
            InsertText::Rope(r) => r.rope().clone(),
        }
    }
}

impl PartialEq for InsertText {
    fn eq(&self, other: &InsertText) -> bool {
        if let (Some(a), Some(b)) = (self.as_str(), other.as_str()) {
            return a == b;
        }
        self.len() == other.len()
            && self
                .chunks()
                .flat_map(str::bytes)
                .eq(other.chunks().flat_map(str::bytes))
    }
}

impl Eq for InsertText {}

impl From<&str> for InsertText {
    fn from(s: &str) -> InsertText {
        InsertText::Str(Arc::from(s))
    }
}

impl fmt::Debug for InsertText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "InsertText({} bytes)", self.len())
    }
}

/// 操作 1 つ。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    /// そのまま残すバイト数。
    Retain(usize),
    /// 削除するバイト数。
    Delete(usize),
    /// 挿入する文字列。
    Insert(InsertText),
}

/// 置換 1 件 (`range` を削除して `insert` を入れる)。`range.is_empty()` なら純挿入。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub range: ByteRange,
    pub insert: Arc<str>,
}

/// 位置写像のバイアス。挿入位置に一致する点を、挿入の前に置くか後ろに置くか。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bias {
    Before,
    After,
}

/// `map_range` の戻り値。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeMap {
    /// 変更が範囲と交差せず、端に接する挿入もない (端に接する削除は交差とみなさない)。新しい範囲。
    Mapped(ByteRange),
    /// 範囲の内側・境界・端で変更があった。分類だけを返し、意味付けは呼び出し側が決める。
    Overlapped {
        /// `Inside` / `Touch` では写像した範囲、`Straddle` では `None`。
        mapped: Option<ByteRange>,
        kind: OverlapKind,
    },
}

/// 交差の分類。後から分類を足せるように `non_exhaustive`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum OverlapKind {
    /// 範囲の内側で挿入、または範囲に収まる削除 (範囲全体の削除を含む) があった。写像した範囲は、
    /// 範囲の端の置換で挿入された文字を含まない。
    Inside,
    /// 削除が範囲の境界をまたぐ (範囲の一部だけが削除された、範囲を覆って削除された、
    /// または空の範囲 (位置) が削除・置換の内側にある)。範囲と完全に一致する削除は `Inside`。
    Straddle,
    /// 範囲の端 (`start` / `end`) に接する挿入があった。
    Touch,
}

/// `from_sorted_edits` のエラー。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditError {
    /// 編集が重なっている、または昇順に並んでいない。
    Overlap,
    /// 編集の端が char 境界でない。
    NotCharBoundary { pos: ByteOff },
    /// 編集の端が文書の範囲外、または `start > end`。
    OutOfRange,
}

/// `Text::apply` のエラー。エラーのとき、文書は何も変更されない。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeSetError {
    /// `len_before` が文書のバイト数と違う (古い版に基づく ChangeSet)。
    LenMismatch { expected: usize, actual: usize },
    /// 操作の境界が char 境界でない (INV-T1)。
    NotCharBoundary { pos: ByteOff },
}

/// `compose` のエラー。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComposeError {
    /// 前の `len_after` と次の `len_before` が違う。
    LenMismatch {
        left_after: usize,
        right_before: usize,
    },
    /// 次の ChangeSet の境界が、前の ChangeSet が挿入した文字列の char の途中にある。
    SplitInsideChar,
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EditError::Overlap => write!(f, "編集が重なっているか、昇順に並んでいません"),
            EditError::NotCharBoundary { pos } => {
                write!(f, "位置 {pos} は char 境界ではありません")
            }
            EditError::OutOfRange => write!(f, "編集の範囲が文書の範囲外です"),
        }
    }
}

impl fmt::Display for ChangeSetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChangeSetError::LenMismatch { expected, actual } => {
                write!(
                    f,
                    "ChangeSet の len_before ({expected}) が文書のバイト数 ({actual}) と違います"
                )
            }
            ChangeSetError::NotCharBoundary { pos } => {
                write!(f, "位置 {pos} は char 境界ではありません")
            }
        }
    }
}

impl fmt::Display for ComposeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ComposeError::LenMismatch {
                left_after,
                right_before,
            } => {
                write!(
                    f,
                    "合成できません: 前の len_after ({left_after}) と次の len_before ({right_before}) が違います"
                )
            }
            ComposeError::SplitInsideChar => write!(
                f,
                "合成できません: 次の境界が、挿入した文字列の char の途中にあります"
            ),
        }
    }
}

impl std::error::Error for EditError {}
impl std::error::Error for ChangeSetError {}
impl std::error::Error for ComposeError {}

/// 文書全体を覆う操作列。`Retain` の合計 + `Delete` の合計 = `len_before`、
/// `Retain` の合計 + `Insert` の長さの合計 = `len_after`。
///
/// 常に **正規形** (CS6): 0 長の操作を持たず、隣接する同種の操作は結合され、`Delete` と `Insert` が
/// 隣接するときは `Delete` が先。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeSet {
    ops: Vec<Op>,
    len_before: usize,
    len_after: usize,
}

/// 操作列を正規形にして返す。
fn normalize(ops: impl IntoIterator<Item = Op>) -> Vec<Op> {
    fn push_delete(out: &mut Vec<Op>, n: usize) {
        if let Some(Op::Delete(m)) = out.last_mut() {
            *m = m.saturating_add(n);
        } else {
            out.push(Op::Delete(n));
        }
    }
    /// 結合中の挿入。1 つだけなら Arc を共有し、2 つ以上なら 1 つのバッファにためる (線形時間)。
    /// `Rope` が混ざれば rope をつなぐ (大きな本文を複写しない。`Str` の側だけ rope にする)。
    enum Pending {
        None,
        One(InsertText),
        Many(String),
        ManyRope(Rope),
    }
    impl Pending {
        fn push(&mut self, s: InsertText) {
            *self = match std::mem::replace(self, Pending::None) {
                Pending::None => Pending::One(s),
                Pending::One(prev) => match (prev.as_str(), s.as_str()) {
                    (Some(a), Some(b)) => {
                        let mut joined = String::with_capacity(a.len() + b.len());
                        joined.push_str(a);
                        joined.push_str(b);
                        Pending::Many(joined)
                    }
                    _ => {
                        let mut r = prev.to_rope();
                        r.append(s.to_rope());
                        Pending::ManyRope(r)
                    }
                },
                Pending::Many(mut buf) => match s.as_str() {
                    Some(b) => {
                        buf.push_str(b);
                        Pending::Many(buf)
                    }
                    None => {
                        let mut r = Rope::from_str(&buf);
                        r.append(s.to_rope());
                        Pending::ManyRope(r)
                    }
                },
                Pending::ManyRope(mut r) => {
                    r.append(s.to_rope());
                    Pending::ManyRope(r)
                }
            };
        }
        fn flush(&mut self, out: &mut Vec<Op>) {
            match std::mem::replace(self, Pending::None) {
                Pending::None => {}
                Pending::One(s) => out.push(Op::Insert(s)),
                Pending::Many(buf) => out.push(Op::Insert(InsertText::Str(Arc::from(buf)))),
                Pending::ManyRope(r) => {
                    out.push(Op::Insert(InsertText::Rope(Snapshot::from_rope(r))));
                }
            }
        }
    }
    let mut out: Vec<Op> = Vec::new();
    // Delete は Insert より先に置くので、挿入は Retain が来るか終わりになるまで保留する
    let mut pending = Pending::None;
    for op in ops {
        match op {
            Op::Retain(0) | Op::Delete(0) => {}
            Op::Retain(n) => {
                pending.flush(&mut out);
                if let Some(Op::Retain(m)) = out.last_mut() {
                    *m = m.saturating_add(n);
                } else {
                    out.push(Op::Retain(n));
                }
            }
            Op::Delete(n) => push_delete(&mut out, n),
            Op::Insert(s) => {
                if !s.is_empty() {
                    pending.push(s);
                }
            }
        }
    }
    pending.flush(&mut out);
    out
}

fn lengths(ops: &[Op]) -> (usize, usize) {
    let mut before = 0usize;
    let mut after = 0usize;
    for op in ops {
        match op {
            Op::Retain(n) => {
                before = before.saturating_add(*n);
                after = after.saturating_add(*n);
            }
            Op::Delete(n) => before = before.saturating_add(*n),
            Op::Insert(s) => after = after.saturating_add(s.len()),
        }
    }
    (before, after)
}

impl ChangeSet {
    /// 何も変えない ChangeSet。
    pub fn identity(len: usize) -> ChangeSet {
        ChangeSet::from_ops([Op::Retain(len)])
    }

    /// 操作列から作る。正規形にし、`len_before` / `len_after` を計算する。
    /// char 境界の検査は、文書に適用するとき (`Text::apply`) に行う。
    pub fn from_ops(ops: impl IntoIterator<Item = Op>) -> ChangeSet {
        let ops = normalize(ops);
        let (len_before, len_after) = lengths(&ops);
        ChangeSet {
            ops,
            len_before,
            len_after,
        }
    }

    /// 昇順で重ならない置換の列から作る。`len_before` は `base` のバイト数。
    /// 端が char 境界でないものは `NotCharBoundary`、重なりや昇順でない並びは `Overlap`
    /// (重なりを切り詰めない。切り詰めは呼び出し側が、渡す前に行う。03 §4.5)。
    pub fn from_sorted_edits(
        base: &dyn TextRead,
        edits: impl IntoIterator<Item = Edit>,
    ) -> Result<ChangeSet, EditError> {
        let len = base.len_bytes();
        let mut ops: Vec<Op> = Vec::new();
        let mut pos = 0usize;
        for e in edits {
            let (s, t) = (e.range.start.0, e.range.end.0);
            if s > t || t > len {
                return Err(EditError::OutOfRange);
            }
            if s < pos {
                return Err(EditError::Overlap);
            }
            for p in [s, t] {
                if !base.is_char_boundary(ByteOff(p)) {
                    return Err(EditError::NotCharBoundary { pos: ByteOff(p) });
                }
            }
            ops.push(Op::Retain(s - pos));
            ops.push(Op::Delete(t - s));
            ops.push(Op::Insert(InsertText::Str(e.insert)));
            pos = t;
        }
        ops.push(Op::Retain(len - pos));
        Ok(ChangeSet::from_ops(ops))
    }

    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    pub fn len_before(&self) -> usize {
        self.len_before
    }

    pub fn len_after(&self) -> usize {
        self.len_after
    }

    /// 何も変えないか (Delete も Insert も持たない)。
    pub fn is_identity(&self) -> bool {
        self.ops.iter().all(|op| matches!(op, Op::Retain(_)))
    }

    /// `self` を適用した後に `next` を適用するのと同じ、1 つの ChangeSet (CS1)。
    pub fn compose(&self, next: &ChangeSet) -> Result<ChangeSet, ComposeError> {
        if self.len_after != next.len_before {
            return Err(ComposeError::LenMismatch {
                left_after: self.len_after,
                right_before: next.len_before,
            });
        }
        /// 操作列の先頭から、操作の一部を消費しながら読むカーソル。
        struct Cursor<'a> {
            ops: &'a [Op],
            i: usize,
            off: usize,
        }
        enum Cur<'a> {
            Retain(usize),
            Delete(usize),
            /// 挿入 `ins` の `off` バイト目からが未消費の部分。
            Insert {
                ins: &'a InsertText,
                off: usize,
            },
        }
        /// 挿入の `off` から `k` バイトを取り出す。全体を使うなら共有し、一部なら `Str` は複写、`Rope` は節を共有する。
        fn take(ins: &InsertText, off: usize, k: usize) -> InsertText {
            if off == 0 && k == ins.len() {
                ins.clone()
            } else {
                ins.slice(off, off + k)
            }
        }
        impl<'a> Cursor<'a> {
            fn peek(&self) -> Option<Cur<'a>> {
                self.ops.get(self.i).map(|op| match op {
                    Op::Retain(n) => Cur::Retain(n - self.off),
                    Op::Delete(n) => Cur::Delete(n - self.off),
                    Op::Insert(s) => Cur::Insert {
                        ins: s,
                        off: self.off,
                    },
                })
            }
            fn advance(&mut self, n: usize) {
                let total = match &self.ops[self.i] {
                    Op::Retain(m) | Op::Delete(m) => *m,
                    Op::Insert(s) => s.len(),
                };
                self.off += n;
                if self.off >= total {
                    self.i += 1;
                    self.off = 0;
                }
            }
        }
        let mut a = Cursor {
            ops: &self.ops,
            i: 0,
            off: 0,
        };
        let mut b = Cursor {
            ops: &next.ops,
            i: 0,
            off: 0,
        };
        let mut out: Vec<Op> = Vec::new();
        loop {
            match (a.peek(), b.peek()) {
                (None, None) => break,
                // 前の削除は、そのまま合成後の削除になる
                (Some(Cur::Delete(n)), _) => {
                    out.push(Op::Delete(n));
                    a.advance(n);
                }
                // 次の挿入は、そのまま合成後の挿入になる
                (_, Some(Cur::Insert { ins, off })) => {
                    let rest = ins.len() - off;
                    out.push(Op::Insert(take(ins, off, rest)));
                    b.advance(rest);
                }
                (Some(Cur::Retain(n)), Some(Cur::Retain(m))) => {
                    let k = n.min(m);
                    out.push(Op::Retain(k));
                    a.advance(k);
                    b.advance(k);
                }
                (Some(Cur::Retain(n)), Some(Cur::Delete(m))) => {
                    let k = n.min(m);
                    out.push(Op::Delete(k));
                    a.advance(k);
                    b.advance(k);
                }
                (Some(Cur::Insert { ins, off }), Some(Cur::Retain(m))) => {
                    let k = (ins.len() - off).min(m);
                    if !ins.is_char_boundary(off + k) {
                        return Err(ComposeError::SplitInsideChar);
                    }
                    out.push(Op::Insert(take(ins, off, k)));
                    a.advance(k);
                    b.advance(k);
                }
                // 前が挿入して、次が削除する: 打ち消し合って何も残らない
                (Some(Cur::Insert { ins, off }), Some(Cur::Delete(m))) => {
                    let k = (ins.len() - off).min(m);
                    if !ins.is_char_boundary(off + k) {
                        return Err(ComposeError::SplitInsideChar);
                    }
                    a.advance(k);
                    b.advance(k);
                }
                // 長さが合っていれば起きない
                (None, Some(_)) | (Some(_), None) => {
                    return Err(ComposeError::LenMismatch {
                        left_after: self.len_after,
                        right_before: next.len_before,
                    });
                }
            }
        }
        Ok(ChangeSet::from_ops(out))
    }

    /// 適用を取り消す ChangeSet (CS2 / CS3)。`before` は `self` を適用する前の文書。
    /// 削除したテキストを `before` から取り出して挿入に変える。文書全体の削除は `before` の rope を共有する
    /// (`InsertText::Rope`。旧全文を複写しない、03 §9.5)。それ以外は複写する (64 KiB 以上の `Slice` は v0.4)。
    ///
    /// 事前条件: `before.len_bytes() == self.len_before()` で、操作の境界が char 境界。
    /// 満たさない場合もパニックはしないが、結果は意味を持たない。
    pub fn invert(&self, before: &Snapshot) -> ChangeSet {
        let mut ops = Vec::with_capacity(self.ops.len());
        let mut pos = 0usize;
        for op in &self.ops {
            match op {
                Op::Retain(n) => {
                    ops.push(Op::Retain(*n));
                    pos = pos.saturating_add(*n);
                }
                Op::Delete(n) if pos == 0 && *n == before.len_bytes() => {
                    ops.push(Op::Insert(InsertText::Rope(before.clone())));
                    pos = pos.saturating_add(*n);
                }
                Op::Delete(n) => {
                    let text = before.slice_to_string(ByteRange::new(
                        ByteOff(pos),
                        ByteOff(pos.saturating_add(*n)),
                    ));
                    ops.push(Op::Insert(InsertText::Str(Arc::from(text))));
                    pos = pos.saturating_add(*n);
                }
                Op::Insert(s) => ops.push(Op::Delete(s.len())),
            }
        }
        ChangeSet::from_ops(ops)
    }

    /// 位置の写像 (03 §6.4)。`p` が挿入位置に一致するとき、`Before` は挿入の前、`After` は後ろ。
    /// 削除範囲の内側・端は削除の開始位置へ潰れる (置換では、`After` は挿入の後ろ)。
    /// 文書の末尾より後ろの `p` は、新しい文書の末尾に丸める。計算量は操作の数に比例する。
    ///
    /// 注意: 合成した ChangeSet (`compose`) の `map_pos` は、元の ChangeSet を順に写像した結果と一致するとは限らない
    /// (合成で「残す部分」の構造が失われるため)。Undo の結合などで合成した ChangeSet で Selection を写像せず、
    /// 元の ChangeSet を順に写像すること。
    pub fn map_pos(&self, p: ByteOff, bias: Bias) -> ByteOff {
        let p = p.0;
        let (mut old, mut new) = (0usize, 0usize);
        let mut it = self.ops.iter().peekable();
        while let Some(op) = it.next() {
            match op {
                Op::Retain(k) => {
                    if p < old.saturating_add(*k) {
                        return ByteOff(new + (p - old));
                    }
                    old = old.saturating_add(*k);
                    new = new.saturating_add(*k);
                }
                Op::Delete(k) => {
                    if p <= old.saturating_add(*k) {
                        // 削除範囲の内側・端。直後に挿入があれば (置換)、After はその後ろ
                        return match (bias, it.peek()) {
                            (Bias::After, Some(Op::Insert(s))) => ByteOff(new + s.len()),
                            _ => ByteOff(new),
                        };
                    }
                    old = old.saturating_add(*k);
                }
                Op::Insert(s) => {
                    if p == old {
                        return match bias {
                            Bias::Before => ByteOff(new),
                            Bias::After => ByteOff(new + s.len()),
                        };
                    }
                    new = new.saturating_add(s.len());
                }
            }
        }
        ByteOff(new)
    }

    /// 範囲の写像 (03 §6.4)。範囲の端の写像は `start` を `After`、`end` を `Before` で行う。
    pub fn map_range(&self, r: ByteRange) -> RangeMap {
        let (s, e) = (r.start.0, r.end.0);
        let mut straddle = false;
        let mut inside = false;
        let mut touch = false;
        for (before, after) in self.changed_ranges() {
            let (a, b) = (before.start.0, before.end.0);
            let has_insert = !after.is_empty();
            if b > a {
                // 空の範囲 (位置) が削除・置換の内側にあるときも、位置が消えるので交差として扱う
                // (保守的に失効させる。端に接するだけなら、下の touch / Mapped)。
                let intersects = b.min(e) > a.max(s) || (s == e && a < s && s < b);
                if intersects {
                    if a >= s && b <= e {
                        inside = true;
                    } else {
                        straddle = true;
                    }
                } else if has_insert && (a == e || b == s) {
                    touch = true;
                }
            } else if a == s || a == e {
                touch = true;
            } else if s < a && a < e {
                inside = true;
            }
        }
        let mapped = {
            let start = self.map_pos(r.start, Bias::After);
            let end = self.map_pos(r.end, Bias::Before);
            ByteRange::new(start, end)
        };
        if straddle {
            RangeMap::Overlapped {
                mapped: None,
                kind: OverlapKind::Straddle,
            }
        } else if inside {
            RangeMap::Overlapped {
                mapped: Some(mapped),
                kind: OverlapKind::Inside,
            }
        } else if touch {
            RangeMap::Overlapped {
                mapped: Some(mapped),
                kind: OverlapKind::Touch,
            }
        } else {
            RangeMap::Mapped(mapped)
        }
    }

    /// 変更のまとまりごとの (適用前の範囲, 適用後の範囲)。購読者 (syntax など) 向け。
    /// 連続する Delete / Insert を 1 つのまとまりとして返す。
    pub fn changed_ranges(&self) -> std::vec::IntoIter<(ByteRange, ByteRange)> {
        let mut out = Vec::new();
        let (mut old, mut new) = (0usize, 0usize);
        let mut open: Option<(usize, usize)> = None; // (old 開始, new 開始)
        for op in &self.ops {
            match op {
                Op::Retain(k) => {
                    if let Some((o, n)) = open.take() {
                        out.push((
                            ByteRange::new(ByteOff(o), ByteOff(old)),
                            ByteRange::new(ByteOff(n), ByteOff(new)),
                        ));
                    }
                    old = old.saturating_add(*k);
                    new = new.saturating_add(*k);
                }
                Op::Delete(k) => {
                    open.get_or_insert((old, new));
                    old = old.saturating_add(*k);
                }
                Op::Insert(s) => {
                    open.get_or_insert((old, new));
                    new = new.saturating_add(s.len());
                }
            }
        }
        if let Some((o, n)) = open {
            out.push((
                ByteRange::new(ByteOff(o), ByteOff(old)),
                ByteRange::new(ByteOff(n), ByteOff(new)),
            ));
        }
        out.into_iter()
    }
}

impl Text {
    /// `ChangeSet` を適用する。`len_before` が文書と違う、または操作の境界が char 境界でないときは
    /// エラーを返し、**何も変更しない** (検査は書き換えの前にまとめて行う。INV-T1)。
    pub fn apply(&mut self, cs: &ChangeSet) -> Result<(), ChangeSetError> {
        if cs.len_before != self.len_bytes() {
            return Err(ChangeSetError::LenMismatch {
                expected: cs.len_before,
                actual: self.len_bytes(),
            });
        }
        let mut pos = 0usize;
        for op in &cs.ops {
            if let Op::Retain(n) | Op::Delete(n) = op {
                pos += n;
                if !self.is_char_boundary(ByteOff(pos)) {
                    return Err(ChangeSetError::NotCharBoundary { pos: ByteOff(pos) });
                }
            }
        }
        let rope = self.rope_mut();
        let mut pos = 0usize;
        for op in &cs.ops {
            match op {
                Op::Retain(n) => pos += n,
                // 全体の削除は空にするだけ (共有している節を複製しない)
                Op::Delete(n) if pos == 0 && *n == rope.len_bytes() => *rope = Rope::new(),
                Op::Delete(n) => {
                    let (a, b) = (rope.byte_to_char(pos), rope.byte_to_char(pos + n));
                    rope.remove(a..b);
                }
                Op::Insert(InsertText::Str(s)) => {
                    rope.insert(rope.byte_to_char(pos), s);
                    pos += s.len();
                }
                // rope を共有してつなぐ (複写しない)。空の文書 (全体の置き換え) は差し替えるだけ
                Op::Insert(InsertText::Rope(r)) => {
                    if rope.len_bytes() == 0 {
                        *rope = r.rope().clone();
                    } else {
                        let tail = rope.split_off(rope.byte_to_char(pos));
                        rope.append(r.rope().clone());
                        rope.append(tail);
                    }
                    pos += r.len_bytes();
                }
            }
        }
        debug_assert_eq!(rope.len_bytes(), cs.len_after);
        Ok(())
    }
}
