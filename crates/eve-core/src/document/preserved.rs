//! 保持方式の記録の表 `PreservedMap` (docs/03-core-design.md §8.7.1、v0.4 の M22 の 4)。
//!
//! 読込の検査 (eve-encoding の `RoundtripChecker`) が集めた記録 (`PreservedRecords`。位置は行番号と行の中の位置) を、
//! 文書の位置 (本文の UTF-8 のバイト位置) に直して持つ。編集・Undo / Redo・範囲の入れ替えで位置を写し、変更に掛かる
//! 記録を外す。保存は、本文を前から読みながら `iter_from` で記録を引く。
//!
//! - **形**: 背骨 (`Arc<Vec<Slot>>`) に、塊 (`Arc<Chunk>`) と塊の基準位置 (文書の絶対位置。塊の最初の記録の始まり) を
//!   並べる。塊の中は、基準位置からの相対の位置・長さと、元のバイト列 (塊ごとにつないだもの) を持つ。索引は 1 件 16 バイト。
//!   1 塊は `CHUNK_MAX` 件・元のバイト列 `CHUNK_MAX_BYTES` までを目安にする (1 件で超える記録は、その 1 件だけの塊)。
//! - **費用** (03 §8.7.1、NFR-105): 編集 1 回の更新は、変更の数と、変更に触れた塊の件数 (1 塊は `CHUNK_MAX` 件まで) と、
//!   変更より後ろの塊の数 (基準位置に差分を足すだけ) に比例し、記録の総数に比例しない。触れない塊は `Arc` を共有したまま。
//! - **複製**: `Clone` は背骨の `Arc` を 1 つ増やすだけ。複製した版は、その後の編集の影響を受けない (編集は背骨を
//!   `Arc::make_mut` で写してから書き換え、触れた塊は作り直す。塊そのものは書き換えない)。
//! - **写像の規則** (03 §8.7.1 の「編集での写像」): `ChangeSet::map_range` (03 §6.4) の分類と同じ。`Mapped` は位置を写して
//!   残す (端に接する削除を含む)。`Inside` / `Straddle` は外す。`Touch` (端にちょうど接する挿入) は、文字の範囲の記録では
//!   残し (範囲は挿入した文字を含まない)、行の記録では外す。加えて、**長さ 0 の記録** (ISO-2022-JP の、冗長なエスケープ
//!   だけの行) は、位置に接する削除でも外す。`map_range` では `Mapped` だが、純粋な削除の始まりと終わりの点は削除の後に
//!   同じ位置になり、Undo (削除の逆の挿入) で元の位置を一意に決められないため (行の終端を消して行が変わったときでもある)。
//! - **Undo / Redo** (03 §8.7.1): 編集は外した記録を (編集の前の座標で) 返す。Undo の適用 (`apply_undo`) は記録を外さずに
//!   位置だけを写し、ノードが持つ記録を戻す。Redo の適用 (`apply_redo`) は同じ記録を外してから位置を写す。逆変換が変更の
//!   間の変わっていない範囲も一緒に置き換えるノード (03 §12.5 の粗い逆変換、文書全体の置き換え) は、`affected_hull` と
//!   `spans_in` で編集の前の記録を取り出しておき、Undo で `replace_range` で入れ替える。
//!
//! 5 本目 (`Document` への組み込み) までは製品から呼ばない。

use std::fmt;
use std::sync::Arc;

use eve_encoding::{Decoder, Encoding, PreserveReason, PreserveState, PreservedRecords};
use eve_text::{ByteOff, ByteRange, ChangeSet, LineIdx, TextRead};

/// 1 つの塊の件数の上限。
pub(crate) const CHUNK_MAX: usize = 1024;
/// 1 つの塊の元のバイト列の合計の上限の目安 (作り直すときの複写を抑える)。
pub(crate) const CHUNK_MAX_BYTES: usize = 64 * 1024;

/// 塊の大きさ (試験では小さくして、塊の境をまたぐ編集を通す)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ChunkLimits {
    /// 1 つの塊の件数の上限 (1 以上)。
    pub(crate) max: usize,
    /// 1 つの塊の元のバイト列の合計の上限の目安 (1 件で超える記録は、その 1 件だけの塊にする)。
    pub(crate) max_bytes: usize,
}

impl Default for ChunkLimits {
    fn default() -> ChunkLimits {
        ChunkLimits {
            max: CHUNK_MAX,
            max_bytes: CHUNK_MAX_BYTES,
        }
    }
}

impl ChunkLimits {
    fn max(self) -> usize {
        self.max.max(1)
    }

    /// 作り直した塊がこれより小さければ、隣の塊とまとめる (小さな塊が増えて背骨が長くなるのを防ぐ)。
    fn min(self) -> usize {
        (self.max() / 4).max(1)
    }
}

/// 1 件の記録 (編集で外した記録・Undo のノードが持つ記録・範囲の入れ替えで渡す記録)。位置は文書の UTF-8 のバイト位置。
/// `Debug` は元のバイト列の中身を出さず、バイト数だけを出す (本文由来のバイト列を log やパニックに出さない)。
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct PreservedSpan {
    /// 本文の範囲 (行の終端を含まない)。長さ 0 のことがある (ISO-2022-JP の、冗長なエスケープだけの行)。
    pub(crate) range: ByteRange,
    /// 元のバイト列 (空でない)。初期状態から復号すると範囲の文字列になり、初期状態で終わる (03 §8.7.1 の不変条件)。
    pub(crate) bytes: Box<[u8]>,
    pub(crate) reason: PreserveReason,
    /// 行の記録 (ISO-2022-JP。範囲は行の内容の全体) か。偽なら文字の範囲の記録 (`Shift_JIS` / EUC-JP)。
    /// 行の記録は、端に接する挿入でも外す (03 §8.7.1 の「編集での写像」)。
    pub(crate) line: bool,
}

