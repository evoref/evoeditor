//! 長大行 (閾値 1 MiB 超の 1 行) の列の索引 (docs/03 §14.4・§11.5.1、01 NFR-106): 読み取り専用にせず編集でき、
//! 行全体を展開せずに表示・カーソル・移動を行う。v0.4 (M25) から書記素の単位 (AC-5)。長大行を含む文書は折り返さない。

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
fn 長大行を含む文書は編集できる文書として開き_折り返さないことを示す() {
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
        assert_eq!(h.selection_range(), (line.len(), line.len()), "{name}");
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
        assert_eq!(h.selection_range().1, line.len() - last, "{name}");
        h.keys("right right");
        assert_eq!(
            h.selection_range().1,
            line.len() + 1,
            "{name}: 次の行の行頭"
        );
        // 上へ戻ると、同じ表示列 (行頭) へ
        h.keys("up");
        assert_eq!(h.selection_range().1, 0, "{name}");
        h.keys("home");
        assert_eq!(h.selection_range().1, 0, "{name}");
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
        assert_eq!(h.selection_range().1, mid + 3, "{name}");
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
    assert_eq!(h.selection_range().1, 64 * 1024);
    h.keys("end ctrl+left");
    assert_eq!(h.selection_range().1, line.len() - 64 * 1024);
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
    assert_eq!(h.selection_range().1, line.len() + 1);
    assert_eq!(long_lines(&h), 1);
}

fn status_text(h: &mut Harness, kind: StatusKind) -> String {
    h.frame()
        .status
        .items
        .iter()
        .find(|i| i.kind == kind)
        .map(|i| i.long.spans.iter().map(|s| s.text.to_string()).collect())
        .unwrap_or_default()
}

// verifies: AC-NFR-106-5
#[test]
fn 長大行でもカーソルの移動_桁_表示は書記素の単位() {
    // 01 NFR-106 AC-5: 結合文字・ZWJ の列・国旗・ハングルの字母の列の途中にカーソルが止まらず、桁は書記素の数、
    // 表示は 1 書記素を 1 つの単位で描く (v0.2 の char 近似では結合文字が `◌` 付きで分かれて見えた)
    let graphemes = [
        "a",
        "e\u{301}",
        "\u{1F468}\u{200D}\u{1F469}",
        "\u{1F1EF}\u{1F1F5}",
        "\u{1100}\u{1161}\u{11A8}",
        "x",
    ];
    let unit = graphemes.concat();
    let line = unit.repeat(LONG_LINE_THRESHOLD / unit.len() + 2);
    let mut h = opened(&line);
    assert_eq!(long_lines(&h), 1);
    // 行の中ほどの単位の先頭から、右へ 1 書記素ずつ
    let start = unit.len() * (line.len() / unit.len() / 2);
    put_cursor(&mut h, start);
    let col0 = (start / unit.len()) * graphemes.len() + 1;
    assert_eq!(
        status_text(&mut h, StatusKind::Position),
        format!("行 1, 桁 {col0}")
    );
    let mut at = start;
    for (i, g) in graphemes.iter().enumerate() {
        h.keys("right");
        at += g.len();
        assert_eq!(h.selection_range().1, at, "{g:?}");
        assert_eq!(
            status_text(&mut h, StatusKind::Position),
            format!("行 1, 桁 {}", col0 + i + 1),
            "{g:?}"
        );
    }
    // 表示: 結合文字は基底と同じ単位 (`◌` を付けて分けない)。行末の付近を描く
    h.keys("end");
    let row = row_text(&mut h, 0);
    assert!(row.contains("e\u{301}"), "{row:?}");
    assert!(!row.contains('\u{25CC}'), "{row:?}");
    // 行末の桁は行の書記素の数 + 1
    assert_eq!(
        status_text(&mut h, StatusKind::Position),
        format!("行 1, 桁 {}", line.len() / unit.len() * graphemes.len() + 1)
    );
}

// ---------- 本物の経路の乱数の行 (レビューで足した) ----------

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n as u64).unwrap_or(0)
    }
}

