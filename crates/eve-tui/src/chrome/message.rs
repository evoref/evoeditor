//! メッセージ行 (04 §8.9)。

use eve_view::{Message, MessageLevel, MessageModel, Span};

use super::parts::{ascii_span, fill, glyph_span, put, truncate_end};
use crate::buffer::Writer;
use crate::glyphs::Glyphs;
use crate::theme::Theme;

pub(super) fn render_message(
    w: &mut Writer<'_>,
    y: u16,
    cols: u16,
    model: &MessageModel,
    theme: &Theme,
    glyphs: &Glyphs,
) {
    let Some(Message { level, text, hint }) = &model.current else {
        return;
    };
    let (style, mark) = match level {
        MessageLevel::Info => (theme.message_info, &glyphs.info),
        MessageLevel::Warn => (theme.message_warning, &glyphs.warning),
        MessageLevel::Error => (theme.message_error, &glyphs.error),
        _ => (theme.message_info, &glyphs.info),
    };
    if style.bg != eve_view::Color::Default
        || !style.mods.is_empty() && *level != MessageLevel::Info
    {
        fill(w, 0, y, cols, style);
    }
    let suffix = if model.unread > 0 {
        format!(" (+{})", model.unread)
    } else {
        String::new()
    };
    let head_w = mark.width + 1;
    let hint_w = hint.as_ref().map_or(0, |h| 2 + h.width);
    let fixed = head_w + hint_w + suffix.len() as u16;
    // 切り詰めは詳細 (本文) から: 対処 (hint) は省略しない
    let text_budget = cols.saturating_sub(fixed);
    let mut line: Vec<Span> = vec![glyph_span(mark, style), ascii_span(" ", style)];
    line.extend(truncate_end(&text.spans, text_budget, &glyphs.ellipsis));
    if let Some(h) = hint {
        line.push(ascii_span("  ", style));
        line.extend(h.spans.iter().cloned());
    }
    if !suffix.is_empty() {
        line.push(ascii_span(&suffix, style));
    }
    let line = truncate_end(&line, cols, &glyphs.ellipsis);
    put(w, 0, y, cols, &line, style);
}
