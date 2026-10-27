//! Undo 履歴のメモリ (01 FR-203 AC-3 / NFR-203 AC-2、docs/09 §3.2)。
//! ランダムな編集 (64 KiB 以上の削除を一定割合で含む) の後、プロセスのメモリ (Windows の Private Bytes) の増分が
//! Undo の会計値の 1.2 倍以下であること。Private Bytes は Windows だけで読める (他の OS では会計値の整合だけを検査する)。

use std::time::Duration;

use eve_core::{Document, Origin, TextFormat, Tick, Transaction};
use eve_text::{ByteRange, ChangeSet, Edit, Text};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// プロセスの Private Bytes (Windows: PagefileUsage)。
fn private_bytes() -> usize {
    memory_stats::memory_stats().map_or(0, |m| m.virtual_mem)
}

// verifies: AC-FR-203-3 AC-NFR-203-2
#[test]
fn ランダムな編集の後_メモリの増分は_undo_の会計値の_1_2_倍以下() {
    // 40 MB の ASCII の文書 (行は 80 B)
    let line = "0123456789 the quick brown fox jumps over the lazy dog 0123456789 abcdefgh\n";
    let text: String = line.repeat(40 * 1024 * 1024 / line.len());
    let mut d = Document::from_text(Text::from(text.as_str()), TextFormat::new_document());
    drop(text);

    std::thread::sleep(Duration::from_secs(1)); // 読込完了から 1 秒後 (NFR-202 と同じ基準点)
    let before = private_bytes();

    let mut r = Rng(42);
    let edits = 9_000; // Undo の上限 (10,000 ステップ) による破棄が起きない範囲
    let mut big_deletes = 0;
    for i in 0..edits {
        let len = d.len_bytes();
        let (start, end, ins) = if i % 20 == 0 && len > 400 * 1024 {
            // 64 KiB 以上の削除 (5%)
            big_deletes += 1;
            let n = 64 * 1024 + r.below(32 * 1024);
            let s = r.below(len - n);
            (s, s + n, String::new())
        } else if r.below(2) == 0 {
            let s = r.below(len);
            (s, s, "x".repeat(1 + r.below(40)))
        } else {
            let n = 1 + r.below(40);
            let s = r.below(len - n);
            (s, s + n, String::new())
        };
        let cs = ChangeSet::from_sorted_edits(
            &d.snapshot(),
            [Edit {
                range: ByteRange::from(start..end),
                insert: ins.as_str().into(),
            }],
        )
        .unwrap();
        // 結合しないよう、毎回 1 秒以上離す
        d.apply(&Transaction::new(cs, Origin::User, Tick(i as u64 * 2000)))
            .unwrap();
    }
    assert!(big_deletes >= 400, "{big_deletes}");
    let accounted = d.history().bytes();
    assert!(d.history().undo_len() >= edits - 1, "破棄が起きていない");

    std::thread::sleep(Duration::from_secs(1));
    let after = private_bytes();
    let grown = after.saturating_sub(before);
    if cfg!(windows) {
        assert!(
            (grown as f64) <= 1.2 * accounted as f64,
            "増分 {grown} B が会計値 {accounted} B の 1.2 倍を超えた"
        );
    }
    // どの OS でも: 会計値は 64 KiB 以上の削除の分を含む
    assert!(accounted >= big_deletes * 64 * 1024);
}
