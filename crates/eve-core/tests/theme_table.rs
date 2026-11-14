//! 組込みテーマ `default` (eve-config) と core の解決が、docs/07 §8.2 のロール表と一致する (09 §3.5 の `xtask check-themes`)。
//! 表の全ロールの 4 段 (truecolor / 256 / 16 / mono) を、core が Frame で渡す形 (`resolve_theme`) で突き合わせる。
#![expect(
    clippy::cast_possible_truncation,
    reason = "テスト用の数値変換。値は小さく収まる"
)]

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

/// eve-config の段のスタイルを、Frame の `Style` の形にする (塗る前の値の突合用)。
fn raw_style(s: eve_config::theme::ThemeStyle) -> Style {
    use eve_config::theme::{ThemeColor, mods};
    let c = |c: ThemeColor| match c {
        ThemeColor::Default => Color::Default,
        ThemeColor::Rgb(r, g, b) => Color::Rgb(r, g, b),
        ThemeColor::Ansi(n) => Color::Indexed(n),
    };
    let m = [
        (mods::BOLD, StyleMods::BOLD),
        (mods::UNDERLINE, StyleMods::UNDERLINE),
        (mods::REVERSE, StyleMods::REVERSE),
        (mods::DIM, StyleMods::DIM),
    ]
    .into_iter()
    .filter(|(bit, _)| s.mods & bit != 0)
    .fold(StyleMods::NONE, |acc, (_, sm)| acc | sm);
    Style {
        fg: c(s.fg),
        bg: c(s.bg),
        mods: m,
        underline_color: None,
    }
}

/// 背景を塗るテーマの期待値: 色の段では、表の `default` (端末の既定色) を本文 (`ui.text`) の色に置き換える (07 §8.7。
/// 重ねるロールは置き換えない)。
fn painted(want: Style, text: Style) -> Style {
    Style {
        fg: if want.fg == Color::Default {
            text.fg
        } else {
            want.fg
        },
        bg: if want.bg == Color::Default {
            text.bg
        } else {
            want.bg
        },
        ..want
    }
}

// verifies: NFR-604
#[test]
fn 高コントラストのテーマは_07_の表の全ロールの_4_段と一致する() {
    use eve_docs_tables::theme::{HIGH_CONTRAST_TABLES, parse_high_contrast, parse_roles};
    let md = include_str!("../../../docs/07-config-extensibility.md");
    let default_roles: Vec<String> = parse_roles(md)
        .expect("07 §8.2 の表を読めません")
        .into_iter()
        .map(|r| r.role)
        .collect();
    let mut errs = Vec::new();
    for (theme, heading) in HIGH_CONTRAST_TABLES {
        let rows = parse_high_contrast(md, heading).expect("07 §8.7 の表を読めません");
        let mut names: Vec<&str> = rows.iter().map(|r| r.role.as_str()).collect();
        let mut want_names: Vec<&str> = default_roles.iter().map(String::as_str).collect();
        names.sort_unstable();
        want_names.sort_unstable();
        assert_eq!(
            names, want_names,
            "{theme}: ロール名の集合は default と同じ"
        );
        let text_row = rows
            .iter()
            .find(|r| r.role == "ui.text")
            .expect("ui.text の行");
        for (depth, label) in [
            (ColorDepth::TrueColor, "truecolor"),
            (ColorDepth::Ansi256, "256"),
            (ColorDepth::Ansi16, "16"),
            (ColorDepth::Mono, "mono"),
        ] {
            let roles = eve_core::resolve_theme(theme, depth);
            assert_eq!(
                roles.iter().count(),
                rows.len(),
                "{theme}: ロールの数 ({label})"
            );
            let pick = |row: &eve_docs_tables::theme::RoleRow| match label {
                "truecolor" => style_of(&row.truecolor),
                "256" => style_of(&row.ansi256),
                "16" => style_of(&row.ansi16),
                _ => style_of(&row.mono),
            };
            let text = pick(text_row);
            // eve-config の組込みの値そのもの (塗る前。表の `default` は端末の既定色のまま) も表と一致する
            let raw = eve_config::theme::builtin(theme).expect("組込みテーマ");
            let raw_depth = match label {
                "truecolor" => eve_config::theme::Depth::TrueColor,
                "256" => eve_config::theme::Depth::Ansi256,
                "16" => eve_config::theme::Depth::Ansi16,
                _ => eve_config::theme::Depth::Mono,
            };
            for row in &rows {
                let want = pick(row);
                match raw.roles.iter().find(|r| r.name == row.role) {
                    Some(r) => {
                        let got = raw_style(r.at(raw_depth));
                        if got != want {
                            errs.push(format!(
                                "{theme} {} ({label}): eve-config {got:?} / 表 {want:?}",
                                row.role
                            ));
                        }
                    }
                    None => errs.push(format!("{theme} {} ({label}): eve-config に無い", row.role)),
                }
            }
            for row in &rows {
                // 重ねるロールは塗らずに下のロールの色を通す (07 §8.6 の 3 本目)
                let layered = eve_config::theme::LAYERED_ROLES.contains(&row.role.as_str());
                let want = if label == "mono" || layered {
                    pick(row)
                } else {
                    painted(pick(row), text)
                };
                match roles.find(&row.role) {
                    Some(got) if got == want => {}
                    Some(got) => errs.push(format!(
                        "{theme} {} ({label}): 実装 {got:?} / 表 {want:?}",
                        row.role
                    )),
                    None => errs.push(format!("{theme} {} ({label}): 実装に無い", row.role)),
                }
            }
        }
    }
    assert!(errs.is_empty(), "{}", errs.join("\n"));
}

#[test]
fn 高コントラストのテーマは明暗のキーで対の他方へ替わる() {
    use eve_config::ThemeBackground as B;
    let text = |name: &str, bg| roles_with(name, bg).get("ui.text");
    let black = Color::Rgb(0, 0, 0);
    let white = Color::Rgb(0xff, 0xff, 0xff);
    assert_eq!(text("high-contrast-dark", B::Auto).bg, black);
    assert_eq!(text("high-contrast-light", B::Auto).bg, white);
    assert_eq!(text("high-contrast-dark", B::Light).bg, white, "名前と逆");
    assert_eq!(text("high-contrast-light", B::Dark).bg, black, "名前と逆");
    // 塗るテーマなので、既定色だったロールにも本文の色が入る
    let info = roles_with("high-contrast-dark", B::Auto).get("ui.message.info");
    assert_eq!((info.fg, info.bg), (white, black));
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
