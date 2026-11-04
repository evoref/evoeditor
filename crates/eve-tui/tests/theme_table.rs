//! 組込みテーマ `default` の実装と、docs/07 §8.2 のロール表の突合 (docs/09 §3.5 の `xtask check-themes` の (e)(f))。
//! eve-tui が描く領域のロール (`theme::ROLES`) は、表の truecolor と mono の段と値まで一致すること。表にあって eve-tui に
//! ないロールは core が解決する編集面のロールで、core の実装 (M3 以降) で別に突合する。

use std::path::PathBuf;

use eve_docs_tables::theme::{RoleRow, StyleSpec, hex_rgb, parse_roles};
use eve_tui::color::{clamp_style, rgb_to_ansi16, rgb_to_ansi256};
use eve_tui::theme::{ROLES, Theme};

use eve_view::{Color, ColorDepth, Style, StyleMods};

fn rows() -> Vec<RoleRow> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/07-config-extensibility.md");
    let md = std::fs::read_to_string(path).expect("docs/07 を読めません");
    parse_roles(&md).expect("07 §8.2 の表を読めません")
}

fn color(c: &Option<String>) -> Color {
    const NAMES: [&str; 8] = [
        "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
    ];
    match c.as_deref() {
        None | Some("default") => Color::Default,
        Some(s) if s.starts_with("ansi.") => Color::Indexed(s[5..].parse().expect("ansi.<n>")),
        Some(s) if s.starts_with('#') => {
            let (r, g, b) = hex_rgb(s).unwrap_or_else(|| panic!("#rrggbb: {s}"));
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

fn mods(m: &[String]) -> StyleMods {
    m.iter().fold(StyleMods::NONE, |acc, w| {
        acc | match w.as_str() {
            "bold" => StyleMods::BOLD,
            "underline" => StyleMods::UNDERLINE,
            "reverse" => StyleMods::REVERSE,
            "dim" => StyleMods::DIM,
            other => panic!("未知の属性 {other}"),
        }
    })
}

fn style_of(s: &StyleSpec) -> Style {
    Style {
        fg: color(&s.fg),
        bg: color(&s.bg),
        mods: mods(&s.mods),
        underline_color: None,
    }
}

// verifies: AC-FR-513-1
// verifies: AC-FR-513-2
// verifies: AC-NFR-604-1
#[test]
fn 組込みテーマの表は全ロールが_truecolor_と_mono_を持ちコントラストを満たす() {
    let rows = rows();
    let errs = eve_docs_tables::theme::check_table(&rows);
    assert!(
        errs.is_empty(),
        "{}",
        errs.join(
            "
"
        )
    );
}

#[test]
fn eve_tui_が引くロールは表にあり_core_が渡す表から引ける() {
    let rows = rows();
    for name in ROLES {
        assert!(
            rows.iter().any(|r| r.role == name),
            "{name}: 07 §8.2 の表にない (ロール名は公開面)"
        );
    }
    // core が Frame で渡す表 (色深度ごと) から、Theme の各欄が表の値どおりに引ける
    for (depth, pick) in [
        (ColorDepth::TrueColor, 0),
        (ColorDepth::Ansi256, 1),
        (ColorDepth::Ansi16, 2),
        (ColorDepth::Mono, 3),
    ] {
        let mut h = eve_testkit::Harness::new().with_caps(eve_view::Capabilities {
            color_depth: depth,
            ..eve_view::Capabilities::default()
        });
        let f = h.frame();
        let theme = Theme::from_roles(&f.roles);
        let row = rows.iter().find(|r| r.role == "ui.status_bar").unwrap();
        let spec = [&row.truecolor, &row.ansi256, &row.ansi16, &row.mono][pick];
        assert_eq!(theme.status_bar, style_of(spec), "{depth:?}");
        for name in ROLES {
            assert!(f.roles.find(name).is_some(), "{name} ({depth:?})");
        }
    }
}

#[test]
fn 近似した_256_と_16_の段で組の前景と背景が同じ番号にならない() {
    // 09 §3.5 (f): 前景と背景が同じ番号になる組は、近似の規則 (04 §12.3) で反転に置き換わるので、置き換えが起きないこと
    for row in rows() {
        let t = &row.truecolor;
        if !(t.has_fg() && t.has_bg()) {
            continue;
        }
        let (Some(f), Some(b)) = (
            t.fg.as_deref().and_then(hex_rgb),
            t.bg.as_deref().and_then(hex_rgb),
        ) else {
            continue;
        };
        assert_ne!(rgb_to_ansi256(f), rgb_to_ansi256(b), "{} (256)", row.role);
        assert_ne!(rgb_to_ansi16(f), rgb_to_ansi16(b), "{} (16)", row.role);
        // clamp_style も同じ結論 (反転への置換が起きない)
        let st = style_of(t);
        for depth in [ColorDepth::Ansi256, ColorDepth::Ansi16] {
            let (out, _) = clamp_style(st, depth);
            assert_eq!(
                out.mods, st.mods,
                "{} ({depth:?}) で反転に置き換わった",
                row.role
            );
        }
    }
}
