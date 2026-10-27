//! `Document::apply` のテスト (docs/03-core-design.md §6.5): 手順 1 の検査、Revision、`eol_stats` の更新 (INV-D5)、
//! Undo / Redo、保存点。`eol_stats` は、どの操作の後でも文書全体の数え直しと一致することをプロパティテストで確かめる。

use eve_core::{
    AppliedTx, ApplyError, Document, Origin, ReadOnlyReason, Tick, Transaction, UndoGroupHint,
};
use eve_text::{ByteRange, ChangeSet, Edit, Selection, TextRead};
use eve_view::Revision;
use proptest::prelude::*;

fn edit(doc: &Document, start: usize, end: usize, ins: &str) -> ChangeSet {
    ChangeSet::from_sorted_edits(
        &doc.snapshot(),
        [Edit {
            range: ByteRange::from(start..end),
            insert: ins.into(),
        }],
    )
    .unwrap()
}

fn user(cs: ChangeSet, t: u64) -> Transaction {
    Transaction::new(cs, Origin::User, Tick(t))
}

#[test]
fn 適用すると_revision_が進み_dirty_になる() {
    let mut d = Document::new_untitled();
    assert_eq!((d.revision(), d.is_dirty()), (Revision(0), false));
    let cs = edit(&d, 0, 0, "hello");
    let AppliedTx::Changed(a) = d.apply(&user(cs, 0)).unwrap() else {
        panic!()
    };
    assert_eq!((a.old_revision, a.revision), (Revision(0), Revision(1)));
    assert_eq!(d.to_string_lossy(), "hello");
    assert!(d.is_dirty());
    assert!(a.record.is_some_and(|r| !r.merged && !r.pruned));
}

#[test]
fn 恒等の_transaction_は何も変えない() {
    let mut d = Document::new_untitled();
    d.apply(&user(edit(&d, 0, 0, "ab"), 0)).unwrap();
    let rev = d.revision();
    let history_len = d.history().undo_len();
    let sel = Selection::point(eve_text::ByteOff(1));
    let tx = user(ChangeSet::identity(2), 10).with_selections(None, Some(sel.clone()));
    let AppliedTx::Identity { selection } = d.apply(&tx).unwrap() else {
        panic!()
    };
    assert_eq!(selection, Some(sel));
    assert_eq!(
        (d.revision(), d.history().undo_len()),
        (rev, history_len),
        "Revision も履歴も変わらない"
    );
}

#[test]
fn 古い_revision_に基づく変更は_stalebase() {
    let mut d = Document::new_untitled();
    let stale = edit(&d, 0, 0, "x");
    d.apply(&user(edit(&d, 0, 0, "abc"), 0)).unwrap();
    match d.apply(&user(stale, 5)) {
        Err(ApplyError::StaleBase {
            expected: 0,
            actual: 3,
        }) => {}
        other => panic!("{other:?}"),
    }
    assert_eq!(d.to_string_lossy(), "abc", "文書は変わらない");
}

#[test]
fn 読み取り専用と読込中は拒否し_再読込と復旧だけが例外() {
    let mut d = Document::new_untitled();
    d.read_only.insert(ReadOnlyReason::InvalidBytes);
    assert!(matches!(
        d.apply(&user(edit(&d, 0, 0, "x"), 0)),
        Err(ApplyError::Rejected)
    ));
    for origin in [Origin::Reload, Origin::Recovery] {
        let mut e = Document::new_untitled();
        e.read_only.insert(ReadOnlyReason::InvalidBytes);
        let tx = Transaction::new(edit(&e, 0, 0, "x"), origin, Tick(0));
        assert!(matches!(e.apply(&tx), Ok(AppliedTx::Changed(_))));
    }
    assert!(matches!(d.undo(), Err(ApplyError::Rejected)));
    let mut l = Document::new_untitled();
    l.load_state = eve_core::LoadState::Loading;
    assert!(matches!(
        l.apply(&user(edit(&l, 0, 0, "x"), 0)),
        Err(ApplyError::Rejected)
    ));
}

#[test]
fn char_境界でない変更は拒否して何も変えない() {
    let mut d = Document::new_untitled();
    d.apply(&user(edit(&d, 0, 0, "あ"), 0)).unwrap();
    // 文書の途中 (char の内側) を指す ChangeSet は from_sorted_edits が作れないので、Retain で組み立てる
    let cs = ChangeSet::from_ops([
        eve_text::Op::Retain(1),
        eve_text::Op::Insert("x".into()),
        eve_text::Op::Retain(2),
    ]);
    assert!(matches!(d.apply(&user(cs, 1)), Err(ApplyError::Text(_))));
    assert_eq!(d.to_string_lossy(), "あ");
    assert_eq!(d.revision(), Revision(1));
}

