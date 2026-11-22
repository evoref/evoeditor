//! タブラインの並べ方の性質 (03 §4.7.1 の L0、03 §9.9 の PANE-12 (e) / (g) / (h))。Harness の文書に頼らずに `TabLineModel` を
//! 無作為に作り、(1) eve-tui が描いた最上行 (ASCII の縮退) と、ヘッドレスの ASCII の Frame の最上行
//! (`eve_testkit::tab_line_text`) が一致し、(2) 並べたタブの矩形が行の幅に収まる (右を省いたときは最後の列の `>` に掛からない)
//! ことを確かめる。名前には全角と 32 セルを超えるもの、印には `[+]` と短い形の `RO`、アクティブには `None` を含める。

use eve_testkit::Harness;
use eve_tui::buffer::row_text;
use eve_tui::chrome::render_screen;
use eve_tui::chrome::tab_line::layout;
use eve_tui::glyphs::Glyphs;
use eve_view::{
    CellSize, CellText, ColorDepth, CutPolicy, DocumentId, Span, StatusAlign, StatusItem,
    StatusKind, Style, TabItem, TabLineModel,
};
use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};

/// 名前の 1 文字 (ASCII・全角・記号)。
fn name_char() -> impl Strategy<Value = char> {
    prop_oneof![
        prop::char::range('a', 'z'),
        Just('.'),
        Just('-'),
        Just('あ'),
        Just('漢'),
        Just('Ａ'),
    ]
}

fn name_item(name: &str) -> StatusItem {
    let spans: Vec<Span> = name
        .chars()
        .map(|c| {
            Span::new(
                c.to_string(),
                if c.is_ascii() { 1 } else { 2 },
                Style::DEFAULT,
            )
        })
        .collect();
    let width = spans.iter().map(|s| s.width).sum();
    StatusItem {
        kind: StatusKind::Path,
        long: CellText {
            spans,
            width,
            cut: CutPolicy::Middle,
        },
        short: None,
        priority: 4,
        align: StatusAlign::Left,
    }
}

fn plain(text: &str) -> CellText {
    let spans: Vec<Span> = text
        .chars()
        .map(|c| Span::new(c.to_string(), 1, Style::DEFAULT))
        .collect();
    CellText {
        width: spans.iter().map(|s| s.width).sum(),
        spans,
        cut: CutPolicy::Never,
    }
}

fn mark(kind: StatusKind, long: &str, short: Option<&str>) -> StatusItem {
    StatusItem {
        kind,
        long: plain(long),
        short: short.map(plain),
        priority: 5,
        align: StatusAlign::Left,
    }
}

/// タブ 1 つ: 名前 (1〜40 文字。全角を含めると 32 セルを超える)・変更あり・読み取り専用。
fn tab() -> impl Strategy<Value = (String, bool, bool)> {
    (
        prop::collection::vec(name_char(), 1..40).prop_map(|v| v.into_iter().collect()),
        any::<bool>(),
        any::<bool>(),
    )
}

fn model() -> impl Strategy<Value = TabLineModel> {
    prop::collection::vec(tab(), 1..9).prop_flat_map(|tabs| {
        let n = tabs.len();
        (Just(tabs), prop::option::weighted(0.85, 0..n)).prop_map(|(tabs, active)| TabLineModel {
            tabs: tabs
                .into_iter()
                .enumerate()
                .map(|(i, (name, dirty, ro))| {
                    let mut items = vec![name_item(&name)];
                    if dirty {
                        items.push(mark(StatusKind::Dirty, "[+]", None));
                    }
                    if ro {
                        items.push(mark(StatusKind::ReadOnly, "[RO]", Some("RO")));
                    }
                    TabItem {
                        doc: DocumentId(u64::try_from(i).expect("小さい") + 1),
                        items,
                    }
                })
                .collect(),
            active,
        })
    })
}

// verifies: AC-FR-510-1
#[test]
fn 無作為のタブの並びは_tui_とヘッドレスで同じで_矩形は行の幅に収まる() {
    // 描く土台の Frame (本文は使わない。最上行にタブラインを足して描く)
    let mut h = Harness::new().with_size(160, 6);
    let base = h.frame();
    assert!(base.tab_line.is_none());
    let glyphs = Glyphs::ascii();
    let mut runner = TestRunner::new(Config::with_cases(512));
    runner
        .run(&(model(), 20u16..160), |(m, cols)| {
            // (1) 描いた最上行とヘッドレスの最上行
            let mut f = base.clone();
            f.tab_line = Some(m.clone());
            let rows = f.size.rows + 1;
            let window = CellSize {
                cols,
                rows: rows + eve_tui::buffer::chrome_rows(rows, f.hint_bar),
            };
            let r = render_screen(&f, window, ColorDepth::Mono, &glyphs);
            let got = row_text(&r.buffer, 0).trim_end().to_owned();
            let want = eve_testkit::tab_line_text(&m, cols);
            prop_assert_eq!(&got, &want, "{} 桁、アクティブ {:?}", cols, m.active);
            // (2) タブの矩形は行の幅の中で、右を省いたときは最後の列 (`>`) に掛からず、左を省いたときは 0 列目に掛からない
            let lay = layout(&m, cols, &glyphs);
            let right = if lay.more_right { cols - 1 } else { cols };
            let left = u16::from(lay.more_left);
            prop_assert!(!lay.tabs.is_empty());
            for t in &lay.tabs {
                prop_assert!(t.rect.col >= left, "{:?}", t.rect);
                prop_assert!(t.rect.col + t.rect.cols <= right, "{:?} / {}", t.rect, cols);
            }
            // アクティブがあれば並べた範囲に入る
            if let Some(a) = m.active {
                let doc = m.tabs[a].doc;
                prop_assert!(lay.tabs.iter().any(|t| t.doc == doc && t.active));
            }
            Ok(())
        })
        .expect("性質");
}
