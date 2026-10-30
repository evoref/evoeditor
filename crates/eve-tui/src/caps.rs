//! 端末能力の判定 (docs/04 §5.2 / §5.2a、§3.7)。環境変数と設定だけで決め、端末には問い合わせない (T3)。
//! 判定は純関数にして、環境を読む部分 (`Env`) と分ける (09 §3.5 の表駆動テスト)。

use std::collections::BTreeMap;

use eve_view::{Capabilities, CellSize, ColorDepth};

/// 環境変数の読み口。本番は `ProcessEnv`、テストは `MapEnv`。
pub trait Env {
    fn var(&self, key: &str) -> Option<String>;
}

/// プロセスの環境変数。
#[derive(Clone, Copy, Debug, Default)]
pub struct ProcessEnv;

impl Env for ProcessEnv {
    fn var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }
}

/// テスト用の環境。
#[derive(Clone, Debug, Default)]
pub struct MapEnv(pub BTreeMap<String, String>);

impl MapEnv {
    pub fn new(pairs: &[(&str, &str)]) -> MapEnv {
        MapEnv(
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        )
    }
}

impl Env for MapEnv {
    fn var(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }
}

/// 設定 `theme.color_mode` (07 §5.4)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorMode {
    #[default]
    Auto,
    TrueColor,
    Ansi256,
    Ansi16,
    None,
}

impl ColorMode {
    /// 設定値の文字列 (`auto` / `truecolor` / `256` / `16` / `none`) から。
    pub fn parse(s: &str) -> Option<ColorMode> {
        Some(match s {
            "auto" => ColorMode::Auto,
            "truecolor" => ColorMode::TrueColor,
            "256" => ColorMode::Ansi256,
            "16" => ColorMode::Ansi16,
            "none" => ColorMode::None,
            _ => return None,
        })
    }
}

/// 端末種別 (04 §3.7 の A2、§5.2 の曖昧幅のヒント)。**色深度の判定とは独立**。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalKind {
    WindowsTerminal,
    /// Windows の conhost、または Windows Terminal と判定できない端末 (`TERM_PROGRAM` を持つ VS Code 等も含む。安全側)。
    Conhost,
    /// Windows 以外 (v0.2)。
    Other,
}

/// 端末種別の推定。
pub fn terminal_kind(env: &dyn Env, is_windows: bool) -> TerminalKind {
    if !is_windows {
        return TerminalKind::Other;
    }
    if env.var("WT_SESSION").is_some() && env.var("TERM_PROGRAM").is_none() {
        TerminalKind::WindowsTerminal
    } else {
        TerminalKind::Conhost
    }
}

/// `Frontend::init` が起動を断る理由 (04 §5.2)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unusable {
    /// `TERM=dumb`。
    DumbTerminal,
}

/// `TERM=dumb` なら起動しない。
pub fn check_usable(env: &dyn Env) -> Result<(), Unusable> {
    if env.var("TERM").as_deref() == Some("dumb") {
        return Err(Unusable::DumbTerminal);
    }
    Ok(())
}

fn nonempty(env: &dyn Env, key: &str) -> bool {
    env.var(key).is_some_and(|v| !v.is_empty())
}

/// 環境変数から既知の TrueColor 端末と判断できるか (§5.2a の順 4)。
fn known_truecolor_terminal(env: &dyn Env) -> bool {
    if env.var("WT_SESSION").is_some() || env.var("KITTY_WINDOW_ID").is_some() {
        return true;
    }
    if let Some(p) = env.var("TERM_PROGRAM")
        && matches!(p.as_str(), "iTerm.app" | "WezTerm" | "vscode" | "ghostty")
    {
        return true;
    }
    if let Some(t) = env.var("TERM")
        && (matches!(t.as_str(), "xterm-kitty" | "alacritty")
            || t.starts_with("foot")
            || t.ends_with("-direct"))
    {
        return true;
    }
    if let Some(v) = env.var("VTE_VERSION")
        && v.parse::<u32>().is_ok_and(|n| n >= 3600)
    {
        return true;
    }
    false
}

