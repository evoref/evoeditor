//! 再読込の確保の量 (SAF-33、03 §9.5 / §7.5): 新全文・旧全文を複写しない。
//!
//! 確保を数える allocator は dev 依存の `stats_alloc` (`bm_allocs` と同じ)。このバイナリの試験は 1 つだけにする
//! (数える範囲に他の試験のスレッドの確保が混ざらないように)。

use std::alloc::System;

use eve_testkit::Harness;
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const MIB: usize = 1 << 20;

fn body(fill: char) -> Vec<u8> {
    let line: String = std::iter::repeat_n(fill, 63).chain(['\n']).collect();
    line.repeat(4 * MIB / line.len()).into_bytes()
}

/// (確保したバイト数, 残ったバイト数)。
fn bytes(st: &Stats) -> (usize, isize) {
    let allocated =
        st.bytes_allocated + usize::try_from(st.bytes_reallocated.max(0)).expect("max(0) で非負");
    let retained = isize::try_from(st.bytes_allocated).expect("確保量は isize に収まる")
        - isize::try_from(st.bytes_deallocated).expect("解放量は isize に収まる")
        + st.bytes_reallocated;
    (allocated, retained)
}

// verifies: NFR-203
// spec: 03 §9.5 (再読込の全置換は旧全文・新全文を複写しない)
#[test]
fn 再読込は本文を複写しない() {
    let old = body('a');
    let new = body('b');
    let mut h = Harness::new().with_size(80, 24).with_file("/w/a.txt", &old);
    let region = Region::new(GLOBAL);
    h.open("/w/a.txt");
    h.run_jobs();
    let (open_alloc, _) = bytes(&region.change());
    h.fs.set_file(&eve_core::FsPath::from("/w/a.txt"), &new);
    let region = Region::new(GLOBAL);
    h.cmd("file.reload");
    h.run_jobs();
    let (alloc, retained) = bytes(&region.change());
    assert_eq!(h.doc_text().len(), new.len());
    // 確保は開くのと同じ程度 (読込の rope・偽 FS の複製・確保の見込みの検査)。全文の複写があると 2 倍を超える
    // (修正前は 8 MiB の再読込で 117 MiB を確保し、47 MiB が残った)
    assert!(
        alloc < open_alloc * 5 / 4,
        "再読込 {alloc} / 開く {open_alloc}"
    );
    // 残るのは新本文の rope だけ (旧本文は Undo の履歴が旧 rope のまま持つ)
    assert!(
        retained < isize::try_from(new.len()).expect("本文は isize に収まる") * 3 / 2,
        "残った確保 {retained}"
    );
    // Undo で旧本文に、Redo で新本文に戻る
    h.cmd("edit.undo");
    assert!(h.doc_text().as_bytes() == old.as_slice());
    h.cmd("edit.redo");
    assert!(h.doc_text().as_bytes() == new.as_slice());
}
