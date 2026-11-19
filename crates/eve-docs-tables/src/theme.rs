//! 組込みテーマのロール表の読み取りと検査 (docs/07 §8.2・§8.7、docs/09 §3.5 の `xtask check-themes`)。
//!
//! `default` の表の見出しは `### 8.2 書式` の下の、`ロール` 列を持つ表。高コントラストのテーマ
//! (`high-contrast-dark` / `high-contrast-light`) の表は、07 §8.7.1 / §8.7.2 の見出しの直後の表 1 つで、列の名前と順を
//! 固定する (docs/11 §8.5)。セルの文法は 07 §8.2 の凡例どおり:
//! (1) 前景と背景の組 (`` `fg` / `bg` ``、後ろに ` + ` と属性列)、(2) 属性列だけ、(3) `` `none` ``。括弧の注記は読まない。

use std::collections::BTreeSet;

use crate::{Error, Table, identifiers, section, strip_parentheticals, tables_in};

pub const THEME_HEADING: &str = "### 8.2 書式";

/// 高コントラストのテーマの名前と、ロール表の見出し (07 §8.7.1 / §8.7.2。docs/11 §8.5 で固定)。
pub const HIGH_CONTRAST_TABLES: [(&str, &str); 2] = [
    (
        "high-contrast-dark",
        "#### 8.7.1 high-contrast-dark のロール表",
    ),
    (
        "high-contrast-light",
        "#### 8.7.2 high-contrast-light のロール表",
    ),
];

/// 高コントラストのロール表の列 (名前と順を固定する。docs/11 §8.5)。
pub const HIGH_CONTRAST_COLUMNS: [&str; 7] = [
    "ロール",
    "用途",
    "形",
    "truecolor (fg / bg)",
    "256 (fg / bg)",
    "16 (fg / bg)",
    "mono",
];

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
///
/// # Errors
///
/// セルに空の項があるとき、色の組が `前景` / `背景` の 1 つずつでないとき、または `none` を他の語と結合したとき (理由の文言を返す)。
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
///
/// # Errors
///
/// 見出しや表がないとき、ロール名や必要な列がないとき、またはセルを読めないとき (理由の文言を返す)。
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
    rows_of(t, &c_true, &c_256, &c_16)
}

/// 07 §8.7 の高コントラストのロール表 (`heading` の直後の表 1 つ) を読む。列の名前と順は
/// `HIGH_CONTRAST_COLUMNS` で固定し、後ろに足した列は無視する (docs/11 §8.5)。
///
/// # Errors
///
/// 見出しや表がないとき、列の名前・順が違うとき、ロール名がないとき、またはセルを読めないとき (理由の文言を返す)。
pub fn parse_high_contrast(doc07: &str, heading: &str) -> Result<Vec<RoleRow>, String> {
    let body = section(doc07, heading).map_err(|e: Error| e.to_string())?;
    let tables = tables_in(&body);
    let t = tables
        .first()
        .ok_or(format!("{heading} の直後に表がありません"))?;
    let head: Vec<&str> = t.columns.iter().map(String::as_str).collect();
    if head.len() < HIGH_CONTRAST_COLUMNS.len()
        || head[..HIGH_CONTRAST_COLUMNS.len()] != HIGH_CONTRAST_COLUMNS
    {
        return Err(format!(
            "{heading} の表 (行 {}) の列が {:?} でない: {head:?}",
            t.line, HIGH_CONTRAST_COLUMNS
        ));
    }
    rows_of(
        t,
        HIGH_CONTRAST_COLUMNS[3],
        HIGH_CONTRAST_COLUMNS[4],
        HIGH_CONTRAST_COLUMNS[5],
    )
}

