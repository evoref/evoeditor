//! L4: 仮想画面 (`VirtualScreen`) を使った出力の検査 (docs/09 §3.5)。Frame → Buffer → 出力バイト列 → 仮想画面の往復で、
//! 画面・カーソル・クロームの省略が Frame の宣言と一致することを確かめる (04 §4.7 の幅不一致対策 (a)(c)、08 S-06 の合否基準)。

use eve_testkit::{Harness, VirtualScreen, WidthConfig};
use eve_tui::ansi::Emitter;
use eve_tui::buffer::{PaneRender, chrome_rows_for_surface};
use eve_tui::chrome::render_screen;
use eve_tui::glyphs::Glyphs;
use eve_view::{CellSize, ColorDepth, Frame};
use ratatui::buffer::CellDiffOption;

/// `Buffer` の各行 (継続セルを除いた文字列)。
fn buffer_rows(r: &PaneRender) -> Vec<String> {
    (0..r.buffer.area.height)
        .map(|y| {
            (0..r.buffer.area.width)
                .filter_map(|x| {
                    let c = r.buffer.cell((x, y)).unwrap();
                    (c.diff_option != CellDiffOption::Skip).then(|| c.symbol().to_owned())
                })
                .collect()
        })
        .collect()
}

fn window_of(f: &Frame) -> CellSize {
    CellSize {
        cols: f.size.cols,
        rows: f.size.rows + chrome_rows_for_surface(f.size.rows),
    }
}

/// 1 つの端末 (出力器 + 仮想画面) を模した描画の組。
struct Term {
    emitter: Emitter,
    screen: VirtualScreen,
    glyphs: Glyphs,
    depth: ColorDepth,
}

impl Term {
    fn new(cols: u16, rows: u16, width: WidthConfig, ascii: bool, depth: ColorDepth) -> Term {
        Term {
            emitter: Emitter::new(),
            screen: VirtualScreen::new(cols, rows + chrome_rows_for_surface(rows), width),
            glyphs: Glyphs::for_ascii_only(ascii),
            depth,
        }
    }

    /// 現在の Frame を描き、(Buffer の行, 出力) を返す。
    fn draw(&mut self, h: &mut Harness) -> (PaneRender, Frame) {
        let f = h.frame();
        let r = render_screen(&f, window_of(&f), self.depth, &self.glyphs);
        let out = self.emitter.frame(&r, &f, true);
        self.screen.feed(&out);
        (r, f)
    }
}

/// 決定的な擬似乱数 (xorshift)。
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[(self.next() % items.len() as u64) as usize]
    }
}

#[test]
fn 曖昧幅_2_の端末でも_ascii_縮退なら千回の更新の後も画面が一致する() {
    // S-06 の合否基準のテスト化: 本文は ASCII・全角・半角カナ (濁点つき)。装飾は ASCII (conhost 相当)
    let mut h = Harness::new().with_size(40, 8);
    let mut t = Term::new(
        40,
        8,
        WidthConfig::ambiguous(2),
        true,
        ColorDepth::TrueColor,
    );
    let texts = [
        "a",
        "bc",
        "あ",
        "日本語",
        "ｶﾞ",
        "ｷﾞｸﾞ",
        "x y",
        "ABC123",
        "ー",
        "！",
        "\n",
        "\n",
    ];
    let keys = [
        "backspace",
        "backspace",
        "left",
        "right",
        "up",
        "down",
        "home",
        "end",
        "delete",
        "enter",
    ];
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for step in 0..1000 {
        if rng.next().is_multiple_of(3) {
            h.keys(rng.pick(&keys));
        } else {
            h.text(rng.pick(&texts));
        }
        let (r, f) = t.draw(&mut h);
        assert_eq!(t.screen.lines(), buffer_rows(&r), "段階 {step}");
        if step % 50 == 0 {
            // 全再描画 (新しい出力器・新しい端末) と同じ
            let mut fresh = Term::new(
                40,
                8,
                WidthConfig::ambiguous(2),
                true,
                ColorDepth::TrueColor,
            );
            let out = fresh.emitter.frame(&r, &f, true);
            fresh.screen.feed(&out);
            assert_eq!(t.screen.lines(), fresh.screen.lines(), "段階 {step}");
        }
    }
    assert!(t.screen.sync_balanced());
    assert!(!t.screen.hid_cursor_during_sync());
}

#[test]
fn 半角カナの濁点を別に進める端末でも編集面が一致する() {
    let mut h = Harness::new().with_size(30, 6);
    let mut t = Term::new(30, 6, WidthConfig::default(), false, ColorDepth::TrueColor);
    h.text("ｶﾞｷﾞｸﾞ\nabc");
    let (r, _) = t.draw(&mut h);
    assert_eq!(t.screen.lines(), buffer_rows(&r));
    h.keys("up right");
    h.text("ｹﾞ");
    let (r, _) = t.draw(&mut h);
    assert_eq!(t.screen.lines(), buffer_rows(&r));
}

#[test]
fn ハードウェアカーソルは毎フレームの最後の_cup_で_編集面の論理位置と一致する() {
    let mut h = Harness::new().with_size(40, 8);
    let mut t = Term::new(40, 8, WidthConfig::default(), false, ColorDepth::TrueColor);
    for step in ["a", "あいう", "\nｶﾞb", "BS"] {
        if step == "BS" {
            h.keys("backspace left");
        } else {
            h.text(step);
        }
        let (r, f) = t.draw(&mut h);
        let pos = f.primary_cursor.expect("編集面にフォーカス").pos;
        assert_eq!(
            t.screen.last_cup(),
            Some((pos.col as usize, pos.row as usize))
        );
        assert_eq!(t.screen.cursor(), (pos.col as usize, pos.row as usize));
        assert!(t.screen.cursor_visible());
        assert_eq!(r.cursor, Some(pos));
        // 全角の上では先頭セル
        let (c, y) = t.screen.cursor();
        assert_ne!(t.screen.text_at(c, y), "", "後ろ半分の上にはない");
    }
    assert_eq!(t.screen.cursor_shape(), Some(6), "既定はバー");
}

