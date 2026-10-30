//! 設定の層の合成と出力 (docs/07 §3・§18.3): ユーザー層のファイル (`--config` で差し替え・複数可) を順に重ね、
//! 最後にコマンドライン層 (`--set key=value`) を重ねる。`--dump-config` の出力もここ。
//!
//! 起動時 (bin) と `config.reload` (core) が同じ規則で合成するよう、読み込んだ本文を受け取る純関数にしている。

use crate::config::{
    BackspaceUnit, ColorMode, Config, DefaultEncoding, DefaultLineEnding, IndentUnit, Osc52Policy,
    ShowWhitespace, ThemeBackground, WrapIndent, WrapMode, load_str_onto,
};
use crate::{DiagKind, Diagnostic, Loaded, Severity};

/// ユーザー層のファイル 1 つ。`text` は、無ければ `Ok(None)`、読めなければ理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// 診断に出す名前 (`config.toml`、`--config` のパス)。
    pub name: String,
    pub text: Result<Option<String>, String>,
}

/// 合成の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub config: Config,
    /// (名前, 診断)。診断の無いものは含めない。
    pub reports: Vec<(String, Vec<Diagnostic>)>,
    /// エラー (重さ `Error`) の診断がある (`--check-config` の終了コード 3、`config.reload` の維持の判定)。
    pub has_error: bool,
}

/// `--set` の診断に出す名前。
pub const SET_SOURCE: &str = "--set";

fn unreadable(message: String) -> Diagnostic {
    Diagnostic {
        severity: Severity::Error,
        kind: DiagKind::Syntax { message },
        key: String::new(),
        value: String::new(),
        line: 0,
        column: 0,
    }
}

/// `--set key=value` を `base` に重ねる。値は TOML の値として読み、読めなければ文字列とする
/// (`--set view.wrap=soft` と書ける)。`=` が無ければ `None` (使い方の誤り)。
pub fn load_set(base: Config, arg: &str) -> Option<Loaded> {
    let (key, value) = arg.split_once('=')?;
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    let value = value.trim();
    let as_toml = load_str_onto(base.clone(), &format!("{key} = {value}\n"));
    let syntax = as_toml
        .diagnostics
        .iter()
        .any(|d| matches!(d.kind, DiagKind::Syntax { .. }));
    if !syntax {
        return Some(as_toml);
    }
    let quoted = value.replace('\\', "\\\\").replace('"', "\\\"");
    Some(load_str_onto(base, &format!("{key} = \"{quoted}\"\n")))
}

/// ユーザー層のファイルを順に重ね、最後に `sets` を重ねる。読めないファイル・構文エラーのファイルは重ねない
/// (既定値または前のファイルまでの値のまま。起動時はそれで動き、`config.reload` は `has_error` を見て前の設定を保つ)。
pub fn resolve(sources: &[Source], sets: &[String]) -> Resolved {
    let mut config = Config::default();
    let mut reports = Vec::new();
    let mut has_error = false;
    for s in sources {
        let loaded = match &s.text {
            Ok(None) => continue,
            Ok(Some(t)) => load_str_onto(config.clone(), t.strip_prefix('\u{feff}').unwrap_or(t)),
            Err(e) => Loaded {
                config: config.clone(),
                diagnostics: vec![unreadable(e.clone())],
            },
        };
        let error = loaded
            .diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error);
        has_error |= error;
        if !error {
            config = loaded.config;
        }
        if !loaded.diagnostics.is_empty() {
            reports.push((s.name.clone(), loaded.diagnostics));
        }
    }
    let mut set_diags = Vec::new();
    for arg in sets {
        if let Some(l) = load_set(config.clone(), arg) {
            config = l.config;
            set_diags.extend(l.diagnostics);
        }
    }
    has_error |= set_diags.iter().any(|d| d.severity == Severity::Error);
    if !set_diags.is_empty() {
        reports.push((SET_SOURCE.to_owned(), set_diags));
    }
    Resolved {
        config,
        reports,
        has_error,
    }
}

