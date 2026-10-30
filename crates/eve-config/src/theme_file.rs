//! テーマファイル (`<設定ディレクトリ>/themes/<name>.toml`、docs/07 §8.2〜§8.4・§8.6、01 FR-707)。
//!
//! 書式だけを解いて `Theme` (ロール → 4 段) を組み立てる。ファイルの読み込みは呼び出し側 (起動時は bin、
//! `config.reload` は core の io ジョブ) が `read` で渡す。`inherits` は親から順に重ね (深さ 8 まで、循環はエラー)、
//! どこにも `inherits` が無ければ組込みの `default` を土台にする。エラー (読めない・構文・循環・深さ) のときは
//! `None` を返し、呼び出し側は組込みの `default` で動く (FR-703)。値の誤り (未知のロール・色・パレット名) は警告で、
//! その値だけ使わない。

use std::collections::BTreeMap;

use toml::Spanned;
use toml::de::{DeTable, DeValue};

use crate::theme::{Role, Theme, ThemeColor, ThemeStyle, builtin, mods};
use crate::{DiagKind, Diagnostic, Severity};

/// `inherits` の深さの上限 (07 §8.3)。
const MAX_DEPTH: usize = 8;

/// xterm 既定の 16 色パレット (04 §12.3)。
const ANSI16: [(u8, u8, u8); 16] = [
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
const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];
const NAMES: [&str; 8] = [
    "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
];

fn xterm_rgb(index: u8) -> (u8, u8, u8) {
    match index {
        0..=15 => ANSI16[index as usize],
        16..=231 => {
            let i = index - 16;
            (
                CUBE[(i / 36) as usize],
                CUBE[((i / 6) % 6) as usize],
                CUBE[(i % 6) as usize],
            )
        }
        232..=255 => {
            let v = 8 + 10 * (index - 232);
            (v, v, v)
        }
    }
}

/// 重み付き RGB 距離 (04 §12.3。eve-tui の最終防衛線と同じ式)。
fn distance(a: (u8, u8, u8), b: (u8, u8, u8)) -> i64 {
    let rmean = (i64::from(a.0) + i64::from(b.0)) / 2;
    let (dr, dg, db) = (
        i64::from(a.0) - i64::from(b.0),
        i64::from(a.1) - i64::from(b.1),
        i64::from(a.2) - i64::from(b.2),
    );
    (((512 + rmean) * dr * dr) >> 8) + 4 * dg * dg + (((767 - rmean) * db * db) >> 8)
}

/// 色を 256 色の段に近似する (RGB はキューブとグレーから。番号はそのまま)。
fn to256(c: ThemeColor) -> ThemeColor {
    match c {
        ThemeColor::Rgb(r, g, b) => ThemeColor::Ansi(
            (16..=255u8)
                .min_by_key(|&i| (distance((r, g, b), xterm_rgb(i)), i))
                .unwrap_or(16),
        ),
        _ => c,
    }
}

/// 色を 16 色の段に近似する。
fn to16(c: ThemeColor) -> ThemeColor {
    let rgb = match c {
        ThemeColor::Rgb(r, g, b) => (r, g, b),
        ThemeColor::Ansi(i) if i >= 16 => xterm_rgb(i),
        _ => return c,
    };
    ThemeColor::Ansi(
        (0..16u8)
            .min_by_key(|&i| (distance(rgb, ANSI16[i as usize]), i))
            .unwrap_or(0),
    )
}

/// 近似した段で前景と背景が同じ番号になったら、前景を外して反転にする (04 §12.3)。
fn fix_same(mut s: ThemeStyle) -> ThemeStyle {
    if s.fg != ThemeColor::Default && s.fg == s.bg {
        s.fg = ThemeColor::Default;
        s.bg = ThemeColor::Default;
        s.mods |= mods::REVERSE;
    }
    s
}

/// 1 つのロールの書き換え (書かれたものだけ `Some`)。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct RoleSpec {
    fg: Option<ThemeColor>,
    bg: Option<ThemeColor>,
    mods: Option<u8>,
    fallback256: Option<(Option<ThemeColor>, Option<ThemeColor>)>,
    fallback16: Option<(Option<ThemeColor>, Option<ThemeColor>)>,
    mono: Option<u8>,
}

/// 解析したテーマファイル 1 つ。
#[derive(Clone, Debug, Default)]
struct Parsed {
    inherits: Option<String>,
    roles: Vec<(String, RoleSpec)>,
}

struct Cx<'a> {
    text: &'a str,
    diags: Vec<Diagnostic>,
    palette: BTreeMap<String, ThemeColor>,
}