fn piece(r: &mut Rng, s: &mut String) {
    const EXTPICT: &[char] = &[
        '\u{1F44D}',
        '\u{A9}',
        '\u{2764}',
        '\u{1F468}',
        '\u{1F469}',
        '\u{1F600}',
    ];
    const EXTEND: &[char] = &[
        '\u{301}',
        '\u{FE0F}',
        '\u{1F3FB}',
        '\u{093C}',
        '\u{3099}',
        '\u{E0067}',
        '\u{20E3}',
    ];
    const LINKER: &[char] = &['\u{094D}', '\u{09CD}'];
    const CONS: &[char] = &['\u{0915}', '\u{0937}', '\u{0995}'];
    const PREPEND: &[char] = &['\u{0600}', '\u{110BD}'];
    const HANGUL: &[char] = &['\u{1100}', '\u{1161}', '\u{11A8}', '\u{AC00}', '\u{D55C}'];
    const RI: &[char] = &['\u{1F1EF}', '\u{1F1F5}'];
    const OTHER: &[char] = &[
        'a',
        'b',
        ' ',
        '\t',
        '\u{1}',
        'あ',
        '漢',
        '\u{10FF80}',
        '\u{3000}',
        '\u{0903}',
    ];
    let pick = |r: &mut Rng, t: &[char]| t[r.below(t.len())];
    match r.below(14) {
        0..=3 => s.push(pick(r, OTHER)),
        4 => s.push(pick(r, EXTPICT)),
        5 => s.push(pick(r, EXTEND)),
        6 => s.push('\u{200D}'),
        7 => s.push(pick(r, LINKER)),
        8 => s.push(pick(r, CONS)),
        9 => s.push(pick(r, PREPEND)),
        10 => s.push(pick(r, HANGUL)),
        11 => s.push(pick(r, RI)),
        12 => {
            s.push(pick(r, EXTPICT));
            for _ in 0..r.below(4) {
                if r.below(2) == 0 {
                    s.push(pick(r, EXTEND));
                }
                s.push('\u{200D}');
                s.push(pick(r, EXTPICT));
            }
        }
        _ => {
            s.push(pick(r, CONS));
            for _ in 0..1 + r.below(3) {
                if r.below(2) == 0 {
                    s.push('\u{093C}');
                }
                s.push(pick(r, LINKER));
                s.push(pick(r, CONS));
            }
        }
    }
    if r.below(5) == 0 {
        for _ in 0..r.below(40) {
            s.push('x');
        }
    }
}

fn opened_with(line: &str, threshold: Option<u64>) -> Harness {
    let mut data = line.as_bytes().to_vec();
    data.extend_from_slice(b"\nsecond\n");
    let mut h = Harness::new()
        .with_file("/w/long.txt", &data)
        .with_size(60, 6);
    if let Some(t) = threshold {
        h.editor.state_mut().set_config(eve_config::Config {
            long_line_threshold: t,
            ..eve_config::Config::default()
        });
    }
    h.open("/w/long.txt");
    h.run_jobs();
    h
}

fn put_cursor_at(h: &mut Harness, at: usize) {
    let st = h.editor.state_mut();
    let w = st.first_window().unwrap();
    let v = st.window_view(w).unwrap();
    st.set_selection(v, Selection::single(Range::point(ByteOff(at))));
}

fn position(f: &eve_view::Frame) -> String {
    f.status
        .items
        .iter()
        .find(|i| i.kind == StatusKind::Position)
        .map(|i| i.long.spans.iter().map(|s| s.text.to_string()).collect())
        .unwrap_or_default()
}

