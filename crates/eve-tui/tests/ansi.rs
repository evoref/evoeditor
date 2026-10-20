//! 差分出力 (`ansi::Emitter`) の検査 (docs/04 §4.3 / §4.5 / §4.6 / §4.7)。出力を最小の仮想端末で解釈し、
//! 部分更新の結果が全再描画と同じ画面になること (S-06 の 1,000 回更新の確認と同じ性質) を確かめる。

use eve_testkit::Harness;
use eve_tui::ansi::{ENTER, Emitter, LEAVE};
use eve_tui::buffer::{PaneRender, render_panes};
use eve_tui::glyphs::Glyphs;
use eve_view::{ColorDepth, Frame};
use ratatui::buffer::CellDiffOption;
use unicode_width::UnicodeWidthChar;

/// 最小の仮想端末: CUP・ED(2)・EL(0)・文字の幅 (半角カナの濁点は別に 1 セル進む、S-06) だけを解釈する。
struct Screen {
    w: usize,
    h: usize,
    cells: Vec<String>,
    x: usize,
    y: usize,
    cursor_visible: bool,
    sync_depth: i32,
    hidden_during_sync: bool,
}

impl Screen {
    fn new(w: usize, h: usize) -> Screen {
        Screen {
            w,
            h,
            cells: vec![" ".to_owned(); w * h],
            x: 0,
            y: 0,
            cursor_visible: true,
            sync_depth: 0,
            hidden_during_sync: false,
        }
    }

    fn width(ch: char) -> usize {
        if matches!(ch, '\u{FF9E}' | '\u{FF9F}') {
            1
        } else {
            ch.width().unwrap_or(0)
        }
    }

    fn feed(&mut self, s: &str) {
        let cs: Vec<char> = s.chars().collect();
        let mut i = 0;
        while i < cs.len() {
            let c = cs[i];
            if c == '\x1b' {
                match cs.get(i + 1) {
                    Some('[') => {
                        let mut j = i + 2;
                        let mut params = String::new();
                        while j < cs.len() && !('\x40'..='\x7e').contains(&cs[j]) {
                            params.push(cs[j]);
                            j += 1;
                        }
                        let fin = cs[j];
                        self.csi(&params, fin);
                        i = j + 1;
                    }
                    Some(']') => {
                        let mut j = i + 2;
                        while j < cs.len()
                            && !(cs[j] == '\x07'
                                || (cs[j] == '\x1b' && cs.get(j + 1) == Some(&'\\')))
                        {
                            j += 1;
                        }
                        i = if cs[j] == '\x07' { j + 1 } else { j + 2 };
                    }
                    _ => panic!("未知のエスケープ"),
                }
                continue;
            }
            self.put(c);
            i += 1;
        }
    }

    fn csi(&mut self, params: &str, fin: char) {
        match fin {
            'H' => {
                let mut it = params.split(';');
                let r: usize = it.next().and_then(|v| v.parse().ok()).unwrap_or(1);
                let c: usize = it.next().and_then(|v| v.parse().ok()).unwrap_or(1);
                self.y = (r - 1).min(self.h - 1);
                self.x = (c - 1).min(self.w - 1);
            }
            'J' if params == "2" => self.cells.iter_mut().for_each(|c| *c = " ".to_owned()),
            'K' if params == "0" => {
                for x in self.x..self.w {
                    self.cells[self.y * self.w + x] = " ".to_owned();
                }
            }
            'h' | 'l' if params == "?25" => {
                self.cursor_visible = fin == 'h';
                if fin == 'l' && self.sync_depth > 0 {
                    self.hidden_during_sync = true;
                }
            }
            'h' | 'l' if params == "?2026" => self.sync_depth += if fin == 'h' { 1 } else { -1 },
            'm' | 'q' | 'h' | 'l' => {}
            other => panic!("未知の CSI: {params:?} {other:?}"),
        }
    }

    fn put(&mut self, ch: char) {
        let w = Screen::width(ch);
        if w == 0 {
            // 結合文字: 直前のセルに足す
            let i = self.y * self.w + self.x.saturating_sub(1);
            self.cells[i].push(ch);
            return;
        }
        if self.x + w > self.w {
            return;
        }
        let i = self.y * self.w + self.x;
        self.cells[i] = ch.to_string();
        if w == 2 {
            self.cells[i + 1] = String::new();
        }
        self.x += w;
        if self.x >= self.w {
            self.x = self.w - 1;
        }
    }

    fn rows(&self) -> Vec<String> {
        (0..self.h)
            .map(|y| self.cells[y * self.w..(y + 1) * self.w].concat())
            .collect()
    }
}

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

fn render(f: &Frame) -> PaneRender {
    render_panes(f, ColorDepth::TrueColor, &Glyphs::unicode())
}

#[test]
fn 最初のフレームは全画面で_buffer_と同じ画面になる() {
    let mut h = Harness::new().with_size(30, 6);
    h.text("hello\nあいう");
    let f = h.frame();
    let r = render(&f);
    let mut e = Emitter::new();
    let out = e.frame(&r, &f, true);
    let mut s = Screen::new(30, r.buffer.area.height as usize);
    s.feed(&out);
    assert_eq!(s.rows(), buffer_rows(&r));
    assert!(
        out.starts_with("\x1b[?2026h") && out.ends_with("\x1b[?2026l"),
        "BSU と ESU で囲む"
    );
    assert!(!s.hidden_during_sync, "描画中にカーソルを隠さない");
    assert!(s.cursor_visible);
}

