//! `Frame` → `Buffer` の変換 (docs/04 §4.2、L4 の TestBackend 相当の検査)。`Harness` (eve-testkit) で Frame を作る。

use eve_testkit::Harness;
use eve_tui::buffer::{chrome_rows_for_surface, render_panes, row_text};
use eve_tui::glyphs::Glyphs;
use eve_view::{Color, ColorDepth, Frame, Style};
use ratatui::buffer::CellDiffOption;

fn frame_of(h: &mut Harness) -> Frame {
    h.frame()
}

#[test]
fn バッファはクロームの行ぶん大きい() {
    let mut h = Harness::new().with_size(40, 10);
    let f = frame_of(&mut h);
    let r = render_panes(&f, ColorDepth::TrueColor, &Glyphs::unicode());
    assert_eq!(r.buffer.area.width, f.size.cols);
    assert_eq!(
        r.buffer.area.height,
        f.size.rows + chrome_rows_for_surface(f.size.rows)
    );
}

#[test]
fn ascii_の行とガターと_eof_の印() {
    let mut h = Harness::new().with_size(30, 8);
    h.text("hello");
    let f = frame_of(&mut h);
    let r = render_panes(&f, ColorDepth::TrueColor, &Glyphs::unicode());
    let gw = f.panes[0].gutter_width as usize;
    assert!(gw >= 2);
    let line = row_text(&r.buffer, 0);
    // ガター: 右寄せの行番号 + 区切り (最後のセル)
    assert!(line.starts_with(" 1│hello"), "{line:?}");
    // 文書末尾より後の行は `~`
    assert!(row_text(&r.buffer, 1).starts_with('~'));
    // ASCII に縮退すると区切りは `|`
    let a = render_panes(&f, ColorDepth::TrueColor, &Glyphs::ascii());
    assert!(row_text(&a.buffer, 0).starts_with(" 1|hello"));
}

#[test]
fn 全角は先頭セルに書記素_後続は継続セル() {
    let mut h = Harness::new().with_size(30, 6);
    h.text("あい");
    let f = frame_of(&mut h);
    let r = render_panes(&f, ColorDepth::TrueColor, &Glyphs::unicode());
    let gw = f.panes[0].gutter_width;
    let x0 = gw;
    assert_eq!(r.buffer.cell((x0, 0)).unwrap().symbol(), "あ");
    assert_eq!(
        r.buffer.cell((x0 + 1, 0)).unwrap().diff_option,
        CellDiffOption::Skip
    );
    assert_eq!(r.buffer.cell((x0 + 2, 0)).unwrap().symbol(), "い");
    assert_eq!(
        r.buffer.cell((x0 + 3, 0)).unwrap().diff_option,
        CellDiffOption::Skip
    );
    assert_eq!(
        r.buffer.cell((x0, 0)).unwrap().diff_option,
        CellDiffOption::None
    );
    // 継続セルを除いた行の文字列は、文字が詰まる
    assert!(row_text(&r.buffer, 0).contains("あい"));
}

#[test]
fn カーソルの位置は_frame_のまま() {
    let mut h = Harness::new().with_size(30, 6);
    h.text("あb");
    let f = frame_of(&mut h);
    let r = render_panes(&f, ColorDepth::TrueColor, &Glyphs::unicode());
    assert_eq!(r.cursor, f.primary_cursor.map(|c| c.pos));
    assert!(r.cursor.is_some());
}

#[test]
fn 幅危険書記素を含む行は記録される() {
    let mut h = Harness::new().with_size(30, 6);
    h.text("a\n\u{1F1EF}\u{1F1F5}\nｶﾞ\nplain");
    let f = frame_of(&mut h);
    let r = render_panes(&f, ColorDepth::TrueColor, &Glyphs::unicode());
    assert_eq!(r.risky_rows, vec![1, 2], "国旗と半角カナの濁点の行");
}

