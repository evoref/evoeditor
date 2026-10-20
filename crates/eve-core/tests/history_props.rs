//! Undo 履歴のプロパティテスト (docs/03-core-design.md §7、09 §22.1 の FR-203 / FR-204)。
//!
//! ランダムな操作列 (入力・削除・Undo・Redo・封印・保存・時間経過) の後で:
//! - すべて Undo すると初期の内容に戻り、すべて Redo すると最後の内容に戻る。
//! - `is_dirty() == false` なら、内容は最後に保存した時点と同じ (INV-H3: 戻れない位置を保存点にしない)。
//!
//! 文書テキストの変更は本来 `Document::apply` (document モジュール) の仕事。ここでは履歴だけを試すため、
//! 試験用の最小の適用を持つ。

#![allow(clippy::disallowed_methods)]

use eve_core::{Millis, Origin, Tick, Transaction, UndoBudget, UndoGroupHint, UndoHistory};
use eve_text::{ByteRange, ChangeSet, Edit, Text, TextRead};
use proptest::prelude::*;

#[derive(Clone, Debug)]
enum Act {
    Type { at: u16, s: &'static str },
    Delete { at: u16, len: u8 },
    Paste { at: u16 },
    Undo,
    Redo,
    Seal,
    Save,
    Wait(u16),
}

fn act() -> impl Strategy<Value = Act> {
    prop_oneof![
        6 => (any::<u16>(), proptest::sample::select(vec!["a", "b", "あ", "\n", "xy", "\r\n"]))
            .prop_map(|(at, s)| Act::Type { at, s }),
        4 => (any::<u16>(), 1u8..4).prop_map(|(at, len)| Act::Delete { at, len }),
        1 => any::<u16>().prop_map(|at| Act::Paste { at }),
        3 => Just(Act::Undo),
        2 => Just(Act::Redo),
        1 => Just(Act::Seal),
        1 => Just(Act::Save),
        2 => (0u16..1500).prop_map(Act::Wait),
    ]
}

struct Doc {
    text: Text,
    h: UndoHistory,
    now: u64,
    saved_text: Option<String>,
}

impl Doc {
    fn s(&self) -> String {
        self.text
            .slice_to_string(ByteRange::all(self.text.len_bytes()))
    }

    /// `at` を char 境界に丸めた位置。
    fn pos(&self, at: u16) -> usize {
        let s = self.s();
        let mut p = (at as usize) % (s.len() + 1);
        while !s.is_char_boundary(p) {
            p -= 1;
        }
        p
    }

    fn apply_edit(&mut self, start: usize, end: usize, ins: &str, origin: Origin) {
        let snap = self.text.snapshot();
        let cs = ChangeSet::from_sorted_edits(
            &snap,
            [Edit {
                range: ByteRange::from(start..end),
                insert: ins.into(),
            }],
        )
        .unwrap();
        if cs.is_identity() {
            return;
        }
        let inverse = cs.invert(&snap);
        self.text.apply(&cs).unwrap();
        let tx = Transaction::new(cs, origin, Tick(self.now)).with_undo_group(UndoGroupHint::Auto);
        self.h.record(&tx, inverse);
    }

    fn run(&mut self, a: &Act) {
        match a {
            Act::Type { at, s } => {
                let p = self.pos(*at);
                self.apply_edit(p, p, s, Origin::User);
            }
            Act::Delete { at, len } => {
                let st = self.pos(*at);
                let mut en = (st + *len as usize).min(self.text.len_bytes());
                let s = self.s();
                while !s.is_char_boundary(en) {
                    en += 1;
                }
                self.apply_edit(st, en, "", Origin::User);
            }
            Act::Paste { at } => {
                let p = self.pos(*at);
                self.apply_edit(p, p, "PASTE", Origin::Paste);
            }
            Act::Undo => {
                if let Some(step) = self.h.undo_step() {
                    self.text.apply(&step.changes).unwrap();
                }
            }
            Act::Redo => {
                if let Some(step) = self.h.redo_step() {
                    self.text.apply(&step.changes).unwrap();
                }
            }
            Act::Seal => self.h.seal(),
            Act::Save => {
                self.h.seal();
                self.h.mark_saved();
                self.saved_text = Some(self.s());
            }
            Act::Wait(ms) => self.now += u64::from(*ms),
        }
    }
}

fn new_doc(initial: &str, budget: UndoBudget) -> Doc {
    Doc {
        text: Text::from(initial),
        h: UndoHistory::new(budget),
        now: 0,
        saved_text: Some(initial.to_owned()),
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    #[test]
    fn すべて_undo_と_redo_で往復する(initial in "[ab\n]{0,10}", acts in proptest::collection::vec(act(), 0..60)) {
        let mut d = new_doc(&initial, UndoBudget::default());
        for a in &acts {
            d.run(a);
            if !d.h.is_dirty() {
                prop_assert_eq!(Some(d.s()), d.saved_text.clone(), "未変更なのに保存時点の内容と違う: {:?}", a);
            }
        }
        let last = d.s();
        while d.h.can_redo() {
            d.run(&Act::Redo);
        }
        let end = d.s();
        while d.h.can_undo() {
            d.run(&Act::Undo);
        }
        prop_assert_eq!(d.s(), initial, "すべて Undo すると初期に戻る");
        while d.h.can_redo() {
            d.run(&Act::Redo);
        }
        prop_assert_eq!(d.s(), end, "すべて Redo すると最後に戻る");
        let _ = last;
    }

    #[test]
    fn 予算で破棄しても未変更の判定は正しい(initial in "[ab\n]{0,10}", acts in proptest::collection::vec(act(), 0..80)) {
        let mut d = new_doc(&initial, UndoBudget { max_bytes: 400, max_nodes: 4 });
        for a in &acts {
            d.run(a);
            if !d.h.is_dirty() {
                prop_assert_eq!(Some(d.s()), d.saved_text.clone(), "破棄の後に誤って未変更: {:?}", a);
            }
            prop_assert!(d.h.undo_len() <= 4);
        }
    }
}

#[test]
fn 時間は_millis_で扱う() {
    // 結合の窓 (1000 ms) の型が Millis であること
    assert_eq!(Tick(1500).saturating_sub(Tick(500)), Millis(1000));
}
