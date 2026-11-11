//! 文書 (`Document`) の単位の、保持方式の記録の性質テスト (03 §22.1 の保持方式の (c)(d)、M22 の 5)。
//!
//! `props.rs` が表 (`PreservedMap`) だけを素朴な実装と比べるのに対し、ここは `Document::apply`・`undo`・`redo` を通す
//! (ノードの結合・粗い逆変換・再読込・復旧・Undo の予算による底の破棄を含む)。区分 B の文字を多く含む `Shift_JIS` /
//! EUC-JP / ISO-2022-JP の文書に、ランダムな編集・Undo・Redo の列を当て、次を確かめる。
//!
//! - (c) どの時点でも、表の各記録が不変条件を満たす (`PreservedMap::check`)。Undo / Redo で表と履歴が食い違わない。
//! - (d) 同じ履歴の位置 (`NodeId`) では、Undo と Redo をどう混ぜても、表・保持方式の状態・本文が同じ。全部 Undo すると
//!   開いた直後に戻る (底を捨てていなければ)。

use std::collections::HashMap;

use eve_encoding::{
    Decoder, Encoding, NoPreserveReason, PreserveStatus, check_roundtrip_with_records,
};
use eve_text::{
    ByteOff, ByteRange, ChangeSet, CharIdx, Edit, InsertText, LineIdx, Op, Text, TextRead,
};
use proptest::prelude::*;

use super::props::{INSERTS, doc, floor_boundary};
use super::tests::all;
use super::*;
use crate::document::{
    Document, FormatChange, FormatState, NodeId, Origin, ReadOnlyReasons, TextFormat, Transaction,
    UndoGroupHint, count_long_lines,
};
use crate::time::Tick;

/// バイト列を読込のジョブと同じ手順で文書にする (復号・記録を集める・表にして照合する)。
pub(crate) fn open_doc(enc: Encoding, bytes: &[u8]) -> Document {
    let (text, format, map) = loaded(enc, bytes);
    Document::from_loaded(text, format, map)
}

/// 読込の結果 (本文・形式・表)。
fn loaded(enc: Encoding, bytes: &[u8]) -> (Text, TextFormat, PreservedMap) {
    let mut d = Decoder::new(enc, false);
    let mut s = String::new();
    d.decode(bytes, true, &mut s);
    let (mut summary, recs) = check_roundtrip_with_records(enc, bytes);
    let text = Text::from(s.as_str());
    let map = PreservedMap::from_records(&recs, enc, &text).expect("照合が通る");
    summary.preserve = PreserveStatus::Active {
        unpreserved: recs.unpreserved,
    };
    let mut format = TextFormat::new_document();
    format.encoding = enc;
    format.roundtrip = summary;
    format.invalid_bytes = d.invalid_bytes();
    format.eol_stats = crate::document::EolStats::count(&text);
    format.long_lines = count_long_lines(&text, format.long_line_threshold);
    (text, format, map)
}

/// 1 つの操作: (種類, 編集の部品の列, 付け足し)。
type Action = (u8, Vec<(u8, u16, u8, u8)>, u8);

fn actions() -> impl Strategy<Value = Vec<Action>> {
    proptest::collection::vec(
        (
            any::<u8>(),
            proptest::collection::vec((any::<u8>(), any::<u16>(), any::<u8>(), any::<u8>()), 1..4),
            any::<u8>(),
        ),
        0..24,
    )
}

/// ある履歴の位置での、文書の状態 (表・保持方式の状態・本文)。
#[derive(Clone, Debug, PartialEq, Eq)]
struct At {
    map: PreservedMap,
    status: PreserveStatus,
    text: String,
}

struct World {
    enc: Encoding,
    bytes: Vec<u8>,
    doc: Document,
    now: u64,
    /// 履歴の位置ごとの状態 (位置は通番なので、結合・破棄で別の位置と取り違えない)。
    seen: HashMap<NodeId, At>,
    /// 底を捨てた (全部 Undo しても開いた直後に戻らない)。
    trimmed: bool,
}

impl World {
    fn at(&self) -> At {
        At {
            map: self.doc.preserved().clone(),
            status: self.doc.format.roundtrip.preserve,
            text: all(self.doc.text()),
        }
    }

    fn check(&self) -> Result<(), TestCaseError> {
        // (c) 不変条件。表と履歴が食い違っていない
        prop_assert_eq!(
            self.doc.preserved().check(self.doc.text(), self.enc),
            Ok(())
        );
        prop_assert_eq!(self.doc.preserved_anomalies(), 0);
        // 会計値の合計がノードの合計と合う (`bytes` の debug_assert)
        let _ = self.doc.history().bytes();
        Ok(())
    }

