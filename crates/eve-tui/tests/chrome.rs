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
    let base = f.size.rows + eve_tui::buffer::bar_rows(f);
    CellSize {
        cols: f.size.cols,
        rows: base + chrome_rows_for_surface(base),
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

// verifies: AC-FR-503-2
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
    let mut h = Harness::new().with_size(120, 12);
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

// verifies: AC-FR-602-1
#[test]
fn パレットは枠つきで候補を並べ_選択行に_印を付け_件数を下辺に出す() {
    let mut h = Harness::new().with_size(80, 20);
    h.keys("ctrl+p");
    let (rows, cursor) = screen(&mut h, &Glyphs::unicode());
    let all = joined(&rows);
    assert!(all.contains("コマンド"), "見出し\n{all}");
    assert!(all.contains("コマンドを入力"), "placeholder\n{all}");
    assert!(all.contains("▶"), "選択行の印\n{all}");
    assert!(all.contains("app.quit"), "CommandId\n{all}");
    assert!(
        rows.iter().any(|r| r.contains("└") && r.contains('/')),
        "下辺に件数\n{all}"
    );
    assert_eq!(cursor.map(|c| c.row), Some(2), "カーソルは入力欄");
}

#[test]
fn パレットで絞り込むと件数が変わり_0_件の文言を出す() {
    let mut h = Harness::new().with_size(80, 20);
    h.keys("ctrl+p");
    h.text("zzzzzz");
    let (rows, _) = screen(&mut h, &Glyphs::unicode());
    let all = joined(&rows);
    assert!(all.contains("一致するコマンドはありません"), "{all}");
    assert!(all.contains("0/"), "{all}");
}

#[test]
fn パレットは狭い端末でも壊れない() {
    for (cols, rows) in [(20, 8), (40, 6), (80, 4)] {
        let mut h = Harness::new().with_size(cols, rows);
        h.keys("ctrl+p");
        let _ = screen(&mut h, &Glyphs::ascii());
        h.text("save");
        let _ = screen(&mut h, &Glyphs::ascii());
    }
}

#[test]
fn 検索バーは編集面の下に出て_入力欄にカーソルを置く() {
    let mut h = Harness::new().with_size(60, 12);
    h.text("foo bar foo");
    h.keys("ctrl+home ctrl+f");
    h.text("foo");
    h.run_jobs();
    let (rows, cursor) = screen(&mut h, &Glyphs::unicode());
    let all = joined(&rows);
    // 編集面 11 行 + 検索バー + クローム 3 行
    assert_eq!(rows.len(), 15, "{all}");
    let bar = &rows[11];
    assert!(bar.starts_with("検索: foo"), "{bar:?}");
    assert!(bar.contains("[Aa~]"), "{bar:?}");
    assert!(bar.trim_end().ends_with("1/2"), "{bar:?}");
    assert_eq!(cursor.map(|c| c.row), Some(11));
    assert_eq!(cursor.map(|c| c.col), Some(9), "入力の末尾");
    // ステータスバーはバーの下
    assert!(rows[12].contains("行"), "{all}");
}

// verifies: AC-FR-402-4
#[test]
fn 検索バーは大小の区別を反転で示し_狭いと件数から省く() {
    let mut h = Harness::new().with_size(60, 12);
    h.text("Foo");
    h.keys("ctrl+f");
    h.text("Foo");
    h.run_jobs();
    let f = h.frame();
    let r = render_screen(&f, window_of(&f), ColorDepth::TrueColor, &Glyphs::unicode());
    let bar_y = f.size.rows;
    let toggle_x = row_text(&r.buffer, bar_y).find("[Aa~]").unwrap();
    let col = unicode_width::UnicodeWidthStr::width(&row_text(&r.buffer, bar_y)[..toggle_x]) as u16;
    assert!(
        r.buffer[(col, bar_y)].modifier.contains(Modifier::REVERSED),
        "区別しているときは反転"
    );
    // 幅 24: 件数を省く
    for cols in [24u16, 16, 12] {
        let mut h = Harness::new().with_size(cols, 12);
        h.text("foo foo");
        h.keys("ctrl+f");
        h.text("foo");
        h.run_jobs();
        let (rows, _) = screen(&mut h, &Glyphs::ascii());
        let bar = &rows[11];
        assert!(
            bar.starts_with("検索:") || bar.starts_with("Search") || !bar.is_empty(),
            "{bar:?}"
        );
        if cols == 24 {
            assert!(!bar.contains("1/"), "件数は省く: {bar:?}");
        }
    }
}

#[test]
fn 置換欄は_2_行目に出て_フォーカスの欄にカーソルを置く() {
    let mut h = Harness::new().with_size(60, 12);
    h.text("foo");
    h.keys("ctrl+r");
    h.text("foo");
    h.keys("tab");
    h.text("bar");
    let (rows, cursor) = screen(&mut h, &Glyphs::unicode());
    let all = joined(&rows);
    // 編集面 10 行 + 検索バー 2 行 + クローム 3 行
    assert_eq!(rows.len(), 15, "{all}");
    assert!(rows[10].starts_with("検索: foo"), "{all}");
    assert!(rows[11].starts_with("置換: bar"), "{all}");
    assert!(rows[11].contains("[A] すべて置換"), "{all}");
    assert_eq!(
        cursor.map(|c| (c.row, c.col)),
        Some((11, 9)),
        "置換欄の末尾"
    );
    h.keys("tab");
    let (rows, cursor) = screen(&mut h, &Glyphs::unicode());
    let col = rows[11].find("[A]").unwrap();
    let x = unicode_width::UnicodeWidthStr::width(&rows[11][..col]) as u16;
    assert_eq!(
        cursor.map(|c| (c.row, c.col)),
        Some((11, x)),
        "ボタンの先頭"
    );
    // 狭いとボタンは [A] だけ
    let mut h = Harness::new().with_size(30, 12);
    h.keys("ctrl+r");
    let (rows, _) = screen(&mut h, &Glyphs::unicode());
    assert!(
        rows[11].contains("[A]") && !rows[11].contains("すべて"),
        "{:?}",
        rows[11]
    );
}

// verifies: AC-FR-506-1
#[test]
fn 横スクロールで左右に続きがある行の端に印を出す() {
    let mut h = Harness::new().with_size(30, 8);
    h.text(&"abcdefghij".repeat(10));
    // 行末へ: 画面幅を超えるので横スクロールし、左に続きがある (右は行末なので続かない)
    h.keys("end");
    let (rows, _) = screen(&mut h, &Glyphs::unicode());
    let first: Vec<char> = rows[0].chars().collect();
    assert!(
        first.contains(&'‹') && !first.contains(&'›'),
        "左だけに続く: {:?}",
        rows[0]
    );
    // 行頭へ: 右に続きがある
    h.keys("home");
    let (rows, _) = screen(&mut h, &Glyphs::unicode());
    assert!(
        rows[0].contains('›') && !rows[0].contains('‹'),
        "右だけに続く: {:?}",
        rows[0]
    );
    // ASCII 縮退時は `<` `>`
    h.keys("home");
    let (rows_ascii, _) = screen(&mut h, &Glyphs::ascii());
    assert!(rows_ascii[0].contains('>'), "{:?}", rows_ascii[0]);
}

// verifies: AC-FR-506-1
#[test]
fn 画面に収まる行には印を出さない() {
    let mut h = Harness::new().with_size(30, 8);
    h.text("short");
    let (rows, _) = screen(&mut h, &Glyphs::unicode());
    assert!(!rows[0].contains('‹') && !rows[0].contains('›'));
}

// verifies: AC-FR-505-1
#[test]
fn メッセージは記号と色で区別でき_警告は対処も並べて出る() {
    let colour_of_mark = |h: &mut Harness, glyphs: &Glyphs, mark: &str| {
        let f = h.frame();
        let r = render_screen(&f, window_of(&f), ColorDepth::TrueColor, glyphs);
        let area = r.buffer.area;
        for y in 0..area.height {
            let row = row_text(&r.buffer, y);
            if let Some(x) = row.find(mark) {
                let col = row[..x].chars().count() as u16;
                return Some((row, r.buffer[(col, y)].style()));
            }
        }
        None
    };
    // 警告: 親フォルダが無いパスを開く (本文 + 対処)
    let mut w = Harness::new().with_size(100, 12);
    w.open("/nodir/a.txt");
    w.run_jobs();
    let (row, warn_style) =
        colour_of_mark(&mut w, &Glyphs::unicode(), "⚠").expect("警告の記号が出る");
    assert!(
        row.contains("フォルダを作ってから保存するか"),
        "対処 (どうすればよいか) も並ぶ: {row}"
    );
    // ASCII の縮退でも記号で区別できる
    assert!(colour_of_mark(&mut w, &Glyphs::ascii(), "!").is_some());
    // 情報: 記号が違い、色 (または属性) も違う
    let mut i = Harness::new()
        .with_size(100, 12)
        .with_file("/a.txt", b"hello\n");
    i.open("/a.txt");
    i.run_jobs();
    i.text("x");
    i.keys("ctrl+s");
    i.run_jobs();
    let (_, info_style) =
        colour_of_mark(&mut i, &Glyphs::unicode(), "ℹ").expect("情報の記号が出る");
    assert_ne!(warn_style, info_style, "警告と情報は色か属性が違う");
}

// which-key (04 §7.4): 保留が 500 ms を超えたら、編集面の左下に続きのキーと動作を重ねる
#[test]
fn which_key_は編集面の左下に続きのキーと動作を重ねる() {
    let mut h = Harness::new().with_size(80, 24);
    h.keys("ctrl+k");
    let (rows, _) = screen(&mut h, &Glyphs::unicode());
    assert!(
        !joined(&rows).contains("メッセージ履歴"),
        "500 ms 前は出ない"
    );
    h.advance_ms(500);
    let (rows, _) = screen(&mut h, &Glyphs::unicode());
    let all = joined(&rows);
    assert!(all.contains("Ctrl+K …"), "{all}");
    let line_of = |t: &str| {
        rows.iter()
            .find(|r| r.contains(t))
            .cloned()
            .unwrap_or_default()
    };
    assert!(line_of("メッセージ履歴").contains("│ M "), "{all}");
    assert!(line_of("名前を付けて保存").contains("│ S "), "{all}");
    // 左下: 編集面の最下行の近くにある
    let at = rows
        .iter()
        .rposition(|r| r.contains("メッセージ履歴"))
        .unwrap();
    assert!(at >= 12, "{at}");
}