impl fmt::Debug for PreservedSpan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreservedSpan")
            .field("range", &self.range)
            .field("bytes", &self.bytes.len())
            .field("reason", &self.reason)
            .field("line", &self.line)
            .finish()
    }
}

impl PreservedSpan {
    fn as_rec(&self) -> Rec<'_> {
        Rec {
            start: self.range.start.0,
            len: self.range.len(),
            bytes: &self.bytes,
            flags: pack(self.reason, self.line),
        }
    }

    fn key(&self) -> (usize, usize) {
        (self.range.start.0, self.range.end.0)
    }
}

/// 記録の列の元のバイト列の合計 (Undo の予算に数える、03 §7.5)。
pub(crate) fn spans_bytes(spans: &[PreservedSpan]) -> usize {
    spans.iter().map(|s| s.bytes.len()).sum()
}

/// 表の 1 件を借りて見る形 (`PreservedMap::iter` など)。`Debug` は元のバイト列の中身を出さない。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct SpanRef<'a> {
    pub(crate) range: ByteRange,
    pub(crate) bytes: &'a [u8],
    pub(crate) reason: PreserveReason,
    pub(crate) line: bool,
}

impl fmt::Debug for SpanRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpanRef")
            .field("range", &self.range)
            .field("bytes", &self.bytes.len())
            .field("reason", &self.reason)
            .field("line", &self.line)
            .finish()
    }
}

impl SpanRef<'_> {
    pub(crate) fn to_span(self) -> PreservedSpan {
        PreservedSpan {
            range: self.range,
            bytes: self.bytes.into(),
            reason: self.reason,
            line: self.line,
        }
    }
}

/// 表を作れなかった・表が不変条件を満たさない理由。本文を含めない (ログに残してよい)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PreservedError {
    /// 記録の番号 (作るときは `PreservedRecords.records` の番号、検査では表の昇順の番号)。
    pub(crate) index: usize,
    pub(crate) kind: PreservedErrorKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PreservedErrorKind {
    /// 行番号・位置が本文の外、または元のバイト列が `arena` の外。
    OutOfRange,
    /// 範囲の端が char 境界でない。
    NotCharBoundary,
    /// 昇順でない、または前の記録と重なる。
    NotSorted,
    /// 行の記録が行の内容の全体でない、または行の記録かどうかが文字コードと合わない。
    WrongUnit,
    /// 元のバイト列を初期状態から復号しても範囲の文字列にならない、または初期状態で終わらない (03 §8.7.1 の不変条件)。
    Mismatch,
    /// 表の内部の形の食い違い (塊が空・相対の位置の誤り・件数とバイト数の合計の誤り)。
    Structure,
}

impl fmt::Display for PreservedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "保持方式の記録 {} 件目: {:?}", self.index, self.kind)
    }
}

impl std::error::Error for PreservedError {}

// ---- 内部の形 ----

/// `Entry::flags` のビット: 下位 2 ビットが理由、その上が行の記録。
const FLAG_LINE: u32 = 1 << 2;

fn pack(reason: PreserveReason, line: bool) -> u32 {
    let r = match reason {
        PreserveReason::DuplicateEncoding => 0,
        PreserveReason::DecodeOnly => 1,
        PreserveReason::StatefulEscape => 2,
    };
    if line { r | FLAG_LINE } else { r }
}

fn reason_of(flags: u32) -> PreserveReason {
    match flags & 3 {
        0 => PreserveReason::DuplicateEncoding,
        1 => PreserveReason::DecodeOnly,
        _ => PreserveReason::StatefulEscape,
    }
}

/// 塊の中の 1 件 (16 バイト)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Entry {
    /// 塊の基準位置からの相対の始まり (最初の記録は 0)。
    start: u32,
    /// 範囲の長さ。
    len: u32,
    /// 塊の `bytes` の中の、この記録の元のバイト列の終わり (始まりは前の記録の終わり、最初の記録は 0)。
    bytes_end: u32,
    flags: u32,
}

/// 塊。作ったら書き換えない (複製した版と共有する)。`Debug` は件数とバイト数だけを出す。
struct Chunk {
    /// 空でない。
    entries: Box<[Entry]>,
    bytes: Box<[u8]>,
}

impl fmt::Debug for Chunk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Chunk")
            .field("entries", &self.entries.len())
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

/// 背骨の 1 項目: 塊と、その基準位置 (= 塊の最初の記録の始まり)。
#[derive(Clone, Debug)]
struct Slot {
    base: usize,
    chunk: Arc<Chunk>,
}

impl Slot {
    fn len(&self) -> usize {
        self.chunk.entries.len()
    }

    /// 最初の記録の始まり。
    fn start(&self) -> usize {
        self.base + self.chunk.entries[0].start as usize
    }

    /// 最後の記録の終わり (記録は重ならないので、塊の中で最も後ろ)。
    fn end(&self) -> usize {
        let e = self.chunk.entries[self.len() - 1];
        self.base + e.start as usize + e.len as usize
    }

    fn rec(&self, i: usize) -> Rec<'_> {
        let e = self.chunk.entries[i];
        let b0 = if i == 0 {
            0
        } else {
            self.chunk.entries[i - 1].bytes_end as usize
        };
        Rec {
            start: self.base + e.start as usize,
            len: e.len as usize,
            bytes: &self.chunk.bytes[b0..e.bytes_end as usize],
            flags: e.flags,
        }
    }

    /// 基準位置を `base` に移したときの `i` 件目。
    fn rec_at(&self, i: usize, base: usize) -> Rec<'_> {
        let r = self.rec(i);
        r.moved(base + (r.start - self.base))
    }
}

/// 処理の途中の 1 件 (元のバイト列は、塊か `PreservedSpan` から借りる)。
#[derive(Clone, Copy, Debug)]
struct Rec<'a> {
    start: usize,
    len: usize,
    bytes: &'a [u8],
    flags: u32,
}

