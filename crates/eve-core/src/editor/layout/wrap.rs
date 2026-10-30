//! 折り返し (ソフトラップ、docs/03 §14.3・§14.3.1、01 FR-506)。
//!
//! `wrap_line` は 1 論理行の表示の単位 (`Unit`) の列を表示行に分ける純関数で、Frame の行・カーソル・移動・スクロールが
//! すべてこれだけを根拠にする。折り返し OFF は「全体で 1 行」を返すので、利用者側に分岐を増やさない。

use super::Unit;

/// 折り返しの方式 (設定 `view.wrap`、07 §5.3)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WrapMode {
    /// 折り返さない (水平スクロール)。
    #[default]
    Off,
    /// 幅いっぱいの書記素単位で折る (禁則だけ適用)。
    Soft,
    /// 単語境界を優先して折る (禁則も適用)。
    Word,
}

/// View の折り返しの設定。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WrapSettings {
    pub mode: WrapMode,
    /// 継続行を論理行のインデント幅で揃える (`view.wrap_indent = same`)。
    pub indent_same: bool,
    /// 禁則処理 (`view.wrap_kinsoku`)。
    pub kinsoku: bool,
}

impl Default for WrapSettings {
    fn default() -> WrapSettings {
        WrapSettings {
            mode: WrapMode::Off,
            indent_same: true,
            kinsoku: true,
        }
    }
}

impl WrapSettings {
    pub fn is_on(&self) -> bool {
        self.mode != WrapMode::Off
    }
}

/// 折り返し位置のカーソルの帰属 (03 §14.3)。`Before` = 上の表示行の末尾、`After` = 下の表示行の先頭。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Affinity {
    Before,
    #[default]
    After,
}

/// 表示行 1 本: 単位の範囲 `first..end` と、行頭のずらし (継続行のインデント)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct WrapRow {
    pub first: usize,
    pub end: usize,
    pub indent: usize,
}

/// 行頭に来てはならない書記素 (行頭禁則の最小集合、OQ-57)。
fn no_line_start(c: char) -> bool {
    "、。，．・：；！？）］｝〕〉》」』】゛゜ー々ゝゞヽヾぁぃぅぇぉっゃゅょゎァィゥェォッャュョヮヵヶ"
        .contains(c)
}

/// 行末に来てはならない書記素 (行末禁則の最小集合、OQ-57)。
fn no_line_end(c: char) -> bool {
    "（［｛〔〈《「『【([{".contains(c)
}

fn is_space(c: char) -> bool {
    c == ' ' || c == '\t'
}

/// 単位 `i` の元の文字 (書記素の先頭)。
fn char_at(content: &str, units: &[Unit], i: usize) -> char {
    content[units[i].byte..].chars().next().unwrap_or(' ')
}

/// 継続行をずらす幅: 論理行の先頭の空白の表示幅。折り返し幅の 2/3 を超えるなら 0 (03 §14.3)。
fn continuation_indent(content: &str, units: &[Unit], cols: usize) -> usize {
    let n = units
        .iter()
        .take_while(|u| content[u.byte..].chars().next().is_some_and(is_space))
        .count();
    let width = units.get(n.wrapping_sub(1)).map_or(0, |u| u.col + u.width);
    if n == units.len() || width * 3 > cols * 2 {
        0
    } else {
        width
    }
}

