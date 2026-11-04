//! NFR-203 (v0.2、01 §5.5 の FR-103 / NFR-203 の行): 既定のプロセス全体の上限 (256 MiB) のまま、複数文書の編集の合計が上限を超えると、
//! 履歴が最も大きい文書から古い順に破棄され、通知が出る (docs/03 §7.5)。数百 MB を使うので release でも数秒かかる。

use eve_testkit::Harness;

const MIB: usize = 1024 * 1024;
const TOTAL: usize = 256 * MIB;

fn history_bytes(h: &Harness, doc: eve_view::DocumentId) -> usize {
    h.editor.state().document(doc).unwrap().history().bytes()
}

fn undo_len(h: &Harness, doc: eve_view::DocumentId) -> usize {
    h.editor.state().document(doc).unwrap().history().undo_len()
}

// verifies: NFR-203
#[test]
fn 既定の_256_mib_を超えると履歴の最も大きい文書から古い順に破棄し_通知する() {
    // 文書ごとの上限 (64 MiB) に掛からない大きさで、5 文書の合計が 256 MiB を超えるようにする:
    // 0 番は 31 MiB × 2 (62 MiB)、1〜4 番は 30 MiB × 2 (60 MiB)。合計 302 MiB
    let mut h = Harness::new();
    assert_eq!(
        h.editor.state().config().undo_total_max_bytes as usize,
        TOTAL,
        "既定の上限で確かめる"
    );
    let mut docs = Vec::new();
    for i in 0..5 {
        let p = format!("/w/{i}.txt");
        h.fs.add_file(&p, b"");
        h.open(&p);
        h.run_jobs();
        docs.push(h.doc_id());
        let mib = if i == 0 { 31 } else { 30 };
        let chunk = "x".repeat(mib * MIB);
        if i == 4 {
            // 最後の編集で合計が上限を超える。その前に通知の記録を空にする
            h.paste(&chunk);
            h.advance_ms(2_000);
            h.clear_log();
            h.paste(&chunk);
        } else {
            for _ in 0..2 {
                h.paste(&chunk);
                h.advance_ms(2_000); // 結合しない
            }
        }
        h.advance_ms(2_000);
    }
    let total: usize = docs.iter().map(|d| history_bytes(&h, *d)).sum();
    assert!(total <= TOTAL, "合計 {} MiB は上限以下", total / MIB);
    // 最も大きい 0 番から古い順に破棄され、それでも超える分は次に大きい文書から
    let lens: Vec<usize> = docs.iter().map(|d| undo_len(&h, *d)).collect();
    assert_eq!(lens[0], 1, "最も大きい 0 番の古い 1 回を破棄: {lens:?}");
    assert_eq!(
        lens.iter().filter(|&&n| n == 1).count(),
        2,
        "上限まで減らすのに 2 文書の 1 回ずつ: {lens:?}"
    );
    for d in &docs {
        assert!(
            h.editor.state().document(*d).unwrap().history().can_undo(),
            "どの文書も直前の操作は取り消せる"
        );
    }
    // 最後に編集した 4 番の直前の 1 操作は残る (上限より優先)
    assert!(undo_len(&h, docs[4]) >= 1);
    let notices = h
        .messages()
        .iter()
        .filter(|m| m.contains("history_trimmed"))
        .count();
    assert!(notices >= 1, "通知が出る: {:?}", h.messages());
}