#[test]
fn undo_と_redo_は選択を復元し_origin_を保つ() {
    let mut d = Document::new_untitled();
    let before = Selection::point(eve_text::ByteOff(0));
    let after = Selection::point(eve_text::ByteOff(3));
    let tx = Transaction::new(edit(&d, 0, 0, "abc"), Origin::Paste, Tick(0))
        .with_selections(Some(before.clone()), Some(after.clone()))
        .with_undo_group(UndoGroupHint::Auto);
    d.apply(&tx).unwrap();
    let u = d.undo().unwrap().unwrap();
    assert_eq!(
        (
            d.to_string_lossy(),
            u.selection,
            u.origin.clone(),
            u.is_undo
        ),
        (String::new(), Some(before), Origin::Paste, true)
    );
    assert_eq!(u.revision, Revision(2), "Undo でも revision は進む");
    assert!(!d.is_dirty(), "保存点 (根) に戻った");
    let r = d.redo().unwrap().unwrap();
    assert_eq!(
        (d.to_string_lossy(), r.selection),
        ("abc".to_owned(), Some(after))
    );
    assert!(
        d.undo().unwrap().is_some() && d.undo().unwrap().is_none(),
        "これ以上戻せない"
    );
}

#[test]
fn 保存の流れ() {
    let mut d = Document::new_untitled();
    d.apply(&user(edit(&d, 0, 0, "a"), 0)).unwrap();
    let (snap, at) = d.begin_save();
    assert_eq!(snap.len_bytes(), 1);
    // 保存中の編集は新しいノードになり、dirty のまま残る
    d.apply(&user(edit(&d, 1, 1, "b"), 10)).unwrap();
    d.finish_save(at, None);
    assert!(d.is_dirty());
    d.undo().unwrap();
    assert!(!d.is_dirty(), "保存時点に戻れば未変更");
}

// ---------- eol_stats のプロパティテスト ----------

#[derive(Clone, Debug)]
enum Act {
    Insert { at: u16, s: &'static str },
    Delete { at: u16, len: u8 },
    Replace { at: u16, len: u8, s: &'static str },
    Undo,
    Redo,
}

fn act() -> impl Strategy<Value = Act> {
    let s = proptest::sample::select(vec![
        "a", "\n", "\r", "\r\n", "x\ny", "\r\r\n", "\n\r", "あ\r",
    ]);
    prop_oneof![
        5 => (any::<u16>(), s.clone()).prop_map(|(at, s)| Act::Insert { at, s }),
        4 => (any::<u16>(), 1u8..5).prop_map(|(at, len)| Act::Delete { at, len }),
        2 => (any::<u16>(), 1u8..5, s).prop_map(|(at, len, s)| Act::Replace { at, len, s }),
        2 => Just(Act::Undo),
        1 => Just(Act::Redo),
    ]
}

fn boundary(text: &str, at: u16) -> usize {
    let mut p = (at as usize) % (text.len() + 1);
    while !text.is_char_boundary(p) {
        p -= 1;
    }
    p
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    #[test]
    fn eol_stats_は常に数え直しと一致する(initial in "[ab\r\n]{0,12}", acts in proptest::collection::vec(act(), 0..40)) {
        let mut d = Document::from_text(eve_text::Text::from(initial.as_str()), eve_core::TextFormat::new_document());
        prop_assert_eq!(d.format.eol_stats, d.recount_eol());
        let mut t = 0;
        for a in &acts {
            t += 10;
            let text = d.to_string_lossy();
            match a {
                Act::Insert { at, s } => {
                    let p = boundary(&text, *at);
                    d.apply(&user(edit(&d, p, p, s), t)).unwrap();
                }
                Act::Delete { at, len } | Act::Replace { at, len, .. } => {
                    let st = boundary(&text, *at);
                    let mut en = (st + *len as usize).min(text.len());
                    while !text.is_char_boundary(en) {
                        en += 1;
                    }
                    let ins = if let Act::Replace { s, .. } = a { *s } else { "" };
                    d.apply(&user(edit(&d, st, en, ins), t)).unwrap();
                }
                Act::Undo => { d.undo().unwrap(); }
                Act::Redo => { d.redo().unwrap(); }
            }
            prop_assert_eq!(d.format.eol_stats, d.recount_eol(), "操作 {:?} の後、文書 {:?}", a, d.to_string_lossy());
        }
    }
}