// verifies: AC-NFR-106-5
#[test]
fn 長大行の経路は通常の行と同じ_写す範囲と記録点をまたぐ乱数の行() {
    // 03 §14.4 の「正しさの確かめ」・01 NFR-106 AC-5: 本物の経路 (写す範囲 12 KiB / 64 KiB の端と記録点 4 KiB をまたぐ
    // 140 KiB の行) で、カーソルの位置・桁・カーソルのセル・表示の行が、同じ内容の通常の行と同じ
    let mut r = Rng(0x2545_F491_4F6C_DD1D);
    for round in 0..3 {
        let mut line = String::new();
        while line.len() < 140 * 1024 {
            piece(&mut r, &mut line);
        }
        let mut long = opened_with(&line, Some(65_536));
        let mut normal = opened_with(&line, None);
        let lf = long
            .editor
            .state()
            .document(long.doc_id())
            .unwrap()
            .format
            .long_lines;
        let nf = normal
            .editor
            .state()
            .document(normal.doc_id())
            .unwrap()
            .format
            .long_lines;
        assert_eq!((lf, nf), (1, 0));
        let starts: Vec<usize> = eve_text::grapheme_indices(&line)
            .iter()
            .map(|&(b, _)| b)
            .collect();
        // 書記素の先頭 (ランダム + 写す範囲の端の付近 + 行末) と、char の境界 (書記素の途中を含む)
        let mut probes: Vec<usize> = (0..120).map(|_| starts[r.below(starts.len())]).collect();
        for edge in [
            4096,
            12 * 1024,
            12 * 1024 + 64 * 1024,
            12 * 1024 + 128 * 1024,
        ] {
            let i = starts.partition_point(|&b| b < edge);
            probes.extend(
                starts[i.saturating_sub(3)..(i + 3).min(starts.len())]
                    .iter()
                    .copied(),
            );
        }
        let chars: Vec<usize> = line.char_indices().map(|(b, _)| b).collect();
        probes.extend((0..60).map(|_| chars[r.below(chars.len())]));
        probes.push(line.len());
        for at in probes {
            put_cursor_at(&mut long, at);
            put_cursor_at(&mut normal, at);
            let (fa, fb) = (long.frame(), normal.frame());
            assert_eq!(
                long.selection_range(),
                normal.selection_range(),
                "round {round} at {at}"
            );
            assert_eq!(position(&fa), position(&fb), "round {round} at {at}");
            assert_eq!(
                fa.primary_cursor, fb.primary_cursor,
                "round {round} at {at}"
            );
            assert_eq!(
                fa.panes[0].rows[0], fb.panes[0].rows[0],
                "round {round} at {at}"
            );
        }
        // 右へ・左へ 1 書記素ずつ (写す範囲の端の付近から)
        for start in [12 * 1024 - 40, 12 * 1024 + 64 * 1024 - 40] {
            let i = starts.partition_point(|&b| b < start);
            put_cursor_at(&mut long, starts[i]);
            put_cursor_at(&mut normal, starts[i]);
            for k in 0..60 {
                let key = if k < 40 { "right" } else { "left" };
                long.keys(key);
                normal.keys(key);
                assert_eq!(
                    long.selection_range(),
                    normal.selection_range(),
                    "{start} {k}"
                );
                let (fa, fb) = (long.frame(), normal.frame());
                assert_eq!(position(&fa), position(&fb), "{start} {k}");
                assert_eq!(fa.primary_cursor, fb.primary_cursor, "{start} {k}");
                assert_eq!(fa.panes[0].rows[0], fb.panes[0].rows[0], "{start} {k}");
            }
        }
        // 行末
        long.keys("end");
        normal.keys("end");
        let (fa, fb) = (long.frame(), normal.frame());
        assert_eq!(position(&fa), position(&fb));
        assert_eq!(fa.panes[0].rows[0], fb.panes[0].rows[0]);
    }
}

/// 既知の制約 (03 §14.4 の「1 書記素の上限」、01 NFR-106 AC-5): 4 KiB を超える 1 書記素は長大行では 4 KiB で切るので、
/// その後ろの桁が通常の行より 1 つ多くなる。
// verifies: AC-NFR-106-5
#[test]
fn 上限を超える書記素の後ろでは桁が通常の行と食い違う_既知の制約() {
    let mut line = "a".repeat(70_000);
    line.push('e');
    line.push_str(&"\u{301}".repeat(3000));
    line.push('z');
    let mut long = opened_with(&line, Some(65_536));
    let mut normal = opened_with(&line, None);
    let at = line.len() - 1; // z の前
    put_cursor_at(&mut long, at);
    put_cursor_at(&mut normal, at);
    assert_eq!(position(&long.frame()), "行 1, 桁 70003");
    assert_eq!(position(&normal.frame()), "行 1, 桁 70002");
}
