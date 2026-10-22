//! クローム (ステータスバー・メッセージ行・ヒントバー) とオーバーレイ (確認ダイアログ・入力プロンプト) の配置と描画
//! (docs/04 §3.1 / §3.3 / §3.5 / §8.2 / §8.3 / §8.7〜§8.9)。core が意味的モデルで渡したものを、TUI が配置・省略して `Buffer` に書く。
//!
//! 幅は core が `Span.width` で与えた値を足すだけで、幅関数は持たない。この層が自分で作る文字列 (キー表記・ボタンの `[S]`) は
//! ASCII か、`glyphs` の定数表の記号だけで、幅は定数で数える。枠線も `glyphs` から 1 文字ずつ書く (ratatui のウィジェットは使わない)。

use eve_view::{
    CaseMode, CellPos, CellSize, CellText, Choice, ColorDepth, CutPolicy, Frame, HintItem,
    HintModel, InputFieldModel, KeyCode, KeyLabel, KeyStroke, Message, MessageLevel, MessageModel,
    Modifiers, OverlayModel, SearchBarModel, Span, StatusAlign, StatusItem, StatusKind,
    StatusModel, Style, StyleMods,
};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::buffer::{PaneRender, Writer, chrome_rows_for_surface, render_panes};
use crate::glyphs::{Glyph, Glyphs};
use crate::theme::{Theme, overlay_style};

/// 端末の最小の大きさ (04 §3.5。`Capabilities.min_surface` で core に知らせる値と同じ)。
pub const MIN_WINDOW: CellSize = CellSize { cols: 20, rows: 5 };

/// 状態バーなどの項目の間の空白。
const ITEM_GAP: u16 = 1;
/// ステータスバーの左群と右群の間の最小の空き。
const GROUP_GAP: u16 = 2;

// ---------------------------------------------------------------------------------------------
// 部品

fn text_width(spans: &[Span]) -> u16 {
    spans.iter().map(|s| s.width).sum()
}

fn ascii_span(text: &str, style: Style) -> Span {
    Span::new(text, text.len() as u16, style)
}

fn glyph_span(g: &Glyph, style: Style) -> Span {
    Span::new(g.text, g.width, style)
}

/// 先頭から入るだけ取り、省略記号を付ける。
fn truncate_end(spans: &[Span], max: u16, ell: &Glyph) -> Vec<Span> {
    if text_width(spans) <= max {
        return spans.to_vec();
    }
    if max < ell.width {
        return Vec::new();
    }
    let budget = max - ell.width;
    let mut out = Vec::new();
    let mut acc = 0;
    for s in spans {
        if acc + s.width > budget {
            break;
        }
        acc += s.width;
        out.push(s.clone());
    }
    let style = out.last().map_or(Style::DEFAULT, |s| s.style);
    out.push(glyph_span(ell, style));
    out
}

/// 中間を省略し、先頭と末尾 (拡張子) を残す。
fn truncate_middle(spans: &[Span], max: u16, ell: &Glyph) -> Vec<Span> {
    if text_width(spans) <= max {
        return spans.to_vec();
    }
    if max < ell.width {
        return Vec::new();
    }
    let budget = max - ell.width;
    let head_budget = budget / 2;
    let tail_budget = budget - head_budget;
    let mut head = Vec::new();
    let mut acc = 0;
    for s in spans {
        if acc + s.width > head_budget {
            break;
        }
        acc += s.width;
        head.push(s.clone());
    }
    let mut tail = Vec::new();
    let mut acc = 0;
    for s in spans[head.len()..].iter().rev() {
        if acc + s.width > tail_budget {
            break;
        }
        acc += s.width;
        tail.push(s.clone());
    }
    tail.reverse();
    let style = head.last().map_or(Style::DEFAULT, |s| s.style);
    head.push(glyph_span(ell, style));
    head.extend(tail);
    head
}

/// `base` の上に各 `Span` のスタイルを重ねて書く。次の位置を返す。
fn put(w: &mut Writer<'_>, x: u16, y: u16, limit: u16, spans: &[Span], base: Style) -> u16 {
    let mut x = x;
    for sp in spans {
        let s = Span::new(sp.text.clone(), sp.width, overlay_style(base, sp.style));
        x = w.span(x, y, limit, &s);
    }
    x
}

fn fill(w: &mut Writer<'_>, x: u16, y: u16, n: u16, style: Style) {
    let st = w.style(style);
    for i in 0..n {
        w.set(x + i, y, " ", st, false);
    }
}

// ---------------------------------------------------------------------------------------------
// キー表記 (04 §7.8)

