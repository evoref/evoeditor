//! 長大行 (行のバイト長が文書の `format.long_line_threshold` を超える行) の列の索引 (char 近似モード、03 §11.5.1)。
//!
//! 行全体を文字列にも表示の単位にも展開しない。行頭から char を 1 回走査し、64 KiB ごとに
//! (行内のバイト位置, 表示列, char 数) を記録する。位置 ↔ 表示列は、手前の記録点からの差分走査 (64 KiB 以内) で求める。
//! 書記素 = char とみなす: 1 char の幅は、その char だけを書記素とした通常の行の幅 (`unit_slow`) と同じ。

// 呼び出し側 (Frame の生成・カーソル・移動) の切り替えは次の PR (03 §11.5.1 の「実装の分け方」の 2)
#![cfg_attr(not(test), allow(dead_code))]

use eve_encoding::unescape_char;
use eve_text::{ByteOff, ByteRange, TextRead, WidthConfig, grapheme_width};

use super::is_plain_wide;

/// 記録点の間隔 (バイト)。
const STEP: usize = 64 * 1024;

/// 1 char の表示幅の決め方 (文書の表示の設定から作る)。
#[derive(Clone, Copy)]
pub(super) struct CharWidth {
    pub tab: usize,
    /// 不正バイトのエスケープ文字を `<0xHH>` で示す (`format.pua_escape`)。
    pub esc: bool,
    pub cfg: WidthConfig,
}

impl CharWidth {
    pub fn new(esc: bool, cfg: WidthConfig) -> Self {
        Self {
            tab: cfg.tab_width.max(1) as usize,
            esc,
            cfg,
        }
    }

    /// 表示列 `col` にある char `c` の幅 (0 にはならない)。
    pub fn of(&self, c: char, col: usize) -> usize {
        match c {
            ' '..='~' => 1,
            '\t' => self.tab - col % self.tab,
            '\u{3000}' => 2,
            c if is_plain_wide(c) => 2,
            c if self.esc && unescape_char(c).is_some() => 6,
            c => {
                let mut buf = [0u8; 4];
                let g: &str = c.encode_utf8(&mut buf);
                match grapheme_width(g, &self.cfg) {
                    0 => {
                        let d = eve_text::grapheme_display(g, &self.cfg);
                        usize::from(d.base_width + d.marks_width).max(1)
                    }
                    w => usize::from(w),
                }
            }
        }
    }
}

/// 行内のある位置の (表示列, 行頭からの char 数)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ColPoint {
    pub byte: usize,
    pub col: usize,
    pub chars: usize,
}

/// 1 行の列の索引。位置はすべて行頭からのバイト。
pub(super) struct ColIndex {
    /// 記録点 (先頭は行頭)。`byte` は char の境界で昇順。
    points: Vec<ColPoint>,
    /// 行末 (行の内容のバイト長・表示幅・char 数)。
    end: ColPoint,
    rule: CharWidth,
}

/// `text` の `[from, to)` の char を、行頭からの位置 `start` (表示列・char 数を含む) から順に `f(位置, char, 幅)` に渡す。
/// `f` が `false` を返したら、その char の手前で止めてその位置を返す。最後まで進めば `to` の位置を返す。
fn scan(
    text: &dyn TextRead,
    ls: usize,
    start: ColPoint,
    to: usize,
    rule: &CharWidth,
    mut f: impl FnMut(ColPoint, char, usize) -> bool,
) -> ColPoint {
    let mut at = start;
    let range = ByteRange::new(ByteOff(ls + start.byte), ByteOff(ls + to));
    for chunk in text.chunks(range) {
        for c in chunk.chars() {
            if at.byte >= to {
                return at;
            }
            let w = rule.of(c, at.col);
            if !f(at, c, w) {
                return at;
            }
            at.byte += c.len_utf8();
            at.col += w;
            at.chars += 1;
        }
    }
    at
}