#[test]
fn カーソルは最後に出力され_形状は変わったときだけ() {
    let mut h = Harness::new().with_size(30, 6);
    h.text("ab");
    let f = h.frame();
    let r = render(&f);
    let mut e = Emitter::new();
    let first = e.frame(&r, &f, true);
    let pos = r.cursor.unwrap();
    let cup = format!("\x1b[{};{}H", pos.row + 1, pos.col + 1);
    let tail = first.strip_suffix("\x1b[?2026l").unwrap();
    assert!(tail.ends_with(&format!("{cup}\x1b[?25h")), "{tail:?}");
    assert!(first.contains(" q"), "最初のフレームは形状を出す");
    let second = e.frame(&r, &f, true);
    assert!(!second.contains(" q"), "形状が同じなら出さない");
    assert!(
        !second.contains("ab"),
        "変化が無ければセルを出さない: {second:?}"
    );
}

#[test]
fn 部分更新の結果は全再描画と同じ() {
    let mut h = Harness::new().with_size(40, 8);
    let mut e = Emitter::new();
    let mut s = Screen::new(40, 8 + 3);
    let steps = [
        "abc",
        "\n日本語",
        "x",
        "\nｶﾞｷﾞ",
        "BS",
        "\nあ\nい\nう",
        "Z",
        "\nlonger line of text here",
        "BS3",
    ];
    for (i, step) in steps.iter().enumerate() {
        match *step {
            "BS" => h.keys("backspace"),
            "BS3" => h.keys("backspace backspace backspace"),
            t => h.text(t),
        }
        let f = h.frame();
        let r = render(&f);
        let out = e.frame(&r, &f, true);
        s.feed(&out);
        // 全再描画 (新しい出力器・新しい端末) と同じ
        let mut fresh = Screen::new(40, 8 + 3);
        fresh.feed(&Emitter::new().frame(&r, &f, true));
        assert_eq!(s.rows(), fresh.rows(), "段階 {i}: {step:?}");
        assert_eq!(s.rows(), buffer_rows(&r), "段階 {i}");
    }
}

#[test]
fn 幅危険行は変化したとき行全体を再出力する() {
    let mut h = Harness::new().with_size(30, 6);
    h.text("a\nｶﾞb");
    let f = h.frame();
    let r = render(&f);
    assert_eq!(r.risky_rows, vec![1]);
    let mut e = Emitter::new();
    e.frame(&r, &f, true);
    h.text("c");
    let f2 = h.frame();
    let r2 = render(&f2);
    let out = e.frame(&r2, &f2, true);
    assert!(out.contains("\x1b[0K"), "EL で行の残骸を消す: {out:?}");
    // 行全体の再出力: 変化していない文字 (半角カナ) も出る
    assert!(out.contains("ｶ"));
}

#[test]
fn 全角の後は_cup_で絶対位置を指定する() {
    let mut h = Harness::new().with_size(30, 6);
    h.text("あb");
    let f = h.frame();
    let r = render(&f);
    let out = Emitter::new().frame(&r, &f, false);
    let gw = f.panes[0].gutter_width;
    // 「b」は「あ」(幅 2) の直後。連続書込に頼らず CUP を出す
    let b_col = gw + 2;
    let want = format!("\x1b[1;{}H", b_col + 1);
    let i = out.find(&want).expect("b の位置への CUP");
    assert!(out[i..].contains('b'));
}

#[test]
fn 再描画の要求とリサイズは全画面() {
    let mut h = Harness::new().with_size(30, 6);
    h.text("x");
    let f = h.frame();
    let r = render(&f);
    let mut e = Emitter::new();
    e.frame(&r, &f, true);
    e.invalidate();
    let out = e.frame(&r, &f, true);
    assert!(out.contains("\x1b[2J"), "全画面再描画は画面消去から");
    // 大きさが変わったときも全画面
    let mut h2 = Harness::new().with_size(20, 5);
    h2.text("x");
    let f2 = h2.frame();
    let out2 = e.frame(&render(&f2), &f2, true);
    assert!(out2.contains("\x1b[2J"));
}

#[test]
fn タイトルは変化したときだけ_制御文字を含めない() {
    let mut h = Harness::new().with_size(30, 6);
    h.text("x");
    let mut f = h.frame();
    f.title = "a\u{7}b - evoeditor".to_owned();
    let r = render(&f);
    let mut e = Emitter::new();
    let first = e.frame(&r, &f, true);
    assert!(first.contains("\x1b]0;ab - evoeditor\x1b\\"), "{first:?}");
    let second = e.frame(&r, &f, true);
    assert!(!second.contains("\x1b]0;"));
}

#[test]
fn 入場と復元の列() {
    assert!(ENTER.starts_with("\x1b[?1049h") && ENTER.contains("\x1b[?7l"));
    // 04 §14.3: ESU が先頭、代替スクリーンを抜けるのが最後
    assert!(LEAVE.starts_with("\x1b[?2026l"));
    assert!(LEAVE.ends_with("\x1b[?1049l"));
    assert!(
        LEAVE.contains("\x1b[?7h") && LEAVE.contains("\x1b[?25h") && LEAVE.contains("\x1b[0 q")
    );
}