impl Cx<'_> {
    fn pos(&self, offset: usize) -> (u32, u32) {
        let head = self.text.get(..offset.min(self.text.len())).unwrap_or("");
        let line = head.matches('\n').count() + 1;
        let col = head.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
        (line as u32, col as u32)
    }

    fn warn(&mut self, kind: DiagKind, key: &str, value: &str, at: usize) {
        let (line, column) = self.pos(at);
        self.diags.push(Diagnostic {
            severity: Severity::Warning,
            kind,
            key: key.to_owned(),
            value: value.to_owned(),
            line,
            column,
        });
    }

    fn info(&mut self, key: &str, at: usize) {
        let (line, column) = self.pos(at);
        self.diags.push(Diagnostic {
            severity: Severity::Info,
            kind: DiagKind::Unsupported,
            key: key.to_owned(),
            value: String::new(),
            line,
            column,
        });
    }

    /// 色の表記 (07 §8.2 の「色」の文法)。読めなければ警告して `None`。
    fn color(&mut self, key: &str, v: &Spanned<DeValue<'_>>) -> Option<ThemeColor> {
        let DeValue::String(s) = v.get_ref() else {
            self.warn(
                DiagKind::TypeMismatch {
                    expected: "string",
                    found: v.get_ref().type_str(),
                },
                key,
                "",
                v.span().start,
            );
            return None;
        };
        let s: &str = s.as_ref();
        let c = parse_color(s).or_else(|| self.palette.get(s).copied());
        if c.is_none() {
            self.warn(
                DiagKind::InvalidValue {
                    allowed: &["#rrggbb", "#rgb", "ansi.<name>", "ansi.<0-255>", "default"],
                },
                key,
                s,
                v.span().start,
            );
        }
        c
    }

    fn mods(&mut self, key: &str, v: &Spanned<DeValue<'_>>) -> Option<u8> {
        let DeValue::Array(items) = v.get_ref() else {
            self.warn(
                DiagKind::TypeMismatch {
                    expected: "array",
                    found: v.get_ref().type_str(),
                },
                key,
                "",
                v.span().start,
            );
            return None;
        };
        let mut m = mods::NONE;
        for it in items {
            let word = match it.get_ref() {
                DeValue::String(s) => s.to_string(),
                _ => String::new(),
            };
            m |= match word.as_str() {
                "bold" => mods::BOLD,
                "underline" => mods::UNDERLINE,
                "reverse" => mods::REVERSE,
                "dim" => mods::DIM,
                // 端末によって効かない属性は、v0.2 では受け付けて使わない
                "italic" | "strikethrough" => mods::NONE,
                _ => {
                    self.warn(
                        DiagKind::InvalidValue {
                            allowed: &[
                                "bold",
                                "underline",
                                "reverse",
                                "dim",
                                "italic",
                                "strikethrough",
                            ],
                        },
                        key,
                        &word,
                        it.span().start,
                    );
                    mods::NONE
                }
            };
        }
        Some(m)
    }

    fn pair(&mut self, key: &str, t: &DeTable<'_>) -> (Option<ThemeColor>, Option<ThemeColor>) {
        let mut out = (None, None);
        for (k, v) in t {
            let name: &str = k.get_ref();
            match name {
                "fg" => out.0 = self.color(&format!("{key}.fg"), v),
                "bg" => out.1 = self.color(&format!("{key}.bg"), v),
                _ => self.warn(
                    DiagKind::UnknownKey { suggestion: None },
                    &format!("{key}.{name}"),
                    "",
                    k.span().start,
                ),
            }
        }
        out
    }

    fn role(&mut self, key: &str, t: &DeTable<'_>) -> RoleSpec {
        let mut r = RoleSpec::default();
        for (k, v) in t {
            let name: &str = k.get_ref();
            let full = format!("{key}.{name}");
            match (name, v.get_ref()) {
                ("fg", _) => r.fg = self.color(&full, v),
                ("bg", _) => r.bg = self.color(&full, v),
                ("mods", _) => r.mods = self.mods(&full, v),
                ("fallback256", DeValue::Table(t)) => r.fallback256 = Some(self.pair(&full, t)),
                ("fallback16", DeValue::Table(t)) => r.fallback16 = Some(self.pair(&full, t)),
                ("mono", DeValue::Table(t)) => {
                    r.mono = t
                        .get("mods")
                        .and_then(|m| self.mods(&format!("{full}.mods"), m))
                        .or(Some(mods::NONE));
                }
                // 下線の種類・色は v0.4 (診断の表示と一緒)
                ("underline" | "underline_color", _) => self.info(&full, k.span().start),
                _ => self.warn(
                    DiagKind::UnknownKey { suggestion: None },
                    &full,
                    "",
                    k.span().start,
                ),
            }
        }
        r
    }
}