#[test]
fn プロンプトの入力欄のカーソルは入力位置_全角と横スクロールを含む() {
    let mut h = Harness::new().with_size(50, 8);
    let mut t = Term::new(50, 8, WidthConfig::default(), false, ColorDepth::TrueColor);
    h.keys("ctrl+g");
    for text in ["12", "全角の行番号", &"9".repeat(80)] {
        h.text(text);
        let (r, _) = t.draw(&mut h);
        let cur = r.cursor.expect("入力欄のカーソル");
        assert_eq!(
            t.screen.last_cup(),
            Some((cur.col as usize, cur.row as usize))
        );
        assert!(t.screen.cursor_visible());
        let line = t.screen.lines()[cur.row as usize].clone();
        let before: String = (0..cur.col as usize)
            .map(|x| t.screen.text_at(x, cur.row as usize))
            .collect();
        let tail = text.chars().last().unwrap().to_string();
        assert!(
            before.trim_end().ends_with(&tail),
            "カーソルの手前が入力の末尾: {line:?} col={}",
            cur.col
        );
    }
}

#[test]
fn 確認ダイアログのカーソルはフォーカス中のボタンの先頭() {
    let mut h = Harness::new().with_size(70, 12);
    let mut t = Term::new(70, 12, WidthConfig::default(), false, ColorDepth::TrueColor);
    h.text("x");
    h.keys("ctrl+q");
    let (_, _) = t.draw(&mut h);
    let (c, y) = t.screen.cursor();
    assert_eq!(t.screen.text_at(c, y), "▶");
    h.keys("tab");
    t.draw(&mut h);
    let (c2, y2) = t.screen.cursor();
    assert_eq!(y2, y);
    assert!(c2 > c, "次のボタンへ");
    assert_eq!(t.screen.text_at(c2, y2), "▶");
}

#[test]
fn ステータスバーとヒントバーは幅ごとに優先度の順で残る() {
    for cols in [100u16, 80, 60, 40, 30, 24] {
        let mut h = Harness::new().with_size(cols, 12);
        let mut t = Term::new(
            cols,
            12,
            WidthConfig::default(),
            false,
            ColorDepth::TrueColor,
        );
        h.text("abc");
        let (r, _) = t.draw(&mut h);
        assert_eq!(t.screen.lines(), buffer_rows(&r), "{cols} 桁");
        let lines = t.screen.lines();
        let (status, hint) = (&lines[12], &lines[14]);
        assert!(
            status.contains("[+]"),
            "{cols} 桁: 変更マーカーは残る: {status:?}"
        );
        assert!(
            hint.contains("^Q") || hint.contains("Ctrl+Q"),
            "{cols} 桁: 優先度 5 は残る: {hint:?}"
        );
        // 幅を超えない (仮想画面は溢れを捨てるので、Buffer と一致していることで確かめている)
        if cols >= 100 {
            assert!(hint.contains("Ctrl+Q 終了"), "100 桁は全表示: {hint:?}");
        }
    }
}

#[test]
fn overlay_文脈のヒントでは届かないキーを出さない() {
    let mut h = Harness::new().with_size(80, 12);
    let mut t = Term::new(80, 12, WidthConfig::default(), false, ColorDepth::TrueColor);
    h.text("x");
    h.keys("ctrl+q");
    t.draw(&mut h);
    let hint = t.screen.lines()[14].clone();
    assert!(hint.contains("Esc"), "{hint:?}");
    assert!(!hint.contains("Ctrl+Q") && !hint.contains("^Q"), "{hint:?}");
}

#[test]
fn ロールの色は出力に載り_色なしでは属性だけになる() {
    let mut h = Harness::new().with_size(60, 12);
    let mut t = Term::new(60, 12, WidthConfig::default(), false, ColorDepth::TrueColor);
    t.draw(&mut h);
    let a = t.screen.attr_at(0, 12);
    assert_eq!(a.bg.as_deref(), Some("rgb(47,51,64)"), "{a:?}");
    let mut t = Term::new(60, 12, WidthConfig::default(), false, ColorDepth::Mono);
    t.draw(&mut h);
    let a = t.screen.attr_at(0, 12);
    assert!(a.reverse && a.bg.is_none(), "{a:?}");
}

#[test]
fn 画面の外へ出た全角の分断を残さない() {
    // 確認ダイアログの枠が全角の本文の上に重なっても、半分だけの全角が残らない
    let mut h = Harness::new().with_size(60, 12);
    let mut t = Term::new(60, 12, WidthConfig::default(), false, ColorDepth::TrueColor);
    h.text(&"あ".repeat(28));
    t.draw(&mut h);
    h.keys("ctrl+q");
    let (r, _) = t.draw(&mut h);
    assert_eq!(t.screen.lines(), buffer_rows(&r));
    h.keys("escape");
    let (r, _) = t.draw(&mut h);
    assert_eq!(
        t.screen.lines(),
        buffer_rows(&r),
        "ダイアログを閉じた後も一致する"
    );
}
