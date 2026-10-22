//! 長大行 (閾値 1 MiB 超の 1 行) の縮退 (docs/03 §11.5、01 NFR-106 AC-3): 読み取り専用で開き、先頭 64 KiB だけを表示する。

use eve_core::{LONG_LINE_VISIBLE, ReadOnlyReason};
use eve_testkit::Harness;
use eve_text::TextRead as _;
use eve_view::StatusKind;

fn opened(line: &str) -> Harness {
    let mut data = line.as_bytes().to_vec();
    data.extend_from_slice(b"\nsecond\n");
    let mut h = Harness::new()
        .with_file("/w/long.txt", &data)
        .with_size(100, 10);
    h.open("/w/long.txt");
    h.run_jobs();
    h
}

fn sel(h: &Harness) -> (usize, usize) {
    let st = h.editor.state();
    let w = st.first_window().unwrap();
    let r = *st
        .view(st.window_view(w).unwrap())
        .unwrap()
        .selection
        .primary();
    (r.start().0, r.end().0)
}

/// 3 種: ASCII / 日本語 / 絵文字混在 (NFR-106 AC-3)。
fn lines() -> Vec<(&'static str, String)> {
    let n = (1 << 20) + 4096;
    vec![
        ("ASCII", "abcdefghij".repeat(n / 10 + 1)),
        ("日本語", "日本語のテキスト".repeat(n / 24 + 1)),
        ("絵文字混在", "a日😀b".repeat(n / 9 + 1)),
    ]
}

// verifies: AC-NFR-106-3
#[test]
fn 長大行を含む文書は読み取り専用の理由つきで開き_先頭だけを表示する() {
    for (name, line) in lines() {
        let mut h = opened(&line);
        let id = h.doc_id();
        let doc = h.editor.state().document(id).unwrap();
        assert!(doc.read_only.contains(ReadOnlyReason::LongLine), "{name}");
        assert!(!doc.is_editable(), "{name}");
        let f = h.frame();
        // ステータスバーに理由が出る
        let st: Vec<_> = f.status.items.iter().map(|i| i.kind).collect();
        assert!(
            st.contains(&StatusKind::ReadOnly) && st.contains(&StatusKind::Degraded),
            "{name}"
        );
        // 1 行目は画面の幅に収まって描かれる
        let w: u16 = f.panes[0].rows[0].spans.iter().map(|s| s.width).sum();
        assert!(w <= 100, "{name}");
        // 編集操作は拒否される
        h.text("x");
        let doc = h.editor.state().document(id).unwrap();
        assert_eq!(doc.text().len_bytes(), line.len() + 8, "{name}");
    }
}

// verifies: AC-NFR-106-3
#[test]
fn end_は先頭_64_kib_の書記素の境界で止まり_その先の印を出す() {
    for (name, line) in lines() {
        let mut h = opened(&line);
        h.keys("end");
        let (_, e) = sel(&h);
        assert!(e <= LONG_LINE_VISIBLE, "{name}: {e}");
        assert!(
            e > LONG_LINE_VISIBLE - 8,
            "{name}: 64 KiB の直前で止まる ({e})"
        );
        let id = h.doc_id();
        assert!(
            h.editor
                .state()
                .document(id)
                .unwrap()
                .text()
                .is_char_boundary(eve_text::ByteOff(e))
        );
        // 行末の印 (省略した分を示す)
        let f = h.frame();
        let row: String = f.panes[0].rows[0]
            .spans
            .iter()
            .map(|s| s.text.to_string())
            .collect();
        assert!(row.contains('…') || row.contains("省略"), "{name}: {row:?}");
        // 右へ進んでも表示範囲を越えない
        h.keys("right right");
        assert!(sel(&h).1 <= LONG_LINE_VISIBLE, "{name}");
        // 次の行へは動ける
        h.keys("down");
        assert!(sel(&h).1 > line.len(), "{name}: 次の行の中");
    }
}

// verifies: AC-NFR-106-3
#[test]
fn 閾値以下の長い行は読み取り専用にならない() {
    let line = "x".repeat((1 << 20) - 10);
    let h = opened(&line);
    let id = h.doc_id();
    assert!(
        !h.editor
            .state()
            .document(id)
            .unwrap()
            .read_only
            .contains(ReadOnlyReason::LongLine)
    );
}