/// 色の表記。パレットの名前は呼び出し側が引く。
fn parse_color(s: &str) -> Option<ThemeColor> {
    if s == "default" {
        return Some(ThemeColor::Default);
    }
    if let Some(hex) = s.strip_prefix('#') {
        let v = |h: &str| u8::from_str_radix(h, 16).ok();
        return match hex.len() {
            6 => Some(ThemeColor::Rgb(
                v(&hex[0..2])?,
                v(&hex[2..4])?,
                v(&hex[4..6])?,
            )),
            3 => {
                let d = |i: usize| v(&hex[i..i + 1]).map(|x| x * 17);
                Some(ThemeColor::Rgb(d(0)?, d(1)?, d(2)?))
            }
            _ => None,
        };
    }
    let name = s.strip_prefix("ansi.")?;
    if let Ok(n) = name.parse::<u8>() {
        return Some(ThemeColor::Ansi(n));
    }
    let (base, n) = match name.strip_prefix("bright_") {
        Some(n) => (8, n),
        None => (0, name),
    };
    NAMES
        .iter()
        .position(|x| *x == n)
        .map(|i| ThemeColor::Ansi(base + i as u8))
}

/// テーマファイル 1 つを解く。構文エラーなら `None` と診断。
fn parse(text: &str) -> (Option<Parsed>, Vec<Diagnostic>) {
    let mut cx = Cx {
        text,
        diags: Vec::new(),
        palette: BTreeMap::new(),
    };
    let root = match DeTable::parse(text) {
        Ok(t) => t,
        Err(e) => {
            let (line, column) = e.span().map_or((0, 0), |s| cx.pos(s.start));
            cx.diags.push(Diagnostic {
                severity: Severity::Error,
                kind: DiagKind::Syntax {
                    message: e.message().to_owned(),
                },
                key: String::new(),
                value: String::new(),
                line,
                column,
            });
            return (None, cx.diags);
        }
    };
    let root = root.get_ref();
    let mut out = Parsed::default();
    // [palette] を先に読む ([ui] から名前で引くため)
    if let Some(DeValue::Table(p)) = root.get("palette").map(|v| v.get_ref()) {
        for (k, v) in p {
            let key = format!("palette.{}", k.get_ref() as &str);
            if let Some(c) = cx.color(&key, v) {
                cx.palette.insert(k.get_ref().to_string(), c);
            }
        }
    }
    for (k, v) in root {
        let name: &str = k.get_ref();
        match (name, v.get_ref()) {
            ("palette", _) => {}
            ("meta", DeValue::Table(m)) => {
                if let Some(DeValue::String(s)) = m.get("inherits").map(|v| v.get_ref()) {
                    out.inherits = Some(s.to_string());
                }
            }
            ("ui", DeValue::Table(ui)) => {
                for (rk, rv) in ui {
                    let role = format!("ui.{}", rk.get_ref() as &str);
                    match rv.get_ref() {
                        DeValue::Table(t) => {
                            let spec = cx.role(&role, t);
                            out.roles.push((role, spec));
                        }
                        other => cx.warn(
                            DiagKind::TypeMismatch {
                                expected: "table",
                                found: other.type_str(),
                            },
                            &role,
                            "",
                            rv.span().start,
                        ),
                    }
                }
            }
            // シンタックスのスコープは v0.4 (tree-sitter と一緒)
            ("syntax", _) => cx.info(name, k.span().start),
            _ => cx.warn(
                DiagKind::UnknownKey { suggestion: None },
                name,
                "",
                k.span().start,
            ),
        }
    }
    (Some(out), cx.diags)
}

/// `base` のロールに `spec` を重ねる。256 / 16 の段は、明示が無く色を書き換えたときだけ近似で求める。
fn apply(base: Role, spec: &RoleSpec) -> Role {
    let mut tc = base.truecolor;
    if let Some(c) = spec.fg {
        tc.fg = c;
    }
    if let Some(c) = spec.bg {
        tc.bg = c;
    }
    if let Some(m) = spec.mods {
        tc.mods = m;
    }
    let recolored = spec.fg.is_some() || spec.bg.is_some();
    let stage = |old: ThemeStyle,
                 explicit: Option<(Option<ThemeColor>, Option<ThemeColor>)>,
                 approx: fn(ThemeColor) -> ThemeColor| {
        match explicit {
            Some((fg, bg)) => ThemeStyle {
                fg: fg.unwrap_or(approx(tc.fg)),
                bg: bg.unwrap_or(approx(tc.bg)),
                mods: tc.mods,
            },
            None if recolored || spec.mods.is_some() => fix_same(ThemeStyle {
                fg: approx(tc.fg),
                bg: approx(tc.bg),
                mods: tc.mods,
            }),
            None => old,
        }
    };
    Role {
        name: base.name,
        truecolor: tc,
        ansi256: stage(base.ansi256, spec.fallback256, to256),
        ansi16: stage(base.ansi16, spec.fallback16, to16),
        mono: spec.mono.unwrap_or(base.mono),
    }
}

