//! テーマの解決 (docs/07 §8・§8.6、04 §12.1 C1): テーマと色深度から、その段のロールの表を作る。
//! 編集面の Span に付けるスタイルと、Frame で frontend に渡すクロームのロールは、どちらもこの表から引く。

use eve_config::theme::{
    Depth, ThemeColor, ThemeStyle, builtin, mods, resolve_roles, with_background,
};
use eve_view::{Color, ColorDepth, RoleStyles, Style, StyleMods};

use super::state::EditorState;

fn color(c: ThemeColor) -> Color {
    match c {
        ThemeColor::Default => Color::Default,
        ThemeColor::Rgb(r, g, b) => Color::Rgb(r, g, b),
        ThemeColor::Ansi(n) => Color::Indexed(n),
    }
}

fn style(s: ThemeStyle) -> Style {
    let mut m = StyleMods::NONE;
    for (bit, sm) in [
        (mods::BOLD, StyleMods::BOLD),
        (mods::UNDERLINE, StyleMods::UNDERLINE),
        (mods::REVERSE, StyleMods::REVERSE),
        (mods::DIM, StyleMods::DIM),
    ] {
        if s.mods & bit != 0 {
            m = m | sm;
        }
    }
    Style {
        fg: color(s.fg),
        bg: color(s.bg),
        mods: m,
        underline_color: None,
    }
}

fn depth(d: ColorDepth) -> Depth {
    match d {
        ColorDepth::Mono => Depth::Mono,
        ColorDepth::Ansi16 => Depth::Ansi16,
        ColorDepth::Ansi256 => Depth::Ansi256,
        _ => Depth::TrueColor,
    }
}

/// テーマ `name` の、色深度 `d` の段のロールの表。未知のテーマは `default`。
pub fn resolve(name: &str, d: ColorDepth) -> RoleStyles {
    let Some(theme) = builtin(name).or_else(|| builtin("default")) else {
        return RoleStyles::default();
    };
    RoleStyles::new(
        resolve_roles(&theme, depth(d))
            .into_iter()
            .map(|(n, s)| (n.to_owned(), style(s)))
            .collect(),
    )
}

impl EditorState {
    /// ロールの表を作り直す (色深度・テーマが変わったとき)。
    pub(super) fn refresh_roles(&mut self) {
        let c = self.config();
        let name = with_background(c.theme_name.as_str(), c.theme_background).to_owned();
        self.roles = std::sync::Arc::new(resolve(&name, self.caps.color_depth));
        self.mark_dirty_render();
    }
}