    /// 新しい位置に来た: 状態を覚える。
    fn remember(&mut self) -> Result<(), TestCaseError> {
        self.check()?;
        let at = self.at();
        self.seen.insert(self.doc.history().current(), at);
        Ok(())
    }

    /// Undo / Redo で前に居た位置に戻った: 状態が同じ (d)。
    fn revisit(&self) -> Result<(), TestCaseError> {
        self.check()?;
        let cur = self.doc.history().current();
        let want = self.seen.get(&cur);
        prop_assert!(want.is_some(), "Undo / Redo で戻った位置は、前に居た位置");
        prop_assert_eq!(&self.at(), want.unwrap());
        Ok(())
    }

    /// 編集の部品から `ChangeSet` を作る (位置は、任意の位置・記録の始まり・終わり・行の終端・文書の端から選び、char
    /// 境界に丸める)。恒等なら `None`。
    fn make_cs(&self, parts: &[(u8, u16, u8, u8)]) -> Option<ChangeSet> {
        let t = self.doc.text();
        let len = t.len_bytes();
        let recs: Vec<(usize, usize)> = self
            .doc
            .preserved()
            .ranges()
            .map(|(r, _)| (r.start.0, r.end.0))
            .collect();
        let mut edits: Vec<(usize, usize, &str)> = parts
            .iter()
            .map(|&(anchor, sel, del, ins)| {
                let sel = usize::from(sel);
                let p = match (anchor % 6, recs.is_empty()) {
                    (1, false) => recs[sel % recs.len()].0,
                    (2, false) => recs[sel % recs.len()].1,
                    (3, _) => {
                        if sel % 2 == 0 {
                            0
                        } else {
                            len
                        }
                    }
                    // 行の終端の位置 (行の結合・分割)
                    (4, _) => t.line_end_byte(LineIdx(sel % t.len_lines())).0,
                    _ => sel % (len + 1),
                };
                let s = floor_boundary(t, p);
                let n = usize::from(del % 4);
                let c = t.byte_to_char(ByteOff(s)).0;
                let (s, e) = if del & 0x80 != 0 && n > 0 {
                    (t.char_to_byte(CharIdx(c.saturating_sub(n))).0, s)
                } else {
                    (s, t.char_to_byte(CharIdx(c + n)).0.min(len))
                };
                (s, e, INSERTS[usize::from(ins) % INSERTS.len()])
            })
            .collect();
        edits.sort_by_key(|e| (e.0, e.1));
        let mut ok: Vec<(usize, usize, &str)> = Vec::new();
        for e in edits {
            if ok.last().is_none_or(|l| e.0 >= l.1) {
                ok.push(e);
            }
        }
        let cs = ChangeSet::from_sorted_edits(
            t,
            ok.iter().map(|&(s, e, x)| Edit {
                range: ByteRange::new(ByteOff(s), ByteOff(e)),
                insert: x.into(),
            }),
        )
        .expect("妥当な編集");
        (!cs.is_identity()).then_some(cs)
    }

    fn apply(&mut self, tx: Transaction) -> Result<(), TestCaseError> {
        let r = self.doc.apply(&tx);
        prop_assert!(r.is_ok(), "適用できる: {:?}", r.err());
        self.remember()
    }

    /// すべて置換と同じ形の編集 (`Origin::Command`・`Break`・粗い逆変換の可)。変更の間を詰めるため、部品の列から
    /// 作った位置に加えて、同じ文字の置き換えを多数足す。
    fn replace_all(&mut self, parts: &[(u8, u16, u8, u8)]) -> Result<(), TestCaseError> {
        let t = self.doc.text();
        let s = all(t);
        // 文書の中の ASCII 小文字を全部、別の文字列に置き換える (区分 B の文字の両側の置き換えになる)
        let with = INSERTS[usize::from(parts[0].3) % INSERTS.len()];
        let edits: Vec<Edit> = s
            .char_indices()
            .filter(|(_, c)| c.is_ascii_lowercase())
            .map(|(i, c)| Edit {
                range: ByteRange::new(ByteOff(i), ByteOff(i + c.len_utf8())),
                insert: with.into(),
            })
            .collect();
        let Some(cs) = ChangeSet::from_sorted_edits(t, edits)
            .ok()
            .filter(|c| !c.is_identity())
        else {
            return Ok(());
        };
        self.now += 10;
        let tx = Transaction::new(
            cs,
            Origin::Command(eve_view::CommandId::new("search.replace_all")),
            Tick(self.now),
        )
        .with_undo_group(UndoGroupHint::Break)
        .with_coarse_undo();
        self.apply(tx)
    }