impl<'a> Rec<'a> {
    fn end(&self) -> usize {
        self.start + self.len
    }

    fn line(&self) -> bool {
        self.flags & FLAG_LINE != 0
    }

    fn moved(self, start: usize) -> Rec<'a> {
        Rec { start, ..self }
    }

    fn key(&self) -> (usize, usize) {
        (self.start, self.end())
    }

    fn view(&self) -> SpanRef<'a> {
        SpanRef {
            range: ByteRange::new(ByteOff(self.start), ByteOff(self.end())),
            bytes: self.bytes,
            reason: reason_of(self.flags),
            line: self.line(),
        }
    }

    fn to_span(self) -> PreservedSpan {
        self.view().to_span()
    }

    fn same(&self, s: &PreservedSpan) -> bool {
        self.key() == s.key() && self.bytes == &*s.bytes && self.flags == pack(s.reason, s.line)
    }
}

/// 塊を作る。`n` 件を、`max` 件を超えない範囲でなるべく同じ大きさに分ける。
struct Builder {
    limits: ChunkLimits,
    target: usize,
    out: Vec<Slot>,
    entries: Vec<Entry>,
    bytes: Vec<u8>,
    base: usize,
}

impl Builder {
    fn new(limits: ChunkLimits, n: usize) -> Builder {
        let max = limits.max();
        let target = if n <= max {
            max
        } else {
            n.div_ceil(n.div_ceil(max))
        };
        Builder {
            limits,
            target,
            out: Vec::new(),
            entries: Vec::with_capacity(target.min(n)),
            bytes: Vec::new(),
            base: 0,
        }
    }

    /// 昇順に 1 件ずつ足す。
    fn push(&mut self, r: Rec<'_>) {
        // 相対の位置は u32 に収める (収まらなければ塊を分ける)
        let rel = if self.entries.is_empty() {
            None
        } else {
            r.start
                .checked_sub(self.base)
                .and_then(|d| u32::try_from(d).ok())
        };
        let full = self.entries.len() >= self.target
            || self.bytes.len().saturating_add(r.bytes.len()) > self.limits.max_bytes;
        let rel = match rel {
            Some(rel) if !full => rel,
            _ => {
                self.flush();
                self.base = r.start;
                0
            }
        };
        self.bytes.extend_from_slice(r.bytes);
        // 元のバイト列の合計は 16 MiB (03 §8.7.1 の上限) に収まり、範囲の長さも u32 に収まる (eve-encoding の記録の形)
        let bytes_end = u32::try_from(self.bytes.len()).unwrap_or(u32::MAX);
        let len = u32::try_from(r.len).unwrap_or(u32::MAX);
        debug_assert!(u32::try_from(self.bytes.len()).is_ok() && u32::try_from(r.len).is_ok());
        self.entries.push(Entry {
            start: rel,
            len,
            bytes_end,
            flags: r.flags,
        });
    }

    fn flush(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let cap = self.target;
        let entries = std::mem::replace(&mut self.entries, Vec::with_capacity(cap));
        let bytes = std::mem::take(&mut self.bytes);
        self.out.push(Slot {
            base: self.base,
            chunk: Arc::new(Chunk {
                entries: entries.into_boxed_slice(),
                bytes: bytes.into_boxed_slice(),
            }),
        });
    }

    fn finish(mut self) -> Vec<Slot> {
        self.flush();
        self.out
    }
}

/// 背骨の後ろの部分を作り直す道具。触れない塊は共有し (`reuse`)、触れた塊の記録は `run` にためて作り直す。
/// 作り直した塊が小さければ、前後の塊とまとめる。
struct Walk<'a> {
    limits: ChunkLimits,
    out: Vec<Slot>,
    run: Vec<Rec<'a>>,
    /// 直前の共有する塊 (まだ `out` に入れていない。後ろの小さな `run` とまとめられるように持っておく) と、新しい基準位置。
    pending: Option<(&'a Slot, usize)>,
}

impl<'a> Walk<'a> {
    fn new(limits: ChunkLimits, cap: usize) -> Walk<'a> {
        Walk {
            limits,
            out: Vec::with_capacity(cap + 1),
            run: Vec::new(),
            pending: None,
        }
    }

    /// 触れない塊: 基準位置を `base` に移して共有する。
    fn reuse(&mut self, slot: &'a Slot, base: usize) {
        if !self.run.is_empty() && self.run.len() < self.limits.min() {
            // 作り直す記録が少ないので、この塊も一緒に作り直す (小さな塊を増やさない)
            for i in 0..slot.len() {
                self.run.push(slot.rec_at(i, base));
            }
            return;
        }
        self.flush_run();
        self.flush_pending();
        self.pending = Some((slot, base));
    }

    /// 作り直す記録を 1 件足す (昇順)。
    fn push(&mut self, r: Rec<'a>) {
        self.run.push(r);
    }

    fn flush_pending(&mut self) {
        if let Some((slot, base)) = self.pending.take() {
            self.out.push(Slot {
                base,
                chunk: Arc::clone(&slot.chunk),
            });
        }
    }

    fn flush_run(&mut self) {
        if self.run.is_empty() {
            return;
        }
        let mut run = std::mem::take(&mut self.run);
        if run.len() < self.limits.min()
            && let Some((slot, base)) = self.pending
            && slot.len() + run.len() <= self.limits.max()
        {
            // 前の塊とまとめる
            let mut merged = Vec::with_capacity(slot.len() + run.len());
            merged.extend((0..slot.len()).map(|i| slot.rec_at(i, base)));
            merged.append(&mut run);
            run = merged;
            self.pending = None;
        }
        self.flush_pending();
        let mut b = Builder::new(self.limits, run.len());
        for r in run {
            b.push(r);
        }
        self.out.extend(b.finish());
    }

    fn finish(mut self) -> Vec<Slot> {
        self.flush_run();
        self.flush_pending();
        self.out
    }
}

