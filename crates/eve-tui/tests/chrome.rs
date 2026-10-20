//! クローム (ステータスバー・メッセージ行・ヒントバー) とオーバーレイ (確認・入力プロンプト) の描画の検査
//! (docs/04 §3.3 / §3.5 / §8.2 / §8.3 / §8.7〜§8.9)。Harness が作る `Frame` を `render_screen` で `Buffer` にして、行の文字列を見る。

use eve_testkit::Harness;
use eve_tui::buffer::{chrome_rows_for_surface, row_text};
use eve_tui::chrome::render_screen;
use eve_tui::glyphs::Glyphs;
use eve_tui::theme::Theme;
use eve_view::{CellPos, CellSize, ColorDepth, Frame};
use ratatui::style::Modifier;

fn window_of(f: &Frame) -> CellSize {
    CellSize {
        cols: f.size.cols,
        rows: f.size.rows + chrome_rows_for_surface(f.size.rows),
    }
}

fn screen(h: &mut Harness, glyphs: &Glyphs) -> (Vec<String>, Option<CellPos>) {
    let f = h.frame();
    let r = render_screen(&f, window_of(&f), ColorDepth::TrueColor, glyphs);
    let rows = (0..r.buffer.area.height)
        .map(|y| row_text(&r.buffer, y))
        .collect();
    (rows, r.cursor)
}

fn joined(rows: &[String]) -> String {
    rows.join("\n")
}

#[test]
fn ステータスバーは左にファイル名と変更_右に位置() {
    let mut h = Harness::new().with_size(80, 12);
    h.text("abc");
    let (rows, _) = screen(&mut h, &Glyphs::unicode());
    let status = &rows[12];
    assert!(status.starts_with("無題-1"), "{status:?}");
    assert!(status.contains("[+]"), "{status:?}");
    assert!(status.contains("行 1, 桁 4"), "{status:?}");
    assert!(status.contains("UTF-8"), "{status:?}");
    let t = status.trim_end();
    assert!(t.ends_with("LF"), "右端に寄る: {status:?}");
}

#[test]
fn ステータスバーは幅が足りなければ短い形にして_優先度の低い項目から落とす() {
    let mut h = Harness::new().with_size(30, 12);
    h.text("abc");
    let (rows, _) = screen(&mut h, &Glyphs::unicode());
    let status = rows[12].trim_end();
    assert!(status.contains("1:4"), "短い位置: {status:?}");
    assert!(status.contains("[+]"), "変更マーカーは残す: {status:?}");
    let mut h = Harness::new().with_size(20, 12);
    h.text("abc");
    let (rows, _) = screen(&mut h, &Glyphs::unicode());
    assert!(
        rows[12].contains("[+]"),
        "優先度 5 は落とさない: {:?}",
        rows[12]
    );
}

#[test]
fn ヒントバーは全表示から圧縮される() {
    let mut h = Harness::new().with_size(100, 12);
    let (rows, _) = screen(&mut h, &Glyphs::unicode());
    let hint = &rows[14];
    assert!(hint.contains("Ctrl+Q 終了"), "{hint:?}");
    let mut h = Harness::new().with_size(30, 12);
    let (rows, _) = screen(&mut h, &Glyphs::unicode());
    let hint = &rows[14];
    assert!(hint.contains("^Q 終了"), "圧縮表記: {hint:?}");
    assert!(hint.starts_with("^Q 終了"), "優先度 5 は残る: {hint:?}");
}

#[test]
fn 十行未満ではメッセージ行を確保しない() {
    assert_eq!(chrome_rows_for_surface(7), 2);
    assert_eq!(chrome_rows_for_surface(8), 3);
    let mut h = Harness::new().with_size(60, 6);
    let (rows, _) = screen(&mut h, &Glyphs::unicode());
    assert_eq!(rows.len(), 8);
    assert!(
        rows[7].contains("終了"),
        "ヒントバーは 2 行目: {:?}",
        rows[7]
    );
}