    /// 再読込 (03 §9.5): 元のバイト列を読み直した結果に、本文・書式・表を入れ替える。
    fn reload(&mut self) -> Result<(), TestCaseError> {
        let (text, format, map) = loaded(self.enc, &self.bytes);
        let change = FormatChange::Reload {
            before: FormatState {
                format: self.doc.format.clone(),
                read_only: self.doc.read_only,
                preserved: self.doc.preserved().clone(),
            },
            after: FormatState {
                format,
                read_only: ReadOnlyReasons::default(),
                preserved: map,
            },
        };
        let cs = ChangeSet::from_ops([
            Op::Delete(self.doc.len_bytes()),
            Op::Insert(InsertText::Rope(text.snapshot())),
        ]);
        self.now += 10;
        self.apply(Transaction::new(cs, Origin::Reload, Tick(self.now)).with_format_change(change))
    }

    /// 復旧 (03 §10.3): 文書の全体を置き換える。表は空になり「保持なし (復旧した)」。
    fn recover(&mut self, with: &str) -> Result<(), TestCaseError> {
        let t = self.doc.text();
        let Ok(cs) = ChangeSet::from_sorted_edits(
            t,
            [Edit {
                range: ByteRange::all(t.len_bytes()),
                insert: with.into(),
            }],
        ) else {
            return Ok(());
        };
        if cs.is_identity() {
            return Ok(());
        }
        self.now += 10;
        self.apply(Transaction::new(cs, Origin::Recovery, Tick(self.now)))?;
        prop_assert!(self.doc.preserved().is_empty());
        prop_assert_eq!(
            self.doc.format.roundtrip.preserve,
            PreserveStatus::Unavailable(NoPreserveReason::Recovered)
        );
        Ok(())
    }

    fn undo(&mut self) -> Result<bool, TestCaseError> {
        let r = self.doc.undo();
        prop_assert!(r.is_ok());
        let done = r.unwrap().is_some();
        if done {
            self.revisit()?;
        }
        Ok(done)
    }

    fn redo(&mut self) -> Result<bool, TestCaseError> {
        let r = self.doc.redo();
        prop_assert!(r.is_ok());
        let done = r.unwrap().is_some();
        if done {
            self.revisit()?;
        }
        Ok(done)
    }
}

fn run(enc: Encoding, bytes: &[u8], acts: &[Action]) -> Result<(), TestCaseError> {
    let doc = open_doc(enc, bytes);
    let mut w = World {
        enc,
        bytes: bytes.to_vec(),
        doc,
        now: 0,
        seen: HashMap::new(),
        trimmed: false,
    };
    w.remember()?;
    let initial = w.at();
    for (kind, parts, extra) in acts {
        match kind % 16 {
            0..=6 => {
                if let Some(cs) = w.make_cs(parts) {
                    // 時刻の差が小さい User の入力は結合し得る (R7 / R8)。Merge の指示は必ず結合する (R1)
                    w.now += u64::from(extra % 4) * 400;
                    let origin = if extra & 4 != 0 {
                        Origin::Paste
                    } else {
                        Origin::User
                    };
                    let hint = if extra & 8 != 0 {
                        UndoGroupHint::Merge
                    } else {
                        UndoGroupHint::Auto
                    };
                    let tx = Transaction::new(cs, origin, Tick(w.now)).with_undo_group(hint);
                    w.apply(tx)?;
                }
            }
            7 => w.replace_all(parts)?,
            8..=10 => {
                w.undo()?;
            }
            11 | 12 => {
                w.redo()?;
            }
            13 => w.reload()?,
            14 => {
                let with = INSERTS[usize::from(*extra) % INSERTS.len()];
                w.recover(with)?;
            }
            _ => {
                // Undo の予算で底を捨てる (03 §7.5。捨てたノードの記録はノードと一緒に消える)
                if w.doc.trim_history_oldest().is_some() {
                    w.trimmed = true;
                }
                w.check()?;
            }
        }
    }
    // (d) 全部 Undo してから全部 Redo しても、どの位置でも同じ状態 (revisit が確かめる)
    while w.undo()? {}
    if !w.trimmed {
        prop_assert_eq!(&w.at(), &initial, "全部 Undo すると開いた直後に戻る");
    }
    while w.redo()? {}
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    #[test]
    fn 文書の編集とundo_redoで記録が不変条件を保ち_同じ位置では同じ表になる(
        (enc, bytes) in doc(),
        acts in actions(),
    ) {
        run(enc, &bytes, &acts)?;
    }
}

