//! 組込みテーマ `default` (eve-config) と core の解決が、docs/07 §8.2 のロール表と一致する (09 §3.5 の `xtask check-themes`)。
//! 表の全ロールの 4 段 (truecolor / 256 / 16 / mono) を、core が Frame で渡す形 (`resolve_theme`) で突き合わせる。

use eve_docs_tables::theme::{StyleSpec, hex_rgb, parse_roles};
use eve_view::{Color, ColorDepth, Style, StyleMods};

fn color(c: &Option<String>) -> Color {
    const NAMES: [&str; 8] = [
        "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
    ];
    match c.as_deref() {
        None | Some("default") => Color::Default,
        Some(s) if s.starts_with("ansi.") => Color::Indexed(s[5..].parse().expect("ansi.<n>")),
        Some(s) if s.starts_with('#') => {
            let (r, g, b) = hex_rgb(s).expect("#rrggbb");
            Color::Rgb(r, g, b)
        }
        Some(s) => {
            let (base, name) = match s.strip_prefix("bright_") {
                Some(n) => (8, n),
                None => (0, s),
            };
            let i = NAMES.iter().position(|n| *n == name).expect("16 色の名前");
            Color::Indexed(base + i as u8)
        }
    }
}

fn style_of(s: &StyleSpec) -> Style {
    let mods = s.mods.iter().fold(StyleMods::NONE, |acc, w| {
        acc | match w.as_str() {
            "bold" => StyleMods::BOLD,
            "underline" => StyleMods::UNDERLINE,
            "reverse" => StyleMods::REVERSE,
            "dim" => StyleMods::DIM,
            other => panic!("未知の属性 {other}"),
        }
    });
    Style {
        fg: color(&s.fg),
        bg: color(&s.bg),
        mods,
        underline_color: None,
    }
}

// verifies: AC-FR-513-1
#[test]
fn 組込みテーマ_default_は表の全ロールの_4_段と一致する() {
    let md = include_str!("../../../docs/07-config-extensibility.md");
    let rows = parse_roles(md).expect("07 §8.2 の表を読めません");
    let mut errs = Vec::new();
    for (depth, label) in [
        (ColorDepth::TrueColor, "truecolor"),
        (ColorDepth::Ansi256, "256"),
        (ColorDepth::Ansi16, "16"),
        (ColorDepth::Mono, "mono"),
    ] {
        let roles = eve_core::resolve_theme("default", depth);
        assert_eq!(roles.iter().count(), rows.len(), "ロールの数 ({label})");
        for row in &rows {
            let spec = match label {
                "truecolor" => &row.truecolor,
                "256" => &row.ansi256,
                "16" => &row.ansi16,
                _ => &row.mono,
            };
            let want = style_of(spec);
            match roles.find(&row.role) {
                Some(got) if got == want => {}
                Some(got) => errs.push(format!(
                    "{} ({label}): 実装 {got:?} / 表 {want:?}",
                    row.role
                )),
                None => errs.push(format!("{} ({label}): 実装に無い", row.role)),
            }
        }
    }
    assert!(errs.is_empty(), "{}", errs.join("\n"));
}
