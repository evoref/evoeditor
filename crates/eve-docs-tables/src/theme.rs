//! 組込みテーマ `default` のロール表の読み取りと検査 (docs/07 §8.2、docs/09 §3.5 の `xtask check-themes`)。
//!
//! 表の見出しは `### 8.2 書式` の下の、`ロール` 列を持つ表。セルの文法は 07 §8.2 の凡例どおり:
//! (1) 前景と背景の組 (`` `fg` / `bg` ``、後ろに ` + ` と属性列)、(2) 属性列だけ、(3) `` `none` ``。括弧の注記は読まない。

use crate::{Error, identifiers, section, strip_parentheticals, tables_in};

pub const THEME_HEADING: &str = "### 8.2 書式";

/// 色の指定 (表の文字列のまま)。`default` は端末の既定色。
pub type Color = String;

/// セル 1 つ (ある段のスタイル)。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StyleSpec {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    /// `bold` `underline` `reverse` `dim` (並びは表のまま)。
    pub mods: Vec<String>,
    /// `` `none` `` と書かれた (色も属性も付けない)。
    pub none: bool,
}

fn is_set(c: &Option<Color>) -> bool {
    c.as_deref().is_some_and(|c| c != "default")
}

impl StyleSpec {
    pub fn has_fg(&self) -> bool {
        is_set(&self.fg)
    }

    pub fn has_bg(&self) -> bool {
        is_set(&self.bg)
    }

    /// 何も書かれていない (`none` も色も属性もない)。
    pub fn is_empty(&self) -> bool {
        self.fg.is_none() && self.bg.is_none() && self.mods.is_empty() && !self.none
    }
}

/// ロール表の 1 行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleRow {
    pub line: usize,
    pub role: String,
    /// `(i)` `(ii)` `(iii)`。
    pub form: String,
    pub truecolor: StyleSpec,
    pub ansi256: StyleSpec,
    pub ansi16: StyleSpec,
    pub mono: StyleSpec,
}

const MODS: [&str; 4] = ["bold", "underline", "reverse", "dim"];

/// セルを読む。読めなければ理由を返す。
pub fn parse_cell(cell: &str) -> Result<StyleSpec, String> {
    let text = strip_parentheticals(cell);
    let mut spec = StyleSpec::default();
    for part in text.split(" + ") {
        let part = part.trim();
        if part.is_empty() {
            return Err(format!("空の項がある: {cell:?}"));
        }
        let toks = identifiers(part);
        if part.contains(" / ") {
            if toks.len() != 2 || spec.fg.is_some() {
                return Err(format!("色の組は `前景` / `背景` の 1 つだけ: {cell:?}"));
            }
            spec.fg = Some(toks[0].clone());
            spec.bg = Some(toks[1].clone());
        } else if toks.len() == 1 && toks[0] == "none" {
            spec.none = true;
        } else if toks.len() == 1 && MODS.contains(&toks[0].as_str()) {
            spec.mods.push(toks[0].clone());
        } else {
            return Err(format!(
                "属性は bold / underline / reverse / dim / none だけ: {cell:?}"
            ));
        }
    }
    if spec.none && (spec.fg.is_some() || !spec.mods.is_empty()) {
        return Err(format!("`none` は他の語と結合しない: {cell:?}"));
    }
    Ok(spec)
}

/// 07 §8.2 のロール表を読む。
pub fn parse_roles(doc07: &str) -> Result<Vec<RoleRow>, String> {
    let body = section(doc07, THEME_HEADING).map_err(|e: Error| e.to_string())?;
    let tables = tables_in(&body);
    let t = tables
        .iter()
        .find(|t| t.column("ロール").is_some())
        .ok_or("07 §8.2 に `ロール` 列を持つ表がありません")?;
    let col = |prefix: &str| -> Result<String, String> {
        t.columns
            .iter()
            .find(|c| c.starts_with(prefix))
            .cloned()
            .ok_or(format!("07 §8.2 の表に `{prefix}…` の列がありません"))
    };
    let (c_true, c_256, c_16) = (col("truecolor")?, col("256")?, col("16")?);
    let mut rows = Vec::new();
    for r in &t.rows {
        let get = |name: &str| t.cell(r, name).unwrap_or_default();
        let role = identifiers(get("ロール"))
            .into_iter()
            .next()
            .ok_or(format!("行 {}: ロール名がありません", r.line))?;
        let parse = |name: &str| {
            parse_cell(get(name)).map_err(|e| format!("行 {} ({role}) の `{name}`: {e}", r.line))
        };
        rows.push(RoleRow {
            line: r.line,
            form: get("形").to_owned(),
            truecolor: parse(&c_true)?,
            ansi256: parse(&c_256)?,
            ansi16: parse(&c_16)?,
            mono: parse("mono")?,
            role,
        });
    }
    Ok(rows)
}

/// `#rrggbb` / `#rgb` を (r, g, b) にする。
pub fn hex_rgb(s: &str) -> Option<(u8, u8, u8)> {
    let h = s.strip_prefix('#')?;
    let v = |i: usize, n: usize| u8::from_str_radix(h.get(i..i + n)?, 16).ok();
    match h.len() {
        6 => Some((v(0, 2)?, v(2, 2)?, v(4, 2)?)),
        3 => {
            let d = |i| v(i, 1).map(|x| x * 17);
            Some((d(0)?, d(1)?, d(2)?))
        }
        _ => None,
    }
}

