//! 表示幅と不可視文字の可視化 (docs/03-core-design.md §13.2)。幅は書記素単位で、`unicode-width` (UAX #11) を土台にする。
//!
//! Unicode の General Category / Emoji のプロパティ表は持たない (依存は `unicode-width` と `unicode-segmentation` だけ)。
//! 次の判定は、該当する範囲を列挙した近似である:
//! - Cf (書式文字): 既知の Cf の集合 (`cf_mark`)
//! - Extended_Pictographic / Emoji_Presentation: 絵文字の主なブロック (`is_pictographic`)
//! - 未割当・新規のコードポイントを `is_width_risky` で拾うこと: できない (表が要る)。端末との不一致は 04 の補正が受ける
//!
//! 不正バイトのエスケープ (`pua_escape` の文書の `U+10FF00..FF`、`<0xHH>` = 6 セル) は文書の状態に依存するので、
//! 幅関数ではなく、文書の情報を持つレイアウト (C5) が扱う。

use unicode_width::UnicodeWidthStr;

/// 表示幅の上限 (可視記号を含む。§13.2 の「最大 48」)。
pub const MAX_GRAPHEME_WIDTH: u8 = 48;

/// 1 書記素に並べる可視化の記号の上限。残りは `<+N>` にまとめる。
const MAX_MARKS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlStyle {
    /// `^X` (2 セル)。C0 と DEL にだけ効く。
    Caret,
    /// `<U+001B>` (8 セル)。
    Hex,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmojiWidth {
    /// 端末補正 (04) が決めた値。決まらなければ 2。eve-text は補正を知らないので 2 として扱う。
    Auto,
    One,
    Two,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WidthConfig {
    pub tab_width: u8,
    /// 東アジアの曖昧幅 (A) を 2 にするか。
    pub ambiguous_wide: bool,
    pub control_style: ControlStyle,
    pub emoji_width: EmojiWidth,
    /// 不可視文字を可視化する。`false` なら不可視文字の幅は 0。
    pub show_invisible: bool,
}

impl Default for WidthConfig {
    fn default() -> WidthConfig {
        WidthConfig {
            tab_width: 4,
            ambiguous_wide: false,
            control_style: ControlStyle::Caret,
            emoji_width: EmojiWidth::Auto,
            show_invisible: true,
        }
    }
}

// ---------- 不可視文字 ----------

/// Cf と、Zl / Zp の可視化の記号。略号のあるものはその略号、他は `<U+XXXX>`。
fn cf_mark(c: char) -> Option<String> {
    let abbr = match c {
        '\u{200B}' => "ZWSP",
        '\u{2060}' => "WJ",
        '\u{FEFF}' => "BOM",
        '\u{00AD}' => "SHY",
        '\u{200E}' => "LRM",
        '\u{200F}' => "RLM",
        '\u{061C}' => "ALM",
        '\u{202A}' => "LRE",
        '\u{202B}' => "RLE",
        '\u{202C}' => "PDF",
        '\u{202D}' => "LRO",
        '\u{202E}' => "RLO",
        '\u{2066}' => "LRI",
        '\u{2067}' => "RLI",
        '\u{2068}' => "FSI",
        '\u{2069}' => "PDI",
        '\u{200C}' => "ZWNJ",
        '\u{200D}' => "ZWJ",
        '\u{2028}' => "LS",
        '\u{2029}' => "PS",
        _ => {
            let is_cf = matches!(c,
                '\u{0600}'..='\u{0605}' | '\u{06DD}' | '\u{070F}' | '\u{0890}'..='\u{0891}' | '\u{08E2}'
                | '\u{180E}' | '\u{2061}'..='\u{2064}' | '\u{206A}'..='\u{206F}' | '\u{FFF9}'..='\u{FFFB}'
                | '\u{110BD}' | '\u{110CD}' | '\u{13430}'..='\u{1343F}' | '\u{1BCA0}'..='\u{1BCA3}'
                | '\u{1D173}'..='\u{1D17A}' | '\u{E0001}' | '\u{E0020}'..='\u{E007F}');
            return is_cf.then(|| format!("<U+{:04X}>", c as u32));
        }
    };
    Some(format!("<{abbr}>"))
}

/// 異体字セレクタの番号 (1〜256)。
fn vs_number(c: char) -> Option<u32> {
    match c {
        '\u{FE00}'..='\u{FE0F}' => Some(c as u32 - 0xFE00 + 1),
        '\u{E0100}'..='\u{E01EF}' => Some(c as u32 - 0xE0100 + 17),
        _ => None,
    }
}

/// Extended_Pictographic の近似 (絵文字の主なブロック)。
fn is_pictographic(c: char) -> bool {
    matches!(c,
        '\u{00A9}' | '\u{00AE}' | '\u{203C}' | '\u{2049}' | '\u{2122}' | '\u{2139}' | '\u{2194}'..='\u{21AA}'
        | '\u{231A}'..='\u{23FF}' | '\u{24C2}' | '\u{25AA}'..='\u{25FE}' | '\u{2600}'..='\u{27BF}'
        | '\u{2934}' | '\u{2935}' | '\u{2B05}'..='\u{2B55}' | '\u{3030}' | '\u{303D}' | '\u{3297}' | '\u{3299}'
        | '\u{1F000}'..='\u{1F2FF}' | '\u{1F300}'..='\u{1FAFF}')
}

fn is_emoji_modifier(c: char) -> bool {
    matches!(c, '\u{1F3FB}'..='\u{1F3FF}')
}

/// 書記素に含まれる不可視文字の可視化の記号を、コードポイントの順に返す (§13.2)。
/// 絵文字 ZWJ 列の中の ZWJ、絵文字タグ列の中のタグ文字、基底文字の直後の異体字セレクタは可視化しない。
pub fn invisible_marks(g: &str) -> Vec<String> {
    let cs: Vec<char> = g.chars().collect();
    let emoji_tag_seq = cs.first() == Some(&'\u{1F3F4}') && cs.last() == Some(&'\u{E007F}');
    let mut out = Vec::new();
    for (i, &c) in cs.iter().enumerate() {
        if let Some(n) = vs_number(c) {
            let lone = i == 0 || vs_number(cs[i - 1]).is_some();
            if lone {
                out.push(format!("<VS{n}>"));
            }
            continue;
        }
        if c == '\u{200D}' {
            // 前後が絵文字 (VS・肌色修飾を挟んでよい) なら、絵文字 ZWJ 列の中
            let before = cs[..i]
                .iter()
                .rev()
                .find(|&&x| vs_number(x).is_none() && !is_emoji_modifier(x));
            let after = cs[i + 1..]
                .iter()
                .find(|&&x| vs_number(x).is_none() && !is_emoji_modifier(x));
            if matches!((before, after), (Some(&b), Some(&a)) if is_pictographic(b) && is_pictographic(a))
            {
                continue;
            }
        }
        if emoji_tag_seq && matches!(c, '\u{E0020}'..='\u{E007F}') {
            continue;
        }
        if let Some(m) = cf_mark(c) {
            out.push(m);
        }
    }
    out
}

/// 記号を並べた表示文字列と、その長さ (セル数)。4 個を超える分は `<+N>`。
fn marks_text(marks: &[String]) -> String {
    let mut s: String = marks.iter().take(MAX_MARKS).map(String::as_str).collect();
    if marks.len() > MAX_MARKS {
        s.push_str(&format!("<+{}>", marks.len() - MAX_MARKS));
    }
    s
}

/// 書記素の可視化した表示文字列 (不可視文字の記号だけ。基底の文字は含まない)。
pub fn invisible_marks_text(g: &str) -> String {
    marks_text(&invisible_marks(g))
}

// ---------- 幅 ----------

fn is_c0_control(c: char) -> bool {
    (c < ' ' && !matches!(c, '\t' | '\n' | '\r')) || c == '\u{7F}'
}

fn is_c1_control(c: char) -> bool {
    matches!(c, '\u{80}'..='\u{9F}')
}

/// 絵文字の表示 (幅 2 が既定) を持つ書記素か。地域指示記号、または絵文字ブロックの文字が
/// 絵文字の表示になるもの (VS16・肌色修飾・ZWJ 列・UAX #11 で幅 2) を指す。テキスト表示の絵文字 (幅 1) は含まない。
fn is_emoji_like(g: &str) -> bool {
    if g.chars().any(|c| matches!(c, '\u{1F1E6}'..='\u{1F1FF}')) {
        return true;
    }
    let Some(base) = g.chars().next() else {
        return false;
    };
    if !is_pictographic(base) {
        return false;
    }
    g.chars()
        .any(|c| c == '\u{FE0F}' || c == '\u{200D}' || is_emoji_modifier(c))
        || UnicodeWidthStr::width(base.encode_utf8(&mut [0u8; 4])) == 2
}

/// 結合文字 (Mn / Me の主なブロック) か。孤立しているときに ◌ を足す判定に使う。
fn is_combining(c: char) -> bool {
    matches!(c,
        '\u{0300}'..='\u{036F}' | '\u{0483}'..='\u{0489}' | '\u{0591}'..='\u{05BD}' | '\u{064B}'..='\u{065F}'
        | '\u{1AB0}'..='\u{1AFF}' | '\u{1DC0}'..='\u{1DFF}' | '\u{20D0}'..='\u{20FF}' | '\u{3099}' | '\u{309A}'
        | '\u{FE20}'..='\u{FE2F}')
}

/// 書記素 `g` の表示幅 (セル数)。0 になる書記素は作らない (幅 0 のセルにカーソルを置けないため)。
/// タブは列に依存するので、呼び出し側が列から計算する (ここでは `tab_width` を返す)。改行は 0。
pub fn grapheme_width(g: &str, cfg: &WidthConfig) -> u8 {
    let Some(first) = g.chars().next() else {
        return 0;
    };
    if first == '\t' {
        return cfg.tab_width;
    }
    if matches!(first, '\n' | '\r') {
        return 0;
    }
    if is_c0_control(first) {
        return match cfg.control_style {
            ControlStyle::Caret => 2,
            ControlStyle::Hex => 8,
        };
    }
    if is_c1_control(first) {
        return 8;
    }

    let marks = invisible_marks(g);
    let marks_len = if cfg.show_invisible {
        marks_text(&marks).chars().count()
    } else {
        0
    };
    // 基底の幅: 可視化の対象を除いた残りの幅
    let rest: String = g
        .chars()
        .filter(|&c| cf_mark(c).is_none() && vs_number(c).is_none())
        .collect();
    let base = if rest.is_empty() {
        0
    } else {
        base_width(g, &rest, cfg)
    };
    let total = base + marks_len;
    if total > 0 {
        return total.min(MAX_GRAPHEME_WIDTH as usize) as u8;
    }
    if !marks.is_empty() {
        return 0; // show_invisible = false の不可視文字
    }
    // どれにも当たらず幅 0: 先頭のコードポイントの `<U+XXXX>`
    format!("<U+{:04X}>", first as u32).chars().count() as u8
}

/// 可視化の対象を除いた部分 `rest` の幅 (`g` は元の書記素。絵文字の判定に使う)。
fn base_width(g: &str, rest: &str, cfg: &WidthConfig) -> usize {
    let first = rest.chars().next().unwrap_or(' ');
    if is_emoji_like(g) && UnicodeWidthStr::width(rest) >= 1 {
        return match cfg.emoji_width {
            EmojiWidth::One => 1,
            EmojiWidth::Two | EmojiWidth::Auto => 2,
        };
    }
    let w = if cfg.ambiguous_wide {
        UnicodeWidthStr::width_cjk(rest)
    } else {
        UnicodeWidthStr::width(rest)
    };
    if w == 0 && is_combining(first) {
        return 1; // 孤立結合文字: ◌ + 結合
    }
    w
}

/// 1 行の文字列の表示幅 (セル数)。タブは次のタブストップまで進める。改行は含めない行の内容を渡す。
pub fn str_display_width(s: &str, cfg: &WidthConfig) -> usize {
    use unicode_segmentation::UnicodeSegmentation;
    let tab = cfg.tab_width.max(1) as usize;
    s.graphemes(true).fold(0, |col, g| {
        if g == "	" {
            col + tab - col % tab
        } else {
            col + grapheme_width(g, cfg) as usize
        }
    })
}

/// 端末との幅の不一致が起きやすい書記素か (§13.2)。`RenderRow.width_risky` の素材。
/// 未割当・新規のコードポイントは表がないので判定できない (04 の補正表が受ける)。
pub fn is_width_risky(g: &str) -> bool {
    g.chars().any(|c| {
        matches!(c,
            '\u{200D}' | '\u{FE0E}' | '\u{FE0F}'                                  // ZWJ・VS15 / VS16
            | '\u{1F1E6}'..='\u{1F1FF}'                                          // 地域指示記号
            | '\u{1F3FB}'..='\u{1F3FF}'                                          // 肌色修飾
            | '\u{E000}'..='\u{F8FF}' | '\u{F0000}'..='\u{FFFFD}' | '\u{100000}'..='\u{10FFFD}') // 私用領域
    }) || is_emoji_like(g)
        || g.chars().any(|c| {
            let mut buf = [0u8; 4];
            let s: &str = c.encode_utf8(&mut buf);
            UnicodeWidthStr::width(s) != UnicodeWidthStr::width_cjk(s) // 東アジアの曖昧幅
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(g: &str) -> u8 {
        grapheme_width(g, &WidthConfig::default())
    }

    #[test]
    fn 基本の幅() {
        assert_eq!(w("a"), 1);
        assert_eq!(w("あ"), 2);
        assert_eq!(w("ｱ"), 1, "半角カナ");
        assert_eq!(w("한"), 2);
        assert_eq!(w("e\u{301}"), 1, "結合文字は基底に吸収");
        assert_eq!(w("\t"), 4);
        assert_eq!(w("\n"), 0);
    }

    #[test]
    fn 曖昧幅は設定に従う() {
        let wide = WidthConfig {
            ambiguous_wide: true,
            ..WidthConfig::default()
        };
        assert_eq!(w("±"), 1);
        assert_eq!(grapheme_width("±", &wide), 2);
        assert_eq!(grapheme_width("あ", &wide), 2);
        assert_eq!(grapheme_width("a", &wide), 1);
    }

    #[test]
    fn 絵文字の幅() {
        assert_eq!(w("😀"), 2);
        assert_eq!(w("👨\u{200D}👩\u{200D}👧"), 2, "ZWJ 列は 1 つで 2");
        assert_eq!(w("🇯🇵"), 2);
        assert_eq!(w("🇯"), 2, "単独の指示記号も 2");
        assert_eq!(w("👍🏽"), 2);
        assert_eq!(w("❤\u{FE0F}"), 2, "VS16 は 2");
        let one = WidthConfig {
            emoji_width: EmojiWidth::One,
            ..WidthConfig::default()
        };
        assert_eq!(grapheme_width("😀", &one), 1);
        assert_eq!(grapheme_width("あ", &one), 2, "絵文字以外は変わらない");
    }

    #[test]
    fn 制御文字() {
        assert_eq!(w("\u{1B}"), 2);
        assert_eq!(w("\u{7F}"), 2);
        assert_eq!(w("\u{0B}"), 2, "VT も ^K");
        let hex = WidthConfig {
            control_style: ControlStyle::Hex,
            ..WidthConfig::default()
        };
        assert_eq!(grapheme_width("\u{1B}", &hex), 8);
        assert_eq!(
            grapheme_width("\u{85}", &WidthConfig::default()),
            8,
            "C1 は常に 16 進"
        );
        assert_eq!(grapheme_width("\u{85}", &hex), 8);
    }

    #[test]
    fn 不可視文字は可視化して幅を持つ() {
        assert_eq!(invisible_marks("\u{200B}"), ["<ZWSP>"]);
        assert_eq!(w("\u{200B}"), 6);
        assert_eq!(w("\u{FEFF}"), 5);
        assert_eq!(w("\u{202E}"), 5, "<RLO>");
        assert_eq!(invisible_marks("a\u{200C}"), ["<ZWNJ>"]);
        assert_eq!(w("a\u{200C}"), 1 + 6, "基底の後ろに記号");
        assert_eq!(invisible_marks("\u{2028}"), ["<LS>"]);
        assert_eq!(w("\u{2028}"), 4);
        assert_eq!(
            invisible_marks("\u{2062}"),
            ["<U+2062>"],
            "略号のない Cf は 16 進"
        );
        assert_eq!(invisible_marks("\u{E0041}"), ["<U+E0041>"]);
        let off = WidthConfig {
            show_invisible: false,
            ..WidthConfig::default()
        };
        assert_eq!(grapheme_width("\u{200B}", &off), 0);
        assert_eq!(grapheme_width("a\u{200C}", &off), 1);
    }

    #[test]
    fn 絵文字の中の_zwj_とタグと異体字は可視化しない() {
        assert!(invisible_marks("👨\u{200D}👩").is_empty());
        assert!(
            invisible_marks("👨🏽\u{200D}👩\u{FE0F}").is_empty(),
            "肌色修飾・VS を挟んでも絵文字列"
        );
        assert_eq!(
            invisible_marks("a\u{200D}b"),
            ["<ZWJ>"],
            "絵文字でない ZWJ は示す"
        );
        assert_eq!(invisible_marks("\u{200D}"), ["<ZWJ>"]);
        // 旗のタグ列 (🏴 + タグ + 取消タグ)
        let flag = "\u{1F3F4}\u{E0067}\u{E0062}\u{E0065}\u{E006E}\u{E0067}\u{E007F}";
        assert!(invisible_marks(flag).is_empty());
        assert_eq!(w(flag), 2);
        // 基底の直後の VS は示さない。単独の VS は示す
        assert!(invisible_marks("a\u{FE0F}").is_empty());
        assert_eq!(invisible_marks("\u{FE01}"), ["<VS2>"]);
        assert_eq!(invisible_marks("\u{E0100}"), ["<VS17>"]);
        assert_eq!(
            invisible_marks("\u{FE0F}\u{FE0F}"),
            ["<VS16>", "<VS16>"],
            "直前も VS なら示す"
        );
    }

    #[test]
    fn 記号は_4_個までで_残りはまとめる() {
        let g = "a\u{200C}\u{200C}\u{200C}\u{200C}\u{200C}\u{200C}";
        assert_eq!(invisible_marks_text(g), "<ZWNJ><ZWNJ><ZWNJ><ZWNJ><+2>");
        assert!(w(g) <= MAX_GRAPHEME_WIDTH);
        assert_eq!(w(g) as usize, 1 + invisible_marks_text(g).chars().count());
    }

    #[test]
    fn 幅_0_の書記素はセルを作る() {
        assert_eq!(w("\u{301}"), 1, "孤立結合文字は ◌ + 結合");
        // 幅 0 で他に当たらないもの (モンゴル語の自由異体字セレクタ U+180B は Mn だが可視化の対象外)
        assert_eq!(w("\u{180B}"), "<U+180B>".chars().count() as u8);
        assert_eq!(
            w("\u{34F}"),
            1,
            "CGJ (Mn) は孤立結合文字の扱い (v0.1 は可視化しない、OQ-197)"
        );
    }

    #[test]
    fn 幅危険書記素() {
        assert!(!is_width_risky("a"));
        assert!(!is_width_risky("あ"));
        assert!(is_width_risky("👨\u{200D}👩"), "ZWJ");
        assert!(is_width_risky("❤\u{FE0F}"), "VS16");
        assert!(is_width_risky("🇯🇵"), "国旗");
        assert!(is_width_risky("👍🏽"), "肌色修飾");
        assert!(is_width_risky("😀"), "絵文字の表示");
        assert!(
            is_width_risky("±"),
            "曖昧幅 (unicode-width は Greek / Cyrillic を曖昧幅にしない)"
        );
        assert!(is_width_risky("\u{E0B0}"), "私用領域");
    }

    #[test]
    fn レビューで見つかった境界の例() {
        assert_eq!(w("a\u{200D}"), 1 + 5, "絵文字でない基底の ZWJ は示す");
        let one = WidthConfig {
            emoji_width: EmojiWidth::One,
            ..WidthConfig::default()
        };
        assert_eq!(grapheme_width("あ\u{200D}", &one), 2 + 5);
        assert_eq!(w("\u{1F5A5}"), 1, "テキスト表示の絵文字は幅 1");
        assert!(!is_width_risky("\u{1F5A5}"));
        assert_eq!(
            grapheme_width("\u{2B50}", &one),
            1,
            "BMP の絵文字表示 (幅 2) も emoji_width に従う"
        );
        assert!(is_width_risky("\u{2B50}"));
        let off = WidthConfig {
            show_invisible: false,
            ..WidthConfig::default()
        };
        assert_eq!(grapheme_width("\u{180B}", &off), 8, "幅 0 のセルは作らない");
        assert_eq!(
            invisible_marks("\u{1F3F4}\u{E0067}\u{E0062}"),
            ["<U+E0067>", "<U+E0062>"],
            "終端のないタグ列は示す"
        );
    }
}