impl ColIndex {
    /// 行 (行頭 `ls`、内容の末尾 `le`) を 1 回走査して作る。
    pub fn build(text: &dyn TextRead, ls: usize, le: usize, rule: CharWidth) -> Self {
        let mut points = vec![ColPoint::default()];
        let mut next = STEP;
        let end = scan(text, ls, ColPoint::default(), le - ls, &rule, |at, _, _| {
            if at.byte >= next {
                points.push(at);
                next = at.byte + STEP;
            }
            true
        });
        Self { points, end, rule }
    }

    /// 行末の (バイト長, 表示幅, char 数)。
    pub fn end(&self) -> ColPoint {
        self.end
    }

    /// 位置 `byte` 以前で最も近い記録点。
    fn point_before_byte(&self, byte: usize) -> ColPoint {
        let i = self.points.partition_point(|p| p.byte <= byte);
        self.points[i.saturating_sub(1)]
    }

    /// 位置 `byte` (char の境界。行末以上は行末) の (表示列, char 数) と、そこにある char の幅 (行末では 1)。
    pub fn at_byte(&self, text: &dyn TextRead, ls: usize, byte: usize) -> (ColPoint, usize) {
        if byte >= self.end.byte {
            return (self.end, 1);
        }
        let mut width = 1;
        let at = scan(
            text,
            ls,
            self.point_before_byte(byte),
            self.end.byte,
            &self.rule,
            |at, _, w| {
                width = w;
                at.byte < byte
            },
        );
        (at, width)
    }

    /// 表示列 `dcol` に対応する位置。セルの途中は、その char の前に寄せる。行の幅以上は行末。
    pub fn at_col(&self, text: &dyn TextRead, ls: usize, dcol: usize) -> ColPoint {
        if dcol >= self.end.col {
            return self.end;
        }
        let i = self.points.partition_point(|p| p.col <= dcol);
        scan(
            text,
            ls,
            self.points[i.saturating_sub(1)],
            self.end.byte,
            &self.rule,
            |at, _, w| at.col + w <= dcol,
        )
    }