/// 1 論理行を表示行に分ける。`cols` は折り返し幅 (本文の幅 − 右余白 1)。
pub(super) fn wrap_line(
    content: &str,
    units: &[Unit],
    cols: usize,
    s: WrapSettings,
) -> Vec<WrapRow> {
    let n = units.len();
    let whole = vec![WrapRow {
        first: 0,
        end: n,
        indent: 0,
    }];
    if !s.is_on() || n == 0 || cols == 0 {
        return whole;
    }
    let indent = if s.indent_same {
        continuation_indent(content, units, cols)
    } else {
        0
    };
    let ch = |i: usize| char_at(content, units, i);
    let kinsoku_ok = |b: usize| !s.kinsoku || !(no_line_start(ch(b)) || no_line_end(ch(b - 1)));
    // 折れる位置 (b の直前で折る): 空白の直後、または CJK などの幅 2 の書記素の前後
    let opportunity = |b: usize| {
        is_space(ch(b - 1))
            || (units[b - 1].width == 2 && !units[b - 1].parts.first().is_some_and(|p| p.2))
            || (units[b].width == 2 && !units[b].parts.first().is_some_and(|p| p.2))
    };
    let mut rows: Vec<WrapRow> = Vec::new();
    let mut start = 0;
    while start < n {
        let row_indent = if rows.is_empty() { 0 } else { indent };
        let avail = cols.saturating_sub(row_indent).max(1);
        let base = units[start].col;
        // 入るところまで進む。行末の空白は 1 セルまで右余白にぶら下げてよい
        let mut i = start;
        while i < n {
            let end = units[i].col + units[i].width - base;
            let hang = ch(i) == ' ' && end <= avail + 1;
            if end > avail && !hang {
                break;
            }
            i += 1;
        }
        if i >= n {
            rows.push(WrapRow {
                first: start,
                end: n,
                indent: row_indent,
            });
            break;
        }
        // `i` が入らない。行には最低 1 つ入れる。折る位置は i 以前で最も後ろの許される位置
        let b = if i == start {
            start + 1
        } else {
            let pick = |need_opportunity: bool| {
                (start + 1..=i)
                    .rev()
                    .find(|&b| kinsoku_ok(b) && (!need_opportunity || opportunity(b)))
            };
            let preferred = if s.mode == WrapMode::Word {
                pick(true)
            } else {
                pick(false)
            };
            // 単語境界で折れないときは書記素単位で、それも禁則で折れないなら禁則を破って i で折る
            preferred.or_else(|| pick(false)).unwrap_or(i)
        };
        rows.push(WrapRow {
            first: start,
            end: b,
            indent: row_indent,
        });
        start = b;
    }
    rows
}

/// カーソル (行内のバイト位置 `rel`) がある表示行と、その行内の列 (継続行のずらしを含む)。
/// 折り返し位置では `affinity` で上の行の末尾か下の行の先頭かを決める。行末は最後の表示行の末尾。
pub(super) fn cursor_in_rows(
    units: &[Unit],
    rows: &[WrapRow],
    rel: usize,
    affinity: Affinity,
) -> (usize, usize) {
    let n = units.len();
    let k = units.iter().position(|u| u.byte >= rel).unwrap_or(n);
    let mut r = rows
        .iter()
        .position(|r| k >= r.first && k < r.end)
        .unwrap_or(rows.len() - 1);
    if k < n && k == rows[r].first && r > 0 && affinity == Affinity::Before {
        r -= 1;
    }
    let row = rows[r];
    let base = units.get(row.first).map_or(0, |u| u.col);
    let col = if k < row.end && k < n {
        row.indent + units[k].col - base
    } else if row.end == 0 {
        row.indent
    } else {
        // 表示行の末尾 (行末、または Before で折り返し位置)
        let last = &units[row.end - 1];
        row.indent + last.col + last.width - base
    };
    (r, col)
}

#[cfg(test)]
mod tests {
    use super::super::{units, width_config};
    use super::*;

    fn rows_of(text: &str, cols: usize, mode: WrapMode) -> Vec<String> {
        let us = units(text, false, &width_config());
        let s = WrapSettings {
            mode,
            ..WrapSettings::default()
        };
        wrap_line(text, &us, cols, s)
            .iter()
            .map(|r| {
                let a = us.get(r.first).map_or(text.len(), |u| u.byte);
                let b = us.get(r.end).map_or(text.len(), |u| u.byte);
                format!("{}{}", " ".repeat(r.indent), &text[a..b])
            })
            .collect()
    }

    #[test]
    fn 折り返し_off_は全体で_1_行() {
        assert_eq!(rows_of("hello world", 5, WrapMode::Off), ["hello world"]);
    }