/// 色深度の判定 (04 §5.2a。先に当てはまったものを採る)。
pub fn detect_color_depth(mode: ColorMode, env: &dyn Env, is_windows: bool) -> ColorDepth {
    // 1 設定
    match mode {
        ColorMode::TrueColor => return ColorDepth::TrueColor,
        ColorMode::Ansi256 => return ColorDepth::Ansi256,
        ColorMode::Ansi16 => return ColorDepth::Ansi16,
        ColorMode::None => return ColorDepth::Mono,
        ColorMode::Auto => {}
    }
    // 2 NO_COLOR
    if nonempty(env, "NO_COLOR") {
        return ColorDepth::Mono;
    }
    // 3 COLORTERM
    if matches!(env.var("COLORTERM").as_deref(), Some("truecolor" | "24bit")) {
        return ColorDepth::TrueColor;
    }
    // 4 既知の TrueColor 端末
    if known_truecolor_terminal(env) {
        return ColorDepth::TrueColor;
    }
    // 5 Terminal.app
    if env.var("TERM_PROGRAM").as_deref() == Some("Apple_Terminal") {
        return ColorDepth::Ansi256;
    }
    // 6 Windows (conhost、VT 有効)
    if is_windows {
        return ColorDepth::TrueColor;
    }
    // 7 tmux
    if env.var("TMUX").is_some() {
        return ColorDepth::Ansi256;
    }
    let term = env.var("TERM").unwrap_or_default();
    // 8 TERM が 256color
    if term.contains("256color") {
        return ColorDepth::Ansi256;
    }
    // 9 古い端末
    if matches!(term.as_str(), "linux" | "vt100" | "vt220" | "ansi") {
        return ColorDepth::Ansi16;
    }
    // 10 それ以外
    ColorDepth::Ansi256
}

/// 曖昧幅の推定値 (04 §5.2。S-06 の実測、2026-10-21)。Windows Terminal は 1、conhost は 2。それ以外は不明。
pub fn ambiguous_width_hint(kind: TerminalKind) -> Option<u8> {
    match kind {
        TerminalKind::WindowsTerminal => Some(1),
        TerminalKind::Conhost => Some(2),
        TerminalKind::Other => None,
    }
}

/// 実効の曖昧幅: 設定 (`view.ambiguous_width`、1 または 2) が明示されていればそれ、無ければヒント、無ければ 1。
pub fn effective_ambiguous_width(setting: Option<u8>, hint: Option<u8>) -> u8 {
    match setting {
        Some(w @ (1 | 2)) => w,
        _ => hint.unwrap_or(1),
    }
}

/// 装飾記号を ASCII に縮退するか (04 §3.7。A1: 実効の曖昧幅が 2、A2: Windows で Windows Terminal でない)。
pub fn ascii_only(effective_ambiguous: u8, kind: TerminalKind) -> bool {
    effective_ambiguous == 2 || kind == TerminalKind::Conhost
}

/// `Frontend::init` に渡す設定 (04 §5.2a)。
#[derive(Clone, Copy, Debug, Default)]
pub struct CapsConfig {
    pub color_mode: ColorMode,
    /// `view.ambiguous_width` (明示されたときだけ)。
    pub ambiguous_width: Option<u8>,
}

/// 判定の結果。`Capabilities` と、frontend の内部で使う値。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detected {
    pub caps: Capabilities,
    pub kind: TerminalKind,
    /// 実効の曖昧幅 (core の `WidthConfig` に渡す値)。
    pub ambiguous_width: u8,
}

