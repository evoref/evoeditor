//! 長大行 (閾値 1 MiB 超の 1 行) の char 近似モード (docs/03 §11.5.1、01 NFR-106 の v0.2): 読み取り専用にせず編集でき、
//! 行全体を展開せずに表示・カーソル・移動を行う。長大行を含む文書は折り返さない。

use eve_core::LONG_LINE_THRESHOLD;
use eve_testkit::Harness;
use eve_text::{ByteOff, Range, Selection, TextRead as _};
use eve_view::{RowSource, StatusKind};

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

fn put_cursor(h: &mut Harness, at: usize) {
    let st = h.editor.state_mut();
    let w = st.first_window().unwrap();
    let v = st.window_view(w).unwrap();
    st.set_selection(v, Selection::single(Range::point(ByteOff(at))));
}

fn long_lines(h: &Harness) -> usize {
    h.editor
        .state()
        .document(h.doc_id())
        .unwrap()
        .format
        .long_lines
}

fn row_text(h: &mut Harness, r: usize) -> String {
    h.frame().panes[0].rows[r]
        .spans
        .iter()
        .map(|s| s.text.to_string())
        .collect()
}

/// 3 種: ASCII / 日本語 / 絵文字混在 (NFR-106 AC-1)。
fn lines() -> Vec<(&'static str, String)> {
    let n = LONG_LINE_THRESHOLD + 4096;
    vec![
        ("ASCII", "abcdefghij".repeat(n / 10 + 1)),
        ("日本語", "日本語のテキスト".repeat(n / 24 + 1)),
        ("絵文字混在", "a日😀b".repeat(n / 9 + 1)),
    ]
}

// verifies: NFR-106
#[test]
fn 長大行を含む文書は編集できる文書として開き_近似の表示であることを示す() {
    for (name, line) in lines() {
        let mut h = opened(&line);
        let doc = h.editor.state().document(h.doc_id()).unwrap();
        assert!(doc.is_editable(), "{name}");
        assert_eq!(doc.format.long_lines, 1, "{name}");
        let f = h.frame();
        let st: Vec<_> = f.status.items.iter().map(|i| i.kind).collect();
        assert!(st.contains(&StatusKind::Degraded), "{name}");
        assert!(!st.contains(&StatusKind::ReadOnly), "{name}");
        // 1 行目は画面の幅に収まり、右が切れていることを示す
        let row = &f.panes[0].rows[0];
        let w: u16 = row.spans.iter().map(|s| s.width).sum();
        assert!(w <= 100 - f.panes[0].gutter_width, "{name}: {w}");
        assert!(row.clipped_right, "{name}");
        assert!(h.messages().contains(&"info.open.long_line".to_owned()));
    }
}

// verifies: NFR-106
#[test]
fn end_は行末まで動き_行末の付近を表示する() {
    for (name, line) in lines() {
        let mut h = opened(&line);
        h.keys("end");
        assert_eq!(sel(&h), (line.len(), line.len()), "{name}");
        // 行末が見えていて、カーソルが画面にある
        let f = h.frame();
        assert!(f.primary_cursor.is_some(), "{name}");
        let tail: String = line
            .chars()
            .rev()
            .take(3)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        assert!(row_text(&mut h, 0).contains(&tail), "{name}");
        // 左へ 1 書記素、右で行末へ戻り、さらに右で次の行へ
        h.keys("left");
        let last = line.chars().last().unwrap().len_utf8();
        assert_eq!(sel(&h).1, line.len() - last, "{name}");
        h.keys("right right");
        assert_eq!(sel(&h).1, line.len() + 1, "{name}: 次の行の行頭");
        // 上へ戻ると、同じ表示列 (行頭) へ
        h.keys("up");
        assert_eq!(sel(&h).1, 0, "{name}");
        h.keys("home");
        assert_eq!(sel(&h).1, 0, "{name}");
    }
}