/// 変更のまとまり 1 つ (`ChangeSet::changed_ranges` の 1 項目)。旧の範囲 `[a, b)`、新の終わり `nb`、挿入があるか。
#[derive(Clone, Copy, Debug)]
struct Hunk {
    a: usize,
    b: usize,
    nb: usize,
    insert: bool,
}

fn hunks_of(cs: &ChangeSet) -> Vec<Hunk> {
    cs.changed_ranges()
        .map(|(old, new)| Hunk {
            a: old.start.0,
            b: old.end.0,
            nb: new.end.0,
            insert: !new.is_empty(),
        })
        .collect()
}

/// 記録と変更の関係 (`eve_text::OverlapKind` と同じ分類。強いものが後ろ)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    None,
    Touch,
    Inside,
    Straddle,
}

impl Hunk {
    /// 記録 `[s, e)` との関係。`ChangeSet::map_range` の判定と同じ式。
    fn classify(&self, s: usize, e: usize) -> Class {
        let (a, b) = (self.a, self.b);
        if b > a {
            let intersects = b.min(e) > a.max(s) || (s == e && a < s && s < b);
            if intersects {
                if a >= s && b <= e {
                    Class::Inside
                } else {
                    Class::Straddle
                }
            } else if self.insert && (a == e || b == s) {
                Class::Touch
            } else {
                Class::None
            }
        } else if a == s || a == e {
            Class::Touch
        } else if s < a && a < e {
            Class::Inside
        } else {
            Class::None
        }
    }
}

/// 位置 `x` の写像。`before` は、旧の終わりが `x` 以下の変更の全部 (最後のものだけを見る)。挿入の位置に一致する点は
/// 挿入の後ろへ写す (記録の始まりの `Bias::After` と同じ)。
fn map_point(before: &[Hunk], x: usize) -> usize {
    match before.last() {
        Some(h) => h.nb + (x - h.b),
        None => x,
    }
}

/// 変更の写像の規則 (`map_changes` の `strict`)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rule {
    /// 編集: `Inside` / `Straddle`、行の記録の `Touch`、変更に接する長さ 0 の記録を外す。
    Edit,
    /// Undo / Redo: 位置だけを写す。本文が編集の前・後と同じになるので、外す記録は無いはず。`Inside` / `Straddle` に
    /// なった記録 (呼び出しの誤り) は、範囲の文字列が変わって不変条件を満たさないので、外して返す。
    Keep,
}

/// 文書が持つ、保持方式の記録の表 (03 §8.7.1)。範囲の昇順で、重ならない (長さ 0 の記録は、前の記録の終わりと同じ位置に
/// あってよい)。
#[derive(Clone, Default)]
pub(crate) struct PreservedMap {
    slots: Arc<Vec<Slot>>,
    count: usize,
    bytes: usize,
    limits: ChunkLimits,
}

impl fmt::Debug for PreservedMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreservedMap")
            .field("len", &self.count)
            .field("bytes", &self.bytes)
            .field("chunks", &self.slots.len())
            .finish()
    }
}

/// 内容 (位置・元のバイト列・理由・行の記録か) が同じなら等しい (塊の分け方は問わない)。
impl PartialEq for PreservedMap {
    fn eq(&self, other: &PreservedMap) -> bool {
        self.count == other.count && self.bytes == other.bytes && self.iter().eq(other.iter())
    }
}

impl Eq for PreservedMap {}

impl PreservedMap {
    /// 空の表。
    pub(crate) fn new() -> PreservedMap {
        PreservedMap::default()
    }

    /// 読込の検査が集めた記録から作る (03 §8.7.1 の「集める側と持つ側」「読込の完了時の照合」)。`text` は記録を集めたのと
    /// 同じバイト列を復号した本文、`enc` はその文字コード。記録の (行番号, 行の中の位置, 長さ) を、本文の行
    /// (区切りは LF・CR・CRLF) で文書の位置に直し、**全部の記録について** 範囲の文字列が元のバイト列を初期状態から
    /// 復号したものと一致し、初期状態で終わることを照合する。`records.state` が `Collected` でなければ空の表。
    ///
    /// # Errors
    ///
    /// 位置が本文の外・char 境界でない・昇順でない、ISO-2022-JP の記録が行の内容の全体でない、または照合に合わない記録が
    /// 1 件でもあれば、その記録の番号と理由を返す (呼び出し側は文書を「保持なし」にする)。
    pub(crate) fn from_records(
        records: &PreservedRecords,
        enc: Encoding,
        text: &dyn TextRead,
    ) -> Result<PreservedMap, PreservedError> {
        if records.state != PreserveState::Collected || records.records.is_empty() {
            return Ok(PreservedMap::new());
        }
        let limits = ChunkLimits::default();
        let line_unit = enc.is_stateful();
        let lines = text.len_lines();
        let mut b = Builder::new(limits, records.records.len());
        let mut buf = Scratch::default();
        let mut prev_end = 0usize;
        let mut total = 0usize;
        // 直前の記録の行 (行番号, 行頭, 行の内容の末尾)。同じ行の記録が続くとき、行の位置を引き直さない
        let mut cur: Option<(u64, usize, usize)> = None;
        for (index, r) in records.records.iter().enumerate() {
            let err = |kind| PreservedError { index, kind };
            let (line_start, line_end) = match cur {
                Some((l, s, e)) if l == r.line => (s, e),
                _ => {
                    let line = usize::try_from(r.line)
                        .ok()
                        .filter(|&l| l < lines)
                        .ok_or(err(PreservedErrorKind::OutOfRange))?;
                    let s = text.line_to_byte(LineIdx(line)).0;
                    let e = text.line_end_byte(LineIdx(line)).0;
                    cur = Some((r.line, s, e));
                    (s, e)
                }
            };
            let start = usize::try_from(r.col)
                .ok()
                .and_then(|c| line_start.checked_add(c))
                .ok_or(err(PreservedErrorKind::OutOfRange))?;
            let end = start
                .checked_add(r.len as usize)
                .filter(|&e| e <= line_end)
                .ok_or(err(PreservedErrorKind::OutOfRange))?;
            if line_unit && (start != line_start || end != line_end) {
                return Err(err(PreservedErrorKind::WrongUnit));
            }
            if start < prev_end {
                return Err(err(PreservedErrorKind::NotSorted));
            }
            if !text.is_char_boundary(ByteOff(start)) || !text.is_char_boundary(ByteOff(end)) {
                return Err(err(PreservedErrorKind::NotCharBoundary));
            }
            let bytes = records
                .arena
                .get(r.bytes.start as usize..r.bytes.end as usize)
                .filter(|b| !b.is_empty())
                .ok_or(err(PreservedErrorKind::OutOfRange))?;
            let range = ByteRange::new(ByteOff(start), ByteOff(end));
            if !matches_original(enc, bytes, text, range, &mut buf) {
                return Err(err(PreservedErrorKind::Mismatch));
            }
            prev_end = end;
            total += bytes.len();
            b.push(Rec {
                start,
                len: end - start,
                bytes,
                flags: pack(r.reason, line_unit),
            });
        }
        Ok(PreservedMap {
            slots: Arc::new(b.finish()),
            count: records.records.len(),
            bytes: total,
            limits,
        })
    }