/// WCAG 2.x の相対輝度。
pub fn luminance((r, g, b): (u8, u8, u8)) -> f64 {
    let f = |c: u8| {
        let c = f64::from(c) / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)
}

/// コントラスト比 (1.0〜21.0)。
pub fn contrast(a: (u8, u8, u8), b: (u8, u8, u8)) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// mono の段が空であってはならない、意味のあるロール (09 §3.5 (d))。
pub const MEANINGFUL_ROLES: [&str; 7] = [
    "ui.message.error",
    "ui.message.warning",
    "ui.selection",
    "ui.search.match",
    "ui.search.current",
    "ui.dialog.button.focused",
    "ui.search_bar.toggle_on",
];

/// 表そのものの検査 (09 §3.5 (a)(b)(c)(d))。エラーを列挙して返す。
pub fn check_table(rows: &[RoleRow]) -> Vec<String> {
    let mut errs = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for r in rows {
        let at = format!("07 §8.2 行 {} ({})", r.line, r.role);
        if !seen.insert(r.role.clone()) {
            errs.push(format!("{at}: ロール名が重複している"));
        }
        // (a) truecolor と mono の値がある
        for (name, s) in [("truecolor", &r.truecolor), ("mono", &r.mono)] {
            if s.is_empty() {
                errs.push(format!(
                    "{at}: {name} の値がない (「なし」は `none` と明示する)"
                ));
            }
        }
        // (b) 形。背景だけの段がない。形の列と truecolor の段が一致する
        for (name, s) in [
            ("truecolor", &r.truecolor),
            ("256", &r.ansi256),
            ("16", &r.ansi16),
        ] {
            if s.has_bg() && !s.has_fg() {
                errs.push(format!("{at}: {name} が背景だけを指定している"));
            }
        }
        let t = &r.truecolor;
        let form = match (t.has_fg(), t.has_bg()) {
            (true, true) => "(i)",
            (true, false) => "(iii)",
            (false, false) => "(ii)",
            (false, true) => "背景だけ",
        };
        if r.form != form {
            errs.push(format!(
                "{at}: 形の列は {} だが、truecolor の段は {form}",
                r.form
            ));
        }
        // (c) コントラスト
        let rgb = |c: &Option<Color>| c.as_deref().and_then(hex_rgb);
        match form {
            "(i)" => match (rgb(&t.fg), rgb(&t.bg)) {
                (Some(f), Some(b)) => {
                    let c = contrast(f, b);
                    if c < 4.5 {
                        errs.push(format!("{at}: 組のコントラストが {c:.2} (4.5 以上が必要)"));
                    }
                }
                _ => errs.push(format!("{at}: truecolor の色が #rrggbb でない")),
            },
            "(iii)" => match rgb(&t.fg) {
                Some(f) => {
                    for (name, bg) in [("白", (255, 255, 255)), ("黒", (0, 0, 0))] {
                        let c = contrast(f, bg);
                        if c < 3.0 {
                            errs.push(format!(
                                "{at}: 前景の{name}背景に対するコントラストが {c:.2} (3.0 以上が必要)"
                            ));
                        }
                    }
                }
                None => errs.push(format!("{at}: truecolor の前景が #rrggbb でない")),
            },
            _ => {}
        }
        // (d) 意味のあるロールの mono は空でない
        if MEANINGFUL_ROLES.contains(&r.role.as_str()) && (r.mono.none || r.mono.mods.is_empty()) {
            errs.push(format!(
                "{at}: mono の段が空 (意味のあるロールは属性を持つ)"
            ));
        }
    }
    errs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn セルの文法() {
        let s = parse_cell("`#1a1a1a` / `#e6c84a` + `underline` (10.6)").unwrap();
        assert_eq!(s.fg.as_deref(), Some("#1a1a1a"));
        assert_eq!(s.bg.as_deref(), Some("#e6c84a"));
        assert_eq!(s.mods, ["underline"]);
        let s = parse_cell("`reverse` + `bold`").unwrap();
        assert!(s.fg.is_none() && s.mods == ["reverse", "bold"]);
        let s = parse_cell("`none` (通常属性。区切りで区別する)").unwrap();
        assert!(s.none && s.mods.is_empty());
        assert!(parse_cell("`none` + `bold`").is_err());
        assert!(parse_cell("`italic`").is_err());
    }

    #[test]
    fn コントラスト比() {
        let w = (255, 255, 255);
        assert!((contrast(w, (0, 0, 0)) - 21.0).abs() < 0.01);
        assert!((contrast(w, w) - 1.0).abs() < 0.01);
        assert_eq!(hex_rgb("#fff"), Some((255, 255, 255)));
        assert_eq!(hex_rgb("#1a2b3c"), Some((0x1a, 0x2b, 0x3c)));
    }

    #[test]
    fn 検査は背景だけの段と比の不足を拒否する() {
        let md = "### 8.2 書式\n\n| ロール | 用途 | 形 | truecolor (fg / bg) | 256 (fg / bg、参考) | 16 (fg / bg、参考) | mono |\n|---|---|---|---|---|---|---|\n| `ui.x` | x | (i) | `#777777` / `#808080` | `default` / `ansi.5` | `default` / `default` | `bold` |\n";
        let rows = parse_roles(md).unwrap();
        let errs = check_table(&rows);
        assert!(errs.iter().any(|e| e.contains("コントラスト")), "{errs:?}");
        assert!(
            errs.iter().any(|e| e.contains("256 が背景だけ")),
            "{errs:?}"
        );
    }
}