fn rows_of(t: &Table, c_true: &str, c_256: &str, c_16: &str) -> Result<Vec<RoleRow>, String> {
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
            truecolor: parse(c_true)?,
            ansi256: parse(c_256)?,
            ansi16: parse(c_16)?,
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

/// 16 色の名前 (07 §8.2 の「色」の文法。番号の順)。
pub const ANSI16_NAMES: [&str; 16] = [
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "white",
    "bright_black",
    "bright_red",
    "bright_green",
    "bright_yellow",
    "bright_blue",
    "bright_magenta",
    "bright_cyan",
    "bright_white",
];

/// xterm の既定の 16 色パレット (07 §8.2 の表の下、04 §12.3)。
pub const XTERM16: [(u8, u8, u8); 16] = [
    (0x00, 0x00, 0x00),
    (0xcd, 0x00, 0x00),
    (0x00, 0xcd, 0x00),
    (0xcd, 0xcd, 0x00),
    (0x00, 0x00, 0xee),
    (0xcd, 0x00, 0xcd),
    (0x00, 0xcd, 0xcd),
    (0xe5, 0xe5, 0xe5),
    (0x7f, 0x7f, 0x7f),
    (0xff, 0x00, 0x00),
    (0x00, 0xff, 0x00),
    (0xff, 0xff, 0x00),
    (0x5c, 0x5c, 0xff),
    (0xff, 0x00, 0xff),
    (0x00, 0xff, 0xff),
    (0xff, 0xff, 0xff),
];

/// Windows Terminal・conhost の既定の配色 Campbell (07 §8.7 の (h5))。
pub const CAMPBELL16: [(u8, u8, u8); 16] = [
    (0x0c, 0x0c, 0x0c),
    (0xc5, 0x0f, 0x1f),
    (0x13, 0xa1, 0x0e),
    (0xc1, 0x9c, 0x00),
    (0x00, 0x37, 0xda),
    (0x88, 0x17, 0x98),
    (0x3a, 0x96, 0xdd),
    (0xcc, 0xcc, 0xcc),
    (0x76, 0x76, 0x76),
    (0xe7, 0x48, 0x56),
    (0x16, 0xc6, 0x0c),
    (0xf9, 0xf1, 0xa5),
    (0x3b, 0x78, 0xff),
    (0xb4, 0x00, 0x9e),
    (0x61, 0xd6, 0xd6),
    (0xf2, 0xf2, 0xf2),
];

/// xterm の 256 色の標準値 (0〜15 は xterm の既定パレット、16〜231 は 6×6×6 の立方体、232〜255 は灰色。07 §8.1)。
pub fn xterm256_rgb(n: u8) -> (u8, u8, u8) {
    const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];
    match n {
        0..=15 => XTERM16[usize::from(n)],
        16..=231 => {
            let i = n - 16;
            (
                CUBE[usize::from(i / 36)],
                CUBE[usize::from((i / 6) % 6)],
                CUBE[usize::from(i % 6)],
            )
        }
        232..=255 => {
            let v = 8 + 10 * (n - 232);
            (v, v, v)
        }
    }
}

/// 色の段 (高コントラストの検査で色を RGB に直すときの文法の違い)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    /// `#rrggbb` / `#rgb`。
    TrueColor,
    /// `ansi.<0-255>`。
    Ansi256,
    /// 16 色の名前 (`ansi.` を省く)。
    Ansi16,
}

impl Stage {
    fn label(self) -> &'static str {
        match self {
            Stage::TrueColor => "truecolor",
            Stage::Ansi256 => "256",
            Stage::Ansi16 => "16",
        }
    }

    fn of(self, r: &RoleRow) -> &StyleSpec {
        match self {
            Stage::TrueColor => &r.truecolor,
            Stage::Ansi256 => &r.ansi256,
            Stage::Ansi16 => &r.ansi16,
        }
    }
}

/// 16 色の名前の番号。
fn ansi16_index(name: &str) -> Option<u8> {
    ANSI16_NAMES
        .iter()
        .position(|n| *n == name)
        .and_then(|i| u8::try_from(i).ok())
}

type Palette = [(u8, u8, u8); 16];

/// 段の文法で書いた色 (`default` 以外) を RGB にする。16 の段は `palette` で引く。
fn stage_rgb(stage: Stage, c: &str, palette: &Palette) -> Result<(u8, u8, u8), String> {
    match stage {
        Stage::TrueColor => hex_rgb(c).ok_or(format!("truecolor の色は #rrggbb で書く: {c}")),
        Stage::Ansi256 => c
            .strip_prefix("ansi.")
            .and_then(|n| n.parse::<u8>().ok())
            .map(xterm256_rgb)
            .ok_or(format!("256 の色は ansi.<0-255> で書く: {c}")),
        Stage::Ansi16 => ansi16_index(c)
            .map(|i| palette[usize::from(i)])
            .ok_or(format!("16 の色は ansi. を省いた名前で書く: {c}")),
    }
}

/// 段の色を RGB にする。`default` (と無指定) は `fallback` (本文の色)。
fn effective(
    stage: Stage,
    c: Option<&str>,
    fallback: (u8, u8, u8),
    palette: &Palette,
) -> Option<(u8, u8, u8)> {
    match c {
        None | Some("default") => Some(fallback),
        Some(c) => stage_rgb(stage, c, palette).ok(),
    }
}

fn mod_set(s: &StyleSpec) -> BTreeSet<&str> {
    s.mods.iter().map(String::as_str).collect()
}

/// 選択・検索の一致・現在の一致 (07 §8.7 の (h7)(h8))。
const MARK_ROLES: [&str; 3] = ["ui.selection", "ui.search.match", "ui.search.current"];

/// 高コントラストの基準の比 (07 §8.7)。
const HC_PAIR: f64 = 7.0;
const HC_HIGHLIGHT_BG: f64 = 3.0;
const HC_16: f64 = 4.5;
const HC_MARK_BG: f64 = 1.5;