    /// 件数。
    pub(crate) fn len(&self) -> usize {
        self.count
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// 元のバイト列の合計のバイト数。
    pub(crate) fn total_bytes(&self) -> usize {
        self.bytes
    }

    /// 範囲の昇順の走査。
    pub(crate) fn iter(&self) -> Iter<'_> {
        Iter {
            slots: &self.slots,
            si: 0,
            ei: 0,
        }
    }

    /// 始まりが `pos` 以上の記録を昇順に走査する (保存の符号化が、本文を前から読みながら次の記録を引く)。費用は
    /// 塊の数と塊の件数の対数。
    pub(crate) fn iter_from(&self, pos: ByteOff) -> Iter<'_> {
        let si = self.slots.partition_point(|s| s.end() < pos.0);
        // 塊 si より前の塊の記録は、どれも終わりが pos より前なので始まりも pos より前
        let ei = self.slots.get(si).map_or(0, |s| {
            let rel = pos.0.saturating_sub(s.base);
            s.chunk
                .entries
                .partition_point(|e| (e.start as usize) < rel)
        });
        Iter {
            slots: &self.slots,
            si,
            ei,
        }
        .normalized()
    }

    /// 終わりが `pos` 以上の記録から昇順に走査する (記録は重ならないので、終わりも昇順)。
    fn iter_from_end(&self, pos: usize) -> Iter<'_> {
        let si = self.slots.partition_point(|s| s.end() < pos);
        let ei = self.slots.get(si).map_or(0, |s| {
            let rel = pos.saturating_sub(s.base);
            s.chunk
                .entries
                .partition_point(|e| (e.start as usize + e.len as usize) < rel)
        });
        Iter {
            slots: &self.slots,
            si,
            ei,
        }
        .normalized()
    }

    /// 編集 (`Transaction` の適用、03 §6.5 手順 4) の写像。規則はモジュールの説明のとおり (`map_range` の分類と同じ。
    /// 長さ 0 の記録は、位置に接する削除でも外す)。外した記録を、**編集の前の座標のまま** 昇順で返す (その Transaction の
    /// Undo のノードが持つ、03 §7.1 の `preserved_removed`)。
    pub(crate) fn apply_edit(&mut self, cs: &ChangeSet) -> Vec<PreservedSpan> {
        self.map_changes(cs, Rule::Edit)
    }

    /// Undo の適用 (`cs` は逆変換)。記録を外さずに位置だけを写し、そのノードが持つ記録 `restore` (`apply_edit` が返した
    /// もの。編集の前の座標 = Undo の後の座標、昇順) を戻す。戻り値は、写像で外れた記録 (履歴と表が食い違ったときだけ。
    /// 正しく使えば空)。
    pub(crate) fn apply_undo(
        &mut self,
        cs: &ChangeSet,
        restore: &[PreservedSpan],
    ) -> Vec<PreservedSpan> {
        let dropped = self.map_changes(cs, Rule::Keep);
        self.insert_spans(restore);
        dropped
    }

    /// Redo の適用 (`cs` は元の変更)。ノードが持つ記録 `remove` (編集の前の座標、昇順) を外してから、位置だけを写す。
    /// 戻り値は、外すはずの記録が表に無かったものと、写像で外れた記録 (どちらも、正しく使えば空)。
    pub(crate) fn apply_redo(
        &mut self,
        cs: &ChangeSet,
        remove: &[PreservedSpan],
    ) -> Vec<PreservedSpan> {
        let mut odd = self.remove_spans(remove);
        odd.extend(self.map_changes(cs, Rule::Keep));
        odd
    }

    /// `range` (閉区間 `[start, end]`) に触れる記録と `range` を合わせた範囲。粗い逆変換 (03 §12.5) のノードを作るとき、
    /// 変更の最初の始まりから最後の終わりまでの範囲 (旧の座標) を渡し、返った範囲の記録を `spans_in` で取り出しておく
    /// (変更の境をまたいで外れる記録も入るように)。
    pub(crate) fn affected_hull(&self, range: ByteRange) -> ByteRange {
        let (mut lo, mut hi) = (range.start.0, range.end.0);
        for r in self.iter_from_end(range.start.0) {
            if r.range.start.0 > range.end.0 {
                break;
            }
            lo = lo.min(r.range.start.0);
            hi = hi.max(r.range.end.0);
        }
        ByteRange::new(ByteOff(lo), ByteOff(hi))
    }

    /// `range` に収まる記録 (`range.start <= 始まり`、`終わり <= range.end`。端にある長さ 0 の記録を含む) の複製。
    pub(crate) fn spans_in(&self, range: ByteRange) -> Vec<PreservedSpan> {
        self.iter_from(range.start)
            .take_while(|r| r.range.start <= range.end)
            .filter(|r| r.range.end <= range.end)
            .map(|r| r.to_span())
            .collect()
    }

    /// 範囲の入れ替え (粗い逆変換・文書全体の置き換えの Undo / Redo)。本文の `range` が長さ `new_len` の文字列に
    /// 置き換わったものとして、`range` に収まる記録 (`spans_in` と同じ条件) を外して返し、`spans` (置き換えた後の座標、
    /// 昇順、`[range.start, range.start + new_len]` の中) を入れ、`range` より後ろの記録を差分だけずらす。
    pub(crate) fn replace_range(
        &mut self,
        range: ByteRange,
        new_len: usize,
        spans: &[PreservedSpan],
    ) -> Vec<PreservedSpan> {
        let (rs, re) = (range.start.0, range.end.0);
        let after = |x: usize| (x - re) + rs + new_len;
        let mut removed = Vec::new();
        let limits = self.limits;
        let slots = Arc::make_mut(&mut self.slots);
        let i0 = slots.partition_point(|s| s.end() < rs);
        let tail: Vec<Slot> = slots.drain(i0.saturating_sub(1)..).collect();
        let mut walk = Walk::new(limits, tail.len());
        // 入れる記録は、範囲より後ろの最初の記録の前 (無ければ最後) に置く
        let mut emitted = false;
        for slot in &tail {
            if slot.end() < rs {
                walk.reuse(slot, slot.base);
            } else if slot.start() > re {
                emit_once(&mut walk, spans, &mut emitted);
                walk.reuse(slot, after(slot.base));
            } else {
                for i in 0..slot.len() {
                    let r = slot.rec(i);
                    let (s, e) = r.key();
                    if s >= rs && e <= re {
                        removed.push(r.to_span());
                    } else if s >= re {
                        emit_once(&mut walk, spans, &mut emitted);
                        walk.push(r.moved(after(s)));
                    } else if e <= rs {
                        walk.push(r);
                    } else {
                        // 範囲の境をまたぐ記録 (呼び出しの誤り。範囲は `affected_hull` で広げてあるはず): 外す
                        debug_assert!(false, "範囲の境をまたぐ記録");
                        removed.push(r.to_span());
                    }
                }
            }
        }
        emit_once(&mut walk, spans, &mut emitted);
        slots.extend(walk.finish());
        self.count = self.count - removed.len() + spans.len();
        self.bytes = self.bytes - spans_bytes(&removed) + spans_bytes(spans);
        removed
    }

    /// 写像の本体。`Rule::Edit` は編集、`Rule::Keep` は Undo / Redo (位置だけを写す)。外した記録を旧の座標で返す。
    fn map_changes(&mut self, cs: &ChangeSet, rule: Rule) -> Vec<PreservedSpan> {
        let mut removed = Vec::new();
        if self.count == 0 {
            return removed;
        }
        let hunks = hunks_of(cs);
        let Some(first) = hunks.first() else {
            return removed;
        };
        let limits = self.limits;
        let slots = Arc::make_mut(&mut self.slots);
        // 最初の変更に触れ得る最初の塊 (それより前の塊は終わりが変更の始まりより前で、変わらない)。前の塊は、小さな塊を
        // まとめる相手として一緒に通す
        let i0 = slots.partition_point(|s| s.end() < first.a);
        let tail: Vec<Slot> = slots.drain(i0.saturating_sub(1)..).collect();
        let mut walk = Walk::new(limits, tail.len());
        // p: 旧の終わりが今の記録 (塊) の始まり以上である最初の変更
        let mut p = 0usize;
        for slot in &tail {
            let start = slot.start();
            while hunks.get(p).is_some_and(|h| h.b < start) {
                p += 1;
            }
            if hunks.get(p).is_none_or(|h| h.a > slot.end()) {
                // どの変更にも触れない (閉区間で交わらない) 塊: 基準位置をずらして共有する
                walk.reuse(slot, map_point(&hunks[..p], slot.base));
                continue;
            }
            for i in 0..slot.len() {
                let r = slot.rec(i);
                let (s, e) = r.key();
                while hunks.get(p).is_some_and(|h| h.b < s) {
                    p += 1;
                }
                // 閉区間 [s, e] で交わる変更 (端に接するものを含む) を見て、最も強い関係を取る
                let mut class = Class::None;
                let mut q = p;
                while let Some(h) = hunks.get(q)
                    && h.a <= e
                {
                    class = class.max(h.classify(s, e));
                    q += 1;
                }
                let drop = match rule {
                    Rule::Edit => {
                        matches!(class, Class::Inside | Class::Straddle)
                            || (class == Class::Touch && r.line())
                            || (s == e && q > p)
                    }
                    Rule::Keep => matches!(class, Class::Inside | Class::Straddle),
                };
                if drop {
                    removed.push(r.to_span());
                    continue;
                }
                // 旧の終わりが s 以下の変更 (s に接する挿入・削除は、記録の前に入る)
                let k = if hunks.get(p).is_some_and(|h| h.b == s) {
                    p + 1
                } else {
                    p
                };
                walk.push(r.moved(map_point(&hunks[..k], s)));
            }
        }
        slots.extend(walk.finish());
        self.count -= removed.len();
        self.bytes -= spans_bytes(&removed);
        removed
    }

    /// 記録を入れる (昇順。今ある記録と重ならないこと)。
    fn insert_spans(&mut self, spans: &[PreservedSpan]) {
        let Some(first) = spans.first() else {
            return;
        };
        debug_assert!(spans.windows(2).all(|w| w[0].key() <= w[1].key()));
        let limits = self.limits;
        let slots = Arc::make_mut(&mut self.slots);
        // 最初の記録を入れる塊 (始まりがその位置以下の最後の塊)。その前の塊も、まとめる相手として通す
        let i0 = slots
            .partition_point(|s| s.start() <= first.range.start.0)
            .saturating_sub(1);
        let tail: Vec<Slot> = slots.drain(i0.saturating_sub(1)..).collect();
        let mut walk = Walk::new(limits, tail.len());
        let mut j = 0usize;
        for (t, slot) in tail.iter().enumerate() {
            // この塊に入れる記録: 次の塊の始まりより前 (最後の塊なら残り全部)
            let limit = tail.get(t + 1).map(Slot::start);
            let k = j + spans[j..].partition_point(|sp| limit.is_none_or(|l| sp.range.start.0 < l));
            if k == j {
                walk.reuse(slot, slot.base);
                continue;
            }
            let mut add = spans[j..k].iter().peekable();
            for i in 0..slot.len() {
                let r = slot.rec(i);
                while let Some(sp) = add.next_if(|sp| sp.key() <= r.key()) {
                    walk.push(sp.as_rec());
                }
                walk.push(r);
            }
            for sp in add {
                walk.push(sp.as_rec());
            }
            j = k;
        }
        // 表が空だったとき
        for sp in &spans[j..] {
            walk.push(sp.as_rec());
        }
        slots.extend(walk.finish());
        self.count += spans.len();
        self.bytes += spans_bytes(spans);
    }

    /// 記録を外す (昇順。位置・元のバイト列・理由・行の記録かが同じものを外す)。表に無かったものを返す。
    fn remove_spans(&mut self, targets: &[PreservedSpan]) -> Vec<PreservedSpan> {
        let mut missing = Vec::new();
        let Some(first) = targets.first() else {
            return missing;
        };
        debug_assert!(targets.windows(2).all(|w| w[0].key() <= w[1].key()));
        let limits = self.limits;
        let slots = Arc::make_mut(&mut self.slots);
        let i0 = slots.partition_point(|s| s.end() < first.range.start.0);
        let tail: Vec<Slot> = slots.drain(i0.saturating_sub(1)..).collect();
        let mut walk = Walk::new(limits, tail.len());
        let mut it = targets.iter().peekable();
        let mut gone = 0usize;
        let mut gone_bytes = 0usize;
        for slot in &tail {
            while let Some(t) = it.next_if(|t| t.range.start.0 < slot.start()) {
                missing.push(t.clone());
            }
            if it.peek().is_none_or(|t| t.range.start.0 > slot.end()) {
                walk.reuse(slot, slot.base);
                continue;
            }
            for i in 0..slot.len() {
                let r = slot.rec(i);
                while let Some(t) = it.next_if(|t| t.key() < r.key()) {
                    missing.push(t.clone());
                }
                if it.next_if(|t| r.same(t)).is_some() {
                    gone += 1;
                    gone_bytes += r.bytes.len();
                } else {
                    walk.push(r);
                }
            }
        }
        missing.extend(it.cloned());
        slots.extend(walk.finish());
        self.count -= gone;
        self.bytes -= gone_bytes;
        missing
    }

    /// 不変条件の検査 (試験と debug ビルド用。記録の総数に比例する): 塊の形、範囲が昇順で重ならない、本文の中で
    /// char 境界にある、行の記録かどうかが文字コードと合う、各記録の範囲の文字列が元のバイト列を初期状態から復号した
    /// ものと一致し、初期状態で終わる (03 §8.7.1 の不変条件)。
    ///
    /// # Errors
    ///
    /// 最初に見つけた食い違いの記録の番号と理由。
    pub(crate) fn check(&self, text: &dyn TextRead, enc: Encoding) -> Result<(), PreservedError> {
        self.check_structure()?;
        let mut buf = Scratch::default();
        for (index, r) in self.iter().enumerate() {
            let err = |kind| Err(PreservedError { index, kind });
            let (s, e) = (r.range.start, r.range.end);
            if e.0 > text.len_bytes() {
                return err(PreservedErrorKind::OutOfRange);
            }
            if !text.is_char_boundary(s) || !text.is_char_boundary(e) {
                return err(PreservedErrorKind::NotCharBoundary);
            }
            if r.line != enc.is_stateful() {
                return err(PreservedErrorKind::WrongUnit);
            }
            if !matches_original(enc, r.bytes, text, r.range, &mut buf) {
                return err(PreservedErrorKind::Mismatch);
            }
        }
        Ok(())
    }

    /// 塊の形の検査 (本文を見ない)。
    fn check_structure(&self) -> Result<(), PreservedError> {
        let bad = |index| {
            Err(PreservedError {
                index,
                kind: PreservedErrorKind::Structure,
            })
        };
        let mut index = 0usize;
        let mut bytes = 0usize;
        let mut prev_end = 0usize;
        for slot in self.slots.iter() {
            let c = &slot.chunk;
            if c.entries.is_empty() || c.entries[0].start != 0 {
                return bad(index);
            }
            if c.entries.len() > self.limits.max() {
                return bad(index);
            }
            let mut b0 = 0u32;
            for i in 0..slot.len() {
                let e = c.entries[i];
                if e.bytes_end <= b0 || e.flags & !(FLAG_LINE | 3) != 0 || e.flags & 3 == 3 {
                    return bad(index);
                }
                b0 = e.bytes_end;
                let r = slot.rec(i);
                if r.start < prev_end {
                    return Err(PreservedError {
                        index,
                        kind: PreservedErrorKind::NotSorted,
                    });
                }
                prev_end = r.end();
                bytes += r.bytes.len();
                index += 1;
            }
            if b0 as usize != c.bytes.len() {
                return bad(index);
            }
        }
        if index != self.count || bytes != self.bytes {
            return bad(index);
        }
        Ok(())
    }

    /// 塊の大きさを変えて作り直す (試験で、小さな塊にして塊の境をまたぐ編集を通す)。
    #[cfg(test)]
    pub(crate) fn rechunk(&mut self, limits: ChunkLimits) {
        let mut b = Builder::new(limits, self.count);
        for slot in self.slots.iter() {
            for i in 0..slot.len() {
                b.push(slot.rec(i));
            }
        }
        self.slots = Arc::new(b.finish());
        self.limits = limits;
    }

    /// 塊の数 (試験用)。
    #[cfg(test)]
    pub(crate) fn chunk_count(&self) -> usize {
        self.slots.len()
    }

    /// 全部の記録の複製 (試験用)。
    #[cfg(test)]
    pub(crate) fn to_spans(&self) -> Vec<PreservedSpan> {
        self.iter().map(|r| r.to_span()).collect()
    }
}