/// テーマ `name` を読む。`read(name)` は `themes/<name>.toml` の本文 (無ければ `Ok(None)`、読めなければ理由) を返す。
/// 戻り値は (テーマ、ファイルごとの診断)。テーマが `None` なら組込みの `default` を使う。組込みの名前はファイルを読まない。
pub fn load_theme(
    name: &str,
    read: &dyn Fn(&str) -> Result<Option<String>, String>,
) -> (Option<Theme>, Vec<(String, Vec<Diagnostic>)>) {
    let mut reports: Vec<(String, Vec<Diagnostic>)> = Vec::new();
    let error = |file: &str, message: String, reports: &mut Vec<(String, Vec<Diagnostic>)>| {
        reports.push((
            file.to_owned(),
            vec![Diagnostic {
                severity: Severity::Error,
                kind: DiagKind::Syntax { message },
                key: String::new(),
                value: String::new(),
                line: 0,
                column: 0,
            }],
        ));
    };
    // 子から親へたどる
    let mut chain: Vec<Parsed> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let mut cur = name.to_owned();
    let base = loop {
        if let Some(t) = builtin(&cur) {
            break t;
        }
        let file = format!("themes/{cur}.toml");
        if seen.contains(&cur) {
            error(
                &file,
                format!("inherits が循環しています ({cur})"),
                &mut reports,
            );
            return (None, reports);
        }
        if seen.len() >= MAX_DEPTH {
            error(
                &file,
                format!("inherits の深さが {MAX_DEPTH} を超えています"),
                &mut reports,
            );
            return (None, reports);
        }
        if !crate::is_theme_name(&cur) {
            error(
                &file,
                format!("テーマの名前として使えません ({cur})"),
                &mut reports,
            );
            return (None, reports);
        }
        seen.push(cur.clone());
        let text = match read(&cur) {
            Ok(Some(t)) => t,
            Ok(None) => {
                error(&file, format!("テーマ {cur} がありません"), &mut reports);
                return (None, reports);
            }
            Err(e) => {
                error(&file, e, &mut reports);
                return (None, reports);
            }
        };
        let (parsed, diags) = parse(text.strip_prefix('\u{feff}').unwrap_or(&text));
        let failed = parsed.is_none();
        if !diags.is_empty() {
            reports.push((file, diags));
        }
        let Some(p) = parsed else {
            debug_assert!(failed);
            return (None, reports);
        };
        cur = p.inherits.clone().unwrap_or_else(|| "default".to_owned());
        chain.push(p);
    };
    let mut roles = base.roles;
    // 親から子の順に重ねる
    for p in chain.iter().rev() {
        for (role, spec) in &p.roles {
            match roles.iter_mut().find(|r| r.name == role) {
                Some(r) => *r = apply(*r, spec),
                None => {
                    let file = format!("themes/{name}.toml");
                    let d = Diagnostic {
                        severity: Severity::Warning,
                        kind: DiagKind::UnknownKey { suggestion: None },
                        key: role.clone(),
                        value: String::new(),
                        line: 0,
                        column: 0,
                    };
                    match reports.iter_mut().find(|(f, _)| *f == file) {
                        Some((_, ds)) => ds.push(d),
                        None => reports.push((file, vec![d])),
                    }
                }
            }
        }
    }
    let paints = base.paints
        || roles
            .iter()
            .find(|r| r.name == "ui.text")
            .is_some_and(|r| r.truecolor.bg != ThemeColor::Default);
    (Some(Theme { roles, paints }), reports)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Depth, resolve_roles};

    fn files(
        list: &'static [(&'static str, &'static str)],
    ) -> impl Fn(&str) -> Result<Option<String>, String> {
        move |n: &str| {
            Ok(list
                .iter()
                .find(|(k, _)| *k == n)
                .map(|(_, v)| (*v).to_owned()))
        }
    }

    fn role(t: &Theme, name: &str, d: Depth) -> ThemeStyle {
        resolve_roles(t, d)
            .into_iter()
            .find(|(n, _)| *n == name)
            .unwrap()
            .1
    }

    #[test]
    fn パレットの名前と色の表記を読み_書いたロールだけを上書きする() {
        let read = files(&[(
            "mine",
            "[palette]\naccent = \"#3366cc\"\n[ui]\nselection = { fg = \"#fff\", bg = \"accent\", fallback16 = { fg = \"ansi.bright_white\", bg = \"ansi.blue\" }, mono = { mods = [\"reverse\", \"bold\"] } }\n",
        )]);
        let (t, reports) = load_theme("mine", &read);
        assert!(reports.is_empty(), "{reports:?}");
        let t = t.unwrap();
        let s = role(&t, "ui.selection", Depth::TrueColor);
        assert_eq!(
            (s.fg, s.bg),
            (
                ThemeColor::Rgb(255, 255, 255),
                ThemeColor::Rgb(0x33, 0x66, 0xcc)
            )
        );
        let s16 = role(&t, "ui.selection", Depth::Ansi16);
        assert_eq!(
            (s16.fg, s16.bg),
            (ThemeColor::Ansi(15), ThemeColor::Ansi(4)),
            "明示の 16 色"
        );
        assert!(
            matches!(
                role(&t, "ui.selection", Depth::Ansi256).bg,
                ThemeColor::Ansi(16..)
            ),
            "256 は近似"
        );
        assert_eq!(
            role(&t, "ui.selection", Depth::Mono).mods,
            mods::REVERSE | mods::BOLD
        );
        // 書いていないロールは default のまま
        let d = crate::theme::builtin("default").unwrap();
        assert_eq!(
            role(&t, "ui.status_bar", Depth::TrueColor),
            role(&d, "ui.status_bar", Depth::TrueColor)
        );
    }

    #[test]
    fn inherits_は親から重ね_組込みのテーマも親にできる() {
        let read = files(&[
            (
                "child",
                "[meta]\ninherits = \"parent\"\n[ui]\nselection = { bg = \"#000000\" }\n",
            ),
            (
                "parent",
                "[meta]\ninherits = \"default-dark\"\n[ui]\nselection = { fg = \"#ff0000\", bg = \"#00ff00\" }\n",
            ),
        ]);
        let (t, _) = load_theme("child", &read);
        let t = t.unwrap();
        let s = role(&t, "ui.selection", Depth::TrueColor);
        assert_eq!(
            (s.fg, s.bg),
            (ThemeColor::Rgb(255, 0, 0), ThemeColor::Rgb(0, 0, 0))
        );
        assert!(t.paints, "default-dark を継ぐので背景を塗る");
    }

    #[test]
    fn 循環_無いファイル_構文エラーは_none_で理由を返す() {
        let read = files(&[
            ("a", "[meta]\ninherits = \"b\"\n"),
            ("b", "[meta]\ninherits = \"a\"\n"),
            ("broken", "[ui\n"),
        ]);
        for name in ["a", "nope", "broken"] {
            let (t, reports) = load_theme(name, &read);
            assert!(t.is_none(), "{name}");
            assert!(
                reports
                    .iter()
                    .any(|(_, d)| d.iter().any(|d| d.severity == Severity::Error)),
                "{name}: {reports:?}"
            );
        }
    }

    #[test]
    fn 値の誤りは警告して_その値だけ使わない() {
        let read = files(&[(
            "w",
            "[ui]\nselection = { fg = \"nope\", bg = \"#123456\" }\nnot_a_role = { fg = \"#ffffff\" }\n[syntax]\ncomment = { fg = \"#888888\" }\n",
        )]);
        let (t, reports) = load_theme("w", &read);
        let t = t.unwrap();
        assert_eq!(
            role(&t, "ui.selection", Depth::TrueColor).bg,
            ThemeColor::Rgb(0x12, 0x34, 0x56)
        );
        let all: Vec<&Diagnostic> = reports.iter().flat_map(|(_, d)| d).collect();
        assert!(
            all.iter()
                .any(|d| d.key == "ui.selection.fg" && d.severity == Severity::Warning)
        );
        assert!(all.iter().any(|d| d.key == "ui.not_a_role"));
        assert!(
            all.iter()
                .any(|d| d.key == "syntax" && d.severity == Severity::Info)
        );
    }

    #[test]
    fn 組込みの名前はファイルを読まない() {
        let (t, reports) = load_theme("ansi", &|_| panic!("読まない"));
        assert!(t.is_some() && reports.is_empty());
    }
}