const ALL_STAGES: [&str; 4] = ["truecolor", "256", "16", "mono"];

fn stage_by_label<'a>(r: &'a RoleRow, label: &str) -> &'a StyleSpec {
    match label {
        "truecolor" => &r.truecolor,
        "256" => &r.ansi256,
        "16" => &r.ansi16,
        _ => &r.mono,
    }
}

/// 高コントラストのテーマの表の検査 (09 §3.5 の (g): (a)(b)(d) と 07 §8.7 の (h1)〜(h9))。エラーを列挙して返す。
/// (e) の実装との突合は core の `theme_table` の試験で、`default` の表とのロール名の集合の一致は呼び出し側で行う。
/// `theme` は診断に出すテーマの名前。
pub fn check_high_contrast(theme: &str, rows: &[RoleRow]) -> Vec<String> {
    let at = |r: &RoleRow| format!("07 §8.7 {theme} 行 {} ({})", r.line, r.role);
    let errs = check_hc_shape(rows, &at);
    if !errs.is_empty() {
        // 文法の誤りがあると比を計算できない
        return errs;
    }
    let mut errs = Vec::new();
    let find = |name: &str| rows.iter().find(|r| r.role == name);
    let Some(text) = find("ui.text") else {
        return vec![format!("07 §8.7 {theme}: `ui.text` の行がない")];
    };
    // (h6) 現在行の帯を付けない
    if let Some(r) = find("ui.cursor_line") {
        for label in ALL_STAGES {
            if !stage_by_label(r, label).none {
                errs.push(format!("{}: {label} の段は `none` にする (h6)", at(r)));
            }
        }
    }
    // (h1)〜(h4): truecolor と 256 の段で 7.0 / 3.0
    for stage in [Stage::TrueColor, Stage::Ansi256] {
        check_hc_contrast(
            rows, text, stage, &XTERM16, HC_PAIR, true, "", &at, &mut errs,
        );
    }
    // (h5): 16 の段は xterm の既定パレットと Campbell の両方で 4.5 ((h4) は課さない)
    for (palette, tag) in [(&XTERM16, " (xterm)"), (&CAMPBELL16, " (Campbell)")] {
        check_hc_contrast(
            rows,
            text,
            Stage::Ansi16,
            palette,
            HC_16,
            false,
            tag,
            &at,
            &mut errs,
        );
    }
    let marks: Vec<&RoleRow> = MARK_ROLES.iter().filter_map(|n| find(n)).collect();
    if marks.len() == MARK_ROLES.len() {
        check_hc_marks(theme, text, &marks, &mut errs);
    } else {
        errs.push(format!(
            "07 §8.7 {theme}: {MARK_ROLES:?} の行がそろっていない"
        ));
    }
    check_hc_bold16(rows, text, &at, &mut errs);
    check_hc_pale(rows, text, &at, &mut errs);
    errs
}

/// 高コントラストの表のロール名の集合が、07 §8.2 の `default` の表と同じか (09 §3.5 の (e) の表どうしの側)。
pub fn check_role_set(theme: &str, rows: &[RoleRow], default_rows: &[RoleRow]) -> Vec<String> {
    let names: BTreeSet<&str> = default_rows.iter().map(|r| r.role.as_str()).collect();
    let hc: BTreeSet<&str> = rows.iter().map(|r| r.role.as_str()).collect();
    let mut errs = Vec::new();
    for missing in names.difference(&hc) {
        errs.push(format!(
            "07 §8.7 {theme}: 07 §8.2 の表にあるロール {missing} が無い"
        ));
    }
    for extra in hc.difference(&names) {
        errs.push(format!(
            "07 §8.7 {theme}: 07 §8.2 の表に無いロール {extra} がある"
        ));
    }
    errs
}

/// 重ねるロールと、その下に来るロール (07 §8.6 の 3 本目)。下が `ui.text` のものは本文の前景で判定する。
pub const LAYERED_UNDER: [(&str, &[&str]); 4] = [
    ("ui.status_bar.chord", &["ui.status_bar"]),
    ("ui.line_number.active", &["ui.line_number"]),
    ("ui.overlay.match", &["ui.overlay", "ui.overlay.selected"]),
    ("ui.search_bar.toggle_on", &["ui.overlay"]),
];

/// (h10): 色の段で、前景だけのロール (形 (iii)) の前景が本文の前景と違う。
fn check_hc_pale(
    rows: &[RoleRow],
    text: &RoleRow,
    at: &dyn Fn(&RoleRow) -> String,
    errs: &mut Vec<String>,
) {
    for stage in [Stage::TrueColor, Stage::Ansi256, Stage::Ansi16] {
        let tfg = stage.of(text).fg.as_deref();
        for r in rows {
            let s = stage.of(r);
            if r.role == "ui.text" || !s.has_fg() || s.has_bg() {
                continue;
            }
            if s.fg.as_deref() == tfg {
                errs.push(format!(
                    "{}: {} の段の淡い表示の前景が本文の前景と同じ (h10)",
                    at(r),
                    stage.label()
                ));
            }
        }
    }
}

