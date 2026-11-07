//! `Buffer` の差分を端末への出力 (エスケープシーケンス) にする (docs/04 §4.1 / §4.3 / §4.5 / §4.6 / §4.7)。
//! 純粋な計算 (文字列を作るだけ) で、書込は呼び側。ratatui の `Terminal` / backend は使わない (BSU/ESU と 1 回の書込を保証するため、
//! 04 §4.8)。ratatui は `Buffer` と `Cell` の型として使う。
//!
//! 1 フレームの出力: BSU (同期開始) → 変化したセル (CUP + SGR + 文字) → カーソル形状 → カーソル位置 → カーソル表示 → ESU。
//! 幅 2 以上の書記素と非 ASCII の直後は、次のセルの前に必ず CUP で絶対位置を指定する (§4.7 (a))。`width_risky` の行が
//! 変化したときと、前のフレームの全角の後続セルが単独のセルに変わる行 (全角を空白などで上書きする) も、行全体を
//! 再出力する (§4.7 (c): EL で残骸を消し、全書記素の前で CUP)。

use std::fmt::Write as _;

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
    /// 前のフレームの出力の長さ (次の出力の確保の目安)。
    last_len: usize,
}

/// 色の SGR 部分 (`;` から) を `s` に書く。
fn push_color(s: &mut String, c: Color, fg: bool) {
    let base = if fg { 38 } else { 48 };
    // String への書込は失敗しない
    let _ = match c {
        Color::Rgb(r, g, b) => write!(s, ";{base};2;{r};{g};{b}"),
        Color::Indexed(i) => write!(s, ";{base};5;{i}"),
        // 既定色。名前付きの色は eve-tui では作らない (core は Default / Rgb / Indexed だけ)。作られたら既定色として扱う
        _ => Ok(()),
    };
}

/// セルのスタイルの SGR (先頭に `0` を付けて、直前のスタイルを消す) を `s` に書く。既定のスタイルなら `\x1b[0m` だけ。
/// 作業用の String を作らずに出力へ直接書く (REF-03)。
fn push_sgr(s: &mut String, cell: &Cell) {
    s.push_str("\x1b[0");
    let m = cell.modifier;
    for (flag, code) in [
        (Modifier::BOLD, ";1"),
        (Modifier::DIM, ";2"),
        (Modifier::ITALIC, ";3"),
        (Modifier::UNDERLINED, ";4"),
        (Modifier::REVERSED, ";7"),
    ] {
        if m.contains(flag) {
            s.push_str(code);
        }
    }
    push_color(s, cell.fg, true);
    push_color(s, cell.bg, false);
    s.push('m');
}

fn style_key(cell: &Cell) -> (Color, Color, Modifier) {
    (cell.fg, cell.bg, cell.modifier)
}

/// カーソルの絶対位置 (CUP) を `s` に書く。
fn push_cup(s: &mut String, x: u16, y: u16) {
    let _ = write!(s, "\x1b[{};{}H", y + 1, x + 1);
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
            push_cup(&mut self.s, x, y);
        }
        if self.style != Some(style_key(cell)) {
            push_sgr(&mut self.s, cell);
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
    /// `r` のバッファは次のフレームとの差分のために覚える (複製せずに移す、PRF-07)。
    ///
    /// # Panics
    ///
    /// `r` のバッファの範囲内のセルを取得できないとき。内部の不変条件が破れたときだけで、実際には起きない。
    pub fn frame(&mut self, r: PaneRender, frame: &Frame, sync: bool) -> String {
        let cur = &r.buffer;
        let area = cur.area;
        let full = self.force_full
            || self
                .prev
                .as_ref()
                .is_none_or(|p| p.area.width != area.width || p.area.height != area.height);
        let mut o = Out {
            s: String::with_capacity(self.last_len),
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
            // 全角の後続セルが単独のセルに変わる行: 先頭セルだけを上書きすると、tmux は後続セルも消えたとみなして
            // 書かず (カーソルを送るだけ)、Windows Terminal は後続セルの色・反転を残す (§4.7 (c))
            let split_wide = prev.is_some_and(|p| {
                (0..area.width).any(|x| {
                    p.cell((x, y))
                        .is_some_and(|pc| pc.diff_option == CellDiffOption::Skip)
                        && cur.cell((x, y)).expect("範囲内").diff_option != CellDiffOption::Skip
                })
            });
            if r.risky_rows.contains(&y) || split_wide {
                // 行全体を再出力: 行頭へ CUP し、EL で残骸を消し、全書記素の前で CUP
                push_cup(&mut o.s, 0, y);
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
            let _ = write!(o.s, "\x1b]0;{title}\x1b\\");
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
                    let _ = write!(o.s, "\x1b[{n} q");
                    self.shape = Some(shape);
                }
                push_cup(&mut o.s, pos.col, pos.row);
                o.s.push_str("\x1b[?25h");
            }
            // 編集面にフォーカスが無い (オーバーレイの入力欄の位置は後の版)
            (None, _) => o.s.push_str("\x1b[?25l"),
        }
        if sync {
            o.s.push_str(ESU);
        }
        self.prev = Some(r.buffer);
        self.force_full = false;
        self.last_len = o.s.len();
        o.s
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// 標準の base64 (パディングあり)。
pub fn base64(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// クリップボードへ書く OSC 52 (`c` = クリップボード)。tmux の中では DCS 透過で包む (05 §8.2)。
pub fn osc52(text: &str, tmux: bool) -> String {
    let seq = format!("\x1b]52;c;{}\x1b\\", base64(text.as_bytes()));
    if tmux {
        // DCS の中では ESC を 2 重にする
        format!("\x1bPtmux;{}\x1b\\", seq.replace('\x1b', "\x1b\x1b"))
    } else {
        seq
    }
}

/// 起動時の出力 (04 §14.1 の 4): 代替スクリーンへ、続けて自動折り返しを無効化する (§4.7 (g))。
pub const ENTER: &str = "\x1b[?1049h\x1b[?7l";

/// 復元の出力 (04 §14.3 の 1〜7): ESU → SGR リセット → カーソル形状を既定へ → カーソル表示 → 自動折り返しを有効へ
/// → 代替スクリーンを抜ける。v0.1 はマウス・フォーカス・ブラケットペースト・拡張キーを有効にしないので送らない。
pub const LEAVE: &str = "\x1b[?2026l\x1b[0m\x1b[0 q\x1b[?25h\x1b[?7h\x1b[?1049l";

#[cfg(test)]
mod osc52_tests {
    use super::*;

    #[test]
    fn base64_は標準の符号化() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64("あ".as_bytes()), "44GC");
    }

    #[test]
    fn osc52_は_st_で閉じ_tmux_では_dcs_で包む() {
        assert_eq!(osc52("foo", false), "\x1b]52;c;Zm9v\x1b\\");
        assert_eq!(
            osc52("foo", true),
            "\x1bPtmux;\x1b\x1b]52;c;Zm9v\x1b\x1b\\\x1b\\"
        );
    }
}