    #[test]
    fn word_は空白の後で折り_空白は行末にぶら下げる() {
        assert_eq!(
            rows_of("aaa bbb ccc", 7, WrapMode::Word),
            ["aaa bbb ", "ccc"],
            "7 桁に 'aaa bbb' が入り、後ろの空白は右余白にぶら下がる"
        );
    }

    #[test]
    fn word_は長い単語を書記素単位で強制的に折る() {
        assert_eq!(
            rows_of("abcdefghij", 4, WrapMode::Word),
            ["abcd", "efgh", "ij"]
        );
        assert_eq!(rows_of("x abcdefghij", 4, WrapMode::Word)[0], "x ");
    }

    #[test]
    fn soft_は幅いっぱいで書記素単位に折る() {
        assert_eq!(
            rows_of("aaa bbb ccc", 6, WrapMode::Soft),
            ["aaa bb", "b ccc"]
        );
    }

    #[test]
    fn 全角は入らなければ次の行へ送る() {
        assert_eq!(
            rows_of("あいうえお", 5, WrapMode::Word),
            ["あい", "うえ", "お"]
        );
    }

    #[test]
    fn 行頭禁則の文字は行頭に来ない() {
        // 幅 6 に「あいう」が入り、次は「、」: 「、」を行頭に出さず「う」ごと次の行へ送る
        assert_eq!(
            rows_of("あいう、えお", 6, WrapMode::Word),
            ["あい", "う、え", "お"]
        );
    }

    #[test]
    fn 行末禁則の文字は行末に来ない() {
        assert_eq!(rows_of("あい「うえ", 6, WrapMode::Word), ["あい", "「うえ"]);
    }

    #[test]
    fn 禁則を満たす位置が無ければ破って折る() {
        // 幅 4 で「、」が続く: どこで折っても行頭が「、」になる
        let r = rows_of("、、、、、、", 4, WrapMode::Word);
        assert!(
            r.len() >= 2 && r.iter().all(|l| l.chars().count() <= 2),
            "{r:?}"
        );
    }

    #[test]
    fn 禁則は設定で外せる() {
        let cfg = width_config();
        let t = "あいう、えお";
        let us = units(t, false, &cfg);
        let s = WrapSettings {
            mode: WrapMode::Word,
            kinsoku: false,
            ..WrapSettings::default()
        };
        let rows = wrap_line(t, &us, 6, s);
        assert_eq!(rows[0].end, 3, "「あいう」で折る (行頭が「、」になる)");
    }

    #[test]
    fn インデントを継続行に引き継ぎ_幅の三分の二を超えるなら引き継がない() {
        let r = rows_of("    aaaa bbbb cccc", 10, WrapMode::Word);
        assert_eq!(r[1], "    bbbb ", "{r:?}");
        let r = rows_of("        aaaa bbbb", 10, WrapMode::Word);
        assert!(!r[1].starts_with(' '), "8/10 > 2/3: {r:?}");
    }

    #[test]
    fn タブは論理行の列で幅が決まり_入らなければ次の行へ() {
        let r = rows_of("abcd\te", 5, WrapMode::Soft);
        assert_eq!(r, ["abcd", "\te"]);
    }

    #[test]
    fn カーソルの表示行と列_折り返し位置は_affinity_で決まる() {
        let t = "abcdefgh";
        let cfg = width_config();
        let us = units(t, false, &cfg);
        let s = WrapSettings {
            mode: WrapMode::Soft,
            ..WrapSettings::default()
        };
        let rows = wrap_line(t, &us, 4, s);
        assert_eq!(cursor_in_rows(&us, &rows, 2, Affinity::After), (0, 2));
        assert_eq!(cursor_in_rows(&us, &rows, 4, Affinity::After), (1, 0));
        assert_eq!(cursor_in_rows(&us, &rows, 4, Affinity::Before), (0, 4));
        assert_eq!(cursor_in_rows(&us, &rows, 8, Affinity::After), (1, 4));
    }
}