/// 環境と設定から `Capabilities` を作る (v0.1 は Windows の 2 端末が対象)。
pub fn detect(config: CapsConfig, env: &dyn Env, is_windows: bool) -> Detected {
    let kind = terminal_kind(env, is_windows);
    let hint = ambiguous_width_hint(kind);
    let ambiguous = effective_ambiguous_width(config.ambiguous_width, hint);
    let caps = Capabilities {
        color_depth: detect_color_depth(config.color_mode, env, is_windows),
        // マウスは v0.2 から (Windows は `ENABLE_MOUSE_INPUT`、04 §6.8)。拡張キー・ジョブ制御は無効 (04 §6.3)
        mouse: is_windows,
        enhanced_keys: false,
        // Windows Terminal は OSC 52 の書込に対応する。conhost は対応しない (05 §8.1)
        osc52_write: kind == TerminalKind::WindowsTerminal,
        can_suspend: false,
        dark_mode: None,
        ascii_only: ascii_only(ambiguous, kind),
        screen_reader: false,
        ambiguous_width_hint: hint,
        min_surface: CellSize { cols: 20, rows: 5 },
        intercepted_keys: Vec::new(),
    };
    Detected {
        caps,
        kind,
        ambiguous_width: ambiguous,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn depth(pairs: &[(&str, &str)], windows: bool) -> ColorDepth {
        detect_color_depth(ColorMode::Auto, &MapEnv::new(pairs), windows)
    }

    // verifies: AC-FR-701-1
    // verifies: AC-FR-513-3
    #[test]
    fn 色深度の判定表の各行() {
        // 順 1: 設定
        for (m, d) in [
            (ColorMode::TrueColor, ColorDepth::TrueColor),
            (ColorMode::Ansi256, ColorDepth::Ansi256),
            (ColorMode::Ansi16, ColorDepth::Ansi16),
            (ColorMode::None, ColorDepth::Mono),
        ] {
            let env = MapEnv::new(&[("NO_COLOR", "1")]);
            assert_eq!(detect_color_depth(m, &env, true), d);
        }
        // 順 2: NO_COLOR (空は無視)
        assert_eq!(
            depth(&[("NO_COLOR", "1"), ("COLORTERM", "truecolor")], true),
            ColorDepth::Mono
        );
        assert_eq!(
            depth(&[("NO_COLOR", ""), ("WT_SESSION", "x")], true),
            ColorDepth::TrueColor
        );
        // 順 3: COLORTERM
        assert_eq!(
            depth(&[("COLORTERM", "truecolor"), ("TERM", "linux")], false),
            ColorDepth::TrueColor
        );
        assert_eq!(
            depth(&[("COLORTERM", "24bit")], false),
            ColorDepth::TrueColor
        );
        // 順 4: 既知の TrueColor 端末
        for pairs in [
            vec![("WT_SESSION", "g")],
            vec![("TERM_PROGRAM", "iTerm.app")],
            vec![("TERM_PROGRAM", "WezTerm")],
            vec![("TERM_PROGRAM", "vscode")],
            vec![("TERM_PROGRAM", "ghostty")],
            vec![("KITTY_WINDOW_ID", "1")],
            vec![("TERM", "xterm-kitty")],
            vec![("TERM", "alacritty")],
            vec![("TERM", "foot-extra")],
            vec![("TERM", "xterm-direct")],
            vec![("VTE_VERSION", "3600")],
        ] {
            assert_eq!(depth(&pairs, false), ColorDepth::TrueColor, "{pairs:?}");
        }
        assert_eq!(
            depth(
                &[("VTE_VERSION", "3599"), ("TERM", "xterm-256color")],
                false
            ),
            ColorDepth::Ansi256
        );
        // 順 5: Terminal.app
        assert_eq!(
            depth(&[("TERM_PROGRAM", "Apple_Terminal")], false),
            ColorDepth::Ansi256
        );
        // 順 6: Windows で 1〜4 に当たらない (conhost)
        assert_eq!(depth(&[], true), ColorDepth::TrueColor);
        // 順 7: tmux (COLORTERM なし)
        assert_eq!(
            depth(&[("TMUX", "/tmp/t"), ("TERM", "screen")], false),
            ColorDepth::Ansi256
        );
        // 順 8 / 9 / 10
        assert_eq!(
            depth(&[("TERM", "xterm-256color")], false),
            ColorDepth::Ansi256
        );
        for t in ["linux", "vt100", "vt220", "ansi"] {
            assert_eq!(depth(&[("TERM", t)], false), ColorDepth::Ansi16, "{t}");
        }
        assert_eq!(depth(&[("TERM", "xterm")], false), ColorDepth::Ansi256);
        assert_eq!(depth(&[], false), ColorDepth::Ansi256);
    }

    #[test]
    fn term_dumb_は起動しない() {
        assert_eq!(
            check_usable(&MapEnv::new(&[("TERM", "dumb")])),
            Err(Unusable::DumbTerminal)
        );
        assert_eq!(check_usable(&MapEnv::new(&[("TERM", "xterm")])), Ok(()));
        assert_eq!(check_usable(&MapEnv::new(&[])), Ok(()));
    }

    #[test]
    fn 端末種別は_wt_session_と_term_program_で決まり_色深度とは独立() {
        let wt = MapEnv::new(&[("WT_SESSION", "g")]);
        assert_eq!(terminal_kind(&wt, true), TerminalKind::WindowsTerminal);
        assert_eq!(
            terminal_kind(&MapEnv::new(&[]), true),
            TerminalKind::Conhost
        );
        // VS Code の統合端末 (TERM_PROGRAM あり) は安全側で conhost と同じ扱い
        let vs = MapEnv::new(&[("WT_SESSION", "g"), ("TERM_PROGRAM", "vscode")]);
        assert_eq!(terminal_kind(&vs, true), TerminalKind::Conhost);
        // NO_COLOR や COLORTERM を設定した conhost も A2 に当たる
        let c = MapEnv::new(&[("NO_COLOR", "1"), ("COLORTERM", "truecolor")]);
        assert_eq!(terminal_kind(&c, true), TerminalKind::Conhost);
        assert_eq!(terminal_kind(&wt, false), TerminalKind::Other);
    }

    #[test]
    fn 曖昧幅のヒントと実効値() {
        assert_eq!(ambiguous_width_hint(TerminalKind::WindowsTerminal), Some(1));
        assert_eq!(ambiguous_width_hint(TerminalKind::Conhost), Some(2));
        assert_eq!(ambiguous_width_hint(TerminalKind::Other), None);
        // 設定が優先
        assert_eq!(effective_ambiguous_width(Some(1), Some(2)), 1);
        assert_eq!(effective_ambiguous_width(Some(2), Some(1)), 2);
        assert_eq!(effective_ambiguous_width(None, Some(2)), 2);
        assert_eq!(effective_ambiguous_width(None, None), 1);
        assert_eq!(
            effective_ambiguous_width(Some(7), Some(2)),
            2,
            "不正な値は無視"
        );
    }

    #[test]
    fn ascii_縮退の条件() {
        // A1: 実効の曖昧幅が 2
        assert!(ascii_only(2, TerminalKind::WindowsTerminal));
        // A2: Windows Terminal でない (conhost)
        assert!(ascii_only(1, TerminalKind::Conhost));
        // 縮退しないのは、曖昧幅 1 の Windows Terminal だけ
        assert!(!ascii_only(1, TerminalKind::WindowsTerminal));
    }

    #[test]
    fn 能力の組み立て() {
        let wt = MapEnv::new(&[("WT_SESSION", "g")]);
        let d = detect(CapsConfig::default(), &wt, true);
        assert_eq!(d.caps.color_depth, ColorDepth::TrueColor);
        assert!(!d.caps.ascii_only);
        assert_eq!(d.ambiguous_width, 1);
        assert_eq!(d.caps.ambiguous_width_hint, Some(1));
        assert_eq!(d.caps.min_surface, CellSize { cols: 20, rows: 5 });
        assert!(d.caps.mouse && !d.caps.enhanced_keys && !d.caps.can_suspend);

        let d = detect(CapsConfig::default(), &MapEnv::new(&[]), true);
        assert_eq!(d.kind, TerminalKind::Conhost);
        assert!(d.caps.ascii_only);
        assert_eq!(d.ambiguous_width, 2);

        // 曖昧幅を 1 と明示しても、conhost は A2 で縮退する
        let cfg = CapsConfig {
            ambiguous_width: Some(1),
            ..CapsConfig::default()
        };
        let d = detect(cfg, &MapEnv::new(&[]), true);
        assert_eq!(d.ambiguous_width, 1);
        assert!(d.caps.ascii_only);

        // Windows Terminal で曖昧幅 2 と明示すると A1 で縮退する
        let cfg = CapsConfig {
            ambiguous_width: Some(2),
            ..CapsConfig::default()
        };
        assert!(detect(cfg, &wt, true).caps.ascii_only);
    }

    #[test]
    fn 色の設定値の読み() {
        assert_eq!(ColorMode::parse("auto"), Some(ColorMode::Auto));
        assert_eq!(ColorMode::parse("truecolor"), Some(ColorMode::TrueColor));
        assert_eq!(ColorMode::parse("256"), Some(ColorMode::Ansi256));
        assert_eq!(ColorMode::parse("16"), Some(ColorMode::Ansi16));
        assert_eq!(ColorMode::parse("none"), Some(ColorMode::None));
        assert_eq!(ColorMode::parse("rgb"), None);
    }
}