fn key_name(code: KeyCode, ascii: bool) -> String {
    match code {
        KeyCode::Char(' ') => "Space".to_owned(),
        KeyCode::Char(c) => c.to_uppercase().collect(),
        KeyCode::F(n) => format!("F{n}"),
        KeyCode::Enter => "Enter".to_owned(),
        KeyCode::Tab => "Tab".to_owned(),
        KeyCode::Esc => "Esc".to_owned(),
        KeyCode::Backspace => "Backspace".to_owned(),
        KeyCode::Delete => "Del".to_owned(),
        KeyCode::Insert => "Ins".to_owned(),
        KeyCode::Home => "Home".to_owned(),
        KeyCode::End => "End".to_owned(),
        KeyCode::PageUp => "PgUp".to_owned(),
        KeyCode::PageDown => "PgDn".to_owned(),
        KeyCode::Up => if ascii { "Up" } else { "↑" }.to_owned(),
        KeyCode::Down => if ascii { "Down" } else { "↓" }.to_owned(),
        KeyCode::Left => if ascii { "Left" } else { "←" }.to_owned(),
        KeyCode::Right => if ascii { "Right" } else { "→" }.to_owned(),
        KeyCode::Menu => "Menu".to_owned(),
        KeyCode::CapsLock => "CapsLock".to_owned(),
        KeyCode::PrintScreen => "PrtSc".to_owned(),
        _ => "?".to_owned(),
    }
}

/// `Ctrl+S` 形式。チョードは空白区切り。
pub fn key_label_full(label: &KeyLabel, ascii: bool) -> String {
    let stroke = |s: &KeyStroke| {
        let mut out = String::new();
        for (flag, name) in [
            (Modifiers::CTRL, "Ctrl"),
            (Modifiers::ALT, "Alt"),
            (Modifiers::SHIFT, "Shift"),
            (Modifiers::SUPER, "Super"),
        ] {
            if s.mods.contains(flag) {
                out.push_str(name);
                out.push('+');
            }
        }
        out.push_str(&key_name(s.key, ascii));
        out
    };
    label
        .strokes
        .iter()
        .map(stroke)
        .collect::<Vec<_>>()
        .join(" ")
}

/// ヒントバーの圧縮表記: `Ctrl` + 1 文字は `^S`、それ以外はそのまま。
pub fn key_label_compact(label: &KeyLabel, ascii: bool) -> String {
    let stroke = |s: &KeyStroke| match (s.mods, s.key) {
        (m, KeyCode::Char(c)) if m == Modifiers::CTRL && c != ' ' => {
            format!("^{}", c.to_uppercase().collect::<String>())
        }
        _ => key_label_full(&KeyLabel { strokes: vec![*s] }, ascii),
    };
    label
        .strokes
        .iter()
        .map(stroke)
        .collect::<Vec<_>>()
        .join(" ")
}

fn label_width(s: &str) -> u16 {
    // この層が作るキー表記は ASCII と、非縮退時の矢印 (1 セル) だけ
    s.chars().count() as u16
}

// ---------------------------------------------------------------------------------------------
// ステータスバー (04 §8.7)

struct StatusEntry {
    kind: StatusKind,
    align: StatusAlign,
    priority: u8,
    /// 現在の形。
    spans: Vec<Span>,
    short: Option<Vec<Span>>,
    middle: bool,
    dropped: bool,
    /// 表示位置の順位 (左群 → 右群、群の中は `items` の順)。
    rank: usize,
}

fn status_total(entries: &[StatusEntry]) -> u16 {
    let group = |a: StatusAlign| {
        let ws: Vec<u16> = entries
            .iter()
            .filter(|e| !e.dropped && e.align == a)
            .map(|e| text_width(&e.spans))
            .collect();
        let sum: u16 = ws.iter().sum();
        let gaps = ws.len().saturating_sub(1) as u16 * ITEM_GAP;
        (ws.len(), sum + gaps)
    };
    let (ln, lw) = group(StatusAlign::Left);
    let (rn, rw) = group(StatusAlign::Right);
    lw + rw + if ln > 0 && rn > 0 { GROUP_GAP } else { 0 }
}

/// 優先度の低いものから、同じ優先度では右にあるものから。
fn status_order(entries: &[StatusEntry]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..entries.len()).collect();
    order.sort_by_key(|&i| (entries[i].priority, std::cmp::Reverse(entries[i].rank)));
    order
}

