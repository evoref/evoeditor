//! ヘルプ画面 (04 §8.6)。

use eve_view::{CellPos, CellSize, HelpLineKind, OverlayModel, Style, StyleMods};

use super::parts::{fill, put};
use crate::buffer::Writer;
use crate::theme::{Theme, overlay_style};

/// ヘルプ画面 (04 §8.6)。編集面いっぱいに描く: 1 行目に見出し、最後の行に操作の案内、間に `scroll` 行目からの本文。
/// 節の見出しは太字 + 下線、小見出しは太字。キー一覧・コマンド一覧の行は、左に固定幅の列 (キー / CommandId)、右に説明。
pub(super) fn render_help(
    w: &mut Writer<'_>,
    area: CellSize,
    overlay: &OverlayModel,
    theme: &Theme,
) -> Option<CellPos> {
    let OverlayModel::Help {
        title,
        lines,
        scroll,
        footer,
    } = overlay
    else {
        return None;
    };
    let style = theme.overlay;
    let (cols, rows) = (area.cols, area.rows);
    if rows < 3 || cols < 10 {
        return None;
    }
    for y in 0..rows {
        fill(w, 0, y, cols, style);
    }
    let bold = overlay_style(style, Style::with_mods(StyleMods::BOLD));
    put(w, 0, 0, cols, &title.spans, bold);
    // 左の列の幅: 狭い画面では全体の 1/3 まで
    let key_w = (cols / 3).clamp(8, 28);
    let body_rows = usize::from(rows) - 2;
    let max = lines.len().saturating_sub(body_rows);
    let first = (*scroll).min(max);
    for (i, line) in lines.iter().skip(first).take(body_rows).enumerate() {
        let y = 1 + i as u16;
        match line.kind {
            HelpLineKind::Heading => {
                let st = overlay_style(
                    style,
                    Style::with_mods(StyleMods::BOLD | StyleMods::UNDERLINE),
                );
                put(w, 0, y, cols, &line.text.spans, st);
            }
            HelpLineKind::Subheading => {
                put(w, 1, y, cols, &line.text.spans, bold);
            }
            HelpLineKind::Body => {
                put(w, 1, y, cols, &line.text.spans, style);
            }
            HelpLineKind::Entry => {
                let mut x = 2;
                if let Some(k) = &line.key {
                    put(w, x, y, x + key_w, &k.spans, bold);
                }
                x += key_w + 1;
                put(w, x, y, cols, &line.text.spans, style);
            }
        }
    }
    // 下端: 操作の案内と位置
    put(
        w,
        0,
        rows - 1,
        cols,
        &footer.spans,
        overlay_style(style, Style::with_mods(StyleMods::DIM)),
    );
    None
}