/// `replace_range` の、入れる記録を 1 回だけ足す。
fn emit_once<'a>(walk: &mut Walk<'a>, spans: &'a [PreservedSpan], emitted: &mut bool) {
    if !*emitted {
        *emitted = true;
        for s in spans {
            walk.push(s.as_rec());
        }
    }
}

/// `PreservedMap` の昇順の走査。
#[derive(Clone, Debug)]
pub(crate) struct Iter<'a> {
    slots: &'a [Slot],
    si: usize,
    ei: usize,
}

impl Iter<'_> {
    /// 塊の終わりを指していれば、次の塊の先頭へ進める。
    fn normalized(mut self) -> Self {
        while self.slots.get(self.si).is_some_and(|s| self.ei >= s.len()) {
            self.si += 1;
            self.ei = 0;
        }
        self
    }
}

impl<'a> Iterator for Iter<'a> {
    type Item = SpanRef<'a>;

    fn next(&mut self) -> Option<SpanRef<'a>> {
        let slot = self.slots.get(self.si)?;
        let r = slot.rec(self.ei);
        self.ei += 1;
        if self.ei >= slot.len() {
            self.si += 1;
            self.ei = 0;
        }
        Some(r.view())
    }
}

/// 位置だけの写像 (`Rule::Keep` と同じ規則) を、表の外の記録の列に掛ける。Undo のノードを結合 (03 §7.3) するとき、
/// 後の Transaction が外した記録 (後の Transaction の前の座標) を、ノードの前の座標へ戻すのに使う
/// (`older_undo` はノードの結合前の Undo の `ChangeSet`)。`spans` は昇順。
pub(crate) fn map_spans(spans: &mut [PreservedSpan], cs: &ChangeSet) {
    let hunks = hunks_of(cs);
    let mut p = 0usize;
    for sp in spans.iter_mut() {
        let (s, len) = (sp.range.start.0, sp.range.len());
        while hunks.get(p).is_some_and(|h| h.b < s) {
            p += 1;
        }
        let k = if hunks.get(p).is_some_and(|h| h.b == s) {
            p + 1
        } else {
            p
        };
        let ns = map_point(&hunks[..k], s);
        sp.range = ByteRange::new(ByteOff(ns), ByteOff(ns + len));
    }
}