// verifies: AC-FR-507-1
#[test]
fn 段を超える色は近似され_制御文字は置換される() {
    let mut h = Harness::new().with_size(30, 6);
    h.text("ab");
    let mut f = frame_of(&mut h);
    let styled = Style {
        fg: Color::Rgb(255, 0, 0),
        bg: Color::Rgb(0, 0, 255),
        ..Style::DEFAULT
    };
    f.panes[0].rows[0].spans = vec![
        eve_view::Span::new("a", 1, styled),
        eve_view::Span::new("\u{1B}", 1, Style::DEFAULT),
    ];
    let r = render_panes(&f, ColorDepth::Ansi256, &Glyphs::unicode());
    assert!(r.clamped_cells >= 1);
    assert_eq!(r.sanitized, 1);
    let x = f.panes[0].gutter_width;
    assert_eq!(r.buffer.cell((x + 1, 0)).unwrap().symbol(), "\u{FFFD}");
    let c = r.buffer.cell((x, 0)).unwrap();
    assert!(matches!(c.fg, ratatui::style::Color::Indexed(_)));
    // TrueColor ならそのまま
    let t = render_panes(&f, ColorDepth::TrueColor, &Glyphs::unicode());
    assert_eq!(t.clamped_cells, 0);
    assert_eq!(
        t.buffer.cell((x, 0)).unwrap().fg,
        ratatui::style::Color::Rgb(255, 0, 0)
    );
}

#[test]
fn 選択は反転で出る() {
    // 無色の段では選択は反転 (色の段は背景色、07 §8.2)
    let mut h = Harness::new()
        .with_size(30, 6)
        .with_caps(eve_view::Capabilities {
            color_depth: ColorDepth::Mono,
            ..eve_view::Capabilities::default()
        });
    h.text("abc");
    h.keys("shift+left");
    let f = frame_of(&mut h);
    let r = render_panes(&f, ColorDepth::Mono, &Glyphs::unicode());
    let x = f.panes[0].gutter_width + 2;
    let c = r.buffer.cell((x, 0)).unwrap();
    assert!(
        c.modifier.contains(ratatui::style::Modifier::REVERSED),
        "{c:?}"
    );
}

#[test]
fn 全角が矩形の端をまたぐときは空白で埋める() {
    let mut h = Harness::new().with_size(30, 6);
    h.text("ab");
    let mut f = frame_of(&mut h);
    // 幅 2 の書記素を、残り 1 セルの位置へ置く (core は発生させない契約だが、防御する)
    let rect = f.panes[0].rect;
    let last = rect.col + rect.cols - 1;
    let row = &mut f.panes[0].rows[0];
    row.gutter.clear();
    row.spans = vec![
        eve_view::Span::new(
            " ".repeat(rect.cols as usize - 1),
            rect.cols - 1,
            Style::DEFAULT,
        ),
        eve_view::Span::new("あ", 2, Style::DEFAULT),
    ];
    let r = render_panes(&f, ColorDepth::TrueColor, &Glyphs::unicode());
    assert_eq!(r.buffer.cell((last, 0)).unwrap().symbol(), " ");
}

#[test]
fn 背景を塗るテーマは行末より右と文書末尾より下も塗る() {
    let mut h = Harness::new().with_size(30, 6);
    // テーマは設定ファイルから (eve-tui は設定の型を知らない)
    h.fs.add_file(
        "/evo/config/config.toml",
        b"[theme]
name = \"default-dark\"
",
    );
    h.cmd("config.reload");
    h.run_jobs();
    h.text("ab");
    let f = frame_of(&mut h);
    let r = render_panes(&f, ColorDepth::TrueColor, &Glyphs::unicode());
    for (x, y) in [(20, 0), (10, 4)] {
        let c = r.buffer.cell((x, y)).unwrap();
        assert_eq!(
            c.bg,
            ratatui::style::Color::Rgb(0x1e, 0x1e, 0x1e),
            "({x},{y})"
        );
    }
}
