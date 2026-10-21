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
    match c.as_deref() {
        None | Some("default") => Color::Default,
        Some(s) => {
            let (r, g, b) = hex_rgb(s).unwrap_or_else(|| panic!("truecolor の色は #rrggbb: {s}"));
            Color::Rgb(r, g, b)
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

#[test]
fn eve_tui_のロールは表の_truecolor_と_mono_の値と一致する() {
    let rows = rows();
    let mut errs = Vec::new();
    for name in ROLES {
        let Some(row) = rows.iter().find(|r| r.role == name) else {
            errs.push(format!("{name}: 07 §8.2 の表にない (ロール名は公開面)"));
            continue;
        };
        for (label, theme, spec) in [
            ("truecolor", Theme::truecolor(), &row.truecolor),
            ("mono", Theme::mono(), &row.mono),
        ] {
            let got = theme.role(name).expect("ROLES のロールは引ける");
            let want = style_of(spec);
            if got != want {
                errs.push(format!("{name} ({label}): 実装 {got:?} / 表 {want:?}"));
            }
        }
    }
    assert!(errs.is_empty(), "{}", errs.join("\n"));
}

#[test]
fn ロール名の対応表は引き漏れがない() {
    let t = Theme::truecolor();
    for name in ROLES {
        assert!(t.role(name).is_some(), "{name}");
    }
    assert!(t.role("ui.nope").is_none());
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