/// (a)(b)(d) と (h6) の `dim`、色の文法。
fn check_hc_shape(rows: &[RoleRow], at: &dyn Fn(&RoleRow) -> String) -> Vec<String> {
    let mut errs = Vec::new();
    let mut seen = BTreeSet::new();
    for r in rows {
        let at = at(r);
        if !seen.insert(r.role.clone()) {
            errs.push(format!("{at}: ロール名が重複している"));
        }
        // (a) 4 段とも値がある (高コントラストの表は 256 / 16 も明示値)
        for label in ALL_STAGES {
            let s = stage_by_label(r, label);
            if s.is_empty() {
                errs.push(format!(
                    "{at}: {label} の値がない (「なし」は `none` と明示する)"
                ));
            }
            // (h6) dim を使わない
            if s.mods.iter().any(|m| m == "dim") {
                errs.push(format!("{at}: {label} の段で dim を使っている (h6)"));
            }
        }
        if r.mono.fg.is_some() || r.mono.bg.is_some() {
            errs.push(format!("{at}: mono の段は属性か `none` だけ"));
        }
        // (b) 背景だけの段が無い。色の文法
        for stage in [Stage::TrueColor, Stage::Ansi256, Stage::Ansi16] {
            let s = stage.of(r);
            if s.has_bg() && !s.has_fg() {
                errs.push(format!("{at}: {} が背景だけを指定している", stage.label()));
            }
            for c in [&s.fg, &s.bg].into_iter().flatten() {
                if c != "default"
                    && let Err(e) = stage_rgb(stage, c, &XTERM16)
                {
                    errs.push(format!("{at}: {e}"));
                }
            }
        }
        // (b) 形の列は truecolor の段の形
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
        // (d) 意味のあるロールの mono は空でない
        if MEANINGFUL_ROLES.contains(&r.role.as_str()) && (r.mono.none || r.mono.mods.is_empty()) {
            errs.push(format!(
                "{at}: mono の段が空 (意味のあるロールは属性を持つ)"
            ));
        }
    }
    errs
}

/// (h1)〜(h5): 段ごとに、本文の色で `default` を置き換えて比を計算する。`highlight_bg` なら (h4) も見る。
#[expect(clippy::too_many_arguments, reason = "検査の条件を並べて渡す")]
fn check_hc_contrast(
    rows: &[RoleRow],
    text: &RoleRow,
    stage: Stage,
    palette: &Palette,
    min: f64,
    highlight_bg: bool,
    tag: &str,
    at: &dyn Fn(&RoleRow) -> String,
    errs: &mut Vec<String>,
) {
    let ts = stage.of(text);
    let rgb = |c: Option<&str>| {
        c.filter(|c| *c != "default")
            .and_then(|c| stage_rgb(stage, c, palette).ok())
    };
    let (Some(tfg), Some(tbg)) = (rgb(ts.fg.as_deref()), rgb(ts.bg.as_deref())) else {
        errs.push(format!(
            "{}: {} の段は前景と背景の組にする (背景を塗る)",
            at(text),
            stage.label()
        ));
        return;
    };
    for r in rows {
        let s = stage.of(r);
        if !s.has_fg() && !s.has_bg() {
            continue;
        }
        let (Some(fg), Some(bg)) = (
            effective(stage, s.fg.as_deref(), tfg, palette),
            effective(stage, s.bg.as_deref(), tbg, palette),
        ) else {
            continue;
        };
        let which = if r.role == "ui.text" {
            "(h1) 本文"
        } else if s.has_bg() {
            "(h2) 組"
        } else {
            "(h3) 前景"
        };
        let c = contrast(fg, bg);
        if c < min {
            errs.push(format!(
                "{}: {} の段{tag}の {which} のコントラストが {c:.2} ({min:.1} 以上が必要)",
                at(r),
                stage.label()
            ));
        }
        if highlight_bg && r.role != "ui.text" && s.has_fg() && s.has_bg() {
            let c = contrast(bg, tbg);
            if c < HC_HIGHLIGHT_BG {
                errs.push(format!(
                    "{}: {} の段の (h4) 背景と本文の背景の比が {c:.2} ({HC_HIGHLIGHT_BG:.1} 以上が必要)",
                    at(r),
                    stage.label()
                ));
            }
        }
    }
}