// verifies: NFR-106
#[test]
fn 長大行の中央に挿入と削除ができる() {
    for (name, line) in lines() {
        let mut h = opened(&line);
        let mid = (0..=line.len() / 2)
            .rev()
            .find(|&i| line.is_char_boundary(i))
            .unwrap();
        put_cursor(&mut h, mid);
        h.text("XYZ");
        let id = h.doc_id();
        let t = h
            .editor
            .state()
            .document(id)
            .unwrap()
            .text()
            .slice_to_string(eve_text::ByteRange::new(ByteOff(mid - 1), ByteOff(mid + 4)));
        assert!(t.contains("XYZ"), "{name}: {t:?}");
        assert_eq!(sel(&h).1, mid + 3, "{name}");
        // 挿入した位置が画面に入っている
        h.frame();
        assert!(row_text(&mut h, 0).contains("XYZ"), "{name}");
        h.keys("backspace backspace backspace");
        let doc = h.editor.state().document(id).unwrap();
        assert_eq!(doc.text().len_bytes(), line.len() + 8, "{name}");
        assert_eq!(doc.format.long_lines, 1, "{name}");
    }
}

// verifies: NFR-106
#[test]
fn 長大行の単語移動は窓の端で止まる() {
    // 区切りの無い ASCII の行: 1 回の Ctrl+→ で行全体を読まずに窓 (64 KiB) の端に止まる
    let line = "a".repeat(LONG_LINE_THRESHOLD + 10);
    let mut h = opened(&line);
    h.keys("ctrl+right");
    assert_eq!(sel(&h).1, 64 * 1024);
    h.keys("end ctrl+left");
    assert_eq!(sel(&h).1, line.len() - 64 * 1024);
}

// verifies: NFR-106
#[test]
fn 長大行の数は編集で増減し_長大行を含む間は折り返さない() {
    let line = "x".repeat(LONG_LINE_THRESHOLD + 10);
    let mut h = opened(&line);
    h.cmd("view.toggle_wrap");
    assert_eq!(long_lines(&h), 1);
    // 折り返さない: 2 行目は文書の 2 行目
    let f = h.frame();
    assert!(matches!(
        f.panes[0].rows[1].source,
        RowSource::Line {
            line: 1,
            wrap_idx: 0
        }
    ));
    // 行の中央で改行すると、どちらも閾値以下になる
    put_cursor(&mut h, line.len() / 2);
    h.keys("enter");
    assert_eq!(long_lines(&h), 0);
    // 長大行が無くなれば折り返しに戻る (`view.wrap` の値は変わっていない)
    let f = h.frame();
    assert!(
        f.panes[0]
            .rows
            .iter()
            .any(|r| matches!(r.source, RowSource::Line { wrap_idx, .. } if wrap_idx > 0))
    );
    h.keys("ctrl+z");
    assert_eq!(long_lines(&h), 1);
    // 差分で更新した数は、文書から数え直した数と同じ
    let doc = h.editor.state().document(h.doc_id()).unwrap();
    assert_eq!(
        eve_core::count_long_lines(doc.text(), doc.format.long_line_threshold),
        1
    );
}

// verifies: NFR-106
#[test]
fn 閾値以下の長い行は通常の行として扱う() {
    let line = "x".repeat(LONG_LINE_THRESHOLD - 10);
    let h = opened(&line);
    assert_eq!(long_lines(&h), 0);
}

// verifies: NFR-106
#[test]
fn 閾値は_file_long_line_threshold_で変えられ_開いた文書は開いたときの閾値を保つ() {
    let line = "x".repeat(100_000);
    let mut data = line.as_bytes().to_vec();
    data.extend_from_slice(b"\nsecond\n");
    let mut h = Harness::new()
        .with_file("/w/long.txt", &data)
        .with_size(100, 10);
    h.editor.state_mut().set_config(eve_config::Config {
        long_line_threshold: 65_536,
        ..eve_config::Config::default()
    });
    h.open("/w/long.txt");
    h.run_jobs();
    assert_eq!(long_lines(&h), 1, "設定した閾値 (64 KiB) を超える行");
    // 設定を戻しても、開いている文書は開いたときの閾値のまま (07 §6 の「新規文書から」)
    h.editor
        .state_mut()
        .set_config(eve_config::Config::default());
    h.keys("end");
    h.text("x");
    assert_eq!(sel(&h).1, line.len() + 1);
    assert_eq!(long_lines(&h), 1);
}
