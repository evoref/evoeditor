//! `Buffer` の差分を端末への出力 (エスケープシーケンス) にする (docs/04 §4.1 / §4.3 / §4.5 / §4.6 / §4.7)。
//! 純粋な計算 (文字列を作るだけ) で、書込は呼び側。ratatui の `Terminal` / backend は使わない (BSU/ESU と 1 回の書込を保証するため、
//! 04 §4.8)。ratatui は `Buffer` と `Cell` の型として使う。
//!
//! 1 フレームの出力: BSU (同期開始) → 変化したセル (CUP + SGR + 文字) → カーソル形状 → カーソル位置 → カーソル表示 → ESU。
//! 幅 2 以上の書記素と非 ASCII の直後は、次のセルの前に必ず CUP で絶対位置を指定する (§4.7 (a))。`width_risky` の行が
//! 変化したときは行全体を再出力する (§4.7 (c): EL で残骸を消し、全書記素の前で CUP)。

use eve_view::{CursorShape, Frame};
use ratatui::buffer::{Buffer, Cell, CellDiffOption};
use ratatui::style::{Color, Modifier};

use crate::buffer::PaneRender;
use crate::sanitize::sanitize_title;

const BSU: &str = "\x1b[?2026h";
const ESU: &str = "\x1b[?2026l";
const RESET: &str = "\x1b[0m";

/// 差分の出力器。直前に出力した `Buffer` などを覚える。
#[derive(Debug, Default)]
pub struct Emitter {
    prev: Option<Buffer>,
    shape: Option<CursorShape>,
    title: Option<String>,
    /// 次のフレームを全画面再描画にする (リサイズ後・`ui.redraw`)。
    force_full: bool,
}

/// 色の SGR 部分。
fn color_params(c: Color, fg: bool) -> Option<String> {
    let base = if fg { 38 } else { 48 };
    match c {
        Color::Reset => None,
        Color::Rgb(r, g, b) => Some(format!("{base};2;{r};{g};{b}")),
        Color::Indexed(i) => Some(format!("{base};5;{i}")),
        // 名前付きの色は eve-tui では作らない (core は Default / Rgb / Indexed だけ)。作られたら既定色として扱う
        _ => None,
    }
}

/// セルのスタイルの SGR (先頭に `0` を付けて、直前のスタイルを消す)。既定のスタイルなら `\x1b[0m` だけ。
fn sgr(cell: &Cell) -> String {
    let mut p: Vec<String> = vec!["0".into()];
    let m = cell.modifier;
    for (flag, code) in [
        (Modifier::BOLD, "1"),
        (Modifier::DIM, "2"),
        (Modifier::ITALIC, "3"),
        (Modifier::UNDERLINED, "4"),
        (Modifier::REVERSED, "7"),
    ] {
        if m.contains(flag) {
            p.push(code.into());
        }
    }
    if let Some(s) = color_params(cell.fg, true) {
        p.push(s);
    }
    if let Some(s) = color_params(cell.bg, false) {
        p.push(s);
    }
    format!("\x1b[{}m", p.join(";"))
}

fn style_key(cell: &Cell) -> (Color, Color, Modifier) {
    (cell.fg, cell.bg, cell.modifier)
}

fn cup(x: u16, y: u16) -> String {
    format!("\x1b[{};{}H", y + 1, x + 1)
}

fn cell_changed(prev: Option<&Cell>, cur: &Cell) -> bool {
    match prev {
        None => true,
        Some(p) => {
            p.symbol() != cur.symbol()
                || style_key(p) != style_key(cur)
                || p.diff_option != cur.diff_option
        }
    }
}

struct Out {
    s: String,
    /// 直前に書いたセルの右隣 (CUP を省ける位置)。`None` なら次は必ず CUP。
    next: Option<(u16, u16)>,
    style: Option<(Color, Color, Modifier)>,
}

impl Out {
    fn put(&mut self, x: u16, y: u16, cell: &Cell, force_cup: bool) {
        if force_cup || self.next != Some((x, y)) {
            self.s.push_str(&cup(x, y));
        }
        if self.style != Some(style_key(cell)) {
            self.s.push_str(&sgr(cell));
            self.style = Some(style_key(cell));
        }
        self.s.push_str(cell.symbol());
        let sym = cell.symbol();
        // ASCII 1 文字だけが「カーソルが 1 セル進む」と信用できる (§4.7 (a))
        self.next = (sym.len() == 1 && sym.is_ascii()).then_some((x + 1, y));
    }
}

impl Emitter {
    pub fn new() -> Emitter {
        Emitter::default()
    }