fn render_status(
    w: &mut Writer<'_>,
    y: u16,
    cols: u16,
    model: &StatusModel,
    theme: &Theme,
    glyphs: &Glyphs,
) {
    let base = theme.status_bar;
    fill(w, 0, y, cols, base);
    let mut entries: Vec<StatusEntry> = model
        .items
        .iter()
        .enumerate()
        .map(|(i, it): (usize, &StatusItem)| StatusEntry {
            kind: it.kind,
            align: it.align,
            priority: it.priority,
            spans: it.long.spans.clone(),
            short: it.short.as_ref().map(|s| s.spans.clone()),
            middle: it.long.cut == CutPolicy::Middle,
            dropped: false,
            rank: usize::from(it.align == StatusAlign::Right) * model.items.len() + i,
        })
        .collect();
    let order = status_order(&entries);
    // (1) 短い形にする
    for &i in &order {
        if status_total(&entries) <= cols {
            break;
        }
        if let Some(s) = entries[i].short.take() {
            entries[i].spans = s;
        }
    }
    // (2) 中間を省略できる項目 (ファイル名) を縮める
    let total = status_total(&entries);
    if total > cols {
        let excess = total - cols;
        if let Some(e) = entries.iter_mut().find(|e| e.middle && !e.dropped) {
            let want = text_width(&e.spans).saturating_sub(excess).max(5);
            e.spans = truncate_middle(&e.spans, want, &glyphs.ellipsis);
        }
    }
    // (3) 項目を落とす (優先度 5 は落とさない)
    for &i in &order {
        if status_total(&entries) <= cols {
            break;
        }
        if entries[i].priority < 5 {
            entries[i].dropped = true;
        }
    }
    let live = |a: StatusAlign| -> Vec<&StatusEntry> {
        entries
            .iter()
            .filter(|e| !e.dropped && e.align == a)
            .collect()
    };
    let styled = |e: &StatusEntry| {
        if e.kind == StatusKind::Chord {
            overlay_style(base, theme.status_chord)
        } else {
            base
        }
    };
    if status_total(&entries) > cols {
        // 優先度 5 だけでも入らない: 左群・右群の順に並べて末尾を省略記号で切る
        let mut line: Vec<Span> = Vec::new();
        for e in live(StatusAlign::Left)
            .into_iter()
            .chain(live(StatusAlign::Right))
        {
            if !line.is_empty() {
                line.push(ascii_span(" ", base));
            }
            let st = styled(e);
            line.extend(
                e.spans
                    .iter()
                    .map(|s| Span::new(s.text.clone(), s.width, overlay_style(st, s.style))),
            );
        }
        let line = truncate_end(&line, cols, &glyphs.ellipsis);
        put(w, 0, y, cols, &line, base);
        return;
    }
    let mut x = 0;
    for e in live(StatusAlign::Left) {
        x = put(w, x, y, cols, &e.spans, styled(e));
        x += ITEM_GAP;
    }
    let right = live(StatusAlign::Right);
    let rw: u16 = right.iter().map(|e| text_width(&e.spans)).sum::<u16>()
        + right.len().saturating_sub(1) as u16 * ITEM_GAP;
    let mut x = cols - rw;
    for e in right {
        x = put(w, x, y, cols, &e.spans, styled(e));
        x += ITEM_GAP;
    }
}

// ---------------------------------------------------------------------------------------------
// ヒントバー (04 §8.8)

struct HintEntry<'a> {
    item: &'a HintItem,
    full: String,
    compact: String,
    /// 説明を落としてキーだけにする (最後の段)。
    key_only: bool,
    dropped: bool,
}