/// (h7)(h8): 選択・検索の一致・現在の一致の区別。
fn check_hc_marks(theme: &str, text: &RoleRow, marks: &[&RoleRow], errs: &mut Vec<String>) {
    // (h7): どの段でも、属性の組が互いに違う
    for label in ALL_STAGES {
        for i in 0..marks.len() {
            for j in i + 1..marks.len() {
                let (a, b) = (marks[i], marks[j]);
                if mod_set(stage_by_label(a, label)) == mod_set(stage_by_label(b, label)) {
                    errs.push(format!(
                        "07 §8.7 {theme}: {label} の段で {} と {} の属性の組が同じ (h7)",
                        a.role, b.role
                    ));
                }
            }
        }
    }
    // (h8): 色の段 (16 は xterm の既定パレット) で、背景の明るさの比が 1.5 未満の組は、bold 以外の属性の差を持つ
    for stage in [Stage::TrueColor, Stage::Ansi256, Stage::Ansi16] {
        let Some(tbg) = stage
            .of(text)
            .bg
            .as_deref()
            .and_then(|c| stage_rgb(stage, c, &XTERM16).ok())
        else {
            continue;
        };
        let strip = |r: &RoleRow| -> BTreeSet<String> {
            mod_set(stage.of(r))
                .into_iter()
                .filter(|m| *m != "bold")
                .map(str::to_owned)
                .collect()
        };
        let tfg = stage
            .of(text)
            .fg
            .as_deref()
            .and_then(|c| stage_rgb(stage, c, &XTERM16).ok())
            .unwrap_or(tbg);
        // 見た目の背景: `reverse` を持つ段は前景の色が背景に見える (`default` は本文の色)
        let shown_bg = |r: &RoleRow| {
            let sp = stage.of(r);
            if sp.mods.iter().any(|m| m == "reverse") {
                effective(stage, sp.fg.as_deref(), tfg, &XTERM16)
            } else {
                effective(stage, sp.bg.as_deref(), tbg, &XTERM16)
            }
        };
        for i in 0..marks.len() {
            for j in i + 1..marks.len() {
                let (a, b) = (marks[i], marks[j]);
                let (Some(ba), Some(bb)) = (shown_bg(a), shown_bg(b)) else {
                    continue;
                };
                let c = contrast(ba, bb);
                if c < HC_MARK_BG && strip(a) == strip(b) {
                    errs.push(format!(
                        "07 §8.7 {theme}: {} の段で {} と {} の背景の比が {c:.2} ({HC_MARK_BG:.1} 未満) なのに、bold 以外の属性の差が無い (h8)",
                        stage.label(),
                        a.role,
                        b.role
                    ));
                }
            }
        }
    }
}

