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

fn roles_with(name: &str, bg: eve_config::ThemeBackground) -> eve_view::RoleStyles {
    let mut h = eve_testkit::Harness::new();
    h.editor.state_mut().set_config(eve_config::Config {
        theme_name: name.to_owned(),
        theme_background: bg,
        ..eve_config::Config::default()
    });
    (*h.frame().roles).clone()
}

#[test]
fn theme_name_で組込みテーマを選び_背景を塗るテーマは明暗をキーで替えられる() {
    use eve_config::ThemeBackground as B;
    let dark = roles_with("default-dark", B::Auto).get("ui.text");
    assert_eq!(dark.bg, Color::Rgb(0x1e, 0x1e, 0x1e));
    // 塗るテーマは、既定色だったロールにも本文の背景が入る
    let info = roles_with("default-dark", B::Auto).get("ui.message.info");
    assert_eq!(info.bg, dark.bg);
    let light = roles_with("default-dark", B::Light).get("ui.text");
    assert_eq!(light.bg, Color::Rgb(0xfa, 0xfa, 0xfa), "明暗のキーで替わる");
    // default は明暗のキーを見ない
    assert_eq!(
        roles_with("default", B::Dark).get("ui.text").bg,
        Color::Default
    );
    // ansi は 16 色の番号だけ
    let a = roles_with("ansi", B::Auto);
    assert!(
        a.iter()
            .all(|(_, s)| !matches!(s.fg, Color::Rgb(..)) && !matches!(s.bg, Color::Rgb(..)))
    );
}

#[test]
fn 背景を塗るテーマの本文はコントラストを満たす() {
    use eve_docs_tables::theme::contrast;
    for (fg, bg) in [
        ((0xd4, 0xd4, 0xd4), (0x1e, 0x1e, 0x1e)),
        ((0x1e, 0x1e, 0x1e), (0xfa, 0xfa, 0xfa)),
    ] {
        assert!(contrast(fg, bg) >= 4.5, "{fg:?} / {bg:?}");
    }
    // (iii) の淡色 (#808080) も、塗った背景に対して 3.0:1 以上
    for bg in [(0x1e, 0x1e, 0x1e), (0xfa, 0xfa, 0xfa)] {
        assert!(contrast((0x80, 0x80, 0x80), bg) >= 3.0, "{bg:?}");
    }
}

fn reload_with(h: &mut eve_testkit::Harness, config: &str) {
    h.fs.add_file("/evo/config/config.toml", config.as_bytes());
    h.cmd("config.reload");
    h.run_jobs();
}

#[test]
fn テーマファイルを読み_書いたロールだけを上書きする() {
    let mut h = eve_testkit::Harness::new();
    h.fs.add_file(
        "/evo/config/themes/mine.toml",
        b"[meta]\ninherits = \"default-dark\"\n[palette]\nhot = \"#ff3366\"\n[ui]\nselection = { fg = \"#000\", bg = \"hot\" }\n",
    );
    reload_with(&mut h, "[theme]\nname = \"mine\"\n");
    let roles = h.frame().roles;
    assert_eq!(roles.get("ui.selection").bg, Color::Rgb(0xff, 0x33, 0x66));
    assert_eq!(
        roles.get("ui.text").bg,
        Color::Rgb(0x1e, 0x1e, 0x1e),
        "default-dark を継ぐ"
    );
    // ファイルを直して読み直すと替わる
    h.fs.add_file(
        "/evo/config/themes/mine.toml",
        b"[ui]\nselection = { bg = \"#112233\" }\n",
    );
    h.cmd("config.reload");
    h.run_jobs();
    let roles = h.frame().roles;
    assert_eq!(roles.get("ui.selection").bg, Color::Rgb(0x11, 0x22, 0x33));
    assert_eq!(
        roles.get("ui.text").bg,
        Color::Default,
        "inherits を外したので default"
    );
}

#[test]
fn 読めないテーマは_default_で動き_問題を知らせる() {
    let mut h = eve_testkit::Harness::new();
    reload_with(&mut h, "[theme]\nname = \"nothere\"\n");
    let roles = h.frame().roles;
    assert_eq!(
        roles.get("ui.selection"),
        eve_core::resolve_theme("default", ColorDepth::TrueColor).get("ui.selection")
    );
    assert!(
        h.messages().iter().any(|m| m.starts_with("msg.config.")),
        "{:?}",
        h.messages()
    );
}
