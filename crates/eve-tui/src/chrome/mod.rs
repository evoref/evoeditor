//! クローム (ステータスバー・メッセージ行・ヒントバー) とオーバーレイ (確認ダイアログ・入力プロンプト) の配置と描画
//! (docs/04 §3.1 / §3.3 / §3.5 / §8.2 / §8.3 / §8.7〜§8.9)。core が意味的モデルで渡したものを、TUI が配置・省略して `Buffer` に書く。
//!
//! 幅は core が `Span.width` で与えた値を足すだけで、幅関数は持たない。この層が自分で作る文字列 (キー表記・ボタンの `[S]`) は
//! ASCII か、`glyphs` の定数表の記号だけで、幅は定数で数える。枠線も `glyphs` から 1 文字ずつ書く (ratatui のウィジェットは使わない)。
//!
//! - `layout`: 画面の行の割り当て (`ScreenLayout`、04 §3.1 / §3.5)。
//! - `parts` / `keys`: 共通の部品とキー表記 (04 §7.8)。
//! - `status` / `message` / `hints` / `search_bar`: 編集面の下の行 (04 §8.4 / §8.7〜§8.9)。
//! - `overlay` / `input_field` / `confirm` / `prompt` / `list` / `help` / `which_key`: オーバーレイ (04 §7.4 / §8.1〜§8.6)。

use eve_view::{CellPos, CellSize, ColorDepth, Frame, OverlayModel};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::buffer::{PaneRender, Writer};
use crate::glyphs::Glyphs;
use crate::theme::Theme;

mod confirm;
mod help;
mod hints;
mod input_field;
mod keys;
pub mod layout;
mod list;
mod message;
mod overlay;
mod parts;
mod prompt;
mod search_bar;
mod status;
mod which_key;

pub use keys::{key_label_compact, key_label_full};
pub use layout::{MIN_WINDOW, ScreenLayout};

use confirm::render_confirm;
use help::render_help;
use hints::render_hints;
use list::render_list;
use message::render_message;
use prompt::render_prompt;
use search_bar::render_search_bar;
use status::render_status;
use which_key::render_which_key;

// ---------------------------------------------------------------------------------------------
// 全体

/// 端末が小さすぎるときの表示 (04 §3.3 (e))。core が解決した文言をそのまま折り返して描く。
fn render_too_small(frame: &Frame, window: CellSize, depth: ColorDepth) -> PaneRender {
    let mut buffer = Buffer::empty(Rect::new(0, 0, window.cols, window.rows));
    let (clamped, sanitized);
    {
        let mut w = Writer {
            buf: &mut buffer,
            depth,
            clamped: 0,
            sanitized: 0,
        };
        let (mut x, mut y) = (0u16, 0u16);
        for sp in &frame.texts.too_small.spans {
            if sp.width > window.cols {
                continue;
            }
            if x + sp.width > window.cols {
                x = 0;
                y += 1;
            }
            if y >= window.rows {
                break;
            }
            x = w.span(x, y, window.cols, sp);
        }
        clamped = w.clamped;
        sanitized = w.sanitized;
    }
    PaneRender {
        buffer,
        cursor: None,
        risky_rows: Vec::new(),
        clamped_cells: clamped,
        sanitized,
    }
}

/// 画面全体 (編集面 + クローム + オーバーレイ) を `Buffer` に書く。`window` は端末の大きさ。
pub fn render_screen(
    frame: &Frame,
    window: CellSize,
    depth: ColorDepth,
    glyphs: &Glyphs,
) -> PaneRender {
    if ScreenLayout::is_too_small(window) {
        return render_too_small(frame, window, depth);
    }
    // 行の割り当ては端末の大きさから決める (04 §3.5。編集面の行数だけでは 10 行の境目を区別できない)
    let layout = ScreenLayout::for_frame_in(frame, window);
    let mut r = crate::buffer::render_panes_in(frame, layout, depth, glyphs);
    let theme = Theme::from_roles(&frame.roles);
    let ascii = glyphs.ellipsis.width == 3;
    // 検索バーの行は編集面の下。クロームはその下に続く
    let cols = layout.cols;
    let area = layout.surface();
    let mut cursor = r.cursor;
    {
        let mut w = Writer {
            buf: &mut r.buffer,
            depth,
            clamped: r.clamped_cells,
            sanitized: r.sanitized,
        };
        render_status(
            &mut w,
            layout.status_row(),
            cols,
            &frame.status,
            &theme,
            glyphs,
        );
        let message_y = layout.message_row();
        match layout.hint_row() {
            // ヒントバーを隠した (FR-504): ステータスバーの下の 1 行はメッセージだけに使う (04 §3.5)
            None => render_message(&mut w, message_y, cols, &frame.messages, &theme, glyphs),
            Some(hint_y) if hint_y != message_y => {
                render_message(&mut w, message_y, cols, &frame.messages, &theme, glyphs);
                render_hints(&mut w, hint_y, cols, &frame.hints, &theme, glyphs, ascii);
            }
            // 10 行未満: メッセージはヒントバーの行に上書きする (04 §3.5)
            Some(_) if frame.messages.current.is_some() => {
                render_message(&mut w, message_y, cols, &frame.messages, &theme, glyphs);
            }
            Some(hint_y) => {
                render_hints(&mut w, hint_y, cols, &frame.hints, &theme, glyphs, ascii);
            }
        }
        if let Some(bar) = &frame.search {
            cursor =
                render_search_bar(&mut w, layout.bar_row(), cols, bar, &theme, glyphs).or(cursor);
        }
        // オーバーレイ (前面順なので、奥から描く)
        for ov in frame.overlays.iter().rev() {
            let c = match ov {
                OverlayModel::Confirm { .. } => render_confirm(&mut w, area, ov, &theme, glyphs),
                OverlayModel::Prompt { .. } => render_prompt(&mut w, area, ov, &theme, glyphs),
                OverlayModel::List { .. } => render_list(&mut w, area, ov, &theme, glyphs),
                OverlayModel::Help { .. } => render_help(&mut w, area, ov, &theme),
                _ => render_unknown_overlay(),
            };
            if c.is_some() {
                cursor = c;
            }
        }
        if let Some(wk) = &frame.which_key {
            render_which_key(&mut w, area, wk, &theme, glyphs, ascii);
        }
        r.clamped_cells = w.clamped;
        r.sanitized = w.sanitized;
    }
    // オーバーレイがあるときの編集面のカーソルは出さない (core は primary_cursor を None にする)
    r.cursor = cursor;
    r
}

/// eve-tui が知らないオーバーレイの既定描画 (02 §8.4 の「frontend は既定描画を持つ」)。何も描かず、カーソルも動かさない
/// (下の編集面・クロームはそのまま見える。パニックしない)。変種の中身は `#[non_exhaustive]` の外から読めないので、
/// 退化させて描くことはこの層ではできない。変種を足す PR が TUI の描画を同時に足す (02 §8.4、04 §16)。今の変種
/// (`Confirm` / `Prompt` / `List` / `Help`) はすべて上の腕で描くので、ここへは来ない。
fn render_unknown_overlay() -> Option<CellPos> {
    None
}