    /// 位置 `from` (char の境界) から、`f(位置, char, 幅)` が `false` を返すまで char を順に渡す (Frame の行の生成が使う)。
    pub fn walk(
        &self,
        text: &dyn TextRead,
        ls: usize,
        from: ColPoint,
        f: impl FnMut(ColPoint, char, usize) -> bool,
    ) -> ColPoint {
        scan(text, ls, from, self.end.byte, &self.rule, f)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{unit_slow, units_with};
    use super::*;
    use eve_text::Text;

    fn rule() -> CharWidth {
        CharWidth::new(true, super::super::width_config())
    }

    /// 通常の行の経路 (`units_with`) で、書記素が char と一致する行の各単位の (バイト位置, 表示列, 幅)。
    fn by_units(s: &str, r: &CharWidth) -> Vec<(usize, usize, usize)> {
        units_with(s, r.esc, true, &r.cfg)
            .iter()
            .map(|u| (u.byte, u.col, u.width))
            .collect()
    }

    fn by_index(s: &str, r: CharWidth) -> Vec<(usize, usize, usize)> {
        let t = Text::from(s);
        let ix = ColIndex::build(&t, 0, s.len(), r);
        s.char_indices()
            .map(|(b, _)| {
                let (p, w) = ix.at_byte(&t, 0, b);
                (p.byte, p.col, w)
            })
            .collect()
    }

    #[test]
    fn char_width_matches_single_char_units() {
        // 1 char を書記素とした通常の経路の幅と一致する (サロゲートを除く BMP と補助面の主な範囲。列 0 と 3 で)
        for esc in [false, true] {
            for ambiguous_wide in [false, true] {
                let cfg = WidthConfig {
                    ambiguous_wide,
                    ..super::super::width_config()
                };
                let r = CharWidth::new(esc, cfg);
                for v in (0x01..0x3_0000u32).filter(|v| !matches!(v, 0x0A | 0x0D)) {
                    let Some(c) = char::from_u32(v) else { continue };
                    let g = c.to_string();
                    for col in [0, 3] {
                        let u = unit_slow(&g, 0, col, r.tab, esc, true, &cfg);
                        assert_eq!(r.of(c, col), u.width, "U+{v:04X} col {col}");
                    }
                }
            }
        }
    }

    #[test]
    fn index_matches_units_when_graphemes_are_chars() {
        let r = rule();
        let samples = [
            "abc def",
            "日本語のテキスト、です。",
            "a\tb\t\tc",
            "混在 abc\t漢字 😀 x",
            "\u{1}ctl\u{7F}\u{85}",
            "全角\u{3000}空白",
        ];
        for s in samples {
            assert_eq!(by_index(s, r), by_units(s, &r), "{s:?}");
            let t = Text::from(s);
            let ix = ColIndex::build(&t, 0, s.len(), r);
            let last = by_units(s, &r).last().map_or(0, |u| u.1 + u.2);
            assert_eq!(ix.end().col, last, "{s:?}");
            assert_eq!(ix.end().chars, s.chars().count());
        }
    }

    #[test]
    fn byte_at_col_snaps_before() {
        let s = "aあb\tc";
        let t = Text::from(s);
        let ix = ColIndex::build(&t, 0, s.len(), rule());
        let byte = |c| ix.at_col(&t, 0, c).byte;
        assert_eq!(byte(0), 0);
        assert_eq!(byte(1), 1); // あ
        assert_eq!(byte(2), 1); // あ の右半分は前に寄せる
        assert_eq!(byte(3), 4); // b
        assert_eq!(byte(4), 5); // タブ (列 4..8)
        assert_eq!(byte(7), 5);
        assert_eq!(byte(8), 6); // c
        assert_eq!(byte(99), s.len());
    }

    /// 記録点をまたぐ長い行 (記録点の境界が char の途中にならないことと、差分走査の結果が先頭からの走査と一致すること)。
    #[test]
    fn long_line_points_and_lookups() {
        let unit = "ab\tあ😀c";
        let s = format!("x{}", unit.repeat(STEP * 3 / unit.len()));
        let t = Text::from(format!("first\n{s}\nlast").as_str());
        let ls = 6;
        let r = rule();
        let ix = ColIndex::build(&t, ls, ls + s.len(), r);
        assert!(ix.points.len() >= 3);
        assert!(ix.points.iter().all(|p| s.is_char_boundary(p.byte)));
        assert_eq!(ix.end().byte, s.len());
        assert_eq!(ix.end().chars, s.chars().count());
        // 先頭から数えた値と照合する (幾つかの位置)
        let mut col = 0;
        for (n, (b, c)) in s.char_indices().enumerate() {
            if n % 7919 == 0 || b + 8 > s.len() {
                let (p, w) = ix.at_byte(&t, ls, b);
                assert_eq!((p.col, p.chars), (col, n), "byte {b}");
                assert_eq!(w, r.of(c, col));
                assert_eq!(ix.at_col(&t, ls, col).byte, b, "col {col}");
            }
            col += r.of(c, col);
        }
        assert_eq!(ix.end().col, col);
    }

    #[test]
    fn walk_stops_where_told() {
        let s = "abcあいう";
        let t = Text::from(s);
        let ix = ColIndex::build(&t, 0, s.len(), rule());
        let from = ix.at_col(&t, 0, 2);
        let mut got = String::new();
        let stop = ix.walk(&t, 0, from, |at, c, _| {
            if at.col >= 7 {
                return false;
            }
            got.push(c);
            true
        });
        assert_eq!(got, "cあい");
        assert_eq!((stop.byte, stop.col), (9, 7));
    }

    #[test]
    fn doc_settings_are_reflected() {
        // タブ幅は文書の設定から (呼び出し側は doc_width_config(doc) を渡す)
        let cfg = WidthConfig {
            tab_width: 8,
            ..super::super::width_config()
        };
        let r = CharWidth::new(false, cfg);
        assert_eq!(r.of('\t', 3), 5);
    }
}