/// (h9): 16 の段で `bold` を持つロールの実効の前景が 0〜7 の色でない。前景が `default` のロールは本文の前景で、
/// 重ねるロール (`LAYERED_UNDER`) は下のロールの前景 (下が複数なら、そのすべて) で判定する。
fn check_hc_bold16(
    rows: &[RoleRow],
    text: &RoleRow,
    at: &dyn Fn(&RoleRow) -> String,
    errs: &mut Vec<String>,
) {
    let fg16 = |r: &RoleRow| {
        r.ansi16
            .fg
            .as_deref()
            .filter(|c| *c != "default")
            .and_then(ansi16_index)
    };
    let text_fg = fg16(text);
    let fg_of = |name: &str| {
        rows.iter()
            .find(|r| r.role == name)
            .and_then(fg16)
            .or(text_fg)
    };
    for r in rows {
        if !r.ansi16.mods.iter().any(|m| m == "bold") {
            continue;
        }
        let fgs: Vec<Option<u8>> = match (fg16(r), LAYERED_UNDER.iter().find(|(n, _)| *n == r.role))
        {
            (Some(i), _) => vec![Some(i)],
            (None, Some((_, under))) => under.iter().map(|u| fg_of(u)).collect(),
            (None, None) => vec![text_fg],
        };
        for fg in fgs {
            if let Some(i @ 0..=7) = fg {
                errs.push(format!(
                    "{}: 16 の段で bold を持つのに、実効の前景が {} (0〜7 の色) (h9)",
                    at(r),
                    ANSI16_NAMES[usize::from(i)]
                ));
            }
        }
    }
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

    const HC_HEAD: &str = "#### 8.7.1 high-contrast-dark のロール表\n\n| ロール | 用途 | 形 | truecolor (fg / bg) | 256 (fg / bg) | 16 (fg / bg) | mono |\n|---|---|---|---|---|---|---|\n";

    /// 基準を満たす小さな表 (本文・現在行・3 つの強調・チョード・行番号)。`edit` で 1 行を差し替える。
    fn hc_rows(edit: &[(&str, &str)]) -> Vec<String> {
        let base = [
            (
                "ui.text",
                "| `ui.text` | x | (i) | `#ffffff` / `#000000` | `ansi.231` / `ansi.16` | `bright_white` / `black` | `none` |",
            ),
            (
                "ui.cursor_line",
                "| `ui.cursor_line` | x | (ii) | `none` | `none` | `none` | `none` |",
            ),
            (
                "ui.line_number",
                "| `ui.line_number` | x | (iii) | `#c0c0c0` / `default` | `ansi.250` / `default` | `white` / `default` | `none` |",
            ),
            (
                "ui.selection",
                "| `ui.selection` | x | (i) | `#000000` / `#9ccfff` | `ansi.16` / `ansi.153` | `black` / `bright_cyan` | `reverse` |",
            ),
            (
                "ui.search.match",
                "| `ui.search.match` | x | (i) | `#000000` / `#ffff00` + `underline` | `ansi.16` / `ansi.226` + `underline` | `black` / `bright_yellow` + `underline` | `underline` |",
            ),
            (
                "ui.search.current",
                "| `ui.search.current` | x | (i) | `#000000` / `#ffa64d` + `underline` + `bold` | `ansi.16` / `ansi.215` + `underline` + `bold` | `black` / `bright_green` + `underline` + `reverse` | `reverse` + `underline` |",
            ),
            (
                "ui.status_bar",
                "| `ui.status_bar` | x | (i) | `#000000` / `#ffffff` | `ansi.16` / `ansi.231` | `black` / `bright_white` | `reverse` |",
            ),
            (
                "ui.status_bar.chord",
                "| `ui.status_bar.chord` | x | (ii) | `bold` | `bold` | `underline` | `bold` |",
            ),
        ];
        base.iter()
            .map(|(role, line)| {
                edit.iter()
                    .find(|(r, _)| r == role)
                    .map_or(*line, |(_, l)| *l)
                    .to_owned()
            })
            .collect()
    }

    fn hc_errs(edit: &[(&str, &str)]) -> Vec<String> {
        let md = format!("{HC_HEAD}{}\n", hc_rows(edit).join("\n"));
        let rows = parse_high_contrast(&md, HIGH_CONTRAST_TABLES[0].1).unwrap();
        check_high_contrast("high-contrast-dark", &rows)
    }

    #[test]
    fn 高コントラストの表_基準を満たす表は通る() {
        assert!(hc_errs(&[]).is_empty(), "{:?}", hc_errs(&[]));
    }

    #[test]
    fn 高コントラストの表_列の名前と順を固定する() {
        let md = "#### 8.7.1 high-contrast-dark のロール表\n\n| ロール | 用途 | 形 | truecolor (fg / bg) | 256 (fg / bg、参考) | 16 (fg / bg) | mono |\n|---|---|---|---|---|---|---|\n";
        assert!(parse_high_contrast(md, HIGH_CONTRAST_TABLES[0].1).is_err());
        assert!(parse_high_contrast("# x\n", HIGH_CONTRAST_TABLES[0].1).is_err());
    }

    #[test]
    fn 高コントラストの表_比の不足を拒否する() {
        // (h2) 組が 7.0 未満 (#777777 / #9ccfff は約 2.6)
        let e = hc_errs(&[(
            "ui.selection",
            "| `ui.selection` | x | (i) | `#777777` / `#9ccfff` | `ansi.16` / `ansi.153` | `black` / `bright_cyan` | `reverse` |",
        )]);
        assert!(e.iter().any(|e| e.contains("(h2)")), "{e:?}");
        // (h3) 前景が本文の背景に 7.0 未満
        let e = hc_errs(&[(
            "ui.line_number",
            "| `ui.line_number` | x | (iii) | `#808080` / `default` | `ansi.250` / `default` | `white` / `default` | `none` |",
        )]);
        assert!(e.iter().any(|e| e.contains("(h3)")), "{e:?}");
        // (h4) 強調の背景が本文の背景に 3.0 未満 (暗い青に白)
        let e = hc_errs(&[(
            "ui.selection",
            "| `ui.selection` | x | (i) | `#ffffff` / `#000080` | `ansi.16` / `ansi.153` | `black` / `bright_cyan` | `reverse` |",
        )]);
        assert!(e.iter().any(|e| e.contains("(h4)")), "{e:?}");
        // (h5) 16 の段で Campbell だけ 4.5 未満 (black / bright_magenta は xterm で 6.7、Campbell で約 3.2)
        let e = hc_errs(&[(
            "ui.selection",
            "| `ui.selection` | x | (i) | `#000000` / `#9ccfff` | `ansi.16` / `ansi.153` | `black` / `bright_magenta` | `reverse` |",
        )]);
        assert!(
            e.iter()
                .any(|e| e.contains("(Campbell)") && e.contains("(h2)"))
                && !e.iter().any(|e| e.contains("(xterm)")),
            "{e:?}"
        );
    }

    #[test]
    fn 高コントラストの表_dim_と現在行の帯と属性の重なりと太字を拒否する() {
        // (h6) dim
        let e = hc_errs(&[(
            "ui.line_number",
            "| `ui.line_number` | x | (iii) | `#c0c0c0` / `default` | `ansi.250` / `default` | `white` / `default` | `dim` |",
        )]);
        assert!(e.iter().any(|e| e.contains("dim")), "{e:?}");
        // (h6) 現在行の帯
        let e = hc_errs(&[(
            "ui.cursor_line",
            "| `ui.cursor_line` | x | (ii) | `bold` | `none` | `none` | `none` |",
        )]);
        assert!(e.iter().any(|e| e.contains("(h6)")), "{e:?}");
        // (h7) mono で選択と現在の一致が同じ属性
        let e = hc_errs(&[(
            "ui.search.current",
            "| `ui.search.current` | x | (i) | `#000000` / `#ffa64d` + `underline` + `bold` | `ansi.16` / `ansi.215` + `underline` + `bold` | `black` / `bright_green` + `underline` + `reverse` | `reverse` |",
        )]);
        assert!(e.iter().any(|e| e.contains("(h7)")), "{e:?}");
        // (h8) 背景が近く、bold だけの差 (#ffff00 と #ffff33 は約 1.0:1)
        let e = hc_errs(&[(
            "ui.search.current",
            "| `ui.search.current` | x | (i) | `#000000` / `#ffff33` + `underline` + `bold` | `ansi.16` / `ansi.215` + `underline` + `bold` | `black` / `bright_green` + `underline` + `reverse` | `reverse` + `underline` |",
        )]);
        assert!(e.iter().any(|e| e.contains("(h8)")), "{e:?}");
        // (h9) 16 の段で、前景が black の太字 (チョードはステータスバーの前景 black で判定する)
        let e = hc_errs(&[(
            "ui.status_bar.chord",
            "| `ui.status_bar.chord` | x | (ii) | `bold` | `bold` | `bold` | `bold` |",
        )]);
        assert!(
            e.iter()
                .any(|e| e.contains("(h9)") && e.contains("ui.status_bar.chord")),
            "{e:?}"
        );
    }

    #[test]
    fn 高コントラストの表_段の色の文法を確かめる() {
        let e = hc_errs(&[(
            "ui.selection",
            "| `ui.selection` | x | (i) | `#000000` / `#9ccfff` | `ansi.16` / `ansi.153` | `ansi.0` / `bright_cyan` | `reverse` |",
        )]);
        assert!(e.iter().any(|e| e.contains("16 の色")), "{e:?}");
    }

    /// `hc_errs` に、表の後ろへ行を足したもの。
    fn hc_errs_with(edit: &[(&str, &str)], extra: &[&str]) -> Vec<String> {
        let mut rows = hc_rows(edit);
        rows.extend(extra.iter().map(|l| (*l).to_owned()));
        let md = format!("{HC_HEAD}{}\n", rows.join("\n"));
        let rows = parse_high_contrast(&md, HIGH_CONTRAST_TABLES[0].1).unwrap();
        check_high_contrast("high-contrast-dark", &rows)
    }

    #[test]
    fn 高コントラストの表_本文の比と_256_の段の強調の背景の不足を拒否する() {
        // (h1) 本文が 7.0 未満 (#777777 / #000000 は約 4.6)
        let e = hc_errs(&[(
            "ui.text",
            "| `ui.text` | x | (i) | `#777777` / `#000000` | `ansi.231` / `ansi.16` | `bright_white` / `black` | `none` |",
        )]);
        assert!(e.iter().any(|e| e.contains("(h1)")), "{e:?}");
        // (h4) 256 の段だけ、強調の背景が本文の背景に 3.0 未満 (ansi.18 は暗い青)
        let e = hc_errs(&[(
            "ui.selection",
            "| `ui.selection` | x | (i) | `#000000` / `#9ccfff` | `ansi.231` / `ansi.18` | `black` / `bright_cyan` | `reverse` |",
        )]);
        assert!(
            e.iter().any(|e| e.contains("256 の段の (h4)"))
                && !e.iter().any(|e| e.contains("truecolor の段の (h4)")),
            "{e:?}"
        );
    }

    #[test]
    fn 高コントラストの表_16_の段は_xterm_の側だけの不足も拒否する() {
        // black / bright_blue は xterm で約 4.4、Campbell で約 4.9
        let e = hc_errs(&[(
            "ui.selection",
            "| `ui.selection` | x | (i) | `#000000` / `#9ccfff` | `ansi.16` / `ansi.153` | `black` / `bright_blue` | `reverse` |",
        )]);
        assert!(
            e.iter()
                .any(|e| e.contains("(xterm)") && e.contains("(h2)"))
                && !e.iter().any(|e| e.contains("(Campbell)")),
            "{e:?}"
        );
    }

    #[test]
    fn 高コントラストの表_背景だけの段_空の_mono_重複を拒否する() {
        // (b) 256 の段が背景だけ
        let e = hc_errs(&[(
            "ui.selection",
            "| `ui.selection` | x | (i) | `#000000` / `#9ccfff` | `default` / `ansi.153` | `black` / `bright_cyan` | `reverse` |",
        )]);
        assert!(e.iter().any(|e| e.contains("256 が背景だけ")), "{e:?}");
        // (d) 意味のあるロールの mono が空
        let e = hc_errs(&[(
            "ui.selection",
            "| `ui.selection` | x | (i) | `#000000` / `#9ccfff` | `ansi.16` / `ansi.153` | `black` / `bright_cyan` | `none` |",
        )]);
        assert!(e.iter().any(|e| e.contains("mono の段が空")), "{e:?}");
        // ロールの重複
        let e = hc_errs_with(
            &[],
            &[
                "| `ui.selection` | x | (i) | `#000000` / `#9ccfff` | `ansi.16` / `ansi.153` | `black` / `bright_cyan` | `reverse` |",
            ],
        );
        assert!(e.iter().any(|e| e.contains("重複")), "{e:?}");
    }

    #[test]
    fn 高コントラストの表_淡い表示が本文の前景と同じなら拒否する() {
        // (h10) 16 の段の行番号が本文と同じ bright_white
        let e = hc_errs(&[(
            "ui.line_number",
            "| `ui.line_number` | x | (iii) | `#c0c0c0` / `default` | `ansi.250` / `default` | `bright_white` / `default` | `none` |",
        )]);
        assert!(
            e.iter()
                .any(|e| e.contains("(h10)") && e.contains("16 の段")),
            "{e:?}"
        );
    }

    #[test]
    fn 高コントラストの表_重ねるロールの太字は下のロールの前景で判定する() {
        // 現在の行番号は行番号 (16 の段は white) に重なるので、太字は (h9) に当たる
        let row = |m16: &str| {
            format!("| `ui.line_number.active` | x | (ii) | `bold` | `bold` | {m16} | `bold` |")
        };
        let e = hc_errs_with(&[], &[row("`bold`").as_str()]);
        assert!(
            e.iter().any(|e| e.contains("(h9)")
                && e.contains("ui.line_number.active")
                && e.contains("white")),
            "{e:?}"
        );
        assert_eq!(
            hc_errs_with(&[], &[row("`underline`").as_str()]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn 高コントラストの表_h8_は反転した見た目の背景で比べる() {
        // 書いた背景 (black と blue) は 2.2:1 だが、反転で見える背景はどちらも bright_cyan で、太字の差しか無い
        let e = hc_errs(&[
            (
                "ui.selection",
                "| `ui.selection` | x | (i) | `#000000` / `#9ccfff` | `ansi.16` / `ansi.153` | `bright_cyan` / `black` + `reverse` | `reverse` |",
            ),
            (
                "ui.search.match",
                "| `ui.search.match` | x | (i) | `#000000` / `#ffff00` + `underline` | `ansi.16` / `ansi.226` + `underline` | `bright_cyan` / `blue` + `reverse` + `bold` | `underline` |",
            ),
        ]);
        assert!(
            e.iter()
                .any(|e| e.contains("(h8)") && e.contains("16 の段")),
            "{e:?}"
        );
    }

    #[test]
    fn 高コントラストの表のロール名の集合は_default_の表と同じ() {
        let md = format!("{HC_HEAD}{}\n", hc_rows(&[]).join("\n"));
        let rows = parse_high_contrast(&md, HIGH_CONTRAST_TABLES[0].1).unwrap();
        assert_eq!(check_role_set("x", &rows, &rows), Vec::<String>::new());
        let fewer: Vec<RoleRow> = rows[1..].to_vec();
        let e = check_role_set("x", &fewer, &rows);
        assert!(
            e.len() == 1 && e[0].contains("ui.text") && e[0].contains("無い"),
            "{e:?}"
        );
        let e = check_role_set("x", &rows, &fewer);
        assert!(
            e.len() == 1 && e[0].contains("ui.text") && e[0].contains("無いロール"),
            "{e:?}"
        );
    }

    #[test]
    fn xterm_の_256_色() {
        assert_eq!(xterm256_rgb(16), (0, 0, 0));
        assert_eq!(xterm256_rgb(231), (255, 255, 255));
        assert_eq!(xterm256_rgb(153), (175, 215, 255));
        assert_eq!(xterm256_rgb(232), (8, 8, 8));
        assert_eq!(xterm256_rgb(255), (238, 238, 238));
        assert_eq!(xterm256_rgb(9), (255, 0, 0));
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