/// Undo のノードの結合 (03 §7.3 / §8.7.1 の「ノードを結合したときは、外した記録も合わせて持つ」): 前のノードが外した
/// 記録 `older` (ノードの前の座標) と、結合する Transaction が外した記録 `newer` (その Transaction の前の座標) を、
/// ノードの前の座標で 1 つの昇順の列にする。`older_undo` は結合する前のノードの Undo の `ChangeSet`。
pub(crate) fn merge_removed(
    older: Vec<PreservedSpan>,
    mut newer: Vec<PreservedSpan>,
    older_undo: &ChangeSet,
) -> Vec<PreservedSpan> {
    map_spans(&mut newer, older_undo);
    if older.is_empty() {
        return newer;
    }
    let mut out = Vec::with_capacity(older.len() + newer.len());
    let mut a = older.into_iter().peekable();
    let mut b = newer.into_iter().peekable();
    loop {
        let take_a = match (a.peek(), b.peek()) {
            (Some(x), Some(y)) => x.key() <= y.key(),
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };
        out.extend(if take_a { a.next() } else { b.next() });
    }
    out
}

/// 元のバイト列 `bytes` を `enc` の初期状態から復号すると、本文 `text` の `range` の文字列になり、初期状態で終わるか
/// (03 §8.7.1 の不変条件。eve-encoding の `fuzzing::check_preserved` と同じ意味)。`buf` は使い回す作業用の文字列
/// (100 万件の照合で、1 件ごとに確保しない)。
///
/// ISO-2022-JP の「ASCII 状態で終わる」は、`Decoder` が復号の後の状態を公開しないので、後ろに `\~` (0x5C 0x7E) を足して
/// 復号し、その 2 文字がそのまま出ることで確かめる (ローマ字状態では `¥‾`、半角カナ状態では半角カナか不正バイト、
/// JIS X 0208 状態では漢字 1 字になる)。
fn matches_original(
    enc: Encoding,
    bytes: &[u8],
    text: &dyn TextRead,
    range: ByteRange,
    buf: &mut Scratch,
) -> bool {
    const PROBE: &str = "\\~";
    let Scratch { want, out } = buf;
    want.clear();
    for c in text.chunks(range) {
        want.push_str(c);
    }
    if want.len() != range.len() {
        return false;
    }
    out.clear();
    let mut d = Decoder::new(enc, false);
    if enc.is_stateful() {
        d.decode(bytes, false, out);
        d.decode(PROBE.as_bytes(), true, out);
        d.invalid_bytes() == 0 && out.strip_suffix(PROBE) == Some(want.as_str())
    } else {
        d.decode(bytes, true, out);
        d.invalid_bytes() == 0 && out == want
    }
}

/// `matches_original` の作業用の文字列 (範囲の本文と、元のバイト列の復号の結果)。
#[derive(Default)]
struct Scratch {
    want: String,
    out: String,
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod props;