/// 外した記録の元のバイト列は、ノードの会計値 (Undo の予算、03 §7.5) に数える。底を捨てると一緒に消える。
#[test]
fn 外した記録は_undo_の会計に数え_底を捨てると一緒に消える() {
    // ￢ (0xFA54、IBM 拡張の重複) を 3 つ
    let bytes = b"a\xFA\x54b\xFA\x54c\xFA\x54\n";
    let mut doc = open_doc(Encoding::ShiftJis, bytes);
    assert_eq!(doc.preserved().len(), 3);
    let base = doc.history().bytes();
    assert_eq!(base, 0);
    // 1 つ目の ￢ を消す (Paste の Origin で 1 ノード)
    let cs = super::tests::cs(doc.text(), &[(1, 4, "")]);
    doc.apply(&Transaction::new(cs, Origin::Paste, Tick(0)))
        .unwrap();
    assert_eq!(doc.preserved().len(), 2);
    let one = doc.history().bytes();
    // 2 つ目を消す
    let cs = super::tests::cs(doc.text(), &[(2, 5, "")]);
    doc.apply(&Transaction::new(cs, Origin::Paste, Tick(10)))
        .unwrap();
    let two = doc.history().bytes();
    // ノードの固定の 128 バイト + 削除した本文 (￢ の UTF-8 の 3 バイト) + 外した記録の元のバイト列 (2 バイト)
    assert_eq!(one, 128 + 3 + 2);
    assert_eq!(two, 2 * one);
    // 底を捨てる: 1 つ目のノードの記録はノードと一緒に消え、Undo で戻るのは 2 つ目だけ
    assert!(doc.trim_history_oldest().is_some());
    assert_eq!(doc.history().bytes(), two - one);
    doc.undo().unwrap();
    assert_eq!(doc.preserved().len(), 2);
    assert_eq!(
        doc.preserved().check(doc.text(), Encoding::ShiftJis),
        Ok(())
    );
    assert!(doc.undo().unwrap().is_none(), "捨てたノードには戻れない");
    assert_eq!(doc.preserved_anomalies(), 0);
}

/// ノードを結合したときは、外した記録も合わせて持ち、1 回の Undo で全部戻る (03 §7.3 / §8.7.1)。
#[test]
fn 結合したノードの_undo_で外した記録が全部戻る() {
    let bytes = b"\xFA\x54\xFA\x55x\xFA\x54\n";
    let mut doc = open_doc(Encoding::ShiftJis, bytes);
    let initial = doc.preserved().clone();
    assert_eq!(
        initial.len(),
        2,
        "￢ (重複) が 2 件。∵ (0xFA55) は互換文字で記録にならない"
    );
    // Backspace の連打 (末尾の ￢ から前へ 1 文字ずつ)
    let mut now = 0;
    let mut end = doc.len_bytes() - 1;
    while end > 0 {
        let start = floor_boundary(doc.text(), end - 1);
        let cs = super::tests::cs(doc.text(), &[(start, end, "")]);
        now += 100;
        doc.apply(&Transaction::new(cs, Origin::User, Tick(now)))
            .unwrap();
        end = start;
    }
    assert_eq!(all(doc.text()), "\n");
    assert!(doc.preserved().is_empty());
    assert_eq!(doc.history().undo_len(), 1, "1 ノードに結合した");
    doc.undo().unwrap();
    assert_eq!(doc.preserved(), &initial);
    doc.redo().unwrap();
    assert!(doc.preserved().is_empty());
    assert_eq!(doc.preserved_anomalies(), 0);
}

/// 粗い逆変換 (03 §12.5) のノードの Undo は、範囲の記録を編集の前のものに入れ替え、Redo は編集の後のものにする。
#[test]
fn 粗い逆変換のノードは範囲の記録を入れ替える() {
    // "x￢" を 200 回 (￢ は 0xFA54)。x を全部 y にすると、変更が密なので粗い逆変換になる
    let bytes: Vec<u8> = b"x\xFA\x54".repeat(200);
    let mut doc = open_doc(Encoding::ShiftJis, &bytes);
    let initial = doc.preserved().clone();
    assert_eq!(initial.len(), 200);
    let s = all(doc.text());
    let edits: Vec<(usize, usize, &str)> = s
        .char_indices()
        .filter(|(_, c)| *c == 'x')
        .map(|(i, _)| (i, i + 1, "yy"))
        .collect();
    let cs = super::tests::cs(doc.text(), &edits);
    let tx = Transaction::new(
        cs,
        Origin::Command(eve_view::CommandId::new("search.replace_all")),
        Tick(0),
    )
    .with_undo_group(UndoGroupHint::Break)
    .with_coarse_undo();
    doc.apply(&tx).unwrap();
    let after = doc.preserved().clone();
    assert_eq!(after.len(), 200, "端に接する置換では文字の範囲の記録は残る");
    assert_eq!(
        doc.preserved().check(doc.text(), Encoding::ShiftJis),
        Ok(())
    );
    doc.undo().unwrap();
    assert_eq!(doc.preserved(), &initial);
    doc.redo().unwrap();
    assert_eq!(doc.preserved(), &after);
    assert_eq!(doc.preserved_anomalies(), 0);
}
