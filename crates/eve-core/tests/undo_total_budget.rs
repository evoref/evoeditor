//! Undo 履歴のプロセス全体の上限 (01 NFR-203、docs/03 §7.5): 合計が上限を超えたら、履歴が最も大きい文書から
//! 古い順に破棄する。直前の 1 操作は上限より優先し、破棄しても dirty 表示は変わらない。

use eve_core::{Document, Origin, TextFormat, Tick, Transaction, trim_to_total_budget};
use eve_testkit::Harness;
use eve_text::{ByteRange, ChangeSet, Edit, Text};

/// 文書の末尾に `bytes` バイトの文字を足す編集を `n` 回行う (結合しないよう 2 秒ずつ離す)。
fn grow(d: &mut Document, n: usize, bytes: usize) {
    for i in 0..n {
        let len = d.len_bytes();
        let cs = ChangeSet::from_sorted_edits(
            &d.snapshot(),
            [Edit {
                range: ByteRange::from(len..len),
                insert: "x".repeat(bytes).as_str().into(),
            }],
        )
        .unwrap();
        d.apply(&Transaction::new(cs, Origin::User, Tick(i as u64 * 2000)))
            .unwrap();
    }
}

fn doc() -> Document {
    Document::from_text(Text::from(""), TextFormat::new_document())
}

#[test]
fn 上限を超えたら履歴が最も大きい文書から古い順に破棄する() {
    let (mut small, mut big, mut mid) = (doc(), doc(), doc());
    grow(&mut small, 5, 1_000);
    grow(&mut big, 5, 100_000);
    grow(&mut mid, 5, 10_000);
    let before = (
        small.history().bytes(),
        big.history().bytes(),
        mid.history().bytes(),
    );
    assert!(before.1 > before.2 && before.2 > before.0, "{before:?}");
    // 合計から、最も大きい文書の 1 ノード分だけ超えている上限
    let total = before.0 + before.1 + before.2;
    let limit = total - 50_000;
    let mut docs = [("small", &mut small), ("big", &mut big), ("mid", &mut mid)];
    let trimmed = trim_to_total_budget(&mut docs, limit);
    assert_eq!(trimmed, vec!["big"], "最も大きい文書だけが削られる");
    let after_total: usize = docs.iter().map(|(_, d)| d.history().bytes()).sum();
    assert!(after_total <= limit, "{after_total} <= {limit}");
    assert_eq!(docs[0].1.history().undo_len(), 5, "小さい文書は削らない");
    assert!(docs[1].1.history().undo_len() < 5);
    assert_eq!(docs[2].1.history().undo_len(), 5);
}

#[test]
fn 直前の_1_操作は上限より優先し_1_ノードしかない文書は削らない() {
    let (mut a, mut b) = (doc(), doc());
    grow(&mut a, 1, 500_000);
    grow(&mut b, 3, 100_000);
    let mut docs = [("a", &mut a), ("b", &mut b)];
    // 上限 1 B: 削れるだけ削る。a は 1 ノードなので残り、b も最低 1 ノードは残る
    let trimmed = trim_to_total_budget(&mut docs, 1);
    assert_eq!(trimmed, vec!["b"]);
    assert_eq!(docs[0].1.history().undo_len(), 1);
    assert_eq!(docs[1].1.history().undo_len(), 1);
    // 無限ループにならず、さらに呼んでも何も削らない
    assert!(trim_to_total_budget(&mut docs, 1).is_empty());
}

#[test]
fn 破棄しても_dirty_は変わらず_保存点を失う() {
    let mut a = doc();
    grow(&mut a, 4, 100_000);
    assert!(a.history().is_dirty());
    let mut docs = [("a", &mut a)];
    let trimmed = trim_to_total_budget(&mut docs, 1);
    assert_eq!(trimmed, vec!["a"]);
    assert!(
        docs[0].1.history().is_dirty(),
        "履歴を削っても未保存の変更は dirty のまま (FR-204 AC-4)"
    );
    assert!(docs[0].1.history().can_undo(), "最後の 1 ノードは残る");
}

#[test]
fn 上限以下なら何もしない() {
    let mut a = doc();
    grow(&mut a, 3, 1_000);
    let bytes = a.history().bytes();
    let mut docs = [("a", &mut a)];
    assert!(trim_to_total_budget(&mut docs, bytes).is_empty());
    assert_eq!(docs[0].1.history().undo_len(), 3);
}

// Editor の編集の後に上限を守り、破棄を 1 回だけ通知する。
#[test]
fn editor_は編集の後にプロセス全体の上限を守り_通知は_1_回() {
    let mut h = Harness::new();
    let mut config = *h.editor.state().config();
    config.undo_total_max_bytes = 1_048_576; // 最小の 1 MiB
    h.editor.state_mut().set_config(config);
    h.clear_log();
    let chunk = "y".repeat(400_000);
    for _ in 0..6 {
        h.paste(&chunk);
        h.advance_ms(2_000); // 結合しない
    }
    let doc = h.doc_id();
    let history = h.editor.state().document(doc).unwrap().history();
    assert!(
        history.bytes() <= 1_048_576 + 400_000,
        "上限を超えた分は古い履歴から破棄される: {}",
        history.bytes()
    );
    assert!(history.undo_len() < 6, "{}", history.undo_len());
    assert!(history.can_undo(), "直前の操作は取り消せる");
    let notices = h
        .messages()
        .iter()
        .filter(|m| m.contains("history_trimmed"))
        .count();
    assert_eq!(notices, 1, "通知は 1 回だけ: {:?}", h.messages());
}
