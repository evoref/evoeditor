//! 行の内容を表示の単位 (書記素・タブ・エスケープ) に分け、表示の列とバイト位置を対応づける (docs/03 §14)。
//!
//! 単位のキャッシュと長い行の列の索引 (`long`) を使う。カーソルの列・列からバイト位置の変換もここ。

use eve_text::{
    ByteOff, ByteRange, ControlStyle, EmojiWidth, LineIdx, TextRead, WidthConfig, first_grapheme,
    grapheme_display, is_width_risky,
};

use eve_encoding::{PuaEscapeMode, unescape_char};

use crate::document::Document;

use super::CharWidth;
use super::ColIndex;
use super::cache::{LongKey, UnitsKey};

/// 空白の可視化の記号 (タブの `>` と続きの空白、全角スペースの `[]`) か。それ以外の記号は制御文字・不可視文字。
pub(super) fn is_whitespace_mark(text: &str) -> bool {
    text == ">" || text == "[]" || (!text.is_empty() && text.bytes().all(|b| b == b' '))
}

pub(super) fn width_config() -> WidthConfig {
    WidthConfig {
        tab_width: 4,
        ambiguous_wide: false,
        control_style: ControlStyle::Caret,
        emoji_width: EmojiWidth::Auto,
        show_invisible: true,
    }
}

/// 本文の表示幅の設定 (文書ごとの `display` を反映する)。
pub(crate) fn doc_width_config(doc: &Document) -> WidthConfig {
    WidthConfig {
        tab_width: doc.display.tab_width,
        ambiguous_wide: doc.display.ambiguous_wide,
        ..width_config()
    }
}

/// 表示の 1 単位 (1 書記素)。
pub(super) struct Unit {
    /// 行頭からの表示列。
    pub(super) col: usize,
    /// 描く Span。
    pub(super) parts: Parts,
    pub(super) width: usize,
    /// 行内のバイト位置。
    pub(super) byte: usize,
    pub(super) risky: bool,
}

/// 表示の単位が描く Span の列。本文の大半 (印字できる ASCII・幅 2 の通常の文字) は、その 1 文字をそのまま描くので、
/// 文字列を確保せずに持つ (PRF-02)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Parts {
    /// その 1 文字を記号でない文字として描く。幅は `Unit::width`。
    Char(char),
    /// 一般の経路で組み立てた (文字列, 幅, 記号か) の列。
    List(Vec<(String, u16, bool)>),
}

