//! 色の縮退 (docs/04 §12.3)。段の選択は core が行い、Frame には端末が描ける色だけが載る (C1)。ここは最終防衛線で、
//! `Capabilities.color_depth` の段を超える色が Frame に来たときだけ、そのセルを近似で描く (常用しない)。

use eve_view::{Color, ColorDepth, Style, StyleMods};

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

/// 6×6×6 キューブの各軸の値。
const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// xterm の 256 色の番号 → RGB。
pub fn xterm_rgb(index: u8) -> (u8, u8, u8) {
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

/// 重み付き RGB 距離 (redmean。決定的で小さい実装、04 §12.3)。
fn distance(a: (u8, u8, u8), b: (u8, u8, u8)) -> i64 {
    let rmean = (i64::from(a.0) + i64::from(b.0)) / 2;
    let (dr, dg, db) = (
        i64::from(a.0) - i64::from(b.0),
        i64::from(a.1) - i64::from(b.1),
        i64::from(a.2) - i64::from(b.2),
    );
    (((512 + rmean) * dr * dr) >> 8) + 4 * dg * dg + (((767 - rmean) * db * db) >> 8)
}

/// RGB に最も近い 256 色の番号 (キューブ 216 色 + グレー 24 階調。0..=15 は端末のパレット次第なので選ばない)。
pub fn rgb_to_ansi256(rgb: (u8, u8, u8)) -> u8 {
    (16..=255u8)
        .min_by_key(|&i| (distance(rgb, xterm_rgb(i)), i))
        .unwrap_or(16)
}

/// RGB に最も近い 16 色の番号 (xterm 既定のパレットとの距離)。
pub fn rgb_to_ansi16(rgb: (u8, u8, u8)) -> u8 {
    (0..16u8)
        .min_by_key(|&i| (distance(rgb, ANSI16[i as usize]), i))
        .unwrap_or(0)
}

/// 色を段に収める。収められなかった (近似した) ときは `true` も返す。
fn clamp_color(c: Color, depth: ColorDepth) -> (Color, bool) {
    match (c, depth) {
        (Color::Default, _) => (c, false),
        (_, ColorDepth::Mono) => (Color::Default, true),
        (_, ColorDepth::TrueColor) => (c, false),
        (Color::Rgb(r, g, b), ColorDepth::Ansi256) => {
            (Color::Indexed(rgb_to_ansi256((r, g, b))), true)
        }
        (Color::Rgb(r, g, b), ColorDepth::Ansi16) => {
            (Color::Indexed(rgb_to_ansi16((r, g, b))), true)
        }
        (Color::Indexed(_), ColorDepth::Ansi256) => (c, false),
        (Color::Indexed(i), ColorDepth::Ansi16) if i < 16 => (c, false),
        (Color::Indexed(i), ColorDepth::Ansi16) => {
            (Color::Indexed(rgb_to_ansi16(xterm_rgb(i))), true)
        }
        // 将来の変種は、段を超えるかが分からないので捨てる
        (_, _) => (Color::Default, true),
    }
}

/// スタイルを `depth` の段に収める。近似したときは件数 (debug ログ用) のために `true` を返す。
/// 近似の結果、前景と背景が同じ番号になったときは、前景を反転属性に置き換える (読めなくなるのを防ぐ)。
pub fn clamp_style(style: Style, depth: ColorDepth) -> (Style, bool) {
    let (fg, a) = clamp_color(style.fg, depth);
    let (bg, b) = clamp_color(style.bg, depth);
    let (underline_color, c) = match style.underline_color {
        Some(u) => {
            let (u, c) = clamp_color(u, depth);
            (Some(u), c)
        }
        None => (None, false),
    };
    let mut out = Style {
        fg,
        bg,
        mods: style.mods,
        underline_color,
    };
    if (a || b) && fg != Color::Default && fg == bg {
        out.fg = Color::Default;
        out.mods = out.mods | StyleMods::REVERSE;
    }
    (out, a || b || c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xterm_の番号と_rgb() {
        assert_eq!(xterm_rgb(0), (0, 0, 0));
        assert_eq!(xterm_rgb(9), (0xff, 0, 0));
        assert_eq!(xterm_rgb(16), (0, 0, 0));
        assert_eq!(xterm_rgb(21), (0, 0, 255));
        assert_eq!(xterm_rgb(196), (255, 0, 0));
        assert_eq!(xterm_rgb(231), (255, 255, 255));
        assert_eq!(xterm_rgb(232), (8, 8, 8));
        assert_eq!(xterm_rgb(255), (238, 238, 238));
    }

    #[test]
    fn rgb_から_256_色への近似() {
        assert_eq!(rgb_to_ansi256((255, 0, 0)), 196);
        assert_eq!(rgb_to_ansi256((255, 255, 255)), 231);
        assert_eq!(rgb_to_ansi256((0, 0, 0)), 16);
        // グレーは階調に寄る
        let g = rgb_to_ansi256((128, 128, 128));
        assert!((232..=255).contains(&g) || g == 244, "{g}");
        // 決定的 (同じ入力は同じ出力)
        assert_eq!(rgb_to_ansi256((59, 91, 181)), rgb_to_ansi256((59, 91, 181)));
    }

    #[test]
    fn rgb_から_16_色への近似() {
        assert_eq!(rgb_to_ansi16((255, 0, 0)), 9);
        assert_eq!(rgb_to_ansi16((0, 0, 0)), 0);
        assert_eq!(rgb_to_ansi16((255, 255, 255)), 15);
        assert_eq!(rgb_to_ansi16((0x1a, 0x1a, 0x1a)), 0);
    }

    #[test]
    fn 段を超える色だけが近似される() {
        let s = Style {
            fg: Color::Rgb(255, 255, 255),
            bg: Color::Rgb(0x3b, 0x5b, 0xb5),
            mods: StyleMods::BOLD,
            underline_color: None,
        };
        let (t, changed) = clamp_style(s, ColorDepth::TrueColor);
        assert_eq!((t, changed), (s, false));
        let (a, changed) = clamp_style(s, ColorDepth::Ansi256);
        assert!(changed);
        assert!(matches!(a.fg, Color::Indexed(_)) && matches!(a.bg, Color::Indexed(_)));
        assert_eq!(a.mods, StyleMods::BOLD);
        let (b, _) = clamp_style(s, ColorDepth::Ansi16);
        assert!(matches!(b.fg, Color::Indexed(i) if i < 16));
        // Mono: 色は捨てて属性だけ残す
        let (m, changed) = clamp_style(s, ColorDepth::Mono);
        assert!(changed);
        assert_eq!(
            (m.fg, m.bg, m.mods),
            (Color::Default, Color::Default, StyleMods::BOLD)
        );
        // 既定色と 0..=15 の番号は、どの段でもそのまま (Mono を除く)
        let d = Style::with_mods(StyleMods::REVERSE);
        assert_eq!(clamp_style(d, ColorDepth::Mono), (d, false));
        let i = Style {
            fg: Color::Indexed(9),
            ..Style::DEFAULT
        };
        assert_eq!(clamp_style(i, ColorDepth::Ansi16), (i, false));
        // 16 色の段に 16 以上の番号が来たら近似する
        let j = Style {
            fg: Color::Indexed(196),
            ..Style::DEFAULT
        };
        let (j16, changed) = clamp_style(j, ColorDepth::Ansi16);
        assert!(changed && matches!(j16.fg, Color::Indexed(i) if i < 16));
    }

    #[test]
    fn 近似で前景と背景が同じになったら反転にする() {
        let s = Style {
            fg: Color::Rgb(250, 10, 10),
            bg: Color::Rgb(255, 0, 0),
            mods: StyleMods::NONE,
            underline_color: None,
        };
        let (c, _) = clamp_style(s, ColorDepth::Ansi16);
        assert_eq!(c.fg, Color::Default);
        assert!(c.mods.contains(StyleMods::REVERSE));
    }
}