/// 公開キーの (キー, TOML の値の表記)。並びは 07 §5 の表の順に近い、セクションごと。
pub fn values(c: &Config) -> Vec<(&'static str, String)> {
    let s = |v: &str| format!("\"{v}\"");
    vec![
        ("editor.tab_width", c.tab_width.to_string()),
        (
            "editor.indent_unit",
            s(match c.indent_unit {
                IndentUnit::Tab => "tab",
                IndentUnit::Space => "space",
            }),
        ),
        ("editor.indent_width", c.indent_width.to_string()),
        (
            "editor.backspace_unit",
            s(match c.backspace_unit {
                BackspaceUnit::Grapheme => "grapheme",
                BackspaceUnit::Codepoint => "codepoint",
            }),
        ),
        (
            "editor.undo.total_max_bytes",
            c.undo_total_max_bytes.to_string(),
        ),
        (
            "file.default_encoding",
            s(match c.default_encoding {
                DefaultEncoding::Utf8 => "utf-8",
                DefaultEncoding::Utf8Bom => "utf-8-bom",
                DefaultEncoding::Utf16Le => "utf-16le",
                DefaultEncoding::Utf16Be => "utf-16be",
                DefaultEncoding::ShiftJis => "shift_jis",
                DefaultEncoding::EucJp => "euc-jp",
                DefaultEncoding::Iso2022Jp => "iso-2022-jp",
                DefaultEncoding::System => "system",
            }),
        ),
        (
            "file.default_line_ending",
            s(match c.default_line_ending {
                DefaultLineEnding::Lf => "lf",
                DefaultLineEnding::Crlf => "crlf",
                DefaultLineEnding::Cr => "cr",
                DefaultLineEnding::Native => "native",
            }),
        ),
        (
            "file.huge_file_warn_bytes",
            c.huge_file_warn_bytes.to_string(),
        ),
        (
            "file.long_line_threshold",
            c.long_line_threshold.to_string(),
        ),
        ("file.swap.enabled", c.swap_enabled.to_string()),
        ("file.swap.interval_ms", c.swap_interval_ms.to_string()),
        (
            "file.swap.min_interval_ms",
            c.swap_min_interval_ms.to_string(),
        ),
        ("file.swap.max_bytes", c.swap_max_bytes.to_string()),
        (
            "file.swap.exclude_secret_files",
            c.swap_exclude_secret_files.to_string(),
        ),
        ("view.ambiguous_width", c.ambiguous_width.to_string()),
        (
            "view.wrap",
            s(match c.wrap {
                WrapMode::Off => "off",
                WrapMode::Soft => "soft",
                WrapMode::Word => "word",
            }),
        ),
        (
            "view.wrap_indent",
            s(match c.wrap_indent {
                WrapIndent::None => "none",
                WrapIndent::Same => "same",
            }),
        ),
        ("view.wrap_kinsoku", c.wrap_kinsoku.to_string()),
        (
            "view.show_whitespace",
            s(match c.show_whitespace {
                ShowWhitespace::Off => "off",
                ShowWhitespace::Minimal => "minimal",
            }),
        ),
        (
            "theme.color_mode",
            s(match c.color_mode {
                ColorMode::Auto => "auto",
                ColorMode::TrueColor => "truecolor",
                ColorMode::Color256 => "256",
                ColorMode::Color16 => "16",
                ColorMode::None => "none",
            }),
        ),
        ("theme.name", s(c.theme_name.as_str())),
        (
            "theme.background",
            s(match c.theme_background {
                ThemeBackground::Auto => "auto",
                ThemeBackground::Dark => "dark",
                ThemeBackground::Light => "light",
            }),
        ),
        ("ui.locale", s(c.ui_locale.map_or("auto", |l| l.code()))),
        ("ui.mouse", c.mouse.to_string()),
        ("ui.mouse_wheel_lines", c.mouse_wheel_lines.to_string()),
        ("ui.which_key_delay_ms", c.which_key_delay_ms.to_string()),
        (
            "clipboard.osc52",
            s(match c.osc52 {
                Osc52Policy::Auto => "auto",
                Osc52Policy::Always => "always",
                Osc52Policy::Never => "never",
            }),
        ),
        ("clipboard.osc52_max_bytes", c.osc52_max_bytes.to_string()),
    ]
}

/// `--dump-config` の出力。TOML ではセクションごとに並べ、JSON では平らなキーの表にする。
/// どちらも、そのまま設定ファイルとして (JSON は値の確認に) 使える形。
pub fn dump(c: &Config, json: bool) -> String {
    let vals = values(c);
    if json {
        let body: Vec<String> = vals
            .iter()
            .map(|(k, v)| format!("  \"{k}\": {v}"))
            .collect();
        return format!("{{\n{}\n}}\n", body.join(",\n"));
    }
    let mut out = String::new();
    let mut section = "";
    for (k, v) in &vals {
        let (sec, rest) = k.split_once('.').unwrap_or(("", k));
        if sec != section {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("[{sec}]\n"));
            section = sec;
        }
        out.push_str(&format!("{rest} = {v}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_str;

    fn src(name: &str, text: &str) -> Source {
        Source {
            name: name.to_owned(),
            text: Ok(Some(text.to_owned())),
        }
    }

    #[test]
    fn ファイルを順に重ね_後のファイルが勝つ() {
        let r = resolve(
            &[
                src("a.toml", "[editor]\ntab_width = 8\nindent_width = 2\n"),
                src("b.toml", "[editor]\ntab_width = 3\n"),
            ],
            &[],
        );
        assert_eq!((r.config.tab_width, r.config.indent_width), (3, 2));
        assert!(!r.has_error && r.reports.is_empty());
    }

    #[test]
    fn 構文エラーのファイルは重ねず_エラーとして報告する() {
        let r = resolve(
            &[
                src("a.toml", "[editor]\ntab_width = 8\n"),
                src("b.toml", "[editor\n"),
            ],
            &[],
        );
        assert_eq!(r.config.tab_width, 8);
        assert!(r.has_error);
        assert_eq!(r.reports[0].0, "b.toml");
    }

    #[test]
    fn set_は最後に重ね_引用符なしの文字列も受け付ける() {
        let r = resolve(
            &[src("a.toml", "[view]\nwrap = \"word\"\n")],
            &["view.wrap=soft".into(), "editor.tab_width=2".into()],
        );
        assert_eq!(r.config.wrap, WrapMode::Soft);
        assert_eq!(r.config.tab_width, 2);
        let r = resolve(&[], &["editor.tab_width=99".into()]);
        assert_eq!(r.config.tab_width, 4, "範囲外は既定値");
        assert_eq!(r.reports[0].0, SET_SOURCE);
        assert!(load_set(Config::default(), "novalue").is_none());
    }

    #[test]
    fn 出力は読み直すと同じ設定になる() {
        let c = Config {
            tab_width: 8,
            wrap: WrapMode::Word,
            ui_locale: Some(crate::Locale::En),
            osc52: Osc52Policy::Never,
            ..Config::default()
        };
        let l = load_str(&dump(&c, false));
        assert!(l.diagnostics.is_empty(), "{:?}", l.diagnostics);
        assert_eq!(l.config, c);
        // 公開キーをすべて出す
        assert_eq!(values(&c).len(), crate::config::KEYS.len());
        assert!(dump(&c, true).contains("\"editor.tab_width\": 8"));
    }
}