/// `Unit::parts` の (文字列, 幅, 記号か) の列。
pub(super) enum PartsIter<'a> {
    One(Option<(&'a str, u16, bool)>),
    Many(std::slice::Iter<'a, (String, u16, bool)>),
}

impl<'a> Iterator for PartsIter<'a> {
    type Item = (&'a str, u16, bool);

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            PartsIter::One(p) => p.take(),
            PartsIter::Many(it) => it.next().map(|(t, w, s)| (t.as_str(), *w, *s)),
        }
    }
}

impl Unit {
    /// 描く (文字列, 幅, 記号か) の列。`buf` は 1 文字の単位の文字列を置く場所。
    pub(super) fn parts<'a>(&'a self, buf: &'a mut [u8; 4]) -> PartsIter<'a> {
        match &self.parts {
            Parts::Char(c) => PartsIter::One(Some((c.encode_utf8(buf), self.width as u16, false))),
            Parts::List(v) => PartsIter::Many(v.iter()),
        }
    }

    /// 最初の Span が記号 (制御文字・不可視文字・空白の可視化) か。
    pub(super) fn starts_with_symbol(&self) -> bool {
        match &self.parts {
            Parts::Char(_) => false,
            Parts::List(v) => v.first().is_some_and(|p| p.2),
        }
    }

    /// 試験で経路の違う展開を比べるための、確保した形の列。
    #[cfg(test)]
    pub(super) fn parts_vec(&self) -> Vec<(String, u16, bool)> {
        let mut buf = [0u8; 4];
        self.parts(&mut buf)
            .map(|(t, w, s)| (t.to_owned(), w, s))
            .collect()
    }
}

/// 単独で幅 2・記号でない・不可視のマークなし・幅の危険なし、の通常の文字か (かな・漢字・全角の英数と主な句読点)。
/// `units` の近道の対象。曖昧幅の設定にも依らないことを、範囲の全文字を `grapheme_display` と照合するテストで確かめている。
pub(super) fn is_plain_wide(c: char) -> bool {
    matches!(c as u32,
        0x3001..=0x3002        // 、。
        | 0x300C..=0x300D      // 「」
        | 0x3041..=0x3096      // ひらがな
        | 0x30A1..=0x30FA | 0x30FC // カタカナ (・ U+30FB は除く)
        | 0x4E00..=0x9FFF      // CJK 統合漢字
        | 0xFF01..=0xFF5E) // 全角の ASCII
}

/// 溜めた文字列を通常の書記素として表示に変換して `parts` に足す。
fn flush_run(run: &mut String, parts: &mut Vec<(String, u16, bool)>, cfg: &WidthConfig) {
    if run.is_empty() {
        return;
    }
    let d = grapheme_display(run, cfg);
    if !d.base.is_empty() {
        parts.push((d.base, d.base_width, false));
    }
    if !d.marks.is_empty() {
        parts.push((d.marks, d.marks_width, true));
    }
    run.clear();
}

/// 展開の覚えを使う行の大きさ (これ未満は毎回展開する)。
const UNITS_CACHE_MIN: usize = 4096;

/// `units` の結果。長い行 (閾値以下で 4 KiB 以上) は、Revision・行・幅の設定が同じ間は覚えを使う
/// (1 回の操作で `cursor_cell` と Frame の生成が同じ行を何度も展開するのを避ける、NFR-106)。
pub(super) fn units_for(doc: &Document, line: usize, content: &str) -> std::rc::Rc<Vec<Unit>> {
    use std::rc::Rc;
    let esc = esc_enabled(doc);
    let ws = doc.display.show_whitespace;
    let cfg = doc_width_config(doc);
    if content.len() < UNITS_CACHE_MIN {
        return Rc::new(units_with(content, esc, ws, &cfg));
    }
    let key: UnitsKey = (
        doc.revision(),
        line,
        esc,
        cfg.tab_width,
        cfg.ambiguous_wide,
        ws,
    );
    if let Some((k, u)) = doc.layout_cache.borrow().units.as_ref()
        && *k == key
    {
        return u.clone();
    }
    let u = Rc::new(units_with(content, esc, ws, &cfg));
    doc.layout_cache.borrow_mut().units = Some((key, u.clone()));
    u
}

/// 1 書記素 `g` (行内のバイト位置 `off`、表示列 `col`) を表示の単位に展開する (一般の経路)。
/// `ws` は全角スペースとタブを記号で示す (`view.show_whitespace = "minimal"`)。偽なら空白のセルにする (幅は同じ)。
pub(super) fn unit_slow(
    g: &str,
    off: usize,
    col: usize,
    tab: usize,
    esc: bool,
    ws: bool,
    cfg: &WidthConfig,
) -> Unit {
    let mut parts: Vec<(String, u16, bool)> = Vec::new();
    let mut risky = false;
    if g == "\t" {
        let w = tab - col % tab;
        if ws {
            parts.push((">".to_owned(), 1, true));
            if w > 1 {
                parts.push((" ".repeat(w - 1), (w - 1) as u16, true));
            }
        } else {
            parts.push((" ".repeat(w), w as u16, false));
        }
    } else if g == "\u{3000}" {
        // 可視化しないときも、端末の全角の空白の描き方に頼らず ASCII の空白 2 つで描く
        parts.push((if ws { "[]" } else { "  " }.to_owned(), 2, ws));
    } else if esc && g.chars().any(|c| unescape_char(c).is_some()) {
        // 不正バイトのエスケープ文字は 1 文字ずつ `<0xHH>` にする (結合文字などと同じ書記素になることがある)
        let mut run = String::new();
        for c in g.chars() {
            if let Some(b) = unescape_char(c) {
                flush_run(&mut run, &mut parts, cfg);
                parts.push((format!("<0x{b:02X}>"), 6, true));
            } else {
                run.push(c);
            }
        }
        flush_run(&mut run, &mut parts, cfg);
    } else {
        let d = grapheme_display(g, cfg);
        let first = g.chars().next().unwrap_or(' ');
        let symbolic = (first as u32) < 0x20
            || (0x7F..=0x9F).contains(&(first as u32))
            || d.base.starts_with("<U+");
        if !d.base.is_empty() {
            parts.push((d.base, d.base_width, symbolic));
        }
        if !d.marks.is_empty() {
            parts.push((d.marks, d.marks_width, true));
        }
        risky = is_width_risky(g);
    }
    let width: usize = parts.iter().map(|p| p.1 as usize).sum();
    Unit {
        col,
        parts: Parts::List(parts),
        width,
        byte: off,
        risky,
    }
}

/// 行の内容を表示の単位に展開する (空白は可視化する。試験が使う)。
#[cfg(test)]
pub(super) fn units(content: &str, esc: bool, cfg: &WidthConfig) -> Vec<Unit> {
    units_with(content, esc, true, cfg)
}

/// `units` の本体。`ws` は空白の可視化 (`unit_slow`)。
pub(super) fn units_with(content: &str, esc: bool, ws: bool, cfg: &WidthConfig) -> Vec<Unit> {
    let tab = cfg.tab_width.max(1) as usize;
    // 印字できる ASCII だけの行 (本文の大半): 書記素の分割も表示の組み立ても要らない (1 文字 = 1 書記素 = 幅 1)
    if content.bytes().all(|b| (0x20..=0x7E).contains(&b)) {
        return content
            .bytes()
            .enumerate()
            .map(|(i, b)| Unit {
                col: i,
                parts: Parts::Char(b as char),
                width: 1,
                byte: i,
                risky: false,
            })
            .collect();
    }
    let mut out = Vec::new();
    let mut col = 0usize;
    let mut at = 0usize;
    while at < content.len() {
        let rest = &content[at..];
        let Some(c) = rest.chars().next() else { break };
        // 印字できる ASCII と幅 2 の通常の文字 (かな・漢字・全角の英数。本文の大半) が続くところは、その 1 文字が 1 書記素
        // (次の文字も同じ種類なら結合しない) で、書記素の分割も表示の組み立て (文字列の確保が複数回) も通さない
        let simple = |c: char| matches!(c as u32, 0x20..=0x7E) || is_plain_wide(c);
        if simple(c) && rest[c.len_utf8()..].chars().next().is_none_or(simple) {
            let w: u16 = if c.is_ascii() { 1 } else { 2 };
            out.push(Unit {
                col,
                parts: Parts::Char(c),
                width: usize::from(w),
                byte: at,
                risky: false,
            });
            col += usize::from(w);
            at += c.len_utf8();
            continue;
        }
        let g = first_grapheme(rest);
        let off = at;
        at += g.len();
        let u = unit_slow(g, off, col, tab, esc, ws, cfg);
        col += u.width;
        out.push(u);
    }
    out
}

/// (行頭, 行の内容)。長大行には使わない (行全体を文字列にする。長大行は `long_index`)。
pub(super) fn line_content(doc: &Document, line: usize) -> (usize, String) {
    let t = doc.text();
    let l = LineIdx(line);
    let (ls, le) = (t.line_to_byte(l).0, t.line_end_byte(l).0);
    (
        ls,
        t.slice_to_string(ByteRange::new(ByteOff(ls), ByteOff(le))),
    )
}

/// 行 `line` が長大行 (バイト長が文書の `format.long_line_threshold` を超える、03 §11.5.1) か。行インデックスから O(1)。
pub(crate) fn is_long_line(doc: &Document, line: usize) -> bool {
    let t = doc.text();
    let l = LineIdx(line);
    t.line_end_byte(l).0 - t.line_to_byte(l).0 > doc.format.long_line_threshold
}

/// 覚えておく長大行の索引の数 (画面に複数の長大行が見えていても、Frame の生成とカーソルの計算で作り直さない)。
const LONG_CACHE_LINES: usize = 4;

/// 行 `line` が長大行 (バイト長が文書の閾値を超える、03 §11.5.1) なら、その (行頭, 列の索引)。
/// 索引は Revision・行・幅の設定が同じ間は覚えを使う (編集のたびに 1 回走査し直す)。
pub(super) fn long_index(doc: &Document, line: usize) -> Option<(usize, std::rc::Rc<ColIndex>)> {
    use std::rc::Rc;
    if !is_long_line(doc, line) {
        return None;
    }
    let t = doc.text();
    let l = LineIdx(line);
    let (ls, le) = (t.line_to_byte(l).0, t.line_end_byte(l).0);
    let esc = esc_enabled(doc);
    let cfg = doc_width_config(doc);
    let key: LongKey = (doc.revision(), line, esc, cfg.tab_width, cfg.ambiguous_wide);
    let mut cache = doc.layout_cache.borrow_mut();
    let entries = &mut cache.long;
    // 古い Revision の索引は使えないので捨てる
    entries.retain(|(k, _)| k.0 == key.0);
    if let Some((_, ix)) = entries.iter().find(|(k, _)| *k == key) {
        return Some((ls, ix.clone()));
    }
    let ix = Rc::new(ColIndex::build(t, ls, le, CharWidth::new(esc, cfg)));
    if entries.len() >= LONG_CACHE_LINES {
        entries.remove(0);
    }
    entries.push((key, ix.clone()));
    Some((ls, ix))
}

/// 長大行の、表示列 `left` から `left + cols` までにかかる char の表示の単位 (行全体は展開しない)。
/// 1 char を 1 書記素として `unit_slow` で展開する (幅は索引と同じ)。
pub(super) fn long_units(
    doc: &Document,
    ls: usize,
    ix: &ColIndex,
    left: usize,
    cols: usize,
) -> Vec<Unit> {
    let cfg = doc_width_config(doc);
    let (esc, ws) = (esc_enabled(doc), doc.display.show_whitespace);
    let tab = cfg.tab_width.max(1) as usize;
    let t = doc.text();
    let mut out = Vec::new();
    let mut buf = [0u8; 4];
    // 右端の列にかかる単位まで (その次の単位で、右が切れていることが分かる)
    ix.walk(t, ls, ix.at_col(t, ls, left), |at, c, w| {
        if at.col > left + cols {
            return false;
        }
        out.push(if (' '..='~').contains(&c) {
            Unit {
                col: at.col,
                parts: Parts::Char(c),
                width: w,
                byte: at.byte,
                risky: false,
            }
        } else {
            unit_slow(c.encode_utf8(&mut buf), at.byte, at.col, tab, esc, ws, &cfg)
        });
        true
    });
    out
}

fn esc_enabled(doc: &Document) -> bool {
    doc.format.pua_escape == PuaEscapeMode::Enabled
}

/// 行内のバイト位置 `byte` にあるカーソルの (表示列, そのセルの幅)。行末では (行の幅, 1)。
pub(crate) fn cursor_cell(doc: &Document, line: usize, byte: usize) -> (usize, usize) {
    if let Some((ls, ix)) = long_index(doc, line) {
        let (p, w) = ix.at_byte(doc.text(), ls, byte);
        return (p.col, w);
    }
    let (_, content) = line_content(doc, line);
    let us = units_for(doc, line, &content);
    match us.iter().find(|u| u.byte >= byte) {
        Some(u) => (u.col, u.width.max(1)),
        None => (us.last().map_or(0, |u| u.col + u.width), 1),
    }
}

/// 長大行なら、行内の位置 `byte` より前の char 数 (ステータスバーの桁。書記素 = char とみなす、03 §11.5.1)。
pub(crate) fn long_line_chars(doc: &Document, line: usize, byte: usize) -> Option<usize> {
    let (ls, ix) = long_index(doc, line)?;
    Some(ix.at_byte(doc.text(), ls, byte).0.chars)
}

/// 行内のバイト位置 `byte` にあるカーソルの表示列 (行頭から)。移動の目標列もこの表示と同じ幅で数える。
pub(crate) fn cursor_col(doc: &Document, line: usize, byte: usize) -> usize {
    cursor_cell(doc, line, byte).0
}

/// 表示列 `dcol` に対応する行内のバイト位置。セルの途中は、その単位の前に寄せる (`Bias::Before`)。行の幅を超えれば行末。
pub(crate) fn byte_at_col(doc: &Document, line: usize, dcol: usize) -> usize {
    if let Some((ls, ix)) = long_index(doc, line) {
        return ix.at_col(doc.text(), ls, dcol).byte;
    }
    let (_, content) = line_content(doc, line);
    let us = units_for(doc, line, &content);
    us.iter()
        .find(|u| dcol < u.col + u.width)
        .map_or(content.len(), |u| u.byte)
}

#[cfg(test)]
mod tests {
    use eve_text::grapheme_indices;

    use super::*;

    /// `units` の ASCII の近道は、一般の経路 (書記素の表示の組み立て) と同じ結果になる前提に立つ:
    /// 印字できる ASCII 1 文字は、幅 1・マークなし・記号でない・幅の危険なし。
    #[test]
    fn 印字できる_ascii_の近道は一般の経路と同じ結果になる() {
        for ambiguous_wide in [false, true] {
            let cfg = WidthConfig {
                ambiguous_wide,
                ..width_config()
            };
            for b in 0x20u8..=0x7E {
                let g = (b as char).to_string();
                let d = grapheme_display(&g, &cfg);
                assert_eq!(d.base, g, "{b:#x}");
                assert_eq!(d.base_width, 1, "{b:#x}");
                assert!(d.marks.is_empty(), "{b:#x}");
                assert!(!d.base.starts_with("<U+"), "{b:#x}");
                assert!(!is_width_risky(&g), "{b:#x}");
            }
        }
    }

    #[test]
    fn 幅_2_の通常の文字の近道は範囲の全文字で一般の経路と同じ結果になる() {
        for ambiguous_wide in [false, true] {
            let cfg = WidthConfig {
                ambiguous_wide,
                ..width_config()
            };
            let mut checked = 0;
            for v in 0x3000u32..=0xFFFF {
                let Some(c) = char::from_u32(v).filter(|&c| is_plain_wide(c)) else {
                    continue;
                };
                let g = c.to_string();
                let d = grapheme_display(&g, &cfg);
                assert_eq!(d.base, g, "{v:#x}");
                assert_eq!((d.base_width, d.marks_width), (2, 0), "{v:#x}");
                assert!(d.marks.is_empty() && !d.base.starts_with("<U+"), "{v:#x}");
                assert!(!is_width_risky(&g), "{v:#x}");
                checked += 1;
            }
            assert!(checked > 20_000, "{checked}");
        }
    }

    #[test]
    fn 近道の展開は列とバイト位置が一般の経路と同じ() {
        let cfg = width_config();
        // ASCII だけの行 (近道) と、同じ文字列に非 ASCII を足して一般の経路を通した行の、共通部分の展開が同じ
        let ascii = "the quick brown fox 0123456789";
        let fast = units(ascii, false, &cfg);
        let mixed = units(&format!("{ascii}日"), false, &cfg);
        assert_eq!(fast.len() + 1, mixed.len());
        for (a, b) in fast.iter().zip(&mixed) {
            assert_eq!(
                (a.col, a.width, a.byte, a.risky, a.parts_vec()),
                (b.col, b.width, b.byte, b.risky, b.parts_vec())
            );
        }
        assert!(units("", false, &cfg).is_empty());
    }

    /// 基準: 全ての書記素を一般の経路で展開する。
    fn units_reference(content: &str, esc: bool, cfg: &WidthConfig) -> Vec<Unit> {
        let tab = cfg.tab_width.max(1) as usize;
        let mut col = 0usize;
        grapheme_indices(content)
            .into_iter()
            .map(|(off, g)| {
                let u = unit_slow(g, off, col, tab, esc, true, cfg);
                col += u.width;
                u
            })
            .collect()
    }

    #[test]
    fn 近道の展開は全ての書記素を一般の経路で展開した結果と同じになる() {
        // 決定的な擬似乱数で、近道の文字と、結合・絵文字・ハングル・国旗・タブ・全角空白・制御文字・エスケープを混ぜる
        let pieces: [&str; 24] = [
            "a",
            "Z",
            " ",
            "~",
            "あ",
            "ア",
            "漢",
            "字",
            "、",
            "「",
            "Ａ",
            "ー",
            "・",
            "e\u{301}",
            "\u{301}",
            "😀",
            "👨\u{200D}👩",
            "🇯🇵",
            "한",
            "\t",
            "\u{3000}",
            "\u{7}",
            "\u{85}",
            "\u{10FF80}",
        ];
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for ambiguous_wide in [false, true] {
            for esc in [false, true] {
                let cfg = WidthConfig {
                    ambiguous_wide,
                    ..width_config()
                };
                for _ in 0..3000 {
                    let mut line = String::new();
                    for _ in 0..(next() % 14) {
                        line.push_str(pieces[(next() % 24) as usize]);
                    }
                    let fast = units(&line, esc, &cfg);
                    let slow = units_reference(&line, esc, &cfg);
                    assert_eq!(fast.len(), slow.len(), "{line:?}");
                    for (a, b) in fast.iter().zip(&slow) {
                        assert_eq!(
                            (a.col, a.width, a.byte, a.risky, a.parts_vec()),
                            (b.col, b.width, b.byte, b.risky, b.parts_vec()),
                            "{line:?}"
                        );
                    }
                }
            }
        }
    }
}