    /// 次のフレームを全画面再描画にする (リサイズ・`ui.redraw`・復帰後。04 §4.3)。
    pub fn invalidate(&mut self) {
        self.force_full = true;
    }

    /// 1 フレームの出力を作る。`sync` が真なら BSU / ESU で囲む (04 §4.5。常に送る)。
    pub fn frame(&mut self, r: &PaneRender, frame: &Frame, sync: bool) -> String {
        let cur = &r.buffer;
        let area = cur.area;
        let full = self.force_full
            || self
                .prev
                .as_ref()
                .is_none_or(|p| p.area.width != area.width || p.area.height != area.height);
        let mut o = Out {
            s: String::new(),
            next: None,
            style: None,
        };
        if sync {
            o.s.push_str(BSU);
        }
        if full {
            // 画面消去 + 全セル出力 (消去の後の既定のスタイルは空白)
            o.s.push_str(RESET);
            o.s.push_str("\x1b[2J");
            o.style = Some((Color::Reset, Color::Reset, Modifier::empty()));
        }
        let prev = if full { None } else { self.prev.as_ref() };
        for y in 0..area.height {
            let row_changed = (0..area.width).any(|x| {
                let c = cur.cell((x, y)).expect("範囲内");
                c.diff_option != CellDiffOption::Skip
                    && cell_changed(prev.and_then(|p| p.cell((x, y))), c)
            }) || (0..area.width).any(|x| {
                // 継続セルが通常のセルに変わった場合も行は変化している
                prev.and_then(|p| p.cell((x, y)))
                    .is_some_and(|p| p.diff_option != cur.cell((x, y)).expect("範囲内").diff_option)
            });
            if !row_changed {
                continue;
            }
            if r.risky_rows.contains(&y) {
                // 行全体を再出力: 行頭へ CUP し、EL で残骸を消し、全書記素の前で CUP
                o.s.push_str(&cup(0, y));
                o.s.push_str(RESET);
                o.style = Some((Color::Reset, Color::Reset, Modifier::empty()));
                o.s.push_str("\x1b[0K");
                o.next = None;
                for x in 0..area.width {
                    let c = cur.cell((x, y)).expect("範囲内");
                    if c.diff_option != CellDiffOption::Skip {
                        o.put(x, y, c, true);
                    }
                }
                continue;
            }
            for x in 0..area.width {
                let c = cur.cell((x, y)).expect("範囲内");
                if c.diff_option == CellDiffOption::Skip {
                    continue;
                }
                if cell_changed(prev.and_then(|p| p.cell((x, y))), c) {
                    o.put(x, y, c, false);
                }
            }
        }
        o.s.push_str(RESET);

        // 端末タイトル (変化したときだけ。BEL / ST を含めない)
        let title = sanitize_title(&frame.title);
        if self.title.as_deref() != Some(title.as_str()) {
            o.s.push_str(&format!("\x1b]0;{title}\x1b\\"));
            self.title = Some(title);
        }
        // カーソル: 形状 (変わったときだけ) → 位置 → 表示 (§4.6)。描画中は隠さない
        match (&r.cursor, frame.primary_cursor) {
            (Some(pos), pc) => {
                let shape = pc.map_or(CursorShape::Bar, |c| c.shape);
                if self.shape != Some(shape) {
                    let n = match shape {
                        CursorShape::Block => 2,
                        CursorShape::Underline => 4,
                        _ => 6,
                    };
                    o.s.push_str(&format!("\x1b[{n} q"));
                    self.shape = Some(shape);
                }
                o.s.push_str(&cup(pos.col, pos.row));
                o.s.push_str("\x1b[?25h");
            }
            // 編集面にフォーカスが無い (オーバーレイの入力欄の位置は後の版)
            (None, _) => o.s.push_str("\x1b[?25l"),
        }
        if sync {
            o.s.push_str(ESU);
        }
        self.prev = Some(cur.clone());
        self.force_full = false;
        o.s
    }
}

/// 起動時の出力 (04 §14.1 の 4): 代替スクリーンへ、続けて自動折り返しを無効化する (§4.7 (g))。
pub const ENTER: &str = "\x1b[?1049h\x1b[?7l";

/// 復元の出力 (04 §14.3 の 1〜7): ESU → SGR リセット → カーソル形状を既定へ → カーソル表示 → 自動折り返しを有効へ
/// → 代替スクリーンを抜ける。v0.1 はマウス・フォーカス・ブラケットペースト・拡張キーを有効にしないので送らない。
pub const LEAVE: &str = "\x1b[?2026l\x1b[0m\x1b[0 q\x1b[?25h\x1b[?7h\x1b[?1049l";