fn hint_total(entries: &[HintEntry<'_>], compact: bool) -> u16 {
    let live: Vec<&HintEntry<'_>> = entries.iter().filter(|e| !e.dropped).collect();
    let gap = if compact { 1 } else { 2 };
    let sum: u16 = live
        .iter()
        .map(|e| {
            let k = label_width(if compact { &e.compact } else { &e.full });
            if e.key_only {
                k
            } else {
                k + 1 + e.item.text.width
            }
        })
        .sum();
    sum + live.len().saturating_sub(1) as u16 * gap
}

fn render_hints(
    w: &mut Writer<'_>,
    y: u16,
    cols: u16,
    model: &HintModel,
    theme: &Theme,
    glyphs: &Glyphs,
    ascii: bool,
) {
    let mut entries: Vec<HintEntry<'_>> = model
        .items
        .iter()
        .map(|item| {
            let label = item.keys.first().cloned().unwrap_or(KeyLabel {
                strokes: Vec::new(),
            });
            HintEntry {
                item,
                full: key_label_full(&label, ascii),
                compact: key_label_compact(&label, ascii),
                key_only: false,
                dropped: false,
            }
        })
        .collect();
    // (1) 全表示 → (2) ^S 表記 → (3) 優先度の低いものから落とす → (4) 優先度 5 はキーだけ
    let mut compact = false;
    if hint_total(&entries, false) > cols {
        compact = true;
        let mut order: Vec<usize> = (0..entries.len()).collect();
        order.sort_by_key(|&i| (entries[i].item.priority, std::cmp::Reverse(i)));
        for &i in &order {
            if hint_total(&entries, true) <= cols {
                break;
            }
            if entries[i].item.priority < 5 {
                entries[i].dropped = true;
            }
        }
        for &i in &order {
            if hint_total(&entries, true) <= cols {
                break;
            }
            if !entries[i].dropped {
                entries[i].key_only = true;
            }
        }
    }
    let gap = if compact { 1 } else { 2 };
    let mut x = 0;
    let mut first = true;
    for e in entries.iter().filter(|e| !e.dropped) {
        if !first {
            x += gap;
        }
        first = false;
        let key = if compact { &e.compact } else { &e.full };
        if x >= cols {
            break;
        }
        // キー表記は ASCII か 1 セルの矢印 → 1 文字ずつ書く
        for ch in key.chars() {
            let mut tmp = [0u8; 4];
            let s = Span::new(ch.encode_utf8(&mut tmp).to_owned(), 1, theme.hint_key);
            x = w.span(x, y, cols, &s);
        }
        if !e.key_only {
            x = w.span(x, y, cols, &ascii_span(" ", theme.hint_bar));
            x = put(w, x, y, cols, &e.item.text.spans, theme.hint_bar);
        }
    }
    let _ = glyphs;
}

// ---------------------------------------------------------------------------------------------
// メッセージ行 (04 §8.9)

fn render_message(
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

// ---------------------------------------------------------------------------------------------
// オーバーレイ

/// 枠。`bordered` が偽なら外周も内側として使う。
#[derive(Clone, Copy)]
struct BoxRect {
    x: u16,
    y: u16,
    cols: u16,
    rows: u16,
    bordered: bool,
}

impl BoxRect {
    /// 内側の左端・幅 (枠ありは左右に 1 セルの余白)。
    fn inner_x(&self) -> u16 {
        self.x + if self.bordered { 2 } else { 0 }
    }

    fn inner_cols(&self) -> u16 {
        self.cols.saturating_sub(if self.bordered { 4 } else { 0 })
    }

    fn inner_y(&self) -> u16 {
        self.y + u16::from(self.bordered)
    }
}

fn draw_box(
    w: &mut Writer<'_>,
    b: &BoxRect,
    title: Option<&CellText>,
    style: Style,
    glyphs: &Glyphs,
) {
    for r in 0..b.rows {
        fill(w, b.x, b.y + r, b.cols, style);
    }
    if !b.bordered || b.cols < 2 || b.rows < 2 {
        return;
    }
    let st = w.style(style);
    let g = &glyphs.boxed;
    let (x1, y1) = (b.x + b.cols - 1, b.y + b.rows - 1);
    for x in b.x + 1..x1 {
        w.set(x, b.y, g.horizontal.text, st, false);
        w.set(x, y1, g.horizontal.text, st, false);
    }
    for y in b.y + 1..y1 {
        w.set(b.x, y, g.vertical.text, st, false);
        w.set(x1, y, g.vertical.text, st, false);
    }
    w.set(b.x, b.y, g.top_left.text, st, false);
    w.set(x1, b.y, g.top_right.text, st, false);
    w.set(b.x, y1, g.bottom_left.text, st, false);
    w.set(x1, y1, g.bottom_right.text, st, false);
    if let Some(t) = title {
        // `┌─ 見出し ─…┐`
        let room = b.cols.saturating_sub(6);
        let spans = truncate_end(&t.spans, room, &glyphs.ellipsis);
        if !spans.is_empty() {
            let x = b.x + 2;
            let x = w.span(x, b.y, x1, &ascii_span(" ", style));
            let x = put(w, x, b.y, x1, &spans, style);
            w.span(x, b.y, x1, &ascii_span(" ", style));
        }
    }
}

/// 1 行のスパン列を、内側の幅に収めて書く。
fn draw_line(
    w: &mut Writer<'_>,
    b: &BoxRect,
    row: u16,
    spans: &[Span],
    style: Style,
    glyphs: &Glyphs,
) {
    let cut = truncate_end(spans, b.inner_cols(), &glyphs.ellipsis);
    put(
        w,
        b.inner_x(),
        b.inner_y() + row,
        b.inner_x() + b.inner_cols(),
        &cut,
        style,
    );
}

/// 枠の幅: 端末幅 − 4 まで。それ未満 (狭すぎる) なら全幅で枠なし (04 §3.5)。
fn box_width(want_inner: u16, cols: u16) -> (u16, bool) {
    const MIN_BOXED: u16 = 16;
    let avail = cols.saturating_sub(4);
    if avail < MIN_BOXED {
        return (cols, false);
    }
    ((want_inner + 4).min(avail).max(MIN_BOXED), true)
}

fn button_spans(
    choices: &[Choice],
    focused: eve_view::ChoiceId,
    theme: &Theme,
    glyphs: &Glyphs,
) -> (Vec<Span>, u16) {
    let mut spans = Vec::new();
    let mut focus_x = 0;
    let mut x = 0u16;
    for (i, c) in choices.iter().enumerate() {
        if i > 0 {
            spans.push(ascii_span("   ", theme.dialog_button));
            x += 3;
        }
        let is_focus = c.id == focused;
        let st = if is_focus {
            theme.dialog_button_focused
        } else {
            theme.dialog_button
        };
        let mark = if is_focus {
            glyph_span(&glyphs.focus, st)
        } else {
            ascii_span(" ", st)
        };
        if is_focus {
            focus_x = x;
        }
        x += mark.width;
        spans.push(mark);
        // 選択肢の文言は `[S] 保存して終了` のようにアクセラレータを含んで届く (03 / 言語ファイル)
        x += c.label.width;
        spans.extend(
            c.label
                .spans
                .iter()
                .map(|s| Span::new(s.text.clone(), s.width, overlay_style(st, s.style))),
        );
    }
    (spans, focus_x)
}

fn render_confirm(
    w: &mut Writer<'_>,
    area: CellSize,
    overlay: &OverlayModel,
    theme: &Theme,
    glyphs: &Glyphs,
) -> Option<CellPos> {
    let OverlayModel::Confirm {
        title,
        message,
        details,
        choices,
        focused,
        nav_hint,
        ..
    } = overlay
    else {
        return None;
    };
    let style = theme.overlay;
    let (mut buttons, mut focus_off) = button_spans(choices, *focused, theme, glyphs);
    let detail_rows = details.as_deref().unwrap_or(&[]);
    let nav = nav_hint.as_ref();
    let want_inner = [
        message.width,
        text_width(&buttons),
        nav.map_or(0, |n| n.width),
        detail_rows.iter().map(|d| d.width).max().unwrap_or(0),
        title.width,
    ]
    .into_iter()
    .max()
    .unwrap_or(0);
    let (bw, mut bordered) = box_width(want_inner, area.cols);
    // 縦 (04 §3.5, OQ-182): 足りないときはボタン行と案内行を優先し、本文を切る。枠も省く
    let nav_n = u16::from(nav.is_some());
    let det_n = detail_rows.len() as u16;
    let min_bordered = 2 + 1 + 1 + 1 + nav_n; // 枠・本文・空行・ボタン・案内 (詳細なし)
    let (show_message, show_details, blank, show_nav);
    if bordered && area.rows >= min_bordered {
        show_message = true;
        blank = true;
        show_nav = nav.is_some();
        show_details = det_n.min(area.rows - min_bordered);
    } else {
        bordered = false;
        blank = false;
        let mut left = area.rows;
        left = left.saturating_sub(1); // ボタン行
        show_nav = nav.is_some() && left >= 1;
        left = left.saturating_sub(u16::from(show_nav));
        show_message = left >= 1;
        left = left.saturating_sub(u16::from(show_message));
        show_details = det_n.min(left);
    }
    let rows_needed = u16::from(bordered) * 2
        + u16::from(show_message)
        + show_details
        + u16::from(blank)
        + 1
        + u16::from(show_nav);
    let b = BoxRect {
        x: (area.cols - bw) / 2,
        y: (area.rows - rows_needed) / 2,
        cols: bw,
        rows: rows_needed,
        bordered,
    };
    draw_box(w, &b, bordered.then_some(title), style, glyphs);
    // ボタン行が幅に入らない: フォーカス中のボタンと、最後の選択肢 (Esc) だけを残す (04 §3.5)
    if text_width(&buttons) > b.inner_cols() {
        let keep: Vec<Choice> = choices
            .iter()
            .filter(|c| c.id == *focused)
            .chain(choices.last().filter(|l| l.id != *focused))
            .cloned()
            .collect();
        (buttons, focus_off) = button_spans(&keep, *focused, theme, glyphs);
    }
    let mut row = 0;
    if show_message {
        draw_line(w, &b, row, &message.spans, style, glyphs);
        row += 1;
    }
    for (i, d) in detail_rows.iter().take(show_details as usize).enumerate() {
        let last_cut = i as u16 + 1 == show_details && show_details < detail_rows.len() as u16;
        let spans = if last_cut {
            // 描けなかった行がある: 最後の行を省略記号にする
            let mut s = truncate_end(
                &d.spans,
                b.inner_cols().saturating_sub(glyphs.ellipsis.width),
                &glyphs.ellipsis,
            );
            if s.last().is_none_or(|l| l.text != glyphs.ellipsis.text) {
                s.push(glyph_span(&glyphs.ellipsis, style));
            }
            s
        } else {
            d.spans.clone()
        };
        draw_line(w, &b, row, &spans, style, glyphs);
        row += 1;
    }
    if blank {
        row += 1;
    }
    let button_row = row;
    let by = b.inner_y() + button_row;
    put(
        w,
        b.inner_x(),
        by,
        b.inner_x() + b.inner_cols(),
        &buttons,
        style,
    );
    row += 1;
    if show_nav && let Some(n) = nav {
        draw_line(w, &b, row, &n.spans, style, glyphs);
    }
    // ハードウェアカーソルはフォーカス中のボタンの先頭 (04 §8.3、IME の候補窓の位置)
    Some(CellPos {
        col: b.inner_x() + focus_off,
        row: by,
    })
}

/// 入力欄の表示: カーソルの周りを幅に収める。返り値は (描く範囲の最初と最後+1、左右の続き、カーソルまでの幅)。
fn field_window(f: &InputFieldModel, width: u16) -> (usize, usize, bool, bool, u16) {
    let cur = f.cursor.saturating_sub(f.first).min(f.cells.len());
    // 左に戻れる幅: カーソルのセルと、左右の続きの印のぶんを残す
    let room = width.saturating_sub(1);
    let mut start = cur;
    let mut used = 0u16;
    while start > 0 {
        let wd = f.cells[start - 1].width;
        let more_left = start - 1 > 0 || f.first > 0;
        let reserve = u16::from(more_left);
        if used + wd + reserve > room.saturating_sub(1) && start != cur {
            break;
        }
        if used + wd + reserve > room {
            break;
        }
        used += wd;
        start -= 1;
    }
    let clip_left = start > 0 || f.first > 0;
    let off = u16::from(clip_left);
    let mut end = start;
    let mut x = off;
    while end < f.cells.len() {
        let wd = f.cells[end].width;
        let more_right = end + 1 < f.cells.len() || f.first + f.cells.len() < f.len;
        if x + wd + u16::from(more_right) > width && end >= cur {
            break;
        }
        if x + wd > width {
            break;
        }
        x += wd;
        end += 1;
    }
    let clip_right = end < f.cells.len() || f.first + f.cells.len() < f.len;
    let cursor_x = off
        + f.cells[start..cur.max(start)]
            .iter()
            .map(|s| s.width)
            .sum::<u16>();
    (start, end, clip_left, clip_right, cursor_x)
}

/// 入力欄を 1 行に描く。カーソルの、欄の左端からのセル位置を返す。
fn draw_input(
    w: &mut Writer<'_>,
    input: &InputFieldModel,
    ix: u16,
    iy: u16,
    iw: u16,
    theme: &Theme,
    glyphs: &Glyphs,
) -> u16 {
    let style = theme.overlay;
    let limit = ix + iw;
    let mut cursor_x = 0;
    if input.cells.is_empty() {
        if let Some(p) = &input.placeholder {
            let ph = truncate_end(&p.spans, iw, &glyphs.ellipsis);
            put(w, ix, iy, limit, &ph, theme.placeholder);
        }
    } else {
        let (s, e, left, right, cx) = field_window(input, iw);
        cursor_x = cx;
        let mut x = ix;
        if left {
            x = w.span(
                x,
                iy,
                limit,
                &glyph_span(&glyphs.clip_left, theme.scroll_marker),
            );
        }
        for (k, sp) in input.cells[s..e].iter().enumerate() {
            let idx = input.first + s + k;
            let selected = input.selection.as_ref().is_some_and(|r| r.contains(&idx));
            let st = if selected {
                overlay_style(style, Style::with_mods(StyleMods::REVERSE))
            } else {
                style
            };
            x = put(w, x, iy, limit, std::slice::from_ref(sp), st);
        }
        if right {
            w.span(
                limit - 1,
                iy,
                limit,
                &glyph_span(&glyphs.clip_right, theme.scroll_marker),
            );
        }
    }
    cursor_x
}

/// 検索バー (04 §8.4): `検索: [入力欄] [Aa~] 1/2`。幅が足りなければ件数 → トグルの括弧の順に省き、入力欄は最低 10 セルを残す。
fn render_search_bar(
    w: &mut Writer<'_>,
    y: u16,
    cols: u16,
    bar: &SearchBarModel,
    theme: &Theme,
    glyphs: &Glyphs,
) -> Option<CellPos> {
    let style = theme.overlay;
    let ascii = glyphs.ellipsis.width == 3;
    fill(w, 0, y, cols, style);
    let mark = match bar.case {
        CaseMode::Smart => "~",
        CaseMode::Sensitive if ascii => "*",
        CaseMode::Sensitive => "\u{2713}",
        CaseMode::Insensitive => "",
    };
    let toggle_full = format!("[Aa{mark}]");
    let toggle_short = format!("Aa{mark}");
    let label_w = text_width(&bar.label.spans);
    let count_w = bar.count_text.as_ref().map_or(0, |c| text_width(&c.spans));
    // 入力欄の最低幅を残して、入る構成を選ぶ
    let fits = |toggle: &str, count: u16| {
        let need = label_w
            + 1
            + 10
            + 1
            + toggle.chars().count() as u16
            + if count > 0 { 1 + count } else { 0 };
        need <= cols
    };
    let (toggle, show_count) = if fits(&toggle_full, count_w) {
        (toggle_full, true)
    } else if fits(&toggle_full, 0) {
        (toggle_full, false)
    } else {
        (toggle_short, false)
    };
    let toggle_w = toggle.chars().count() as u16;
    let mut x = put(w, 0, y, cols, &bar.label.spans, style);
    x = w.span(x, y, cols, &ascii_span(" ", style));
    let right_w = 1 + toggle_w + if show_count { 1 + count_w } else { 0 };
    let iw = cols.saturating_sub(x + right_w).max(1);
    let ix = x;
    let cursor_x = draw_input(w, &bar.input, ix, y, iw, theme, glyphs);
    // 右側: トグル (大小を区別しているときは反転) と件数
    let mut rx = ix + iw;
    rx = w.span(rx, y, cols, &ascii_span(" ", style));
    let tstyle = if bar.case_on {
        overlay_style(style, Style::with_mods(StyleMods::REVERSE))
    } else {
        style
    };
    let ts = Span::new(toggle.as_str(), toggle_w, tstyle);
    rx = w.span(rx, y, cols, &ts);
    if show_count && let Some(c) = &bar.count_text {
        rx = w.span(rx, y, cols, &ascii_span(" ", style));
        put(w, rx, y, cols, &c.spans, style);
    }
    Some(CellPos {
        col: ix + cursor_x.min(iw.saturating_sub(1)),
        row: y,
    })
}

/// リスト選択 (コマンドパレット、04 §8.1)。上部中央。入力欄 1 行 + 候補 (最大 12 行) + 枠。
fn render_list(
    w: &mut Writer<'_>,
    area: CellSize,
    overlay: &OverlayModel,
    theme: &Theme,
    glyphs: &Glyphs,
) -> Option<CellPos> {
    let OverlayModel::List {
        title,
        input,
        items,
        first,
        selected,
        footer,
        no_match,
    } = overlay
    else {
        return None;
    };
    let style = theme.overlay;
    let ascii = glyphs.ellipsis.width == 3;
    let (bw, mut bordered) = box_width(76, area.cols);
    let body_rows = items.len().max(1) as u16;
    let rows_for = |bordered: bool, n: u16| u16::from(bordered) * 2 + 1 + n;
    if rows_for(bordered, body_rows) > area.rows {
        bordered = false;
    }
    let n = body_rows.min(area.rows.saturating_sub(rows_for(bordered, 0)));
    let rows = rows_for(bordered, n).min(area.rows);
    let b = BoxRect {
        x: (area.cols - bw) / 2,
        y: u16::from(area.rows > rows + 1).min(area.rows - rows),
        cols: bw,
        rows,
        bordered,
    };
    draw_box(w, &b, bordered.then_some(title), style, glyphs);
    let (iw, ix, iy) = (b.inner_cols(), b.inner_x(), b.inner_y());
    let limit = ix + iw;
    // 入力欄 (空のときは placeholder)
    let cursor_x = if input.cells.is_empty() {
        if let Some(p) = &input.placeholder {
            let ph = truncate_end(&p.spans, iw, &glyphs.ellipsis);
            put(w, ix, iy, limit, &ph, theme.placeholder);
        }
        0
    } else {
        draw_input(w, input, ix, iy, iw, theme, glyphs)
    };
    if items.is_empty() {
        let t = truncate_end(&no_match.spans, iw, &glyphs.ellipsis);
        put(w, ix, iy + 1, limit, &t, theme.placeholder);
    }
    for (k, item) in items.iter().take(n as usize).enumerate() {
        let y = iy + 1 + k as u16;
        let is_sel = *first + k == *selected;
        let disabled = item.disabled_reason.is_some();
        let base = if disabled { theme.placeholder } else { style };
        let base = if is_sel {
            overlay_style(base, Style::with_mods(StyleMods::REVERSE))
        } else {
            base
        };
        fill(w, ix, y, iw, base);
        // 先頭の印 (選択中は ▶。色に頼らない)
        let mut x = if is_sel {
            let x = w.span(ix, y, limit, &glyph_span(&glyphs.focus, base));
            w.span(x, y, limit, &ascii_span(" ", base))
        } else {
            ix + 2
        };
        // 右側: 実行できないときは理由、そうでなければ CommandId とキー
        let mut right: Vec<Span> = Vec::new();
        if let Some(reason) = &item.disabled_reason {
            right.extend(reason.spans.iter().cloned());
        } else {
            if let Some(d) = &item.detail {
                right.extend(d.spans.iter().cloned());
            }
            if let Some(k) = item.keys.first() {
                if !right.is_empty() {
                    right.push(ascii_span("  ", base));
                }
                right.push(ascii_span(&key_label_full(k, ascii), base));
            }
        }
        let left_room = limit.saturating_sub(x);
        let label_w = text_width(&item.label.spans);
        // 右側は、ラベルの後に 2 セル空けて入るときだけ出す (狭いと省略)
        let right_w = text_width(&right);
        let show_right = right_w > 0 && label_w + 2 + right_w <= left_room;
        let label_room = if show_right {
            left_room - right_w - 2
        } else {
            left_room
        };
        let label = truncate_end(&item.label.spans, label_room, &glyphs.ellipsis);
        for (i, sp) in label.iter().enumerate() {
            let hit = item.match_ranges.iter().any(|r| r.contains(&i));
            let st = if hit {
                overlay_style(
                    base,
                    Style::with_mods(StyleMods::UNDERLINE | StyleMods::BOLD),
                )
            } else {
                base
            };
            x = put(w, x, y, limit, std::slice::from_ref(sp), st);
        }
        if show_right {
            put(
                w,
                limit - right_w,
                y,
                limit,
                &right,
                if is_sel { base } else { theme.placeholder },
            );
        }
    }
    // 件数は下辺に右寄せ
    if bordered && b.cols > 8 {
        let y1 = b.y + b.rows - 1;
        let fw = text_width(&footer.spans).min(b.cols.saturating_sub(8));
        let fs = truncate_end(&footer.spans, fw, &glyphs.ellipsis);
        let x = b.x + b.cols - 3 - fw;
        let x = w.span(x - 1, y1, b.x + b.cols - 1, &ascii_span(" ", style));
        let x = put(w, x, y1, b.x + b.cols - 1, &fs, style);
        w.span(x, y1, b.x + b.cols - 1, &ascii_span(" ", style));
    }
    Some(CellPos {
        col: ix + cursor_x.min(iw.saturating_sub(1)),
        row: iy,
    })
}

fn render_prompt(
    w: &mut Writer<'_>,
    area: CellSize,
    overlay: &OverlayModel,
    theme: &Theme,
    glyphs: &Glyphs,
) -> Option<CellPos> {
    let OverlayModel::Prompt {
        title,
        input,
        hint,
        error,
    } = overlay
    else {
        return None;
    };
    let style = theme.overlay;
    let (bw, mut bordered) = box_width(60, area.cols);
    let mut show_hint = hint.is_some();
    let mut show_error = error.is_some();
    let rows_for = |bordered: bool, h: bool, e: bool| {
        u16::from(bordered) * 2 + 1 + u16::from(h) + u16::from(e)
    };
    if rows_for(bordered, show_hint, show_error) > area.rows {
        show_hint = false;
    }
    if rows_for(bordered, show_hint, show_error) > area.rows {
        bordered = false;
    }
    if rows_for(bordered, show_hint, show_error) > area.rows {
        show_error = false;
    }
    let rows = rows_for(bordered, show_hint, show_error).min(area.rows);
    let b = BoxRect {
        x: (area.cols - bw) / 2,
        y: u16::from(area.rows > rows + 1).min(area.rows - rows),
        cols: bw,
        rows,
        bordered,
    };
    draw_box(w, &b, bordered.then_some(title), style, glyphs);
    let (iw, ix, iy) = (b.inner_cols(), b.inner_x(), b.inner_y());
    let cursor_x = draw_input(w, input, ix, iy, iw, theme, glyphs);
    let mut row = 1;
    if show_error && let Some(err) = error {
        draw_line(w, &b, row, &err.spans, theme.message_error, glyphs);
        row += 1;
    }
    if show_hint && let Some(h) = hint {
        draw_line(w, &b, row, &h.spans, theme.placeholder, glyphs);
    }
    Some(CellPos {
        col: ix + cursor_x.min(iw.saturating_sub(1)),
        row: iy,
    })
}

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
    if window.cols < MIN_WINDOW.cols || window.rows < MIN_WINDOW.rows {
        return render_too_small(frame, window, depth);
    }
    let mut r = render_panes(frame, depth, glyphs);
    let theme = Theme::for_depth(depth);
    let ascii = glyphs.ellipsis.width == 3;
    // 検索バーの行は編集面の下。クロームはその下に続く
    let bar_row = frame.size.rows;
    let (cols, surface_rows) = (
        frame.size.cols,
        frame.size.rows + u16::from(frame.search.is_some()),
    );
    let chrome = chrome_rows_for_surface(surface_rows);
    let area = CellSize {
        cols,
        rows: surface_rows,
    };
    let mut cursor = r.cursor;
    {
        let mut w = Writer {
            buf: &mut r.buffer,
            depth,
            clamped: r.clamped_cells,
            sanitized: r.sanitized,
        };
        let status_y = surface_rows;
        render_status(&mut w, status_y, cols, &frame.status, &theme, glyphs);
        if chrome >= 3 {
            render_message(&mut w, status_y + 1, cols, &frame.messages, &theme, glyphs);
            render_hints(
                &mut w,
                status_y + 2,
                cols,
                &frame.hints,
                &theme,
                glyphs,
                ascii,
            );
        } else if frame.messages.current.is_some() {
            // 10 行未満: メッセージはヒントバーの行に上書きする (04 §3.5)
            render_message(&mut w, status_y + 1, cols, &frame.messages, &theme, glyphs);
        } else {
            render_hints(
                &mut w,
                status_y + 1,
                cols,
                &frame.hints,
                &theme,
                glyphs,
                ascii,
            );
        }
        if let Some(bar) = &frame.search {
            cursor = render_search_bar(&mut w, bar_row, cols, bar, &theme, glyphs).or(cursor);
        }
        // オーバーレイ (前面順なので、奥から描く)
        for ov in frame.overlays.iter().rev() {
            let c = match ov {
                OverlayModel::Confirm { .. } => render_confirm(&mut w, area, ov, &theme, glyphs),
                OverlayModel::Prompt { .. } => render_prompt(&mut w, area, ov, &theme, glyphs),
                OverlayModel::List { .. } => render_list(&mut w, area, ov, &theme, glyphs),
                _ => None,
            };
            if c.is_some() {
                cursor = c;
            }
        }
        r.clamped_cells = w.clamped;
        r.sanitized = w.sanitized;
    }
    // オーバーレイがあるときの編集面のカーソルは出さない (core は primary_cursor を None にする)
    r.cursor = cursor;
    r
}