#[test]
fn 確認ダイアログは中央に枠とボタンを描き_カーソルは既定のボタン() {
    let mut h = Harness::new().with_size(70, 14);
    h.text("x");
    h.keys("ctrl+q");
    assert_eq!(h.overlay(), "dialog");
    let g = Glyphs::unicode();
    let (rows, cur) = screen(&mut h, &g);
    let all = joined(&rows);
    assert!(all.contains("┌─ "), "{all}");
    assert!(all.contains("[S] "), "{all}");
    assert!(all.contains("[D] "), "{all}");
    assert!(all.contains("[Esc] "), "{all}");
    assert!(all.contains("▶[S] "), "既定のボタンに ▶: {all}");
    let cur = cur.expect("ボタンの先頭にカーソル");
    let line = &rows[cur.row as usize];
    let at: String = line.chars().skip(cur.col as usize).take(4).collect();
    assert_eq!(at, "▶[S]", "{line:?}");
    // ヒントバーは overlay 文脈
    assert!(rows[16].contains("取消"), "{:?}", rows[16]);
}

#[test]
fn 縮退時の枠線は_ascii() {
    let mut h = Harness::new().with_size(70, 14);
    h.text("x");
    h.keys("ctrl+q");
    let (rows, _) = screen(&mut h, &Glyphs::for_ascii_only(true));
    let all = joined(&rows);
    assert!(all.contains("+- "), "{all}");
    assert!(all.contains(">[S]"), "{all}");
    assert!(!all.contains('┌') && !all.contains('▶'));
}

#[test]
fn 低い端末では確認ダイアログは本文とボタンと案内だけ_枠なし() {
    // 端末 20×7 → 編集面 5 行、クローム 2 行 (04 §3.5)
    let mut h = Harness::new().with_size(30, 5);
    h.text("x");
    h.keys("ctrl+q");
    let (rows, cur) = screen(&mut h, &Glyphs::unicode());
    let all = joined(&rows[..5]);
    assert!(!all.contains('┌'), "枠なし: {all}");
    assert!(all.contains("[S]") && all.contains("[Esc]"), "{all}");
    assert!(cur.is_some());
}

#[test]
fn 入力プロンプトは上部中央に出て_カーソルは入力位置() {
    let mut h = Harness::new().with_size(70, 14);
    h.keys("ctrl+g");
    assert_eq!(h.overlay(), "prompt");
    h.text("12");
    let (rows, cur) = screen(&mut h, &Glyphs::unicode());
    let all = joined(&rows[..14]);
    assert!(all.contains("┌─ "), "{all}");
    assert!(all.contains("12"), "{all}");
    let cur = cur.expect("入力欄のカーソル");
    let line = &rows[cur.row as usize];
    let before: String = line.chars().take(cur.col as usize).collect();
    assert!(
        before.trim_end().ends_with("12"),
        "{line:?} col={}",
        cur.col
    );
}

#[test]
fn 端末が小さすぎるときは文言だけを描く() {
    let mut h = Harness::new().with_size(18, 2);
    let f = h.frame();
    let window = CellSize { cols: 18, rows: 3 };
    let r = render_screen(&f, window, ColorDepth::TrueColor, &Glyphs::unicode());
    let text: String = (0..3).map(|y| row_text(&r.buffer, y)).collect();
    assert!(text.contains("端末が小さすぎます"), "{text}");
    assert!(r.cursor.is_none());
}

#[test]
fn テーマのロール_ステータスバーは反転か背景色() {
    let t = Theme::truecolor();
    assert_ne!(t.status_bar.bg, eve_view::Color::Default);
    let m = Theme::mono();
    assert!(m.status_bar.mods.contains(eve_view::StyleMods::REVERSE));
    let mut h = Harness::new().with_size(60, 12);
    let f = h.frame();
    let r = render_screen(&f, window_of(&f), ColorDepth::Mono, &Glyphs::unicode());
    let c = r.buffer.cell((0, 12)).unwrap();
    assert!(c.modifier.contains(Modifier::REVERSED));
}
